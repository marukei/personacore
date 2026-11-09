//! **実際の応答を、仕様書（OpenAPI）で検証する**（ADR-0047 §② / 作業単位 P-2a）。
//!
//! # 何を見るか
//!
//! 本物の HTTP の口を叩き、返った本文を**仕様書の応答の型**で検証する。
//!
//! | 見ること | どう |
//! |---|---|
//! | 型・必須・`enum`・`null` の可否 | JSON Schema の検証器（`boon` / dev-dependency）で `$ref` ごと検証 |
//! | **仕様書に無い欄が載っていないか** | 下の [`Spec::undocumented`]（JSON Schema は既定で余計な欄を許すので、自分で見る） |
//! | SSE の各 `data` | `ChatCompletionEvent`（**写し** / `server::openapi` の doc）で検証。**`contentSchema` まで効かせる** |
//!
//! # ⚠ これだけでは見えない向き
//!
//! **仕様書が実際より緩い**（応答の `Option` を「省かれうる」と書く等）ずれは、この検証では赤にならない
//! ——緩い仕様書は、より厳しい応答を必ず通す。**その向きは `server::openapi` の単体テスト
//! （`a_response_never_omits_a_field_unless_it_is_named_here`）が構造で見る。**
//!
//! # 対照
//!
//! **検証の道具が何でも通す形に壊れていないこと**を、植えた応答で見る
//! （`the_checker_refuses_planted_responses`）。

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use boon::{Compiler, Draft, Schemas};
use personacore::domain::{
    AxisCandidateId, AxisCandidateRecord, AxisCandidateState, AxisTrialFailure, AxisTrialVote,
    CharacterId, DriftDelta, DriftOutcome, DriftProposal, DriftSuggestion, Handle, MemoryContent,
    MemoryId, NewMemory, SessionId, StatAxis, Subject, apply_drift,
};
use personacore::llm::{
    ChatChunk, ChatRequest, ChatStream, Completion, FinishReason, LlmClient, LlmError, LlmFuture,
};
use personacore::server::{
    App, AppConfig, BindAddr, ENV_LLM_ENDPOINT, ENV_PORT, LaunchPlan, ServerError, Startup,
    api_document,
};
use personacore::store::Store;
use reqwest::{Method, StatusCode};
use serde_json::{Map, Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

// ----------------------------------------------------------------------
// 仕様書と、それで検証する道具
// ----------------------------------------------------------------------

/// 仕様書と、その `components.schemas` を `$defs` へ移したもの（検証器に渡す形）。
struct Spec {
    document: Value,
    defs: Value,
}

impl Spec {
    fn load() -> Self {
        let document = api_document().expect("仕様書は JSON にできること");
        let defs = Self::rewrite(&document["components"]["schemas"]);
        Self { document, defs }
    }

    /// `#/components/schemas/X` を `#/$defs/X` へ（検証器の根に `$defs` として置くため）。
    fn rewrite(value: &Value) -> Value {
        let text = value
            .to_string()
            .replace("#/components/schemas/", "#/$defs/");
        serde_json::from_str(&text).expect("書き換えても JSON のままであること")
    }

    /// その口・その状態コードの応答の型（無ければ `default`）。
    fn response_schema(&self, path: &str, method: &str, status: u16) -> &Value {
        let responses = &self.document["paths"][path][method]["responses"];
        assert!(responses.is_object(), "仕様書に {method} {path} が無い");
        let response = if responses[status.to_string()].is_object() {
            &responses[status.to_string()]
        } else {
            &responses["default"]
        };
        let schema = &response["content"]["application/json"]["schema"];
        assert!(
            schema.is_object(),
            "{method} {path} の {status} に JSON の型が無い: {response}"
        );
        schema
    }

    /// `schema` で `instance` を検証する（**`format` と `contentSchema` も効かせる**）。
    fn validate(&self, schema: &Value, instance: &Value) -> Result<(), String> {
        let mut root = Self::rewrite(schema);
        root.as_object_mut()
            .expect("型はオブジェクトであること")
            .insert("$defs".to_owned(), self.defs.clone());
        let mut compiler = Compiler::new();
        compiler.set_default_draft(Draft::V2020_12);
        compiler.enable_format_assertions();
        compiler.enable_content_assertions();
        compiler
            .add_resource("file:///personacore-openapi.json", root)
            .map_err(|err| format!("{err:#}"))?;
        let mut schemas = Schemas::new();
        let index = compiler
            .compile("file:///personacore-openapi.json", &mut schemas)
            .map_err(|err| format!("{err:#}"))?;
        schemas
            .validate(instance, index)
            .map_err(|err| format!("{err:#}"))
    }

    fn resolve<'a>(&'a self, schema: &'a Value) -> &'a Value {
        let mut schema = schema;
        while let Some(target) = schema["$ref"].as_str() {
            let name = target
                .strip_prefix("#/components/schemas/")
                .expect("仕様書の中を指す $ref であること");
            schema = &self.document["components"]["schemas"][name];
        }
        schema
    }

    /// **仕様書に載っていない欄**（JSON ポインタの綴り）。
    ///
    /// JSON Schema は既定で余計な欄を許すので、検証器だけでは
    /// 「応答に増えた欄を仕様書が知らない」が見えない。**ここで見る。**
    fn undocumented(&self, schema: &Value, value: &Value, at: &str, out: &mut Vec<String>) {
        let schema = self.resolve(schema);
        if value.is_null() {
            return;
        }
        for combinator in ["oneOf", "anyOf"] {
            if let Some(arms) = schema[combinator].as_array() {
                // **いちばん合う腕で見る**（`Option<T>` は `oneOf: [null, T]` になる）。
                let best = arms
                    .iter()
                    .map(|arm| self.resolve(arm))
                    .filter(|arm| arm["type"] != "null")
                    .map(|arm| {
                        let mut found = Vec::new();
                        self.undocumented(arm, value, at, &mut found);
                        found
                    })
                    .min_by_key(Vec::len);
                out.extend(best.unwrap_or_default());
                return;
            }
        }
        match value {
            Value::Object(map) => {
                let mut properties = Map::new();
                for part in
                    std::iter::once(schema).chain(schema["allOf"].as_array().into_iter().flatten())
                {
                    if let Some(found) = self.resolve(part)["properties"].as_object() {
                        properties.extend(found.clone());
                    }
                }
                for (key, inner) in map {
                    match properties.get(key) {
                        Some(property) => {
                            self.undocumented(property, inner, &format!("{at}/{key}"), out);
                        }
                        None => out.push(format!("{at}/{key}")),
                    }
                }
            }
            Value::Array(items) => {
                if let Some(item) = schema.get("items") {
                    for (index, inner) in items.iter().enumerate() {
                        self.undocumented(item, inner, &format!("{at}/{index}"), out);
                    }
                }
            }
            _ => {}
        }
    }

    /// **仕様書がその応答で必須と名乗るヘッダ**が、実際の応答に載り、その型に合うか（P-2b のレビュー）。
    ///
    /// 仕様書の `headers` の鍵と実際のヘッダ名は大文字小文字を区別せずに突き合わせる（HTTP の規則）。
    fn header_problems(
        &self,
        path: &str,
        method: &str,
        status: u16,
        headers: &reqwest::header::HeaderMap,
    ) -> Vec<String> {
        let responses = &self.document["paths"][path][method]["responses"];
        let response = if responses[status.to_string()].is_object() {
            &responses[status.to_string()]
        } else {
            &responses["default"]
        };
        let mut problems = Vec::new();
        for (name, declared) in response["headers"].as_object().into_iter().flatten() {
            if declared["required"] != json!(true) {
                continue;
            }
            match headers.get(name.as_str()).map(|value| value.to_str()) {
                None => problems.push(format!("必須のヘッダ {name} が無い")),
                Some(Err(_)) => problems.push(format!("ヘッダ {name} が ASCII でない")),
                Some(Ok(value)) => {
                    if let Err(err) = self.validate(&declared["schema"], &json!(value)) {
                        problems.push(format!("ヘッダ {name} = {value}: {err}"));
                    }
                }
            }
        }
        problems
    }

    /// 1つの応答の本文を検証する（**型と、余計な欄の両方**）。
    fn problems(&self, path: &str, method: &str, status: u16, body: &Value) -> Vec<String> {
        let schema = self.response_schema(path, method, status);
        let mut problems = Vec::new();
        if let Err(err) = self.validate(schema, body) {
            problems.push(err);
        }
        let mut extra = Vec::new();
        self.undocumented(schema, body, "", &mut extra);
        problems.extend(extra.into_iter().map(|at| format!("仕様書に無い欄: {at}")));
        problems
    }

    fn check(&self, path: &str, method: &str, status: StatusCode, body: &Value) {
        let problems = self.problems(path, method, status.as_u16(), body);
        assert!(
            problems.is_empty(),
            "{method} {path} ({status}) が仕様書と食い違う: {problems:#?}\n本文: {body:#}"
        );
    }

    /// SSE の1件（`data:` の中身）を `ChatCompletionEvent` で検証する。
    fn event_problems(&self, data: &str) -> Vec<String> {
        let event_schema = self.response_schema_of_event();
        let event = json!({ "data": data });
        let mut problems = Vec::new();
        if let Err(err) = self.validate(&event_schema, &event) {
            problems.push(err);
        }
        // **`data` が JSON なら、中身の余計な欄も見る**（増分か、エラーか）。
        if let Ok(parsed) = serde_json::from_str::<Value>(data) {
            let arms = ["ChatCompletionChunk", "ErrorBody"].map(|name| {
                let mut found = Vec::new();
                self.undocumented(
                    &json!({ "$ref": format!("#/components/schemas/{name}") }),
                    &parsed,
                    "/data",
                    &mut found,
                );
                found
            });
            let best = arms.into_iter().min_by_key(Vec::len).unwrap_or_default();
            problems.extend(best.into_iter().map(|at| format!("仕様書に無い欄: {at}")));
        }
        problems
    }

    fn response_schema_of_event(&self) -> Value {
        let item = &self.document["paths"]["/v1/chat/completions"]["post"]["responses"]["200"]["content"]
            ["text/event-stream"]["itemSchema"];
        assert!(item.is_object(), "SSE の itemSchema が仕様書にあること");
        item.clone()
    }
}

// ----------------------------------------------------------------------
// サーバ
// ----------------------------------------------------------------------

/// 本文と中継の両方を返す偽の上流。**回ごとに1本ずつ取り出す。**
struct MockLlm {
    streams: Mutex<VecDeque<Vec<Result<ChatChunk, LlmError>>>>,
}

impl LlmClient for MockLlm {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        Box::pin(async move {
            Ok(Completion {
                content: "「こんにちは」と彼女は笑った。".to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        Box::pin(async move {
            let items = self
                .streams
                .lock()
                .expect("ロックを取れること")
                .pop_front()
                .ok_or(LlmError::MissingContent)?;
            let stream: ChatStream = Box::pin(futures_util::stream::iter(items));
            Ok(stream)
        })
    }
}

struct TestServer {
    addr: SocketAddr,
    store: Store,
    client: reqwest::Client,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    async fn start(streams: Vec<Vec<Result<ChatChunk, LlmError>>>) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let config = AppConfig::from_lookup(|_| None)
            .expect("既定値だけで設定を組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(dir.path().join("personacore.sqlite"));
        let llm = Arc::new(MockLlm {
            streams: Mutex::new(streams.into()),
        });
        let app = App::bind_with_client(&config, llm)
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
            client: reqwest::Client::new(),
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let (status, value, _) = self.send_with_headers(method, path, body).await;
        (status, value)
    }

    /// [`Self::send`] に、応答のヘッダを添えたもの。
    async fn send_with_headers(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value, reqwest::header::HeaderMap) {
        let mut request = self
            .client
            .request(method, format!("http://{}{path}", self.addr));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.expect("サーバへ接続できること");
        let status = response.status();
        let headers = response.headers().clone();
        let text = response.text().await.expect("本文を読めること");
        let value = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("JSON であること: {text}: {err}"))
        };
        (status, value, headers)
    }

    async fn text(&self, path: &str, body: Value) -> (StatusCode, String) {
        let response = self
            .client
            .post(format!("http://{}{path}", self.addr))
            .json(&body)
            .send()
            .await
            .expect("サーバへ接続できること");
        let status = response.status();
        (status, response.text().await.expect("本文を読めること"))
    }

    async fn character_id(&self, handle: &str) -> CharacterId {
        self.store
            .find_character_by_handle(&Handle::parse(handle).expect("正当な識別子"))
            .await
            .expect("キャラクターを引けること")
            .expect("キャラクターが存在すること")
            .id
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).expect("停止シグナルを送れること");
        }
        if let Some(handle) = self.handle.take() {
            handle
                .await
                .expect("サーバタスクが panic しないこと")
                .expect("正常に終了すること");
        }
    }
}

/// 口を叩いて、**期待した状態コード**で返り、**仕様書どおり**であることを見る。
async fn call(
    server: &TestServer,
    spec: &Spec,
    method: Method,
    (path, template): (&str, &str),
    body: Option<Value>,
    expected: StatusCode,
) -> Value {
    let lower = method.as_str().to_ascii_lowercase();
    let (status, value, headers) = server.send_with_headers(method, path, body).await;
    assert_eq!(status, expected, "{lower} {path}: {value:#}");
    spec.check(template, &lower, status, &value);
    let problems = spec.header_problems(template, &lower, status.as_u16(), &headers);
    assert!(
        problems.is_empty(),
        "{lower} {path} ({status}) のヘッダが仕様書と食い違う: {problems:#?}"
    );
    value
}

fn chunk(delta: &str, finish_reason: Option<FinishReason>) -> Result<ChatChunk, LlmError> {
    Ok(ChatChunk {
        delta: delta.to_owned(),
        finish_reason,
        usage: None,
    })
}

// ----------------------------------------------------------------------
// 口ごとの検証
// ----------------------------------------------------------------------

/// **キャラクター・性格値・変容・属性・軸の候補・記憶・一覧**の口。
///
/// 行を仕込んでから読む——**空の一覧だけでは、要素の型（`enum` の綴り・`null` の可否）を
/// 一度も検証しない。**
#[tokio::test]
async fn the_character_endpoints_answer_as_documented() {
    let spec = Spec::load();
    let server = TestServer::start(Vec::new()).await;
    const C: &str = "/admin/characters";
    const CH: &str = "/admin/characters/{handle}";

    call(
        &server,
        &spec,
        Method::POST,
        (C, C),
        Some(json!({
            "handle": "aoi",
            "display_name": "葵",
            "base_persona": "ペルソナ本文",
            "output_format_instruction": "「」で台詞を書く",
            "stat_drift_mode": "shadow",
        })),
        StatusCode::CREATED,
    )
    .await;
    call(&server, &spec, Method::GET, (C, C), None, StatusCode::OK).await;
    call(
        &server,
        &spec,
        Method::PATCH,
        ("/admin/characters/aoi", CH),
        // **見いだす枠を開ける**（下で軸の候補を仕込むため / 既定の 0 では候補を作れない）。
        Some(json!({
            "output_format_instruction": null,
            "stat_drift_mode": null,
            "axis_discovery_slots": 2,
        })),
        StatusCode::OK,
    )
    .await;

    let stats = (
        "/admin/characters/aoi/stats",
        "/admin/characters/{handle}/stats",
    );
    call(
        &server,
        &spec,
        Method::PUT,
        stats,
        Some(json!({"stats": [{"axis": "自信", "value": 3}, {"axis": "好奇心", "value": 0}]})),
        StatusCode::OK,
    )
    .await;
    call(&server, &spec, Method::GET, stats, None, StatusCode::OK).await;

    let attributes = (
        "/admin/characters/aoi/attributes",
        "/admin/characters/{handle}/attributes",
    );
    call(
        &server,
        &spec,
        Method::PUT,
        attributes,
        Some(json!({"attributes": [
            {"subject": "character", "key": "年齢", "value": "19"},
            {"subject": "user", "key": "呼び名", "value": "先輩"},
        ]})),
        StatusCode::OK,
    )
    .await;
    call(
        &server,
        &spec,
        Method::GET,
        attributes,
        None,
        StatusCode::OK,
    )
    .await;
    call(
        &server,
        &spec,
        Method::GET,
        (
            "/admin/characters/aoi/attributes/revisions",
            "/admin/characters/{handle}/attributes/revisions",
        ),
        None,
        StatusCode::OK,
    )
    .await;

    // **変容の票と、割り当ての無い軸**（`value: null` の行を作る）。
    let id = server.character_id("aoi").await;
    let session = server
        .store
        .create_session(id)
        .await
        .expect("セッションを作れること");
    record_vote(&server.store, id, session, "自信", DriftDelta::Up).await;
    record_vote(&server.store, id, session, "自信", DriftDelta::Down).await;
    call(
        &server,
        &spec,
        Method::PUT,
        stats,
        Some(json!({"stats": [{"axis": "好奇心", "value": 2}]})),
        StatusCode::OK,
    )
    .await;
    let drift = call(
        &server,
        &spec,
        Method::GET,
        (
            "/admin/characters/aoi/drift",
            "/admin/characters/{handle}/drift",
        ),
        None,
        StatusCode::OK,
    )
    .await;
    assert!(
        drift["axes"]
            .as_array()
            .is_some_and(|axes| axes.iter().any(|axis| axis["value"].is_null())),
        "前提: 割り当ての無い軸（value: null）を検証したこと: {drift:#}"
    );
    assert!(
        drift["recent_votes"]
            .as_array()
            .is_some_and(|votes| votes.len() == 2),
        "前提: 票を検証したこと: {drift:#}"
    );

    // **記憶と、上書きされた記憶**。
    let memory = add_memory(&server.store, id, Subject::User, "猫が好き").await;
    add_memory(&server.store, id, Subject::Character, "紅茶が好き").await;
    call(
        &server,
        &spec,
        Method::POST,
        (
            &format!(
                "/admin/characters/aoi/memories/{}/corrections",
                memory.get()
            ),
            "/admin/characters/{handle}/memories/{memory_id}/corrections",
        ),
        Some(json!({"subject": "user", "content": "犬が好き", "category": "好み"})),
        StatusCode::CREATED,
    )
    .await;
    let memories = (
        "/admin/characters/aoi/memories",
        "/admin/characters/{handle}/memories",
    );
    call(&server, &spec, Method::GET, memories, None, StatusCode::OK).await;
    let with_superseded = call(
        &server,
        &spec,
        Method::GET,
        (
            "/admin/characters/aoi/memories?include=superseded",
            memories.1,
        ),
        None,
        StatusCode::OK,
    )
    .await;
    assert!(
        with_superseded["superseded"]["memories"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1),
        "前提: 上書きされた記憶（flatten した型）を検証したこと: {with_superseded:#}"
    );

    // **軸の候補**（不合格の理由が `null` でない行と、試用中の行）。
    let source = add_memory(&server.store, id, Subject::Character, "静かな場所が好き").await;
    let failed = propose(&server.store, id, "静けさ", source).await;
    server
        .store
        .record_axis_candidate(
            id,
            &AxisCandidateRecord::Advance {
                candidate: failed,
                to: AxisCandidateState::Failed(AxisTrialFailure::Silent),
            },
        )
        .await
        .expect("状態を進められること");
    let trial = propose(&server.store, id, "几帳面さ", source).await;
    server
        .store
        .record_axis_candidate(
            id,
            &AxisCandidateRecord::TrialVote {
                candidate: trial,
                session,
                vote: AxisTrialVote::Up,
            },
        )
        .await
        .expect("票を足せること");
    let candidates = call(
        &server,
        &spec,
        Method::GET,
        (
            "/admin/characters/aoi/axis-candidates",
            "/admin/characters/{handle}/axis-candidates",
        ),
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        candidates["candidates"].as_array().map(Vec::len),
        Some(2),
        "前提: 候補を検証したこと: {candidates:#}"
    );

    call(
        &server,
        &spec,
        Method::GET,
        ("/v1/models", "/v1/models"),
        None,
        StatusCode::OK,
    )
    .await;

    // **消す口**（会話の無いキャラクターだけ / 本文の無い 204）。
    call(
        &server,
        &spec,
        Method::POST,
        (C, C),
        Some(json!({"handle": "rin", "base_persona": "別のペルソナ"})),
        StatusCode::CREATED,
    )
    .await;
    let (status, body) = server
        .send(Method::DELETE, "/admin/characters/rin", None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(body.is_null(), "204 に本文は無い: {body}");
    let deleted = &spec.document["paths"][CH]["delete"]["responses"]["204"];
    assert!(
        deleted.is_object() && deleted["content"].is_null(),
        "仕様書も 204 を本文なしで名乗ること: {deleted}"
    );

    server.stop().await;
}

/// **会話**（本文の生成・会話の一覧・1つの会話のメッセージ）。
#[tokio::test]
async fn the_conversation_endpoints_answer_as_documented() {
    let spec = Spec::load();
    let server = TestServer::start(Vec::new()).await;
    call(
        &server,
        &spec,
        Method::POST,
        ("/admin/characters", "/admin/characters"),
        Some(json!({"handle": "aoi", "display_name": "葵", "base_persona": "ペルソナ本文"})),
        StatusCode::CREATED,
    )
    .await;

    let completion = call(
        &server,
        &spec,
        Method::POST,
        ("/v1/chat/completions", "/v1/chat/completions"),
        Some(json!({"model": "aoi", "messages": [{"role": "user", "content": "やあ"}]})),
        StatusCode::OK,
    )
    .await;
    let session = completion["model"]
        .as_str()
        .and_then(|model| model.strip_prefix("aoi@"))
        .expect("着地したセッションを名乗ること")
        .to_owned();

    call(
        &server,
        &spec,
        Method::GET,
        (
            "/admin/characters/aoi/sessions",
            "/admin/characters/{handle}/sessions",
        ),
        None,
        StatusCode::OK,
    )
    .await;
    let page = call(
        &server,
        &spec,
        Method::GET,
        (
            &format!("/admin/characters/aoi/sessions/{session}/messages?limit=10"),
            "/admin/characters/{handle}/sessions/{session_id}/messages",
        ),
        None,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        page["messages"].as_array().map(Vec::len),
        Some(2),
        "前提: 利用者と応答の2件を検証したこと: {page:#}"
    );

    server.stop().await;
}

/// **上流**（観測・プロファイルの一覧）。
#[tokio::test]
async fn the_upstream_endpoints_answer_as_documented() {
    let spec = Spec::load();
    let server = TestServer::start(Vec::new()).await;
    call(
        &server,
        &spec,
        Method::GET,
        ("/admin/upstream", "/admin/upstream"),
        None,
        StatusCode::OK,
    )
    .await;
    // **いま照会する口**（作業単位 B-66）。応答は `GET /admin/upstream` と同じ形。
    call(
        &server,
        &spec,
        Method::POST,
        ("/admin/upstream/check", "/admin/upstream/check"),
        None,
        StatusCode::OK,
    )
    .await;
    call(
        &server,
        &spec,
        Method::GET,
        ("/admin/upstream/profiles", "/admin/upstream/profiles"),
        None,
        StatusCode::OK,
    )
    .await;
    server.stop().await;
}

/// **配る仕様書**（`GET /openapi.json` / P-2b）。
///
/// 仕様書は最上位の必須の欄だけを名乗る（`OpenApiDocument`）ので、**余計な欄の走査
/// （[`Spec::undocumented`]）はかけない**——中身がまるごと生成物と一致することは
/// `tests/server_api_version.rs` が `api_document()` との等しさで見る。
#[tokio::test]
async fn the_served_document_answers_as_documented() {
    let spec = Spec::load();
    let server = TestServer::start(Vec::new()).await;
    let (status, value) = server.send(Method::GET, "/openapi.json", None).await;
    assert_eq!(status, StatusCode::OK, "{value:#}");
    let schema = spec.response_schema("/openapi.json", "get", 200);
    spec.validate(schema, &value)
        .unwrap_or_else(|err| panic!("配った仕様書が仕様書の形と食い違う: {err}"));
    // **対照: 必須の欄を欠いた文書は通らない**（型が何でも通す形に壊れていないこと）。
    assert!(
        spec.validate(schema, &json!({"openapi": "3.2.0", "info": {}}))
            .is_err(),
        "paths を欠いた文書が通った: {schema}"
    );
    server.stop().await;
}

/// **エラーは `default` の `ErrorBody`**（司令塔の決定）。
#[tokio::test]
async fn errors_answer_with_the_documented_error_body() {
    let spec = Spec::load();
    let server = TestServer::start(Vec::new()).await;

    // 404（無いキャラクター）・400（知らない欄）・409（差し替えも保存もできない起動）。
    call(
        &server,
        &spec,
        Method::GET,
        (
            "/admin/characters/nobody/stats",
            "/admin/characters/{handle}/stats",
        ),
        None,
        StatusCode::NOT_FOUND,
    )
    .await;
    call(
        &server,
        &spec,
        Method::PATCH,
        ("/admin/characters/nobody", "/admin/characters/{handle}"),
        Some(json!({"handle": "renamed"})),
        StatusCode::BAD_REQUEST,
    )
    .await;
    call(
        &server,
        &spec,
        Method::POST,
        ("/admin/upstream/profile", "/admin/upstream/profile"),
        Some(json!({"name": "x"})),
        StatusCode::CONFLICT,
    )
    .await;
    call(
        &server,
        &spec,
        Method::POST,
        ("/v1/chat/completions", "/v1/chat/completions"),
        Some(json!({"model": "nobody", "messages": [{"role": "user", "content": "やあ"}]})),
        StatusCode::NOT_FOUND,
    )
    .await;

    // **`Host` / `Origin` の検査の 403**（作業単位 B-58）。**口ごとに並べず `default` が受ける**
    // （`server::openapi` の doc「エラー」）。レイヤが返すので、handler の宣言には現れない。
    let response = server
        .client
        .put(format!("http://{}/admin/upstream/profiles/x", server.addr))
        .header(reqwest::header::ORIGIN, "http://evil.example")
        .json(&json!({"revision": "r", "main": {"endpoint": "http://127.0.0.1:1", "model": "m"}}))
        .send()
        .await
        .expect("サーバへ接続できること");
    let status = response.status();
    let headers = response.headers().clone();
    let value: Value = response.json().await.expect("JSON であること");
    assert_eq!(status, StatusCode::FORBIDDEN, "{value:#}");
    assert_eq!(value["error"]["code"], "request_origin_not_allowed");
    spec.check("/admin/upstream/profiles/{name}", "put", status, &value);
    let problems = spec.header_problems("/admin/upstream/profiles/{name}", "put", 403, &headers);
    assert!(
        problems.is_empty(),
        "403 のヘッダが仕様書と食い違う: {problems:#?}"
    );

    server.stop().await;
}

/// **中継が実際に出した各 `data` は、仕様書の `ChatCompletionEvent` の形**（司令塔の決定）。
///
/// 仕様書の SSE の形は**写し**である（中継は `serde_json::Value` を手で組む /
/// `server::openapi::ChatCompletionEvent` の doc）。**中継が形を変えたら、ここが赤になる。**
///
/// 2本の中継を流す: **尽きた回**（役割を名乗る先頭・名乗らない続き・停止理由・`[DONE]`）と、
/// **途中で壊れた回**（エラーの `data`、その後に `[DONE]` は来ない）。
#[tokio::test]
async fn every_relayed_event_matches_the_documented_event() {
    let spec = Spec::load();
    let server = TestServer::start(vec![
        vec![
            chunk("「こん", None),
            chunk("にちは", None),
            chunk("」", Some(FinishReason::Stop)),
        ],
        vec![chunk("「途中", None), Err(LlmError::Truncated)],
    ])
    .await;
    call(
        &server,
        &spec,
        Method::POST,
        ("/admin/characters", "/admin/characters"),
        Some(json!({"handle": "aoi", "display_name": "葵", "base_persona": "ペルソナ本文"})),
        StatusCode::CREATED,
    )
    .await;

    let mut data = Vec::new();
    for _ in 0..2 {
        let (status, body) = server
            .text(
                "/v1/chat/completions",
                json!({
                    "model": "aoi",
                    "messages": [{"role": "user", "content": "やあ"}],
                    "stream": true,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        data.extend(
            body.lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|line| line.trim().to_owned()),
        );
    }

    // **前提: 4つの形を全部検証したこと**（1つでも欠けると、その形は見ていない）。
    let parsed: Vec<Value> = data
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    assert!(data.iter().any(|line| line == "[DONE]"), "終端: {data:#?}");
    assert!(
        parsed
            .iter()
            .any(|value| value["choices"][0]["delta"]["role"].is_string()),
        "役割を名乗る先頭: {data:#?}"
    );
    assert!(
        parsed
            .iter()
            .any(|value| value["choices"][0]["delta"].is_object()
                && value["choices"][0]["delta"]["role"].is_null()),
        "役割を名乗らない続き: {data:#?}"
    );
    assert!(
        parsed.iter().any(|value| value["error"].is_object()),
        "途中で壊れた回のエラー: {data:#?}"
    );

    for line in &data {
        let problems = spec.event_problems(line);
        assert!(problems.is_empty(), "data: {line}\n{problems:#?}");
    }

    server.stop().await;
}

/// **上流プロファイルを編集できる起動**（既定の置き場所を渡す本番の入口 / P-2a のレビュー是正）。
///
/// **上の `TestServer` はプロファイルの置き場所を持たない**ので、保存は必ず 409 で断られ、
/// **保存の 422（鍵を足した本文）も、一覧が空でない応答も、保存・差し替えの 200 も一度も作らない**
/// ——そこが検証の外に残っていた（レビューの指摘: 422 の本文が仕様書と食い違っていたのに緑だった）。
///
/// **環境変数は読まない**。本文の上流は閉じた loopback（LM Studio に繋ぎに行かせない）。
/// 待ち受けは空いていたポートで起こし、**取られていたら起こし直す**（書き手の生存確認つき /
/// `tests/upstream_profile_editor.rs` の `serve` と同じ理由）。
struct EditableServer {
    port: u16,
    client: reqwest::Client,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), ServerError>>>,
}

impl EditableServer {
    async fn start(dir: &std::path::Path, profiles: std::path::PathBuf) -> Self {
        const MAX_BINDS: usize = 5;
        const MAX_WAITS: usize = 300;
        let client = reqwest::Client::builder()
            // **上限を持つ**（この機が飽和すると、ファイル操作が数十秒止まる回がある /
            // `tests/upstream_profile_editor.rs` の `REQUEST_TIMEOUT` の doc）。
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .expect("HTTP クライアントを作れること");
        for _ in 0..MAX_BINDS {
            let port = std::net::TcpListener::bind("127.0.0.1:0")
                .and_then(|listener| listener.local_addr())
                .expect("ポートを取れること")
                .port();
            // **番号は `PERSONACORE_PORT` で明示する**（作業単位 F-11b）——取られていたら待ち受けを開けずに返り、
            // 下の起こし直しに入る。明示していない番号（`with_bind`）は、取られていれば次の空きへずれて起き、
            // このポートには別の誰かが答える。
            let port_text = port.to_string();
            let config = AppConfig::from_lookup_with_data_dir(
                move |key: &str| match key {
                    ENV_LLM_ENDPOINT => Some("http://127.0.0.1:9".to_owned()),
                    ENV_PORT => Some(port_text.clone()),
                    _ => None,
                },
                Some(dir.to_path_buf()),
            )
            .expect("設定を組めること")
            .with_database_path(dir.join("db").join("personacore.sqlite"));
            let startup =
                Startup::new(LaunchPlan::none()).with_editable_profiles_file(profiles.clone());
            let (shutdown, wait) = oneshot::channel();
            let task = tokio::spawn(async move {
                App::run_with_upstreams(&config, startup, async move {
                    let _ = wait.await;
                })
                .await
            });
            let mut server = Self {
                port,
                client: client.clone(),
                shutdown: Some(shutdown),
                task: Some(task),
            };
            for _ in 0..MAX_WAITS {
                if server.task.as_ref().is_none_or(JoinHandle::is_finished) {
                    break;
                }
                let answered = server
                    .client
                    .get(format!("http://127.0.0.1:{port}/admin/upstream/profiles"))
                    .send()
                    .await;
                if let Ok(response) = answered
                    && let Ok(body) = response.json::<Value>().await
                    && body["editable"] == json!(true)
                {
                    return server;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            // 待ち受けを開けなかった（別の誰かにポートを取られた）。起こし直す。
            server.task = None;
        }
        panic!("{MAX_BINDS} 回起こし直しても待ち受けが答えなかった");
    }

    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = self
            .client
            .request(method, format!("http://127.0.0.1:{}{path}", self.port));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.expect("サーバへ接続できること");
        let status = response.status();
        let body = response.json::<Value>().await.expect("JSON であること");
        (status, body)
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.await
                .expect("サーバタスクが panic しないこと")
                .expect("正常に終了すること");
        }
    }
}

/// **上流プロファイルの口を、編集できる起動で**（P-2a のレビュー是正 / 提案B）。
///
/// | 作る応答 | 検証すること |
/// |---|---|
/// | 一覧（空でない） | `WrittenProfile` / 応答の `WrittenFace`（書いた鍵と `null` の鍵の両方。`context_tokens` も） |
/// | 保存の 200 | 保存した直後の一覧（依頼に `context_tokens` を載せる） |
/// | **保存の 422 `upstream_profile_invalid`** | **`error` の下に `reason` / `reason_message` / `face` / `key` が足された形**（`ShapeReason` の綴り・`face` / `key` の綴り。`key` が `context_tokens` の腕も） |
/// | 差し替えの 200 | `UpstreamReport` |
/// | 検証に通らないファイルの一覧 | `ProblemView`（`reason` / `face` / `key` が `null` でない） |
/// | 保存の 422 `upstream_profiles_unreadable` | `ErrorBody` のままの形（**同じ 422 のもう1つの腕**） |
#[tokio::test]
async fn the_profile_endpoints_answer_as_documented_when_the_file_is_editable() {
    const LIST: &str = "/admin/upstream/profiles";
    const SAVE: &str = "/admin/upstream/profiles/{name}";
    let spec = Spec::load();
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("upstreams.json");
    std::fs::write(
        &path,
        json!({
            "north": {
                "main": {"endpoint": "http://127.0.0.1:47101", "model": "north-main",
                         "gguf": "~/models/north-main.gguf", "args": ["--ctx-size", "4096"],
                         "command": "C:/tools/north-llama.exe", "context_tokens": 28672},
                "sub": {"endpoint": "http://127.0.0.1:47102", "model": "north-sub"}
            },
            "south": {"main": {"endpoint": "http://127.0.0.1:47103", "model": "south-main"}}
        })
        .to_string(),
    )
    .expect("プロファイルを置けること");
    let server = EditableServer::start(dir.path(), path.clone()).await;

    let (status, listing) = server.send(Method::GET, LIST, None).await;
    assert_eq!(status, StatusCode::OK, "{listing:#}");
    spec.check(LIST, "get", status, &listing);
    let profiles = listing["profiles"].as_array().expect("一覧があること");
    assert_eq!(
        profiles.len(),
        2,
        "前提: 空でない一覧を検証したこと: {listing:#}"
    );
    assert!(
        profiles.iter().any(|p| p["sub"].is_null())
            && profiles.iter().any(|p| p["sub"].is_object()),
        "前提: sub が無いプロファイルと有るプロファイルの両方: {listing:#}"
    );
    assert!(
        profiles.iter().any(|p| p["main"]["gguf"].is_string())
            && profiles.iter().any(|p| p["main"]["gguf"].is_null()),
        "前提: 書いた鍵と書いていない鍵（null）の両方: {listing:#}"
    );
    // **`context_tokens` も、書いた（整数）面と書いていない（null）面の両方**（作業単位 A-13a-2）。
    assert!(
        profiles
            .iter()
            .any(|p| p["main"]["context_tokens"].is_u64())
            && profiles
                .iter()
                .any(|p| p["main"]["context_tokens"].is_null()),
        "前提: context_tokens を書いた面と書いていない面の両方: {listing:#}"
    );

    // **保存の 200**（宛先は答える偽物 —— 差し替えの測り直しが別の誰かに当たらないように）。
    // **依頼にも `context_tokens` を載せる**（要求の形 `WrittenFaceRequest` の欄）。
    let upstream = wiremock::MockServer::start().await;
    let (status, saved) = server
        .send(
            Method::PUT,
            "/admin/upstream/profiles/west",
            Some(json!({"revision": listing["revision"],
                        "main": {"endpoint": upstream.uri(), "model": "west-model",
                                 "context_tokens": 20480}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved:#}");
    spec.check(SAVE, "put", status, &saved);

    // **保存の 422（鍵が `context_tokens`）**——仕様書の `key` の閉じた綴りがこの鍵を含むこと。
    let (status, window) = server
        .send(
            Method::PUT,
            "/admin/upstream/profiles/east",
            Some(json!({"revision": saved["revision"],
                        "main": {"endpoint": upstream.uri(), "model": "east-model",
                                 "context_tokens": 1024}})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{window:#}");
    assert_eq!(
        (&window["error"]["reason"], &window["error"]["key"]),
        (&json!("context_tokens_unusable"), &json!("context_tokens")),
        "前提: context_tokens を名指す 422 を作ったこと: {window:#}"
    );
    spec.check(SAVE, "put", status, &window);

    // **保存の 422（鍵を足した形）**。gguf を起こす面の宛先が loopback でない
    // → `endpoint_not_loopback` / `main` / `endpoint`（gguf が無い面の宛先は外でもよい）。
    let (status, invalid) = server
        .send(
            Method::PUT,
            "/admin/upstream/profiles/east",
            Some(json!({"revision": saved["revision"],
                        "main": {"endpoint": "http://192.168.10.2:1", "model": "east-model",
                                 "gguf": "/models/east.gguf"}})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{invalid:#}");
    assert_eq!(
        (
            &invalid["error"]["code"],
            &invalid["error"]["reason"],
            &invalid["error"]["face"],
            &invalid["error"]["key"],
        ),
        (
            &json!("upstream_profile_invalid"),
            &json!("endpoint_not_loopback"),
            &json!("main"),
            &json!("endpoint"),
        ),
        "前提: 鍵を足した形の 422 を作ったこと: {invalid:#}"
    );
    spec.check(SAVE, "put", status, &invalid);

    // **差し替えの 200**。
    let (status, report) = server
        .send(
            Method::POST,
            "/admin/upstream/profile",
            Some(json!({"name": "west"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{report:#}");
    spec.check("/admin/upstream/profile", "post", status, &report);

    // **検証に通らないファイル**の一覧（`ProblemView`）と、そのときの保存の 422（`ErrorBody` の腕）。
    std::fs::write(
        &path,
        json!({"bad": {"main": {"endpoint": "http://192.168.10.2:1", "model": "m",
                                "gguf": "/models/bad.gguf"}}})
        .to_string(),
    )
    .expect("書き換えられること");
    let (status, broken) = server.send(Method::GET, LIST, None).await;
    assert_eq!(status, StatusCode::OK, "{broken:#}");
    assert!(
        broken["problem"]["face"].is_string() && broken["problem"]["key"].is_string(),
        "前提: face / key のある ProblemView を検証したこと: {broken:#}"
    );
    spec.check(LIST, "get", status, &broken);
    let (status, unreadable) = server
        .send(
            Method::PUT,
            "/admin/upstream/profiles/west",
            Some(json!({"revision": broken["revision"],
                        "main": {"endpoint": upstream.uri(), "model": "west-model"}})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{unreadable:#}");
    assert_eq!(
        unreadable["error"]["code"],
        json!("upstream_profiles_unreadable"),
        "前提: ErrorBody の形の 422 を作ったこと: {unreadable:#}"
    );
    spec.check(SAVE, "put", status, &unreadable);

    server.stop().await;
}

/// **待ち受けのポートの口を、保存できる起動で**（ADR-0057 §決定6 / 作業単位 F-12a）。
///
/// | 作る応答 | 検証すること |
/// |---|---|
/// | `GET /admin/server`（保存なし） | `ServerView`（`saved_port` が `null`） |
/// | 保存の 200（空いている番号・掴まれた番号） | `SavePortView`（`saved_port_in_use` が `false` と `true`。`saved_port` が整数） |
/// | 保存を消した 200 | `SavePortView`（`saved_port` と `saved_port_in_use` が `null`） |
/// | 400 `invalid_listening_port` / 400 `invalid_body` | `default` の `ErrorBody` |
#[tokio::test]
async fn the_server_endpoints_answer_as_documented_when_the_file_is_editable() {
    const GET: &str = "/admin/server";
    const PUT: &str = "/admin/server/port";
    let spec = Spec::load();
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("upstreams.json");
    std::fs::write(&path, "{}").expect("プロファイルを置けること");
    let server = EditableServer::start(dir.path(), path.clone()).await;

    let (status, report) = server.send(Method::GET, GET, None).await;
    assert_eq!(status, StatusCode::OK, "{report:#}");
    assert!(report["saved_port"].is_null(), "前提: 保存なし: {report:#}");
    spec.check(GET, "get", status, &report);

    let free = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("ポートを取れること")
        .port();
    let (status, saved) = server
        .send(Method::PUT, PUT, Some(json!({ "port": free })))
        .await;
    assert_eq!(status, StatusCode::OK, "{saved:#}");
    assert_eq!(saved["saved_port_in_use"], json!(false), "前提: {saved:#}");
    spec.check(PUT, "put", status, &saved);

    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("ポートを取れること");
    let held_port = held.local_addr().expect("宛先を読めること").port();
    let (status, taken) = server
        .send(Method::PUT, PUT, Some(json!({ "port": held_port })))
        .await;
    assert_eq!(status, StatusCode::OK, "{taken:#}");
    assert_eq!(taken["saved_port_in_use"], json!(true), "前提: {taken:#}");
    spec.check(PUT, "put", status, &taken);
    drop(held);

    let (status, read) = server.send(Method::GET, GET, None).await;
    assert_eq!(read["saved_port"], json!(held_port), "前提: {read:#}");
    spec.check(GET, "get", status, &read);

    let (status, forgot) = server
        .send(Method::PUT, PUT, Some(json!({ "port": null })))
        .await;
    assert_eq!(status, StatusCode::OK, "{forgot:#}");
    assert!(
        forgot["saved_port"].is_null() && forgot["saved_port_in_use"].is_null(),
        "前提: {forgot:#}"
    );
    spec.check(PUT, "put", status, &forgot);

    for (body, code) in [
        (json!({ "port": 80 }), "invalid_listening_port"),
        (json!({}), "invalid_body"),
    ] {
        let (status, refused) = server.send(Method::PUT, PUT, Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {refused:#}");
        assert_eq!(refused["error"]["code"], json!(code), "{body}: {refused:#}");
        spec.check(PUT, "put", status, &refused);
    }

    server.stop().await;
}

/// **対照: 検証の道具は、仕様書と食い違う応答を通さない**（植えた応答）。
///
/// 道具が何でも通す形に壊れていると、上の検証は全部緑のまま何も見ていない。
#[tokio::test]
async fn the_checker_refuses_planted_responses() {
    let spec = Spec::load();
    let character = json!({
        "id": 1,
        "handle": "aoi",
        "display_name": "葵",
        "base_persona": "ペルソナ本文",
        "output_format_instruction": null,
        "stat_scale": 5,
        "stat_drift_mode": "shadow",
        "axis_discovery_slots": 0,
        "created_at": "2026-09-30T00:00:00Z",
        "archived_at": null,
    });
    let list = |row: Value| json!({"characters": [row], "unreadable": []});
    let problems = |body: &Value| spec.problems("/admin/characters", "get", 200, body);

    // **ヘッダの検証も何でも通す形に壊れていないこと**（成功と `default` の両方の腕）。
    let mut stamped = reqwest::header::HeaderMap::new();
    stamped.insert(
        "X-Personacore-Api-Version",
        reqwest::header::HeaderValue::from_static("1.0-planted"),
    );
    for status in [200, 404] {
        assert_eq!(
            spec.header_problems(
                "/admin/characters",
                "get",
                status,
                &reqwest::header::HeaderMap::new()
            )
            .len(),
            1,
            "版のヘッダが無い {status} の応答が通った"
        );
        assert_eq!(
            spec.header_problems("/admin/characters", "get", status, &stamped),
            Vec::<String>::new(),
            "対照の対照: ヘッダがあれば通る（{status}）"
        );
    }

    assert_eq!(
        problems(&list(character.clone())),
        Vec::<String>::new(),
        "対照の対照: 正しい形は通る"
    );

    let mut extra = character.clone();
    extra["nickname"] = json!("あーちゃん");
    let mut missing = character.clone();
    missing
        .as_object_mut()
        .expect("オブジェクト")
        .remove("output_format_instruction");
    let mut wrong_enum = character.clone();
    wrong_enum["stat_drift_mode"] = json!("true");
    let mut wrong_time = character;
    wrong_time["created_at"] = json!("きのう");
    for (label, row) in [
        ("仕様書に無い欄", extra),
        ("必須の欄が無い", missing),
        ("enum に無い綴り", wrong_enum),
        ("date-time でない時刻", wrong_time),
    ] {
        assert!(!problems(&list(row)).is_empty(), "{label} を通してしまった");
    }

    // SSE: 増分に余計な欄・終端の綴り違い・JSON の中身が型に合わない。
    let good = r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1,"model":"aoi@1","choices":[{"index":0,"delta":{"content":"a"},"finish_reason":null}]}"#;
    assert_eq!(
        spec.event_problems(good),
        Vec::<String>::new(),
        "対照の対照: 正しい増分は通る"
    );
    assert_eq!(spec.event_problems("[DONE]"), Vec::<String>::new());
    for (label, data) in [
        (
            "増分に余計な欄",
            r#"{"id":"chatcmpl-x","object":"chat.completion.chunk","created":1,"model":"aoi@1","choices":[{"index":0,"delta":{"content":"a"},"finish_reason":null,"logprobs":null}]}"#,
        ),
        ("終端の綴り違い", "[done]"),
        (
            "object の綴り違い",
            r#"{"id":"chatcmpl-x","object":"chat.completion","created":1,"model":"aoi@1","choices":[{"index":0,"delta":{"content":"a"},"finish_reason":null}]}"#,
        ),
    ] {
        assert!(
            !spec.event_problems(data).is_empty(),
            "{label} を通してしまった"
        );
    }
}

// ----------------------------------------------------------------------
// 仕込み
// ----------------------------------------------------------------------

async fn record_vote(
    store: &Store,
    id: CharacterId,
    session: SessionId,
    axis: &str,
    delta: DriftDelta,
) {
    let current = store.stat_allocs(id).await.expect("性格値を読めること");
    let outcome = apply_drift(
        &current,
        DriftProposal::Suggested(DriftSuggestion {
            axis: StatAxis::parse(axis).expect("正当な軸名"),
            delta,
        }),
    );
    assert!(
        matches!(
            outcome,
            DriftOutcome::Applied(_) | DriftOutcome::Saturated { .. }
        ),
        "門を通る提案であること: {outcome:?}"
    );
    store
        .record_drift_vote(id, session, &outcome.vote().expect("票になること"))
        .await
        .expect("票を記録できること");
}

async fn add_memory(store: &Store, id: CharacterId, subject: Subject, content: &str) -> MemoryId {
    store
        .add_memory(
            id,
            &NewMemory {
                subject,
                content: MemoryContent::parse(content).expect("正当な本文"),
                category: None,
            },
        )
        .await
        .expect("記憶を追加できること")
}

async fn propose(store: &Store, id: CharacterId, name: &str, source: MemoryId) -> AxisCandidateId {
    store
        .record_axis_candidate(
            id,
            &AxisCandidateRecord::Propose {
                name: StatAxis::parse(name).expect("正当な軸名"),
                source_memory: source,
            },
        )
        .await
        .expect("候補を作れること")
}
