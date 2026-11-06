//! **配る zip と、その中の利用者向け README** を見張る（作業単位 F-4d / Issue origin#25）。
//!
//! ```text
//! scripts/package.ps1
//!   1. 画面をビルド → 2. cargo build --release → 3. dist/personacore.exe へ複製
//!   4. scripts/notices.ps1 → scripts/THIRD-PARTY-NOTICES.txt を作り直す（O-1 / Issue origin#48）
//!   5. scripts/package-zip.ps1 → dist/personacore-<版>-windows-x64.zip
//!                                  ├─ personacore.exe
//!                                  ├─ README.md                 （= scripts/README-dist.md）
//!                                  ├─ THIRD-PARTY-NOTICES.txt   （= scripts/THIRD-PARTY-NOTICES.txt）
//!                                  ├─ LICENSE・LICENSE-MIT・LICENSE-APACHE・LICENSE-ACCRL.md・LICENSE-ACCRL.en.md
//!                                  │                            （= リポジトリの根の同じ名前 / O-2 / Issue origin#49）
//!                                  └─ NOTICE-AI-COLLABORATION.md（= リポジトリの根の同じ名前 / O-3 / Issue origin#50）
//! ```
//!
//! # 何を見るか
//!
//! | 見るもの | どこで |
//! |---|---|
//! | **zip の中身が exe と README と第三者の著作権表示と、ライセンスの文書5つと AI 協働の開示の文書きり**（開発用の DB や `upstreams.json` が入らない） | [`the_zip_holds_exactly_the_executable_the_readme_the_notices_and_the_licenses`] |
//! | **README の冒頭が、zip に入っているものを挙げている** | [`the_distributed_readme_names_what_the_zip_holds`] |
//! | **zip の名前の版が、中の exe の `--version` と食い違わない** | 同上 ＋ [`an_executable_that_does_not_name_itself_makes_no_zip`] |
//! | **README が開発者の言葉（ADR の番号・`cargo` 等）を持ち込まない** | [`the_distributed_readme_speaks_no_developer_words`] |
//! | **README が画面の実際の文言で操作を呼ぶ** | [`the_distributed_readme_calls_screen_things_by_their_screen_names`] |
//! | **README が書く起こし方が、実行ファイルの実際の起こし方と同じ** | [`the_distributed_readme_names_the_command_the_executable_actually_runs`] |
//! | **README が書く旗が、実行ファイルの受け付ける旗だけ** | [`the_distributed_readme_names_only_flags_the_executable_accepts`] |
//! | **4段だけ** | [`the_distributed_readme_has_exactly_four_steps`] |
//! | **起こせなかったとき（ログ名が出ない失敗）の文面が、画面に出るとおり** | [`the_distributed_readme_shows_the_spawn_failure_as_the_screen_does`] |
//! | **README の保存先の名前が、実行ファイルの定数と同じ** | [`the_distributed_readme_names_the_files_the_executable_actually_writes`] |
//! | **zip を組む途中で失敗したら、`.partial` を残さない** | [`a_failure_while_zipping_leaves_no_partial_file`] |
//!
//! `package.ps1` の中の**順番**（zip は複製の後）は `tests/start_script.rs` が見る
//! （PowerShell のコメントと文字列を潰す走査があちらに在るため）。
//!
//! # 何を守っていないか（**先に書く**）
//!
//! * **`package.ps1` を最後まで通していない。** `cargo build --release` を回すのが重すぎるので、
//!   **zip を組む段（`package-zip.ps1`）だけを、植えた exe で走らせる。**
//!   植える exe は**このテストのために組まれた本物の `personacore.exe`（debug）**で、
//!   `--version` だけを叩く（DB も画面も上流も触らない。`tests/upstream_cli.rs` の
//!   `the_version_flag_names_the_commit_and_does_nothing_else` が見ている）。
//!   **本番の経路（`package.ps1` → release の exe → zip）は司令塔が実機で1回通す。**
//! * **zip を走らせるテストは Windows だけ**（`#[cfg(windows)]`）。配るのは
//!   `windows-x64` の zip で、`pwsh` を要る。**Mac では README の走査だけが走る。**
//! * **README の散文が正しいこと（その手順で本当に歩けること）は、一切見ていない。**
//!   見ているのは語彙と、画面・実行ファイルとの綴りの一致だけである。
//!   **歩いて確かめるのは利用者と司令塔**（Issue origin#25 の閉じる条件）。
//! * **版の綴りがファイル名に使えない形（タグに `/` を含む等）で止まる枝は、走らせていない。**
//!   植える exe は本物なので、その綴りを名乗らせられない。
//! * **開発者の言葉の一覧（[`DEVELOPER_WORDS`]）は帳簿であって、減らせば通る。**
//!
//! # 走査の作法
//!
//! **走査の本体はすべてソースを引数で受け取る。** 実ファイルと植えたソースを
//! **同じ本体に通す**（`15b`「対照は自己成就しうる」）。
//! **植えたソースは LF と CRLF の両方で通す**（`E-1`: clone 直後は CRLF になる）。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use personacore::server::{
    APP_DATA_DIR_NAME, DATABASE_FILE_NAME, DEFAULT_COMMAND, FRESH_FLAG, NO_BROWSER_FLAG,
    NO_UI_FLAG, UPSTREAM_FLAG, UPSTREAM_LOG_DIR_NAME, UPSTREAMS_FILE_NAME, VERSION_FLAG,
    VERSION_LINE,
};

// ----------------------------------------------------------------------
// 見る相手
// ----------------------------------------------------------------------

/// **利用者向け README の元**（zip の中では [`ZIP_README_ENTRY`] の名前になる）。
const DIST_README: &str = "scripts/README-dist.md";

/// **第三者の著作権表示の元**（zip の中では [`ZIP_NOTICES_ENTRY`] の名前になる / O-1 / Issue origin#48）。
/// 中身の見張りは `tests/notices.rs`。
const DIST_NOTICES: &str = "scripts/THIRD-PARTY-NOTICES.txt";

/// zip を組むスクリプト。
const ZIP_SCRIPT: &str = "scripts/package-zip.ps1";

/// **README が「」で名指す文言を持っている画面のファイル**（レビュー1周目の提案で絞った）。
///
/// `web/src` 全体をつないだ文字列への部分一致だと、**コメントや別の画面の同じ語で素通りした**
/// （レビューの変異 R1 / R2 が緑）。**README が指す画面のファイルだけを読み、
/// コメントを落とし、要素の文字か文字列リテラルの「まるごと」と一致するものだけを数える**
/// （[`screen_texts`]）。
///
/// **⚠ 帳簿である。** README が別の画面を指すようになったら、ここへ足さないと赤になる
/// （足し忘れは赤の側に倒れる）。
const SCREEN_FILES: [&str; 8] = [
    "web/src/lib/router.ts",
    "web/src/lib/UpstreamPanel.svelte",
    "web/src/lib/UpstreamProfiles.svelte",
    "web/src/lib/upstreamProfiles.ts",
    "web/src/lib/CharacterList.svelte",
    // 約束・決まりの画面（配布物の README の「キャラクターに覚えていてほしいこと」が名指しする）。
    "web/src/lib/AgreementsPanel.svelte",
    "web/src/lib/agreements.ts",
    // 会話の画面の、約束・決まりの提案の問いかけ（同じ節が「違う」を名指しする / 作業単位 A-12c-2）。
    "web/src/lib/ProposalPrompt.svelte",
];

/// 起こした上流の状態の文面を組んでいる画面のファイル（`失敗しました: …` の形）。
const LAUNCH_LABEL_SOURCE: &str = "web/src/lib/upstreamProfiles.ts";

/// 画面が「失敗」の理由の前に付ける綴り（`launchLabel` の `` `失敗しました: ${…}` ``）。
const FAILED_PREFIX: &str = "失敗しました: ";

/// 起こすときの引数の形を持っているソース（`Command::new(spec.command())` の直後）。
const LAUNCH_SOURCE: &str = "src/server/launch.rs";

/// `launch.rs` の起こす引数の並びの始まりと終わり（この間の `.arg("…")` を順に読む）。
const LAUNCH_CHAIN_START: &str = "Command::new(spec.command());";
const LAUNCH_CHAIN_END: &str = ".args(spec.extra_args())";

/// **起こせなかったとき（`log: None`）に面へ書く理由**の宣言（`launch.rs` の非公開の定数）。
const SPAWN_FAILED_KEY: &str = "const SPAWN_FAILED_REASON: &str =";

/// README の「保存される場所」の節の見出し（表の名前の列を、定数と突き合わせる）。
const STORAGE_SECTION: &str = "保存される場所";

/// zip の中の実行ファイルの名前。
const ZIP_EXE_ENTRY: &str = "personacore.exe";

/// zip の中の README の名前。
const ZIP_README_ENTRY: &str = "README.md";

/// zip の中の第三者の著作権表示の名前。
const ZIP_NOTICES_ENTRY: &str = "THIRD-PARTY-NOTICES.txt";

/// **zip の中のライセンスの文書の名前**（O-2 / Issue origin#49）。`-LicenseDir` で渡したディレクトリの、
/// 同じ名前のファイルそのものが入る（`package.ps1` はリポジトリの根を渡す）。
/// 文書の中身と式の綴りの見張りは `tests/license.rs`。
const ZIP_LICENSE_ENTRIES: [&str; 5] = [
    "LICENSE",
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "LICENSE-ACCRL.md",
    "LICENSE-ACCRL.en.md",
];

/// **zip の中の AI 協働の開示の文書の名前**（O-3 / Issue origin#50）。`LICENSE` が名指す文書で、
/// ライセンスの文書と同じく `-LicenseDir` で渡したディレクトリの、同じ名前のファイルそのものが入る。
/// 名前の一致と承認した版の指紋の見張りは `tests/license.rs`。
const ZIP_DISCLOSURE_ENTRY: &str = "NOTICE-AI-COLLABORATION.md";

/// **zip の中身のうち、1つずつ名前で呼ぶもの**（ライセンスの文書5つ = [`ZIP_LICENSE_ENTRIES`] の外）。
///
/// zip のテスト（[`the_zip_holds_exactly_the_executable_the_readme_the_notices_and_the_licenses`]）の
/// 期待と、配布物の README の見張り（[`the_distributed_readme_names_what_the_zip_holds`]）の期待は、
/// **どちらもこの配列から引く。** zip に10個目を足してここに足せば README の見張りも要求し、
/// ここに足さなければ zip のテストが赤になる。
const ZIP_NAMED_ENTRIES: [&str; 4] = [
    ZIP_EXE_ENTRY,
    ZIP_README_ENTRY,
    ZIP_NOTICES_ENTRY,
    ZIP_DISCLOSURE_ENTRY,
];

/// 配布物の README がライセンスの文書5つをまとめて呼ぶ綴り（「名前が `LICENSE` で始まる」）。
const ZIP_LICENSE_GROUP_NAME: &str = "LICENSE";

/// zip の名前の頭と尻。**間に `<Cargo.toml の版>-<git describe>` が入る。**
const ZIP_NAME_PREFIX: &str = "personacore-";
const ZIP_NAME_SUFFIX: &str = "-windows-x64.zip";

/// **README に出てはいけない、開発者の言葉。**
///
/// 読み手は「llama.cpp を入れて Hugging Face からモデルを落とせる程度の人」で、
/// **リポジトリを開いていない**（F-8「画面が利用者に ADR の番号を見せている」と同じ家族）。
/// 作業単位の番号（`F-4d` の形）は [`unit_ids`] が別に拾う。
///
/// **⚠ 帳簿である。減らせば通る**（上の「守っていないもの」）。
const DEVELOPER_WORDS: [&str; 14] = [
    "ADR",
    "cargo",
    "Cargo",
    "rustc",
    "pnpm",
    "CLAUDE.md",
    "作業単位",
    "見張り番",
    "Issue",
    "docs/",
    "src/",
    "scripts/",
    ".ps1",
    "PERSONACORE_",
];

/// **README が必ず「」で名指す、画面の文言**（手順4の操作）。
///
/// 綴りの権威は画面のソース（[`SCREEN_FILES`]）。ここは「README が落としていない」ことだけを見る。
const REQUIRED_SCREEN_WORDS: [&str; 7] = [
    "上流モデル",
    "直す",
    "gguf のパス（gguf）",
    "実行ファイル（command）",
    "保存する",
    "このプロファイルに切り替える",
    "準備完了",
];

// ----------------------------------------------------------------------
// 走査の本体。**すべてソースを引数で受け取る。**
// ----------------------------------------------------------------------

/// ソースに現れる開発者の言葉（[`DEVELOPER_WORDS`] と [`unit_ids`]）を、行番号つきで返す。
fn developer_words(source: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (index, line) in source.lines().enumerate() {
        for word in DEVELOPER_WORDS {
            if line.contains(word) {
                found.push((index + 1, word.to_owned()));
            }
        }
        for id in unit_ids(line) {
            found.push((index + 1, id));
        }
    }
    found
}

/// **作業単位の番号の形**（`F-4d` / `B-45` / `C-18`）を拾う。
///
/// 形は「大文字1つ・`-`・数字」で、**前が英数字でないこと**
/// （`UTF-8` の `F-8` を拾わない）。**後ろは数字1つまでしか見ない**（`F-4d` も `F-4` として拾う）。
fn unit_ids(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut found = Vec::new();
    for at in 0..chars.len() {
        let letter = chars[at];
        if !letter.is_ascii_uppercase() {
            continue;
        }
        if at > 0 && chars[at - 1].is_ascii_alphanumeric() {
            continue;
        }
        if chars.get(at + 1) != Some(&'-') {
            continue;
        }
        let Some(digit) = chars.get(at + 2).filter(|ch| ch.is_ascii_digit()) else {
            continue;
        };
        found.push(format!("{letter}-{digit}"));
    }
    found
}

/// ソースの「…」を全部拾う（入れ子は読まない。閉じていない「 は捨てる）。
fn corner_quoted(source: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = source;
    while let Some(open) = rest.find('「') {
        let after = &rest[open + '「'.len_utf8()..];
        let Some(close) = after.find('」') else {
            break;
        };
        found.push(&after[..close]);
        rest = &after[close + '」'.len_utf8()..];
    }
    found
}

/// README の「…」のうち、画面の文言（[`screen_texts`] の集合）に**まるごと**は無い綴りを返す。
fn quoted_words_missing_from_screen<'a>(
    readme: &'a str,
    screen: &BTreeSet<String>,
) -> Vec<&'a str> {
    corner_quoted(readme)
        .into_iter()
        .filter(|word| !screen.contains(*word))
        .collect()
}

/// 画面のソースを、コメントを落とした写しと、文字列リテラルの中身に分ける。
///
/// 落とすコメントは `<!-- … -->` / `/* … */` / `// …（行末まで）`。
/// **文字列リテラル（`"…"` / `'…'` / `` `…` ``）の中の同じ綴りはコメントと読まない**
/// （`"http://…"` の `//` 等）。写しにはリテラルをそのまま残す（属性の値の後の `>` を読むため）。
///
/// # 読めないもの（**先に書く**）
///
/// * **正規表現リテラルは読まない。** `/["']/` の引用符を文字列の始まりと読みうる。
///   `"` と `'` は**改行までに閉じなければ文字列と読まない**ので、崩れは1行に留まる。
///   **崩れは「在るものを見落とす」側（赤）に倒れる。**
/// * `` ` `` の中の `${…}` は分けない（中身をまるごと1つのリテラルとして持つ）。
fn lex_screen(source: &str) -> (String, Vec<String>) {
    let chars: Vec<char> = source.chars().collect();
    let mut code = String::new();
    let mut literals = Vec::new();
    let mut at = 0;
    let starts = |at: usize, pat: &str| {
        pat.chars()
            .enumerate()
            .all(|(k, ch)| chars.get(at + k) == Some(&ch))
    };
    while at < chars.len() {
        if starts(at, "<!--") || starts(at, "/*") {
            let close = if starts(at, "<!--") { "-->" } else { "*/" };
            let mut end = at + 2;
            while end < chars.len() && !starts(end, close) {
                end += 1;
            }
            at = (end + close.chars().count()).min(chars.len());
            code.push(' ');
            continue;
        }
        if starts(at, "//") {
            while at < chars.len() && chars[at] != '\n' {
                at += 1;
            }
            continue;
        }
        let ch = chars[at];
        if matches!(ch, '"' | '\'' | '`') {
            let mut end = at + 1;
            let mut body = String::new();
            let mut closed = false;
            while end < chars.len() {
                let c = chars[end];
                if c == '\\' {
                    if let Some(next) = chars.get(end + 1) {
                        body.push(c);
                        body.push(*next);
                    }
                    end += 2;
                    continue;
                }
                if c == ch {
                    closed = true;
                    break;
                }
                if c == '\n' && ch != '`' {
                    break;
                }
                body.push(c);
                end += 1;
            }
            if closed {
                code.push(ch);
                code.push_str(&body);
                code.push(ch);
                literals.push(body);
                at = end + 1;
            } else {
                // 閉じない引用符は、ただの文字として読む。
                code.push(ch);
                at += 1;
            }
            continue;
        }
        code.push(ch);
        at += 1;
    }
    (code, literals)
}

/// **画面に「まるごと」出る文言**の集合。
///
/// * **要素の文字**: コメントを落とした写しで、`>` の直後から次の `<` / `>` / `{` / `}` までを前後の空白を落として。
///   `>直す</button>` の `直す`、`>\n  新規作成\n</button>` の `新規作成`。
/// * **文字列リテラルの中身**（前後の空白を落として）: `return "上流モデル";` の `上流モデル`。
///
/// **散文の中に同じ語がある（`下の一覧の「直す」で`）だけでは数えない** ——
/// レビューの変異 R2（ボタンの文言だけを変える）が、散文の同じ語で素通りしていた。
fn screen_texts(source: &str) -> BTreeSet<String> {
    let (code, literals) = lex_screen(source);
    let mut found: BTreeSet<String> = literals
        .iter()
        .map(|body| body.trim().to_owned())
        .filter(|body| !body.is_empty())
        .collect();
    for (at, _) in code.match_indices('>') {
        let rest = &code[at + 1..];
        let end = rest.find(['<', '>', '{', '}']).unwrap_or(rest.len());
        let text = rest[..end].trim();
        if !text.is_empty() {
            found.insert(text.to_owned());
        }
    }
    found
}

/// `launch.rs` の起こす引数の並びから、**リテラルの** `.arg("…")` を順に拾う。
///
/// `.arg(spec.gguf())` のような値の引数は拾わない（README ではそこが `<…>` になる）。
fn launch_literal_args(source: &str) -> Option<Vec<String>> {
    let start = source.find(LAUNCH_CHAIN_START)? + LAUNCH_CHAIN_START.len();
    let len = source[start..].find(LAUNCH_CHAIN_END)?;
    let chain = &source[start..start + len];
    Some(
        chain
            .match_indices(".arg(\"")
            .filter_map(|(at, pat)| {
                let rest = &chain[at + pat.len()..];
                rest.find('"').map(|end| rest[..end].to_owned())
            })
            .collect(),
    )
}

/// README の、`head` で始まる最初の行から、`<…>` の置き場所を除いた語を順に返す。
fn readme_command_words(source: &str, head: &str) -> Option<Vec<String>> {
    let line = source
        .lines()
        .map(str::trim_start)
        .find(|line| line.starts_with(head))?;
    let mut plain = String::new();
    let mut depth = 0usize;
    for ch in line.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => plain.push(ch),
            _ => {}
        }
    }
    Some(plain.split_whitespace().map(str::to_owned).collect())
}

/// Rust のソースから、`key` の直後の最初の文字列リテラルを取り出す（改行を挟んでよい）。
fn rust_string_after<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    let rest = &source[source.find(key)? + key.len()..];
    let open = rest.find('"')?;
    let inside = &rest[open + 1..];
    let close = inside.find('"')?;
    Some(&inside[..close])
}

/// `## <数字>. ` で始まる見出しの数字を、順に返す。
fn step_numbers(source: &str) -> Vec<u32> {
    source
        .lines()
        .filter_map(|line| line.strip_prefix("## "))
        .filter_map(|rest| {
            let (number, tail) = rest.split_once(". ")?;
            if tail.is_empty() {
                return None;
            }
            number.parse().ok()
        })
        .collect()
}

/// `personacore.exe` を含む行に現れる `--旗` を全部拾う。
///
/// **他の道具（`hf download --local-dir` 等）の旗を拾わないため、行で絞る。**
fn executable_flags(source: &str) -> BTreeSet<&str> {
    let mut found = BTreeSet::new();
    for line in source.lines().filter(|line| line.contains(ZIP_EXE_ENTRY)) {
        for (at, _) in line.match_indices("--") {
            let rest = &line[at..];
            let len = rest
                .char_indices()
                .skip(2)
                .find(|(_, ch)| !(ch.is_ascii_alphanumeric() || *ch == '-'))
                .map_or(rest.len(), |(end, _)| end);
            if len > 2 {
                found.insert(&rest[..len]);
            }
        }
    }
    found
}

/// `prefix` の直後に続く名前（英数字と、ハイフン・下線・ピリオド）を、現れた順に全部拾う。
///
/// `where.exe llama` の `llama` を拾う。**散文の中の1箇所だけでなく、コードブロックの中も拾う**
/// （片方だけ書き換えた README を通さないため）。
fn names_after<'a>(source: &'a str, prefix: &str) -> Vec<&'a str> {
    source
        .match_indices(prefix)
        .map(|(at, _)| {
            let rest = &source[at + prefix.len()..];
            let len = rest
                .find(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')))
                .unwrap_or(rest.len());
            &rest[..len]
        })
        .collect()
}

/// 行頭（前の空白は落とす）が `head` で始まる行があるか。
fn has_line_starting_with(source: &str, head: &str) -> bool {
    source
        .lines()
        .any(|line| line.trim_start().starts_with(head))
}

/// `--version` の1行から、zip の名前を組む（`personacore 0.1.0 (abc1234)` →
/// `personacore-0.1.0-abc1234-windows-x64.zip`）。**読めなければ `None`。**
fn zip_name_for(version_line: &str) -> Option<String> {
    let rest = version_line.trim().strip_prefix("personacore ")?;
    let (package, describe) = rest.split_once(" (")?;
    let describe = describe.strip_suffix(')')?;
    if package.is_empty() || describe.is_empty() {
        return None;
    }
    Some(format!(
        "{ZIP_NAME_PREFIX}{package}-{describe}{ZIP_NAME_SUFFIX}"
    ))
}

/// 見出し `## <title>` の節の表の行から、**最後の列が `` `…` `` だけのもの**の中身を、上から順に返す。
///
/// 節は次の `## ` の見出しで終わる。表の行は `|` で始まる行（区切りの `|---|` は、中身が
/// バッククォートで囲まれていないので拾わない）。**節が無ければ `None`。**
fn table_names_in_section(source: &str, title: &str) -> Option<Vec<String>> {
    let heading = format!("## {title}");
    let mut lines = source.lines().skip_while(|line| line.trim_end() != heading);
    lines.next()?;
    Some(
        lines
            .take_while(|line| !line.starts_with("## "))
            .filter(|line| line.trim_start().starts_with('|'))
            .filter_map(|line| {
                let last = line.trim().trim_end_matches('|').rsplit('|').next()?.trim();
                let inner = last.strip_prefix('`')?.strip_suffix('`')?;
                (!inner.contains('`')).then(|| inner.to_owned())
            })
            .collect(),
    )
}

/// **LF と CRLF の両方**を返す（植えたソースを両方の改行で通す）。
fn both_newlines(source: &str) -> [String; 2] {
    let lf = source.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    [lf, crlf]
}

// ----------------------------------------------------------------------
// 実ファイルの読み込み
// ----------------------------------------------------------------------

/// リポジトリの根（`package.ps1` が `-LicenseDir` に渡すもの）。
#[cfg(windows)]
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} を読めること: {err}", path.display()))
}

fn dist_readme() -> String {
    read_repo_file(DIST_README)
}

/// [`SCREEN_FILES`] の画面の文言を、全部まとめて返す。
fn screen_words() -> BTreeSet<String> {
    SCREEN_FILES
        .iter()
        .flat_map(|file| screen_texts(&read_repo_file(file)))
        .collect()
}

// ----------------------------------------------------------------------
// 走査本体の対照（**植えたソースを、実ファイルと同じ本体に通す**）
// ----------------------------------------------------------------------

#[test]
fn the_developer_word_scan_reads_the_source_it_is_given() {
    for word in DEVELOPER_WORDS {
        let planted = format!("# 見出し\n\nここに {word} と書いた。\n");
        for source in both_newlines(&planted) {
            assert_eq!(
                developer_words(&source),
                vec![(3, word.to_owned())],
                "植えた `{word}` を、その行で拾うこと"
            );
        }
    }
    for id in ["F-4d", "B-45", "C-18"] {
        let planted = format!("（{id} で決めた）\n");
        assert_eq!(
            developer_words(&planted).len(),
            1,
            "作業単位の番号 {id} を拾うこと"
        );
    }
    let clean = "UTF-8 の Ministral-3-14B-Instruct-2512-Q4_K_M.gguf を windows-x64 で。\n";
    for source in both_newlines(clean) {
        assert_eq!(
            developer_words(&source),
            Vec::new(),
            "作業単位の番号に似た綴り（`UTF-8` / モデル名）を拾わないこと"
        );
    }

    // **実ファイルも同じ本体を通ること**（空振りしていないこと）。
    assert!(
        !dist_readme().is_empty(),
        "{DIST_README} が空でないこと（空なら何も拾えない）"
    );
}

#[test]
fn the_quote_scan_reads_the_source_it_is_given() {
    let planted = "「直す」を押し、「無いボタン」を押す。「閉じていない\n";
    for source in both_newlines(planted) {
        assert_eq!(
            corner_quoted(&source),
            vec!["直す", "無いボタン"],
            "閉じた「…」だけを順に拾うこと"
        );
        let screen = BTreeSet::from(["直す".to_owned()]);
        assert_eq!(
            quoted_words_missing_from_screen(&source, &screen),
            vec!["無いボタン"],
            "画面に無い綴りだけを返すこと"
        );
    }

    // **実ファイルも同じ本体を通ること。**
    assert!(
        corner_quoted(&dist_readme()).len() >= REQUIRED_SCREEN_WORDS.len(),
        "実ファイルからも拾えること"
    );
}

/// `screen_texts` は、コメントを落とし、要素の文字とリテラルの「まるごと」だけを拾うこと。
#[test]
fn the_screen_text_scan_reads_the_source_it_is_given() {
    // **コメントの中の「文字列」と「要素」を植える** —— コメントの中身が地の文だけだと、
    // 落とさなくても「まるごと」には一致しないので、落とす処理を消しても対照が緑のままになる
    // （2周目の変異 P3a / P3b で実測）。
    let planted = r#"<!-- <button>コメントの中のボタン</button> -->
<script lang="ts">
  // const old = "行のコメントの中の文字列";
  /** 旧: "ブロックのコメントの中の文字列" */
  const nav = "上流モデル";
  const url = "http://127.0.0.1/"; // "行末のコメントの中の文字列"
  const failed = `失敗しました: ${reason}`;
</script>
<p class="note">下の一覧の「直す」で直します。</p>
<button onclick={() => go()}>このプロファイルに切り替える</button>
<button type="button" onclick={() => (editing = { kind: "create" })}>
  新規作成
</button>
"#;
    for source in both_newlines(planted) {
        let texts = screen_texts(&source);
        for present in [
            "上流モデル",
            "http://127.0.0.1/",
            "失敗しました: ${reason}",
            "このプロファイルに切り替える",
            "新規作成",
        ] {
            assert!(
                texts.contains(present),
                "「{present}」を拾うこと: {texts:?}"
            );
        }
        for absent in [
            "コメントの中のボタン",
            "行のコメントの中の文字列",
            "ブロックのコメントの中の文字列",
            "行末のコメントの中の文字列",
            "直す",
        ] {
            assert!(
                !texts.contains(absent),
                "「{absent}」を画面の文言として拾わないこと: {texts:?}"
            );
        }
    }

    // **実ファイルも同じ本体を通ること**（帳簿の各ファイルから何か拾えること）。
    for file in SCREEN_FILES {
        assert!(
            !screen_texts(&read_repo_file(file)).is_empty(),
            "{file} から何も拾えていない（走査が空振りしている疑い）"
        );
    }
}

/// `launch_literal_args` / `readme_command_words` / `rust_string_after` は、渡されたソースを読むこと。
#[test]
fn the_launch_scans_read_the_source_they_are_given() {
    let planted = "let mut command = Command::new(spec.command());\n    command\n        .arg(\"serve\")\n        .arg(\"-m\")\n        .arg(spec.gguf())\n        .arg(\"--port\")\n        .args(spec.extra_args())\n        .arg(\"--late\")\n";
    for source in both_newlines(planted) {
        assert_eq!(
            launch_literal_args(&source),
            Some(vec![
                "serve".to_owned(),
                "-m".to_owned(),
                "--port".to_owned()
            ]),
            "並びの中のリテラルだけを、順に拾うこと（`.args` の後は拾わない）"
        );
    }
    assert_eq!(launch_literal_args("何も無い"), None, "並びが無ければ None");

    let readme = "```text\n  llama serve -m <gguf のパス> --port <番号> <起動の引数>\n```\n";
    for source in both_newlines(readme) {
        assert_eq!(
            readme_command_words(&source, "llama serve"),
            Some(vec![
                "llama".to_owned(),
                "serve".to_owned(),
                "-m".to_owned(),
                "--port".to_owned()
            ]),
            "`<…>` の置き場所（空白を含む）を除いた語を、順に返すこと"
        );
    }

    let rust = "const SPAWN_FAILED_REASON: &str =\n    \"植えた理由\";\n";
    for source in both_newlines(rust) {
        assert_eq!(
            rust_string_after(&source, SPAWN_FAILED_KEY),
            Some("植えた理由"),
            "改行を挟んだ文字列リテラルを拾うこと"
        );
    }

    // **実ファイルも同じ本体を通ること。**
    let launch = read_repo_file(LAUNCH_SOURCE);
    assert!(
        launch_literal_args(&launch).is_some_and(|args| !args.is_empty()),
        "{LAUNCH_SOURCE} から起こす引数を拾えること"
    );
    assert!(
        rust_string_after(&launch, SPAWN_FAILED_KEY).is_some(),
        "{LAUNCH_SOURCE} から起こせなかったときの理由を拾えること"
    );
}

/// `table_names_in_section` は、その節の表の最後の列だけを読むこと。
#[test]
fn the_table_scan_reads_the_source_it_is_given() {
    let planted = "## 手順\n見本（`upstreams.json`）を作ります。\n\n## 保存される場所\n\n| もの | 名前 |\n|---|---|\n| DB | `a.sqlite` |\n| 設定（`x`） | `b.json` |\n| 説明 | 地の文 |\n\n## ほかの使い方\n\n| やりたいこと | 打つもの |\n|---|---|\n| 版 | `--version` |\n";
    for source in both_newlines(planted) {
        assert_eq!(
            table_names_in_section(&source, "保存される場所"),
            Some(vec!["a.sqlite".to_owned(), "b.json".to_owned()]),
            "その節の表の、バッククォートだけの最後の列を順に拾うこと（節の外・地の文は拾わない）"
        );
    }
    assert_eq!(
        table_names_in_section("## 別の節\n| a | `b` |\n", "保存される場所"),
        None,
        "節が無ければ None"
    );
    assert!(
        table_names_in_section(&dist_readme(), STORAGE_SECTION)
            .is_some_and(|names| !names.is_empty()),
        "実ファイルからも拾えること"
    );
}

#[test]
fn the_step_scan_reads_the_source_it_is_given() {
    let planted = "# 題\n## 1. 入れる\n## 用意\n## 2. 落とす\n### 3. 小見出し\n## 3.5 ではない\n";
    for source in both_newlines(planted) {
        assert_eq!(
            step_numbers(&source),
            vec![1, 2],
            "`## <数字>. ` の見出しだけを拾うこと"
        );
    }
    assert!(
        !step_numbers(&dist_readme()).is_empty(),
        "実ファイルからも拾えること"
    );
}

#[test]
fn the_flag_scan_reads_the_source_it_is_given() {
    let planted = "`.\\personacore.exe --fresh --no-browser`\n`hf download x --local-dir y`\n";
    for source in both_newlines(planted) {
        assert_eq!(
            executable_flags(&source),
            BTreeSet::from(["--fresh", "--no-browser"]),
            "personacore.exe の行の旗だけを拾うこと"
        );
    }
    assert!(
        !executable_flags(&dist_readme()).is_empty(),
        "実ファイルからも拾えること"
    );
}

#[test]
fn the_command_scans_read_the_source_they_are_given() {
    let planted = "```powershell
where.exe llama-server
```
`where.exe llama` を打つ。
  llama serve -m x.gguf
";
    for source in both_newlines(planted) {
        assert_eq!(
            names_after(&source, "where.exe "),
            vec!["llama-server", "llama"],
            "コードブロックの中も散文の中も、現れた順に拾うこと"
        );
        assert!(
            has_line_starting_with(&source, "llama serve -m "),
            "字下げした行も拾うこと"
        );
        assert!(
            !has_line_starting_with(&source, "llama-server -m "),
            "無い行を在ると言わないこと"
        );
    }
    assert!(
        !names_after(&dist_readme(), "where.exe ").is_empty(),
        "実ファイルからも拾えること"
    );
}

#[test]
fn the_zip_name_is_built_from_the_version_line() {
    assert_eq!(
        zip_name_for("personacore 9.8.7 (v1-2-gabc1234)\r\n").as_deref(),
        Some("personacore-9.8.7-v1-2-gabc1234-windows-x64.zip"),
        "版と describe を `-` でつなぐこと"
    );
    assert_eq!(zip_name_for(""), None, "空は読めない");
    assert_eq!(zip_name_for("other 1.0 (x)"), None, "別の名乗りは読めない");
    assert_eq!(
        zip_name_for("personacore 1.0"),
        None,
        "括弧が無いものは読めない"
    );
    assert!(
        zip_name_for(VERSION_LINE).is_some(),
        "この実行ファイルの名乗りからは組めること"
    );
}

// ----------------------------------------------------------------------
// README（実ファイル）
// ----------------------------------------------------------------------

/// **README が開発者の言葉を持ち込まないこと**（F-8 の作法 / Issue origin#25）。
#[test]
fn the_distributed_readme_speaks_no_developer_words() {
    assert_eq!(
        developer_words(&dist_readme()),
        Vec::new(),
        "{DIST_README} に開発者の言葉がある（読み手はリポジトリを開いていない）"
    );
}

/// **README が「」で名指す綴りは、全部画面の文言に「まるごと」在ること。**
///
/// **画面と README が同じ操作を別の名前で呼ぶと、読み手は押すボタンを見つけられない。**
/// README では**画面の文言だけを「」で囲む**（ログの綴りやコマンドはバッククォート）。
///
/// # 何を見て、何を見ていないか
///
/// 探すのは [`SCREEN_FILES`] の、**コメントを落とした**ソースの**要素の文字と文字列リテラル**
/// （[`screen_texts`]）。**「その文言が、README の言う場所（ボタン・見出し・表の行）に出ること」までは
/// 見ていない** —— 同じ綴りの文字列が帳簿のファイルのどこかにまるごと在れば通る。
#[test]
fn the_distributed_readme_calls_screen_things_by_their_screen_names() {
    let readme = dist_readme();
    let screen = screen_words();

    assert_eq!(
        quoted_words_missing_from_screen(&readme, &screen),
        Vec::<&str>::new(),
        "{DIST_README} が「」で呼んでいる綴りが、画面に無い"
    );

    let quoted: BTreeSet<&str> = corner_quoted(&readme).into_iter().collect();
    for word in REQUIRED_SCREEN_WORDS {
        assert!(
            quoted.contains(word),
            "{DIST_README} が画面の「{word}」を名指していない（手順4の操作が落ちている）"
        );
    }
}

/// **README が書く起こし方が、実行ファイルの実際の起こし方と同じこと。**
///
/// 既定の実行ファイル名は [`DEFAULT_COMMAND`]、引数の並びは `launch.rs` のリテラルの `.arg("…")`。
/// **どちらかを変えたら、README の案内（`where.exe llama` 等）も変わる。**
#[test]
fn the_distributed_readme_names_the_command_the_executable_actually_runs() {
    let launch = read_repo_file(LAUNCH_SOURCE);
    let mut expected = vec![DEFAULT_COMMAND.to_owned()];
    expected.extend(launch_literal_args(&launch).expect("起こす引数の並びを拾えること"));

    let readme = dist_readme();
    // **起こし方を示すコードブロックの行**の語の並びが、`launch.rs` の並び
    // （既定の実行ファイル名 ＋ リテラルの `.arg("…")`）と同じであること。
    // レビューの変異 R4（`-m` → `--model`）が、前の形（`serve` までしか見ない）を素通りしていた。
    let head = format!("{DEFAULT_COMMAND} serve ");
    assert_eq!(
        readme_command_words(&readme, &head),
        Some(expected),
        "{DIST_README} のコードブロックの起こし方が、{LAUNCH_SOURCE} の起こし方と違う"
    );
    // **`where.exe <名前>` は、全部既定の実行ファイル名であること**（1つ以上）。
    let located = names_after(&readme, "where.exe ");
    assert!(
        !located.is_empty(),
        "{DIST_README} が `where.exe {DEFAULT_COMMAND}` で確かめる案内を書いていない"
    );
    assert!(
        located.iter().all(|name| *name == DEFAULT_COMMAND),
        "{DIST_README} の `where.exe` が既定の実行ファイル名（{DEFAULT_COMMAND}）以外を探させている: {located:?}"
    );
}

/// **README が書く旗は、実行ファイルが受け付ける5つだけで、5つとも書くこと。**
///
/// **5つ目の `--no-ui` は ADR-0047 §①（作業単位 P-1）。**
///
/// 知らない旗は実行ファイルが起動を止める（`src/server/fresh.rs`）ので、
/// **README の綴り違いは「書いてあるとおりに打つと起動しない」になる。**
#[test]
fn the_distributed_readme_names_only_flags_the_executable_accepts() {
    let accepted = BTreeSet::from([
        FRESH_FLAG,
        UPSTREAM_FLAG,
        NO_BROWSER_FLAG,
        VERSION_FLAG,
        NO_UI_FLAG,
    ]);
    let readme = dist_readme();
    let named = executable_flags(&readme);
    assert_eq!(
        named, accepted,
        "{DIST_README} が personacore.exe に付ける旗が、実行ファイルの受け付ける旗と違う"
    );
}

/// README の「zip の中身を言う文」の書き出し。
const ZIP_CONTENTS_SENTENCE_HEAD: &str = "この zip に入っているのは";

/// README の「zip の中身を言う文」の結び（書き出しの後で最初に現れるもの）。
const ZIP_CONTENTS_SENTENCE_TAIL: &str = "だけです";

/// README の「zip の中身を言う文」（[`ZIP_CONTENTS_SENTENCE_HEAD`] で始まり、その後の最初の
/// [`ZIP_CONTENTS_SENTENCE_TAIL`] で終わる。行をまたいでよい）の中で、バッククォートで囲んだ綴りを
/// 全部返す。**その文の外は読まない**（同じ名前を別の文が持っていても数えない）。
/// 書き出しが無い・結びが無いなら空。
fn names_in_zip_contents_sentence(source: &str) -> BTreeSet<String> {
    let Some(start) = source.find(ZIP_CONTENTS_SENTENCE_HEAD) else {
        return BTreeSet::new();
    };
    let rest = &source[start..];
    let Some(end) = rest.find(ZIP_CONTENTS_SENTENCE_TAIL) else {
        return BTreeSet::new();
    };
    rest[..end]
        .split('`')
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, span)| span.to_owned())
        .collect()
}

/// `names_in_zip_contents_sentence` は、渡されたソースの「中身を言う文」だけを読むこと
/// （植えたソース / LF と CRLF）。**同じ名前が文の外（前・後の行）に在っても数えない。**
#[test]
fn the_zip_contents_sentence_scan_reads_only_that_sentence() {
    let planted = "# 題\n\n`c.txt` は前の行にある。\n\
                   この zip に入っているのは `a.exe` と、\n`b.md` だけです。\n\
                   `c.txt` には説明が書いてあります。\n\n## 1. 手順\n\n`d.txt` だけです。\n";
    for source in both_newlines(planted) {
        assert_eq!(
            names_in_zip_contents_sentence(&source),
            BTreeSet::from(["a.exe".to_owned(), "b.md".to_owned()]),
            "文の外にある `c.txt`・`d.txt` を数えてはならない"
        );
    }
    assert!(
        names_in_zip_contents_sentence("`a.exe` だけです。\n").is_empty(),
        "書き出しが無ければ空"
    );
    assert!(
        names_in_zip_contents_sentence("この zip に入っているのは `a.exe` です。\n").is_empty(),
        "結びが無ければ空"
    );
}

/// **README の「この zip に入っているのは … だけです」の文が、zip に入っているものをちょうど挙げること**
/// （O-3 / Issue origin#50 で AI 協働の開示の文書を足したときに置いた）。
///
/// zip に足したのに README のこの文が古いままだと、**利用者は自分が受け取ったものを知らされない。**
/// 期待の名前は zip のテストと同じ [`ZIP_NAMED_ENTRIES`] から引く（zip に足して
/// [`ZIP_NAMED_ENTRIES`] に足せば、ここも一緒に要求が増える）。ライセンスの文書は
/// 「名前が `LICENSE` で始まる」とまとめて呼ぶので、[`ZIP_LICENSE_GROUP_NAME`] の綴り1つで数える。
#[test]
fn the_distributed_readme_names_what_the_zip_holds() {
    let named = names_in_zip_contents_sentence(&dist_readme());
    let expected: BTreeSet<String> = ZIP_NAMED_ENTRIES
        .into_iter()
        .chain([ZIP_LICENSE_GROUP_NAME])
        .map(str::to_owned)
        .collect();
    assert_eq!(
        named, expected,
        "{DIST_README} の「{ZIP_CONTENTS_SENTENCE_HEAD} … {ZIP_CONTENTS_SENTENCE_TAIL}」の文が挙げる名前が、zip の中身と違う"
    );
}

/// **4段だけ**（Issue origin#25: llama.cpp を入れる → gguf を落とす → exe を叩く → 画面で選ぶ）。
#[test]
fn the_distributed_readme_has_exactly_four_steps() {
    assert_eq!(
        step_numbers(&dist_readme()),
        vec![1, 2, 3, 4],
        "{DIST_README} の手順が 1〜4 の4段になっていない"
    );
}

/// **起こせなかったとき（ログのファイル名が出ない失敗）の文面を、画面に出るとおりに書くこと**
/// （レビュー1周目 要修正1）。
///
/// `llama` が PATH に無い・`command` の書き間違いでは、`launch.rs` が理由に
/// `SPAWN_FAILED_REASON` を書き、**ログは `None`**（画面にログのファイル名が出ない）。
/// **いちばん起きやすい失敗なので、README は「ログを開け」だけでは足りない。**
///
/// 見るもの:
/// * README が `` `失敗しました: <SPAWN_FAILED_REASON>` `` を**そのまま**書いている。
///   理由の綴りは `launch.rs` から、前置きは画面の `launchLabel` のテンプレートから取る。
/// * README が、ログのファイル名が付く形 `` `（ログ: <UPSTREAM_LOG_DIR_NAME> の …）` `` も書き、
///   その綴りが画面のテンプレートに在る。
#[test]
fn the_distributed_readme_shows_the_spawn_failure_as_the_screen_does() {
    let launch = read_repo_file(LAUNCH_SOURCE);
    let reason =
        rust_string_after(&launch, SPAWN_FAILED_KEY).expect("起こせなかったときの理由を拾えること");

    let (_, literals) = lex_screen(&read_repo_file(LAUNCH_LABEL_SOURCE));
    let failed_template = format!("{FAILED_PREFIX}${{");
    assert!(
        literals
            .iter()
            .any(|body| body.starts_with(&failed_template)),
        "{LAUNCH_LABEL_SOURCE} に `{failed_template}…` の文面が無い（README の前置きを見直すこと）"
    );
    let log_suffix = format!("（ログ: {UPSTREAM_LOG_DIR_NAME} の ");
    assert!(
        literals
            .iter()
            .any(|body| body.contains(&format!("{log_suffix}${{"))),
        "{LAUNCH_LABEL_SOURCE} に `{log_suffix}${{…}}）` の文面が無い（README のログの案内を見直すこと）"
    );

    let readme = dist_readme();
    let shown = format!("`{FAILED_PREFIX}{reason}`");
    assert!(
        readme.contains(&shown),
        "{DIST_README} が、起こせなかったときに画面に出る文面 {shown} をそのまま書いていない"
    );
    assert!(
        readme.contains(&format!("`{log_suffix}")),
        "{DIST_README} が、ログのファイル名が付く形 `{log_suffix}…）` を書いていない"
    );
}

/// **README に書いた保存先の名前が、実行ファイルの実際の名前と同じこと**（レビュー1周目の提案）。
///
/// レビューの変異 R3（`UPSTREAM_LOG_DIR_NAME` を変える）が素通りしていた。
/// 見るもの:
/// * `%LOCALAPPDATA%\<名前>` の `<名前>` が全部 [`APP_DATA_DIR_NAME`]（1つ以上）。
/// * `%LOCALAPPDATA%\personacore\<名前>` の `<名前>` が全部 [`UPSTREAM_LOG_DIR_NAME`]（1つ以上）。
/// * 「保存される場所」の**表の行の名前の列**が、上から [`DATABASE_FILE_NAME`] /
///   [`UPSTREAMS_FILE_NAME`] / [`UPSTREAM_LOG_DIR_NAME`]`\` と同じ（**表の行に絞る** ——
///   README のどこかに在れば通る形だと、手順3の「見本（`upstreams.json`）」が同じ綴りを持つので、
///   表の行だけを書き換える変異 P1a が緑だった / レビュー2周目 要修正1）。
#[test]
fn the_distributed_readme_names_the_files_the_executable_actually_writes() {
    let readme = dist_readme();
    let data_dir = names_after(&readme, "%LOCALAPPDATA%\\");
    assert!(
        !data_dir.is_empty() && data_dir.iter().all(|name| *name == APP_DATA_DIR_NAME),
        "{DIST_README} の `%LOCALAPPDATA%\\…` が {APP_DATA_DIR_NAME} を指していない: {data_dir:?}"
    );
    let under = names_after(&readme, &format!("%LOCALAPPDATA%\\{APP_DATA_DIR_NAME}\\"));
    let named: Vec<&str> = under.into_iter().filter(|name| !name.is_empty()).collect();
    assert!(
        !named.is_empty() && named.iter().all(|name| *name == UPSTREAM_LOG_DIR_NAME),
        "{DIST_README} の保存先の下の名前が {UPSTREAM_LOG_DIR_NAME} と違う: {named:?}"
    );
    assert_eq!(
        table_names_in_section(&readme, STORAGE_SECTION),
        Some(vec![
            DATABASE_FILE_NAME.to_owned(),
            UPSTREAMS_FILE_NAME.to_owned(),
            format!("{UPSTREAM_LOG_DIR_NAME}\\"),
        ]),
        "{DIST_README} の「{STORAGE_SECTION}」の表の名前が、実行ファイルの名前と違う"
    );
}

// ----------------------------------------------------------------------
// zip（植えた exe で `package-zip.ps1` を走らせる。**Windows だけ**）
// ----------------------------------------------------------------------

/// zip の中央ディレクトリの1件。
#[cfg(windows)]
#[derive(Debug, PartialEq, Eq)]
struct ZipEntry {
    name: String,
    crc32: u32,
    size: u64,
}

/// **zip の中央ディレクトリを読む**（依存を足さないため、自前で読む）。
///
/// 読むのは「終わりの印（EOCD）」と、そこが指す中央ディレクトリの各項目の
/// 名前・CRC-32・展開後の大きさだけ。**ZIP64 は読まない**（中身は 4 GiB 未満）。
#[cfg(windows)]
fn zip_entries(bytes: &[u8]) -> Vec<ZipEntry> {
    fn u16_at(bytes: &[u8], at: usize) -> usize {
        usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
    }
    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    let eocd = (0..=bytes.len().saturating_sub(22))
        .rev()
        .find(|&at| bytes[at..].starts_with(&[0x50, 0x4b, 0x05, 0x06]))
        .expect("zip の終わりの印（EOCD）が在ること");
    let count = u16_at(bytes, eocd + 10);
    let mut at =
        usize::try_from(u32_at(bytes, eocd + 16)).expect("オフセットが usize に収まること");

    let mut entries = Vec::new();
    for _ in 0..count {
        assert!(
            bytes[at..].starts_with(&[0x50, 0x4b, 0x01, 0x02]),
            "中央ディレクトリの項目の印が在ること"
        );
        let crc32 = u32_at(bytes, at + 16);
        let size = u64::from(u32_at(bytes, at + 24));
        let name_len = u16_at(bytes, at + 28);
        let extra_len = u16_at(bytes, at + 30);
        let comment_len = u16_at(bytes, at + 32);
        let name = String::from_utf8(bytes[at + 46..at + 46 + name_len].to_vec())
            .expect("項目の名前が UTF-8 であること");
        entries.push(ZipEntry { name, crc32, size });
        at += 46 + name_len + extra_len + comment_len;
    }
    entries
}

/// CRC-32（zip と同じ IEEE の多項式）。
#[cfg(windows)]
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// そのファイルの中身を、zip の項目と同じ形で表す。
#[cfg(windows)]
fn entry_for(name: &str, path: &Path) -> ZipEntry {
    let bytes =
        std::fs::read(path).unwrap_or_else(|err| panic!("{} を読めること: {err}", path.display()));
    ZipEntry {
        name: name.to_owned(),
        crc32: crc32(&bytes),
        size: u64::try_from(bytes.len()).expect("大きさが u64 に収まること"),
    }
}

/// `package-zip.ps1` を `pwsh` で走らせる（**`package.ps1` と同じ5つの引数だけを渡す**）。
#[cfg(windows)]
fn run_zip_script(
    exe: &Path,
    readme: &Path,
    notices: &Path,
    licenses: &Path,
    out: &Path,
    extra_env: &[(&str, &Path)],
) -> std::process::Output {
    let mut command = std::process::Command::new("pwsh");
    command
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(repo_path(ZIP_SCRIPT))
        .arg("-ExePath")
        .arg(exe)
        .arg("-ReadmePath")
        .arg(readme)
        .arg("-NoticesPath")
        .arg(notices)
        .arg("-LicenseDir")
        .arg(licenses)
        .arg("-OutDir")
        .arg(out);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    command
        .output()
        .expect("pwsh を起こせること（配布物を作る機には在る）")
}

/// ディレクトリの中のファイル名を全部返す。
#[cfg(windows)]
fn file_names(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .expect("ディレクトリを読めること")
        .map(|entry| {
            entry
                .expect("項目を読めること")
                .file_name()
                .into_string()
                .expect("名前が UTF-8 であること")
        })
        .collect()
}

/// **開発用の DB・プロファイル・シンボル・別の README・別の著作権表示** を、そのディレクトリに植える。
///
/// **zip に入ってはいけないものの代表。** exe の隣と出力先の両方に植える
/// （「exe のディレクトリを丸ごと詰める」「出力先を丸ごと詰める」の両方を落とす）。
#[cfg(windows)]
fn plant_decoys(dir: &Path) {
    for (name, body) in [
        ("personacore.sqlite", "開発用の DB"),
        ("personacore.sqlite-wal", "開発用の DB の WAL"),
        ("upstreams.json", "{\"mine\": {}}"),
        ("personacore.pdb", "シンボル"),
        (
            ZIP_README_ENTRY,
            "植えた別の README（これが入ったら -ReadmePath を見ていない）",
        ),
        (
            ZIP_NOTICES_ENTRY,
            "植えた別の表示（これが入ったら -NoticesPath を見ていない）",
        ),
    ] {
        std::fs::write(dir.join(name), body).expect("囮を植えられること");
    }
    for name in ZIP_LICENSE_ENTRIES
        .into_iter()
        .chain([ZIP_DISCLOSURE_ENTRY])
    {
        std::fs::write(
            dir.join(name),
            format!("植えた別の {name}（これが入ったら -LicenseDir を見ていない）"),
        )
        .expect("囮を植えられること");
    }
}

/// **zip の中身は exe と README と第三者の著作権表示と、ライセンスの文書5つと AI 協働の開示の文書きりで、
/// どれも渡したものそのもの**（Issue origin#25 / 表示は O-1 / Issue origin#48 / ライセンスの文書は
/// O-2 / Issue origin#49 / 開示の文書は O-3 / Issue origin#50）。
///
/// # 見るもの
///
/// * **項目は [`ZIP_EXE_ENTRY`] と [`ZIP_README_ENTRY`] と [`ZIP_NOTICES_ENTRY`] と
///   [`ZIP_LICENSE_ENTRIES`] と [`ZIP_DISCLOSURE_ENTRY`] の9つきり**（重複も落とす）。
///   **exe の隣と出力先に植えた囮（DB・`upstreams.json`・シンボル・別の README・別の表示・
///   別のライセンスの文書・別の開示の文書）は入らない。**
///   **名指したもの以外は詰めない設計は、2つだったときと同じ**（`package-zip.ps1` の `$ZipEntries`）。
/// * **中身は渡したファイルのバイト列そのもの**（CRC-32 と大きさ）。
///   README は `scripts/README-dist.md` であって、exe の隣の `README.md` ではない。
/// * **表示は `-NoticesPath` で渡したもの**で、スクリプトの隣の `scripts/THIRD-PARTY-NOTICES.txt` ではない
///   （渡すのは一時ディレクトリに植えた別の中身。本物を渡すと、スクリプトが隣の本物を
///   決め打ちで読む形と区別できない）。表示の中身の見張りは `tests/notices.rs`。
/// * **ライセンスの文書と開示の文書は `-LicenseDir` で渡したディレクトリのもの**で、リポジトリの根の本物ではない
///   （同じ理由で、一時ディレクトリに植えた別の中身を渡す）。文書の中身の見張りは `tests/license.rs`。
/// * **名前の版は、中の exe が `--version` で名乗るものから作られている**
///   （この exe の名乗りは [`VERSION_LINE`]。期待値はスクリプトの外で組む）。
/// * **同じ名前の古い zip（壊れた中身）が在っても、置き換わる。**
/// * **出力先に、途中のファイルが残らない。**
#[cfg(windows)]
#[test]
fn the_zip_holds_exactly_the_executable_the_readme_the_notices_and_the_licenses() {
    let root = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let bin = root.path().join("bin");
    let out = root.path().join("出力 先");
    std::fs::create_dir_all(&bin).expect("作れること");
    std::fs::create_dir_all(&out).expect("作れること");

    let exe = bin.join(ZIP_EXE_ENTRY);
    std::fs::copy(env!("CARGO_BIN_EXE_personacore"), &exe).expect("exe を複製できること");
    plant_decoys(&bin);
    plant_decoys(&out);

    let expected_name = zip_name_for(VERSION_LINE).expect("この exe の名乗りから名前を組めること");
    std::fs::write(out.join(&expected_name), "壊れた古い zip").expect("古い zip を植えられること");

    let readme = repo_path(DIST_README);
    let notices = root.path().join("notices-planted.txt");
    std::fs::write(&notices, "植えた表示（-NoticesPath で渡したもの）")
        .expect("表示を植えられること");
    assert_ne!(
        std::fs::read(&notices).expect("読めること"),
        std::fs::read(repo_path(DIST_NOTICES)).expect("本物の表示を読めること"),
        "植えた表示が本物と同じ中身（取り違えを検出できない）"
    );
    let licenses = root.path().join("ライセンス の文書");
    std::fs::create_dir_all(&licenses).expect("作れること");
    for name in ZIP_LICENSE_ENTRIES
        .into_iter()
        .chain([ZIP_DISCLOSURE_ENTRY])
    {
        let planted = licenses.join(name);
        std::fs::write(
            &planted,
            format!("植えた {name}（-LicenseDir で渡したもの）"),
        )
        .expect("ライセンスの文書を植えられること");
        assert_ne!(
            std::fs::read(&planted).expect("読めること"),
            std::fs::read(repo_path(name)).expect("本物のライセンスの文書を読めること"),
            "植えた {name} が本物と同じ中身（取り違えを検出できない）"
        );
    }
    let output = run_zip_script(&exe, &readme, &notices, &licenses, &out, &[]);
    assert!(
        output.status.success(),
        "package-zip.ps1 が失敗した: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // **出力先に増えたのは zip 1つだけ**（途中のファイルが残らない）。
    let mut expected_files: BTreeSet<String> = file_names(&bin)
        .into_iter()
        .filter(|name| name != ZIP_EXE_ENTRY)
        .collect();
    expected_files.insert(expected_name.clone());
    assert_eq!(file_names(&out), expected_files, "出力先の中身が違う");

    let zip = std::fs::read(out.join(&expected_name)).expect("zip を読めること");
    let mut entries = zip_entries(&zip);
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    // **期待の名前は [`ZIP_NAMED_ENTRIES`] と [`ZIP_LICENSE_ENTRIES`] から引く**（README の見張りと同じ配列）。
    // 元のファイルは名前ごとに、このテストが渡したもの。
    let source_of = |name: &str| -> PathBuf {
        match name {
            ZIP_EXE_ENTRY => exe.clone(),
            ZIP_README_ENTRY => readme.clone(),
            ZIP_NOTICES_ENTRY => notices.clone(),
            ZIP_DISCLOSURE_ENTRY => licenses.join(name),
            other => {
                panic!("{other} の元のファイルをこのテストが渡していない（ここに腕を足すこと）")
            }
        }
    };
    let mut expected: Vec<ZipEntry> = ZIP_NAMED_ENTRIES
        .into_iter()
        .map(|name| entry_for(name, &source_of(name)))
        .collect();
    for name in ZIP_LICENSE_ENTRIES {
        expected.push(entry_for(name, &licenses.join(name)));
    }
    expected.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        entries, expected,
        "zip の中身が「渡した exe と README と表示とライセンスの文書5つと AI 協働の開示の文書きり」でない"
    );
}

/// **`--version` で名乗れない exe からは、zip を作らない。**
///
/// **スクリプトが exe 自身に版を聞いていること**も、ここで見る —— 植えた偽物
/// （`fake_recorder`。何も印字せずに終わる）が **`--version` の1つだけで呼ばれたこと**。
/// 版を別のところ（`Cargo.toml` 等）から読む形にすると、偽物は呼ばれず、zip が出来る。
#[cfg(windows)]
#[test]
fn an_executable_that_does_not_name_itself_makes_no_zip() {
    let root = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let out = root.path().join("out");
    std::fs::create_dir_all(&out).expect("作れること");
    let exe = root.path().join(ZIP_EXE_ENTRY);
    std::fs::copy(env!("CARGO_BIN_EXE_fake_recorder"), &exe).expect("偽物を複製できること");
    let log = root.path().join("calls.log");

    let output = run_zip_script(
        &exe,
        &repo_path(DIST_README),
        &repo_path(DIST_NOTICES),
        &repo_root(),
        &out,
        &[("FAKE_RECORDER_LOG", &log)],
    );
    assert!(
        !output.status.success(),
        "名乗れない exe なのに成功した: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        file_names(&out),
        BTreeSet::new(),
        "名乗れない exe なのに何か作った"
    );

    let calls =
        std::fs::read_to_string(&log).expect("偽物が呼ばれていること（版を exe に聞いていない）");
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        vec![r#"["--version"]"#],
        "exe への問い合わせが `--version` の1回でない"
    );
}

/// **x64 でない exe からは、zip を作らない**（名前が `windows-x64` を名乗るため）。
///
/// 植えるのは本物の exe の複製で、**PE ヘッダの機種だけを ARM64（`0xAA64`）へ書き換えたもの。**
/// **書き換えた exe はこの機では起動できない**ので、`--version` の失敗でも止まりうる。
/// **機種の検査で止まったことは、その文面に出る機種の綴り（`0xAA64`）で見る**
/// （検査を消すと、起動の失敗という別の文面で止まる）。
///
/// **⚠ 日本語の文面では見ない** —— 出力を受け取った `pwsh` はコンソールの符号化（この機は CP932）で
/// 書くので、UTF-8 として読むと化ける。**ASCII の綴りだけを当てにする。**
#[cfg(windows)]
#[test]
fn an_executable_for_another_architecture_makes_no_zip() {
    let root = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let out = root.path().join("out");
    std::fs::create_dir_all(&out).expect("作れること");

    let mut bytes = std::fs::read(env!("CARGO_BIN_EXE_personacore")).expect("exe を読めること");
    let pe = usize::try_from(u32::from_le_bytes([
        bytes[0x3c],
        bytes[0x3d],
        bytes[0x3e],
        bytes[0x3f],
    ]))
    .expect("usize に収まること");
    assert_eq!(&bytes[pe..pe + 4], b"PE\0\0", "植える元が PE であること");
    assert_eq!(
        u16::from_le_bytes([bytes[pe + 4], bytes[pe + 5]]),
        0x8664,
        "植える元が x64 であること（対照）"
    );
    bytes[pe + 4..pe + 6].copy_from_slice(&0xAA64_u16.to_le_bytes());
    let exe = root.path().join(ZIP_EXE_ENTRY);
    std::fs::write(&exe, &bytes).expect("書き換えた exe を植えられること");

    let output = run_zip_script(
        &exe,
        &repo_path(DIST_README),
        &repo_path(DIST_NOTICES),
        &repo_root(),
        &out,
        &[],
    );
    assert!(!output.status.success(), "x64 でない exe なのに成功した");
    assert_eq!(
        file_names(&out),
        BTreeSet::new(),
        "x64 でない exe なのに何か作った"
    );
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        said.contains("0xAA64"),
        "機種の検査で止まっていない: {said}"
    );
}

/// **zip を組む途中で失敗したら、途中のファイル（`.partial`）を残さずに止まる**（レビュー1周目 要修正3）。
///
/// 失敗の起こし方: **README の複製を、共有を許さずに開いたまま走らせる。**
/// `Test-Path` は通り、exe の項目は書けるが、README の項目を詰めるところで読めずに失敗する
/// （＝ `.partial` を作った**後**の失敗）。
///
/// レビューの変異 R5（`catch` の中の後始末を `if ($false)` にする）が、この腕が無いと全緑だった。
///
/// **⚠ 失敗が `.partial` を作った後で起きたこと自体は、この腕の中では観測していない**
/// （出力の文面は CP932 で化けるので読まない）。**それを言うのは R5 の変異が赤になること**である。
#[cfg(windows)]
#[test]
fn a_failure_while_zipping_leaves_no_partial_file() {
    use std::os::windows::fs::OpenOptionsExt as _;

    let root = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let out = root.path().join("out");
    std::fs::create_dir_all(&out).expect("作れること");
    let exe = root.path().join(ZIP_EXE_ENTRY);
    std::fs::copy(env!("CARGO_BIN_EXE_personacore"), &exe).expect("exe を複製できること");
    let readme = root.path().join("README-locked.md");
    std::fs::copy(repo_path(DIST_README), &readme).expect("README を複製できること");

    // **共有を許さずに開いたまま**（`share_mode(0)`）。このハンドルが生きている間、誰も読めない。
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&readme)
        .expect("README の複製を排他で開けること");

    let output = run_zip_script(
        &exe,
        &readme,
        &repo_path(DIST_NOTICES),
        &repo_root(),
        &out,
        &[],
    );
    drop(lock);

    assert!(
        !output.status.success(),
        "README を読めないのに成功した: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        file_names(&out),
        BTreeSet::new(),
        "途中で失敗したのに、出力先に何か残っている（`.partial` の後始末が効いていない）"
    );
}
