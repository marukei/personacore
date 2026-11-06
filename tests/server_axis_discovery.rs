//! **軸の候補づくりの配線**（[ADR-0046](../docs/adr/0046-drift-proposes-new-axes.md) §決定2 /
//! 作業単位 A-9b / Issue origin#42）の検証。**本番の入口（`App::bind`）を通す。**
//!
//! いつ尋ねるか・何を載せるか・何を捨てるかは `server::axis_discovery` の単体テストが持つ。
//! ここが見るのは**本番の入口から届くもの**だけ:
//!
//! | 腕 | 変容の走らせ方 | `N` | 1往復で記憶が 2 件増えたとき |
//! |---|---|---|---|
//! | 設定した間隔 | `shadow` | **2**（既定ではない） | **候補の依頼が1回届き、試用の候補が1件できる** |
//! | 既定の間隔 | `shadow` | 未設定（既定） | 届かない（既定は 2 より大きい） |
//! | 走らせ方が off | 未設定（既定 off） | 2 | **届かない**（off では候補づくりも走らない / `server::extract::run` の doc） |
//!
//! **1本目だけでは足りない。** 「設定を読まずに常に尋ねる」実装も1本目を満たす。
//! 2本目（既定の腕）が「間隔を読んでいる」を、3本目が「off で走らない」を押さえる
//! （CLAUDE.md「既定と違う値で回る腕を1本置く」「対比は安い側だけでは足りない」）。
//!
//! 上流は `wiremock`。実機の LM Studio には繋がない。

mod common;

use std::time::{Duration, Instant};

use personacore::domain::{
    AxisCandidateState, AxisDiscoverySlots, CharacterId, CharacterUpdate, DisplayName, Handle,
    Persona,
};
use personacore::server::{
    App, AppConfig, BackgroundProbe, BindAddr, ENV_AXIS_DISCOVERY_MEMORIES, ENV_LLM_ENDPOINT,
    ENV_MAIN_MODEL, ENV_STAT_DRIFT, ENV_SUB_MODEL,
};
use personacore::store::Store;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

use common::fixture_scale;

const HANDLE: &str = "aoi";
const DISPLAY_NAME: &str = "葵";
const MAIN_MODEL: &str = "ministral-3-14b-reasoning-2512";
const SUB_MODEL: &str = "ministral-3-3b-reasoning-2512";

/// **候補の依頼だけに現れる見出し**（振り分けに使う）。
///
/// **`prompt::axis_discovery` の定数を呼ばず、テスト側に期待値として持つ**
/// （呼ぶと綴りを変える変異が両側で同時に動いて緑になる）。
const DISCOVERY_HEADING: &str = "# あなた自身について最近わかったこと";

/// 抽出の依頼だけに現れる見出し。
const EXTRACTION_HEADING: &str = "# 会話";

/// 本文の依頼だけに現れる見出し。
const PERSONA_HEADING: &str = "# ペルソナ";

/// **1往復の抽出で増える、キャラクター自身の記憶**（2 件）。
const EXTRACTED: &str = "- あなた: 雨の日は店を早く閉める\n- あなた: 常連の名前を全部覚えている";

/// 1往復で増える記憶の件数（[`EXTRACTED`] の行数）。
const EXTRACTED_COUNT: u32 = 2;

/// 候補の依頼への答え。
const CANDIDATE: &str = "照れ屋";

fn upstream_body(model: &str, content: &str) -> String {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop",
        }],
    })
    .to_string()
}

struct Harness {
    upstream: MockServer,
    store: Store,
    addr: std::net::SocketAddr,
    character: CharacterId,
    background: BackgroundProbe,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<Result<(), personacore::server::ServerError>>>,
    _dir: tempfile::TempDir,
}

impl Harness {
    /// **本番の入口（`App::bind`）で起動する。** `env` に無い設定は既定。
    async fn start(env: &[(&str, &str)]) -> Self {
        let upstream = MockServer::start().await;
        // **候補の依頼を先に据える**（いちばん狭い見出しで振り分ける）。
        for (heading, model, content) in [
            (DISCOVERY_HEADING, SUB_MODEL, CANDIDATE),
            (EXTRACTION_HEADING, SUB_MODEL, EXTRACTED),
            (
                PERSONA_HEADING,
                MAIN_MODEL,
                "「おかえりなさい」 彼女は本を閉じた。",
            ),
        ] {
            Mock::given(matchers::method("POST"))
                .and(matchers::path("/v1/chat/completions"))
                .and(matchers::body_string_contains(heading))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(upstream_body(model, content), "application/json"),
                )
                .mount(&upstream)
                .await;
        }

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let mut pairs: Vec<(String, String)> = vec![
            (ENV_LLM_ENDPOINT.to_owned(), upstream.uri()),
            (ENV_MAIN_MODEL.to_owned(), MAIN_MODEL.to_owned()),
            (ENV_SUB_MODEL.to_owned(), SUB_MODEL.to_owned()),
        ];
        pairs.extend(
            env.iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
        );
        let config = AppConfig::from_lookup(|key| {
            pairs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(dir.path().join("personacore.sqlite"));

        // **`bind_with_clients` ではない。** 本番と同じ入口であることが要点。
        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let store = app.store().clone();
        let background = app.background_probe();

        let handle = Handle::parse(HANDLE).expect("正当な識別子");
        let character = store
            .create_character(
                &handle,
                &DisplayName::parse(DISPLAY_NAME).expect("正当な表示名"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナ"),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");
        // **枠は 1**（空きがある）。軸は持たない（変容は `no_axes` で飛ぶ → 依頼は本文・抽出・候補だけ）。
        store
            .update_character(
                &handle,
                &CharacterUpdate {
                    axis_discovery_slots: Some(AxisDiscoverySlots::parse(1).expect("正当な枠の数")),
                    ..CharacterUpdate::default()
                },
            )
            .await
            .expect("枠の数を書けること");

        let (shutdown, rx) = oneshot::channel();
        let serving = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });

        Self {
            upstream,
            store,
            addr,
            character,
            background,
            shutdown: Some(shutdown),
            handle: Some(serving),
            _dir: dir,
        }
    }

    /// 1往復投げて**読み切り**、そのターンの裏方が終わるまで待つ。
    ///
    /// **「A が B になる」を待つ形**: 先に抽出の依頼が届いたこと（＝裏方が走った）を押さえ、
    /// その後で枠が空くのを待つ。**どちらも上限つき。**
    async fn chat_and_settle(&self) -> Vec<Value> {
        let response = reqwest::Client::new()
            .post(format!("http://{}/v1/chat/completions", self.addr))
            .json(&json!({
                "model": HANDLE,
                "messages": [{"role": "user", "content": "ただいま。"}],
            }))
            .send()
            .await
            .expect("サーバへ接続できること");
        assert!(response.status().is_success(), "{:?}", response.status());
        let _ = response.text().await.expect("本文を読めること");

        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let requests = self.requests().await;
            if requests
                .iter()
                .any(|request| request["messages"].to_string().contains(EXTRACTION_HEADING))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "抽出の依頼が 20 秒たっても届かない"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        while !self.background.is_idle() {
            assert!(
                Instant::now() < deadline,
                "裏方の枠が 20 秒たっても空かない"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.requests().await
    }

    async fn requests(&self) -> Vec<Value> {
        self.upstream
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|request| request.url.path() == "/v1/chat/completions")
            .map(|request| serde_json::from_slice(&request.body).expect("JSON として読めること"))
            .collect()
    }

    async fn candidates(&self) -> Vec<(String, AxisCandidateState)> {
        self.store
            .axis_candidates(self.character, 10)
            .await
            .expect("候補を読めること")
            .iter()
            .map(|candidate| (candidate.name().as_str().to_owned(), candidate.state()))
            .collect()
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).expect("停止シグナルを送れること");
        }
        self.handle
            .take()
            .expect("起動時のタスクがあること")
            .await
            .expect("サーバタスクが panic しないこと")
            .expect("正常に終了すること");
    }
}

fn discovery_requests(requests: &[Value]) -> usize {
    requests
        .iter()
        .filter(|request| request["messages"].to_string().contains(DISCOVERY_HEADING))
        .count()
}

/// **`PERSONACORE_AXIS_DISCOVERY_MEMORIES` が本番の入口から裏方まで届く。off では走らない。**
///
/// 腕の表はモジュールの doc。**期待値は既定ではない値（2）で測る。**
#[tokio::test]
async fn the_axis_discovery_interval_reaches_the_background_through_the_real_bind() {
    const SET: &str = "2";
    assert_eq!(
        SET.parse::<u32>().expect("数"),
        EXTRACTED_COUNT,
        "前提: 1往復で増える記憶の件数ちょうどに設定する"
    );
    // **既定は 1往復ぶんより大きいこと**（既定の腕が「尋ねない」になる前提）。
    assert!(
        AppConfig::from_lookup(|_| None)
            .expect("既定で組み立てられること")
            .axis_discovery_memories()
            .get()
            > EXTRACTED_COUNT,
        "前提: 既定の間隔は 1往復で増える記憶より多い"
    );

    // --- 設定した間隔（shadow・N = 2）: 1回尋ね、試用の候補ができる ---
    let set = Harness::start(&[
        (ENV_STAT_DRIFT, "shadow"),
        (ENV_AXIS_DISCOVERY_MEMORIES, SET),
    ])
    .await;
    let requests = set.chat_and_settle().await;
    assert_eq!(
        discovery_requests(&requests),
        1,
        "N = 2 なら、2 件増えた回に1回尋ねること: {requests:?}"
    );
    let discovery = requests
        .iter()
        .find(|request| request["messages"].to_string().contains(DISCOVERY_HEADING))
        .expect("候補の依頼があること");
    assert_eq!(discovery["model"], SUB_MODEL, "候補の依頼はサブモデルへ");
    assert_eq!(
        set.candidates().await,
        vec![(CANDIDATE.to_owned(), AxisCandidateState::Trial)],
        "採った名前が試用として残ること"
    );
    set.stop().await;

    // --- 既定の間隔（shadow・未設定）: 尋ねない ---
    let default = Harness::start(&[(ENV_STAT_DRIFT, "shadow")]).await;
    let requests = default.chat_and_settle().await;
    assert_eq!(
        discovery_requests(&requests),
        0,
        "既定の間隔では、2 件では尋ねないこと: {requests:?}"
    );
    assert!(default.candidates().await.is_empty());
    default.stop().await;

    // --- 走らせ方が off（既定）・N = 2: 尋ねない ---
    let off = Harness::start(&[(ENV_AXIS_DISCOVERY_MEMORIES, SET)]).await;
    let requests = off.chat_and_settle().await;
    assert_eq!(
        discovery_requests(&requests),
        0,
        "変容の走らせ方が off なら、候補づくりも走らないこと: {requests:?}"
    );
    assert!(off.candidates().await.is_empty());
    off.stop().await;
}

/// **読めない間隔は起動を止める**（黙って既定へ落とさない）。本番の入口の設定の読みで見る。
#[test]
fn an_unreadable_axis_discovery_interval_stops_the_startup() {
    for raw in ["0", "abc"] {
        let result = AppConfig::from_lookup(|key| {
            (key == ENV_AXIS_DISCOVERY_MEMORIES).then(|| raw.to_owned())
        });
        assert!(result.is_err(), "{raw:?} で起動を止めること");
    }
}
