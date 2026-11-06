//! **約束・決まりの提案の出力の読み・票・機械の後段**（[ADR-0051](../../docs/adr/0051-promises-and-rules.md)
//! §軽い形の模索の結果と、実装する形 —— **δ3 ＋ dedup2** / 作業単位 A-12c-1）。
//!
//! **I/O を持たない純粋関数のみ。panic しない。** 依頼の組み立ては `prompt::agreement_proposal`、
//! 走らせ方は `server::agreement_proposal`。
//!
//! # 受理集合は測った版のまま（ADR-0051 の決定 —— 一字も変えずに写す）
//!
//! | ここ | 測定の道具（`<measurements>\b60x-99981812\p1\tools\`） |
//! |---|---|
//! | [`read_detection`] | `rp3.py` の `parse_rule_lines` |
//! | [`detection_candidates`] | `rp3.py` の `Delta.detect`（`items[:3]`・`pc.find_quote`・`pc.field_of_quote`・`pc.same_utterance`） |
//! | [`read_check`] | `rp1.py` の `parse_chk`（`class_of`・`normalize_keeper`） |
//! | [`Vote`] / [`needs_another_draw`] | `rp1.py` の `vote_of` / `majority` と `rp3.py` の `Delta.check` の引き直しの判定 |
//! | [`decide`] | `rp3.py` の `Delta.check` の後半（`mind=2`・多数決・`rp1.subject`・dedup） |
//! | [`DedupRule::Plain`] | `rp2.py` の `same_content`（δ3 で測った dedup） |
//! | [`DedupRule::Stripped`] | `rp4.py` の `same_content2`（**dedup2**。本番はこちら —— [`PRODUCTION_DEDUP`]） |
//!
//! 測定の道具は Python の正規表現で読んでいた。ここは**正規表現の最短一致・後戻りの結果と同じ
//! 答えになるように手で書いた**（依存を足さない）。`\s` と `strip()` は Python の `str.isspace` の
//! 集合（[`is_space`]）、`splitlines()` は Python の行区切りの集合（[`is_line_break`]）で読む
//! （作業単位 A-12b-1 で同じ集合を作った作法を写した）。
//! **記録済みの δ3 の3通し（弥生 `yD3a`・翼 `tD3a`・`tD3b`）の出力をここに通すと、`rp3.py` と、
//! 候補 109 件の引用・欄・票・提案になったか、提案 29 件の往復・守る側・中身が全部一致した**（dedup を
//! [`DedupRule::Plain`] にして。`#[ignore]` の裏取り `tests/agreement_proposal_delta3.rs` / 作業単位 A-12c-1）。
//! [`DedupRule::Stripped`] で通すと（記録の応答を使い回す近似）、翼 `tD3a` に1件増える（往復 60 の候補 ——
//! NOTES が dedup の穴と名指しした行）。
//!
//! # 測った版から違えたところ（司令塔の採否が要る）
//!
//! * **dedup は dedup2**（[`PRODUCTION_DEDUP`]）。δ3 は dedup（[`DedupRule::Plain`]）で測った。
//!   **「δ3 ＋ dedup2」の組は測っていない**（ADR-0051 の決定のとおり）。
//! * **種類**: 検出も確かめも種類を書かない。**提案は決まり（[`PROPOSED_KIND`]）で書く**
//!   （利用者が採るときに直せる / ADR-0051 §決定2）。
//! * **守る側が読めない提案**（`曖昧:…` / `-`）: 測定の道具は提案として数えるが、保存できない
//!   （守る側は3値 / CLAUDE.md「`memory.subject` を省略しない」と同じ理由）。**落として数える**。
//! * **中身が 60 字を超える・空の提案**: `AgreementContent` を通らないので**落として数える**
//!   （削れない枠の上限 / CLAUDE.md §1）。**記号と空白しか無い中身**（[`norm`] が空になる `-` など ——
//!   中身の欄が無かった確かめの綴り）も同じく落とす（作業単位 A-12c-1 のレビューの提案1）。
//! * 上の2つで落ちた提案も、**同じ確かめの中の後ろの候補の dedup には数える**（測定の道具は
//!   落とさずに提案の一覧へ積んでいたので、その順のまま）。次の往復からは保存された行だけが「# 提案済み」と
//!   dedup の相手になる。

use std::collections::{BTreeMap, HashSet};

use crate::domain::{AgreementContent, AgreementKeeper, AgreementKind, NewAgreement};

/// **提案に付ける種類**（検出も確かめも種類を書かないため / モジュールの doc）。
pub const PROPOSED_KIND: AgreementKind = AgreementKind::Rule;

/// **本番の dedup**（ADR-0051 の決定 —— δ3 に dedup2 を足した形）。
pub const PRODUCTION_DEDUP: DedupRule = DedupRule::Stripped;

/// 検出の行で使う候補の数（`rp3.py` の `items[:3]`）。
const DETECTION_ITEMS: usize = 3;

/// **心の持ち方の語**（`rp2.py` の `MIND`。`s2-v5` が「心」に挙げた語と、その言い換え）。
///
/// 確かめの「行い」の欄にこれを含む「残す」の票を「落とす」に替える（`mind=2`）。
pub const MIND_WORDS: [&str; 14] = [
    "考え",
    "思考",
    "思う",
    "思い",
    "想",
    "感じ",
    "委ね",
    "従",
    "受け入れ",
    "受容",
    "迷",
    "信じ",
    "忘れ",
    "意識",
];

/// 重なりを見るとき落とす記号（`pc.py` の `NORM_RE` の文字クラス。空白は [`is_space`]）。
const NORM_DROP: [char; 26] = [
    '「', '」', '『', '』', '（', '）', '(', ')', '…', '・', '、', '。', '，', '．', '！', '？',
    '!', '?', '〜', '~', 'ー', '―', '-', '‥', '＊', '*',
];

/// 分類の語（`rp1.py` の `CLASS_WORDS`。**この順で前方一致を見る**）。
const CLASS_WORDS: [(&str, Class); 5] = [
    ("取り決め", Class::Agreement),
    ("持論", Class::Opinion),
    ("その場", Class::Scene),
    ("他", Class::Other),
    ("既存", Class::Existing),
];

/// **Python の `str.isspace`（＝正規表現の `\s`）と同じ集合。**
///
/// Rust の `char::is_whitespace`（Unicode の `White_Space`）に、Python だけが空白とみなす
/// U+001C〜U+001F（情報区切り）を足したもの。
#[must_use]
pub fn is_space(character: char) -> bool {
    character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character)
}

/// **Python の `str.splitlines()` が行を割る文字。**
#[must_use]
pub const fn is_line_break(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r'
            | '\u{0b}'
            | '\u{0c}'
            | '\u{1c}'
            | '\u{1d}'
            | '\u{1e}'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// 欄の区切り（`rp1.py` の `SEP_RE` の `[｜|│]`）。
const fn is_separator(character: char) -> bool {
    matches!(character, '｜' | '|' | '│')
}

/// 前後の空白を落とす（Python の `strip()`）。
fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// 出力を行に割る（Python の `splitlines()`。空の行は呼び出し側が飛ばす —— `\r\n` が空の行を
/// 1つ挟むが、空の行は読まないので答えは同じ）。
fn lines(content: &str) -> impl Iterator<Item = &str> {
    content.split(is_line_break)
}

/// **1行を欄に割る**（`[re.sub(r"\*+", "", f).strip() for f in SEP_RE.split(s)]` から空の欄を除いたもの）。
fn fields_of(line: &str) -> Vec<String> {
    line.split(is_separator)
        .map(|field| strip(&field.replace('*', "")).to_owned())
        .filter(|field| !field.is_empty())
        .collect()
}

/// 守る側の読み（`rp1.py` の `normalize_keeper` の値）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeeperReading {
    /// `利用者`
    User,
    /// `キャラクター`
    Character,
    /// `両方`
    Both,
    /// `曖昧:…`（中身は前後の空白を落とした元の綴り）。
    Unclear(String),
    /// 守る側の欄が無かった（`-`）。
    Missing,
}

impl KeeperReading {
    /// 保存できる守る側（3値）。読めなければ `None`。
    #[must_use]
    pub const fn keeper(&self) -> Option<AgreementKeeper> {
        match self {
            Self::User => Some(AgreementKeeper::User),
            Self::Character => Some(AgreementKeeper::Character),
            Self::Both => Some(AgreementKeeper::Both),
            Self::Unclear(_) | Self::Missing => None,
        }
    }
}

/// 分類の読み（`rp1.py` の `class_of` の値）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    /// `取り決め`
    Agreement,
    /// `持論`
    Opinion,
    /// `その場`
    Scene,
    /// `他`
    Other,
    /// `既存`
    Existing,
}

/// **分類の欄を読む**（`rp1.py` の `class_of`）。
///
/// 前方一致（取り決め・持論・その場・他・既存の順）、「持ち論」、綴りの崩れ（「取り」＋1〜2字 ——
/// 「取り定め」「取り分け」が測定で実際に出た）、末尾の「／分類」の順に見る。
#[must_use]
pub fn class_of(field: &str) -> Option<Class> {
    let text = strip(field);
    for (word, class) in CLASS_WORDS {
        if text.starts_with(word) {
            return Some(class);
        }
    }
    if text.starts_with("持ち論") {
        return Some(Class::Opinion);
    }
    if let Some(rest) = text.strip_prefix("取り")
        && (1..=2).contains(&rest.chars().count())
    {
        return Some(Class::Agreement);
    }
    // `[／/]\s*(取り決め|持論|その場|他|既存)\s*$`（`text` は前後の空白を落としてある）。
    for (word, class) in CLASS_WORDS {
        if let Some(before) = text.strip_suffix(word)
            && before.trim_end_matches(is_space).ends_with(['／', '/'])
        {
            return Some(class);
        }
    }
    None
}

/// **守る側の読み**（`rp1.py` の `normalize_keeper`）。`name` はキャラクターの表示名。
#[must_use]
pub fn normalize_keeper(raw: &str, name: &str) -> KeeperReading {
    let text = strip(raw);
    match text {
        "利用者" => return KeeperReading::User,
        "キャラクター" => return KeeperReading::Character,
        "両方" => return KeeperReading::Both,
        _ => {}
    }
    if text == name {
        return KeeperReading::Character;
    }
    let user = text.contains("利用者");
    let character = text.contains("キャラクター") || text.contains(name);
    let both = text.contains("両方");
    match (user, character, both) {
        (true, false, false) => KeeperReading::User,
        (false, true, false) => KeeperReading::Character,
        (false, false, true) => KeeperReading::Both,
        _ => KeeperReading::Unclear(text.to_owned()),
    }
}

/// 検出の1行（`rp3.py` の `parse_rule_lines` が作る辞書）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectionItem {
    /// 引用（鉤括弧の中。複数あれば「」「」で繋いだもの）。
    pub quote: String,
    /// 分類（**持論・その場・他・既存と明示されていなければ取り決め**）。
    pub class: Class,
    /// 守る側（読めなければ [`KeeperReading::Missing`]）。
    pub keeper: KeeperReading,
    /// 中身（最後の欄）。
    pub content: String,
}

/// 検出の1回の出力の読み。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DetectionReading {
    /// 読めた行（出てきた順）。
    pub items: Vec<DetectionItem>,
    /// 「なし」で始まる行があったか。
    pub nothing: bool,
    /// 読めなかった行の数（空の行は数えない）。
    pub unread_lines: usize,
}

/// **検出の出力を読む**（`rp3.py` の `parse_rule_lines`）。崩れても落ちない（0 件になりうる）。
///
/// 読むのは「抜｜引用｜…｜中身」の行（**欄は3つ以上**）。「抜」の欄は `[-・]?\s*抜.{0,2}`。
/// 分類の欄が持論・その場・他・既存と明示していなければ取り決めとみなす（分類の欄が抜けた行・
/// 別の語の行も読む）。守る側は最後の欄の手前のうち、**読めた最後の欄**。中身は最後の欄。
#[must_use]
pub fn read_detection(content: &str, name: &str) -> DetectionReading {
    let mut reading = DetectionReading::default();
    for line in lines(content) {
        let stripped = strip(line);
        if stripped.is_empty() {
            continue;
        }
        if stripped.starts_with("なし") {
            reading.nothing = true;
            continue;
        }
        let fields = fields_of(stripped);
        let (Some(head), Some(quote_field), Some((content, middle))) = (
            fields.first(),
            fields.get(1),
            fields.get(2..).and_then(<[String]>::split_last),
        ) else {
            reading.unread_lines += 1;
            continue;
        };
        if !is_excerpt_marker(head) {
            reading.unread_lines += 1;
            continue;
        }
        let mut class = Class::Agreement;
        for field in middle {
            if let Some(found @ (Class::Opinion | Class::Scene | Class::Other | Class::Existing)) =
                class_of(field)
            {
                class = found;
            }
        }
        let mut keeper = KeeperReading::Missing;
        for field in middle {
            let read = normalize_keeper(field, name);
            if !matches!(read, KeeperReading::Unclear(_)) {
                keeper = read;
            }
        }
        reading.items.push(DetectionItem {
            quote: bracketed_quote(quote_field),
            class,
            keeper,
            content: content.clone(),
        });
    }
    reading
}

/// 「抜」の欄か（`re.fullmatch(r"[-・]?\s*抜.{0,2}", f)`）。欄は前後の空白を落としてある。
fn is_excerpt_marker(field: &str) -> bool {
    let rest = field.strip_prefix(['-', '・']).unwrap_or(field);
    rest.trim_start_matches(is_space)
        .strip_prefix('抜')
        .is_some_and(|tail| tail.chars().count() <= 2)
}

/// **引用の欄から引用を取り出す**（`re.findall(r"「([^」]{2,})」", qt)` を「」「」で繋ぐ。
/// 無ければ欄の前後の「」を落としたもの）。
fn bracketed_quote(field: &str) -> String {
    let chars: Vec<char> = field.chars().collect();
    let mut found: Vec<String> = Vec::new();
    let mut at = 0;
    while let Some(character) = chars.get(at) {
        if *character == '「'
            && let Some(offset) = chars
                .get(at + 1..)
                .and_then(|rest| rest.iter().position(|next| *next == '」'))
            && offset >= 2
        {
            found.push(
                chars
                    .get(at + 1..at + 1 + offset)
                    .unwrap_or(&[])
                    .iter()
                    .collect(),
            );
            at += offset + 2;
            continue;
        }
        at += 1;
    }
    if found.is_empty() {
        field.trim_matches(['「', '」']).to_owned()
    } else {
        found.join("」「")
    }
}

/// **重なりを見る綴り**（`pc.norm`）。記号と空白を落とす。
#[must_use]
pub fn norm(text: &str) -> String {
    text.chars()
        .filter(|character| !is_space(*character) && !NORM_DROP.contains(character))
        .collect()
}

/// **引用を断片に割り、記号と空白を落とし、4字以上のものだけを残す**（`pc.find_quote` の `parts`）。
///
/// 割り目は `」\s*「|」\s*[、,／/]\s*「|……|…|（中略）|[｜|/／]`（この順に、左から）。
fn quote_parts(quote: &str) -> Vec<String> {
    let chars: Vec<char> = quote.chars().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut at = 0;
    while let Some(character) = chars.get(at) {
        if let Some(length) = split_length(&chars, at) {
            parts.push(std::mem::take(&mut current));
            at += length;
            continue;
        }
        current.push(*character);
        at += 1;
    }
    parts.push(current);
    parts
        .iter()
        .map(|part| norm(part))
        .filter(|part| part.chars().count() >= 4)
        .collect()
}

/// `at` から割り目が始まるなら、その長さ（文字数）。
fn split_length(chars: &[char], at: usize) -> Option<usize> {
    let character = *chars.get(at)?;
    match character {
        '」' => {
            // `」\s*「` → `」\s*[、,／/]\s*「`（どちらも `\s*` は貪欲。後ろの文字と重ならない）。
            let after = at + 1 + spaces_from(chars, at + 1);
            if chars.get(after) == Some(&'「') {
                return Some(after + 1 - at);
            }
            if chars
                .get(after)
                .is_some_and(|next| matches!(next, '、' | ',' | '／' | '/'))
            {
                let opening = after + 1 + spaces_from(chars, after + 1);
                if chars.get(opening) == Some(&'「') {
                    return Some(opening + 1 - at);
                }
            }
            None
        }
        '…' => Some(if chars.get(at + 1) == Some(&'…') {
            2
        } else {
            1
        }),
        '（' => {
            let tail: String = chars.get(at..at + 4)?.iter().collect();
            (tail == "（中略）").then_some(4)
        }
        '｜' | '|' | '/' | '／' => Some(1),
        _ => None,
    }
}

/// `at` から続く空白の数。
fn spaces_from(chars: &[char], at: usize) -> usize {
    chars
        .get(at..)
        .map_or(0, |rest| rest.iter().take_while(|c| is_space(**c)).count())
}

/// **引用がその往復の本文にあるか**（`pc.find_quote` を往復1つで。断片が**すべて**、記号と空白を落とした
/// 「利用者の発言 ＋ 応答」の綴りに含まれること。4字以上の断片が1つも無ければ「無い」）。
#[must_use]
pub fn quote_is_in_turn(quote: &str, user: &str, character: &str) -> bool {
    let parts = quote_parts(quote);
    if parts.is_empty() {
        return false;
    }
    let hay = norm(&format!("{user}{character}"));
    parts.iter().all(|part| hay.contains(part.as_str()))
}

/// **引用が利用者の発言から取られたか**（`pc.field_of_quote`。断片がすべて利用者の発言にあれば利用者）。
#[must_use]
pub fn quote_is_from_user(quote: &str, user: &str) -> bool {
    let parts = quote_parts(quote);
    let hay = norm(user);
    !parts.is_empty() && parts.iter().all(|part| hay.contains(part.as_str()))
}

/// 最長の共通部分文字列の長さ（文字数 / `pc.lcs_len`）。
fn longest_common_substring(left: &[char], right: &[char]) -> usize {
    let mut best = 0;
    let mut previous = vec![0_usize; right.len() + 1];
    for left_char in left {
        let mut current = vec![0_usize; right.len() + 1];
        for (index, right_char) in right.iter().enumerate() {
            if left_char == right_char {
                let run = previous.get(index).copied().unwrap_or(0) + 1;
                if let Some(slot) = current.get_mut(index + 1) {
                    *slot = run;
                }
                best = best.max(run);
            }
        }
        previous = current;
    }
    best
}

/// **同じ発言からの引用か**（`pc.same_utterance`）。記号と空白を落とした綴りで、一方が他方を含むか、
/// 共通部分が `min(8, max(4, 短い側の半分))` 字以上。
#[must_use]
pub fn same_utterance(left: &str, right: &str) -> bool {
    let left = norm(left);
    let right = norm(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left.contains(right.as_str()) || right.contains(left.as_str()) {
        return true;
    }
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let shorter = left.len().min(right.len());
    // `min(8, max(4, 短い側 // 2))`（4 ≤ 8 なので clamp と同じ）。
    longest_common_substring(&left, &right) >= (shorter / 2).clamp(4, 8)
}

/// **確かめにかける候補1件**（検出が取り決めと書き、引用がその往復の本文に見つかったもの）。
///
/// **中身はどれもモデルの出力で、信頼できない入力である**（依頼に戻すときは `prompt` が無害化する）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// 引用（検出が書いた綴り）。
    pub quote: String,
    /// 引用が利用者の発言から取られたか。
    pub from_user: bool,
    /// 検出の守る側（**1票目**）。
    pub detected_keeper: KeeperReading,
    /// 検出の中身（1票目が勝ったときに使う）。
    pub detected_content: String,
}

/// 検出から候補を作った結果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DetectionCandidates {
    /// 候補（出てきた順）。
    pub candidates: Vec<Candidate>,
    /// 引用がその往復の本文に無かったので捨てた行の数（先頭の3行のうち）。
    pub quote_missing: usize,
    /// 分類が取り決めでなかったので捨てた行の数（同上）。
    pub not_agreement: usize,
    /// 同じ往復の先の候補と同じ発言だったので1つにまとめた数（同上）。
    pub merged: usize,
}

/// **検出の読みから候補を作る**（`rp3.py` の `Delta.detect`）。
///
/// 使うのは**先頭の3行だけ**。引用がその往復の本文にあり（[`quote_is_in_turn`]）、分類が取り決めの
/// 行だけを候補にする。**同じ往復の中で引用が重なるもの**（[`same_utterance`]）は先に出たものだけ。
#[must_use]
pub fn detection_candidates(
    reading: &DetectionReading,
    user: &str,
    character: &str,
) -> DetectionCandidates {
    let mut outcome = DetectionCandidates::default();
    for item in reading.items.iter().take(DETECTION_ITEMS) {
        if !quote_is_in_turn(&item.quote, user, character) {
            outcome.quote_missing += 1;
            continue;
        }
        if item.class != Class::Agreement {
            outcome.not_agreement += 1;
            continue;
        }
        if outcome
            .candidates
            .iter()
            .any(|kept| same_utterance(&kept.quote, &item.quote))
        {
            outcome.merged += 1;
            continue;
        }
        outcome.candidates.push(Candidate {
            quote: item.quote.clone(),
            from_user: quote_is_from_user(&item.quote, user),
            detected_keeper: item.keeper.clone(),
            detected_content: item.content.clone(),
        });
    }
    outcome
}

/// 確かめの判定（`残す` / `落とす` / `既存`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judgement {
    /// `残す`
    Keep,
    /// `落とす`
    Drop,
    /// `既存`
    Existing,
}

/// 確かめの1行の読み（`rp1.py` の `parse_chk` が作る辞書）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckVerdict {
    /// 行い（番号と分類の間の欄。無ければ `-`）。
    pub act: String,
    /// 分類。
    pub class: Class,
    /// 守る側（分類と判定の間に欄が無ければ [`KeeperReading::Missing`]）。
    pub keeper: KeeperReading,
    /// 判定。
    pub judgement: Judgement,
    /// 中身（判定の後ろの欄。無ければ `-`）。
    pub content: String,
}

/// 確かめの1回の出力の読み。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CheckReading {
    /// 一覧の番号（1 から）ごとの読み。
    pub by_number: BTreeMap<usize, CheckVerdict>,
    /// 読めなかった行の数（空の行は数えない）。
    pub unread_lines: usize,
}

/// **確かめの出力を読む**（`rp1.py` の `parse_chk`）。崩れても落ちない。
///
/// 「点検｜番号｜行い｜分類｜守る側｜判定｜中身」を、**欄の数を数えずに**読む: 判定は3つ目以降の欄で
/// 「残す」「落と」「既存」で始まる最初の欄、分類はその手前で分類として読める最後の欄。
/// **番号は 1〜`marks`、同じ番号は先に読んだほうだけ。**
#[must_use]
pub fn read_check(content: &str, name: &str, marks: usize) -> CheckReading {
    let mut reading = CheckReading::default();
    for line in lines(content) {
        let stripped = strip(line);
        if stripped.is_empty() {
            continue;
        }
        let mut fields = fields_of(stripped);
        if fields.first().is_some_and(|first| {
            first
                .strip_prefix('点')
                .is_some_and(|tail| tail.chars().count() <= 2)
        }) {
            fields.remove(0);
        }
        match check_line(&fields, name, marks, &reading) {
            Some((number, verdict)) => {
                reading.by_number.insert(number, verdict);
            }
            None => reading.unread_lines += 1,
        }
    }
    reading
}

/// 欄に割った確かめの1行を読む。読めなければ `None`（呼び出し側が数える）。
fn check_line(
    fields: &[String],
    name: &str,
    marks: usize,
    read: &CheckReading,
) -> Option<(usize, CheckVerdict)> {
    let number_field = fields.first()?;
    if number_field.is_empty() || !number_field.chars().all(is_digit) {
        return None;
    }
    let verdict_at = (2..fields.len()).find(|&index| {
        fields.get(index).is_some_and(|field| {
            field.starts_with("残す") || field.starts_with("落と") || field.starts_with("既存")
        })
    })?;
    let class_at = (1..verdict_at).rev().find(|&index| {
        fields
            .get(index)
            .and_then(|field| class_of(field))
            .is_some()
    })?;
    let number = number_of(number_field)?;
    if !(1..=marks).contains(&number) || read.by_number.contains_key(&number) {
        return None;
    }
    let class = class_of(fields.get(class_at)?)?;
    let verdict_field = fields.get(verdict_at)?;
    let judgement = if verdict_field.starts_with("既存") || class == Class::Existing {
        Judgement::Existing
    } else if verdict_field.starts_with("残す") {
        Judgement::Keep
    } else {
        Judgement::Drop
    };
    let keeper = if verdict_at > class_at + 1 {
        normalize_keeper(&fields.get(class_at + 1..verdict_at)?.join(" "), name)
    } else {
        KeeperReading::Missing
    };
    let content = strip(&fields.get(verdict_at + 1..).unwrap_or(&[]).join(" ")).to_owned();
    let act = fields.get(1..class_at).unwrap_or(&[]).join(" ");
    Some((
        number,
        CheckVerdict {
            act: if act.is_empty() { "-".to_owned() } else { act },
            class,
            keeper,
            judgement,
            content: if content.is_empty() {
                "-".to_owned()
            } else {
                content
            },
        },
    ))
}

/// 番号の数字（`[0-9０-９]`）。
fn is_digit(character: char) -> bool {
    character.is_ascii_digit() || ('０'..='９').contains(&character)
}

/// 数字の綴りを番号にする（全角は半角に）。**桁あふれは `None`**（一覧の外として扱う —— Python は
/// 大きな整数のまま範囲の外に落ちるので、答えは同じ）。
fn number_of(text: &str) -> Option<usize> {
    text.chars().try_fold(0_usize, |acc, character| {
        let digit = if character.is_ascii_digit() {
            character.to_digit(10)?
        } else {
            u32::from(character).checked_sub(u32::from('０'))?
        };
        acc.checked_mul(10)?
            .checked_add(usize::try_from(digit).ok()?)
    })
}

/// **1票**（`rp1.py` の `vote_of` の値）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Vote {
    /// 残す（取り決め）。守る側つき —— **守る側が違えば別の票**。
    Keep(KeeperReading),
    /// 落とす（取り決めでない・判定が落とす）。
    Drop,
    /// 既存（「# 提案済み」と同じ）。
    Existing,
    /// 無判定（その番号の行が読めなかった）。
    Unjudged,
}

impl Vote {
    /// 確かめの1行を票にする（`rp1.py` の `vote_of`）。
    #[must_use]
    pub fn of(verdict: Option<&CheckVerdict>) -> Self {
        match verdict {
            None => Self::Unjudged,
            Some(verdict)
                if verdict.judgement == Judgement::Keep && verdict.class == Class::Agreement =>
            {
                Self::Keep(verdict.keeper.clone())
            }
            Some(verdict) if verdict.judgement == Judgement::Existing => Self::Existing,
            Some(_) => Self::Drop,
        }
    }
}

/// **2票以上そろった票**（`rp1.py` の `majority`。並びの先にあるものから見る）。無ければ `None`。
#[must_use]
pub fn majority(votes: &[Vote]) -> Option<&Vote> {
    votes
        .iter()
        .find(|vote| votes.iter().filter(|other| other == vote).count() >= 2)
}

/// **確かめをもう1本引くか**（`rp3.py` の `Delta.check` —— 1本目の確かめと検出の2票で、
/// 多数の決まらない候補が1つでもあれば引く）。**心の語の読み替えは、ここでは掛けない**（測定の道具の順）。
#[must_use]
pub fn needs_another_draw(candidates: &[Candidate], first: &CheckReading) -> bool {
    candidates.iter().enumerate().any(|(index, candidate)| {
        let votes = [
            Vote::Keep(candidate.detected_keeper.clone()),
            Vote::of(first.by_number.get(&(index + 1))),
        ];
        majority(&votes).is_none()
    })
}

/// 行いの欄に心の語があるか（`rp2.py` の `is_mind` を `{"act": act, "text": ""}` で —— `mind=2`）。
#[must_use]
pub fn is_mind(act: &str) -> bool {
    MIND_WORDS.iter().any(|word| act.contains(word))
}

/// **中身の主語**（`rp1.py` の `subject` —— `comb3.py` の写し）。読めなければ `None`。
#[must_use]
pub fn subject(content: &str, name: &str) -> Option<KeeperReading> {
    let text = strip(content);
    let both = [
        "二人".to_owned(),
        "ふたり".to_owned(),
        format!("利用者と{name}"),
        format!("{name}と利用者"),
        "両者".to_owned(),
        "両方".to_owned(),
    ];
    if both.iter().any(|prefix| text.starts_with(prefix.as_str())) {
        return Some(KeeperReading::Both);
    }
    if text.starts_with("利用者") {
        return Some(KeeperReading::User);
    }
    if text.starts_with(name) {
        return Some(KeeperReading::Character);
    }
    None
}

/// **dedup の形**（中身どうしが同じ取り決めかの比べ方）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupRule {
    /// δ3 で測った形（`rp2.py` の `same_content`）。主語と名前を外し、2文字の組の重なりが短い側の 0.6 以上。
    Plain,
    /// **dedup2**（`rp4.py` の `same_content2`）。主語と名前に加えて、「許しが出るまで」の形の条件と
    /// 文末の言い回し（〜してはいけない・〜しない など）を外してから比べる。
    Stripped,
}

impl DedupRule {
    /// **2つの中身が同じ取り決めか。**
    #[must_use]
    pub fn same(self, left: &str, right: &str, name: &str) -> bool {
        let core = |text: &str| match self {
            Self::Plain => without_subjects(text, name),
            Self::Stripped => strip_ending(&strip_condition(&without_subjects(text, name))),
        };
        let left = bigrams(&core(left));
        let right = bigrams(&core(right));
        if left.is_empty() || right.is_empty() {
            return false;
        }
        // `len(A & B) / min(len(A), len(B)) >= 0.6` を整数で（5 倍して 3 倍と比べる。分母は 0 でない）。
        let shared = left.intersection(&right).count();
        shared * 5 >= left.len().min(right.len()) * 3
    }
}

/// 記号と空白を落とし、主語と名前を外す（`rp2.same_content` の `core`。**この順に**全部置き換える）。
fn without_subjects(text: &str, name: &str) -> String {
    let mut core = norm(text);
    for word in [
        "利用者は".to_owned(),
        format!("{name}は"),
        "利用者".to_owned(),
        name.to_owned(),
        "キャラクター".to_owned(),
    ] {
        if !word.is_empty() {
            core = core.replace(word.as_str(), "");
        }
    }
    core
}

/// 2文字の組の集合（`rp2.bigrams`）。
fn bigrams(text: &str) -> HashSet<(char, char)> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .windows(2)
        .filter_map(|pair| match pair {
            [first, second] => Some((*first, *second)),
            _ => None,
        })
        .collect()
}

/// 「許しが出るまで」の形の条件の語（`rp4.py` の `COND_RE` の各群。**この順に後戻りする**）。
const CONDITION_CORE: [&str; 2] = ["許し", "許可"];
const CONDITION_PARTICLE: [&str; 2] = ["が", "を"];
const CONDITION_VERB: [&str; 7] = [
    "出る",
    "出ない",
    "下りる",
    "与えられる",
    "来る",
    "得る",
    "もらう",
];
const CONDITION_END: [&str; 4] = ["まで", "なく", "なしに", "限り"];

/// **「許しが出るまで」の形の条件を外す**（`COND_RE.sub("", x)`。
/// `(の)?(許し|許可)(が|を)?(出る|…|もらう)?(まで|なく|なしに|限り)` を左から、重ならずに）。
fn strip_condition(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut kept = String::new();
    let mut at = 0;
    while let Some(character) = chars.get(at) {
        if let Some(length) = chars.get(at..).and_then(condition_at) {
            at += length;
            continue;
        }
        kept.push(*character);
        at += 1;
    }
    kept
}

/// 文字の並びの先頭が `word` なら、その後ろ。
fn after_word<'a>(chars: &'a [char], word: &str) -> Option<&'a [char]> {
    let mut rest = chars;
    for expected in word.chars() {
        let (first, tail) = rest.split_first()?;
        if *first != expected {
            return None;
        }
        rest = tail;
    }
    Some(rest)
}

/// 先頭から条件の形が始まるなら、その文字数（**正規表現の後戻りの順で最初に当たる形**）。
///
/// 後戻りの順は「の」を読む → 読まない、核の語、助詞（が・を・無し）、動詞（7つ・無し）、終わりの語（4つ）。
fn condition_at(chars: &[char]) -> Option<usize> {
    for leading in [Some("の"), None] {
        let after_leading = match leading {
            Some(word) => match after_word(chars, word) {
                Some(rest) => rest,
                None => continue,
            },
            None => chars,
        };
        for core in CONDITION_CORE {
            let Some(after_core) = after_word(after_leading, core) else {
                continue;
            };
            for particle in CONDITION_PARTICLE.map(Some).into_iter().chain([None]) {
                let after_particle = match particle {
                    Some(word) => match after_word(after_core, word) {
                        Some(rest) => rest,
                        None => continue,
                    },
                    None => after_core,
                };
                for verb in CONDITION_VERB.map(Some).into_iter().chain([None]) {
                    let after_verb = match verb {
                        Some(word) => match after_word(after_particle, word) {
                            Some(rest) => rest,
                            None => continue,
                        },
                        None => after_particle,
                    };
                    for end in CONDITION_END {
                        if let Some(after_end) = after_word(after_verb, end) {
                            return Some(chars.len() - after_end.len());
                        }
                    }
                }
            }
        }
    }
    None
}

/// 文末の言い回し（`rp4.py` の `END_RE` の語。**`(…)+$`** —— 語をいくつ繋いでもよい）。
const ENDINGS: [&str; 14] = [
    "しなければならない",
    "しなければいけない",
    "してはいけない",
    "してはならない",
    "ではいけない",
    "てはいけない",
    "てはならない",
    "しないこと",
    "しない",
    "させない",
    "ない",
    "する",
    "こと",
    "べき",
];

/// **文末の言い回しを外す**（`END_RE.sub("", x)`）。
///
/// 正規表現は左から始点を探すので、**語を1つ以上繋いで末尾まで届く、最も左の始点**から後ろを落とす。
fn strip_ending(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    // `reaches_end[i]`: i 文字目から末尾まで、語を 0 個以上繋いで届くか（後ろから決める）。
    let mut reaches_end = vec![false; chars.len() + 1];
    if let Some(end) = reaches_end.last_mut() {
        *end = true;
    }
    for start in (0..chars.len()).rev() {
        let tail = chars.get(start..).unwrap_or(&[]);
        let reaches = ENDINGS.iter().any(|word| {
            after_word(tail, word).is_some_and(|rest| {
                reaches_end
                    .get(chars.len() - rest.len())
                    .copied()
                    .unwrap_or(false)
            })
        });
        if let Some(slot) = reaches_end.get_mut(start) {
            *slot = reaches;
        }
    }
    // **語を1つ以上**繋いで届く、最も左の始点（その始点は末尾より前）。
    let start = (0..chars.len()).find(|&start| reaches_end.get(start).copied().unwrap_or(false));
    match start {
        Some(start) => chars.get(..start).unwrap_or(&[]).iter().collect(),
        None => text.to_owned(),
    }
}

/// 1つの候補の結末（`rp3.py` の `Delta.check` の後半）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// 多数の票が「残す」でなかった（多数が無い・落とす・既存・無判定）。
    NotKept,
    /// 提案済み（または同じ確かめの先の提案）と同じ中身だった（dedup）。
    Deduped,
    /// 中身の主語が守る側と合わなかった。
    SubjectMismatch,
    /// **提案する**（測定の道具が提案の一覧に積んだもの）。保存できるかは [`proposal_to_save`] が決める。
    Proposed {
        /// 守る側（勝った票のもの）。
        keeper: KeeperReading,
        /// 中身（勝った票のうち**最初の回**のもの。検出が先頭）。
        content: String,
    },
}

/// 確かめの票の内訳（ログに出す件数）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Decided {
    /// 候補ごとの結末（候補の順）。
    pub outcomes: Vec<Outcome>,
    /// 心の語で「落とす」に替えた票の数。
    pub mind_flips: usize,
}

/// **確かめの票を数え、機械の後段を通す**（`rp3.py` の `Delta.check` の後半。`vote=2of3`・`mind=2`）。
///
/// 候補ごとに:
///
/// 1. 票 = 検出（残す・検出の守る側）＋ 確かめの各回（[`Vote::of`]）。
/// 2. **mind=2**: 確かめの「行い」の欄に心の語があれば、その確かめの「残す」の票を「落とす」に替える
///    （検出の票と中身には掛けない）。
/// 3. 2票以上そろった票（[`majority`]）が「残す」なら、中身は**その票を入れた最初の回**のもの。
/// 4. **dedup**: `existing`（提案済み）か、同じ確かめで先に提案したものと同じ中身なら出さない。
/// 5. **主語の照合**: 中身の主語が読めて、守る側と違えば出さない。
///
/// `existing` は「# 提案済み」の元（そのキャラクターの提案中・却下・有効の中身。件数で切らない）。
#[must_use]
pub fn decide(
    candidates: &[Candidate],
    draws: &[CheckReading],
    name: &str,
    existing: &[&str],
    dedup: DedupRule,
) -> Decided {
    let mut decided = Decided::default();
    let mut proposed: Vec<String> = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let number = index + 1;
        let verdicts: Vec<Option<&CheckVerdict>> = draws
            .iter()
            .map(|draw| draw.by_number.get(&number))
            .collect();
        let (votes, flips) = votes_of(candidate, &verdicts);
        decided.mind_flips += flips;
        let Some(winner @ Vote::Keep(keeper)) = majority(&votes) else {
            decided.outcomes.push(Outcome::NotKept);
            continue;
        };
        let first = votes.iter().position(|vote| vote == winner).unwrap_or(0);
        let content = if first == 0 {
            candidate.detected_content.clone()
        } else {
            verdicts
                .get(first - 1)
                .copied()
                .flatten()
                .map_or_else(|| "-".to_owned(), |verdict| verdict.content.clone())
        };
        let duplicate = existing
            .iter()
            .copied()
            .chain(proposed.iter().map(String::as_str))
            .any(|earlier| dedup.same(earlier, &content, name));
        if duplicate {
            decided.outcomes.push(Outcome::Deduped);
            continue;
        }
        if subject(&content, name).is_some_and(|read| &read != keeper) {
            decided.outcomes.push(Outcome::SubjectMismatch);
            continue;
        }
        proposed.push(content.clone());
        decided.outcomes.push(Outcome::Proposed {
            keeper: keeper.clone(),
            content,
        });
    }
    decided
}

/// **1つの候補の票**（検出の1票 ＋ 確かめの各回。**mind=2 の読み替えを掛けた後**）と、読み替えた票の数。
///
/// `verdicts` はその候補の番号の、確かめの各回の読み（引いた順。読めなかった回は `None`）。
#[must_use]
pub fn votes_of(candidate: &Candidate, verdicts: &[Option<&CheckVerdict>]) -> (Vec<Vote>, usize) {
    let mut flips = 0;
    let mut votes = vec![Vote::Keep(candidate.detected_keeper.clone())];
    for verdict in verdicts {
        let vote = Vote::of(*verdict);
        if matches!(vote, Vote::Keep(_)) && verdict.is_some_and(|v| is_mind(&v.act)) {
            flips += 1;
            votes.push(Vote::Drop);
        } else {
            votes.push(vote);
        }
    }
    (votes, flips)
}

/// 提案を保存できなかった理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsaveable {
    /// 守る側が3値のどれでもない（`曖昧:…` / `-`）。
    UnclearKeeper,
    /// 中身が `AgreementContent` を通らない（空・60 字超・制御文字）か、記号と空白しか無い（`-` など）。
    InvalidContent,
}

/// **提案を保存する形にする**（モジュールの doc「違えたところ」）。種類は [`PROPOSED_KIND`]。
pub fn proposal_to_save(keeper: &KeeperReading, content: &str) -> Result<NewAgreement, Unsaveable> {
    let keeper = keeper.keeper().ok_or(Unsaveable::UnclearKeeper)?;
    // **記号と空白しか無い中身（`-` など —— 中身の欄が無かった確かめの綴り）は書かない**
    // （`AgreementContent` は通るが、取り決めとして読めない / 作業単位 A-12c-1 のレビューの提案1）。
    if norm(content).is_empty() {
        return Err(Unsaveable::InvalidContent);
    }
    let content = AgreementContent::parse(content).map_err(|_| Unsaveable::InvalidContent)?;
    Ok(NewAgreement {
        content,
        kind: PROPOSED_KIND,
        keeper,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// キャラクターの表示名（守る側の欄・主語・dedup が読む）。
    const NAME: &str = "葵";

    fn keep(keeper: KeeperReading) -> Vote {
        Vote::Keep(keeper)
    }

    // -----------------------------------------------------------------
    // 読み（期待値は測定の道具に同じ入力を通して出したもの / 作業単位 A-12c-1）
    // -----------------------------------------------------------------

    /// **分類の欄の読みは `rp1.class_of` と同じ**（期待値は `rp1.class_of` に同じ綴りを通した結果）。
    #[test]
    fn the_class_reading_is_the_measured_one() {
        for (field, expected) in [
            ("取り決め", Some(Class::Agreement)),
            ("取り定め", Some(Class::Agreement)),
            ("取り分け", Some(Class::Agreement)),
            ("取り決めた", Some(Class::Agreement)),
            ("持ち論です", Some(Class::Opinion)),
            ("他人", Some(Class::Other)),
            ("動かない／その場", Some(Class::Scene)),
            ("行い / 持論", Some(Class::Opinion)),
            ("その他", None),
            ("既存の約束", Some(Class::Existing)),
            ("心", None),
            ("取り", None),
            ("x/他 ", Some(Class::Other)),
            ("取りきめた", None),
        ] {
            assert_eq!(class_of(field), expected, "{field:?}");
        }
    }

    /// **守る側の読みは `rp1.normalize_keeper` と同じ**（名前はキャラクター。2つ以上読めたら曖昧）。
    #[test]
    fn the_keeper_reading_is_the_measured_one() {
        for (raw, expected) in [
            ("利用者", KeeperReading::User),
            ("葵", KeeperReading::Character),
            (
                "利用者（葵に）",
                KeeperReading::Unclear("利用者（葵に）".to_owned()),
            ),
            ("両方", KeeperReading::Both),
            (" キャラクター ", KeeperReading::Character),
            (
                "利用者と葵",
                KeeperReading::Unclear("利用者と葵".to_owned()),
            ),
            ("-", KeeperReading::Unclear("-".to_owned())),
            ("", KeeperReading::Unclear(String::new())),
            (
                "両方の利用者",
                KeeperReading::Unclear("両方の利用者".to_owned()),
            ),
        ] {
            assert_eq!(normalize_keeper(raw, NAME), expected, "{raw:?}");
        }
    }

    fn item(quote: &str, class: Class, keeper: KeeperReading, content: &str) -> DetectionItem {
        DetectionItem {
            quote: quote.to_owned(),
            class,
            keeper,
            content: content.to_owned(),
        }
    }

    /// **検出の行の読みは `rp3.parse_rule_lines` と同じ**（期待値はその関数に同じ行を通した結果）。
    #[test]
    fn the_detection_reading_is_the_measured_one() {
        for (line, expected) in [
            (
                "抜｜「もう出かけない」｜取り決め｜利用者｜利用者は出かけない",
                vec![item(
                    "もう出かけない",
                    Class::Agreement,
                    KeeperReading::User,
                    "利用者は出かけない",
                )],
            ),
            (
                "- 抜｜「a」｜持論｜利用者｜x",
                vec![item("a", Class::Opinion, KeeperReading::User, "x")],
            ),
            // 分類の欄が抜けた行も取り決めとして読む（dD2 で実際に出た）。
            (
                "抜き｜「もう出かけない」｜利用者｜利用者は出かけない",
                vec![item(
                    "もう出かけない",
                    Class::Agreement,
                    KeeperReading::User,
                    "利用者は出かけない",
                )],
            ),
            // 欄が3つだけ: 守る側は無く、中身は最後の欄。
            (
                "抜｜「もう出かけない」｜取り決め",
                vec![item(
                    "もう出かけない",
                    Class::Agreement,
                    KeeperReading::Missing,
                    "取り決め",
                )],
            ),
            // 守る側は読めた最後の欄。
            (
                "抜｜「もう出かけない」｜その場｜両方｜葵｜中身",
                vec![item(
                    "もう出かけない",
                    Class::Scene,
                    KeeperReading::Character,
                    "中身",
                )],
            ),
            ("抜｜「q」", vec![]),
            ("x｜y｜z", vec![]),
        ] {
            assert_eq!(read_detection(line, NAME).items, expected, "{line:?}");
        }
        let nothing = read_detection("なし", NAME);
        assert!(nothing.nothing && nothing.items.is_empty());
        assert_eq!(
            read_detection("抜｜「q」\nx｜y｜z\n\n", NAME).unread_lines,
            2
        );
    }

    /// **引用の取り出しは `re.findall(r"「([^」]{2,})」")` と同じ**（1字の鉤括弧は飛ばす・複数は繋ぐ）。
    #[test]
    fn the_quote_is_taken_from_the_brackets_like_the_measured_tool() {
        for (field, expected) in [
            ("「もう黙って出かけない」", "もう黙って出かけない"),
            (
                "「これからは」「名前で呼んで」",
                "これからは」「名前で呼んで",
            ),
            ("「あ」", "あ"),
            ("そのまま", "そのまま"),
            ("「これからは……名前で呼んで」", "これからは……名前で呼んで"),
            ("a」 、 「b", "a」 、 「b"),
            ("「あ」「いう」", "いう"),
        ] {
            assert_eq!(bracketed_quote(field), expected, "{field:?}");
        }
    }

    /// **引用の照合は `pc.find_quote` と同じ**（断片は「…」などで割り、4字以上の断片が**すべて**本文にあること）。
    #[test]
    fn the_quote_lookup_is_the_measured_one() {
        for (quote, hay, expected) in [
            ("もう黙って出かけない", "私はもう黙って出かけないよ", true),
            (
                "もう黙って……出かけない",
                "もう黙ってるけど、出かけないよ",
                true,
            ),
            ("出かけ", "出かけない", false),
            (
                "もう、黙って（中略）出かけない",
                "もう黙って、絶対に出かけない",
                true,
            ),
            ("「先生」「名前で呼んで」", "先生、名前で呼んで", true),
            ("先生｜名前で呼んで", "先生と名前で呼んで", true),
            ("先生」、「名前で呼んで", "先生名前で呼んで", true),
            // 片方の断片が無ければ「無い」。
            ("もう黙って……海へ行かない", "もう黙っている", false),
        ] {
            assert_eq!(quote_is_in_turn(quote, hay, ""), expected, "{quote:?}");
        }
        // 利用者の発言と応答は**繋いで**照合する（`user + assistant`）。欄は利用者の発言だけで決める。
        assert!(quote_is_in_turn("出かけないで", "もう出かけ", "ないで"));
        assert!(quote_is_from_user("もう出かけない", "私はもう出かけない"));
        assert!(!quote_is_from_user("もう出かけない", "私はもう"));
    }

    /// **同じ発言かの照合は `pc.same_utterance` と同じ。**
    #[test]
    fn the_same_utterance_rule_is_the_measured_one() {
        for (left, right, expected) in [
            ("もう黙って出かけない", "黙って出かけない", true),
            ("名前で呼んでください", "これからは名前で呼んで", true),
            ("あいうえおかきくけこ", "かきくけこさしすせそ", true),
            ("abcd", "wxyz", false),
            (
                "あいうえおかきくけこさしすせそ",
                "たちつてとあいうえおかきなにぬねの",
                true,
            ),
            ("", "なにか", false),
        ] {
            assert_eq!(same_utterance(left, right), expected, "{left:?} {right:?}");
        }
    }

    fn verdict(
        act: &str,
        class: Class,
        keeper: KeeperReading,
        judgement: Judgement,
        content: &str,
    ) -> CheckVerdict {
        CheckVerdict {
            act: act.to_owned(),
            class,
            keeper,
            judgement,
            content: content.to_owned(),
        }
    }

    /// **確かめの行の読みは `rp1.parse_chk` と同じ**（期待値はその関数に同じ行を通した結果。一覧は2件）。
    #[test]
    fn the_check_reading_is_the_measured_one() {
        for (line, expected) in [
            (
                "点検｜1｜戸棚を開ける｜取り決め｜利用者｜残す｜利用者は戸棚を開けない",
                Some((
                    1,
                    verdict(
                        "戸棚を開ける",
                        Class::Agreement,
                        KeeperReading::User,
                        Judgement::Keep,
                        "利用者は戸棚を開けない",
                    ),
                )),
            ),
            (
                "点検｜1｜心｜持論｜利用者｜落とす｜-",
                Some((
                    1,
                    verdict(
                        "心",
                        Class::Opinion,
                        KeeperReading::User,
                        Judgement::Drop,
                        "-",
                    ),
                )),
            ),
            (
                "1｜戸棚を開ける｜取り定め｜利用者｜残す｜利用者は戸棚を開けない",
                Some((
                    1,
                    verdict(
                        "戸棚を開ける",
                        Class::Agreement,
                        KeeperReading::User,
                        Judgement::Keep,
                        "利用者は戸棚を開けない",
                    ),
                )),
            ),
            (
                "点検｜1｜戸棚を開ける／その場｜残す｜-",
                Some((
                    1,
                    verdict(
                        "-",
                        Class::Scene,
                        KeeperReading::Missing,
                        Judgement::Keep,
                        "-",
                    ),
                )),
            ),
            (
                "点検｜2｜戸棚｜取り決め｜残す",
                Some((
                    2,
                    verdict(
                        "戸棚",
                        Class::Agreement,
                        KeeperReading::Missing,
                        Judgement::Keep,
                        "-",
                    ),
                )),
            ),
            ("点検｜3｜戸棚｜取り決め｜利用者｜残す｜x", None),
            (
                "点検｜１｜戸棚｜取り決め｜葵｜既存｜x",
                Some((
                    1,
                    verdict(
                        "戸棚",
                        Class::Agreement,
                        KeeperReading::Character,
                        Judgement::Existing,
                        "x",
                    ),
                )),
            ),
            ("**点検**｜1｜戸棚｜既存｜利用者｜残す｜x", None),
            (
                "点検 | 1 | a | b | 取り決め | 利用者 | 残す | 中 | 身",
                Some((
                    1,
                    verdict(
                        "a b",
                        Class::Agreement,
                        KeeperReading::User,
                        Judgement::Keep,
                        "中 身",
                    ),
                )),
            ),
        ] {
            let read = read_check(line, NAME, 2);
            let got = read
                .by_number
                .iter()
                .next()
                .map(|(number, verdict)| (*number, verdict.clone()));
            assert_eq!(got, expected, "{line:?}");
            assert_eq!(
                read.unread_lines,
                usize::from(expected.is_none()),
                "{line:?}"
            );
        }
        // **同じ番号は先に読んだほうだけ**（後の行は読めなかった行に数える）。
        let read = read_check(
            "点検｜1｜a｜取り決め｜利用者｜残す｜先\n点検｜1｜b｜取り決め｜両方｜残す｜後",
            NAME,
            2,
        );
        assert_eq!(read.by_number.len(), 1);
        assert_eq!(
            read.by_number
                .get(&1)
                .map(|verdict| verdict.content.as_str()),
            Some("先")
        );
        assert_eq!(read.unread_lines, 1);
    }

    /// **崩れた出力では panic せず、候補も判定も 0 件で終わる**（パーサは絶対に落ちない / CLAUDE.md）。
    #[test]
    fn broken_outputs_never_panic_and_leave_nothing() {
        let garbage = [
            "",
            "\u{0}\u{1b}[31m",
            "｜｜｜｜",
            "抜｜｜｜",
            "抜｜「",
            "抜｜「」」」｜取り決め｜｜",
            "点検｜99999999999999999999999999｜a｜取り決め｜利用者｜残す｜x",
            "点検｜０｜a｜取り決め｜利用者｜残す｜x",
            "点｜｜残す",
            "\r\n\r\n\u{2028}\u{85}",
            "抜｜「…」｜取り決め｜利用者｜…",
            "抜｜（中略）｜取り決め｜利用者｜x",
            // 割り目の「（中略）」を読みかけて、引用の終わりに届く形（4字先が無い）。
            "抜｜「ab（」｜取り決め｜利用者｜x",
            "抜｜ab（｜取り決め｜利用者｜x",
            "𠮷𠮷𠮷｜「𠮷」｜𠮷",
        ];
        for content in garbage {
            let reading = read_detection(content, NAME);
            let found = detection_candidates(&reading, "利用者の発言", "応答");
            assert!(found.candidates.is_empty(), "{content:?}");
            let check = read_check(content, NAME, 3);
            assert!(check.by_number.is_empty(), "{content:?}");
            let decided = decide(&found.candidates, &[check], NAME, &[], PRODUCTION_DEDUP);
            assert!(decided.outcomes.is_empty(), "{content:?}");
        }
    }

    // -----------------------------------------------------------------
    // 候補（検出）
    // -----------------------------------------------------------------

    /// **引用がその往復の本文に無い行は候補にしない**（見張り番。この絞りを外すと、モデルが作った引用が残る）。
    ///
    /// 3行のうち**真ん中**だけが本文に無い。分類が取り決めでない行・同じ発言の言い直しも落ちる。
    #[test]
    fn only_quotes_found_in_the_turn_become_candidates() {
        let user = "わかった。もう黙って出かけないよ。";
        let character = "それと、これからは名前で呼んでくださいね。";
        let reading = read_detection(
            "抜｜「もう黙って出かけない」｜取り決め｜利用者｜利用者は黙って出かけない\n\
             抜｜「夜は台所に入らない」｜取り決め｜利用者｜利用者は夜に台所へ入らない\n\
             抜｜「これからは名前で呼んで」｜取り決め｜利用者｜利用者は葵を名前で呼ぶ",
            NAME,
        );
        let found = detection_candidates(&reading, user, character);
        assert_eq!(
            found
                .candidates
                .iter()
                .map(|candidate| (candidate.quote.as_str(), candidate.from_user))
                .collect::<Vec<_>>(),
            vec![
                ("もう黙って出かけない", true),
                ("これからは名前で呼んで", false)
            ]
        );
        assert_eq!(
            (found.quote_missing, found.not_agreement, found.merged),
            (1, 0, 0)
        );
    }

    /// **使うのは先頭の3行だけ。持論・その場は候補にしない。同じ発言は1つにまとめる。**
    #[test]
    fn the_first_three_lines_only_and_overlapping_quotes_merge() {
        let user = "もう黙って出かけない。夜も出かけない。外では気をつける。台所も開けない。";
        let reading = read_detection(
            "抜｜「もう黙って出かけない」｜取り決め｜利用者｜一\n\
             抜｜「黙って出かけない」｜取り決め｜利用者｜二\n\
             抜｜「外では気をつける」｜持論｜利用者｜三\n\
             抜｜「台所も開けない」｜取り決め｜利用者｜四",
            NAME,
        );
        let found = detection_candidates(&reading, user, "");
        assert_eq!(
            found
                .candidates
                .iter()
                .map(|candidate| candidate.detected_content.as_str())
                .collect::<Vec<_>>(),
            vec!["一"]
        );
        assert_eq!(
            (found.quote_missing, found.not_agreement, found.merged),
            (0, 1, 1)
        );
    }

    // -----------------------------------------------------------------
    // 票と後段（見張り番 —— 条件を1つずつ外すと、残るはずのないものが残る）
    // -----------------------------------------------------------------

    fn candidate(keeper: KeeperReading, content: &str) -> Candidate {
        Candidate {
            quote: "引用".to_owned(),
            from_user: true,
            detected_keeper: keeper,
            detected_content: content.to_owned(),
        }
    }

    fn draw(verdicts: &[(usize, CheckVerdict)]) -> CheckReading {
        CheckReading {
            by_number: verdicts.iter().cloned().collect(),
            unread_lines: 0,
        }
    }

    fn kept(keeper: KeeperReading, content: &str) -> CheckVerdict {
        verdict(
            "戸棚を開ける",
            Class::Agreement,
            keeper,
            Judgement::Keep,
            content,
        )
    }

    fn dropped() -> CheckVerdict {
        verdict(
            "姿勢を正す",
            Class::Scene,
            KeeperReading::User,
            Judgement::Drop,
            "-",
        )
    }

    fn proposed(keeper: KeeperReading, content: &str) -> Outcome {
        Outcome::Proposed {
            keeper,
            content: content.to_owned(),
        }
    }

    /// **票は 2of3**: 検出と1本目が割れたら2本目を引き、2票そろった票が「残す」なら出す。
    ///
    /// 3つの候補（**真ん中が肝**）:
    /// 1. 検出・1本目・2本目が「残す・利用者／落とす／残す・利用者」→ **出す**（中身は最初の回 = 検出）。
    /// 2. 「残す・利用者／落とす／落とす」→ **出さない**（多数が落とす）。
    /// 3. 「残す・両方／残す・利用者／落とす」→ **出さない**（守る側が割れて、2票そろった票が無い）。
    #[test]
    fn two_of_three_votes_decide_and_a_lone_keep_is_not_enough() {
        let candidates = [
            candidate(KeeperReading::User, "利用者は戸棚を開けない"),
            candidate(KeeperReading::User, "利用者は夜に出かけない"),
            candidate(KeeperReading::Both, "二人は海へ行かない"),
        ];
        let first = draw(&[
            (1, dropped()),
            (2, dropped()),
            (3, kept(KeeperReading::User, "利用者は海へ行かない")),
        ]);
        assert!(needs_another_draw(&candidates, &first), "割れたら引く");
        let second = draw(&[
            (1, kept(KeeperReading::User, "利用者は勝手に戸棚を開けない")),
            (2, dropped()),
            (3, dropped()),
        ]);
        let decided = decide(&candidates, &[first, second], NAME, &[], PRODUCTION_DEDUP);
        assert_eq!(
            decided.outcomes,
            vec![
                proposed(KeeperReading::User, "利用者は戸棚を開けない"),
                Outcome::NotKept,
                Outcome::NotKept,
            ]
        );
    }

    /// **検出と1本目がそろえば、2本目は引かない**（「落とす」でそろっても決まり）。
    /// 1本目が読めなかった（無判定）候補が1つでもあれば引く。
    #[test]
    fn agreeing_first_votes_need_no_second_draw() {
        let candidates = [
            candidate(KeeperReading::User, "利用者は戸棚を開けない"),
            candidate(KeeperReading::Both, "二人は海へ行かない"),
        ];
        let agreeing = draw(&[
            (1, kept(KeeperReading::User, "利用者は戸棚を開けない")),
            (2, kept(KeeperReading::Both, "二人は海へ行かない")),
        ]);
        assert!(!needs_another_draw(&candidates, &agreeing));
        let missing_one = draw(&[(1, kept(KeeperReading::User, "利用者は戸棚を開けない"))]);
        assert!(needs_another_draw(&candidates, &missing_one));
        // そろった「残す」で決まり、中身は**最初の回（検出）**のもの。
        let decided = decide(&candidates, &[agreeing], NAME, &[], PRODUCTION_DEDUP);
        assert_eq!(
            decided.outcomes,
            vec![
                proposed(KeeperReading::User, "利用者は戸棚を開けない"),
                proposed(KeeperReading::Both, "二人は海へ行かない"),
            ]
        );
    }

    /// **中身は勝った票の最初の回のもの**（検出の守る側が負けたら、確かめの中身になる）。
    #[test]
    fn the_content_comes_from_the_first_draw_of_the_winning_vote() {
        let candidates = [candidate(KeeperReading::Both, "二人は戸棚を開けない")];
        let first = draw(&[(1, kept(KeeperReading::User, "利用者は戸棚を開けない"))]);
        let second = draw(&[(1, kept(KeeperReading::User, "利用者は台所の戸棚を開けない"))]);
        let decided = decide(&candidates, &[first, second], NAME, &[], PRODUCTION_DEDUP);
        assert_eq!(
            decided.outcomes,
            vec![proposed(KeeperReading::User, "利用者は戸棚を開けない")]
        );
    }

    /// **mind=2**: 確かめの「行い」の欄に心の語があれば、その「残す」は「落とす」になる（見張り番）。
    ///
    /// 2つの候補は**行いの欄だけが違う**。心の語が無い方は出て、ある方は出ない。
    /// **検出の中身に心の語が混ざっても落とさない**（本物の決まり —— 測定の T101）。
    #[test]
    fn a_mind_word_in_the_check_act_turns_the_keep_into_a_drop() {
        let candidates = [
            candidate(KeeperReading::User, "利用者は目を見つめず、思考を止める"),
            candidate(KeeperReading::User, "利用者は目を見つめず、思考を止める"),
        ];
        let mut minded = kept(KeeperReading::User, "利用者は目を見つめない");
        minded.act = "思考を止める".to_owned();
        let first = draw(&[
            (1, kept(KeeperReading::User, "利用者は目を見つめない")),
            (2, minded),
        ]);
        let decided = decide(&candidates, &[first], NAME, &[], PRODUCTION_DEDUP);
        assert_eq!(
            decided.outcomes,
            vec![
                proposed(KeeperReading::User, "利用者は目を見つめず、思考を止める"),
                Outcome::NotKept,
            ]
        );
        assert_eq!(decided.mind_flips, 1);
        for word in MIND_WORDS {
            assert!(is_mind(&format!("相手に{word}")), "{word}");
        }
        assert!(!is_mind("戸棚を開ける"));
    }

    /// **主語の照合**: 中身の主語が守る側と違えば出さない。**主語が読めなければ通す**（見張り番）。
    #[test]
    fn a_content_whose_subject_contradicts_the_keeper_is_not_proposed() {
        let candidates = [
            candidate(KeeperReading::Character, "利用者は戸棚を開けない"),
            candidate(KeeperReading::Character, "毎朝あいさつする"),
            candidate(KeeperReading::Character, "葵は夜に店を閉める"),
        ];
        let first = draw(&[
            (1, kept(KeeperReading::Character, "-")),
            (2, kept(KeeperReading::Character, "-")),
            (3, kept(KeeperReading::Character, "-")),
        ]);
        let decided = decide(&candidates, &[first], NAME, &[], PRODUCTION_DEDUP);
        assert_eq!(
            decided.outcomes,
            vec![
                Outcome::SubjectMismatch,
                proposed(KeeperReading::Character, "毎朝あいさつする"),
                proposed(KeeperReading::Character, "葵は夜に店を閉める"),
            ]
        );
        for (content, expected) in [
            ("二人は海へ行く", Some(KeeperReading::Both)),
            ("利用者と葵は海へ行く", Some(KeeperReading::Both)),
            ("葵と利用者は海へ行く", Some(KeeperReading::Both)),
            ("ふたりは", Some(KeeperReading::Both)),
            ("両者は", Some(KeeperReading::Both)),
            ("利用者は", Some(KeeperReading::User)),
            ("葵は", Some(KeeperReading::Character)),
            ("毎朝", None),
        ] {
            assert_eq!(subject(content, NAME), expected, "{content:?}");
        }
    }

    /// **dedup2 の芯は `rp4.core2` と同じ**（期待値はその関数に同じ綴りを通した結果）。
    #[test]
    fn the_dedup2_core_is_the_measured_one() {
        for (raw, expected) in [
            ("利用者は葵の許しが出るまで射精してはいけない", "射精"),
            ("利用者は声を発してはいけない", "声を発"),
            ("利用者は許可なく台所の戸棚を開けない", "台所の戸棚を開け"),
            ("利用者は台所の戸棚を開けない", "台所の戸棚を開け"),
            ("葵は毎朝あいさつすること", "毎朝あいさつ"),
            ("しない", ""),
            ("利用者は許しを得るまで目を開けない", "目を開け"),
            ("許可なしに出かけてはならない", "出かけ"),
            ("のの許しが来る限り待つべき", "の待つ"),
            ("外ではしないことしない", "外では"),
            ("許可を与えられるまでこと", ""),
            ("許しが出ないまで", ""),
            ("許しがまで", ""),
        ] {
            assert_eq!(
                strip_ending(&strip_condition(&without_subjects(raw, NAME))),
                expected,
                "{raw:?}"
            );
        }
    }

    /// **dedup と dedup2 の答えは `rp2.same_content` / `rp4.same_content2` と同じ。**
    ///
    /// 1行目が dedup2 を足した理由（翼 1 の T60 —— 文末の「〜してはいけない」だけで同じと読んだ）。
    #[test]
    fn the_two_dedup_rules_answer_like_the_measured_tools() {
        for (left, right, plain, stripped) in [
            (
                "利用者は声を発してはいけない",
                "利用者は葵の許しが出るまで射精してはいけない",
                true,
                false,
            ),
            (
                "利用者は台所の戸棚を開けない",
                "利用者は許可なく台所の戸棚を開けない",
                true,
                true,
            ),
            (
                "葵は毎朝あいさつする",
                "葵は毎朝あいさつすること",
                true,
                true,
            ),
            (
                "利用者は夜に出かけない",
                "利用者は朝に海へ行く",
                false,
                false,
            ),
            ("利用者はしない", "利用者はしない", true, false),
        ] {
            assert_eq!(
                DedupRule::Plain.same(left, right, NAME),
                plain,
                "{left:?} {right:?}"
            );
            assert_eq!(
                DedupRule::Stripped.same(left, right, NAME),
                stripped,
                "{left:?} {right:?}"
            );
        }
        assert_eq!(PRODUCTION_DEDUP, DedupRule::Stripped);
    }

    /// **dedup2**: 提案済みと同じ取り決めは出さない。**文末だけが同じものは出す**（見張り番）。
    ///
    /// 同じ確かめの中で先に出したものとも比べる（3つ目が2つ目の言い直し）。
    #[test]
    fn dedup2_drops_restatements_but_not_different_rules_with_the_same_ending() {
        let candidates = [
            candidate(
                KeeperReading::User,
                "利用者は葵の許しが出るまで射精してはいけない",
            ),
            candidate(KeeperReading::User, "利用者は許可なく台所の戸棚を開けない"),
            candidate(KeeperReading::User, "利用者は台所の戸棚を開けてはいけない"),
        ];
        let first = draw(&[
            (1, kept(KeeperReading::User, "-")),
            (2, kept(KeeperReading::User, "-")),
            (3, kept(KeeperReading::User, "-")),
        ]);
        let existing = ["利用者は声を発してはいけない"];
        let decided = decide(
            &candidates,
            std::slice::from_ref(&first),
            NAME,
            &existing,
            DedupRule::Stripped,
        );
        assert_eq!(
            decided.outcomes,
            vec![
                proposed(
                    KeeperReading::User,
                    "利用者は葵の許しが出るまで射精してはいけない"
                ),
                proposed(KeeperReading::User, "利用者は許可なく台所の戸棚を開けない"),
                Outcome::Deduped,
            ]
        );
        // **δ3 で測った dedup では、1つ目が提案済みと同じと読まれて落ちる**（作りの穴 —— dedup2 の理由）。
        let decided = decide(&candidates, &[first], NAME, &existing, DedupRule::Plain);
        assert_eq!(decided.outcomes.first(), Some(&Outcome::Deduped));
        // 提案済み（`existing`）と同じものは出さない。
        let decided = decide(
            &candidates[1..2],
            &[draw(&[(1, kept(KeeperReading::User, "-"))])],
            NAME,
            &["利用者は台所の戸棚を開けない"],
            DedupRule::Stripped,
        );
        assert_eq!(decided.outcomes, vec![Outcome::Deduped]);
    }

    /// **「既存」「無判定」の票も数える**（2票そろえば、その票が勝つ —— 「残す」ではないので出さない）。
    #[test]
    fn existing_and_unjudged_votes_are_votes_too() {
        let candidates = [
            candidate(KeeperReading::User, "利用者は戸棚を開けない"),
            candidate(KeeperReading::User, "利用者は夜に出かけない"),
        ];
        let existing = verdict(
            "戸棚を開ける",
            Class::Agreement,
            KeeperReading::User,
            Judgement::Existing,
            "-",
        );
        let first = draw(&[(1, existing.clone())]);
        let second = draw(&[(1, existing)]);
        let decided = decide(&candidates, &[first, second], NAME, &[], PRODUCTION_DEDUP);
        assert_eq!(decided.outcomes, vec![Outcome::NotKept, Outcome::NotKept]);
        assert_eq!(
            majority(&[keep(KeeperReading::User), Vote::Unjudged, Vote::Unjudged]),
            Some(&Vote::Unjudged)
        );
        assert_eq!(
            majority(&[
                keep(KeeperReading::User),
                keep(KeeperReading::Both),
                Vote::Drop
            ]),
            None
        );
    }

    /// **保存できない提案は落とす**（守る側が読めない・中身が 60 字を超える）。種類は決まり。
    #[test]
    fn only_storable_proposals_become_new_agreements() {
        let saved = proposal_to_save(&KeeperReading::Both, " 二人は海へ行く ").expect("保存できる");
        assert_eq!(
            (saved.content.as_str(), saved.kind, saved.keeper),
            ("二人は海へ行く", AgreementKind::Rule, AgreementKeeper::Both)
        );
        assert_eq!(
            proposal_to_save(&KeeperReading::Missing, "利用者は海へ行く"),
            Err(Unsaveable::UnclearKeeper)
        );
        assert_eq!(
            proposal_to_save(&KeeperReading::Unclear("x".to_owned()), "利用者は海へ行く"),
            Err(Unsaveable::UnclearKeeper)
        );
        assert_eq!(
            proposal_to_save(&KeeperReading::User, &"約".repeat(61)),
            Err(Unsaveable::InvalidContent)
        );
        assert!(proposal_to_save(&KeeperReading::User, &"約".repeat(60)).is_ok());
        // **記号と空白しか無い中身は書かない**（`-` は中身の欄が無かった確かめの綴り）。
        for content in ["-", " - ", "「」", "…", "ー"] {
            assert_eq!(
                proposal_to_save(&KeeperReading::User, content),
                Err(Unsaveable::InvalidContent),
                "{content:?}"
            );
        }
        assert!(proposal_to_save(&KeeperReading::User, "a").is_ok());
    }

    /// **2本目を引くかは、心の語の読み替えを掛ける前の票で決める**（`rp3.py` の `Delta.check` の順 /
    /// 作業単位 A-12c-1 のレビュー 要修正2）。
    ///
    /// 検出「残す・利用者」、1本目「残す・利用者・行いに心の語」の候補は、**読み替える前はそろっている**ので
    /// 2本目を引かない。その後、読み替えた票（残す・落とす）では多数が無いので**出さない**。
    /// 引く判定に読み替えを掛けると、2本目を引いてしまう（測定の依頼の本数と変わる）。
    #[test]
    fn the_second_draw_is_decided_before_the_mind_words_are_applied() {
        let candidates = [candidate(KeeperReading::User, "利用者は相手に従う")];
        let mut minded = kept(KeeperReading::User, "利用者は相手に従う");
        minded.act = "相手に従う".to_owned();
        let first = draw(&[(1, minded)]);
        assert!(
            !needs_another_draw(&candidates, &first),
            "読み替える前はそろっている"
        );
        let decided = decide(&candidates, &[first], NAME, &[], PRODUCTION_DEDUP);
        assert_eq!(decided.outcomes, vec![Outcome::NotKept]);
        assert_eq!(decided.mind_flips, 1);
    }
}
