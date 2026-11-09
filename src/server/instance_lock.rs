//! **同じデータを使う personacore を2つ動かさない**（[ADR-0057](../../docs/adr/0057-the-listening-port-shifts-when-taken-and-is-set-from-the-screen.md)
//! §決定2 / 作業単位 F-11a / marukei/personacore#14）。
//!
//! ポートの重なりは二重起動を止める仕組みではない（ADR-0057 §決定1・3 でずらすようになると、なおさら止めない）。
//! そこで、**DB と同じディレクトリのロックのファイルを OS のファイルロックで排他的に取り**、
//! 取れなかった起動は、何もせずに動いている側を案内して終わる。
//!
//! ```text
//! <DB のディレクトリ>/
//!   personacore.lock     … OS のロックを掛けるだけのファイル（中身は書かない）
//!   personacore.running  … 動いている側が、待ち受けを開いた後に書く2行（URL と画面の有無）
//! ```
//!
//! # 起動の順（`main`）
//!
//! | 順 | すること |
//! |---|---|
//! | 1 | 引数を読む。`--version` ならここで終わる（**ロックを見ない**） |
//! | 2 | ログを初期化する（ファイルに触らない） |
//! | 3 | **DB の置き場所を [`super::config::database_path_from_env`] で決め、[`acquire_beside`] でロックを取る** |
//! | 4 | 取れなければ [`guide_second_start`] で案内して 0 で終わる。**見本も書かず、`--fresh` の退避もせず、`--upstream` も読まない** |
//! | 5 | 取れたら、見本・プロファイル・設定・`--fresh` の退避・待ち受け（今までどおり） |
//! | 6 | 待ち受けを開いた後に、`App::run_with_upstreams` が [`InstanceLock::announce`] で URL を書く |
//! | 7 | `serve` が返ったら手放す（手放す前に中身を消す）。**異常終了でも OS がロックを外す** |
//!
//! # 決めたこと
//!
//! * **ロックの手段は標準ライブラリの [`File::try_lock`]**（Rust 1.89 で安定化。依存を足さない）。
//!   Windows では `LockFileEx`（排他・即時に失敗）、Unix では `flock` になる。
//!   どちらもプロセスが終われば OS が外す（`tests/instance_lock_cli.rs` の `a_killed_first_leaves_no_lock_behind`）。
//! * **URL はロックのファイルとは別のファイルに書く。** Windows の排他ロックは、他のハンドルからの
//!   **読み取りも止める**（`the_locked_file_itself_is_unreadable_on_windows_so_the_url_lives_beside_it` が実測で固定）。
//! * **中身は2行・LF 終わりの厳密な形**（[`render_running`]）。それ以外は全部「起動の途中」と読む
//!   （[`parse_running`]）。**相手の書きかけを読んでも**、2行目が改行まで揃うまでは読めない形なので、
//!   書き手は置き換え（一時ファイル → 改名）をせず、そのまま書く。
//! * **URL は読んだ文字列を使わない。** 読んだ番号から [`super::browser::ui_url`] で組み直す——
//!   Windows ではその URL を `cmd /C start` へ渡すので、ファイルの中身（`&` 等）を `cmd` に届かせない。
//! * **ロックを取ったら、まず前の起動の中身を消す**（異常終了の残り）。**手放すときも消す**（[`Drop`]）。
//!
//! # ⚠ 守れないもの
//!
//! * **ロックを取ってから古い中身を消すまでの一瞬**（同じプロセスの次の1行）に2つ目が読むと、
//!   **異常終了した前の起動の URL**を言いうる。前の起動が異常終了していて、かつ2つの起動がその一瞬に重なったときだけ。
//! * **ロックはディレクトリに1つ。** `PERSONACORE_DB_PATH` で**同じディレクトリの別の名前の DB** を指した2つは、
//!   同じロックで止まる（ADR-0057 §決定2 の「データのディレクトリ」のとおり）。
//! * **開いたブラウザが実際に画面を出したか**は分からない（`browser::open` は開くコマンドを起こせたかだけを返す）。

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read as _, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use super::fresh::{Args, FRESH_FLAG, UPSTREAM_FLAG};
use super::{Ui, browser};

/// **OS のロックを掛けるファイルの名前**（DB と同じディレクトリ）。**中身は書かない。**
pub const LOCK_FILE_NAME: &str = "personacore.lock";

/// **動いている側が、待ち受けを開いた後に URL と画面の有無を書くファイルの名前**（DB と同じディレクトリ）。
pub const RUNNING_FILE_NAME: &str = "personacore.running";

/// **読む上限（バイト）。** 正しい中身は 50 バイトに満たない。これを超えたら「起動の途中」と読む。
const MAX_RUNNING_BYTES: u64 = 256;

/// 中身の1行目の頭。
const URL_KEY: &str = "url=";
/// 中身の2行目の頭。
const UI_KEY: &str = "ui=";
/// 画面を配っているときの2行目の値。
const UI_SERVE: &str = "serve";
/// 画面を配っていない（`--no-ui`）ときの2行目の値。
const UI_WITHHOLD: &str = "withhold";
/// 読める URL の頭（**ループバックだけ**）。尻尾は `/`。
const URL_PREFIX: &str = "http://127.0.0.1:";

/// **2つ目が「動いている」と言う文の前半**（間に URL が入る）。
///
/// **写しが `tests/instance_lock_cli.rs` の `RUNNING_HEAD` / `RUNNING_TAIL` に在る**
/// （文面を変えたら、そちらは「言っていない」で赤になる）。
const RUNNING_HEAD: &str = "personacore はすでに ";
/// 同じ文の後半。
const RUNNING_TAIL: &str = " で動いています";

/// **動いている側が、まだ URL を書いていない**（起動の途中）ときの文。
const STARTING: &str = "personacore は起動の途中です（同じデータを使う personacore が、まだ待ち受けを始めていません。少し待ってから、もう一度起動してください）";

/// **使わなかった旗を言う文の前半**（間に旗の名前が入る）。
///
/// **写しが `tests/instance_lock_cli.rs` の `NOT_USED_HEAD` / `NOT_USED_TAIL` に在る。**
const NOT_USED_HEAD: &str = "渡された ";
/// 同じ文の後半。
const NOT_USED_TAIL: &str = " は使っていません（動いている personacore には何もしていません）";

/// **相手が画面を配っていないので、開く設定の2つ目でも開かない**ときの文。
///
/// **写しが `tests/instance_lock_cli.rs` の `WITHHELD_LINE` に在る。**
const WITHHELD: &str = "動いている personacore は画面を配っていないので、ブラウザは開きません（--no-ui で起動しています）";

/// **2つ目から見た、動いている側**（[`RUNNING_FILE_NAME`] を読んだ結果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtherInstance {
    /// **待ち受けている。** 宛先は読んだ番号から組み直したもの（ループバックだけ）。
    Listening {
        /// 実際の宛先（`local_addr`）。
        addr: SocketAddr,
        /// 画面を配っているか（`--no-ui` でないか）。
        ui: Ui,
    },
    /// **起動の途中**——まだ書いていない。読めない・空・壊れている・書きかけも、ここに倒す。
    Starting,
}

/// **持っているロック。** 落とすと（プロセスが終わっても）手放す。
///
/// **`lock` は開いたままにしておくことに意味がある**——閉じると OS がロックを外す。
#[derive(Debug)]
pub struct InstanceLock {
    lock: File,
    running_file: PathBuf,
}

/// **ロックを取りに行った結果。**
#[derive(Debug)]
pub enum Acquired {
    /// **取れた。** 同じデータを使う personacore は、ほかに動いていない。
    Held(InstanceLock),
    /// **取れなかった。** 同じデータを使う personacore が動いている（その案内のための中身）。
    Taken(OtherInstance),
}

/// **ロックを取れたかどうかを確かめられない**（取れなかった、ではない）。
///
/// `Display` は定数文字列。置き場所は呼び出し側（`main`）が文脈に足す。
#[derive(Debug, thiserror::Error)]
pub enum InstanceLockError {
    /// DB のディレクトリを作れない（初回はまだ無い）。
    #[error("データのディレクトリを作れませんでした")]
    Dir(#[source] std::io::Error),
    /// ロックのファイルを開けない。
    #[error("ロックのファイルを開けませんでした")]
    Open(#[source] std::io::Error),
    /// OS がロックの問い合わせに答えない（このファイルシステムでは使えない等）。
    #[error("ロックのファイルを確かめられませんでした")]
    Lock(#[source] std::io::Error),
}

/// **ロックを置くディレクトリ**（DB と同じ）。**純粋関数。**
///
/// **親の無い相対パス**（`personacore.sqlite` だけ）は `.`——SQLite が開くのと同じカレントディレクトリ。
#[must_use]
pub fn lock_dir_of(database_path: &Path) -> PathBuf {
    match database_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// **DB と同じディレクトリのロックを、排他的に取りに行く**（ADR-0057 §決定2）。**待たない。**
///
/// * 取れた → [`Acquired::Held`]。**前の起動が残した中身を消してから返す。**
/// * 誰かが持っている → [`Acquired::Taken`]（隣のファイルを読んだ結果）。
/// * 確かめられない → [`InstanceLockError`]（`main` は起動を止める——二重起動を黙って許さない）。
pub fn acquire_beside(database_path: &Path) -> Result<Acquired, InstanceLockError> {
    let dir = lock_dir_of(database_path);
    std::fs::create_dir_all(&dir).map_err(InstanceLockError::Dir)?;
    let running_file = dir.join(RUNNING_FILE_NAME);
    // **作るだけで切り詰めない**——持っている相手のファイルを消さない（中身は無いが、作法として）。
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(LOCK_FILE_NAME))
        .map_err(InstanceLockError::Open)?;
    match lock.try_lock() {
        Ok(()) => {
            let held = InstanceLock { lock, running_file };
            held.clear();
            Ok(Acquired::Held(held))
        }
        Err(TryLockError::WouldBlock) => Ok(Acquired::Taken(read_running(&running_file))),
        Err(TryLockError::Error(err)) => Err(InstanceLockError::Lock(err)),
    }
}

impl InstanceLock {
    /// **待ち受けを開いた後に、実際の宛先と画面の有無を隣のファイルへ書く。書けなくても止めない**（警告だけ）。
    ///
    /// **置き換え（一時ファイル → 改名）をせず、そのまま書く**——読み手は書きかけを「起動の途中」と読む
    /// （[`parse_running`]）。
    pub(crate) fn announce(&self, addr: SocketAddr, ui: Ui) {
        if let Err(err) = std::fs::write(&self.running_file, render_running(addr, ui)) {
            tracing::warn!(
                running_file = %self.running_file.display(),
                error = %err,
                "動いている URL を書けませんでした（同じデータで起こした2つ目は「起動の途中」と言います）"
            );
        }
    }

    /// **中身を消す**（取った直後と手放す前）。**消せなくても止めない**（警告だけ）。
    fn clear(&self) {
        if let Err(err) = std::fs::write(&self.running_file, b"") {
            tracing::warn!(
                running_file = %self.running_file.display(),
                error = %err,
                "動いている URL のファイルを空にできませんでした（同じデータで起こした2つ目が古い URL を言うことがあります）"
            );
        }
    }
}

impl Drop for InstanceLock {
    /// **ロックを手放す前に中身を消す**（このあとフィールドの `File` が閉じて、OS がロックを外す）。
    fn drop(&mut self) {
        self.clear();
        // 明示して外す（閉じれば外れるが、外れる時点を `clear` の直後に揃える）。失敗しても閉じれば外れる。
        let _ = self.lock.unlock();
    }
}

/// **隣のファイルに書く中身。純粋関数。** 2行・LF 終わり:
///
/// ```text
/// url=http://127.0.0.1:54321/
/// ui=serve        （--no-ui なら ui=withhold）
/// ```
fn render_running(addr: SocketAddr, ui: Ui) -> String {
    let ui = match ui {
        Ui::Serve => UI_SERVE,
        Ui::Withhold => UI_WITHHOLD,
    };
    format!("{URL_KEY}{}\n{UI_KEY}{ui}\n", browser::ui_url(addr))
}

/// **隣のファイルの中身を読む。純粋関数。panic しない。**
///
/// **[`render_running`] が書く形にぴったり合うときだけ [`OtherInstance::Listening`]。**
/// それ以外（空・1行だけ・最後の改行が無い・3行目・CRLF・知らない値・ループバック以外・番号が 0 や範囲の外）は
/// 全部 [`OtherInstance::Starting`]——**書きかけを読んでも、どの長さの頭も読めない。**
fn parse_running(text: &str) -> OtherInstance {
    let Some(body) = text.strip_suffix('\n') else {
        return OtherInstance::Starting;
    };
    let mut lines = body.split('\n');
    let (Some(url_line), Some(ui_line), None) = (lines.next(), lines.next(), lines.next()) else {
        return OtherInstance::Starting;
    };
    let Some(addr) = url_line.strip_prefix(URL_KEY).and_then(parse_ui_url) else {
        return OtherInstance::Starting;
    };
    let ui = match ui_line.strip_prefix(UI_KEY) {
        Some(UI_SERVE) => Ui::Serve,
        Some(UI_WITHHOLD) => Ui::Withhold,
        _ => return OtherInstance::Starting,
    };
    OtherInstance::Listening { addr, ui }
}

/// **`http://127.0.0.1:<番号>/` だけを読む。純粋関数。** 番号は 1〜5 桁の ASCII の数字で、1〜65535。
fn parse_ui_url(url: &str) -> Option<SocketAddr> {
    let digits = url.strip_prefix(URL_PREFIX)?.strip_suffix('/')?;
    if digits.is_empty() || digits.len() > 5 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let port: u16 = digits.parse().ok()?;
    (port != 0).then(|| SocketAddr::from(([127, 0, 0, 1], port)))
}

/// **隣のファイルを読む。読めなければ「起動の途中」**（無い・ディレクトリ・大きすぎる・UTF-8 でない）。
fn read_running(path: &Path) -> OtherInstance {
    let Ok(file) = File::open(path) else {
        return OtherInstance::Starting;
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_RUNNING_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_RUNNING_BYTES
    {
        return OtherInstance::Starting;
    }
    String::from_utf8(bytes).map_or(OtherInstance::Starting, |text| parse_running(&text))
}

/// **2つ目が言うことと、開く URL。**
#[derive(Debug, Clone, PartialEq, Eq)]
struct Guidance {
    /// 標準出力へ1行ずつ出す文。
    lines: Vec<String>,
    /// **開く URL**（開かないなら `None`）。
    open: Option<String>,
}

/// **2つ目が何を言い、何を開くか。純粋関数**（ADR-0057 §決定2）。
///
/// | 相手 | 2つ目 | 言うこと | 開くか |
/// |---|---|---|---|
/// | 待ち受け・画面あり | 開く設定（`--no-browser` も `--no-ui` も無い） | URL | **開く** |
/// | 待ち受け・画面あり | `--no-browser` か `--no-ui` | URL | 開かない |
/// | 待ち受け・`--no-ui` | 開く設定 | URL ＋ 画面を配っていない | 開かない |
/// | 待ち受け・`--no-ui` | 開かない設定 | URL | 開かない |
/// | 起動の途中 | どれでも | 起動の途中 | 開かない |
///
/// **`--fresh` / `--upstream` が渡されていたら、使わなかったと言う**（どの行でも）。
fn guidance_for(other: OtherInstance, args: &Args) -> Guidance {
    let would_open = !args.no_browser() && !args.no_ui();
    let mut lines = Vec::new();
    let mut open = None;
    let mut withheld = false;
    match other {
        OtherInstance::Listening { addr, ui } => {
            let url = browser::ui_url(addr);
            lines.push(format!("{RUNNING_HEAD}{url}{RUNNING_TAIL}"));
            match (ui, would_open) {
                (Ui::Serve, true) => open = Some(url),
                (Ui::Withhold, true) => withheld = true,
                (_, false) => {}
            }
        }
        OtherInstance::Starting => lines.push(STARTING.to_owned()),
    }
    let not_used: Vec<&str> = [
        (args.fresh(), FRESH_FLAG),
        (args.upstream().is_some(), UPSTREAM_FLAG),
    ]
    .into_iter()
    .filter_map(|(given, flag)| given.then_some(flag))
    .collect();
    if !not_used.is_empty() {
        lines.push(format!(
            "{NOT_USED_HEAD}{}{NOT_USED_TAIL}",
            not_used.join("、")
        ));
    }
    if withheld {
        lines.push(WITHHELD.to_owned());
    }
    Guidance { lines, open }
}

/// **窓を閉じる前に Enter を待つ候補か。純粋関数。** 開くと言って、実際に開けたときだけ待たない。
///
/// **本当に待つかは B-76 の判定**（`console::wait_before_closing_if_alone`。ダブルクリックの窓だけ）。
fn waits_for_enter(guidance: &Guidance, opened: bool) -> bool {
    !(guidance.open.is_some() && opened)
}

/// **取れなかった2つ目の案内**（ADR-0057 §決定2）。**戻り値は「窓を閉じる前に Enter を待つ候補か」。**
///
/// 文は**標準出力へそのまま**書く（`tracing` を通さない——`RUST_LOG` で黙らせると、ダブルクリックの窓に
/// 何も出ずに閉じる）。書けなくても止めない（`println!` を使わない）。
///
/// **tokio の実行時の中で呼ぶこと**——ブラウザを開くのに [`browser::open`] を使う（その中で `tokio::spawn`）。
pub fn guide_second_start(other: OtherInstance, args: &Args) -> bool {
    let guidance = guidance_for(other, args);
    {
        let mut stdout = std::io::stdout().lock();
        for line in &guidance.lines {
            if writeln!(stdout, "{line}").is_err() {
                break;
            }
        }
        let _ = stdout.flush();
    }
    let opened = guidance.open.as_deref().is_some_and(browser::open);
    waits_for_enter(&guidance, opened)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::fresh::parse_args;

    fn args(raw: &[&str]) -> Args {
        parse_args(raw.iter().copied()).expect("引数として読めること")
    }

    fn at(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    // ------------------------------------------------------------------
    // 中身の形（書く・読む）
    // ------------------------------------------------------------------

    /// **書く形を綴りで固定する**（期待値を `render_running` から作らない）。
    #[test]
    fn the_running_file_says_the_url_and_whether_the_screen_is_served_on_two_lines() {
        assert_eq!(
            render_running(at(54_321), Ui::Serve),
            "url=http://127.0.0.1:54321/\nui=serve\n"
        );
        assert_eq!(
            render_running(at(8081), Ui::Withhold),
            "url=http://127.0.0.1:8081/\nui=withhold\n"
        );
    }

    /// **書いたものは、そのまま読める**（両方の `ui`・番号の両端）。
    #[test]
    fn what_is_written_reads_back() {
        for port in [1_u16, 8080, 65_535] {
            for ui in [Ui::Serve, Ui::Withhold] {
                assert_eq!(
                    parse_running(&render_running(at(port), ui)),
                    OtherInstance::Listening { addr: at(port), ui },
                    "{port} / {ui:?}"
                );
            }
        }
    }

    /// **読めない・空・壊れている・書きかけは、どれも「起動の途中」**（ADR-0057 §決定2）。
    ///
    /// **書きかけ**（相手が書いている途中を読んだ）は、正しい中身の**どの長さの頭**でも読めないことを
    /// 全部の切り口で見る——2行目が言い終わって改行まで来たときだけ読める。
    #[test]
    fn anything_but_the_exact_two_lines_reads_as_starting() {
        let cases = [
            ("", "空（ロックを取った直後に消した）"),
            ("\n", "改行だけ"),
            ("url=http://127.0.0.1:8081/\n", "1行目だけ"),
            ("url=http://127.0.0.1:8081/\nui=serve", "最後の改行が無い"),
            (
                "url=http://127.0.0.1:8081/\nui=serve\nextra\n",
                "3行目がある",
            ),
            (
                "url=http://127.0.0.1:8081/\nui=serve\n\n",
                "空の3行目がある",
            ),
            ("ui=serve\nurl=http://127.0.0.1:8081/\n", "行の順が逆"),
            (
                "url=http://127.0.0.1:8081/\r\nui=serve\r\n",
                "CRLF（書くのは LF だけ）",
            ),
            ("url=http://127.0.0.1:8081/\nui=both\n", "知らない ui"),
            ("url=http://127.0.0.1:8081/\nui=\n", "空の ui"),
            ("url=http://127.0.0.1:8081/\nui=Serve\n", "ui の大文字"),
            ("url=http://127.0.0.1:8081/\n ui=serve\n", "ui の前の空白"),
            ("url=http://localhost:8081/\nui=serve\n", "127.0.0.1 でない"),
            ("url=http://0.0.0.0:8081/\nui=serve\n", "全部のアドレス"),
            ("url=https://127.0.0.1:8081/\nui=serve\n", "https"),
            ("url=http://127.0.0.1:0/\nui=serve\n", "番号 0"),
            ("url=http://127.0.0.1:65536/\nui=serve\n", "番号が範囲の外"),
            ("url=http://127.0.0.1:080811/\nui=serve\n", "6桁"),
            ("url=http://127.0.0.1:/\nui=serve\n", "番号が空"),
            ("url=http://127.0.0.1:+8081/\nui=serve\n", "符号"),
            ("url=http://127.0.0.1:8081\nui=serve\n", "最後の / が無い"),
            ("url=http://127.0.0.1:8081/x\nui=serve\n", "パスがある"),
            (
                "url=http://127.0.0.1:8081/&calc\nui=serve\n",
                "cmd が解釈する字",
            ),
            ("url=http://127.0.0.1:8081/ \nui=serve\n", "後ろの空白"),
            ("URL=http://127.0.0.1:8081/\nui=serve\n", "鍵の大文字"),
            ("url= http://127.0.0.1:8081/\nui=serve\n", "値の前の空白"),
            ("url=http://127.0.0.1:８０８１/\nui=serve\n", "全角の数字"),
        ];
        for (text, what) in cases {
            assert_eq!(
                parse_running(text),
                OtherInstance::Starting,
                "{what}: {text:?}"
            );
        }

        let whole = render_running(at(8081), Ui::Withhold);
        for cut in 0..whole.len() {
            assert_eq!(
                parse_running(&whole[..cut]),
                OtherInstance::Starting,
                "書きかけ（{cut} バイトまで）: {:?}",
                &whole[..cut]
            );
        }
    }

    /// **ファイルが無い・大きすぎる・UTF-8 でない・ディレクトリも「起動の途中」。**
    #[test]
    fn an_unreadable_running_file_reads_as_starting() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let missing = dir.path().join("missing");
        assert_eq!(read_running(&missing), OtherInstance::Starting, "無い");

        let large = dir.path().join("large");
        let mut text = render_running(at(8081), Ui::Serve);
        text.push_str(&"x".repeat(4096));
        std::fs::write(&large, text).expect("書けること");
        assert_eq!(read_running(&large), OtherInstance::Starting, "大きすぎる");

        let not_utf8 = dir.path().join("not-utf8");
        std::fs::write(&not_utf8, b"url=http://127.0.0.1:8081/\nui=\xff\n").expect("書けること");
        assert_eq!(
            read_running(&not_utf8),
            OtherInstance::Starting,
            "UTF-8 でない"
        );

        assert_eq!(
            read_running(dir.path()),
            OtherInstance::Starting,
            "ディレクトリ"
        );

        // 対照: 同じ道具で、正しい中身は読める（上の「起動の途中」が読み方の故障から来ていない）。
        let good = dir.path().join("good");
        std::fs::write(&good, render_running(at(8081), Ui::Serve)).expect("書けること");
        assert_eq!(
            read_running(&good),
            OtherInstance::Listening {
                addr: at(8081),
                ui: Ui::Serve
            }
        );
    }

    // ------------------------------------------------------------------
    // ロックを置く場所
    // ------------------------------------------------------------------

    /// **ロックは DB と同じディレクトリ。** 親の無い相対パスはカレントディレクトリ（SQLite が開く場所と同じ）。
    #[test]
    fn the_lock_lives_in_the_directory_of_the_database() {
        assert_eq!(
            lock_dir_of(Path::new("data").join("a").join("db.sqlite").as_path()),
            Path::new("data").join("a")
        );
        assert_eq!(lock_dir_of(Path::new("db.sqlite")), Path::new("."));
    }

    // ------------------------------------------------------------------
    // 本物のロック
    // ------------------------------------------------------------------

    /// **1つ目が持っている間、2つ目は取れない。URL を書く前は「起動の途中」、書いた後は URL を読む。
    /// 1つ目が手放せば、また取れる。**
    ///
    /// 同じプロセスの別のハンドルでも、OS のロックは取れない（Windows の `LockFileEx` はハンドルごと、
    /// Unix の `flock` は開いたファイルごと）。
    #[test]
    fn only_one_holds_the_lock_and_the_other_reads_what_it_announced() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let database = dir.path().join("nested").join("personacore.sqlite");

        let Acquired::Held(first) = acquire_beside(&database).expect("取れること") else {
            panic!("誰も持っていないのに取れなかった");
        };
        assert!(
            dir.path().join("nested").join(LOCK_FILE_NAME).is_file(),
            "ロックのファイルが DB と同じディレクトリに無い"
        );
        match acquire_beside(&database).expect("確かめられること") {
            Acquired::Taken(other) => assert_eq!(other, OtherInstance::Starting, "書く前"),
            Acquired::Held(_) => panic!("1つ目が持っているのに、2つ目も取れた"),
        }

        first.announce(at(54_321), Ui::Withhold);
        match acquire_beside(&database).expect("確かめられること") {
            Acquired::Taken(other) => assert_eq!(
                other,
                OtherInstance::Listening {
                    addr: at(54_321),
                    ui: Ui::Withhold
                },
                "書いた後"
            ),
            Acquired::Held(_) => panic!("1つ目が持っているのに、2つ目も取れた"),
        }

        drop(first);
        assert!(
            matches!(acquire_beside(&database), Ok(Acquired::Held(_))),
            "手放した後に取れない"
        );
    }

    /// **前の起動が残した中身（異常終了）は、ロックを取った時点で消す**——2つ目が古い URL を言わない。
    /// **手放すときも消す。**
    #[test]
    fn a_stale_announcement_is_cleared_when_the_lock_is_taken_and_when_it_is_released() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let database = dir.path().join("personacore.sqlite");
        let running = dir.path().join(RUNNING_FILE_NAME);
        std::fs::write(&running, render_running(at(40_000), Ui::Serve)).expect("書けること");

        let Acquired::Held(lock) = acquire_beside(&database).expect("取れること") else {
            panic!("誰も持っていないのに取れなかった");
        };
        assert_eq!(
            std::fs::read_to_string(&running).expect("読めること"),
            "",
            "取った時点で古い中身を消していない"
        );
        lock.announce(at(40_001), Ui::Serve);
        assert_eq!(
            std::fs::read_to_string(&running).expect("読めること"),
            "url=http://127.0.0.1:40001/\nui=serve\n"
        );
        drop(lock);
        assert_eq!(
            std::fs::read_to_string(&running).expect("読めること"),
            "",
            "手放すときに中身を消していない"
        );
    }

    /// **⚠ ロックのファイルそのものに書いた中身は、Windows では他のハンドルから読めない**
    /// （`LockFileEx` の排他ロックは読み取りも止める）。**だから URL は隣の別のファイルに書く。**
    ///
    /// Unix の `flock` は助言的で、読める（ここでは決めつけず、Windows でだけ「読めない」を主張する）。
    #[test]
    fn the_locked_file_itself_is_unreadable_on_windows_so_the_url_lives_beside_it() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let database = dir.path().join("personacore.sqlite");
        let Acquired::Held(lock) = acquire_beside(&database).expect("取れること") else {
            panic!("誰も持っていないのに取れなかった");
        };
        (&lock.lock)
            .write_all(b"written through the lock")
            .expect("持っているハンドルからは書けること");
        let read = std::fs::read(dir.path().join(LOCK_FILE_NAME));
        if cfg!(windows) {
            assert!(
                read.is_err(),
                "Windows でロック中のファイルが読めた: {read:?}"
            );
        }
        lock.announce(at(40_002), Ui::Serve);
        assert_eq!(
            read_running(&dir.path().join(RUNNING_FILE_NAME)),
            OtherInstance::Listening {
                addr: at(40_002),
                ui: Ui::Serve
            },
            "隣のファイルは、ロック中でも読めること"
        );
    }

    // ------------------------------------------------------------------
    // 2つ目の案内
    // ------------------------------------------------------------------

    fn running_line(port: u16) -> String {
        format!("personacore はすでに http://127.0.0.1:{port}/ で動いています")
    }

    /// **相手が画面を配り、2つ目が開く設定なら、URL を言ってその URL を開く。** 開いたら待たない。
    #[test]
    fn a_second_start_opens_the_served_screen_when_it_would_have_opened_one() {
        let guidance = guidance_for(
            OtherInstance::Listening {
                addr: at(8081),
                ui: Ui::Serve,
            },
            &args(&[]),
        );
        assert_eq!(
            guidance,
            Guidance {
                lines: vec![running_line(8081)],
                open: Some("http://127.0.0.1:8081/".to_owned()),
            }
        );
        assert!(!waits_for_enter(&guidance, true), "開けたのに待つ");
        assert!(
            waits_for_enter(&guidance, false),
            "開けなかったのに待たない"
        );
    }

    /// **開かない腕は全部「待つ」**（`--no-browser`・2つ目の `--no-ui`・相手の `--no-ui`・起動の途中）。
    #[test]
    fn every_arm_that_opens_nothing_waits() {
        let served = OtherInstance::Listening {
            addr: at(8081),
            ui: Ui::Serve,
        };
        let withheld = OtherInstance::Listening {
            addr: at(8081),
            ui: Ui::Withhold,
        };
        let cases = [
            (
                served,
                args(&["--no-browser"]),
                vec![running_line(8081)],
                "--no-browser",
            ),
            (
                served,
                args(&["--no-ui"]),
                vec![running_line(8081)],
                "2つ目の --no-ui",
            ),
            (
                served,
                args(&["--no-ui", "--no-browser"]),
                vec![running_line(8081)],
                "2つ目の --no-ui --no-browser",
            ),
            (
                withheld,
                args(&[]),
                vec![running_line(8081), WITHHELD.to_owned()],
                "相手の --no-ui（2つ目は開く設定）",
            ),
            (
                withheld,
                args(&["--no-browser"]),
                vec![running_line(8081)],
                "相手の --no-ui（2つ目も開かない設定）",
            ),
            (
                OtherInstance::Starting,
                args(&[]),
                vec![STARTING.to_owned()],
                "起動の途中",
            ),
            (
                OtherInstance::Starting,
                args(&["--no-browser"]),
                vec![STARTING.to_owned()],
                "起動の途中（開かない設定）",
            ),
        ];
        for (other, given, lines, what) in cases {
            let guidance = guidance_for(other, &given);
            assert_eq!(guidance, Guidance { lines, open: None }, "{what}");
            assert!(waits_for_enter(&guidance, false), "{what}: 待たない");
            // 開いていないのだから、`opened` が何であっても待つ（開くと言っていない）。
            assert!(waits_for_enter(&guidance, true), "{what}: 待たない");
        }
    }

    /// **渡された `--fresh` / `--upstream` は使わず、使わなかったと言う**（起動の途中でも）。
    #[test]
    fn the_flags_it_did_not_use_are_named() {
        let served = OtherInstance::Listening {
            addr: at(8081),
            ui: Ui::Serve,
        };
        let not_used = |flags: &str| {
            format!(
                "渡された {flags} は使っていません（動いている personacore には何もしていません）"
            )
        };
        let cases = [
            (
                served,
                args(&["--fresh"]),
                vec![running_line(8081), not_used("--fresh")],
            ),
            (
                served,
                args(&["--upstream", "qwen"]),
                vec![running_line(8081), not_used("--upstream")],
            ),
            (
                served,
                args(&["--upstream", "qwen", "--fresh", "--no-browser"]),
                vec![running_line(8081), not_used("--fresh、--upstream")],
            ),
            (
                OtherInstance::Starting,
                args(&["--fresh"]),
                vec![STARTING.to_owned(), not_used("--fresh")],
            ),
            (
                OtherInstance::Listening {
                    addr: at(8081),
                    ui: Ui::Withhold,
                },
                args(&["--fresh"]),
                vec![running_line(8081), not_used("--fresh"), WITHHELD.to_owned()],
            ),
        ];
        for (other, given, lines) in cases {
            assert_eq!(guidance_for(other, &given).lines, lines, "{given:?}");
        }
    }

    /// **文の写し（`tests/instance_lock_cli.rs`）が当てにしている頭と尻尾が、本文と食い違っていないこと。**
    #[test]
    fn the_sentences_are_built_from_the_constants_the_tests_copy() {
        assert_eq!(
            running_line(8081),
            format!("{RUNNING_HEAD}http://127.0.0.1:8081/{RUNNING_TAIL}")
        );
        assert!(STARTING.starts_with("personacore は起動の途中です"));
        assert!(NOT_USED_HEAD == "渡された " && NOT_USED_TAIL.starts_with(" は使っていません"));
    }
}
