//! **上流プロファイルの面に書いたサンプラの値**（[ADR-0058](../docs/adr/0058-sampling-values-in-the-upstream-profile.md) /
//! 作業単位 A-16）が、**本番の入口（`App::bind` → `App::serve`）と走ったままの切り替えを通って**、
//! その面の要求にだけ載ることの検証。**実際の上流には繋がない**（`wiremock` の偽物）。
//!
//! # 何を見るか（ADR-0058 §決定4 の表の腕）
//!
//! | 見張り番 | 腕 | 観測すること |
//! |---|---|---|
//! | [`arm1_the_main_faces_value_reaches_the_body_request`] | 1 | `main` 面の値が本文の要求に載る |
//! | [`arm2_without_a_sub_face_the_main_value_reaches_the_background_too`] | 2 | `sub` 面が無いとき、`main` 面の値が裏方の要求にも載る |
//! | [`arm3_the_sub_faces_value_reaches_the_background_and_not_the_body`] | 3 | `sub` 面の値が裏方に載り、本文には載らない |
//! | [`arm4_a_sub_face_without_sampling_uses_the_default_not_the_main_value`] | 4 | `sub` 面があって値が無いとき、裏方は既定で、`main` の値を写さない |
//! | [`arm5_switching_follows_the_sampling_even_when_endpoint_and_model_stay`] | 5 | 切り替えの3通りの遷移が次の往復から効く。**宛先とモデルが同じ**プロファイルの間で。`sub` 面を書いたプロファイルへの切り替えでは、裏方が裏方の面の値に追従する |
//! | [`arm6_switching_again_to_the_same_profile_without_a_sub_face_touches_nothing`] | 6 | `sub` 面の無いプロファイルへ切り替え直しても、裏方の面を触らない（照会が増えない） |
//!
//! **腕2と腕3・4は、互いを隠す実装を赤にする対**（「`sub` は常に `main` の値」「`sub` は常に既定」）。
//! **腕1〜4の値は既定の 1.1 とも互いとも違える**（0.95 / 1.45 / 1.05 / 1.25 / 1.3）。
//! **腕5は宛先とモデルの綴りを揃える** —— 揃えないと、`sampling` を比べない実装でも切り替えが起きて緑になる。
//!
//! # 「裏方」は抽出と、約束・決まりの提案
//!
//! 裏方の全経路（抽出・変容・約束・軸の発見と試行）は同じ裏方の面のクライアントを通る。ここで実際に走るのは
//! **抽出と提案**（変容は既定で off、軸の発見と試行はこの往復数では走らない）で、**両方の値を見る。**
//!
//! # 守っていないもの（先に書く）
//!
//! * **`main` の結線**（`config_for` が `with_profile_sampling` を呼ぶこと）は bin なので、ここからは見えない。
//!   **ここは `main` と同じ段（`overlay_profile` → `with_profile_context` → `with_profile_keys` →
//!   `with_profile_sampling`）を自分で組む。** 結線は `tests/upstream_cli.rs` の
//!   `a_sampling_written_in_the_profile_reaches_the_executable` が実行ファイルで見る。
//! * **値が上流で効いたか**は見ない（送った値まで / ADR-0058 §決定3）。

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use personacore::domain::{DisplayName, Handle, Persona};
use personacore::server::{
    App, AppConfig, BackgroundProbe, BindAddr, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL, ENV_SUB_MODEL,
    ServerError, overlay_profile, parse_profiles,
};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

use common::{bracketed_field, fixture_scale, has_field, install_global_log};

/// **切り替える腕を1本ずつ走らせる**（腕5・腕6）。ログはバイナリで1つなので、腕6の切り替えの行が
/// 腕5の「出た順の列」に混ざらないように。
static SWITCHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 本文に指定するモデル。
const MAIN_MODEL: &str = "sampling-main-model";
/// 裏方に指定するモデル。**本文用と別の綴り。**
const SUB_MODEL: &str = "sampling-sub-model";

/// 会話するキャラクター（`model` に入れる値）。
const HANDLE: &str = "aoi";
/// 呼び名。**識別子とは別の綴り。**
const DISPLAY_NAME: &str = "葵";
/// 利用者の発言。
const USER_MESSAGE: &str = "ただいま";

/// **既定の `repeat_penalty`**（`llm::RequestPolicy::PERSONACORE`）。**定数から作らず手で書く。**
const DEFAULT_PENALTY: f64 = 1.1;

/// **本文の依頼だけに現れる見出し**（`prompt` の定数を呼ばない）。
const PERSONA_HEADING: &str = "# ペルソナ";

/// **送るサンプラの行**（`server::context::SAMPLERS_LINE` の写し）。
const SAMPLERS_LINE: &str = "上流へ送るサンプラの値（面ごと）";

/// 待つ上限。**超えたら落とす**（緑のまま何も測らない形にしない）。
const WAIT_LIMIT: Duration = Duration::from_secs(20);
/// 待ちの刻み。
const POLL_STEP: Duration = Duration::from_millis(10);

/// 上流が返す本文（OpenAI 互換）。**抽出として読める行**を1件だけ含む。
fn upstream_body(model: &str) -> String {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "- 利用者: 山あいの町で育った"},
            "finish_reason": "stop",
        }],
    })
    .to_string()
}

/// **本文と裏方の両方を受けられる上流**（モデルの一覧には2つのモデルを答える）。
async fn upstream() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(upstream_body(MAIN_MODEL), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(
                r#"{{"object":"list","data":[
                  {{"id":"{MAIN_MODEL}","object":"model","type":"llm","state":"loaded",
                    "max_context_length":262144,"loaded_context_length":16384}},
                  {{"id":"{SUB_MODEL}","object":"model","type":"llm","state":"loaded",
                    "max_context_length":262144,"loaded_context_length":16384}}]}}"#
            ),
            "application/json",
        ))
        .mount(&server)
        .await;
    server
}

/// 上流へ届いた依頼の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// 本文。
    Body,
    /// 裏方（抽出・約束と決まりの提案）。
    Background,
}

/// **依頼の本文から種類を決める。** 分からなければ落とす（黙って数えない形にしない）。
fn kind_of(request: &Value) -> Kind {
    let messages = request["messages"].to_string();
    let extraction = messages.contains(personacore::prompt::extraction::EXAMPLE_USER_LINE);
    let proposal = [
        "抜｜引用｜取り決め｜守る側｜中身",
        "点検｜番号｜行い｜分類｜守る側｜判定｜中身",
    ]
    .iter()
    .any(|marker| messages.contains(marker));
    if extraction || proposal {
        Kind::Background
    } else if messages.contains(PERSONA_HEADING) {
        Kind::Body
    } else {
        panic!("依頼の種類が決まらない: {messages}")
    }
}

/// **その上流へ届いたチャットの依頼**（種類と `repeat_penalty`。届いた順）。
async fn penalties(server: &MockServer) -> Vec<(Kind, Value)> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .map(|request| {
            let body: Value = serde_json::from_slice(&request.body).expect("JSON として読めること");
            (kind_of(&body), body["repeat_penalty"].clone())
        })
        .collect()
}

/// **面の1つ**（`sampling` は書くときだけ）。
fn face(endpoint: &str, model: &str, repeat_penalty: Option<f64>) -> Value {
    let mut face = json!({ "endpoint": endpoint, "model": model });
    if let Some(value) = repeat_penalty {
        face["sampling"] = json!({ "repeat_penalty": value });
    }
    face
}

/// **`main` と同じ段で設定を組む**（`overlay_profile` → `with_profile_context` → `with_profile_keys` →
/// `with_profile_sampling`）。
fn config_of(source: &Value, name: &str, db_path: &Path) -> AppConfig {
    let pairs: Vec<(String, String)> = vec![
        // **囮**: プロファイルが勝つので、ここへは何も届かない。
        (ENV_LLM_ENDPOINT.to_owned(), "http://127.0.0.1:9".to_owned()),
        (ENV_MAIN_MODEL.to_owned(), "decoy-main".to_owned()),
        (ENV_SUB_MODEL.to_owned(), "decoy-sub".to_owned()),
    ];
    let base = move |key: &str| {
        pairs
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.clone())
    };
    let profiles =
        parse_profiles(&source.to_string(), None).expect("植えたプロファイルが読めること");
    let settings = profiles.select(name).expect("在ること").settings();
    AppConfig::from_lookup(overlay_profile(&settings, base))
        .expect("設定を組み立てられること")
        .with_profile_context(&settings)
        .expect("書いた値を載せられること")
        .with_profile_keys(&settings)
        .with_profile_sampling(&settings)
        .with_bind(BindAddr::loopback(0))
        .with_database_path(db_path)
}

/// **`App::bind`（本番の入口）で起動し、走らせる。**
struct Running {
    port: u16,
    background: BackgroundProbe,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
}

impl Running {
    async fn start(config: &AppConfig, profiles: Option<PathBuf>) -> Self {
        let mut app = App::bind(config).await.expect("待ち受けを開始できること");
        if let Some(path) = profiles {
            app = app.with_profiles_file_for_tests(path);
        }
        app.store()
            .create_character(
                &Handle::parse(HANDLE).expect("正当な名前であること"),
                &DisplayName::parse(DISPLAY_NAME).expect("正当な表示名であること"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");
        let port = app.local_addr().port();
        let background = app.background_probe();
        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });
        Self {
            port,
            background,
            shutdown: Some(shutdown),
            handle: Some(handle),
        }
    }

    /// **1往復させ、そのターンの裏方（抽出と提案）が終わるまで待つ。**
    ///
    /// 「A が B になる」を待つ形: 先に `servers` に届いた抽出の依頼が `extractions` 件になるのを待ち
    /// （そのターンの裏方が走った）、次に裏方の枠が空くのを待つ（終わった）。空くのだけを待つと、
    /// 裏方がまだ走り出していない回にも成立してしまう。
    async fn turn(&self, servers: &[&MockServer], extractions: usize) {
        let response = reqwest::Client::new()
            .post(format!(
                "http://127.0.0.1:{}/v1/chat/completions",
                self.port
            ))
            .json(&json!({
                "model": HANDLE,
                "messages": [{"role": "user", "content": USER_MESSAGE}],
                "stream": false,
            }))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .expect("サーバへ接続できること");
        let status = response.status();
        let text = response.text().await.expect("本文を読めること");
        assert!(status.is_success(), "{status}: {text}");

        let deadline = Instant::now() + WAIT_LIMIT;
        loop {
            let mut seen = 0;
            for server in servers {
                seen += extraction_count(server).await;
            }
            if seen >= extractions {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "抽出の依頼が {extractions} 件に届かない（いま {seen} 件）"
            );
            tokio::time::sleep(POLL_STEP).await;
        }
        while !self.background.is_idle() {
            assert!(Instant::now() < deadline, "裏方の枠が空かない");
            tokio::time::sleep(POLL_STEP).await;
        }
    }

    /// **`POST /admin/upstream/profile`**。200 でなければ落とす。
    async fn switch_to(&self, name: &str) {
        let response = reqwest::Client::new()
            .post(format!(
                "http://127.0.0.1:{}/admin/upstream/profile",
                self.port
            ))
            .json(&json!({ "name": name }))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .expect("切り替えを依頼できること");
        let status = response.status().as_u16();
        let text = response.text().await.expect("本文を読めること");
        assert_eq!(status, 200, "{name} へ切り替えられること: {text}");
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

/// その上流へ届いた抽出の依頼の件数。
async fn extraction_count(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .filter(|request| {
            String::from_utf8_lossy(&request.body)
                .contains(personacore::prompt::extraction::EXAMPLE_USER_LINE)
        })
        .count()
}

/// **種類ごとの `repeat_penalty` の並び**（重複を畳まない）。
fn values_of(seen: &[(Kind, Value)], kind: Kind) -> Vec<Value> {
    seen.iter()
        .filter(|(found, _)| *found == kind)
        .map(|(_, value)| value.clone())
        .collect()
}

/// **1つの腕を走らせ、本文と裏方の上流が受けた値を返す**（1往復）。
///
/// `sub` が `None` なら `sub` 面を書かない（裏方は本文と同じ上流へ行く）。
async fn one_turn(
    arm: &str,
    main_penalty: Option<f64>,
    sub: Option<Option<f64>>,
) -> (Vec<(Kind, Value)>, Vec<(Kind, Value)>) {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let main_upstream = upstream().await;
    let sub_upstream = upstream().await;
    let mut profile = json!({ "main": face(&main_upstream.uri(), MAIN_MODEL, main_penalty) });
    if let Some(sub_penalty) = sub {
        profile["sub"] = face(&sub_upstream.uri(), SUB_MODEL, sub_penalty);
    }
    let source = json!({ arm: profile });
    let running = Running::start(
        &config_of(&source, arm, &dir.path().join("arm.sqlite")),
        None,
    )
    .await;
    running.turn(&[&main_upstream, &sub_upstream], 1).await;
    let to_main = penalties(&main_upstream).await;
    let to_sub = penalties(&sub_upstream).await;
    running.stop().await;
    (to_main, to_sub)
}

/// **腕1: `main` 面の値が本文の要求に載る**（ADR-0058 §決定1・§決定4）。
///
/// `sub` 面は別の上流で、別の値（1.45）を書く —— **本文の要求は本文の上流にだけ届き、`main` の値（0.95）を持つ。**
#[tokio::test]
async fn arm1_the_main_faces_value_reaches_the_body_request() {
    let (to_main, to_sub) = one_turn("arm1", Some(0.95), Some(Some(1.45))).await;
    assert_eq!(
        values_of(&to_main, Kind::Body),
        vec![json!(0.95)],
        "本文の要求が main 面の値を持たない: {to_main:?}"
    );
    assert!(
        values_of(&to_main, Kind::Background).is_empty(),
        "前提: 裏方は本文の上流へ行かない: {to_main:?}"
    );
    assert!(
        values_of(&to_sub, Kind::Body).is_empty(),
        "前提: 本文は裏方の上流へ行かない: {to_sub:?}"
    );
}

/// **腕2: `sub` 面が無いとき、`main` 面の値が裏方の要求にも載る**（ADR-0058 §決定1）。
///
/// 裏方は本文と同じ上流・同じモデルへ行く。**抽出と提案のどれもが 1.05 を持つ。**
#[tokio::test]
async fn arm2_without_a_sub_face_the_main_value_reaches_the_background_too() {
    let (to_main, to_sub) = one_turn("arm2", Some(1.05), None).await;
    assert_eq!(
        values_of(&to_main, Kind::Body),
        vec![json!(1.05)],
        "{to_main:?}"
    );
    let background = values_of(&to_main, Kind::Background);
    assert!(
        !background.is_empty(),
        "前提: 裏方が本文の上流へ届いた: {to_main:?}"
    );
    assert!(
        background.iter().all(|value| *value == json!(1.05)),
        "sub 面が無いのに、裏方が main 面の値（1.05）で送っていない: {to_main:?}"
    );
    assert!(
        to_sub.is_empty(),
        "前提: 裏方の上流は使われない: {to_sub:?}"
    );
}

/// **腕3: `sub` 面に書いた値が裏方の要求に載り、本文の要求には載らない**（ADR-0058 §決定1）。
///
/// `main` 面は値を書かない —— **本文は既定の 1.1 のまま**（裏方の 1.25 を持たない）。
#[tokio::test]
async fn arm3_the_sub_faces_value_reaches_the_background_and_not_the_body() {
    let (to_main, to_sub) = one_turn("arm3", None, Some(Some(1.25))).await;
    assert_eq!(
        values_of(&to_main, Kind::Body),
        vec![json!(DEFAULT_PENALTY)],
        "本文の要求が既定の値でない（裏方の値が本文へ載った？）: {to_main:?}"
    );
    let background = values_of(&to_sub, Kind::Background);
    assert!(
        !background.is_empty(),
        "前提: 裏方が裏方の上流へ届いた: {to_sub:?}"
    );
    assert!(
        background.iter().all(|value| *value == json!(1.25)),
        "裏方の要求が sub 面の値（1.25）でない: {to_sub:?}"
    );
}

/// **腕4: `sub` 面があって `sampling` が無いとき、裏方は既定の値で、`main` 面の値を写さない**（ADR-0058 §決定1）。
///
/// `main` の値（1.3）が既定と違うので、写す実装はここで赤になる。
#[tokio::test]
async fn arm4_a_sub_face_without_sampling_uses_the_default_not_the_main_value() {
    let (to_main, to_sub) = one_turn("arm4", Some(1.3), Some(None)).await;
    assert_eq!(
        values_of(&to_main, Kind::Body),
        vec![json!(1.3)],
        "{to_main:?}"
    );
    let background = values_of(&to_sub, Kind::Background);
    assert!(
        !background.is_empty(),
        "前提: 裏方が裏方の上流へ届いた: {to_sub:?}"
    );
    assert!(
        background
            .iter()
            .all(|value| *value == json!(DEFAULT_PENALTY)),
        "sub 面に値が無いのに、裏方が既定（1.1）で送っていない（main の 1.3 を写した？）: {to_sub:?}"
    );
}

/// **腕5: 走ったままの切り替えで、3通りの遷移がそれぞれ次の往復から効く。宛先とモデルが同じで `sampling`
/// だけが違うプロファイルの間で**（ADR-0058 §決定2・§決定4）。
///
/// | 回 | プロファイル | 遷移 | 本文と裏方の値 |
/// |---|---|---|---|
/// | 1 | `tuned-a`（1.05） | 起動 | 1.05 |
/// | 2 | `tuned-b`（1.2） | 値あり → 値あり | 1.2 |
/// | 3 | `plain`（なし） | 値あり → 値なし | **1.1（既定へ戻る。持ち越さない）** |
/// | 4 | `tuned-a`（1.05） | 値なし → 値あり | 1.05 |
/// | 5 | `split`（本文 0.95・裏方 1.45） | `sub` 面を書いたプロファイルへ | 本文 0.95・**裏方 1.45** |
/// | 6 | `split-bare`（本文 0.95・裏方なし） | **裏方の面だけ**、値あり → 値なし | 本文 0.95・**裏方 1.1（本文の値を写さない）** |
///
/// **1〜4 の3つのプロファイルは宛先もモデルも同じ綴り**（`sub` 面も書かない）—— 綴りが違うと、`sampling` を
/// 比べない実装でも切り替えが起きて緑になる。`sub` 面が無いので、裏方も本文の値に追従する。
///
/// **5・6 は `sub` 面を書いた切り替え**（作業単位 A-16 の引き継ぎで足した）。1〜4 は `sub` 面を書かないので、
/// 切り替えが裏方の候補を**裏方の面の値**で組むこと（`switch::prepare` の `face_settings(sub)`）を見ていなかった。
/// `split` と `split-bare` は**本文の面が1バイトも同じで、裏方の面も宛先とモデルが同じ**（違うのは裏方の
/// `sampling` だけ）—— 6 は裏方の面で、腕5の「`sampling` だけが違う」を見る。
///
/// **切り替えの後のログ**（ADR-0058 §決定3）は、面ごとに `repeat_penalty` と `overridden` を**出た順の列**で
/// 見る（同じ行の中で読む）。言う位置を差し替えの前へ動かすと、列が1回ずつ前の値へずれて赤になる。
#[tokio::test]
async fn arm5_switching_follows_the_sampling_even_when_endpoint_and_model_stay() {
    let _serial = SWITCHING.lock().await;
    let log = install_global_log("personacore=info");
    let mark = log.mark();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let shared = upstream().await;
    let uri = shared.uri();
    let source = json!({
        "tuned-a": { "main": face(&uri, MAIN_MODEL, Some(1.05)) },
        "tuned-b": { "main": face(&uri, MAIN_MODEL, Some(1.2)) },
        "plain": { "main": face(&uri, MAIN_MODEL, None) },
        "split": {
            "main": face(&uri, MAIN_MODEL, Some(0.95)),
            "sub": face(&uri, SUB_MODEL, Some(1.45)),
        },
        "split-bare": {
            "main": face(&uri, MAIN_MODEL, Some(0.95)),
            "sub": face(&uri, SUB_MODEL, None),
        },
    });
    let path = dir.path().join("upstreams.json");
    std::fs::write(&path, source.to_string()).expect("プロファイルを置けること");
    let running = Running::start(
        &config_of(&source, "tuned-a", &dir.path().join("arm5.sqlite")),
        Some(path),
    )
    .await;

    let mut before = 0;
    for (turn, switch, body, background_expected) in [
        (1, None, json!(1.05), json!(1.05)),
        (2, Some("tuned-b"), json!(1.2), json!(1.2)),
        (
            3,
            Some("plain"),
            json!(DEFAULT_PENALTY),
            json!(DEFAULT_PENALTY),
        ),
        (4, Some("tuned-a"), json!(1.05), json!(1.05)),
        (5, Some("split"), json!(0.95), json!(1.45)),
        (6, Some("split-bare"), json!(0.95), json!(DEFAULT_PENALTY)),
    ] {
        if let Some(name) = switch {
            running.switch_to(name).await;
        }
        running.turn(&[&shared], turn).await;
        let seen = penalties(&shared).await;
        let this_turn = &seen[before..];
        before = seen.len();
        assert_eq!(
            values_of(this_turn, Kind::Body),
            vec![body.clone()],
            "{turn} 回目（{switch:?}）の本文が切り替え先の値でない: {this_turn:?}"
        );
        let background = values_of(this_turn, Kind::Background);
        assert!(
            !background.is_empty(),
            "前提: {turn} 回目の裏方が届いた: {this_turn:?}"
        );
        assert!(
            background.iter().all(|value| *value == background_expected),
            "{turn} 回目（{switch:?}）の裏方が切り替え先の値でない: {this_turn:?}"
        );
    }

    // **切り替えの後に、送る値を面ごとに言う**（ADR-0058 §決定3）。**出た順の列で見る** —— 「どこかに在る」
    // だけだと、言う位置を差し替えの前へ動かして**切り替え前の値**を `occasion="switch"` で言う実装が緑になる
    // （どの値も別の回の切り替えで正しく出るので、列の中のどこかには必ず在る）。値と `overridden` は同じ行から読む。
    let output = log.contents_since(mark);
    let r = r#"["repeat_penalty"]"#;
    for (face, expected) in [
        (
            "main",
            [
                ("1.2", r),
                ("1.1", "[]"),
                ("1.05", r),
                ("0.95", r),
                ("0.95", r),
            ],
        ),
        (
            "sub",
            [
                ("1.2", r),
                ("1.1", "[]"),
                ("1.05", r),
                ("1.45", r),
                ("1.1", "[]"),
            ],
        ),
    ] {
        let said: Vec<(Option<&str>, Option<&str>)> = output
            .lines()
            .filter(|line| {
                line.contains(SAMPLERS_LINE)
                    && has_field(line, &format!("upstream_face={face}"))
                    && has_field(line, "occasion=\"switch\"")
            })
            .map(|line| {
                (
                    token_value(line, "repeat_penalty"),
                    bracketed_field(line, "overridden"),
                )
            })
            .collect();
        let expected: Vec<(Option<&str>, Option<&str>)> = expected
            .iter()
            .map(|(value, overridden)| (Some(*value), Some(*overridden)))
            .collect();
        assert_eq!(
            said, expected,
            "切り替えの後に {face} の面の送る値を、切り替え先の値で順に言っていない"
        );
    }

    running.stop().await;
}

/// **その名前の欄の値**（`キー=値` のトークンの `値`。最初の1つ）。
fn token_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let key = format!("{key}=");
    line.split_whitespace()
        .find_map(|token| token.strip_prefix(key.as_str()))
}

/// その上流へ届いた `GET`（モデルの一覧の照会）の件数。**面を触ると、切り替えがその場で照会する**
/// （`switch::execute` の 4.）。
async fn probe_count(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.method.as_str() == "GET")
        .count()
}

/// **腕6: `sub` 面の無いプロファイルで、同じ名前へ切り替え直しても、裏方の面を触らない**（ADR-0058 §決定2）。
///
/// `sub` 面が無いと、裏方は本文の候補の複製で、**サンプラの値も本文のもの**（`switch::prepare` の `next_sub`）。
/// 比べる側（`next_sub`）がそれを本文の値で組まないと、**`sampling` を書いたプロファイルへ切り替え直すたびに
/// 裏方の面が作り直される**（測った事実が捨てられ、照会し直す）。**同じ名前への切り替え直しでは送る値は
/// 正しいまま**なので、値ではなく**「照会が増えないこと」**で見る（同じ変異は、値の無いプロファイルから値のある
/// プロファイルへの切り替えでは値も壊す —— そちらは腕5の回4が見る）。
///
/// **対照**: 別の名前（値が違う）への切り替えでは照会が増える —— 増えない形が「触っても観測できない」でないこと。
///
/// **⚠ 件数で見るこの検査の前提は2つ**: 触った面の照会は切り替えの中（`switch::execute`）で待ち終わること、
/// フィクスチャに `context_tokens` を書かない（起動直後の照会が裏で走らない）こと。`context_tokens` を足すと、
/// 起動直後の GET が数えた後に届いて偽の赤になりうる（コードからの推論。測っていない）。
#[tokio::test]
async fn arm6_switching_again_to_the_same_profile_without_a_sub_face_touches_nothing() {
    let _serial = SWITCHING.lock().await;
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let shared = upstream().await;
    let uri = shared.uri();
    let source = json!({
        "first": { "main": face(&uri, MAIN_MODEL, Some(1.35)) },
        "again": { "main": face(&uri, MAIN_MODEL, Some(0.85)) },
    });
    let path = dir.path().join("upstreams.json");
    std::fs::write(&path, source.to_string()).expect("プロファイルを置けること");
    let running = Running::start(
        &config_of(&source, "first", &dir.path().join("arm6.sqlite")),
        Some(path),
    )
    .await;

    // **対照**: 値の違うプロファイルへ切り替えると、触った面をその場で照会する。
    let before = probe_count(&shared).await;
    running.switch_to("again").await;
    let after_switch = probe_count(&shared).await;
    assert!(
        after_switch > before,
        "前提: 面を触る切り替えで照会が増える（{before} → {after_switch}）"
    );

    // **同じ名前へ切り替え直す。** 本文も裏方も何も変わらないので、どの面も触らない。
    running.switch_to("again").await;
    let after_again = probe_count(&shared).await;
    assert_eq!(
        after_again, after_switch,
        "同じプロファイルへ切り替え直しただけで、面を触って照会し直した"
    );

    // **送る値は正しいまま**（本文も裏方も 0.85）。
    running.turn(&[&shared], 1).await;
    let seen = penalties(&shared).await;
    assert_eq!(values_of(&seen, Kind::Body), vec![json!(0.85)], "{seen:?}");
    let background = values_of(&seen, Kind::Background);
    assert!(!background.is_empty(), "前提: 裏方が届いた: {seen:?}");
    assert!(
        background.iter().all(|value| *value == json!(0.85)),
        "{seen:?}"
    );

    running.stop().await;
}
