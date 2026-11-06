//! **上流プロファイルの面に書いた鍵**（[ADR-0056](../docs/adr/0056-a-key-for-the-upstream.md) /
//! 作業単位 A-13b-1）が、**本番の入口（`App::bind` → `App::serve`）と走ったままの切り替えを通って**、
//! その面の上流にだけ届くことの検証。**実際の上流には繋がない**（`wiremock` の偽物）。
//!
//! # 何を見るか
//!
//! | 見張り番 | 決定 |
//! |---|---|
//! | [`each_face_sends_its_own_key_to_its_own_upstream_through_the_real_bind`] | §決定1・2: 本文と裏方は別の鍵・別の上流。`sub` に鍵が無ければ、本文に鍵があっても裏方には付かない |
//! | [`switching_carries_the_key_to_the_next_profile`] | §決定2: 切り替えの3通り（なし→あり・あり→別の鍵・あり→なし） |
//! | [`switching_gives_the_sub_upstream_only_the_sub_faces_key`] | §決定1・2: 切り替えでも、裏方の上流には裏方の面の鍵だけ（レビュー是正） |
//! | [`the_upstream_screen_says_which_face_had_its_key_refused`] | §決定4: 照会が 401 / 403 で断られた面を、上流の画面が言う |
//! | どの腕も | §決定3: `GET /admin/upstream` の本文に鍵が出ない |
//!
//! # 守っていないもの（先に書く）
//!
//! * **`main` の結線**（`config_for` が `with_profile_keys` を呼ぶこと）は、bin なのでここからは見えない。
//!   **ここは `main` と同じ段（`overlay_profile` → `with_profile_context` → `with_profile_keys`）を自分で組む。**
//!   結線そのものは `tests/upstream_cli.rs` の `a_key_written_in_the_profile_reaches_the_executable_and_never_its_log`
//!   が実行ファイルで見る。
//! * **本文の依頼（チャット）に鍵が付くこと**は `tests/llm_api_key.rs` が `llm` の線の上で見る。
//!   ここで観測するのは起動時と切り替えのときのモデルの照会（`/api/v0/models`）だけである。

use std::path::{Path, PathBuf};
use std::time::Duration;

use personacore::server::{
    App, AppConfig, BindAddr, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL, ENV_SUB_MODEL, ServerError,
    overlay_profile, parse_profiles,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

/// 本文に指定するモデル。
const MAIN_MODEL: &str = "planted-key-main-model";
/// 抽出に指定するモデル。**本文用と別の綴り。**
const SUB_MODEL: &str = "planted-key-sub-model";

/// 本文の面の鍵（囮）。
const MAIN_KEY: &str = "pc-bind-main-KEY-0a1b2c3d";
/// 裏方の面の鍵（囮）。**本文の鍵と別の綴り。**
const SUB_KEY: &str = "pc-bind-sub-KEY-4e5f6a7b";
/// 切り替え先の鍵（囮）。**上の2つと別の綴り。**
const OTHER_KEY: &str = "pc-bind-other-KEY-8c9d0e1f";

/// **モデルの一覧に答える上流**（`/api/v0/models`）。届いた要求は `received_requests` で読む。
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

/// **どちらの一覧の綴りにも、同じステータスで答える上流。**
async fn refusing_upstream(status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .respond_with(ResponseTemplate::new(status))
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

/// **面の1つ**（`api_key` は書くときだけ）。
fn face(endpoint: &str, model: &str, api_key: Option<&str>) -> Value {
    let mut face = json!({ "endpoint": endpoint, "model": model });
    if let Some(key) = api_key {
        face["api_key"] = json!(key);
    }
    face
}

/// **`main` と同じ段で設定を組む**（`overlay_profile` → `with_profile_context` → `with_profile_keys`）。
///
/// `chosen` が `None` なら、プロファイルを選ばない起動（環境変数だけ）。
fn config_of(upstream: &str, chosen: Option<(&Value, &str)>, db_path: &Path) -> AppConfig {
    let pairs: Vec<(String, String)> = vec![
        (ENV_LLM_ENDPOINT.to_owned(), upstream.to_owned()),
        (ENV_MAIN_MODEL.to_owned(), MAIN_MODEL.to_owned()),
        (ENV_SUB_MODEL.to_owned(), SUB_MODEL.to_owned()),
    ];
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
                .with_profile_keys(&settings)
        }
    };
    config
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

    /// **`POST /admin/upstream/profile`**。200 でなければ落とす。応答の本文を返す。
    async fn switch_to(&self, name: &str) -> String {
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
        text
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

/// **本文に、囮の鍵が1つも載っていないこと**（ADR-0056 §決定3）。
fn assert_no_key_in(text: &str) {
    for key in [MAIN_KEY, SUB_KEY, OTHER_KEY] {
        assert!(!text.contains(key), "鍵が載っている（{key}）: {text}");
    }
}

fn write_profiles(dir: &TempDir, source: &Value) -> PathBuf {
    let path = dir.path().join("upstreams.json");
    std::fs::write(&path, source.to_string()).expect("プロファイルを置けること");
    path
}

/// **本文と裏方は、それぞれの鍵をそれぞれの上流へ送る。`sub` に鍵が無ければ、本文に鍵があっても
/// 裏方には何も付かない**（ADR-0056 §決定1・2）。
///
/// | 腕 | 本文の上流が受けた鍵 | 裏方の上流が受けた鍵 |
/// |---|---|---|
/// | 両方に別の鍵 | 本文の鍵だけ | 裏方の鍵だけ |
/// | 本文だけに鍵（`sub` は書いた） | 本文の鍵だけ | **何も無い** |
///
/// **どちらも起動時の照会が実際に届いたこと**を前提として見る（届かなければ何も見ていない）。
#[tokio::test]
async fn each_face_sends_its_own_key_to_its_own_upstream_through_the_real_bind() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    for (arm, sub_key, expected_sub) in [
        ("both", Some(SUB_KEY), bearer(SUB_KEY)),
        ("main-only", None, None),
    ] {
        let main_upstream = listing_upstream().await;
        let sub_upstream = listing_upstream().await;
        let source = json!({
            arm: {
                "main": face(&main_upstream.uri(), MAIN_MODEL, Some(MAIN_KEY)),
                "sub": face(&sub_upstream.uri(), SUB_MODEL, sub_key),
            }
        });
        let running = Running::start(
            &config_of(
                "http://127.0.0.1:9",
                Some((&source, arm)),
                &dir.path().join(format!("{arm}.sqlite")),
            ),
            None,
        )
        .await;

        let main_seen = authorizations(&main_upstream).await;
        let sub_seen = authorizations(&sub_upstream).await;
        assert!(
            !main_seen.is_empty() && !sub_seen.is_empty(),
            "前提: 起動時の照会が両方の上流へ届いたこと（{arm}）: {main_seen:?} / {sub_seen:?}"
        );
        assert!(
            main_seen.iter().all(|header| *header == bearer(MAIN_KEY)),
            "本文の上流が本文の鍵以外を受けた（{arm}）: {main_seen:?}"
        );
        assert!(
            sub_seen.iter().all(|header| *header == expected_sub),
            "裏方の上流が {expected_sub:?} 以外を受けた（{arm}）: {sub_seen:?}"
        );

        let (text, _) = running.upstream_report().await;
        assert_no_key_in(&text);
        running.stop().await;
    }
}

/// **走ったままの切り替えで、鍵は切り替え先に追従する**（ADR-0056 §決定2）。
///
/// | 切り替え | 前 | 後 |
/// |---|---|---|
/// | なし → あり | 何も付かない | 鍵 A |
/// | あり → 別の鍵 | 鍵 A | **鍵 B**（宛先もモデルも同じ。鍵だけが違う） |
/// | あり → なし | 鍵 B | 何も付かない |
///
/// **どの段も、切り替えの後に照会が1本以上届いたこと**を前提として見る ——
/// 鍵だけが違う切り替えを「触らない」と読む実装（`switch::plan_switch` が鍵を比べない）では、
/// 照会が1本も出ず、前の鍵のままのクライアントが残る。
#[tokio::test]
async fn switching_carries_the_key_to_the_next_profile() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let upstream = listing_upstream().await;
    let endpoint = upstream.uri();
    let profiles = write_profiles(
        &dir,
        &json!({
            "keyed": { "main": face(&endpoint, MAIN_MODEL, Some(MAIN_KEY)) },
            "rekeyed": { "main": face(&endpoint, MAIN_MODEL, Some(OTHER_KEY)) },
            "bare": { "main": face(&endpoint, MAIN_MODEL, None) },
        }),
    );

    // **起動は環境変数だけ**（鍵の口は無い）。
    let running = Running::start(
        &config_of(&endpoint, None, &dir.path().join("switch.sqlite")),
        Some(profiles),
    )
    .await;
    let before = authorizations(&upstream).await;
    assert!(
        !before.is_empty() && before.iter().all(Option::is_none),
        "前提: 起動時は鍵が付かないこと: {before:?}"
    );

    let mut seen = before.len();
    for (name, expected) in [
        ("keyed", bearer(MAIN_KEY)),
        ("rekeyed", bearer(OTHER_KEY)),
        ("bare", None),
    ] {
        let answer = running.switch_to(name).await;
        assert_no_key_in(&answer);
        let all = authorizations(&upstream).await;
        let after = &all[seen..];
        assert!(
            !after.is_empty(),
            "{name} へ切り替えたのに照会が1本も出ていない（面を触らずに済ませている）"
        );
        assert!(
            after.iter().all(|header| *header == expected),
            "{name} へ切り替えた後の照会が {expected:?} 以外を持っている: {after:?}"
        );
        seen = all.len();
    }

    let (text, _) = running.upstream_report().await;
    assert_no_key_in(&text);
    running.stop().await;
}

/// **走ったままの切り替えで、裏方の上流には裏方の面の鍵だけが付く**（ADR-0056 §決定1・2 /
/// 作業単位 A-13b-1 のレビュー是正）。
///
/// **本文と裏方を別の偽の上流にする**（同じ上流だと、どちらの鍵が付いたかを裏方の上流で見分けられない）。
///
/// | 切り替え | 本文の上流が受ける | 裏方の上流が受ける |
/// |---|---|---|
/// | 本文に鍵・裏方は鍵なし | 本文の鍵 | **何も付かない**（本文の鍵を貸さない） |
/// | 両方に別の鍵 | 本文の鍵（触らない面なので照会は出なくてよい） | **裏方の鍵** |
///
/// **各段で、裏方の上流に照会が1本以上届いたこと**を前提として見る。
/// 切り替えで裏方の候補に本文の面の鍵を渡す実装（`switch::prepare` の `retargeted`）は、どちらの段でも赤になる。
#[tokio::test]
async fn switching_gives_the_sub_upstream_only_the_sub_faces_key() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let main_upstream = listing_upstream().await;
    let sub_upstream = listing_upstream().await;
    let main_endpoint = main_upstream.uri();
    let sub_endpoint = sub_upstream.uri();
    let profiles = write_profiles(
        &dir,
        &json!({
            "main-keyed": {
                "main": face(&main_endpoint, MAIN_MODEL, Some(MAIN_KEY)),
                "sub": face(&sub_endpoint, SUB_MODEL, None),
            },
            "both-keyed": {
                "main": face(&main_endpoint, MAIN_MODEL, Some(MAIN_KEY)),
                "sub": face(&sub_endpoint, SUB_MODEL, Some(SUB_KEY)),
            },
        }),
    );

    // **起動は環境変数だけ**（本文の上流だけを指す。裏方の上流にはまだ何も届かない）。
    let running = Running::start(
        &config_of(&main_endpoint, None, &dir.path().join("sub-switch.sqlite")),
        Some(profiles),
    )
    .await;
    assert!(
        authorizations(&sub_upstream).await.is_empty(),
        "前提: 起動時は裏方の上流へ何も届かないこと"
    );

    let mut seen = 0;
    for (name, expected_sub) in [("main-keyed", None), ("both-keyed", bearer(SUB_KEY))] {
        let answer = running.switch_to(name).await;
        assert_no_key_in(&answer);
        let all = authorizations(&sub_upstream).await;
        let after = &all[seen..];
        assert!(
            !after.is_empty(),
            "{name} へ切り替えたのに裏方の上流へ照会が1本も出ていない"
        );
        assert!(
            after.iter().all(|header| *header == expected_sub),
            "{name} へ切り替えた後、裏方の上流が {expected_sub:?} 以外を受けた: {after:?}"
        );
        seen = all.len();
    }
    let main_seen = authorizations(&main_upstream).await;
    assert!(
        main_seen.iter().any(|header| *header == bearer(MAIN_KEY))
            && !main_seen.iter().any(|header| *header == bearer(SUB_KEY)),
        "本文の上流が本文の鍵を受けていない／裏方の鍵を受けた: {main_seen:?}"
    );

    let (text, _) = running.upstream_report().await;
    assert_no_key_in(&text);
    running.stop().await;
}

/// **照会が 401 / 403 で断られた面を、上流の画面が言う**（ADR-0056 §決定4）。
///
/// **本文は 401・裏方は 500**（面を取り違える実装を赤にする）。**403 の腕も回す。**
/// **500 は「断られた」ではない**（届いたが壊れている）。
#[tokio::test]
async fn the_upstream_screen_says_which_face_had_its_key_refused() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    for refusal in [401, 403] {
        let main_upstream = refusing_upstream(refusal).await;
        let sub_upstream = refusing_upstream(500).await;
        let source = json!({
            "refused": {
                "main": face(&main_upstream.uri(), MAIN_MODEL, Some(MAIN_KEY)),
                "sub": face(&sub_upstream.uri(), SUB_MODEL, Some(SUB_KEY)),
            }
        });
        let running = Running::start(
            &config_of(
                "http://127.0.0.1:9",
                Some((&source, "refused")),
                &dir.path().join(format!("refused-{refusal}.sqlite")),
            ),
            None,
        )
        .await;

        let (text, report) = running.upstream_report().await;
        assert_eq!(
            (
                &report["main"]["unauthorized"],
                &report["sub"]["unauthorized"]
            ),
            (&json!(true), &json!(false)),
            "{refusal}: {report}"
        );
        assert_no_key_in(&text);
        running.stop().await;
    }
}
