//! **同じデータを使う personacore を2つ動かさない（データのディレクトリのロック）を、実行ファイルそのもので確かめる**
//! （[ADR-0057](../docs/adr/0057-the-listening-port-shifts-when-taken-and-is-set-from-the-screen.md) §決定2 /
//! 作業単位 F-11a / marukei/personacore#14）。**本物の `llama` にも LM Studio にも繋がない。本物のブラウザも開かない。**
//!
//! # なぜ実行ファイルを起こすのか
//!
//! **ロックを取る1行と、取れなかった2つ目の振る舞い（案内して 0 で終わる）は `src/main.rs` にしか無い**
//! （bin なので統合テストから `use` できない）。中身の読み書き・案内の文・待つか開くかの判定は
//! `src/server/instance_lock.rs` の単体テストが全部の腕を見る。
//!
//! # 何を固定するか
//!
//! | 見張り番 | 破れると |
//! |---|---|
//! | `a_second_start_on_the_same_data_names_the_first_and_listens_nowhere` | 2つ目が待ち受ける（二重起動）／1つ目の URL を言わない／0 以外で終わる／**ポートを変えると止まらない** |
//! | `a_second_start_uses_neither_fresh_nor_upstream_and_says_so` | 2つ目の `--fresh` が動いている側の DB を退避する／`--upstream` を読む／使わなかったと言わない |
//! | `a_killed_first_leaves_no_lock_behind` | 異常終了したプロセスのロックが残り、次から起動できない |
//! | `different_database_paths_run_side_by_side` | データのディレクトリが違うのに止まる |
//! | `the_version_flag_answers_while_another_one_runs` | `--version` がロックを見る |
//! | `the_second_start_opens_the_screen_only_when_asked_to` | 開く設定のときに開かない／`--no-browser` で開く／URL が違う／二重に開く |
//! | `the_second_start_never_opens_a_screen_the_first_withholds` | `--no-ui` の相手を開く／開かない理由を言わない |
//!
//! # 「起動の途中」の腕はここでは作らない
//!
//! **1つ目がロックを取ってから URL を書くまでの間**は、DB の接続（マイグレーションを含む）と上流を起こす間だけで、
//! テストから狙ってその間に2つ目を差し込む手段が無い（つまみを足さない / ADR-0036）。
//! **その腕は `src/server/instance_lock.rs` の単体テスト**（中身が空・壊れている・書きかけ → 起動の途中、
//! 本物のロックを取った直後の中身が空であること）**が押さえる。**
//!
//! # 1つ目が URL を書いたことを待つ
//!
//! **1つ目の起動ログの「ブラウザは開きません」（画面を配るとき）か、`--no-ui` の1行を待ってから**2つ目を起こす。
//! どちらも `App::run_with_upstreams` で、**URL を書いた後**に出る（`src/server/mod.rs`）。
//! 待ち受けの行（`addr=`）は `bind` の中で出るので、それだけを待つと、URL を書く前の2つ目が「起動の途中」と言いうる。
//!
//! # 隔離（CLAUDE.md「exe を起こすときは、DB だけでなく上流の設定も隔離する」）
//!
//! **どの腕も `LOCALAPPDATA` / `XDG_DATA_HOME` / `HOME` / `USERPROFILE` を一時ディレクトリへ向け、
//! `PERSONACORE_UPSTREAMS` を存在しないファイルへ向ける。** 上流の宛先は閉じたポート（[`DECOY_ENDPOINT`]）、
//! `PATH` の先頭は偽の `llama`、待ち受けは OS から取った空きの番号。**`8080` には触らない。**
//!
//! # ブラウザ
//!
//! **ブラウザを開く腕は、`PERSONACORE_TEST_BROWSER` に偽物（`tests/support/fake_recorder.rs`）を渡す。**
//! `cargo test` が起こす実行ファイルは `testing` 付きで、**その環境変数が指すものしか起こさない**
//! （`src/server/browser.rs` の doc / `tests/upstream_cli.rs` の `a_test_build_without_the_fake_opens_nothing`）。

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// 待つ側の上限（回数）。**「働く側」ではなく「待つ側」に置く**（CLAUDE.md `G-4`）。
const MAX_WAITS: usize = 300;

/// 1回あたりの待ち時間。**上限は `MAX_WAITS` と合わせて 30 秒。**
const WAIT_STEP: Duration = Duration::from_millis(100);

/// **囮の上流の宛先**（閉じている loopback のポート）。
const DECOY_ENDPOINT: &str = "http://127.0.0.1:9";

/// **2つ目が「動いている」と言う文の前半と後半**（`src/server/instance_lock.rs` の `RUNNING_*`）。**写しである。**
///
/// 文面が変わったら、その腕は「言っていない」で赤になる（空振りの向きは安全側）。
const RUNNING_HEAD: &str = "personacore はすでに ";
const RUNNING_TAIL: &str = " で動いています";

/// **2つ目が「使わなかった」と言う文の前半**（`src/server/instance_lock.rs` の `NOT_USED_HEAD`）。**写しである。**
const NOT_USED_HEAD: &str = "渡された ";
const NOT_USED_TAIL: &str = " は使っていません";

/// **相手が画面を配っていないと言う文**（`src/server/instance_lock.rs` の `WITHHELD`）。**写しである。**
const WITHHELD_LINE: &str = "動いている personacore は画面を配っていないので、ブラウザは開きません";

/// **1つ目が URL を書いた後に出る行**（`src/server/mod.rs` の `run_with_upstreams`）。**写しである。**
const AFTER_ANNOUNCE_UI: &str = "ブラウザは開きません";
const AFTER_ANNOUNCE_NO_UI: &str = "画面は配りません（--no-ui）";

/// **待ち受けを始めた行**（`src/server/mod.rs`）。**2つ目からは出てはならない。**
const LISTENING_LINE: &str = "待ち受けを開始しました";

/// **起こした実行ファイルの番人。`Drop` で必ず止める**（PID で止める。イメージ名では止めない）。
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
    /// **待ち受けの行から実際のポートを読み、`after` を含む行が出るまで待つ。** 読んだ行も返す。
    ///
    /// **毎周「書き手が生きているか」を見る**——プロセスが死んだ瞬間、その行は永久に来ない。
    fn port_after(&mut self, after: &str) -> (u16, Vec<String>) {
        let mut seen = Vec::new();
        let mut port = None;
        for _ in 0..MAX_WAITS {
            match self.lines.recv_timeout(WAIT_STEP) {
                Ok(line) => {
                    let line = strip_ansi(&line);
                    if port.is_none() {
                        port = port_in(&line);
                    }
                    let done = line.contains(after);
                    seen.push(line);
                    if done && let Some(port) = port {
                        return (port, seen);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if !self.running.is_alive() {
                panic!(
                    "待ち受けの行と「{after}」を読む前にプロセスが終了した\n{}",
                    seen.join("\n")
                );
            }
        }
        panic!(
            "待ち受けの行と「{after}」を {MAX_WAITS} 回待っても読めなかった\n{}",
            seen.join("\n")
        );
    }
}

/// **終わるまで待った起動の結果。**
struct Ended {
    status: ExitStatus,
    stdout: String,
    stderr: String,
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
    let at = line.find("addr=127.0.0.1:")? + "addr=127.0.0.1:".len();
    let rest = &line[at..];
    let end = rest
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// **空いているポートを、互いに違う `N` 個取る**（全部を掴んでから放す）。
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
        assert_ne!(port, 8080, "前提: 利用者の 8080 と違う番号であること");
    }
    ports
}

/// [`machine_command`] の下で、personacore が既定で使う置き場所（`<データディレクトリ>/personacore`）。
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

/// **偽の `llama` を置いたディレクトリ**（本物を起こさないための保険。正しい実装は呼ばない）。
fn fake_llama_dir(root: &Path) -> PathBuf {
    let bin_dir = root.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("置き場所を作れること");
    let target = bin_dir.join(format!("llama{}", std::env::consts::EXE_SUFFIX));
    if !target.exists() {
        std::fs::copy(env!("CARGO_BIN_EXE_fake_llama"), &target).expect("偽の llama を置けること");
    }
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
/// `port` は `PERSONACORE_PORT`。`extra` で環境変数をもう何組か立てる（`PERSONACORE_DB_PATH` 等）。
fn machine_command(root: &Path, port: u16, args: &[&str], extra: &[(&str, &Path)]) -> Command {
    let bin_dir = fake_llama_dir(root);
    let mut command = Command::new(env!("CARGO_BIN_EXE_personacore"));
    for (key, _) in std::env::vars() {
        if key.starts_with("PERSONACORE_") {
            command.env_remove(key);
        }
    }
    command
        .env("LOCALAPPDATA", root.join("localappdata"))
        .env("XDG_DATA_HOME", root.join("xdg"))
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("PERSONACORE_UPSTREAMS", root.join("no-such-upstreams.json"))
        .env("PERSONACORE_PORT", port.to_string())
        .env("PERSONACORE_LLM_ENDPOINT", DECOY_ENDPOINT)
        .env("PATH", prepended_path(&bin_dir))
        .args(args)
        .stdin(Stdio::null());
    for (key, value) in extra {
        command.env(key, value);
    }
    command
}

/// **1つ目として起こす**（標準出力を行で読み続ける）。
fn start_first(root: &Path, port: u16, args: &[&str], extra: &[(&str, &Path)]) -> Started {
    let mut command = machine_command(root, port, args, extra);
    command.stdout(Stdio::piped()).stderr(Stdio::null());
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

/// **終わるまで起こす。上限つき**——`.output()` は上限なしで待つ（壊れた実装が待ち受けると返らない）。
fn run_to_end(mut command: Command, what: &str) -> Ended {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut running = Running(command.spawn().expect("実行ファイルを起こせること"));
    let stdout = running.0.stdout.take().expect("標準出力を取れること");
    let stderr = running.0.stderr.take().expect("標準エラーを取れること");
    let read_all = |pipe: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut BufReader::new(pipe), &mut text);
            text
        })
    };
    let stdout = read_all(Box::new(stdout));
    let stderr = read_all(Box::new(stderr));
    let mut status = None;
    for _ in 0..MAX_WAITS {
        if let Some(done) = running.0.try_wait().expect("プロセスの状態を見られること")
        {
            status = Some(done);
            break;
        }
        std::thread::sleep(WAIT_STEP);
    }
    let Some(status) = status else {
        drop(running);
        panic!(
            "{what} が {MAX_WAITS} 回待っても終了しなかった（待ち受けた？）\n{}",
            stdout.join().unwrap_or_default()
        );
    };
    Ended {
        status,
        stdout: strip_ansi(&stdout.join().expect("標準出力を読み切れること")),
        stderr: stderr.join().expect("標準エラーを読み切れること"),
    }
}

/// **ブラウザの代わりに呼ばれる偽物が書き残した行**（1回の呼び出しが1行 / `fake_recorder.rs`）。
fn recorded_calls(log: &Path) -> Vec<Vec<String>> {
    match std::fs::read_to_string(log) {
        Ok(body) => body
            .lines()
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|err| panic!("記録を読めること: {err} / {line}"))
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// **この OS で、ブラウザを開くコマンドが受け取るはずの引数。**
///
/// **期待値を `src/server/browser.rs` から作らない**（CLAUDE.md `16b`）。
fn expected_opener_args(url: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            "/C".to_owned(),
            "start".to_owned(),
            String::new(),
            url.to_owned(),
        ]
    } else {
        vec![url.to_owned()]
    }
}

/// **2つ目が、1つ目の URL を言って 0 で終わり、待ち受けず、Enter を待つと言わなかったこと。**
fn assert_guided_to(ended: &Ended, first_port: u16) {
    let running_line = format!("{RUNNING_HEAD}http://127.0.0.1:{first_port}/{RUNNING_TAIL}");
    assert!(
        ended.status.success(),
        "2つ目が 0 で終わっていない（{:?}）\n{}\n{}",
        ended.status,
        ended.stdout,
        ended.stderr
    );
    assert!(
        ended.stdout.contains(&running_line),
        "2つ目が 1つ目の URL を言っていない（{running_line}）\n{}\n{}",
        ended.stdout,
        ended.stderr
    );
    assert!(
        !ended.stdout.contains(LISTENING_LINE),
        "2つ目が待ち受けた（二重起動）\n{}",
        ended.stdout
    );
    assert!(
        !ended.stderr.contains("Enter"),
        "標準入力が端末でないのに、Enter を待つと言った\n{}",
        ended.stderr
    );
}

// ----------------------------------------------------------------------
// 本命
// ----------------------------------------------------------------------

/// **同じデータで2つ目を起こすと、1つ目の URL を言って 0 で終わり、待ち受けない。**
/// **2つ目に別の `PERSONACORE_PORT` を渡しても止まる**（ポートの重なりに頼らない / ADR-0057 §決定2）。
#[test]
fn a_second_start_on_the_same_data_names_the_first_and_listens_nowhere() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [first_port, second_port] = distinct_free_ports::<2>();

    let mut first = start_first(root, first_port, &["--no-ui", "--no-browser"], &[]);
    let (port, _) = first.port_after(AFTER_ANNOUNCE_NO_UI);
    assert_eq!(port, first_port, "前提: 1つ目が渡した番号で待ち受けること");

    let ended = run_to_end(
        machine_command(root, second_port, &["--no-ui", "--no-browser"], &[]),
        "2つ目",
    );
    assert_guided_to(&ended, first_port);
    assert!(
        !ended.stdout.contains(NOT_USED_TAIL),
        "旗を渡していないのに、使わなかったと言った\n{}",
        ended.stdout
    );
    // **2つ目の番号は空いたまま**（待ち受けを開いて閉じた、でもない限り。上の行の不在と対で見る）。
    TcpListener::bind(("127.0.0.1", second_port))
        .expect("2つ目に渡した番号が空いていること（2つ目は待ち受けていない）");
    assert!(first.running.is_alive(), "1つ目が止まった");
}

/// **2つ目の `--fresh` は、動いている側の DB を退避しない。`--upstream` も読まない。使わなかったと言う。**
///
/// **`--upstream` には無い名前を渡す**——読めば「選べませんでした」で 0 以外になる
/// （`PERSONACORE_UPSTREAMS` は存在しないファイル）。
#[test]
fn a_second_start_uses_neither_fresh_nor_upstream_and_says_so() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [first_port, second_port] = distinct_free_ports::<2>();

    let mut first = start_first(root, first_port, &["--no-ui", "--no-browser"], &[]);
    first.port_after(AFTER_ANNOUNCE_NO_UI);
    let app_dir = app_dir_of(root);
    let database = app_dir.join("personacore.sqlite");
    assert!(
        database.exists(),
        "前提: 1つ目の DB が既定の置き場所に在ること"
    );
    let names_before = file_names(&app_dir);

    let ended = run_to_end(
        machine_command(
            root,
            second_port,
            &[
                "--no-ui",
                "--no-browser",
                "--fresh",
                "--upstream",
                "no-such-profile",
            ],
            &[],
        ),
        "--fresh を付けた2つ目",
    );
    assert_guided_to(&ended, first_port);
    let not_used = format!("{NOT_USED_HEAD}--fresh、--upstream{NOT_USED_TAIL}");
    assert!(
        ended.stdout.contains(&not_used),
        "使わなかった旗を言っていない（{not_used}）\n{}",
        ended.stdout
    );
    assert!(database.exists(), "動いている側の DB が退避された");
    let names_after = file_names(&app_dir);
    assert!(
        !names_after.iter().any(|name| name.contains(".bak-")),
        "退避したファイルが出来ている: {names_after:?}"
    );
    assert_eq!(
        names_before, names_after,
        "2つ目が置き場所のファイルを増やした・減らした"
    );
    assert!(first.running.is_alive(), "1つ目が止まった");
}

/// ディレクトリの中の名前（並べ替え済み）。
fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("置き場所を読めること")
        .map(|entry| {
            entry
                .expect("項目を読めること")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// **1つ目を PID で止める（kill = 異常終了）と、同じデータでまた起動できる**（ロックは OS が外す）。
///
/// **止めた後の起動は `PERSONACORE_PORT=0`（OS に選ばせる）**——3つ目が言う URL は、設定の番号（`0`）ではなく
/// **実際に待ち受けた番号**であること（URL を `local_addr` から書くこと）をここで見る。ほかの腕は設定と実際が同じ番号。
#[test]
fn a_killed_first_leaves_no_lock_behind() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [first_port, third_port] = distinct_free_ports::<2>();

    let mut first = start_first(root, first_port, &["--no-ui", "--no-browser"], &[]);
    first.port_after(AFTER_ANNOUNCE_NO_UI);
    first.running.0.kill().expect("1つ目を止められること");
    first.running.0.wait().expect("1つ目の終わりを待てること");

    let mut again = start_first(root, 0, &["--no-ui", "--no-browser"], &[]);
    let (port, _) = again.port_after(AFTER_ANNOUNCE_NO_UI);
    assert_ne!(port, 0, "前提: 実際に掴んだ番号を言うこと");

    // **後の起動が言う URL は、新しい起動の実際の番号**（設定の `0` を言わない）。
    // kill で残った古い中身を消すことは、`src/server/instance_lock.rs` の単体テストが見る
    // （ここでは OS が同じ番号を選び直しうるので、古い URL と見分けられない）。
    let ended = run_to_end(
        machine_command(root, third_port, &["--no-ui", "--no-browser"], &[]),
        "3つ目",
    );
    assert_guided_to(&ended, port);
}

/// **データのディレクトリ（`PERSONACORE_DB_PATH`）が違えば、2つ同時に動く。**
///
/// **`LOCALAPPDATA` は同じ**——ロックが DB の置き場所に付く（OS のデータディレクトリに付かない）ことを見る。
#[test]
fn different_database_paths_run_side_by_side() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [first_port, second_port] = distinct_free_ports::<2>();
    let first_db = root.join("data-a").join("personacore.sqlite");
    let second_db = root.join("data-b").join("personacore.sqlite");

    let mut first = start_first(
        root,
        first_port,
        &["--no-ui", "--no-browser"],
        &[("PERSONACORE_DB_PATH", &first_db)],
    );
    first.port_after(AFTER_ANNOUNCE_NO_UI);
    let mut second = start_first(
        root,
        second_port,
        &["--no-ui", "--no-browser"],
        &[("PERSONACORE_DB_PATH", &second_db)],
    );
    let (port, _) = second.port_after(AFTER_ANNOUNCE_NO_UI);
    assert_eq!(port, second_port, "別のデータの2つ目が待ち受けていない");
    assert!(first.running.is_alive(), "1つ目が止まった");
    assert!(second.running.is_alive(), "2つ目が止まった");

    // **対照: 同じ `PERSONACORE_DB_PATH` なら止まる**（上の緑が「ロックが何も止めない」から来ていない）。
    let [third_port] = distinct_free_ports::<1>();
    let ended = run_to_end(
        machine_command(
            root,
            third_port,
            &["--no-ui", "--no-browser"],
            &[("PERSONACORE_DB_PATH", &first_db)],
        ),
        "1つ目と同じデータの3つ目",
    );
    assert_guided_to(&ended, first_port);
}

/// **`--version` はロックを見ない**——1つ目が動いている間も、版を言って 0 で終わる。
#[test]
fn the_version_flag_answers_while_another_one_runs() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [first_port, second_port] = distinct_free_ports::<2>();

    let mut first = start_first(root, first_port, &["--no-ui", "--no-browser"], &[]);
    first.port_after(AFTER_ANNOUNCE_NO_UI);

    let ended = run_to_end(
        machine_command(root, second_port, &["--version"], &[]),
        "--version",
    );
    assert!(ended.status.success(), "{:?}", ended.status);
    let prefix = format!("personacore {} (", env!("CARGO_PKG_VERSION"));
    assert!(
        ended.stdout.starts_with(&prefix) && ended.stdout.lines().count() == 1,
        "版の1行だけを言っていない: {:?}",
        ended.stdout
    );
    assert!(first.running.is_alive(), "1つ目が止まった");
}

/// **1つ目が画面を配っていれば、2つ目は開く設定のときだけ、その URL を1回開く。**
///
/// 同じ1つ目に、`--no-browser` の2つ目（開かない）と、旗の無い2つ目（開く）を順に当てる。
#[test]
fn the_second_start_opens_the_screen_only_when_asked_to() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [first_port, second_port] = distinct_free_ports::<2>();
    let log = root.join("browser-calls.jsonl");
    let fake = PathBuf::from(env!("CARGO_BIN_EXE_fake_recorder"));
    let browser: [(&str, &Path); 2] = [
        ("PERSONACORE_TEST_BROWSER", &fake),
        ("FAKE_RECORDER_LOG", &log),
    ];

    // 1つ目は画面を配る（`--no-ui` を付けない）。自分ではブラウザを開かない。
    let mut first = start_first(root, first_port, &["--no-browser"], &browser);
    first.port_after(AFTER_ANNOUNCE_UI);
    assert!(recorded_calls(&log).is_empty(), "前提: 1つ目は開かないこと");

    let stayed = run_to_end(
        machine_command(root, second_port, &["--no-browser"], &browser),
        "--no-browser の2つ目",
    );
    assert_guided_to(&stayed, first_port);
    assert!(
        recorded_calls(&log).is_empty(),
        "--no-browser の2つ目がブラウザを開いた: {:?}",
        recorded_calls(&log)
    );

    let opened = run_to_end(
        machine_command(root, second_port, &[], &browser),
        "旗の無い2つ目",
    );
    assert_guided_to(&opened, first_port);
    let url = format!("http://127.0.0.1:{first_port}/");
    // **偽物は2つ目と別のプロセス**なので、2つ目が終わった後に書き終えることがある。上限つきで待つ。
    let mut calls = recorded_calls(&log);
    for _ in 0..MAX_WAITS {
        if !calls.is_empty() {
            break;
        }
        std::thread::sleep(WAIT_STEP);
        calls = recorded_calls(&log);
    }
    assert_eq!(
        calls,
        vec![expected_opener_args(&url)],
        "2つ目が 1つ目の URL を1回だけ開いていない\n{}",
        opened.stdout
    );
    assert!(first.running.is_alive(), "1つ目が止まった");
}

/// **1つ目が `--no-ui` なら、2つ目は開く設定でも開かず、開かない理由を言う。**
#[test]
fn the_second_start_never_opens_a_screen_the_first_withholds() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let [first_port, second_port] = distinct_free_ports::<2>();
    let log = root.join("browser-calls.jsonl");
    let fake = PathBuf::from(env!("CARGO_BIN_EXE_fake_recorder"));
    let browser: [(&str, &Path); 2] = [
        ("PERSONACORE_TEST_BROWSER", &fake),
        ("FAKE_RECORDER_LOG", &log),
    ];

    let mut first = start_first(root, first_port, &["--no-ui", "--no-browser"], &browser);
    first.port_after(AFTER_ANNOUNCE_NO_UI);

    let ended = run_to_end(
        machine_command(root, second_port, &[], &browser),
        "旗の無い2つ目",
    );
    assert_guided_to(&ended, first_port);
    assert!(
        ended.stdout.contains(WITHHELD_LINE),
        "画面を配っていない相手を開かない理由を言っていない\n{}",
        ended.stdout
    );
    // **偽物が呼ばれていたら、書き終えるまでの間を置いてから見る**（開く腕と同じ待ちの長さは要らない）。
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        recorded_calls(&log).is_empty(),
        "画面を配っていない相手をブラウザで開いた: {:?}",
        recorded_calls(&log)
    );
    assert!(first.running.is_alive(), "1つ目が止まった");
}
