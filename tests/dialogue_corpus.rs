//! **`web/src/lib/dialogue.ts` が `src/parse` の写しとして成立していること**を
//! 突き合わせるための、字母の全数フィクスチャ。
//!
//! # 赤くなる場所が2つに割れている
//!
//! **Rust 側が権威。** このテストは `parse::parse_response` の出力を
//! `tests/fixtures/dialogue_corpus.tsv` と突き合わせるだけで、**TypeScript を1行も読まない。**
//! `src/parse` の振る舞いが変わると、まずここが赤くなる（＝フィクスチャを作り直せ、と言う）。
//!
//! **同じフィクスチャを `web/tests/dialogue-corpus.test.ts` が読み**、
//! `dialogue.ts` の `split()` から組み立てた結果と突き合わせる。
//! つまり **「Rust が変わった」と「TS が追随していない」が別々に赤くなる。**
//! 1本にまとめると、Rust を変えた回に FE のテストが赤くなり、原因から遠い場所で気付くことになる。
//!
//! **これが ADR-0031 §6（写しなので黙って古びる）に対する唯一の見張り番である。**
//!
//! # 字母に空白と行区切りを入れていない
//!
//! **`dialogue.ts` が写したのは「括弧の対応だけを見る」判定そのもの**で、
//! 保存のための整形（行区切りの畳み込み・前後の空白落とし・空行の始末）は写していない
//! （`dialogue.ts` の冒頭にそう書いてある。**画面は保存しないから**）。
//!
//! **字母から空白と行区切りを外すと、その整形が恒等写像になる。**
//! そうして初めて「写した部分だけ」を突き合わせられる。
//! 入れると TS 側が Rust の整形をもう一度写すことになり、
//! **写しが古びていないかを別の写しで確かめる**という無意味な形になる。
//!
//! # 字母に astral 面の文字（`𠮟` / U+20B9F）を1つ入れてある
//!
//! **Rust は `chars()`（コードポイント）で、TypeScript は UTF-16 の符号単位で走査する。**
//! `dialogue.ts` は「`「` `」` はどちらも BMP なのでサロゲートペアを割らない」と書いているが、
//! **それを通る入力が1つも無ければ、ただの主張である。**
//!
//! # フィクスチャを作り直す
//!
//! ```text
//! PERSONACORE_BLESS_DIALOGUE_CORPUS=1 cargo test --test dialogue_corpus
//! ```
//!
//! **受けるのは `"1"` だけ**（`…=0` は作り直さない）。
//! **そして作り直した回は、書いたうえで必ず失敗する。**
//! 緑にすると、**変数がシェルに残ったまま `cargo test` を回す限り、
//! この見張り番が毎回黙って自分を承認し続ける**ことになる。
//!
//! **作り直したら `web` 側の `pnpm test` も必ず回すこと。**
//! そこが赤くなるなら、**`dialogue.ts` か `web/tests/dialogue-corpus.test.ts` の
//! `asRust` が Rust に追随していない**（どちらを直すかは、あちらの doc にある）。

use std::collections::BTreeSet;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use personacore::parse::{FallbackReason, ParseOutcome, parse_response};

/// 作り直しを頼む環境変数。**受けるのは `"1"` だけ。**
const BLESS_ENV: &str = "PERSONACORE_BLESS_DIALOGUE_CORPUS";

/// 作り直しが頼まれているか。
///
/// # ⚠ **「立っているか」で見ないこと**（D-0b のレビュー / 2026-08-26）
///
/// `var_os(..).is_some()` にすると **`…=0` でも作り直しが走る**
/// （`RUST_LOG=infoo` と同じ形）。**否定のつもりで書いた値が肯定になる。**
fn bless_requested() -> bool {
    std::env::var(BLESS_ENV).is_ok_and(|value| value == "1")
}

/// 主の字母。**`「` `」` に加えて、地の文と台詞の中身を区別できる2文字。**
///
/// **`a` と `b` を別の文字にしてあるのが要点**——同じ文字にすると、
/// 「地の文の側の文字と台詞の側の文字を取り違える」変異が観測できなくなる。
const ALPHABET_MAIN: &[&str] = &["「", "」", "a", "b"];

/// 主の字母の最大長。**フィクスチャが 1MB に収まる最大**（8 にすると 4 倍になる）。
const MAX_LEN_MAIN: usize = 7;

/// 副の字母。**BMP の外**（`𠮟` は UTF-8 で 4 バイト / UTF-16 で 2 符号単位）。
const ALPHABET_ASTRAL: &[&str] = &["「", "」", "𠮟"];

/// 副の字母の最大長。
const MAX_LEN_ASTRAL: usize = 6;

/// 1MB の上限。**超えたら字母か最大長を削る**（作業単位で決めた枠）。
const MAX_FIXTURE_BYTES: usize = 1_000_000;

/// 件数の下限。
///
/// # ⚠ **上限だけでは片側しか押さえていない**（D-0b のレビュー / 実測）
///
/// **`MAX_LEN_MAIN` を 7 → 4 に落として作り直すと 22,811 → 1,403 件になり、
/// Rust 5本・TS 13本が全部緑のまま通った。** 6判定も astral も残るので、
/// **どの見張り番も「痩せた」と言わない。**
/// しかも doc は**削る側だけ**を案内しているので、実際に踏まれうる導線だった。
///
/// **値は `MAX_LEN_MAIN` から作らないこと。** そこから作ると、
/// **定数を変える変異と一緒に期待値も動いて緑のまま**になる
/// （このリポジトリで繰り返し実測されている形）。
const MIN_INPUTS: usize = 20_000;

/// 最も長い入力の下限（**文字数**）。
///
/// 件数だけだと、**字母を増やして長さを縮める**変え方が通り抜ける。
/// 入れ子・閉じない・跡地の組み合わせは**長さの側からしか出ない**ので、両方を押さえる。
const MIN_LONGEST_INPUT_CHARS: usize = 7;

/// フィクスチャの置き場所。**`web/tests/dialogue-corpus.test.ts` も同じ場所を指す。**
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("dialogue_corpus.tsv")
}

/// 判定の1文字表現。
///
/// **`match` で書いてある**ので、`ParseOutcome` に値が増えたらここがコンパイルを通らない。
/// 増やした側に「フィクスチャも作り直せ」と言わせるため。
fn code(outcome: ParseOutcome) -> char {
    match outcome {
        ParseOutcome::Structured => 'S',
        ParseOutcome::StructuredNested => 'N',
        ParseOutcome::NarrationOnly => 'R',
        ParseOutcome::Empty => 'E',
        ParseOutcome::Fallback(FallbackReason::Unclosed) => 'U',
        ParseOutcome::Fallback(FallbackReason::StrayClose) => 'C',
    }
}

/// 字母の全組み合わせを、長さの昇順・字母の順で並べる。
fn enumerate(alphabet: &[&str], max_len: usize) -> Vec<String> {
    let mut all: Vec<String> = vec![String::new()];
    let mut current: Vec<String> = vec![String::new()];

    for _ in 0..max_len {
        let mut next: Vec<String> = Vec::with_capacity(current.len() * alphabet.len());
        for prefix in &current {
            for symbol in alphabet {
                let mut grown = prefix.clone();
                grown.push_str(symbol);
                next.push(grown);
            }
        }
        all.extend(next.iter().cloned());
        current = next;
    }

    all
}

/// 入力の集合。**主の字母 → 副の字母**の順で、重なった分は落とす。
fn inputs() -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut ordered: Vec<String> = Vec::new();

    for source in [
        enumerate(ALPHABET_MAIN, MAX_LEN_MAIN),
        enumerate(ALPHABET_ASTRAL, MAX_LEN_ASTRAL),
    ] {
        for input in source {
            if seen.insert(input.clone()) {
                ordered.push(input);
            }
        }
    }

    ordered
}

/// 欄を書く。**改行だけを `\n` の2文字へ逃がす。**
///
/// 字母に `\` も TAB も行区切りも無いので、これで曖昧さが無くなる。
/// **その前提が崩れたらここで落とす**（黙って壊れたフィクスチャを書かない）。
fn encode(field: &str) -> String {
    assert!(
        !field.contains('\\') && !field.contains('\t') && !field.contains('\r'),
        "フィクスチャの欄に区切りと衝突する文字が入りました: {field:?}"
    );
    field.replace('\n', "\\n")
}

/// `parse` に通した結果を、そのままフィクスチャの本文にする。
fn render() -> String {
    let inputs = inputs();
    let mut body = String::new();

    for input in &inputs {
        let parsed = parse_response(input);
        body.push_str(&encode(input));
        body.push('\t');
        body.push(code(parsed.outcome));
        body.push('\t');
        body.push_str(&encode(parsed.dialogue.as_deref().unwrap_or_default()));
        body.push('\t');
        body.push_str(&encode(parsed.narration.as_deref().unwrap_or_default()));
        body.push('\n');
    }

    let header = format!(
        "\
# personacore / 台詞と地の文の割り方の全数フィクスチャ
# **これは `src/parse::parse_response` の出力そのものである。Rust が権威。**
# 作り直す: PERSONACORE_BLESS_DIALOGUE_CORPUS=1 cargo test --test dialogue_corpus
# 読む側: tests/dialogue_corpus.rs（Rust）/ web/tests/dialogue-corpus.test.ts（TypeScript）
#
# 字母1: {main} / 最大長 {main_len} / 全組み合わせ
# 字母2: {astral} / 最大長 {astral_len} / 全組み合わせ（字母1と重なる入力は落とす）
# 件数: {count}
#
# 欄: 入力 <TAB> 判定 <TAB> dialogue <TAB> narration
# 判定: S=structured N=structured_nested R=narration_only E=empty
#       U=fallback_unclosed C=fallback_stray_close
# 空欄は None。改行は \\n と書く（字母に \\ が無いので曖昧にならない）。
# 字母に空白と行区切りを入れていない理由は tests/dialogue_corpus.rs の冒頭にある。
",
        main = ALPHABET_MAIN.join(" "),
        main_len = MAX_LEN_MAIN,
        astral = ALPHABET_ASTRAL.join(" "),
        astral_len = MAX_LEN_ASTRAL,
        count = inputs.len(),
    );

    format!("{header}{body}")
}

/// 行に割る。**行末の CR は落とす。**
///
/// `core.autocrlf=true` のこのリポジトリでは、LF で書いたファイルが
/// **checkout で CRLF になって戻ってくる。** 中身の比較で改行の作法を争わない。
fn lines(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

#[test]
fn the_committed_corpus_is_what_parse_actually_does() {
    let expected = render();
    let path = fixture_path();

    if bless_requested() {
        fs::write(&path, expected.as_bytes()).expect("フィクスチャを書けませんでした");
        // **書いた回は緑にしない**（`cargo insta` の作法 / D-0b のレビュー）。
        //
        // ここで `return` すると、**シェルに変数が残ったまま `cargo test` を回す限り、
        // この1本は毎回黙ってフィクスチャを書き直して緑になる。**
        // `dialogue.ts` の唯一の守りが、自分で自分を承認し続ける経路になってしまう。
        panic!(
            "{} を作り直しました。{BLESS_ENV} を外してもう一度回してください\
             （作り直した回は緑にしない）。差分を目で見て、web の pnpm test も回すこと",
            path.display()
        );
    }

    let actual = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{} を読めませんでした（{error}）。{BLESS_ENV}=1 で作り直せます",
            path.display()
        )
    });

    let expected_lines = lines(&expected);
    let actual_lines = lines(&actual);

    for (index, (want, got)) in expected_lines.iter().zip(actual_lines.iter()).enumerate() {
        assert_eq!(
            want,
            got,
            "{} の {} 行目が parse の出力と食い違います。\
             src/parse を変えたなら {BLESS_ENV}=1 で作り直し、web の pnpm test も回すこと",
            path.display(),
            index + 1
        );
    }
    assert_eq!(
        expected_lines.len(),
        actual_lines.len(),
        "{} の行数が parse の出力と食い違います（{BLESS_ENV}=1 で作り直せます）",
        path.display()
    );
}

#[test]
fn the_corpus_covers_every_parse_outcome() {
    // **フィクスチャではなく、いま列挙している入力から数える。**
    // 字母を削って「入れ子が1件も出ない集合」になったら、ここで気付く。
    let seen: BTreeSet<char> = inputs()
        .iter()
        .map(|input| code(parse_response(input).outcome))
        .collect();

    let want: BTreeSet<char> = ['S', 'N', 'R', 'E', 'U', 'C'].into_iter().collect();
    assert_eq!(
        seen, want,
        "字母が痩せて、通らない判定が出ました（入れ子・閉じない・空・地の文だけ・台詞だけが全部要る）"
    );
}

#[test]
fn the_corpus_has_no_duplicate_inputs() {
    let all = inputs();
    let unique: HashSet<&String> = all.iter().collect();
    assert_eq!(unique.len(), all.len(), "同じ入力が2度出ています");
}

#[test]
fn the_corpus_reaches_outside_the_basic_multilingual_plane() {
    // **TypeScript 側は UTF-16 の添字で切る。** サロゲートペアを跨ぐ入力が
    // 1件も無ければ、`dialogue.ts` の「BMP だから割らない」は確かめられていない。
    let astral = inputs()
        .into_iter()
        .filter(|input| input.chars().any(|character| character as u32 > 0xFFFF))
        .count();
    assert!(astral > 0, "BMP の外の文字を含む入力が1件もありません");
}

#[test]
fn the_fixture_stays_under_one_megabyte() {
    let size = render().len();
    assert!(
        size <= MAX_FIXTURE_BYTES,
        "フィクスチャが {size} バイトになりました（上限 {MAX_FIXTURE_BYTES}）。\
         字母か最大長を削ること"
    );
}

#[test]
fn the_corpus_does_not_shrink() {
    // **上限（`the_fixture_stays_under_one_megabyte`）と対になる下限。**
    // 片方だけだと「削る」方向が無防備になる（D-0b のレビューで実測。`MIN_INPUTS` の doc）。
    let all = inputs();
    let longest = all
        .iter()
        .map(|input| input.chars().count())
        .max()
        .unwrap_or(0);

    assert!(
        all.len() >= MIN_INPUTS,
        "入力が {} 件しかありません（下限 {MIN_INPUTS}）。\
         字母か最大長を削ったなら、削ってよい理由を先に決めること",
        all.len()
    );
    assert!(
        longest >= MIN_LONGEST_INPUT_CHARS,
        "最も長い入力が {longest} 文字しかありません（下限 {MIN_LONGEST_INPUT_CHARS}）。\
         件数だけ揃えて長さを縮めると、入れ子と跡地の組み合わせが出なくなる"
    );
}

#[test]
fn the_committed_fixture_itself_did_not_shrink() {
    // **上の1本は「これから書くもの」を数えている。こちらは置いてあるものを数える。**
    // 作り直しを忘れた／古い版を戻した、という形はここでしか出ない。

    // **⚠ 作り直しの最中は読まない。**
    // `the_committed_corpus_is_what_parse_actually_does` が同じファイルを
    // **並行して書いている**ので、書きかけを読んで赤くなりうる
    // （実測: 作り直しの回に「1403 行しかありません」と、直前の版を読んで落ちた）。
    // **緑の逃げ道にはならない** ——作り直しの回はあちらが必ず失敗し、
    // **次の（変数を外した）回にこの検査が本番の判定を下す。**
    if bless_requested() {
        return;
    }

    let text = fs::read_to_string(fixture_path()).expect("フィクスチャを読めませんでした");
    let rows = lines(&text)
        .into_iter()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .count();

    assert!(
        rows >= MIN_INPUTS,
        "置いてあるフィクスチャが {rows} 行しかありません（下限 {MIN_INPUTS}）"
    );
}
