//! **会話が目安の下限を一度割ったら、その会話の間は終わりのトークンを下げる**
//! （[ADR-0066](../docs/adr/0066-the-end-token-is-lowered-once-a-session-has-shrunk.md) / 作業単位 A-22b）が、
//! **本番の入口（`App::bind` → `App::serve`）と走ったままの切り替えを通って**、本文の要求にだけ・条件が揃った回にだけ
//! 載ることの検証。**実際の上流には繋がない**（`wiremock` の偽物）。
//!
//! # 何を見るか（ADR-0066 §決定4 の表の腕）
//!
//! | 見張り番 | 腕 | 観測すること |
//! |---|---|---|
//! | [`arm1_a_session_with_a_reply_below_the_lower_bound_gets_the_logit_bias`] | 1 | 面に値・目安・下限未満の返答 → 本文の要求に `logit_bias` が載る（新しい会話の最初の往復には載らない） |
//! | [`arm2_when_every_reply_meets_the_lower_bound_the_request_is_byte_for_byte_the_unwritten_one`] | 2 | 返答がすべて下限以上 → 載らない（要求のバイト列が欄の無い面と同じ） |
//! | [`arm3_once_short_the_session_keeps_the_logit_bias_after_a_long_reply`] | 3 | 下限未満の返答の後に下限以上の返答が来ても、次の往復に載り続ける |
//! | [`arm4_without_a_response_length_target_nothing_is_lowered`] | 4 | 目安が無いキャラクターでは、短い返答があっても載らない |
//! | [`arm1_a_session_with_a_reply_below_the_lower_bound_gets_the_logit_bias`] | 5 | 裏方の要求には載らない（`sub` 面が無い構成で見る） |
//! | [`arm6_a_short_reply_in_another_session_does_not_count`] | 6 | 同じキャラクターの別の会話の短い返答では載らない |
//! | [`arm7_a_reply_exactly_at_the_lower_bound_does_not_count`] | 7 | 下限ちょうどの返答だけなら載らない |
//! | [`arm8_switching_follows_the_end_token_bias_even_when_endpoint_and_model_stay`] | 8 | 切り替えで追従する。宛先とモデルが同じで `end_token_bias` だけが違うプロファイルの間で |
//!
//! ほかに [`an_empty_body_is_retried_with_the_same_logit_bias`]（空の本文の引き直しも同じ値 / §決定2）。
//!
//! # 字数の作り方
//!
//! 偽の上流は本文にも裏方にも同じ本文 [`REPLY`]（**16 字・40 バイト**）を返す。保存される返答は 16 字。
//! 目安の下限を 17（割る）・16（ちょうど）・10（割らない）に置き分ける。**バイト数で数える実装は、16 字の返答を
//! 40 と数えて「割らない」と読む**ので、腕1が赤になる。**短い返答は種から入れる**（[`seed_turn`]。日本語5字 = 15 バイト。
//! 下限 10 に対して、字数では割り、バイト数では割らない）。
//!
//! # 守っていないもの（先に書く）
//!
//! * **`main` の結線**（`config_for` が `with_profile_context` を呼ぶこと）は bin なので、ここからは見えない
//!   （`tests/server_profile_sampling.rs` と同じく、`main` と同じ段をここで組む）。
//! * **ストリーミングの経路**は見ない（送る本文はストリーミングでも同じ `ChatRequest` で、線の上の位置は
//!   `llm::request` の `an_end_token_bias_adds_one_logit_bias_at_the_top_level` がストリーミングの腕で見る）。
//! * **値が上流で効いたか**は見ない（送った値まで）。

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use personacore::domain::{
    CharacterId, DisplayName, Handle, NewMessage, Persona, ResponseLength, Role, SessionId,
};
use personacore::server::{
    App, AppConfig, BackgroundProbe, BindAddr, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL, ENV_SUB_MODEL,
    ServerError, overlay_profile, parse_profiles,
};
use personacore::store::{CharacterSettings, Store};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

use common::{bracketed_field, fixture_scale, has_field, install_global_log};

/// **腕を1本ずつ走らせる**（ログはバイナリで1つ。切り替えの行と掛けた往復の行を、腕ごとに読む）。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 本文に指定するモデル。
const MAIN_MODEL: &str = "end-token-main-model";

/// 会話するキャラクター（`model` に入れる値）。
const HANDLE: &str = "kaede";
/// 呼び名。**識別子とは別の綴り。**
const DISPLAY_NAME: &str = "楓";
/// 利用者の発言。
const USER_MESSAGE: &str = "ただいま";

/// **偽の上流が返す本文**（16 字・40 バイト）。**抽出として読める行**でもある。
const REPLY: &str = "- 利用者: 山あいの町で育った";

/// **種から入れる短い返答**（5 字・15 バイト）。
const SHORT_REPLY: &str = "うなずいた";

/// **書く番号**（測ったモデルの終わりのトークンの番号と同じ数。番号そのものに意味は無い）。
const TOKEN: u32 = 248_046;

/// **本文の依頼だけに現れる見出し**（`prompt` の定数を呼ばない）。
const PERSONA_HEADING: &str = "# ペルソナ";

/// **送るサンプラの行**（`server::context::SAMPLERS_LINE` の写し）。
const SAMPLERS_LINE: &str = "上流へ送るサンプラの値（面ごと）";

/// **掛けた往復の行**（`server::end_token::LOWERED_LINE` の写し）。
const LOWERED_LINE: &str =
    "この会話は返答の長さの目安の下限を割ったので、終わりのトークンを下げて送ります";

/// 待つ上限。**超えたら落とす**（緑のまま何も測らない形にしない）。
const WAIT_LIMIT: Duration = Duration::from_secs(20);
/// 待ちの刻み。
const POLL_STEP: Duration = Duration::from_millis(10);

/// 上流が返す本文（OpenAI 互換）。
fn upstream_body(content: &str) -> String {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": MAIN_MODEL,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop",
        }],
    })
    .to_string()
}

/// **本文と裏方の両方を受けられる上流**（どちらにも [`REPLY`] を返す）。
async fn upstream() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(upstream_body(REPLY), "application/json"),
        )
        .mount(&server)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(
                r#"{{"object":"list","data":[
                  {{"id":"{MAIN_MODEL}","object":"model","type":"llm","state":"loaded",
                    "max_context_length":262144,"loaded_context_length":16384}}]}}"#
            ),
            "application/json",
        ))
        .mount(&server)
        .await;
    server
}

/// 上流へ届いた依頼の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// 本文。
    Body,
    /// 裏方（抽出・約束と決まりの提案）。
    Background,
}

/// **依頼の本文から種類を決める。** 分からなければ落とす（黙って数えない形にしない）。
fn kind_of(request: &Value) -> Kind {
    let messages = request["messages"].to_string();
    let extraction = messages.contains(personacore::prompt::extraction::EXAMPLE_USER_LINE);
    let proposal = [
        "抜｜引用｜取り決め｜守る側｜中身",
        "点検｜番号｜行い｜分類｜守る側｜判定｜中身",
    ]
    .iter()
    .any(|marker| messages.contains(marker));
    if extraction || proposal {
        Kind::Background
    } else if messages.contains(PERSONA_HEADING) {
        Kind::Body
    } else {
        panic!("依頼の種類が決まらない: {messages}")
    }
}

/// **届いたチャットの依頼1件**（種類・`logit_bias` の欄・生のバイト列。届いた順）。
#[derive(Debug, Clone)]
struct Seen {
    kind: Kind,
    /// `logit_bias` の欄（無ければ `None` —— **欄が無いことと `null` を区別する**）。
    logit_bias: Option<Value>,
    bytes: Vec<u8>,
}

/// その上流へ届いたチャットの依頼（届いた順）。
async fn seen(server: &MockServer) -> Vec<Seen> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .map(|request| {
            let body: Value = serde_json::from_slice(&request.body).expect("JSON として読めること");
            Seen {
                kind: kind_of(&body),
                logit_bias: body.get("logit_bias").cloned(),
                bytes: request.body.clone(),
            }
        })
        .collect()
}

/// **種類ごとの `logit_bias` の並び**（重複を畳まない）。
fn biases_of(seen: &[Seen], kind: Kind) -> Vec<Option<Value>> {
    seen.iter()
        .filter(|one| one.kind == kind)
        .map(|one| one.logit_bias.clone())
        .collect()
}

/// **本文の面**（`end_token_bias` は書くときだけ）。
fn main_face(endpoint: &str, bias: Option<f64>) -> Value {
    let mut face = json!({ "endpoint": endpoint, "model": MAIN_MODEL });
    if let Some(bias) = bias {
        face["sampling"] = json!({ "end_token_bias": { "token": TOKEN, "bias": bias } });
    }
    face
}

/// **送られるはずの `logit_bias`**（手で書く。`llm` の綴りから作らない）。
fn wire(bias: f64) -> Option<Value> {
    Some(json!({ "248046": bias }))
}

/// **`main` と同じ段で設定を組む**（`overlay_profile` → `with_profile_context` → `with_profile_keys` →
/// `with_profile_sampling`）。
fn config_of(source: &Value, name: &str, db_path: &Path) -> AppConfig {
    let pairs: Vec<(String, String)> = vec![
        // **囮**: プロファイルが勝つので、ここへは何も届かない。
        (ENV_LLM_ENDPOINT.to_owned(), "http://127.0.0.1:9".to_owned()),
        (ENV_MAIN_MODEL.to_owned(), "decoy-main".to_owned()),
        (ENV_SUB_MODEL.to_owned(), "decoy-sub".to_owned()),
    ];
    let base = move |key: &str| {
        pairs
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.clone())
    };
    let profiles =
        parse_profiles(&source.to_string(), None).expect("植えたプロファイルが読めること");
    let settings = profiles.select(name).expect("在ること").settings();
    AppConfig::from_lookup(overlay_profile(&settings, base))
        .expect("設定を組み立てられること")
        .with_profile_context(&settings)
        .expect("書いた値を載せられること")
        .with_profile_keys(&settings)
        .with_profile_sampling(&settings)
        .with_bind(BindAddr::loopback(0))
        .with_database_path(db_path)
}

/// **目安**（下限, 上限）。上限は下限と違う値にする（下限と上限を取り違える実装を赤にする）。
fn target(min: u32) -> ResponseLength {
    ResponseLength::parse(min, 600).expect("範囲の中")
}

/// **`App::bind`（本番の入口）で起動し、走らせる。**
struct Running {
    port: u16,
    store: Store,
    character: CharacterId,
    background: BackgroundProbe,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
}

impl Running {
    /// `length` はキャラクターの目安（`None` なら書かない）。
    async fn start(
        config: &AppConfig,
        profiles: Option<PathBuf>,
        length: Option<ResponseLength>,
    ) -> Self {
        let mut app = App::bind(config).await.expect("待ち受けを開始できること");
        if let Some(path) = profiles {
            app = app.with_profiles_file_for_tests(path);
        }
        let store = app.store().clone();
        let character = store
            .create_character_with_settings(
                &Handle::parse(HANDLE).expect("正当な名前であること"),
                &DisplayName::parse(DISPLAY_NAME).expect("正当な表示名であること"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
                None,
                fixture_scale(),
                CharacterSettings {
                    response_length: length,
                    ..CharacterSettings::default()
                },
            )
            .await
            .expect("キャラクターを作成できること");
        let port = app.local_addr().port();
        let background = app.background_probe();
        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });
        Self {
            port,
            store,
            character,
            background,
            shutdown: Some(shutdown),
            handle: Some(handle),
        }
    }

    /// **会話を1つ作る**（往復は書かない）。
    async fn session(&self) -> SessionId {
        self.store
            .create_session(self.character)
            .await
            .expect("会話を作れること")
    }

    /// **種の往復を1つ書く**（利用者の発言と、その返答 `reply`）。
    async fn seed_turn(&self, session: SessionId, reply: &str) {
        let message = |role: Role, text: &str| NewMessage {
            role,
            dialogue: None,
            narration: None,
            raw_content: text.to_owned(),
        };
        self.store
            .append_turn(
                session,
                &message(Role::User, USER_MESSAGE),
                &message(Role::Assistant, reply),
            )
            .await
            .expect("種の往復を書けること");
    }

    /// **その会話で1往復させ、そのターンの裏方（抽出と提案）が終わるまで待つ。**
    ///
    /// 「A が B になる」を待つ形: 先に `server` に届いた抽出の依頼が `extractions` 件になるのを待ち（そのターンの裏方が
    /// 走った）、次に裏方の枠が空くのを待つ（終わった）。
    async fn turn(&self, session: SessionId, server: &MockServer, extractions: usize) {
        let response = reqwest::Client::new()
            .post(format!(
                "http://127.0.0.1:{}/v1/chat/completions",
                self.port
            ))
            .json(&json!({
                "model": format!("{HANDLE}@{}", session.get()),
                "messages": [{"role": "user", "content": USER_MESSAGE}],
                "stream": false,
            }))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .expect("サーバへ接続できること");
        let status = response.status();
        let text = response.text().await.expect("本文を読めること");
        assert!(status.is_success(), "{status}: {text}");

        let deadline = Instant::now() + WAIT_LIMIT;
        loop {
            let seen = extraction_count(server).await;
            if seen >= extractions {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "抽出の依頼が {extractions} 件に届かない（いま {seen} 件）"
            );
            tokio::time::sleep(POLL_STEP).await;
        }
        while !self.background.is_idle() {
            assert!(Instant::now() < deadline, "裏方の枠が空かない");
            tokio::time::sleep(POLL_STEP).await;
        }
    }

    /// **`POST /admin/upstream/profile`**。200 でなければ落とす。
    async fn switch_to(&self, name: &str) {
        let response = reqwest::Client::new()
            .post(format!(
                "http://127.0.0.1:{}/admin/upstream/profile",
                self.port
            ))
            .json(&json!({ "name": name }))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .expect("切り替えを依頼できること");
        let status = response.status().as_u16();
        let text = response.text().await.expect("本文を読めること");
        assert_eq!(status, 200, "{name} へ切り替えられること: {text}");
    }

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

/// その上流へ届いた抽出の依頼の件数。
async fn extraction_count(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .filter(|request| {
            String::from_utf8_lossy(&request.body)
                .contains(personacore::prompt::extraction::EXAMPLE_USER_LINE)
        })
        .count()
}

/// **1つのプロファイル（`sub` 面なし）で起こす。**
async fn start_one(
    name: &str,
    bias: Option<f64>,
    length: Option<ResponseLength>,
    dir: &Path,
) -> (Running, MockServer) {
    let server = upstream().await;
    let source = json!({ name: { "main": main_face(&server.uri(), bias) } });
    let running = Running::start(
        &config_of(&source, name, &dir.join(format!("{name}.sqlite"))),
        None,
        length,
    )
    .await;
    (running, server)
}

/// **その往復の本文の要求の `logit_bias`**（届いた依頼の `before` 件目から後で、本文はちょうど1件）。
fn body_bias_since(all: &[Seen], before: usize) -> Option<Value> {
    let bodies = biases_of(&all[before..], Kind::Body);
    assert_eq!(bodies.len(), 1, "その往復の本文の要求は1件: {bodies:?}");
    bodies[0].clone()
}

/// **腕1・腕5: 面に値・目安・下限未満の返答 → 本文の要求に `logit_bias` が載る。裏方には載らない**
/// （ADR-0066 §決定2・§決定1）。
///
/// 目安の下限は 17。偽の上流の返答は 16 字（40 バイト）。
///
/// | 往復 | その会話の保存済みの返答 | 本文の `logit_bias` |
/// |---|---|---|
/// | 1 | 無い（新しい会話） | **載らない** |
/// | 2 | 16 字（下限未満） | `{"248046":-3.0}` |
///
/// **腕5**: `sub` 面が無い（裏方は本文と同じ宛先・同じモデル）構成で、**抽出と提案のどれにも載らない**。
/// 面の設定に値を入れる実装（`sub` 面が無ければ裏方へ写る）はここで赤になる。
/// **起動の行**（ADR-0058 §決定3）は本文の面に書いた値を言い、裏方の面の行には欄を出さない。
#[tokio::test]
async fn arm1_a_session_with_a_reply_below_the_lower_bound_gets_the_logit_bias() {
    let _serial = SERIAL.lock().await;
    let log = install_global_log("personacore=info");
    let mark = log.mark();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (running, server) = start_one("arm1", Some(-3.0), Some(target(17)), dir.path()).await;
    let session = running.session().await;

    running.turn(session, &server, 1).await;
    let first = seen(&server).await;
    assert_eq!(
        biases_of(&first, Kind::Body),
        vec![None],
        "新しい会話の最初の往復は掛からない: {first:?}"
    );
    running.turn(session, &server, 2).await;
    let all = seen(&server).await;
    assert_eq!(
        body_bias_since(&all, first.len()),
        wire(-3.0),
        "下限未満の返答のある会話の本文の要求に載っていない"
    );

    // **腕5**: 裏方には一度も載らない。
    let background = biases_of(&all, Kind::Background);
    assert!(
        background.len() >= 2,
        "前提: 2往復ぶんの裏方が本文の上流へ届いた: {background:?}"
    );
    assert!(
        background.iter().all(Option::is_none),
        "裏方の要求に logit_bias が載った（sub 面が無いのに裏方へ写した？）: {background:?}"
    );

    let output = log.contents_since(mark);
    let startup = |face: &str| -> Vec<String> {
        output
            .lines()
            .filter(|line| {
                line.contains(SAMPLERS_LINE)
                    && has_field(line, &format!("upstream_face={face}"))
                    && has_field(line, "occasion=\"startup\"")
            })
            .map(str::to_owned)
            .collect()
    };
    let main_lines = startup("main");
    assert_eq!(main_lines.len(), 1, "{main_lines:?}");
    assert!(
        has_field(&main_lines[0], r#"end_token_bias={"248046":-3.0}"#)
            && bracketed_field(&main_lines[0], "overridden") == Some(r#"["end_token_bias"]"#),
        "起動の本文の面の行が、書いた end_token_bias を言っていない: {}",
        main_lines[0]
    );
    let sub_lines = startup("sub");
    assert_eq!(sub_lines.len(), 1, "{sub_lines:?}");
    assert!(
        !sub_lines[0].contains("end_token_bias"),
        "裏方の面の行に end_token_bias を出した: {}",
        sub_lines[0]
    );
    running.stop().await;
}

/// **腕2: 返答がすべて下限以上 → 載らない。要求のバイト列が欄の無い面と同じ**（ADR-0066 §決定2・§決定1）。
///
/// 目安の下限は 10（返答は 16 字）。**同じ流れを、欄を書いた面と書いていない面で1回ずつ流し、本文の要求を
/// バイト列で比べる**（2往復とも）。**条件を見ずに毎回載せる実装は、2往復目で赤になる**（1往復目は返答が無い）。
///
/// **対照**: 下限を 17 にした腕1では、同じ2往復目に載る（「何も送らない実装」ではない）。
#[tokio::test]
async fn arm2_when_every_reply_meets_the_lower_bound_the_request_is_byte_for_byte_the_unwritten_one()
 {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let mut bodies = Vec::new();
    for (name, bias) in [("written", Some(-3.0)), ("unwritten", None)] {
        let (running, server) = start_one(name, bias, Some(target(10)), dir.path()).await;
        let session = running.session().await;
        running.turn(session, &server, 1).await;
        running.turn(session, &server, 2).await;
        let all = seen(&server).await;
        let body: Vec<Vec<u8>> = all
            .iter()
            .filter(|one| one.kind == Kind::Body)
            .map(|one| one.bytes.clone())
            .collect();
        assert_eq!(body.len(), 2, "{name}: 本文の要求は2件");
        assert_eq!(
            biases_of(&all, Kind::Body),
            vec![None, None],
            "{name}: 返答がすべて下限以上なのに載った"
        );
        bodies.push(body);
        running.stop().await;
    }
    assert_eq!(
        bodies[0], bodies[1],
        "欄を書いた面の要求のバイト列が、書いていない面と違う（2往復目まで）"
    );
}

/// **腕3: 下限未満の返答の後に下限以上の返答が来ても、次の往復に載り続ける**（ADR-0066 §決定2「一度割ったら掛け続ける」）。
///
/// 目安の下限は 10。種に [`SHORT_REPLY`]（5 字・15 バイト）を1往復入れ、その後に実際の往復を2回流す
/// （偽の上流の返答は 16 字 —— 下限以上）。
///
/// | 往復 | 直前の返答 | その会話の最短 | 本文の `logit_bias` |
/// |---|---|---|---|
/// | 2 | 5 字（種） | 5 | `-2.5` |
/// | 3 | **16 字（下限以上）** | 5 | **`-2.5`（載り続ける）** |
///
/// **「直前の返答だけを見る」実装は3往復目で赤になる。** バイト数で数える実装は、種の 15 バイトを下限以上と読んで
/// 2往復目で赤になる。**掛けた往復の行**（§決定2）を、その往復の会話の識別子・下限・最短の字数・送った値で読む。
#[tokio::test]
async fn arm3_once_short_the_session_keeps_the_logit_bias_after_a_long_reply() {
    let _serial = SERIAL.lock().await;
    let log = install_global_log("personacore=info");
    let mark = log.mark();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (running, server) = start_one("arm3", Some(-2.5), Some(target(10)), dir.path()).await;
    let session = running.session().await;
    running.seed_turn(session, SHORT_REPLY).await;

    running.turn(session, &server, 1).await;
    let second = seen(&server).await;
    assert_eq!(
        body_bias_since(&second, 0),
        wire(-2.5),
        "種の短い返答（5 字）のある会話に載っていない"
    );
    running.turn(session, &server, 2).await;
    let all = seen(&server).await;
    assert_eq!(
        body_bias_since(&all, second.len()),
        wire(-2.5),
        "直前の返答が下限以上になったら外れた（直前だけを見ている？）"
    );

    let output = log.contents_since(mark);
    let lowered: Vec<&str> = output
        .lines()
        .filter(|line| line.contains(LOWERED_LINE))
        .collect();
    assert_eq!(lowered.len(), 2, "掛けた往復ごとに1行: {lowered:?}");
    for line in &lowered {
        for field in [
            format!("character={HANDLE}"),
            format!("session_id={}", session.get()),
            "response_length_min=10".to_owned(),
            "shortest_reply_chars=5".to_owned(),
            r#"logit_bias={"248046":-2.5}"#.to_owned(),
        ] {
            assert!(has_field(line, &field), "{field} が無い: {line}");
        }
    }
    running.stop().await;
}

/// **腕4: 目安が無いキャラクターでは、短い返答があっても載らない**（ADR-0066 §決定2 の条件2）。
///
/// 面には値を書き、会話には種の短い返答（5 字）と偽の上流の返答（16 字）がある。
#[tokio::test]
async fn arm4_without_a_response_length_target_nothing_is_lowered() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (running, server) = start_one("arm4", Some(-3.0), None, dir.path()).await;
    let session = running.session().await;
    running.seed_turn(session, SHORT_REPLY).await;
    running.turn(session, &server, 1).await;
    running.turn(session, &server, 2).await;
    let all = seen(&server).await;
    assert_eq!(
        biases_of(&all, Kind::Body),
        vec![None, None],
        "目安の無いキャラクターに載った"
    );
    running.stop().await;
}

/// **腕6: 同じキャラクターの別の会話の短い返答では載らない**（ADR-0066 §決定2）。
///
/// 目安の下限は 10。会話 A には種の短い返答（5 字）だけがある。会話 B で2往復流す（返答は 16 字）。
/// **会話を区別しない実装（キャラクターの返答の最短を見る）は、B の往復に載せて赤になる。**
/// **対照**: 同じ起動で A に1往復流すと載る（A の短い返答はちゃんと数えられている）。
#[tokio::test]
async fn arm6_a_short_reply_in_another_session_does_not_count() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (running, server) = start_one("arm6", Some(-3.0), Some(target(10)), dir.path()).await;
    let other = running.session().await;
    running.seed_turn(other, SHORT_REPLY).await;
    let session = running.session().await;
    assert_ne!(other, session, "前提: 別の会話");

    running.turn(session, &server, 1).await;
    running.turn(session, &server, 2).await;
    let in_b = seen(&server).await;
    assert_eq!(
        biases_of(&in_b, Kind::Body),
        vec![None, None],
        "別の会話の短い返答で載った（会話を区別していない？）"
    );

    // **対照**: 短い返答のある会話 A では載る。
    running.turn(other, &server, 3).await;
    let all = seen(&server).await;
    assert_eq!(body_bias_since(&all, in_b.len()), wire(-3.0));
    running.stop().await;
}

/// **腕7: 下限ちょうどの返答だけなら載らない**（ADR-0066 §決定2「下限ちょうどは割っていない」）。
///
/// 目安の下限は 16、偽の上流の返答は 16 字。**`<=` と取り違える実装は2往復目で赤になる。**
/// **対照**: 下限を 17 にした腕1では同じ2往復目に載る（境界の片側）。
#[tokio::test]
async fn arm7_a_reply_exactly_at_the_lower_bound_does_not_count() {
    let _serial = SERIAL.lock().await;
    assert_eq!(REPLY.chars().count(), 16, "前提: 偽の上流の返答は 16 字");
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let (running, server) = start_one("arm7", Some(-3.0), Some(target(16)), dir.path()).await;
    let session = running.session().await;
    running.turn(session, &server, 1).await;
    running.turn(session, &server, 2).await;
    running.turn(session, &server, 3).await;
    let all = seen(&server).await;
    assert_eq!(
        biases_of(&all, Kind::Body),
        vec![None, None, None],
        "下限ちょうどの返答で載った（< ではなく <= で比べている？）"
    );
    running.stop().await;
}

/// **腕8: 走ったままの切り替えで追従する。宛先とモデルが同じで `end_token_bias` だけが違うプロファイルの間で**
/// （ADR-0066 §決定4 の腕8 / ADR-0058 §決定2）。
///
/// 目安の下限は 17（偽の上流の返答 16 字は下限未満 —— 2往復目からは毎回条件が揃う）。
///
/// | 往復 | プロファイル | 遷移 | 本文の `logit_bias` |
/// |---|---|---|---|
/// | 1 | `lowered-a`（-3.0） | 起動 | 無し（返答がまだ無い） |
/// | 2 | `lowered-a` | — | -3.0 |
/// | 3 | `lowered-b`（-1.5） | 値あり → 値あり | -1.5 |
/// | 4 | `plain`（なし） | 値あり → 値なし | **無し（持ち越さない）** |
/// | 5 | `lowered-a` | 値なし → 値あり | -3.0 |
///
/// **3つのプロファイルは宛先もモデルも同じ綴り** —— 違うと、`end_token_bias` を比べない実装でも切り替えが起きて緑になる。
/// **切り替えの後の行**（ADR-0058 §決定3）は本文の面の `end_token_bias` を**出た順の列**で見る。
#[tokio::test]
async fn arm8_switching_follows_the_end_token_bias_even_when_endpoint_and_model_stay() {
    let _serial = SERIAL.lock().await;
    let log = install_global_log("personacore=info");
    let mark = log.mark();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let shared = upstream().await;
    let uri = shared.uri();
    let source = json!({
        "lowered-a": { "main": main_face(&uri, Some(-3.0)) },
        "lowered-b": { "main": main_face(&uri, Some(-1.5)) },
        "plain": { "main": main_face(&uri, None) },
    });
    let path = dir.path().join("upstreams.json");
    std::fs::write(&path, source.to_string()).expect("プロファイルを置けること");
    let running = Running::start(
        &config_of(&source, "lowered-a", &dir.path().join("arm8.sqlite")),
        Some(path),
        Some(target(17)),
    )
    .await;
    let session = running.session().await;

    let mut before = 0;
    for (turn, switch, expected) in [
        (1, None, None),
        (2, None, wire(-3.0)),
        (3, Some("lowered-b"), wire(-1.5)),
        (4, Some("plain"), None),
        (5, Some("lowered-a"), wire(-3.0)),
    ] {
        if let Some(name) = switch {
            running.switch_to(name).await;
        }
        running.turn(session, &shared, turn).await;
        let all = seen(&shared).await;
        assert_eq!(
            body_bias_since(&all, before),
            expected,
            "{turn} 往復目（{switch:?}）の本文が切り替え先の値でない"
        );
        assert!(
            biases_of(&all[before..], Kind::Background)
                .iter()
                .all(Option::is_none),
            "{turn} 往復目の裏方に載った"
        );
        before = all.len();
    }

    let output = log.contents_since(mark);
    let said: Vec<(Option<&str>, Option<&str>)> = output
        .lines()
        .filter(|line| {
            line.contains(SAMPLERS_LINE)
                && has_field(line, "upstream_face=main")
                && has_field(line, "occasion=\"switch\"")
        })
        .map(|line| {
            (
                token_value(line, "end_token_bias"),
                bracketed_field(line, "overridden"),
            )
        })
        .collect();
    let lowered = r#"["end_token_bias"]"#;
    assert_eq!(
        said,
        vec![
            (Some(r#"{"248046":-1.5}"#), Some(lowered)),
            (Some("none"), Some("[]")),
            (Some(r#"{"248046":-3.0}"#), Some(lowered)),
        ],
        "切り替えの後に、本文の面の end_token_bias を切り替え先の値で順に言っていない"
    );
    running.stop().await;
}

/// **その名前の欄の値**（`キー=値` のトークンの `値`。最初の1つ）。
fn token_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let key = format!("{key}=");
    line.split_whitespace()
        .find_map(|token| token.strip_prefix(key.as_str()))
}

/// **空の本文の引き直しも、元の要求と同じ `logit_bias` で送る**（ADR-0066 §決定2 / ADR-0059 §決定3）。
///
/// 目安の下限は 10。種の短い返答（5 字）のある会話で1往復流す。偽の上流は**本文の依頼にだけ、最初の1回は空**を返す
/// （裏方は今までどおり [`REPLY`]）。**引き直しの要求が載せ忘れる実装・別の値で組み直す実装は、ここで赤になる。**
#[tokio::test]
async fn an_empty_body_is_retried_with_the_same_logit_bias() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    // **ふだんの偽の上流**（[`upstream`]）に、本文の最初の1回だけ空を返す口を、先に当たる優先度で足す。
    let server = upstream().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .and(matchers::body_string_contains(PERSONA_HEADING))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(upstream_body(""), "application/json"),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    let source = json!({ "retry": { "main": main_face(&server.uri(), Some(-3.0)) } });
    let running = Running::start(
        &config_of(&source, "retry", &dir.path().join("retry.sqlite")),
        None,
        Some(target(10)),
    )
    .await;
    let session = running.session().await;
    running.seed_turn(session, SHORT_REPLY).await;
    running.turn(session, &server, 1).await;
    let all = seen(&server).await;
    assert_eq!(
        biases_of(&all, Kind::Body),
        vec![wire(-3.0), wire(-3.0)],
        "空の本文の引き直しが元と同じ値で送られていない"
    );
    running.stop().await;
}
