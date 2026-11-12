//! `POST /v1/chat/completions`（**非ストリーミング経路のみ**）の検証。
//!
//! **実際の LM Studio には繋がない**（docs/05-development.md）。上流は `LlmClient` の
//! モックを注入し、「上流へ何を渡したか」まで観測する。**SSE 中継は
//! `tests/server_chat_stream.rs`**（11a-2b / 11b-2 の中継側の永続化を含む）。
//! **パース成功率のログは `tests/server_chat_parse_log.rs`**（両経路をまとめて見る）。
//!
//! ここで固定するのは主に次の点:
//!
//!   * セッション同定（ADR-0004）— `aoi` / `aoi@new` / `aoi@<id>` の3経路と、
//!     **`aoi@new` の直後の `aoi` が新しい方を継続する**こと（`updated_at` の押し上げ）。
//!   * **往復の永続化と多ターン**（11b-2）— 台詞／地の文への分解、`raw_content` の保持、
//!     **同じセッション id を2回叩いたときに1回目の往復が次のプロンプトへ載る**こと。
//!   * **上流が失敗した回に残る空セッションが、次の素の `aoi` を横取りしない**こと（B-2b）。
//!     **明示した `aoi@<id>` は空セッションにも届く**こと（順位を下げただけで塞がない）。
//!   * **他キャラのセッション id では引けない**こと（`find_session` の所有確認）。
//!   * **`ModelSelector::parse` の失敗を 404 に潰さない**こと。
//!   * 採用するのは**最後の `user` メッセージだけ**であること。
//!   * `stream: false` を**黙って中継に振り替えない**こと。
//!   * `max_tokens` が**生成用の余白で上限を切られる**こと。
//!   * エラーレスポンスに**内部情報を載せない**こと（本文を完全一致で固定する）。

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use personacore::domain::{
    AttributeKey, AttributeSet, AttributeValue, CharacterAttribute, DisplayName, Handle,
    MemoryContent, Message, NewMemory, OutputFormat, Persona, Role, StatAlloc, StatAxis, Subject,
};
use personacore::llm::{
    ChatRequest, ChatRole, ChatStream, Completion, DEFAULT_ENDPOINT, DEFAULT_MAIN_MODEL,
    FinishReason, LlmClient, LlmError, LlmFuture, TokenUsage,
};
use personacore::prompt::TokenBudget;
use personacore::server::{
    App, AppConfig, BindAddr, ENV_CONTEXT_TOKENS, ENV_LLM_ENDPOINT, ServerError, SharedBudget,
};
use personacore::store::Store;
use reqwest::StatusCode;
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

use common::{
    fixture_scale, has_field, insert_character_attribute_ignoring_checks, install_global_log,
    line_containing,
};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

/// 上流（LM Studio）の代役。**ネットワークには一切触らない。**
struct MockLlm {
    /// `complete` が受け取った要求。**上流へ渡した中身を検査するために持つ。**
    calls: Mutex<Vec<ChatRequest>>,
    /// `stream` が呼ばれた回数。非ストリーミング経路では 0 のままであること。
    streams: Mutex<usize>,
    /// 返す応答。
    reply: Completion,
    /// **回ごとに差し替える本文。** 空になったら `reply` の本文へ戻る。
    ///
    /// 多ターンの検証で「1回目の往復」と「2回目の往復」を**別の文字列**にするために持つ。
    /// 同じ本文を返し続けると、2回目のプロンプトに載っているのが
    /// 1回目の応答なのか2回目の応答なのかを区別できない（CLAUDE.md「最頻の穴」）。
    queued: Mutex<VecDeque<String>>,
    /// `Some` なら `complete` は失敗する。
    /// `LlmError` は `Clone` でないので「作り方」を持つ。
    failure: Option<fn() -> LlmError>,
    /// **1回だけ返す失敗。** `Connect` / `Timeout` は `reqwest::Error` を包むが、
    /// `reqwest::Error` は外から構築できず、実物を得るには `await` が要る。
    /// つまり `fn() -> LlmError` では作れないので、**実体を持って取り出す**形にしてある。
    failure_once: Mutex<Option<LlmError>>,
    /// **応答を返す途中で永続化層を壊す**（保存だけを失敗させるための細工）。
    ///
    /// セッション解決・プロンプト組立・上流呼び出しはすべて済んだ後、
    /// **保存だけが失敗する**状況を作る唯一の手段。ここを持たないと
    /// 「保存の失敗で応答を壊さない」を見張るテストが1本も書けない。
    sabotage: Mutex<Option<Store>>,
}

impl MockLlm {
    /// 固定の本文を返すモック。**`usage` は返さない**
    /// （ADR-0013 第3層の「答え合わせできない上流」の再現でもある）。
    fn replying(content: &str) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            streams: Mutex::new(0),
            reply: Completion {
                content: content.to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            },
            queued: Mutex::new(VecDeque::new()),
            failure: None,
            failure_once: Mutex::new(None),
            sabotage: Mutex::new(None),
        })
    }

    /// **呼び出しごとに違う本文を返す**モック（多ターンの検証用）。
    ///
    /// 使い切ったら最後の本文を返し続ける。
    fn replying_each(contents: &[&str]) -> Arc<Self> {
        let mock = Self::replying(contents.last().copied().unwrap_or_default());
        *mock.queued.lock().expect("ロックを取れること") = contents
            .iter()
            .map(|content| (*content).to_owned())
            .collect();
        mock
    }

    /// **上流が数えた実トークン数を申告する**モック（ADR-0013 第3層）。
    fn replying_with_usage(content: &str, prompt_tokens: u32) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            streams: Mutex::new(0),
            reply: Completion {
                content: content.to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: Some(TokenUsage {
                    prompt_tokens,
                    completion_tokens: None,
                }),
            },
            queued: Mutex::new(VecDeque::new()),
            failure: None,
            failure_once: Mutex::new(None),
            sabotage: Mutex::new(None),
        })
    }

    /// 必ず失敗するモック。
    fn failing(make: fn() -> LlmError) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            streams: Mutex::new(0),
            reply: Completion {
                content: String::new(),
                finish_reason: None,
                usage: None,
            },
            queued: Mutex::new(VecDeque::new()),
            failure: Some(make),
            failure_once: Mutex::new(None),
            sabotage: Mutex::new(None),
        })
    }

    /// **任意の `LlmError` を1回だけ返す**モック（`Connect` / `Timeout` 用）。
    fn failing_once(error: LlmError) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            streams: Mutex::new(0),
            reply: Completion {
                content: String::new(),
                finish_reason: None,
                usage: None,
            },
            queued: Mutex::new(VecDeque::new()),
            failure: None,
            failure_once: Mutex::new(Some(error)),
            sabotage: Mutex::new(None),
        })
    }

    /// **次の `complete` を1回だけ失敗させる。**
    ///
    /// `failing_once` は構築時に仕込むので**1回目**が失敗する。
    /// 「成功したターンの後に失敗したターンが来る」順序はそれでは作れないが、
    /// B-2b の欠陥は**空セッションが会話より新しいときにしか起きない**ので、
    /// その順序を作れないと見張り番が成立しない。
    fn fail_the_next_call(&self, error: LlmError) {
        *self.failure_once.lock().expect("ロックを取れること") = Some(error);
    }

    /// **次の `complete` で永続化層を閉じる。**
    ///
    /// 保存だけを失敗させたいので、**上流の応答を返す直前**に閉じる
    /// （セッション解決も上流呼び出しも成功した後）。
    fn break_the_store_on_the_next_call(&self, store: Store) {
        *self.sabotage.lock().expect("ロックを取れること") = Some(store);
    }

    fn recorded(&self) -> Vec<ChatRequest> {
        self.calls.lock().expect("ロックを取れること").clone()
    }

    fn last_request(&self) -> ChatRequest {
        let mut recorded = self.recorded();
        recorded.pop().expect("上流へ1回以上要求していること")
    }

    fn call_count(&self) -> usize {
        self.recorded().len()
    }

    fn stream_count(&self) -> usize {
        *self.streams.lock().expect("ロックを取れること")
    }
}

impl LlmClient for MockLlm {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("ロックを取れること")
                .push(request.clone());
            if let Some(error) = self.failure_once.lock().expect("ロックを取れること").take()
            {
                return Err(error);
            }
            // **ロックを握ったまま `await` しない**（取り出してから閉じる）。
            let sabotage = self.sabotage.lock().expect("ロックを取れること").take();
            if let Some(store) = sabotage {
                store.close().await;
            }
            match self.failure {
                Some(make) => Err(make()),
                None => {
                    let mut reply = self.reply.clone();
                    // **回ごとに違う本文**（`replying_each`）。空なら既定の本文。
                    if let Some(content) =
                        self.queued.lock().expect("ロックを取れること").pop_front()
                    {
                        reply.content = content;
                    }
                    Ok(reply)
                }
            }
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        Box::pin(async move {
            *self.streams.lock().expect("ロックを取れること") += 1;
            // この作業単位では中継しない。呼ばれたこと自体が失敗。
            Err(LlmError::MissingContent)
        })
    }
}

/// エフェメラルポートで起動した実サーバ。**上流はモック。**
struct TestServer {
    addr: SocketAddr,
    store: Store,
    llm: Arc<MockLlm>,
    /// DB ファイルの絶対パス。エラーレスポンスに漏れていないことの確認に使う。
    db_path: String,
    client: reqwest::Client,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    async fn start(llm: Arc<MockLlm>) -> Self {
        Self::start_with_env(llm, &[]).await
    }

    /// 環境変数を差して起動する（**設定が `chat` まで届いているか**の検証に使う）。
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
            db_path: db_path.display().to_string(),
            client: reqwest::Client::new(),
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    /// 応答を返すモックで起動する（大半のテストはこちら）。
    async fn with_reply(content: &str) -> Self {
        Self::start(MockLlm::replying(content)).await
    }

    async fn chat(&self, body: &Value) -> (StatusCode, Value) {
        let (status, text) = self.chat_raw(body.to_string()).await;
        (status, parse_json(&text))
    }

    async fn chat_raw(&self, body: String) -> (StatusCode, String) {
        let response = self
            .client
            .post(format!("http://{}/v1/chat/completions", self.addr))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .expect("サーバへ接続できること");
        let status = response.status();
        let text = response.text().await.expect("本文を読めること");
        (status, text)
    }

    /// キャラクターを作る（管理 API と同じ経路を通さず、`store` から直接投入する）。
    ///
    /// **識別子と表示名は必ず別の値を渡すこと**（ADR-0011 / 0004a のレビュー指摘）。
    /// 同じ文字列を渡すと、プロンプトに載っているのが `handle` なのか `display_name` なのかを
    /// アサートで区別できず、**`store` → `prompt` の受け渡しを潰す変更が
    /// 結合レベルで一切捕まらない**（実測: `chat.rs` で `display_name := name` に
    /// 潰しても全件緑だった）。`prompt` 層は `SanitizedCharacter` が識別子を持たないので
    /// 型で守られているが、**`server` 層はここが唯一の関門。**
    async fn seed(&self, raw_name: &str, raw_display: &str) {
        assert_ne!(
            raw_name, raw_display,
            "識別子と表示名は別の値にすること（同値だとどちらが載ったか区別できない）"
        );
        self.store
            .create_character(
                &name(raw_name),
                &display(raw_display),
                &persona("落ち着いた口調で話す。"),
                Some(&output_format(
                    "台詞は「」で囲み、それ以外は地の文として書いてください。",
                )),
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");
    }

    /// 性格ステータスを設定する。
    async fn seed_stats(&self, raw_name: &str, stats: &[(&str, u32)]) {
        let character = self
            .store
            .find_character_by_handle(&name(raw_name))
            .await
            .expect("キャラクターを引けること")
            .expect("キャラクターが存在すること");
        // **値は `seed` が渡した尺度（`fixture_scale()`）を通す**（作業単位 0011a-2）。
        // 保存済みの尺度と食い違う値は `store` が拒否する。
        let allocs: Vec<StatAlloc> = stats
            .iter()
            .map(|(axis, value)| StatAlloc {
                axis: StatAxis::parse(axis).expect("正当な軸名であること"),
                value: fixture_scale()
                    .value(*value)
                    .expect("テストで使う値は尺度に収まること"),
            })
            .collect();
        self.store
            .replace_stat_allocs(character.id, &allocs)
            .await
            .expect("性格ステータスを設定できること");
    }

    /// **そのキャラクターの「最新セッション」の生の id**（`updated_at` 基準）。
    ///
    /// 保存先が「解決したセッション」ではなく**最新セッションへすり替わっていない**ことを
    /// 見るために、テスト側からも同じ問い合わせを行う。
    /// 「古い方を明示したのに最新へ書かれる」欠陥は、
    /// **この2つが別物である状況を作って初めて観測できる。**
    async fn latest_session_id(&self, raw_name: &str) -> i64 {
        let character = self
            .store
            .find_character_by_handle(&name(raw_name))
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

    /// **そのキャラクターのセッションを id の昇順で全部返す**（`(id, message の件数, updated_at)`）。
    ///
    /// B-2b の見張り番の**前提**——「空セッションが本当に、しかも会話より新しく残ったか」
    /// ——を確かめるために使う。ここだけ直接 DB を読むのは、
    /// **502 で終わった回のセッション id が誰にも返っていない**ため
    /// （応答は 200 の前に返るので `model` が無い）。それがこの欠陥の気付きにくさの理由でもある。
    async fn sessions_with_message_counts(&self, raw_name: &str) -> Vec<(i64, i64, String)> {
        let character = self
            .store
            .find_character_by_handle(&name(raw_name))
            .await
            .expect("キャラクターを引けること")
            .expect("キャラクターが存在すること");
        sqlx::query_as(
            "SELECT session.id, \
                    (SELECT COUNT(*) FROM message WHERE message.session_id = session.id), \
                    session.updated_at \
             FROM session WHERE session.character_id = ? ORDER BY session.id",
        )
        .bind(character.id.get())
        .fetch_all(self.store.pool())
        .await
        .expect("セッションを読めること")
    }

    /// **保存された会話を古い順で読む**（11b-2 の検証）。
    ///
    /// セッションは応答の `model`（`aoi@<id>`）から取った生の id で引く。
    /// `SessionId` は所有確認を通ってしか作れないので、ここも `find_session` を通す。
    async fn saved_messages(&self, raw_name: &str, raw_session_id: i64) -> Vec<Message> {
        let character = self
            .store
            .find_character_by_handle(&name(raw_name))
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

    /// **DB ファイルを開き直して**保存された会話を読む。
    ///
    /// 細工（`break_the_store_on_the_next_call`）で閉じたのは**プールだけ**で、
    /// `TempDir` も DB ファイルも生きている。新しい接続で開き直せば、
    /// 「その回が本当に1行も保存されなかったこと」を直接確かめられる。
    /// **`self.store` は閉じているので使えない**（それが唯一の理由）。
    async fn messages_from_a_fresh_connection(
        &self,
        raw_name: &str,
        raw_session_id: i64,
    ) -> Vec<Message> {
        let store = Store::connect(&self.db_path)
            .await
            .expect("DB ファイルを開き直せること");
        let character = store
            .find_character_by_handle(&name(raw_name))
            .await
            .expect("キャラクターを引けること")
            .expect("キャラクターが存在すること");
        let session = store
            .find_session(character.id, raw_session_id)
            .await
            .expect("セッションを引けること")
            .expect("セッションが存在すること");
        let messages = store
            .recent_messages(session.id, 50)
            .await
            .expect("会話履歴を読めること");
        store.close().await;
        messages
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

fn name(raw: &str) -> Handle {
    Handle::parse(raw).expect("テストで使う名前は正当であること")
}

/// 表示名。**識別子とは別の文字列を渡す**（`seed` の doc を参照）。
fn display(raw: &str) -> DisplayName {
    DisplayName::parse(raw).expect("テストで使う表示名は正当であること")
}

fn persona(raw: &str) -> Persona {
    Persona::parse(raw).expect("テストで使うペルソナは正当であること")
}

fn output_format(raw: &str) -> OutputFormat {
    OutputFormat::parse(raw).expect("テストで使う形式指示は正当であること")
}

fn parse_json(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|err| panic!("JSON として読めること: {err} / {body}"))
}

/// 最小のリクエスト本文。
fn request_of(model: &str, user_message: &str) -> Value {
    json!({
        "model": model,
        "messages": [{"role": "user", "content": user_message}],
    })
}

/// 応答の `model` フィールドからセッション id を取り出す。
///
/// **`aoi@<session-id>` を返す**ことで、クライアントは着地したセッションを固定できる。
fn session_id_of(body: &Value, expected_character: &str) -> i64 {
    let model = body["model"].as_str().expect("model は文字列であること");
    let (character, session) = model
        .split_once('@')
        .unwrap_or_else(|| panic!("`名前@セッションid` の形であること: {model}"));
    assert_eq!(character, expected_character, "model = {model}");
    session
        .parse::<i64>()
        .unwrap_or_else(|err| panic!("セッション id が整数であること: {err} / {model}"))
}

/// **接続を拒否された** `LlmError::Connect` を作る。
///
/// `reqwest::Error` は外から構築できないので、**実際に閉じたポートを叩いて得る。**
/// 叩き先は `127.0.0.1` の予約ポートなので、外部へは一切出ない。
async fn connect_error() -> LlmError {
    let error = reqwest::Client::new()
        .get("http://127.0.0.1:1/")
        .send()
        .await
        .expect_err("接続は拒否されること");
    assert!(error.is_connect(), "接続失敗であること: {error}");
    LlmError::Connect(error)
}

/// **応答が返ってこない** `LlmError::Timeout` を作る。
///
/// 受け付けるだけで何も返さないリスナーを立て、短い制限時間で叩く。
/// **`Connect` と取り違えないよう `is_timeout()` を確認してから包む。**
async fn timeout_error() -> LlmError {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("待ち受けを開始できること");
    let addr = listener.local_addr().expect("アドレスを取得できること");
    // 接続を受けたまま保持する（応答は返さない）。
    let holder = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });

    let error = reqwest::Client::builder()
        .timeout(Duration::from_millis(200))
        .build()
        .expect("クライアントを構築できること")
        .get(format!("http://{addr}/"))
        .send()
        .await
        .expect_err("応答は返ってこないこと");
    holder.abort();

    assert!(error.is_timeout(), "時間切れであること: {error}");
    LlmError::Timeout(error)
}

/// **生の TCP で1往復する。** `reqwest` を通さないのは、
/// 「本文を書き終える前に接続をリセットされる」という壊れ方を、
/// クライアント側の再試行やバッファに隠されずに観測するため。
///
/// 戻り値は `(ステータス行, 本文)`。**ヘッダと本文の境界まで含めて自前で解く。**
///
/// `Connection: close` を付けているのは、応答を読み切ったことを
/// `read_to_end` の終端で判定するため（keep-alive のままだと永久に待つ）。
async fn post_over_raw_tcp(addr: SocketAddr, path: &str, body: &str) -> (String, String) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let head = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );

    let exchange = async {
        let mut socket = tokio::net::TcpStream::connect(addr).await?;
        socket.write_all(head.as_bytes()).await?;
        // **少しずつ書く。** 「サーバが応答を書いた時点でも、クライアントはまだ書いている」
        // という現実の形を再現する（欠陥はこの重なりでしか出ない）。
        for chunk in body.as_bytes().chunks(64 * 1024) {
            socket.write_all(chunk).await?;
        }
        socket.flush().await?;
        let mut response = Vec::new();
        socket.read_to_end(&mut response).await?;
        Ok::<_, std::io::Error>(response)
    };

    let response = tokio::time::timeout(Duration::from_secs(30), exchange)
        .await
        .expect("応答が時間内に届くこと")
        .expect("接続をリセットされずに、本文を書き切って応答を読み切れること");
    let response = String::from_utf8(response).expect("応答が UTF-8 であること");

    let (headers, body) = response
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("ヘッダと本文の境界があること: {response:?}"));
    let status_line = headers
        .lines()
        .next()
        .unwrap_or_else(|| panic!("ステータス行があること: {response:?}"))
        .to_owned();
    (status_line, body.to_owned())
}

/// 応答本文の本文（`choices[0].message.content`）。
fn content_of(body: &Value) -> String {
    body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_else(|| panic!("content があること: {body}"))
        .to_owned()
}

// ----------------------------------------------------------------------
// 正常系
// ----------------------------------------------------------------------

/// 応答は OpenAI 互換の `chat.completion`。**`model` には着地したセッションが載る。**
#[tokio::test]
async fn a_bare_handle_answers_and_reports_the_session_it_landed_on() {
    let server = TestServer::with_reply("「こんにちは」 彼女は微笑んだ。").await;
    server.seed("aoi", "葵").await;

    let (status, body) = server.chat(&request_of("aoi", "こんにちは")).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["object"], json!("chat.completion"));
    assert_eq!(body["choices"][0]["index"], json!(0));
    assert_eq!(body["choices"][0]["message"]["role"], json!("assistant"));
    assert_eq!(content_of(&body), "「こんにちは」 彼女は微笑んだ。");
    assert_eq!(body["choices"][0]["finish_reason"], json!("stop"));
    assert!(
        body["id"]
            .as_str()
            .expect("id は文字列であること")
            .starts_with("chatcmpl-"),
        "{body}"
    );
    assert!(
        body["created"].as_i64().expect("created は整数であること") > 1_700_000_000,
        "{body}"
    );

    let session = session_id_of(&body, "aoi");
    assert!(session > 0, "セッションが作られていること: {body}");

    // 2回目の `aoi` は同じセッションを継続する。
    let (status, again) = server.chat(&request_of("aoi", "また来たよ")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        session_id_of(&again, "aoi"),
        session,
        "同じセッションを継続すること"
    );

    assert_eq!(server.llm.call_count(), 2);
    assert_eq!(server.llm.stream_count(), 0, "中継経路は使わないこと");

    server.stop().await;
}

/// **`aoi@new` の直後に `aoi` を投げたら、新しい方が継続される**（ADR-0004）。
///
/// `session.updated_at` の押し上げが効いていないと、新規セッションを作った直後に
/// `aoi` が旧セッションへ戻り、**記憶が静かに混線する。**
#[tokio::test]
async fn the_new_suffix_creates_a_session_that_becomes_the_latest() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    let (_, first) = server.chat(&request_of("aoi", "1回目")).await;
    let old = session_id_of(&first, "aoi");

    let (status, created) = server.chat(&request_of("aoi@new", "新しい会話")).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let fresh = session_id_of(&created, "aoi");
    assert_ne!(fresh, old, "@new は新規セッションを作ること: {created}");

    let (_, latest) = server.chat(&request_of("aoi", "続き")).await;
    assert_eq!(
        session_id_of(&latest, "aoi"),
        fresh,
        "@new の直後の `aoi` は新しい方を継続すること"
    );

    server.stop().await;
}

/// `aoi@<id>` は所有確認を通ったうえで継続できる（復旧・デバッグ用）。
#[tokio::test]
async fn an_explicit_session_id_continues_that_session() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    let (_, first) = server.chat(&request_of("aoi@new", "1つ目")).await;
    let first_id = session_id_of(&first, "aoi");
    let (_, second) = server.chat(&request_of("aoi@new", "2つ目")).await;
    let second_id = session_id_of(&second, "aoi");
    assert_ne!(first_id, second_id);

    // 明示指定で**古い方**へ戻れる。
    let (status, body) = server
        .chat(&request_of(&format!("aoi@{first_id}"), "戻ってきた"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(session_id_of(&body, "aoi"), first_id);

    server.stop().await;
}

/// **OpenAI 互換の入口では未知のフィールドを拒否しない**（`/admin/` とは方針が違う）。
///
/// 実クライアントは `top_p` / `presence_penalty` などを普通に送ってくる。
#[tokio::test]
async fn unknown_fields_from_a_real_client_are_accepted() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    let (status, body) = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "こんにちは", "name": "someone"}],
            "temperature": 0.7,
            "top_p": 0.9,
            "presence_penalty": 0.0,
            "frequency_penalty": 0.0,
            "n": 1,
            "stop": ["\n"],
            "user": "someone",
            "stream": false,
            "logit_bias": {},
        }))
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    server.stop().await;
}

// ----------------------------------------------------------------------
// 永続化と多ターン（11b-2 / マイルストーン4）
// ----------------------------------------------------------------------

/// 1回目の利用者の発言。**それ自体が `「」` を含む**（パースすれば台詞と地の文に割れる形）。
///
/// **利用者の発言はパースしない**ので、`dialogue` / `narration` は `None` になる。
/// ここを普通の文にすると、`parse_response` を通す変異が「割れる材料が無かった」だけで
/// 緑を通り抜ける。
const FIRST_USER: &str = "「ただいま」 と彼は書いた。";

/// 1回目の応答。**台詞と地の文が両方入る**（片方だけだと入れ替えが検出できない）。
const FIRST_REPLY: &str = "「おかえりなさい」 彼女は微笑んだ。";

/// 2回目の利用者の発言。**1回目と別の文字列。**
const SECOND_USER: &str = "今日はいい天気だね";

/// 2回目の応答。**1回目と別の文字列。**
const SECOND_REPLY: &str = "「そうね」 彼女は窓の外へ視線を移した。";

/// **往復が分解されて保存され、次のターンのプロンプトに載る**（マイルストーン4）。
///
/// # なぜ同じセッション id を明示して2回叩くのか
///
/// **`aoi@new` を2回叩くのでは検証にならない**（毎回新しいセッションになるので、
/// 履歴が1件も載らなくても緑になる）。1回目の応答の `model` に載った
/// `aoi@<id>` をそのまま2回目の `model` に使う——ADR-0004 が用意した復旧経路そのもの。
///
/// # 何を見ているか
///
/// 1. 利用者の発言が**パースされずに**保存されること（`dialogue` / `narration` が `None`）。
/// 2. 応答が**台詞と地の文に分解され**、かつ `raw_content` に**生出力が丸ごと**残ること。
/// 3. 2回目のプロンプトに**1回目の往復が載る**こと（`recent_messages` が空を返さなくなった）。
#[tokio::test]
async fn a_turn_is_parsed_persisted_and_carried_into_the_next_prompt() {
    assert_ne!(
        FIRST_USER, FIRST_REPLY,
        "前提: 発言と応答は別の文字列であること"
    );
    assert_ne!(
        FIRST_REPLY, SECOND_REPLY,
        "前提: 1回目と2回目は別の文字列であること"
    );

    let server = TestServer::start(MockLlm::replying_each(&[FIRST_REPLY, SECOND_REPLY])).await;
    server.seed("aoi", "葵").await;

    // --- 1回目 ---
    let (status, first) = server.chat(&request_of("aoi", FIRST_USER)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(content_of(&first), FIRST_REPLY, "応答はそのまま返すこと");
    let session = session_id_of(&first, "aoi");

    // **1回目のプロンプトには履歴が無い**（新規セッション）。
    assert_eq!(
        server.llm.last_request().messages.len(),
        2,
        "前提: 1回目は [システム, 今回の発言] の2件であること"
    );

    let saved = server.saved_messages("aoi", session).await;
    assert_eq!(saved.len(), 2, "往復の2件が保存されること: {saved:?}");

    // 1件目 = 利用者の発言。**モデルの出力ではないのでパースしない。**
    assert_eq!(saved[0].role, Role::User);
    assert_eq!(saved[0].raw_content, FIRST_USER, "本文を丸ごと残すこと");
    assert_eq!(
        saved[0].dialogue, None,
        "利用者の発言に `parse_response` を通さないこと"
    );
    assert_eq!(
        saved[0].narration, None,
        "利用者の発言に `parse_response` を通さないこと"
    );

    // 2件目 = 応答。**台詞と地の文に分解し、生出力も丸ごと残す。**
    assert_eq!(saved[1].role, Role::Assistant);
    assert_eq!(saved[1].dialogue.as_deref(), Some("おかえりなさい"));
    assert_eq!(saved[1].narration.as_deref(), Some(" 彼女は微笑んだ。"));
    assert_eq!(
        saved[1].raw_content, FIRST_REPLY,
        "`raw_content` は生出力そのもの（分解結果を入れないこと）"
    );

    // --- 2回目。**1回目が着地したセッション id を明示する。** ---
    let (status, second) = server
        .chat(&request_of(&format!("aoi@{session}"), SECOND_USER))
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(
        session_id_of(&second, "aoi"),
        session,
        "同じセッションに着地すること"
    );

    // **2回目のプロンプトに1回目の往復が載っていること**（履歴は `raw_content` を戻す）。
    let request = server.llm.last_request();
    assert_eq!(
        request.messages.len(),
        4,
        "[システム, 1回目の発言, 1回目の応答, 今回の発言] の4件: {:?}",
        request.messages
    );
    assert_eq!(request.messages[1].role, ChatRole::User);
    assert_eq!(request.messages[1].content, FIRST_USER);
    assert_eq!(request.messages[2].role, ChatRole::Assistant);
    assert_eq!(
        request.messages[2].content, FIRST_REPLY,
        "履歴は分解結果ではなく生出力を戻すこと"
    );
    assert_eq!(request.messages[3].role, ChatRole::User);
    assert_eq!(request.messages[3].content, SECOND_USER);

    // 2回目の往復も同じセッションに積み上がる。
    let saved = server.saved_messages("aoi", session).await;
    assert_eq!(saved.len(), 4, "往復が積み上がること: {saved:?}");
    assert_eq!(saved[2].raw_content, SECOND_USER);
    assert_eq!(saved[3].raw_content, SECOND_REPLY);
    assert_eq!(saved[3].dialogue.as_deref(), Some("そうね"));

    server.stop().await;
}

/// **想起した記憶の前置きは、送る最新の発言にだけ付く。保存にも、次のターンの履歴にも出ない**
/// （ADR-0061 §決定1 / 作業単位 A-15）。
///
/// # なぜ記憶を入れて回すのか
///
/// 記憶が0件なら前置きは出ないので、送った本文と保存した原文が同じになる。そのままでは
/// 「保存（`TurnSink` に渡す発言）に、送る本文の末尾を使う」変更が緑を通る。記憶を1件入れて、
/// **送った末尾の発言と、保存した原文が違う**状態を作ってから、保存と読み戻しが原文であることを見る。
///
/// # 見ているもの
///
/// 1. 1回目に送った末尾の発言は「前置き（記憶の引用行を含む）・改行・原文」で、system に記憶は無い。
/// 2. 保存した利用者の発言は原文（前置きを含まない。抽出もこの原文を読む）。
/// 3. 2回目の履歴に読み戻された1回目の発言は原文（過去のターンに記憶が付かない）。
///    前置きは2回目の末尾の発言にだけ付く。
#[tokio::test]
async fn the_recalled_memory_precedes_only_the_sent_message_and_never_the_stored_one() {
    /// 利用者についての記憶。**どの発言・応答とも部分文字列を共有しない。**
    const MEMORY: &str = "黒猫のコーヒーと暮らしている";

    let server = TestServer::start(MockLlm::replying_each(&[FIRST_REPLY, SECOND_REPLY])).await;
    server.seed("aoi", "葵").await;
    let character = server
        .store
        .find_character_by_handle(&name("aoi"))
        .await
        .expect("検索できること")
        .expect("作ったキャラクターが引けること");
    server
        .store
        .add_memory(
            character.id,
            &NewMemory {
                subject: Subject::User,
                content: MemoryContent::parse(MEMORY).expect("正当な本文であること"),
                category: None,
            },
        )
        .await
        .expect("記憶を追加できること");

    // --- 1回目 ---
    let (status, first) = server.chat(&request_of("aoi", FIRST_USER)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let session = session_id_of(&first, "aoi");

    let sent = server.llm.last_request();
    assert_eq!(
        sent.messages.len(),
        2,
        "前提: 1回目は [システム, 今回の発言] の2件であること"
    );
    assert!(
        !sent.messages[0].content.contains(MEMORY),
        "記憶が system に載った: {}",
        sent.messages[0].content
    );
    let latest = &sent.messages[1].content;
    assert_eq!(sent.messages[1].role, ChatRole::User);
    assert!(
        latest.contains(&format!("> - {MEMORY}")),
        "前提: 記憶が前置きの引用行として載ること: {latest}"
    );
    assert!(
        latest.ends_with(&format!("\n{FIRST_USER}")) && latest != FIRST_USER,
        "末尾の発言は前置き・改行・原文の順: {latest}"
    );

    let saved = server.saved_messages("aoi", session).await;
    assert_eq!(saved.len(), 2, "往復の2件が保存されること: {saved:?}");
    assert_eq!(saved[0].role, Role::User);
    assert_eq!(
        saved[0].raw_content, FIRST_USER,
        "保存する発言はクライアントの原文（前置きを含めない）"
    );

    // --- 2回目。**1回目が着地したセッション id を明示する。** ---
    let (status, second) = server
        .chat(&request_of(&format!("aoi@{session}"), SECOND_USER))
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");

    let sent = server.llm.last_request();
    assert_eq!(
        sent.messages.len(),
        4,
        "[システム, 1回目の発言, 1回目の応答, 今回の発言] の4件: {:?}",
        sent.messages
    );
    assert_eq!(
        sent.messages[1].content, FIRST_USER,
        "履歴に読み戻した発言は原文（過去のターンに記憶が付かない）"
    );
    assert_eq!(sent.messages[2].content, FIRST_REPLY);
    let latest = &sent.messages[3].content;
    assert!(
        latest.contains(&format!("> - {MEMORY}")) && latest.ends_with(&format!("\n{SECOND_USER}")),
        "前置きは今回の発言にだけ付く: {latest}"
    );

    let saved = server.saved_messages("aoi", session).await;
    assert_eq!(saved.len(), 4, "往復が積み上がること: {saved:?}");
    assert_eq!(saved[2].raw_content, SECOND_USER, "2回目の保存も原文");

    server.stop().await;
}

/// **往復は「解決したセッション」に書かれる。最新セッションではない。**
///
/// # なぜ既存のテストでは足りないのか
///
/// `an_explicit_session_id_continues_that_session` は**応答の `model` しか見ていない。**
/// `TurnSink` へ渡す id を `latest_session(character.id)` にすり替える変異は、
/// 応答の `model` を変えないので**全緑で通り抜ける**（レビュアーの実測: 38/38 緑）。
///
/// そのとき起きるのは、FE が古いセッション `aoi@1` を明示して再開したときに
/// **応答の `model` は `aoi@1` を返すのに、往復は別のセッション `aoi@2` に書かれる**
/// という壊れ方。`aoi@1` は次のターンでも履歴が空のままで、
/// `aoi@2` には無関係な会話が混入する。**どちらの側も 200 のままなので誰も気づけない。**
///
/// # 形
///
/// 1. `aoi@new` を2回叩いて**別々のセッションを2つ**作る（`new` の方が最新になる）。
/// 2. **古い方**を明示して3回目を叩く。
/// 3. 両方のセッションの**件数と中身**を見る。
///
/// **3ターンすべてで別の文字列**を使う（同値だと入れ違いが検出できない）。
#[tokio::test]
async fn a_turn_is_persisted_into_the_resolved_session_not_the_latest_one() {
    /// 古い方のセッションの1回目（発言と応答）。
    const OLD_FIRST_USER: &str = "古い方のセッションで話しかける";
    const OLD_FIRST_REPLY: &str = "「ここは1つ目の会話」 彼女は答えた。";
    /// 新しい方のセッション（**古い方とは別の文字列**）。
    const NEW_USER: &str = "新しい方のセッションを始める";
    const NEW_REPLY: &str = "「ここは2つ目の会話」 彼女は振り向いた。";
    /// 古い方へ戻ってきた3回目（**上のどれとも別の文字列**）。
    const OLD_SECOND_USER: &str = "古い方のセッションへ戻ってきた";
    const OLD_SECOND_REPLY: &str = "「おかえり、1つ目の続き」 彼女は微笑んだ。";

    let server = TestServer::start(MockLlm::replying_each(&[
        OLD_FIRST_REPLY,
        NEW_REPLY,
        OLD_SECOND_REPLY,
    ]))
    .await;
    server.seed("aoi", "葵").await;

    // 1. 古い方。
    let (status, first) = server.chat(&request_of("aoi@new", OLD_FIRST_USER)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let old_id = session_id_of(&first, "aoi");

    // 2. 新しい方。**こちらが「最新」になる。**
    let (status, second) = server.chat(&request_of("aoi@new", NEW_USER)).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let new_id = session_id_of(&second, "aoi");
    assert_ne!(old_id, new_id, "前提: 別のセッションであること");
    assert_eq!(
        server.latest_session_id("aoi").await,
        new_id,
        "前提: このあと明示する古い方と「最新」は別物であること（同じなら何も検出できない）"
    );

    // 3. **古い方**を明示して3回目。
    let (status, third) = server
        .chat(&request_of(&format!("aoi@{old_id}"), OLD_SECOND_USER))
        .await;
    assert_eq!(status, StatusCode::OK, "{third}");
    assert_eq!(
        session_id_of(&third, "aoi"),
        old_id,
        "応答の model は解決したセッションを指すこと"
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

/// **パースが崩れた応答でも `raw_content` が丸ごと残る**（絶対制約）。
///
/// 崩れた回は `dialogue` が `None`、`narration` は生出力そのもの（ADR-0007 の
/// フォールバック）。それでも**次のターンのプロンプトには生出力がそのまま載る**ので、
/// 会話は続く。
#[tokio::test]
async fn a_broken_response_keeps_its_raw_output_and_still_continues_the_conversation() {
    /// `「` が閉じない応答（`FallbackReason::Unclosed`）。
    const BROKEN: &str = "彼女は言った。「おかえりなさい";

    let server = TestServer::with_reply(BROKEN).await;
    server.seed("aoi", "葵").await;

    let (status, first) = server.chat(&request_of("aoi", "ただいま")).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let session = session_id_of(&first, "aoi");

    let saved = server.saved_messages("aoi", session).await;
    assert_eq!(
        saved.len(),
        2,
        "崩れた回も往復として保存すること: {saved:?}"
    );
    assert_eq!(
        saved[1].raw_content, BROKEN,
        "生出力を丸ごと保持すること（唯一の調査手段）"
    );
    assert_eq!(saved[1].dialogue, None, "崩れたら台詞を拾わないこと");
    assert_eq!(
        saved[1].narration.as_deref(),
        Some(BROKEN),
        "フォールバックは生出力を丸ごと地の文にする"
    );

    // **崩れた応答もそのまま履歴に載る**（意図した挙動）。
    let (status, second) = server
        .chat(&request_of(&format!("aoi@{session}"), "それで？"))
        .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let request = server.llm.last_request();
    assert_eq!(request.messages[2].role, ChatRole::Assistant);
    assert_eq!(request.messages[2].content, BROKEN);

    server.stop().await;
}

/// **上流が失敗した回は1行も保存しない**（利用者の発言も）。
///
/// 返答の無い利用者ターンが履歴に残ると、次のプロンプトに利用者の発言が2つ並ぶ。
#[tokio::test]
async fn a_failed_upstream_call_persists_nothing() {
    let server = TestServer::start(MockLlm::failing(|| LlmError::MissingContent)).await;
    server.seed("aoi", "葵").await;

    let (status, body) = server
        .chat(&request_of("aoi", "この発言は残らないこと"))
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");

    // セッションは作られている（`aoi` は最新か新規を返す）。そこに1行も入っていないこと。
    let character = server
        .store
        .find_character_by_handle(&name("aoi"))
        .await
        .expect("キャラクターを引けること")
        .expect("キャラクターが存在すること");
    let session = server
        .store
        .latest_session(character.id)
        .await
        .expect("セッションを引けること")
        .expect("セッションは作られていること");
    let saved = server
        .store
        .recent_messages(session.id, 50)
        .await
        .expect("会話履歴を読めること");

    assert!(
        saved.is_empty(),
        "応答を得られなかった回は利用者の発言も保存しないこと: {saved:?}"
    );

    server.stop().await;
}

/// B-2b の見張り番で使う3ターン。**すべて別の文字列**（同値だと入れ違いが検出できない）。
const TALKED_USER: &str = "港の灯台を見に行った話をしていたよね";
const TALKED_REPLY: &str = "「霧の日でも見えたね」 彼女は窓の外を指した。";
/// **502 で終わる回の発言。** どこにも保存されず、どのプロンプトにも載らない。
const ABANDONED_USER: &str = "ここから新しい話を始めたい";
const RESUMED_USER: &str = "さっきの続きだけど";
const RESUMED_REPLY: &str = "「うん、灯台の話ね」 彼女は頷いた。";

/// **上流が失敗した回の空セッションが、次の素の `aoi` を横取りしない**（B-2b / 実例あり）。
///
/// `chat` は上流を開くより前にセッションを作るので、502 で終わった回は
/// **`session` の行だけが残る**（`message` は0件）。`INSERT` でも `updated_at` は
/// 押し上がる（ADR-0004）ため、`updated_at` だけで「最新」を決めると
/// **その残骸が最新になり、素の `aoi` が会話ではなく空の続きに着地する。**
/// **画面上は普通に見えるので、履歴が分断されたことに気付けない。**
///
/// 2026-08-26 にブラウザで実例が作られている（`session` 1行 / `message` 0件）。
///
/// # 形
///
/// 1. `aoi` で1ターン成立させる（＝中身のあるセッション）。
/// 2. **その後で** `aoi@new` を上流で落とす。空セッションが**より新しく**残る。
///    （`failing_once` は1回目に効くので、この順序は `fail_the_next_call` でしか作れない。）
/// 3. 前提を確認する —— セッションが2つあり、**新しい方が空**であること。
///    ここを見ないと、そもそも残骸ができていない状態で緑になりうる。
/// 4. 素の `aoi` が 1. のセッションへ戻ること。
///
/// # id の一致だけでは足りない
///
/// 着地したセッションが正しくても**履歴が載らなければ**利用者から見た壊れ方
/// （会話が分断される）は同じなので、**上流へ渡った本文まで**見る。
#[tokio::test]
async fn an_empty_session_left_by_a_failed_turn_does_not_capture_the_bare_handle() {
    let server = TestServer::start(MockLlm::replying_each(&[TALKED_REPLY, RESUMED_REPLY])).await;
    server.seed("aoi", "葵").await;

    // 1. 会話が1ターン成立する。
    let (status, first) = server.chat(&request_of("aoi", TALKED_USER)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let talked = session_id_of(&first, "aoi");

    // 2. その後の `aoi@new` が上流で落ちる。**セッション行だけが残る。**
    server.llm.fail_the_next_call(LlmError::MissingContent);
    let (status, failed) = server.chat(&request_of("aoi@new", ABANDONED_USER)).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{failed}");

    // 3. 前提の確認。
    let sessions = server.sessions_with_message_counts("aoi").await;
    assert_eq!(
        sessions.len(),
        2,
        "上流が落ちた回のセッション行が残っていること: {sessions:?}"
    );
    let (talked_id, talked_messages, talked_at) = sessions[0].clone();
    let (abandoned_id, abandoned_messages, abandoned_at) = sessions[1].clone();
    assert_eq!(talked_id, talked, "1つ目が会話の方であること");
    assert_eq!(talked_messages, 2, "会話の方には往復が入っていること");
    assert_eq!(abandoned_messages, 0, "残骸に行が入っていないこと");
    assert!(
        abandoned_at > talked_at,
        "残骸の方が新しいこと（この前提が崩れるとこのテストは何も見ていない）: \
         残骸 = {abandoned_at} / 会話 = {talked_at}"
    );

    // 4. 素の `aoi` は会話の方へ戻る。
    let (status, resumed) = server.chat(&request_of("aoi", RESUMED_USER)).await;
    assert_eq!(status, StatusCode::OK, "{resumed}");
    assert_eq!(
        session_id_of(&resumed, "aoi"),
        talked,
        "空の残骸ではなく、会話の続きへ着地すること"
    );

    // **id が合っていても履歴が載らなければ「続き」ではない。**
    let request = server.llm.last_request();
    assert_eq!(
        request.messages.len(),
        4,
        "[システム, 1回目の発言, 1回目の応答, 今回の発言] の4件: {:?}",
        request.messages
    );
    assert_eq!(request.messages[1].content, TALKED_USER);
    assert_eq!(request.messages[2].content, TALKED_REPLY);
    assert_eq!(request.messages[3].content, RESUMED_USER);
    assert!(
        request
            .messages
            .iter()
            .all(|message| message.content != ABANDONED_USER),
        "上流が失敗した回の発言はどこにも残らないこと: {:?}",
        request.messages
    );

    // 往復は会話の方に積み上がり、残骸は空のまま。
    let sessions = server.sessions_with_message_counts("aoi").await;
    assert_eq!(sessions[0].1, 4, "会話の方に積み上がること: {sessions:?}");
    assert_eq!(sessions[1].1, 0, "残骸には書き込まれないこと: {sessions:?}");
    assert_eq!(sessions[1].0, abandoned_id, "残骸の id は変わらないこと");

    server.stop().await;
}

/// **`aoi@<id>` は中身の無いセッションにも届く**（B-2b で順位を下げただけ）。
///
/// 空セッションを「最新」の候補から**除外**する実装にすると、`find_session` まで
/// 巻き込んで「明示的に指したのに 404」になりうる。そこは ADR-0004 が用意した
/// 復旧経路なので塞がない。
///
/// **ただし 502 で終わった回の id は利用者に返っていない**ので、
/// 実運用でこの経路に入れるのは DB を直接覗いた場合だけである
/// （このテストも `sessions_with_message_counts` で id を拾っている）。
/// **その空セッションは読み出し経路から完全に外れる**、というのが B-2b の帰結。
#[tokio::test]
async fn an_explicit_id_reaches_a_session_that_has_no_messages() {
    let server = TestServer::start(MockLlm::replying_each(&[TALKED_REPLY, RESUMED_REPLY])).await;
    server.seed("aoi", "葵").await;

    let (status, first) = server.chat(&request_of("aoi", TALKED_USER)).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let talked = session_id_of(&first, "aoi");

    server.llm.fail_the_next_call(LlmError::MissingContent);
    let (status, failed) = server.chat(&request_of("aoi@new", ABANDONED_USER)).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{failed}");

    let sessions = server.sessions_with_message_counts("aoi").await;
    assert_eq!(sessions.len(), 2, "残骸があること: {sessions:?}");
    let empty = sessions[1].0;
    assert_eq!(sessions[1].1, 0, "残骸が空であること: {sessions:?}");

    // 明示すれば届く。
    let (status, body) = server
        .chat(&request_of(&format!("aoi@{empty}"), RESUMED_USER))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        session_id_of(&body, "aoi"),
        empty,
        "明示した空セッションに着地すること"
    );

    // **履歴は空**（もう一方の会話を引きずらない）。
    let request = server.llm.last_request();
    assert_eq!(
        request.messages.len(),
        2,
        "[システム, 今回の発言] の2件であること: {:?}",
        request.messages
    );
    assert_eq!(request.messages[1].content, RESUMED_USER);

    // 往復はそこに書かれ、会話の方は増えない。
    assert_eq!(
        server.saved_messages("aoi", empty).await.len(),
        2,
        "明示したセッションに書かれること"
    );
    assert_eq!(
        server.saved_messages("aoi", talked).await.len(),
        2,
        "もう一方には書かれないこと"
    );

    server.stop().await;
}

/// **`chat` は `Store::append_turn` を通る**（`append_message` を2回呼ぶ形にしない）。
///
/// # なぜこの見張り番が要るか
///
/// 逐次に1本ずつ叩くかぎり、`append_turn` と「`append_message` を2回」は
/// **まったく同じ行を同じ順序で書く。** つまり他のテストは全部緑のまま通る（実測）。
/// 差が出るのは**同時に書いたとき**だけで、非不可分な実装では
/// 往復の2件の間に別のリクエストの書き込みが割り込む。
/// **11b-1 の見張り番と同じ判定**（往復の2件の rowid が連続すること）をここで行う。
///
/// 割り込みが起きると「返答の無い利用者ターン」が履歴に残るのと同じ順序の乱れになり、
/// 次のターンのプロンプトに利用者の発言が2つ並ぶ。`prompt` は `raw_content` を
/// 読むだけなので、後段の誰も気づかない。
#[tokio::test]
async fn concurrent_turns_on_the_same_session_are_written_indivisibly() {
    /// 同時に投げる本数。**非不可分な実装が割り込まれるだけの数**が要る。
    const CONCURRENCY: usize = 16;

    let server = TestServer::with_reply("「はい」 彼女は頷いた。").await;
    server.seed("aoi", "葵").await;

    // セッションを1つ確定させる（以降は全部この id を明示する）。
    let (status, first) = server.chat(&request_of("aoi", "はじめまして")).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    let session = session_id_of(&first, "aoi");
    let model = format!("aoi@{session}");

    let mut tasks = Vec::with_capacity(CONCURRENCY);
    for index in 0..CONCURRENCY {
        let client = server.client.clone();
        let url = format!("http://{}/v1/chat/completions", server.addr);
        let body = request_of(&model, &format!("同時の発言 {index}"));
        tasks.push(tokio::spawn(async move {
            client
                .post(url)
                .json(&body)
                .send()
                .await
                .expect("サーバへ接続できること")
                .status()
        }));
    }
    for task in tasks {
        assert_eq!(
            task.await.expect("タスクが panic しないこと"),
            StatusCode::OK,
            "同時実行でも全部成功すること"
        );
    }

    let saved = server.saved_messages("aoi", session).await;
    assert_eq!(
        saved.len(),
        2 * (CONCURRENCY + 1),
        "全部の往復が入ること: {saved:?}"
    );

    // **2件ずつが1つの往復。** 割り込まれていれば役割か id の連続が崩れる。
    let (pairs, rest) = saved.as_chunks::<2>();
    assert!(rest.is_empty(), "往復が半端に残った: {saved:?}");
    for [user, reply] in pairs {
        assert_eq!(user.role, Role::User, "往復の並びが崩れた: {saved:?}");
        assert_eq!(reply.role, Role::Assistant, "往復の並びが崩れた: {saved:?}");
        assert_eq!(
            reply.id.get(),
            user.id.get() + 1,
            "往復の2件の間に別の書き込みが割り込んだ: {saved:?}"
        );
    }

    server.stop().await;
}

/// **保存に失敗しても応答は返す。** 500 に化けさせない。
///
/// 生成し終えた応答を捨てて 500 にすると、利用者から見れば「同じ発言をもう一度
/// 生成させられる」ことになり、失敗の代償として高すぎる（`server::turn` の doc）。
/// **黙ったデータ欠落**なので、代わりに `error!` が残る
/// （その `error!` が消えていないことは `tests/server_chat_parse_log.rs` が見張る）。
///
/// 上流の応答を返す直前に永続化層を閉じることで、**保存だけが失敗する**状況を作る。
///
/// # 「保存できなかった」ことまで確かめる
///
/// 200 と本文だけを見ていると、**細工が効いておらず普通に保存できていた**場合と
/// 区別が付かない（それでは「保存の失敗で応答を壊さない」を一度も試していない）。
/// 閉じたのは**プールだけ**で `TempDir` も DB ファイルも無傷なので、
/// **新しい接続で開き直せば**その回の行が入っていないことを直接読める。
#[tokio::test]
async fn a_failure_to_persist_never_breaks_the_response() {
    const REPLY: &str = "「保存できなくても返すこと」 彼女は頷いた。";
    /// 細工の前の発言。**保存されて残る方。**
    const WARMUP_USER: &str = "まずは普通に";
    /// 細工の後の発言。**1行も残ってはいけない方**（上と別の文字列）。
    const LOST_USER: &str = "ここから保存できない";

    let server = TestServer::with_reply(REPLY).await;
    server.seed("aoi", "葵").await;
    // 前提: 細工の無い状態なら保存まで通る（下の 200 が「元から壊れている」ではない）。
    let (status, warmup) = server.chat(&request_of("aoi", WARMUP_USER)).await;
    assert_eq!(status, StatusCode::OK, "{warmup}");
    let session = session_id_of(&warmup, "aoi");
    assert_eq!(
        server.saved_messages("aoi", session).await.len(),
        2,
        "前提: 細工の前は保存できていること"
    );

    server
        .llm
        .break_the_store_on_the_next_call(server.store.clone());

    let (status, body) = server
        .chat(&request_of(&format!("aoi@{session}"), LOST_USER))
        .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "保存の失敗を 500 に化けさせないこと: {body}"
    );
    assert_eq!(content_of(&body), REPLY, "応答の本文も落とさないこと");

    // **その回は本当に保存されていない。** プールは閉じているので開き直して読む。
    let saved = server
        .messages_from_a_fresh_connection("aoi", session)
        .await;
    assert_eq!(
        saved.len(),
        2,
        "前提: 細工が効いていて、その回は1行も書かれていないこと: {saved:?}"
    );
    assert_eq!(saved[0].raw_content, WARMUP_USER, "{saved:?}");
    assert!(
        saved.iter().all(|m| m.raw_content != LOST_USER),
        "保存できなかった回の発言が入っていないこと: {saved:?}"
    );

    // 永続化層を閉じたので `stop()` の期待（正常終了）はそのまま成立する。
    server.stop().await;
}

// ----------------------------------------------------------------------
// セッション同定の失敗
// ----------------------------------------------------------------------

/// **他キャラクターのセッション id では引けない**（`find_session` の所有確認）。
///
/// ここが抜けると、`model` に他人のセッション id を書くだけで別キャラの会話へ
/// 書き込めてしまう（ADR-0004 の「記憶の混線」）。
#[tokio::test]
async fn a_session_of_another_character_is_not_found() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;
    server.seed("rin", "凛乃").await;

    let (_, rin) = server.chat(&request_of("rin@new", "りんの会話")).await;
    let rin_session = session_id_of(&rin, "rin");
    // aoi 側にもセッションを作っておく（「そもそもセッションが無いから 404」ではないこと）。
    let (_, aoi) = server.chat(&request_of("aoi@new", "あおいの会話")).await;
    assert_ne!(session_id_of(&aoi, "aoi"), rin_session);

    let before = server.llm.call_count();
    let (status, body) = server
        .chat(&request_of(&format!("aoi@{rin_session}"), "混ぜてみる"))
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body,
        json!({
            "error": {
                "message": "指定されたセッションが存在しません",
                "type": "invalid_request_error",
                "code": "session_not_found",
            }
        }),
        "内部情報を載せないこと"
    );
    assert_eq!(
        server.llm.call_count(),
        before,
        "所有確認に落ちたら上流を呼ばないこと"
    );

    server.stop().await;
}

/// 存在しないセッション id も 404。
#[tokio::test]
async fn an_unknown_session_id_is_not_found() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    let (status, body) = server.chat(&request_of("aoi@999999", "だれ？")).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], json!("session_not_found"));
    assert_eq!(server.llm.call_count(), 0);

    server.stop().await;
}

/// 未知のキャラクターは 404 / `model_not_found`（ADR-0004）。
#[tokio::test]
async fn an_unknown_character_is_a_model_not_found() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    let (status, body) = server.chat(&request_of("dareka", "やあ")).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body,
        json!({
            "error": {
                "message": "指定された model のキャラクターが存在しません",
                "type": "invalid_request_error",
                "code": "model_not_found",
            }
        }),
        "リクエストの内容を反射しないこと"
    );
    assert_eq!(server.llm.call_count(), 0);

    server.stop().await;
}

/// **`ModelSelector::parse` の失敗を 404 `model_not_found` に潰さない。**
///
/// `aoi@new `（接尾辞に空白）が「そんなキャラクターは無い」に化けると、
/// 接尾辞を trim しない設計を知らない利用者は原因に到達できない。
#[tokio::test]
async fn a_malformed_model_is_not_reported_as_an_unknown_character() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    for model in ["aoi@new ", "aoi@ new", "aoi@0042", "aoi@-1", "aoi@NEW"] {
        let (status, body) = server.chat(&request_of(model, "やあ")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "model = {model:?}: {body}");
        assert_eq!(
            body,
            json!({
                "error": {
                    "message": "model の @ の後ろを解釈できません",
                    "type": "invalid_request_error",
                    "code": "model_invalid_suffix",
                }
            }),
            "model = {model:?}"
        );
    }

    // `aoi@` は「接尾辞が空」。**`aoi` と同じ扱いにも、上と同じ理由にもしない。**
    let (status, body) = server.chat(&request_of("aoi@", "やあ")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], json!("model_empty_suffix"));

    // 名前部の不正は名前部の理由で返す（404 でも接尾辞の理由でもない）。
    let (status, body) = server.chat(&request_of("   ", "やあ")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], json!("character_handle_empty"));

    assert_eq!(server.llm.call_count(), 0, "上流を呼ばないこと");

    server.stop().await;
}

// ----------------------------------------------------------------------
// リクエストの解釈
// ----------------------------------------------------------------------

/// **採用するのは最後の `user` メッセージだけ**（自前DBを正とする）。
#[tokio::test]
async fn only_the_last_user_message_is_sent_upstream() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    let (status, body) = server
        .chat(&json!({
            "model": "aoi",
            "messages": [
                {"role": "system", "content": "クライアントが勝手に付けたシステム指示"},
                {"role": "user", "content": "最初の発言"},
                {"role": "assistant", "content": "クライアントが持っている履歴"},
                {"role": "user", "content": "最後の発言"},
            ],
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let request = server.llm.last_request();
    let last = request.messages.last().expect("メッセージがあること");
    assert_eq!(last.role, ChatRole::User);
    assert_eq!(last.content, "最後の発言", "最後の user を採ること");

    for message in &request.messages {
        for ignored in [
            "最初の発言",
            "クライアントが持っている履歴",
            "クライアントが勝手に付けたシステム指示",
        ] {
            assert!(
                !message.content.contains(ignored),
                "クライアントの履歴を採用しないこと: {ignored:?} / {}",
                message.content
            );
        }
    }

    server.stop().await;
}

/// `user` のメッセージが1つも無ければ 400。
#[tokio::test]
async fn a_request_without_a_user_message_is_rejected() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    for messages in [
        json!([]),
        json!([{"role": "system", "content": "指示だけ"}]),
        json!([{"role": "assistant", "content": "返事だけ"}]),
    ] {
        let (status, body) = server
            .chat(&json!({"model": "aoi", "messages": messages}))
            .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            body,
            json!({
                "error": {
                    "message": "messages に user のメッセージがありません",
                    "type": "invalid_request_error",
                    "code": "no_user_message",
                }
            })
        );
    }
    assert_eq!(server.llm.call_count(), 0);

    server.stop().await;
}

/// **採用しないメッセージの `content` は一切見ない**（ADR-0001 の互換性）。
///
/// `tool_calls` 付きの履歴を持つクライアントは `content: null` の `assistant` を送るし、
/// マルチモーダル対応のクライアントは配列形式の `content` を混ぜてくる。
/// **採用するのは最後の `user` だけ**なので、それ以外の形は要求全体を落とす理由にならない
/// （`deny_unknown_fields` を付けなかったのと同じ理由が、フィールドの型の側にもある）。
#[tokio::test]
async fn content_shapes_we_never_adopt_do_not_reject_the_whole_request() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    let (status, body) = server
        .chat(&json!({
            "model": "aoi",
            "messages": [
                {"role": "system", "content": [{"type": "text", "text": "配列形式の指示"}]},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "search", "arguments": "{}"},
                    }],
                },
                {"role": "tool", "content": "ツールの出力", "tool_call_id": "call_1"},
                {"role": "assistant"},
                {"role": "user", "content": "こんにちは"},
            ],
        }))
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let request = server.llm.last_request();
    let last = request.messages.last().expect("メッセージがあること");
    assert_eq!(last.role, ChatRole::User);
    assert_eq!(last.content, "こんにちは");

    server.stop().await;
}

/// 採用対象（**最後の `user`**）が文字列でなければ、**専用のコードで 400**。
///
/// * `invalid_body` に潰さない — 「本文が壊れている」と「この形式の content は未対応」は
///   原因が違い、送った側の直し方も違う。
/// * **より前の `user` へフォールバックしない** — 古い発言に黙って答えることになる。
#[tokio::test]
async fn an_unsupported_content_shape_is_rejected_without_falling_back() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    for content in [
        json!([{"type": "text", "text": "画像つきの発言"}]),
        json!(null),
        json!(42),
        json!({"type": "text", "text": "オブジェクト形式"}),
    ] {
        let (status, body) = server
            .chat(&json!({
                "model": "aoi",
                "messages": [
                    {"role": "user", "content": "ひとつ前の発言"},
                    {"role": "assistant", "content": "その返事"},
                    {"role": "user", "content": content},
                ],
            }))
            .await;

        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "content = {content}: {body}"
        );
        assert_eq!(
            body,
            json!({
                "error": {
                    "message": "この形式の content にはまだ対応していません",
                    "type": "invalid_request_error",
                    "code": "unsupported_message_content",
                }
            }),
            "`invalid_body` に潰さないこと: content = {content}"
        );
    }

    // `content` のキーごと無い場合も同じ（`null` と区別しない）。
    let (status, body) = server
        .chat(&json!({
            "model": "aoi",
            "messages": [
                {"role": "user", "content": "ひとつ前の発言"},
                {"role": "user"},
            ],
        }))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], json!("unsupported_message_content"));

    assert_eq!(
        server.llm.call_count(),
        0,
        "古い発言に黙って答えないこと（上流を呼ばない）"
    );

    server.stop().await;
}

/// 壊れた JSON は**入力を反射せずに** 400。
#[tokio::test]
async fn a_broken_body_is_rejected_without_reflecting_the_input() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    for raw in [
        "{".to_owned(),
        "not json".to_owned(),
        json!({"model": "aoi"}).to_string(),
        json!({"messages": [{"role": "user", "content": "x"}]}).to_string(),
        json!({"model": 1, "messages": []}).to_string(),
        json!({"model": "aoi", "messages": "秘密の文字列"}).to_string(),
    ] {
        let (status, body) = server.chat_raw(raw.clone()).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "raw = {raw}: {body}");
        assert_eq!(
            parse_json(&body),
            json!({
                "error": {
                    "message": "リクエスト本文の形式が不正です",
                    "type": "invalid_request_error",
                    "code": "invalid_body",
                }
            }),
            "serde のメッセージを反射しないこと: raw = {raw}"
        );
        assert!(!body.contains("秘密の文字列"), "raw = {raw}: {body}");
    }

    server.stop().await;
}

/// **本文の上限超過は 413。**「JSON が壊れている」と読める 400 に吸収しない。
///
/// 判定は `ApiError::from_json_rejection`（`/admin/` と共有）にある。
/// **片方だけ 400 に潰れる形の劣化が起きないよう、両方の入口から見張る。**
#[tokio::test]
async fn an_oversized_chat_body_is_rejected_as_too_large() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    // 3MB。中身は正当な JSON だが、上限で読む前に打ち切られる。
    let raw = request_of("aoi", &"あ".repeat(1_000_000)).to_string();
    assert!(raw.len() > 2 * 1024 * 1024, "前提: 既定の上限を超えること");

    let (status, text) = server.chat_raw(raw).await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{text}");
    assert_eq!(
        parse_json(&text),
        json!({
            "error": {
                "message": "リクエスト本文が大きすぎます",
                "type": "invalid_request_error",
                "code": "body_too_large",
            }
        }),
        "上限超過を JSON の形式エラーに潰さないこと"
    );
    assert!(!text.contains('あ'), "入力を反射しないこと");
    assert_eq!(server.llm.call_count(), 0);

    server.stop().await;
}

/// **413 は、上限を大きく超える本文を送っている最中でも最後まで届く。**
///
/// # このテストを緩めてはいけない理由
///
/// 2026-08-14 に実測した製品欠陥の再現手順そのもの（docs/06-roadmap.md「0005 の範囲」）。
/// 2.86MB で 15 回中 **9 回**が応答を失い、2.10MB では 15 回中 0 回だった。
/// **超過量が大きいほど壊れる**サイズ依存の競合なので、
/// **本文サイズを 2.1MB 付近まで下げると緑になるが、それは欠陥を隠しただけ。**
/// 同じ理由で `#[ignore]` にもしないこと。
///
/// 壊れ方:
///
/// 1. 本文上限は Content-Length ではなく**実際に読んだバイト数**で打ち切られる
/// 2. サーバは上限に達した時点で読むのをやめ、413 を書いて接続を閉じる
/// 3. クライアントはまだ書いている。**未読データを残して閉じると OS が RST を送る**ので、
///    クライアントの write が失敗し、**受信済みの 413 も破棄される**
///
/// 直っている状態＝「413 を返すと決めた後、残りの本文を上限付きで読み捨ててから応答する」。
/// **1回の往復では取りこぼす**（当時の実測で 6/15 は成功していた）ので繰り返す。
#[tokio::test]
async fn an_oversized_chat_body_receives_the_whole_413_over_a_raw_socket() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    // 3MB。中身は正当な JSON だが、上限で読む前に打ち切られる。
    let raw = request_of("aoi", &"あ".repeat(1_000_000)).to_string();
    assert!(raw.len() > 2 * 1024 * 1024, "前提: 既定の上限を超えること");

    for attempt in 1..=5 {
        let (status_line, body) =
            post_over_raw_tcp(server.addr, "/v1/chat/completions", &raw).await;

        assert!(
            status_line.starts_with("HTTP/1.1 413"),
            "{attempt} 回目: 413 であること: {status_line}"
        );
        assert_eq!(
            parse_json(&body),
            json!({
                "error": {
                    "message": "リクエスト本文が大きすぎます",
                    "type": "invalid_request_error",
                    "code": "body_too_large",
                }
            }),
            "{attempt} 回目: 本文を最後まで読めること"
        );
        assert!(!body.contains('あ'), "{attempt} 回目: 入力を反射しないこと");
    }

    assert_eq!(server.llm.call_count(), 0);

    server.stop().await;
}

/// **`stream: false` を黙って中継に振り替えない。**
///
/// 明示的に非ストリーミングを要求したクライアントに SSE を返すと、
/// 本文が `text/event-stream` になって JSON として読めなくなる。
/// **中継そのものの検証は `tests/server_chat_stream.rs`**（作業単位 11a-2b）。
#[tokio::test]
async fn an_explicit_stream_false_stays_on_the_non_streaming_path() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    let (status, body) = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "こんにちは"}],
            "stream": false,
        }))
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["object"], json!("chat.completion"));
    assert_eq!(content_of(&body), "はい。");
    assert_eq!(server.llm.call_count(), 1);
    assert_eq!(server.llm.stream_count(), 0, "中継経路は使わないこと");

    server.stop().await;
}

// ----------------------------------------------------------------------
// 上流へ渡す内容
// ----------------------------------------------------------------------

/// **`max_tokens` は生成用の余白で上限を切る。**
///
/// クライアントの値を素通しすると、プロンプト＋応答が上流のコンテキストを超えうる。
#[tokio::test]
async fn max_tokens_is_capped_by_the_response_room() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;
    let room = TokenBudget::default().response_tokens();

    // 巨大な値はクランプされる。
    let (status, body) = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "こんにちは"}],
            "max_tokens": 1_000_000,
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        server.llm.last_request().max_tokens,
        Some(room),
        "クライアントの値を素通ししないこと"
    );

    // 余白より小さい値はそのまま通る。
    let (status, body) = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "こんにちは"}],
            "max_tokens": 16,
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(server.llm.last_request().max_tokens, Some(16));

    // 未指定なら余白いっぱい。
    let (status, body) = server.chat(&request_of("aoi", "こんにちは")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(server.llm.last_request().max_tokens, Some(room));

    server.stop().await;
}

/// **`max_tokens: 0` は入口で拒否する。** 黙って上流へ流さない。
///
/// `cap_max_tokens` は「余白まで切り下げる」関数なので `Some(0)` をそのまま通す。
/// OpenAI 側も最小 1 なので、0 は要求として成立していない。
#[tokio::test]
async fn a_zero_max_tokens_is_rejected_at_the_door() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    let (status, body) = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "こんにちは"}],
            "max_tokens": 0,
        }))
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body,
        json!({
            "error": {
                "message": "max_tokens には 1 以上を指定してください",
                "type": "invalid_request_error",
                "code": "invalid_max_tokens",
            }
        })
    );
    assert_eq!(
        server.llm.call_count(),
        0,
        "0 を上流へ送らないこと（1文字も生成できない要求になる）"
    );

    // 1 は正当（境界）。
    let (status, body) = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "こんにちは"}],
            "max_tokens": 1,
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(server.llm.last_request().max_tokens, Some(1));

    server.stop().await;
}

/// **`temperature` はクライアントの値を通す**（マイルストーン5は `temperature = 0` で測る）。
#[tokio::test]
async fn the_client_temperature_is_passed_through() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    let (status, body) = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "こんにちは"}],
            "temperature": 0,
        }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        server.llm.last_request().temperature,
        Some(0.0),
        "0 を「未指定」に潰さないこと"
    );

    // 未指定なら上流の既定に任せる（`None` のまま送らない）。
    let (status, _) = server.chat(&request_of("aoi", "こんにちは")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(server.llm.last_request().temperature, None);

    server.stop().await;
}

/// 上流へはシステムプロンプトが載り、**`model` 由来の文字列は混入しない。**
///
/// `ChatRequest` に `model` が無いのは意図的な型の制約（ADR-0004）。
/// 加えて、`aoi@new` のような**生の `model` 値**がプロンプト本文へ紛れ込まないことを見る。
///
/// **識別子と表示名を別の値にしてある**（`s10` / `碧`）。同じ値だと、載っているのが
/// `display_name` なのか `handle` なのかをこのテストでは区別できず、
/// `server` が両者を取り違えても緑のまま通る（`seed` の doc を参照）。
/// `prompt` 層は `SanitizedCharacter` が識別子を持たないので型で守られているが、
/// **`store` から `prompt` への受け渡しを見張れるのは結合テストのここだけ。**
#[tokio::test]
async fn the_upstream_request_carries_the_system_prompt_but_never_the_model_field() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("s10", "碧").await;
    server
        .seed_stats("s10", &[("ロジック", 3), ("パッション", 0)])
        .await;

    let (status, body) = server.chat(&request_of("s10@new", "こんにちは")).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let request = server.llm.last_request();
    assert_eq!(
        request.messages.len(),
        2,
        "`@new` で始めた直後は履歴が無いので [システム, 今回の発言] の2件: {:?}",
        request.messages
    );

    let system = &request.messages[0];
    assert_eq!(system.role, ChatRole::System);
    assert!(
        system.content.contains("落ち着いた口調で話す。"),
        "ペルソナが載ること: {}",
        system.content
    );
    // **載るのは表示名。** 識別子（`s10`）は1文字も現れない（ADR-0011）。
    assert!(
        system.content.contains("碧"),
        "表示名が載ること: {}",
        system.content
    );
    assert!(
        !system.content.contains("s10"),
        "識別子はプロンプトに載らないこと: {}",
        system.content
    );
    assert!(
        system.content.contains("ロジック"),
        "性格ステータスが載ること: {}",
        system.content
    );
    assert!(
        !system.content.contains("パッション"),
        "値 0 の軸は載らないこと: {}",
        system.content
    );

    for message in &request.messages {
        // **このキャラクター名で実際に一致しうる形だけを並べる。**
        // `"aoi@"` のような他キャラ由来の値は原理的に一致せず、
        // 「3つ見ている」ように読めて実質2つになる。
        for leak in ["s10@new", "@new", "s10@"] {
            assert!(
                !message.content.contains(leak),
                "生の model 値が混入しないこと: {leak:?} / {}",
                message.content
            );
        }
    }

    server.stop().await;
}

// ----------------------------------------------------------------------
// 失敗経路が内部情報を漏らさない
// ----------------------------------------------------------------------

/// **予算を超えた要求は 422 で「収まらない」と伝える。500 に潰さない。**
///
/// これは異常系ではなく普通に踏む経路。`estimate_tokens` は実質 UTF-8 バイト長で、
/// 今回の発言は**クライアントが送る自由文（上限なし）が丸ごと削れない枠に載る**ので、
/// 7KB 強（日本語で約 2,400 文字）を貼り付けただけでここに来る。
/// `internal()` に潰すと、利用者は「サーバが壊れた」としか読めず原因に到達できない。
#[tokio::test]
async fn a_user_message_that_does_not_fit_the_budget_is_reported_as_such() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    // 削れない枠（プロンプト取り分）を、今回の発言だけで超えさせる。
    // 「あ」は UTF-8 で 3 バイトなので、取り分と同じ文字数で確実に超える。
    let allowance =
        usize::try_from(TokenBudget::default().prompt_allowance()).expect("usize に収まること");
    let user_message = "あ".repeat(allowance);

    let (status, body) = server.chat(&request_of("aoi", &user_message)).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        body,
        json!({
            "error": {
                "message": "キャラクター設定と発言がコンテキスト予算に収まりません",
                "type": "invalid_request_error",
                "code": "prompt_budget_exhausted",
            }
        }),
        "500 に潰さないこと（原因が運用者に伝わらなくなる）"
    );
    assert_eq!(
        server.llm.call_count(),
        0,
        "組み立てられなかったプロンプトを上流へ送らないこと"
    );
    assert!(!body.to_string().contains('あ'), "入力を反射しないこと");

    // 収まる長さなら通る（上の 422 が「常に 422」ではないことの確認）。
    let (status, body) = server.chat(&request_of("aoi", "こんにちは")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(server.llm.call_count(), 1);

    server.stop().await;
}

// ----------------------------------------------------------------------
// コンテキスト予算（ADR-0013 第2層）
// ----------------------------------------------------------------------

/// **予算は設定から来る。`TokenBudget::default()` を直に呼んでいないこと。**
///
/// ここが既定値に固定されていると、LM Studio 側で 16384 をロードしていても
/// **8192 しか使わない**（実測。ADR-0013 の出発点）。逆に設定を小さくしても
/// 減らないなら、窓を超えたプロンプトを送る状態を素通りさせる（LM Studio では前方から黙って
/// 切り捨てられる / ADR-0013。llama は 400 `exceed_context_size_error` で断るか、
/// プロンプト ＋ 応答の上限が窓を超えて応答が黙って短くなる / ADR-0052 M5）。
/// **両方向を見る**ことで「設定が無視されている」を捕まえる。
#[tokio::test]
async fn the_prompt_budget_comes_from_the_configuration() {
    // 既定の取り分（8192 - 1024 = 7168）を超えるが、16384 なら収まる長さ。
    // 「あ」は UTF-8 で 3 バイト。
    let long = "あ".repeat(3_000);
    // 既定なら収まるが、2048（取り分 1024）では収まらない長さ。
    let short = "あ".repeat(400);

    // 1. **前提**: 既定値では長い方が入らず、短い方は入る。
    let server = TestServer::with_reply("「はい」").await;
    server.seed("aoi", "葵 碧").await;
    let (status, body) = server.chat(&request_of("aoi", &long)).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "前提: 既定の予算には収まらない長さであること: {body}"
    );
    let (status, body) = server.chat(&request_of("aoi", &short)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "前提: 既定の予算には収まる長さであること: {body}"
    );
    server.stop().await;

    // 2. **設定を大きくすると通る。** 既定値を直に使っていたら 422 のまま。
    let server = TestServer::start_with_env(
        MockLlm::replying("「はい」"),
        &[(ENV_CONTEXT_TOKENS, "16384")],
    )
    .await;
    server.seed("aoi", "葵 碧").await;
    let (status, body) = server.chat(&request_of("aoi", &long)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "設定した 16384 が `chat` まで届いていないこと（既定値 8192 のまま）: {body}"
    );
    assert_eq!(server.llm.call_count(), 1, "上流まで到達すること");
    server.stop().await;

    // 3. **設定を小さくすると通らない。** 予算が設定と連動していることの逆向きの証明。
    let server = TestServer::start_with_env(
        MockLlm::replying("「はい」"),
        &[(ENV_CONTEXT_TOKENS, "2048")],
    )
    .await;
    server.seed("aoi", "葵 碧").await;
    let (status, body) = server.chat(&request_of("aoi", &short)).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "設定した 2048 が `chat` まで届いていないこと: {body}"
    );
    assert_eq!(
        server.llm.call_count(),
        0,
        "組み立てられなかったプロンプトを上流へ送らないこと"
    );
    server.stop().await;
}

/// **上流へ照会できなくても起動する。** 既定値へ落ちて動き続ける。
///
/// `/api/v0/models` は **LM Studio 独自**なので、上流を差し替えれば必ず失敗する。
/// ここで起動を止めると「LM Studio が起動していないと本サービスも起動しない」という
/// 逆立ちした依存になる（ADR-0013 第2層は「取れたら得をする」経路）。
///
/// **モックを注入しない `App::bind` を通す**唯一のテスト。上流は
/// **閉じたポート**へ向けるので、実機の LM Studio には繋がらない。
#[tokio::test]
async fn the_server_starts_even_when_the_context_length_cannot_be_queried() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let db_path = dir.path().join("personacore.sqlite");
    // **誰も待ち受けていないアドレス。** 実機（127.0.0.1:1234）は叩かない。
    let closed = closed_address().await;

    let config = AppConfig::from_lookup(|key| match key {
        ENV_LLM_ENDPOINT => Some(closed.clone()),
        _ => None,
    })
    .expect("設定を組み立てられること")
    .with_bind(BindAddr::loopback(0))
    .with_database_path(&db_path);

    // **照会に失敗しても `bind` は成功すること。**
    let app = App::bind(&config)
        .await
        .expect("上流へ照会できなくても待ち受けを開始できること");
    let addr = app.local_addr();
    let store = app.store().clone();

    let (shutdown, rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        app.serve(async move {
            let _ = rx.await;
        })
        .await
    });

    store
        .create_character(
            &name("aoi"),
            &display("葵 碧"),
            &persona("落ち着いた口調で話す。"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    let client = reqwest::Client::new();
    let post = |body: Value| {
        let client = client.clone();
        async move {
            let response = client
                .post(format!("http://{addr}/v1/chat/completions"))
                .json(&body)
                .send()
                .await
                .expect("サーバへ接続できること");
            response.status()
        }
    };

    // **既定値へ落ちていること。** 既定の取り分（7168）を超える発言は 422、
    // 収まる発言は上流まで進んで 503（接続できない）になる。
    assert_eq!(
        post(request_of("aoi", &"あ".repeat(3_000))).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "既定値より大きい予算が湧いていないこと"
    );
    assert_eq!(
        post(request_of("aoi", "こんにちは")).await,
        StatusCode::SERVICE_UNAVAILABLE,
        "予算を通り抜けて上流まで進むこと（照会の失敗で経路が壊れていないこと）"
    );

    shutdown.send(()).expect("停止シグナルを送れること");
    handle
        .await
        .expect("サーバタスクが panic しないこと")
        .expect("正常に終了すること");
}

// ----------------------------------------------------------------------
// 本番の配線（`App::bind`）
//
// **`TestServer` はここを通らない。** あちらはテスト専用の `bind_with_client` を
// 通るので、`App::bind` が「明示設定を渡すこと」「照会した予算を渡すこと」を
// 一切観測しない。実測でも、`App::bind` の該当2行を
// `resolve_budget(None, ..)` / `bind_with(.., TokenBudget::default())` に潰しても
// 全件緑だった（11c-1 のレビュー指摘）。**本番の入口はここだけが見張る。**
// ----------------------------------------------------------------------

/// 実機で観測した `/api/v0/models` の応答（2026-08-14 / ADR-0013）。
///
/// **`max_context_length` と `loaded_context_length` は必ず違えておくこと。**
/// 同じ値だと、どちらを読んでいるかがここから区別できなくなる。
/// `PLACEHOLDER` はモデル識別子、`LOADED_CONTEXT` は `LARGER_CONTEXT_TOKENS` に差し替える
/// （**予算の算出と申告値がずれないように、直書きしない**）。
const NATIVE_MODELS_BODY: &str = r#"{
    "object": "list",
    "data": [
        {"id": "PLACEHOLDER", "object": "model", "type": "llm",
         "state": "loaded", "max_context_length": 262144,
         "loaded_context_length": LOADED_CONTEXT}
    ]
}"#;

/// 上流が申告する／明示設定で与える大きい方のコンテキスト長。
/// `NATIVE_MODELS_BODY` の `loaded_context_length` と `ENV_CONTEXT_TOKENS` の設定値。
const LARGER_CONTEXT_TOKENS: u32 = 16_384;

/// 「あ」1文字あたりの見積もり。**UTF-8 のバイト数と 1:1**（ADR-0013 第1層）。
const BYTES_PER_CHAR: usize = 3;

/// `context_tokens` の予算がプロンプトに使える取り分（バイト）。
fn allowance_of(context_tokens: u32) -> usize {
    TokenBudget::new(context_tokens, TokenBudget::DEFAULT_RESPONSE_TOKENS)
        .expect("テストで使う予算は正当であること")
        .prompt_allowance() as usize
}

/// 既定の取り分は超えるが、大きい方の取り分には収まる長さ（「あ」の文字数）。
///
/// **予算から算出する。直書きしない。** 既定予算を将来引き上げると、直書きの値は
/// 「既定でも収まる」側に落ちてしまい、**「ヘルパの既定（閉じたポート）が壊れた」と
/// 読める文言でこのテストが落ちる**（原因を取り違えさせる）。
/// 両者の取り分の中点を採るので、どちら側にも余裕がある。
fn fits_only_in_the_larger_budget() -> usize {
    allowance_of(TokenBudget::DEFAULT_CONTEXT_TOKENS).midpoint(allowance_of(LARGER_CONTEXT_TOKENS))
        / BYTES_PER_CHAR
}

/// 大きい方の取り分にも収まらない長さ（予算が青天井になっていないことの確認用）。
///
/// **この発言だけで取り分を超える**ので、システムプロンプトの大きさに依存しない。
fn fits_in_neither_budget() -> usize {
    allowance_of(LARGER_CONTEXT_TOKENS) / BYTES_PER_CHAR + 1
}

/// 誰も待ち受けていないアドレス（**上流が停止している状態**の再現）。
///
/// 実機（`127.0.0.1:1234`）は叩かない。
async fn closed_address() -> String {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("一時ポートを確保できること");
    let addr = listener.local_addr().expect("アドレスを取得できること");
    drop(listener);
    format!("http://{addr}")
}

/// **`App::bind`（本番の入口）で起動した実サーバ。**
///
/// モックは注入しない。上流は必ずローカルの閉じたポートかモックサーバへ向けるので、
/// 実機の LM Studio には繋がらない（docs/05-development.md）。
struct BoundServer {
    addr: SocketAddr,
    /// アプリと**同じプールを指す**ハンドル。属性の投入に使う（12b）。
    store: Store,
    client: reqwest::Client,
    /// **走っているサーバが読んでいる予算そのもの**（`App::budget_handle`）。
    ///
    /// 引き直しは 5 分ごと（ADR-0020 §2）なので、実時間では待てない。
    /// 「上流が答えを変えた」ところまでを外から起こして、
    /// **ハンドラが現在値を読むか**だけを見るための口。
    budget: SharedBudget,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl BoundServer {
    /// 設定を与えて起動する。
    ///
    /// **プロセスの環境変数には触らない**（`AppConfig::from_lookup` に参照関数を渡す）。
    /// `std::env::set_var` を使うと並行するテストを巻き添えにする（`server::config` の doc）。
    ///
    /// **上流を指定しない呼び出しは、閉じたポートを既定にする。**
    /// `ENV_LLM_ENDPOINT` を渡さないと `DEFAULT_ENDPOINT`（`127.0.0.1:1234`）へ
    /// **実機宛の照会が飛ぶ**。この機械では LM Studio が起動していることがあるので、
    /// 照会が成功したりしなかったりして**テスト結果が機械の状態で変わる**
    /// （docs/05-development.md「実際の LM Studio には繋がない」）。
    /// 呼び出し側の注意ではなくここで塞ぐ。
    async fn start(env: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db_path = dir.path().join("personacore.sqlite");

        let mut pairs: Vec<(String, String)> = env
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        if !pairs.iter().any(|(name, _)| name == ENV_LLM_ENDPOINT) {
            pairs.push((ENV_LLM_ENDPOINT.to_owned(), closed_address().await));
        }

        // **既定を構造的に固定する。** これを振る舞い
        //（`nothing_configured_means_the_default_budget` の 422）だけに任せると、
        // **実機の LM Studio が動いていて 16384 をロードしているときにしか赤にならない。**
        // ここで直接主張すれば、機械の状態に関わらず赤になる。
        let endpoint = pairs
            .iter()
            .find(|(name, _)| name == ENV_LLM_ENDPOINT)
            .map(|(_, value)| value.trim().trim_end_matches('/').to_owned())
            .expect("上流のエンドポイントが決まっていること");
        assert_ne!(
            endpoint, DEFAULT_ENDPOINT,
            "実機（{DEFAULT_ENDPOINT}）へ照会しないこと。\
             上流は必ず閉じたポートかモックサーバへ向ける（docs/05-development.md）"
        );
        let config = AppConfig::from_lookup(|key| {
            pairs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(&db_path);

        // **`bind_with_client` ではない。** ここが本番と同じ入口であることが要点。
        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let store = app.store().clone();
        let budget = app.budget_handle();

        store
            .create_character(
                &name("aoi"),
                &display("葵 碧"),
                &persona("落ち着いた口調で話す。"),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");

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
            budget,
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    /// 「あ」を `chars` 文字並べた発言を投げ、ステータスだけ受け取る。
    async fn status_for(&self, chars: usize) -> StatusCode {
        self.client
            .post(format!("http://{}/v1/chat/completions", self.addr))
            .json(&request_of("aoi", &"あ".repeat(chars)))
            .send()
            .await
            .expect("サーバへ接続できること")
            .status()
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

/// **`PERSONACORE_CONTEXT_TOKENS` は本番の入口でも効く。**
///
/// `the_prompt_budget_comes_from_the_configuration` は `bind_with_client` を通るので、
/// **`App::bind` が `config.context_budget()` を渡すのをやめても気付かない。**
/// ここが無いと、本番でだけ明示設定が黙って無視される変更が素通りする。
#[tokio::test]
async fn the_explicit_context_setting_survives_the_real_bind() {
    let closed = closed_address().await;
    let configured = LARGER_CONTEXT_TOKENS.to_string();
    let server = BoundServer::start(&[
        (ENV_CONTEXT_TOKENS, &configured),
        (ENV_LLM_ENDPOINT, &closed),
    ])
    .await;

    // 既定値のままなら 422。設定した 16384 が届いていれば上流まで進んで 503。
    assert_eq!(
        server.status_for(fits_only_in_the_larger_budget()).await,
        StatusCode::SERVICE_UNAVAILABLE,
        "設定した {LARGER_CONTEXT_TOKENS} が `App::bind` から `chat` まで届いていないこと\
         （既定値 {} のまま）",
        TokenBudget::DEFAULT_CONTEXT_TOKENS
    );
    // 逆向き: 予算が青天井になっているわけではない。
    assert_eq!(
        server.status_for(fits_in_neither_budget()).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{LARGER_CONTEXT_TOKENS} でも収まらない長さは 422 のままであること"
    );

    server.stop().await;
}

/// **上流へ照会した実コンテキスト長が本番の入口を通って `chat` まで届く。**
///
/// 明示設定は与えない。ここが死ぬと、LM Studio が 16384 をロードしていても
/// **8192 しか使わない**状態がエラーも出さずに続く（ADR-0013 の出発点そのもの）。
#[tokio::test]
async fn the_queried_context_length_survives_the_real_bind() {
    let upstream = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                NATIVE_MODELS_BODY
                    .replace("PLACEHOLDER", DEFAULT_MAIN_MODEL)
                    .replace("LOADED_CONTEXT", &LARGER_CONTEXT_TOKENS.to_string()),
                "application/json",
            ),
        )
        .mount(&upstream)
        .await;

    // **明示設定は与えない。** 照会結果だけが既定値を上回る根拠になる。
    let server = BoundServer::start(&[(ENV_LLM_ENDPOINT, &upstream.uri())]).await;

    // 上流は `/v1/chat/completions` を持たない（未 mount ＝ 404）ので、
    // 予算を通り抜けたことは 502 として観測できる。**422 なら届いていない。**
    assert_eq!(
        server.status_for(fits_only_in_the_larger_budget()).await,
        StatusCode::BAD_GATEWAY,
        "照会した {LARGER_CONTEXT_TOKENS} が `App::bind` から `chat` まで届いていないこと\
         （既定値 {} のまま）",
        TokenBudget::DEFAULT_CONTEXT_TOKENS
    );
    // 逆向き: 申告された 16384 を超えて膨らんではいない。
    assert_eq!(
        server.status_for(fits_in_neither_budget()).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "申告値を超える予算が湧いていないこと（比率で膨らませない）"
    );

    assert!(
        upstream
            .received_requests()
            .await
            .is_some_and(|requests| requests
                .iter()
                .any(|request| request.url.path() == "/api/v0/models")),
        "起動時に独自 API へ照会していること"
    );

    server.stop().await;
}

/// 引き直しで縮んだ後のコンテキスト長（ADR-0020 の筋書き: 上流が小さく再ロードされた）。
///
/// **既定値とも `LARGER_CONTEXT_TOKENS` とも別の数**にすること。既定値と同じにすると、
/// 「差し替わった」のか「起動時の値を捨てて既定へ落ちた」のかを区別できない。
const SHRUNK_CONTEXT_TOKENS: u32 = 4_096;

/// **走っているサーバのハンドラは、共有の予算の「現在値」を読む**（ADR-0020）。
///
/// 引き直しが新しい値を書いても、`AppState` が起動時の値を**写し取って**いれば
/// 何も変わらない——上流が 4096 で再ロードされたのに 16384 のつもりで組み続け、
/// **窓を超える。** LM Studio では**ペルソナ・性格・記憶が黙って切り捨てられる**（ADR-0013）。
/// llama は 400 `exceed_context_size_error` で断るか、プロンプト ＋ 応答の上限が窓を超えて
/// 応答が黙って短くなる（ADR-0052 M5）。
///
/// # 5分を待たない
///
/// 引き直しの間隔は 5 分の定数（ADR-0020 §2）なので、本物の引き直しは実時間では待てない。
/// ここでは**引き直しが値を差し替えたのと同じこと**を `App::budget_handle` から起こし、
/// **ハンドラ側が現在値を読むか**だけを見る（引き直し自体の振る舞いは
/// `server::context` の単体テストと `tests/server_context_refresh.rs`）。
///
/// # 往復で見る
///
/// 縮めた後に元へ戻して 502 に返ることまで見る。**片道だけだと、
/// 2回目以降を無条件に 422 にする壊れ方と区別が付かない。**
#[tokio::test]
async fn the_running_budget_is_shared_with_the_handlers() {
    assert_ne!(
        SHRUNK_CONTEXT_TOKENS,
        TokenBudget::DEFAULT_CONTEXT_TOKENS,
        "前提: 縮んだ後の値は既定値と別であること"
    );
    assert_ne!(
        SHRUNK_CONTEXT_TOKENS, LARGER_CONTEXT_TOKENS,
        "前提: 縮んだ後の値は起動時の申告値と別であること"
    );

    let upstream = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                NATIVE_MODELS_BODY
                    .replace("PLACEHOLDER", DEFAULT_MAIN_MODEL)
                    .replace("LOADED_CONTEXT", &LARGER_CONTEXT_TOKENS.to_string()),
                "application/json",
            ),
        )
        .mount(&upstream)
        .await;

    let server = BoundServer::start(&[(ENV_LLM_ENDPOINT, &upstream.uri())]).await;
    let message = fits_only_in_the_larger_budget();

    // 起動時の照会値（16384）で組めている。上流に `/v1/chat/completions` は無いので 502。
    assert_eq!(
        server.status_for(message).await,
        StatusCode::BAD_GATEWAY,
        "起動時の照会値で組めていること"
    );

    // **引き直しが値を差し替えたのと同じこと**を起こす。
    server.budget.replace_for_tests(
        TokenBudget::new(SHRUNK_CONTEXT_TOKENS, TokenBudget::DEFAULT_RESPONSE_TOKENS)
            .expect("テストで使う予算は正当であること"),
    );

    assert_eq!(
        server.status_for(message).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "差し替えた {SHRUNK_CONTEXT_TOKENS} が次の組立から効くこと\
         （起動時の {LARGER_CONTEXT_TOKENS} を写し取っていないこと）"
    );

    // 元へ戻せば元の判定に戻る（一方通行の壊れ方ではない）。
    server.budget.replace_for_tests(
        TokenBudget::new(LARGER_CONTEXT_TOKENS, TokenBudget::DEFAULT_RESPONSE_TOKENS)
            .expect("テストで使う予算は正当であること"),
    );

    assert_eq!(
        server.status_for(message).await,
        StatusCode::BAD_GATEWAY,
        "戻した値も次の組立から効くこと"
    );

    server.stop().await;
}

/// **何も設定しなければ既定の予算で起動する。**
///
/// `BoundServer::start` の**上流の既定**（閉じたポート）を通る唯一のテスト。
/// ここを `DEFAULT_ENDPOINT`（`127.0.0.1:1234`）に戻すと、**実機の LM Studio が
/// 起動しているかどうかで結果が変わる**（起動していて 16384 がロード済みなら、
/// 照会が成功して 422 が 502/503 に化ける）。テストは機械の状態に依存させない。
///
/// **既定そのものの見張りはこのテストではなくヘルパ側の `assert_ne!`。**
/// こちらは実機が動いているときにしか赤にならないので、
/// 気付ける機会を増やす分として残してある。
#[tokio::test]
async fn nothing_configured_means_the_default_budget() {
    let server = BoundServer::start(&[]).await;

    assert_eq!(
        server.status_for(fits_only_in_the_larger_budget()).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "照会できないなら既定値（{}）の取り分に収まらない発言は 422 であること",
        TokenBudget::DEFAULT_CONTEXT_TOKENS
    );
    // 逆向き: 予算そのものが壊れているわけではない（収まる発言は上流まで進む）。
    assert_eq!(
        server.status_for(8).await,
        StatusCode::SERVICE_UNAVAILABLE,
        "既定の取り分に収まる発言は上流まで進むこと（上流は閉じているので 503）"
    );

    server.stop().await;
}

/// **接続できないときは 503 で「起動しているか確認してください」と案内する。**
///
/// LM Studio は GPU を占有し続けないよう普段は停止している運用（docs/04-environment.md）
/// なので、**この環境で最も高頻度に起こる失敗**。他の上流エラー（502）に潰すと、
/// 「通信に失敗しました」としか出ず、起動を促せない。
#[tokio::test]
async fn an_upstream_that_is_not_running_asks_the_operator_to_start_it() {
    let server = TestServer::start(MockLlm::failing_once(connect_error().await)).await;
    server.seed("aoi", "葵").await;

    let (status, body) = server.chat(&request_of("aoi", "こんにちは")).await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(
        body,
        json!({
            "error": {
                "message": "推論サーバへ接続できませんでした。起動しているか確認してください",
                "type": "server_error",
                "code": "upstream_unavailable",
            }
        }),
        "502 `upstream_failed` に潰さないこと"
    );
    // `reqwest::Error` の `Display`（接続先の URL を含む）を反射しない。
    for leak in ["127.0.0.1", "http", "tcp", "os error"] {
        assert!(
            !body.to_string().contains(leak),
            "内部情報 {leak:?} が漏れていないこと: {body}"
        );
    }

    server.stop().await;
}

/// **応答が時間内に完了しなければ 504。** 接続できない場合（503）と分ける。
///
/// 分けないと「起動していない」と「モデルのロードが終わらない」が同じ応答になり、
/// 待てば直るのか起動が要るのかが判断できない。
#[tokio::test]
async fn an_upstream_timeout_is_reported_as_a_gateway_timeout() {
    let server = TestServer::start(MockLlm::failing_once(timeout_error().await)).await;
    server.seed("aoi", "葵").await;

    let (status, body) = server.chat(&request_of("aoi", "こんにちは")).await;

    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT, "{body}");
    assert_eq!(
        body,
        json!({
            "error": {
                "message": "推論サーバの応答が時間内に完了しませんでした",
                "type": "server_error",
                "code": "upstream_timeout",
            }
        }),
        "503 `upstream_unavailable` とも 502 `upstream_failed` とも分けること"
    );
    for leak in ["127.0.0.1", "http", "os error"] {
        assert!(
            !body.to_string().contains(leak),
            "内部情報 {leak:?} が漏れていないこと: {body}"
        );
    }

    server.stop().await;
}

/// 上流の失敗は**定数だけ**を返す（`{err:?}` を載せない）。
#[tokio::test]
async fn an_upstream_failure_does_not_leak_internal_details() {
    let server = TestServer::start(MockLlm::failing(|| LlmError::MissingContent)).await;
    server.seed("aoi", "葵").await;

    let (status, body) = server.chat(&request_of("aoi", "こんにちは")).await;

    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(
        body,
        json!({
            "error": {
                "message": "推論サーバとの通信に失敗しました",
                "type": "server_error",
                "code": "upstream_failed",
            }
        }),
        "詳細を足さないこと（`{{err:?}}` を載せない）"
    );

    server.stop().await;
}

/// **上流が鍵を断った（401 / 403）ら `upstream_unauthorized`、ほかのステータス異常は今までどおり
/// `upstream_failed`**（ADR-0056 §決定4 / 作業単位 A-13b-1）。
///
/// **状態は 502 のまま**——401 を返すと、クライアントは「personacore への鍵が違う」と読む。
/// 断ったのは personacore の先の上流である。**上流のステータス値は反射しない。**
#[tokio::test]
async fn an_upstream_that_refuses_the_key_is_reported_apart_from_other_failures() {
    let unauthorized = json!({
        "error": {
            "message": "推論サーバが鍵を受け付けませんでした",
            "type": "server_error",
            "code": "upstream_unauthorized",
        }
    });
    let failed = json!({
        "error": {
            "message": "推論サーバとの通信に失敗しました",
            "type": "server_error",
            "code": "upstream_failed",
        }
    });
    for (upstream_status, expected) in [
        (401, &unauthorized),
        (403, &unauthorized),
        (400, &failed),
        (404, &failed),
        (407, &failed),
        (500, &failed),
        (503, &failed),
    ] {
        let server = TestServer::start(MockLlm::failing_once(LlmError::Status {
            status: upstream_status,
        }))
        .await;
        server.seed("aoi", "葵").await;

        let (status, body) = server.chat(&request_of("aoi", "こんにちは")).await;

        assert_eq!(status, StatusCode::BAD_GATEWAY, "{upstream_status}: {body}");
        assert_eq!(&body, expected, "上流 {upstream_status}");
        assert!(
            !body.to_string().contains(&upstream_status.to_string()),
            "上流のステータスを反射しないこと: {body}"
        );
        server.stop().await;
    }
}

/// **鍵を平文で外へ送る承認が無くて送らなかったら `upstream_key_not_approved`**
/// （ADR-0056 §決定5 / 作業単位 A-13b-2）。**`upstream_unauthorized` とも `upstream_failed` とも違う**
/// —— 上流は鍵を見ていない。状態は 502（401 / 403 にすると「personacore への鍵が違う」と読まれる）。
#[tokio::test]
async fn a_send_without_the_plain_http_approval_is_reported_as_key_not_approved() {
    let server = TestServer::start(MockLlm::failing_once(LlmError::KeyNotApproved)).await;
    server.seed("aoi", "葵").await;

    let (status, body) = server.chat(&request_of("aoi", "こんにちは")).await;

    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(
        body,
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

/// 上流のステータス異常も定数だけ（上流のステータス値を反射しない）。
#[tokio::test]
async fn an_upstream_status_error_does_not_reflect_the_upstream_status() {
    let server = TestServer::start(MockLlm::failing(|| LlmError::Status { status: 429 })).await;
    server.seed("aoi", "葵").await;

    let (status, body) = server.chat(&request_of("aoi", "こんにちは")).await;

    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(
        body,
        json!({
            "error": {
                "message": "推論サーバとの通信に失敗しました",
                "type": "server_error",
                "code": "upstream_failed",
            }
        })
    );
    assert!(!body.to_string().contains("429"), "{body}");

    server.stop().await;
}

/// DB の失敗も**定数だけ**（パス・sqlx の型名を漏らさない）。
#[tokio::test]
async fn a_store_failure_does_not_leak_internal_details() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    // 稼働中にプールを閉じ、キャラクターの照合を確実に失敗させる。
    server.store.close().await;

    let (status, text) = server
        .chat_raw(request_of("aoi", "こんにちは").to_string())
        .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{text}");
    assert_eq!(
        parse_json(&text),
        json!({"error": {"message": "内部エラーが発生しました", "type": "server_error"}}),
        "本文は定数のみであること"
    );
    for leak in [
        server.db_path.as_str(),
        "sqlite",
        "sqlx",
        "PoolClosed",
        "src\\",
        "src/",
    ] {
        assert!(
            !text.contains(leak),
            "内部情報 {leak:?} が漏れていないこと: {text}"
        );
    }

    server.stop().await;
}

// ----------------------------------------------------------------------
// 答え合わせの配線（ADR-0013 第3層）
//
// **`server::usage` の単体テストは、`chat` がその関数を一度も呼ばなくても緑のまま。**
// 呼び出しごと消える変更を捕まえられるのはここだけ。
// ----------------------------------------------------------------------

/// 上界が破れたときのログの手掛かり。**全文一致にしないこと**
/// （文言の推敲で、見張りたい「実際に照合が走るか」と無関係に割れる）。
const UPPER_BOUND_MARKER: &str = "上界が破れ";

/// 想定コンテキストを超えたときのログの手掛かり。
/// **こちらだけが `max_tokens` と `context_tokens` を並べて載せる。**
const OVERRUN_MARKER: &str = "想定コンテキスト長を超え";

/// 答え合わせのテストでクライアントが指定する生成上限。
///
/// **既定の余白（`DEFAULT_RESPONSE_TOKENS` = 1024）と別の値にすること。**
/// 同値だと、`chat` が `usage::report` へ渡しているのが切り下げ後の `max_tokens` なのか
/// 予算の余白なのかを区別できない。
const REQUESTED_MAX_TOKENS: u32 = 16;

/// **答え合わせが実際のリクエスト経路で走る**（ADR-0013 第3層）。
///
/// 上流が「プロンプトは 100,000 トークンだった」と申告すれば、
/// こちらの見積もり（既定予算 8,192 の枠に収まる程度）を必ず超える。
/// **`chat` から `usage::report` の呼び出しを消すと、この警告が消えて赤になる。**
///
/// # なぜグローバルの subscriber を差すのか
///
/// 判定はサーバのタスク側で走るので、スレッドローカルな `with_default` では捕まらない。
/// **このテストバイナリで `set_global_default` を呼ぶのはここだけ。**
/// 並行して走る他のテストのログも同じバッファに混ざるが、この文言を出すのは
/// ここだけなので「含むか」の判定は壊れない。
///
/// **⚠ 「ここだけ」の根拠はコメントではない**（作業単位 B-3）。
/// 差し込みは [`common::install_global_log`] の `OnceLock` に畳んであり、
/// **2度目の呼び出しは同じバッファを返すだけ**なので `set_global_default` は1回しか走らない。
/// 器（`Write` / `MakeWriter`）も `src/testlog/assert.rs` と同じ1本を読む
/// ——**このファイルに写しを作り直さないこと**（押さえの見張り番はあちらにしか無い）。
///
/// **逆向き（正常時に鳴らないこと）はここでは見られない。** 他のテストの出力が
/// 混ざるため。そちらは `server::usage` の単体テストが見張っている。
///
/// # 既定の予算で起動しないこと
///
/// **`TestServer::start`（既定予算 8192）だと `state.budget()` と
/// `TokenBudget::default()` が同値になり、判別できない**（11c-2 のレビュー指摘。
/// 実測でこの2つを入れ替える変異が緑を通った）。実運用は上流が 16384 を申告する
/// 構成（ADR-0013）なので、そこで退行が入ると
/// **正常なリクエストで「想定コンテキスト長を超えました」が毎回鳴る。**
/// 同じ理由で `max_tokens` も**余白（1024）と別の値**を指定する。
#[tokio::test]
async fn the_usage_crosscheck_runs_on_the_real_request_path() {
    let buffer = install_global_log(personacore::server::DEFAULT_LOG_FILTER);

    let configured = LARGER_CONTEXT_TOKENS.to_string();
    assert_ne!(
        LARGER_CONTEXT_TOKENS,
        TokenBudget::DEFAULT_CONTEXT_TOKENS,
        "前提: 既定の予算と別の値であること（同値だと設定が届いたか分からない）"
    );
    let server = TestServer::start_with_env(
        MockLlm::replying_with_usage("「はい」", 100_000),
        &[(ENV_CONTEXT_TOKENS, &configured)],
    )
    .await;
    server.seed("aoi", "葵").await;

    let mut body = request_of("aoi", "こんにちは");
    body["max_tokens"] = json!(REQUESTED_MAX_TOKENS);
    assert_ne!(
        REQUESTED_MAX_TOKENS,
        TokenBudget::DEFAULT_RESPONSE_TOKENS,
        "前提: 余白と別の値であること（同値だと切り下げ後の値か余白か分からない）"
    );

    let (status, _) = server.chat(&body).await;
    assert_eq!(status, StatusCode::OK, "応答そのものは返ること");

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
        has_field(overrun, &format!("context_tokens={LARGER_CONTEXT_TOKENS}")),
        "設定した予算を答え合わせに使うこと（既定値 {} ではない）: {overrun:?}",
        TokenBudget::DEFAULT_CONTEXT_TOKENS
    );
    // **切り下げ後の `max_tokens` がそのまま届いていること。**
    // `0` や予算の余白に潰れると、超過の判定が実際と食い違う。
    assert!(
        has_field(overrun, &format!("max_tokens={REQUESTED_MAX_TOKENS}")),
        "上流へ渡した生成上限をそのまま載せること: {overrun:?}"
    );

    server.stop().await;
}

/// メソッド不一致は 405（`/v1/chat/completions` が生えていることの確認も兼ねる）。
#[tokio::test]
async fn the_endpoint_rejects_other_methods() {
    let server = TestServer::with_reply("はい。").await;

    let response = server
        .client
        .get(format!("http://{}/v1/chat/completions", server.addr))
        .send()
        .await
        .expect("サーバへ接続できること");

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);

    server.stop().await;
}

/// **属性が本番の入口（`App::bind`）を通ってプロンプトまで届く**（ADR-0009 / 12b）。
///
/// `prompt` 側の単体テストは `PromptInput` に属性を**手で詰めて**呼ぶので、
/// **`chat` が `Store::character_attributes` を呼んでいるか**は一切見ていない。
/// モック注入の `TestServer` を使うと `App::bind` の配線がまた観測対象から外れる
/// （CLAUDE.md「テスト専用の入口を作ると…」）ので、**ここは `BoundServer`。**
///
/// # フィクスチャ
///
/// **`user` と `character` の両方**を持たせ、**同じキー（`年齢`）で値を変える。**
/// 片方しか持たないフィクスチャでは「全部キャラクター扱い」の取り違えが検出できない。
#[tokio::test]
async fn character_attributes_reach_the_prompt_through_the_real_bind() {
    let upstream = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,
                "model":"m","choices":[{"index":0,
                "message":{"role":"assistant","content":"「はい」"},
                "finish_reason":"stop"}]}"#,
            "application/json",
        ))
        .mount(&upstream)
        .await;

    let server = BoundServer::start(&[(ENV_LLM_ENDPOINT, &upstream.uri())]).await;

    let character = server
        .store
        .find_character_by_handle(&name("aoi"))
        .await
        .expect("検索できること")
        .expect("`BoundServer` が作ったキャラクターが引けること");
    let attributes = AttributeSet::parse(vec![
        CharacterAttribute {
            subject: Subject::User,
            key: AttributeKey::parse("年齢").expect("正当なキーであること"),
            value: AttributeValue::parse("24").expect("正当な値であること"),
        },
        CharacterAttribute {
            subject: Subject::Character,
            key: AttributeKey::parse("年齢").expect("正当なキーであること"),
            value: AttributeValue::parse("19").expect("正当な値であること"),
        },
        CharacterAttribute {
            subject: Subject::Character,
            key: AttributeKey::parse("一人称").expect("正当なキーであること"),
            value: AttributeValue::parse("僕").expect("正当な値であること"),
        },
    ])
    .expect("件数の上限に収まること");
    server
        .store
        .replace_character_attributes(character.id, &attributes)
        .await
        .expect("属性を保存できること");

    let response = server
        .client
        .post(format!("http://{}/v1/chat/completions", server.addr))
        .json(&request_of("aoi@new", "こんにちは"))
        .send()
        .await
        .expect("サーバへ接続できること");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await.expect("本文を読めること")
    );

    let requests = upstream
        .received_requests()
        .await
        .expect("要求を記録していること");
    let forwarded = requests
        .iter()
        .find(|request| request.url.path() == "/v1/chat/completions")
        .expect("上流へ中継していること");
    let body: Value = serde_json::from_slice(&forwarded.body).expect("JSON であること");
    let system = body["messages"][0]["content"]
        .as_str()
        .expect("システムプロンプトが文字列であること");
    assert_eq!(
        body["messages"][0]["role"], "system",
        "先頭がシステムであること: {body}"
    );

    // **属性は「# 記憶」節ではなく専用の節**（ADR-0009）。ここから先は
    // **その節だけを切り出して見る。**
    //
    // **プロンプト全体で探さないこと。** 主語の小見出しは「# 属性」節と「# 記憶」節が
    // **同じ定数を共有している**（12b）ので、送る本文の全体で探すと、属性の主語が片方に潰れても
    // **記憶の側の小見出しで緑になる。** A-15 から「# 記憶」節は system ではなく最新の利用者の発言の
    // 前置きに出る（ADR-0061）ので、system だけを見ている限りは記憶の小見出しと混ざらない。
    // それでも節で切る形は残す —— 測定で悪ければ記憶を system へ戻す（ADR-0061 §決定5）ので、
    // 戻した日に同じ穴が開く。
    let attribute_at = system
        .find("# 属性")
        .unwrap_or_else(|| panic!("属性の節が出ること: {system}"));
    let persona_at = system
        .find("# ペルソナ")
        .unwrap_or_else(|| panic!("ペルソナの節が出ること: {system}"));
    assert!(
        attribute_at < persona_at,
        "属性の節は名前とペルソナのあいだであること: {system}"
    );
    let section = &system[attribute_at..persona_at];

    // **キーと値の対で見る。** 「載っている」だけを見ると取り違えが通る。
    for line in ["> - 年齢: 24", "> - 年齢: 19", "> - 一人称: 僕"] {
        assert!(section.contains(line), "{line:?} が載ること: {section}");
    }

    // **小見出しは全文で照合する。** 前方一致の短い綴り（`## 利用者について`）だと
    // **小見出しの文面が変わっても気付けない。**
    let user_heading = section
        .find("## 利用者について（利用者の事実であり、あなたの事実ではありません）")
        .unwrap_or_else(|| panic!("利用者の小見出しが出ること: {section}"));
    let own_heading = section
        .find("## あなた自身について（キャラクター本人の事実）")
        .unwrap_or_else(|| panic!("キャラクターの小見出しが出ること: {section}"));

    // **主語の区別が読み取れること。** 24（利用者）と 19（キャラクター）は
    // それぞれの小見出しの配下に入る。
    let user_at = section.find("> - 年齢: 24").expect("あること");
    let own_at = section.find("> - 年齢: 19").expect("あること");
    assert!(
        user_heading < user_at && user_at < own_heading,
        "24 は利用者の小見出しの配下であること: {section}"
    );
    assert!(
        own_heading < own_at,
        "19 はキャラクターの小見出しの配下であること: {section}"
    );

    server.stop().await;
}

/// **属性が1行でも壊れていたら、黙って空として続けない**（`chat.rs` の手順 7）。
///
/// # 何を固定しているのか
///
/// `Store::character_attributes` は 1 行壊れると丸ごと `InvalidStoredValue` を返す
/// （`store::attribute` の doc）。`chat` はそれを 500 にして**止める。**
/// **`unwrap_or_default()` で空として続けない**——属性が落ちたキャラクターは
/// 種族も一人称も失った**別人**として応答してしまう。
///
/// **判断はコメントに書かれていたが、固定するテストが1本も無かった**（12b のレビュー）。
/// 実測: `.map_err(…)?` を `.unwrap_or_default()` に変える変異は **959 全緑**を通った。
///
/// # 壊れた行の作り方
///
/// スキーマの CHECK（`typeof` / NUL / trim / 12 文字）は**制御文字を見ていない**ので、
/// `AttributeKey::parse` だけが拒否する行を作れる。
/// `tests/store_attribute.rs` と同じ `PRAGMA ignore_check_constraints` の helper を使う。
///
/// # 上流を呼んでいないことまで見る
///
/// 状態符号だけだと「上流に投げてから失敗した」形と区別できない。
/// **属性を読めない時点で止まる**ことが要点なので、`call_count()` を 0 で固定する。
#[tokio::test]
async fn a_broken_attribute_row_stops_the_turn_instead_of_dropping_the_attributes() {
    let server = TestServer::with_reply("はい。").await;
    server.seed("aoi", "葵").await;

    let character = server
        .store
        .find_character_by_handle(&name("aoi"))
        .await
        .expect("検索できること")
        .expect("作ったキャラクターが引けること");

    // 正当な属性を1件入れてから、**DB 直編集で1行だけ壊す。**
    let attributes = AttributeSet::parse(vec![CharacterAttribute {
        subject: Subject::Character,
        key: AttributeKey::parse("一人称").expect("正当なキーであること"),
        value: AttributeValue::parse("僕").expect("正当な値であること"),
    }])
    .expect("件数の上限に収まること");
    server
        .store
        .replace_character_attributes(character.id, &attributes)
        .await
        .expect("属性を保存できること");
    insert_character_attribute_ignoring_checks(
        &server.store,
        character.id.get(),
        "character",
        "種族\u{0007}",
        "猫獣人",
    )
    .await;

    let (status, body) = server.chat(&request_of("aoi@new", "こんにちは")).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(
        body,
        json!({"error": {"message": "内部エラーが発生しました", "type": "server_error"}}),
        "本文は定数のみであること"
    );
    assert_eq!(
        server.llm.call_count(),
        0,
        "属性を読めない時点で止まり、上流を呼ばないこと"
    );

    server.stop().await;
}

/// **上書き済みの記憶は、どの層からも引かれない**（ADR-0022 / ADR-0023 §結果）。
///
/// # なぜ 15b で改めて要るのか
///
/// 全件注入のうちは「`superseded_by IS NULL` で絞る」で話が終わっていた
/// （`tests/store_memory.rs::superseded_memories_are_excluded_but_not_deleted`）。
/// **層3が入ると、新しい経路が1本できる** —— 語の重なりで引く層は、
/// **利用者が誤りをそのまま口に出した瞬間に、その誤りといちばん強く重なる。**
/// ADR-0023 が「訂正済みの記憶を索引に載せないこと。**消したはずの誤りが戻る**」と
/// 名指ししているのはこの形である。
///
/// # フィクスチャ
///
/// **発言は、上書きされた側の本文をそのまま含む。** こうしないと
/// 「そもそも関連度が低いから載らなかった」と区別が付かない
/// ——**層3が引きに行く状態を作ってから、引かれないことを見る。**
/// 訂正版は語を共有しない別の文にしてあるので、
/// **載っているのが訂正版であることも同時に読める。**
#[tokio::test]
async fn a_superseded_memory_is_never_recalled_even_when_the_user_repeats_it() {
    /// 上書きされた（誤った）記憶。**発言がこれをそのまま含む。**
    const WRONG_MEMORY: &str = "犬のコーヒーを飼育している";
    /// 訂正版。**誤りと語を共有しすぎない**ようにしてある。
    const CORRECTION: &str = "黒猫のコーヒーと暮らす";

    let upstream = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,
                "model":"m","choices":[{"index":0,
                "message":{"role":"assistant","content":"「はい」"},
                "finish_reason":"stop"}]}"#,
            "application/json",
        ))
        .mount(&upstream)
        .await;

    let server = BoundServer::start(&[(ENV_LLM_ENDPOINT, &upstream.uri())]).await;

    let character = server
        .store
        .find_character_by_handle(&name("aoi"))
        .await
        .expect("検索できること")
        .expect("`BoundServer` が作ったキャラクターが引けること");

    // **主語は利用者**（A-11a）。キャラクター自身の記憶は本文の想起に載らないので、
    // `Character` のままだと「上書きされたから載らない」が「主語で落ちたから載らない」と
    // 区別できず、下の否定側が何も見なくなる。
    let wrong = server
        .store
        .add_memory(
            character.id,
            &NewMemory {
                subject: Subject::User,
                content: MemoryContent::parse(WRONG_MEMORY).expect("正当な本文であること"),
                category: None,
            },
        )
        .await
        .expect("記憶を追加できること");
    server
        .store
        .supersede_memory(
            character.id,
            wrong,
            &NewMemory {
                subject: Subject::User,
                content: MemoryContent::parse(CORRECTION).expect("正当な本文であること"),
                category: None,
            },
        )
        .await
        .expect("訂正できること");

    // **発言は誤りの本文をそのまま含む**（層3が引きに行く状態）。
    let utterance = format!("{WRONG_MEMORY}んだよね？");
    let response = server
        .client
        .post(format!("http://{}/v1/chat/completions", server.addr))
        .json(&request_of("aoi@new", &utterance))
        .send()
        .await
        .expect("サーバへ接続できること");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await.expect("本文を読めること")
    );

    let requests = upstream
        .received_requests()
        .await
        .expect("要求を記録していること");
    let forwarded = requests
        .iter()
        .find(|request| request.url.path() == "/v1/chat/completions")
        .expect("上流へ中継していること");
    let body: Value = serde_json::from_slice(&forwarded.body).expect("JSON であること");
    let last = body["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .and_then(|message| message["content"].as_str())
        .expect("今回の発言が最後にあること");

    // **記憶は最新の利用者の発言の前置きに載る**（ADR-0061 / 作業単位 A-15）。同じメッセージに
    // 今回の発言（誤りの本文を含む）も載るので、**引用行（`> ` で始まる行）だけ**を見る ——
    // 今回の発言は引用されないので、引用行に誤りの本文があれば、それは記憶の節から来たものである。
    let quoted: Vec<&str> = last.lines().filter(|line| line.starts_with("> ")).collect();
    assert!(
        quoted.iter().any(|line| line.contains(CORRECTION)),
        "有効な記憶（訂正版）が載っていない:\n{last}"
    );
    assert!(
        !quoted.iter().any(|line| line.contains(WRONG_MEMORY)),
        "上書き済みの記憶が想起へ戻っている:\n{last}"
    );
    // 記憶は system には載らない（前置きへ移った）。
    let system = body["messages"][0]["content"]
        .as_str()
        .expect("システムプロンプトが文字列であること");
    assert!(
        !system.contains(CORRECTION) && !system.contains(WRONG_MEMORY),
        "記憶が system に載った:\n{system}"
    );
    // 前提: 発言の側には現に載っている（＝語が重なる状態を作れている）。
    assert!(
        last.lines()
            .any(|line| !line.starts_with("> ") && line.contains(WRONG_MEMORY)),
        "前提: 発言が誤りの本文を含んでいること: {last:?}"
    );

    server.stop().await;
}
