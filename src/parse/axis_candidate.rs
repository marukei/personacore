//! **軸の候補の応答の分解**（[ADR-0046](../../docs/adr/0046-drift-proposes-new-axes.md) §決定2 /
//! 作業単位 A-9b）。**純粋関数。panic しない**（CLAUDE.md 絶対制約）。
//!
//! 依頼（`prompt::axis_discovery`）は「新しい項目の名前を1つ、1行で」と頼む。
//! ここはその応答から**名前を1つ取り出すか、取り出せなかった理由を返す**だけで、
//! 既存の名前との照合（`domain::axis_name_clash`）も保存もしない。
//!
//! # 受ける形
//!
//! **空行を除いてちょうど1行**。その行の
//!
//! 1. 先頭の箇条書きの印（`- ` / `-` / `・` / `* ` / `• `）か、番号付きの印
//!    （`1. ` / `1) ` / `１．`）を1つ落とす。
//! 2. 全体を囲む括弧・引用符の対（`「」` / `『』` / `“”` / `""` / `` ` `` / `**`）を1組落とす。
//! 3. 残りを `StatAxis::parse`（上限つきの型）に通す。
//!
//! **1〜2 は応答の形の読み取りであって、名前の正規化ではない**（`parse::drift` が
//! 引用記号を剥がすのと同じ立場）。**名前の中の字は1字も変えない** ——
//! 送り仮名違い・簡体字は別の名前のまま通る（ADR-0030 §決定3 の ⚠）。
//!
//! # 捨てる形（理由つき）
//!
//! | 形 | 理由 | なぜ救わないか |
//! |---|---|---|
//! | 上限で打ち切られた | [`AxisCandidateReading::Truncated`] | **名前の途中で切れても `StatAxis` を通る**（`甘えたが` が候補になる） |
//! | 空 | `empty` | |
//! | 2行以上 | `several_lines` | **どの行が名前か決められない**（説明を添えたのか、2つ挙げたのか） |
//! | 手本の `項目名` をそのまま写した | `placeholder` | 名前ではない |
//! | 「なし」系（[`NOTHING_ANSWERS`]。末尾の `。` / `.` を1つ剥がして比べる） | `nothing` | 名前ではない（試用の枠を1つ占めさせない） |
//! | **文字・数字を1つも含まない**（`**` / `-` / `…` / `「`） | `no_letter` | `StatAxis` は通るが名前ではない。**型は締めない**（下の節） |
//! | `StatAxis` を通らない（長すぎる・制御文字…） | `not_a_name` | 上限つきの型が唯一の関所（CLAUDE.md） |
//!
//! # 「文字を含まない」をここで捨て、`StatAxis` を締めない理由（A-9b のレビュー）
//!
//! `StatAxis::parse` は**既存の軸の書き込みと DB の読み出しの両方**が通る関所で、
//! 締めると保存済みの軸が読めなくなりうる（`StatAxis::MAX_CHARS` の doc「後から下げられない」と同じ構造）。
//! **これは「答えの形」の問題**——モデルが印や記号だけを返した回——なので、応答を読む側で捨てる。
//! 判定は `char::is_alphanumeric`（Unicode の Alphabetic / Numeric）で、
//! **漢字・仮名・長音符 `ー`（Lm）は文字として数える。**
//!
//! # LLM の出力は信頼できない入力
//!
//! 返すのは `StatAxis`（上限つきの型）か定数ラベルだけ。**生出力はどこへも渡さない**
//! （SQL・シェル・パスにもログにも。ログに名前を出さないのは A-9a の方針）。

use crate::domain::{AXIS_CANDIDATE_PLACEHOLDER, StatAxis};

use super::extraction::Completeness;

/// 先頭の箇条書きの印。**長いものから試す**（`- ` を `-` より先に）。
///
/// **`*` 単独は印にしない** —— `**照れ屋**`（強調）の先頭の `*` を印として落とすと、
/// 囲みの対が崩れて `*照れ屋**` が名前になる。
const BULLETS: [&str; 5] = ["- ", "-", "・", "* ", "• "];

/// 番号付きの印の、数字の後に来る区切り（`1.` / `1)` / `１．` / `１）`）。
///
/// **区切りが無ければ番号ではない**——`1番手` は名前のまま通す（数字で始まる名前を削らない）。
const NUMBER_DELIMITERS: [char; 4] = ['.', ')', '．', '）'];

/// **「無い」と答えた綴りの一覧**（`nothing` として捨てる）。**完全一致だけ**（`なし崩し` は名前）。
///
/// # なぜ一覧で持つのか（A-9b のレビュー）
///
/// ADR-0027 §診断のとおりモデルは「なし」をほとんど言わないが、言うときの綴りは1つではない
/// （`特になし` / `無し` / `ありません`）。**1つだけ見ると、残りの綴りが `特になし` という名前の
/// 試用中の候補になって枠を1つ占める**（試用は値を動かさないが、T ターン枠を塞ぐ）。
/// **名前で美徳を弾く規則ではない**（ADR-0046 §決定2 の注記）——これは答えの形の話で、
/// 一覧に載せるのは「無い」と言っている綴りだけにする。
///
/// 比べる前に**末尾の `。` / `.` を1つだけ**剥がす（`なし。`）。**名前そのものの句点は剥がさない**
/// ——剥がすのは「無い」との比較のためだけで、`照れ屋。` は `照れ屋。` のまま通る（正規化しない）。
const NOTHING_ANSWERS: [&str; 6] = [
    "なし",
    "特になし",
    "無し",
    "特に無し",
    "ありません",
    "特にありません",
];

/// 「無い」と比べる前に剥がす末尾の句点。
const TRAILING_STOPS: [char; 2] = ['。', '.'];

/// 全体を囲む括弧・引用符の対。**長いものから試す**（`**` を `*` より先に —— `*` 単独は対に無い）。
const WRAPPERS: [(&str, &str); 6] = [
    ("**", "**"),
    ("「", "」"),
    ("『", "』"),
    ("“", "”"),
    ("\"", "\""),
    ("`", "`"),
];

/// 応答から読み取ったもの。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AxisCandidateReading {
    /// **名前を1つ取り出せた**（`StatAxis` を通っている）。
    Named(StatAxis),
    /// 上限で打ち切られていた（**丸ごと捨てる**）。
    Truncated,
    /// 名前を取り出せなかった。
    Malformed(AxisCandidateMalformed),
}

/// 名前を取り出せなかった理由。**集計キーは値ごとの定数**（入力を反射しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AxisCandidateMalformed {
    /// 空（空白だけも含む）。
    Empty,
    /// 空行を除いて2行以上。
    SeveralLines,
    /// 手本の名前の代わり（`domain::AXIS_CANDIDATE_PLACEHOLDER`）をそのまま写した。
    Placeholder,
    /// 「無い」と答えた（[`NOTHING_ANSWERS`]）。
    Nothing,
    /// **文字・数字を1つも含まない**（`**` / `-` / `…` / `「`）。`StatAxis` は通るが名前ではない。
    NoLetter,
    /// `StatAxis::parse` を通らない。
    NotAName,
}

impl AxisCandidateMalformed {
    /// すべての値。
    pub const ALL: [Self; 6] = [
        Self::Empty,
        Self::SeveralLines,
        Self::Placeholder,
        Self::Nothing,
        Self::NoLetter,
        Self::NotAName,
    ];

    /// **ログの集計キー**（`axis_discovery_detail`）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::SeveralLines => "several_lines",
            Self::Placeholder => "placeholder",
            Self::Nothing => "nothing",
            Self::NoLetter => "no_letter",
            Self::NotAName => "not_a_name",
        }
    }
}

/// 候補の応答を分解する。**純粋関数。どんな入力でも panic しない。**
#[must_use]
pub fn parse_axis_candidate(raw: &str, completeness: Completeness) -> AxisCandidateReading {
    if completeness == Completeness::Truncated {
        return AxisCandidateReading::Truncated;
    }

    let mut lines = raw.lines().map(str::trim).filter(|line| !line.is_empty());
    let Some(line) = lines.next() else {
        return AxisCandidateReading::Malformed(AxisCandidateMalformed::Empty);
    };
    if lines.next().is_some() {
        return AxisCandidateReading::Malformed(AxisCandidateMalformed::SeveralLines);
    }

    let unbulleted = BULLETS
        .iter()
        .find_map(|bullet| line.strip_prefix(bullet))
        .or_else(|| strip_number(line))
        .unwrap_or(line)
        .trim();
    let unwrapped = WRAPPERS
        .iter()
        .find_map(|(open, close)| {
            unbulleted
                .strip_prefix(open)
                .and_then(|rest| rest.strip_suffix(close))
        })
        .unwrap_or(unbulleted)
        .trim();

    if unwrapped.is_empty() {
        return AxisCandidateReading::Malformed(AxisCandidateMalformed::Empty);
    }
    if unwrapped == AXIS_CANDIDATE_PLACEHOLDER {
        return AxisCandidateReading::Malformed(AxisCandidateMalformed::Placeholder);
    }
    let unstopped = unwrapped
        .strip_suffix(TRAILING_STOPS)
        .unwrap_or(unwrapped)
        .trim_end();
    if NOTHING_ANSWERS.contains(&unstopped) {
        return AxisCandidateReading::Malformed(AxisCandidateMalformed::Nothing);
    }
    if !unwrapped.chars().any(char::is_alphanumeric) {
        return AxisCandidateReading::Malformed(AxisCandidateMalformed::NoLetter);
    }
    StatAxis::parse(unwrapped).map_or(
        AxisCandidateReading::Malformed(AxisCandidateMalformed::NotAName),
        AxisCandidateReading::Named,
    )
}

/// **番号付きの印を1つ剥がす**（`1. ` / `12) ` / `１．`）。番号でなければ `None`。
///
/// 数字（ASCII と全角）が1つ以上続き、その直後が [`NUMBER_DELIMITERS`] のどれかのときだけ剥がす。
/// **`chars()` と `strip_prefix` だけで走査する**（バイト添字でスライスしない / `parse` の絶対制約）。
fn strip_number(line: &str) -> Option<&str> {
    let rest =
        line.trim_start_matches(|ch: char| ch.is_ascii_digit() || ('０'..='９').contains(&ch));
    if rest.len() == line.len() {
        return None;
    }
    rest.strip_prefix(NUMBER_DELIMITERS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str) -> AxisCandidateReading {
        AxisCandidateReading::Named(StatAxis::parse(name).expect("正当な軸名"))
    }

    fn read(raw: &str) -> AxisCandidateReading {
        parse_axis_candidate(raw, Completeness::Complete)
    }

    /// **1行の名前は、印と囲みを1組ずつ落として取り出す。名前の中の字は変えない。**
    #[test]
    fn a_single_line_yields_the_name_without_its_bullet_or_quotes() {
        for (raw, name) in [
            ("照れ屋", "照れ屋"),
            ("- 照れ屋", "照れ屋"),
            ("-照れ屋", "照れ屋"),
            ("・照れ屋", "照れ屋"),
            ("* 照れ屋", "照れ屋"),
            ("「照れ屋」", "照れ屋"),
            ("- 「照れ屋」", "照れ屋"),
            ("**照れ屋**", "照れ屋"),
            ("`照れ屋`", "照れ屋"),
            ("\n\n  - 照れ屋  \n\n", "照れ屋"),
            ("照れ屋\r\n", "照れ屋"),
            // **名前の中の空白・送り仮名・簡体字は変えない**（正規化しない）。
            ("- 内なる 情熱", "内なる 情熱"),
            ("落着き", "落着き"),
            ("阳気さ", "阳気さ"),
            // **囲みは1組だけ**（二重の囲みの内側は名前の一部として `StatAxis` が決める）。
            ("「「照れ屋」」", "「照れ屋」"),
            // **番号付きの印も箇条書きの印と同じく1つ剥がす**（レビュー指摘 / A-9b）。
            ("1. 照れ屋", "照れ屋"),
            ("1) 照れ屋", "照れ屋"),
            ("１．照れ屋", "照れ屋"),
            ("12. 「照れ屋」", "照れ屋"),
            // **番号の後に印が無ければ名前の一部**（数字で始まる名前を削らない）。
            ("1番手", "1番手"),
        ] {
            assert_eq!(read(raw), named(name), "{raw:?}");
        }
    }

    /// **打ち切られた応答は、名前として読める形でも丸ごと捨てる。**
    #[test]
    fn a_truncated_response_is_dropped_whole() {
        assert_eq!(
            parse_axis_candidate("- 甘えたが", Completeness::Truncated),
            AxisCandidateReading::Truncated
        );
    }

    /// **名前を取り出せない形は、理由ごとに捨てる。**
    #[test]
    fn shapes_without_a_name_are_dropped_with_their_reason() {
        let too_long = "あ".repeat(StatAxis::MAX_CHARS + 1);
        for (raw, reason) in [
            ("", AxisCandidateMalformed::Empty),
            ("  \n \r\n\u{3000}", AxisCandidateMalformed::Empty),
            ("- ", AxisCandidateMalformed::Empty),
            ("「」", AxisCandidateMalformed::Empty),
            (
                "- 照れ屋\n- 負けず嫌い",
                AxisCandidateMalformed::SeveralLines,
            ),
            (
                "照れ屋\n恥ずかしがる様子が記憶に多いからです。",
                AxisCandidateMalformed::SeveralLines,
            ),
            ("- 項目名", AxisCandidateMalformed::Placeholder),
            ("なし", AxisCandidateMalformed::Nothing),
            ("- 「なし」", AxisCandidateMalformed::Nothing),
            // **「なし」系は末尾の句点を1つ剥がしてから一覧と比べる**（レビュー指摘 / A-9b）。
            ("なし。", AxisCandidateMalformed::Nothing),
            ("なし.", AxisCandidateMalformed::Nothing),
            ("特になし", AxisCandidateMalformed::Nothing),
            ("- 特になし。", AxisCandidateMalformed::Nothing),
            // **文字・数字を1つも含まない名前は答えの形になっていない**（レビュー指摘 / A-9b）。
            ("**", AxisCandidateMalformed::NoLetter),
            ("- **", AxisCandidateMalformed::NoLetter),
            ("--", AxisCandidateMalformed::NoLetter),
            ("***", AxisCandidateMalformed::NoLetter),
            ("*", AxisCandidateMalformed::NoLetter),
            ("「", AxisCandidateMalformed::NoLetter),
            ("…", AxisCandidateMalformed::NoLetter),
            (too_long.as_str(), AxisCandidateMalformed::NotAName),
            ("照れ\u{0007}屋", AxisCandidateMalformed::NotAName),
        ] {
            assert_eq!(
                read(raw),
                AxisCandidateReading::Malformed(reason),
                "{raw:?}"
            );
        }
        // **上限ちょうどは通る**（境目の反対側）。
        let at_limit = "あ".repeat(StatAxis::MAX_CHARS);
        assert_eq!(read(&at_limit), named(&at_limit));

        // **対照: 捨てる形の隣にある、名前として通る形。**
        // 長音符 `ー` は文字として数える（Unicode の Alphabetic）。`ー` だけの名前も通す
        // ——判定を「かな・漢字の範囲」で書き直して `ー` を落とす実装はここで止まる。
        for name in ["マイペース", "ー", "なし崩し", "特になしの人", "照れ屋。"]
        {
            assert_eq!(read(name), named(name), "{name:?} は名前として通ること");
        }
    }

    /// **敵対的な入力でも panic しない**（多バイト・制御文字・巨大な入力・印だけ）。
    #[test]
    fn the_parser_never_panics_on_hostile_input() {
        let huge = "照".repeat(100_000);
        let inputs = [
            "\u{0}",
            "-",
            "*",
            "**",
            "「",
            "」",
            "`",
            "\"",
            "- **",
            "「照れ屋",
            "照れ屋」",
            "\u{FEFF}照れ屋",
            "𠮷𠮷𠮷",
            huge.as_str(),
            "\r\r\r",
            "\u{2028}照れ屋\u{2029}",
            // 番号付きの印の断片（数字だけ・区切りだけ・全角）。
            "1",
            "1.",
            "１．",
            "9)",
            "123456789012345678901234567890.",
            "。",
            "なし。。",
        ];
        for raw in inputs {
            for completeness in [Completeness::Complete, Completeness::Truncated] {
                let _ = parse_axis_candidate(raw, completeness);
            }
        }
    }

    /// **集計キーは値ごとに異なり、外の値と一致する。**
    #[test]
    fn every_reason_has_its_own_spelling() {
        let labels: Vec<&str> = AxisCandidateMalformed::ALL
            .into_iter()
            .map(AxisCandidateMalformed::label)
            .collect();
        assert_eq!(
            labels,
            vec![
                "empty",
                "several_lines",
                "placeholder",
                "nothing",
                "no_letter",
                "not_a_name"
            ]
        );
    }
}
