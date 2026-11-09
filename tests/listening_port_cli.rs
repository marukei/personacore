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
//! | `a_remembered_profile_that_collides_with_the_saved_port_is_not_used` | 覚えた名前の事前検査が保存した番号を見ない |
//! | `the_environment_wins_over_the_saved_port` | 環境変数が保存に負ける／`GET /admin/server` の次の起動が環境変数を見ない |
//! | `an_overridden_profile_file_neither_reads_nor_writes_the_saved_port` | `PERSONACORE_UPSTREAMS` の起動が保存を読む・書く |
//! | `an_unreadable_saved_port_warns_and_the_file_is_kept` | 読めないファイルで止まる・黙る・ファイルを消す |
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

/// **隔離した機で起こす。** 親から `PERSONACORE_*` を1つも継がせない。
///
/// **`PERSONACORE_PORT` も `PERSONACORE_UPSTREAMS` も、`extra` で渡したときだけ立つ**
/// （保存を読む腕は既定の置き場所が要るので、`PERSONACORE_UPSTREAMS` を渡さない）。
fn spawn_on_machine(root: &Path, args: &[&str], extra: &[(&str, &std::ffi::OsStr)]) -> Started {
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
        assert_eq!(port, first_port, "保存した番号で待ち受けていない");
        let lines = first.all_lines(early);
        assert_settings_line(&lines, first_port, "saved");

        assert_eq!(
            server_report(port),
            serde_json::json!({
                "listening_port": first_port,
                "listening_port_source": "saved",
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
                "listening_port": first_port,
                "listening_port_source": "saved",
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
    assert_eq!(
        port, second_port,
        "API で保存した番号で、次の起動が待ち受けていない"
    );
    assert_settings_line(&second.all_lines(early), second_port, "saved");

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
    assert_eq!(
        port, saved_port,
        "--upstream の起動が保存した番号で待ち受けていない"
    );
    let lines = started.all_lines(early);
    assert_settings_line(&lines, saved_port, "saved");
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

/// **覚えた名前の事前検査も、保存した番号を見る**（ADR-0057 §決定5「覚えた名前の事前検査も、この番号を見る」）。
///
/// 覚えた名前のプロファイルの gguf の面が、**保存した番号**と同じポートを使う。事前検査が保存した番号を見れば
/// 取り合いで覚えた名前を使わず（今までどおりの振る舞い）、環境変数の既定の経路で**保存した番号**で起きる。
/// **偽の `llama` は1度も呼ばれない**（覚えた名前の起動は起こさない／使わない）。
#[test]
fn a_remembered_profile_that_collides_with_the_saved_port_is_not_used() {
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
    .expect("取り合うプロファイルを置けること");
    std::fs::write(app_dir_of(root).join("selected-upstream.txt"), "beta\n")
        .expect("覚えた名前を置けること");

    let mut started = spawn_on_machine(root, &[], &[]);
    let (port, early) = started.listening_port_keeping_lines();
    assert_eq!(port, saved_port, "保存した番号で待ち受けていない");
    let lines = started.all_lines(early);
    assert!(
        lines
            .iter()
            .any(|line| line.contains(REMEMBERED_UNUSABLE_LINE)
                && has_field(line, "upstream_profile=\"beta\"")),
        "保存した番号と取り合う覚えた名前を、使えないと言っていない\n{}",
        lines.join("\n")
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("設定を読み込みました")
                && has_field(line, "upstream_profile_from=-")
                && has_field(line, "bind_from=saved")),
        "環境変数の既定の経路で、保存した番号で起きたと言っていない\n{}",
        lines.join("\n")
    );
    assert!(!marker.exists(), "使わないはずの上流を起こしている");
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
