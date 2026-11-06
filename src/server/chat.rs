//! `POST /v1/chat/completions`（作業単位 11a-2a ＋ 11a-2b）。
//!
//! この層は薄く保つ（CLAUDE.md 3節）。判断は `domain`（`ModelSelector`）と
//! `prompt`（予算配分）に置き、ここは「受ける・解決する・呼ぶ・返す」だけを担う。
//! SQL は書かない。
//!
//! # 2つの出口
//!
//! **入口の検査・セッション解決・プロンプト組立・上流へ渡す内容はまったく同じ**で、
//! 分かれるのは最後だけ。
//!
//! | `stream` | 上流 | 応答 |
//! |---|---|---|
//! | 省略 / `false` | `LlmClient::complete` | `chat.completion` の JSON |
//! | `true` | `LlmClient::stream` | `chat.completion.chunk` の SSE（`super::relay`） |
//!
//! **分岐は上流を呼ぶ直前の1箇所だけ**にしてある。前に置くと、入口の検査や
//! 予算の切り下げが片方の経路だけ緩む形の劣化が起こりうる。
//!
//! # 応答を返した後（11b-2）
//!
//! **台詞／地の文への分解と `message` への保存は `super::turn` が持つ。**
//! この層は「どこへ・何を渡すか」だけを決める。
//!
//! | 経路 | 保存する場所 |
//! |---|---|
//! | 非ストリーミング | 上流の応答を受け取った直後（ここ） |
//! | 中継 | **上流が尽きた枝だけ**（`super::relay::chunk_events`） |
//!
//! **応答は上流から受け取ったまま返す。** 分解結果は DB にだけ入る
//! （クライアントに返す `content` を組み直すと、モデルが直前に自分で出した形式と
//! 食い違う）。**保存の失敗で応答を壊さない**（`super::turn` の doc）。
//!
//! # 信頼できない入力の扱い
//!
//! | 入力 | 通す関門 |
//! |---|---|
//! | `model` | `ModelSelector::parse` → `Store::find_character_by_handle`（完全一致） |
//! | `aoi@<id>` の生の `i64` | **`Store::find_session` の所有確認だけ**（ADR-0004） |
//! | `messages` の本文 | `prompt::build_prompt`（`SafeText` による無害化） |
//!
//! **`SessionRef::Existing(i64)` を `find_session` 以外へ渡さないこと。**
//! バリアントは公開で中身は生の `i64` なので、型は何も守っていない。
//! `find_session` が `character_id` と併せて引くことだけが、
//! 「他キャラのセッション id を書くと別キャラの会話へ書き込める」を塞いでいる。

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa::openapi::schema::Object;
use uuid::Uuid;

use super::error::{ApiError, ErrorBody};
use super::openapi::ChatCompletionEvent;
use super::relay::{Relay, relay_response};
use super::turn::TurnSink;
use super::{AppState, counted_plan, openapi, seal, usage};
use crate::domain::{Character, Handle, ModelSelector, SessionId, SessionRef};
use crate::llm::{FinishReason, LlmError};
use crate::prompt::{PromptError, PromptInput};
use crate::store::StoreError;

/// `object` フィールドの定数（OpenAI 互換）。
///
/// **中継側は `chat.completion.chunk`**（`super::relay`）。取り違えないこと。
pub(super) const COMPLETION_OBJECT: &str = "chat.completion";

/// 応答の役割。OpenAI 互換の綴り。**中継側と共有する**（綴りを2箇所に置かない）。
pub(super) const ASSISTANT_ROLE: &str = "assistant";

/// 今回の発言として採用する役割。**完全一致**（`User` / `USER` は受けない）。
const USER_ROLE: &str = "user";

/// 応答 id の接頭辞（OpenAI 互換クライアントが `chatcmpl-` を期待する）。
const COMPLETION_ID_PREFIX: &str = "chatcmpl-";

/// DB から読む会話履歴の上限件数。
///
/// **ここは「読む上限」であって「載せる上限」ではない。** 実際に何件載るかは
/// `prompt::budget` が決める（履歴は最初に削られる枠）。多めに読んでおいて
/// 予算に削らせる形にしてあるので、この値を増やしても
/// プロンプトが膨らむわけではない（DB の読み取り量だけが増える）。
const HISTORY_LIMIT: u32 = 50;

/// 本文を読めなかったときにログへ残す入口のラベル（**応答には載らない**）。
const ENDPOINT: &str = "chat_completions";

/// `POST /v1/chat/completions` の要求。
///
/// **`deny_unknown_fields` を付けないこと。** ここは OpenAI 互換の境界（ADR-0001）で、
/// 実クライアントは `top_p` / `presence_penalty` / `stop` などを普通に送ってくる。
/// 未知のフィールドで 400 にすると、既存クライアントをそのまま FE に使うという
/// フェーズ1の狙いが崩れる。**`/admin/` とは方針が違う**（あちらは綴り間違いを
/// 黙って通さないために厳格にしてある）。
#[derive(Debug, Deserialize, ToSchema)]
pub(super) struct ChatCompletionRequest {
    /// **キャラクター／セッションの選択子**（ADR-0004）。推論モデルの指定ではない。
    /// 信頼できない入力なので `ModelSelector::parse` と許可リスト照合を通す。
    model: String,
    /// クライアントが持っている会話。**採用するのは最後の `user` だけ。**
    messages: Vec<IncomingMessage>,
    /// 温度。**クライアントの値をそのまま上流へ通す**
    /// （マイルストーン5の検証が `temperature = 0` を使う）。
    #[serde(default)]
    temperature: Option<f32>,
    /// 生成トークン上限。**素通ししない**（`cap_max_tokens` で余白まで切り下げる）。
    #[serde(default)]
    max_tokens: Option<u32>,
    /// ストリーミング要求。**未指定と `false` を区別しない**（どちらも非ストリーミング）。
    #[serde(default)]
    stream: Option<bool>,
}

/// クライアントが送ってきたメッセージ1件。
///
/// # `content` を `String` で受けないこと
///
/// **採用しないメッセージの形が原因で要求全体を 400 にしない。**
/// `content: String`（必須かつ文字列限定）にすると、
///
/// * `{"role": "assistant", "content": null}` — `tool_calls` 付きの履歴を持つクライアント
/// * `{"role": "user", "content": [{"type": "text", ...}]}` — マルチモーダル形式
///
/// のどちらか1件が混ざっただけで本文全体が解釈できなくなる。これは
/// `deny_unknown_fields` を付けなかったのと**同じ穴がフィールドの型の側に空く**形
/// （ADR-0001「実クライアントをそのまま FE に使う」）。
///
/// **中身を見るのは採用する1件だけ**（`last_user_message`）で、
/// それ以外は `Value` のまま一切触らない。
#[derive(Debug, Deserialize, ToSchema)]
struct IncomingMessage {
    role: String,
    /// **キーごと無い場合と `null` を区別しない。** どちらも「文字列ではない」。
    #[serde(default)]
    content: Option<Value>,
}

/// OpenAI 互換の応答。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct ChatCompletionView {
    id: String,
    #[schema(schema_with = completion_object_schema)]
    object: &'static str,
    /// 生成時刻（Unix 秒）。
    created: i64,
    /// **着地したセッション（`aoi@<session-id>`）。**
    ///
    /// どのセッションに書かれたのかがクライアントから見え、その値を次のリクエストの
    /// `model` にそのまま使えばセッションを固定できる（ADR-0004 の復旧経路）。
    model: String,
    choices: Vec<ChoiceView>,
}

/// 候補1件。**`n > 1` は要求しないので常に1件。**
#[derive(Debug, Serialize, ToSchema)]
struct ChoiceView {
    index: u32,
    message: MessageView,
    /// 上流が示さなかった場合と、こちらが綴りを決められない場合は `null`。
    #[schema(schema_with = finish_reason_schema, required = true)]
    finish_reason: Option<&'static str>,
}

/// 応答のメッセージ。
///
/// **仕様書では `ChatMessageView` を名乗る**（`admin::MessageView` と名前が重なるため /
/// `openapi::tests::no_two_types_share_a_schema_name`）。
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ChatMessageView)]
struct MessageView {
    #[schema(schema_with = assistant_role_schema)]
    role: &'static str,
    content: String,
}

/// 補完（非ストリーミングと SSE 中継の共通経路）。
///
/// 手順は「`max_tokens` の検査 → `model` の解釈 → 今回の発言の決定 →
/// 許可リスト照合 → セッション解決 → プロンプト組立 → 上流呼び出し → 応答」。
/// **入力の検査は DB に触れる前に済ませる。**
/// **DB のトランザクション内で上流を呼ばない**（絶対制約）。`store` の各関数は
/// それぞれ完結したトランザクションなので、上流呼び出しはそのすべての外にある。
/// **SSE 中継はさらにその外側**（応答本文のストリームは `store` に触れない）。
#[utoipa::path(
    post,
    path = "/v1/chat/completions",
    summary = "Chat with a character (OpenAI-compatible, JSON or SSE)",
    tag = "openai",
    request_body = ChatCompletionRequest,
    responses(
        (status = 200, content(
            (ChatCompletionView = "application/json"),
            ("text/event-stream", item_schema = ChatCompletionEvent),
        )),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn chat_completions(
    State(state): State<AppState>,
    // **`Json<T>` を直接受けない。** 直接受けると axum の既定の拒否本文
    // （serde のメッセージ = 入力値を含む）がそのまま応答に載る。
    body: Result<Json<ChatCompletionRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    // **本文の拒否は `ApiError::from_json_rejection` に一本化してある。**
    // 入力を反射しないこと・413 を 400 に潰さないことは、そちらの doc を参照。
    let Json(request) =
        body.map_err(|rejection| ApiError::from_json_rejection(rejection, ENDPOINT))?;

    // 1. ストリーミング要求か。**分岐そのものは上流を呼ぶ直前まで持ち越す**
    //    （入口の検査を片方の経路だけ通らない形にしないため）。
    let streaming = request.stream.unwrap_or(false);

    // **予算はここで1回だけ読む。** 稼働中に背景タスクが差し替えうる（ADR-0020）ので、
    // 組立・中継・答え合わせで読み直すと、ちょうど差し替わった回だけ
    // 「組んだときの予算」と「照合に使う予算」が食い違う。
    // **新しい値は次のリクエストの組立から効く**（組立中のものには遡らない）。
    let budget = state.budget();

    // 2. **`max_tokens: 0` は入口で拒否する。** `cap_max_tokens` は上限を切り下げる
    //    関数なので 0 を素通しし、そのまま上流へ「1トークンも生成するな」と送ってしまう。
    if request.max_tokens == Some(0) {
        tracing::debug!("max_tokens に 0 が指定されました");
        return Err(ApiError::invalid_max_tokens());
    }

    // 3. `model` の解釈（ADR-0004 の「単一の関数に閉じ込める」）。
    //    **失敗を 404 に潰さない。** 詳細（入力そのもの）はログにだけ出す。
    let selector = ModelSelector::parse(&request.model).map_err(|err| {
        tracing::debug!(error = %err, "model を解釈できませんでした");
        ApiError::invalid_model(err)
    })?;
    let (handle, session_ref) = selector.into_parts();

    // 4. 今回の発言。**最後の `user` メッセージだけ**を採る
    //    （docs/02-architecture.md「自前DBを正とする」。履歴は DB から読む）。
    //    **「1件も無い」と「形式が未対応」を同じ応答に潰さない。**
    let user_message = match last_user_message(&request.messages) {
        Adopted::Text(text) => text,
        Adopted::UnsupportedContent => {
            tracing::debug!(
                messages = request.messages.len(),
                "最後の user メッセージの content が文字列ではありません"
            );
            return Err(ApiError::unsupported_message_content());
        }
        Adopted::Missing => {
            tracing::debug!(
                messages = request.messages.len(),
                "user のメッセージがありません"
            );
            return Err(ApiError::no_user_message());
        }
    };

    // 5. 許可リスト照合（格納値との完全一致）。
    let character = state
        .store()
        .find_character_by_handle(&handle)
        .await
        .map_err(|err| internal(&err, "キャラクターを取得できませんでした"))?
        .ok_or_else(|| {
            tracing::debug!(character = %handle, "存在しないキャラクターが指定されました");
            ApiError::model_not_found()
        })?;

    // 5b. **しまったキャラクターとは話さない**（ADR-0049 §決定1 / 作業単位 A-10）。
    //     **セッション解決より前に断る。** 後に置くと `aoi@new` / 会話の無い `aoi` が
    //     空の会話を作ってから断ることになる（「黙って会話を足さない」に反する）。
    //
    //     ⚠ **これは読んでから書く形**で、ここを通った直後にしまわれた要求は、その1ターンを書く。
    //     中身を消す操作ではない（戻せば元どおり）ので、`DELETE_CHARACTER` のように1文には寄せていない。
    if character.archived_at.is_some() {
        tracing::debug!(character = %handle, "しまったキャラクターが指定されました");
        return Err(ApiError::model_archived());
    }

    // 6. セッション解決（生の id はここでだけ `find_session` に渡す）。
    let session_id = resolve_session(&state, &character, session_ref).await?;

    // 7. プロンプト組立。
    let stats = state
        .store()
        .stat_allocs(character.id)
        .await
        .map_err(|err| internal(&err, "性格ステータスを取得できませんでした"))?;
    // **属性は「削れない枠」に載る**（ADR-0009 / 作業単位 12b）。
    // 読み出しは1行でも壊れていたら丸ごと失敗する（`store::attribute` の doc）ので、
    // **黙って空として続けない**——属性が落ちたキャラクターは別人として応答する。
    let attributes = state
        .store()
        .character_attributes(character.id)
        .await
        .map_err(|err| internal(&err, "キャラクター属性を取得できませんでした"))?;
    // **有効な取り決めは、削れない枠の次の「優先の枠」に載る**（ADR-0051 §決定4 の追記 / 作業単位 A-12a）。
    // 入りきらないときだけ古い側から落ち、その件数は `warn` と `last_turn` で伝わる（`prompt::budget`）。
    // 読むのは有効なものだけ（終わった行は読まない）。**1行でも壊れていたら丸ごと失敗する**
    // ——黙って空として続けると、キャラクターが理由なく約束を忘れて応答する（属性と同じ判断）。
    let agreements = state
        .store()
        .active_agreements(character.id)
        .await
        .map_err(|err| internal(&err, "取り決めを取得できませんでした"))?;
    let memories = state
        .store()
        .recall_memories(character.id)
        .await
        .map_err(|err| internal(&err, "記憶を取得できませんでした"))?;
    let history = state
        .store()
        .recent_messages(session_id, HISTORY_LIMIT)
        .await
        .map_err(|err| internal(&err, "会話履歴を取得できませんでした"))?;

    // 8. 上流へ送る本文を組む（**数えた計画と封** / ADR-0052 / 作業単位 B-60d-1 / `super::counted_plan`）。
    //
    // **クライアントの `max_tokens` を素通ししない。** 素通しすると
    // プロンプト＋応答が上流のコンテキストを超えうる。応答の余白はどの計画でも元の予算のもの。
    let max_tokens = cap_max_tokens(request.max_tokens, budget.response_tokens());

    // **世代を先に、写しを後に読む**（`context::SharedUpstream::generation` の doc）。
    // **部品を数えるのも、封をするのも、送るのもこの写し**——2回取ると、間の差し替えで
    // 数えた上流と送った上流が食い違う。
    // 数える往復の分だけ本文の応答は遅れる（上限は `llm` の照会の上限 / `super::seal` の doc）。
    // **封は上流を開く前に終わる**ので、ストリーミングでも失敗はまだ 4xx/5xx で返せる。
    let generation = state.main_generation();
    let sharing = state.main_window_sharing();
    // **差し引く裏方の1要求分の、変容の軸の本数**（`counted_plan::background_request`）。
    // **このキャラクターの本数ではなく、DB にある全キャラクターの最大**（`Store::max_stat_axes`）
    // ——本文と重なるのは直前のターンの裏方で、それは別のキャラクターの仕事でありうる。
    // 別の上流なら差し引かないので数えない（1回の集計を省く）。
    // **⚠ 守れないもの**: 管理画面が軸を減らした直後、古い本数で組んだ変容がまだ走っている間は、
    // 差し引く量がその差だけ足りない（受容 / 作業単位 B-60d-1 の再レビュー）。
    // **世代と写しの順は崩さない**（この `await` は世代を読んだ後、写しを取る前）。
    let axes = match sharing {
        counted_plan::WindowSharing::SameUpstream => state
            .store()
            .max_stat_axes()
            .await
            .map_err(|err| internal(&err, "性格ステータスの軸の本数を数えられませんでした"))?,
        counted_plan::WindowSharing::Separate => 0,
    };
    let llm = state.llm_for_turn();
    let planned = counted_plan::plan_turn(
        &counted_plan::Turn {
            generation: &generation,
            llm: llm.as_ref(),
            uncountable: state.uncountable(),
            parts: state.parts(),
            character: character.id.get(),
            // **起動時に決めた予算を使う。`TokenBudget::default()` を直に呼ばないこと**
            // （設定も上流の実コンテキスト長も無視され、実測 16384 に対して
            // 8192 しか使わない状態に戻る。ADR-0013 第2層）。
            budget,
            sharing,
            sub_context: state.sub_context(),
            // **変容の応答の上限の出どころ**（裏方の1要求分 / `counted_plan::background_request`）。
            axes,
            streaming,
            temperature: request.temperature,
            max_tokens,
        },
        &PromptInput {
            character: &character,
            stats: &stats,
            attributes: &attributes,
            agreements: &agreements,
            memories: &memories,
            history: &history,
            user_message,
        },
    )
    .await
    .map_err(|err| prompt_error(err, &character.handle))?;
    // **取り決めの載り方を覚える**（作業単位 A-12a の決め直し）。落ちたら画面に出すため
    // （`GET .../agreements` の `last_turn`）。DB には持たない。**送った計画のもの**を覚える。
    state.record_agreement_fit(
        &character,
        planned.plan.included_agreements(),
        planned.plan.dropped_agreements(),
    );
    // **`model` は渡らない**（`ChatRequest` に項目が無い / ADR-0004）。
    let counted_plan::Planned {
        plan,
        request: upstream,
        seal: sealed,
    } = planned;
    // **封を数えた世代と記憶を添える**（応答の後の答え合わせで掛け金を掛けるため / `super::seal::Sealed`）。
    // **書く直前にいまの世代を読み直す口も渡す**（遅れて終わった古い世代のターンが、いまの世代の記憶を消さないように）。
    let sealed = seal::Sealed::new(
        sealed,
        state.uncountable_shared(),
        generation,
        state.main_generation_source(),
    );

    // 9. **ストリーミングならここで中継へ抜ける**（11a-2b）。
    //
    // **上流を開くまでの失敗は、まだ 4xx/5xx で返せる**（200 を1バイトも送っていない）。
    // 開いた後の失敗は SSE の中でしか伝えられない（`super::relay` の doc）。
    // `LlmClient::stream` の future が本文の到着を待たずに解決するのは、
    // まさにこの境界を作るため（`llm::LlmClient` の doc）。
    if streaming {
        let chunks = llm
            .stream(&upstream)
            .await
            .map_err(|err| upstream_error(&err))?;
        // **本文を受け付けた後に覚える**（受け付けなかった上流の答えを覚えない / `super::seal` の doc）。
        // **`stream()` の前へ移さないこと**——見張り番は `tests/server_seal.rs` の
        // `a_count_refused_while_loading_is_not_remembered`（1ターン目が中継の腕）。
        sealed.accepted();

        tracing::info!(
            character = %character.handle,
            session_id = session_id.get(),
            "SSE 中継を開始します"
        );

        // **`model` は非ストリーミングと同じ `aoi@<session-id>`。**
        // id / created も1本の応答を通して共通（`Relay` の doc）。
        return Ok(relay_response(
            chunks,
            Relay::new(
                completion_id(),
                session_model(&character.handle, session_id),
                Utc::now().timestamp(),
            ),
            budget,
            plan,
            max_tokens,
            // **封の行は中継の終わりに `usage` と同じ1行へ出す**（`super::relay::finish_relay`）。
            // **`封 < 実` の判定と掛け金もそこ**（`usage` は最後のチャンクで届く / 作業単位 B-60d-2）。
            sealed,
            // **中継が完結した回だけ書く**（途中で壊れた回は1行も残さない / `super::relay`）。
            // **プロンプトに載せたのと同じ軸の一覧**を渡す（ADR-0018 の判定材料）。
            // ここで引き直さない——追加のクエリを足さないのがこの検出の前提。
            // **記憶抽出の配線**（ADR-0021 §6 / 作業単位 13b）。走り出すのは
            // `TurnSink::persist` が保存に成功した直後で、ここでは渡すだけ。
            TurnSink::new(
                state.store().clone(),
                session_id,
                user_message,
                stats,
                state.extraction(&character),
            ),
        ));
    }

    let completion = llm
        .complete(&upstream)
        .await
        .map_err(|err| upstream_error(&err))?;
    // **本文を受け付けた後に覚える**（中継の枝と同じ / `super::seal` の doc）。
    sealed.accepted();

    // 10. **答え合わせ**（ADR-0013 第3層）。上流が数えた実トークン数と突き合わせる。
    //
    // **応答を返す前でも後でもよいが、ここから外さないこと。** 起動時の照会
    // （第2層）は稼働中の変化を検出できないので、モデル・上流・テンプレートの
    // 変化に気付ける唯一の場所がこの1行になる。判断は `super::usage` の純粋関数。
    //
    // **上界の判定は、バイト数の計画を送った回だけ**（数えた計画の和は上界ではない /
    // ADR-0052 §背景）。**コンテキストの超過の判定は毎回**（ADR-0052 §推奨5 の4行目「今と同じ」）。
    // 封をした回の答え合わせ（封 < 実）は下の `settle`（作業単位 B-60d-2）。
    usage::report(
        &plan,
        sealed.estimate(),
        budget,
        max_tokens,
        completion.usage,
    );
    // **封の行と、封をした回の答え合わせ**（ADR-0052 §推奨5）。**`usage` は答え合わせと同じもの**を渡す。
    // `封 < 実` なら、この世代に掛け金を掛ける（次のターンからバイト数の計画 / `super::seal::Sealed::settle`）。
    sealed.settle(&plan, completion.usage, false);

    // 11. **保存**（11b-2 / マイルストーン4）。応答は完全に返り切っている。
    //
    // **トランザクションの中で上流を呼ばない**（絶対制約）ので、書くのはここ——
    // `complete` を await し終えた後。**保存に失敗しても応答は返す**
    // （生成し直させる方が高くつく / `super::turn` の doc）。
    // **軸名の漏れの判定材料**（ADR-0018）。プロンプトに載せたのと同じ一覧を渡す。
    // **記憶抽出の配線**（ADR-0021 §6 / 作業単位 13b）。中継側と同じものを渡す
    // ——走り出す場所は `TurnSink::persist` の1箇所だけ。
    TurnSink::new(
        state.store().clone(),
        session_id,
        user_message,
        stats,
        state.extraction(&character),
    )
    .persist(&completion.content)
    .await;

    tracing::info!(
        character = %character.handle,
        session_id = session_id.get(),
        finish_reason = ?completion.finish_reason,
        "応答を返します"
    );

    Ok(Json(ChatCompletionView {
        id: completion_id(),
        object: COMPLETION_OBJECT,
        created: Utc::now().timestamp(),
        model: session_model(&character.handle, session_id),
        choices: vec![ChoiceView {
            index: 0,
            message: MessageView {
                role: ASSISTANT_ROLE,
                // **上流の本文をそのまま返す。** 分解結果は DB にだけ入る（上の doc）。
                content: completion.content,
            },
            finish_reason: finish_reason_to_wire(completion.finish_reason),
        }],
    })
    .into_response())
}

/// `model` の接尾辞に従ってセッションを決める。
///
/// **`SessionRef::Existing` の生の `i64` を `Store::find_session` 以外へ渡さないこと。**
/// `SessionId` が存在する = そのキャラクターの所有確認を通った、が crate 全体の
/// 不変条件（`domain::session` の doc）。ここが唯一の関門で、型は守ってくれない。
///
/// # ここは上流を開くより前に走る（＝失敗した回もセッション行が残る）
///
/// 上流が開けなければこの後 502 で返るので、**`message` が0件の `session` 行だけが残る。**
/// 中継が打ち切った回・人が中止した回も1行も保存しない（ADR-0019 §4）ので、
/// **同じ状態は正常系にもできる。順番を入れ替えても空セッションは無くならない。**
///
/// 空セッションが次の `aoi` を横取りしないことは **`store` 側で担保している**
/// （`SELECT_LATEST_SESSION` は中身のあるセッションを先に見る / 作業単位 B-2b）。
/// 見張り番は `tests/server_chat.rs` の
/// `an_empty_session_left_by_a_failed_turn_does_not_capture_the_bare_handle`。
async fn resolve_session(
    state: &AppState,
    character: &Character,
    session: SessionRef,
) -> Result<SessionId, ApiError> {
    match session {
        // `aoi` — 最新を継続。1つも無ければ作る。
        // 「最新」は**中身のあるセッションのうち `updated_at` が最大**のもの（B-2b）。
        // 空セッションしか無ければ、いちばん新しい空セッションをそのまま使う。
        SessionRef::Latest => Ok(state
            .store()
            .latest_or_create_session(character.id)
            .await
            .map_err(|err| internal(&err, "セッションを解決できませんでした"))?
            .id),
        // `aoi@new` — **必ず新規に作る。** ここを `latest_or_create_session` にすると
        // 「新しい会話を始めたのに前の記憶が続く」形で静かに壊れる。
        SessionRef::New => state
            .store()
            .create_session(character.id)
            .await
            .map_err(|err| internal(&err, "セッションを作成できませんでした")),
        // `aoi@<id>` — **所有確認を通す。** 他キャラのセッションは `None` になる。
        SessionRef::Existing(raw_session_id) => Ok(state
            .store()
            .find_session(character.id, raw_session_id)
            .await
            .map_err(|err| internal(&err, "セッションを取得できませんでした"))?
            .ok_or_else(|| {
                // 生の id は**ログにだけ**出す（応答は定数のみ）。
                tracing::debug!(
                    character = %character.handle,
                    requested_session = raw_session_id,
                    "そのキャラクターのセッションではありません"
                );
                ApiError::session_not_found()
            })?
            .id),
    }
}

/// 採用する発言の探索結果。**「無い」と「形式が未対応」を分ける。**
#[derive(Debug, PartialEq, Eq)]
enum Adopted<'a> {
    /// 最後の `user` の `content` が文字列だった。
    Text(&'a str),
    /// 最後の `user` はあるが、`content` が文字列ではない
    /// （マルチモーダルの配列・`null`・キーごと不在など）。
    UnsupportedContent,
    /// `user` のメッセージが1件も無い。
    Missing,
}

/// 採用する発言 = **最後の `user` メッセージ**。
///
/// クライアントは毎回全履歴を送ってくるが、履歴の権威は自前の DB 側にある
/// （docs/02-architecture.md「履歴の二重管理」）。**先頭ではなく末尾**を採ること。
///
/// # 文字列でなければ、そこで止める
///
/// **より前の `user` へフォールバックしないこと。** 落ちた先の発言に答えると、
/// クライアントから見れば「送っていない古い発言に返事が来た」状態になり、
/// しかも 200 が返るので誰も異常に気づけない。**採用対象は常に最後の `user` 1件**で、
/// それが読めないなら `UnsupportedContent` として拒否する。
///
/// 逆に、**採用しないメッセージの `content` は一切見ない**（`IncomingMessage` の doc）。
fn last_user_message(messages: &[IncomingMessage]) -> Adopted<'_> {
    let Some(message) = messages.iter().rev().find(|m| m.role == USER_ROLE) else {
        return Adopted::Missing;
    };

    match message.content.as_ref().and_then(Value::as_str) {
        Some(text) => Adopted::Text(text),
        None => Adopted::UnsupportedContent,
    }
}

/// 上流へ渡す生成トークン上限。**必ず生成用の余白まで切り下げる。**
///
/// クライアントが未指定なら余白いっぱい。指定していても余白を超えさせない
/// （超えるとプロンプト＋応答が上流のコンテキストを超えうる）。
///
/// **これは上限の話なので 0 を弾けない**（`Some(0)` はそのまま返る）。
/// 0 の拒否は `chat_completions` の入口（`ApiError::invalid_max_tokens`）が担う。
const fn cap_max_tokens(requested: Option<u32>, response_room: u32) -> u32 {
    match requested {
        Some(value) if value < response_room => value,
        _ => response_room,
    }
}

/// 応答の `model` フィールド（`aoi@<session-id>`）。
///
/// **そのまま次のリクエストの `model` として使える形**にする（ADR-0004）。
/// 識別子に `@` は使えないので、この表記は一意に読める。
fn session_model(handle: &Handle, session_id: SessionId) -> String {
    format!("{}@{}", handle.as_str(), session_id.get())
}

/// 応答 id。**内容から導出しない**（推測されても困らないが、意味も持たせない）。
fn completion_id() -> String {
    format!("{COMPLETION_ID_PREFIX}{}", Uuid::new_v4().simple())
}

/// 仕様書の `finish_reason`（作業単位 P-2a）。**値は [`finish_reason_to_wire`] に全部の停止理由を通して作る**
/// （綴りを写さない）。SSE の増分（`openapi::ChunkChoice`）も同じ綴りなので共有する。
pub(super) fn finish_reason_schema() -> Object {
    openapi::nullable_closed_string(
        FinishReason::ALL
            .into_iter()
            .filter_map(|reason| finish_reason_to_wire(Some(reason))),
    )
}

fn completion_object_schema() -> Object {
    openapi::closed_string([COMPLETION_OBJECT])
}

fn assistant_role_schema() -> Object {
    openapi::closed_string([ASSISTANT_ROLE])
}

/// 停止理由の OpenAI 互換の綴り。**中継側と共有する**（綴りを2箇所に置かない）。
///
/// **`FinishReason::Other` は `null` にする。** 上流が返した綴り（`content_filter` /
/// `tool_calls` / 未知の値）は `llm` の型に残っていないので、ここで `"stop"` と
/// 書くと「自然に停止した」という嘘になる。
pub(super) const fn finish_reason_to_wire(reason: Option<FinishReason>) -> Option<&'static str> {
    match reason {
        Some(FinishReason::Stop) => Some("stop"),
        Some(FinishReason::Length) => Some("length"),
        Some(FinishReason::Other) | None => None,
    }
}

/// プロンプトを組み立てられなかった理由を応答へ写す。
///
/// **`internal()` に潰さない。** `BudgetExhausted` は障害ではなく
/// 「設定が予算に対して大きすぎる」という運用者向けの情報で、
/// **理由が分からなければ `PATCH /admin/characters/{handle}` で何を縮めればよいかも
/// 分からない**（CLAUDE.md 絶対制約の背景）。
fn prompt_error(err: PromptError, character: &Handle) -> ApiError {
    match err {
        PromptError::BudgetExhausted => {
            tracing::warn!(
                error = %err,
                character = %character,
                "削れない枠だけでトークン予算を超えました（設定が大きすぎます）"
            );
            ApiError::prompt_budget_exhausted()
        }
    }
}

/// 上流の失敗を応答へ写す。**詳細（ステータス値・本文）はログにだけ出す。**
///
/// `Connect` を他と分けるのは、LM Studio が普段停止している運用だから
/// （docs/04-environment.md）。「起動しているか確認する」という案内は
/// 通信障害の案内とは別物になる。
///
/// **中継が途中で壊れたときもここを通す**（`super::relay::error_data`）。
/// 経路ごとに分けると、同じ失敗が SSE でだけ別の文言になる。
pub(super) fn upstream_error(err: &LlmError) -> ApiError {
    tracing::error!(error = %err, detail = ?err, "推論サーバへの要求に失敗しました");
    match err {
        LlmError::Connect(_) => ApiError::upstream_unavailable(),
        LlmError::Timeout(_) => ApiError::upstream_timeout(),
        // **上流が鍵を断った**（401 / 403 / ADR-0056 §決定4）。直す場所が通信ではなく鍵なので分ける。
        err if err.refused_the_key() => ApiError::upstream_unauthorized(),
        // **送らなかった**（鍵を平文で外へ送る承認が無い / ADR-0056 §決定5）。上流は鍵を見ていない。
        LlmError::KeyNotApproved => ApiError::upstream_key_not_approved(),
        _ => ApiError::upstream_failed(),
    }
}

/// 想定外の失敗。**詳細はログにだけ出す。**
///
/// `Debug` は `#[source]` 経由でパスを含みうるので、出すのはログに限る。
fn internal(err: &StoreError, message: &'static str) -> ApiError {
    tracing::error!(error = %err, detail = ?err, "{}", message);
    ApiError::internal()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// 文字列の `content` を持つメッセージ。
    fn message(role: &str, content: &str) -> IncomingMessage {
        IncomingMessage {
            role: role.to_owned(),
            content: Some(json!(content)),
        }
    }

    /// 文字列**以外**の `content` を持つメッセージ（`null` / 配列 / キー無し）。
    fn message_with(role: &str, content: Option<Value>) -> IncomingMessage {
        IncomingMessage {
            role: role.to_owned(),
            content,
        }
    }

    fn handle(raw: &str) -> Handle {
        Handle::parse(raw).expect("テストで使う名前は正当であること")
    }

    /// **採用するのは最後の `user`。** 先頭でも、最後のメッセージでもない。
    #[test]
    fn the_adopted_message_is_the_last_user_one() {
        let messages = vec![
            message("system", "クライアントのシステム指示"),
            message("user", "最初の発言"),
            message("assistant", "クライアントが持っている履歴"),
            message("user", "最後の発言"),
        ];
        assert_eq!(last_user_message(&messages), Adopted::Text("最後の発言"));

        // 末尾が `user` でなくても、最後の `user` を採る。
        let trailing = vec![
            message("user", "最後の発言"),
            message("assistant", "その後の返事"),
        ];
        assert_eq!(last_user_message(&trailing), Adopted::Text("最後の発言"));
    }

    /// `user` が1件も無ければ `Missing`（空の配列を含む）。
    #[test]
    fn without_a_user_message_there_is_nothing_to_answer() {
        assert_eq!(last_user_message(&[]), Adopted::Missing);
        assert_eq!(
            last_user_message(&[message("system", "指示"), message("assistant", "返事")]),
            Adopted::Missing
        );
        // **役割は完全一致。** 綴りを広げると `store` の綴りと非対称になる。
        assert_eq!(
            last_user_message(&[message("User", "x"), message("USER", "y")]),
            Adopted::Missing
        );
        // 空文字の発言は正当（「何も言わなかった」も入力として扱う）。
        assert_eq!(last_user_message(&[message("user", "")]), Adopted::Text(""));
    }

    /// **採用しないメッセージの `content` は一切見ない。**
    ///
    /// `tool_calls` 付きの `assistant`（`content: null`）や配列形式の `content` が
    /// 混ざっていても、採用する最後の `user` が文字列なら通す。
    #[test]
    fn the_content_of_messages_we_never_adopt_is_not_inspected() {
        let messages = vec![
            message_with("system", Some(json!([{"type": "text", "text": "指示"}]))),
            message_with("assistant", Some(Value::Null)),
            message_with("assistant", None),
            message_with("tool", Some(json!({"output": 1}))),
            message("user", "こんにちは"),
        ];

        assert_eq!(last_user_message(&messages), Adopted::Text("こんにちは"));
    }

    /// 採用対象が文字列でなければ `UnsupportedContent`。
    ///
    /// **より前の `user` へフォールバックしないこと。** 落ちた先の発言に答えると、
    /// クライアントから見れば「送っていない古い発言に 200 で返事が来た」状態になり、
    /// 誰も異常に気づけない。
    #[test]
    fn an_unsupported_content_never_falls_back_to_an_older_message() {
        for content in [
            Some(json!([{"type": "text", "text": "画像つきの発言"}])),
            Some(Value::Null),
            Some(json!(42)),
            Some(json!({"type": "text"})),
            None,
        ] {
            let messages = vec![
                message("user", "ひとつ前の発言"),
                message("assistant", "その返事"),
                message_with("user", content.clone()),
            ];

            assert_eq!(
                last_user_message(&messages),
                Adopted::UnsupportedContent,
                "content = {content:?}"
            );
        }

        // **「1件も無い」とも区別する。**
        assert_ne!(
            last_user_message(&[message_with("user", None)]),
            Adopted::Missing
        );
    }

    /// **上限は必ず余白まで切り下げる。** 未指定なら余白いっぱい。
    #[test]
    fn max_tokens_never_exceeds_the_response_room() {
        assert_eq!(cap_max_tokens(None, 1_024), 1_024, "未指定なら余白いっぱい");
        assert_eq!(cap_max_tokens(Some(16), 1_024), 16, "小さい値はそのまま");
        assert_eq!(
            cap_max_tokens(Some(1_000_000), 1_024),
            1_024,
            "巨大な値は切り下げること"
        );
        assert_eq!(cap_max_tokens(Some(u32::MAX), 1_024), 1_024);
        // 境界。
        assert_eq!(cap_max_tokens(Some(1_023), 1_024), 1_023);
        assert_eq!(cap_max_tokens(Some(1_024), 1_024), 1_024);
        assert_eq!(cap_max_tokens(Some(1_025), 1_024), 1_024);
    }

    /// 応答の `model` は**そのまま次の要求に使える形**（`aoi@<session-id>`）。
    #[test]
    fn the_response_model_points_at_the_session_it_landed_on() {
        let model = session_model(&handle("aoi"), SessionId::from_raw_for_tests(42));
        assert_eq!(model, "aoi@42");

        // 往復する: 返した値をそのまま `model` に書けば同じセッションを指す。
        let selector = ModelSelector::parse(&model).expect("自分が返した値は解釈できること");
        assert_eq!(selector.handle().as_str(), "aoi");
        assert_eq!(selector.session(), SessionRef::Existing(42));

        // `-` / `_` 入りの識別子でも壊れない（`@` の分割に巻き込まれない）。
        let model = session_model(&handle("aoi-v2_7"), SessionId::from_raw_for_tests(7));
        assert_eq!(model, "aoi-v2_7@7");
        let selector = ModelSelector::parse(&model).expect("解釈できること");
        assert_eq!(selector.handle().as_str(), "aoi-v2_7");
        assert_eq!(selector.session(), SessionRef::Existing(7));
    }

    /// 停止理由の綴りは OpenAI 互換。**`Other` を `"stop"` と偽らない。**
    #[test]
    fn finish_reason_uses_the_openai_spelling() {
        assert_eq!(
            finish_reason_to_wire(Some(FinishReason::Stop)),
            Some("stop")
        );
        assert_eq!(
            finish_reason_to_wire(Some(FinishReason::Length)),
            Some("length")
        );
        assert_eq!(
            finish_reason_to_wire(Some(FinishReason::Other)),
            None,
            "綴りが残っていないので「自然に停止した」と偽らないこと"
        );
        assert_eq!(finish_reason_to_wire(None), None);
    }

    /// 応答 id は毎回異なる（クライアントが id で束ねても混ざらない）。
    #[test]
    fn completion_ids_are_unique_and_prefixed() {
        let first = completion_id();
        let second = completion_id();
        assert!(first.starts_with(COMPLETION_ID_PREFIX), "{first}");
        assert_ne!(first, second);
    }

    /// 応答の形。**`finish_reason` は `null` でもキーごと落とさない。**
    #[test]
    fn the_response_shape_is_openai_compatible() {
        let view = ChatCompletionView {
            id: "chatcmpl-1".to_owned(),
            object: COMPLETION_OBJECT,
            created: 1_760_000_000,
            model: "aoi@42".to_owned(),
            choices: vec![ChoiceView {
                index: 0,
                message: MessageView {
                    role: ASSISTANT_ROLE,
                    content: "「こんにちは」".to_owned(),
                },
                finish_reason: None,
            }],
        };

        assert_eq!(
            serde_json::to_value(view).expect("JSON にできること"),
            json!({
                "id": "chatcmpl-1",
                "object": "chat.completion",
                "created": 1_760_000_000,
                "model": "aoi@42",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "「こんにちは」"},
                    "finish_reason": null,
                }],
            })
        );
    }

    /// **未知のフィールドを拒否しない**（OpenAI 互換の境界。`/admin/` とは方針が違う）。
    #[test]
    fn unknown_fields_are_accepted() {
        let request: ChatCompletionRequest = serde_json::from_value(json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "こんにちは", "name": "someone"}],
            "top_p": 0.9,
            "presence_penalty": 0.0,
            "stop": ["\n"],
            "user": "someone",
        }))
        .expect("実クライアントの余分な項目で落ちないこと");

        assert_eq!(request.model, "aoi");
        assert_eq!(
            last_user_message(&request.messages),
            Adopted::Text("こんにちは")
        );
        assert_eq!(request.temperature, None);
        assert_eq!(request.max_tokens, None);
        assert_eq!(request.stream, None);
    }

    /// `stream` は**未指定と `false` を区別しない**（どちらも非ストリーミング）。
    #[test]
    fn stream_defaults_to_non_streaming() {
        let without: ChatCompletionRequest =
            serde_json::from_value(json!({"model": "aoi", "messages": []}))
                .expect("解釈できること");
        assert_eq!(without.stream, None);

        let explicit: ChatCompletionRequest =
            serde_json::from_value(json!({"model": "aoi", "messages": [], "stream": true}))
                .expect("解釈できること");
        assert_eq!(explicit.stream, Some(true));
    }
}
