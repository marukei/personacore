//! 上流の応答の型と、**panic しない**解釈。
//!
//! ここは `llm` の中で唯一の純粋関数の島で、入力は**すべて信頼できない**
//! （上流 LLM が返した文字列）。どんな形が来ても `LlmError` に落ちるだけで、
//! `unwrap` / `expect` / 添字アクセスによる panic 経路を作らない。
//!
//! 取り出した文字列を SQL・シェル・パスへ渡す API は**ここには生やさない**。
//! 値はただのデータとして上位へ返し、扱いは呼び出し側（`store` はプレースホルダ、
//! `parse` は台詞／地の文の分解）に委ねる。

use serde::Deserialize;
use serde_json::Value;

use super::error::LlmError;

/// ストリーム終端を表す番兵。OpenAI 互換サーバが最後に送る。
const DONE_MARKER: &str = "[DONE]";

/// `/api/v0/models` の `state` が「実際にロード済み」を示す綴り。**完全一致。**
const LOADED_STATE: &str = "loaded";

/// OpenAI 互換の `/v1/models` の `meta` で、**いま載っている窓**を持つ項目
/// （`llama-server` / 作業単位 G-2）。
///
/// **`n_ctx_train`（訓練時の上限 / 実測 262144）ではない。**
/// 取り違えると本文の予算が 32 倍になり、窓を超えたプロンプトを送る。llama は 400
/// `exceed_context_size_error` で断る（ADR-0052 M5。LM Studio なら前方から黙って切り捨てられる
/// / ADR-0013）。ただし、プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる
/// （`finish_reason=length`）。
const EFFECTIVE_CONTEXT_FIELD: &str = "n_ctx";

/// `usage` の中で**プロンプトとして数えられたトークン数**を持つ項目（OpenAI 互換）。
const PROMPT_TOKENS_FIELD: &str = "prompt_tokens";

/// `usage` の中で**生成したトークン数**を持つ項目（OpenAI 互換 / ADR-0059 §決定3）。
const COMPLETION_TOKENS_FIELD: &str = "completion_tokens";

/// 生成が止まった理由。
///
/// **未知の値をエラーにしない。** 上流の実装差でここが増えても、中継そのものは
/// 成立させたい（`Other` に落とす）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FinishReason {
    /// 自然に停止した。
    Stop,
    /// トークン上限で打ち切られた。**本文が途中で切れている**ので、
    /// 上位はこれを見て再生成や `max_tokens` の調整を判断できる。
    Length,
    /// 上記以外（`content_filter` / `tool_calls` / 未知の値）。
    Other,
}

impl FinishReason {
    /// **全部の値**（作業単位 P-2a —— 仕様書が `finish_reason` の `enum` を
    /// `server::chat::finish_reason_to_wire` に通してここから作る）。
    /// 数え漏れは `every_finish_reason_variant_is_listed_in_all` が見る。
    pub const ALL: [Self; 3] = [Self::Stop, Self::Length, Self::Other];

    /// JSON の文字列から。**未知の綴りは `Other`**（エラーにしない）。
    fn from_wire(raw: &str) -> Self {
        match raw {
            "stop" => Self::Stop,
            "length" => Self::Length,
            _ => Self::Other,
        }
    }
}

/// 上流が申告した使用トークン数（ADR-0013 第3層）。
///
/// **これは実際に答えたモデル自身が数えた真値**で、こちらの見積もり
/// （`prompt::estimate_tokens` = UTF-8 バイト数）の答え合わせに使う。
///
/// # 答え合わせに使うのは `prompt_tokens` だけ
///
/// 第3層が見るのは「**送った側の見積もりが上界だったか**」なので、
/// 突き合わせる相手はプロンプト側の実測1つで足りる。`total_tokens` は持たない。
///
/// **`completion_tokens` は答え合わせに使わない。** 持つのは、空の本文を引き直した回に
/// **捨てた1回目の生成数**を分解の行に出すためだけ（ADR-0059 §決定3 / 作業単位 A-18b）。
/// **`TokenUsage` が在るかはプロンプトの数だけで決まる**（生成数が読めなくても `None` にしない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenUsage {
    /// プロンプトとして数えられたトークン数。**必ず 1 以上**（`parse_usage` の doc）。
    pub prompt_tokens: u32,
    /// 上流が生成したトークン数（思考を含みうる）。**読めなければ `None`**（0 は正当）。
    pub completion_tokens: Option<u32>,
}

/// 非ストリーミングの補完結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// 生成された本文。**空文字も正当**（「何も言わなかった」は事実として扱う）。
    pub content: String,
    /// 停止理由。上流が示さなければ `None`。
    pub finish_reason: Option<FinishReason>,
    /// 上流が数えたトークン数。**`None` は「答え合わせできない」であって異常ではない**
    /// （`usage` を返さない上流の実装がある）。
    pub usage: Option<TokenUsage>,
}

/// ストリーミングの1チャンク。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatChunk {
    /// 増分。役割だけを載せた先頭チャンクなどでは空文字になる。
    pub delta: String,
    /// 停止理由。最後のチャンク以外は `None`。
    pub finish_reason: Option<FinishReason>,
    /// 上流が数えたトークン数（ADR-0013 第3層）。
    ///
    /// **ほぼ常に `None`。** ストリーミングでは `usage` が**最終チャンクにしか
    /// 載らない**（載せない実装もある）。`None` は「答え合わせできない」であって
    /// 異常ではないので、**欠落で警告を出さないこと**（`Completion::usage` と同じ）。
    pub usage: Option<TokenUsage>,
}

// ---- 以下、JSON の写し取り。**`#[serde(default)]` を外さないこと。** ----
// 欠けている項目でエラーにすると、上流の実装差でストリームごと落ちる。

#[derive(Debug, Deserialize)]
struct WireCompletion {
    #[serde(default)]
    choices: Vec<WireChoice>,
    /// **構造体に写し取らず `Value` で受けること。** 写し取ると
    /// `usage` 側の型違い1つ（`"prompt_tokens":"123"` など）で本文まるごとが
    /// `Decode` に落ち、**答え合わせのために応答を失う**という本末転倒が起きる。
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct WireChoice {
    #[serde(default)]
    message: Option<WireContent>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireChunk {
    #[serde(default)]
    choices: Vec<WireChunkChoice>,
    /// **構造体に写し取らず `Value` で受けること**（`WireCompletion::usage` と同じ理由）。
    /// 写し取ると `usage` 側の型違い1つで**そのチャンクの本文まで失う。**
    #[serde(default)]
    usage: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct WireChunkChoice {
    #[serde(default)]
    delta: Option<WireContent>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireContent {
    #[serde(default)]
    content: Option<String>,
}

/// `data:` の中身が終端の番兵か。
///
/// 前後の空白を許すのは、SSE の `data: [DONE]` と `data:[DONE]` の両方が来うるため。
pub(super) fn is_done(data: &str) -> bool {
    data.trim() == DONE_MARKER
}

/// 非ストリーミング応答の本文を解釈する。
///
/// `choices` が空、または `content` が無ければ `MissingContent`
/// （`Decode` とは分ける。前者はプロンプトや上流設定の問題を示唆する）。
pub(super) fn parse_completion(body: &str) -> Result<Completion, LlmError> {
    let wire: WireCompletion = serde_json::from_str(body).map_err(|err| {
        // 本文は Display に載せられないので、調査の手掛かりはログにしか残らない。
        tracing::warn!(error = %err, "上流の応答を JSON として解釈できませんでした");
        LlmError::Decode(err)
    })?;

    if wire.choices.len() > 1 {
        // n > 1 は要求していない。来たら先頭だけ使う（黙って捨てない）。
        tracing::debug!(count = wire.choices.len(), "上流が複数の候補を返しました");
    }

    // **`choices` を消費する前に取り出す。** 順序を入れ替えると部分ムーブで詰まる。
    let usage = parse_usage(wire.usage.as_ref());

    let Some(choice) = wire.choices.into_iter().next() else {
        tracing::warn!("上流の応答に choices がありません");
        return Err(LlmError::MissingContent);
    };
    let Some(content) = choice.message.and_then(|message| message.content) else {
        tracing::warn!("上流の応答に content がありません");
        return Err(LlmError::MissingContent);
    };

    Ok(Completion {
        content,
        finish_reason: choice.finish_reason.as_deref().map(FinishReason::from_wire),
        usage,
    })
}

/// `usage` から**プロンプトのトークン数**を取り出す（ADR-0013 第3層）。
///
/// # `None` は異常ではない
///
/// `usage` を返さない上流の実装があるので、欠落は「**答え合わせできない**」以上の
/// 意味を持たない。**ここで `warn` を出さないこと。** 毎リクエスト鳴る警告は、
/// 鳴っていること自体の意味を消す（docs/06-roadmap.md の既知の問題と同じ轍）。
///
/// # `0` を信じないこと
///
/// 削れない枠（システムプロンプト＋今回の発言）が必ず載るので、プロンプトが
/// 0 トークンということはありえない。受け入れると「見積もり ≥ 実トークン」が
/// **常に成立して見え**、第3層が黙って死ぬ。欠落と同じ扱いにする。
///
/// 型違い・負値・u32 に収まらない値のいずれでも panic しない
/// （`parse_loaded_context_length` と同じ作法）。
fn parse_usage(usage: Option<&Value>) -> Option<TokenUsage> {
    // `usage` そのものがオブジェクトでなければ `get` が `None` を返す。
    let reported = usage?.get(PROMPT_TOKENS_FIELD)?;

    // `as_u64` は文字列・小数・負数・null をすべて `None` にする。
    match reported
        .as_u64()
        .and_then(|raw| u32::try_from(raw).ok())
        .filter(|tokens| *tokens > 0)
    {
        Some(prompt_tokens) => Some(TokenUsage {
            prompt_tokens,
            // **プロンプトの数を巻き込まない。** 読めなければ生成数だけ `None`。
            completion_tokens: usage?
                .get(COMPLETION_TOKENS_FIELD)
                .and_then(Value::as_u64)
                .and_then(|raw| u32::try_from(raw).ok()),
        }),
        None => {
            // **項目はあるのに読めない**ケースだけがここに来る（欠落は上で抜けている）。
            tracing::debug!(
                reported = %reported,
                "上流が申告したプロンプトのトークン数を解釈できません"
            );
            None
        }
    }
}

/// ストリーミングの1イベントを解釈する。
///
/// 戻り値の `None` は「**流すべき中身が無いので飛ばす**」の意味。
/// `choices` が空のチャンクは上流の実装差で普通に来るので、
/// エラーにせず読み飛ばす（ここでエラーにすると本文が途中で死ぬ）。
/// **ただし読める `usage` があれば読み飛ばさない**（下記）。
///
/// 一方で **`data:` 自体が空白のみ**なら `EmptyPayload`。上流の異常であり、
/// 黙って飲み込むと「なぜ応答が空なのか」が追えなくなる。
///
/// # 使用量だけのチャンクは読み飛ばさない
///
/// `stream_options.include_usage` を持つ上流は、本文を出し切った後に
/// **`choices` が空で `usage` だけ**のチャンクを送る。読み取れる `usage` があれば
/// **空の増分を持つチャンクとして返す**（ADR-0013 第3層は、ストリーミングでは
/// ここからしか真値を得られない）。読み取れなければ従来どおり読み飛ばす。
pub(super) fn parse_chunk(data: &str) -> Result<Option<ChatChunk>, LlmError> {
    if data.trim().is_empty() {
        tracing::warn!("上流が空のチャンクを送りました");
        return Err(LlmError::EmptyPayload);
    }

    let wire: WireChunk = serde_json::from_str(data).map_err(|err| {
        tracing::warn!(error = %err, "上流のチャンクを JSON として解釈できませんでした");
        LlmError::Decode(err)
    })?;

    // **`choices` を消費する前に取り出す**（`parse_completion` と同じ理由）。
    let usage = parse_usage(wire.usage.as_ref());

    let Some(choice) = wire.choices.into_iter().next() else {
        if usage.is_some() {
            tracing::debug!("上流が使用量だけのチャンクを送りました");
            return Ok(Some(ChatChunk {
                delta: String::new(),
                finish_reason: None,
                usage,
            }));
        }
        tracing::trace!("中身の無いチャンクを読み飛ばしました");
        return Ok(None);
    };

    Ok(Some(ChatChunk {
        delta: choice
            .delta
            .and_then(|delta| delta.content)
            .unwrap_or_default(),
        finish_reason: choice.finish_reason.as_deref().map(FinishReason::from_wire),
        usage,
    }))
}

/// LM Studio 独自 API（`/api/v0/models`）の応答から、
/// **そのモデルに実際にロードされているコンテキスト長**を取り出す。
///
/// # `max_context_length` を読まないこと
///
/// 一覧にはモデルが取りうる最大値（実測 262144）も並ぶが、実際に使えるのは
/// ロード時に決まった `loaded_context_length`（実測 16384）だけ。
/// 最大値を信じて窓を超えると、**LM Studio ではプロンプトが前方から黙って切り捨てられる**
/// ——エラーも出ず応答も返るので、最も気付けない壊れ方をする（ADR-0013）。
/// llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。ただし、
/// プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
///
/// # 上流の JSON を信用しない
///
/// `serde` の構造体に写し取らず `Value` を辿るのは、**無関係な行の型違いで
/// 全体が落ちないようにする**ため（一覧には未ロードのモデルや埋め込みモデルも並ぶ）。
/// 欠落・型違い・`0`・u32 に収まらない値のいずれも panic せず
/// `MissingContextLength` に落ち、呼び出し側が既定値へ退避できる。
pub(super) fn parse_loaded_context_length(body: &str, model: &str) -> Result<u32, LlmError> {
    let wire: Value = serde_json::from_str(body).map_err(|err| {
        // 本文は Display に載せられないので、調査の手掛かりはログにしか残らない。
        tracing::warn!(error = %err, "上流のモデル一覧を JSON として解釈できませんでした");
        LlmError::Decode(err)
    })?;

    let Some(entries) = wire.get("data").and_then(Value::as_array) else {
        tracing::warn!("上流のモデル一覧に data がありません");
        return Err(LlmError::MissingContextLength);
    };

    // **完全一致で引く。** 前方一致にすると別サイズのモデルを掴みうる。
    let Some(entry) = entries
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(model))
    else {
        tracing::warn!(model, "上流のモデル一覧に設定中のモデルがありません");
        return Err(LlmError::MissingContextLength);
    };

    let state = entry.get("state").and_then(Value::as_str);
    if state != Some(LOADED_STATE) {
        tracing::warn!(model, state, "設定中のモデルはロードされていません");
        return Err(LlmError::MissingContextLength);
    }

    // `as_u64` は文字列・小数・負数・null をすべて `None` にする。
    let Some(raw) = entry.get("loaded_context_length").and_then(Value::as_u64) else {
        tracing::warn!(model, "ロード済みのコンテキスト長が読めません");
        return Err(LlmError::MissingContextLength);
    };

    // 0 は予算として成立しない。u32 を超える値は「正しく報告された」とは考えない。
    match u32::try_from(raw) {
        Ok(tokens) if tokens > 0 => Ok(tokens),
        _ => {
            tracing::warn!(model, raw, "ロード済みのコンテキスト長が不正です");
            Err(LlmError::MissingContextLength)
        }
    }
}

/// **どうやってその行を同定したか**（作業単位 G-2）。
///
/// **2つは危険の度合いが違う。** 前者は上流が名乗った綴りとの一致で、推測が無い。
/// 後者は「**答えられるモデルが1つしか無い**」という推論で、
/// その前提が崩れれば別のモデルの窓を読むことになる。**ログで区別が付くこと。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModelMatch {
    /// `id` が設定中のモデルと完全一致した。
    ExactId,
    /// 一覧が1件しか無かった（＝**答えるモデルはそれしかない**）。
    SoleEntry,
}

impl ModelMatch {
    /// ログに載せる綴り。
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::ExactId => "exact-id",
            Self::SoleEntry => "sole-entry",
        }
    }
}

/// **モデル一覧を、どの読み手が答えたか**（作業単位 G-3b）。
///
/// # なぜ値を持たない列挙なのか
///
/// **同定した行の `id` は、`llama-server` では gguf の絶対パス**である。
/// **絶対制約: エラーレスポンスに内部パスを含めない**（CLAUDE.md）。
/// この観測は `AppState` が持ち続け、`GET /admin/upstream`（作業単位 G-10）が
/// HTTP へ出す予定なので、**綴りをここに載せると、そのまま外へ出る道ができる。**
///
/// **必要なのは「どう同定したか」までで、綴りそのものではない。**
/// 綴りは `llm::lmstudio::compat_models` の `info` の1行（`upstream_model_id`）に残る
/// ——**ログには出してよい。**
///
/// **⚠ 「型で表現できなくした」とは書けない。** 持ち上げる形は少なくとも2通り書ける ——
/// [`crate::llm::ModelsReport`] に `model_id: String` を足す形と、
/// **同じ綴りが並ぶ一覧（`catalogue`）をそのまま観測に残す形。**
/// **後者は実際に書いて測った**（変異 `m4` / **2 赤**。**前者は書いていない**）。
/// **止めているのは型ではなく見張り番**
/// （`tests/server_upstream_facts.rs::the_held_facts_never_carry_the_upstreams_own_model_id`
/// と `crate::server` の `upstream::tests::the_catalogue_the_upstream_listed_is_not_kept`。
/// **`m4_notest` で、この2本を外すと緑に戻ることまで測ってある**）。
///
/// **⚠ その2本が言えるのは「いまの形の `Debug` に綴りが出ない」までである。**
/// **限界と、`G-10` への申し送りは [`crate::server`] の `upstream` モジュールの doc**
/// （同じ発見を2箇所に書かない / CLAUDE.md）。
///
/// # なぜ `lmstudio` ではなく、この継ぎ目のファイルに置くのか
///
/// **`llm` の外へ出てよい綴りは、継ぎ目の面に限る**
/// （`crate::llm` の `upstream_leaks` / ADR-0036 §決定4）。
/// **この型は `server` が持ち続ける**ので、`lmstudio.rs` に置くと
/// **上流固有の綴りが `llm` の外へ出た**ことになり、帳簿（`EXPECTED_LEAKS`）が増える。
///
/// **増やさずに済むのは、これが「上流の応答をどう読んだか」の値型だから**
/// ——[`ModelMatch`] と同じ層の話で、`Completion` / `TokenUsage` と同じ扱いになる。
/// **腕の綴りは LM Studio と OpenAI 互換の2通りを名指ししている**が、
/// **別の上流を足すときに増やせばよい列挙**であって、特定の上流の型ではない。
///
/// **⚠ この判断は G-3b で下したもので、レビューが覆すなら
/// 置き場所ではなく `EXPECTED_LEAKS` を増やす向きになる**（＝ADR-0036 §決定4 の変更）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsweredBy {
    /// **上流自身の、OpenAI 互換ではない API が 2xx を返した。**
    ///
    /// **いまの上流では `/api/v0/models`**（LM Studio 独自）。
    /// **これは「LM Studio の型」ではなく分類である**——
    /// 綴りも上流の名前を名乗らない（`"native-api"`）ので、
    /// **別の上流が自分の API で答えたときも、この腕のままでよい。**
    /// **上流の名前で腕を増やすのは、この型を置いた理由（下の節）と噛み合わない。**
    NativeApi,
    /// **OpenAI 互換**（`/v1/models`）が答え、
    /// `id` が設定中の綴りと**完全一致**する行を読んだ。
    CompatExactId,
    /// OpenAI 互換が答え、**一覧が1件しか無かった**のでその行を読んだ。
    ///
    /// **推論が1つ入っている**（「答えられるモデルが1つなら、答えるのはそれ」）。
    /// [`Self::CompatExactId`] と潰さないこと——危険の度合いが違う。
    CompatSoleEntry,
    /// OpenAI 互換が**綴りの一覧だけ**を返した（実効コンテキスト長の行を同定できなかった）。
    CompatCatalogueOnly,
}

impl AnsweredBy {
    /// **全部の値**（作業単位 P-2a —— 仕様書が `answered_by` の `enum` をここから作る）。
    /// 数え漏れは `every_answered_by_variant_is_listed_in_all` が見る。
    pub const ALL: [Self; 4] = [
        Self::NativeApi,
        Self::CompatExactId,
        Self::CompatSoleEntry,
        Self::CompatCatalogueOnly,
    ];

    /// ログと応答に載せる綴り。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::NativeApi => "native-api",
            Self::CompatExactId => "openai-exact-id",
            Self::CompatSoleEntry => "openai-sole-entry",
            Self::CompatCatalogueOnly => "openai-catalogue-only",
        }
    }

    /// [`ModelMatch`] から。**綴りは写さない**（この型の doc）。
    pub(super) const fn from_match(matched: ModelMatch) -> Self {
        match matched {
            ModelMatch::ExactId => Self::CompatExactId,
            ModelMatch::SoleEntry => Self::CompatSoleEntry,
        }
    }
}

/// **OpenAI 互換の一覧から読み取った実効コンテキスト長**（作業単位 G-2）。
///
/// **`model_id` は上流が名乗った綴り**で、`llama-server` では **gguf の絶対パス**。
/// **ログにだけ出す**（エラーレスポンスに載せない / CLAUDE.md 絶対制約）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EffectiveContext {
    /// いま載っている窓（`meta.n_ctx`）。
    pub(super) tokens: u32,
    /// 実際に読んだ行の `id`。
    pub(super) model_id: String,
    /// その行をどう同定したか。
    pub(super) matched: ModelMatch,
}

/// **OpenAI 互換の `/v1/models` から、いま載っている実効コンテキスト長を読む**
/// （作業単位 G-2 / `llama-server` を 2026-09-04 に実測）。
///
/// # `n_ctx_train` を読まないこと
///
/// `meta` には**訓練時の上限**（実測 262144）も並ぶが、実際に使えるのは
/// **いまロードされている窓**（`meta.n_ctx` / 実測 8192・16384）だけ。
/// `parse_loaded_context_length` の `max_context_length` と**同じ罠**で、
/// **32 倍**の差がある。取り違えると窓を超えたプロンプトを送る。llama は 400
/// `exceed_context_size_error` で断る（ADR-0052 M5。LM Studio なら前方から黙って切り捨てられる
/// / ADR-0013）。ただし、プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる
/// （`finish_reason=length`）。
///
/// # 綴りでは引けない（**同定の規則**）
///
/// **`llama-server` の `id` は gguf の絶対パス**で、設定中のモデルの綴り
/// （`ministral-3-14b-instruct-2512`）とは**永久に一致しない。**
/// そこで2段にする。**上ほど安全な順で、下へは落ちるが横へは行かない。**
///
/// | 段 | 条件 | 根拠 |
/// |---|---|---|
/// | 1 | `id` が設定中のモデルと完全一致する行がある | 上流が自分でその綴りを名乗った。推測が無い |
/// | 2 | 一致が無く、**一覧が1件しか無い** | **答えられるモデルが1つしかないなら、答えるのはそれ** |
///
/// **1段目で見つけた行に長さが無くても、2段目へは行かない**
/// ——同定はできているのだから、答えは「取れなかった」であって「隣の値」ではない。
///
/// > **⚠ これは「意図」であって、単独では守られていない**（変異検査で実測 / G-2）。
/// > **2段目へ落ちる形に書き換えても 1932 全緑**だった。
/// > **ただし「等価」は無条件ではない。寄りかかっているのは
/// > 「2段目の入口が、一覧そのものが1件であること」の1点だけ**
/// > ——それが成り立つ限り、落ちた先は必ず同じ行か、拒否になる。
/// >
/// > **⚠ 2段目を「長さを持つ行が1件」に緩める変異と組にすると、等価ではなくなる**
/// > （CLAUDE.md「**第5の分類 —— 2つの変異が互いを隠し合う**」）——
/// > **一致した行に長さが無いとき、隣のモデルの窓を `sole-entry` として掴む。**
/// > **この doc が「そもそも表現できない」と書いている状態が、変異2つで作れる。**
/// > **相方の変異は `a_list_with_several_models_and_no_match_is_refused` が赤にする**
/// > （`meta` を持つ行が1件しかない多件の一覧 / G-2 の是正）ので、
/// > **組を同時に入れることはできない。**
///
/// # ⚠ 「1件しか無ければそれを採る」の危険を、どう抑えたか
///
/// **間違ったモデルの窓を本文の予算にすると壊れる。** この経路の上流は llama（`meta` を返す）で、
/// 窓を**大きく**取り違えれば、400 `exceed_context_size_error` で断られるか、
/// プロンプト ＋ 応答の上限が窓を超えて応答が黙って短くなる（`finish_reason=length` /
/// ADR-0052 M5）。**小さく**取り違えれば窓が余るだけ。**どちらでも前方からの切り捨ては起きない**
/// （それは LM Studio で測った話 / ADR-0013）。
/// 抑えているのは**「一覧そのものが1件」を条件にしていること**で、
/// 「長さを持つ行が1件」ではない。**後者だと、`meta` を持たない別のモデルが
/// 答える上流で、居もしないモデルの窓を掴む。**
///
/// **見張り番は `a_list_with_several_models_and_no_match_is_refused`**
/// （`ONLY_ONE_HAS_A_LENGTH` の腕）。**この2つの条件を区別できる形は、そこにしか無い。**
///
/// **2件以上あって一致もしないなら、読まない。** 読まなければ落とし所へ落ちるだけで、
/// **遅くなるだけ・壊れない**側に倒れる（ADR-0036 §決定1 の判定の問い）。
///
/// # 読めなかったことを、この層は警告にしない（作業単位 G-8 ② / G-9）
///
/// **「上流が長さを名乗らない」は上流の性質であって、故障ではない。**
/// LM Studio の `/v1/models` は `meta` を持たず（G-2 が実測 / 2026-09-04）、
/// `meta` を持たない OpenAI 互換シム（vLLM / Ollama）も同じ形になる。
/// **`warn` にすると、起動のたびと5分ごとの引き直しのたびに鳴る**
/// （ADR-0020 の引き直し / `crate::measured` の doc「毎回鳴る警告は読まれなくなる」）。
///
/// **深刻度は「何が失われたか」を知っている側が決める**（作業単位 G-5 の判断基準）。
/// **長さが読めなかった結果は、必ずどちらか1本が `warn` で言う** ——
///
/// | 綴りの一覧は取れたか | `warn` を出す層 | 文面 |
/// |---|---|---|
/// | 取れた | `server::context` | 「落とし所を使います」/「既定値を使います」 |
/// | 取れなかった | `llm::lmstudio` | 「どちらの綴りでも読めませんでした」 |
///
/// **この層が足すと、同じ事実が2本になる**
/// （`LmStudioClient::compat_models` の doc が着手前からそう書いていた）。
///
/// # ⚠ 黙るのは「答えを持っていない」側だけ
///
/// **「答えが壊れている」側は `warn` のまま。** 正常な上流では起きないので鳴り続けない。
///
/// | 綴り | 水準 | 分類 |
/// |---|---|---|
/// | 同定できない（多件・一致なし） | `debug` | 答えを持っていない |
/// | 同定はできたが `meta.n_ctx` が無い | `debug` | 答えを持っていない |
/// | `meta.n_ctx` が `null` | `debug` | 答えを持っていない |
/// | **`meta.n_ctx` は在るが `u64` として読めない**（文字列・負数・小数） | **`warn`** | **答えが壊れている** |
/// | JSON として読めない / `data` が無い | `warn` | 答えが壊れている |
/// | 項目に `id` が無い | `warn` | 答えが壊れている |
/// | 長さが `0` / u32 を超える | `warn` | 答えが壊れている |
///
/// 見張り番は `tests::a_list_that_does_not_name_a_length_stays_out_of_the_default_log_filter`
/// と `tests::an_upstream_that_answers_something_broken_is_still_a_warning`
/// ——**境目の両側を測らないと、全部 `debug` へ落とす変異が緑を通る。**
///
/// # 第3の状態は、G-9 のレビューが実測して見つけた（2026-09-07 に是正）
///
/// `Value::as_u64` は**文字列・負数・小数・`null` をすべて `None`** にするので、
/// **是正前は「答えが壊れている」側の3つが「長さが無い」の枝（`debug`）へ落ちていた。**
/// **実測: `{"id":"upstream-only-id","meta":{"n_ctx":-1}}` を既定のフィルタで回すと、
/// この層の捕捉が空文字列**（1行も出ない）。
///
/// **⚠ 「失われるのは理由の水準だけ」と書いていたが、失われるのは理由そのものだった。**
/// `server::context` の `warn`（「落とし所を使います」）は、上流が**何も答えなかった**のか
/// **壊れた型で答えた**のかを区別しない。**区別できる唯一の行がこの層のもの**なので、
/// そこを `debug` へ落とすと、既定のフィルタでは両者が同じに見える。
///
/// **`read_effective_entry` で欄の有無を先に分け、在るのに読めない側は
/// 既存の「不正です」の `warn` へ合流させた**（ADR-0039 の境目
/// 「異常について言っている警告は残す。異常でないと言っている警告だけ下げる」）。
///
/// # 上流の JSON を信用しない
///
/// `parse_loaded_context_length` と同じ作法。欠落・型違い・`0`・u32 に収まらない値の
/// いずれも panic せず `MissingContextLength` に落ち、呼び出し側が既定値へ退避できる。
pub(super) fn parse_effective_context_length(
    body: &str,
    model: &str,
) -> Result<EffectiveContext, LlmError> {
    let wire: Value = serde_json::from_str(body).map_err(|err| {
        // 本文は Display に載せられないので、調査の手掛かりはログにしか残らない。
        tracing::warn!(error = %err, "上流の OpenAI 互換モデル一覧を JSON として解釈できませんでした");
        LlmError::Decode(err)
    })?;

    let Some(entries) = wire.get("data").and_then(Value::as_array) else {
        tracing::warn!("上流の OpenAI 互換モデル一覧に data がありません");
        return Err(LlmError::MissingContextLength);
    };

    // 1段目。**完全一致があるなら、そこで決める**（見つかった行に長さが無くても落ちない）。
    if let Some(entry) = entries
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(model))
    {
        return read_effective_entry(entry, model, ModelMatch::ExactId);
    }

    // 2段目。**一覧そのものが1件のときだけ。** 2件以上なら、どれが答えるか分からない。
    //
    // **⚠ ここは `debug`。`warn` に戻さないこと**（作業単位 G-8 ②）——
    // **多件の一覧を返す OpenAI 互換の上流では、これが正常な経路**である
    // （設定中の綴りを名乗らない上流はいくらでもある）。
    // **`warn` にすると起動のたび＋5分ごとの引き直しのたびに鳴る**（ADR-0020）。
    // 深刻度は「何が失われたか」を知っている側が決める（この関数の doc の
    // 「# 読めなかったことを、この層は警告にしない」）。
    let [only] = entries.as_slice() else {
        tracing::debug!(
            model,
            entries = entries.len(),
            "OpenAI 互換のモデル一覧から実効コンテキスト長を同定できません"
        );
        return Err(LlmError::MissingContextLength);
    };
    read_effective_entry(only, model, ModelMatch::SoleEntry)
}

/// 同定できた1行から `meta.n_ctx` を読む。**読めなければ `MissingContextLength`。**
fn read_effective_entry(
    entry: &Value,
    model: &str,
    matched: ModelMatch,
) -> Result<EffectiveContext, LlmError> {
    // **読んだ行を名乗れないなら採らない。** ログに出す `id` が無い状態で
    // 予算を決めると、どの行を読んだのかが後から辿れない。
    let Some(id) = entry.get("id").and_then(Value::as_str) else {
        tracing::warn!(model, "OpenAI 互換のモデル一覧の項目に id がありません");
        return Err(LlmError::MissingContextLength);
    };

    // **`n_ctx_train`（訓練時の上限）ではない。** 読むのは「いま載っている窓」。
    //
    // **欄が在るかどうかで、まず分ける**（作業単位 G-9 の是正 / 2026-09-07）。
    // **`null` は「欄が無い」側に数える** ——`n_ctx: null` と名乗る上流は
    // 「長さを持っていない」と言っているのであって、壊れているのではない。
    //
    // **根拠（G-9 のレビューの求めで明記 / 3周目）: `serde` で `Option<u32>` を
    // 素直に直列化すると `null` になる。** `#[serde(skip_serializing_if = "Option::is_none")]`
    // を付けていない実装は全部そうなるので、**「長さを持っていない上流」が
    // 現に吐きうる綴り**である。**故障の綴りではない。**
    let field = entry
        .get("meta")
        .and_then(|meta| meta.get(EFFECTIVE_CONTEXT_FIELD))
        .filter(|value| !value.is_null());

    let Some(field) = field else {
        // **⚠ ここは `debug`。`warn` に戻さないこと**（作業単位 G-9）——
        // **`meta` を持たない OpenAI 互換の上流**（LM Studio の `/v1/models`／
        // vLLM / Ollama のシム）**では、これが起動のたびに通る正常な経路**である。
        // 深刻度は呼び出し側が決める（この関数の doc の
        // 「# 読めなかったことを、この層は警告にしない」）。
        tracing::debug!(
            model,
            upstream_model_id = id,
            "OpenAI 互換のモデル一覧に実効コンテキスト長がありません"
        );
        return Err(LlmError::MissingContextLength);
    };

    // **欄が在るなら、読めないのは上流の故障。** 3つとも同じ1本の `warn` へ合流させる:
    //
    // * `u64` として読めない（**文字列・負数・小数** / `Value::as_u64` が `None`）。
    // * `0`（予算として成立しない）。
    // * u32 を超える（「正しく報告された」とは考えない）。
    //
    // **⚠ 枝を割って水準を分けないこと**（CLAUDE.md / 作業単位 B-6）——
    // **同じ「答えが壊れている」の権威が2箇所に分かれ、片方が観測されなくなる。**
    // 見張り番は `tests::an_upstream_that_answers_something_broken_is_still_a_warning`
    // （**読めない綴りを1つずつ回している**。**代表1つでは足りない** / 作業単位 D-6）。
    let Some(tokens) = field
        .as_u64()
        .and_then(|raw| u32::try_from(raw).ok())
        .filter(|tokens| *tokens > 0)
    else {
        tracing::warn!(
            model,
            upstream_model_id = id,
            // **`Display` は JSON の綴りそのもの**（`"8192"` は引用符ごと出る）。
            // **型が違ったことが、この1行の中で読めること。**
            // 見張り番は `tests::an_upstream_that_answers_something_broken_is_still_a_warning`
            // の `raw` の欄（**読めない綴りごとに、名指しで突き合わせている**）。
            //
            // **⚠ この欄に上限は無い。** 上流が名乗った JSON の値がそのまま1行に載る
            // ので、`meta.n_ctx` に巨大な文字列や配列を入れた上流なら、その全部が出る。
            // **同じファイルの `parse_usage` も同じ作法**（`raw` を素で載せる）だが、
            // **逆向きの前例もある**——`llm::lmstudio` の `ERROR_BODY_LOG_LIMIT`（512 バイト）。
            // **G-9 のレビューは深刻度を低いと判定した**（ログにしか出ず、
            // 応答には載らないので `LlmError` の `Display` の制約には触れない）。
            // **上限を足すなら別の作業単位。**
            raw = %field,
            "OpenAI 互換のモデル一覧の実効コンテキスト長が不正です"
        );
        return Err(LlmError::MissingContextLength);
    };

    Ok(EffectiveContext {
        tokens,
        model_id: id.to_owned(),
        matched,
    })
}

/// LM Studio 独自 API（`/api/v0/models`）の応答から、
/// **上流が持っているモデル識別子を、並んだ順に**取り出す（作業単位 E-2）。
///
/// # `state` で絞らないこと
///
/// 一覧には未ロードのモデルも並ぶが、**LM Studio は JIT ロード**なので、
/// **一覧に在る綴りを依頼すれば、その綴りが載って答える**（docs/archive/04-environment-lm-studio.md）。
/// **未ロードは「これから載る」であって「別のモデルが答える」ではない。**
///
/// 絞ると、**冷えた起動のたびに「そんなモデルは無い」と報告する**ことになる。
/// 判定側の理由は `crate::measured::check` の doc。
///
/// # 何のために要るのか
///
/// **依頼する綴りが上流に実在するかを知る、唯一の手段。**
/// LM Studio は**存在しないモデル名でもエラーにせず、載っているモデルが黙って答える**ので、
/// **「こちらが何を依頼したか」は確認手段にならない**（実測 / 2026-08-28）。
///
/// # 読めなければ空
///
/// 崩れた JSON・`data` が無い・`id` が文字列でない、のいずれも**空**を返す。
/// 呼び出し側（`measured::check`）は**空を「聞けなかった」に倒す**ので、
/// 読めなかったことが「ずれ」として報告されることはない。
pub(super) fn parse_model_catalogue(body: &str) -> Vec<String> {
    let Ok(wire) = serde_json::from_str::<Value>(body) else {
        tracing::debug!("上流のモデル一覧を JSON として解釈できませんでした");
        return Vec::new();
    };

    let Some(entries) = wire.get("data").and_then(Value::as_array) else {
        tracing::debug!("上流のモデル一覧に data がありません");
        return Vec::new();
    };

    entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect()
}

/// **数える口の応答から、数を読む**（ADR-0052 §推奨4 / 作業単位 B-60a）。
///
/// **形で判定する。** 読めるのは `{"tokens":[非負の整数…]}` だけで、
/// それ以外（エラー本文・文字列・負数・小数・入れ子・`null`）は `None`（数えられない）。
/// **LM Studio は未知の口に 200 ＋ エラー本文を返す**（docs/04-environment.md）ので、
/// 状態コードが成功だったことは数えた証拠にならない。
///
/// **⚠ 入れ子（`with_pieces` の形 `[{"id":…,"piece":…}]`）も読まない。**
/// 頼んでいない形が返ったら、それは頼んだ口ではない。
///
/// 返すのは**要素の数**（トークン id の値は使わない）。
pub(super) fn parse_token_count(body: &str) -> Option<u32> {
    let value: Value = serde_json::from_str(body).ok()?;
    let tokens = value.as_object()?.get("tokens")?.as_array()?;
    if !tokens.iter().all(|token| token.as_u64().is_some()) {
        return None;
    }
    u32::try_from(tokens.len()).ok()
}

/// **組み上げの口の応答から、組み上がりの文字列を読む**（ADR-0052 §推奨3 / 作業単位 B-60a）。
///
/// 読めるのは `{"prompt":"…"}` だけ。**空の文字列も読まない** ——
/// テンプレートは必ず何かを出すので、空は壊れた答えであり、
/// 数えると 0 トークンの封になる（窓を超えて送る向きの誤り）。
pub(super) fn parse_applied_template(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let prompt = value.as_object()?.get("prompt")?.as_str()?;
    (!prompt.is_empty()).then(|| prompt.to_owned())
}

#[cfg(test)]
mod tests {
    /// **`FinishReason::ALL` / `AnsweredBy::ALL` に数え漏れが無い**（作業単位 P-2a）。
    ///
    /// 仕掛けは `domain::memory` の `every_memory_column_variant_is_listed_in_all` と同じ
    /// （網羅の `match`。**ワイルドカードの腕を足さないこと**。`ALL` からは回さない）。
    #[test]
    fn every_finish_reason_variant_is_listed_in_all() {
        use super::FinishReason;
        let listed = |reason: FinishReason| match reason {
            FinishReason::Stop | FinishReason::Length | FinishReason::Other => {
                FinishReason::ALL.contains(&reason)
            }
        };
        for reason in [
            FinishReason::Stop,
            FinishReason::Length,
            FinishReason::Other,
        ] {
            assert!(listed(reason), "{reason:?} が ALL に無い");
        }
        assert_eq!(FinishReason::ALL.len(), 3);
    }

    #[test]
    fn every_answered_by_variant_is_listed_in_all() {
        use super::AnsweredBy;
        let listed = |by: AnsweredBy| match by {
            AnsweredBy::NativeApi
            | AnsweredBy::CompatExactId
            | AnsweredBy::CompatSoleEntry
            | AnsweredBy::CompatCatalogueOnly => AnsweredBy::ALL.contains(&by),
        };
        for by in [
            AnsweredBy::NativeApi,
            AnsweredBy::CompatExactId,
            AnsweredBy::CompatSoleEntry,
            AnsweredBy::CompatCatalogueOnly,
        ] {
            assert!(listed(by), "{by:?} が ALL に無い");
        }
        assert_eq!(AnsweredBy::ALL.len(), 4);
    }

    use super::*;
    use crate::server::DEFAULT_LOG_FILTER;
    use crate::testlog::{
        DEBUG_FILTER, VERBOSE_FILTER, captured_with_filter, has_field, has_key, line_containing,
        quoted_field,
    };

    /// 「項目はあるのに読めない」ログの手掛かり。**全文一致にしないこと**
    /// （文言の推敲で、見張りたい「既定フィルタに載らないか」と無関係に割れる）。
    const UNREADABLE_MARKER: &str = "トークン数を解釈できません";

    /// 実機で観測した `/api/v0/models` の応答（2026-08-14 / ADR-0013）。
    ///
    /// **`max_context_length` と `loaded_context_length` の値を必ず違えておくこと。**
    /// 同じ値にすると、どちらを読んでいるかがテストから区別できなくなる。
    const NATIVE_MODELS_BODY: &str = r#"{
        "object": "list",
        "data": [
            {"id": "text-embedding-nomic-embed-text-v1.5", "object": "model", "type": "embeddings",
             "state": "not-loaded", "max_context_length": 2048},
            {"id": "ministral-3-14b-instruct-2512", "object": "model", "type": "llm",
             "publisher": "mistralai", "quantization": "Q4_K_M", "state": "loaded",
             "max_context_length": 262144, "loaded_context_length": 16384},
            {"id": "ministral-3-3b-instruct-2512", "object": "model", "type": "llm",
             "state": "not-loaded", "max_context_length": 262144}
        ]
    }"#;

    /// 上流が返しうる、あるいは崩れうる入力の詰め合わせ。
    /// **どれを与えても panic しない**ことを最後のテストで確かめる。
    const HOSTILE: [&str; 18] = [
        "",
        " ",
        "null",
        "[]",
        "{}",
        "0",
        "\"文字列\"",
        "{\"choices\":",
        "{\"choices\":{}}",
        "{\"choices\":[{}]}",
        "{\"choices\":[null]}",
        "{\"choices\":[{\"delta\":null,\"finish_reason\":null}]}",
        "{\"choices\":[{\"message\":{\"content\":null}}]}",
        "\u{0000}\u{FFFD}",
        // `usage` 側（ADR-0013 第3層）。**本文が読めても読めなくても panic しない。**
        "{\"usage\":null}",
        "{\"usage\":[{\"prompt_tokens\":1}]}",
        "{\"choices\":[{\"message\":{\"content\":\"x\"}}],\"usage\":{\"prompt_tokens\":-1}}",
        "{\"choices\":[{\"message\":{\"content\":\"x\"}}],\"usage\":{\"prompt_tokens\":{}}}",
    ];

    fn completion(body: &str) -> Completion {
        parse_completion(body).expect("解釈できること")
    }

    /// 正常系。本文と停止理由を取り出す。
    #[test]
    fn parses_a_normal_completion() {
        let parsed = completion(
            r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"こんにちは"},
                "finish_reason":"stop"}],"usage":{"total_tokens":2}}"#,
        );
        assert_eq!(parsed.content, "こんにちは");
        assert_eq!(parsed.finish_reason, Some(FinishReason::Stop));
    }

    /// **空文字の本文は正当。** `MissingContent` に落とさない。
    #[test]
    fn empty_content_is_a_valid_completion() {
        let parsed =
            completion(r#"{"choices":[{"message":{"content":""},"finish_reason":"stop"}]}"#);
        assert_eq!(parsed.content, "");
    }

    /// 停止理由が無い／未知でも解釈は続く。
    #[test]
    fn unknown_finish_reason_does_not_fail() {
        assert_eq!(
            completion(r#"{"choices":[{"message":{"content":"x"}}]}"#).finish_reason,
            None,
            "項目が無ければ None"
        );
        for raw in ["content_filter", "tool_calls", "", "STOP", "未知"] {
            let body = format!(
                r#"{{"choices":[{{"message":{{"content":"x"}},"finish_reason":"{raw}"}}]}}"#
            );
            assert_eq!(
                completion(&body).finish_reason,
                Some(FinishReason::Other),
                "raw = {raw:?}"
            );
        }
        let length =
            completion(r#"{"choices":[{"message":{"content":"x"},"finish_reason":"length"}]}"#);
        assert_eq!(length.finish_reason, Some(FinishReason::Length));
    }

    /// 候補が複数でも先頭を使う（panic しない）。
    #[test]
    fn uses_the_first_choice() {
        let parsed = completion(
            r#"{"choices":[{"message":{"content":"一番目"}},{"message":{"content":"二番目"}}]}"#,
        );
        assert_eq!(parsed.content, "一番目");
    }

    /// 本文が無いのは `MissingContent`。壊れた JSON（`Decode`）とは別物。
    #[test]
    fn missing_content_is_distinguished_from_broken_json() {
        for body in [
            r#"{"choices":[]}"#,
            r#"{"id":"1"}"#,
            r#"{"choices":[{"finish_reason":"stop"}]}"#,
            r#"{"choices":[{"message":{}}]}"#,
            r#"{"choices":[{"message":{"content":null}}]}"#,
        ] {
            assert!(
                matches!(parse_completion(body), Err(LlmError::MissingContent)),
                "body = {body}"
            );
        }
        for body in ["{", "not json", "{\"choices\":[", "[1,2,"] {
            assert!(
                matches!(parse_completion(body), Err(LlmError::Decode(_))),
                "body = {body}"
            );
        }
    }

    /// **上流が数えた `usage.prompt_tokens` を落とさない**（ADR-0013 第3層の入口）。
    ///
    /// ここを捨てると、モデル・上流・チャットテンプレートのどれが変わっても
    /// **「エラーも出ず応答も返る」まま**気付けない状態に戻る。
    #[test]
    fn keeps_the_prompt_tokens_the_upstream_counted() {
        let parsed = completion(
            r#"{"choices":[{"message":{"content":"こんにちは"},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":123,"completion_tokens":45,"total_tokens":168}}"#,
        );

        assert_eq!(
            parsed.usage,
            Some(TokenUsage {
                prompt_tokens: 123,
                completion_tokens: Some(45),
            }),
            "completion_tokens（45）や total_tokens（168）と取り違えないこと"
        );
        assert_eq!(parsed.content, "こんにちは", "本文はそのまま");
    }

    /// **生成トークン数も読む**（ADR-0059 §決定3 / 作業単位 A-18b —— 捨てた1回目の生成数を分解の行に出す）。
    ///
    /// **読めない値は `None` に落ち、プロンプトの数（答え合わせの材料）を巻き込まない。**
    /// 0 は正当（何も生成しなかった回）。プロンプトの数・合計（`total_tokens`）と取り違えないよう、
    /// 3つは別の値にしてある。
    #[test]
    fn keeps_the_completion_tokens_without_risking_the_prompt_tokens() {
        let usage_of = |completion_raw: &str| {
            completion(&format!(
                r#"{{"choices":[{{"message":{{"content":"本文"}}}}],
                    "usage":{{"prompt_tokens":123,"completion_tokens":{completion_raw},"total_tokens":999}}}}"#
            ))
            .usage
        };
        assert_eq!(
            usage_of("45"),
            Some(TokenUsage {
                prompt_tokens: 123,
                completion_tokens: Some(45),
            })
        );
        assert_eq!(
            usage_of("0"),
            Some(TokenUsage {
                prompt_tokens: 123,
                completion_tokens: Some(0),
            }),
            "0 は正当な生成数"
        );
        for broken in ["\"45\"", "4.5", "-1", "4294967296", "null", "true"] {
            assert_eq!(
                usage_of(broken),
                Some(TokenUsage {
                    prompt_tokens: 123,
                    completion_tokens: None,
                }),
                "raw = {broken}"
            );
        }
        // **項目が無い**のも `None`（プロンプトの数は残る）。
        assert_eq!(
            completion(
                r#"{"choices":[{"message":{"content":"本文"}}],"usage":{"prompt_tokens":123}}"#
            )
            .usage,
            Some(TokenUsage {
                prompt_tokens: 123,
                completion_tokens: None,
            })
        );
        // **使用量だけのチャンク**（中継の1回目の生成数はここから来る）。
        let chunk =
            parse_chunk(r#"{"choices":[],"usage":{"prompt_tokens":321,"completion_tokens":77}}"#)
                .expect("解釈できること")
                .expect("使用量だけのチャンクは読み飛ばさないこと");
        assert_eq!(
            chunk.usage,
            Some(TokenUsage {
                prompt_tokens: 321,
                completion_tokens: Some(77),
            })
        );
    }

    /// u32 の上限ちょうどは受け取る（境界で 1 ずれない）。
    #[test]
    fn the_largest_representable_prompt_tokens_is_accepted() {
        let body = format!(
            r#"{{"choices":[{{"message":{{"content":"x"}}}}],"usage":{{"prompt_tokens":{}}}}}"#,
            u32::MAX
        );
        assert_eq!(
            completion(&body).usage,
            Some(TokenUsage {
                prompt_tokens: u32::MAX,
                completion_tokens: None,
            })
        );
    }

    /// **`usage` が無いのは異常ではない。** 返さない上流の実装がある。
    ///
    /// 「答え合わせできない」だけなので、本文は普通に取れること。
    #[test]
    fn a_missing_usage_is_not_an_error() {
        for body in [
            r#"{"choices":[{"message":{"content":"x"}}]}"#,
            r#"{"choices":[{"message":{"content":"x"}}],"usage":null}"#,
            r#"{"choices":[{"message":{"content":"x"}}],"usage":{}}"#,
            // **`prompt_tokens` だけが無い**（他の項目は載っている）。
            r#"{"choices":[{"message":{"content":"x"}}],
                "usage":{"completion_tokens":45,"total_tokens":168}}"#,
        ] {
            let parsed = completion(body);
            assert_eq!(parsed.content, "x", "body = {body}");
            assert_eq!(parsed.usage, None, "body = {body}");
        }
    }

    /// **壊れた `usage` のために本文を失わない。** 型違い・負値・巨大値のいずれも
    /// 「答え合わせできない」に落ちるだけ。
    ///
    /// `usage` を構造体に写し取ると、ここが全部 `Decode`（= 応答ごと失う）になる。
    #[test]
    fn a_broken_usage_never_costs_us_the_content() {
        for raw in [
            "\"123\"",
            "123.5",
            "true",
            "false",
            "[123]",
            "{\"value\":123}",
            "null",
            "-123",
            "4294967296",
            "18446744073709551615",
        ] {
            let body = format!(
                r#"{{"choices":[{{"message":{{"content":"本文"}}}}],
                    "usage":{{"prompt_tokens":{raw}}}}}"#
            );
            let parsed = completion(&body);
            assert_eq!(parsed.content, "本文", "raw = {raw}");
            assert_eq!(parsed.usage, None, "raw = {raw}");
        }

        // `usage` そのものがオブジェクトでない場合も同じ。
        for raw in ["5", "\"none\"", "[]", "[{\"prompt_tokens\":3}]", "true"] {
            let body =
                format!(r#"{{"choices":[{{"message":{{"content":"本文"}}}}],"usage":{raw}}}"#);
            let parsed = completion(&body);
            assert_eq!(parsed.content, "本文", "raw = {raw}");
            assert_eq!(parsed.usage, None, "raw = {raw}");
        }
    }

    /// **`0` を信じない。**
    ///
    /// 削れない枠が必ず載るのでプロンプトが 0 トークンということはありえず、
    /// 受け取ると「見積もり ≥ 実トークン」が**常に成立して見える**——
    /// 第3層がエラーも出さずに死ぬ、最も気付けない壊れ方をする。
    #[test]
    fn a_reported_zero_is_not_believed() {
        let parsed = completion(
            r#"{"choices":[{"message":{"content":"本文"}}],"usage":{"prompt_tokens":0}}"#,
        );

        assert_eq!(parsed.content, "本文");
        assert_eq!(
            parsed.usage, None,
            "0 を受け取ると上界の破れが永久に検出できなくなる"
        );

        // **境界は 1。** `TokenUsage` の doc が「必ず 1 以上」と宣言している通り、
        // 1 は**正当な値として通す**。ここを固定しないと、除外条件を
        // `> 1` へ広げても気付けない（正当な答え合わせが黙って捨てられる）。
        let boundary = completion(
            r#"{"choices":[{"message":{"content":"本文"}}],"usage":{"prompt_tokens":1}}"#,
        );

        assert_eq!(
            boundary.usage,
            Some(TokenUsage {
                prompt_tokens: 1,
                completion_tokens: None,
            }),
            "1 は正当な実トークン数なので捨てないこと"
        );
    }

    /// 読めない `usage` を持つ応答を一通り通す（本文はどれも正当）。
    fn parse_unreadable_usages() {
        for raw in ["\"123\"", "123.5", "-1", "0", "4294967296", "true"] {
            let body = format!(
                r#"{{"choices":[{{"message":{{"content":"本文"}}}}],
                    "usage":{{"prompt_tokens":{raw}}}}}"#
            );
            assert_eq!(
                parse_completion(&body).expect("本文は取れること").usage,
                None,
                "raw = {raw}"
            );
        }
    }

    /// **読めない `usage` は既定のログ設定に載せない。**
    ///
    /// 現状の実装は正しい（`debug`）。ここを `warn` に上げると、
    /// `usage.prompt_tokens` を文字列で返す上流に差し替えた瞬間から
    /// **毎リクエスト鳴り続け**、本当に見たい警告（`server::usage` の上界の破れ）が埋もれる。
    /// 深刻度は `server::usage` 側だけでなく**ここでも固定する**
    /// （`report` は読めた場合しか通らないので、あちらは素通しになる）。
    #[test]
    fn an_unreadable_usage_stays_out_of_the_default_log_filter() {
        // **まず「イベントが出ている」ことを確かめる。** 出ていないなら
        // 下の主張は「何も起きていない」を見ているだけになる。
        let verbose = captured_with_filter(VERBOSE_FILTER, parse_unreadable_usages);
        assert!(
            verbose.contains(UNREADABLE_MARKER),
            "前提: 読めなかったこと自体は記録すること: {verbose:?}"
        );

        let output = captured_with_filter(DEFAULT_LOG_FILTER, parse_unreadable_usages);
        assert!(
            !output.contains(UNREADABLE_MARKER),
            "既定の設定では鳴らさないこと（毎リクエスト鳴る警告にしない）: {output:?}"
        );
        assert!(
            output.is_empty(),
            "本文が取れている応答で既定の設定に何も残さないこと: {output:?}"
        );
    }

    /// 終端の番兵。前後の空白は許すが、別物と混同しない。
    #[test]
    fn recognizes_the_done_marker() {
        for data in ["[DONE]", " [DONE]", "[DONE] ", "\n[DONE]\n"] {
            assert!(is_done(data), "data = {data:?}");
        }
        for data in ["", "[done]", "DONE", "[DONE][DONE]", "{\"choices\":[]}"] {
            assert!(!is_done(data), "data = {data:?}");
        }
    }

    /// チャンクの正常系。
    #[test]
    fn parses_a_normal_chunk() {
        let parsed = parse_chunk(
            r#"{"choices":[{"index":0,"delta":{"content":"こん"},"finish_reason":null}]}"#,
        )
        .expect("解釈できること")
        .expect("中身があること");
        assert_eq!(parsed.delta, "こん");
        assert_eq!(parsed.finish_reason, None);
    }

    /// 役割だけ・停止理由だけのチャンクは**空の増分**として流す（捨てない）。
    #[test]
    fn keeps_chunks_without_content() {
        let role_only = parse_chunk(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#)
            .expect("解釈できること")
            .expect("中身があること");
        assert_eq!(role_only.delta, "");
        assert_eq!(role_only.finish_reason, None);

        let last = parse_chunk(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#)
            .expect("解釈できること")
            .expect("中身があること");
        assert_eq!(last.delta, "");
        assert_eq!(last.finish_reason, Some(FinishReason::Stop));
    }

    /// `choices` が空で、**読める `usage` も無い**チャンクは読み飛ばす。
    /// ここをエラーにすると、本文の途中でストリームが死ぬ。
    ///
    /// **`usage` が読める場合は読み飛ばさない**
    /// （`a_usage_only_chunk_is_not_skipped`）。
    #[test]
    fn skips_chunks_without_choices() {
        for data in [
            r#"{"choices":[]}"#,
            r#"{"id":"1","usage":{"total_tokens":9}}"#,
        ] {
            // `LlmError` は比較できない（`reqwest::Error` を含む）ので matches! で見る。
            assert!(matches!(parse_chunk(data), Ok(None)), "data = {data}");
        }
    }

    /// **チャンクに載った `usage` を落とさない**（ADR-0013 第3層 / ストリーミング）。
    ///
    /// ストリーミングでは `usage` が**最終チャンクにしか載らない**ので、
    /// ここで捨てると答え合わせの材料が中継経路から丸ごと消える。
    #[test]
    fn keeps_the_prompt_tokens_a_chunk_reports() {
        let parsed = parse_chunk(
            r#"{"choices":[{"delta":{"content":"い」"},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":123,"completion_tokens":45,"total_tokens":168}}"#,
        )
        .expect("解釈できること")
        .expect("中身があること");

        assert_eq!(parsed.delta, "い」", "本文はそのまま");
        assert_eq!(
            parsed.usage,
            Some(TokenUsage {
                prompt_tokens: 123,
                completion_tokens: Some(45),
            }),
            "completion_tokens（45）や total_tokens（168）と取り違えないこと"
        );
    }

    /// **使用量だけを載せた最終チャンクは読み飛ばさない。**
    ///
    /// `stream_options.include_usage` を持つ上流は、本文を出し切った後に
    /// **`choices` が空で `usage` だけのチャンク**を送る。ここを
    /// 「中身の無いチャンク」として捨てると、第3層はストリーミングで永久に働かない。
    #[test]
    fn a_usage_only_chunk_is_not_skipped() {
        let parsed = parse_chunk(r#"{"choices":[],"usage":{"prompt_tokens":321}}"#)
            .expect("解釈できること")
            .expect("使用量だけのチャンクは読み飛ばさないこと");

        assert_eq!(parsed.delta, "", "増分は無い");
        assert_eq!(parsed.finish_reason, None);
        assert_eq!(
            parsed.usage,
            Some(TokenUsage {
                prompt_tokens: 321,
                completion_tokens: None,
            })
        );
    }

    /// **`usage` が無いチャンクは `None`。** 欠落を `0` などで埋めない
    /// （埋めると「見積もり ≥ 実トークン」が常に成立して見え、第3層が黙って死ぬ）。
    #[test]
    fn a_chunk_without_usage_reports_nothing() {
        for data in [
            r#"{"choices":[{"delta":{"content":"x"}}]}"#,
            r#"{"choices":[{"delta":{"content":"x"}}],"usage":null}"#,
            r#"{"choices":[{"delta":{"content":"x"}}],"usage":{"total_tokens":9}}"#,
            r#"{"choices":[{"delta":{"content":"x"}}],"usage":{"prompt_tokens":0}}"#,
            r#"{"choices":[{"delta":{"content":"x"}}],"usage":{"prompt_tokens":"123"}}"#,
        ] {
            let parsed = parse_chunk(data)
                .expect("解釈できること")
                .expect("中身があること");
            assert_eq!(parsed.delta, "x", "data = {data}");
            assert_eq!(parsed.usage, None, "data = {data}");
        }
    }

    /// 空のペイロードは `EmptyPayload`。壊れた JSON とは区別する。
    #[test]
    fn empty_payload_is_distinguished_from_broken_json() {
        for data in ["", " ", "\t", "\n "] {
            assert!(
                matches!(parse_chunk(data), Err(LlmError::EmptyPayload)),
                "data = {data:?}"
            );
        }
        for data in ["{", "data", "{\"choices\":"] {
            assert!(
                matches!(parse_chunk(data), Err(LlmError::Decode(_))),
                "data = {data:?}"
            );
        }
    }

    /// **読むのは `loaded_context_length`。`max_context_length` ではない。**
    ///
    /// 最大値（262144）を信じると、実際には 16384 しか無いので
    /// **LM Studio ではプロンプトが前方から黙って切り捨てられる**——エラーも出ず応答も返るので
    /// 気付けない（ADR-0013 第2層）。llama は 400 `exceed_context_size_error` で断る
    /// （ADR-0052 M5）。ただし、プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる
    /// （`finish_reason=length`）。
    #[test]
    fn reads_the_loaded_context_length_not_the_maximum() {
        assert_eq!(
            parse_loaded_context_length(NATIVE_MODELS_BODY, "ministral-3-14b-instruct-2512")
                .expect("実機の応答は解釈できること"),
            16_384,
            "max_context_length（262144）ではなく loaded_context_length を読むこと"
        );
    }

    /// **ロードされていないモデルの値は使わない。**
    ///
    /// 一覧には未ロードのモデルも並ぶ。`state` を見ないと、たまたま
    /// 別のモデルの残骸が拾われる。
    #[test]
    fn an_unloaded_model_has_no_usable_context_length() {
        // 一覧にはあるが `state` が `loaded` ではない。
        for model in [
            "ministral-3-3b-instruct-2512",
            "text-embedding-nomic-embed-text-v1.5",
        ] {
            assert!(
                matches!(
                    parse_loaded_context_length(NATIVE_MODELS_BODY, model),
                    Err(LlmError::MissingContextLength)
                ),
                "model = {model}"
            );
        }

        // `loaded_context_length` があっても `state` が `loaded` でなければ使わない。
        let body = r#"{"data":[{"id":"m","state":"loading","loaded_context_length":16384}]}"#;
        assert!(matches!(
            parse_loaded_context_length(body, "m"),
            Err(LlmError::MissingContextLength)
        ));
    }

    /// 一覧に無いモデルは「取れなかった」。**別の行で代用しない。**
    #[test]
    fn a_model_that_is_not_listed_yields_nothing() {
        for model in ["", "unknown-model", "ministral-3-14b-instruct-2512 "] {
            assert!(
                matches!(
                    parse_loaded_context_length(NATIVE_MODELS_BODY, model),
                    Err(LlmError::MissingContextLength)
                ),
                "model = {model:?}"
            );
        }
    }

    /// **id は完全一致で引く。前方一致にすると別サイズ・別量子化の行を掴む。**
    ///
    /// 一覧には `m` と `m-extra` のように**片方が他方の前置**になる id が並びうる
    /// （実機なら `ministral-3-14b-instruct-2512` と `…-2512@q8` のような派生）。
    /// 前方一致だと**先に並んだ方**を掴むので、設定したモデルとは別の
    /// コンテキスト長を信じたまま起動し、窓を超える（LM Studio では前方から黙って切り捨てられる
    /// / ADR-0013。llama は 400 `exceed_context_size_error` で断る / ADR-0052 M5。ただし、
    /// プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`））。
    #[test]
    fn the_model_id_is_matched_exactly() {
        // **派生 id を先に並べる。** 前方一致なら 99999 を掴む。
        let body = r#"{"data":[
            {"id":"m-extra","state":"loaded","loaded_context_length":99999},
            {"id":"m","state":"loaded","loaded_context_length":16384}
        ]}"#;
        assert_eq!(
            parse_loaded_context_length(body, "m").expect("完全一致する行があること"),
            16_384,
            "前方一致で `m-extra` を掴まないこと"
        );

        // 逆向き: 派生 id を引いたときに素の方で代用しない。
        assert_eq!(
            parse_loaded_context_length(body, "m-extra").expect("完全一致する行があること"),
            99_999
        );

        // 完全一致する行が無ければ「取れなかった」。**前置の行で代用しない。**
        let only_derived =
            r#"{"data":[{"id":"m-extra","state":"loaded","loaded_context_length":99999}]}"#;
        assert!(matches!(
            parse_loaded_context_length(only_derived, "m"),
            Err(LlmError::MissingContextLength)
        ));
    }

    /// **上流の JSON を信用しない。** 欠落・型違い・0・巨大値のどれでも
    /// panic せず「取れなかった」に落ちる。
    #[test]
    fn a_broken_context_length_is_never_believed() {
        for body in [
            // `data` が無い・配列でない。
            r#"{}"#,
            r#"{"data":null}"#,
            r#"{"data":{"id":"m","state":"loaded","loaded_context_length":16384}}"#,
            r#"{"data":[]}"#,
            // 項目が配列や null（`id` を引けない）。
            r#"{"data":[null,[1,2]]}"#,
            // 値が欠けている。
            r#"{"data":[{"id":"m","state":"loaded"}]}"#,
            r#"{"data":[{"id":"m","state":"loaded","loaded_context_length":null}]}"#,
            // **`loaded_context_length` が無ければ「取れなかった」。max で代用しない。**
            // ここを `max_context_length` へ落とすと、上流が 262144 と申告するまま
            // 261120 バイトまで積み、**LM Studio ではプロンプトが前方から黙って切り捨てられる**
            // （ADR-0013）。llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。
            // ただし、プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる
            // （`finish_reason=length`）。
            //
            // **他の否定ケースはこれを見張れない。** どれも `max_context_length` が
            // 同時に欠けているので、代用されても「取れなかった」のままで緑を通る。
            // 見張れるのは `max_context_length` を載せた次の2件だけ。
            //
            // 2026-08-15 に、フォールバックを足す2つの変種で**各ケース単独**の
            // 実測を採った（もう一方のケースを外して1件ずつ確かめた）:
            //
            // | | `or_else` が `as_u64` の**前** | `as_u64` の**後** |
            // |---|---|---|
            // | 1件目（max のみ） | 捕まえる | 捕まえる |
            // | 2件目（max ＋ `null`） | **捕まえない** | 捕まえる |
            //
            // **1件目を消さないこと。** 2件目は「前」の変種を素通りさせるので、
            // 1件目の代わりにはならない（`null` があると `or_else` に入らない）。
            r#"{"data":[{"id":"m","state":"loaded","max_context_length":262144}]}"#,
            r#"{"data":[{"id":"m","state":"loaded","max_context_length":262144,
                "loaded_context_length":null}]}"#,
            // 型違い。
            r#"{"data":[{"id":"m","state":"loaded","loaded_context_length":"16384"}]}"#,
            r#"{"data":[{"id":"m","state":"loaded","loaded_context_length":16384.5}]}"#,
            r#"{"data":[{"id":"m","state":"loaded","loaded_context_length":[16384]}]}"#,
            r#"{"data":[{"id":"m","state":"loaded","loaded_context_length":true}]}"#,
            r#"{"data":[{"id":"m","state":"loaded","loaded_context_length":-16384}]}"#,
            // `state` が欠けている・型違い。
            r#"{"data":[{"id":"m","loaded_context_length":16384}]}"#,
            r#"{"data":[{"id":"m","state":true,"loaded_context_length":16384}]}"#,
            // 0（予算として成立しない）。
            r#"{"data":[{"id":"m","state":"loaded","loaded_context_length":0}]}"#,
            // u32 に収まらない巨大値。
            r#"{"data":[{"id":"m","state":"loaded","loaded_context_length":4294967296}]}"#,
            r#"{"data":[{"id":"m","state":"loaded","loaded_context_length":18446744073709551615}]}"#,
        ] {
            assert!(
                matches!(
                    parse_loaded_context_length(body, "m"),
                    Err(LlmError::MissingContextLength)
                ),
                "body = {body}"
            );
        }

        // JSON として壊れている場合は `Decode`（「値が無い」とは別物）。
        for body in ["", "{", "not json", r#"{"data":["#] {
            assert!(
                matches!(
                    parse_loaded_context_length(body, "m"),
                    Err(LlmError::Decode(_))
                ),
                "body = {body:?}"
            );
        }
    }

    /// u32 の上限ちょうどは受け取る（境界で 1 ずれない）。
    #[test]
    fn the_largest_representable_context_length_is_accepted() {
        let body = format!(
            r#"{{"data":[{{"id":"m","state":"loaded","loaded_context_length":{}}}]}}"#,
            u32::MAX
        );
        assert_eq!(
            parse_loaded_context_length(&body, "m").expect("u32 に収まること"),
            u32::MAX
        );
    }

    /// **独自 API 側の警告も、どの綴りを探した結果なのかを、その行の中で名乗る**
    /// （作業単位 B-43）。
    ///
    /// # 着手前は、この4本の `model` の欄を誰も見ていなかった
    ///
    /// **実測（2026-09-08 / 着手前のツリー）: `parse_loaded_context_length` の
    /// `warn` 4本から `model` の欄を落とす変異は、4本とも 2005 全緑・exit 0。**
    /// 綴りを探す処理は独自 API と OpenAI 互換の2系統あり、
    /// **どちらの綴りを探した結果なのかが読めない警告は、
    /// 読み手を上流のモデル構成へ案内できない。**
    ///
    /// # ⚠ この見張り番は深刻度を主張しない
    ///
    /// **捕捉は `personacore=debug`。** 4本とも現に `warn` だが、
    /// **この作業単位は欄の帰属の話であって、深刻度の話ではない**
    /// （G-8 / G-9 が引いた境目を動かさない）。
    /// **したがって「この4本が既定のフィルタで鳴ること」は、いまも誰も見ていない**
    /// ——**実測（B-43 / 是正後）: `warn!` を `debug!` へ落とす変異が 2006 全緑・exit 0。**
    /// 塞ぐなら別の作業単位で、`DEFAULT_LOG_FILTER` の側に腕を置くこと。
    ///
    /// # ⚠ 取り違えを見分けられるのは「一覧に居ない」の腕だけ
    ///
    /// 残る3本は**完全一致で引いた行**なので、その行の `id` と依頼した綴りは
    /// 必ず同値になる——`model` をその行の `id` へ差し替えても、その3本では
    /// 同じ綴りが出る。
    ///
    /// **⚠ 着手時は「推論であって未測定」と書いていた。測った**（B-43 のレビュー / 2026-09-08）——
    /// **`model` をその行の `id` へ差し替える変異は 2006 全緑・exit 0。**
    /// **ただしこれは穴ではなく等価変異である**（`CLAUDE.md` の第3の分類）——
    /// **その分岐は `entry.get("id") == Some(model)` の完全一致でしか到達しないので、
    /// 受理集合が単集合に潰れ、両者は必ず同一バイト列になる。**
    ///
    /// **⚠ 「この欄が無防備」ではない** —— 決め打ちへの差し替え（4本とも赤）と、
    /// compat 側の `model = id`（赤）が、欄そのものは押さえている。
    /// **2つの綴りが違う状態を作れるのは「一覧に居ない」の腕だけ**なので、
    /// そこだけ上流に [`UPSTREAM_ONLY_MODEL`] を名乗らせてある。
    ///
    /// **値の側は `assert_eq!` なので、決め打ちの変異は4本とも赤くなる**
    /// ——**実測（B-43 / 是正後）: 4本それぞれに
    /// `model = "upstream-only-id"` を埋め込む変異を当てて、4本とも
    /// 2005 passed / 1 failed / exit 101**（赤くなるのはこのテスト1本）。
    #[test]
    fn the_native_api_warnings_name_the_model_that_was_asked_for() {
        for (body, marker, arm) in [
            (
                // 依頼した綴りが一覧に居ない。**上流は別の綴りだけを名乗る。**
                format!(
                    r#"{{"data":[{{"id":"{UPSTREAM_ONLY_MODEL}","state":"loaded",
                        "loaded_context_length":16384}}]}}"#
                ),
                "上流のモデル一覧に設定中のモデルがありません",
                "一覧に居ない",
            ),
            (
                format!(
                    r#"{{"data":[{{"id":"{ASKED_FOR_MODEL}","state":"not-loaded",
                        "max_context_length":262144}}]}}"#
                ),
                "設定中のモデルはロードされていません",
                "ロードされていない",
            ),
            (
                format!(r#"{{"data":[{{"id":"{ASKED_FOR_MODEL}","state":"loaded"}}]}}"#),
                "ロード済みのコンテキスト長が読めません",
                "長さが読めない",
            ),
            (
                format!(
                    r#"{{"data":[{{"id":"{ASKED_FOR_MODEL}","state":"loaded",
                        "loaded_context_length":0}}]}}"#
                ),
                "ロード済みのコンテキスト長が不正です",
                "長さが不正",
            ),
        ] {
            let output = captured_with_filter(DEBUG_FILTER, || {
                assert!(
                    parse_loaded_context_length(&body, ASKED_FOR_MODEL).is_err(),
                    "前提: {arm} 応答からは長さが取れないこと"
                );
            });

            let line = line_said_by_this_layer(&output, marker);
            assert_eq!(
                quoted_field(line, "model"),
                Some(ASKED_FOR_MODEL),
                "{arm}: 探した綴りを同じ行に載せること: {line:?}"
            );
        }

        assert_ne!(
            ASKED_FOR_MODEL, UPSTREAM_ONLY_MODEL,
            "この2つを同値にすると、取り違えが検出できなくなる"
        );
    }

    /// **どんな入力でも panic しない。** 崩れた出力は前提であって例外ではない。
    #[test]
    fn never_panics_on_hostile_input() {
        for data in HOSTILE {
            let _ = parse_completion(data);
            let _ = parse_chunk(data);
            let _ = is_done(data);
            let _ = parse_loaded_context_length(data, "m");
            let _ = parse_loaded_context_length(data, "");
            let _ = parse_model_catalogue(data);
            // OpenAI 互換の側（作業単位 G-2）。**同じ入力でどちらも panic しない。**
            let _ = parse_effective_context_length(data, "m");
            let _ = parse_effective_context_length(data, "");
        }
    }

    /// **一覧は `state` で絞らない**（作業単位 E-2）。
    ///
    /// 実機の応答には**ロード済み1件と未ロード2件**が並んでいる。
    /// **3件とも返ること**が要点——絞ると、冷えた起動のたびに
    /// 「依頼する綴りが上流に無い」と報告することになる。
    ///
    /// **並び順も保つ。** 上流が返した順そのままにしておくと、
    /// 報告を読む人が実機の応答と突き合わせられる。
    #[test]
    fn the_catalogue_lists_unloaded_models_too() {
        assert_eq!(
            parse_model_catalogue(NATIVE_MODELS_BODY),
            vec![
                // 未ロード（かつ埋め込みモデル）。
                "text-embedding-nomic-embed-text-v1.5".to_owned(),
                // ロード済み。
                "ministral-3-14b-instruct-2512".to_owned(),
                // 未ロード。
                "ministral-3-3b-instruct-2512".to_owned(),
            ],
            "state で絞らず、並んだ順のまま返すこと"
        );
    }

    /// **読めない応答は空。** 「聞けなかった」に倒すのは呼び出し側の判定。
    #[test]
    fn an_unreadable_body_yields_an_empty_catalogue() {
        for body in [
            // JSON ですらない。
            "",
            "{",
            "not json",
            // `data` が無い / 配列でない。
            r#"{"object":"list"}"#,
            r#"{"data":{"id":"m"}}"#,
            r#"{"data":null}"#,
            // 項目に `id` が無い / 文字列でない。
            r#"{"data":[{"state":"loaded"}]}"#,
            r#"{"data":[{"id":42}]}"#,
            r#"{"data":[{"id":null}]}"#,
            // 空の一覧。
            r#"{"data":[]}"#,
        ] {
            assert!(
                parse_model_catalogue(body).is_empty(),
                "読めない応答は空であること: {body:?}"
            );
        }
    }

    /// **読める項目だけを拾う。** 1件が壊れていても、残りは使える。
    #[test]
    fn a_broken_entry_does_not_hide_the_readable_ones() {
        let body = r#"{"data":[{"id":42},{"id":"m"},{"state":"loaded"},{"id":"n"}]}"#;
        assert_eq!(
            parse_model_catalogue(body),
            vec!["m".to_owned(), "n".to_owned()]
        );
    }

    // ------------------------------------- OpenAI 互換の一覧（作業単位 G-2 / `/v1/models`）

    /// **実機の `llama-server` が返した `/v1/models`**（2026-09-04 / 作業単位 G-1 の保存物）。
    ///
    /// 3つの事実がここに全部入っている。**どれも実装の分かれ目。**
    ///
    /// * **`id` は gguf の絶対パス**で、設定中のモデルの綴りとは**一致しない。**
    /// * **`meta.n_ctx`（いま載っている窓）と `meta.n_ctx_train`（訓練時の上限）が並ぶ。**
    ///   **必ず違う値にしておくこと**——同じ値だと、どちらを読んだかが区別できない。
    /// * **`data` の隣に `models`（Ollama 互換の写し）がある。**
    ///   そちらには `meta` が無いので、**読む先を取り違えたら長さが取れない。**
    const LLAMA_SERVER_V1_MODELS_BODY: &str = r#"{
        "models": [
            {"name": "C:\\Users\\someone\\.lmstudio\\models\\mistralai\\Ministral-3-14B-Instruct-2512-GGUF\\Ministral-3-14B-Instruct-2512-Q4_K_M.gguf",
             "model": "C:\\Users\\someone\\.lmstudio\\models\\mistralai\\Ministral-3-14B-Instruct-2512-GGUF\\Ministral-3-14B-Instruct-2512-Q4_K_M.gguf",
             "type": "model", "capabilities": ["completion"]}
        ],
        "object": "list",
        "data": [
            {"id": "C:\\Users\\someone\\.lmstudio\\models\\mistralai\\Ministral-3-14B-Instruct-2512-GGUF\\Ministral-3-14B-Instruct-2512-Q4_K_M.gguf",
             "aliases": ["C:\\Users\\someone\\.lmstudio\\models\\mistralai\\Ministral-3-14B-Instruct-2512-GGUF\\Ministral-3-14B-Instruct-2512-Q4_K_M.gguf"],
             "tags": [], "object": "model", "created": 1788531110, "owned_by": "llamacpp",
             "meta": {"vocab_type": true, "n_vocab": 131072, "n_ctx": 8192, "n_ctx_train": 262144,
                      "n_embd": 5120, "n_params": 13506073600, "size": 8231178240,
                      "ftype": "Q4_K - Medium"}}
        ]
    }"#;

    /// 上の実機の応答に載っている `id`（**gguf の絶対パス**）。
    ///
    /// **設定中のモデルの綴りとは一致しない。** 綴りで引く限り、必ず外れる。
    const LLAMA_SERVER_MODEL_ID: &str = r"C:\Users\someone\.lmstudio\models\mistralai\Ministral-3-14B-Instruct-2512-GGUF\Ministral-3-14B-Instruct-2512-Q4_K_M.gguf";

    /// 上の実機の応答の `meta.n_ctx`（**いま載っている窓**）。
    const LLAMA_SERVER_N_CTX: u32 = 8_192;

    /// 上の実機の応答の `meta.n_ctx_train`（**訓練時の上限**）。**読んではいけない方。**
    ///
    /// **32 倍ある。** 取り違えると本文の予算が 32 倍になり、
    /// 窓を超えたプロンプトを送る。llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5。
    /// LM Studio なら前方から黙って切り捨てられる＝ADR-0013 の「最も気付けない壊れ方」）。
    /// ただし、プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
    const LLAMA_SERVER_N_CTX_TRAIN: u32 = 262_144;

    /// **実機の LM Studio が返した `/v1/models`**（2026-09-04 / 作業単位 G-1 の保存物）。
    ///
    /// **200 で、一覧としては読める。しかし `meta` を持たない。**
    /// **「200 が返った」は「答えた」ではない**——ここが分かれ目。
    const LM_STUDIO_V1_MODELS_BODY: &str = r#"{
        "data": [
            {"id": "qwen/qwen3.5-9b", "object": "model", "owned_by": "organization_owner"},
            {"id": "ministral-3-14b-instruct-2512", "object": "model",
             "owned_by": "organization_owner"},
            {"id": "text-embedding-nomic-embed-text-v1.5", "object": "model",
             "owned_by": "organization_owner"}
        ],
        "object": "list"
    }"#;

    /// 設定中のモデルの綴り（実機の設定と同じ）。
    const CONFIGURED_MODEL: &str = "ministral-3-14b-instruct-2512";

    /// **実機の `llama-server` の応答から、いま載っている窓が読める**（作業単位 G-2）。
    ///
    /// **同定は `id` の一致ではない。** 一覧が1件しか無いこと（＝**答えるモデルは
    /// それしかない**）を根拠にする。実機の `id` は gguf の絶対パスなので、
    /// **綴りで引く限り永久に外れる。**
    #[test]
    fn the_effective_context_length_comes_from_the_sole_entry_of_a_llama_server() {
        let found = parse_effective_context_length(LLAMA_SERVER_V1_MODELS_BODY, CONFIGURED_MODEL)
            .expect("実機の応答なら読めること");

        assert_eq!(found.tokens, LLAMA_SERVER_N_CTX);
        assert_eq!(
            found.model_id, LLAMA_SERVER_MODEL_ID,
            "実際に読んだ行の id を持ち帰ること（ログにしか出さない）"
        );
        assert_eq!(
            found.matched,
            ModelMatch::SoleEntry,
            "綴りが一致しないので、根拠は「一覧が1件しかない」であること"
        );
    }

    /// **`n_ctx_train` を読まないこと**（作業単位 G-2 / 32 倍の取り違え）。
    ///
    /// `loaded_context_length` に対する `max_context_length` と同じ罠
    /// （[`parse_loaded_context_length`] の doc）。**エラーも出ず応答も返る**ので、
    /// 取り違えたことは本番では気付けない。
    #[test]
    fn the_trained_context_length_is_not_the_effective_one() {
        assert_ne!(
            LLAMA_SERVER_N_CTX, LLAMA_SERVER_N_CTX_TRAIN,
            "前提: 2つは違う値であること（同じだと取り違えが観測できない）"
        );

        let found = parse_effective_context_length(LLAMA_SERVER_V1_MODELS_BODY, CONFIGURED_MODEL)
            .expect("実機の応答なら読めること");

        assert_ne!(
            found.tokens, LLAMA_SERVER_N_CTX_TRAIN,
            "訓練時の上限を実効の窓として読まないこと"
        );
        assert_eq!(found.tokens, LLAMA_SERVER_N_CTX);
    }

    /// **200 が返ることは「答えた」ではない**（作業単位 G-2 / 実機の LM Studio）。
    ///
    /// LM Studio の `/v1/models` は**短い id が並ぶだけで `meta` を持たない。**
    /// **一覧としては読める**（下の `assert`）が、**長さは1つも無い。**
    /// ここを「200 だから答えた」と読むと、次の行で存在しない値を掴むことになる。
    #[test]
    fn an_openai_list_without_meta_reports_no_context_length() {
        assert!(
            !parse_model_catalogue(LM_STUDIO_V1_MODELS_BODY).is_empty(),
            "前提: 一覧としては読めること（読めないから失敗した、ではない）"
        );
        assert!(
            LM_STUDIO_V1_MODELS_BODY.contains(CONFIGURED_MODEL),
            "前提: 設定中のモデルの綴りは一覧に在ること（居ないから失敗した、ではない）"
        );

        let error = parse_effective_context_length(LM_STUDIO_V1_MODELS_BODY, CONFIGURED_MODEL)
            .expect_err("`meta` が無ければ長さは取れないこと");
        assert!(matches!(error, LlmError::MissingContextLength), "{error:?}");
    }

    /// **綴りが一致する行があるなら、そちらが優先**（推測の余地を残さない）。
    ///
    /// **「一覧が1件」の規則より先に効くこと。** 逆にすると、
    /// 一覧に自分の行があるのに別の行を読む形が書ける。
    #[test]
    fn an_exact_id_match_wins_over_the_other_entries() {
        const BODY: &str = r#"{"data":[
            {"id":"m","meta":{"n_ctx":4096,"n_ctx_train":262144}},
            {"id":"n","meta":{"n_ctx":9216,"n_ctx_train":262144}}
        ]}"#;

        for (model, expected) in [("m", 4_096_u32), ("n", 9_216)] {
            let found = parse_effective_context_length(BODY, model).expect("綴りが一致すること");
            assert_eq!(found.tokens, expected, "model = {model}");
            assert_eq!(found.model_id, model, "読んだ行の id は自分の行であること");
            assert_eq!(found.matched, ModelMatch::ExactId, "model = {model}");
        }
    }

    /// **どれとも一致せず、一覧が2件以上あるなら読まない**（作業単位 G-2）。
    ///
    /// **ここが「1件しか無ければそれを採る」の危険を抑えている唯一の場所。**
    /// 間違ったモデルの窓を本文の予算にすると壊れる。この経路の上流（llama）は、窓を大きく
    /// 取り違えれば 400 `exceed_context_size_error` で断るか、プロンプト ＋ 応答の上限が窓を超えて
    /// 応答が黙って短くなる（`finish_reason=length` / ADR-0052 M5）。小さく取り違えれば余るだけ。
    /// どちらでも前方からの切り捨ては起きない（それは LM Studio の話 / ADR-0013）。
    /// **読まなければ落とし所へ落ちるだけ**——遅くなるだけで、壊れない。
    ///
    /// # 2つの腕は別の主張である（**G-2 のレビューが実測**）
    ///
    /// 着手時のフィクスチャは**2件とも `meta` を持っていた**ので、
    /// 2段目の入口を「**長さを持つ行がちょうど1件**」に緩める変異が **1934 全緑**だった。
    /// ——つまり [`parse_effective_context_length`] の doc が
    /// 「一覧そのものが1件を条件にしている」と書いていた区別を、
    /// **どのフィクスチャも一度も見ていなかった。**
    ///
    /// **`meta` を持つ行が1件しかない多件の一覧**が要る。
    /// これは架空の上流ではない——**埋め込みモデルが隣に並んでいる形**で、
    /// 実機の LM Studio の `/v1/models` にはその綴りがそのまま載っている。
    #[test]
    fn a_list_with_several_models_and_no_match_is_refused() {
        // どの行も窓を申告する（「一致が無ければ読まない」そのもの）。
        const BOTH_HAVE_A_LENGTH: &str = r#"{"data":[
            {"id":"m","meta":{"n_ctx":4096}},
            {"id":"n","meta":{"n_ctx":9216}}
        ]}"#;
        // **窓を申告するのは1行だけ。** 隣は `meta` を持たない別のモデル（埋め込み）。
        // **「長さを持つ行が1件」を条件にすると、居もしないモデルの窓を掴む。**
        const ONLY_ONE_HAS_A_LENGTH: &str = r#"{"data":[
            {"id":"nomic-embed-text-v1.5","object":"model"},
            {"id":"n","meta":{"n_ctx":9216}}
        ]}"#;

        for body in [BOTH_HAVE_A_LENGTH, ONLY_ONE_HAS_A_LENGTH] {
            // **「常に最初の1件」でも「常に最後の1件」でもないこと。**
            // どちらの決め打ちも、ここでは値を返してしまう形になる。
            let found = parse_effective_context_length(body, "z");
            assert!(
                matches!(found, Err(LlmError::MissingContextLength)),
                "どれが答えるのか分からないなら読まないこと: body = {body:?} / {found:?}"
            );
        }
    }

    /// **綴りが一致した行に長さが無いとき、別の行から借りてこないこと。**
    ///
    /// 一覧はこちらの綴りを知っている（＝**同定はできている**）。
    /// それでも長さが無いなら、答えは「取れなかった」であって「隣の値」ではない。
    #[test]
    fn an_exact_match_without_a_length_does_not_borrow_another_entry() {
        const BODY: &str = r#"{"data":[
            {"id":"m","object":"model"},
            {"id":"n","meta":{"n_ctx":9216}}
        ]}"#;

        let error = parse_effective_context_length(BODY, "m")
            .expect_err("自分の行に長さが無いなら取れないこと");
        assert!(matches!(error, LlmError::MissingContextLength), "{error:?}");
    }

    /// **一覧が1件でも、`meta` が読めなければ取れない。**
    #[test]
    fn a_sole_entry_without_a_readable_length_is_refused() {
        for body in [
            // `meta` そのものが無い。
            r#"{"data":[{"id":"m"}]}"#,
            // `meta` がオブジェクトでない。
            r#"{"data":[{"id":"m","meta":42}]}"#,
            // `n_ctx` が無い（`n_ctx_train` はある）。
            r#"{"data":[{"id":"m","meta":{"n_ctx_train":262144}}]}"#,
            // `n_ctx` が 0 / 負 / 文字列 / 小数 / u32 を超える。
            r#"{"data":[{"id":"m","meta":{"n_ctx":0}}]}"#,
            r#"{"data":[{"id":"m","meta":{"n_ctx":-1}}]}"#,
            r#"{"data":[{"id":"m","meta":{"n_ctx":"8192"}}]}"#,
            r#"{"data":[{"id":"m","meta":{"n_ctx":8192.5}}]}"#,
            r#"{"data":[{"id":"m","meta":{"n_ctx":4294967296}}]}"#,
            // **`u64` の上端。** 上の行と重複ではない（G-9 のレビューの `mtrunc`）——
            // **`4294967296` は `as u32` すると `0`** なので、切り捨ての変異でも
            // `> 0` の門が同じ答え（失敗）を返す。**`u64::MAX as u32` は
            // `4294967295`** なので、こちらだけが切り捨てを赤にできる。
            r#"{"data":[{"id":"m","meta":{"n_ctx":18446744073709551615}}]}"#,
            // 項目に `id` が無い（読んだ行を名乗れない）。
            r#"{"data":[{"meta":{"n_ctx":8192}}]}"#,
            // `data` が無い / 配列でない / 空。
            r#"{"object":"list"}"#,
            r#"{"data":{"id":"m","meta":{"n_ctx":8192}}}"#,
            r#"{"data":[]}"#,
        ] {
            assert!(
                parse_effective_context_length(body, "m").is_err(),
                "読めない応答は失敗すること: {body:?}"
            );
        }
    }

    /// u32 に収まる最大値は受け入れる（境界の上側）。
    #[test]
    fn the_largest_representable_effective_context_length_is_accepted() {
        let body = format!(
            r#"{{"data":[{{"id":"m","meta":{{"n_ctx":{}}}}}]}}"#,
            u32::MAX
        );
        assert_eq!(
            parse_effective_context_length(&body, "m")
                .expect("u32 に収まること")
                .tokens,
            u32::MAX
        );
    }

    /// **同定の根拠は、ログで区別が付く綴りであること。**
    ///
    /// 2つの根拠は**危険の度合いが違う**（一致は確実 / 1件しかないのは推論）ので、
    /// ログを読む人が見分けられなければ意味が無い。
    #[test]
    fn the_two_identification_grounds_have_distinct_labels() {
        assert_ne!(ModelMatch::ExactId.label(), ModelMatch::SoleEntry.label());
        assert!(!ModelMatch::ExactId.label().is_empty());
        assert!(!ModelMatch::SoleEntry.label().is_empty());
    }

    // ------------- 長さを名乗らない一覧の深刻度（作業単位 G-8 ② / G-9）
    //
    // **上流が「長さを持っていない」ことは、上流の性質であって故障ではない。**
    // LM Studio の `/v1/models` は `meta` を持たず（G-2 が実測）、
    // `meta` を持たない OpenAI 互換シム（vLLM / Ollama）も同じ形になる。
    // **そこを `warn` にすると、起動のたび＋5分ごとの引き直しのたびに鳴る**
    // （`llm::lmstudio` の `LmStudioClient::models` の doc / ADR-0020 の引き直し）。
    //
    // **深刻度を決めるのは「何が失われたか」を知っている側**（作業単位 G-5 の判断基準）。
    // 長さが読めなかった結果は、必ずどちらかが `warn` で1本出す ——
    //
    // | 綴りの一覧 | 誰が言うか |
    // |---|---|
    // | 取れた | `server::context` の「落とし所を使います / 既定値を使います」 |
    // | 取れなかった | `llm::lmstudio` の「どちらの綴りでも読めませんでした」 |
    //
    // **この層が足すと、同じ事実が2本になる**（`compat_models` の doc がそう書いている）。

    /// ログに出るこの層の綴り（`TARGET:` の位置）。
    const THIS_LAYER: &str = "personacore::llm::response:";

    /// **この層が「言っていること」で行を選ぶ**（`llm::lmstudio` の同名の道具と同じ理由）。
    ///
    /// `LlmError` の `Display` は定数文字列なので、**同じ文面が別の行の `error=` に載りうる。**
    /// 層の綴りと文面を繋いで `TARGET: メッセージ` の位置に固定する。
    fn line_said_by_this_layer<'a>(output: &'a str, message: &str) -> &'a str {
        line_containing(output, &format!("{THIS_LAYER} {message}"))
    }

    /// **依頼する綴り。** 上流が名乗る綴りと必ず違えること
    /// （同値にすると `model` と `upstream_model_id` の取り違えが検出できない）。
    const ASKED_FOR_MODEL: &str = "asked-for-model";

    /// **上流だけが名乗る綴り。** [`ASKED_FOR_MODEL`] と一致しない。
    const UPSTREAM_ONLY_MODEL: &str = "upstream-only-id";

    /// 同定できない一覧（**多件・一致なし**）。**件数は 0 でも 1 でもない値にすること。**
    const UNIDENTIFIABLE_LIST: &str = r#"{"object":"list","data":[
        {"id":"one","object":"model"},
        {"id":"two","object":"model"},
        {"id":"three","object":"model"}
    ]}"#;

    /// 長さを名乗らない一覧（**1件・`meta` が無い** / LM Studio の `/v1/models` の形）。
    const LIST_WITHOUT_A_LENGTH: &str =
        r#"{"object":"list","data":[{"id":"upstream-only-id","object":"model"}]}"#;

    /// **欄は在るが `null`** の一覧（作業単位 G-9 の是正 / 2026-09-07）。
    ///
    /// **`null` は「答えを持っていない」側**——`n_ctx: null` と名乗る上流は
    /// 「長さを持っていない」と言っているのであって、壊れているのではない。
    /// **境目のすぐ内側**なので、`filter(|value| !value.is_null())` を落とす変異は
    /// これで赤になる（[`an_upstream_that_answers_something_broken_is_still_a_warning`]
    /// の3本と対になる腕）。
    ///
    /// [`an_upstream_that_answers_something_broken_is_still_a_warning`]:
    ///     an_upstream_that_answers_something_broken_is_still_a_warning
    const LIST_WITH_A_NULL_LENGTH: &str =
        r#"{"object":"list","data":[{"id":"upstream-only-id","meta":{"n_ctx":null}}]}"#;

    /// **長さを名乗らない上流は、既定のログ設定を汚さない**（作業単位 G-8 ② / G-9）。
    ///
    /// # 肯定側と否定側を対にする
    ///
    /// **否定側だけだと、行を丸ごと落とす変異でも「何も出なかった」まま緑**になる
    /// （`crate::testlog` の doc）。だから **`personacore=debug` では現に出ていること**を
    /// 先に確かめ、**そのうえで既定のフィルタに `WARN` が1本も無い**ことを言う。
    ///
    /// # 2つの枝を1本ずつ当てる
    ///
    /// 「同定できない」と「同定はできたが長さが無い」は**別の枝**で、
    /// **読み手が調べに行く先も違う**（前者は上流のモデル構成、後者は上流の API の形）。
    /// **代表1つでは足りない**（CLAUDE.md / 作業単位 D-6）。
    #[test]
    fn a_list_that_does_not_name_a_length_stays_out_of_the_default_log_filter() {
        for (body, marker, arm) in [
            (
                UNIDENTIFIABLE_LIST,
                "OpenAI 互換のモデル一覧から実効コンテキスト長を同定できません",
                "同定できない",
            ),
            (
                LIST_WITHOUT_A_LENGTH,
                "OpenAI 互換のモデル一覧に実効コンテキスト長がありません",
                "長さが無い",
            ),
            // **境目のすぐ内側**（作業単位 G-9 の是正）。**欄は在るが `null`。**
            // ここが `warn` 側へ滑ると、`null` を返す上流で5分ごとに鳴る。
            (
                LIST_WITH_A_NULL_LENGTH,
                "OpenAI 互換のモデル一覧に実効コンテキスト長がありません",
                "長さが null",
            ),
        ] {
            let parse = || {
                assert!(
                    parse_effective_context_length(body, ASKED_FOR_MODEL).is_err(),
                    "前提: {arm} 一覧からは長さが取れないこと"
                );
            };

            // 肯定側——**その事実は現に1行出ている**（行ごと消す変異と区別が付くこと）。
            let verbose = captured_with_filter(DEBUG_FILTER, parse);
            let line = line_said_by_this_layer(&verbose, marker);
            assert!(
                has_field(line, "DEBUG"),
                "{arm}: 上流が長さを持っていないことは debug で残すこと: {line:?}"
            );
            assert_eq!(
                quoted_field(line, "model"),
                Some(ASKED_FOR_MODEL),
                "{arm}: 依頼した綴りを同じ行に載せること: {line:?}"
            );

            // 否定側——運用者が実際に受け取る綴りには、この層から1本も出さない。
            let output = captured_with_filter(DEFAULT_LOG_FILTER, parse);
            assert!(
                !output.lines().any(|line| has_field(line, "WARN")),
                "{arm}: 5分ごとに鳴る警告にしないこと: {output:?}"
            );
        }
    }

    /// **同定の根拠は、その行の中で読めること**（作業単位 G-8 ② / G-9）。
    ///
    /// 深刻度を下げても、**調査の手掛かりまで落とさない。**
    /// `debug` の1行だけで「どの綴りを探して・何件あって・どの行を読んだか」が分かること。
    ///
    /// **⚠ 欄は枝ごとに違う。** 「同定できない」側は `entries`（件数）、
    /// 「長さが無い」側は `upstream_model_id`（実際に読んだ行）。
    /// **取り違えると、読み手が向かう先が変わる。**
    #[test]
    fn the_quiet_lines_still_carry_what_the_reader_needs() {
        let unidentifiable = captured_with_filter(DEBUG_FILTER, || {
            let _ = parse_effective_context_length(UNIDENTIFIABLE_LIST, ASKED_FOR_MODEL);
        });
        let line = line_said_by_this_layer(
            &unidentifiable,
            "OpenAI 互換のモデル一覧から実効コンテキスト長を同定できません",
        );
        assert!(
            has_field(line, "entries=3"),
            "同定できなかった一覧の件数を同じ行に載せること: {line:?}"
        );

        let without_a_length = captured_with_filter(DEBUG_FILTER, || {
            let _ = parse_effective_context_length(LIST_WITHOUT_A_LENGTH, ASKED_FOR_MODEL);
        });
        let line = line_said_by_this_layer(
            &without_a_length,
            "OpenAI 互換のモデル一覧に実効コンテキスト長がありません",
        );
        assert_eq!(
            quoted_field(line, "upstream_model_id"),
            Some(UPSTREAM_ONLY_MODEL),
            "実際に読んだ行の綴りを同じ行に載せること: {line:?}"
        );
        assert_ne!(
            ASKED_FOR_MODEL, UPSTREAM_ONLY_MODEL,
            "この2つを同値にすると、取り違えが検出できなくなる"
        );
    }

    /// **「答えを持っていない」と「答えが壊れている」を分ける**（作業単位 G-8 ② / G-9）。
    ///
    /// **黙らせてよいのは前者だけ。** 後者は上流が壊れているので、
    /// **既定のフィルタで鳴り続けてよい**（正常な上流では起きないので、鳴り続けない）。
    ///
    /// # この見張り番が無いと、境目が観測できない
    ///
    /// **この層の `warn` を全部 `debug` へ落とす変異**は、
    /// 上の2本（静かになった側）だけでは**緑を通る。**
    /// **境目を主張するなら、境目の両側を測ること。**
    ///
    /// # 読めない綴りを1つずつ回すこと（G-9 のレビューの是正 / 2026-09-07）
    ///
    /// **着手時は「長さが 0」1本しか `read_effective_entry` の枝を通っていなかった。**
    /// `Value::as_u64` は**文字列・負数・小数**を `None` にするので、
    /// **その3つは「答えが壊れている」側なのに「ありません」の枝（`debug`）へ落ちていた**
    /// ——**実測: `n_ctx: -1` を既定のフィルタで回すと、この層の捕捉が空文字列。**
    ///
    /// **`u32` を超える側も、doc の表に在って腕が無かった。**
    /// **代表1つでは足りない**（CLAUDE.md / 作業単位 D-6）。
    ///
    /// **境目のすぐ内側**（`null`）は
    /// [`a_list_that_does_not_name_a_length_stays_out_of_the_default_log_filter`] が持つ。
    /// **片側だけだと、境目を全部こちらへ寄せる変異が緑を通る。**
    ///
    /// # ⚠ 「u32 を超える」の腕は、`4294967296` だけでは足りない（3周目の是正）
    ///
    /// **`4294967296`（= 2^32）は `as u32` すると `0`** になり、
    /// **直後の `> 0` の門が同じ `warn` を出す。**
    /// つまり `u32::try_from` を `as u32` へ替える**切り捨ての変異と区別が付かない**
    /// （レビュアーの `mtrunc` / **passed=1958 failed=0 / exit 0**）。
    /// **`u64::MAX` の腕を1本足して初めて赤になる**（`u64::MAX as u32` は `4294967295`）。
    /// **姉妹2箇所は着手前から両方を並べていた**——
    /// **写す元が持っていた腕が、写った先で1本だけ落ちた形。**
    ///
    /// # 欄の帰属も、この行で見る（3周目の是正）
    ///
    /// [`the_quiet_lines_still_carry_what_the_reader_needs`] は
    /// **`debug` 側の `upstream_model_id` しか見ていない。**
    /// **`warn` 側は無防備だった**——欄ごと落とす変異が全緑を通る
    /// （レビュアーの `muid`）。**抜けるのはいつも帰属**（CLAUDE.md）。
    ///
    /// # 依頼した綴り（`model`）の帰属も、この行で見る（作業単位 B-43）
    ///
    /// **`upstream_model_id` を足したときに、隣の `model` は素通りしていた。**
    /// **着手前の実測（2026-09-08 / この2本は `model` を持つ `warn`）:**
    ///
    /// | 変異 | 結果 |
    /// |---|---|
    /// | 「項目に id がありません」から `model` を落とす | **2005 全緑 / exit 0** |
    /// | 「実効コンテキスト長が不正です」から `model` を落とす | **2005 全緑 / exit 0** |
    /// | 前者を `model = "upstream-only-id"` へ決め打ち | **2005 全緑 / exit 0** |
    /// | 後者を `model = id`（上流だけが名乗る綴り）へ差し替え | **2005 全緑 / exit 0** |
    ///
    /// **`debug` 側だけが
    /// [`a_list_that_does_not_name_a_length_stays_out_of_the_default_log_filter`] に
    /// 見られており、`warn` 側は2本とも無防備だった。**
    ///
    /// **⚠ 腕ごとに期待値が違う。** 行を同定してから出す2本は依頼した綴りを名乗り、
    /// **一覧そのものについて言う2本（`data` が無い / JSON が壊れている）は名乗らない。**
    /// **「全腕で `Some(ASKED_FOR_MODEL)`」という形は、当ててみると偽**
    /// ——**実測: `data` が無い腕の期待値を `Some(ASKED_FOR_MODEL)` にすると
    /// `left: None` で 1 赤。** だから表に `Option` の列を持たせて
    /// `assert_eq!` で腕ごとに突き合わせている。
    ///
    /// [`a_list_that_does_not_name_a_length_stays_out_of_the_default_log_filter`]:
    ///     a_list_that_does_not_name_a_length_stays_out_of_the_default_log_filter
    /// [`the_quiet_lines_still_carry_what_the_reader_needs`]:
    ///     the_quiet_lines_still_carry_what_the_reader_needs
    #[test]
    fn an_upstream_that_answers_something_broken_is_still_a_warning() {
        for (body, marker, raw, model_field, arm) in [
            // 長さは在るのに、予算として成立しない（`0`）。
            (
                r#"{"data":[{"id":"upstream-only-id","meta":{"n_ctx":0}}]}"#,
                "OpenAI 互換のモデル一覧の実効コンテキスト長が不正です",
                Some("raw=0"),
                Some(ASKED_FOR_MODEL),
                "長さが 0",
            ),
            // u32 に収まらない。**「正しく報告された」とは考えない側。**
            (
                r#"{"data":[{"id":"upstream-only-id","meta":{"n_ctx":4294967296}}]}"#,
                "OpenAI 互換のモデル一覧の実効コンテキスト長が不正です",
                Some("raw=4294967296"),
                Some(ASKED_FOR_MODEL),
                "長さが u32 を超える",
            ),
            // **`u64` の上端。** 上の腕と重複ではない（G-9 のレビューが実測 / `mtrunc`）——
            // **`4294967296`（= 2^32）は `as u32` すると `0`** になり、
            // 直後の `.filter(|tokens| *tokens > 0)` が拾って**同じ `warn` を出す。**
            // つまり上の腕だけでは、**`u32::try_from` を `as u32` へ替える切り捨ての変異と
            // 区別が付かない**（実測: passed=1958 failed=0 / exit 0）。
            // **`u64::MAX as u32` は `4294967295`（> 0）**なので、そちらは通り抜ける
            // ——ここが唯一の関所である。
            //
            // **⚠ 姉妹2箇所（`parse_usage` / `parse_loaded_context_length`）は着手前から
            // 両方を並べていた。写す元が持っていた腕が、写った先で1本だけ落ちた形。**
            (
                r#"{"data":[{"id":"upstream-only-id","meta":{"n_ctx":18446744073709551615}}]}"#,
                "OpenAI 互換のモデル一覧の実効コンテキスト長が不正です",
                Some("raw=18446744073709551615"),
                Some(ASKED_FOR_MODEL),
                "長さが u64 の上端",
            ),
            // ------- ここから3本は、G-9 のレビューが「境目に乗っていない」と実測した側。
            //
            // **`Value::as_u64` は文字列・負数・小数をすべて `None` にする**ので、
            // **是正前は「長さが無い」の枝（`debug`）へ落ちていた**
            // ——`n_ctx: -1` を既定のフィルタで回すと**捕捉が空文字列**だった（実測）。
            // **欄が在るのに読めないのは上流の故障**なので、`warn` の側へ合流させた。
            (
                r#"{"data":[{"id":"upstream-only-id","meta":{"n_ctx":-1}}]}"#,
                "OpenAI 互換のモデル一覧の実効コンテキスト長が不正です",
                Some("raw=-1"),
                Some(ASKED_FOR_MODEL),
                "長さが負数",
            ),
            (
                r#"{"data":[{"id":"upstream-only-id","meta":{"n_ctx":"8192"}}]}"#,
                "OpenAI 互換のモデル一覧の実効コンテキスト長が不正です",
                // **引用符ごと出る。** 数と文字列を、この1行の中で見分けられること。
                Some(r#"raw="8192""#),
                Some(ASKED_FOR_MODEL),
                "長さが文字列",
            ),
            (
                r#"{"data":[{"id":"upstream-only-id","meta":{"n_ctx":8192.5}}]}"#,
                "OpenAI 互換のモデル一覧の実効コンテキスト長が不正です",
                Some("raw=8192.5"),
                Some(ASKED_FOR_MODEL),
                "長さが小数",
            ),
            // 読んだ行を名乗れない。
            (
                r#"{"data":[{"meta":{"n_ctx":8192}}]}"#,
                "OpenAI 互換のモデル一覧の項目に id がありません",
                None,
                // **`raw` を持たない腕。** ここを「`raw` があるときだけ見る」形に
                // 畳むと、この行の `model` は誰も見なくなる（B-43 の実測）。
                Some(ASKED_FOR_MODEL),
                "id が無い",
            ),
            // 一覧の形をしていない。
            (
                r#"{"object":"list"}"#,
                "上流の OpenAI 互換モデル一覧に data がありません",
                None,
                // **この2本は一覧そのものについて言っており、行を同定していない。**
                // いまの実装は `model` を載せていないので、期待値も `None` で固定する。
                // **⚠ 「この表を直すのが正しい入口」は言い過ぎだった**（B-43 のレビューが実測）——
                // **`Debug` の綴りで載せる形は表を通るが、`model = %model` は通らなかった。**
                // **下の `has_key` の否定が、その残りを塞いでいる。**
                None,
                "data が無い",
            ),
            // そもそも JSON として読めない。
            (
                r#"{"object":"#,
                "上流の OpenAI 互換モデル一覧を JSON として解釈できませんでした",
                None,
                None,
                "JSON が壊れている",
            ),
        ] {
            let output = captured_with_filter(DEFAULT_LOG_FILTER, || {
                assert!(
                    parse_effective_context_length(body, ASKED_FOR_MODEL).is_err(),
                    "前提: {arm} 応答からは長さが取れないこと"
                );
            });
            let line = line_said_by_this_layer(&output, marker);
            assert!(
                has_field(line, "WARN"),
                "{arm}: 壊れた応答は既定のフィルタで鳴らすこと: {line:?}"
            );
            // **どの綴りを探した結果なのかを、同じ行で読めること**（作業単位 B-43）。
            //
            // **⚠ 下の `if let Some(raw)` の中に置いてはいけない。**
            // `raw` を持たない腕（`id` が無い）が1本も通らなくなり、
            // **その行の `model` は誰も見ていない状態に戻る**
            // ——**着手前の実測: 欄を落とす変異も、上流の綴りへ差し替える変異も、
            // どちらも 2005 全緑・exit 0 で通った。**
            //
            // **⚠ 欄の有無は腕ごとに違う**ので、`Option` を表に持たせて突き合わせる
            // （「全腕で `Some(ASKED_FOR_MODEL)`」は偽——一覧そのものについて言う
            // 2本は行を同定していないので `model` を名乗らない）。
            assert_eq!(
                quoted_field(line, "model"),
                model_field,
                "{arm}: 依頼した綴りの欄は、この腕の期待どおりであること: {line:?}"
            );
            // **⚠ `quoted_field` は `キー="…"`（`Debug` の綴り）しか見ない。**
            // **`model = %model`（`Display` / 引用符なし）で載せる形は、上の `assert_eq!` を素通りする**
            // ——**B-43 のレビューが実測（2006 全緑・exit 0）。しかも `%` は絵空事ではなく、
            // 同じ関数の隣の行が現に使っている**（`error = %err` / `raw = %field`）。
            //
            // **「そのキーが、値によらず一切現れない」を言うのは [`has_key`] の仕事**
            // （`testlog::assert` の表がそう書いている）。**`None` の腕はそれを言いたい。**
            if model_field.is_none() {
                assert!(
                    !has_key(line, "model"),
                    "{arm}: この行は綴りを同定していないので `model` を名乗らないこと: {line:?}"
                );
            }
            // **上流が名乗った綴りが、そのまま同じ行に載ること。**
            // **これが無いと「型が違ったことが1行の中で読める」は誰も見ていない**
            // ——欄を落とす変異も、値を決め打ちにする変異も緑を通る（G-9 の是正で実測）。
            if let Some(raw) = raw {
                assert!(
                    has_field(line, raw),
                    "{arm}: 上流が名乗った綴りを同じ行に載せること（{raw}）: {line:?}"
                );
                // **どの行を読んでその綴りを見たのか**まで、同じ行で読めること
                // （G-9 のレビューの是正 / `muid`）。
                //
                // **`the_quiet_lines_still_carry_what_the_reader_needs` は
                // `debug` 側の `upstream_model_id` しか見ていない。**
                // **`warn` 側は無防備だった**——欄ごと落とす変異が全緑を通る（実測）。
                // **抜けるのはいつも帰属**（CLAUDE.md）。
                assert_eq!(
                    quoted_field(line, "upstream_model_id"),
                    Some(UPSTREAM_ONLY_MODEL),
                    "{arm}: 実際に読んだ行の綴りを同じ行に載せること: {line:?}"
                );
                assert_ne!(
                    ASKED_FOR_MODEL, UPSTREAM_ONLY_MODEL,
                    "この2つを同値にすると、取り違えが検出できなくなる"
                );
            }
        }
    }

    /// **OpenAI 互換の一覧も、同じ関数で綴りを取り出せる**（`data[].id` は共通の形）。
    #[test]
    fn the_catalogue_reads_the_openai_shaped_list_too() {
        assert_eq!(
            parse_model_catalogue(LLAMA_SERVER_V1_MODELS_BODY),
            vec![LLAMA_SERVER_MODEL_ID.to_owned()],
            "`models`（Ollama 互換の写し）ではなく `data` を読むこと"
        );
        assert_eq!(
            parse_model_catalogue(LM_STUDIO_V1_MODELS_BODY),
            vec![
                "qwen/qwen3.5-9b".to_owned(),
                "ministral-3-14b-instruct-2512".to_owned(),
                "text-embedding-nomic-embed-text-v1.5".to_owned(),
            ]
        );
    }

    // ---- 数える口（ADR-0052 / 作業単位 B-60a）----
    // 往復を通した表は `tests/llm_count.rs` の
    // `a_count_shape_that_is_not_a_token_list_is_not_a_count`（N8）。ここは関数の境目だけを見る。

    /// 数の列は、要素の数を返す。**トークン id の値は使わない**（u32 を超える id でも数える）。
    #[test]
    fn a_token_list_counts_its_elements() {
        assert_eq!(parse_token_count(r#"{"tokens":[1,2,3]}"#), Some(3));
        assert_eq!(parse_token_count(r#"{"tokens":[4294967296]}"#), Some(1));
        assert_eq!(parse_token_count(r#"{"tokens":[]}"#), Some(0));
        // 余分な欄は邪魔しない。
        assert_eq!(
            parse_token_count(r#"{"tokens":[7,8],"extra":true}"#),
            Some(2)
        );
    }

    /// **1つでも整数でない要素があれば、列全体を読まない**（数え落としを数として返さない）。
    #[test]
    fn a_single_bad_element_spoils_the_whole_list() {
        for body in [
            r#"{"tokens":[1,-1]}"#,
            r#"{"tokens":[1,1.5]}"#,
            r#"{"tokens":[1,"2"]}"#,
            r#"{"tokens":[1,null]}"#,
            r#"{"tokens":[1,[2]]}"#,
            r#"{"tokens":[1,{"id":2,"piece":"b"}]}"#,
            r#"{"tokens":[1,true]}"#,
        ] {
            assert_eq!(parse_token_count(body), None, "{body:?}");
        }
    }

    /// 組み上がりは `prompt` の文字列。**空は読まない。**
    #[test]
    fn an_applied_template_is_a_non_empty_prompt_string() {
        assert_eq!(
            parse_applied_template(r#"{"prompt":"<s>[INST]x[/INST]"}"#).as_deref(),
            Some("<s>[INST]x[/INST]")
        );
        for body in [
            r#"{"prompt":""}"#,
            r#"{"prompt":1}"#,
            r#"{"prompt":null}"#,
            r#"{"error":"Unexpected endpoint or method. (POST /apply-template)"}"#,
            r#""prompt""#,
            "",
        ] {
            assert_eq!(parse_applied_template(body), None, "{body:?}");
        }
    }

    /// **敵意のある入力でも panic しない**（深い入れ子・巨大な数・壊れた UTF-8 相当）。
    #[test]
    fn the_count_parsers_never_panic_on_hostile_input() {
        let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
        for body in [
            deep.as_str(),
            r#"{"tokens":[1e400]}"#,
            r#"{"tokens":[18446744073709551616]}"#,
            "\u{0}",
            "{\"tokens\":[1,",
        ] {
            assert_eq!(parse_token_count(body), None);
            assert_eq!(parse_applied_template(body), None);
        }
    }
}
