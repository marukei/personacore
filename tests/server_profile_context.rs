//! **上流プロファイルの面に書いたコンテキスト長**（[ADR-0054](../docs/adr/0054-context-length-in-the-upstream-profile.md) /
//! 作業単位 A-13a-1）が、**本番の入口（`App::bind` → `App::serve`）を通って効く**ことの検証。
//!
//! # 何を見るか
//!
//! | 見張り番 | つまみ / 決定 |
//! |---|---|
//! | [`the_main_value_reaches_the_real_bind_and_the_extraction_too`] | **本文の値**（ADR-0036 §決定3 の1本目）＋ `sub` 面が無ければ抽出にも使う |
//! | [`the_sub_value_reaches_the_real_bind_and_leaves_the_body_queried`] | **抽出の値**（ADR-0036 §決定3 の2本目）＋ `sub` 面があって値が無ければ照会する |
//! | [`a_profile_value_wins_over_the_environment_and_says_so_on_one_line`] | ADR-0054 §決定2（勝つ・`warn` を1本・**同じ行の中**） |
//! | [`switching_carries_the_written_values_to_the_next_profile`] | ADR-0054 §決定3 の3通り（照会→明示・明示→明示・明示→照会） |
//! | [`a_switch_to_a_profile_without_a_value_falls_back_to_the_environment`] | ADR-0054 §決定3（戻る先は起動時の環境変数） |
//!
//! # 値は全部違える
//!
//! **既定（8192）とも、抽出の落とし所（16384）とも、`PERSONACORE_CONTEXT_TOKENS` に入れる値とも、
//! 上流が申告する値とも違う**（CLAUDE.md「フィクスチャで2つの値が同じだと、その取り違えは検出できない」）。
//! 前提は各テストの冒頭の `distinct` が assert する。
//!
//! # 値の読み方
//!
//! **`App::budget_handle` / `App::sub_context_handle`**（`testing` の下だけ）——
//! **ハンドラと抽出器が読むのと同じ器**（`context::PendingRefresh` の doc）。
//! **切り替えは `POST /admin/upstream/profile` の 200 が返った時点で器へ書き終わっている**
//! （`switch::execute` の 2 と 4 は応答の前に走る）ので、待たずに読む。
//!
//! # 守っていないもの（先に書く）
//!
//! * **`main` の結線**（`config_for` が `with_profile_context` を呼ぶこと）。`main` は bin なので
//!   ここからは見えない。**ここは `main` と同じ2段（`overlay_profile` → `with_profile_context`）を
//!   自分で組んで `App::bind` へ渡す。**
//! * **5分ごとの引き直しが、切り替えの後に止まる／戻ること**——5分を待てないので、
//!   `server::context` の単体テスト（短い間隔）が見る。
//! * **切り替えの置き場所は `App::with_profiles_file_for_tests` で渡す**（`run_with_upstreams` が
//!   `Startup` から写す1行と同じ代入）。`run_with_upstreams` を通る切り替えは `tests/upstream_launch.rs`。

use std::path::{Path, PathBuf};
use std::time::Duration;

use personacore::server::{
    App, AppConfig, BindAddr, DEFAULT_LOG_FILTER, ENV_CONTEXT_TOKENS, ENV_LLM_ENDPOINT,
    ENV_MAIN_MODEL, ENV_SUB_MODEL, ServerError, SharedBudget, SharedSubContext, overlay_profile,
    parse_profiles,
};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

mod common;

use common::{SharedBuffer, has_field, install_global_log};

/// 本文に指定するモデル。**抽出用とも既定とも別の綴り。**
const MAIN_MODEL: &str = "ministral-3-14b-reasoning-2512";
/// 抽出に指定するモデル。**本文用とも既定とも別の綴り。**
const SUB_MODEL: &str = "ministral-3-3b-reasoning-2512";

/// 上流が本文のモデルについて申告する窓。
const QUERIED_MAIN: u32 = 20_480;
/// 上流が抽出のモデルについて申告する窓。
const QUERIED_SUB: u32 = 6_144;
/// 本文の面に書く値。
const WRITTEN_MAIN: u32 = 28_672;
/// 抽出の面に書く値。
const WRITTEN_SUB: u32 = 10_240;
/// 切り替え先で本文の面に書く、別の値。
const REWRITTEN_MAIN: u32 = 24_576;
/// 切り替え先で抽出の面に書く、別の値。
const REWRITTEN_SUB: u32 = 14_336;
/// `PERSONACORE_CONTEXT_TOKENS` に入れる値。
const ENVIRONMENT: u32 = 12_288;
/// **環境変数と同じ値をプロファイルにも書く腕だけが使う値**（ぶつからない対照）。
const AGREED: u32 = 26_624;
/// **ぶつかる腕だけが本文の面に書く値**（ログは1プロセスで共有なので、行を拾う値は腕ごとに違える）。
const CONFLICTING: u32 = 30_720;
/// **切り替えで環境変数へ戻る腕だけが本文の面に書く値**（同上）。
const RETURNING: u32 = 22_528;

/// 既定の予算（`TokenBudget::DEFAULT_CONTEXT_TOKENS`）。**写しではなく前提の検査に使う。**
fn default_context() -> u32 {
    personacore::prompt::TokenBudget::DEFAULT_CONTEXT_TOKENS
}

/// 抽出の落とし所（**上限からの逆算** / 落とし所の綴りをテスト側に写さない）。
fn fallback_sub_context() -> u32 {
    personacore::server::extraction_prompt_budget_for_tests() * 2
}

/// **ぶつかったときの行の本文**（`main.rs` の `warn_on_conflicts` と同じ文面）。
const CONFLICT_MARKER: &str = "環境変数よりも選んだ上流プロファイルを優先します";

/// **値が全部違うこと。**
fn distinct(values: &[u32]) {
    let mut all = values.to_vec();
    all.extend([default_context(), fallback_sub_context()]);
    let mut sorted = all.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), all.len(), "前提: 値が全部違うこと: {all:?}");
}

fn shared_log() -> &'static SharedBuffer {
    install_global_log(DEFAULT_LOG_FILTER)
}

/// **2つのモデルを、それぞれ別の窓で申告する上流**（`/api/v0/models`）。実機には繋がない。
async fn upstream_reporting_both() -> MockServer {
    let entry = |id: &str, loaded: u32| {
        format!(
            r#"{{"id": "{id}", "object": "model", "type": "llm", "state": "loaded",
                "max_context_length": 262144, "loaded_context_length": {loaded}}}"#
        )
    };
    let body = format!(
        r#"{{"object": "list", "data": [{}, {}]}}"#,
        entry(MAIN_MODEL, QUERIED_MAIN),
        entry(SUB_MODEL, QUERIED_SUB)
    );
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;
    server
}

/// `/api/v0/models` を叩いた回数。
async fn queries_to(server: &MockServer) -> usize {
    server.received_requests().await.map_or(0, |requests| {
        requests
            .iter()
            .filter(|request| request.url.path() == "/api/v0/models")
            .count()
    })
}

/// **面の1つ**（`context_tokens` は書くときだけ）。
fn face(endpoint: &str, model: &str, context_tokens: Option<u32>) -> serde_json::Value {
    let mut face = serde_json::json!({ "endpoint": endpoint, "model": model });
    if let Some(tokens) = context_tokens {
        face["context_tokens"] = serde_json::json!(tokens);
    }
    face
}

/// **2面のプロファイル**（`sub` を書くときだけ `Some`）。
fn profile(endpoint: &str, main: Option<u32>, sub: Option<Option<u32>>) -> serde_json::Value {
    let mut profile = serde_json::json!({ "main": face(endpoint, MAIN_MODEL, main) });
    if let Some(sub) = sub {
        profile["sub"] = face(endpoint, SUB_MODEL, sub);
    }
    profile
}

/// **`main` と同じ2段で設定を組む**（`overlay_profile` → `with_profile_context`）。
///
/// `chosen` が `None` なら、プロファイルを選ばない起動（環境変数だけ）。
fn config_of(
    upstream: &str,
    environment: Option<u32>,
    chosen: Option<(&serde_json::Value, &str)>,
    db_path: &Path,
) -> AppConfig {
    let mut pairs: Vec<(String, String)> = vec![
        (ENV_LLM_ENDPOINT.to_owned(), upstream.to_owned()),
        (ENV_MAIN_MODEL.to_owned(), MAIN_MODEL.to_owned()),
        (ENV_SUB_MODEL.to_owned(), SUB_MODEL.to_owned()),
    ];
    if let Some(tokens) = environment {
        pairs.push((ENV_CONTEXT_TOKENS.to_owned(), tokens.to_string()));
    }
    let base = move |key: &str| {
        pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    };

    let config = match chosen {
        None => AppConfig::from_lookup(base).expect("設定を組み立てられること"),
        Some((source, name)) => {
            let profiles =
                parse_profiles(&source.to_string(), None).expect("植えたプロファイルが読めること");
            let settings = profiles.select(name).expect("在ること").settings();
            AppConfig::from_lookup(overlay_profile(&settings, base))
                .expect("設定を組み立てられること")
                .with_profile_context(&settings)
                .expect("書いた値を載せられること")
        }
    };
    config
        .with_bind(BindAddr::loopback(0))
        .with_database_path(db_path)
}

/// **`App::bind`（本番の入口）で起動し、走らせる。**
struct Running {
    port: u16,
    budget: SharedBudget,
    sub: SharedSubContext,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
}

impl Running {
    /// 起動して、**走らせる前に**器を控える。`profiles` があれば走ったまま切り替えられる。
    async fn start(config: &AppConfig, profiles: Option<PathBuf>) -> Self {
        Self::start_counting(config, profiles, None).await.0
    }

    /// **[`Self::start`] と同じ起動で、`bind` が返った時点（走らせる前）の照会の回数も返す**（作業単位 B-66）。
    ///
    /// **書いた面も、走り出した直後に1度照会する**（生きているか・鍵が通るか / Issue origin#94）ので、
    /// 「起動時に照会しない」は **`bind` の中で照会しない**の意味になった。走らせた後に数えると、
    /// その1回が入るかどうかが走り出しとの競走で決まる。
    async fn start_counting(
        config: &AppConfig,
        profiles: Option<PathBuf>,
        upstream: Option<&MockServer>,
    ) -> (Self, usize) {
        // **`bind_with_client` ではない。** ここが本番と同じ入口であることが要点。
        let mut app = App::bind(config).await.expect("待ち受けを開始できること");
        let queried_in_bind = match upstream {
            Some(server) => queries_to(server).await,
            None => 0,
        };
        if let Some(path) = profiles {
            app = app.with_profiles_file_for_tests(path);
        }
        let port = app.local_addr().port();
        let budget = app.budget_handle();
        let sub = app.sub_context_handle();
        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });
        (
            Self {
                port,
                budget,
                sub,
                shutdown: Some(shutdown),
                handle: Some(handle),
            },
            queried_in_bind,
        )
    }

    /// **いまの本文の予算と抽出の窓**（ハンドラと抽出器が読むのと同じ器）。
    fn windows(&self) -> (u32, u32) {
        (self.budget.tokens_for_tests(), self.sub.tokens_for_tests())
    }

    /// **`POST /admin/upstream/profile`**。200 でなければ落とす。
    async fn switch_to(&self, name: &str) {
        let response = reqwest::Client::new()
            .post(format!(
                "http://127.0.0.1:{}/admin/upstream/profile",
                self.port
            ))
            .json(&serde_json::json!({ "name": name }))
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

fn write_profiles(dir: &TempDir, source: &serde_json::Value) -> PathBuf {
    let path = dir.path().join("upstreams.json");
    std::fs::write(&path, source.to_string()).expect("プロファイルを置けること");
    path
}

// ----------------------------------------------------------------------
// 起動（ADR-0054 §決定1・2）
// ----------------------------------------------------------------------

/// **本文の面に書いた値が、本番の入口を通って本文の予算になる**
/// （ADR-0036 §決定3 —— つまみ「本文の値」の見張り番）。
///
/// **`sub` 面を書いていないので、抽出の窓も同じ値になる**（ADR-0054 §決定1）。
/// **書いた面は `bind` の中で照会しない**——`bind` が返った時点の照会が 0 回であることも見る。
///
/// > **作業単位 B-66 で「起動時に照会しない」を「`bind` の中で照会しない」へ狭めた**（司令塔の決定1）。
/// > **書いた面も、走り出した直後に1度だけ観測のために照会する**（生きているか・鍵が通るか /
/// > Issue origin#94）——その照会は予算を動かさない（下の `windows` は書いた値のまま）。
/// > 走り出した後の照会は `tests/server_upstream_check.rs` が見る。
///
/// # 対照（同じ上流・値を書かないプロファイル）
///
/// **上流は窓を申告しているので、書かなければ照会した値になり、照会は 2 回**
/// （本文と抽出で1回ずつ）。**書いた腕が「照会しない」のは、上流が黙っているからではない。**
#[tokio::test]
async fn the_main_value_reaches_the_real_bind_and_the_extraction_too() {
    distinct(&[QUERIED_MAIN, QUERIED_SUB, WRITTEN_MAIN]);
    let dir = tempfile::tempdir().expect("一時ディレクトリ");

    let upstream = upstream_reporting_both().await;
    let source = serde_json::json!({
        "written": profile(&upstream.uri(), Some(WRITTEN_MAIN), None),
    });
    let (running, queried_in_bind) = Running::start_counting(
        &config_of(
            &upstream.uri(),
            None,
            Some((&source, "written")),
            &dir.path().join("written.sqlite"),
        ),
        None,
        Some(&upstream),
    )
    .await;
    assert_eq!(
        running.windows(),
        (WRITTEN_MAIN, WRITTEN_MAIN),
        "本文の値が本文の予算に、sub 面が無いので抽出の窓にもなること"
    );
    assert_eq!(
        queried_in_bind, 0,
        "値を書いた面へ `bind` の中で照会している"
    );
    running.stop().await;

    // **対照**: 同じ上流で、値を書かないプロファイル。
    let upstream = upstream_reporting_both().await;
    let source = serde_json::json!({ "queried": profile(&upstream.uri(), None, None) });
    let (running, queried_in_bind) = Running::start_counting(
        &config_of(
            &upstream.uri(),
            None,
            Some((&source, "queried")),
            &dir.path().join("queried.sqlite"),
        ),
        None,
        Some(&upstream),
    )
    .await;
    // **sub 面が無いので、抽出のモデルも本文と同じ綴り**（`ProfileSettings::sub_model`）。
    assert_eq!(
        running.windows(),
        (QUERIED_MAIN, QUERIED_MAIN),
        "前提: 書かなければ照会した値になること"
    );
    assert_eq!(
        queried_in_bind, 2,
        "前提: 書かなければ両面とも `bind` の中で照会すること"
    );
    running.stop().await;
}

/// **抽出の面に書いた値が、本番の入口を通って抽出の窓になる**
/// （ADR-0036 §決定3 —— つまみ「抽出の値」の見張り番）。
///
/// **本文の面は書いていないので、照会した値のまま**（照会は本文の1回だけ）。
///
/// # 2つ目の腕: `sub` 面があって値が無ければ、本文の値を写さずに照会する
///
/// **本文に書いた値（[`WRITTEN_MAIN`]）が抽出の窓になっていたら赤**（ADR-0054 §決定1）。
#[tokio::test]
async fn the_sub_value_reaches_the_real_bind_and_leaves_the_body_queried() {
    distinct(&[QUERIED_MAIN, QUERIED_SUB, WRITTEN_MAIN, WRITTEN_SUB]);
    let dir = tempfile::tempdir().expect("一時ディレクトリ");

    let upstream = upstream_reporting_both().await;
    let source = serde_json::json!({
        "sub_written": profile(&upstream.uri(), None, Some(Some(WRITTEN_SUB))),
    });
    let (running, queried_in_bind) = Running::start_counting(
        &config_of(
            &upstream.uri(),
            None,
            Some((&source, "sub_written")),
            &dir.path().join("sub-written.sqlite"),
        ),
        None,
        Some(&upstream),
    )
    .await;
    assert_eq!(
        running.windows(),
        (QUERIED_MAIN, WRITTEN_SUB),
        "抽出の値が抽出の窓に、本文は照会した値になること"
    );
    // **`bind` の中の照会**（作業単位 B-66 —— 書いた面は走り出した直後に観測のために聞く）。
    assert_eq!(
        queried_in_bind, 1,
        "`bind` の中の照会は本文の1回だけ（値を書いた抽出の面へは聞かない）"
    );
    running.stop().await;

    let upstream = upstream_reporting_both().await;
    let source = serde_json::json!({
        "sub_unwritten": profile(&upstream.uri(), Some(WRITTEN_MAIN), Some(None)),
    });
    let (running, queried_in_bind) = Running::start_counting(
        &config_of(
            &upstream.uri(),
            None,
            Some((&source, "sub_unwritten")),
            &dir.path().join("sub-unwritten.sqlite"),
        ),
        None,
        Some(&upstream),
    )
    .await;
    assert_eq!(
        running.windows(),
        (WRITTEN_MAIN, QUERIED_SUB),
        "sub 面があって値が無ければ、本文の値を写さずに照会すること"
    );
    assert_eq!(
        queried_in_bind, 1,
        "`bind` の中の照会は抽出の1回だけ（値を書いた本文の面へは聞かない）"
    );
    running.stop().await;
}

/// **面の値は環境変数に勝ち、ぶつかったら `warn` を1本出す**（ADR-0054 §決定2）。
///
/// **同じ行の中で**、何がぶつかったか（`setting`）・両方の値・深刻度を見る
/// （CLAUDE.md「ログを見る見張り番は、同じ行の中で見る」）。
/// **行は値（`from_profile=`）で拾う**——[`CONFLICTING`] はこの腕だけの値なので、
/// 他のテストの行を拾わない（**ログは1プロセスで共有**。他の腕も環境変数とぶつかる起動をする）。
///
/// # 対照: 同じ値なら言わない
///
/// **[`AGREED`] は環境変数とプロファイルの両方に入れる腕だけの値**なので、
/// その値を名乗る行が無いことを、他のテストに邪魔されずに言える。
#[tokio::test]
async fn a_profile_value_wins_over_the_environment_and_says_so_on_one_line() {
    distinct(&[QUERIED_MAIN, QUERIED_SUB, CONFLICTING, ENVIRONMENT, AGREED]);
    let log = shared_log();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");

    let upstream = upstream_reporting_both().await;
    let source = serde_json::json!({
        "written": profile(&upstream.uri(), Some(CONFLICTING), Some(None)),
        "agreed": profile(&upstream.uri(), Some(AGREED), Some(None)),
    });
    let running = Running::start(
        &config_of(
            &upstream.uri(),
            Some(ENVIRONMENT),
            Some((&source, "written")),
            &dir.path().join("conflict.sqlite"),
        ),
        None,
    )
    .await;
    assert_eq!(
        running.windows().0,
        CONFLICTING,
        "プロファイルの値が環境変数に勝つこと"
    );
    running.stop().await;

    let logged = log.contents();
    let line = logged
        .lines()
        .find(|line| has_field(line, &format!("from_profile={CONFLICTING}")))
        .unwrap_or_else(|| panic!("ぶつかったことを言っていない: {logged}"));
    assert!(line.contains(CONFLICT_MARKER), "文面: {line}");
    assert!(line.contains("WARN"), "warn で言うこと: {line}");
    assert!(
        // **`main.rs` の `warn_on_conflicts` と同じ書き方**（`&str` を素で渡すので引用符つき）。
        has_field(line, &format!("setting={ENV_CONTEXT_TOKENS:?}")),
        "何がぶつかったかを同じ行で言うこと: {line}"
    );
    assert!(
        has_field(line, &format!("from_environment={ENVIRONMENT}")),
        "環境変数の値を同じ行で言うこと: {line}"
    );
    assert_eq!(
        logged
            .lines()
            .filter(|line| has_field(line, &format!("from_profile={CONFLICTING}")))
            .count(),
        1,
        "1本だけ言うこと: {logged}"
    );

    // **対照**: 同じ値なら言わない（勝ち負けが無い）。
    let running = Running::start(
        &config_of(
            &upstream.uri(),
            Some(AGREED),
            Some((&source, "agreed")),
            &dir.path().join("agreed.sqlite"),
        ),
        None,
    )
    .await;
    assert_eq!(running.windows().0, AGREED, "前提: 起動できること");
    running.stop().await;
    let logged = log.contents();
    assert!(
        !logged
            .lines()
            .any(|line| has_field(line, &format!("from_profile={AGREED}"))),
        "同じ値なのに、ぶつかったと言っている: {logged}"
    );
}

// ----------------------------------------------------------------------
// 走ったままの切り替え（ADR-0054 §決定3）
// ----------------------------------------------------------------------

/// **走ったまま切り替えると、明示の有無と値が切り替え先に追従する**（3通り）。
///
/// | 切り替え | 前 | 後 |
/// |---|---|---|
/// | 照会 → 明示 | 照会した値 | 書いた値 |
/// | 明示 → 明示 | 書いた値 | **別の**書いた値 |
/// | 明示 → 照会 | 書いた値 | 照会した値（切り替えがその場で測り直す） |
///
/// **どの段も、前の値と後の値が違う**——「A が B になる」は A のままでは成立しない。
/// **明示 → 明示は宛先もモデルも同じで、値だけが違う**——
/// 「宛先と綴りが同じなら触らない」（`switch::plan_switch`）に値が入っていなければ、ここで赤になる。
#[tokio::test]
async fn switching_carries_the_written_values_to_the_next_profile() {
    distinct(&[
        QUERIED_MAIN,
        QUERIED_SUB,
        WRITTEN_MAIN,
        WRITTEN_SUB,
        REWRITTEN_MAIN,
        REWRITTEN_SUB,
    ]);
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let upstream = upstream_reporting_both().await;
    let endpoint = upstream.uri();
    let profiles = write_profiles(
        &dir,
        &serde_json::json!({
            "queried": profile(&endpoint, None, Some(None)),
            "written": profile(&endpoint, Some(WRITTEN_MAIN), Some(Some(WRITTEN_SUB))),
            "rewritten": profile(&endpoint, Some(REWRITTEN_MAIN), Some(Some(REWRITTEN_SUB))),
        }),
    );

    // **起動は環境変数だけ**（「queried」と同じ宛先と綴り）。
    let running = Running::start(
        &config_of(&endpoint, None, None, &dir.path().join("switch.sqlite")),
        Some(profiles),
    )
    .await;
    assert_eq!(
        running.windows(),
        (QUERIED_MAIN, QUERIED_SUB),
        "前提: 起動時は照会した値"
    );

    running.switch_to("written").await;
    assert_eq!(
        running.windows(),
        (WRITTEN_MAIN, WRITTEN_SUB),
        "照会 → 明示: 書いた値になること"
    );

    running.switch_to("rewritten").await;
    assert_eq!(
        running.windows(),
        (REWRITTEN_MAIN, REWRITTEN_SUB),
        "明示 → 明示: 切り替え先の値になること（値だけが違う切り替えを触らずに済ませていないか）"
    );

    running.switch_to("queried").await;
    assert_eq!(
        running.windows(),
        (QUERIED_MAIN, QUERIED_SUB),
        "明示 → 照会: 照会した値に戻ること"
    );

    running.stop().await;
}

/// **切り替え先に値が無ければ、起動時の環境変数の値に戻る**（ADR-0054 §決定3 の最後）。
///
/// **起動はプロファイルの値**（環境変数に勝つ）。**値の無いプロファイルへ切り替えると、
/// 照会した値ではなく環境変数の値**になる。**抽出には環境変数が無いので、照会した値になる。**
/// **戻ってもう一度書いた方へ切り替えると、また書いた値**（行って戻る）。
///
/// # 切り替えでも、ぶつかったら言う（ADR-0054 §決定3「決定2 の順位を切り替え後にも当てはめる」）
///
/// **[`RETURNING`] はこの腕だけの値**なので、それを名乗る `warn` の行を数えられる——
/// 起動で1本、書いた方へ戻す切り替えで**もう1本**（値の無い方への切り替えでは言わない）。
#[tokio::test]
async fn a_switch_to_a_profile_without_a_value_falls_back_to_the_environment() {
    distinct(&[
        QUERIED_MAIN,
        QUERIED_SUB,
        RETURNING,
        WRITTEN_SUB,
        ENVIRONMENT,
    ]);
    let log = shared_log();
    let conflicts = || {
        log.contents()
            .lines()
            .filter(|line| {
                has_field(line, &format!("from_profile={RETURNING}"))
                    && has_field(line, &format!("from_environment={ENVIRONMENT}"))
                    && line.contains(CONFLICT_MARKER)
            })
            .count()
    };
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let upstream = upstream_reporting_both().await;
    let endpoint = upstream.uri();
    let source = serde_json::json!({
        "queried": profile(&endpoint, None, Some(None)),
        "written": profile(&endpoint, Some(RETURNING), Some(Some(WRITTEN_SUB))),
    });
    let profiles = write_profiles(&dir, &source);

    let running = Running::start(
        &config_of(
            &endpoint,
            Some(ENVIRONMENT),
            Some((&source, "written")),
            &dir.path().join("fallback.sqlite"),
        ),
        Some(profiles),
    )
    .await;
    assert_eq!(
        running.windows(),
        (RETURNING, WRITTEN_SUB),
        "前提: 起動時はプロファイルの値"
    );
    assert_eq!(conflicts(), 1, "前提: 起動でぶつかったと1本言うこと");

    running.switch_to("queried").await;
    assert_eq!(
        running.windows(),
        (ENVIRONMENT, QUERIED_SUB),
        "値の無いプロファイルへ切り替えたら、本文は環境変数の値・抽出は照会した値になること"
    );
    assert_eq!(
        conflicts(),
        1,
        "値の無い方へ切り替えたのに、ぶつかったと言っている"
    );

    running.switch_to("written").await;
    assert_eq!(
        running.windows(),
        (RETURNING, WRITTEN_SUB),
        "書いた方へ戻せば、また書いた値になること"
    );
    assert_eq!(
        conflicts(),
        2,
        "書いた方へ戻す切り替えでも、ぶつかったと1本言うこと"
    );

    running.stop().await;
}
