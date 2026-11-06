//! **英語の API ガイド（`docs/api/README.md`）の綴りを、仕様書と定数に突き合わせる**
//! （ADR-0047 §② P-2c の閉じる条件）。
//!
//! # 何を見るか
//!
//! | ガイドの綴り | 突き合わせる先 | 見張り番 |
//! |---|---|---|
//! | 口（`` `GET /admin/…` ``・コードブロックの `メソッド パス` の行・`curl` の行・それ以外の `/v1/…` `/admin/…` `/openapi.json`） | 仕様書の `(パス, メソッド)`。**読めない形の `curl` の行は赤** | `every_route_the_guide_names_is_in_the_document` |
//! | 例の URL の `127.0.0.1:` の後ろ | 既定のポート・`$PORT`・`${PORT}`・`<port>` | `every_example_url_uses_the_default_port_or_the_variable` |
//! | 仕様書の全部の口 | ガイドが `` `メソッド パス` `` で名指すこと | `the_guide_names_every_operation` |
//! | 「いまの API の版」の行・ヘッダ名・ヘッダの値の例 | [`API_VERSION`] / [`API_VERSION_HEADER`] | `the_guide_names_the_api_version_and_its_header` |
//! | 既定のポート・環境変数・起動の旗 | `config` / `fresh` の定数 | `the_guide_names_the_real_port_variables_and_flags` |
//!
//! **どれも植えた文面を走査本体に通す対照を持つ**（実ファイルだけを見ると、
//! 走査が壊れて「何も見つけない」ときに緑になる）。
//!
//! # ⚠ 見ていないこと
//!
//! * **欄の名前**（`base_persona` 等）は突き合わせていない。欄は `/openapi.json` を見よ、とガイドが言う形にして、
//!   ガイドに載る欄を `curl` の例に絞った。例は司令塔が実際の exe で通す（P-2c の閉じる条件）。
//! * **文章の意味**（互換の方針の翻訳が ADR-0047 と同じことを言っているか）は走査では見えない。

use std::collections::BTreeSet;

use serde_json::Value;

use super::super::config::{
    DEFAULT_PORT, ENV_CONTEXT_TOKENS, ENV_DATABASE_PATH, ENV_LLM_ENDPOINT, ENV_PORT,
    ENV_STAT_DRIFT, ENV_UPSTREAMS,
};
use super::super::fresh::{FRESH_FLAG, NO_BROWSER_FLAG, NO_UI_FLAG, UPSTREAM_FLAG, VERSION_FLAG};
use super::tests::documented_operations;
use super::{API_VERSION, API_VERSION_HEADER, STABILITY_EXTENSION, api_document};

/// 走査する実物。
const GUIDE: &str = include_str!("../../../docs/api/README.md");

/// ガイドが口を名指すときのメソッドの綴り（大文字）。
const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// 口の綴りの始まり。**この3つのどれかで始まる綴りを、口として読む。**
const ROUTE_PREFIXES: [&str; 3] = ["/v1/", "/admin/", "/openapi.json"];

fn document() -> Value {
    api_document().expect("仕様書は JSON にできること")
}

/// `\` で終わる行を次の行へ繋ぐ（シェルの継続行。**CRLF の腕も**）。
fn join_continuations(text: &str) -> String {
    text.replace("\\\r\n", " ").replace("\\\n", " ")
}

/// 口の綴りに含めてよい文字。**`?` で止める**（クエリは口ではない）。
fn route_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '{' | '}' | '.' | '*' | '$')
}

/// `line` の中の口の綴り（**書かれた順**）。文末の `.` は落とす。
fn route_tokens(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < line.len() {
        let rest = &line[at..];
        let next = ROUTE_PREFIXES
            .iter()
            .filter_map(|prefix| rest.find(prefix))
            .min();
        let Some(offset) = next else {
            break;
        };
        let start = at + offset;
        let length = line[start..]
            .find(|c: char| !route_char(c))
            .unwrap_or(line.len() - start);
        let token = line[start..start + length].trim_end_matches('.');
        found.push(token.to_owned());
        at = start + length.max(1);
    }
    found
}

/// **コードブロック（` ``` ` で囲んだ行）を落とした本文。**
///
/// ` ``` ` は `` ` `` を3つ数えるので、落とさずに `` ` `` で割ると、ブロックの中で
/// 「`` ` `` の内と外」が入れ替わる。
fn prose(text: &str) -> String {
    let mut inside = false;
    let mut kept = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        if !inside {
            kept.push(line);
        }
    }
    kept.join("\n")
}

/// ガイドが名指す口1つ。`method` が `None` なら、メソッドを言わずにパスだけを書いている。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Mention {
    method: Option<String>,
    path: String,
}

impl Mention {
    fn label(&self) -> String {
        format!("{} {}", self.method.as_deref().unwrap_or("*"), self.path)
    }
}

/// シェルの語に割る（**`'…'` と `"…"` の中の空白で割らない**。引用符は落とす）。
///
/// 閉じない引用符は `None`（読めない形）。`"…"` の中の `\` は次の1文字をそのまま取る。
fn shell_words(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                started = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        inner => current.push(inner),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => current.push(chars.next()?),
                        inner => current.push(inner),
                    }
                }
            }
            c if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                started = true;
                current.push(c);
            }
        }
    }
    if started {
        words.push(current);
    }
    Some(words)
}

/// 値を取らない `curl` の旗（**読んでよいと分かっているものだけ**）。
const CURL_SWITCHES: [&str; 12] = [
    "-s",
    "-S",
    "-i",
    "-N",
    "-v",
    "-f",
    "--silent",
    "--show-error",
    "--include",
    "--no-buffer",
    "--verbose",
    "--fail",
];

/// 束ねてよい、値を取らない短い旗の文字（`-sS` 等）。
const CURL_SWITCH_LETTERS: &str = "sSiNvf";

/// 値を1つ取るが、メソッドを変えない `curl` の旗。
const CURL_VALUE_OPTIONS: [&str; 8] = [
    "-H",
    "--header",
    "-o",
    "--output",
    "-D",
    "--dump-header",
    "-w",
    "--write-out",
];

/// **本文を付ける旗**（`curl` は、`-X` が無ければ POST で送る）。
const CURL_BODY_OPTIONS: [&str; 9] = [
    "-d",
    "--data",
    "--data-raw",
    "--data-binary",
    "--data-ascii",
    "--data-urlencode",
    "--json",
    "-F",
    "--form",
];

/// 値を取らない旗か（[`CURL_SWITCHES`] か、[`CURL_SWITCH_LETTERS`] だけを束ねた `-sS` 等）。
fn is_curl_switch(word: &str) -> bool {
    CURL_SWITCHES.contains(&word)
        || (!word.starts_with("--")
            && word.strip_prefix('-').is_some_and(|letters| {
                !letters.is_empty() && letters.chars().all(|c| CURL_SWITCH_LETTERS.contains(c))
            }))
}

/// **`curl` の1行（継続行を繋いだ後）が送るメソッドとパス。** 読めない形は `Err`（理由）。
///
/// * メソッド: `-X <m>` / `-X<m>` / `--request <m>` / `--request=<m>`。**大文字の [`METHODS`] だけ**を読む。
/// * それが無ければ、[`CURL_BODY_OPTIONS`]（`-d…` / `-F…` の値を付けた綴りと `--data…=` / `--json=` / `--form=` も）が
///   あれば `POST`、無ければ `GET`。
/// * **知らない旗（`-G` / `-I` 等、メソッドを変えうるもの）・閉じない引用符・URL が0個か2個以上・URL に口が無い**は
///   読めない形（**黙って GET と読まない**）。
fn curl_request(line: &str) -> Result<Mention, String> {
    let words = shell_words(line).ok_or("閉じない引用符")?;
    let mut rest = words
        .iter()
        .skip_while(|word| word.as_str() != "curl")
        .skip(1);
    let mut explicit: Option<String> = None;
    let mut body = false;
    let mut urls = Vec::new();
    while let Some(word) = rest.next() {
        let word = word.as_str();
        let value_of =
            |taken: Option<&String>| taken.cloned().ok_or_else(|| format!("{word} の値が無い"));
        if word == "-X" || word == "--request" {
            explicit = Some(value_of(rest.next())?);
        } else if let Some(method) = word.strip_prefix("--request=") {
            explicit = Some(method.to_owned());
        } else if let Some(method) = word.strip_prefix("-X") {
            explicit = Some(method.to_owned());
        } else if CURL_BODY_OPTIONS.contains(&word) {
            value_of(rest.next())?;
            body = true;
        } else if (word.starts_with("-d") || word.starts_with("-F")) && !word.starts_with("--")
            || ["--data", "--json=", "--form="]
                .iter()
                .any(|prefix| word.starts_with(prefix))
                && word.contains('=')
        {
            body = true;
        } else if CURL_VALUE_OPTIONS.contains(&word) {
            value_of(rest.next())?;
        } else if word.starts_with('-') && !is_curl_switch(word) {
            return Err(format!("知らない旗: {word}"));
        } else if !word.starts_with('-') {
            urls.push(word.to_owned());
        }
    }
    let [url] = urls.as_slice() else {
        return Err(format!("URL が {} 個", urls.len()));
    };
    let path = route_tokens(url)
        .into_iter()
        .next()
        .ok_or_else(|| format!("URL に口が無い: {url}"))?;
    let method = match explicit {
        Some(method) if METHODS.contains(&method.as_str()) => method,
        Some(method) => return Err(format!("読めないメソッド: {method}")),
        None if body => "POST".to_owned(),
        None => "GET".to_owned(),
    };
    Ok(Mention {
        method: Some(method),
        path,
    })
}

/// **ガイドが名指す口を全部拾う**（名指し、読めなかった `curl` の行）。
///
/// 1. 本文の `` `GET /admin/characters` `` —— `` ` `` の中の「メソッド 空白 パス」。
/// 2. **コードブロックの中の、`メソッド パス` で始まる行**（`PATCH /admin/… HTTP/1.1` 等）。
/// 3. `curl` で始まる行（継続行を繋いだ後 / [`curl_request`]）。読めなければ2つ目の戻り値へ。
/// 4. それ以外の口の綴り —— メソッドを言わない（パスだけを突き合わせる）。
///
/// # ⚠ 見ていない形
///
/// * **`curl` で始まらない行の `curl`**（本文の中の `` `curl -X …` `` 等）は3で読まず、パスだけを見る。
/// * 本文の `` ` `` の外の「メソッド パス」（`GET /admin/characters` を飾らずに書いた形）はメソッドを読まない。
fn mentions(guide: &str) -> (Vec<Mention>, Vec<String>) {
    let joined = join_continuations(guide);
    let mut found = Vec::new();
    let mut unreadable = Vec::new();

    for (index, span) in prose(&joined).split('`').enumerate() {
        if index % 2 == 0 {
            continue;
        }
        if let Some((method, path)) = span.split_once(' ')
            && METHODS.contains(&method)
            && ROUTE_PREFIXES.iter().any(|prefix| path.starts_with(prefix))
        {
            found.push(Mention {
                method: Some(method.to_owned()),
                path: path.to_owned(),
            });
        }
    }

    let mut inside = false;
    for line in joined.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            inside = !inside;
            continue;
        }
        let tokens = route_tokens(line);
        if inside {
            let mut words = trimmed.split_whitespace();
            if let (Some(method), Some(target)) = (words.next(), words.next())
                && METHODS.contains(&method)
                && let Some(path) = route_tokens(target).into_iter().next()
            {
                found.push(Mention {
                    method: Some(method.to_owned()),
                    path,
                });
            }
        }
        if trimmed.starts_with("curl ") {
            match curl_request(trimmed) {
                Ok(mention) => found.push(mention),
                Err(reason) => unreadable.push(format!("curl を読めない（{reason}）: {trimmed}")),
            }
        }
        for path in tokens {
            found.push(Mention { method: None, path });
        }
    }
    (found, unreadable)
}

/// 書かれたパスが、仕様書のパスの型に当たるか。
///
/// * 仕様書の `{…}` の段には、ガイドの**`{…}` でない段**（`aoi`・`1`・`$SESSION_ID` 等の具体の値）か、
///   **同じ綴りの `{…}`** が当たる（`{h}` は `{handle}` に当たらない）。
/// * ガイドの最後の段が `*` なら、残りの1段以上に当たる（`/admin/*`）。
fn path_matches(template: &str, written: &str) -> bool {
    let template: Vec<&str> = template.split('/').collect();
    let written: Vec<&str> = written.split('/').collect();
    for (index, segment) in written.iter().enumerate() {
        if *segment == "*" && index + 1 == written.len() {
            return template.len() > index;
        }
        let Some(expected) = template.get(index) else {
            return false;
        };
        let placeholder = expected.starts_with('{');
        let ok = if segment.starts_with('{') || !placeholder {
            segment == expected
        } else {
            !segment.is_empty()
        };
        if !ok {
            return false;
        }
    }
    written.len() == template.len()
}

/// **仕様書に無い口と、読めなかった `curl` の行**（`label()` の綴り、重複を落として並べる）。
fn unknown_mentions(guide: &str, operations: &BTreeSet<(String, String)>) -> Vec<String> {
    let (found, unreadable) = mentions(guide);
    let unknown: BTreeSet<String> = found
        .into_iter()
        .filter(|mention| {
            !operations.iter().any(|(path, method)| {
                path_matches(path, &mention.path)
                    && mention
                        .method
                        .as_ref()
                        .is_none_or(|named| named.to_ascii_lowercase() == *method)
            })
        })
        .map(|mention| mention.label())
        .chain(unreadable)
        .collect();
    unknown.into_iter().collect()
}

/// **ガイドが `` `メソッド パス` `` の形で名指していない口**（`method path`）。
fn operations_not_named(guide: &str, operations: &BTreeSet<(String, String)>) -> Vec<String> {
    let named: BTreeSet<(String, String)> = mentions(guide)
        .0
        .into_iter()
        .filter_map(|mention| {
            let method = mention.method?.to_ascii_lowercase();
            Some((mention.path, method))
        })
        .collect();
    operations
        .iter()
        .filter(|operation| !named.contains(*operation))
        .map(|(path, method)| format!("{method} {path}"))
        .collect()
}

/// 植えた仕様書の口（対照用）。
fn planted_operations() -> BTreeSet<(String, String)> {
    [
        ("/v1/models", "get"),
        ("/admin/characters", "get"),
        ("/admin/characters", "post"),
        ("/admin/characters/{handle}", "patch"),
        (
            "/admin/characters/{handle}/sessions/{session_id}/messages",
            "get",
        ),
        ("/openapi.json", "get"),
    ]
    .into_iter()
    .map(|(path, method)| (path.to_owned(), method.to_owned()))
    .collect()
}

/// **ガイドが名指す口は、全部仕様書に在る**（P-2c の閉じる条件）。
#[test]
fn every_route_the_guide_names_is_in_the_document() {
    let operations = documented_operations(&document());
    let (found, _) = mentions(GUIDE);
    let curls = GUIDE.lines().filter(|line| line.contains("curl ")).count();
    assert!(
        found.len() >= 30 && curls >= 5,
        "前提: 走査がガイドの口と curl の例を読めていること（口 {} / curl の行 {curls}）",
        found.len()
    );
    assert_eq!(unknown_mentions(GUIDE, &operations), Vec::<String>::new());
}

/// **仕様書の全部の口を、ガイドが `` `メソッド パス` `` の形で名指す**（口を足したらガイドにも1行）。
#[test]
fn the_guide_names_every_operation() {
    let operations = documented_operations(&document());
    assert!(operations.len() >= 22, "前提: 全部の口を回すこと");
    assert_eq!(
        operations_not_named(GUIDE, &operations),
        Vec::<String>::new()
    );
}

/// **対照: 口の走査が、綴り違い・メソッド違い・`curl` のメソッドの決め方・型の段を見分ける**
/// （植えた文面を走査本体に通す。**CRLF の継続行も**）。
#[test]
fn the_route_scan_reads_a_planted_guide() {
    let planted = concat!(
        "List them with `GET /admin/characters` or `GET /admin/charactrs`.\n",
        "Update with `PUT /admin/characters/{handle}` (wrong method) or ",
        "`PATCH /admin/characters/{h}` (wrong placeholder).\n",
        "Everything under `/admin/*` and `/v1/models`, then `/v1/nothing`.\n",
        "Fetch the spec from http://127.0.0.1:8080/openapi.json.\n",
        "```sh\n",
        "curl -s \"$BASE/admin/characters\" \\\r\n",
        "  -H 'Content-Type: application/json' \\\n",
        "  -d '{\"handle\":\"aoi\"}'\n",
        "curl -s -X PATCH \"$BASE/admin/characters/aoi\" -d '{}'\n",
        "curl -s -X DELETE \"$BASE/admin/characters/aoi\"\n",
        "curl -s \"$BASE/admin/characters/aoi/sessions/$SESSION_ID/messages?limit=20\"\n",
        "curl -s \"$BASE/admin/characters/aoi/sessions\"\n",
        "```\n",
    );
    let operations = planted_operations();
    assert_eq!(
        unknown_mentions(planted, &operations),
        vec![
            "* /admin/characters/aoi/sessions",
            "* /admin/characters/{h}",
            "* /admin/charactrs",
            "* /v1/nothing",
            "DELETE /admin/characters/aoi",
            "GET /admin/characters/aoi/sessions",
            "GET /admin/charactrs",
            "PATCH /admin/characters/{h}",
            "PUT /admin/characters/{handle}",
        ]
    );
    // **`-d` だけの `curl` は POST と読み、`-X` はそれに勝つ。**
    let (found, _) = mentions(planted);
    for expected in [
        ("POST", "/admin/characters"),
        ("PATCH", "/admin/characters/aoi"),
        ("GET", "/admin/characters/aoi/sessions/$SESSION_ID/messages"),
    ] {
        assert!(
            found.contains(&Mention {
                method: Some(expected.0.to_owned()),
                path: expected.1.to_owned(),
            }),
            "{expected:?} を読めていない: {found:#?}"
        );
    }
    assert!(
        found.contains(&Mention {
            method: None,
            path: "/openapi.json".to_owned(),
        }),
        "文末の `.` を落とせていない: {found:#?}"
    );
    // **名指しの網羅の対照**: 植えた文面は `GET /v1/models` と `GET /openapi.json` を
    // `` `メソッド パス` `` で書いていない（パスだけ・URL だけ）。
    assert_eq!(
        operations_not_named(planted, &operations),
        vec![
            "patch /admin/characters/{handle}",
            "get /admin/characters/{handle}/sessions/{session_id}/messages",
            "get /openapi.json",
            "get /v1/models",
        ]
    );
}

// ----------------------------------------------------------------------
// 版とヘッダ
// ----------------------------------------------------------------------

/// ガイドの「いまの API の版」の行が名乗る版。
const VERSION_MARKER: &str = "The current API version is `";

fn versions_named(guide: &str) -> Vec<&str> {
    guide
        .match_indices(VERSION_MARKER)
        .filter_map(|(at, _)| {
            let rest = &guide[at + VERSION_MARKER.len()..];
            rest.find('`').map(|end| &rest[..end])
        })
        .collect()
}

/// **`x-personacore-` で始まる綴り**（大文字小文字を問わない）のうち、定数と違うものと、
/// **ヘッダの値の例（名前の後ろに `:`）が [`API_VERSION`] でないもの**。数えた綴りの数も返す。
///
/// # ⚠ 値として読まない形
///
/// `名前=値`、表の別のセル（`| 名前 | 値 |`）、次の行に書いた値。ガイドはいま
/// どれも使っていない（値の例はコードブロックの `名前: 値` の1箇所）。
fn header_problems(guide: &str) -> (usize, Vec<String>) {
    const PREFIX: &str = "x-personacore-";
    let lower = guide.to_ascii_lowercase();
    let mut count = 0;
    let mut problems = Vec::new();
    for (at, _) in lower.match_indices(PREFIX) {
        count += 1;
        let length = guide[at..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .unwrap_or(guide.len() - at);
        let name = &guide[at..at + length];
        let lowered = name.to_ascii_lowercase();
        if lowered != API_VERSION_HEADER && lowered != STABILITY_EXTENSION {
            problems.push(format!("名前: {name}"));
            continue;
        }
        // **名前の後ろの引用符・空白を飛ばして `:` が来たら、値の例として読む**
        // （`名前: 値` / `名前:値` / `"名前": "値"` / `'名前: 値'`）。
        let quote_or_space = |c: char| c == ' ' || c == '"' || c == '\'' || c == '`';
        if lowered == API_VERSION_HEADER
            && let Some(value) = guide[at + length..]
                .trim_start_matches(quote_or_space)
                .strip_prefix(':')
        {
            let value = value.trim_start_matches(quote_or_space);
            let end = value
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
                .unwrap_or(value.len());
            let value = &value[..end];
            if value != API_VERSION {
                problems.push(format!("値: {name}: {value}"));
            }
        }
    }
    (count, problems)
}

/// **ガイドが名乗る版・ヘッダ名・ヘッダの値の例が、定数と同じ**（版の綴りの正は1箇所 / ADR-0047 §② P-2b）。
#[test]
fn the_guide_names_the_api_version_and_its_header() {
    assert_eq!(versions_named(GUIDE), vec![API_VERSION]);
    let (count, problems) = header_problems(GUIDE);
    assert!(
        count >= 3,
        "前提: ガイドがヘッダを名指していること（{count}）"
    );
    assert_eq!(problems, Vec::<String>::new());
}

/// **対照: 版の行・ヘッダ名・値の例の走査**（植えた文面）。
#[test]
fn the_version_and_header_scan_reads_a_planted_guide() {
    // **正しい値の行は [`API_VERSION`] から作る**（版を上げるたびに対照が赤くならないように /
    // 作業単位 A-10 で `1.0` → `1.1` に上げたときに直した）。
    let planted = format!(
        concat!(
            "The current API version is `7.3`.\r\n",
            "Read `X-Personacore-Api-Version`, not `X-Personacore-Version`.\n",
            "x-personacore-api-version: 0.9\n",
            "x-personacore-api-version: {version}\r\n",
            "curl -H 'x-personacore-api-version:0.8' -H 'X-Personacore-Api-Version:{version}'\n",
            "{{\"x-personacore-api-version\": \"0.7\"}}\n",
            "`x-personacore-api-version`: `0.6`\n",
            "Experimental routes carry `x-personacore-stability`.\n",
        ),
        version = API_VERSION
    );
    let planted = planted.as_str();
    assert_eq!(versions_named(planted), vec!["7.3"]);
    let (count, problems) = header_problems(planted);
    assert_eq!(count, 9);
    assert_eq!(
        problems,
        vec![
            "名前: X-Personacore-Version".to_owned(),
            "値: x-personacore-api-version: 0.9".to_owned(),
            "値: x-personacore-api-version: 0.8".to_owned(),
            "値: x-personacore-api-version: 0.7".to_owned(),
            "値: x-personacore-api-version: 0.6".to_owned(),
        ]
    );
    assert!(versions_named("no version here").is_empty());
}

// ----------------------------------------------------------------------
// 既定のポート・環境変数・起動の旗
// ----------------------------------------------------------------------

const PORT_MARKER: &str = "The default port is `";

fn ports_named(guide: &str) -> Vec<&str> {
    guide
        .match_indices(PORT_MARKER)
        .filter_map(|(at, _)| {
            let rest = &guide[at + PORT_MARKER.len()..];
            rest.find('`').map(|end| &rest[..end])
        })
        .collect()
}

/// ガイドが書いてよい環境変数（**実在する定数**）。
///
/// **`ENV_CONTEXT_TOKENS` は作業単位 A-13a-1 で足した**（ガイドの上流プロファイルの節が、
/// 面に書いた `context_tokens` がそれに勝つと書く / ADR-0054 §決定2）。
const KNOWN_VARIABLES: [&str; 6] = [
    ENV_PORT,
    ENV_DATABASE_PATH,
    ENV_UPSTREAMS,
    ENV_LLM_ENDPOINT,
    ENV_STAT_DRIFT,
    ENV_CONTEXT_TOKENS,
];

/// 起動の旗（`fresh::parse_args` が受けるもの）。
const KNOWN_FLAGS: [&str; 5] = [
    NO_UI_FLAG,
    NO_BROWSER_FLAG,
    UPSTREAM_FLAG,
    FRESH_FLAG,
    VERSION_FLAG,
];

/// **実在しない環境変数と起動の旗**（数えた数も返す）。
///
/// * 環境変数: `PERSONACORE_` で始まる綴り全部。
/// * 旗: **`curl` を含まない行**の、空白で区切った語のうち、前後の記号を落として `--<英字>` で始まるもの
///   （`curl` の `--data` 等を拾わない。`---` の区切り線は英字が続かないので拾わない）。
fn spelling_problems(guide: &str) -> (usize, usize, Vec<String>) {
    const PREFIX: &str = "PERSONACORE_";
    let mut variables = 0;
    let mut flags = 0;
    let mut problems = Vec::new();
    for (at, _) in guide.match_indices(PREFIX) {
        variables += 1;
        let length = guide[at..]
            .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .unwrap_or(guide.len() - at);
        let name = &guide[at..at + length];
        if !KNOWN_VARIABLES.contains(&name) {
            problems.push(format!("環境変数: {name}"));
        }
    }
    for line in join_continuations(guide).lines() {
        if line.contains("curl") {
            continue;
        }
        for word in line.split_whitespace() {
            let word = word.trim_matches(|c: char| "`*(),.;:\"'".contains(c));
            let Some(rest) = word.strip_prefix("--") else {
                continue;
            };
            if !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
                continue;
            }
            flags += 1;
            if !KNOWN_FLAGS.contains(&word) {
                problems.push(format!("旗: {word}"));
            }
        }
    }
    (variables, flags, problems)
}

/// **ガイドの既定のポート・環境変数・起動の旗は、実在する綴り**（写しを定数に突き合わせる）。
#[test]
fn the_guide_names_the_real_port_variables_and_flags() {
    assert_eq!(ports_named(GUIDE), vec![DEFAULT_PORT.to_string()]);
    let (variables, flags, problems) = spelling_problems(GUIDE);
    assert!(
        variables >= 3 && flags >= 2,
        "前提: ガイドが環境変数と旗を書いていること（環境変数 {variables} / 旗 {flags}）"
    );
    assert_eq!(problems, Vec::<String>::new());
}

/// **対照: ポートの行・環境変数・旗の走査**（植えた文面）。
#[test]
fn the_port_variable_and_flag_scan_reads_a_planted_guide() {
    let planted = concat!(
        "The default port is `9999`.\n",
        "Set `PERSONACORE_PORT` and `PERSONACORE_PROT`.\r\n",
        "Run `personacore --no-ui` or `personacore --headless`.\n",
        "---\n",
        "curl --data '{}' --no-buffer \"$BASE/v1/models\"\n",
    );
    assert_eq!(ports_named(planted), vec!["9999"]);
    let (variables, flags, problems) = spelling_problems(planted);
    assert_eq!((variables, flags), (2, 2));
    assert_eq!(
        problems,
        vec![
            "環境変数: PERSONACORE_PROT".to_owned(),
            "旗: --headless".to_owned(),
        ]
    );
}

// ----------------------------------------------------------------------
// curl の読み方と、例の URL のポート（P-2c のレビューの是正）
// ----------------------------------------------------------------------

/// **対照: `curl` のメソッドの名乗り方を全部読み、読めない形を読めないと言う**（走査本体に通す）。
///
/// レビューの変異で緑だった形（`--request DELETE` / `--json` / `-XPUT`）を含む。
#[test]
fn the_curl_scan_reads_every_way_of_naming_the_method() {
    let read = |line: &str| {
        curl_request(line)
            .map(|mention| format!("{} {}", mention.method.unwrap_or_default(), mention.path))
    };
    let cases = [
        (
            r#"curl -s "$BASE/admin/characters""#,
            Ok("GET /admin/characters"),
        ),
        (
            r#"curl -s -X DELETE "$BASE/admin/characters/aoi""#,
            Ok("DELETE /admin/characters/aoi"),
        ),
        (
            r#"curl -s -XPUT "$BASE/admin/characters""#,
            Ok("PUT /admin/characters"),
        ),
        (
            r#"curl -s --request DELETE "$BASE/admin/characters""#,
            Ok("DELETE /admin/characters"),
        ),
        (
            r#"curl -s --request=PATCH "$BASE/admin/characters/aoi""#,
            Ok("PATCH /admin/characters/aoi"),
        ),
        (
            r#"curl -s --json '{}' "$BASE/admin/characters""#,
            Ok("POST /admin/characters"),
        ),
        (
            r#"curl -s --json='{}' "$BASE/admin/characters""#,
            Ok("POST /admin/characters"),
        ),
        (
            r#"curl -s -d '{"a": 1}' "$BASE/admin/characters""#,
            Ok("POST /admin/characters"),
        ),
        (
            r#"curl -s -d'{}' "$BASE/admin/characters""#,
            Ok("POST /admin/characters"),
        ),
        (
            r#"curl -s --data-raw '{}' "$BASE/admin/characters""#,
            Ok("POST /admin/characters"),
        ),
        (
            r#"curl -s --data-binary=@f "$BASE/admin/characters""#,
            Ok("POST /admin/characters"),
        ),
        (
            r#"curl -s -F 'a=b' "$BASE/admin/characters""#,
            Ok("POST /admin/characters"),
        ),
        (
            r#"curl -s --form 'a=b' "$BASE/admin/characters""#,
            Ok("POST /admin/characters"),
        ),
        (
            r#"curl -sS -X PUT -d '{}' "$BASE/admin/characters/aoi/stats""#,
            Ok("PUT /admin/characters/aoi/stats"),
        ),
        (
            r#"curl -s -D - -o openapi.json "$BASE/openapi.json""#,
            Ok("GET /openapi.json"),
        ),
        (
            r#"curl -s -H 'X: -X DELETE' "$BASE/admin/characters""#,
            Ok("GET /admin/characters"),
        ),
        (
            r#"curl -s -G -d 'a=b' "$BASE/admin/characters""#,
            Err("知らない旗: -G"),
        ),
        (
            r#"curl -s -I "$BASE/admin/characters""#,
            Err("知らない旗: -I"),
        ),
        (
            r#"curl -s -X delete "$BASE/admin/characters""#,
            Err("読めないメソッド: delete"),
        ),
        (
            r#"curl -s --request "$BASE/admin/characters""#,
            Err("URL が 0 個"),
        ),
        (r#"curl -s -X"#, Err("-X の値が無い")),
        (
            r#"curl -s "$BASE/admin/a" "$BASE/admin/b""#,
            Err("URL が 2 個"),
        ),
        (r#"curl -s "$BASE/admin/characters"#, Err("閉じない引用符")),
        (
            r#"curl -s "http://example.test/""#,
            Err("URL に口が無い: http://example.test/"),
        ),
    ];
    for (line, expected) in cases {
        let expected = expected.map(str::to_owned).map_err(str::to_owned);
        assert_eq!(read(line), expected, "{line}");
    }
}

/// **対照: 読めない `curl` の行とコードブロックの `メソッド パス` の行は、ガイドの走査で赤になる**
/// （`mentions` → `unknown_mentions` を通す。**CRLF の腕も**）。
#[test]
fn unreadable_curl_lines_and_request_lines_in_code_blocks_are_reported() {
    let planted = concat!(
        "```sh\r\n",
        "curl -s -G \"$BASE/admin/characters\"\r\n",
        "curl -s -XPUT \"$BASE/admin/characters\"\n",
        "```\n",
        "```\n",
        "PATCH /admin/characters/aoi/stats HTTP/1.1\n",
        "GET /v1/models HTTP/1.1\n",
        "```\n",
        "Outside a block, PATCH /admin/characters/aoi/stats is not read as a request line.\n",
    );
    let operations = planted_operations();
    assert_eq!(
        unknown_mentions(planted, &operations),
        vec![
            "* /admin/characters/aoi/stats",
            "PATCH /admin/characters/aoi/stats",
            "PUT /admin/characters",
            "curl を読めない（知らない旗: -G）: curl -s -G \"$BASE/admin/characters\"",
        ]
    );
}

/// ガイドの `127.0.0.1:` の後ろのポートのうち、**既定のポートでも `$PORT` / `${PORT}` / `<port>` でもないもの**。
/// 数えた数も返す。
fn example_port_problems(guide: &str) -> (usize, Vec<String>) {
    const HOST: &str = "127.0.0.1:";
    let default = DEFAULT_PORT.to_string();
    let mut count = 0;
    let mut problems = Vec::new();
    for (at, _) in guide.match_indices(HOST) {
        count += 1;
        let rest = &guide[at + HOST.len()..];
        let allowed = ["$PORT", "${PORT}", "<port>"]
            .iter()
            .any(|spelled| rest.starts_with(spelled));
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let next = rest[digits.len()..].chars().next();
        let is_default = digits == default && !next.is_some_and(|c| c.is_ascii_alphanumeric());
        if !(allowed && digits.is_empty()) && !is_default {
            let shown: String = rest
                .chars()
                .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '`' | ')'))
                .collect();
            problems.push(format!("{HOST}{shown}"));
        }
    }
    (count, problems)
}

/// **ガイドの例の URL のポートは、既定のポートか `$PORT`**（レビューの変異 r4 は緑だった）。
#[test]
fn every_example_url_uses_the_default_port_or_the_variable() {
    let (count, problems) = example_port_problems(GUIDE);
    assert!(count >= 3, "前提: ガイドが URL を書いていること（{count}）");
    assert_eq!(problems, Vec::<String>::new());
}

/// **対照: 例の URL のポートの走査**（植えた文面）。
#[test]
fn the_example_port_scan_reads_a_planted_guide() {
    let default = DEFAULT_PORT;
    let other = DEFAULT_PORT + 1;
    let planted = format!(
        "curl \"http://127.0.0.1:{default}/openapi.json\"\n\
         curl \"http://127.0.0.1:{other}/openapi.json\"\n\
         BASE=\"http://127.0.0.1:$PORT\"\r\n\
         BASE=\"http://127.0.0.1:${{PORT}}\"\n\
         Base URL: `http://127.0.0.1:<port>/v1`\n\
         BASE=\"http://127.0.0.1:$PROT\"\n\
         http://127.0.0.1:{default}0/x\n"
    );
    let (count, problems) = example_port_problems(&planted);
    assert_eq!(count, 7);
    assert_eq!(
        problems,
        vec![
            format!("127.0.0.1:{other}/openapi.json"),
            "127.0.0.1:$PROT".to_owned(),
            format!("127.0.0.1:{default}0/x"),
        ]
    );
}
