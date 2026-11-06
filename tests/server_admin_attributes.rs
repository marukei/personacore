//! `/admin/characters/{handle}/attributes` の検証（ADR-0009 / 作業単位 12a）。
//!
//! **LM Studio には繋がない。** 管理 API は上流を必要としない。
//! **サーバは本番の入口（`App::bind`）から起動する**——テスト専用の入口を作ると、
//! 本番の配線が誰にも通られないまま残る（CLAUDE.md「テスト専用の入口を作ると…」）。
//!
//! ここで固定するのは主に次の7点:
//!
//!   * **往復**——`PUT` した属性が `GET` でそのまま返る（`subject` と並びを含む）。
//!   * **`subject` を落とさない**——同じキーを `user` と `character` の両方に持つ
//!     キャラクターで測る（片方しか持たないフィクスチャでは取り違えが検出できない）。
//!   * **丸ごと置き換え**——要求に含まれない属性は消える。
//!   * **件数の上限**——8 件は通り、9 件は 400 で**1行も書かれない**。
//!   * **キー・値の上限と空**——理由ごとに区別できるエラーになる。
//!   * **壊れた行の復旧が HTTP で1本の線として繋がる**——`GET` が 500 になっても
//!     `PUT` で直せる。
//!   * エラーレスポンスに**内部情報を載せない**（本文を完全一致で固定する）。

use std::net::SocketAddr;

use personacore::server::{App, AppConfig, BindAddr, ServerError};
use personacore::store::Store;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

mod common;

use common::{
    insert_attribute_revision_ignoring_checks, insert_character_attribute_ignoring_checks,
};

/// エフェメラルポートで起動した実サーバ。
struct TestServer {
    addr: SocketAddr,
    /// アプリと**同じプールを指す**ハンドル。壊れた行の投入に使う。
    store: Store,
    /// DB ファイルの絶対パス。エラーレスポンスに漏れていないことの確認に使う。
    db_path: String,
    client: reqwest::Client,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    /// **本番の入口（`App::bind`）から起動する。**
    async fn start() -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db_path = dir.path().join("personacore.sqlite");

        let config = AppConfig::from_lookup(|_| None)
            .expect("既定値だけで設定を組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(&db_path);

        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let store = app.store().clone();

        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });

        Self {
            addr,
            store,
            db_path: db_path.display().to_string(),
            client: reqwest::Client::new(),
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    async fn send(&self, method: Method, path: &str, body: Option<&Value>) -> (StatusCode, Value) {
        let mut request = self
            .client
            .request(method, format!("http://{}{path}", self.addr));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.expect("サーバへ接続できること");
        let status = response.status();
        let text = response.text().await.expect("本文を読めること");
        let value = serde_json::from_str(&text)
            .unwrap_or_else(|err| panic!("応答が JSON であること: {text}: {err}"));
        (status, value)
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send(Method::GET, path, None).await
    }

    async fn put(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        self.send(Method::PUT, path, Some(body)).await
    }

    async fn post(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        self.send(Method::POST, path, Some(body)).await
    }

    /// キャラクターを1件作る。**識別子と表示名は違う文字列。**
    async fn create_character(&self, handle: &str, display_name: &str) {
        assert_ne!(handle, display_name, "識別子と表示名を同値にしないこと");
        let (status, body) = self
            .post(
                "/admin/characters",
                &json!({
                    "handle": handle,
                    "display_name": display_name,
                    "base_persona": "ペルソナ本文",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "作成できること: {body}");
    }

    /// `character_attribute` の行数。
    async fn attribute_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM character_attribute")
            .fetch_one(self.store.pool())
            .await
            .expect("行数を数えられること")
    }

    /// キャラクターの主キー（壊れた行を直接入れるために要る）。
    async fn character_id(&self, handle: &str) -> i64 {
        sqlx::query_scalar("SELECT id FROM character WHERE handle = ?")
            .bind(handle)
            .fetch_one(self.store.pool())
            .await
            .expect("id を引けること")
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).expect("停止シグナルを送れること");
        }
        if let Some(handle) = self.handle.take() {
            handle
                .await
                .expect("サーバタスクが panic していないこと")
                .expect("サーバが正常終了すること");
        }
    }
}

/// **このファイルの基準フィクスチャ**（`tests/store_attribute.rs` と同じ仕込み）。
///
/// | # | subject | key | value |
/// |---|---|---|---|
/// | 0 | character | 一人称 | 僕 |
/// | 1 | user | 年齢 | 32 |
/// | 2 | user | 呼び方 | 先輩 |
/// | 3 | character | 年齢 | 19 |
/// | 4 | character | 種族 | 猫獣人 |
///
/// * `user` と `character` の**両方**を持つ。
/// * 同じキー（`年齢`）を両方が持ち、**値が違う**。
/// * 並びと `subject` を相関させない（C, U, U, C, C）。
/// * キーと値が同じ文字列になっている行が1つも無い。
fn fixture() -> Value {
    json!([
        {"subject": "character", "key": "一人称", "value": "僕"},
        {"subject": "user", "key": "年齢", "value": "32"},
        {"subject": "user", "key": "呼び方", "value": "先輩"},
        {"subject": "character", "key": "年齢", "value": "19"},
        {"subject": "character", "key": "種族", "value": "猫獣人"},
    ])
}

/// エラー応答の本文を**完全一致**で組み立てる（内部情報が混ざったら赤になる）。
fn error_body(message: &str, code: &str) -> Value {
    json!({"error": {"message": message, "type": "invalid_request_error", "code": code}})
}

/// **往復。** `PUT` した属性が `GET` でそのまま返る。
#[tokio::test]
async fn attributes_round_trip_through_the_api() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": fixture()}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "置き換えが成功すること: {body}");
    assert_eq!(
        body,
        json!({"character": "aoi", "attributes": fixture()}),
        "応答が要求した属性をそのまま返すこと（並び・subject を含む）"
    );

    let (status, body) = server.get("/admin/characters/aoi/attributes").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"character": "aoi", "attributes": fixture()}),
        "読み直しても同じであること"
    );

    server.stop().await;
}

/// **`subject` の取り違えが赤になる。**
///
/// 同じキー（`年齢`）を `user` と `character` の両方が持ち、**値が違う**。
/// 応答から `subject` が落ちる／固定される変異は、ここで必ず崩れる。
#[tokio::test]
async fn the_subject_of_each_attribute_survives_the_api() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": fixture()}),
        )
        .await;

    let (_, body) = server.get("/admin/characters/aoi/attributes").await;
    let attributes = body["attributes"]
        .as_array()
        .expect("属性が配列で返ること")
        .clone();

    let ages: Vec<(&str, &str)> = attributes
        .iter()
        .filter(|attribute| attribute["key"] == "年齢")
        .map(|attribute| {
            (
                attribute["subject"].as_str().expect("subject があること"),
                attribute["value"].as_str().expect("value があること"),
            )
        })
        .collect();
    assert_eq!(
        ages,
        vec![("user", "32"), ("character", "19")],
        "同じキーでも主語で値が割れること"
    );

    // 主語ごとの件数も見る（`subject` を1つ固定する変異は、こちらでも崩れる）。
    let count = |subject: &str| {
        attributes
            .iter()
            .filter(|attribute| attribute["subject"] == subject)
            .count()
    };
    assert_eq!(count("user"), 2, "利用者の属性が2件であること");
    assert_eq!(count("character"), 3, "キャラクターの属性が3件であること");

    server.stop().await;
}

/// **丸ごと置き換え。** 要求に含まれない属性は消える。
#[tokio::test]
async fn replacing_attributes_is_wholesale() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": fixture()}),
        )
        .await;

    let replacement = json!([
        {"subject": "character", "key": "一人称", "value": "俺"},
        {"subject": "user", "key": "出身", "value": "北国"},
    ]);
    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": replacement}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["attributes"], replacement);
    assert_eq!(server.attribute_count().await, 2, "DB にも2行だけ残ること");

    // 空の配列で全消し。
    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": []}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"character": "aoi", "attributes": []}));
    assert_eq!(server.attribute_count().await, 0);

    server.stop().await;
}

/// **件数の上限。** 8 件は通り、9 件は 400 で**1行も書かれない。**
#[tokio::test]
async fn the_ninth_attribute_is_rejected_and_nothing_is_written() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let many = |count: usize| -> Value {
        Value::Array(
            (0..count)
                .map(|index| {
                    json!({
                        "subject": "character",
                        "key": format!("キー{index}"),
                        "value": format!("値{index}"),
                    })
                })
                .collect(),
        )
    };

    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": many(8)}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "8 件は通ること: {body}");
    assert_eq!(server.attribute_count().await, 8);

    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": many(9)}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "9 件は拒否されること");
    assert_eq!(
        body,
        error_body("属性の件数が多すぎます", "too_many_attributes"),
        "件数も上限も本文に載せないこと"
    );
    assert_eq!(
        server.attribute_count().await,
        8,
        "拒否された要求で1行も書き換わらないこと"
    );

    server.stop().await;
}

/// **キー・値の不備は理由ごとに区別できる。**
///
/// キーの上限超過と値の上限超過を**同じ応答に潰さない**——潰すと、
/// 受け取った側はどちらの欄を縮めればよいか分からない。
#[tokio::test]
async fn an_invalid_key_or_value_is_rejected_with_its_own_reason() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let long_key = "あ".repeat(13);
    let long_value = "い".repeat(21);
    let cases: [(Value, &str, &str); 6] = [
        (
            json!({"subject": "character", "key": "", "value": "僕"}),
            "属性のキーが空です",
            "attribute_key_empty",
        ),
        (
            json!({"subject": "character", "key": "一人\u{0007}称", "value": "僕"}),
            "属性のキーに改行や制御文字は使えません",
            "attribute_key_control",
        ),
        (
            json!({"subject": "character", "key": long_key, "value": "僕"}),
            "属性のキーが長すぎます",
            "attribute_key_too_long",
        ),
        (
            json!({"subject": "character", "key": "一人称", "value": "\u{3000}"}),
            "属性の値が空です",
            "attribute_value_empty",
        ),
        (
            json!({"subject": "character", "key": "一人称", "value": "僕\n俺"}),
            "属性の値に改行や制御文字は使えません",
            "attribute_value_control",
        ),
        (
            json!({"subject": "character", "key": "一人称", "value": long_value}),
            "属性の値が長すぎます",
            "attribute_value_too_long",
        ),
    ];

    for (attribute, message, code) in cases {
        let (status, body) = server
            .put(
                "/admin/characters/aoi/attributes",
                &json!({"attributes": [attribute.clone()]}),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{attribute}: {body}");
        assert_eq!(body, error_body(message, code), "要求 = {attribute}");
    }

    assert_eq!(
        server.attribute_count().await,
        0,
        "不正な要求で1行も書かれないこと"
    );

    // **キーの上限超過と値の上限超過は別の応答**（同じ長さで両方を試す）。
    let (_, as_key) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": [{"subject": "character", "key": long_key, "value": "僕"}]}),
        )
        .await;
    let (_, as_value) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": [{"subject": "character", "key": "一人称", "value": long_key}]}),
        )
        .await;
    assert_ne!(as_key, as_value, "キーと値のエラーを同じ応答に潰さないこと");
    assert_eq!(
        as_key["error"]["code"], "attribute_key_too_long",
        "13 文字はキーとしては長すぎること"
    );
    assert_eq!(
        as_value["error"],
        Value::Null,
        "同じ 13 文字が値としては通ること（上限が別である証拠）"
    );

    server.stop().await;
}

/// **キーの上限を超える長さでも、値としては通る**（上限が別であることの裏取り）。
#[tokio::test]
async fn a_value_may_be_longer_than_a_key() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let between = "う".repeat(13);
    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": [{"subject": "character", "key": "一人称", "value": between}]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "値としては通ること: {body}");

    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": [{"subject": "character", "key": between, "value": "僕"}]}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "キーとしては通らないこと");
    assert_eq!(
        body,
        error_body("属性のキーが長すぎます", "attribute_key_too_long")
    );

    server.stop().await;
}

/// **未知の `subject` は専用の理由で拒否する。**
///
/// `assistant` は `message.role` の綴りで、取り違えの実例。
/// 「本文の形式が不正」に潰すと、どこを直せばよいか分からない。
#[tokio::test]
async fn an_unknown_subject_is_rejected_with_its_own_reason() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    for subject in ["assistant", "User", "system", "", "CHARACTER"] {
        let (status, body) = server
            .put(
                "/admin/characters/aoi/attributes",
                &json!({"attributes": [{"subject": subject, "key": "一人称", "value": "僕"}]}),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "subject = {subject:?}");
        assert_eq!(
            body,
            error_body(
                "属性の主語は user か character のどちらかです",
                "attribute_subject_unknown"
            ),
            "subject = {subject:?}"
        );
    }

    // **綴りは `user` / `character` のちょうど2つ**（受理される側も見る）。
    for subject in ["user", "character"] {
        let (status, body) = server
            .put(
                "/admin/characters/aoi/attributes",
                &json!({"attributes": [{"subject": subject, "key": "一人称", "value": "僕"}]}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "subject = {subject}: {body}");
        assert_eq!(body["attributes"][0]["subject"], subject, "往復すること");
    }

    server.stop().await;
}

/// **同じ主語の同じキーを重複させたら 400。前の設定は残る。**
#[tokio::test]
async fn a_duplicate_attribute_is_rejected_and_the_previous_set_survives() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": fixture()}),
        )
        .await;

    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": [
                {"subject": "character", "key": "一人称", "value": "僕"},
                {"subject": "character", "key": " 一人称 ", "value": "俺"},
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        error_body(
            "同じ属性を重複して指定することはできません",
            "duplicate_attribute"
        )
    );

    let (_, body) = server.get("/admin/characters/aoi/attributes").await;
    assert_eq!(
        body["attributes"],
        fixture(),
        "巻き戻って前の設定が残ること"
    );

    // **主語が違えば重複ではない。**
    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": [
                {"subject": "user", "key": "年齢", "value": "32"},
                {"subject": "character", "key": "年齢", "value": "19"},
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    server.stop().await;
}

/// **存在しないキャラクターは 404**（`GET` / `PUT` の両方）。
#[tokio::test]
async fn attributes_of_an_unknown_character_are_not_found() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let expected = error_body(
        "指定されたキャラクターが存在しません",
        "character_not_found",
    );
    let (status, body) = server.get("/admin/characters/rin/attributes").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, expected);

    let (status, body) = server
        .put(
            "/admin/characters/rin/attributes",
            &json!({"attributes": fixture()}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, expected);
    assert_eq!(server.attribute_count().await, 0, "1行も書かれないこと");

    server.stop().await;
}

/// **不正な本文 × 未知のキャラクター → 400。** `PUT` は**本文の不備を優先する。**
///
/// # 何を固定しているか
///
/// `replace_attributes` が `attribute_set`（本文の検証）を
/// `find_character`（宛先の照合）**より先に**呼ぶこと。
/// **2つを入れ替えると同じ要求が 404 になる**（実測。着手前はこの入れ替えが
/// 948 全緑を通り抜けた）。
///
/// `attributes_of_an_unknown_character_are_not_found` は**正当な本文**を
/// 未知のキャラクターへ送るので、この順序に依存しない。
/// **「不正な本文 × 未知のキャラクター」の組み合わせだけがこの決定を観測できる。**
///
/// # なぜ 400 でなければならないか
///
/// 識別子を打ち間違えた利用者が `subject` も間違えていると、404 を先に返す実装では
/// **本文を直しても 404 が返り続ける**（直すべきものが2つあるのに、片方しか見えない）。
/// 宛先が正しくても本文は直さなければならないので、先に本文の理由を返す。
///
/// # この決定の境界 —— **すべてが 400 になるわけではない**
///
/// 重複（同じ `(subject, key)`）は `AttributeSet::parse` では見ず、
/// **DB の主キー違反として `store` が返す**（`AttributeSet` の doc「重複は見ない」）。
/// `store` を叩くには `character.id` が要るので、**重複は宛先の方が先に効いて 404** になる。
/// ここを併せて固定してあるので、**「常に 400 を返す」実装でもこのテストは緑にならない。**
#[tokio::test]
async fn an_invalid_body_for_an_unknown_character_is_a_bad_request() {
    let server = TestServer::start().await;
    // **存在するキャラクターを1件は置く**（表が空だと「常に 404」と区別が付かない）。
    server.create_character("aoi", "葵").await;

    // 本文の不備の理由ごとに、**その理由**が返ること（404 に潰れないこと）。
    let cases: [(Value, &str, &str); 5] = [
        (
            json!([{"subject": "assistant", "key": "一人称", "value": "僕"}]),
            "属性の主語は user か character のどちらかです",
            "attribute_subject_unknown",
        ),
        (
            json!([{"subject": "character", "key": "\u{3000}", "value": "僕"}]),
            "属性のキーが空です",
            "attribute_key_empty",
        ),
        (
            json!([{"subject": "character", "key": "一人称", "value": ""}]),
            "属性の値が空です",
            "attribute_value_empty",
        ),
        (
            json!([{"subject": "character", "key": "あ".repeat(13), "value": "僕"}]),
            "属性のキーが長すぎます",
            "attribute_key_too_long",
        ),
        (
            Value::Array(
                (0..9)
                    .map(|index| {
                        json!({
                            "subject": "character",
                            "key": format!("キー{index}"),
                            "value": format!("値{index}"),
                        })
                    })
                    .collect(),
            ),
            "属性の件数が多すぎます",
            "too_many_attributes",
        ),
    ];

    for (attributes, message, code) in cases {
        let (status, body) = server
            .put(
                "/admin/characters/nobody/attributes",
                &json!({"attributes": attributes}),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "未知のキャラクターでも本文の不備を先に返すこと: {code}: {body}"
        );
        assert_eq!(
            body,
            error_body(message, code),
            "本文の理由をそのまま返すこと（404 に潰さない）: {code}"
        );
    }

    // **同じ宛先へ正当な本文を送れば 404**（400 を返し続ける実装ではない）。
    let (status, body) = server
        .put(
            "/admin/characters/nobody/attributes",
            &json!({"attributes": fixture()}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        error_body(
            "指定されたキャラクターが存在しません",
            "character_not_found"
        )
    );

    // **重複は DB が見るので、宛先の方が先に効く。** 上の決定の境界。
    let (status, body) = server
        .put(
            "/admin/characters/nobody/attributes",
            &json!({"attributes": [
                {"subject": "character", "key": "一人称", "value": "僕"},
                {"subject": "character", "key": " 一人称 ", "value": "俺"},
            ]}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "重複は `AttributeSet::parse` では見ないので 404: {body}"
    );
    assert_eq!(
        body,
        error_body(
            "指定されたキャラクターが存在しません",
            "character_not_found"
        )
    );

    assert_eq!(server.attribute_count().await, 0, "1行も書かれないこと");

    server.stop().await;
}

/// **2人のキャラクターの属性が混ざらない。**
#[tokio::test]
async fn two_characters_keep_their_own_attributes() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    server.create_character("rin", "凛").await;

    server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": fixture()}),
        )
        .await;
    let rin_attributes = json!([{"subject": "character", "key": "一人称", "value": "わたし"}]);
    server
        .put(
            "/admin/characters/rin/attributes",
            &json!({"attributes": rin_attributes}),
        )
        .await;

    let (_, body) = server.get("/admin/characters/aoi/attributes").await;
    assert_eq!(body, json!({"character": "aoi", "attributes": fixture()}));
    let (_, body) = server.get("/admin/characters/rin/attributes").await;
    assert_eq!(
        body,
        json!({"character": "rin", "attributes": rin_attributes})
    );

    // **他人の置き換えで消えない。**
    server
        .put(
            "/admin/characters/rin/attributes",
            &json!({"attributes": []}),
        )
        .await;
    let (_, body) = server.get("/admin/characters/aoi/attributes").await;
    assert_eq!(body["attributes"], fixture(), "葵の属性が残っていること");

    server.stop().await;
}

/// **壊れた行の復旧が HTTP で1本の線として繋がる**（作業単位 0011a-2 の先例と同じ形）。
///
/// **「直せるはず」で書かず、実際に直す手順を測る。**
///
/// ```text
/// GET  .../attributes        → 500（内部エラー。格納値は載らない）
/// PUT  .../attributes        → 200 ← 既存の行を読まないので届く
/// GET  .../attributes        → 200
/// ```
#[tokio::test]
async fn a_broken_stored_row_blocks_the_get_and_the_put_fixes_it() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": fixture()}),
        )
        .await;

    let id = server.character_id("aoi").await;
    let over_long = "あ".repeat(13);
    insert_character_attribute_ignoring_checks(
        &server.store,
        id,
        "character",
        &over_long,
        "猫獣人",
    )
    .await;

    let (status, body) = server.get("/admin/characters/aoi/attributes").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json!({"error": {"message": "内部エラーが発生しました", "type": "server_error"}}),
        "格納値も内部の詳細も載せないこと"
    );
    let text = body.to_string();
    assert!(
        !text.contains(&over_long) && !text.contains(&server.db_path),
        "格納値も DB のパスも漏らさないこと: {text}"
    );

    // **復旧の1手目がそのまま最後の手。**
    let fixed = json!([{"subject": "character", "key": "種族", "value": "猫獣人"}]);
    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": fixed}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "置き換えは届くこと: {body}");

    let (status, body) = server.get("/admin/characters/aoi/attributes").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"character": "aoi", "attributes": fixed}));

    server.stop().await;
}

/// **前後の空白は正規化されて保存される**（`" 一人称 "` と `"一人称"` が併存しない）。
#[tokio::test]
async fn padding_is_normalized_before_it_is_stored() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": [
                {"subject": "character", "key": " 一人称 ", "value": "\u{3000}僕\u{3000}"},
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["attributes"],
        json!([{"subject": "character", "key": "一人称", "value": "僕"}]),
        "正規化して保存・応答すること"
    );

    server.stop().await;
}

/// **このパスが受けるのは `PUT` と `GET` だけ。**
///
/// `PATCH` を生やすと「部分更新」と読まれる（`store` の意味論は丸ごと置き換え）。
/// `POST` / `DELETE` も塞ぐ。
#[tokio::test]
async fn only_put_and_get_are_exposed_for_attributes() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let method_not_allowed = json!({
        "error": {
            "message": "このエンドポイントは指定されたメソッドを受け付けません",
            "type": "invalid_request_error",
        }
    });
    for method in [Method::POST, Method::PATCH, Method::DELETE] {
        let (status, body) = server
            .send(
                method.clone(),
                "/admin/characters/aoi/attributes",
                Some(&json!({"attributes": []})),
            )
            .await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method}");
        assert_eq!(body, method_not_allowed, "{method}");
    }

    server.stop().await;
}

/// **未知のフィールドは要求まるごと拒否**（`/admin/` 共通の方針）。
///
/// 黙って無視すると、綴りを間違えた利用者が 200 を受け取って
/// 「設定したつもりの属性が入っていない」状態になる。
#[tokio::test]
async fn an_unknown_field_rejects_the_whole_request() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let invalid_body = error_body("リクエスト本文の形式が不正です", "invalid_body");
    for body in [
        json!({"attributes": [], "unknown": 1}),
        json!({"attribute": []}),
        json!({"attributes": [{"subject": "character", "key": "一人称", "value": "僕", "x": 1}]}),
        json!({"attributes": [{"subject": "character", "key": "一人称"}]}),
        json!({"attributes": [{"subject": "character", "key": 1, "value": "僕"}]}),
    ] {
        let (status, response) = server.put("/admin/characters/aoi/attributes", &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "要求 = {body}");
        assert_eq!(response, invalid_body, "要求 = {body}");
    }

    assert_eq!(server.attribute_count().await, 0, "1行も書かれないこと");

    server.stop().await;
}

// =========================================================================
// 属性の履歴（ADR-0028 §決定5 / 作業単位 A-3a）
//
// **読むだけの入口。** 書く口は `PUT .../attributes` ひとつ（§決定5 の「書き込み経路は1つ」）。
// =========================================================================

/// 履歴の応答から、**時刻と id を除いた形**を取り出す。
///
/// 時刻は DB が書くので固定できない。**`id` と `created_at` は別のテストで見る。**
fn revision_shapes(body: &Value) -> Vec<Value> {
    body["revisions"]
        .as_array()
        .expect("revisions が配列であること")
        .iter()
        .map(|revision| {
            json!({
                "subject": revision["subject"],
                "key": revision["key"],
                "before": revision["before"],
                "after": revision["after"],
            })
        })
        .collect()
}

/// **置き換えの履歴が、本番の入口（`App::bind`）を通って返る。**
///
/// 追加・変更・削除の3形と、**主語**まで応答に載ることを見る。
/// `before` / `after` を入れ替える変異は、値が全部違うのでここで崩れる。
#[tokio::test]
async fn the_history_of_a_replacement_comes_back_through_the_real_bind() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": [
                {"subject": "character", "key": "一人称", "value": "僕"},
                {"subject": "user", "key": "年齢", "value": "32"},
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "1手目が成功すること: {body}");

    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": [
                {"subject": "character", "key": "一人称", "value": "俺"},
                {"subject": "character", "key": "種族", "value": "猫獣人"},
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "2手目が成功すること: {body}");

    let (status, body) = server
        .get("/admin/characters/aoi/attributes/revisions")
        .await;
    assert_eq!(status, StatusCode::OK, "履歴を読めること: {body}");
    assert_eq!(body["character"], json!("aoi"), "どのキャラクターのものか");
    assert_eq!(
        revision_shapes(&body),
        vec![
            // **新しい順。** 2手目は「変更 → 追加 → 削除」の順に書かれる。
            json!({"subject": "user", "key": "年齢", "before": "32", "after": null}),
            json!({"subject": "character", "key": "種族", "before": null, "after": "猫獣人"}),
            json!({"subject": "character", "key": "一人称", "before": "僕", "after": "俺"}),
            json!({"subject": "user", "key": "年齢", "before": null, "after": "32"}),
            json!({"subject": "character", "key": "一人称", "before": null, "after": "僕"}),
        ],
        "追加・変更・削除が主語ごと載ること"
    );

    // **`id` と `created_at` も落とさない。** id は `sqlite3` で同じ行を指す唯一の手掛かり、
    // 時刻は「いつ静かに変わったのか」を読むためのもの（この表の存在理由）。
    let revisions = body["revisions"].as_array().expect("配列であること");
    let ids: Vec<i64> = revisions
        .iter()
        .map(|revision| revision["id"].as_i64().expect("id が整数であること"))
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.reverse();
    assert_eq!(ids, sorted, "id は降順（新しい順）であること: {ids:?}");
    for revision in revisions {
        let created_at = revision["created_at"]
            .as_str()
            .expect("created_at が文字列であること");
        assert!(
            created_at.contains('T') && created_at.ends_with('Z'),
            "UTC の時刻であること: {created_at}"
        );
    }

    server.stop().await;
}

/// **履歴の無いキャラクターは空の配列**（キーごと落とさない）。
///
/// **上限も一緒に返す。** 返した件数が上限と同じとき、読み手は
/// 「これで全部」なのか「切られた」のかを、これでしか区別できない。
#[tokio::test]
async fn a_character_without_history_returns_an_empty_array() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server
        .get("/admin/characters/aoi/attributes/revisions")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["character"], json!("aoi"));
    assert_eq!(body["revisions"], json!([]), "空でもキーごと落とさないこと");
    assert!(
        body["revisions_limit"].as_u64().unwrap_or(0) > 0,
        "上限を載せること: {body}"
    );
    // **総数は件数に関係なく名乗る**（0 件でもキーごと落とさない）。
    // **ここで言えるのは「欄が在る」までである**——0 件では総数と件数が同値なので、
    // すり替えの権威は `only_the_newest_revisions_come_back_but_the_total_counts_them_all`。
    assert_eq!(
        body["total_revisions"],
        json!(0),
        "総数を載せること: {body}"
    );

    server.stop().await;
}

/// **8 件の属性の値を一斉に書き換える。** 1手で 8 件の履歴が増える。
///
/// 上限（200）を超える履歴を積むための手段。**`PUT` を回すだけ**で作れる
/// ——履歴の書き手は1つ（ADR-0028 §決定5）なので、直接 INSERT する道具を作らない。
async fn put_generation(server: &TestServer, generation: usize) {
    let attributes: Vec<Value> = (1..=8)
        .map(|index| {
            json!({
                "subject": "character",
                "key": format!("属性{index}"),
                "value": format!("世代{generation}"),
            })
        })
        .collect();
    let (status, body) = server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": attributes}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "第 {generation} 世代を書けること: {body}"
    );
}

/// **上限を超えたら新しい側だけが返る。総数は全部を数える。**
///
/// # 期待値を定数から作らない（CLAUDE.md `16b`）
///
/// `ATTRIBUTE_REVISIONS_LIMIT` から期待値を組むと、**その定数を倍にする変異が緑を通る。**
/// ここは**外の値**（直書きの 208 と 200）で押さえる。
///
/// # ⚠ **この1本が3つの主張を持っている**（分割・簡略化するときの注意）
///
/// 1. **総数**が「返した件数」ではないこと（`total_revisions == 208`）。
/// 2. **上限**が効いていること（`revisions.len() == 200`）。
/// 3. **名乗る上限が、実際に切った上限**であること（`revisions_limit == 200`）。
///
/// **3つとも、履歴が上限を超えている状態でしか観測できない。**
/// 上限以下では総数と件数が同値になり、どちらへ倒す実装も緑になる。
/// **`revisions_limit` はさらに弱く、切られていなければ何を名乗っても件数と食い違わない**
/// ——レビューの実測では `revisions_limit: 1` の決め打ちが 2046 全緑を通った。
/// **この状態を作るテストはここ1本だけ**である。
/// **分けるなら、分けた先それぞれで 200 件超を積むこと。**
/// 積まずに分けると、**3つ同時に無防備になる。**
///
/// **溢れる側も見る**（末尾が2手目の変更）——切る側を新しい方へ倒す変異は、
/// 先頭と末尾の両方が動く。
#[tokio::test]
async fn only_the_newest_revisions_come_back_but_the_total_counts_them_all() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    // 1手目は 8 件の**追加**、以降は1手ごとに 8 件の**変更**。**26 手で 208 件。**
    for generation in 0..26 {
        put_generation(&server, generation).await;
    }

    let (status, body) = server
        .get("/admin/characters/aoi/attributes/revisions")
        .await;
    assert_eq!(status, StatusCode::OK, "履歴を読めること: {body}");

    let revisions = body["revisions"].as_array().expect("配列であること");
    assert_eq!(
        revisions.len(),
        200,
        "直近 200 件だけを返すこと: {}",
        revisions.len()
    );
    assert_eq!(
        body["revisions_limit"],
        json!(200),
        "名乗る上限が、実際に切った件数と同じであること: {body}"
    );
    assert_eq!(
        body["total_revisions"],
        json!(208),
        "総数は返した件数ではなく、保存されている全部であること: {body}"
    );

    // **いちばん新しい世代が先頭。**
    assert_eq!(
        (
            revisions[0]["before"].clone(),
            revisions[0]["after"].clone()
        ),
        (json!("世代24"), json!("世代25")),
        "新しい順であること: {}",
        revisions[0]
    );
    // **溢れたのは古い側の 8 件**（1手目の追加）。末尾は2手目の変更になる。
    let oldest = revisions.last().expect("1件はあること");
    assert_eq!(
        (oldest["before"].clone(), oldest["after"].clone()),
        (json!("世代0"), json!("世代1")),
        "切るのは古い側であること: {oldest}"
    );

    server.stop().await;
}

/// **履歴の応答は、それを訊いたキャラクターのものである。**
///
/// 兄弟の `two_characters_keep_their_own_attributes` と同じ形。
/// **両方に履歴を作る**——片方だけだと、`character` の決め打ちも、
/// 総数の絞り込みを落とす変異も緑を通る（件数も値も違えてある）。
///
/// **応答を丸ごと突き合わせる**ので、欄が欠けても余っても赤くなる。
/// `id` と `created_at` は DB が書くので応答から採るが、
/// **キーごと無くなれば形が合わない。**
#[tokio::test]
async fn the_history_of_each_character_is_its_own() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    server.create_character("rin", "凛").await;

    // 葵は2件（追加 → 変更）、凛は1件（追加）。**件数も値も違う。**
    for value in ["僕", "俺"] {
        let (status, body) = server
            .put(
                "/admin/characters/aoi/attributes",
                &json!({"attributes": [
                    {"subject": "character", "key": "一人称", "value": value},
                ]}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "葵を書けること: {body}");
    }
    let (status, body) = server
        .put(
            "/admin/characters/rin/attributes",
            &json!({"attributes": [
                {"subject": "character", "key": "一人称", "value": "わたし"},
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "凛を書けること: {body}");

    let (status, body) = server
        .get("/admin/characters/rin/attributes/revisions")
        .await;
    assert_eq!(status, StatusCode::OK, "履歴を読めること: {body}");
    let revisions = body["revisions"].as_array().expect("配列であること");
    assert_eq!(revisions.len(), 1, "凛の履歴は1件だけであること: {body}");
    let id = revisions[0]["id"].clone();
    let created_at = revisions[0]["created_at"].clone();
    assert!(
        id.as_i64().is_some_and(|value| value > 0),
        "id が主キーであること: {body}"
    );
    assert_eq!(
        body,
        json!({
            "character": "rin",
            "revisions": [{
                "id": id,
                "subject": "character",
                "key": "一人称",
                "before": null,
                "after": "わたし",
                "created_at": created_at,
            }],
            "total_revisions": 1,
            "revisions_limit": 200,
        }),
        "凛の履歴だけが、欠けも余りもなく返ること"
    );

    // **葵の側も自分のものだけ。**
    let (status, body) = server
        .get("/admin/characters/aoi/attributes/revisions")
        .await;
    assert_eq!(status, StatusCode::OK, "履歴を読めること: {body}");
    assert_eq!(body["character"], json!("aoi"), "葵のものと名乗ること");
    assert_eq!(
        body["total_revisions"],
        json!(2),
        "葵は2件であること: {body}"
    );
    assert_eq!(
        revision_shapes(&body),
        vec![
            json!({"subject": "character", "key": "一人称", "before": "僕", "after": "俺"}),
            json!({"subject": "character", "key": "一人称", "before": null, "after": "僕"}),
        ],
        "葵の履歴が混ざらないこと"
    );

    server.stop().await;
}

/// **未知のキャラクターの履歴は 404**（属性の入口と同じ）。
#[tokio::test]
async fn the_history_of_an_unknown_character_is_not_found() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server
        .get("/admin/characters/rin/attributes/revisions")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        error_body(
            "指定されたキャラクターが存在しません",
            "character_not_found",
        ),
        "内部の詳細を載せないこと"
    );

    server.stop().await;
}

/// **履歴のパスに生えているのは `GET` だけ**（ADR-0028 §決定5「追記のみ」）。
///
/// 書き込みの口を生やすと、**外から書き換えられる記録**になる。
///
/// # **半分が2つある**
///
/// 写し元は `tests/server_admin.rs::no_admin_endpoint_deletes_or_rewrites_a_memory`
/// で、**あちらは両方の半分を持っている。**
///
/// | 見るもの | 落とすと |
/// |---|---|
/// | 一覧のパスは `GET` だけ（405） | `DELETE .../revisions`（履歴を全部消す）が生える |
/// | **1件を指すパスが無い（404）** | `PUT`（置き換え）と `DELETE`（削除）の**置き場**ができる |
///
/// **後半が無いと、`.../revisions/{id}` に `GET` を1本生やす変異が素通りする。**
/// それ自体は読むだけだが、**置き場ができた**時点で ADR-0028 の「追記のみ」は
/// パスとして表現できてしまう（記憶の側で塞いだのとまったく同じ穴）。
#[tokio::test]
async fn only_get_is_exposed_for_the_revisions_path() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let method_not_allowed = json!({
        "error": {
            "message": "このエンドポイントは指定されたメソッドを受け付けません",
            "type": "invalid_request_error",
        }
    });
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        let (status, body) = server
            .send(
                method.clone(),
                "/admin/characters/aoi/attributes/revisions",
                Some(&json!({"revisions": []})),
            )
            .await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method}");
        assert_eq!(body, method_not_allowed, "{method}");
    }

    // **1件を指すパスは、どのメソッドでも 404**（そこには何も生えていない）。
    let not_found = json!({
        "error": {
            "message": "指定されたエンドポイントは存在しません",
            "type": "invalid_request_error",
        }
    });
    for method in [
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
    ] {
        let (status, body) = server
            .send(
                method.clone(),
                "/admin/characters/aoi/attributes/revisions/1",
                Some(&json!({})),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}");
        assert_eq!(body, not_found, "{method}");
    }

    server.stop().await;
}

/// **読めない履歴の行があれば 500。ただし内部情報は載せない。**
///
/// **止まるのは履歴の一覧だけ**——属性そのものは読めるし、置き換えも届く
/// （`store` の `attribute_revisions` の doc）。
#[tokio::test]
async fn a_broken_history_row_is_a_five_hundred_without_internals() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    server
        .put(
            "/admin/characters/aoi/attributes",
            &json!({"attributes": fixture()}),
        )
        .await;

    let id = server.character_id("aoi").await;
    let over_long = "い".repeat(21);
    insert_attribute_revision_ignoring_checks(
        &server.store,
        id,
        "character",
        "種族",
        Some(&over_long),
        None,
    )
    .await;

    let (status, body) = server
        .get("/admin/characters/aoi/attributes/revisions")
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json!({"error": {"message": "内部エラーが発生しました", "type": "server_error"}}),
        "格納値も内部の詳細も載せないこと"
    );
    let text = body.to_string();
    assert!(
        !text.contains(&over_long) && !text.contains(&server.db_path),
        "格納値も DB のパスも漏らさないこと: {text}"
    );

    // **属性の側は無事。**
    let (status, body) = server.get("/admin/characters/aoi/attributes").await;
    assert_eq!(status, StatusCode::OK, "属性は読めること: {body}");

    server.stop().await;
}

/// **属性を持たないキャラクターは空の配列**（キーごと落とさない）。
#[tokio::test]
async fn a_character_without_attributes_returns_an_empty_array() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server.get("/admin/characters/aoi/attributes").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"character": "aoi", "attributes": []}));

    server.stop().await;
}
