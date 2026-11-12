//! **上流プロファイルの本文の面に書いた思考の量（`sampling.reasoning_effort`）と返答の上限（`response_tokens`）**
//! （[ADR-0059](../docs/adr/0059-thinking-and-response-room-in-the-upstream-profile.md) / 作業単位 A-18a）が、
//! **本番の入口（`App::bind` → `App::serve`）と走ったままの切り替えを通って**、本文の要求にだけ効くことの検証。
//! **実際の上流には繋がない**（`wiremock` の偽物）。
//!
//! # 何を見るか（ADR-0059 §決定4 の表の腕。腕5（空の本文の引き直し）は A-18b）
//!
//! | 見張り番 | 腕 | 観測すること |
//! |---|---|---|
//! | [`arm1_the_main_faces_reasoning_effort_reaches_the_body_request`] | 1 | 本文の面の `reasoning_effort` が本文の要求に載る（裏方の上流へは `none`） |
//! | [`arm2_without_a_sub_face_the_background_still_sends_none`] | 2 | `sub` 面の無い構成で、本文に `low` を書いても裏方（抽出・提案）は `none`。**経路と値を対にして見る** |
//! | [`arm3_the_response_room_reaches_the_body_max_tokens_and_budget_but_not_the_background`] | 3 | 本文の面の `response_tokens` が本文の `max_tokens` と予算の余白に効き、裏方の上限は変わらない（クライアントは `max_tokens` を送らない） |
//! | [`arm4_switching_follows_the_body_settings_even_when_endpoint_and_model_stay`] | 4 | 宛先・モデルが同じで値だけ違うプロファイルへの切り替えにも追従する（予算の余白も）。裏方の面は触らない |
//! | [`arm4_with_a_sub_face_a_body_only_switch_leaves_the_background_face_alone`] | 4 | `sub` 面を書いた構成で、本文の面の値だけが違う切り替えは裏方の面を触らない |
//! | [`arm6_a_queried_window_not_larger_than_the_room_falls_back_and_recovers`] | 6 | 照会した窓が `response_tokens` 以下なら既定の 1024 に戻して `warn`。照会し直して窓が戻れば面の値に戻る |
//! | [`arm7_the_environment_window_carries_the_room_at_startup_and_on_switching`] | （§決定2） | 環境変数の窓にも面の上限が載る（起動・切り替え）。収まらない上限への切り替えは 422 |
//! | [`arm8_an_unknown_window_keeps_the_default_room_until_the_window_is_known`] | （§決定2） | 窓が分からない間は面の上限を使わず既定の 1024。窓が分かったら面の値に戻る |
//! | [`arm9_switching_to_an_upstream_that_does_not_answer_the_window_keeps_the_default_room`] | （§決定2） | 窓を答えない上流への切り替えでも面の上限を使わず既定の 1024（切り替えの送る値の行は `window_unknown`） |
//!
//! 変容（`drift`）の経路の帰属は `tests/server_stat_drift.rs` の `each_path_sends_its_own_reasoning_effort` が見る
//! （ここでは変容は既定で off）。
//!
//! # 固定データの値
//!
//! **既定（`none`・1024）とも互いとも違える**（`low`・2000・3000・5000・6000）。腕4は宛先とモデルの綴りを揃える ——
//! 揃えないと、値を比べない実装でも切り替えが起きて緑になる。
//!
//! # 1本ずつ走らせる
//!
//! ログはバイナリで1つなので、どの腕も [`SERIAL`] を取る（腕4・腕6・起動のログの列に、別の腕の行が混ざらないように）。
//!
//! # 守っていないもの（先に書く）
//!
//! * **`main` の結線**（`config_for` が `with_profile_context` を呼ぶこと）は bin なので、ここからは見えない ——
//!   ここは `main` と同じ段を自分で組む（A-16 の `tests/server_profile_sampling.rs` と同じ）。`with_profile_context` 自体は
//!   A-13a-1 から `main` が呼んでいる（`tests/upstream_cli.rs` の窓の腕）。
//! * **思考が上流で効いたか**は見ない（送った値まで / ADR-0059 §決定1）。

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use personacore::domain::{DisplayName, Handle, Persona};
use personacore::server::{
    App, AppConfig, BackgroundProbe, BindAddr, ENV_CONTEXT_TOKENS, ENV_LLM_ENDPOINT,
    ENV_MAIN_MODEL, ENV_SUB_MODEL, ServerError, SharedBudget, overlay_profile, parse_profiles,
};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

use common::{fixture_scale, has_field, install_global_log};

/// **腕を1本ずつ走らせる**（モジュールの doc）。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 本文に指定するモデル。
const MAIN_MODEL: &str = "thinking-main-model";
/// 裏方に指定するモデル。**本文用と別の綴り。**
const SUB_MODEL: &str = "thinking-sub-model";

/// 会話するキャラクター（`model` に入れる値）。
const HANDLE: &str = "aoi";
/// 呼び名。
const DISPLAY_NAME: &str = "葵";
/// 利用者の発言。
const USER_MESSAGE: &str = "ただいま";

/// **本文の依頼だけに現れる見出し**（`prompt` の定数を呼ばない）。
const PERSONA_HEADING: &str = "# ペルソナ";

/// **送るサンプラの行**（`server::context::SAMPLERS_LINE` の写し）。
const SAMPLERS_LINE: &str = "上流へ送るサンプラの値（面ごと）";
/// **返答の上限を既定に戻したと言う行**（`server::context::RESPONSE_FALLBACK_LINE` の写し）。
const RESPONSE_FALLBACK_LINE: &str = "窓が本文の返答の上限以下なので、返答の上限を既定に戻します";
/// **窓が分からないので返答の上限を既定にしたと言う行**（`server::context::RESPONSE_UNKNOWN_WINDOW_LINE` の写し）。
const RESPONSE_UNKNOWN_WINDOW_LINE: &str =
    "上流のコンテキスト長が分からないので、窓が分かるまで返答の上限を既定にします";

/// 上流が答える窓（既定の 8192 と違える）。
const ROOMY_WINDOW: u32 = 16_384;

/// 待つ上限。**超えたら落とす。**
const WAIT_LIMIT: Duration = Duration::from_secs(20);
/// 待ちの刻み。
const POLL_STEP: Duration = Duration::from_millis(10);

/// 上流が返す本文（OpenAI 互換）。**抽出として読める行**を1件だけ含む。
fn upstream_body() -> String {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": MAIN_MODEL,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "- 利用者: 山あいの町で育った"},
            "finish_reason": "stop",
        }],
    })
    .to_string()
}

/// **その上流に、チャットと窓の照会の答えを載せる**（窓は `window`。2つのモデルとも同じ窓を答える）。
async fn mount(server: &MockServer, window: u32) {
    mount_chat(server).await;
    mount_models(server, window).await;
}

/// **その上流に、チャットの答えだけを載せる**（窓の照会は 404 —— 窓が分からない上流）。
async fn mount_chat(server: &MockServer) {
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(upstream_body(), "application/json"))
        .mount(server)
        .await;
}

/// **その上流に、窓の照会の答えだけを載せる**（窓は `window`）。
async fn mount_models(server: &MockServer, window: u32) {
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(
                r#"{{"object":"list","data":[
                  {{"id":"{MAIN_MODEL}","object":"model","type":"llm","state":"loaded",
                    "max_context_length":262144,"loaded_context_length":{window}}},
                  {{"id":"{SUB_MODEL}","object":"model","type":"llm","state":"loaded",
                    "max_context_length":262144,"loaded_context_length":{window}}}]}}"#
            ),
            "application/json",
        ))
        .mount(server)
        .await;
}

/// **本文と裏方の両方を受けられる上流**（窓は `window`）。
async fn upstream(window: u32) -> MockServer {
    let server = MockServer::start().await;
    mount(&server, window).await;
    server
}

/// 上流へ届いた依頼の種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// 本文。
    Body,
    /// 抽出。
    Extraction,
    /// 約束・決まりの提案。
    Proposal,
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
    match (extraction, proposal, messages.contains(PERSONA_HEADING)) {
        (true, false, _) => Kind::Extraction,
        (false, true, _) => Kind::Proposal,
        (false, false, true) => Kind::Body,
        _ => panic!("依頼の種類が決まらない: {messages}"),
    }
}

/// **上流へ届いた1件のチャットの依頼**（種類と、見る2つの欄）。
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    kind: Kind,
    reasoning_effort: Value,
    max_tokens: Value,
}

/// **その上流へ届いたチャットの依頼**（届いた順）。
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
                reasoning_effort: body["reasoning_effort"].clone(),
                max_tokens: body["max_tokens"].clone(),
            }
        })
        .collect()
}

/// **種類ごとの、その欄の並び**（重複を畳まない）。
fn of_kind(seen: &[Seen], kind: Kind, field: impl Fn(&Seen) -> Value) -> Vec<Value> {
    seen.iter()
        .filter(|one| one.kind == kind)
        .map(field)
        .collect()
}

/// **裏方の依頼を、経路の名前と値の対で並べる**（`each_path_sends_its_own_temperature` と同じ形）。
fn background_pairs(seen: &[Seen], field: impl Fn(&Seen) -> Value) -> Vec<(Kind, Value)> {
    seen.iter()
        .filter(|one| one.kind != Kind::Body)
        .map(|one| (one.kind, field(one)))
        .collect()
}

/// その上流へ届いた `GET`（窓の照会）の件数。**面を触ると、切り替えがその場で照会する**（`switch::execute` の 4.）
/// —— 触らなければ増えない（`tests/server_profile_sampling.rs` の `probe_count` と同じ）。
async fn probe_count(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.method.as_str() == "GET")
        .count()
}

/// **その名前の欄の値**（`キー=値` のトークンの `値`。最初の1つ / `tests/server_profile_sampling.rs` と同じ）。
fn token_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let key = format!("{key}=");
    line.split_whitespace()
        .find_map(|token| token.strip_prefix(key.as_str()))
}

/// **送る値の行のうち、その面・その場面のもの**（出た順）。
fn samplers_lines<'a>(output: &'a str, face: &str, occasion: &str) -> Vec<&'a str> {
    output
        .lines()
        .filter(|line| {
            line.contains(SAMPLERS_LINE)
                && has_field(line, &format!("upstream_face={face}"))
                && has_field(line, &format!("occasion=\"{occasion}\""))
        })
        .collect()
}

/// **面の1つ**（`extra` の鍵をそのまま足す）。
fn face(endpoint: &str, model: &str, extra: &Value) -> Value {
    let mut face = json!({ "endpoint": endpoint, "model": model });
    if let (Some(face), Some(extra)) = (face.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            face.insert(key.clone(), value.clone());
        }
    }
    face
}

/// **`main` と同じ段で設定を組む**（`overlay_profile` → `with_profile_context` → `with_profile_keys` →
/// `with_profile_sampling`）。
fn config_of(source: &Value, name: &str, db_path: &Path) -> AppConfig {
    config_with_environment_window(source, name, db_path, None)
}

/// **[`config_of`] に `PERSONACORE_CONTEXT_TOKENS`（環境変数の窓）を足したもの**（`None` なら置かない）。
fn config_with_environment_window(
    source: &Value,
    name: &str,
    db_path: &Path,
    environment_window: Option<u32>,
) -> AppConfig {
    let mut pairs: Vec<(String, String)> = vec![
        // **囮**: プロファイルが勝つので、ここへは何も届かない。
        (ENV_LLM_ENDPOINT.to_owned(), "http://127.0.0.1:9".to_owned()),
        (ENV_MAIN_MODEL.to_owned(), "decoy-main".to_owned()),
        (ENV_SUB_MODEL.to_owned(), "decoy-sub".to_owned()),
    ];
    if let Some(window) = environment_window {
        pairs.push((ENV_CONTEXT_TOKENS.to_owned(), window.to_string()));
    }
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

/// **`App::bind`（本番の入口）で起動し、走らせる。**
struct Running {
    port: u16,
    background: BackgroundProbe,
    /// **本文の予算の器**（`App::budget_handle`。予算の余白に効いたかを見る）。
    budget: SharedBudget,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
}

impl Running {
    async fn start(config: &AppConfig, profiles: Option<&Path>) -> Self {
        let mut app = App::bind(config).await.expect("待ち受けを開始できること");
        if let Some(path) = profiles {
            app = app.with_profiles_file_for_tests(path.to_path_buf());
        }
        app.store()
            .create_character(
                &Handle::parse(HANDLE).expect("正当な名前であること"),
                &DisplayName::parse(DISPLAY_NAME).expect("正当な表示名であること"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");
        let port = app.local_addr().port();
        let background = app.background_probe();
        let budget = app.budget_handle();
        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });
        Self {
            port,
            background,
            budget,
            shutdown: Some(shutdown),
            handle: Some(handle),
        }
    }

    /// **1往復させ（クライアントは `max_tokens` を送らない）、そのターンの裏方が終わるまで待つ。**
    ///
    /// 「A が B になる」を待つ形: 先に `servers` に届いた抽出の依頼が `extractions` 件になるのを待ち、
    /// 次に裏方の枠が空くのを待つ（`tests/server_profile_sampling.rs` と同じ）。
    async fn turn(&self, servers: &[&MockServer], extractions: usize) {
        let response = reqwest::Client::new()
            .post(format!(
                "http://127.0.0.1:{}/v1/chat/completions",
                self.port
            ))
            .json(&json!({
                "model": HANDLE,
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
            let mut count = 0;
            for server in servers {
                count += seen(server)
                    .await
                    .iter()
                    .filter(|one| one.kind == Kind::Extraction)
                    .count();
            }
            if count >= extractions {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "抽出の依頼が {extractions} 件に届かない（いま {count} 件）"
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
        self.post_ok("/admin/upstream/profile", &json!({ "name": name }))
            .await;
    }

    /// **`POST /admin/upstream/check`**（両方の面をいま照会し直す）。200 でなければ落とす。
    async fn check_now(&self) {
        self.post_ok("/admin/upstream/check", &json!({})).await;
    }

    async fn post_ok(&self, path: &str, body: &Value) {
        let (status, text) = self.post(path, body).await;
        assert_eq!(status, 200, "{path}: {text}");
    }

    /// **依頼して、状態と本文を返す**（断られる腕が状態を見る）。
    async fn post(&self, path: &str, body: &Value) -> (u16, String) {
        let response = reqwest::Client::new()
            .post(format!("http://127.0.0.1:{}{path}", self.port))
            .json(body)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .expect("依頼できること");
        let status = response.status().as_u16();
        let text = response.text().await.expect("本文を読めること");
        (status, text)
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

/// **1つの腕を走らせ、本文と裏方の上流が受けた依頼と、いまの予算の余白を返す**（1往復）。
///
/// `sub` が `None` なら `sub` 面を書かない（裏方は本文と同じ上流へ行く）。
async fn one_turn(arm: &str, main_extra: &Value, sub: bool) -> (Vec<Seen>, Vec<Seen>, u32) {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let main_upstream = upstream(ROOMY_WINDOW).await;
    let sub_upstream = upstream(ROOMY_WINDOW).await;
    let mut profile = json!({ "main": face(&main_upstream.uri(), MAIN_MODEL, main_extra) });
    if sub {
        profile["sub"] = face(&sub_upstream.uri(), SUB_MODEL, &json!({}));
    }
    let source = json!({ arm: profile });
    let running = Running::start(
        &config_of(&source, arm, &dir.path().join("arm.sqlite")),
        None,
    )
    .await;
    running.turn(&[&main_upstream, &sub_upstream], 1).await;
    let to_main = seen(&main_upstream).await;
    let to_sub = seen(&sub_upstream).await;
    let room = running.budget.response_tokens_for_tests();
    running.stop().await;
    (to_main, to_sub, room)
}

/// **腕1: 本文の面に書いた `reasoning_effort` が本文の要求に載る**（ADR-0059 §決定1・§決定4）。
///
/// `sub` 面は別の上流 —— **本文の要求は本文の上流にだけ届いて `low` を持ち、裏方の上流へは `none` が届く。**
/// **起動のログ**（ADR-0059 §決定1 の「送る値のログ」）も見る: 本文の面の行が `reasoning_effort=low` と
/// `overridden` に `reasoning_effort` を言い、裏方の面の行は `none`。
#[tokio::test]
async fn arm1_the_main_faces_reasoning_effort_reaches_the_body_request() {
    let _serial = SERIAL.lock().await;
    let log = install_global_log("personacore=info");
    let mark = log.mark();
    let (to_main, to_sub, _) = one_turn(
        "arm1",
        &json!({ "sampling": { "reasoning_effort": "low" } }),
        true,
    )
    .await;
    assert_eq!(
        of_kind(&to_main, Kind::Body, |one| one.reasoning_effort.clone()),
        vec![json!("low")],
        "本文の要求が本文の面の値を持たない: {to_main:?}"
    );
    assert!(
        background_pairs(&to_main, |one| one.reasoning_effort.clone()).is_empty(),
        "前提: 裏方は本文の上流へ行かない: {to_main:?}"
    );
    let background = background_pairs(&to_sub, |one| one.reasoning_effort.clone());
    assert!(
        !background.is_empty(),
        "前提: 裏方が裏方の上流へ届いた: {to_sub:?}"
    );
    assert!(
        background.iter().all(|(_, value)| *value == json!("none")),
        "裏方の要求が none でない: {background:?}"
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
    assert_eq!(
        main_lines.len(),
        1,
        "起動の本文の面の行は1本: {main_lines:?}"
    );
    assert!(
        has_field(&main_lines[0], "reasoning_effort=low")
            && main_lines[0].contains(r#"overridden=["reasoning_effort"]"#),
        "起動の行が本文の面の思考の量を言っていない: {}",
        main_lines[0]
    );
    let sub_lines = startup("sub");
    assert_eq!(sub_lines.len(), 1, "起動の裏方の面の行は1本: {sub_lines:?}");
    assert!(
        has_field(&sub_lines[0], "reasoning_effort=none"),
        "裏方の面の行は none を言うこと: {}",
        sub_lines[0]
    );
}

/// **腕2: `sub` 面の無い構成で、本文の面に `low` を書いても、裏方の要求は `none`**（ADR-0059 §決定1）。
///
/// 裏方は本文のクライアントの複製（同じ宛先・同じモデル）で走る —— **面の設定に思考の量を載せる実装は、ここで
/// 抽出と提案が `low` になって赤になる。** 経路と値は対にして並べる（どの経路が何を送ったかが失敗に出る）。
/// **対照**: 本文は `low`（書いた値が本文には届いている＝「何も送っていないから none」ではない）。
#[tokio::test]
async fn arm2_without_a_sub_face_the_background_still_sends_none() {
    let _serial = SERIAL.lock().await;
    let (to_main, to_sub, _) = one_turn(
        "arm2",
        &json!({ "sampling": { "reasoning_effort": "low" } }),
        false,
    )
    .await;
    assert!(
        to_sub.is_empty(),
        "前提: 裏方の上流は使われない: {to_sub:?}"
    );
    assert_eq!(
        of_kind(&to_main, Kind::Body, |one| one.reasoning_effort.clone()),
        vec![json!("low")],
        "{to_main:?}"
    );
    let background = background_pairs(&to_main, |one| one.reasoning_effort.clone());
    assert!(
        background.iter().any(|(kind, _)| *kind == Kind::Extraction),
        "前提: 抽出が届いた: {background:?}"
    );
    let expected: Vec<(Kind, Value)> = background
        .iter()
        .map(|(kind, _)| (*kind, json!("none")))
        .collect();
    assert_eq!(
        background, expected,
        "sub 面が無いのに、本文の思考の量が裏方へ写った"
    );
}

/// **腕3: 本文の面の `response_tokens` が本文の `max_tokens` と予算の余白に効き、裏方の上限は変わらない**
/// （ADR-0059 §決定2・§決定4）。**クライアントは `max_tokens` を送らない**（＝余白いっぱいが送られる）。
///
/// | 腕 | 本文の `max_tokens` | 予算の余白 |
/// |---|---|---|
/// | 対照（何も書かない） | 1024 | 1024 |
/// | 窓なし・上限 2000（照会した窓 16384） | **2000** | **2000** |
/// | 窓 8192・上限 3000（明示の窓） | **3000** | **3000** |
///
/// **裏方の `max_tokens` は、対照と同じ並び**（経路の名前と対で比べる。期待値を実装の定数から作らない）。
/// 予算の余白は `App::budget_handle` から読む —— `max_tokens` だけを面の値にして予算を 1024 のままにする実装を赤にする。
///
/// **起動の送る値の行**も見る（A-18a のレビュー是正）: 3つとも `response_tokens` が上の余白、
/// `response_tokens_fallback=false`・`response_tokens_fallback_cause=none`（戻していない行。腕6・腕8 の `true` の行と対）。
#[tokio::test]
async fn arm3_the_response_room_reaches_the_body_max_tokens_and_budget_but_not_the_background() {
    let _serial = SERIAL.lock().await;
    let log = install_global_log("personacore=info");
    let said = |mark: usize, expected: u32| {
        let output = log.contents_since(mark);
        let lines = samplers_lines(&output, "main", "startup");
        assert_eq!(lines.len(), 1, "起動の本文の面の行は1本: {lines:?}");
        assert_eq!(
            (
                token_value(lines[0], "response_tokens"),
                token_value(lines[0], "response_tokens_fallback"),
                token_value(lines[0], "response_tokens_fallback_cause"),
            ),
            (
                Some(expected.to_string().as_str()),
                Some("false"),
                Some("none")
            ),
            "起動の行が戻していない余白を言っていない: {}",
            lines[0]
        );
    };
    let mark = log.mark();
    let (control_main, control_sub, control_room) =
        one_turn("arm3-control", &json!({}), true).await;
    said(mark, 1024);
    assert_eq!(
        of_kind(&control_main, Kind::Body, |one| one.max_tokens.clone()),
        vec![json!(1024)],
        "前提: 書かなければ 1024"
    );
    assert_eq!(control_room, 1024, "前提: 書かなければ余白は 1024");
    let control_background = background_pairs(&control_sub, |one| one.max_tokens.clone());
    assert!(
        !control_background.is_empty(),
        "前提: 裏方が届いた: {control_sub:?}"
    );

    for (arm, extra, expected) in [
        ("arm3-queried", json!({ "response_tokens": 2000 }), 2000),
        (
            "arm3-explicit",
            json!({ "response_tokens": 3000, "context_tokens": 8192 }),
            3000,
        ),
    ] {
        let mark = log.mark();
        let (to_main, to_sub, room) = one_turn(arm, &extra, true).await;
        said(mark, expected);
        assert_eq!(
            of_kind(&to_main, Kind::Body, |one| one.max_tokens.clone()),
            vec![json!(expected)],
            "{arm}: 本文の max_tokens が面の上限でない: {to_main:?}"
        );
        assert_eq!(room, expected, "{arm}: 予算の余白が面の上限でない");
        assert_eq!(
            background_pairs(&to_sub, |one| one.max_tokens.clone()),
            control_background,
            "{arm}: 裏方の上限が変わった"
        );
    }
}

/// **腕4: 走ったままの切り替えで、宛先とモデルが同じで値だけ違うプロファイルへも追従する（予算の余白も）**
/// （ADR-0059 §決定4 の腕4）。
///
/// | 回 | プロファイル | 本文の `reasoning_effort` | 本文の `max_tokens` = 予算の余白 |
/// |---|---|---|---|
/// | 1 | `plain`（何も書かない） | `none` | 1024 |
/// | 2 | `thinking`（`low`・2000） | `low` | 2000 |
/// | 3 | `roomy`（上限 3000 だけ） | **`none`（持ち越さない）** | 3000 |
/// | 4 | `effort-only`（`low` だけ） | `low` | **1024（持ち越さない）** |
/// | 5 | `plain` | `none` | 1024 |
///
/// **5つとも宛先・モデルが同じ綴り**（`sub` 面も書かない）。裏方は毎回 `none`（`sub` 面が無くても写さない）。
/// **切り替えの後のログ**も、本文の面の行の `reasoning_effort` と `response_tokens` を**出た順の列で、値で**見る
/// （A-18a のレビュー是正。切り替えのログは外の上流を照会し直した後に出るので、窓 16384 の上の面の値を言う）。
///
/// **裏方の面は触らない**（A-18a のレビュー是正）: `sub` 面の無い構成では、裏方の比べる値は本文の候補の複製だが、
/// **本文の面の値は既定と比べる**（`switch::prepare` の `next_sub`）。本文の面の値を裏方の比べる値へ渡す実装は、
/// 切り替えのたびに裏方の面まで触り、その場で照会し直す —— **切り替え1回あたりの照会（`GET`）が 1 から 2 になる。**
#[tokio::test]
async fn arm4_switching_follows_the_body_settings_even_when_endpoint_and_model_stay() {
    let _serial = SERIAL.lock().await;
    let log = install_global_log("personacore=info");
    let mark = log.mark();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let shared = upstream(ROOMY_WINDOW).await;
    let uri = shared.uri();
    let source = json!({
        "plain": { "main": face(&uri, MAIN_MODEL, &json!({})) },
        "thinking": { "main": face(&uri, MAIN_MODEL, &json!({
            "sampling": { "reasoning_effort": "low" }, "response_tokens": 2000 })) },
        "roomy": { "main": face(&uri, MAIN_MODEL, &json!({ "response_tokens": 3000 })) },
        "effort-only": { "main": face(&uri, MAIN_MODEL, &json!({
            "sampling": { "reasoning_effort": "low" } })) },
    });
    let path = dir.path().join("upstreams.json");
    std::fs::write(&path, source.to_string()).expect("プロファイルを置けること");
    let running = Running::start(
        &config_of(&source, "plain", &dir.path().join("arm4.sqlite")),
        Some(&path),
    )
    .await;

    let mut before = 0;
    for (turn, switch, effort, room) in [
        (1, None, "none", 1024),
        (2, Some("thinking"), "low", 2000),
        (3, Some("roomy"), "none", 3000),
        (4, Some("effort-only"), "low", 1024),
        (5, Some("plain"), "none", 1024),
    ] {
        if let Some(name) = switch {
            let probes = probe_count(&shared).await;
            running.switch_to(name).await;
            assert_eq!(
                probe_count(&shared).await - probes,
                1,
                "{turn} 回目（{switch:?}）: 照会し直すのは本文の面だけ（裏方の面は触らない）"
            );
        }
        running.turn(&[&shared], turn).await;
        let all = seen(&shared).await;
        let this_turn = &all[before..];
        before = all.len();
        assert_eq!(
            of_kind(this_turn, Kind::Body, |one| one.reasoning_effort.clone()),
            vec![json!(effort)],
            "{turn} 回目（{switch:?}）の本文の思考の量が切り替え先の値でない: {this_turn:?}"
        );
        assert_eq!(
            of_kind(this_turn, Kind::Body, |one| one.max_tokens.clone()),
            vec![json!(room)],
            "{turn} 回目（{switch:?}）の本文の max_tokens が切り替え先の上限でない: {this_turn:?}"
        );
        assert_eq!(
            running.budget.response_tokens_for_tests(),
            room,
            "{turn} 回目（{switch:?}）の予算の余白が切り替え先の上限でない"
        );
        let background = background_pairs(this_turn, |one| one.reasoning_effort.clone());
        assert!(!background.is_empty(), "前提: {turn} 回目の裏方が届いた");
        assert!(
            background.iter().all(|(_, value)| *value == json!("none")),
            "{turn} 回目の裏方が none でない: {background:?}"
        );
    }

    // **切り替えの後に言う本文の面の値**（出た順の列 / 同じ行の中で、値で読む）。
    let output = log.contents_since(mark);
    let said: Vec<[Option<&str>; 4]> = samplers_lines(&output, "main", "switch")
        .into_iter()
        .map(|line| {
            [
                token_value(line, "reasoning_effort"),
                token_value(line, "response_tokens"),
                token_value(line, "response_tokens_fallback"),
                token_value(line, "response_tokens_fallback_cause"),
            ]
        })
        .collect();
    let expected: Vec<[Option<&str>; 4]> = [
        ["low", "2000"],
        ["none", "3000"],
        ["low", "1024"],
        ["none", "1024"],
    ]
    .iter()
    .map(|[effort, room]| [Some(*effort), Some(*room), Some("false"), Some("none")])
    .collect();
    assert_eq!(
        said, expected,
        "切り替えの後に、本文の面の思考の量と上限を切り替え先の値で順に言っていない"
    );

    running.stop().await;
}

/// **腕4（`sub` 面を書いた構成）: 本文の面の値だけが違うプロファイルへの切り替えは、裏方の面を触らない**
/// （A-18a のレビュー是正 / `switch::prepare` の `next_sub` の `body_of(ModelFace::Sub, …)`）。
///
/// 2つのプロファイルは `sub` 面が同じ綴りで、本文の面だけ `thinking`（`low`・2000）と `plain` が違う。
/// **裏方の上流への照会（`GET`）は切り替えで増えない**（触った面だけがその場で照会し直す）。
/// 本文の面の値を裏方の比べる値へ渡す実装は、ここで裏方の上流への照会が 1 増えて赤になる。
/// **対照**: 本文の上流への照会は 1 増える（切り替え自体は起きた）。
#[tokio::test]
async fn arm4_with_a_sub_face_a_body_only_switch_leaves_the_background_face_alone() {
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let main_upstream = upstream(ROOMY_WINDOW).await;
    let sub_upstream = upstream(ROOMY_WINDOW).await;
    let sub = face(&sub_upstream.uri(), SUB_MODEL, &json!({}));
    let source = json!({
        "plain": { "main": face(&main_upstream.uri(), MAIN_MODEL, &json!({})), "sub": sub.clone() },
        "thinking": {
            "main": face(&main_upstream.uri(), MAIN_MODEL, &json!({
                "sampling": { "reasoning_effort": "low" }, "response_tokens": 2000 })),
            "sub": sub,
        },
    });
    let path = dir.path().join("upstreams.json");
    std::fs::write(&path, source.to_string()).expect("プロファイルを置けること");
    let running = Running::start(
        &config_of(&source, "plain", &dir.path().join("arm4b.sqlite")),
        Some(&path),
    )
    .await;

    for (name, room) in [("thinking", 2000), ("plain", 1024)] {
        let main_before = probe_count(&main_upstream).await;
        let sub_before = probe_count(&sub_upstream).await;
        running.switch_to(name).await;
        assert_eq!(
            (
                probe_count(&main_upstream).await - main_before,
                probe_count(&sub_upstream).await - sub_before,
            ),
            (1, 0),
            "{name}: 本文の面だけを照会し直すこと（裏方の面は触らない）"
        );
        assert_eq!(running.budget.response_tokens_for_tests(), room, "{name}");
    }

    running.stop().await;
}

/// **腕7: 環境変数の窓（`PERSONACORE_CONTEXT_TOKENS`）にも、本文の面の返答の上限が載る —— 起動でも切り替えでも**
/// （ADR-0059 §決定2 / A-18a のレビュー是正）。環境変数の窓は 12288、プロファイルはどれも窓を書かない。
///
/// | 段 | プロファイル | 期待 | 守るもの |
/// |---|---|---|---|
/// | 起動 | `env-room`（上限 3000） | 本文の `max_tokens` も予算の余白も **3000**、窓 12288 | `App::bind` が `explicit_main_budget` に本文の面を渡す |
/// | 切り替え | `plain`（何も書かない） | 1024 | 対照（書かなければ既定） |
/// | 切り替え | `env-room` | **3000** | `install_face` が `explicit_main_budget` に切り替え先の本文の面を渡す |
/// | 切り替え | `too-big`（上限 12288） | **422 `upstream_profile_unusable`**、予算は動かない（3000） | `switch::prepare` が `ProfileContext::of` に環境変数の窓を渡す |
///
/// 「上限 12288」は読み込みでは止まらない（面に窓が無いので、読み込みは環境変数を見ていない）—— 止めるのは切り替えの検証だけ。
#[tokio::test]
async fn arm7_the_environment_window_carries_the_room_at_startup_and_on_switching() {
    const ENVIRONMENT_WINDOW: u32 = 12_288;
    let _serial = SERIAL.lock().await;
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let main_upstream = upstream(ROOMY_WINDOW).await;
    let sub_upstream = upstream(ROOMY_WINDOW).await;
    let sub = face(&sub_upstream.uri(), SUB_MODEL, &json!({}));
    let main = |extra: Value| face(&main_upstream.uri(), MAIN_MODEL, &extra);
    let source = json!({
        "env-room": { "main": main(json!({ "response_tokens": 3000 })), "sub": sub.clone() },
        "plain": { "main": main(json!({})), "sub": sub.clone() },
        "too-big": { "main": main(json!({ "response_tokens": ENVIRONMENT_WINDOW })), "sub": sub },
    });
    let path = dir.path().join("upstreams.json");
    std::fs::write(&path, source.to_string()).expect("プロファイルを置けること");
    let running = Running::start(
        &config_with_environment_window(
            &source,
            "env-room",
            &dir.path().join("arm7.sqlite"),
            Some(ENVIRONMENT_WINDOW),
        ),
        Some(&path),
    )
    .await;

    // **起動**（変異 (a) を赤にする段）。
    running.turn(&[&sub_upstream], 1).await;
    assert_eq!(
        of_kind(&seen(&main_upstream).await, Kind::Body, |one| one
            .max_tokens
            .clone()),
        vec![json!(3000)],
        "起動: 環境変数の窓に本文の面の上限が載っていない"
    );
    assert_eq!(
        (
            running.budget.tokens_for_tests(),
            running.budget.response_tokens_for_tests()
        ),
        (ENVIRONMENT_WINDOW, 3000),
        "起動: 予算が環境変数の窓と面の上限でない"
    );

    // **切り替え**（対照 → 変異 (b) を赤にする段）。
    for (name, room) in [("plain", 1024), ("env-room", 3000)] {
        running.switch_to(name).await;
        assert_eq!(
            (
                running.budget.tokens_for_tests(),
                running.budget.response_tokens_for_tests()
            ),
            (ENVIRONMENT_WINDOW, room),
            "{name}: 切り替え先の面の上限が環境変数の窓に載っていない"
        );
    }

    // **窓に収まらない上限へは切り替えない**（変異 (c) を赤にする段）。
    let (status, text) = running
        .post("/admin/upstream/profile", &json!({ "name": "too-big" }))
        .await;
    assert_eq!(
        status, 422,
        "環境変数の窓に収まらない上限へ切り替えた: {text}"
    );
    assert!(
        text.contains("upstream_profile_unusable"),
        "断る理由が違う: {text}"
    );
    assert_eq!(
        (
            running.budget.tokens_for_tests(),
            running.budget.response_tokens_for_tests()
        ),
        (ENVIRONMENT_WINDOW, 3000),
        "断ったのに予算が動いた"
    );

    running.stop().await;
}

/// **腕6: 照会した窓が `response_tokens` 以下なら、既定の 1024 に戻して `warn` を出す。照会し直して窓が戻れば、
/// 面の値に戻る**（ADR-0059 §決定2 / §決定4 の腕6 —— `App::bind` を通る実行時の分岐）。
///
/// 本文の上流は窓 6000 を答え、本文の面の上限は 6000（窓と同じ＝上限以下）。裏方は別の上流（窓 16384）へ行く
/// （裏方が窓の小ささで飛ばないように）。
///
/// 1. 起動: 本文の `max_tokens` も予算の余白も **1024**。`warn` の行は窓・書いた上限・戻した余白を同じ行で言う。
///    起動の送る値の行は `response_tokens=1024 response_tokens_fallback=true`。
/// 2. 本文の上流の窓を 16384 に変え、`POST /admin/upstream/check` で照会し直す → **6000** に戻る。
#[tokio::test]
async fn arm6_a_queried_window_not_larger_than_the_room_falls_back_and_recovers() {
    let _serial = SERIAL.lock().await;
    let log = install_global_log("personacore=info");
    let mark = log.mark();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let main_upstream = upstream(6_000).await;
    let sub_upstream = upstream(ROOMY_WINDOW).await;
    let source = json!({ "arm6": {
        "main": face(&main_upstream.uri(), MAIN_MODEL, &json!({ "response_tokens": 6000 })),
        "sub": face(&sub_upstream.uri(), SUB_MODEL, &json!({})),
    } });
    let running = Running::start(
        &config_of(&source, "arm6", &dir.path().join("arm6.sqlite")),
        None,
    )
    .await;

    running.turn(&[&sub_upstream], 1).await;
    assert_eq!(
        of_kind(&seen(&main_upstream).await, Kind::Body, |one| one
            .max_tokens
            .clone()),
        vec![json!(1024)],
        "窓が上限以下なのに既定へ戻していない"
    );
    assert_eq!(running.budget.response_tokens_for_tests(), 1024);

    let output = log.contents_since(mark);
    let warned: Vec<&str> = output
        .lines()
        .filter(|line| line.contains(RESPONSE_FALLBACK_LINE))
        .collect();
    assert_eq!(warned.len(), 1, "戻したと言う行は1本: {warned:?}");
    assert!(warned[0].contains("WARN"), "警告であること: {}", warned[0]);
    for field in [
        "context_tokens=6000",
        "response_tokens_written=6000",
        "response_tokens=1024",
    ] {
        assert!(
            has_field(warned[0], field),
            "{field} を同じ行で言うこと: {}",
            warned[0]
        );
    }
    let startup: Vec<&str> = output
        .lines()
        .filter(|line| {
            line.contains(SAMPLERS_LINE)
                && has_field(line, "upstream_face=main")
                && has_field(line, "occasion=\"startup\"")
        })
        .collect();
    assert_eq!(startup.len(), 1, "{startup:?}");
    for field in [
        "response_tokens=1024",
        "response_tokens_written=Some(6000)",
        "response_tokens_fallback=true",
        // **戻した理由は窓の不足**（窓は分かっている / A-18a のレビュー是正。腕8 の `window_unknown` と対）。
        "response_tokens_fallback_cause=window_too_small",
    ] {
        assert!(
            has_field(startup[0], field),
            "起動の行が {field} を言っていない: {}",
            startup[0]
        );
    }
    assert!(
        !output.contains(RESPONSE_UNKNOWN_WINDOW_LINE),
        "窓は分かっているのに、分からないと言った"
    );

    // **窓が戻る**（上流を載せ直したのと同じ）。照会し直すと、面の値に戻る。
    main_upstream.reset().await;
    mount(&main_upstream, ROOMY_WINDOW).await;
    running.check_now().await;
    assert_eq!(
        running.budget.response_tokens_for_tests(),
        6000,
        "窓が戻ったのに面の上限へ戻していない"
    );
    running.turn(&[&sub_upstream], 2).await;
    assert_eq!(
        of_kind(&seen(&main_upstream).await, Kind::Body, |one| one
            .max_tokens
            .clone()),
        vec![json!(6000)],
        "照会し直した後の本文の max_tokens が面の上限でない"
    );

    running.stop().await;
}

/// **腕8: 窓が分からない（照会に失敗して落とし所の窓 8192 を使う）間は、面の上限を使わず既定の 1024。窓が分かったら
/// 面の値に戻る**（A-18a のレビュー是正 —— ADR-0059 §決定2 の「高い側は遅くなるだけ」は窓が分かっている前提）。
///
/// 本文の上流は窓の照会に答えない（404）。本文の面の上限は 2000。
///
/// 1. 起動: 本文の `max_tokens` も予算の余白も **1024**、窓は落とし所の 8192。`warn` は**窓が分からない**と言う行
///    （窓の不足の行は出ない）。起動の送る値の行は `response_tokens_fallback=true response_tokens_fallback_cause=window_unknown`。
/// 2. 本文の上流が窓 **8192**（落とし所と同じ数）を答えるようにして照会し直す → **2000** に戻る
///    （窓の数だけを比べて「同じだから採らない」実装を赤にする）。
#[tokio::test]
async fn arm8_an_unknown_window_keeps_the_default_room_until_the_window_is_known() {
    let _serial = SERIAL.lock().await;
    let log = install_global_log("personacore=info");
    let mark = log.mark();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let main_upstream = MockServer::start().await;
    mount_chat(&main_upstream).await;
    let sub_upstream = upstream(ROOMY_WINDOW).await;
    let source = json!({ "arm8": {
        "main": face(&main_upstream.uri(), MAIN_MODEL, &json!({ "response_tokens": 2000 })),
        "sub": face(&sub_upstream.uri(), SUB_MODEL, &json!({})),
    } });
    let running = Running::start(
        &config_of(&source, "arm8", &dir.path().join("arm8.sqlite")),
        None,
    )
    .await;

    running.turn(&[&sub_upstream], 1).await;
    assert_eq!(
        of_kind(&seen(&main_upstream).await, Kind::Body, |one| one
            .max_tokens
            .clone()),
        vec![json!(1024)],
        "窓が分からないのに面の上限を使った"
    );
    assert_eq!(
        (
            running.budget.tokens_for_tests(),
            running.budget.response_tokens_for_tests()
        ),
        (8_192, 1024),
        "窓が分からない間は落とし所の窓・既定の余白"
    );

    let output = log.contents_since(mark);
    assert!(
        !output.contains(RESPONSE_FALLBACK_LINE),
        "窓が分からないのに、窓の不足と言った"
    );
    let warned: Vec<&str> = output
        .lines()
        .filter(|line| line.contains(RESPONSE_UNKNOWN_WINDOW_LINE))
        .collect();
    assert_eq!(warned.len(), 1, "窓が分からないと言う行は1本: {warned:?}");
    assert!(warned[0].contains("WARN"), "警告であること: {}", warned[0]);
    for field in ["response_tokens_written=2000", "response_tokens=1024"] {
        assert!(
            has_field(warned[0], field),
            "{field} を同じ行で言うこと: {}",
            warned[0]
        );
    }
    let startup = samplers_lines(&output, "main", "startup");
    assert_eq!(startup.len(), 1, "{startup:?}");
    assert_eq!(
        (
            token_value(startup[0], "response_tokens"),
            token_value(startup[0], "response_tokens_fallback"),
            token_value(startup[0], "response_tokens_fallback_cause"),
        ),
        (Some("1024"), Some("true"), Some("window_unknown")),
        "起動の行が、窓が分からないので既定にしたと言っていない: {}",
        startup[0]
    );

    // **窓が分かる**（落とし所と同じ 8192）。照会し直すと、面の値に戻る。
    mount_models(&main_upstream, 8_192).await;
    running.check_now().await;
    assert_eq!(
        (
            running.budget.tokens_for_tests(),
            running.budget.response_tokens_for_tests()
        ),
        (8_192, 2000),
        "窓が分かったのに面の上限へ戻していない"
    );
    running.turn(&[&sub_upstream], 2).await;
    assert_eq!(
        of_kind(&seen(&main_upstream).await, Kind::Body, |one| one
            .max_tokens
            .clone()),
        vec![json!(1024), json!(2000)],
        "窓が分かった後の本文の max_tokens が面の上限でない"
    );

    running.stop().await;
}

/// **腕9: 窓を答えない上流への切り替えでも、面の上限を使わず既定の 1024**（A-18a の再レビュー是正 / `install_face` の
/// 窓が分からない枝）。
///
/// 起動は窓を答える上流（`plain`）。切り替え先 `unknown` の本文の上流は窓の照会に答えない（腕8 と同じ `mount_chat`
/// だけの上流）、本文の面の上限は 2000。切り替えはその場で照会し直すが窓は取れないので、**切り替えで入れた落とし所の
/// 値がそのまま残る** —— 落とし所で面の上限を使う実装は、ここで (8192, 2000) になって赤になる
/// （照会し直しが成功する上流への切り替えでは、落とし所の値が観測される前に上書きされて隠れる）。
///
/// 切り替えの送る値の行は、同じ行の中で `response_tokens_fallback=true` と `response_tokens_fallback_cause=window_unknown`。
#[tokio::test]
async fn arm9_switching_to_an_upstream_that_does_not_answer_the_window_keeps_the_default_room() {
    let _serial = SERIAL.lock().await;
    let log = install_global_log("personacore=info");
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let known = upstream(ROOMY_WINDOW).await;
    let unknown = MockServer::start().await;
    mount_chat(&unknown).await;
    let sub_upstream = upstream(ROOMY_WINDOW).await;
    let sub = face(&sub_upstream.uri(), SUB_MODEL, &json!({}));
    let source = json!({
        "plain": { "main": face(&known.uri(), MAIN_MODEL, &json!({})), "sub": sub.clone() },
        "unknown": {
            "main": face(&unknown.uri(), MAIN_MODEL, &json!({ "response_tokens": 2000 })),
            "sub": sub,
        },
    });
    let path = dir.path().join("upstreams.json");
    std::fs::write(&path, source.to_string()).expect("プロファイルを置けること");
    let running = Running::start(
        &config_of(&source, "plain", &dir.path().join("arm9.sqlite")),
        Some(&path),
    )
    .await;
    assert_eq!(
        (
            running.budget.tokens_for_tests(),
            running.budget.response_tokens_for_tests()
        ),
        (ROOMY_WINDOW, 1024),
        "前提: 起動は窓を答える上流"
    );

    let mark = log.mark();
    running.switch_to("unknown").await;
    assert_eq!(
        (
            running.budget.tokens_for_tests(),
            running.budget.response_tokens_for_tests()
        ),
        (8_192, 1024),
        "窓を答えない上流へ切り替えたのに、面の上限を使った"
    );
    running.turn(&[&sub_upstream], 1).await;
    assert_eq!(
        of_kind(&seen(&unknown).await, Kind::Body, |one| one
            .max_tokens
            .clone()),
        vec![json!(1024)],
        "窓が分からないのに本文の max_tokens が面の上限"
    );

    let output = log.contents_since(mark);
    let switched = samplers_lines(&output, "main", "switch");
    assert_eq!(switched.len(), 1, "{switched:?}");
    assert_eq!(
        (
            token_value(switched[0], "response_tokens"),
            token_value(switched[0], "response_tokens_fallback"),
            token_value(switched[0], "response_tokens_fallback_cause"),
        ),
        (Some("1024"), Some("true"), Some("window_unknown")),
        "切り替えの行が、窓が分からないので既定にしたと言っていない: {}",
        switched[0]
    );

    running.stop().await;
}
