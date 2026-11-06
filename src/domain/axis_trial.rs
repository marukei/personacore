//! **試用中の候補を、数で合格・不合格に振り分ける規則**
//! （[ADR-0046](../../docs/adr/0046-drift-proposes-new-axes.md) §決定3・§決定4 / 作業単位 A-9c / Issue origin#43）。
//!
//! 候補の形と状態（`domain::axis_candidate`）の上に、**試用の判断**だけを置く。
//! どれも**純粋関数**で、DB も上流も触らない（呼ぶのは `server::axis_trial`）。
//!
//! | 型・関数 | 何か |
//! |---|---|
//! | [`AxisTrialRule`] | **`T`（試用のターン数）・`K`（標本不足の線）・線（割合の閾値）**。**測定の器具** |
//! | [`judge_axis_trial`] | 票の内訳から、**判定するか・何と判定するか**を決める |
//! | [`AxisTrialVerdict`] | 判定の結果（まだ・合格・不合格の理由） |
//! | [`trial_vote`] | 採点（`+1` / `-1` / `0`）を試用の票の種類へ写す |
//! | [`candidate_to_score`] | **どの候補を採点するか**（試用中の候補の輪番。鍵はそのターンの応答の `message.id`） |
//!
//! # 判定の表（ADR-0046 §決定4）と、**上から順に見る**こと
//!
//! 試用の票が **`T` 件以上**そろったら判定する。**上の行から順に見て、最初に当たった行**で決まる。
//!
//! | 順 | 判定 | 条件 | 行き先 |
//! |---|---|---|---|
//! | 1 | **沈黙** | `0` の率 ＞ 線 | 不合格（`silent`） |
//! | 2 | **標本不足** | 非ゼロの票 ＜ `K` | 不合格（`too_few_samples`） |
//! | 3 | **張り付き** | 非ゼロの票のうち上げの割合 ＞ 線 | 不合格（`pinned`） |
//! | 4 | それ以外 | | **合格**（`passed`） |
//!
//! ## なぜ張り付きを最後に見るのか（ADR の表の並びと違う）
//!
//! ADR-0046 の表は張り付き・沈黙・標本不足の順に並んでいるが、**表は条件の一覧であって
//! 評価の順序を決めていない。** 1つの内訳が2つの行に当たることがある（例: 上げ 5・`0` が 55
//! —— 上げの割合 100% かつ `0` の率 92%）。そのときの答えを決めておかないと、
//! §測る問い の「不合格の内訳」が実装の書き順で変わる。
//!
//! **上げの割合は、非ゼロの票が足りるときだけ読む**（ADR-0030 §実測3 の宿題
//! 「非ゼロが N 件未満なら割合を読まない」）。だから割合を読む張り付きを、
//! 標本を数える2つ（沈黙・標本不足）の後に置いた。**非ゼロが 5 件しかない軸を
//! 「張り付き」と呼ばない**（ADR-0030 の `短気` / `皮肉` の形）。
//!
//! 沈黙を標本不足より先に置くのは、**沈黙の方が強い記述**だから
//! （「ほとんど採点されない」は「非ゼロが足りない」の特別な場合）。
//! ADR-0046 は標本不足を「沈黙の側に数える」としており、どちらも沈黙の側の理由である。
//!
//! ## 境目（ちょうど線）は**合格の側**
//!
//! ADR-0046 §決定4 と Issue origin#43 は「90% を**超える**」と書いている。**ちょうど 90% は不合格にしない。**
//! 比較は整数の掛け算で行う（`上げ × 100 > 線 × 非ゼロ`）ので、浮動小数の丸めで境目が揺れない。
//!
//! > **⚠ ADR-0030 §決定1 の表は「≥ 90%」と書いている**（こちらは手で回した手順の目安）。
//! > **この実装は ADR-0046 と Issue origin#43 の「超える」に従う。** 違いが効くのは率がちょうど線の回だけで、
//! > ADR-0030 の実測では `短気`（`0` の率 90% / 非ゼロ 6）がそれに当たる —— この規則なら
//! > 沈黙ではなく**標本不足**（既定の `K` = 10 に届かない）で落ちる。
//!
//! # ⚠ 既定の `T` / `K` / 線は**測っていない値**
//!
//! [`AxisTrialRule::DEFAULT`] の 60 / 10 / 90 のうち、**`T` = 60 と線 90% は ADR-0030 が
//! 手で回した手順の値**（そこでも線は「実測の分布の隙間に置いただけ」）で、**候補の分布では測っていない**
//! （ADR-0046 §決定4 の注記）。**`K` = 10 には根拠が無い** —— ADR-0030 の実測で
//! 非ゼロ 5〜6 件の軸を「割合に意味が無い」、12 件の軸を読める側に置いていたので、その間に置いた。
//! **§測る問い を回すための器具**として環境変数で振れる形にしてある（`server::config`）。
//! 外したときに起きるのは「合格・不合格の振り分けが変わる」だけで、壊れはしない
//! （採用は利用者だけが行う / ADR-0046 §決定5）。

use std::fmt;

use super::axis_candidate::{
    AxisCandidateId, AxisCandidateState, AxisTrialFailure, AxisTrialTally, AxisTrialVote,
};
use super::drift::DriftDelta;
use super::message::MessageId;

/// **試用の判定のつまみ**（ADR-0046 §決定4）——`T`・`K`・線。
///
/// **3つを別々の値で持ち回らない**（`DriftTuning` と同じ判断）。別々にすると
/// 「`T` だけ検証されている」状態が作れるうえ、渡すときに順番を取り違えても型が通る。
/// **[`Self::new`] が唯一のコンストラクタ**（`From` も `pub` の欄も生やさない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AxisTrialRule {
    turns: u32,
    min_nonzero: u32,
    line_percent: u32,
}

impl AxisTrialRule {
    /// 既定の `T`（**ADR-0030 の手順の値。候補では測っていない**）。
    pub const DEFAULT_TURNS: u32 = 60;

    /// 既定の `K`（**根拠の無い値**。モジュールの doc）。
    pub const DEFAULT_MIN_NONZERO: u32 = 10;

    /// 既定の線（百分率。**ADR-0030 の手順の値。候補では測っていない**）。
    pub const DEFAULT_LINE_PERCENT: u32 = 90;

    /// **既定。** 3つとも測っていない値（モジュールの doc）。
    pub const DEFAULT: Self = Self {
        turns: Self::DEFAULT_TURNS,
        min_nonzero: Self::DEFAULT_MIN_NONZERO,
        line_percent: Self::DEFAULT_LINE_PERCENT,
    };

    /// 唯一のコンストラクタ。
    ///
    /// | 引数 | 受け付ける範囲 | 外れたら |
    /// |---|---|---|
    /// | `turns`（`T`） | 1 以上 | [`AxisTrialRuleError::Turns`] |
    /// | `min_nonzero`（`K`） | 0 以上 `T` 以下 | [`AxisTrialRuleError::MinNonzeroAboveTurns`]（**組の不整合** —— どちらを直してもよい） |
    /// | `line_percent`（線） | 1〜99 | [`AxisTrialRuleError::Line`] |
    ///
    /// * **`T` = 0 を取らない** —— 票が1件も無いうちに判定する（`0` の率が決まらない）。
    /// * **`K` > `T` を取らない** —— 非ゼロの票は `T` を超えないので、**沈黙しない候補が全部
    ///   標本不足で落ちる**。測定の器具として意味を成さない設定を、起動の時点で止める。
    /// * **線 0 と 100 を取らない** —— 100 では張り付きも沈黙も起こりえず（率は 100% を超えない）、
    ///   0 では上げが1票でもあれば張り付きになる。どちらも判定を捨てるのと同じ。
    ///
    /// # Errors
    ///
    /// 上の表のとおり。
    pub const fn new(
        turns: u32,
        min_nonzero: u32,
        line_percent: u32,
    ) -> Result<Self, AxisTrialRuleError> {
        if turns == 0 {
            return Err(AxisTrialRuleError::Turns);
        }
        if min_nonzero > turns {
            return Err(AxisTrialRuleError::MinNonzeroAboveTurns);
        }
        if line_percent == 0 || line_percent >= 100 {
            return Err(AxisTrialRuleError::Line);
        }
        Ok(Self {
            turns,
            min_nonzero,
            line_percent,
        })
    }

    /// `T` —— 試用の票が何件そろったら判定するか。
    #[must_use]
    pub const fn turns(self) -> u32 {
        self.turns
    }

    /// `K` —— 非ゼロの票がこれに届かなければ標本不足。
    #[must_use]
    pub const fn min_nonzero(self) -> u32 {
        self.min_nonzero
    }

    /// 線（百分率）—— 率がこれを**超えたら**不合格。
    #[must_use]
    pub const fn line_percent(self) -> u32 {
        self.line_percent
    }
}

impl Default for AxisTrialRule {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// [`AxisTrialRule::new`] が受け付けなかったつまみ。**`Display` は定数文字列**（値を反射しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AxisTrialRuleError {
    /// `T` が読めない、または 0。
    #[error("軸の候補の試用のターン数の指定が不正です")]
    Turns,
    /// `K` が読めない。
    #[error("軸の候補の試用の標本不足の線の指定が不正です")]
    MinNonzero,
    /// **`K` が `T` を超える（組の不整合）。** 片方だけ設定した回にも起きる
    /// （`T` = 9 だけを設定すると、既定の `K` = 10 との組でここに来る）。
    ///
    /// [`Self::MinNonzero`] と分けるのは、**触っていない方を疑わせない**ため（A-9c のレビュー提案4）。
    /// **文言は両方のつまみを名指しし、値は載せない**（値はログの欄 —— `server::config` の `axis_trial_rule`）。
    #[error(
        "軸の候補の試用の標本不足の線（K）が試用のターン数（T）を超えています。K を T 以下にしてください"
    )]
    MinNonzeroAboveTurns,
    /// 線が読めない、または 1〜99 の外。
    #[error("軸の候補の試用の判定の線の指定が不正です")]
    Line,
}

/// **判定の結果。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisTrialVerdict {
    /// まだ `T` 件そろっていない。**試用を続ける。**
    Pending,
    /// 合格（利用者への提案 —— 画面は第2段）。
    Passed,
    /// 不合格（理由つき）。**枠を空ける。**
    Failed(AxisTrialFailure),
}

impl AxisTrialVerdict {
    /// **進める先の状態。** まだなら `None`。
    #[must_use]
    pub const fn next_state(self) -> Option<AxisCandidateState> {
        match self {
            Self::Pending => None,
            Self::Passed => Some(AxisCandidateState::Passed),
            Self::Failed(reason) => Some(AxisCandidateState::Failed(reason)),
        }
    }

    /// **ログの綴り。** 不合格は理由の綴り（`AxisTrialFailure::label`）をそのまま使う。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Passed => "passed",
            Self::Failed(reason) => reason.label(),
        }
    }
}

impl fmt::Display for AxisTrialVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// **票の内訳から判定する**（ADR-0046 §決定4）。**純粋関数。panic しない。**
///
/// 規則と順序はモジュールの doc の表。票の合計が `T` に届かなければ [`AxisTrialVerdict::Pending`]。
/// **`T` を超えていても判定する**（判定を書けなかった回の後や、`T` を下げて起こし直した後）。
///
/// 率の比較は `u64` の整数の掛け算で行う（溢れない・丸めない）。
#[must_use]
pub fn judge_axis_trial(rule: AxisTrialRule, tally: AxisTrialTally) -> AxisTrialVerdict {
    let up = u64::from(tally.up);
    let nonzero = up + u64::from(tally.down);
    let zero = u64::from(tally.zero);
    let total = nonzero + zero;
    if total < u64::from(rule.turns) {
        return AxisTrialVerdict::Pending;
    }
    let line = u64::from(rule.line_percent);
    // **沈黙を先に見る**（非ゼロが 0 件なら、ここで必ず当たる —— 割り算は無い）。
    if zero * 100 > line * total {
        return AxisTrialVerdict::Failed(AxisTrialFailure::Silent);
    }
    if nonzero < u64::from(rule.min_nonzero) {
        return AxisTrialVerdict::Failed(AxisTrialFailure::TooFewSamples);
    }
    if up * 100 > line * nonzero {
        return AxisTrialVerdict::Failed(AxisTrialFailure::Pinned);
    }
    AxisTrialVerdict::Passed
}

/// **採点を試用の票の種類へ写す。** `None`（モデルが `0` と書いた）は [`AxisTrialVote::Zero`]。
///
/// **「行が無かった」は `0` ではない**（ADR-0027 §12）——それは分解器が捨てるので、
/// ここへは来ない（呼び出し側は読めた採点だけを渡す）。
#[must_use]
pub const fn trial_vote(delta: Option<DriftDelta>) -> AxisTrialVote {
    match delta {
        Some(DriftDelta::Up) => AxisTrialVote::Up,
        Some(DriftDelta::Down) => AxisTrialVote::Down,
        None => AxisTrialVote::Zero,
    }
}

/// **どの候補を採点するか** —— 試用中の候補の**輪番**。鍵はそのターンの応答の `message.id`。**純粋関数。**
///
/// 試用中の候補を id の昇順に並べ、`mix(turn) mod 件数` 番目を選ぶ。試用中が無ければ `None`。
///
/// # 試用は1ターンに1つ（ADR-0046「まだ決めていないこと」への答え）
///
/// 枠が 2 以上あると、試用中の候補が同時に2つ以上ありうる。**1回の依頼で並べて採点しない**
/// —— CLAUDE.md「複数の項目を1回で採点させると、項目は独立に判定されない」（固定の軸と
/// 別の依頼にした ADR-0046 §決定3 と同じ理由）。**候補ごとに1回ずつ依頼もしない** ——
/// 裏方の呼び出しが「試用中だけ1ターンに1回増える」（§決定3 の代償）を超える。
///
/// # なぜ輪番なのか（A-9c のレビュー / 司令塔の決定）
///
/// **最初の版は「最も古い1つ」だった。** 読める答えが一度も返らない候補（モデルが名前を写せない等）が
/// 最も古いと、**その候補が毎ターン選ばれて票が入らず、後ろの候補には票が1件も入らない** ——
/// そのキャラクターの試用全体が止まり、枠が埋まったままなので、やがて候補づくりも止まる。
/// 輪番なら、**止まった1件が食うのはターンのおよそ 1/k だけ**（k = 試用中の件数）で、他の候補は進む。
///
/// # 鍵は「毎ターン必ず変わる値」でなければならない
///
/// **状態を持たない**（どこまで回したかを保存しない）ので、鍵は外から来る値になる。
/// **票の件数は鍵にできない** —— 止まった候補を選んだ回は票が増えないので、鍵が進まず同じ候補を選び続ける。
/// 応答の `message.id` は往復ごとに必ず新しくなる。
///
/// # なぜ素の余り（`id mod k`）にしないのか
///
/// 応答の id は**1往復に2ずつ進む**（利用者の発言と応答の2行 / `Store::append_turn`）。
/// `k` = 2 なら偶奇が毎回同じになり、**素の余りは同じ候補を選び続ける**。2体のキャラクターが交互に
/// 話せば4ずつ進み、裏方が1ターンおきに `Busy` で飛べば、さらに歩幅が揃う。
/// **歩幅が揃うと輪番は回らない**ので、id を混ぜて（splitmix64 の仕上げ）から割る。
/// 帰結として並びは厳密な順番ではなく、**各ターンでおよそ 1/k の確率で各候補が選ばれる**形になる
/// （見張り番 `the_rotation_reaches_every_candidate_whatever_the_stride`）。
///
/// # ⚠ 代償: 止まった候補は枠と 1/k のターンを食い続ける
///
/// 止まった候補は判定されないので、**枠を1つ埋めたまま、以後ずっと試用のターンのおよそ 1/k を使い、
/// そのぶん裏方の呼び出しを空費する**（他の候補の試用は k/(k−1) 倍ほど遅くなる）。
/// **止まった候補が1つだけで枠も1つなら、そのキャラクターの試用と候補づくりは止まる**（輪番でも救えない）。
/// 打ち切る規則（崩れが続いたら不合格）は置いていない —— 不合格の理由は3つ（`0015` の CHECK）しか無く、
/// 新しい理由には移行が要る。崩れた回はログの `axis_trial_outcome` に理由つきで残る。
#[must_use]
pub fn candidate_to_score(
    candidates: impl IntoIterator<Item = (AxisCandidateId, AxisCandidateState)>,
    turn: MessageId,
) -> Option<AxisCandidateId> {
    let mut on_trial: Vec<AxisCandidateId> = candidates
        .into_iter()
        .filter(|(_, state)| *state == AxisCandidateState::Trial)
        .map(|(id, _)| id)
        .collect();
    if on_trial.is_empty() {
        return None;
    }
    // **並びは id の昇順にそろえる**（呼び出し側の並び —— 一覧は新しい順 —— に寄りかからない）。
    on_trial.sort_unstable();
    let count = u64::try_from(on_trial.len()).unwrap_or(u64::MAX);
    let at = usize::try_from(mix(u64::from_ne_bytes(turn.get().to_ne_bytes())) % count).ok()?;
    on_trial.get(at).copied()
}

/// **鍵を混ぜる**（splitmix64 の仕上げ）。歩幅の揃った鍵の列を、余りが偏らない列に変える。
const fn mix(key: u64) -> u64 {
    let mut z = key.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use AxisTrialFailure::{Pinned, Silent, TooFewSamples};

    fn rule(turns: u32, min_nonzero: u32, line: u32) -> AxisTrialRule {
        AxisTrialRule::new(turns, min_nonzero, line).expect("正当なつまみ")
    }

    fn tally(up: u32, down: u32, zero: u32) -> AxisTrialTally {
        AxisTrialTally { up, down, zero }
    }

    // ---------------------------------------------------------------------
    // つまみ
    // ---------------------------------------------------------------------

    /// **既定は 60 / 10 / 90**（外の値で押さえる。定数から期待値を作らない）。
    #[test]
    fn the_default_rule_is_sixty_turns_ten_samples_and_ninety_percent() {
        let default = AxisTrialRule::DEFAULT;
        assert_eq!(
            (
                default.turns(),
                default.min_nonzero(),
                default.line_percent()
            ),
            (60, 10, 90)
        );
        assert_eq!(AxisTrialRule::default(), default);
        assert_eq!(AxisTrialRule::new(60, 10, 90), Ok(default));
    }

    /// **範囲の両端を、通る側と拒む側の両方で。**
    #[test]
    fn the_rule_refuses_values_that_would_throw_the_judgement_away() {
        for (turns, min_nonzero, line) in [(1, 0, 1), (1, 1, 99), (60, 60, 50), (60, 0, 90)] {
            let built = AxisTrialRule::new(turns, min_nonzero, line).expect("通ること");
            assert_eq!(
                (built.turns(), built.min_nonzero(), built.line_percent()),
                (turns, min_nonzero, line)
            );
        }
        for (turns, min_nonzero, line, expected) in [
            (0, 0, 90, AxisTrialRuleError::Turns),
            (60, 61, 90, AxisTrialRuleError::MinNonzeroAboveTurns),
            (1, 2, 90, AxisTrialRuleError::MinNonzeroAboveTurns),
            (60, 10, 0, AxisTrialRuleError::Line),
            (60, 10, 100, AxisTrialRuleError::Line),
            (60, 10, u32::MAX, AxisTrialRuleError::Line),
        ] {
            assert_eq!(
                AxisTrialRule::new(turns, min_nonzero, line),
                Err(expected),
                "({turns}, {min_nonzero}, {line})"
            );
        }
    }

    // ---------------------------------------------------------------------
    // 判定の表（境目の両側）
    // ---------------------------------------------------------------------

    /// **`T` に届くまでは判定しない。届いた回に判定する。超えても判定する。**
    #[test]
    fn nothing_is_judged_before_t_votes() {
        let rule = rule(10, 2, 90);
        assert_eq!(
            judge_axis_trial(rule, tally(3, 3, 3)),
            AxisTrialVerdict::Pending
        );
        assert_eq!(
            judge_axis_trial(rule, tally(0, 0, 0)),
            AxisTrialVerdict::Pending
        );
        assert_eq!(
            judge_axis_trial(rule, tally(4, 3, 3)),
            AxisTrialVerdict::Passed
        );
        assert_eq!(
            judge_axis_trial(rule, tally(5, 3, 3)),
            AxisTrialVerdict::Passed,
            "T を超えていても判定する"
        );
    }

    /// **張り付き: 上げの割合がちょうど線なら合格、1票でも超えれば不合格。**
    ///
    /// 線 90%・非ゼロ 20（上げ 18 / 下げ 2 = ちょうど 90%）と、上げ 19 / 下げ 1（95%）。
    /// **非ゼロが `K` 以上**で、`0` の率が線より低い内訳にしてある（他の行に当てない）。
    #[test]
    fn pinned_is_strictly_above_the_line() {
        let rule = rule(40, 10, 90);
        assert_eq!(
            judge_axis_trial(rule, tally(18, 2, 20)),
            AxisTrialVerdict::Passed,
            "ちょうど 90% は合格"
        );
        assert_eq!(
            judge_axis_trial(rule, tally(19, 1, 20)),
            AxisTrialVerdict::Failed(Pinned),
            "90% を超えたら張り付き"
        );
        // 下げばかりは張り付きではない（上げの割合を見る）。
        assert_eq!(
            judge_axis_trial(rule, tally(0, 20, 20)),
            AxisTrialVerdict::Passed
        );
    }

    /// **沈黙: `0` の率がちょうど線なら沈黙ではなく、超えれば沈黙。**
    ///
    /// 60 票・線 90%: `0` が 54（ちょうど 90%）なら沈黙ではない —— 非ゼロ 6 は `K` = 6 に届き、
    /// 上げ 3 / 下げ 3 なので合格。`0` が 55（91.7%）なら沈黙。
    #[test]
    fn silent_is_strictly_above_the_line() {
        let rule = rule(60, 5, 90);
        assert_eq!(
            judge_axis_trial(rule, tally(3, 3, 54)),
            AxisTrialVerdict::Passed,
            "ちょうど 90% は沈黙ではない"
        );
        assert_eq!(
            judge_axis_trial(rule, tally(3, 2, 55)),
            AxisTrialVerdict::Failed(Silent),
            "90% を超えたら沈黙"
        );
        assert_eq!(
            judge_axis_trial(rule, tally(0, 0, 60)),
            AxisTrialVerdict::Failed(Silent),
            "全部 0 は沈黙（割り算をしない）"
        );
    }

    /// **標本不足: 非ゼロがちょうど `K` なら足りていて、1つ少なければ標本不足。**
    #[test]
    fn too_few_samples_is_strictly_below_k() {
        let rule = rule(40, 10, 90);
        assert_eq!(
            judge_axis_trial(rule, tally(5, 5, 30)),
            AxisTrialVerdict::Passed,
            "ちょうど K 件は足りている"
        );
        assert_eq!(
            judge_axis_trial(rule, tally(5, 4, 31)),
            AxisTrialVerdict::Failed(TooFewSamples),
            "K − 1 件は標本不足"
        );
    }

    /// **2つの行に当たる内訳は、表の順（沈黙 → 標本不足 → 張り付き）で決まる。**
    ///
    /// * 上げ 5・`0` 55 —— 上げ 100% かつ `0` の率 91.7% → **沈黙**（張り付きと呼ばない）
    /// * 上げ 8・`0` 52（K = 10）—— 上げ 100% かつ非ゼロ 8 → **標本不足**（割合を読まない）
    #[test]
    fn an_overlap_is_decided_by_the_order_of_the_table() {
        let rule = rule(60, 10, 90);
        assert_eq!(
            judge_axis_trial(rule, tally(5, 0, 55)),
            AxisTrialVerdict::Failed(Silent)
        );
        assert_eq!(
            judge_axis_trial(rule, tally(8, 0, 52)),
            AxisTrialVerdict::Failed(TooFewSamples)
        );
        assert_eq!(
            judge_axis_trial(rule, tally(12, 0, 48)),
            AxisTrialVerdict::Failed(Pinned),
            "非ゼロが K 以上なら割合を読む"
        );
    }

    /// **線と `K` は既定でない値でも効く**（写しの定数に寄りかかっていない）。
    #[test]
    fn the_line_and_k_are_read_from_the_rule() {
        // 上げ 75%: 線 70 なら張り付き、線 80 なら合格。
        assert_eq!(
            judge_axis_trial(rule(4, 1, 70), tally(3, 1, 0)),
            AxisTrialVerdict::Failed(Pinned)
        );
        assert_eq!(
            judge_axis_trial(rule(4, 1, 80), tally(3, 1, 0)),
            AxisTrialVerdict::Passed
        );
        // 非ゼロ 2: K = 2 なら合格、K = 3 なら標本不足。
        assert_eq!(
            judge_axis_trial(rule(4, 2, 90), tally(1, 1, 2)),
            AxisTrialVerdict::Passed
        );
        assert_eq!(
            judge_axis_trial(rule(4, 3, 90), tally(1, 1, 2)),
            AxisTrialVerdict::Failed(TooFewSamples)
        );
    }

    /// **件数が `u32` の端でも溢れずに判定する。**
    #[test]
    fn huge_tallies_never_overflow() {
        let rule = rule(60, 10, 90);
        assert_eq!(
            judge_axis_trial(rule, tally(u32::MAX, 0, 0)),
            AxisTrialVerdict::Failed(Pinned)
        );
        assert_eq!(
            judge_axis_trial(rule, tally(u32::MAX, u32::MAX, u32::MAX)),
            AxisTrialVerdict::Passed
        );
    }

    // ---------------------------------------------------------------------
    // 行き先と綴り
    // ---------------------------------------------------------------------

    /// **判定の行き先と綴り**（外の値で押さえる）。
    #[test]
    fn every_verdict_has_its_next_state_and_spelling() {
        let all = [
            AxisTrialVerdict::Pending,
            AxisTrialVerdict::Passed,
            AxisTrialVerdict::Failed(Pinned),
            AxisTrialVerdict::Failed(Silent),
            AxisTrialVerdict::Failed(TooFewSamples),
        ];
        let spelled: Vec<(&str, Option<AxisCandidateState>)> = all
            .iter()
            .map(|verdict| (verdict.label(), verdict.next_state()))
            .collect();
        assert_eq!(
            spelled,
            vec![
                ("pending", None),
                ("passed", Some(AxisCandidateState::Passed)),
                ("pinned", Some(AxisCandidateState::Failed(Pinned))),
                ("silent", Some(AxisCandidateState::Failed(Silent))),
                (
                    "too_few_samples",
                    Some(AxisCandidateState::Failed(TooFewSamples))
                ),
            ]
        );
        for verdict in all {
            if let Some(next) = verdict.next_state() {
                assert!(
                    AxisCandidateState::Trial.can_become(next),
                    "{verdict:?} の行き先は試用から進める状態であること"
                );
            }
        }
        assert_eq!(AxisTrialVerdict::Passed.to_string(), "passed");
    }

    /// **採点の写し: `+1` → 上げ・`-1` → 下げ・`0` → `0`。**
    #[test]
    fn a_score_maps_to_its_own_kind_of_trial_vote() {
        assert_eq!(trial_vote(Some(DriftDelta::Up)), AxisTrialVote::Up);
        assert_eq!(trial_vote(Some(DriftDelta::Down)), AxisTrialVote::Down);
        assert_eq!(trial_vote(None), AxisTrialVote::Zero);
    }

    /// **試用中でない候補は選ばない。試用中が1件なら常にそれ、無ければ `None`。同じ鍵なら同じ答え。**
    #[test]
    fn only_a_candidate_on_trial_is_chosen() {
        let id = AxisCandidateId::from_row;
        for key in 1..=50 {
            let turn = MessageId::from_row(key);
            assert_eq!(
                candidate_to_score(
                    [
                        (id(9), AxisCandidateState::Passed),
                        (id(5), AxisCandidateState::Trial),
                        (id(2), AxisCandidateState::Failed(Silent)),
                    ],
                    turn
                ),
                Some(id(5)),
                "鍵 {key}"
            );
            assert_eq!(
                candidate_to_score(
                    [
                        (id(3), AxisCandidateState::Passed),
                        (id(1), AxisCandidateState::Adopted),
                    ],
                    turn
                ),
                None
            );
        }
        assert_eq!(candidate_to_score([], MessageId::from_row(4)), None);
        let listed = [
            (id(9), AxisCandidateState::Trial),
            (id(4), AxisCandidateState::Trial),
            (id(6), AxisCandidateState::Trial),
        ];
        for key in 1..=20 {
            assert_eq!(
                candidate_to_score(listed, MessageId::from_row(key)),
                candidate_to_score(listed, MessageId::from_row(key)),
                "状態を持たない（同じ鍵なら同じ答え）"
            );
        }
    }

    /// **歩幅が揃った鍵の列でも、輪番はどの候補にも回る**（素の余りにしない理由 / 関数の doc）。
    ///
    /// 3件（真ん中を含む）と2件で、歩幅 1・2（1往復の id の進み）・4（2体が交互に話す）・6 の
    /// 60 ターンを回し、**どの候補も 60 / k の半分以上**選ばれることを見る。
    /// 素の余りだと、2件・歩幅 2 で片方が 0 回になる。**並びは渡した順（新しい順）に寄りかからない**
    /// （id の昇順で数える）。
    #[test]
    fn the_rotation_reaches_every_candidate_whatever_the_stride() {
        let id = AxisCandidateId::from_row;
        for listed in [
            vec![
                (id(8), AxisCandidateState::Trial),
                (id(3), AxisCandidateState::Trial),
            ],
            vec![
                (id(8), AxisCandidateState::Trial),
                (id(5), AxisCandidateState::Trial),
                (id(3), AxisCandidateState::Trial),
            ],
        ] {
            let k = listed.len();
            for stride in [1, 2, 4, 6] {
                let mut counts = std::collections::BTreeMap::new();
                for turn in 1..=60 {
                    let chosen = candidate_to_score(
                        listed.iter().copied(),
                        MessageId::from_row(turn * stride),
                    )
                    .expect("試用中があること");
                    *counts.entry(chosen.get()).or_insert(0usize) += 1;
                }
                assert_eq!(
                    counts.len(),
                    k,
                    "k = {k}・歩幅 {stride}: 全員に回ること {counts:?}"
                );
                for (candidate, count) in &counts {
                    assert!(
                        *count >= 60 / k / 2,
                        "k = {k}・歩幅 {stride}: 候補 {candidate} が {count} 回しか選ばれない {counts:?}"
                    );
                }
            }
        }
    }
}
