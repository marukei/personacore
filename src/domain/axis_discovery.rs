//! **軸の候補を「いつ」「何と照らして」出させるか**のドメイン規則
//! （[ADR-0046](../../docs/adr/0046-drift-proposes-new-axes.md) §決定2 / 作業単位 A-9b / Issue origin#42）。
//!
//! 候補の形と状態（`domain::axis_candidate`）の上に、**候補づくりの判断**だけを置く。
//! どれも**純粋関数**で、DB も上流も触らない（呼ぶのは `server::axis_discovery`）。
//!
//! | 型・関数 | 何か |
//! |---|---|
//! | [`AxisDiscoveryMemories`] | **`N`**（前回の試みから何件の記憶が増えたら尋ねるか）。**測定の器具** |
//! | [`memories_since`] | 「前回の試みより後に増えた、そのキャラクター自身の記憶」を選ぶ |
//! | [`axis_discovery_due`] | 空いた枠と `N` から、**いま尋ねるか**を決める |
//! | [`axis_name_clash`] | 返ってきた名前が**既存の名前と完全一致するか**（正規化しない） |
//! | [`AxisDiscoveryOutcome`] | 1回の試みの結末（**試みの表に残る綴り**） |
//!
//! # 「前回の試み」は候補の表からは取れない（Issue origin#42 の申し送り1）
//!
//! 候補の行が作られるのは**採った回だけ**なので、完全一致で捨てた回・崩れた応答の回は
//! 候補の表に痕跡を残さない。そこから「前回」を取ると、**採るまで毎ターン尋ねる**
//! （ADR-0046 §決定2 に正面から反する）。だから**試みを1回ずつ別の表に追記**し、
//! その表の「そこまで数えた記憶」（目印）から先だけを [`memories_since`] が数える。

use std::fmt;

use super::axis_candidate::{AxisCandidateState, AxisDiscoverySlots};
use super::memory::{Memory, MemoryId, Subject};
use super::stat::StatAxis;

/// **`N`** —— 前回の試みから、そのキャラクター自身の記憶が何件増えたら候補を尋ねるか
/// （ADR-0046 §決定2 / `PERSONACORE_AXIS_DISCOVERY_MEMORIES`）。
///
/// # ⚠ 既定値は**測っていない値**
///
/// [`Self::DEFAULT`] の 10 に根拠は無い（ADR-0046「まだ決めていないこと」）。
/// **§測る問い を回すための器具**として環境変数で振れる形にしてある。
/// 外したときに起きるのは「尋ねる回数が増える／減る」だけで、壊れはしない
/// （CLAUDE.md「遅くなるだけか、壊れるか」の前者）。
///
/// # 0 を取らない
///
/// 0 は「増えていなくても尋ねる」＝**毎ターン尋ねる**ことになり、
/// ADR-0028 §決定2（「なし」を言わないモデルに毎ターン尋ねない）を設定1つで裏返せてしまう。
/// **`parse` が唯一のコンストラクタ**（`From<u32>` も `pub fn new` も生やさない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AxisDiscoveryMemories(u32);

impl AxisDiscoveryMemories {
    /// **既定。測っていない値**（型の doc）。
    pub const DEFAULT: Self = Self(10);

    /// 唯一のコンストラクタ。**0 は拒否する**（型の doc）。上端は縛らない
    /// （大きすぎる値が起こすのは「尋ねる回数が減る」だけ）。
    ///
    /// # Errors
    ///
    /// 0 なら [`AxisDiscoveryMemoriesError::Zero`]。
    pub const fn parse(raw: u32) -> Result<Self, AxisDiscoveryMemoriesError> {
        if raw == 0 {
            return Err(AxisDiscoveryMemoriesError::Zero);
        }
        Ok(Self(raw))
    }

    /// 件数。
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl Default for AxisDiscoveryMemories {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Display for AxisDiscoveryMemories {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// [`AxisDiscoveryMemories::parse`] が受け付けなかった理由。**`Display` は定数文字列。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AxisDiscoveryMemoriesError {
    /// 0（毎ターン尋ねることになる）。
    #[error("軸の候補を尋ねる間隔の記憶の件数は 1 以上にしてください")]
    Zero,
}

/// **前回の試みより後に増えた、そのキャラクター自身の記憶**を古い順に返す（純粋関数）。
///
/// * **主語が `character` のものだけ**（ADR-0046 §決定2「そのキャラクター自身の記憶」）。
///   利用者についての記憶を入れると、利用者の性質をキャラクターの軸として出させることになる。
/// * **`through`（前回の試みで数え終えた記憶）より大きい id だけ。** `None`（まだ一度も
///   試みていない）なら全部。
///
/// 渡すのは想起の入口（`Store::recall_memories`）が返した列——**上書きされた記憶は
/// もう入っていない**（想起は `superseded_by IS NULL` しか読まない）。
/// 並びは id の昇順にそろえる（呼び出し側の並びに寄りかからない）。
#[must_use]
pub fn memories_since(memories: &[Memory], through: Option<MemoryId>) -> Vec<&Memory> {
    let mut selected: Vec<&Memory> = memories
        .iter()
        .filter(|memory| memory.subject == Subject::Character)
        .filter(|memory| through.is_none_or(|through| memory.id > through))
        .collect();
    selected.sort_by_key(|memory| memory.id);
    selected
}

/// **いま尋ねるか**の判断（ADR-0046 §決定2 / Issue origin#42）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisDiscoveryDue {
    /// 空いた枠が無い（**`m` = 0 も必ずここ**）。
    NoFreeSlot,
    /// 空いた枠はあるが、記憶がまだ `N` 件増えていない。
    NotEnoughMemories,
    /// **尋ねる。** 空いた枠の数を持つ（ログに出す）。
    Due {
        /// 空いている枠の数（1 以上）。
        free_slots: u32,
    },
}

/// 空いた枠と、前回の試みから増えた記憶の件数から、**いま尋ねるか**を決める（純粋関数）。
///
/// **枠を先に見る。** `m` = 0 のキャラクターは記憶が何件あっても尋ねない
/// （Issue origin#42「`m` = 0 のキャラクターには1回も依頼しない」）。
#[must_use]
pub fn axis_discovery_due(
    slots: AxisDiscoverySlots,
    states: impl IntoIterator<Item = AxisCandidateState>,
    new_memories: usize,
    every: AxisDiscoveryMemories,
) -> AxisDiscoveryDue {
    let free_slots = slots.free(states);
    if free_slots == 0 {
        return AxisDiscoveryDue::NoFreeSlot;
    }
    let every = usize::try_from(every.get()).unwrap_or(usize::MAX);
    if new_memories < every {
        return AxisDiscoveryDue::NotEnoughMemories;
    }
    AxisDiscoveryDue::Due { free_slots }
}

/// **返ってきた名前が、どの既存の名前と一致したか**（ADR-0046 §決定2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisNameClash {
    /// いまの性格の軸（固定の n 本）。
    Fixed,
    /// 既存の候補（状態つき。**却下・不合格も含む** —— 二度出させない / §決定5）。
    Candidate(AxisCandidateState),
}

impl AxisNameClash {
    /// **ログの集計キー**（`axis_discovery_detail`）。候補の状態の綴りは
    /// `AxisCandidateState::label` をそのまま使う（写しを作らない）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Candidate(state) => state.label(),
        }
    }
}

/// **返ってきた名前が既存の名前と完全一致するか**（純粋関数 / ADR-0046 §決定2）。
///
/// # 正規化しない
///
/// 比べるのは `StatAxis` どうしの `==`（`StatAxis::parse` が前後の空白を落とした後の綴り）だけ。
/// **送り仮名違い（`落ち着き` と `落着き`）・簡体字（`陽` と `阳`）は別の名前として通す**
/// （ADR-0030 §決定3 の ⚠「綴りを救う方向へ倒さない」/ 却下欄「綴りをアプリ側で正規化する」）。
/// 揺れた綴りは試用で数が決める。
///
/// 固定の軸を先に見る（同じ名前が両方にあるなら `Fixed` と名乗る）。
#[must_use]
pub fn axis_name_clash<'a>(
    name: &StatAxis,
    fixed: impl IntoIterator<Item = &'a StatAxis>,
    candidates: impl IntoIterator<Item = (&'a StatAxis, AxisCandidateState)>,
) -> Option<AxisNameClash> {
    if fixed.into_iter().any(|axis| axis == name) {
        return Some(AxisNameClash::Fixed);
    }
    candidates
        .into_iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, state)| AxisNameClash::Candidate(state))
}

/// **候補を採らずに終わった試み**の理由（試みの表と `axis_discovery_outcome` の綴り）。
///
/// どれも**「試みた」に数える**（Issue origin#42 の申し送り1）——次の試みは、また `N` 件
/// 増えてから。数えないと、崩れた応答を返し続けるモデルに毎ターン尋ねることになる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisDiscoveryMiss {
    /// 既存の名前と完全一致した（[`axis_name_clash`]）。
    Duplicate,
    /// 応答から名前を取り出せなかった（`parse::axis_candidate`）。
    Malformed,
    /// 応答が上限で打ち切られていた（**丸ごと捨てる** —— 名前の途中で切れていても
    /// `StatAxis` を通ってしまう）。
    Truncated,
    /// 上流を呼べなかった。
    UpstreamFailed,
    /// 名前は採れたが、候補として書けなかった（枠が埋まった・同名が先に入った・DB の失敗）。
    NotRecorded,
}

/// 1回の試みの結末。**試みの表の `outcome` 列の綴りはここが正**
/// （`migrations/0016_*.sql` の CHECK は移行した時点の写し）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisDiscoveryOutcome {
    /// 候補を採り、試用として追記した。
    Proposed,
    /// 採らずに終わった。
    Missed(AxisDiscoveryMiss),
}

impl AxisDiscoveryOutcome {
    /// すべての値。**綴りの照合と網羅テストの走査に使う。**
    pub const ALL: [Self; 6] = [
        Self::Proposed,
        Self::Missed(AxisDiscoveryMiss::Duplicate),
        Self::Missed(AxisDiscoveryMiss::Malformed),
        Self::Missed(AxisDiscoveryMiss::Truncated),
        Self::Missed(AxisDiscoveryMiss::UpstreamFailed),
        Self::Missed(AxisDiscoveryMiss::NotRecorded),
    ];

    /// **保存とログの綴り。**
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Missed(AxisDiscoveryMiss::Duplicate) => "duplicate",
            Self::Missed(AxisDiscoveryMiss::Malformed) => "malformed",
            Self::Missed(AxisDiscoveryMiss::Truncated) => "truncated",
            Self::Missed(AxisDiscoveryMiss::UpstreamFailed) => "upstream_failed",
            Self::Missed(AxisDiscoveryMiss::NotRecorded) => "not_recorded",
        }
    }
}

/// **依頼の書き出し方の手本に使う名前の代わり**（`prompt::axis_discovery` と
/// `parse::axis_candidate` が共有する / 2つは互いに依存しない）。
///
/// **そのまま写してきたら名前ではない**（`parse` が捨てる）。捨てないと、
/// `項目名` という試用中の候補が枠を1つ占める。
pub const AXIS_CANDIDATE_PLACEHOLDER: &str = "項目名";

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::domain::{AxisTrialFailure, CharacterId, MemoryContent};

    fn memory(id: i64, subject: Subject, content: &str) -> Memory {
        Memory {
            id: MemoryId::from_row(id),
            character_id: CharacterId::from_row(2),
            subject,
            content: MemoryContent::parse(content).expect("正当な本文"),
            category: None,
            created_at: Utc.with_ymd_and_hms(2026, 9, 27, 0, 0, 0).unwrap(),
            locked: false,
        }
    }

    fn axis(name: &str) -> StatAxis {
        StatAxis::parse(name).expect("正当な軸名")
    }

    fn slots(raw: u32) -> AxisDiscoverySlots {
        AxisDiscoverySlots::parse(raw).expect("正当な枠の数")
    }

    fn every(raw: u32) -> AxisDiscoveryMemories {
        AxisDiscoveryMemories::parse(raw).expect("正当な間隔")
    }

    // ---------------------------------------------------------------------
    // N
    // ---------------------------------------------------------------------

    /// **0 は拒否、1 以上は通る。既定は 10**（外の値で押さえる）。
    #[test]
    fn the_interval_refuses_zero_and_defaults_to_ten() {
        assert_eq!(
            AxisDiscoveryMemories::parse(0),
            Err(AxisDiscoveryMemoriesError::Zero)
        );
        for raw in [1, 2, 10, u32::MAX] {
            assert_eq!(
                AxisDiscoveryMemories::parse(raw).map(AxisDiscoveryMemories::get),
                Ok(raw)
            );
        }
        assert_eq!(AxisDiscoveryMemories::DEFAULT.get(), 10);
        assert_eq!(
            AxisDiscoveryMemories::default(),
            AxisDiscoveryMemories::DEFAULT
        );
    }

    // ---------------------------------------------------------------------
    // 前回の試みより後の記憶
    // ---------------------------------------------------------------------

    /// **キャラクター自身の記憶だけを、目印より後だけ、id の昇順で。**
    ///
    /// 数えるもの（キャラクター・目印より後）と数えないもの（利用者・目印以前）を
    /// **同じ入力に混ぜる**。並びも崩して渡す（呼び出し側の並びに寄りかからない）。
    #[test]
    fn only_the_characters_own_memories_after_the_mark_are_counted() {
        let memories = [
            memory(7, Subject::Character, "七"),
            memory(3, Subject::Character, "三"),
            memory(5, Subject::User, "五"),
            memory(6, Subject::Character, "六"),
            memory(4, Subject::Character, "四"),
            memory(8, Subject::User, "八"),
        ];
        let ids = |selected: Vec<&Memory>| -> Vec<i64> {
            selected.iter().map(|memory| memory.id.get()).collect()
        };
        assert_eq!(
            ids(memories_since(&memories, Some(MemoryId::from_row(4)))),
            vec![6, 7],
            "目印（4）ちょうどは数え終えている。利用者の記憶（5・8）は数えない"
        );
        assert_eq!(
            ids(memories_since(&memories, None)),
            vec![3, 4, 6, 7],
            "まだ一度も試みていなければ、キャラクター自身の記憶を全部"
        );
        assert!(memories_since(&memories, Some(MemoryId::from_row(7))).is_empty());
    }

    // ---------------------------------------------------------------------
    // いま尋ねるか
    // ---------------------------------------------------------------------

    /// **m = 0 は記憶が何件あっても尋ねない。空きがあり N 件に届いたときだけ尋ねる。**
    #[test]
    fn a_request_is_due_only_with_a_free_slot_and_enough_memories() {
        let three = every(3);
        assert_eq!(
            axis_discovery_due(AxisDiscoverySlots::NONE, [], 1000, three),
            AxisDiscoveryDue::NoFreeSlot,
            "m = 0 は尋ねない"
        );
        assert_eq!(
            axis_discovery_due(slots(2), [], 2, three),
            AxisDiscoveryDue::NotEnoughMemories,
            "N − 1 件では尋ねない"
        );
        assert_eq!(
            axis_discovery_due(slots(2), [], 3, three),
            AxisDiscoveryDue::Due { free_slots: 2 },
            "N 件ちょうどで尋ねる"
        );
        assert_eq!(
            axis_discovery_due(
                slots(2),
                [
                    AxisCandidateState::Trial,
                    AxisCandidateState::Rejected,
                    AxisCandidateState::Failed(AxisTrialFailure::Silent),
                ],
                4,
                three,
            ),
            AxisDiscoveryDue::Due { free_slots: 1 },
            "却下・不合格は枠を空ける"
        );
        assert_eq!(
            axis_discovery_due(
                slots(2),
                [AxisCandidateState::Trial, AxisCandidateState::Adopted],
                1000,
                three,
            ),
            AxisDiscoveryDue::NoFreeSlot,
            "試用と採用で埋まっていれば尋ねない"
        );
    }

    // ---------------------------------------------------------------------
    // 名前の照合
    // ---------------------------------------------------------------------

    /// **完全一致だけが一致。送り仮名違い・簡体字・前後の文字違いは別の名前。**
    ///
    /// 照合の相手（固定の軸・候補の5つの状態）を全部並べ、真ん中の候補に当てる腕を置く。
    #[test]
    fn only_an_exact_spelling_clashes() {
        let fixed = [axis("落ち着き"), axis("陽気さ")];
        let candidates = [
            (axis("照れ屋"), AxisCandidateState::Trial),
            (axis("負けず嫌い"), AxisCandidateState::Rejected),
            (
                axis("皮肉屋"),
                AxisCandidateState::Failed(AxisTrialFailure::Pinned),
            ),
        ];
        let clash = |name: &str| {
            axis_name_clash(
                &axis(name),
                fixed.iter(),
                candidates.iter().map(|(name, state)| (name, *state)),
            )
        };

        assert_eq!(clash("落ち着き"), Some(AxisNameClash::Fixed));
        assert_eq!(
            clash("  陽気さ "),
            Some(AxisNameClash::Fixed),
            "前後の空白は型が落とす"
        );
        assert_eq!(
            clash("負けず嫌い"),
            Some(AxisNameClash::Candidate(AxisCandidateState::Rejected)),
            "真ん中の候補（却下）"
        );
        assert_eq!(
            clash("皮肉屋"),
            Some(AxisNameClash::Candidate(AxisCandidateState::Failed(
                AxisTrialFailure::Pinned
            )))
        );
        assert_eq!(
            clash("照れ屋"),
            Some(AxisNameClash::Candidate(AxisCandidateState::Trial))
        );

        for different in ["落着き", "阳気さ", "照れ屋さん", "負けず嫌"] {
            assert_eq!(
                clash(different),
                None,
                "{different} は別の名前として通ること"
            );
        }
    }

    /// **集計キーは値ごとに異なり、外の値と一致する。**
    #[test]
    fn every_outcome_and_clash_has_its_own_spelling() {
        let outcomes: Vec<&str> = AxisDiscoveryOutcome::ALL
            .into_iter()
            .map(AxisDiscoveryOutcome::label)
            .collect();
        assert_eq!(
            outcomes,
            vec![
                "proposed",
                "duplicate",
                "malformed",
                "truncated",
                "upstream_failed",
                "not_recorded",
            ]
        );
        assert_eq!(AxisNameClash::Fixed.label(), "fixed");
        assert_eq!(
            AxisNameClash::Candidate(AxisCandidateState::Rejected).label(),
            "rejected"
        );
    }
}
