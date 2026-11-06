//! **自分で起こした上流の一生を、本物のプロセスで確かめる**
//! （[ADR-0042](../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md) §見張り番 /
//! 作業単位 F-4b-1）。**本物の `llama` にも LM Studio にも繋がない。**
//!
//! # なぜ偽の `llama` を `PATH` の先頭に置くのか（CLAUDE.md §4）
//!
//! > **本物を起こせない経路は、偽物で起こすこと。**
//! > 「走らせられないから走査だけ」は、その経路に見張り番が1本も無いのと同じ。
//!
//! **`F-9` はこれを守らず、実機で回帰を出した** ——
//! 起こす経路が死に、`llama` が GPU を 11.7 GB 掴んだまま残った。
//! **プロファイルに `command` を書けば絶対パスで指せるが、それでは
//! 「既定の綴り `llama` を `PATH` から引く」経路を一度も通らない。**
//! **だから偽物は `llama`（＋ `.exe`）という名前で置く。**
//!
//! # なぜ `App::run_with_upstreams` を直に呼ぶのか（**実行ファイルを起こさない**）
//!
//! **止めるところまで見たいから。** 実行ファイルを起こすと、テストから送れるのは
//! **`kill`（強制終了）だけ**で、**`serve` が返らない＝`Supervisor::stop` を1度も通らない。**
//! **しかも、そのとき起こした子は孤児として残る**（実測 / 2026-09-19。
//! **`cargo test` の出力を掴んだまま、`tail` が返らなくなった**）。
//!
//! `App::run_with_upstreams` は**本番の `main` が呼ぶ関数そのもの**なので、
//! **停止の経路まで同じものを通る。** `main` にしか無い1行（プロファイルを読む・重ねる）は
//! `tests/upstream_cli.rs` が実行ファイルで見る。
//!
//! # 何を守っていないか（**先に書く**）
//!
//! * **本物の `llama serve` の引数の形は見ていない。** 偽物が受け入れるだけである
//!   （本物を起こす確認は司令塔が実機で行う / CLAUDE.md §4）。
//! * **Ctrl+C の経路（`shutdown_signal`）は通っていない。** 停止の合図は oneshot。
//! * **GPU も VRAM も1バイトも使っていない。**

mod common;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use common::install_global_log;
use personacore::server::{
    App, AppConfig, BindAddr, DEFAULT_LOG_FILTER, LaunchPlan, LaunchTiming, Startup,
    UpstreamProfiles, overlay_profile, parse_profiles,
};
use tokio::sync::oneshot;

/// 待つ側の上限（回数）。**「働く側」ではなく「待つ側」に置く**（CLAUDE.md `G-4`）。
const MAX_WAITS: usize = 300;

/// 1回あたりの待ち時間。**上限は `MAX_WAITS` と合わせて 30 秒。**
const WAIT_STEP: Duration = Duration::from_millis(100);

/// **偽物が起きた印**（中身は自分の PID）。**一度も呼ばれないこと**を見る腕が読む。
const TOUCH_MARKER: &str = "fake-llama-was-started";

/// **偽の `llama` を `PATH` の先頭に置く。プロセスにつき1回だけ。**
///
/// # ⚠ このプロセスの `PATH` を書き換えている
///
/// **`App::run_with_upstreams` は同じプロセスの中で走る**ので、
/// 子へ渡す `PATH` を別に用意する道が無い（本番の経路は `PATH` を触らない）。
///
/// **[`std::sync::OnceLock`] で1回に閉じてある**——
/// **テストは並列に走る**ので、毎回書き換えると `PATH` の読み書きが競走する。
///
/// **置き場所は `CARGO_TARGET_TMPDIR`**（`target/` の下。`.gitignore` 済み）。
/// **一時ディレクトリを leak しない**ためで、テストの外には何も残らない。
fn fake_llama_on_path() -> &'static Path {
    static PLACED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    PLACED.get_or_init(|| {
        let bin_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fake-llama-launch");
        std::fs::create_dir_all(&bin_dir).expect("置き場所を作れること");
        // **名前は `llama`（＋この OS の実行ファイルの接尾辞）** ——
        // プロファイルが `command` を書かなければ、この綴りが `PATH` から引かれる。
        let target = bin_dir.join(format!("llama{}", std::env::consts::EXE_SUFFIX));
        // **⚠ 前の回の偽物がまだ生きていると、複製は「使用中」で失敗する**
        // （Windows / 2026-09-19 に実測: 打ち切られた走行が残した `llama.exe` が掴んでいた）。
        // **既に写しが在れば、それでよい**——中身は同じ実行ファイルである。
        if std::fs::copy(env!("CARGO_BIN_EXE_fake_llama"), &target).is_err() {
            assert!(
                target.is_file(),
                "偽の llama を置けず、前の写しも無い: {target:?}"
            );
        }

        let mut value = std::ffi::OsString::from(&bin_dir);
        if let Some(existing) = std::env::var_os("PATH") {
            value.push(if cfg!(windows) { ";" } else { ":" });
            value.push(existing);
        }
        // SAFETY: `OnceLock` の中なので、このプロセスで書くのは1回だけ。
        // **外す側は居ない**（足すのは `llama` しか入っていないディレクトリ）。
        unsafe { std::env::set_var("PATH", value) };
        bin_dir
    })
}
/// **偽の gguf**（中身は見られない。**実在するかだけ**が見られる）。
fn fake_gguf(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"not a real gguf").expect("置けること");
    path
}

/// **空いているポートを1つ取る。**
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("ポートを取れること");
    let port = listener.local_addr().expect("宛先を読めること").port();
    drop(listener);
    port
}

/// プロファイルの綴りから、設定と計画を作る。**`main` と同じ2つ。**
fn read(source: &str) -> (UpstreamProfiles, String) {
    (
        parse_profiles(source, None).expect("プロファイルが読めること"),
        source.to_owned(),
    )
}

/// **走っているサーバ。** 止めると、控えた子も止まる。
struct Serving {
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), personacore::server::ServerError>>>,
}

impl Serving {
    /// **止めて、`serve` が返るまで待つ。** ここを通ると `Supervisor::stop` が走る。
    async fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.await
                .expect("サーバのタスクが落ちていないこと")
                .expect("サーバが正常に止まること");
        }
    }
}

impl Drop for Serving {
    /// **落ちた経路でもタスクを止める**（`tests/fresh_cli.rs` の `Running` と同じ向き）。
    ///
    /// **`abort` では `Supervisor::stop` を通らない**ので、
    /// **止めたことを見る腕は必ず [`Self::stop`] を通ること。**
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// **本番の入口（`App::run_with_upstreams`）でサーバを起こす。**
fn serve(config: AppConfig, plan: LaunchPlan) -> Serving {
    let (shutdown, wait) = oneshot::channel();
    let task = tokio::spawn(async move {
        App::run_with_upstreams(&config, plan, async move {
            let _ = wait.await;
        })
        .await
    });
    Serving {
        shutdown: Some(shutdown),
        task: Some(task),
    }
}

/// **設定を組む。** プロファイルの4つを重ね、待ち受けと DB だけを差し替える。
///
/// **環境変数は1つも読まない**（`|_| None`）——
/// **測定中の機に立っている `PERSONACORE_*` に結果を左右させない。**
fn config_of(
    profiles: &UpstreamProfiles,
    name: &str,
    dir: &Path,
    port: u16,
) -> (AppConfig, LaunchPlan) {
    let profile = profiles.select(name).expect("プロファイルが在ること");
    let settings = profile.settings();
    let config = AppConfig::from_lookup_with_data_dir(
        overlay_profile(&settings, |_| None),
        Some(dir.to_path_buf()),
    )
    .expect("設定を組めること")
    .with_bind(BindAddr::loopback(port))
    .with_database_path(dir.join("db").join("personacore.sqlite"));
    (config, LaunchPlan::of(profile))
}

/// **`GET /admin/upstream` を読む。** まだ待ち受けていなければ `None`。
///
/// **繋がらないことを失敗にしない**——`App::run_with_upstreams` は別のタスクで
/// 走り出したところなので、**最初の何回かは必ず断られる**（実測 / 2026-09-19）。
async fn upstream_report(port: u16) -> Option<serde_json::Value> {
    let url = format!("http://127.0.0.1:{port}/admin/upstream");
    let body = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    Some(
        serde_json::from_str(&body)
            .unwrap_or_else(|err| panic!("JSON として読めること: {err} / {body}")),
    )
}

/// **面の状態が期待の綴りになるまで、上限つきで待つ。**
///
/// **待ち受けが始まるのも、ここで待つ**（最初の何回かは繋がらない）。
async fn wait_for_state(port: u16, face: &str, expected: &str) -> serde_json::Value {
    let mut last = serde_json::Value::Null;
    for _ in 0..MAX_WAITS {
        if let Some(report) = upstream_report(port).await {
            last = report[face]["launch"].clone();
            if last["state"] == serde_json::json!(expected) {
                return last;
            }
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
    panic!("{face} が {MAX_WAITS} 回待っても {expected} にならなかった（いま {last}）");
}

/// **待ち受けが始まってから、いまの応答を読む。**
async fn report_once(port: u16) -> serde_json::Value {
    for _ in 0..MAX_WAITS {
        if let Some(report) = upstream_report(port).await {
            return report;
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
    panic!("待ち受けを {MAX_WAITS} 回待っても始まらなかった");
}
/// **起こす腕が使うホスト。127.0.0.1 ではない**（レビュー是正1 / 2026-09-19）。
///
/// # なぜ `127.0.0.1` を使わないのか
///
/// **パーサはループバックを3通り受理する**（`127.0.0.0/8` / `localhost` / `[::1]`）のに、
/// **起こす腕が全部 `127.0.0.1` だと、`--host` を `"127.0.0.1"` に決め打つ変異が同値**になる
/// （実測: 是正前は 2157 全緑）。
///
/// # なぜ `localhost` ではなく `127.9.9.9` なのか
///
/// **名前解決を挟まないため。** `localhost` は `::1` にも `127.0.0.1` にも解決しうる
/// ——**偽物の待ち受けと personacore の照会が別の側を選ぶと、競走で落ちる。**
/// **`127.9.9.9` はリテラルなので解決が要らず、それでいて `127.0.0.1` ではない**
/// （`127.0.0.0/8` はまるごとループバック）。
/// **`localhost` と `[::1]` を受理することは、`tests/upstream_profiles.rs` の
/// `the_loopback_check_knows_the_three_spellings` が見ている。**
const LAUNCH_HOST: &str = "127.9.9.9";

/// **その宛先に、まだ誰か居るか。**
async fn answers(port: u16) -> bool {
    answers_at("127.0.0.1", port).await
}

/// **ホストを指定して同じことを見る**（起こす腕は `127.0.0.1` を使わない）。
async fn answers_at(host: &str, port: u16) -> bool {
    reqwest::Client::new()
        .get(format!("http://{host}:{port}/v1/models"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .is_ok()
}

/// **止まるはずの起動に渡す「停止の合図」。上限つき。**
///
/// # ⚠ `std::future::pending()` を渡さないこと（**変異検査で実測 / 2026-09-19**）
///
/// **止まるはずの起動が止まらなかったとき、テストが永久に走り続ける。**
/// **gguf の実在確認を落とす変異（`m5`）で実際にそうなった** ——
/// 偽の `llama` は `-m` の綴りを見ないので**起きてしまい**、`bind` が成功し、
/// **合図が来ないまま `cargo test` が固まった**（手で止めた）。
///
/// **上限は「待つ側」に置く**（CLAUDE.md `G-4`）——
/// **止まらなかったら、そう言って落ちる**（`expect_err` が `Ok` を見る）。
async fn bounded_shutdown() {
    tokio::time::sleep(Duration::from_secs(20)).await;
}

/// **その宛先に「先客」を立てる**（偽の `llama` を、名乗る綴りを指定して起こす）。
///
/// **`kill_on_drop(true)`** —— このテストが落ちても残さない。
fn squatter_on(bin_dir: &Path, port: u16, answers_with: &str) -> tokio::process::Child {
    tokio::process::Command::new(bin_dir.join(format!("llama{}", std::env::consts::EXE_SUFFIX)))
        .args([
            "serve",
            "-m",
            "squatter.gguf",
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--pc-answer",
            answers_with,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("先客を起こせること")
}

/// **その宛先が答えるようになるまで、上限つきで待つ。**
async fn wait_until_answers(port: u16) -> bool {
    for _ in 0..MAX_WAITS {
        if answers(port).await {
            return true;
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
    false
}

/// **待ち受けが消えるまで、上限つきで待つ。**
async fn wait_until_gone(port: u16, what: &str) {
    wait_until_gone_at("127.0.0.1", port, what).await;
}

/// **ホストを指定して同じことを待つ。**
async fn wait_until_gone_at(host: &str, port: u16, what: &str) {
    for _ in 0..MAX_WAITS {
        if !answers_at(host, port).await {
            return;
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
    panic!("{what} が {MAX_WAITS} 回待っても消えなかった");
}

// ----------------------------------------------------------------------
// 本命
// ----------------------------------------------------------------------

/// **旗を渡さない起動では、どちらの面も「起こしていない」。**
///
/// **対比の腕**（CLAUDE.md「対比は安い側だけでは足りない」）——
/// これが無いと、**常に `loading` を書く実装**がここで緑を通る。
/// **偽の `llama` は `PATH` に在るのに、一度も呼ばれない**ことまで見る。
#[tokio::test]
async fn an_empty_plan_launches_nothing_and_leaves_both_faces_external() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let marker = dir.path().join(TOUCH_MARKER);
    let port = free_port();

    let (profiles, _) = read(
        r#"{ "planted": { "main": { "endpoint": "http://127.0.0.1:1234",
                                    "model": "planted-body-model-id" } } }"#,
    );
    let (config, plan) = config_of(&profiles, "planted", dir.path(), port);
    assert!(plan.is_empty(), "gguf が無いのに起こす計画になっている");

    let mut serving = serve(config, plan);
    let report = wait_for_state(port, "main", "external").await;
    assert_eq!(report["state"], serde_json::json!("external"));
    let report = report_once(port).await;
    assert_eq!(
        report["sub"]["launch"]["state"],
        serde_json::json!("external")
    );
    assert!(!marker.exists(), "偽の llama が呼ばれている");

    serving.stop().await;
}

/// **名指しされた `llama` を `PATH` から起こし、載ったら「準備完了」になり、
/// 止めたら子も止まる。**
///
/// # 何を1本で見ているか
///
/// | 主張 | 破れると |
/// |---|---|
/// | 偽物が実際に起きる（印のファイル） | 起こす経路が死んでいる（`F-9` の回帰そのもの） |
/// | `ready` になる | 載ったのを見届けていない |
/// | 名乗った名前が **gguf のファイル名** | 頼んだ綴りで確かめている（`llama serve` は `model` を無視する / `B-45`） |
/// | **止めたら待ち受けが消える** | `Supervisor::stop` を通っていない（GPU を掴んだまま残る） |
/// | 反対の面が `external` のまま | 帰属（片面だけ起こす形が壊れる） |
#[tokio::test]
async fn the_named_gguf_is_started_reaches_ready_and_is_stopped_with_the_server() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-body-model.gguf");
    let marker = dir.path().join(TOUCH_MARKER);
    let upstream_port = free_port();
    let port = free_port();

    let (profiles, _) = read(
        &serde_json::json!({
            "planted": {
                "main": {
                    // **⚠ `127.0.0.1` にしない**（レビュー是正1 / 2026-09-19）。
                    // **パーサはループバックを3通り受理する**のに、起こす腕が全部
                    // `127.0.0.1` だと、**`--host` を `"127.0.0.1"` に決め打つ変異が同値**になる
                    // （実測: 是正前は 2159 全緑）。
                    "endpoint": format!("http://{LAUNCH_HOST}:{upstream_port}"),
                    "model": "planted-body-model-id",
                    "gguf": gguf,
                    "args": ["--pc-touch", marker],
                },
                // **外の上流を明示した面**（gguf 無し）。**起こさない側の対照。**
                // **`sub` を省くと、抽出も同じ子が担う**ので帰属が見えなくなる
                // （`tests/upstream_profiles.rs` の
                // `a_profile_without_sub_launches_one_child_for_both_faces`）。
                "sub": {
                    "endpoint": format!("http://127.0.0.1:{}", free_port()),
                    "model": "planted-sub-model-id",
                }
            }
        })
        .to_string(),
    );
    let (config, plan) = config_of(&profiles, "planted", dir.path(), port);

    let mut serving = serve(config, plan);
    let launch = wait_for_state(port, "main", "ready").await;
    assert_eq!(
        launch["answered_name"],
        serde_json::json!("planted-body-model.gguf"),
        "gguf のファイル名で「載った」と認めること: {launch}"
    );
    assert!(marker.exists(), "偽の llama が一度も呼ばれていない");

    // **`--host` に渡った値を読む**（レビュー是正1）。偽物は引数列を印に書いている。
    // **`endpoint` のホストがそのまま渡ること**が主張。
    // **⚠ 決め打ちの変異（`"127.0.0.1"`）を赤にしているのは、この2行ではなくフィクスチャ（`LAUNCH_HOST`）である**
    // —— 偽物が `127.0.0.1` で待ち受け、personacore は `LAUNCH_HOST` へ聞きに行くので、
    // 上の `wait_for_state` が `ready` に届かず、ここへ来る前に落ちる（レビュー2周目が実測。
    // この2行を消しても同じ場所で同じ理由で赤のまま）。
    // **この2行が単独で効くのは、宛先を広げる変異（`"0.0.0.0"` 等）に対して** ——
    // ワイルドカードの待ち受けは `LAUNCH_HOST` にも答えるので `ready` に届き、
    // 印の中の `--host` の綴りだけがそれを見分ける。**⚠ その変異は実測していない**
    // （この機にワイルドカードの待ち受けを作りたくない）。構成上の推論である。
    let invocation = std::fs::read_to_string(&marker).expect("印を読めること");
    assert!(
        invocation.contains(&format!("--host {LAUNCH_HOST}")),
        "--host にプロファイルのホストが渡っていない: {invocation}"
    );
    // **`--port` も同じ往復で見る**（片方だけ決め打つ変異を素通りさせない）。
    assert!(
        invocation.contains(&format!("--port {upstream_port}")),
        "--port にプロファイルのポートが渡っていない: {invocation}"
    );

    assert!(
        answers_at(LAUNCH_HOST, upstream_port).await,
        "上流がまだ答えないこと"
    );
    // **対照**: 起こした回には、親の足跡（`upstream-logs/`）が必ず残ること。
    // **これが無いと、`a_failed_bind_leaves_no_child_behind` の「無いこと」は
    // 走査が壊れているだけでも成立する。**
    let logs = dir.path().join("db").join("upstream-logs");
    assert!(
        std::fs::read_dir(&logs)
            .map(|entries| entries.count() > 0)
            .unwrap_or(false),
        "起こしたのに出力の落とし先が空（足跡の検査が空振りしている）: {logs:?}"
    );

    let report = report_once(port).await;

    // **載ったら、その往復の観測をそのまま採る**（ADR-0042 §決定3）。
    //
    // **`launch.answered_name` の写しではない** —— あちらは「載ったと認めた時点の記録」で、
    // こちらは**上流の観測**（`UpstreamFacts`）である。
    // **採らないと、次の引き直し（5分後）まで「上流は黙っている」と言い続ける** ——
    // 予算も抽出の上限も落とし所のままになる。
    // **⚠ この2行を足すまで、決定3 の「測り直す」には見張り番が1本も無かった**
    // （実測 / 2026-09-19: `adopt_face_probe` の観測の書き込みを落とす変異が 2153 全緑）。
    assert_ne!(
        report["main"]["answered_by"],
        serde_json::Value::Null,
        "載ったのに、上流の観測が「答えなし」のまま: {report}"
    );
    assert_eq!(
        report["main"]["answered_name"],
        serde_json::json!("planted-body-model.gguf"),
        "載ったのに、観測が上流の名乗りを持っていない: {report}"
    );

    // **起こしていない面は `external` のまま**（帰属）。
    assert_eq!(
        report["sub"]["launch"]["state"],
        serde_json::json!("external"),
        "起こしていない面まで動いている: {report}"
    );

    // **止めたら、控えた子も止まる**（ADR-0042 §決定2）。
    serving.stop().await;
    wait_until_gone_at(LAUNCH_HOST, upstream_port, "起こした上流").await;
}

/// **起こす面には、起動時に上流を聞かない**（ADR-0042 §決定3 / 作業単位 F-4b-1）。
///
/// # なぜ聞かないのか（G-5 の深刻度を壊さないため）
///
/// **これから起こす面は、まだ載っていない。** 聞けば必ず外れるので、
/// **`measured::report` が `Drift::Absent` を並べる** ——
/// **`llama-server` を上流にすると 9 行が全部そうなる**形（作業単位 G-5 が
/// 「1つの事実 × 9」と診断して畳んだもの）**が、起動のたびに再生される。**
/// **聞かなければ `Unanswered` に倒れ、ずれとは混ざらない。**
///
/// # どうやって観測するか（**印のファイルでは見えない**）
///
/// **宛先に「別物」を先に立てておく。**
/// **起こす面（`main`）**は聞かないので**全部 `unanswered`**、
/// **起こさない面（`sub`）**は聞くので**`drifted_absent` が立つ**
/// ——**同じ1回の応答の中で、面ごとに答えが割れる。**
///
/// **⚠ この腕を足すまで、この決定には見張り番が1本も無かった**
/// （実測 / 2026-09-19: `launches_face` を常に偽にする変異が 2153 全緑）。
#[tokio::test]
async fn the_face_we_are_about_to_start_is_not_asked_at_startup() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let bin_dir = fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-wanted-at-startup.gguf");
    let upstream_port = free_port();
    let port = free_port();

    // **先客を立てる。** 名乗る綴りは、依頼する綴りとも gguf の名前とも別。
    let mut squatter = squatter_on(bin_dir, upstream_port, "planted-squatter-name");
    assert!(
        wait_until_answers(upstream_port).await,
        "本文側の先客が答えなかった"
    );

    // **抽出の面にも先客を立てる。** **別の宛先**にするのが要点 ——
    // `sub` を省くと抽出も同じ子が担うので、**起こす面と起こさない面が作れない。**
    let sub_port = free_port();
    let mut sub_squatter = squatter_on(bin_dir, sub_port, "planted-sub-squatter-name");
    assert!(
        wait_until_answers(sub_port).await,
        "抽出側の先客が答えなかった"
    );

    let (profiles, _) = read(
        &serde_json::json!({
            "planted": {
                "main": {
                    "endpoint": format!("http://127.0.0.1:{upstream_port}"),
                    "model": "planted-body-model-id",
                    "gguf": gguf,
                },
                "sub": {
                    "endpoint": format!("http://127.0.0.1:{sub_port}"),
                    "model": "planted-sub-model-id",
                }
            }
        })
        .to_string(),
    );
    let (config, plan) = config_of(&profiles, "planted", dir.path(), port);

    let mut serving = serve(config, plan);
    let report = report_once(port).await;

    // **起こす面は、一度も聞いていない。**
    assert_eq!(
        report["main"]["answered_by"],
        serde_json::Value::Null,
        "これから起こす面に、起動時から聞きに行っている: {report}"
    );
    assert_eq!(
        report["main"]["measured"]["unanswered"], report["main"]["measured"]["checked"],
        "起こす面の突き合わせが「聞けなかった」に倒れていない（G-5 の畳み方が壊れる）: {report}"
    );

    // **対照: 起こさない面は、いままでどおり聞いている。**
    // **これが無いと、「聞いていない」は先客が居ないだけでも成立する。**
    assert_ne!(
        report["sub"]["answered_by"],
        serde_json::Value::Null,
        "起こさない面まで聞かなくなっている: {report}"
    );
    assert_ne!(
        report["sub"]["measured"]["drifted_absent"],
        serde_json::json!(0),
        "先客が答えているのに、聞いた面のずれが立っていない: {report}"
    );

    serving.stop().await;
    let _ = squatter.kill().await;
    let _ = sub_squatter.kill().await;
}

/// **止めるのは、控えた子だけ。**
///
/// # 形（ADR-0042 §見張り番）
///
/// **偽物を2つ起こす。** 片方は personacore の子、もう片方は**このテストの子。**
/// サーバを止めたあと、**こちらの子が生きていること**を見る。
///
/// **「名前で全部止める」形にすると、この腕が赤になる** ——
/// `F-9` が PowerShell で塞いだものを、Rust 側でも塞いである。
#[tokio::test]
async fn stopping_the_server_leaves_someone_elses_llama_alone() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let bin_dir = fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-ours.gguf");
    let ours_port = free_port();
    let theirs_port = free_port();
    let port = free_port();

    // **利用者が手で立てた `llama`**（このテストの子。personacore は知らない）。
    let mut theirs = tokio::process::Command::new(
        bin_dir.join(format!("llama{}", std::env::consts::EXE_SUFFIX)),
    )
    .args([
        "serve",
        "-m",
        "someone-elses.gguf",
        "--host",
        "127.0.0.1",
        "--port",
        &theirs_port.to_string(),
    ])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    // **このテストが落ちても残さない。**
    .kill_on_drop(true)
    .spawn()
    .expect("他人の llama を起こせること");

    let (profiles, _) = read(
        &serde_json::json!({
            "planted": {
                "main": {
                    "endpoint": format!("http://127.0.0.1:{ours_port}"),
                    "model": "planted-body-model-id",
                    "gguf": gguf,
                }
            }
        })
        .to_string(),
    );
    let (config, plan) = config_of(&profiles, "planted", dir.path(), port);

    let mut serving = serve(config, plan);
    wait_for_state(port, "main", "ready").await;

    // **`sub` を書いていないプロファイルなので、抽出も同じ子が担う**
    // （ADR-0039 §決定1 / `a_profile_without_sub_launches_one_child_for_both_faces`）。
    // **載ったら、2つ目の面も測り直すこと**（レビュー是正2 / 2026-09-19）——
    // **`FaceWatcher` の「2つ目以降の面を聞き直す」ループを丸ごと消す変異は、
    // 是正前は 2157 全緑だった。** その形では**抽出の面だけが「答えなし」のまま**で、
    // **抽出の上限も落とし所のまま 5 分間動かない。**
    let report = report_once(port).await;
    assert_ne!(
        report["sub"]["answered_by"],
        serde_json::Value::Null,
        "載ったのに、抽出の面の観測が「答えなし」のまま: {report}"
    );
    assert_eq!(
        report["sub"]["answered_name"],
        serde_json::json!("planted-ours.gguf"),
        "抽出の面も、上流が名乗った名前を持つこと: {report}"
    );
    // **本文の面も同じ子を見ている**（同値ではない主張——面ごとに別の往復である）。
    assert_ne!(
        report["main"]["answered_by"],
        serde_json::Value::Null,
        "本文の面の観測が「答えなし」のまま: {report}"
    );
    // **どちらの面も `ready`**（1つの子が2つの面を担う形）。
    for face in ["main", "sub"] {
        assert_eq!(
            report[face]["launch"]["state"],
            serde_json::json!("ready"),
            "sub を省いたのに、{face} が ready になっていない: {report}"
        );
    }

    serving.stop().await;

    // **控えた子は止まった。**
    wait_until_gone(ours_port, "起こした上流").await;
    // **他人の子は生きている。** しかも**その子が答えている**ことまで見る
    // （生きているだけでは、別物にすり替わっていても気づけない）。
    let answered = reqwest::get(format!("http://127.0.0.1:{theirs_port}/v1/models"))
        .await
        .expect("他人の llama が答えること")
        .text()
        .await
        .expect("本文を読めること");
    assert!(
        answered.contains("someone-elses.gguf"),
        "自分が起こしていない llama まで止めている（または別物になっている）: {answered}"
    );

    let _ = theirs.kill().await;
}

/// **子が自分で終了したら、即「失敗」**（ADR-0042 §決定3 / CLAUDE.md `G-4`）。
///
/// **死んだ書き手を待たない。** 待つと、**15 分の天井まで「載せている」と言い続ける。**
/// **ログの名前は載るが、置き場所は載らない**（絶対制約）。
#[tokio::test]
async fn a_child_that_exits_on_its_own_is_reported_as_failed() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-dies.gguf");
    let upstream_port = free_port();
    let port = free_port();

    let (profiles, _) = read(
        &serde_json::json!({
            "planted": {
                "main": {
                    "endpoint": format!("http://127.0.0.1:{upstream_port}"),
                    "model": "planted-body-model-id",
                    "gguf": gguf,
                    "args": ["--pc-exit", "7"],
                }
            }
        })
        .to_string(),
    );
    let (config, plan) = config_of(&profiles, "planted", dir.path(), port);

    let mut serving = serve(config, plan);
    let launch = wait_for_state(port, "main", "failed").await;
    let reason = launch["reason"].as_str().unwrap_or_default();
    assert!(
        reason.contains('7'),
        "終了コードを言うこと（どこを見に行けばよいか）: {launch}"
    );
    let log = launch["log"].as_str().expect("ログの名前が載ること");
    assert!(
        log.ends_with(".err.log"),
        "標準エラーの名前であること: {log}"
    );
    // **置き場所は載らない**（絶対制約 / `server::upstream` の doc）。
    assert!(
        !log.contains('/') && !log.contains('\\'),
        "応答に置き場所が載っている: {log}"
    );

    serving.stop().await;
}

/// **別の名前を名乗ったら「失敗」**（載ったのは別のモデル）。
///
/// **`llama serve` は `model` の欄を無視する**ので、
/// **「200 が返った」を「載った」と読むと、別物が答えたまま緑になる**（`C-18` / `B-45`）。
///
/// **3つの綴りを全部違える**——依頼する綴り・期待する gguf の名前・実際に名乗る綴り。
/// 同値だと、どの取り違えも観測できない（CLAUDE.md の最頻の穴）。
#[tokio::test]
async fn an_upstream_that_names_another_model_is_reported_as_failed() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-wanted.gguf");
    let upstream_port = free_port();
    let port = free_port();

    let (profiles, _) = read(
        &serde_json::json!({
            "planted": {
                "main": {
                    "endpoint": format!("http://127.0.0.1:{upstream_port}"),
                    "model": "planted-body-model-id",
                    "gguf": gguf,
                    "args": ["--pc-answer", "planted-something-else"],
                }
            }
        })
        .to_string(),
    );
    let (config, plan) = config_of(&profiles, "planted", dir.path(), port);

    let mut serving = serve(config, plan);
    let launch = wait_for_state(port, "main", "failed").await;
    let reason = launch["reason"].as_str().unwrap_or_default();
    assert!(
        reason.contains("planted-something-else"),
        "何が載っていたのかを言うこと: {launch}"
    );
    assert!(
        reason.contains("planted-wanted.gguf"),
        "何を期待していたのかも言うこと: {launch}"
    );

    serving.stop().await;
    wait_until_gone(upstream_port, "起こした上流").await;
}

/// **gguf が無ければ、1つも起こさずに止まる**（ADR-0042 §決定2）。
///
/// **偽の `llama` は `PATH` に在るのに、一度も呼ばれない。**
/// **待ち受けも残らない**（`bind` が失敗として返る）。
#[tokio::test]
async fn a_missing_gguf_starts_nothing_and_stops_the_startup() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let marker = dir.path().join(TOUCH_MARKER);
    let upstream_port = free_port();
    let port = free_port();

    let (profiles, _) = read(
        &serde_json::json!({
            "planted": {
                "main": {
                    "endpoint": format!("http://127.0.0.1:{upstream_port}"),
                    "model": "planted-body-model-id",
                    "gguf": dir.path().join("never-here.gguf"),
                    "args": ["--pc-touch", marker],
                }
            }
        })
        .to_string(),
    );
    let (config, plan) = config_of(&profiles, "planted", dir.path(), port);

    let err = App::run_with_upstreams(&config, plan, bounded_shutdown())
        .await
        .expect_err("gguf が無ければ止まること");
    assert_eq!(
        err.to_string(),
        "名指しされた上流を起こせませんでした",
        "表の文は定数文字列であること"
    );
    assert!(
        !marker.exists(),
        "1つも起こさないはずが、偽の llama が呼ばれている"
    );
    assert!(!answers(port).await, "止まったのに待ち受けが残っている");
}

/// **待ち受けに失敗したら、子は1本も居ない**（ADR-0042 §決定3 の順序）。
///
/// # なぜこれが順序の見張り番なのか
///
/// **「起こす → 待ち受ける」の順にすると、この腕で子が残る** ——
/// **起こした後に `bind` が失敗すると、止める人がもう居ない。**
/// **偽の `llama` が一度も呼ばれないこと**が、順序そのものの観測である。
#[tokio::test]
async fn a_failed_bind_leaves_no_child_behind() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-never.gguf");
    let marker = dir.path().join(TOUCH_MARKER);
    let upstream_port = free_port();

    // **待ち受けたいポートを、先に塞ぐ。**
    let blocker = std::net::TcpListener::bind("127.0.0.1:0").expect("塞げること");
    let blocked = blocker.local_addr().expect("宛先を読めること").port();

    let (profiles, _) = read(
        &serde_json::json!({
            "planted": {
                "main": {
                    "endpoint": format!("http://127.0.0.1:{upstream_port}"),
                    "model": "planted-body-model-id",
                    "gguf": gguf,
                    "args": ["--pc-touch", marker],
                }
            }
        })
        .to_string(),
    );
    let (config, plan) = config_of(&profiles, "planted", dir.path(), blocked);

    let err = App::run_with_upstreams(&config, plan, bounded_shutdown())
        .await
        .expect_err("塞がったポートでは止まること");
    assert_eq!(
        err.to_string(),
        "待ち受けアドレスを開けませんでした",
        "止まった理由が待ち受けであること"
    );
    // **⚠ 印のファイルだけを見ない。** **それは競走である**（2026-09-19 に実測）——
    // 子が印を書くより先に `kill_on_drop` が殺すと、**起こしていても印は残らない。**
    // **実測: 順序を入れ替える変異（`m9`）が、印だけを見る形では 2153 全緑を通った。**
    //
    // **親の側の足跡を見る。** `Supervisor::start` は**起こす前に**
    // 出力の落とし先（`upstream-logs/`）を作ってファイルを開くので、
    // **そこに1件でも在れば、待ち受けより先に起こしに行っている。**
    // **これは同期で、競走しない。**
    let logs = dir.path().join("db").join("upstream-logs");
    assert!(
        !logs.exists(),
        "待ち受けに失敗したのに上流を起こしに行っている（止める人が居ない）: {logs:?}"
    );
    assert!(
        !marker.exists(),
        "待ち受けに失敗したのに上流を起こしている（止める人が居ない）"
    );
    assert!(!answers(upstream_port).await, "起こした上流が残っている");
    drop(blocker);
}

// ----------------------------------------------------------------------
// 待ち受けとの取り合い（作業単位 B-52 / Issue origin#21）
// ----------------------------------------------------------------------

/// **`Display` と `#[source]` の鎖を1つの綴りに畳む**（`tests/upstream_profiles.rs` と同じ形）。
///
/// **外向きの `Display` は定数文字列**なので、**何が駄目だったかは `#[source]` 側にしか無い。**
fn rendered_chain(err: &dyn std::error::Error) -> String {
    let mut rendered = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        rendered.push_str(" / ");
        rendered.push_str(&cause.to_string());
        source = cause.source();
    }
    rendered
}

/// **gguf を名指しした面が personacore 自身の待ち受けを取りに行ったら、
/// 1つも起こさずに止まる**（作業単位 B-52 / Issue origin#21）。
///
/// # なぜ起こす前なのか（利用者の実機 / 2026-09-19）
///
/// **起こしてしまうと、`llama` が 2 秒後に `exit code: 1` で終わる。**
/// 画面に出せるのは「失敗（exit code: 1）」だけで、
/// **本当の理由（ポートの取り合い）は `llama` の err ログの中にしか無い。**
/// **exe は両方の値を知っている**のだから、起こす前に言える。
///
/// # ⚠ 待ち受けは既定（8080）ではない
///
/// **`free_port()` はエフェメラルな範囲を返す。**
/// **比べる相手を `DEFAULT_PORT` に決め打った実装は、ここで衝突を見落とす**
/// ——偽の `llama` が起き、印が残る。
#[tokio::test]
async fn a_face_that_wants_the_servers_own_port_stops_before_anything_is_started() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-collides.gguf");
    let marker = dir.path().join(TOUCH_MARKER);
    // **personacore の待ち受けと、起こす面の宛先が同じポート。**
    let port = free_port();

    let (profiles, _) = read(
        &serde_json::json!({
            "planted": {
                "main": {
                    // **宛先のホストは `localhost`** —— 待ち受けの綴り（`127.0.0.1:…`）と
                    // 字面が重ならないようにする。同じ綴りだと、下の2つの `contains` が
                    // 部分文字列の関係になり、待ち受けの値を落とす実装でも両方通る
                    // （レビュー2周目が実測: `bind` を空にする変異が統合側では緑だった）。
                    // 判定はポートだけを見るので、`localhost` でも衝突と読まれる。
                    "endpoint": format!("http://localhost:{port}"),
                    "model": "planted-body-model-id",
                    "gguf": gguf,
                    "args": ["--pc-touch", marker],
                }
            }
        })
        .to_string(),
    );
    let (config, plan) = config_of(&profiles, "planted", dir.path(), port);

    let err = App::run_with_upstreams(&config, plan, bounded_shutdown())
        .await
        .expect_err("待ち受けと同じポートなら止まること");
    assert_eq!(
        err.to_string(),
        "名指しされた上流を起こせませんでした",
        "表の文は定数文字列であること"
    );

    // **両方の値を挙げる**（どちらを直せばよいかは、片方だけでは決まらない）。
    // 2つの探し語は互いに部分文字列ではない（`localhost:…` と `127.0.0.1:…`）。
    let chain = rendered_chain(&err);
    assert!(
        chain.contains(&format!("http://localhost:{port}")),
        "プロファイルの宛先を挙げていない: {chain}"
    );
    assert!(
        chain.contains(&format!("127.0.0.1:{port}")),
        "personacore 自身の待ち受けを挙げていない: {chain}"
    );

    // **1つも起こしていない。** 印のファイルと、親の側の足跡の両方を見る
    // （印だけでは競走になる / `a_failed_bind_leaves_no_child_behind` と同じ理由）。
    let logs = dir.path().join("db").join("upstream-logs");
    assert!(
        !logs.exists(),
        "止まったのに上流を起こしに行っている: {logs:?}"
    );
    assert!(!marker.exists(), "止まったのに偽の llama が呼ばれている");
    assert!(!answers(port).await, "止まったのに待ち受けが残っている");
}

/// **gguf の無い面（外の上流）は、同じポートを名乗っても縛らない。**
///
/// # なぜ縛らないのか
///
/// **そこに待ち受けを作ったのは personacore ではない。**
/// 起こさない面は**同居していない**ので、取り合いにならない
/// （縛ると「別の機に上流を置く」形が書けなくなる / ADR-0039 §結果）。
///
/// **対比の腕でもある** —— これが無いと、
/// **設定の4つの宛先を丸ごと縛る実装**（gguf の有無を見ない）が緑を通る。
#[tokio::test]
async fn an_external_face_may_name_the_servers_own_port() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-external-peer.gguf");
    let marker = dir.path().join(TOUCH_MARKER);
    let port = free_port();
    let upstream_port = free_port();

    let (profiles, _) = read(
        &serde_json::json!({
            "planted": {
                // **本文は外の上流。しかも personacore 自身と同じポートを名乗る。**
                "main": {
                    "endpoint": format!("http://127.0.0.1:{port}"),
                    "model": "planted-body-model-id",
                },
                // **抽出だけを起こす**（起こす面は別のポート）。
                "sub": {
                    "endpoint": format!("http://{LAUNCH_HOST}:{upstream_port}"),
                    "model": "planted-sub-model-id",
                    "gguf": gguf,
                    "args": ["--pc-touch", marker],
                }
            }
        })
        .to_string(),
    );
    let (config, plan) = config_of(&profiles, "planted", dir.path(), port);

    let mut serving = serve(config, plan);
    let launch = wait_for_state(port, "sub", "ready").await;
    assert_eq!(
        launch["answered_name"],
        serde_json::json!("planted-external-peer.gguf"),
        "起こした面が載っていない: {launch}"
    );
    assert!(marker.exists(), "偽の llama が一度も呼ばれていない");

    // **起こさない面は `external` のまま**（帰属）。
    let report = report_once(port).await;
    assert_eq!(
        report["main"]["launch"]["state"],
        serde_json::json!("external"),
        "起こしていない面まで動いている: {report}"
    );

    serving.stop().await;
    wait_until_gone_at(LAUNCH_HOST, upstream_port, "起こした上流").await;
}

// ----------------------------------------------------------------------
// 走ったまま差し替える（ADR-0042 §決定4 / 作業単位 F-4b-2 / Issue origin#2）
// ----------------------------------------------------------------------

/// **プロファイルを書いたファイルを置く**（差し替えのたびに読み直される）。
fn write_profiles(dir: &Path, source: &serde_json::Value) -> PathBuf {
    let path = dir.join("upstreams.json");
    std::fs::write(&path, source.to_string()).expect("プロファイルを置けること");
    path
}

/// **差し替えられる起動**（本番の `main` と同じく、置き場所を渡す）。
fn serve_switchable(config: AppConfig, plan: LaunchPlan, profiles: PathBuf) -> Serving {
    let (shutdown, wait) = oneshot::channel();
    let task = tokio::spawn(async move {
        App::run_with_upstreams(
            &config,
            Startup::new(plan).with_profiles_file(profiles),
            async move {
                let _ = wait.await;
            },
        )
        .await
    });
    Serving {
        shutdown: Some(shutdown),
        task: Some(task),
    }
}

/// **`POST /admin/upstream/profile`**。待ち受けが始まる前の数回は繋がらないので、上限つきで待つ。
async fn switch_to(port: u16, name: &str) -> (u16, serde_json::Value) {
    post_switch(port, &serde_json::json!({ "name": name })).await
}

async fn post_switch(port: u16, body: &serde_json::Value) -> (u16, serde_json::Value) {
    let url = format!("http://127.0.0.1:{port}/admin/upstream/profile");
    for _ in 0..MAX_WAITS {
        let Ok(response) = reqwest::Client::new()
            .post(&url)
            .json(body)
            .timeout(Duration::from_secs(30))
            .send()
            .await
        else {
            tokio::time::sleep(WAIT_STEP).await;
            continue;
        };
        let status = response.status().as_u16();
        let text = response.text().await.expect("本文を読めること");
        let json = serde_json::from_str(&text)
            .unwrap_or_else(|err| panic!("JSON として読めること: {err} / {text}"));
        return (status, json);
    }
    panic!("待ち受けを {MAX_WAITS} 回待っても繋がらなかった");
}

/// **面の状態が期待の綴りになり、しかも名乗りが `answered` になるまで待つ。**
///
/// **差し替えの前も後も `ready` なので、状態の綴りだけでは「差し替わった」と読めない**
/// （CLAUDE.md「A が B になるのを待つ検査は、A のままでは成立しない形にする」）。
async fn wait_for_ready_as(port: u16, face: &str, answered: &str) -> serde_json::Value {
    let mut last = serde_json::Value::Null;
    for _ in 0..MAX_WAITS {
        if let Some(report) = upstream_report(port).await {
            last = report[face]["launch"].clone();
            if last["state"] == serde_json::json!("ready")
                && last["answered_name"] == serde_json::json!(answered)
            {
                return report;
            }
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
    panic!("{face} が {MAX_WAITS} 回待っても {answered} で ready にならなかった（いま {last}）");
}

/// **その宛先の `/v1/models` が名乗る綴り**（本物の子が入れ替わったかを、子そのものに聞く）。
async fn models_named_at(host: &str, port: u16) -> String {
    reqwest::Client::new()
        .get(format!("http://{host}:{port}/v1/models"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .expect("答えること")
        .text()
        .await
        .expect("本文を読めること")
}

/// **本文の gguf だけを変えたプロファイルへ差し替えると、本文の子だけが起こし直され、
/// 抽出の子と観測は1バイトも触られない**（ADR-0042 §決定4 / §見張り番「片面だけの差し替え」）。
///
/// # 何を1本で見ているか
///
/// | 主張 | 破れると |
/// |---|---|
/// | 本文が新しい gguf で `ready` になる | 起こし直していない |
/// | **同じポートを取り直せる** | 前の子が残っている（止めていない） |
/// | 応答の時点で、本文の古い事実（前の窓・前の名乗り）が無い | 画面が前のモデルの窓を言い続ける |
/// | 本文の観測が新しい窓（33333）を持つ | 測り直していない |
/// | **抽出の子が起こし直されていない**（印が再び作られない） | 変わらない面まで触っている |
/// | 抽出の観測がそのまま（窓 22222・名乗り） | 同上 |
///
/// **2面の値を全部違える**（宛先・綴り・gguf・窓）——`assert_ne!` で防げない帰属の族。
///
/// # ⚠ 「同じポートを取り直せる」は「待った」の見張り番ではない
///
/// **`kill` は子を数ミリ秒以内に終わらせ、次の子が起きて bind するまではそれ以上かかる**ので、
/// **待たない実装でもここはたいてい緑**（変異検査で測った / 作業単位の報告）。
/// **待つ側の見張り番は `launch::tests::stopping_a_child_waits_until_it_has_exited`、
/// 「止めてから起こす」順の見張り番は `a_switch_stops_the_old_child_before_it_starts_the_next`。**
/// **ここが赤にするのは「前の子を止めない」形**（前の子がポートを掴んだまま残る）。
#[tokio::test]
async fn switching_restarts_only_the_face_whose_gguf_changed() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let main_a = fake_gguf(dir.path(), "planted-main-a.gguf");
    let main_b = fake_gguf(dir.path(), "planted-main-b.gguf");
    let sub_s = fake_gguf(dir.path(), "planted-sub-s.gguf");
    let sub_marker = dir.path().join("sub-started");
    let main_b_marker = dir.path().join("main-b-started");
    let main_port = free_port();
    let sub_port = free_port();
    let port = free_port();

    let sub = serde_json::json!({
        "endpoint": format!("http://127.0.0.1:{sub_port}"),
        "model": "planted-sub-model-id",
        "gguf": sub_s,
        "args": ["--pc-ctx", "22222", "--pc-touch", sub_marker],
    });
    let source = serde_json::json!({
        "a": {
            "main": {
                "endpoint": format!("http://{LAUNCH_HOST}:{main_port}"),
                "model": "planted-main-a-model-id",
                "gguf": main_a,
                "args": ["--pc-ctx", "11111"],
            },
            "sub": sub,
        },
        "b": {
            "main": {
                // **同じポート**——前の子が残っていれば、新しい子は待ち受けられない。
                "endpoint": format!("http://{LAUNCH_HOST}:{main_port}"),
                "model": "planted-main-b-model-id",
                "gguf": main_b,
                "args": ["--pc-ctx", "33333", "--pc-touch", main_b_marker],
            },
            "sub": sub,
        }
    });
    let profiles_path = write_profiles(dir.path(), &source);
    let (profiles, _) = read(&source.to_string());
    let (config, plan) = config_of(&profiles, "a", dir.path(), port);

    let mut serving = serve_switchable(config, plan, profiles_path);
    wait_for_ready_as(port, "main", "planted-main-a.gguf").await;
    wait_for_ready_as(port, "sub", "planted-sub-s.gguf").await;
    // **抽出の子が起こし直されたら、この印がまた作られる。**
    std::fs::remove_file(&sub_marker).expect("抽出の子の印を消せること");

    let (status, body) = switch_to(port, "b").await;
    assert_eq!(status, 200, "差し替えられること: {body}");

    // **応答の時点で、本文の古い事実は1つも残っていない**（ADR-0042 §決定4）。
    // 番人が既に測り直していれば新しい値、まだなら「聞いていない」——**前の値だけは出ない。**
    assert_eq!(
        body["main"]["model"],
        serde_json::json!("planted-main-b-model-id"),
        "依頼する綴りが差し替わっていない: {body}"
    );
    assert!(
        [serde_json::Value::Null, serde_json::json!(33333)]
            .contains(&body["main"]["queried_context_tokens"]),
        "前の子の窓が残っている: {body}"
    );
    assert!(
        [
            serde_json::Value::Null,
            serde_json::json!("planted-main-b.gguf")
        ]
        .contains(&body["main"]["answered_name"]),
        "前の子の名乗りが残っている: {body}"
    );

    let report = wait_for_ready_as(port, "main", "planted-main-b.gguf").await;
    assert!(main_b_marker.exists(), "新しい本文の子が呼ばれていない");
    assert!(
        models_named_at(LAUNCH_HOST, main_port)
            .await
            .contains("planted-main-b.gguf"),
        "同じポートで答えているのが新しい子ではない"
    );
    // **測り直した**（新しい子の窓を名乗る）。
    assert_eq!(
        report["main"]["queried_context_tokens"],
        serde_json::json!(33333),
        "本文を測り直していない: {report}"
    );

    // **抽出の面は1バイトも触っていない。**
    // **抽出の子そのものが、まだ答えること**（レビュー1周目の是正）——印と観測だけでは、
    // **残すはずの子まで止める実装**が緑を通る（止めても観測は書き替わらず、印も作られない）。
    assert!(
        models_named_at("127.0.0.1", sub_port)
            .await
            .contains("planted-sub-s.gguf"),
        "変わらない抽出の子が答えていない（止められている）"
    );
    assert!(
        !sub_marker.exists(),
        "変わらない抽出の子まで起こし直している"
    );
    assert_eq!(
        report["sub"]["launch"]["answered_name"],
        serde_json::json!("planted-sub-s.gguf"),
        "{report}"
    );
    assert_eq!(
        report["sub"]["answered_name"],
        serde_json::json!("planted-sub-s.gguf"),
        "{report}"
    );
    assert_eq!(
        report["sub"]["queried_context_tokens"],
        serde_json::json!(22222),
        "抽出の観測が書き替わっている: {report}"
    );
    assert_eq!(
        report["sub"]["model"],
        serde_json::json!("planted-sub-model-id"),
        "{report}"
    );

    serving.stop().await;
    wait_until_gone_at(LAUNCH_HOST, main_port, "起こし直した本文の子").await;
    wait_until_gone(sub_port, "抽出の子").await;
}

/// **本文と抽出の両方を受けられる外の上流**（`wiremock`）。名乗る綴りは `model` だけ。
async fn external_upstream(model: &str) -> wiremock::MockServer {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers};
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                serde_json::json!({
                    "id": "chatcmpl-1",
                    "object": "chat.completion",
                    "created": 1,
                    "model": model,
                    "choices": [{"index": 0,
                                 "message": {"role": "assistant", "content": "「おかえり」"},
                                 "finish_reason": "stop"}],
                })
                .to_string(),
                "application/json",
            ),
        )
        .mount(&server)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                serde_json::json!({"object": "list", "data": [{
                    "id": model, "object": "model", "type": "llm", "state": "loaded",
                    "max_context_length": 262_144, "loaded_context_length": 16_384,
                }]})
                .to_string(),
                "application/json",
            ),
        )
        .mount(&server)
        .await;
    server
}

/// その上流へ届いた本文生成の依頼（**照会は数えない**）。
async fn chats_to(server: &wiremock::MockServer) -> Vec<serde_json::Value> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .map(|request| serde_json::from_slice(&request.body).expect("JSON として読めること"))
        .collect()
}

/// その上流へ届いた照会の回数。
async fn probes_to(server: &wiremock::MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path().ends_with("/models"))
        .count()
}

/// **外の上流どうしの差し替えは、宛先と綴りを替えるだけ。次のターンは新しい上流へ行く**
/// （ADR-0042 §決定4「gguf の無い面」）。
///
/// **本当に行き先が変わったかは、上流に届いた依頼で見る**（応答の欄は差し替え口の中身を
/// 言うだけで、本文がそこを通ったかは言わない）。**抽出の面は触らない**——
/// その上流への照会の回数が、差し替えの前後で変わらないこと。
#[tokio::test]
async fn switching_an_external_face_sends_the_next_turn_to_the_new_upstream() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let before = external_upstream("planted-before-model").await;
    let after = external_upstream("planted-after-model").await;
    let sub = external_upstream("planted-sub-model").await;
    let port = free_port();

    let sub_face = serde_json::json!({ "endpoint": sub.uri(), "model": "planted-sub-model" });
    let source = serde_json::json!({
        "before": { "main": { "endpoint": before.uri(), "model": "planted-before-model" },
                    "sub": sub_face },
        "after":  { "main": { "endpoint": after.uri(), "model": "planted-after-model" },
                    "sub": sub_face },
    });
    let profiles_path = write_profiles(dir.path(), &source);
    let (profiles, _) = read(&source.to_string());
    let (config, plan) = config_of(&profiles, "before", dir.path(), port);
    let mut serving = serve_switchable(config, plan, profiles_path);
    report_once(port).await;

    let client = reqwest::Client::new();
    let created = client
        .post(format!("http://127.0.0.1:{port}/admin/characters"))
        .json(&serde_json::json!({ "handle": "aoi", "base_persona": "落ち着いた口調で話す。" }))
        .send()
        .await
        .expect("繋がること");
    assert!(created.status().is_success(), "{:?}", created.status());
    let chat = |message: &'static str| {
        let client = client.clone();
        async move {
            let response = client
                .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
                .json(&serde_json::json!({
                    "model": "aoi",
                    "messages": [{"role": "user", "content": message}],
                    "stream": false,
                }))
                .send()
                .await
                .expect("繋がること");
            assert!(response.status().is_success(), "{:?}", response.status());
        }
    };

    chat("ただいま").await;
    assert_eq!(chats_to(&before).await.len(), 1, "前の上流へ行っていない");
    let sub_probes = probes_to(&sub).await;

    let (status, body) = switch_to(port, "after").await;
    assert_eq!(status, 200, "差し替えられること: {body}");
    assert!(
        body["main"]["endpoint"]
            .as_str()
            .is_some_and(|endpoint| endpoint.starts_with(&after.uri())),
        "宛先が差し替わっていない: {body}"
    );
    assert_eq!(
        body["main"]["model"],
        serde_json::json!("planted-after-model")
    );
    // **その場で測り直した**（新しい上流が名乗った綴り）。
    assert_eq!(
        body["main"]["answered_name"],
        serde_json::json!("planted-after-model"),
        "{body}"
    );
    assert_eq!(
        body["main"]["launch"]["state"],
        serde_json::json!("external")
    );
    // **抽出の面は触っていない。**
    assert!(
        body["sub"]["endpoint"]
            .as_str()
            .is_some_and(|endpoint| endpoint.starts_with(&sub.uri())),
        "{body}"
    );
    assert_eq!(body["sub"]["model"], serde_json::json!("planted-sub-model"));
    assert_eq!(
        probes_to(&sub).await,
        sub_probes,
        "変わらない抽出の面まで測り直している"
    );

    chat("もう一度").await;
    let arrived = chats_to(&after).await;
    assert_eq!(arrived.len(), 1, "次のターンが新しい上流へ行っていない");
    assert_eq!(
        arrived[0]["model"],
        serde_json::json!("planted-after-model"),
        "新しい綴りで依頼していない"
    );
    assert_eq!(
        chats_to(&before).await.len(),
        1,
        "差し替えた後も前の上流へ行っている"
    );

    serving.stop().await;
}

/// **差し替えは同時に1つだけ。進行中に次が来たら 409**（ADR-0042 §決定4「列に並べない」）。
///
/// # 競走しない作り方
///
/// **1つ目の差し替えを、答えない上流の照会で止めておく。**
/// **その上流が接続を受けた時点で、1つ目は錠を持っている**（照会は錠の中）——
/// そこで2つ目を投げる。**受けた接続を切れば、1つ目は終わる。**
///
/// **対照**: 1つ目が終わった後の同じ依頼は通る（「常に 409」を赤にする）。
/// **断った依頼は何も変えていない**——1つ目が終わった後、本文は1つ目の宛先を向いている。
#[tokio::test]
async fn a_second_switch_while_one_is_running_is_refused() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let port = free_port();
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("答えない上流を立てられること");
    let silent_port = silent.local_addr().expect("宛先を読めること").port();
    let closed = free_port();
    let other = free_port();

    let source = serde_json::json!({
        "base":   { "main": { "endpoint": format!("http://127.0.0.1:{closed}"), "model": "planted-base" } },
        "silent": { "main": { "endpoint": format!("http://127.0.0.1:{silent_port}"), "model": "planted-silent" } },
        "other":  { "main": { "endpoint": format!("http://127.0.0.1:{other}"), "model": "planted-other" } },
    });
    let profiles_path = write_profiles(dir.path(), &source);
    let (profiles, _) = read(&source.to_string());
    let (config, plan) = config_of(&profiles, "base", dir.path(), port);
    let mut serving = serve_switchable(config, plan, profiles_path);
    report_once(port).await;

    let first = tokio::spawn(switch_to(port, "silent"));
    // **1つ目の照会が届いた＝1つ目は錠を持っている。**
    let (held, _) = tokio::time::timeout(Duration::from_secs(30), silent.accept())
        .await
        .expect("1つ目の差し替えが照会に来ること")
        .expect("受けられること");

    let (status, body) = switch_to(port, "other").await;
    assert_eq!(status, 409, "進行中なのに断っていない: {body}");
    assert_eq!(
        body["error"]["code"],
        serde_json::json!("upstream_switch_in_progress"),
        "{body}"
    );

    // **受けた接続を切り、残りの照会も受けては切る**（1つ目を終わらせる）。
    drop(held);
    let drain = tokio::spawn(async move {
        while let Ok((stream, _)) = silent.accept().await {
            drop(stream);
        }
    });
    let (status, body) = tokio::time::timeout(Duration::from_secs(30), first)
        .await
        .expect("1つ目が終わること")
        .expect("タスクが落ちていないこと");
    assert_eq!(status, 200, "1つ目は差し替えられること: {body}");
    assert!(
        body["main"]["endpoint"]
            .as_str()
            .is_some_and(|endpoint| endpoint.contains(&format!(":{silent_port}/"))),
        "断った依頼が何かを変えている: {body}"
    );
    drain.abort();

    // **対照: 終わった後なら通る。**
    let (status, body) = switch_to(port, "other").await;
    assert_eq!(status, 200, "錠が解けていない: {body}");
    assert!(
        body["main"]["endpoint"]
            .as_str()
            .is_some_and(|endpoint| endpoint.contains(&format!(":{other}/"))),
        "{body}"
    );

    serving.stop().await;
}

/// **断る差し替えは、何も止めず、何も起こさない**（ADR-0042 §決定2「起こす前に確かめる」）。
///
/// | 依頼 | 応答 |
/// |---|---|
/// | 無い名前 | 404 `upstream_profile_not_found` |
/// | gguf が無いプロファイル | 422 `upstream_gguf_missing` |
/// | personacore 自身の待ち受けと同じポートを取りに行くプロファイル | 422 `upstream_port_collision` |
/// | 本文が JSON でない | 400 |
///
/// **どの後でも、走っている子は同じ子のまま**（印が再び作られない・同じ名前で答える）。
/// **断った側の偽物は一度も呼ばれない。**
#[tokio::test]
async fn a_refused_switch_stops_nothing_and_starts_nothing() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let running = fake_gguf(dir.path(), "planted-running.gguf");
    let spare = fake_gguf(dir.path(), "planted-spare.gguf");
    let running_marker = dir.path().join("running-started");
    let refused_marker = dir.path().join("refused-started");
    let upstream_port = free_port();
    let spare_port = free_port();
    let port = free_port();

    let source = serde_json::json!({
        "running": { "main": {
            "endpoint": format!("http://127.0.0.1:{upstream_port}"),
            "model": "planted-running-id", "gguf": running,
            "args": ["--pc-touch", running_marker] } },
        "missing": { "main": {
            "endpoint": format!("http://127.0.0.1:{spare_port}"),
            "model": "planted-missing-id", "gguf": dir.path().join("never-here.gguf"),
            "args": ["--pc-touch", refused_marker] } },
        "collide": { "main": {
            // **ホストの綴りは `localhost`**——待ち受けの綴りと字面を重ねない。
            "endpoint": format!("http://localhost:{port}"),
            "model": "planted-collide-id", "gguf": spare,
            "args": ["--pc-touch", refused_marker] } },
    });
    let profiles_path = write_profiles(dir.path(), &source);
    let (profiles, _) = read(&source.to_string());
    let (config, plan) = config_of(&profiles, "running", dir.path(), port);
    let mut serving = serve_switchable(config, plan, profiles_path);
    wait_for_ready_as(port, "main", "planted-running.gguf").await;
    std::fs::remove_file(&running_marker).expect("印を消せること");

    for (body, want_status, want_code) in [
        (
            serde_json::json!({ "name": "never-written" }),
            404,
            Some("upstream_profile_not_found"),
        ),
        (
            serde_json::json!({ "name": "missing" }),
            422,
            Some("upstream_gguf_missing"),
        ),
        (
            serde_json::json!({ "name": "collide" }),
            422,
            Some("upstream_port_collision"),
        ),
        (serde_json::json!({ "profile": "missing" }), 400, None),
    ] {
        let (status, answer) = post_switch(port, &body).await;
        assert_eq!(status, want_status, "{body} → {answer}");
        if let Some(code) = want_code {
            assert_eq!(answer["error"]["code"], serde_json::json!(code), "{body}");
        }
        // **応答に置き場所を載せない**（絶対制約）。
        assert!(
            !answer.to_string().contains("never-here"),
            "応答に gguf の置き場所が載っている: {answer}"
        );
    }

    assert!(
        !refused_marker.exists(),
        "断った差し替えが偽物を起こしている"
    );
    assert!(
        !running_marker.exists(),
        "断った差し替えが、走っている子を起こし直している"
    );
    let report = report_once(port).await;
    assert_eq!(
        report["main"]["launch"]["state"],
        serde_json::json!("ready"),
        "{report}"
    );
    assert!(
        models_named_at("127.0.0.1", upstream_port)
            .await
            .contains("planted-running.gguf"),
        "走っていた子が答えていない"
    );

    serving.stop().await;
    wait_until_gone(upstream_port, "起こした上流").await;
}

/// **`--host` にワイルドカードを渡すプロファイルは、起こす前に止まる**
/// （作業単位 F-4b-2 / Issue origin#2 の持ち越し3）。
///
/// **偽物が一度も呼ばれない**ことで固定する。**この機にワイルドカードの待ち受けは作らない**
/// ——検証を外す変異で偽物が呼ばれても、偽物はワイルドカードでは待ち受けずに終わる
/// （`tests/support/fake_llama.rs` の `wildcard_host`）。
///
/// | 腕 | 止めるもの |
/// |---|---|
/// | `args` に `--host 0.0.0.0` | `upstreams` の `reserved_serve_flag` |
/// | `args` に `--host=0.0.0.0` | 同上（`=` の形） |
/// | `endpoint` が `0.0.0.0` | `upstreams` のループバックの検証（F-4b-1 から在る） |
/// | `endpoint` が `[::]` | 同上（IPv6） |
///
/// **1つのファイルに1つずつ書く**——プロファイルのファイルは1つでも不正なら丸ごと読めない
/// （F-9 の作法）ので、同居させると最初の1つだけを見ることになる。
#[tokio::test]
async fn a_wildcard_host_never_reaches_the_child() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-wild.gguf");
    let marker = dir.path().join(TOUCH_MARKER);
    let port = free_port();
    let upstream_port = free_port();

    let start = serde_json::json!({
        "start": { "main": { "endpoint": format!("http://127.0.0.1:{}", free_port()),
                             "model": "planted-start" } }
    });
    let profiles_path = write_profiles(dir.path(), &start);
    let (profiles, _) = read(&start.to_string());
    let (config, plan) = config_of(&profiles, "start", dir.path(), port);
    let mut serving = serve_switchable(config, plan, profiles_path.clone());
    report_once(port).await;

    let loopback = format!("http://127.0.0.1:{upstream_port}");
    for (endpoint, args) in [
        (
            loopback.clone(),
            serde_json::json!(["--host", "0.0.0.0", "--pc-touch", marker]),
        ),
        (
            loopback.clone(),
            serde_json::json!(["--host=0.0.0.0", "--pc-touch", marker]),
        ),
        (
            format!("http://0.0.0.0:{upstream_port}"),
            serde_json::json!(["--pc-touch", marker]),
        ),
        (
            format!("http://[::]:{upstream_port}"),
            serde_json::json!(["--pc-touch", marker]),
        ),
    ] {
        let wild = serde_json::json!({
            "wild": { "main": { "endpoint": endpoint, "model": "planted-wild",
                                "gguf": gguf, "args": args } }
        });
        std::fs::write(&profiles_path, wild.to_string()).expect("書き換えられること");

        let (status, answer) = switch_to(port, "wild").await;
        assert_eq!(status, 422, "{endpoint} / {args} → {answer}");
        assert_eq!(
            answer["error"]["code"],
            serde_json::json!("upstream_profiles_unreadable"),
            "{endpoint} / {args}"
        );
        assert!(
            !marker.exists(),
            "ワイルドカードを渡すプロファイルで偽物が呼ばれた（{endpoint} / {args}）"
        );
    }

    serving.stop().await;
}

/// **外の上流で起動してから gguf のプロファイルへ差し替えると子が起き、
/// 外の上流へ戻すと、その子は止まる**（ADR-0042 §決定2「起こしたものは止める」/ §決定4）。
///
/// **`--upstream` 無しの起動でも差し替えられる**ことの腕でもある（空の束を持つ）。
#[tokio::test]
async fn switching_to_a_gguf_and_back_starts_and_stops_the_child() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-switched-in.gguf");
    let upstream_port = free_port();
    let port = free_port();
    let lm = free_port();

    let source = serde_json::json!({
        "lm":   { "main": { "endpoint": format!("http://127.0.0.1:{lm}"), "model": "planted-lm" } },
        "gguf": { "main": { "endpoint": format!("http://{LAUNCH_HOST}:{upstream_port}"),
                            "model": "planted-gguf-id", "gguf": gguf } },
    });
    let profiles_path = write_profiles(dir.path(), &source);
    let (profiles, _) = read(&source.to_string());
    let (config, plan) = config_of(&profiles, "lm", dir.path(), port);
    assert!(plan.is_empty(), "前提: 外の上流だけで起動すること");
    let mut serving = serve_switchable(config, plan, profiles_path);
    report_once(port).await;

    let (status, body) = switch_to(port, "gguf").await;
    assert_eq!(status, 200, "{body}");
    wait_for_ready_as(port, "main", "planted-switched-in.gguf").await;
    // **`sub` を書いていないので、抽出も同じ子が担う**（ADR-0039 §決定1）。
    wait_for_ready_as(port, "sub", "planted-switched-in.gguf").await;

    let (status, body) = switch_to(port, "lm").await;
    assert_eq!(status, 200, "{body}");
    for face in ["main", "sub"] {
        assert_eq!(
            body[face]["launch"]["state"],
            serde_json::json!("external"),
            "{face}: {body}"
        );
        assert!(
            body[face]["endpoint"]
                .as_str()
                .is_some_and(|endpoint| endpoint.contains(&format!(":{lm}/"))),
            "{face}: {body}"
        );
        // **前の子の事実を残さない。**
        assert_ne!(
            body[face]["answered_name"],
            serde_json::json!("planted-switched-in.gguf"),
            "{face}: {body}"
        );
    }
    // **外の上流へ戻したら、起こした子は止まっている**（応答の時点で。待たずに見る）。
    assert!(
        !answers_at(LAUNCH_HOST, upstream_port).await,
        "外の上流へ戻したのに、起こした子が残っている"
    );

    serving.stop().await;
}

/// **置き場所を渡されていない起動は、差し替えを断る**（409 `upstream_switch_unavailable`）。
///
/// **`GET` は受けない**（405）——差し替えの入口は `POST` だけ。
#[tokio::test]
async fn a_start_without_a_profiles_file_refuses_to_switch() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let port = free_port();
    let (profiles, _) = read(
        r#"{ "planted": { "main": { "endpoint": "http://127.0.0.1:1234",
                                    "model": "planted-body-model-id" } } }"#,
    );
    let (config, plan) = config_of(&profiles, "planted", dir.path(), port);
    let mut serving = serve(config, plan);
    report_once(port).await;

    let (status, body) = switch_to(port, "planted").await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(
        body["error"]["code"],
        serde_json::json!("upstream_switch_unavailable"),
        "{body}"
    );

    let status = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/admin/upstream/profile"))
        .send()
        .await
        .expect("繋がること")
        .status();
    assert_eq!(status.as_u16(), 405);

    serving.stop().await;
}

/// **差し替えは「前の子を止める → 終了を見届ける → 次の子を起こす」の順**
/// （ADR-0042 §決定2・§決定4 / 作業単位 F-4b-2）。
///
/// # なぜログの順で見るのか（**競走しない**）
///
/// **「起こしてから止める」順の実装は、同じポートを取り直す腕では赤にならない**——
/// `kill` は数ミリ秒で効き、次の子が起きて bind するまでにはそれ以上かかるので、
/// **たいてい間に合ってしまう**（変異検査で測った / 作業単位の報告）。
/// **この実行ファイル自身が出す3行の並び**（止めます → 終了しました → 起こします）は、
/// 1つのタスクの中の順そのものなので、**時間に左右されない。**
///
/// # 行の帰属（**同じ行の中で見る**）
///
/// **グローバルのログには、同じバイナリの他のテストの行も混ざる。**
/// **前の子は宛先（このテストだけのポート）で、終了の行はその行が名乗った PID で、
/// 次の子は宛先で**引き当てる。
///
/// **⚠ 「終了を待ったか」は、この腕では見ていない**——待たない実装も「終了しました」を
/// 同じ順で書ける。**待つ側の見張り番は `launch::tests::stopping_a_child_waits_until_it_has_exited`。**
#[tokio::test]
async fn a_switch_stops_the_old_child_before_it_starts_the_next() {
    let log = install_global_log(DEFAULT_LOG_FILTER);
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let old = fake_gguf(dir.path(), "planted-order-old.gguf");
    let new = fake_gguf(dir.path(), "planted-order-new.gguf");
    let old_port = free_port();
    let new_port = free_port();
    let port = free_port();

    let old_endpoint = format!("http://{LAUNCH_HOST}:{old_port}");
    let new_endpoint = format!("http://{LAUNCH_HOST}:{new_port}");
    let source = serde_json::json!({
        "old": { "main": { "endpoint": old_endpoint, "model": "planted-order-old-id", "gguf": old } },
        "new": { "main": { "endpoint": new_endpoint, "model": "planted-order-new-id", "gguf": new } },
    });
    let profiles_path = write_profiles(dir.path(), &source);
    let (profiles, _) = read(&source.to_string());
    let (config, plan) = config_of(&profiles, "old", dir.path(), port);
    let mut serving = serve_switchable(config, plan, profiles_path);
    wait_for_ready_as(port, "main", "planted-order-old.gguf").await;

    let mark = log.mark();
    let (status, body) = switch_to(port, "new").await;
    assert_eq!(status, 200, "{body}");
    let section = log.contents_since(mark);
    let lines: Vec<&str> = section.lines().collect();

    let stopping = lines
        .iter()
        .position(|line| {
            line.contains("起こした上流を止めます")
                && line.contains(&format!("upstream_endpoint={old_endpoint}"))
        })
        .unwrap_or_else(|| panic!("前の子を止めた行が無い: {section}"));
    let pid = lines[stopping]
        .split_whitespace()
        .find(|token| token.starts_with("upstream_pid="))
        .unwrap_or_else(|| panic!("止めた行が PID を名乗っていない: {}", lines[stopping]));
    let exited = lines
        .iter()
        .position(|line| line.contains("起こした上流が終了しました") && line.contains(pid))
        .unwrap_or_else(|| panic!("前の子（{pid}）の終了を見届けた行が無い: {section}"));
    let starting = lines
        .iter()
        .position(|line| {
            line.contains("上流を起こします")
                && line.contains(&format!("upstream_endpoint={new_endpoint}"))
        })
        .unwrap_or_else(|| panic!("次の子を起こした行が無い: {section}"));

    assert!(
        stopping < exited && exited < starting,
        "順が違う（止める {stopping} → 終了 {exited} → 起こす {starting} であること）: {section}"
    );

    wait_for_ready_as(port, "main", "planted-order-new.gguf").await;
    serving.stop().await;
    wait_until_gone_at(LAUNCH_HOST, new_port, "次の子").await;
}

/// **差し替えで子を起こせなかった面は `failed` になり、応答に `command` の綴りを載せない**
/// （レビュー1周目の是正 / `launch::Supervisor::start_more`）。
///
/// **前の子は既に止めてある**ので、面に残るのは「起こせなかった」という事実だけ。
/// **`failed` にしないと、前の子の `ready` と名乗りが応答に残る**（止まった子を「載っている」と言う）。
///
/// **`command` の綴りは絶対パスでありうる**ので、理由は定数（絶対制約）。
/// **`sub` を書かないプロファイル**なので、1つの子が2面を担う——**両面とも `failed`**。
#[tokio::test]
async fn a_child_that_cannot_be_started_on_a_switch_marks_its_faces_failed() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let first = fake_gguf(dir.path(), "planted-unstartable-first.gguf");
    let next = fake_gguf(dir.path(), "planted-unstartable-next.gguf");
    let first_port = free_port();
    let next_port = free_port();
    let port = free_port();
    let command = dir.path().join("personacore-no-such-command");

    let source = serde_json::json!({
        "first": { "main": { "endpoint": format!("http://{LAUNCH_HOST}:{first_port}"),
                             "model": "planted-unstartable-first-id", "gguf": first } },
        "broken": { "main": { "endpoint": format!("http://127.0.0.1:{next_port}"),
                              "model": "planted-unstartable-next-id", "gguf": next,
                              "command": command } },
    });
    let profiles_path = write_profiles(dir.path(), &source);
    let (profiles, _) = read(&source.to_string());
    let (config, plan) = config_of(&profiles, "first", dir.path(), port);
    let mut serving = serve_switchable(config, plan, profiles_path);
    wait_for_ready_as(port, "main", "planted-unstartable-first.gguf").await;

    let (status, body) = switch_to(port, "broken").await;
    assert_eq!(status, 200, "{body}");
    for face in ["main", "sub"] {
        assert_eq!(
            body[face]["launch"]["state"],
            serde_json::json!("failed"),
            "起こせなかった面が failed になっていない（{face}）: {body}"
        );
        assert_ne!(
            body[face]["launch"]["answered_name"],
            serde_json::json!("planted-unstartable-first.gguf"),
            "止めた前の子の名乗りが残っている（{face}）: {body}"
        );
        // **起こせなかった面にはログの名前を付けない**（作業単位 F-4d のレビュー2周目）。
        // 利用者向け README（`scripts/README-dist.md` の「うまくいかないとき」）は
        // 「ログのファイル名が付いていなければ、起こすこと自体ができていない」と案内している。
        // **名前を付けると、その案内が別の事実に読める**（開いても何も無いログを探させる）。
        assert_eq!(
            body[face]["launch"]["log"],
            serde_json::Value::Null,
            "起こせなかった面にログの名前が付いている（{face}）: {body}"
        );
    }
    let rendered = body.to_string();
    assert!(
        !rendered.contains("personacore-no-such-command"),
        "応答に command の綴りが載っている: {body}"
    );
    // **前の子は止まっている**（起こせなかったからといって戻さない）。
    assert!(
        !answers_at(LAUNCH_HOST, first_port).await,
        "起こせなかったのに、前の子が残っている"
    );

    serving.stop().await;
}

// ----------------------------------------------------------------------
// 起こすのを待つ起動（ADR-0055 §決定1・3・4 / 作業単位 F-10a / Issue origin#82）
// ----------------------------------------------------------------------
//
// **`main` が覚えた名前で起きるときの形**（`LaunchTiming::WhenAsked`）。**実行ファイルを起こさず**、
// `main` と同じ `Startup` を組んで `App::run_with_upstreams` に渡す——止めるところまで見るため
// （このファイルの doc）。**覚えた名前から `WhenAsked` を選ぶ1行は `main`（bin）にしか無い**ので、
// そちらは `tests/upstream_cli.rs` の F-10a の節が実行ファイルで見る。
//
// **「起きる」側の対照は `the_named_gguf_is_started_reaches_ready_and_is_stopped_with_the_server`**
// （同じ形の計画を `WhenAsked` 無しで渡すと、待たずに起こして `ready` まで行く）。

/// **起こすのを待つ起動**（`main` が覚えた名前で起きるときと同じ `Startup`）。
fn serve_waiting(config: AppConfig, plan: LaunchPlan, profiles: PathBuf) -> Serving {
    let (shutdown, wait) = oneshot::channel();
    let task = tokio::spawn(async move {
        App::run_with_upstreams(
            &config,
            Startup::new(plan)
                .with_profiles_file(profiles)
                .launch_timing(LaunchTiming::WhenAsked),
            async move {
                let _ = wait.await;
            },
        )
        .await
    });
    Serving {
        shutdown: Some(shutdown),
        task: Some(task),
    }
}

/// **起こすのを待つ起動では、gguf の面は `not_started` のまま何も起こさず、
/// 同じ名前を頼まれて初めて起こす**（ADR-0055 §決定1・3・4）。
///
/// # 何を1本で見ているか
///
/// | 主張 | 破れると |
/// |---|---|
/// | 本文の面が `not_started`（**待ってから読む**。`loading` や `ready` のままでは成立しない） | 起動時に起こしている／状態を言っていない |
/// | **偽の `llama` が一度も呼ばれず、宛先に誰も居ない** | 状態だけ変えて起こしている |
/// | **`upstream-logs/` が無い**（起こした後には在る＝対照） | 起こさないのに親の足跡を作っている |
/// | 宛先・モデルはプロファイルのもの | 中身まで読み込まなくなった（§決定1「起こさないだけ」） |
/// | 反対の面（外の上流）は `external` | 帰属 |
/// | **OpenAI 互換の口は 503 `upstream_unavailable`**（載せている間と同じ形） | サーバの振る舞いを隠した／別の形で断った（§決定4） |
/// | **同じ名前の `POST` が 200 で `loading` を返し、そのあと `ready` になる** | 同じ名前で起こさない（§決定3） |
/// | **起こした後に印が在る** | 対照——印の検査が空振りしていないこと |
/// | 測り直した（子が名乗った窓） | 起こした面を測り直していない |
///
/// **`--pc-loading 3`** で、起きてから 3 秒は 503 を返させる——`POST` の応答の `loading` を
/// 競走にしないため（番人が先に `ready` を書く前に応答が返る）。
///
/// # ⚠ 見ていないもの
///
/// **「待っている面へ起動時に聞かない」は見ていない。** 宛先に誰も居ないので、聞いても聞かなくても
/// `answered_by` は `null` になる（区別できない）。聞かないことは `LaunchPlan::launches_face` の判定で決まり、
/// その判定は `the_face_we_are_about_to_start_is_not_asked_at_startup` が見ている（`LaunchTiming` では分けていない）。
#[tokio::test]
async fn a_waiting_start_launches_nothing_until_the_same_name_is_asked() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-waiting-main.gguf");
    let marker = dir.path().join(TOUCH_MARKER);
    let upstream_port = free_port();
    let port = free_port();

    let source = serde_json::json!({
        "planted": {
            "main": {
                "endpoint": format!("http://{LAUNCH_HOST}:{upstream_port}"),
                "model": "planted-waiting-model-id",
                "gguf": gguf,
                "args": ["--pc-touch", marker, "--pc-loading", "3", "--pc-ctx", "24680"],
            },
            "sub": {
                "endpoint": format!("http://127.0.0.1:{}", free_port()),
                "model": "planted-waiting-sub-id",
            }
        }
    });
    let profiles_path = write_profiles(dir.path(), &source);
    let (profiles, _) = read(&source.to_string());
    let (config, plan) = config_of(&profiles, "planted", dir.path(), port);
    assert!(
        plan.launches_face(personacore::measured::ModelFace::Main),
        "前提: 本文の面は gguf を名指ししていること"
    );

    let mut serving = serve_waiting(config, plan, profiles_path);

    // **起こしていない。**
    let launch = wait_for_state(port, "main", "not_started").await;
    for field in ["since", "answered_name", "reason", "log"] {
        assert_eq!(
            launch[field],
            serde_json::Value::Null,
            "not_started の面が {field} を持っている: {launch}"
        );
    }
    let report = report_once(port).await;
    assert_eq!(
        report["main"]["endpoint"],
        serde_json::json!(format!(
            "http://{LAUNCH_HOST}:{upstream_port}/v1/chat/completions"
        )),
        "プロファイルの宛先を読み込んでいない: {report}"
    );
    assert_eq!(
        report["main"]["model"],
        serde_json::json!("planted-waiting-model-id"),
        "{report}"
    );
    assert_eq!(
        report["sub"]["launch"]["state"],
        serde_json::json!("external"),
        "外の上流の面まで not_started になっている: {report}"
    );
    assert!(
        !marker.exists(),
        "起こすのを待つ起動が、偽の llama を呼んでいる"
    );
    assert!(
        !answers_at(LAUNCH_HOST, upstream_port).await,
        "起こしていないはずの宛先に誰か居る"
    );
    // **親の足跡（`upstream-logs/`）も作らない**（F-10a のレビュー是正）。
    // **対照は起こした後**（下）——同じ置き場所にできることを見て、この「無い」が空振りでないことを押さえる。
    let logs = dir.path().join("db").join("upstream-logs");
    assert!(
        !logs.exists(),
        "起こすのを待つ起動が、上流のログの置き場所を作っている: {logs:?}"
    );

    // **OpenAI 互換の口は、載せている間と同じ上流エラーの形で返る**（§決定4）。
    let client = reqwest::Client::new();
    let created = client
        .post(format!("http://127.0.0.1:{port}/admin/characters"))
        .json(&serde_json::json!({ "handle": "aoi", "base_persona": "落ち着いた口調で話す。" }))
        .send()
        .await
        .expect("繋がること");
    assert!(created.status().is_success(), "{:?}", created.status());
    let chat = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "ただいま"}],
            "stream": false,
        }))
        .send()
        .await
        .expect("繋がること");
    assert_eq!(chat.status().as_u16(), 503, "上流エラーの形で返っていない");
    let chat: serde_json::Value = chat.json().await.expect("JSON として読めること");
    assert_eq!(
        chat["error"]["code"],
        serde_json::json!("upstream_unavailable"),
        "{chat}"
    );
    assert!(!marker.exists(), "会話の依頼で上流を起こしている");

    // **同じ名前を頼むと、その面を起こす**（§決定3）。
    let (status, body) = switch_to(port, "planted").await;
    assert_eq!(status, 200, "同じ名前で起こせること: {body}");
    assert_eq!(
        body["main"]["launch"]["state"],
        serde_json::json!("loading"),
        "同じ名前の依頼で起こしていない: {body}"
    );
    assert_eq!(
        body["sub"]["launch"]["state"],
        serde_json::json!("external"),
        "{body}"
    );

    let report = wait_for_ready_as(port, "main", "planted-waiting-main.gguf").await;
    assert!(
        marker.exists(),
        "ready なのに偽の llama の印が無い（印の検査が空振りしている）"
    );
    assert!(
        std::fs::read_dir(&logs)
            .map(|entries| entries.count() > 0)
            .unwrap_or(false),
        "起こしたのに出力の落とし先が空（足跡の検査が空振りしている）: {logs:?}"
    );
    assert_eq!(
        report["main"]["queried_context_tokens"],
        serde_json::json!(24680),
        "起こした面を測り直していない: {report}"
    );

    serving.stop().await;
    wait_until_gone_at(LAUNCH_HOST, upstream_port, "起こした上流").await;
}

/// **起こすのを待っていた面は、それを起こさないプロファイルへ切り替えたら `external` になる**
/// （ADR-0055 / 作業単位 F-10a）。
///
/// **宛先と依頼する綴りが同じ外の上流へ切り替える**のが要点——新旧を比べるだけの実装では、
/// その面は「触らない」になり、**起こさない面が `not_started` を言い続ける**（押しても何も起きないボタンになる）。
/// **起動時に聞いていない面なので、切り替えでその場で1回測る**（外の上流を名乗る `wiremock`）。
#[tokio::test]
async fn a_waiting_face_switched_to_a_profile_that_does_not_launch_it_becomes_external() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-never-started.gguf");
    let marker = dir.path().join(TOUCH_MARKER);
    let external = external_upstream("planted-shared-model-id").await;
    let port = free_port();

    let source = serde_json::json!({
        "launching": { "main": { "endpoint": external.uri(), "model": "planted-shared-model-id",
                                 "gguf": gguf, "args": ["--pc-touch", marker] } },
        "bare": { "main": { "endpoint": external.uri(), "model": "planted-shared-model-id" } },
    });
    let profiles_path = write_profiles(dir.path(), &source);
    let (profiles, _) = read(&source.to_string());
    let (config, plan) = config_of(&profiles, "launching", dir.path(), port);
    let mut serving = serve_waiting(config, plan, profiles_path);
    wait_for_state(port, "main", "not_started").await;

    let (status, body) = switch_to(port, "bare").await;
    assert_eq!(status, 200, "{body}");
    for face in ["main", "sub"] {
        assert_eq!(
            body[face]["launch"]["state"],
            serde_json::json!("external"),
            "起こさないプロファイルへ切り替えたのに not_started のまま（{face}）: {body}"
        );
    }
    assert_eq!(
        body["main"]["answered_name"],
        serde_json::json!("planted-shared-model-id"),
        "外の上流になった面を測っていない: {body}"
    );
    assert!(
        !marker.exists(),
        "起こさないプロファイルなのに偽の llama を呼んでいる"
    );

    serving.stop().await;
}

// ----------------------------------------------------------------------
// 切り替えの依頼が、止める段の途中で切られる（作業単位 B-56 / Issue origin#39）
// ----------------------------------------------------------------------

/// **止める段に入ったことを名乗る行**（`launch::stop_started`）。**`kill` の手前で出る。**
const STOPPING_LINE: &str = "起こした上流を止めます";

/// **「止めます」の行が出た瞬間に、その場で依頼の接続を切る層**（作業単位 B-56）。
///
/// # なぜログの行で切るのか（**競走しない**）
///
/// **止める段（`kill` → 終了を待つ）は短い**（`kill` は子を数ミリ秒で終わらせる /
/// `switching_restarts_only_the_face_whose_gguf_changed` の doc）ので、テストの側で
/// 「ログを読んでから切る」と、**差し替えが終わった後に切っていない保証が無い**
/// ——そのとき「古い ready が残らない」は、**止める段で一度も切っていなくても成立する**
/// （CLAUDE.md「A が B になるのを待つ検査は、A のままでは成立しない形にする」）。
///
/// **この層は、その行を書いている呼び出しの中で（＝サーバのタスクの中で、`kill` より前に）
/// ソケットを閉じる。** だから**切ったのは必ず止める段の途中**で、
/// 切れたかどうかは [`Self::cut`] が言う。
///
/// **⚠ スレッドローカルに差す**（`tracing::subscriber::set_default`）。
/// `#[tokio::test]` は current_thread なので、**サーバのタスクも同じスレッドで走る**。
/// 同じバイナリの他のテストのスレッドには効かない。
struct CutWhenStopping {
    /// 止める子の宛先（**このテストだけのポート**）。他のテストの行では切らない。
    endpoint: String,
    /// 切る接続。**切ったら `None`**（2回は切らない）。
    socket: Arc<Mutex<Option<std::net::TcpStream>>>,
    /// 止める段の途中で切ったか。
    cut: Arc<AtomicBool>,
    /// **切った後に出た行**（診断用。赤のときに何が起きなかったかを言う）。
    after_cut: Arc<Mutex<Vec<String>>>,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CutWhenStopping {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = EventFields::default();
        event.record(&mut fields);
        let message = fields.message.unwrap_or_default();
        if self.cut.load(Ordering::SeqCst) {
            // **personacore の行だけ**（`hyper` / `reqwest` の行は照会のたびに十数行出る）。
            if event.metadata().target().starts_with("personacore") {
                self.after_cut
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(message);
            }
            return;
        }
        if message == STOPPING_LINE && fields.endpoint.as_deref() == Some(self.endpoint.as_str()) {
            let socket = self
                .socket
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(socket) = socket {
                // **ここが「画面を移った」**（`UpstreamProfiles.svelte` の POST が中断される）。
                drop(socket);
                self.cut.store(true, Ordering::SeqCst);
            }
        }
    }
}

/// **生の TCP で切り替えの依頼を書き、そのソケットを返す**（**応答は読まない**）。
///
/// `reqwest` では「途中で切る」を確実に表せない（`tests/server_chat_cancel.rs` の `send_request`）。
/// **返したソケットを落とすことが「画面を移った」。**
fn post_switch_raw(port: u16, name: &str) -> std::net::TcpStream {
    let mut socket =
        std::net::TcpStream::connect(("127.0.0.1", port)).expect("サーバへ接続できること");
    let body = serde_json::json!({ "name": name }).to_string();
    write!(
        socket,
        "POST /admin/upstream/profile HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .expect("依頼を書けること");
    socket.flush().expect("送出できること");
    socket
}

/// 行から読む欄（本文・宛先・PID だけ）。
#[derive(Default)]
struct EventFields {
    message: Option<String>,
    endpoint: Option<String>,
    /// `upstream_pid = ?pid` の綴り（`Some(1234)`）。**止める行と終了の行を同じ子で引き当てる。**
    pid: Option<String>,
}

impl tracing::field::Visit for EventFields {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        // **`record_debug` へ回さない** —— `&str` の `Debug` は引用符で囲む。
        self.keep(field, value.to_owned());
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        // **本文（`format_args!`）と `%` で書かれた欄（`DisplayValue`）の `Debug` は `Display` そのもの。**
        self.keep(field, format!("{value:?}"));
    }
}

impl EventFields {
    fn keep(&mut self, field: &tracing::field::Field, value: String) {
        match field.name() {
            "message" => self.message = Some(value),
            "upstream_endpoint" => self.endpoint = Some(value),
            "upstream_pid" => self.pid = Some(value),
            _ => {}
        }
    }
}

/// **切り替えの依頼を、止める段の途中（`kill` の手前）で接続ごと切っても、
/// 面が死んだ子の「古い ready」のまま残らない**（作業単位 B-56 / Issue origin#39）。
///
/// # 何が起きていたか（**是正前に実測した**）
///
/// **axum は接続が切れるとハンドラの future を落とす**（`tests/server_chat_cancel.rs`）。
/// 差し替えをハンドラの中で直接待っていたので、**止める段の途中で future ごと落ち**、
///
/// * 前の子は `kill_on_drop` で死ぬ（止める段は状態を書かない）
/// * クライアントの差し替えも、次の子を起こすことも走らない
///
/// ——**面は「準備完了（前の gguf）」と言い続け、照会先は死んだ子の宛先のまま**だった
/// （回数と直し方の判断は `server::switch::run_detached` の doc）。
///
/// # 何を見ているか
///
/// | 主張 | 破れると |
/// |---|---|
/// | **止める段の途中で切った**（[`CutWhenStopping`]） | 前提が崩れている（止める段に入っていない） |
/// | **両面が新しい gguf で `ready` になる** | 差し替えが接続と一緒に落ちている |
/// | 本文の照会先が新しい子の宛先 | クライアントを差し替えていない |
/// | 前の子は居ない | 止めていない |
/// | 新しい子がその宛先で新しい gguf を名乗る | 起こしていない |
///
/// **`sub` を書かないプロファイル**なので、1つの子が2面を担う——**両面とも見る。**
#[tokio::test]
async fn a_switch_cut_off_while_stopping_the_old_child_still_finishes() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let old = fake_gguf(dir.path(), "planted-cut-old.gguf");
    let new = fake_gguf(dir.path(), "planted-cut-new.gguf");
    let old_port = free_port();
    let new_port = free_port();
    let port = free_port();

    let old_endpoint = format!("http://{LAUNCH_HOST}:{old_port}");
    let source = serde_json::json!({
        "old": { "main": { "endpoint": old_endpoint, "model": "planted-cut-old-id", "gguf": old } },
        "new": { "main": { "endpoint": format!("http://{LAUNCH_HOST}:{new_port}"),
                           "model": "planted-cut-new-id", "gguf": new } },
    });
    let profiles_path = write_profiles(dir.path(), &source);
    let (profiles, _) = read(&source.to_string());
    let (config, plan) = config_of(&profiles, "old", dir.path(), port);
    let mut serving = serve_switchable(config, plan, profiles_path);
    wait_for_ready_as(port, "main", "planted-cut-old.gguf").await;
    wait_for_ready_as(port, "sub", "planted-cut-old.gguf").await;

    let socket = post_switch_raw(port, "new");

    let cut = Arc::new(AtomicBool::new(false));
    let after_cut = Arc::new(Mutex::new(Vec::new()));
    let layer = CutWhenStopping {
        endpoint: old_endpoint.clone(),
        socket: Arc::new(Mutex::new(Some(socket))),
        cut: Arc::clone(&cut),
        after_cut: Arc::clone(&after_cut),
    };
    // **サーバのタスクが走るのは、このテストが次に `await` したとき**（current_thread）
    // ——**差すのは依頼を書いた後でも間に合う。**
    let _guard = tracing::subscriber::set_default(tracing_subscriber::layer::SubscriberExt::with(
        tracing_subscriber::registry(),
        layer,
    ));

    // **前提: 止める段の途中で切った**（A に一度も入らなければ、ここで落ちる）。
    for _ in 0..MAX_WAITS {
        if cut.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
    assert!(
        cut.load(Ordering::SeqCst),
        "前提: 止める段（「{STOPPING_LINE}」の行）に入らなかった。依頼が差し替えまで届いていない"
    );

    // **本題: 両面が新しい gguf で ready になる**（前の gguf の ready のままにならない）。
    // **待つのは「新しい名前で ready」**——前の名前の ready（＝是正前に残っていた状態）では成立しない。
    let mut last = serde_json::Value::Null;
    let mut finished = false;
    for _ in 0..MAX_WAITS {
        if let Some(report) = upstream_report(port).await {
            finished = ["main", "sub"].iter().all(|face| {
                report[face]["launch"]["state"] == serde_json::json!("ready")
                    && report[face]["launch"]["answered_name"]
                        == serde_json::json!("planted-cut-new.gguf")
            });
            last = report;
            if finished {
                break;
            }
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
    if !finished {
        // **赤のときだけ、子そのものにも聞いて言う**（面の言い分と子の生死の食い違いが本題）。
        let old_child_answers = answers_at(LAUNCH_HOST, old_port).await;
        panic!(
            "止める段の途中で依頼を切ったら、差し替えが最後まで走らなかった（{MAX_WAITS} 回待った）。\n\
             いまの面: main {} / sub {}\n\
             本文の照会先: {}（前の子のポート {old_port} / 次の子のポート {new_port}）\n\
             前の子はまだ答えるか: {old_child_answers}\n\
             切った後に出た personacore の行: {:?}",
            last["main"]["launch"],
            last["sub"]["launch"],
            last["main"]["endpoint"],
            after_cut.lock().unwrap_or_else(PoisonError::into_inner)
        );
    }
    assert!(
        last["main"]["endpoint"]
            .as_str()
            .is_some_and(|endpoint| endpoint.contains(&format!(":{new_port}/"))),
        "照会先が新しい子へ差し替わっていない: {last}"
    );

    // **前の子は居ない・新しい子が答える**（子そのものに聞く）。
    wait_until_gone_at(LAUNCH_HOST, old_port, "前の子").await;
    assert!(
        models_named_at(LAUNCH_HOST, new_port)
            .await
            .contains("planted-cut-new.gguf"),
        "新しい子が答えていない"
    );

    serving.stop().await;
    wait_until_gone_at(LAUNCH_HOST, new_port, "新しい子").await;
}

// ----------------------------------------------------------------------
// 終了処理と差し替えが重なる（作業単位 B-56 のレビュー提案1）
// ----------------------------------------------------------------------

/// **終了処理が始まったと名乗る行**（`server::switch::Switcher::shutdown`。**錠を取る前に出る**）。
const SHUTDOWN_LINE: &str =
    "上流の差し替えを閉じます（進行中の差し替えがあれば、終わってから子を止めます）";

/// 控えた1行。
#[derive(Debug, Clone)]
struct SeenLine {
    message: String,
    endpoint: Option<String>,
    pid: Option<String>,
}

/// **personacore の行を全部控え、終了処理の行が出た瞬間に、掴んでいた照会を放す層。**
///
/// # なぜ行で放すのか（**競走しない**）
///
/// **放すのが早すぎると**、差し替えは終了処理より前に終わり、錠は空いている
/// ——「終了処理が、走っている差し替えを待った」を一度も作らずに緑になる。
/// **放さなければ**、正しい実装の終了処理は錠を待ち続ける（照会が返らない）。
/// **終了処理が錠を取りに行く直前（その行を書いている呼び出しの中）で放す**と、
/// 差し替えは**必ず錠を持ったまま**終了処理を迎え、しかも終了処理が待てば終わる。
///
/// **⚠ スレッドローカルに差す**（[`CutWhenStopping`] と同じ理由）。
struct ReleaseOnShutdown {
    /// 差し替えが掴んでいる照会（**放したら `None`**）。
    held: Arc<Mutex<Option<tokio::net::TcpStream>>>,
    /// **放したのが [`Self::lines`] の何本目の行でか**（放していなければ `None`）。
    released_at: Arc<Mutex<Option<usize>>>,
    /// personacore の行（`hyper` / `reqwest` の行は入れない）。
    lines: Arc<Mutex<Vec<SeenLine>>>,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for ReleaseOnShutdown {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if !event.metadata().target().starts_with("personacore") {
            return;
        }
        let mut fields = EventFields::default();
        event.record(&mut fields);
        let message = fields.message.unwrap_or_default();
        let index = {
            let mut lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
            lines.push(SeenLine {
                message: message.clone(),
                endpoint: fields.endpoint,
                pid: fields.pid,
            });
            lines.len() - 1
        };
        if message == SHUTDOWN_LINE {
            let held = self
                .held
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(held) = held {
                drop(held);
                *self
                    .released_at
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(index);
            }
        }
    }
}

/// **差し替えが錠を持っている最中に終了処理が始まったら、終了処理はその差し替えを待ち、
/// その差し替えが起こした子まで止めてから終わる**（作業単位 B-56 のレビュー提案1）。
///
/// # なぜ要るのか
///
/// **差し替えは依頼の接続から切り離したタスクで走る**（`server::switch::run_detached`）ので、
/// 依頼の接続を切れば、`serve` は差し替えの途中でも終了処理へ進む。
/// **終了処理が錠を待たない実装**（`try_lock` で取れなければ返す等）では、
/// **差し替えが起こした子を誰も止めない**——`kill_on_drop` が後で殺すだけで、終了は待たない。
///
/// # 重ね方
///
/// 1. 前の子（本文と抽出の両面）が `ready` の起動で、**本文は新しい gguf・抽出は答えない外の上流**の
///    プロファイルへ差し替える。差し替えは「止める → 起こす → **抽出の面を測り直す**」の順なので、
///    **答えない上流が照会を受けた時点で、新しい子は起きていて、差し替えはまだ錠を持っている。**
/// 2. 依頼の接続を切り、停止の合図を送る。
/// 3. **終了処理が錠を取りに行く直前に、照会を放す**（[`ReleaseOnShutdown`]）。
///
/// # 何を見ているか
///
/// | 主張 | 破れると |
/// |---|---|
/// | **前提**: 新しい子を起こした後、照会を掴んだまま終了処理が始まった | 重なっていない |
/// | 差し替えが終わった行が、終了処理の行より後で、新しい子を止める行より前 | 終了処理が差し替えを待っていない |
/// | **新しい子を止める行と、同じ PID の終了の行が、終了処理の行より後にある** | 差し替えが起こした子を止めていない |
/// | `serve` が返った時点で、新しい子は答えない | 同上 |
#[tokio::test]
async fn a_shutdown_waits_for_a_running_switch_and_stops_what_it_started() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let old = fake_gguf(dir.path(), "planted-closing-old.gguf");
    let new = fake_gguf(dir.path(), "planted-closing-new.gguf");
    let old_port = free_port();
    let new_port = free_port();
    let port = free_port();
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("答えない上流を立てられること");
    let silent_port = silent.local_addr().expect("宛先を読めること").port();

    let new_endpoint = format!("http://{LAUNCH_HOST}:{new_port}");
    let source = serde_json::json!({
        // **`sub` を書かない** —— 前の子が両面を担う。
        "old": { "main": { "endpoint": format!("http://{LAUNCH_HOST}:{old_port}"),
                           "model": "planted-closing-old-id", "gguf": old } },
        // **本文は起こし直し、抽出は答えない外の上流**（差し替えの最後の測り直しで錠を持ったまま止まる）。
        "next": { "main": { "endpoint": new_endpoint, "model": "planted-closing-new-id", "gguf": new },
                  "sub":  { "endpoint": format!("http://127.0.0.1:{silent_port}"),
                            "model": "planted-closing-silent-id" } },
    });
    let profiles_path = write_profiles(dir.path(), &source);
    let (profiles, _) = read(&source.to_string());
    let (config, plan) = config_of(&profiles, "old", dir.path(), port);
    let mut serving = serve_switchable(config, plan, profiles_path);
    wait_for_ready_as(port, "main", "planted-closing-old.gguf").await;
    wait_for_ready_as(port, "sub", "planted-closing-old.gguf").await;

    let held = Arc::new(Mutex::new(None));
    let released_at = Arc::new(Mutex::new(None));
    let lines = Arc::new(Mutex::new(Vec::new()));
    let _guard = tracing::subscriber::set_default(tracing_subscriber::layer::SubscriberExt::with(
        tracing_subscriber::registry(),
        ReleaseOnShutdown {
            held: Arc::clone(&held),
            released_at: Arc::clone(&released_at),
            lines: Arc::clone(&lines),
        },
    ));
    let seen = || lines.lock().unwrap_or_else(PoisonError::into_inner).clone();

    let socket = post_switch_raw(port, "next");
    // **答えない上流が照会を受けた＝差し替えは測り直しの最中で、錠を持っている。**
    let (probe, _) = tokio::time::timeout(Duration::from_secs(30), silent.accept())
        .await
        .expect("前提: 差し替えが抽出の面の測り直しに来ること")
        .expect("受けられること");
    assert!(
        seen().iter().any(|line| line.message == "上流を起こします"
            && line.endpoint.as_deref() == Some(new_endpoint.as_str())),
        "前提: 照会を受けた時点で、新しい子を起こしていること: {:?}",
        seen()
    );
    *held.lock().unwrap_or_else(PoisonError::into_inner) = Some(probe);
    // **放した後の照会（別の綴りへの読み替え）は、受けては切る**（終了処理を吊るさない）。
    let drain = tokio::spawn(async move {
        while let Ok((stream, _)) = silent.accept().await {
            drop(stream);
        }
    });

    // **依頼の接続を切り、停止の合図を送る**（切らないと、`serve` は応答を待ち続ける）。
    drop(socket);
    tokio::time::timeout(Duration::from_secs(60), serving.stop())
        .await
        .expect("終了処理が時間内に終わること");
    drain.abort();

    let lines = seen();
    let released = released_at
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .unwrap_or_else(|| {
            panic!("前提: 照会を掴んだまま終了処理が始まっていない（「{SHUTDOWN_LINE}」の行が無い）: {lines:?}")
        });
    let position = |from: usize, found: &dyn Fn(&SeenLine) -> bool| {
        lines
            .iter()
            .enumerate()
            .skip(from)
            .find(|(_, line)| found(line))
            .map(|(at, _)| at)
    };
    let stopping = position(released, &|line| {
        line.message == STOPPING_LINE && line.endpoint.as_deref() == Some(new_endpoint.as_str())
    })
    .unwrap_or_else(|| {
        panic!("終了処理が、差し替えが起こした新しい子を止めていない（止める行が無い）: {lines:?}")
    });
    let pid = lines[stopping]
        .pid
        .clone()
        .unwrap_or_else(|| panic!("止める行が PID を名乗っていない: {:?}", lines[stopping]));
    position(stopping, &|line| {
        line.message == "起こした上流が終了しました" && line.pid.as_deref() == Some(pid.as_str())
    })
    .unwrap_or_else(|| panic!("新しい子（{pid}）の終了を見届けていない: {lines:?}"));
    let switched = position(released, &|line| line.message == "上流を差し替えました")
        .unwrap_or_else(|| panic!("終了処理の後に、差し替えが終わった行が無い: {lines:?}"));
    assert!(
        switched < stopping,
        "終了処理が差し替えを待たずに止めている（差し替え {switched} → 止める {stopping} であること）: {lines:?}"
    );
    assert!(
        !answers_at(LAUNCH_HOST, new_port).await,
        "serve が返ったのに、差し替えが起こした子がまだ答える"
    );
}

/// **抽出の面だけを外の上流どうしで差し替えると、本文は触られない**（レビュー1周目の是正）。
///
/// **差し替え口の帰属を、本番の入口で見る**——候補が本文の口へ入る実装は、
/// 次のターンを抽出の新しい上流へ送り、抽出の面の測り直しは前の上流へ聞く。
/// **ターンは本文の上流にだけ届き、抽出の面は新しい上流の名乗りを持つこと。**
#[tokio::test]
async fn switching_only_the_extraction_face_leaves_the_body_alone() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    let main = external_upstream("planted-body-model").await;
    let sub_before = external_upstream("planted-sub-before-model").await;
    let sub_after = external_upstream("planted-sub-after-model").await;
    let port = free_port();

    let main_face = serde_json::json!({ "endpoint": main.uri(), "model": "planted-body-model" });
    let source = serde_json::json!({
        "before": { "main": main_face,
                    "sub": { "endpoint": sub_before.uri(), "model": "planted-sub-before-model" } },
        "after":  { "main": main_face,
                    "sub": { "endpoint": sub_after.uri(), "model": "planted-sub-after-model" } },
    });
    let profiles_path = write_profiles(dir.path(), &source);
    let (profiles, _) = read(&source.to_string());
    let (config, plan) = config_of(&profiles, "before", dir.path(), port);
    let mut serving = serve_switchable(config, plan, profiles_path);
    report_once(port).await;
    let main_probes = probes_to(&main).await;

    let (status, body) = switch_to(port, "after").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["sub"]["answered_name"],
        serde_json::json!("planted-sub-after-model"),
        "抽出の面を新しい上流で測り直していない: {body}"
    );
    assert!(
        body["sub"]["endpoint"]
            .as_str()
            .is_some_and(|endpoint| endpoint.starts_with(&sub_after.uri())),
        "{body}"
    );
    assert!(
        body["main"]["endpoint"]
            .as_str()
            .is_some_and(|endpoint| endpoint.starts_with(&main.uri())),
        "本文の宛先が動いている: {body}"
    );
    assert_eq!(
        probes_to(&main).await,
        main_probes,
        "変わらない本文の面まで測り直している"
    );

    let client = reqwest::Client::new();
    let created = client
        .post(format!("http://127.0.0.1:{port}/admin/characters"))
        .json(&serde_json::json!({ "handle": "aoi", "base_persona": "落ち着いた口調で話す。" }))
        .send()
        .await
        .expect("繋がること");
    assert!(created.status().is_success(), "{:?}", created.status());
    let response = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "ただいま"}],
            "stream": false,
        }))
        .send()
        .await
        .expect("繋がること");
    assert!(response.status().is_success(), "{:?}", response.status());

    let arrived = chats_to(&main).await;
    assert_eq!(arrived.len(), 1, "本文のターンが本文の上流へ行っていない");
    assert_eq!(arrived[0]["model"], serde_json::json!("planted-body-model"));
    // **抽出の新しい上流へ本文のターンが行っていない**（抽出の依頼は来うるので、綴りで見る）。
    assert!(
        chats_to(&sub_after)
            .await
            .iter()
            .all(|request| request["model"] == serde_json::json!("planted-sub-after-model")),
        "本文のターンが抽出の上流へ行っている"
    );

    serving.stop().await;
}

// ----------------------------------------------------------------------
// 載せている最中の 503（作業単位 B-51 / Issue origin#20）
// ----------------------------------------------------------------------

/// どちらの読み手も読めなかったことを告げる行の手掛かり（`llm::lmstudio`）。
const NEITHER_READER_MARKER: &str = "どちらの綴りでも読めませんでした";

/// 偽物が 503 を返したたびに書く行（`tests/support/fake_llama.rs` の `LOADING_LINE`）。
const FAKE_LOADING_LINE: &str = "載せている最中なので 503 を返しました";

/// **どの綴りにも 503 を返す外の上流**（`wiremock`）。**起こしていない面の対照。**
async fn unreadable_upstream() -> wiremock::MockServer {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers};
    let server = MockServer::start().await;
    for listing in ["/api/v0/models", "/v1/models"] {
        Mock::given(matchers::method("GET"))
            .and(matchers::path(listing))
            .respond_with(ResponseTemplate::new(503).set_body_string(
                r#"{"error":{"code":503,"message":"Loading model","type":"unavailable_error"}}"#,
            ))
            .mount(&server)
            .await;
    }
    server
}

/// **起こした子の標準出力を、全世代ぶんつないで読む**
/// （`upstream-logs/<面>-<時刻>.log`。標準エラーは `.err.log` なので除く）。
fn child_stdout(dir: &Path) -> String {
    let logs = dir.join("db").join("upstream-logs");
    let mut out = String::new();
    for entry in std::fs::read_dir(&logs).expect("子の出力の落とし先を読めること") {
        let path = entry.expect("項目を読めること").path();
        let name = path.to_string_lossy();
        if name.ends_with(".log") && !name.ends_with(".err.log") {
            out.push_str(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|err| panic!("子の標準出力を読めること: {path:?} / {err}")),
            );
        }
    }
    out
}

/// **載せている面の「一覧が読めない」は警告にしない。外の上流の同じ 503 は警告のまま**
/// （作業単位 B-51 / Issue origin#20 / ADR-0042 §決定3）。
///
/// # 何を見るか（**同じ1回の起動・同じ既定のフィルタ・同じ行の中で**）
///
/// | 面 | 上流 | 期待 |
/// |---|---|---|
/// | `main`（**起こす**） | 偽の `llama`。起きてから数秒、どの綴りにも 503（`--pc-loading`） | **警告の行が1本も無い**（番人が 503 を受けても） |
/// | `sub`（**起こさない**） | `wiremock`。ずっと 503 | **起動時の照会が警告を出す**（今のまま） |
///
/// **行の帰属は依頼した綴り（`model=`）で見る** —— グローバルのログには同じバイナリの
/// 他のテストの行も混ざるので、**このテストだけの綴り**にしてある。
///
/// # 「警告が無い」が空振りしていないことの確かめ
///
/// * **番人が本当に 503 を受けたこと**を、偽物の標準出力（[`FAKE_LOADING_LINE`]）で見る。
///   **受けていなければ、警告が無いのは当たり前**（503 の腕が壊れているだけでも緑になる）。
/// * **同じ文面の行が、同じ回に `sub` については警告で出ていること**（対照）。
///   **文面が変わった・行ごと消えた・綴りの欄が消えた**なら、`main` の否定は何も見ていない。
///
/// **⚠ 既定のフィルタでは `debug` は見えない。** 「経過として出ている」の肯定は
/// `server::context::tests::an_unreadable_list_is_progress_for_a_launching_face_but_a_warning_for_an_external_one`
/// が `DEBUG` で捕まえて見る。**ここが見るのは、番人がその経路を通っていること（配線）。**
#[tokio::test]
async fn a_list_the_launching_face_cannot_read_yet_is_not_a_warning_but_an_external_one_is() {
    const LAUNCHED_MODEL: &str = "planted-b51-launched-id";
    const EXTERNAL_MODEL: &str = "planted-b51-external-id";

    let log = install_global_log(DEFAULT_LOG_FILTER);
    let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
    fake_llama_on_path();
    let gguf = fake_gguf(dir.path(), "planted-b51-loading.gguf");
    let upstream_port = free_port();
    let port = free_port();
    let sub = unreadable_upstream().await;

    let (profiles, _) = read(
        &serde_json::json!({
            "planted": {
                "main": {
                    "endpoint": format!("http://{LAUNCH_HOST}:{upstream_port}"),
                    "model": LAUNCHED_MODEL,
                    "gguf": gguf,
                    // **番人は2秒ごとに聞く**ので、5秒あれば 503 を少なくとも1回は受ける。
                    "args": ["--pc-loading", "5"],
                },
                "sub": {
                    "endpoint": sub.uri(),
                    "model": EXTERNAL_MODEL,
                }
            }
        })
        .to_string(),
    );
    let (config, plan) = config_of(&profiles, "planted", dir.path(), port);

    let mark = log.mark();
    let mut serving = serve(config, plan);
    let launch = wait_for_state(port, "main", "ready").await;
    assert_eq!(
        launch["answered_name"],
        serde_json::json!("planted-b51-loading.gguf"),
        "前提: 載せ終わってから ready になること: {launch}"
    );
    let section = log.contents_since(mark);

    // **前提: 番人は載せている最中の 503 を本当に受けた。**
    let stdout = child_stdout(dir.path());
    assert!(
        stdout.contains(FAKE_LOADING_LINE),
        "前提: 偽物が 503 を一度も返していない（番人が載せている最中を通っていない）: {stdout}"
    );

    let neither_lines = |model: &str| -> Vec<&str> {
        let field = format!("model={model}");
        section
            .lines()
            .filter(|line| line.contains(NEITHER_READER_MARKER) && common::has_field(line, &field))
            .collect()
    };

    // **対照: 外の上流の面は、今のまま警告を出している**（同じ行に文面・水準・綴り）。
    let external = neither_lines(EXTERNAL_MODEL);
    assert!(
        external
            .iter()
            .any(|line| line.split_whitespace().nth(1) == Some("WARN")),
        "外の上流の「どちらの綴りでも読めない」が警告として出ていない: {section}"
    );

    // **本命: 載せている面については、警告の行が1本も無い。**
    let launched_warnings: Vec<&str> = neither_lines(LAUNCHED_MODEL)
        .into_iter()
        .filter(|line| line.split_whitespace().nth(1) == Some("WARN"))
        .collect();
    assert!(
        launched_warnings.is_empty(),
        "載せている面の 503 が警告として出ている: {launched_warnings:?}"
    );

    serving.stop().await;
    wait_until_gone_at(LAUNCH_HOST, upstream_port, "起こした上流").await;
}
