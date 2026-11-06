//! `/admin/characters/{handle}/sessions`（会話履歴を読む口 / 作業単位 D-12c-1 / Issue origin#35）の検証。
//!
//! **LM Studio には繋がない。** 上流が要るのは「`aoi@<id>` で古い会話に書く」1本だけで、
//! そこは `LlmClient` のモックを注入する（[`MockLlm`]）。
//! **それ以外は本番の入口（`App::bind`）から起動する**——テスト専用の入口だけを通すと、
//! 本番の配線が誰にも通られないまま残る（CLAUDE.md「テスト専用の入口を足したら…」）。
//!
//! ここで固定するのは主に次の点:
//!
//!   * **一覧の順は、素の `aoi` が続ける順と同じ**（`Store::latest_session`）。
//!     **`aoi@<id>` で古い会話に書いた直後に、その会話が先頭へ来る。**
//!     **空のセッションは、`updated_at` が新しくても後ろへ回る**（B-2b の降格と同じ）。
//!   * **遡りに重複も欠けも無い**——3ページ以上に割れる会話で、**真ん中のページ**を見る。
//!   * **上限と総数**——返した件数と総数を取り違えない（期待値は直書き / CLAUDE.md `16b`）。
//!   * **他のキャラクターの会話は id で覗けない**（404）。存在しない id も 404。
//!   * **`before` / `limit` は DB に触れる前に検証する**（0・負・巨大・数でない・知らない欄 → 400）。
//!   * **読むだけ**——書く口も、1件を指すパスも無い。
//!   * エラーレスポンスに**内部情報を載せない**（本文を完全一致で固定する）。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use personacore::domain::{Handle, NewMessage, Role, SessionId};
use personacore::llm::{
    ChatRequest, ChatStream, Completion, FinishReason, LlmClient, LlmError, LlmFuture,
};
use personacore::server::{App, AppConfig, BindAddr, ServerError};
use personacore::store::Store;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// 上流の代役。**「`aoi@<id>` で書く」1本のためだけに在る。**
///
/// 返す本文は呼ばれるたびに変える（同じ本文だと、どの回の応答が保存されたかを区別できない）。
struct MockLlm {
    replies: Mutex<Vec<String>>,
}

impl MockLlm {
    fn replying(replies: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.iter().rev().map(|r| (*r).to_owned()).collect()),
        })
    }
}

impl LlmClient for MockLlm {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        Box::pin(async move {
            let content = self
                .replies
                .lock()
                .expect("ロックを取れること")
                .pop()
                .unwrap_or_else(|| "「ほかに言うことはない」".to_owned());
            Ok(Completion {
                content,
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        // この測定は中継を使わない。呼ばれたこと自体が誤り。
        Box::pin(async move { Err(LlmError::MissingContent) })
    }
}

/// エフェメラルポートで起動した実サーバ。
struct TestServer {
    addr: SocketAddr,
    /// アプリと**同じプールを指す**ハンドル。会話の仕込みに使う。
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
        Self::start_with(None).await
    }

    /// **上流をモックに差し替えて起動する**（`aoi@<id>` で書く1本だけが使う）。
    async fn start_with_llm(llm: Arc<MockLlm>) -> Self {
        Self::start_with(Some(llm)).await
    }

    async fn start_with(llm: Option<Arc<MockLlm>>) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db_path = dir.path().join("personacore.sqlite");

        let config = AppConfig::from_lookup(|_| None)
            .expect("既定値だけで設定を組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(&db_path);

        let app = match llm {
            None => App::bind(&config).await,
            Some(llm) => App::bind_with_client(&config, llm).await,
        }
        .expect("待ち受けを開始できること");
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

    /// キャラクターを1件作る。**識別子と表示名は違う文字列。**
    async fn create_character(&self, handle: &str, display_name: &str) {
        assert_ne!(handle, display_name, "識別子と表示名を同値にしないこと");
        let (status, body) = self
            .send(
                Method::POST,
                "/admin/characters",
                Some(&json!({
                    "handle": handle,
                    "display_name": display_name,
                    "base_persona": "ペルソナ本文",
                })),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "作成できること: {body}");
    }

    /// 空のセッションを1つ作る（`aoi@new` の直後と同じ状態）。
    async fn new_session(&self, handle: &str) -> SessionId {
        let character = self
            .store
            .find_character_by_handle(&Handle::parse(handle).expect("正当な識別子"))
            .await
            .expect("キャラクターを引けること")
            .expect("キャラクターが存在すること");
        self.store
            .create_session(character.id)
            .await
            .expect("セッションを作れること")
    }

    /// 往復を1つ書く（**本文処理が書くのと同じ `append_turn`**）。
    async fn turn(&self, session: SessionId, user: &str, reply: &str) {
        self.store
            .append_turn(
                session,
                &NewMessage {
                    role: Role::User,
                    dialogue: None,
                    narration: None,
                    raw_content: user.to_owned(),
                },
                &NewMessage {
                    role: Role::Assistant,
                    dialogue: Some(format!("台詞:{reply}")),
                    narration: Some(format!("地の文:{reply}")),
                    raw_content: reply.to_owned(),
                },
            )
            .await
            .expect("往復を書けること");
    }

    /// **素の `aoi` が続けるセッション**（`Store::latest_session`）の id。
    async fn latest_session_id(&self, handle: &str) -> i64 {
        let character = self
            .store
            .find_character_by_handle(&Handle::parse(handle).expect("正当な識別子"))
            .await
            .expect("キャラクターを引けること")
            .expect("キャラクターが存在すること");
        self.store
            .latest_session(character.id)
            .await
            .expect("セッションを引けること")
            .expect("セッションが存在すること")
            .id
            .get()
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

/// 一覧の `id` を並びのまま。
fn session_ids(body: &Value) -> Vec<i64> {
    body["sessions"]
        .as_array()
        .unwrap_or_else(|| panic!("sessions が配列であること: {body}"))
        .iter()
        .map(|session| session["id"].as_i64().expect("id が数であること"))
        .collect()
}

/// ページの `id` を並びのまま。
fn message_ids(body: &Value) -> Vec<i64> {
    body["messages"]
        .as_array()
        .unwrap_or_else(|| panic!("messages が配列であること: {body}"))
        .iter()
        .map(|message| message["id"].as_i64().expect("id が数であること"))
        .collect()
}

/// ページの生出力を並びのまま。
fn raw_contents(body: &Value) -> Vec<String> {
    body["messages"]
        .as_array()
        .unwrap_or_else(|| panic!("messages が配列であること: {body}"))
        .iter()
        .map(|message| {
            message["raw_content"]
                .as_str()
                .expect("raw_content が文字列であること")
                .to_owned()
        })
        .collect()
}

fn messages_path(handle: &str, session: i64, query: &str) -> String {
    format!("/admin/characters/{handle}/sessions/{session}/messages{query}")
}

/// **一覧の順は、素の `aoi` が続ける順と同じ。**
///
/// # 3つの状態を1本で通る
///
/// 1. **書いた順**（a → b → c）。先頭は c。
/// 2. **`aoi@new` の直後**（空のセッション d）。**`updated_at` は d がいちばん新しい**が、
///    素の `aoi` は c を続ける（B-2b）。**一覧も d を末尾へ回す。**
///    ここで `updated_at` だけで並べる実装は d を先頭に出し、
///    **画面が「続ける会話」として空の会話を出す**（本文は c へ行く）。
/// 3. **`aoi@<a>` で最も古い a に書いた直後**。**a が先頭へ来る**（ADR-0004 の `updated_at` 順）。
///    **`created_at` / `id` で並べる実装はここで落ちる。**
///
/// どの状態でも、**先頭が `Store::latest_session` と一致する**ことを見る
/// （帰属は最終的な出力で突き合わせる / CLAUDE.md）。
#[tokio::test]
async fn the_session_list_follows_the_order_the_bare_handle_continues() {
    let server = TestServer::start_with_llm(MockLlm::replying(&["「古い話の続きだね」"])).await;
    server.create_character("aoi", "葵").await;

    let a = server.new_session("aoi").await;
    server.turn(a, "一つ目の会話", "「一つ目だね」").await;
    let b = server.new_session("aoi").await;
    server.turn(b, "二つ目の会話", "「二つ目だね」").await;
    let c = server.new_session("aoi").await;
    server.turn(c, "三つ目の会話", "「三つ目だね」").await;

    // 1. 書いた順。
    let (status, body) = server.get("/admin/characters/aoi/sessions").await;
    assert_eq!(status, StatusCode::OK, "一覧を読めること: {body}");
    assert_eq!(session_ids(&body), vec![c.get(), b.get(), a.get()]);
    assert_eq!(server.latest_session_id("aoi").await, c.get());

    // 2. 空のセッションは後ろへ回る（`updated_at` はいちばん新しい）。
    let d = server.new_session("aoi").await;
    let (_, body) = server.get("/admin/characters/aoi/sessions").await;
    assert_eq!(
        session_ids(&body),
        vec![c.get(), b.get(), a.get(), d.get()],
        "空のセッションは、素の aoi と同じく後ろへ回ること: {body}"
    );
    assert_eq!(server.latest_session_id("aoi").await, c.get());

    // 3. **`aoi@<a>` で古い会話に書く**（本文処理を実際に通す）。
    let response = server
        .client
        .post(format!("http://{}/v1/chat/completions", server.addr))
        .json(&json!({
            "model": format!("aoi@{}", a.get()),
            "messages": [{"role": "user", "content": "古い話に戻ろう"}],
        }))
        .send()
        .await
        .expect("サーバへ接続できること");
    assert_eq!(response.status(), StatusCode::OK);

    let (_, body) = server.get("/admin/characters/aoi/sessions").await;
    let ids = session_ids(&body);
    assert_eq!(
        ids,
        vec![a.get(), c.get(), b.get(), d.get()],
        "書いた会話が先頭へ来ること: {body}"
    );
    assert_eq!(
        server.latest_session_id("aoi").await,
        ids[0],
        "一覧の先頭が、素の aoi が続けるセッションであること"
    );

    server.stop().await;
}

/// **1行の中身。** 件数・最初の利用者の発言・日時を、**その会話のもの**で返す。
///
/// 2つの会話の件数と最初の発言を**違えてある**（同じだと取り違えが見えない）。
/// **最初の発言は「利用者の」発言。** 本文処理は必ず利用者 → 応答の順に書くので、
/// 役割の条件を落とす変異は**この形では観測できない**（最初の行が常に利用者）。
/// そちらは `the_opening_is_the_first_user_message_even_after_a_reply` が見る。
#[tokio::test]
async fn each_row_describes_its_own_session() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let short = server.new_session("aoi").await;
    server.turn(short, "短い", "「うん」").await;
    let long = server.new_session("aoi").await;
    // **80 文字を超える最初の発言**（ちょうど 81 文字）。
    let opening: String = "あ".repeat(80) + "い";
    server.turn(long, &opening, "「長いね」").await;
    server.turn(long, "二回目", "「まだ続くの」").await;

    let (status, body) = server.get("/admin/characters/aoi/sessions").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["character"], json!("aoi"));
    assert_eq!(body["total_sessions"], json!(2));
    assert_eq!(body["sessions_limit"], json!(100));

    let rows = body["sessions"].as_array().expect("配列であること");
    let newest = &rows[0];
    assert_eq!(newest["id"], json!(long.get()));
    assert_eq!(newest["message_count"], json!(4));
    assert_eq!(newest["label"], Value::Null);
    assert_eq!(
        newest["first_user_message"],
        json!("あ".repeat(80)),
        "先頭 80 文字で切ること: {newest}"
    );
    assert_eq!(newest["first_user_message_truncated"], json!(true));

    let older = &rows[1];
    assert_eq!(older["id"], json!(short.get()));
    assert_eq!(older["message_count"], json!(2));
    assert_eq!(older["first_user_message"], json!("短い"));
    assert_eq!(older["first_user_message_truncated"], json!(false));

    // **日時は DB のものをそのまま**（新しい方が大きい）。
    for row in rows {
        for key in ["created_at", "updated_at"] {
            assert!(row[key].is_string(), "{key} が文字列であること: {row}");
        }
    }
    assert!(
        newest["updated_at"].as_str() > older["updated_at"].as_str(),
        "updated_at は新しい方が大きいこと: {body}"
    );

    server.stop().await;
}

/// **最初の発言は「利用者の」発言**（応答が先に入っている会話でも）。
///
/// 本文処理は利用者 → 応答の順に書くので、この並びは `append_message` で作る
/// （応答だけが書かれた行は、DB を直接いじった状態に相当する）。
/// **役割の条件を落とすと、ここで応答の本文が出る。**
#[tokio::test]
async fn the_opening_is_the_first_user_message_even_after_a_reply() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let session = server.new_session("aoi").await;
    server
        .store
        .append_message(
            session,
            &NewMessage {
                role: Role::Assistant,
                dialogue: None,
                narration: None,
                raw_content: "先に話しかけた".to_owned(),
            },
        )
        .await
        .expect("書けること");
    server.turn(session, "あとから答えた", "「そうだね」").await;

    let (_, body) = server.get("/admin/characters/aoi/sessions").await;
    let row = &body["sessions"][0];
    assert_eq!(row["first_user_message"], json!("あとから答えた"), "{row}");
    assert_eq!(row["message_count"], json!(3));

    server.stop().await;
}

/// **ちょうど 80 文字は切らない**（境目の内側）。
#[tokio::test]
async fn an_opening_of_exactly_the_limit_is_not_truncated() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let session = server.new_session("aoi").await;
    let opening = "か".repeat(80);
    server.turn(session, &opening, "「ちょうどだね」").await;

    let (_, body) = server.get("/admin/characters/aoi/sessions").await;
    let row = &body["sessions"][0];
    assert_eq!(row["first_user_message"], json!(opening));
    assert_eq!(row["first_user_message_truncated"], json!(false));

    server.stop().await;
}

/// **空のセッション（`@new` の直後）。** 一覧には出る（件数 0・最初の発言 `null`）、
/// メッセージは空のページ。**404 にしない**（`aoi@<id>` はそこへ届く / ADR-0004 の追記）。
#[tokio::test]
async fn an_empty_session_is_listed_and_reads_as_an_empty_page() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let empty = server.new_session("aoi").await;

    let (status, body) = server.get("/admin/characters/aoi/sessions").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(session_ids(&body), vec![empty.get()]);
    let row = &body["sessions"][0];
    assert_eq!(row["message_count"], json!(0));
    assert_eq!(row["first_user_message"], Value::Null);
    assert_eq!(row["first_user_message_truncated"], json!(false));

    let (status, body) = server.get(&messages_path("aoi", empty.get(), "")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["messages"], json!([]));
    assert_eq!(body["total_messages"], json!(0));
    assert_eq!(body["has_older"], json!(false));
    assert_eq!(body["session_id"], json!(empty.get()));

    server.stop().await;
}

/// **会話が1つも無いキャラクターは空の一覧**（404 にしない）。
#[tokio::test]
async fn a_character_without_sessions_has_an_empty_list() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server.get("/admin/characters/aoi/sessions").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({
            "character": "aoi",
            "sessions": [],
            "total_sessions": 0,
            "sessions_limit": 100,
        })
    );

    server.stop().await;
}

/// **遡りに重複も欠けも無い**（3ページに割れる会話 / **真ん中のページを見る**）。
///
/// 7往復 = 14件を `limit=5` で読む ——
/// **最新のページ 5件 → 真ん中 5件 → 最後 4件。**
///
/// 真ん中のページは両端に境目を持つ唯一のページで、
/// **`<` を `<=` にする変異（境目の1件が2回出る）と、
/// 切る向きを古い側へ倒す変異**はここで見える。
/// **ページの中は古い順**（画面は上から古い順に積む）。
#[tokio::test]
async fn paging_backwards_visits_every_message_exactly_once() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let session = server.new_session("aoi").await;
    for index in 1..=7 {
        server
            .turn(
                session,
                &format!("発言{index}"),
                &format!("「応答{index}」"),
            )
            .await;
    }

    // 最新のページ。
    let (status, newest) = server
        .get(&messages_path("aoi", session.get(), "?limit=5"))
        .await;
    assert_eq!(status, StatusCode::OK, "{newest}");
    assert_eq!(
        raw_contents(&newest),
        ["「応答5」", "発言6", "「応答6」", "発言7", "「応答7」"].map(str::to_owned),
        "最新のページは新しい側の 5 件を古い順に: {newest}"
    );
    assert_eq!(newest["has_older"], json!(true));
    assert_eq!(newest["total_messages"], json!(14));
    assert_eq!(newest["limit"], json!(5));

    // **真ん中のページ**（最新のページの先頭より前）。
    let before = message_ids(&newest)[0];
    let (status, middle) = server
        .get(&messages_path(
            "aoi",
            session.get(),
            &format!("?before={before}&limit=5"),
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{middle}");
    assert_eq!(
        raw_contents(&middle),
        ["発言3", "「応答3」", "発言4", "「応答4」", "発言5"].map(str::to_owned),
        "真ん中のページは、境目の両側と重ならないこと: {middle}"
    );
    assert_eq!(middle["has_older"], json!(true));
    assert_eq!(middle["total_messages"], json!(14));

    // 最後のページ。
    let before = message_ids(&middle)[0];
    let (status, oldest) = server
        .get(&messages_path(
            "aoi",
            session.get(),
            &format!("?before={before}&limit=5"),
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{oldest}");
    assert_eq!(
        raw_contents(&oldest),
        ["発言1", "「応答1」", "発言2", "「応答2」"].map(str::to_owned),
    );
    assert_eq!(oldest["has_older"], json!(false));

    // **3ページを繋ぐと、全件が1回ずつ、古い順に並ぶ。**
    let mut all = message_ids(&oldest);
    all.extend(message_ids(&middle));
    all.extend(message_ids(&newest));
    let mut sorted = all.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(all.len(), 14, "欠けが無いこと: {all:?}");
    assert_eq!(all, sorted, "重複が無く、古い順であること: {all:?}");

    // **`has_older` の境目**（1件多く頼む仕掛けの両側）。真ん中のページより前には 4 件ある。
    // 上の3ページは 6 / 6 / 4 件を引くので、**ちょうど上限ぶんが残る形を一度も作らない**
    // ——ここで作る（`>` を `>=` にする変異は、ここでしか見えない）。
    let before = message_ids(&middle)[0];
    let (_, exact) = server
        .get(&messages_path(
            "aoi",
            session.get(),
            &format!("?before={before}&limit=4"),
        ))
        .await;
    assert_eq!(message_ids(&exact).len(), 4, "{exact}");
    assert_eq!(
        exact["has_older"],
        json!(false),
        "ちょうど上限ぶんしか残っていなければ、もう前は無い: {exact}"
    );
    let (_, one_short) = server
        .get(&messages_path(
            "aoi",
            session.get(),
            &format!("?before={before}&limit=3"),
        ))
        .await;
    assert_eq!(
        raw_contents(&one_short),
        ["「応答1」", "発言2", "「応答2」"].map(str::to_owned),
    );
    assert_eq!(
        one_short["has_older"],
        json!(true),
        "1件でも残っていれば、まだ前がある: {one_short}"
    );

    server.stop().await;
}

/// **1行の中身**——役割・台詞・地の文・生出力・日時を、そのまま返す。
///
/// **`raw_content` を落とさない**（絶対制約）。台詞と地の文は保存された列そのもの
/// （画面は生出力を整え直す。ここは加工しない）。
#[tokio::test]
async fn a_message_row_keeps_every_stored_column() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let session = server.new_session("aoi").await;
    server
        .turn(session, "こんにちは", "「**やあ**」と笑った。")
        .await;

    let (status, body) = server.get(&messages_path("aoi", session.get(), "")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["character"], json!("aoi"));
    let rows = body["messages"].as_array().expect("配列であること");
    assert_eq!(rows.len(), 2);

    let user = &rows[0];
    assert_eq!(user["role"], json!("user"));
    assert_eq!(user["raw_content"], json!("こんにちは"));
    assert_eq!(user["dialogue"], Value::Null);
    assert_eq!(user["narration"], Value::Null);

    let reply = &rows[1];
    assert_eq!(reply["role"], json!("assistant"));
    assert_eq!(reply["raw_content"], json!("「**やあ**」と笑った。"));
    assert_eq!(reply["dialogue"], json!("台詞:「**やあ**」と笑った。"));
    assert_eq!(reply["narration"], json!("地の文:「**やあ**」と笑った。"));
    assert!(reply["created_at"].is_string(), "{reply}");
    assert!(
        reply["id"].as_i64() > user["id"].as_i64(),
        "id は書いた順に増えること"
    );

    server.stop().await;
}

/// **既定のページは 50 件、上限は 200 件**（期待値は直書き / CLAUDE.md `16b`）。
///
/// **上限を超える件数を積まないと、既定と上限は観測できない**（切られていなければ
/// 何を名乗っても件数と食い違わない）。**206 件**（103 往復）を積む。
#[tokio::test]
async fn the_default_page_and_the_largest_page_are_bounded() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let session = server.new_session("aoi").await;
    for index in 1..=103 {
        server
            .turn(
                session,
                &format!("発言{index}"),
                &format!("「応答{index}」"),
            )
            .await;
    }

    let (status, body) = server.get(&messages_path("aoi", session.get(), "")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(message_ids(&body).len(), 50, "既定は 50 件であること");
    assert_eq!(body["limit"], json!(50));
    assert_eq!(body["total_messages"], json!(206));
    assert_eq!(body["has_older"], json!(true));
    assert_eq!(
        raw_contents(&body).last().map(String::as_str),
        Some("「応答103」"),
        "最新が末尾であること"
    );

    let (status, body) = server
        .get(&messages_path("aoi", session.get(), "?limit=200"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(message_ids(&body).len(), 200);
    assert_eq!(body["limit"], json!(200));
    assert_eq!(body["has_older"], json!(true));

    let (status, body) = server
        .get(&messages_path("aoi", session.get(), "?limit=201"))
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "上限を超える limit は 400: {body}"
    );

    server.stop().await;
}

/// **一覧は直近 100 件。総数は全部を数える**（期待値は直書き）。
///
/// 切られるのは**古い側**（先頭は最新、末尾は 4 番目に古いもの）。
#[tokio::test]
async fn only_the_newest_sessions_come_back_but_the_total_counts_them_all() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let mut sessions = Vec::new();
    for index in 1..=103 {
        let session = server.new_session("aoi").await;
        server
            .turn(session, &format!("会話{index}"), "「うん」")
            .await;
        sessions.push(session.get());
    }

    let (status, body) = server.get("/admin/characters/aoi/sessions").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids = session_ids(&body);
    assert_eq!(ids.len(), 100);
    assert_eq!(body["sessions_limit"], json!(100));
    assert_eq!(body["total_sessions"], json!(103));
    assert_eq!(ids.first(), sessions.last(), "先頭は最新であること");
    assert_eq!(ids.last(), sessions.get(3), "切るのは古い側であること");

    server.stop().await;
}

/// **他のキャラクターの会話を id で覗けない**（404）。**存在しない id も 404。**
///
/// **応答は完全一致**（どちらも同じ本文 —— 存在の有無で文言を変えない）。
#[tokio::test]
async fn a_session_of_another_character_or_an_unknown_id_is_not_found() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    server.create_character("rin", "凛").await;
    let rins = server.new_session("rin").await;
    server.turn(rins, "凛の秘密", "「内緒だよ」").await;
    let aois = server.new_session("aoi").await;
    server.turn(aois, "葵の話", "「なに」").await;

    let not_found = json!({
        "error": {
            "message": "指定されたセッションが存在しません",
            "type": "invalid_request_error",
            "code": "session_not_found",
        }
    });

    // 前提: 凛のセッションは凛の側からは読める（id は正しい）。
    let (status, _) = server.get(&messages_path("rin", rins.get(), "")).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = server.get(&messages_path("aoi", rins.get(), "")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body, not_found);

    let unknown = rins.get().max(aois.get()) + 1000;
    let (status, body) = server.get(&messages_path("aoi", unknown, "")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body, not_found);

    // **一覧にも他人の会話は混ざらない。**
    let (_, body) = server.get("/admin/characters/aoi/sessions").await;
    assert_eq!(session_ids(&body), vec![aois.get()]);
    assert_eq!(body["total_sessions"], json!(1));

    server.stop().await;
}

/// **存在しないキャラクターは `character_not_found`**（セッションの 404 に潰さない）。
#[tokio::test]
async fn an_unknown_character_is_a_character_not_found_on_both_paths() {
    let server = TestServer::start().await;
    let not_found = json!({
        "error": {
            "message": "指定されたキャラクターが存在しません",
            "type": "invalid_request_error",
            "code": "character_not_found",
        }
    });
    for path in [
        "/admin/characters/ghost/sessions".to_owned(),
        messages_path("ghost", 1, ""),
    ] {
        let (status, body) = server.get(&path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert_eq!(body, not_found, "{path}");
    }
    server.stop().await;
}

/// **`before` / `limit` の検証**（0・負・巨大・数でない・空・知らない欄 → 400）。
///
/// **宛先が未知のキャラクターでも 400**——検証は DB に触れる前に済ませる
/// （`list_memories` と同じ順序。正当な宛先だけで測ると順序は観測できない）。
#[tokio::test]
async fn malformed_paging_parameters_are_rejected_before_touching_the_database() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let session = server.new_session("aoi").await;
    server.turn(session, "こんにちは", "「やあ」").await;

    for query in [
        "?before=0",
        "?before=-1",
        "?before=abc",
        "?before=",
        "?before=99999999999999999999",
        "?limit=0",
        "?limit=-5",
        "?limit=abc",
        "?limit=",
        "?limit=201",
        "?limit=99999999999999999999",
        "?after=3",
    ] {
        for handle in ["aoi", "ghost"] {
            let (status, body) = server
                .get(&messages_path(handle, session.get(), query))
                .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{handle}{query}: {body}");
            assert_eq!(
                body["error"]["type"],
                json!("invalid_request_error"),
                "{handle}{query}: {body}"
            );
        }
    }

    // 対照: 正当な値は通る（`before` は存在しない大きな id でもよい——カーソルである）。
    for query in [
        "?before=1",
        "?limit=1",
        "?limit=200",
        "?before=9000000000&limit=3",
    ] {
        let (status, body) = server
            .get(&messages_path("aoi", session.get(), query))
            .await;
        assert_eq!(status, StatusCode::OK, "{query}: {body}");
    }

    server.stop().await;
}

/// **範囲外と、形式の不正は別の理由**（直す相手が違う）。本文は完全一致で固定する。
#[tokio::test]
async fn out_of_range_and_malformed_parameters_say_different_things() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let session = server.new_session("aoi").await;

    let (status, body) = server
        .get(&messages_path("aoi", session.get(), "?limit=0"))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json!({
            "error": {
                "message": "before と limit は 1 以上で、limit は上限以内で指定してください",
                "type": "invalid_request_error",
                "code": "message_page_out_of_range",
            }
        })
    );

    let (status, body) = server
        .get(&messages_path("aoi", session.get(), "?limit=abc"))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json!({
            "error": {
                "message": "問い合わせパラメータの形式が不正です",
                "type": "invalid_request_error",
                "code": "invalid_query",
            }
        })
    );

    server.stop().await;
}

/// **数として読めないセッション id は 400**（404 に潰さない —— 直すのはパスの形）。
/// **識別子の綴りの理由は `invalid_path` に潰さない。**
#[tokio::test]
async fn a_malformed_path_keeps_its_own_reason() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server
        .get("/admin/characters/aoi/sessions/abc/messages")
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], json!("invalid_path"));

    for path in [
        "/admin/characters/A@B/sessions",
        "/admin/characters/A@B/sessions/1/messages",
    ] {
        let (status, body) = server.get(path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert_eq!(
            body["error"]["code"],
            json!("character_handle_at_sign"),
            "{path}: {body}"
        );
    }

    server.stop().await;
}

/// **読むだけ。** 書く口も、1件を指すパスも無い。
///
/// 1件のセッション・1件のメッセージを指すパスを生やすと、`PUT` と `DELETE` が
/// 自然な形になる（記憶・票・属性の履歴で塞いだのと同じ穴）。
#[tokio::test]
async fn only_get_is_exposed_for_the_session_paths() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let session = server.new_session("aoi").await;
    server.turn(session, "こんにちは", "「やあ」").await;
    let id = session.get();

    let method_not_allowed = json!({
        "error": {
            "message": "このエンドポイントは指定されたメソッドを受け付けません",
            "type": "invalid_request_error",
        }
    });
    for path in [
        "/admin/characters/aoi/sessions".to_owned(),
        messages_path("aoi", id, ""),
    ] {
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            let (status, body) = server
                .send(method.clone(), &path, Some(&json!({"messages": []})))
                .await;
            assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
            assert_eq!(body, method_not_allowed, "{method} {path}");
        }
    }

    let not_found = json!({
        "error": {
            "message": "指定されたエンドポイントは存在しません",
            "type": "invalid_request_error",
        }
    });
    for path in [
        format!("/admin/characters/aoi/sessions/{id}"),
        format!("/admin/characters/aoi/sessions/{id}/messages/1"),
        "/admin/sessions".to_owned(),
        format!("/admin/sessions/{id}/messages"),
    ] {
        for method in [
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ] {
            let (status, body) = server.send(method.clone(), &path, Some(&json!({}))).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
            assert_eq!(body, not_found, "{method} {path}");
        }
    }

    // **何も消えていない。**
    let (_, body) = server.get(&messages_path("aoi", id, "")).await;
    assert_eq!(body["total_messages"], json!(2));

    server.stop().await;
}

/// **エラーの本文に DB のパスを載せない。**
#[tokio::test]
async fn error_bodies_never_mention_the_database_path() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    for path in [
        messages_path("aoi", 424_242, ""),
        messages_path("aoi", 1, "?limit=0"),
        messages_path("aoi", 1, "?limit=abc"),
        "/admin/characters/ghost/sessions".to_owned(),
    ] {
        let (_, body) = server.get(&path).await;
        let text = body.to_string();
        assert!(!text.contains(&server.db_path), "{path}: {text}");
        assert!(!text.contains("sqlite"), "{path}: {text}");
    }
    server.stop().await;
}
