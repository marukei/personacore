//! トークン予算の配分。
//!
//! # 4つの枠
//!
//! docs/02-architecture.md「コンテキスト予算」の枠をそのまま実装する。
//!
//! | 枠 | 中身 | 削るか |
//! |---|---|---|
//! | システムプロンプト | ペルソナ＋性格＋出力形式指示＋今回のユーザー発言 | **削らない** |
//! | メモリ | 想起で選んだ分（ADR-0023） | 残りを履歴と**半々**に分ける |
//! | 会話履歴 | 直近 N 件 | 残りを記憶と**半々**に分ける |
//! | 生成用の余白 | 応答トークン分 | **絶対に削らない** |
//!
//! # 削り方（明示 / origin#56 で改めた）
//!
//! 1. 削れない枠と余白を引いた**残りを、記憶と履歴で半々に分ける**（[`Frames`]）。
//!    **片方が取り分を余らせたら、もう片方が使う。**
//! 2. 記憶も履歴も、自分の取り分の中で**古い順に落とす**。
//! 3. 削れない枠だけで入らなければ `PromptError::BudgetExhausted`。
//!    **余白を削って押し込むことはしない。** 削ると応答が途中で切れる。
//!
//! > **以前は記憶を履歴より優先していた**（ADR-0003 の全件注入の方針。
//! > 「履歴を 0 件まで落としてから記憶に手を付ける」）。想起の選別（15b）が
//! > 残りを丸ごと埋めるようになってから、**記憶が溜まると履歴が常に 0 件**になり、
//! > 会話の流れを見ない応答になった（origin#56 の測定）。理由の詳細は
//! > [`HISTORY_SHARE_NUMERATOR`] の doc。
//!
//! # 履歴を削る単位は**往復**（ADR-0025）
//!
//! 会話履歴は `[利用者, キャラクター, 利用者, …]` の平坦な列なので、
//! **件数で削ると `system` の直後が `assistant` になる予算が現れ、上流が拒否する。**
//! [`align_to_a_turn`] が採用開始を**偶数へ切り上げる**（＝1件多く、古い側から落とす）。
//! **切り下げない。** 切り下げると採用が1件増えて取り分を超える。
//!
//! # ベクトル検索への移行信号は、ここにはもう無い（15b で撤去）
//!
//! ADR-0003 の判断基準は **`memory_tokens` がプロンプト取り分の半分を超えたか**で、
//! `BudgetPlan::memory_dominates_the_prompt` がその判定を持っていた。
//! **移行そのものが 15b（ADR-0023 の3層想起）で完了し、
//! しかもこの信号は選別の導入後、記憶が増えると鳴りっぱなしになる**ので撤去した。
//! 後継は `super::report_recall` の `info`（`omitted`）。理由はそちらの doc。
//!
//! # 見積もりから独立している
//!
//! この層は**文字列を一切見ない。** コストは `u32` の配列として受け取る。
//! トークン見積もり（`super::estimate`）を差し替えても、ここのテストは壊れない。

/// コンテキスト全体の割り当て。
///
/// **`new` が唯一のコンストラクタ**で、「余白が確保できない設定」は作れない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenBudget {
    context_tokens: u32,
    response_tokens: u32,
}

impl TokenBudget {
    /// 既定のコンテキスト長。docs/04-environment.md の「実用 8k〜16k」の下限を採る。
    pub const DEFAULT_CONTEXT_TOKENS: u32 = 8_192;

    /// 既定の生成用の余白。
    pub const DEFAULT_RESPONSE_TOKENS: u32 = 1_024;

    /// 唯一のコンストラクタ。
    pub fn new(context_tokens: u32, response_tokens: u32) -> Result<Self, BudgetError> {
        if response_tokens == 0 {
            return Err(BudgetError::NoResponseRoom);
        }
        if context_tokens <= response_tokens {
            return Err(BudgetError::ReservationTooLarge);
        }

        Ok(Self {
            context_tokens,
            response_tokens,
        })
    }

    /// コンテキスト全体の上限。
    pub const fn context_tokens(self) -> u32 {
        self.context_tokens
    }

    /// 生成用に必ず残すトークン数。
    pub const fn response_tokens(self) -> u32 {
        self.response_tokens
    }

    /// プロンプトに使ってよい上限。
    ///
    /// **`context - response`。この差を侵す実装を書かないこと。**
    /// 侵すと応答が途中で切れる（利用者から見て最も分かりにくい壊れ方をする）。
    pub const fn prompt_allowance(self) -> u32 {
        self.context_tokens - self.response_tokens
    }
}

impl Default for TokenBudget {
    fn default() -> Self {
        // `new` の検証を通る値であることは `default_is_a_valid_budget` が固定している。
        Self {
            context_tokens: Self::DEFAULT_CONTEXT_TOKENS,
            response_tokens: Self::DEFAULT_RESPONSE_TOKENS,
        }
    }
}

/// 予算の設定値が受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BudgetError {
    /// 生成用の余白が 0。
    #[error("生成用の余白が確保されていません")]
    NoResponseRoom,

    /// 生成用の余白がコンテキスト上限以上で、プロンプトの取り分が残らない。
    #[error("生成用の余白がコンテキスト上限を超えています")]
    ReservationTooLarge,
}

/// プロンプトを組み立てられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PromptError {
    /// 削れない枠（システムプロンプト＋今回の発言）だけで予算を超えた。
    #[error("プロンプトがコンテキスト予算に収まりません")]
    BudgetExhausted,
}

/// 予算配分の結果。**どの枠をどれだけ採用したか**を呼び出し側から観測できる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetPlan {
    memory_start: usize,
    memory_count: usize,
    history_start: usize,
    history_count: usize,
    /// 往復の境目へ揃えるために**追加で落とした履歴の件数**（0 か 1）。
    history_aligned_away: usize,
    fixed_tokens: u32,
    /// 有効な取り決めの件数（作業単位 A-12a。載せた分と落とした分の和）。
    agreement_count: usize,
    /// **載せた取り決めの件数**（新しい側から。[`Frames::new`] が決める）。
    agreements_kept: usize,
    /// 取り決めの節の見積もり（**優先の枠** / ADR-0051 §決定4 の追記）。
    agreement_tokens: u32,
    /// 記憶に許した枠（[`Frames::memory`]）。想起が受け取ったのと同じ値。
    memory_frame: u32,
    memory_tokens: u32,
    history_tokens: u32,
    response_tokens: u32,
    prompt_allowance: u32,
}

impl BudgetPlan {
    /// 採用する記憶の開始添字（これより前は落とした＝古い側）。
    pub const fn memory_start(&self) -> usize {
        self.memory_start
    }

    /// 採用する履歴の開始添字（これより前は落とした＝古い側）。
    pub const fn history_start(&self) -> usize {
        self.history_start
    }

    /// 落とした記憶の件数。
    ///
    /// **本番経路では常に 0**（`plan` の「後詰めの保険」の doc）。
    /// 想起が欠けたことは `super::RecallCounts::omitted` に出る。
    pub const fn dropped_memories(&self) -> usize {
        self.memory_start
    }

    /// 落とした履歴の件数。
    pub const fn dropped_history(&self) -> usize {
        self.history_start
    }

    /// 採用した記憶の件数。
    pub const fn included_memories(&self) -> usize {
        self.memory_count - self.memory_start
    }

    /// 採用した履歴の件数。
    pub const fn included_history(&self) -> usize {
        self.history_count - self.history_start
    }

    /// 往復の境目へ揃えるために**追加で落とした履歴の件数**（0 か 1）。
    ///
    /// **掃引の見張り番の対照**（ADR-0025 §4）。「不変条件を破る入力を1つも
    /// 見つけられなかった」と「**そもそも壊れる状態に入っていない**」は別物なので、
    /// 掃引の側で**丸めが実際に働いた回数**を数えられるようにしてある。
    /// ここが 0 のまま終わった掃引は、見張り番として何も見ていない。
    pub const fn history_aligned_away(&self) -> usize {
        self.history_aligned_away
    }

    /// 削らない枠（システムプロンプト＋今回の発言）の見積もり。
    ///
    /// **取り決めの節は入らない**（作業単位 A-12a の決め直し —— 取り決めは削れない枠の次の
    /// **優先の枠**。[`Self::agreement_tokens`]）。
    pub const fn fixed_tokens(&self) -> u32 {
        self.fixed_tokens
    }

    /// 載せた取り決めの件数（**新しい側から**）。
    pub const fn included_agreements(&self) -> usize {
        self.agreements_kept
    }

    /// **予算に入りきらずに落とした取り決めの件数**（古い側から）。**0 でなければ `warn` が出る**
    /// （`super::report_agreements`）。黙って落とさない。
    pub const fn dropped_agreements(&self) -> usize {
        self.agreement_count.saturating_sub(self.agreements_kept)
    }

    /// 取り決めの節の見積もり（**優先の枠**。載せた分の見出し・小見出し・行を含む）。
    pub const fn agreement_tokens(&self) -> u32 {
        self.agreement_tokens
    }

    /// 記憶に許した枠（残りの半分 ＋ 履歴が先取りで使わなかった分 / origin#56）。
    ///
    /// **`super::build_prompt` が想起の選別に渡したのと同じ値**（どちらも
    /// [`Frames::new`] から出る）。`memory_tokens` はこれを超えない。
    pub const fn memory_frame(&self) -> u32 {
        self.memory_frame
    }

    /// 記憶の枠の見積もり。
    pub const fn memory_tokens(&self) -> u32 {
        self.memory_tokens
    }

    /// 会話履歴の枠の見積もり。
    pub const fn history_tokens(&self) -> u32 {
        self.history_tokens
    }

    /// 生成用に残した余白。
    pub const fn response_tokens(&self) -> u32 {
        self.response_tokens
    }

    /// この配分が使ってよかったプロンプトの取り分（`context - response`）。
    pub const fn prompt_allowance(&self) -> u32 {
        self.prompt_allowance
    }

    /// プロンプト側の合計見積もり。
    pub fn prompt_tokens(&self) -> u32 {
        self.fixed_tokens
            .saturating_add(self.agreement_tokens)
            .saturating_add(self.memory_tokens)
            .saturating_add(self.history_tokens)
    }

    /// プロンプトと余白を合わせた合計。**コンテキスト上限を超えない。**
    pub fn total_tokens(&self) -> u32 {
        self.prompt_tokens().saturating_add(self.response_tokens)
    }
}

/// 削れない枠を引いた残りのうち、**会話履歴が先に使ってよい割合**（分子）。
///
/// # なぜ分けるか（origin#56）
///
/// 以前は記憶が残りを丸ごと使ってよく、履歴は記憶の後の余りしか貰えなかった。
/// **記憶が溜まると記憶が先に全部取り、履歴が 0 件になる**
/// （弥生の DB で、有効な記憶 547 件・再現の 30 ターンすべてで `kept=0`）。
/// そうなると、**会話の流れを1行も見ずに記憶の定型を組み直す応答になる**
/// （origin#56 の測定）。
///
/// いまは半々。**比率は測ってから動かす**（`super::recall` の直近と関連の半々と同じ立場）。
const HISTORY_SHARE_NUMERATOR: u32 = 1;

/// 会話履歴が先に使ってよい割合（分母）。
const HISTORY_SHARE_DENOMINATOR: u32 = 2;

/// **比率そのものはコンパイル時に縛る。**
///
/// 「分子＝0」は記憶が全部取る形（origin#56 の壊れ方）、「分子＝分母」は
/// 履歴が全部取る形で、どちらも半々の意味が無い。
/// **これで「記憶が全部取る形」を表現できなくしたわけではない**
/// （[`history_share`] が 0 を返す実装はこのまま通る）。
/// そちらを落とすのは見張り番 `memories_do_not_crowd_the_history_out`。
const _: () = assert!(
    HISTORY_SHARE_NUMERATOR > 0 && HISTORY_SHARE_NUMERATOR < HISTORY_SHARE_DENOMINATOR,
    "履歴の先取り分は 0 より大きく、残りより小さいこと"
);

/// 履歴の先取り分。**残りに対する割合**で、切り捨て。
///
/// 掛けてから割るので `u64` で受ける（`u32` のままだと大きな残りで溢れる）。
fn history_share(remaining: u32) -> u32 {
    let scaled = u64::from(remaining) * u64::from(HISTORY_SHARE_NUMERATOR)
        / u64::from(HISTORY_SHARE_DENOMINATOR.max(1));
    u32::try_from(scaled).unwrap_or(remaining)
}

/// 削れない枠を引いた残りの、**記憶と履歴への分け方**。
///
/// # 取り分 —— 半々。片方が余ったらもう片方が使う
///
/// 1. 履歴が、残りの [`HISTORY_SHARE_NUMERATOR`] / [`HISTORY_SHARE_DENOMINATOR`] の
///    中で**新しい順に**（往復の単位で）入るだけ入る。
///    **取り分に1往復も入らなくても、最新の1往復が残りに入るならそれだけは入る**
///    （B-59 のレビュー。長い返答1回で履歴 0 件に戻らないように）。
/// 2. **記憶の枠は「残り − 1. で履歴が使った分」。** ＝ 残りの半分 ＋ 履歴が使わなかった分。
/// 3. 記憶を入れた後、**履歴は「残り − 記憶が使った分」まで伸びる**（`plan`）。
///    ＝ 記憶が使わなかった分を履歴が受け取る。
///
/// # **`plan` と、想起の選別（`super::recall`）が同じ値を見るための1本**
///
/// 選別は「記憶に回してよい枠」を先に知る必要があるが、
/// **その枠の計算を2箇所に書くと、片方だけ緩んだときに
/// 選別が `plan` の想定より多く選び、`plan` が黙って削る**という
/// 二段構えの壊れ方になる（どちらの見張り番も緑のまま）。
/// だから**残りも記憶の枠も、この関数だけが出す。**
///
/// # 取り決めの優先の枠（作業単位 A-12a の決め直し / ADR-0051 §決定4 の追記）
///
/// 取る順は **削れない枠 → 最新の1往復（残りに入るなら） → 取り決め → 残りを記憶と履歴で半々**。
///
/// * **最新の1往復を取り決めより先に確保する**（司令塔の決定 / ADR-0051 の追記）。取り決めが残りを
///   取りすぎると、B-59 の先取り（最新の1往復）の入る隙間が消え、**履歴が 0 件**になる
///   （既定の予算・長いペルソナ・取り決め2件で残りが約 40 になる筋書き。見張り番は
///   `super::tests::the_newest_exchange_is_kept_before_the_agreements_take_the_rest`）。
/// * 取り決めは、削れない枠と最新の1往復を引いた残りから、要るだけを取る
///   （入りきらないときだけ**古い側から**落とす）。
/// * **取り決めが使った分を引いた残り**を記憶と履歴で半々に分ける（最新の1往復はその中の履歴の側に入る）。
///
/// 取り決めの件数も、ここが1回だけ決める（想起の選別と `plan` が別々に数えると、二段構えの壊れ方に戻る）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Frames {
    /// 削れない枠と**取り決めが使った分**を引いた残り（記憶と履歴で分ける）。
    remaining: u32,
    memory: u32,
    /// 載せる取り決めの件数（新しい側から）。
    agreements_kept: usize,
    /// その件数の取り決めの節の見積もり。
    agreement_tokens: u32,
}

impl Frames {
    /// 削れない枠だけで超えていたら諦める。`history_costs` は**古い順**。
    ///
    /// `agreement_costs[k]` は**新しい側から k 件の取り決めを載せたときの節の費用**
    /// （k = 0〜件数。`[0]` から始まり、件数とともに増える）。空なら取り決めは無い。
    /// 見出しの費用が「載る守る側の分だけ」なので、件数ごとの表で受ける（`super::agreement_costs`）。
    pub(super) fn new(
        budget: TokenBudget,
        fixed_tokens: u32,
        agreement_costs: &[u32],
        history_costs: &[u32],
    ) -> Result<Self, PromptError> {
        let after_fixed = remaining_after_fixed(budget, fixed_tokens)?;
        // **最新の1往復を、取り決めより先に確保する**（B-59 の先取りを取り決めの前へ）。
        // 削れない枠の後の残りに入らないなら 0（長い返答1回で、取り決めの枠まで消さない）。
        let newest_turn = newest_turn_cost(history_costs)
            .filter(|cost| *cost <= after_fixed)
            .unwrap_or(0);
        // **取り決めは、最新の1往復を引いた残りから取る。** 入る中で最も多い件数（新しい側から）。
        let (agreements_kept, agreement_tokens) =
            take_agreements(agreement_costs, after_fixed.saturating_sub(newest_turn));
        // **取り決めが使った分を引いた残り**を、記憶と履歴で分ける（最新の1往復はこの中に入る）。
        let remaining = after_fixed.saturating_sub(agreement_tokens);
        // 先取りも**往復の単位で**数える。丸めで落ちる1件分を数えたままにすると、
        // その分だけ記憶の枠が理由なく狭くなる。
        let (_, within_share, _) = take_newest_turns(history_costs, history_share(remaining));
        // **取り分に1往復も入らなくても、最新の1往復は先取りする**（B-59 のレビュー）。
        // しないと長い返答1回で先取りが 0 になり、記憶が残りを全部取って履歴 0 件
        // （origin#56 の形）に戻る。**確保は上で済んでいる**（`remaining` ≥ `newest_turn`）。
        let history_first = if within_share == 0 {
            newest_turn
        } else {
            within_share
        };
        Ok(Self {
            remaining,
            memory: remaining.saturating_sub(history_first),
            agreements_kept,
            agreement_tokens,
        })
    }

    /// 記憶に回してよい枠。**想起の選別にはこれを渡す。**
    pub(super) const fn memory(self) -> u32 {
        self.memory
    }
}

/// 取り決めの件数ごとの費用の表から、`allowance` に入る中で**最も多い件数**を選ぶ。
///
/// 戻り値は `(件数, 費用)`。表が空なら `(0, 0)`。
/// **入らない件数に当たったらそこで打ち切る**（[`take_newest`] と同じ —— 表は件数とともに増える）。
fn take_agreements(costs: &[u32], allowance: u32) -> (usize, u32) {
    costs
        .iter()
        .copied()
        .enumerate()
        .take_while(|(_, cost)| *cost <= allowance)
        .last()
        .unwrap_or((0, 0))
}

/// 削れない枠を引いた**残り**。削れない枠だけで超えていたら諦める。
///
/// **[`Frames::new`] からしか呼ばない。** 外から呼べると、残りを丸ごと想起へ渡す
/// 形（origin#56 の壊れ方）に戻せてしまう。
fn remaining_after_fixed(budget: TokenBudget, fixed_tokens: u32) -> Result<u32, PromptError> {
    let allowance = budget.prompt_allowance();

    // 削れない枠だけで超えたら、押し込まずに諦める。
    if fixed_tokens > allowance {
        return Err(PromptError::BudgetExhausted);
    }
    Ok(allowance - fixed_tokens)
}

/// 予算を配分する。**文字列を見ない純粋な算術。**
///
/// `memory_costs` / `history_costs` はどちらも**古い順**に並んでいること。
///
/// 記憶と履歴の取り分は [`Frames`]（半々。片方の余りはもう片方が使う）。
///
/// # 記憶を削る枝は、いまや**後詰めの保険**である（作業単位 15b）
///
/// `super::build_prompt` は [`Frames::memory`] の枠の内側でしか
/// 記憶を選ばない（`super::recall`）ので、**本番の経路ではここで記憶が落ちない。**
/// それでも枝を残してあるのは、**この関数だけが「取り分を超えない」ことを
/// 最終的に保証する場所**だから。選別側が枠を読み違えても、
/// プロンプトが予算を超えて上流へ出ることはなく、履歴の取り分も侵されない。
///
/// `agreement_costs` は [`Frames::new`] と同じ形（新しい側から k 件の費用の表）。
pub fn plan(
    budget: TokenBudget,
    fixed_tokens: u32,
    agreement_costs: &[u32],
    memory_costs: &[u32],
    history_costs: &[u32],
) -> Result<BudgetPlan, PromptError> {
    // **余白はここで引かれ、以降どの分岐でも戻ってこない。**
    let allowance = budget.prompt_allowance();
    let frames = Frames::new(budget, fixed_tokens, agreement_costs, history_costs)?;

    // 記憶は自分の枠（残りの半分 ＋ 履歴が使わなかった分）の中で、**古い順に落とす**。
    let (memory_start, memory_tokens) = take_newest(memory_costs, frames.memory);

    // 履歴は「残り − 記憶が使った分」まで伸びる（記憶が使わなかった分を受け取る）。
    // **履歴を削る単位は往復**（ADR-0025 §2）。件数で削ると役割の並びが崩れる。
    let (history_start, history_tokens, history_aligned_away) = take_newest_turns(
        history_costs,
        frames.remaining.saturating_sub(memory_tokens),
    );

    Ok(BudgetPlan {
        memory_start,
        memory_count: memory_costs.len(),
        history_start,
        history_count: history_costs.len(),
        history_aligned_away,
        fixed_tokens,
        agreement_count: agreement_costs.len().saturating_sub(1),
        agreements_kept: frames.agreements_kept,
        agreement_tokens: frames.agreement_tokens,
        memory_frame: frames.memory,
        memory_tokens,
        history_tokens,
        response_tokens: budget.response_tokens(),
        prompt_allowance: allowance,
    })
}

/// **最新の1往復**の費用。履歴が空なら `None`、合計が溢れても `None`。
///
/// 往復の頭は偶数の添字（ADR-0025 §2）なので、最新の往復は
/// 「末尾以下で最大の偶数の添字」から末尾まで。履歴は `super::history_turns` が
/// 偶数件に揃えるので、本番では末尾の2件になる。
fn newest_turn_cost(costs: &[u32]) -> Option<u32> {
    let last = costs.len().checked_sub(1)?;
    let start = last - last % 2;
    costs
        .get(start..)?
        .iter()
        .try_fold(0_u32, |sum, cost| sum.checked_add(*cost))
}

/// 履歴を新しい側から `allowance` に収まるだけ採り、**往復の境目へ揃える。**
///
/// 戻り値は `(採用開始添字, 採用分の合計, 揃えるために追加で落とした件数)`。
/// 履歴の先取り（[`Frames::new`]）と最終の配分（`plan`）が**同じ採り方**をするための1本。
fn take_newest_turns(costs: &[u32], allowance: u32) -> (usize, u32, usize) {
    let (start, tokens) = take_newest(costs, allowance);
    align_to_a_turn(costs, start, tokens)
}

/// 履歴の採用開始を**往復の境目へ切り上げる**（ADR-0025 §2）。
///
/// 戻り値は `(採用開始添字, 採用分の合計, 揃えるために追加で落とした件数)`。
///
/// # なぜ要るか —— **本番で 502 が出た**
///
/// 履歴は `[利用者, キャラクター, 利用者, …]` の平坦な列で、[`take_newest`] は
/// **件数で**前から落とす。開始が**奇数**になると `system` の直後が `assistant` になり、
/// 上流（Ministral 系のテンプレート）が
/// `roles must alternate` を返して **502**。しかも失敗した往復では記憶が増えないので、
/// **そのセッションは自力では抜け出せない**（ADR-0025 の実測）。
///
/// # **切り上げる。切り下げないこと。**
///
/// 切り下げると採用が1件**増え**、その分の費用が予算を超える。
/// 削る枠を増やす方向は常に安全である
/// （[ADR-0013](../../docs/adr/0013-context-budget-layers.md) の「履歴は最初に削る枠」）。
///
/// **落とした1件分の費用も引く。** 引き忘れると `history_tokens` が実体より多くなり、
/// 「見積もりは常に多めに倒す」向きなので予算は破らないが、
/// 履歴が1件分ぶん余計に切り詰められ続ける。
fn align_to_a_turn(costs: &[u32], start: usize, tokens: u32) -> (usize, u32, usize) {
    if start.is_multiple_of(2) {
        return (start, tokens, 0);
    }

    match costs.get(start) {
        // 開始の1件（＝いま採っている中で最も古い1件）を落として、往復の頭に揃える。
        Some(cost) => (start + 1, tokens.saturating_sub(*cost), 1),
        // 採用が 0 件（`start == costs.len()` で件数が奇数）なら揃える相手がいない。
        None => (start, tokens, 0),
    }
}

/// 末尾（新しい側）から、予算に収まる分だけ**連続して**採る。
///
/// 戻り値は `(採用開始添字, 採用分の合計)`。
///
/// **入らない要素に当たったらそこで打ち切る。** 飛ばして更に古い小さな要素を拾うと、
/// 会話や記憶が時系列で歯抜けになり、モデルが辻褄合わせを始める。
fn take_newest(costs: &[u32], allowance: u32) -> (usize, u32) {
    let mut used: u32 = 0;
    let mut start = costs.len();

    for (index, cost) in costs.iter().enumerate().rev() {
        match used.checked_add(*cost) {
            Some(next) if next <= allowance => {
                used = next;
                start = index;
            }
            _ => break,
        }
    }

    (start, used)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// 余白 400、プロンプトに使えるのは 600 の予算。
    fn budget() -> TokenBudget {
        TokenBudget::new(1_000, 400).expect("正当な予算であること")
    }

    fn planned(fixed: u32, memories: &[u32], history: &[u32]) -> BudgetPlan {
        plan(budget(), fixed, &[], memories, history).expect("収まること")
    }

    /// **取り決めが先に取り、その残りだけを記憶と履歴が分ける**（作業単位 A-12a）。
    ///
    /// 取り分 600、削れない枠 100 → 残り 500。取り決めは3件で 120 / 240 / 360 の表なので、
    /// 3件とも入って 360 を使う。**記憶と履歴が使えるのは 140 だけ**（500 − 360）。
    /// 引かずに渡す形（残り 500 を記憶と履歴で分ける）は、合計が取り分を超えて赤になる。
    #[test]
    fn agreements_are_taken_first_and_the_rest_is_split() {
        let plan = plan(
            budget(),
            100,
            &[0, 120, 240, 360],
            &[50, 50, 50, 50],
            &[30, 30, 30, 30, 30, 30],
        )
        .expect("収まること");
        assert_eq!(plan.included_agreements(), 3);
        assert_eq!(plan.dropped_agreements(), 0);
        assert_eq!(plan.agreement_tokens(), 360);
        assert!(
            plan.memory_frame() <= 140,
            "記憶の枠は取り決めの後の残り以下: {plan:?}"
        );
        assert!(
            plan.memory_tokens() + plan.history_tokens() <= 140,
            "記憶と履歴は取り決めの後の残りに収まる: {plan:?}"
        );
        assert!(plan.prompt_tokens() <= plan.prompt_allowance(), "{plan:?}");
        assert_eq!(
            plan.prompt_tokens(),
            100 + 360 + plan.memory_tokens() + plan.history_tokens(),
            "取り決めの費用は合計に入る"
        );
    }

    /// **入りきらないときは、入る中で最も多い件数だけを載せる**（落とすのは古い側 = 表の後ろの分）。
    #[test]
    fn agreements_that_do_not_fit_are_dropped_and_counted() {
        // 残り 500 に対し、2件で 400・3件で 600。**2件まで**載り、1件落ちる。
        let plan = plan(budget(), 100, &[0, 200, 400, 600], &[], &[]).expect("収まること");
        assert_eq!(plan.included_agreements(), 2);
        assert_eq!(plan.dropped_agreements(), 1);
        assert_eq!(plan.agreement_tokens(), 400);
        // 1件も入らないなら 0 件で、組み立ては失敗しない（`BudgetExhausted` にしない）。
        let plan = plan_of(&[0, 501]);
        assert_eq!(
            (plan.included_agreements(), plan.dropped_agreements()),
            (0, 1)
        );
        assert_eq!(plan.agreement_tokens(), 0);
        // 取り決めが無い（空の表）なら 0 件。
        let plan = plan_of(&[]);
        assert_eq!(
            (plan.included_agreements(), plan.dropped_agreements()),
            (0, 0)
        );
    }

    /// **最新の1往復は、取り決めより先に確保される**（司令塔の決定 / 取る順の変更）。
    ///
    /// 残り 500。取り決めは1件 230・2件 460。**取り決めを先に取る順だと** 2件で 460 を使い、
    /// 残り 40 では最新の1往復（60）が入らず**履歴が 0 件**になる。最新の1往復を先に確保すれば、
    /// 取り決めは 440 の中から1件だけになり、履歴は1往復（2件）載る。
    #[test]
    fn the_newest_exchange_is_reserved_before_the_agreements() {
        let plan = plan(budget(), 100, &[0, 230, 460], &[], &[30, 30]).expect("収まること");
        assert_eq!(
            plan.included_history(),
            2,
            "最新の1往復が載ること: {plan:?}"
        );
        assert_eq!(
            (plan.included_agreements(), plan.dropped_agreements()),
            (1, 1)
        );
        assert!(plan.prompt_tokens() <= plan.prompt_allowance(), "{plan:?}");
        // **最新の1往復が残りに入らないなら確保しない**（取り決めの枠まで消さない）。
        let plan = plan_with_history(&[0, 230], &[300, 300]);
        assert_eq!(plan.included_history(), 0);
        assert_eq!(plan.included_agreements(), 1);
    }

    fn plan_with_history(agreement_costs: &[u32], history: &[u32]) -> BudgetPlan {
        plan(budget(), 100, agreement_costs, &[], history).expect("収まること")
    }

    fn plan_of(agreement_costs: &[u32]) -> BudgetPlan {
        plan(budget(), 100, agreement_costs, &[], &[]).expect("収まること")
    }

    /// 既定値は `new` の検証を通る（`Default` が抜け穴にならない）。
    #[test]
    fn default_is_a_valid_budget() {
        let default = TokenBudget::default();
        assert_eq!(
            TokenBudget::new(default.context_tokens(), default.response_tokens()),
            Ok(default)
        );
        assert!(default.prompt_allowance() > 0);
    }

    /// 余白 0 は作れない（「余白を必ず残す」を型の入口で守る）。
    #[test]
    fn budget_rejects_a_zero_response_room() {
        assert_eq!(TokenBudget::new(1_000, 0), Err(BudgetError::NoResponseRoom));
    }

    /// 余白がコンテキスト以上なら作れない。
    #[test]
    fn budget_rejects_a_reservation_that_leaves_no_room() {
        assert_eq!(
            TokenBudget::new(1_000, 1_000),
            Err(BudgetError::ReservationTooLarge)
        );
        assert_eq!(
            TokenBudget::new(100, 1_000),
            Err(BudgetError::ReservationTooLarge)
        );
        assert_ne!(
            BudgetError::NoResponseRoom.to_string(),
            BudgetError::ReservationTooLarge.to_string()
        );
    }

    /// プロンプトの取り分は `context - response`。
    #[test]
    fn allowance_excludes_the_response_room() {
        assert_eq!(budget().prompt_allowance(), 600);
    }

    /// 全部入るなら何も削らない。
    #[test]
    fn nothing_is_dropped_when_everything_fits() {
        let plan = planned(100, &[10, 10], &[20, 20]);

        assert_eq!(plan.dropped_memories(), 0);
        assert_eq!(plan.dropped_history(), 0);
        assert_eq!(plan.memory_tokens(), 20);
        assert_eq!(plan.history_tokens(), 40);
        assert_eq!(plan.prompt_tokens(), 160);
    }

    /// **記憶が取り分に収まるなら、削られるのは会話履歴。しかも古い順。**
    ///
    /// **16c で履歴を 4 件（往復2つ）にした。** 主張は変えていない ——
    /// 以前は 2 件のうち 1 件だけを残す形だったが、
    /// **削る単位が往復になった**（ADR-0025 §2）ので、
    /// 「新しい往復が残り、古い往復が落ちる」形で同じことを見る。
    ///
    /// > **origin#56 で名前と主張を狭めた**（旧名 `history_is_dropped_first_and_oldest_first`）。
    /// > 記憶と履歴は残りを半々に分けるので、「いつでも履歴が先」ではなくなった。
    /// > 値はそのまま通る —— 取り分 30 に最新の往復 40 は入らないが、残り 60 には入るので
    /// > その1往復を先取りする（B-59 のレビュー）。記憶の枠は 20 で、記憶 20 がちょうど入る。
    #[test]
    fn history_is_dropped_oldest_first_when_memories_fit_their_share() {
        // 余地は 600 - 540 = 60。記憶 20 を入れると履歴には 40（往復1つ分）しか残らない。
        let plan = planned(540, &[10, 10], &[20, 20, 20, 20]);

        assert_eq!(
            plan.dropped_memories(),
            0,
            "取り分に収まる記憶は削らないこと"
        );
        assert_eq!(plan.dropped_history(), 2);
        assert_eq!(plan.history_start(), 2, "古い側から落とすこと");
        assert_eq!(plan.included_history(), 2, "新しい側は残ること");
        assert_eq!(plan.history_tokens(), 40);
        assert_eq!(
            plan.history_aligned_away(),
            0,
            "この配分は丸めを使っていない"
        );
    }

    /// 最新の1往復が**残りにも**入らないなら、**残りは全部記憶が使う。**
    ///
    /// > **origin#56 で名前と主張を改めた**（旧名 `history_is_emptied_before_any_memory_is_dropped`、
    /// > 旧主張は「履歴は 0 件まで削られてから、記憶に手が付く」）。
    /// > 記憶と履歴は半々に分けるので、旧主張はもう一般には成り立たない
    /// > （履歴が取り分に入るなら、履歴が残ったまま記憶が削られる）。
    /// >
    /// > **B-59 のレビューで、もう一度狭めた**（旧名
    /// > `a_history_that_cannot_fit_its_share_leaves_it_all_to_the_memories`）。
    /// > 取り分に1往復も入らなくても、**残りに入るなら最新の1往復は載る**
    /// > （`the_newest_turn_is_kept_even_when_it_exceeds_the_history_share`）。
    /// > ここはその「残りにも入らない」側の腕 —— 最新の往復 40 は残り 20 に入らない。
    #[test]
    fn a_newest_turn_that_cannot_fit_the_remainder_leaves_it_all_to_the_memories() {
        // 余地は 600 - 580 = 20。記憶 20 でちょうど使い切る。
        let plan = planned(580, &[10, 10], &[20, 20]);

        assert_eq!(plan.included_history(), 0);
        assert_eq!(plan.dropped_history(), 2);
        assert_eq!(plan.dropped_memories(), 0);
        assert_eq!(plan.memory_frame(), 20);
        assert_eq!(plan.memory_tokens(), 20);
    }

    /// **最新の1往復が履歴の取り分を超えても、残りに入るならその1往復は載る**
    /// （B-59 のレビュー）。
    ///
    /// 取り分だけで先取りすると、長い返答1回で最新の往復が取り分を超え、
    /// **先取り 0 → 記憶が残りを全部取る → 履歴 0 件**と、origin#56 の形に戻る。
    /// 見積もりはバイト数なので、上流が 8k 級ならこの形はすぐ起きる。
    ///
    /// 残り 61、取り分 30、最新の往復 40。往復を先取りし、記憶の枠は 21（3件）。
    #[test]
    fn the_newest_turn_is_kept_even_when_it_exceeds_the_history_share() {
        let plan = planned(SPLIT_FIXED, &[7; 20], &[20, 20]);

        assert_eq!(plan.included_history(), 2, "最新の1往復が載っていない");
        assert_eq!(plan.history_tokens(), 40);
        assert_eq!(plan.memory_frame(), 21);
        assert_eq!(plan.memory_tokens(), 21);
        assert_eq!(plan.included_memories(), 3);
    }

    /// **履歴の取り分は切り捨て**（`history_share`）。
    ///
    /// 残り 63 で、取り分は切り捨てなら 31、切り上げなら 32。費用 1 の履歴は
    /// 往復の単位でしか採れないので、31 なら 30 件、32 なら 32 件になり、**件数に出る。**
    /// 記憶も費用 1 にしてあるので、記憶の枠（63 − 先取り）も件数に出る。
    #[test]
    fn the_history_share_is_rounded_down() {
        let plan = planned(537, &[1; 100], &[1; 40]);

        assert_eq!(plan.included_history(), 30, "取り分が 31 になっていない");
        assert_eq!(plan.memory_frame(), 33);
        assert_eq!(plan.included_memories(), 33);
    }

    /// **記憶は自分の枠を超えた分を、古い順に削る。**
    ///
    /// > **origin#56 で名前と主張を改めた**（旧名 `memory_is_dropped_last_and_oldest_first`、
    /// > 旧主張は「記憶を削るのは履歴を使い切った後だけ」）。
    /// > 値はそのまま通る —— 最新の往復 40 は取り分 6 にも残り 12 にも入らず、記憶の枠は 12。
    #[test]
    fn memories_beyond_their_frame_are_dropped_oldest_first() {
        // 余地は 600 - 588 = 12。記憶 20 は入らないので古い方を落とす。
        let plan = planned(588, &[10, 10], &[20, 20]);

        assert_eq!(plan.dropped_history(), 2, "履歴は取り分に1件も入らないこと");
        assert_eq!(plan.dropped_memories(), 1);
        assert_eq!(plan.memory_start(), 1, "古い側から落とすこと");
        assert_eq!(plan.memory_tokens(), 10);
        assert_eq!(plan.included_memories(), 1);
    }

    /// 記憶が1件も入らないこともある（それでも組み立ては成功する）。
    #[test]
    fn all_memories_can_be_dropped() {
        let plan = planned(595, &[10, 10], &[20]);

        assert_eq!(plan.included_memories(), 0);
        assert_eq!(plan.dropped_memories(), 2);
        assert_eq!(plan.memory_tokens(), 0);
        assert_eq!(plan.prompt_tokens(), 595);
    }

    /// **生成用の余白は絶対に侵さない。**
    #[test]
    fn the_response_room_is_never_consumed() {
        for fixed in [0_u32, 100, 300, 599, 600] {
            let plan = planned(fixed, &[50; 20], &[50; 20]);
            assert!(
                plan.prompt_tokens() <= budget().prompt_allowance(),
                "プロンプトが取り分を超えた: fixed = {fixed}, prompt = {}",
                plan.prompt_tokens()
            );
            assert!(
                plan.total_tokens() <= budget().context_tokens(),
                "合計がコンテキストを超えた: fixed = {fixed}"
            );
            assert_eq!(plan.response_tokens(), budget().response_tokens());
        }
    }

    /// 削れない枠だけで予算を超えたら**押し込まずに失敗する**。
    #[test]
    fn fixed_part_over_the_allowance_fails() {
        assert_eq!(
            plan(budget(), 601, &[], &[], &[]),
            Err(PromptError::BudgetExhausted)
        );
        assert_eq!(
            plan(budget(), 1_000, &[], &[10], &[10]),
            Err(PromptError::BudgetExhausted),
            "余白を食って押し込まないこと"
        );
        // 境界: ちょうど取り分と同じなら通る。
        assert!(plan(budget(), 600, &[], &[10], &[10]).is_ok());
    }

    /// 配分結果は**プロンプトの取り分を自分で持ち回る**。
    ///
    /// 呼び出し側から予算を渡させると、別の予算を渡す取り違えが型検査を素通りする。
    ///
    /// **`memory_dominates_the_prompt` は 15b で撤去した**（ADR-0023）。
    /// 取り分を持ち回ること自体は、`plan` が
    /// `remaining_after_fixed` と同じ予算を見ていることの観測点として残す。
    #[test]
    fn a_plan_carries_the_prompt_allowance() {
        assert_eq!(planned(0, &[], &[]).prompt_allowance(), 600);
        assert_eq!(
            planned(0, &[], &[]).prompt_allowance(),
            budget().prompt_allowance()
        );
        // 予算そのものと同値に潰れていないこと（取り分は `context - response`）。
        assert_ne!(
            planned(0, &[], &[]).prompt_allowance(),
            budget().context_tokens()
        );
    }

    /// 入らない要素の手前で打ち切る（飛ばして後ろの小さいものを拾わない）。
    ///
    /// 拾ってしまうと会話が時系列で歯抜けになり、モデルが辻褄を合わせようとして壊れる。
    #[test]
    fn selection_stays_contiguous_from_the_newest_end() {
        // 余地 600 - 560 = 40。記憶なし。履歴は新しい側から 5, 5 を採ると 10。
        // その手前の 100 は入らないので、さらに古い 1 は拾わない。
        let plan = planned(560, &[], &[1, 100, 5, 5]);

        assert_eq!(plan.history_start(), 2);
        assert_eq!(plan.history_tokens(), 10);
    }

    /// 空の入力でも成立する。
    #[test]
    fn empty_inputs_are_handled() {
        let plan = planned(0, &[], &[]);
        assert_eq!(plan.included_memories(), 0);
        assert_eq!(plan.included_history(), 0);
        assert_eq!(plan.prompt_tokens(), 0);
    }

    /// 巨大なコストでも panic しない（飽和して落ちるだけ）。
    #[test]
    fn huge_costs_do_not_panic() {
        let plan = planned(0, &[u32::MAX, u32::MAX], &[u32::MAX]);
        assert_eq!(plan.included_memories(), 0);
        assert_eq!(plan.included_history(), 0);
    }

    // ------------------------------------------------------------------
    // 往復の境目（ADR-0025 §2）
    // ------------------------------------------------------------------

    /// **履歴は往復（2件）単位でしか採らない** —— 奇数の開始は古い側へ**切り上げる。**
    ///
    /// 切り下げると採用が1件増え、**取り分を超える**
    /// （ADR-0025 §2 / ADR-0013「履歴は最初に削る枠」）。
    #[test]
    fn an_odd_history_start_is_rounded_up_to_a_turn_boundary() {
        // 余地は 600 - 570 = 30。10 の履歴 4 件のうち 3 件が収まるが、
        // 3 件目は往復の途中なので落とす。
        let plan = planned(570, &[], &[10, 10, 10, 10]);

        assert_eq!(plan.history_start(), 2, "切り下げると 1 件多く載る");
        assert_eq!(plan.included_history(), 2);
        assert_eq!(plan.dropped_history(), 2);
        assert_eq!(plan.history_tokens(), 20, "落とした 1 件分の費用も引くこと");
        assert_eq!(plan.history_aligned_away(), 1);
        assert!(
            plan.prompt_tokens() <= budget().prompt_allowance(),
            "取り分を超えた: {}",
            plan.prompt_tokens()
        );
    }

    /// 偶数の開始は動かさない（「無条件に1件落とす」実装と区別する）。
    #[test]
    fn an_even_history_start_is_left_alone() {
        // 余地は 600 - 580 = 20。ちょうど 2 件（往復1つ）が収まる。
        let plan = planned(580, &[], &[10, 10, 10, 10]);

        assert_eq!(plan.history_start(), 2);
        assert_eq!(plan.included_history(), 2);
        assert_eq!(plan.history_tokens(), 20);
        assert_eq!(plan.history_aligned_away(), 0);
    }

    /// **予算をずらしても、開始は常に偶数で、費用は採用分とぴったり合う。**
    ///
    /// # 対照
    ///
    /// `history_aligned_away` の合計が**丸めが実際に働いた回数**。
    /// **0 のまま終わったら、この掃引は往復の境目を一度もまたいでいない**ので
    /// 見張り番として無意味である（ADR-0025 §4 の警告）。
    ///
    /// # 「揃える」と「諦める」を区別する
    ///
    /// **並びと整合だけを見ると、奇数のときに履歴を丸ごと捨てる実装も通る**
    /// （実測: `align_to_a_turn` を `(costs.len(), 0, 1)` にする変異は、
    /// 開始の集合も費用の一致も破らない）。落とすのが**1件だけ**であることは、
    /// 「**余地が減る向きにしか採用件数は減らない**」で押さえる。
    #[test]
    fn the_history_start_stays_on_a_turn_boundary_across_a_sweep() {
        // **費用は全部違う値にする。** 同値だと「落とした1件分を引いたか」が見えない。
        let costs = [7_u32, 11, 13, 5, 9, 3];
        let mut aligned = 0;
        let mut starts = BTreeSet::new();
        // 余地が最も広い（`fixed = 0`）ところから始めるので、採用件数は単調に減る。
        let mut previous = costs.len();

        for fixed in 0..=budget().prompt_allowance() {
            let plan = planned(fixed, &[], &costs);

            assert!(
                plan.included_history() <= previous,
                "余地を減らしたのに採用が増えた: fixed = {fixed}, {} → {}",
                previous,
                plan.included_history()
            );
            previous = plan.included_history();

            assert!(
                plan.history_start().is_multiple_of(2),
                "奇数の開始が残った: fixed = {fixed}, start = {}",
                plan.history_start()
            );
            let included: u32 = costs[plan.history_start()..].iter().sum();
            assert_eq!(
                plan.history_tokens(),
                included,
                "費用が採用分と食い違った: fixed = {fixed}"
            );
            assert_eq!(
                plan.included_history(),
                costs.len() - plan.history_start(),
                "採用件数が開始と食い違った: fixed = {fixed}"
            );
            assert_eq!(plan.dropped_history(), plan.history_start());
            assert!(
                plan.prompt_tokens() <= budget().prompt_allowance(),
                "取り分を超えた: fixed = {fixed}, prompt = {}",
                plan.prompt_tokens()
            );

            aligned += plan.history_aligned_away();
            starts.insert(plan.history_start());
        }

        assert!(
            aligned > 0,
            "対照: 丸めが一度も働いていない掃引は見張り番にならない"
        );
        assert_eq!(
            starts,
            BTreeSet::from([0, 2, 4, 6]),
            "掃引が削り位置を端から端まで動かしていない"
        );
    }

    /// 履歴が奇数件で、しかも1件も入らないときでも panic しない。
    ///
    /// 揃える相手（`costs[start]`）が無い形なので、添字の +1 が範囲外になりうる。
    ///
    /// # ここが**開始が奇数のまま出てくる唯一の口**
    ///
    /// `align_to_a_turn` の `None` 枝は `history_start` を奇数のまま返す。
    /// **いま安全なのは、`super::super::history_turns` が偶数件しか作らないから**であって、
    /// この関数が奇数を弾いているからではない。**その結び付きは散文にしかない**ので、
    /// 「奇数がそのまま出る」ことを値で固定しておく
    /// （採用は 0 件なので、外に出る `messages` の並びには影響しない）。
    #[test]
    fn an_odd_history_length_does_not_panic() {
        // 余地は 600 - 600 = 0。1 件も入らないので開始は末尾（= 3、奇数）。
        let plan = planned(600, &[], &[10, 10, 10]);

        assert_eq!(plan.history_start(), 3, "揃える相手がいないので奇数のまま");
        assert_eq!(plan.included_history(), 0);
        assert_eq!(plan.history_tokens(), 0);
        assert_eq!(plan.history_aligned_away(), 0);
    }

    // ------------------------------------------------------------------
    // 記憶と履歴の取り分（origin#56）
    //
    // **残りは奇数（61）にしてある。** 履歴の取り分は切り捨てで 30、記憶は 31 で、
    // 2つの取り分が同じ値にならない（同値だと取り違えを観測できない）。
    // ------------------------------------------------------------------

    /// 余地が 61 になる削れない枠。
    const SPLIT_FIXED: u32 = 539;

    /// **記憶が残りを全部埋められるほど多くても、履歴は1往復以上載る**（origin#56）。
    ///
    /// 以前は記憶が先に残りを全部取り、履歴が 0 件になっていた
    /// （弥生の DB で 30 ターンすべて `kept=0`）。
    ///
    /// 履歴の先取りは 30 のうち 24（4件。5件目は往復の途中なので落とす）。
    /// 記憶は 31 ＋ 履歴が使わなかった 6 ＝ 37 の中で 5件（35）。
    #[test]
    fn memories_do_not_crowd_the_history_out() {
        let plan = planned(SPLIT_FIXED, &[7; 20], &[6; 10]);

        assert!(plan.included_history() >= 2, "履歴が1往復も載っていない");
        assert_eq!(plan.history_tokens(), 24);
        assert_eq!(plan.memory_frame(), 37);
        assert_eq!(plan.memory_tokens(), 35);
        assert!(
            plan.history_tokens() <= 61 / 2,
            "履歴が自分の取り分を超えた: {}",
            plan.history_tokens()
        );
        assert!(
            plan.memory_tokens() <= 61 - plan.history_tokens(),
            "記憶が履歴の使った分に食い込んだ: {}",
            plan.memory_tokens()
        );
        assert_eq!(plan.memory_start(), 15, "記憶は古い側から落とすこと");
        assert_eq!(plan.history_start(), 6, "履歴は古い側から落とすこと");
    }

    /// **履歴が取り分を使い切らなければ、その余りは記憶が使う。**
    ///
    /// 履歴は 10 しか使わないので、記憶は半分（31）を超えて 51 まで使える。
    #[test]
    fn the_history_leaves_its_unused_share_to_the_memories() {
        let plan = planned(SPLIT_FIXED, &[7; 20], &[5, 5]);

        assert_eq!(plan.included_history(), 2);
        assert_eq!(plan.history_tokens(), 10);
        assert_eq!(plan.memory_frame(), 51);
        assert_eq!(plan.memory_tokens(), 49, "履歴の余りが記憶へ回っていない");
        assert!(
            plan.memory_tokens() > 61 - 61 / 2,
            "記憶が半分の内側に留まった"
        );
    }

    /// **記憶が取り分を使い切らなければ、その余りは履歴が使う。**
    ///
    /// 記憶は 7 しか使わないので、履歴は半分（30）を超えて 48 まで使える。
    #[test]
    fn the_memories_leave_their_unused_share_to_the_history() {
        let plan = planned(SPLIT_FIXED, &[3, 4], &[6; 20]);

        assert_eq!(plan.included_memories(), 2);
        assert_eq!(plan.memory_tokens(), 7);
        assert_eq!(plan.memory_frame(), 37, "記憶の枠は履歴の先取り 24 の残り");
        assert_eq!(plan.history_tokens(), 48, "記憶の余りが履歴へ回っていない");
        assert!(plan.history_tokens() > 61 / 2, "履歴が半分の内側に留まった");
        assert_eq!(plan.history_start(), 12);
    }
}
