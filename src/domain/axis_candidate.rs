//! **変容が見いだす軸の候補**のドメイン型
//! （[ADR-0046](../../docs/adr/0046-drift-proposes-new-axes.md) §決定1・§決定6 / 作業単位 A-9a）。
//!
//! ここにあるのは**形と口**（作業単位 A-9a / ADR-0046 §決定7 の第1段の1つ目）。
//! 候補を出す規則は `domain::axis_discovery`（A-9b）、試用して判定する規則は `domain::axis_trial`（A-9c）。
//! **採用する（第2段）はまだ無い** —— 合格の候補は記録されるだけで、性格ステータスには何も足されない。
//!
//! # ここにあるもの
//!
//! | 型 | 何か |
//! |---|---|
//! | [`AxisDiscoverySlots`] | キャラクターごとの**見いだす枠の数 `m`**（0〜4・既定 0） |
//! | [`AxisCandidateState`] | 候補の状態（試用 → 合格／不合格 → 採用／却下） |
//! | [`AxisTrialFailure`] | 不合格の理由（張り付き・沈黙・標本不足 / §決定4） |
//! | [`AxisTrialVote`] | 試用の票（上げ・下げ・`0`）。**変容の票（`DriftDelta`）と混ぜない**（§決定6） |
//! | [`AxisCandidateRecord`] | **唯一の書き手**（`Store::record_axis_candidate`）に渡す1件 |
//!
//! # 状態は追記で進み、いまの状態は導出（§決定5・6）
//!
//! 状態の移り変わりは1件ずつ追記され、**行を書き換えない。**
//! いまの状態は [`current_axis_candidate_state`]（純粋関数）が移り変わりの列から導く。
//! **導出の規則をここに1つだけ置く**——書き手（遷移を許すか）と読み手（いまどこか）が
//! 同じ [`AxisCandidateState::can_become`] を通るので、片方だけが緩む形にならない。

use std::fmt;

use super::memory::MemoryId;
use super::session::SessionId;
use super::stat::StatAxis;

/// **見いだす枠の数 `m`**（ADR-0046 §決定1）。**既定は 0（見いださない）。**
///
/// `StatScale` と同じく **`parse` が唯一のコンストラクタ**（`From<u32>` も `pub fn new` も生やさない）。
///
/// # 上限 4 の根拠は無い
///
/// ADR-0046 §決定1 のとおり**測っていない値**である。枠の軸は採用されると
/// 削れない枠に載る行になるので、上限そのものは要る。**測ってから動かす。**
/// **下げる向きは取れない**（保存済みの 4 が読めなくなる）。
/// **上げるときはこの定数だけでは足りない** —— `migrations/0015_stat_axis_candidate.sql` の
/// CHECK が同じ 4 で縛っているので、新しい移行で CHECK も上げないと書き込みが落ちる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AxisDiscoverySlots(u32);

impl AxisDiscoverySlots {
    /// 枠の数の上限（ADR-0046 §決定1。**測っていない値**）。
    pub const MAX: u32 = 4;

    /// **既定。見いださない。** 列を足しただけでは何も起きない（ADR-0046 §決定1）。
    pub const NONE: Self = Self(0);

    /// 唯一のコンストラクタ。`0..=MAX` の外は拒否する。
    ///
    /// **下端は `u32` そのもの**（0 は正当な値で、負値は型が表現できない）。
    ///
    /// # Errors
    ///
    /// `MAX` を超えれば [`AxisDiscoverySlotsError::TooLarge`]。
    pub const fn parse(raw: u32) -> Result<Self, AxisDiscoverySlotsError> {
        if raw > Self::MAX {
            return Err(AxisDiscoverySlotsError::TooLarge);
        }
        Ok(Self(raw))
    }

    /// 枠の数。SQL へのバインド値と応答にはこれを使う。
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// **空いている枠の数**（Issue origin#42 の「`m` − 試用中・合格・採用の数」）。
    ///
    /// 枠を埋めるのは [`AxisCandidateState::occupies_slot`] が真の状態だけ。
    /// **不合格と却下は枠を空ける**（ADR-0046 §決定5）。
    ///
    /// 埋まっている数が `m` を超えていても（後から `m` を下げた場合）**0 で止まる**。
    /// 下げた `m` は、すでに枠に居る候補を追い出さない（新しい候補を止めるだけ）。
    #[must_use]
    pub fn free(self, states: impl IntoIterator<Item = AxisCandidateState>) -> u32 {
        let occupied = states
            .into_iter()
            .filter(|state| state.occupies_slot())
            .count();
        let occupied = u32::try_from(occupied).unwrap_or(u32::MAX);
        self.0.saturating_sub(occupied)
    }
}

impl fmt::Display for AxisDiscoverySlots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// [`AxisDiscoverySlots::parse`] が受け付けなかった理由。
///
/// **`Display` は定数文字列**（入力も上限も反射しない / 絶対制約）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AxisDiscoverySlotsError {
    /// [`AxisDiscoverySlots::MAX`] を超える。
    #[error("軸を見いだす枠の数が大きすぎます")]
    TooLarge,
}

/// 軸の候補の主キー（`CharacterId` と同じく裸の `i64` にしない）。
///
/// **`From<i64>` も `pub fn new` も生やさない。** 得る手段は
/// `Store::record_axis_candidate` の戻り値と、`store` の読み出しだけ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AxisCandidateId(i64);

impl AxisCandidateId {
    /// DB から復元する内部経路。**crate 外からは呼べない。**
    pub(crate) const fn from_row(raw: i64) -> Self {
        Self(raw)
    }

    /// 主キーの値。SQL へのバインドと応答に使う。
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// **不合格の理由**（ADR-0046 §決定4）。
///
/// 標本不足は ADR-0046 の表で「沈黙の側に数える」とされているが、**記録は分ける**
/// ——§測る問い が「不合格の内訳（張り付き・沈黙・標本不足）」を数えるため。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisTrialFailure {
    /// 非ゼロの票のうち上げが線を超えた（張り付き）。
    Pinned,
    /// `0` の率が線を超えた（沈黙）。
    Silent,
    /// 非ゼロの票が `K` 件に届かなかった（標本不足）。
    TooFewSamples,
}

impl AxisTrialFailure {
    /// すべての値。**綴りの照合と網羅テストの走査に使う。**
    pub const ALL: [Self; 3] = [Self::Pinned, Self::Silent, Self::TooFewSamples];

    /// **保存と応答の綴り。** 綴りの正はここ（SQL の CHECK は移行した時点の写し）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pinned => "pinned",
            Self::Silent => "silent",
            Self::TooFewSamples => "too_few_samples",
        }
    }
}

/// **候補の状態**（ADR-0046 §決定3〜5）。
///
/// ```text
/// 試用 ──→ 合格 ──→ 採用
///   │        └───→ 却下
///   └───→ 不合格（理由つき）
/// ```
///
/// **行き先の無い状態**（不合格・採用・却下）からはどこへも動かない。
/// 却下・不合格の名前は二度と出させない（§決定5）ので、戻す経路を作らない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisCandidateState {
    /// 試用中（§決定3）。値を動かさず、本文にも載せず、別の依頼で採点される。
    Trial,
    /// 合格（§決定4）。**利用者への提案**になる（画面は第2段）。
    Passed,
    /// 不合格（§決定4）。**枠を空ける。**
    Failed(AxisTrialFailure),
    /// 採用（§決定5）。正式な軸になる。**枠を1つ埋めたまま。**
    Adopted,
    /// 却下（§決定5）。**枠を空ける。**
    Rejected,
}

impl AxisCandidateState {
    /// すべての値（不合格は理由ごと）。**網羅テストの走査に使う。**
    pub const ALL: [Self; 7] = [
        Self::Trial,
        Self::Passed,
        Self::Failed(AxisTrialFailure::Pinned),
        Self::Failed(AxisTrialFailure::Silent),
        Self::Failed(AxisTrialFailure::TooFewSamples),
        Self::Adopted,
        Self::Rejected,
    ];

    /// **保存と応答の状態の綴り。** 不合格の理由は [`Self::reason`] が別に持つ。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Trial => "trial",
            Self::Passed => "passed",
            Self::Failed(_) => "failed",
            Self::Adopted => "adopted",
            Self::Rejected => "rejected",
        }
    }

    /// 不合格の理由。不合格以外は `None`。
    #[must_use]
    pub const fn reason(self) -> Option<AxisTrialFailure> {
        match self {
            Self::Failed(reason) => Some(reason),
            _ => None,
        }
    }

    /// **保存された綴りから読み戻す。** 規則外は `None`（呼び出し側が壊れた行として扱う）。
    ///
    /// **状態と理由の組が揃っていること**まで見る——不合格なのに理由が無い、
    /// 不合格でないのに理由がある、はどちらも `None`。
    /// **前後の空白も大文字も畳まない**（`StatDriftMode::parse` と同じ。
    /// 畳むと CHECK を外して書かれた行が黙って読めてしまう）。
    /// 比較の相手は [`Self::ALL`] の綴りから取るので、ここに文字列は無い。
    #[must_use]
    pub fn from_stored(state: &str, reason: Option<&str>) -> Option<Self> {
        Self::ALL.into_iter().find(|candidate| {
            candidate.label() == state && candidate.reason().map(AxisTrialFailure::label) == reason
        })
    }

    /// **この状態から `next` へ動けるか。** 書き手と読み手の両方がここを通る。
    ///
    /// | 今 | 行き先 |
    /// |---|---|
    /// | 試用 | 合格 / 不合格 |
    /// | 合格 | 採用 / 却下 |
    /// | 不合格 / 採用 / 却下 | **なし** |
    ///
    /// **試用へ入る経路は候補を作ることだけ**（どの状態からも試用へは動けない）。
    #[must_use]
    pub const fn can_become(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Trial, Self::Passed | Self::Failed(_))
                | (Self::Passed, Self::Adopted | Self::Rejected)
        )
    }

    /// **枠を埋めるか**（Issue origin#42 の「試用中・合格・採用」）。不合格と却下は枠を空ける。
    #[must_use]
    pub const fn occupies_slot(self) -> bool {
        matches!(self, Self::Trial | Self::Passed | Self::Adopted)
    }
}

/// **移り変わりの列から、いまの状態を導く**（ADR-0046 §決定5・6）。**純粋関数。**
///
/// 列は**古い順**。規則は2つだけ:
///
/// * 最初は**試用**（候補は試用として生まれる）。
/// * 隣り合う2つは [`AxisCandidateState::can_become`] を満たす。
///
/// どちらかが崩れていれば `None`（呼び出し側が壊れた行として扱う）。
/// **空の列も `None`**——候補を作る書き手は試用の移り変わりを同じトランザクションで書くので、
/// 移り変わりの無い候補は壊れている。
#[must_use]
pub fn current_axis_candidate_state(history: &[AxisCandidateState]) -> Option<AxisCandidateState> {
    let (first, _) = history.split_first()?;
    if *first != AxisCandidateState::Trial {
        return None;
    }
    let chained = history.windows(2).all(|pair| pair[0].can_become(pair[1]));
    if !chained {
        return None;
    }
    history.last().copied()
}

/// **試用の票**（ADR-0046 §決定3・6）。上げ・下げ・`0`。
///
/// **変容の票（`DriftDelta`）と別の型にする。** あちらは `0` を記録しない
/// （門を通った提案だけが票になる）が、試用の判定は **`0` の率**を数える（§決定4 の沈黙）。
/// 型を共有すると、`0` を足した瞬間に**固定の軸の票の表へ `0` が書ける形**になる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisTrialVote {
    /// 上げ。
    Up,
    /// 下げ。
    Down,
    /// `0`（動かす理由が無い）。
    Zero,
}

impl AxisTrialVote {
    /// すべての値。**綴りの照合と網羅テストの走査に使う。**
    pub const ALL: [Self; 3] = [Self::Up, Self::Down, Self::Zero];

    /// **保存の綴り。** 綴りの正はここ（SQL の CHECK は移行した時点の写し）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Zero => "zero",
        }
    }

    /// 保存された綴りから読み戻す。**完全一致だけ**（畳まない）。
    #[must_use]
    pub fn from_stored(stored: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|vote| vote.label() == stored)
    }
}

/// **試用の票の内訳**（上げ・下げ・`0` の件数 / ADR-0046 §決定5「票の内訳を添える」）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct AxisTrialTally {
    /// 上げの件数。
    pub up: u32,
    /// 下げの件数。
    pub down: u32,
    /// `0` の件数。
    pub zero: u32,
}

impl AxisTrialTally {
    /// 1種類の票の件数を足し込む。**溢れない**（飽和する）。
    pub fn add(&mut self, vote: AxisTrialVote, count: u32) {
        let slot = match vote {
            AxisTrialVote::Up => &mut self.up,
            AxisTrialVote::Down => &mut self.down,
            AxisTrialVote::Zero => &mut self.zero,
        };
        *slot = slot.saturating_add(count);
    }
}

/// **唯一の書き手に渡す1件**（ADR-0046 §決定6 / ADR-0028 §決定5 と同じ形）。
///
/// 3つの表（候補・移り変わり・試用の票）へ書く経路を1つの関数
/// （`Store::record_axis_candidate`）に閉じるための入れ物。
/// **どの腕も、書く前にいまの状態を読んで検査される**（だから `BEGIN IMMEDIATE`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AxisCandidateRecord {
    /// **候補を作る。** 試用として生まれる（候補の行と試用の移り変わりを同時に書く）。
    ///
    /// 名前は `StatAxis`（上限つきの型）を通っている。**LLM の出力はこの型を通してから渡す**
    /// （Issue origin#42）。空いた枠が無ければ書き手が拒む。
    ///
    /// # 試みの表にも1行足す（作業単位 A-9b）
    ///
    /// **候補を採ったことは、それ自体が1回の試み**（結末 `proposed`）。
    /// 同じトランザクションで試みの表へ「`source_memory` まで数えた」を追記する
    /// ——次の候補づくりは、そこから `N` 件増えてから（`domain::memories_since`）。
    /// **出どころは「その回に数えた記憶のうち最新」を渡すこと**（`server::axis_discovery`）。
    Propose {
        /// 候補の名前。**同じキャラクターの中で二度使えない**（表の UNIQUE）。
        name: StatAxis,
        /// 出どころの記憶（ADR-0046 §決定2 の入力）。**同じキャラクターの記憶に限る**（複合外部キー）。
        ///
        /// **試みの目印を兼ねる**（上の節）。
        source_memory: MemoryId,
    },
    /// **状態を1つ進める。** [`AxisCandidateState::can_become`] を満たさなければ書き手が拒む。
    Advance {
        /// どの候補か（同じキャラクターのものに限る）。
        candidate: AxisCandidateId,
        /// 行き先。
        to: AxisCandidateState,
    },
    /// **試用の票を1件足す。** 候補が試用中でなければ書き手が拒む。
    TrialVote {
        /// どの候補か（同じキャラクターのものに限る）。
        candidate: AxisCandidateId,
        /// どの会話のターンで入った票か（変容の票と同じく、会話へ辿れるように）。
        session: SessionId,
        /// 票。
        vote: AxisTrialVote,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // 枠の数
    // ---------------------------------------------------------------------

    /// **0〜4 は通り、5 は拒否される。** 期待値は定数から作らない（外の値で押さえる）。
    #[test]
    fn slots_accept_zero_to_four_and_refuse_five() {
        for raw in 0..=4 {
            assert_eq!(
                AxisDiscoverySlots::parse(raw).map(AxisDiscoverySlots::get),
                Ok(raw),
                "{raw} は正当な枠の数であること"
            );
        }
        for raw in [5, 6, 100, u32::MAX] {
            assert_eq!(
                AxisDiscoverySlots::parse(raw),
                Err(AxisDiscoverySlotsError::TooLarge),
                "{raw} は拒否されること"
            );
        }
    }

    /// **既定は 0**（見いださない / ADR-0046 §決定1）。
    #[test]
    fn the_default_is_zero() {
        assert_eq!(AxisDiscoverySlots::NONE.get(), 0);
        assert_eq!(AxisDiscoverySlots::parse(0), Ok(AxisDiscoverySlots::NONE));
    }

    /// **枠を埋めるのは試用・合格・採用だけ**（Issue origin#42）。
    ///
    /// 7つの状態を全部並べ、埋める数を外の値（3 = 試用・合格・採用）で押さえる。
    #[test]
    fn only_trial_passed_and_adopted_occupy_a_slot() {
        let occupying: Vec<AxisCandidateState> = AxisCandidateState::ALL
            .into_iter()
            .filter(|state| state.occupies_slot())
            .collect();
        assert_eq!(
            occupying,
            vec![
                AxisCandidateState::Trial,
                AxisCandidateState::Passed,
                AxisCandidateState::Adopted,
            ]
        );
    }

    /// **空いている枠 = `m` − 埋めている数。** 下回らない。
    ///
    /// 枠を空ける状態（不合格・却下）を**同じ入力に混ぜる**——「数えるものと数えないもの」が
    /// 同居していないと、全部数える変異も何も数えない変異も片方で緑になる。
    #[test]
    fn free_slots_count_only_the_occupying_candidates() {
        let four = AxisDiscoverySlots::parse(4).expect("正当な枠の数");
        let mixed = [
            AxisCandidateState::Trial,
            AxisCandidateState::Failed(AxisTrialFailure::Silent),
            AxisCandidateState::Adopted,
            AxisCandidateState::Rejected,
        ];
        assert_eq!(four.free(mixed), 2, "試用と採用の2つだけが枠を埋める");
        assert_eq!(four.free([]), 4, "候補が無ければ全部空いている");

        let one = AxisDiscoverySlots::parse(1).expect("正当な枠の数");
        assert_eq!(one.free(mixed), 0, "埋まっている数が上回っても 0 で止まる");
        assert_eq!(
            AxisDiscoverySlots::NONE.free([]),
            0,
            "m = 0 は常に空きが無い"
        );
    }

    // ---------------------------------------------------------------------
    // 状態の綴り
    // ---------------------------------------------------------------------

    /// **綴りは値ごとに異なり、外の値（直書き）と一致する。**
    ///
    /// SQL の CHECK（`0015`）と応答はこの綴りを前提にしている。
    #[test]
    fn every_state_and_reason_has_its_own_spelling() {
        let spelled: Vec<(&str, Option<&str>)> = AxisCandidateState::ALL
            .into_iter()
            .map(|state| (state.label(), state.reason().map(AxisTrialFailure::label)))
            .collect();
        assert_eq!(
            spelled,
            vec![
                ("trial", None),
                ("passed", None),
                ("failed", Some("pinned")),
                ("failed", Some("silent")),
                ("failed", Some("too_few_samples")),
                ("adopted", None),
                ("rejected", None),
            ]
        );
        let votes: Vec<&str> = AxisTrialVote::ALL
            .into_iter()
            .map(AxisTrialVote::label)
            .collect();
        assert_eq!(votes, vec!["up", "down", "zero"]);
    }

    /// **読み戻しは綴りの往復で、状態と理由の組が揃わなければ `None`。**
    #[test]
    fn stored_spellings_round_trip_and_mismatched_pairs_are_refused() {
        for state in AxisCandidateState::ALL {
            assert_eq!(
                AxisCandidateState::from_stored(
                    state.label(),
                    state.reason().map(AxisTrialFailure::label)
                ),
                Some(state),
                "{state:?} が往復すること"
            );
        }
        for (state, reason) in [
            ("failed", None),
            ("trial", Some("pinned")),
            ("passed", Some("silent")),
            ("failed", Some("unknown")),
            ("Trial", None),
            (" trial", None),
            ("", None),
        ] {
            assert_eq!(
                AxisCandidateState::from_stored(state, reason),
                None,
                "({state:?}, {reason:?}) は読めないこと"
            );
        }
        for vote in AxisTrialVote::ALL {
            assert_eq!(AxisTrialVote::from_stored(vote.label()), Some(vote));
        }
        for stored in ["UP", " up", "0", "", "neutral"] {
            assert_eq!(AxisTrialVote::from_stored(stored), None, "{stored:?}");
        }
    }

    // ---------------------------------------------------------------------
    // 遷移といまの状態の導出
    // ---------------------------------------------------------------------

    /// **遷移の表を 7 × 7 で総当たりにする**（許されるのは外の値で書いた組だけ）。
    #[test]
    fn only_the_documented_transitions_are_allowed() {
        use AxisCandidateState::{Adopted, Failed, Passed, Rejected, Trial};
        let allowed = [
            (Trial, Passed),
            (Trial, Failed(AxisTrialFailure::Pinned)),
            (Trial, Failed(AxisTrialFailure::Silent)),
            (Trial, Failed(AxisTrialFailure::TooFewSamples)),
            (Passed, Adopted),
            (Passed, Rejected),
        ];
        for from in AxisCandidateState::ALL {
            for to in AxisCandidateState::ALL {
                assert_eq!(
                    from.can_become(to),
                    allowed.contains(&(from, to)),
                    "{from:?} → {to:?}"
                );
            }
        }
    }

    /// **いまの状態は列の最後。** 列の形が崩れていれば `None`。
    #[test]
    fn the_current_state_is_the_last_of_a_valid_chain() {
        use AxisCandidateState::{Adopted, Failed, Passed, Rejected, Trial};
        assert_eq!(current_axis_candidate_state(&[Trial]), Some(Trial));
        assert_eq!(current_axis_candidate_state(&[Trial, Passed]), Some(Passed));
        assert_eq!(
            current_axis_candidate_state(&[Trial, Passed, Adopted]),
            Some(Adopted)
        );
        assert_eq!(
            current_axis_candidate_state(&[Trial, Passed, Rejected]),
            Some(Rejected)
        );
        assert_eq!(
            current_axis_candidate_state(&[Trial, Failed(AxisTrialFailure::TooFewSamples)]),
            Some(Failed(AxisTrialFailure::TooFewSamples))
        );

        for broken in [
            &[][..],
            &[Passed][..],
            &[Trial, Trial][..],
            &[Trial, Adopted][..],
            &[Trial, Passed, Passed][..],
            &[Trial, Failed(AxisTrialFailure::Pinned), Passed][..],
            &[Trial, Passed, Adopted, Rejected][..],
        ] {
            assert_eq!(
                current_axis_candidate_state(broken),
                None,
                "壊れた列は読めないこと: {broken:?}"
            );
        }
    }

    /// **内訳は種類ごとに足し込まれ、溢れない。**
    #[test]
    fn the_tally_adds_each_kind_to_its_own_count() {
        let mut tally = AxisTrialTally::default();
        tally.add(AxisTrialVote::Up, 3);
        tally.add(AxisTrialVote::Down, 5);
        tally.add(AxisTrialVote::Zero, 7);
        tally.add(AxisTrialVote::Down, 1);
        assert_eq!(
            tally,
            AxisTrialTally {
                up: 3,
                down: 6,
                zero: 7
            }
        );
        tally.add(AxisTrialVote::Up, u32::MAX);
        assert_eq!(tally.up, u32::MAX, "溢れずに飽和すること");
    }
}
