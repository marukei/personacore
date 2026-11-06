//! **鍵を平文の `http://` で別の機へ送るとき**（[ADR-0056](../docs/adr/0056-a-key-for-the-upstream.md)
//! §決定5 / 作業単位 A-13b-2 / Issue origin#85）が、**本番の入口（`App::bind` → `App::serve`）と走ったままの
//! 切り替えを通って**効き、`GET /admin/upstream` の面に出ることの検証。
//!
//! **実際の上流にも、実際の DNS にも繋がない**（`wiremock` の偽物と、差し替えた名前解決）。
//!
//! # 何を見るか
//!
//! | 見張り番 | 決定 |
//! |---|---|
//! | [`the_upstream_screen_names_how_the_key_crosses_plain_http`] | 面の `key_over_plain_http`（ローカル・承認なし・承認あり）。承認の無い面は、起動時の照会も偽の上流に届かない |
//! | [`a_face_that_states_its_context_still_names_its_address_verdict`] | 照会しない面（窓を明示）でも、IP アドレスの宛先の判定が面に出る（本文と裏方） |
//! | [`the_approval_belongs_to_its_own_face_at_launch_and_on_switch`] | 承認は面ごと（本文の承認を裏方へ貸さない）。起動時と切り替えの両方 |
//! | [`switching_to_a_profile_with_the_approval_carries_the_approval`] | 宛先・モデル・鍵が同じで承認だけが違うプロファイルへの切り替えでも、承認が追従する |
//!
//! # 守っていないもの（先に書く）
//!
//! * **`main` の結線**（`config_for` が `with_profile_keys` を呼ぶこと）は bin なので、ここからは見えない
//!   （`tests/server_profile_keys.rs` と同じ。ここは `main` と同じ段を自分で組む）。
//! * **チャットの応答（`upstream_key_not_approved`）**は `tests/server_chat.rs` / `tests/server_chat_stream.rs` が
//!   偽の `LlmClient` で見る。ここで観測するのは起動時と切り替えのときのモデルの照会だけである。

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use personacore::llm::{HostLookup, HostResolver};
use personacore::server::{
    App, AppConfig, BindAddr, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL, ENV_SUB_MODEL, ServerError,
    overlay_profile, parse_profiles,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

const MAIN_MODEL: &str = "planted-plain-main-model";
const SUB_MODEL: &str = "planted-plain-sub-model";
/// 本文の面の鍵（囮）。
const MAIN_KEY: &str = "pc-plain-bind-KEY-9e8d7c6b";
/// 偽の名前（`.test` は実在しない）。**実際の DNS には聞かない。**
const HOST: &str = "upstream-bind-a13b2.test";

/// **返すアドレスを決めた名前解決**（実際の DNS に依らない）。
#[derive(Debug)]
struct FixedResolver(Mutex<Vec<IpAddr>>);

impl FixedResolver {
    fn answering(addresses: &[&str]) -> Arc<Self> {
        Arc::new(Self(Mutex::new(
            addresses
                .iter()
                .map(|raw| raw.parse().expect("正当なアドレス"))
                .collect(),
        )))
    }
}

impl HostResolver for FixedResolver {
    fn lookup(&self, _host: &str) -> HostLookup {
        let answer = self.0.lock().expect("錠").clone();
        Box::pin(async move { Ok(answer) })
    }
}

/// **モデルの一覧に答える上流**（`/api/v0/models`）。
async fn listing_upstream() -> MockServer {
    let body = format!(
        r#"{{"object": "list", "data": [
            {{"id": "{MAIN_MODEL}", "object": "model", "type": "llm", "state": "loaded",
              "max_context_length": 262144, "loaded_context_length": 20480}},
            {{"id": "{SUB_MODEL}", "object": "model", "type": "llm", "state": "loaded",
              "max_context_length": 262144, "loaded_context_length": 6144}}]}}"#
    );
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;
    server
}

/// **届いた要求の `Authorization`**（届いた順）。
async fn authorizations(server: &MockServer) -> Vec<Option<String>> {
    server
        .received_requests()
        .await
        .expect("記録が有効であること")
        .iter()
        .map(|request| {
            request
                .headers
                .get("authorization")
                .map(|value| value.to_str().expect("ASCII").to_owned())
        })
        .collect()
}

fn bearer(key: &str) -> Option<String> {
    Some(format!("Bearer {key}"))
}

/// **偽の名前で、偽の上流のポートを指す宛先。**
fn named(upstream: &MockServer) -> String {
    format!("http://{HOST}:{}", upstream.address().port())
}

/// **鍵を書いた面**（承認は書くときだけ）。
fn keyed_face(endpoint: &str, allow: Option<bool>) -> Value {
    let mut face = json!({ "endpoint": endpoint, "model": MAIN_MODEL, "api_key": MAIN_KEY });
    if let Some(allow) = allow {
        face["allow_key_over_plain_http"] = json!(allow);
    }
    face
}

/// **`main` と同じ段で設定を組む**（`overlay_profile` → `with_profile_context` → `with_profile_keys`）。
/// **名前解決はテストの口へ差し替える。**
fn config_of(
    source: &Value,
    name: &str,
    resolver: &Arc<FixedResolver>,
    db_path: &Path,
) -> AppConfig {
    let pairs: Vec<(String, String)> = vec![
        (ENV_LLM_ENDPOINT.to_owned(), "http://127.0.0.1:9".to_owned()),
        (ENV_MAIN_MODEL.to_owned(), MAIN_MODEL.to_owned()),
        (ENV_SUB_MODEL.to_owned(), SUB_MODEL.to_owned()),
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
    let resolver: Arc<dyn HostResolver> = Arc::clone(resolver) as Arc<dyn HostResolver>;
    AppConfig::from_lookup(overlay_profile(&settings, base))
        .expect("設定を組み立てられること")
        .with_profile_context(&settings)
        .expect("書いた値を載せられること")
        .with_profile_keys(&settings)
        .with_host_resolver_for_tests(resolver)
        .with_bind(BindAddr::loopback(0))
        .with_database_path(db_path)
}

/// **`App::bind`（本番の入口）で起動し、走らせる。**
struct Running {
    port: u16,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
}

impl Running {
    async fn start(config: &AppConfig, profiles: Option<PathBuf>) -> Self {
        let mut app = App::bind(config).await.expect("待ち受けを開始できること");
        if let Some(path) = profiles {
            app = app.with_profiles_file_for_tests(path);
        }
        let port = app.local_addr().port();
        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });
        Self {
            port,
            shutdown: Some(shutdown),
            handle: Some(handle),
        }
    }

    /// **`GET /admin/upstream`**（本文の文字列と JSON の両方）。
    async fn upstream_report(&self) -> (String, Value) {
        let text = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{}/admin/upstream", self.port))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .expect("管理 API へ繋がること")
            .text()
            .await
            .expect("本文を読めること");
        let json = serde_json::from_str(&text)
            .unwrap_or_else(|err| panic!("JSON として読めること: {err} / {text}"));
        (text, json)
    }

    /// **`POST /admin/upstream/profile`**。200 でなければ落とす。応答の JSON を返す。
    async fn switch_to(&self, name: &str) -> Value {
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
        serde_json::from_str(&text).expect("JSON")
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

fn write_profiles(dir: &TempDir, source: &Value) -> PathBuf {
    let path = dir.path().join("upstreams.json");
    std::fs::write(&path, source.to_string()).expect("プロファイルを置けること");
    path
}

/// **面の `key_over_plain_http`。** 欄が無ければ `None`（警告が無い面は欄ごと省く）。
fn plain_http_of<'a>(report: &'a Value, face: &str) -> Option<&'a Value> {
    report[face].get("key_over_plain_http")
}

/// **面に、鍵を平文の `http://` で送る扱いが出る。承認の無い面は、起動時の照会も偽の上流に届かない**
/// （ADR-0056 §決定5 / Issue origin#85）。
///
/// | 腕 | 名前解決（本文の面） | 承認 | 本文の面 | 本文の上流に届く |
/// |---|---|---|---|---|
/// | `local` | `127.0.0.1`・`192.168.0.7` | なし | `local_network` | 鍵つきで届く |
/// | `outside` | `127.0.0.1`・`203.0.113.5` | なし | `not_approved` | **何も届かない** |
/// | `approved` | `127.0.0.1`・`203.0.113.5` | あり | `approved` | 鍵つきで届く |
///
/// **裏方の面は鍵の無いループバック**（別の偽の上流）—— 欄が無いこと（扱いを変えない）と、面を取り違えないことを見る。
#[tokio::test]
async fn the_upstream_screen_names_how_the_key_crosses_plain_http() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    for (arm, answer, allow, expected, reaches) in [
        (
            "local",
            &["127.0.0.1", "192.168.0.7"][..],
            None,
            "local_network",
            true,
        ),
        (
            "outside",
            &["127.0.0.1", "203.0.113.5"][..],
            None,
            "not_approved",
            false,
        ),
        (
            "approved",
            &["127.0.0.1", "203.0.113.5"][..],
            Some(true),
            "approved",
            true,
        ),
    ] {
        let main_upstream = listing_upstream().await;
        let sub_upstream = listing_upstream().await;
        let source = json!({
            arm: {
                "main": keyed_face(&named(&main_upstream), allow),
                "sub": { "endpoint": sub_upstream.uri(), "model": SUB_MODEL },
            }
        });
        let resolver = FixedResolver::answering(answer);
        let running = Running::start(
            &config_of(
                &source,
                arm,
                &resolver,
                &dir.path().join(format!("{arm}.sqlite")),
            ),
            None,
        )
        .await;

        let (text, report) = running.upstream_report().await;
        assert_eq!(
            plain_http_of(&report, "main"),
            Some(&json!(expected)),
            "{arm}: {report}"
        );
        assert_eq!(
            plain_http_of(&report, "sub"),
            None,
            "{arm}: 鍵の無いループバックの面に欄が出ている: {report}"
        );
        let main_seen = authorizations(&main_upstream).await;
        if reaches {
            assert!(
                !main_seen.is_empty() && main_seen.iter().all(|h| *h == bearer(MAIN_KEY)),
                "{arm}: 本文の上流へ鍵つきで届くこと: {main_seen:?}"
            );
        } else {
            assert_eq!(
                main_seen,
                Vec::<Option<String>>::new(),
                "{arm}: 承認の無い送信が偽の上流に届いている"
            );
        }
        assert!(
            !authorizations(&sub_upstream).await.is_empty(),
            "前提: 裏方の照会は届いたこと（{arm}）"
        );
        assert!(!text.contains(MAIN_KEY), "{arm}: 鍵が載っている: {text}");
        running.stop().await;
    }
}

/// **照会しない面（`context_tokens` を書いた面）でも、IP アドレスの宛先の判定は面に出る**
/// （作業単位 A-13b-2 のレビュー是正 / `context::resolve_budget` と `resolve_sub_context` の明示の枝）。
///
/// 窓を明示した面は起動時にモデルを照会しない（ADR-0054）ので、照会の後に判定を読む経路（`probe_with`）を通らない。
/// **IP アドレスの宛先は組んだときに判定が決まる**ので、明示の枝がそれを観測に載せる。本文と裏方で**別の値**
/// （本文はローカルネットワーク、裏方は承認なしの外）にして、取り違えと片方の欠落を見る。
/// どちらの宛先も届かないアドレスで、照会しないので接続もしない。
#[tokio::test]
async fn a_face_that_states_its_context_still_names_its_address_verdict() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let source = json!({
        "explicit": {
            "main": { "endpoint": "http://10.0.0.5:9", "model": MAIN_MODEL,
                      "api_key": MAIN_KEY, "context_tokens": 32768 },
            "sub": { "endpoint": "http://203.0.113.5:9", "model": SUB_MODEL,
                     "api_key": MAIN_KEY, "context_tokens": 8192 },
        }
    });
    let resolver = FixedResolver::answering(&["127.0.0.1"]);
    let running = Running::start(
        &config_of(
            &source,
            "explicit",
            &resolver,
            &dir.path().join("explicit.sqlite"),
        ),
        None,
    )
    .await;

    let (text, report) = running.upstream_report().await;
    assert_eq!(
        (
            plain_http_of(&report, "main"),
            plain_http_of(&report, "sub")
        ),
        (Some(&json!("local_network")), Some(&json!("not_approved"))),
        "{report}"
    );
    assert_eq!(
        (
            &report["main"]["answered_by"],
            &report["sub"]["answered_by"]
        ),
        (&Value::Null, &Value::Null),
        "前提: どちらの面も照会していないこと（明示の枝を通ったこと）: {report}"
    );
    assert!(!text.contains(MAIN_KEY), "鍵が載っている: {text}");
    running.stop().await;
}

/// **承認は面ごと。本文の承認を裏方へ貸さない**（ADR-0056 §決定5 / 鍵と同じ規則）。
///
/// 本文と裏方を**別の偽の上流**（どちらもホスト名・鍵つき・名前解決は外が混ざる）にし、承認を**本文だけ**に書く。
/// **起動時（`with_profile_keys`）と切り替え（`switch::prepare`）の両方**で、裏方は `not_approved` のまま、
/// 裏方の上流には何も届かないことを見る。切り替えは、承認を裏方に書いて本文から外したプロファイルとの往復
/// （面を取り違える実装はどちらかの段で赤になる）。
#[tokio::test]
async fn the_approval_belongs_to_its_own_face_at_launch_and_on_switch() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let main_upstream = listing_upstream().await;
    let sub_upstream = listing_upstream().await;
    let sub_face = |allow: Option<bool>| {
        let mut face = keyed_face(&named(&sub_upstream), allow);
        face["model"] = json!(SUB_MODEL);
        face
    };
    let source = json!({
        "main-approved": {
            "main": keyed_face(&named(&main_upstream), Some(true)),
            "sub": sub_face(None),
        },
        "sub-approved": {
            "main": keyed_face(&named(&main_upstream), None),
            "sub": sub_face(Some(true)),
        },
    });
    let profiles = write_profiles(&dir, &source);
    let resolver = FixedResolver::answering(&["127.0.0.1", "203.0.113.5"]);
    let running = Running::start(
        &config_of(
            &source,
            "main-approved",
            &resolver,
            &dir.path().join("faces.sqlite"),
        ),
        Some(profiles),
    )
    .await;

    let (_, report) = running.upstream_report().await;
    assert_eq!(
        (
            plain_http_of(&report, "main"),
            plain_http_of(&report, "sub")
        ),
        (Some(&json!("approved")), Some(&json!("not_approved"))),
        "起動時: {report}"
    );
    assert!(
        !authorizations(&main_upstream).await.is_empty(),
        "前提: 本文の上流へは届くこと"
    );
    assert!(
        authorizations(&sub_upstream).await.is_empty(),
        "起動時: 承認の無い裏方の上流に届いている"
    );

    let main_before = authorizations(&main_upstream).await.len();
    let switched = running.switch_to("sub-approved").await;
    assert_eq!(
        (
            plain_http_of(&switched, "main"),
            plain_http_of(&switched, "sub")
        ),
        (Some(&json!("not_approved")), Some(&json!("approved"))),
        "切り替え: {switched}"
    );
    assert!(
        !authorizations(&sub_upstream).await.is_empty(),
        "切り替え: 承認を書いた裏方の上流へ届いていない"
    );
    assert_eq!(
        authorizations(&main_upstream).await.len(),
        main_before,
        "切り替え: 承認を外した本文の上流へ届いている"
    );
    running.stop().await;
}

/// **宛先・モデル・鍵が同じで、承認だけが違うプロファイルへ切り替えても、承認が追従する**
/// （ADR-0056 §決定5 / `switch::plan_switch` の比べるもの）。
///
/// 承認を比べない実装は「触らない」と読み、**前の（承認の無い）クライアントが残る** —— 偽の上流に何も届かない。
/// 切り替えの応答（`UpstreamReport`）と `GET /admin/upstream` の両方で面を見る。
#[tokio::test]
async fn switching_to_a_profile_with_the_approval_carries_the_approval() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let upstream = listing_upstream().await;
    let endpoint = named(&upstream);
    let source = json!({
        "plain": { "main": keyed_face(&endpoint, None) },
        "approved": { "main": keyed_face(&endpoint, Some(true)) },
    });
    let profiles = write_profiles(&dir, &source);
    let resolver = FixedResolver::answering(&["127.0.0.1", "203.0.113.5"]);
    let running = Running::start(
        &config_of(
            &source,
            "plain",
            &resolver,
            &dir.path().join("switch.sqlite"),
        ),
        Some(profiles),
    )
    .await;

    let (_, report) = running.upstream_report().await;
    assert_eq!(plain_http_of(&report, "main"), Some(&json!("not_approved")));
    assert!(
        authorizations(&upstream).await.is_empty(),
        "前提: 承認の無い起動では何も届かないこと"
    );

    let switched = running.switch_to("approved").await;
    assert_eq!(
        plain_http_of(&switched, "main"),
        Some(&json!("approved")),
        "{switched}"
    );
    let seen = authorizations(&upstream).await;
    assert!(
        !seen.is_empty() && seen.iter().all(|h| *h == bearer(MAIN_KEY)),
        "承認のあるプロファイルへ切り替えたのに、鍵つきの照会が届いていない: {seen:?}"
    );

    // **戻すと、また止まる**（承認は切り替え先の面のもの。前の承認を引き継がない）。
    let before = seen.len();
    let back = running.switch_to("plain").await;
    assert_eq!(plain_http_of(&back, "main"), Some(&json!("not_approved")));
    let (_, report) = running.upstream_report().await;
    assert_eq!(plain_http_of(&report, "main"), Some(&json!("not_approved")));
    assert_eq!(
        authorizations(&upstream).await.len(),
        before,
        "承認の無いプロファイルへ戻した後の照会が届いている"
    );
    running.stop().await;
}
