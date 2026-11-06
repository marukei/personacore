//! **UI（Svelte のビルド成果物）の配布**（ADR-0031 / 作業単位 D-0）。
//!
//! **F-4a で配り方が2通りになった**（[ADR-0041](../../docs/adr/0041-one-executable-per-os.md) §決定2）
//! ——**既定は実行ファイルへの埋め込み**で、`PERSONACORE_WEB_DIR` を立てたときだけディスク。
//! **P-1 で「配らない」が足された**（`--no-ui` / [ADR-0047](../../docs/adr/0047-the-api-is-the-product.md) §①）
//! ——**`PERSONACORE_WEB_DIR` が立っていても配らない**（[`UiFiles::Withheld`]）。
//!
//! ```text
//! 未知のパス
//!   ↓ percent デコード（1回だけ）→ 空 / `.` セグメントを飛ばす
//!   ├ 最初のセグメントが v1 / admin → ApiError::not_found()（OpenAI 互換のエラー形）
//!   └ それ以外 → UiFiles
//!                 ├ --no-ui                    → 本文の無い 404（何も配らない）
//!                 ├ PERSONACORE_WEB_DIR 未設定 → 埋め込み（include_dir）
//!                 └ 設定あり                   → ServeDir（そのディレクトリ）
//! ```
//!
//! **⚠ 分かれるのは [`is_api_path`] を通ったあと。** 3つの腕は
//! **同じ判定の下流に並んでいる**ので、片方だけ API を飲み込む形は書けない
//! （`super::unknown_route` の1箇所でしか分岐しない）。
//!
//! # ⚠ **UI のフォールバックが API の 404 を飲み込まないこと**（ADR-0031 §4）
//!
//! `/v1/*` と `/admin/*` の未知パスは、**いまも OpenAI 形式のエラーを返す。**
//! ここを取り違えると、綴りを間違えた API 呼び出しに **`index.html` が 200 で返る。**
//! クライアントは JSON を期待しているので「壊れた JSON が返ってきた」としか分からず、
//! **原因（パスの綴り間違い）から最も遠い形で現れる。**
//!
//! **UI へ落ちるのは、その2つに属さないパスだけ。**
//! 見張り番は `tests/server_web_ui.rs::the_ui_fallback_never_swallows_an_api_error`
//! （**`/v1` / `/admin` / どちらでもないパスの3つを1本の中で対比している**）。
//!
//! # ⚠ **判定は生パスに対して行わない**（[`is_api_path`] の doc）
//!
//! **`Request::uri().path()` は percent エンコードのまま・未正規化**なので、
//! `starts_with` で見ると **`//v1/…` / `/./v1/…` / `/%76%31/…` が全部 UI へ落ちる。**
//! **`//v1/…` は珍しい形ではない**——base URL が `/` で終わるクライアントが
//! `base + "/v1/chat/completions"` を組むと必ずこうなる。
//! 見張り番は `tests/server_web_ui.rs::an_unnormalised_api_path_is_still_an_api_path` と
//! `a_doubled_slash_before_the_chat_endpoint_answers_in_the_openai_shape`
//! （**`reqwest` は `.` / `..` / `%2e` を畳むので、
//! そこは生の要求行を書く方の検査でしか観測できない**——
//! 畳む綴りと畳まない綴りの一覧は
//! `the_url_crate_folds_dot_segments_but_not_empty_ones` が実測して固定している）。
//!
//! # `..` から始まるパスをどちらに寄せたか
//!
//! **`/../v1/x` は API と読まない**（＝ UI へ落ち、`ServeDir` の本文の無い 404 になる）。
//! **`.` と空セグメントは飛ばすのに `..` は飛ばさない**のは、
//! **`..` だけが「上へ登る」という意味を持つ**ため——飛ばすと、
//! `is_api_path` が `ServeDir` の解釈（`..` は拒否する）と食い違う。
//! **現実のクライアントが作る形ではない**ので、食い違いを作らない方を採った。
//! **URL ライブラリを使うクライアントは `/../v1/x` を線に乗せない**
//! （`url` クレートが `/v1/x` に畳む。実測は
//! `the_url_crate_folds_dot_segments_but_not_empty_ones`）。
//! **承知のうえで選んだ振る舞い**であり、`is_api_path` の
//! `a_leading_dot_dot_segment_is_not_an_api_path` が固定している。
//!
//! # SPA のフォールバック（`index.html` を常に返す）を**入れていない**
//!
//! D-0 は1画面で、**ルータを依存として足していない**（ADR-0031 / 作業単位 D-0）ので、
//! 深いリンクが存在しない。入れる必要が出るのは画面を増やしてルータを決めるときで、
//! **そのときにここへ足す。**
//!
//! **入れないことには、いま効いている利点がある**——
//! 「UI に無いパス」は素の 404 のままなので、
//! **上の見張り番が「API のエラー形」と「UI 側の 404」を区別できる。**
//! 常に `index.html` を返す形にすると **UI 側は全部 200** になり、
//! **フォールバックの順序を入れ替えた変異が「200 と index.html」で緑を通りうる。**
//! 足すときは、その区別をどう保つかを一緒に決めること。
//!
//! # ⚠ **`ServeDir` はリンクを辿る**（配り先には信頼できるディレクトリだけを指すこと）
//!
//! **`..` と絶対パスは弾く**（D-0 のレビューで 15 通り試して全部阻止された。
//! 二重エンコードの `%252e%252e%252f` も、[`is_api_path`] が1回しか解かないので
//! UI 側へ落ち、そこで 404 になる）。
//!
//! **一方、リンクは辿る。** 配り先の中に**ジャンクション**（Windows の `mklink /J`。
//! **管理者権限が要らない**）を1つ置くと、**その先のルート外の内容がそのまま配られる**
//! ——実測済み。
//!
//! **これは `ServeDir` の仕様であって、ここで塞ぐものではない**（塞ぐと
//! 正当な配置——`web/dist` をシンボリックリンクで別の場所に置く——まで壊れる）。
//! **`PERSONACORE_WEB_DIR` は信頼できるディレクトリだけを指すこと。**
//! 外部から書き込める場所を配り先にしない。

use std::path::{Path, PathBuf};

use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use tower_http::services::ServeDir;

use super::ApiError;

/// **API の前置き（最初のセグメント）。この下の未知パスは UI へ落とさない**（ADR-0031 §4）。
///
/// **`router` が生やしているパスの前置きと一致していること**は
/// `every_api_route_is_recognised_as_an_api_path` が見張る
/// （`router` に第3の前置きを足したら、そこだけ UI に飲み込まれる）。
const API_SEGMENTS: [&[u8]; 2] = [b"v1", b"admin"];

/// そのパスが **API のもの**か。**純粋関数。**
///
/// # 生パスを `starts_with` で見ないこと
///
/// ここへ来るのは **`Request::uri().path()`**、つまり
/// **percent エンコードされたままの、正規化されていない綴り**である。
/// `starts_with("/v1")` だけで判定すると、**次が全部 UI へ落ちる**——
///
/// | 綴り | どう作られるか |
/// |---|---|
/// | `//v1/chat/completions` | **base URL が `/` で終わるクライアント**（`base + "/v1/…"`） |
/// | `/./v1/…` | パスを繋ぐ実装によっては普通に出る |
/// | `/%76%31/…` | 手書き・中継・走査 |
///
/// **いちばん実害が出るのは1行目。** `ServeDir` は GET / HEAD しか受けないので
/// `POST //v1/chat/completions` が**本文の無い 405** になり、
/// クライアントには **`Unexpected end of JSON input`** としか出ない
/// （モジュール doc の「原因から最も遠い形で現れる」そのもの）。
///
/// # 分類の前に正規化する
///
/// 1. **percent デコードを1回だけ行う。** **繰り返さない**——
///    二重エンコード（`%252e%252e%252f`）まで解くと受理集合が広がりすぎる。
///    **1回しか解かないので、二重エンコードは UI 側（`ServeDir`）へ落ち、
///    あちらが 404 で弾く**（実測済み）。
/// 2. `/` で分割し、**空セグメントと `.` セグメントを飛ばす。**
/// 3. **残った最初のセグメント**が [`API_SEGMENTS`] のどれかなら API。
///
/// **`..` は飛ばさない。** `/../v1/x` は API ではなく UI へ落ちる
/// （＝ `ServeDir` の本文の無い 404）。理由はモジュール doc の
/// 「`..` から始まるパスをどちらに寄せたか」。見張り番は
/// `a_leading_dot_dot_segment_is_not_an_api_path`。
///
/// **`/v1x` は API ではない**——セグメントが丸ごと一致しなければ別のパスである
/// （`/version` のような UI のパスが**永久に 404 になる**のを防ぐ）。
pub(super) fn is_api_path(path: &str) -> bool {
    let decoded = percent_decode_once(path);
    decoded
        .split(|byte| *byte == b'/')
        .find(|segment| !segment.is_empty() && *segment != b".")
        .is_some_and(|segment| API_SEGMENTS.contains(&segment))
}

/// **percent エンコードを1回だけ解く。** **純粋関数。**
///
/// 解けない `%` はそのまま通す（`%zz` / 末尾の `%`）。
/// **出力を走査し直さない**ので、`%2576` は `%76` までしか戻らない。
///
/// バイト列のまま返す理由: 解いた結果が UTF-8 とは限らないため。
/// **判定に使うのは ASCII のセグメントだけ**なので、文字列にする必要が無い。
fn percent_decode_once(path: &str) -> Vec<u8> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && at + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex_value(bytes[at + 1]), hex_value(bytes[at + 2]))
        {
            decoded.push(high * 16 + low);
            at += 3;
            continue;
        }
        decoded.push(bytes[at]);
        at += 1;
    }
    decoded
}

/// 16 進数字1桁の値。数字でなければ `None`。
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

// ----------------------------------------------------------------------
// 埋め込み（ADR-0041 §決定2 / 作業単位 F-4a）
// ----------------------------------------------------------------------

/// **実行ファイルに埋め込んだ画面。**
///
/// **綴りは [`super::DEFAULT_WEB_DIR`] と同じでなければならない**——
/// 見張り番は `the_embedded_ui_comes_from_the_default_web_dir`
/// （**このファイルを読んで、マクロに書いた綴りを定数と突き合わせる**）。
/// **マクロは定数を受け取れない**ので、写しを消す代わりに走査で押さえてある。
///
/// **`web/dist` が無くてもビルドは通る**——`build.rs` が空で作る。
/// **そのときは画面が0件になり、起動行がそう言う。**
static EMBEDDED_UI: include_dir::Dir<'static> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/web/dist");

/// 埋め込みを「**名前 → バイト列**」の平らな表にしたもの。
///
/// **表にしてから引く**のは、判定を**引数で表を受け取る純粋関数**に切り出すため
/// （CLAUDE.md `15b`「対照は植えたソースを走査本体に通す」）。
/// **埋め込みの中身はコンパイル時に決まるのでテストから植えられない**が、
/// **表を引数にすれば、植えた表と実物を同じ本体に通せる。**
static EMBEDDED_TABLE: std::sync::OnceLock<Vec<(&'static str, &'static [u8])>> =
    std::sync::OnceLock::new();

/// **UI の入れ子をどこまで降りるか**（埋め込みもディスクも同じ）。
///
/// # ⚠ 2つの腕で必ず同じ値を使うこと（**レビュー指摘2 / 2026-09-18 に実測**）
///
/// **初版は埋め込みだけ `files()` ＋ `dirs().flat_map(files)` の2段止まり**で、
/// **ディスク側は 8 段**だった。**現れ方がいちばん悪い形になる**——
/// **`PERSONACORE_WEB_DIR` を立てた開発中は 3 段目が出るのに、配布物では 404。**
/// 実測: `web/dist/assets/fonts/deep.txt` が埋め込みでは引けず、`ui_files` も 3 対 4 で食い違った。
///
/// **`web/dist` はいま 2 段**（`index.html` / `assets/*`）なので、8 段は十分に余裕がある。
const MAX_UI_DEPTH: usize = 8;

/// 埋め込みを平らな表に畳む。**再帰。深さは [`MAX_UI_DEPTH`] で止める。**
///
/// **`Dir` を引数で受け取る**ので、**テストが植てた `Dir` を同じ本体に通せる**
/// （CLAUDE.md `15b`。埋め込みの中身はコンパイル時に決まるので、これしか測る道が無い）。
fn flatten_embedded(
    dir: &'static include_dir::Dir<'static>,
    depth: usize,
    out: &mut Vec<(&'static str, &'static [u8])>,
) {
    if depth >= MAX_UI_DEPTH {
        return;
    }
    for entry in dir.entries() {
        match entry {
            include_dir::DirEntry::File(file) => {
                if let Some(name) = file.path().to_str() {
                    out.push((name, file.contents()));
                }
            }
            include_dir::DirEntry::Dir(child) => flatten_embedded(child, depth + 1, out),
        }
    }
}

/// 埋め込みの表。**名前は `/` 区切りの相対パス**（`index.html` / `assets/app.js`）。
fn embedded_table() -> &'static [(&'static str, &'static [u8])] {
    EMBEDDED_TABLE.get_or_init(|| {
        let mut files = Vec::new();
        flatten_embedded(&EMBEDDED_UI, 0, &mut files);
        // **Windows の `\` を `/` へ寄せない**——`include_dir` は `/` で持つ。
        // ここでは並びだけを決める（起動ログの件数を安定させる）。
        files.sort_unstable_by_key(|(name, _)| *name);
        files
    })
}

/// ディレクトリを指す要求に返すファイル名（`ServeDir` の既定と同じ）。
const DIRECTORY_INDEX: &str = "index.html";

/// 拡張子から Content-Type を引く表。**引けない拡張子は [`FALLBACK_CONTENT_TYPE`]。**
///
/// **止めない**（ADR-0041 §決定2）——画面のビルドが新しい種類のファイルを出したとき、
/// **配布が壊れるより「表示が変」のほうが原因に近い。**
const CONTENT_TYPES: [(&str, &str); 16] = [
    ("html", "text/html; charset=utf-8"),
    ("js", "text/javascript; charset=utf-8"),
    ("mjs", "text/javascript; charset=utf-8"),
    ("css", "text/css; charset=utf-8"),
    ("json", "application/json"),
    ("map", "application/json"),
    ("svg", "image/svg+xml"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("ico", "image/x-icon"),
    ("woff2", "font/woff2"),
    ("woff", "font/woff"),
    ("txt", "text/plain; charset=utf-8"),
];

/// 表に無い拡張子の Content-Type。
const FALLBACK_CONTENT_TYPE: &str = "application/octet-stream";

/// 名前から Content-Type を決める。**純粋関数。**
///
/// **拡張子は大文字小文字を区別しない**（`INDEX.HTML` も HTML）。
fn content_type(name: &str) -> &'static str {
    let extension = match name.rsplit_once('.') {
        Some((_, extension)) => extension.to_ascii_lowercase(),
        None => return FALLBACK_CONTENT_TYPE,
    };
    CONTENT_TYPES
        .iter()
        .find(|(suffix, _)| *suffix == extension)
        .map_or(FALLBACK_CONTENT_TYPE, |(_, kind)| *kind)
}

/// 要求のパスを、**埋め込みの表を引く名前**に直す。**純粋関数。**
///
/// 1. **percent デコードを1回だけ**行う（[`percent_decode_once`] と同じ作法）。
///    解いた結果が UTF-8 でなければ `None`（埋め込みの名前は UTF-8 しかない）。
/// 2. `/` で分割し、**空セグメントと `.` を飛ばす。**
/// 3. **`..` が1つでも在れば `None`。** `ServeDir` と同じ解釈
///    （モジュール doc の「`..` から始まるパスをどちらに寄せたか」）。
///    **飛ばして上へ登らせない**——飛ばすと `/assets/../../secret` が
///    `assets/secret` として引けてしまう。
/// 4. **何も残らなければ [`DIRECTORY_INDEX`]**（`/` に `index.html` を返す）。
fn embedded_name(path: &str) -> Option<String> {
    let decoded = String::from_utf8(percent_decode_once(path)).ok()?;
    let mut segments = Vec::new();
    for segment in decoded.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return None;
        }
        segments.push(segment);
    }
    if segments.is_empty() {
        return Some(DIRECTORY_INDEX.to_owned());
    }
    Some(segments.join("/"))
}

/// 埋め込みの表から、要求のパスに当たるものを引く。**純粋関数。**
///
/// **表を引数で受け取る**ので、実物の表と植えた表を同じ本体に通せる
/// （CLAUDE.md `15b`）。見つからなければ `None`＝**本文の無い 404**。
fn lookup_embedded<'a>(
    files: &'a [(&'a str, &'a [u8])],
    path: &str,
) -> Option<(&'a str, &'a [u8])> {
    let name = embedded_name(path)?;
    files
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .copied()
}

/// ディスクの配り先に在るファイルの数（**上限つきの走査**）。
///
/// # なぜ上限を付けるか
///
/// **止まらなくなるのを防ぐのは、深さと件数の天井のほう**
/// （[`MAX_UI_DEPTH`] と `MAX_FILES`）。**輪に入っても、そこで必ず止まる。**
/// 天井に当たった数はそこで止まるので、
/// **起動行の件数は「下限」である**——`0` か否かだけが意味を持つ。
///
/// # リンクを辿らないのは、正しさの話ではなく数え方の話
///
/// > **⚠ 初版は「辿るとジャンクションの輪で起動が返らなくなる」と書いていた。言い過ぎだった**
/// > （**レビュアーが実測** / 2026-09-18）。**`symlink_metadata` を `metadata` にして
/// > リンクを辿る変異は 2096 全緑**——**実害が [`MAX_UI_DEPTH`] にマスクされる**からで、
/// > **doc が「守った」と書いている場所に、守る仕掛けが無かった。**
///
/// **辿らないのは、`ServeDir` の配り先の外にあるものを「配れる件数」に数えたくない**から。
/// **`ServeDir` は辿る**（モジュール doc）ので、**この数と、実際に配れるものは一致しない。**
/// **数える側の作法であって、安全の仕掛けではない。**
fn count_disk_files(root: &Path) -> usize {
    /// 数える件数の上限。
    const MAX_FILES: usize = 10_000;

    let mut found = 0usize;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if found >= MAX_FILES {
                return found;
            }
            // **リンクは辿らない**（`symlink_metadata` はリンク自身を見る）。
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            if meta.is_file() {
                found += 1;
            } else if meta.is_dir() && depth + 1 < MAX_UI_DEPTH {
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    found
}

/// UI をどこから配るか。
///
/// **未設定なら埋め込み、[`super::ENV_WEB_DIR`] が設定されていればディスク**
/// （ADR-0041 §決定2）。**`/v1` / `/admin` の判定は、どちらの腕も
/// [`is_api_path`] を通ったあとに来る**（`super::unknown_route`）。
#[derive(Clone, Debug)]
pub(super) enum UiFiles {
    /// **実行ファイルに埋め込んだ画面**（既定）。
    Embedded {
        /// 名前 → バイト列の表。
        files: &'static [(&'static str, &'static [u8])],
    },
    /// **ディスクから配る**（開発用 / [`super::ENV_WEB_DIR`]）。
    ///
    /// **設定された配り先そのものを別に持つ**: `ServeDir` は内部で `.` を
    /// 前置きした写しを持つので、そこから取り出すと
    /// **起動ログに出る綴りが設定した綴りと変わる。**
    Disk {
        /// **設定された配り先そのもの**（ログと件数の確認に使う）。
        root: PathBuf,
        files: ServeDir,
    },
    /// **配らない**（`--no-ui` / ADR-0047 §① / 作業単位 P-1）。
    ///
    /// **画面のどのパスにも本文の無い 404**（[`Self::respond`]）。
    /// **`/v1` / `/admin` はここへ来ない**（[`is_api_path`] が先に取る）ので、API の応答は変わらない。
    /// **[`Self::new`] からは作らない**——`PERSONACORE_WEB_DIR` の有無とは別の決定で、
    /// 決めるのは `bind_with`（`--no-ui` の旗から来る `super::Ui`）。
    Withheld,
}

/// 起動ログの `ui_source` の綴り（埋め込み）。
pub(super) const UI_SOURCE_EMBEDDED: &str = "embedded";

/// 起動ログの `ui_source` の綴り（ディスク）。
pub(super) const UI_SOURCE_DISK: &str = "disk";

/// 起動ログの `ui_source` の綴り（**配らない** / `--no-ui`）。
///
/// **集計はこの綴りで引く**（「画面なしの起動が何回あったか」）。
/// 見張り番は `the_withheld_ui_names_itself_and_never_warns_that_it_is_empty`（綴りを直に書く）と
/// `tests/upstream_cli.rs::the_no_ui_flag_never_opens_the_browser_and_says_so_on_one_line`（同じ行の中で見る）。
pub(super) const UI_SOURCE_NONE: &str = "none";

/// 配らない形の `ui_origin`（**出どころが無い**）。**空文字にしない**——欄が消えたのか無いのかが読めなくなる。
const UI_ORIGIN_NONE: &str = "-";

impl UiFiles {
    /// 配り先を決める。**この時点では実在を要求しない**（ビルド成果物なので）。
    ///
    /// `web_dir` が `None` なら埋め込み。`Some` ならそのディレクトリ。
    pub(super) fn new(web_dir: Option<&Path>) -> Self {
        match web_dir {
            // **ディレクトリを指す要求には `index.html` を返す**（`ServeDir` の既定）。
            Some(root) => Self::Disk {
                root: root.to_path_buf(),
                files: ServeDir::new(root),
            },
            None => Self::Embedded {
                files: embedded_table(),
            },
        }
    }

    /// **画面がどこから来たか**（起動ログの `ui_source`）。
    pub(super) const fn source(&self) -> &'static str {
        match self {
            Self::Embedded { .. } => UI_SOURCE_EMBEDDED,
            Self::Disk { .. } => UI_SOURCE_DISK,
            Self::Withheld => UI_SOURCE_NONE,
        }
    }

    /// **その出どころの綴り**（起動ログの `ui_origin`）。
    ///
    /// 埋め込みなら**埋め込み元**（[`super::DEFAULT_WEB_DIR`]）、
    /// ディスクなら**設定された配り先そのもの**、配らないなら `-`。
    pub(super) fn origin(&self) -> &Path {
        match self {
            Self::Embedded { .. } => Path::new(super::DEFAULT_WEB_DIR),
            Self::Disk { root, .. } => root,
            Self::Withheld => Path::new(UI_ORIGIN_NONE),
        }
    }

    /// **配れるファイルの数**（起動ログの `ui_files`）。
    ///
    /// **起動ごとに1回だけ見る値**であって、以後の要求ごとには見ない
    /// （見ても意味が無い——`ServeDir` はそのつど実ファイルを開きに行く）。
    ///
    /// **ディレクトリが無いときも、空のときも `0`。**
    /// **どちらからも何も配れない**ので、区別する意味が無い
    /// （`0` なら起動ログが声を上げる）。
    pub(super) fn file_count(&self) -> usize {
        match self {
            Self::Embedded { files } => files.len(),
            Self::Disk { root, .. } => count_disk_files(root),
            Self::Withheld => 0,
        }
    }

    /// **1件も配れないときに出す警告の本文。** 出どころによって直す手が違う。
    ///
    /// **配らない形は `None`**——配らないと決めた起動に「画面が無い」と言うと、
    /// 利用者は `pnpm build` を探しに行く（作業単位 P-1）。
    pub(super) const fn missing_message(&self) -> Option<&'static str> {
        match self {
            Self::Embedded { .. } => Some(super::UI_EMBED_EMPTY_MESSAGE),
            Self::Disk { .. } => Some(super::WEB_DIR_MISSING_MESSAGE),
            Self::Withheld => None,
        }
    }

    /// 要求を UI のファイルとして処理する。
    ///
    /// 見つからなければ**本文の無い 404**（**API のエラー形にはしない**——
    /// モジュール doc の「SPA のフォールバックを入れていない」）。
    ///
    /// # `Err` の枝は、HTTP からは到達できない（実測）
    ///
    /// `ServeDir::try_call` は**ファイルが無いことを `Err` にしない**（404 の `Ok`）。
    /// D-0 のレビューで **4通り試して、どれも 404 の `Ok` に落ちた**
    /// （存在しないファイル / 配り先そのものが無い / ディレクトリを指す /
    /// 読めない置き方）。**`Err` を引ける HTTP 要求は見つかっていない。**
    ///
    /// **それでもこの枝を残す**のは、`Infallible` ではないから——
    /// 上流が将来 `Err` を返すようにしたときに、ここが**唯一の受け皿**になる。
    ///
    /// **⚠ 到達しないからといって `{err:?}` を載せてよいわけではない**
    /// （絶対制約「エラーレスポンスに内部パスを含めない」）。
    /// 詳細は**ログにだけ**残す。**変異検査では、この枝に `{err:?}` を載せる変異が
    /// 全緑になる**——それは「守られていない」ではなく「**HTTP から到達できない**」の意味だと
    /// 分かったうえで、意図をここに残しておく。
    pub(super) async fn respond(&self, request: Request) -> Response {
        match self {
            // **配らない形は、メソッドを問わず本文の無い 404**（画面そのものが無い）。
            Self::Withheld => {
                let mut response = Response::new(axum::body::Body::empty());
                *response.status_mut() = axum::http::StatusCode::NOT_FOUND;
                response
            }
            Self::Embedded { files } => embedded_response(files, &request),
            Self::Disk { files, .. } => {
                // `try_call` は `&mut self` を要る。
                // **`ServeDir` の複製は `PathBuf` 1本ぶん。**
                let mut files = files.clone();
                match files.try_call(request).await {
                    Ok(response) => response.into_response(),
                    Err(err) => {
                        // **応答には載せない**（内部パスが漏れる / 絶対制約）。
                        // ログにだけ残す。
                        tracing::error!(error = %err, "UI のファイルを読めませんでした");
                        ApiError::internal().into_response()
                    }
                }
            }
        }
    }
}

/// 埋め込みからの応答を組み立てる。
///
/// **`ServeDir` に合わせる**——受けるのは `GET` と `HEAD` だけ、
/// 見つからなければ**本文の無い 404**、それ以外のメソッドは**本文の無い 405**。
/// **どれも API のエラー形にしない**（モジュール doc）。
fn embedded_response(files: &[(&str, &[u8])], request: &Request) -> Response {
    use axum::http::{Method, StatusCode, header};

    let empty = |status: StatusCode| -> Response {
        let mut response = Response::new(axum::body::Body::empty());
        *response.status_mut() = status;
        response
    };

    if request.method() != Method::GET && request.method() != Method::HEAD {
        return empty(StatusCode::METHOD_NOT_ALLOWED);
    }

    let Some((name, contents)) = lookup_embedded(files, request.uri().path()) else {
        return empty(StatusCode::NOT_FOUND);
    };

    // **`HEAD` は本文を持たない**（欄は同じ）。
    let body = if request.method() == Method::HEAD {
        axum::body::Body::empty()
    } else {
        axum::body::Body::from(contents.to_vec())
    };
    let mut response = Response::new(body);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(content_type(name)),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        axum::http::HeaderValue::from(contents.len()),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **`router` が実際に生やしている API のパス。**
    ///
    /// **`API_PREFIXES` から組み立てないこと。** 組み立てると
    /// 「前置きを変えたら期待値も一緒に動く」ので、何も見ていないことになる
    /// （CLAUDE.md「関数の外にある値で押さえる見張り番」）。
    /// ここは `src/server/mod.rs` の `router` を**目で写した綴り**である。
    const API_ROUTES: [&str; 8] = [
        "/v1/models",
        "/v1/chat/completions",
        "/admin/characters",
        "/admin/characters/aoi",
        "/admin/characters/aoi/stats",
        "/admin/characters/aoi/attributes",
        "/admin/characters/aoi/memories",
        "/admin/characters/aoi/memories/1/corrections",
    ];

    /// **生えている API のパスは、どれも API と判定される。**
    ///
    /// 判定から漏れた前置きは、**そこだけ未知パスが UI に飲み込まれる。**
    #[test]
    fn every_api_route_is_recognised_as_an_api_path() {
        for route in API_ROUTES {
            assert!(is_api_path(route), "API のパスと判定されないこと: {route}");
        }
    }

    /// **前置きの直後が区切りでなければ API ではない。**
    ///
    /// `starts_with` だけで書くと `/version` や `/administration` まで API になり、
    /// **UI 側のそのパスが永久に 404 になる**（しかも API のエラー形で返るので、
    /// ブラウザから見ると原因が分からない）。
    ///
    /// # 崩した綴りも同じ答えになること
    ///
    /// **受理する側と拒否する側を1本に置く**（CLAUDE.md「高い側と安い側を1本に」）。
    /// 分けると「全部 API」に潰す変異と「全部 UI」に潰す変異のどちらかが緑を通る。
    ///
    /// **`//v1/x` は珍しい形ではない**——base URL が `/` で終わるクライアントが
    /// `base + "/v1/chat/completions"` を組むと必ずこうなる。
    #[test]
    fn a_prefix_that_is_not_followed_by_a_separator_is_not_an_api_path() {
        // (path, API か)
        let cases: [(&str, bool); 22] = [
            // --- 素の綴り（D-0 から変わらない） ---
            ("/version", false),
            ("/v1x", false),
            ("/administration", false),
            ("/admins", false),
            ("/v10/x", false),
            ("/v1", true),
            ("/v1/does-not-exist", true),
            ("/admin/does-not-exist", true),
            // --- 空セグメント（`base + "/v1/…"`）。**飛ばす。** ---
            ("//v1/x", true),
            ("//admin/x", true),
            ("///v1/x", true),
            ("//version", false),
            // --- `.` セグメント。**飛ばす。** ---
            ("/./v1/x", true),
            ("/././admin/x", true),
            ("/./version", false),
            // --- percent エンコード。**1回だけ解く。** ---
            ("/%76%31/x", true),
            ("/%61dmin/x", true),
            ("/%2e/v1/x", true),
            // 16 進は**大文字も読む**（`%2E` は `.`）。
            ("/%2E/v1/x", true),
            // **解いた結果は大文字小文字を区別する**（`%41dmin` は `Admin`）。
            ("/%41dmin/x", false),
            // **2回解かない。** 解き切ると受理集合が広がりすぎる
            // （`%252e%252e%252f` の類が API 側へ入る）。
            ("/%2576%2531/x", false),
            ("/%252e%252e%252fv1/x", false),
        ];

        for (path, want) in cases {
            assert_eq!(
                is_api_path(path),
                want,
                "{path} は {} と判定すること",
                if want { "API" } else { "UI" }
            );
        }
    }

    /// **解けない `%` はそのまま通す。落ちない。**
    ///
    /// パスは**信頼できない入力**である。`%` の後ろが 16 進でない・そもそも
    /// **途中で切れている**綴りは普通に飛んでくる（走査・手打ち・壊れた中継）。
    /// **ここで添字が範囲外になると、未知パス1本でサーバが落ちる。**
    ///
    /// **`/v1` の下にあるかどうかの判定は、解けた分だけで行う**——
    /// 解けなかった `%` は**セグメントの一部として残る**ので、
    /// `/%7/x` は `v1` にも `admin` にもならない。
    #[test]
    fn an_undecodable_escape_is_left_alone_and_never_panics() {
        for (path, want) in [
            ("/%zz/v1/x", false),
            ("/%7", false),
            ("/%", false),
            ("/v1%", false),
            ("/%76%3", false),
            // **解けない `%` が後ろにあっても、最初のセグメントが `v1` なら API。**
            ("/v1/%", true),
            ("/%76%31/x%", true),
        ] {
            assert_eq!(is_api_path(path), want, "{path}");
        }
    }

    /// **最初のセグメントが `..` なら API ではない**（＝ UI へ落ちる）。
    ///
    /// **意図的にこちらを選んだ。** `..` を飛ばして `/../v1/x` を API と読むこともできるが、
    /// **飛ばすと「上へ登る」という意味を正規化が勝手に消す**ことになり、
    /// `is_api_path` が `ServeDir` の解釈（`..` は拒否する）と食い違う。
    /// **現実のクライアントが作る形ではない**ので、食い違いを作らない方を採る。
    ///
    /// **選んだ結果**: `/../v1/x` は `ServeDir` が本文の無い 404 を返す
    /// （D-0 以前は OpenAI 形の 404 だった）。**これを承知で固定している。**
    /// 理由は `web` モジュールの doc。
    #[test]
    fn a_leading_dot_dot_segment_is_not_an_api_path() {
        for path in ["/../v1/x", "/..%2fv1/x", "/%2e%2e/admin/x", "/../admin"] {
            assert!(
                !is_api_path(path),
                "`..` から始まるパスは API と読まないこと: {path}"
            );
        }

        // **対比**: 同じ形でも `..` が無ければ API 側。
        // これが無いと「全部 UI」に潰す変異がここで緑を通る。
        for path in ["/v1/x", "/%2e/admin/x"] {
            assert!(is_api_path(path), "`..` が無ければ API であること: {path}");
        }
    }

    /// **前置きそのもの（`/v1` / `/admin`）も API 側。**
    ///
    /// ここを UI へ落とすと、`/admin` を開いた運用者に**素の 404** が返る一方で
    /// `/admin/` には API のエラーが返る——同じものを指す2つの綴りが別の答えを返す。
    #[test]
    fn the_prefix_itself_is_an_api_path() {
        for path in ["/v1", "/admin", "/v1/", "/admin/"] {
            assert!(is_api_path(path), "API のパスであること: {path}");
        }
    }

    /// **UI のパスは API ではない。**
    #[test]
    fn a_ui_path_is_not_an_api_path() {
        for path in ["/", "/index.html", "/assets/app.js", "/characters"] {
            assert!(!is_api_path(path), "UI のパスであること: {path}");
        }
    }

    /// **配れる件数は、実際に配れるものだけを数える。**
    ///
    /// 4つを1本に置く（無い / ファイルを指している / 空のディレクトリ / 中身あり）——
    /// 「あると言い張る」変異も「無いと言い張る」変異もここで赤くなる。
    ///
    /// **⚠ D-0 の `only_a_real_directory_counts_as_present` の後継**
    /// （F-4a で `exists()` が [`UiFiles::file_count`] になった）。
    /// **空のディレクトリの扱いが変わった**——以前は「実在する」（＝黙る）だったが、
    /// **そこからは何も配れない**ので、いまは `0`（＝警告する）。
    /// **`exists()` の doc が「ファイルが置いてあった場合も false。
    /// そこからは何も配れないので」と書いていた向きを、空のディレクトリにも当てた。**
    #[test]
    fn only_files_that_can_actually_be_served_are_counted() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");

        let missing = dir.path().join("missing");
        let file = dir.path().join("a-file");
        std::fs::write(&file, "not a directory").expect("ファイルを書けること");
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).expect("ディレクトリを作れること");
        let filled = dir.path().join("filled");
        std::fs::create_dir_all(filled.join("assets")).expect("ディレクトリを作れること");
        std::fs::write(filled.join("index.html"), "<!doctype html>").expect("書けること");
        std::fs::write(filled.join("assets").join("app.js"), "//").expect("書けること");

        assert_eq!(
            UiFiles::new(Some(&missing)).file_count(),
            0,
            "無いものからは何も配れない"
        );
        assert_eq!(
            UiFiles::new(Some(&file)).file_count(),
            0,
            "ファイルからは何も配れない"
        );
        assert_eq!(
            UiFiles::new(Some(&empty)).file_count(),
            0,
            "空のディレクトリからは何も配れない"
        );
        assert_eq!(
            UiFiles::new(Some(&filled)).file_count(),
            2,
            "入れ子の中まで数えること"
        );
    }

    /// **設定した綴りがそのまま `origin` に残る**（`ServeDir` の内部の写しではない）。
    ///
    /// ここが `.` 付きに化けると、起動ログの綴りが設定と食い違い、
    /// **「どこを配っているか」を確かめる唯一の手掛かりが濁る。**
    ///
    /// **埋め込みの腕も同じ1本で見る**——出どころの綴りは**埋め込み元**であって、
    /// 設定された配り先ではない（**未設定なのだから、設定された配り先は無い**）。
    #[test]
    fn each_arm_names_its_own_origin() {
        let disk = UiFiles::new(Some(Path::new("some/where/else")));
        assert_eq!(disk.source(), "disk");
        assert_eq!(disk.origin(), Path::new("some/where/else"));

        let embedded = UiFiles::new(None);
        assert_eq!(embedded.source(), "embedded");
        assert_eq!(
            embedded.origin(),
            Path::new(super::super::DEFAULT_WEB_DIR),
            "埋め込みの出どころは埋め込み元の綴りであること"
        );

        // **2つは別の綴り**（同値だと取り違えが観測できない）。
        assert_ne!(disk.source(), embedded.source());
        assert_ne!(disk.origin(), embedded.origin());
    }

    /// **出どころごとに、直す手の違う文面を出す。**
    #[test]
    fn the_empty_warning_tells_you_which_build_to_rerun() {
        assert_eq!(
            UiFiles::new(None).missing_message(),
            Some(super::super::UI_EMBED_EMPTY_MESSAGE)
        );
        assert_eq!(
            UiFiles::new(Some(Path::new("x"))).missing_message(),
            Some(super::super::WEB_DIR_MISSING_MESSAGE)
        );
        assert_ne!(
            super::super::UI_EMBED_EMPTY_MESSAGE,
            super::super::WEB_DIR_MISSING_MESSAGE,
            "前提: 2つの文面は別であること"
        );
        assert!(
            super::super::UI_EMBED_EMPTY_MESSAGE.contains("cargo build"),
            "埋め込みは組み直しが要ることを言うこと: {}",
            super::super::UI_EMBED_EMPTY_MESSAGE
        );
    }

    // ------------------------------------------------------------------
    // 埋め込みの探索（**表を引数で受ける純粋関数**を、植えた表で測る）
    // ------------------------------------------------------------------

    /// 植えた表。**中身はどれも違う文字列**——同じにすると取り違えが観測できない。
    const PLANTED: [(&str, &[u8]); 4] = [
        ("index.html", b"PLANTED-INDEX"),
        ("assets/app.js", b"PLANTED-JS"),
        ("assets/app.css", b"PLANTED-CSS"),
        ("v1/does-not-exist", b"PLANTED-V1"),
    ];

    /// **`/` は `index.html`**（ディスク側の `ServeDir` と同じ振る舞い）。
    ///
    /// **崩れた綴りも同じ答えになること**を1本に置く（`//` / `/./` / percent）。
    #[test]
    fn the_root_of_the_embedded_ui_is_the_index() {
        for path in ["/", "//", "/.", "/./", "///./"] {
            assert_eq!(
                lookup_embedded(&PLANTED, path),
                Some(("index.html", &b"PLANTED-INDEX"[..])),
                "{path} はルートであること"
            );
        }
    }

    /// **名前の合うものを、その中身と一緒に返す。**
    ///
    /// **無いものは `None`**（＝本文の無い 404。API のエラー形にはしない）。
    #[test]
    fn the_embedded_lookup_returns_the_file_it_was_asked_for() {
        assert_eq!(
            lookup_embedded(&PLANTED, "/assets/app.js"),
            Some(("assets/app.js", &b"PLANTED-JS"[..]))
        );
        assert_eq!(
            lookup_embedded(&PLANTED, "/assets/app.css"),
            Some(("assets/app.css", &b"PLANTED-CSS"[..])),
            "隣のファイルと取り違えないこと"
        );
        // **崩れた綴りでも同じものを引く。**
        assert_eq!(
            lookup_embedded(&PLANTED, "//assets/./app.js"),
            Some(("assets/app.js", &b"PLANTED-JS"[..]))
        );
        assert_eq!(
            lookup_embedded(&PLANTED, "/assets/%61pp.js"),
            Some(("assets/app.js", &b"PLANTED-JS"[..])),
            "percent は1回だけ解くこと"
        );

        for path in ["/no-such-file", "/assets/", "/assets", "/INDEX.HTML"] {
            assert_eq!(
                lookup_embedded(&PLANTED, path),
                None,
                "{path} は引けないこと（大文字小文字も区別する）"
            );
        }
    }

    /// **`..` は絶対に辿らない**（ADR-0041 §決定2 / `ServeDir` と同じ解釈）。
    ///
    /// # なぜ「飛ばす」ではなく「拒む」か
    ///
    /// **飛ばすと `/assets/../v1/does-not-exist` が引けてしまう。**
    /// 上へ登る意味を正規化が勝手に消すと、
    /// [`is_api_path`]（`..` を飛ばさない）と食い違う。
    ///
    /// **対比**: `..` を含まない同じ形は引けること
    /// ——これが無いと「全部 `None`」に潰す変異がここで緑を通る。
    #[test]
    fn a_dot_dot_segment_never_resolves_in_the_embedded_ui() {
        for path in [
            "/../index.html",
            "/assets/../index.html",
            "/assets/../v1/does-not-exist",
            "/..%2findex.html",
            "/%2e%2e/index.html",
            "/a/b/../../index.html",
            "/..",
        ] {
            assert_eq!(
                lookup_embedded(&PLANTED, path),
                None,
                "`..` を辿ってはいけない: {path}"
            );
        }

        // **対比**——`..` が無ければ引ける。
        assert_eq!(
            lookup_embedded(&PLANTED, "/v1/does-not-exist"),
            Some(("v1/does-not-exist", &b"PLANTED-V1"[..])),
            "`..` が無ければ引けること"
        );
    }

    /// **Content-Type は拡張子から引く。引けなければ落とし所。**
    ///
    /// # なぜ全部を1本に置くか
    ///
    /// **`.js` と `.css` を入れ替える変異**は、片方だけ見ていると通る。
    /// **表そのものを突き合わせる**ので、1つ落としても入れ替えても赤くなる。
    ///
    /// **期待値を [`CONTENT_TYPES`] から作らない**——作ると表と一緒に動く。
    #[test]
    fn the_content_type_comes_from_the_extension() {
        for (name, want) in [
            ("index.html", "text/html; charset=utf-8"),
            ("assets/app.js", "text/javascript; charset=utf-8"),
            ("assets/app.mjs", "text/javascript; charset=utf-8"),
            ("assets/app.css", "text/css; charset=utf-8"),
            ("data.json", "application/json"),
            ("app.js.map", "application/json"),
            ("logo.svg", "image/svg+xml"),
            ("logo.png", "image/png"),
            ("photo.jpg", "image/jpeg"),
            ("photo.jpeg", "image/jpeg"),
            ("anim.gif", "image/gif"),
            ("photo.webp", "image/webp"),
            ("favicon.ico", "image/x-icon"),
            ("font.woff2", "font/woff2"),
            ("font.woff", "font/woff"),
            ("readme.txt", "text/plain; charset=utf-8"),
            // **引けないものは落とし所**（止めない / ADR-0041 §決定2）。
            ("archive.tar.zst", "application/octet-stream"),
            ("no-extension", "application/octet-stream"),
            ("trailing.", "application/octet-stream"),
            // **大文字小文字は区別しない。**
            ("INDEX.HTML", "text/html; charset=utf-8"),
            ("APP.Js", "text/javascript; charset=utf-8"),
        ] {
            assert_eq!(content_type(name), want, "{name} の Content-Type");
        }

        // **`.js` と `.css` が別の答えを返すこと**（入れ替えの変異を1行で名指しする）。
        assert_ne!(content_type("a.js"), content_type("a.css"));
    }

    // ------------------------------------------------------------------
    // 埋め込み元の綴り（**鎖のもう一方の端** / `config` の `DEFAULT_WEB_DIR`）
    // ------------------------------------------------------------------

    /// 埋め込みのマクロが出どころを書いている形。
    ///
    /// **`$CARGO_MANIFEST_DIR/` の後ろが埋め込み元。**
    const EMBED_MACRO_KEY: &str = "include_dir!(\"$CARGO_MANIFEST_DIR/";

    /// ソースから**埋め込み元の綴り**を拾う（見つからなければ `None`）。
    ///
    /// **引数でソースを受け取る**——実ファイルと植えたソースを同じ本体に通すため
    /// （CLAUDE.md `15b`）。**綴りに `\n` を含めない**ので CRLF でも同じ答えを返す。
    fn embedded_source_dir(source: &str) -> Option<&str> {
        let at = source.find(EMBED_MACRO_KEY)?;
        let rest = &source[at + EMBED_MACRO_KEY.len()..];
        let end = rest.find('"')?;
        Some(&rest[..end])
    }

    /// このファイルの中身。
    fn web_source() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("server")
            .join("web.rs");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("{} を読めること: {err}", path.display()))
    }

    /// **埋め込み元は [`super::super::DEFAULT_WEB_DIR`] と同じ綴り**
    /// （ADR-0041 §決定2 / 作業単位 F-4a）。
    ///
    /// # なぜ走査で押さえるか
    ///
    /// **`include_dir!` は定数を受け取れない**（マクロが要求するのはリテラル）。
    /// **つまり綴りの写しが1つ生える。**
    /// **写しどうしが食い違うと、埋め込まれる画面が「ビルドした画面」でなくなる**
    /// ——しかも**ビルドは通る**（そのディレクトリが在れば）。
    ///
    /// `config` 側の `the_default_web_dir_is_where_vite_actually_builds` が
    /// **`vite.config.ts` ← 定数**を押さえ、ここが**定数 → 埋め込み元**を押さえる。
    /// **2本で1本の鎖。**
    ///
    /// # ⚠ 守っていないもの
    ///
    /// **`build.rs` が作る（そして `cargo:rerun-if-changed` に登録する）ディレクトリの
    /// 綴りは見ていない。** あちらは build script なので lib の定数を参照できず、
    /// **3つ目の写し**になっている。**食い違ったときの現れ方は
    /// 「`pnpm build` しても実行ファイルの中の画面が変わらない」。**
    #[test]
    fn the_embedded_ui_comes_from_the_default_web_dir() {
        let source = web_source();

        // **門**: 綴りが1箇所きりであること（増えたら、どれを見ているか分からない）。
        assert_eq!(
            source.matches(EMBED_MACRO_KEY).count(),
            1,
            "埋め込みのマクロは1箇所きりであること"
        );

        assert_eq!(
            embedded_source_dir(&source),
            Some(super::super::DEFAULT_WEB_DIR),
            "埋め込み元と DEFAULT_WEB_DIR が食い違っている\
             （片方だけ変えると、ビルドした画面が入らなくなる）"
        );
    }

    /// **走査本体の対照**——植えたソースを、実ファイルと同じ本体に通す。
    ///
    /// これが無いと、走査の綴りを壊す変異が**空振りしたまま緑**になりうる
    /// （CLAUDE.md `15b`「自己成就」）。
    #[test]
    fn the_embed_scan_reads_the_source_it_is_given() {
        assert_eq!(
            embedded_source_dir("static X = include_dir!(\"$CARGO_MANIFEST_DIR/planted/dir\");"),
            Some("planted/dir"),
            "植えた綴りをそのまま拾うこと"
        );
        // **CRLF でも同じ答え**（clone 直後の作業ツリーは全ファイル CRLF）。
        assert_eq!(
            embedded_source_dir(
                "static X =\r\n    include_dir!(\"$CARGO_MANIFEST_DIR/planted/dir\");\r\n"
            ),
            Some("planted/dir"),
        );
        assert_eq!(
            embedded_source_dir("include_dir!(\"some/other/form\")"),
            None,
            "`$CARGO_MANIFEST_DIR/` で始まらない形は拾わないこと"
        );
        assert_eq!(
            embedded_source_dir("埋め込みは行っていない"),
            None,
            "マクロが無いソースを、黙って何かに解釈しないこと"
        );
        assert_eq!(
            embedded_source_dir(""),
            None,
            "空のソースは「見つからない」"
        );
    }

    /// **植えた `Dir` を、実物と同じ畳み方に通す**（レビュー指摘2 / CLAUDE.md `15b`）。
    ///
    /// # なぜ `Dir` を植えるのか
    ///
    /// **埋め込みの中身はコンパイル時に決まるので、`web/dist` に3段目を作って測れない。**
    /// **`include_dir::Dir::new` / `File::new` は `const fn`** なので、
    /// **入れ子を手で組んで [`flatten_embedded`] に通せる。**
    ///
    /// **3段目まで畳めること**と、**[`MAX_UI_DEPTH`] で止まること**を1本で見る。
    #[test]
    fn the_embedded_flattening_goes_all_the_way_down() {
        use include_dir::{Dir, DirEntry, File};

        // web/dist/
        //   index.html
        //   assets/
        //     app.js
        //     fonts/
        //       deep.txt      ← **3段目。ここが初版で落ちていた。**
        static ROOT: Dir<'static> = Dir::new(
            "",
            &[
                DirEntry::File(File::new("index.html", b"PLANTED-INDEX")),
                DirEntry::Dir(Dir::new(
                    "assets",
                    &[
                        DirEntry::File(File::new("assets/app.js", b"PLANTED-APP")),
                        DirEntry::Dir(Dir::new(
                            "assets/fonts",
                            &[DirEntry::File(File::new(
                                "assets/fonts/deep.txt",
                                b"PLANTED-DEEP",
                            ))],
                        )),
                    ],
                )),
            ],
        );

        let mut files = Vec::new();
        flatten_embedded(&ROOT, 0, &mut files);
        files.sort_unstable_by_key(|(name, _)| *name);

        assert_eq!(
            files,
            vec![
                ("assets/app.js", &b"PLANTED-APP"[..]),
                ("assets/fonts/deep.txt", &b"PLANTED-DEEP"[..]),
                ("index.html", &b"PLANTED-INDEX"[..]),
            ],
            "入れ子の奥まで畳むこと（2段で止まる実装はここで赤くなる）"
        );

        // **探索も3段目まで届くこと**（表に在っても引けなければ意味が無い）。
        assert_eq!(
            lookup_embedded(&files, "/assets/fonts/deep.txt"),
            Some(("assets/fonts/deep.txt", &b"PLANTED-DEEP"[..]))
        );

        // **深さの天井で止まること**——`MAX_UI_DEPTH` 段目からは畳まない。
        let mut stopped = Vec::new();
        flatten_embedded(&ROOT, MAX_UI_DEPTH, &mut stopped);
        assert!(
            stopped.is_empty(),
            "天井に当たったら降りないこと: {stopped:?}"
        );
    }

    /// **実物の表も、植えた表と同じ本体を通ること**（対照だけが通る形にしない）。
    ///
    /// # ⚠ ここが見ているもの
    ///
    /// **埋め込みの中身はコンパイル時に決まるので、テストから植えられない。**
    /// **`web/dist` をビルドしていなければ表は空**で、そのときこの検査は
    /// **「空でも落ちない」しか言わない。**
    ///
    /// **`pnpm build` 済みなら、`index.html` が引けることまで見る**
    /// ——`web/dist` を実際に読んで、**実ファイルと表が同じものを指していること**を
    /// 突き合わせる（埋め込み元を別の場所へ向ける変異は、ここでも赤くなる）。
    #[test]
    fn the_real_embedded_table_goes_through_the_same_lookup() {
        let files = embedded_table();
        let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("web")
            .join("dist");
        let index = dist.join("index.html");

        match std::fs::read(&index) {
            Ok(contents) => {
                assert_eq!(
                    lookup_embedded(files, "/").map(|(_, body)| body.to_vec()),
                    Some(contents),
                    "埋め込んだ index.html が web/dist のものと違う\
                     （`pnpm build` の後に `cargo build` し直したか / 埋め込み元は正しいか）"
                );
                assert!(
                    !files.is_empty(),
                    "web/dist に中身があるのに埋め込みが空である"
                );

                // **表の名前の集合が、`web/dist` を歩いた集合と一致すること**
                // （レビュー指摘2）。**入れ子を落とす変異は、その日の `web/dist` が
                // 深くなった瞬間にここで赤くなる。**
                // **⚠ いまの `web/dist` は 2 段**なので、**この検査は 3 段目を持っていない。**
                // **3 段目の決定的な権威は
                // `the_embedded_flattening_goes_all_the_way_down`**（植えた `Dir`）。
                let mut on_disk = Vec::new();
                walk_relative(&dist, &dist, &mut on_disk);
                on_disk.sort();
                let mut embedded: Vec<String> =
                    files.iter().map(|(name, _)| (*name).to_owned()).collect();
                embedded.sort();
                assert_eq!(
                    embedded, on_disk,
                    "埋め込んだ名前の集合が web/dist と食い違っている"
                );
            }
            Err(_) => {
                // **`pnpm build` していない状態。** 落とさない
                // （`cargo test` は FE をビルドしない / ADR-0041 §決定2）。
                assert_eq!(
                    lookup_embedded(files, "/"),
                    None,
                    "web/dist が無いのに何かを引けている"
                );
            }
        }
    }

    /// ディレクトリの下のファイルを、`root` からの `/` 区切りの相対名で集める。
    ///
    /// **`count_disk_files` を呼ばない**——呼ぶと、数え方を変える変異と一緒に
    /// 期待値が動く（CLAUDE.md `16b`）。**ここはテストが自分で歩く側。**
    fn walk_relative(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_relative(root, &path, out);
            } else if let Ok(relative) = path.strip_prefix(root) {
                out.push(
                    relative
                        .components()
                        .filter_map(|part| part.as_os_str().to_str())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
    }

    // ------------------------------------------------------------------
    // 画面を配らない形（`--no-ui` / ADR-0047 §① / 作業単位 P-1）
    // ------------------------------------------------------------------

    /// 要求を1本組む（本文なし）。
    fn request(method: &str, path: &str) -> Request {
        Request::builder()
            .method(method)
            .uri(path)
            .body(axum::body::Body::empty())
            .expect("要求を組めること")
    }

    /// **配らない形は、画面のどのパスにも本文の無い 404 を返す。**
    ///
    /// # 対比（同じ要求を、配る形にも通す）
    ///
    /// **同じ要求が配る形では 200 になる**ことを同じ1本で見る——見ないと、
    /// 「要求の綴りが壊れていて、どちらでも 404」という形でこの検査が自己成就する。
    /// **配る形は植えたディレクトリ**（`web/dist` をビルドしていない機でも中身がある）。
    #[tokio::test]
    async fn the_withheld_ui_answers_404_to_every_screen_path() {
        use axum::http::StatusCode;

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        std::fs::create_dir_all(dir.path().join("assets")).expect("作れること");
        std::fs::write(dir.path().join("index.html"), "PLANTED-INDEX").expect("書けること");
        std::fs::write(dir.path().join("assets").join("app.js"), "PLANTED-JS").expect("書けること");
        let served = UiFiles::new(Some(dir.path()));

        for (method, path) in [
            ("GET", "/"),
            ("GET", "/index.html"),
            ("GET", "/assets/app.js"),
            ("HEAD", "/"),
        ] {
            let response = served.respond(request(method, path)).await;
            assert_eq!(
                response.status(),
                StatusCode::OK,
                "前提: 配る形では {method} {path} が引けること"
            );

            let response = UiFiles::Withheld.respond(request(method, path)).await;
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "配らない形が {method} {path} に答えている"
            );
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("本文を読めること");
            assert!(body.is_empty(), "本文の無い 404 であること: {body:?}");
        }

        // **GET 以外も 404**（配る形は 405 だが、配らない形には「画面」が無い）。
        let response = UiFiles::Withheld.respond(request("POST", "/")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// **配らない形は、起動ログの3つの欄で自分を名乗り、空の警告を出させない。**
    ///
    /// **`ui_source` の綴りは直に書く**（定数から作らない / CLAUDE.md `16b`）——
    /// 集計はこの綴りで引くので、変えたら集計が黙って空になる。
    #[test]
    fn the_withheld_ui_names_itself_and_never_warns_that_it_is_empty() {
        let withheld = UiFiles::Withheld;
        assert_eq!(withheld.source(), "none");
        assert_eq!(withheld.origin(), Path::new("-"));
        assert_eq!(withheld.file_count(), 0);
        assert_eq!(
            withheld.missing_message(),
            None,
            "配らないと決めた起動に「画面が無い」と警告しない"
        );

        // **他の2つとは別の綴り**（同値だと取り違えが観測できない）。
        assert_ne!(withheld.source(), UiFiles::new(None).source());
        assert_ne!(
            withheld.source(),
            UiFiles::new(Some(Path::new("x"))).source()
        );
    }
}
