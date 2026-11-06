//! **コンテキスト長を書いた面にも、上流が生きているか・鍵が通るかを照会する**
//! （作業単位 B-66 / Issue origin#94 / ADR-0020 §4 の追記）。
//!
//! **実際の上流には繋がない**（`wiremock` の偽物と、閉じたポート）。**本番の入口（`App::bind` → `serve`）を通す。**
//!
//! # 何を見るか
//!
//! | 見張り番 | 決定 |
//! |---|---|
//! | [`a_written_face_is_checked_after_the_bind_without_moving_its_budget`] | 起動時: `bind` の中では聞かず、走り出した直後に聞く。観測は動き、予算は書いた値のまま |
//! | [`a_silent_upstream_does_not_hold_up_the_bind_of_a_written_face`] | 決定1: 黙った上流でも `bind` は照会を待たない |
//! | [`an_upstream_that_wakes_up_later_is_seen_on_check_now`] | 利用者の場面: 落ちたまま起動 → 「答えない」→ 起こす → いま確かめる → 「答えた」（両方の面） |
//! | [`a_refused_key_is_reported_right_after_startup_for_a_written_face`] | Issue 本文の「起動直後に鍵の失敗が分からない」 |
//! | [`a_second_check_while_one_is_running_waits_for_it_instead_of_asking_again`] | 決定3: 2本目を並べない（走っている回を待って返る） |
//! | [`a_foreign_host_cannot_make_the_server_check`] | 新しい口にも `Host` の検査が効く |
//!
//! **間隔（5分）ごとの照会**は `server::context` の単体テストが短い間隔で見る
//! （`an_explicit_setting_is_checked_but_never_adopted_until_it_is_lifted`）。
//!
//! # ⚠ 書いた値と、上流が申告する値を違えておく
//!
//! **同じだと「照会の答えで予算を書いた」形が観測できない**（司令塔の指定）。

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use personacore::server::{
    App, AppConfig, BindAddr, ENV_CONTEXT_TOKENS, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL,
    ENV_SUB_LLM_ENDPOINT, ENV_SUB_MODEL, ServerError, SharedBudget,
};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

/// いま確かめる口。
const CHECK_PATH: &str = "/admin/upstream/check";
/// 観測を読む口。
const UPSTREAM_PATH: &str = "/admin/upstream";

/// 本文のモデル。**抽出用とも既定とも別。**
const MAIN_MODEL: &str = "planted-b66-main-model";
/// 抽出のモデル。**本文用とも既定とも別。**
const SUB_MODEL: &str = "planted-b66-sub-model";

/// **書いたコンテキスト長**（`PERSONACORE_CONTEXT_TOKENS`）。上流の申告値とも既定とも別。
const WRITTEN_TOKENS: u32 = 12_288;
/// 本文の上流が申告する窓。**書いた値とも既定とも別**（照会の答えで予算を書いたら、ここへ動く）。
const MAIN_DECLARED: u32 = 20_480;
/// 抽出の上流が申告する窓。**上の2つとも別。**
const SUB_DECLARED: u32 = 6_144;

/// 待つ上限。超えたら落とす（緑のまま何も測らない形にしない）。
const WAIT_LIMIT: Duration = Duration::from_secs(30);
/// 待ちの刻み。
const POLL_STEP: Duration = Duration::from_millis(20);

/// 一覧の本文（`/api/v0/models`）。
fn listing(model: &str, loaded: u32) -> String {
    format!(
        r#"{{"object":"list","data":[
          {{"id":"{model}","object":"model","type":"llm","state":"loaded",
            "max_context_length":262144,"loaded_context_length":{loaded}}}]}}"#
    )
}

/// **`/api/v0/models` に答える上流**（実機の LM Studio と同じ形）。`delay` があれば黙ってから答える。
async fn upstream_reporting(model: &str, loaded: u32, delay: Option<Duration>) -> MockServer {
    let server = MockServer::start().await;
    mount_listing(&server, model, loaded, delay).await;
    server
}

async fn mount_listing(server: &MockServer, model: &str, loaded: u32, delay: Option<Duration>) {
    let mut answer =
        ResponseTemplate::new(200).set_body_raw(listing(model, loaded), "application/json");
    if let Some(delay) = delay {
        answer = answer.set_delay(delay);
    }
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(answer)
        .mount(server)
        .await;
}

/// **一覧を断る上流**（どちらの綴りにも 401）。
async fn upstream_refusing_the_key() -> MockServer {
    let server = MockServer::start().await;
    for listing in ["/api/v0/models", "/v1/models"] {
        Mock::given(matchers::method("GET"))
            .and(matchers::path(listing))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
    }
    server
}

/// その上流がモデルの一覧を聞かれた回数（どちらの綴りでも）。
async fn listing_queries(server: &MockServer) -> usize {
    server.received_requests().await.map_or(0, |requests| {
        requests
            .iter()
            .filter(|request| matches!(request.url.path(), "/api/v0/models" | "/v1/models"))
            .count()
    })
}

/// **誰も待ち受けていない宛先**（上流が落ちている状態）。返すのはそのアドレス。
fn closed_address() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("一時ポートを取れること");
    let addr = listener.local_addr().expect("アドレスを読めること");
    drop(listener);
    addr
}

/// **`App::bind`（本番の入口）で起動したサーバ。** 走らせる前の時点を観測できるように、`bind` と `serve` を分ける。
struct Bound {
    app: App,
    budget: SharedBudget,
    dir: TempDir,
}

impl Bound {
    async fn bind(env: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let pairs: Vec<(String, String)> = env
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        assert!(
            pairs.iter().any(|(name, _)| name == ENV_LLM_ENDPOINT),
            "上流は必ず偽物へ向けること（実機の LM Studio に繋がない）"
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
        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        let budget = app.budget_handle();
        Self { app, budget, dir }
    }

    fn serve(self) -> Serving {
        let addr = self.app.local_addr();
        let (shutdown, rx) = oneshot::channel();
        let app = self.app;
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });
        Serving {
            addr,
            budget: self.budget,
            client: reqwest::Client::new(),
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: self.dir,
        }
    }
}

struct Serving {
    addr: SocketAddr,
    budget: SharedBudget,
    client: reqwest::Client,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl Serving {
    async fn report(&self) -> Value {
        let response = self
            .client
            .get(format!("http://{}{UPSTREAM_PATH}", self.addr))
            .send()
            .await
            .expect("観測を読めること");
        assert_eq!(response.status(), StatusCode::OK);
        response.json().await.expect("JSON であること")
    }

    /// **`POST /admin/upstream/check`**。状態コードと本文を返す。
    async fn check_now(&self) -> (StatusCode, Value) {
        let response = self
            .client
            .post(format!("http://{}{CHECK_PATH}", self.addr))
            .timeout(WAIT_LIMIT)
            .send()
            .await
            .expect("いま確かめる口へ届くこと");
        let status = response.status();
        let text = response.text().await.expect("本文を読めること");
        let body = serde_json::from_str(&text)
            .unwrap_or_else(|err| panic!("応答が JSON であること: {text}: {err}"));
        (status, body)
    }

    /// **観測が `done` を満たすまで読み直す。** 満たさなければ落とす。
    async fn wait_for(&self, what: &str, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + WAIT_LIMIT;
        loop {
            let report = self.report().await;
            if done(&report) {
                return report;
            }
            assert!(
                Instant::now() < deadline,
                "{what} にならなかった: {report:#}"
            );
            tokio::time::sleep(POLL_STEP).await;
        }
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle
                .await
                .expect("サーバのタスクが panic しないこと")
                .expect("正常に終了すること");
        }
    }
}

/// **照会した時刻が RFC 3339 で載っていること**（`null` ではないこと）。
fn checked_at(face: &Value) -> chrono::DateTime<chrono::FixedOffset> {
    let text = face["checked_at"]
        .as_str()
        .unwrap_or_else(|| panic!("照会した時刻が載っていること: {face:#}"));
    chrono::DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|err| panic!("RFC 3339 で読めること（{text}）: {err}"))
}

/// フィクスチャの値が全部違うこと（同値だと取り違えが観測できない）。
#[test]
fn the_fixture_never_makes_two_values_equal() {
    let defaults = personacore::prompt::TokenBudget::DEFAULT_CONTEXT_TOKENS;
    for (a, b) in [
        (WRITTEN_TOKENS, MAIN_DECLARED),
        (WRITTEN_TOKENS, SUB_DECLARED),
        (MAIN_DECLARED, SUB_DECLARED),
        (WRITTEN_TOKENS, defaults),
        (MAIN_DECLARED, defaults),
    ] {
        assert_ne!(a, b);
    }
    assert_ne!(MAIN_MODEL, SUB_MODEL);
}

/// **書いた面は `bind` の中では聞かれず、走り出した直後に聞かれる。観測は動き、予算は動かない。**
///
/// | 時点 | 本文の上流への照会 | 本文の観測 |
/// |---|---|---|
/// | `bind` が返った | **0 回** | `not_checked` / `checked_at: null` |
/// | 走り出した後 | 1 回 | `answered`・名乗った名前・申告した窓（**書いた値と違う**） |
///
/// **予算は書いた値のまま**（申告値 [`MAIN_DECLARED`] へ動いたら、照会の答えで予算を書いている）。
/// 抽出の面は別の上流（値を書いていない）—— `bind` の中で照会済みで、本文の上流の数に混ざらない。
#[tokio::test]
async fn a_written_face_is_checked_after_the_bind_without_moving_its_budget() {
    let main = upstream_reporting(MAIN_MODEL, MAIN_DECLARED, None).await;
    let sub = upstream_reporting(SUB_MODEL, SUB_DECLARED, None).await;
    let written = WRITTEN_TOKENS.to_string();
    let bound = Bound::bind(&[
        (ENV_LLM_ENDPOINT, &main.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
        (ENV_CONTEXT_TOKENS, &written),
    ])
    .await;

    assert_eq!(
        listing_queries(&main).await,
        0,
        "書いた面の上流へ `bind` の中で照会している（起動を待たせる形）"
    );
    assert_eq!(
        listing_queries(&sub).await,
        1,
        "前提: 書いていない抽出の面は `bind` の中で照会すること"
    );

    let serving = bound.serve();
    let report = serving
        .wait_for("本文の面が照会された観測", |report| {
            report["main"]["check_result"] == json!("answered")
        })
        .await;
    assert_eq!(
        (
            &report["main"]["answered_name"],
            &report["main"]["queried_context_tokens"],
            &report["main"]["answered_by"],
        ),
        (
            &json!(MAIN_MODEL),
            &json!(MAIN_DECLARED),
            &json!("native-api")
        ),
        "書いた面の観測に、上流が名乗った名前と申告した窓が載ること: {report:#}"
    );
    let _ = checked_at(&report["main"]);
    assert_eq!(
        serving.budget.tokens_for_tests(),
        WRITTEN_TOKENS,
        "照会の答え（{MAIN_DECLARED}）で書いた予算を動かした"
    );
    assert_eq!(
        listing_queries(&main).await,
        1,
        "走り出した直後の照会は1回だけ（5分の間隔はまだ経っていない）"
    );

    serving.stop().await;
}

/// **黙った上流でも、書いた面の `bind` は照会を待たずに返る**（決定1「起動を遅くしない」）。
///
/// 上流は受け付けて黙る（答えるまで 30 秒）。**`bind` が返った時点で、その上流は1度も聞かれていない**
/// ——`bind` の中で照会を待つ形なら、照会の上限（5秒）を待ってから返り、聞かれた数は 1 になる。
/// 走り出した後の照会は上限で切れ、観測は「答えない」になる（時刻つき）。
#[tokio::test]
async fn a_silent_upstream_does_not_hold_up_the_bind_of_a_written_face() {
    let silent = upstream_reporting(MAIN_MODEL, MAIN_DECLARED, Some(Duration::from_secs(30))).await;
    let sub = upstream_reporting(SUB_MODEL, SUB_DECLARED, None).await;
    let written = WRITTEN_TOKENS.to_string();

    let started = Instant::now();
    let bound = Bound::bind(&[
        (ENV_LLM_ENDPOINT, &silent.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
        (ENV_CONTEXT_TOKENS, &written),
    ])
    .await;
    let took = started.elapsed();
    assert_eq!(
        listing_queries(&silent).await,
        0,
        "`bind` が黙った上流への照会を待った（{took:?} かかった）"
    );

    let serving = bound.serve();
    let report = serving
        .wait_for(
            "黙った上流が「答えない」になった観測",
            |report| report["main"]["check_result"] == json!("no_answer"),
        )
        .await;
    let _ = checked_at(&report["main"]);
    assert_eq!(
        serving.budget.tokens_for_tests(),
        WRITTEN_TOKENS,
        "答えない照会で予算を動かした"
    );

    serving.stop().await;
}

/// **利用者の場面**（Issue origin#94 のコメント1）: 上流が落ちたまま起動し、後から起こして「いま確かめる」を押す。
///
/// | 時点 | 本文（書いた面） | 抽出（書いていない面） |
/// |---|---|---|
/// | 落ちたまま起動・走り出した後 | `no_answer`（時刻つき） | `no_answer`（`bind` の中で照会済み） |
/// | 上流を起こし、いま確かめる | **`answered`** | **`answered`** |
///
/// **両方の面が変わること**を見る（片方の面しか照会しない形は、抽出の面が `no_answer` のまま残って赤）。
/// **本文の予算は書いた値のまま**、抽出の上限は照会した値へ動く（書いていない面は今の引き直しと同じ）。
#[tokio::test]
async fn an_upstream_that_wakes_up_later_is_seen_on_check_now() {
    let address = closed_address();
    let endpoint = format!("http://{address}");
    let written = WRITTEN_TOKENS.to_string();
    let serving = Bound::bind(&[
        (ENV_LLM_ENDPOINT, &endpoint),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
        (ENV_CONTEXT_TOKENS, &written),
    ])
    .await
    .serve();

    let asleep = serving
        .wait_for(
            "落ちた上流が両方の面で「答えない」になった観測",
            |report| {
                report["main"]["check_result"] == json!("no_answer")
                    && report["sub"]["check_result"] == json!("no_answer")
            },
        )
        .await;
    let asleep_at = checked_at(&asleep["main"]);

    // **同じ宛先で上流を起こす**（落ちていたポートで待ち受け直す）。
    let listener =
        std::net::TcpListener::bind(address).expect("落ちていたポートで待ち受け直せること");
    let woken = MockServer::builder().listener(listener).start().await;
    let both = format!(
        r#"{{"object":"list","data":[
          {{"id":"{MAIN_MODEL}","object":"model","type":"llm","state":"loaded",
            "max_context_length":262144,"loaded_context_length":{MAIN_DECLARED}}},
          {{"id":"{SUB_MODEL}","object":"model","type":"llm","state":"loaded",
            "max_context_length":262144,"loaded_context_length":{SUB_DECLARED}}}]}}"#
    );
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(both, "application/json"))
        .mount(&woken)
        .await;

    let (status, report) = serving.check_now().await;
    assert_eq!(
        status,
        StatusCode::OK,
        "いま確かめる口が答えること: {report:#}"
    );
    assert_eq!(
        (
            &report["main"]["check_result"],
            &report["sub"]["check_result"]
        ),
        (&json!("answered"), &json!("answered")),
        "起きた上流を、両方の面で「答えた」と言うこと: {report:#}"
    );
    assert_eq!(
        (
            &report["main"]["queried_context_tokens"],
            &report["sub"]["queried_context_tokens"]
        ),
        (&json!(MAIN_DECLARED), &json!(SUB_DECLARED)),
        "面ごとに、その面のモデルの窓が載ること: {report:#}"
    );
    assert!(
        checked_at(&report["main"]) >= asleep_at,
        "照会した時刻が新しくなっていること: {report:#}"
    );
    assert_eq!(
        serving.budget.tokens_for_tests(),
        WRITTEN_TOKENS,
        "書いた予算を、いま確かめた答えで動かした"
    );
    // **応答と `GET` は同じ形・同じ中身**（確かめた後の観測を返す）。
    assert_eq!(
        serving.report().await,
        report,
        "応答は `GET /admin/upstream` と同じ形であること"
    );

    serving.stop().await;
}

/// **鍵を断る上流なら、書いた面でも起動の直後に `unauthorized` が立つ**（Issue 本文 / origin#84 の報告）。
///
/// 着手前は、書いた面には照会しなかったので `false` のまま —— 初めての知らせは本文の依頼の
/// `upstream_unauthorized` だった。
#[tokio::test]
async fn a_refused_key_is_reported_right_after_startup_for_a_written_face() {
    let refusing = upstream_refusing_the_key().await;
    let sub = upstream_reporting(SUB_MODEL, SUB_DECLARED, None).await;
    let written = WRITTEN_TOKENS.to_string();
    let serving = Bound::bind(&[
        (ENV_LLM_ENDPOINT, &refusing.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
        (ENV_CONTEXT_TOKENS, &written),
    ])
    .await
    .serve();

    let report = serving
        .wait_for("断られた照会の観測", |report| {
            report["main"]["check_result"] != json!("not_checked")
        })
        .await;
    assert_eq!(
        (
            &report["main"]["check_result"],
            &report["main"]["unauthorized"],
            &report["sub"]["check_result"],
            &report["sub"]["unauthorized"],
        ),
        (
            &json!("refused"),
            &json!(true),
            &json!("answered"),
            &json!(false)
        ),
        "断られた面にだけ「断られた」が立つこと: {report:#}"
    );

    serving.stop().await;
}

/// **照会が走っている間に「いま確かめる」をもう1度押しても、2本目を並べない**（決定3）。
///
/// 1本目を送り、上流が聞かれたのを見てから2本目を送る（**1本目の回の最中**であることを、
/// 上流が受けた数で確かめてから）。上流は2秒黙ってから答える。
///
/// * **2本とも 200**（409 で断らない）
/// * **上流が聞かれた数は、2本で1回だけ増える**（2本目は走っている回を待って返る）
/// * **2本の応答は同じ観測**（同じ時刻）
#[tokio::test]
async fn a_second_check_while_one_is_running_waits_for_it_instead_of_asking_again() {
    let main = upstream_reporting(MAIN_MODEL, MAIN_DECLARED, Some(Duration::from_secs(2))).await;
    let sub = upstream_reporting(SUB_MODEL, SUB_DECLARED, None).await;
    let written = WRITTEN_TOKENS.to_string();
    let serving = Bound::bind(&[
        (ENV_LLM_ENDPOINT, &main.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
        (ENV_CONTEXT_TOKENS, &written),
    ])
    .await
    .serve();

    // 前提: 起動の直後の照会が終わっている（その回と取り違えない）。
    serving
        .wait_for("起動の直後の照会", |report| {
            report["main"]["check_result"] == json!("answered")
        })
        .await;
    let before = listing_queries(&main).await;

    let serving = std::sync::Arc::new(serving);
    let first = {
        let serving = std::sync::Arc::clone(&serving);
        tokio::spawn(async move { serving.check_now().await })
    };
    let deadline = Instant::now() + WAIT_LIMIT;
    while listing_queries(&main).await == before {
        assert!(Instant::now() < deadline, "1本目が上流へ届かなかった");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let (second_status, second) = serving.check_now().await;
    let (first_status, first) = first.await.expect("1本目が落ちていないこと");

    assert_eq!(
        (first_status, second_status),
        (StatusCode::OK, StatusCode::OK),
        "2本とも答えること（断らない）: {first:#} / {second:#}"
    );
    assert_eq!(
        listing_queries(&main).await - before,
        1,
        "走っている回に2本目の照会を重ねた"
    );
    assert_eq!(
        first["main"]["checked_at"], second["main"]["checked_at"],
        "2本目は走っていた回の結果を返すこと"
    );

    match std::sync::Arc::try_unwrap(serving) {
        Ok(serving) => serving.stop().await,
        Err(_) => panic!("サーバの持ち手が残っている"),
    }
}

/// **新しい口にも `Host` の検査が効く**（origin#54 B-58 / `server::host_guard`）。
///
/// 外の `Host` で叩くと 403 で、**上流へは1度も聞かない**。対照として正しい `Host` では聞く。
#[tokio::test]
async fn a_foreign_host_cannot_make_the_server_check() {
    let main = upstream_reporting(MAIN_MODEL, MAIN_DECLARED, None).await;
    let sub = upstream_reporting(SUB_MODEL, SUB_DECLARED, None).await;
    let written = WRITTEN_TOKENS.to_string();
    let serving = Bound::bind(&[
        (ENV_LLM_ENDPOINT, &main.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
        (ENV_CONTEXT_TOKENS, &written),
    ])
    .await
    .serve();
    serving
        .wait_for("起動の直後の照会", |report| {
            report["main"]["check_result"] == json!("answered")
        })
        .await;
    let before = listing_queries(&main).await;

    let status = raw_post(serving.addr, "evil.example").await;
    assert_eq!(status, 403, "外の `Host` を断ること");
    assert_eq!(
        listing_queries(&main).await,
        before,
        "断った要求で上流へ照会した"
    );

    // **対照**: 自分の `Host` なら照会する（断ったのが口の不在ではないこと）。
    let status = raw_post(serving.addr, &serving.addr.to_string()).await;
    assert_eq!(status, 200, "自分の `Host` なら答えること");
    assert_eq!(
        listing_queries(&main).await,
        before + 1,
        "対照: 照会したこと"
    );

    serving.stop().await;
}

/// **起動の直後の片面の回の最中に「いま確かめる」を押したら、待った後に自分で両方の面を照会する**
/// （レビュー是正 / `context::CheckGate` の表の3行目）。
///
/// 本文は書いた面（起動の直後に照会する）で、上流は2秒黙ってから答える。抽出は書いていない面（`bind` の中で
/// 照会済み・起動の直後には照会しない）で、別の上流。**本文の上流が聞かれたのを見てから押す**（起動の直後の回の最中）。
/// **抽出の上流の照会の数が「`bind` の中の1回 ＋ 1」になること**を値で見る —— 起動の直後の回に乗って
/// 照会せずに返る形なら、抽出の側は `bind` の中の1回のまま。
#[tokio::test]
async fn a_check_during_the_startup_check_of_one_face_still_checks_both() {
    let main = upstream_reporting(MAIN_MODEL, MAIN_DECLARED, Some(Duration::from_secs(2))).await;
    let sub = upstream_reporting(SUB_MODEL, SUB_DECLARED, None).await;
    let written = WRITTEN_TOKENS.to_string();
    let bound = Bound::bind(&[
        (ENV_LLM_ENDPOINT, &main.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
        (ENV_CONTEXT_TOKENS, &written),
    ])
    .await;
    assert_eq!(
        listing_queries(&sub).await,
        1,
        "前提: 書いていない抽出の面は `bind` の中で1回照会する"
    );
    let serving = bound.serve();

    // **起動の直後の回（本文の面だけ）の最中であること**を、上流が受けた数で確かめてから押す。
    let deadline = Instant::now() + WAIT_LIMIT;
    while listing_queries(&main).await == 0 {
        assert!(
            Instant::now() < deadline,
            "起動の直後の照会が本文の上流へ届かなかった"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let (status, report) = serving.check_now().await;
    assert_eq!(status, StatusCode::OK, "{report:#}");
    assert_eq!(
        (listing_queries(&main).await, listing_queries(&sub).await),
        (2, 2),
        "起動の直後の片面の回に乗って、抽出の面を照会せずに返った（本文は起動の直後と押した回で2回、抽出は `bind` の中と押した回で2回のはず）"
    );

    serving.stop().await;
}

/// **持ち主の「いま確かめる」が切断されて回が落ちても、その回を待っていた側は照会の結果を受け取る**
/// （レビュー是正 / `context::CheckGate` の「乗ってよいか」の節）。
///
/// 本文は書いた面で、上流は2秒黙る。起動の直後の照会が終わってから、1本目を 300 ms で諦める要求として送り
/// （上流が聞かれたのを見てから）2本目を送る。**2本目の本文の観測の時刻が、起動の直後の照会より新しいこと**
/// —— 1本目の回が落ちたのに「乗った」と見なして照会せずに返る形なら、2本目は起動の直後の時刻のまま返る。
#[tokio::test]
async fn a_check_waiting_on_a_dropped_check_still_gets_checked() {
    let main = upstream_reporting(MAIN_MODEL, MAIN_DECLARED, Some(Duration::from_secs(2))).await;
    let sub = upstream_reporting(SUB_MODEL, SUB_DECLARED, None).await;
    let written = WRITTEN_TOKENS.to_string();
    let serving = Bound::bind(&[
        (ENV_LLM_ENDPOINT, &main.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
        (ENV_CONTEXT_TOKENS, &written),
    ])
    .await
    .serve();
    let started = serving
        .wait_for("起動の直後の照会", |report| {
            report["main"]["check_result"] == json!("answered")
        })
        .await;
    let baseline = checked_at(&started["main"]);
    let before = listing_queries(&main).await;

    // **1本目は 300 ms で諦める**（クライアントが接続を閉じる → サーバの要求の future が落ちる）。
    let abandoned = {
        let client = serving.client.clone();
        let url = format!("http://{}{CHECK_PATH}", serving.addr);
        tokio::spawn(async move {
            client
                .post(url)
                .timeout(Duration::from_millis(300))
                .send()
                .await
        })
    };
    let deadline = Instant::now() + WAIT_LIMIT;
    while listing_queries(&main).await == before {
        assert!(Instant::now() < deadline, "1本目が上流へ届かなかった");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let (status, report) = serving.check_now().await;
    assert!(
        abandoned
            .await
            .expect("1本目のタスクが落ちていないこと")
            .is_err(),
        "前提: 1本目は待ちきれずに諦めたこと"
    );
    assert_eq!(status, StatusCode::OK, "{report:#}");
    assert!(
        checked_at(&report["main"]) > baseline,
        "落ちた回に乗って、照会しないまま起動の直後の観測を返した: {report:#}"
    );
    // **落ちた回の経路を通ったこと**を数で見る（再レビューの提案）—— 本文の上流は「落とされた1本目の回の1回」と
    // 「2本目が自分で走らせた回の1回」で2回聞かれる。hyper が要求の future を落とさなくなると、2本目は1本目の回に
    // 乗って時刻の検査は緑のまま、ここが 1 になって赤になる（落ちた回を見ていない形を通さない）。
    assert_eq!(
        listing_queries(&main).await - before,
        2,
        "落とされた1本目の回と、2本目が自分で走らせた回で、本文の上流が2回聞かれること"
    );

    serving.stop().await;
}

/// **生の HTTP/1.1 で `POST /admin/upstream/check` を1往復し、状態コードを返す。**
async fn raw_post(addr: SocketAddr, host: &str) -> u16 {
    let head = format!(
        "POST {CHECK_PATH} HTTP/1.1\r\nHost: {host}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let exchange = async {
        let mut socket = tokio::net::TcpStream::connect(addr).await?;
        socket.write_all(head.as_bytes()).await?;
        let mut answer = Vec::new();
        socket.read_to_end(&mut answer).await?;
        Ok::<_, std::io::Error>(answer)
    };
    let answer = tokio::time::timeout(WAIT_LIMIT, exchange)
        .await
        .expect("上限の中で答えること")
        .expect("サーバと往復できること");
    let text = String::from_utf8(answer).expect("応答は UTF-8 であること");
    text.split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("状態の行を読めない: {text}"))
}
