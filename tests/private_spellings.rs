//! 個人を特定しうる綴り（開発機のユーザー名・私的なパス）が、リポジトリに残っていないことを見る。
//!
//! # なぜ要るか（作業単位 O-4 / ADR-0048 §4）
//!
//! 公開すると、`git ls-files` に載っているものは全部が世界に読める。
//! 絶対パス（`C:\Users\<ユーザー名>\…` / 開発機のドライブ配置）は、
//! docs・コメント・**テストの入力データ**に、書いた本人も気づかないまま入り込む。
//! 2026-09-27 の監査で約 70 か所あった。**一度片付けても、次の作業単位が書き戻す。**
//! だから綴りそのものを見張る。
//!
//! # 何を見ているか
//!
//! `git ls-files` が挙げる**追跡されたファイル**を（UTF-8 として不正なバイトは置き換え文字にして）1行ずつ読み、
//! 次の綴りが無いことを見る。**大文字小文字・`\` と `/`・JSON / Rust の文字列の `\\` を区別しない**
//! （ドライブ文字つきの大文字の綴りも、小文字で `/` 区切りの綴りも、エスケープした `\\` の綴りも、同じものとして当たる）。
//!
//! | 種類 | 綴り |
//! |---|---|
//! | ユーザー名 | 開発機のユーザー名（**ただし、その直後に `ei` が続く綴りは除く** —— 公開するアカウント名で、リポジトリの URL に入る。綴りは下の `leaks_in_line` にだけ、断片で書く） |
//! | 開発機のドライブ配置 | **Git Bash 形（ドライブ文字だけの階層）・WSL 形（`mnt` 経由）・連なった区切りも同じ綴りとして当たる**。開発機の `F:` の Rust 置き場・`F:` の Other 置き場・`D:` のモデル置き場、および Claude が作るプロジェクト名の綴り（**綴りそのものは下の `leaks_in_line` にだけ、断片で書く**） |
//! | メールアドレス | **誰のものでも**、メールアドレスの形（`局所部@ドメイン`、ドメインは `.` を1つ以上含み、最後のラベルが2文字以上の英字）。作業単位 O-11 で足した。例示用のドメインと、下の除外だけを通す |
//!
//! ## メールアドレスの形と除外（作業単位 O-11 / origin#105）
//!
//! 局所部に使えるのは ASCII の英字・数字と `.` `_` `%` `+` `-`、ドメインに使えるのは ASCII の英字・数字と `.` `-`
//! （[`email_addresses_in_line`]）。区切りは `@` と、全角の `＠`（U+FF20）の両方を読む。
//! **最後のラベルが英字だけのときに限る**ので、版の綴り（`vite@8.2.2`）と、URL のユーザー情報つきの IP アドレス
//! （`http://user:pass@127.0.0.1`）は当たらない。除外は次の3つだけで、どれも理由を持つ。
//!
//! * **例示用のドメイン**（[`is_example_domain`]）: `example.com` / `example.net` / `example.org`（とその下）、
//!   最上位が `.example` / `.test` / `.invalid` のもの。RFC 2606 / RFC 6761 が例示とテストのために取ってあり、
//!   実在の誰にも届かない。テストの入力（`http://user:pass@example.com` 等）が使う。
//! * **生成物のファイル**（[`EMAIL_EXEMPT_FILES`]）: `scripts/THIRD-PARTY-NOTICES.txt`。依存の著作権表示を
//!   `cargo-about` が写したもので、作者の連絡先は表示の一部（ADR-0048 §4 の O-1）。手で直すと次の生成で戻る。
//!   **このファイルで飛ばすのはメールアドレスだけ**で、ユーザー名とドライブ配置は見る。
//! * **どうしても要る固定の綴り**（[`EMAIL_ALLOWED_ADDRESSES`]）: 理由つきで1つずつ。
//!
//! 除外は、**どれも実際に1回以上当たること**を見る（直した後に除外だけが残らないように /
//! [`every_email_exemption_is_still_needed`]）。O-11 で足す前の数は、生成物 169 か所・例示用のドメイン 6 か所・
//! 固定の綴り 1 か所で、それ以外は 0 だった。
//!
//! # 何を見ていないか（先に書く）
//!
//! * **知らない人のユーザー名は見ていない。** 見ているのは**この開発機の綴りだけ**で、
//!   別の機で開発した人が別の綴りを入れても鳴らない（`C:\Users\<誰か>\` を一般に禁じると、
//!   `C:\Users\…` や `C:\Users\someone\` の正当な例まで巻き込む）。
//! * **実名はここでは見ていない。** 利用者の実名は、綴りを木に書けないので
//!   `tests/hashed_spellings.rs` が見る（テストの実行時に `git config user.name` を読む —— 詳細はそのファイルの頭）。
//! * **メールアドレスの形の外は見ていない。** `@` の前後を分けた書き方（`someone [at] example`）・
//!   百分率符号で書いた `%40`・日本語のドメイン・最上位が英字でないもの・局所部に引用符を使う形は当たらない。
//! * **語の境界**: ドライブ配置の綴りは、直後が英数字・`_` のとき（別の語の一部）は当てない
//!   （`F:\Rustacean` のような無関係な語に誤爆しない）。**末尾に区切りが無い綴り**（行末・空白・`` ` `` の前）は当てる。
//!   逆に、**語の一部を前から伸ばした綴り**（別のドライブの同名ディレクトリ等）は見ていない。
//! * **追跡されていないファイル・`.git/` の中（コミットのメッセージ・作者）は見ていない。**
//! * **UTF-8 として不正なファイルも読む**（不正なバイトだけを置き換え文字にし、ASCII の部分は見る）。
//!   **UTF-16 などで書かれたファイルは見ていない**（文字の間に別のバイトが挟まり、綴りとして読めない）。
//! * **このファイルも走査の対象**である。禁じる綴りは**断片から組み立てて**書いてあり、
//!   このファイル自身には現れない。
//!
//! 許可リスト [`ALLOWED`] は空である。足すときは「なぜここだけ残すのか」を理由に書くこと
//! （理由が空ならテストが落ちる）。

use std::path::Path;
use std::process::Command;

/// 許可する（ファイル, 理由）。**いまは1つも無い。**
const ALLOWED: &[(&str, &str)] = &[];

/// メールアドレスだけを見ない（ファイル, 理由）。ユーザー名とドライブ配置は、このファイルでも見る。
const EMAIL_EXEMPT_FILES: &[(&str, &str)] = &[(
    "scripts/THIRD-PARTY-NOTICES.txt",
    "cargo-about が生成した依存の著作権表示。作者の連絡先は表示の一部で、手で直しても次の生成で戻る（ADR-0048 §4 の O-1）",
)];

/// 鳴らさないメールアドレス（綴り, 理由）。比べるときは大文字小文字を区別しない。
const EMAIL_ALLOWED_ADDRESSES: &[(&str, &str)] = &[(
    "marukei@users.noreply.github.com",
    "公開側のコミットの作者に使う GitHub の noreply のアドレス。ADR-0048 §2 が手順として名指しする。公開するアカウント名だけを含み、届く先は無い",
)];

/// 禁じる綴りの種類。
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Kind {
    UserName,
    DriveLayout,
    EmailAddress,
}

/// 局所部に使える文字か。
fn is_local_part_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '%' | '+' | '-')
}

/// ドメインに使える文字か。
fn is_domain_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-')
}

/// 1行の中の、メールアドレスの形をした綴りを（小文字にして）返す。
///
/// 全角の `＠`（U+FF20）は `@` に揃えてから読む。
/// `@` ごとに、前へ局所部の文字を、後ろへドメインの文字を伸ばす。局所部の頭の `.`、ドメインの末尾の `.` `-` は外す。
/// ドメインが `.` を1つ以上含み、空のラベルが無く、最後のラベルが2文字以上の ASCII の英字のときだけ、形と認める。
fn email_addresses_in_line(line: &str) -> Vec<String> {
    let line = line.replace('\u{FF20}', "@");
    let line = line.as_str();
    let mut found = Vec::new();
    for (at, _) in line.match_indices('@') {
        let before = &line[..at];
        let local_start = before
            .char_indices()
            .rev()
            .take_while(|(_, ch)| is_local_part_char(*ch))
            .last()
            .map_or(at, |(index, _)| index);
        let local = before[local_start..].trim_start_matches('.');
        let after = &line[at + 1..];
        let domain_len = after
            .char_indices()
            .find(|(_, ch)| !is_domain_char(*ch))
            .map_or(after.len(), |(index, _)| index);
        let domain = after[..domain_len].trim_end_matches(['.', '-']);
        if local.is_empty() || !domain.contains('.') {
            continue;
        }
        let labels: Vec<&str> = domain.split('.').collect();
        if labels.iter().any(|label| label.is_empty()) {
            continue;
        }
        let last = labels[labels.len() - 1];
        if last.len() < 2 || !last.chars().all(|ch| ch.is_ascii_alphabetic()) {
            continue;
        }
        found.push(format!("{local}@{domain}").to_ascii_lowercase());
    }
    found
}

/// 例示用のドメインか（RFC 2606 / RFC 6761）。`domain` は小文字で渡す。
fn is_example_domain(domain: &str) -> bool {
    let second_level = ["example.com", "example.net", "example.org"];
    let top_level = ["example", "test", "invalid"];
    second_level
        .iter()
        .any(|d| domain == *d || domain.ends_with(&format!(".{d}")))
        || domain
            .rsplit('.')
            .next()
            .is_some_and(|tld| top_level.contains(&tld))
}

/// 1行の中の、除外されないメールアドレスを返す。
fn reported_email_addresses(line: &str) -> Vec<String> {
    email_addresses_in_line(line)
        .into_iter()
        .filter(|address| {
            let domain = address.rsplit('@').next().unwrap_or_default();
            !is_example_domain(domain)
                && !EMAIL_ALLOWED_ADDRESSES
                    .iter()
                    .any(|(allowed, _)| allowed.eq_ignore_ascii_case(address))
        })
        .collect()
}

/// 綴りを断片から組み立てる（このファイル自身が走査に掛からないように）。
fn spelled(parts: &[&str]) -> String {
    parts.concat()
}

/// 1行から、禁じる綴りの種類を返す。
fn leaks_in_line(line: &str) -> Vec<Kind> {
    let lower = line.to_lowercase();
    let mut found = Vec::new();

    // ユーザー名: 直後が `ei`（公開するアカウント名）でない出現が1つでもあれば。
    let name = spelled(&["ma", "ruk"]);
    let account = spelled(&["ma", "ruk", "ei"]);
    let mut from = 0;
    while let Some(at) = lower[from..].find(&name) {
        let start = from + at;
        if !lower[start..].starts_with(&account) {
            found.push(Kind::UserName);
            break;
        }
        from = start + name.len();
    }

    // ドライブ配置: `\` と `/` の連なり（エスケープの `\\`・`\\\\`・混在）を `\` 1つに揃えてから見る。
    let normalized = collapse_separators(&lower);
    let layouts = [
        // Windows 形（ドライブ文字つき）
        spelled(&["f:", "\\", "ru", "st"]),
        spelled(&["f:", "\\", "ot", "her"]),
        spelled(&["d:", "\\", "ll", "m"]),
        // Git Bash 形（`/f/…`）と WSL 形（`/mnt/f/…`）。正規化すると `\f\…` になり、どちらも含む。
        spelled(&["\\f", "\\", "ru", "st"]),
        spelled(&["\\f", "\\", "ot", "her"]),
        spelled(&["\\d", "\\", "ll", "m"]),
        // Claude が作るプロジェクトのディレクトリ名
        spelled(&["f-", "-ru", "st"]),
    ];
    if layouts
        .iter()
        .any(|layout| contains_as_word(&normalized, layout))
    {
        found.push(Kind::DriveLayout);
    }

    // メールアドレス: 元の行（大文字小文字は中で畳む）から、除外されないものが1つでもあれば。
    if !reported_email_addresses(line).is_empty() {
        found.push(Kind::EmailAddress);
    }
    found
}

/// `\` と `/` の連なりを `\` 1つにする（`F:\\\\x`・`//`・混在も同じ綴りになる）。
fn collapse_separators(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_run = false;
    for ch in text.chars() {
        if ch == '\\' || ch == '/' {
            if !in_run {
                out.push('\\');
            }
            in_run = true;
        } else {
            out.push(ch);
            in_run = false;
        }
    }
    out
}

/// `needle` が `haystack` に、**直後が英数字・`_` でない形**で含まれるか。
/// 末尾（行末）に区切りが無くても当たり、別の語の一部（`…rustacean`）には当たらない。
fn contains_as_word(haystack: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(at) = haystack[from..].find(needle) {
        let end = from + at + needle.len();
        let next = haystack[end..].chars().next();
        if !next.is_some_and(|ch| ch.is_alphanumeric() || ch == '_') {
            return true;
        }
        from = end;
    }
    false
}

/// 1つのテキストから（行番号, 種類）を集める。`str::lines` は LF と CRLF の両方を1行の終わりとして読む。
fn leaks_in_text(text: &str) -> Vec<(usize, Kind)> {
    text.lines()
        .enumerate()
        .flat_map(|(index, line)| {
            leaks_in_line(line)
                .into_iter()
                .map(move |kind| (index + 1, kind))
        })
        .collect()
}

/// `root` の下の `paths`（`root` からの相対）を読み、許可されていない漏れを `ファイル:行:種類` で返す。
/// `email_exempt` に挙げたファイルでは、メールアドレスだけを数えない。
fn scan(
    root: &Path,
    paths: &[String],
    allowed: &[(&str, &str)],
    email_exempt: &[(&str, &str)],
) -> Vec<String> {
    let mut leaks = Vec::new();
    for path in paths {
        if allowed.iter().any(|(name, _)| name == path) {
            continue;
        }
        let Ok(bytes) = std::fs::read(root.join(path)) else {
            continue;
        };
        // UTF-8 として不正なバイトは置き換え文字にする（残りの ASCII の部分は見る）。
        let text = String::from_utf8_lossy(&bytes);
        let skip_email = email_exempt.iter().any(|(name, _)| name == path);
        for (line, kind) in leaks_in_text(&text) {
            if skip_email && kind == Kind::EmailAddress {
                continue;
            }
            leaks.push(format!("{path}:{line}:{kind:?}"));
        }
    }
    leaks
}

fn tracked_files(root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .expect("git が起こせること");
    assert!(output.status.success(), "git ls-files が成功すること");
    String::from_utf8(output.stdout)
        .expect("ファイル名が UTF-8 であること")
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn both_newlines(source: &str) -> [String; 2] {
    let lf = source.replace("\r\n", "\n");
    let crlf = lf.replace('\n', "\r\n");
    [lf, crlf]
}

/// 本体: 追跡されたテキストファイルに、禁じる綴りが残っていない。
#[test]
fn no_tracked_text_file_carries_a_private_spelling() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = tracked_files(root);
    // このファイル自身も必ず走査する（まだ `git add` されていない間も、自分の綴りを見張れるように）。
    let own = "tests/private_spellings.rs".to_owned();
    if !files.contains(&own) {
        files.push(own);
    }
    // 走査が空振りしていない（git の出力が空・読めない、で緑にならない）こと。
    assert!(
        files.len() > 100,
        "追跡ファイルが少なすぎる: {}",
        files.len()
    );
    assert!(files.iter().any(|f| f == "CLAUDE.md"));
    let leaks = scan(root, &files, ALLOWED, EMAIL_EXEMPT_FILES);
    assert!(
        leaks.is_empty(),
        "個人を特定しうる綴りが残っている（一般化すること。\
         測定の置き場所は `<measurements>`、リポジトリは `<repo>`、\
         テストの入力は `C:\\Users\\someone\\…` の形、\
         メールアドレスは `someone@example.com` のような例示用のドメイン）:\n{}",
        leaks.join("\n")
    );
}

/// 対照: 植えた綴りは、走査本体を通すと種類つきで見つかる（LF と CRLF の両方）。
#[test]
fn a_planted_spelling_is_found_by_the_scan_in_both_newline_styles() {
    let name = spelled(&["ma", "ruk"]);
    let cases: Vec<(String, Kind)> = vec![
        (format!("C:\\Users\\{name}\\x"), Kind::UserName),
        (format!("C:\\\\Users\\\\{name}\\\\x"), Kind::UserName),
        (format!("/home/{}/x", name.to_uppercase()), Kind::UserName),
        (
            format!("{}ei/{name}", spelled(&["ma", "ruk"])),
            Kind::UserName,
        ),
        (spelled(&["F:", "\\", "Ru", "st", "\\x"]), Kind::DriveLayout),
        (
            spelled(&["F:", "\\\\", "Ru", "st", "\\\\x"]),
            Kind::DriveLayout,
        ),
        (spelled(&["f:/", "ru", "st/x"]), Kind::DriveLayout),
        // (a) 末尾に区切りが無い
        (spelled(&["F:", "\\", "Ru", "st"]), Kind::DriveLayout),
        (spelled(&["see `F:", "\\", "Ru", "st`"]), Kind::DriveLayout),
        // (b) Git Bash 形と WSL 形
        (spelled(&["/f/", "Ru", "st/x"]), Kind::DriveLayout),
        (spelled(&["/mnt/f/", "Ru", "st/x"]), Kind::DriveLayout),
        (spelled(&["/d/", "ll", "m/x"]), Kind::DriveLayout),
        // (c) 二重エスケープ（`\` が4つ）と、連なった区切り
        (
            spelled(&["F:", "\\\\\\\\", "Ru", "st", "\\\\\\\\x"]),
            Kind::DriveLayout,
        ),
        (spelled(&["F://", "Ru", "st//x"]), Kind::DriveLayout),
        (spelled(&["F:", "\\", "Ot", "her\\x"]), Kind::DriveLayout),
        (spelled(&["D:", "\\", "LL", "M\\models"]), Kind::DriveLayout),
        (spelled(&["d:/", "ll", "m/models"]), Kind::DriveLayout),
        (
            spelled(&["~/.claude/projects/F-", "-Ru", "st-x"]),
            Kind::DriveLayout,
        ),
        // メールアドレス（O-11）。綴りは断片から組む（このファイル自身が走査に掛からないように）。
        (
            spelled(&["someone", "@", "mail.example.jp"]),
            Kind::EmailAddress,
        ),
        (
            spelled(&["<First.Last+tag", "@", "Sub.Mail-Host.CO.jp>"]),
            Kind::EmailAddress,
        ),
        (
            spelled(&["Author: x_y%z", "@", "host.dev."]),
            Kind::EmailAddress,
        ),
        (
            spelled(&["連絡先は", "a", "@", "b.cc", "まで"]),
            Kind::EmailAddress,
        ),
        // 全角の `＠`
        (
            spelled(&["someone", "\u{FF20}", "mail.example.jp"]),
            Kind::EmailAddress,
        ),
        // ユーザー情報つきの URL でも、宛先が実在しうるドメインなら当たる
        (
            spelled(&["http://user:pass", "@", "real-host.net/"]),
            Kind::EmailAddress,
        ),
        // 例示用のドメインに似ているだけのもの（`example.com` の下ではない・最上位が違う）
        (spelled(&["x", "@", "notexample.com"]), Kind::EmailAddress),
        (spelled(&["x", "@", "example.co"]), Kind::EmailAddress),
        (spelled(&["x", "@", "mail.testing"]), Kind::EmailAddress),
        // 許可した固定の綴りの近く（局所部が違う）
        (
            spelled(&["other", "@", "users.noreply.github.com"]),
            Kind::EmailAddress,
        ),
    ];
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    for (planted, kind) in &cases {
        // 漏れは行の末尾にも置く（CR が綴りの直後に来る形）。
        let body = format!("前の行\n{planted}\n次の行 {planted}");
        for (arm, text) in both_newlines(&body).iter().enumerate() {
            std::fs::write(dir.path().join("planted.md"), text).expect("書ける");
            let leaks = scan(dir.path(), &["planted.md".to_owned()], &[], &[]);
            assert_eq!(
                leaks,
                vec![
                    format!("planted.md:2:{kind:?}"),
                    format!("planted.md:3:{kind:?}")
                ],
                "腕 {arm} / 植えた綴り {planted:?}"
            );
        }
    }
}

/// 対照の裏: 正当な綴りは鳴らない。**公開するアカウント名・一般化した書き方・ふつうの文。**
#[test]
fn the_generalised_spellings_are_not_reported() {
    let account = spelled(&["ma", "ruk", "ei"]);
    let clean = [
        format!("https://github.com/{account}/personacore"),
        format!("{account}/personacore-origin"),
        r"C:\Users\someone\AppData\Local\Temp\pc1".to_owned(),
        r"C:\\Users\\someone\\x.gguf".to_owned(),
        r"<measurements>\c19-2026-09-17\RESULT.md".to_owned(),
        r"%TEMP%\pcx-<agentId>\t".to_owned(),
        r"F:\dev\measurements".to_owned(),
        r"F:\ や C:\Users\… のような書き方".to_owned(),
        "Rust の話。llm の話。".to_owned(),
        // 語の境界: 別の語の一部には当てない
        spelled(&["F:", "\\", "Ru", "stacean"]),
        spelled(&["/f/", "Ru", "stic/x"]),
        spelled(&["/mnt/f/", "ot", "herwise"]),
        // メールアドレスの形でないもの・例示用のドメイン・許可した固定の綴り（O-11）
        "vite@8.2.2 / decimal.js@10.6.0 / node@26.3.0 / aoi@1.0".to_owned(),
        "http://pw-secret@127.0.0.1:1 / http://u:p@[::1]:1".to_owned(),
        "@sveltejs/vite-plugin-svelte / `@` だけ / a@b / a@.com / a@b..com".to_owned(),
        // 見ていない形（doc の「何を見ていないか」）: 百分率符号の `%40`
        "someone%40mail.example.jp".to_owned(),
        spelled(&[
            "x",
            "@",
            "example.com y",
            "@",
            "EXAMPLE.NET z",
            "@",
            "a.example.org",
        ]),
        spelled(&[
            "http://u:p",
            "@",
            "upstream.test:1 / s3cr3t",
            "@",
            "relay.example",
        ]),
        spelled(&["x", "@", "host.invalid / x", "@", "mail.EXAMPLE"]),
        spelled(&["作者は ", "Marukei", "@", "Users.Noreply.GitHub.com"]),
    ];
    for line in &clean {
        assert_eq!(leaks_in_line(line), vec![], "誤爆: {line:?}");
    }
}

/// メールアドレスの切り出しは、形の端（局所部の頭の `.`・ドメインの末尾の `.` `-`・`@` の連なり）で正しく切る。
#[test]
fn the_email_scan_extracts_the_address_itself() {
    let line = spelled(&[
        "(.a.b",
        "@",
        "c.d.ef.) <g",
        "@",
        "h-i.jk-> l",
        "@",
        "m",
        "@",
        "n.op",
    ]);
    assert_eq!(
        email_addresses_in_line(&line),
        vec![
            spelled(&["a.b", "@", "c.d.ef"]),
            spelled(&["g", "@", "h-i.jk"]),
            spelled(&["m", "@", "n.op"]),
        ]
    );
}

/// UTF-8 として不正なファイルも、ASCII の部分を見る（O-11 のレビュー是正）。
#[test]
fn an_invalid_utf8_file_is_still_scanned() {
    let name = spelled(&["ma", "ruk"]);
    let address = spelled(&["someone", "@", "mail.example.jp"]);
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let mut bytes = b"\xff\xfe\n".to_vec();
    bytes.extend_from_slice(format!("\x01{address}").as_bytes());
    bytes.extend_from_slice(b"\x80\n");
    bytes.extend_from_slice(format!("C:\\Users\\{name}\\x").as_bytes());
    bytes.push(0xc3);
    std::fs::write(dir.path().join("broken.bin"), &bytes).expect("書ける");
    assert_eq!(
        scan(dir.path(), &["broken.bin".to_owned()], &[], &[]),
        vec![
            "broken.bin:2:EmailAddress".to_owned(),
            "broken.bin:3:UserName".to_owned(),
        ]
    );
}

/// メールアドレスだけを見ないファイルでも、ユーザー名とドライブ配置は見る。ほかのファイルではメールアドレスも見る。
#[test]
fn an_email_exempt_file_skips_only_email_addresses() {
    let name = spelled(&["ma", "ruk"]);
    let address = spelled(&["someone", "@", "mail.example.jp"]);
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let body = format!("{address}\nC:\\Users\\{name}\\x\n");
    std::fs::write(dir.path().join("notices.txt"), &body).expect("書ける");
    std::fs::write(dir.path().join("other.md"), &body).expect("書ける");
    let paths = vec!["notices.txt".to_owned(), "other.md".to_owned()];
    assert_eq!(
        scan(dir.path(), &paths, &[], &[("notices.txt", "テスト用")]),
        vec![
            "notices.txt:2:UserName".to_owned(),
            "other.md:1:EmailAddress".to_owned(),
            "other.md:2:UserName".to_owned(),
        ]
    );
}

/// メールアドレスの除外は、どれも実際に当たる（直した後に除外だけが残らないように）。
/// 生成物のファイルには除外されないメールアドレスが1つ以上あり、固定の綴りは追跡ファイルのどこかに現れる。
#[test]
fn every_email_exemption_is_still_needed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = tracked_files(root);
    for (path, reason) in EMAIL_EXEMPT_FILES {
        assert!(!reason.trim().is_empty(), "{path} の理由が空");
        assert!(files.iter().any(|f| f == path), "{path} が追跡されていない");
        let text = std::fs::read_to_string(root.join(path)).expect("読める");
        assert!(
            text.lines()
                .any(|line| !reported_email_addresses(line).is_empty()),
            "{path} にはもう除外の要るメールアドレスが無い（除外を外すこと）"
        );
    }
    for (address, reason) in EMAIL_ALLOWED_ADDRESSES {
        assert!(!reason.trim().is_empty(), "{address} の理由が空");
        assert_eq!(
            email_addresses_in_line(address),
            vec![address.to_ascii_lowercase()],
            "{address} がメールアドレスの形として読めること"
        );
        let used = files
            .iter()
            .filter(|f| *f != "tests/private_spellings.rs")
            .filter_map(|f| std::fs::read_to_string(root.join(f)).ok())
            .any(|text| {
                text.lines()
                    .flat_map(email_addresses_in_line)
                    .any(|found| found.eq_ignore_ascii_case(address))
            });
        assert!(used, "{address} はもうどこにも無い（許可を外すこと）");
    }
}

/// 許可リストは効く（許可したファイルだけ飛ばし、ほかは見つける）。
#[test]
fn an_allowed_file_is_skipped_and_nothing_else() {
    let name = spelled(&["ma", "ruk"]);
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    std::fs::write(dir.path().join("kept.md"), format!("C:\\Users\\{name}\\x")).expect("書ける");
    std::fs::write(dir.path().join("other.md"), format!("C:\\Users\\{name}\\y")).expect("書ける");
    let paths = vec!["kept.md".to_owned(), "other.md".to_owned()];
    assert_eq!(
        scan(dir.path(), &paths, &[("kept.md", "テスト用")], &[]),
        vec!["other.md:1:UserName".to_owned()]
    );
    assert_eq!(scan(dir.path(), &paths, &[], &[]).len(), 2);
}

/// 許可リストの全項目に理由がある。
#[test]
fn every_allowed_entry_states_its_reason() {
    for (path, reason) in ALLOWED {
        assert!(!reason.trim().is_empty(), "{path} の理由が空");
    }
}
