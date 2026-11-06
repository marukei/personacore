//! `POST /v1/chat/completions` の **SSE 中継**（作業単位 11a-2b）の検証。
//!
//! **実際の LM Studio には繋がない**（docs/05-development.md）。上流は `LlmClient` の
//! モックを注入し、こちらが用意したチャンク列をそのまま流させる。
//!
//! ここで固定するのは次の点:
//!
//!   * チャンクが**順序どおり**流れ、**`data: [DONE]` で終端する**こと。
//!   * **溜め込まずに流す**こと（docs/02-architecture.md）。
//!     上流がまだ喋り終えていない時点で、先に来たチャンクが下流へ届く。
//!   * 各チャンクの `model` が**非ストリーミングと同じ `aoi@<session-id>`** であること。
//!   * **中継の途中で上流が壊れたとき**の伝え方。
//!     200 を送った後なのでステータスでは伝えられないため、
//!     **error オブジェクトを1件流し、`[DONE]` は送らずに終端する。**
//!   * 上流が**開く前に**失敗したときは、従来どおり 4xx/5xx の JSON で返すこと。
//!   * `stream: false` / 省略時が**従来どおり**であること（回帰）。
//!   * **中継が完結した回だけ往復を保存する**こと（11b-2）。途中で壊れた回は1行も残さない。
//!
//! **パース成功率のログは `tests/server_chat_parse_log.rs`**（両経路をまとめて見る。
//! このバイナリでグローバルの subscriber を差せるテストは1つだけなので分けてある）。
//! 無通信の見張りと切断時キャンセルは 11c-3 なので、ここでは扱わない。

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use personacore::domain::{DisplayName, Handle, Message, Persona, Role};
use personacore::llm::{
    ChatChunk, ChatRequest, ChatStream, Completion, FinishReason, LlmClient, LlmError, LlmFuture,
    TokenUsage,
};
use personacore::prompt::TokenBudget;
use personacore::server::{App, AppConfig, BindAddr, ENV_CONTEXT_TOKENS, ServerError};
use personacore::store::Store;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

use common::{fixture_scale, has_field, install_global_log, line_containing};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;

/// 上流（LM Studio）の代役。**ネットワークには一切触らない。**
struct MockLlm {
    /// `stream` が返すチャンク列。**呼ばれるたびに先頭から1本ずつ取り出す。**
    ///
    /// 多くのテストは1本しか要らないが、**セッションを跨ぐ検証では
    /// 中継を3回開く**必要があり、しかも回ごとに別の本文を流さないと
    /// 「どの回がどのセッションへ入ったか」を区別できない。
    sources: Mutex<VecDeque<ChatStream>>,
    /// `stream` の future そのものを失敗させる（上流を開く前の失敗）。
    open_failure: Mutex<Option<LlmError>>,
    /// `stream` が受け取った要求。
    stream_requests: Mutex<Vec<ChatRequest>>,
    /// `complete`（非ストリーミング）が呼ばれた回数。
    complete_calls: Mutex<usize>,
    /// `complete` が返す応答。
    reply: Completion,
    /// **中継を始める直前に永続化層を壊す**（保存だけを失敗させるための細工）。
    ///
    /// セッション解決も上流を開くのも成功した後で保存だけが失敗する状況を作る。
    /// これが無いと「保存の失敗で中継を壊さない」を見張るテストが書けない。
    sabotage: Mutex<Option<Store>>,
}

impl MockLlm {
    fn with_source(source: Option<ChatStream>) -> Arc<Self> {
        Arc::new(Self {
            sources: Mutex::new(source.into_iter().collect()),
            open_failure: Mutex::new(None),
            stream_requests: Mutex::new(Vec::new()),
            complete_calls: Mutex::new(0),
            reply: Completion {
                content: "非ストリーミングの本文".to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            },
            sabotage: Mutex::new(None),
        })
    }

    /// **次の `stream` で永続化層を閉じる。**
    fn break_the_store_on_the_next_call(&self, store: Store) {
        *self.sabotage.lock().expect("ロックを取れること") = Some(store);
    }

    /// 用意したチャンク列を順に流し、**正常に終端する**モック。
    fn streaming(items: Vec<Result<ChatChunk, LlmError>>) -> Arc<Self> {
        Self::with_source(Some(Box::pin(futures_util::stream::iter(items))))
    }

    /// **中継を開くたびに違うチャンク列を流す**モック（多セッションの検証用）。
    ///
    /// 用意した本数を超えて開かれたら `MissingContent` になる（黙って
    /// 同じ本文を流し続けると、どの回の応答かが区別できなくなるため）。
    fn streaming_each(streams: Vec<Vec<Result<ChatChunk, LlmError>>>) -> Arc<Self> {
        let mock = Self::with_source(None);
        *mock.sources.lock().expect("ロックを取れること") = streams
            .into_iter()
            .map(|items| -> ChatStream { Box::pin(futures_util::stream::iter(items)) })
            .collect();
        mock
    }

    /// **外から1件ずつ送り込める**モック（溜め込んでいないことの検証用）。
    fn streaming_from(rx: mpsc::Receiver<Result<ChatChunk, LlmError>>) -> Arc<Self> {
        Self::with_source(Some(Box::pin(ReceiverStream::new(rx))))
    }

    /// **上流を開く前に**失敗するモック。
    fn failing_to_open(error: LlmError) -> Arc<Self> {
        let mock = Self::with_source(None);
        *mock.open_failure.lock().expect("ロックを取れること") = Some(error);
        mock
    }

    fn stream_count(&self) -> usize {
        self.stream_requests
            .lock()
            .expect("ロックを取れること")
            .len()
    }

    fn last_stream_request(&self) -> ChatRequest {
        self.stream_requests
            .lock()
            .expect("ロックを取れること")
            .last()
            .cloned()
            .expect("中継を1回以上要求していること")
    }

    fn complete_count(&self) -> usize {
        *self.complete_calls.lock().expect("ロックを取れること")
    }
}

impl LlmClient for MockLlm {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        Box::pin(async move {
            *self.complete_calls.lock().expect("ロックを取れること") += 1;
            Ok(self.reply.clone())
        })
    }

    fn stream<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        Box::pin(async move {
            self.stream_requests
                .lock()
                .expect("ロックを取れること")
                .push(request.clone());
            if let Some(error) = self.open_failure.lock().expect("ロックを取れること").take()
            {
                return Err(error);
            }
            // **ロックを握ったまま `await` しない**（取り出してから閉じる）。
            let sabotage = self.sabotage.lock().expect("ロックを取れること").take();
            if let Some(store) = sabotage {
                store.close().await;
            }
            self.sources
                .lock()
                .expect("ロックを取れること")
                .pop_front()
                .ok_or(LlmError::MissingContent)
        })
    }
}

/// 増分だけのチャンク。**`usage` は載らない**（上流の大半のチャンクがこの形）。
fn chunk(delta: &str) -> Result<ChatChunk, LlmError> {
    Ok(ChatChunk {
        delta: delta.to_owned(),
        finish_reason: None,
        usage: None,
    })
}

/// 停止理由を載せた最後のチャンク。
fn last_chunk(delta: &str) -> Result<ChatChunk, LlmError> {
    Ok(ChatChunk {
        delta: delta.to_owned(),
        finish_reason: Some(FinishReason::Stop),
        usage: None,
    })
}

/// **使用量だけを載せたチャンク。**
///
/// `stream_options.include_usage` を要求した上流が、本文を出し切った後に
/// `{"choices":[],"usage":{…}}` として送る形（2026-08-15 に LM Studio で実測）。
/// `llm::parse_chunk` がこれを「空の増分を持つチャンク」に均すので、
/// **中継側が意識して落とさないと下流へ空の増分が1件余分に流れる。**
fn usage_only_chunk(prompt_tokens: u32) -> Result<ChatChunk, LlmError> {
    Ok(ChatChunk {
        delta: String::new(),
        finish_reason: None,
        usage: Some(TokenUsage { prompt_tokens }),
    })
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
    async fn start(llm: Arc<MockLlm>) -> Self {
        Self::start_with_env(llm, &[]).await
    }

    async fn start_with_env(llm: Arc<MockLlm>, env: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db_path = dir.path().join("personacore.sqlite");

        let pairs: Vec<(String, String)> = env
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        let config = AppConfig::from_lookup(|key| {
            pairs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(&db_path);

        // **モックを注入する。** ここが実際の LM Studio に繋がらないことの根拠。
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

    /// キャラクターを作る。**識別子と表示名は必ず別の値**（`server_chat.rs` と同じ理由）。
    async fn seed(&self, raw_name: &str, raw_display: &str) {
        assert_ne!(
            raw_name, raw_display,
            "識別子と表示名は別の値にすること（同値だとどちらが載ったか区別できない）"
        );
        self.store
            .create_character(
                &Handle::parse(raw_name).expect("正当な名前であること"),
                &DisplayName::parse(raw_display).expect("正当な表示名であること"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");
    }

    async fn send(&self, body: &Value) -> reqwest::Response {
        self.client
            .post(format!("http://{}/v1/chat/completions", self.addr))
            .json(body)
            .send()
            .await
            .expect("サーバへ接続できること")
    }

    /// 応答を最後まで読み切る（ステータス・`Content-Type`・本文）。
    async fn sse(&self, body: &Value) -> (StatusCode, String, String) {
        let response = self.send(body).await;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let text = response.text().await.expect("本文を読めること");
        (status, content_type, text)
    }

    /// **そのキャラクターの「最新セッション」の生の id**（`updated_at` 基準）。
    ///
    /// 中継の保存先が「解決したセッション」ではなく**最新セッションへすり替わっていない**
    /// ことを見るために、テスト側からも同じ問い合わせを行う（`tests/server_chat.rs` と同じ形）。
    async fn latest_session_id(&self, raw_name: &str) -> i64 {
        let character = self
            .store
            .find_character_by_handle(&Handle::parse(raw_name).expect("正当な名前であること"))
            .await
            .expect("キャラクターを引けること")
            .expect("キャラクターが存在すること");
        self.store
            .latest_session(character.id)
            .await
            .expect("セッションを引けること")
            .expect("セッションが存在すること")
            .id
            .get()
    }

    /// **保存された会話を古い順で読む**（11b-2 の検証）。
    ///
    /// セッションは SSE のチャンクに載った `model`（`aoi@<id>`）から取る。
    async fn saved_messages(&self, raw_name: &str, raw_session_id: i64) -> Vec<Message> {
        let character = self
            .store
            .find_character_by_handle(&Handle::parse(raw_name).expect("正当な名前であること"))
            .await
            .expect("キャラクターを引けること")
            .expect("キャラクターが存在すること");
        let session = self
            .store
            .find_session(character.id, raw_session_id)
            .await
            .expect("セッションを引けること")
            .expect("セッションが存在すること");
        self.store
            .recent_messages(session.id, 50)
            .await
            .expect("会話履歴を読めること")
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

/// ストリーミング要求の本文。
fn streaming_request(model: &str, user_message: &str) -> Value {
    json!({
        "model": model,
        "messages": [{"role": "user", "content": user_message}],
        "stream": true,
    })
}

/// SSE 本文から `data:` の中身だけを順序どおり取り出す。
fn data_lines(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| data.trim().to_owned())
        .collect()
}

/// `data:` の中身を JSON として読む。
fn json_of(data: &str) -> Value {
    serde_json::from_str(data).unwrap_or_else(|err| panic!("JSON として読めること: {err} / {data}"))
}

/// チャンクの増分。
fn delta_of(data: &str) -> String {
    json_of(data)["choices"][0]["delta"]["content"]
        .as_str()
        .unwrap_or_else(|| panic!("delta.content があること: {data}"))
        .to_owned()
}

/// `aoi@<session-id>` からセッション id を取り出す。
fn session_id_of(model: &str, expected_character: &str) -> i64 {
    let (character, session) = model
        .split_once('@')
        .unwrap_or_else(|| panic!("`名前@セッションid` の形であること: {model}"));
    assert_eq!(character, expected_character, "model = {model}");
    session
        .parse::<i64>()
        .unwrap_or_else(|err| panic!("セッション id が整数であること: {err} / {model}"))
}

/// 中継を**最後まで読み切って**、着地したセッション id を返す。
///
/// **読み切ることが必須。** 保存は `[DONE]` を流す直前に走るので、
/// 応答を途中で捨てるとその回は保存されない（`server::relay`）。
async fn relayed_session_id(server: &TestServer, body: &Value) -> i64 {
    let (status, _, text) = server.sse(body).await;
    assert_eq!(status, StatusCode::OK, "{text}");

    let events = data_lines(&text);
    assert_eq!(
        events.last().map(String::as_str),
        Some("[DONE]"),
        "前提: 中継が完結すること（保存まで進むこと）: {text}"
    );
    let model = json_of(&events[0])["model"]
        .as_str()
        .expect("model は文字列であること")
        .to_owned();
    session_id_of(&model, "aoi")
}

// ----------------------------------------------------------------------
// 正常系
// ----------------------------------------------------------------------

/// **チャンクは順序どおり流れ、`data: [DONE]` で終端する。**
///
/// `[DONE]` を送らないと、OpenAI 互換クライアントは応答の完了を判定できず
/// 「まだ続きがある」と待ち続ける。
#[tokio::test]
async fn chunks_are_relayed_in_order_and_terminated_by_done() {
    let server = TestServer::start(MockLlm::streaming(vec![
        chunk("「こん"),
        chunk("にちは」"),
        last_chunk(" 彼女は微笑んだ。"),
    ]))
    .await;
    server.seed("aoi", "葵").await;

    let (status, content_type, body) = server.sse(&streaming_request("aoi", "こんにちは")).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        content_type.starts_with("text/event-stream"),
        "SSE として返すこと: {content_type}"
    );

    let events = data_lines(&body);
    assert_eq!(events.len(), 4, "3チャンク ＋ 終端: {events:?}");
    assert_eq!(
        events
            .iter()
            .take(3)
            .map(|data| delta_of(data))
            .collect::<Vec<_>>(),
        vec!["「こん", "にちは」", " 彼女は微笑んだ。"],
        "順序を入れ替えないこと"
    );
    assert_eq!(events[3], "[DONE]", "終端は [DONE] であること: {events:?}");

    // 停止理由は最後のチャンクにだけ載る。
    assert_eq!(
        json_of(&events[0])["choices"][0]["finish_reason"],
        json!(null)
    );
    assert_eq!(
        json_of(&events[2])["choices"][0]["finish_reason"],
        json!("stop")
    );

    assert_eq!(server.llm.stream_count(), 1, "中継経路を通ること");
    assert_eq!(
        server.llm.complete_count(),
        0,
        "黙って非ストリーミングで補完しないこと"
    );

    server.stop().await;
}

/// **チャンクの `model` は非ストリーミングと同じ `aoi@<session-id>`。**
///
/// ここが上流のモデル名やクライアントが送った生の `model` に化けると、
/// クライアントは着地したセッションを固定できない（ADR-0004 の復旧経路）。
#[tokio::test]
async fn every_chunk_reports_the_session_it_landed_on() {
    let server = TestServer::start(MockLlm::streaming(vec![chunk("は"), last_chunk("い")])).await;
    server.seed("aoi", "葵").await;

    // **`aoi@new`（生の値）で要求する。** 素通しされたらここで露見する。
    let (status, _, body) = server
        .sse(&streaming_request("aoi@new", "こんにちは"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let events = data_lines(&body);
    let first = json_of(&events[0]);
    let model = first["model"].as_str().expect("model は文字列であること");
    let session = session_id_of(model, "aoi");
    assert!(session > 0, "セッションが作られていること: {model}");

    for data in events.iter().take(events.len() - 1) {
        let value = json_of(data);
        assert_eq!(value["object"], json!("chat.completion.chunk"), "{data}");
        assert_eq!(value["model"], json!(model), "全チャンクで同じであること");
        assert_eq!(value["id"], first["id"], "id は1本の応答で共通であること");
        assert_eq!(
            value["created"], first["created"],
            "created も共通であること"
        );
        assert_eq!(value["choices"][0]["index"], json!(0));
    }
    assert!(
        first["id"]
            .as_str()
            .expect("id は文字列であること")
            .starts_with("chatcmpl-"),
        "{first}"
    );
    // 先頭チャンクは役割を名乗る（OpenAI 互換クライアントが期待する形）。
    assert_eq!(first["choices"][0]["delta"]["role"], json!("assistant"));
    assert!(
        json_of(&events[1])["choices"][0]["delta"]
            .get("role")
            .is_none(),
        "役割は先頭チャンクにだけ載せること: {}",
        events[1]
    );

    server.stop().await;
}

/// **溜め込まずに流す**（docs/02-architecture.md）。
///
/// 上流がまだ喋り終えていない時点で、先に届いたチャンクが下流へ出ていること。
/// **全部溜めてから一度に流す実装だと、ここで時間切れになる。**
#[tokio::test]
async fn chunks_are_flushed_as_they_arrive_not_after_the_upstream_finishes() {
    let (tx, rx) = mpsc::channel(8);
    let server = TestServer::start(MockLlm::streaming_from(rx)).await;
    server.seed("aoi", "葵").await;

    tx.send(chunk("さき")).await.expect("送れること");

    let mut response = server.send(&streaming_request("aoi", "こんにちは")).await;
    assert_eq!(response.status(), StatusCode::OK);

    // **上流はまだ終わっていない**（`tx` を握ったまま）。それでも1件目が届くこと。
    let first = tokio::time::timeout(Duration::from_secs(5), response.chunk())
        .await
        .expect("上流の完了を待たずに1件目が届くこと（溜め込まないこと）")
        .expect("本文を読めること")
        .expect("本文があること");
    let first = String::from_utf8(first.to_vec()).expect("UTF-8 であること");
    assert_eq!(
        data_lines(&first)
            .iter()
            .map(|data| delta_of(data))
            .collect::<Vec<_>>(),
        vec!["さき"],
        "先に届いたチャンクだけが出ていること: {first:?}"
    );

    // 2件目も、送った時点で届く。
    tx.send(last_chunk("あと")).await.expect("送れること");
    let second = tokio::time::timeout(Duration::from_secs(5), response.chunk())
        .await
        .expect("2件目も届くこと")
        .expect("本文を読めること")
        .expect("本文があること");
    let second = String::from_utf8(second.to_vec()).expect("UTF-8 であること");
    assert_eq!(
        data_lines(&second)
            .iter()
            .map(|data| delta_of(data))
            .collect::<Vec<_>>(),
        vec!["あと"],
        "{second:?}"
    );

    // 上流が尽きたら終端する。
    drop(tx);
    let rest = tokio::time::timeout(Duration::from_secs(5), response.text())
        .await
        .expect("終端が届くこと")
        .expect("本文を読めること");
    assert_eq!(data_lines(&rest), vec!["[DONE]"], "{rest:?}");

    server.stop().await;
}

// ----------------------------------------------------------------------
// 中継の途中で上流が壊れたとき
// ----------------------------------------------------------------------

/// **中継の途中で上流が壊れたら、error オブジェクトを流して `[DONE]` を送らずに終端する。**
///
/// 200 とヘッダは既に送ってしまっているのでステータスでは伝えられない。決めた形:
///
/// 1. **そこまでのチャンクはそのまま有効**（`llm` の `ChatStream` の doc）。
/// 2. 最後に**非ストリーミングと同じ error オブジェクト**を1件流す（定数のみ）。
/// 3. **`[DONE]` は送らない。** 送ると「正常に完結した」と読めてしまう。
///    こちら自身が上流に対して「`[DONE]` 無しの終端＝ `Truncated`」と解釈している
///    （`llm::lmstudio::chunk_stream`）ので、下流にも同じ規約で伝える。
#[tokio::test]
async fn a_mid_stream_failure_sends_an_error_event_and_never_says_done() {
    let server = TestServer::start(MockLlm::streaming(vec![
        chunk("ここ"),
        chunk("まで"),
        Err(LlmError::Truncated),
    ]))
    .await;
    server.seed("aoi", "葵").await;

    let (status, content_type, body) = server.sse(&streaming_request("aoi", "こんにちは")).await;

    assert_eq!(status, StatusCode::OK, "200 は既に送っている: {body}");
    assert!(content_type.starts_with("text/event-stream"));

    let events = data_lines(&body);
    assert_eq!(events.len(), 3, "2チャンク ＋ エラー1件: {events:?}");
    assert_eq!(delta_of(&events[0]), "ここ");
    assert_eq!(delta_of(&events[1]), "まで");
    assert_eq!(
        json_of(&events[2]),
        json!({
            "error": {
                "message": "推論サーバとの通信に失敗しました",
                "type": "server_error",
                "code": "upstream_failed",
            }
        }),
        "非ストリーミングと同じ定数を流すこと（内部情報を載せない）"
    );
    assert!(
        !events.iter().any(|data| data == "[DONE]"),
        "壊れた応答を「完結した」と伝えないこと: {events:?}"
    );

    server.stop().await;
}

/// **上流が接続できないときも、途中で壊れたときと同じ形で伝える。**
///
/// 案内の文言は非ストリーミング（503 `upstream_unavailable`）と同じものを使う。
/// LM Studio は普段停止している運用なので、ここを潰すと起動を促せない。
#[tokio::test]
async fn a_mid_stream_disconnect_keeps_the_reason_it_had_in_the_non_streaming_path() {
    let server = TestServer::start(MockLlm::streaming(vec![
        chunk("と"),
        Err(LlmError::MalformedEvent),
    ]))
    .await;
    server.seed("aoi", "葵").await;

    let (status, _, body) = server.sse(&streaming_request("aoi", "こんにちは")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let events = data_lines(&body);
    assert_eq!(
        json_of(&events[1])["error"]["code"],
        json!("upstream_failed")
    );
    // 内部情報（接続先・パス）を反射しない。
    for leak in ["127.0.0.1", "os error", "src/", "src\\"] {
        assert!(!body.contains(leak), "{leak:?} が漏れていないこと: {body}");
    }

    server.stop().await;
}

/// **上流を開く前の失敗は、まだ 200 を送っていないので普通のエラー応答で返す。**
///
/// ここを SSE の 200 に化けさせると、「LM Studio が起動していない」が
/// クライアントからは「空の応答」に見える。
#[tokio::test]
async fn a_failure_before_the_first_chunk_is_a_normal_error_response() {
    let server =
        TestServer::start(MockLlm::failing_to_open(LlmError::Status { status: 429 })).await;
    server.seed("aoi", "葵").await;

    let (status, content_type, body) = server.sse(&streaming_request("aoi", "こんにちは")).await;

    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(
        content_type.starts_with("application/json"),
        "SSE ではなく普通のエラー応答であること: {content_type}"
    );
    assert_eq!(
        json_of(&body),
        json!({
            "error": {
                "message": "推論サーバとの通信に失敗しました",
                "type": "server_error",
                "code": "upstream_failed",
            }
        })
    );
    assert!(!body.contains("429"), "上流のステータスを反射しないこと");

    server.stop().await;
}

/// **中継でも、上流が鍵を断った（401 / 403）ことを `upstream_unauthorized` で言う**
/// （ADR-0056 §決定4 / 作業単位 A-13b-1）。非ストリーミングと同じ定数・同じ 502。
///
/// **対照は上の `a_failure_before_the_first_chunk_is_a_normal_error_response`**（429 は `upstream_failed`）。
#[tokio::test]
async fn a_refused_key_before_the_first_chunk_is_reported_as_unauthorized() {
    for upstream_status in [401, 403] {
        let server = TestServer::start(MockLlm::failing_to_open(LlmError::Status {
            status: upstream_status,
        }))
        .await;
        server.seed("aoi", "葵").await;

        let (status, content_type, body) =
            server.sse(&streaming_request("aoi", "こんにちは")).await;

        assert_eq!(status, StatusCode::BAD_GATEWAY, "{upstream_status}: {body}");
        assert!(
            content_type.starts_with("application/json"),
            "SSE ではなく普通のエラー応答であること: {content_type}"
        );
        assert_eq!(
            json_of(&body),
            json!({
                "error": {
                    "message": "推論サーバが鍵を受け付けませんでした",
                    "type": "server_error",
                    "code": "upstream_unauthorized",
                }
            }),
            "上流 {upstream_status}"
        );
        server.stop().await;
    }
}

/// **中継でも、承認が無くて送らなかったことを `upstream_key_not_approved` で言う**
/// （ADR-0056 §決定5 / 作業単位 A-13b-2）。非ストリーミングと同じ定数・同じ 502。
#[tokio::test]
async fn a_send_without_the_plain_http_approval_before_the_first_chunk_is_reported_as_such() {
    let server = TestServer::start(MockLlm::failing_to_open(LlmError::KeyNotApproved)).await;
    server.seed("aoi", "葵").await;

    let (status, content_type, body) = server.sse(&streaming_request("aoi", "こんにちは")).await;

    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert!(
        content_type.starts_with("application/json"),
        "SSE ではなく普通のエラー応答であること: {content_type}"
    );
    assert_eq!(
        json_of(&body),
        json!({
            "error": {
                "message": "鍵を暗号化せずにローカルネットワークの外へ送る承認が無いので、推論サーバへ送りませんでした",
                "type": "server_error",
                "code": "upstream_key_not_approved",
            }
        })
    );
    server.stop().await;
}

// ----------------------------------------------------------------------
// 入口の検査は中継でも同じ（上流を呼ばない）
// ----------------------------------------------------------------------

/// ストリーミングでも**入口の検査は先に走る**（上流を開かない）。
#[tokio::test]
async fn the_door_checks_still_run_before_the_stream_is_opened() {
    let server = TestServer::start(MockLlm::streaming(vec![last_chunk("はい")])).await;
    server.seed("aoi", "葵").await;

    // 未知のキャラクター。
    let (status, _, body) = server.sse(&streaming_request("dareka", "やあ")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(json_of(&body)["error"]["code"], json!("model_not_found"));

    // `max_tokens: 0`。
    let mut zero = streaming_request("aoi", "やあ");
    zero["max_tokens"] = json!(0);
    let (status, _, body) = server.sse(&zero).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(json_of(&body)["error"]["code"], json!("invalid_max_tokens"));

    // 予算に収まらない発言。
    let allowance =
        usize::try_from(TokenBudget::default().prompt_allowance()).expect("usize に収まること");
    let (status, _, body) = server
        .sse(&streaming_request("aoi", &"あ".repeat(allowance)))
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        json_of(&body)["error"]["code"],
        json!("prompt_budget_exhausted")
    );

    assert_eq!(server.llm.stream_count(), 0, "上流を開かないこと");

    server.stop().await;
}

/// 上流へ渡す内容は非ストリーミングと同じ（`max_tokens` は余白で切り下げる）。
#[tokio::test]
async fn the_upstream_request_is_built_the_same_way_as_the_non_streaming_path() {
    let server = TestServer::start(MockLlm::streaming(vec![last_chunk("はい")])).await;
    server.seed("aoi", "葵").await;

    let mut body = streaming_request("aoi", "こんにちは");
    body["max_tokens"] = json!(1_000_000);
    body["temperature"] = json!(0);
    let (status, _, text) = server.sse(&body).await;
    assert_eq!(status, StatusCode::OK, "{text}");

    let request = server.llm.last_stream_request();
    assert_eq!(
        request.max_tokens,
        Some(TokenBudget::default().response_tokens()),
        "クライアントの値を素通ししないこと"
    );
    assert_eq!(request.temperature, Some(0.0));
    // **`model` は上流へ渡らない**（`ChatRequest` に項目が無い / ADR-0004）。
    for message in &request.messages {
        assert!(
            !message.content.contains("aoi"),
            "生の model 値が混入しないこと: {}",
            message.content
        );
    }

    server.stop().await;
}

// ----------------------------------------------------------------------
// 回帰: `stream: false` / 省略時
// ----------------------------------------------------------------------

/// **`stream: false` と省略時は従来どおり JSON を返す**（中継経路を使わない）。
#[tokio::test]
async fn a_non_streaming_request_is_unchanged() {
    let server = TestServer::start(MockLlm::streaming(vec![last_chunk("流れないこと")])).await;
    server.seed("aoi", "葵").await;

    for body in [
        json!({"model": "aoi", "messages": [{"role": "user", "content": "やあ"}]}),
        json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "やあ"}],
            "stream": false,
        }),
    ] {
        let (status, content_type, text) = server.sse(&body).await;

        assert_eq!(status, StatusCode::OK, "{text}");
        assert!(
            content_type.starts_with("application/json"),
            "SSE にしないこと: {content_type}"
        );
        let value = json_of(&text);
        assert_eq!(value["object"], json!("chat.completion"));
        assert_eq!(
            value["choices"][0]["message"]["content"],
            json!("非ストリーミングの本文")
        );
        assert!(!text.contains("data:"), "SSE の枠を混ぜないこと: {text}");
    }

    assert_eq!(server.llm.complete_count(), 2);
    assert_eq!(server.llm.stream_count(), 0, "中継経路は使わないこと");

    server.stop().await;
}

// ----------------------------------------------------------------------
// 永続化（11b-2）
// ----------------------------------------------------------------------

/// **完結した中継は、蓄積した本文を分解して往復として保存する。**
///
/// 保存するのは**中継が完結した後**なので、ここで見えるのは
/// 「流したチャンクの連結」と一致する1件の応答になる。
#[tokio::test]
async fn a_completed_relay_persists_the_parsed_turn() {
    /// 利用者の発言。**応答の本文とは別の文字列**（入れ替えを検出するため）。
    const USER_MESSAGE: &str = "ただいま、葵";
    /// 流した増分の連結。**台詞と地の文が両方入る。**
    const WHOLE_REPLY: &str = "「おかえりなさい」 彼女は微笑んだ。";

    let server = TestServer::start(MockLlm::streaming(vec![
        chunk("「おかえり"),
        chunk("なさい」"),
        last_chunk(" 彼女は微笑んだ。"),
    ]))
    .await;
    server.seed("aoi", "葵").await;

    let (status, _, body) = server.sse(&streaming_request("aoi", USER_MESSAGE)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let events = data_lines(&body);
    assert_eq!(events.last().map(String::as_str), Some("[DONE]"), "{body}");
    let model = json_of(&events[0])["model"]
        .as_str()
        .expect("model は文字列であること")
        .to_owned();
    let session = session_id_of(&model, "aoi");

    let saved = server.saved_messages("aoi", session).await;
    assert_eq!(saved.len(), 2, "往復の2件が保存されること: {saved:?}");

    // 利用者の発言は**パースしない。**
    assert_eq!(saved[0].role, Role::User);
    assert_eq!(saved[0].raw_content, USER_MESSAGE);
    assert_eq!(saved[0].dialogue, None);
    assert_eq!(saved[0].narration, None);

    // 応答は分解され、`raw_content` には**流した増分の連結**が丸ごと残る。
    assert_eq!(saved[1].role, Role::Assistant);
    assert_eq!(saved[1].dialogue.as_deref(), Some("おかえりなさい"));
    assert_eq!(saved[1].narration.as_deref(), Some(" 彼女は微笑んだ。"));
    assert_eq!(saved[1].raw_content, WHOLE_REPLY);

    server.stop().await;
}

/// **中継した往復も「解決したセッション」に書かれる。最新セッションではない。**
///
/// # なぜ既存のテストでは足りないのか
///
/// 中継側の `TurnSink` へ渡す id を `latest_session(character.id)` にすり替える変異は、
/// チャンクに載る `model` を変えないので**全緑で通り抜ける**
/// （レビュアーの実測: `server_chat_stream` 14/14 緑、`server_chat_parse_log` も緑）。
///
/// そのとき起きるのは、FE が古いセッション `aoi@1` を明示して再開したときに
/// **チャンクの `model` は `aoi@1` を返すのに、往復は別のセッション `aoi@2` に書かれる**
/// という壊れ方。`[DONE]` まで正常に届くので、クライアント側からは一切見えない。
///
/// 形は非ストリーミング側の
/// `a_turn_is_persisted_into_the_resolved_session_not_the_latest_one` と同じ
/// （2つ作って、古い方を明示して3回目を叩き、両方の件数と中身を見る）。
#[tokio::test]
async fn a_relayed_turn_is_persisted_into_the_resolved_session_not_the_latest_one() {
    /// 古い方のセッションの1回目。
    const OLD_FIRST_USER: &str = "古い方のセッションで話しかける";
    const OLD_FIRST_REPLY: &str = "「ここは1つ目の会話」 彼女は答えた。";
    /// 新しい方のセッション（**古い方とは別の文字列**）。
    const NEW_USER: &str = "新しい方のセッションを始める";
    const NEW_REPLY: &str = "「ここは2つ目の会話」 彼女は振り向いた。";
    /// 古い方へ戻ってきた3回目（**上のどれとも別の文字列**）。
    const OLD_SECOND_USER: &str = "古い方のセッションへ戻ってきた";
    const OLD_SECOND_REPLY: &str = "「おかえり、1つ目の続き」 彼女は微笑んだ。";

    // **回ごとに違う本文を流す。** 同じ本文だと、どの回がどちらへ入ったか分からない。
    let server = TestServer::start(MockLlm::streaming_each(vec![
        vec![
            chunk("「ここは1つ目の会話」"),
            last_chunk(" 彼女は答えた。"),
        ],
        vec![
            chunk("「ここは2つ目の会話」"),
            last_chunk(" 彼女は振り向いた。"),
        ],
        vec![
            chunk("「おかえり、1つ目の続き」"),
            last_chunk(" 彼女は微笑んだ。"),
        ],
    ]))
    .await;
    server.seed("aoi", "葵").await;

    // 1. 古い方。
    let old_id = relayed_session_id(&server, &streaming_request("aoi@new", OLD_FIRST_USER)).await;

    // 2. 新しい方。**こちらが「最新」になる。**
    let new_id = relayed_session_id(&server, &streaming_request("aoi@new", NEW_USER)).await;
    assert_ne!(old_id, new_id, "前提: 別のセッションであること");
    assert_eq!(
        server.latest_session_id("aoi").await,
        new_id,
        "前提: このあと明示する古い方と「最新」は別物であること（同じなら何も検出できない）"
    );

    // 3. **古い方**を明示して3回目。
    let third = relayed_session_id(
        &server,
        &streaming_request(&format!("aoi@{old_id}"), OLD_SECOND_USER),
    )
    .await;
    assert_eq!(
        third, old_id,
        "チャンクの model は解決したセッションを指すこと"
    );

    // 古い方に**2往復ぶん**積み上がっていること。
    let old = server.saved_messages("aoi", old_id).await;
    assert_eq!(
        old.len(),
        4,
        "解決したセッションへ書くこと（最新へ逃がさない）: {old:?}"
    );
    assert_eq!(old[0].raw_content, OLD_FIRST_USER);
    assert_eq!(old[1].raw_content, OLD_FIRST_REPLY);
    assert_eq!(
        old[2].raw_content, OLD_SECOND_USER,
        "3回目の発言は古い方に入ること: {old:?}"
    );
    assert_eq!(
        old[3].raw_content, OLD_SECOND_REPLY,
        "3回目の応答は古い方に入ること: {old:?}"
    );

    // 新しい方は**1往復のまま**（3回目が混入していないこと）。
    let new = server.saved_messages("aoi", new_id).await;
    assert_eq!(
        new.len(),
        2,
        "解決しなかったセッションへ書かないこと: {new:?}"
    );
    assert_eq!(new[0].raw_content, NEW_USER);
    assert_eq!(new[1].raw_content, NEW_REPLY);

    server.stop().await;
}

/// **中継が途中で壊れた回は1行も保存しない**（利用者の発言も）。
///
/// 部分的な応答を履歴に混ぜると、以後の文脈が汚れる。しかも
/// **`[DONE]` を送っていないので、クライアントはその回を失敗として扱う**——
/// こちらだけが「途中まで喋った」ことを覚えていると食い違う。
#[tokio::test]
async fn a_mid_stream_failure_persists_nothing() {
    let server = TestServer::start(MockLlm::streaming(vec![
        chunk("ここまで"),
        Err(LlmError::Truncated),
    ]))
    .await;
    server.seed("aoi", "葵").await;

    let (status, _, body) = server
        .sse(&streaming_request("aoi", "この発言は残らないこと"))
        .await;
    assert_eq!(status, StatusCode::OK, "200 は既に送っている: {body}");

    let events = data_lines(&body);
    assert!(
        !events.iter().any(|data| data == "[DONE]"),
        "前提: 壊れた中継であること: {events:?}"
    );
    let model = json_of(&events[0])["model"]
        .as_str()
        .expect("model は文字列であること")
        .to_owned();
    let session = session_id_of(&model, "aoi");

    let saved = server.saved_messages("aoi", session).await;
    assert!(
        saved.is_empty(),
        "途中で壊れた回は利用者の発言も保存しないこと: {saved:?}"
    );

    server.stop().await;
}

/// **保存に失敗しても中継は壊さない。** `[DONE]` は最後まで届く。
///
/// 保存が走るのは `[DONE]` を流す直前なので、**そこで止めても応答の成否は変えられない**
/// （クライアントは既に本文を受け取っている）。エラーイベントに化けさせると、
/// 正常に生成できた応答が失敗として扱われる。
#[tokio::test]
async fn a_failure_to_persist_never_breaks_the_relay() {
    let server = TestServer::start(MockLlm::streaming(vec![
        chunk("「保存できなくても"),
        last_chunk("流し切ること」"),
    ]))
    .await;
    server.seed("aoi", "葵").await;
    server
        .llm
        .break_the_store_on_the_next_call(server.store.clone());

    let (status, content_type, body) = server.sse(&streaming_request("aoi", "こんにちは")).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(content_type.starts_with("text/event-stream"));

    let events = data_lines(&body);
    assert_eq!(
        events
            .iter()
            .take(events.len().saturating_sub(1))
            .map(|data| delta_of(data))
            .collect::<Vec<_>>(),
        vec!["「保存できなくても", "流し切ること」"],
        "本文を落とさないこと: {events:?}"
    );
    assert_eq!(
        events.last().map(String::as_str),
        Some("[DONE]"),
        "保存の失敗をエラーイベントに化けさせないこと: {events:?}"
    );

    server.stop().await;
}

// ----------------------------------------------------------------------
// 答え合わせの配線（ADR-0013 第3層 / ストリーミング経路）
// ----------------------------------------------------------------------

/// 上界が破れたときのログの手掛かり。**全文一致にしないこと。**
const UPPER_BOUND_MARKER: &str = "上界が破れ";

/// 想定コンテキストを超えたときの手掛かり。
/// **こちらだけが `max_tokens` と `context_tokens` を並べて載せる。**
const OVERRUN_MARKER: &str = "想定コンテキスト長を超え";

/// 答え合わせのテストで設定するコンテキスト長。**既定（8192）と別の値。**
const CONFIGURED_CONTEXT_TOKENS: u32 = 16_384;

/// 答え合わせのテストでクライアントが指定する生成上限。
///
/// **既定の余白（`DEFAULT_RESPONSE_TOKENS` = 1024）と別の値にすること。**
/// 同値だと、中継へ渡っているのが切り下げ後の `max_tokens` なのか
/// 予算の余白なのかを区別できない（CLAUDE.md「最頻の穴」）。
const REQUESTED_MAX_TOKENS: u32 = 16;

/// **最終チャンクに `usage` が載っていれば、ストリーミングでも答え合わせが走る**
/// （ADR-0013 第3層）。
///
/// 上流が「プロンプトは 100,000 トークンだった」と申告すれば、こちらの見積もりを
/// 必ず超える。**中継の終わりで `usage::report` を呼ぶのをやめると、この警告が消えて赤になる。**
///
/// # なぜグローバルの subscriber を差すのか
///
/// 判定はサーバのタスク側で走るので、スレッドローカルな `with_default` では捕まらない。
/// **このテストバイナリで `set_global_default` を呼ぶのはここだけ。**
///
/// **⚠ 「ここだけ」の根拠はコメントではない**（作業単位 B-3）。
/// 差し込みは [`common::install_global_log`] の `OnceLock` に畳んであり、
/// **2度目の呼び出しは同じバッファを返すだけ**なので `set_global_default` は1回しか走らない。
/// 器（`Write` / `MakeWriter`）も `src/testlog/assert.rs` と同じ1本を読む
/// ——**このファイルに写しを作り直さないこと**（押さえの見張り番はあちらにしか無い）。
///
/// **逆向き（`usage` が無ければ黙ること）はここでは見られない**（他のテストの出力が
/// 混ざるため）。そちらは `server::relay` の単体テストが決定的に見張っている。
///
/// # 既定の予算・既定の余白で走らせないこと
///
/// **`TestServer::start`（既定予算 8192）だと `state.budget()` と
/// `TokenBudget::default()` が同値になり、判別できない。** 同じ理由で
/// `max_tokens` も余白（1024）と別の値を指定する。
/// **どちらも 11a-2b のレビューで実測された穴**——`relay_response` の
/// 第3引数を `TokenBudget::default()` に、第5引数を `0` に潰す変異が
/// 651/651 緑を通り抜けた。超過側の行で両方を同一行の完全一致に固定して塞ぐ
/// （非ストリーミング側の `the_usage_crosscheck_runs_on_the_real_request_path` と同じ形）。
#[tokio::test]
async fn the_usage_crosscheck_runs_on_the_streaming_path() {
    let buffer = install_global_log(personacore::server::DEFAULT_LOG_FILTER);

    assert_ne!(
        CONFIGURED_CONTEXT_TOKENS,
        TokenBudget::DEFAULT_CONTEXT_TOKENS,
        "前提: 既定の予算と別の値であること（同値だと設定が届いたか分からない）"
    );
    assert_ne!(
        REQUESTED_MAX_TOKENS,
        TokenBudget::DEFAULT_RESPONSE_TOKENS,
        "前提: 余白と別の値であること（同値だと切り下げ後の値か余白か分からない）"
    );

    // **最終チャンクにだけ `usage` が載る**（OpenAI 互換の実装がそうする）。
    let configured = CONFIGURED_CONTEXT_TOKENS.to_string();
    let server = TestServer::start_with_env(
        MockLlm::streaming(vec![
            chunk("「は"),
            Ok(ChatChunk {
                delta: "い」".to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: Some(TokenUsage {
                    prompt_tokens: 100_000,
                }),
            }),
        ]),
        &[(ENV_CONTEXT_TOKENS, &configured)],
    )
    .await;
    server.seed("aoi", "葵").await;

    let mut request = streaming_request("aoi", "こんにちは");
    request["max_tokens"] = json!(REQUESTED_MAX_TOKENS);
    let (status, _, body) = server.sse(&request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        data_lines(&body).last().map(String::as_str),
        Some("[DONE]"),
        "答え合わせが中継を壊さないこと: {body}"
    );

    let logged = buffer.contents();

    // 1. 上界の破れ。**上流の申告値をそのまま載せる。**
    //    欄は `has_field` でトークンとして突き合わせる（`contains` だと
    //    `actual_tokens=1000000` のような**桁が伸びた別の値**にも当たる / B-3a）。
    let broken = line_containing(&logged, UPPER_BOUND_MARKER);
    assert!(
        has_field(broken, "actual_tokens=100000"),
        "上流の申告値をそのまま載せること: {broken:?}"
    );

    // 2. 想定コンテキストの超過。**比べた相手が起動時に決めた予算であること。**
    //    ここが `TokenBudget::default()` に潰れると 8192 が載る。
    let overrun = line_containing(&logged, OVERRUN_MARKER);
    assert!(
        has_field(
            overrun,
            &format!("context_tokens={CONFIGURED_CONTEXT_TOKENS}")
        ),
        "設定した予算を中継の答え合わせに使うこと（既定値 {} ではない）: {overrun:?}",
        TokenBudget::DEFAULT_CONTEXT_TOKENS
    );
    // **切り下げ後の `max_tokens` がそのまま中継へ届いていること。**
    // `0` や予算の余白に潰れると、超過の判定が実際と食い違う。
    assert!(
        has_field(overrun, &format!("max_tokens={REQUESTED_MAX_TOKENS}")),
        "上流へ渡した生成上限をそのまま載せること: {overrun:?}"
    );

    server.stop().await;
}

/// **使用量だけのチャンクは下流へ流さない。**
///
/// `stream_options.include_usage` を要求すると、上流は本文を出し切った後に
/// `{"choices":[],"usage":{…}}` を送る。`llm::parse_chunk` はこれを
/// 「空の増分を持つチャンク」に均すので、素通しすると
/// **`{"delta":{"content":""}}` が1件余分に流れる**（OpenAI 本家の使用量チャンクは
/// `choices: []` なので、クライアントから見ると別物のイベントが増える）。
///
/// **答え合わせに使うこと自体は `server::relay` の単体テストが見張っている**
/// （このバイナリでグローバルの subscriber を差せるのは1つのテストだけ）。
///
/// # 申告値は「警告が鳴らない」値にすること
///
/// グローバルの subscriber は1つのバッファを共有するので、ここで上界の破れや
/// 想定コンテキストの超過を鳴らすと、
/// **`the_usage_crosscheck_runs_on_the_streaming_path` が引く行がこちらの行に化ける**
/// （実測でそうなった）。`1` なら見積もりにも予算にも収まるので何も鳴らない。
#[tokio::test]
async fn a_usage_only_chunk_is_never_relayed_downstream() {
    let server = TestServer::start(MockLlm::streaming(vec![
        chunk("「は"),
        last_chunk("い」"),
        usage_only_chunk(1),
    ]))
    .await;
    server.seed("aoi", "葵").await;

    let (status, _, body) = server.sse(&streaming_request("aoi", "こんにちは")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let events = data_lines(&body);
    assert_eq!(
        events
            .iter()
            .take(events.len().saturating_sub(1))
            .map(|data| delta_of(data))
            .collect::<Vec<_>>(),
        vec!["「は", "い」"],
        "本文のチャンクだけが流れること: {events:?}"
    );
    assert_eq!(
        events.len(),
        3,
        "本文2件 ＋ 終端。使用量だけのチャンクを流さないこと: {events:?}"
    );
    assert_eq!(events[2], "[DONE]", "{events:?}");
    assert!(
        !body.contains(r#""content":"""#),
        "空の増分を余分に流さないこと: {body}"
    );

    server.stop().await;
}
