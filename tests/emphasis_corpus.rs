//! **本文から Markdown の強調を落とす規則**を、Rust と TypeScript の両方に対して
//! 同じ1つの表で固定する（作業単位 B-46）。
//!
//! # 表は手書きである
//!
//! 読んでいる `tests/fixtures/emphasis_corpus.tsv` は
//! **`tests/fixtures/dialogue_corpus.tsv` とは性質が違う。**
//! あちらは `parse::parse_response` の出力そのもの（実装が権威）だが、
//! **こちらは人が1行ずつ書いた期待値**で、**作り直しの仕掛けを持たない。**
//!
//! **理由は CLAUDE.md の `16b`** ——「期待値を実装から作ると、定数を変える変異と一緒に
//! 期待値も動いて緑のまま」。**強調の規則は全数列挙できない**（印の種類 × 対の作り方 ×
//! 行の割れ方）ので、全数フィクスチャの作法を持ち込めない。
//! **代わりに、規則が名指ししている場合をこちらから1つずつ挙げる。**
//!
//! # 同じ表を TypeScript 側も読む
//!
//! **`web/tests/emphasis-corpus.test.ts` が同じファイルを読む。**
//! 規則の写しが2つ（Rust と TypeScript）ある以上、**片方だけが古びる**のが既定の壊れ方で、
//! ADR-0031 §6 の「黙って古びる写し」がそのまま当てはまる。
//!
//! | 何が起きた | 赤くなるのは |
//! |---|---|
//! | `strip_emphasis`（Rust）が表と食い違う | **ここ** |
//! | `stripEmphasis`（TypeScript）が表と食い違う | **`pnpm test`** |
//!
//! **どちらの側も、表を書き換えて緑にしないこと。** 表は人の決定であって、
//! 実装の写しではない。
//!
//! # 表が押さえている「写しの部品」
//!
//! TypeScript 側は **`domain::TRIMMABLE`（8点）と `domain::LINE_BREAKS`（7点）を
//! 手で写す**ことになる。**写した表が痩せても誰も言わない**という形を避けるため、
//! この表は次を1行ずつ持っている。
//!
//! * **行区切り7点＋CRLF** —— `**朝<区切り>夜**` は、区切りが**効いていれば**
//!   別々の行になって対にならず、**効いていなければ**1行の対になって落ちる。
//!   **出力が変わるので、表から観測できる。**
//! * **`TRIMMABLE` のうち行区切りでない4点**（半角空白 / TAB / 全角空白 / NBSP）——
//!   `**<空白>あ` の `**` は、空白と認めれば**宙に浮いた印**として落ち、
//!   認めなければ残る。
//! * **空白でない不可視文字2点**（U+2009 / U+FEFF）—— **JavaScript の正規表現の
//!   空白類や `trim()` に寄りかかった実装はここで落ちる**（どちらも JS は空白と認めるが、
//!   `TRIMMABLE` には無い）。
//! * **U+200B** —— 行区切りでも空白でもないので、`**朝<U+200B>夜**` は**対になる。**
//! * **対が `「」` をまたぐ行**（`「**朝」と**夜` と、文まるごとの `**「おはよう」**`）——
//!   **落とすのが「割る前」であることは、この2行でしか観測できない。**
//!   割った後に落とす実装（＝行の区切りに `「` `」` を足したのと同じ受理集合）は、
//!   **この2行が無いと全緑を通る。**
//!   **実測**（B-46 の是正 / 2026-09-17）——その変異は**この2行があれば 2 赤**で、
//!   **2行を外すと 2057 全緑**（`cargo test --no-fail-fast` / exit 0）に戻った。

use std::fs;
use std::path::PathBuf;

use personacore::parse::{parse_response, strip_body_emphasis};

/// 逃げ方に使う文字（U+005C）。
const BACKSLASH: char = '\\';

/// 逃げ方の接頭辞。**16進4桁が続く。**
const UNICODE_ESCAPE: &str = "\\u";

/// バックスラッシュ自身の逃げ方。
const DOUBLED_BACKSLASH: &str = "\\\\";

/// 逃げ方の16進の桁数。
const ESCAPE_DIGITS: usize = 4;

/// 見出しに書いてある件数の行。
const COUNT_PREFIX: &str = "# 件数: ";

/// 表の行数の下限。**痩せたら言う**（`dialogue_corpus.rs` の `MIN_INPUTS` と同じ理由）。
///
/// **表に書いてある件数から作らないこと。** そこから作ると、行を削る変異と一緒に
/// 期待値も動いて緑のままになる。
const MIN_ROWS: usize = 40;

/// 表の1行。
struct Row {
    /// 生出力にあたる入力。
    source: String,
    /// 落とした**対**の数。
    pairs: usize,
    /// 落とした後の本文。
    stripped: String,
}

/// 表の置き場所。**`web/tests/emphasis-corpus.test.ts` も同じ場所を指す。**
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("emphasis_corpus.tsv")
}

/// 欄を読む。**受ける逃げ方は2つだけ**で、他の綴りでは落とす
/// （黙って読み替えると、表に書いたつもりの文字とは違うものを測ることになる）。
fn decode(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut rest = field;

    while let Some(at) = rest.find(BACKSLASH) {
        let (before, tail) = rest.split_at(at);
        out.push_str(before);

        if let Some(after) = tail.strip_prefix(UNICODE_ESCAPE) {
            let (digits, remainder) = after
                .split_at_checked(ESCAPE_DIGITS)
                .unwrap_or_else(|| panic!("逃げ方の後ろが16進4桁ではありません: {field:?}"));
            let code = u32::from_str_radix(digits, 16)
                .unwrap_or_else(|_| panic!("16進4桁ではありません: {digits:?}（{field:?}）"));
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
        let [source, pairs, stripped] = fields.as_slice() else {
            panic!("欄が3つではありません: {line:?}")
        };
        rows.push(Row {
            source: decode(source),
            pairs: pairs.parse().expect("対の数が数ではありません"),
            stripped: decode(stripped),
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

#[test]
fn the_table_spells_invisible_characters_with_escapes() {
    // **生の不可視文字が1つも無いこと。**
    // 生で入れると、編集の道具が黙って潰したときに目で気付けない
    // （CLAUDE.md「見えない文字を扱うテストは逃げ方の綴りで書き、走査で数える」）。
    let text = fs::read_to_string(fixture_path()).expect("表を読めませんでした");

    let raw: Vec<(usize, u32)> = text
        .chars()
        .enumerate()
        .filter(|(_, character)| {
            // 欄の区切り（TAB）と行の区切り（LF / CR）と半角空白だけが生で入ってよい。
            !matches!(character, ' ' | '\t' | '\n' | '\r')
                && (character.is_whitespace() || character.is_control())
        })
        .map(|(at, character)| (at, character as u32))
        .collect();

    assert!(raw.is_empty(), "生の不可視文字が入っています: {raw:?}");
}

#[test]
fn every_row_is_what_the_rule_actually_does() {
    // **表の全行を Rust の実装に通す。** ここが赤いなら、
    // 「規則を変えた」か「表が誤っている」のどちらか——**表は書き換えないこと。**
    let (rows, _) = read_rows();
    let mut wrong: Vec<String> = Vec::new();

    for row in &rows {
        let actual = strip_body_emphasis(&row.source);
        if actual.text != row.stripped || actual.pairs != row.pairs {
            wrong.push(format!(
                "{:?}: 表=({:?}, {}) 実装=({:?}, {})",
                row.source, row.stripped, row.pairs, actual.text, actual.pairs
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
fn the_table_covers_none_one_and_many_pairs() {
    // **「常に 0」を返す実装は、0 の行だけでは捕まらない。**
    let (rows, _) = read_rows();
    let mut seen: Vec<usize> = rows.iter().map(|row| row.pairs).collect();
    seen.sort_unstable();
    seen.dedup();

    for want in [0_usize, 1, 2, 3] {
        assert!(
            seen.contains(&want),
            "対の数が {want} の行がありません（{seen:?}）"
        );
    }
}

#[test]
fn the_body_that_reaches_the_reader_has_the_emphasis_removed() {
    // **表の入力をそのまま `parse_response` に通す。**
    // 落とす場所が `parse_response` の中にあることは、ここでしか見ていない
    // （`strip_body_emphasis` を直に呼ぶ上の1本は、配線を見ていない）。
    let (rows, _) = read_rows();
    let mut wrong: Vec<String> = Vec::new();

    for row in &rows {
        let parsed = parse_response(&row.source);
        // **表の入力は全部「崩れない」もの**にしてある（崩れた回は生出力のままなので、
        // 強調の規則をここから観測できない）。行を増やすときもそうすること。
        assert!(
            !parsed.outcome.fell_back(),
            "表の入力が崩れました: {:?}（{}）",
            row.source,
            parsed.outcome.label()
        );

        let already = parse_response(&row.stripped);
        if parsed.dialogue != already.dialogue
            || parsed.narration != already.narration
            || parsed.outcome != already.outcome
        {
            wrong.push(format!(
                "{:?}: 落としてから割った結果と食い違います（{:?} / {:?} ≠ {:?} / {:?}）",
                row.source, parsed.dialogue, parsed.narration, already.dialogue, already.narration
            ));
        }
        if parsed.emphasis_pairs_stripped != row.pairs {
            wrong.push(format!(
                "{:?}: 対の数が {} ではなく {} でした",
                row.source, row.pairs, parsed.emphasis_pairs_stripped
            ));
        }
    }

    assert!(
        wrong.is_empty(),
        "{} 件が食い違いました:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}
