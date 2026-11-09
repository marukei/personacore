//! **待ち受けのポートの口**（`GET /admin/server` / `PUT /admin/server/port` /
//! [ADR-0057](../docs/adr/0057-the-listening-port-shifts-when-taken-and-is-set-from-the-screen.md) §決定5・6 /
//! 作業単位 F-12a）を、**本番の入口（`App::run_with_upstreams`）で**確かめる。
//!
//! **本物の `llama` にも LM Studio にも繋がない**（計画は空・上流の宛先は閉じたポート）。
//! **置き場所はどれも一時ディレクトリ**で、`Startup` に直に渡す。
//! **`main` が「既定なら保存できる／`PERSONACORE_UPSTREAMS` なら読むだけ」と決める1行と、保存した番号で
//! 実際に待ち受けることは `tests/listening_port_cli.rs` が実行ファイルで見る**（このファイルからは観測できない）。
//!
//! # ここが見ているもの
//!
//! | 見張り番 | 何が壊れたら赤になるか |
//! |---|---|
//! | `the_report_names_the_actual_port_and_where_it_came_from` | 実際の番号でなく設定値を言う／出どころを取り違える |
//! | `a_saved_port_reads_back_and_null_forgets_it` | 保存が `GET` に出ない・ファイルに書かれない・`null` で消えない・次の起動の番号が変わらない |
//! | `a_port_outside_1024_to_65535_is_refused_and_nothing_is_written` | 範囲の外を保存する／理由が届かない／`port` を省いた依頼が保存を消す |
//! | `a_read_only_location_neither_reads_nor_writes_the_saved_port` | 読むだけの置き場所で保存を読む・書く |
//! | `in_use_is_said_only_for_a_port_another_program_holds` | 自分の番号を「使われている」と言う／他が掴んだ番号を「空いている」と言う／使われていると保存しない |
//!
//! # 前提
//!
//! **このテストの実行ファイルに `PERSONACORE_PORT` が立っていないこと**（`GET /admin/server` の「次の起動」は
//! いまのプロセスの環境変数を読む）。環境変数が勝つ腕は `tests/listening_port_cli.rs` が子の環境で見る。

use std::path::{Path, PathBuf};
use std::time::Duration;

use personacore::server::{
    App, AppConfig, BindAddr, DEFAULT_PORT, ENV_LLM_ENDPOINT, ENV_PORT, LaunchPlan, SavedPort,
    ServerError, Startup,
};
use reqwest::Method;
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// 待つ側の上限（回数）と1回あたりの待ち時間（合わせて 30 秒）。
const MAX_WAITS: usize = 300;
const WAIT_STEP: Duration = Duration::from_millis(100);

/// **上流の宛先**（閉じている loopback のポート）。**LM Studio に繋ぎに行かせない。**
const CLOSED_ENDPOINT: &str = "http://127.0.0.1:9";

/// ポートを取られて待ち受けを開けなかったとき、別のポートで起こし直す回数の上限。
const MAX_BINDS: usize = 5;

/// **1回の依頼の上限**（`tests/upstream_profile_editor.rs` の `REQUEST_TIMEOUT` と同じ理由）。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("ポートを取れること");
    let port = listener.local_addr().expect("宛先を読めること").port();
    drop(listener);
    port
}

/// **このテストの前提**（モジュールの doc）。
fn assert_no_port_in_this_process() {
    assert!(
        std::env::var_os(ENV_PORT).is_none(),
        "前提: このテストの実行ファイルに {ENV_PORT} が立っていないこと"
    );
}

/// **出どころをどう作るか**（設定の入口ごとに1つ）。
#[derive(Clone, Copy)]
enum Source {
    /// `PERSONACORE_PORT` を参照関数から渡す。
    Environment,
    /// 保存した番号として渡す（`from_lookup_with_data_dir_and_saved_port`）。
    Saved,
    /// 渡さずに組み、`with_bind` で番号だけ差し替える（出どころは `default` のまま）。
    Default,
}

/// **走っているサーバ。**
struct Serving {
    port: u16,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), ServerError>>>,
}

impl Serving {
    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.await
                .expect("サーバのタスクが落ちていないこと")
                .expect("サーバが正常に止まること");
        }
    }

    /// **この待ち受けが `GET /admin/server` に答えるまで待つ。** サーバのタスクが終わったら `false`。
    ///
    /// **答えの形まで見る**（ポートを取った別の誰かの応答を、このサーバの応答と読まない）。
    async fn answers(&self) -> bool {
        let url = format!("http://127.0.0.1:{}/admin/server", self.port);
        for _ in 0..MAX_WAITS {
            if self
                .task
                .as_ref()
                .is_none_or(tokio::task::JoinHandle::is_finished)
            {
                return false;
            }
            let answered = reqwest::Client::new()
                .get(&url)
                .timeout(Duration::from_secs(2))
                .send()
                .await;
            if let Ok(response) = answered
                && let Ok(body) = response.json::<Value>().await
                && body.get("listening_port").is_some()
            {
                return true;
            }
            tokio::time::sleep(WAIT_STEP).await;
        }
        panic!("待ち受けを {MAX_WAITS} 回待っても答えなかった");
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// **本番の入口でサーバを起こし、答えるまで待つ。** 置き場所の渡し方は呼び出し側が決める。
///
/// **ポートを取られたら別のポートで起こし直す**（`tests/upstream_profile_editor.rs` の `serve` と同じ理由）。
async fn serve(dir: &Path, source: Source, startup: impl Fn(Startup) -> Startup) -> Serving {
    for _ in 0..MAX_BINDS {
        let port = free_port();
        let port_text = port.to_string();
        let lookup = move |key: &str| match key {
            ENV_LLM_ENDPOINT => Some(CLOSED_ENDPOINT.to_owned()),
            ENV_PORT if matches!(source, Source::Environment) => Some(port_text.clone()),
            _ => None,
        };
        let saved = match source {
            Source::Saved => Some(SavedPort::new(port).expect("前提: OS が選んだ番号は 1024 以上")),
            Source::Environment | Source::Default => None,
        };
        let mut config = AppConfig::from_lookup_with_data_dir_and_saved_port(
            lookup,
            Some(dir.to_path_buf()),
            saved,
        )
        .expect("設定を組めること")
        .with_database_path(dir.join("db").join("personacore.sqlite"));
        if matches!(source, Source::Default) {
            config = config.with_bind(BindAddr::loopback(port));
        }
        let plan = startup(Startup::new(LaunchPlan::none()));

        let (shutdown, wait) = oneshot::channel();
        let task = tokio::spawn(async move {
            App::run_with_upstreams(&config, plan, async move {
                let _ = wait.await;
            })
            .await
        });
        let mut serving = Serving {
            port,
            shutdown: Some(shutdown),
            task: Some(task),
        };
        if serving.answers().await {
            return serving;
        }
        serving.task = None;
    }
    panic!("{MAX_BINDS} 回起こし直しても待ち受けを開けなかった");
}

/// **1回の依頼。**
async fn send(port: u16, method: Method, path: &str, body: Option<&Value>) -> (u16, Value) {
    let url = format!("http://127.0.0.1:{port}{path}");
    let mut request = reqwest::Client::new()
        .request(method, &url)
        .timeout(REQUEST_TIMEOUT);
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.expect("管理 API へ繋がること");
    let status = response.status().as_u16();
    let text = response.text().await.expect("本文を読めること");
    let json = serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("JSON として読めること: {err} / {text}"));
    (status, json)
}

/// **生の本文で1回送る**（JSON にならない本文・型の違う本文を作るため）。
async fn send_raw(port: u16, path: &str, body: &str) -> (u16, Value) {
    let response = reqwest::Client::new()
        .put(format!("http://127.0.0.1:{port}{path}"))
        .header("Content-Type", "application/json")
        .body(body.to_owned())
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .expect("管理 API へ繋がること");
    let status = response.status().as_u16();
    let text = response.text().await.expect("本文を読めること");
    let json = serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("JSON として読めること: {err} / {text}"));
    (status, json)
}

async fn report(port: u16) -> Value {
    let (status, body) = send(port, Method::GET, "/admin/server", None).await;
    assert_eq!(status, 200, "{body}");
    body
}

async fn save(port: u16, body: &Value) -> (u16, Value) {
    send(port, Method::PUT, "/admin/server/port", Some(body)).await
}

/// **編集できる置き場所**（`upstreams.json` は置かなくてよい —— 保存のファイルはその隣）。
fn editable_dir(root: &Path) -> (PathBuf, PathBuf) {
    let app = root.join("app");
    std::fs::create_dir_all(&app).expect("作れること");
    (app.join("upstreams.json"), app.join("listening-port.txt"))
}

/// ディレクトリの中身（**一時ファイルが残っていないか**を見る）。
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("読めること")
        .map(|entry| {
            entry
                .expect("読めること")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

// ----------------------------------------------------------------------
// 本命
// ----------------------------------------------------------------------

/// **`listening_port` は実際に待ち受けている番号、`listening_port_source` はその出どころ。**
///
/// **3つの出どころを、同じ形のサーバで1つずつ作る**（取り違える実装は、どれかの腕で赤になる）。
/// `default` の腕は `with_bind` でエフェメラルポートへ差し替えている——**設定値（8080）を言う実装は、
/// `listening_port` が実際の番号と食い違って赤になる。**
#[tokio::test]
async fn the_report_names_the_actual_port_and_where_it_came_from() {
    assert_no_port_in_this_process();
    for (source, label) in [
        (Source::Environment, "environment"),
        (Source::Saved, "saved"),
        (Source::Default, "default"),
    ] {
        let dir = tempfile::tempdir().expect("一時ディレクトリ");
        let (profiles, _) = editable_dir(dir.path());
        let server = serve(dir.path(), source, |startup| {
            startup.with_editable_profiles_file(profiles.clone())
        })
        .await;
        assert_ne!(
            server.port, DEFAULT_PORT,
            "前提: 既定と違う番号で待ち受けること"
        );

        assert_eq!(
            report(server.port).await,
            json!({
                "listening_port": server.port,
                "listening_port_source": label,
                // **保存のファイルは無い**（起動の設定に渡した保存は、ファイルではない）。
                "saved_port": null,
                "next_port": DEFAULT_PORT,
                "next_port_source": "default",
                "editable": true,
            }),
            "{label}"
        );
        server.stop().await;
    }
}

/// **保存すると、ファイルに1行で書かれ、`GET` と「次の起動」がすぐ変わる。いまの待ち受けは移らない。
/// `null` で消すと、ファイルが消え、次の起動は既定に戻る。**
#[tokio::test]
async fn a_saved_port_reads_back_and_null_forgets_it() {
    assert_no_port_in_this_process();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (profiles, file) = editable_dir(dir.path());
    let app_dir = file.parent().expect("親があること").to_path_buf();
    let server = serve(dir.path(), Source::Environment, |startup| {
        startup.with_editable_profiles_file(profiles.clone())
    })
    .await;

    // **保存する番号は、いまの待ち受けとも既定とも違う**（取り違えが観測できるように）。
    let wanted = free_port();
    assert_ne!(wanted, server.port, "前提");
    assert_ne!(wanted, DEFAULT_PORT, "前提");

    let (status, saved) = save(server.port, &json!({ "port": wanted })).await;
    assert_eq!(status, 200, "{saved}");
    assert_eq!(
        saved["listening_port"],
        json!(server.port),
        "待ち受けが移っている: {saved}"
    );
    assert_eq!(saved["listening_port_source"], "environment", "{saved}");
    assert_eq!(saved["saved_port"], json!(wanted), "{saved}");
    assert_eq!(saved["next_port"], json!(wanted), "{saved}");
    assert_eq!(saved["next_port_source"], "saved", "{saved}");
    assert_eq!(saved["editable"], json!(true), "{saved}");
    assert_eq!(
        std::fs::read_to_string(&file).expect("保存のファイルが在ること"),
        format!("{wanted}\n")
    );
    assert_eq!(
        entries(&app_dir),
        vec!["listening-port.txt".to_owned()],
        "一時ファイルが残っている"
    );

    // **`GET` も同じことを言う**（保存の応答の写しを返しているだけ、ではない）。
    let read = report(server.port).await;
    assert_eq!(read["saved_port"], json!(wanted), "{read}");
    assert_eq!(read["next_port"], json!(wanted), "{read}");
    assert_eq!(read["next_port_source"], "saved", "{read}");

    // **手で書き換えた中身も、次の `GET` が読み直す**（起動時の写しを持ち続けない）。
    let edited = free_port();
    std::fs::write(&file, format!("\u{feff}{edited}\r\n")).expect("書けること");
    assert_eq!(report(server.port).await["saved_port"], json!(edited));

    let (status, forgot) = save(server.port, &json!({ "port": null })).await;
    assert_eq!(status, 200, "{forgot}");
    assert_eq!(
        forgot,
        json!({
            "listening_port": server.port,
            "listening_port_source": "environment",
            "saved_port": null,
            "next_port": DEFAULT_PORT,
            "next_port_source": "default",
            "editable": true,
            "saved_port_in_use": null,
        })
    );
    assert!(!file.exists(), "保存を消したのにファイルが残っている");

    // **無いものを消しても 200**（冪等）。
    let (status, again) = save(server.port, &json!({ "port": null })).await;
    assert_eq!(status, 200, "{again}");
    assert_eq!(entries(&app_dir), Vec::<String>::new());

    server.stop().await;
}

/// **1024〜65535 の外は 400 `invalid_listening_port`。本文の誤りは 400 `invalid_body`。
/// どれもファイルは1バイトも変わらない。両端の内側は保存できる。**
///
/// **`port` を省いた本文は「消す」にならない**（書き忘れた依頼が保存を黙って消さない）。
#[tokio::test]
async fn a_port_outside_1024_to_65535_is_refused_and_nothing_is_written() {
    assert_no_port_in_this_process();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (profiles, file) = editable_dir(dir.path());
    let server = serve(dir.path(), Source::Environment, |startup| {
        startup.with_editable_profiles_file(profiles.clone())
    })
    .await;
    std::fs::write(&file, "8090\n").expect("置けること");
    let original = std::fs::read(&file).expect("読めること");

    for value in [
        json!(0),
        json!(1023),
        json!(65536),
        json!(-1),
        json!(i64::MAX),
    ] {
        let (status, refused) = save(server.port, &json!({ "port": value })).await;
        assert_eq!(status, 400, "{value}: {refused}");
        assert_eq!(
            refused["error"]["code"], "invalid_listening_port",
            "{value}: {refused}"
        );
        assert_eq!(
            std::fs::read(&file).expect("読めること"),
            original,
            "{value}"
        );
    }

    for body in [
        "{}",
        r#"{"port": "8091"}"#,
        r#"{"port": 8091.5}"#,
        r#"{"port": 8091, "extra": true}"#,
        "not json",
    ] {
        let (status, refused) = send_raw(server.port, "/admin/server/port", body).await;
        assert_eq!(status, 400, "{body}: {refused}");
        assert_eq!(
            refused["error"]["code"], "invalid_body",
            "{body}: {refused}"
        );
        assert_eq!(
            std::fs::read(&file).expect("読めること"),
            original,
            "{body}"
        );
    }

    // **対照: 両端の内側は保存できる**（断る実装が全部を断っていないこと）。
    for edge in [1024_u16, 65535] {
        let (status, saved) = save(server.port, &json!({ "port": edge })).await;
        assert_eq!(status, 200, "{edge}: {saved}");
        assert_eq!(
            std::fs::read_to_string(&file).expect("読めること"),
            format!("{edge}\n")
        );
    }

    server.stop().await;
}

/// **読むだけの置き場所（`PERSONACORE_UPSTREAMS` の形）では、隣の保存を読まず、保存の依頼は 409。**
///
/// 保存のファイルを**置いたうえで**見る——置かずに `saved_port: null` を見ても、読まなかったのか
/// 無かったのかが区別できない。
#[tokio::test]
async fn a_read_only_location_neither_reads_nor_writes_the_saved_port() {
    assert_no_port_in_this_process();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (profiles, file) = editable_dir(dir.path());
    std::fs::write(&file, "8090\n").expect("置けること");
    let original = std::fs::read(&file).expect("読めること");
    let server = serve(dir.path(), Source::Environment, |startup| {
        startup.with_profiles_file(profiles.clone())
    })
    .await;

    let read = report(server.port).await;
    assert_eq!(read["editable"], json!(false), "{read}");
    assert_eq!(
        read["saved_port"],
        Value::Null,
        "読むだけの置き場所で保存を読んでいる: {read}"
    );
    assert_eq!(read["next_port"], json!(DEFAULT_PORT), "{read}");
    assert_eq!(read["next_port_source"], "default", "{read}");

    for body in [json!({ "port": 9001 }), json!({ "port": null })] {
        let (status, refused) = save(server.port, &body).await;
        assert_eq!(status, 409, "{body}: {refused}");
        assert_eq!(
            refused["error"]["code"], "listening_port_read_only",
            "{body}: {refused}"
        );
        assert_eq!(
            std::fs::read(&file).expect("読めること"),
            original,
            "{body}"
        );
    }
    // **範囲の外は、置き場所より先に 400**（依頼そのものの誤り）。
    let (status, refused) = save(server.port, &json!({ "port": 80 })).await;
    assert_eq!(status, 400, "{refused}");
    assert_eq!(refused["error"]["code"], "invalid_listening_port");

    server.stop().await;
}

/// **他のプログラムが掴んでいる番号は `saved_port_in_use: true`。それでも保存する。
/// いまこの personacore が使っている番号は `false`。空いている番号も `false`。**
#[tokio::test]
async fn in_use_is_said_only_for_a_port_another_program_holds() {
    assert_no_port_in_this_process();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (profiles, file) = editable_dir(dir.path());
    let server = serve(dir.path(), Source::Environment, |startup| {
        startup.with_editable_profiles_file(profiles.clone())
    })
    .await;

    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("ポートを取れること");
    let held_port = held.local_addr().expect("宛先を読めること").port();
    let (status, taken) = save(server.port, &json!({ "port": held_port })).await;
    assert_eq!(status, 200, "使われていても保存は止めないこと: {taken}");
    assert_eq!(taken["saved_port_in_use"], json!(true), "{taken}");
    assert_eq!(taken["saved_port"], json!(held_port), "{taken}");
    assert_eq!(
        std::fs::read_to_string(&file).expect("読めること"),
        format!("{held_port}\n"),
        "使われている番号を保存していない"
    );

    // **自分自身の番号は「使われている」と言わない**（開いてみれば開けないが、それは自分である）。
    let (status, own) = save(server.port, &json!({ "port": server.port })).await;
    assert_eq!(status, 200, "{own}");
    assert_eq!(own["saved_port_in_use"], json!(false), "{own}");

    // **同じ番号を放したら「使われていない」**（いつも真を返す実装を赤にする）。
    drop(held);
    let (status, released) = save(server.port, &json!({ "port": held_port })).await;
    assert_eq!(status, 200, "{released}");
    assert_eq!(released["saved_port_in_use"], json!(false), "{released}");

    server.stop().await;
}
