//! `server` の起動 / graceful shutdown / `GET /v1/models` の検証。
//!
//! **LM Studio には繋がない。** `/v1/models` はキャラクターを並べるだけで
//! 上流を必要としない（ADR-0004）。ここで固定するのは主に次の5点:
//!
//!   * `/v1/models` が **キャラクターのみ**を OpenAI 互換の形で返すこと（0件でも空の一覧）。
//!   * **停止時に `Store::close()` を必ず通る**こと（WAL のデータは `-wal` 側に残るため）。
//!     **待ち受けに失敗した経路では、そもそも DB を開かない**（閉じ忘れる余地を無くす）。
//!   * **ループバック以外に待ち受けない**こと（絶対制約）。
//!   * エラーレスポンスに**内部情報を載せない**こと（`{err:?}` を含めない）。
//!   * 未知のパス・未対応のメソッドも OpenAI 互換のエラー形で返すこと。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use personacore::domain::{DisplayName, Handle, Persona};
use personacore::server::{App, AppConfig, BindAddr, ServerError, shutdown_signal};
use personacore::store::{Store, StoreError};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

mod common;

use common::{CLOCK_SKEW, fixture_scale};

/// 一時ディレクトリ上の DB パスで、既定値だけの設定を組み立てる。**ポートは OS 任せ。**
fn config_for(db_path: &Path) -> AppConfig {
    AppConfig::from_lookup(|_| None)
        .expect("既定値だけで設定を組み立てられること")
        // ポート 0 は OS に空きを選ばせる。**アドレスは指定できない**（常にループバック）。
        .with_bind(BindAddr::loopback(0))
        .with_database_path(db_path)
}

/// SQLite の副次ファイル（`-wal` / `-shm`）のパス。
///
/// **接続が開いている間だけ存在し、最後の接続が閉じるときに片付けられる。**
/// つまり `Store::close()` を通ったかどうかを外から観測できる。
fn side_files(db_path: &Path) -> [PathBuf; 2] {
    let base = db_path.display().to_string();
    [
        PathBuf::from(format!("{base}-wal")),
        PathBuf::from(format!("{base}-shm")),
    ]
}

/// `-wal` / `-shm` が残っていないこと（= プールを閉じたこと）。
fn assert_store_is_closed(db_path: &Path, context: &str) {
    assert!(
        db_path.exists(),
        "{context}: 前提として DB ファイルは作られていること: {}",
        db_path.display()
    );
    for side in side_files(db_path) {
        assert!(
            !side.exists(),
            "{context}: Store を閉じていれば {} は残らない",
            side.display()
        );
    }
}

/// **DB を一度も開いていない**こと（本体ファイルすら作られていない）。
///
/// 「閉じ忘れていない」より強い保証。`Store::connect` は `create_if_missing(true)` なので、
/// 開いた形跡があればファイルが残る。
fn assert_store_was_never_opened(db_path: &Path, context: &str) {
    assert!(
        !db_path.exists(),
        "{context}: DB を開かずに抜けること（閉じ忘れる余地を無くす）: {}",
        db_path.display()
    );
    for side in side_files(db_path) {
        assert!(
            !side.exists(),
            "{context}: {} も作られないこと",
            side.display()
        );
    }
}

/// エフェメラルポートで起動した実サーバ。
struct TestServer {
    addr: SocketAddr,
    /// アプリと**同じプールを指す**ハンドル。停止後の状態を観測するために持つ。
    store: Store,
    /// DB ファイルの絶対パス。エラーレスポンスに漏れていないことの確認に使う。
    db_path: String,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    /// テスト終了まで DB を生かしておくための保持。
    _dir: TempDir,
}

impl TestServer {
    /// 一時ディレクトリ上の新規 DB でサーバを起動する。**ポートは OS 任せ。**
    async fn start() -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db_path = dir.path().join("personacore.sqlite");

        let config = config_for(&db_path);

        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let store = app.store().clone();

        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                // 送信元が落ちた場合も停止する（テストが panic しても宙吊りにしない）。
                let _ = rx.await;
            })
            .await
        });

        Self {
            addr,
            store,
            db_path: db_path.display().to_string(),
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    /// GET してステータスと本文を返す。
    async fn get(&self, path: &str) -> (reqwest::StatusCode, String) {
        let response = reqwest::get(format!("http://{}{path}", self.addr))
            .await
            .expect("サーバへ接続できること");
        let status = response.status();
        let body = response.text().await.expect("本文を読めること");
        (status, body)
    }

    /// POST してステータスと本文を返す（メソッド不一致の確認に使う）。
    async fn post(&self, path: &str) -> (reqwest::StatusCode, String) {
        let response = reqwest::Client::new()
            .post(format!("http://{}{path}", self.addr))
            .send()
            .await
            .expect("サーバへ接続できること");
        let status = response.status();
        let body = response.text().await.expect("本文を読めること");
        (status, body)
    }

    /// 停止シグナルを送り、サーバタスクの正常終了を待つ。
    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).expect("停止シグナルを送れること");
        }
        self.handle
            .take()
            .expect("起動時のタスクがあること")
            .await
            .expect("サーバタスクが panic しないこと")
            .expect("正常に終了すること");
    }
}

fn name(raw: &str) -> Handle {
    Handle::parse(raw).expect("テストで使う名前は正当であること")
}

/// 表示名。**識別子とは別の文字列を渡す**（`CHARACTERS` の doc を参照）。
fn display(raw: &str) -> DisplayName {
    DisplayName::parse(raw).expect("テストで使う表示名は正当であること")
}

/// 一覧テストで投入するキャラクター。`(識別子, 表示名)`。
///
/// **識別子と表示名は必ず別の値にする**（ADR-0011 / 0004a のレビュー指摘）。
/// 同じ文字列にすると、`/v1/models` が `handle` ではなく `display_name` を並べるように
/// 壊してもアサートが区別できず、**変異が緑を通り抜ける**（実測: `models.rs` を
/// `display_name` に潰しても全件緑だった）。
///
/// 表示名を並べてはならない理由は「見た目の問題」ではない。
/// **`display_name` は一意ではない**ので、一覧が「葵」「葵」の2件になり、
/// クライアントがそれを `model` に送っても `find_character_by_handle` は完全一致で
/// 引けず **404** になる。「一覧に出ているのに使えない」という、
/// ADR-0004 が名指しした袋小路そのもの。
///
/// **表示名は識別子のどれとも重ならない値にすること。** 重なっていると
/// 「表示名が本文に出ていないこと」のアサートが正当な `id` に反応してしまう。
const CHARACTERS: [(&str, &str); 3] = [("rin", "凛乃"), ("s10", "碧"), ("aoi", "アオイ")];

fn persona(raw: &str) -> Persona {
    Persona::parse(raw).expect("テストで使うペルソナは正当であること")
}

/// 本文を JSON として読む。
fn parse_json(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|err| panic!("JSON として読めること: {err} / {body}"))
}

/// キャラクターが1件も無ければ**空の一覧**（エラーにしない）。
#[tokio::test]
async fn models_returns_an_empty_list_without_characters() {
    let server = TestServer::start().await;

    let (status, body) = server.get("/v1/models").await;

    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(
        parse_json(&body),
        json!({"object": "list", "data": []}),
        "0件でも一覧として成立すること"
    );

    server.stop().await;
}

/// **キャラクターのみ**が名前順に並ぶ（セッションは並べない / ADR-0004）。
///
/// 並ぶのは**識別子**であって表示名ではない。理由は `CHARACTERS` の doc を参照。
#[tokio::test]
async fn models_lists_characters_in_handle_order() {
    let server = TestServer::start().await;
    let before = Utc::now() - CLOCK_SKEW;

    for (raw_name, raw_display) in CHARACTERS {
        assert_ne!(
            raw_name, raw_display,
            "識別子と表示名は別の値にすること（同値だとどちらが並んだか区別できない）"
        );
        server
            .store
            .create_character(
                &name(raw_name),
                &display(raw_display),
                &persona("ペルソナ本文"),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");
    }

    let (status, body) = server.get("/v1/models").await;
    assert_eq!(status, reqwest::StatusCode::OK);

    let value = parse_json(&body);
    assert_eq!(value["object"], json!("list"));

    let data = value["data"].as_array().expect("data は配列であること");
    let ids: Vec<&str> = data
        .iter()
        .map(|entry| entry["id"].as_str().expect("id は文字列であること"))
        .collect();
    assert_eq!(
        ids,
        vec!["aoi", "rin", "s10"],
        "挿入順ではなく名前順に並ぶこと / 並ぶのは識別子であること: {body}"
    );

    // **表示名は一覧のどこにも出さない。** `id` だけでなく応答全体を見る
    // （`id` は識別子のまま、別フィールドで表示名を足す形も塞ぐ）。
    for (_, raw_display) in CHARACTERS {
        assert!(
            !body.contains(raw_display),
            "表示名 {raw_display:?} を一覧に出さないこと\
             （display_name は一意でないため、並べると model 指定で引けなくなる）: {body}"
        );
    }

    for entry in data {
        assert_eq!(entry["object"], json!("model"), "OpenAI 互換の形であること");
        let created = entry["created"].as_i64().expect("created は整数であること");
        assert!(
            created >= before.timestamp() && created <= (Utc::now() + CLOCK_SKEW).timestamp(),
            "created に妥当な作成日時が入ること: {created}"
        );
        assert!(
            entry.get("owned_by").is_some(),
            "owned_by を落とさないこと: {entry}"
        );
    }

    server.stop().await;
}

/// **停止時に `Store::close()` を通る。**
///
/// WAL のデータは `-wal` 側に残るため、閉じずに落とすと失われる。
/// 「閉じたこと」はアプリと同じプールを指すハンドルから観測する
/// （閉じたプールへの問い合わせは `sqlx::Error::PoolClosed` になる）。
#[tokio::test]
async fn graceful_shutdown_closes_the_store() {
    let server = TestServer::start().await;
    let store = server.store.clone();

    store
        .list_characters()
        .await
        .expect("稼働中はプールが開いていること");

    server.stop().await;

    let err = store
        .list_characters()
        .await
        .expect_err("停止後はプールが閉じていること");
    match &err {
        StoreError::Query(inner) => assert!(
            matches!(inner, sqlx::Error::PoolClosed),
            "プールが閉じていること（実際のエラー = {inner}）"
        ),
        other => panic!("Query エラーを期待したが {other:?} だった"),
    }
}

/// **ループバック以外には待ち受けない**（絶対制約）。
#[tokio::test]
async fn listener_is_bound_to_loopback_only() {
    let server = TestServer::start().await;

    assert!(
        server.addr.ip().is_loopback(),
        "待ち受けアドレスがループバックであること: {}",
        server.addr
    );
    assert_eq!(
        server.addr.ip().to_string(),
        "127.0.0.1",
        "外部から到達できるアドレスに開かないこと: {}",
        server.addr
    );
    assert_ne!(server.addr.port(), 0, "実際のポートが解決されていること");

    server.stop().await;
}

/// エラーレスポンスは**定数文字列だけ**を載せる（`{err:?}` を含めない）。
///
/// 稼働中に DB を落として 500 を作る。パス・接続文字列・内部型名が本文に出ないこと。
#[tokio::test]
async fn store_failure_does_not_leak_internal_details() {
    let server = TestServer::start().await;

    // 稼働中にプールを閉じ、`list_characters` を確実に失敗させる。
    server.store.close().await;

    let (status, body) = server.get("/v1/models").await;

    assert_eq!(status, reqwest::StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        parse_json(&body),
        json!({"error": {"message": "内部エラーが発生しました", "type": "server_error"}}),
        "本文は定数のみであること"
    );
    for leak in [
        server.db_path.as_str(),
        "sqlite",
        "sqlx",
        "PoolClosed",
        "personacore.sqlite",
        "src\\",
        "src/",
    ] {
        assert!(
            !body.contains(leak),
            "内部情報 {leak:?} が漏れていないこと: {body}"
        );
    }

    server.stop().await;
}

/// 未知のパスも OpenAI 互換のエラー形で返す（HTML やスタックトレースを返さない）。
#[tokio::test]
async fn unknown_route_returns_an_openai_shaped_error() {
    let server = TestServer::start().await;

    let (status, body) = server.get("/v1/does-not-exist").await;

    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    assert_eq!(
        parse_json(&body),
        json!({
            "error": {
                "message": "指定されたエンドポイントは存在しません",
                "type": "invalid_request_error",
            }
        }),
        "リクエスト内容を反射しないこと"
    );
    assert!(
        !body.contains("does-not-exist"),
        "パスを反射しないこと: {body}"
    );

    server.stop().await;
}

/// **メソッド不一致（405）も OpenAI 互換のエラー形で返す。**
///
/// `Router::fallback` は未一致の**パス**にしか効かないので、
/// 405 だけが既定の空ボディのまま残りやすい。JSON 前提でエラーを読む
/// クライアントは、そこだけパースに失敗する。
#[tokio::test]
async fn method_mismatch_returns_an_openai_shaped_error() {
    let server = TestServer::start().await;

    let (status, body) = server.post("/v1/models").await;

    assert_eq!(status, reqwest::StatusCode::METHOD_NOT_ALLOWED);
    assert!(!body.is_empty(), "空ボディにしないこと");
    assert_eq!(
        parse_json(&body),
        json!({
            "error": {
                "message": "このエンドポイントは指定されたメソッドを受け付けません",
                "type": "invalid_request_error",
            }
        }),
        "リクエスト内容を反射しないこと"
    );

    // 存在しないパスは 405 ではなく 404 のまま（区別が潰れていない）。
    let (status, _) = server.post("/v1/does-not-exist").await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);

    server.stop().await;
}

/// **待ち受けに失敗した経路では、そもそも `Store` を開かない。**
///
/// `Store::connect` を `bind` の**最後**に置いているので、それより前で `?` を書いても
/// 閉じるべきものが存在しない。「閉じ忘れないよう分岐を足す」ではなく
/// 「閉じる必要が無い順序にする」形。
///
/// **この順序が戻ると（`Store::connect` を先頭に置き直すと）ここが落ちる。**
/// 逆順のままでも「閉じてから返す」分岐を書けば `-wal` は残らないので、
/// 副次ファイルだけを見るアサートでは検出できない。DB 本体の不在まで見る。
#[tokio::test]
async fn bind_failure_never_opens_the_store() {
    // 1台目にポートを占有させる。
    let occupied = TestServer::start().await;

    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let db_path = dir.path().join("second.sqlite");
    // **別の DB ファイル**を使う（1台目のプールと混ざらないようにする）。
    let config = config_for(&db_path).with_bind(BindAddr::loopback(occupied.addr.port()));

    let err = App::bind(&config)
        .await
        .expect_err("使用中のポートは開けないこと");
    assert!(
        matches!(err, ServerError::Bind(_)),
        "待ち受けの失敗として返すこと: {err:?}"
    );

    assert_store_was_never_opened(&db_path, "待ち受けに失敗した場合");

    occupied.stop().await;
}

/// **`App::run` は待ち受けから停止までをまとめて持つ。**
///
/// `bind` と `serve` の間で早期 return を書ける形にしておくと、
/// 待ち受け済みの `App` を落としたまま関数を抜ける経路が生まれる
/// （= `Store` が閉じられない）。`main` が `App` を手に持たない形にして、
/// その経路を構造的に作れなくする。
#[tokio::test]
async fn run_binds_and_closes_the_store() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let db_path = dir.path().join("run.sqlite");
    let config = config_for(&db_path);

    // 既に解決済みの停止シグナル。待ち受け開始後、すぐ graceful shutdown に入る。
    App::run(&config, std::future::ready(()))
        .await
        .expect("起動から停止まで通ること");

    assert_store_is_closed(&db_path, "run が正常終了した場合");
}

/// **停止シグナルは「来るまで待つ」。**
///
/// Windows では `Ctrl+C` / `Ctrl+Break` / コンソール終了 / ログオフ / シャットダウンを
/// それぞれ別に登録する。**どれか1つでも登録に失敗した経路を即座に解決させると、
/// 起動した瞬間にサーバが止まる。** 登録できなかった経路は永久に待たせる。
///
/// 実際のシグナル送出はテストプロセス自身を落とすので確認できない。
/// ここで見るのは「勝手に解決しないこと」の1点。
#[tokio::test]
async fn shutdown_signal_waits_for_an_actual_signal() {
    let result = tokio::time::timeout(Duration::from_millis(200), shutdown_signal()).await;

    assert!(
        result.is_err(),
        "シグナルが来ていないのに停止処理へ進まないこと"
    );
}

/// `App::run` の失敗も**握り潰さず**返し、`Store` を閉じる。
#[tokio::test]
async fn run_reports_bind_failure() {
    let occupied = TestServer::start().await;

    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let db_path = dir.path().join("run-conflict.sqlite");
    let config = config_for(&db_path).with_bind(BindAddr::loopback(occupied.addr.port()));

    let err = App::run(&config, std::future::ready(()))
        .await
        .expect_err("使用中のポートは開けないこと");
    assert!(
        matches!(err, ServerError::Bind(_)),
        "待ち受けの失敗として返すこと: {err:?}"
    );

    assert_store_was_never_opened(&db_path, "run が待ち受けに失敗した場合");

    occupied.stop().await;
}
