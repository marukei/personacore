//! **`--upstream` の結線を、実行ファイルそのもので確かめる**
//! （[ADR-0042](../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md) §決定1 /
//! 作業単位 F-4b-1）。**本物の `llama` にも LM Studio にも繋がない。**
//!
//! # なぜ実行ファイルを起こすのか（`tests/fresh_cli.rs` と同じ理由）
//!
//! **`src/main.rs` は bin なので、統合テストから `use` できない。**
//! **プロファイルを読む1行・環境変数に重ねる1行・計画を渡す1行は、そこにしか無い。**
//! **`if args.fresh()` を常に偽にする変異が 2096 全緑を通った**のと同じ族である。
//!
//! # ⚠ ここでは「起こしたまま止める」腕を置かない
//!
//! **テストから実行ファイルへ送れるのは `kill`（強制終了）だけ**で、
//! **`serve` が返らない＝起こした子が孤児として残る**（2026-09-19 に実測 ——
//! **`cargo test` の出力を掴んだまま、読み手が返らなくなった**）。
//! **だからこのファイルの腕は、どれも「生きた偽物を残さない」——正確には「上限つきで残す」。**
//! **⚠ `budget_line_after_load` は走行の直後に偽物を最大2本残し、`--pc-exit-after`（`FAKE_LIFETIME_SECONDS` 秒）で消える**
//! （レビュー2周目が実測 / 2026-09-19）。**「残存 0」は、その秒数を待ってから数えたときの主張である。**
//! 走行の直後に数えて「1本残っている＝壊れている」と読まないこと。
//!
//! > **⚠ それは「正しい実装のとき」の話である**（レビュー是正8 / 2026-09-19）。
//! > **変異検査で、この約束は破れる** —— 例えば gguf の実在確認を落とす変異は、
//! > **起こさないはずの腕が偽物を起こす**（レビュアーが実際に `llama.exe` を 4 本残した）。
//! > **変異検査の後は、`llama.exe` の残存を数えて、居たら止めること。**
//! > **Windows では、残った偽物が `cargo test` の出力のパイプを掴んで読み手が返らなくなる**
//! > ——**「テストが固まった」と読めるが、原因はこれである。**
//!
//! **一生（起こす → 載る → 止める）を見るのは `tests/upstream_launch.rs`**
//! ——あちらは `App::run_with_upstreams`（`main` が呼ぶ関数そのもの）を直に呼び、
//! **止める合図を送れる。**
//!
//! # 何も無い機で初めて叩く（作業単位 F-4c / Issue origin#23）
//!
//! **下の方の「何も無い機」の節**が、見本を書く・ブラウザを開く・`--version` を見る。
//! **どの腕も利用者の本物の置き場所に届かない**（[`isolate_machine`]。
//! **`PERSONACORE_UPSTREAMS` を渡さない腕は、置き場所に見本を書く**ので、断たないと書く）。
//! **ブラウザは偽物**（`tests/support/fake_recorder.rs`）で、**本物は1度も開かない。**
//!
//! # 何を守っていないか（**先に書く**）
//!
//! * **Ctrl+C の経路（`shutdown_signal`）は通っていない。**
//! * **GPU も VRAM も1バイトも使っていない。**
//! * **本物のブラウザが開くことは見ていない**（`src/server/browser.rs` の「守れないもの」）。
//!   **確かめるのは司令塔の実機の確認。**
//! * **見本の名前を選ぶ2本（`the_sample_names_ggufs_…` / `the_very_first_run_…`）は、
//!   `~` の隔離（`USERPROFILE` / `HOME`）が外れると、この機では gguf が実在するので偽の `llama` が起こされ、
//!   personacore が待ち受けたまま終わらず、30 秒の時間切れで落ちる。** **赤にはなるが、
//!   後ろの「`llama` を呼んでいない」のアサートには届かない**（落ちる理由の名前が違う。
//!   レビュー1周目 / 司令塔が据え置きと判断）。

mod common;

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use common::has_field;
use personacore::server::API_VERSION;

/// **API の版のヘッダの名前**（司令塔の決定の綴り / ADR-0047 §② P-2b）。定数から作らない。
const API_VERSION_HEADER: &str = "X-Personacore-Api-Version";

/// 待つ側の上限（回数）。**「働く側」ではなく「待つ側」に置く**（CLAUDE.md `G-4`）。
const MAX_WAITS: usize = 300;

/// 1回あたりの待ち時間。**上限は `MAX_WAITS` と合わせて 30 秒。**
const WAIT_STEP: Duration = Duration::from_millis(100);

/// **上流が載ったあと、本文の予算が動いたときに出る行**（`context::adopt_refreshed`）。
///
/// **写しである。** `src/server/context.rs` の文面が変わったら、この腕は空振りする
/// ——**空振りは「行が無い」側に倒れるので、A の腕（在ること）が赤になる。**
const BUDGET_CHANGED: &str = "上流のコンテキスト長が変わりました";

/// **腕 A で上流が名乗る窓。** 腕 B とも、明示設定とも、既定とも別の値。
const ARM_A_CTX: u32 = 12_345;

/// **腕 B で上流が名乗る窓。** 腕 A とも、明示設定とも、既定とも別の値。
const ARM_B_CTX: u32 = 23_456;

/// **腕 B で立てる明示設定。** 上流が名乗る2つとも別の値。
const EXPLICIT_CONTEXT: &str = "9876";

/// **プロファイルの本文の面に書く窓**（作業単位 A-13a-1）。既定 8192・抽出の落とし所 16384・
/// 上の3つ・[`ARM_C_CTX`] のどれとも別の値。
const WRITTEN_CTX: u32 = 27_648;

/// **プロファイルに窓を書いた腕で、上流が名乗る窓。** 書いた値とも、上の値とも別。
const ARM_C_CTX: u32 = 19_456;

/// **偽物が自分で終わるまでの秒数。** 生きた偽物を残さないための上限。
///
/// **20 秒では足りなかった**（2026-11-02）。負荷がかかると、載って測り直すより先に偽物が終わり、
/// `a_context_length_written_in_the_profile_reaches_the_executable` が
/// 「main が 300 回待っても ready にならなかった（いま "failed"）」で落ちた
/// （F-10a を含まない `main` でも 3回中1回、F-10a のブランチでは 4回中3回）。
const FAKE_LIFETIME_SECONDS: u64 = 60;

/// **偽物が起きた印。** **一度も呼ばれないこと**を見る腕が読む。
const TOUCH_MARKER: &str = "fake-llama-was-started";

/// **起こした実行ファイルの番人。`Drop` で必ず止める。**
///
/// **[`std::process::Child`] は drop しても kill しない**
/// （`tests/fresh_cli.rs` の `Running` と同じ / レビュー指摘1 / 2026-09-18）。
/// **`panic!` でも巻き戻しは走る**ので、`Drop` に置けば必ず通る。
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

    /// **上限つきで終了を待ち、終了コードを返す。**
    fn wait_for_exit(&mut self, what: &str) -> std::process::ExitStatus {
        for _ in 0..MAX_WAITS {
            if let Some(status) = self.0.try_wait().expect("プロセスの状態を見られること")
            {
                return status;
            }
            std::thread::sleep(WAIT_STEP);
        }
        panic!("{what} を {MAX_WAITS} 回待っても終了しなかった");
    }
}

/// **偽の `llama` を置いたディレクトリ。**
///
/// **名前は `llama`（＋この OS の実行ファイルの接尾辞）** ——
/// プロファイルが `command` を書かなければ、この綴りが `PATH` から引かれる。
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

/// **偽の gguf**（中身は見られない。**実在するかだけ**が見られる）。
fn fake_gguf(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"not a real gguf").expect("置けること");
    path
}

/// **プロファイルを1つ書く。**
fn write_profile(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("upstreams.json");
    std::fs::write(&path, body).expect("プロファイルを置けること");
    path
}

/// **親から `PERSONACORE_*` を1つも継がせない**（`tests/fresh_cli.rs` と同じ）。
///
/// **測定中の機には `PERSONACORE_STAT_DRIFT` などが立っていることがある。**
/// 継ぐと、**このテストの結果が「誰がどの環境変数を立てていたか」に左右される。**
fn without_inherited_settings() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_personacore"));
    for (key, _) in std::env::vars() {
        if key.starts_with("PERSONACORE_") {
            command.env_remove(key);
        }
    }
    command
}

/// **起こした personacore と、その標準出力の行。**
struct Started {
    running: Running,
    lines: mpsc::Receiver<String>,
}

impl Started {
    /// **いままでに読めた行を全部取り出す。**
    ///
    /// **`recv_timeout` で1周ぶん待ってから畳む** ——
    /// 直前に出たはずの行が、まだ読み取りスレッドから届いていないことがある。
    fn collect_lines(&mut self) -> Vec<String> {
        let mut found = Vec::new();
        while let Ok(line) = self.lines.recv_timeout(WAIT_STEP) {
            found.push(line);
        }
        found
    }

    /// **待ち受けを始めた行から、実際のポートを読む。**
    ///
    /// **ポートを決め打ちにしない**（`PERSONACORE_PORT=0` で OS に選ばせる）ので、
    /// 他のテストと取り合わない。
    /// **毎周「書き手が生きているか」を見る**（CLAUDE.md `G-4`）——
    /// **プロセスが死んだ瞬間、その行は永久に来ない。**
    fn listening_port(&mut self) -> u16 {
        self.listening_port_keeping_lines().0
    }

    /// **[`Self::listening_port`] と同じ。読み進めた行（待ち受けの行を含む）も返す**
    /// （作業単位 F-4c）。
    ///
    /// **待ち受けより前に出る行**（見本を置いた行など）は、
    /// [`Self::collect_lines`] からはもう読めない——ここで読み進めて捨てているから。
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
                panic!("待ち受けの行を読む前にプロセスが終了した");
            }
        }
        panic!("待ち受けの行を {MAX_WAITS} 回待っても読めなかった");
    }
}

/// **端末の色づけ（ANSI エスケープ）を落とす。純粋関数。**
///
/// # ⚠ これが無いと、走査は空振りする（**実測 / 2026-09-18**）
///
/// `tracing_subscriber::fmt()` は**既定で色を付ける**ので、
/// 起動ログの欄は `addr` と `=` の間にエスケープが挟まった綴りで出る
/// （`\x1b[3maddr\x1b[0m\x1b[2m=\x1b[0m127.0.0.1:65354`）。
/// **素で `addr=` を探すと1件も見つからない。**
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        // `ESC [ … <終端>` を読み飛ばす（終端は `@`〜`~` の1文字）。
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

/// personacore を起こす。**`PATH` の先頭に偽の `llama` を置く。**
///
/// **囮の `PERSONACORE_LLM_ENDPOINT` を必ず立てる**——
/// **プロファイルが勝つ**ことを、同値のフィクスチャで空振りさせないため。
fn spawn(db_dir: &Path, bin_dir: &Path, profile: Option<&Path>, port: &str, name: &str) -> Started {
    spawn_with(db_dir, bin_dir, profile, port, name, &[])
}

/// **環境変数をもう何組か立てて起こす**（レビュー是正4 / 2026-09-19）。
///
/// **`PERSONACORE_CONTEXT_TOKENS` を立てた起動を作るため。**
fn spawn_with(
    db_dir: &Path,
    bin_dir: &Path,
    profile: Option<&Path>,
    port: &str,
    name: &str,
    extra: &[(&str, &str)],
) -> Started {
    let mut command = without_inherited_settings();
    // **利用者の本物の置き場所へ届く道を断つ**（作業単位 F-4c）。
    // **`PERSONACORE_UPSTREAMS` を渡さない腕は、置き場所に見本を書く**ので、
    // **断たないと `%LOCALAPPDATA%\personacore\upstreams.json` を作る。**
    isolate_machine(&mut command, db_dir);
    command
        .env("PERSONACORE_DB_PATH", db_dir.join("personacore.sqlite"))
        .env("PERSONACORE_PORT", port)
        .env("PERSONACORE_LLM_ENDPOINT", DECOY_ENDPOINT)
        .env("PATH", prepended_path(bin_dir))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in extra {
        command.env(key, value);
    }
    if let Some(profile) = profile {
        command.env("PERSONACORE_UPSTREAMS", profile);
        command.args(["--upstream", name]);
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

/// **囮の上流の宛先**（閉じている loopback のポート）。
///
/// **プロファイルが書く宛先とは必ず別**——同値だと「どちらが勝ったか」が観測できない。
const DECOY_ENDPOINT: &str = "http://127.0.0.1:9";

fn prepended_path(bin_dir: &Path) -> std::ffi::OsString {
    let mut value = std::ffi::OsString::from(bin_dir);
    if let Some(existing) = std::env::var_os("PATH") {
        value.push(if cfg!(windows) { ";" } else { ":" });
        value.push(existing);
    }
    value
}

/// **`GET /admin/upstream` を読む。**
fn upstream_report(port: u16) -> serde_json::Value {
    let url = format!("http://127.0.0.1:{port}/admin/upstream");
    let body = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime を作れること")
        .block_on(async move {
            reqwest::get(&url)
                .await
                .expect("管理 API へ繋がること")
                .text()
                .await
                .expect("本文を読めること")
        });
    serde_json::from_str(&body)
        .unwrap_or_else(|err| panic!("JSON として読めること: {err} / {body}"))
}

// ----------------------------------------------------------------------
// 本命
// ----------------------------------------------------------------------

/// **旗を渡さない起動では、どちらの面も「起こしていない」。**
///
/// **対比の腕**（CLAUDE.md「対比は安い側だけでは足りない」）——
/// これが無いと、**常に `loading` を書く実装**がここで緑を通る。
/// **偽の `llama` は `PATH` に在るのに、一度も呼ばれない**ことまで見る。
#[test]
fn without_the_flag_no_face_is_launched() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let bin_dir = fake_llama_dir(dir.path());
    let marker = dir.path().join(TOUCH_MARKER);

    let mut started = spawn(dir.path(), &bin_dir, None, "0", "unused");
    let port = started.listening_port();

    let report = upstream_report(port);
    for face in ["main", "sub"] {
        assert_eq!(
            report[face]["launch"]["state"],
            serde_json::json!("external"),
            "旗が無いのに起こしている（{face}）: {report}"
        );
    }
    // **上流の宛先も、囮のまま**（プロファイルを読みに行っていない）。
    assert_eq!(
        report["main"]["endpoint"],
        serde_json::json!(format!("{DECOY_ENDPOINT}/v1/chat/completions")),
        "旗が無いのに上流が差し替わっている: {report}"
    );
    assert!(!marker.exists(), "偽の llama が呼ばれている");
}

/// **旗を渡すと、プロファイルの4つが環境変数に勝って、実行ファイルへ届く。**
///
/// # なぜ「即終了する偽物」を使うのか
///
/// **このファイルは生きた偽物を残さない**（モジュールの doc）。
/// **起こすところまでは通したい**ので、偽物には `--pc-exit` を渡す
/// ——**起こされた印は残り、プロセスは自分で終わる。**
///
/// # 何を1本で見ているか
///
/// | 主張 | 破れると |
/// |---|---|
/// | **宛先とモデルがプロファイルから来る** | 旗を渡した意味が消える（環境変数が勝つ） |
/// | **抽出の面も一緒に動く** | `sub` を省いた形（ADR-0039 §決定1）が壊れる |
/// | 偽物が実際に起きる（印のファイル） | 起こす経路が死んでいる（`F-9` の回帰そのもの） |
#[test]
fn the_named_profile_reaches_the_executable_and_wins_over_the_environment() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let bin_dir = fake_llama_dir(dir.path());
    let gguf = fake_gguf(dir.path(), "planted-body-model.gguf");
    let marker = dir.path().join(TOUCH_MARKER);
    let upstream_port = free_port();

    let endpoint = format!("http://127.0.0.1:{upstream_port}");
    assert_ne!(endpoint, DECOY_ENDPOINT, "囮と同じ宛先では何も見ていない");

    let profile = write_profile(
        dir.path(),
        &serde_json::json!({
            "planted": {
                "main": {
                    "endpoint": endpoint,
                    "model": "planted-body-model-id",
                    "gguf": gguf,
                    // **起こした印だけ残して、自分で終わる。**
                    "args": ["--pc-touch", marker, "--pc-exit", "0"],
                }
            }
        })
        .to_string(),
    );

    let mut started = spawn(dir.path(), &bin_dir, Some(&profile), "0", "planted");
    let port = started.listening_port();

    // **起こされたこと**は印で見る（起こす経路が死んでいれば、これが無い）。
    for _ in 0..MAX_WAITS {
        if marker.exists() {
            break;
        }
        std::thread::sleep(WAIT_STEP);
    }
    assert!(marker.exists(), "偽の llama が一度も呼ばれていない");

    let report = upstream_report(port);
    assert_eq!(
        report["main"]["endpoint"],
        serde_json::json!(format!("{endpoint}/v1/chat/completions")),
        "環境変数がプロファイルに勝っている: {report}"
    );
    assert_eq!(
        report["main"]["model"],
        serde_json::json!("planted-body-model-id"),
        "依頼する綴りもプロファイルから来ること: {report}"
    );
    // **`sub` を省いたプロファイルは、抽出も本文と同じ上流**（ADR-0039 §決定1）。
    assert_eq!(
        report["sub"]["endpoint"], report["main"]["endpoint"],
        "sub を省いたのに、抽出だけ別の上流を向いている: {report}"
    );
    assert_eq!(
        report["sub"]["model"],
        serde_json::json!("planted-body-model-id"),
        "sub を省いたのに、抽出のモデルが本文と違う: {report}"
    );

    // **`PERSONACORE_UPSTREAMS` を立てた起動は、既定の置き場所にも見本を書かない**
    // （作業単位 F-4c / レビュー1周目の指摘2 / r01）。
    //
    // **「指した先に書かない」だけでは足りなかった**——**指した先は在るので、
    // 既定の置き場所へ書き、画面に「書いた」と言わせる変異が 2197 全緑を通った。**
    // **ファイルと画面の口の両方を見る**（片方だけだと、書いて黙る／書かずに言う、の片方が通る）。
    assert!(
        !app_dir_of(dir.path()).join("upstreams.json").exists(),
        "PERSONACORE_UPSTREAMS を立てたのに、既定の置き場所に見本を書いている"
    );
    assert_eq!(
        report["profile_sample"],
        serde_json::Value::Null,
        "PERSONACORE_UPSTREAMS を立てたのに、見本を書いたと言っている: {report}"
    );
}

/// **gguf が無ければ、1つも起こさずに止まる**（ADR-0042 §決定2）。
///
/// **実行ファイルが 0 以外で終わること**まで見る——
/// **黙って起動すると、利用者は指定したつもりで別の上流と話す。**
#[test]
fn a_missing_gguf_stops_the_executable() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let bin_dir = fake_llama_dir(dir.path());
    let marker = dir.path().join(TOUCH_MARKER);

    let profile = write_profile(
        dir.path(),
        &serde_json::json!({
            "planted": {
                "main": {
                    "endpoint": "http://127.0.0.1:18080",
                    "model": "planted-body-model-id",
                    "gguf": dir.path().join("never-here.gguf"),
                    "args": ["--pc-touch", marker],
                }
            }
        })
        .to_string(),
    );

    let mut started = spawn(dir.path(), &bin_dir, Some(&profile), "0", "planted");
    let status = started.running.wait_for_exit("gguf が無い起動");

    assert!(
        !status.success(),
        "gguf が無いのに起動している（{status:?}）"
    );
    assert!(
        !marker.exists(),
        "1つも起こさないはずが、偽の llama が呼ばれている"
    );
}

/// **在らない名前を渡したら止まる**（黙って既定へ落ちない / ADR-0042 §決定1）。
#[test]
fn an_unknown_profile_name_stops_the_executable() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let bin_dir = fake_llama_dir(dir.path());
    let profile = write_profile(
        dir.path(),
        r#"{ "alpha": { "main": { "endpoint": "http://127.0.0.1:1234", "model": "m" } } }"#,
    );

    let mut started = spawn(dir.path(), &bin_dir, Some(&profile), "0", "gamma");
    let status = started.running.wait_for_exit("在らない名前の起動");

    assert!(
        !status.success(),
        "在らない名前で起動している（{status:?}）"
    );
}

/// **`--upstream` を渡さない起動でも、プロファイルの置き場所は走ったままの差し替えへ届く**
/// （ADR-0042 §決定4 / 作業単位 F-4b-2 / `main` の `with_profiles_file` の1行）。
///
/// **その1行は `src/main.rs`（bin）にしか無い**ので、`tests/upstream_launch.rs`
/// （`App::run_with_upstreams` を直に呼ぶ）からは観測できない。**落とすと、画面から選んでも
/// 永久に 409（`upstream_switch_unavailable`）になる。**
///
/// **gguf の無いプロファイルだけを使う**——このファイルは生きた偽物を残さない（モジュールの doc）。
/// **宛先は囮とも、起動時の宛先とも別**（同値だと「差し替わった」が観測できない）。
#[test]
fn the_executable_hands_the_profile_file_to_the_switch() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let bin_dir = fake_llama_dir(dir.path());
    let target = format!("http://127.0.0.1:{}", free_port());
    assert_ne!(target, DECOY_ENDPOINT, "前提: 囮とは別の宛先であること");
    let profile = write_profile(
        dir.path(),
        &serde_json::json!({
            "switched": { "main": { "endpoint": target, "model": "planted-switched-model" } }
        })
        .to_string(),
    );
    let profile_value = profile.display().to_string();

    // **旗は渡さない。置き場所だけを環境変数で渡す。**
    let mut started = spawn_with(
        dir.path(),
        &bin_dir,
        None,
        "0",
        "unused",
        &[("PERSONACORE_UPSTREAMS", &profile_value)],
    );
    let port = started.listening_port();

    let (status, body) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime を作れること")
        .block_on(async move {
            let response = reqwest::Client::new()
                .post(format!("http://127.0.0.1:{port}/admin/upstream/profile"))
                .json(&serde_json::json!({ "name": "switched" }))
                .send()
                .await
                .expect("管理 API へ繋がること");
            let status = response.status().as_u16();
            let text = response.text().await.expect("本文を読めること");
            (status, text)
        });
    assert_eq!(status, 200, "置き場所が差し替えに届いていない: {body}");

    let report = upstream_report(port);
    assert_eq!(
        report["main"]["endpoint"],
        serde_json::json!(format!("{target}/v1/chat/completions")),
        "差し替えた宛先を向いていない: {report}"
    );
    assert_eq!(
        report["main"]["model"],
        serde_json::json!("planted-switched-model"),
        "{report}"
    );
}

/// **プロファイルのファイルが無ければ止まる**（同上）。
#[test]
fn a_missing_profile_file_stops_the_executable() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let bin_dir = fake_llama_dir(dir.path());

    let mut started = spawn(
        dir.path(),
        &bin_dir,
        Some(&dir.path().join("never-here.json")),
        "0",
        "planted",
    );
    let status = started.running.wait_for_exit("プロファイルが無い起動");

    assert!(
        !status.success(),
        "プロファイルが無いのに起動している（{status:?}）"
    );
    // **`PERSONACORE_UPSTREAMS` が指した場所には、見本を書かない**（作業単位 F-4c / `main` の doc）。
    //
    // **書くと、この腕は「在らない名前」で止まる形に変わって緑のまま通る**——
    // **綴り違いの場所に見本が出来る**実害を、止まったかどうかでは観測できない。
    assert!(
        !dir.path().join("never-here.json").exists(),
        "PERSONACORE_UPSTREAMS が指した場所に見本を書いている"
    );
    // **既定の置き場所にも書かない**（レビュー1周目の指摘2 / r01。画面の口は
    // `the_named_profile_reaches_the_executable_and_wins_over_the_environment` が見る——
    // この腕は止まるので、応答を読めない）。
    assert!(
        !app_dir_of(dir.path()).join("upstreams.json").exists(),
        "PERSONACORE_UPSTREAMS を立てたのに、既定の置き場所に見本を書いている"
    );
}

/// **明示設定があるときは、載っても本文の予算を動かさない**（ADR-0020 §4 /
/// レビュー是正4 / 2026-09-19）。
///
/// # 何を見ているか（**両向きを1本に置く**）
///
/// **上流が載ったあと、`context::adopt_refreshed` が出す1行**
/// （[`BUDGET_CHANGED`]）**が出るか出ないか。**
///
/// | 腕 | `PERSONACORE_CONTEXT_TOKENS` | 期待 |
/// |---|---|---|
/// | A | **立てない** | **その行が出る**（上流の窓を採った） |
/// | B | **立てる** | **その行が出ない**（明示設定が優先される） |
///
/// **`main_budget_is_explicit` を `true` に決め打つ変異は A を殺し、
/// `false` に決め打つ変異は B を殺す。**
/// **是正前は両方とも 2157 全緑だった**（決め打ちに見張り番が1本も無かった）。
///
/// > **作業単位 A-13a-1 で `main_budget_is_explicit` は無くなった**——「明示か」は
/// > 予算の器が持つ印になり（`context::SharedBudget` / ADR-0054 §決定3）、起動時の印は
/// > `App::bind_launching` の `context::Chosen::explicit` / `Chosen::measured` が決める。
/// > **上の2つの決め打ちに当たる変異は、その2行の決め打ち**である。
///
/// # ⚠ ログは「同じ行の中で」見る（CLAUDE.md `B-1-2`）
///
/// **腕ごとに違う窓**（[`ARM_A_CTX`] / [`ARM_B_CTX`]）**を上流に名乗らせ、
/// その値を同じ行の中で突き合わせる。** 行だけを数えると、
/// **もう一方の腕の行や、抽出側の似た文面**（`サブモデルのコンテキスト長が変わりました`）
/// **を拾う。**
///
/// # ⚠ この腕は生きた偽物を残さない（このファイルの約束）
///
/// **偽物は `--pc-exit-after` で自分から終わる。**
/// **載って、窓を名乗って、personacore がそれを採るところまでは通る。**
#[test]
fn an_explicit_context_setting_survives_the_upstream_that_was_started() {
    // **A: 明示設定なし** —— 上流の窓を採るので、その行が出る。
    let (seen_a, lines_a) = budget_line_after_load(None, None, ARM_A_CTX);
    let line = seen_a.unwrap_or_else(|| {
        panic!(
            "明示設定が無いのに、載った上流の窓を採っていない（{BUDGET_CHANGED} の行が無い）
{}",
            lines_a.join(
                "
"
            )
        )
    });
    assert!(
        has_field(&line, &format!("context_tokens={ARM_A_CTX}")),
        "採った窓が上流の名乗りと違う: {line}"
    );

    // **B: 明示設定あり** —— 優先されるので、その行は出ない。
    //
    // **⚠ 「出ない」を確かめるには、出るはずの時点まで待つ必要がある。**
    // **`ready` になるまで待ってから読む**ので、待ち足りずに空振りはしない。
    let (seen_b, _) = budget_line_after_load(Some(EXPLICIT_CONTEXT), None, ARM_B_CTX);
    assert!(
        seen_b.is_none(),
        "明示設定があるのに、載った上流の窓で予算を上書きしている: {seen_b:?}"
    );
}

/// **上流プロファイルの本文の面に `context_tokens` を書き、`--upstream` で起こした実行ファイルが、
/// その値で起動し、載った上流の窓で上書きしない**（ADR-0054 §決定1 / 作業単位 A-13a-1 のレビュー是正）。
///
/// # 何を守るか
///
/// **`main.rs` の `config_for` が `AppConfig::with_profile_context` を呼ぶこと**——bin なので、
/// **実行ファイルを起こさない限り観測できない**（`tests/server_profile_context.rs` は
/// `main` と同じ2段を自分で組んで `App::bind` へ渡すので、ここが外れても緑のまま）。
///
/// # 見るもの（どちらも**同じ行の中**で、値で絞る）
///
/// * **待ち受けを始めた行（`addr=`）の `context_tokens=` が、書いた値**（[`WRITTEN_CTX`]）
/// * **載った上流が別の窓（[`ARM_C_CTX`]）を名乗っても、[`BUDGET_CHANGED`] の行が出ない**
///
/// **値はどれも違える**——既定 8192・抽出の落とし所 16384・上流の名乗り・環境変数（立てない）。
#[test]
fn a_context_length_written_in_the_profile_reaches_the_executable() {
    let (seen, lines) = budget_line_after_load(None, Some(WRITTEN_CTX), ARM_C_CTX);
    let startup = lines
        .iter()
        .find(|line| port_in(line).is_some())
        .unwrap_or_else(|| panic!("待ち受けの行が無い:\n{}", lines.join("\n")));
    assert!(
        has_field(startup, &format!("context_tokens={WRITTEN_CTX}")),
        "プロファイルに書いた本文の窓（{WRITTEN_CTX}）で起動していない（main の config_for が載せていない）: {startup}"
    );
    assert!(
        seen.is_none(),
        "プロファイルに書いた窓を、載った上流の窓（{ARM_C_CTX}）で上書きしている: {seen:?}"
    );
}

/// 本文の面の鍵（囮 / 作業単位 A-13b-1）。
const CLI_MAIN_KEY: &str = "pc-cli-main-KEY-31a4c5e9";
/// 裏方の面の鍵（囮）。**本文の鍵と別の綴り・別の末尾4文字。**
const CLI_SUB_KEY: &str = "pc-cli-sub-KEY-b7d2f0a6";

/// **プロファイルに書いた鍵が実行ファイルへ届き、ログにも応答にも出ない**
/// （[ADR-0056](../docs/adr/0056-a-key-for-the-upstream.md) §決定1〜4 / 作業単位 A-13b-1）。
///
/// # なぜ実行ファイルを起こすのか
///
/// **鍵を設定に載せる1行（`config_for` の `with_profile_keys`）と、環境変数とぶつかったことを
/// 言う `warn_on_conflicts` は `src/main.rs` にしか無い**（bin なので統合テストから `use` できない）。
///
/// # 何を1本で見ているか
///
/// | 主張 | 見るもの |
/// |---|---|
/// | 本文と裏方に、それぞれの鍵が届く（別の上流・別の鍵） | 偽の上流が受けた `Authorization` |
/// | 上流の画面が「鍵を断られた」と言う | `GET /admin/upstream` の `unauthorized` |
/// | 一覧は鍵を値で返さず、印と末尾4文字だけ | `GET /admin/upstream/profiles` |
/// | **鍵がログに出ない**（`warn_on_conflicts`・上流が鍵を写し返した 401 の本文） | 標準出力の全行 |
///
/// **偽の上流は 401 を返し、本文に鍵を写し返す**（鍵で守られた中継は写し返さないが、ほかは確かめていない）。
/// **ログの肯定側**（ぶつかった行・本文が載る行が出ていること）も見る —— 行が無くて緑、にしない。
#[test]
fn a_key_written_in_the_profile_reaches_the_executable_and_never_its_log() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime を作れること");
    let refusing = |key: &'static str| {
        runtime.block_on(async move {
            let server = wiremock::MockServer::start().await;
            wiremock::Mock::given(wiremock::matchers::any())
                .respond_with(wiremock::ResponseTemplate::new(401).set_body_raw(
                    format!(r#"{{"error":"Invalid API key {key}","marker":"cli-echo-marker"}}"#),
                    "application/json",
                ))
                .mount(&server)
                .await;
            server
        })
    };
    let main_upstream = refusing(CLI_MAIN_KEY);
    let sub_upstream = refusing(CLI_SUB_KEY);

    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let bin_dir = fake_llama_dir(dir.path());
    let profile = write_profile(
        dir.path(),
        &serde_json::json!({
            "keyed": {
                "main": { "endpoint": main_upstream.uri(), "model": "cli-main-model",
                          "api_key": CLI_MAIN_KEY },
                "sub": { "endpoint": sub_upstream.uri(), "model": "cli-sub-model",
                         "api_key": CLI_SUB_KEY },
            }
        })
        .to_string(),
    );

    // **`PERSONACORE_LLM_ENDPOINT` は囮が立っている**（`spawn_with`）——プロファイルが勝つので、
    // `warn_on_conflicts` が値を並べる行が出る。そこに鍵を混ぜないことを見る。
    let mut started = spawn(dir.path(), &bin_dir, Some(&profile), "0", "keyed");
    let (port, mut lines) = started.listening_port_keeping_lines();

    let report_text = serde_json::to_string(&upstream_report(port)).expect("JSON を書けること");
    let report: serde_json::Value = serde_json::from_str(&report_text).expect("JSON");
    let (status, profiles) = call(port, reqwest::Method::GET, "/admin/upstream/profiles", None);
    assert_eq!(status, 200, "{profiles}");
    lines.extend(started.collect_lines());

    // **それぞれの上流に、それぞれの鍵だけが届いた。**
    for (server, own) in [(&main_upstream, CLI_MAIN_KEY), (&sub_upstream, CLI_SUB_KEY)] {
        let seen: Vec<Option<String>> = runtime.block_on(async {
            server
                .received_requests()
                .await
                .expect("記録が有効であること")
                .iter()
                .map(|request| {
                    request
                        .headers
                        .get("authorization")
                        .map(|value| value.to_str().expect("ASCII").to_owned())
                })
                .collect()
        });
        assert!(!seen.is_empty(), "前提: 起動時の照会が届いたこと（{own}）");
        assert!(
            seen.iter()
                .all(|header| header.as_deref() == Some(format!("Bearer {own}").as_str())),
            "その上流の鍵以外が届いた（main の config_for が鍵を載せていない？）: {seen:?}"
        );
    }

    // **上流の画面は、両面とも鍵を断られたと言う。**
    assert_eq!(
        (
            &report["main"]["unauthorized"],
            &report["sub"]["unauthorized"]
        ),
        (&serde_json::json!(true), &serde_json::json!(true)),
        "{report}"
    );
    // **一覧は印と末尾4文字だけ。**
    let keyed = &profiles["profiles"][0];
    assert_eq!(
        (
            &keyed["main"]["api_key_set"],
            &keyed["main"]["api_key_hint"],
            &keyed["sub"]["api_key_set"],
            &keyed["sub"]["api_key_hint"]
        ),
        (
            &serde_json::json!(true),
            &serde_json::json!("c5e9"),
            &serde_json::json!(true),
            &serde_json::json!("f0a6")
        ),
        "{profiles}"
    );

    // **ログの肯定側**: ぶつかった行と、上流の本文が載る行は出ている。
    let log = lines
        .iter()
        .map(|line| strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        log.contains("環境変数よりも選んだ上流プロファイルを優先します"),
        "前提: warn_on_conflicts の行が出ていること:\n{log}"
    );
    assert!(
        log.contains("cli-echo-marker"),
        "前提: 上流が写し返した本文がログに載る経路を通ったこと:\n{log}"
    );
    for key in [CLI_MAIN_KEY, CLI_SUB_KEY] {
        assert!(!log.contains(key), "ログに鍵が出ている（{key}）:\n{log}");
        assert!(
            !report_text.contains(key),
            "上流の画面に鍵が出ている: {report_text}"
        );
        assert!(
            !profiles.to_string().contains(key),
            "一覧に鍵が出ている: {profiles}"
        );
    }
}

/// **宛先に書いた秘密（ユーザー情報・クエリ・フラグメント）は、起動のログのどの行にも出ない**
/// （作業単位 B-71 / Issue origin#100）。**上流の画面（`GET /admin/upstream`）にも出ない。**
///
/// 着手前は「設定を読み込みました」の `upstream=` / `sub_upstream=`、「待ち受けを開始しました」の
/// `llm_endpoint=` / `sub_llm_endpoint=` に書いたままの宛先が出ていた（司令塔が起動して確かめた）。
///
/// # 1回の起動で、宛先を出す行を全部通す
///
/// | 行 | どう通すか |
/// |---|---|
/// | 設定を読み込みました（`src/main.rs`） | 起動すれば出る |
/// | 環境変数よりも選んだ上流プロファイルを優先します（`src/main.rs`） | 囮の `PERSONACORE_LLM_ENDPOINT` と、秘密を入れた `PERSONACORE_SUB_LLM_ENDPOINT` を立てる |
/// | 末尾が /v1 の警告（`llm::lmstudio`） | 本文の面の宛先を `/v1` で終える |
/// | 上流を起こします（`server::launch`） | 本文の面に gguf を書く（偽の `llama`） |
/// | 推論サーバへ接続できませんでした（`reqwest` のエラーの URL） | 裏方の面を閉じたポートへ向ける |
/// | 待ち受けを開始しました（`src/server/mod.rs`） | 起動すれば出る |
///
/// **秘密は面ごと・欄ごとに別の綴り**——1つを落とし忘れた実装を、残りが隠さない。
///
/// # 空振りではないこと
///
/// **同じ捕捉の中で、各行が出ていて、宛先のホストとポートを名指していること**を先に見る。
/// **宛先を丸ごと消す実装**も、**行が出ない実装**も、ここで赤になる。
///
/// **⚠ 偽の `llama` は走行の直後に最大1本残り、`--pc-exit-after` で消える**（このファイルの doc）。
#[test]
fn the_secrets_written_in_an_endpoint_never_reach_the_log_or_the_upstream_report() {
    const MAIN_USER: &str = "pc-b71-main-user";
    const MAIN_PASSWORD: &str = "pc-b71-main-password";
    const MAIN_QUERY: &str = "pc-b71-main-query";
    const MAIN_FRAGMENT: &str = "pc-b71-main-fragment";
    const SUB_USER: &str = "pc-b71-sub-user";
    const SUB_PASSWORD: &str = "pc-b71-sub-password";
    const SUB_QUERY: &str = "pc-b71-sub-query";
    const SUB_FRAGMENT: &str = "pc-b71-sub-fragment";
    const ENV_USER: &str = "pc-b71-env-user";
    const ENV_PASSWORD: &str = "pc-b71-env-password";
    const ENV_QUERY: &str = "pc-b71-env-query";
    const ENV_FRAGMENT: &str = "pc-b71-env-fragment";
    const SECRETS: [&str; 12] = [
        MAIN_USER,
        MAIN_PASSWORD,
        MAIN_QUERY,
        MAIN_FRAGMENT,
        SUB_USER,
        SUB_PASSWORD,
        SUB_QUERY,
        SUB_FRAGMENT,
        ENV_USER,
        ENV_PASSWORD,
        ENV_QUERY,
        ENV_FRAGMENT,
    ];

    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let bin_dir = fake_llama_dir(dir.path());
    let gguf = fake_gguf(dir.path(), "planted-b71-model.gguf");
    let launch_port = free_port();
    // **誰も待ち受けない**（取ってすぐ放す）。裏方の照会は接続で失敗し、`reqwest` のエラーが行に載る。
    let closed_port = free_port();
    assert_ne!(launch_port, closed_port, "前提: 2つの面のポートが違うこと");

    let main_endpoint = format!(
        "http://{MAIN_USER}:{MAIN_PASSWORD}@127.0.0.1:{launch_port}/v1?token={MAIN_QUERY}#{MAIN_FRAGMENT}"
    );
    let sub_endpoint = format!(
        "http://{SUB_USER}:{SUB_PASSWORD}@127.0.0.1:{closed_port}?token={SUB_QUERY}#{SUB_FRAGMENT}"
    );
    let env_sub_endpoint =
        format!("http://{ENV_USER}:{ENV_PASSWORD}@127.0.0.1:9?token={ENV_QUERY}#{ENV_FRAGMENT}");
    let profile = write_profile(
        dir.path(),
        &serde_json::json!({
            "leaky": {
                "main": {
                    "endpoint": main_endpoint,
                    "model": "b71-main-model",
                    "gguf": gguf,
                    "args": ["--pc-exit-after", FAKE_LIFETIME_SECONDS.to_string()],
                },
                "sub": { "endpoint": sub_endpoint, "model": "b71-sub-model" },
            }
        })
        .to_string(),
    );

    let mut started = spawn_with(
        dir.path(),
        &bin_dir,
        Some(&profile),
        "0",
        "leaky",
        &[("PERSONACORE_SUB_LLM_ENDPOINT", env_sub_endpoint.as_str())],
    );
    let (port, early) = started.listening_port_keeping_lines();
    let report_text = serde_json::to_string(&upstream_report(port)).expect("JSON を書けること");
    let (status, profiles) = call(port, reqwest::Method::GET, "/admin/upstream/profiles", None);
    assert_eq!(status, 200, "{profiles}");
    let lines: Vec<String> = early
        .into_iter()
        .chain(started.collect_lines())
        .map(|line| strip_ansi(&line))
        .collect();
    let log = lines.join("\n");

    // **肯定側**: 宛先を出す行が全部出ていて、ホストとポートを名指している。
    let main_at = format!("127.0.0.1:{launch_port}");
    let sub_at = format!("127.0.0.1:{closed_port}");
    let line_of = |message: &str| -> Vec<&String> {
        lines.iter().filter(|line| line.contains(message)).collect()
    };
    for (message, must_name) in [
        (
            "設定を読み込みました",
            vec![main_at.as_str(), sub_at.as_str()],
        ),
        (
            "待ち受けを開始しました",
            vec![main_at.as_str(), sub_at.as_str()],
        ),
        ("上流の宛先の末尾が /v1 です", vec![main_at.as_str()]),
        ("上流を起こします", vec![main_at.as_str()]),
        ("推論サーバへ接続できませんでした", vec![sub_at.as_str()]),
    ] {
        let found = line_of(message);
        assert!(
            found
                .iter()
                .any(|line| must_name.iter().all(|at| line.contains(at))),
            "前提: 「{message}」の行が {must_name:?} を名指して出ていること:\n{log}"
        );
    }
    let conflicts = line_of("環境変数よりも選んだ上流プロファイルを優先します");
    for at in [main_at.as_str(), sub_at.as_str(), "127.0.0.1:9"] {
        assert!(
            conflicts.iter().any(|line| line.contains(at)),
            "前提: ぶつかった行が {at} を名指していること:\n{log}"
        );
    }
    // 上流の画面も宛先を名指している（丸ごと消していない）。
    for at in [main_at.as_str(), sub_at.as_str()] {
        assert!(
            report_text.contains(at),
            "前提: 上流の画面が {at} を名指していること: {report_text}"
        );
    }

    // **本題**: どの行にも、上流の画面にも、どの秘密も無い。
    for secret in SECRETS {
        for line in &lines {
            assert!(
                !line.contains(secret),
                "ログに宛先の秘密が出ている（{secret}）: {line}"
            );
        }
        assert!(
            !report_text.contains(secret),
            "上流の画面に宛先の秘密が出ている（{secret}）: {report_text}"
        );
    }

    // **対照**: 書いたプロファイルの一覧は書いたままの値を返す（編集欄は書いた値を見せないと直せない）。
    let leaky = &profiles["profiles"][0];
    assert_eq!(
        (&leaky["main"]["endpoint"], &leaky["sub"]["endpoint"]),
        (
            &serde_json::json!(main_endpoint),
            &serde_json::json!(sub_endpoint)
        ),
        "{profiles}"
    );
}

/// **上流が載るまで待って、予算が動いた行を（あれば）返す。**
///
/// `explicit` は `PERSONACORE_CONTEXT_TOKENS` の値、`written` は本文の面に書く `context_tokens`、
/// `declared` は上流が名乗る窓。**返す行には、待ち受けの行より前の行も含む**（作業単位 A-13a-1）。
fn budget_line_after_load(
    explicit: Option<&str>,
    written: Option<u32>,
    declared: u32,
) -> (Option<String>, Vec<String>) {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let bin_dir = fake_llama_dir(dir.path());
    let gguf = fake_gguf(dir.path(), "planted-budget-model.gguf");
    let upstream_port = free_port();

    let mut main = serde_json::json!({
        "endpoint": format!("http://127.0.0.1:{upstream_port}"),
        "model": "planted-body-model-id",
        "gguf": gguf,
        // **窓を名乗り、少ししたら自分で終わる**（生きた偽物を残さない）。
        "args": [
            "--pc-ctx", declared.to_string(),
            "--pc-exit-after", FAKE_LIFETIME_SECONDS.to_string(),
        ],
    });
    if let Some(tokens) = written {
        main["context_tokens"] = serde_json::json!(tokens);
    }
    let profile = write_profile(
        dir.path(),
        &serde_json::json!({ "planted": { "main": main } }).to_string(),
    );

    let extra: Vec<(&str, &str)> = match explicit {
        Some(value) => vec![("PERSONACORE_CONTEXT_TOKENS", value)],
        None => Vec::new(),
    };
    let mut started = spawn_with(dir.path(), &bin_dir, Some(&profile), "0", "planted", &extra);
    let (port, early) = started.listening_port_keeping_lines();

    // **載るまで待つ**（ここまで来れば、採る側は必ず走っている）。
    wait_for_launch_state(&mut started, port, "main", "ready");

    // **溜まった行から探す。** `ready` の後なので、出るなら既に出ている。
    let lines: Vec<String> = early
        .into_iter()
        .chain(started.collect_lines())
        .map(|l| strip_ansi(&l))
        .collect();
    // **⚠ 値で絞る**（CLAUDE.md `B-1-2`「同じ行の中で見る」）——
    // **腕ごとに違う窓を名乗らせてある**ので、
    // **もう一方の腕の行も、抽出側の似た文面の行も、ここでは拾わない。**
    let field = format!("context_tokens={declared}");
    let found = lines
        .iter()
        .find(|line| line.contains(BUDGET_CHANGED) && has_field(line, &field))
        .cloned();
    (found, lines)
}

/// **面の状態が期待の綴りになるまで、上限つきで待つ。**
///
/// **毎周「書き手が生きているか」を見る**（CLAUDE.md `G-4`）。
fn wait_for_launch_state(started: &mut Started, port: u16, face: &str, expected: &str) {
    let mut last = serde_json::Value::Null;
    for _ in 0..MAX_WAITS {
        last = upstream_report(port)[face]["launch"]["state"].clone();
        if last == serde_json::json!(expected) {
            return;
        }
        if !started.running.is_alive() {
            panic!("{face} が {expected} になる前にプロセスが終了した（いま {last}）");
        }
        std::thread::sleep(WAIT_STEP);
    }
    panic!("{face} が {MAX_WAITS} 回待っても {expected} にならなかった（いま {last}）");
}

/// **空いているポートを1つ取る。**
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("ポートを取れること");
    let port = listener.local_addr().expect("宛先を読めること").port();
    drop(listener);
    port
}

// ----------------------------------------------------------------------
// 何も無い機で初めて叩く（作業単位 F-4c / Issue origin#23）
// ----------------------------------------------------------------------

/// **子の環境から、利用者の本物の置き場所へ届く道を全部断つ**（司令塔の決定5）。
///
/// | 変数 | 何に効くか |
/// |---|---|
/// | `LOCALAPPDATA` | Windows のデータディレクトリ（DB と `upstreams.json`） |
/// | `XDG_DATA_HOME` | Mac 以外の Unix のデータディレクトリ |
/// | `HOME` | Mac のデータディレクトリ／Windows 以外の `~` |
/// | `USERPROFILE` | **Windows の `~`**（見本の gguf は `~/.lmstudio/...` と書いてある） |
///
/// **⚠ `USERPROFILE` を断たないと、見本の gguf がこの機では実在する**
/// （司令塔の機に `~/.lmstudio/models/...` が在る）。**そのとき `--upstream qwen-official` は
/// `llama` を起こしに行く**——`PATH` の先頭は偽物だが、**断ち忘れた日に本物が GPU を掴む。**
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

/// **見本**（リポジトリに在るもの。**実行時に読む**——埋め込んだ側から期待値を作らない）。
fn committed_sample() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join("upstreams.sample.json"),
    )
    .expect("見本を読めること")
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

/// **何も無い機のまま起こす。** `PERSONACORE_DB_PATH` も `PERSONACORE_UPSTREAMS` も渡さない
/// ——**どちらも OS の置き場所から決まる**（初めて叩いた利用者と同じ形）。
///
/// `browser_log` が `Some` なら、**ブラウザの代わりに偽物を呼ばせる**。
fn spawn_first_run(root: &Path, args: &[&str], browser_log: Option<&Path>) -> Started {
    spawn_first_run_with(root, args, browser_log, &[])
}

/// **[`spawn_first_run`] と同じ。環境変数をもう何組か立てる**（作業単位 F-4b-3）。
///
/// **`PERSONACORE_UPSTREAMS` を立てた「何も無い機」を作るため**（読むだけの置き場所）。
fn spawn_first_run_with(
    root: &Path,
    args: &[&str],
    browser_log: Option<&Path>,
    extra: &[(&str, &Path)],
) -> Started {
    let mut command = without_inherited_settings();
    isolate_machine(&mut command, root);
    for (key, value) in extra {
        command.env(key, value);
    }
    command
        .args(args)
        .env("PERSONACORE_PORT", "0")
        // **上流には繋ぎに行かせない**（既定の宛先は LM Studio の :1234）。
        .env("PERSONACORE_LLM_ENDPOINT", DECOY_ENDPOINT)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(log) = browser_log {
        command
            .env(
                "PERSONACORE_TEST_BROWSER",
                env!("CARGO_BIN_EXE_fake_recorder"),
            )
            .env("FAKE_RECORDER_LOG", log);
    }

    let mut child = command.spawn().expect("実行ファイルを起こせること");
    let stdout = child.stdout.take().expect("標準出力を取れること");
    let (sender, lines) = mpsc::channel();
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

/// **その行が出た、かつ、その行が見本の置き場所を言っている**（CLAUDE.md `B-1-2`「同じ行の中で」）。
const SAMPLE_WRITTEN_LINE: &str = "上流プロファイルの見本を置きました";

/// **何も無い機で叩くと、DB と見本が出来て、画面が見本の場所を言い、ブラウザが1回開く**
/// （作業単位 F-4c / Issue origin#23 の閉じる条件1。**ブラウザは偽物**）。
///
/// # 1本で見ているもの
///
/// | 主張 | 破れると |
/// |---|---|
/// | 見本がコミットされた見本そのもので書かれる | 利用者が書き換える土台が違う |
/// | DB も同じ置き場所に出来る | 設定と DB の場所が割れる（ADR-0042 §決定1） |
/// | **起動ログの同じ行が、書いた場所を言う** | 利用者が見本を探せない |
/// | **`GET /admin/upstream` の `profile_sample` が、書いた場所を言う** | 画面が見本の場所を言えない |
/// | **ブラウザを開くコマンドが、実際に掴んだポートの URL で1回だけ呼ばれる** | 開かない／`:0` を開く／二重に開く |
#[test]
fn a_first_run_on_an_empty_machine_writes_the_sample_and_opens_the_screen() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let app_dir = app_dir_of(root);
    let profiles = app_dir.join("upstreams.json");
    let browser_log = root.join("browser-calls.jsonl");
    assert!(!app_dir.exists(), "前提: 置き場所がまだ無いこと");

    let mut started = spawn_first_run(root, &[], Some(&browser_log));
    let (port, early) = started.listening_port_keeping_lines();

    // **見本と DB**。
    assert_eq!(
        std::fs::read_to_string(&profiles).expect("見本が書かれていること"),
        committed_sample(),
        "見本がそのまま書かれていない"
    );
    assert!(
        app_dir.join("personacore.sqlite").is_file(),
        "DB が同じ置き場所に出来ていない"
    );

    // **画面の口**が、書いた場所を言う。
    let report = upstream_report(port);
    assert_eq!(
        report["profile_sample"],
        serde_json::json!(profiles.display().to_string()),
        "書いた見本の場所を言っていない: {report}"
    );

    // **ブラウザ**——実際に掴んだポートの URL で、1回だけ。
    let url = format!("http://127.0.0.1:{port}/");
    for _ in 0..MAX_WAITS {
        if !recorded_calls(&browser_log).is_empty() {
            break;
        }
        assert!(started.running.is_alive(), "ブラウザを開く前に終了した");
        std::thread::sleep(WAIT_STEP);
    }
    // **二度目を待つ**——1回目を見た直後に数えると、二重に開く形を見逃す。
    std::thread::sleep(Duration::from_millis(1_000));
    assert_eq!(
        recorded_calls(&browser_log),
        vec![expected_opener_args(&url)],
        "ブラウザを開くコマンドが、この URL で1回だけ呼ばれていない"
    );

    // **起動ログ**——**同じ行の中で**、書いた場所を言う。
    let lines: Vec<String> = early
        .into_iter()
        .chain(started.collect_lines())
        .map(|line| strip_ansi(&line))
        .collect();
    let field = format!("upstream_profiles_path={}", profiles.display());
    assert!(
        lines
            .iter()
            .any(|line| line.contains(SAMPLE_WRITTEN_LINE) && has_field(line, &field)),
        "見本を置いた場所を、起動ログの同じ行で言っていない\n{}",
        lines.join("\n")
    );
}

/// **既に在るプロファイルは1バイトも触らず、「書いた」とも言わない。`--no-browser` なら開かない**
/// （作業単位 F-4c / 上の腕の対比）。
///
/// **対比が無いと、「常に見本で上書きする」「常に書いたと言う」「旗を無視して開く」が
/// 上の腕で緑を通る**（CLAUDE.md「対比は安い側だけでは足りない」）。
/// **偽物は渡してある**——渡さなければ、`testing` 付きのビルドはそもそも開かない
/// （`src/server/browser.rs`）ので、`--no-browser` を見ていないのと同じになる。
#[test]
fn an_existing_profile_file_is_left_alone_and_no_browser_keeps_it_closed() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let app_dir = app_dir_of(root);
    std::fs::create_dir_all(&app_dir).expect("置き場所を作れること");
    let profiles = app_dir.join("upstreams.json");
    let mine =
        r#"{ "mine": { "main": { "endpoint": "http://127.0.0.1:1234", "model": "mine" } } }"#;
    std::fs::write(&profiles, mine).expect("置けること");
    let browser_log = root.join("browser-calls.jsonl");

    let mut started = spawn_first_run(root, &["--no-browser"], Some(&browser_log));
    let (port, early) = started.listening_port_keeping_lines();

    let report = upstream_report(port);
    assert_eq!(
        report["profile_sample"],
        serde_json::Value::Null,
        "書いていないのに、見本を置いたと言っている: {report}"
    );
    assert_eq!(
        std::fs::read_to_string(&profiles).expect("読めること"),
        mine,
        "利用者のプロファイルが書き換えられている"
    );

    // **開くなら、もう開いている時刻まで待ってから数える**（上の腕は 1 秒以内に開く）。
    std::thread::sleep(Duration::from_millis(1_500));
    assert!(started.running.is_alive(), "待ち受けが続いていること");
    assert_eq!(
        recorded_calls(&browser_log),
        Vec::<Vec<String>>::new(),
        "--no-browser なのにブラウザを開いている"
    );
    // **待ち受けより前の行も読む**——見本を置いた行はそこに出るので、
    // **後ろだけを読むと「言っていない」が空振りで成り立つ。**
    let lines: Vec<String> = early
        .into_iter()
        .chain(started.collect_lines())
        .map(|line| strip_ansi(&line))
        .collect();
    assert!(
        !lines.iter().any(|line| line.contains(SAMPLE_WRITTEN_LINE)),
        "書いていないのに、起動ログが見本を置いたと言っている\n{}",
        lines.join("\n")
    );
}

/// **`testing` 付きのビルドは、偽物を渡されなければ何も起こさない**
/// （作業単位 F-4c / レビュー1周目の指摘1 / r10）。
///
/// # なぜ要るのか
///
/// **`src/server/browser.rs` は「テストを書いた人が偽物を渡し忘れても、本物は開かない」と書くが、
/// 初版にはその見張り番が無かった** —— **環境変数が無いときに何かを起こす変異
/// （`.or_else(|| Some("…".into()))`）が 2197 全緑・exit 0 を通った。**
/// **偽物を渡した腕は「呼ばれた」しか見ず、`--no-browser` の腕は開く枝に入らない**ので、
/// **「渡し忘れ」の状態を作る腕が1本も無かった。**
///
/// # 何を見ているか（**同じ行の中で** / CLAUDE.md `B-1-2`）
///
/// * **「テスト用のビルドなので、ブラウザは開きません」の行が、この起動の URL を持って出る。**
/// * **起こそうとした行**（「開きます」／「開けませんでした」）**が1本も無い**
///   ——何かを起こしに行けば、成否どちらかの行が必ず出る。
///
/// # ⚠ 「渡し忘れ」の腕はこれ1本だけ、ではない
///
/// **ブラウザを主題にしない起動**（`tests/fresh_cli.rs` / このファイルの `spawn_with` 等）も、
/// **偽物も `--no-browser` も渡していない**のでこの枝を通る。**`--no-browser` は足さなかった**
/// （足すと、偽物を渡し忘れた起動が現に走る場所がここ1本に減る）。
/// **ただしそれらはログを読まない**ので、**この枝の振る舞いを見ている権威はこの1本だけである。**
#[test]
fn a_test_build_without_the_fake_opens_nothing() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();

    let mut started = spawn_first_run(root, &[], None);
    let (port, early) = started.listening_port_keeping_lines();
    let url_field = format!("url=\"http://127.0.0.1:{port}/\"");

    // **その行が出るまで、上限つきで読み進める**（出るのは待ち受けの直後）。
    let mut lines: Vec<String> = early.iter().map(|line| strip_ansi(line)).collect();
    for _ in 0..MAX_WAITS {
        if lines
            .iter()
            .any(|line| line.contains(TEST_BUILD_OPENS_NOTHING) && has_field(line, &url_field))
        {
            break;
        }
        assert!(started.running.is_alive(), "行を読む前に終了した");
        lines.extend(started.collect_lines().iter().map(|line| strip_ansi(line)));
    }
    // **起こしに行ったなら、その結果の行もここまでに出ている**——もう1周読んでから数える。
    std::thread::sleep(Duration::from_millis(1_000));
    lines.extend(started.collect_lines().iter().map(|line| strip_ansi(line)));

    assert!(
        lines
            .iter()
            .any(|line| line.contains(TEST_BUILD_OPENS_NOTHING) && has_field(line, &url_field)),
        "偽物を渡さない起動が「開きません」と、この URL の行で言っていない\n{}",
        lines.join("\n")
    );
    for tried in [
        "既定のブラウザで画面を開きます",
        "ブラウザを開けませんでした",
    ] {
        assert!(
            !lines.iter().any(|line| line.contains(tried)),
            "偽物を渡していないのに、何かを起こしに行っている（{tried}）\n{}",
            lines.join("\n")
        );
    }
}

/// **偽物を渡されなかった `testing` 付きのビルドが言う行**（`src/server/browser.rs` の `open`）。
///
/// **写しである。** 文面が変わったら、上の腕は「言っていない」で赤になる（空振りの向きは安全側）。
const TEST_BUILD_OPENS_NOTHING: &str = "テスト用のビルドなので、ブラウザは開きません";

/// **見本の gguf はこの機のもので、他の機には無い。見本を書いた直後の起動は何も起こさず、
/// `--upstream <見本の名前>` を付ければ、gguf のパスを言って止まる**
/// （作業単位 F-4c / Issue origin#23 の閉じる条件2）。
///
/// # どう組んでいるか
///
/// * **`~` の先（`USERPROFILE` / `HOME`）を空のディレクトリにする**——
///   **この機では見本の gguf が実在する**ので、断たないと「無い」を作れない。
/// * **`PATH` の先頭に、`llama` の名前で [`fake_recorder`] を置く**——
///   **呼ばれたら書き残してすぐ終わる**ので、呼ばれないことを数えられ、
///   **破る変異でも生きた偽物を残さない。**
///
/// [`fake_recorder`]: ../support/fake_recorder.rs
#[test]
fn the_sample_names_ggufs_this_machine_may_not_have_and_launches_nothing() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let app_dir = app_dir_of(root);
    let profiles = app_dir.join("upstreams.json");
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("空のホームを作れること");

    let bin_dir = root.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("置き場所を作れること");
    std::fs::copy(
        env!("CARGO_BIN_EXE_fake_recorder"),
        bin_dir.join(format!("llama{}", std::env::consts::EXE_SUFFIX)),
    )
    .expect("偽の llama を置けること");
    let llama_log = root.join("llama-calls.jsonl");

    let command_for = |args: &[&str]| {
        let mut command = without_inherited_settings();
        isolate_machine(&mut command, root);
        command
            .args(args)
            .env("PERSONACORE_PORT", "0")
            .env("PERSONACORE_LLM_ENDPOINT", DECOY_ENDPOINT)
            .env("PATH", prepended_path(&bin_dir))
            .env("FAKE_RECORDER_LOG", &llama_log)
            .stdin(Stdio::null());
        command
    };

    // **1回目: 見本を書く起動。何も起こさない。**
    {
        let mut first = command_for(&["--no-browser"]);
        first.stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = first.spawn().expect("実行ファイルを起こせること");
        let stdout = child.stdout.take().expect("標準出力を取れること");
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    return;
                }
            }
        });
        let mut started = Started {
            running: Running(child),
            lines,
        };
        let port = started.listening_port();
        assert_eq!(
            upstream_report(port)["profile_sample"],
            serde_json::json!(profiles.display().to_string()),
            "前提: この起動が見本を書いたこと"
        );
        for face in ["main", "sub"] {
            assert_eq!(
                upstream_report(port)[face]["launch"]["state"],
                serde_json::json!("external"),
                "見本を書いただけの起動が {face} を起こしている"
            );
        }
    }
    assert_eq!(
        std::fs::read_to_string(&profiles).expect("見本が書かれていること"),
        committed_sample()
    );

    // **2回目: 見本の名前を選ぶ。gguf が無いので、何も起こさずに止まる。**
    //
    // **見本の gguf を持つ名前を全部回す**——どれか1つだけ別の扱いになる形を見逃さない。
    // **期待するパスは直に組む**（`~` の展開規則を実装から写さない）。
    let expected: [(&str, PathBuf); 3] = [
        (
            "ministral-llama",
            home.join(".lmstudio")
                .join("models")
                .join("mistralai")
                .join("Ministral-3-14B-Instruct-2512-GGUF")
                .join("Ministral-3-14B-Instruct-2512-Q4_K_M.gguf"),
        ),
        (
            "qwen-official",
            home.join(".lmstudio")
                .join("models")
                .join("lmstudio-community")
                .join("Qwen3.6-35B-A3B-GGUF")
                .join("Qwen3.6-35B-A3B-Q6_K.gguf"),
        ),
        (
            "split",
            home.join(".lmstudio")
                .join("models")
                .join("lmstudio-community")
                .join("Qwen3.6-35B-A3B-GGUF")
                .join("Qwen3.6-35B-A3B-Q6_K.gguf"),
        ),
    ];
    for (name, gguf) in expected {
        let mut second = command_for(&["--no-browser", "--upstream", name]);
        second.stdout(Stdio::null()).stderr(Stdio::piped());
        let mut running = Running(second.spawn().expect("実行ファイルを起こせること"));
        let stderr = running.0.stderr.take().expect("標準エラーを取れること");
        let reader = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut text);
            text
        });
        let status = running.wait_for_exit("見本の gguf を選んだ起動");
        let stderr = reader.join().expect("標準エラーを読み切れること");

        assert!(
            !status.success(),
            "{name}: gguf が無いのに起動している（{status:?}）"
        );
        assert!(
            stderr.contains(&gguf.display().to_string()),
            "{name}: 止まった理由が gguf のパスを言っていない\n期待: {}\n{stderr}",
            gguf.display()
        );
    }

    assert_eq!(
        recorded_calls(&llama_log),
        Vec::<Vec<String>>::new(),
        "見本の gguf が無いのに llama を起こしている"
    );
    assert_eq!(
        std::fs::read_to_string(&profiles).expect("読めること"),
        committed_sample(),
        "2回目の起動が見本を書き換えている"
    );
}

/// **初めて叩いた起動で、いきなり `--upstream <見本の名前>` を付けても、書いた見本から選び、
/// gguf が無いので止まる**（作業単位 F-4c）。
///
/// # なぜ上の腕と別に要るのか
///
/// **上の腕は「見本を書く起動」と「選ぶ起動」を分けている**ので、
/// **`main` が見本を書くのを `--upstream` を読んだ後に回す変異**を観測できない
/// （その変異では、この起動は「プロファイルを読めませんでした」で止まる——**止まることは同じ**）。
/// **止まった理由が gguf のパスであること**まで見て、初めて順番が見える。
#[test]
fn the_very_first_run_can_already_name_a_sample_profile() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("空のホームを作れること");
    let bin_dir = root.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("置き場所を作れること");
    std::fs::copy(
        env!("CARGO_BIN_EXE_fake_recorder"),
        bin_dir.join(format!("llama{}", std::env::consts::EXE_SUFFIX)),
    )
    .expect("偽の llama を置けること");
    let llama_log = root.join("llama-calls.jsonl");

    let mut command = without_inherited_settings();
    isolate_machine(&mut command, root);
    command
        .args(["--no-browser", "--upstream", "qwen-official"])
        .env("PERSONACORE_PORT", "0")
        .env("PERSONACORE_LLM_ENDPOINT", DECOY_ENDPOINT)
        .env("PATH", prepended_path(&bin_dir))
        .env("FAKE_RECORDER_LOG", &llama_log)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut running = Running(command.spawn().expect("実行ファイルを起こせること"));
    let stderr = running.0.stderr.take().expect("標準エラーを取れること");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut text);
        text
    });
    let status = running.wait_for_exit("初めての起動で見本の名前を選んだ起動");
    let stderr = reader.join().expect("標準エラーを読み切れること");

    let gguf = home
        .join(".lmstudio")
        .join("models")
        .join("lmstudio-community")
        .join("Qwen3.6-35B-A3B-GGUF")
        .join("Qwen3.6-35B-A3B-Q6_K.gguf");
    assert!(
        !status.success(),
        "gguf が無いのに起動している（{status:?}）"
    );
    assert!(
        stderr.contains(&gguf.display().to_string()),
        "止まった理由が gguf のパスではない（見本を選ぶより前に止まっている？）\n{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(app_dir_of(root).join("upstreams.json"))
            .expect("見本が書かれていること"),
        committed_sample()
    );
    assert_eq!(
        recorded_calls(&llama_log),
        Vec::<Vec<String>>::new(),
        "見本の gguf が無いのに llama を起こしている"
    );
}

/// **`--version` は版を言って、何もせずに終わる**（作業単位 F-4c / Issue origin#23）。
///
/// # 何を見ているか
///
/// * **1行だけ、`personacore <版> (<コミット>)` の形で言う。**
/// * **括弧の中が、いまのコミットを指している**——`git rev-parse` で引き直して `HEAD` と突き合わせる
///   （**期待値を `build.rs` と同じ計算で作らない**。タグの有無で綴りが変わっても、指す先は1つ）。
///   **リポジトリの根で組んでいない（`git` が無い等）なら `unknown`。**
/// * **他の旗が並んでいても何もしない**——`--fresh` を並べても DB を退避しない、見本も書かない。
#[test]
fn the_version_flag_names_the_commit_and_does_nothing_else() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let db_dir = root.join("db");
    std::fs::create_dir_all(&db_dir).expect("置き場所を作れること");
    let database = db_dir.join("personacore.sqlite");
    std::fs::write(&database, "DB-BEFORE-VERSION").expect("置けること");

    let mut command = without_inherited_settings();
    isolate_machine(&mut command, root);
    command
        .args(["--version", "--fresh"])
        .env("PERSONACORE_DB_PATH", &database)
        .env("PERSONACORE_PORT", "0")
        .env("PERSONACORE_LLM_ENDPOINT", DECOY_ENDPOINT)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // **`.output()` を使わない**——上限なしで待つ（`tests/fresh_cli.rs` の doc）。
    // **旗を無視して待ち受けを開く変異で、テストが返らなくなる。** 待つ側に上限を置く。
    let mut running = Running(command.spawn().expect("実行ファイルを起こせること"));
    let pipe = running.0.stdout.take().expect("標準出力を取れること");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(pipe), &mut text);
        text
    });
    let status = running.wait_for_exit("--version の起動");
    let stdout = reader.join().expect("標準出力を読み切れること");

    assert!(status.success(), "{status:?}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "1行だけ言うこと: {stdout:?}");

    let prefix = format!("personacore {} (", env!("CARGO_PKG_VERSION"));
    let described = lines[0]
        .strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or_else(|| panic!("形が違う: {:?}", lines[0]));

    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    let at_repository_root = git(&["rev-parse", "--show-toplevel"])
        .and_then(|top| std::fs::canonicalize(top).ok())
        .is_some_and(|top| {
            std::fs::canonicalize(env!("CARGO_MANIFEST_DIR")).is_ok_and(|here| here == top)
        });
    if at_repository_root {
        let head = git(&["rev-parse", "HEAD"]).expect("HEAD を引けること");
        let named = git(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{described}^{{commit}}"),
        ]);
        assert_eq!(
            named.as_deref(),
            Some(head.as_str()),
            "--version の綴り {described:?} が、いまのコミットを指していない"
        );
    } else {
        assert_eq!(described, "unknown");
    }

    // **何もしていない。**
    assert_eq!(
        std::fs::read_to_string(&database).expect("DB を読めること"),
        "DB-BEFORE-VERSION",
        "--version なのに DB を触っている"
    );
    let entries: Vec<_> = std::fs::read_dir(&db_dir)
        .expect("読めること")
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "--version なのに退避している: {entries:?}"
    );
    assert!(
        !app_dir_of(root).exists(),
        "--version なのに置き場所を作っている（見本を書いた）"
    );
}

// ----------------------------------------------------------------------
// 画面から上流プロファイルを直す（ADR-0042 §決定8 / 作業単位 F-4b-3 / Issue origin#24）
// ----------------------------------------------------------------------

/// **管理 API を1回叩く**（作業単位 F-4b-3）。
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
            // **上限は待つ側に**（止まった実行ファイルを永久に待たない）。長さの根拠は
            // `tests/upstream_profile_editor.rs` の `REQUEST_TIMEOUT`（負荷の下で 55〜60 秒止まる回がある）。
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

/// **置き場所のディレクトリに一時ファイルが残っていないか**（作業単位 F-4b-3）。
fn temporaries_in(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .expect("読めること")
        .map(|entry| {
            entry
                .expect("読めること")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".tmp"))
        .collect()
}

/// **2つの置き場所に、互いに違う中身を置く**（同値だと、どちらを読んだかが観測できない）。
const DEFAULT_PLACE_PROFILES: &str = r#"{ "at-default": { "main": { "endpoint": "http://127.0.0.1:47501", "model": "default-model" } } }"#;
const OVERRIDE_PLACE_PROFILES: &str = r#"{ "at-override": { "main": { "endpoint": "http://127.0.0.1:47502", "model": "override-model" } } }"#;

/// **`PERSONACORE_UPSTREAMS` が別の場所を指すときは、画面からは書けない。
/// 指した先のファイルも既定の置き場所も、1バイトも変わらない**（ADR-0042 §決定8 / 作業単位 F-4b-3）。
///
/// # なぜ実行ファイルを起こすのか
///
/// **「既定なら書ける／`PERSONACORE_UPSTREAMS` なら読むだけ」を決める1行は `src/main.rs` にしか無い**
/// （bin なので統合テストから `use` できない）。**`Startup` を直に組む `tests/upstream_profile_editor.rs`
/// からは、その1行を取り違える変異が観測できない。**
///
/// **一覧は指した先を読んでいる**ことまで見る（既定の置き場所の名前が出ていないこと）。
#[test]
fn an_overridden_profile_file_is_read_only_and_neither_file_changes() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let app_dir = app_dir_of(root);
    std::fs::create_dir_all(&app_dir).expect("置き場所を作れること");
    let default_file = app_dir.join("upstreams.json");
    std::fs::write(&default_file, DEFAULT_PLACE_PROFILES).expect("置けること");
    let override_dir = root.join("repo").join("scripts");
    std::fs::create_dir_all(&override_dir).expect("作れること");
    let override_file = override_dir.join("upstreams.json");
    std::fs::write(&override_file, OVERRIDE_PLACE_PROFILES).expect("置けること");

    let mut started = spawn_first_run_with(
        root,
        &["--no-browser"],
        None,
        &[("PERSONACORE_UPSTREAMS", &override_file)],
    );
    let port = started.listening_port();

    let (status, listing) = call(port, reqwest::Method::GET, "/admin/upstream/profiles", None);
    assert_eq!(status, 200, "{listing}");
    assert_eq!(listing["editable"], serde_json::json!(false), "{listing}");
    assert_eq!(
        listing["path"],
        serde_json::json!(override_file.display().to_string()),
        "{listing}"
    );
    assert_eq!(listing["profiles"][0]["name"], "at-override", "{listing}");
    assert!(
        !listing.to_string().contains("at-default"),
        "既定の置き場所を読んでいる: {listing}"
    );

    let (status, response) = call(
        port,
        reqwest::Method::PUT,
        "/admin/upstream/profiles/at-override",
        Some(serde_json::json!({
            "revision": listing["revision"],
            "main": { "endpoint": "http://127.0.0.1:47503", "model": "rewritten" }
        })),
    );
    assert_eq!(status, 409, "{response}");
    assert_eq!(response["error"]["code"], "upstream_profiles_read_only");

    assert_eq!(
        std::fs::read_to_string(&override_file).expect("読めること"),
        OVERRIDE_PLACE_PROFILES,
        "PERSONACORE_UPSTREAMS が指した先を書き換えている"
    );
    assert_eq!(
        std::fs::read_to_string(&default_file).expect("読めること"),
        DEFAULT_PLACE_PROFILES,
        "既定の置き場所を書き換えている"
    );
    assert_eq!(temporaries_in(&override_dir), Vec::<String>::new());
    assert_eq!(temporaries_in(&app_dir), Vec::<String>::new());
    assert!(started.running.is_alive(), "待ち受けが続いていること");
}

/// **既定の置き場所なら、実行ファイルそのものから書ける**（上の腕の対比 / 本番の入口を通る保存）。
///
/// **対比が無いと、「常に読むだけにする」実装が上の腕で緑を通る。**
#[test]
fn the_default_profile_file_is_editable_through_the_executable() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let app_dir = app_dir_of(root);
    std::fs::create_dir_all(&app_dir).expect("置き場所を作れること");
    let default_file = app_dir.join("upstreams.json");
    std::fs::write(&default_file, DEFAULT_PLACE_PROFILES).expect("置けること");

    let mut started = spawn_first_run(root, &["--no-browser"], None);
    let port = started.listening_port();

    let (status, listing) = call(port, reqwest::Method::GET, "/admin/upstream/profiles", None);
    assert_eq!(status, 200, "{listing}");
    assert_eq!(listing["editable"], serde_json::json!(true), "{listing}");
    assert_eq!(
        listing["path"],
        serde_json::json!(default_file.display().to_string()),
        "{listing}"
    );

    let main = serde_json::json!({
        "endpoint": "http://127.0.0.1:47504", "model": "added-model",
        "gguf": "~/models/added.gguf", "args": null, "command": null
    });
    let (status, saved) = call(
        port,
        reqwest::Method::PUT,
        "/admin/upstream/profiles/added",
        Some(serde_json::json!({ "revision": listing["revision"], "main": main })),
    );
    assert_eq!(status, 200, "{saved}");

    let on_disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&default_file).expect("読めること"))
            .expect("JSON");
    assert_eq!(
        on_disk["added"],
        serde_json::json!({ "main": {
            "endpoint": "http://127.0.0.1:47504", "model": "added-model", "gguf": "~/models/added.gguf"
        } }),
        "既定の置き場所に書かれていない"
    );
    let before: serde_json::Value = serde_json::from_str(DEFAULT_PLACE_PROFILES).expect("JSON");
    assert_eq!(
        on_disk["at-default"], before["at-default"],
        "他のプロファイルが変わっている"
    );
    assert_eq!(temporaries_in(&app_dir), Vec::<String>::new());
    assert!(started.running.is_alive(), "待ち受けが続いていること");
}

// ----------------------------------------------------------------------
// 最後に選んだプロファイルで起きる（ADR-0042 §決定9 / 作業単位 F-4f / Issue origin#28）
// ----------------------------------------------------------------------
//
// **どの腕も実行ファイルを起こす。** 「覚える」「読む」「どちらが勝つか」を決める行は
// `src/main.rs`（bin）と、`main` が組む `Startup` の中にしか無い。
//
// **プロファイルは4つ、値は全部違える**（宛先・モデルの綴り）——同値だと、どれで起きたかが観測できない。
// **選ぶのは端でない `beta`**（名前の順で2番目 / CLAUDE.md「選ぶ操作は真ん中を選ぶ」）。
//
// | 名前 | 面 | 何のため |
// |---|---|---|
// | `alpha` | 外の上流 | 端の囮 |
// | `beta` | **gguf（偽の `llama`。印を付けて即終了）** | **選ぶ対象**。起こしたことを印で見る |
// | `delta` | gguf（**実在しない**） | 覚えた名前が使えない腕 |
// | `gamma` | 外の上流 | 端の囮／覚えた名前の初期値 |
//
// **偽の `llama` は即終了する**（`--pc-exit 0`）——生きた偽物を残さない（このファイルの約束）。
//
// **この節が見ていないもの**:
//
// * **2つの personacore が同時に覚える形。**
// * **覚えた名前のプロファイルで、2つ目の子を起こすところで失敗する形**（1つ目の子は `kill_on_drop` が
//   殺すはずだが、2つの子を起こすプロファイルでその経路を通す腕は無い）。

/// **覚えた名前を置くファイル**（ADR-0042 §決定9）。**期待値を実装から作らない**——綴りを直に書く。
fn memory_file_of(root: &Path) -> PathBuf {
    app_dir_of(root).join("selected-upstream.txt")
}

/// **覚えた名前が使えないときの警告の本文**（`src/main.rs`）。**写しである。**
///
/// 文面が変わったら、その腕は「言っていない」で赤になる（空振りの向きは安全側）。
const REMEMBERED_UNUSABLE_LINE: &str =
    "覚えていた上流プロファイルを使えないので、環境変数の既定で起動します";

/// **覚えた名前で起き、gguf の面を起こさずに待つと言う行の本文**（`src/main.rs` の `WAITING_TO_LAUNCH`）。**写しである。**
///
/// 文面が変わったら、その腕は「言っていない」で赤になる（空振りの向きは安全側）。
///
/// **画面のボタンでも起こせることまで写す**（作業単位 F-10b）—— 起動ログだけを読む利用者に、
/// 端末の外の起こし方を言う部分が落ちたら赤にする。
const WAITING_TO_LAUNCH_LINE: &str = "覚えていた上流プロファイルで起動しました。gguf の面はまだ起こしていません（画面の「起こす」ボタンか、同じ名前の POST /admin/upstream/profile で起こします";

/// **4つのプロファイルの宛先とモデル**（`(名前, 宛先, モデル)`）。**全部違える。**
const REMEMBERED_FACES: [(&str, &str, &str); 4] = [
    ("alpha", "http://127.0.0.1:47611", "alpha-model"),
    ("beta", "http://127.0.0.1:47612", "beta-model"),
    ("delta", "http://127.0.0.1:47614", "delta-model"),
    ("gamma", "http://127.0.0.1:47613", "gamma-model"),
];

/// 名前から宛先とモデルを引く（テストの表から。**実装からは作らない**）。
fn face_of(name: &str) -> (&'static str, &'static str) {
    REMEMBERED_FACES
        .iter()
        .find(|(candidate, _, _)| *candidate == name)
        .map(|(_, endpoint, model)| (*endpoint, *model))
        .unwrap_or_else(|| panic!("表に無い名前: {name}"))
}

/// **既定の置き場所に4つのプロファイルを書き、偽の `llama` を置く。**
///
/// 戻り値は `(偽の llama のディレクトリ, 起こされた印)`。
fn plant_remembered_profiles(root: &Path) -> (PathBuf, PathBuf) {
    let app_dir = app_dir_of(root);
    std::fs::create_dir_all(&app_dir).expect("置き場所を作れること");
    let bin_dir = fake_llama_dir(root);
    let gguf = fake_gguf(root, "planted-beta.gguf");
    let marker = root.join(TOUCH_MARKER);
    let mut body = serde_json::Map::new();
    for (name, endpoint, model) in REMEMBERED_FACES {
        let face = match name {
            "beta" => serde_json::json!({
                "endpoint": endpoint, "model": model, "gguf": gguf,
                "args": ["--pc-touch", marker, "--pc-exit", "0"],
            }),
            "delta" => serde_json::json!({
                "endpoint": endpoint, "model": model, "gguf": root.join("never-here.gguf"),
                "args": ["--pc-touch", marker, "--pc-exit", "0"],
            }),
            _ => serde_json::json!({ "endpoint": endpoint, "model": model }),
        };
        body.insert(name.to_owned(), serde_json::json!({ "main": face }));
    }
    std::fs::write(
        app_dir.join("upstreams.json"),
        serde_json::Value::Object(body).to_string(),
    )
    .expect("プロファイルを置けること");
    (bin_dir, marker)
}

/// **何も無い機の形で起こす**（DB も既定の置き場所）。`PATH` の先頭は偽の `llama`。
///
/// **`--no-browser` を必ず足す**（作業者・レビュアーはブラウザを起こさない）。
/// **`extra` は既定の後に立てる**（`PERSONACORE_PORT` を上書きする腕がある）。
fn spawn_on_machine(
    root: &Path,
    bin_dir: &Path,
    args: &[&str],
    extra: &[(&str, &std::ffi::OsStr)],
) -> Started {
    let mut command = without_inherited_settings();
    isolate_machine(&mut command, root);
    command
        .arg("--no-browser")
        .args(args)
        .env("PERSONACORE_PORT", "0")
        // **既定の上流は囮**（覚えた名前が勝ったかを、これと違うことで見る）。
        .env("PERSONACORE_LLM_ENDPOINT", DECOY_ENDPOINT)
        .env("PATH", prepended_path(bin_dir))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in extra {
        command.env(key, value);
    }

    let mut child = command.spawn().expect("実行ファイルを起こせること");
    let stdout = child.stdout.take().expect("標準出力を取れること");
    let (sender, lines) = mpsc::channel();
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

/// **本文の面が、その名前のプロファイルを向いているか**（抽出の面も同じ上流 / `sub` を書いていない）。
fn assert_faces_on(report: &serde_json::Value, name: &str) {
    let (endpoint, model) = face_of(name);
    for face in ["main", "sub"] {
        assert_eq!(
            report[face]["endpoint"],
            serde_json::json!(format!("{endpoint}/v1/chat/completions")),
            "{face} が {name} を向いていない: {report}"
        );
        assert_eq!(
            report[face]["model"],
            serde_json::json!(model),
            "{face} が {name} のモデルを依頼していない: {report}"
        );
    }
    // **いま選ばれているプロファイルの名前も言う**（作業単位 F-10b）——画面の「起こす」は
    // この名前を `POST /admin/upstream/profile` へ送る。**`main` の配線（旗・覚えた名前）はここで見る。**
    assert_eq!(
        report["selected_profile"],
        serde_json::json!(name),
        "いま選ばれているプロファイルとして {name} を言っていない: {report}"
    );
}

/// **本文の面が環境変数の既定（囮）を向いているか。**
///
/// **プロファイルを選んでいないので、`selected_profile` は `null`**（作業単位 F-10b）。
fn assert_faces_on_the_environment(report: &serde_json::Value) {
    assert_eq!(
        report["main"]["endpoint"],
        serde_json::json!(format!("{DECOY_ENDPOINT}/v1/chat/completions")),
        "環境変数の既定を向いていない: {report}"
    );
    assert_eq!(
        report["selected_profile"],
        serde_json::Value::Null,
        "プロファイルを選んでいない起動が、選んだ名前を言っている: {report}"
    );
}

/// **印が付くまで、上限つきで待つ**（毎周、書き手の生存を見る）。
fn wait_for_marker(started: &mut Started, marker: &Path) {
    for _ in 0..MAX_WAITS {
        if marker.exists() {
            return;
        }
        assert!(started.running.is_alive(), "印が付く前に終了した");
        std::thread::sleep(WAIT_STEP);
    }
    panic!("偽の llama が {MAX_WAITS} 回待っても呼ばれなかった");
}

/// **覚えた名前で起きた起動は、gguf の面を起こさずに `not_started` で待ち、
/// 同じ名前を頼まれて初めて起こす**（ADR-0055 §決定1・3 / 作業単位 F-10a / Issue origin#82）。
///
/// * **状態は待ち受けより前に決まる**（`bind` の中）ので、**1回目の読みで `not_started` でなければ赤**
///   ——起動時に起こす実装は `loading` か（偽物が即終了したあとの）`failed` を言う。
/// * **印が無いことを見た後に、同じ名前を頼んで印が付くことまで見る**——
///   「起きなかった」が、印の検査の空振り（偽物が `PATH` に無い等）で成立する形にしない。
///   **「起きる」側の対照は同じ節の `--upstream` の起動**（待たずに印が付く）。
/// * **起動ログの同じ行が、待っている面とプロファイルを言う**（[`WAITING_TO_LAUNCH_LINE`] / F-10a のレビュー是正）。
///   **その行は待ち受けより前に出る**ので、`early`（[`Started::listening_port_keeping_lines`] の行）から探す。
///   見本のプロファイルは `sub` を書いていないので、待っているのは2面（`main+sub`）。
fn assert_waits_then_starts_on_the_same_name(
    started: &mut Started,
    port: u16,
    early: &[String],
    marker: &Path,
    name: &str,
) {
    let name_field = format!("upstream_profile=\"{name}\"");
    assert!(
        early.iter().map(|line| strip_ansi(line)).any(|line| {
            line.contains(WAITING_TO_LAUNCH_LINE)
                && has_field(&line, &name_field)
                && has_field(&line, "upstream_faces=main+sub")
        }),
        "起動ログの同じ行が、待っている面（main+sub）と {name_field} を言っていない\n{}",
        early.join("\n")
    );
    let report = upstream_report(port);
    assert_eq!(
        report["main"]["launch"]["state"],
        serde_json::json!("not_started"),
        "覚えた名前の起動が、gguf の面を待たせていない: {report}"
    );
    assert!(
        !marker.exists(),
        "覚えた名前の起動が偽の llama を呼んでいる"
    );
    let (status, body) = call(
        port,
        reqwest::Method::POST,
        "/admin/upstream/profile",
        Some(serde_json::json!({ "name": name })),
    );
    assert_eq!(status, 200, "同じ名前で起こせること: {body}");
    wait_for_marker(started, marker);
}

/// **起動ログ（待ち受けより前を含む）を、色づけを落として全部読む。**
fn all_lines(started: &mut Started, early: Vec<String>) -> Vec<String> {
    early
        .into_iter()
        .chain(started.collect_lines())
        .map(|line| strip_ansi(&line))
        .collect()
}

/// **起動ログの同じ行が、どのプロファイルで・何から決まったかを言う**（`設定を読み込みました` の行）。
fn assert_started_with(lines: &[String], name: &str, from: &str) {
    let name_field = format!("upstream_profile=\"{name}\"");
    let from_field = format!("upstream_profile_from={from}");
    assert!(
        lines
            .iter()
            .any(|line| line.contains("設定を読み込みました")
                && has_field(line, &name_field)
                && has_field(line, &from_field)),
        "起動ログの同じ行が {name_field} / {from_field} を言っていない\n{}",
        lines.join("\n")
    );
}

/// **画面で切り替えたら覚え、次に引数なしで起こすと、覚えた名前で起きる**（ADR-0042 §決定9）。
///
/// **2回目の起動は gguf の面を起こさずに待ち、同じ名前を頼むと偽の `llama` を起こす**
/// （ADR-0055 / 作業単位 F-10a。1回目の印は消してから起こす）
/// ——綴りだけが覚えた名前へ向き、起こす計画が付いてこない形を見逃さない。
#[test]
fn a_switched_profile_is_where_the_next_bare_start_begins() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (bin_dir, marker) = plant_remembered_profiles(root);
    let memory = memory_file_of(root);

    {
        let mut first = spawn_on_machine(root, &bin_dir, &[], &[]);
        let port = first.listening_port();
        // **前提: 何も覚えていない起動は、環境変数の既定を向く。**
        assert_faces_on_the_environment(&upstream_report(port));
        assert!(!memory.exists(), "前提: まだ何も覚えていないこと");

        let (status, body) = call(
            port,
            reqwest::Method::POST,
            "/admin/upstream/profile",
            Some(serde_json::json!({ "name": "beta" })),
        );
        assert_eq!(status, 200, "{body}");
        // **受け付けた切り替えの名前を、応答も次の読みも言う**（作業単位 F-10b）。
        assert_eq!(
            body["selected_profile"],
            serde_json::json!("beta"),
            "切り替えの応答が、選んだ名前を言っていない: {body}"
        );
        assert_faces_on(&upstream_report(port), "beta");
        assert_eq!(
            std::fs::read_to_string(&memory).expect("覚えたファイルが在ること"),
            "beta\n",
            "切り替えた名前を1行で覚えていない"
        );
        assert_eq!(temporaries_in(&app_dir_of(root)), Vec::<String>::new());
        wait_for_marker(&mut first, &marker);
    }
    std::fs::remove_file(&marker).expect("1回目の印を消せること");

    let mut second = spawn_on_machine(root, &bin_dir, &[], &[]);
    let (port, early) = second.listening_port_keeping_lines();
    assert_faces_on(&upstream_report(port), "beta");
    assert_waits_then_starts_on_the_same_name(&mut second, port, &early, &marker, "beta");
    let lines = all_lines(&mut second, early);
    assert_started_with(&lines, "beta", "remembered");
    assert_eq!(
        std::fs::read_to_string(&memory).expect("読めること"),
        "beta\n",
        "覚えた名前が変わっている"
    );
}

/// **`--upstream` で選んで起こしたら覚え、次に引数なしで起こすと、それで起きる**（ADR-0042 §決定9）。
///
/// **「起きる」と「起きない」の対**（ADR-0055 §決定1・2 / 作業単位 F-10a）——
/// **同じプロファイル・同じ偽の `llama` で、`--upstream` の起動は待たずに起こし、
/// 覚えた名前の起動は `not_started` で待つ**（同じ名前を頼むと起こす）。
/// **起きる側が成立しないと、起きない側は印の検査の空振りでも緑になる。**
///
/// **起こせなかった `--upstream`（gguf が無い）は覚えない**——止まった起動の名前を覚えると、
/// 次の引数なしの起動が「使えない」と言うだけになる（作業者の判断。報告に書く）。
#[test]
fn an_upstream_flag_is_remembered_for_the_next_bare_start() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (bin_dir, marker) = plant_remembered_profiles(root);
    let memory = memory_file_of(root);

    {
        let mut first = spawn_on_machine(root, &bin_dir, &["--upstream", "beta"], &[]);
        let (port, early) = first.listening_port_keeping_lines();
        assert_faces_on(&upstream_report(port), "beta");
        let lines = all_lines(&mut first, early);
        assert_started_with(&lines, "beta", "flag");
        // **対照**: 起こす起動は「待つ」と言わない（言い続ける実装を赤にする）。
        assert!(
            !lines
                .iter()
                .any(|line| line.contains(WAITING_TO_LAUNCH_LINE)),
            "--upstream で起こす起動が、起こさずに待つと言っている\n{}",
            lines.join("\n")
        );
        assert_eq!(
            std::fs::read_to_string(&memory).expect("覚えたファイルが在ること"),
            "beta\n",
            "--upstream で選んだ名前を覚えていない"
        );
        wait_for_marker(&mut first, &marker);
    }
    std::fs::remove_file(&marker).expect("1回目の印を消せること");

    {
        let mut second = spawn_on_machine(root, &bin_dir, &[], &[]);
        let (port, early) = second.listening_port_keeping_lines();
        assert_faces_on(&upstream_report(port), "beta");
        assert_waits_then_starts_on_the_same_name(&mut second, port, &early, &marker, "beta");
    }

    // **起こせなかった `--upstream` は覚えない。止まるのは今までどおり。**
    let mut failed = spawn_on_machine(root, &bin_dir, &["--upstream", "delta"], &[]);
    let status = failed.running.wait_for_exit("gguf が無い --upstream");
    assert!(
        !status.success(),
        "gguf が無いのに起動している（{status:?}）"
    );
    assert_eq!(
        std::fs::read_to_string(&memory).expect("読めること"),
        "beta\n",
        "起こせなかった名前を覚えている"
    );
}

/// **覚えた名前があっても、`--upstream` が勝つ。勝った名前を覚え直す**（ADR-0042 §決定9 の優先順位）。
#[test]
fn an_upstream_flag_wins_over_the_remembered_name() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (bin_dir, marker) = plant_remembered_profiles(root);
    let memory = memory_file_of(root);
    std::fs::write(&memory, "gamma\n").expect("覚えた名前を置けること");

    let mut started = spawn_on_machine(root, &bin_dir, &["--upstream", "beta"], &[]);
    let (port, early) = started.listening_port_keeping_lines();
    assert_faces_on(&upstream_report(port), "beta");
    wait_for_marker(&mut started, &marker);
    let lines = all_lines(&mut started, early);
    assert_started_with(&lines, "beta", "flag");
    assert_eq!(
        std::fs::read_to_string(&memory).expect("読めること"),
        "beta\n",
        "--upstream で選んだ名前を覚え直していない"
    );
}

/// **覚えた名前が使えなければ、止まらずに環境変数の既定で起き、警告の同じ行が名前を言う。
/// 覚えた名前は消さない**（ADR-0042 §決定9）。
///
/// | 腕 | 覚えた名前 | なぜ使えない |
/// |---|---|---|
/// | 消えた | `vanished` | `upstreams.json` に無い |
/// | gguf | `delta` | gguf が無い（**偽の `llama` は1度も呼ばれない**） |
/// | 検証 | `beta` | `upstreams.json` が検証に通らない（知らない鍵） |
/// | 取り合い | `beta` | **起こす面の宛先が personacore 自身の待ち受けと同じポート**（`PERSONACORE_PORT`） |
/// | 設定 | `beta` | `upstreams.json` の検証は通るが、設定が断る（モデルの綴りが空） |
///
/// **取り合いの腕は `PERSONACORE_PORT` を 0 以外にする唯一の腕**（他の腕は 0 なので、取り合いの門は効かない）。
///
/// **⚠ F-10a まであった「起こせない」の腕（`command` が存在しない絶対パス）は、ここから外した。**
/// 覚えた名前の起動は gguf の面を起こさない（ADR-0055 §決定1）ので、**起こして初めて分かる失敗は
/// 起動の時点では起きず、既定へ落ちる理由にならない。** その形は
/// `a_remembered_profile_whose_command_is_missing_waits_and_fails_only_when_asked` が見る。
#[test]
fn an_unusable_remembered_name_falls_back_to_the_environment_and_is_kept() {
    for (arm, remembered) in [
        ("vanished", "vanished"),
        ("gguf", "delta"),
        ("invalid", "beta"),
        ("collision", "beta"),
        ("config", "beta"),
    ] {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let root = dir.path();
        let (bin_dir, marker) = plant_remembered_profiles(root);
        let mut port_setting = None;
        if arm == "config" {
            // **`upstreams.json` の検証は通るが、設定（`AppConfig`）が断る**——モデルの綴りが空。
            std::fs::write(
                app_dir_of(root).join("upstreams.json"),
                r#"{ "beta": { "main": { "endpoint": "http://127.0.0.1:47612", "model": "" } } }"#,
            )
            .expect("設定の組めないプロファイルを置けること");
        }
        if arm == "collision" {
            // **gguf は在る**（取り合いだけが使えない理由になる）。
            let port = free_port();
            std::fs::write(
                app_dir_of(root).join("upstreams.json"),
                serde_json::json!({ "beta": { "main": {
                    "endpoint": format!("http://127.0.0.1:{port}"), "model": "beta-model",
                    "gguf": root.join("planted-beta.gguf"),
                    "args": ["--pc-touch", marker, "--pc-exit", "0"],
                } } })
                .to_string(),
            )
            .expect("取り合うプロファイルを置けること");
            port_setting = Some(port.to_string());
        }
        if arm == "invalid" {
            std::fs::write(
                app_dir_of(root).join("upstreams.json"),
                r#"{ "beta": { "main": { "endpoint": "http://127.0.0.1:47612", "model": "beta-model", "ggml": "x" } } }"#,
            )
            .expect("壊れたプロファイルを置けること");
        }
        let memory = memory_file_of(root);
        let before = format!("{remembered}\n");
        std::fs::write(&memory, &before).expect("覚えた名前を置けること");

        let extra: Vec<(&str, &std::ffi::OsStr)> = port_setting
            .as_deref()
            .map(|port| ("PERSONACORE_PORT", std::ffi::OsStr::new(port)))
            .into_iter()
            .collect();
        let mut started = spawn_on_machine(root, &bin_dir, &[], &extra);
        let (port, early) = started.listening_port_keeping_lines();
        assert_faces_on_the_environment(&upstream_report(port));
        let lines = all_lines(&mut started, early);
        let name_field = format!("upstream_profile=\"{remembered}\"");
        assert!(
            lines.iter().any(
                |line| line.contains(REMEMBERED_UNUSABLE_LINE) && has_field(line, &name_field)
            ),
            "{arm}: 使えない覚えた名前を、警告の同じ行で言っていない\n{}",
            lines.join("\n")
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("設定を読み込みました")
                    && has_field(line, "upstream_profile=\"-\"")
                    && has_field(line, "upstream_profile_from=-")),
            "{arm}: 既定で起きたと、起動ログが言っていない\n{}",
            lines.join("\n")
        );
        assert!(started.running.is_alive(), "{arm}: 止まっている");
        assert_eq!(
            std::fs::read(&memory).expect("読めること"),
            before.as_bytes(),
            "{arm}: 覚えた名前を書き換えている"
        );
        assert!(!marker.exists(), "{arm}: 使えないはずの上流を起こしている");
    }
}

/// **覚えた名前のプロファイルの `command` が存在しなくても、起動は止まらず既定にも落ちない。
/// その面は `not_started` で待ち、起こせないことは同じ名前を頼んだときに `failed` で分かる。
/// もう一度同じ名前を頼むと、もう一度起こしに行く**（ADR-0055 §決定1・3 / 作業単位 F-10a）。
///
/// **F-10a より前は、起動の中で起こして失敗し、環境変数の既定で起き直していた**（`main` の
/// `ServerError::Launch` の枝。覚えた名前の起動が起こさなくなり、その枝へ来る道が無くなったので消した）。
/// `command` を絶対パスにしてあるので、偽の `llama` を置いた `PATH` の影響を受けない。
#[test]
fn a_remembered_profile_whose_command_is_missing_waits_and_fails_only_when_asked() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (bin_dir, marker) = plant_remembered_profiles(root);
    let missing_command = root
        .join("no-such-dir")
        .join(format!("no-such-llama{}", std::env::consts::EXE_SUFFIX));
    assert!(!missing_command.exists(), "前提: command が存在しないこと");
    std::fs::write(
        app_dir_of(root).join("upstreams.json"),
        serde_json::json!({ "beta": { "main": {
            "endpoint": "http://127.0.0.1:47612", "model": "beta-model",
            "gguf": root.join("planted-beta.gguf"),
            "command": missing_command,
            "args": ["--pc-touch", marker, "--pc-exit", "0"],
        } } })
        .to_string(),
    )
    .expect("起こせないプロファイルを置けること");
    let memory = memory_file_of(root);
    std::fs::write(&memory, "beta\n").expect("覚えた名前を置けること");

    let mut started = spawn_on_machine(root, &bin_dir, &[], &[]);
    let (port, early) = started.listening_port_keeping_lines();
    let report = upstream_report(port);
    assert_faces_on(&report, "beta");
    assert_eq!(
        report["main"]["launch"]["state"],
        serde_json::json!("not_started"),
        "{report}"
    );

    let (status, body) = call(
        port,
        reqwest::Method::POST,
        "/admin/upstream/profile",
        Some(serde_json::json!({ "name": "beta" })),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["main"]["launch"]["state"],
        serde_json::json!("failed"),
        "起こせなかった面が failed になっていない: {body}"
    );
    assert!(
        !body.to_string().contains("no-such-dir"),
        "応答に command の綴りが載っている: {body}"
    );

    // **起こすこと自体に失敗した面は、同じ名前でもう一度起こしに行く**（F-10a のレビュー是正 /
    // `switch` のモジュール doc）——子が束に入らないので、次の依頼でも「新しく起こす」になる。
    // **数えるのは「起こせませんでした」の行**（2回目の依頼が何も触らない実装なら 1 のまま）。
    let (status, body) = call(
        port,
        reqwest::Method::POST,
        "/admin/upstream/profile",
        Some(serde_json::json!({ "name": "beta" })),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["main"]["launch"]["state"],
        serde_json::json!("failed"),
        "{body}"
    );

    let lines = all_lines(&mut started, early);
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("差し替えた上流を起こせませんでした"))
            .count(),
        2,
        "同じ名前の2回目の依頼で、起こせなかった面をもう一度起こしに行っていない\n{}",
        lines.join("\n")
    );
    assert_started_with(&lines, "beta", "remembered");
    assert!(
        !lines
            .iter()
            .any(|line| line.contains(REMEMBERED_UNUSABLE_LINE)),
        "起こしていないのに、使えないと言って既定へ落ちている\n{}",
        lines.join("\n")
    );
    assert!(started.running.is_alive(), "止まっている");
    assert!(!marker.exists(), "起こせないはずの上流が起きている");
    assert_eq!(
        std::fs::read(&memory).expect("読めること"),
        b"beta\n",
        "覚えた名前が変わっている"
    );
}

/// **`PERSONACORE_UPSTREAMS` で別のファイルを読んでいる起動は、覚えた名前を読みも書きもしない**
/// （ADR-0042 §決定9）。
///
/// **指した先にも覚えた名前と同じ名前（`gamma`）を置く**——読んでしまう実装なら、
/// 指した先の `gamma` を向くので観測できる。
///
/// | 腕 | 覚えたファイル（既定の置き場所と、指した先の隣の両方） | 期待 |
/// |---|---|---|
/// | 在る | `gamma` | 使わない・どちらのバイト列も変わらない |
/// | 無い | — | どちらにも作らない |
///
/// **指した先の隣にも置く**——「指した先の隣を読む／書く」実装も観測する。
#[test]
fn an_overridden_profile_file_neither_reads_nor_writes_the_remembered_name() {
    for remembered in [Some("gamma\n"), None] {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let root = dir.path();
        let (bin_dir, marker) = plant_remembered_profiles(root);
        let memory = memory_file_of(root);
        let override_dir = root.join("repo").join("scripts");
        std::fs::create_dir_all(&override_dir).expect("作れること");
        let memory_beside_override = override_dir.join("selected-upstream.txt");
        if let Some(body) = remembered {
            std::fs::write(&memory, body).expect("覚えた名前を置けること");
            std::fs::write(&memory_beside_override, body).expect("覚えた名前を置けること");
        }
        let override_file = override_dir.join("upstreams.json");
        std::fs::write(
            &override_file,
            r#"{ "alpha": { "main": { "endpoint": "http://127.0.0.1:47621", "model": "override-alpha" } },
                 "gamma": { "main": { "endpoint": "http://127.0.0.1:47623", "model": "override-gamma" } } }"#,
        )
        .expect("置けること");
        let extra = [("PERSONACORE_UPSTREAMS", override_file.as_os_str())];

        // **引数なし**——覚えた名前を読まない。切り替えても覚えない。
        {
            let mut started = spawn_on_machine(root, &bin_dir, &[], &extra);
            let (port, early) = started.listening_port_keeping_lines();
            assert_faces_on_the_environment(&upstream_report(port));
            let lines = all_lines(&mut started, early);
            assert!(
                !lines
                    .iter()
                    .any(|line| line.contains(REMEMBERED_UNUSABLE_LINE)),
                "{remembered:?}: 読まないはずの覚えた名前に触れている\n{}",
                lines.join("\n")
            );
            let (status, body) = call(
                port,
                reqwest::Method::POST,
                "/admin/upstream/profile",
                Some(serde_json::json!({ "name": "alpha" })),
            );
            assert_eq!(status, 200, "{body}");
        }
        // **`--upstream`**——覚えない。
        {
            let mut started = spawn_on_machine(root, &bin_dir, &["--upstream", "gamma"], &extra);
            let port = started.listening_port();
            assert_eq!(
                upstream_report(port)["main"]["model"],
                serde_json::json!("override-gamma"),
                "前提: 指した先から選んでいること"
            );
        }

        for place in [&memory, &memory_beside_override] {
            match remembered {
                Some(body) => assert_eq!(
                    std::fs::read(place).expect("読めること"),
                    body.as_bytes(),
                    "PERSONACORE_UPSTREAMS の起動が、覚えた名前を書き換えている: {}",
                    place.display()
                ),
                None => assert!(
                    !place.exists(),
                    "PERSONACORE_UPSTREAMS の起動が覚えている: {}",
                    place.display()
                ),
            }
        }
        assert_eq!(temporaries_in(&app_dir_of(root)), Vec::<String>::new());
        assert_eq!(temporaries_in(&override_dir), Vec::<String>::new());
        assert!(!marker.exists(), "起こさないはずの上流を起こしている");
    }
}

/// **`--fresh` は DB を退避するが、覚えた名前は忘れない**（ADR-0042 §決定9 / 利用者の決定）。
///
/// **DB も既定の置き場所に置く**——覚えたファイルと同じディレクトリで `--fresh` が動くこと。
/// **退避が実際に起きたこと**（`bak-` の名前が出来た）まで見る——起きていなければ、何も見ていない。
#[test]
fn a_fresh_start_still_remembers_the_profile() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (bin_dir, _marker) = plant_remembered_profiles(root);
    let memory = memory_file_of(root);
    std::fs::write(&memory, "gamma\n").expect("覚えた名前を置けること");

    // **1回目: DB を作る。**
    {
        let mut first = spawn_on_machine(root, &bin_dir, &[], &[]);
        let port = first.listening_port();
        assert_faces_on(&upstream_report(port), "gamma");
    }
    assert!(
        app_dir_of(root).join("personacore.sqlite").is_file(),
        "前提: DB が既定の置き場所に在ること"
    );

    // **2回目: `--fresh`。**
    let mut fresh = spawn_on_machine(root, &bin_dir, &["--fresh"], &[]);
    let port = fresh.listening_port();
    assert_faces_on(&upstream_report(port), "gamma");
    let stashed: Vec<String> = std::fs::read_dir(app_dir_of(root))
        .expect("読めること")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("bak-"))
        .collect();
    assert!(!stashed.is_empty(), "前提: --fresh が DB を退避したこと");
    assert_eq!(
        std::fs::read_to_string(&memory).expect("--fresh の後も覚えたファイルが在ること"),
        "gamma\n",
        "--fresh で覚えた名前を忘れている"
    );
}

/// **覚えたファイルを読めない・書けなくても、起動も切り替えも止めない**（ADR-0042 §決定9）。
///
/// **その名前にディレクトリを置く**——読めない・置き換えられない（Windows でも Unix でも）。
#[test]
fn an_unusable_memory_file_stops_neither_the_start_nor_the_switch() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (bin_dir, _marker) = plant_remembered_profiles(root);
    let memory = memory_file_of(root);
    std::fs::create_dir_all(&memory).expect("その名前にディレクトリを置けること");

    {
        let mut started = spawn_on_machine(root, &bin_dir, &[], &[]);
        let port = started.listening_port();
        assert_faces_on_the_environment(&upstream_report(port));
        let (status, body) = call(
            port,
            reqwest::Method::POST,
            "/admin/upstream/profile",
            Some(serde_json::json!({ "name": "gamma" })),
        );
        assert_eq!(status, 200, "覚えられないと切り替えまで断っている: {body}");
        assert_faces_on(&upstream_report(port), "gamma");
        assert!(started.running.is_alive(), "止まっている");
    }
    {
        let mut started = spawn_on_machine(root, &bin_dir, &["--upstream", "alpha"], &[]);
        let port = started.listening_port();
        assert_faces_on(&upstream_report(port), "alpha");
        assert!(started.running.is_alive(), "止まっている");
    }
    assert!(memory.is_dir(), "置いたディレクトリが消えている");
    assert_eq!(temporaries_in(&app_dir_of(root)), Vec::<String>::new());
}

/// **断った切り替え（404 / 422）は覚えない。受け付けた切り替えだけを覚える**
/// （ADR-0042 §決定9「受け付けられたとき」/ レビュー1周目の是正2）。
///
/// | 依頼 | 応答 | 覚えた名前 |
/// |---|---|---|
/// | 無い名前（`vanished`） | 404 | `gamma` のまま |
/// | gguf の無いプロファイル（`delta`） | 422 | `gamma` のまま |
/// | **対照**: 在る名前（`alpha`） | 200 | `alpha` になる |
///
/// **対照を同じ起動の最後に置く**——この起動が覚える経路を持っていることを見てから、
/// 前の2つの「変わらない」を読む（覚えない起動なら、どれも変わらずに緑になる）。
#[test]
fn a_refused_switch_is_not_remembered() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (bin_dir, marker) = plant_remembered_profiles(root);
    let memory = memory_file_of(root);
    std::fs::write(&memory, "gamma\n").expect("覚えた名前を置けること");

    let mut started = spawn_on_machine(root, &bin_dir, &[], &[]);
    let port = started.listening_port();
    assert_faces_on(&upstream_report(port), "gamma");

    for (name, expected_status, code) in [
        ("vanished", 404, "upstream_profile_not_found"),
        ("delta", 422, "upstream_gguf_missing"),
    ] {
        let (status, body) = call(
            port,
            reqwest::Method::POST,
            "/admin/upstream/profile",
            Some(serde_json::json!({ "name": name })),
        );
        assert_eq!(status, expected_status, "{name}: {body}");
        assert_eq!(body["error"]["code"], code, "{name}: {body}");
        assert_eq!(
            std::fs::read(&memory).expect("読めること"),
            b"gamma\n",
            "{name}: 断った切り替えの名前を覚えている"
        );
        // **断った切り替えは、いま選ばれている名前も変えない**（作業単位 F-10b）。
        assert_eq!(
            upstream_report(port)["selected_profile"],
            serde_json::json!("gamma"),
            "{name}: 断った切り替えで、いま選ばれている名前が変わっている"
        );
    }
    assert!(!marker.exists(), "断ったはずの切り替えが上流を起こしている");

    // **対照**: 受け付けた切り替えは覚える。
    let (status, body) = call(
        port,
        reqwest::Method::POST,
        "/admin/upstream/profile",
        Some(serde_json::json!({ "name": "alpha" })),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["selected_profile"],
        serde_json::json!("alpha"),
        "前提: 受け付けた切り替えは、いま選ばれている名前を変えること"
    );
    assert_eq!(
        std::fs::read(&memory).expect("読めること"),
        b"alpha\n",
        "前提: この起動は受け付けた切り替えを覚えること"
    );
    assert!(started.running.is_alive(), "止まっている");
}

/// **改行を含むプロファイル名で、ログの行を偽造できない**（レビュー1周目の是正3）。
const FORGED_MARKER: &str = "FORGED-LINE-MARKER";

/// **その行が、偽造した行の頭になっていないか**（改行で割れた2行目は、この綴りで始まる）。
fn forged_lines(lines: &[String]) -> Vec<&String> {
    lines
        .iter()
        .filter(|line| line.trim_start().starts_with(FORGED_MARKER))
        .collect()
}

/// **改行を含む名前は、切り替えでも `--upstream` でもログの行を割らない。覚えもしない**
/// （作業単位 F-4f / レビュー1周目の是正3）。
///
/// | 入口 | 期待 |
/// |---|---|
/// | 切り替え（`upstreams.json` の鍵に改行） | 200。**名前は同じ行の中に `\n` の綴りで出る**。覚えない（ファイルを作らない） |
/// | `--upstream <改行入りの名前>` | **引数の段で止まる**。標準出力にも標準エラーにも、偽造した行の頭が無い |
///
/// **行を割る実装では、改行の後ろの `FORGED-LINE-MARKER` が行頭に来る**——それを数える。
#[test]
fn a_profile_name_with_a_line_break_cannot_forge_a_log_line() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (bin_dir, _marker) = plant_remembered_profiles(root);
    let forged_name = format!("x\n{FORGED_MARKER}");
    let profiles = app_dir_of(root).join("upstreams.json");
    let mut body: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&profiles).expect("読めること"))
            .expect("JSON");
    body.insert(
        forged_name.clone(),
        serde_json::json!({ "main": { "endpoint": "http://127.0.0.1:47631", "model": "forged-model" } }),
    );
    std::fs::write(&profiles, serde_json::Value::Object(body).to_string())
        .expect("プロファイルを置けること");
    let memory = memory_file_of(root);

    // **切り替え。**
    {
        let mut started = spawn_on_machine(root, &bin_dir, &[], &[]);
        let (port, early) = started.listening_port_keeping_lines();
        let (status, body) = call(
            port,
            reqwest::Method::POST,
            "/admin/upstream/profile",
            Some(serde_json::json!({ "name": forged_name })),
        );
        assert_eq!(
            status, 200,
            "前提: 改行入りの名前へも切り替わること: {body}"
        );
        std::thread::sleep(Duration::from_millis(500));
        let lines = all_lines(&mut started, early);
        assert_eq!(
            forged_lines(&lines),
            Vec::<&String>::new(),
            "改行入りの名前でログの行が割れている\n{}",
            lines.join("\n")
        );
        // **名前は同じ行の中に、`\n` の綴りで出る**（行が消えたのではないことを見る）。
        let escaped = format!("upstream_profile=\"x\\n{FORGED_MARKER}\"");
        for message in [
            "上流を差し替えました",
            "上流プロファイルの名前を覚えられませんでした",
        ] {
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains(message) && has_field(line, &escaped)),
                "「{message}」の行が、名前を同じ行の中で言っていない（{escaped}）\n{}",
                lines.join("\n")
            );
        }
        assert!(!memory.exists(), "改行入りの名前を覚えている");
    }

    // **`--upstream`。** 引数の段で止まる。
    let mut command = without_inherited_settings();
    isolate_machine(&mut command, root);
    command
        .args(["--no-browser", "--upstream", &forged_name])
        .env("PERSONACORE_PORT", "0")
        .env("PERSONACORE_LLM_ENDPOINT", DECOY_ENDPOINT)
        .env("PATH", prepended_path(&bin_dir))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut running = Running(command.spawn().expect("実行ファイルを起こせること"));
    let readers = [
        running
            .0
            .stdout
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn std::io::Read + Send>),
        running
            .0
            .stderr
            .take()
            .map(|pipe| Box::new(pipe) as Box<dyn std::io::Read + Send>),
    ]
    .map(|pipe| {
        let mut pipe = pipe.expect("出力を取れること");
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut pipe, &mut text);
            text
        })
    });
    let status = running.wait_for_exit("改行入りの --upstream");
    let lines: Vec<String> = readers
        .into_iter()
        .flat_map(|reader| {
            reader
                .join()
                .expect("出力を読み切れること")
                .lines()
                .map(strip_ansi)
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        !status.success(),
        "改行入りの名前で起動している（{status:?}）"
    );
    assert_eq!(
        forged_lines(&lines),
        Vec::<&String>::new(),
        "改行入りの --upstream で行が割れている\n{}",
        lines.join("\n")
    );
    assert!(!memory.exists(), "改行入りの名前を覚えている");
}

/// **`upstreams.json` の鍵に改行があっても、覚えた名前が使えないときの警告は割れない**
/// （レビュー2周目の要修正 / 形 G）。
///
/// **覚えた名前 `vanished` は無い**ので、`select` の失敗の鎖が**在る名前を全部並べる**
/// （`upstreams::UnknownProfile`）——その中に改行入りの鍵がある。
///
/// **`a_profile_name_with_a_line_break_cannot_forge_a_log_line` と別の関数にしてある**のは、
/// 変異検査で「この見張り番だけを外す」を `--skip` 1つで書けるようにするため。
#[test]
fn a_line_break_in_a_listed_profile_name_cannot_forge_the_fallback_warning() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (bin_dir, _marker) = plant_remembered_profiles(root);
    let profiles = app_dir_of(root).join("upstreams.json");
    let mut body: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&profiles).expect("読めること"))
            .expect("JSON");
    body.insert(
        format!("x\n{FORGED_MARKER}"),
        serde_json::json!({ "main": { "endpoint": "http://127.0.0.1:47631", "model": "forged-model" } }),
    );
    std::fs::write(&profiles, serde_json::Value::Object(body).to_string())
        .expect("プロファイルを置けること");
    std::fs::write(memory_file_of(root), "vanished\n").expect("覚えた名前を置けること");

    let mut started = spawn_on_machine(root, &bin_dir, &[], &[]);
    let (port, early) = started.listening_port_keeping_lines();
    assert_faces_on_the_environment(&upstream_report(port));
    let lines = all_lines(&mut started, early);
    // **前提: 警告の行は出ている**（出ていなければ、割れないことを見ていない）。
    assert!(
        lines
            .iter()
            .any(|line| line.contains(REMEMBERED_UNUSABLE_LINE)
                && has_field(line, "upstream_profile=\"vanished\"")
                && line.contains(&format!("\\n{FORGED_MARKER}"))),
        "前提: 警告の同じ行が、並べた名前を `\\n` の綴りで言っていること\n{}",
        lines.join("\n")
    );
    assert_eq!(
        forged_lines(&lines),
        Vec::<&String>::new(),
        "並べた名前の改行で、警告の行が割れている\n{}",
        lines.join("\n")
    );
}

/// **覚えた名前のプロファイルの `command` に改行があっても、起こす行も起こせなかった警告も割れない**
/// （レビュー2周目の要修正 / 形 F）。
///
/// **gguf は在る**ので起こす前の確かめは通る。**覚えた名前の起動は起こさない**（ADR-0055 / 作業単位 F-10a）
/// ので、**同じ名前を頼んで起こし、そこで失敗させる**
/// ——`launch::spawn_one` の「上流を起こします」の行（`upstream_command`）と、
/// 差し替えの「起こせませんでした」の警告（`SpawnFailure` の鎖）の2行が名前を載せる。
///
/// **F-10a より前は、2行目が `main` の起き直しの警告だった**（起動の中で起こして失敗していた。
/// その枝は消えた）。
#[test]
fn a_line_break_in_a_remembered_command_cannot_forge_a_log_line() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let (bin_dir, marker) = plant_remembered_profiles(root);
    let command = format!(
        "{}\n{FORGED_MARKER}{}",
        root.join("no-such-dir").display(),
        std::env::consts::EXE_SUFFIX
    );
    std::fs::write(
        app_dir_of(root).join("upstreams.json"),
        serde_json::json!({ "beta": { "main": {
            "endpoint": "http://127.0.0.1:47612", "model": "beta-model",
            "gguf": root.join("planted-beta.gguf"),
            "command": command,
            "args": ["--pc-touch", marker, "--pc-exit", "0"],
        } } })
        .to_string(),
    )
    .expect("プロファイルを置けること");
    std::fs::write(memory_file_of(root), "beta\n").expect("覚えた名前を置けること");

    let mut started = spawn_on_machine(root, &bin_dir, &[], &[]);
    let (port, early) = started.listening_port_keeping_lines();
    assert_faces_on(&upstream_report(port), "beta");
    let (status, body) = call(
        port,
        reqwest::Method::POST,
        "/admin/upstream/profile",
        Some(serde_json::json!({ "name": "beta" })),
    );
    assert_eq!(status, 200, "{body}");
    std::thread::sleep(Duration::from_millis(500));
    let lines = all_lines(&mut started, early);
    let escaped = format!("\\n{FORGED_MARKER}");
    // **前提: 2つの行は出ている**（出ていなければ、割れないことを見ていない）。
    assert!(
        lines.iter().any(|line| line.contains("上流を起こします")
            && line.contains("upstream_command=")
            && line.contains(&escaped)),
        "前提: 「上流を起こします」の行が command を `\\n` の綴りで言っていること\n{}",
        lines.join("\n")
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("差し替えた上流を起こせませんでした")
                && line.contains(&escaped)),
        "前提: 起こせなかった警告の同じ行が、command を `\\n` の綴りで言っていること\n{}",
        lines.join("\n")
    );
    assert_eq!(
        forged_lines(&lines),
        Vec::<&String>::new(),
        "command の改行で行が割れている\n{}",
        lines.join("\n")
    );
    assert!(started.running.is_alive(), "止まっている");
    assert!(!marker.exists(), "起こせないはずの上流が起きている");
}

// ----------------------------------------------------------------------
// 走査本体の対照（**植えたソースを、同じ本体に通す** / CLAUDE.md `15b`）
// ----------------------------------------------------------------------

/// `strip_ansi` は、渡された綴りの色づけだけを落とすこと。
#[test]
fn the_ansi_scan_strips_only_the_escapes_in_the_value_it_is_given() {
    // **期待値そのものを固定する**（`contains` では「落としすぎ」も見えない）。
    assert_eq!(
        strip_ansi("\u{1b}[3maddr\u{1b}[0m\u{1b}[2m=\u{1b}[0m127.0.0.1:65354"),
        "addr=127.0.0.1:65354"
    );
    // **色づけが無ければ1バイトも触らない。**
    assert_eq!(strip_ansi("addr=127.0.0.1:1"), "addr=127.0.0.1:1");
    // **日本語も落とさない**（この家のログは日本語である）。
    assert_eq!(
        strip_ansi("\u{1b}[32m INFO\u{1b}[0m 待ち受けを開始しました"),
        " INFO 待ち受けを開始しました"
    );
}

/// `port_in` は、渡された行の値を読むこと。
#[test]
fn the_port_scan_reads_the_line_it_is_given() {
    assert_eq!(
        port_in("2026-09-18 INFO personacore::server: addr=127.0.0.1:54321 ui_files=3"),
        Some(54_321)
    );
    // **色づけが付いていても読めること**（本物のログはこちらの形）。
    assert_eq!(
        port_in(
            "\u{1b}[3maddr\u{1b}[0m\u{1b}[2m=\u{1b}[0m127.0.0.1:54321 \u{1b}[3mui_files\u{1b}[0m"
        ),
        Some(54_321)
    );
    // **欄が無ければ読まない**（別の行を掴まない）。
    assert_eq!(port_in("addr=0.0.0.0:54321"), None);
    assert_eq!(port_in("port=54321"), None);
    // **行末でも読める。**
    assert_eq!(port_in("addr=127.0.0.1:1"), Some(1));
}

// ----------------------------------------------------------------------
// 画面を配らない形（`--no-ui` / ADR-0047 §① / 作業単位 P-1 / Issue origin#44）
// ----------------------------------------------------------------------

/// **`--no-ui` の起動が言う1行の本文**（`src/server/mod.rs` の `NO_UI_LINE`）。
///
/// **写しである。** 文面が変わったら、下の腕は「言っていない」で赤になる（空振りの向きは安全側）。
const NO_UI_LINE_COPY: &str = "画面は配りません";

/// **画面が1件も無いときの警告の書き出し**（`src/server/mod.rs` の2つの文面の写し）。
///
/// **配らないと決めた起動は、どちらも言わない。** 言うと利用者は `pnpm build` を探しに行く。
const UI_EMPTY_WARNINGS: [&str; 2] = [
    "UI の配り先が見つかりません",
    "画面が実行ファイルに1件も埋め込まれていません",
];

/// **ブラウザを開きに行った／開けなかった／テスト用ビルドだから開かない、の行**
/// （`src/server/browser.rs` の `open` の文面の写し）。
///
/// **`--no-ui` の起動は、どれも出さない**——出れば `browser::open` に入っている。
const BROWSER_OPEN_LINES: [&str; 3] = [
    "既定のブラウザで画面を開きます",
    "ブラウザを開けませんでした",
    TEST_BUILD_OPENS_NOTHING,
];

/// **配り先に置く目印**（`vite` が出す名前ではない綴り / `tests/server_web_ui.rs` と同じ作法）。
const NO_UI_MARKER_PATH: &str = "/marker-that-is-never-built-by-vite";
const NO_UI_MARKER_BODY: &str = "UI-FROM-DISK-MARKER-P1";
const NO_UI_INDEX_BODY: &str = "<!doctype html><title>UI-INDEX-MARKER-P1</title>";

/// **旗の有無で比べるキャラクター。** 既定ではない尺度と形式指示を持つ
/// （空の DB どうしを比べると、「常に空の一覧を返す」が一致で通る）。
const NO_UI_HANDLE: &str = "noui-probe";
const NO_UI_SCALE: u64 = 7;

/// **1回叩いて、応答をバイトのまま返す**（状態・`content-type`・API の版のヘッダ・本文）。
///
/// **JSON として読まない**——読むと、欄の順番や空白の違い（＝バイトの違い）が消える。
///
/// **API の版のヘッダ（`X-Personacore-Api-Version` / ADR-0047 §② P-2b）も比べる組に入れる**
/// ——ルータに届いた要求への全部の応答に付くので、旗の有無で比べる組の一部になる
/// （片方にだけ付けば食い違いで赤）。
fn raw_exchange(
    port: u16,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (u16, Option<String>, Option<String>, Vec<u8>) {
    let url = format!("http://127.0.0.1:{port}{path}");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime を作れること")
        .block_on(async move {
            // **上限は待つ側に**（`call` と同じ長さ・同じ理由）。
            let mut request = reqwest::Client::new()
                .request(method, &url)
                .timeout(Duration::from_secs(120));
            if let Some(body) = body {
                request = request.json(&body);
            }
            let response = request.send().await.expect("待ち受けへ繋がること");
            let status = response.status().as_u16();
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .map(|value| value.to_str().unwrap_or("<not ascii>").to_owned());
            let api_version = response
                .headers()
                .get(API_VERSION_HEADER)
                .map(|value| value.to_str().unwrap_or("<not ascii>").to_owned());
            let body = response.bytes().await.expect("本文を読めること").to_vec();
            (status, content_type, api_version, body)
        })
}

/// **旗の有無で同じであるべき API の口**（読むだけの口と、API の 404 / 405）。
///
/// **`/admin/upstream` は入れない**——`profile_sample`（最初の起動だけが見本を書く）と
/// 上流の観測が起動ごとに違うので、旗と関係なく食い違う。
fn api_probes() -> Vec<(reqwest::Method, String)> {
    let h = NO_UI_HANDLE;
    vec![
        (reqwest::Method::GET, "/v1/models".to_owned()),
        (reqwest::Method::GET, "/admin/characters".to_owned()),
        (reqwest::Method::GET, format!("/admin/characters/{h}/stats")),
        (
            reqwest::Method::GET,
            format!("/admin/characters/{h}/attributes"),
        ),
        (
            reqwest::Method::GET,
            format!("/admin/characters/{h}/memories"),
        ),
        (
            reqwest::Method::GET,
            format!("/admin/characters/{h}/sessions"),
        ),
        (reqwest::Method::GET, format!("/admin/characters/{h}/drift")),
        // **API の未知パスは、旗があっても OpenAI 形式のエラーのまま**（ADR-0031 §4）。
        (reqwest::Method::GET, "/v1/does-not-exist".to_owned()),
        (reqwest::Method::GET, "/admin/does-not-exist".to_owned()),
        // **API の未対応メソッドも同じ**（`/admin/characters/{h}` は `PATCH` だけ / ADR-0016 §1）。
        (reqwest::Method::DELETE, "/v1/models".to_owned()),
        (reqwest::Method::GET, format!("/admin/characters/{h}")),
        // **仕様書を配る口は画面ではなく API の一部**（ADR-0047 §② P-2b）——旗があっても配る。
        (reqwest::Method::GET, "/openapi.json".to_owned()),
    ]
}

/// **`--no-ui` は画面だけを消し、API の応答は1バイトも変えない**（Issue origin#44 の閉じる条件1・2）。
///
/// # 組み方
///
/// **同じ置き場所（＝同じ DB）で、旗なし → 旗ありの順に2回起こす。**
/// 1回目で既定ではない値のキャラクターを作り、2回目は何も書かずに読む
/// ——**どちらの起動も、同じ DB の中身を読んで答える。**
///
/// **配り先は `PERSONACORE_WEB_DIR` で目印入りのディレクトリにする**——
/// 埋め込み（`web/dist`）は `pnpm build` していない機では空なので、
/// 「旗なしで配る」を観測できない。**配り先を指していても、旗があれば配らないこと**まで見る。
/// 埋め込みの腕は下の `the_no_ui_flag_never_opens_the_browser_and_says_so_on_one_line`。
///
/// | 主張 | 破れると |
/// |---|---|
/// | 旗なしで `/` と目印が 200・中身どおり | 対比が消える（常に 404 の実装が通る） |
/// | 旗ありで `/` と目印が 404 | 画面を配らないつもりで配る |
/// | 旗ありで API の口が、状態・`content-type`・本文のバイトまで旗なしと同じ | 画面の有無で API が変わる |
/// | 比べる応答が作ったキャラクターの値を持つ | 空の DB どうしの一致で自己成就する |
/// | 比べる組に API の版のヘッダが入り、旗の有無のどちらでも値が `API_VERSION`（画面の 200 / 404 にも） | 版を名乗らない応答が残る・片方の起動にだけ付く（P-2b） |
/// | `/openapi.json` が旗の有無のどちらでも 200 でバイトまで同じ | 画面と一緒に仕様書まで消える（P-2b） |
#[test]
fn the_no_ui_flag_withholds_the_screen_and_leaves_the_api_byte_for_byte() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let web_dir = root.join("web-dist");
    std::fs::create_dir_all(&web_dir).expect("配り先を作れること");
    std::fs::write(web_dir.join("index.html"), NO_UI_INDEX_BODY).expect("置けること");
    std::fs::write(web_dir.join(&NO_UI_MARKER_PATH[1..]), NO_UI_MARKER_BODY).expect("置けること");
    let extra: [(&str, &Path); 1] = [("PERSONACORE_WEB_DIR", &web_dir)];

    // --- 1回目: 旗なし。**画面を配り、キャラクターを作る。** ---
    let mut plain = spawn_first_run_with(root, &["--no-browser"], None, &extra);
    let port = plain.listening_port();

    for (path, expected) in [
        ("/", NO_UI_INDEX_BODY),
        (NO_UI_MARKER_PATH, NO_UI_MARKER_BODY),
    ] {
        let (status, _, api_version, body) = raw_exchange(port, reqwest::Method::GET, path, None);
        assert_eq!(status, 200, "前提: 旗なしでは {path} を配ること");
        assert_eq!(
            api_version.as_deref(),
            Some(API_VERSION),
            "画面の {path} にも API の版のヘッダが付くこと"
        );
        assert_eq!(
            body,
            expected.as_bytes(),
            "前提: 旗なしの {path} は配り先の中身"
        );
    }

    let (status, _, _, body) = raw_exchange(
        port,
        reqwest::Method::POST,
        "/admin/characters",
        Some(serde_json::json!({
            "handle": NO_UI_HANDLE,
            "display_name": "画面なしの確かめ",
            "base_persona": "画面の有無で答えが変わらないことを確かめるためのキャラクター。",
            "output_format_instruction": "台詞は「」で囲む。",
            "stat_scale": NO_UI_SCALE,
        })),
    );
    assert_eq!(status, 201, "{}", String::from_utf8_lossy(&body));

    let before: Vec<_> = api_probes()
        .into_iter()
        .map(|(method, path)| {
            let answer = raw_exchange(port, method.clone(), &path, None);
            (method, path, answer)
        })
        .collect();
    drop(plain);

    // **比べる応答が、作った値を持っていること**（空の DB どうしの一致で通らないように）。
    let models = String::from_utf8_lossy(&before[0].2.3).into_owned();
    assert!(
        models.contains(NO_UI_HANDLE),
        "前提: 一覧に作ったキャラクターが居ること: {models}"
    );
    let characters: serde_json::Value =
        serde_json::from_slice(&before[1].2.3).expect("キャラクターの一覧を JSON で読めること");
    let scales: Vec<&serde_json::Value> = characters
        .as_array()
        .or_else(|| characters["characters"].as_array())
        .into_iter()
        .flatten()
        .filter(|character| character["handle"] == NO_UI_HANDLE)
        .map(|character| &character["stat_scale"])
        .collect();
    assert_eq!(
        scales,
        vec![&serde_json::json!(NO_UI_SCALE)],
        "前提: 作ったキャラクターの既定ではない尺度が一覧から読めること: {characters}"
    );

    // --- 2回目: 旗あり。同じ DB・同じ配り先。 ---
    let mut headless = spawn_first_run_with(root, &["--no-ui"], None, &extra);
    let port = headless.listening_port();

    for path in ["/", "/index.html", NO_UI_MARKER_PATH] {
        let (status, _, api_version, body) = raw_exchange(port, reqwest::Method::GET, path, None);
        assert_eq!(
            status,
            404,
            "--no-ui なのに {path} を配っている: {}",
            String::from_utf8_lossy(&body)
        );
        assert_eq!(
            api_version.as_deref(),
            Some(API_VERSION),
            "--no-ui の画面の 404（{path}）にも API の版のヘッダが付くこと"
        );
    }

    for (method, path, expected) in before {
        let actual = raw_exchange(port, method.clone(), &path, None);
        assert_eq!(
            actual,
            expected,
            "--no-ui で {method} {path} の応答が変わった\n旗あり: {}\n旗なし: {}",
            String::from_utf8_lossy(&actual.3),
            String::from_utf8_lossy(&expected.3)
        );
        // **一致だけでは「どちらにも付いていない」が通る。** 付いていて、値が定数であること。
        assert_eq!(
            expected.2.as_deref(),
            Some(API_VERSION),
            "{method} {path} に API の版のヘッダが付くこと"
        );
        if path == "/openapi.json" {
            assert_eq!(expected.0, 200, "旗なしでも仕様書を配ること");
            let document: serde_json::Value =
                serde_json::from_slice(&expected.3).expect("仕様書は JSON であること");
            assert_eq!(document["info"]["version"], serde_json::json!(API_VERSION));
        }
    }
    assert!(headless.running.is_alive(), "待ち受けが続いていること");
}

/// **`--no-ui` はブラウザを開く経路に入らず、「画面は配りません」を1行で言う**
/// （Issue origin#44 の閉じる条件3・4）。
///
/// # `--no-browser` の見張り番と同じ形
///
/// **偽物は渡してある**（`an_existing_profile_file_is_left_alone_and_no_browser_keeps_it_closed` と同じ理由
/// ——渡さなければ、`testing` 付きのビルドはそもそも開かないので、旗を見ていないのと同じになる）。
/// **`--no-browser` は渡さない**——渡すと、`--no-ui` が開かないのか `--no-browser` が開かないのかが分からない。
///
/// # 同じ行の中で見る（CLAUDE.md `B-1-2`）
///
/// * **「画面は配りません」の行が `ui_source=none` を持つ**（集計はこの欄で引く）。
/// * **待ち受けを始めた行（`addr=` の行）も `ui_source=none` を持つ**——`embedded` と言えば嘘になる。
/// * **ブラウザを開きに行った行が1本も無い**（開けば成否どちらかの行が出る）。
/// * **画面が空だという警告が出ない**（配らないと決めた起動に `pnpm build` を探させない）。
///
/// # 2つの腕
///
/// | 腕 | 画面の出どころ | 何を足すか |
/// |---|---|---|
/// | 埋め込み | `PERSONACORE_WEB_DIR` なし | 旗が埋め込みの腕にも効くこと |
/// | 無い配り先 | `PERSONACORE_WEB_DIR` = 無いディレクトリ | **空の警告を必ず出す形**（旗が無ければ `UI の配り先が見つかりません` が出る） |
#[test]
fn the_no_ui_flag_never_opens_the_browser_and_says_so_on_one_line() {
    for missing_web_dir in [false, true] {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let root = dir.path();
        let browser_log = root.join("browser-calls.jsonl");
        let nowhere = root.join("no-such-web-dir");
        let extra: Vec<(&str, &Path)> = if missing_web_dir {
            vec![("PERSONACORE_WEB_DIR", &nowhere)]
        } else {
            Vec::new()
        };

        let mut started = spawn_first_run_with(root, &["--no-ui"], Some(&browser_log), &extra);
        let (port, early) = started.listening_port_keeping_lines();

        let (status, _, _, _) = raw_exchange(port, reqwest::Method::GET, "/", None);
        assert_eq!(
            status, 404,
            "--no-ui なのに / を配っている（{missing_web_dir}）"
        );

        // **開くなら、もう開いている時刻まで待ってから数える**（開く腕は 1 秒以内に開く）。
        std::thread::sleep(Duration::from_millis(1_500));
        assert!(started.running.is_alive(), "待ち受けが続いていること");
        assert_eq!(
            recorded_calls(&browser_log),
            Vec::<Vec<String>>::new(),
            "--no-ui なのにブラウザを開いている（{missing_web_dir}）"
        );

        let lines = all_lines(&mut started, early);
        let text = lines.join("\n");
        assert!(
            lines
                .iter()
                .any(|line| line.contains(NO_UI_LINE_COPY) && has_field(line, "ui_source=none")),
            "「画面は配りません」を ui_source=none の行で言っていない（{missing_web_dir}）\n{text}"
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains(NO_UI_LINE_COPY))
                .count(),
            1,
            "「画面は配りません」は1行だけ（{missing_web_dir}）\n{text}"
        );
        assert!(
            lines
                .iter()
                .any(|line| port_in(line) == Some(port) && has_field(line, "ui_source=none")),
            "待ち受けを始めた行が ui_source=none と言っていない（{missing_web_dir}）\n{text}"
        );
        for said in BROWSER_OPEN_LINES.iter().chain(UI_EMPTY_WARNINGS.iter()) {
            assert!(
                !lines.iter().any(|line| line.contains(said)),
                "--no-ui の起動が「{said}」と言っている（{missing_web_dir}）\n{text}"
            );
        }
    }
}

/// **対比: 旗なしで無い配り先を指すと、空の警告が出る**（上の腕の「言わない」が空振りでないこと）。
///
/// **文面の写し（[`UI_EMPTY_WARNINGS`]）が古びると、上の腕の「言わない」は黙って成り立つ。**
/// **同じ写しで「言う」側を1本持つ**ので、写しが古びればここが赤になる。
#[test]
fn without_the_no_ui_flag_a_missing_web_dir_is_still_warned_about() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let root = dir.path();
    let nowhere = root.join("no-such-web-dir");
    let extra: [(&str, &Path); 1] = [("PERSONACORE_WEB_DIR", &nowhere)];

    let mut started = spawn_first_run_with(root, &["--no-browser"], None, &extra);
    let (_, early) = started.listening_port_keeping_lines();
    let lines = all_lines(&mut started, early);
    assert!(
        lines.iter().any(|line| line.contains(UI_EMPTY_WARNINGS[0])),
        "旗なしで無い配り先を指したのに警告していない\n{}",
        lines.join("\n")
    );
    assert!(
        !lines.iter().any(|line| line.contains(NO_UI_LINE_COPY)),
        "旗なしなのに「画面は配りません」と言っている\n{}",
        lines.join("\n")
    );
}
