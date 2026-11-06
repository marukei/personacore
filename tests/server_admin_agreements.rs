//! `/admin/characters/{handle}/agreements` の検証（ADR-0051 §決定4〜6 / 作業単位 A-12a）。
//!
//! **LM Studio には繋がない**（本文の確認はモックの上流 `wiremock` に受けさせる）。
//! **サーバは本番の入口（`App::bind`）から起動する**——書いた取り決めが `chat` の配線を通って
//! 本文に載ることを、ここで1本の線として見る。
//!
//! ここで固定するのは主に次の点:
//!
//!   * **書く → 本文の「# 約束・決まり」節の、守る側の小見出しの下に載る。**
//!     **終わらせる → 本文から消える。履歴に「有効→果たした」が残る。**
//!   * **帰属**——利用者の取り決めがキャラクターの小見出しの下に出ない（最終的なプロンプトで見る）。
//!   * **上限**——上限に達したら 409 で断られ、何も保存されない。
//!   * **中身の上限**——超える入力は 400 で断られ、何も保存されない。
//!   * 理由ごとに区別できるエラー、生えていないメソッド、内部情報を載せない本文。

use std::net::SocketAddr;

use personacore::domain::{
    AgreementContent, AgreementEnding, AgreementKeeper, AgreementKind, CharacterId, NewAgreement,
};
use personacore::server::{
    App, AppConfig, BindAddr, ENV_CONTEXT_TOKENS, ENV_LLM_ENDPOINT, ServerError,
};
use personacore::store::Store;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

/// エフェメラルポートで起動した実サーバ（上流はモック）。
struct TestServer {
    addr: SocketAddr,
    store: Store,
    db_path: String,
    client: reqwest::Client,
    upstream: MockServer,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    /// **本番の入口（`App::bind`）から起動する。** 上流は必ずモックへ向ける（実機へ照会しない）。
    async fn start() -> Self {
        Self::start_with_context(None).await
    }

    /// コンテキスト長を明示して起動する（`None` なら既定の経路 —— モックは照会に答えない）。
    async fn start_with_context(context_tokens: Option<&str>) -> Self {
        let context_tokens = context_tokens.map(str::to_owned);
        let upstream = MockServer::start().await;
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,
                    "model":"m","choices":[{"index":0,
                    "message":{"role":"assistant","content":"「はい」"},
                    "finish_reason":"stop"}]}"#,
                "application/json",
            ))
            .mount(&upstream)
            .await;

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db_path = dir.path().join("personacore.sqlite");
        let endpoint = upstream.uri();
        let config = AppConfig::from_lookup(|key| match key {
            ENV_LLM_ENDPOINT => Some(endpoint.clone()),
            ENV_CONTEXT_TOKENS => context_tokens.clone(),
            _ => None,
        })
        .expect("設定を組み立てられること")
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
            upstream,
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
        assert!(
            !text.contains(&self.db_path),
            "応答に DB のパスを載せないこと: {text}"
        );
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

    async fn character_id(&self, handle: &str) -> CharacterId {
        self.store
            .find_character_by_handle(&personacore::domain::Handle::parse(handle).expect("識別子"))
            .await
            .expect("引けること")
            .expect("居ること")
            .id
    }

    /// 取り決めを1件書いて、応答の id を返す（**201 であること**まで見る）。
    async fn write(&self, handle: &str, content: &str, kind: &str, keeper: &str) -> i64 {
        let (status, body) = self
            .post(
                &format!("/admin/characters/{handle}/agreements"),
                &json!({"content": content, "kind": kind, "keeper": keeper}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "書けること: {body}");
        body["id"].as_i64().expect("id が数であること")
    }

    /// 会話を1往復して、**上流へ渡ったシステムプロンプト**を返す。
    async fn chat_system_prompt(&self, model: &str) -> String {
        let before = self
            .upstream
            .received_requests()
            .await
            .expect("要求を記録していること")
            .len();
        let response = self
            .client
            .post(format!("http://{}/v1/chat/completions", self.addr))
            .json(&json!({
                "model": model,
                "messages": [{"role": "user", "content": "こんにちは"}],
            }))
            .send()
            .await
            .expect("サーバへ接続できること");
        let status = response.status();
        let text = response.text().await.expect("本文を読めること");
        assert_eq!(status, StatusCode::OK, "{text}");

        // **この往復の本文の要求**（以前の要求と、後から走る裏方の要求を除く）。
        // 本文の要求はシステムプロンプトが「# 名前」の節を持つ。
        let requests = self
            .upstream
            .received_requests()
            .await
            .expect("要求を記録していること");
        requests[before..]
            .iter()
            .filter(|request| request.url.path() == "/v1/chat/completions")
            .filter_map(|request| serde_json::from_slice::<Value>(&request.body).ok())
            .filter_map(|body| body["messages"][0]["content"].as_str().map(str::to_owned))
            .find(|system| system.contains("# 名前"))
            .expect("本文の要求が上流へ渡っていること")
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

/// エラー応答の本文を**完全一致**で組み立てる（内部情報が混ざったら赤になる）。
fn error_body(message: &str, code: &str) -> Value {
    json!({"error": {"message": message, "type": "invalid_request_error", "code": code}})
}

/// 「# 約束・決まり」節を、**小見出しごとの行の列**に割る（節が無ければ空）。
///
/// **`prompt` の単体テストの同名の関数の写し**（統合テストは crate の内部を見られない）。
/// 期待値の小見出しは**直書き**する（表から作ると、表の中の入れ替えが両側で動いて緑になる）。
fn agreement_groups(system: &str) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    let mut inside = false;
    for line in system.lines() {
        if line == "# 約束・決まり" {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if line.is_empty() {
            break;
        }
        if line.starts_with("## ") {
            groups.push((line.to_owned(), Vec::new()));
            continue;
        }
        if groups.is_empty() {
            groups.push((String::new(), Vec::new()));
        }
        if let Some((_, lines)) = groups.last_mut() {
            lines.push(line.to_owned());
        }
    }
    groups
}

fn group(heading: &str, lines: &[&str]) -> (String, Vec<String>) {
    (
        heading.to_owned(),
        lines.iter().map(|line| (*line).to_owned()).collect(),
    )
}

/// **書く → 本文の節の、守る側の小見出しの下に載る。終わらせる → 本文から消える。
/// 履歴に「有効→果たした」が残る。一覧には終わったものも残る。**
///
/// 3件書き、**真ん中（キャラクターの約束）**を終わらせる。守る側も種類も3件で互いに違う。
#[tokio::test]
async fn a_written_agreement_reaches_the_prompt_under_its_keeper_and_ending_removes_it() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let first = server
        .write("aoi", "名前を呼び捨てにしない", "rule", "user")
        .await;
    let middle = server
        .write("aoi", "週末に海へ連れて行く", "promise", "character")
        .await;
    let last = server
        .write("aoi", "毎朝あいさつを交わす", "rule", "both")
        .await;

    let system = server.chat_system_prompt("aoi@new").await;
    assert_eq!(
        agreement_groups(&system),
        vec![
            group(
                "## 利用者が守るもの",
                &["> - [決まり] 名前を呼び捨てにしない"]
            ),
            group("## あなたが守るもの", &["> - [約束] 週末に海へ連れて行く"]),
            group(
                "## 利用者とあなたの両方が守るもの",
                &["> - [決まり] 毎朝あいさつを交わす"]
            ),
        ],
        "守る側の小見出しの下に、それぞれの取り決めが載ること: {system}"
    );

    let (status, body) = server
        .post(
            &format!("/admin/characters/aoi/agreements/{middle}/transitions"),
            &json!({"state": "fulfilled"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"character": "aoi", "agreement_id": middle, "state": "fulfilled"})
    );

    let system = server.chat_system_prompt("aoi").await;
    assert_eq!(
        agreement_groups(&system),
        vec![
            group(
                "## 利用者が守るもの",
                &["> - [決まり] 名前を呼び捨てにしない"]
            ),
            group(
                "## 利用者とあなたの両方が守るもの",
                &["> - [決まり] 毎朝あいさつを交わす"]
            ),
        ],
        "終わった取り決めは本文から消え、残りは残ること: {system}"
    );
    assert!(!system.contains("週末に海へ連れて行く"), "{system}");

    // **一覧には終わったものも、状態つきで残る**（行は消えない）。
    let (status, body) = server.get("/admin/characters/aoi/agreements").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed: Vec<(i64, &str, &str, &str, &str)> = body["agreements"]
        .as_array()
        .expect("配列であること")
        .iter()
        .map(|agreement| {
            (
                agreement["id"].as_i64().expect("id"),
                agreement["content"].as_str().expect("content"),
                agreement["kind"].as_str().expect("kind"),
                agreement["keeper"].as_str().expect("keeper"),
                agreement["state"].as_str().expect("state"),
            )
        })
        .collect();
    assert_eq!(
        listed,
        vec![
            (first, "名前を呼び捨てにしない", "rule", "user", "active"),
            (
                middle,
                "週末に海へ連れて行く",
                "promise",
                "character",
                "fulfilled"
            ),
            (last, "毎朝あいさつを交わす", "rule", "both", "active"),
        ]
    );
    assert_eq!(body["active_limit"], json!(16), "上限を名乗ること");
    assert!(
        body["agreements"][0]["source_session_id"].is_null(),
        "手で書いたものは出どころが null（キーごと落とさない）: {body}"
    );

    // **履歴に「有効→果たした」が残る**（新しい順の先頭）。書かれた3件も残る。
    let (status, body) = server
        .get("/admin/characters/aoi/agreements/transitions")
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let transitions = body["transitions"].as_array().expect("配列であること");
    let read: Vec<(i64, Value, &str, &str)> = transitions
        .iter()
        .map(|transition| {
            (
                transition["agreement_id"].as_i64().expect("agreement_id"),
                transition["before"].clone(),
                transition["after"].as_str().expect("after"),
                transition["keeper"].as_str().expect("keeper"),
            )
        })
        .collect();
    assert_eq!(
        read,
        vec![
            (middle, json!("active"), "fulfilled", "character"),
            (last, Value::Null, "active", "both"),
            (middle, Value::Null, "active", "character"),
            (first, Value::Null, "active", "user"),
        ]
    );
    assert_eq!(transitions[0]["content"], json!("週末に海へ連れて行く"));
    assert_eq!(body["total_transitions"], json!(4));
    assert_eq!(body["transitions_limit"], json!(200));

    server.stop().await;
}

/// **上限に達したら 409 で断られ、何も保存されない。** 1件終わらせれば、また書ける。
#[tokio::test]
async fn the_limit_is_refused_with_a_conflict_and_nothing_is_stored() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let mut ids = Vec::new();
    for index in 0..16 {
        ids.push(
            server
                .write("aoi", &format!("約束その{index}"), "promise", "character")
                .await,
        );
    }

    let (status, body) = server
        .post(
            "/admin/characters/aoi/agreements",
            &json!({"content": "上限を越える約束", "kind": "rule", "keeper": "user"}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body,
        error_body(
            "有効な取り決めが上限に達しています。古いものを終わらせてください",
            "agreement_limit_reached"
        )
    );

    let (_, listed) = server.get("/admin/characters/aoi/agreements").await;
    let agreements = listed["agreements"].as_array().expect("配列であること");
    assert_eq!(agreements.len(), 16, "1件も増えないこと");
    assert!(
        agreements
            .iter()
            .all(|agreement| agreement["content"] != json!("上限を越える約束")),
        "断った取り決めはどこにも無いこと"
    );
    let (_, history) = server
        .get("/admin/characters/aoi/agreements/transitions")
        .await;
    assert_eq!(
        history["total_transitions"],
        json!(16),
        "履歴も増えないこと"
    );

    // **本文にも載らない**（16 件のまま）。
    let system = server.chat_system_prompt("aoi@new").await;
    assert!(!system.contains("上限を越える約束"), "{system}");

    let (status, _) = server
        .post(
            &format!("/admin/characters/aoi/agreements/{}/transitions", ids[5]),
            &json!({"state": "withdrawn"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    server.write("aoi", "空いた枠の約束", "rule", "user").await;

    server.stop().await;
}

/// **中身の上限を超える入力は 400 で断られ、何も保存されない。** 上限ちょうどは書ける。
#[tokio::test]
async fn content_over_the_limit_is_refused_and_nothing_is_stored() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    // **上限は関数の外の値（60）で押さえる。** 定数と揃っていることは domain が見る。
    assert_eq!(AgreementContent::MAX_CHARS, 60);
    let (status, body) = server
        .post(
            "/admin/characters/aoi/agreements",
            &json!({"content": "約".repeat(61), "kind": "promise", "keeper": "both"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body,
        error_body("取り決めの中身が長すぎます", "agreement_content_too_long")
    );

    for (content, message, code) in [
        (
            "\u{3000} ",
            "取り決めの中身が空です",
            "agreement_content_empty",
        ),
        (
            "毎朝\nあいさつする",
            "取り決めの中身に改行や制御文字は使えません",
            "agreement_content_control",
        ),
    ] {
        let (status, body) = server
            .post(
                "/admin/characters/aoi/agreements",
                &json!({"content": content, "kind": "promise", "keeper": "both"}),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{content:?}: {body}");
        assert_eq!(body, error_body(message, code));
    }

    let (_, listed) = server.get("/admin/characters/aoi/agreements").await;
    assert_eq!(listed["agreements"], json!([]), "1件も書かれないこと");

    server
        .write("aoi", &"約".repeat(60), "promise", "both")
        .await;

    server.stop().await;
}

/// **種類・守る側・終わらせ方の綴り違いは、それぞれの理由で 400。** 欄の欠け・未知の欄は `invalid_body`。
#[tokio::test]
async fn misspelled_fields_have_their_own_reasons() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let id = server
        .write("aoi", "嘘をつかない", "rule", "character")
        .await;

    for (body, message, code) in [
        (
            json!({"content": "約束", "kind": "Promise", "keeper": "user"}),
            "取り決めの種類は promise か rule のどちらかです",
            "agreement_kind_unknown",
        ),
        (
            json!({"content": "約束", "kind": "promise", "keeper": "assistant"}),
            "取り決めの守る側は user / character / both のどれかです",
            "agreement_keeper_unknown",
        ),
        (
            json!({"content": "約束", "kind": "promise"}),
            "リクエスト本文の形式が不正です",
            "invalid_body",
        ),
        (
            json!({"content": "約束", "kind": "promise", "keeper": "user", "state": "fulfilled"}),
            "リクエスト本文の形式が不正です",
            "invalid_body",
        ),
    ] {
        let (status, response) = server.post("/admin/characters/aoi/agreements", &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {response}");
        assert_eq!(response, error_body(message, code), "{body}");
    }

    for state in ["active", "done", "Fulfilled"] {
        let (status, response) = server
            .post(
                &format!("/admin/characters/aoi/agreements/{id}/transitions"),
                &json!({"state": state}),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{state}: {response}");
        assert_eq!(
            response,
            error_body(
                "取り決めの終わらせ方は fulfilled / broken / withdrawn のどれかです",
                "agreement_ending_unknown"
            )
        );
    }

    let (_, listed) = server.get("/admin/characters/aoi/agreements").await;
    assert_eq!(
        listed["agreements"].as_array().expect("配列").len(),
        1,
        "断られた要求は何も書かないこと"
    );
    assert_eq!(listed["agreements"][0]["state"], json!("active"));

    server.stop().await;
}

/// **他のキャラクターの取り決めは 404、もう終わったものは 409。** どちらも何も書かない。
#[tokio::test]
async fn another_characters_agreement_is_not_found_and_an_ended_one_conflicts() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    server.create_character("mio", "澪").await;
    let aois = server
        .write("aoi", "嘘をつかない", "rule", "character")
        .await;
    let mios = server.write("mio", "本を返す", "promise", "user").await;

    let (status, body) = server
        .post(
            &format!("/admin/characters/aoi/agreements/{mios}/transitions"),
            &json!({"state": "broken"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body,
        error_body("指定された取り決めが存在しません", "agreement_not_found")
    );
    let (_, listed) = server.get("/admin/characters/mio/agreements").await;
    assert_eq!(
        listed["agreements"][0]["state"],
        json!("active"),
        "隣は有効のまま"
    );
    let (_, listed) = server.get("/admin/characters/aoi/agreements").await;
    assert_eq!(
        listed["agreements"].as_array().expect("配列").len(),
        1,
        "一覧も持ち主ごと"
    );

    let (status, _) = server
        .post(
            &format!("/admin/characters/aoi/agreements/{aois}/transitions"),
            &json!({"state": "broken"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = server
        .post(
            &format!("/admin/characters/aoi/agreements/{aois}/transitions"),
            &json!({"state": "fulfilled"}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body,
        error_body(
            "指定された取り決めは有効ではありません",
            "agreement_not_active"
        )
    );
    let (_, listed) = server.get("/admin/characters/aoi/agreements").await;
    assert_eq!(
        listed["agreements"][0]["state"],
        json!("broken"),
        "最初の終わり方のまま"
    );

    server.stop().await;
}

/// **不正な本文 × 未知のキャラクターは 400 を優先する。** 正当な本文 × 未知のキャラクターは 404。
#[tokio::test]
async fn an_invalid_body_for_an_unknown_character_is_a_bad_request() {
    let server = TestServer::start().await;

    let (status, body) = server
        .post(
            "/admin/characters/ghost/agreements",
            &json!({"content": "約".repeat(61), "kind": "rule", "keeper": "user"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], json!("agreement_content_too_long"));

    let (status, body) = server
        .post(
            "/admin/characters/ghost/agreements",
            &json!({"content": "約束", "kind": "rule", "keeper": "user"}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], json!("character_not_found"));

    let (status, body) = server
        .post(
            "/admin/characters/ghost/agreements/1/transitions",
            &json!({"state": "active"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], json!("agreement_ending_unknown"));

    for path in [
        "/admin/characters/ghost/agreements",
        "/admin/characters/ghost/agreements/transitions",
    ] {
        let (status, body) = server.get(path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert_eq!(body["error"]["code"], json!("character_not_found"));
    }

    // **数として読めない id は 400**（404 に潰さない）。
    let (status, body) = server
        .post(
            "/admin/characters/ghost/agreements/abc/transitions",
            &json!({"state": "broken"}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    server.stop().await;
}

/// **生えているのは一覧・書く・終わらせる・履歴の4つだけ。** 1件を指すパスは無い。
#[tokio::test]
async fn only_the_listed_methods_are_exposed() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let id = server
        .write("aoi", "嘘をつかない", "rule", "character")
        .await;

    let method_not_allowed = json!({
        "error": {
            "message": "このエンドポイントは指定されたメソッドを受け付けません",
            "type": "invalid_request_error",
        }
    });
    let transitions = format!("/admin/characters/aoi/agreements/{id}/transitions");
    for (method, path) in [
        (Method::PUT, "/admin/characters/aoi/agreements"),
        (Method::PATCH, "/admin/characters/aoi/agreements"),
        (Method::DELETE, "/admin/characters/aoi/agreements"),
        (Method::POST, "/admin/characters/aoi/agreements/transitions"),
        (Method::PUT, "/admin/characters/aoi/agreements/transitions"),
        (
            Method::DELETE,
            "/admin/characters/aoi/agreements/transitions",
        ),
        (Method::GET, transitions.as_str()),
        (Method::PUT, transitions.as_str()),
        (Method::DELETE, transitions.as_str()),
    ] {
        let (status, body) = server.send(method.clone(), path, Some(&json!({}))).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
        assert_eq!(body, method_not_allowed, "{method} {path}");
    }

    let not_found = json!({
        "error": {
            "message": "指定されたエンドポイントは存在しません",
            "type": "invalid_request_error",
        }
    });
    let one = format!("/admin/characters/aoi/agreements/{id}");
    for method in [Method::GET, Method::PUT, Method::PATCH, Method::DELETE] {
        let (status, body) = server.send(method.clone(), &one, Some(&json!({}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}");
        assert_eq!(body, not_found, "{method}");
    }
    let (_, listed) = server.get("/admin/characters/aoi/agreements").await;
    assert_eq!(
        listed["agreements"][0]["state"],
        json!("active"),
        "何も変わらないこと"
    );

    server.stop().await;
}

/// **履歴は上限で切って返し、総数は切らない**（外の値 208 と 200 で押さえる）。
#[tokio::test]
async fn only_the_newest_transitions_come_back_but_the_total_counts_them_all() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let id = server.character_id("aoi").await;

    // 書いて終わらせるを 104 回（2行ずつ / 208 行）。**有効なものは常に 0〜1 件**なので上限に当たらない。
    for index in 0..104 {
        let written = server
            .store
            .add_agreement(
                id,
                &NewAgreement {
                    content: AgreementContent::parse(&format!("約束その{index}")).expect("中身"),
                    kind: AgreementKind::Promise,
                    keeper: AgreementKeeper::User,
                },
            )
            .await
            .expect("書けること");
        server
            .store
            .end_agreement(id, written.id.get(), AgreementEnding::Fulfilled)
            .await
            .expect("終わらせられること");
    }

    let (status, body) = server
        .get("/admin/characters/aoi/agreements/transitions")
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["transitions"].as_array().expect("配列").len(), 200);
    assert_eq!(body["total_transitions"], json!(208));
    assert_eq!(body["transitions_limit"], json!(200));
    assert_eq!(
        body["transitions"][0]["content"],
        json!("約束その103"),
        "新しい順であること"
    );

    server.stop().await;
}

/// CHECK を外して、中身の上限を超える取り決めの行を置く（**同じ接続で外して・置いて・戻す**）。
async fn insert_broken_agreement(store: &Store, character: CharacterId, state: &str) -> i64 {
    let mut connection = store.pool().acquire().await.expect("接続を取れること");
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *connection)
        .await
        .expect("CHECK を外せること");
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO character_agreement (character_id, content, kind, keeper, state) \
         VALUES (?, ?, 'rule', 'user', ?) RETURNING id",
    )
    .bind(character.get())
    .bind("長".repeat(AgreementContent::MAX_CHARS + 1))
    .bind(state)
    .fetch_one(&mut *connection)
    .await
    .expect("行を置けること");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *connection)
        .await
        .expect("CHECK を戻せること");
    id
}

/// **終わった行が壊れていても会話は止まらない。有効な行が壊れたら会話は 500（中身を漏らさない）で、
/// その行を終わらせれば戻る**（docs/03-data-model.md「1行壊れたときに何が止まるか」）。
#[tokio::test]
async fn a_broken_active_agreement_stops_the_chat_and_ending_it_restores_it() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    let id = server.character_id("aoi").await;
    server
        .write("aoi", "嘘をつかない", "rule", "character")
        .await;

    insert_broken_agreement(&server.store, id, "fulfilled").await;
    let system = server.chat_system_prompt("aoi@new").await;
    assert!(
        system.contains("嘘をつかない"),
        "終わった壊れた行は本文を止めない: {system}"
    );

    let broken = insert_broken_agreement(&server.store, id, "active").await;
    let response = server
        .client
        .post(format!("http://{}/v1/chat/completions", server.addr))
        .json(&json!({"model": "aoi", "messages": [{"role": "user", "content": "こんにちは"}]}))
        .send()
        .await
        .expect("サーバへ接続できること");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let text = response.text().await.expect("本文を読めること");
    assert!(
        !text.contains("長長"),
        "格納値を応答に漏らさないこと: {text}"
    );
    assert!(!text.contains(&server.db_path), "{text}");

    let (status, body) = server
        .post(
            &format!("/admin/characters/aoi/agreements/{broken}/transitions"),
            &json!({"state": "withdrawn"}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "中身を読まずに終わらせられること: {body}"
    );
    let system = server.chat_system_prompt("aoi").await;
    assert!(system.contains("嘘をつかない"), "{system}");

    server.stop().await;
}

/// **小さい窓では、取り決めが古い側から落ち、落ちた件数が一覧の口（`last_turn`）に出る**
/// （作業単位 A-12a の決め直し —— 黙って落とさない）。ターンの前は `null`。
#[tokio::test]
async fn a_small_window_drops_the_oldest_agreements_and_the_listing_says_so() {
    let server = TestServer::start_with_context(Some("2048")).await;
    server.create_character("aoi", "葵").await;
    for index in 0..16 {
        server
            .write(
                "aoi",
                &format!("約束{index:02}{}", "長".repeat(50)),
                "promise",
                "character",
            )
            .await;
    }

    let (_, before) = server.get("/admin/characters/aoi/agreements").await;
    assert!(
        before["last_turn"].is_null(),
        "この起動でまだターンが無ければ null（キーごと落とさない）: {before}"
    );
    assert!(
        before
            .as_object()
            .expect("オブジェクト")
            .contains_key("last_turn")
    );

    let system = server.chat_system_prompt("aoi@new").await;
    let (_, after) = server.get("/admin/characters/aoi/agreements").await;
    let included = after["last_turn"]["included"].as_u64().expect("included");
    let dropped = after["last_turn"]["dropped"].as_u64().expect("dropped");
    assert!(dropped > 0, "前提: 小さい窓では落ちること: {after}");
    assert_eq!(included + dropped, 16, "{after}");
    assert!(after["last_turn"]["at"].is_string(), "{after}");

    // **落ちたのは古い側**（いちばん古いものは無く、いちばん新しいものは在る）。
    assert!(!system.contains("約束00"), "古い側から落ちること: {system}");
    assert!(
        included >= 1,
        "前提: 新しい側が1件は載る窓であること: {after}"
    );
    assert!(system.contains("約束15"), "新しい側が残ること: {system}");

    // **`included` は本文の節に実際に描かれた行の数**（`dropped` と取り違えると赤になる）。
    assert_ne!(
        included, dropped,
        "前提: 2つの欄の値が重ならないこと（取り違えが観測できる）: {after}"
    );
    let drawn: usize = agreement_groups(&system)
        .iter()
        .map(|(_, lines)| lines.len())
        .sum();
    assert_eq!(
        u64::try_from(drawn).expect("収まること"),
        included,
        "描かれた行数と last_turn.included が一致すること: {after} / {system}"
    );

    server.stop().await;
}

/// **同じ id で作り直したキャラクターに、前のキャラクターの `last_turn` を見せない**
/// （`AppState::agreement_fit` が作成時刻で突き合わせる / `character.id` は使い回されうる）。
///
/// ターン → しまう → 完全に消す → 同じ識別子で作り直す。**id が実際に同じ**であることを前提で確かめる
/// （違う id なら、この形は作成時刻の照合を何も見ていない）。
#[tokio::test]
async fn a_recreated_character_with_the_same_id_has_no_last_turn() {
    let server = TestServer::start_with_context(Some("2048")).await;
    server.create_character("aoi", "葵").await;
    let first_id = server.character_id("aoi").await;
    server
        .write("aoi", "嘘をつかない", "rule", "character")
        .await;
    server.chat_system_prompt("aoi@new").await;
    let (_, listed) = server.get("/admin/characters/aoi/agreements").await;
    assert!(
        listed["last_turn"].is_object(),
        "前提: ターンの後は last_turn があること: {listed}"
    );

    let (status, body) = server
        .send(
            Method::PATCH,
            "/admin/characters/aoi",
            Some(&json!({"archived": true})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "しまえること: {body}");
    let response = server
        .client
        .delete(format!("http://{}/admin/characters/aoi", server.addr))
        .send()
        .await
        .expect("サーバへ接続できること");
    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "完全に消せること"
    );

    server.create_character("aoi", "葵").await;
    let second_id = server.character_id("aoi").await;
    assert_eq!(
        first_id, second_id,
        "前提: 作り直したキャラクターが同じ id を使っていること"
    );

    let (status, listed) = server.get("/admin/characters/aoi/agreements").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(
        listed["last_turn"].is_null(),
        "作り直したキャラクターには、前のキャラクターの last_turn を見せないこと: {listed}"
    );
    assert_eq!(listed["agreements"], json!([]), "取り決めも消えていること");

    server.stop().await;
}
