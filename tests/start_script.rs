//! 起動スクリプト（`scripts/start.ps1`）が名指しする綴りを、サーバの実体と突き合わせる。
//!
//! # なぜ要るか（作業単位 F-1）
//!
//! 起動スクリプトは**サーバの外側にある写し**である。環境変数の綴り・既定のポート・
//! 上流のエンドポイント・DB の置き場所を書いており、**片方だけ変えても何も落ちない。**
//! 現れ方は「起動はするのに、繋がらない／古い DB が使われる」という**原因から遠い形**になる。
//!
//! # 何を守っていないか（**先に書く**）
//!
//! * **これはスクリプトを実行していない。** 綴りの突き合わせだけである。
//!   **書いてあるとおりに動くこと**は、このファイルは一切見ていない。
//!   **例外が1つある**: `tests/distribution.rs` は `scripts/package-zip.ps1` を `pwsh` で走らせる
//!   （作業単位 F-4d / 司令塔が採用）。**`package.ps1` と同じ4つの引数で呼び、テスト専用の入口を持たない**ので、
//!   F-1 の判断（テスト専用の出力経路を足さない）の趣旨には反しない。
//! * **⚠ 「使っているか」を見る節も、綴りの検査でしかない**（レビュー / 要修正1）。
//!   **宣言だけを見ていたので、宣言はそのままで使用箇所を差し替える変異が3本とも
//!   全緑を通った。** そこで [`DB_WIRING`] / [`PORT_WIRING`] と代入の回数を足したが、
//!   **言えるのは「その行がソースにある」までで、その行が効いていることは見ていない。**
//!   **走らせて確かめる形は、痛みが出たら別の作業単位にする**（司令塔の判断
//!   ——スクリプトにテスト専用の出力経路を足すと、CLAUDE.md
//!   「テスト専用の入口を作ると、本番の入口が観測対象から外れる」に当たるため）。
//! * **`127.0.0.1` 以外へ待ち受ける形を「型で表現できなくした」のは
//!   [`BindAddr`] であって、このファイルではない**（`BindAddr::loopback` が唯一の
//!   コンストラクタで、IP を受け取らない）。ここが見ているのは
//!   **スクリプトが利用者に見せる／叩きに行く宛先の綴り**だけ。
//! * **`[System.Net.IPAddress]::Any` のような「数字で書かれていない宛先」は拾えない。**
//!   走査しているのは IPv4 の**リテラル**である。
//!
//! # 走査の作法
//!
//! **走査の本体はすべてソースを引数で受け取る。** 実ファイルと、テストが植えたソースを
//! **同じ本体に通す**ため（CLAUDE.md `15b`「対照は自己成就しうる」）。
//! 実ファイルだけを見る対照は、走査の綴りを壊す変異が
//! **壊した綴りを走査対象に書き込むだけで緑になる。**

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use personacore::llm::DEFAULT_ENDPOINT;
use personacore::server::{
    AppConfig, BindAddr, DATABASE_FILE_NAME, DEFAULT_PORT, ENV_DATABASE_PATH, ENV_LLM_ENDPOINT,
    ENV_MAIN_MODEL, ENV_PORT, ENV_SUB_LLM_ENDPOINT, ENV_SUB_MODEL, ENV_UPSTREAMS,
};

/// 起動スクリプト（リポジトリのルートからの位置）。
const START_SCRIPT: &str = "scripts/start.ps1";

/// 配布物を作るスクリプト（ADR-0041 §決定5 / 作業単位 F-4a）。
const PACKAGE_SCRIPT: &str = "scripts/package.ps1";

/// `package.ps1` が画面をビルドする呼び出し（**cargo より先に在ること**）。
const PACKAGE_WEB_BUILD: &str = "pnpm run build";

/// `package.ps1` が実行ファイルを作る呼び出し（**画面より後に在ること**）。
///
/// **`--release` まで綴りに含める**——debug を配ると、画面は入っていても別物になる。
const PACKAGE_CARGO_BUILD: &str = "cargo build --release";

/// `package.ps1` が複製元を宣言している鍵。
const PACKAGE_BIN_KEY: &str = "$ReleaseBinRelative = ";

/// 複製元が在るべき場所（**`target/release` の下**）。
const PACKAGE_BIN_PREFIX: &str = "target/release/";

/// 配布用の zip を組むスクリプト（作業単位 F-4d / Issue origin#25）。**`package.ps1` の最後の段。**
const ZIP_SCRIPT: &str = "scripts/package-zip.ps1";

/// `package.ps1` が zip を組むスクリプトの置き場所を宣言している鍵。
const PACKAGE_ZIP_SCRIPT_KEY: &str = "$ZipScriptRelative = ";

/// `package.ps1` が利用者向け README の置き場所を宣言している鍵。
const PACKAGE_DIST_README_KEY: &str = "$DistReadmeRelative = ";

/// 利用者向け README（zip の中では `README.md`）。
const DIST_README: &str = "scripts/README-dist.md";

/// 第三者の著作権表示を作り直すスクリプト（作業単位 O-1 / Issue origin#48）。
const NOTICES_SCRIPT: &str = "scripts/notices.ps1";

/// `package.ps1` が表示を作るスクリプトの置き場所を宣言している鍵。
const PACKAGE_NOTICES_SCRIPT_KEY: &str = "$NoticesScriptRelative = ";

/// `package.ps1` が表示（zip の中では `THIRD-PARTY-NOTICES.txt`）の置き場所を宣言している鍵。
const PACKAGE_DIST_NOTICES_KEY: &str = "$DistNoticesRelative = ";

/// 第三者の著作権表示（`scripts/notices.ps1` が書き、コミットしてある）。
const DIST_NOTICES: &str = "scripts/THIRD-PARTY-NOTICES.txt";

/// `notices.ps1` が書き出す先を宣言している鍵（**`package.ps1` が詰めるものと同じ**であること）。
const NOTICES_OUT_KEY: &str = "$NoticesRelative = ";

/// **宣言した4つの置き場所から**組み立てている式（宣言を死んだ変数にしない）。
const PACKAGE_ZIP_WIRING: [&str; 4] = [
    "$zipScript = Join-Path $RepoRoot $ZipScriptRelative",
    "$distReadme = Join-Path $RepoRoot $DistReadmeRelative",
    "$noticesScript = Join-Path $RepoRoot $NoticesScriptRelative",
    "$distNotices = Join-Path $RepoRoot $DistNoticesRelative",
];

/// `package.ps1` が表示を作り直す呼び出し（**zip より先**。依存が増減しても古びた表示を詰めない）。
const PACKAGE_NOTICES_CALL: &str = "& $noticesScript";

/// `package.ps1` が cargo-about の有無を確かめる呼び出し（**画面とリリースビルドより先**）。
const PACKAGE_ABOUT_CHECK: &str = "cargo about --version";

/// `package.ps1` が zip を組む呼び出し。**渡す exe は複製した後のもの（`$outBin`）。**
/// ライセンスの文書はリポジトリの根から詰める（`-LicenseDir $RepoRoot` / O-2 / Issue origin#49）。
const PACKAGE_ZIP_CALL: &str = "& $zipScript -ExePath $outBin -ReadmePath $distReadme -NoticesPath $distNotices -LicenseDir $RepoRoot -OutDir $resolvedOut";

/// `package.ps1` が実行ファイルを配布物の置き場所へ複製する呼び出し。
const PACKAGE_COPY: &str = "Copy-Item -LiteralPath $builtBin -Destination $outBin";

/// **冒頭に `param(…)` を持つスクリプト**（引数の名前を局所変数に使い回さない検査の対象）。
///
/// **⚠ ここへ足すのを忘れても、何も鳴らない。**
/// `param(…)` を持つスクリプトを増やしたら、必ず足すこと。
const PARAM_SCRIPTS: [&str; 4] = [START_SCRIPT, PACKAGE_SCRIPT, ZIP_SCRIPT, NOTICES_SCRIPT];

/// キャラクターの投入スクリプト（既定の宛先がサーバの既定ポートの写しになっている）。
const SEED_SCRIPT: &str = "scripts/seed-characters.ps1";

/// 投入スクリプトが既定の宛先を書いている変数。
const SEED_BASE_URL_KEY: &str = "$BaseUrl = ";

/// 環境変数の綴りを定義しているファイル。
const CONFIG_SOURCE: &str = "src/server/config.rs";

/// `pub const ENV_…: &str = "PERSONACORE_…";` の頭。
const ENV_CONST_PREFIX: &str = "pub const ENV_";

/// 環境変数の綴りの接頭辞。
const ENV_NAME_PREFIX: &str = "PERSONACORE_";

/// スクリプトが「どのファイルから既定のポートを読むか」を書いている変数。
const PORT_SOURCE_KEY: &str = "$DefaultPortSource = ";

/// スクリプトが「どの綴りの直後を読むか」を書いている変数（ポート）。
const PORT_PREFIX_KEY: &str = "$DefaultPortPrefix = ";

/// 上流のエンドポイントについての同じ2つ。
const ENDPOINT_SOURCE_KEY: &str = "$DefaultEndpointSource = ";
const ENDPOINT_PREFIX_KEY: &str = "$DefaultEndpointPrefix = ";

/// スクリプトが既定で使う DB のパス（リポジトリのルートからの位置）。
const DB_RELATIVE_KEY: &str = "$DefaultDbRelative = ";

// ----------------------------------------------------------------------
// **宣言だけでなく「使っているか」を見る綴り**（レビュー指摘 / 要修正1）。
//
// ⚠ **これは依然として綴りの検査であり、走らせて確かめてはいない。**
// 言えるのは「その行がソースにある」までで、**その行が効いていること**は見ていない。
// 実行して確かめる形は、痛みが出たら別の作業単位にする（司令塔の判断）。
//
// # なぜ足したか
//
// 宣言だけを見ていたので、**宣言はそのままで使用箇所だけを差し替える変異が3本とも
// 全緑を通った** —— 既定 DB の組み立て／既定ポートの読み出し／選んだポートの受け渡し。
// **いちばん重いのは1つめ**で、`$DefaultDbRelative` が死んだ変数になり、
// **DB がリポジトリ直下の古い DB へ落ちたまま起動が成功する。**
// ----------------------------------------------------------------------

/// 既定の DB を**宣言した変数から**組み立てている式。
const DB_WIRING: &str = "Join-Path $RepoRoot $DefaultDbRelative";

/// 既定のポートを**読んで**決めている式（写しに差し替えるとこれが消える）。
const PORT_WIRING: &str =
    "$defaultPort = [int](Read-RustConst -File $DefaultPortSource -Prefix $DefaultPortPrefix)";

/// 既定のポートへの代入。**1回きりであること**を数える
/// （後ろに書き足して上書きする形を落とす）。
const PORT_ASSIGNMENT: &str = "$defaultPort = ";

// ----------------------------------------------------------------------
// 走査の本体。**すべてソースを引数で受け取る。**
// ----------------------------------------------------------------------

/// PowerShell の**単一引用符の文字列**を、鍵の直後から1つ取り出す。
///
/// 単一引用符しか読まない（`'…'` は PowerShell では変数展開が起きない）。
/// **二重引用符を読まないのは意図的**——展開が起きる綴りを「値」として扱うと、
/// 静的に突き合わせたものが実行時の値と違いうる。
fn ps_single_quoted<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    let at = source.find(key)?;
    let rest = source[at + key.len()..].trim_start();
    let inside = rest.strip_prefix('\'')?;
    let end = inside.find('\'')?;
    Some(&inside[..end])
}

/// Rust のソースから `<接頭辞><値>;` の値を取り出す（前後の空白と `"` は落とす）。
///
/// `pub const DEFAULT_PORT: u16 = 8080;` → `8080` /
/// `pub const DEFAULT_ENDPOINT: &str = "http://…";` → `http://…`
fn rust_const_after<'a>(source: &'a str, prefix: &str) -> Option<&'a str> {
    let at = source.find(prefix)?;
    let rest = &source[at + prefix.len()..];
    let end = rest.find(';')?;
    Some(rest[..end].trim().trim_matches('"'))
}

/// ソースに現れる `PERSONACORE_…` の綴りを全部拾う。
///
/// 大文字・数字・下線が続く限り伸ばす。**後ろへ伸ばすのが要点**——
/// `PERSONACORE_DB_PATHX` を `PERSONACORE_DB_PATH` として通してしまうと、
/// 綴り間違いを見逃す。
///
/// **⚠ 前の境界は見ていない。** 見る枝を置いても、
/// `XPERSONACORE_PORT` は「実在しない綴りとして落ちる」ではなく
/// 「1件も拾わない」になるだけで、判定は変わらない（＝到達しない検査になる）。
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

/// ソースの中に綴りが何回現れるか。
///
/// **「宣言されているか」ではなく「使われているか」を数えるための本体。**
fn occurrences(source: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    source.match_indices(needle).count()
}

/// URL の綴りからポート番号を取り出す（書いていなければ `None`）。
///
/// **`http://…:8080/` の末尾の `/` を落として読む。**
/// ポートを省いた URL（`http://example.test`）は `None` —— そこは
/// **既定ポートを名乗っていない**ので、突き合わせる相手がいない。
fn url_port(url: &str) -> Option<u16> {
    let tail = url.rsplit(':').next()?;
    tail.trim_end_matches('/').parse().ok()
}

/// ソースに現れる IPv4 の**リテラル**を全部拾う。
///
/// 数字と `.` の並びを切り出し、`Ipv4Addr` として読めたものだけを採る
/// （`1.97.1` のような3つ組は落ちる）。
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

// ----------------------------------------------------------------------
// 実ファイルの読み込み
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

fn start_script() -> String {
    read_repo_file(START_SCRIPT)
}

// ----------------------------------------------------------------------
// 走査本体の対照（**植えたソースを、実ファイルと同じ本体に通す**）
// ----------------------------------------------------------------------

/// `occurrences` は、渡されたソースを数えること。
#[test]
fn the_occurrence_scan_counts_the_source_it_is_given() {
    assert_eq!(
        occurrences("$defaultPort = 1\n$defaultPort = 2\n", PORT_ASSIGNMENT),
        2,
        "植えた重複を数えること（上書きする形を落とすのはこの数）"
    );
    assert_eq!(
        occurrences("$defaultPort = 1\n", PORT_ASSIGNMENT),
        1,
        "1回なら 1"
    );
    assert_eq!(
        occurrences("$otherPort = 1\n", PORT_ASSIGNMENT),
        0,
        "別の綴りは数えないこと"
    );
    assert_eq!(
        occurrences("何でも", ""),
        0,
        "空の綴りを「全部一致」と読まないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert_eq!(
        occurrences(&start_script(), PORT_ASSIGNMENT),
        1,
        "実ファイルでも数えられること"
    );
}

/// `url_port` は、渡された URL のポートを読むこと。
#[test]
fn the_url_port_scan_reads_the_value_it_is_given() {
    assert_eq!(
        url_port("http://127.0.0.1:4242"),
        Some(4242),
        "植えたポートを拾うこと"
    );
    assert_eq!(
        url_port("http://127.0.0.1:4242/"),
        Some(4242),
        "末尾の `/` を落として読むこと"
    );
    assert_eq!(
        url_port("http://example.test"),
        None,
        "ポートを書いていない URL は「名乗っていない」と言うこと"
    );
    assert_eq!(url_port(""), None, "空の綴りは None");

    // **実ファイルも同じ本体を通ること。**
    let seed = read_repo_file(SEED_SCRIPT);
    let base = ps_single_quoted(&seed, SEED_BASE_URL_KEY).expect("既定の宛先を拾えること");
    assert!(url_port(base).is_some(), "実ファイルからも拾えること");
}

/// `ps_single_quoted` は、渡された値をそのまま読むこと。
#[test]
fn the_powershell_scan_reads_the_value_it_is_given() {
    assert_eq!(
        ps_single_quoted("$DefaultPortSource = 'planted/path.rs'\n", PORT_SOURCE_KEY),
        Some("planted/path.rs"),
        "植えた値をそのまま拾うこと"
    );
    assert_eq!(
        ps_single_quoted(
            "# 注記\n$DefaultPortSource   =    'spaced'\n",
            PORT_SOURCE_KEY
        ),
        None,
        "鍵の綴り（`= ` まで）が一致しないソースは拾わないこと"
    );
    assert_eq!(
        ps_single_quoted("$DefaultPortSource = \"expanded\"\n", PORT_SOURCE_KEY),
        None,
        "二重引用符は値として読まないこと（変数展開が起きうる）"
    );
    assert_eq!(
        ps_single_quoted("$DefaultPortSource = 'unclosed\n", PORT_SOURCE_KEY),
        None,
        "閉じていない引用符は「見つからない」と言うこと"
    );
    assert_eq!(
        ps_single_quoted("何も無い", PORT_SOURCE_KEY),
        None,
        "鍵が無いソースを、黙って何かに解釈しないこと"
    );

    // **実ファイルも同じ本体を通ること**（対照だけが通る形にしない）。
    assert!(
        ps_single_quoted(&start_script(), PORT_SOURCE_KEY).is_some(),
        "実ファイルからも拾えること"
    );
}

/// `rust_const_after` は、渡された接頭辞の直後を読むこと。
#[test]
fn the_rust_const_scan_reads_the_value_it_is_given() {
    assert_eq!(
        rust_const_after("pub const X: u16 = 4242;\n", "pub const X: u16 = "),
        Some("4242"),
        "数はそのまま拾うこと"
    );
    assert_eq!(
        rust_const_after(
            "pub const X: &str = \"http://planted:1\";\n",
            "pub const X: &str = "
        ),
        Some("http://planted:1"),
        "文字列は引用符を落として拾うこと"
    );
    assert_eq!(
        rust_const_after("const X: u16 = 4242;\n", "pub const X: u16 = "),
        None,
        "接頭辞が一致しないソースは拾わないこと"
    );
    assert_eq!(
        rust_const_after("pub const X: u16 = 4242\n", "pub const X: u16 = "),
        None,
        "`;` で終わっていない綴りは「見つからない」と言うこと"
    );

    // **実ファイルも同じ本体を通ること。**
    let script = start_script();
    let prefix = ps_single_quoted(&script, PORT_PREFIX_KEY).expect("接頭辞を拾えること");
    assert!(
        rust_const_after(&read_repo_file(CONFIG_SOURCE), prefix).is_some(),
        "実ファイルからも拾えること"
    );
}

/// `personacore_env_names` は、渡されたソースの綴りを拾うこと。
#[test]
fn the_env_name_scan_reads_the_source_it_is_given() {
    assert_eq!(
        personacore_env_names("$env:PERSONACORE_PORT = '1'\n"),
        BTreeSet::from(["PERSONACORE_PORT"]),
        "植えた綴りを拾うこと"
    );
    assert_eq!(
        personacore_env_names("PERSONACORE_DB_PATHX"),
        BTreeSet::from(["PERSONACORE_DB_PATHX"]),
        "後ろへ伸ばすこと（綴り間違いを既知の綴りに丸めない）"
    );
    assert_eq!(
        personacore_env_names("PERSONACORE_PORT と PERSONACORE_DB_PATH"),
        BTreeSet::from(["PERSONACORE_PORT", "PERSONACORE_DB_PATH"]),
        "複数を拾うこと"
    );
    assert!(
        personacore_env_names("PERSONACORE_PORT=8080").contains("PERSONACORE_PORT"),
        "`=` で切れること"
    );
    assert!(
        personacore_env_names("何も無い").is_empty(),
        "無いソースからは1件も拾わないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        !personacore_env_names(&start_script()).is_empty(),
        "実ファイルからも拾えること"
    );
}

/// `defined_env_names` は、渡されたソースの定義を拾うこと。
#[test]
fn the_defined_env_scan_reads_the_source_it_is_given() {
    assert_eq!(
        defined_env_names("pub const ENV_PLANTED: &str = \"PERSONACORE_PLANTED\";\n"),
        BTreeSet::from(["PERSONACORE_PLANTED"]),
        "植えた定義を拾うこと"
    );
    assert!(
        defined_env_names("/// `PERSONACORE_PLANTED` の話\n").is_empty(),
        "doc の中の言及を「定義」と読まないこと"
    );
    assert!(
        defined_env_names("const ENV_PLANTED: &str = \"PERSONACORE_PLANTED\";\n").is_empty(),
        "非公開の定義は拾わないこと（接頭辞が一致しない）"
    );

    // **実ファイルも同じ本体を通ること。** ここが「1件も無いから緑」を潰す。
    let config = read_repo_file(CONFIG_SOURCE);
    let defined = defined_env_names(&config);
    assert!(
        defined.contains(ENV_DATABASE_PATH),
        "実ファイルから {ENV_DATABASE_PATH} を拾えること（走査が空振りしていない）"
    );
}

/// `ipv4_literals` は、渡されたソースのリテラルを拾うこと。
#[test]
fn the_address_scan_reads_the_source_it_is_given() {
    assert_eq!(
        ipv4_literals("http://127.0.0.1:8080/"),
        BTreeSet::from(["127.0.0.1".to_owned()]),
        "植えた宛先を拾うこと"
    );
    assert_eq!(
        ipv4_literals("-TargetHost '0.0.0.0'"),
        BTreeSet::from(["0.0.0.0".to_owned()]),
        "**外へ出す綴りを拾うこと**（これを拾えなければ、この検査は何もしていない）"
    );
    assert_eq!(
        ipv4_literals("cargo 1.97.1 / ポート 8080"),
        BTreeSet::new(),
        "IPv4 として読めない数の並びは拾わないこと"
    );
    assert_eq!(
        ipv4_literals("末尾が句点の 127.0.0.1. と 10.0.0.1"),
        BTreeSet::from(["127.0.0.1".to_owned(), "10.0.0.1".to_owned()]),
        "末尾の `.` を落として読むこと／複数を拾うこと"
    );

    // **実ファイルも同じ本体を通ること。**
    assert!(
        !ipv4_literals(&start_script()).is_empty(),
        "実ファイルからも拾えること"
    );
}

// ----------------------------------------------------------------------
// 本命——スクリプトとサーバの突き合わせ
// ----------------------------------------------------------------------

/// **スクリプトが名指しする環境変数は、全部サーバが実際に読むものであること。**
///
/// 綴りを1文字間違えた環境変数は**黙って無視される**——起動は通り、
/// 現れ方は「指定したはずの設定が効いていない」になる。
#[test]
fn the_start_script_only_names_environment_variables_the_server_reads() {
    let config = read_repo_file(CONFIG_SOURCE);
    let defined = defined_env_names(&config);
    let script = start_script();
    let named = personacore_env_names(&script);

    assert!(
        !named.is_empty(),
        "スクリプトが環境変数を1つも名指ししていない（走査が空振りしている疑い）"
    );

    for name in &named {
        assert!(
            defined.contains(name),
            "起動スクリプトが `{name}` を名指ししているが、\
             {CONFIG_SOURCE} にその綴りの定義が無い（サーバは黙って無視する）"
        );
    }
}

/// **スクリプトの DB は、リポジトリのルート直下でもなく、サーバの既定でもないこと。**
///
/// # ⚠ 主張は2つある。別々に見張る（レビュー指摘6 / 作業単位 F-4a）
///
/// | 主張 | 見張る形 | 破れると |
/// |---|---|---|
/// | **リポジトリのルート直下の綴りを指さない** | [`DATABASE_FILE_NAME`] と `assert_ne!` | **そこには 0004 までの古い DB が置いてある**（docs/06-roadmap.md の F 箱）。続きのマイグレーションがそれに当たる |
/// | **サーバの既定と別の場所** | 既定値だけで組んだ設定の `database_path()` と `assert_ne!` | **開発の DB と遊びの DB が同じになる**（ADR-0041 §決定3 が分けたもの） |
///
/// **⚠ 2つ目だけでは、1つ目が死んだ検査になる**（レビュアーが実測 / 2026-09-18）。
/// **ADR-0041 §決定3 でサーバの既定が `%LOCALAPPDATA%` の下へ移った**ので、
/// **左辺は必ずリポジトリの下・右辺は必ずその外**になり、**`assert_ne!` が反証不能**になった。
/// 実測: **`$DefaultDbRelative` を `'personacore.sqlite'` にする変異が、
/// F-4a の差分では緑、`main`（`b1ba6c1`）では 1 赤。**
/// **`DATABASE_FILE_NAME` との比較を復活させて、その変異を赤に戻してある。**
///
/// # ⚠ **主張2 を単独で見張るものは無い**（レビュー指摘5 / 是正2周目）
///
/// **上の ⚠ は、主張1 が死んでいたことを言っている。主張2 は**いまも**死んでいる。**
/// **両辺が構造的に別の根の下に在る**ので、
/// **スクリプト側の1トークンをどう変えても、この `assert_ne!` は赤にならない**
/// （相対パスをどう書き換えても、`repo_path` はリポジトリの下を指す）。
///
/// **それでも置いてあるのは、意図の記録としてである** ——
/// **「サーバの既定と別の場所」は F-1 が決めたことで、
/// ADR-0041 §決定3 が既定を動かしたあとも生きている。**
/// **⚠ 「2つとも見張っている」と読まないこと。**
/// **赤にできるのは主張1 だけ**（実測: 是正2周目の `r6`）。
///
/// # ⚠ 名前が変わった（`…_outside_the_repository_default` の後継）
///
/// 元の名前は「リポジトリの既定の外」で、**2つの主張が1語に潰れていた。**
#[test]
fn the_start_script_names_a_database_of_its_own() {
    let script = start_script();

    let assignment = format!("$env:{ENV_DATABASE_PATH} =");
    assert!(
        script.contains(&assignment),
        "起動スクリプトが `{assignment}` を書いていない\
         （既定のままだと遊びの DB と同じものを触る）"
    );

    let relative = ps_single_quoted(&script, DB_RELATIVE_KEY)
        .expect("起動スクリプトから既定の DB のパスを拾えること");

    // --- 主張1: リポジトリのルート直下の綴りを指さない（そこには古い DB がある）。 ---
    assert_ne!(
        Path::new(relative),
        Path::new(DATABASE_FILE_NAME),
        "起動スクリプトの DB が、リポジトリのルート直下の古い DB を指している"
    );

    // --- 主張2: サーバの既定とは別の場所。 ---
    let server_default = AppConfig::from_lookup(|_| None)
        .expect("既定値だけで設定を組み立てられること")
        .database_path()
        .to_path_buf();
    assert_ne!(
        repo_path(relative),
        server_default,
        "起動スクリプトの既定の DB が、サーバの既定と同じ場所を指している"
    );
    // **前提**——サーバの既定は絶対パス（OS の作法の場所の下）であること。
    // **この前提が成り立つ限り、主張2 だけでは主張1 を見張れない**（上の ⚠）。
    assert!(
        server_default.is_absolute(),
        "前提: サーバの既定は OS の作法の場所の下（絶対パス）であること: {}",
        server_default.display()
    );

    // **⚠ 宣言だけでは足りない。** 宣言をそのままにして組み立て側だけを
    // 直値へ差し替える変異が、フルスイート 1754 全緑を通った（レビュー / 要修正1）。
    // **`$DefaultDbRelative` が死んだ変数になり、DB は古い方へ落ちる。**
    assert!(
        script.contains(DB_WIRING),
        "起動スクリプトが `{DB_WIRING}` を書いていない\
         （宣言した既定を使わずに組み立てると、宣言だけが正しくて DB は古い方へ落ちる）"
    );
    assert_eq!(
        occurrences(&script, &assignment),
        1,
        "`{assignment}` が1回きりでない（後ろで上書きすると、手前の決定が効かない）"
    );
}

/// **既定のポートは写さずに読むこと**（`web/vite.config.ts` と同じ作法）。
///
/// スクリプトはブラウザに見せる URL を組み立てるためにポートを知る必要がある。
/// **写すと、Rust 側で既定を変えたときにスクリプトだけが黙って古びる**
/// ——現れ方は「起動はしたのに、開いた画面が繋がらない」。
#[test]
fn the_start_script_reads_the_default_port_from_the_rust_source() {
    let script = start_script();
    let source_path = ps_single_quoted(&script, PORT_SOURCE_KEY)
        .expect("起動スクリプトから既定ポートの読み出し元を拾えること");
    let prefix = ps_single_quoted(&script, PORT_PREFIX_KEY)
        .expect("起動スクリプトから既定ポートの接頭辞を拾えること");

    let source = read_repo_file(source_path);
    let read = rust_const_after(&source, prefix)
        .unwrap_or_else(|| panic!("{source_path} から `{prefix}` の直後を読めること"));

    assert_eq!(
        read.parse::<u16>().ok(),
        Some(DEFAULT_PORT),
        "起動スクリプトの読み方では {source_path} から {DEFAULT_PORT} を読めない"
    );

    // **写していないこと。** 読む形と写す形は、読める限り見分けが付かない。
    assert!(
        !script.contains(&DEFAULT_PORT.to_string()),
        "起動スクリプトが既定ポート {DEFAULT_PORT} を写している（読む形にすること）"
    );

    // **⚠ 上の1本だけでは「いまの正しい値を写した場合」しか捕まらない。**
    // `$defaultPort = 9999` に差し替える変異は、10 本すべて緑を通った（レビュー / 要修正1）。
    // **読み出し元と接頭辞が死んだ変数になり、宣言だけが正しく残る。**
    assert!(
        script.contains(PORT_WIRING),
        "起動スクリプトが `{PORT_WIRING}` を書いていない\
         （読まずに決めると、上の2つの宣言は使われないまま正しく見える）"
    );
    assert_eq!(
        occurrences(&script, PORT_ASSIGNMENT),
        1,
        "`{PORT_ASSIGNMENT}` が1回きりでない（後ろで上書きすると、読んだ値が効かない）"
    );
}

/// **`-Port` で選んだポートを、サーバへ実際に渡すこと。**
///
/// # なぜ要るか（レビュー / 要修正1）
///
/// `$env:PERSONACORE_PORT = "$Port"` の1行を消す変異は **10 本すべて緑**だった。
/// **消えても、スクリプトはそのポートの空きを確かめ、その URL を表示し、
/// ブラウザをそのポートで開く。サーバだけが既定のポートで待つ。**
/// **起動ログを読まない限り分からない。**
///
/// 綴りは [`ENV_PORT`] から作る——`config.rs` 側で名前が変わってもここが鳴る。
#[test]
fn the_start_script_passes_the_chosen_port_to_the_server() {
    let script = start_script();
    let wiring = format!("$env:{ENV_PORT} = \"$Port\"");

    assert!(
        script.contains(&wiring),
        "起動スクリプトが `{wiring}` を書いていない\
         （-Port が画面の案内にだけ効いて、サーバへ届かない）"
    );
}

/// **既定の上流のエンドポイントも写さずに読むこと。**
///
/// 疎通確認の宛先である。写すと、上流の既定を変えたときに
/// **「上流に繋がりません」と嘘をつくスクリプト**が残る。
#[test]
fn the_start_script_reads_the_default_upstream_endpoint_from_the_rust_source() {
    let script = start_script();
    let source_path = ps_single_quoted(&script, ENDPOINT_SOURCE_KEY)
        .expect("起動スクリプトから上流の読み出し元を拾えること");
    let prefix = ps_single_quoted(&script, ENDPOINT_PREFIX_KEY)
        .expect("起動スクリプトから上流の接頭辞を拾えること");

    let source = read_repo_file(source_path);
    let read = rust_const_after(&source, prefix)
        .unwrap_or_else(|| panic!("{source_path} から `{prefix}` の直後を読めること"));

    assert_eq!(
        read, DEFAULT_ENDPOINT,
        "起動スクリプトの読み方では {source_path} から既定のエンドポイントを読めない"
    );

    assert!(
        !script.contains(DEFAULT_ENDPOINT),
        "起動スクリプトが既定のエンドポイントを写している（読む形にすること）"
    );
}

/// **スクリプトが名指しする宛先は、ループバックだけであること。**
///
/// # ⚠ これは「外部公開を防いでいる」ではない
///
/// 待ち受け先を緩める手段は**そもそも無い**——[`BindAddr::loopback`] が唯一の
/// コンストラクタで、IP を受け取らない。**守っているのは型のほう。**
///
/// ここが見ているのは**スクリプトが利用者に見せる URL と、叩きに行く宛先の綴り**で、
/// 「利用者を別の宛先へ案内する」形を落とす。
/// **数字で書かれていない宛先（`::Any` / ホスト名）は拾えない。**
#[test]
fn the_start_script_names_no_address_other_than_the_loopback() {
    let named = ipv4_literals(&start_script());
    let loopback = BindAddr::loopback(DEFAULT_PORT)
        .socket_addr()
        .ip()
        .to_string();

    assert_eq!(
        named,
        BTreeSet::from([loopback.clone()]),
        "起動スクリプトが {loopback} 以外の宛先を名指ししている"
    );
}

/// **投入スクリプトの既定の宛先が、サーバの既定ポートと食い違ったら鳴る。**
///
/// # なぜここで見るのか（レビュー / 要修正6）
///
/// `scripts/seed-characters.ps1` の `$BaseUrl` の既定は **[`DEFAULT_PORT`] の写し**で、
/// **いままで何も見ていなかった**（`tests/server_admin.rs` の走査は API の欄名しか見ない）。
/// 片方を変えると、投入スクリプトだけが黙って何も居ない先を叩く。
///
/// **⚠ そして `scripts/README-start.md` がこの踏み方を新しく作った** ——
/// 「別のポートで: `-Port 9000`」と「まとめて投入したいなら `seed-characters.ps1`」が
/// 同じ文書に並んでいるので、**9000 で立てて 8080 を叩く導線が読者の目の前にある。**
///
/// # ⚠ 守っていないもの
///
/// * **`-BaseUrl` を渡し忘れた利用者は、これでは助からない。**
///   見ているのは**2つの既定が揃っていること**だけで、上の導線は塞がらない。
/// * **`web/README.md` の写しは、ここでは見ていない。**
///   **⚠ 「誰も見ていない」ではなくなった**（作業単位 B-30 / 2026-09-02）——
///   **写しそのものが消え**（`http://127.0.0.1:8080/ で画面が出る` →
///   `起動ログが出す URL で画面が出る`）、
///   **`tests/readme.rs` の
///   `the_documents_the_readme_links_to_point_at_the_default_port_instead_of_copying_it`
///   が「写していないこと」を見張っている。**
///   置き場所があちらなのは、**根の README がその文書を3回リンクしている**からで、
///   読者から見れば案内の続きだからである。
///   **`5173`（Vite の dev server）は、いまも誰も見ていない** ——
///   **権威がリポジトリのどこにも無い**（Vite の組み込みの既定で、
///   `web/vite.config.ts` は `server.port` を設定していない）。
///   そこまで走査を広げると、**誰も宣言していない値をテストが写す**ことになる
///   ——直そうとしている病気そのもの。**B-30 も広げないと判断した。**
#[test]
fn the_seed_script_talks_to_the_port_the_server_listens_on() {
    let seed = read_repo_file(SEED_SCRIPT);
    let base = ps_single_quoted(&seed, SEED_BASE_URL_KEY)
        .expect("投入スクリプトから既定の宛先を拾えること");

    assert_eq!(
        url_port(base),
        Some(DEFAULT_PORT),
        "{SEED_SCRIPT} の既定の宛先（{base}）が、\
         サーバの既定ポート {DEFAULT_PORT} と食い違っている"
    );
}

// ======================================================================
// 上流プロファイル（作業単位 F-9）
//
// `start.ps1 -Upstream <名前>` が `scripts/upstreams.json`（無ければ見本）を読み、
// **既存の環境変数4つ**へ流す。gguf が書いてある面だけ `llama serve` を起こす。
//
// # ⚠ ここが見ていないもの（**先に書く**）
//
// * **スクリプトは1行も実行していない。** 綴りと見本の突き合わせだけである。
//   **`-Upstream <無い名前>` で在る名前を並べて止まること／`llama serve` が実際に起きること・
//   止まることは、司令塔が実機で確かめる**（このファイルは `pwsh` を起動しない。
//   F-1 の判断——スクリプトにテスト専用の出力経路を足さない——をそのまま継いでいる）。
// * **`llama serve` を起こす振る舞いは、テストからは確かめない。** GPU を掴むので、
//   この機の測定と取り合う（CLAUDE.md「先客が居たら始めない」）。
// * **見本（`upstreams.sample.json`）しか見ていない。** 利用者の写し
//   （`scripts/upstreams.json`）は `.gitignore` 済みで、ここからは読めない。
// ======================================================================

use serde_json::Value as JsonValue;

/// 上流プロファイルの**見本**（コミットする側）。
const UPSTREAMS_SAMPLE: &str = "scripts/upstreams.sample.json";

/// `scripts/README-start.md`（使い方の案内。プロファイル名を挙げている）。
const START_README: &str = "scripts/README-start.md";

/// `.gitignore`（利用者の写しを追跡しないこと）。
const GITIGNORE: &str = ".gitignore";

/// スクリプトが「見本の置き場所」を書いている変数。
const UPSTREAM_SAMPLE_KEY: &str = "$UpstreamProfileSample = ";

/// スクリプトが「利用者の写しの置き場所」を書いている変数。
const UPSTREAM_LOCAL_KEY: &str = "$UpstreamProfileFile = ";

/// `-Upstream` の分岐の頭。**上流の設定は、この中でしか立てないこと。**
const UPSTREAM_BRANCH_HEAD: &str = "if ($PSBoundParameters.ContainsKey('Upstream'))";

/// **旗を実行ファイルへ流す行**（ADR-0042 §決定6 / 作業単位 F-4b-1）。
///
/// **スクリプトがするのは、これと置き場所を渡すことだけ。**
/// 読む・確かめる・起こす・止めるは、全部 personacore の中にある。
const UPSTREAM_FLAG_WIRING: &str = "$serverArgs = @('--upstream', $Upstream)";

/// **その引数をサーバへ渡す行。** 旗を渡さない起動では `$serverArgs` が空のまま。
const UPSTREAM_ARGS_HANDOFF: &str = "cargo run -- @serverArgs";

/// **置き場所を渡す行。**
const UPSTREAM_PROFILE_HANDOFF: &str = "$env:PERSONACORE_UPSTREAMS = $profilePath";

/// **スクリプトから消えた綴り**（ADR-0042 §決定6 / 作業単位 F-4b-1）。
///
/// # なぜ「無いこと」を見張るのか
///
/// **同じことをする実装を2つ持たない**（CLAUDE.md）。
/// **片方が黙って古びる**うえ、**配布物にはこのスクリプトが入らない**（ADR-0041）ので、
/// ここに残しておくと**「配ったものには無い機能」の案内**になる。
///
/// # ⚠ ここが赤くなったら、まず ADR-0042 を読むこと
///
/// **戻すのは「起こす実装が2つになる」ということである。**
/// `F-9` の PowerShell 実装は、**実機で1度回帰を出している**
/// （控えが PID を失い、`llama` が GPU を 11.7 GB 掴んだまま残った）。
const MOVED_INTO_THE_EXECUTABLE: [&str; 6] = [
    // プロセスを起こす・止める・数える。
    "Stop-Process",
    "Get-Process",
    "Start-UpstreamProcess",
    "Wait-UpstreamReady",
    // プロファイルを読む。
    "ConvertFrom-Json",
    "Read-UpstreamFace",
];

/// **上流の宛先を「決めも検証もしない」ことの錨**（レビュー是正6 / 作業単位 F-4b-1）。
///
/// **`-Upstream` を渡した起動では、`PERSONACORE_LLM_ENDPOINT`（や既定）は使われない。**
/// **それを検証すると、壊れた環境変数が立っている機では**
/// **プロファイルが勝つ起動でも `throw` で止まる。**
const ENDPOINT_CHECK_HEAD: &str = "if (-not $PSBoundParameters.ContainsKey('Upstream'))";

/// 検証の本体（この綴りが、上の分岐の中に在ること）。
const ENDPOINT_CHECK_BODY: &str = "$upstreamUri = $endpoint -as [uri]";

/// 文書が `-Upstream <名前>` の形で名前を挙げている鍵。
const UPSTREAM_FLAG: &str = "-Upstream ";

// ----------------------------------------------------------------------
// 走査の本体（**すべてソースを引数で受け取る**）
// ----------------------------------------------------------------------

/// PowerShell のソースから、**コメントと文字列リテラルを空白へ潰した写し**を作る。
///
/// 中括弧を数える前に必ず通す。**この家の doc とコメントは日本語**なので、
/// 潰さずに数えると `"` が1つ出ただけで以降が丸ごと狂う（CLAUDE.md `B-21d`）。
///
/// **返り値はバイト単位で同じ長さ**になる（潰した範囲は 1 バイトずつ空白で埋める）。
/// だから、ここで見つけた位置をそのまま元のソースへ使える。
///
/// # 読めないもの（**先に書く**）
///
/// * **二重引用符の中の入れ子は読まない。** 次の `"` までを丸ごと潰す。
/// * **ヒアドキュメントは、たまたま潰れる** —— 頭の `"` で始まり、閉じの `"` で終わるため。
///   **設計してそうしたのではない。**
/// * 逃がし（バッククォート）を見るのは二重引用符の中だけ。
fn masked_powershell(source: &str) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Code,
        LineComment,
        BlockComment,
        SingleQuoted,
        DoubleQuoted,
    }

    let bytes = source.as_bytes();
    let mut out = vec![b' '; bytes.len()];
    let mut state = State::Code;
    let mut at = 0usize;

    while at < bytes.len() {
        let byte = bytes[at];
        let keep_newline = byte == b'\n' || byte == b'\r';
        match state {
            State::Code => {
                if byte == b'<' && bytes.get(at + 1) == Some(&b'#') {
                    state = State::BlockComment;
                    at += 2;
                    continue;
                }
                if byte == b'#' {
                    state = State::LineComment;
                    at += 1;
                    continue;
                }
                if byte == b'\'' {
                    state = State::SingleQuoted;
                    at += 1;
                    continue;
                }
                if byte == b'"' {
                    state = State::DoubleQuoted;
                    at += 1;
                    continue;
                }
                out[at] = byte;
                at += 1;
            }
            State::LineComment => {
                if keep_newline {
                    out[at] = byte;
                    state = State::Code;
                }
                at += 1;
            }
            State::BlockComment => {
                if byte == b'#' && bytes.get(at + 1) == Some(&b'>') {
                    state = State::Code;
                    at += 2;
                    continue;
                }
                if keep_newline {
                    out[at] = byte;
                }
                at += 1;
            }
            State::SingleQuoted => {
                if byte == b'\'' {
                    state = State::Code;
                }
                if keep_newline {
                    out[at] = byte;
                }
                at += 1;
            }
            State::DoubleQuoted => {
                if byte == b'`' {
                    at += 2;
                    continue;
                }
                if byte == b'"' {
                    state = State::Code;
                }
                if keep_newline {
                    out[at] = byte;
                }
                at += 1;
            }
        }
    }

    String::from_utf8(out).expect("潰した範囲は ASCII の空白なので UTF-8 であること")
}

/// **潰した写しの中括弧を数えて**、`from` 以降の最初の `{` と、対応する `}` の位置を返す。
///
/// 返すのは `(開き括弧の次のバイト, 閉じ括弧のバイト)`。
fn brace_span_after(masked: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = masked.as_bytes();
    let open = (from..bytes.len()).find(|&at| bytes[at] == b'{')?;
    let mut depth = 0usize;
    for (offset, byte) in bytes[open..].iter().enumerate() {
        match *byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open + 1, open + offset));
                }
            }
            _ => {}
        }
    }
    None
}

/// `key` の直後の中括弧ブロックの範囲を返す（`key` は**元のソース**から探す）。
///
/// 鍵そのものは引用符を含みうるので、**探すのは元のソース、数えるのは潰した写し。**
fn block_span_for(source: &str, masked: &str, key: &str) -> Option<(usize, usize)> {
    let at = source.find(key)?;
    brace_span_after(masked, at + key.len())
}

/// **潰した写しの丸括弧を数えて**、`from` 以降の最初の `(` と、対応する `)` の位置を返す。
///
/// 返すのは `(開き括弧の次のバイト, 閉じ括弧のバイト)`。`brace_span_after` の丸括弧版
/// ——`param(…)` の中には `[ValidateRange(1, 65535)]` のような入れ子があるので、数えないと切れない。
fn paren_span_after(masked: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = masked.as_bytes();
    let open = (from..bytes.len()).find(|&at| bytes[at] == b'(')?;
    let mut depth = 0usize;
    for (offset, byte) in bytes[open..].iter().enumerate() {
        match *byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open + 1, open + offset));
                }
            }
            _ => {}
        }
    }
    None
}

/// スクリプト冒頭の `param(…)` の範囲を返す。
///
/// **最初の `param(` を採る。** この家のスクリプトは `[CmdletBinding()]` の直後に
/// スクリプト引数を書き、関数はその後ろに定義されているので、最初の1つが
/// **スクリプト全体で見える引数**になる。
fn script_param_span(masked: &str) -> Option<(usize, usize)> {
    let at = masked.find("param(")?;
    paren_span_after(masked, at)
}

/// `$名前` を拾って、**小文字に畳んで**返す。
///
/// 畳むのは、**PowerShell の変数名が大文字小文字を区別しない**から。
/// 畳まないと `$Upstream` と `$upstream` を別物として通してしまう。
fn dollar_names(masked: &str) -> BTreeSet<String> {
    let bytes = masked.as_bytes();
    let mut found = BTreeSet::new();
    for (at, _) in masked.match_indices('$') {
        let start = at + 1;
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        if end > start {
            found.insert(masked[start..end].to_ascii_lowercase());
        }
    }
    found
}

/// **スクリプト引数と同じ名前へ代入している箇所**を `(名前, 行番号)` で返す。
///
/// 見るのは代入（`$名前 =` / `$名前 +=` の頭）だけ。**読むのは違反ではない。**
/// `==` は落とす（比較であって代入ではない）。
///
/// # 読めないもの（**先に書く**）
///
/// * **`Set-Variable` / `New-Variable` の形は見ていない。**
/// * **関数の引数は見ていない。** 見るのはスクリプト冒頭の `param(…)` だけ
///   （そこに書いた名前が**スクリプト全体で見える**ため、事故の面がいちばん広い）。
/// * `$env:名前` は拾わない（`$` の直後の識別子が `env` で止まる）。
fn parameters_assigned_outside_the_param_block(masked: &str) -> Vec<(String, usize)> {
    let Some((param_start, param_end)) = script_param_span(masked) else {
        return Vec::new();
    };
    let names = dollar_names(&masked[param_start..param_end]);
    let bytes = masked.as_bytes();
    let mut found = Vec::new();

    for (at, _) in masked.match_indices('$') {
        if at >= param_start && at < param_end {
            continue;
        }
        let start = at + 1;
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        if end == start {
            continue;
        }
        let name = masked[start..end].to_ascii_lowercase();
        if !names.contains(&name) {
            continue;
        }

        // 直後の空白（`\r` は飛ばさない——代入は行を跨がない）を飛ばして `=` かを見る。
        let mut probe = end;
        while probe < bytes.len() && (bytes[probe] == b' ' || bytes[probe] == b'\t') {
            probe += 1;
        }
        if bytes.get(probe) == Some(&b'+') {
            probe += 1;
        }
        if bytes.get(probe) != Some(&b'=') || bytes.get(probe + 1) == Some(&b'=') {
            continue;
        }

        found.push((name, masked[..at].matches('\n').count() + 1));
    }
    found
}

/// `http://ホスト:ポート` から `(ホスト, ポート)` を取り出す。
///
/// ポートを省いた URL は `None` —— **この家の上流はどれもポートを名乗る。**
fn endpoint_host_port(endpoint: &str) -> Option<(String, u16)> {
    let after_scheme = endpoint.split_once("://")?.1;
    let authority = after_scheme.split('/').next()?;
    let (host, port) = authority.rsplit_once(':')?;
    Some((host.to_owned(), port.parse().ok()?))
}

/// 文書に現れる `-Upstream <名前>` の名前を全部拾う。
///
/// 使える文字は `[A-Za-z0-9_-]`。**後ろへ伸ばす**（`personacore_env_names` と同じ理由——
/// 綴り間違いを既知の名前に丸めない）。**空の名前は拾わない**
/// ——書き方の見本（山括弧の置き字）を「実在のプロファイル名」と読まないため。
fn upstream_names_named(source: &str) -> BTreeSet<&str> {
    let mut found = BTreeSet::new();
    for (at, _) in source.match_indices(UPSTREAM_FLAG) {
        let rest = &source[at + UPSTREAM_FLAG.len()..];
        let len = rest
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
            .unwrap_or(rest.len());
        if len > 0 {
            found.insert(&rest[..len]);
        }
    }
    found
}

/// 上流プロファイルの1つの面（`main` = 本文 / `sub` = 抽出・変容）。
#[derive(Debug, Clone)]
struct UpstreamFace {
    profile: String,
    face: String,
    endpoint: String,
    model: String,
    gguf: Option<String>,
}

/// 上流プロファイルの JSON を、面ごとに平らに並べる。
///
/// ここで**形そのもの**も見る——トップレベルはプロファイル名のオブジェクト、
/// 各プロファイルは `main`（必須）と `sub`（任意）だけ、面は `endpoint` と `model` を持つ。
/// **`sub` を省いたら本文と同じ上流**（ADR-0039 §決定1）。
fn upstream_faces(source: &str) -> Vec<UpstreamFace> {
    let root: JsonValue =
        serde_json::from_str(source).expect("上流プロファイルが JSON として読めること");
    let profiles = root
        .as_object()
        .expect("トップレベルがプロファイル名のオブジェクトであること");

    let mut faces = Vec::new();
    for (profile, body) in profiles {
        let body = body
            .as_object()
            .unwrap_or_else(|| panic!("プロファイル `{profile}` がオブジェクトでない"));
        assert!(
            body.contains_key("main"),
            "プロファイル `{profile}` に `main` が無い（本文の上流は省けない）"
        );
        for (face, spec) in body {
            assert!(
                face == "main" || face == "sub",
                "プロファイル `{profile}` に未知の面 `{face}` がある（`main` と `sub` だけ）"
            );
            let spec = spec
                .as_object()
                .unwrap_or_else(|| panic!("`{profile}` の `{face}` がオブジェクトでない"));
            let text = |key: &str| spec.get(key).and_then(JsonValue::as_str).map(str::to_owned);
            faces.push(UpstreamFace {
                profile: profile.clone(),
                face: face.clone(),
                endpoint: text("endpoint")
                    .unwrap_or_else(|| panic!("`{profile}` の `{face}` に endpoint が無い")),
                model: text("model")
                    .unwrap_or_else(|| panic!("`{profile}` の `{face}` に model が無い")),
                gguf: text("gguf"),
            });
        }
    }
    faces
}

// ----------------------------------------------------------------------
// 走査本体の対照（**植えたソースを、実ファイルと同じ本体に通す**）
// ----------------------------------------------------------------------

/// `masked_powershell` は、渡されたソースのコメントと文字列だけを潰すこと。
#[test]
fn the_powershell_mask_blanks_comments_and_strings_in_the_source_it_is_given() {
    // **期待値そのものを固定する**（`contains` では「潰しすぎ」も「潰し足りない」も見えない）。
    let planted = "a { # x }\nb = 'y}'\n<# z} #>\nc = \"w}\"\n}\n";
    let expected = format!(
        "a {{{}\nb ={}\n{}\nc ={}\n}}\n",
        " ".repeat(6),
        " ".repeat(5),
        " ".repeat(8),
        " ".repeat(5)
    );
    assert_eq!(masked_powershell(planted), expected, "植えたソースの潰し方");
    assert_eq!(
        masked_powershell(planted).len(),
        planted.len(),
        "バイト数を変えないこと（ここで見つけた位置を元のソースに使う）"
    );

    // **CRLF でも同じこと**（位置をバイトで数える本体なので腕を付ける）。
    let crlf = planted.replace('\n', "\r\n");
    assert_eq!(
        masked_powershell(&crlf),
        expected.replace('\n', "\r\n"),
        "CRLF でも同じ潰し方をすること"
    );
    assert_eq!(
        masked_powershell(&crlf).len(),
        crlf.len(),
        "CRLF でも長さ不変"
    );

    // **単一引用符の中の二重引用符**で状態が壊れないこと（実ファイルに実在する形）。
    assert_eq!(
        masked_powershell("t('\"') { }"),
        format!("t({}) {{ }}", " ".repeat(3)),
        "単一引用符の中の二重引用符で、以降が文字列だと誤読しないこと"
    );

    // **逃がし（バッククォート）の腕**——PowerShell は二重引用符の中の `"` を
    // バッククォートで逃がす。これを閉じ引用符と読むと、**そこから先をコードだと誤読して
    // 中括弧を数え始める**（切り出しが後ろの定義まで伸びる）。
    // **この1本を足すまで、その腕を外す変異は 2073 全緑を通った**
    // （F-9 のレビュアーが実測 / 2026-09-18。囮は植えたソースの側にしか置けない——
    // いまの `scripts/start.ps1` は、二重引用符の中でバッククォートを使っていない
    // ので、腕を外しても実ファイルの潰し方は1バイトも変わらない）。
    assert_eq!(
        masked_powershell("a = \"x`\"y\" { }"),
        format!("a ={}{{ }}", " ".repeat(8)),
        "逃がした二重引用符を閉じ引用符と読まないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    let script = start_script();
    let masked = masked_powershell(&script);
    assert_eq!(masked.len(), script.len(), "実ファイルでも長さ不変");
    assert!(
        masked.contains("param("),
        "実ファイルのコードが残ること（走査が全部潰していない）"
    );
    assert!(
        !masked.contains("SYNOPSIS"),
        "実ファイルのブロックコメントが潰れること"
    );
}

/// `brace_span_after` / `block_span_for` は、渡されたソースを数えること。
#[test]
fn the_brace_block_scan_reads_the_source_it_is_given() {
    let planted = "head {\n  inner { deep }\n  tail\n}\nafter {\n}\n";
    let (start, end) = block_span_for(planted, &masked_powershell(planted), "head")
        .expect("植えた塊を切り出せること");
    assert_eq!(
        &planted[start..end],
        "\n  inner { deep }\n  tail\n",
        "入れ子の閉じ括弧で早く閉じないこと"
    );

    // **CRLF の腕**（自分で添字を計算する本体なので付ける）。
    let crlf = planted.replace('\n', "\r\n");
    let (cstart, cend) =
        block_span_for(&crlf, &masked_powershell(&crlf), "head").expect("CRLF でも切り出せること");
    assert_eq!(
        &crlf[cstart..cend],
        "\r\n  inner { deep }\r\n  tail\r\n",
        "CRLF でも同じ範囲を返すこと"
    );

    // **コメントの中の閉じ括弧を掴まないこと**（CLAUDE.md `D-11a` が実際に踏んだ形）。
    let commented = "head {\n  # }\n  body\n}\n";
    let (dstart, dend) = block_span_for(commented, &masked_powershell(commented), "head")
        .expect("コメント入りでも切り出せること");
    assert_eq!(&commented[dstart..dend], "\n  # }\n  body\n");

    let unclosed = "head {\n";
    assert_eq!(
        block_span_for(unclosed, &masked_powershell(unclosed), "head"),
        None,
        "閉じていない塊は「見つからない」と言うこと"
    );
    let missing = "何も無い";
    assert_eq!(
        block_span_for(missing, &masked_powershell(missing), "head"),
        None,
        "鍵が無いソースを、黙って何かに解釈しないこと"
    );

    // **実ファイルも同じ本体を通ること。**
    let script = start_script();
    assert!(
        block_span_for(&script, &masked_powershell(&script), UPSTREAM_BRANCH_HEAD).is_some(),
        "実ファイルからも `-Upstream` の分岐を切り出せること"
    );
}

/// `parameters_assigned_outside_the_param_block` は、渡されたソースを読むこと。
#[test]
fn the_parameter_reuse_scan_reads_the_source_it_is_given() {
    // **大文字小文字を畳むこと。** これが畳めていないと、実ファイルの事故（`-Upstream` の
    // 引数へ `$upstream` で代入）を1件も拾わない。
    let planted = concat!(
        "param(\n",
        "    [ValidateRange(1, 65535)]\n",
        "    [int]$Port,\n",
        "\n",
        "    [string]$Upstream\n",
        ")\n",
        "$upstream = Start-UpstreamProcess -Launch $launch\n",
    );
    assert_eq!(
        parameters_assigned_outside_the_param_block(&masked_powershell(planted)),
        vec![("upstream".to_owned(), 7)],
        "綴りの違いだけ（`$Upstream` と `$upstream`）を別物として通さないこと"
    );

    // **CRLF の腕**（自分で添字と行数を数える本体なので付ける）。
    let crlf = planted.replace('\n', "\r\n");
    assert_eq!(
        parameters_assigned_outside_the_param_block(&masked_powershell(&crlf)),
        vec![("upstream".to_owned(), 7)],
        "CRLF でも同じ名前と同じ行を返すこと"
    );

    // **足し込みも代入である**（控えの配列を引数の名前で持つと同じ事故になる）。
    let plus = planted.replace("$upstream = ", "$upstream += ");
    assert_eq!(
        parameters_assigned_outside_the_param_block(&masked_powershell(&plus)),
        vec![("upstream".to_owned(), 7)],
        "`+=` も代入として拾うこと"
    );

    // **読むのは違反ではない。**
    let reads_only = concat!(
        "param(\n",
        "    [string]$Upstream\n",
        ")\n",
        "if ($Upstream -eq 'x') { Write-Host \"$Upstream\" }\n",
        "$launched = Start-UpstreamProcess -Launch $launch\n",
        "$upstreamLaunches = @()\n",
        "$env:UPSTREAM = 'x'\n",
        "$Upstreams = @()\n",
    );
    assert_eq!(
        parameters_assigned_outside_the_param_block(&masked_powershell(reads_only)),
        Vec::new(),
        "読み／別の綴り／`$env:` を代入と読まないこと"
    );

    // **比較は代入ではない。**
    let compared = concat!(
        "param(\n",
        "    [string]$Upstream\n",
        ")\n",
        "if ($Upstream == 'x') { }\n",
    );
    assert_eq!(
        parameters_assigned_outside_the_param_block(&masked_powershell(compared)),
        Vec::new(),
        "`==` を代入と読まないこと"
    );

    // **`param(` が無いソースを、黙って何かに解釈しないこと。**
    assert_eq!(
        parameters_assigned_outside_the_param_block(&masked_powershell("$upstream = 1\n")),
        Vec::new(),
        "引数の宣言が無ければ、突き合わせる相手がいない"
    );

    // **実ファイルも同じ本体を通ること**（引数を1つも拾えていない＝空振りを落とす）。
    let masked = masked_powershell(&start_script());
    let names = script_param_span(&masked)
        .map(|(start, end)| dollar_names(&masked[start..end]))
        .expect("実ファイルの `param(…)` を切り出せること");
    assert!(
        names.contains("upstream"),
        "実ファイルの引数に `upstream` が見えない（走査が空振りしている疑い）: {names:?}"
    );
}

/// `endpoint_host_port` は、渡された綴りを読むこと。
#[test]
fn the_endpoint_scan_reads_the_value_it_is_given() {
    assert_eq!(
        endpoint_host_port("http://127.0.0.1:4242"),
        Some(("127.0.0.1".to_owned(), 4242)),
        "植えた宛先を拾うこと"
    );
    assert_eq!(
        endpoint_host_port("http://127.0.0.1:4242/v1"),
        Some(("127.0.0.1".to_owned(), 4242)),
        "経路が付いていても権限部だけを読むこと"
    );
    assert_eq!(
        endpoint_host_port("http://0.0.0.0:4242"),
        Some(("0.0.0.0".to_owned(), 4242)),
        "**外へ出す綴りを拾うこと**（拾えなければこの検査は何もしていない）"
    );
    assert_eq!(
        endpoint_host_port("http://example.test"),
        None,
        "ポートを名乗らない URL は None"
    );
    assert_eq!(
        endpoint_host_port("127.0.0.1:1"),
        None,
        "スキームが無ければ None"
    );
}

/// `upstream_names_named` は、渡された文書の名前を拾うこと。
#[test]
fn the_upstream_name_scan_reads_the_source_it_is_given() {
    assert_eq!(
        upstream_names_named("pwsh ./scripts/start.ps1 -Upstream qwen-official\n"),
        BTreeSet::from(["qwen-official"]),
        "植えた名前を拾うこと"
    );
    assert_eq!(
        upstream_names_named("-Upstream a_b-1 と -Upstream c"),
        BTreeSet::from(["a_b-1", "c"]),
        "複数を拾うこと／使える文字で切れること"
    );
    assert!(
        upstream_names_named("`-Upstream <名前>`").is_empty(),
        "書き方の見本を実在の名前と読まないこと"
    );
    assert!(
        upstream_names_named("上流の話はしていない").is_empty(),
        "無い文書からは1件も拾わないこと"
    );
}

/// `upstream_faces` は、渡された JSON を読むこと。
#[test]
fn the_upstream_profile_scan_reads_the_source_it_is_given() {
    let planted = concat!(
        "{\n",
        "  \"planted\": {\n",
        "    \"main\": {\"endpoint\": \"http://127.0.0.1:1\", \"model\": \"m\"},\n",
        "    \"sub\": {\"endpoint\": \"http://127.0.0.1:2\", \"model\": \"s\", \"gguf\": \"~/x.gguf\"}\n",
        "  }\n",
        "}\n"
    );
    let faces = upstream_faces(planted);
    assert_eq!(faces.len(), 2, "植えた2面を拾うこと");
    assert_eq!(
        faces
            .iter()
            .map(|face| (
                face.profile.as_str(),
                face.face.as_str(),
                face.model.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![("planted", "main", "m"), ("planted", "sub", "s")],
        "面ごとに帰属を持って返すこと"
    );
    assert_eq!(faces[0].gguf, None, "gguf を書いていない面は None");
    assert_eq!(
        faces[1].gguf.as_deref(),
        Some("~/x.gguf"),
        "書いてある面はそのまま読むこと"
    );
    assert_eq!(faces[1].endpoint, "http://127.0.0.1:2");

    // **CRLF の腕**（見本は LF で書かれるが、clone した作業ツリーでは CRLF になる）。
    let crlf = planted.replace('\n', "\r\n");
    assert_eq!(upstream_faces(&crlf).len(), 2, "CRLF でも同じ数を読むこと");

    // **実ファイルも同じ本体を通ること。**
    assert!(
        !upstream_faces(&read_repo_file(UPSTREAMS_SAMPLE)).is_empty(),
        "実ファイルからも拾えること"
    );
}

// ----------------------------------------------------------------------
// 本命——スクリプト・見本・案内の突き合わせ
// ----------------------------------------------------------------------

/// **スクリプトが名指しする上流プロファイルの置き場所が、実在と食い違わないこと。**
///
/// 見本は**コミットする**（読めなければ `-Upstream` は何もできない）。
/// 利用者の写しは**追跡しない**（機ごとに gguf の置き場所が違う）。
#[test]
fn the_start_script_points_at_the_sample_profile_that_exists() {
    let script = start_script();

    let sample = ps_single_quoted(&script, UPSTREAM_SAMPLE_KEY)
        .expect("起動スクリプトから見本の置き場所を拾えること");
    assert_eq!(
        sample, UPSTREAMS_SAMPLE,
        "起動スクリプトが見ている見本と、このテストが見ている見本が違う"
    );
    assert!(
        repo_path(sample).exists(),
        "起動スクリプトが名指しする見本 `{sample}` が実在しない"
    );

    let local = ps_single_quoted(&script, UPSTREAM_LOCAL_KEY)
        .expect("起動スクリプトから利用者の写しの置き場所を拾えること");
    assert_ne!(
        Path::new(local),
        Path::new(sample),
        "利用者の写しと見本が同じ場所を指している（写しを作った瞬間に見本が上書きされる）"
    );

    let ignored = read_repo_file(GITIGNORE);
    let name = local
        .rsplit('/')
        .next()
        .expect("写しの綴りからファイル名を取れること");
    assert!(
        ignored
            .lines()
            .any(|line| line.trim() == local || line.trim() == name),
        "`{local}` が {GITIGNORE} に無い（機ごとの gguf の置き場所がコミットされる）"
    );
}

/// **上流の設定は、`-Upstream` の分岐の中でしか立てないこと。**
///
/// # なぜ（利用者の決定 / 2026-09-18）
///
/// **既定（ministral・LM Studio・旗なし）は変えない。**
/// この代入が分岐の外へ出ると、**旗を渡していない起動が黙ってプロファイルを読みに行く。**
/// 現れ方は「いつもどおり立ち上げたのに、別の上流を向いている」で、原因から遠い。
///
/// # ⚠ この見張り番の相手は、ADR-0042 で入れ替わった（司令塔の採否を仰いだ / 作業単位 F-4b-1）
///
/// **着手前は `PERSONACORE_LLM_ENDPOINT` など4つ**を見ていた。
/// **スクリプトはもうその4つを立てない**（実行ファイルがプロファイルから決める）ので、
/// **いま見るのは `PERSONACORE_UPSTREAMS` 1つ。**
/// **主張の形は1バイトも変わっていない**——「旗の分岐の中だけ」である。
/// **4つのほうは、`tests/upstream_cli.rs::
/// the_named_profile_reaches_the_executable_and_wins_over_the_environment` が
/// 実行ファイルで見る**（綴りではなく、実際に届いた値を）。
#[test]
fn the_upstream_environment_variables_are_set_only_inside_the_upstream_branch() {
    let script = start_script();
    let masked = masked_powershell(&script);
    let (start, end) = block_span_for(&script, &masked, UPSTREAM_BRANCH_HEAD)
        .expect("`-Upstream` の分岐を切り出せること");

    let assignment = format!("$env:{ENV_UPSTREAMS} = ");
    let hits: Vec<usize> = masked
        .match_indices(&assignment)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "`{assignment}` が1回きりでない（後ろで上書きすると、どちらが効くか分からない）"
    );
    assert!(
        hits[0] >= start && hits[0] < end,
        "`{assignment}` が `-Upstream` の分岐の外にある\
         （旗を渡していない起動が、黙ってプロファイルを読みに行く）"
    );

    // **4つのほうは、1つも立てないこと**（実行ファイルが決める / ADR-0042 §決定1）。
    for name in [
        ENV_LLM_ENDPOINT,
        ENV_SUB_LLM_ENDPOINT,
        ENV_MAIN_MODEL,
        ENV_SUB_MODEL,
    ] {
        let assignment = format!("$env:{name} = ");
        assert!(
            !masked.contains(&assignment),
            "起動スクリプトが `{assignment}` を書いている\
             （プロファイルの決定と、この行の2つが上流を決めることになる）"
        );
    }
}

/// **旗は、実行ファイルへそのまま流すこと**（ADR-0042 §決定6）。
///
/// # 3つを1本で見る
///
/// | 見るもの | 破れると |
/// |---|---|
/// | 置き場所を渡す行が**分岐の中に1つ** | 旗を渡していない起動がプロファイルを読む |
/// | 旗を組む行が**分岐の中に1つ** | 名前が実行ファイルに届かない（**黙って既定の上流で立つ**） |
/// | その引数を `cargo run` へ渡す行が1つ | 同上。**組んだのに渡していない**形 |
///
/// # ⚠ 守っていないもの
///
/// **綴りの検査でしかない。** 「その行が効いていること」は見ていない
/// （このファイルは `pwsh` を起動しない / F-1 の判断）。
/// **実際に届くことは `tests/upstream_cli.rs` が実行ファイルで見る。**
#[test]
fn the_start_script_hands_the_upstream_flag_to_the_executable() {
    let script = start_script();
    let masked = masked_powershell(&script);
    let (start, end) = block_span_for(&script, &masked, UPSTREAM_BRANCH_HEAD)
        .expect("`-Upstream` の分岐を切り出せること");

    // **⚠ 素のソースで探す。** `masked_powershell` は**文字列リテラルを潰す**ので、
    // `@('--upstream', $Upstream)` のような綴りは masked には残らない
    // （置き場所を切り出すのには masked を使う——コメントの中の同じ綴りを数えないため）。
    for wiring in [UPSTREAM_PROFILE_HANDOFF, UPSTREAM_FLAG_WIRING] {
        let hits: Vec<usize> = script.match_indices(wiring).map(|(at, _)| at).collect();
        assert_eq!(hits.len(), 1, "`{wiring}` が1回きりでない");
        assert!(
            hits[0] >= start && hits[0] < end,
            "`{wiring}` が `-Upstream` の分岐の外にある"
        );
    }

    assert_eq!(
        occurrences(&masked, UPSTREAM_ARGS_HANDOFF),
        1,
        "`{UPSTREAM_ARGS_HANDOFF}` が1回きりでない（組んだ引数を渡す場所は1つ）"
    );
    // **渡す行は分岐の外**（旗を渡さない起動もここを通る。そのとき `$serverArgs` は空）。
    let handoff = masked
        .find(UPSTREAM_ARGS_HANDOFF)
        .expect("引数を渡す行が在ること");
    assert!(
        handoff >= end,
        "引数を渡す行が分岐の中にある（旗を渡さない起動がサーバを起こさなくなる）"
    );
}

/// `-NoBrowser` の分岐の頭（作業単位 F-4c）。
const NO_BROWSER_BRANCH_HEAD: &str = "if ($NoBrowser)";

/// **旗を実行ファイルへ流す行**（作業単位 F-4c）。**ブラウザを開くのは personacore 自身。**
const NO_BROWSER_FLAG_WIRING: &str = "$serverArgs += '--no-browser'";

/// **スクリプトから消えた、ブラウザを開く綴り**（作業単位 F-4c）。
///
/// **残すと二重に開く**（personacore も開く）。
const BROWSER_MOVED_INTO_THE_EXECUTABLE: [&str; 2] = ["Start-Process", "Start-Job"];

/// **ブラウザを開くのは実行ファイル。スクリプトは `--no-browser` を流すだけ**（作業単位 F-4c）。
///
/// # 1本で見ているもの
///
/// | 見るもの | 破れると |
/// |---|---|
/// | 開く綴り（`Start-Process` / `Start-Job`）が**無い** | **二重に開く** |
/// | `--no-browser` を足す行が **`-NoBrowser` の分岐の中に1つ** | `-NoBrowser` が効かない／常に効く |
/// | その行が **`-Upstream` の分岐より後** | **`-Upstream` の分岐が `$serverArgs` を代入で作り直すので、`--no-browser` が黙って消える** |
/// | その行が **`cargo run -- @serverArgs` より前** | 足した旗が渡らない |
///
/// # ⚠ 守っていないもの
///
/// **綴りの検査でしかない。** 別の綴りで開く実装（`Invoke-Item` 等）は素通りする。
/// **旗が実行ファイルに効くことは `tests/upstream_cli.rs` が見る**
/// （`an_existing_profile_file_is_left_alone_and_no_browser_keeps_it_closed`）。
#[test]
fn the_start_script_leaves_opening_the_browser_to_the_executable() {
    let script = start_script();
    let masked = masked_powershell(&script);

    for moved in BROWSER_MOVED_INTO_THE_EXECUTABLE {
        assert!(
            !masked.contains(moved),
            "起動スクリプトに `{moved}` が残っている（personacore もブラウザを開くので、二重に開く）"
        );
    }

    // **⚠ 素のソースで探す**（`'--no-browser'` は文字列リテラルなので masked では潰れる）。
    let hits: Vec<usize> = script
        .match_indices(NO_BROWSER_FLAG_WIRING)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(hits.len(), 1, "`{NO_BROWSER_FLAG_WIRING}` が1回きりでない");
    let (start, end) = block_span_for(&script, &masked, NO_BROWSER_BRANCH_HEAD)
        .expect("`-NoBrowser` の分岐を切り出せること");
    assert!(
        hits[0] >= start && hits[0] < end,
        "`{NO_BROWSER_FLAG_WIRING}` が `-NoBrowser` の分岐の外にある"
    );

    let (_, upstream_end) = block_span_for(&script, &masked, UPSTREAM_BRANCH_HEAD)
        .expect("`-Upstream` の分岐を切り出せること");
    assert!(
        hits[0] > upstream_end,
        "`--no-browser` を足す行が `-Upstream` の分岐より前にある（代入で作り直されて消える）"
    );
    let handoff = masked
        .find(UPSTREAM_ARGS_HANDOFF)
        .expect("引数を渡す行が在ること");
    assert!(
        hits[0] < handoff,
        "`--no-browser` を足す行が、引数を渡す行より後にある（渡らない）"
    );
}

/// **スクリプトは、もう上流を起こさないし止めないし読まない**（ADR-0042 §決定6）。
///
/// # なぜ「無いこと」を見張るのか
///
/// **同じことをする実装を2つ持たない**（CLAUDE.md）。**片方が黙って古びる。**
/// **そして配布物にはこのスクリプトが入らない**（ADR-0041）ので、
/// 残しておくと**「配ったものには無い機能」の案内**になる。
///
/// # ⚠ この1本は、2本を置き換えたものである（司令塔の採否を仰いだ / 作業単位 F-4b-1）
///
/// **消したのは `the_started_upstreams_are_stopped_by_recorded_pid_when_the_script_ends` と
/// `the_started_upstream_is_recorded_before_the_script_waits_for_it`。**
/// **どちらも主張が実行ファイルへ移った**ので、そちらで見る:
///
/// | 移った主張 | 移った先で、何がそれを保つか |
/// |---|---|
/// | 控えた PID だけを止める | **見張り番**: `tests/upstream_launch.rs::stopping_the_server_leaves_someone_elses_llama_alone`（偽物を2つ起こし、他人の側が生き残ることを見る） |
/// | 起こすのは待ち受けの後 | **見張り番**: 同 `a_failed_bind_leaves_no_child_behind`（親の足跡が1つも残らないことを見る） |
/// | **控えるのは待つ前** | **見張り番は無い。所有と `kill_on_drop` が構造的に保証する**（下記） |
///
/// **上の2つは、綴りではなく本物のプロセスで見ている。**
///
/// # ⚠ 3つ目だけは「見張り番」ではない（レビュー是正9 / 2026-09-19）
///
/// **PowerShell では、控えは `Start-Process` が返した PID の写し**だったので、
/// **「起こしたが、まだ控えていない」窓が現に存在した**（`F-9` のレビューが閉じた穴）。
///
/// **Rust では、その窓が構造的に存在しない** ——
/// **控えは `tokio::process::Child` そのもの**で、`spawn` が返した瞬間から
/// **`kill_on_drop(true)` が効いている。** 控える前に落ちる経路でも子は死ぬ。
/// **`Supervisor` の外へ `Child` を渡す道が無い**ので、「控え忘れ」を書く形が無い。
///
/// **⚠ だから「この見張り番が控えの位置を見ている」と書かないこと。**
/// **初版はここに `the_named_gguf_is_started_reaches_ready_and_is_stopped_with_the_server` を
/// 書いていたが、あの腕は控えの位置を1バイトも観測していない。**
/// **「作法がある」と「見張り番がある」は別のことである**（CLAUDE.md `D-9`）。
///
/// # ⚠ 守っていないもの
///
/// **綴りの検査でしかない。** 別の綴りで同じことをする実装（`taskkill` を呼ぶ等）は素通りする。
#[test]
fn the_start_script_no_longer_starts_or_stops_the_upstream() {
    let masked = masked_powershell(&start_script());

    for moved in MOVED_INTO_THE_EXECUTABLE {
        assert!(
            !masked.contains(moved),
            "起動スクリプトに `{moved}` が残っている\
             （上流を起こす／止める／読む実装が2つになる。ADR-0042 §決定6）"
        );
    }

    // **空振りではないこと**——旗そのものは、いまも在る。
    // **⚠ 素のソースで探す**（`UPSTREAM_BRANCH_HEAD` は文字列リテラルを含み、masked では潰れる）。
    assert!(
        start_script().contains(UPSTREAM_BRANCH_HEAD),
        "`-Upstream` の分岐ごと消えている（走査が空振りしている疑い）"
    );
}

/// **`-Upstream` を渡した起動では、上流の宛先を決めも検証もしないこと**
/// （ADR-0042 §決定6 / レビュー是正6 / 2026-09-19）。
///
/// # なぜ（疎通の確認を黙らせたのと同じ理由）
///
/// **旗を渡した起動で、宛先を決めるのはプロファイルである。**
/// スクリプトはその中身を読まないので、**`PERSONACORE_LLM_ENDPOINT`（や既定）を**
/// **検証しても、その値はその起動では一度も使われない。**
/// **にもかかわらず `throw` は起動を止める** ——
/// **壊れた環境変数が立っている機では、プロファイルが勝つはずの起動が立たない。**
///
/// **書式が壊れていれば、実行ファイルの側が自分で止める**（`AppConfig` の検証）。
///
/// # ⚠ 守っていないもの
///
/// **綴りと位置しか見ていない。** 走らせて確かめてはいない（このファイルは `pwsh` を起動しない）。
#[test]
fn the_start_script_skips_the_endpoint_check_when_a_profile_is_named() {
    let script = start_script();
    let masked = masked_powershell(&script);

    // **⚠ 分岐の頭は文字列リテラルを含むので、素のソースで探す**
    // （`masked_powershell` はリテラルを潰す）。位置の比較は素のソースの添字で行う。
    assert_eq!(
        occurrences(&script, ENDPOINT_CHECK_HEAD),
        1,
        "`{ENDPOINT_CHECK_HEAD}` が1回きりでない（どちらの分岐を見たか分からなくなる）"
    );
    let (start, end) = block_span_for(&script, &masked, ENDPOINT_CHECK_HEAD)
        .expect("宛先の検証を囲う分岐を切り出せること");

    let hits: Vec<usize> = script
        .match_indices(ENDPOINT_CHECK_BODY)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "`{ENDPOINT_CHECK_BODY}` が1回きりでない（外にもう1つ在れば、旗を渡しても検証が走る）"
    );
    assert!(
        hits[0] >= start && hits[0] < end,
        "上流の宛先の検証が `-Upstream` を除く分岐の外にある\
         （壊れた PERSONACORE_LLM_ENDPOINT が立っている機で、\
         プロファイルが勝つはずの起動が止まる）"
    );
}

/// **スクリプト引数と同じ名前の変数へ代入しないこと。**
///
/// # なぜ（2026-09-18 に実機で実測。F-9 の2周目）
///
/// **PowerShell の変数名は大文字小文字を区別しない。** そして `param(…)` に型を書くと、
/// **その型はスクリプト全体で、その名前への代入すべてに効く。**
///
/// `$upstream = Start-UpstreamProcess …` は、引数 `[string]$Upstream` への代入だったので、
/// **返ってきた `Process` が黙って文字列 `System.Diagnostics.Process (llama)` に化けた。**
/// 現れ方はこう ——
///
/// ```text
/// [上流] main: llama serve を起こします（http://127.0.0.1:8080）
/// start.ps1: このオブジェクトにはプロパティ 'Id' がありません。
/// ```
///
/// **控えに PID が入らず、`finally` は何も止められず、llama が GPU を掴んだまま残った。**
/// しかも `finally` の中で出たこの例外が、**本当の原因（`Wait-UpstreamReady` への型違いの
/// 受け渡し）を上書きして消した**ので、読み手には原因が1つも見えない。
///
/// # ⚠ 守っていないもの
///
/// * **関数の引数は見ていない**（走査の doc を参照）。見ているのはスクリプト冒頭の
///   `param(…)` だけ。
/// * **「型が付いている引数だけ」に絞っていない。** 型の無い引数への代入も違反として拾う
///   ——同じ名前を2つの意味で使うこと自体が、この事故の素だから。
/// # ⚠ 見る相手を広げた（作業単位 F-4a）
///
/// **`scripts/package.ps1` も同じ穴を持ちうる**ので、[`PARAM_SCRIPTS`] の
/// **全部**に当てる。**1つずつ書き足す形にしない**——足した日に書き忘れる。
/// **「`param(…)` を持つスクリプトが増えたら、ここへ足す」が唯一の作法**で、
/// **その足し忘れを止める仕掛けは無い**（CLAUDE.md「免除の一覧」と同じ限界）。
#[test]
fn no_script_parameter_is_reused_as_a_local_variable() {
    for script in PARAM_SCRIPTS {
        let masked = masked_powershell(&read_repo_file(script));
        assert!(
            script_param_span(&masked).is_some(),
            "{script} から param(…) を拾えていない（走査が空振りしている疑い）"
        );

        let reused = parameters_assigned_outside_the_param_block(&masked);
        assert_eq!(
            reused,
            Vec::new(),
            "{script} がスクリプト引数と同じ名前へ代入している\
             （引数に型が付いていれば、代入した値がその型へ黙って変換される）"
        );
    }
}

/// **配布物は「画面 → 実行ファイル」の順に作ること**（ADR-0041 §決定5 / レビュー指摘7）。
///
/// # なぜ順番が本質なのか
///
/// **画面はコンパイル時に実行ファイルへ埋め込まれる**（ADR-0041 §決定2）。
/// **`cargo build --release` を先に回すと、その時点の（古い、または空の）画面が入る。**
/// **入っても作成は成功し、起動も成功する**ので、
/// **気づく手掛かりは起動ログの `ui_files` だけ**である。
///
/// # 4つを1本で見る
///
/// | 見るもの | 破れると |
/// |---|---|
/// | 画面のビルドが1回きり | どちらを見ているか分からなくなる（門） |
/// | 画面 → cargo の順 | **古い画面が入る**（この検査の本命） |
/// | `--release` の綴り | **debug を配る**（画面は入っていても別物） |
/// | 複製元が `target/release` の下 | **debug の実行ファイルを配る** |
///
/// # ⚠ 守っていないもの
///
/// **綴りの順番しか見ていない。** **実際にその順で回ること**は見ていない
/// （このファイルは `pwsh` を起動しない / F-1 の判断）。
/// **コメントと文字列リテラルは潰してから見る**ので、
/// 説明文や `Write-Host` の中の同じ綴りは数えない。
#[test]
fn the_package_script_builds_the_web_before_the_executable() {
    let script = read_repo_file(PACKAGE_SCRIPT);
    let masked = masked_powershell(&script);

    // **門**: それぞれ1回きりであること（複数在ると、どれを見たか分からない）。
    for needle in [PACKAGE_WEB_BUILD, PACKAGE_CARGO_BUILD] {
        assert_eq!(
            occurrences(&masked, needle),
            1,
            "{PACKAGE_SCRIPT} の `{needle}` が1回きりでない"
        );
    }

    let web_at = masked
        .find(PACKAGE_WEB_BUILD)
        .expect("画面をビルドする呼び出しが在ること");
    let cargo_at = masked
        .find(PACKAGE_CARGO_BUILD)
        .expect("実行ファイルを作る呼び出しが在ること");

    assert!(
        web_at < cargo_at,
        "{PACKAGE_SCRIPT} が実行ファイルを先に作っている\
         （画面はコンパイル時に埋め込まれるので、古い画面が入る）"
    );

    // **複製元は `target/release` の下**（debug を配らない）。
    let relative = ps_single_quoted(&script, PACKAGE_BIN_KEY).expect("複製元の宣言を拾えること");
    assert!(
        relative.starts_with(PACKAGE_BIN_PREFIX),
        "{PACKAGE_SCRIPT} の複製元が `{PACKAGE_BIN_PREFIX}` の下にない: {relative}"
    );
}

/// **cargo-about が無ければ、画面とリリースビルドより先に止まること**（O-1 のレビュー1周目 要修正2）。
///
/// 表示を作り直す段（4）は cargo-about を要る。確かめる段が後ろにあると、
/// **数分のリリースビルドを終えてから「cargo about が無い」で止まる。**
///
/// # ⚠ 守っていないもの
///
/// **綴りの順番しか見ていない**（このファイルは `pwsh` を起動しない / F-1 の判断）。
/// 無いときに本当に止まることは、作業者が cargo-about の無い `PATH` で1回走らせて確かめた（O-1 の報告）。
#[test]
fn the_package_script_checks_for_cargo_about_before_building() {
    let script = read_repo_file(PACKAGE_SCRIPT);
    let masked = masked_powershell(&script);

    assert_eq!(
        occurrences(&masked, PACKAGE_ABOUT_CHECK),
        1,
        "{PACKAGE_SCRIPT} の `{PACKAGE_ABOUT_CHECK}` が1回きりでない"
    );
    let check_at = masked
        .find(PACKAGE_ABOUT_CHECK)
        .expect("cargo-about を確かめる呼び出しが在ること");
    for later in [PACKAGE_WEB_BUILD, PACKAGE_CARGO_BUILD] {
        let at = masked.find(later).expect("ビルドの呼び出しが在ること");
        assert!(
            check_at < at,
            "{PACKAGE_SCRIPT} が cargo-about を確かめる前に `{later}` を回している"
        );
    }
}

/// **zip は、実行ファイルを作って複製した「後」に、複製したものから組むこと**（作業単位 F-4d / Issue origin#25）。
///
/// # 見るもの
///
/// | 見るもの | 破れると |
/// |---|---|
/// | zip の呼び出しが1回きり | どちらを見ているか分からなくなる（門） |
/// | `cargo build --release` → 複製 → zip の順 | **古い（前回の）実行ファイルが zip に入る** |
/// | 渡す exe が `$outBin`（複製した後のもの） | 配った exe と zip の中の exe が別物になりうる |
/// | 表示を作り直す呼び出しが1回きりで、zip より先（O-1） | **依存が増減したのに、古い表示を詰める** |
/// | 4つの置き場所の宣言が実在するファイルを指し、宣言から組み立てている | **宣言が死んだ変数になる**（`DB_WIRING` と同じ穴） |
/// | 詰める表示が、`notices.ps1` の書き出す先と同じ（O-1） | **作り直した表示ではなく、別のファイルを詰める** |
///
/// # ⚠ 守っていないもの
///
/// **綴りの順番しか見ていない。** zip の段そのものを走らせるのは `tests/distribution.rs`
/// （植えた exe で `package-zip.ps1` を走らせる）で、**`package.ps1` を最後まで通すのは司令塔の実機。**
#[test]
fn the_package_script_zips_the_copied_executable_last() {
    let script = read_repo_file(PACKAGE_SCRIPT);
    let masked = masked_powershell(&script);

    for needle in [PACKAGE_ZIP_CALL, PACKAGE_COPY, PACKAGE_NOTICES_CALL] {
        assert_eq!(
            occurrences(&masked, needle),
            1,
            "{PACKAGE_SCRIPT} の `{needle}` が1回きりでない"
        );
    }
    let cargo_at = masked
        .find(PACKAGE_CARGO_BUILD)
        .expect("実行ファイルを作る呼び出しが在ること");
    let copy_at = masked.find(PACKAGE_COPY).expect("複製が在ること");
    let zip_at = masked
        .find(PACKAGE_ZIP_CALL)
        .expect("zip の呼び出しが在ること");
    assert!(
        cargo_at < copy_at && copy_at < zip_at,
        "{PACKAGE_SCRIPT} の順が「cargo → 複製 → zip」でない"
    );
    let notices_at = masked
        .find(PACKAGE_NOTICES_CALL)
        .expect("表示を作り直す呼び出しが在ること");
    assert!(
        notices_at < zip_at,
        "{PACKAGE_SCRIPT} が表示を作り直す前に zip を組んでいる（古びた表示を詰める）"
    );

    for (key, expected) in [
        (PACKAGE_ZIP_SCRIPT_KEY, ZIP_SCRIPT),
        (PACKAGE_DIST_README_KEY, DIST_README),
        (PACKAGE_NOTICES_SCRIPT_KEY, NOTICES_SCRIPT),
        (PACKAGE_DIST_NOTICES_KEY, DIST_NOTICES),
    ] {
        let declared = ps_single_quoted(&script, key).expect("置き場所の宣言を拾えること");
        assert_eq!(declared, expected, "{PACKAGE_SCRIPT} の `{key}` が違う");
        assert!(
            repo_path(declared).is_file(),
            "{PACKAGE_SCRIPT} の `{key}` が指す {declared} が無い"
        );
    }
    for wiring in PACKAGE_ZIP_WIRING {
        assert_eq!(
            occurrences(&masked, wiring),
            1,
            "{PACKAGE_SCRIPT} が宣言から組み立てていない: `{wiring}`"
        );
    }

    // **詰める表示は、作り直した表示そのもの**（`notices.ps1` の書き出す先と同じ綴り）。
    let notices_script = read_repo_file(NOTICES_SCRIPT);
    let written = ps_single_quoted(&notices_script, NOTICES_OUT_KEY)
        .expect("notices.ps1 の書き出す先の宣言を拾えること");
    assert_eq!(
        written, DIST_NOTICES,
        "{NOTICES_SCRIPT} の書き出す先と {PACKAGE_SCRIPT} が詰める表示が違う"
    );
}

/// **見本のどの面も、ループバック以外の宛先を名乗らないこと。**
///
/// `the_start_script_names_no_address_other_than_the_loopback` と同じ主張を、
/// **プロファイルの JSON にも当てる** —— 宛先はスクリプトからこちらへ移った。
/// **移した先が無防備なら、移した瞬間に守りが消える**（CLAUDE.md `B-6`）。
///
/// # ⚠ これは「外部公開を防いでいる」ではない
///
/// 見ているのは**利用者が写して使う見本の綴り**だけ。
/// 利用者の写し（`scripts/upstreams.json`）には届かない。
#[test]
fn the_upstream_profiles_name_no_address_other_than_the_loopback() {
    let sample = read_repo_file(UPSTREAMS_SAMPLE);
    let loopback = BindAddr::loopback(DEFAULT_PORT)
        .socket_addr()
        .ip()
        .to_string();

    assert_eq!(
        ipv4_literals(&sample),
        BTreeSet::from([loopback.clone()]),
        "{UPSTREAMS_SAMPLE} が {loopback} 以外の宛先を名乗っている"
    );

    // **「載っているか」の次は「どれのものか」**（CLAUDE.md `D-11b`）。
    let faces = upstream_faces(&sample);
    assert!(!faces.is_empty(), "見本が空（走査が空振りしている疑い）");
    for face in &faces {
        let (host, _) = endpoint_host_port(&face.endpoint).unwrap_or_else(|| {
            panic!(
                "`{}` の `{}` の endpoint がホストとポートに読めない: {}",
                face.profile, face.face, face.endpoint
            )
        });
        assert_eq!(
            host, loopback,
            "プロファイル `{}` の `{}` が {loopback} 以外を向いている",
            face.profile, face.face
        );
    }
}

/// **1つのプロファイルの中で、gguf を持つ2つの面が同じポートを指さないこと。**
///
/// # なぜ（`llama serve` は 1 プロセス 1 モデル）
///
/// gguf が書いてある面は、スクリプトが**その面のために1プロセス起こす。**
/// 2つの面が同じポートを名乗っていたら、**2つめは束ねられずに黙って失敗する**か、
/// **1つめの上流が両方の面に答える**（`llama serve` は `model` の欄を見ない / `G-1`）。
/// どちらも「設定したつもりで別のモデルが答えている」という、いちばん見えない形になる。
#[test]
fn no_upstream_profile_points_two_gguf_at_the_same_port() {
    let sample = read_repo_file(UPSTREAMS_SAMPLE);
    let faces = upstream_faces(&sample);

    let mut launched = 0usize;
    let mut seen: BTreeSet<(String, u16)> = BTreeSet::new();
    for face in &faces {
        if face.gguf.is_none() {
            continue;
        }
        launched += 1;
        let (_, port) = endpoint_host_port(&face.endpoint)
            .unwrap_or_else(|| panic!("`{}` の endpoint を読めること", face.profile));
        assert!(
            seen.insert((face.profile.clone(), port)),
            "プロファイル `{}` が、同じポート {port} に2つの gguf を載せようとしている",
            face.profile
        );
    }

    assert!(
        launched >= 2,
        "見本に gguf を持つ面が {launched} 件しかない（この検査が届く形になっていない）"
    );
}

/// **案内（README）が挙げるプロファイル名と、見本にあるプロファイル名が一致すること。**
///
/// 片方だけ増やすと、**在りもしない名前を打たせる案内**か、
/// **誰も知らないプロファイル**ができる。どちらも「書いてあるのに動かない」になる。
#[test]
fn the_readme_names_exactly_the_profiles_the_sample_defines() {
    let readme = read_repo_file(START_README);
    let sample = read_repo_file(UPSTREAMS_SAMPLE);

    let named = upstream_names_named(&readme);
    assert!(
        !named.is_empty(),
        "{START_README} が `{UPSTREAM_FLAG}` に続けて名前を1つも挙げていない（走査が空振りしている疑い）"
    );

    let defined: BTreeSet<String> = upstream_faces(&sample)
        .into_iter()
        .map(|face| face.profile)
        .collect();
    let named: BTreeSet<String> = named.into_iter().map(str::to_owned).collect();

    assert_eq!(
        named, defined,
        "{START_README} が挙げる名前と {UPSTREAMS_SAMPLE} のプロファイルが食い違っている"
    );
}
