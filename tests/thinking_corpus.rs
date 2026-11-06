//! **上流が本文に漏らした思考を剥がす規則**を、Rust と TypeScript の両方に対して
//! 同じ1つの表で固定する（作業単位 B-61 / ADR-0053）。
//!
//! 作りは B-46 の `tests/emphasis_corpus.rs` を踏襲している（あちらの doc の
//! 「表は手書きである」「同じ表を TypeScript 側も読む」がそのまま当てはまる）。
//!
//! | 何が起きた | 赤くなるのは |
//! |---|---|
//! | `domain::strip_leaked_thinking`（Rust）が表と食い違う | **ここ** |
//! | `stripThinking`（`web/src/lib/thinking.ts`）が表と食い違う | **`pnpm test`**（`web/tests/thinking-corpus.test.ts`） |
//!
//! **どちらの側も、表を書き換えて緑にしないこと。** 表は人の決定（ADR-0053 §決定2）であって、
//! 実装の写しではない。
//!
//! # 表が押さえている「写しの部品」
//!
//! TypeScript 側は **タグの綴り**（`<think` / `</think` ＋ 半角空白・TAB ＋ `>`）と、
//! 境目の空行を決める **`domain::LINE_BREAKS`（7点）・`domain::TRIMMABLE` のうち行区切りでない4点**
//! を手で持つ。**写した表が痩せても誰も言わない**形を避けるため、この表は次を1行ずつ持つ。
//!
//! * **空白入り・TAB 入りの綴り** —— 受けなければ思考が本文に残る。
//! * **全角空白・NBSP・改行入りの綴り** —— **JavaScript の正規表現の空白類（`\s`）で
//!   タグを読む実装はここで落ちる**（どれも `\s` に入るが、規則は受けない）。
//! * **行区切り7点＋CRLF** —— 思考の直後の空行として落ちるかどうかで出力が変わる。
//!   **U+0085（NEL）は `\s` に入らない**ので、`\s` に寄りかかった実装はここでも落ちる。
//! * **空行に入る空白4点**と、**JavaScript だけが空白と認める2点**（U+2009 / U+FEFF）——
//!   後者を空白と認めると、その行が空行として消える。

use std::fs;
use std::path::PathBuf;

use personacore::domain::strip_leaked_thinking;

/// 逃げ方に使う文字（U+005C）。
const BACKSLASH: char = '\\';

/// 逃げ方の接頭辞。**16進1〜6桁と `}` が続く。**
///
/// **`emphasis_corpus.tsv` の `\uXXXX`（括弧なし）と違う綴りにしてある。** 編集の道具
/// （Write / Edit）が括弧なしの綴りを実文字へ化けさせた（B-61 で実際に起きた）。
const UNICODE_ESCAPE: &str = "\\u{";

/// 逃げ方の終わり。
const ESCAPE_END: char = '}';

/// バックスラッシュ自身の逃げ方。
const DOUBLED_BACKSLASH: &str = "\\\\";

/// 逃げ方の16進の桁数の上限（Unicode のスカラ値は6桁に収まる）。
const MAX_ESCAPE_DIGITS: usize = 6;

/// 見出しに書いてある件数の行。
const COUNT_PREFIX: &str = "# 件数: ";

/// 表の行数の下限。**痩せたら言う。**
///
/// **表に書いてある件数から作らないこと。** そこから作ると、行を削る変異と一緒に
/// 期待値も動いて緑のままになる。
const MIN_ROWS: usize = 45;

/// 表の1行。
struct Row {
    /// 剥がした思考のブロックの数。
    blocks: usize,
    /// 閉じないまま末尾に達したか。
    unclosed: bool,
    /// 剥がした後の本文。
    stripped: String,
    /// 上流の生出力にあたる入力。
    source: String,
}

/// 表の置き場所。**`web/tests/thinking-corpus.test.ts` も同じ場所を指す。**
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("thinking_corpus.tsv")
}

/// 欄を読む。**受ける逃げ方は2つだけ**で、他の綴りでは落とす
/// （`tests/emphasis_corpus.rs` の `decode` と同じ規則。黙って読み替えると、
/// 表に書いたつもりの文字とは違うものを測ることになる）。
fn decode(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut rest = field;

    while let Some(at) = rest.find(BACKSLASH) {
        let (before, tail) = rest.split_at(at);
        out.push_str(before);

        if let Some(after) = tail.strip_prefix(UNICODE_ESCAPE) {
            let (digits, remainder) = after
                .split_once(ESCAPE_END)
                .unwrap_or_else(|| panic!("逃げ方が閉じていません: {field:?}"));
            assert!(
                (1..=MAX_ESCAPE_DIGITS).contains(&digits.len())
                    && digits.chars().all(|c| c.is_ascii_hexdigit()),
                "逃げ方の中が16進1〜6桁ではありません: {digits:?}（{field:?}）"
            );
            let code = u32::from_str_radix(digits, 16)
                .unwrap_or_else(|_| panic!("16進ではありません: {digits:?}（{field:?}）"));
            let character = char::from_u32(code)
                .unwrap_or_else(|| panic!("文字になりません: {digits:?}（{field:?}）"));
            out.push(character);
            rest = remainder;
        } else if let Some(remainder) = tail.strip_prefix(DOUBLED_BACKSLASH) {
            out.push(BACKSLASH);
            rest = remainder;
        } else {
            panic!("受けていない逃げ方です: {field:?}");
        }
    }

    out.push_str(rest);
    out
}

/// 「閉じなかったか」の欄を読む。**`0` と `1` 以外は落とす。**
fn decode_flag(field: &str) -> bool {
    match field {
        "0" => false,
        "1" => true,
        other => panic!("閉じなかったかの欄が 0 / 1 ではありません: {other:?}"),
    }
}

/// 表を読む。**行末の CR は落とす**（`core.autocrlf=true` のリポジトリ）。
fn read_rows() -> (Vec<Row>, usize) {
    let text = fs::read_to_string(fixture_path()).expect("表を読めませんでした");
    let mut rows: Vec<Row> = Vec::new();
    let mut declared: Option<usize> = None;

    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(value) = line.strip_prefix(COUNT_PREFIX) {
            declared = Some(value.trim().parse().expect("件数が数ではありません"));
            continue;
        }
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        let fields: Vec<&str> = line.split('\t').collect();
        let [blocks, unclosed, stripped, source] = fields.as_slice() else {
            panic!("欄が4つではありません: {line:?}")
        };
        rows.push(Row {
            blocks: blocks.parse().expect("ブロックの数が数ではありません"),
            unclosed: decode_flag(unclosed),
            stripped: decode(stripped),
            source: decode(source),
        });
    }

    (rows, declared.expect("見出しに件数がありません"))
}

#[test]
fn the_table_is_read_in_full() {
    // **読み落とすと、下の突き合わせが「少ない行数で全部緑」になる。**
    let (rows, declared) = read_rows();
    assert_eq!(rows.len(), declared, "見出しの件数と行数が食い違います");
    assert!(
        rows.len() >= MIN_ROWS,
        "表が {} 行しかありません（下限 {MIN_ROWS}）",
        rows.len()
    );
}

/// **生で書くと見えない書式文字**のうち、表に紛れ込みうる主なものの範囲（両端を含む）。
/// 一般カテゴリ Cf の全部ではなく、`U+E0001`〜`U+E007F` は未割り当ての位置も含む。
///
/// 安定版の Rust には一般カテゴリを引く API が無いので、範囲で持つ。
/// **`web/tests/thinking-corpus.test.ts` の `FORMAT_RANGES` と同じ範囲**にしてある ——
/// TS 側の `\s` は U+FEFF を空白と認めるが U+200B を認めず、Rust 側の `is_whitespace` は
/// どちらも認めないので、この一覧が無いと**両側で数える文字がずれる**（レビューの指摘）。
const FORMAT_RANGES: [(u32, u32); 10] = [
    (0x00AD, 0x00AD),   // SOFT HYPHEN
    (0x061C, 0x061C),   // ARABIC LETTER MARK
    (0x180E, 0x180E),   // MONGOLIAN VOWEL SEPARATOR
    (0x200B, 0x200F),   // ZERO WIDTH SPACE 〜 RIGHT-TO-LEFT MARK
    (0x202A, 0x202E),   // 双方向の埋め込み・上書き
    (0x2060, 0x2064),   // WORD JOINER 〜 INVISIBLE PLUS
    (0x2066, 0x206F),   // 双方向の分離 〜 NOMINAL DIGIT SHAPES
    (0xFEFF, 0xFEFF),   // ZERO WIDTH NO-BREAK SPACE（BOM）
    (0xFFF9, 0xFFFB),   // INTERLINEAR ANNOTATION
    (0xE0001, 0xE007F), // タグ文字
];

/// 生で入っていてはいけない文字か。
///
/// 欄の区切り（TAB）と行の区切り（LF / CR）と半角空白だけが生で入ってよい。
/// それ以外の空白・制御文字・書式文字（[`FORMAT_RANGES`]）は `\u{…}` で綴ること。
fn is_raw_invisible(character: char) -> bool {
    let code = character as u32;
    !matches!(character, ' ' | '\t' | '\n' | '\r')
        && (character.is_whitespace()
            || character.is_control()
            || FORMAT_RANGES
                .iter()
                .any(|(low, high)| (*low..=*high).contains(&code)))
}

/// **走査の本体。** 生の不可視文字の位置（文字の番号）と符号位置を返す。
fn raw_invisibles(text: &str) -> Vec<(usize, u32)> {
    text.chars()
        .enumerate()
        .filter(|(_, character)| is_raw_invisible(*character))
        .map(|(at, character)| (at, character as u32))
        .collect()
}

#[test]
fn the_table_spells_invisible_characters_with_escapes() {
    // **生の不可視文字が1つも無いこと**（`tests/emphasis_corpus.rs` と同じ理由）。
    let text = fs::read_to_string(fixture_path()).expect("表を読めませんでした");
    let raw = raw_invisibles(&text);
    assert!(raw.is_empty(), "生の不可視文字が入っています: {raw:?}");
}

#[test]
fn the_invisible_scan_catches_planted_characters() {
    // **植えた対照を走査の本体に通す**（実ファイルだけだと、走査が何も数えなくても緑になる）。
    // 表と同じ形の1行（4欄）に、生の文字を1つだけ植える。
    for planted in [
        '\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}', '\u{00AD}', '\u{3000}',
        '\u{00A0}', '\u{0085}', '\u{2028}', '\u{000B}',
    ] {
        let line = format!("1\t0\t本文{planted}\t<think>x</think>本文\r\n");
        assert_eq!(
            raw_invisibles(&line),
            [(6, planted as u32)],
            "U+{:04X} を生の不可視文字として数えること",
            planted as u32
        );
    }
    // **数えてはいけない側の対照**: 区切りと半角空白と、ふつうの文字・逃げ方の綴りだけの行。
    let clean = "# 件数: 1\n1\t0\t\\u{200B}「やあ」 本文\t<think>x</think>\\u{FEFF}本文\r\n";
    assert_eq!(raw_invisibles(clean), [] as [(usize, u32); 0], "{clean:?}");
}

#[test]
fn every_row_is_what_the_rule_actually_does() {
    // **表の全行を Rust の実装に通す。** ここが赤いなら、
    // 「規則を変えた」か「表が誤っている」のどちらか——**表は書き換えないこと。**
    let (rows, _) = read_rows();
    let mut wrong: Vec<String> = Vec::new();

    for row in &rows {
        let actual = strip_leaked_thinking(&row.source);
        if actual.text != row.stripped
            || actual.blocks != row.blocks
            || actual.unclosed != row.unclosed
        {
            wrong.push(format!(
                "{:?}: 表=({:?}, {}, {}) 実装=({:?}, {}, {})",
                row.source,
                row.stripped,
                row.blocks,
                row.unclosed,
                actual.text,
                actual.blocks,
                actual.unclosed
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "{} / {} 行が規則と食い違いました:\n{}",
        wrong.len(),
        rows.len(),
        wrong.join("\n")
    );
}

#[test]
fn the_table_covers_every_count_and_both_flags() {
    // **「常に 0」「常に閉じた」を返す実装は、その値の行だけでは捕まらない。**
    let (rows, _) = read_rows();
    let mut seen: Vec<usize> = rows.iter().map(|row| row.blocks).collect();
    seen.sort_unstable();
    seen.dedup();

    for want in [0_usize, 1, 2, 3] {
        assert!(
            seen.contains(&want),
            "ブロックの数が {want} の行がありません（{seen:?}）"
        );
    }
    for want in [false, true] {
        assert!(
            rows.iter().any(|row| row.unclosed == want),
            "閉じなかったかが {want} の行がありません"
        );
    }
    // **空になる行**（思考だけの回）も要る。画面の「応答が空でした。」の入口。
    assert!(
        rows.iter().any(|row| row.stripped.is_empty()),
        "剥がして空になる行がありません"
    );
}
