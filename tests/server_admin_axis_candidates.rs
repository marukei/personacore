//! **軸を見いだす枠の数を書く口と、軸の候補を読む口**の検証
//! （[ADR-0046](../docs/adr/0046-drift-proposes-new-axes.md) §決定1・§決定6 / 作業単位 A-9a / Issue origin#41）。
//!
//! **LM Studio には繋がない。** 管理 API は上流を必要としない。
//! **サーバは本番の入口（`App::bind`）から起動する。**
//!
//! 候補を**書く HTTP の口は無い**（書くのは裏方だけ / この作業単位では未配線）。
//! 候補はアプリと同じプールを指す `Store` の唯一の書き手（`record_axis_candidate`）で作る。
//!
//! ここで固定するのは:
//!
//!   * **`PATCH` の3状態**——キーを省く・`null` はどちらも「変えない」、数は書く。
//!   * **範囲外は 400 で、1列も書かれない**（同じ要求の他の欄も）。未知のキャラクターは 404。
//!   * **キャラクターの応答（一覧・`PATCH`）に枠の数が載る。**
//!   * **候補の一覧**——いまの状態（導出）・移り変わり・試用の票の内訳。
//!     新しい順・上限つき・総数。**他のキャラクターの候補は出ない。** 読むだけ（405 / 404）。
//!   * エラーレスポンスに内部情報を載せない（本文を完全一致で固定する）。

use std::net::SocketAddr;

use personacore::domain::{
    AxisCandidateId, AxisCandidateRecord, AxisCandidateState, AxisTrialFailure, AxisTrialVote,
    CharacterId, Handle, MemoryContent, MemoryId, NewMemory, SessionId, StatAxis, Subject,
};
use personacore::server::{App, AppConfig, BindAddr, ServerError};
use personacore::store::Store;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// エフェメラルポートで起動した実サーバ。
struct TestServer {
    addr: SocketAddr,
    /// アプリと**同じプールを指す**ハンドル。候補の投入に使う。
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

    async fn patch(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        self.send(Method::PATCH, path, Some(body)).await
    }

    /// キャラクターを1件作り、主キーを返す。**識別子と表示名は違う文字列。**
    async fn create_character(&self, handle: &str, display_name: &str) -> CharacterId {
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
        self.store
            .find_character_by_handle(&Handle::parse(handle).expect("正当な識別子"))
            .await
            .expect("読めること")
            .expect("引けること")
            .id
    }

    /// 枠の数を `PATCH` で書く（**本番の口を通す**）。
    async fn set_slots(&self, handle: &str, slots: u32) {
        let (status, body) = self
            .patch(
                &format!("/admin/characters/{handle}"),
                &json!({"axis_discovery_slots": slots}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "枠の数を書けること: {body}");
    }

    /// そのキャラクター自身の記憶を1件作る（候補の出どころ）。
    async fn memory(&self, id: CharacterId, content: &str) -> MemoryId {
        self.store
            .add_memory(
                id,
                &NewMemory {
                    subject: Subject::Character,
                    content: MemoryContent::parse(content).expect("正当な本文"),
                    category: None,
                },
            )
            .await
            .expect("記憶を追加できること")
    }

    async fn propose(&self, id: CharacterId, name: &str, source: MemoryId) -> AxisCandidateId {
        self.store
            .record_axis_candidate(
                id,
                &AxisCandidateRecord::Propose {
                    name: StatAxis::parse(name).expect("正当な軸名"),
                    source_memory: source,
                },
            )
            .await
            .expect("候補を作れること")
    }

    async fn advance(&self, id: CharacterId, candidate: AxisCandidateId, to: AxisCandidateState) {
        self.store
            .record_axis_candidate(id, &AxisCandidateRecord::Advance { candidate, to })
            .await
            .expect("状態を進められること");
    }

    async fn vote(
        &self,
        id: CharacterId,
        candidate: AxisCandidateId,
        session: SessionId,
        vote: AxisTrialVote,
        times: usize,
    ) {
        for _ in 0..times {
            self.store
                .record_axis_candidate(
                    id,
                    &AxisCandidateRecord::TrialVote {
                        candidate,
                        session,
                        vote,
                    },
                )
                .await
                .expect("票を足せること");
        }
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

/// 一覧から、その識別子のキャラクターの `axis_discovery_slots` を取る。
async fn listed_slots(server: &TestServer, handle: &str) -> Value {
    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK, "一覧を読めること: {body}");
    body["characters"]
        .as_array()
        .expect("配列であること")
        .iter()
        .find(|character| character["handle"] == json!(handle))
        .unwrap_or_else(|| panic!("{handle} が一覧に居ること: {body}"))["axis_discovery_slots"]
        .clone()
}

// ---------------------------------------------------------------------------
// 枠の数を書く口
// ---------------------------------------------------------------------------

/// **`PATCH` の3状態**——数は書き、キーを省いても `null` でも変わらない。
///
/// **値は既定（0）とも上限（4）とも違う 3。** 応答の `character` と一覧の両方に載る。
#[tokio::test]
async fn patch_writes_the_slots_and_omitting_or_null_keeps_them() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;
    server.create_character("rin", "凛").await;
    assert_eq!(listed_slots(&server, "aoi").await, json!(0), "既定は 0");

    let (status, body) = server
        .patch("/admin/characters/aoi", &json!({"axis_discovery_slots": 3}))
        .await;
    assert_eq!(status, StatusCode::OK, "書けること: {body}");
    assert_eq!(
        body["character"]["axis_discovery_slots"],
        json!(3),
        "更新の応答に載ること: {body}"
    );
    assert_eq!(
        listed_slots(&server, "aoi").await,
        json!(3),
        "一覧に載ること"
    );
    assert_eq!(
        listed_slots(&server, "rin").await,
        json!(0),
        "他のキャラクターは変わらないこと"
    );

    // **キーを省く**（表示名だけを変える）。
    let (status, body) = server
        .patch("/admin/characters/aoi", &json!({"display_name": "あおい"}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["character"]["axis_discovery_slots"],
        json!(3),
        "省けば変わらない"
    );

    // **`null`**（NOT NULL の列なので「変えない」/ `stat_scale` と同じ約束）。
    let (status, body) = server
        .patch(
            "/admin/characters/aoi",
            &json!({"axis_discovery_slots": null}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["character"]["axis_discovery_slots"],
        json!(3),
        "null でも変わらない"
    );

    // **0 へ戻せる**（下端も値として書ける）。
    let (status, body) = server
        .patch("/admin/characters/aoi", &json!({"axis_discovery_slots": 0}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["character"]["axis_discovery_slots"], json!(0));

    server.stop().await;
}

/// **上端 4 は通り、5 は 400。1列も書かれない**（同じ要求に正当な欄があっても）。
///
/// 範囲外の形は3通り——大きすぎる数（専用の理由）、負の数・整数でない値（本文の形の不正）。
/// **どれも 400 で、内部情報を載せない。**
#[tokio::test]
async fn slots_out_of_range_are_refused_and_nothing_is_written() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server
        .patch("/admin/characters/aoi", &json!({"axis_discovery_slots": 4}))
        .await;
    assert_eq!(status, StatusCode::OK, "上端ちょうどは通ること: {body}");

    let (status, body) = server
        .patch(
            "/admin/characters/aoi",
            &json!({"display_name": "あおい", "axis_discovery_slots": 5}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        error_body(
            "軸を見いだす枠の数が大きすぎます",
            "axis_discovery_slots_too_large"
        ),
        "専用の理由で、入力も上限も反射しないこと"
    );

    for bad in [json!(-1), json!(2.5), json!("2"), json!(u64::MAX)] {
        let (status, body) = server
            .patch(
                "/admin/characters/aoi",
                &json!({"display_name": "あおい", "axis_discovery_slots": bad}),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
        assert_eq!(
            body,
            error_body("リクエスト本文の形式が不正です", "invalid_body"),
            "{bad}"
        );
    }

    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (
            body["characters"][0]["display_name"].clone(),
            body["characters"][0]["axis_discovery_slots"].clone(),
        ),
        (json!("葵"), json!(4)),
        "拒まれた要求は、同じ要求の他の欄も書かないこと: {body}"
    );

    server.stop().await;
}

/// **未知のキャラクターへの `PATCH` は 404**（枠の数だけの要求でも）。
#[tokio::test]
async fn patching_the_slots_of_an_unknown_character_is_not_found() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server
        .patch("/admin/characters/rin", &json!({"axis_discovery_slots": 2}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        error_body(
            "指定されたキャラクターが存在しません",
            "character_not_found"
        )
    );

    server.stop().await;
}

// ---------------------------------------------------------------------------
// 候補を読む口
// ---------------------------------------------------------------------------

/// **候補が無いキャラクターは空の配列。総数と上限も名乗る。**
#[tokio::test]
async fn a_character_without_candidates_returns_an_empty_array() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server.get("/admin/characters/aoi/axis-candidates").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({
            "character": "aoi",
            "candidates": [],
            "total_candidates": 0,
            "candidates_limit": 50,
        })
    );

    server.stop().await;
}

/// **候補ごとに、いまの状態・移り変わり・試用の票の内訳が載る。新しい順。**
///
/// 3件にして**真ん中**（几帳面さ）だけを合格まで進め、票も違う数にする。
/// 真ん中の候補は**応答を丸ごと突き合わせる**（欄が欠けても余っても赤）。
#[tokio::test]
async fn candidates_come_back_with_their_state_history_and_votes() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    server.set_slots("aoi", 3).await;
    let source = server.memory(aoi, "夜に星を見るのが好き").await;
    let session = server
        .store
        .create_session(aoi)
        .await
        .expect("会話を作れること");

    let oldest = server.propose(aoi, "好奇心", source).await;
    let middle = server.propose(aoi, "几帳面さ", source).await;
    server.propose(aoi, "茶目っ気", source).await;
    server
        .vote(aoi, middle, session, AxisTrialVote::Up, 2)
        .await;
    server
        .vote(aoi, middle, session, AxisTrialVote::Down, 1)
        .await;
    server
        .vote(aoi, middle, session, AxisTrialVote::Zero, 3)
        .await;
    server
        .vote(aoi, oldest, session, AxisTrialVote::Zero, 5)
        .await;
    server
        .advance(aoi, middle, AxisCandidateState::Passed)
        .await;
    server
        .advance(
            aoi,
            oldest,
            AxisCandidateState::Failed(AxisTrialFailure::Silent),
        )
        .await;

    let (status, body) = server.get("/admin/characters/aoi/axis-candidates").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["character"], json!("aoi"));
    assert_eq!(body["total_candidates"], json!(3));

    let candidates = body["candidates"].as_array().expect("配列であること");
    let summary: Vec<Value> = candidates
        .iter()
        .map(|candidate| {
            json!({
                "name": candidate["name"],
                "state": candidate["state"],
                "reason": candidate["reason"],
                "trial_votes": candidate["trial_votes"],
            })
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            json!({"name": "茶目っ気", "state": "trial", "reason": null,
                   "trial_votes": {"up": 0, "down": 0, "zero": 0}}),
            json!({"name": "几帳面さ", "state": "passed", "reason": null,
                   "trial_votes": {"up": 2, "down": 1, "zero": 3}}),
            json!({"name": "好奇心", "state": "failed", "reason": "silent",
                   "trial_votes": {"up": 0, "down": 0, "zero": 5}}),
        ],
        "新しい順に、それぞれの状態と内訳が載ること"
    );

    // **真ん中を丸ごと。** id と時刻は DB が書くので応答から採る（キーごと無くなれば形が合わない）。
    let middle_view = &candidates[1];
    let transitions = middle_view["transitions"]
        .as_array()
        .expect("配列であること");
    assert_eq!(transitions.len(), 2, "{middle_view}");
    assert_eq!(
        middle_view,
        &json!({
            "id": middle.get(),
            "name": "几帳面さ",
            "source_memory_id": source.get(),
            "created_at": middle_view["created_at"],
            "state": "passed",
            "reason": null,
            "transitions": [
                {"state": "trial", "reason": null, "created_at": transitions[0]["created_at"]},
                {"state": "passed", "reason": null, "created_at": transitions[1]["created_at"]},
            ],
            "trial_votes": {"up": 2, "down": 1, "zero": 3},
        }),
        "欠けも余りもなく返ること"
    );
    for at in [
        &middle_view["created_at"],
        &transitions[0]["created_at"],
        &transitions[1]["created_at"],
    ] {
        let at = at.as_str().expect("時刻が文字列であること");
        assert!(
            at.contains('T') && at.ends_with('Z'),
            "UTC の時刻であること: {at}"
        );
    }
    // **不合格の理由は移り変わりの側にも載る。**
    assert_eq!(
        candidates[2]["transitions"][1],
        json!({"state": "failed", "reason": "silent",
               "created_at": candidates[2]["transitions"][1]["created_at"]}),
    );

    server.stop().await;
}

/// **候補の一覧は、訊いたキャラクターのものだけ。**
///
/// **両方に候補を作る**（件数も名前も違う）——片方だけだと、持ち主で絞らない実装が緑になる。
#[tokio::test]
async fn the_candidates_of_each_character_are_their_own() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    let rin = server.create_character("rin", "凛").await;
    server.set_slots("aoi", 2).await;
    server.set_slots("rin", 1).await;
    let aoi_source = server.memory(aoi, "夜に星を見るのが好き").await;
    let rin_source = server.memory(rin, "朝は紅茶を淹れる").await;
    server.propose(aoi, "好奇心", aoi_source).await;
    server.propose(aoi, "几帳面さ", aoi_source).await;
    server.propose(rin, "茶目っ気", rin_source).await;

    let (status, body) = server.get("/admin/characters/rin/axis-candidates").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["character"], json!("rin"));
    assert_eq!(body["total_candidates"], json!(1), "{body}");
    let names: Vec<Value> = body["candidates"]
        .as_array()
        .expect("配列であること")
        .iter()
        .map(|candidate| candidate["name"].clone())
        .collect();
    assert_eq!(names, vec![json!("茶目っ気")], "凛の候補だけ");
    assert_eq!(
        body["candidates"][0]["source_memory_id"],
        json!(rin_source.get())
    );

    let (status, body) = server.get("/admin/characters/aoi/axis-candidates").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total_candidates"], json!(2), "{body}");

    server.stop().await;
}

/// **未知のキャラクターの候補は 404。**
#[tokio::test]
async fn the_candidates_of_an_unknown_character_are_not_found() {
    let server = TestServer::start().await;
    server.create_character("aoi", "葵").await;

    let (status, body) = server.get("/admin/characters/rin/axis-candidates").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        error_body(
            "指定されたキャラクターが存在しません",
            "character_not_found"
        )
    );

    server.stop().await;
}

/// **上限を超えたら新しい側だけが返る。総数は全部を数える。**
///
/// # 期待値を定数から作らない（外の値 52 と 50）
///
/// 3つの主張（総数・件数・名乗る上限）は、**候補が上限を超えている状態でしか観測できない**
/// （`tests/server_admin_attributes.rs::only_the_newest_revisions_come_back_but_the_total_counts_them_all`
/// と同じ形）。枠は 1 なので、作っては不合格にして 52 件積む。
#[tokio::test]
async fn only_the_newest_candidates_come_back_but_the_total_counts_them_all() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    server.set_slots("aoi", 1).await;
    let source = server.memory(aoi, "夜に星を見るのが好き").await;
    for index in 0..52 {
        let candidate = server.propose(aoi, &format!("候補{index}"), source).await;
        server
            .advance(
                aoi,
                candidate,
                AxisCandidateState::Failed(AxisTrialFailure::TooFewSamples),
            )
            .await;
    }

    let (status, body) = server.get("/admin/characters/aoi/axis-candidates").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let candidates = body["candidates"].as_array().expect("配列であること");
    assert_eq!(candidates.len(), 50, "直近 50 件だけを返すこと");
    assert_eq!(body["candidates_limit"], json!(50), "{body}");
    assert_eq!(body["total_candidates"], json!(52), "{body}");
    assert_eq!(candidates[0]["name"], json!("候補51"), "新しい順であること");
    assert_eq!(
        candidates.last().expect("1件はあること")["name"],
        json!("候補2"),
        "切るのは古い側であること"
    );

    server.stop().await;
}

/// **候補のパスに生えているのは `GET` だけ。1件を指すパスも無い**（書く口を作らない）。
#[tokio::test]
async fn only_get_is_exposed_for_the_candidates_path() {
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
                "/admin/characters/aoi/axis-candidates",
                Some(&json!({"candidates": []})),
            )
            .await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{method}");
        assert_eq!(body, method_not_allowed, "{method}");
    }

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
                "/admin/characters/aoi/axis-candidates/1",
                Some(&json!({})),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method}");
        assert_eq!(body, not_found, "{method}");
    }

    server.stop().await;
}

/// **読めない候補があれば 500。ただし内部情報は載せない。キャラクターの一覧は無事。**
#[tokio::test]
async fn a_broken_candidate_is_a_five_hundred_without_internals() {
    let server = TestServer::start().await;
    let aoi = server.create_character("aoi", "葵").await;
    server.set_slots("aoi", 1).await;
    let source = server.memory(aoi, "夜に星を見るのが好き").await;
    let candidate = server.propose(aoi, "好奇心", source).await;

    // **CHECK を満たすが遷移の規則に反する行**（試用 → 採用）を直接足す。
    sqlx::query("INSERT INTO stat_axis_candidate_transition (candidate_id, state) VALUES (?, ?)")
        .bind(candidate.get())
        .bind("adopted")
        .execute(server.store.pool())
        .await
        .expect("行を足せること");

    let (status, body) = server.get("/admin/characters/aoi/axis-candidates").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json!({"error": {"message": "内部エラーが発生しました", "type": "server_error"}}),
    );
    assert!(
        !body.to_string().contains(&server.db_path),
        "DB のパスを漏らさないこと"
    );

    let (status, body) = server.get("/admin/characters").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "キャラクターの一覧は読めること: {body}"
    );

    server.stop().await;
}
