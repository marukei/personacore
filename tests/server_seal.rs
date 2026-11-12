//! **封**（ADR-0052 §推奨3〜7 / §新しく要る見張り番 N1・N3・N4・N7 / 作業単位 B-60b・B-60d-1・B-60d-2）の結合検証。
//!
//! **実際の llama も LM Studio も起こさない。** 上流は `wiremock` の偽物か注入したモックで、
//! llama.cpp の `/apply-template`・`/tokenize` と、LM Studio の「未知の口に 200 ＋ エラー本文」を演じる。
//!
//! 見ているのは次のもの:
//!
//! | テスト | 主張 |
//! |---|---|
//! | [`counting_survives_the_face_client`]（N7） | **本番の入口（`App::bind`）から `dyn` を通って、封の数がログの行に出る**（非ストリーミングと中継の両方） |
//! | [`an_upstream_that_cannot_count_is_asked_once_per_generation`]（N7 の後半） | 数えられない上流は**1回だけ**聞き、同じ世代の間は聞きに行かない。世代が変われば聞き直す |
//! | [`an_upstream_that_cannot_count_is_sent_exactly_the_byte_plan`] | **数えられない上流（404 / 200 ＋ エラー本文）には、バイト数の計画そのものを送る**（B-60d-1 で、B-60b の「送る本文は1バイトも変わらない」を置き換えた） |
//! | [`a_count_that_failed_is_not_remembered`] / [`a_count_refused_while_loading_is_not_remembered`] | 一時的な失敗は覚えない |
//! | [`the_seal_and_the_send_use_one_copy_of_the_upstream`] | **数える写しと送る写しが同じ**（ADR-0052 §推奨3） |
//! | [`a_prompt_is_sent_only_after_the_upstream_counted_it_within_the_allowance`]（N1） | **送った本文を数えた数が取り分以下**。超えたら1回だけ組み直す |
//! | [`a_seal_that_still_overflows_falls_back_to_the_byte_plan`]（N3） | 組み直しても超えたら、**バイト数の計画で送る**（`BudgetExhausted` にしない） |
//! | [`a_shared_upstream_leaves_room_for_one_background_request`] | 本文と裏方が同じ上流なら、**裏方の1要求分を窓から差し引く**（ADR-0052 §決定7）。差し引く量は変容の応答の上限で決まりうる |
//! | [`a_shared_upstream_through_the_real_bind_deducts_the_background_window_it_queried`] | 同上を**本番の入口**で。差し引く量は**上流が申告した裏方の窓**から出る |
//! | [`a_counted_plan_still_warns_when_the_usage_overruns_the_context`] | 数えた計画の回も**コンテキストの超過は警告する**（上界の破れは言わない） |
//! | [`a_rebuilt_turn_logs_one_recall_line_for_the_plan_it_sent`] | 組み直したターンでも**想起の行は1本、送った計画のもの** |
//! | [`a_usage_above_the_sealed_count_turns_the_face_back_to_bytes`]（N4） | **`封 < 実` なら `warn`（封・実数・計画の和が同じ行）、次のターンはバイト数**。`封 = 実` なら次も封をする。組み直した回（送った本文の封で比べる）・バイト数の計画に封をした回の腕も持つ |
//! | [`a_sealed_count_above_the_usage_does_not_turn_the_face_back`] | `封 > 実` では掛け金が掛からない |
//! | [`a_latch_is_released_when_the_generation_changes`] | 答えたモデルが別の名前に変わったら掛け金が外れる。引き直しの失敗（名前が `None`）では外れない |
//!
//! # なぜ専用のテストバイナリなのか
//!
//! 封の行はサーバのタスク側で出るので、`set_global_default` が要る
//! （`tests/server_chat_parse_log.rs` のモジュールの doc と同じ理由）。
//! **このバイナリのテストは1本ずつ走る**（[`LogSection`]）——
//! どのテストも封の行を出すので、並べると他のテストの行で主張が満たされうる。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use personacore::domain::{ChatMessage, DisplayName, Handle, NewMessage, Persona, Role, SessionId};
use personacore::llm::{
    ChatChunk, ChatRequest, ChatStream, Completion, DEFAULT_ENDPOINT, FinishReason, LlmClient,
    LlmFuture, TokenCount, TokenUsage,
};
use personacore::measured::ModelFace;
use personacore::prompt::{
    CountedParts, MESSAGE_OVERHEAD_TOKENS, PromptInput, TokenBudget, build_prompt,
};
use personacore::server::{
    App, AppConfig, BindAddr, ENV_CONTEXT_TOKENS, ENV_LLM_ENDPOINT, ENV_LLM_IDLE_TIMEOUT,
    ENV_MAIN_MODEL, ENV_SUB_LLM_ENDPOINT, ENV_SUB_MODEL, ServerError,
};
use personacore::store::Store;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

mod common;

use common::{SharedBuffer, fixture_scale, has_field, has_key, install_global_log};

/// 封の行の手掛かり。**全文一致にしない**（文言の推敲で割れる）。
const SEAL_MARKER: &str = "封を観測しました";

/// テストで使うキャラクターの識別子。
const HANDLE: &str = "aoi";

/// `/apply-template` が返す組み上がり（**植えた値**。本文のどれとも違う綴り）。
const PLANTED_TEMPLATE: &str = "<|planted_start|>system\n封の見張り番<|planted_end|>\n";

/// 偽の上流が数える数。**実数（下の2つ）とも互いとも違う値**にしてある
/// ——取り違え（封の欄に実数を載せる・経路の取り違え）が値の差として出るように。
///
/// **実数の2つより大きい**（作業単位 B-60d-2）。`封 < 実` だとその世代に掛け金が掛かり、
/// 2ターン目から封をしなくなる（N4）。ここは `封 > 実`（多めに数える＝無害）の側に置く。
const SEALED_TOKENS: usize = 47;

/// 非ストリーミングの応答に載せる `usage.prompt_tokens`。
const COMPLETION_PROMPT_TOKENS: u32 = 41;

/// 中継の最終チャンクに載せる `usage.prompt_tokens`。**非ストリーミングと別の値。**
const STREAM_PROMPT_TOKENS: u32 = 43;

/// 偽の上流が返す本文。**両方の経路で同じ本文**にする（2ターン目の履歴を腕の間で揃えるため）。
const REPLY: &str = "「はい」";

/// 履歴を読む上限（`server::chat` の `HISTORY_LIMIT` より大きい値。台本の履歴はそれより少ない）。
const HISTORY_READ: u32 = 1_000;

/// 狭い予算（明示のコンテキスト長）。**バイト数の計画が履歴を削る**大きさ。
const TIGHT_CONTEXT: &str = "4096";
const TIGHT_CONTEXT_TOKENS: u32 = 4_096;

/// 履歴の1件の本文。**ASCII で、ちょうど `bytes` バイト**（番号ごとに別の綴り）。
fn history_text(index: usize, bytes: usize) -> String {
    let head = format!("{index:04}");
    format!("{head}{}", "a".repeat(bytes - head.len()))
}

/// 記憶の本文の頭（送った本文の中で記憶の行を数える目印）。
const MEMORY_HEAD: &str = "MEM";

/// 記憶1件の本文。**ASCII で、ちょうど `bytes` バイト**（番号ごとに別の綴り）。
fn memory_text(index: usize, bytes: usize) -> String {
    let head = format!("{MEMORY_HEAD}{index:04}");
    format!("{head}{}", "m".repeat(bytes - head.len()))
}

/// 送った本文（**末尾の利用者の発言**）に載った記憶の行の数。
///
/// A-15 から想起した記憶は system ではなく、最新の利用者の発言の前置きに載る（ADR-0061）。
/// このファイルの発言（`ただいま` 等）は目印 `MEM` を含まない。
fn memory_lines(contents: &[String]) -> usize {
    contents
        .last()
        .map_or(0, |latest| latest.matches(MEMORY_HEAD).count())
}

/// LM Studio が未知の口に返す形を模した本文（04-environment.md「200 ＋ エラー本文」）。
const LMSTUDIO_UNKNOWN_ENDPOINT: &str =
    r#"{"error":"Unexpected endpoint or method. (POST /apply-template)"}"#;

// ---------------------------------------------------------------------- 偽の上流

fn completion_body() -> String {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": "planted-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": REPLY},
            "finish_reason": "stop",
        }],
        "usage": {
            "prompt_tokens": COMPLETION_PROMPT_TOKENS,
            "completion_tokens": 5,
            "total_tokens": COMPLETION_PROMPT_TOKENS + 5,
        },
    })
    .to_string()
}

/// 中継の本文。**最終チャンクが使用量だけ**（`stream_options.include_usage` の形）。
fn sse_body() -> String {
    let delta = json!({
        "id": "chatcmpl-1",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "planted-model",
        "choices": [{"index": 0, "delta": {"content": REPLY}, "finish_reason": "stop"}],
    });
    let usage = json!({
        "id": "chatcmpl-1",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": "planted-model",
        "choices": [],
        "usage": {
            "prompt_tokens": STREAM_PROMPT_TOKENS,
            "completion_tokens": 5,
            "total_tokens": STREAM_PROMPT_TOKENS + 5,
        },
    });
    format!("data: {delta}\n\ndata: {usage}\n\ndata: [DONE]\n\n")
}

/// **チャットの口だけを持つ偽の上流**（非ストリーミングと中継の両方）。
async fn chat_only() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .and(matchers::body_partial_json(json!({"stream": false})))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(completion_body(), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .and(matchers::body_partial_json(json!({"stream": true})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse_body(), "text/event-stream"))
        .mount(&server)
        .await;
    server
}

/// `/apply-template` と `/tokenize` に答える応答。
fn template_answer() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(
        json!({ "prompt": PLANTED_TEMPLATE }).to_string(),
        "application/json",
    )
}

fn tokens_answer() -> ResponseTemplate {
    let tokens: Vec<usize> = (0..SEALED_TOKENS).collect();
    ResponseTemplate::new(200)
        .set_body_raw(json!({ "tokens": tokens }).to_string(), "application/json")
}

/// **llama.cpp を演じる偽の上流**（数える口を持つ）。
async fn countable() -> MockServer {
    let server = chat_only().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/apply-template"))
        .respond_with(template_answer())
        .mount(&server)
        .await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/tokenize"))
        .respond_with(tokens_answer())
        .mount(&server)
        .await;
    server
}

/// **LM Studio を演じる偽の上流**（数える口に 200 ＋ エラー本文）。
async fn lmstudio_like() -> MockServer {
    let server = chat_only().await;
    for wanted in ["/apply-template", "/tokenize"] {
        Mock::given(matchers::method("POST"))
            .and(matchers::path(wanted))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(LMSTUDIO_UNKNOWN_ENDPOINT, "application/json"),
            )
            .mount(&server)
            .await;
    }
    server
}

/// **封（全体）を数えた `/tokenize`**（届いた順）。部品の往復（`add_special: false`）は除く。
async fn whole_prompt_tokenizations(server: &MockServer) -> Vec<Value> {
    bodies_to(server, "/tokenize")
        .await
        .iter()
        .map(|body| serde_json::from_slice::<Value>(body).expect("本文が JSON であること"))
        .filter(|body| body["add_special"] == Value::Bool(true))
        .collect()
}

/// チャットへ届いた本文の `messages` の中身（届いた順に1ターンずつ）。
async fn chat_contents(server: &MockServer) -> Vec<Vec<String>> {
    bodies_to(server, "/v1/chat/completions")
        .await
        .iter()
        .map(|body| {
            let body: Value = serde_json::from_slice(body).expect("本文が JSON であること");
            body["messages"]
                .as_array()
                .expect("messages が配列であること")
                .iter()
                .map(|message| {
                    message["content"]
                        .as_str()
                        .expect("content が文字列であること")
                        .to_owned()
                })
                .collect()
        })
        .collect()
}

/// その口へ届いた本文（届いた順）。
async fn bodies_to(server: &MockServer, wanted: &str) -> Vec<Vec<u8>> {
    server
        .received_requests()
        .await
        .expect("記録が有効であること")
        .into_iter()
        .filter(|request| request.url.path() == wanted)
        .map(|request| request.body)
        .collect()
}

// ---------------------------------------------------------------------- サーバ

/// エフェメラルポートで起動した実サーバ。
struct TestServer {
    addr: SocketAddr,
    client: reqwest::Client,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    upstream: personacore::server::SharedUpstream,
    /// **走っているサーバと同じ DB**（履歴を置き、バイト数の計画を組み直すのに使う）。
    store: Store,
    _dir: TempDir,
}

impl TestServer {
    /// **本番の入口（`App::bind`）で起こす。** 抽出は別の上流（何も答えない）へ逃がす
    /// ——本文の上流に届く要求を本文の往復だけにするため。
    async fn bind(main: &MockServer, sub: &MockServer, idle_timeout: Option<&str>) -> Self {
        Self::bind_with_context(main, sub, idle_timeout, None).await
    }

    /// [`Self::bind`] に、**明示のコンテキスト長**（`PERSONACORE_CONTEXT_TOKENS`）を足した形。
    async fn bind_with_context(
        main: &MockServer,
        sub: &MockServer,
        idle_timeout: Option<&str>,
        context_tokens: Option<&str>,
    ) -> Self {
        for uri in [main.uri(), sub.uri()] {
            assert_ne!(
                uri.trim_end_matches('/'),
                DEFAULT_ENDPOINT,
                "実機（{DEFAULT_ENDPOINT}）へ向けないこと（docs/05-development.md）"
            );
        }
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let main_uri = main.uri();
        let sub_uri = sub.uri();
        let idle = idle_timeout.map(str::to_owned);
        let context = context_tokens.map(str::to_owned);
        let config = AppConfig::from_lookup(|key| match key {
            ENV_LLM_ENDPOINT => Some(main_uri.clone()),
            ENV_SUB_LLM_ENDPOINT => Some(sub_uri.clone()),
            ENV_LLM_IDLE_TIMEOUT => idle.clone(),
            ENV_CONTEXT_TOKENS => context.clone(),
            _ => None,
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(dir.path().join("personacore.sqlite"));

        // **`bind_with_client` ではない。** 本番と同じ入口であることが N7 の要点。
        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        Self::start(app, dir).await
    }

    /// **注入の入口で起こす**（写しの見張り番が使う）。
    async fn inject(llm: Arc<dyn LlmClient>) -> Self {
        Self::inject_with(llm, &[]).await
    }

    /// **注入の入口で、設定を足して起こす**（N1・N3・共有の差し引きの見張り番）。
    ///
    /// 抽出の上流は注入の入口が `SilentUpstream` に据える（本文のモックへは行かない）。
    /// **`ENV_SUB_LLM_ENDPOINT` を渡すと、観測の上では本文と裏方が別の上流になる**
    /// （共有の判別はその宛先で行う / `server::counted_plan`）。
    async fn inject_with(llm: Arc<dyn LlmClient>, settings: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let settings: Vec<(String, String)> = settings
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        let config = AppConfig::from_lookup(|key| {
            settings
                .iter()
                .find(|(wanted, _)| wanted == key)
                .map(|(_, value)| value.clone())
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(dir.path().join("personacore.sqlite"));
        let app = App::bind_with_client(&config, llm)
            .await
            .expect("待ち受けを開始できること");
        Self::start(app, dir).await
    }

    async fn start(app: App, dir: TempDir) -> Self {
        let addr = app.local_addr();
        let upstream = app.upstream_handle();
        let store = app.store().clone();
        app.store()
            .create_character(
                &Handle::parse(HANDLE).expect("正当な名前であること"),
                &DisplayName::parse("葵").expect("正当な表示名であること"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
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
            client: reqwest::Client::new(),
            shutdown: Some(shutdown),
            handle: Some(handle),
            upstream,
            store,
            _dir: dir,
        }
    }

    /// **会話の履歴を直に置く**（本文処理が書くのと同じ `append_turn`）。
    ///
    /// 利用者の発言と応答の本文は、[`history_text`] で番号ごとに別の綴りにする
    /// （同じ綴りは1つの部品にまとまり、数える往復の数が読めなくなる）。
    async fn seed_history(&self, turns: usize, bytes: usize) -> SessionId {
        let character = self
            .store
            .find_character_by_handle(&Handle::parse(HANDLE).expect("正当な名前であること"))
            .await
            .expect("キャラクターを読めること")
            .expect("キャラクターがあること");
        let session = self
            .store
            .create_session(character.id)
            .await
            .expect("セッションを作れること");
        for turn in 0..turns {
            let message = |role: Role, index: usize| NewMessage {
                role,
                dialogue: None,
                narration: None,
                raw_content: history_text(index, bytes),
            };
            self.store
                .append_turn(
                    session,
                    &message(Role::User, turn * 2),
                    &message(Role::Assistant, turn * 2 + 1),
                )
                .await
                .expect("往復を書けること");
        }
        session
    }

    /// **いまの DB から、バイト数の計画を組む**（関数の外の参照値 / N2・N3）。中身の列を返す。
    async fn byte_plan(
        &self,
        session: SessionId,
        user_message: &str,
        context_tokens: u32,
    ) -> Vec<String> {
        self.byte_plan_built(session, user_message, context_tokens)
            .await
            .messages
            .into_iter()
            .map(|message| message.content)
            .collect()
    }

    /// **いまの DB から、バイト数の計画を組む**（組み上がりそのもの）。
    ///
    /// 本文処理と同じ `build_prompt` に、本文処理と同じ入口で DB から読んだ入力と、
    /// **何も数えていない表**と明示の予算を渡す（ADR-0052 §決定1 の (i)。いまのバイト数の計画そのもの）。
    /// **ターンを送る前に呼ぶこと**（送ると、その往復が履歴に入る）。
    async fn byte_plan_built(
        &self,
        session: SessionId,
        user_message: &str,
        context_tokens: u32,
    ) -> personacore::prompt::BuiltPrompt {
        self.try_byte_plan(session, user_message, context_tokens)
            .await
            .expect("バイト数の計画が組めること")
    }

    /// [`Self::byte_plan_built`] の、組めない回も返す形（`BudgetExhausted` の見張り番）。
    async fn try_byte_plan(
        &self,
        session: SessionId,
        user_message: &str,
        context_tokens: u32,
    ) -> Result<personacore::prompt::BuiltPrompt, personacore::prompt::PromptError> {
        let character = self.character().await;
        let history = self
            .store
            .recent_messages(session, HISTORY_READ)
            .await
            .expect("履歴を読めること");
        let stats = self
            .store
            .stat_allocs(character.id)
            .await
            .expect("性格を読めること");
        let attributes = self
            .store
            .character_attributes(character.id)
            .await
            .expect("属性を読めること");
        let agreements = self
            .store
            .active_agreements(character.id)
            .await
            .expect("取り決めを読めること");
        let memories = self
            .store
            .recall_memories(character.id)
            .await
            .expect("記憶を読めること");
        build_prompt(
            &PromptInput {
                character: &character,
                stats: &stats,
                attributes: &attributes,
                agreements: &agreements,
                memories: &memories,
                history: &history,
                user_message,
            },
            TokenBudget::new(context_tokens, TokenBudget::DEFAULT_RESPONSE_TOKENS)
                .expect("予算になる値であること"),
            &CountedParts::default(),
        )
    }

    async fn character(&self) -> personacore::domain::Character {
        self.store
            .find_character_by_handle(&Handle::parse(HANDLE).expect("正当な名前であること"))
            .await
            .expect("キャラクターを読めること")
            .expect("キャラクターがあること")
    }

    /// **`handle` のキャラクターに性格の軸を置く**（`values` の値で1本ずつ。変容の応答の上限が軸の本数で決まる）。
    ///
    /// **値 0 の軸も置ける**（変容の依頼には値 0 の軸も載る / ADR-0027 §決定4。数え落とす変異を見分けるため）。
    async fn seed_axes_on(&self, handle: &str, values: &[i64]) {
        const AXES: [&str; 5] = ["勇気", "慎重", "陽気", "几帳面", "好奇心"];
        assert!(values.len() <= AXES.len(), "前提: 軸の名前が足りること");
        let character = self
            .store
            .find_character_by_handle(&Handle::parse(handle).expect("正当な名前であること"))
            .await
            .expect("キャラクターを読めること")
            .expect("キャラクターがあること");
        for (axis, value) in AXES.iter().zip(values) {
            common::insert_stat_alloc_directly(&self.store, character.id.get(), axis, *value).await;
        }
    }

    /// **もう1人のキャラクターを置く**（裏方の枠はキャラクターをまたいで1つ —— 差し引く量の見張り番）。
    async fn add_character(&self, handle: &str) {
        self.store
            .create_character(
                &Handle::parse(handle).expect("正当な名前であること"),
                &DisplayName::parse("別人").expect("正当な表示名であること"),
                &Persona::parse("別のキャラクター。").expect("正当なペルソナであること"),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");
    }

    /// **利用者についての記憶を `count` 件置く**（ASCII で、ちょうど `bytes` バイト。番号ごとに別の綴り）。
    async fn seed_memories(&self, count: usize, bytes: usize) {
        let character = self.character().await;
        for index in 0..count {
            self.store
                .add_memory(
                    character.id,
                    &personacore::domain::NewMemory {
                        subject: personacore::domain::Subject::User,
                        content: personacore::domain::MemoryContent::parse(&memory_text(
                            index, bytes,
                        ))
                        .expect("正当な記憶であること"),
                        category: None,
                    },
                )
                .await
                .expect("記憶を書けること");
        }
    }

    /// 1ターン送って、状態コードと本文を返す（**本文は読み切る**——読み切らないと中継が終わらない）。
    async fn turn(&self, message: &str, streaming: bool) -> (u16, String) {
        let response = self
            .client
            .post(format!("http://{}/v1/chat/completions", self.addr))
            .json(&json!({
                "model": HANDLE,
                "messages": [{"role": "user", "content": message}],
                "stream": streaming,
            }))
            .send()
            .await
            .expect("サーバへ接続できること");
        let status = response.status().as_u16();
        let body = response.text().await.expect("本文を読めること");
        (status, body)
    }

    /// 成功するはずの1ターン。
    async fn ok_turn(&self, message: &str, streaming: bool) -> String {
        let (status, body) = self.turn(message, streaming).await;
        assert_eq!(status, 200, "ターンが成功すること: {body}");
        if streaming {
            assert!(body.contains("[DONE]"), "中継が完結すること: {body}");
        }
        body
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

// ---------------------------------------------------------------------- ログ

/// **このバイナリのテストを1本ずつ通す回転扉**（`tests/server_chat_parse_log.rs` と同じ仕掛け）。
static ONE_TEST_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 1本のテストが出したログの区間。
struct LogSection {
    _pass: tokio::sync::MutexGuard<'static, ()>,
    buffer: &'static SharedBuffer,
}

impl LogSection {
    async fn enter() -> Self {
        let pass = ONE_TEST_AT_A_TIME.lock().await;
        Self {
            _pass: pass,
            buffer: install_global_log(personacore::server::DEFAULT_LOG_FILTER),
        }
    }

    /// いまの位置。**1ターンごとに控え、その後ろの行だけを読む。**
    fn mark(&self) -> usize {
        self.buffer.mark()
    }

    /// `mark` より後に出た**封の行**（出た順）。
    fn seal_lines_since(&self, mark: usize) -> Vec<String> {
        self.buffer
            .contents_since(mark)
            .lines()
            .filter(|line| line.contains(SEAL_MARKER))
            .map(str::to_owned)
            .collect()
    }

    /// `mark` より後に出た封の行が**ちょうど1本**であることを確かめて返す。
    fn one_seal_line_since(&self, mark: usize) -> String {
        let lines = self.seal_lines_since(mark);
        assert_eq!(
            lines.len(),
            1,
            "1ターンにつき封の行はちょうど1本であること: {lines:#?}"
        );
        lines.into_iter().next().unwrap_or_default()
    }
}

/// 行の `seal_ms=` の値（封の往復の経過ミリ秒）。**整数で載っていなければ落とす。**
fn seal_ms(line: &str) -> u64 {
    line.split_whitespace()
        .find_map(|token| token.strip_prefix("seal_ms="))
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("封の往復の時間を整数で同じ行に載せること: {line:?}"))
}

// ---------------------------------------------------------------------- N7

/// **N7: 本番の入口から `dyn` を通って、封の数がログの行に出る**（ADR-0052 §新しく要る見張り番 N7）。
///
/// **偽の上流は数える口を持つ**（`/apply-template` ＋ `/tokenize`）。非ストリーミングと中継の
/// 2ターンを回し、**どちらの行にも封の数・実数・計画の和・数えられたかが同じ1行に載る**ことを見る。
///
/// **実数は経路ごとに違う値**（[`COMPLETION_PROMPT_TOKENS`] / [`STREAM_PROMPT_TOKENS`]）なので、
/// 経路を取り違えた `usage` の読み方（中継で非ストリーミングの値を読む等）は値の差で赤になる。
///
/// # 封の本文は、送った本文そのもの
///
/// `/apply-template` へ届いた本文と `/v1/chat/completions` へ届いた本文を**バイト列で**比べる
/// （ADR-0052 §推奨3「本文を別に組まない」の、本番の入口からの確認。`llm` 側の N6 と対）。
#[tokio::test]
async fn counting_survives_the_face_client() {
    let log = LogSection::enter().await;
    let main = countable().await;
    let sub = MockServer::start().await;
    let server = TestServer::bind(&main, &sub, None).await;

    for (index, (streaming, actual)) in [
        (false, COMPLETION_PROMPT_TOKENS),
        (true, STREAM_PROMPT_TOKENS),
    ]
    .into_iter()
    .enumerate()
    {
        let mark = log.mark();
        server.ok_turn("ただいま", streaming).await;
        let line = log.one_seal_line_since(mark);

        assert!(
            has_field(&line, "seal=counted"),
            "数えた回であること（streaming={streaming}）: {line:?}"
        );
        assert!(
            has_field(&line, &format!("sealed_tokens=Some({SEALED_TOKENS})")),
            "封の数を載せること（streaming={streaming}）: {line:?}"
        );
        assert!(
            has_field(&line, &format!("usage_prompt_tokens=Some({actual})")),
            "上流が数えた実数を同じ行に載せること（streaming={streaming}）: {line:?}"
        );
        assert!(
            has_key(&line, "planned_tokens"),
            "計画の和を同じ行に載せること（streaming={streaming}）: {line:?}"
        );
        // **封の往復の時間**（ADR-0052 M2 / 作業単位 B-60b のレビュー）。値は機の速さで揺れるので
        // ここでは「同じ行に整数で載る」だけを見る（値の意味は `a_count_that_failed_is_not_remembered`）。
        seal_ms(&line);
        assert!(
            has_field(&line, &format!("streaming={streaming}")),
            "どちらの経路の行かを名乗ること: {line:?}"
        );
        assert!(
            line.contains("INFO"),
            "既定のログ設定で残ること（測定の材料）: {line:?}"
        );

        let chat = bodies_to(&main, "/v1/chat/completions").await;
        let sealed = bodies_to(&main, "/apply-template").await;
        assert_eq!(
            chat.len(),
            index + 1,
            "チャットへは1ターンに1回だけ送ること"
        );
        assert_eq!(sealed.len(), index + 1, "封は1ターンに1回だけ聞くこと");
        assert_eq!(
            String::from_utf8_lossy(&sealed[index]),
            String::from_utf8_lossy(&chat[index]),
            "封の本文が、送った本文と違う（streaming={streaming}）"
        );
        // **封の `/tokenize` だけを選ぶ**（作業単位 B-60d-1）。部品も同じ口で数える
        // （`add_special: false`）ので、届いた順の添字では封の往復を引けない。
        // 全体は `add_special: true`（`llm::lmstudio` の `TokenizeWhat`）。
        let wholes = whole_prompt_tokenizations(&main).await;
        assert_eq!(
            wholes.len(),
            index + 1,
            "封の数える往復は1ターンに1回だけであること"
        );
        assert_eq!(
            wholes[index]["content"], PLANTED_TEMPLATE,
            "組み上げが返した文字列を数えること"
        );
    }

    server.stop().await;
}

/// **N7 の後半: 数えられない上流は、面の世代ごとに1回だけ聞く**（ADR-0052 §推奨4）。
///
/// 偽の上流は LM Studio を演じる（数える口に 200 ＋ エラー本文）。
///
/// | ターン | 期待 |
/// |---|---|
/// | 1 | 聞いて「数えられない」（`seal=uncountable`） |
/// | 2・3（中継を含む） | **聞きに行かない**（`seal=remembered`）。`/apply-template` は 1 回のまま |
/// | 引き直しで答えたモデルが変わった後の 4 | **聞き直す**（世代が変わった） |
///
/// **4 の腕が無いと、「一度数えられなければ二度と聞かない」実装が緑になる**
/// （LM Studio から llama へ差し替えた利用者が、再起動するまで窓の2割に留まる）。
///
/// # 1ターン目を、非ストリーミングと中継の両方で回す（作業単位 B-60b のレビュー）
///
/// 覚えるのは `server::chat` の**2つの枝**（`complete` の後と `stream` の後）で別々に行う。
/// **1ターン目が片方の経路だけだと、もう片方の枝の「覚える」を消す変異が緑を通る。**
/// FE はほぼ中継で送るので、中継の枝が壊れると LM Studio の利用者が毎ターン往復を1回無駄にする。
#[tokio::test]
async fn an_upstream_that_cannot_count_is_asked_once_per_generation() {
    let log = LogSection::enter().await;

    for first_streaming in [false, true] {
        let main = lmstudio_like().await;
        let sub = MockServer::start().await;
        let server = TestServer::bind(&main, &sub, None).await;

        let mark = log.mark();
        server.ok_turn("ただいま", first_streaming).await;
        let first = log.one_seal_line_since(mark);
        assert!(
            has_field(&first, "seal=uncountable") && has_field(&first, "sealed_tokens=None"),
            "1ターン目は聞いて「数えられない」になること（streaming={first_streaming}）: {first:?}"
        );
        let actual = if first_streaming {
            STREAM_PROMPT_TOKENS
        } else {
            COMPLETION_PROMPT_TOKENS
        };
        assert!(
            has_field(&first, &format!("usage_prompt_tokens=Some({actual})")),
            "数えられない回も実数は載せること: {first:?}"
        );
        assert_eq!(bodies_to(&main, "/apply-template").await.len(), 1);

        for streaming in [false, true] {
            let mark = log.mark();
            server.ok_turn("もう一度", streaming).await;
            let line = log.one_seal_line_since(mark);
            assert!(
                has_field(&line, "seal=remembered") && has_field(&line, "sealed_tokens=None"),
                "同じ世代の間は聞きに行かないこと\
                 （1ターン目 streaming={first_streaming} / いま streaming={streaming}）: {line:?}"
            );
            assert!(
                has_field(&line, "seal_ms=0"),
                "聞かなかった回の封の時間は 0 であること: {line:?}"
            );
        }
        assert_eq!(
            bodies_to(&main, "/apply-template").await.len(),
            1,
            "覚えた世代の間は、もう数えに行かないこと（1ターン目 streaming={first_streaming}）"
        );
        // **部品を数える口へは、世代ごとに1回だけ聞く**（作業単位 B-60d-1）。
        // 最初の部品を1つだけ先に聞き、「数えられない」と言われたら残りの部品へは行かない
        // （`server::counted_plan` の doc）。**封の数える口（全体）までは進まない。**
        assert_eq!(
            bodies_to(&main, "/tokenize").await.len(),
            1,
            "部品の最初の1つで「数えられない」と分かったら、残りへは聞きに行かないこと\
             （1ターン目 streaming={first_streaming}）"
        );
        assert!(
            whole_prompt_tokenizations(&main).await.is_empty(),
            "前提: 組み上げが読めない上流では、封の数える口まで進まないこと"
        );

        // **引き直しで答えたモデルが変わった**（ADR-0052 §推奨4「面の世代」/ ADR-0020）。
        server.upstream.refresh_for_tests(
            ModelFace::Main,
            None,
            None,
            None,
            Some("another-model.gguf".to_owned()),
        );
        let mark = log.mark();
        server.ok_turn("載せ替えた？", false).await;
        let after = log.one_seal_line_since(mark);
        assert!(
            has_field(&after, "seal=uncountable"),
            "世代が変わったら聞き直すこと: {after:?}"
        );
        assert_eq!(
            bodies_to(&main, "/apply-template").await.len(),
            2,
            "世代が変わったら聞き直すこと"
        );

        server.stop().await;
    }
}

/// **数えられない上流には、バイト数の計画そのものを送る**（ADR-0052 §決定4 / N2 の結合側 / 作業単位 B-60d-1）。
///
/// B-60b の `the_body_sent_does_not_change_whether_or_not_the_upstream_counted`（観測だけ —— 数えても
/// 送る本文は変わらない）を**置き換えた**。B-60d-1 から、数えた上流には数えた計画を送るので、
/// あの主張はもう成り立たない。**数えられない上流の側だけ**が「今と同じ」を約束する。
///
/// | 腕 | 数える口 | 期待 |
/// |---|---|---|
/// | 404 | 無い（`wiremock` の既定の 404） | **バイト数の計画とバイト単位で同じ** |
/// | 200 ＋ エラー本文 | LM Studio の形 | 同上 |
/// | 数える（対照） | 部品も封も 47 | **バイト数の計画より多く載せる**（比べ方が数えたかどうかを見分けられる前提） |
///
/// **参照値は関数の外で組む**（[`TestServer::byte_plan`] —— 本文処理と同じ `build_prompt` に、
/// 何も数えていない表を渡す）。予算は**明示で狭く**して履歴を削らせる
/// —— 削らない予算では、どの値で数えても同じ本文になり、「数えられないときに 0 を使う」変異を見分けられない。
#[tokio::test]
async fn an_upstream_that_cannot_count_is_sent_exactly_the_byte_plan() {
    let log = LogSection::enter().await;

    for (arm, main, counts) in [
        ("404", chat_only().await, false),
        ("200 ＋ エラー本文", lmstudio_like().await, false),
        ("数える（対照）", countable().await, true),
    ] {
        let sub = MockServer::start().await;
        let server = TestServer::bind_with_context(&main, &sub, None, Some(TIGHT_CONTEXT)).await;
        let session = server.seed_history(10, 400).await;
        let expected = server
            .byte_plan(session, "ただいま", TIGHT_CONTEXT_TOKENS)
            .await;

        let mark = log.mark();
        server.ok_turn("ただいま", false).await;
        let line = log.one_seal_line_since(mark);
        let sent = chat_contents(&main).await;
        assert_eq!(sent.len(), 1, "前提: 1ターンぶん届いていること（{arm}）");
        assert!(
            expected.len() < 2 + 20,
            "前提: バイト数の計画が履歴を削っていること（{arm}）: {} 件",
            expected.len()
        );

        if counts {
            assert!(
                sent[0].len() > expected.len(),
                "対照: 数えた上流には、バイト数の計画より多く載せること: {} / {} 件",
                sent[0].len(),
                expected.len()
            );
            assert!(has_field(&line, "plan=counted"), "{line:?}");
        } else {
            assert_eq!(
                sent[0], expected,
                "数えられない上流（{arm}）には、バイト数の計画そのものを送ること"
            );
            assert!(has_field(&line, "plan=bytes"), "{arm}: {line:?}");
        }

        server.stop().await;
    }
}

/// **数える往復そのものが失敗した回は覚えない**（ADR-0052 §推奨4 / `LlmClient::count_request` の表）。
///
/// 数える往復の上限は照会の上限（`context_timeout`）で、天井（`PERSONACORE_LLM_IDLE_TIMEOUT`）の
/// 半分から導かれる（2 秒 → 1 秒）。`/apply-template` はそれより遅く答えるので、
/// **封の往復は時間切れになり、バイト数の計画を封をせずに送る**（バイト数の計画は上界で示せる /
/// ADR-0052 §決定1。部品は数えられるので、数えた計画は組むが送らない —— `server::counted_plan`）。
///
/// **2ターン目も聞きに行くこと**が主張の本体 —— 時間切れは「今回は聞けなかった」であって、
/// 「数えられない上流だと分かった」ではない。覚えると、混んでいた1回のせいで
/// その世代の間ずっと数えなくなる。
#[tokio::test]
async fn a_count_that_failed_is_not_remembered() {
    let log = LogSection::enter().await;
    let main = chat_only().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/apply-template"))
        .respond_with(template_answer().set_delay(Duration::from_secs(4)))
        .mount(&main)
        .await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/tokenize"))
        .respond_with(tokens_answer())
        .mount(&main)
        .await;
    let sub = MockServer::start().await;
    let server = TestServer::bind(&main, &sub, Some("2")).await;

    for (turn, streaming) in [false, true].into_iter().enumerate() {
        let mark = log.mark();
        server.ok_turn("ただいま", streaming).await;
        let line = log.one_seal_line_since(mark);
        assert!(
            has_field(&line, "seal=failed") && has_field(&line, "sealed_tokens=None"),
            "時間切れは「聞けなかった」であること（turn={turn}）: {line:?}"
        );
        assert!(
            has_field(&line, "plan=bytes"),
            "封で示せなかった回はバイト数の計画を送ること（turn={turn}）: {line:?}"
        );
        // **遅れた量が `seal_ms` に出る**（照会の上限は 1 秒）。上限まで待って切ったことが
        // 行から読める（ADR-0052 M2）。上は緩く取る（機が飽和しても赤にしない）。
        let waited = seal_ms(&line);
        assert!(
            waited >= 900,
            "封の往復の時間（上限まで待った分）を載せること（turn={turn}）: {waited} / {line:?}"
        );
        assert_eq!(
            bodies_to(&main, "/apply-template").await.len(),
            turn + 1,
            "聞けなかった回を覚えず、次のターンも聞きに行くこと"
        );
    }
    assert_eq!(
        bodies_to(&main, "/v1/chat/completions").await.len(),
        2,
        "封が時間切れでも本文は送ること"
    );

    server.stop().await;
}

/// **本文を受け付けなかった上流の「数えられない」は覚えない**（ADR-0052 §推奨4 / `server::seal` の doc）。
///
/// 1ターン目は**チャットも 503**（読み込み中）。数える口の答え方を2通り回す:
///
/// | 数える口の答え | 封の結果 | 守っているもの |
/// |---|---|---|
/// | **503** | `failed`（5xx は `Err` / `llm` の `post_for_count`） | 5xx を「数えられない」と取り違えない |
/// | **200 ＋ エラー本文**（形が読めない） | `uncountable` | **覚えるのは本文を受け付けた後**（`accepted` の位置） |
///
/// どちらの腕も、**2ターン目は載り終えた上流に聞き直して数える**（`seal=counted`）。
/// **1ターン目は本文が失敗するので封の行が出ない**——表の「封の結果」は実装から読んだもので、
/// この試験は1ターン目の結果そのものを観測していない（観測しているのは「覚えなかった」こと）。
///
/// # 1ターン目を、非ストリーミングと中継の両方で回す（作業単位 B-60b のレビュー）
///
/// **中継の枝で「覚える」を `stream()` の前へ移す変異**は、2つ目の腕の中継でしか赤にならない
/// （`stream()` が 503 で失敗した後でも、覚えてしまっている）。
#[tokio::test]
async fn a_count_refused_while_loading_is_not_remembered() {
    let log = LogSection::enter().await;

    for first_streaming in [false, true] {
        for (count_refusal, arm) in [
            (
                ResponseTemplate::new(503)
                    .set_body_raw(r#"{"error":"Loading model"}"#, "application/json"),
                "数える口も 503",
            ),
            (
                ResponseTemplate::new(200)
                    .set_body_raw(LMSTUDIO_UNKNOWN_ENDPOINT, "application/json"),
                "数える口は 200 ＋ エラー本文",
            ),
        ] {
            let main = MockServer::start().await;
            Mock::given(matchers::method("POST"))
                .and(matchers::path("/apply-template"))
                .respond_with(count_refusal)
                .up_to_n_times(1)
                .with_priority(1)
                .mount(&main)
                .await;
            Mock::given(matchers::method("POST"))
                .and(matchers::path("/v1/chat/completions"))
                .respond_with(
                    ResponseTemplate::new(503)
                        .set_body_raw(r#"{"error":"Loading model"}"#, "application/json"),
                )
                .up_to_n_times(1)
                .with_priority(1)
                .mount(&main)
                .await;
            Mock::given(matchers::method("POST"))
                .and(matchers::path("/v1/chat/completions"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_raw(completion_body(), "application/json"),
                )
                .mount(&main)
                .await;
            Mock::given(matchers::method("POST"))
                .and(matchers::path("/apply-template"))
                .respond_with(template_answer())
                .mount(&main)
                .await;
            Mock::given(matchers::method("POST"))
                .and(matchers::path("/tokenize"))
                .respond_with(tokens_answer())
                .mount(&main)
                .await;
            let sub = MockServer::start().await;
            let server = TestServer::bind(&main, &sub, None).await;

            let (status, body) = server.turn("ただいま", first_streaming).await;
            assert!(
                status >= 500,
                "前提: 読み込み中のターンは上流のエラーで返ること\
                 （streaming={first_streaming} / {arm}）: {status} {body}"
            );

            let mark = log.mark();
            server.ok_turn("載った？", false).await;
            let line = log.one_seal_line_since(mark);
            assert!(
                has_field(&line, "seal=counted"),
                "受け付けなかった回を覚えず、載った後に数えること\
                 （1ターン目 streaming={first_streaming} / {arm}）: {line:?}"
            );
            assert_eq!(
                bodies_to(&main, "/apply-template").await.len(),
                2,
                "1ターン目 streaming={first_streaming} / {arm}"
            );

            server.stop().await;
        }
    }
}

// ---------------------------------------------------------------------- 写しの固定

/// どの写しが何をしたか。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Count(u32),
    Complete(u32),
    Stream(u32),
}

/// **写しを取るたびに別の写しを返す上流**（走ったままの差し替えが毎回起きる状態を演じる）。
///
/// 自分自身（差し替え口）へ届いた往復は `u32::MAX` の写しとして数える。
struct Swapping {
    next: Mutex<u32>,
    calls: Arc<Mutex<Vec<Call>>>,
}

/// 写し1つ。**数えた数は自分の番号**（どの写しが数えたかが行からも読める）。
struct PinnedCopy {
    id: u32,
    calls: Arc<Mutex<Vec<Call>>>,
}

/// 差し替え口そのものの番号。
const OUTER: u32 = u32::MAX;

impl PinnedCopy {
    fn record(&self, call: Call) {
        self.calls.lock().expect("ロックを取れること").push(call);
    }
}

impl LlmClient for PinnedCopy {
    fn pinned(&self) -> Option<Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        self.record(Call::Complete(self.id));
        Box::pin(async {
            Ok(Completion {
                content: REPLY.to_owned(),
                finish_reason: Some(FinishReason::Stop),
                // **`usage` を載せない**（作業単位 B-60d-2）。封の数は写しの番号（0 から）なので、実数を載せると
                // `封 < 実` で掛け金が掛かり、次のターン（中継の腕）が封をしなくなる。ここが見るのは写しの同一性だけ。
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        self.record(Call::Stream(self.id));
        Box::pin(async {
            let chunk = ChatChunk {
                delta: REPLY.to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            };
            Ok(Box::pin(futures_util::stream::iter([Ok(chunk)])) as ChatStream)
        })
    }

    fn count_request<'a>(
        &'a self,
        _request: &'a ChatRequest,
        _streaming: bool,
    ) -> LlmFuture<'a, TokenCount> {
        self.record(Call::Count(self.id));
        let id = self.id;
        Box::pin(async move { Ok(TokenCount::Counted(id)) })
    }
}

impl LlmClient for Swapping {
    fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        let copy = self.outer();
        Box::pin(async move { copy.complete(request).await })
    }

    fn stream<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        let copy = self.outer();
        Box::pin(async move { copy.stream(request).await })
    }

    fn count_request<'a>(
        &'a self,
        request: &'a ChatRequest,
        streaming: bool,
    ) -> LlmFuture<'a, TokenCount> {
        let copy = self.outer();
        Box::pin(async move { copy.count_request(request, streaming).await })
    }

    fn pinned(&self) -> Option<Arc<dyn LlmClient>> {
        let mut next = self.next.lock().expect("ロックを取れること");
        let id = *next;
        *next += 1;
        Some(Arc::new(PinnedCopy {
            id,
            calls: Arc::clone(&self.calls),
        }))
    }
}

impl Swapping {
    /// 差し替え口そのものへ届いた往復の受け手。
    fn outer(&self) -> PinnedCopy {
        PinnedCopy {
            id: OUTER,
            calls: Arc::clone(&self.calls),
        }
    }
}

/// **数える写しと送る写しは同じ**（ADR-0052 §推奨3「封と送信は同じ上流の写しを通す」）。
///
/// 写しを取るたびに別の写しを返す上流（[`Swapping`]）を注入し、
/// **1ターンの数える往復と送る往復が、同じ番号の写しに届いた**ことを見る。
///
/// | 変異 | 見え方 |
/// |---|---|
/// | 写しを2回取る（数える用と送る用） | 番号が食い違う |
/// | 片方を差し替え口そのもので行う | 片方が [`OUTER`] になる |
///
/// **本物の差し替え口（`FaceClient`）が写しを返すこと**は `src/server/context.rs` の
/// `tests::a_pinned_copy_keeps_talking_to_the_upstream_it_was_taken_from` が見る
/// （ここは注入の入口なので、本物の差し替え口を通らない）。
#[tokio::test]
async fn the_seal_and_the_send_use_one_copy_of_the_upstream() {
    let log = LogSection::enter().await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let llm = Arc::new(Swapping {
        next: Mutex::new(0),
        calls: Arc::clone(&calls),
    });
    let server = TestServer::inject(llm).await;

    for streaming in [false, true] {
        calls.lock().expect("ロックを取れること").clear();
        let mark = log.mark();
        server.ok_turn("ただいま", streaming).await;
        let line = log.one_seal_line_since(mark);

        let seen = calls.lock().expect("ロックを取れること").clone();
        let [Call::Count(counted), sent] = seen.as_slice() else {
            panic!("1ターンに、数える往復1回と送る往復1回であること: {seen:?}");
        };
        let sent_by = match sent {
            Call::Complete(id) if !streaming => *id,
            Call::Stream(id) if streaming => *id,
            other => panic!("経路に合った往復であること（streaming={streaming}）: {other:?}"),
        };
        assert_ne!(
            *counted, OUTER,
            "差し替え口そのもので数えないこと: {seen:?}"
        );
        assert_eq!(
            *counted, sent_by,
            "数えた写しと送った写しが違う（streaming={streaming}）: {seen:?}"
        );
        assert!(
            has_field(&line, &format!("sealed_tokens=Some({counted})")),
            "封の行は、その写しが数えた数を載せること: {line:?}"
        );
    }

    server.stop().await;
}

// ---------------------------------------------------------------------- 封の強制（B-60d-1）

/// 数える上流への1往復の記録。**本文は中身（`content` の列）で持つ**（帰属は最終的な出力の中身で見る）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    /// 部品を数えた（`count_text`）。
    Part(String),
    /// 封をした（`count_request`）。数えた本文と、答えた数。
    Seal(Vec<String>, u32),
    /// 送った（`complete` / `stream`）。
    Send(Vec<String>),
}

/// 封の数え方（送る本文の中身 → 数）。
type SealRule = fn(&[String]) -> u32;

/// 1往復ぶんの記録の列（写しの番号つき）。
type Calls = Arc<Mutex<Vec<(u32, Seen)>>>;

/// **部品はバイト数の 1/4、全体は [`SealRule`] で数える上流**（ADR-0052 §新しく要る見張り番 N1）。
///
/// 写しを取るたびに別の番号の写しを返す（[`Swapping`] と同じ形）。**部品・封・送信が
/// 同じ写しに届いたか**を番号で見る（ADR-0052 §推奨3）。
struct Tokenizing {
    seal: SealRule,
    /// 応答に載せる `usage.prompt_tokens`（`None` なら載せない）。
    usage: Option<u32>,
    /// **送った本文から `usage.prompt_tokens` を作る規則**（N4。あれば [`Self::usage`] より優先）。
    usage_of: Option<SealRule>,
    /// **部品は数えられないと答える**（全体は数える —— バイト数の計画に封をする回を作る / N4 のバイト数の腕）。
    parts_uncountable: bool,
    next: Mutex<u32>,
    calls: Calls,
}

impl Tokenizing {
    fn new(seal: SealRule, calls: &Calls) -> Self {
        Self {
            seal,
            usage: None,
            usage_of: None,
            parts_uncountable: false,
            next: Mutex::new(0),
            calls: Arc::clone(calls),
        }
    }

    fn copy(&self, id: u32) -> TokenizingCopy {
        TokenizingCopy {
            id,
            seal: self.seal,
            usage: self.usage,
            usage_of: self.usage_of,
            parts_uncountable: self.parts_uncountable,
            calls: Arc::clone(&self.calls),
        }
    }
}

/// [`Tokenizing`] の写し1つ。
struct TokenizingCopy {
    id: u32,
    seal: SealRule,
    usage: Option<u32>,
    usage_of: Option<SealRule>,
    parts_uncountable: bool,
    calls: Calls,
}

impl TokenizingCopy {
    fn record(&self, seen: Seen) {
        self.calls
            .lock()
            .expect("ロックを取れること")
            .push((self.id, seen));
    }

    /// 送った本文に対する応答の `usage`。
    fn usage_for(&self, body: &[String]) -> Option<TokenUsage> {
        self.usage_of
            .map(|rule| rule(body))
            .or(self.usage)
            .map(|prompt_tokens| TokenUsage {
                prompt_tokens,
                completion_tokens: None,
            })
    }
}

/// 本文の中身。
fn contents(messages: &[ChatMessage]) -> Vec<String> {
    messages
        .iter()
        .map(|message| message.content.clone())
        .collect()
}

/// **部品1つの数**（バイト数の 1/4 を切り上げ）。
fn quarter(text: &str) -> u32 {
    u32::try_from(text.len().div_ceil(4)).expect("テストの文字列は短いこと")
}

/// **全体を部品と同じ規則で数えた数**（メッセージごとに部品の数 ＋ 1件の上乗せ）。
///
/// 記憶も取り決めも置かない台本では、**計画の和とちょうど同じ**になる
/// （`prompt::build_prompt` の値付けが、メッセージごとに `MESSAGE_OVERHEAD_TOKENS` を足すため）。
/// これに足した分が「境目の併合」で増えた分になる（N1 の [`n1_seal`]）。
fn whole_of(contents: &[String]) -> u32 {
    contents
        .iter()
        .map(|content| quarter(content) + MESSAGE_OVERHEAD_TOKENS)
        .sum()
}

impl LlmClient for TokenizingCopy {
    fn pinned(&self) -> Option<Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        let body = contents(&request.messages);
        let usage = self.usage_for(&body);
        self.record(Seen::Send(body));
        Box::pin(async move {
            Ok(Completion {
                content: REPLY.to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage,
            })
        })
    }

    fn stream<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        let body = contents(&request.messages);
        let usage = self.usage_for(&body);
        self.record(Seen::Send(body));
        Box::pin(async move {
            let chunk = ChatChunk {
                delta: REPLY.to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage,
            };
            Ok(Box::pin(futures_util::stream::iter([Ok(chunk)])) as ChatStream)
        })
    }

    fn count_request<'a>(
        &'a self,
        request: &'a ChatRequest,
        _streaming: bool,
    ) -> LlmFuture<'a, TokenCount> {
        let body = contents(&request.messages);
        let tokens = (self.seal)(&body);
        self.record(Seen::Seal(body, tokens));
        Box::pin(async move { Ok(TokenCount::Counted(tokens)) })
    }

    fn count_text<'a>(&'a self, text: &'a str) -> LlmFuture<'a, TokenCount> {
        self.record(Seen::Part(text.to_owned()));
        let answer = if self.parts_uncountable {
            TokenCount::Uncountable
        } else {
            TokenCount::Counted(quarter(text))
        };
        Box::pin(async move { Ok(answer) })
    }
}

impl LlmClient for Tokenizing {
    fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        let copy = self.copy(OUTER);
        Box::pin(async move { copy.complete(request).await })
    }

    fn stream<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        let copy = self.copy(OUTER);
        Box::pin(async move { copy.stream(request).await })
    }

    fn count_request<'a>(
        &'a self,
        request: &'a ChatRequest,
        streaming: bool,
    ) -> LlmFuture<'a, TokenCount> {
        let copy = self.copy(OUTER);
        Box::pin(async move { copy.count_request(request, streaming).await })
    }

    fn count_text<'a>(&'a self, text: &'a str) -> LlmFuture<'a, TokenCount> {
        let copy = self.copy(OUTER);
        Box::pin(async move { copy.count_text(text).await })
    }

    fn pinned(&self) -> Option<Arc<dyn LlmClient>> {
        let mut next = self.next.lock().expect("ロックを取れること");
        let id = *next;
        *next += 1;
        Some(Arc::new(self.copy(id)))
    }
}

/// 1ターンの記録を読み、**全部が同じ写しに届いた**ことを確かめてから返す。
fn one_copy(calls: &Calls) -> Vec<Seen> {
    let seen = calls.lock().expect("ロックを取れること").clone();
    let ids: std::collections::BTreeSet<u32> = seen.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        ids.len(),
        1,
        "部品・封・送信は1ターンに1つの写しで行うこと: {ids:?}"
    );
    assert!(
        !ids.contains(&OUTER),
        "差し替え口そのもので数えたり送ったりしないこと"
    );
    seen.into_iter().map(|(_, seen)| seen).collect()
}

fn seals_in(seen: &[Seen]) -> Vec<(Vec<String>, u32)> {
    seen.iter()
        .filter_map(|seen| match seen {
            Seen::Seal(body, tokens) => Some((body.clone(), *tokens)),
            Seen::Part(_) | Seen::Send(_) => None,
        })
        .collect()
}

fn sends_in(seen: &[Seen]) -> Vec<Vec<String>> {
    seen.iter()
        .filter_map(|seen| match seen {
            Seen::Send(body) => Some(body.clone()),
            Seen::Part(_) | Seen::Seal(..) => None,
        })
        .collect()
}

/// **観測の上で本文と別になる裏方の宛先**（どこにも繋がない。注入の入口は裏方に `SilentUpstream` を据える）。
const SEPARATE_SUB: &str = "http://127.0.0.1:9";

/// 狭い予算の取り分（**関数の外の値**: 4096 − 既定の応答の余白 1024）。
const TIGHT_ALLOWANCE: u32 = 4_096 - 1_024;

/// N1 の「境目の併合」で増える分（**全体は部品の和よりこれだけ多い**）。
const N1_MERGE: u32 = 300;

/// N1 の封: **部品の和 ＋ 併合の分**。
fn n1_seal(body: &[String]) -> u32 {
    whole_of(body) + N1_MERGE
}

/// **N1: 封をした回は、上流へ実際に送った本文を数えた数が取り分以下**（ADR-0052 §新しく要る見張り番 N1）。
///
/// 数え手は**全体を部品の和より [`N1_MERGE`] だけ多く数える**（境目の併合を模す）。
/// 履歴を取り分より多く置くので、1回目の計画はほぼ取り分を使い切り、**1回目の封は取り分を超える。**
/// 組み直しの取り分は「1回目の計画の和 − 超えた分」で、併合の分が一定なら2回目の封は収まる。
///
/// | 見ているもの | 止める変異 |
/// |---|---|
/// | 封は2回（組み直しは1回） | 組み直しを0回にする（1回）・2回にする（N3 の台本で3回になる） |
/// | **送った本文 = 2回目に封をした本文**（中身で突き合わせる） | **組み直す前の本文に封をして、組み直した本文を送る** |
/// | 送った本文を**テストが自分で数え直して**取り分以下 | 封の数を読み違える・取り分を取り違える |
/// | 部品・封・送信が同じ写し | 封と送信で別の上流の写しを使う |
/// | バイト数の計画より多く載せている | 黙ってバイト数の計画へ落ちる |
///
/// **非ストリーミングと中継の両方**で回す（分岐は上流を呼ぶ直前だけ / `server::chat` の doc）。
#[tokio::test]
async fn a_prompt_is_sent_only_after_the_upstream_counted_it_within_the_allowance() {
    let log = LogSection::enter().await;

    for streaming in [false, true] {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let server = TestServer::inject_with(
            Arc::new(Tokenizing::new(n1_seal, &calls)),
            &[
                (ENV_CONTEXT_TOKENS, TIGHT_CONTEXT),
                (ENV_SUB_LLM_ENDPOINT, SEPARATE_SUB),
            ],
        )
        .await;
        let session = server.seed_history(20, 400).await;
        let byte_plan = server
            .byte_plan(session, "ただいま", TIGHT_CONTEXT_TOKENS)
            .await;

        let mark = log.mark();
        server.ok_turn("ただいま", streaming).await;
        let line = log.one_seal_line_since(mark);
        let seen = one_copy(&calls);

        let seals = seals_in(&seen);
        assert_eq!(
            seals.len(),
            2,
            "超えた封のあと、1回だけ組み直して封をし直すこと（streaming={streaming}）"
        );
        assert!(
            seals[0].1 > TIGHT_ALLOWANCE,
            "前提: 1回目の封は取り分を超えること: {} / {TIGHT_ALLOWANCE}",
            seals[0].1
        );
        let sends = sends_in(&seen);
        assert_eq!(sends.len(), 1, "送るのは1回だけであること");
        assert_eq!(
            sends[0], seals[1].0,
            "送った本文は、最後に封をした本文そのものであること（streaming={streaming}）"
        );
        assert_ne!(
            sends[0], seals[0].0,
            "超えた本文ではなく、組み直した本文を送ること（streaming={streaming}）"
        );
        let recounted = n1_seal(&sends[0]);
        assert!(
            recounted <= TIGHT_ALLOWANCE,
            "送った本文を数え直すと取り分以下であること: {recounted} / {TIGHT_ALLOWANCE}"
        );
        assert!(
            sends[0].len() > byte_plan.len(),
            "前提: 数えた計画を送っていること（バイト数の計画より多く載る）: {} / {} 件",
            sends[0].len(),
            byte_plan.len()
        );
        for field in [
            "plan=counted".to_owned(),
            "rebuilds=1".to_owned(),
            format!("sealed_tokens=Some({recounted})"),
        ] {
            assert!(has_field(&line, &field), "{field} を載せること: {line:?}");
        }

        server.stop().await;
    }
}

/// N3 の封: **何を数えても取り分を1だけ超える。**
fn n3_seal(_body: &[String]) -> u32 {
    TIGHT_ALLOWANCE + 1
}

/// **N3: 組み直しても超えたら、バイト数の計画で送る**（ADR-0052 §新しく要る見張り番 N3）。
///
/// 封が2回とも超える数え手。**`BudgetExhausted` にしない**（200 で返る）こと、
/// **送った本文がバイト数の計画とバイト単位で同じ**こと（参照は [`TestServer::byte_plan`]）、
/// **封は2回で止まる**（組み直しは1回だけ）ことを見る。
#[tokio::test]
async fn a_seal_that_still_overflows_falls_back_to_the_byte_plan() {
    let log = LogSection::enter().await;

    for streaming in [false, true] {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let server = TestServer::inject_with(
            Arc::new(Tokenizing::new(n3_seal, &calls)),
            &[
                (ENV_CONTEXT_TOKENS, TIGHT_CONTEXT),
                (ENV_SUB_LLM_ENDPOINT, SEPARATE_SUB),
            ],
        )
        .await;
        let session = server.seed_history(20, 400).await;
        let byte_plan = server
            .byte_plan(session, "ただいま", TIGHT_CONTEXT_TOKENS)
            .await;

        let mark = log.mark();
        let (status, body) = server.turn("ただいま", streaming).await;
        assert_eq!(
            status, 200,
            "組み直しても超えたときに断らないこと（streaming={streaming}）: {body}"
        );
        let line = log.one_seal_line_since(mark);
        let seen = one_copy(&calls);

        let seals = seals_in(&seen);
        assert_eq!(
            seals.len(),
            2,
            "組み直しは1回だけであること（streaming={streaming}）"
        );
        assert_ne!(
            seals[0].0, seals[1].0,
            "前提: 2回目の封は組み直した本文を数えていること"
        );
        let sends = sends_in(&seen);
        assert_eq!(
            sends,
            vec![byte_plan],
            "組み直しても超えたら、バイト数の計画そのものを送ること（streaming={streaming}）"
        );
        for field in ["plan=bytes", "rebuilds=1", "sealed_tokens=None"] {
            assert!(has_field(&line, field), "{field} を載せること: {line:?}");
        }

        server.stop().await;
    }
}

/// 共有の見張り番の窓（明示のコンテキスト長）。**裏方の落とし所の窓（16384）と別の値**
/// （レビューの P4 —— 同じ値だと、本文の窓と裏方の窓を取り違えても数が合う）。
const SHARED_CONTEXT: &str = "20000";
const SHARED_CONTEXT_TOKENS: u32 = 20_000;

/// 別のキャラクターが持つ性格の軸の値（**5本。変容の応答の上限が抽出の上限を上回る本数**）。
/// **1本は値 0**（変容の依頼には載るので数える / 再レビュー 要修正3）。
const SHARED_AXIS_VALUES: [i64; 5] = [1, 2, 0, 1, 3];

/// 軸を持つ、もう1人のキャラクター。
const OTHER_HANDLE: &str = "bee";

/// **裏方の1要求分**（**関数の外の値**）: 注入の入口の裏方の窓は落とし所の 16384 で、
/// 依頼の上限はその半分（8192）。応答の上限は、抽出の 320 より**変容の全軸の形の 86 × 5 本 = 430** が大きい
/// （作業単位 B-60d-1 のレビュー 要修正2）。
const BACKGROUND_REQUEST: u32 = 16_384 / 2 + 86 * 5;

/// 差し引いた後の取り分（20000 − 裏方の1要求分 − 応答の余白 1024）。
const SHARED_ALLOWANCE: u32 = SHARED_CONTEXT_TOKENS - BACKGROUND_REQUEST - 1_024;

/// **本文と裏方が同じ上流なら、本文の窓から裏方の1要求分を差し引く**（ADR-0052 §決定7 / M4）。
///
/// | 腕 | 観測の上の宛先 | 期待 |
/// |---|---|---|
/// | 別の上流 | 本文と裏方が別（`ENV_SUB_LLM_ENDPOINT`） | **差し引かない**（取り分いっぱいまで使う） |
/// | 同じ上流 | 未設定（裏方は本文と同じ宛先） | **差し引く** —— 窓が要求の間で分け合われているかは判別できない（`server::counted_plan` の doc）ので、遅くなるだけの側へ倒す |
///
/// 送った本文を**テストが自分で数え直して**、差し引いた取り分（[`SHARED_ALLOWANCE`]）と比べる。
/// **どちらの腕も数えた計画であること**（バイト数の計画より多く載る）を前提として確かめる
/// —— 差し引けずにバイト数の計画へ落ちる形は、取り分の比較だけでは緑になりうる。
///
/// # 封は併合のある数え手（N1 と同じ）で回す（レビューの P1）
///
/// **1回目の封が、差し引いた取り分を超える**ので、組み直しが1回起きる（`rebuilds=1`）。
/// 封を**差し引く前の取り分**と比べる形・**差し引く前の予算**で狭める形は、ここで組み直しが起きないか、
/// 差し引いた取り分を超えた本文を送るので赤になる。
///
/// # 差し引く量は変容の上限で決まる（要修正2）
///
/// **軸を5本（うち1本は値 0）持つのは別のキャラクター**（[`OTHER_HANDLE`]）で、話す相手は軸を持たない
/// （再レビュー 要修正1・3）。差し引いた量（封の行の `shared_deduction`）を、抽出の 320 で出した値ではなく
/// 変容の 430（5本 × 86）で出した値（[`BACKGROUND_REQUEST`]）とリテラルで突き合わせる。
/// **話す相手の本数で数える形**（0本 → 320）と、**値 0 の軸を数え落とす形**（4本 → 344）は赤になる。
#[tokio::test]
async fn a_shared_upstream_leaves_room_for_one_background_request() {
    let log = LogSection::enter().await;

    for (arm, separate) in [("別の上流", true), ("同じ上流", false)] {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let mut settings = vec![(ENV_CONTEXT_TOKENS, SHARED_CONTEXT)];
        if separate {
            settings.push((ENV_SUB_LLM_ENDPOINT, SEPARATE_SUB));
        }
        let server =
            TestServer::inject_with(Arc::new(Tokenizing::new(n1_seal, &calls)), &settings).await;
        // **話す相手（`HANDLE`）は軸を持たない。軸を持つのは別のキャラクター**（再レビュー 要修正1）。
        // 本文と重なる裏方は直前のターンのもので、別のキャラクターの仕事でありうる。
        server.add_character(OTHER_HANDLE).await;
        server.seed_axes_on(OTHER_HANDLE, &SHARED_AXIS_VALUES).await;
        let session = server.seed_history(25, 2_000).await;
        let byte_plan = server
            .byte_plan(session, "ただいま", SHARED_CONTEXT_TOKENS)
            .await;

        let mark = log.mark();
        server.ok_turn("ただいま", false).await;
        let line = log.one_seal_line_since(mark);
        let seen = one_copy(&calls);
        let sends = sends_in(&seen);
        assert_eq!(sends.len(), 1, "{arm}");
        let sent = n1_seal(&sends[0]);
        assert!(
            sends[0].len() > byte_plan.len(),
            "前提: 数えた計画を送っていること（{arm}）: {} / {} 件",
            sends[0].len(),
            byte_plan.len()
        );
        assert!(
            has_field(&line, "plan=counted"),
            "数えた計画を送ること（{arm}）: {line:?}"
        );
        assert!(
            has_field(&line, &format!("sealed_tokens=Some({sent})")),
            "封の数は送った本文の数え直しと同じであること（{arm}）: {line:?}"
        );

        if separate {
            assert!(
                sent > SHARED_ALLOWANCE,
                "別の上流なら差し引かないこと: {sent} / {SHARED_ALLOWANCE}"
            );
            assert!(has_field(&line, "shared_deduction=0"), "{line:?}");
        } else {
            assert!(
                sent <= SHARED_ALLOWANCE,
                "同じ上流なら、差し引いた取り分に収まることを封で示してから送ること: {sent} / {SHARED_ALLOWANCE}"
            );
            assert!(
                has_field(&line, "rebuilds=1"),
                "1回目の封が差し引いた取り分を超え、1回だけ組み直すこと: {line:?}"
            );
            assert!(
                has_field(&line, &format!("shared_deduction={BACKGROUND_REQUEST}")),
                "差し引く量は、変容の応答の上限（430）で決まること: {line:?}"
            );
        }

        server.stop().await;
    }
}

/// **数えた計画の回でも、コンテキストの超過は警告する。上界の破れは言わない**
/// （作業単位 B-60d-1 のレビュー 要修正1 / ADR-0052 §推奨5 の表の4行目「今と同じ」）。
///
/// 上流は `usage.prompt_tokens` に 4000 を返す。取り分は 3072（4096 − 1024）で、
/// **4000 ＋ 生成上限 1024 は 4096 を超え**、**4000 は数えた計画の和を上回る**（バイト数の計画なら上界の破れ）。
/// 両方の判定が「鳴る」入力で、鳴るのは超過だけであることを見る。**非ストリーミングと中継の両方。**
///
/// | 止める変異 | 見え方 |
/// |---|---|
/// | 数えた計画の回は答え合わせを丸ごと飛ばす（B-60d-1 の1周目の形） | 超過の警告が出ない |
/// | 上界の判定の門を外す | 上界の破れが出る |
#[tokio::test]
async fn a_counted_plan_still_warns_when_the_usage_overruns_the_context() {
    let log = LogSection::enter().await;

    for streaming in [false, true] {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let mut llm = Tokenizing::new(whole_of, &calls);
        llm.usage = Some(4_000);
        let server = TestServer::inject_with(
            Arc::new(llm),
            &[
                (ENV_CONTEXT_TOKENS, TIGHT_CONTEXT),
                (ENV_SUB_LLM_ENDPOINT, SEPARATE_SUB),
            ],
        )
        .await;
        server.seed_history(3, 400).await;

        let mark = log.mark();
        server.ok_turn("ただいま", streaming).await;
        let line = log.one_seal_line_since(mark);
        assert!(
            has_field(&line, "plan=counted"),
            "前提: 数えた計画を送った回であること: {line:?}"
        );
        let output = log.buffer.contents_since(mark);
        let overrun = output
            .lines()
            .find(|line| line.contains("想定コンテキスト長を超え"))
            .unwrap_or_else(|| {
                panic!("数えた計画の回でも超過を警告すること（streaming={streaming}）: {output}")
            });
        assert!(
            overrun.contains("WARN") && has_field(overrun, "actual_tokens=4000"),
            "{overrun:?}"
        );
        assert!(
            !output.contains("上界が破れ"),
            "数えた計画の和は上界ではないので、上界の破れを言わないこと（streaming={streaming}）: {output}"
        );

        server.stop().await;
    }
}

// ---------------------------------------------------------------------- N4（作業単位 B-60d-2）

/// N4 の封の「併合の分」。**封は部品の和よりこれだけ多い。**
///
/// 記憶も取り決めも置かない台本では、計画の和は [`whole_of`] そのものなので、
/// **封の数と計画の和が別の値になる**（CLAUDE.md §4「フィクスチャで2つの値が同じだと、その取り違えは検出できない」）。
const N4_MERGE: u32 = 7;

/// N4 の封: **部品の和 ＋ [`N4_MERGE`]**。
fn n4_seal(body: &[String]) -> u32 {
    whole_of(body) + N4_MERGE
}

/// **封 < 実**: 実数は封より 5 多い（計画の和より 12 多い）。
fn n4_usage_above(body: &[String]) -> u32 {
    n4_seal(body) + 5
}

/// **封 = 実**（対照）。
fn n4_usage_equal(body: &[String]) -> u32 {
    n4_seal(body)
}

/// **封 > 実**: 実数は封より 3 少ない（計画の和よりは 4 多い —— 3つとも別の値）。
fn n4_usage_below(body: &[String]) -> u32 {
    n4_seal(body) - 3
}

/// 封の腕の `warn` の手掛かり（`server::seal` の `settle`）。
const UNDERCOUNTED_MARKER: &str = "封の数が上流の実数を下回りました";

/// 1ターンの観測（その区間のログ全体・封の行・上流に届いたもの）。
struct Observed {
    output: String,
    line: String,
    seen: Vec<Seen>,
}

/// **1ターン送って観測する。** 記録はターンの前に空にする（写しの番号はターンごとに違う）。
async fn observe(
    log: &LogSection,
    server: &TestServer,
    calls: &Calls,
    message: &str,
    streaming: bool,
) -> Observed {
    calls.lock().expect("ロックを取れること").clear();
    let mark = log.mark();
    server.ok_turn(message, streaming).await;
    let line = log.one_seal_line_since(mark);
    Observed {
        output: log.buffer.contents_since(mark),
        line,
        seen: one_copy(calls),
    }
}

/// N4 の台本で置く履歴の往復の数（1件 400 バイト）。
///
/// **バイト数の計画が履歴を削る大きさ**（10件 × 400 バイト ＞ 狭い予算の取り分 3072）にしてある。
/// 削らない大きさでは、バイト数の計画と数えた計画が同じ本文になり、「2ターン目はバイト数で送った」を
/// 送った本文で見分けられない。数えた計画（4 バイトで1つ）には全部が載る。
const N4_HISTORY_TURNS: usize = 5;

/// **N4 の上流**（封は [`n4_seal`]、実数は `usage_of`）で、狭い予算・別の裏方の宛先のサーバを起こす。
async fn n4_server(usage_of: SealRule, calls: &Calls) -> TestServer {
    n4_server_with(n4_seal, usage_of, false, calls).await
}

/// [`n4_server`] の、封の数え方と「部品は数えられない」を選べる形（組み直しの腕・バイト数の腕）。
async fn n4_server_with(
    seal: SealRule,
    usage_of: SealRule,
    parts_uncountable: bool,
    calls: &Calls,
) -> TestServer {
    let mut llm = Tokenizing::new(seal, calls);
    llm.usage_of = Some(usage_of);
    llm.parts_uncountable = parts_uncountable;
    TestServer::inject_with(
        Arc::new(llm),
        &[
            (ENV_CONTEXT_TOKENS, TIGHT_CONTEXT),
            (ENV_SUB_LLM_ENDPOINT, SEPARATE_SUB),
        ],
    )
    .await
}

/// **履歴が `messages` 件になるまで待つ**（前のターンの保存が終わってから、次のターンの参照値を組む）。
///
/// 待つ回数に上限を持つ（CLAUDE.md §4「番兵を待つループには反復回数の上限」）。
async fn settled_history(server: &TestServer, session: SessionId, messages: usize) {
    for _ in 0..500 {
        let now = server
            .store
            .recent_messages(session, HISTORY_READ)
            .await
            .expect("履歴を読めること")
            .len();
        if now >= messages {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("履歴が {messages} 件にならなかった（前のターンの保存が終わらない）");
}

/// 1ターン目（数えた計画）の観測から、**送った本文の封・実数・計画の和**を取り、封の行と突き合わせる。
fn first_turn_numbers(first: &Observed, usage_of: SealRule) -> (u32, u32, u32, Vec<String>) {
    first_turn_numbers_of(first, n4_seal, usage_of, "counted")
}

/// 行の `key=` の値を整数で読む。
fn field_u32(line: &str, key: &str) -> u32 {
    line.split_whitespace()
        .find_map(|token| token.strip_prefix(key))
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("{key} を整数で載せること: {line:?}"))
}

/// [`first_turn_numbers`] の、封の数え方と送った計画（`plan=`）を選べる形。
///
/// 計画の和は行から読み、**数えた計画なら部品の和（[`whole_of`]）と一致する**ことを確かめる
/// （バイト数の計画の和はバイト数なので、部品の和とは比べない）。
fn first_turn_numbers_of(
    first: &Observed,
    seal: SealRule,
    usage_of: SealRule,
    plan: &str,
) -> (u32, u32, u32, Vec<String>) {
    assert!(
        has_field(&first.line, &format!("plan={plan}")) && has_field(&first.line, "seal=counted"),
        "前提: 1ターン目は {plan} の計画に封をして送ったこと: {:?}",
        first.line
    );
    let sends = sends_in(&first.seen);
    assert_eq!(sends.len(), 1, "前提: 送るのは1回だけであること");
    let sent = sends.into_iter().next().unwrap_or_default();
    let (sealed, actual) = (seal(&sent), usage_of(&sent));
    let planned = field_u32(&first.line, "planned_tokens=");
    if plan == "counted" {
        assert_eq!(
            planned,
            whole_of(&sent),
            "前提: 数えた計画の和は部品の和であること"
        );
    }
    assert!(
        sealed != planned && actual != planned,
        "前提: 封・実数と計画の和が別の値であること: {sealed} / {actual} / {planned}"
    );
    for field in [
        format!("sealed_tokens=Some({sealed})"),
        format!("usage_prompt_tokens=Some({actual})"),
    ] {
        assert!(
            has_field(&first.line, &field),
            "前提: 封の行の値が関数の外の値と合うこと: {field} / {:?}",
            first.line
        );
    }
    (sealed, actual, planned, sent)
}

/// N4 の組み直しの腕の「併合の分」（**N1 と同じ大きさ**。1回目の封が取り分を超え、組み直す）。
const N4_REBUILD_MERGE: u32 = N1_MERGE;

/// N4 の組み直しの腕の封: **部品の和 ＋ [`N4_REBUILD_MERGE`]**。
fn n4_rebuild_seal(body: &[String]) -> u32 {
    whole_of(body) + N4_REBUILD_MERGE
}

/// 組み直しの腕の **封 < 実**: 実数は送った本文の封より 5 多い。
fn n4_rebuild_usage_above(body: &[String]) -> u32 {
    n4_rebuild_seal(body) + 5
}

/// N4 の1つの腕。
struct N4Arm {
    name: &'static str,
    seal: SealRule,
    usage_of: SealRule,
    /// 部品は数えられないと答える（1ターン目はバイト数の計画に封をする）。
    parts_uncountable: bool,
    history_turns: usize,
    /// 1ターン目に送る計画（`plan=`）。
    plan: &'static str,
    rebuilds: u8,
    latches: bool,
}

/// N4 の腕の表（[`a_usage_above_the_sealed_count_turns_the_face_back_to_bytes`] の doc）。
const N4_ARMS: [N4Arm; 5] = [
    N4Arm {
        name: "封 < 実",
        seal: n4_seal,
        usage_of: n4_usage_above,
        parts_uncountable: false,
        history_turns: N4_HISTORY_TURNS,
        plan: "counted",
        rebuilds: 0,
        latches: true,
    },
    N4Arm {
        name: "封 = 実（対照）",
        seal: n4_seal,
        usage_of: n4_usage_equal,
        parts_uncountable: false,
        history_turns: N4_HISTORY_TURNS,
        plan: "counted",
        rebuilds: 0,
        latches: false,
    },
    N4Arm {
        name: "組み直した回の封 < 実",
        seal: n4_rebuild_seal,
        usage_of: n4_rebuild_usage_above,
        parts_uncountable: false,
        history_turns: 20,
        plan: "counted",
        rebuilds: 1,
        latches: true,
    },
    N4Arm {
        name: "バイト数の計画に封をした回の封 < 実",
        seal: n4_seal,
        usage_of: n4_usage_above,
        parts_uncountable: true,
        history_turns: N4_HISTORY_TURNS,
        plan: "bytes",
        rebuilds: 0,
        latches: true,
    },
    N4Arm {
        name: "バイト数の計画に封をした回の封 = 実（対照）",
        seal: n4_seal,
        usage_of: n4_usage_equal,
        parts_uncountable: true,
        history_turns: N4_HISTORY_TURNS,
        plan: "bytes",
        rebuilds: 0,
        latches: false,
    },
];

/// **N4: `封 < 実` なら `warn` を出し、次のターンはバイト数の計画で送る**（ADR-0052 §推奨5 の表の2行目 / §新しく要る見張り番 N4）。
///
/// | 腕 | 1ターン目に送る計画 | 実数 | 1ターン目 | 2ターン目 |
/// |---|---|---|---|---|
/// | 封 < 実 | 数えた計画 | 封 ＋ 5 | **`warn` に封・実数・計画の和が同じ1行で載る** | **部品も封も聞かず、バイト数の計画そのものを送る**（`seal=latched`） |
/// | 封 = 実（対照） | 数えた計画 | 封 | `warn` を出さない | **また封をして、数えた計画を送る** |
/// | 組み直した回の封 < 実 | 組み直した数えた計画（併合 300） | **送った本文の封** ＋ 5（**最初の封より少ない**） | 同上（行の封は送った本文の数） | 同上（掛かる） |
/// | バイト数の計画に封をした回の封 < 実 | バイト数の計画（部品は数えられない） | 封 ＋ 5 | 同上（`plan=bytes`） | 同上（掛かる） |
/// | 同・封 = 実（対照） | バイト数の計画 | 封 | `warn` を出さない | **また封をする**（`seal=counted`） |
///
/// 3つの数は互いに違う値（[`N4_MERGE`]）。2ターン目の参照値は関数の外で組む（[`TestServer::byte_plan`]）。
///
/// | 止める変異 | 赤になる腕 |
/// |---|---|
/// | 比較を `<=` にする（等しいときも掛ける） | 対照の2ターン目 |
/// | 比較を `>` にする（向きを逆にする） | `封 < 実` の両ターン |
/// | 掛け金を掛けない | `封 < 実` の2ターン目 |
/// | 中継の経路だけ判定を呼ばない | `streaming=true` の `封 < 実` |
/// | 送った本文の封ではなく最初の封で比べる | 組み直した回 |
/// | 数えた計画の回だけ判定する（B-60d-2 の1周目の形） | バイト数の計画に封をした回 |
///
/// **非ストリーミングと中継の両方**で回す（中継の `usage` は最後のチャンクで届く）。
#[tokio::test]
async fn a_usage_above_the_sealed_count_turns_the_face_back_to_bytes() {
    let log = LogSection::enter().await;

    for streaming in [false, true] {
        for arm in &N4_ARMS {
            let name = arm.name;
            let calls: Calls = Arc::new(Mutex::new(Vec::new()));
            let server =
                n4_server_with(arm.seal, arm.usage_of, arm.parts_uncountable, &calls).await;
            let session = server.seed_history(arm.history_turns, 400).await;

            let first = observe(&log, &server, &calls, "ただいま", streaming).await;
            let (sealed, actual, planned, first_sent) =
                first_turn_numbers_of(&first, arm.seal, arm.usage_of, arm.plan);
            assert!(
                has_field(&first.line, &format!("rebuilds={}", arm.rebuilds)),
                "前提: 組み直した回数（{name}）: {:?}",
                first.line
            );
            if arm.rebuilds > 0 {
                let seals = seals_in(&first.seen);
                assert!(
                    seals
                        .first()
                        .is_some_and(|(_, first_seal)| *first_seal > actual),
                    "前提: 最初の封（送らなかった本文）は実数より多いこと（{name}）: {seals:?} / {actual}"
                );
            }
            let warned = first
                .output
                .lines()
                .find(|line| line.contains(UNDERCOUNTED_MARKER));
            if arm.latches {
                let warned = warned.unwrap_or_else(|| {
                    panic!(
                        "封 < 実なら警告すること（{name} / streaming={streaming}）: {}",
                        first.output
                    )
                });
                assert!(warned.contains("WARN"), "{warned:?}");
                for field in [
                    format!("sealed_tokens={sealed}"),
                    format!("usage_prompt_tokens={actual}"),
                    format!("planned_tokens={planned}"),
                    format!("plan={}", arm.plan),
                    format!("streaming={streaming}"),
                ] {
                    assert!(
                        has_field(warned, &field),
                        "3つの数を同じ1行に載せること（{name}）: {field} / {warned:?}"
                    );
                }
            } else {
                assert!(
                    warned.is_none(),
                    "封 = 実なら警告しないこと（{name} / streaming={streaming}）: {warned:?}"
                );
            }

            settled_history(&server, session, arm.history_turns * 2 + 2).await;
            let byte_plan = server
                .byte_plan(session, "もう一度", TIGHT_CONTEXT_TOKENS)
                .await;
            let second = observe(&log, &server, &calls, "もう一度", streaming).await;
            let sends = sends_in(&second.seen);
            assert_eq!(sends.len(), 1, "前提: 送るのは1回だけであること");
            if arm.latches {
                for field in ["plan=bytes", "seal=latched", "sealed_tokens=None"] {
                    assert!(
                        has_field(&second.line, field),
                        "掛け金が掛かった世代はバイト数の計画で送ること\
                         （{name} / streaming={streaming}）: {field} / {:?}",
                        second.line
                    );
                }
                assert!(
                    second.seen.iter().all(|seen| matches!(seen, Seen::Send(_))),
                    "掛かった世代では部品も封も聞かないこと（{name} / streaming={streaming}）"
                );
                assert_eq!(
                    sends[0], byte_plan,
                    "送った本文がバイト数の計画そのものであること（{name} / streaming={streaming}）"
                );
                if arm.plan == "counted" {
                    assert!(
                        byte_plan.len() < first_sent.len() + 2,
                        "前提: バイト数の計画は、数えた計画より少なく載せること: {} / {} 件",
                        byte_plan.len(),
                        first_sent.len() + 2
                    );
                }
            } else {
                for field in [format!("plan={}", arm.plan), "seal=counted".to_owned()] {
                    assert!(
                        has_field(&second.line, &field),
                        "封 = 実なら次のターンも封をすること（{name} / streaming={streaming}）: \
                         {field} / {:?}",
                        second.line
                    );
                }
                assert!(
                    !seals_in(&second.seen).is_empty(),
                    "封 = 実なら次のターンも封を聞くこと（{name}）"
                );
                if arm.plan == "counted" {
                    assert_ne!(sends[0], byte_plan, "前提: 数えた計画を送っていること");
                }
            }

            server.stop().await;
        }
    }
}

/// **`封 > 実` では掛け金が掛からない**（多めに数えるのは無害 / ADR-0052 §推奨5 の表の3行目）。
///
/// 実数は封より 3 少なく、計画の和より 4 多い（3つとも別の値）。`warn` を出さず、次のターンも封をする。
#[tokio::test]
async fn a_sealed_count_above_the_usage_does_not_turn_the_face_back() {
    let log = LogSection::enter().await;

    for streaming in [false, true] {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let server = n4_server(n4_usage_below, &calls).await;
        let session = server.seed_history(N4_HISTORY_TURNS, 400).await;

        let first = observe(&log, &server, &calls, "ただいま", streaming).await;
        first_turn_numbers(&first, n4_usage_below);
        assert!(
            !first.output.contains(UNDERCOUNTED_MARKER),
            "封 > 実で警告しないこと（streaming={streaming}）: {}",
            first.output
        );

        settled_history(&server, session, N4_HISTORY_TURNS * 2 + 2).await;
        let second = observe(&log, &server, &calls, "もう一度", streaming).await;
        for field in ["plan=counted", "seal=counted"] {
            assert!(
                has_field(&second.line, field),
                "封 > 実なら次のターンも封をすること（streaming={streaming}）: {field} / {:?}",
                second.line
            );
        }

        server.stop().await;
    }
}

/// 引き直しが答えたモデルの名前を書き替える（`None` は引き直しの失敗）。
fn answered(server: &TestServer, name: Option<&str>) {
    server
        .upstream
        .refresh_for_tests(ModelFace::Main, None, None, None, name.map(str::to_owned));
}

/// **掛け金は、答えたモデルが別の名前に変わったら外れる。引き直しの失敗では外れない**
/// （ADR-0052 §推奨4 の「面の世代」/ 作業単位 B-60d-2 のレビュー 提案4）。
///
/// | ターン | 答えた名前 | 期待 |
/// |---|---|---|
/// | 1 | `first-model.gguf` | 封 < 実で掛かる |
/// | 2 | 同じ | `seal=latched` |
/// | 3 | **`None`（引き直しの失敗）** | **`seal=latched` のまま** |
/// | 4 | `first-model.gguf` に戻る | `seal=latched` のまま |
/// | 5 | **`another-model.gguf`** | **外れて、また封をする** |
///
/// 5 の腕が無いと「一度掛かったら二度と数えない」実装が、3・4 の腕が無いと「世代の `==` で外す」実装が緑になる。
#[tokio::test]
async fn a_latch_is_released_when_the_generation_changes() {
    let log = LogSection::enter().await;

    for streaming in [false, true] {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let server = n4_server(n4_usage_above, &calls).await;
        let session = server.seed_history(N4_HISTORY_TURNS, 400).await;
        answered(&server, Some("first-model.gguf"));

        let first = observe(&log, &server, &calls, "ただいま", streaming).await;
        first_turn_numbers(&first, n4_usage_above);
        let mut messages = N4_HISTORY_TURNS * 2 + 2;
        for (turn, name) in [
            ("もう一度", Some("first-model.gguf")),
            ("引き直しに失敗した？", None),
            ("戻った？", Some("first-model.gguf")),
        ] {
            answered(&server, name);
            settled_history(&server, session, messages).await;
            messages += 2;
            let seen = observe(&log, &server, &calls, turn, streaming).await;
            assert!(
                has_field(&seen.line, "seal=latched"),
                "掛け金が掛かったままであること（答えた名前 {name:?} / streaming={streaming}）: {:?}",
                seen.line
            );
        }

        answered(&server, Some("another-model.gguf"));
        settled_history(&server, session, messages).await;
        let third = observe(&log, &server, &calls, "載せ替えた？", streaming).await;
        for field in ["plan=counted", "seal=counted"] {
            assert!(
                has_field(&third.line, field),
                "世代が変わったら掛け金が外れること（streaming={streaming}）: {field} / {:?}",
                third.line
            );
        }
        assert!(
            !seals_in(&third.seen).is_empty(),
            "世代が変わったら封を聞き直すこと"
        );

        server.stop().await;
    }
}

/// **組み直したターンでも、想起の行は1本だけで、送った計画のもの**（作業単位 B-60d-1 のレビュー 要修正4）。
///
/// 封は何を数えても取り分を1だけ超える（[`n3_seal`]）ので、**1ターンに3回組む**
/// （数えた計画 → 組み直し → バイト数の計画）。測定の道具は想起の行1本を1ターンと数える。
///
/// 記憶を取り分より多く置き、**1回目の計画とバイト数の計画で載る記憶の件数が違う**ことを前提として確かめる。
/// 想起の行の数は、関数の外で組んだバイト数の計画（[`TestServer::byte_plan_built`]）の `recall` と突き合わせる。
#[tokio::test]
async fn a_rebuilt_turn_logs_one_recall_line_for_the_plan_it_sent() {
    let log = LogSection::enter().await;
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let server = TestServer::inject_with(
        Arc::new(Tokenizing::new(n3_seal, &calls)),
        &[
            (ENV_CONTEXT_TOKENS, TIGHT_CONTEXT),
            (ENV_SUB_LLM_ENDPOINT, SEPARATE_SUB),
        ],
    )
    .await;
    server.seed_memories(40, 200).await;
    let session = server.seed_history(10, 400).await;
    let reference = server
        .byte_plan_built(session, "ただいま", TIGHT_CONTEXT_TOKENS)
        .await;

    let mark = log.mark();
    server.ok_turn("ただいま", false).await;
    let line = log.one_seal_line_since(mark);
    assert!(
        has_field(&line, "plan=bytes") && has_field(&line, "rebuilds=1"),
        "前提: 組み直してもなおバイト数の計画へ戻った回であること: {line:?}"
    );
    let seen = one_copy(&calls);
    let seals = seals_in(&seen);
    let sends = sends_in(&seen);
    assert_eq!(seals.len(), 2, "前提: 2回封をしたこと");
    assert_ne!(
        memory_lines(&seals[0].0),
        memory_lines(&sends[0]),
        "前提: 1回目の計画と送った計画で、載る記憶の件数が違うこと"
    );

    let output = log.buffer.contents_since(mark);
    let recall: Vec<&str> = output
        .lines()
        .filter(|line| line.contains(RECALL_MARKER))
        .collect();
    assert_eq!(
        recall.len(),
        1,
        "1ターンに想起の行は1本だけであること: {recall:#?}"
    );
    let counts = reference.recall;
    for field in [
        format!("recalled_recent={}", counts.recalled_recent()),
        format!("recalled_related={}", counts.recalled_related()),
        format!("omitted={}", counts.omitted()),
    ] {
        assert!(
            has_field(recall[0], &field),
            "想起の行は送った計画（バイト数の計画）のものであること: {field} / {:?}",
            recall[0]
        );
    }
    assert_eq!(
        counts.recalled_recent() + counts.recalled_related(),
        memory_lines(&sends[0]),
        "前提: 参照の想起の件数が、送った本文に載った記憶の件数と同じこと"
    );

    server.stop().await;
}

/// 想起の行の手掛かり（`prompt` の `RECALL_MESSAGE` の先頭）。
const RECALL_MARKER: &str = "記憶を想起しました";

/// **予算に入らない回（422）も、想起の行の数は `prompt::build_prompt` を直に呼んだときと同じ**
/// （作業単位 B-60d-1 の再レビュー 要修正2）。
///
/// # 再レビューの前提との食い違い（測った）
///
/// 再レビューは「`BudgetExhausted` の回にも想起の行が出ていた」とした（`Report::Now` の1周目の doc の主張）。
/// **その doc は誤りだった。** `BudgetExhausted` は想起より前（`budget::Frames::new`）で決まるので、
/// `build_prompt` を直に呼んでも想起の行は出ない（`prompt` の `a_prompt_over_the_budget_logs_no_recall_line`）。
/// ここでは数を決め打ちせず、**同じ入力を `build_prompt` に直に通したときの行の数**と比べる ——
/// 「server を通しても今と同じ」がこのテストの主張である。
///
/// 数え手は部品も封も数える（数えた計画も組めない大きさの発言 —— 4 バイトで1つに数えても取り分を超える）。
#[tokio::test]
async fn a_turn_over_the_budget_logs_as_many_recall_lines_as_the_prompt_does() {
    let log = LogSection::enter().await;
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let server = TestServer::inject_with(
        Arc::new(Tokenizing::new(whole_of, &calls)),
        &[
            (ENV_CONTEXT_TOKENS, TIGHT_CONTEXT),
            (ENV_SUB_LLM_ENDPOINT, SEPARATE_SUB),
        ],
    )
    .await;
    server.seed_memories(5, 200).await;
    let session = server.seed_history(2, 400).await;
    let huge = "a".repeat(4 * TIGHT_ALLOWANCE as usize + 400);
    assert!(
        quarter(&huge) > TIGHT_ALLOWANCE,
        "前提: 数えた計画でも削れない枠が取り分を超えること"
    );

    let direct_mark = log.mark();
    let direct = server
        .try_byte_plan(session, &huge, TIGHT_CONTEXT_TOKENS)
        .await;
    assert!(
        matches!(
            direct,
            Err(personacore::prompt::PromptError::BudgetExhausted)
        ),
        "前提: バイト数の計画は組めないこと"
    );
    let direct_lines = log
        .buffer
        .contents_since(direct_mark)
        .lines()
        .filter(|line| line.contains(RECALL_MARKER))
        .count();

    let mark = log.mark();
    let (status, body) = server.turn(&huge, false).await;
    assert_eq!(status, 422, "予算に入らない回は今と同じく 422: {body}");
    let served_lines = log
        .buffer
        .contents_since(mark)
        .lines()
        .filter(|line| line.contains(RECALL_MARKER))
        .count();
    assert_eq!(
        served_lines, direct_lines,
        "server を通しても、想起の行の数は build_prompt を直に呼んだときと同じであること"
    );
    assert!(
        one_copy(&calls)
            .iter()
            .any(|seen| matches!(seen, Seen::Part(_))),
        "前提: 部品を数えに行ったこと（数えた計画も試したこと）"
    );

    server.stop().await;
}

/// 本番の入口の腕で、本文と裏方に別々に依頼するモデルの綴り（**同じ上流の、別のモデル**）。
const BIND_MAIN_MODEL: &str = "planted-main";
const BIND_SUB_MODEL: &str = "planted-sub";

/// 本番の入口の腕で、上流が申告する窓。**本文と裏方で別の値、どちらも落とし所（16384）と別の値。**
const BIND_MAIN_CONTEXT: u32 = 20_000;
const BIND_SUB_CONTEXT: u32 = 12_000;

/// **llama を演じる数え手**（`wiremock` の応答を本文から作る）。
///
/// * `/tokenize`: `content` を 4 バイトで1つに数える（空でなければ 1 以上）。
/// * `/apply-template`: 送られた `messages` の中身をつないだ文字列を、組み上がりとして返す。
struct CountingResponder;

impl wiremock::Respond for CountingResponder {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let answer = if request.url.path() == "/tokenize" {
            let content = body["content"].as_str().unwrap_or_default();
            let tokens: Vec<usize> = (0..content.len().div_ceil(4).max(1)).collect();
            json!({ "tokens": tokens })
        } else {
            let joined: Vec<&str> = body["messages"]
                .as_array()
                .map(|messages| {
                    messages
                        .iter()
                        .filter_map(|message| message["content"].as_str())
                        .collect()
                })
                .unwrap_or_default();
            json!({ "prompt": joined.join("\n") })
        };
        ResponseTemplate::new(200).set_body_raw(answer.to_string(), "application/json")
    }
}

/// **本番の入口（`App::bind`）で、本文と裏方が同じ上流のとき、裏方の窓から差し引く量を決める**
/// （作業単位 B-60d-1 のレビュー P4 / CLAUDE.md「テスト専用の入口を足したら、本番の入口を通るテストを1本置く」）。
///
/// `PERSONACORE_SUB_LLM_ENDPOINT` を**設定しない**（裏方は本文と同じ宛先 → 同じ上流）。
/// 上流は本文のモデルに 20000、裏方のモデルに 12000 を申告する。キャラクターは軸を5本持つ（うち1本は値 0 ——
/// 数え落とすと 6000 ＋ 344 = 6344 で赤）。
///
/// 期待する差し引きは **12000 の半分（6000）＋ 変容の 86 × 5 = 430 → 6430**（関数の外の値）。
/// **裏方の窓を落とし所（16384）から取る形**（`state.sub_context()` → `SubContext::FALLBACK`）は
/// 8192 ＋ 430 = 8622 になって赤。**本文の窓で数える形**も 10000 ＋ 430 で赤。
#[tokio::test]
async fn a_shared_upstream_through_the_real_bind_deducts_the_background_window_it_queried() {
    let log = LogSection::enter().await;
    let upstream = chat_only().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                json!({
                    "object": "list",
                    "data": [
                        {"id": BIND_MAIN_MODEL, "object": "model", "type": "llm", "state": "loaded",
                         "max_context_length": 262_144, "loaded_context_length": BIND_MAIN_CONTEXT},
                        {"id": BIND_SUB_MODEL, "object": "model", "type": "llm", "state": "loaded",
                         "max_context_length": 262_144, "loaded_context_length": BIND_SUB_CONTEXT},
                    ],
                })
                .to_string(),
                "application/json",
            ),
        )
        .mount(&upstream)
        .await;
    for wanted in ["/apply-template", "/tokenize"] {
        Mock::given(matchers::method("POST"))
            .and(matchers::path(wanted))
            .respond_with(CountingResponder)
            .mount(&upstream)
            .await;
    }

    assert_ne!(
        upstream.uri().trim_end_matches('/'),
        DEFAULT_ENDPOINT,
        "実機へ向けないこと"
    );
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let uri = upstream.uri();
    let config = AppConfig::from_lookup(|key| match key {
        ENV_LLM_ENDPOINT => Some(uri.clone()),
        ENV_MAIN_MODEL => Some(BIND_MAIN_MODEL.to_owned()),
        ENV_SUB_MODEL => Some(BIND_SUB_MODEL.to_owned()),
        _ => None,
    })
    .expect("設定を組み立てられること")
    .with_bind(BindAddr::loopback(0))
    .with_database_path(dir.path().join("personacore.sqlite"));
    let app = App::bind(&config).await.expect("待ち受けを開始できること");
    let server = TestServer::start(app, dir).await;
    // **5本のうち1本は値 0**（変容の依頼には載るので数える / 再レビュー 要修正3）。
    server.seed_axes_on(HANDLE, &SHARED_AXIS_VALUES).await;

    let mark = log.mark();
    server.ok_turn("ただいま", false).await;
    let line = log.one_seal_line_since(mark);
    assert!(
        has_field(&line, "plan=counted"),
        "前提: 数えた計画を送った回であること: {line:?}"
    );
    let expected = BIND_SUB_CONTEXT / 2 + 86 * 5;
    assert!(
        has_field(&line, &format!("shared_deduction={expected}")),
        "裏方の窓（上流が申告した {BIND_SUB_CONTEXT}）から差し引く量を決めること: {line:?}"
    );

    server.stop().await;
}
