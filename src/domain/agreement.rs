//! 取り決め（約束・決まり）のドメイン型（[ADR-0051](../../docs/adr/0051-promises-and-rules.md) /
//! 作業単位 A-12a）。
//!
//! 「明日また話そう」（約束）や「呼び捨てにしない」（決まり）のような、**会話をまたいで
//! 覚えておくべき取り決め**を表す。記憶とも属性とも別の型（ADR-0051 §決定1）。
//!
//! # 作ったもの・作らないもの
//!
//! | 作った | 作らない（ADR-0051 §「作る前」の測定と決め直し） |
//! |---|---|
//! | 手で書く・手で終わらせる・有効なものを毎回本文に載せる（A-12a） | 終わりの提案（決め直し 3 で置き換わった。終わらせるのは利用者だけ） |
//! | モデルによる提案（δ3 ＋ dedup2）・提案中と却下の状態・採る／捨てる（A-12c-1） | |
//!
//! 提案の依頼の組み立ては `prompt::agreement_proposal`、読みと票と後段は `parse::agreement_proposal`。

use std::fmt;

use chrono::{DateTime, Utc};

use super::SessionId;
use super::text::{contains_control_or_line_break, is_effectively_blank, trim_blank};

/// **有効な取り決めの件数の上限**（ADR-0051 §決定4 / 叩き台）。
///
/// 有効な取り決めは**削れない枠の次の「優先の枠」**に載る（ADR-0051 §決定4 の追記）。
/// 記憶と履歴より先に取るが、**入りきらないときは古い側から落とし、落とした件数を伝える**
/// （`warn` と一覧の口の `last_turn`）。件数に上限が無いと、落ちる件数が際限なく増え、
/// 記憶と履歴の取り分も細る。
///
/// **数えるのは有効なものだけ。** 終わった取り決めは本文に載らないので、枠を食わない。
/// 判定は [`admits_another_active_agreement`] の1箇所（書き手は `store` の1つのトランザクション）。
///
/// # 下げる向きは取れる
///
/// 下げても保存済みの行は読める（件数は読み出しで数えない）。超えた分は書けなくなるだけ
/// （書く口が断る）。**本文で落とすかどうかは予算が決める**（上限ではなく `prompt::budget`）。
/// 予算の見張り番は `prompt` の `agreements_at_their_limit_never_exhaust_the_default_budget_and_the_drop_is_reported`
/// と `agreements_at_their_limit_all_fit_a_sufficient_budget`。
pub const MAX_ACTIVE_AGREEMENTS: usize = 16;

/// **有効な取り決めがいま `active` 件のとき、もう1件書けるか**（ADR-0051 §決定4）。**純粋関数。**
///
/// 上限に達していたら書かせない。**古いものを黙って落とさない**——
/// 利用者が古いものを終わらせてから書く（画面が促す）。
#[must_use]
pub const fn admits_another_active_agreement(active: usize) -> bool {
    active < MAX_ACTIVE_AGREEMENTS
}

/// 検証・正規化を通った取り決めの中身（要旨の平叙文）。
///
/// **`parse` が唯一のコンストラクタ**（`AttributeValue` / `MemoryContent` と同じ作法）。
///
/// # 1行に限る（`AttributeValue` の側）
///
/// 取り決めは**本文に1行ずつ載る**（`> - [約束] …` / 優先の枠）。`MemoryContent` のように
/// 行区切りを許すと、1件が表示上2行になり、2行目が引用の外の行に見える形を作れてしまう
/// （無害化が畳むとはいえ、型の段階で入れさせない / `AttributeValue` と同じ衛生条件）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgreementContent(String);

impl AgreementContent {
    /// 中身の上限。**文字数**（Unicode スカラー値の個数）で数える（ADR-0051 §決定1 / 叩き台）。
    ///
    /// バイト数で数えないのは `MemoryContent::MAX_CHARS` と同じ理由
    /// （日本語だけが 1/3 しか書けなくなる）。
    ///
    /// # **この値は後から下げられない**
    ///
    /// 書き込みの検証と**読み出しの検証**（`store` → `StoreError::InvalidStoredValue`）の両方に使う。
    /// 下げると保存済みの長い取り決めが、そのキャラクターの本文の組み立てを丸ごと止める。
    /// スキーマ側にも同じ数の CHECK がある（`migrations/0018_agreement.sql`）。
    /// 揃っていることは `tests/store_schema.rs::agreement_content_limit_matches_the_schema_check` が見る。
    pub const MAX_CHARS: usize = 60;

    /// 唯一のコンストラクタ。正規化してから検証する。**手順は `AttributeValue::parse` と同じ。**
    ///
    /// 1. 前後の空白（`text::TRIMMABLE`）を除去する。中間の空白は保持する。
    /// 2. 空・見える文字が1つも無いなら拒否する。
    /// 3. 制御文字・行区切りを含むなら拒否する。
    /// 4. `MAX_CHARS` を超えるなら拒否する（**正規化の後**に数える）。
    pub fn parse(raw: &str) -> Result<Self, AgreementContentError> {
        let normalized = trim_blank(raw);

        if normalized.is_empty() || is_effectively_blank(normalized) {
            return Err(AgreementContentError::Empty);
        }
        if contains_control_or_line_break(normalized) {
            return Err(AgreementContentError::Control);
        }
        if normalized.chars().count() > Self::MAX_CHARS {
            return Err(AgreementContentError::TooLong);
        }

        Ok(Self(normalized.to_owned()))
    }

    /// 正規化後の文字列。SQL へのバインド値はこれを使う。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgreementContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 中身が受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AgreementContentError {
    /// 正規化すると空になる（空文字・空白のみ・見えない文字のみ）。
    #[error("取り決めの中身が空です")]
    Empty,

    /// 制御文字・行区切りを含む。
    #[error("取り決めの中身に改行や制御文字は使えません")]
    Control,

    /// 正規化後の文字数が `AgreementContent::MAX_CHARS` を超える。**上限値を埋め込まない。**
    #[error("取り決めの中身が長すぎます")]
    TooLong,
}

/// **取り決めの種類**（ADR-0051 §決定1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgreementKind {
    /// 約束（いつか果たす）。
    Promise,
    /// 決まり（守り続ける）。
    Rule,
}

impl AgreementKind {
    /// すべての値。**綴りの照合と網羅テストの走査に使う。**
    pub const ALL: [Self; 2] = [Self::Promise, Self::Rule];

    /// **保存と応答の綴り。** 綴りの正はここ（SQL の CHECK は移行した時点の写し）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Promise => "promise",
            Self::Rule => "rule",
        }
    }

    /// 綴りから戻す。**知らない綴りは `None`**（trim も小文字化もしない）。
    #[must_use]
    pub fn from_label(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.label() == raw)
    }
}

/// **守る側**（ADR-0051 §決定1）。**「誰が誰に」を落とさない**（`memory.subject` と同じ理由）。
///
/// 3値。`Subject` と**別の型**にしてある —— 「両方」は記憶にも属性にも無い値で、
/// `Subject` を使い回すと「両方」が表現できず、どちらかに寄せた時点で一人称が混線する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgreementKeeper {
    /// 利用者が守る。
    User,
    /// キャラクターが守る。
    Character,
    /// 利用者とキャラクターの両方が守る。
    Both,
}

impl AgreementKeeper {
    /// すべての値（**本文に書く順**でもある: 利用者 → キャラクター → 両方）。
    pub const ALL: [Self; 3] = [Self::User, Self::Character, Self::Both];

    /// **保存と応答の綴り。** 綴りの正はここ。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Character => "character",
            Self::Both => "both",
        }
    }

    /// 綴りから戻す。**知らない綴りは `None`。**
    #[must_use]
    pub fn from_label(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|keeper| keeper.label() == raw)
    }
}

/// **取り決めの状態**（ADR-0051 §決定1）。
///
/// ```text
/// 提案中 ──→ 有効 ──→ 果たした
///   │          ├───→ 破った
///   │          └───→ 取り消した
///   └───→ 却下
/// ```
///
/// **終わった状態と却下からはどこへも動かない**（戻す口は作らない。書き直すなら新しく書く）。
/// 提案中は裏方の提案（ADR-0051 §決定2 / 作業単位 A-12c-1）が書き、**利用者が採ると有効、
/// 捨てると却下**になる。**提案中は本文に載らず、有効な件数の上限にも数えない。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgreementState {
    /// 提案中（裏方が会話から見つけ、利用者がまだ採っていない）。**本文に載らない。上限に数えない。**
    Proposed,
    /// 有効。**本文に載るのはこの状態だけ**（§決定4）。
    Active,
    /// 却下（利用者が提案を捨てた）。**行は消さない** —— 提案の依頼の「# 提案済み」に載り、
    /// 同じものを二度出さない手掛かりになる（`server::agreement_proposal`）。
    Rejected,
    /// 果たした。
    Fulfilled,
    /// 破った。
    Broken,
    /// 取り消した。
    Withdrawn,
}

impl AgreementState {
    /// すべての値（並びは `migrations/0018_agreement.sql` の CHECK と同じ）。
    pub const ALL: [Self; 6] = [
        Self::Proposed,
        Self::Active,
        Self::Rejected,
        Self::Fulfilled,
        Self::Broken,
        Self::Withdrawn,
    ];

    /// **保存と応答の綴り。** 綴りの正はここ（SQL の CHECK は移行した時点の写し）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Active => "active",
            Self::Rejected => "rejected",
            Self::Fulfilled => "fulfilled",
            Self::Broken => "broken",
            Self::Withdrawn => "withdrawn",
        }
    }

    /// 綴りから戻す。**知らない綴りは `None`**（読み出しは `InvalidStoredValue` に落ちる）。
    ///
    /// # 提案中・却下は A-12c-1 で読めるようになった
    ///
    /// A-12a の時点ではスキーマが綴りだけを残していて、この関数は `None` を返していた。
    /// **読み手を先に広げてから書き手を足す**順で入れた（書き手を先に足すと、書いた瞬間に
    /// 一覧の口が 500 になる）。見張り番は
    /// `tests/store_agreement.rs::a_proposed_row_is_listed_but_not_read_as_active`。
    #[must_use]
    pub fn from_label(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.label() == raw)
    }

    /// **本文に載るか**（§決定4）。述語はここ1箇所（`prompt` も `store` の読み分けもこれを見る）。
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    /// **提案中か**（採る・捨てる口が受け付けるのはこの状態だけ / 作業単位 A-12c-1）。
    #[must_use]
    pub const fn is_proposed(self) -> bool {
        matches!(self, Self::Proposed)
    }
}

/// **終わらせ方**（利用者が選ぶ3つ / ADR-0051 §決め直し 3）。
///
/// [`AgreementState`] と別の型にしてあるのは、**「有効へ終わらせる」を表現できなくする**ため
/// （終わらせる口が `Active` を受け取れる形だと、終わっていない行に終わりの履歴が付く）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgreementEnding {
    /// 果たした。
    Fulfilled,
    /// 破った。
    Broken,
    /// 取り消した。
    Withdrawn,
}

impl AgreementEnding {
    /// すべての値。
    pub const ALL: [Self; 3] = [Self::Fulfilled, Self::Broken, Self::Withdrawn];

    /// 終わった後の状態。
    #[must_use]
    pub const fn state(self) -> AgreementState {
        match self {
            Self::Fulfilled => AgreementState::Fulfilled,
            Self::Broken => AgreementState::Broken,
            Self::Withdrawn => AgreementState::Withdrawn,
        }
    }

    /// 綴りから戻す（**綴りは [`AgreementState::label`] と同じ**。写しを作らない）。
    /// `active` や知らない綴りは `None`。
    #[must_use]
    pub fn from_label(raw: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|ending| ending.state().label() == raw)
    }
}

/// 取り決めの主キー（裸の `i64` にしない / `MemoryId` と同じ理由）。
///
/// **`From<i64>` も `pub fn new` も生やさない。** 得る手段は `store` の書き込みの戻り値と読み出しだけ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AgreementId(i64);

impl AgreementId {
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

/// 保存済みの取り決め1件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agreement {
    /// 主キー。
    pub id: AgreementId,
    /// 中身（要旨の平叙文）。
    pub content: AgreementContent,
    /// 種類。
    pub kind: AgreementKind,
    /// 守る側。**落とさない。**
    pub keeper: AgreementKeeper,
    /// いまの状態（**投影**。履歴の表の最後の行と同じトランザクションで書かれる）。
    pub state: AgreementState,
    /// 生まれた会話。**手で書いたものは `None`。**
    pub source_session: Option<SessionId>,
    /// 書いた時刻。
    pub created_at: DateTime<Utc>,
}

/// 手で書く取り決め（ADR-0051 §決定2「利用者が画面で自分で書いた取り決めは、提案を経ずに有効になる」）。
///
/// **状態を持たない**（書いたものは必ず有効で始まる）。**出どころも持たない**（手で書いたものは無し）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAgreement {
    /// 中身。
    pub content: AgreementContent,
    /// 種類。
    pub kind: AgreementKind,
    /// 守る側。**`Option` にしない**（省略できる形にした時点で、省略した行が必ず生まれる）。
    pub keeper: AgreementKeeper,
}

/// **提案を採るときの直し**（ADR-0051 §決定2「直してから採る」/ 作業単位 A-12c-1）。
///
/// **`None` の欄は提案のまま。** 3つとも `None`（[`Default`]）なら、提案をそのまま採る。
/// 中身は直しても上限を持つ型（[`AgreementContent`]）を通る。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgreementEdits {
    /// 直した中身。
    pub content: Option<AgreementContent>,
    /// 直した種類。
    pub kind: Option<AgreementKind>,
    /// 直した守る側。
    pub keeper: Option<AgreementKeeper>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LINE_BREAKS;
    use crate::domain::text::INVISIBLE_SAMPLES;

    fn parsed(raw: &str) -> AgreementContent {
        AgreementContent::parse(raw).expect("正当な中身として解釈できること")
    }

    /// 前後の空白は除去し、中間の空白は保持する。
    #[test]
    fn content_trims_the_edges_and_keeps_the_middle() {
        for (raw, expected) in [
            (" 毎朝あいさつする ", "毎朝あいさつする"),
            ("\u{3000}呼び捨てにしない\u{3000}", "呼び捨てにしない"),
            ("\t海へ 行く\u{00A0}", "海へ 行く"),
        ] {
            assert_eq!(parsed(raw).as_str(), expected, "raw = {raw:?}");
        }
    }

    /// 空・空白だけ・見えない文字だけは拒否する（`- [約束] ` だけの行を本文に載せない）。
    #[test]
    fn content_rejects_blank_and_invisible() {
        for raw in ["", " ", "\u{3000}", "\u{00A0}\t "] {
            assert_eq!(
                AgreementContent::parse(raw),
                Err(AgreementContentError::Empty),
                "raw = {raw:?}"
            );
        }
        for character in INVISIBLE_SAMPLES {
            assert_eq!(
                AgreementContent::parse(&character.to_string()),
                Err(AgreementContentError::Empty),
                "U+{:04X}",
                character as u32
            );
        }
    }

    /// 行区切り・制御文字は拒否する（**中間にあるときも**）。
    #[test]
    fn content_rejects_line_breaks_and_controls_in_the_middle() {
        for separator in LINE_BREAKS.iter().copied().chain(['\u{0007}', '\u{001B}']) {
            assert_eq!(
                AgreementContent::parse(&format!("毎朝{separator}あいさつする")),
                Err(AgreementContentError::Control),
                "U+{:04X}",
                separator as u32
            );
        }
    }

    /// **上限ちょうどは通り、1文字超えると拒否する。文字数で数える**（バイト数ではない）。
    #[test]
    fn content_is_limited_by_characters_not_bytes() {
        let at_limit = "約".repeat(AgreementContent::MAX_CHARS);
        assert!(
            at_limit.len() > AgreementContent::MAX_CHARS,
            "前提: 多バイト"
        );
        assert_eq!(
            AgreementContent::parse(&at_limit).map(|content| content.as_str().chars().count()),
            Ok(AgreementContent::MAX_CHARS)
        );
        assert_eq!(
            AgreementContent::parse(&"約".repeat(AgreementContent::MAX_CHARS + 1)),
            Err(AgreementContentError::TooLong)
        );
        // **長さは正規化の後に数える**（空白を足しただけで上限ちょうどが弾かれない）。
        assert!(AgreementContent::parse(&format!("\u{3000} {at_limit} ")).is_ok());
    }

    /// **上限の値そのものを、関数の外の値で押さえる**（ADR-0051 の叩き台: 60 字・16 件）。
    ///
    /// 期待値を定数から作ると、定数を倍にする変異が緑を通る（CLAUDE.md「見張り番の書き方」）。
    /// **値を変えるときは ADR-0051 の決定と、この2行を一緒に変えること。**
    #[test]
    fn the_limits_are_the_values_adr_0051_starts_with() {
        assert_eq!(AgreementContent::MAX_CHARS, 60);
        assert_eq!(MAX_ACTIVE_AGREEMENTS, 16);
    }

    /// **上限に達したら書かせない**（ちょうど上限の手前は書ける）。
    #[test]
    fn another_agreement_is_admitted_only_below_the_limit() {
        assert!(admits_another_active_agreement(0));
        assert!(admits_another_active_agreement(MAX_ACTIVE_AGREEMENTS - 1));
        assert!(!admits_another_active_agreement(MAX_ACTIVE_AGREEMENTS));
        assert!(!admits_another_active_agreement(MAX_ACTIVE_AGREEMENTS + 1));
    }

    /// `Display` は定数（入力も上限値も反射しない）。
    #[test]
    fn content_errors_do_not_reflect_the_input_or_the_limit() {
        let over = "秘密".repeat(AgreementContent::MAX_CHARS);
        let message = AgreementContent::parse(&over)
            .expect_err("長すぎること")
            .to_string();
        assert!(!message.contains("秘密"), "{message}");
        assert!(
            !message.contains(&AgreementContent::MAX_CHARS.to_string()),
            "{message}"
        );
    }

    /// **綴りを関数の外の値で押さえる**（保存と応答の両方に乗る綴り）。
    #[test]
    fn the_spellings_are_fixed() {
        assert_eq!(
            AgreementKind::ALL.map(AgreementKind::label),
            ["promise", "rule"]
        );
        assert_eq!(
            AgreementKeeper::ALL.map(AgreementKeeper::label),
            ["user", "character", "both"]
        );
        // **並びは `migrations/0018_agreement.sql` の CHECK と同じ**（提案中・却下を A-12c-1 で足した）。
        assert_eq!(
            AgreementState::ALL.map(AgreementState::label),
            [
                "proposed",
                "active",
                "rejected",
                "fulfilled",
                "broken",
                "withdrawn"
            ]
        );
    }

    /// **綴りから戻すと元に戻る。知らない綴りは `None`**（大文字・空白を足した形も）。
    #[test]
    fn labels_round_trip_and_unknown_spellings_are_refused() {
        for kind in AgreementKind::ALL {
            assert_eq!(AgreementKind::from_label(kind.label()), Some(kind));
        }
        for keeper in AgreementKeeper::ALL {
            assert_eq!(AgreementKeeper::from_label(keeper.label()), Some(keeper));
        }
        for state in AgreementState::ALL {
            assert_eq!(AgreementState::from_label(state.label()), Some(state));
        }
        for raw in ["", "Promise", " rule", "assistant"] {
            assert_eq!(AgreementKind::from_label(raw), None, "{raw:?}");
        }
        for raw in ["", "User", "both ", "assistant"] {
            assert_eq!(AgreementKeeper::from_label(raw), None, "{raw:?}");
        }
        // **提案中・却下は A-12c-1 で読めるようになった**（スキーマが綴りを残していたもの）。
        assert_eq!(
            AgreementState::from_label("proposed"),
            Some(AgreementState::Proposed)
        );
        assert_eq!(
            AgreementState::from_label("rejected"),
            Some(AgreementState::Rejected)
        );
        for raw in ["Proposed", " rejected", "Active", "", "done"] {
            assert_eq!(AgreementState::from_label(raw), None, "{raw:?}");
        }
    }

    /// **提案中なのは `Proposed` だけ**（採る・捨てる口が受け付ける状態）。
    #[test]
    fn only_the_proposed_state_is_proposed() {
        let proposed: Vec<AgreementState> = AgreementState::ALL
            .into_iter()
            .filter(|state| state.is_proposed())
            .collect();
        assert_eq!(proposed, vec![AgreementState::Proposed]);
    }

    /// **提案中・却下は終わらせ方として読めない**（終わらせる口は有効なものだけを終わらせる）。
    #[test]
    fn proposed_and_rejected_are_not_endings() {
        assert_eq!(AgreementEnding::from_label("proposed"), None);
        assert_eq!(AgreementEnding::from_label("rejected"), None);
    }

    /// **有効なのは `Active` だけ。**
    #[test]
    fn only_the_active_state_is_active() {
        let active: Vec<AgreementState> = AgreementState::ALL
            .into_iter()
            .filter(|state| state.is_active())
            .collect();
        assert_eq!(active, vec![AgreementState::Active]);
    }

    /// **終わらせ方は3つで、どれも有効ではない状態へ行く。`active` は終わらせ方として読めない。**
    #[test]
    fn endings_lead_to_distinct_inactive_states() {
        let states: Vec<AgreementState> = AgreementEnding::ALL
            .into_iter()
            .map(AgreementEnding::state)
            .collect();
        assert_eq!(
            states,
            vec![
                AgreementState::Fulfilled,
                AgreementState::Broken,
                AgreementState::Withdrawn
            ]
        );
        for ending in AgreementEnding::ALL {
            assert_eq!(
                AgreementEnding::from_label(ending.state().label()),
                Some(ending)
            );
        }
        assert_eq!(AgreementEnding::from_label("active"), None);
        assert_eq!(AgreementEnding::from_label("done"), None);
    }
}
