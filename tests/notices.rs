//! **配る zip に同梱する第三者の著作権表示**（`scripts/THIRD-PARTY-NOTICES.txt`）を見張る
//! （作業単位 O-1 / Issue origin#48 / ADR-0048 §4）。
//!
//! ```text
//! scripts/notices.ps1（package.ps1 が zip を組む前に毎回呼ぶ）
//!   1. cargo about generate          → [crate] の各群
//!   2. vite build --sourcemap の sources ＋ 成果物に入る道具 → [npm] の各群
//!   → scripts/THIRD-PARTY-NOTICES.txt（コミットする。zip の中では THIRD-PARTY-NOTICES.txt）
//! ```
//!
//! # 何を見るか
//!
//! | 見るもの | どこで |
//! |---|---|
//! | **実行ファイルに入る crate（`cargo tree -e normal`）が全部載っていて、それ以外（消えた依存・古い版）が載っていない** | [`the_notices_name_exactly_the_crates_the_executable_links`] |
//! | **載っている npm のパッケージが `web/pnpm-lock.yaml` の解決した版で、画面の土台（svelte）を含む** | [`the_notices_name_the_web_packages_at_the_versions_the_lockfile_resolves`] |
//! | **どの群にも、名前とライセンスの文の両方がある**（名前だけ・文だけの群が無い） | 上の2本（[`parse_notices`] の `broken`） |
//! | **組んだ機のパスを持ち込まない** | [`the_notices_carry_no_path_of_the_machine_that_built_them`] |
//! | **ライセンスの文が HTML の実体参照に化けていない** | [`the_notices_are_not_html_escaped`] |
//! | **著作権者の行がひな形のままの群に載るのは、理由つきの帳簿の crate だけ** | [`only_the_listed_crates_carry_a_placeholder_copyright_line`] |
//! | **ソースマップに出ないのに成果物に入る道具（vite / rolldown / tailwindcss）が載っている** | [`the_tools_whose_code_enters_the_bundle_are_listed`] |
//! | **personacore 自身が載っていない**（自分は第三者ではない / O-2） | [`the_notices_do_not_list_personacore_itself`] |
//!
//! zip に実際に入ることは `tests/distribution.rs` の
//! `the_zip_holds_exactly_the_executable_the_readme_the_notices_and_the_licenses` が、
//! `package.ps1` が生成を zip の前に呼ぶことは `tests/start_script.rs` が見る。
//!
//! # 何を守っていないか（**先に書く**）
//!
//! * **ライセンスの文が正しいこと（その crate の本当のライセンスであること）は見ていない。**
//!   見ているのは「群に名前と文がある」ことまで。文の中身の権威は cargo-about と各パッケージ。
//! * **画面の成果物に入る npm のパッケージが「全部」載っていることは、このテストでは見ていない。**
//!   それを決められるのはビルドだけで（`notices.ps1` がソースマップで拾う）、テストはビルドしない。
//!   ここが見ているのは「載っているものの版が lock と一致する」「svelte が載っている」まで ——
//!   **svelte の版を上げれば赤になるが、新しいパッケージを画面に取り込んでも赤にならない。**
//!   その場合の表示が古びないことは、**`package.ps1` が zip の前に毎回作り直すこと**で担保している。
//! * **ソースマップに現れないのに成果物に入る道具の一覧（`notices.ps1` の `$ToolsInTheBundle`）は帳簿である。**
//!   その3つが載っていることは見ているが、**新しい道具のコードが成果物に入るようになったことは止められない。**
//! * **ひな形の行の走査は綴り（`<year>` 等）を見る。** 別の綴りのひな形は拾わない。
//!   Apache-2.0 の標準文の付録にある `[yyyy] [name of copyright owner]` は「適用のしかた」の説明で、
//!   著作権者の行ではないので、拾わないのが正しい。
//! * **`cargo tree` を走らせる。** 実行ファイルの依存の集合の権威は cargo 自身なので、
//!   Cargo.lock を自前で辿る写しを作らない。
//!
//! # 走査の作法
//!
//! **走査の本体はすべてソースを引数で受け取る。** 実ファイルと植えたソースを
//! **同じ本体に通す**（`15b`「対照は自己成就しうる」）。植えたソースは LF と CRLF の両方で通す。

use std::collections::BTreeSet;
use std::path::PathBuf;

/// 生成物（コミットしてある）。
const NOTICES: &str = "scripts/THIRD-PARTY-NOTICES.txt";

/// 画面の依存の lock。
const PNPM_LOCK: &str = "web/pnpm-lock.yaml";

/// 自分自身（表示の対象ではない）。
const ROOT_CRATE: &str = "personacore";

/// 配る実行ファイルの target（zip の名前が `windows-x64` を名乗る）。
const DIST_TARGET: &str = "x86_64-pc-windows-msvc";

/// **画面の成果物に必ず入るパッケージ。** 部品はすべて `.svelte` で、コンパイル結果は
/// svelte の実行時のコードを取り込む（これが載っていなければ、画面の側の収集が空振りしている）。
const WEB_FRAMEWORK: &str = "svelte";

/// 群の頭・名前・文の始まりの印（`scripts/notices/about.hbs` と `scripts/notices.ps1` が書く）。
const LICENSE_TAG: &str = "[license] ";
const CRATE_TAG: &str = "[crate] ";
const NPM_TAG: &str = "[npm] ";
const TEXT_TAG: &str = "[text]";

/// ライセンスの文として最低限要る、空でない行の数（区切りの `====` の行は数えない）。
const MIN_TEXT_LINES: usize = 3;

/// **SPDX の標準文のひな形の綴り**（著作権者の行が埋まっていない印）。
///
/// crate が表示のファイルを同梱していない（または cargo-about が拾えない）と、cargo-about は
/// SPDX の標準文で補い、著作権者の行がこのひな形のまま載る —— **帰属が1文字も無い表示になる。**
const PLACEHOLDERS: [&str; 3] = ["<year>", "<copyright holders>", "<owner>"];

/// **ひな形の行を持つ群に載ってよい crate と、その理由**（帳簿）。
///
/// 直し方は2つ（`scripts/notices/about.toml`）:
/// * 表示を写せるファイルが crate にあるなら `clarify` でそのファイルを指す（windows-sys / windows-link）。
/// * 無くて `MIT OR Apache-2.0` なら Apache-2.0 を選ぶ（Apache の文は著作権者の行を要らない）。
///
/// **どちらもできない crate だけをここに書く。** ここに書いた crate がひな形の群から外れたら、
/// それも赤にする（直ったのに帳簿が残ると、次に同じ crate が崩れても素通りする）。
const PLACEHOLDER_ALLOWED: [(&str, &str); 3] = [
    (
        "aws-lc-sys",
        "BSD-3-Clause の群がひな形になるが、同じ crate の Apache-2.0 の群に載る LICENSE          （AWS-LC の合本）が BSD-3-Clause の部分の表示と条文を含む",
    ),
    (
        "include_dir",
        "license が MIT だけで Apache-2.0 を選べず、crate に表示のファイルが無い          （著作権者は authors の行 Michael Bryan として名前の下に載る）",
    ),
    (
        "include_dir_macros",
        "include_dir と同じ（MIT だけ・表示のファイルが無い・authors の行が載る）",
    ),
];

/// **ソースマップに現れないのに成果物に入る道具**（`scripts/notices.ps1` の `$ToolsInTheBundle`）。
///
/// **帳簿の写しをここに持つのは意図的**（`notices.ps1` から読むと、帳簿の行を消したとき
/// こちらも一緒に減って緑のまま）。**⚠ 新しい道具のコードが成果物に入るようになっても、
/// ここと帳簿の両方に足さない限り止まらない。**
const TOOLS_IN_THE_BUNDLE: [&str; 3] = ["rolldown", "tailwindcss", "vite"];

/// 組んだ機のパスの綴り（どれか1つでも在れば、パスを持ち込んでいる）。
const MACHINE_PATH_WORDS: [&str; 5] = [":\\", "/Users/", "node_modules", ".cargo", "registry/src"];

/// HTML の実体参照（handlebars の2重の波括弧が出す綴り）。
const HTML_ENTITIES: [&str; 5] = ["&lt;", "&gt;", "&amp;", "&quot;", "&#x"];

// ----------------------------------------------------------------------
// 走査の本体。**すべてソースを引数で受け取る。**
// ----------------------------------------------------------------------

/// 表示を読んだ結果。
#[derive(Debug, Default, PartialEq, Eq)]
struct Notices {
    /// `[crate] <名前> <版>` の組。
    crates: BTreeSet<(String, String)>,
    /// `[npm] <名前> <版>` の組。
    npm: BTreeSet<(String, String)>,
    /// **壊れているもの**（読めない名前の行・群の外の名前・名前の無い群・文の無い群）。
    broken: Vec<String>,
    /// **ライセンスの文が [`PLACEHOLDERS`] のひな形の行を持つ群**に載っている名前。
    placeholder: BTreeSet<String>,
}

/// 1つの群を読んでいる途中の状態。
struct Group {
    license: String,
    names: Vec<String>,
    in_text: bool,
    text_lines: usize,
    placeholder: bool,
}

impl Group {
    fn finish(self, notices: &mut Notices) {
        if self.names.is_empty() {
            notices
                .broken
                .push(format!("名前の無い群: {}", self.license));
        }
        if !self.in_text {
            notices
                .broken
                .push(format!("{TEXT_TAG} の無い群: {}", self.license));
        } else if self.text_lines < MIN_TEXT_LINES {
            notices
                .broken
                .push(format!("ライセンスの文が無い群: {}", self.license));
        }
        if self.placeholder {
            notices.placeholder.extend(self.names);
        }
    }
}

/// `<名前> <版>` を読む。**2語きりで、版が数字で始まるものだけ。**
fn name_and_version(rest: &str) -> Option<(String, String)> {
    let mut words = rest.split_whitespace();
    let name = words.next()?;
    let version = words.next()?;
    if words.next().is_some() || !version.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    Some((name.to_owned(), version.to_owned()))
}

/// 表示を読む（[`LICENSE_TAG`] で群が始まり、[`TEXT_TAG`] の前が名前、後ろが文）。
fn parse_notices(source: &str) -> Notices {
    let mut notices = Notices::default();
    let mut group: Option<Group> = None;

    for line in source.lines() {
        if let Some(license) = line.strip_prefix(LICENSE_TAG) {
            if let Some(done) = group.take() {
                done.finish(&mut notices);
            }
            group = Some(Group {
                license: license.trim().to_owned(),
                names: Vec::new(),
                in_text: false,
                text_lines: 0,
                placeholder: false,
            });
            continue;
        }

        let item = line
            .strip_prefix(CRATE_TAG)
            .map(|rest| (true, rest))
            .or_else(|| line.strip_prefix(NPM_TAG).map(|rest| (false, rest)));

        match group.as_mut() {
            None => {
                if item.is_some() || line.trim_end() == TEXT_TAG {
                    notices.broken.push(format!("群の外: {line}"));
                }
            }
            Some(current) if current.in_text => {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.chars().all(|c| c == '=') {
                    current.text_lines += 1;
                }
                if PLACEHOLDERS.iter().any(|mark| line.contains(mark)) {
                    current.placeholder = true;
                }
            }
            Some(current) => {
                if line.trim_end() == TEXT_TAG {
                    current.in_text = true;
                } else if let Some((is_crate, rest)) = item {
                    match name_and_version(rest) {
                        Some(pair) => {
                            current.names.push(pair.0.clone());
                            if is_crate {
                                notices.crates.insert(pair);
                            } else {
                                notices.npm.insert(pair);
                            }
                        }
                        None => notices.broken.push(format!("読めない名前の行: {line}")),
                    }
                }
            }
        }
    }
    if let Some(done) = group.take() {
        done.finish(&mut notices);
    }
    notices
}

/// `cargo tree --prefix none --format {p}` の出力から `(名前, 版)` を読む
/// （`serde_derive v1.0.229 (proc-macro)` / `tokio v1.53.1 (*)` / 根の `personacore v0.1.3 (F:\…)`）。
fn tree_crates(output: &str) -> BTreeSet<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let name = words.next()?;
            let version = words.next()?.strip_prefix('v')?;
            version
                .starts_with(|c: char| c.is_ascii_digit())
                .then(|| (name.to_owned(), version.to_owned()))
        })
        .collect()
}

/// `pnpm-lock.yaml` の `packages:` の節から `(名前, 版)` を読む
/// （`  svelte@5.56.10:` / `  '@scope/name@1.2.3':`）。**`snapshots:` 等の他の節は読まない。**
fn lockfile_packages(source: &str) -> BTreeSet<(String, String)> {
    source
        .lines()
        .skip_while(|line| line.trim_end() != "packages:")
        .skip(1)
        .take_while(|line| line.is_empty() || line.starts_with(' '))
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .filter_map(|line| {
            let key = line.trim().strip_suffix(':')?.trim_matches('\'');
            let at = key.rfind('@').filter(|&at| at > 0)?;
            Some((key[..at].to_owned(), key[at + 1..].to_owned()))
        })
        .collect()
}

/// **LF と CRLF の両方**を返す。
fn both_newlines(source: &str) -> [String; 2] {
    let lf = source.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    [lf, crlf]
}

fn repo_path(relative: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for part in relative.split('/') {
        path.push(part);
    }
    path
}

fn read_repo_file(relative: &str) -> String {
    let path = repo_path(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "{} を読めること（無ければ pwsh ./scripts/notices.ps1 で作る）: {err}",
            path.display()
        )
    })
}

/// **実行ファイルに入る crate の集合**（cargo 自身に聞く。根の crate は除く）。
fn executable_crates() -> BTreeSet<(String, String)> {
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "--locked",
            "--offline",
            "--edges",
            "normal",
            "--target",
            DIST_TARGET,
        ])
        .args(["--prefix", "none", "--format", "{p}", "--manifest-path"])
        .arg(repo_path("Cargo.toml"))
        .output()
        .expect("cargo tree を起こせること");
    assert!(
        output.status.success(),
        "cargo tree が失敗した: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    tree_crates(&String::from_utf8_lossy(&output.stdout))
        .into_iter()
        .filter(|(name, _)| name != ROOT_CRATE)
        .collect()
}

fn pair(name: &str, version: &str) -> (String, String) {
    (name.to_owned(), version.to_owned())
}

// ----------------------------------------------------------------------
// 走査本体の対照（**植えたソースを、実ファイルと同じ本体に通す**）
// ----------------------------------------------------------------------

/// 表示の走査が、植えた群を読み、壊れた群を壊れていると言うこと。
#[test]
fn the_notice_scan_reads_the_source_it_is_given() {
    let good = "\
THIRD-PARTY NOTICES
見出しの中の [crate] や [text] は、行頭でなければ数えない。
群の外の Copyright (c) <year> <copyright holders> も数えない。

================================================================================
[license] MIT License (MIT)
[crate] anyhow 1.0.104
        https://github.com/dtolnay/anyhow
        名前の下の <owner> は文ではないので数えない
[crate] serde 1.0.229
[text]
MIT License

Copyright (c) someone

Permission is hereby granted.
================================================================================
[license] MIT License (MIT)
[crate] ghost 0.1.0
[crate] ghost-macros 0.1.0
[text]
MIT License

Copyright (c) <year> <copyright holders>

Permission is hereby granted.
================================================================================
[license] BSD-3-Clause
[npm] owned 1.0.0
[text]
Copyright (c) <owner>. All rights reserved.
Redistribution and use in source and binary forms.
THIS SOFTWARE IS PROVIDED AS IS.
================================================================================
[license] MIT
[npm] @scope/pkg 2.1.1
[npm] svelte 5.56.10
[text]
Copyright (c) 2016 Svelte Contributors
Permission is hereby granted, free of charge.
THE SOFTWARE IS PROVIDED AS IS.
";
    for source in both_newlines(good) {
        assert_eq!(
            parse_notices(&source),
            Notices {
                crates: [
                    pair("anyhow", "1.0.104"),
                    pair("ghost", "0.1.0"),
                    pair("ghost-macros", "0.1.0"),
                    pair("serde", "1.0.229"),
                ]
                .into(),
                npm: [
                    pair("@scope/pkg", "2.1.1"),
                    pair("owned", "1.0.0"),
                    pair("svelte", "5.56.10"),
                ]
                .into(),
                broken: Vec::new(),
                placeholder: ["ghost", "ghost-macros", "owned"].map(str::to_owned).into(),
            }
        );
    }

    // **壊れた形は、それぞれ壊れていると言う**（名前は読めたものだけ集める）。
    let broken = "\
[crate] orphan 1.0.0
[license] A
[crate] only-name
[crate] three words 1.0.0
[crate] kept 1.0.0
[text]
one
two
three
[license] B
[text]
one
two
three
[license] C
[crate] no-text 1.0.0
[license] D
[crate] short-text 1.0.0
[text]
one
================================================================================
================================================================================
";
    for source in both_newlines(broken) {
        let notices = parse_notices(&source);
        assert_eq!(
            notices.crates,
            [
                pair("kept", "1.0.0"),
                pair("no-text", "1.0.0"),
                pair("short-text", "1.0.0")
            ]
            .into(),
        );
        assert_eq!(
            notices.broken,
            vec![
                "群の外: [crate] orphan 1.0.0".to_owned(),
                "読めない名前の行: [crate] only-name".to_owned(),
                "読めない名前の行: [crate] three words 1.0.0".to_owned(),
                "名前の無い群: B".to_owned(),
                "[text] の無い群: C".to_owned(),
                "ライセンスの文が無い群: D".to_owned(),
            ]
        );
    }
}

/// `cargo tree` の走査が、印の付いた行も根の行も同じ形で読むこと。
#[test]
fn the_tree_scan_reads_the_output_it_is_given() {
    let output = "\
personacore v0.1.3 (F:\\dev\\personacore)
anyhow v1.0.104
serde_derive v1.0.229 (proc-macro)
tokio v1.53.1 (*)
tokio v1.53.1
    Blocking waiting for file lock on package cache
";
    for source in both_newlines(output) {
        assert_eq!(
            tree_crates(&source),
            [
                pair("anyhow", "1.0.104"),
                pair("personacore", "0.1.3"),
                pair("serde_derive", "1.0.229"),
                pair("tokio", "1.53.1"),
            ]
            .into()
        );
    }
}

/// lock の走査が `packages:` の節だけを読み、`importers:` / `snapshots:` を読まないこと。
#[test]
fn the_lockfile_scan_reads_the_source_it_is_given() {
    let lock = "\
lockfileVersion: '9.0'

importers:

  .:
    devDependencies:
      svelte:
        specifier: 5.56.10
        version: 5.56.10

packages:

  '@scope/pkg@2.1.1':
    resolution: {integrity: sha512-x}

  svelte@5.56.10:
    resolution: {integrity: sha512-y}
    engines: {node: '>=18'}

snapshots:

  clsx@9.9.9: {}

  vite@8.2.2(@types/node@26.3.0):
    dependencies:
      rolldown: 1.2.5
";
    for source in both_newlines(lock) {
        assert_eq!(
            lockfile_packages(&source),
            [pair("@scope/pkg", "2.1.1"), pair("svelte", "5.56.10")].into()
        );
    }
}

// ----------------------------------------------------------------------
// 実ファイル
// ----------------------------------------------------------------------

/// **実行ファイルに入る crate が全部載っていて、それ以外が載っていない**（Issue origin#48 の閉じる条件）。
///
/// 集合の権威は `cargo tree --edges normal --target x86_64-pc-windows-msvc`
/// （dev / build の依存は実行ファイルに入らないので除く。proc-macro は含む —— 生成した
/// コードが実行ファイルに入るので、載せる側に倒す）。
///
/// * **足りない** —— 依存を足したのに作り直していない（配る zip に表示が欠ける）。
/// * **余る** —— 依存を消した・版を上げたのに作り直していない（表示が古びている）。
///
/// どちらも直し方は `pwsh ./scripts/notices.ps1` を回してコミットすること。
#[test]
fn the_notices_name_exactly_the_crates_the_executable_links() {
    let notices = parse_notices(&read_repo_file(NOTICES));
    assert_eq!(
        notices.broken,
        Vec::<String>::new(),
        "{NOTICES} に壊れた群がある"
    );

    let linked = executable_crates();
    assert!(
        linked.iter().any(|(name, _)| name == "tokio"),
        "cargo tree の出力を読めていない（走査が空振りしている疑い）: {linked:?}"
    );

    let listed: BTreeSet<(String, String)> = notices
        .crates
        .into_iter()
        .filter(|(name, _)| name != ROOT_CRATE)
        .collect();
    let missing: Vec<_> = linked.difference(&listed).collect();
    assert_eq!(
        missing,
        Vec::<&(String, String)>::new(),
        "実行ファイルに入るのに {NOTICES} に載っていない crate がある\
         （pwsh ./scripts/notices.ps1 で作り直してコミットする）"
    );
    let extra: Vec<_> = listed.difference(&linked).collect();
    assert_eq!(
        extra,
        Vec::<&(String, String)>::new(),
        "{NOTICES} に、実行ファイルに入らない crate（消えた依存・古い版）が載っている\
         （pwsh ./scripts/notices.ps1 で作り直してコミットする）"
    );
}

/// **personacore 自身は載らない**（自分は第三者ではない / 作業単位 O-2 / Issue origin#49）。
///
/// `Cargo.toml` に `license` を書くと、cargo-about は personacore を MIT の群に載せる（O-2 で実測）。
/// 外しているのは `Cargo.toml` の `publish = false` と `scripts/notices/about.toml` の
/// `private = { ignore = true }` の組で、どちらを消しても作り直した表示に載り、ここが赤になる。
///
/// 上の [`the_notices_name_exactly_the_crates_the_executable_links`] は根の crate を比べる前に外すので、
/// 載っていても鳴らない。
#[test]
fn the_notices_do_not_list_personacore_itself() {
    let notices = parse_notices(&read_repo_file(NOTICES));
    assert!(
        !notices.crates.is_empty(),
        "{NOTICES} の crate を読めていない（走査が空振りしている疑い）"
    );
    let own: Vec<_> = notices
        .crates
        .iter()
        .filter(|(name, _)| name == ROOT_CRATE)
        .collect();
    assert_eq!(
        own,
        Vec::<&(String, String)>::new(),
        "{NOTICES} に {ROOT_CRATE} 自身が載っている\
         （Cargo.toml の publish = false と about.toml の private = {{ ignore = true }} を確かめる）"
    );
}

/// **画面の成果物に入る npm のパッケージが、lock の解決した版で載っている**（Issue origin#48 の閉じる条件）。
///
/// **「全部載っている」はここでは見ていない**（モジュールの doc の「守っていないもの」）。
#[test]
fn the_notices_name_the_web_packages_at_the_versions_the_lockfile_resolves() {
    let notices = parse_notices(&read_repo_file(NOTICES));
    assert_eq!(
        notices.broken,
        Vec::<String>::new(),
        "{NOTICES} に壊れた群がある"
    );
    assert!(
        notices.npm.iter().any(|(name, _)| name == WEB_FRAMEWORK),
        "{NOTICES} に {WEB_FRAMEWORK} が載っていない（画面の側の収集が空振りしている疑い）: {:?}",
        notices.npm
    );

    let locked = lockfile_packages(&read_repo_file(PNPM_LOCK));
    assert!(
        locked.iter().any(|(name, _)| name == WEB_FRAMEWORK),
        "{PNPM_LOCK} を読めていない（走査が空振りしている疑い）"
    );
    let stale: Vec<_> = notices.npm.difference(&locked).collect();
    assert_eq!(
        stale,
        Vec::<&(String, String)>::new(),
        "{NOTICES} の npm のパッケージが {PNPM_LOCK} の版と食い違う\
         （pwsh ./scripts/notices.ps1 で作り直してコミットする）"
    );
}

/// **組んだ機のパス（利用者名を含みうる）を、配る表示に持ち込まない**（ADR-0048 の O-4 と同じ家族）。
#[test]
fn the_notices_carry_no_path_of_the_machine_that_built_them() {
    let notices = read_repo_file(NOTICES);
    let found: Vec<_> = MACHINE_PATH_WORDS
        .iter()
        .filter(|word| notices.contains(*word))
        .collect();
    assert_eq!(
        found,
        Vec::<&&str>::new(),
        "{NOTICES} が組んだ機のパスを含む"
    );
}

/// **ライセンスの文が HTML の実体参照に化けていない**（`about.hbs` を2重の波括弧で書くと、
/// 文の `<` `>` `&` `"` が `&lt;` 等になる。読めはするが、文そのものが改変されている）。
#[test]
fn the_notices_are_not_html_escaped() {
    let notices = read_repo_file(NOTICES);
    let found: Vec<_> = HTML_ENTITIES
        .iter()
        .filter(|entity| notices.contains(*entity))
        .collect();
    assert_eq!(
        found,
        Vec::<&&str>::new(),
        "{NOTICES} が HTML の実体参照を含む（about.hbs は3重の波括弧で書く）"
    );
}

/// **ひな形の著作権者の行（`<year>` / `<copyright holders>` / `<owner>`）を持つ群に載るのは、
/// 理由つきの帳簿 [`PLACEHOLDER_ALLOWED`] の crate だけ**（O-1 のレビュー1周目 要修正1）。
///
/// * **帳簿に無い crate が載る** —— 帰属の無い表示を配っている。
///   `scripts/notices/about.toml` に `clarify`（crate の中のファイルを checksum 付きで指す）を足すか、
///   `MIT OR Apache-2.0` なら Apache-2.0 を選ばせる。
/// * **帳簿の crate が載らなくなる** —— 直ったのだから帳簿から消す。
#[test]
fn only_the_listed_crates_carry_a_placeholder_copyright_line() {
    let notices = parse_notices(&read_repo_file(NOTICES));
    assert_eq!(
        notices.broken,
        Vec::<String>::new(),
        "{NOTICES} に壊れた群がある"
    );

    let allowed: BTreeSet<String> = PLACEHOLDER_ALLOWED
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    let unexplained: Vec<_> = notices.placeholder.difference(&allowed).collect();
    assert_eq!(
        unexplained,
        Vec::<&String>::new(),
        "{NOTICES} で、著作権者の行がひな形（{PLACEHOLDERS:?}）の群に、帳簿に無い名前が載っている\
         （scripts/notices/about.toml の clarify か、Apache-2.0 の選択で直す）"
    );
    let stale: Vec<_> = allowed.difference(&notices.placeholder).collect();
    assert_eq!(
        stale,
        Vec::<&String>::new(),
        "PLACEHOLDER_ALLOWED の crate が、もうひな形の群に載っていない（帳簿から消す）"
    );
}

/// **ソースマップに現れないのに成果物に入る道具が、`[npm]` に載っている**（レビュー1周目の提案）。
///
/// `notices.ps1` の帳簿 `$ToolsInTheBundle` の行を消すと、作り直した表示から落ちる —— それを止める。
/// **⚠ 新しい道具の載せ漏れは止められない**（[`TOOLS_IN_THE_BUNDLE`] の doc）。
#[test]
fn the_tools_whose_code_enters_the_bundle_are_listed() {
    let notices = parse_notices(&read_repo_file(NOTICES));
    let listed: BTreeSet<&str> = notices.npm.iter().map(|(name, _)| name.as_str()).collect();
    let missing: Vec<_> = TOOLS_IN_THE_BUNDLE
        .iter()
        .filter(|tool| !listed.contains(*tool))
        .collect();
    assert_eq!(
        missing,
        Vec::<&&str>::new(),
        "成果物に入る道具が {NOTICES} の [npm] に載っていない（notices.ps1 の $ToolsInTheBundle を見る）"
    );
}
