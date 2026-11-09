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
//! | `a_saved_port_reads_back_and_null_forgets_it` | 保存が `GET` に出ない・ファイルに書かれない・`null` で消えない・次の起動の番号が変わらない・在るのに読めないファイルを `saved_port_unreadable` で言わない（空を壊れていると言う） |
//! | `a_port_outside_1024_to_65535_is_refused_and_nothing_is_written` | 範囲の外を保存する／理由が届かない／`port` を省いた依頼が保存を消す |
//! | `a_read_only_location_neither_reads_nor_writes_the_saved_port` | 読むだけの置き場所で保存を読む・書く・壊れたファイルを読めないと言う |
//! | `in_use_is_said_only_for_a_port_another_program_holds` | 自分の番号を「使われている」と言う／他が掴んだ番号を「空いている」と言う／使われていると保存しない |
//! | `a_taken_saved_port_shifts_past_the_face_ports_and_the_report_says_so` | 保存した番号が塞がっていると止まる／面の番号へずれる／`GET` が元の番号・ずらした・飛ばした番号を言わない（作業単位 F-11b） |
//! | `a_requested_port_that_a_face_uses_is_skipped_even_when_free` | 明示していないのに、面が使う元の番号で待ち受ける |
//! | `an_explicit_port_in_use_stops_without_shifting` | `PERSONACORE_PORT` の番号をずらす／B-76 の判定（`is_port_in_use`）が外れる |
//! | `a_whole_range_taken_stops_and_names_the_range` | 範囲が全部塞がっているのに起きる／範囲（元の番号〜+10）を言わない／範囲の外まで探す |
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

/// **空いている番号を1つ**（OS が自分では配らない帯から / [`hold_quiet_range`]。掴まずに返す）。
fn quiet_free_port() -> u16 {
    let (held, first) = hold_quiet_range();
    drop(held);
    first
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
///
/// # 番号は OS が配らない帯から取る（作業単位 F-11b）
///
/// **`saved` / `default` の腕は、取られていても止まらずに次の空きへずれる**（ADR-0057 §決定1）ので、上の
/// 起こし直しに入らず、ここで待つ番号には別の誰かが答える。**OS に選ばせた番号は、並んで走る他の腕が次に
/// 受け取る番号の隣**（Windows は順に配る / [`hold_quiet_range`] の doc）なので、取られうる。OS が配らない帯の
/// 番号は、このファイルの腕どうしでしか取り合わない。
async fn serve(dir: &Path, source: Source, startup: impl Fn(Startup) -> Startup) -> Serving {
    for _ in 0..MAX_BINDS {
        let port = quiet_free_port();
        let port_text = port.to_string();
        let lookup = move |key: &str| match key {
            ENV_LLM_ENDPOINT => Some(CLOSED_ENDPOINT.to_owned()),
            ENV_PORT if matches!(source, Source::Environment) => Some(port_text.clone()),
            _ => None,
        };
        let saved = match source {
            Source::Saved => Some(SavedPort::new(port).expect("前提: 帯の番号は 1024 以上")),
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
///
/// # ⚠ 「設定の番号を言う実装」は、このテストでは止まらない
///
/// **どの腕も、設定の番号（`config.bind()`）と実際に開いた番号が同じ**（`default` の腕も `with_bind` で
/// 開く番号そのものへ差し替えている）。だから `Listening::new` に設定の番号を渡す実装は、ここでは等価で緑のまま。
/// **それを止めるのは `tests/listening_port_cli.rs` の `PERSONACORE_PORT=0` の腕**
/// （`an_overridden_profile_file_neither_reads_nor_writes_the_saved_port` /
/// `an_unreadable_saved_port_warns_and_the_file_is_kept`）——設定は `0`、実際は OS が選んだ番号なので食い違う。
/// 実測（変異 M11）は `<measurements>\f12a-saved-port\MUTATIONS.md`。
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
                // **ずらしていない**（作業単位 F-11b。元の番号＝実際の番号、飛ばした番号は無い）。
                "listening_port_requested": server.port,
                "listening_port_shifted": false,
                "listening_port_skipped": [],
                // **保存のファイルは無い**（起動の設定に渡した保存は、ファイルではない）。
                "saved_port": null,
                "saved_port_unreadable": false,
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

    // **保存する番号は、いまの待ち受けとも既定とも、後で手で書く番号とも違う**（取り違えが観測できるように）。
    // **このテストは bind しない**ので空いている番号を取らず、定数にする（`saved_port_in_use` の確かめで
    // サーバが一瞬 bind を試すが、このテストはその結果を見ない —— 見る腕は `in_use_is_said_only_…`）。
    let wanted: u16 = 9101;
    let edited: u16 = 9202;
    assert_ne!(wanted, server.port, "前提");
    assert_ne!(wanted, DEFAULT_PORT, "前提");
    assert_ne!(edited, wanted, "前提");
    assert_ne!(edited, server.port, "前提");

    let (status, saved) = save(server.port, &json!({ "port": wanted })).await;
    assert_eq!(status, 200, "{saved}");
    assert_eq!(
        saved["listening_port"],
        json!(server.port),
        "待ち受けが移っている: {saved}"
    );
    assert_eq!(saved["listening_port_source"], "environment", "{saved}");
    assert_eq!(saved["saved_port"], json!(wanted), "{saved}");
    assert_eq!(saved["saved_port_unreadable"], json!(false), "{saved}");
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
    std::fs::write(&file, format!("\u{feff}{edited}\r\n")).expect("書けること");
    let reread = report(server.port).await;
    assert_eq!(reread["saved_port"], json!(edited), "{reread}");
    assert_eq!(reread["saved_port_unreadable"], json!(false), "{reread}");

    // **在るのに読めないファイルは `saved_port_unreadable: true`**（「保存なし」と見分ける）。
    // 理由ごとに1つ（範囲の外・数でない・2行以上・ディレクトリ＝I/O の失敗）。
    for (arm, text) in [
        ("範囲の外", Some("1023\n")),
        ("数でない", Some("eighty-ninety\n")),
        ("2行", Some("8090\n8091\n")),
        ("ディレクトリ", None),
    ] {
        match text {
            Some(text) => std::fs::write(&file, text).expect("書けること"),
            None => {
                std::fs::remove_file(&file).expect("消せること");
                std::fs::create_dir(&file).expect("ディレクトリを置けること");
            }
        }
        let broken = report(server.port).await;
        assert_eq!(
            broken["saved_port_unreadable"],
            json!(true),
            "{arm}: {broken}"
        );
        assert_eq!(broken["saved_port"], Value::Null, "{arm}: {broken}");
        assert_eq!(broken["next_port_source"], "default", "{arm}: {broken}");
    }
    std::fs::remove_dir(&file).expect("置いたディレクトリを消せること");
    // **空のファイルは「保存なし」で、壊れてはいない。**
    std::fs::write(&file, "").expect("書けること");
    let empty = report(server.port).await;
    assert_eq!(empty["saved_port_unreadable"], json!(false), "{empty}");
    assert_eq!(empty["saved_port"], Value::Null, "{empty}");

    let (status, forgot) = save(server.port, &json!({ "port": null })).await;
    assert_eq!(status, 200, "{forgot}");
    assert_eq!(
        forgot,
        json!({
            "listening_port": server.port,
            "listening_port_source": "environment",
            "listening_port_requested": server.port,
            "listening_port_shifted": false,
            "listening_port_skipped": [],
            "saved_port": null,
            "saved_port_unreadable": false,
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

    // **壊れたファイルを置いても `saved_port_unreadable` は `false`**——読まないのだから、壊れているとも言わない。
    // （`a_saved_port_reads_back_and_null_forgets_it` の同じ中身が、編集できる置き場所では `true` になる対照。）
    std::fs::write(&file, "eighty-ninety\n").expect("書けること");
    let broken = report(server.port).await;
    assert_eq!(
        broken["saved_port_unreadable"],
        json!(false),
        "読むだけの置き場所で保存を読んでいる: {broken}"
    );
    assert_eq!(broken["saved_port"], Value::Null, "{broken}");

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

// ----------------------------------------------------------------------
// ずらし（作業単位 F-11b / ADR-0057 §決定1・3・4）
// ----------------------------------------------------------------------
//
// **ここの腕は `App::bind` で起こす**（`App::run_with_upstreams` は実際の番号を返さない。ずれた先の番号は
// `App::local_addr` でしか知れない）。**塞ぐ番号は全部このテストが掴んだもの**で、OS が自分では配らない帯
// （[`hold_quiet_range`]）から選ぶ。**利用者の 8080 には触らない。**

/// **範囲（元の番号 +10）の幅の写し**（`port_search::PORT_SHIFT_SPAN`）。**定数から作らない。**
const SPAN: u16 = 10;

/// **OS が自分では配らない番号の帯（20000〜28999）で、続いた 11 個（元の番号〜+10）の空きを全部掴む。**
///
/// # なぜ OS に選ばせた番号の近くを使わないのか（作業単位 F-11b で実測）
///
/// **Windows は `127.0.0.1:0` の番号を、プロセスをまたいで1つずつ順に配る**（2回続けて8個ずつ取ると
/// `61545〜61552`、`61553〜61560`）。同じテストの実行ファイルで並んで走る他の腕は、OS に選ばせた番号を
/// 放して子やサーバに開かせるので、**その近くの 11 個を掴むと、他の腕の番号を奪う**（実測: 範囲を全部
/// 塞ぐ腕と並んだ「保存した番号がずれる」腕の子が、範囲を使い切って止まった）。動的な範囲
/// （Windows 49152〜65535・Linux 32768〜60999）の外なら、OS に選ばせる腕とは重ならない。
///
/// 帯の中の始まりは、プロセス番号と呼んだ回数から散らす（同じ実行ファイルの腕どうし・並んだ実行どうしを分ける）。
/// 掴めなかった（使われている・Windows の除外範囲）ら、別の始まりで取り直す。
fn hold_quiet_range() -> (Vec<std::net::TcpListener>, u16) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CALLS: AtomicU32 = AtomicU32::new(0);
    let seed = std::process::id().wrapping_mul(2_654_435_761);
    for _ in 0..200 {
        let call = CALLS.fetch_add(1, Ordering::Relaxed);
        let offset = seed.wrapping_add(call.wrapping_mul(7_919)) % 9_000;
        let first = 20_000 + u16::try_from(offset).expect("9000 未満であること");
        let mut held = Vec::new();
        for port in first..=first + SPAN {
            match std::net::TcpListener::bind(("127.0.0.1", port)) {
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
fn hold_a_free_port() -> (std::net::TcpListener, u16) {
    let (mut held, first) = hold_quiet_range();
    let kept = held.swap_remove(0);
    drop(held);
    (kept, first)
}

/// **元の番号から +10 までの 11 個を、全部このテストが掴む**（範囲が全部塞がった機を作る）。
fn hold_a_whole_range() -> (Vec<std::net::TcpListener>, u16) {
    hold_quiet_range()
}

/// **出どころと番号を決めて設定を組む**（DB は一時ディレクトリ。上流は閉じたポート）。
fn config_on(dir: &Path, source: Source, port: u16) -> AppConfig {
    let port_text = port.to_string();
    let lookup = move |key: &str| match key {
        ENV_LLM_ENDPOINT => Some(CLOSED_ENDPOINT.to_owned()),
        ENV_PORT if matches!(source, Source::Environment) => Some(port_text.clone()),
        _ => None,
    };
    let saved = match source {
        Source::Saved => Some(SavedPort::new(port).expect("前提: 1024 以上")),
        Source::Environment | Source::Default => None,
    };
    let config =
        AppConfig::from_lookup_with_data_dir_and_saved_port(lookup, Some(dir.to_path_buf()), saved)
            .expect("設定を組めること")
            .with_database_path(dir.join("db").join("personacore.sqlite"));
    match source {
        Source::Default => config.with_bind(BindAddr::loopback(port)),
        Source::Environment | Source::Saved => config,
    }
}

/// **`App::bind` で起こし、実際の番号で答えるまで待つ。**
async fn serve_bound(config: &AppConfig) -> Result<Serving, ServerError> {
    let app = App::bind(config).await?;
    let port = app.local_addr().port();
    let (shutdown, wait) = oneshot::channel();
    let task = tokio::spawn(async move {
        app.serve(async move {
            let _ = wait.await;
        })
        .await
    });
    let serving = Serving {
        port,
        shutdown: Some(shutdown),
        task: Some(task),
    };
    assert!(serving.answers().await, "起こしたサーバが答えない");
    Ok(serving)
}

/// **保存した番号が塞がっていると、面の番号を飛ばして次の空きで待ち受け、`GET /admin/server` が
/// 元の番号・ずらしたこと・飛ばした番号を言う**（ADR-0057 §決定3・4）。
///
/// 保存した番号 `P` を塞ぎ、`P+1` を面の番号として渡す。**`P+1` は空いていても開きに行かない**ので、
/// 待ち受けは `P+2` 以降（`P+2` を他の誰かが掴んでいればさらに先）。**飛ばした番号は `P+1` だけ。**
/// **見本の `upstreams.json` の形**（既定の 8080 が塞がると、llama の面の 8081 を飛ばして 8082 になる）を、
/// 利用者の 8080 に触らずに作ったもの。
#[tokio::test]
async fn a_taken_saved_port_shifts_past_the_face_ports_and_the_report_says_so() {
    assert_no_port_in_this_process();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (blocker, saved) = hold_a_free_port();
    let face = saved + 1;
    let config = config_on(dir.path(), Source::Saved, saved).with_face_ports([face, 1]);

    let server = serve_bound(&config)
        .await
        .expect("保存した番号が塞がっていても、ずらして起きること");
    assert!(
        server.port > face && server.port <= saved + SPAN,
        "面の番号を飛ばした範囲の中で待ち受けていない（保存 {saved}・面 {face}・実際 {}）",
        server.port
    );
    assert_eq!(
        report(server.port).await,
        json!({
            "listening_port": server.port,
            "listening_port_source": "saved",
            "listening_port_requested": saved,
            "listening_port_shifted": true,
            "listening_port_skipped": [face],
            "saved_port": null,
            "saved_port_unreadable": false,
            "next_port": DEFAULT_PORT,
            "next_port_source": "default",
            "editable": false,
        })
    );
    server.stop().await;
    drop(blocker);
}

/// **明示していなければ、面が使う元の番号は空いていても開かない**（ADR-0057 §決定3「元の番号も」）。
///
/// 出どころは `default`（`with_bind` で番号だけを差し替える）。元の番号 `P` は空いている（掴んですぐ放す）。
#[tokio::test]
async fn a_requested_port_that_a_face_uses_is_skipped_even_when_free() {
    assert_no_port_in_this_process();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (probe, requested) = hold_a_free_port();
    drop(probe);
    let config = config_on(dir.path(), Source::Default, requested).with_face_ports([requested]);

    let server = serve_bound(&config).await.expect("起きること");
    assert_ne!(server.port, requested, "面が使う元の番号で待ち受けた");
    let body = report(server.port).await;
    assert_eq!(body["listening_port_requested"], json!(requested), "{body}");
    assert_eq!(body["listening_port_source"], "default", "{body}");
    assert_eq!(body["listening_port_shifted"], json!(true), "{body}");
    assert_eq!(body["listening_port_skipped"], json!([requested]), "{body}");
    server.stop().await;
}

/// **`PERSONACORE_PORT` の番号は、塞がっていてもずらさずに止まる**（ADR-0057 §決定1）。
///
/// **止まり方は今までどおり**（`ServerError::is_port_in_use` が真 —— `main` はこれで B-76 の文を足す）。
/// **面の番号を渡しても見ない**（明示の番号は面と同じなら B-52 が止める）。
#[tokio::test]
async fn an_explicit_port_in_use_stops_without_shifting() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (blocker, port) = hold_a_free_port();
    let config = config_on(dir.path(), Source::Environment, port).with_face_ports([port + 1]);

    let err = serve_bound(&config)
        .await
        .err()
        .expect("明示した番号が塞がっていれば止まること");
    assert!(err.is_port_in_use(), "B-76 の判定から外れた: {err:?}");
    assert_eq!(
        err.tried_ports(),
        None,
        "明示の番号で範囲を言っている: {err:?}"
    );
    assert_eq!(err.to_string(), "待ち受けアドレスを開けませんでした");
    drop(blocker);
}

/// **範囲が全部塞がっていると、範囲（元の番号〜+10）を言って止まる**（ADR-0057 §決定3）。
///
/// **11 個とも、このテストが掴んだもの**（`+11` は塞がない —— 範囲の外まで探す実装なら、そこで起きて赤）。
#[tokio::test]
async fn a_whole_range_taken_stops_and_names_the_range() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (held, first) = hold_a_whole_range();
    for source in [Source::Saved, Source::Default] {
        let config = config_on(dir.path(), source, first);
        let err = serve_bound(&config)
            .await
            .err()
            .expect("範囲が全部塞がっていれば止まること");
        assert_eq!(
            err.tried_ports(),
            Some((first, first + SPAN)),
            "試した範囲を言っていない: {err:?}"
        );
        assert!(
            !err.is_port_in_use(),
            "範囲の失敗を、明示の番号の失敗（B-76 の1つの番号の文）として扱っている: {err:?}"
        );
        assert_eq!(err.to_string(), "待ち受けアドレスを開けませんでした");
        // **探すのは `Store::connect` より前**（`App::bind` の doc の約束）——止まった起動は DB の置き場所すら作らない。
        assert!(
            !dir.path().join("db").exists(),
            "範囲を使い切って止まったのに、DB の置き場所を作っている"
        );
    }
    drop(held);
}
