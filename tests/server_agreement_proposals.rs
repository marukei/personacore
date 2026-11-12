//! **約束・決まりの提案の配線と、採る・捨てるの口**（[ADR-0051](../docs/adr/0051-promises-and-rules.md)
//! §決定2 —— δ3 ＋ dedup2 / 作業単位 A-12c-1 / Issue origin#71）の検証。
//!
//! 依頼の形・読み・票・後段・持ち越しは `prompt` / `parse` / `server::agreement_proposal` の単体テストが持つ。
//! ここが見るのは**本番の入口から届くもの**:
//!
//! | テスト | 入口 | 見るもの |
//! |---|---|---|
//! | `a_turn_after_a_promise_proposes_it_and_adopting_puts_it_in_the_prompt` | **`App::bind`** | 2往復で提案中が1件でき、一覧に載り、本文に載らず、採ると次の往復から載る |
//! | `proposals_can_be_adopted_with_edits_or_rejected` | `App::bind` | 採る（直してから）・捨てる・409・404・400・405 |
//! | `the_proposal_requests_run_in_the_same_queue_as_extraction` | `App::bind_with_clients` | 裏方の依頼は同時に1本まで（抽出 → 検出 → 確かめの順） |
//! | `a_turn_during_the_proposal_stage_takes_over_and_its_extraction_runs` | 同上 | 提案の段の最中に次の往復が来たら、止めて次の往復の抽出を走らせる |
//! | `a_turn_during_extraction_is_still_dropped_as_busy` | 同上 | 抽出の段の最中なら、今までどおり次の往復を捨てる |
//! | `a_preempted_check_writes_no_proposal` | 同上 | 止めた往復の提案は書かれない |
//!
//! 上流は `wiremock` かモック注入。実機の上流には繋がない。

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use personacore::domain::{
    AgreementContent, AgreementKeeper, AgreementKind, CharacterId, DisplayName, Handle,
    NewAgreement, Persona,
};
use personacore::llm::{
    ChatRequest, ChatStream, Completion, FinishReason, LlmClient, LlmError, LlmFuture,
};
use personacore::server::{
    App, AppConfig, BackgroundProbe, BindAddr, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL, ENV_SUB_MODEL,
};
use personacore::store::Store;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::{Notify, oneshot};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

use common::fixture_scale;

const HANDLE: &str = "aoi";
const DISPLAY_NAME: &str = "葵";
const MAIN_MODEL: &str = "ministral-3-14b-reasoning-2512";
const SUB_MODEL: &str = "ministral-3-3b-reasoning-2512";

/// **検出の依頼だけに現れる綴り**（`jd-v3` の書き方の行）。**`prompt` の定数を呼ばず、期待値として持つ**
/// （呼ぶと綴りを変える変異が両側で同時に動いて緑になる）。
const DETECTION_MARKER: &str = "抜｜引用｜取り決め｜守る側｜中身";

/// **確かめの依頼だけに現れる綴り**（`c1-v1` の書き方の行）。
const CHECK_MARKER: &str = "点検｜番号｜行い｜分類｜守る側｜判定｜中身";

/// 抽出の依頼だけに現れる綴り（`prompt::extraction` の冒頭の文）。
const EXTRACTION_MARKER: &str = "後の会話で役に立つ事実だけを";

/// 本文の依頼だけに現れる見出し。
const PERSONA_HEADING: &str = "# ペルソナ";

/// 1往復目の利用者の発言（**引用はこの中にある**）。
const PROMISING_MESSAGE: &str = "わかったよ。もう黙って出かけない。";

/// 2往復目以降の利用者の発言（引用を含まない）。
const PLAIN_MESSAGE: &str = "ただいま。";

/// 本文の応答。
const REPLY: &str = "「おかえりなさい」 彼女は本を閉じた。";

/// 検出の答え（利用者の発言から1件）。
const DETECTED: &str = "抜｜「もう黙って出かけない」｜取り決め｜利用者｜利用者は黙って出かけない";

/// 確かめの答え（残す・利用者 —— 検出とそろう）。
const KEPT: &str = "点検｜1｜黙って出かける｜取り決め｜利用者｜残す｜利用者は黙って出かけない";

/// 提案の中身（勝った票の最初の回 = 検出の中身）。
const PROPOSED_CONTENT: &str = "利用者は黙って出かけない";

fn upstream_body(model: &str, content: &str) -> String {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop",
        }],
    })
    .to_string()
}

struct Harness {
    upstream: MockServer,
    store: Store,
    addr: std::net::SocketAddr,
    character: CharacterId,
    background: BackgroundProbe,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<Result<(), personacore::server::ServerError>>>,
    _dir: tempfile::TempDir,
}

impl Harness {
    /// **本番の入口（`App::bind`）で起動する。**
    async fn start() -> Self {
        let upstream = MockServer::start().await;
        // **狭い綴りから先に据える**（検出・確かめ・抽出・本文の順）。
        for (marker, model, content) in [
            (DETECTION_MARKER, SUB_MODEL, DETECTED),
            (CHECK_MARKER, SUB_MODEL, KEPT),
            (EXTRACTION_MARKER, SUB_MODEL, ""),
            (PERSONA_HEADING, MAIN_MODEL, REPLY),
        ] {
            Mock::given(matchers::method("POST"))
                .and(matchers::path("/v1/chat/completions"))
                .and(matchers::body_string_contains(marker))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(upstream_body(model, content), "application/json"),
                )
                .mount(&upstream)
                .await;
        }

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let pairs: Vec<(String, String)> = vec![
            (ENV_LLM_ENDPOINT.to_owned(), upstream.uri()),
            (ENV_MAIN_MODEL.to_owned(), MAIN_MODEL.to_owned()),
            (ENV_SUB_MODEL.to_owned(), SUB_MODEL.to_owned()),
        ];
        let config = AppConfig::from_lookup(|key| {
            pairs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(dir.path().join("personacore.sqlite"));

        // **`bind_with_clients` ではない。** 本番と同じ入口であることが要点。
        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let store = app.store().clone();
        let background = app.background_probe();
        let character = create_character(&store, HANDLE, DISPLAY_NAME).await;

        let (shutdown, rx) = oneshot::channel();
        let serving = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });

        Self {
            upstream,
            store,
            addr,
            character,
            background,
            shutdown: Some(shutdown),
            handle: Some(serving),
            _dir: dir,
        }
    }

    /// 1往復投げて**読み切り**、そのターンの裏方が終わるまで待つ。
    ///
    /// **「A が B になる」を待つ形**: 先に、この往復の検出の依頼が届いたこと（＝提案の仕事が走った）を
    /// 押さえ、その後で枠が空くのを待つ。**どちらも上限つき。**
    async fn chat_and_settle(&self, message: &str) -> Vec<Value> {
        let detections_before = self.requests_with(DETECTION_MARKER).await.len();
        let response = reqwest::Client::new()
            .post(format!("http://{}/v1/chat/completions", self.addr))
            .json(&json!({
                "model": HANDLE,
                "messages": [{"role": "user", "content": message}],
            }))
            .send()
            .await
            .expect("サーバへ接続できること");
        assert!(response.status().is_success(), "{:?}", response.status());
        let _ = response.text().await.expect("本文を読めること");

        let deadline = Instant::now() + Duration::from_secs(20);
        while self.requests_with(DETECTION_MARKER).await.len() <= detections_before {
            assert!(
                Instant::now() < deadline,
                "検出の依頼が 20 秒たっても届かない"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        while !self.background.is_idle() {
            assert!(
                Instant::now() < deadline,
                "裏方の枠が 20 秒たっても空かない"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.requests().await
    }

    async fn requests(&self) -> Vec<Value> {
        self.upstream
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|request| request.url.path() == "/v1/chat/completions")
            .map(|request| serde_json::from_slice(&request.body).expect("JSON として読めること"))
            .collect()
    }

    async fn requests_with(&self, marker: &str) -> Vec<Value> {
        self.requests()
            .await
            .into_iter()
            .filter(|request| request["messages"].to_string().contains(marker))
            .collect()
    }

    async fn send(&self, method: Method, path: &str, body: Option<&Value>) -> (StatusCode, Value) {
        let mut request =
            reqwest::Client::new().request(method, format!("http://{}{path}", self.addr));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.expect("サーバへ接続できること");
        let status = response.status();
        let text = response.text().await.expect("本文を読めること");
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send(Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        self.send(Method::POST, path, Some(body)).await
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).expect("停止シグナルを送れること");
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

async fn create_character(store: &Store, handle: &str, display_name: &str) -> CharacterId {
    store
        .create_character(
            &Handle::parse(handle).expect("正当な識別子"),
            &DisplayName::parse(display_name).expect("正当な表示名"),
            &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナ"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること")
}

/// 本文の依頼（`# ペルソナ` を持つもの）の system を全部繋いだもの。
fn persona_prompts(requests: &[Value]) -> Vec<String> {
    requests
        .iter()
        .filter(|request| request["messages"].to_string().contains(PERSONA_HEADING))
        .map(|request| request["messages"].to_string())
        .collect()
}

/// **約束を含む往復の次の往復で、提案中が1件できる。提案中は本文に載らず、採ると次の往復から載る。**
/// （本番の入口 `App::bind` を通る —— `MemoryExtractor::with_agreement_proposals` の配線の見張り番）。
///
/// 上流へ出た依頼も見る:
/// * 検出は往復ごとに1本、確かめは候補のある往復の次に1本（票がそろったので2本目は無い）。
/// * **検出に「# 提案済み」は載らず、確かめに載る。**
/// * 提案の依頼は**サブモデル**へ、応答の上限 320・温度 0.8 で出る。
#[tokio::test]
async fn a_turn_after_a_promise_proposes_it_and_adopting_puts_it_in_the_prompt() {
    let harness = Harness::start().await;

    harness.chat_and_settle(PROMISING_MESSAGE).await;
    let (_, listed) = harness.get("/admin/characters/aoi/agreements").await;
    assert_eq!(
        listed["agreements"],
        json!([]),
        "確かめる前は書かない: {listed}"
    );

    let requests = harness.chat_and_settle(PLAIN_MESSAGE).await;
    let (status, listed) = harness.get("/admin/characters/aoi/agreements").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let agreements = listed["agreements"].as_array().expect("配列");
    assert_eq!(agreements.len(), 1, "{listed}");
    let proposal = &agreements[0];
    assert_eq!(
        (
            &proposal["content"],
            &proposal["kind"],
            &proposal["keeper"],
            &proposal["state"]
        ),
        (
            &json!(PROPOSED_CONTENT),
            &json!("rule"),
            &json!("user"),
            &json!("proposed")
        )
    );
    let session = proposal["source_session_id"].as_i64().expect("生んだ会話");
    assert!(
        harness
            .store
            .find_session(harness.character, session)
            .await
            .expect("会話を引けること")
            .is_some(),
        "生んだ会話は、このキャラクターの会話であること"
    );

    let detections = harness.requests_with(DETECTION_MARKER).await;
    let checks = harness.requests_with(CHECK_MARKER).await;
    assert_eq!((detections.len(), checks.len()), (2, 1), "{requests:#?}");
    for detection in &detections {
        assert!(!detection["messages"].to_string().contains("# 提案済み"));
    }
    assert!(checks[0]["messages"].to_string().contains("# 提案済み"));
    for request in detections.iter().chain(&checks) {
        assert_eq!(request["model"], SUB_MODEL);
        assert_eq!(request["max_tokens"], 320);
        assert!((request["temperature"].as_f64().expect("温度") - 0.8).abs() < 1e-6);
        // **裏方は思考させない**（ADR-0059 §決定1）。
        assert_eq!(request["reasoning_effort"], "none");
    }

    // **提案中は本文に載らない**（3往復目の本文の依頼に中身が無い）。
    let before = persona_prompts(&harness.requests().await).len();
    let requests = harness.chat_and_settle(PLAIN_MESSAGE).await;
    let prompts = persona_prompts(&requests);
    assert_eq!(prompts.len(), before + 1);
    assert!(
        !prompts
            .last()
            .expect("本文の依頼")
            .contains(PROPOSED_CONTENT),
        "提案中が本文に載っている"
    );

    // **採ると、次の往復から本文に載る。**
    let id = proposal["id"].as_i64().expect("id");
    let (status, adopted) = harness
        .post(
            &format!("/admin/characters/aoi/agreements/{id}/adoption"),
            &json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{adopted}");
    assert_eq!(adopted["state"], "active");
    let requests = harness.chat_and_settle(PLAIN_MESSAGE).await;
    assert!(
        persona_prompts(&requests)
            .last()
            .expect("本文の依頼")
            .contains(PROPOSED_CONTENT),
        "採った取り決めが本文に載っていない"
    );
    harness.stop().await;
}

/// **採る（直してから）・捨てる・提案中でないもの・上限・綴り・持ち主・生やしていないメソッド。**
#[tokio::test]
async fn proposals_can_be_adopted_with_edits_or_rejected() {
    let harness = Harness::start().await;
    // 持ち主の縛りを見るための2人目（提案は持たない）。
    create_character(&harness.store, "ren", "蓮").await;
    let session = harness
        .store
        .create_session(harness.character)
        .await
        .expect("会話");
    let proposal = |content: &str| NewAgreement {
        content: AgreementContent::parse(content).expect("中身"),
        kind: AgreementKind::Rule,
        keeper: AgreementKeeper::User,
    };
    harness
        .store
        .record_agreement_proposals(
            harness.character,
            session,
            &[
                proposal("扉を閉める"),
                proposal("夜は出かけない"),
                proposal("名前で呼ぶ"),
            ],
        )
        .await
        .expect("書けること");
    let (_, listed) = harness.get("/admin/characters/aoi/agreements").await;
    let ids: Vec<i64> = listed["agreements"]
        .as_array()
        .expect("配列")
        .iter()
        .map(|agreement| agreement["id"].as_i64().expect("id"))
        .collect();
    let adoption = format!("/admin/characters/aoi/agreements/{}/adoption", ids[1]);
    let rejection = format!("/admin/characters/aoi/agreements/{}/rejection", ids[1]);

    // 綴りの違いは 400（何も変わらない）。
    for (body, code) in [
        (json!({"kind": "oath"}), "agreement_kind_unknown"),
        (json!({"keeper": "assistant"}), "agreement_keeper_unknown"),
        (json!({"content": ""}), "agreement_content_empty"),
        (
            json!({"content": "長".repeat(61)}),
            "agreement_content_too_long",
        ),
        (json!({"state": "active"}), "invalid_body"),
    ] {
        let (status, refused) = harness.post(&adoption, &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {refused}");
        assert_eq!(refused["error"]["code"], code, "{body}");
    }
    // 他のキャラクターの提案は「無い」。
    for path in [
        format!("/admin/characters/ren/agreements/{}/adoption", ids[1]),
        format!("/admin/characters/ren/agreements/{}/rejection", ids[1]),
    ] {
        let (status, missing) = harness.post(&path, &json!({})).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {missing}");
        assert_eq!(missing["error"]["code"], "agreement_not_found");
    }
    let (_, unchanged) = harness.get("/admin/characters/aoi/agreements").await;
    assert_eq!(unchanged, listed, "断った要求では何も変わらないこと");

    // 真ん中を直してから採る。
    let (status, adopted) = harness
        .post(
            &adoption,
            &json!({"content": "利用者は夜に黙って出かけない", "kind": "promise", "keeper": "both"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{adopted}");
    assert_eq!(
        (
            &adopted["id"],
            &adopted["content"],
            &adopted["kind"],
            &adopted["keeper"],
            &adopted["state"],
            &adopted["source_session_id"]
        ),
        (
            &json!(ids[1]),
            &json!("利用者は夜に黙って出かけない"),
            &json!("promise"),
            &json!("both"),
            &json!("active"),
            &json!(session.get())
        )
    );
    // もう提案中ではない（採り直し・捨てるは 409）。
    for path in [&adoption, &rejection] {
        let (status, refused) = harness.post(path, &json!({})).await;
        assert_eq!(status, StatusCode::CONFLICT, "{path}: {refused}");
        assert_eq!(refused["error"]["code"], "agreement_not_proposed");
    }

    // 3つ目を捨てる —— 行は残り、却下になる。
    let (status, rejected) = harness
        .post(
            &format!("/admin/characters/aoi/agreements/{}/rejection", ids[2]),
            &json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{rejected}");
    assert_eq!(
        rejected,
        json!({"character": "aoi", "agreement_id": ids[2], "state": "rejected"})
    );
    let (_, listed) = harness.get("/admin/characters/aoi/agreements").await;
    let states: Vec<&Value> = listed["agreements"]
        .as_array()
        .expect("配列")
        .iter()
        .map(|agreement| &agreement["state"])
        .collect();
    assert_eq!(
        states,
        vec![&json!("proposed"), &json!("active"), &json!("rejected")]
    );
    let (_, transitions) = harness
        .get("/admin/characters/aoi/agreements/transitions")
        .await;
    assert_eq!(
        (
            &transitions["transitions"][0]["before"],
            &transitions["transitions"][0]["after"],
            &transitions["transitions"][1]["before"],
            &transitions["transitions"][1]["after"]
        ),
        (
            &json!("proposed"),
            &json!("rejected"),
            &json!("proposed"),
            &json!("active")
        )
    );
    // 提案中は終わらせられない。
    let (status, refused) = harness
        .post(
            &format!("/admin/characters/aoi/agreements/{}/transitions", ids[0]),
            &json!({"state": "fulfilled"}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"]["code"], "agreement_not_active");

    // **有効が上限なら採れない**（提案中は数えない）。
    let limit = listed["active_limit"].as_u64().expect("上限");
    for index in 1..limit {
        let (status, body) = harness
            .post(
                "/admin/characters/aoi/agreements",
                &json!({"content": format!("決まり{index}"), "kind": "rule", "keeper": "user"}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    let (status, refused) = harness
        .post(
            &format!("/admin/characters/aoi/agreements/{}/adoption", ids[0]),
            &json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"]["code"], "agreement_limit_reached");
    // **採る口でも、文言は「書いて」と言わない**（作業単位 A-12c-2 で操作に依らない形にした）。
    assert_eq!(
        refused["error"]["message"],
        "有効な取り決めが上限に達しています。古いものを終わらせてください"
    );

    // **生やしたメソッドだけが通る**（`PUT` / `DELETE` を生やさない —— 行は消さない）。
    let (_, before) = harness.get("/admin/characters/aoi/agreements").await;
    for path in [&adoption, &rejection] {
        for method in [Method::GET, Method::PUT, Method::PATCH, Method::DELETE] {
            let (status, body) = harness.send(method.clone(), path, Some(&json!({}))).await;
            assert_eq!(
                status,
                StatusCode::METHOD_NOT_ALLOWED,
                "{method} {path}: {body}"
            );
        }
    }
    let (_, after) = harness.get("/admin/characters/aoi/agreements").await;
    assert_eq!(after, before, "何も変わらないこと");
    harness.stop().await;
}

// ---------------------------------------------------------------------------
// 同じ裏方の列（作業単位 A-12c-1 —— 新しい調停を足さない）
// ---------------------------------------------------------------------------

/// 本文の上流の代役（いつも同じ応答）。
struct MainLlm;

impl LlmClient for MainLlm {
    fn pinned(&self) -> Option<Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        Box::pin(async move {
            Ok(Completion {
                content: REPLY.to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        Box::pin(async move { Err(LlmError::MissingContent) })
    }
}

/// 裏方の上流の代役。**同時に何本の依頼が生きているか**を数え、**指定した種類の最初の依頼だけ**を
/// 開くまで止める（止めた依頼の future が落とされたら数える —— 横取りの観測）。
struct QueueSub {
    in_flight: Arc<AtomicUsize>,
    most: AtomicUsize,
    kinds: Mutex<Vec<&'static str>>,
    /// 止める依頼の種類（`None` なら止めない）。
    gated_kind: Option<&'static str>,
    gate: Mutex<Option<oneshot::Receiver<()>>>,
    held: Notify,
    /// 止めた依頼の future が、答えずに落とされた回数。
    dropped: Arc<AtomicUsize>,
}

/// **生きている依頼の数を、future が落ちたときにも減らす番人**（横取りで落とされた依頼も数え終える）。
struct InFlight(Arc<AtomicUsize>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// **答えずに落とされたことを数える番人**（答えたら `answered` にしてから落とす）。
struct DropWatch {
    dropped: Arc<AtomicUsize>,
    answered: bool,
}

impl Drop for DropWatch {
    fn drop(&mut self) {
        if !self.answered {
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl QueueSub {
    fn new(gated_kind: Option<&'static str>) -> (Arc<Self>, oneshot::Sender<()>) {
        let (open, gate) = oneshot::channel();
        let sub = Arc::new(Self {
            in_flight: Arc::new(AtomicUsize::new(0)),
            most: AtomicUsize::new(0),
            kinds: Mutex::new(Vec::new()),
            gated_kind,
            gate: Mutex::new(Some(gate)),
            held: Notify::new(),
            dropped: Arc::new(AtomicUsize::new(0)),
        });
        (sub, open)
    }

    fn kinds(&self) -> Vec<&'static str> {
        self.kinds.lock().expect("ロック").clone()
    }

    fn kind_of(request: &ChatRequest) -> &'static str {
        let text: String = request
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        if text.contains(DETECTION_MARKER) {
            "detection"
        } else if text.contains(CHECK_MARKER) {
            "check"
        } else if text.contains(EXTRACTION_MARKER) {
            "extraction"
        } else {
            "other"
        }
    }
}

impl LlmClient for QueueSub {
    fn pinned(&self) -> Option<Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        let kind = Self::kind_of(request);
        self.kinds.lock().expect("ロック").push(kind);
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        let alive = InFlight(Arc::clone(&self.in_flight));
        let gate = if self.gated_kind == Some(kind) {
            self.gate.lock().expect("ロック").take()
        } else {
            None
        };
        Box::pin(async move {
            let _alive = alive;
            if let Some(gate) = gate {
                let mut watch = DropWatch {
                    dropped: Arc::clone(&self.dropped),
                    answered: false,
                };
                self.held.notify_one();
                let _ = gate.await;
                watch.answered = true;
            } else {
                // **どの依頼も少しの間は生きている**（すぐ返すと、2本が同時に走る作りでも重なりが観測できない
                // —— 抽出と検出を `join!` で並べる変異が、これが無いと緑だった）。
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok(Completion {
                content: match kind {
                    "detection" => DETECTED.to_owned(),
                    "check" => KEPT.to_owned(),
                    _ => String::new(),
                },
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        Box::pin(async move { Err(LlmError::MissingContent) })
    }
}

/// **提案の依頼は抽出と同じ1本の列で直列に走る**（本文の最中に、提案の依頼と別の裏方の依頼が
/// 同時に走らない —— 新しい調停を足さずに、本文の数えた計画の差し引き（裏方1要求分）に収まる理由）。
///
/// 2往復を、裏方の枠が空くのを待ちながら投げる。**裏方の上流へ同時に生きている依頼は最大1本**で、
/// 列は 抽出 → 検出（→ 次の往復の 抽出 → 検出 → 確かめ）の順に出る。代役の上流は各依頼を 100ms 生かす。
#[tokio::test]
async fn the_proposal_requests_run_in_the_same_queue_as_extraction() {
    let (sub, _open) = QueueSub::new(None);
    let server = QueueServer::start(Arc::clone(&sub)).await;

    for (message, want) in [(PROMISING_MESSAGE, 1), (PLAIN_MESSAGE, 2)] {
        server.chat(message).await;
        // **「A が B になる」を待つ**: この往復の検出が届いてから、枠が空くのを待つ。
        wait_until(|| detections(&sub.kinds()) >= want).await;
        wait_until(|| server.background.is_idle()).await;
    }
    assert_eq!(
        sub.kinds(),
        vec![
            "extraction",
            "detection",
            "extraction",
            "detection",
            "check"
        ],
        "列の順は 抽出 → 検出（→ 次の往復で 抽出 → 検出 → 確かめ）"
    );
    assert_eq!(
        sub.most.load(Ordering::SeqCst),
        1,
        "同時に生きていた裏方の依頼は1本まで"
    );
    server.stop().await;
}

/// 検出の依頼の数。
fn detections(kinds: &[&str]) -> usize {
    kinds.iter().filter(|kind| **kind == "detection").count()
}

/// 条件が成り立つまで待つ（**上限 20 秒。ならなければ落とす**）。
async fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(Instant::now() < deadline, "20 秒たっても条件が成り立たない");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// **裏方の上流を [`QueueSub`] にした、本番と同じ組み立てのサーバ**（`App::bind_with_clients`）。
struct QueueServer {
    addr: std::net::SocketAddr,
    store: Store,
    character: CharacterId,
    background: BackgroundProbe,
    shutdown: oneshot::Sender<()>,
    serving: tokio::task::JoinHandle<Result<(), personacore::server::ServerError>>,
    _dir: tempfile::TempDir,
}

impl QueueServer {
    async fn start(sub: Arc<QueueSub>) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリ");
        let config = AppConfig::from_lookup(|_| None)
            .expect("設定")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(dir.path().join("personacore.sqlite"));
        let app = App::bind_with_clients(&config, Arc::new(MainLlm), sub)
            .await
            .expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let store = app.store().clone();
        let background = app.background_probe();
        let character = create_character(&store, HANDLE, DISPLAY_NAME).await;
        let (shutdown, rx) = oneshot::channel::<()>();
        let serving = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });
        Self {
            addr,
            store,
            character,
            background,
            shutdown,
            serving,
            _dir: dir,
        }
    }

    async fn chat(&self, message: &str) {
        let response = reqwest::Client::new()
            .post(format!("http://{}/v1/chat/completions", self.addr))
            .json(&json!({"model": HANDLE, "messages": [{"role": "user", "content": message}]}))
            .send()
            .await
            .expect("サーバへ接続できること");
        assert!(response.status().is_success(), "{:?}", response.status());
        let _ = response.text().await;
    }

    async fn stop(self) {
        let _ = self.shutdown.send(());
        let _ = self.serving.await;
    }
}

/// **提案の段の最中に次の往復が来ると、その段を止めて、次の往復の抽出を走らせる**
/// （作業単位 A-12c-1 のレビュー 要修正1 —— 抽出を `busy` で捨てない）。
///
/// 1往復目の検出を上流で止めたまま2往復目を投げる。止めていた検出は**答えずに落とされ**、
/// 2往復目の抽出と検出が出る。
#[tokio::test]
async fn a_turn_during_the_proposal_stage_takes_over_and_its_extraction_runs() {
    let (sub, _open) = QueueSub::new(Some("detection"));
    let server = QueueServer::start(Arc::clone(&sub)).await;

    server.chat(PROMISING_MESSAGE).await;
    tokio::time::timeout(Duration::from_secs(20), sub.held.notified())
        .await
        .expect("1往復目の検出が 20 秒たっても届かない");
    assert_eq!(sub.kinds(), vec!["extraction", "detection"]);

    server.chat(PLAIN_MESSAGE).await;
    wait_until(|| detections(&sub.kinds()) >= 2).await;
    wait_until(|| server.background.is_idle()).await;
    assert_eq!(
        sub.kinds(),
        vec!["extraction", "detection", "extraction", "detection"],
        "2往復目の抽出が捨てられずに走ること（持ち越しは止めた1本と一緒に失われ、確かめは無い）"
    );
    assert_eq!(
        sub.dropped.load(Ordering::SeqCst),
        1,
        "止めていた1往復目の検出は、答えずに落とされること"
    );
    server.stop().await;
}

/// **抽出の段の最中に次の往復が来たら、今までどおり次の往復を捨てる**（横取りできるのは提案の段だけ）。
#[tokio::test]
async fn a_turn_during_extraction_is_still_dropped_as_busy() {
    let (sub, open) = QueueSub::new(Some("extraction"));
    let server = QueueServer::start(Arc::clone(&sub)).await;

    server.chat(PROMISING_MESSAGE).await;
    tokio::time::timeout(Duration::from_secs(20), sub.held.notified())
        .await
        .expect("1往復目の抽出が 20 秒たっても届かない");
    server.chat(PLAIN_MESSAGE).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        sub.kinds(),
        vec!["extraction"],
        "抽出の最中に、次の往復の裏方を走らせないこと"
    );
    assert_eq!(sub.dropped.load(Ordering::SeqCst), 0, "抽出は止めない");

    open.send(()).expect("開けられること");
    wait_until(|| detections(&sub.kinds()) >= 1).await;
    wait_until(|| server.background.is_idle()).await;
    assert_eq!(
        sub.kinds(),
        vec!["extraction", "detection"],
        "1往復目の列は最後まで走り、2往復目の裏方は丸ごと飛んでいる"
    );
    server.stop().await;
}

/// **止めた往復の提案は書かれない**（確かめの最中に横取りされた1本は、何も書かずに落ちる）。
///
/// 1往復目の候補を2往復目の後で確かめる —— その確かめを上流で止めたまま3往復目を投げる。
#[tokio::test]
async fn a_preempted_check_writes_no_proposal() {
    let (sub, _open) = QueueSub::new(Some("check"));
    let server = QueueServer::start(Arc::clone(&sub)).await;

    server.chat(PROMISING_MESSAGE).await;
    wait_until(|| detections(&sub.kinds()) >= 1).await;
    wait_until(|| server.background.is_idle()).await;
    server.chat(PLAIN_MESSAGE).await;
    tokio::time::timeout(Duration::from_secs(20), sub.held.notified())
        .await
        .expect("2往復目の後の確かめが 20 秒たっても届かない");

    server.chat(PLAIN_MESSAGE).await;
    wait_until(|| detections(&sub.kinds()) >= 3).await;
    wait_until(|| server.background.is_idle()).await;
    assert_eq!(
        sub.kinds(),
        vec![
            "extraction",
            "detection",
            "extraction",
            "detection",
            "check",
            "extraction",
            "detection"
        ]
    );
    assert_eq!(sub.dropped.load(Ordering::SeqCst), 1, "確かめは落とされた");
    assert!(
        server
            .store
            .agreements(server.character)
            .await
            .expect("読める")
            .is_empty(),
        "止めた往復の提案は書かれないこと"
    );
    server.stop().await;
}
