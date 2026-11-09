//! **画面で保存した待ち受けのポートを、実行ファイルそのもので確かめる**
//! （[ADR-0057](../docs/adr/0057-the-listening-port-shifts-when-taken-and-is-set-from-the-screen.md) §決定5・6 /
//! 作業単位 F-12a / marukei/personacore#15）。**本物の `llama` にも LM Studio にも繋がない。**
//!
//! # なぜ実行ファイルを起こすのか（`tests/upstream_cli.rs` と同じ理由）
//!
//! **保存のファイルを読む1行と、それを設定の入口へ渡す3つの経路（環境変数の既定・`--upstream`・
//! 覚えた名前）は `src/main.rs` にしか無い**（bin なので統合テストから `use` できない）。
//!
//! # 何を固定するか
//!
//! | 見張り番 | 破れると |
//! |---|---|
//! | `a_saved_port_is_where_a_bare_start_listens_and_the_api_moves_it` | 環境変数の既定の経路が保存した番号を見ない／出どころを言わない／保存が次の起動に効かない／`null` で消えない |
//! | `the_upstream_flag_path_listens_on_the_saved_port_too` | プロファイル経由の経路（`config_for`）が保存した番号を見ない |
//! | `a_remembered_profile_on_the_saved_port_is_kept_and_the_saved_port_is_skipped` | 明示していないのに、覚えた名前を番号の取り合いで捨てる／面が使う保存した番号で待ち受ける（作業単位 F-11b） |
//! | `the_environment_wins_over_the_saved_port` | 環境変数が保存に負ける／`GET /admin/server` の次の起動が環境変数を見ない |
//! | `an_overridden_profile_file_neither_reads_nor_writes_the_saved_port` | `PERSONACORE_UPSTREAMS` の起動が保存を読む・書く |
//! | `an_unreadable_saved_port_warns_and_the_file_is_kept` | 読めないファイルで止まる・黙る・ファイルを消す |
//! | `a_taken_saved_port_shifts_past_a_face_port_and_the_log_and_the_api_say_so` | 保存した番号が塞がっていると止まる／`upstreams.json` の gguf の面の番号を飛ばさない／警告・`addr=`・`bind=`・`GET /admin/server` が実際と食い違う（F-11b） |
//! | `a_whole_range_taken_names_the_range_and_how_to_get_out` | 範囲が全部塞がっているのに起きる／止まる文が範囲と直し方を言わない（F-11b） |
//! | `an_unreadable_profile_file_warns_once_and_the_start_goes_on` | 飛ばす番号のために読む `upstreams.json` が読めないと止まる・黙る／明示した起動でも読む（F-11b） |
//!
//! # ずれたときの主張（作業単位 F-11b / marukei/personacore#14 の持ち越し）
//!
//! **空いている番号を OS から取って放し、子に保存させて待ち受けさせる形には隙がある**（放してから子が
//! `bind` するまでに、他のテストがその番号を取りうる）。F-12 の間はそこで「待ち受けの行を読む前に
//! プロセスが終了した」になった。**いまは保存した番号が塞がっていれば子がずれて起きる**ので、
//! 「保存した番号で待ち受ける」腕は [`assert_shift_said`] で**「ずれたら、そう言う」**を主張する。
//!
//! # 隔離（CLAUDE.md「exe を起こすときは、DB だけでなく上流の設定も隔離する」）
//!
//! **どの腕も `LOCALAPPDATA` / `XDG_DATA_HOME` / `HOME` / `USERPROFILE` を一時ディレクトリへ向ける**
//! （[`isolate_machine`]）。保存を読む腕は **`PERSONACORE_UPSTREAMS` を渡さない**（既定の置き場所が要る）
//! ——置き場所は一時ディレクトリの下になる。上流の宛先は閉じたポート（[`DECOY_ENDPOINT`]）、
//! `PATH` の先頭は偽の `llama`。**`--no-ui --no-browser`。**
//!
//! # ⚠ 環境変数を渡さない腕の、壊れたときの倒れ方
//!
//! **保存した番号を読めない実装では、既定の `8080` を開きに行く**（`DEFAULT_PORT`）。正しい実装では
//! 一度も開かない。**このファイルの腕を壊す変異は「保存した番号を落とす」形で当てないこと**
//! （別の番号へずらす形で当てる。`8080` は利用者の別のサーバが使っている）。

mod common;

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use common::has_field;

/// 待つ側の上限（回数）。**「働く側」ではなく「待つ側」に置く**（CLAUDE.md `G-4`）。
const MAX_WAITS: usize = 300;

/// 1回あたりの待ち時間。**上限は `MAX_WAITS` と合わせて 30 秒。**
const WAIT_STEP: Duration = Duration::from_millis(100);

/// **囮の上流の宛先**（閉じている loopback のポート）。
const DECOY_ENDPOINT: &str = "http://127.0.0.1:9";

/// **偽物が起きた印。** **一度も呼ばれないこと**を見る腕が読む。
const TOUCH_MARKER: &str = "fake-llama-was-started";

/// **保存した番号を読めないときの警告の本文**（`src/main.rs` の `SAVED_PORT_UNREADABLE`）。**写しである。**
///
/// 文面が変わったら、その腕は「言っていない」で赤になる（空振りの向きは安全側）。
const SAVED_PORT_UNREADABLE_LINE: &str =
    "保存した待ち受けのポートを読めないので、使わずに起動します";

/// **同じ警告の行が、直す場所として名指す画面**（作業単位 F-12b）。**写しである。**
///
/// 画面の「設定」で保存し直すか消せば直る（ファイルを手で直さなくてよい）ことを、警告が言う。
const SAVED_PORT_UNREADABLE_SCREEN: &str = "画面の「設定」";

/// **覚えた名前が使えないときの警告の本文**（`src/main.rs` の `REMEMBERED_UNUSABLE`）。**写しである。**
const REMEMBERED_UNUSABLE_LINE: &str =
    "覚えていた上流プロファイルを使えないので、環境変数の既定で起動します";

/// **飛ばす番号のために読む `upstreams.json` が読めないときの警告の本文**
/// （`src/main.rs` の `FACE_PORTS_UNREADABLE`）。**写しである。**
const FACE_PORTS_UNREADABLE_LINE: &str =
    "上流プロファイルを読めないので、待ち受けのポートを探すときに llama の面の番号を飛ばせません";

/// **範囲の幅の写し**（`port_search::PORT_SHIFT_SPAN`）。**定数から作らない。**
const SPAN: u16 = 10;

/// **元の番号を、なぜ使わなかったか**（ずらしの警告の文の頭が違う / `port_search::shifted_message`）。
#[derive(Clone, Copy)]
enum Why {
    /// 塞がっていた（「使えなかったので」）。
    Taken,
    /// llama の面が使う番号として飛ばした（「…面が使う番号なので」）。
    FacePort,
}

/// **ずらしたときの警告の頭**（`port_search::shifted_message` の写し。**定数から作らない**）。
///
/// **写すのは文の頭だけ**——括弧の中（次の起動・ブックマーク・「設定」）を変えても、これは赤にならない。
fn shifted_head(requested: u16, port: u16, why: Why) -> String {
    match why {
        Why::Taken => format!("ポート {requested} は使えなかったので、{port} で待ち受けます"),
        Why::FacePort => format!(
            "ポート {requested} は上流プロファイルの llama の面が使う番号なので、{port} で待ち受けます"
        ),
    }
}

/// **ずらしの警告の行に必ず入る綴り**（両方の文の括弧の頭 / 写し）。ずらしの警告を拾うのに使う。
const SHIFT_WARNING_MARK: &str = "（ずらした番号は覚え";

/// **ずれたかを起動ログから確かめ、`GET /admin/server` の `listening_port_shifted` が言うべき値を返す。**
///
/// * 実際の番号が元の番号と同じなら、**ずらしたという警告が無い**こと。
/// * 違うなら、**同じ1行が**元の番号・実際の番号を `why` の文で言い、欄（`requested_port` / `port` /
///   `port_source`）も持つこと。
fn assert_shift_said(lines: &[String], requested: u16, port: u16, source: &str, why: Why) -> bool {
    let shifted = port != requested;
    let said: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains(SHIFT_WARNING_MARK))
        .collect();
    if shifted {
        let head = shifted_head(requested, port, why);
        assert!(
            said.iter().any(|line| line.contains(&head)
                && line.contains("WARN")
                && has_field(line, &format!("requested_port={requested}"))
                && has_field(line, &format!("port={port}"))
                && has_field(line, &format!("port_source={source}"))),
            "{requested} から {port} へずれたのに、同じ行がそう言っていない\n{}",
            lines.join("\n")
        );
    } else {
        assert!(
            said.is_empty(),
            "ずれていないのに、ずらしたと言っている\n{}",
            lines.join("\n")
        );
    }
    shifted
}

/// **OS が自分では配らない番号の帯（20000〜28999）で、続いた 11 個（元の番号〜+10）の空きを全部掴む。**
///
/// **既定の 8080 を含まない**（帯の外）。**OS に選ばせた番号の近くを使わない理由**は
/// `tests/server_listening_port.rs` の同名の関数の doc（Windows は `127.0.0.1:0` の番号をプロセスをまたいで
/// 順に配るので、その近くの 11 個を掴むと、並んで走る他の腕が放した番号を奪う。作業単位 F-11b で実測）。
fn hold_quiet_range() -> (Vec<TcpListener>, u16) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CALLS: AtomicU32 = AtomicU32::new(0);
    let seed = std::process::id().wrapping_mul(2_654_435_761);
    for _ in 0..200 {
        let call = CALLS.fetch_add(1, Ordering::Relaxed);
        let offset = seed.wrapping_add(call.wrapping_mul(7_919)) % 9_000;
        let first = 20_000 + u16::try_from(offset).expect("9000 未満であること");
        let mut held = Vec::new();
        for port in first..=first + SPAN {
            match TcpListener::bind(("127.0.0.1", port)) {
                Ok(listener) => held.push(listener),
                Err(_) => break,
            }
        }
        if held.len() == usize::from(SPAN) + 1 {
            return (held, first);
        }
    }
    panic!("前提: 続いた 11 個の空きを 200 回で掴めなかった");
}

/// **元の番号だけを掴んだまま返す**（+1〜+10 は空いていることを確かめてから放す / [`hold_quiet_range`]）。
fn hold_a_free_port() -> (TcpListener, u16) {
    let (mut held, first) = hold_quiet_range();
    let kept = held.swap_remove(0);
    drop(held);
    (kept, first)
}

/// **元の番号から +10 までの 11 個を、全部このテストが掴む。**
fn hold_a_whole_range() -> (Vec<TcpListener>, u16) {
    hold_quiet_range()
}

/// **起こした実行ファイルの番人。`Drop` で必ず止める**（`tests/upstream_cli.rs` の `Running` と同じ）。
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Running {
    fn is_alive(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }
}

/// **起こした personacore と、その標準出力の行。**
struct Started {
    running: Running,
    lines: mpsc::Receiver<String>,
}

impl Started {
    /// **いままでに読めた行を全部取り出す**（1周ぶん待ってから畳む）。
    fn collect_lines(&mut self) -> Vec<String> {
        let mut found = Vec::new();
        while let Ok(line) = self.lines.recv_timeout(WAIT_STEP) {
            found.push(line);
        }
        found
    }

    /// **待ち受けを始めた行から実際のポートを読み、読み進めた行（待ち受けの行を含む）も返す。**
    ///
    /// **毎周「書き手が生きているか」を見る**——プロセスが死んだ瞬間、その行は永久に来ない。
    fn listening_port_keeping_lines(&mut self) -> (u16, Vec<String>) {
        let mut seen = Vec::new();
        for _ in 0..MAX_WAITS {
            match self.lines.recv_timeout(WAIT_STEP) {
                Ok(line) => {
                    let port = port_in(&line);
                    seen.push(line);
                    if let Some(port) = port {
                        return (port, seen);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if !self.running.is_alive() {
                panic!(
                    "待ち受けの行を読む前にプロセスが終了した\n{}",
                    seen.join("\n")
                );
            }
        }
        panic!("待ち受けの行を {MAX_WAITS} 回待っても読めなかった");
    }

    /// **待ち受けより前の行と後の行を、色づけを落として全部読む。**
    fn all_lines(&mut self, early: Vec<String>) -> Vec<String> {
        early
            .into_iter()
            .chain(self.collect_lines())
            .map(|line| strip_ansi(&line))
            .collect()
    }
}

/// **端末の色づけ（ANSI エスケープ）を落とす。純粋関数**（`tests/upstream_cli.rs` と同じ理由）。
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        if chars.next() != Some('[') {
            continue;
        }
        for tail in chars.by_ref() {
            if ('@'..='~').contains(&tail) {
                break;
            }
        }
    }
    out
}

/// 起動ログの `addr=127.0.0.1:NNNNN` からポートを読む。**純粋関数。**
fn port_in(line: &str) -> Option<u16> {
    let line = strip_ansi(line);
    let at = line.find("addr=127.0.0.1:")? + "addr=127.0.0.1:".len();
    let rest = &line[at..];
    let end = rest
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// **空いているポートを、互いに違う `N` 個取る**（同値だと取り違えが観測できない）。
///
/// **全部を掴んでから放す**——1つずつ取って放すと、同じ番号が2度返りうる。
fn distinct_free_ports<const N: usize>() -> [u16; N] {
    let held: Vec<TcpListener> = (0..N)
        .map(|_| TcpListener::bind("127.0.0.1:0").expect("ポートを取れること"))
        .collect();
    let mut ports = [0_u16; N];
    for (slot, listener) in ports.iter_mut().zip(&held) {
        *slot = listener.local_addr().expect("宛先を読めること").port();
    }
    drop(held);
    for port in ports {
        assert!(
            port >= 1024,
            "前提: OS が選んだ番号が保存できる範囲にあること"
        );
        assert_ne!(port, 8080, "前提: 既定のポートと違う番号であること");
    }
    ports
}

/// **子の環境から、利用者の本物の置き場所へ届く道を全部断つ**（`tests/upstream_cli.rs` と同じ）。
fn isolate_machine(command: &mut Command, root: &Path) {
    command
        .env("LOCALAPPDATA", root.join("localappdata"))
        .env("XDG_DATA_HOME", root.join("xdg"))
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"));
}

/// [`isolate_machine`] の下で、personacore が使う置き場所（`<データディレクトリ>/personacore`）。
///
/// **期待値を実装から作らない**——OS の作法（ADR-0041 §決定3）をここに直に書く。
fn app_dir_of(root: &Path) -> PathBuf {
    let data = if cfg!(windows) {
        root.join("localappdata")
    } else if cfg!(target_os = "macos") {
        root.join("home")
            .join("Library")
            .join("Application Support")
    } else {
        root.join("xdg")
    };
    data.join("personacore")
}

/// **保存した番号を置くファイル**（ADR-0057 §決定5）。**期待値を実装から作らない**——綴りを直に書く。
fn saved_port_file_of(root: &Path) -> PathBuf {
    app_dir_of(root).join("listening-port.txt")
}

/// **保存した番号を置く**（置き場所のディレクトリも作る）。
fn plant_saved_port(root: &Path, text: &str) -> PathBuf {
    let file = saved_port_file_of(root);
    std::fs::create_dir_all(file.parent().expect("親があること")).expect("置き場所を作れること");
    std::fs::write(&file, text).expect("保存した番号を置けること");
    file
}

/// **偽の `llama` を置いたディレクトリ**（本物を起こさないための保険。正しい実装は呼ばない）。
fn fake_llama_dir(dir: &Path) -> PathBuf {
    let bin_dir = dir.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("置き場所を作れること");
    std::fs::copy(
        env!("CARGO_BIN_EXE_fake_llama"),
        bin_dir.join(format!("llama{}", std::env::consts::EXE_SUFFIX)),
    )
    .expect("偽の llama を置けること");
    bin_dir
}

fn prepended_path(bin_dir: &Path) -> std::ffi::OsString {
    let mut value = std::ffi::OsString::from(bin_dir);
    if let Some(existing) = std::env::var_os("PATH") {
        value.push(if cfg!(windows) { ";" } else { ":" });
        value.push(existing);
    }
    value
}

/// **隔離した機で起こすコマンド。** 親から `PERSONACORE_*` を1つも継がせない。
///
/// **`PERSONACORE_PORT` も `PERSONACORE_UPSTREAMS` も、`extra` で渡したときだけ立つ**
/// （保存を読む腕は既定の置き場所が要るので、`PERSONACORE_UPSTREAMS` を渡さない）。
fn machine_command(root: &Path, args: &[&str], extra: &[(&str, &std::ffi::OsStr)]) -> Command {
    let bin_dir = fake_llama_dir(root);
    let mut command = Command::new(env!("CARGO_BIN_EXE_personacore"));
    for (key, _) in std::env::vars() {
        if key.starts_with("PERSONACORE_") {
            command.env_remove(key);
        }
    }
    isolate_machine(&mut command, root);
    command
        .args(["--no-ui", "--no-browser"])
        .args(args)
        .env("PERSONACORE_LLM_ENDPOINT", DECOY_ENDPOINT)
        .env("PATH", prepended_path(&bin_dir))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in extra {
        command.env(key, value);
    }
    command
}

/// **止まるはずの起動を、終わるまで（上限つきで）待ち、終了の状態と標準エラーを返す。**
///
/// **上限を過ぎたら止めて赤にする**——止まるはずの起動が起きてしまう実装で、テストが返らなくならないように。
fn run_to_exit(
    root: &Path,
    args: &[&str],
    extra: &[(&str, &std::ffi::OsStr)],
) -> (std::process::ExitStatus, String) {
    let mut command = machine_command(root, args, extra);
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = command.spawn().expect("実行ファイルを起こせること");
    let stderr = child.stderr.take().expect("標準エラーを取れること");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut text);
        text
    });
    let mut running = Running(child);
    for _ in 0..MAX_WAITS {
        if let Ok(Some(status)) = running.0.try_wait() {
            let text = reader.join().expect("標準エラーを読むスレッドが終わること");
            return (status, text);
        }
        std::thread::sleep(WAIT_STEP);
    }
    drop(running);
    let text = reader.join().expect("標準エラーを読むスレッドが終わること");
    panic!("止まるはずの起動が {MAX_WAITS} 回待っても終わらなかった\n{text}");
}

/// **隔離した機で起こす**（[`machine_command`]）。
fn spawn_on_machine(root: &Path, args: &[&str], extra: &[(&str, &std::ffi::OsStr)]) -> Started {
    let mut command = machine_command(root, args, extra);
    let mut child = command.spawn().expect("実行ファイルを起こせること");
    let stdout = child.stdout.take().expect("標準出力を取れること");
    let (sender, lines) = mpsc::channel();
    // **別のスレッドで読み続ける。** 読まないと、パイプが埋まった時点で相手が止まる。
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                return;
            }
        }
    });
    Started {
        running: Running(child),
        lines,
    }
}

/// **管理 API を1回叩く**（上限は待つ側に / `tests/upstream_cli.rs` の `call` と同じ）。
fn call(
    port: u16,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (u16, serde_json::Value) {
    let url = format!("http://127.0.0.1:{port}{path}");
    let (status, text) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime を作れること")
        .block_on(async move {
            let mut request = reqwest::Client::new()
                .request(method, &url)
                .timeout(Duration::from_secs(120));
            if let Some(body) = body {
                request = request.json(&body);
            }
            let response = request.send().await.expect("管理 API へ繋がること");
            let status = response.status().as_u16();
            (status, response.text().await.expect("本文を読めること"))
        });
    let json = serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("JSON として読めること: {err} / {text}"));
    (status, json)
}

fn server_report(port: u16) -> serde_json::Value {
    let (status, body) = call(port, reqwest::Method::GET, "/admin/server", None);
    assert_eq!(status, 200, "{body}");
    body
}

/// **起動ログの `設定を読み込みました` の同じ行が、番号と出どころを言う。**
fn assert_settings_line(lines: &[String], port: u16, from: &str) {
    let bind_field = format!("bind=127.0.0.1:{port}");
    let from_field = format!("bind_from={from}");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("設定を読み込みました")
                && has_field(line, &bind_field)
                && has_field(line, &from_field)),
        "起動ログの同じ行が {bind_field} / {from_field} を言っていない\n{}",
        lines.join("\n")
    );
}

// ----------------------------------------------------------------------
// 本命
// ----------------------------------------------------------------------

/// **保存した番号で、引数なしの起動が待ち受ける。API で保存し直すと、次の起動はその番号で待ち受ける。
/// `null` で消せる**（ADR-0057 §決定5・6）。
///
/// * **環境変数の既定の経路**（`AppConfig::from_env`）が保存した番号を見ること——`PERSONACORE_PORT` を渡さない。
/// * 起動ログの `bind` / `bind_from` と、`GET /admin/server` の全部の欄。
/// * **保存し直した番号は、いまの待ち受けを移さない**（`listening_port` は1回目の番号のまま）が、
///   `next_port` はすぐ変わる。
/// * **3回目は起こさない**——消した後の起動は `8080` を開きに行く（モジュールの doc）。
#[test]
fn a_saved_port_is_where_a_bare_start_listens_and_the_api_moves_it() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [first_port, second_port] = distinct_free_ports::<2>();
    let file = plant_saved_port(root, &format!("{first_port}\n"));

    {
        let mut first = spawn_on_machine(root, &[], &[]);
        let (port, early) = first.listening_port_keeping_lines();
        let lines = first.all_lines(early);
        assert_settings_line(&lines, first_port, "saved");
        // **保存した番号で待ち受ける。放してから子が開くまでに取られていたら、ずれたと言う**（モジュールの doc）。
        let shifted = assert_shift_said(&lines, first_port, port, "saved", Why::Taken);

        assert_eq!(
            server_report(port),
            serde_json::json!({
                "listening_port": port,
                "listening_port_source": "saved",
                "listening_port_requested": first_port,
                "listening_port_shifted": shifted,
                "listening_port_skipped": [],
                "saved_port": first_port,
                "saved_port_unreadable": false,
                "next_port": first_port,
                "next_port_source": "saved",
                "editable": true,
            })
        );

        let (status, saved) = call(
            port,
            reqwest::Method::PUT,
            "/admin/server/port",
            Some(serde_json::json!({ "port": second_port })),
        );
        assert_eq!(status, 200, "{saved}");
        assert_eq!(
            saved,
            serde_json::json!({
                "listening_port": port,
                "listening_port_source": "saved",
                "listening_port_requested": first_port,
                "listening_port_shifted": shifted,
                "listening_port_skipped": [],
                "saved_port": second_port,
                "saved_port_unreadable": false,
                "next_port": second_port,
                "next_port_source": "saved",
                "editable": true,
                "saved_port_in_use": false,
            }),
            "保存の応答が、いまの待ち受けを移したか、保存を言っていない"
        );
        assert_eq!(
            std::fs::read_to_string(&file).expect("読めること"),
            format!("{second_port}\n"),
            "保存のファイルが書き換わっていない"
        );
        assert!(first.running.is_alive(), "待ち受けが続いていること");
    }

    let mut second = spawn_on_machine(root, &[], &[]);
    let (port, early) = second.listening_port_keeping_lines();
    let lines = second.all_lines(early);
    // **API で保存した番号から、次の起動が試し始める**（取られていたら、ずれたと言う）。
    assert_settings_line(&lines, second_port, "saved");
    let shifted = assert_shift_said(&lines, second_port, port, "saved", Why::Taken);
    let report = server_report(port);
    assert_eq!(report["listening_port_requested"], second_port, "{report}");
    assert_eq!(report["listening_port_shifted"], shifted, "{report}");

    let (status, forgot) = call(
        port,
        reqwest::Method::PUT,
        "/admin/server/port",
        Some(serde_json::json!({ "port": null })),
    );
    assert_eq!(status, 200, "{forgot}");
    assert_eq!(forgot["saved_port"], serde_json::Value::Null, "{forgot}");
    assert_eq!(forgot["next_port"], serde_json::json!(8080), "{forgot}");
    assert_eq!(forgot["next_port_source"], "default", "{forgot}");
    assert_eq!(
        forgot["saved_port_in_use"],
        serde_json::Value::Null,
        "{forgot}"
    );
    assert!(!file.exists(), "保存を消したのにファイルが残っている");
}

/// **`--upstream` で選んだ起動も、保存した番号で待ち受ける**（プロファイル経由の経路 `config_for`）。
///
/// **プロファイルは gguf を書かない**（何も起こさない）。
#[test]
fn the_upstream_flag_path_listens_on_the_saved_port_too() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [saved_port] = distinct_free_ports::<1>();
    plant_saved_port(root, &format!("{saved_port}\n"));
    std::fs::write(
        app_dir_of(root).join("upstreams.json"),
        r#"{ "plain": { "main": { "endpoint": "http://127.0.0.1:47701", "model": "plain-model" } } }"#,
    )
    .expect("プロファイルを置けること");

    let mut started = spawn_on_machine(root, &["--upstream", "plain"], &[]);
    let (port, early) = started.listening_port_keeping_lines();
    let lines = started.all_lines(early);
    // **`--upstream` の起動も保存した番号から試し始める**（取られていたら、ずれたと言う）。
    assert_settings_line(&lines, saved_port, "saved");
    assert_shift_said(&lines, saved_port, port, "saved", Why::Taken);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("設定を読み込みました")
                && has_field(line, "upstream_profile_from=flag")),
        "前提: --upstream の経路で起きていること\n{}",
        lines.join("\n")
    );
    assert_eq!(server_report(port)["listening_port_source"], "saved");
}

/// **覚えた名前のプロファイルの gguf の面が保存した番号を使っていても、覚えた名前は捨てない。
/// 待ち受けは保存した番号を飛ばす**（ADR-0057 §決定3「事前検査は、番号を明示したときだけ番号の取り合いで
/// プロファイルを捨てる」「明示していなければ元の番号も面と同じなら飛ばす」/ 作業単位 F-11b）。
///
/// **F-12 の間は逆だった**（取り合いで覚えた名前を捨て、環境変数の既定の経路で保存した番号で起きた）。
///
/// * 1回目（番号を明示しない）: 覚えた名前で起き（`upstream_profile_from=remembered`）、使えないと言わない。
///   保存した番号は面が使うので開かず、ずれたと言う（飛ばした番号 = 保存した番号）。
/// * 2回目（**対照**: 同じ番号を `PERSONACORE_PORT` で明示）: 今までどおり、取り合いで覚えた名前を捨てる。
///
/// **偽の `llama` は1度も呼ばれない**（覚えた名前の起動は起こさない／使わない）。
#[test]
fn a_remembered_profile_on_the_saved_port_is_kept_and_the_saved_port_is_skipped() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [saved_port] = distinct_free_ports::<1>();
    plant_saved_port(root, &format!("{saved_port}\n"));
    let gguf = root.join("planted-beta.gguf");
    std::fs::write(&gguf, b"not a real gguf").expect("置けること");
    let marker = root.join(TOUCH_MARKER);
    std::fs::write(
        app_dir_of(root).join("upstreams.json"),
        serde_json::json!({ "beta": { "main": {
            "endpoint": format!("http://127.0.0.1:{saved_port}"), "model": "beta-model",
            "gguf": gguf,
            "args": ["--pc-touch", marker, "--pc-exit", "0"],
        } } })
        .to_string(),
    )
    .expect("保存した番号を使うプロファイルを置けること");
    std::fs::write(app_dir_of(root).join("selected-upstream.txt"), "beta\n")
        .expect("覚えた名前を置けること");

    {
        let mut started = spawn_on_machine(root, &[], &[]);
        let (port, early) = started.listening_port_keeping_lines();
        let lines = started.all_lines(early);
        assert_ne!(port, saved_port, "面が使う保存した番号で待ち受けた");
        // **元の番号は塞がっていたのではなく面の番号なので、警告は「面が使う番号なので」の文で言う**
        // （「使えなかった」「次の起動ではまた試す」は偽になる / F-11b のレビュー是正）。
        assert!(
            assert_shift_said(&lines, saved_port, port, "saved", Why::FacePort),
            "前提: ずれていること"
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("は使えなかったので、")),
            "面の番号で飛ばした元の番号を「使えなかった」と言っている\n{}",
            lines.join("\n")
        );
        assert!(
            lines.iter().any(|line| line.contains(SHIFT_WARNING_MARK)
                && has_field(line, &format!("skipped_ports=[{saved_port}]"))),
            "飛ばした番号（保存した番号）を言っていない\n{}",
            lines.join("\n")
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains(REMEMBERED_UNUSABLE_LINE)),
            "番号を明示していないのに、覚えた名前を取り合いで捨てた\n{}",
            lines.join("\n")
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("設定を読み込みました")
                    && has_field(line, "upstream_profile_from=remembered")
                    && has_field(line, "bind_from=saved")),
            "覚えた名前で、保存した番号から起きたと言っていない\n{}",
            lines.join("\n")
        );
        let report = server_report(port);
        assert_eq!(
            report["listening_port_skipped"],
            serde_json::json!([saved_port]),
            "{report}"
        );
    }

    // **対照: 同じ番号を明示すると、今までどおり覚えた名前を捨てる**（明示した番号はずらさず、面と取り合う）。
    let explicit = saved_port.to_string();
    let mut started = spawn_on_machine(
        root,
        &[],
        &[("PERSONACORE_PORT", std::ffi::OsStr::new(&explicit))],
    );
    let (port, early) = started.listening_port_keeping_lines();
    assert_eq!(port, saved_port, "明示した番号で待ち受けていない");
    let lines = started.all_lines(early);
    assert!(
        lines
            .iter()
            .any(|line| line.contains(REMEMBERED_UNUSABLE_LINE)
                && has_field(line, "upstream_profile=\"beta\"")),
        "明示した番号と取り合う覚えた名前を、使えないと言っていない\n{}",
        lines.join("\n")
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("設定を読み込みました")
                && has_field(line, "upstream_profile_from=-")
                && has_field(line, "bind_from=environment")),
        "環境変数の既定の経路で、明示した番号で起きたと言っていない\n{}",
        lines.join("\n")
    );
    assert!(!marker.exists(), "起こさないはずの上流を起こしている");
}

/// **環境変数が保存に勝つ。`GET /admin/server` は保存した番号と、次の起動も環境変数で決まることを言う。**
#[test]
fn the_environment_wins_over_the_saved_port() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [saved_port, env_port] = distinct_free_ports::<2>();
    plant_saved_port(root, &format!("{saved_port}\n"));
    let env_value = env_port.to_string();

    let mut started = spawn_on_machine(
        root,
        &[],
        &[("PERSONACORE_PORT", std::ffi::OsStr::new(&env_value))],
    );
    let (port, early) = started.listening_port_keeping_lines();
    assert_eq!(port, env_port, "環境変数が保存に負けている");
    assert_settings_line(&started.all_lines(early), env_port, "environment");
    assert_eq!(
        server_report(port),
        serde_json::json!({
            "listening_port": env_port,
            "listening_port_source": "environment",
            "listening_port_requested": env_port,
            "listening_port_shifted": false,
            "listening_port_skipped": [],
            "saved_port": saved_port,
            "saved_port_unreadable": false,
            "next_port": env_port,
            "next_port_source": "environment",
            "editable": true,
        })
    );
}

/// **`PERSONACORE_UPSTREAMS` が別の場所を指す起動は、保存を読まない・書かない**
/// （ADR-0057 §決定5「既定の置き場所のときだけ」）。
///
/// **`PERSONACORE_PORT=0`**（保存を読まないので、渡さないと `8080` を開きに行く）。
///
/// # 指した先は、保存のファイルと同じディレクトリに置く
///
/// **別のディレクトリを指すと、「読むだけの置き場所でも隣を読む」実装が、無いファイルを読んで緑になる。**
/// 同じディレクトリなら、その実装は植えた番号を読む（`GET` の `saved_port` に出る）。
/// **`main` の側（起動時に読むか）は、環境変数が勝つので番号では観測できない**——
/// 2つ目の腕で、壊れた中身を植えて**警告が出ないこと**で見る。
#[test]
fn an_overridden_profile_file_neither_reads_nor_writes_the_saved_port() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [saved_port, wanted_port] = distinct_free_ports::<2>();
    let file = plant_saved_port(root, &format!("{saved_port}\n"));
    let before = std::fs::read(&file).expect("読めること");
    let overridden = app_dir_of(root).join("no-such-upstreams.json");

    let mut started = spawn_on_machine(
        root,
        &[],
        &[
            ("PERSONACORE_PORT", std::ffi::OsStr::new("0")),
            ("PERSONACORE_UPSTREAMS", overridden.as_os_str()),
        ],
    );
    let (port, early) = started.listening_port_keeping_lines();
    assert_ne!(
        port, saved_port,
        "読まないはずの保存した番号で待ち受けている"
    );
    assert_settings_line(&started.all_lines(early), 0, "environment");

    let report = server_report(port);
    // **実際に待ち受けた番号を言う**（設定は `0`。設定の番号を言う実装はここで赤 —— 変異 M11）。
    assert_eq!(
        report["listening_port"],
        serde_json::json!(port),
        "{report}"
    );
    // **元の番号は設定の `0`、ずらしてはいない**（OS に選ばせた番号は「ずれた」ではない / 作業単位 F-11b）。
    assert_eq!(
        report["listening_port_requested"],
        serde_json::json!(0),
        "{report}"
    );
    assert_eq!(
        report["listening_port_shifted"],
        serde_json::json!(false),
        "{report}"
    );
    assert_eq!(report["saved_port"], serde_json::Value::Null, "{report}");
    assert_eq!(
        report["saved_port_unreadable"],
        serde_json::json!(false),
        "{report}"
    );
    assert_eq!(report["editable"], serde_json::json!(false), "{report}");
    assert_eq!(report["next_port"], serde_json::json!(0), "{report}");
    assert_eq!(report["next_port_source"], "environment", "{report}");

    let (status, refused) = call(
        port,
        reqwest::Method::PUT,
        "/admin/server/port",
        Some(serde_json::json!({ "port": wanted_port })),
    );
    assert_eq!(status, 409, "{refused}");
    assert_eq!(refused["error"]["code"], "listening_port_read_only");
    assert_eq!(
        std::fs::read(&file).expect("読めること"),
        before,
        "読むだけの起動が保存のファイルを書き換えた"
    );
    assert!(started.running.is_alive(), "待ち受けが続いていること");
    drop(started);

    // **`main` の側も読まない**——壊れた中身を植えても、読まないので警告は出ない。
    // **対照は `an_unreadable_saved_port_warns_and_the_file_is_kept`**（既定の置き場所なら同じ中身で警告が出る）。
    std::fs::write(&file, "eighty-ninety\n").expect("書けること");
    let mut started = spawn_on_machine(
        root,
        &[],
        &[
            ("PERSONACORE_PORT", std::ffi::OsStr::new("0")),
            ("PERSONACORE_UPSTREAMS", overridden.as_os_str()),
        ],
    );
    let (port, early) = started.listening_port_keeping_lines();
    let lines = started.all_lines(early);
    assert!(
        !lines
            .iter()
            .any(|line| line.contains(SAVED_PORT_UNREADABLE_LINE)),
        "読むだけの起動が、起動時に保存のファイルを読んでいる\n{}",
        lines.join("\n")
    );
    assert_settings_line(&lines, 0, "environment");
    // **応答も「壊れている」と言わない**（読まないのだから。編集できる置き場所なら `true` になる対照は下のテスト）。
    let report = server_report(port);
    assert_eq!(
        report["saved_port_unreadable"],
        serde_json::json!(false),
        "読むだけの起動が、保存のファイルを読んで壊れていると言っている: {report}"
    );
}

/// **読めない保存のファイルは、警告を1本出して使わない。ファイルは消さない。止まらない。**
///
/// **`PERSONACORE_PORT=0`**（読めない保存は使わないので、渡さないと `8080` を開きに行く）——
/// **環境変数があっても保存を読み、壊れていれば知らせる**（`main` の `saved_port_of` の doc）。
#[test]
fn an_unreadable_saved_port_warns_and_the_file_is_kept() {
    for (arm, text) in [
        ("数でない", "eighty-ninety\n"),
        ("下端の外", "1023\n"),
        ("2行", "8090\n8091\n"),
    ] {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let root = dir.path();
        let file = plant_saved_port(root, text);

        let mut started = spawn_on_machine(
            root,
            &[],
            &[("PERSONACORE_PORT", std::ffi::OsStr::new("0"))],
        );
        let (port, early) = started.listening_port_keeping_lines();
        let lines = started.all_lines(early);
        let warnings: Vec<&String> = lines
            .iter()
            .filter(|line| line.contains(SAVED_PORT_UNREADABLE_LINE))
            .collect();
        assert_eq!(
            warnings.len(),
            1,
            "{arm}: 読めない保存を、警告1本で言っていない\n{}",
            lines.join("\n")
        );
        assert!(
            warnings[0].contains("listening-port.txt"),
            "{arm}: 警告の同じ行が置き場所を言っていない: {}",
            warnings[0]
        );
        // **同じ行が、直せる画面を名指す**（作業単位 F-12b）。
        assert!(
            warnings[0].contains(SAVED_PORT_UNREADABLE_SCREEN),
            "{arm}: 警告の同じ行が「設定」の画面を名指していない: {}",
            warnings[0]
        );
        // **前提: 警告の行は `WARN` を名乗る**（下の「GET が警告を積まない」の検出が空振りしないこと）。
        assert!(
            warnings[0].contains("WARN"),
            "{arm}: 警告の行に WARN が無い（検出の綴りが合っていない）: {}",
            warnings[0]
        );
        // **応答は「壊れたファイルが在る」と言い、実際に待ち受けた番号を言う**（設定は `0` —— 変異 M11）。
        let report = server_report(port);
        assert_eq!(
            report["listening_port"],
            serde_json::json!(port),
            "{arm}: {report}"
        );
        assert_eq!(
            report["saved_port"],
            serde_json::Value::Null,
            "{arm}: {report}"
        );
        assert_eq!(
            report["saved_port_unreadable"],
            serde_json::json!(true),
            "{arm}: {report}"
        );
        // **`GET` は警告を積まない**（読めないことは応答の欄が言い、警告は起動時の1本だけ）。
        // 2回目も読んでから数える。
        let _ = server_report(port);
        let after = started.all_lines(Vec::new());
        assert!(
            !after
                .iter()
                .any(|line| line.contains("WARN") && line.contains("保存した待ち受けのポート")),
            "{arm}: GET のたびに警告を出している\n{}",
            after.join("\n")
        );
        assert_eq!(
            std::fs::read_to_string(&file).expect("読めること"),
            text,
            "{arm}: 読めない保存のファイルを書き換えた"
        );
        assert!(started.running.is_alive(), "{arm}: 止まっている");
    }
}

// ----------------------------------------------------------------------
// ずらし（作業単位 F-11b / ADR-0057 §決定1・3・4）
// ----------------------------------------------------------------------
//
// **ずらしの観測は保存した番号で行う**（ADR-0057 §却下した選択肢「F-11 を先に入れる」の理由）。
// **既定の 8080 を経由する腕は作らない**——利用者の別のサーバが 8080 を使っていることがある。
// 見本の `upstreams.json` の 8081 を飛ばす形は、`port_search` の単体テスト
// （`the_sample_profiles_make_a_taken_default_port_skip_8081`）が純粋関数で見る。

/// **保存した番号が塞がっていると、`upstreams.json` の gguf の面の番号を飛ばして次の空きで待ち受け、
/// 警告・待ち受けの行の `addr=`・`GET /admin/server` が実際の番号を言う。`bind=` は保存した番号のまま。**
///
/// 保存した番号 `P` をこのテストが掴み、**選ばれていないプロファイル**の gguf の面を `P+1` に置く
/// （起動の計画に入っているかを問わず飛ばす / ADR-0057 §決定3）。待ち受けは `P+2` 以降で、飛ばした番号は `P+1`。
/// **偽の `llama` は1度も呼ばれない**（プロファイルを選んでいない起動は何も起こさない）。
///
/// **gguf の面を持つプロファイルを、名前の順で2つ目に置く**（1つ目 `aa-external` は外の上流だけ）——
/// 最初のプロファイルだけを見る実装を、ここでも赤にする（変異 M15 で、単体テストだけが止めていたのを足した）。
#[test]
fn a_taken_saved_port_shifts_past_a_face_port_and_the_log_and_the_api_say_so() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (blocker, saved_port) = hold_a_free_port();
    let face_port = saved_port + 1;
    plant_saved_port(root, &format!("{saved_port}\n"));
    let marker = root.join(TOUCH_MARKER);
    std::fs::write(
        app_dir_of(root).join("upstreams.json"),
        serde_json::json!({
            "aa-external": { "main": {
                "endpoint": DECOY_ENDPOINT, "model": "external-model",
            } },
            "zz-unchosen": { "main": {
                "endpoint": format!("http://127.0.0.1:{face_port}"), "model": "unchosen-model",
                "gguf": root.join("planted-unchosen.gguf"),
                "args": ["--pc-touch", marker],
            } },
        })
        .to_string(),
    )
    .expect("プロファイルを置けること");

    let mut started = spawn_on_machine(root, &[], &[]);
    let (port, early) = started.listening_port_keeping_lines();
    let lines = started.all_lines(early);
    assert!(
        port > face_port && port <= saved_port + SPAN,
        "面の番号を飛ばした範囲の中で待ち受けていない（保存 {saved_port}・面 {face_port}・実際 {port}）\n{}",
        lines.join("\n")
    );
    // **警告の同じ1行が、元の番号・実際の番号（文と欄）・出どころ・飛ばした番号を言う。**
    assert!(assert_shift_said(
        &lines,
        saved_port,
        port,
        "saved",
        Why::Taken
    ));
    assert!(
        lines.iter().any(
            |line| line.contains(&shifted_head(saved_port, port, Why::Taken))
                && has_field(line, &format!("skipped_ports=[{face_port}]"))
        ),
        "警告の同じ行が飛ばした番号を言っていない\n{}",
        lines.join("\n")
    );
    // **`bind=` は元の番号（出どころの番号）のまま**（ADR-0057 §決定4）。**実際の番号は待ち受けの行の `addr=`**
    // （`listening_port_keeping_lines` がそこから読んだ番号が `port`）。
    assert_settings_line(&lines, saved_port, "saved");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("待ち受けを開始しました")
                && has_field(line, &format!("addr=127.0.0.1:{port}"))),
        "待ち受けの行の addr= が実際の番号を言っていない\n{}",
        lines.join("\n")
    );

    assert_eq!(
        server_report(port),
        serde_json::json!({
            "listening_port": port,
            "listening_port_source": "saved",
            "listening_port_requested": saved_port,
            "listening_port_shifted": true,
            "listening_port_skipped": [face_port],
            "saved_port": saved_port,
            "saved_port_unreadable": false,
            "next_port": saved_port,
            "next_port_source": "saved",
            "editable": true,
        })
    );
    // **ずらした番号は覚えない**（保存のファイルは保存した番号のまま）。
    assert_eq!(
        std::fs::read_to_string(saved_port_file_of(root)).expect("読めること"),
        format!("{saved_port}\n"),
        "ずらした番号を保存している"
    );
    assert!(
        !marker.exists(),
        "選んでいないプロファイルの上流を起こしている"
    );
    drop(started);
    drop(blocker);
}

/// **範囲（保存した番号〜+10）が全部塞がっていると、範囲と直し方を言って止まる**（ADR-0057 §決定3）。
///
/// 11 個とも、このテストが掴んだもの。**止まる文**（`config::ports_in_use_message` の写し）は、範囲・
/// `PERSONACORE_PORT`・画面の「設定」を言い、**1つの番号の文（B-76）は言わない。**
#[test]
fn a_whole_range_taken_names_the_range_and_how_to_get_out() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (held, first) = hold_a_whole_range();
    let last = first + SPAN;
    plant_saved_port(root, &format!("{first}\n"));

    let (status, stderr) = run_to_exit(root, &[], &[]);
    drop(held);
    assert!(
        !status.success(),
        "範囲が全部塞がっているのに 0 で終わった\n{stderr}"
    );
    assert!(
        stderr.contains(&format!(
            "ポート {first}〜{last} は、どれも他のプログラムが使っているか、使えない番号でした"
        )),
        "止まる文が試した範囲を言っていない\n{stderr}"
    );
    assert!(
        stderr.contains("PERSONACORE_PORT"),
        "止まる文が PERSONACORE_PORT を言っていない\n{stderr}"
    );
    assert!(
        stderr.contains("画面の「設定」"),
        "止まる文が画面の「設定」を言っていない\n{stderr}"
    );
    assert!(
        !stderr.contains(&format!("ポート {first} は他のプログラムが使っています")),
        "範囲の失敗を、1つの番号の文で言っている\n{stderr}"
    );
    assert!(
        stderr.contains("待ち受けアドレスを開けませんでした"),
        "OS の文に至る鎖が消えている\n{stderr}"
    );
    // **鎖の `Caused by` の行（`PortsTaken` の文）も、試した数を偽りなく言う**（F-11 の再レビュー是正）。
    assert!(
        stderr.contains(&format!(
            "ポート {first}〜{last} のうち 11 個を順に試しましたが、どれも開けませんでした"
        )),
        "鎖の文が試した数を言っていない\n{stderr}"
    );
    assert!(
        !stderr.contains("を除いて"),
        "飛ばしていないのに、面の番号を除いたと言っている\n{stderr}"
    );
}

/// **範囲の中に llama の面の番号が在ると、止まる文がその番号を「除いて」と言う**（F-11 の再レビュー是正）。
///
/// 範囲の 11 個のうち `first+5` だけを塞がずに残し、その番号を gguf の面に持つプロファイルを置く
/// （選ばない）。`first+5` は空いているが面の番号なので試さず、ほかの 10 個は塞がっているので止まる。
/// **`main` が飛ばした番号を文へ渡す配線**（`taken.skipped()`）を見る——渡さなければ「を除いて」が出ない
/// （変異 M20）。`a_whole_range_taken_names_the_range_and_how_to_get_out` は `upstreams.json` を置かない
/// （見本に 8081 があるだけ）ので、この配線を区別できない。
#[test]
fn a_whole_range_with_a_face_port_names_the_skipped_port() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (mut held, first) = hold_a_whole_range();
    let last = first + SPAN;
    let face_port = first + 5;
    // **`first+5` だけを放す**（並びは `first` から順なので、6 番目）。
    drop(held.remove(5));
    plant_saved_port(root, &format!("{first}\n"));
    let marker = root.join(TOUCH_MARKER);
    std::fs::write(
        app_dir_of(root).join("upstreams.json"),
        serde_json::json!({ "unchosen": { "main": {
            "endpoint": format!("http://127.0.0.1:{face_port}"), "model": "unchosen-model",
            "gguf": root.join("planted-unchosen.gguf"),
            "args": ["--pc-touch", marker],
        } } })
        .to_string(),
    )
    .expect("プロファイルを置けること");

    let (status, stderr) = run_to_exit(root, &[], &[]);
    drop(held);
    assert!(
        !status.success(),
        "面の番号のほかが全部塞がっているのに 0 で終わった（面の番号 {face_port} で待ち受けた？）\n{stderr}"
    );
    assert!(
        stderr.contains(&format!(
            "ポート {first}〜{last} は、上流プロファイルの llama の面が使う番号（{face_port}）を除いて、\
             どれも他のプログラムが使っているか、使えない番号でした"
        )),
        "止まる文が、飛ばした面の番号を除いたと言っていない\n{stderr}"
    );
    assert!(
        stderr.contains(&format!(
            "ポート {first}〜{last} のうち 10 個を順に試しましたが、どれも開けませんでした"
        )),
        "鎖の文が、試した数（飛ばした番号を除く）を言っていない\n{stderr}"
    );
    assert!(
        !marker.exists(),
        "選んでいないプロファイルの上流を起こしている"
    );
}

/// **飛ばす番号のために読む `upstreams.json` が読めなければ、警告を1本出して、飛ばさずに起きる。
/// 番号を明示した起動は読まない**（ADR-0057 §決定3）。
///
/// * 1回目（明示しない・保存した番号）: 警告がちょうど1本・`WARN`・置き場所を言う。止まらない。
/// * 2回目・3回目（**対照**: `PERSONACORE_PORT` を `0` と、0 でない空いている番号で明示）: 警告が無い（読まない）。
///   **0 でない腕が要る**——「明示か」を `bind().port() == 0` で決める実装は、`0` の腕だけでは見分けられない
///   （F-11b のレビューが予想した変異 M19）。
#[test]
fn an_unreadable_profile_file_warns_once_and_the_start_goes_on() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [saved_port] = distinct_free_ports::<1>();
    plant_saved_port(root, &format!("{saved_port}\n"));
    std::fs::write(app_dir_of(root).join("upstreams.json"), "{ not json")
        .expect("壊れたプロファイルを置けること");

    {
        let mut started = spawn_on_machine(root, &[], &[]);
        let (port, early) = started.listening_port_keeping_lines();
        let lines = started.all_lines(early);
        let warnings: Vec<&String> = lines
            .iter()
            .filter(|line| line.contains(FACE_PORTS_UNREADABLE_LINE))
            .collect();
        assert_eq!(
            warnings.len(),
            1,
            "読めない upstreams.json を、警告1本で言っていない\n{}",
            lines.join("\n")
        );
        assert!(warnings[0].contains("WARN"), "{}", warnings[0]);
        assert!(
            warnings[0].contains("upstreams.json"),
            "警告の同じ行が置き場所を言っていない: {}",
            warnings[0]
        );
        assert_settings_line(&lines, saved_port, "saved");
        assert_shift_said(&lines, saved_port, port, "saved", Why::Taken);
        assert!(started.running.is_alive(), "止まっている");
    }

    // **明示の番号は、OS が配らない帯から取る**（放してから子が開くまでに取られにくい / `hold_quiet_range`）。
    let (held, quiet) = hold_quiet_range();
    drop(held);
    for explicit in ["0".to_owned(), quiet.to_string()] {
        let mut started = spawn_on_machine(
            root,
            &[],
            &[("PERSONACORE_PORT", std::ffi::OsStr::new(&explicit))],
        );
        let (_port, early) = started.listening_port_keeping_lines();
        let lines = started.all_lines(early);
        assert_settings_line(
            &lines,
            explicit.parse().expect("番号であること"),
            "environment",
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains(FACE_PORTS_UNREADABLE_LINE)),
            "番号を明示した起動（{explicit}）が、飛ばす番号のために upstreams.json を読んでいる\n{}",
            lines.join("\n")
        );
    }
}
