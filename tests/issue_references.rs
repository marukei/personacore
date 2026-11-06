//! **GitHub が Issue の参照として自動でリンクしうる `#` ＋数字が、追跡ファイルに残っていない**ことを見張る
//! （作業単位 O-5 / Issue origin#52 / ADR-0048 §4）。
//!
//! 公開側のリポジトリでは Issue の番号が1から振り直される。いまの文面の `#31` は、公開した後に
//! **将来の無関係な Issue へ自動でリンクされる**。そこで origin の Issue を指す参照は `origin#31` の形に書く
//! （GitHub は `#N` と `owner/repo#N` を自動でリンクし、`origin#N` はどちらにも当たらない）。
//!
//! # 判定の規則（GitHub の Markdown API で実測 / 2026-10-01）
//!
//! `POST /markdown`（`mode: gfm`、`context` にこのリポジトリ）へ植えた文を送って、リンクになったものを読んだ。
//!
//! - **リンクされた**: 行頭・空白の後・`（` `、` `「` `〜` の後・漢字の直後（`作業単位#31`）・`~~` `**` の中・
//!   `/` `.` `-` `=` `:` `"` `(` の後・`owner/repo#31`。
//!
//! `owner/repo#N` は拾う（改名や移転の後に指す先が変わりうるし、origin を指すものは公開側で辿れない）。
//! ただし**公開側の `marukei/personacore#N` だけは通す**（[`PUBLIC_REPO`]）—— O-6 の後は名前が変わらず、
//! どこで描画しても（origin の中でも）公開側を指すため。決まりは ADR-0048 の追記 決定5・CLAUDE.md §4「公開の後の運用」。
//! 比較は `owner` と `repo` の完全一致（大文字小文字も区別）で、`Marukei/Personacore`・`xmarukei/personacore`・
//! `marukei/personacore.git`・`marukei/personacore-origin` などは安全側に倒して拾う。通すのはその直後の `#数字` だけで、
//! 同じ行の他の参照は拾う。
//! - **リンクされなかった**: 英数字と `_` の直後（`abc#31` / `_#31` / `origin#31`）・数字の直後に英字（`#31a`）・
//!   `&#31;`（文字参照。**セミコロンで閉じたときだけ**で、`&#31 x` はリンクされる）・コードスパンの中・`#31の`（直後が日本語）・`#666`（その番号の Issue が無い）。
//!
//! この走査は**安全側に倒す**: 前が ASCII の英数字・アンダースコアでなく（`&#数字;` の形だけは文字参照として外す）、後ろが ASCII の英数字・アンダースコアでない `#数字` を
//! すべて拾う（直後が日本語のもの、コードスパンの中のもの、いま Issue が無い番号も拾う —— 追跡ファイルは
//! Issue や PR の本文へ写されうるし、番号はいつか埋まる）。
//! Rust の属性（`#[`）・Markdown の見出し（`# `）・`#fff` のような英字で始まる色は、`#` の直後が数字でないので当たらない。
//!
//! **見ていない形**: GitHub は `GH-31` / `gh-31` もリンクする。この走査はそれを拾わない（いまリポジトリに無く、
//! Issue の参照にこの書き方を使っていない）。
//!
//! 意味が Issue の参照でないものは [`ALLOWED`] に理由つきで載せる。
use std::path::Path;
use std::process::Command;

/// 許可の1行: `(パス, 場所, 理由)`。
///
/// `場所` が `/` で終わればその下を丸ごと、`*` ならそのファイルを丸ごと許す。それ以外は**その行に現れる綴り**で、
/// 拾った `#数字` がその綴りの中にあるときだけ許す（同じファイルの他の `#数字` は許さない）。
/// どの許可も、**実際に1回以上当たること**を見る（直した後に許可だけが残らないように）。丸ごとの許可なら、
/// 当てはまるファイルが1つ以上あり、その中に拾われる `#数字` が1つ以上あること。
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "migrations/",
        "/",
        "適用済みのマイグレーションは1文字も変えない（コメントの中でも sqlx のチェックサムが変わり、既存の DB が起動しなくなる）",
    ),
    (
        "tests/issue_references.rs",
        "*",
        "この走査の定義そのもの（規則の説明と許可の綴りに `#数字` を書く）。植えた対照は実行時に組み、走査の本体に通している",
    ),
    (
        "docs/adr/0048-open-source-policy.md",
        "`#31` 等",
        "O-5 自身の定義。書き換える前の形を例として挙げている",
    ),
    (
        "docs/adr/0048-open-source-policy.md",
        "`#1` から番号",
        "公開側で振られる番号の話で、origin の Issue を指していない",
    ),
    (
        "src/domain/character.rs",
        "\"#1\"",
        "ハンドルとして拒まれる入力の例（Issue の参照ではない）",
    ),
    (
        "web/README.md",
        "`#666`",
        "CSS の色の綴り（記録として残している直書きの色）",
    ),
    (
        "web/tests/support/design-tokens.ts",
        "`#666`",
        "CSS の色の綴り（記録として残している直書きの色）",
    ),
    (
        "web/tests/design-tokens.test.ts",
        "`#666`",
        "CSS の色の綴り（記録として残している直書きの色）",
    ),
    (
        "web/tests/design-tokens.test.ts",
        "#666",
        "CSS の色の値（直書きの色を見つける走査に植えた対照）",
    ),
    (
        "web/tests/design-tokens.test.ts",
        "#12345678",
        "CSS の色の値（8桁の色を見つける走査に植えた対照）",
    ),
    (
        "web/tests/screen-wording.test.ts",
        "#666",
        "CSS の色の値（植えた対照）",
    ),
    (
        "docs/adr/0040-tailwind-for-the-screens.md",
        "`#0000`",
        "CSS の色の綴り（配布 CSS で `border-transparent` が出す完全な透明）",
    ),
];

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// 追跡ファイルに `owner/repo#N` の形で書いてよい、ただ1つのリポジトリ（公開側）。`(owner, repo)`。
///
/// O-6 の後、公開側の名前は変わらないので、この形はどこで描画されても（origin の中でも）公開側の Issue を指す。
/// 決まりは ADR-0048 の追記 決定5・CLAUDE.md §4「公開の後の運用」。比較は大文字小文字まで含めた完全一致
/// （`Marukei/Personacore` は GitHub 上では同じリポジトリへリンクするが、安全側に倒して拾い、決まった綴り1つだけを通す）。
const PUBLIC_REPO: (&str, &str) = ("marukei", "personacore");

/// `before` が `owner/repo` で終わるなら、その `(owner, repo)` を返す。
/// `https://…/a/b#12` のように `/` が2つ以上あるものは URL として外す（`None`）。
fn owner_repo(before: &str) -> Option<(&str, &str)> {
    let len: usize = before
        .chars()
        .rev()
        .take_while(|c| is_word(*c) || matches!(c, '.' | '-' | '/'))
        .map(char::len_utf8)
        .sum();
    let tail = &before[before.len() - len..];
    let mut parts = tail.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(repo), None) if !owner.is_empty() && !repo.is_empty() => {
            Some((owner, repo))
        }
        _ => None,
    }
}

/// `before` が、拾うべき `owner/repo` で終わるか。
///
/// `owner/repo#N` も GitHub はリンクする。公開側の [`PUBLIC_REPO`] 以外は拾う —— 改名や移転の後に
/// どのリポジトリを指すかが変わりうるし、origin（`marukei/personacore-origin#N`）を指す参照は公開側では辿れない。
/// 公開側の `marukei/personacore#N` だけは、名前が変わらずどこで描画しても公開側を指すので通す。
fn is_owner_repo(before: &str) -> bool {
    owner_repo(before).is_some()
}

/// `before` が、ちょうど公開側の `marukei/personacore` で終わるか（その直後の `#数字` は拾わない）。
fn is_public_repo(before: &str) -> bool {
    owner_repo(before) == Some(PUBLIC_REPO)
}

/// 1行の中の、リンクされうる `#数字` を `(バイトの開始, バイトの終わり)` で返す。
fn references_in_line(line: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut prev: Option<char> = None;
    let mut chars = line.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let opens = (!prev.is_some_and(is_word) || is_owner_repo(&line[..i]))
            && !is_public_repo(&line[..i]);
        if c == '#' && opens {
            let mut end = i + 1;
            while let Some(&(j, d)) = chars.peek() {
                if d.is_ascii_digit() {
                    end = j + 1;
                    chars.next();
                } else {
                    break;
                }
            }
            let has_digits = end > i + 1;
            let next = line[end..].chars().next();
            let bounded = !next.is_some_and(is_word);
            // `&#31;` は文字参照（リンクされない）。`&#31 x` のようにセミコロンで閉じないものはリンクされる。
            let char_ref = prev == Some('&') && next == Some(';');
            if has_digits && bounded && !char_ref {
                found.push((i, end));
            }
            prev = line[..end].chars().next_back();
            continue;
        }
        prev = Some(c);
    }
    found
}

fn allowed_by_spelling(line: &str, start: usize, end: usize, spelling: &str) -> bool {
    line.match_indices(spelling)
        .any(|(at, s)| at <= start && end <= at + s.len())
}

/// 走査の本体。`(path, text)` の列を読み、許可されていない参照を `path:行:綴り` で返す。
/// 2つ目の戻り値は、許可の行ごとに当たった回数（`ALLOWED` と同じ並び。丸ごとの許可は、その下で拾われた数）。
fn scan<'a>(
    files: impl IntoIterator<Item = (&'a str, &'a str)>,
    allowed: &[(&str, &str, &str)],
) -> (Vec<String>, Vec<usize>) {
    let mut found = Vec::new();
    let mut hits = vec![0; allowed.len()];
    for (path, text) in files {
        let whole = allowed.iter().position(|(p, place, _)| {
            (*place == "/" && path.starts_with(p)) || (*place == "*" && path == *p)
        });
        if let Some(k) = whole {
            hits[k] += text
                .lines()
                .map(|l| references_in_line(l).len())
                .sum::<usize>();
            continue;
        }
        for (n, line) in text.lines().enumerate() {
            for (start, end) in references_in_line(line) {
                let mut ok = false;
                for (k, (p, place, _)) in allowed.iter().enumerate() {
                    if *p == path
                        && *place != "/"
                        && *place != "*"
                        && allowed_by_spelling(line, start, end, place)
                    {
                        hits[k] += 1;
                        ok = true;
                    }
                }
                if !ok {
                    found.push(format!("{path}:{}:{}", n + 1, &line[start..end]));
                }
            }
        }
    }
    (found, hits)
}

/// 1回も当たらなかった許可（丸ごとの許可も含む）。
fn stale_entries(allowed: &[(&str, &str, &str)], hits: &[usize]) -> Vec<String> {
    allowed
        .iter()
        .zip(hits)
        .filter(|(_, n)| **n == 0)
        .map(|((p, place, _), _)| format!("{p}: {place}"))
        .collect()
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

/// `#` と番号を分けて書いた綴り（このファイル自身が走査に当たらないように）。
fn issue(n: u32) -> String {
    format!("{}{n}", '#')
}

/// 本体: 追跡されたテキストファイルに、リンクされうる Issue の参照が残っていない。
#[test]
fn no_tracked_text_file_carries_a_bare_issue_reference() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths = tracked_files(root);
    let own = "tests/issue_references.rs".to_owned();
    if !paths.contains(&own) {
        paths.push(own);
    }
    assert!(
        paths.len() > 100,
        "追跡ファイルが少なすぎる: {}",
        paths.len()
    );
    let texts: Vec<(String, String)> = paths
        .into_iter()
        .filter_map(|p| {
            let bytes = std::fs::read(root.join(&p)).ok()?;
            let text = String::from_utf8(bytes).ok()?;
            Some((p, text))
        })
        .collect();
    assert!(texts.iter().any(|(p, _)| p == "docs/06-roadmap.md"));
    let (found, hits) = scan(texts.iter().map(|(p, t)| (p.as_str(), t.as_str())), ALLOWED);
    assert!(
        found.is_empty(),
        "GitHub が Issue の参照としてリンクしうる綴りが残っている（origin の Issue なら `origin#N` と書く）:\n{}",
        found.join("\n")
    );
    let stale = stale_entries(ALLOWED, &hits);
    assert!(
        stale.is_empty(),
        "もう当たらない許可が残っている:\n{}",
        stale.join("\n")
    );
}

/// 植えた対照: リンクされる形は赤、リンクされない形は緑。LF / CRLF の両方で、走査の本体に通す。
#[test]
fn planted_references_are_found_and_look_alikes_are_not_in_both_newline_styles() {
    let n = issue(31);
    let red = [
        format!("{n} 行頭"),
        format!("空白の後 {n} です"),
        format!("括弧（{n}）"),
        format!("読点、{n}"),
        format!("鉤括弧「{n}」"),
        format!("作業単位{n}"),
        format!("範囲 {}〜{}", issue(31), issue(33)),
        format!("~~{n}~~ と **{n}**"),
        format!("marukei/personacore-origin{n}"),
        format!("example/personacore{n}"),
        format!("xmarukei/personacore{n}"),
        format!("foo.marukei/personacore{n}"),
        format!("Marukei/Personacore{n}"),
        format!("marukei/Personacore{n}"),
        format!("marukei/personacore.git{n}"),
        format!("example/other{n}"),
        format!("（marukei/personacorex{n}）"),
        format!("行末の {n}"),
        format!("閉じない文字参照 &{n} x"),
    ];
    for line in &red {
        let source = format!("前の行\n{line}\n後の行\n");
        for text in both_newlines(&source) {
            let (found, _) = scan([("x.md", text.as_str())], &[]);
            assert!(!found.is_empty(), "拾うべき綴りを見逃した: {line:?}");
            assert!(
                found.iter().all(|f| f.starts_with("x.md:2:")),
                "行番号が合わない: {found:?}"
            );
        }
    }
    let range = format!("範囲 {}〜{}\n", issue(31), issue(33));
    for text in both_newlines(&range) {
        let (found, _) = scan([("x.md", text.as_str())], &[]);
        assert_eq!(
            found,
            vec![
                format!("x.md:1:{}", issue(31)),
                format!("x.md:1:{}", issue(33))
            ]
        );
    }
    let green = [
        format!("marukei/personacore{n}"),
        format!("（marukei/personacore{n}）"),
        format!("行頭でなく、marukei/personacore{n} の形"),
        format!("`marukei/personacore{n}`"),
        "#[derive(Debug)]".to_owned(),
        "#![allow(dead_code)]".to_owned(),
        "# 見出し".to_owned(),
        "### 見出し3".to_owned(),
        "color: #fff; #1a2b3c;".to_owned(),
        format!("origin{n} の形"),
        format!("abc{n}"),
        format!("&{n};"),
        format!("{n}a"),
        "# 1 はコメント".to_owned(),
        format!("https://example.com/a/b{n}"),
        "## ".to_owned(),
    ];
    for line in &green {
        let source = format!("{line}\n");
        for text in both_newlines(&source) {
            let (found, _) = scan([("x.rs", text.as_str())], &[]);
            assert!(
                found.is_empty(),
                "拾ってはいけない綴りを拾った: {line:?} → {found:?}"
            );
        }
    }
    // 公開側の綴りが通すのは、その直後の `#数字` だけ。同じ行の素の参照は拾う。
    let mixed = format!(
        "公開側 marukei/personacore{} と素の {}\n",
        issue(31),
        issue(33)
    );
    for text in both_newlines(&mixed) {
        let (found, _) = scan([("x.md", text.as_str())], &[]);
        assert_eq!(found, vec![format!("x.md:1:{}", issue(33))]);
    }
}

/// 許可は綴りの中の参照だけを許し、同じ行の他の参照は許さない。丸ごとの許可はそのパスの下だけ。
#[test]
fn an_allowed_spelling_covers_only_itself() {
    let text = format!("色 `{}` と {}\n", issue(666), issue(31));
    let allowed = [("a.md", "`#666`", "色")];
    let (found, hits) = scan([("a.md", text.as_str())], &allowed);
    assert_eq!(found, vec![format!("a.md:1:{}", issue(31))]);
    assert_eq!(hits, vec![1]);
    let (found, _) = scan([("b.md", text.as_str())], &allowed);
    assert_eq!(found.len(), 2, "別のファイルには効かない: {found:?}");

    // 当たらなかった許可は「古い」と言われる。丸ごとの許可も同じ —— 当てはまるファイルが無いもの（`docs/api/`）、
    // 当てはまるファイルはあるが中に `#数字` が無いもの（`n.txt`）は古く、中で拾われたもの（`m/`）は古くない。
    let quiet = "番号の無い文\n".to_owned();
    let mixed = [
        ("a.md", "`#666`", "色"),
        ("a.md", "`#777`", "もう無い色"),
        ("docs/api/", "/", "並行"),
        ("m/", "/", "適用済み"),
        ("n.txt", "*", "第三者"),
    ];
    let (found, hits) = scan(
        [
            ("a.md", text.as_str()),
            ("m/x.sql", text.as_str()),
            ("n.txt", quiet.as_str()),
        ],
        &mixed,
    );
    assert_eq!(found, vec![format!("a.md:1:{}", issue(31))]);
    assert_eq!(hits, vec![1, 0, 0, 2, 0]);
    assert_eq!(
        stale_entries(&mixed, &hits),
        vec![
            "a.md: `#777`".to_owned(),
            "docs/api/: /".to_owned(),
            "n.txt: *".to_owned()
        ]
    );

    let whole = [("migrations/", "/", "適用済み")];
    let (found, _) = scan([("migrations/0001.sql", text.as_str())], &whole);
    assert!(found.is_empty());
    let (found, _) = scan([("src/migrations.rs", text.as_str())], &whole);
    assert_eq!(found.len(), 2);
}

#[test]
fn every_allowed_entry_states_its_reason() {
    for (path, place, reason) in ALLOWED {
        assert!(!reason.trim().is_empty(), "{path}: {place} に理由が無い");
    }
}
