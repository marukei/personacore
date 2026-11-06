//! `/admin/` 配下の管理 API の検証（ADR-0008）。
//!
//! **LM Studio には繋がない。** 管理 API は上流を必要としない。
//! ここで固定するのは主に次の8点:
//!
//!   * 作成 → 一覧 → **`/v1/models` に出る**こと（既存の一覧と同じ経路に繋がっていること）。
//!   * 性格ステータスが**丸ごと置き換え**であること（前の軸が残らない）。
//!   * **値 0 の軸が保存・取得できる**こと（落とすのは `prompt` の責務）。
//!   * 不正な名前・軸名が**理由ごとに区別できるエラー**になること。
//!   * **ゴースト行との衝突が正当な重複と区別できる**こと（ADR-0004 の袋小路を押し付けない）。
//!   * エラーレスポンスに**内部情報を載せない**こと（本文を完全一致で固定する）。
//!   * **「一覧 → 修復」と「一覧 → 掃除」が HTTP で1本の線として繋がる**こと
//!     （ADR-0016 / 作業単位 0012c）。壊れた行は `GET` が返した手掛かりだけで
//!     `PATCH` / `DELETE` に届く。
//!   * **会話のあるキャラクターは消せない**こと（応答で区別でき、行数が変わらない）。
//!
//! `memory` を**消す・書き換える**エンドポイントが存在しないことも合わせて固定する。
//! **「1つも生えていない」ではない**——ADR-0022 が読む入口（`GET .../memories`）と
//! 行を追加する入口（`POST .../corrections`）の2本を足した。**その2本だけ**であることを
//! `no_admin_endpoint_deletes_or_rewrites_a_memory` が見る
//! （その2本の中身は `tests/server_admin_memories.rs` が持つ）。

use std::net::SocketAddr;

use chrono::{DateTime, Utc};
use personacore::domain::{Handle, Persona};
use personacore::server::{App, AppConfig, BindAddr, ServerError};
use personacore::store::Store;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

mod common;

use common::{CLOCK_SKEW, insert_ghost_character_with};

/// エフェメラルポートで起動した実サーバ。
struct TestServer {
    addr: SocketAddr,
    /// アプリと**同じプールを指す**ハンドル。ゴースト行の投入と障害の再現に使う。
    store: Store,
    /// DB ファイルの絶対パス。エラーレスポンスに漏れていないことの確認に使う。
    db_path: String,
    client: reqwest::Client,
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

    /// JSON 本文つきで送る（本文が無いメソッドは `None`）。
    async fn send(&self, method: Method, path: &str, body: Option<&Value>) -> (StatusCode, String) {
        let mut request = self
            .client
            .request(method, format!("http://{}{path}", self.addr));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.expect("サーバへ接続できること");
        let status = response.status();
        let body = response.text().await.expect("本文を読めること");
        (status, body)
    }

    async fn get(&self, path: &str) -> (StatusCode, String) {
        self.send(Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: &Value) -> (StatusCode, String) {
        self.send(Method::POST, path, Some(body)).await
    }

    async fn put(&self, path: &str, body: &Value) -> (StatusCode, String) {
        self.send(Method::PUT, path, Some(body)).await
    }

    async fn patch(&self, path: &str, body: &Value) -> (StatusCode, String) {
        self.send(Method::PATCH, path, Some(body)).await
    }

    /// **本文は付けない。** 削除の対象はパスだけで決まる。
    async fn delete(&self, path: &str) -> (StatusCode, String) {
        self.send(Method::DELETE, path, None).await
    }

    /// `character` の行数（**拒否されたのに消えていない**ことの確認に使う）。
    async fn character_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM character")
            .fetch_one(self.store.pool())
            .await
            .expect("行数を数えられること")
    }

    /// `session` の行数（会話を巻き添えにしていないことの確認に使う）。
    async fn session_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM session")
            .fetch_one(self.store.pool())
            .await
            .expect("行数を数えられること")
    }

    /// **JSON として壊れた本文**を送る（serde のエラーが反射されないことの確認用）。
    async fn post_raw(&self, path: &str, body: &'static str) -> (StatusCode, String) {
        self.post_bytes(path, body.to_owned()).await
    }

    /// 生の本文を送る（上限超過の確認に使う。`String` なので長さを作れる）。
    async fn post_bytes(&self, path: &str, body: String) -> (StatusCode, String) {
        let response = self
            .client
            .post(format!("http://{}{path}", self.addr))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
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

/// 本文を JSON として読む。
fn parse_json(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|err| panic!("JSON として読めること: {err} / {body}"))
}

/// `created_at` を日時として読む。
fn parse_time(value: &Value) -> DateTime<Utc> {
    let raw = value.as_str().expect("created_at は文字列であること");
    DateTime::parse_from_rfc3339(raw)
        .unwrap_or_else(|err| panic!("RFC3339 として読めること: {err} / {raw}"))
        .with_timezone(&Utc)
}

/// 性格ステータスのパス。
fn stats_path(name: &str) -> String {
    format!("/admin/characters/{name}/stats")
}

/// キャラクター1件を指すパス（`PATCH` / `DELETE`）。
fn character_path(name: &str) -> String {
    format!("/admin/characters/{name}")
}

// --- 修復・掃除（0012c）のフィクスチャ -------------------------------------
//
// **どれも互いに違う文字列にする。** 2つを同値にした時点で、その2つの取り違え
// （別のキャラクターを引く / 別の列に書く / 要求をそのまま反射する）は検出できない。

/// 巻き添えの見張り役。**対象より先に作って id を 1 から外す**（囮）。
///
/// **名前順でも対象より前に来ること**（`a` で始め、`TARGET_HANDLE` より小さくする）。
/// 後ろに置くと、応答を「一覧の先頭の行」から組み立てる実装が
/// **対象と一致してしまい緑を通る**（0012c の変異検査で実測。
/// `kaede` だったときは修復のテスト1本しか赤にならなかった）。
const DECOY_HANDLE: &str = "akane";
const DECOY_DISPLAY: &str = "茜音";
const DECOY_PERSONA: &str = "昼番の受付。";
const DECOY_FORMAT: &str = "地の文は簡潔に。";
const DECOY_SCALE: u32 = 3;

/// 更新・削除の対象。**識別子と呼び名を別の値にする**（ADR-0011）。
const TARGET_HANDLE: &str = "aoi-v2";
const TARGET_DISPLAY: &str = "葵";
const TARGET_PERSONA: &str = "落ち着いた物腰の司書。";
const TARGET_FORMAT: &str = "台詞は「」で囲むこと。";
const TARGET_SCALE: u32 = 7;

/// **尺度の出どころを縛るための追加の囮**（作業単位 0011a-1 のレビュー指摘）。
///
/// `DECOY_HANDLE` 1件だけでは足りない。囮が `akane`（3）と対象 `aoi-v2`（7）の2件だと、
/// **「一覧の2件目を取る」実装も「一番大きい尺度を取る」実装も対象と一致して緑を通る。**
/// そこで対象を**識別子順で3番目**に置き、前に2件・後ろに1件を配る:
///
/// | 位置 | 識別子 | 尺度 |
/// |---|---|---|
/// | 1 | `akane`（`DECOY_HANDLE`） | 3 ← **最小** |
/// | 2 | `amane`（ここ） | 5 |
/// | 3 | `aoi-v2`（`TARGET_HANDLE`） | **7**（対象） |
/// | 4 | `yuzu`（ここ） | 55 ← **最大** |
///
/// これで「先頭 / 2件目 / 末尾 / 最小 / 最大」のどれを取っても対象と食い違う。
/// 配置そのものは `create_scale_decoys` が実行時に確かめる。
const SECOND_DECOY_HANDLE: &str = "amane";
const SECOND_DECOY_SCALE: u32 = 5;
const TRAILING_DECOY_HANDLE: &str = "yuzu";
const TRAILING_DECOY_SCALE: u32 = 55;

/// 更新後の値。
const NEW_DISPLAY: &str = "葵 碧";
const NEW_PERSONA: &str = "夜勤の警備員。";
const NEW_FORMAT: &str = "独白は（）で囲むこと。";
const NEW_SCALE: u32 = 42;

/// **変容の走らせ方**（ADR-0045 / 作業単位 A-8）。**囮・対象・更新後で3つとも違う綴り。**
///
/// 同値にすると、`UPDATE_CHARACTER` の `?6` / `?7` を取り違える実装や、
/// 囮の行へ書く実装が区別できない。**未設定（`null`）も使わない**——
/// 「読み捨てて `null` を返す」実装が通る。
const DECOY_DRIFT_MODE: &str = "shadow";
const TARGET_DRIFT_MODE: &str = "on";
const NEW_DRIFT_MODE: &str = "off";

/// **省略時に配られる尺度**（ADR-0015 / `domain::StatScale::DEFAULT`）。
///
/// **この値をフィクスチャに使わないこと。** 使うと「要求の尺度を捨てて
/// 常に既定を書く」実装が緑を通る（CLAUDE.md「フィクスチャが2つの値を同値にすると…」）。
/// 期待値としてこれを使ってよいのは、`an_omitted_stat_scale_falls_back_to_the_default`
/// のように**省略の振る舞いそのもの**を見るテストだけ。
const DEFAULT_SCALE: u32 = 10;

/// フィクスチャの値が互いに違うことを、使う前に確かめる。
fn assert_fixture_values_are_distinct() {
    let values = [
        DECOY_HANDLE,
        DECOY_DISPLAY,
        DECOY_PERSONA,
        DECOY_FORMAT,
        TARGET_HANDLE,
        TARGET_DISPLAY,
        TARGET_PERSONA,
        TARGET_FORMAT,
        NEW_DISPLAY,
        NEW_PERSONA,
        NEW_FORMAT,
        SECOND_DECOY_HANDLE,
        TRAILING_DECOY_HANDLE,
    ];
    for (index, left) in values.iter().enumerate() {
        for right in &values[index + 1..] {
            assert_ne!(left, right, "フィクスチャの値は互いに違うこと");
        }
    }

    // **尺度も同じ扱い。** 囮・対象・更新後がどれも違い、しかもどれも既定と違うこと。
    // 1つでも既定と同値にすると、その場面では「常に既定を書く」実装を検出できない。
    let scales = [
        DECOY_SCALE,
        TARGET_SCALE,
        NEW_SCALE,
        SECOND_DECOY_SCALE,
        TRAILING_DECOY_SCALE,
    ];
    for (index, left) in scales.iter().enumerate() {
        assert_ne!(
            *left, DEFAULT_SCALE,
            "フィクスチャの尺度は既定と違うこと（同値だと『常に既定を書く』実装が通る）"
        );
        for right in &scales[index + 1..] {
            assert_ne!(left, right, "フィクスチャの尺度は互いに違うこと");
        }
    }

    // **走らせ方も同じ扱い**（作業単位 A-8）。
    let modes = [DECOY_DRIFT_MODE, TARGET_DRIFT_MODE, NEW_DRIFT_MODE];
    for (index, left) in modes.iter().enumerate() {
        for right in &modes[index + 1..] {
            assert_ne!(left, right, "フィクスチャの走らせ方は互いに違うこと");
        }
    }
}

/// 囮 → 対象の順に1件ずつ作り、**それぞれの id** を返す。
///
/// **順序が意味を持つ。** 囮を先に作ることで対象の id が 1 にならず、
/// 「応答の id を決め打ちする」実装と区別できる（0012a で実際に踏んだ穴）。
async fn create_decoy_and_target(server: &TestServer) -> (i64, i64) {
    assert_fixture_values_are_distinct();

    let mut ids = Vec::new();
    for (handle, display_name, persona, format, scale, drift_mode) in [
        (
            DECOY_HANDLE,
            DECOY_DISPLAY,
            DECOY_PERSONA,
            DECOY_FORMAT,
            DECOY_SCALE,
            DECOY_DRIFT_MODE,
        ),
        (
            TARGET_HANDLE,
            TARGET_DISPLAY,
            TARGET_PERSONA,
            TARGET_FORMAT,
            TARGET_SCALE,
            TARGET_DRIFT_MODE,
        ),
    ] {
        let (status, body) = server
            .post(
                "/admin/characters",
                &json!({
                    "handle": handle,
                    "display_name": display_name,
                    "base_persona": persona,
                    "output_format_instruction": format,
                    "stat_scale": scale,
                    "stat_drift_mode": drift_mode,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        ids.push(
            parse_json(&body)["id"]
                .as_i64()
                .unwrap_or_else(|| panic!("id は整数であること: {body}")),
        );
    }

    let (decoy, target) = (ids[0], ids[1]);
    assert_ne!(decoy, target, "別の行であること");
    assert_ne!(target, 1, "対象の id が既定値にならないこと");
    (decoy, target)
}

/// 囮だけを1件作り、その id を返す（壊れた行より**先に**作ること）。
///
/// `create_decoy_and_target` と違い、対象は呼び出し側が直接 INSERT する
/// （`/admin/` からは作れない壊れ方を仕込むため）。
async fn create_decoy(server: &TestServer) -> i64 {
    assert_fixture_values_are_distinct();

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": DECOY_HANDLE,
                "display_name": DECOY_DISPLAY,
                "base_persona": DECOY_PERSONA,
                "output_format_instruction": DECOY_FORMAT,
                "stat_scale": DECOY_SCALE,
                "stat_drift_mode": DECOY_DRIFT_MODE,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    parse_json(&body)["id"]
        .as_i64()
        .unwrap_or_else(|| panic!("id は整数であること: {body}"))
}

/// **尺度の囮を3件作る**（`TARGET_HANDLE` を作る**前**に呼ぶこと）。
///
/// 返すのは `(識別子, 尺度)` の一覧で、呼び出し側はそのまま
/// 「この行を引いたら**この行の**尺度が返る」ことの確認に使える。
///
/// **見ているのは識別子と尺度だけ。** 呼び名・ペルソナ・形式指示は省略に任せる
/// （`create_decoy` が `DECOY_DISPLAY` などを揃えるのは修復・掃除の巻き添えを見るためで、
///  尺度の出どころとは関係が無い。ここで同じ値を並べると、
///  この囮が何を縛っているのかが読みにくくなる）。
///
/// # 作るだけで終わらせない
///
/// 囮の**配置**（識別子の順序と尺度の大小）が狙いどおりであることを、
/// 作る前にその場で確かめる。ここが崩れると
/// 「囮を作っている」形だけが残り、取り違えを1つも捕まえなくなる。
async fn create_scale_decoys(server: &TestServer) -> [(&'static str, u32); 3] {
    assert_fixture_values_are_distinct();

    let decoys = [
        (DECOY_HANDLE, DECOY_SCALE),
        (SECOND_DECOY_HANDLE, SECOND_DECOY_SCALE),
        (TRAILING_DECOY_HANDLE, TRAILING_DECOY_SCALE),
    ];

    // 識別子順（`Store::list_characters` は `ORDER BY handle`）で
    // 「囮 → 囮 → 対象 → 囮」に並ぶこと。**対象を先頭にも2番目にも末尾にも置かない。**
    assert!(
        DECOY_HANDLE < SECOND_DECOY_HANDLE,
        "囮1は囮2より前に来ること: {DECOY_HANDLE} < {SECOND_DECOY_HANDLE}"
    );
    assert!(
        SECOND_DECOY_HANDLE < TARGET_HANDLE,
        "囮2は対象より前に来ること: {SECOND_DECOY_HANDLE} < {TARGET_HANDLE}"
    );
    assert!(
        TARGET_HANDLE < TRAILING_DECOY_HANDLE,
        "囮3は対象より後に来ること: {TARGET_HANDLE} < {TRAILING_DECOY_HANDLE}"
    );

    // 尺度が対象を**挟む**こと。最小も最大も対象ではない
    // （挟まないと「一番大きい尺度を取る」実装が対象と一致して緑を通る）。
    assert!(
        decoys.iter().any(|(_, scale)| *scale < TARGET_SCALE),
        "対象より小さい尺度の囮があること: {decoys:?} / 対象 = {TARGET_SCALE}"
    );
    assert!(
        decoys.iter().any(|(_, scale)| *scale > TARGET_SCALE),
        "対象より大きい尺度の囮があること: {decoys:?} / 対象 = {TARGET_SCALE}"
    );

    for (handle, scale) in decoys {
        let (status, body) = server
            .post(
                "/admin/characters",
                &json!({
                    "handle": handle,
                    "base_persona": format!("囮のペルソナ本文（{handle}）。"),
                    "stat_scale": scale,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{handle}: {body}");
    }

    decoys
}

/// `character` の1行を **`parse` もデコードも通さずに**読む。
///
/// **`GET` では見えない行の中身を確かめる唯一の手段。** ゴースト行（完全一致で
/// 引けない）や `created_at` が壊れた行（一覧ごと 500）は、HTTP からは中身を
/// 覗けない——だからこそ「200 が空手形でないこと」をここで見る必要がある。
async fn raw_character_row(
    server: &TestServer,
    id: i64,
) -> (String, String, String, Option<String>) {
    sqlx::query_as(
        "SELECT handle, display_name, base_persona, output_format_instruction \
         FROM character WHERE id = ?",
    )
    .bind(id)
    .fetch_one(server.store.pool())
    .await
    .unwrap_or_else(|err| panic!("id={id} の行を生のまま読めること: {err}"))
}

/// 一覧から1件を名前で取り出す（**読めた行だけ**）。
fn character_in(body: &str, name: &str) -> Value {
    parse_json(body)["characters"]
        .as_array()
        .unwrap_or_else(|| panic!("characters は配列であること: {body}"))
        .iter()
        .find(|entry| entry["handle"] == json!(name))
        .unwrap_or_else(|| panic!("{name} が一覧にあること: {body}"))
        .clone()
}

/// 囮が巻き添えになっていないことを確かめる。
///
/// **更新・削除は正規化キーで引く**（ADR-0016）。照合が広がると隣の行を
/// 巻き込むので、対象以外が1文字も変わっていないことを毎回見る。
async fn assert_decoy_is_untouched(server: &TestServer) {
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let decoy = character_in(&body, DECOY_HANDLE);
    assert_eq!(
        (
            &decoy["display_name"],
            &decoy["base_persona"],
            &decoy["output_format_instruction"],
            &decoy["stat_scale"],
            &decoy["stat_drift_mode"],
        ),
        (
            &json!(DECOY_DISPLAY),
            &json!(DECOY_PERSONA),
            &json!(DECOY_FORMAT),
            &json!(DECOY_SCALE),
            &json!(DECOY_DRIFT_MODE),
        ),
        "囮が巻き添えになっていないこと: {body}"
    );
}

/// 一覧レスポンスからキャラクター名だけを取り出す。
fn names_in(body: &str, key: &str) -> Vec<String> {
    parse_json(body)[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} は配列であること: {body}"))
        .iter()
        .map(|entry| {
            let field = if key == "characters" { "handle" } else { "id" };
            entry[field]
                .as_str()
                .unwrap_or_else(|| panic!("{field} は文字列であること: {entry}"))
                .to_owned()
        })
        .collect()
}

/// 400 のエラー応答（`code` つき）を組み立てる。
fn invalid_request(message: &str, code: &str) -> Value {
    json!({"error": {"message": message, "type": "invalid_request_error", "code": code}})
}

/// 作成したキャラクターが**管理 API の一覧と `/v1/models` の両方**に現れる。
///
/// 既存の `/v1/models` と繋がっていること（= 書き込み経路が1つであること）の確認。
#[tokio::test]
async fn a_created_character_appears_in_both_listings() {
    let server = TestServer::start().await;
    let before = Utc::now() - CLOCK_SKEW;

    // **識別子は ASCII 小文字・数字・`-`・`_` だけ**（ADR-0017 / 作業単位 0004b-1）。
    // 0004b-1 より前はここが `" 葵 "` で「前後の空白は正規化されて保存される」
    // ことを見ていたが、いまは正規化せず**拒否する**（`the_handle_is_never_normalized`）。
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": "aoi",
                "base_persona": "落ち着いた物腰の司書。",
                "output_format_instruction": "台詞は「」で囲むこと。",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let created = parse_json(&body);
    assert_eq!(
        created["handle"],
        json!("aoi"),
        "識別子を書き換えずに保存すること: {body}"
    );
    assert_eq!(created["base_persona"], json!("落ち着いた物腰の司書。"));
    assert_eq!(
        created["output_format_instruction"],
        json!("台詞は「」で囲むこと。")
    );
    let created_at = parse_time(&created["created_at"]);
    assert!(
        created_at >= before && created_at <= Utc::now() + CLOCK_SKEW,
        "created_at に妥当な作成日時が入ること: {created_at}"
    );

    // 形式指示は省略できる（既定値はアプリ側が決める）。
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({"handle": "rin", "base_persona": "ペルソナ本文"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        parse_json(&body)["output_format_instruction"],
        Value::Null,
        "未設定は null で返ること: {body}"
    );

    // 管理 API の一覧。
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        names_in(&body, "characters"),
        vec!["aoi", "rin"],
        "名前順に並ぶこと: {body}"
    );

    // **`/v1/models` に出る。** ここが繋がっていないと FE からキャラを選べない。
    let (status, body) = server.get("/v1/models").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        names_in(&body, "data"),
        vec!["aoi", "rin"],
        "作成したキャラクターがモデル一覧に現れること: {body}"
    );

    server.stop().await;
}

/// 投入サンプルの**実物**。写しではない（作業単位 0004b-3）。
///
/// `include_str!` はビルドの入力なので、**JSON を書き換えればこのテストバイナリは
/// 必ず再コンパイルされる。** 古い版が使い回されて緑になることが無い。
const SAMPLE_SEED_JSON: &str = include_str!("../scripts/characters.sample.json");

/// **`scripts/characters.sample.json` の各要素が、そのまま投入経路を通る。**
///
/// 覆うのは2本立て。**どちらもサンプルの中身をそのまま投げる。**
///
/// | 要素の中身 | 投げ先 | 期待 |
/// |---|---|---|
/// | `stats` を除いた全キー | `POST /admin/characters` | 201 |
/// | `stats` の配列 | `PUT /admin/characters/{handle}/stats` | 200 |
///
/// # なぜ要るのか（作業単位 0004b-3）
///
/// この JSON は投入スクリプトが読む唯一のサンプルで、**API の要求本文とキー名が
/// 1文字も違ってはいけない**（`/admin/` は全入口で `deny_unknown_fields`）。
/// にもかかわらず 0004b の改名では、この JSON の `"handle"` を `"name"` に戻しても
/// **スイート全体が緑のまま**だった（実測）。「合わせ忘れると投入が壊れる」と
/// 書かれていた箇所が、機械的には1つも守られていなかった。
///
/// **`stats` の側も同じ穴が開いていた。** 追補前は下のループが `stats` を
/// 本文から外すだけで投げていなかったので、`{ "axis", "value" }` を
/// `{ "name", "value" }` に書き換えても **781 passed / 0 failed**（実測）。
/// 綴りを直せるのは `PUT` まで投げたときだけ。
///
/// # **内容を写さない**
///
/// 期待値をテスト側に書き写すと写しが2つになり、片方だけが古びる——この見張り番が
/// 潰そうとしている問題そのものが起きる。だから**ファイルから取る。**
///
/// # **`scripts/seed-characters.ps1` は依然として1行も覆っていない**
///
/// ここが見ているのは **JSON ↔ API** の対応だけ。PowerShell はテストから起動していないので、
/// スクリプトが本文を組み立てる部分——`Test-Property` によるキーの取捨、
/// `EscapeDataString`、409（既存）の扱い、`/v1/models` の表示——は
/// **一度も実行されない。** スクリプト側だけを壊した変更は、ここでは**緑のまま**通る。
/// **投入経路を1本増やしてもこれは変わらない**（増えたのは「JSON をこの形で投げれば通る」
/// という保証であって、「スクリプトがこの形で投げている」という保証ではない）。
///
/// スクリプトが API の綴りを**書いているか**だけは
/// `the_seed_script_still_spells_the_admin_api_the_way_the_server_expects` が見る。
/// **それも実行はしない**（あちらの doc を参照）。
#[tokio::test]
async fn every_character_in_the_sample_seed_file_is_accepted_as_is() {
    let server = TestServer::start().await;

    let document = parse_json(SAMPLE_SEED_JSON);
    let characters = document["characters"]
        .as_array()
        .unwrap_or_else(|| panic!("トップレベルに characters 配列があること: {document}"))
        .clone();
    // **空なら何も確かめていない**（要素0だとループが回らずに緑になる）。
    assert!(
        !characters.is_empty(),
        "サンプルに少なくとも1件あること: {document}"
    );

    // **`stats` を投げた回数。** 0 のまま終わったら `PUT` 側は何も確かめていない
    // （サンプルから `stats` が丸ごと消えると、下の分岐が一度も通らずに緑になる）。
    let mut stats_seeded = 0_u32;

    for character in characters {
        let mut body = character.clone();
        let fields = body
            .as_object_mut()
            .unwrap_or_else(|| panic!("characters の要素はオブジェクトであること: {character}"));
        // **作成本文に載らない唯一のキー。** 残りは1つでも綴りが違えば 400 になる。
        // 作成本文に混ぜると `deny_unknown_fields` が 400 にするので、
        // スクリプトと同じく別の `PUT` に回す（下）。
        let stats = fields.remove("stats");

        let (status, response) = server.post("/admin/characters", &body).await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "サンプルの1件をそのまま投入できること: 要求 = {body}, 応答 = {response}"
        );

        // **`stats` は省略できる**（サンプルの実態に合わせる。無い要素があってよい）。
        let Some(stats) = stats else {
            continue;
        };
        let handle = body["handle"]
            .as_str()
            .unwrap_or_else(|| panic!("handle は文字列であること: {body}"));
        // **配列をそのまま `stats` キーに載せる。** 要素の綴り（`axis` / `value`）も
        // 中身も一切書き換えない——書き換えた時点で「サンプルが通ること」を見ていない。
        let request = json!({ "stats": stats });
        let (status, response) = server.put(&stats_path(handle), &request).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "サンプルの stats をそのまま投入できること: 要求 = {request}, 応答 = {response}"
        );
        stats_seeded += 1;
    }

    assert!(
        stats_seeded > 0,
        "少なくとも1件は stats を投入していること（0 なら PUT 側を何も確かめていない）"
    );

    server.stop().await;
}

/// 投入スクリプトの**実物**。写しではない（作業単位 0004b-3 の追補）。
///
/// `include_str!` はビルドの入力なので、**スクリプトを書き換えればこのテストバイナリは
/// 必ず再コンパイルされる。** 古い版が使い回されて緑になることが無い
/// （`SAMPLE_SEED_JSON` と同じ手口）。
const SEED_SCRIPT: &str = include_str!("../scripts/seed-characters.ps1");

/// 見た目を揃えるための連続空白を1つに潰す（`handle        = $handle` → `handle = $handle`）。
///
/// **桁揃えの空白まで固定すると、無害な整形で赤くなる見張り番になる。**
/// 潰すのは半角空白の連続だけ（改行・タブは触らない）。
///
/// **文字列リテラルの中も区別せずに潰れる**（`Write-Host "  $($_.id)"` は
/// `" $($_.id)"` になる）ので、**連続空白そのものを見る needle は書けない。**
fn collapse_spaces(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut previous_was_space = false;
    for ch in source.chars() {
        let is_space = ch == ' ';
        if !(is_space && previous_was_space) {
            out.push(ch);
        }
        previous_was_space = is_space;
    }
    out
}

/// **投入スクリプトが管理 API の綴りを書いていること。**（作業単位 0004b-3 の追補）
///
/// # なぜ要るのか
///
/// 0004b の改名で `name` → `handle` に揃えたとき、`scripts/seed-characters.ps1` の
/// **`handle = $handle` を `name = $handle` に戻してもスイートは 781 全緑**だった（実測）。
/// スクリプトには「合わせ忘れると投入が壊れる」と書いてあるのに、
/// **機械的には1つも守られていなかった**——`characters.sample.json` を
/// `include_str!` で塞いだのとまったく同じ穴が、PS1 側に残っていた。
///
/// # **この検査が覆うもの — 綴りの存在だけ**
///
/// 見ているのは「この文字列がスクリプトに現れるか」**だけ**である。
/// **スクリプトが正しく動くことは1行も保証しない。** 具体的に覆えないもの:
///
/// * **PowerShell は起動していない。** 構文が壊れていても、実行が即座に落ちても緑。
/// * 制御フロー（`Test-Property` によるキーの取捨、409 の分岐、`continue` の位置、
///   `$script:Failures` の集計）。**行が現れていれば、到達不能でも緑。**
/// * 判定に使う値（`-eq 201` / `-eq 409` / `-eq 200`）、送信の順序、
///   UTF-8 バイト列への変換、`ConvertTo-Json -Depth 10`。
/// * サーバが実際に 201 / 200 を返すこと（それは
///   `every_character_in_the_sample_seed_file_is_accepted_as_is` の側）。
///
/// # 見る綴りの選び方
///
/// **要求本文・ルート・応答の読み出しで、サーバと1文字も違ってはいけない箇所**に限る。
/// 綴りを間違えたときの壊れ方を添える（400 で落ちるものと、**黙って落ちる**ものがある）。
///
/// **needle は「その綴りが使われている行の形」まで含めること。** 単語だけを見ると
/// `.NOTES` のコメント欄（`"handle": "aoi",`）や別の用途の行
/// （`$handle = $character.handle`）で満たされてしまい、**綴りを変えても緑になる。**
/// いまの一覧はどれもコメント欄とは一致しない形になっている
/// ——`handle = $handle` を `name = $handle` に変える変異が実際に赤になることは
/// 測って確認した（コメント欄には `handle` が3回残っているが、それでは満たされない）。
#[test]
fn the_seed_script_still_spells_the_admin_api_the_way_the_server_expects() {
    let script = collapse_spaces(SEED_SCRIPT);

    for (needle, breakage) in [
        (
            "Test-Property $document 'characters'",
            "トップレベルのキー。違えば1件も投入されない",
        ),
        (
            "$handle = $character.handle",
            "JSON 側の識別子キー。違えば識別子が空のまま組み立てられる",
        ),
        (
            "handle = $handle",
            "作成本文の識別子キー。`name` へ戻すと deny_unknown_fields で 400（実測で無防備だった箇所）",
        ),
        (
            "base_persona = $character.base_persona",
            "作成本文の必須キー。違えば 400",
        ),
        (
            "$create['display_name'] = $character.display_name",
            "省略可キー。違うと 400 か、**黙って handle 埋めの既定に落ちる**（ADR-0011）",
        ),
        (
            "$create['output_format_instruction'] = $character.output_format_instruction",
            "省略可キー。違うと 400 か、**黙って既定の形式指示に落ちる**",
        ),
        (
            "$create['stat_scale'] = $character.stat_scale",
            "省略可キー。違うと 400 か、**黙って既定の尺度（ADR-0015）に落ちる**",
        ),
        (
            "-Route '/admin/characters' -Body $create",
            "作成のルート。ここだけは POST 本文との組み合わせも1行で見ている",
        ),
        (
            "Test-Property $character 'stats'",
            "JSON 側の stats。違えば性格ステータスが**黙って1件も投入されない**",
        ),
        (
            "\"/admin/characters/$([uri]::EscapeDataString($handle))/stats\"",
            "置き換えのルート。識別子をエスケープせずに埋めないこと",
        ),
        (
            "-Method 'PUT' -Route $route -Body @{ stats = @($character.stats) }",
            "置き換え本文のキー。`@()` を外すと1軸のとき配列にならない",
        ),
        (
            "\"$($_.axis)=$($_.value)\"",
            "応答の読み出し（**表示だけ**。投入の成否は変わらない）",
        ),
        (
            "\"$script:BaseUrl/v1/models\"",
            "投入後の確認に使う一覧のパス",
        ),
        (
            "($models.Content | ConvertFrom-Json).data",
            "一覧の応答のキー。違えば確認表示が空になる",
        ),
    ] {
        assert!(
            script.contains(needle),
            "投入スクリプトが管理 API の綴りと食い違っている: {needle:?}\n\
             壊れ方: {breakage}\n\
             （綴りを変えたなら scripts/seed-characters.ps1 と揃えること。\
             この検査は綴りの存在しか見ていないので、スクリプトの動作は別途手で確かめること）"
        );
    }
}

/// **サンプル JSON のキーは、投入スクリプトのどこかに綴られている。**（作業単位 0004b-3 の追補）
///
/// 狙いは1つだけ——**サンプルにキーを足したのに、スクリプトがそれを送っていない**状態の検出。
/// この形の取りこぼしは
/// `every_character_in_the_sample_seed_file_is_accepted_as_is` では**緑を通る**
/// （あちらは JSON を直接 API へ投げるので、スクリプトが何を送るかを見ていない）。
///
/// # **これは上の検査より弱い**
///
/// 見るのは**綴りが1回でも現れるか**だけなので、`.NOTES` のコメント欄に書いてあれば通る。
/// 「スクリプトがそのキーを送っている」ことは**保証しない**。
/// 送っている形まで見るのは上の
/// `the_seed_script_still_spells_the_admin_api_the_way_the_server_expects` の側で、
/// そちらは綴りを**手で選んでいる**（サンプルに無いキーも見られる代わりに、
/// サンプルの増減には追随しない）。**役割が違うので片方に寄せない。**
#[test]
fn every_key_in_the_sample_seed_file_is_spelled_somewhere_in_the_seed_script() {
    fn collect_keys(value: &Value, into: &mut std::collections::BTreeSet<String>) {
        match value {
            Value::Object(fields) => {
                for (key, child) in fields {
                    into.insert(key.clone());
                    collect_keys(child, into);
                }
            }
            Value::Array(items) => {
                for item in items {
                    collect_keys(item, into);
                }
            }
            _ => {}
        }
    }

    let mut keys = std::collections::BTreeSet::new();
    collect_keys(&parse_json(SAMPLE_SEED_JSON), &mut keys);
    // **空なら何も確かめていない**（キーが1つも取れないと下のループが回らない）。
    assert!(
        keys.len() >= 5,
        "サンプルからキーを取り出せていること: {keys:?}"
    );

    for key in &keys {
        assert!(
            SEED_SCRIPT.contains(key.as_str()),
            "サンプルにあるキーを投入スクリプトが一度も綴っていない: {key:?}\n\
             （スクリプトが送っていない可能性がある。scripts/seed-characters.ps1 を確認すること）"
        );
    }
}

/// **同じ表示名で、識別子の違うキャラクターを2件作れる**（ADR-0011 / この作業単位の存在理由）。
///
/// `aoi` と `aoi-v2` が**どちらもプロンプト上は「葵」**として共存する。
/// これができないと、ペルソナの変種を並べるために `葵-実験A` のような名前を付けることになり、
/// **比べたいのはペルソナなのにプロンプトへ実験用の名前が載る。**
#[tokio::test]
async fn two_characters_may_share_a_display_name() {
    let server = TestServer::start().await;

    for (handle, persona) in [
        ("aoi", "落ち着いた物腰の司書。"),
        ("aoi-v2", "快活な司書。"),
    ] {
        let (status, body) = server
            .post(
                "/admin/characters",
                &json!({
                    "handle": handle,
                    "display_name": " 葵 ",
                    "base_persona": persona,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let created = parse_json(&body);
        assert_eq!(created["handle"], json!(handle), "{body}");
        assert_eq!(
            created["display_name"],
            json!("葵"),
            "表示名も正規化して保存すること: {body}"
        );
    }

    // 一覧では**識別子が別・呼び名が同じ**の2件として並ぶ。
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        names_in(&body, "characters"),
        vec!["aoi", "aoi-v2"],
        "{body}"
    );
    let characters = parse_json(&body);
    let display_names: Vec<&str> = characters["characters"]
        .as_array()
        .expect("配列であること")
        .iter()
        .map(|entry| entry["display_name"].as_str().expect("文字列であること"))
        .collect();
    assert_eq!(display_names, vec!["葵", "葵"], "呼び名は同じであること");

    // **`/v1/models` に並ぶのは識別子**（0004a の時点では `name`）。
    let (status, body) = server.get("/v1/models").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names_in(&body, "data"), vec!["aoi", "aoi-v2"], "{body}");

    server.stop().await;
}

/// **表示名は省略できる。省略時は名前で埋まる**（0004a 以前と同じ振る舞い）。
///
/// 既に動いている投入クライアント（`scripts/seed-characters.ps1`）を壊さないための既定。
#[tokio::test]
async fn an_omitted_display_name_falls_back_to_the_handle() {
    let server = TestServer::start().await;

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi-v2_7", "base_persona": "落ち着いた物腰の司書。"}),
        )
        .await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    let created = parse_json(&body);
    assert_eq!(created["handle"], json!("aoi-v2_7"), "{body}");
    assert_eq!(
        created["display_name"],
        json!("aoi-v2_7"),
        "識別子の綴りがそのまま呼び名になること（記号も落とさない）: {body}"
    );

    // 明示的な `null` も「省略」と同じ扱い（キーの有無で意味を変えない）。
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({"handle": "rin", "display_name": null, "base_persona": "ペルソナ本文"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(parse_json(&body)["display_name"], json!("rin"), "{body}");

    server.stop().await;
}

/// **性格ステータスの尺度が、本番の入口を通って往復する**（ADR-0015 / 作業単位 0011a-1）。
///
/// # 「本番の入口」であることが要点
///
/// `TestServer::start` は **`App::bind`**（モック注入用の別入口ではない）でサーバを起動する。
/// `POST` → `GET /admin/characters` → `GET .../stats` の3経路すべてで同じ値が見えるので、
/// **要求の反射でも、応答だけの取り繕いでもない**ことが分かる
/// （CLAUDE.md「テスト専用の入口を作ると、本番の入口が観測対象から外れる」）。
///
/// # 期待値に既定（10）を使わない
///
/// `TARGET_SCALE` は既定と違う値。既定を使うと
/// **「要求の尺度を捨てて常に `StatScale::DEFAULT` を書く」実装が緑を通る。**
/// 省略時の振る舞いは `an_omitted_stat_scale_falls_back_to_the_default` の担当。
///
/// **`value <= stat_scale` はここでは主張していない**（0011a-2 の持ち物。
/// 見張り番は `a_stat_value_above_the_characters_scale_is_refused_with_its_own_reason`）。
/// ここが投げる値はどれも尺度に収まっているので、**このテストが赤くなるのは
/// 尺度そのものが別の行から来たときだけ。**
///
/// # **囮を3件置く**（0011a-1 のレビュー指摘 / 実測付き）
///
/// 追補前のこのテストは**キャラクターを1件しか作っていなかった。**
/// そのため `/admin/characters/{handle}/stats` の尺度を
/// **「一覧の先頭（読める行）の尺度」に差し替える変異が `GET` / `PUT` の両方で緑を通った**
/// （798 passed / 0 failed。`store` 層は囮を置いていたので守れていたが、HTTP の入口だけ無防備）。
///
/// 尺度が別の行から来ても応答は「それらしい数」を返し続けるので、
/// **GUI には他人の上端が表示される**——ADR-0015 が尺度を独立項目にした理由が裏返る。
///
/// 囮の配置（`create_scale_decoys`）は「先頭 / 2件目 / 末尾 / 最小 / 最大」の
/// どれを取っても対象と食い違うように選んである。加えて**囮それぞれについても**
/// 「自分の尺度が返ること」を、**一覧・`GET` ・`PUT` の3経路すべてで**見る。
///
/// # 一覧経路は、囮を置いただけでは無防備だった（再レビューの実測）
///
/// 囮を3件置いた後も、**一覧については対象1件しか見ていなかった。**
/// `list_characters` の応答組み立て直後に「ある1行の尺度を全行へ配る」変異を差し込むと、
/// **対象がいる位置（識別子順で3番目 = `characters.get(2)`）を選んだときだけ緑を通った。**
///
/// | 全行へ配る元 | 一覧の検査を足す前 | 足した後 |
/// |---|---|---|
/// | `first()` | 3 failed | 3 failed |
/// | `get(1)` | 5 failed | 5 failed |
/// | `get(2)`（＝対象の位置） | **0 failed** | **1 failed** |
/// | `last()` | 5 failed | 5 failed |
/// | 最大 | 5 failed | 5 failed |
/// | 最小 | 3 failed | 3 failed |
///
/// （直列 / `cargo test --no-fail-fast` / 全 798 件 / 変異ごとに `Compiling personacore` を確認）
///
/// **測れているのはこの6通りだけ。** 「どの行を選んでも赤くなる」とは書かない
/// ——試していない選び方が残っている以上、それは測定ではなく期待にすぎない。
#[tokio::test]
async fn a_stat_scale_survives_the_round_trip_through_the_real_entry_point() {
    let server = TestServer::start().await;
    assert_fixture_values_are_distinct();

    // **対象より先に作る。** 識別子順でも対象の前後に並ぶ（`create_scale_decoys` の表）。
    let decoys = create_scale_decoys(&server).await;

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": TARGET_HANDLE,
                "display_name": TARGET_DISPLAY,
                "base_persona": TARGET_PERSONA,
                "stat_scale": TARGET_SCALE,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        parse_json(&body)["stat_scale"],
        json!(TARGET_SCALE),
        "作成の応答が要求した尺度を返すこと: {body}"
    );

    // 一覧（DB から読み直した姿）。
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        character_in(&body, TARGET_HANDLE)["stat_scale"],
        json!(TARGET_SCALE),
        "一覧にも同じ尺度が出ること: {body}"
    );

    // **囮も一覧の中で自分の尺度を持つ。** 対象の1件だけを見ていると、
    // **一覧が「ある1行の尺度を全行へ配る」実装**を通してしまう。
    // 対象は識別子順で3番目なので、`characters.get(2)` を全行へ写す変異は
    // 対象の値だけは正しいまま——実測で 798 passed / 0 failed だった（0011a-1 の再レビュー）。
    // 下の `GET` / `PUT` の囮ループは**一覧経路を通らない**ので、ここが要る。
    for (handle, scale) in decoys {
        assert_eq!(
            character_in(&body, handle)["stat_scale"],
            json!(scale),
            "一覧: {handle} は**自分の**尺度を持つこと（対象は {TARGET_SCALE}）: {body}"
        );
    }

    // **性格ステータスの応答にも載る。** ADR-0015 が独立項目にした理由の1つが
    // 「GUI で『この軸は 0〜N の整数』と出せること」なので、ここが返せないと
    // 呼び出し側は軸の一覧と尺度を別々に取りに行くことになる。
    let (status, body) = server
        .put(
            &stats_path(TARGET_HANDLE),
            // **値は尺度（7）と別の数にする。** 同値にすると、応答が値の欄に
            // 尺度を書いていても気付けない。
            &json!({"stats": [{"axis": "パッション", "value": 6}]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body),
        json!({
            "character": TARGET_HANDLE,
            "stat_scale": TARGET_SCALE,
            "stats": [{"axis": "パッション", "value": 6}],
        }),
        "置き換えの応答に尺度が載ること: {body}"
    );

    let (status, body) = server.get(&stats_path(TARGET_HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["stat_scale"],
        json!(TARGET_SCALE),
        "取得の応答にも尺度が載ること: {body}"
    );

    // --- **囮も自分の尺度を返す** ----------------------------------------
    //
    // ここが対象と同じ値を返したら、尺度は「引いた行」ではなく
    // どこか固定の1行から来ている。**`GET` と `PUT` の両方**を見る
    // （どちらも `find_character` の結果とは別の場所から取れてしまう形）。
    for (handle, scale) in decoys {
        let (status, body) = server.get(&stats_path(handle)).await;
        assert_eq!(status, StatusCode::OK, "{handle}: {body}");
        assert_eq!(
            parse_json(&body)["stat_scale"],
            json!(scale),
            "GET: {handle} は**自分の**尺度を返すこと（対象は {TARGET_SCALE}）: {body}"
        );

        let (status, body) = server
            .put(
                &stats_path(handle),
                &json!({"stats": [{"axis": "共感", "value": 1}]}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{handle}: {body}");
        assert_eq!(
            parse_json(&body)["stat_scale"],
            json!(scale),
            "PUT: {handle} は**自分の**尺度を返すこと（対象は {TARGET_SCALE}）: {body}"
        );
    }

    // 対象の尺度が囮の `PUT` で書き換わっていないこと（置き換えは軸だけを触る）。
    let (_, body) = server.get(&stats_path(TARGET_HANDLE)).await;
    assert_eq!(
        parse_json(&body)["stat_scale"],
        json!(TARGET_SCALE),
        "囮を触っても対象の尺度は変わらないこと: {body}"
    );

    server.stop().await;
}

/// **尺度は省略できる。省略時は既定（ADR-0015）が配られる。**
///
/// 既に動いている投入クライアント（`scripts/seed-characters.ps1` /
/// `scripts/characters.sample.json`）は尺度を送らないので、
/// 必須にすると `deny_unknown_fields` ではなく**欠落**で 400 になる。
///
/// **このテストだけが `DEFAULT_SCALE` を期待値にしてよい。**
#[tokio::test]
async fn an_omitted_stat_scale_falls_back_to_the_default() {
    let server = TestServer::start().await;

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi", "base_persona": "落ち着いた物腰の司書。"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        parse_json(&body)["stat_scale"],
        json!(DEFAULT_SCALE),
        "省略時は既定の尺度になること: {body}"
    );

    // 明示的な `null` も「省略」と同じ扱い（表示名と揃える）。
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({"handle": "rin", "stat_scale": null, "base_persona": "ペルソナ本文"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        parse_json(&body)["stat_scale"],
        json!(DEFAULT_SCALE),
        "{body}"
    );

    // 一覧から見ても既定（応答だけが既定になっていないこと）。
    let (_, body) = server.get("/admin/characters").await;
    for handle in ["aoi", "rin"] {
        assert_eq!(
            character_in(&body, handle)["stat_scale"],
            json!(DEFAULT_SCALE),
            "{handle}: {body}"
        );
    }

    server.stop().await;
}

/// 不正な尺度は**理由ごとに区別できる**（小さすぎる / 大きすぎる）。
///
/// 本文は**完全一致で固定**する。入力も上下限も反射しない
/// （`ScaleError` の `Display` は定数文字列 / CLAUDE.md 絶対制約）。
///
/// **「小さすぎる」と「大きすぎる」を潰さないこと。** 潰すと
/// 送り直すときに増やすのか減らすのかが応答から読めない。
#[tokio::test]
async fn invalid_stat_scales_are_rejected_with_distinguishable_reasons() {
    let server = TestServer::start().await;

    for (scale, message, code) in [
        (
            json!(0),
            "性格ステータスの尺度が小さすぎます",
            "stat_scale_too_small",
        ),
        (
            json!(101),
            "性格ステータスの尺度が大きすぎます",
            "stat_scale_too_large",
        ),
        (
            json!(u64::from(u32::MAX)),
            "性格ステータスの尺度が大きすぎます",
            "stat_scale_too_large",
        ),
    ] {
        let (status, body) = server
            .post(
                "/admin/characters",
                &json!({
                    "handle": "aoi",
                    "base_persona": "ペルソナ本文",
                    "stat_scale": scale,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{code}: {body}");
        assert_eq!(parse_json(&body), invalid_request(message, code), "{code}");
        assert!(
            !body.contains("100") && !body.contains("101"),
            "上下限も入力も反射しないこと: {body}"
        );
    }

    // **負値と小数は JSON の型として弾かれる**（`u32` にデコードできない）。
    // 理由の粒度は落ちるが、`invalid_body` は入力を反射しない。
    for scale in [json!(-1), json!(2.5)] {
        let (status, body) = server
            .post(
                "/admin/characters",
                &json!({
                    "handle": "aoi",
                    "base_persona": "ペルソナ本文",
                    "stat_scale": scale,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{scale}: {body}");
        assert_eq!(
            parse_json(&body),
            invalid_request("リクエスト本文の形式が不正です", "invalid_body"),
            "{scale}"
        );
    }

    // **1件も作られていない。**
    assert_eq!(server.character_count().await, 0);

    server.stop().await;
}

/// 不正な表示名は**理由ごとに区別できる**（空 / 空白のみ / 制御文字 / 長すぎる）。
///
/// 本文は**完全一致で固定**する。入力を反射しないことも同時に保証される。
/// **識別子のエラーに潰さない。** 作成要求には `handle` と `display_name` の
/// 両方が載るので、同じ文言だとどちらの欄を直せばよいのか応答から読めない。
///
/// 表示名は**削れない枠**に載るので、上限が無いと巨大な値を1件入れただけで
/// 以後のすべてのチャットが予算超過で失敗する。
/// **気付くのは「そのキャラクターと話せなくなってから」**なので入口で止める
/// （重さの権威は `domain::character::Persona` の doc の 0012a の注記。ペルソナと同じ構造）。
#[tokio::test]
async fn invalid_display_names_are_rejected_with_distinguishable_reasons() {
    let server = TestServer::start().await;

    // 上限は `domain::DisplayName::MAX_CHARS`。境界は `domain` の単体テストが固定するので、
    // ここでは**確実に超える長さ**を送る。
    let too_long = "あ".repeat(1_000);
    let cases = [
        ("", "表示名が空です", "display_name_empty"),
        ("\u{3000} \t", "表示名が空です", "display_name_empty"),
        // **単独の行区切り・制御文字は「空」側**（作業単位 0009b）。
        // `DisplayName::parse` が `is_effectively_blank` を先に見るため。
        // 対応そのものは
        // `domain::character::tests::display_name_reports_empty_rather_than_control_when_nothing_is_visible`
        // が固定しており、ここは**利用者に届くコード**が同じ側に落ちることを見る。
        ("\u{2028}", "表示名が空です", "display_name_empty"),
        ("\u{0007}", "表示名が空です", "display_name_empty"),
        (
            "葵\u{0007}碧",
            "表示名に改行や制御文字は使えません",
            "display_name_control",
        ),
        (
            too_long.as_str(),
            "表示名が長すぎます",
            "display_name_too_long",
        ),
    ];

    for (display_name, message, code) in cases {
        let (status, body) = server
            .post(
                "/admin/characters",
                &json!({
                    "handle": "aoi",
                    "display_name": display_name,
                    "base_persona": "ペルソナ本文",
                }),
            )
            .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            parse_json(&body),
            invalid_request(message, code),
            "理由ごとに区別できること: display_name = {display_name:?}"
        );
    }

    // **`@` は表示名では正当**（禁止しているのは `model` に載る識別子の側だけ / ADR-0011）。
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": "aoi",
                "display_name": "葵@図書館",
                "base_persona": "ペルソナ本文",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(parse_json(&body)["display_name"], json!("葵@図書館"));

    // 弾かれた要求は**1件も作っていない**（識別子が空いたままであること）。
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names_in(&body, "characters"), vec!["aoi"], "{body}");

    server.stop().await;
}

/// キャラクターが1件も無ければ**空の一覧**（エラーにしない）。
///
/// 読み出せない行の欄（`unreadable`）も**空配列で必ず出る**。キーごと落とすと、
/// 「壊れた行が無い」と「壊れた行を見せない古い版」が区別できない（ADR-0016 §3）。
#[tokio::test]
async fn the_character_listing_is_empty_before_anything_is_created() {
    let server = TestServer::start().await;

    let (status, body) = server.get("/admin/characters").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parse_json(&body),
        json!({"characters": [], "unreadable": []})
    );

    server.stop().await;
}

/// **壊れた行があっても一覧は落ちず、修復の手掛かりが応答に出る**
/// （ADR-0016 §3 / 作業単位 0012b）。
///
/// 以前は不正な行が1件あるだけで `GET /admin/characters` も `GET /v1/models` も
/// **全キャラクターぶん 500** になり、正常なキャラの一覧すら見られなかった。
/// **更新 API は名前で引く**ので、ここに壊れた行の識別子が出ないと直しに行けない。
///
/// `/v1/models` の中身は変えない（あちらは「今そのまま話せるキャラクター」の一覧で、
/// 壊れた行を混ぜると**選べるのに必ず失敗するモデル**になる）。
#[tokio::test]
async fn a_broken_row_is_pointed_at_without_breaking_the_listings() {
    let server = TestServer::start().await;

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": "aoi",
                "display_name": "葵",
                "base_persona": "落ち着いた物腰の司書。",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // **0009 以前は `/admin/` から作れた表示名**（U+2028 は `char::is_control` が偽）。
    sqlx::query("INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)")
        .bind("rin")
        .bind("凛\u{2028}花")
        .bind("夜勤の警備員。")
        .execute(server.store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");
    let broken_id: i64 = sqlx::query_scalar("SELECT id FROM character WHERE handle = ?")
        .bind("rin")
        .fetch_one(server.store.pool())
        .await
        .expect("壊れた行の id を取得できること");

    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "1件壊れていても一覧そのものは成功すること: {body}"
    );
    assert_eq!(
        names_in(&body, "characters"),
        vec!["aoi"],
        "正常な行が巻き添えにならないこと: {body}"
    );

    let listed = parse_json(&body);
    let unreadable = listed["unreadable"]
        .as_array()
        .unwrap_or_else(|| panic!("unreadable は配列であること: {body}"));
    assert_eq!(
        unreadable.len(),
        1,
        "壊れた行を黙って読み飛ばさないこと: {body}"
    );
    assert_eq!(
        unreadable[0]["id"],
        json!(broken_id),
        "壊れた行を指す id が出ること: {body}"
    );
    assert_eq!(
        unreadable[0]["handle"],
        json!("rin"),
        "更新 API に渡せる識別子が出ること: {body}"
    );
    assert_eq!(
        unreadable[0]["invalid_columns"],
        json!(["display_name"]),
        "どの列が読めなかったかが分かること: {body}"
    );
    assert!(
        !body.contains('凛') && !body.contains('\u{2028}'),
        "壊れた列の格納値を応答に載せないこと: {body}"
    );

    // **`/v1/models` は今までどおり読める行だけ。**
    let (status, body) = server.get("/v1/models").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        names_in(&body, "data"),
        vec!["aoi"],
        "壊れた行を選べるモデルとして並べないこと: {body}"
    );

    server.stop().await;
}

/// **識別子そのものが `parse` を通らない行も、HTTP の応答に出る**
/// （ADR-0016 §3 / 作業単位 0012b）。
///
/// # なぜ `CharacterListView` を直接 JSON にするだけでは足りないのか
///
/// この形（`handle` が `null`）は**最も直せない行**で、`id` だけが手掛かりになる。
/// にもかかわらず、ハンドラを通さずに `CharacterListView` を組み立てて
/// シリアライズするテストしか無かったため、**`list_characters` が `handle: None` の行を
/// 落としても全テストが緑**だった（レビュアーの実測）。
/// 「テスト専用の入口を作ると本番の入口が観測対象から外れる」の再発形なので、
/// ここは**必ず `GET /admin/characters` を通す。**
///
/// # 落とすとどうなるか
///
/// 応答は `"unreadable": []`。運用者は「壊れた行は無い」と読むが、
/// その行は `/v1/models` にも出ないので**完全に不可視のまま正規化キーだけを塞ぎ続ける**
/// （ADR-0004 の袋小路そのもの）。`0004b` の移行が途中で落ちたときに残るのがこの形。
#[tokio::test]
async fn a_row_whose_handle_cannot_be_parsed_is_pointed_at_by_id_alone() {
    let server = TestServer::start().await;

    // 正常な行（壊れた行の巻き添えを見る。id も 1 から始まらなくなる）。
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": "aoi",
                "display_name": "葵",
                "base_persona": "落ち着いた物腰の司書。",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // **`Handle` を作れない名前**（上限超過）。`model` からも
    // `find_character_by_handle` からも決して引けないので、id が唯一の手掛かりになる。
    //
    // **文字集合の CHECK（0005）には触れない綴り**にしてある。引っかかるのは
    // アプリ層の `MAX_CHARS` だけ、という状況をわざと作る。
    let unparsable = "a".repeat(Handle::MAX_CHARS + 1);
    sqlx::query("INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)")
        .bind(&unparsable)
        .bind("凛")
        .bind("夜勤の警備員。")
        .execute(server.store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること（長さはスキーマでは縛らない）");
    let broken_id: i64 = sqlx::query_scalar("SELECT id FROM character WHERE handle = ?")
        .bind(&unparsable)
        .fetch_one(server.store.pool())
        .await
        .expect("壊れた行の id を取得できること");

    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "識別子が読めない行があっても一覧そのものは成功すること: {body}"
    );
    let listed = parse_json(&body);
    assert_eq!(
        names_in(&body, "characters"),
        vec!["aoi"],
        "正常な行が巻き添えにならないこと: {body}"
    );
    assert_eq!(
        listed["unreadable"],
        json!([{"id": broken_id, "handle": null, "invalid_columns": ["handle"]}]),
        "`handle` が null の行こそ応答に出ること（id だけが手掛かり）: {body}"
    );
    assert!(
        !body.contains('名'),
        "壊れた列の格納値を応答に載せないこと: {body}"
    );

    // **`/v1/models` には出ない。** 出しても `id` に載せられる識別子が無い。
    let (status, body) = server.get("/v1/models").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        names_in(&body, "data"),
        vec!["aoi"],
        "識別子が読めない行を選べるモデルとして並べないこと: {body}"
    );

    server.stop().await;
}

/// **ペルソナの上限の境界が HTTP まで通っている**（作業単位 12b で 1000 → 600）。
///
/// `Persona::MAX_CHARS` ちょうどは作成でき、1文字超えたら 400。
/// **`Persona::MAX_CHARS` を使わず 600 / 601 を直に書く**——定数を参照すると
/// 「上限がいくつであっても通る」テストになり、**下げる変更を1つも検出しない。**
/// 数字を直に書いた見張り番は `domain` 側の
/// `persona_limit_stays_in_the_intended_range` と対になっている。
#[tokio::test]
async fn the_persona_limit_is_six_hundred_characters_at_the_http_boundary() {
    let server = TestServer::start().await;

    // **前提を先に固定する。** ここが崩れたら以下の 600 / 601 は境界ではない。
    assert_eq!(Persona::MAX_CHARS, 600, "ADR-0009 が決めた上限であること");

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": "aoi",
                "display_name": "葵",
                "base_persona": "あ".repeat(600),
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "600 文字は通ること: {body}");

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": "rin",
                "display_name": "凛",
                "base_persona": "あ".repeat(601),
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "601 文字は拒否されること: {body}"
    );
    assert_eq!(
        server.character_count().await,
        1,
        "拒否した要求で行が増えないこと"
    );

    server.stop().await;
}

/// **上限を下げたことで読めなくなった行は、`PATCH` で直る**（作業単位 12b）。
///
/// `Persona::MAX_CHARS` を 1000 → 600 に下げたので、**それ以前に保存された
/// 601〜1000 文字のペルソナは `CharacterEntry::Unreadable` になる。**
/// ADR-0009 は「`PATCH` で直せる」を前提に下げる判断をしているので、
/// **「直せるはず」ではなく実際に直す手順をここで測る。**
///
/// 見ているのは3点:
///
/// 1. 保存済みの 601 文字の行が**一覧では壊れた行として指し示される**（消えない）。
/// 2. その状態でも `PATCH` が**届く**（更新は既存の行を読まない）。
/// 3. 直した後は**普通に読める行に戻る**。
#[tokio::test]
async fn a_persona_over_the_lowered_limit_is_unreadable_until_patched() {
    let server = TestServer::start().await;

    // 巻き添えの見張り役。
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": "aoi",
                "display_name": "葵",
                "base_persona": "落ち着いた物腰の司書。",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // **上限を下げる前なら作れた長さ**（601 文字）。スキーマは長さを縛らない。
    let stale = "あ".repeat(601);
    sqlx::query("INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)")
        .bind("rin")
        .bind("凛")
        .bind(&stale)
        .execute(server.store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること（長さはスキーマでは縛らない）");

    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "一覧そのものは成功すること: {body}");
    let listed = parse_json(&body);
    assert_eq!(
        names_in(&body, "characters"),
        vec!["aoi"],
        "正常な行が巻き添えにならないこと: {body}"
    );
    let unreadable = listed["unreadable"]
        .as_array()
        .unwrap_or_else(|| panic!("unreadable は配列であること: {body}"));
    assert_eq!(unreadable.len(), 1, "壊れた行を黙って消さないこと: {body}");
    assert_eq!(unreadable[0]["handle"], json!("rin"), "{body}");
    assert_eq!(
        unreadable[0]["invalid_columns"],
        json!(["base_persona"]),
        "どの列が読めなくなったかが分かること: {body}"
    );
    assert!(
        !body.contains('あ'),
        "格納値そのものを応答に載せないこと: {body}"
    );

    // **`/v1/models` からも消えている**（選べるのに必ず失敗するモデルにしない）。
    let (status, body) = server.get("/v1/models").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(names_in(&body, "data"), vec!["aoi"], "{body}");

    // **ここが本題: `PATCH` が届く。** 更新は既存の行を読まないので、
    // 読めない行にも書ける。600 文字ちょうど（＝新しい上限）に詰め直す。
    let repaired = "い".repeat(600);
    let (status, body) = server
        .patch(
            &character_path("rin"),
            &json!({ "base_persona": repaired.clone() }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "読めない行にも更新が届くこと: {body}"
    );

    // 直った。一覧にも `/v1/models` にも戻る。
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed = parse_json(&body);
    assert_eq!(
        listed["unreadable"],
        json!([]),
        "壊れた行が残らないこと: {body}"
    );
    assert_eq!(names_in(&body, "characters"), vec!["aoi", "rin"], "{body}");

    let (status, body) = server.get("/v1/models").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(names_in(&body, "data"), vec!["aoi", "rin"], "{body}");

    // **格納されたのは詰め直した本文**（更新が空振りしていない）。
    let stored: String = sqlx::query_scalar("SELECT base_persona FROM character WHERE handle = ?")
        .bind("rin")
        .fetch_one(server.store.pool())
        .await
        .expect("引けること");
    assert_eq!(stored, repaired, "601 文字が残っていないこと");
    assert_ne!(stored, stale, "前提: 直す前後で別の文字列であること");

    server.stop().await;
}

/// 性格ステータスは**丸ごと置き換え**。前の軸は残らない。
#[tokio::test]
async fn stats_are_replaced_wholesale() {
    let server = TestServer::start().await;
    server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi", "base_persona": "ペルソナ本文", "stat_scale": TARGET_SCALE}),
        )
        .await;

    // 設定前は空。**未設定と「キャラクターが無い」は別物**なので 200 で返る。
    let (status, body) = server.get(&stats_path("aoi")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parse_json(&body),
        json!({"character": "aoi", "stat_scale": TARGET_SCALE, "stats": []})
    );

    let (status, body) = server
        .put(
            &stats_path("aoi"),
            &json!({"stats": [
                {"axis": "ロジック", "value": 5},
                {"axis": "パッション", "value": 3},
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body),
        json!({"character": "aoi", "stat_scale": TARGET_SCALE, "stats": [
            {"axis": "ロジック", "value": 5},
            {"axis": "パッション", "value": 3},
        ]}),
        "設定した順序のまま返ること（辞書順に並べ替えない）: {body}"
    );

    let (status, body) = server.get(&stats_path("aoi")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parse_json(&body),
        json!({"character": "aoi", "stat_scale": TARGET_SCALE, "stats": [
            {"axis": "ロジック", "value": 5},
            {"axis": "パッション", "value": 3},
        ]})
    );

    // **置き換え。** 前の軸が残ったら差分更新になっている。
    let (status, body) = server
        .put(
            &stats_path("aoi"),
            &json!({"stats": [{"axis": "共感", "value": 2}]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = server.get(&stats_path("aoi")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parse_json(&body),
        json!({"character": "aoi", "stat_scale": TARGET_SCALE, "stats": [{"axis": "共感", "value": 2}]}),
        "前の軸が残らないこと: {body}"
    );
    assert!(
        !body.contains("ロジック") && !body.contains("パッション"),
        "前の軸が残らないこと: {body}"
    );

    // 空配列で全消し。
    let (status, _) = server.put(&stats_path("aoi"), &json!({"stats": []})).await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = server.get(&stats_path("aoi")).await;
    assert_eq!(
        parse_json(&body),
        json!({"character": "aoi", "stat_scale": TARGET_SCALE, "stats": []})
    );

    server.stop().await;
}

/// **値 0 の軸も保存・取得できる。** 落とすのは `prompt` 層の責務。
#[tokio::test]
async fn a_zero_valued_axis_survives_the_round_trip() {
    let server = TestServer::start().await;
    server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi", "base_persona": "ペルソナ本文", "stat_scale": TARGET_SCALE}),
        )
        .await;

    let (status, body) = server
        .put(
            &stats_path("aoi"),
            &json!({"stats": [
                {"axis": "パッション", "value": 0},
                {"axis": "ロジック", "value": 5},
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = server.get(&stats_path("aoi")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parse_json(&body),
        json!({"character": "aoi", "stat_scale": TARGET_SCALE, "stats": [
            {"axis": "パッション", "value": 0},
            {"axis": "ロジック", "value": 5},
        ]}),
        "値 0 の軸を保存段階で落とさないこと: {body}"
    );

    server.stop().await;
}

/// 前後に空白のある軸名は正規化して保存される（軸が分裂しない）。
#[tokio::test]
async fn axis_names_are_normalized() {
    let server = TestServer::start().await;
    server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi", "base_persona": "ペルソナ本文", "stat_scale": TARGET_SCALE}),
        )
        .await;

    let (status, body) = server
        .put(
            &stats_path("aoi"),
            &json!({"stats": [{"axis": " \u{3000}パッション\t", "value": 3}]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body),
        json!({"character": "aoi", "stat_scale": TARGET_SCALE, "stats": [{"axis": "パッション", "value": 3}]}),
        "軸名を正規化して保存すること: {body}"
    );

    server.stop().await;
}

/// 不正な**識別子**は理由ごとに区別できる（`@` / 空白のみ / 制御文字 / 長すぎる）。
///
/// 本文は**完全一致で固定**する。入力を反射しないことも同時に保証される。
///
/// # 文言が「識別子」であること自体が見張り対象（作業単位 0004b-3）
///
/// 列も型も `handle` に揃えたのに応答だけ「キャラクター名」のままだと、
/// **受け取った側は `handle` と `display_name` のどちらを直せばよいのか読めない**
/// （作成要求には両方が載る）。`display_name` 側の文言（「表示名が…」）と
/// **同じ語を使わない**ことも、この対で固定している。
///
/// **長すぎる識別子も弾く。** 識別子は `model` フィールドに載る（ADR-0004）ので、
/// `GET /v1/models` を通じて**クライアントのモデル選択 UI を壊す**。
/// 上限は `domain::Handle::MAX_CHARS`。ここでは**確実に超える長さ**を送る
/// （境界そのものは `domain` の単体テストが定数で固定している）。
#[tokio::test]
async fn invalid_handles_are_rejected_with_distinguishable_reasons() {
    let server = TestServer::start().await;
    let oversized = "葵".repeat(10_000);

    for (raw, expected) in [
        (
            "aoi@new",
            invalid_request(
                "キャラクター識別子に @ は使えません",
                "character_handle_at_sign",
            ),
        ),
        (
            " \u{3000}\t ",
            invalid_request("キャラクター識別子が空です", "character_handle_empty"),
        ),
        (
            "a\u{0007}b",
            invalid_request(
                "キャラクター識別子に改行や制御文字は使えません",
                "character_handle_control",
            ),
        ),
        (
            oversized.as_str(),
            invalid_request(
                "キャラクター識別子が長すぎます",
                "character_handle_too_long",
            ),
        ),
        // **規則外の文字**（ADR-0017 で大文字・空白・記号・日本語が全部ここに落ちる）。
        // 0004b-3 の変異検査で判明: この行が無いと、`character_handle_charset` の
        // **文言を他の理由と同じ文字列にしても、スイート全体が緑のまま**通る（実測）。
        // 実運用でいちばん多く返る理由なので、他の4つと同じ強さで固定しておく。
        (
            "Aoi",
            invalid_request(
                "キャラクター識別子に使えない文字が含まれています",
                "character_handle_charset",
            ),
        ),
    ] {
        let (status, body) = server
            .post(
                "/admin/characters",
                &json!({"handle": raw, "base_persona": "ペルソナ本文"}),
            )
            .await;

        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "長さ {} の名前: {body}",
            raw.chars().count()
        );
        assert_eq!(parse_json(&body), expected, "長さ {}", raw.chars().count());
        assert!(
            !body.contains('葵') && !body.contains("10000"),
            "入力も上限値も反射しないこと: {body}"
        );
    }

    // どれも作成されていないこと（弾いたつもりで書き込んでいない）。
    let (_, body) = server.get("/admin/characters").await;
    assert_eq!(
        parse_json(&body),
        json!({"characters": [], "unreadable": []})
    );

    server.stop().await;
}

/// **ペルソナ本文も検証される**（空 / 空白のみ / 長すぎる）。
///
/// ペルソナは**削れない枠**に載るので、巨大なペルソナを持つキャラクターは
/// 以後のすべてのチャットが予算超過で失敗する。
/// **気付くのは「そのキャラクターと話せなくなってから」**なので入口で止める
/// （重さの権威は `domain::character::Persona` の doc の 0012a の注記。
/// `memory` の本文上限と同じ構造）。
#[tokio::test]
async fn invalid_personas_are_rejected_with_distinguishable_reasons() {
    let server = TestServer::start().await;

    let empty = invalid_request("ペルソナ本文が空です", "persona_empty");
    let too_long = invalid_request("ペルソナ本文が長すぎます", "persona_too_long");
    // 上限は `domain::Persona::MAX_CHARS`。ここでは**確実に超える長さ**を送る
    // （境界そのものは `domain` の単体テストが定数で固定している）。
    let oversized = "あ".repeat(100_000);

    for (raw, expected) in [
        ("", &empty),
        ("   ", &empty),
        (" \u{3000}\t\n", &empty),
        (oversized.as_str(), &too_long),
    ] {
        let (status, body) = server
            .post(
                "/admin/characters",
                &json!({"handle": "aoi", "base_persona": raw}),
            )
            .await;

        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "長さ {} のペルソナ: {body}",
            raw.chars().count()
        );
        assert_eq!(parse_json(&body), *expected, "長さ {}", raw.chars().count());
        assert!(
            !body.contains('あ') && !body.contains("100000"),
            "入力も上限値も反射しないこと: {body}"
        );
    }

    // **1件も作成されていない。** 弾いたつもりで書き込んでいたら、
    // **話しかけるまで壊れていると分からない**キャラクターが残る
    // （`PATCH` で直せるが、直そうと思う機会が来ない）。
    let (_, body) = server.get("/admin/characters").await;
    assert_eq!(
        parse_json(&body),
        json!({"characters": [], "unreadable": []})
    );

    // 妥当な長さのペルソナは通る（弾きすぎていないことの確認）。
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi", "base_persona": "あ".repeat(500)}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        parse_json(&body)["base_persona"],
        json!("あ".repeat(500)),
        "本文をそのまま保存すること"
    );

    server.stop().await;
}

/// **出力形式指示も検証される**（空 / 空白のみ / 長すぎる）。
///
/// 形式指示はペルソナと同じく**削れない枠**に載るうえ、プロンプト上で
/// 「指示として従え」と宣言される**唯一の節**（ADR-0007）。上限が無いと、
/// そこは面積無制限の指示チャネルになる。
/// **気付くのが「そのキャラクターと話せなくなってから」**なのもペルソナと同じ
/// （重さの権威は `domain::character::Persona` の doc の 0012a の注記）。
///
/// **空文字は「未設定」ではなくエラー。** 未設定はキーごと省く（または `null`）。
/// 2通りの「未設定」を作ると、「設定したのに節が出ない」が黙って成立する。
#[tokio::test]
async fn invalid_output_format_instructions_are_rejected_with_distinguishable_reasons() {
    let server = TestServer::start().await;

    let empty = invalid_request("出力形式指示が空です", "output_format_empty");
    let too_long = invalid_request("出力形式指示が長すぎます", "output_format_too_long");
    // 上限は `domain::OutputFormat::MAX_CHARS`。ここでは**確実に超える長さ**を送る。
    let oversized = "囲".repeat(10_000);

    for (raw, expected) in [
        ("", &empty),
        ("   ", &empty),
        (" \u{3000}\t\n", &empty),
        (oversized.as_str(), &too_long),
    ] {
        let (status, body) = server
            .post(
                "/admin/characters",
                &json!({
                    "handle": "aoi",
                    "base_persona": "ペルソナ本文",
                    "output_format_instruction": raw,
                }),
            )
            .await;

        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "長さ {} の形式指示: {body}",
            raw.chars().count()
        );
        assert_eq!(parse_json(&body), *expected, "長さ {}", raw.chars().count());
        assert!(
            !body.contains('囲') && !body.contains("10000"),
            "入力も上限値も反射しないこと: {body}"
        );
        assert!(
            !body.contains("ペルソナ"),
            "ペルソナのエラーと取り違えないこと: {body}"
        );
    }

    // **1件も作成されていない。** 弾いたつもりで書き込んでいたら、話しかけるまで壊れていると分からない行が残る。
    let (_, body) = server.get("/admin/characters").await;
    assert_eq!(
        parse_json(&body),
        json!({"characters": [], "unreadable": []})
    );

    // `null` は「未設定」として通る（空文字とは別物）。
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": "aoi",
                "base_persona": "ペルソナ本文",
                "output_format_instruction": Value::Null,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(parse_json(&body)["output_format_instruction"], Value::Null);

    server.stop().await;
}

/// 不正な軸名も**理由ごとに区別できる**（空白のみ / 制御文字 / 長すぎる）。
///
/// 識別子のエラーとも区別できること（同じ文言に潰さない）。
///
/// **軸名は削れない枠に件数分載る。** 1軸が長いほど載せられる軸が減るので、
/// 上限が無いと「軸を1つ足したら以後のチャットが全部予算超過」が起こりうる。
#[tokio::test]
async fn invalid_stat_axes_are_rejected_with_distinguishable_reasons() {
    let server = TestServer::start().await;
    server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi", "base_persona": "ペルソナ本文", "stat_scale": TARGET_SCALE}),
        )
        .await;
    let oversized = "情".repeat(10_000);

    for (raw, expected) in [
        (
            "\u{3000} ",
            invalid_request("性格ステータスの軸名が空です", "stat_axis_empty"),
        ),
        (
            "ロ\u{000B}ジック",
            invalid_request(
                "性格ステータスの軸名に改行や制御文字は使えません",
                "stat_axis_control",
            ),
        ),
        (
            oversized.as_str(),
            invalid_request("性格ステータスの軸名が長すぎます", "stat_axis_too_long"),
        ),
    ] {
        let (status, body) = server
            .put(
                &stats_path("aoi"),
                &json!({"stats": [{"axis": raw, "value": 3}]}),
            )
            .await;

        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "長さ {} の軸名: {body}",
            raw.chars().count()
        );
        assert_eq!(parse_json(&body), expected, "長さ {}", raw.chars().count());
        assert!(
            !body.contains("キャラクター識別子"),
            "識別子のエラーと取り違えないこと: {body}"
        );
        assert!(
            !body.contains('情') && !body.contains("10000"),
            "入力も上限値も反射しないこと: {body}"
        );
    }

    // 弾かれた設定は書き込まれていない。
    let (_, body) = server.get(&stats_path("aoi")).await;
    assert_eq!(
        parse_json(&body),
        json!({"character": "aoi", "stat_scale": TARGET_SCALE, "stats": []})
    );

    server.stop().await;
}

/// **ゴースト行は作成を塞がない**（0004b-2 で袋小路が消えた / ADR-0017）。
///
/// # このテストは意味が反転している。**それが要点**
///
/// 以前は `a_ghost_row_conflict_is_distinguishable_from_a_legitimate_duplicate` で、
/// ゴースト行との衝突が 409 `conflicting_unnormalized_name` として
/// 正当な重複と区別できることを固定していた（ADR-0004 / ADR-0008）。
///
/// **0004b-2 で正規化キーの索引ごと落としたので、その衝突は起きない。**
/// 「一覧に出ない・チャットは 404・しかし作ろうとすると既に存在する」という袋小路は、
/// 区別できるようにするのではなく**発生しなくなった**のでここで固定する。
///
/// ここで見るもの:
///
/// * 正当な重複は今までどおり 409 `character_already_exists`。
/// * **ゴースト行があっても同じ識別子で 201 が返る。**
/// * それでもゴースト行は**黙って消えない**（一覧が id で指し示す）。
///
/// # 409 の文言も「識別子」で書く（作業単位 0004b-3 の追補）
///
/// **409 が返る経路は `handle` の列 UNIQUE だけ**（`display_name` は一意ではない /
/// ADR-0011）。にもかかわらず「同じ**名前**の…」と返すと、受け取った側は
/// **呼び名が衝突したと読んで `display_name` を変えて再送し、また 409** になる
/// ——作成要求には両方が載るので、「名前」ではどちらの欄か読めない。
/// **`code` は変えない**（`character_already_exists`）。変えるのは文言だけ。
#[tokio::test]
async fn a_ghost_row_no_longer_blocks_creating_the_same_handle() {
    let server = TestServer::start().await;

    // --- 正当な重複 ---
    let (status, _) = server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi", "base_persona": "最初のペルソナ"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, duplicate_body) = server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi", "base_persona": "あとから来たペルソナ"}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        parse_json(&duplicate_body),
        invalid_request(
            "同じ識別子のキャラクターが既に存在します",
            "character_already_exists"
        )
    );
    assert!(
        !duplicate_body.contains("名前"),
        "「名前」と呼ばないこと（識別子と表示名のどちらが衝突したのか読めなくなる）: \
         {duplicate_body}"
    );

    // --- ゴースト行（**0004b-1 以降は CHECK を無効にしないと作れない** / 0005） ---
    insert_ghost_character_with(&server.store, " rin ", "凛", "ゴーストのペルソナ").await;

    let (status, ghost_body) = server
        .post(
            "/admin/characters",
            &json!({"handle": "rin", "base_persona": "本物のペルソナ"}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "ゴースト行が正当な識別子の作成を塞いでいる（正規化キーの索引が復活している）: \
         {ghost_body}"
    );
    assert_eq!(
        parse_json(&ghost_body)["handle"],
        json!("rin"),
        "作れたのは打った綴りの行であること: {ghost_body}"
    );

    // 作った行は `/v1/models` にも出る（**ゴースト行は出ない**）。
    let (_, body) = server.get("/v1/models").await;
    assert_eq!(
        names_in(&body, "data"),
        vec!["aoi", "rin"],
        "作れたキャラクターは選べること／ゴースト行は並ばないこと: {body}"
    );

    // 重複作成で既存のペルソナが書き換わっていないこと。
    let (_, body) = server.get("/admin/characters").await;
    let listed = parse_json(&body);
    assert_eq!(
        listed["characters"][1]["base_persona"],
        json!("本物のペルソナ"),
        "読める側に来たのは作った行であって、ゴースト行ではないこと: {body}"
    );
    assert_eq!(
        listed["characters"][0]["base_persona"],
        json!("最初のペルソナ"),
        "重複作成が既存行を上書きしないこと: {body}"
    );
    assert_eq!(
        names_in(&body, "characters"),
        vec!["aoi", "rin"],
        "ゴースト行は読める行としては出ないこと: {body}"
    );
    // **しかし黙って消えもしない**（ADR-0016 §3）。掃除の相手が分からなくなるため。
    // **0004b-1 で指し示す手掛かりは id だけになった**（ADR-0017。正規化しないので
    // `" rin "` からは `Handle` を作れない）。
    assert_eq!(
        listed["unreadable"][0]["handle"],
        Value::Null,
        "正規化しない以上、ゴースト行の識別子は返せないこと: {body}"
    );
    assert!(
        listed["unreadable"][0]["id"].is_i64(),
        "id は手掛かりとして必ず載ること: {body}"
    );
    assert_eq!(
        listed["unreadable"][0]["invalid_columns"],
        json!(["handle"]),
        "壊れているのが識別子の列だと分かること: {body}"
    );

    server.stop().await;
}

/// 存在しないキャラクターの性格ステータスは 404（空の一覧にしない）。
#[tokio::test]
async fn stats_of_an_unknown_character_are_not_found() {
    let server = TestServer::start().await;

    let expected = json!({
        "error": {
            "message": "指定されたキャラクターが存在しません",
            "type": "invalid_request_error",
            "code": "character_not_found",
        }
    });

    let (status, body) = server.get(&stats_path("aoi")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(parse_json(&body), expected, "GET も 404 であること");

    let (status, body) = server
        .put(
            &stats_path("aoi"),
            &json!({"stats": [{"axis": "パッション", "value": 3}]}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(parse_json(&body), expected, "PUT も 404 であること");

    server.stop().await;
}

/// 重複した軸は 400。**失敗した置き換えで前の設定が消えない**（同一トランザクション）。
#[tokio::test]
async fn duplicate_axes_are_rejected_without_losing_the_previous_stats() {
    let server = TestServer::start().await;
    server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi", "base_persona": "ペルソナ本文", "stat_scale": TARGET_SCALE}),
        )
        .await;
    server
        .put(
            &stats_path("aoi"),
            &json!({"stats": [{"axis": "パッション", "value": 3}]}),
        )
        .await;

    // 正規化すると同じ軸になる組み合わせ（前後の空白では分裂しない）。
    let (status, body) = server
        .put(
            &stats_path("aoi"),
            &json!({"stats": [
                {"axis": "共感", "value": 1},
                {"axis": " 共感 ", "value": 2},
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        parse_json(&body),
        invalid_request(
            "同じ軸を重複して指定することはできません",
            "duplicate_stat_axis"
        )
    );

    let (_, body) = server.get(&stats_path("aoi")).await;
    assert_eq!(
        parse_json(&body),
        json!({"character": "aoi", "stat_scale": TARGET_SCALE, "stats": [{"axis": "パッション", "value": 3}]}),
        "失敗した置き換えがロールバックされること: {body}"
    );

    server.stop().await;
}

/// **壊れた本文はリクエストの内容を反射しない。**
///
/// axum の既定の `JsonRejection` は serde のメッセージ（入力値を含む）をそのまま返す。
/// そのまま通すと、管理 API が入力反射の窓口になる。
#[tokio::test]
async fn a_malformed_body_does_not_reflect_the_request() {
    let server = TestServer::start().await;
    let expected = invalid_request("リクエスト本文の形式が不正です", "invalid_body");

    // 型違い。
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({"handle": 1234, "base_persona": "ペルソナ本文"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(parse_json(&body), expected);
    assert!(!body.contains("1234"), "入力値を反射しないこと: {body}");

    // 必須フィールドの欠落。
    let (status, body) = server
        .post("/admin/characters", &json!({"handle": "aoi"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(parse_json(&body), expected);

    // **未知のフィールドは弾く。** 投入スクリプトの綴り間違いが黙って通ると、
    // 「設定したはずの値が効かない」形で後から表面化する。
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": "aoi",
                "base_persona": "ペルソナ本文",
                "stats": [{"axis": "パッション", "value": 3}],
            }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(parse_json(&body), expected);
    assert!(
        !body.contains("stats"),
        "フィールド名を反射しないこと: {body}"
    );

    // JSON として壊れている。
    let (status, body) = server.post_raw("/admin/characters", "{\"name\":").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(parse_json(&body), expected);

    // 負値は `u32` に入らない。
    server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi", "base_persona": "ペルソナ本文"}),
        )
        .await;
    let (status, body) = server
        .put(
            &stats_path("aoi"),
            &json!({"stats": [{"axis": "パッション", "value": -1}]}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(parse_json(&body), expected);
    assert!(!body.contains("-1"), "入力値を反射しないこと: {body}");

    server.stop().await;
}

/// **パスを読めない要求も、リクエストの内容を反射しない。**
///
/// `%FF` は UTF-8 として読めないため `Path<String>` の抽出が拒否する
/// （`Handle::parse` まで到達しない）。axum の既定の `PathRejection` は
/// **失敗したパスの断片を本文に載せる**ので、そのまま返すと入力反射の窓口になる。
///
/// この経路はハンドラの中で唯一「拒否側」に落ちる分岐であり、
/// **ここを通るテストが1本も無いと `invalid_path()` を別の応答に書き換えても緑のまま**になる。
#[tokio::test]
async fn a_malformed_path_does_not_reflect_the_request() {
    let server = TestServer::start().await;
    let expected = invalid_request("パスの形式が不正です", "invalid_path");

    // GET / PUT のどちらもパスを先に読む（PUT は本文より前で落ちる）。
    let (status, get_body) = server.get("/admin/characters/%FF/stats").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{get_body}");
    assert_eq!(parse_json(&get_body), expected, "{get_body}");

    let (status, put_body) = server
        .put(
            "/admin/characters/%FF/stats",
            &json!({"stats": [{"axis": "パッション", "value": 3}]}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{put_body}");
    assert_eq!(parse_json(&put_body), expected, "{put_body}");

    // **更新・削除も同じ**（1件を指すパスなので、ここが唯一の入力）。
    let (status, patch_body) = server
        .patch("/admin/characters/%FF", &json!({"display_name": "葵"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{patch_body}");
    assert_eq!(parse_json(&patch_body), expected, "{patch_body}");

    let (status, delete_body) = server.delete("/admin/characters/%FF").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{delete_body}");
    assert_eq!(parse_json(&delete_body), expected, "{delete_body}");

    for body in [&get_body, &put_body, &patch_body, &delete_body] {
        for leak in ["FF", "utf", "UTF", "Invalid", "percent", "decode"] {
            assert!(
                !body.contains(leak),
                "拒否の詳細 {leak:?} を反射しないこと: {body}"
            );
        }
    }

    // **名前が不正な場合とは別の応答。** パスを読めないのと、読めた名前が
    // 受け付けられないのは原因が違う（前者はクライアントのエンコード誤り）。
    let (_, name_body) = server.get(&stats_path("aoi@new")).await;
    assert_ne!(
        parse_json(&get_body),
        parse_json(&name_body),
        "パスの形式の不正と名前の不正を同じ応答に潰さないこと"
    );

    server.stop().await;
}

/// **本文の上限超過は 413。**「JSON が壊れている」と読める 400 に吸収しない。
///
/// axum の既定の本文上限は 2MB。超過は `JsonRejection` の中で
/// `BytesRejection`（413）として現れるが、`invalid_body`（400）に潰すと
/// 「本文の形式が不正です」と返り、**原因（大きすぎる）から遠い**。
#[tokio::test]
async fn an_oversized_body_is_rejected_as_too_large() {
    let server = TestServer::start().await;

    // 3MB。中身は正当な JSON だが、上限で読む前に打ち切られる。
    let persona = "あ".repeat(1_000_000);
    let body = json!({"handle": "aoi", "base_persona": persona}).to_string();
    assert!(body.len() > 2 * 1024 * 1024, "前提: 既定の上限を超えること");

    let (status, response) = server.post_bytes("/admin/characters", body).await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{response}");
    assert_eq!(
        parse_json(&response),
        invalid_request("リクエスト本文が大きすぎます", "body_too_large"),
        "上限超過を JSON の形式エラーに潰さないこと"
    );
    assert!(
        !response.contains("あ"),
        "入力を反射しないこと: {}",
        &response[..response.len().min(200)]
    );

    // 上限内の壊れた本文は従来どおり 400（両者を同じ応答に潰さない）。
    let (status, malformed) = server.post_raw("/admin/characters", "{\"name\":").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{malformed}");
    assert_ne!(parse_json(&response), parse_json(&malformed));

    server.stop().await;
}

/// 管理 API の 500 も**定数文字列だけ**を載せる（`{err:?}` を含めない）。
#[tokio::test]
async fn admin_failures_do_not_leak_internal_details() {
    let server = TestServer::start().await;

    // 稼働中にプールを閉じ、DB 操作を確実に失敗させる。
    server.store.close().await;

    for (status, body) in [
        server
            .post(
                "/admin/characters",
                &json!({"handle": "aoi", "base_persona": "ペルソナ本文"}),
            )
            .await,
        server.get("/admin/characters").await,
        server.get(&stats_path("aoi")).await,
        server
            .put(
                &stats_path("aoi"),
                &json!({"stats": [{"axis": "パッション", "value": 3}]}),
            )
            .await,
        // **更新・削除も同じ。** `store` の失敗は `UnknownCharacter` にも
        // `CharacterHasSessions` にも化けず、詳細を載せない 500 になる。
        server
            .patch(&character_path("aoi"), &json!({"display_name": "葵"}))
            .await,
        server.delete(&character_path("aoi")).await,
    ] {
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
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
    }

    server.stop().await;
}

/// **`memory` を消す・書き換えるエンドポイントは存在しない**（ADR-0008 / ADR-0022）。
///
/// # 以前はこのパス自体が全メソッド 404 だった（**見張る内容を変えてある**）
///
/// 14b より前、このテストは「`memory` を触る入口は1つも無い」を固定していた
/// ——不変条件2（`superseded_by` は時系列保持）の防御が
/// 「API を生やさないこと」しか無かったため。
///
/// **ADR-0022 がその前提を覆した。** 記憶が1行壊れるとそのキャラクターの
/// チャットも抽出も止まり、**アプリ経由で直す手段が無い**（実際に起きた / 13c-3b）。
/// 決定は「消す道は作らない。**いま何が正しいか**を書かせ、古い行にそれを指させる」で、
/// 生えたのは**読む入口1つ**と**行を追加する入口1つ**だけである。
///
/// **したがって見張る内容は「生やしていない」から「生やしたのはこの2つだけ」へ変わる**
/// （`only_patch_and_delete_are_exposed_for_a_single_character` が 0012c で通った道と同じ）。
/// **このテストが落ちたら ADR-0022 に戻ること。**
///
/// # 何を塞ぎ続けるのか
///
/// * **1件の記憶を指すパスが存在しない。** `.../memories/{id}` は 404 のまま
///   ——生やせば `PUT`（置き換え）と `DELETE`（削除）が自然な形になり、
///   **ADR-0022 が却下した2つの操作**がパスとして表現できてしまう。
/// * **一覧のパスは `GET` だけ。** `DELETE .../memories` は
///   「そのキャラクターの記憶を全部消す」の最も自然な綴り。
/// * **訂正のパスは `POST` だけ。** `DELETE` は「訂正を取り消す」
///   = 後継の印を外す文（`superseded_by` を NULL に戻す）の綴りになる。
/// * `/admin/memories` / `/admin/memory` は**どのメソッドでも 404**
///   ——キャラクターに属さない記憶の入口は無い。
#[tokio::test]
async fn no_admin_endpoint_deletes_or_rewrites_a_memory() {
    let server = TestServer::start().await;
    server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi", "base_persona": "ペルソナ本文"}),
        )
        .await;

    let not_found = json!({
        "error": {
            "message": "指定されたエンドポイントは存在しません",
            "type": "invalid_request_error",
        }
    });
    let method_not_allowed = json!({
        "error": {
            "message": "このエンドポイントは指定されたメソッドを受け付けません",
            "type": "invalid_request_error",
        }
    });

    // **キャラクターに属さない記憶の入口は無い**（全メソッド 404）。
    // **1件の記憶を指すパスも無い**——`PUT` / `DELETE` が自然に生える形だから。
    for path in [
        "/admin/memories",
        "/admin/memory",
        "/admin/characters/aoi/memories/1",
        "/admin/characters/aoi/memory/1",
    ] {
        for method in [Method::GET, Method::POST, Method::PUT, Method::DELETE] {
            let (status, body) = server.send(method.clone(), path, Some(&json!({}))).await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "{method} {path} が生えていないこと: {body}"
            );
            assert_eq!(parse_json(&body), not_found, "{method} {path}");
        }
    }

    // **生えているパスでも、受けるメソッドは1つずつ**（405）。
    for (method, path) in [
        (Method::POST, "/admin/characters/aoi/memories"),
        (Method::PUT, "/admin/characters/aoi/memories"),
        (Method::PATCH, "/admin/characters/aoi/memories"),
        (Method::DELETE, "/admin/characters/aoi/memories"),
        (Method::GET, "/admin/characters/aoi/memories/1/corrections"),
        (Method::PUT, "/admin/characters/aoi/memories/1/corrections"),
        (
            Method::PATCH,
            "/admin/characters/aoi/memories/1/corrections",
        ),
        (
            Method::DELETE,
            "/admin/characters/aoi/memories/1/corrections",
        ),
    ] {
        let (status, body) = server.send(method.clone(), path, Some(&json!({}))).await;
        assert_eq!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} {path} が生えていないこと: {body}"
        );
        assert_eq!(parse_json(&body), method_not_allowed, "{method} {path}");
    }

    server.stop().await;
}

/// 1件を指すパスが受けるのは **`PATCH` と `DELETE` だけ**（ADR-0016 / 作業単位 0012c）。
///
/// # 以前はこのパス自体が 404 だった
///
/// 0012c より前、`/admin/characters/{name}` は「削除・更新を生やしていないこと」の
/// 見張り番として **全メソッド 404** を固定していた。ADR-0016 でその2つを足したので、
/// **見張る内容を「生やしていない」から「生やしたのはこの2つだけ」へ変えてある。**
///
/// `PUT` / `POST` を塞ぎ続けるのは意味がある。**`PUT` は「丸ごと置き換え」**であり、
/// 識別子を含む全フィールドを受ける形（= 識別子の変更）に最も自然に書かれる。
/// 識別子は不変（ADR-0016 §1）なので、その形はここに生えてはいけない。
#[tokio::test]
async fn only_patch_and_delete_are_exposed_for_a_single_character() {
    let server = TestServer::start().await;
    server
        .post(
            "/admin/characters",
            &json!({"handle": "aoi", "base_persona": "ペルソナ本文"}),
        )
        .await;

    let method_not_allowed = json!({
        "error": {
            "message": "このエンドポイントは指定されたメソッドを受け付けません",
            "type": "invalid_request_error",
        }
    });
    let not_found = json!({
        "error": {
            "message": "指定されたエンドポイントは存在しません",
            "type": "invalid_request_error",
        }
    });

    // 既存のパスにメソッドだけ足す形（405）。
    for (method, path) in [
        (Method::DELETE, "/admin/characters"),
        (Method::PUT, "/admin/characters"),
        (Method::PATCH, "/admin/characters"),
        (Method::DELETE, "/admin/characters/aoi/stats"),
        (Method::POST, "/admin/characters/aoi/stats"),
        // **1件を指すパスでも、置き換え系は生えていない。**
        (Method::PUT, "/admin/characters/aoi"),
        (Method::POST, "/admin/characters/aoi"),
        (Method::GET, "/admin/characters/aoi"),
    ] {
        let (status, body) = server.send(method.clone(), path, Some(&json!({}))).await;
        assert_eq!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} {path}: {body}"
        );
        assert_eq!(parse_json(&body), method_not_allowed, "{method} {path}");
    }

    // **複数形のパスに `{name}` を足した形は今も存在しない**（404）。
    for (method, path) in [
        (Method::PATCH, "/admin/characters/aoi/persona"),
        (Method::DELETE, "/admin/characters/aoi/name"),
    ] {
        let (status, body) = server.send(method.clone(), path, Some(&json!({}))).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {path} が生えていないこと: {body}"
        );
        assert_eq!(parse_json(&body), not_found, "{method} {path}");
    }

    // 405 で拒まれた要求はキャラクターを書き換えても消してもいない。
    let (_, body) = server.get("/admin/characters").await;
    assert_eq!(names_in(&body, "characters"), vec!["aoi"]);
    assert_eq!(
        character_in(&body, "aoi")["base_persona"],
        json!("ペルソナ本文"),
        "{body}"
    );

    server.stop().await;
}

/// **「一覧 → 修復」が HTTP で1本の線として繋がる**〈この作業単位の最重要〉
/// （ADR-0016 §3・§4 / 作業単位 0012c）。
///
/// 0012b は同じ線を `store` 層で張った。ここは **HTTP だけで完結すること**を見る。
///
/// # テストが名前を知らない
///
/// 修復に使う識別子は**`GET /admin/characters` の `unreadable` が返した値だけ**から取る。
/// 定数から名前を知ると、「一覧が識別子を返さなくなった」状態でも緑を通り、
/// **一覧が修復の入口であるという性質**が見張られなくなる。
///
/// # 1列だけ直した段階も見る
///
/// 壊れた列が2つある行を1つだけ直すと、その行は**まだ `find_character_by_handle` で
/// 読めない**（`InvalidStoredValue`）。ここを 500 にすると、
/// **修復の途中が「サーバ障害」に見えて手が止まる。**
#[tokio::test]
async fn a_broken_row_is_repaired_over_http_using_only_what_the_listing_returned() {
    let server = TestServer::start().await;
    assert_fixture_values_are_distinct();

    // 囮（正常な行）。**先に作って壊れた行の id を 1 から外す。**
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": DECOY_HANDLE,
                "display_name": DECOY_DISPLAY,
                "base_persona": DECOY_PERSONA,
                "output_format_instruction": DECOY_FORMAT,
                "stat_scale": DECOY_SCALE,
                "stat_drift_mode": DECOY_DRIFT_MODE,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let decoy_id = parse_json(&body)["id"]
        .as_i64()
        .expect("id は整数であること");

    // **2列が壊れた行。** U+2028 の表示名は 0009 以前なら `/admin/` から作れた。
    // 空のペルソナはスキーマの CHECK が無いので直接 INSERT なら通る。
    sqlx::query("INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)")
        .bind(TARGET_HANDLE)
        .bind(format!("{TARGET_DISPLAY}\u{2028}花"))
        .bind("")
        .execute(server.store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");

    // --- 一覧が指し示す ---
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed = parse_json(&body);
    let broken = listed["unreadable"]
        .as_array()
        .and_then(|rows| rows.first())
        .unwrap_or_else(|| panic!("壊れた行が指し示されること: {body}"))
        .clone();
    assert_eq!(
        broken["invalid_columns"],
        json!(["display_name", "base_persona"]),
        "壊れている列を全部並べること: {body}"
    );
    let broken_id = broken["id"].as_i64().expect("id は整数であること");
    assert_ne!(broken_id, decoy_id, "囮とは別の行であること");
    // **ここから先、テストは定数から名前を知らない。**
    let handle = broken["handle"]
        .as_str()
        .unwrap_or_else(|| panic!("修復に使える識別子が返ること: {body}"))
        .to_owned();

    // --- 1列だけ直す。まだ読めないが、失敗ではない ---
    let (status, body) = server
        .patch(
            &character_path(&handle),
            &json!({"display_name": NEW_DISPLAY}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body),
        json!({"id": broken_id, "character": null}),
        "壊れた列が残る行は読み直せないが、更新そのものは成功していること: {body}"
    );

    let (_, body) = server.get("/admin/characters").await;
    assert_eq!(
        parse_json(&body)["unreadable"],
        json!([{"id": broken_id, "handle": handle, "invalid_columns": ["base_persona"]}]),
        "直した列が `unreadable` から消え、残りだけが指し示されること: {body}"
    );

    // --- 残りを直す。ここで読めるようになる ---
    let (status, body) = server
        .patch(
            &character_path(&handle),
            &json!({"base_persona": NEW_PERSONA}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let updated = parse_json(&body);
    assert_eq!(updated["id"], json!(broken_id), "{body}");
    let repaired = updated["character"].clone();
    assert_eq!(repaired["id"], json!(broken_id), "同じ行を返すこと: {body}");
    assert_eq!(repaired["handle"], json!(handle), "{body}");
    assert_eq!(repaired["display_name"], json!(NEW_DISPLAY), "{body}");
    assert_eq!(repaired["base_persona"], json!(NEW_PERSONA), "{body}");
    assert_eq!(
        repaired["output_format_instruction"],
        Value::Null,
        "触れていない列が湧いて出ないこと: {body}"
    );
    let created_at = parse_time(&repaired["created_at"]);
    assert!(created_at <= Utc::now() + CLOCK_SKEW, "{created_at}");

    // --- 一覧に壊れた行が無くなる ---
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["unreadable"],
        json!([]),
        "修復後は壊れた行が無いこと: {body}"
    );
    assert_eq!(
        names_in(&body, "characters"),
        vec![DECOY_HANDLE, TARGET_HANDLE],
        "修復した行が読める行として並ぶこと: {body}"
    );

    // **`/v1/models` にも出る**（修復のゴールは「また話せる」こと）。
    let (status, body) = server.get("/v1/models").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(names_in(&body, "data"), vec![DECOY_HANDLE, TARGET_HANDLE]);

    assert_decoy_is_untouched(&server).await;
    assert_eq!(server.character_count().await, 2, "行が増えていないこと");

    server.stop().await;
}

/// **ゴースト行は HTTP からも触れないが、袋小路にはならない**（0004b-2 / ADR-0017）。
///
/// # このテストは意味が変わっている。**その履歴が要点**
///
/// もとは `a_ghost_row_is_removed_over_http_using_only_what_the_listing_returned`
/// で、「一覧 → 掃除」が HTTP で1本に繋がることを固定していた（0012c）。
///
/// 1. **0004b-1** で一覧が `name: null` を返すようになり、
///    「一覧が返したものだけで届く」は成り立たなくなった（綴りはテストが打っていた）。
/// 2. **0004b-2** で `DELETE_CHARACTER` の `trim` を外したので、打っても届かない。
///
/// **失った代わりに得たもの**——ゴースト行が**正当な識別子を塞がなくなった。**
/// 以前は「掃除しないと同じ名前を作れない」状態で、
/// その掃除の綴りを API からは知り得なかった（＝本当の袋小路）。
/// いまは掃除せずにそのまま作り直せる。
///
/// # ここで固定するもの
///
/// * ゴースト行があっても **`POST` は 201**。
/// * ゴースト行は一覧に**残り続ける**（黙って消さない）。
/// * `DELETE` は **404**（成功したと嘘をつかない）。**行数も変わらない。**
/// * 囮のキャラクターが最後まで無傷。
#[tokio::test]
async fn a_ghost_row_cannot_be_reached_over_http_but_no_longer_blocks_the_handle() {
    let server = TestServer::start().await;
    assert_fixture_values_are_distinct();

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": DECOY_HANDLE,
                "display_name": DECOY_DISPLAY,
                "base_persona": DECOY_PERSONA,
                "output_format_instruction": DECOY_FORMAT,
                "stat_scale": DECOY_SCALE,
                "stat_drift_mode": DECOY_DRIFT_MODE,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // ゴースト行。**完全一致でも `model` でも引けない**（ADR-0004）。
    // **0004b-1 以降は CHECK を無効にしないと作れない**（0005 / ADR-0017）。
    insert_ghost_character_with(
        &server.store,
        &format!(" {TARGET_HANDLE} "),
        TARGET_DISPLAY,
        TARGET_PERSONA,
    )
    .await;

    // --- 一覧が指し示す（起点はここにしかない） ---
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ghost = parse_json(&body)["unreadable"]
        .as_array()
        .and_then(|rows| rows.first())
        .unwrap_or_else(|| panic!("ゴースト行が指し示されること: {body}"))
        .clone();
    assert_eq!(
        ghost["invalid_columns"],
        json!(["handle"]),
        "壊れているのが識別子だと分かること: {body}"
    );
    // **0004b-1 で一覧は識別子を返さなくなった**（ADR-0017。正規化しないので
    // `" aoi "` は `Handle` を作れない）。
    assert_eq!(
        ghost["handle"],
        Value::Null,
        "正規化しない以上、ゴースト行の識別子は返せないこと: {body}"
    );
    let ghost_id = ghost["id"].as_i64().expect("id は整数であること");
    let handle = TARGET_HANDLE.to_owned();

    // --- 正規化した綴りを打っても届かない（0004b-2） ---
    let (status, body) = server.delete(&character_path(&handle)).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "引き当てが完全一致より広い（`trim` が復活している）: {body}"
    );
    assert_eq!(
        parse_json(&body),
        invalid_request(
            "指定されたキャラクターが存在しません",
            "character_not_found"
        )
    );
    assert_eq!(
        server.character_count().await,
        2,
        "拒否したのに行が消えていないこと"
    );

    // --- **それでも袋小路ではない。** 掃除せずに同じ識別子を作れる ---
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({"handle": TARGET_HANDLE, "base_persona": NEW_PERSONA}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "ゴースト行が正当な識別子を塞いでいる（正規化キーの索引が復活している）: {body}"
    );

    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        names_in(&body, "characters"),
        vec![DECOY_HANDLE, TARGET_HANDLE],
        "作った行が読める側に来ること: {body}"
    );
    assert_eq!(
        parse_json(&body)["unreadable"],
        json!([{"id": ghost_id, "handle": null, "invalid_columns": ["handle"]}]),
        "ゴースト行は指し示されたまま残ること（黙って消さない）: {body}"
    );

    // 作った方は普通に消せる（**ゴースト行は巻き添えにならない**）。
    let (status, body) = server.delete(&character_path(&handle)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(body.is_empty(), "204 に本文を付けないこと: {body}");
    assert_eq!(
        server.character_count().await,
        2,
        "消えたのは1行だけ（ゴースト行が残っていること）"
    );

    assert_decoy_is_untouched(&server).await;
    server.stop().await;
}

/// **ゴースト行への `PATCH` は 404 で、1バイトも書かない**（0004b-2 / ADR-0017）。
///
/// # このテストは意味が反転している。**その履歴が要点**
///
/// もとは `patching_a_ghost_row_reports_success_and_actually_writes_the_row` で、
/// 「ゴースト行を直せるが読み直せないので 200 + `character: null`」を固定していた
/// （0012c / ADR-0016 §4）。**0004b-2 で引き当てを完全一致にしたので、
/// ゴースト行にはそもそも届かない。**
///
/// `UpdatedCharacterView` の「正しい `null`」のうち**もう片方**
/// （壊れた列の片方だけを直した）は
/// `a_broken_row_is_repaired_over_http_using_only_what_the_listing_returned` が押さえており、
/// そちらは今も生きている。
///
/// # なぜ 404 でよいのか
///
/// 以前 404 を避けていたのは「実際には行が書き換わっているのに、運用者が
/// 『何も起きていない』と読む」ことを嫌ったから。**いまは本当に何も起きていない**ので、
/// 404 は正しい報告になる。**書いていないことまで確かめる**のがこのテストの本体で、
/// 「404 を返しつつ書く」実装をここで落とす。
#[tokio::test]
async fn patching_a_ghost_row_is_not_found_and_writes_nothing() {
    let server = TestServer::start().await;
    let decoy_id = create_decoy(&server).await;

    // ゴースト行。**壊れているのは識別子だけ**で、他の列は読める。
    // **0004b-1 以降は CHECK を無効にしないと作れない**（0005 / ADR-0017）。
    let stored_name = format!(" {TARGET_HANDLE} ");
    insert_ghost_character_with(&server.store, &stored_name, TARGET_DISPLAY, TARGET_PERSONA).await;

    // --- 一覧が指し示す ---
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ghost = parse_json(&body)["unreadable"]
        .as_array()
        .and_then(|rows| rows.first())
        .unwrap_or_else(|| panic!("ゴースト行が指し示されること: {body}"))
        .clone();
    assert_eq!(
        ghost["invalid_columns"],
        json!(["handle"]),
        "壊れているのが識別子だと分かること: {body}"
    );
    let ghost_id = ghost["id"].as_i64().expect("id は整数であること");
    assert_ne!(ghost_id, decoy_id, "囮とは別の行であること");
    // **0004b-1 で一覧は識別子を返さなくなった**（ADR-0017）。
    assert_eq!(
        ghost["handle"],
        Value::Null,
        "正規化しない以上、ゴースト行の識別子は返せないこと: {body}"
    );
    let handle = TARGET_HANDLE.to_owned();

    // --- 正規化した綴りを打っても届かない ---
    let (status, body) = server
        .patch(
            &character_path(&handle),
            &json!({
                "display_name": NEW_DISPLAY,
                "base_persona": NEW_PERSONA,
                "output_format_instruction": NEW_FORMAT,
            }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "引き当てが完全一致より広い（`trim` が復活している）: {body}"
    );
    assert_eq!(
        parse_json(&body),
        invalid_request(
            "指定されたキャラクターが存在しません",
            "character_not_found"
        )
    );

    // --- **404 が「書いていない」ことまで含むこと。** 生の行を直接読む ---
    let (name, display_name, persona, format) = raw_character_row(&server, ghost_id).await;
    assert_eq!(
        (
            name.as_str(),
            display_name.as_str(),
            persona.as_str(),
            format.as_deref()
        ),
        (stored_name.as_str(), TARGET_DISPLAY, TARGET_PERSONA, None),
        "拒否したなら1列も書き換えていないこと"
    );

    // --- 一覧の見え方も変わらない ---
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["unreadable"],
        json!([{"id": ghost_id, "handle": null, "invalid_columns": ["handle"]}]),
        "ゴースト行は指し示されたまま残ること: {body}"
    );
    assert_eq!(
        names_in(&body, "characters"),
        vec![DECOY_HANDLE],
        "読める行として出てこないこと: {body}"
    );

    assert_decoy_is_untouched(&server).await;
    assert_eq!(server.character_count().await, 2, "行が増えていないこと");

    server.stop().await;
}

/// **本物の DB 障害は「修復の途中」に化けない**（ADR-0016 §4 / 作業単位 0012c）。
///
/// `created_at` にデコードできない値が入った行は、`parse` 由来の壊れ方とは違って
/// **今でも一覧を丸ごと落とす**（`Store::list_characters` の doc）。この形の行に
/// `PATCH` すると、更新後の読み直しが `StoreError::Query` になる。
///
/// ここを `Ok(None)` に混ぜると **200 `{"id": N, "character": null}`**、つまり
/// **「更新は成功した。読み直せないだけなので修復を続けてよい」**という応答になる。
/// `character: null` が正しいのは `UpdatedCharacterView` の doc が挙げる2つの場合だけで、
/// **読み出しそのものが失敗した場合は含まれない。**
///
/// `admin_failures_do_not_leak_internal_details` はプールを閉じるので
/// `update_character` の側で先に落ち、**この枝には届かない**（0012c で実測）。
#[tokio::test]
async fn a_row_whose_created_at_cannot_be_decoded_fails_the_update_loudly() {
    let server = TestServer::start().await;
    let decoy_id = create_decoy(&server).await;

    // **`created_at` が日時として読めない行。** 列に CHECK は無いので直接 INSERT なら通る
    // （DB の直接編集か、外部で作った DB を持ち込んだ場合に現れる形）。
    let broken_id = sqlx::query(
        "INSERT INTO character (handle, display_name, base_persona, created_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(TARGET_HANDLE)
    .bind(TARGET_DISPLAY)
    .bind(TARGET_PERSONA)
    .bind("壊れた日時")
    .execute(server.store.pool())
    .await
    .expect("直接の INSERT はスキーマ上通ること")
    .last_insert_rowid();
    assert_ne!(broken_id, decoy_id, "囮とは別の行であること");

    // 前提: この壊れ方は `unreadable` では表現できない（`UnreadableCharacter` は
    // 日時を持たない）ので、**一覧ごと 500 になる。** つまり
    // `invalid_columns` からは辿れず、`PATCH` の応答だけが手掛かりになる。
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "前提: デコードできない列は一覧を丸ごと落とすこと: {body}"
    );

    // --- 更新は 500。**200 の `character: null` に化けないこと** ---
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"display_name": NEW_DISPLAY}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "本物の障害を「修復の途中なので先へ進んでよい」に見せないこと: {body}"
    );
    assert_eq!(
        parse_json(&body),
        json!({"error": {"message": "内部エラーが発生しました", "type": "server_error"}}),
        "本文は定数のみであること: {body}"
    );
    assert_ne!(
        parse_json(&body),
        json!({"id": broken_id, "character": null}),
        "読み出しの失敗を「読み直せないだけ」に混ぜないこと"
    );
    for leak in [
        server.db_path.as_str(),
        "sqlite",
        "sqlx",
        "ColumnDecode",
        "created_at",
        "src\\",
        "src/",
    ] {
        assert!(
            !body.contains(leak),
            "内部情報 {leak:?} が漏れていないこと: {body}"
        );
    }

    // **500 でも書き込みそのものは済んでいる**（更新と読み直しは別の文で、
    // 前者は commit 済み）。だからこそ「先へ進んでよい」と読める応答にできない
    // ——運用者が知るべきなのは「書けたが、この DB は壊れている」であって、
    // 「修復の途中」ではない。**この段取りを変えるなら見張り番の意味も変わる。**
    let (_, display_name, _, _) = raw_character_row(&server, broken_id).await;
    assert_eq!(
        display_name, NEW_DISPLAY,
        "更新自体は行われていること（500 の意味が「読めなかった」であること）"
    );

    // 囮は巻き添えになっていない（一覧が 500 なので生の行で見る）。
    let (name, display_name, persona, format) = raw_character_row(&server, decoy_id).await;
    assert_eq!(
        (
            name.as_str(),
            display_name.as_str(),
            persona.as_str(),
            format.as_deref()
        ),
        (
            DECOY_HANDLE,
            DECOY_DISPLAY,
            DECOY_PERSONA,
            Some(DECOY_FORMAT)
        ),
        "囮が巻き添えになっていないこと"
    );
    assert_eq!(server.character_count().await, 2, "行が増えていないこと");

    server.stop().await;
}

/// **しまっていなければ、会話が1件でもあれば消せない**（ADR-0016 §2 / 作業単位 0012c）。
/// しまった後に消せることは `tests/server_character_archive.rs` が見る（ADR-0049 / 作業単位 A-10）。
///
/// 「該当なし」と「会話があるので消せない」を**応答で区別できる**こと、
/// そして拒否のときに **`character` も `session` も1行も減らない**ことを見る
/// （エラーが返っても消えていたら、この制約は守れていない）。
///
/// 会話の無い囮を同じ DB に置いてあるので、**一律に拒否する実装**とも区別できる。
#[tokio::test]
async fn a_character_with_a_conversation_cannot_be_deleted() {
    let server = TestServer::start().await;
    let (_decoy_id, target_id) = create_decoy_and_target(&server).await;

    sqlx::query("INSERT INTO session (character_id) VALUES (?)")
        .bind(target_id)
        .execute(server.store.pool())
        .await
        .expect("会話を作れること");

    let (status, refused) = server.delete(&character_path(TARGET_HANDLE)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(
        parse_json(&refused),
        json!({
            "error": {
                "message": "会話のあるキャラクターは、先にしまわないと削除できません",
                "type": "invalid_request_error",
                "code": "character_has_sessions",
            }
        }),
        "仕様上の拒否だと分かる応答であること"
    );

    // **行数が変化しないこと。**
    assert_eq!(server.character_count().await, 2, "1行も消えていないこと");
    assert_eq!(server.session_count().await, 1, "会話も消えていないこと");
    let (_, body) = server.get("/admin/characters").await;
    assert_eq!(
        names_in(&body, "characters"),
        vec![DECOY_HANDLE, TARGET_HANDLE],
        "{body}"
    );

    // **「該当なし」とは別の応答。** 打ち間違いと仕様上の拒否は対処が正反対。
    let (status, missing) = server.delete(&character_path("nobody")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    assert_eq!(
        parse_json(&missing),
        invalid_request(
            "指定されたキャラクターが存在しません",
            "character_not_found"
        )
    );
    assert_ne!(
        parse_json(&refused),
        parse_json(&missing),
        "2つの理由を同じ応答に潰さないこと"
    );

    // **会話の無いキャラクターは消せる**（一律の拒否ではないこと）。
    let (status, body) = server.delete(&character_path(DECOY_HANDLE)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(server.character_count().await, 1);
    assert_eq!(server.session_count().await, 1, "会話は残っていること");

    // 更新は会話があっても通る（禁じているのは削除だけ）。
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"display_name": NEW_DISPLAY}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    server.stop().await;
}

/// **部分更新。指定しなかった列は変わらない**（作業単位 0012c）。
///
/// 5つとも別の値にしてあるので、「省略された列を既定値で潰す」実装も
/// 「別の列に書く」実装もここで赤くなる（**5つ目の走らせ方は A-8 で足した**。
/// `handle` の番号は `?6` → `?8` へずれた）。
///
/// # **`UPDATE_CHARACTER` のバインド順を見張っているのはここ**（作業単位 0011a-1）
///
/// `sqlx` の `.bind()` は呼んだ順に `?1` から割り当てるので、SQL の `?N` を
/// 並べ替えても bind の並びを直さなければ**黙って別の列へ書く。**
/// 尺度（`stat_scale`）を足したことで `handle` の番号が `?5` → `?6` にずれており、
/// そこを取り違えると WHERE が尺度と照合して**1行も更新されない**（404 になる）。
///
/// **識別子・id・作成日時も動かない**（ADR-0016 §1）。
/// 応答は**保存後の姿を読み直したもの**であることも見る（要求の反射ではない）。
#[tokio::test]
async fn a_partial_update_leaves_the_other_columns_alone() {
    let server = TestServer::start().await;
    let (_decoy_id, target_id) = create_decoy_and_target(&server).await;

    let (_, body) = server.get("/admin/characters").await;
    let created_at = character_in(&body, TARGET_HANDLE)["created_at"].clone();

    // --- 表示名だけ。**前後の空白は正規化されて返る**（要求の反射ではない） ---
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"display_name": format!(" {NEW_DISPLAY} ")}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let updated = parse_json(&body);
    assert_eq!(updated["id"], json!(target_id), "更新した行の id: {body}");
    assert_eq!(
        updated["character"],
        json!({
            "id": target_id,
            "handle": TARGET_HANDLE,
            "display_name": NEW_DISPLAY,
            "base_persona": TARGET_PERSONA,
            "output_format_instruction": TARGET_FORMAT,
            "stat_scale": TARGET_SCALE,
            "stat_drift_mode": TARGET_DRIFT_MODE,
            "axis_discovery_slots": 0,
            "created_at": created_at,
            // **しまう欄を省いた更新は、しまった状態に触らない**（作業単位 A-10）。
            "archived_at": null,
        }),
        "指定した列だけが変わり、識別子・id・作成日時は動かないこと: {body}"
    );

    // --- ペルソナだけ ---
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"base_persona": NEW_PERSONA}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["character"],
        json!({
            "id": target_id,
            "handle": TARGET_HANDLE,
            "display_name": NEW_DISPLAY,
            "base_persona": NEW_PERSONA,
            "output_format_instruction": TARGET_FORMAT,
            "stat_scale": TARGET_SCALE,
            "stat_drift_mode": TARGET_DRIFT_MODE,
            "axis_discovery_slots": 0,
            "created_at": created_at,
            // **しまう欄を省いた更新は、しまった状態に触らない**（作業単位 A-10）。
            "archived_at": null,
        }),
        "{body}"
    );

    // --- 形式指示だけ ---
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"output_format_instruction": NEW_FORMAT}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["character"],
        json!({
            "id": target_id,
            "handle": TARGET_HANDLE,
            "display_name": NEW_DISPLAY,
            "base_persona": NEW_PERSONA,
            "output_format_instruction": NEW_FORMAT,
            "stat_scale": TARGET_SCALE,
            "stat_drift_mode": TARGET_DRIFT_MODE,
            "axis_discovery_slots": 0,
            "created_at": created_at,
            // **しまう欄を省いた更新は、しまった状態に触らない**（作業単位 A-10）。
            "archived_at": null,
        }),
        "{body}"
    );

    // --- 尺度だけ（作業単位 0011a-1） ---
    //
    // ここまでの3件は**尺度をキーごと省いている**ので、上の期待値が
    // 「尺度を省いた更新で尺度が変わらない」ことを既に固定している
    // （省略を `StatScale::DEFAULT` に潰す実装は 7 ≠ 10 で赤くなる）。
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"stat_scale": NEW_SCALE}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["character"],
        json!({
            "id": target_id,
            "handle": TARGET_HANDLE,
            "display_name": NEW_DISPLAY,
            "base_persona": NEW_PERSONA,
            "output_format_instruction": NEW_FORMAT,
            "stat_scale": NEW_SCALE,
            "stat_drift_mode": TARGET_DRIFT_MODE,
            "axis_discovery_slots": 0,
            "created_at": created_at,
            // **しまう欄を省いた更新は、しまった状態に触らない**（作業単位 A-10）。
            "archived_at": null,
        }),
        "尺度だけを変えても他の列が動かないこと: {body}"
    );

    // --- 走らせ方だけ（作業単位 A-8） ---
    //
    // ここまでの4件は**走らせ方をキーごと省いている**ので、上の期待値が
    // 「走らせ方を省いた更新で走らせ方が変わらない」ことを既に固定している。
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"stat_drift_mode": NEW_DRIFT_MODE}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["character"],
        json!({
            "id": target_id,
            "handle": TARGET_HANDLE,
            "display_name": NEW_DISPLAY,
            "base_persona": NEW_PERSONA,
            "output_format_instruction": NEW_FORMAT,
            "stat_scale": NEW_SCALE,
            "stat_drift_mode": NEW_DRIFT_MODE,
            "axis_discovery_slots": 0,
            "created_at": created_at,
            // **しまう欄を省いた更新は、しまった状態に触らない**（作業単位 A-10）。
            "archived_at": null,
        }),
        "走らせ方だけを変えても他の列が動かないこと: {body}"
    );

    // --- 空の要求は何も変えない（成功する） ---
    let (status, body) = server
        .patch(&character_path(TARGET_HANDLE), &json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["character"]["display_name"],
        json!(NEW_DISPLAY),
        "1列も指定しない更新が既定値で潰さないこと: {body}"
    );
    assert_eq!(
        parse_json(&body)["character"]["stat_scale"],
        json!(NEW_SCALE),
        "1列も指定しない更新が尺度を既定へ戻さないこと: {body}"
    );

    // 一覧から見ても同じ（応答だけが正しい形になっていないこと）。
    let (_, body) = server.get("/admin/characters").await;
    assert_eq!(
        character_in(&body, TARGET_HANDLE),
        json!({
            "id": target_id,
            "handle": TARGET_HANDLE,
            "display_name": NEW_DISPLAY,
            "base_persona": NEW_PERSONA,
            "output_format_instruction": NEW_FORMAT,
            "stat_scale": NEW_SCALE,
            "stat_drift_mode": NEW_DRIFT_MODE,
            "axis_discovery_slots": 0,
            "created_at": created_at,
            // **しまう欄を省いた更新は、しまった状態に触らない**（作業単位 A-10）。
            "archived_at": null,
        }),
        "{body}"
    );
    assert_decoy_is_untouched(&server).await;

    server.stop().await;
}

/// **形式指示の「変えない」と「消す」は区別される**（`FieldUpdate` の3値 / 作業単位 0012c）。
///
/// キーの省略が「消す」と同じ扱いになると、**表示名だけを変える更新のたびに
/// 形式指示が黙って消える。** 消えても `prompt` が節ごと落とすだけなので、
/// エラーにも警告にもならず「なんとなく前と違う」としか観測できない。
#[tokio::test]
async fn keeping_the_format_instruction_is_not_the_same_as_clearing_it() {
    let server = TestServer::start().await;
    create_decoy_and_target(&server).await;

    // --- 省略 = 変えない ---
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"display_name": NEW_DISPLAY}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["character"]["output_format_instruction"],
        json!(TARGET_FORMAT),
        "キーを省いた更新で形式指示が消えないこと: {body}"
    );

    // --- 文字列 = 設定する ---
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"output_format_instruction": NEW_FORMAT}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["character"]["output_format_instruction"],
        json!(NEW_FORMAT),
        "{body}"
    );

    // --- null = 消す ---
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"output_format_instruction": null}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["character"]["output_format_instruction"],
        Value::Null,
        "`null` は「消す」であること: {body}"
    );
    let (_, body) = server.get("/admin/characters").await;
    assert_eq!(
        character_in(&body, TARGET_HANDLE)["output_format_instruction"],
        Value::Null,
        "消えた状態が保存されていること: {body}"
    );

    // --- 消えた後も「変えない」は「変えない」（未設定に戻す往復ができること） ---
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"base_persona": NEW_PERSONA}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["character"]["output_format_instruction"],
        Value::Null,
        "{body}"
    );
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"output_format_instruction": TARGET_FORMAT}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["character"]["output_format_instruction"],
        json!(TARGET_FORMAT),
        "消した後にまた設定できること: {body}"
    );

    assert_decoy_is_untouched(&server).await;
    server.stop().await;
}

/// **識別子は更新の要求に載せられない**（ADR-0016 §1 / 作業単位 0012c）。
///
/// `UpdateCharacterRequest` に識別子のフィールドが無く、`/admin/` の方針である
/// `deny_unknown_fields` が効くので、`{"handle": ...}` は**要求まるごと 400** になる。
///
/// **部分的に成功させないことが重要。** 「名前は無視して他は書く」形にすると、
/// 改名したつもりの利用者が「成功した」と読んで**別の列だけが変わった状態**を作る。
#[tokio::test]
async fn the_handle_cannot_be_changed_through_the_update() {
    let server = TestServer::start().await;
    let (_decoy_id, target_id) = create_decoy_and_target(&server).await;
    let expected = invalid_request("リクエスト本文の形式が不正です", "invalid_body");

    // 識別子を指しうるキーは、綴りを問わずすべて弾かれる。
    for key in ["name", "handle", "id", "created_at"] {
        let (status, body) = server
            .patch(
                &character_path(TARGET_HANDLE),
                &json!({key: "rin", "display_name": NEW_DISPLAY}),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{key}: {body}");
        // 本文を完全一致で固定しているので、フィールド名も値も反射しえない
        // （`id` は `invalid_body` の部分文字列なので `contains` では見られない）。
        assert_eq!(parse_json(&body), expected, "{key}");
        assert!(!body.contains("rin"), "入力値を反射しないこと: {body}");
    }

    // **1列も変わっていない**（同じ要求に載せた `display_name` も書かれていない）。
    let (_, body) = server.get("/admin/characters").await;
    assert_eq!(
        character_in(&body, TARGET_HANDLE)["display_name"],
        json!(TARGET_DISPLAY),
        "弾いた要求の他の列も書き込まないこと: {body}"
    );

    // 正当な更新でも識別子は動かない（`model` に保存された値が指し続けること）。
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"display_name": NEW_DISPLAY}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["character"]["handle"],
        json!(TARGET_HANDLE)
    );
    assert_eq!(parse_json(&body)["id"], json!(target_id));

    let (status, body) = server.get("/v1/models").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        names_in(&body, "data"),
        vec![DECOY_HANDLE, TARGET_HANDLE],
        "更新で `model` に載る識別子が変わらないこと: {body}"
    );

    server.stop().await;
}

/// **更新も上限を通る**（削れない枠の絶対制約 / 作業単位 0012c）。
///
/// 表示名・ペルソナ・形式指示はどれも**プロンプトの削れない枠**に載る。
/// 更新がここを素通しすると、`POST` で塞いだ穴が `PATCH` に開き直る
/// ——巨大な値を1件入れただけで以後のチャットが全部 `BudgetExhausted` になり、
/// **しかも今度は更新 API があるので直せる**とはいえ、症状は同じく分かりにくい。
///
/// 理由ごとに区別できること・入力を反射しないことも作成時と同じ。
#[tokio::test]
async fn invalid_updates_are_rejected_with_distinguishable_reasons() {
    let server = TestServer::start().await;
    create_decoy_and_target(&server).await;

    let oversized_display = "零".repeat(1_000);
    let oversized_persona = "零".repeat(100_000);
    let oversized_format = "零".repeat(10_000);

    for (patch, message, code) in [
        (
            json!({"display_name": ""}),
            "表示名が空です",
            "display_name_empty",
        ),
        (
            json!({"display_name": "葵\u{0007}碧"}),
            "表示名に改行や制御文字は使えません",
            "display_name_control",
        ),
        (
            json!({"display_name": oversized_display}),
            "表示名が長すぎます",
            "display_name_too_long",
        ),
        (
            json!({"base_persona": " \u{3000}\t\n"}),
            "ペルソナ本文が空です",
            "persona_empty",
        ),
        (
            json!({"base_persona": oversized_persona}),
            "ペルソナ本文が長すぎます",
            "persona_too_long",
        ),
        (
            json!({"output_format_instruction": ""}),
            "出力形式指示が空です",
            "output_format_empty",
        ),
        (
            json!({"output_format_instruction": oversized_format}),
            "出力形式指示が長すぎます",
            "output_format_too_long",
        ),
        // **尺度も更新の入口で止める**（作業単位 0011a-1）。
        // 素通しにすると、スキーマの CHECK で 500 になるか、
        // CHECK が外れている DB では下限 0 の行が書けてしまう。
        (
            json!({"stat_scale": 0}),
            "性格ステータスの尺度が小さすぎます",
            "stat_scale_too_small",
        ),
        (
            json!({"stat_scale": 101}),
            "性格ステータスの尺度が大きすぎます",
            "stat_scale_too_large",
        ),
        // **走らせ方の別名も更新の入口で止める**（ADR-0045 §決定3 / 作業単位 A-8）。
        (
            json!({"stat_drift_mode": "yes"}),
            "変容の走らせ方の綴りが不正です",
            "stat_drift_mode_invalid",
        ),
        // **見いだす枠の数の上限も更新の入口で止める**（ADR-0046 §決定1 / 作業単位 A-9a）。
        (
            json!({"axis_discovery_slots": 5}),
            "軸を見いだす枠の数が大きすぎます",
            "axis_discovery_slots_too_large",
        ),
    ] {
        let (status, body) = server.patch(&character_path(TARGET_HANDLE), &patch).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{code}: {body}");
        assert_eq!(parse_json(&body), invalid_request(message, code), "{code}");
        assert!(
            !body.contains('零') && !body.contains("1000"),
            "入力も上限値も反射しないこと: {body}"
        );
    }

    // **1件も書き込まれていない。**
    let (_, body) = server.get("/admin/characters").await;
    assert_eq!(
        character_in(&body, TARGET_HANDLE),
        json!({
            "id": character_in(&body, TARGET_HANDLE)["id"],
            "handle": TARGET_HANDLE,
            "display_name": TARGET_DISPLAY,
            "base_persona": TARGET_PERSONA,
            "output_format_instruction": TARGET_FORMAT,
            "stat_scale": TARGET_SCALE,
            "stat_drift_mode": TARGET_DRIFT_MODE,
            "axis_discovery_slots": 0,
            "created_at": character_in(&body, TARGET_HANDLE)["created_at"],
            "archived_at": null,
        }),
        "弾かれた更新が1列も書き込んでいないこと: {body}"
    );
    assert_decoy_is_untouched(&server).await;

    // 存在しないキャラクターの更新は 404（作成しない）。
    let (status, body) = server
        .patch(&character_path("nobody"), &json!({"display_name": "誰か"}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        parse_json(&body),
        invalid_request(
            "指定されたキャラクターが存在しません",
            "character_not_found"
        )
    );
    assert_eq!(
        server.character_count().await,
        2,
        "更新が行を作らないこと（UPSERT にしない）"
    );

    server.stop().await;
}

// ---------------------------------------------------------------------------
// 値が尺度に従う（ADR-0015 / 作業単位 0011a-2）
// ---------------------------------------------------------------------------

/// **`PUT .../stats` はそのキャラクターの尺度で値を検証する。**
///
/// # 既定（10）で検証する実装を、両方向から落とす
///
/// 尺度を `StatScale::DEFAULT` から取る実装は、**尺度が既定と違うキャラクターで
/// 判定が反転する。** 片方向だけでは足りないので、両方を投げる:
///
/// | キャラクター | 尺度 | 値 | 正しい応答 | 既定（10）で検証した場合 |
/// |---|---|---|---|---|
/// | `aoi-v2` | 7 | 8 | **400** | 200（通ってしまう） |
/// | `aoi-v2` | 7 | 7 | 200 | 200（同じ） |
/// | `akane` | 3 | 5 | **400** | 200（通ってしまう） |
/// | `yuzu` | 55 | 20 | **200** | 400（弾かれてしまう） |
///
/// # 「値が尺度を超えた」は入力の誤り（400）
///
/// 状態との衝突（`PATCH` で尺度を下げようとした 409）とは別物。
/// 前者は値を直せば通り、後者は値を先に直さない限り通らない。
#[tokio::test]
async fn a_stat_value_above_the_characters_scale_is_refused_with_its_own_reason() {
    let server = TestServer::start().await;
    assert_fixture_values_are_distinct();

    let decoys = create_scale_decoys(&server).await;
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": TARGET_HANDLE,
                "display_name": TARGET_DISPLAY,
                "base_persona": TARGET_PERSONA,
                "stat_scale": TARGET_SCALE,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // --- 尺度ちょうどは通る（境界の下側） ---
    let (status, body) = server
        .put(
            &stats_path(TARGET_HANDLE),
            &json!({"stats": [{"axis": "パッション", "value": TARGET_SCALE}]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "尺度ちょうどは通ること: {body}");
    assert_eq!(
        parse_json(&body),
        json!({
            "character": TARGET_HANDLE,
            "stat_scale": TARGET_SCALE,
            "stats": [{"axis": "パッション", "value": TARGET_SCALE}],
        }),
        "{body}"
    );

    // --- 1つ超えたら 400（境界の上側）。**既定（10）で検証していたら通ってしまう値** ---
    let (status, body) = server
        .put(
            &stats_path(TARGET_HANDLE),
            &json!({"stats": [{"axis": "ロジック", "value": TARGET_SCALE + 1}]}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        parse_json(&body),
        invalid_request(
            "性格ステータスの値が尺度を超えています",
            "stat_value_above_scale"
        ),
        "理由が区別できること: {body}"
    );
    assert!(
        !body.contains(&TARGET_SCALE.to_string()) && !body.contains("ロジック"),
        "尺度も入力も反射しないこと: {body}"
    );

    // **拒否されたら1行も書き換わっていない**（置き換えは丸ごとの操作）。
    let (status, body) = server.get(&stats_path(TARGET_HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["stats"],
        json!([{"axis": "パッション", "value": TARGET_SCALE}]),
        "拒否された置き換えが前の設定を消していないこと: {body}"
    );

    // --- 囮でも「自分の尺度」で判定される（既定で検証する実装を両方向から落とす） ---
    for (handle, scale) in decoys {
        // 尺度ちょうどは通る。
        let (status, body) = server
            .put(
                &stats_path(handle),
                &json!({"stats": [{"axis": "共感", "value": scale}]}),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "{handle}（尺度 {scale}）はちょうどの値を受け付けること: {body}"
        );

        // 1つ超えたら 400。
        let (status, body) = server
            .put(
                &stats_path(handle),
                &json!({"stats": [{"axis": "共感", "value": scale + 1}]}),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{handle}（尺度 {scale}）は超過を拒むこと: {body}"
        );
        assert_eq!(
            parse_json(&body),
            invalid_request(
                "性格ステータスの値が尺度を超えています",
                "stat_value_above_scale"
            ),
            "{handle}: {body}"
        );
    }

    server.stop().await;
}

/// **保存済みの値より小さい尺度への `PATCH` は 409。**
///
/// # 400 でも 404 でもない
///
/// 要求そのものは正しく、**現在の状態と衝突している**だけなので 409
/// （先例は `character_has_sessions`）。400 にすると「送り方が悪い」と読めてしまい、
/// 実際にやるべきこと（**先に `PUT .../stats` で値を下げる**）に辿り着けない。
///
/// 入力として不正な尺度（`0` / `101`）は今までどおり 400 で、**別のコード**。
#[tokio::test]
async fn lowering_the_stat_scale_below_a_stored_value_is_a_conflict() {
    let server = TestServer::start().await;
    assert_fixture_values_are_distinct();

    let decoys = create_scale_decoys(&server).await;
    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": TARGET_HANDLE,
                "display_name": TARGET_DISPLAY,
                "base_persona": TARGET_PERSONA,
                "stat_scale": TARGET_SCALE,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // 保存済みの最大値を尺度ちょうど（7）にする。**値と尺度の境界を等号で見る**ため。
    let (status, body) = server
        .put(
            &stats_path(TARGET_HANDLE),
            &json!({"stats": [
                {"axis": "パッション", "value": TARGET_SCALE},
                {"axis": "ロジック", "value": 2},
            ]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // --- 保存済みの値より小さい尺度は 409 ---
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"display_name": NEW_DISPLAY, "stat_scale": TARGET_SCALE - 1}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        parse_json(&body),
        invalid_request(
            "保存されている性格ステータスの値より小さい尺度には変更できません",
            "stat_scale_below_stored_value"
        ),
        "理由が区別できること: {body}"
    );

    // **同じ要求に載せた表示名も書き換わっていない**（判定と更新は不可分）。
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        character_in(&body, TARGET_HANDLE)["stat_scale"],
        json!(TARGET_SCALE),
        "尺度が変わっていないこと: {body}"
    );
    assert_eq!(
        character_in(&body, TARGET_HANDLE)["display_name"],
        json!(TARGET_DISPLAY),
        "同じ要求の他の列も書き換わっていないこと: {body}"
    );

    // --- **等しい尺度は通る**（`>` を `>=` に変える変異の見張り） ---
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"stat_scale": TARGET_SCALE}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "等しい尺度は許すこと: {body}");

    // --- 入力として不正な尺度は 400（別のコード） ---
    let (status, body) = server
        .patch(&character_path(TARGET_HANDLE), &json!({"stat_scale": 0}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        parse_json(&body),
        invalid_request("性格ステータスの尺度が小さすぎます", "stat_scale_too_small"),
        "状態との衝突と、入力の不正を潰さないこと: {body}"
    );

    // --- 存在しないキャラクターは 404（衝突ではない） ---
    let (status, body) = server
        .patch(&character_path("nobody"), &json!({"stat_scale": 1}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        parse_json(&body),
        invalid_request(
            "指定されたキャラクターが存在しません",
            "character_not_found"
        ),
        "{body}"
    );

    // --- 復旧経路（ADR-0015「順序で足りる」）: 先に値を下げれば尺度も下げられる ---
    let (status, body) = server
        .put(
            &stats_path(TARGET_HANDLE),
            &json!({"stats": [{"axis": "パッション", "value": 1}]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = server
        .patch(
            &character_path(TARGET_HANDLE),
            &json!({"stat_scale": TARGET_SCALE - 1}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "値を下げた後なら尺度も下げられること: {body}"
    );

    // --- 囮は巻き添えにならない ---
    let (_, body) = server.get("/admin/characters").await;
    for (handle, scale) in decoys {
        assert_eq!(
            character_in(&body, handle)["stat_scale"],
            json!(scale),
            "囮の尺度が動かないこと: {handle}: {body}"
        );
    }

    server.stop().await;
}

/// **尺度を超える行が保存されていても `PUT .../stats` で直せる**（ADR-0015）。
///
/// `stat_allocs` は1行壊れると丸ごと失敗する——**これは既知の制約**で、
/// この作業単位では直さない（`docs/06-roadmap.md` の積み残し）。
/// **だからこそ、復旧経路が実際に通ることを固定する。**
/// 「`replace_stat_allocs` は既存の軸を読まないから届くはず」ではなく、
/// **壊れた行を作って、実際に HTTP で直す。**
#[tokio::test]
async fn a_row_beyond_the_scale_can_be_repaired_through_the_stats_endpoint() {
    let server = TestServer::start().await;
    assert_fixture_values_are_distinct();

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": TARGET_HANDLE,
                "display_name": TARGET_DISPLAY,
                "base_persona": TARGET_PERSONA,
                "stat_scale": TARGET_SCALE,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = parse_json(&body)["id"]
        .as_i64()
        .unwrap_or_else(|| panic!("id は整数であること: {body}"));

    // **尺度を超える行を直接作る。** `PRAGMA ignore_check_constraints` は要らない
    // ——スキーマの CHECK は `value >= 0` しか見ておらず、
    // 尺度と値を結ぶ制約はどこにも無い（`migrations/0001_initial.sql`）。
    sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(id)
        .bind("パッション")
        .bind(i64::from(TARGET_SCALE) + 1)
        .execute(server.store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");

    // --- 取得は止まる（握り潰さない） ---
    let (status, body) = server.get(&stats_path(TARGET_HANDLE)).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "尺度を超える行があると取得は止まること: {body}"
    );
    assert!(
        !body.contains("パッション") && !body.contains(&server.db_path),
        "格納値も内部パスも載せないこと: {body}"
    );

    // --- **置き換えは届く**（既存の軸を読まないので、壊れた行があっても書ける） ---
    let (status, body) = server
        .put(
            &stats_path(TARGET_HANDLE),
            &json!({"stats": [{"axis": "パッション", "value": 4}]}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "壊れた行があっても直せること: {body}"
    );
    assert_eq!(
        parse_json(&body),
        json!({
            "character": TARGET_HANDLE,
            "stat_scale": TARGET_SCALE,
            "stats": [{"axis": "パッション", "value": 4}],
        }),
        "{body}"
    );

    // --- 直った後は取得できる ---
    let (status, body) = server.get(&stats_path(TARGET_HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        parse_json(&body)["stats"],
        json!([{"axis": "パッション", "value": 4}]),
        "{body}"
    );

    server.stop().await;
}

/// **壊れた性格ステータスがあっても、尺度は一覧から引ける**（作業単位 D-8）。
///
/// # なぜこの1本が要るのか
///
/// **画面（`web/src/lib/StatsPanel.svelte`）が、この事実に寄りかかる。**
/// `GET .../stats` が 500 の状態でも修復のフォームを描くには、
/// **そのキャラクターの尺度**（値の上端）が要る。
/// **`GET /admin/characters/{handle}` は存在しない**ので（`PATCH` と `DELETE` だけ /
/// ADR-0016 §1）、**尺度の出どころは一覧しか無い。**
///
/// **「一覧は `stat_alloc` を読まないから大丈夫なはず」で済ませない**
/// （CLAUDE.md「『API はある』と書くときは、読む側を実際に呼んで確かめること」）。
/// **壊れた行を作って、3つの入口を実際に叩く。**
///
/// # 測っていること
///
/// | 入口 | どうなるか |
/// |---|---|
/// | `GET .../stats` | **500** |
/// | `GET .../drift` | **500**（`get_drift` も `Store::stat_allocs` を読む） |
/// | `GET /admin/characters` | **200。しかもこの行の尺度が出る** |
///
/// **`drift` も落ちることが効く。** 画面は2つの読みを独立に持っているが、
/// **格納値が壊れた回は両方落ちる**ので、修復の入口を「変容が読めたとき」に
/// 結び付けてはならない（結び付けると、本物の壊れ方では一度も出ない）。
///
/// # ⚠ 500 に `code` は付かない
///
/// `ApiError::internal()` の `code` は `None`（`src/server/error.rs`）。
/// つまり **FE は「格納値が壊れている」と「DB 障害」を区別できない。**
/// 区別できるのは「サーバへ届かなかった」（`fetch` が失敗 = 状態番号 0）までで、
/// **そこから先は同じ 500 である。** 画面はそう書く（`statsRepair.ts`）。
#[tokio::test]
async fn a_broken_stat_row_stops_stats_and_drift_but_the_listing_still_reports_the_scale() {
    let server = TestServer::start().await;
    // **囮を先に作る**（対象は識別子順で3番目 / 尺度の取り違えを見るため）。
    let decoys = create_scale_decoys(&server).await;

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": TARGET_HANDLE,
                "display_name": TARGET_DISPLAY,
                "base_persona": TARGET_PERSONA,
                "stat_scale": TARGET_SCALE,
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = parse_json(&body)["id"]
        .as_i64()
        .unwrap_or_else(|| panic!("id は整数であること: {body}"));

    // **尺度を超える行を直接作る**（`PUT` からは作れない壊れ方 /
    // `a_row_beyond_the_scale_can_be_repaired_through_the_stats_endpoint` と同じ仕込み）。
    sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(id)
        .bind("パッション")
        .bind(i64::from(TARGET_SCALE) + 1)
        .execute(server.store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");

    // --- 2つの読みは、どちらも止まる ---
    let (status, body) = server.get(&stats_path(TARGET_HANDLE)).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "いまの値の読みが止まること: {body}"
    );
    let (status, drift_body) = server
        .get(&format!("/admin/characters/{TARGET_HANDLE}/drift"))
        .await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "変容の読みも止まること（同じ `Store::stat_allocs` を通る）: {drift_body}"
    );

    // **`code` が無い** —— FE は「壊れている」と「DB 障害」を区別できない。
    for body in [&body, &drift_body] {
        assert_eq!(
            parse_json(body)["error"].get("code"),
            None,
            "500 に細分類は付かないこと: {body}"
        );
        assert!(
            !body.contains("パッション") && !body.contains(&server.db_path),
            "格納値も内部パスも載せないこと: {body}"
        );
    }

    // --- 一覧は読める。**この行の尺度が出る** ---
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        character_in(&body, TARGET_HANDLE)["stat_scale"],
        json!(TARGET_SCALE),
        "壊れた性格ステータスがあっても、尺度は一覧から引けること: {body}"
    );
    // **囮の尺度と取り違えていない**（先頭 / 2件目 / 末尾 / 最小 / 最大のどれでもない）。
    for (handle, scale) in decoys {
        assert_eq!(
            character_in(&body, handle)["stat_scale"],
            json!(scale),
            "囮の尺度が動かないこと: {handle}: {body}"
        );
    }
    // **壊れた行として指し示されてもいない** —— 壊れているのは `stat_alloc` であって、
    // `character` の行ではない（画面はこの区別で修復の入口を出し分ける）。
    assert_eq!(
        parse_json(&body)["unreadable"],
        json!([]),
        "character の行は読めていること: {body}"
    );

    server.stop().await;
}

// ---------------------------------------------------------------------------
// 変容の走らせ方（ADR-0045 / 作業単位 A-8）
// ---------------------------------------------------------------------------

/// **作成で書いた走らせ方が、作成の応答・一覧・更新の応答に出る。省略は `null`**
/// （ADR-0045 §決定1・§決定3）。本番の入口を通す。
///
/// * **省略した作成は `null`**（既定の値で埋めない）。
/// * **囮（省略）と対象（`shadow`）を並べる**——一覧の行ごとに値が違うことを見る。
#[tokio::test]
async fn a_drift_mode_written_at_creation_is_named_in_every_character_response() {
    let server = TestServer::start().await;

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({"handle": "akane", "base_persona": "昼番の受付。"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        parse_json(&body).get("stat_drift_mode"),
        Some(&Value::Null),
        "省略は未設定（null）でキーごと残る: {body}"
    );

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": "aoi",
                "base_persona": "ペルソナ本文",
                "stat_drift_mode": "shadow",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        parse_json(&body).get("stat_drift_mode"),
        Some(&json!("shadow")),
        "{body}"
    );

    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        [
            character_in(&body, "akane").get("stat_drift_mode").cloned(),
            character_in(&body, "aoi").get("stat_drift_mode").cloned(),
        ],
        [Some(Value::Null), Some(json!("shadow"))],
        "一覧でも行ごとの値が出ること: {body}"
    );

    server.stop().await;
}

/// **`PATCH` の3状態**（省く＝変えない / `null`＝未設定へ戻す / 綴り＝設定）を本番の入口で見る
/// （ADR-0045 §決定3）。
///
/// **省いた更新は、形式指示だけを変える要求で試す**——同じ要求の中で
/// 「変えるもの（形式指示）」と「変えないもの（走らせ方）」が同居する。
/// **囮の行は最後まで `null`**（別の行へ書く取り違えを見る）。
#[tokio::test]
async fn the_drift_mode_patch_has_three_states_over_http() {
    let server = TestServer::start().await;
    for (handle, persona) in [("akane", "昼番の受付。"), ("aoi", "ペルソナ本文")] {
        let (status, body) = server
            .post(
                "/admin/characters",
                &json!({"handle": handle, "base_persona": persona}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    let patch_mode = |body: Value| {
        let server = &server;
        async move {
            let (status, text) = server.patch(&character_path("aoi"), &body).await;
            assert_eq!(status, StatusCode::OK, "{body}: {text}");
            parse_json(&text)["character"]["stat_drift_mode"].clone()
        }
    };

    assert_eq!(
        patch_mode(json!({"stat_drift_mode": "on"})).await,
        json!("on"),
        "綴りで設定"
    );
    assert_eq!(
        patch_mode(json!({"output_format_instruction": "台詞は「」で囲むこと。"})).await,
        json!("on"),
        "キーを省いた更新は変えない"
    );
    assert_eq!(
        patch_mode(json!({"stat_drift_mode": "off"})).await,
        json!("off"),
        "別の綴りへ置き換わる"
    );
    assert_eq!(
        patch_mode(json!({"stat_drift_mode": null})).await,
        Value::Null,
        "null で未設定へ戻る"
    );

    let (_, body) = server.get("/admin/characters").await;
    assert_eq!(
        character_in(&body, "akane").get("stat_drift_mode"),
        Some(&Value::Null),
        "囮は無傷: {body}"
    );

    server.stop().await;
}

/// **環境変数の別名は、作成でも `PATCH` でも 400**（ADR-0045 §決定3）。1列も書かれない。
///
/// 本文は**完全一致で固定**する（入力を反射しないことも同時に見る）。
#[tokio::test]
async fn an_alias_of_the_drift_mode_is_refused_over_http() {
    let server = TestServer::start().await;
    let refused = invalid_request("変容の走らせ方の綴りが不正です", "stat_drift_mode_invalid");

    for alias in ["1", "true", "yes", "0", "false", "no", "", "ON", " on"] {
        let (status, body) = server
            .post(
                "/admin/characters",
                &json!({
                    "handle": "aoi",
                    "base_persona": "ペルソナ本文",
                    "stat_drift_mode": alias,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{alias:?}: {body}");
        assert_eq!(parse_json(&body), refused, "{alias:?}");
    }
    assert_eq!(server.character_count().await, 0, "1件も作られていない");

    let (status, body) = server
        .post(
            "/admin/characters",
            &json!({
                "handle": "aoi",
                "base_persona": "ペルソナ本文",
                "stat_drift_mode": "shadow",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    for alias in ["yes", "ON", "true"] {
        let (status, body) = server
            .patch(
                &character_path("aoi"),
                // **他の列を一緒に送る**——1つでも不正なら1列も書かれないこと。
                &json!({"display_name": "碧", "stat_drift_mode": alias}),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{alias:?}: {body}");
        assert_eq!(parse_json(&body), refused, "{alias:?}");
    }

    let (_, body) = server.get("/admin/characters").await;
    let row = character_in(&body, "aoi");
    assert_eq!(
        (row.get("stat_drift_mode"), row.get("display_name")),
        (Some(&json!("shadow")), Some(&json!("aoi"))),
        "拒否された更新は1列も書かない: {body}"
    );

    server.stop().await;
}
