//! **キャラクターを「しまう」「完全に消す」**の入口の検証
//! （[ADR-0049](../docs/adr/0049-deleting-characters-and-conversations.md) §決定1 / 作業単位 A-10 / Issue origin#55）。
//!
//! **上流はモック**（実際の LM Studio には繋がない）。しまったキャラクターへの chat は
//! **上流を呼ぶ前に断る**ので、呼ばれた回数が 0 であることも見る。
//!
//! ここで固定するのは主に次の点:
//!
//!   * `PATCH /admin/characters/{handle}` の `archived` でしまう／戻す。応答に `archived_at` が載る。
//!   * しまったキャラクターは `GET /v1/models` に出ない。戻すとまた出る。
//!   * しまったキャラクターへの chat は 409 `model_archived` で、**会話もメッセージも増えない**。
//!   * 管理の読み取りの口（会話の一覧・メッセージ・記憶）は、しまったキャラクターにも答える。
//!   * `DELETE` は「会話が無い」**または「しまってある」**なら消す。しまっていない・会話のあるものは 409。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use personacore::domain::{DisplayName, Handle, MemoryContent, NewMemory, Persona, Subject};
use personacore::llm::{
    ChatRequest, ChatStream, Completion, FinishReason, LlmClient, LlmError, LlmFuture,
};
use personacore::server::{App, AppConfig, BindAddr, ServerError};
use personacore::store::Store;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

mod common;

use common::{CLOCK_SKEW, fixture_scale};

/// 上流の代役。**呼ばれた回数だけを数える。**
struct MockLlm {
    calls: Mutex<usize>,
}

impl MockLlm {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(0),
        })
    }

    fn call_count(&self) -> usize {
        *self.calls.lock().expect("ロックを取れること")
    }
}

impl LlmClient for MockLlm {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        Box::pin(async move {
            *self.calls.lock().expect("ロックを取れること") += 1;
            Ok(Completion {
                content: "「はい」".to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        Box::pin(async move {
            *self.calls.lock().expect("ロックを取れること") += 1;
            Err(LlmError::MissingContent)
        })
    }
}

/// エフェメラルポートで起動した実サーバ。**上流はモック。**
struct TestServer {
    addr: SocketAddr,
    store: Store,
    llm: Arc<MockLlm>,
    client: reqwest::Client,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    async fn start() -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db_path = dir.path().join("personacore.sqlite");
        let config = AppConfig::from_lookup(|_| None)
            .expect("既定値だけで設定を組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(&db_path);

        let llm = MockLlm::new();
        let app = App::bind_with_client(&config, llm.clone())
            .await
            .expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let store = app.store().clone();

        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });

        Self {
            addr,
            store,
            llm,
            client: reqwest::Client::new(),
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    async fn send(&self, method: Method, path: &str, body: Option<&Value>) -> (StatusCode, String) {
        let mut request = self
            .client
            .request(method, format!("http://{}{path}", self.addr));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.expect("サーバへ接続できること");
        let status = response.status();
        let text = response.text().await.expect("本文を読めること");
        (status, text)
    }

    async fn get_json(&self, path: &str) -> Value {
        let (status, body) = self.send(Method::GET, path, None).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        parse_json(&body)
    }

    async fn patch(&self, handle: &str, body: &Value) -> (StatusCode, Value) {
        let (status, text) = self
            .send(
                Method::PATCH,
                &format!("/admin/characters/{handle}"),
                Some(body),
            )
            .await;
        (status, parse_json(&text))
    }

    async fn delete(&self, handle: &str) -> (StatusCode, String) {
        self.send(Method::DELETE, &format!("/admin/characters/{handle}"), None)
            .await
    }

    async fn chat(&self, model: &str, stream: bool) -> (StatusCode, String) {
        self.send(
            Method::POST,
            "/v1/chat/completions",
            Some(&json!({
                "model": model,
                "stream": stream,
                "messages": [{"role": "user", "content": "こんにちは"}],
            })),
        )
        .await
    }

    async fn seed(&self, handle: &str) {
        self.store
            .create_character(
                &Handle::parse(handle).expect("正当な識別子"),
                &DisplayName::parse("葵").expect("正当な表示名"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナ"),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");
    }

    async fn count(&self, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(self.store.pool())
            .await
            .expect("行数を数えられること")
    }

    async fn model_ids(&self) -> Vec<String> {
        self.get_json("/v1/models").await["data"]
            .as_array()
            .expect("data は配列であること")
            .iter()
            .map(|entry| entry["id"].as_str().expect("id は文字列").to_owned())
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

fn parse_json(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|err| panic!("JSON として読めること: {err} / {body}"))
}

fn parse_time(value: &Value) -> DateTime<Utc> {
    value
        .as_str()
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|time| time.with_timezone(&Utc))
        .unwrap_or_else(|| panic!("RFC 3339 の時刻であること: {value}"))
}

/// 一覧の1件（`handle` で引く）。
fn listed<'a>(list: &'a Value, handle: &str) -> &'a Value {
    list["characters"]
        .as_array()
        .expect("characters は配列であること")
        .iter()
        .find(|character| character["handle"] == json!(handle))
        .unwrap_or_else(|| panic!("一覧に {handle} が出ること: {list}"))
}

// ======================================================================
// しまう → /v1/models から消える → 戻す → また出る
// ======================================================================

/// **しまうと `/v1/models` から消え、戻すとまた出る。** 3体並べて真ん中をしまう。
///
/// 応答のキャラクターには `archived_at` が載る（しまったら時刻、戻したら `null`）。
/// `GET /admin/characters` は**しまったキャラクターも返す**（しまった一覧の元）。
#[tokio::test]
async fn a_put_away_character_leaves_the_model_list_until_it_is_restored() {
    let server = TestServer::start().await;
    for handle in ["akane", "aoi", "yuzu"] {
        server.seed(handle).await;
    }
    assert_eq!(server.model_ids().await, vec!["akane", "aoi", "yuzu"]);
    assert_eq!(
        listed(&server.get_json("/admin/characters").await, "aoi")["archived_at"],
        Value::Null,
        "作ったときは null（キーごと落とさない）"
    );

    let before = Utc::now();
    let (status, body) = server.patch("aoi", &json!({"archived": true})).await;
    let after = Utc::now();
    assert_eq!(status, StatusCode::OK, "{body}");
    let archived_at = parse_time(&body["character"]["archived_at"]);
    assert!(
        archived_at >= before - CLOCK_SKEW && archived_at <= after + CLOCK_SKEW,
        "しまった時刻はいま: {archived_at}"
    );

    assert_eq!(
        server.model_ids().await,
        vec!["akane", "yuzu"],
        "しまったキャラクターはモデルの一覧に出ない"
    );
    let list = server.get_json("/admin/characters").await;
    assert_eq!(
        parse_time(&listed(&list, "aoi")["archived_at"]),
        archived_at,
        "管理の一覧には時刻つきで出る"
    );
    for neighbour in ["akane", "yuzu"] {
        assert_eq!(
            listed(&list, neighbour)["archived_at"],
            Value::Null,
            "{neighbour}"
        );
    }

    let (status, body) = server.patch("aoi", &json!({"archived": false})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["character"]["archived_at"],
        Value::Null,
        "戻したら null"
    );
    assert_eq!(
        server.model_ids().await,
        vec!["akane", "aoi", "yuzu"],
        "戻すとまた出る"
    );

    server.stop().await;
}

/// **`archived` の `null` と省略は「変えない」**（`stat_scale` と同じ約束）。真偽値以外は 400。
#[tokio::test]
async fn archived_null_or_omitted_keeps_the_state_and_other_types_are_rejected() {
    let server = TestServer::start().await;
    server.seed("aoi").await;
    let (status, _) = server.patch("aoi", &json!({"archived": true})).await;
    assert_eq!(status, StatusCode::OK);

    for body in [json!({"archived": null}), json!({"display_name": "碧"})] {
        let (status, response) = server.patch("aoi", &body).await;
        assert_eq!(status, StatusCode::OK, "{body}: {response}");
        assert!(
            response["character"]["archived_at"].is_string(),
            "しまったまま: {body}: {response}"
        );
    }

    let (status, response) = server.patch("aoi", &json!({"archived": "yes"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    assert_eq!(response["error"]["code"], json!("invalid_body"));

    server.stop().await;
}

// ======================================================================
// しまったキャラクターへの chat は断る
// ======================================================================

/// **しまったキャラクターへの chat は 409 `model_archived`。何も保存しない。上流も呼ばない。**
///
/// `aoi`（会話を作りうる）・`aoi@new`（必ず作る）・`aoi@<id>`（その会話に書く）・ストリーミングの
/// 全部で断る。**会話を解決する前に断る**ことが要点で、後に置くと `aoi@new` が空の会話を残す。
#[tokio::test]
async fn chatting_with_a_put_away_character_is_refused_and_stores_nothing() {
    let server = TestServer::start().await;
    server.seed("aoi").await;
    // **会話は `store` から直接作る。** chat で作ると裏方（抽出）が上流を呼び、
    // 「上流を呼ばない」を回数で見られなくなる。
    let character = server
        .store
        .find_character_by_handle(&Handle::parse("aoi").expect("正当な識別子"))
        .await
        .expect("検索が成功すること")
        .expect("キャラクターが存在すること");
    let session_id = server
        .store
        .create_session(character.id)
        .await
        .expect("会話を作れること")
        .get();

    let (status, _) = server.patch("aoi", &json!({"archived": true})).await;
    assert_eq!(status, StatusCode::OK);
    let sessions = server.count("SELECT count(*) FROM session").await;
    let messages = server.count("SELECT count(*) FROM message").await;

    for (model, stream) in [
        ("aoi".to_owned(), false),
        ("aoi@new".to_owned(), false),
        (format!("aoi@{session_id}"), false),
        ("aoi@new".to_owned(), true),
    ] {
        let (status, body) = server.chat(&model, stream).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "{model} / stream={stream}: {body}"
        );
        assert_eq!(
            parse_json(&body),
            json!({
                "error": {
                    "message": "指定された model のキャラクターはしまわれています。戻すと話せます",
                    "type": "invalid_request_error",
                    "code": "model_archived",
                }
            }),
            "{model} / stream={stream}"
        );
    }

    assert_eq!(
        server.count("SELECT count(*) FROM session").await,
        sessions,
        "会話は増えない"
    );
    assert_eq!(
        server.count("SELECT count(*) FROM message").await,
        messages,
        "メッセージも増えない"
    );
    assert_eq!(server.llm.call_count(), 0, "上流は呼ばない");

    // **戻せばまた話せる。**
    let (status, _) = server.patch("aoi", &json!({"archived": false})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = server.chat("aoi", false).await;
    assert_eq!(status, StatusCode::OK, "戻したら話せること: {body}");

    server.stop().await;
}

/// **管理の読み取りの口は、しまったキャラクターにも答える**（中を見てから消すかを決められるように）。
#[tokio::test]
async fn the_admin_read_routes_still_answer_for_a_put_away_character() {
    let server = TestServer::start().await;
    server.seed("aoi").await;
    let (status, body) = server.chat("aoi@new", false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let character = server
        .store
        .find_character_by_handle(&Handle::parse("aoi").expect("正当な識別子"))
        .await
        .expect("検索が成功すること")
        .expect("キャラクターが存在すること");
    server
        .store
        .add_memory(
            character.id,
            &NewMemory {
                subject: Subject::User,
                content: MemoryContent::parse("猫が好き").expect("正当な本文"),
                category: None,
            },
        )
        .await
        .expect("記憶を足せること");

    let (status, _) = server.patch("aoi", &json!({"archived": true})).await;
    assert_eq!(status, StatusCode::OK);

    let sessions = server.get_json("/admin/characters/aoi/sessions").await;
    let session_id = sessions["sessions"][0]["id"]
        .as_i64()
        .unwrap_or_else(|| panic!("会話が1件読めること: {sessions}"));
    let messages = server
        .get_json(&format!(
            "/admin/characters/aoi/sessions/{session_id}/messages"
        ))
        .await;
    assert_eq!(
        messages["messages"].as_array().map(Vec::len),
        Some(2),
        "往復の2件が読めること: {messages}"
    );
    let memories = server.get_json("/admin/characters/aoi/memories").await;
    assert_eq!(
        memories["memories"][0]["content"],
        json!("猫が好き"),
        "記憶が読めること: {memories}"
    );

    server.stop().await;
}

// ======================================================================
// 完全に消す
// ======================================================================

/// **しまっていない・会話のあるキャラクターの `DELETE` は 409。しまった後は 204 で全部消える。**
#[tokio::test]
async fn a_character_with_a_conversation_is_deleted_only_after_it_is_put_away() {
    let server = TestServer::start().await;
    for handle in ["akane", "aoi", "yuzu"] {
        server.seed(handle).await;
    }
    for model in ["akane@new", "aoi@new", "yuzu@new"] {
        let (status, body) = server.chat(model, false).await;
        assert_eq!(status, StatusCode::OK, "{model}: {body}");
    }

    let (status, body) = server.delete("aoi").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        parse_json(&body)["error"]["code"],
        json!("character_has_sessions")
    );
    assert_eq!(server.count("SELECT count(*) FROM character").await, 3);

    let (status, _) = server.patch("aoi", &json!({"archived": true})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = server.delete("aoi").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(body.is_empty(), "204 に本文を付けないこと: {body}");

    let list = server.get_json("/admin/characters").await;
    let handles: Vec<&str> = list["characters"]
        .as_array()
        .expect("配列")
        .iter()
        .filter_map(|character| character["handle"].as_str())
        .collect();
    assert_eq!(handles, vec!["akane", "yuzu"], "対象だけが消える");
    assert_eq!(
        server.count("SELECT count(*) FROM session").await,
        2,
        "対象の会話だけが消える"
    );
    assert_eq!(
        server.count("SELECT count(*) FROM message").await,
        4,
        "対象のメッセージだけが消える"
    );
    let (status, _) = server
        .send(Method::GET, "/admin/characters/aoi/sessions", None)
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "消えたキャラクターは引けない"
    );

    // **同じ識別子でもう一度作れる。**
    let (status, body) = server
        .send(
            Method::POST,
            "/admin/characters",
            Some(&json!({"handle": "aoi", "base_persona": "新しいペルソナ。"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(parse_json(&body)["archived_at"], Value::Null);

    server.stop().await;
}

/// **会話の無いキャラクターの `DELETE` は、しまわなくても今までどおり 204。**
#[tokio::test]
async fn a_character_without_a_conversation_is_still_deleted_directly() {
    let server = TestServer::start().await;
    for handle in ["akane", "aoi", "yuzu"] {
        server.seed(handle).await;
    }

    let (status, body) = server.delete("aoi").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(server.model_ids().await, vec!["akane", "yuzu"]);

    server.stop().await;
}
