//! **API の版のヘッダと、仕様書を配る口**（ADR-0047 §② P-2b / Issue origin#46）の検証。
//! **LM Studio には繋がない**（偽の上流を注入する）。
//!
//! # 何を固定するか
//!
//! | 主張 | 破れると |
//! |---|---|
//! | **どの種類の応答にも** `X-Personacore-Api-Version` が付き、値が [`API_VERSION`] | 使い手が版を見て分岐できない応答が残る |
//! | `GET /openapi.json` の本文 == `api_document()`、`content-type` は JSON | 配っている仕様書が、テストが検証している仕様書と別物になる |
//! | 仕様書の `info.version` == ヘッダの値 | 2つの口が別の版を名乗る |
//!
//! **応答の種類は全部並べる**: 成功（JSON）・handler のエラー・API の 404・405・
//! 本文の上限（レイヤが返す 413）・**`Host` / `Origin` の検査（レイヤが返す 403 / 作業単位 B-58）**・
//! 壊れた JSON（400）・画面（`/` と資産）・画面の 404・
//! SSE・`/openapi.json`。**ヘッダはレイヤで付けているので、レイヤの外で作られる応答
//! （フォールバック・405・上限のミドルウェア）が漏れやすい**——それを1本ずつ叩く。
//!
//! **画面なし（`--no-ui`）の腕は本物の実行ファイルを通す**
//! （`tests/upstream_cli.rs::the_no_ui_flag_withholds_the_screen_and_leaves_the_api_byte_for_byte`）。

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use personacore::llm::{
    ChatChunk, ChatRequest, ChatStream, Completion, FinishReason, LlmClient, LlmError, LlmFuture,
};
use personacore::server::{
    API_VERSION, App, AppConfig, BindAddr, ENV_WEB_DIR, ServerError, api_document,
};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// **ヘッダの名前**（司令塔の決定の綴り。**定数から作らない**——大文字小文字は HTTP では区別しない）。
const HEADER: &str = "X-Personacore-Api-Version";

/// 画面の配り先に置く目印（`vite` が出さない綴り）。
const UI_INDEX: &str = "<!doctype html><title>API-VERSION-INDEX</title>";
const UI_ASSET_PATH: &str = "assets/api-version-marker.js";
const UI_ASSET: &str = "/* API-VERSION-ASSET */";

/// 本文と中継を返す偽の上流。
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

fn chunk(delta: &str, finish_reason: Option<FinishReason>) -> Result<ChatChunk, LlmError> {
    Ok(ChatChunk {
        delta: delta.to_owned(),
        finish_reason,
        usage: None,
    })
}

struct TestServer {
    addr: SocketAddr,
    client: reqwest::Client,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

/// 1回の応答（状態・`content-type`・版のヘッダ・本文）。
struct Answer {
    status: StatusCode,
    content_type: Option<String>,
    versions: Vec<String>,
    body: Vec<u8>,
}

impl TestServer {
    /// **画面を配る起動**（配り先は目印入りの一時ディレクトリ）。
    async fn start() -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let web_dir = dir.path().join("web-dist");
        std::fs::create_dir_all(web_dir.join("assets")).expect("配り先を作れること");
        std::fs::write(web_dir.join("index.html"), UI_INDEX).expect("置けること");
        std::fs::write(web_dir.join(UI_ASSET_PATH), UI_ASSET).expect("置けること");
        let web_dir = web_dir.display().to_string();
        let config = AppConfig::from_lookup(|key| (key == ENV_WEB_DIR).then(|| web_dir.clone()))
            .expect("設定を組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(dir.path().join("personacore.sqlite"));
        let llm = Arc::new(MockLlm {
            streams: Mutex::new(
                vec![vec![
                    chunk("「こん", None),
                    chunk("にちは」", Some(FinishReason::Stop)),
                ]]
                .into(),
            ),
        });
        let app = App::bind_with_client(&config, llm)
            .await
            .expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });
        Self {
            addr,
            client: reqwest::Client::new(),
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    async fn exchange(&self, method: Method, path: &str, body: Option<Vec<u8>>) -> Answer {
        self.exchange_with(method, path, body, None).await
    }

    /// [`Self::exchange`] に、見出しを1つ足したもの（`Origin` の検査の 403 を作るため）。
    async fn exchange_with(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        header: Option<(&str, &str)>,
    ) -> Answer {
        let mut request = self
            .client
            .request(method, format!("http://{}{path}", self.addr))
            .timeout(std::time::Duration::from_secs(120));
        if let Some((name, value)) = header {
            request = request.header(name, value);
        }
        if let Some(body) = body {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        let response = request.send().await.expect("サーバへ接続できること");
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .map(|value| value.to_str().unwrap_or("<not ascii>").to_owned());
        let versions = response
            .headers()
            .get_all(HEADER)
            .iter()
            .map(|value| value.to_str().unwrap_or("<not ascii>").to_owned())
            .collect();
        let body = response.bytes().await.expect("本文を読めること").to_vec();
        Answer {
            status,
            content_type,
            versions,
            body,
        }
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

fn json_body(value: &Value) -> Option<Vec<u8>> {
    Some(value.to_string().into_bytes())
}

/// 1つの種類: (種類, メソッド, パス, 本文, 期待する状態, `content-type` の書き出し, 本文に含む目印)。
type Case = (
    &'static str,
    Method,
    &'static str,
    Option<Vec<u8>>,
    StatusCode,
    &'static str,
    &'static str,
);

/// **どの種類の応答にも、版のヘッダが1つだけ付き、値は [`API_VERSION`]。**
///
/// **種類ごとに「その種類の応答が本当に返った」ことを先に確かめる**
/// （状態・`content-type`・本文の目印）——確かめないと、全部が同じ 404 でも緑になる。
#[tokio::test]
async fn every_kind_of_response_carries_the_api_version() {
    let server = TestServer::start().await;

    let created = server
        .exchange(
            Method::POST,
            "/admin/characters",
            json_body(
                &json!({"handle": "aoi", "display_name": "葵", "base_persona": "ペルソナ本文"}),
            ),
        )
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );

    let oversized = {
        let raw = json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "あ".repeat(1_000_000)}],
        })
        .to_string()
        .into_bytes();
        assert!(raw.len() > 2 * 1024 * 1024, "前提: 本文の上限を超えること");
        raw
    };

    // (種類, メソッド, パス, 本文, 期待する状態, content-type の書き出し, 本文に含む目印)
    let cases: Vec<Case> = vec![
        (
            "作成（201）",
            Method::POST,
            "/admin/characters",
            json_body(&json!({"handle": "mio", "display_name": "澪", "base_persona": "本文"})),
            StatusCode::CREATED,
            "application/json",
            "mio",
        ),
        (
            "成功（JSON）",
            Method::GET,
            "/v1/models",
            None,
            StatusCode::OK,
            "application/json",
            "aoi",
        ),
        (
            "本文（JSON・OpenAI 互換）",
            Method::POST,
            "/v1/chat/completions",
            json_body(&json!({"model": "aoi", "messages": [{"role": "user", "content": "やあ"}]})),
            StatusCode::OK,
            "application/json",
            "chat.completion",
        ),
        (
            "SSE",
            Method::POST,
            "/v1/chat/completions",
            json_body(&json!({
                "model": "aoi",
                "messages": [{"role": "user", "content": "やあ"}],
                "stream": true,
            })),
            StatusCode::OK,
            "text/event-stream",
            "[DONE]",
        ),
        (
            "handler のエラー（404）",
            Method::GET,
            "/admin/characters/nobody/stats",
            None,
            StatusCode::NOT_FOUND,
            "application/json",
            "\"error\"",
        ),
        (
            "壊れた JSON（400）",
            Method::POST,
            "/admin/characters",
            Some(b"{not json".to_vec()),
            StatusCode::BAD_REQUEST,
            "application/json",
            "\"error\"",
        ),
        (
            "本文の上限（413・レイヤが返す）",
            Method::POST,
            "/v1/chat/completions",
            Some(oversized),
            StatusCode::PAYLOAD_TOO_LARGE,
            "application/json",
            "body_too_large",
        ),
        (
            "API の未知パス（404・フォールバック）",
            Method::GET,
            "/admin/does-not-exist",
            None,
            StatusCode::NOT_FOUND,
            "application/json",
            "\"error\"",
        ),
        (
            "未対応メソッド（405）",
            Method::DELETE,
            "/v1/models",
            None,
            StatusCode::METHOD_NOT_ALLOWED,
            "application/json",
            "\"error\"",
        ),
        (
            "画面（/）",
            Method::GET,
            "/",
            None,
            StatusCode::OK,
            "text/html",
            "API-VERSION-INDEX",
        ),
        (
            "画面の資産",
            Method::GET,
            "/assets/api-version-marker.js",
            None,
            StatusCode::OK,
            "text/javascript",
            "API-VERSION-ASSET",
        ),
        (
            "画面の 404",
            Method::GET,
            "/assets/never-built.js",
            None,
            StatusCode::NOT_FOUND,
            "",
            "",
        ),
        (
            "仕様書",
            Method::GET,
            "/openapi.json",
            None,
            StatusCode::OK,
            "application/json",
            "\"openapi\"",
        ),
    ];
    assert_eq!(cases.len(), 13, "前提: 並べた種類の数");

    for (kind, method, path, body, status, content_type, marker) in cases {
        let answer = server.exchange(method, path, body).await;
        let text = String::from_utf8_lossy(&answer.body);
        assert_eq!(answer.status, status, "前提（{kind}）: {text}");
        if !content_type.is_empty() {
            let actual = answer.content_type.as_deref().unwrap_or("");
            assert!(
                actual.starts_with(content_type),
                "前提（{kind}）: content-type が {actual}"
            );
        }
        assert!(
            text.contains(marker),
            "前提（{kind}）: 本文に {marker}: {text}"
        );
        assert_eq!(
            answer.versions,
            vec![API_VERSION.to_owned()],
            "{kind}（{path}）の版のヘッダ"
        );
    }

    // **`Host` / `Origin` の検査（403・レイヤが返す / 作業単位 B-58）。** 見出しが要るので表の外で叩く。
    // 画面のパスと API のパスの両方。
    for path in ["/v1/models", "/"] {
        let answer = server
            .exchange_with(
                Method::GET,
                path,
                None,
                Some(("Origin", "http://evil.example")),
            )
            .await;
        let text = String::from_utf8_lossy(&answer.body);
        assert_eq!(
            answer.status,
            StatusCode::FORBIDDEN,
            "前提（403 / {path}）: {text}"
        );
        assert!(
            text.contains("request_origin_not_allowed"),
            "前提（403 / {path}）: {text}"
        );
        assert_eq!(
            answer.versions,
            vec![API_VERSION.to_owned()],
            "Host / Origin の検査の 403（{path}）の版のヘッダ"
        );
    }

    server.stop().await;
}

/// **`GET /openapi.json` は `api_document()` をそのまま返し、`info.version` はヘッダと同じ。**
#[tokio::test]
async fn the_served_document_is_the_generated_one_and_names_the_header_version() {
    let server = TestServer::start().await;
    let answer = server.exchange(Method::GET, "/openapi.json", None).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.content_type.as_deref(), Some("application/json"));
    let served: Value = serde_json::from_slice(&answer.body).expect("JSON であること");
    let generated = api_document().expect("仕様書は JSON にできること");
    assert_eq!(served, generated);
    assert!(
        served["paths"]["/openapi.json"]["get"].is_object(),
        "配る口自身も仕様書に載っていること"
    );
    assert_eq!(answer.versions, vec![API_VERSION.to_owned()]);
    assert_eq!(served["info"]["version"], json!(answer.versions[0]));
    server.stop().await;
}
