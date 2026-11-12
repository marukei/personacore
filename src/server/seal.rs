//! **封**（ADR-0052 §推奨3〜5 / 作業単位 B-60b・B-60d-1）。
//!
//! # 何をするか
//!
//! 送る直前に、**送るのと同じ本文**を上流に数えさせ（[`LlmClient::count_request`]）、
//! 上流の応答の後に **封の数・実数（`usage.prompt_tokens`）・計画の和・数えられたか・封の往復の時間** を
//! **同じ1行**に出す（[`report`]）。ADR-0052 §測る問い M1（封の数は本番の数え方と等しいか）と
//! M2（封の往復の時間）の材料。
//!
//! # B-60b では観測だけだった。B-60d-1 から封の数で送るかを決める
//!
//! **ここ（このモジュール）は1回の封と「数えられない」の記憶だけを持つ。** 封の数を見て
//! 送る・組み直す・バイト数の計画へ戻るを決めるのは `super::counted_plan`（数えた計画）で、
//! 1ターンの結果は [`TurnSeal`] にまとまって封の行へ出る。
//! B-60b の見張り番 `the_body_sent_does_not_change_whether_or_not_the_upstream_counted`
//! （観測だけ）は、B-60d-1 で `tests/server_seal.rs` の
//! `an_upstream_that_cannot_count_is_sent_exactly_the_byte_plan`（数えられない上流の側だけを約束する）に置き換えた。
//!
//! # 数える写しと送る写しは同じ（§推奨3）
//!
//! 呼び出し側（`server::chat`）は**先に面の世代を、次に写しを**取り
//! （`AppState::main_generation` → `AppState::llm_for_turn`）、
//! **数えるのも送るのもその写しで行う。** この順の理由は `SharedUpstream::generation` の doc。
//!
//! # 「数えられない」を面の世代ごとに覚える（§推奨4）
//!
//! | 封の結果 | 覚えるか | 理由 |
//! |---|---|---|
//! | 数えた（[`Outcome::Counted`]） | 覚えない | 次のターンも数える（覚える必要が無い） |
//! | 数えられない（[`Outcome::Uncountable`]） | **同じ写しが本文を受け付けたときだけ**、その世代の間 | 下の「受け付けた後に覚える」 |
//! | 往復が失敗した（[`Outcome::Failed`]） | **覚えない** | 「今回は聞けなかった」であって「数えられない上流だと分かった」ではない |
//! | 聞かなかった（[`Outcome::Remembered`] / [`Outcome::Latched`]） | 覚えない（もう覚えている） | 掛け金（`Latched`）は下の「第3層の封の腕」 |
//!
//! **⚠ `Err` を覚えない理由。** 往復の失敗は接続・時間切れ・通信の途切れ・**5xx**（読み込み中の 503 等）で、
//! **上流の性質ではない**。覚えると、**混んでいた1回のせいで、その世代の間ずっと数えなくなる**
//! （B-60d-1 から、数えない回はバイト数の計画で送るので、窓が約2割に戻る）。`llm` が `Ok(Uncountable)` と `Err` を分けているのは、
//! この判断をここでさせるため（`LlmClient::count_request` の表）。
//!
//! **⚠ 受け付けた後に覚える。** `Ok(Uncountable)` は 4xx か「2xx だが形が読めない」で、
//! **それだけでは、上流がまともに動いているかは分からない**（本文の往復がその後に失敗しうる）。
//! **だから「数えられない」を覚えるのは、同じ写しが本文を受け付けた後**（[`UncountableMemory::accepted`]）
//! ——本文まで通った上流が「数えられない」と言ったなら、それは上流の性質である。
//! **呼ぶ場所は非ストリーミングと中継の2つ**（`server::chat`。中継は `stream()` が上流を開けた後）。
//! 見張り番は `tests/server_seal.rs` の
//! `an_upstream_that_cannot_count_is_asked_once_per_generation`（受け付けた回は覚える）と
//! `a_count_refused_while_loading_is_not_remembered`（受け付けなかった回は覚えない）。
//! **どちらも1ターン目を非ストリーミングと中継の両方で回す。**
//!
//! # 第3層の封の腕 —— `封 < 実` なら、世代が変わるまでバイト数へ戻す（§推奨5 / 作業単位 B-60d-2）
//!
//! 上流の応答の後、[`Sealed::settle`] が封の行を出し、**送った本文に封をした回**（数えた計画でもバイト数の計画でも）
//! **だけ**、送った本文の封の数と `usage.prompt_tokens` を比べる（[`verdict`]）。
//!
//! | 判定 | 反応 |
//! |---|---|
//! | `封 < 実` | `warn` ＋ **その面の世代に掛け金**（[`UncountableMemory::latch`]）。掛かった世代の間は、部品も封も聞かずにバイト数の計画を送る（`seal=latched`） |
//! | `封 > 実` | `debug` に差を残す（多めに数えている＝無害） |
//! | `封 = 実` | 何もしない（M1 では `parse_special=true` にした後は一致する） |
//!
//! **どの行にも封の数・実数・計画の和を同じ1行に出す**（CLAUDE.md「ログを見る見張り番は同じ行の中で見る」）。
//! 掛け金は「数えられない」と同じ記憶に載る（理由は [`UncountableMemory`] の doc）。
//! 見張り番は `tests/server_seal.rs` の `a_usage_above_the_sealed_count_turns_the_face_back_to_bytes`（N4）・
//! `a_sealed_count_above_the_usage_does_not_turn_the_face_back`・`a_latch_is_released_when_the_generation_changes`。
//! **どれも非ストリーミングと中継の両方で回す**（呼ぶ場所は `server::chat` と `server::relay::finish_relay` の2つ）。
//!
//! 掛け金が外れるのは**版が変わったか、答えた名前が別の名前に変わったとき**だけで、引き直しの失敗（名前が分からない）
//! では外れない（[`Why::still_applies`]）。**遅れて終わった古い世代のターンは、いまの世代の記憶を書き換えない**
//! （[`UncountableMemory`] の doc）。
//!
//! **⚠ 掛け金は一時的な失敗ではない。** `封 < 実` は往復が成功し、上流が両方の数を答えた回にしか出ない
//! （`Failed` を覚えない理由とは逆に、上流の数え方そのものの食い違いである）。
//!
//! # 数える往復の時間（§推奨4 / ADR-0035）
//!
//! **数える往復は本文の応答を遅らせる**（llama では2往復）。上限は `llm` 層の照会の上限
//! （`LmStudioConfig::context_timeout` / **往復ごと**）で、**ここでは新しい定数を足さない。**
//! 上限に当たると `Err` が返り、[`Outcome::Failed`] として**バイト数の計画で送る**
//! （封をせずに送ってよいのはバイト数の上界で示せる計画だけ / ADR-0052 §決定1）。
//! 見張り番は `tests/server_seal.rs` の `a_count_that_failed_is_not_remembered`。
//! **遅れた量は行の `seal_ms` に出る**（[`report`]）。
//!
//! **⚠ 上限を持つのは `llm` の具象の上流だけ**である。テストのモックのように
//! 上限を持たない [`LlmClient`] の実装は、黙った分だけターンを待たせる（本番の経路には無い）。

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::context::FaceGeneration;
use crate::llm::{ChatRequest, LlmClient, TokenCount, TokenUsage};
use crate::prompt::BudgetPlan;

/// 封の結果の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    /// 上流が送る本文を数えた。
    Counted(u32),
    /// **聞いたが、上流は数えられないと答えた**（口が無い＝4xx・形が読めない）。
    Uncountable,
    /// **聞いていない。** この世代は「数えられない」と覚えている。
    Remembered,
    /// **聞いていない。** この世代は、封をした回に `封 < 実` が出て**掛け金が掛かっている**
    /// （ADR-0052 §推奨5 の表の2行目 / 作業単位 B-60d-2 / [`Sealed::settle`]）。
    Latched,
    /// **聞こうとしたが、往復そのものが失敗した**（接続できない・時間切れ・5xx）。覚えない。
    Failed,
}

/// 1回の封（送る本文を1回数えさせた結果）。1ターンのまとめは [`TurnSeal`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Seal {
    outcome: Outcome,
    /// **封の往復にかかった時間**（聞かなかった回は 0）。
    elapsed: Duration,
}

impl Seal {
    /// 聞かなかった回（この世代は「数えられない」と覚えている）。
    pub(super) const fn remembered() -> Self {
        Self::skipped(Outcome::Remembered)
    }

    /// 聞かなかった回（`outcome` は [`Outcome::Remembered`] か [`Outcome::Latched`]）。
    const fn skipped(outcome: Outcome) -> Self {
        Self {
            outcome,
            elapsed: Duration::ZERO,
        }
    }

    /// 結果の種類（数えた計画が「送る・組み直す・バイト数へ戻る」を決める材料）。
    pub(super) const fn outcome(self) -> Outcome {
        self.outcome
    }

    /// ログの `seal=` の綴り。**定数**（集計はこの欄で引く）。
    pub(super) const fn label(self) -> &'static str {
        match self.outcome {
            Outcome::Counted(_) => "counted",
            Outcome::Uncountable => "uncountable",
            Outcome::Remembered => "remembered",
            Outcome::Latched => "latched",
            Outcome::Failed => "failed",
        }
    }

    /// 封の数。**数えた回だけ `Some`**（数えなかった回を 0 で埋めない ——
    /// 埋めると「封 < 実」が毎ターン成立して見える）。
    pub(super) const fn sealed_tokens(self) -> Option<u32> {
        match self.outcome {
            Outcome::Counted(tokens) => Some(tokens),
            Outcome::Uncountable | Outcome::Remembered | Outcome::Latched | Outcome::Failed => None,
        }
    }

    /// **封の往復の経過ミリ秒**（ADR-0052 M2 の材料 / 作業単位 B-60b のレビュー）。
    /// 聞かなかった回（[`Outcome::Remembered`]）は 0。**本番の行は [`TurnSeal`] の合計を載せる**（テスト専用）。
    #[cfg(test)]
    pub(super) fn elapsed_ms(self) -> u64 {
        millis(self.elapsed)
    }

    /// 封の往復にかかった時間。
    pub(super) const fn elapsed(self) -> Duration {
        self.elapsed
    }
}

fn millis(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// **送った計画の種類**（封の行の `plan=` / 作業単位 B-60d-1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SentPlan {
    /// **数えた計画**（部品を上流で数えて組み、封が取り分以下だった本文）。
    Counted,
    /// **バイト数の計画**（ADR-0052 §決定1 の (i)。上界で示せるので封を要しない）。
    Bytes,
}

impl SentPlan {
    /// ログの `plan=` の綴り。**定数**（集計はこの欄で引く）。
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Counted => "counted",
            Self::Bytes => "bytes",
        }
    }
}

/// **部品を数えた勘定**（ADR-0052 §推奨6「数えきれなかった部品があった回は、その件数をログに出す」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct PartsTally {
    /// **このターンで新しく数えた部品の数**（覚えていた部品は入らない）＝ 部品の数える往復の数。
    pub(super) counted: usize,
    /// **数えに行ったのに数えられなかった部品の数**（聞かなかった回は 0）。
    /// 0 でない回の計画は、その部品だけバイト数で数えている（測定ではその回を除ける）。
    pub(super) uncounted: usize,
    /// 部品を数える往復にかかった時間。
    pub(super) elapsed: Duration,
}

/// **1ターンの封のまとめ**（作業単位 B-60d-1）。封の行（[`report`]）と「数えられない」の記憶
/// （[`UncountableMemory::accepted`] には [`Self::first`] を渡す）の材料。
///
/// | 欄 | 中身 |
/// |---|---|
/// | `first` | **最初の封**の結果。`seal=` の綴りと「数えられない」の記憶はこれで決める |
/// | `sealed_tokens` | **送った本文を数えた数**。送った本文に封をしていない回（バイト数の計画へ戻った回）は `None` |
/// | `seal_elapsed` | 封の往復の合計（組み直した回は2回分） |
/// | `plan` | 送った計画の種類 |
/// | `rebuilds` | 組み直した回数（0 か 1） |
/// | `parts` | 部品を数えた勘定 |
/// | `shared_deduction` | 本文と裏方が同じ上流のとき、数えた計画の窓から差し引いた裏方の1要求分（ADR-0052 §決定7。数えた計画を組まなかった回は 0） |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TurnSeal {
    pub(super) first: Seal,
    pub(super) sealed_tokens: Option<u32>,
    pub(super) seal_elapsed: Duration,
    pub(super) plan: SentPlan,
    pub(super) rebuilds: u8,
    pub(super) parts: PartsTally,
    pub(super) shared_deduction: u32,
}

impl TurnSeal {
    /// **聞かなかった回**（「数えられない」と覚えた世代。バイト数の計画をそのまま送る）。
    /// 本番の経路は [`Self::skipped`] を通る（理由ごとに封の結果が違う）。
    #[cfg(test)]
    pub(super) const fn remembered() -> Self {
        Self::skipped(Seal::remembered())
    }

    /// **聞かなかった回**（`first` は [`UncountableMemory::skipped`] が返したもの。
    /// 「数えられない」と覚えた世代か、掛け金が掛かった世代。バイト数の計画をそのまま送る）。
    pub(super) const fn skipped(first: Seal) -> Self {
        Self {
            first,
            sealed_tokens: None,
            seal_elapsed: Duration::ZERO,
            plan: SentPlan::Bytes,
            rebuilds: 0,
            parts: PartsTally {
                counted: 0,
                uncounted: 0,
                elapsed: Duration::ZERO,
            },
            shared_deduction: 0,
        }
    }

    /// **最初の封**（「数えられない」の記憶に渡す）。
    pub(super) const fn first(self) -> Seal {
        self.first
    }

    /// **送った計画の和は上界か**（第3層の答え合わせ `super::usage::report` に渡す）。
    ///
    /// 上界の判定（見積もり < 実）は、計画の和が上界である回（バイト数の計画）にしか意味を持たない
    /// （数えた計画の和は上界ではない / ADR-0052 §背景）。**コンテキストの超過の判定は、どちらでも行う。**
    /// **封をした回の答え合わせ（封 < 実）は [`Sealed::settle`]**（ADR-0052 §決定5・N4 / 作業単位 B-60d-2）。
    pub(super) const fn estimate(self) -> super::usage::Estimate {
        match self.plan {
            SentPlan::Bytes => super::usage::Estimate::UpperBound,
            SentPlan::Counted => super::usage::Estimate::Planning,
        }
    }
}

/// **なぜその世代を数えないか**（[`UncountableMemory`] に載る理由 / ログの `seal=` で分かれる）。
///
/// **理由ごとに外れ方が違う**（[`Why::still_applies`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Why {
    /// 上流が「数えられない」と答え、同じ写しが本文を受け付けた（ADR-0052 §推奨4 / `seal=remembered`）。
    Uncountable,
    /// 封をした回に `封 < 実` が出た（ADR-0052 §推奨5 の表の2行目 / `seal=latched`）。
    Undercounted,
}

impl Why {
    /// **`stored` の世代で覚えた理由が、いまの世代 `current` にもまだ効くか。**
    ///
    /// | 理由 | 外れる変化 | 理由 |
    /// |---|---|---|
    /// | `Uncountable` | **世代が少しでも変われば**（`==` でない） | 外れても往復が1つ増えるだけ（聞き直して、また覚える） |
    /// | `Undercounted` | **版が変わった・答えた名前が別の `Some` に変わった**ときだけ（`FaceGeneration::same_model_as`） | 引き直しの失敗（名前が `None`）で外すと、食い違いが分かっている数え方で数えた計画をまた送る |
    fn still_applies(self, stored: &FaceGeneration, current: &FaceGeneration) -> bool {
        match self {
            Self::Uncountable => stored == current,
            Self::Undercounted => stored.same_model_as(current),
        }
    }
}

/// **いまの本文の面の世代を読む口**（`AppState::main_generation` と同じもの / [`Sealed`]）。
///
/// 応答の後に「書こうとしている世代が、まだいまの世代か」を確かめるために読む（[`UncountableMemory::latch`]）。
pub(super) type CurrentGeneration = Arc<dyn Fn() -> FaceGeneration + Send + Sync>;

/// **数えない面の世代**（ADR-0052 §推奨4・§推奨5 / §推奨6「DB に持たない」）。
///
/// 名前は B-60b のまま（「数えられない」の記憶）。**作業単位 B-60d-2 から、封の腕の掛け金も同じ記憶に載る。**
///
/// # 掛け金を別の記憶にしなかった理由
///
/// | 観点 | 「数えられない」 | 掛け金（`封 < 実`） |
/// |---|---|---|
/// | 鍵 | 面の世代 | **同じ**（origin#69「同じ形」） |
/// | 置き場所 | プロセスの中だけ | **同じ** |
/// | 掛かった世代の間の振る舞い | 部品も封も聞かず、バイト数の計画を送る | **同じ** |
///
/// 振る舞いが同じなので、**「この世代は数えない」の問いを1か所で答える**（`super::counted_plan` の段0 が見るのはここだけ）。
/// 別にすると、段0 と [`Self::seal`] の両方に2つの記憶を見る分岐が要り、片方だけ見る実装が書けてしまう。
/// **理由は [`Why`] で持ち、ログの `seal=` の綴り（`remembered` / `latched`）で区別できる。**
///
/// **持つのは1つだけ**（本文の面のいまの世代と、その理由）。世代が変われば、覚えた値は鍵が合わずに使われない
/// （どこまで変われば外れるかは理由ごとに違う / [`Why::still_applies`]）。
/// **プロセスの中にしか無い**ので、再起動で冷える（最初のターンで1回聞き直すだけ）。
///
/// # 古い世代の書き込みは捨てる
///
/// 枠が1つなので、**遅れて終わった古い世代のターンが書くと、いまの世代の記憶を消す**
/// （プロファイルを差し替えた後に、前の上流の長い中継が終わって `latch` する等）。
/// だから [`Self::latch`] と [`Self::accepted`] は、**書こうとしている世代がいまの世代にまだ効くときだけ書く。**
/// 加えて、**いまの世代に効いている掛け金を「数えられない」で上書きしない**（どちらも数えないが、外れ方は掛け金の方が固い）。
///
/// **錠は `await` を跨いで持たない**（読む・書くの1手ずつ）。
#[derive(Debug, Default)]
pub(super) struct UncountableMemory(Mutex<Option<(FaceGeneration, Why)>>);

impl UncountableMemory {
    /// **その世代を数えないと覚えているか**（理由は問わない）。
    #[cfg(test)]
    pub(super) fn remembers(&self, generation: &FaceGeneration) -> bool {
        self.skipped(generation).is_some()
    }

    /// **その世代を数えないなら、聞かなかった回の封を返す。** 覚えている世代では、部品も封も数えに行かない
    /// （`super::counted_plan`）。返す封の結果は理由ごとに違う（[`Outcome::Remembered`] / [`Outcome::Latched`]）。
    pub(super) fn skipped(&self, generation: &FaceGeneration) -> Option<Seal> {
        match &*self.0.lock().unwrap_or_else(PoisonError::into_inner) {
            Some((remembered, why)) if why.still_applies(remembered, generation) => {
                Some(match why {
                    Why::Uncountable => Seal::skipped(Outcome::Remembered),
                    Why::Undercounted => Seal::skipped(Outcome::Latched),
                })
            }
            Some(_) | None => None,
        }
    }

    /// **掛け金を掛ける**（その世代は、変わるまでバイト数の計画で送る / [`Sealed::settle`]）。
    ///
    /// **`generation` が `current`（いまの世代）にもう効かないなら捨てる**（型の doc の「古い世代の書き込みは捨てる」）。
    /// 比べ方は掛け金の外れ方と同じ（`FaceGeneration::same_model_as`）—— 名前が `None` になっただけの
    /// いまの世代へは書く（そこで捨てると、引き直しの失敗と重なった回の観測が消える）。
    pub(super) fn latch(&self, generation: FaceGeneration, current: &FaceGeneration) {
        if !Why::Undercounted.still_applies(&generation, current) {
            tracing::debug!("古い世代の封の食い違いなので、掛け金を掛けません");
            return;
        }
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((generation, Why::Undercounted));
    }

    /// **送る直前に封をする**（1回ぶん。結果の使い方は `super::counted_plan`）。
    ///
    /// `request` と `streaming` は、**このあと同じ `llm` で送るものと同じ値**を渡すこと
    /// （[`LlmClient::count_request`] の doc）。`llm` は**そのターンの写し**（このモジュールの doc）。
    ///
    /// **覚えている世代なら聞かない**（[`Outcome::Remembered`]）。**ここでは覚えない**
    /// ——覚えるのは本文が受け付けられた後（[`Self::accepted`]）。
    pub(super) async fn seal(
        &self,
        generation: &FaceGeneration,
        llm: &dyn LlmClient,
        request: &ChatRequest,
        streaming: bool,
    ) -> Seal {
        if let Some(skipped) = self.skipped(generation) {
            return skipped;
        }
        let started = Instant::now();
        let answer = llm.count_request(request, streaming).await;
        let elapsed = started.elapsed();
        let outcome = match answer {
            Ok(TokenCount::Counted(tokens)) => Outcome::Counted(tokens),
            Ok(TokenCount::Uncountable) => Outcome::Uncountable,
            Err(err) => {
                // **警告にしない。** 失敗しても何も止まらない（その回はバイト数の計画で送るだけ）。
                // 往復の失敗そのものは `llm` が既に言っている。
                tracing::debug!(
                    error = %err,
                    "封のために数えられませんでした（今回はバイト数の計画で送ります）"
                );
                Outcome::Failed
            }
        };
        Seal { outcome, elapsed }
    }

    /// **同じ写しが本文を受け付けた後に呼ぶ。** 「数えられない」だった回だけ、その世代を覚える。
    ///
    /// **⚠ [`Outcome::Failed`] を覚えないこと**（このモジュールの doc の表）。
    /// 見張り番は `tests/server_seal.rs` の `a_count_that_failed_is_not_remembered`。
    ///
    /// **`generation` が `current`（いまの世代）と違えば捨てる。** いまの世代に効いている掛け金も上書きしない
    /// （型の doc の「古い世代の書き込みは捨てる」）。
    pub(super) fn accepted(
        &self,
        generation: FaceGeneration,
        current: &FaceGeneration,
        seal: Seal,
    ) {
        match seal.outcome {
            Outcome::Uncountable => {
                if generation != *current {
                    return;
                }
                let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some((latched, Why::Undercounted)) = &*held
                    && Why::Undercounted.still_applies(latched, current)
                {
                    return;
                }
                *held = Some((generation, Why::Uncountable));
            }
            Outcome::Counted(_) | Outcome::Remembered | Outcome::Latched | Outcome::Failed => {}
        }
    }
}

/// **封をした回の答え合わせの判定**（ADR-0052 §推奨5 の表の2・3行目 / 作業単位 B-60d-2）。純粋関数 [`verdict`] の結果。
///
/// 判定した回は **封の数（`sealed`）と実数（`actual`）を持つ**（行に載せる値を、呼び出し側が分解し直さない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    /// **判定しない**（送った本文に封をしていない・`usage` が無い）。
    Unjudged,
    /// `封 = 実`。何もしない。
    Matched { sealed: u32, actual: u32 },
    /// `封 > 実`（多めに数えている＝無害）。`debug` に差を残す。
    Overcounted { sealed: u32, actual: u32 },
    /// `封 < 実`（封が本番の数え方と食い違った）。`warn` ＋ 掛け金。
    Undercounted { sealed: u32, actual: u32 },
}

/// **封の数と実数を比べる**（純粋関数）。
///
/// **比べる封の数は、送った本文を数えた数**（[`TurnSeal`] の `sealed_tokens`）。組み直した回の最初の封
/// （`first`。送らなかった本文の数）ではない。
///
/// **送った本文に封をした回は、どちらの計画でも判定する**（ADR-0052 §推奨5「封をした回で `封 < 実`」/
/// 作業単位 B-60d-2 のレビュー）。バイト数の計画は上界で示せているのでその回は壊れないが、
/// `封 < 実` は「封が本番の数え方と食い違った」という観測で、**次のターンで数えた計画を無防備に送らないため**に掛け金を掛ける。
/// 送った本文に封をしていない回（バイト数の計画へ戻った回・聞かなかった回）と、
/// **`usage` が無い回は判定しない**（0 で埋めない / `super::usage` の doc）。
pub(super) fn verdict(turn: TurnSeal, usage: Option<TokenUsage>) -> Verdict {
    let (Some(sealed), Some(usage)) = (turn.sealed_tokens, usage) else {
        return Verdict::Unjudged;
    };
    let actual = usage.prompt_tokens;
    match sealed.cmp(&actual) {
        std::cmp::Ordering::Less => Verdict::Undercounted { sealed, actual },
        std::cmp::Ordering::Equal => Verdict::Matched { sealed, actual },
        std::cmp::Ordering::Greater => Verdict::Overcounted { sealed, actual },
    }
}

/// **送ったターンの封と、その答え合わせの行き先**（作業単位 B-60d-2）。
///
/// 封のまとめ（[`TurnSeal`]）に、**それを数えた世代と記憶**を添えたもの。
/// 上流の応答の後に [`Self::settle`] を呼ぶと、封の行を出し、`封 < 実` ならその世代に掛け金を掛ける。
///
/// # 判定を `super::usage::report` に足さなかった理由
///
/// * `usage::report` は**送った計画の和**の答え合わせ（ADR-0013 第3層の上界とコンテキストの超過）で、
///   **封の数を知らない**。足すなら封の数と記憶と世代を渡すことになり、上流の応答の観測だけをする関数に
///   「次のターンの計画を変える」副作用が入る。
/// * 封の行（[`report`]）は封の数・実数・計画の和を既に1行に持つ。**判定と掛け金をその隣に置けば、
///   呼び出し口（非ストリーミングと中継の2つ）は今までどおり1つの呼び出しで済む**（片方の経路だけ判定を落とす形を作りにくい）。
///
/// 中継は応答の本体が `'static` の流れなので、記憶は [`Arc`] で持つ。
/// **いまの世代を読む口**（[`CurrentGeneration`]）も持つ —— 書く直前に、書こうとしている世代がまだいまの世代かを確かめる
/// （[`UncountableMemory`] の doc「古い世代の書き込みは捨てる」）。
pub(super) struct Sealed {
    turn: TurnSeal,
    memory: Arc<UncountableMemory>,
    generation: FaceGeneration,
    current: CurrentGeneration,
}

impl Sealed {
    /// `generation` は**そのターンの写しより先に読んだ世代**（`super::counted_plan::Turn::generation` と同じもの）。
    /// `current` は**書く直前に読む**いまの世代（`AppState::main_generation` と同じ読み方）。
    pub(super) fn new(
        turn: TurnSeal,
        memory: Arc<UncountableMemory>,
        generation: FaceGeneration,
        current: CurrentGeneration,
    ) -> Self {
        Self {
            turn,
            memory,
            generation,
            current,
        }
    }

    /// **テスト専用。** 記憶と世代を捨て値で作る（封の判定を見ない中継のテスト）。
    #[cfg(test)]
    pub(super) fn for_tests(turn: TurnSeal) -> Self {
        Self::new(
            turn,
            Arc::new(UncountableMemory::default()),
            FaceGeneration::for_tests(0, None),
            Arc::new(|| FaceGeneration::for_tests(0, None)),
        )
    }

    /// 送った計画の和が上界か（`super::usage::report` に渡す / [`TurnSeal::estimate`]）。
    pub(super) const fn estimate(&self) -> super::usage::Estimate {
        self.turn.estimate()
    }

    /// **同じ写しが本文を受け付けた後に呼ぶ**（[`UncountableMemory::accepted`] に最初の封を渡す）。
    pub(super) fn accepted(&self) {
        self.memory.accepted(
            self.generation.clone(),
            &(self.current)(),
            self.turn.first(),
        );
    }

    /// **上流の応答の後に1回だけ呼ぶ。** 封の行（[`report`]）を出し、封をした回の判定（[`verdict`]）に従う。
    ///
    /// | 判定 | 反応 |
    /// |---|---|
    /// | `封 < 実` | **`warn`**（封の数・実数・計画の和を同じ1行に）＋ **その世代に掛け金**（世代が変わるまでバイト数の計画） |
    /// | `封 > 実` | `debug` に差を残す（同じ3つの数を同じ1行に） |
    /// | `封 = 実`・判定しない | 何もしない |
    ///
    /// `usage` の出どころは `super::usage::report` と同じ（非ストリーミングは応答本文、中継は最終チャンク）。
    pub(super) fn settle(&self, plan: &BudgetPlan, usage: Option<TokenUsage>, streaming: bool) {
        report(self.turn, plan, usage, streaming);
        let planned_tokens = plan.prompt_tokens();
        let sent_plan = self.turn.plan.label();
        match verdict(self.turn, usage) {
            Verdict::Undercounted { sealed, actual } => {
                // **毎ターン鳴らない。** 掛け金が掛かった世代は封をしないので、次に鳴るのは世代が変わった後。
                tracing::warn!(
                    sealed_tokens = sealed,
                    usage_prompt_tokens = actual,
                    planned_tokens,
                    undercounted_by = actual - sealed,
                    plan = %sent_plan,
                    streaming,
                    "{UNDERCOUNTED_LINE}"
                );
                self.memory
                    .latch(self.generation.clone(), &(self.current)());
            }
            Verdict::Overcounted { sealed, actual } => tracing::debug!(
                sealed_tokens = sealed,
                usage_prompt_tokens = actual,
                planned_tokens,
                overcounted_by = sealed - actual,
                plan = %sent_plan,
                streaming,
                "{OVERCOUNTED_LINE}"
            ),
            Verdict::Matched { .. } | Verdict::Unjudged => {}
        }
    }
}

/// `封 < 実` の行のメッセージ。
const UNDERCOUNTED_LINE: &str =
    "封の数が上流の実数を下回りました（この面は、世代が変わるまでバイト数の計画で送ります）";

/// `封 > 実` の行のメッセージ。
const OVERCOUNTED_LINE: &str =
    "封の数が上流の実数を上回りました（多めに数えているだけで、無害です）";

/// 封の行のメッセージ。**テストは先頭の「封を観測しました」で行を選ぶ**（B-60b から変えていない）。
const SEAL_LINE: &str = "封を観測しました（送った本文の数・実数・計画の和）";

/// **上流の応答の後に、そのターンの封を1行に出す**（ADR-0052 §推奨5 / 作業単位 B-60b・B-60d-1）。
///
/// `usage` の出どころは第3層の答え合わせ（`server::usage::report`）と同じ
/// ——非ストリーミングは応答本文の `usage`、中継は**最終チャンク**の `usage`。
///
/// | 欄 | 中身 |
/// |---|---|
/// | `seal` | **最初の封**の [`Seal::label`] |
/// | `sealed_tokens` | **送った本文**を数えた数（送った本文に封をしていない回は `None`） |
/// | `usage_prompt_tokens` | 上流が処理した実数（返さない上流は `None`） |
/// | `planned_tokens` | **送った計画**の和（`BudgetPlan::prompt_tokens`。数えた計画では上界ではない） |
/// | `seal_ms` | **封の往復の経過ミリ秒の合計**（聞かなかった回は 0 / ADR-0052 M2） |
/// | `streaming` | どちらの経路か（M1 は両方を測る） |
/// | `plan` | 送った計画（[`SentPlan::label`]） |
/// | `rebuilds` | 組み直した回数（0 か 1） |
/// | `counted_parts` | **このターンで新しく数えた部品の数**（部品の数える往復の数） |
/// | `uncounted_parts` | **数えに行ったのに数えられなかった部品の数**（ADR-0052 §推奨6） |
/// | `parts_ms` | 部品を数える往復の経過ミリ秒 |
/// | `shared_deduction` | 数えた計画の窓から差し引いた裏方の1要求分（ADR-0052 §決定7。別の上流の回・数えた計画を組まなかった回は 0） |
///
/// **`info` にしてある。** 測定（M1・M2・M3・M6）は本番と同じ既定のログ設定で読むため。
/// 1ターンに1行で、`chat` の「応答を返します」と同じ頻度である。
/// **`封 < 実` の `warn` と掛け金はここではなく [`Sealed::settle`]**（この関数を呼んだ後に判定する / 作業単位 B-60d-2）。
pub(super) fn report(
    turn: TurnSeal,
    plan: &BudgetPlan,
    usage: Option<TokenUsage>,
    streaming: bool,
) {
    tracing::info!(
        seal = %turn.first.label(),
        sealed_tokens = ?turn.sealed_tokens,
        usage_prompt_tokens = ?usage.map(|usage| usage.prompt_tokens),
        planned_tokens = plan.prompt_tokens(),
        seal_ms = millis(turn.seal_elapsed),
        streaming,
        plan = %turn.plan.label(),
        rebuilds = turn.rebuilds,
        counted_parts = turn.parts.counted,
        uncounted_parts = turn.parts.uncounted,
        parts_ms = millis(turn.parts.elapsed),
        shared_deduction = turn.shared_deduction,
        "{SEAL_LINE}"
    );
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::Utc;

    use super::*;
    use crate::domain::fixture_scale;
    use crate::domain::{Character, CharacterId, DisplayName, Handle, Persona};
    use crate::llm::{ChatMessage, ChatStream, Completion, LlmError, LlmFuture};
    use crate::prompt::{CountedParts, PromptInput, TokenBudget, build_prompt};
    use crate::testlog::{
        VERBOSE_FILTER, captured_under_default_filter, captured_with_filter, has_field,
        line_containing,
    };

    /// 数える往復の答えを決めて返す上流。**聞かれた回数を数える。**
    struct Counting {
        answer: fn() -> Result<TokenCount, LlmError>,
        delay: Duration,
        asked: AtomicUsize,
    }

    impl Counting {
        fn new(answer: fn() -> Result<TokenCount, LlmError>) -> Self {
            Self {
                answer,
                delay: Duration::ZERO,
                asked: AtomicUsize::new(0),
            }
        }

        fn asked(&self) -> usize {
            self.asked.load(Ordering::SeqCst)
        }
    }

    impl LlmClient for Counting {
        fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            Box::pin(async { Err(LlmError::MissingContent) })
        }

        fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
            Box::pin(async { Err(LlmError::MissingContent) })
        }

        fn count_request<'a>(
            &'a self,
            _request: &'a ChatRequest,
            _streaming: bool,
        ) -> LlmFuture<'a, TokenCount> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let answer = (self.answer)();
            let delay = self.delay;
            Box::pin(async move {
                tokio::time::sleep(delay).await;
                answer
            })
        }
    }

    fn request() -> ChatRequest {
        ChatRequest::new(vec![ChatMessage::user("ただいま")])
    }

    fn generation(epoch: u64) -> FaceGeneration {
        FaceGeneration::for_tests(epoch, None)
    }

    /// 聞かなかった回以外の封（経過時間は 0 で作る）。
    const fn sealed(outcome: Outcome) -> Seal {
        Seal {
            outcome,
            elapsed: Duration::ZERO,
        }
    }

    /// **3つの答えは、3つの結果に別々に写る**（取り違えると覚える・覚えないが入れ替わる）。
    #[tokio::test]
    async fn each_answer_maps_to_its_own_seal() {
        for (answer, expected) in [
            (
                (|| Ok(TokenCount::Counted(37))) as fn() -> Result<TokenCount, LlmError>,
                Outcome::Counted(37),
            ),
            (|| Ok(TokenCount::Uncountable), Outcome::Uncountable),
            (|| Err(LlmError::MissingContent), Outcome::Failed),
        ] {
            let memory = UncountableMemory::default();
            let llm = Counting::new(answer);
            let seal = memory.seal(&generation(0), &llm, &request(), false).await;
            assert_eq!(seal.outcome(), expected);
            assert_eq!(llm.asked(), 1, "覚えていない世代では1回聞くこと");
        }
    }

    /// **経過時間は封の往復そのもの**（ADR-0052 M2）。聞かなかった回は 0。
    #[tokio::test]
    async fn the_elapsed_time_is_the_round_trip_of_the_count() {
        let memory = UncountableMemory::default();
        let mut llm = Counting::new(|| Ok(TokenCount::Uncountable));
        llm.delay = Duration::from_millis(60);

        let asked = memory.seal(&generation(0), &llm, &request(), false).await;
        assert!(
            asked.elapsed_ms() >= 60,
            "往復の時間を測ること: {}",
            asked.elapsed_ms()
        );

        memory.accepted(generation(0), &generation(0), asked);
        let skipped = memory.seal(&generation(0), &llm, &request(), false).await;
        assert_eq!(skipped.outcome(), Outcome::Remembered);
        assert_eq!(skipped.elapsed_ms(), 0, "聞かなかった回は 0 であること");
    }

    /// **「数えられない」は、本文が受け付けられた後に覚え、同じ世代の間はもう聞かない。**
    /// **別の世代では聞く。**
    #[tokio::test]
    async fn an_accepted_uncountable_answer_is_remembered_for_its_generation_only() {
        let memory = UncountableMemory::default();
        let llm = Counting::new(|| Ok(TokenCount::Uncountable));

        let first = memory.seal(&generation(0), &llm, &request(), false).await;
        assert_eq!(first.outcome(), Outcome::Uncountable);
        assert_eq!(
            memory
                .seal(&generation(0), &llm, &request(), true)
                .await
                .outcome(),
            Outcome::Uncountable,
            "受け付けられる前は覚えないこと"
        );
        assert_eq!(llm.asked(), 2);

        memory.accepted(generation(0), &generation(0), first);
        assert_eq!(
            memory
                .seal(&generation(0), &llm, &request(), false)
                .await
                .outcome(),
            Outcome::Remembered
        );
        assert_eq!(llm.asked(), 2, "覚えた世代では、もう聞きに行かないこと");

        assert_eq!(
            memory
                .seal(
                    &FaceGeneration::for_tests(0, Some("another.gguf")),
                    &llm,
                    &request(),
                    false
                )
                .await
                .outcome(),
            Outcome::Uncountable,
            "答えた名前が違えば別の世代として聞くこと"
        );
        assert_eq!(
            memory
                .seal(&generation(1), &llm, &request(), false)
                .await
                .outcome(),
            Outcome::Uncountable,
            "版が違えば別の世代として聞くこと"
        );
        assert_eq!(llm.asked(), 4);
    }

    /// **数えた回と、往復が失敗した回は覚えない**（このモジュールの doc の表）。
    #[test]
    fn only_an_uncountable_answer_is_remembered() {
        for outcome in [
            Outcome::Counted(3),
            Outcome::Failed,
            Outcome::Remembered,
            Outcome::Latched,
        ] {
            let memory = UncountableMemory::default();
            memory.accepted(generation(0), &generation(0), sealed(outcome));
            assert!(
                !memory.remembers(&generation(0)),
                "{outcome:?} を覚えないこと"
            );
        }
        let memory = UncountableMemory::default();
        memory.accepted(generation(0), &generation(0), sealed(Outcome::Uncountable));
        assert!(memory.remembers(&generation(0)));
    }

    /// **綴りと封の数**（集計はこの欄で引く。数えなかった回を 0 で埋めない）。
    #[test]
    fn labels_and_counts_are_fixed() {
        assert_eq!(
            [
                Outcome::Counted(5),
                Outcome::Uncountable,
                Outcome::Remembered,
                Outcome::Latched,
                Outcome::Failed
            ]
            .map(|outcome| (sealed(outcome).label(), sealed(outcome).sealed_tokens())),
            [
                ("counted", Some(5)),
                ("uncountable", None),
                ("remembered", None),
                ("latched", None),
                ("failed", None),
            ]
        );
    }

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
            TokenBudget::default(),
            &CountedParts::default(),
        )
        .expect("テストのプロンプトは予算に収まること")
        .plan
    }

    /// **全部の値が、既定のログ設定で同じ1行に載る**（CLAUDE.md「ログを見る見張り番は同じ行の中で見る」）。
    ///
    /// **数は互いに違う値**にしてある（取り違えが値の差として出るように）。
    /// **封の数は最初の封（37）ではなく、送った本文の数（29）**を載せる（作業単位 B-60d-1）。
    #[test]
    fn the_seal_line_carries_every_number_in_one_line_under_the_default_filter() {
        let plan = plan();
        let planned = plan.prompt_tokens();
        assert!(
            ![37, 29, 41, 23, 17, 5, 3, 11, 13].contains(&planned),
            "前提: 計画の和が他のどの数とも違う値であること: {planned}"
        );

        let output = captured_under_default_filter(|| {
            report(
                TurnSeal {
                    first: Seal {
                        outcome: Outcome::Counted(37),
                        elapsed: Duration::from_millis(11),
                    },
                    sealed_tokens: Some(29),
                    seal_elapsed: Duration::from_millis(23),
                    plan: SentPlan::Counted,
                    rebuilds: 1,
                    parts: PartsTally {
                        counted: 5,
                        uncounted: 3,
                        elapsed: Duration::from_millis(17),
                    },
                    shared_deduction: 13,
                },
                &plan,
                Some(TokenUsage {
                    prompt_tokens: 41,
                    completion_tokens: None,
                }),
                true,
            );
        });
        let line = line_containing(&output, "封を観測しました");
        assert!(line.contains("INFO"), "既定のログ設定で残ること: {line:?}");
        for field in [
            "seal=counted".to_owned(),
            "sealed_tokens=Some(29)".to_owned(),
            "usage_prompt_tokens=Some(41)".to_owned(),
            format!("planned_tokens={planned}"),
            "seal_ms=23".to_owned(),
            "streaming=true".to_owned(),
            "plan=counted".to_owned(),
            "rebuilds=1".to_owned(),
            "counted_parts=5".to_owned(),
            "uncounted_parts=3".to_owned(),
            "parts_ms=17".to_owned(),
            "shared_deduction=13".to_owned(),
        ] {
            assert!(has_field(line, &field), "{field} を載せること: {line:?}");
        }

        let output = captured_under_default_filter(|| {
            report(TurnSeal::remembered(), &plan, None, false);
        });
        let line = line_containing(&output, "封を観測しました");
        for field in [
            "seal=remembered",
            "sealed_tokens=None",
            "usage_prompt_tokens=None",
            "seal_ms=0",
            "streaming=false",
            "plan=bytes",
            "rebuilds=0",
            "counted_parts=0",
            "uncounted_parts=0",
            "shared_deduction=0",
        ] {
            assert!(has_field(line, field), "{field} を載せること: {line:?}");
        }
    }

    // ------------------------------------------------------------ 第3層の封の腕（B-60d-2）

    /// **数えた計画を、封の数 `sealed` で送った回**（封の腕の判定の材料）。
    fn counted_turn(sealed_tokens: u32) -> TurnSeal {
        TurnSeal {
            first: sealed(Outcome::Counted(sealed_tokens)),
            sealed_tokens: Some(sealed_tokens),
            seal_elapsed: Duration::ZERO,
            plan: SentPlan::Counted,
            rebuilds: 0,
            parts: PartsTally::default(),
            shared_deduction: 0,
        }
    }

    fn usage(prompt_tokens: u32) -> Option<TokenUsage> {
        Some(TokenUsage {
            prompt_tokens,
            completion_tokens: None,
        })
    }

    /// **バイト数の計画に、封の数 `sealed` で封をして送った回**（何も数えられなかった回の形）。
    fn sealed_byte_turn(sealed_tokens: u32) -> TurnSeal {
        TurnSeal {
            plan: SentPlan::Bytes,
            ..counted_turn(sealed_tokens)
        }
    }

    /// **組み直した回**: 最初の封は `first`（送らなかった本文）、送った本文の封は `sent`。
    fn rebuilt_turn(first: u32, sent: u32) -> TurnSeal {
        TurnSeal {
            first: sealed(Outcome::Counted(first)),
            rebuilds: 1,
            ..counted_turn(sent)
        }
    }

    fn generation_named(epoch: u64, name: Option<&str>) -> FaceGeneration {
        FaceGeneration::for_tests(epoch, name)
    }

    /// 決まった世代を「いま」と答える口。
    fn current_is(epoch: u64, name: Option<&'static str>) -> CurrentGeneration {
        Arc::new(move || FaceGeneration::for_tests(epoch, name))
    }

    /// **判定は、送った本文に封をした回の封と実数だけで決まる**（ADR-0052 §推奨5 の表の2・3行目）。
    ///
    /// 境目（1 違い・等しい）と、判定しない回（`usage` が無い・聞かなかった回・バイト数の計画へ戻った回）を並べる。
    /// **判定した回は封と実数をそのまま持つ**（行に載せる値）。
    #[test]
    fn the_verdict_compares_the_sealed_count_of_the_sent_body_with_the_usage() {
        assert_eq!(
            verdict(counted_turn(29), usage(41)),
            Verdict::Undercounted {
                sealed: 29,
                actual: 41
            }
        );
        assert_eq!(
            verdict(counted_turn(29), usage(30)),
            Verdict::Undercounted {
                sealed: 29,
                actual: 30
            }
        );
        assert_eq!(
            verdict(counted_turn(29), usage(29)),
            Verdict::Matched {
                sealed: 29,
                actual: 29
            }
        );
        assert_eq!(
            verdict(counted_turn(29), usage(28)),
            Verdict::Overcounted {
                sealed: 29,
                actual: 28
            }
        );
        assert_eq!(
            verdict(counted_turn(0), usage(u32::MAX)),
            Verdict::Undercounted {
                sealed: 0,
                actual: u32::MAX
            },
            "端の値で panic しないこと"
        );
        assert_eq!(
            verdict(counted_turn(29), None),
            Verdict::Unjudged,
            "usage が無ければ判定しないこと（0 で埋めない）"
        );
        assert_eq!(
            verdict(TurnSeal::remembered(), usage(41)),
            Verdict::Unjudged,
            "聞かなかった回は判定しないこと"
        );

        // **組み直した回は、送った本文の封で比べる**（最初の封 50 は 41 より多いが、送った本文の封 29 は少ない）。
        assert_eq!(
            verdict(rebuilt_turn(50, 29), usage(41)),
            Verdict::Undercounted {
                sealed: 29,
                actual: 41
            },
            "送らなかった本文の封で比べないこと"
        );

        // **バイト数の計画に封をした回も判定する**（作業単位 B-60d-2 のレビュー 提案3）。
        assert_eq!(
            verdict(sealed_byte_turn(29), usage(41)),
            Verdict::Undercounted {
                sealed: 29,
                actual: 41
            }
        );
        // バイト数の計画へ戻った回（送った本文に封をしていない）は判定しない。
        let fell_back = TurnSeal {
            sealed_tokens: None,
            ..sealed_byte_turn(29)
        };
        assert_eq!(verdict(fell_back, usage(41)), Verdict::Unjudged);
    }

    /// 封の腕の `warn` の手掛かり。
    const UNDERCOUNTED_MARKER: &str = "封の数が上流の実数を下回りました";
    /// 封の腕の `debug` の手掛かり。
    const OVERCOUNTED_MARKER: &str = "封の数が上流の実数を上回りました";

    /// **`封 < 実` は、`warn` を封の数・実数・計画の和と同じ1行に出し、その世代を掛け金で止める。**
    ///
    /// 3つの数は互いに違う値（29 / 41 / 計画の和）。**別の世代は止めない。**
    #[test]
    fn an_undercounted_seal_warns_with_three_numbers_in_one_line_and_latches_its_generation() {
        let plan = plan();
        let planned = plan.prompt_tokens();
        assert!(
            ![29, 41].contains(&planned),
            "前提: 計画の和が封とも実数とも違う値であること: {planned}"
        );
        let memory = Arc::new(UncountableMemory::default());
        let sealed = Sealed::new(
            counted_turn(29),
            Arc::clone(&memory),
            generation(3),
            current_is(3, None),
        );

        let output = captured_under_default_filter(|| sealed.settle(&plan, usage(41), true));
        let line = line_containing(&output, UNDERCOUNTED_MARKER);
        assert!(line.contains("WARN"), "警告として出すこと: {line:?}");
        for field in [
            "sealed_tokens=29".to_owned(),
            "usage_prompt_tokens=41".to_owned(),
            format!("planned_tokens={planned}"),
            "undercounted_by=12".to_owned(),
            "plan=counted".to_owned(),
            "streaming=true".to_owned(),
        ] {
            assert!(
                has_field(line, &field),
                "{field} を同じ行に載せること: {line:?}"
            );
        }
        assert!(
            output.contains("封を観測しました"),
            "封の行も今までどおり出すこと: {output}"
        );

        assert_eq!(
            memory.skipped(&generation(3)).map(Seal::outcome),
            Some(Outcome::Latched),
            "その世代を掛け金で止めること"
        );
        assert_eq!(
            memory.skipped(&generation(4)),
            None,
            "別の世代は止めないこと"
        );
    }

    /// **`封 > 実` は `debug` に差を残すだけ、`封 = 実` は何もしない。どちらも掛け金を掛けない。**
    #[test]
    fn an_overcounted_or_matching_seal_does_not_latch() {
        let plan = plan();
        let planned = plan.prompt_tokens();
        assert!(
            ![29, 23].contains(&planned),
            "前提: 計画の和が封とも実数とも違う値であること: {planned}"
        );

        let memory = Arc::new(UncountableMemory::default());
        let over = Sealed::new(
            counted_turn(29),
            Arc::clone(&memory),
            generation(0),
            current_is(0, None),
        );
        let quiet = captured_under_default_filter(|| over.settle(&plan, usage(23), false));
        assert!(
            quiet.contains("封を観測しました"),
            "前提: 封の行は出ていること: {quiet}"
        );
        assert!(
            !quiet.contains("WARN"),
            "多めに数えるのは無害なので警告しないこと: {quiet}"
        );
        let verbose = captured_with_filter(VERBOSE_FILTER, || over.settle(&plan, usage(23), false));
        let line = line_containing(&verbose, OVERCOUNTED_MARKER);
        assert!(line.contains("DEBUG"), "{line:?}");
        for field in [
            "sealed_tokens=29".to_owned(),
            "usage_prompt_tokens=23".to_owned(),
            format!("planned_tokens={planned}"),
            "overcounted_by=6".to_owned(),
            "streaming=false".to_owned(),
        ] {
            assert!(
                has_field(line, &field),
                "{field} を同じ行に載せること: {line:?}"
            );
        }
        assert_eq!(memory.skipped(&generation(0)), None, "掛け金を掛けないこと");

        let matched = Sealed::new(
            counted_turn(29),
            Arc::clone(&memory),
            generation(0),
            current_is(0, None),
        );
        let verbose =
            captured_with_filter(VERBOSE_FILTER, || matched.settle(&plan, usage(29), false));
        assert!(
            verbose.contains("封を観測しました"),
            "前提: 封の行は出ていること: {verbose}"
        );
        assert!(
            !verbose.contains(UNDERCOUNTED_MARKER) && !verbose.contains(OVERCOUNTED_MARKER),
            "一致した回は何も言わないこと: {verbose}"
        );
        assert_eq!(memory.skipped(&generation(0)), None, "掛け金を掛けないこと");
    }

    /// **掛け金と「数えられない」は同じ記憶に載り、理由はログの綴りで分かれる**（作業単位 B-60d-2）。
    ///
    /// 掛かった世代では封を聞かない（`seal=latched`）。世代が変われば外れる。
    #[tokio::test]
    async fn a_latched_generation_is_not_sealed_and_names_why() {
        let memory = UncountableMemory::default();
        let llm = Counting::new(|| Ok(TokenCount::Counted(5)));
        memory.latch(generation(0), &generation(0));
        let skipped = memory.seal(&generation(0), &llm, &request(), false).await;
        assert_eq!(skipped.outcome(), Outcome::Latched);
        assert_eq!(skipped.label(), "latched");
        assert_eq!(skipped.sealed_tokens(), None);
        assert_eq!(llm.asked(), 0, "掛かった世代では聞かないこと");
        assert!(memory.remembers(&generation(0)));

        // **受け付けても上書きしない**（掛かった回の封は聞いていないので、覚えることが無い）。
        memory.accepted(generation(0), &generation(0), skipped);
        assert_eq!(
            memory.skipped(&generation(0)).map(Seal::label),
            Some("latched")
        );

        assert_eq!(
            memory
                .seal(&generation(1), &llm, &request(), false)
                .await
                .outcome(),
            Outcome::Counted(5),
            "世代が変われば掛け金は外れること"
        );
        assert_eq!(llm.asked(), 1);

        // 「数えられない」の側は `remembered` のまま（理由がログで区別できる）。
        let uncountable = UncountableMemory::default();
        uncountable.accepted(generation(0), &generation(0), sealed(Outcome::Uncountable));
        assert_eq!(
            uncountable.skipped(&generation(0)).map(Seal::label),
            Some("remembered")
        );
    }

    /// **バイト数の計画に封をした回も、`封 < 実` なら掛け金を掛ける。`封 = 実` では掛けない**
    /// （作業単位 B-60d-2 のレビュー 提案3）。行は `plan=bytes` を名乗る。
    #[test]
    fn an_undercounted_seal_on_the_byte_plan_latches_too() {
        let plan = plan();
        let memory = Arc::new(UncountableMemory::default());
        let under = Sealed::new(
            sealed_byte_turn(29),
            Arc::clone(&memory),
            generation(0),
            current_is(0, None),
        );
        let output = captured_under_default_filter(|| under.settle(&plan, usage(41), false));
        let line = line_containing(&output, UNDERCOUNTED_MARKER);
        assert!(
            has_field(line, "plan=bytes") && has_field(line, "sealed_tokens=29"),
            "{line:?}"
        );
        assert_eq!(
            memory.skipped(&generation(0)).map(Seal::outcome),
            Some(Outcome::Latched)
        );

        let memory = Arc::new(UncountableMemory::default());
        let matched = Sealed::new(
            sealed_byte_turn(29),
            Arc::clone(&memory),
            generation(0),
            current_is(0, None),
        );
        matched.settle(&plan, usage(29), false);
        assert_eq!(
            memory.skipped(&generation(0)),
            None,
            "封 = 実では掛けないこと"
        );
    }

    /// **組み直した回は、送った本文の封で判定して掛ける**（最初の封 50 > 実 41 でも、送った本文の封 29 < 実）。
    #[test]
    fn a_rebuilt_turn_is_judged_by_the_seal_of_the_body_it_sent() {
        let plan = plan();
        let memory = Arc::new(UncountableMemory::default());
        let rebuilt = Sealed::new(
            rebuilt_turn(50, 29),
            Arc::clone(&memory),
            generation(0),
            current_is(0, None),
        );
        let output = captured_under_default_filter(|| rebuilt.settle(&plan, usage(41), false));
        let line = line_containing(&output, UNDERCOUNTED_MARKER);
        assert!(has_field(line, "sealed_tokens=29"), "{line:?}");
        assert_eq!(
            memory.skipped(&generation(0)).map(Seal::outcome),
            Some(Outcome::Latched)
        );
    }

    /// **古い世代の書き込みは、いまの世代の掛け金を消さない**（作業単位 B-60d-2 のレビュー 要修正1）。
    ///
    /// | 筋書き | 順 |
    /// |---|---|
    /// | A（差し替えの後に、前の上流のターンが受け付けられた） | `latch(G1)` → `accepted(G0, Uncountable)` |
    /// | B（差し替えの後に、前の上流の長い中継が終わった） | `latch(G1)` → `latch(G0)`（[`Sealed::settle`] を通す） |
    #[test]
    fn a_write_from_an_old_generation_does_not_replace_the_current_latch() {
        let (g0, g1) = (generation(0), generation(1));

        let memory = Arc::new(UncountableMemory::default());
        memory.latch(g1.clone(), &g1);
        memory.accepted(g0.clone(), &g1, sealed(Outcome::Uncountable));
        assert_eq!(
            memory.skipped(&g1).map(Seal::outcome),
            Some(Outcome::Latched),
            "筋書き A: 古い世代の「数えられない」が G1 の掛け金を消さないこと"
        );
        assert_eq!(memory.skipped(&g0), None, "古い世代を覚えないこと");

        // **掛け金の無いときも、古い世代の「数えられない」は書かない**（上の腕は「掛け金を上書きしない」の
        // 守りにも当たるので、世代の確認を外しても緑になる —— 2つの守りが互いを隠さない腕）。
        let memory = UncountableMemory::default();
        memory.accepted(g1.clone(), &g1, sealed(Outcome::Uncountable));
        memory.accepted(g0.clone(), &g1, sealed(Outcome::Uncountable));
        assert_eq!(
            memory.skipped(&g1).map(Seal::outcome),
            Some(Outcome::Remembered),
            "古い世代の「数えられない」が G1 の記憶を書き換えないこと"
        );
        assert_eq!(memory.skipped(&g0), None, "古い世代を覚えないこと");

        let memory = Arc::new(UncountableMemory::default());
        memory.latch(g1.clone(), &g1);
        let late = Sealed::new(
            counted_turn(29),
            Arc::clone(&memory),
            g0.clone(),
            current_is(1, None),
        );
        late.settle(&plan(), usage(41), true);
        assert_eq!(
            memory.skipped(&g1).map(Seal::outcome),
            Some(Outcome::Latched),
            "筋書き B: 古い世代の掛け金が G1 の掛け金を書き換えないこと"
        );
        memory.latch(g0.clone(), &g1);
        assert_eq!(
            memory.skipped(&g1).map(Seal::outcome),
            Some(Outcome::Latched)
        );
        assert_eq!(memory.skipped(&g0), None, "古い世代を覚えないこと");

        // **対照**: いまの世代の書き込みは書く（古い「数えられない」を、いまの世代の掛け金が置き換える）。
        let memory = UncountableMemory::default();
        memory.accepted(g1.clone(), &g1, sealed(Outcome::Uncountable));
        assert_eq!(
            memory.skipped(&g1).map(Seal::outcome),
            Some(Outcome::Remembered)
        );
        memory.latch(g1.clone(), &g1);
        assert_eq!(
            memory.skipped(&g1).map(Seal::outcome),
            Some(Outcome::Latched)
        );
        // **いまの世代の掛け金を「数えられない」で上書きしない**（遅れて受け付けられた同じ世代のターン）。
        memory.accepted(g1.clone(), &g1, sealed(Outcome::Uncountable));
        assert_eq!(
            memory.skipped(&g1).map(Seal::outcome),
            Some(Outcome::Latched)
        );
    }

    /// **掛け金は、版が変わったか、答えた名前が別の名前に変わったときだけ外れる**（作業単位 B-60d-2 のレビュー 提案4）。
    ///
    /// 引き直しの失敗（名前が `None`）では外れない。**「数えられない」は今までどおり、世代が少しでも変われば外れる**（対照）。
    #[test]
    fn a_latch_survives_a_failed_refresh_but_not_another_model() {
        let named = generation_named(5, Some("m"));
        let memory = UncountableMemory::default();
        memory.latch(named.clone(), &named);
        for (now, latched, why) in [
            (
                generation_named(5, None),
                true,
                "引き直しの失敗では外れないこと",
            ),
            (
                generation_named(5, Some("m")),
                true,
                "同じ名前に戻っても掛かったまま",
            ),
            (
                generation_named(5, Some("n")),
                false,
                "別の名前に変わったら外れること",
            ),
            (
                generation_named(6, Some("m")),
                false,
                "版が変わったら外れること",
            ),
            (
                generation_named(6, None),
                false,
                "版が変われば名前が分からなくても外れること",
            ),
        ] {
            assert_eq!(
                memory.skipped(&now).map(Seal::outcome),
                latched.then_some(Outcome::Latched),
                "{why}: {now:?}"
            );
        }

        // **名前が分からない世代に掛けた掛け金への書き込みも同じ規則**（いまの名前が `None` でも、同じ版なら書く）。
        let memory = UncountableMemory::default();
        memory.latch(named.clone(), &generation_named(5, None));
        assert_eq!(
            memory.skipped(&named).map(Seal::outcome),
            Some(Outcome::Latched)
        );

        // 対照: 「数えられない」は名前が `None` になっただけで外れる（往復が1つ増えるだけ）。
        let memory = UncountableMemory::default();
        memory.accepted(named.clone(), &named, sealed(Outcome::Uncountable));
        assert_eq!(
            memory.skipped(&named).map(Seal::outcome),
            Some(Outcome::Remembered)
        );
        assert_eq!(memory.skipped(&generation_named(5, None)), None);
    }
}
