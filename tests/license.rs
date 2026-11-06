//! **ライセンスの式の綴りと、写した条文** を見張る（作業単位 O-2 / Issue origin#49 / ADR-0048 §1）。
//!
//! personacore は `MIT OR Apache-2.0 OR LicenseRef-ACCRL-0.3` の三択で配る。
//! この式は6か所に書いてあり、**どれか1つだけが違う綴りになると、どれが正しいのか誰にも分からなくなる**
//! （cargo のメタデータ・npm・人が読む README（日本語・英語・繁体字中国語）・法的な文書が、別々のことを名乗る）。
//!
//! # 何を見るか
//!
//! | 見るもの | どこで |
//! |---|---|
//! | `LICENSE` の SPDX の行・`Cargo.toml` の `license`・`web/package.json` の `license`・README の「ライセンス」の節・訳の README（[`TRANSLATED_READMES`]: `README.en.md` の「License」の節と `README.zh-TW.md` の「授權」の節）の式が、**どれも1つきりで、同じ綴り** | [`every_place_names_the_same_license_expression`] |
//! | 訳の README（英語・繁体字中国語）のライセンスの節が、`LICENSE` の英語の段落（三択の案内）を**訳さずにそのまま**載せている（英語は O-10 / Issue origin#104、繁体字中国語は O-14 / marukei/personacore#8） | [`the_translated_readmes_quote_the_english_paragraph_of_the_license`] |
//! | `LICENSE` が名指すライセンスの文書が実在し、MIT の文書の著作権の行が `LICENSE` と同じ | [`the_license_names_documents_that_exist`] |
//! | `LICENSE-ACCRL.md` / `LICENSE-ACCRL.en.md` / `LICENSE-APACHE` が、写し元から1字も変わっていない | [`the_copied_license_texts_are_unchanged`] |
//! | `LICENSE-MIT` の著作権の行を除いた本文が、MIT の標準の本文から1字も変わっていない | [`the_mit_license_body_is_the_standard_text`] |
//! | **利用者が署名した `LICENSE` の全文**が、署名した版から1字も変わっていない | [`the_signed_license_is_unchanged`] |
//! | `LICENSE` が名指す AI 協働の開示の文書の名前が、根に置いた文書のファイル名と同じ（O-3 / Issue origin#50） | [`the_license_names_the_ai_collaboration_disclosure_that_exists`] |
//! | **利用者が承認した AI 協働の開示の文書の全文**が、承認した版から1字も変わっていない（O-3 / Issue origin#50） | [`the_approved_ai_collaboration_disclosure_is_unchanged`] |
//!
//! # 何を見ていないか（先に書く）
//!
//! * **訳の README の散文が `LICENSE` と矛盾しないか**は、載せた段落の外については見ていない
//!   （繁体字中国語の README の、段落の前の断り書き・後の案内の文も見ていない。翻訳のレビューが受け持つ）。
//!   見ているのは、`LICENSE` の英語の段落が空白の違いを除いて1字も変えずに節の中に在ることだけ。
//! * **式が SPDX として正しいか**は見ていない。見ているのは6か所の綴りが揃っていることと、
//!   それが [`EXPRESSION`] であることだけ（cargo が式を受けるかは `cargo build` / `cargo metadata` で確かめた）。
//! * **`LICENSE` の散文が正しいか**は見ていない。見ているのは、署名した版から変わっていないことだけ
//!   （[`SIGNED_LICENSE_SHA256`]）。
//! * **AI 協働の開示の文書の散文が事実どおりか**（コミットの数・期間等）は見ていない。
//!   見ているのは、承認した版から変わっていないことだけ（[`APPROVED_DISCLOSURE_SHA256`]）。
//!   文書の中のリンクが実在することは `tests/readme.rs` が、3つの README から案内されていることも
//!   `tests/readme.rs` が見る。
//! * **zip に入ること**は `tests/distribution.rs` が見る（AI 協働の開示の文書も）。
//! * **写し元の版を上げたとき**は、下の指紋も一緒に変えないと赤になる（赤の側に倒れる）。
//!
//! # 走査の作法
//!
//! 走査の本体はすべてソースを引数で受け取り、植えたソースを LF と CRLF の両方で通す
//! （clone 直後の作業ツリーは CRLF になる）。

use std::path::PathBuf;

/// **SHA-256 は `tests/api_shape_fingerprint.rs` と共有する1つの実装**（既知の値の検査もそちら / 作業単位 O-7）。
#[path = "common/sha256.rs"]
mod sha256;

use sha256::sha256_hex;

/// **ADR-0048 §1 で決めた式。** 6か所はこれと同じ綴りであること。
const EXPRESSION: &str = "MIT OR Apache-2.0 OR LicenseRef-ACCRL-0.3";

/// 採用の表明と、三択の案内（利用者が承認し署名した文面）。
const LICENSE: &str = "LICENSE";

/// `LICENSE` の中の SPDX の行の頭。
const SPDX_PREFIX: &str = "SPDX-License-Identifier:";

/// Rust のマニフェスト。
const CARGO_MANIFEST: &str = "Cargo.toml";

/// 画面のマニフェスト。
const WEB_MANIFEST: &str = "web/package.json";

/// README と、その中のライセンスの節の見出し。
const README: &str = "README.md";
const README_SECTION: &str = "## ライセンス";

/// 英語の README と、その中のライセンスの節の見出し（作業単位 O-10 / Issue origin#104）。
const README_EN: &str = "README.en.md";
const README_EN_SECTION: &str = "## License";

/// 繁体字中国語（台湾）の README と、その中のライセンスの節の見出し
/// （作業単位 O-14 / marukei/personacore#8）。
const README_ZH_TW: &str = "README.zh-TW.md";
const README_ZH_TW_SECTION: &str = "## 授權";

/// 訳の README と、その中のライセンスの節の見出し。**訳を足すときはここに1行足す**
/// （式の突き合わせと、英語の段落をそのまま載せているかの両方がこの一覧を回す）。
const TRANSLATED_READMES: [(&str, &str); 2] = [
    (README_EN, README_EN_SECTION),
    (README_ZH_TW, README_ZH_TW_SECTION),
];

/// `LICENSE` の英語の段落（三択の案内）の書き出し。段落はここから次の空行の手前まで。
///
/// 訳の README（英語・繁体字中国語）はこの段落を**訳さずにそのまま**載せる
/// （`LICENSE` は利用者が署名した文面なので、言い換えて食い違わせない）。
const LICENSE_ENGLISH_PARAGRAPH_HEAD: &str = "personacore is available under your choice of";

/// MIT の文書（著作権の行だけを差し替えた標準の本文）。
const LICENSE_MIT: &str = "LICENSE-MIT";

/// `LICENSE` が名指すライセンスの文書。
const LICENSE_DOCUMENTS: [&str; 4] = [
    LICENSE_MIT,
    "LICENSE-APACHE",
    "LICENSE-ACCRL.md",
    "LICENSE-ACCRL.en.md",
];

/// **写した条文の指紋**（改行を LF に揃えた中身の SHA-256）。
///
/// * `LICENSE-ACCRL.md`: https://github.com/marukei/ACCRL の `LICENSE-v0.3-PROTOTYPE.md`（コミット d452001）。
///   先頭 16 桁 `d01acdcf334410df` は、司令塔が d452001 の blob で確かめた値と一致する。
/// * `LICENSE-ACCRL.en.md`: 同じリポジトリの `translations/en/LICENSE-v0.3-PROTOTYPE.md`（同じコミット）。
///   先頭 16 桁 `4d24665282ec68f8` が同じく一致する。
/// * `LICENSE-APACHE`: Apache License 2.0 の全文。手元の crate の写し（`ahash-0.8.12` /
///   `atomic-waker-1.1.2` / `base64-0.22.1` 等、レジストリの 81 個の crate の `LICENSE-APACHE` が
///   バイト単位で同じ）から取った。`serde-1.0.229` の写しとは、APPENDIX の有無だけが違う。
const COPIED_TEXTS: [(&str, &str); 3] = [
    (
        "LICENSE-ACCRL.md",
        "d01acdcf334410df9719be6113d3f2938d897f1f0209a95b0067a934aa46c49b",
    ),
    (
        "LICENSE-ACCRL.en.md",
        "4d24665282ec68f811b62ba444c1fd40a2813a92cb5512d6dc8012a42db4284f",
    ),
    (
        "LICENSE-APACHE",
        "a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2",
    ),
];

/// **`LICENSE-MIT` の本文の指紋**（改行を LF に揃え、`Copyright (c) ` で始まる行と、
/// その後に残る先頭の空行を除いた中身の SHA-256 / [`mit_body`]）。
///
/// 出どころ: `serde-1.0.229/LICENSE-MIT`（著作権の行を持たない MIT の標準の本文）の、
/// LF に揃えた全体の SHA-256 と同じ値。`LICENSE-MIT` は `ahash-0.8.12/LICENSE-MIT` を写して
/// 著作権の行だけを差し替えたもので、2行目以降は serde の写しと diff が空だった（O-2）。
/// 著作権の行は [`the_license_names_documents_that_exist`] が `LICENSE` と突き合わせる。
const MIT_BODY_SHA256: &str = "23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3";

/// **利用者が承認し署名した `LICENSE` の全文の指紋**（改行を LF に揃えた中身の SHA-256）。
///
/// 出どころ: 利用者が 2026-11-04 に承認し署名した文面（司令塔が用意したファイル）の SHA-256。
/// O-2 で `LICENSE` として置いたものとバイト単位で同じだった。
///
/// **⚠ この指紋を替えるのは、文面を変えることについて利用者の承認と署名をもう一度得てからにすること。**
/// `LICENSE` は ACCRL の採用の表明と誓約で、人間創作者が行うものである（ACCRL 第1条4号 / ADR-0048 §1）。
/// エージェントが文面を直して、ここの値を合わせて緑に戻してはならない。
const SIGNED_LICENSE_SHA256: &str =
    "8ac54e6f5b9244b7acb1cb8b184d041af1a304fb0cc23abcdb8b858c9740d290";

/// **AI 協働の開示の文書**（作業単位 O-3 / Issue origin#50）。`LICENSE` の誓約の1と、
/// 「AI協働の開示（第10条）」の節が、この名前で名指している。
const DISCLOSURE: &str = "NOTICE-AI-COLLABORATION.md";

/// `LICENSE` の中で、AI 協働の開示の文書の名前の書き出し（[`notice_names`] が拾う語の頭）。
const DISCLOSURE_NAME_HEAD: &str = "NOTICE-";

/// **利用者が承認した AI 協働の開示の文書の全文の指紋**（改行を LF に揃えた中身の SHA-256）。
///
/// 出どころ: 利用者が 2026-11-06 に承認した文面（O-6 の前に数を数え直した版。司令塔が用意した）の、
/// 改行を LF に揃えた SHA-256。最初の版は 2026-11-05 に承認され、O-3 で [`DISCLOSURE`] として置いた。
///
/// **⚠ この指紋を替えるのは、文面を変えることについて利用者の承認をもう一度得てからにすること。**
/// この文書は、人間創作者が `LICENSE` の誓約（透明性レベル3）で公開を約束した記録の一部である
/// （ACCRL 第6条1号・第10条 / ADR-0048 §1）。エージェントが文面を直して（数の更新も含む）、
/// ここの値を合わせて緑に戻してはならない。
const APPROVED_DISCLOSURE_SHA256: &str =
    "f29fc370a32c3b8851aaeb33dac8e430300a41d8e5a642c626a283a7389f5ed3";

// ----------------------------------------------------------------------
// 走査（ソースを引数で受け取る）
// ----------------------------------------------------------------------

/// `LICENSE` の SPDX の行の式を全部返す。
fn spdx_expressions(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| line.trim_end().strip_prefix(SPDX_PREFIX))
        .map(|rest| rest.trim().to_owned())
        .collect()
}

/// `Cargo.toml` の `[package]` の表の `license` の値を全部返す（`license-file` は数えない）。
fn cargo_license_values(source: &str) -> Vec<String> {
    let mut in_package = false;
    let mut found = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "license" {
            continue;
        }
        let value = value.trim();
        let unquoted = value
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or(value);
        found.push(unquoted.to_owned());
    }
    found
}

/// `package.json` の `license` の値（文字列でなければ空）。
fn package_json_license_values(source: &str) -> Vec<String> {
    let json: serde_json::Value =
        serde_json::from_str(source).expect("package.json が JSON として読めること");
    json.get("license")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .into_iter()
        .collect()
}

/// README の `heading` の節の行を返す（見出しの行は含まない）。
///
/// 節は `heading` の行から、次の `## ` の行の手前まで。節が無ければ空。
fn section_lines<'a>(source: &'a str, heading: &str) -> Vec<&'a str> {
    let mut in_section = false;
    let mut found = Vec::new();
    for line in source.lines() {
        let line = line.trim_end();
        if line.starts_with("## ") || line == "##" {
            in_section = line == heading;
            continue;
        }
        if in_section {
            found.push(line);
        }
    }
    found
}

/// README の `heading` の節（[`README_SECTION`] / [`README_EN_SECTION`]）の中で、
/// バッククォートで囲んだ式（` OR ` を含むもの）を全部返す。節が無ければ空。
fn readme_license_expressions(source: &str, heading: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in section_lines(source, heading) {
        for (index, span) in line.split('`').enumerate() {
            if index % 2 == 1 && span.contains(" OR ") {
                found.push(span.to_owned());
            }
        }
    }
    found
}

/// 空白の並び（改行を含む）を1つの空白に潰す。
fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `LICENSE` の英語の段落（[`LICENSE_ENGLISH_PARAGRAPH_HEAD`] の行から次の空行の手前まで）を、
/// 空白を潰して返す。書き出しの行が無い・2つ以上あるなら `None`（どれを載せるべきか決まらない）。
fn license_english_paragraph(license: &str) -> Option<String> {
    let lines: Vec<&str> = license.lines().map(str::trim_end).collect();
    let mut starts = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with(LICENSE_ENGLISH_PARAGRAPH_HEAD))
        .map(|(index, _)| index);
    let start = starts.next()?;
    if starts.next().is_some() {
        return None;
    }
    let paragraph: Vec<&str> = lines[start..]
        .iter()
        .take_while(|line| !line.trim().is_empty())
        .copied()
        .collect();
    Some(collapse_whitespace(&paragraph.join("\n")))
}

/// README の `heading` の節が、`paragraph`（空白を潰したもの）をそのまま含むか。
///
/// **空白の違い（折り返しの位置）だけを許す。** 段落は README の中で別の行で折り返してよいが、
/// 語と記号は1字も変えないこと。
fn section_quotes(source: &str, heading: &str, paragraph: &str) -> bool {
    collapse_whitespace(&section_lines(source, heading).join("\n")).contains(paragraph)
}

/// 「どの場所も、ちょうど1つの式を名乗り、それが `expected` である」に反する場所を挙げる。
fn disagreements(expected: &str, places: &[(&str, Vec<String>)]) -> Vec<String> {
    places
        .iter()
        .filter(|(_, values)| values.len() != 1 || values[0] != expected)
        .map(|(place, values)| format!("{place}: {values:?}"))
        .collect()
}

/// 改行を LF に揃える。
fn to_lf(source: &str) -> String {
    source.replace("\r\n", "\n")
}

/// MIT の文書から、著作権の行（`Copyright (c) ` で始まる行）と、その後に残る先頭の空行を除いた本文
/// （改行は LF に揃える）。
fn mit_body(source: &str) -> String {
    let body: String = to_lf(source)
        .lines()
        .filter(|line| !line.starts_with("Copyright (c) "))
        .map(|line| format!("{line}\n"))
        .collect();
    body.trim_start_matches('\n').to_owned()
}

/// ソースの中で [`DISCLOSURE_NAME_HEAD`] から始まる語（ASCII の英数字・ハイフン・ピリオド・下線の並び）を、
/// 書いてある順に全部返す。語の尻の `.`（文末の句点代わり）は落とす。
fn notice_names(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let is_name_char = |ch: char| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_');
    let mut rest = source;
    while let Some(at) = rest.find(DISCLOSURE_NAME_HEAD) {
        let preceded_by_name = rest[..at].chars().next_back().is_some_and(is_name_char);
        let tail = &rest[at..];
        let len = tail
            .find(|ch: char| !is_name_char(ch))
            .unwrap_or(tail.len());
        if !preceded_by_name {
            found.push(tail[..len].trim_end_matches('.').to_owned());
        }
        rest = &tail[len..];
    }
    found
}

fn repo_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn read_repo_file(relative: &str) -> String {
    std::fs::read_to_string(repo_path(relative))
        .unwrap_or_else(|err| panic!("{relative} を読めること: {err}"))
}

/// 植えたソースを LF と CRLF の両方で返す。
fn both_newlines(source: &str) -> [String; 2] {
    let lf = to_lf(source);
    let crlf = lf.replace('\n', "\r\n");
    [lf, crlf]
}

// ----------------------------------------------------------------------
// 走査の対照（植えたソースを、同じ本体に通す）
// ----------------------------------------------------------------------

#[test]
fn the_spdx_scan_reads_the_source_it_is_given() {
    for source in both_newlines("植えた\n\nSPDX-License-Identifier: A OR B  \n本文\n") {
        assert_eq!(spdx_expressions(&source), vec!["A OR B"]);
    }
    assert_eq!(
        spdx_expressions("SPDX-License-Identifier: A\nSPDX-License-Identifier: B\n"),
        vec!["A", "B"],
        "2行あれば2つとも拾うこと（1つきりかは比べる側が見る）"
    );
    assert!(spdx_expressions("SPDX の行が無い\n").is_empty());
}

#[test]
fn the_cargo_scan_reads_only_the_package_table() {
    let planted = "[package]\nname = \"x\"\nlicense-file = \"LICENSE\"\nlicense = \"A OR B\"\n\n\
                   [dependencies]\nlicense = \"別の表\"\n";
    for source in both_newlines(planted) {
        assert_eq!(cargo_license_values(&source), vec!["A OR B"]);
    }
    assert!(
        cargo_license_values("[package]\nname = \"x\"\n").is_empty(),
        "license が無ければ空"
    );
    assert!(
        cargo_license_values("[dependencies]\nlicense = \"A\"\n").is_empty(),
        "[package] の外の license は数えない"
    );
}

#[test]
fn the_package_json_scan_reads_the_source_it_is_given() {
    for source in both_newlines("{\n  \"name\": \"x\",\n  \"license\": \"A OR B\"\n}\n") {
        assert_eq!(package_json_license_values(&source), vec!["A OR B"]);
    }
    assert!(package_json_license_values("{\"name\": \"x\"}").is_empty());
    assert!(
        package_json_license_values("{\"license\": {\"type\": \"A\"}}").is_empty(),
        "文字列でない license は数えない"
    );
}

#[test]
fn the_readme_scan_reads_only_the_license_section() {
    let planted = "# 題\n\n`外の A OR B`\n\n## ライセンス\n\n`C OR D` から選ぶ。`LICENSE` を見る。\n\n---\n\n\
                   ## 次の節\n\n`E OR F`\n";
    for source in both_newlines(planted) {
        assert_eq!(
            readme_license_expressions(&source, README_SECTION),
            vec!["C OR D"]
        );
        assert!(
            readme_license_expressions(&source, README_EN_SECTION).is_empty(),
            "別の見出しの節は読まない"
        );
    }
    assert!(
        readme_license_expressions("# 題\n\n`A OR B`\n", README_SECTION).is_empty(),
        "節が無ければ空"
    );

    // 英語の README の見出しでも、同じ本体が節の中だけを読む。
    let planted_en =
        "# Title\n\n`outside A OR B`\n\n## License\n\n`C OR D`\n\n## Next\n\n`E OR F`\n";
    for source in both_newlines(planted_en) {
        assert_eq!(
            readme_license_expressions(&source, README_EN_SECTION),
            vec!["C OR D"]
        );
    }

    // 繁体字中国語の README の見出しでも同じ（O-14）。
    let planted_zh = "# 標題\n\n`外 A OR B`\n\n## 授權\n\n`C OR D`\n\n## 參與貢獻\n\n`E OR F`\n";
    for source in both_newlines(planted_zh) {
        assert_eq!(
            readme_license_expressions(&source, README_ZH_TW_SECTION),
            vec!["C OR D"]
        );
    }
}

#[test]
fn the_license_paragraph_scan_reads_the_source_it_is_given() {
    let planted = "題\n\n日本語の段落。\n\n\
                   personacore is available under your choice of A (a),\n\
                   B (b), or C (c).\n\n後ろの段落。\n";
    for source in both_newlines(planted) {
        assert_eq!(
            license_english_paragraph(&source).as_deref(),
            Some("personacore is available under your choice of A (a), B (b), or C (c).")
        );
    }
    assert_eq!(
        license_english_paragraph("段落が無い\n"),
        None,
        "書き出しが無ければ None"
    );
    let twice = "personacore is available under your choice of A.\n\n\
                 personacore is available under your choice of B.\n";
    assert_eq!(
        license_english_paragraph(twice),
        None,
        "書き出しが2つあれば None（どちらを載せるべきか決まらない）"
    );
}

#[test]
fn the_quote_check_allows_only_a_different_wrapping() {
    let paragraph = "available under A (a), B (b), or C (c).";
    let wrapped = "# T\n\n## License\n\navailable under A (a),\nB (b), or C (c).\n\n## Next\n";
    for source in both_newlines(wrapped) {
        assert!(
            section_quotes(&source, README_EN_SECTION, paragraph),
            "折り返しの位置だけが違うなら載せていると数える"
        );
    }
    let reworded = "# T\n\n## License\n\navailable under A (a), B (b) or C (c).\n";
    assert!(
        !section_quotes(reworded, README_EN_SECTION, paragraph),
        "記号を1つ落とせば載せていない"
    );
    let outside = "# T\n\navailable under A (a), B (b), or C (c).\n\n## License\n\n別のこと。\n";
    assert!(
        !section_quotes(outside, README_EN_SECTION, paragraph),
        "節の外にあっても数えない"
    );
}

#[test]
fn the_mit_body_drops_only_the_copyright_line() {
    for source in both_newlines(
        "Copyright (c) 2026 誰か\n\nPermission です。\n\n本文の Copyright (c) は残る\n",
    ) {
        assert_eq!(
            mit_body(&source),
            "Permission です。\n\n本文の Copyright (c) は残る\n"
        );
    }
    assert_eq!(
        mit_body("Permission です。\n"),
        "Permission です。\n",
        "著作権の行が無ければそのまま"
    );
}

#[test]
fn the_notice_name_scan_reads_the_source_it_is_given() {
    let planted = "- 開示（NOTICE-A.md）。\n詳細は NOTICE-B_c.md\n末尾 NOTICE-D.md.\n\
                   OTHER-NOTICE-E.md は数えない（頭が語の途中）\n";
    for source in both_newlines(planted) {
        assert_eq!(
            notice_names(&source),
            vec!["NOTICE-A.md", "NOTICE-B_c.md", "NOTICE-D.md"]
        );
    }
    assert!(notice_names("名指していない\n").is_empty());
}

/// **1つだけ綴りが違えば、その場所だけが挙がる**（植えた対照）。
#[test]
fn a_single_misspelled_place_is_reported() {
    let good = || vec![EXPRESSION.to_owned()];
    assert!(disagreements(EXPRESSION, &[("a", good()), ("b", good())]).is_empty());

    let misspelled = "MIT OR Apache-2.0 OR LicenseRef-ACCRL-0.2";
    assert_eq!(
        disagreements(
            EXPRESSION,
            &[
                ("a", good()),
                ("b", vec![misspelled.to_owned()]),
                ("c", good())
            ]
        ),
        vec![format!("b: [\"{misspelled}\"]")]
    );
    assert_eq!(
        disagreements(EXPRESSION, &[("無い", vec![])]).len(),
        1,
        "名乗っていない場所も挙がること"
    );
    assert_eq!(
        disagreements(EXPRESSION, &[("2つ", vec![EXPRESSION.to_owned(); 2])]).len(),
        1,
        "2つ名乗る場所も挙がること（どちらが正か分からない）"
    );
}

// SHA-256 の既知の値の検査は `tests/common/sha256.rs` の `sha256_matches_the_published_values`
// （このバイナリにも取り込まれて走る）。

// ----------------------------------------------------------------------
// 実ファイル
// ----------------------------------------------------------------------

/// **6か所の式が、どれも1つきりで、[`EXPRESSION`] と同じ綴り**（訳の README は [`TRANSLATED_READMES`] を回す）。
#[test]
fn every_place_names_the_same_license_expression() {
    let mut places = vec![
        (LICENSE, spdx_expressions(&read_repo_file(LICENSE))),
        (
            CARGO_MANIFEST,
            cargo_license_values(&read_repo_file(CARGO_MANIFEST)),
        ),
        (
            WEB_MANIFEST,
            package_json_license_values(&read_repo_file(WEB_MANIFEST)),
        ),
        (
            README,
            readme_license_expressions(&read_repo_file(README), README_SECTION),
        ),
    ];
    for (name, heading) in TRANSLATED_READMES {
        places.push((
            name,
            readme_license_expressions(&read_repo_file(name), heading),
        ));
    }
    let wrong = disagreements(EXPRESSION, &places);
    assert!(
        wrong.is_empty(),
        "ライセンスの式が `{EXPRESSION}` に揃っていない場所: {wrong:#?}"
    );
}

/// **訳の README のライセンスの節が、`LICENSE` の英語の段落をそのまま載せている**
/// （英語は作業単位 O-10 / Issue origin#104、繁体字中国語は O-14 / marukei/personacore#8）。
///
/// `LICENSE` は利用者が署名した文面である。訳の README が三択を自分の言葉で言い直すと
/// （訳すことも含む）、**署名した文面と違うことを言いうる。** なので言い直さず、段落ごと載せる
/// （空白・折り返しの違いだけを許す）。訳は [`TRANSLATED_READMES`] を回す。
#[test]
fn the_translated_readmes_quote_the_english_paragraph_of_the_license() {
    let paragraph = license_english_paragraph(&read_repo_file(LICENSE)).unwrap_or_else(|| {
        panic!("{LICENSE} に `{LICENSE_ENGLISH_PARAGRAPH_HEAD}` で始まる段落が1つきりで在ること")
    });
    let missing: Vec<String> = TRANSLATED_READMES
        .iter()
        .filter(|(name, heading)| !section_quotes(&read_repo_file(name), heading, &paragraph))
        .map(|(name, heading)| format!("{name} の `{heading}` の節"))
        .collect();
    assert!(
        missing.is_empty(),
        "{LICENSE} の英語の段落をそのまま載せていない: {} —— 段落: {paragraph}",
        missing.join(" / ")
    );
}

/// **`LICENSE` が名指す文書が実在し、MIT の文書の著作権の行が `LICENSE` と同じ。**
#[test]
fn the_license_names_documents_that_exist() {
    let license = read_repo_file(LICENSE);
    for document in LICENSE_DOCUMENTS {
        assert!(
            license.contains(document),
            "{LICENSE} が {document} を名指していない"
        );
        assert!(
            repo_path(document).is_file(),
            "{LICENSE} が名指す {document} が無い"
        );
    }

    let copyright_lines = |source: &str| -> Vec<String> {
        source
            .lines()
            .map(str::trim_end)
            .filter(|line| line.starts_with("Copyright (c) "))
            .map(str::to_owned)
            .collect()
    };
    let in_license = copyright_lines(&license);
    assert_eq!(
        in_license.len(),
        1,
        "{LICENSE} の著作権の行が1つきりでない: {in_license:?}"
    );
    assert_eq!(
        copyright_lines(&read_repo_file(LICENSE_MIT)),
        in_license,
        "{LICENSE_MIT} の著作権の行が {LICENSE} と違う"
    );
}

/// **写した条文が、写し元から1字も変わっていない**（改行は LF に揃えて比べる）。
#[test]
fn the_copied_license_texts_are_unchanged() {
    for (document, expected) in COPIED_TEXTS {
        let actual = sha256_hex(to_lf(&read_repo_file(document)).as_bytes());
        assert_eq!(
            actual, expected,
            "{document} が写し元から変わっている（版を上げたなら指紋も変えること）"
        );
    }
}

/// **`LICENSE-MIT` の本文が MIT の標準の本文のまま**（著作権の行は除いて比べる）。
#[test]
fn the_mit_license_body_is_the_standard_text() {
    assert_eq!(
        sha256_hex(mit_body(&read_repo_file(LICENSE_MIT)).as_bytes()),
        MIT_BODY_SHA256,
        "{LICENSE_MIT} の本文が MIT の標準の本文から変わっている"
    );
}

/// **利用者が署名した `LICENSE` が、署名した版のまま**（改行は LF に揃えて比べる）。
///
/// 赤になったら、`LICENSE` を元に戻すこと。文面を変えるなら、利用者の承認と署名をもう一度得てから
/// [`SIGNED_LICENSE_SHA256`] を替える。
#[test]
fn the_signed_license_is_unchanged() {
    assert_eq!(
        sha256_hex(to_lf(&read_repo_file(LICENSE)).as_bytes()),
        SIGNED_LICENSE_SHA256,
        "{LICENSE} が利用者の署名した版から変わっている（変えるなら利用者の承認と署名をもう一度得ること）"
    );
}

/// **`LICENSE` が名指す AI 協働の開示の文書の名前が、根に置いた文書のファイル名と同じ**
/// （作業単位 O-3 / Issue origin#50）。
///
/// `LICENSE` の誓約の1と「AI協働の開示（第10条）」の節の2か所が名指している（署名した文面なので、
/// 名前を変えるなら直すのはファイル名の側）。どちらか1つでも違う名前なら赤。
#[test]
fn the_license_names_the_ai_collaboration_disclosure_that_exists() {
    let named = notice_names(&read_repo_file(LICENSE));
    assert_eq!(
        named,
        vec![DISCLOSURE; 2],
        "{LICENSE} が名指す開示の文書の名前が {DISCLOSURE} の2か所になっていない"
    );
    // **根の実名と、大小文字まで突き合わせる。** `is_file` だと、大小文字を区別しない
    // ファイルシステム（Windows の既定）では `notice-ai-collaboration.md` に改名されても真になる。
    let root_names: Vec<String> = std::fs::read_dir(repo_path(""))
        .expect("リポジトリの根を読めること")
        .map(|entry| {
            let entry = entry.expect("根の項目を読めること");
            entry.file_name().to_string_lossy().into_owned()
        })
        .collect();
    assert!(
        root_names.iter().any(|name| name == DISCLOSURE),
        "{LICENSE} が名指す {DISCLOSURE} が、その綴りのまま根に無い（大小文字の違いも数える）"
    );
    assert!(
        repo_path(DISCLOSURE).is_file(),
        "{LICENSE} が名指す {DISCLOSURE} がファイルでない"
    );
}

/// **利用者が承認した AI 協働の開示の文書が、承認した版のまま**（改行は LF に揃えて比べる /
/// 作業単位 O-3 / Issue origin#50）。
///
/// 赤になったら、文書を元に戻すこと。文面を変えるなら（表の数を更新するだけでも）、
/// 利用者の承認をもう一度得てから [`APPROVED_DISCLOSURE_SHA256`] を替える。
#[test]
fn the_approved_ai_collaboration_disclosure_is_unchanged() {
    assert_eq!(
        sha256_hex(to_lf(&read_repo_file(DISCLOSURE)).as_bytes()),
        APPROVED_DISCLOSURE_SHA256,
        "{DISCLOSURE} が利用者の承認した版から変わっている（変えるなら利用者の承認をもう一度得ること）"
    );
}
