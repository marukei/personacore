//! `/admin/characters/{handle}/memories` の検証（ADR-0022 / 作業単位 14b）。
//!
//! **LM Studio には繋がない。** 管理 API は上流を必要としない。
//! **サーバは本番の入口（`App::bind`）から起動する**——テスト専用の入口を作ると、
//! 本番の配線が誰にも通られないまま残る（CLAUDE.md「テスト専用の入口を作ると…」）。
//!
//! ここで固定するのは主に次の7点:
//!
//!   * **修復の一覧が `subject` を必ず載せる**こと（絶対制約。落とすと誰の記憶か分からない）。
//!   * **`locked` を応答に出さない**こと（ADR-0022 §4。意味が未確定の値を API に出すと、
//!     外から意味を決められてしまう）。**壊れた列としては指し示す**（値は出さない）。
//!   * **壊れた行が一覧に現れる**こと、かつ**壊れた行があっても一覧が丸ごと落ちない**こと。
//!   * **訂正 → もう一度一覧**で、古い行が消え訂正版が現れること。
//!     **行は消えていない**こと（`superseded_by` が後継を指しているだけ）。
//!     **想起（`recall_memories`）からも消えている**こと。
//!   * **拒否3種がそれぞれ別の応答**になること（404 / 404+別コード / 409）。
//!     **1つに潰す変異が赤になる**ように本文を完全一致で固定する。
//!   * **後継の無い取り消しを作れない**こと（訂正版の中身が必須 / ADR-0022 §1）。
//!   * エラーレスポンスに**内部情報を載せない**こと。
//!
//! 一覧そのものの意味論（上書き済みを外す・並び・キャラクターで閉じる）は
//! `tests/store_memory.rs`（14a）が持つ。ここは **HTTP 越しに同じ線が繋がること**を見る。

use std::net::SocketAddr;

use chrono::{DateTime, Utc};
use personacore::domain::{CharacterId, Handle, MemoryCategory, MemoryContent, NewMemory, Subject};
use personacore::server::{App, AppConfig, BindAddr, ServerError};
use personacore::store::Store;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

mod common;

use common::{CLOCK_SKEW, insert_memory_ignoring_checks, insert_superseded_memory_at};

/// エフェメラルポートで起動した実サーバ。
struct TestServer {
    addr: SocketAddr,
    /// アプリと**同じプールを指す**ハンドル。記憶の仕込みと行の直接確認に使う。
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

    async fn post(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        self.send(Method::POST, path, Some(body)).await
    }

    /// キャラクターを1件作る。**識別子と表示名は違う文字列。**
    async fn create_character(&self, handle: &str, display_name: &str) -> CharacterId {
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
        self.character_id(handle).await
    }

    /// キャラクターの主キー（記憶を仕込むために要る）。
    async fn character_id(&self, raw: &str) -> CharacterId {
        let handle = Handle::parse(raw).expect("テストで使う識別子は正当であること");
        self.store
            .find_character_by_handle(&handle)
            .await
            .expect("引けること")
            .expect("作成済みであること")
            .id
    }

    /// **健全な記憶**を1件積む（`store` 経由 = 抽出と同じ書き込み口）。
    async fn add_memory(
        &self,
        character_id: CharacterId,
        subject: Subject,
        content: &str,
        category: Option<&str>,
    ) -> i64 {
        let memory = NewMemory {
            subject,
            content: MemoryContent::parse(content).expect("正当な本文であること"),
            category: category.map(|raw| MemoryCategory::parse(raw).expect("正当な分類であること")),
        };
        self.store
            .add_memory(character_id, &memory)
            .await
            .expect("記憶を積めること")
            .get()
    }

    /// **本番の入口で**訂正する（`POST .../corrections`）。追加された行の id を返す。
    ///
    /// 素の SQL の仕込みと違い、**書き手と読み手が繋がっていること**を通す唯一の道。
    async fn correct(
        &self,
        handle: &str,
        memory_id: i64,
        subject: &str,
        content: &str,
        category: Option<&str>,
    ) -> i64 {
        let (status, body) = self
            .post(
                &corrections_path(handle, memory_id),
                &json!({"subject": subject, "content": content, "category": category}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "訂正できること: {body}");
        assert_eq!(
            body.get("superseded_id"),
            Some(&json!(memory_id)),
            "訂正した相手を名乗ること: {body}"
        );
        body.get("correction_id")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| panic!("correction_id を載せること: {body}"))
    }

    /// **上書き済みの行を、時刻を明示して仕込む**（作業単位 D-3b）。行の id を返す。
    ///
    /// 本番の書き込み口（`POST .../corrections`）は `created_at` を現在時刻に任せるので、
    /// **行ごとに違う時刻も、上限を超える件数も、こちらでしか作れない。**
    async fn plant_superseded(
        &self,
        character_id: CharacterId,
        subject: &str,
        content: &str,
        category: Option<&str>,
        created_at: &str,
        superseded_by: i64,
    ) -> i64 {
        insert_superseded_memory_at(
            &self.store,
            character_id,
            subject,
            content,
            category,
            created_at,
            superseded_by,
        )
        .await
    }

    /// `memory` の行数（**拒否されたのに書かれていない**ことの確認に使う）。
    async fn memory_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM memory")
            .fetch_one(self.store.pool())
            .await
            .expect("行数を数えられること")
    }

    /// 行の `superseded_by`（**想起から外れた行も引ける**）。
    async fn superseded_by_of(&self, id: i64) -> Option<i64> {
        sqlx::query_scalar("SELECT superseded_by FROM memory WHERE id = ?")
            .bind(id)
            .fetch_one(self.store.pool())
            .await
            .expect("行を直接引けること")
    }

    /// 行の本文（**消えていない**ことの確認に使う）。
    async fn content_of(&self, id: i64) -> Option<String> {
        sqlx::query_scalar("SELECT content FROM memory WHERE id = ?")
            .bind(id)
            .fetch_optional(self.store.pool())
            .await
            .expect("行を直接引けること")
    }

    /// **想起の側**（`recall_memories`）から見える本文。
    ///
    /// 修復の一覧とは**別の入口**（ADR-0003 / ADR-0022 §2）。
    /// 訂正が効いたことをここでも見る——一覧だけを見ていると、
    /// 「一覧の絞り込みだけが変わって想起は変わっていない」形を見逃す。
    async fn recalled_contents(&self, character_id: CharacterId) -> Vec<String> {
        self.store
            .recall_memories(character_id)
            .await
            .expect("想起できること")
            .into_iter()
            .map(|memory| memory.content.as_str().to_owned())
            .collect()
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

/// 一覧のパス。
fn memories_path(handle: &str) -> String {
    format!("/admin/characters/{handle}/memories")
}

/// 訂正のパス。
fn corrections_path(handle: &str, memory_id: i64) -> String {
    format!("/admin/characters/{handle}/memories/{memory_id}/corrections")
}

/// エラー応答の本文を**完全一致**で組み立てる（内部情報が混ざったら赤になる）。
fn error_body(message: &str, code: &str) -> Value {
    json!({"error": {"message": message, "type": "invalid_request_error", "code": code}})
}

/// **内部情報が漏れていないこと。** どのエラー応答にも通す。
fn assert_no_internals(server: &TestServer, body: &Value) {
    let text = body.to_string();
    for leak in [
        server.db_path.as_str(),
        "sqlite",
        "sqlx",
        "personacore.sqlite",
        "src\\",
        "src/",
        "memory.id",
        "superseded_by",
    ] {
        assert!(
            !text.contains(leak),
            "内部情報 {leak:?} が漏れていないこと: {text}"
        );
    }
}

/// 応答から `created_at` を**抜き取って**日時として検証する。
///
/// 動的な値が1つ混ざると本文を完全一致で比較できない。**抜くときに必ず見る**ので、
/// `created_at` を落とす変異はここで赤になる（キーが無ければ panic する）。
fn take_created_at(memory: &mut Value) -> DateTime<Utc> {
    let shown = memory.to_string();
    let object = memory
        .as_object_mut()
        .unwrap_or_else(|| panic!("記憶はオブジェクトであること: {shown}"));
    let raw = object
        .remove("created_at")
        .unwrap_or_else(|| panic!("created_at を載せること: {shown}"));
    let raw = raw.as_str().expect("created_at は文字列であること");
    DateTime::parse_from_rfc3339(raw)
        .unwrap_or_else(|err| panic!("RFC3339 として読めること: {err} / {raw}"))
        .with_timezone(&Utc)
}

/// 一覧の `memories` から `created_at` を抜き、**どれも現在時刻の近傍**であることを見る。
fn strip_created_at(body: &mut Value) {
    let now = Utc::now();
    let shown = body.to_string();
    let memories = body
        .get_mut("memories")
        .unwrap_or_else(|| panic!("memories を載せること: {shown}"))
        .as_array_mut()
        .expect("memories は配列であること");
    for memory in memories {
        let created_at = take_created_at(memory);
        assert!(
            created_at >= now - CLOCK_SKEW && created_at <= now + CLOCK_SKEW,
            "created_at が現在時刻の近傍であること: {created_at}"
        );
    }
}

/// **修復の一覧は `subject` を必ず載せ、`locked` は載せない。**
///
/// # `subject` を落とせない理由（絶対制約）
///
/// `user` / `character` の区別を落とすと、応答を見た運用者は
/// 「猫を飼っている」が**誰の話か判断できない**。訂正版を書くときに主語を取り違えると、
/// 一人称が混線したまま記憶が固定される。
///
/// # `locked` を載せない理由（ADR-0022 §4）
///
/// **`locked` が何を意味するかはまだ決まっていない。** 決めているのは
/// 「0 / 1 以外なら読めない行」だけ。意味が未確定の値を API に出すと、
/// **外から意味を決められてしまう。**
///
/// # フィクスチャが同値を作っていないこと
///
/// * `subject` は `user` と `character` の**両方**が並ぶ（片方だけだと固定と区別できない）。
/// * `category` は**有り・無し**の両方（`null` を落とす変異が赤になる）。
/// * **`locked = 1` の健全な行を1件入れてある**——`locked` を応答に足す変異は、
///   この行で「true が漏れる」形になる（0 だけのフィクスチャでは既定値に埋もれる）。
/// * 本文・分類・主語のどれも互いに同じ文字列にしていない。
#[tokio::test]
async fn memories_are_listed_for_repair_with_their_subject_and_without_the_locked_flag() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    let rin = server.create_character("rin", "凛").await;

    let first = server
        .add_memory(aoi, Subject::User, "猫を飼っている", Some("好み"))
        .await;
    let second = server
        .add_memory(aoi, Subject::Character, "図書館で働いている", None)
        .await;
    // **`locked = 1` の健全な行**（`store` に印を立てる API は無い / ADR-0022 §4）。
    let locked = insert_memory_ignoring_checks(
        &server.store,
        aoi,
        "user",
        "コーヒーが苦手",
        Some("食事"),
        1,
    )
    .await;
    // **別キャラクターの記憶は混ざらない。**
    server
        .add_memory(rin, Subject::Character, "犬を飼っている", Some("家族"))
        .await;

    let (status, mut body) = server.get(&memories_path("aoi")).await;
    assert_eq!(status, StatusCode::OK, "一覧が返ること: {body}");
    strip_created_at(&mut body);

    assert_eq!(
        body,
        json!({
            "character": "aoi",
            "memories": [
                {"id": first, "subject": "user", "content": "猫を飼っている", "category": "好み"},
                {
                    "id": second,
                    "subject": "character",
                    "content": "図書館で働いている",
                    "category": null,
                },
                {"id": locked, "subject": "user", "content": "コーヒーが苦手", "category": "食事"},
            ],
            "unreadable": [],
        }),
        "subject を載せ、locked を載せず、別キャラクターの記憶を混ぜないこと"
    );

    server.stop().await;
}

/// **壊れた行は「壊れている」と示され、健全な行を巻き添えにしない。**
///
/// `/admin/characters` の一覧（`unreadable`）と同じ形（ADR-0016 §3 / ADR-0022 §2）。
/// **黙って読み飛ばす実装（`filter_map`）はそのままコンパイルが通る**ので、
/// 守っているのはこの見張り番だけ（`MemoryEntry` の doc）。
///
/// # `locked` が壊れた行を入れてある理由
///
/// ADR-0022 §4 が訂正した当の穴。`locked ∉ {0,1}` の行は
/// **チャットも抽出も止めている**のに、`locked` を検証しない一覧では
/// 「読めた行」として健全に見える。**値は出さないが、壊れた列としては指し示す。**
#[tokio::test]
async fn a_broken_memory_row_appears_in_the_listing_without_taking_the_healthy_ones() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;

    let healthy = server
        .add_memory(aoi, Subject::Character, "図書館で働いている", Some("経歴"))
        .await;
    // 本文が空白だけの行（長さ・空白の制約はスキーマに無いので素の INSERT でも作れる）。
    let blank =
        insert_memory_ignoring_checks(&server.store, aoi, "user", "\u{3000} \t", None, 0).await;
    // `locked` が 0 / 1 以外の行（**この行がチャットと抽出を止めている**）。
    let bad_locked = insert_memory_ignoring_checks(
        &server.store,
        aoi,
        "character",
        "ロックが壊れた記憶",
        Some("好み"),
        7,
    )
    .await;

    let (status, mut body) = server.get(&memories_path("aoi")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "壊れた行があっても一覧が落ちないこと: {body}"
    );
    strip_created_at(&mut body);

    assert_eq!(
        body,
        json!({
            "character": "aoi",
            "memories": [
                {
                    "id": healthy,
                    "subject": "character",
                    "content": "図書館で働いている",
                    "category": "経歴",
                },
            ],
            "unreadable": [
                {"id": blank, "invalid_columns": ["content"]},
                {"id": bad_locked, "invalid_columns": ["locked"]},
            ],
        }),
        "壊れた行を id と列名で指し示し、健全な行は普通に並ぶこと"
    );

    // **格納値そのものは応答に出さない**（`UnreadableMemory` は持っていない）。
    let text = body.to_string();
    assert!(
        !text.contains("ロックが壊れた記憶"),
        "読めなかった行の格納値を載せないこと: {text}"
    );

    server.stop().await;
}

/// **訂正 → もう一度一覧。古い行は消え、訂正版が現れる。行そのものは残る。**
///
/// ADR-0022 の中心を HTTP 越しに1本の線として見る。確かめるのは4つ:
///
/// 1. 訂正の応答が**古い行と訂正版の id を両方**返す（次の一手の手掛かり）。
/// 2. **一覧から古い行が消え、訂正版が現れる。**
/// 3. **想起（`recall_memories`）からも消えている**——一覧の絞り込みだけが
///    変わった形を見逃さない。
/// 4. **行は消えていない**（本文がそのまま残り、`superseded_by` が後継を指す）。
///
/// # フィクスチャは訂正の前後で全部の値を違えてある
///
/// `subject` も `content` も `category` も違う。同値にすると
/// 「訂正版の列を古い行から写す」実装が緑を通る。
#[tokio::test]
async fn a_correction_replaces_the_old_memory_in_the_listing_and_in_recall() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;

    let untouched = server
        .add_memory(aoi, Subject::Character, "図書館で働いている", Some("経歴"))
        .await;
    let wrong = server
        .add_memory(aoi, Subject::User, "黒い犬を飼っている", Some("家族"))
        .await;

    let (status, body) = server
        .post(
            &corrections_path("aoi", wrong),
            &json!({
                "subject": "character",
                "content": "黒い猫を飼っている",
                "category": "好み",
            }),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "訂正できること: {body}");

    let correction = body
        .get("correction_id")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| panic!("correction_id を載せること: {body}"));
    assert_eq!(
        body,
        json!({"character": "aoi", "superseded_id": wrong, "correction_id": correction}),
        "訂正した行と訂正版の id を両方返すこと"
    );
    assert_ne!(correction, wrong, "訂正版は新しい行であること");

    let (status, mut listing) = server.get(&memories_path("aoi")).await;
    assert_eq!(status, StatusCode::OK, "一覧が返ること: {listing}");
    strip_created_at(&mut listing);
    assert_eq!(
        listing,
        json!({
            "character": "aoi",
            "memories": [
                {
                    "id": untouched,
                    "subject": "character",
                    "content": "図書館で働いている",
                    "category": "経歴",
                },
                {
                    "id": correction,
                    "subject": "character",
                    "content": "黒い猫を飼っている",
                    "category": "好み",
                },
            ],
            "unreadable": [],
        }),
        "古い行が消え、訂正版が現れること（他の記憶は動かない）"
    );

    assert_eq!(
        server.recalled_contents(aoi).await,
        vec![
            "図書館で働いている".to_owned(),
            "黒い猫を飼っている".to_owned(),
        ],
        "想起の側からも古い行が消えていること"
    );

    // **行は消えていない**（時系列保持 / 絶対制約）。
    assert_eq!(
        server.content_of(wrong).await.as_deref(),
        Some("黒い犬を飼っている"),
        "古い行の本文がそのまま残っていること"
    );
    assert_eq!(
        server.superseded_by_of(wrong).await,
        Some(correction),
        "古い行が後継を指していること"
    );
    assert_eq!(
        server.memory_count().await,
        3,
        "行は増えるだけで、減らないこと"
    );

    server.stop().await;
}

/// **壊れた行も API だけで直せる**（ADR-0022 §2 の存在理由）。
///
/// 一覧が指した id を訂正に貼るだけで、`unreadable` から消える。
/// **壊れた行でも訂正版の中身は必須**（§1。「消す」道は作らない）。
#[tokio::test]
async fn a_broken_memory_can_be_repaired_through_the_api() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;

    let broken =
        insert_memory_ignoring_checks(&server.store, aoi, "user", "\u{3000} \t", None, 0).await;

    let (status, body) = server.get(&memories_path("aoi")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["unreadable"],
        json!([{"id": broken, "invalid_columns": ["content"]}]),
        "一覧が壊れた行を指すこと: {body}"
    );

    let (status, body) = server
        .post(
            &corrections_path("aoi", broken),
            &json!({"subject": "user", "content": "紅茶が好き", "category": "好み"}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "壊れた行も訂正できること: {body}"
    );

    let (status, mut listing) = server.get(&memories_path("aoi")).await;
    assert_eq!(status, StatusCode::OK);
    strip_created_at(&mut listing);
    assert_eq!(
        listing["unreadable"],
        json!([]),
        "壊れた行が修復の対象から外れること: {listing}"
    );
    assert_eq!(
        listing["memories"],
        json!([{
            "id": body["correction_id"],
            "subject": "user",
            "content": "紅茶が好き",
            "category": "好み",
        }]),
        "訂正版だけが並ぶこと: {listing}"
    );

    // **壊れた行も消えていない。**
    assert_eq!(
        server.superseded_by_of(broken).await,
        body["correction_id"].as_i64(),
        "壊れた行が後継を指していること"
    );

    server.stop().await;
}

/// **存在しない記憶を訂正しようとしたら 404。**
///
/// 打ち間違いなので、綴り（id）を直せば通る。**別キャラクターの記憶とは別のコード**で返す
/// ——調査先が違う（`UnknownMemory` の doc）。
#[tokio::test]
async fn correcting_a_memory_that_does_not_exist_is_refused_with_its_own_reason() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    server
        .add_memory(aoi, Subject::User, "猫を飼っている", Some("好み"))
        .await;

    let (status, body) = server
        .post(
            &corrections_path("aoi", 9_999),
            &json!({"subject": "user", "content": "犬を飼っている", "category": "家族"}),
        )
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "404 であること: {body}");
    assert_eq!(
        body,
        error_body("指定された記憶が存在しません", "memory_not_found")
    );
    assert_no_internals(&server, &body);
    assert_eq!(
        server.memory_count().await,
        1,
        "拒否された訂正版が書かれていないこと"
    );

    server.stop().await;
}

/// **別のキャラクターの記憶を指したら、宛先違いとして 404 + 専用コード。**
///
/// # なぜ「存在しない」と同じ応答に潰さないのか
///
/// **同じ応答にすると、運用者は id を疑って探し直す**——正しい id を持っているのに。
/// 直すべきはパスの `handle` の方で、**綴りを直せば通る**。
/// この API に認証は無く（ADR-0008）、**同じ相手が
/// `GET /admin/characters/rin/memories` を叩けば同じ記憶を読める**ので、
/// 存在を隠しても守れるものが無い。
///
/// # なぜ「上書き済み」（409）とも別なのか
///
/// ADR-0022 の「拒否の優先順」がそのまま HTTP に出る。
/// **上書き済みかどうかは、他キャラクターの記憶については明かさない。**
#[tokio::test]
async fn correcting_a_memory_of_another_character_is_refused_with_its_own_reason() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let rin = server.create_character("rin", "凛").await;

    let rins = server
        .add_memory(rin, Subject::User, "凛に話したこと", Some("会話"))
        .await;

    let (status, body) = server
        .post(
            &corrections_path("aoi", rins),
            &json!({"subject": "user", "content": "葵に話したこと", "category": "会話"}),
        )
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "404 であること: {body}");
    assert_eq!(
        body,
        error_body(
            "指定された記憶はこのキャラクターのものではありません",
            "memory_belongs_to_another_character",
        )
    );
    assert_no_internals(&server, &body);
    assert_eq!(
        server.memory_count().await,
        1,
        "訂正版が書かれていないこと（INSERT が先に走っていない）"
    );
    assert_eq!(
        server.superseded_by_of(rins).await,
        None,
        "他キャラクターの記憶に印が付いていないこと"
    );

    server.stop().await;
}

/// **別キャラクターの・すでに上書き済みの記憶**を指すと、返るのは**宛先違い**。
///
/// # フィクスチャが「2つの条件を同時に成立させる」ことに意味がある
///
/// 1つずつしか成立しないフィクスチャでは、**宛先違いの判定を
/// 「上書きされていないときだけ」に条件化する変異が緑を通る**（14a のレビューで実測）。
///
/// `memory_already_superseded` を返してしまうと、
/// **「その記憶は存在し、訂正済みである」という他キャラクターの状態の開示**になる
/// （ADR-0022「拒否の優先順」）。**宛先違いは鎖の状態を一切明かさない。**
#[tokio::test]
async fn a_superseded_memory_of_another_character_still_answers_wrong_owner() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let rin = server.create_character("rin", "凛").await;

    let rins_old = server
        .add_memory(rin, Subject::User, "凛に話したこと", Some("会話"))
        .await;
    let (status, correction) = server
        .post(
            &corrections_path("rin", rins_old),
            &json!({"subject": "user", "content": "凛に話し直したこと", "category": "会話"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "凛の側では訂正できること");
    assert_eq!(
        server.superseded_by_of(rins_old).await,
        correction["correction_id"].as_i64(),
        "上書き済みの状態を作れていること"
    );

    let (status, body) = server
        .post(
            &corrections_path("aoi", rins_old),
            &json!({"subject": "user", "content": "葵に話したこと", "category": "会話"}),
        )
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "404 であること: {body}");
    assert_eq!(
        body,
        error_body(
            "指定された記憶はこのキャラクターのものではありません",
            "memory_belongs_to_another_character",
        ),
        "宛先違いを先に返すこと（鎖の状態を明かさない）"
    );

    server.stop().await;
}

/// **すでに訂正済みの記憶をもう一度訂正しようとしたら 409。**
///
/// 要求そのものは正しく、**現在の状態と衝突している**（先例は `character_has_sessions`）。
/// 400 にすると「送り方が悪い」と読めてしまい、実際にやるべきこと
/// （**一覧を取り直して、いま有効な行を指す**）に辿り着けない。
#[tokio::test]
async fn correcting_an_already_superseded_memory_is_refused_with_its_own_reason() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;

    let old = server
        .add_memory(aoi, Subject::User, "黒い犬を飼っている", Some("家族"))
        .await;
    let (status, first) = server
        .post(
            &corrections_path("aoi", old),
            &json!({"subject": "user", "content": "黒い猫を飼っている", "category": "好み"}),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "1回目は通ること: {first}");

    let (status, body) = server
        .post(
            &corrections_path("aoi", old),
            &json!({"subject": "character", "content": "白い猫を飼っている", "category": "好み"}),
        )
        .await;

    assert_eq!(status, StatusCode::CONFLICT, "409 であること: {body}");
    assert_eq!(
        body,
        error_body(
            "指定された記憶はすでに訂正されています。いま有効な記憶を指定してください",
            "memory_already_superseded",
        )
    );
    assert_no_internals(&server, &body);
    assert_eq!(
        server.memory_count().await,
        2,
        "2回目の訂正版が書かれていないこと"
    );
    assert_eq!(
        server.superseded_by_of(old).await,
        first["correction_id"].as_i64(),
        "鎖が分岐していないこと（後継は最初の訂正版のまま）"
    );

    server.stop().await;
}

/// **後継の無い取り消しは要求として組み立てられない**（ADR-0022 §1）。
///
/// 本文を省くと 400。**`memory_content_empty` ではなく `invalid_body`** で返る
/// ——「本文は任意」に変える変異は、この違いで赤になる
/// （任意にすると空文字が既定になり、`MemoryContent::parse` の空エラーに化ける）。
///
/// > **⚠ 未知のキーは、このテストでは観測できない**（14b のレビューで実測）。
/// > ここの入力は**すべて本文を欠いている**ので、`deny_unknown_fields` が無くても
/// > `missing field content` で同じ `invalid_body` になる。
/// > **未知のキーの見張り番は
/// > `an_unknown_key_in_an_otherwise_valid_correction_is_refused_and_writes_nothing`**
/// > ——**本文が正当であることが要点**で、本文を欠かせた入力ではその枝に届かない。
#[tokio::test]
async fn a_correction_without_content_is_refused_and_writes_nothing() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    let target = server
        .add_memory(aoi, Subject::User, "黒い犬を飼っている", Some("家族"))
        .await;

    for request in [
        json!({"subject": "user"}),
        json!({"subject": "user", "category": "好み"}),
        json!({"subject": "user", "content": null}),
        json!({}),
        // 「後継なしで無効化する」形の要求（本文が無い）。
        json!({"subject": "user", "supersede": true}),
    ] {
        let (status, body) = server
            .post(&corrections_path("aoi", target), &request)
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "本文の無い訂正を拒むこと: {request} → {body}"
        );
        assert_eq!(
            body,
            error_body("リクエスト本文の形式が不正です", "invalid_body"),
            "要求 {request}"
        );
        assert_no_internals(&server, &body);
    }

    assert_eq!(server.memory_count().await, 1, "1行も書かれていないこと");
    assert_eq!(
        server.superseded_by_of(target).await,
        None,
        "対象に印が付いていないこと"
    );

    server.stop().await;
}

/// **不備は欄ごとに別の理由で返す。** どこを直せばよいかが応答から分かる。
///
/// `assistant` は `message.role` の綴りで、**現実に起きる取り違え**。
/// `invalid_body`（「本文の形式が不正」）に潰すと直す場所が分からない。
#[tokio::test]
async fn an_invalid_correction_is_refused_with_the_reason_of_the_field() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    let target = server
        .add_memory(aoi, Subject::User, "黒い犬を飼っている", Some("家族"))
        .await;

    let overlong_content = "あ".repeat(501);
    let overlong_category = "い".repeat(51);
    for (request, message, code) in [
        (
            json!({"subject": "assistant", "content": "猫を飼っている"}),
            "記憶の主語は user か character のどちらかです",
            "memory_subject_unknown",
        ),
        (
            json!({"subject": "user", "content": "\u{3000} \t"}),
            "記憶の本文が空です",
            "memory_content_empty",
        ),
        (
            json!({"subject": "user", "content": overlong_content.clone()}),
            "記憶の本文が長すぎます",
            "memory_content_too_long",
        ),
        (
            json!({"subject": "user", "content": "猫を飼っている", "category": "  "}),
            "記憶の分類が空です",
            "memory_category_empty",
        ),
        (
            json!({"subject": "user", "content": "猫を飼っている", "category": "好み\u{0007}"}),
            "記憶の分類に改行や制御文字は使えません",
            "memory_category_control",
        ),
        (
            json!({
                "subject": "user",
                "content": "猫を飼っている",
                "category": overlong_category.clone(),
            }),
            "記憶の分類が長すぎます",
            "memory_category_too_long",
        ),
    ] {
        let (status, body) = server
            .post(&corrections_path("aoi", target), &request)
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "400 であること: {body}");
        assert_eq!(body, error_body(message, code), "要求 {request}");
        assert_no_internals(&server, &body);
    }

    assert_eq!(server.memory_count().await, 1, "1行も書かれていないこと");

    server.stop().await;
}

/// **知らないキャラクターは、どちらの入口でも 404 `character_not_found`。**
///
/// **記憶の 404 と別のコード**で返る（直す場所が `handle` なのか id なのかが分かる）。
#[tokio::test]
async fn an_unknown_character_is_refused_before_the_memory_is_looked_up() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    let existing = server
        .add_memory(aoi, Subject::User, "猫を飼っている", Some("好み"))
        .await;

    let expected = error_body(
        "指定されたキャラクターが存在しません",
        "character_not_found",
    );

    let (status, body) = server.get(&memories_path("miya")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "一覧: {body}");
    assert_eq!(body, expected);

    // **葵の記憶の id を、居ないキャラクターのパスに貼る。**
    // ここで `memory_not_found` が返ると、「その id は存在する / しない」を
    // 知らないキャラクターのパス越しに問い合わせられることになる。
    let (status, body) = server
        .post(
            &corrections_path("miya", existing),
            &json!({"subject": "user", "content": "犬を飼っている", "category": "家族"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "訂正: {body}");
    assert_eq!(body, expected);

    assert_eq!(server.memory_count().await, 1, "1行も書かれていないこと");
    assert_eq!(
        server.superseded_by_of(existing).await,
        None,
        "葵の記憶に印が付いていないこと"
    );

    server.stop().await;
}

/// **記憶の id が数値でないパスは 400。** 404 に潰さない（直すのはパスの形）。
#[tokio::test]
async fn a_memory_id_that_is_not_a_number_is_a_bad_path() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server
        .post(
            &corrections_path("aoi", 0).replace("/0/", "/abc/"),
            &json!({"subject": "user", "content": "猫を飼っている"}),
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "400 であること: {body}");
    assert_eq!(body, error_body("パスの形式が不正です", "invalid_path"));
    assert_no_internals(&server, &body);

    server.stop().await;
}

/// **未知のキーは、本文が正当でも 400**（`deny_unknown_fields` / ADR-0022 §4）。
///
/// # `a_correction_without_content_is_refused_and_writes_nothing` では観測できない
///
/// あちらの5ケースは**どれも `content` が欠けている**ので、
/// `deny_unknown_fields` が無くても `missing field content` で同じ `invalid_body` になる。
/// **未知のキーの枝を通るには、残りが正当な本文でなければならない。**
/// 実際に `CorrectMemoryRequest` から `deny_unknown_fields` を外すと、
/// あちらは全緑のまま、**`{"locked": true}` 付きの要求が 201 になる。**
///
/// # 実害の向き —— **黙って捨てられるのが `locked`**
///
/// ADR-0022 §4 が「受けない」と決めた当のキーである。受理されると
/// **運用者は保護印を立てたつもりで立っていない**（訂正版は DB の既定値 0 で入る）。
/// 綴り違い（`contents`）も同じで、「設定したはずの値が効かない」形で後から表面化する。
///
/// # 本文が正当であることを、このテスト自身が確かめる
///
/// 最後に**同じ本文から未知のキーだけを外して送り、201 になる**ことを見る。
/// これが無いと、本文のどこか別の不備で 400 になっていても緑になる（空振りのテスト）。
#[tokio::test]
async fn an_unknown_key_in_an_otherwise_valid_correction_is_refused_and_writes_nothing() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    let target = server
        .add_memory(aoi, Subject::User, "黒い犬を飼っている", Some("家族"))
        .await;

    // **これだけなら 201 になる本文**（末尾で実際に確かめる）。
    let valid = json!({
        "subject": "user",
        "content": "白い犬を飼っている",
        "category": "好み",
    });

    for (key, value) in [
        // ADR-0022 §4 が「受けない」と決めた当のキー。
        ("locked", json!(true)),
        ("locked", json!(false)),
        // 「後継なしで無効化する」形（本文はあるが、消す指示を足した要求）。
        ("supersede", json!(true)),
        ("superseded", json!(true)),
        // 鎖を直接書き換えようとする形。
        ("superseded_by", json!(3)),
        // 主キーを本文からも指す形（パスと食い違いうる）。
        ("id", json!(7)),
        // 綴り違い（`content` と1文字違い）。
        ("contents", json!("茶色い犬を飼っている")),
    ] {
        let mut request = valid.clone();
        request[key] = value;

        let (status, body) = server
            .post(&corrections_path("aoi", target), &request)
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "未知のキー {key} を黙って捨てないこと: {request} → {body}"
        );
        assert_eq!(
            body,
            error_body("リクエスト本文の形式が不正です", "invalid_body"),
            "要求 {request}"
        );
        assert_no_internals(&server, &body);
    }

    assert_eq!(server.memory_count().await, 1, "1行も書かれていないこと");
    assert_eq!(
        server.superseded_by_of(target).await,
        None,
        "対象に印が付いていないこと"
    );

    // **未知のキーを外しただけの同じ本文は通る。** 上の 400 が本文の別の不備では
    // なく、未知のキーだけによるものであることを、このテスト自身が固定する。
    let (status, body) = server.post(&corrections_path("aoi", target), &valid).await;
    assert_eq!(status, StatusCode::CREATED, "正当な本文は通ること: {body}");
    assert_eq!(server.memory_count().await, 2, "訂正版が1行だけ増えること");
    assert_eq!(
        server.superseded_by_of(target).await,
        body["correction_id"].as_i64(),
        "対象が訂正版を指すこと"
    );

    server.stop().await;
}

/// **不正な本文を未知のキャラクターへ送ったら 400**（検証は DB に触れる前）。
///
/// # このテストだけがこの順序を観測できる
///
/// `an_unknown_character_is_refused_before_the_memory_is_looked_up` は
/// **正当な本文**を未知のキャラクターへ送るので、`find_character` が本文検証の
/// 前にあっても後にあっても 404 になる——**順序を観測できない。**
/// 「**不正な本文 × 未知のキャラクター**」の組み合わせだけが観測できる
/// （`tests/server_admin_attributes.rs::an_invalid_body_for_an_unknown_character_is_a_bad_request`
/// と同じ形。`replace_attributes` の doc がその条件を名指ししている）。
///
/// # なぜ 400 でなければならないか
///
/// 識別子を打ち間違えた運用者が `subject` も間違えていると、404 を先に返す実装では
/// **本文を直しても 404 が返り続ける**（直すべきものが2つあるのに、片方しか見えない）。
/// 宛先が正しくても本文は直さなければならないので、先に本文の理由を返す。
///
/// # この決定の境界 —— **すべてが 400 になるわけではない**
///
/// 記憶の3種の拒否（`memory_not_found` 等）は `store` が見るので、
/// 呼ぶには `character.id` が要る。**宛先の方が先に効いて 404** になる。
/// ここを併せて固定してあるので、**「常に 400 を返す」実装でもこのテストは緑にならない。**
#[tokio::test]
async fn an_invalid_body_for_an_unknown_character_is_a_bad_request() {
    let server = TestServer::start().await;
    // **存在するキャラクターを1件は置く**（表が空だと「常に 404」と区別が付かない）。
    let aoi = server.create_character("aoi", "葵").await;
    let existing = server
        .add_memory(aoi, Subject::User, "猫を飼っている", Some("好み"))
        .await;

    let overlong_content = "あ".repeat(501);
    let overlong_category = "い".repeat(51);
    for (request, message, code) in [
        (
            json!({"subject": "assistant", "content": "犬を飼っている"}),
            "記憶の主語は user か character のどちらかです",
            "memory_subject_unknown",
        ),
        (
            json!({"subject": "user", "content": "\u{3000} \t"}),
            "記憶の本文が空です",
            "memory_content_empty",
        ),
        (
            json!({"subject": "user", "content": overlong_content.clone()}),
            "記憶の本文が長すぎます",
            "memory_content_too_long",
        ),
        (
            json!({"subject": "user", "content": "犬を飼っている", "category": "  "}),
            "記憶の分類が空です",
            "memory_category_empty",
        ),
        (
            json!({
                "subject": "user",
                "content": "犬を飼っている",
                "category": overlong_category.clone(),
            }),
            "記憶の分類が長すぎます",
            "memory_category_too_long",
        ),
        // **本文の形式そのものが不正な場合も、宛先より先に返る。**
        (
            json!({"subject": "user", "content": "犬を飼っている", "locked": true}),
            "リクエスト本文の形式が不正です",
            "invalid_body",
        ),
        (
            json!({"subject": "user"}),
            "リクエスト本文の形式が不正です",
            "invalid_body",
        ),
    ] {
        let (status, body) = server
            .post(&corrections_path("miya", existing), &request)
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
        assert_no_internals(&server, &body);
    }

    // **同じ宛先へ正当な本文を送れば 404**（400 を返し続ける実装ではない）。
    let not_found = error_body(
        "指定されたキャラクターが存在しません",
        "character_not_found",
    );
    let (status, body) = server
        .post(
            &corrections_path("miya", existing),
            &json!({"subject": "user", "content": "犬を飼っている"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "正当な本文は 404: {body}");
    assert_eq!(body, not_found);

    // **記憶の拒否は `store` が見るので、宛先の方が先に効く。** 上の決定の境界。
    let (status, body) = server
        .post(
            &corrections_path("miya", existing + 9_999),
            &json!({"subject": "user", "content": "犬を飼っている"}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "記憶の存在は `store` が見るので 404: {body}"
    );
    assert_eq!(body, not_found);

    assert_eq!(server.memory_count().await, 1, "1行も書かれていないこと");
    assert_eq!(
        server.superseded_by_of(existing).await,
        None,
        "葵の記憶に印が付いていないこと"
    );

    server.stop().await;
}

/// **綴りの不正な識別子は、理由ごとに別のコードで 400**（`invalid_path` に潰さない）。
///
/// # 訂正のパスは `path_handle` を共有していない
///
/// 一覧は `path_handle`（他所のテストが見張っている）を通るが、訂正は
/// **`path_handle_and_memory_id` という別の関数**を通る。そちらの
/// `Handle::parse(&raw_handle)?` を `invalid_path` に潰す変異は、
/// 着手前は**全緑を通り抜けた**（訂正のパスに綴りの不正な識別子を送るテストが
/// 1本も無かった）。
///
/// # なぜ理由を分けるのか
///
/// **直す相手が違う。** `invalid_path` は「パスの形が不正」（記憶の id が数値でない等）で、
/// `character_handle_*` は「識別子の綴りが不正」。同じ 400 でも、
/// 前者を見た運用者は id を、後者を見た運用者は識別子を直す。
/// **理由を分けることがこの作業単位の中心の主張**なので、両方の入口で見る。
#[tokio::test]
async fn a_malformed_character_handle_keeps_its_own_reason_on_both_memory_paths() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let overlong_handle = "a".repeat(51);
    for (raw_handle, message, code) in [
        (
            "Aoi".to_owned(),
            "キャラクター識別子に使えない文字が含まれています",
            "character_handle_charset",
        ),
        (
            "aoi.v2".to_owned(),
            "キャラクター識別子に使えない文字が含まれています",
            "character_handle_charset",
        ),
        (
            "aoi@new".to_owned(),
            "キャラクター識別子に @ は使えません",
            "character_handle_at_sign",
        ),
        (
            overlong_handle.clone(),
            "キャラクター識別子が長すぎます",
            "character_handle_too_long",
        ),
    ] {
        let expected = error_body(message, code);

        // **訂正の入口**（`path_handle_and_memory_id`）。本文は正当なものを送る
        // ——本文で 400 になっていては、識別子の理由を観測できない。
        let (status, body) = server
            .post(
                &corrections_path(&raw_handle, 1),
                &json!({"subject": "user", "content": "猫を飼っている"}),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "訂正 {raw_handle}: {body}");
        assert_eq!(body, expected, "訂正 {raw_handle}");
        assert_no_internals(&server, &body);

        // **一覧の入口**（`path_handle`）。同じ理由が返る。
        let (status, body) = server.get(&memories_path(&raw_handle)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "一覧 {raw_handle}: {body}");
        assert_eq!(body, expected, "一覧 {raw_handle}");
        assert_no_internals(&server, &body);
    }

    // **`invalid_path` は別物。** 記憶の id が数値でないときだけこちらが返る
    // （`a_memory_id_that_is_not_a_number_is_a_bad_path` と同じ応答）。
    let (status, body) = server
        .post(
            &corrections_path("aoi", 0).replace("/0/", "/abc/"),
            &json!({"subject": "user", "content": "猫を飼っている"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "id の不正: {body}");
    assert_eq!(body, error_body("パスの形式が不正です", "invalid_path"));

    assert_eq!(server.memory_count().await, 0, "1行も書かれていないこと");

    server.stop().await;
}

// ---------------------------------------------------------------------------
// 上書きされた記憶を読む入口（ADR-0022 / 作業単位 D-3b）
//
// **経路は増やしていない。** 同じ `GET .../memories` に問い合わせの選択肢
// （`?include=superseded`）を足しただけで、**既定の応答は1バイトも変わらない。**
// 別の経路（`.../memories/history` 等）を生やすと、入口が2つになって
// 「どちらが正か」が分かれる。**1件の記憶を指すパスも増やしていない**
// （生やせば `PUT` と `DELETE` が自然な形になる /
// `tests/server_admin.rs::no_admin_endpoint_deletes_or_rewrites_a_memory`）。
// ---------------------------------------------------------------------------

/// 仕込む上書き済みの行の時刻。**行ごとに違う**（新しい順を時刻そのもので見る）。
const OLDEST: &str = "2020-01-01T00:00:00.000Z";
const MIDDLE: &str = "2020-01-01T00:00:01.000Z";
const NEWEST: &str = "2020-01-01T00:00:02.000Z";

/// 上書き済みも一緒に引くときのパス。
fn memories_path_with_superseded(handle: &str) -> String {
    format!("{}?include=superseded", memories_path(handle))
}

/// 応答の `superseded` の節を取り出す（**無ければ panic**）。
fn superseded_section(body: &Value) -> Value {
    body.get("superseded")
        .unwrap_or_else(|| panic!("superseded を載せること: {body}"))
        .clone()
}

/// 仕込んだ時刻の綴りを日時にする（期待値の側）。
fn instant(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .expect("テストで使う時刻は RFC3339 であること")
        .with_timezone(&Utc)
}

/// `superseded.memories` から `created_at` を**抜き取って**日時の列にする。
///
/// **抜くときに必ず見る**ので、`created_at` を落とす変異はここで panic して赤になる。
fn take_superseded_created_at(body: &mut Value) -> Vec<DateTime<Utc>> {
    let shown = body.to_string();
    let memories = body
        .get_mut("superseded")
        .unwrap_or_else(|| panic!("superseded を載せること: {shown}"))
        .get_mut("memories")
        .unwrap_or_else(|| panic!("superseded.memories を載せること: {shown}"))
        .as_array_mut()
        .expect("superseded.memories は配列であること");
    memories.iter_mut().map(take_created_at).collect()
}

/// **既定の応答は、上書きされた記憶について何も言わない。**
///
/// # この1本が「既定を変えていない」ことの権威
///
/// 訂正を**本番の入口で**通した後に、**問い合わせの選択肢を付けずに**引く。
/// 応答は 14b の形そのまま——**`superseded` のキーごと存在しない。**
///
/// キーを常に載せる（空の節で）形にしなかったのは、
/// **「訊いていない」と「訊いたが1件も無い」を受け取った側が区別できる**から
/// （訊いたのは自分である）。`unreadable` を空でも落とさないのとは事情が違う
/// ——あちらは**訊かなくても常に返る**節で、空とキーの不在が
/// 「壊れた行が無い」と「壊れた行を見せない古い版」の区別になる。
///
/// # 想起への影響もここで見る
///
/// 「常に上書き済みも返す」変異は、**この一覧だけでなく想起の絞り込みを外す形でも
/// 書ける。** 想起が上書き済みの行を返し始めると、
/// **「以前はXだった」が現在の事実としてプロンプトに載る**（ADR-0003）。
#[tokio::test]
async fn by_default_the_listing_says_nothing_about_superseded_memories() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;

    let untouched = server
        .add_memory(aoi, Subject::Character, "図書館で働いている", Some("経歴"))
        .await;
    let wrong = server
        .add_memory(aoi, Subject::User, "黒い犬を飼っている", Some("家族"))
        .await;
    let correction = server
        .correct("aoi", wrong, "user", "黒い猫を飼っている", Some("好み"))
        .await;

    let (status, mut body) = server.get(&memories_path("aoi")).await;
    assert_eq!(status, StatusCode::OK, "一覧が返ること: {body}");
    strip_created_at(&mut body);

    assert_eq!(
        body,
        json!({
            "character": "aoi",
            "memories": [
                {
                    "id": untouched,
                    "subject": "character",
                    "content": "図書館で働いている",
                    "category": "経歴",
                },
                {
                    "id": correction,
                    "subject": "user",
                    "content": "黒い猫を飼っている",
                    "category": "好み",
                },
            ],
            "unreadable": [],
        }),
        "既定の応答は 14b の形のままであること（上書き済みの節が生えない）"
    );
    assert!(
        body.get("superseded").is_none(),
        "訊いていないのに上書き済みの節を返さないこと: {body}"
    );

    // **想起も今までどおり。** 上書きされた行は載らない。
    assert_eq!(
        server.recalled_contents(aoi).await,
        vec!["図書館で働いている", "黒い猫を飼っている"],
        "想起に上書き済みの行が戻らないこと"
    );

    server.stop().await;
}

/// **訊けば、鎖のどの環も自分の後継を指して返る。有効な側は変わらない。**
///
/// `A → B → C` を**本番の入口で**作る。**B は上書きされていて、かつ誰かを上書きしている**
/// ——この状態が無いと、「後継は必ず最後に書かれた行」のような決め打ちが緑を通る。
///
/// # 仕込みが同値を作っていないこと
///
/// * 鎖の3件は `subject` も `content` も `category` も全部違う。
/// * **鎖の環の間に無関係な行を挟む**ので、後継は `id + 1` ではない。
/// * 有効な行が別に3件ある（訂正版と、触っていない行2件）。
#[tokio::test]
async fn asking_for_superseded_memories_returns_the_chain_with_each_link_pointing_at_its_successor()
{
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;

    let untouched = server
        .add_memory(aoi, Subject::Character, "図書館で働いている", Some("経歴"))
        .await;
    let first = server
        .add_memory(aoi, Subject::User, "犬を飼っている", Some("家族"))
        .await;
    // **無関係な行**（鎖の環の id を隣り合わせにしないため）。
    let tea = server
        .add_memory(aoi, Subject::Character, "紅茶が好き", None)
        .await;
    let second = server
        .correct("aoi", first, "user", "猫を飼っている", Some("好み"))
        .await;
    let books = server
        .add_memory(aoi, Subject::Character, "文庫本を集めている", Some("好み"))
        .await;
    let third = server
        .correct("aoi", second, "user", "黒い猫を飼っている", Some("しごと"))
        .await;

    assert_ne!(second, first + 1, "前提: 後継が id + 1 でないこと");
    assert_ne!(third, second + 1, "前提: 後継が id + 1 でないこと");

    let (status, mut body) = server.get(&memories_path_with_superseded("aoi")).await;
    assert_eq!(status, StatusCode::OK, "一覧が返ること: {body}");
    let _ = take_superseded_created_at(&mut body);
    strip_created_at(&mut body);

    assert_eq!(
        body,
        json!({
            "character": "aoi",
            "memories": [
                {
                    "id": untouched,
                    "subject": "character",
                    "content": "図書館で働いている",
                    "category": "経歴",
                },
                {
                    "id": tea,
                    "subject": "character",
                    "content": "紅茶が好き",
                    "category": null,
                },
                {
                    "id": books,
                    "subject": "character",
                    "content": "文庫本を集めている",
                    "category": "好み",
                },
                {
                    "id": third,
                    "subject": "user",
                    "content": "黒い猫を飼っている",
                    "category": "しごと",
                },
            ],
            "unreadable": [],
            "superseded": {
                "memories": [
                    {
                        "id": second,
                        "subject": "user",
                        "content": "猫を飼っている",
                        "category": "好み",
                        "superseded_by": third,
                    },
                    {
                        "id": first,
                        "subject": "user",
                        "content": "犬を飼っている",
                        "category": "家族",
                        "superseded_by": second,
                    },
                ],
                "unreadable": [],
                "total": 2,
                "limit": 100,
            },
        }),
        "鎖の環がそれぞれ自分の後継を指し、有効な側は変わらないこと"
    );

    server.stop().await;
}

/// **上書き済みの行も、自分の主語・分類・時刻を持ったまま返る。壊れた行は指し示される。**
///
/// # ここでしか作れない状態
///
/// * **`created_at` が行ごとに違う**（本番の書き込み口は現在時刻に任せるので、
///   仕込みでしか作れない）。**新しい順**であることを時刻そのもので見る。
/// * **上書き済みの壊れた行**——訂正して想起から外した後も残る。
///   読み飛ばすと**壊れた環だけが鎖から落ち、総数にだけ残る。**
/// * **別のキャラクターの上書き済みの行**（**こちらより新しい**ので、
///   絞り込みが消えたら先頭に出る）。
///
/// # 同値を作っていないこと
///
/// 3件の `subject` は `user` と `character` の両方、`category` は有り・無しの両方、
/// **後継は3件とも別の行**（「常に同じ行を指す」実装が赤になる）。
#[tokio::test]
async fn every_superseded_row_keeps_its_own_time_subject_category_and_successor() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    let rin = server.create_character("rin", "凛").await;

    // 後継になる有効な行を3件（**別々の行を指させる**ため）。
    let mut successors = Vec::new();
    for (subject, content) in [
        (Subject::User, "いまの記憶1"),
        (Subject::Character, "いまの記憶2"),
        (Subject::User, "いまの記憶3"),
    ] {
        successors.push(server.add_memory(aoi, subject, content, None).await);
    }

    let oldest = server
        .plant_superseded(
            aoi,
            "user",
            "いちばん古い記憶",
            Some("家族"),
            OLDEST,
            successors[2],
        )
        .await;
    let middle = server
        .plant_superseded(
            aoi,
            "character",
            "まんなかの記憶",
            None,
            MIDDLE,
            successors[0],
        )
        .await;
    // 本文が空白だけの行（長さ・空白の制約はスキーマに無いので素の INSERT で作れる）。
    let broken = server
        .plant_superseded(
            aoi,
            "user",
            "\u{3000} \t",
            Some("好み"),
            NEWEST,
            successors[1],
        )
        .await;

    // **別のキャラクターの上書き済みの行。こちらより新しい。**
    let rin_successor = server
        .add_memory(rin, Subject::Character, "凛のいまの記憶", None)
        .await;
    server
        .plant_superseded(
            rin,
            "character",
            "凛の古い記憶",
            Some("経歴"),
            "2020-01-02T00:00:00.000Z",
            rin_successor,
        )
        .await;

    let (status, mut body) = server.get(&memories_path_with_superseded("aoi")).await;
    assert_eq!(status, StatusCode::OK, "一覧が返ること: {body}");

    let times = take_superseded_created_at(&mut body);
    assert_eq!(
        times,
        vec![instant(MIDDLE), instant(OLDEST)],
        "時刻がその行のものであること（新しい順。壊れた行は時刻を返さない）"
    );

    assert_eq!(
        superseded_section(&body),
        json!({
            "memories": [
                {
                    "id": middle,
                    "subject": "character",
                    "content": "まんなかの記憶",
                    "category": null,
                    "superseded_by": successors[0],
                },
                {
                    "id": oldest,
                    "subject": "user",
                    "content": "いちばん古い記憶",
                    "category": "家族",
                    "superseded_by": successors[2],
                },
            ],
            "unreadable": [
                {"id": broken, "invalid_columns": ["content"], "superseded_by": successors[1]},
            ],
            "total": 3,
            "limit": 100,
        }),
        "主語・分類・後継が行ごとに正しく、壊れた行が指し示されること"
    );

    // **読めなかった行の格納値は載せない**（`UnreadableMemory` は持っていない）。
    assert!(
        !body.to_string().contains("好み"),
        "読めなかった行の格納値を載せないこと: {body}"
    );

    // **相手側から見ても混ざらない。**
    let (_, rins) = server.get(&memories_path_with_superseded("rin")).await;
    let rin_section = superseded_section(&rins);
    assert_eq!(
        rin_section.get("total"),
        Some(&json!(1)),
        "凛の総数は凛の行だけであること: {rins}"
    );
    let rin_ids: Vec<i64> = rin_section
        .get("memories")
        .and_then(Value::as_array)
        .expect("superseded.memories は配列であること")
        .iter()
        .filter_map(|memory| memory.get("id").and_then(Value::as_i64))
        .collect();
    assert!(
        !rin_ids.contains(&oldest) && !rin_ids.contains(&middle),
        "葵の行が凛の一覧に混ざらないこと: {rin_ids:?}"
    );

    server.stop().await;
}

/// **上限まで返し、総数は全部を数える。**
///
/// # 期待値を `RECENT_SUPERSEDED_MEMORIES` から作らない（CLAUDE.md `16b`）
///
/// 定数から期待値を組むと、**その定数を倍にする変異が緑を通る。**
/// ここは**外の値**（直書きの 100 と 113）で押さえる。
///
/// # ⚠ **この1本が4つの主張を持っている**（分割・簡略化するときの注意）
///
/// 1. **総数**が「返した件数」ではないこと（`total == 113`）。
/// 2. **上限**が効いていること（返る行が 100、`limit == 100`）。
/// 3. **上限は行に効く**こと——窓の中に壊れた行が3件あり、
///    読めた 97 件と合わせて 100。「読めなかった行を上限から除く」実装は赤になる。
/// 4. **新しい順**であること（先頭が最後に入れた行、末尾が窓の下端）。
///
/// **4つとも、行が上限を超えている状態でしか観測できない。**
/// 上限以下では総数と件数が同値になり、どちらへ倒す実装も緑になる
/// ——**この状態を作るテストはここ1本だけ**である。
/// **分けるなら、分けた先それぞれで上限超えを積むこと。**
///
/// # ⚠ **`total` が上限で切られていないことの end-to-end の権威は、この1本だけ**
///
/// 単体の `src/server/admin.rs::the_total_is_not_the_number_of_rows_that_came_back` は
/// **組み立て関数（`superseded_list_view`）を直接呼ぶ**ので、
/// **呼び出し側を変える変異には届かない。**
/// 実測（D-3b の変異検査 / 2026-08-27）——**総数を「返した件数」にすり替える変異**は、
/// 置いた場所で赤くなる相手が変わった:
///
/// | すり替えた場所 | 赤 |
/// |---|---|
/// | ハンドラ（`superseded_list_view` へ渡す値） | **このテストだけ**（単体は緑） |
/// | 組み立て関数の中 | このテスト ＋ 単体 |
///
/// **「単体があるから守られている」と読まないこと。**
/// 上のどちらの場所へ変異を置いても赤になる見張り番は、ここ以外に無い。
#[tokio::test]
async fn only_the_most_recent_superseded_memories_come_back_but_the_total_counts_them_all() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    let successor = server
        .add_memory(aoi, Subject::User, "いまの記憶", None)
        .await;

    // **窓の中に入る位置**に壊れた行を置く（新しい方から 100 件の中）。
    let broken_at = [60, 105, 110];
    let mut planted = Vec::new();
    for index in 0..113 {
        let content = if broken_at.contains(&index) {
            "\u{3000} \t".to_owned()
        } else {
            format!("古い記憶{index}")
        };
        planted.push(
            server
                .plant_superseded(
                    aoi,
                    "user",
                    &content,
                    None,
                    &format!("2020-01-01T00:{:02}:{:02}.000Z", index / 60, index % 60),
                    successor,
                )
                .await,
        );
    }

    let (status, mut body) = server.get(&memories_path_with_superseded("aoi")).await;
    assert_eq!(status, StatusCode::OK, "一覧が返ること: {body}");
    let times = take_superseded_created_at(&mut body);
    let section = superseded_section(&body);

    let readable = section
        .get("memories")
        .and_then(Value::as_array)
        .expect("superseded.memories は配列であること");
    let unreadable = section
        .get("unreadable")
        .and_then(Value::as_array)
        .expect("superseded.unreadable は配列であること");
    assert_eq!(
        readable.len() + unreadable.len(),
        100,
        "上限は行に効くこと（読めた {} 件 + 読めなかった {} 件）",
        readable.len(),
        unreadable.len()
    );
    assert_eq!(readable.len(), 97, "読めた行は 97 件であること");
    assert_eq!(
        unreadable
            .iter()
            .filter_map(|broken| broken.get("id").and_then(Value::as_i64))
            .collect::<Vec<i64>>(),
        vec![planted[110], planted[105], planted[60]],
        "窓の中の壊れた行が、新しい順に指し示されること"
    );
    assert_eq!(
        section.get("limit"),
        Some(&json!(100)),
        "上限を名乗ること（画面が『これで全部』と言えないため）: {section}"
    );
    assert_eq!(
        section.get("total"),
        Some(&json!(113)),
        "総数は返した件数ではなく、保存されている全部であること: {section}"
    );

    assert_eq!(
        readable.first().and_then(|memory| memory.get("id")),
        Some(&json!(planted[112])),
        "いちばん新しい行が先頭であること: {section}"
    );
    assert_eq!(
        readable.last().and_then(|memory| memory.get("id")),
        Some(&json!(planted[13])),
        "窓の下端が 113 件目から数えて 100 件目であること: {section}"
    );
    assert_eq!(
        times.first().copied(),
        Some(instant("2020-01-01T00:01:52.000Z")),
        "先頭の時刻がいちばん新しい行のものであること: {times:?}"
    );

    server.stop().await;
}

/// **1件も上書きされていなくても、訊けば節は返る**（空の配列と 0 で）。
///
/// 「訊いたが1件も無い」と「訊いていない」を、受け取った側が区別できること。
/// **上限は件数に関係なく名乗る**（`limit` を「切ったときだけ」載せる形にしない）。
#[tokio::test]
async fn a_character_without_superseded_memories_still_gets_the_section_when_asked() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    server
        .add_memory(aoi, Subject::User, "上書きされていない記憶", Some("好み"))
        .await;

    let (status, body) = server.get(&memories_path_with_superseded("aoi")).await;
    assert_eq!(status, StatusCode::OK, "一覧が返ること: {body}");
    assert_eq!(
        superseded_section(&body),
        json!({"memories": [], "unreadable": [], "total": 0, "limit": 100}),
        "空でも節ごと返し、上限を名乗ること"
    );

    server.stop().await;
}

/// **知らない問い合わせは 400。宛先より先に返る。**
///
/// # 黙って無視しない
///
/// `?includes=superseded`（綴り違い）を無視すると、
/// **訊いたつもりの側は「上書きされた記憶は1件も無い」と読む。**
/// 節ごと返らないので、空との区別も付かない——**画面が嘘をつく**形そのもの。
///
/// # 検証は DB に触れる前
///
/// `replace_attributes` / `correct_memory` と同じ順序。
/// **正当な問い合わせを未知のキャラクターへ送るテストではこの順序を観測できない**
/// （どちらの順でも 404 になる）。**「不正な問い合わせ × 未知のキャラクター」が要る**
/// ——14b が同じ穴に落ちた（`an_invalid_body_for_an_unknown_character_is_a_bad_request`）。
#[tokio::test]
async fn an_unknown_query_parameter_is_a_bad_request_even_for_an_unknown_character() {
    let server = TestServer::start().await;
    // **存在するキャラクターを1件は置く**（表が空だと「常に 400」と区別が付かない）。
    server.create_character("aoi", "葵").await;

    for query in [
        // 値が知らない綴り。
        "?include=everything",
        "?include=all",
        "?include=",
        // 欄の綴り違い。**無視すると「1件も無い」と読まれる。**
        "?includes=superseded",
        "?with=superseded",
        // 正しい欄に、知らない欄を足した形。
        "?include=superseded&limit=1000",
        // **同じ欄を2回。** 一覧に入れる前に測った（D-3b のレビューで指摘 / 2026-08-28）
        // ——`serde` の派生が「重複した欄」として落とすので、
        // **黙って後勝ちにはならない。** 実測で 400 / `invalid_query`。
        "?include=superseded&include=superseded",
    ] {
        for handle in ["aoi", "miya"] {
            let path = format!("{}{query}", memories_path(handle));
            let (status, body) = server.get(&path).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{path} は 400 であること: {body}"
            );
            assert_eq!(
                body,
                error_body("問い合わせパラメータの形式が不正です", "invalid_query"),
                "理由をそのまま返すこと（404 に潰さない / 入力を反射しない）: {path}"
            );
            assert_no_internals(&server, &body);
        }
    }

    // **正当な問い合わせなら、未知のキャラクターは 404 のまま。**
    let (status, body) = server.get(&memories_path_with_superseded("miya")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "404 であること: {body}");
    assert_eq!(
        body,
        error_body(
            "指定されたキャラクターが存在しません",
            "character_not_found"
        ),
        "宛先の理由が返ること"
    );

    server.stop().await;
}

/// **問い合わせの選択肢は、生えているメソッドを1本も増やさない。**
///
/// `no_admin_endpoint_deletes_or_rewrites_a_memory` が塞いでいるものを、
/// **選択肢を付けた形でも**確かめる——`?include=superseded` を付ければ
/// `DELETE .../memories` が通る、というような形が生えていないこと。
#[tokio::test]
async fn the_query_option_does_not_open_any_other_method() {
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
                &memories_path_with_superseded("aoi"),
                Some(&json!({})),
            )
            .await;
        assert_eq!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} が生えていないこと: {body}"
        );
        assert_eq!(body, method_not_allowed, "{method}");
    }

    // **1件を指すパスは、選択肢を付けても 404 のまま。**
    let (status, body) = server
        .get("/admin/characters/aoi/memories/1?include=superseded")
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "1件を指すパスが生えていないこと: {body}"
    );

    server.stop().await;
}
