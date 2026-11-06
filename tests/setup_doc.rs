//! `docs/09-setup.md`（各環境のセットアップ）が名指しするものを、実体と突き合わせる。
//!
//! # なぜ要るか（作業単位 F-3）
//!
//! この文書の本質的な危険は1つしかない ——**歩いていない道を「歩いた」と書くこと。**
//! 実際に踏んだ: 根の README（F-2）は**手順の順番を実体と逆に書き**、
//! **ブラウザが開くと断定していた**（開かない）。**どちらも、書いた人が歩いていないから出た。**
//!
//! だから `09-setup.md` は「**要求**」と「**歩いた道**」を構造として分けてある。
//! ここが機械的に見るのは、**その構造が保たれていること**と、
//! **名指しした先が実在すること**である。
//!
//! # 何を守っていないか（**先に書く**）
//!
//! * **手順を1つも実行していない。** 綴りの突き合わせだけである。
//!   「その順で本当に通る」ことは、ここでは一切見ていない
//!   （実際に歩いた範囲は `docs/09-setup.md` の §歩いた道が持つ）。
//! * **名乗り（[`WALK_PROVENANCE`]）は、10 欄のうち 2 欄しか実体と突き合わせていない。**
//!   **突き合わせているのは [`WALK_PROVENANCE_DECLARED`] の `pnpm` と `PowerShell` だけ**で、
//!   残りは「**欄が埋まっている**」しか見ていない ——`rustc` / OS / GPU / 日付 / モデル /
//!   リビジョン / 上流の版を書き換えても鳴らない。
//!
//!   **⚠ 初版は「突き合わせる相手がリポジトリの中に無いので直せない」と書いていた。**
//!   **それは 10 欄のうち 8 欄にしか当たらない誤りだった**（F-3 のレビューが実測）——
//!   **`pnpm` には `web/package.json` の `packageManager` が、**
//!   **`PowerShell` には `scripts/start.ps1` の `#Requires` が在る。**
//!   **どちらも嘘の版に書き換えて 1861 全緑を通っていた。**
//!   **この家で最多の欠陥（doc が、守られていない場所に「守った／直せない」と書く）である。**
//! * **「歩いていない道」に手順が無いことは、コードブロックの有無と
//!   [`WALKED_STEPS`] の綴りでしか見ていない。**
//!   **その綴りを使わずに散文で手順を書けば素通りする。**
//! * **リンクは行き先が在ることしか見ていない。** 錨（`#…`）は落として読むので、
//!   **節が消えても鳴らない。**
//! * **`cargo` の並びを [`CLAUDE.md`] と突き合わせていない**（`tests/readme.rs` はやっている）。
//!   §歩いた道は `cargo test --no-fail-fast` で回しており、**あちらの要求とは並びが違う。**
//!   ここで突き合わせると「歩いた事実」を「規約に合わせて書き換える」圧力になるので、やらない。
//! * **散文の主張は1つも見ていない。**
//! * **⚠ 床と帳簿そのものは無防備。** 挙げると次の5つで、例外は無い ——
//!   [`LEAST_UNWATCHED`] / [`WALK_PROVENANCE`] / [`WALK_PROVENANCE_DECLARED`] /
//!   [`WALKED_STEPS`] / [`POINTED_CONSTANTS`]。
//!   **下げれば通る。** [`the_ledgers_name_each_thing_once`] が**重複だけ**は止めるが、
//!   **減らす手を止めるものは無い**（`tests/readme.rs` と同じ限界）。
//!
//! # 走査の作法（この家で実際に事故が出たものだけ）
//!
//! * **走査本体はすべてソースを引数で受け取る。** 実ファイルと**植えたソース**を
//!   同じ本体に通すため（`15b`: 実ファイルだけを見る対照は自己成就する）。
//! * **植えたソースは LF と CRLF の両方で用意する。**
//!   `E-1`（2026-09-02）で、**作業者のツリー（LF）で 1877 全緑・`main`（CRLF）で 1 赤**が出た。
//!   **clone 直後は全ファイルが CRLF になる**ので、誰がクローンしても踏む形だった。
//!   **綴りに `\n` を含む走査を書かない**（ここは全部 [`str::lines`] の上に組んである）。
//! * **走査の綴りが、走査される文書の散文で満たされないこと。**
//!   [`the_setup_doc_points_at_the_upstream_requirements_instead_of_copying_them`] は
//!   **綴りを ADR の見出しと突き合わせる**ので、
//!   「散文が満たす一般的な綴り」へ緩めると**あちらで見つからなくなって赤になる。**

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::path::PathBuf;

use personacore::llm::DEFAULT_ENDPOINT;
use personacore::server::{DEFAULT_PORT, ENV_LLM_ENDPOINT};

// ----------------------------------------------------------------------
// 見る相手
// ----------------------------------------------------------------------

/// この見張り番の対象。
const SETUP_DOC: &str = "docs/09-setup.md";

/// 根の README（ここから辿れること）。
const README: &str = "README.md";

/// docs の索引（ここから辿れること）。
const DOCS_INDEX: &str = "docs/README.md";

/// 上流への要求の**権威**。
const UPSTREAM_ADR: &str = "docs/adr/0038-what-personacore-requires-of-an-upstream.md";

/// `docs/` から見た、同じ ADR の相対パス（文書が書くのはこちら）。
const UPSTREAM_ADR_LINK: &str = "adr/0038-what-personacore-requires-of-an-upstream.md";

/// 3層の表が載っている節の**見出しそのもの**。
///
/// **一般的な綴り（「決定」など）へ緩めてはいけない** ——
/// ADR の見出しと突き合わせているので、緩めた瞬間に見つからなくなって鳴る。
const UPSTREAM_SECTION: &str = "1. **上流に要求する能力を、3層に分けて名前を付ける**";

/// 環境変数の綴りを定義しているソース。
const CONFIG_SOURCE: &str = "src/server/config.rs";

/// 起動スクリプト（要求する PowerShell の版の権威）。
const START_SCRIPT: &str = "scripts/start.ps1";

/// pnpm の版の権威。
const WEB_PACKAGE_JSON: &str = "web/package.json";

/// `web/package.json` が pnpm の版を書いている形。
const PACKAGE_MANAGER_PREFIX: &str = "\"packageManager\": \"pnpm@";

// ----------------------------------------------------------------------
// 節の見出し。**この4つの並びに、見張り番が寄りかかっている。**
// ----------------------------------------------------------------------

/// 要求の節。
const REQUIREMENT_SECTION: &str = "要求 —— 環境が満たすこと";

/// **手順を書いてはいけない**節。
const UNTRODDEN_SECTION: &str = "歩いていない道";

/// **実際に歩いた1通り**を書く節。
const WALKED_SECTION: &str = "歩いた道";

/// 「見ていないこと」を並べる節。
const UNWATCHED_SECTION: &str = "この文書が見ていないこと";

/// 上流への要求を**指すだけ**の小節。**ここに表を置かせない。**
const UPSTREAM_SUBSECTION: &str = "上流（推論サーバ）";

// ----------------------------------------------------------------------
// 帳簿と床（**ここ自身は無防備。module doc に書いた**）
// ----------------------------------------------------------------------

/// §歩いた道が**名乗らねばならない欄**（[ADR-0033](../docs/adr/0033-measured-constants-name-what-they-were-measured-on.md) の作法）。
const WALK_PROVENANCE: [&str; 10] = [
    "日付",
    "リビジョン",
    "OS",
    "GPU",
    "rustc",
    "pnpm",
    "Node",
    "PowerShell",
    "上流",
    "モデル",
];

/// **リポジトリの中に権威が在る**名乗りの欄。**ここは値まで突き合わせる。**
///
/// **残りの欄には権威が無い**（`rustc` の版 / OS / GPU / 日付 / モデル / リビジョン / 上流）。
/// **「無い」と言えるのはこの2つを除いた後だけである** ——初版はそれを 10 欄全部に広げていた。
const WALK_PROVENANCE_DECLARED: [&str; 2] = ["pnpm", "PowerShell"];

/// §歩いた道のコードブロックに、**この順で**現れねばならない命令。
///
/// **順番が主張の一部である** —— F-2 で間違えたのがまさにここだった。
const WALKED_STEPS: [&str; 7] = [
    "git clone",
    "cargo build",
    "cargo test",
    "pnpm install",
    "pnpm run build",
    "pwsh ./scripts/start.ps1",
    "/v1/chat/completions",
];

/// **値ではなく在処を書かせる**定数。
const POINTED_CONSTANTS: [&str; 2] = ["DEFAULT_PORT", "DEFAULT_ENDPOINT"];

/// 「見ていないこと」の床。**1件落としたら鳴る値**にしてある。
const LEAST_UNWATCHED: usize = 10;

/// `PERSONACORE_…` の接頭辞。
const ENV_NAME_PREFIX: &str = "PERSONACORE_";

/// `config.rs` の環境変数定数の接頭辞。
const ENV_CONST_PREFIX: &str = "pub const ENV_";

// ----------------------------------------------------------------------
// 走査の本体。**すべてソースを引数で受け取る。**
// **すべて `lines()` の上に組む** —— 綴りに `\n` を入れると CRLF で一致しなくなる（E-1）。
// ----------------------------------------------------------------------

/// markdown の見出し行を `(段, 見出し文字列)` として読む。見出しでなければ `None`。
///
/// `trim_end` が `\r` を落とすので、**CRLF でも同じ答えを返す。**
fn heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_end();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let text = trimmed[hashes..].strip_prefix(' ')?.trim();
    if text.is_empty() {
        None
    } else {
        Some((hashes, text))
    }
}

/// 見出し `title` の節の**中身の行**を返す（見出し行そのものは含まない）。
///
/// **同じ段か、より浅い見出しで止まる。** 深い見出し（小節）は中身として含む。
/// **コードブロックの中の `#` は見出しとして読まない。**
///
/// # ⚠ ここを「末尾まで読む」に緩めると何が起きるか
///
/// [`the_untrodden_section_writes_no_procedure`] が、
/// **後ろにある §歩いた道のコードブロックを「歩いていない道の中にある」と読んで赤になる。**
/// **節の順番に寄りかかっているのは、この一点である**（`E-1` で同型の事故が出ている）。
fn section_body<'a>(source: &'a str, title: &str) -> Option<Vec<&'a str>> {
    let mut body = Vec::new();
    let mut level = None;
    let mut fenced = false;

    for line in source.lines() {
        let fence = line.trim_start().starts_with("```");
        if fence {
            fenced = !fenced;
        }
        let head = if fenced || fence { None } else { heading(line) };

        match (level, head) {
            (None, Some((found, text))) if text == title => level = Some(found),
            (Some(open), Some((found, _))) if found <= open => break,
            (Some(_), _) => body.push(line),
            _ => {}
        }
    }

    level.map(|_| body)
}

/// ``` で囲まれた中の行を返す。
fn fenced_code_lines<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let mut inside = false;
    let mut found = Vec::new();
    for line in lines {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        if inside {
            found.push(*line);
        }
    }
    found
}

/// 行頭の箇条書き（`- ` / `* `）を返す。**折り返しの続き（字下げ）は数えない。**
fn bullets<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let mut inside = false;
    let mut found = Vec::new();
    for line in lines {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        if !inside && (line.starts_with("- ") || line.starts_with("* ")) {
            found.push(*line);
        }
    }
    found
}

/// 表の行を「セルの並び」として返す。**両端の空セルは落とす。**
fn table_rows(lines: &[&str]) -> Vec<Vec<String>> {
    lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| line.starts_with('|'))
        .map(|line| {
            line.trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_owned())
                .collect()
        })
        .collect()
}

/// 強調の `*` と前後の空白を落とした見出し文字列。
fn plain(cell: &str) -> &str {
    cell.trim().trim_matches('*').trim()
}

/// markdown の**相対リンク**を「表示文字列, 行き先」で書いてある順に返す。
///
/// `http://` `https://` `mailto:` と、同じ文書の中の錨（`#…`）は落とす。
fn relative_links(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (at, _) in source.match_indices("](") {
        let Some(open) = source[..at].rfind('[') else {
            continue;
        };
        let rest = &source[at + 2..];
        let Some(end) = rest.find(')') else {
            continue;
        };
        let target = &rest[..end];
        let external = target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with("mailto:")
            || target.starts_with('#');
        if external {
            continue;
        }
        let file = target.split('#').next().unwrap_or(target);
        if file.is_empty() {
            continue;
        }
        found.push((source[open + 1..at].to_owned(), file.to_owned()));
    }
    found
}

/// 1行の中で、バッククォートで囲まれた区間を書いてある順に返す。
fn backticked(line: &str) -> Vec<&str> {
    line.split('`').skip(1).step_by(2).collect()
}

/// 「`<パス>.rs` の `<定数名>`」という**指し先**を、書いてある順に返す。
fn rust_const_pointers(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for line in source.lines() {
        let segments = backticked(line);
        for pair in segments.windows(2) {
            let (path, name) = (pair[0], pair[1]);
            let named = name.starts_with(|ch: char| ch.is_ascii_uppercase())
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_');
            if path.ends_with(".rs") && named {
                found.push((path.to_owned(), name.to_owned()));
            }
        }
    }
    found
}

/// Rust のソースから `pub const <名前> … = <値>;` の値を読む。
fn declared_const_value<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let at = source.find(&format!("pub const {name}"))?;
    let rest = &source[at..];
    let equals = rest.find("= ")?;
    let end = rest.find(';')?;
    if equals > end {
        return None;
    }
    Some(rest[equals + 2..end].trim().trim_matches('"'))
}

/// ソースに現れる `PERSONACORE_…` の綴りを全部拾う（後ろへ伸ばす）。
fn personacore_env_names(source: &str) -> BTreeSet<&str> {
    let mut found = BTreeSet::new();
    for (at, _) in source.match_indices(ENV_NAME_PREFIX) {
        let rest = &source[at..];
        let len = rest
            .find(|ch: char| !(ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_'))
            .unwrap_or(rest.len());
        found.insert(&rest[..len]);
    }
    found
}

/// `config.rs` が `pub const ENV_… = "…";` で定義している綴りを全部拾う。
fn defined_env_names(source: &str) -> BTreeSet<&str> {
    let mut found = BTreeSet::new();
    for (at, _) in source.match_indices(ENV_CONST_PREFIX) {
        let rest = &source[at..];
        let Some(end) = rest.find(';') else {
            continue;
        };
        let statement = &rest[..end];
        let Some(open) = statement.find('"') else {
            continue;
        };
        let inside = &statement[open + 1..];
        let Some(close) = inside.find('"') else {
            continue;
        };
        found.insert(&inside[..close]);
    }
    found
}

/// ソースに現れる IPv4 の**リテラル**を全部拾う。
fn ipv4_literals(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let bytes = source.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if !bytes[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b'.') {
            at += 1;
        }
        let run = source[start..at].trim_end_matches('.');
        if let Ok(addr) = run.parse::<Ipv4Addr>() {
            found.insert(addr.to_string());
        }
    }
    found
}

/// 文書が名指ししている `scripts/….ps1` を全部拾う。
fn script_paths(source: &str) -> BTreeSet<String> {
    const HEAD: &str = "scripts/";
    let mut found = BTreeSet::new();
    for (at, _) in source.match_indices(HEAD) {
        let rest = &source[at..];
        let len = rest
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || "/._-".contains(ch)))
            .unwrap_or(rest.len());
        let path = &rest[..len];
        if path.ends_with(".ps1") {
            found.insert(path.to_owned());
        }
    }
    found
}

/// `web/package.json` が宣言している pnpm の版を読む。
///
/// **`packageManager` は pnpm 自身が読む欄**なので、写しではなく実体である。
fn declared_pnpm_version(source: &str) -> Option<&str> {
    let at = source.find(PACKAGE_MANAGER_PREFIX)?;
    let rest = &source[at + PACKAGE_MANAGER_PREFIX.len()..];
    let end = rest.find('"')?;
    let version = &rest[..end];
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

/// 版の綴りの**メジャー**を読む（`7.6.5` → `7`）。
fn major_of(version: &str) -> Option<u32> {
    version.trim().split('.').next()?.parse().ok()
}

/// PowerShell スクリプトが要求する版の**メジャー**を読む。
fn required_powershell_major(script: &str) -> Option<u32> {
    const KEY: &str = "#Requires -Version ";
    let at = script.find(KEY)?;
    let rest = &script[at + KEY.len()..];
    let end = rest
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .unwrap_or(rest.len());
    rest[..end].split('.').next()?.parse().ok()
}

/// ソースが**名指ししている** PowerShell の版（メジャー）を全部返す。
fn powershell_versions_named(source: &str) -> BTreeSet<u32> {
    const KEY: &str = "PowerShell ";
    let mut found = BTreeSet::new();
    for (at, _) in source.match_indices(KEY) {
        let rest = &source[at + KEY.len()..];
        let end = rest
            .find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(rest.len());
        if let Ok(major) = rest[..end].parse::<u32>() {
            found.insert(major);
        }
    }
    found
}

// ----------------------------------------------------------------------
// 実ファイルの読み込み。**絶対パスで組む**（相対パスはプロセスの作業ディレクトリで
// 解決され、別のツリーを読みうる / CLAUDE.md の `A-1a`）。
// ----------------------------------------------------------------------

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

fn setup_doc() -> String {
    read_repo_file(SETUP_DOC)
}

/// 節を取り出す（**無ければ、その場で落とす**）。
fn body_of(source: &str, title: &str) -> Vec<String> {
    section_body(source, title)
        .unwrap_or_else(|| panic!("{SETUP_DOC} に「{title}」の節が無い"))
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn as_refs(lines: &[String]) -> Vec<&str> {
    lines.iter().map(String::as_str).collect()
}

/// §歩いた道の名乗りの表から、1欄の値を取る（**無ければ、その場で落とす**）。
fn provenance_value(doc: &str, field: &str) -> String {
    let body = body_of(doc, WALKED_SECTION);
    let row = table_rows(&as_refs(&body))
        .into_iter()
        .find(|row| row.len() >= 2 && plain(&row[0]) == field)
        .unwrap_or_else(|| panic!("§{WALKED_SECTION} が「{field}」を名乗っていない"));
    plain(&row[1]).to_owned()
}

/// 植えたソースを **LF と CRLF の両方**で返す。
///
/// **`E-1`（2026-09-02）で、LF の対照だけが実ファイルより弱かった。**
fn both_newlines(source: &str) -> [String; 2] {
    let lf = source.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    [lf, crlf]
}

// ----------------------------------------------------------------------
// 走査本体の対照（**植えたソースを、実ファイルと同じ本体に通す**）
// ----------------------------------------------------------------------

/// `section_body` は、**同じ段か浅い見出しで止まる**こと。
///
/// **末尾まで読む形に緩めたら、ここが鳴る。**
#[test]
fn the_section_scan_stops_at_the_next_heading_of_the_same_depth() {
    const PLANTED: &str = "\
# 題
## 前
まえ
## 的
なか
### 小節
ふかい
## 後
うしろ
";

    for source in both_newlines(PLANTED) {
        let body = section_body(&source, "的").expect("「的」の節が在ること");
        let body: Vec<&str> = body.iter().map(|line| line.trim_end()).collect();
        assert_eq!(
            body,
            vec!["なか", "### 小節", "ふかい"],
            "同じ段の見出しで止まり、深い見出しは中身に含めること"
        );

        assert!(
            section_body(&source, "在らぬ節").is_none(),
            "無い見出しは None を返すこと"
        );
    }
}

/// `section_body` は、**コードブロックの中の `#` を見出しとして読まない**こと。
#[test]
fn the_section_scan_ignores_hashes_inside_code_blocks() {
    const PLANTED: &str = "\
## 的
```powershell
# これは見出しではない
pwsh ./scripts/start.ps1
```
おわり
## 後
うしろ
";

    for source in both_newlines(PLANTED) {
        let body = section_body(&source, "的").expect("「的」の節が在ること");
        let body: Vec<&str> = body.iter().map(|line| line.trim_end()).collect();
        assert_eq!(
            body.last(),
            Some(&"おわり"),
            "コードブロックの中の `#` で節を切らないこと"
        );
        assert!(
            !body.contains(&"うしろ"),
            "次の節の中身を取り込まないこと（コードブロックの検出が壊れると起きる）"
        );
    }
}

/// `fenced_code_lines` は、渡された行のコードブロックだけを返すこと。
#[test]
fn the_code_block_scan_reads_the_lines_it_is_given() {
    const PLANTED: &str = "\
散文の `cargo build` は拾わない
```powershell
cargo build
```
また散文
";

    for source in both_newlines(PLANTED) {
        let lines: Vec<&str> = source.lines().collect();
        let code: Vec<&str> = fenced_code_lines(&lines)
            .into_iter()
            .map(str::trim_end)
            .collect();
        assert_eq!(
            code,
            vec!["cargo build"],
            "コードブロックの中だけを返すこと（散文の言及を拾わないこと）"
        );
    }
}

/// `bullets` は、行頭の箇条書きだけを数えること。
#[test]
fn the_bullet_scan_counts_only_top_level_items() {
    const PLANTED: &str = "\
- ひとつ
  続きの行
* ふたつ
```
- これはコードブロックの中
```
段落
";

    for source in both_newlines(PLANTED) {
        let lines: Vec<&str> = source.lines().collect();
        assert_eq!(
            bullets(&lines).len(),
            2,
            "折り返しの続きと、コードブロックの中は数えないこと"
        );
    }
}

/// `table_rows` は、渡された行の表をセルに割ること。
#[test]
fn the_table_scan_reads_the_lines_it_is_given() {
    const PLANTED: &str = "\
段落
| **日付** | 2026-09-03 |
|---|---|
| **OS** | Windows |
| 空 |  |
";

    for source in both_newlines(PLANTED) {
        let lines: Vec<&str> = source.lines().collect();
        let rows = table_rows(&lines);
        assert_eq!(rows.len(), 4, "表の行だけを拾うこと");
        assert_eq!(rows[0], vec!["**日付**", "2026-09-03"]);
        assert_eq!(plain(&rows[2][0]), "OS", "強調の `*` は落として読むこと");
        assert!(rows[3][1].is_empty(), "空のセルは空として読むこと");
    }
}

/// `declared_pnpm_version` は、渡されたソースの `packageManager` を読むこと。
#[test]
fn the_package_manager_scan_reads_the_source_it_is_given() {
    const PLANTED: &str = "\
{
  \"name\": \"植えた\",
  \"packageManager\": \"pnpm@1.2.3\",
  \"scripts\": { \"test\": \"vitest run\" }
}
";

    for source in both_newlines(PLANTED) {
        assert_eq!(
            declared_pnpm_version(&source),
            Some("1.2.3"),
            "`packageManager` の版だけを返すこと"
        );
        assert_eq!(major_of("1.2.3"), Some(1), "メジャーを読むこと");
        assert_eq!(major_of("7"), Some(7), "点が無くても読むこと");
        assert_eq!(major_of("なし"), None, "版でなければ None を返すこと");
    }

    assert_eq!(
        declared_pnpm_version("{ \"name\": \"欄が無い\" }"),
        None,
        "欄が無ければ None を返すこと"
    );
    assert_eq!(
        declared_pnpm_version("{ \"packageManager\": \"pnpm@\" }"),
        None,
        "版が空なら None を返すこと（空文字列を「一致」にしない）"
    );
}

/// `relative_links` は、渡されたソースのリンクを読むこと。
#[test]
fn the_link_scan_reads_the_source_it_is_given() {
    const PLANTED: &str = "\
[植えた案内](docs/planted.md) と [節](docs/planted.md#anchor) と
[外](https://example.com/x) と [同じ文書](#その節)
";

    for source in both_newlines(PLANTED) {
        let links = relative_links(&source);
        assert_eq!(
            links,
            vec![
                ("植えた案内".to_owned(), "docs/planted.md".to_owned()),
                ("節".to_owned(), "docs/planted.md".to_owned()),
            ],
            "相対リンクだけを、錨を落として返すこと"
        );
    }
}

// ----------------------------------------------------------------------
// 実ファイルを見る
// ----------------------------------------------------------------------

/// 帳簿は、同じものを2回挙げていないこと。
///
/// **床と帳簿は無防備だと module doc に書いた。**
/// **せめて「同じ欄を並べて件数だけ満たす」形は止める。**
#[test]
fn the_ledgers_name_each_thing_once() {
    for (name, ledger) in [
        ("WALK_PROVENANCE", WALK_PROVENANCE.to_vec()),
        (
            "WALK_PROVENANCE_DECLARED",
            WALK_PROVENANCE_DECLARED.to_vec(),
        ),
        ("WALKED_STEPS", WALKED_STEPS.to_vec()),
        ("POINTED_CONSTANTS", POINTED_CONSTANTS.to_vec()),
    ] {
        let unique: BTreeSet<&str> = ledger.iter().copied().collect();
        assert_eq!(
            unique.len(),
            ledger.len(),
            "帳簿 {name} が同じものを2回挙げている（件数だけ満たす形になっている）"
        );
    }
}

/// リンクの行き先が実在すること。
#[test]
fn every_link_in_the_setup_doc_points_at_something_that_exists() {
    let doc = setup_doc();
    let links = relative_links(&doc);
    assert!(
        links.len() >= 10,
        "{SETUP_DOC} の相対リンクが少なすぎる（{} 件）——走査が壊れていないか",
        links.len()
    );

    for (text, target) in links {
        let path = repo_path(&format!("docs/{target}"));
        assert!(
            path.exists(),
            "{SETUP_DOC} の「{text}」が指す {} が無い",
            path.display()
        );
    }
}

/// 名指しした `.ps1` が実在すること。
#[test]
fn the_setup_doc_only_names_scripts_that_exist() {
    for path in script_paths(&setup_doc()) {
        assert!(
            repo_path(&path).exists(),
            "{SETUP_DOC} が実在しない {path} を名指ししている"
        );
    }
}

/// **上流への要求は、写さずに指すこと**（[ADR-0038] §決定1）。
///
/// # 何を見ているか（3つ）
///
/// 1. **同じ行**の中で ADR をリンクし、**節の見出しそのもの**を名乗っていること。
///    **リンクと節名が別の行に割れたら、どちらが何を指しているか言えなくなる。**
/// 2. **その見出しが、ADR に実在すること。**
///    ——これが、走査の綴りを緩める変異への守りになっている。
///    「決定」のような**散文が満たす綴り**へ緩めると、**ADR 側で見つからず赤になる。**
/// 3. **あちらの表の行が、こちらに写っていないこと。**
///    権威の表から**セルを読み出して**突き合わせるので、**表が育てば守りも育つ。**
///
/// # ⚠ 守っていないもの
///
/// * **3層の中身が正しいかは見ていない。** それは ADR の側の話である。
/// * **言い換えた写しは捕まえられない。** 見ているのは**逐語の一致**だけ。
/// * **短いセル（10 文字未満）は見ていない** ——「何」のような見出しのセルまで
///   禁止語にすると、文書が普通の日本語を書けなくなる。
///
/// [ADR-0038]: ../docs/adr/0038-what-personacore-requires-of-an-upstream.md
#[test]
fn the_setup_doc_points_at_the_upstream_requirements_instead_of_copying_them() {
    let doc = setup_doc();

    // 1. 同じ行でリンクし、節を名乗っていること。
    let pointer = doc
        .lines()
        .find(|line| {
            line.contains(&format!("]({UPSTREAM_ADR_LINK})")) && line.contains(UPSTREAM_SECTION)
        })
        .unwrap_or_else(|| {
            panic!(
                "{SETUP_DOC} に、{UPSTREAM_ADR_LINK} へのリンクと「{UPSTREAM_SECTION}」を\
                 **同じ行で**書いた箇所が無い"
            )
        });
    assert!(
        pointer.contains("持つ"),
        "指す形（「…が持つ」）で書くこと: {pointer}"
    );

    // 2. その見出しが ADR に実在すること。
    let adr = read_repo_file(UPSTREAM_ADR);
    assert!(
        adr.lines()
            .filter_map(heading)
            .any(|(_, text)| text == UPSTREAM_SECTION),
        "{UPSTREAM_ADR} に「{UPSTREAM_SECTION}」という見出しが無い（指し先が節ごと動いた）"
    );

    // 3. あちらの表の行が、こちらに写っていないこと。
    let section = section_body(&adr, UPSTREAM_SECTION)
        .unwrap_or_else(|| panic!("{UPSTREAM_ADR} の「{UPSTREAM_SECTION}」の節を取れない"));
    let rows = table_rows(&section);
    assert!(
        rows.len() >= 5,
        "{UPSTREAM_ADR} の3層の表が読めていない（{} 行）",
        rows.len()
    );

    // **見出しの行は突き合わせない。** 「満たせないとどうなるか」のような**列の名前**まで
    // 禁止語にすると、こちらの文書が「それぞれ満たせないとどうなるかまで、あちらの表にある」
    // と**指すことすらできなくなる**（実際にそれで赤くなった）。
    // 禁止したいのは**中身の行**である。
    let separator = rows
        .iter()
        .position(|row| {
            !row.is_empty()
                && row
                    .iter()
                    .all(|cell| !cell.is_empty() && cell.chars().all(|ch| ch == '-' || ch == ':'))
        })
        .unwrap_or_else(|| panic!("{UPSTREAM_ADR} の3層の表に区切り行が無い"));

    let mut compared = 0;
    for row in &rows[separator + 1..] {
        for cell in row {
            if cell.chars().count() < 10 {
                continue;
            }
            compared += 1;
            assert!(
                !doc.contains(cell.as_str()),
                "{SETUP_DOC} が ADR-0038 の3層の表を写している: {cell}"
            );
        }
    }
    assert!(
        compared >= 5,
        "突き合わせたセルが少なすぎる（{compared} 件）——走査が壊れていないか"
    );
}

/// §上流に、**自前の表を置かない**こと。
///
/// # なぜ要るか（**変異検査で見つけた穴** / F-3）
///
/// [`the_setup_doc_points_at_the_upstream_requirements_instead_of_copying_them`] は
/// **逐語の一致しか見ていない。** 実際に測ったら、
/// **3層を自分の言葉で言い換えた表を貼る変異が 1860 全緑を通った。**
///
/// **言い換えた散文までは止められない**（それは人が読むしかない）**が、
/// 「表の形で3層を再掲する」という一番出やすい形は、構造で止められる。**
/// この小節は**指すだけの場所**なので、**表が在ること自体が写しの徴候**である。
///
/// # ⚠ 守っていないもの
///
/// * **散文で言い換えた写しは止まらない**（上の変異の残り）。
/// * **この小節の外に貼れば止まらない。** 見ているのはこの小節だけ。
#[test]
fn the_upstream_subsection_points_instead_of_tabulating() {
    let doc = setup_doc();
    let body = body_of(&doc, UPSTREAM_SUBSECTION);
    let rows = table_rows(&as_refs(&body));

    assert!(
        rows.is_empty(),
        "§{UPSTREAM_SUBSECTION} に表が置かれている（ここは ADR-0038 §決定1 を指すだけの場所）: {rows:?}"
    );

    // **囮**: 走査が生きていること。表を持つ小節では、ちゃんと拾えている。
    let elsewhere = body_of(&doc, WALKED_SECTION);
    assert!(
        !table_rows(&as_refs(&elsewhere)).is_empty(),
        "表の走査が壊れている（§{WALKED_SECTION} の名乗りの表すら拾えていない）"
    );
}

/// 既定値は写さず、**在処を指す**こと。
///
/// # ⚠ 指し先は**全部**見る（`find` で最初の1つだけを見てはいけない）
///
/// **初版は `find` で書いていた**（F-3 のレビューが実測）——
/// **文書の後ろに `src/llm/mod.rs` の `DEFAULT_PORT` を見ること、と足しても 1861 全緑。**
/// **`DEFAULT_PORT` を宣言していない実在のファイルを読者に指したまま緑**になる。
/// **前に置くと 1 赤**なので、**並び順に寄りかかって片側だけ守っていた。**
///
/// **同じ穴を [`tests/readme.rs`] が B-30 のレビューで実測し、自分の doc で名指ししていた**
/// （`1827 全緑を通った` / `読者がたどると行き止まりになる`）。
/// **写す元は同じリポジトリにあり、しかも是正済みだった。写したのは是正前の形だった。**
///
/// [`tests/readme.rs`]: ../tests/readme.rs
#[test]
fn the_setup_doc_points_at_the_defaults_instead_of_copying_them() {
    let doc = setup_doc();
    let pointers = rust_const_pointers(&doc);

    for name in POINTED_CONSTANTS {
        let pointing: Vec<&(String, String)> = pointers
            .iter()
            .filter(|(_, pointed)| pointed == name)
            .collect();
        assert!(
            !pointing.is_empty(),
            "{SETUP_DOC} が `{name}` の在処を指していない"
        );

        let actual = match name {
            "DEFAULT_PORT" => DEFAULT_PORT.to_string(),
            "DEFAULT_ENDPOINT" => DEFAULT_ENDPOINT.to_owned(),
            other => panic!("帳簿にある `{other}` の突き合わせ相手が書かれていない"),
        };

        // **全部の指し先が、本当にその値を持っていること。**
        for (path, _) in &pointing {
            let source = read_repo_file(path);
            let value = declared_const_value(&source, name).unwrap_or_else(|| {
                panic!("{SETUP_DOC} が指す {path} に `pub const {name}` が無い")
            });
            assert_eq!(
                value, actual,
                "{SETUP_DOC} が指す {path} の `{name}` が、実体（{actual}）と違う"
            );
        }

        assert!(
            !doc.contains(&actual),
            "{SETUP_DOC} が `{name}` の値（{actual}）を写している（在処を指す形にすること）"
        );
    }
}

/// 名指しした環境変数を、サーバが実際に読むこと。
#[test]
fn the_setup_doc_only_names_environment_variables_the_server_reads() {
    let doc = setup_doc();
    let config = read_repo_file(CONFIG_SOURCE);
    let defined = defined_env_names(&config);
    assert!(
        defined.contains(ENV_LLM_ENDPOINT),
        "{CONFIG_SOURCE} から環境変数の綴りを読めていない（走査が壊れている）"
    );

    let named = personacore_env_names(&doc);
    assert!(
        !named.is_empty(),
        "{SETUP_DOC} が環境変数を1つも名指ししていない（走査が壊れていないか）"
    );
    for name in named {
        assert!(
            defined.contains(name),
            "{SETUP_DOC} がサーバの読まない環境変数 {name} を名指ししている"
        );
    }
}

/// 名乗る PowerShell の版が、起動スクリプトの要求と一致すること。
#[test]
fn the_setup_doc_names_only_the_powershell_version_the_script_requires() {
    let required = required_powershell_major(&read_repo_file(START_SCRIPT))
        .expect("start.ps1 が #Requires -Version を書いていること");
    let named = powershell_versions_named(&setup_doc());

    assert_eq!(
        named,
        BTreeSet::from([required]),
        "{SETUP_DOC} が名乗る PowerShell の版が、{START_SCRIPT} の要求（{required}）と違う"
    );
}

/// 名指しする宛先が、ループバックだけであること。
#[test]
fn the_setup_doc_names_no_address_other_than_the_loopback() {
    let named = ipv4_literals(&setup_doc());
    assert!(
        named.contains(&Ipv4Addr::LOCALHOST.to_string()),
        "{SETUP_DOC} がループバックを1度も書いていない（走査が壊れていないか）"
    );
    for addr in named {
        assert_eq!(
            addr,
            Ipv4Addr::LOCALHOST.to_string(),
            "{SETUP_DOC} がループバック以外の宛先 {addr} を名指ししている（外部へ出さない）"
        );
    }
}

/// §歩いた道が、**何の上で歩いたか**を名乗ること（ADR-0033 の作法）。
///
/// # ⚠ 守っていないもの
///
/// **ここは「欄が在って、中身が空でない」しか見ていない。**
/// **値そのものは [`the_walked_provenance_matches_what_the_repository_declares`] が、
/// 権威の在る2欄についてだけ**見る。**残り8欄の値は誰も見ていない。**
#[test]
fn the_walked_section_names_the_machine_it_was_walked_on() {
    let doc = setup_doc();

    for field in WALK_PROVENANCE {
        assert!(
            !provenance_value(&doc, field).is_empty(),
            "§{WALKED_SECTION} の「{field}」の欄が空である"
        );
    }
}

/// 名乗った版のうち、**リポジトリが宣言しているものは実体と一致する**こと。
///
/// # なぜ2欄だけなのか（**レビューの実測で分かった** / F-3）
///
/// 初版はこの検査を持たず、module doc に
/// 「**突き合わせる相手がリポジトリの中に無いからで、これは直せない種類の限界である**」と書いていた。
/// **10 欄のうち 8 欄には当たるが、2 欄には当たらない誤りだった:**
///
/// * **`pnpm`** —— [`WEB_PACKAGE_JSON`] の `packageManager`（**pnpm 自身が読む欄**）。
///   **名乗りを `1.0.0` に嘘にしても 1861 全緑だった。**
/// * **`PowerShell`** —— [`START_SCRIPT`] の `#Requires -Version`。
///   **既存の [`the_setup_doc_names_only_the_powershell_version_the_script_requires`] は
///   鍵が `"PowerShell "`（末尾に空白）なので、表の `| **PowerShell** | 7.6.5 |` には当たらない。**
///
/// # ⚠ 守っていないもの
///
/// * **`PowerShell` はメジャーしか見ていない。** `#Requires` が言うのは下限なので、
///   `7.6.5` を `7.0.0` に書き換えても鳴らない。**「7 系で歩いた」までが権威の言えること。**
/// * **`pnpm` は逐語で一致させている。** `packageManager` は実行される版を固定する欄なので、
///   **ここだけは下限ではなく同値**でよい。
/// * **残り8欄には権威が無い**（`rust-toolchain.toml` は channel だけで、版を書いていない）。
#[test]
fn the_walked_provenance_matches_what_the_repository_declares() {
    let doc = setup_doc();

    for field in WALK_PROVENANCE_DECLARED {
        assert!(
            WALK_PROVENANCE.contains(&field),
            "帳簿の食い違い: 「{field}」は WALK_PROVENANCE に無い"
        );
        let named = provenance_value(&doc, field);

        match field {
            "pnpm" => {
                let package_json = read_repo_file(WEB_PACKAGE_JSON);
                let declared = declared_pnpm_version(&package_json).unwrap_or_else(|| {
                    panic!("{WEB_PACKAGE_JSON} から `packageManager` の版を読めない")
                });
                assert_eq!(
                    named, declared,
                    "§{WALKED_SECTION} が名乗る pnpm（{named}）が、\
                     {WEB_PACKAGE_JSON} の宣言（{declared}）と違う"
                );
            }
            "PowerShell" => {
                let required = required_powershell_major(&read_repo_file(START_SCRIPT))
                    .expect("start.ps1 が #Requires -Version を書いていること");
                let walked = major_of(&named).unwrap_or_else(|| {
                    panic!("§{WALKED_SECTION} の PowerShell の欄（{named}）が版として読めない")
                });
                assert_eq!(
                    walked, required,
                    "§{WALKED_SECTION} が名乗る PowerShell（{named}）の系が、\
                     {START_SCRIPT} の要求（{required}）と違う"
                );
            }
            other => panic!("帳簿にある「{other}」の突き合わせ相手が書かれていない"),
        }
    }
}

/// §歩いた道が、**歩いた命令をその順で**書いていること。
///
/// **順番が主張の一部である** —— F-2 は順番を実体と逆に書いて、人の目で見つかった。
#[test]
fn the_walked_section_shows_the_steps_in_the_order_they_were_walked() {
    let doc = setup_doc();
    let body = body_of(&doc, WALKED_SECTION);
    let code = fenced_code_lines(&as_refs(&body)).join("\n");

    let mut previous = 0;
    let mut previous_step = "";
    for step in WALKED_STEPS {
        let at = code
            .find(step)
            .unwrap_or_else(|| panic!("§{WALKED_SECTION} のコードブロックに `{step}` が無い"));
        assert!(
            at >= previous,
            "§{WALKED_SECTION} で `{step}` が `{previous_step}` より前に来ている（歩いた順と違う）"
        );
        previous = at;
        previous_step = step;
    }
}

/// §歩いていない道に、**手順を書かない**こと。
///
/// # ⚠ ここが節の順番に寄りかかっている
///
/// [`section_body`] が「末尾まで読む」形に壊れると、
/// **後ろにある §歩いた道のコードブロックを拾って赤になる。**
/// **そのために、この2つの節はこの順で並べてある**（文書の §この文書の立て方 にも書いた）。
///
/// # ⚠ コードブロックだけでは足りなかった（**レビューが実測** / F-3）
///
/// **散文で「`cargo build` → `pnpm install` → `pnpm run build` →
/// `pwsh ./scripts/start.ps1` の順で通る。」と書き足して 1861 全緑だった。**
/// **いちばん出やすい形**（コードブロックを避けて、地の文で手順を書く）**が素通りしていた。**
///
/// **その4つは、どれも既に [`WALKED_STEPS`] に載っていた。**
/// だから**新しい床も帳簿も増やさずに**、
/// 「§歩いていない道 に [`WALKED_STEPS`] の綴りを1つも書かせない」を足して塞いだ。
///
/// # ⚠ 守っていないもの
///
/// * **[`WALKED_STEPS`] に無い綴りで手順を書けば、まだ素通りする**
///   （`cargo run` / `sudo apt install …` など）。**綴りの帳簿である以上、外側は残る。**
/// * **禁止しているのは §歩いていない道 の中だけ。** 他の節では自由に書ける。
#[test]
fn the_untrodden_section_writes_no_procedure() {
    let doc = setup_doc();

    let untrodden = body_of(&doc, UNTRODDEN_SECTION);
    let inside = fenced_code_lines(&as_refs(&untrodden));
    assert!(
        inside.is_empty(),
        "§{UNTRODDEN_SECTION} に手順（コードブロック）が書かれている: {inside:?}"
    );

    // **散文でも書かせない。** 歩いた命令の綴りが、この節に現れてはいけない。
    let prose = untrodden.join("\n");
    for step in WALKED_STEPS {
        assert!(
            !prose.contains(step),
            "§{UNTRODDEN_SECTION} に歩いた命令の綴り `{step}` が書かれている\
             （コードブロックを避けても、手順は手順である）"
        );
    }

    // **囮**: 歩いた道の側には手順が在ること。
    // 無ければ「手順が無い」は、走査が壊れているだけでも成立してしまう。
    let walked = body_of(&doc, WALKED_SECTION);
    assert!(
        !fenced_code_lines(&as_refs(&walked)).is_empty(),
        "§{WALKED_SECTION} に手順が1つも無い（走査が壊れていないか）"
    );
}

/// §歩いていない道が、§歩いた道より**前**に在ること。
///
/// # なぜ順番を見張るのか（**変異検査で決めた** / F-3）
///
/// [`the_untrodden_section_writes_no_procedure`] が
/// **「節の終わりで止まる」ことに寄りかかっている**（[`section_body`] の doc）。
/// **その寄りかかりは、2つの節がこの順に並んでいる間しか成立しない** ——
/// 逆順にすると、[`section_body`] を「末尾まで読む」形に壊しても
/// **実ファイルの側からは誰も鳴らなくなる**（植えたソースの対照だけが残る。**E-1 と同じ弱さ**）。
///
/// 文書の §この文書の立て方 が「節の順番には意味がある」と書いているので、
/// **doc が『守った』と書いた主張は、実際に当てられる形にしておく**（CLAUDE.md）。
///
/// # ⚠ 守っていないもの
///
/// **見ているのはこの2つの前後関係だけ。** 他の節がどこに在るかは見ていない。
#[test]
fn the_untrodden_section_comes_before_the_walked_section() {
    let doc = setup_doc();
    let at = |title: &str| {
        doc.lines()
            .position(|line| heading(line).is_some_and(|(_, text)| text == title))
            .unwrap_or_else(|| panic!("{SETUP_DOC} に「{title}」の節が無い"))
    };

    assert!(
        at(UNTRODDEN_SECTION) < at(WALKED_SECTION),
        "§{UNTRODDEN_SECTION} は §{WALKED_SECTION} より前に置くこと\
         （[`the_untrodden_section_writes_no_procedure`] がこの順番に寄りかかっている）"
    );
}

/// §要求に、**満たさないとどうなるか**が書いてあること。
#[test]
fn the_requirement_section_says_what_happens_when_it_is_not_met() {
    let doc = setup_doc();
    let body = body_of(&doc, REQUIREMENT_SECTION);
    let rows = table_rows(&as_refs(&body));

    let filled: Vec<&Vec<String>> = rows
        .iter()
        .filter(|row| row.len() >= 3)
        .filter(|row| !row[0].starts_with("---") && !plain(&row[0]).is_empty())
        .filter(|row| plain(&row[0]) != "要るもの")
        .collect();
    assert!(
        filled.len() >= 5,
        "§{REQUIREMENT_SECTION} の表が読めていない（{} 行）",
        filled.len()
    );
    for row in filled {
        assert!(
            !plain(&row[1]).is_empty(),
            "§{REQUIREMENT_SECTION} の「{}」に、満たさないとどうなるかが書かれていない",
            plain(&row[0])
        );
    }
}

/// 「見ていないこと」を、最後に並べてあること。
#[test]
fn the_setup_doc_lists_what_it_does_not_watch() {
    let doc = setup_doc();
    let body = body_of(&doc, UNWATCHED_SECTION);
    let listed = bullets(&as_refs(&body));

    assert!(
        listed.len() >= LEAST_UNWATCHED,
        "§{UNWATCHED_SECTION} が {} 件しか挙げていない（{LEAST_UNWATCHED} 件以上）",
        listed.len()
    );
}

/// 読み手がたどり着けること（根の README と docs の索引の両方から）。
#[test]
fn the_setup_doc_is_reachable_from_the_readme_and_the_index() {
    let from_readme = relative_links(&read_repo_file(README))
        .into_iter()
        .any(|(_, target)| target == SETUP_DOC);
    assert!(from_readme, "{README} が {SETUP_DOC} をリンクしていない");

    let from_index = relative_links(&read_repo_file(DOCS_INDEX))
        .into_iter()
        .any(|(_, target)| target == "09-setup.md");
    assert!(from_index, "{DOCS_INDEX} が {SETUP_DOC} をリンクしていない");
}
