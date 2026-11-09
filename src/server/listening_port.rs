//! **画面で保存する待ち受けのポート**（[ADR-0057](../../docs/adr/0057-the-listening-port-shifts-when-taken-and-is-set-from-the-screen.md)
//! §決定5・6 / 作業単位 F-12a / marukei/personacore#15）。
//!
//! ```text
//! %LOCALAPPDATA%\personacore\
//!   upstreams.json          … 利用者のファイル（ここには書かない）
//!   selected-upstream.txt   … 覚えた上流の名前（`remembered`）
//!   listening-port.txt      … 保存した待ち受けのポート 1 行（このモジュール）
//! ```
//!
//! # 置き場所は覚えた名前の隣（**既定の置き場所のときだけ**）
//!
//! **場所の決め方は `remembered` と同じ関数を通す**（[`saved_port_path_beside`] / [`memory_for`] は
//! `remembered::remembered_path_beside` / `remembered::memory_for` の結果のファイル名だけを替える）。
//! `PERSONACORE_UPSTREAMS` が指す先（`ProfilesLocation::ReadOnly`）では読まない・書かない。
//! **DB には置かない**（`--fresh` で忘れない）。
//!
//! # 効く順（`config::resolve_port`）
//!
//! `PERSONACORE_PORT` → 保存した番号 → `DEFAULT_PORT`。**保存した番号は `PERSONACORE_PORT` の値として混ぜず、
//! 出どころ（`PortSource`）と一緒に `AppConfig` に載せる**（`AppConfig::from_lookup_with_saved_port`）。
//!
//! # 読めないとき
//!
//! **範囲の外（1023 以下・65536 以上）・数でない・2行以上は「読めない」**（[`parse_saved_port`]）。
//! `main` は警告を1本出して保存が無いものとして起きる。**ファイルは消さない**（覚えた名前と同じ扱い）。
//! **空のファイルは「保存なし」**（読めない、ではない）。
//!
//! # API（ADR-0057 §決定6 の F-12 の行）
//!
//! | 口 | 何をする |
//! |---|---|
//! | `GET /admin/server` | いまの待ち受け・出どころ・元の番号・ずらしたか・飛ばした番号・保存した番号・次の起動で使う番号と出どころ・保存できるか（[`ServerView`]） |
//! | `PUT /admin/server/port` | `{"port": 8090}` で保存、`{"port": null}` で保存を消す（[`SavePortView`]） |
//!
//! **動いている間に待ち受けを移さない**（保存は次の起動から効く / ADR-0057 §決定5）。
//! **保存した番号が次の起動で使われていれば、その起動は次の空きへずらす**（ADR-0057 §決定3 / 作業単位 F-11b /
//! `port_search` モジュール）。元の番号・ずらしたか・飛ばした番号は `GET /admin/server` が言う（API 1.6）。
//!
//! # ⚠ 守れないもの
//!
//! * **2つの personacore が同時に書くと、後に置き換えた方が残る**（`remembered` と同じ）。
//!   この実行ファイルの中では [`Listening`] の錠の下で書いて読み直す。
//! * **「いま使われているか」は確かめた瞬間の話**（[`port_in_use`]）。次の起動までに変わりうる。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa::openapi::schema::Object;

use super::config::{BindAddr, ENV_PORT, PortSource, SavedPort, resolve_port};
use super::error::{ApiError, ErrorBody};
use super::port_search::OpenedPort;
use super::profile_editor::{ProfilesLocation, RealSteps, ReplaceError, replace_file_with};
use super::remembered;
use super::{AppState, openapi};

/// **保存した待ち受けのポートを置くファイルの名前**（ADR-0057 §決定5）。
pub const LISTENING_PORT_FILE_NAME: &str = "listening-port.txt";

/// ログに出す入口の名前（`ApiError::from_json_rejection`）。
const ENDPOINT: &str = "admin/server/port";

// ----------------------------------------------------------------------
// 置き場所（**純粋関数**）
// ----------------------------------------------------------------------

/// **保存した番号の置き場所**——`upstreams.json` と同じディレクトリ。**純粋関数。**
///
/// **覚えた名前（`remembered::remembered_path_beside`）と同じ関数を通し、ファイル名だけを替える**
/// （ディレクトリの決め方の写しを作らない）。
#[must_use]
pub fn saved_port_path_beside(profiles: &Path) -> PathBuf {
    remembered::remembered_path_beside(profiles).with_file_name(LISTENING_PORT_FILE_NAME)
}

/// **この起動が読み・書く先。** 既定の置き場所（画面から書ける）のときだけ `Some`。**純粋関数。**
///
/// **`remembered::memory_for` と同じ1つの値から決める**——「覚えた名前は覚えるのに、ポートは保存しない」
/// （またはその逆）の配線が書けない。
pub(crate) fn memory_for(location: &ProfilesLocation) -> Option<PathBuf> {
    remembered::memory_for(location).map(|path| path.with_file_name(LISTENING_PORT_FILE_NAME))
}

// ----------------------------------------------------------------------
// 読む・書く
// ----------------------------------------------------------------------

/// **保存した番号を読めない**（`main` が警告にして、保存が無いものとして起きる）。
///
/// `Display` は**定数文字列**。理由は `#[source]` 側（[`std::io::Error`] は置き場所を含まない）。
/// **置き場所は呼び出し側がログに出す。**
#[derive(Debug, thiserror::Error)]
pub enum SavedPortUnreadable {
    /// 読めない（権限・その名前がディレクトリ・UTF-8 でない）。
    #[error("保存した待ち受けのポートを読めませんでした")]
    Io(#[source] std::io::Error),
    /// **1行ではない**（手で書き換えた）。どの行を採るかを推測しない。
    #[error("保存した待ち受けのポートが1行ではありません")]
    NotOneLine,
    /// **数（半角の 0〜9 だけ）ではない。**
    #[error("保存した待ち受けのポートが数ではありません")]
    NotANumber,
    /// **1024〜65535 の外**（`0` を含む）。
    #[error("保存した待ち受けのポートが 1024〜65535 の範囲の外です")]
    OutOfRange,
}

/// **ファイルの中身から番号を取り出す。純粋関数。**
///
/// * **前後の空白と BOM は落とす**（メモ帳で書き換えたときの癖を吸う / `remembered::parse_remembered` と同じ）。
/// * **空なら「保存なし」**（`None`）。
/// * **2行以上・数でない・範囲の外は読まない**（[`SavedPortUnreadable`]）。
pub(crate) fn parse_saved_port(text: &str) -> Result<Option<SavedPort>, SavedPortUnreadable> {
    let value = text.trim_start_matches('\u{feff}').trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.contains(['\n', '\r']) {
        return Err(SavedPortUnreadable::NotOneLine);
    }
    // **半角の数字だけ**——`str::parse` は `+8090` を通すので、先に字を見る。
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(SavedPortUnreadable::NotANumber);
    }
    // 数字だけなので、`parse` が失敗するのは桁あふれだけ（＝範囲の外）。
    value
        .parse::<u64>()
        .ok()
        .and_then(|number| u16::try_from(number).ok())
        .and_then(SavedPort::new)
        .map(Some)
        .ok_or(SavedPortUnreadable::OutOfRange)
}

/// **保存した番号を読む。** 無ければ `Ok(None)`（何も保存していない）。
///
/// # Errors
///
/// 読めない、または中身が番号として読めないとき。
pub fn read_saved_port(path: &Path) -> Result<Option<SavedPort>, SavedPortUnreadable> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_saved_port(&text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(SavedPortUnreadable::Io(err)),
    }
}

/// **ファイルに書く中身。純粋関数。** 番号1行（LF で終える）。
pub(crate) fn saved_port_text(port: SavedPort) -> String {
    format!("{}\n", port.get())
}

/// **保存できなかった・消せなかった。** `Display` は定数。
#[derive(Debug, thiserror::Error)]
pub(crate) enum SavePortError {
    /// 置き換えで書けなかった。
    #[error("待ち受けのポートを保存できませんでした")]
    Write(#[source] ReplaceError),
    /// 消せなかった。
    #[error("保存した待ち受けのポートを消せませんでした")]
    Remove(#[source] std::io::Error),
}

/// **保存する。置き換えで書く**（途中で落ちても、前の番号が読める形で残る）。
pub(crate) fn save_port(path: &Path, port: SavedPort) -> Result<(), SavePortError> {
    replace_file_with(&RealSteps, path, saved_port_text(port).as_bytes())
        .map_err(SavePortError::Write)
}

/// **保存を消す。ファイルごと消す**（無ければ何もしない）。
///
/// **空のファイルを書く形は採らなかった**——「保存なし」の表し方が「ファイルが無い」と「空」の2つに
/// 割れる。消せば、一度も保存していない機と同じ状態に戻る。
pub(crate) fn forget_saved_port(path: &Path) -> Result<(), SavePortError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(SavePortError::Remove(err)),
    }
}

// ----------------------------------------------------------------------
// いま使われているか
// ----------------------------------------------------------------------

/// **`127.0.0.1:<port>` を、いま他のプログラムが使っているか**（ADR-0057 §決定5）。
///
/// # 確かめ方は「開いてみる」（`bind`）。繋いではみない（`connect`）
///
/// * **次の起動がすることと同じ**——起動は `127.0.0.1:<port>` を `bind` する。開けなければ
///   （`AddrInUse`、Windows の除外ポート範囲の `PermissionDenied`）、次の起動はそこで待ち受けられない。
///   `connect` は除外ポート範囲を「空いている」と読む。
/// * **相手に何も送らない**——`connect` は塞いでいるプログラムに接続を1本作る（相手のログに残る）。
/// * 開けたら、その場で閉じる（掴み続けない）。
///
/// **宛先は [`BindAddr::loopback`] で作る**（`127.0.0.1` 以外を開く道を作らない）。
/// **いまこの personacore が使っている番号は、呼び出し側が先に除く**（自分自身を「使われている」と言わない）。
pub(crate) fn port_in_use(port: u16) -> bool {
    match std::net::TcpListener::bind(BindAddr::loopback(port).socket_addr()) {
        // **開けた＝空いている。** 束縛はこの文の終わりで放す。
        Ok(_listener) => false,
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            true
        }
        Err(err) => {
            // **それ以外の失敗は「使われている」と言わない**（他のプログラムが掴んでいる証拠ではない）。
            tracing::warn!(
                port,
                error = %err,
                "待ち受けのポートが使われているかを確かめられませんでした"
            );
            false
        }
    }
}

// ----------------------------------------------------------------------
// API
// ----------------------------------------------------------------------

/// **待ち受けについて、この起動が知っていること**（`AppState` が持つ / `serve` が作る）。
#[derive(Debug)]
pub(crate) struct Listening {
    /// **実際に待ち受けている番号**（`local_addr`。設定値ではない）。
    port: u16,
    /// **待ち受けをどう開けたか**——元の番号・その出どころ・ずらしたか・飛ばした番号
    /// （`port_search::open_first` の結果 / 作業単位 F-11b）。
    opened: OpenedPort,
    /// **保存した番号の置き場所**（[`memory_for`]。既定の置き場所でなければ `None`＝保存できない）。
    memory: Option<PathBuf>,
    /// **書いて読み直すまでを1つずつ**（同じ実行ファイルの中の2つの保存を混ぜない）。
    writing: Mutex<()>,
}

impl Listening {
    pub(crate) const fn new(port: u16, opened: OpenedPort, memory: Option<PathBuf>) -> Self {
        Self {
            port,
            opened,
            memory,
            writing: Mutex::new(()),
        }
    }

    /// **保存した番号を読み直す。** 戻り値は `(番号, 読めないか)`。
    ///
    /// | 状態 | 戻り値 |
    /// |---|---|
    /// | 保存できない置き場所（`ReadOnly` / 置き場所が無い） | `(None, false)`——**ファイルを読みもしない** |
    /// | ファイルが無い・空 | `(None, false)` |
    /// | 読めた | `(Some(番号), false)` |
    /// | **在るのに読めない**（範囲の外・数でない・2行以上・I/O の失敗） | `(None, true)` |
    ///
    /// # 読めないことはログでは言わない（`debug` だけ）
    ///
    /// **警告は起動時の1本（`main` の `SAVED_PORT_UNREADABLE`）だけ。** ここは `GET` のたびに通るので、
    /// `warn` にすると画面が読み直すたびに同じ警告が積もる（レビューの指摘）。**利用者へは応答の
    /// `saved_port_unreadable` が言う。** 理由（`#[source]`）は `debug` に残す——ログの絞りを上げれば、
    /// どのファイルがなぜ読めないかを起動し直さずに追える（出さない形はその手掛かりを捨てる）。
    fn saved(&self) -> (Option<SavedPort>, bool) {
        let Some(memory) = self.memory.as_deref() else {
            return (None, false);
        };
        match read_saved_port(memory) {
            Ok(saved) => (saved, false),
            Err(err) => {
                // **パスはログにだけ出す**（応答には載らない）。
                tracing::debug!(
                    saved_in = %memory.display(),
                    error = %err,
                    cause = ?std::error::Error::source(&err),
                    "保存した待ち受けのポートを読めません（応答の saved_port_unreadable で知らせます）"
                );
                (None, true)
            }
        }
    }

    /// **`GET /admin/server` の本文。** 保存のファイルと環境変数を、呼ぶたびに読み直す。
    pub(crate) fn view(&self) -> ServerView {
        let (saved, saved_port_unreadable) = self.saved();
        let (next_port, next_port_source) = next_start(saved);
        ServerView {
            listening_port: self.port,
            listening_port_source: self.opened.source().label(),
            listening_port_requested: self.opened.requested(),
            listening_port_shifted: self.opened.shifted(),
            listening_port_skipped: self.opened.skipped().to_vec(),
            saved_port: saved.map(SavedPort::get),
            saved_port_unreadable,
            next_port,
            next_port_source: next_port_source.label(),
            editable: self.memory.is_some(),
        }
    }

    /// **保存する・消す**（`PUT /admin/server/port`）。
    ///
    /// | 応答 | いつ |
    /// |---|---|
    /// | 400 `invalid_listening_port` | 1024〜65535 の外（`0`・負の値を含む）。**置き場所を見る前に断る**（依頼そのものの誤り） |
    /// | 409 `listening_port_read_only` | 保存できない置き場所（`PERSONACORE_UPSTREAMS`／置き場所が無い） |
    /// | 500 `listening_port_write_failed` | 書けなかった・消せなかった（**前の中身は残る**。原因はログだけ） |
    fn save(&self, request: SavePortRequest) -> Result<SavePortView, ApiError> {
        let wanted = match request.port {
            Some(value) => {
                Some(SavedPort::from_integer(value).ok_or_else(ApiError::invalid_listening_port)?)
            }
            None => None,
        };
        let Some(memory) = self.memory.as_deref() else {
            return Err(ApiError::listening_port_read_only());
        };

        // **錠が毒されていても続ける**（中身は `()`。守っているのは書いて読み直す順だけ）。
        let _writing = match self.writing.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let written = match wanted {
            Some(port) => save_port(memory, port),
            None => forget_saved_port(memory),
        };
        if let Err(err) = written {
            tracing::error!(
                saved_in = %memory.display(),
                error = %err,
                cause = ?std::error::Error::source(&err),
                "待ち受けのポートの保存を書き換えられませんでした（前の中身は残っています）"
            );
            return Err(ApiError::listening_port_write_failed());
        }
        tracing::info!(
            saved_port = ?wanted.map(SavedPort::get),
            saved_in = %memory.display(),
            "待ち受けのポートの保存を書き換えました（次の起動から効きます）"
        );

        // **いまこの personacore が使っている番号なら「使われている」と言わない**（ADR-0057 §決定5）。
        // **使われていても保存は止めない**（次の起動は、塞がっていれば次の空きへずらす / `port_search`）。
        let saved_port_in_use =
            wanted.map(|port| port.get() != self.port && port_in_use(port.get()));
        Ok(SavePortView {
            server: self.view(),
            saved_port_in_use,
        })
    }
}

/// **次の起動で使う番号と出どころ**——**いまのプロセスの環境変数**と保存した番号から、
/// 起動時と同じ関数（[`resolve_port`]）で決める。
///
/// **`PERSONACORE_PORT` が読めない綴りなら、次の起動は止まる**ので、番号は `None`・出どころは `environment`。
/// （この実行ファイルが同じ環境で起きている以上、本番の経路では起きない。）
fn next_start(saved: Option<SavedPort>) -> (Option<u16>, PortSource) {
    // **環境に触るのは起動時の `AppConfig::from_env` と、ここだけ**（「次の起動」を読み直すため）。
    match resolve_port(std::env::var(ENV_PORT).ok().as_deref(), saved) {
        Ok((port, source)) => (Some(port), source),
        Err(_) => (None, PortSource::Environment),
    }
}

/// 出どころの綴り（`PortSource::label` から）。
fn port_source_schema() -> Object {
    openapi::closed_string(PortSource::ALL.map(PortSource::label))
}

/// **`GET /admin/server` の本文**（ADR-0057 §決定6 の F-12・F-11 の行）。
///
/// | 欄 | 意味 |
/// |---|---|
/// | `listening_port` | **いま実際に待ち受けている番号**（`local_addr`。`PERSONACORE_PORT=0` なら OS が選んだ番号） |
/// | `listening_port_source` | **元の番号**の出どころ（`environment` / `saved` / `default`）。ずらしても変わらない |
/// | `listening_port_requested` | **元の番号**（出どころの番号。試したとは限らない——llama の面の番号なら試さずに飛ばす。`PERSONACORE_PORT=0` なら `0`）。ずらしておらず、`0` でもなければ `listening_port` と同じ |
/// | `listening_port_shifted` | **元の番号を使えず、別の番号で待ち受けたか**（`0` で OS に選ばせたのは `false`） |
/// | `listening_port_skipped` | **探すときに飛ばした番号**（llama の面が使う番号のうち、開けた番号より前のもの。ずらしていなければ空） |
/// | `saved_port` | 保存した番号（無い・読めない・保存できない置き場所なら `null`） |
/// | `saved_port_unreadable` | **保存のファイルが在るのに読めない**（範囲の外・数でない・2行以上・I/O の失敗）なら `true`。無い・空・保存できない置き場所なら `false`。**画面が「保存なし」と「壊れたファイルが在る」を見分けるため** |
/// | `next_port` | 次の起動で使う番号（`PERSONACORE_PORT` が読めない綴りなら `null` —— 次の起動は止まる） |
/// | `next_port_source` | その出どころ |
/// | `editable` | 保存できるか（既定の置き場所か。`GET /admin/upstream/profiles` の同名の欄と同じ意味） |
///
/// # 欄名を `listening_port_*` にした理由（作業単位 F-11b）
///
/// F-12 の欄は**何についての話か**を頭に置いている——`listening_port_*` は**いまの待ち受け**、`saved_port_*`
/// は保存、`next_port_*` は次の起動。**元の番号・ずらしたか・飛ばした番号は、どれもいまの待ち受けをどう開けたか**
/// の話なので、`listening_port_source` と同じ頭に揃えた。`original_port` / `shifted` のような裸の名前にすると、
/// 保存や次の起動の話と読み分けられない（「保存した番号から何かをずらしたのか」と読める）。
/// **`listening_port_shifted` を持つのは、元の番号と実際の番号の比較では `0` の起動を見分けられないから**
/// （元の番号 `0`・実際の番号は OS が選んだ番号で、ずらしてはいない）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct ServerView {
    listening_port: u16,
    #[schema(schema_with = port_source_schema)]
    listening_port_source: &'static str,
    listening_port_requested: u16,
    listening_port_shifted: bool,
    listening_port_skipped: Vec<u16>,
    #[schema(required = true)]
    saved_port: Option<u16>,
    saved_port_unreadable: bool,
    #[schema(required = true)]
    next_port: Option<u16>,
    #[schema(schema_with = port_source_schema)]
    next_port_source: &'static str,
    editable: bool,
}

/// **`PUT /admin/server/port` の本文**——`GET /admin/server` と同じ形に1欄を足す。
///
/// `saved_port_in_use` は**保存した番号を、いま他のプログラムが使っているか**（[`port_in_use`]）。
/// **いまこの personacore が使っている番号なら `false`。保存を消した依頼（`null`）なら `null`。**
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct SavePortView {
    #[serde(flatten)]
    server: ServerView,
    #[schema(required = true)]
    saved_port_in_use: Option<bool>,
}

/// **保存の依頼。** `deny_unknown_fields`（`/admin/` の全入口で共通の方針）。
///
/// **`port` は省けない**（`null` が「消す」）——省いた本文を「消す」と読むと、本文を書き忘れた依頼が
/// 保存を黙って消す。**整数で受けてから範囲を見る**（`u16` で読むと、範囲の外が本文の誤りの 400
/// `invalid_body` になり、理由（`invalid_listening_port`）が届かない）。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavePortRequest {
    #[serde(deserialize_with = "Option::deserialize")]
    #[schema(required = true, minimum = 1024, maximum = 65535)]
    port: Option<i64>,
}

/// `GET /admin/server` — **待ち受けのポートの、いま・保存・次の起動**（ADR-0057 §決定6）。
///
/// **失敗しない**（保存が読めなければ `saved_port` が `null`、理由はログ）。
#[utoipa::path(
    get,
    path = "/admin/server",
    summary = "Get the listening port, its source and the saved port",
    tag = "server",
    responses(
        (status = 200, body = ServerView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn get_server(State(state): State<AppState>) -> Json<ServerView> {
    Json(state.listening().view())
}

/// `PUT /admin/server/port` — **待ち受けのポートを保存する・保存を消す**（ADR-0057 §決定5・6）。
///
/// **本文**: `{ "port": <1024〜65535 | null> }`。**次の起動から効く**（いまの待ち受けは移さない）。
///
/// | 応答 | いつ |
/// |---|---|
/// | 200 | 保存した・消した（`GET /admin/server` と同じ形 ＋ `saved_port_in_use`） |
/// | 400 `invalid_body` | 本文が読めない・`port` が無い・知らない欄・整数でない |
/// | 400 `invalid_listening_port` | 1024〜65535 の外 |
/// | 409 `listening_port_read_only` | 保存できない置き場所 |
/// | 500 `listening_port_write_failed` | 書けなかった・消せなかった |
#[utoipa::path(
    put,
    path = "/admin/server/port",
    summary = "Save the listening port for the next start, or clear it",
    tag = "server",
    request_body = SavePortRequest,
    responses(
        (status = 200, body = SavePortView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn save_listening_port(
    State(state): State<AppState>,
    body: Result<Json<SavePortRequest>, JsonRejection>,
) -> Result<Json<SavePortView>, ApiError> {
    let Json(request) =
        body.map_err(|rejection| ApiError::from_json_rejection(rejection, ENDPOINT))?;
    state.listening().save(request).map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saved_port_sits_beside_the_profiles_file() {
        let profiles = Path::new("root").join("personacore").join("upstreams.json");
        assert_eq!(
            saved_port_path_beside(&profiles),
            Path::new("root")
                .join("personacore")
                .join("listening-port.txt")
        );
        // **親の無い綴り**はカレントの下（覚えた名前と同じ）。
        assert_eq!(
            saved_port_path_beside(Path::new("upstreams.json")),
            Path::new(".").join("listening-port.txt")
        );
    }

    /// **読み・書くのは既定の置き場所だけ**（`PERSONACORE_UPSTREAMS` が指す先では読まない・書かない）。
    #[test]
    fn only_the_editable_location_keeps_a_saved_port() {
        let path = Path::new("dir").join("upstreams.json");
        assert_eq!(
            memory_for(&ProfilesLocation::Editable(path.clone())),
            Some(Path::new("dir").join("listening-port.txt"))
        );
        assert_eq!(memory_for(&ProfilesLocation::ReadOnly(path)), None);
        assert_eq!(memory_for(&ProfilesLocation::None), None);
    }

    fn parsed(text: &str) -> Option<u16> {
        match parse_saved_port(text) {
            Ok(Some(port)) => Some(port.get()),
            other => panic!("{text:?} を番号として読めていない: {other:?}"),
        }
    }

    /// **範囲の両端の内側は読む**（1024 / 65535）。BOM・CRLF・前後の空白も吸う。
    #[test]
    fn a_port_from_1024_to_65535_is_read() {
        assert_eq!(parsed("1024\n"), Some(1024));
        assert_eq!(parsed("65535"), Some(65535));
        assert_eq!(parsed("8090\n"), Some(8090));
        assert_eq!(parsed("\u{feff}  8090 \r\n\r\n"), Some(8090));
    }

    /// **空なら「保存なし」**（読めない、ではない）。
    #[test]
    fn an_empty_file_saves_nothing() {
        for text in ["", "\n", "  \r\n", "\u{feff}", "\u{feff}\r\n"] {
            assert!(
                matches!(parse_saved_port(text), Ok(None)),
                "{text:?} を番号として読んでいる"
            );
        }
    }

    /// **範囲の両端の外側と `0` は読まない**（1023 / 65536 / 0。桁あふれも）。
    #[test]
    fn a_port_outside_1024_to_65535_is_not_read() {
        for text in ["1023", "0", "65536", "99999999999999999999999999", "00000"] {
            assert!(
                matches!(parse_saved_port(text), Err(SavedPortUnreadable::OutOfRange)),
                "{text:?} を範囲の外として断っていない: {:?}",
                parse_saved_port(text)
            );
        }
    }

    /// **数（半角の 0〜9 だけ）でなければ読まない**（符号・小数・空白入り・全角も）。
    #[test]
    fn a_value_that_is_not_a_number_is_not_read() {
        for text in [
            "abc",
            "80a",
            "+8090",
            "-1",
            "8090.0",
            "8 090",
            "0x1F90",
            "８０９０",
        ] {
            assert!(
                matches!(parse_saved_port(text), Err(SavedPortUnreadable::NotANumber)),
                "{text:?} を数でないとして断っていない: {:?}",
                parse_saved_port(text)
            );
        }
    }

    /// **2行以上は推測しない**（どちらの行を採っても、利用者の意図と違いうる）。
    #[test]
    fn more_than_one_line_is_not_read() {
        for text in ["8090\n8091\n", "8090\r\n8091", "8090\r8091", "8090\n\nabc"] {
            assert!(
                matches!(parse_saved_port(text), Err(SavedPortUnreadable::NotOneLine)),
                "{text:?} を1行として読んでいる: {:?}",
                parse_saved_port(text)
            );
        }
    }

    /// **書く形は番号1行（LF）。読み戻すと同じ番号。**
    #[test]
    fn the_written_text_reads_back_as_the_same_port() {
        for port in [1024_u16, 8090, 65535] {
            let saved = SavedPort::new(port).expect("範囲の中であること");
            let text = saved_port_text(saved);
            assert_eq!(text, format!("{port}\n"));
            assert_eq!(parsed(&text), Some(port));
        }
    }

    #[test]
    fn a_missing_file_saves_nothing_and_a_written_one_reads_back_and_can_be_forgotten() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let path = dir.path().join(LISTENING_PORT_FILE_NAME);
        assert!(matches!(read_saved_port(&path), Ok(None)));
        // **無いものを消しても失敗しない。**
        forget_saved_port(&path).expect("無いファイルを消しても失敗しないこと");

        let first = SavedPort::new(8090).expect("範囲の中");
        save_port(&path, first).expect("書けること");
        assert_eq!(std::fs::read(&path).expect("読めること"), b"8090\n");
        assert_eq!(read_saved_port(&path).ok().flatten(), Some(first));

        // **置き換える**（前の番号を残さない）。一時ファイルも残さない。
        let second = SavedPort::new(9101).expect("範囲の中");
        save_port(&path, second).expect("書けること");
        assert_eq!(std::fs::read(&path).expect("読めること"), b"9101\n");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("読めること")
            .flatten()
            .map(|entry| entry.file_name())
            .filter(|name| name != LISTENING_PORT_FILE_NAME)
            .collect();
        assert!(
            leftovers.is_empty(),
            "一時ファイルが残っている: {leftovers:?}"
        );

        forget_saved_port(&path).expect("消せること");
        assert!(!path.exists(), "消したのにファイルが残っている");
        assert!(matches!(read_saved_port(&path), Ok(None)));
    }

    /// **その名前がディレクトリなら、読めない・書けない・消せない。どれも panic しない。**
    #[test]
    fn a_directory_in_its_place_is_unreadable_unwritable_and_unremovable() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let path = dir.path().join(LISTENING_PORT_FILE_NAME);
        std::fs::create_dir(&path).expect("置けること");

        assert!(matches!(
            read_saved_port(&path),
            Err(SavedPortUnreadable::Io(_))
        ));
        let port = SavedPort::new(8090).expect("範囲の中");
        assert!(matches!(
            save_port(&path, port),
            Err(SavePortError::Write(_))
        ));
        assert!(matches!(
            forget_saved_port(&path),
            Err(SavePortError::Remove(_))
        ));
        assert!(path.is_dir(), "置いたディレクトリが消えている");
    }

    /// **他のリスナが掴んでいる番号は「使われている」、放した番号は「使われていない」。**
    ///
    /// 同じ番号で両方を見る（掴んでいる間は真、放したら偽）——いつも真・いつも偽の実装を両方とも赤にする。
    #[test]
    fn a_port_held_by_another_listener_is_in_use_and_a_released_one_is_not() {
        let held = std::net::TcpListener::bind("127.0.0.1:0").expect("ポートを取れること");
        let port = held.local_addr().expect("宛先を読めること").port();
        assert!(
            port_in_use(port),
            "掴まれている {port} を空いていると言った"
        );
        drop(held);
        assert!(!port_in_use(port), "放した {port} を使われていると言った");
    }
}
