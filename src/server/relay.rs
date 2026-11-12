//! **SSE 中継**（作業単位 11a-2b）。上流のチャンク列を OpenAI 互換の SSE へ均す。
//!
//! ```text
//! LM Studio ──SSE──► personacore ──SSE──► FE
//! ```
//!
//! # 溜め込まない
//!
//! **1チャンク受けたら1イベント返す**（docs/02-architecture.md）。
//! 本文の蓄積（永続化が使う）は `Relay::relay` の中の `push_str` 1行で、
//! **中継と同じ場所にある**ので、蓄積のためだけにストリームを止めることはしない
//! （流すものと蓄積するものが、同じ1回の呼び出しで決まる）。
//!
//! 例外は2つ（どちらも「蓄積のために止める」ではなく、「流すのを遅らせる／流さない」）。
//!
//! 1. **使用量だけのチャンク**（`is_usage_only`）。受け取って答え合わせに使うが下流へは出さない。
//!    **溜め込みではない**（次のチャンクを待たずにその場で捨てる）。
//! 2. **`finish_reason` 付きのチャンクと終わりの印（`[DONE]`）に限って保留する**
//!    （[ADR-0059](../../../docs/adr/0059-thinking-and-response-room-in-the-upstream-profile.md) §決定3 /
//!    作業単位 A-18b）。下の「本文が空なら1回だけ引き直す」の節。**本文の差分は今までどおり受けたそばに流す。**
//!
//! # 本文が空なら1回だけ引き直す（ADR-0059 §決定3 / 作業単位 A-18b）
//!
//! `finish_reason` 付きのチャンクが届いた時点で、それまでに積み上げた本文（そのチャンクの差分を含む）が
//! 剥がして空（空白だけを含む）で、**かつ閉じない思考ではない**とき（`super::empty_retry::retries`）、
//! そのチャンクを流さずに保留する。1回目の上流が尽きたら（使用量のチャンクまで読み切ってから —— 捨てた回の
//! 生成数を取るため）、**同じ上流の写し・同じ要求**（[`Redo`]）で2回目を開き、**同じ応答の続き**として流す。
//!
//! | 2回目 | 下流に見えるもの | 保存 |
//! |---|---|---|
//! | 開けた | 保留したチャンクの差分（空でなければ、`finish_reason` を外して） → 2回目のチャンク列 → `[DONE]` | **2回目の生の出力**（2回目も空なら空のまま。3回目は無い） |
//! | 開けなかった（接続の失敗・上流のエラー） | 保留したチャンクをそのまま → `[DONE]`（引き直しが無かったときと同じ） | 1回目の生の出力 |
//!
//! * **役割は最初のチャンクにしか付かない** —— 2回目も同じ [`Relay`] を通すので、二重にならない。
//! * **保留したチャンクの差分を捨てない**（閉じタグ `</think>` でありうる。捨てると使い手の側で、
//!   閉じない思考の後ろに2回目の本文が続く形になり、本文まで思考として剥がされる）。
//! * **空かの判定は、最初の `finish_reason` 付きのチャンクを受けた時点で行う**（`RelayState::holds`）。
//!   保留した後に届くものでは、判定をやり直さない:
//!   * **また `finish_reason` 付きのチャンクが届いたら、保留に足す**（差分を後ろに連結し、`finish_reason` は
//!     後のものにする）。上書きしない —— 先の差分が流れず蓄積からも落ちると、`raw_content` が欠ける。
//!   * **`finish_reason` の無い本文の差分**が届いたら、今までどおり受けたそばに流して蓄積する（保留した差分より
//!     前に並ぶ）。その後も引き直すことは変わらない（そういう上流は想定していない）。
//! * 1回目の上流が尽きる前に壊れたら、下の「途中で上流が壊れたとき」と同じ（保留したチャンクは流さず、保存しない）。
//! * 2回目の途中で上流が壊れたら、下の「途中で上流が壊れたとき」と同じ（保存しない）。
//! * 答え合わせと封の清算は、2回目の `usage` で1回だけ（[`Relay::restart`] が1回目の `usage` を捨てる）。
//!
//! # 途中で上流が壊れたときの伝え方
//!
//! **200 とヘッダは既に送っているので、4xx/5xx では伝えられない。** 決めた形:
//!
//! 1. **そこまでに流したチャンクは有効**（`llm::ChatStream` の doc）。取り消さない。
//! 2. 最後に**非ストリーミングと同じ error オブジェクト**を1件流す（定数のみ。
//!    上流のステータス値も `{err:?}` も載せない）。
//! 3. **`[DONE]` は送らない。** 送ると「正常に完結した」と読めてしまう。
//!    こちら自身が上流に対して「`[DONE]` 無しの終端 ＝ `LlmError::Truncated`」と
//!    解釈している（`llm::lmstudio::chunk_stream`）ので、**下流にも同じ規約で伝える。**
//!
//! つまりクライアントから見た成否の判定は**2つの独立した手掛かり**で付く
//! （error オブジェクトが流れたか／`[DONE]` が来たか）。
//!
//! # 保存するのは「完結した回」だけ（11b-2）
//!
//! **`[DONE]` を流す枝でだけ [`TurnSink::persist`] を呼ぶ。** 途中で壊れた枝では
//! **利用者の発言も含めて1行も書かない。** 部分的な応答を履歴に混ぜると以後の文脈が汚れ、
//! しかも**クライアント側はその回を失敗として扱っている**（`[DONE]` を受けていない）ので、
//! こちらだけが「途中まで喋った」ことを覚えていると食い違う。
//!
//! 分解・保存・パース成功率のログは `super::turn` が持つ。**保存の失敗で中継は壊さない**
//! （そもそも `[DONE]` の直前なので、もう応答の成否は変えられない）。
//!
//! # 中継中の無通信は**報告する。打ち切らない**（11c-3a → ADR-0035 / 作業単位 B-6）
//!
//! **生成が途中で止まったことは、ここでしか観測できない。** `llm` の読み取り予算
//! （既定 180秒）は**JIT ロードを飲むための値**で、生成中の停止の検出には緩すぎるうえ、
//! `reqwest` の `read_timeout` はクライアント単位でしか設定できないので
//! 「最初のトークンまで」と「トークン間」を分けられない。
//!
//! そこで `chunk_events` の `events.next()` を [`IDLE_BUDGET`] で巻いてある。
//! **巻いた先でやるのは `warn` を出すことだけで、中継はそのまま待ち続ける。**
//! 詳しくは [`next_reporting_silence`] の doc。
//!
//! **⚠ ここで打ち切っていたのは B-6 まで**（[ADR-0019](../../../docs/adr/0019-relay-idle-watchdog.md) §3）。
//! **30 秒はこの機のトークン間隔（数十ミリ秒）から出た数**で、
//! **CPU オフロード・上流の混雑・スリープ・画像生成との取り合いでは超えうる。**
//! **外から「遅い」と「死んだ」は原理的に区別できない**ので、
//! **値を正しくするのではなく、外れても壊れない場所へ移した**
//! （[ADR-0035](../../../docs/adr/0035-the-watchdog-reports-the-human-decides.md) /
//! [ADR-0036](../../../docs/adr/0036-the-range-of-machines-is-a-constraint.md) 第2階層）。
//!
//! **打ち切りの天井は `llm` 層が持つ**（`llm::lmstudio` の `DEFAULT_IDLE_TIMEOUT` と、
//! それを差し替える `PERSONACORE_LLM_IDLE_TIMEOUT`）。天井に当たると上流の
//! ストリームが `LlmError::Timeout` を返し、**下の「上流が壊れた」枝**へ合流する。
//! **その回を保存しないのは変わらない**（ADR-0019 §4 は生きている）。
//!
//! # 切断時のキャンセル（11c-3b）— **分岐は無い。drop が全部やっている**
//!
//! **このモジュールに「切断」を扱うコードは1行も無い。** FE が切断すると axum が
//! 本文のストリーム（= この [`RelayState`]）を落とし、その中の `ChatStream` が
//! 落ちると上流への TCP が閉じる。終端の `state.upstream = None` も同じ1手。
//!
//! **2026-08-20 に実測した**（それまでは「そうなっているはず」だった）。
//! **実測値は各見張り番の側に書いてある**（両方に書くと片方が黙って古びる）:
//!
//! | 主張 | 見張り番 |
//! |---|---|
//! | `ChatStream` を drop すると上流の TCP が閉じる | `tests/llm_lmstudio.rs` の `dropping_the_stream_closes_the_upstream_connection` |
//! | FE の切断で axum が本文を落とす | `tests/server_chat_cancel.rs`（前半） |
//! | **応答が1バイトも出る前**に切られても上流を落とす | `tests/server_chat_cancel.rs`（後半 / 本番の入口と偽 LM Studio を通す） |
//! | 終端がその場で上流を落とす | 下の `a_failed_relay_drops_the_upstream_before_the_error_reaches_the_client` |
//!
//! **抱え込む実装（別タスクへ move して先読みする等）に変えても、
//! 下流に見える SSE の中身は変わらない。** ただし
//! **「その1本以外は全部緑」ではない**——`relay_response` で上流を先読みタスクへ
//! move する変異の実測（2026-08-20）:
//!
//! * `server::relay::tests` の **6本**が赤。すべて 11c-3a の無通信打ち切りで、
//!   **先読みタスクが沈黙を吸ってしまい打ち切りが起きない**（`[DONE]` まで流れる）。
//! * `tests/server_chat_cancel.rs` の **3本**が赤（上流が黙っている側・端から端まで・対照の第2相）。
//! * `tests/server_chat_stream.rs` と `tests/llm_lmstudio.rs` は**全件緑。**
//!
//! > **⚠ この実測は打ち切りがあった頃のもの**（B-6 で撤回）。
//! > **6本の赤は「打ち切りが起きない」だった**ので、いま同じ変異を当てても
//! > **同じ内訳にはならない**（沈黙の報告が出なくなる形で赤くなる）。
//! > **`tests/server_chat_cancel.rs` の3本は打ち切りに依存していない。**
//!
//! **無通信の報告（11c-3a → ADR-0035）と切断時キャンセル（11c-3b）は、
//! 「上流を逐次読む」という同じ1点に寄りかかっている。**

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::{Stream, StreamExt as _, stream};
use serde_json::{Map, Value};

use super::chat::{ASSISTANT_ROLE, finish_reason_to_wire, upstream_error};
use super::empty_retry::{self, Delivery, EmptyRetry};
use super::seal::Sealed;
use super::turn::TurnSink;
use super::usage;
use crate::llm::{ChatChunk, ChatRequest, ChatStream, LlmClient, LlmError, LlmFuture, TokenUsage};
use crate::prompt::{BudgetPlan, TokenBudget};

/// `object` フィールドの定数（OpenAI 互換のストリーミング）。
///
/// **非ストリーミングの `chat.completion` と別物。** 取り違えるとクライアントは
/// 増分を「1回で完結した応答の本文」として読み、最後の1文字だけを表示する。
pub(super) const CHUNK_OBJECT: &str = "chat.completion.chunk";

/// 終端の番兵。**OpenAI 互換クライアントはこれで応答の完了を判定する。**
pub(super) const DONE_DATA: &str = "[DONE]";

/// **中継中の無通信を報告する間隔**（ADR-0035 §決定2 / 作業単位 B-6）。
///
/// # ⚠ **打ち切りの予算ではない。「言う」までの間隔である**
///
/// 名前は [ADR-0019](../../../docs/adr/0019-relay-idle-watchdog.md) から引き継いでいるが、
/// **役目は B-6 で「打ち切る」から「言う」へ移った**（同 §3 の撤回 / ADR-0035 §決定1）。
/// ここを外しても起きるのは**余計なログが出るか、出ないか**だけで、
/// **ターンは死なない。** それが移した理由そのものである。
///
/// # 設定にしない（ADR-0035 §決定5）
///
/// 「言うだけ」になったので、外しても壊れない。
/// **つまみ1つにつき `App::bind` を通る見張り番が1本要る**という費用は
/// [ADR-0020](../../../docs/adr/0020-context-length-requery.md) §2 のまま生きているので、
/// 壊れない値をつまみにしない。
///
/// **設定可能なのは天井の側だけ**（`llm::lmstudio` の `DEFAULT_IDLE_TIMEOUT` /
/// `PERSONACORE_LLM_IDLE_TIMEOUT`）。あちらは**根拠が無く・外すと破壊的で・
/// こちらからは導出も観測もできない**という3条件が揃っている（ADR-0036 §決定3）。
const IDLE_BUDGET: Duration = Duration::from_secs(30);

/// 中継1本分の状態。
///
/// **`Relay::new` が唯一のコンストラクタ**で、id・model・created は最初に1回だけ決める
/// （OpenAI は1本の応答の全チャンクでこの3つを共通にする）。
pub(super) struct Relay {
    /// 応答 id（`chatcmpl-…`）。全チャンクで共通。
    id: String,
    /// **着地したセッション（`aoi@<session-id>`）。** 全チャンクで共通。
    ///
    /// **上流のモデル名でもクライアントが送った生の `model` でもない**（ADR-0004）。
    /// ここが化けると、クライアントは着地したセッションを固定できなくなる。
    model: String,
    /// 生成時刻（Unix 秒）。全チャンクで共通。
    created: i64,
    /// **蓄積した本文。** 中継が完結したときの保存対象（`super::turn`）。
    content: String,
    /// 先頭チャンクを流したか（役割は先頭にだけ載せる）。
    started: bool,
    /// 上流が申告したトークン数（ADR-0013 第3層）。
    ///
    /// **最終チャンクにしか載らない**（載せない実装もある）ので、
    /// 受け取った最後の値を持ち続ける。
    usage: Option<TokenUsage>,
}

impl Relay {
    /// 1本の応答を通して共通の値を決める。
    pub(super) const fn new(id: String, model: String, created: i64) -> Self {
        Self {
            id,
            model,
            created,
            content: String::new(),
            started: false,
            usage: None,
        }
    }

    /// 1チャンクを受けて、下流へ流す `data:` の中身を返す。**同時に蓄積する。**
    ///
    /// # `None` は「受け取ったが下流へは流さない」
    ///
    /// **使用量だけのチャンク**（[`is_usage_only`]）がそれ。蓄積と答え合わせには
    /// 使うが、下流へは出さない。`llm::parse_chunk` が `choices` の空いた
    /// 使用量チャンクを「空の増分を持つチャンク」に均すので、素通しすると
    /// `{"delta":{"content":""}}` が1件余分に流れる——**OpenAI 本家の使用量チャンクは
    /// `choices: []`** なので、クライアントから見ると存在しないはずのイベントになる。
    ///
    /// # `Value` を直に組み立てる理由
    ///
    /// 直列化の失敗という**到達不能な分岐**を作らないため
    /// （`unwrap` も、誰も通らない `unwrap_or_else` も置かない）。
    ///
    /// # 下流の枠を組むのはこの関数だけ（作業単位 A-18b）
    ///
    /// 空の本文の引き直しで保留したチャンク（モジュールの doc）を後から流すときも、ここを通す
    /// （`web/tests/chat-wire.test.ts` がこの関数の本体を枠の権威として読む）。
    pub(super) fn relay(&mut self, chunk: &ChatChunk) -> Option<String> {
        // **蓄積は中継と同じ1行。** ここが「ストリームを止めない」の実装そのもの。
        self.content.push_str(&chunk.delta);
        // **最終チャンクの `usage` を落とさない。** 無ければ持っている値のまま。
        self.note_usage(chunk.usage);

        // **受け取ってから判定する。** 先に返すと答え合わせの材料ごと捨てる。
        if is_usage_only(chunk) {
            return None;
        }

        let mut delta = Map::new();
        // **役割は先頭チャンクにだけ載せる**（OpenAI 互換クライアントが期待する形）。
        if !self.started {
            delta.insert("role".to_owned(), Value::from(ASSISTANT_ROLE));
        }
        self.started = true;
        delta.insert("content".to_owned(), Value::from(chunk.delta.as_str()));

        let mut choice = Map::new();
        choice.insert("index".to_owned(), Value::from(0));
        choice.insert("delta".to_owned(), Value::Object(delta));
        choice.insert(
            "finish_reason".to_owned(),
            finish_reason_to_wire(chunk.finish_reason).map_or(Value::Null, Value::from),
        );

        let mut view = Map::new();
        view.insert("id".to_owned(), Value::from(self.id.as_str()));
        view.insert("object".to_owned(), Value::from(CHUNK_OBJECT));
        view.insert("created".to_owned(), Value::from(self.created));
        view.insert("model".to_owned(), Value::from(self.model.as_str()));
        view.insert(
            "choices".to_owned(),
            Value::Array(vec![Value::Object(choice)]),
        );

        Some(Value::Object(view).to_string())
    }

    /// **`usage` だけを受け取る**（後勝ち。`None` なら持っている値のまま）。
    ///
    /// 保留するチャンク（`finish_reason` 付き / モジュールの doc）は、本文の差分を流すときまで蓄積しないが、
    /// `usage` はその場で受け取る —— 後から届く使用量だけのチャンクに負けるように（届いた順の後勝ち）。
    fn note_usage(&mut self, usage: Option<TokenUsage>) {
        self.usage = usage.or(self.usage);
    }

    /// **2回目の上流の前に、1回目の蓄積と `usage` を捨てる**（ADR-0059 §決定3）。
    ///
    /// 保存するのは採用した回（2回目）の生の出力、清算するのは2回目の `usage`。**役割を載せたか（`started`）は
    /// 捨てない** —— 2回目は同じ応答の続きなので、役割をもう一度載せない。
    fn restart(&mut self) {
        self.content.clear();
        self.usage = None;
    }

    /// ここまでに蓄積した本文。**完結したときに保存する本文そのもの。**
    pub(super) fn accumulated(&self) -> &str {
        &self.content
    }

    /// 上流が申告したトークン数。**無ければ「答え合わせできない」。**
    pub(super) const fn usage(&self) -> Option<TokenUsage> {
        self.usage
    }
}

/// **使用量だけを載せたチャンクか。**
///
/// `stream_options.include_usage`（`llm::request::wire_request`）を要求したときに、
/// 上流が本文を出し切った後で送ってくる `{"choices":[],"usage":{…}}` がこれ。
///
/// **「増分が空」だけで判定しないこと。** 役割だけを名乗る先頭チャンクや
/// 停止理由だけの最終チャンクも増分は空で、**そちらは下流へ流す**
/// （OpenAI 互換クライアントが期待する形）。3つ揃って初めて
/// 「本文としては何も持っていない、使用量の申告」になる。
const fn is_usage_only(chunk: &ChatChunk) -> bool {
    chunk.usage.is_some() && chunk.delta.is_empty() && chunk.finish_reason.is_none()
}

/// 中継の終わりに1回だけ走る記録。**`chunk_events` から呼ぶのはこれだけ。**
///
/// 単体テストから直接呼べるように切り出してある（`server::usage` が
/// `log_check` を `report` から分けてあるのと同じ形）。
///
/// # 答え合わせ（ADR-0013 第3層）
///
/// **最終チャンクに `usage` が載っていたときだけ**走る。載っていなければ
/// 「答え合わせできない」だけなので**黙る**（`report` の中で `debug`）。
/// ここで `warn` を出すと、`usage` を載せない上流に繋いだ瞬間から
/// **応答1本ごとに鳴り続け**、本当に見たい警告が埋もれる。
///
/// # 封の行（ADR-0052 §推奨5 / 作業単位 B-60b・B-60d-1）
///
/// **答え合わせと同じ `usage`（最終チャンクのもの）**で、封の行を1本出す（`super::seal::report`）。
/// **途中で壊れた回も出す**（答え合わせと同じ。実数は `None` になりうる）。
///
/// **上界の判定はバイト数の計画を送った回だけ、超過の判定は毎回**（`TurnSeal::estimate` / 非ストリーミングと同じ）。
///
/// **封をした回の答え合わせ（`封 < 実` なら掛け金）もここ**（`super::seal::Sealed::settle` / 作業単位 B-60d-2）。
/// 中継の `usage` は最後のチャンクで届くので、判定できるのは中継の終わりだけである。
pub(super) fn finish_relay(
    relay: &Relay,
    budget: TokenBudget,
    plan: &BudgetPlan,
    max_tokens: u32,
    sealed: &Sealed,
    message: &'static str,
) {
    usage::report(plan, sealed.estimate(), budget, max_tokens, relay.usage());
    sealed.settle(plan, relay.usage(), true);

    // 蓄積した本文そのものはログに出さない（LLM 出力を垂れ流さない）。
    tracing::info!(chars = relay.accumulated().chars().count(), "{}", message);
}

/// 中継中に持ち回る状態。
struct RelayState {
    /// **`None` になったら終端**（`[DONE]` を流した後、または失敗を流した後）。
    upstream: Option<ChatStream>,
    relay: Relay,
    budget: TokenBudget,
    plan: BudgetPlan,
    max_tokens: u32,
    /// **送る前に取った封のまとめ**と、その世代・記憶（`super::seal::Sealed`）。終わりの1行に載せ、答え合わせする。
    sealed: Sealed,
    /// **完結した回の保存先**（11b-2）。途中で壊れた枝では使わない。
    turn: TurnSink,
    /// **上流から1件でも受け取ったか**（11c-3a → B-6）。
    ///
    /// # ⚠ **免除の門ではなく、報告の欄になった**（作業単位 B-6）
    ///
    /// [ADR-0019](../../../docs/adr/0019-relay-idle-watchdog.md) §2 は
    /// 最初の1件を見張りの**外**に置いていた。理由は**「JIT ロードの待ちを
    /// 打ち切ってしまうから」**で、**打ち切らなくなった以上その理由は消えた。**
    ///
    /// いまここが決めるのは、報告の `after_first_token` 欄だけである
    /// ——`false` なら「まだ1トークンも来ていない」（JIT ロード中かもしれない）、
    /// `true` なら「**このターンの上流が、もう1件は返した後の沈黙**」。**読み手にとって意味がまるで違う。**
    ///
    /// **⚠ `true` は「喋り出してから止まった」より広い**（ADR-0059 §決定3 / 作業単位 A-18b）。
    /// 空の本文を引き直した回は、**2回目の上流を開いて最初のチャンクを待つ間の沈黙**も `true` で報告される
    /// （1回目が返した後なので）。2回目のプロンプトの処理の待ちは、ここでは「喋り出してから止まった」と
    /// 見分けが付かない。分解の行の `empty_retries` で、そのターンが引き直したかが分かる。
    ///
    /// **使用量だけのチャンクも「来た1件」に数える**
    /// （下流へ出ないだけで、上流が喋っている証拠ではある）。
    received_first: bool,
    /// **空の本文の引き直しに使う、同じ上流の写しと同じ要求**（ADR-0059 §決定3）。
    ///
    /// **引き直したら `None`**（2回目を開いた時点で取り出す）—— 2回目も空でも3回目は無い。
    redo: Option<Redo>,
    /// **保留している `finish_reason` 付きのチャンク**（ADR-0059 §決定3 / モジュールの doc）。
    ///
    /// 1回目の上流が尽きた時点で `Some` なら、引き直す。2回目が開けなければ、このチャンクをそのまま流す。
    held: Option<ChatChunk>,
    /// **2回目が開けなかったので、次の1件で `[DONE]` を流して終わる**（保留したチャンクを流した後）。
    finish_next: bool,
    /// 引き直しの記録（分解の行の欄 / `super::empty_retry::EmptyRetry`）。
    retry: EmptyRetry,
}

/// **空の本文を引き直すための、そのターンの上流の写しと要求**（ADR-0059 §決定3 / 作業単位 A-18b）。
///
/// 写しは `AppState::llm_for_turn`（封と1回目の送信に使ったもの）—— **途中で上流が切り替わっても、
/// 引き直しは1回目と同じ上流へ送る。** 要求は1回目と同じもの（`counted_plan` が作った本文）。
pub(super) struct Redo {
    llm: Arc<dyn LlmClient>,
    request: ChatRequest,
}

impl Redo {
    /// そのターンの写しと、1回目に送った要求。
    pub(super) const fn new(llm: Arc<dyn LlmClient>, request: ChatRequest) -> Self {
        Self { llm, request }
    }
}

/// 上流のチャンク列を SSE の応答にする。**溜め込まない**（`finish_reason` と `[DONE]` の保留だけ / モジュールの doc）。
#[expect(
    clippy::too_many_arguments,
    reason = "中継1本分の持ち物を `chat` から1回で渡す。束ねる型を作っても、作る側で同じ数を並べる"
)]
pub(super) fn relay_response(
    upstream: ChatStream,
    relay: Relay,
    budget: TokenBudget,
    plan: BudgetPlan,
    max_tokens: u32,
    sealed: Sealed,
    turn: TurnSink,
    redo: Redo,
) -> Response {
    Sse::new(chunk_events(RelayState {
        upstream: Some(upstream),
        relay,
        budget,
        plan,
        max_tokens,
        sealed,
        turn,
        // 最初の1件はまだ来ていない。**報告の欄がそう名乗る**（免除ではない / B-6）。
        received_first: false,
        redo: Some(redo),
        held: None,
        finish_next: false,
        retry: EmptyRetry::default(),
    }))
    .into_response()
}

impl RelayState {
    /// **このチャンクを保留するか**（ADR-0059 §決定3）。チャンクを蓄積する**前**に聞く。
    ///
    /// `finish_reason` 付きで、まだ引き直していなくて、次のどちらか:
    ///
    /// * **既に保留がある**（判定は最初の `finish_reason` 付きのチャンクで済んでいる。後のものは保留に足す /
    ///   [`Self::hold`]）。
    /// * 積み上げた本文（**このチャンクの差分を含む**）が、中継で引き直す空（閉じない思考ではない）。
    fn holds(&self, chunk: &ChatChunk) -> bool {
        chunk.finish_reason.is_some()
            && self.redo.is_some()
            && (self.held.is_some()
                || empty_retry::retries(
                    &format!("{}{}", self.relay.accumulated(), chunk.delta),
                    Delivery::Stream,
                ))
    }

    /// **保留する。** 既に保留があれば、**上書きせずに足す**（差分を後ろに連結し、`finish_reason` は後のもの）。
    ///
    /// 上書きすると、先の差分が流れず蓄積からも落ちる（`raw_content` が欠ける / 絶対制約）。
    /// `usage` はその場で受け取る（後から届く使用量だけのチャンクに負けるように / `Relay::note_usage`）。
    fn hold(&mut self, chunk: ChatChunk) {
        self.relay.note_usage(chunk.usage);
        let delta = match self.held.take() {
            Some(earlier) => earlier.delta + &chunk.delta,
            None => chunk.delta,
        };
        self.held = Some(ChatChunk {
            delta,
            finish_reason: chunk.finish_reason,
            usage: None,
        });
    }

    /// **完結した回の締め**: 答え合わせと封の清算（採用した回の `usage` で1回）、保存、`[DONE]`。
    async fn complete(&mut self) -> String {
        self.upstream = None;
        finish_relay(
            &self.relay,
            self.budget,
            &self.plan,
            self.max_tokens,
            &self.sealed,
            "SSE 中継を完了しました",
        );
        // **応答が完全に返り切った回だけ保存する。**
        // `[DONE]` の直前なので、失敗しても中継の成否は変えられない
        // （`super::turn` は `Result` を返さない）。
        self.turn
            .persist(self.relay.accumulated(), self.retry)
            .await;
        DONE_DATA.to_owned()
    }
}

/// 1チャンク受けるたびに1イベント返すストリーム。
///
/// `unfold` は**次が要求されるまで上流を読まない**ので、下流の消費速度がそのまま
/// 反映される（`llm::lmstudio::chunk_stream` と同じ作法）。
///
/// 終わり方は2通り。**どちらも必ず終端する**（接続を宙吊りにしない）。
///
/// | 上流の様子 | 下流に見えるもの | 保存 |
/// |---|---|---|
/// | 尽きた（`[DONE]` を受けた） | `data: [DONE]` を流して終端 | **する** |
/// | **本文が空のまま尽きた**（ADR-0059 §決定3） | 2回目の上流を開いて続ける（モジュールの doc の表）。終端は「尽きた」か「途中で失敗した」の行と同じ | 2回目（開けなければ1回目） |
/// | 途中で失敗した（**天井に当たった回を含む**） | error オブジェクトを1件流して終端（**`[DONE]` は送らない**） | **しない** |
/// | **黙り込んだ** | **何も見えない**（`warn` がログに出るだけ。中継は続く） | — |
///
/// # 無通信の報告（11c-3a → ADR-0035 / 作業単位 B-6）
///
/// 見張りを巻くのは **`events.next()` の1件分**（[`next_reporting_silence`]）と、**空の本文を引き直す
/// 2回目の上流を開く future**（[`open_reporting_silence`] / ADR-0059 §決定3）の2つで、掛けるのは**1件ごとの間隔**。
/// **やるのは報告だけで、打ち切りはしない。**
///
/// * **最初の1件も報告する。** かつては免除していたが、それは
///   「JIT ロードの待ちを**打ち切って**しまうから」だった。打ち切らない以上、
///   免除の理由は無い。**代わりに `after_first_token` の欄で見分ける。**
/// * **使用量だけのチャンクで `continue` した後も掛かり直す**（見張りはループの中）。
///   持ち越すと、**下流から見えない1件のせいで「黙っている」と言い出す。**
/// * **保存（`persist`）は `next()` の外**なので、どれだけ遅くても報告されない
///   （`a_very_slow_persist_is_outside_the_watchdog_budget` で実測）。
fn chunk_events(state: RelayState) -> impl Stream<Item = Result<Event, Infallible>> + Send {
    stream::unfold(state, |mut state| async move {
        // **流すものが決まるまで上流を読み続ける。** 使用量だけのチャンクは
        // 受け取っても下流へ出さないので、その場合だけここを回る（`Relay::relay`）。
        loop {
            // **2回目が開けなかった回の締め**（保留したチャンクを前の1件で流した後 / ADR-0059 §決定3）。
            if std::mem::take(&mut state.finish_next) {
                let data = state.complete().await;
                return Some((Ok(Event::default().data(data)), state));
            }

            // 終端済み。**`fuse` 相当**（もう一度読まれても `None` を返し続ける）。
            let events = state.upstream.as_mut()?;

            // **黙っていれば報告しながら、来るまで待つ**（打ち切らない）。
            let received = next_reporting_silence(events, state.received_first).await;
            // **使用量だけのチャンクも「来た1件」に数える**
            // （下流へ出ないだけで、上流が喋っている証拠ではある）。
            state.received_first = true;

            let data = match received {
                // **本文が空のまま終わった。保留して、1回目の上流が尽きるのを待つ**（ADR-0059 §決定3）。
                // **`usage` はその場で受け取り、本文の差分は流すときまで蓄積しない**（流すときに
                // `Relay::relay` を通すので、2回蓄積しないように / `RelayState::hold`）。
                Some(Ok(chunk)) if state.holds(&chunk) => {
                    state.hold(chunk);
                    continue;
                }
                // **受けたそばに1件返す。** ここで溜めると体感が死ぬ。
                Some(Ok(chunk)) => match state.relay.relay(&chunk) {
                    Some(data) => data,
                    // 蓄積と答え合わせには使った。下流へは流さない。
                    None => continue,
                },
                Some(Err(err)) => {
                    // 上流が壊れた／切れた／**天井に当たった**。
                    // 直後に必ず終端する（`llm::ChatStream` の doc）。
                    state.upstream = None;
                    finish_relay(
                        &state.relay,
                        state.budget,
                        &state.plan,
                        state.max_tokens,
                        &state.sealed,
                        "SSE 中継が完了前に終わりました",
                    );
                    // **保存しない。** 途中までの応答を履歴に混ぜない（モジュールの doc）。
                    error_data(&err)
                }
                // **1回目が空のまま尽きた。同じ写し・同じ要求で2回目を開く**（ADR-0059 §決定3）。
                None if state.held.is_some() => {
                    let (Some(held), Some(redo)) = (state.held.take(), state.redo.take()) else {
                        // 到達しない（保留は `redo` が在るときだけ / `holds`）。締めへ回す。
                        state.finish_next = true;
                        continue;
                    };
                    // **2回目を待つ間の沈黙も報告する**（打ち切らない / `received_first` の doc）。
                    match open_reporting_silence(redo.llm.stream(&redo.request)).await {
                        Ok(second) => {
                            // **保留した差分は捨てない**（閉じタグでありうる / モジュールの doc）。
                            // `finish_reason` だけを外して流す。空なら何も流さない。
                            // **1回目の蓄積を捨てる前に通す**（2回目の蓄積に混ぜない）。
                            let carried = if held.delta.is_empty() {
                                None
                            } else {
                                state.relay.relay(&ChatChunk {
                                    finish_reason: None,
                                    ..held
                                })
                            };
                            // **捨てる1回目を記録してから、蓄積と `usage` を捨てる**（保存も清算も2回目で1回）。
                            state.retry = EmptyRetry::discarded(
                                state.relay.accumulated(),
                                state.relay.usage(),
                            );
                            state.relay.restart();
                            state.upstream = Some(second);
                            match carried {
                                Some(data) => data,
                                None => continue,
                            }
                        }
                        Err(err) => {
                            // **引き直しが無かったときと同じ結果にする**: 保留したチャンクをそのまま流し、
                            // 次の1件で `[DONE]`（1回目の空を保存する）。
                            empty_retry::log_retry_failure(&err, Delivery::Stream);
                            state.retry = EmptyRetry::failed();
                            state.finish_next = true;
                            match state.relay.relay(&held) {
                                Some(data) => data,
                                // 到達しない（`finish_reason` 付きは使用量だけのチャンクではない）。
                                None => continue,
                            }
                        }
                    }
                }
                None => state.complete().await,
            };

            return Some((Ok(Event::default().data(data)), state));
        }
    })
}

/// 途中で壊れたことを伝えるデータ。**非ストリーミングと同じ定数を使う。**
///
/// 案内の出し分け（接続できない／時間切れ／その他）も `upstream_error` に任せるので、
/// **同じ失敗が経路によって別の文言になることがない。**
fn error_data(err: &LlmError) -> String {
    upstream_error(err).as_event_data()
}

/// 上流の次の1件を待つ。**黙っている間は [`IDLE_BUDGET`] ごとに `warn` を1本出す。**
///
/// **打ち切らない**（ADR-0035 §決定1）。ここが返らないまま伸び続けたら、
/// 止めるのは `llm` 層の天井か、**待っている人**（画面の「中止」/ §決定4）である。
///
/// # 何を出すか
///
/// | 欄 | 中身 |
/// |---|---|
/// | `silent_secs` | **この沈黙が始まってからの通算の秒数。** 間隔ではなく実際の経過 |
/// | `after_first_token` | `false` なら**まだ1トークンも来ていない**（JIT ロード中かもしれない）。`true` は「このターンの上流が1件は返した後」で、**空の本文を引き直す2回目を待つ沈黙も含む**（`RelayState::received_first` の doc / ADR-0059 §決定3） |
///
/// **秒数は間隔ではなく経過にする。** 間隔（＝定数）を出すと、
/// **1本目と5本目のログが同じ文字列**になり、
/// 「まだ続いているのか、また止まったのか」が読めない。
///
/// # 繰り返し出る
///
/// 黙っている限り [`IDLE_BUDGET`] ごとに出続ける。**天井までに数本**——
/// `llm` の既定の天井（180秒）なら 5 本で、天井を伸ばせばそのぶん増える。
/// **打ち切らない以上、沈黙はいくらでも伸びうる**ので、
/// 1本で終わらせると「まだ続いている」を誰も言わなくなる。
///
/// # ⚠ **期限のたびに `events.next()` を作り直してよい**（B-6 で実測）
///
/// **`StreamExt::next` の future は状態を持たない**（`&mut ChatStream` を握るだけで、
/// 途中まで読んだ分は**ストリームの側**にある）。**落として作り直しても1件も失われない。**
///
/// ~~`events.next()` を1回だけ作り、`timeout` には `&mut` で渡すこと。~~
/// **書いてみて、変異で確かめた** —— `&mut pending` を
/// `events.next()` に置き換える変異は **1573 全緑**だった（**等価変異**）。
/// **効果を測れない仕掛けは残さない**（CLAUDE.md 4節 / `Mutex` を外した回と同じ判断）ので、
/// **素直に毎回作る形にしてある。**
///
/// **⚠ 他の future で同じことをしないこと。** これは
/// `StreamExt::next` が状態を持たないという性質に寄りかかっている。
/// 途中まで読んだ分を自分で抱える future（`AsyncRead::read_exact` 等）を
/// `timeout` で巻き直すと、**そちらは本当に失う。**
async fn next_reporting_silence(
    events: &mut ChatStream,
    after_first_token: bool,
) -> Option<Result<ChatChunk, LlmError>> {
    let started = tokio::time::Instant::now();

    loop {
        match tokio::time::timeout(IDLE_BUDGET, events.next()).await {
            Ok(received) => return received,
            Err(_elapsed) => report_silence(started, after_first_token),
        }
    }
}

/// **空の本文を引き直す2回目の上流を開く。** 黙っている間は [`next_reporting_silence`] と同じ行を出す
/// （ADR-0059 §決定3 / 作業単位 A-18b）。
///
/// 開く（ヘッダが返る）までの待ちは、上流によってはプロンプトの処理を含む。1回目が返した後の沈黙なので
/// `after_first_token=true` で言う（`RelayState::received_first` の doc）。
///
/// **こちらは future を1回だけ作り、`&mut` で巻き直す**（[`next_reporting_silence`] の doc の ⚠ ——
/// 開く future は途中の状態（送りかけの要求）を自分で持つので、作り直すと失う）。
async fn open_reporting_silence(
    mut opening: LlmFuture<'_, ChatStream>,
) -> Result<ChatStream, LlmError> {
    let started = tokio::time::Instant::now();

    loop {
        match tokio::time::timeout(IDLE_BUDGET, &mut opening).await {
            Ok(opened) => return opened,
            Err(_elapsed) => report_silence(started, true),
        }
    }
}

/// 沈黙の報告の1行（[`next_reporting_silence`] の doc の表）。**応答には何も出さない**（絶対制約）。
fn report_silence(started: tokio::time::Instant, after_first_token: bool) {
    tracing::warn!(
        silent_secs = started.elapsed().as_secs(),
        after_first_token,
        "上流からトークンが届かないまま待っています"
    );
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{self, AtomicBool};
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use serde_json::json;
    use tracing::span::{Attributes, Record};
    use tracing::subscriber::Interest;
    use tracing::{Event, Id, Metadata, Subscriber};

    use super::*;
    use crate::domain::fixture_scale;
    use crate::domain::{Character, CharacterId, DisplayName, Handle, Persona, Role, SessionId};
    use crate::llm::FinishReason;
    use crate::prompt::{CountedParts, PromptInput, build_prompt};
    use crate::server::empty_retry::test_upstream::Again;
    use crate::server::extract::Extraction;
    use crate::server::seal::TurnSeal;
    use crate::store::Store;
    use crate::testlog::{
        VERBOSE_FILTER, captured_under_default_filter, captured_with_filter, has_field,
        line_containing,
    };

    /// 上界が破れた側のログの手掛かり。**全文一致にしないこと。**
    const UPPER_BOUND_MARKER: &str = "上界が破れ";

    /// 無通信を**告げる**側の手掛かり（作業単位 B-6 / ADR-0035 決定2）。
    ///
    /// **全文一致にしないこと**——文言の推敲で、見張りたい
    /// 「秒数がこの行に載るか」と無関係に割れる（[`UPPER_BOUND_MARKER`] と同じ理由）。
    ///
    /// # ⚠ 締めの文言と取り違えないこと
    ///
    /// `finish_relay` が出す `info` は**2種**（完了 / 完了前に終わり）で、
    /// **欄は `chars=` だけ。** その2つは [`FINISH_MESSAGES`] が全部並べている。
    /// 秒数（`silent_secs`）が載るのは**こちら（`warn`）だけ**である。
    /// **B-2 で実際に取り違えた**ので、
    /// **締めのどちらにも含まれない部分文字列**にしてある
    /// （その性質は使う側が前提として確かめる）。
    ///
    /// **⚠ 「中止」という締めの文言は存在しない**（レビュー指摘 R-4 で訂正）。
    /// **`grep '"SSE 中継' src/` の全件は3つ**で、`finish_relay` から出るのは上の2つ、
    /// 残る1つは `server::chat` の**開始**ログ（`"SSE 中継を開始します"`）である。
    const IDLE_REPORT_MARKER: &str = "届かないまま待って";

    /// `usage` の欠落を記録した側（`debug`）の手掛かり。
    const MISSING_MARKER: &str = "答え合わせできません";

    /// 答え合わせに使う想定コンテキスト長。**生成用の余白（1024）とは別の値。**
    const CONTEXT: u32 = 4_096;

    /// 上流へ渡した生成上限。**余白とも想定コンテキストとも別の値。**
    const MAX_TOKENS: u32 = 16;

    fn budget() -> TokenBudget {
        TokenBudget::new(CONTEXT, TokenBudget::DEFAULT_RESPONSE_TOKENS)
            .expect("テストで使う予算は正当であること")
    }

    /// 実際に組み立てたプロンプトの配分。
    ///
    /// **`server::usage` の「どの項目を読むか」を見張っているのはあちらのテスト。**
    /// ここで要るのは「正当な `BudgetPlan` が1つある」ことだけなので、
    /// 記憶・履歴は空でよい。
    fn plan() -> BudgetPlan {
        let character = Character {
            id: CharacterId::from_row(1),
            handle: Handle::parse("aoi").expect("正当な名前であること"),
            display_name: DisplayName::parse("葵").expect("正当な表示名であること"),
            base_persona: Persona::parse("落ち着いた口調で話す。")
                .expect("正当なペルソナであること"),
            output_format_instruction: None,
            stat_scale: fixture_scale(),
            stat_drift_mode: None,
            axis_discovery_slots: crate::domain::AxisDiscoverySlots::NONE,
            created_at: Utc::now(),
            archived_at: None,
        };

        build_prompt(
            &PromptInput {
                character: &character,
                stats: &[],
                attributes: &[],
                agreements: &[],
                memories: &[],
                history: &[],
                user_message: "こんにちは",
            },
            budget(),
            &CountedParts::default(),
        )
        .expect("テストのプロンプトは予算に収まること")
        .plan
    }

    fn relay() -> Relay {
        Relay::new(
            "chatcmpl-test".to_owned(),
            "aoi@42".to_owned(),
            1_760_000_000,
        )
    }

    fn chunk(delta: &str, finish_reason: Option<FinishReason>) -> ChatChunk {
        ChatChunk {
            delta: delta.to_owned(),
            finish_reason,
            usage: None,
        }
    }

    /// **使用量だけを載せたチャンク。** `stream_options.include_usage` を要求した
    /// 上流が、本文を出し切った後に送る形（`choices` が空 → `llm::parse_chunk` が
    /// 空の増分に均す）。
    fn usage_only(prompt_tokens: u32) -> ChatChunk {
        ChatChunk {
            delta: String::new(),
            finish_reason: None,
            usage: Some(TokenUsage {
                prompt_tokens,
                completion_tokens: None,
            }),
        }
    }

    /// 中継の保存先に使う利用者の発言。**応答の本文とは別の文字列。**
    const USER_MESSAGE: &str = "ただいま、葵";

    /// 一時 DB の上に「キャラクター1件 ＋ セッション1件」を作る。
    ///
    /// 中継は**完結した回だけ**往復を書く（`RelayState::turn`）ので、
    /// 「書いたか／書かなかったか」を確かめるには本物の `Store` が要る。
    async fn fresh_session(path: &std::path::Path) -> (Store, SessionId) {
        let store = Store::connect(path).await.expect("DB を開けること");
        let character = store
            .create_character(
                &Handle::parse("aoi").expect("正当な名前であること"),
                &DisplayName::parse("葵").expect("正当な表示名であること"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");
        let session = store
            .create_session(character)
            .await
            .expect("セッションを作成できること");
        (store, session)
    }

    /// 中継を最後まで走らせ、**下流へ出たイベントの件数と、保存された往復**を返す。
    ///
    /// `axum` の `Event` は中身を読み出せないので、ここで見えるのは件数だけ
    /// （中身は `tests/server_chat_stream.rs` が見る）。
    ///
    /// **`plan` は引数で受ける。** プロンプト組立を観測窓の中でやると
    /// `prompt` 層のログが混ざる。
    fn relayed(
        items: Vec<Result<ChatChunk, LlmError>>,
        plan: BudgetPlan,
    ) -> (usize, Vec<crate::domain::Message>) {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");

        // **現在のスレッドで走らせる。** ログの捕捉はスレッドローカルなので、
        // 別スレッドへ飛ばすと `captured_under_default_filter` から見えなくなる。
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime を作れること")
            .block_on(async {
                let (store, session_id) = fresh_session(&dir.path().join("relay.sqlite")).await;

                let upstream: ChatStream = Box::pin(stream::iter(items));
                let state = RelayState {
                    upstream: Some(upstream),
                    relay: relay(),
                    budget: budget(),
                    plan,
                    max_tokens: MAX_TOKENS,
                    // 封はここの見張り番の外（`super::seal` と `tests/server_seal.rs` の担当）。
                    sealed: Sealed::for_tests(TurnSeal::remembered()),
                    // 中継のテストは軸を持たない（漏れの判定は `super::turn` の担当）。
                    turn: TurnSink::new(
                        store.clone(),
                        session_id,
                        USER_MESSAGE,
                        Vec::new(),
                        Extraction::quiet_for_tests(store.clone()),
                    ),
                    // **本番の入口と同じ初期値。** この helper の上流は
                    // 待たずに次を返すので、無通信の報告は最後まで出ない。
                    received_first: false,
                    // **引き直しは開けない**（空の回は、引き直しが無かったときと同じ結果になる）。
                    redo: Some(unreachable_redo()),
                    held: None,
                    finish_next: false,
                    retry: EmptyRetry::default(),
                };

                let events = Box::pin(chunk_events(state)).count().await;
                let saved = store
                    .recent_messages(session_id, 10)
                    .await
                    .expect("会話履歴を読めること");
                store.close().await;
                (events, saved)
            })
    }

    /// 下流へ出たイベントの件数だけを見る呼び出し。
    fn relayed_event_count(items: Vec<Result<ChatChunk, LlmError>>, plan: BudgetPlan) -> usize {
        relayed(items, plan).0
    }

    /// **蓄積した本文は、流したチャンクの連結と一致する**（11b の永続化が使う）。
    ///
    /// ここが欠けると、11b は「返したはずの応答」を保存できない。
    #[test]
    fn the_accumulated_body_is_the_concatenation_of_the_relayed_chunks() {
        let mut relay = relay();
        assert_eq!(relay.accumulated(), "", "最初は空であること");

        let deltas = ["「こん", "", "にちは」", " 彼女は微笑んだ。"];
        for delta in deltas {
            let _ = relay.relay(&chunk(delta, None));
        }

        assert_eq!(
            relay.accumulated(),
            "「こんにちは」 彼女は微笑んだ。",
            "流した増分の連結そのものであること"
        );
        assert_eq!(
            relay.accumulated(),
            deltas.concat(),
            "順序も落とし物も無いこと"
        );
    }

    /// 流すデータは OpenAI 互換のチャンク。**id / model / created は全チャンク共通。**
    #[test]
    fn a_relayed_chunk_is_an_openai_compatible_chunk() {
        let mut relay = relay();

        let first: Value = serde_json::from_str(
            &relay
                .relay(&chunk("「こん", None))
                .expect("本文のチャンクは下流へ流すこと"),
        )
        .expect("JSON として読めること");
        assert_eq!(
            first,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion.chunk",
                "created": 1_760_000_000,
                "model": "aoi@42",
                "choices": [{
                    "index": 0,
                    // **先頭チャンクだけが役割を名乗る。**
                    "delta": {"role": "assistant", "content": "「こん"},
                    "finish_reason": null,
                }],
            })
        );

        let last: Value = serde_json::from_str(
            &relay
                .relay(&chunk("にちは」", Some(FinishReason::Stop)))
                .expect("本文のチャンクは下流へ流すこと"),
        )
        .expect("JSON として読めること");
        assert_eq!(
            last,
            json!({
                "id": "chatcmpl-test",
                "object": "chat.completion.chunk",
                "created": 1_760_000_000,
                "model": "aoi@42",
                "choices": [{
                    "index": 0,
                    // 2件目以降に役割は載らない。
                    "delta": {"content": "にちは」"},
                    "finish_reason": "stop",
                }],
            })
        );
    }

    /// 停止理由の綴りは非ストリーミングと同じ規則（**`Other` を `"stop"` と偽らない**）。
    #[test]
    fn the_finish_reason_follows_the_non_streaming_rule() {
        for (reason, expected) in [
            (Some(FinishReason::Stop), json!("stop")),
            (Some(FinishReason::Length), json!("length")),
            (Some(FinishReason::Other), json!(null)),
            (None, json!(null)),
        ] {
            let value: Value = serde_json::from_str(
                &relay()
                    .relay(&chunk("x", reason))
                    .expect("本文のチャンクは下流へ流すこと"),
            )
            .expect("JSON として読めること");
            assert_eq!(
                value["choices"][0]["finish_reason"], expected,
                "reason = {reason:?}"
            );
        }
    }

    /// **最終チャンクの `usage` を拾い、それ以降も持ち続ける。**
    ///
    /// ストリーミングでは `usage` が最終チャンクにしか載らないので、
    /// ここで落とすと答え合わせ（ADR-0013 第3層）の材料が消える。
    #[test]
    fn the_reported_usage_survives_until_the_end_of_the_relay() {
        let mut relay = relay();
        let _ = relay.relay(&chunk("は", None));
        assert_eq!(relay.usage(), None, "まだ申告されていない");

        let _ = relay.relay(&ChatChunk {
            delta: "い".to_owned(),
            finish_reason: Some(FinishReason::Stop),
            usage: Some(TokenUsage {
                prompt_tokens: 123,
                completion_tokens: None,
            }),
        });
        assert_eq!(
            relay.usage(),
            Some(TokenUsage {
                prompt_tokens: 123,
                completion_tokens: None
            })
        );

        // 後続の（`usage` を持たない）チャンクで消えないこと。
        let _ = relay.relay(&chunk("", None));
        assert_eq!(
            relay.usage(),
            Some(TokenUsage {
                prompt_tokens: 123,
                completion_tokens: None
            }),
            "後から来た None で上書きしないこと"
        );
    }

    /// **`usage` は後勝ち。** 2回申告されたら、後から来た方が残る。
    ///
    /// 上のテストは「後から来た `None` で上書きしない」方向だけを固定していて、
    /// **異なる値を2回渡していない**ので、先勝ち（`self.usage.or(chunk.usage)`）に
    /// 退行しても区別が付かない。
    ///
    /// **`completion_tokens` を足した（A-18b）ので、先勝ちは「最初の1チャンク分」を
    /// 最終値として報告する** —— しかも数字はもっともらしく、捨てた1回目の生成数
    /// （ADR-0059 §決定3）が静かに嘘をつく。2つの申告は生成数も別の値にしてある。
    #[test]
    fn a_later_reported_usage_wins_over_an_earlier_one() {
        let mut relay = relay();

        let first = TokenUsage {
            prompt_tokens: 100,
            completion_tokens: Some(3),
        };
        let last = TokenUsage {
            prompt_tokens: 277,
            completion_tokens: Some(41),
        };
        // **2つが同値だと先勝ち／後勝ちを区別できない**（フィクスチャの取り違え）。
        assert_ne!(first, last, "前提: 2回の申告は別の値であること");

        let _ = relay.relay(&ChatChunk {
            delta: "は".to_owned(),
            finish_reason: None,
            usage: Some(first),
        });
        assert_eq!(relay.usage(), Some(first), "前提: 1回目を拾っていること");

        let _ = relay.relay(&ChatChunk {
            delta: "い".to_owned(),
            finish_reason: Some(FinishReason::Stop),
            usage: Some(last),
        });
        assert_eq!(
            relay.usage(),
            Some(last),
            "後から来た申告で更新すること（先勝ちにしない）"
        );
    }

    /// **`usage` が載っていれば、ストリーミングでも答え合わせが走る**（ADR-0013 第3層）。
    #[test]
    fn a_reported_usage_is_crosschecked_at_the_end_of_the_relay() {
        let mut relay = relay();
        let _ = relay.relay(&ChatChunk {
            delta: "はい".to_owned(),
            finish_reason: Some(FinishReason::Stop),
            // 見積もりを必ず超える値（上界の破れ）。
            usage: Some(TokenUsage {
                prompt_tokens: u32::MAX,
                completion_tokens: None,
            }),
        });

        let output = captured_under_default_filter(|| {
            finish_relay(
                &relay,
                budget(),
                &plan(),
                MAX_TOKENS,
                &Sealed::for_tests(TurnSeal::remembered()),
                "中継を終えました",
            );
        });

        // **申告値は答え合わせの行の中で見る**（作業単位 B-2）。
        //
        // **囮は `usage::log_check` の超過側の `warn`**（B-2-2 / レビュー指摘 R-4 で訂正）。
        // `prompt_tokens = u32::MAX` は上界も超過も同時に破るので、
        // **超過側の行にも `actual_tokens=4294967295` がそのまま載る。**
        // 出力全体で見ると、上界の行が何を書いていても超過の行が答えを供給する。
        //
        // **⚠ 締めの記録（`中継を終えました`）は囮ではない。** 欄は `chars=` だけで、
        // この値を供給できない。B-2 でここに「締めの記録も並んでいる」と書いたのは
        // **取り違え**だった（2026-08-26 に捕捉を実測 / 4行のうち答えを供給するのは超過側の1行）。
        // **欄はトークンとして突き合わせる**（作業単位 B-12）——`contains` だと
        // 桁が伸びた別の値や、キーの側が伸びた別の欄にも当たる。
        let broken = line_containing(&output, UPPER_BOUND_MARKER);
        assert!(
            has_field(broken, "actual_tokens=4294967295"),
            "上流の申告値をそのまま載せること: {broken:?}"
        );
    }

    /// **`usage` が無ければ黙る。** 欠落は「答え合わせできない」であって異常ではない。
    ///
    /// ストリーミングでは `usage` を載せない上流の方が普通なので、ここで警告を出すと
    /// **応答1本ごとに鳴り続ける。**
    ///
    /// **否定の前に肯定を置く**（イベントが丸ごと消えても「何も出なかった」で
    /// 緑になる形にしない / `src/testlog.rs` の doc）。
    #[test]
    fn a_missing_usage_never_warns_at_the_end_of_the_relay() {
        let mut relay = relay();
        let _ = relay.relay(&chunk("はい", Some(FinishReason::Stop)));
        assert_eq!(relay.usage(), None, "前提: 申告が無いこと");

        // **プロンプト組立は観測窓の外で済ませる**（`prompt` 層のログと混ぜない）。
        let plan = plan();

        // 前提: 答え合わせできなかったこと自体は `debug` で残る（＝配線は生きている）。
        let verbose = captured_with_filter(VERBOSE_FILTER, || {
            finish_relay(
                &relay,
                budget(),
                &plan,
                MAX_TOKENS,
                &Sealed::for_tests(TurnSeal::remembered()),
                "中継を終えました",
            );
        });
        assert!(
            verbose.contains(MISSING_MARKER),
            "前提: 欠落は `debug` で残すこと（`report` を呼んでいること）: {verbose:?}"
        );

        let output = captured_under_default_filter(|| {
            finish_relay(
                &relay,
                budget(),
                &plan,
                MAX_TOKENS,
                &Sealed::for_tests(TurnSeal::remembered()),
                "中継を終えました",
            );
        });

        assert!(
            !output.contains("WARN"),
            "usage の欠落で警告を出さないこと: {output:?}"
        );
        assert!(
            !output.contains(UPPER_BOUND_MARKER) && !output.contains(MISSING_MARKER),
            "既定の設定で答え合わせの文言を出さないこと: {output:?}"
        );
        assert!(
            output.contains("中継を終えました"),
            "前提: 中継そのものの記録は残ること（何も出ていないのではない）: {output:?}"
        );
    }

    /// 途中で壊れたときに流すデータは、**非ストリーミングと同じ定数**。
    ///
    /// 経路によって案内が変わると、「LM Studio を起動してください」が
    /// ストリーミングでだけ消える。
    #[test]
    fn the_error_data_reuses_the_non_streaming_constants() {
        let value: Value =
            serde_json::from_str(&error_data(&LlmError::Truncated)).expect("JSON として読めること");

        assert_eq!(
            value,
            json!({
                "error": {
                    "message": "推論サーバとの通信に失敗しました",
                    "type": "server_error",
                    "code": "upstream_failed",
                }
            }),
            "内部情報を載せないこと"
        );

        // 上流のステータス値も反射しない。
        let status: Value = serde_json::from_str(&error_data(&LlmError::Status { status: 429 }))
            .expect("JSON として読めること");
        assert_eq!(status["error"]["code"], json!("upstream_failed"));
        assert!(!status.to_string().contains("429"));
    }

    /// **使用量だけのチャンクを下流へ流さない。**
    ///
    /// `stream_options.include_usage` を要求すると、上流は本文を出し切った後に
    /// `choices` が空のチャンクを送ってくる。`llm::parse_chunk` はこれを
    /// 「空の増分を持つチャンク」に均すので、そのまま中継すると
    /// **`{"delta":{"content":""}}` が1件余分に流れる**（OpenAI 本家は
    /// `choices: []` を送るので、クライアントから見ると別物）。
    ///
    /// 蓄積と答え合わせには使い、下流へは流さない。
    #[test]
    fn a_usage_only_chunk_is_consumed_but_never_relayed() {
        let plan = plan();

        // **受け取ってはいる。** 本文も申告値も落とさない。
        let mut relaying = relay();
        let _ = relaying.relay(&chunk("「はい」", Some(FinishReason::Stop)));
        assert_eq!(
            relaying.relay(&usage_only(321)),
            None,
            "使用量だけのチャンクを下流へ流さないこと"
        );
        assert_eq!(
            relaying.usage(),
            Some(TokenUsage {
                prompt_tokens: 321,
                completion_tokens: None
            }),
            "答え合わせの材料としては受け取ること"
        );
        assert_eq!(relaying.accumulated(), "「はい」", "本文を変えないこと");

        // **「増分が空」だけで落とさない。** 役割だけの先頭チャンクと
        // 停止理由だけの最終チャンクは、増分が空でも下流へ流す。
        assert!(
            relay().relay(&chunk("", None)).is_some(),
            "役割だけの先頭チャンクを落とさないこと"
        );
        assert!(
            relay()
                .relay(&chunk("", Some(FinishReason::Stop)))
                .is_some(),
            "停止理由だけの最終チャンクを落とさないこと"
        );
        assert!(
            relay()
                .relay(&ChatChunk {
                    delta: "い」".to_owned(),
                    finish_reason: Some(FinishReason::Stop),
                    usage: Some(TokenUsage {
                        prompt_tokens: 321,
                        completion_tokens: None
                    }),
                })
                .is_some(),
            "本文と一緒に usage が載っているチャンクを落とさないこと"
        );
        // **停止理由と usage が同居した最終チャンク**（上の2件では届かない形）。
        //
        // 上の「停止理由だけの最終チャンク」は `usage` を持たないので、
        // **第1条件（`usage.is_some()`）で弾かれて停止理由の検査まで到達しない。**
        // 本文を出し切った最終チャンクに停止理由と使用量を同居させる上流
        // （`{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{…}}`）を
        // 落とすと、**クライアントは `finish_reason` を一度も受け取らないまま
        // `[DONE]` だけを見る**——打ち切りか自然停止かが区別できなくなる。
        // `[DONE]` は来るので目視では正常に見え、気付けない。
        assert!(
            relay()
                .relay(&ChatChunk {
                    delta: String::new(),
                    finish_reason: Some(FinishReason::Stop),
                    usage: Some(TokenUsage {
                        prompt_tokens: 654,
                        completion_tokens: None
                    }),
                })
                .is_some(),
            "usage が同乗していても、停止理由を運ぶチャンクは落とさないこと"
        );

        assert_eq!(
            relayed_event_count(
                vec![
                    Ok(chunk("「はい」", Some(FinishReason::Stop))),
                    Ok(usage_only(321)),
                ],
                plan,
            ),
            2,
            "本文1件 ＋ [DONE] のみ（使用量だけのチャンクを流さないこと）"
        );

        // 前提: 中身のあるチャンクなら、同じ本数でも 3 件出る（件数の主張が
        // 「そもそも1件も出ていない」で満たされていないこと）。
        assert_eq!(
            relayed_event_count(
                vec![
                    Ok(chunk("「はい」", Some(FinishReason::Stop))),
                    Ok(chunk("……", None)),
                ],
                plan,
            ),
            3,
            "前提: 本文のあるチャンクは2件とも流れること"
        );
    }

    /// **途中で壊れた中継でも、答え合わせと完了の記録は走る。**
    ///
    /// エラーの枝から `finish_relay` が消えても中継そのものは動くので、
    /// **最も知りたい場面（途中で切れた）でだけ記録が消える**という
    /// 気付けない壊れ方をする。11b はこの枝で「途中まで蓄積した本文を
    /// どうするか」を決めるので、枝が生きていること自体を固定しておく。
    ///
    /// 正常終了側（下のテスト）と**文言で区別する**——取り違えると
    /// 壊れた応答が「完了しました」として記録される。
    #[test]
    fn a_broken_relay_still_crosschecks_and_records_the_end() {
        // **プロンプト組立は観測窓の外で済ませる**（`prompt` 層のログと混ぜない）。
        let plan = plan();

        let output = captured_under_default_filter(|| {
            let events = relayed_event_count(
                vec![
                    Ok(ChatChunk {
                        delta: "ここまで".to_owned(),
                        finish_reason: None,
                        // 見積もりを必ず超える値（上界の破れ）。
                        usage: Some(TokenUsage {
                            prompt_tokens: u32::MAX,
                            completion_tokens: None,
                        }),
                    }),
                    Err(LlmError::Truncated),
                ],
                plan,
            );
            assert_eq!(
                events, 2,
                "本文1件 ＋ error オブジェクト1件（[DONE] は送らない）"
            );
        });

        // **申告値は答え合わせの行の中で見る**（作業単位 B-2）。
        // **囮は `usage::log_check` の超過側の `warn`**（B-2-2 / レビュー指摘 R-4 で訂正）
        // ——`prompt_tokens = u32::MAX` は上界も超過も同時に破るので、
        // **超過側の行にも `actual_tokens=4294967295` が載る。**
        // **締めの記録（`chars=` だけ）は囮ではない**（上の
        // `a_reported_usage_is_crosschecked_at_the_end_of_the_relay` の注記）。
        let crosschecked = line_containing(&output, UPPER_BOUND_MARKER);
        assert!(
            has_field(crosschecked, "actual_tokens=4294967295"),
            "上流の申告値をそのまま載せること: {crosschecked:?}"
        );
        assert!(
            output.contains("SSE 中継が完了前に終わりました"),
            "完了前に終わったことを記録すること: {output:?}"
        );
        assert!(
            !output.contains("SSE 中継を完了しました"),
            "壊れた中継を「完了した」と記録しないこと: {output:?}"
        );
    }

    /// **完結した中継は、蓄積した本文を分解して往復を書く**（11b-2）。
    ///
    /// 下の `a_broken_relay_persists_nothing` と対。片方だけだと
    /// 「常に書く」実装と「完結した回だけ書く」実装が区別できない。
    #[test]
    fn a_completed_relay_persists_the_accumulated_turn() {
        let (events, saved) = relayed(
            vec![
                Ok(chunk("「おかえり", None)),
                Ok(chunk("なさい」 彼女は微笑んだ。", Some(FinishReason::Stop))),
            ],
            plan(),
        );
        assert_eq!(events, 3, "前提: 本文2件 ＋ [DONE]");

        assert_eq!(saved.len(), 2, "往復の2件が入ること: {saved:?}");
        assert_eq!(saved[0].role, Role::User);
        assert_eq!(saved[0].raw_content, USER_MESSAGE);
        assert_eq!(saved[1].role, Role::Assistant);
        assert_eq!(
            saved[1].raw_content, "「おかえりなさい」 彼女は微笑んだ。",
            "流した増分の連結を丸ごと残すこと"
        );
        assert_eq!(saved[1].dialogue.as_deref(), Some("おかえりなさい"));
        assert_eq!(saved[1].narration.as_deref(), Some(" 彼女は微笑んだ。"));
    }

    /// **途中で壊れた中継は1行も書かない**（利用者の発言も）。
    ///
    /// ここで書いてしまうと、`[DONE]` を受けていないクライアントが失敗として扱った回の
    /// 半端な応答だけが履歴に残り、以後の文脈が汚れる。
    #[test]
    fn a_broken_relay_persists_nothing() {
        let (events, saved) = relayed(
            vec![Ok(chunk("「ここまで", None)), Err(LlmError::Truncated)],
            plan(),
        );
        assert_eq!(
            events, 2,
            "前提: 本文1件 ＋ error オブジェクト（[DONE] は無し）"
        );

        assert!(
            saved.is_empty(),
            "途中で壊れた回は利用者の発言も書かないこと: {saved:?}"
        );
    }

    /// **正常に終わった中継でも、答え合わせと完了の記録は走る**（上の対照）。
    #[test]
    fn a_completed_relay_crosschecks_and_records_the_end() {
        let plan = plan();

        let output = captured_under_default_filter(|| {
            let events = relayed_event_count(
                vec![Ok(ChatChunk {
                    delta: "はい".to_owned(),
                    finish_reason: Some(FinishReason::Stop),
                    usage: Some(TokenUsage {
                        prompt_tokens: u32::MAX,
                        completion_tokens: None,
                    }),
                })],
                plan,
            );
            assert_eq!(events, 2, "本文1件 ＋ [DONE]");
        });

        // **申告値は答え合わせの行の中で見る**（作業単位 B-2）。
        // **囮は `usage::log_check` の超過側の `warn`**（B-2-2 / レビュー指摘 R-4 で訂正）
        // ——`prompt_tokens = u32::MAX` は上界も超過も同時に破るので、
        // **超過側の行にも `actual_tokens=4294967295` が載る。**
        // **締めの記録（`chars=` だけ）は囮ではない**（上の
        // `a_reported_usage_is_crosschecked_at_the_end_of_the_relay` の注記）。
        let crosschecked = line_containing(&output, UPPER_BOUND_MARKER);
        assert!(
            has_field(crosschecked, "actual_tokens=4294967295"),
            "正常終了でも答え合わせすること: {crosschecked:?}"
        );
        assert!(
            output.contains("SSE 中継を完了しました"),
            "完了を記録すること: {output:?}"
        );
        assert!(
            !output.contains("SSE 中継が完了前に終わりました"),
            "正常な中継を「完了前に終わった」と記録しないこと: {output:?}"
        );
    }

    // ------------------------------------------------------------------
    // **中継中の無通信の見張り**（作業単位 11c-3a / ADR-0019）
    //
    // ここから下は**実時間を1ミリ秒も待たない。** 時計は `tokio::time::pause` で
    // 止めてあり、進むのは**こちらが明示的に進めたときだけ**——
    // 台本（[`Step::Silence`]）と、保存の区間の [`jump_the_virtual_clock`] の2箇所。
    //
    // **tokio の自動前進は最後まで開けない**（[`run_relay`] の doc）。開けると
    // 仮想時計が sqlx の取得予算まで飛んで、DB が `PoolTimedOut` で落ちる。
    // ------------------------------------------------------------------

    /// 上流の代役に渡す台本の1手。
    #[derive(Debug)]
    enum Step {
        /// **仮想時計を進めるだけ**の手。上流が黙っている時間を作る。
        Silence(Duration),
        /// 上流が1件返す手（時計は進めない）。
        Chunk(Result<ChatChunk, LlmError>),
    }

    /// 時計を進めた後、**上流を黙らせたまま回す回数。**
    ///
    /// `tokio::time::advance` は時計を進めるだけで、**期限切れの通知は
    /// driver がタイマーの輪を処理したときに起きる。** current_thread の
    /// runtime がそれをやるのは `event_interval`（既定 61）tick ごとなので、
    /// 十分な余裕を持って空回りさせる。
    ///
    /// **進めた直後に次の1手を返してはいけない。** `tokio::time::timeout` は
    /// **内側の future を先に polling する**ので、同じ poll で項目が揃うと
    /// 期限切れを観測しないまま `Ok` を返してしまう。
    const YIELDS_AFTER_ADVANCE: usize = 256;

    /// **drop されたら記録する印**（11c-3b）。上流の代役に相乗りさせる。
    ///
    /// 終端が `ChatStream` を**その場で**落としているかを見るために要る。
    /// 中継が終われば応答ごと落ちるので、**いつ立ったか**でしか区別できない。
    struct DropFlag(Arc<AtomicBool>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, atomic::Ordering::SeqCst);
        }
    }

    /// 台本どおりに振る舞う上流。**台本を最後まで進めれば必ず終端する**ので、
    /// 見張りを外す変異を当てても**ハングせずに赤くなる。**
    ///
    /// **`dropped` に drop の印を相乗りさせる**（11c-3b）。印は `unfold` の状態に
    /// 入るので、中継がストリームを別の器へ移し替えても
    /// **その器が生きている限り立たない。**
    fn scripted_upstream(steps: Vec<Step>, dropped: &Arc<AtomicBool>) -> ChatStream {
        let guard = DropFlag(Arc::clone(dropped));
        Box::pin(stream::unfold(
            (steps.into_iter(), guard),
            |(mut rest, guard)| async move {
                loop {
                    match rest.next()? {
                        Step::Silence(wait) => {
                            tokio::time::advance(wait).await;
                            for _ in 0..YIELDS_AFTER_ADVANCE {
                                tokio::task::yield_now().await;
                            }
                        }
                        Step::Chunk(item) => return Some((item, (rest, guard))),
                    }
                }
            },
        ))
    }

    // ------------------------------------------------------------------
    // **保存の区間だけ仮想時計を飛ばす仕掛け**（11c-3a のレビュー指摘）
    // ------------------------------------------------------------------

    /// 保存の入口が記録を出す target。**`super::turn` の `module_path!()`。**
    ///
    /// `TurnSink::persist` は `Store::append_turn` を叩く**前に**分解の結果を
    /// `tracing::info!` で出す（`server::turn::log_outcome`）。**このモジュールから
    /// 出るイベントは保存経路にしか無い**ので、これを捕まえれば
    /// 「保存に入った直後・まだ DB に触っていない」1点を狙える。
    ///
    /// 捕まえられなければ [`relay_with_a_stalled_persist`] が前提で赤くなる。
    ///
    /// # **あの位置に寄りかかっているもの**
    ///
    /// [`a_very_slow_persist_is_outside_the_watchdog_budget`] が
    /// 「`persist` を [`IDLE_BUDGET`] で巻く」変異を捕まえられるのは、
    /// **時計を飛ばす瞬間が DB 書き込みより前にある**からだけ。後ろへ回ると
    /// `tokio::time::timeout` は**内側の future を先に poll する**ので、
    /// 内側が同じ poll で完了すれば期限を過ぎていても `Ok` を返し、
    /// **保存は済んでいるので `saved.len() == 2` が成り立ってしまう。**
    /// `fired` も `relayed.elapsed >= PERSIST_STALL` も**「飛ばした」ことしか
    /// 見ていない**ので、飛ばした位置の違いはそこには出ない。
    ///
    /// **[`the_persist_log_comes_before_the_database_write`] を足す前の実測**
    /// （直列・`cargo test --no-fail-fast`）:
    ///
    /// | 変異 | 結果 |
    /// |---|---|
    /// | `persist` を `timeout(IDLE_BUDGET, …)` で巻く | 1 failed |
    /// | `log_outcome` を `append_turn` の後ろへ（無条件のまま）移す | **848 全緑** |
    /// | 上の2つを**同時に**当てる | **848 全緑**（巻く変異が素通りする） |
    ///
    /// # 何が固定されていて、何が固定されていないか
    ///
    /// * `tests/server_chat_parse_log.rs::a_failure_to_persist_is_recorded_as_an_error`
    ///   が固定しているのは**「保存に失敗した回にも記録が出る」だけ。**
    ///   `else` 枝（成功時だけ）へ移す変異は捕まえるが、
    ///   **`append_turn` の後ろへ動かす変異は捕まえない**（上表2行目）。
    /// * **順序そのもの**は [`the_persist_log_comes_before_the_database_write`] が見る。
    const PERSIST_TARGET: &str = "personacore::server::turn";

    /// 分解を記録した行の手掛かり（`super::turn::log_outcome`）。**全文一致にしない。**
    const PARSE_MARKER: &str = "応答を分解しました";

    /// 往復を書き終えた行の手掛かり（`Store::append_turn` が commit の**後**に出す）。
    ///
    /// **`debug!` なので既定のフィルタ（`personacore=info`）では出ない。**
    const APPEND_MARKER: &str = "会話の往復を追加しました";

    /// [`the_persist_log_comes_before_the_database_write`] で差すフィルタ。
    ///
    /// [`APPEND_MARKER`] が `debug!` なので、既定より1段深く拾う必要がある。
    const PERSIST_ORDER_FILTER: &str = "personacore=debug";

    /// **分解の記録は、DB への書き込みより先に出る。**
    ///
    /// [`PERSIST_TARGET`] の doc の「あの位置」そのものを見る唯一の見張り番。
    /// ここが崩れると [`a_very_slow_persist_is_outside_the_watchdog_budget`] が
    /// 何も測らなくなる（表の3行目）。
    ///
    /// **2つのログの前後で見る。** `Store::append_turn` は commit した後に
    /// [`APPEND_MARKER`] を出すので、`log_outcome` を `append_turn` の後ろへ移すと
    /// 前後が入れ替わる。**「まだ DB に触っていない」より弱い主張**だが
    /// （書き込みの開始ではなく完了と比べている）、狙っている変異は
    /// 「呼び出しごと後ろへ動かす」なので、この位置関係で捕まえられる。
    ///
    /// 中継は通さない。見たいのは `TurnSink::persist` の中の順序だけで、
    /// 仮想時計も上流の台本も要らない。
    ///
    /// **`Store::append_turn` の `debug!` に寄りかかっている。** あれを消すと
    /// ここは「書き込みの記録が出ること」で赤くなる——順序が壊れたのではないので、
    /// 落ちたときは panic の文言まで読むこと。
    #[test]
    fn the_persist_log_comes_before_the_database_write() {
        /// 保存させる応答。**利用者の発言とは別の文字列。**
        const REPLY: &str = "「おかえりなさい」 彼女は微笑んだ。";

        assert_ne!(
            PARSE_MARKER, APPEND_MARKER,
            "前提: 2つの手掛かりは別の文字列"
        );
        assert_ne!(REPLY, USER_MESSAGE, "前提: 応答と発言は別の文字列");

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");

        // **現在のスレッドで走らせる**（ログの捕捉はスレッドローカル）。
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime を作れること");

        // **準備は観測窓の外で済ませる**（キャラクター作成の記録を混ぜない）。
        let (store, session_id) = runtime.block_on(fresh_session(&dir.path().join("order.sqlite")));
        let sink = TurnSink::new(
            store.clone(),
            session_id,
            USER_MESSAGE,
            Vec::new(),
            Extraction::quiet_for_tests(store.clone()),
        );

        let output = captured_with_filter(PERSIST_ORDER_FILTER, || {
            runtime.block_on(sink.persist(REPLY, EmptyRetry::default()));
        });

        // 前提: 保存は現に成功したこと（失敗していれば書き込みの記録は出ない）。
        let saved = runtime
            .block_on(store.recent_messages(session_id, 10))
            .expect("会話履歴を読めること");
        runtime.block_on(store.close());
        assert_eq!(saved.len(), 2, "前提: 往復2件が書けたこと: {saved:?}");

        let parse_at = output
            .find(PARSE_MARKER)
            .unwrap_or_else(|| panic!("分解の記録が出ること: {output:?}"));
        let append_at = output
            .find(APPEND_MARKER)
            .unwrap_or_else(|| panic!("書き込みの記録が出ること: {output:?}"));

        assert!(
            parse_at < append_at,
            "分解の記録を DB への書き込みより先に出すこと（{PERSIST_TARGET} の doc）: {output:?}"
        );
    }

    /// 仮想時計を**その場で**（`await` を挟まずに）飛ばす。
    ///
    /// # なぜ同期でなければならないか
    ///
    /// 飛ばしたい瞬間は次の2つを同時に満たす必要がある。
    ///
    /// 1. **`persist` に入った後**——さもないと `persist` を予算で巻く変異
    ///    （M2）の期限が飛ばした後に張られてしまい、その変異が緑を通り抜ける。
    /// 2. **`Store::append_turn` に入る前**——sqlx のプール取得は
    ///    **30秒の期限を張ってから**接続を ping する。その期限が張られている間に
    ///    30秒を超えて飛ばすと `PoolTimedOut` になり、**保存が落ちる。**
    ///
    /// この2点の間に `await` は1つも無い（`turn_messages` → `log_outcome` →
    /// `append_turn`）。つまり**同期のフック以外では原理的に狙えない。**
    /// `tracing` のイベントは `persist` を走らせているタスクの中で同期に届くので、
    /// subscriber の `event` から時計を動かせばちょうどこの隙間に入る。
    ///
    /// # 1回 poll するだけでよい理由
    ///
    /// `tokio::time::advance` は**本体の先頭で時計を進めてから** `yield_now` を待つ。
    /// 最初の poll で進み切るので、その後は捨ててよい（`yield_now` は
    /// 空回りのタスクが代わりに回している）。
    fn jump_the_virtual_clock(by: Duration) {
        let mut advance = std::pin::pin!(tokio::time::advance(by));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let _ = std::future::Future::poll(advance.as_mut(), &mut cx);
    }

    /// イベントの `message` だけを取り出す visitor。
    ///
    /// **[`StallPersist`] は `with_default` で subscriber を占有する**ので、
    /// `crate::testlog` の捕捉を重ねられない。中身を見たいときは自分で拾う。
    #[derive(Default)]
    struct MessageOf(String);

    impl tracing::field::Visit for MessageOf {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.0 = format!("{value:?}");
            }
        }
    }

    /// 保存に入った瞬間を捕まえて仮想時計を飛ばす subscriber。
    ///
    /// # ⚠ **文言も控える**（作業単位 B-6 / レビュー指摘 R-2）
    ///
    /// 打ち切りがあった頃、「保存は見張りの外」は**保存の有無だけで確かめられた**
    /// （見張りが掛かれば保存が消えるので `saved.len()` で捕まる）。
    /// **B-6 で「掛かっても報告が出るだけ」になったので、主張が2つに割れた** ——
    /// 「保存が消えない」と「**報告が出ない**」。後者は保存の有無に一切現れない。
    ///
    /// そこで、**通り過ぎたイベントの文言をここで控える。**
    /// **`with_default` は subscriber を1つしか差せない**ので、
    /// `captured_under_default_filter` を重ねる形は採れない。
    struct StallPersist {
        /// 飛ばす量。
        stall: Duration,
        /// **1回だけ飛ばす。** `persist` の中の記録は1件だが、
        /// 保存が失敗した回には `error!` も出るので歯止めを置く。
        fired: Arc<AtomicBool>,
        /// **通り過ぎたイベントの文言**（出た順）。使う側が中身を見る。
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl Subscriber for StallPersist {
        /// **`never` に焼き付けさせない**（`crate::testlog` のモジュール doc）。
        fn register_callsite(&self, _metadata: &'static Metadata<'static>) -> Interest {
            Interest::sometimes()
        }

        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn event(&self, event: &Event<'_>) {
            // **先に控える。** 時計を飛ばす枝の中に入れると、
            // 保存以外のイベントが1件も残らない。
            let mut message = MessageOf::default();
            event.record(&mut message);
            if let Ok(mut seen) = self.seen.lock() {
                seen.push(message.0);
            }

            if event.metadata().target() == PERSIST_TARGET
                && !self.fired.swap(true, atomic::Ordering::SeqCst)
            {
                jump_the_virtual_clock(self.stall);
            }
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            // `Id` は 0 を作れないので 1 を返す（panic しない）。
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    /// 中継を**本番の入口**（[`relay_response`]）から1本走らせた結果。
    struct Relayed {
        /// 下流へ出た `data:` の中身（`[DONE]` を含む）。
        events: Vec<String>,
        /// 保存された往復。
        saved: Vec<crate::domain::Message>,
        /// 中継の間に**仮想時計**が進んだ量。
        elapsed: Duration,
        /// **下流へ1件出た「直後」に見た、上流が落ちているかの印**（11c-3b / B-6）。
        ///
        /// [`Relayed::events`] と**同じ長さ・同じ並び。**
        ///
        /// **中継が終わってから見ても意味が無い。** 応答の本文ごと drop されるので、
        /// 実装が何をしていても立つ。**下流へ1件出た瞬間ごと**に見ることで
        /// 「どの枝が、その場で上流を落としたか」だけを取り出せる。
        ///
        /// # なぜ1件目と最後だけでは足りなくなったか（B-6）
        ///
        /// 打ち切りが無くなったので、**「黙っている間も上流を保持し続ける」**が
        /// 新しく主張したいことになった。それは**沈黙の後に出た本文の時点**で
        /// 見るしかない——1件目（沈黙の前）でも最後（`[DONE]` で必ず落ちる）でもない。
        dropped_by_event: Vec<bool>,
    }

    impl Relayed {
        /// 最初の1件が出た時点の印。**`false` でなければ、以後の観測は何も測っていない。**
        fn dropped_at_the_first_event(&self) -> bool {
            self.dropped_by_event.first().copied().unwrap_or(false)
        }

        /// 最後の1件が出た時点の印。**終わりを決めた枝がその場で落としたか。**
        fn dropped_at_the_last_event(&self) -> bool {
            self.dropped_by_event.last().copied().unwrap_or(false)
        }
    }

    /// 上流が黙り込む長さ。**報告の間隔の倍数にしない**（境界がずれたときに気づけるよう、
    /// 間隔より確実に長い値を選ぶ）。
    ///
    /// **報告に載る秒数の期待値でもある。** 間隔（30秒）とは別の値なので、
    /// **欄を定数に固定する変異**（間隔をそのまま出す形）はここで落ちる。
    const STALL: Duration = Duration::from_secs(600);

    /// 報告の間隔に収まる間隔。
    const WITHIN: Duration = Duration::from_secs(20);

    /// **報告の間隔のすぐ内側**。しきい値を縮める変異はここで落ちる。
    const NEARLY: Duration = Duration::from_secs(29);

    /// 最初の1件が来るまでの空白（JIT ロードを模す）。**報告の間隔より長い。**
    const JIT_WAIT: Duration = Duration::from_secs(90);

    /// **保存の区間に開ける空白**（`a_very_slow_persist_…`）。
    ///
    /// * 報告の間隔（30秒）より**桁で長い**——「はるかに超えた」を主張するため。
    /// * ただし sqlx のプール整備タスクの周期（`idle_timeout` の 10 分）より**短い**。
    ///   跨ぐと、この空白の途中で接続が回収されて余計な出入りが混ざる。
    const PERSIST_STALL: Duration = Duration::from_secs(300);

    /// SSE の本文から `data:` の中身だけを取り出す。
    fn data_lines(body: &str) -> Vec<String> {
        body.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(|data| data.trim().to_owned())
            .collect()
    }

    /// 台本どおりの上流で中継を1本走らせる。**実時間では待たない。**
    fn relay_on_a_virtual_clock(steps: Vec<Step>, plan: BudgetPlan) -> Relayed {
        run_relay(steps, plan, None)
    }

    /// **保存の区間で仮想時計を `stall` だけ飛ばして**中継を1本走らせる。
    ///
    /// 飛ばす位置の理屈は [`jump_the_virtual_clock`] の doc を参照。
    /// **捕まえられなかったら前提で赤くする**——飛ばさないまま緑になると、
    /// 「保存は見張りの外」という主張が何も測っていないことになる。
    ///
    /// **通り過ぎたイベントの文言も一緒に返す**（作業単位 B-6 / レビュー指摘 R-2）。
    /// 「報告が出ない」の側は保存の有無に現れないので、
    /// **使う側がこの列を見て初めて主張になる。**
    fn relay_with_a_stalled_persist(
        steps: Vec<Step>,
        plan: BudgetPlan,
        stall: Duration,
    ) -> (Relayed, Vec<String>) {
        let fired = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let relayed = run_relay(
            steps,
            plan,
            Some(StallPersist {
                stall,
                fired: Arc::clone(&fired),
                seen: Arc::clone(&seen),
            }),
        );

        assert!(
            fired.load(atomic::Ordering::SeqCst),
            "前提: 保存の入口（`{PERSIST_TARGET}` の記録）を捕まえたこと"
        );
        let seen = seen.lock().expect("控えた文言を読めること").clone();
        (relayed, seen)
    }

    /// 中継1本の実行本体。
    ///
    /// # 仮想時計を動かすのは**台本と `persist_stall` だけ**
    ///
    /// 空回りのタスク（spinner）で runtime を**最後まで**忙しくしてあるので、
    /// tokio の自動前進は一度も起きない。**ここを緩めてはいけない。**
    /// 暇になった瞬間、仮想時計はそのとき控えている一番近いタイマー
    /// ——実際には **sqlx のプール取得予算（30秒）**——まで飛び、
    /// 接続・保存・読み出しが `PoolTimedOut` で落ちる。
    ///
    /// **`store.close()` まで止めないこと。** 以前は中継が終わった直後に
    /// spinner を落としていたため、**最後の `recent_messages` だけ**が
    /// 自動前進の下に取り残され、`Query(PoolTimedOut)` で落ちていた
    /// （実測: 単独プロセスで 2/240、16並列で 33/160）。
    /// しかもその赤は**変異の赤と見分けが付かない。**
    fn run_relay(
        steps: Vec<Step>,
        plan: BudgetPlan,
        persist_stall: Option<StallPersist>,
    ) -> Relayed {
        run_relay_with(steps, plan, persist_stall, unreachable_redo())
    }

    /// [`run_relay`] に、**空の本文の引き直しの相手**（`redo`）を渡す形（作業単位 A-18b）。
    fn run_relay_with(
        steps: Vec<Step>,
        plan: BudgetPlan,
        persist_stall: Option<StallPersist>,
        redo: Redo,
    ) -> Relayed {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");

        // **現在のスレッドで走らせる**（ログの捕捉はスレッドローカル）。
        // 仮想時計（`start_paused`）は current_thread でしか使えない。
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .expect("runtime を作れること");

        let body = async move {
            // **自動前進の見張り。** runtime が暇にならない限り時計は飛ばない。
            let spinner = tokio::spawn(async {
                loop {
                    tokio::task::yield_now().await;
                }
            });

            let (store, session_id) = fresh_session(&dir.path().join("relay.sqlite")).await;

            // **上流が落ちた瞬間を捕まえる印**（11c-3b）。
            let dropped = Arc::new(AtomicBool::new(false));

            let started = tokio::time::Instant::now();
            let response = relay_response(
                scripted_upstream(steps, &dropped),
                relay(),
                budget(),
                plan,
                MAX_TOKENS,
                // 封はここの見張り番の外（`super::seal` と `tests/server_seal.rs` の担当）。
                Sealed::for_tests(TurnSeal::remembered()),
                // 中継のテストは軸を持たない（漏れの判定は `super::turn` の担当）。
                TurnSink::new(
                    store.clone(),
                    session_id,
                    USER_MESSAGE,
                    Vec::new(),
                    Extraction::quiet_for_tests(store.clone()),
                ),
                redo,
            );

            // **1件ずつ読む。** `to_bytes` で読み切ると、印を見られるのは
            // 応答の本文が drop された後だけになり、**いつ落ちたかが分からない。**
            let mut frames = response.into_body().into_data_stream();
            let mut body = Vec::new();
            let mut dropped_by_event = Vec::new();
            while let Some(frame) = frames.next().await {
                let frame = frame.expect("SSE の本文を読めること");
                body.extend_from_slice(&frame);
                // **この1件が出た「直後」の値。** 次を要求する前に見る。
                dropped_by_event.push(dropped.load(atomic::Ordering::SeqCst));
            }
            // **中継の分だけを測る。** 後始末の読み出しはここに入らない。
            let elapsed = started.elapsed();

            let saved = store
                .recent_messages(session_id, 10)
                .await
                .expect("会話履歴を読めること");
            store.close().await;

            // **DB を触り終えてから**空回りを止める（上記）。
            spinner.abort();

            Relayed {
                events: data_lines(&String::from_utf8_lossy(&body)),
                saved,
                elapsed,
                dropped_by_event,
            }
        };

        match persist_stall {
            None => runtime.block_on(body),
            Some(stall) => tracing::subscriber::with_default(stall, || runtime.block_on(body)),
        }
    }

    /// 下流へ出たチャンクの増分。
    fn delta_of(data: &str) -> String {
        let value: Value = serde_json::from_str(data).expect("JSON として読めること");
        value["choices"][0]["delta"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// `finish_relay` が出す締めの文言（**全部**）。
    ///
    /// **[`IDLE_REPORT_MARKER`] がどれにも当たらないこと**を使う側が前提として確かめる。
    /// B-2 で実際に取り違えた（あのときは打ち切りの締めと警告が語を共有していた）。
    ///
    /// **無通信の締め（「SSE 中継を無通信で打ち切りました」）は B-6 で消えた**
    /// ——中継はもう無通信では終わらないので、その枝ごと無い。
    const FINISH_MESSAGES: [&str; 2] = ["SSE 中継を完了しました", "SSE 中継が完了前に終わりました"];

    /// 沈黙を報告した行だけを取り出す。**判定は必ずこの行の中で行う。**
    ///
    /// 出力全体に対して欄を見ると、**別の行の欄を拾って空振りする**
    /// （CLAUDE.md「ログを見る見張り番は、必ず『同じ行の中で』見ること」）。
    /// [`crate::testlog::line_containing`] は1本しか返さないので、
    /// **繰り返し出る**この行にはこちらを使う。
    fn report_lines<'a>(output: &'a str, marker: &str) -> Vec<&'a str> {
        output
            .lines()
            .filter(|line| line.contains(marker))
            .collect()
    }

    /// ログに載った `silent_secs` の値。**部分一致にしないための取り出し。**
    ///
    /// `contains("silent_secs=600")` は `silent_secs=6000`（桁が伸びた別の値）にも
    /// 当たってしまう。**数字の終わりまで読んで、値として比べる。**
    fn logged_silent_secs(line: &str) -> Option<u64> {
        let digits: String = line
            .split_once("silent_secs=")?
            .1
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    }

    /// 沈黙を報告した行に載った秒数を、**出た順に**並べる。
    fn reported_silences(output: &str) -> Vec<u64> {
        report_lines(output, IDLE_REPORT_MARKER)
            .into_iter()
            .map(|line| {
                logged_silent_secs(line)
                    .unwrap_or_else(|| panic!("沈黙の秒数が載っていること: {line:?}"))
            })
            .collect()
    }

    /// **上流が黙っても打ち切らない。報告して待ち続ける**（ADR-0035 決定1・決定2 / B-6）。
    ///
    /// # これは [ADR-0019](../../../docs/adr/0019-relay-idle-watchdog.md) §3 の撤回そのもの
    ///
    /// 以前はここで error オブジェクトを1件流して終端し、**その回は利用者の発言も含めて
    /// 1行も保存しなかった。** いま起きるのは `warn` が1本出ることだけで、
    /// **沈黙の後に届いた本文もそのまま流れるし、完結すれば保存される。**
    ///
    /// **黙っている間も上流を手放さない**ことを [`Relayed::dropped_by_event`] で見る。
    /// 打ち切りへ戻す変異は、**沈黙の後の本文が下流に出ない**ので件数で赤くなり、
    /// **保存が消える**ので保存でも赤くなる。
    ///
    /// # 答え合わせと締めの記録も、この枝で見る
    ///
    /// 沈黙を挟んだ回こそ**いちばん知りたい**ので、
    /// 答え合わせ（ADR-0013 第3層）が走ることと、
    /// 締めが**完了**として記録されること（壊れた枝の文言にならないこと）を併せて固定する。
    #[test]
    fn a_silent_upstream_is_reported_and_the_relay_keeps_going() {
        assert!(
            STALL > IDLE_BUDGET,
            "前提: 報告の間隔より長く黙ること（{STALL:?} / {IDLE_BUDGET:?}）"
        );
        assert!(
            WITHIN < IDLE_BUDGET,
            "前提: 前半の間隔は報告の間隔の内側であること（{WITHIN:?} / {IDLE_BUDGET:?}）"
        );
        // **前提: 手掛かりが締めのどれにも当たらないこと。**
        // 当たると「報告が出た」と「締めが出た」を区別できなくなる（B-2 で実際に踏んだ）。
        for finish in FINISH_MESSAGES {
            assert!(
                !finish.contains(IDLE_REPORT_MARKER),
                "前提: 報告の手掛かりが締めの文言（{finish}）にも当たると、行を取り違える"
            );
        }

        // **プロンプト組立は観測窓の外で済ませる**（`prompt` 層のログと混ぜない）。
        let plan = plan();
        let mut outcome = None;
        let output = captured_under_default_filter(|| {
            outcome = Some(relay_on_a_virtual_clock(
                vec![
                    Step::Chunk(Ok(ChatChunk {
                        delta: "「おかえり".to_owned(),
                        finish_reason: None,
                        // 見積もりを必ず超える値（上界の破れ）。
                        usage: Some(TokenUsage {
                            prompt_tokens: u32::MAX,
                            completion_tokens: None,
                        }),
                    })),
                    Step::Silence(WITHIN),
                    Step::Chunk(Ok(chunk("なさい」", None))),
                    // ここで上流が黙り込む。**以前はここで打ち切られていた。**
                    Step::Silence(STALL),
                    Step::Chunk(Ok(chunk(" 彼女は微笑んだ。", Some(FinishReason::Stop)))),
                ],
                plan,
            ));
        });
        let relayed = outcome.expect("中継が走ること");

        assert_eq!(
            relayed.events.len(),
            4,
            "本文3件 ＋ [DONE]（沈黙の後の1件も流すこと）: {:?}",
            relayed.events
        );
        assert_eq!(delta_of(&relayed.events[0]), "「おかえり");
        assert_eq!(delta_of(&relayed.events[1]), "なさい」");
        assert_eq!(
            delta_of(&relayed.events[2]),
            " 彼女は微笑んだ。",
            "沈黙の後に届いた本文を捨てないこと: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.events.last().map(String::as_str),
            Some(DONE_DATA),
            "黙っただけの回を完結として終端すること: {:?}",
            relayed.events
        );

        // **黙っている間も上流を保持し続ける。**
        // ここが立つのは `[DONE]` を流した最後の1件だけ。
        assert!(
            !relayed.dropped_by_event[..3].iter().any(|dropped| *dropped),
            "沈黙をまたいでも上流を落とさないこと: {:?}",
            relayed.dropped_by_event
        );

        assert_eq!(
            relayed.saved.len(),
            2,
            "黙った回でも、完結すれば保存すること: {:?}",
            relayed.saved
        );
        assert_eq!(
            relayed.saved[1].raw_content, "「おかえりなさい」 彼女は微笑んだ。",
            "沈黙の前後を1つの応答として残すこと"
        );

        // **秒数は「黙っている」と告げる行の中で読む。**
        assert_eq!(
            reported_silences(&output),
            vec![STALL.as_secs()],
            "黙った長さだけ、実際の経過を1本報告すること: {output:?}"
        );

        // **申告値は答え合わせの行の中で見る**（作業単位 B-2）。
        // **囮は `usage::log_check` の超過側の `warn`**——`prompt_tokens = u32::MAX` は
        // 上界も超過も同時に破るので、**超過側の行にも同じ値が載る。**
        let crosschecked = line_containing(&output, UPPER_BOUND_MARKER);
        assert!(
            has_field(crosschecked, "actual_tokens=4294967295"),
            "黙った回でも答え合わせすること: {crosschecked:?}"
        );
        assert!(
            output.contains(FINISH_MESSAGES[0]),
            "黙っただけの回を「完了した」と記録すること: {output:?}"
        );
        assert!(
            !output.contains(FINISH_MESSAGES[1]),
            "黙っただけの回を「上流が壊れた」と記録しないこと（枝を取り違えない）: {output:?}"
        );
    }

    /// **黙り続けている間、報告は繰り返し出る。載るのは実際の経過**（B-6 / a）。
    ///
    /// # なぜ「1本出れば十分」ではないのか
    ///
    /// 打ち切らなくなった以上、**沈黙はいくらでも伸びる。**
    /// 1本しか出さないと、ログを見た人は「30秒黙った」までしか分からず、
    /// **それが今も続いているのか、直後に再開したのか**が読めない。
    ///
    /// # この1本が塞いでいる変異
    ///
    /// * **欄を定数（[`IDLE_BUDGET`] の秒数）に固定する。**
    ///   [`STALL`] は間隔の 20 倍なので、固定した値とは一致しない。
    /// * **最初の1本しか出さない**（歯止めを噛ませる形）。2本目が消える。
    /// * **経過の起点を取り違える**（毎回いまから測り直す）。
    ///   2本目が `600` になり、`1200` にならない。
    #[test]
    fn a_lasting_silence_is_reported_again_and_again() {
        assert!(
            STALL.as_secs() != IDLE_BUDGET.as_secs(),
            "前提: 沈黙の長さは報告の間隔と別の値であること（同値だと定数への固定を検出できない）"
        );

        let plan = plan();
        let mut outcome = None;
        let output = captured_under_default_filter(|| {
            outcome = Some(relay_on_a_virtual_clock(
                vec![
                    Step::Chunk(Ok(chunk("「おかえり", None))),
                    // **黙りを2回に割る。** 台本の1手ごとに仮想時計が飛ぶので、
                    // ここで報告の機会も2回来る。
                    Step::Silence(STALL),
                    Step::Silence(STALL),
                    Step::Chunk(Ok(chunk("なさい」", Some(FinishReason::Stop)))),
                ],
                plan,
            ));
        });
        let relayed = outcome.expect("中継が走ること");

        assert_eq!(
            reported_silences(&output),
            vec![STALL.as_secs(), STALL.as_secs() * 2],
            "黙り続けている間、**同じ沈黙の通算の長さ**を繰り返し報告すること: {output:?}"
        );

        assert_eq!(
            relayed.events.last().map(String::as_str),
            Some(DONE_DATA),
            "何度報告しても打ち切らないこと: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.saved.len(),
            2,
            "完結した回は保存すること: {:?}",
            relayed.saved
        );
    }

    /// **上流が壊れた枝は、その場で上流を落とす**（作業単位 11c-3b / B-6 で失敗源を移した）。
    ///
    /// 終わりを決めた実装は `state.upstream = None` の1行だけで、**その1行が上流の
    /// コネクションを閉じる**（`tests/llm_lmstudio.rs` の
    /// `dropping_the_stream_closes_the_upstream_connection` が実測で固定している）。
    ///
    /// **落とさない実装へ変えても、下流に見える SSE は1バイトも変わらない。**
    /// 例えば「終わったことにする印」を別に持ち、ストリーム自体は応答が
    /// 落ちるまで抱えておく形——イベント列も `[DONE]` の有無も保存の有無も同じで、
    /// **違うのは「終わった後も上流が生成し続ける」ことだけ。**
    ///
    /// # ⚠ 失敗源が変わった（B-6）
    ///
    /// **以前ここを起こしていたのは中継の打ち切り**（30秒の無通信）だった。
    /// 打ち切りは撤回された（ADR-0035）ので、**いま同じ枝へ入るのは
    /// 上流が壊れた回と、`llm` 層の天井に当たった回**（`LlmError::Timeout` として届く）。
    /// **見ている1行は同じ。**
    ///
    /// **見るのは「error オブジェクトが下流へ出た時点」。** 中継が終わってから見ると
    /// 応答の本文ごと drop されるので、**実装が何をしていても立つ**
    /// （[`Relayed::dropped_by_event`] の doc）。
    #[test]
    fn a_failed_relay_drops_the_upstream_before_the_error_reaches_the_client() {
        let relayed = relay_on_a_virtual_clock(
            vec![
                Step::Chunk(Ok(chunk("「おかえり", None))),
                // 上流が壊れた（天井に当たった回もここへ来る）。
                Step::Chunk(Err(LlmError::Truncated)),
                // **終端するので、この先は下流へ出ない。**
                Step::Chunk(Ok(chunk("なさい」", Some(FinishReason::Stop)))),
            ],
            plan(),
        );

        assert_eq!(
            relayed.events.len(),
            2,
            "前提: 本文1件 ＋ error オブジェクト1件（終端したこと）: {:?}",
            relayed.events
        );
        assert!(
            !relayed.dropped_at_the_first_event(),
            "前提: 中継の最中は上流を保持していること（印が最初から立っていないこと）"
        );

        assert!(
            relayed.dropped_at_the_last_event(),
            "終端と同時に上流を落とすこと（下流へ error を出した時点で）"
        );
    }

    /// **最初の1件が遅れても打ち切らない。ただし黙っていることは告げる**
    /// （ADR-0019 §2 の免除は B-6 で「欄」に変わった）。
    ///
    /// # 免除をやめた理由
    ///
    /// §2 の免除は**「打ち切ってしまうから」**だった。JIT ロード中は最初のトークンまで
    /// 数十秒かかるので、そこを切るとモデルが載る前に毎回失敗する。
    /// **打ち切らなくなった以上、その理由は消えた。**
    ///
    /// **代わりに欄になった** —— `after_first_token` が `false` なら
    /// 「まだ1トークンも来ていない」（JIT ロード中かもしれない）、
    /// `true` なら「このターンの上流が1件は返した後の沈黙」（喋り出してから止まった回のほか、
    /// 空の本文を引き直す2回目を待つ回も含む / `RelayState::received_first` の doc / A-18b）。
    /// **読み手にとって意味がまるで違う**ので、報告からは落とさない。
    ///
    /// **打ち切りの天井は引き続き `llm` 層**（`DEFAULT_IDLE_TIMEOUT`）が持つ。
    #[test]
    fn a_slow_first_chunk_is_reported_but_never_cut_off() {
        assert!(
            JIT_WAIT > IDLE_BUDGET,
            "前提: 最初の1件は報告の間隔を超えて待たされること（{JIT_WAIT:?} / {IDLE_BUDGET:?}）"
        );

        let plan = plan();
        let mut outcome = None;
        let output = captured_under_default_filter(|| {
            outcome = Some(relay_on_a_virtual_clock(
                vec![
                    // モデルが載るまでの空白。**報告の間隔より長い。**
                    Step::Silence(JIT_WAIT),
                    Step::Chunk(Ok(chunk("「おかえり", None))),
                    Step::Silence(WITHIN),
                    Step::Chunk(Ok(chunk("なさい」", Some(FinishReason::Stop)))),
                ],
                plan,
            ));
        });
        let relayed = outcome.expect("中継が走ること");

        assert_eq!(
            relayed.events.last().map(String::as_str),
            Some(DONE_DATA),
            "最初の1件が遅れただけで打ち切らないこと: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.events.len(),
            3,
            "本文2件 ＋ [DONE]: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.saved.len(),
            2,
            "完結した回は保存すること: {:?}",
            relayed.saved
        );
        assert_eq!(relayed.saved[1].raw_content, "「おかえりなさい」");

        assert_eq!(
            reported_silences(&output),
            vec![JIT_WAIT.as_secs()],
            "最初の1件を待っている間も、黙っていることは告げること: {output:?}"
        );
        let report = report_lines(&output, IDLE_REPORT_MARKER)[0];
        assert!(
            has_field(report, "after_first_token=false"),
            "まだ1トークンも来ていないことを、その行で見分けられること: {report:?}"
        );
    }

    /// **「1件来た」は「下流へ1件出た」ではない**（使用量だけのチャンクも数える）。
    ///
    /// 使用量だけのチャンクは下流へ出ないが、**上流が喋っている証拠ではある。**
    /// これを「まだ1件も来ていない」と数えると、報告の欄が
    /// **「JIT ロード中かもしれない」と言い続ける**——
    /// 実際には喋り出した後で止まっているのに、読み手はそう読めない。
    ///
    /// （かつては免除の境目だったので、ずれると停止を丸ごと見逃していた。
    /// **B-6 で免除そのものが無くなり、残ったのは欄の意味だけ。**）
    #[test]
    fn a_usage_only_chunk_counts_as_the_first_token_in_the_report() {
        let plan = plan();
        let mut outcome = None;
        let output = captured_under_default_filter(|| {
            outcome = Some(relay_on_a_virtual_clock(
                vec![
                    // 上流が最初に寄越したのが使用量だけのチャンクだった場合。
                    Step::Chunk(Ok(usage_only(321))),
                    Step::Silence(STALL),
                    Step::Chunk(Ok(chunk("「はい」", Some(FinishReason::Stop)))),
                ],
                plan,
            ));
        });
        let relayed = outcome.expect("中継が走ること");

        assert_eq!(
            relayed.events.len(),
            2,
            "本文1件 ＋ [DONE]（使用量だけのチャンクは流さない）: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.saved.len(),
            2,
            "完結した回は保存すること: {:?}",
            relayed.saved
        );

        let reports = report_lines(&output, IDLE_REPORT_MARKER);
        assert_eq!(reports.len(), 1, "黙った長さだけ報告すること: {output:?}");
        assert!(
            has_field(reports[0], "after_first_token=true"),
            "下流へ出ない1件も「上流が喋った」に数えること: {:?}",
            reports[0]
        );
    }

    /// **報告の間隔の内側で届き続ける限り、1本も報告しない。**
    ///
    /// 見張るのは**1件ごとの間隔**であって、応答全体の長さではない。
    /// 累積で判定する実装（開始時に1本だけ期限を切る形）はここで落ちる。
    ///
    /// # 第2相が、この否定を縛っている
    ///
    /// 「報告が出なかった」だけなら、**捕捉が壊れていても緑**になる
    /// （`crate::testlog` のモジュール doc「危険なのは偽陽性ではなく偽陰性」）。
    /// **同じ捕捉・同じ手掛かりで、報告が現に出る台本**も回す。
    #[test]
    fn chunks_that_keep_arriving_within_the_interval_are_never_reported() {
        assert!(
            NEARLY < IDLE_BUDGET && WITHIN < IDLE_BUDGET,
            "前提: 1件ごとの間隔は報告の間隔の内側であること"
        );
        assert!(
            WITHIN + NEARLY > IDLE_BUDGET,
            "前提: 合計は間隔を超えること（累積で判定する実装と区別するため）"
        );

        let quiet_plan = plan();
        let mut outcome = None;
        // 第1相: **間隔の内側で届き続ける。**
        let quiet = captured_under_default_filter(|| {
            outcome = Some(relay_on_a_virtual_clock(
                vec![
                    Step::Chunk(Ok(chunk("「おかえり", None))),
                    Step::Silence(WITHIN),
                    Step::Chunk(Ok(chunk("なさい」", None))),
                    Step::Silence(NEARLY),
                    Step::Chunk(Ok(chunk(" 彼女は微笑んだ。", Some(FinishReason::Stop)))),
                ],
                quiet_plan,
            ));
        });
        let relayed = outcome.expect("中継が走ること");

        assert_eq!(
            relayed.events.last().map(String::as_str),
            Some(DONE_DATA),
            "間隔の内側で届き続けたら完走すること: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.events.len(),
            4,
            "本文3件 ＋ [DONE]: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.saved.len(),
            2,
            "完結した回は保存すること: {:?}",
            relayed.saved
        );
        assert_eq!(
            relayed.saved[1].raw_content, "「おかえりなさい」 彼女は微笑んだ。",
            "流した増分の連結を丸ごと残すこと"
        );
        assert!(
            report_lines(&quiet, IDLE_REPORT_MARKER).is_empty(),
            "間隔の内側で届いている間は1本も報告しないこと: {quiet:?}"
        );

        // 第2相（対照）: **同じ捕捉・同じ手掛かりで、報告が現に出る。**
        let loud_plan = plan();
        let loud = captured_under_default_filter(|| {
            relay_on_a_virtual_clock(
                vec![
                    Step::Chunk(Ok(chunk("「おかえり", None))),
                    Step::Silence(STALL),
                    Step::Chunk(Ok(chunk("なさい」", Some(FinishReason::Stop)))),
                ],
                loud_plan,
            );
        });
        assert!(
            !report_lines(&loud, IDLE_REPORT_MARKER).is_empty(),
            "第2相で報告を観測できなかった。**捕捉か手掛かりが壊れている**ので、\
             第1相の「1本も無い」は「報告しない」ではなく「何も見ていない」を意味する: {loud:?}"
        );
    }

    /// **使用量だけのチャンクを挟んでも、間隔は1件ごとに掛かり直す。**
    ///
    /// 使用量だけのチャンクは下流へ出さずに `continue` する（`Relay::relay` が `None`）。
    /// ここで期限を持ち越すと、**下流から見えない1件のせいで報告が出る**
    /// ——「黙っている」と言いながら、実は上流は喋っている。
    #[test]
    fn a_usage_only_chunk_restarts_the_interval() {
        assert!(
            WITHIN + NEARLY > IDLE_BUDGET,
            "前提: 使用量チャンクを挟んだ前後の合計は間隔を超えること"
        );

        let plan = plan();
        let mut outcome = None;
        let output = captured_under_default_filter(|| {
            outcome = Some(relay_on_a_virtual_clock(
                vec![
                    Step::Chunk(Ok(chunk("「はい」", Some(FinishReason::Stop)))),
                    Step::Silence(WITHIN),
                    // **下流へは出ない1件。**
                    Step::Chunk(Ok(usage_only(321))),
                    Step::Silence(NEARLY),
                    Step::Chunk(Ok(chunk(" 彼女は頷いた。", None))),
                ],
                plan,
            ));
        });
        let relayed = outcome.expect("中継が走ること");

        assert_eq!(
            relayed.events.last().map(String::as_str),
            Some(DONE_DATA),
            "使用量だけのチャンクで期限を食い潰さないこと: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.events.len(),
            3,
            "本文2件 ＋ [DONE]（使用量だけのチャンクは流さない）: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.saved.len(),
            2,
            "完結した回は保存すること: {:?}",
            relayed.saved
        );
        assert!(
            report_lines(&output, IDLE_REPORT_MARKER).is_empty(),
            "下流へ出ない1件を「黙っていた」に数えないこと: {output:?}"
        );
    }

    /// **使用量だけのチャンクの直後に黙り込んでも報告する。**
    ///
    /// `continue` する枝は上流を読み直すので、**そこで見張りが掛からないと
    /// 中継はその場で回り続ける**（下流は何も受け取らないまま、誰も何も言わない）。
    #[test]
    fn a_silence_after_a_usage_only_chunk_is_still_reported() {
        let plan = plan();
        let mut outcome = None;
        let output = captured_under_default_filter(|| {
            outcome = Some(relay_on_a_virtual_clock(
                vec![
                    Step::Chunk(Ok(chunk("「はい」", None))),
                    Step::Silence(WITHIN),
                    Step::Chunk(Ok(usage_only(321))),
                    // 使用量だけのチャンクを受けた直後に黙り込む。
                    Step::Silence(STALL),
                    Step::Chunk(Ok(chunk("……", Some(FinishReason::Stop)))),
                ],
                plan,
            ));
        });
        let relayed = outcome.expect("中継が走ること");

        assert_eq!(
            relayed.events.len(),
            3,
            "本文2件 ＋ [DONE]: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.events.last().map(String::as_str),
            Some(DONE_DATA),
            "黙り込んでも打ち切らないこと: {:?}",
            relayed.events
        );
        assert_eq!(
            reported_silences(&output),
            vec![STALL.as_secs()],
            "使用量だけのチャンクの後の沈黙も報告すること: {output:?}"
        );
    }

    /// **下流へ出す案内に、見張りの事情を載せない**（CLAUDE.md 絶対制約）。
    ///
    /// 何秒黙っていたのか、上流のどこで止まったのかは**応答に載せない。**
    /// 数字が1つでも載ると、そこから内部の設定値が読める。
    ///
    /// # ⚠ 見ている案内が変わった（B-6）
    ///
    /// **以前は「無通信で打ち切りました」の案内**を見ていた。その案内は消えたので、
    /// **黙った後で上流が壊れた回の案内**を見る——
    /// 見張りが `warn` に出した秒数が、こちら側へ漏れていないこと。
    #[test]
    fn the_failure_notice_never_leaks_the_watchdog() {
        let relayed = relay_on_a_virtual_clock(
            vec![
                Step::Chunk(Ok(chunk("「はい", None))),
                // **報告が1本出る長さ。** その秒数が下流へ漏れないことを見る。
                Step::Silence(STALL),
                Step::Chunk(Err(LlmError::Truncated)),
            ],
            plan(),
        );

        let notice = relayed
            .events
            .last()
            .expect("error オブジェクトが流れていること")
            .clone();

        // 前提: 失敗の案内そのものを見ていること。
        let value: Value = serde_json::from_str(&notice).expect("JSON として読めること");
        assert_eq!(
            value,
            json!({
                "error": {
                    "message": "推論サーバとの通信に失敗しました",
                    "type": "server_error",
                    "code": "upstream_failed",
                }
            }),
            "既存のエラー形で伝えること（新しい形を作らない）"
        );

        assert!(
            !notice.chars().any(|c| c.is_ascii_digit()),
            "秒数を載せないこと（数字を1つも含まない）: {notice}"
        );
        // **`upstream_failed` は非ストリーミングと共通の `code` なので載ってよい**
        // （経路を明かす内部の事情ではない）。載せてはいけないのは、
        // どれだけ黙っていたか・上流のどこで止まったかという中の話。
        for leaked in [
            "30",
            "600",
            "秒",
            "sec",
            "idle",
            "silent",
            "無通信",
            "next()",
        ] {
            assert!(
                !notice.contains(leaked),
                "内部の事情を載せないこと（{leaked}）: {notice}"
            );
        }
    }

    /// **報告の間隔は 30 秒ちょうど。** 秒未満まで固定する（ADR-0035 §決定2 の「既定 30 秒」）。
    ///
    /// 挙動側の境界は [`NEARLY`]（29秒）と [`STALL`]（600秒）の間にしか無く、
    /// `Duration::from_millis(30_900)` のような値では挙動が変わらない
    /// （**穴でも死んだコードでもなく粒度の限界**）。ここで直に押さえる。
    ///
    /// 期待値は**実装から導かない**（[`IDLE_BUDGET`] を書き換える変異と一緒に動く）。
    #[test]
    fn the_idle_report_interval_is_exactly_thirty_seconds() {
        assert_eq!(IDLE_BUDGET, Duration::from_secs(30));
    }

    /// **`[DONE]` の直前に入る保存の空白は、見張りの外にある**（ADR-0019 §3 の宿題）。
    ///
    /// 保存は `events.next()` の**外**で `await` するので、原理的に見張りに掛からない。
    /// **「掛からないはず」で通さず、実際にとても遅い保存を作って測る。**
    ///
    /// # ⚠ **B-6 で主張が2つに割れた**（レビュー指摘 R-2）
    ///
    /// 打ち切りがあった頃は、**保存の有無だけで両方言えていた**
    /// ——見張りが `persist` に掛かれば保存が消えるので `saved.len()` で捕まる。
    ///
    /// **いまは掛かっても「報告が出るだけ」の場合がある**ので、割れている:
    ///
    /// | 主張 | 見るもの |
    /// |---|---|
    /// | 保存が消えない | `saved.len() == 2`（[`PERSIST_TARGET`] の doc の M2 はここで赤くなる） |
    /// | **報告が出ない** | **通り過ぎたイベントの文言**（[`StallPersist`] が控える） |
    ///
    /// **doc（`chunk_events` の「どれだけ遅くても報告されない」）が
    /// 名指ししているのは後者**なので、そちらを実際に見る。
    /// **控えが空でないことを前提で確かめる**——空なら「報告が無い」ではなく
    /// 「何も見ていない」である（`crate::testlog` の「偽陰性」）。
    ///
    /// 空白は[`保存に入った瞬間`][jump_the_virtual_clock]に**こちらから**
    /// [`PERSIST_STALL`] だけ飛ばして作る。**自動前進に任せない**——任せると
    /// 進む量が制御できず（実測 30秒ちょうど〜36,000秒）、負荷が掛かったときに
    /// 間隔ぴったりへ張り付いて前提が偽になる。しかも DB を待つ間に自動前進が
    /// 起きると `PoolTimedOut` で保存ごと落ちる。
    #[test]
    fn a_very_slow_persist_is_outside_the_watchdog_budget() {
        assert!(
            PERSIST_STALL > IDLE_BUDGET,
            "前提: 空白は報告の間隔より長いこと（{PERSIST_STALL:?} / {IDLE_BUDGET:?}）"
        );

        let (relayed, seen) = relay_with_a_stalled_persist(
            vec![
                Step::Chunk(Ok(chunk("「おかえり", None))),
                Step::Chunk(Ok(chunk("なさい」", Some(FinishReason::Stop)))),
            ],
            plan(),
            PERSIST_STALL,
        );

        // 前提: 保存の待ちの間に、仮想時計が**報告の間隔をはるかに超えて**進んだこと。
        // ここが偽なら以下の検査は何も測っていない。
        assert!(
            relayed.elapsed >= PERSIST_STALL,
            "前提: 保存の間に空けた空白がそのまま中継の長さに乗ること（{:?} / {PERSIST_STALL:?}）",
            relayed.elapsed
        );
        // 前提: 文言を現に控えていること（空なら「報告が無い」を主張できない）。
        assert!(
            seen.iter().any(|line| line.contains(PARSE_MARKER)),
            "前提: 保存の記録を控えていること（控えが働いていないと下の否定は無意味）: {seen:?}"
        );

        assert_eq!(
            relayed.events.last().map(String::as_str),
            Some(DONE_DATA),
            "遅い保存で終端を壊さないこと: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.saved.len(),
            2,
            "遅くても保存し切ること: {:?}",
            relayed.saved
        );
        assert!(
            !seen.iter().any(|line| line.contains(IDLE_REPORT_MARKER)),
            "遅い保存を「上流が黙っている」と報告しないこと: {seen:?}"
        );
    }

    // ------------------------------------------------------------------
    // **空の本文を1回だけ引き直す**（ADR-0059 §決定3 / 作業単位 A-18b）
    //
    // 本番の入口からの見張り番は `tests/server_empty_retry.rs`。ここは中継の中の形
    // （保留した差分・終わりの印の数・沈黙の報告）を仮想時計の上で見る。
    // ------------------------------------------------------------------

    /// **引き直しを開けない相手**（`stream` が必ず失敗する）。引き直しを見ないテストの既定。
    ///
    /// 空の回が来ても「引き直しが無かったときと同じ結果」になる（ADR-0059 §決定3 の2回目の失敗）。
    fn unreachable_redo() -> Redo {
        Redo::new(Again::streaming(Vec::new()), ChatRequest::new(Vec::new()))
    }

    /// 引き直しの相手と、それに渡す要求。**要求は空でない値**（同じ要求が届いたかを見るため）。
    fn redo_to(again: &Arc<Again>) -> Redo {
        Redo::new(Arc::clone(again) as Arc<dyn LlmClient>, redo_request())
    }

    fn redo_request() -> ChatRequest {
        let mut request = ChatRequest::new(Vec::new());
        request.max_tokens = Some(777);
        request
    }

    /// 使用量だけのチャンク（プロンプトの数と生成数）。
    fn usage_chunk(prompt_tokens: u32, completion_tokens: u32) -> ChatChunk {
        ChatChunk {
            delta: String::new(),
            finish_reason: None,
            usage: Some(TokenUsage {
                prompt_tokens,
                completion_tokens: Some(completion_tokens),
            }),
        }
    }

    /// 2回目の本文（2つの差分に割って流す）。
    const SECOND: [&str; 2] = ["「おかえり", "なさい」"];

    /// 2回目の上流の台本（本文 → 終わり → 使用量）。
    fn second_stream() -> Vec<Result<ChatChunk, LlmError>> {
        vec![
            Ok(chunk("", None)),
            Ok(chunk(SECOND[0], None)),
            Ok(chunk(SECOND[1], None)),
            Ok(chunk("", Some(FinishReason::Stop))),
            Ok(usage_chunk(803, 9)),
        ]
    }

    /// 下流に見えたもの（増分の連結・役割の数・`finish_reason` の位置・`[DONE]` の位置）。
    struct Downstream {
        content: String,
        roles: usize,
        finishes: Vec<usize>,
        dones: Vec<usize>,
        len: usize,
    }

    fn downstream(events: &[String]) -> Downstream {
        let mut seen = Downstream {
            content: String::new(),
            roles: 0,
            finishes: Vec::new(),
            dones: Vec::new(),
            len: events.len(),
        };
        for (index, data) in events.iter().enumerate() {
            if data == DONE_DATA {
                seen.dones.push(index);
                continue;
            }
            let value: Value = serde_json::from_str(data).expect("JSON として読めること");
            let choice = &value["choices"][0];
            seen.content
                .push_str(choice["delta"]["content"].as_str().unwrap_or_default());
            if choice["delta"].get("role").is_some() {
                seen.roles += 1;
            }
            if !choice["finish_reason"].is_null() {
                seen.finishes.push(index);
            }
        }
        seen
    }

    impl Downstream {
        /// **`finish_reason` と `[DONE]` が最後に1回ずつ**（`finish_reason` の直後に `[DONE]` で終わり）。役割も1回。
        fn finished_once_at_the_end(&self) -> bool {
            self.dones == [self.len - 1] && self.finishes == [self.len - 2] && self.roles == 1
        }
    }

    /// **本文が空のまま終わった中継は、同じ要求で1回だけ引き直し、2回目を同じ応答の続きとして流す。**
    ///
    /// 1回目は思考を別欄で返した形（役割だけの差分 → 空の差分で `stop`、使用量は終わりのチャンク）。
    /// 下流には `finish_reason` と `[DONE]` が最後に1回ずつ、役割は1回。保存は2回目、清算は1回で2回目の値、
    /// 捨てた1回目の生成数は分解の行に出る。
    #[test]
    fn a_blank_stream_is_retried_once_as_the_continuation_of_the_same_response() {
        let again = Again::streaming(vec![Ok(second_stream())]);
        let mut relayed = None;
        let output = captured_under_default_filter(|| {
            relayed = Some(run_relay_with(
                vec![
                    Step::Chunk(Ok(chunk("", None))),
                    Step::Chunk(Ok(ChatChunk {
                        delta: String::new(),
                        finish_reason: Some(FinishReason::Stop),
                        usage: Some(TokenUsage {
                            prompt_tokens: 701,
                            completion_tokens: Some(41),
                        }),
                    })),
                ],
                plan(),
                None,
                redo_to(&again),
            ));
        });
        let relayed = relayed.expect("走ったこと");

        assert_eq!(again.opens(), 1, "1回だけ引き直すこと");
        assert_eq!(again.requests(), vec![redo_request()], "同じ要求を送ること");
        let seen = downstream(&relayed.events);
        assert!(seen.finished_once_at_the_end(), "{:?}", relayed.events);
        assert_eq!(seen.content, SECOND.concat());
        assert_eq!(relayed.saved.len(), 2, "{:?}", relayed.saved);
        assert_eq!(relayed.saved[1].raw_content, SECOND.concat(), "保存は2回目");

        let seals = report_lines(&output, "封を観測しました");
        assert_eq!(seals.len(), 1, "清算は1回だけ: {seals:?}");
        assert!(
            has_field(seals[0], "usage_prompt_tokens=Some(803)"),
            "2回目の値: {}",
            seals[0]
        );
        let line = line_containing(&output, PARSE_MARKER);
        assert!(has_field(line, "empty_retries=1"), "{line:?}");
        assert!(
            has_field(line, "discarded_completion_tokens=Some(41)"),
            "{line:?}"
        );
    }

    /// **2回目が `usage` を返さなければ、清算は「答え合わせできない」**（1回目の `usage` を持ち越さない）。
    ///
    /// 2回目が `usage` を返す材料だけだと、1回目の `usage` を捨てない実装（`Relay::restart` で `usage` を残す）は
    /// 2回目の値に上書きされて緑になる（A-18b の変異 M13 で実測）。
    #[test]
    fn the_first_usage_is_not_carried_into_a_retry_without_usage() {
        let again = Again::streaming(vec![Ok(vec![
            Ok(chunk(SECOND[0], None)),
            Ok(chunk(SECOND[1], Some(FinishReason::Stop))),
        ])]);
        let mut relayed = None;
        let output = captured_under_default_filter(|| {
            relayed = Some(run_relay_with(
                vec![Step::Chunk(Ok(ChatChunk {
                    delta: String::new(),
                    finish_reason: Some(FinishReason::Stop),
                    usage: Some(TokenUsage {
                        prompt_tokens: 701,
                        completion_tokens: Some(41),
                    }),
                }))],
                plan(),
                None,
                redo_to(&again),
            ));
        });
        let relayed = relayed.expect("走ったこと");

        assert_eq!(again.opens(), 1, "前提: 引き直したこと");
        assert_eq!(relayed.saved[1].raw_content, SECOND.concat());
        let seals = report_lines(&output, "封を観測しました");
        assert_eq!(seals.len(), 1, "清算は1回だけ: {seals:?}");
        assert!(
            has_field(seals[0], "usage_prompt_tokens=None"),
            "1回目の値を2回目の清算に使わないこと: {}",
            seals[0]
        );
        // 捨てた1回目の生成数は、分解の行には残る。
        let line = line_containing(&output, PARSE_MARKER);
        assert!(
            has_field(line, "discarded_completion_tokens=Some(41)"),
            "{line:?}"
        );
    }

    /// **保留したチャンクの差分は捨てない**（閉じタグでありうる / モジュールの doc）。
    ///
    /// 1回目は本文の中で閉じた思考だけで、閉じタグが `finish_reason` のチャンクに載って届く形。
    /// 捨てると下流は「閉じない思考＋2回目の本文」を受け取り、使い手の剥がしが2回目まで思考として捨てる。
    ///
    /// **捨てた回の文字数は、保留した差分（閉じタグ）込み**（レビューの T2）——記録を、保留した差分を蓄積する前に
    /// 取ると、閉じタグの分だけ短く出る。
    #[test]
    fn the_held_delta_is_still_relayed_before_the_retry() {
        /// 1回目の本文（閉じタグの前まで）と、終わりのチャンクに載る閉じタグ。
        const THOUGHT: &str = "<think>\nThe user is home.\n";
        const CLOSE: &str = "</think>";

        let again = Again::streaming(vec![Ok(second_stream())]);
        let mut relayed = None;
        let output = captured_under_default_filter(|| {
            relayed = Some(run_relay_with(
                vec![
                    Step::Chunk(Ok(chunk(THOUGHT, None))),
                    Step::Chunk(Ok(chunk(CLOSE, Some(FinishReason::Stop)))),
                ],
                plan(),
                None,
                redo_to(&again),
            ));
        });
        let relayed = relayed.expect("走ったこと");

        assert_eq!(again.opens(), 1, "閉じた思考だけの回は引き直すこと");
        let seen = downstream(&relayed.events);
        assert!(seen.finished_once_at_the_end(), "{:?}", relayed.events);
        assert_eq!(
            seen.content,
            format!("{THOUGHT}{CLOSE}{}", SECOND.concat()),
            "閉じタグを落とさずに、2回目を続けること"
        );
        assert_eq!(relayed.saved[1].raw_content, SECOND.concat());
        let line = line_containing(&output, PARSE_MARKER);
        assert!(
            has_field(
                line,
                &format!(
                    "discarded_reply_chars={}",
                    format!("{THOUGHT}{CLOSE}").chars().count()
                )
            ),
            "捨てた回の文字数は閉じタグ込み: {line:?}"
        );
    }

    /// **`finish_reason` 付きのチャンクが2本来ても、先の保留の差分を落とさない**（レビューの C1 / 絶対制約の
    /// `raw_content`）。後の1本は保留に足す（差分を連結し、`finish_reason` は後のもの）。
    ///
    /// 2回目を開けない腕で見る —— 保存されるのが1回目の生の出力なので、差分が欠ければ `raw_content` に出る。
    /// 下流には、連結した差分が後の `finish_reason` 付きで1件だけ流れ、`[DONE]` で終わる。
    #[test]
    fn a_second_finish_chunk_is_added_to_the_held_one_without_losing_its_delta() {
        let steps = vec![
            Step::Chunk(Ok(chunk("<think>a", None))),
            Step::Chunk(Ok(chunk("</think>", Some(FinishReason::Stop)))),
            Step::Chunk(Ok(chunk("\n", Some(FinishReason::Length)))),
        ];
        let again = Again::streaming(vec![Err(())]);
        let relayed = run_relay_with(steps, plan(), None, redo_to(&again));

        assert_eq!(again.opens(), 1, "前提: 引き直しを試みたこと");
        assert_eq!(
            relayed.saved[1].raw_content, "<think>a</think>\n",
            "保留したどの差分も、届いた順のまま蓄積に残ること"
        );
        let seen = downstream(&relayed.events);
        assert_eq!(seen.content, "<think>a</think>\n", "{:?}", relayed.events);
        assert!(seen.finished_once_at_the_end(), "{:?}", relayed.events);
        let last: Value = serde_json::from_str(&relayed.events[relayed.events.len() - 2])
            .expect("JSON として読めること");
        assert_eq!(
            last["choices"][0]["finish_reason"],
            json!("length"),
            "finish_reason は後のもの"
        );
    }

    /// **2回目を開いた後に上流が壊れたら、途中で壊れた中継と同じ**（レビューの T1）。
    ///
    /// error オブジェクトが最後に1件流れ、`finish_reason` も `[DONE]` も流れない。**何も保存しない** ——
    /// 1回目の空も保存しない（2回目の差分を既に流しているので、1回目へは戻れない / モジュールの doc）。
    #[test]
    fn a_retry_that_breaks_midway_saves_nothing_and_ends_with_an_error() {
        let again = Again::streaming(vec![Ok(vec![
            Ok(chunk(SECOND[0], None)),
            Err(LlmError::MissingContent),
        ])]);
        let relayed = run_relay_with(
            vec![Step::Chunk(Ok(chunk("", Some(FinishReason::Stop))))],
            plan(),
            None,
            redo_to(&again),
        );

        assert_eq!(again.opens(), 1, "前提: 2回目を開いたこと");
        let seen = downstream_until_error(&relayed.events);
        assert_eq!(seen.content, SECOND[0], "2回目の差分は流れている");
        assert!(
            seen.finishes.is_empty(),
            "finish_reason は流さない: {:?}",
            relayed.events
        );
        assert!(
            seen.dones.is_empty(),
            "[DONE] は流さない: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.events.len(),
            2,
            "差分1件と error 1件: {:?}",
            relayed.events
        );
        assert_eq!(
            relayed.events.last(),
            Some(&error_data(&LlmError::MissingContent)),
            "最後に error オブジェクトが1件"
        );
        assert!(
            relayed.saved.is_empty(),
            "1回目の空も2回目の途中も保存しないこと: {:?}",
            relayed.saved
        );
    }

    /// [`downstream`] の、最後の error オブジェクトを読まない形。
    fn downstream_until_error(events: &[String]) -> Downstream {
        downstream(events.split_last().map_or(&[][..], |(_, rest)| rest))
    }

    /// **閉じない思考は引き直さない**（続けると2回目まで思考として捨てられる）。**空でない回も引き直さない。**
    #[test]
    fn an_unclosed_thought_or_a_body_is_not_retried_on_the_stream() {
        for first in ["<think>\nThe user is asking", "「ただいま」"] {
            let again = Again::streaming(vec![Ok(second_stream())]);
            let relayed = run_relay_with(
                vec![
                    Step::Chunk(Ok(chunk(first, None))),
                    Step::Chunk(Ok(chunk("", Some(FinishReason::Stop)))),
                ],
                plan(),
                None,
                redo_to(&again),
            );
            assert_eq!(again.opens(), 0, "{first:?}: 引き直さないこと");
            assert!(
                downstream(&relayed.events).finished_once_at_the_end(),
                "{first:?}: {:?}",
                relayed.events
            );
            assert_eq!(relayed.saved[1].raw_content, first);
        }
    }

    /// **2回目も空なら、そのまま終える**（3回目は無い）。保存は2回目の空。
    #[test]
    fn a_second_blank_stream_is_not_retried_again() {
        let again = Again::streaming(vec![
            Ok(vec![Ok(chunk(" \n", Some(FinishReason::Stop)))]),
            Ok(second_stream()),
        ]);
        let relayed = run_relay_with(
            vec![Step::Chunk(Ok(chunk("", Some(FinishReason::Stop))))],
            plan(),
            None,
            redo_to(&again),
        );
        assert_eq!(again.opens(), 1, "3回目を開かないこと");
        assert!(
            downstream(&relayed.events).finished_once_at_the_end(),
            "{:?}",
            relayed.events
        );
        assert_eq!(relayed.saved[1].raw_content, " \n", "2回目の空を保存する");
    }

    /// **2回目が開けなければ、保留したチャンクをそのまま流して `[DONE]`**（引き直しが無かったときと同じ）。
    ///
    /// 下流は、引き直しの相手を持たない形と**同じ並び**になる。保存は1回目、失敗は `warn` に出る。
    #[test]
    fn a_failed_reopen_relays_the_held_chunk_and_done_once() {
        let steps = || {
            vec![
                Step::Chunk(Ok(chunk("<think>a</think>", None))),
                Step::Chunk(Ok(chunk("\n", Some(FinishReason::Stop)))),
            ]
        };
        let again = Again::streaming(vec![Err(())]);
        let mut relayed = None;
        let output = captured_under_default_filter(|| {
            relayed = Some(run_relay_with(steps(), plan(), None, redo_to(&again)));
        });
        let relayed = relayed.expect("走ったこと");

        assert_eq!(again.opens(), 1);
        let seen = downstream(&relayed.events);
        assert!(seen.finished_once_at_the_end(), "{:?}", relayed.events);
        assert_eq!(seen.content, "<think>a</think>\n");
        assert_eq!(relayed.saved[1].raw_content, "<think>a</think>\n");
        let line = line_containing(&output, super::empty_retry::RETRY_FAILED_LINE);
        assert!(line.contains("WARN"), "{line:?}");
        let line = line_containing(&output, PARSE_MARKER);
        assert!(has_field(line, "empty_retry_failed=true"), "{line:?}");

        // **引き直しが無かったときの並び**（受けたチャンクを1件ずつそのまま流して `[DONE]`）と同じ。
        let mut plain = relay();
        let expected: Vec<String> = steps()
            .into_iter()
            .filter_map(|step| match step {
                Step::Chunk(Ok(chunk)) => plain.relay(&chunk),
                Step::Chunk(Err(_)) | Step::Silence(_) => None,
            })
            .chain([DONE_DATA.to_owned()])
            .collect();
        assert_eq!(relayed.events, expected);
    }

    /// 開くまでに `wait` だけ黙る相手（2回目のプロンプトの処理を模す）。**仮想時計を自分で進める。**
    struct SlowToOpen {
        wait: Duration,
        items: Mutex<Option<Vec<Result<ChatChunk, LlmError>>>>,
    }

    impl LlmClient for SlowToOpen {
        fn pinned(&self) -> Option<Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(
            &'a self,
            _request: &'a ChatRequest,
        ) -> crate::llm::LlmFuture<'a, crate::llm::Completion> {
            Box::pin(async { Err(LlmError::MissingContent) })
        }

        fn stream<'a>(
            &'a self,
            _request: &'a ChatRequest,
        ) -> crate::llm::LlmFuture<'a, ChatStream> {
            let items = self
                .items
                .lock()
                .expect("ロック")
                .take()
                .unwrap_or_default();
            let wait = self.wait;
            Box::pin(async move {
                tokio::time::advance(wait).await;
                for _ in 0..YIELDS_AFTER_ADVANCE {
                    tokio::task::yield_now().await;
                }
                Ok(Box::pin(stream::iter(items)) as ChatStream)
            })
        }
    }

    /// **2回目を開くまでの沈黙も報告する。最初のトークンの後の沈黙として**（`RelayState::received_first` の doc）。
    #[test]
    fn the_silence_while_reopening_is_reported_after_the_first_token() {
        let slow = Arc::new(SlowToOpen {
            wait: STALL,
            items: Mutex::new(Some(second_stream())),
        });
        let mut relayed = None;
        let output = captured_under_default_filter(|| {
            relayed = Some(run_relay_with(
                vec![Step::Chunk(Ok(chunk("", Some(FinishReason::Stop))))],
                plan(),
                None,
                Redo::new(slow, redo_request()),
            ));
        });
        let relayed = relayed.expect("走ったこと");

        assert_eq!(downstream(&relayed.events).content, SECOND.concat());
        let lines = report_lines(&output, IDLE_REPORT_MARKER);
        assert!(!lines.is_empty(), "開く間の沈黙を報告すること: {output:?}");
        assert!(
            lines
                .iter()
                .all(|line| has_field(line, "after_first_token=true")),
            "最初のトークンの後の沈黙として言うこと: {lines:?}"
        );
        assert_eq!(
            reported_silences(&output),
            vec![STALL.as_secs()],
            "開く間の沈黙を、通算の秒数で1本言うこと: {lines:?}"
        );
    }
}
