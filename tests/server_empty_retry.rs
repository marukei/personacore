//! **本文が空なら1回だけ引き直す**（[ADR-0059](../docs/adr/0059-thinking-and-response-room-in-the-upstream-profile.md)
//! §決定3 / §決定4 の腕5 / 作業単位 A-18b）が、**本番の入口（`App::bind` → `App::serve`）を通って**両方の経路で効くことの検証。
//! **実際の上流には繋がない**（`wiremock` の偽物）。
//!
//! # 何を見るか
//!
//! | 見張り番 | 経路 | 観測すること |
//! |---|---|---|
//! | [`a_blank_reply_is_retried_once_on_a_face_without_reasoning_effort`] | 非ストリーミング | 思考を書いていない面でも空なら1回だけ同じ要求で引き直し、2回目を返して保存する。使用量の清算は1回で2回目の値。分解の行の欄 |
//! | [`a_reply_with_a_body_is_sent_once`] | 両方 | 空でない回は1往復だけ。分解の行の欄は 0 |
//! | [`a_second_blank_reply_is_kept_without_a_third_request`] | 両方 | 2回目も空なら空として保存し、3回目は送らない |
//! | [`a_failed_retry_keeps_the_first_blank_reply`] | 両方 | 2回目が失敗したら1回目の空を保存する。中継では `finish_reason` と `[DONE]` が最後に1回だけ流れる |
//! | [`the_stream_is_retried_when_the_thought_came_in_a_separate_field`] | ストリーミング | 思考を別欄で返し本文が空のまま終わった回（実物の形）は引き直し、2回目を同じ応答の続きとして流す |
//! | [`the_stream_is_retried_after_a_closed_thought_or_whitespace`] | ストリーミング | 本文の中で閉じた思考だけ・空白だけの回も引き直す。思考の面（`low`）でも |
//! | [`the_stream_is_not_retried_after_an_unclosed_thought`] | 両方 | 閉じない思考は、中継では引き直さない（非ストリーミングでは引き直す） |
//!
//! 偽の上流は本文の依頼（`# ペルソナ` を持つ）にだけ台本を返し、裏方（抽出・提案）には固定の答えを返す。
//! **台本を使い切った後の本文の依頼には 500 を返す**（数えるのは届いた本文の依頼の件数）。
//!
//! # 1本ずつ走らせる
//!
//! ログはバイナリで1つなので、どのテストも [`SERIAL`] を取る。

mod common;

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use personacore::domain::{CharacterId, DisplayName, Handle, Message, Persona};
use personacore::server::{
    App, AppConfig, BackgroundProbe, BindAddr, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL, ENV_SUB_MODEL,
    ServerError, overlay_profile, parse_profiles,
};
use personacore::store::Store;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate, matchers};

use common::{fixture_scale, has_field, install_global_log};

/// **1本ずつ走らせる**（モジュールの doc）。
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// このバイナリのログの綴り（答え合わせの `debug` 行も読む）。
const LOG_FILTER: &str = "personacore=debug";

/// 本文に指定するモデル。
const MAIN_MODEL: &str = "retry-main-model";
/// 会話するキャラクター。
const HANDLE: &str = "aoi";
/// 利用者の発言。
const USER_MESSAGE: &str = "ただいま";
/// **本文の依頼だけに現れる見出し**（`prompt` の定数を呼ばない）。
const PERSONA_HEADING: &str = "# ペルソナ";

/// 2回目の本文。**1回目のどの形とも、利用者の発言とも別の文字列。**
const SECOND: &str = "「おかえりなさい」 彼女は微笑んだ。";
/// [`SECOND`] を中継で流すときの割り方（チャンクをまたいでも1本の本文になること）。
const SECOND_PARTS: [&str; 3] = ["「おかえり", "なさい」", " 彼女は微笑んだ。"];

/// 1回目のプロンプトの数。**2回目（[`SECOND_PROMPT`]）と違える** —— 清算がどちらの値かを見分ける。
const FIRST_PROMPT: u32 = 701;
/// 1回目の生成数（捨てた回の欄に出る値）。
const FIRST_COMPLETION: u32 = 41;
/// 2回目のプロンプトの数。
const SECOND_PROMPT: u32 = 803;
/// 2回目の生成数。
const SECOND_COMPLETION: u32 = 9;

/// 分解の行。
const PARSE_LINE: &str = "応答を分解しました";
/// 封の行（清算の度に1本 / `server::seal::report`）。
const SEAL_LINE: &str = "封を観測しました";
/// 答え合わせの行（`debug` / `server::usage::log_check`）。
const CROSSCHECK_LINE: &str = "トークン見積もりを実測と照合しました";
/// 2回目の要求が失敗したことを言う行（`server::empty_retry::RETRY_FAILED_LINE` の先頭）。
const RETRY_FAILED_LINE: &str = "空の本文の引き直しに失敗しました";

/// 待つ上限。
const WAIT_LIMIT: Duration = Duration::from_secs(20);
/// 待ちの刻み。
const POLL_STEP: Duration = Duration::from_millis(10);

// ---------------------------------------------------------------- 偽の上流

/// **本文の依頼か**（裏方の依頼は抽出の例か提案の印を持つ）。
fn is_body(request: &Value) -> bool {
    let messages = request["messages"].to_string();
    let background = messages.contains(personacore::prompt::extraction::EXAMPLE_USER_LINE)
        || [
            "抜｜引用｜取り決め｜守る側｜中身",
            "点検｜番号｜行い｜分類｜守る側｜判定｜中身",
        ]
        .iter()
        .any(|marker| messages.contains(marker));
    !background && messages.contains(PERSONA_HEADING)
}

/// 本文の依頼に台本を順に返す偽物。**使い切ったら 500。** 裏方には固定の答え。
struct Script(Mutex<VecDeque<ResponseTemplate>>);

impl Respond for Script {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        if is_body(&body) {
            return self
                .0
                .lock()
                .expect("ロック")
                .pop_front()
                .unwrap_or_else(|| ResponseTemplate::new(500));
        }
        completion("- 利用者: 山あいの町で育った", None)
    }
}

/// 非ストリーミングの答え。
fn completion(content: &str, usage: Option<(u32, u32)>) -> ResponseTemplate {
    let mut body = json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": MAIN_MODEL,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop",
        }],
    });
    if let Some(usage) = usage {
        body["usage"] = usage_json(usage);
    }
    ResponseTemplate::new(200).set_body_raw(body.to_string(), "application/json")
}

fn usage_json((prompt, completion): (u32, u32)) -> Value {
    json!({"prompt_tokens": prompt, "completion_tokens": completion, "total_tokens": prompt + completion})
}

/// 中継の1チャンク（`delta` はそのまま、`finish_reason` と `usage` は任意）。
fn chunk(delta: Value, finish_reason: Option<&str>, usage: Option<(u32, u32)>) -> Value {
    let mut chunk = json!({
        "id": "chatcmpl-1",
        "object": "chat.completion.chunk",
        "created": 1,
        "model": MAIN_MODEL,
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish_reason}],
    });
    if let Some(usage) = usage {
        chunk["usage"] = usage_json(usage);
    }
    chunk
}

/// SSE の答え（`[DONE]` で閉じる。間に注釈の行を挟む —— 実物の形）。
fn sse(chunks: &[Value]) -> ResponseTemplate {
    let mut text = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        text.push_str(&format!("data: {chunk}\n\n"));
        if index == 0 {
            text.push_str(": keep-alive\n\n");
        }
    }
    text.push_str("data: [DONE]\n\n");
    ResponseTemplate::new(200).set_body_raw(text, "text/event-stream")
}

/// **思考を別欄（`reasoning_content`）で返し、本文が空のまま終わる**（測定で記録した実物の形。使用量は終わりのチャンク）。
fn stream_thought_in_a_separate_field(usage: (u32, u32)) -> ResponseTemplate {
    sse(&[
        chunk(json!({"role": "assistant", "content": ""}), None, None),
        chunk(json!({"reasoning_content": "The"}), None, None),
        chunk(json!({"reasoning_content": " user is home."}), None, None),
        chunk(json!({}), Some("stop"), Some(usage)),
    ])
}

/// **本文を差分で流す**（使用量は終わりの後の使用量だけのチャンク）。
fn stream_content(parts: &[&str], usage: (u32, u32)) -> ResponseTemplate {
    let mut chunks = vec![chunk(
        json!({"role": "assistant", "content": ""}),
        None,
        None,
    )];
    chunks.extend(
        parts
            .iter()
            .map(|part| chunk(json!({"content": part}), None, None)),
    );
    chunks.push(chunk(json!({}), Some("stop"), None));
    chunks.push(
        json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1,
        "model": MAIN_MODEL, "choices": [], "usage": usage_json(usage)}),
    );
    sse(&chunks)
}

/// 本文の依頼に `script` を順に返す上流（窓の照会にも答える）。
async fn upstream(script: Vec<ResponseTemplate>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(Script(Mutex::new(script.into())))
        .mount(&server)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(
                r#"{{"object":"list","data":[{{"id":"{MAIN_MODEL}","object":"model","type":"llm",
                  "state":"loaded","max_context_length":262144,"loaded_context_length":16384}}]}}"#
            ),
            "application/json",
        ))
        .mount(&server)
        .await;
    server
}

/// **届いた本文の依頼**（届いた順。本文そのもの）。
async fn body_requests(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .map(|request| serde_json::from_slice(&request.body).expect("JSON として読めること"))
        .filter(is_body)
        .collect()
}

// ---------------------------------------------------------------- 本番の入口

/// **`main` と同じ段で設定を組む**（`tests/server_profile_thinking.rs` と同じ）。`main_extra` は本文の面に足す鍵。
fn config_of(endpoint: &str, main_extra: &Value, db_path: &Path) -> AppConfig {
    let mut main = json!({ "endpoint": endpoint, "model": MAIN_MODEL });
    if let (Some(main), Some(extra)) = (main.as_object_mut(), main_extra.as_object()) {
        for (key, value) in extra {
            main.insert(key.clone(), value.clone());
        }
    }
    let source = json!({ "retry": { "main": main } });
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
    let settings = profiles.select("retry").expect("在ること").settings();
    AppConfig::from_lookup(overlay_profile(&settings, base))
        .expect("設定を組み立てられること")
        .with_profile_context(&settings)
        .expect("書いた値を載せられること")
        .with_profile_keys(&settings)
        .with_profile_sampling(&settings)
        .with_bind(BindAddr::loopback(0))
        .with_database_path(db_path)
}

/// 1往復の結果。
struct Turn {
    /// 本文の依頼（届いた順）。
    requests: Vec<Value>,
    /// 利用者へ返した本文（非ストリーミングは JSON、ストリーミングは SSE の生の本文）。
    response: String,
    /// 保存された往復。
    saved: Vec<Message>,
    /// このターンのログ。
    log: String,
    /// 着地したセッション。
    session: i64,
}

impl Turn {
    /// **このセッションの分解の行**（1本であること）。
    fn parse_line(&self) -> &str {
        let lines: Vec<&str> = self
            .log
            .lines()
            .filter(|line| {
                line.contains(PARSE_LINE)
                    && has_field(line, &format!("session_id={}", self.session))
            })
            .collect();
        assert_eq!(lines.len(), 1, "分解の行は1本: {lines:?}");
        lines[0]
    }

    /// その文言を持つ行。
    fn lines(&self, marker: &str) -> Vec<&str> {
        self.log
            .lines()
            .filter(|line| line.contains(marker))
            .collect()
    }

    /// 保存された応答の生の出力。
    fn saved_raw_reply(&self) -> &str {
        assert_eq!(self.saved.len(), 2, "往復2件が入ること: {:?}", self.saved);
        &self.saved[1].raw_content
    }

    /// 本文の依頼が全部同じ要求であること（引き直しは同じ要求を送る）。
    fn assert_requests_are_identical(&self) {
        for request in &self.requests {
            assert_eq!(request, &self.requests[0], "同じ要求を送り直すこと");
        }
    }
}

/// **本文の面に `main_extra` を書いた構成で起動し、1往復させる。** 裏方が終わるまで待ってから止める。
async fn one_turn(script: Vec<ResponseTemplate>, main_extra: &Value, streaming: bool) -> Turn {
    let log = install_global_log(LOG_FILTER);
    let mark = log.mark();
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let server = upstream(script).await;
    let config = config_of(&server.uri(), main_extra, &dir.path().join("retry.sqlite"));
    let app = App::bind(&config).await.expect("待ち受けを開始できること");
    let store: Store = app.store().clone();
    let character: CharacterId = store
        .create_character(
            &Handle::parse(HANDLE).expect("正当な名前であること"),
            &DisplayName::parse("葵").expect("正当な表示名であること"),
            &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");
    let port = app.local_addr().port();
    let background: BackgroundProbe = app.background_probe();
    let (shutdown, rx) = oneshot::channel::<()>();
    let handle: JoinHandle<Result<(), ServerError>> = tokio::spawn(async move {
        app.serve(async move {
            let _ = rx.await;
        })
        .await
    });

    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .json(&json!({
            "model": HANDLE,
            "messages": [{"role": "user", "content": USER_MESSAGE}],
            "stream": streaming,
        }))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .expect("サーバへ接続できること");
    let status = response.status();
    let text = response.text().await.expect("本文を読めること");
    assert!(status.is_success(), "{status}: {text}");

    let deadline = Instant::now() + WAIT_LIMIT;
    while !background.is_idle() {
        assert!(Instant::now() < deadline, "裏方の枠が空かない");
        tokio::time::sleep(POLL_STEP).await;
    }

    let session = session_of(&text, streaming);
    let found = store
        .find_session(character, session)
        .await
        .expect("セッションを引けること")
        .expect("セッションが在ること");
    let saved = store
        .recent_messages(found.id, 10)
        .await
        .expect("会話履歴を読めること");
    let requests = body_requests(&server).await;

    shutdown.send(()).expect("停止シグナルを送れること");
    handle
        .await
        .expect("サーバタスクが panic しないこと")
        .expect("正常に終了すること");

    Turn {
        requests,
        response: text,
        saved,
        log: log.contents_since(mark),
        session,
    }
}

/// 応答の `model`（`aoi@<id>`）からセッションを読む。
fn session_of(text: &str, streaming: bool) -> i64 {
    let model = if streaming {
        let first = data_lines(text)
            .into_iter()
            .find(|data| data != "[DONE]")
            .expect("チャンクが1件はあること");
        serde_json::from_str::<Value>(&first).expect("JSON")["model"].clone()
    } else {
        serde_json::from_str::<Value>(text).expect("JSON")["model"].clone()
    };
    model
        .as_str()
        .and_then(|model| model.split_once('@'))
        .and_then(|(_, id)| id.parse().ok())
        .unwrap_or_else(|| panic!("model からセッションを読めること: {model}"))
}

/// SSE の本文の `data:` の中身。
fn data_lines(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| data.trim().to_owned())
        .collect()
}

/// 中継で利用者に見えたもの。
#[derive(Debug)]
struct Seen {
    /// 増分の連結。
    content: String,
    /// `null` でない `finish_reason`（チャンクの位置と値）。
    finishes: Vec<(usize, String)>,
    /// `role` を持つチャンクの数。
    roles: usize,
    /// `[DONE]` の位置（`data:` の並びの中の添字）。
    dones: Vec<usize>,
    /// `data:` の件数。
    events: usize,
}

fn seen(body: &str) -> Seen {
    let lines = data_lines(body);
    let mut seen = Seen {
        content: String::new(),
        finishes: Vec::new(),
        roles: 0,
        dones: Vec::new(),
        events: lines.len(),
    };
    for (index, data) in lines.iter().enumerate() {
        if data == "[DONE]" {
            seen.dones.push(index);
            continue;
        }
        let value: Value = serde_json::from_str(data).expect("JSON として読めること");
        let choice = &value["choices"][0];
        seen.content
            .push_str(choice["delta"]["content"].as_str().unwrap_or_default());
        if choice["delta"].get("role").is_some() {
            seen.roles += 1;
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            seen.finishes.push((index, reason.to_owned()));
        }
    }
    seen
}

impl Seen {
    /// **`finish_reason` と `[DONE]` が最後に1回だけ**流れたこと（`finish_reason` の直後に `[DONE]`、それで終わり）。
    fn assert_finished_once_at_the_end(&self) {
        assert_eq!(
            self.dones,
            vec![self.events - 1],
            "[DONE] は最後に1回だけ: {self:?}"
        );
        assert_eq!(
            self.finishes,
            vec![(self.events - 2, "stop".to_owned())],
            "finish_reason は [DONE] の直前に1回だけ: {self:?}"
        );
        assert_eq!(self.roles, 1, "役割は1回だけ: {self:?}");
    }
}

// ---------------------------------------------------------------- 見張り番

/// **思考を書いていない面でも、空の本文は1回だけ同じ要求で引き直され、2回目が返って保存される**（非ストリーミング）。
///
/// 「思考を入れた面だけ引き直す」実装はここで赤になる（面に `reasoning_effort` が無い）。
/// **清算は1回で2回目の値**: 封の行が1本で `usage_prompt_tokens` が2回目、答え合わせの行も1本で2回目。
#[tokio::test]
async fn a_blank_reply_is_retried_once_on_a_face_without_reasoning_effort() {
    let _serial = SERIAL.lock().await;
    let turn = one_turn(
        vec![
            completion("", Some((FIRST_PROMPT, FIRST_COMPLETION))),
            completion(SECOND, Some((SECOND_PROMPT, SECOND_COMPLETION))),
        ],
        &json!({}),
        false,
    )
    .await;

    assert_eq!(turn.requests.len(), 2, "1回だけ引き直すこと");
    turn.assert_requests_are_identical();
    assert!(
        turn.requests[0].get("reasoning_effort") == Some(&json!("none")),
        "前提: 思考を書いていない面: {}",
        turn.requests[0]
    );
    let returned: Value = serde_json::from_str(&turn.response).expect("JSON");
    assert_eq!(returned["choices"][0]["message"]["content"], json!(SECOND));
    assert_eq!(turn.saved_raw_reply(), SECOND, "raw_content は2回目の出力");

    let line = turn.parse_line();
    for field in [
        "outcome=structured",
        "empty_retries=1",
        "empty_retry_failed=false",
        "discarded_reply_chars=0",
        "discarded_reply_unclosed=false",
        &format!("discarded_completion_tokens=Some({FIRST_COMPLETION})"),
    ] {
        assert!(has_field(line, field), "{field}: {line:?}");
    }

    let seals = turn.lines(SEAL_LINE);
    assert_eq!(seals.len(), 1, "封の清算は1回だけ: {seals:?}");
    assert!(
        has_field(
            seals[0],
            &format!("usage_prompt_tokens=Some({SECOND_PROMPT})")
        ),
        "清算は2回目の値: {}",
        seals[0]
    );
    let checks = turn.lines(CROSSCHECK_LINE);
    assert_eq!(checks.len(), 1, "答え合わせは1回だけ: {checks:?}");
    assert!(
        has_field(checks[0], &format!("actual_tokens={SECOND_PROMPT}")),
        "答え合わせは2回目の値: {}",
        checks[0]
    );
}

/// **空でない回は1往復だけ**（「常に2回送る」実装を赤に）。分解の行の新しい欄は 0 で出る。両方の経路。
#[tokio::test]
async fn a_reply_with_a_body_is_sent_once() {
    let _serial = SERIAL.lock().await;
    for streaming in [false, true] {
        let script = if streaming {
            stream_content(&SECOND_PARTS, (SECOND_PROMPT, SECOND_COMPLETION))
        } else {
            completion(SECOND, Some((SECOND_PROMPT, SECOND_COMPLETION)))
        };
        let turn = one_turn(
            vec![script, completion("余分", None)],
            &json!({}),
            streaming,
        )
        .await;
        assert_eq!(turn.requests.len(), 1, "streaming={streaming}: 1往復だけ");
        assert_eq!(turn.saved_raw_reply(), SECOND, "streaming={streaming}");
        let line = turn.parse_line();
        for field in [
            "empty_retries=0",
            "empty_retry_failed=false",
            "discarded_reply_chars=0",
            "discarded_reply_unclosed=false",
            "discarded_completion_tokens=None",
        ] {
            assert!(
                has_field(line, field),
                "streaming={streaming}: {field}: {line:?}"
            );
        }
        if streaming {
            let seen = seen(&turn.response);
            seen.assert_finished_once_at_the_end();
            assert_eq!(seen.content, SECOND);
        }
    }
}

/// **2回目も空なら空として保存し、3回目は送らない**（両方の経路）。保存するのは2回目の生の出力。
#[tokio::test]
async fn a_second_blank_reply_is_kept_without_a_third_request() {
    let _serial = SERIAL.lock().await;
    /// 2回目（本文の中で閉じた思考だけ）。1回目（空）と別の綴り。
    const SECOND_BLANK: &str = "<think>\nStill thinking.\n</think>";
    for streaming in [false, true] {
        let script = if streaming {
            vec![
                stream_thought_in_a_separate_field((FIRST_PROMPT, FIRST_COMPLETION)),
                stream_content(&[SECOND_BLANK], (SECOND_PROMPT, SECOND_COMPLETION)),
                stream_content(&SECOND_PARTS, (1, 1)),
            ]
        } else {
            vec![
                completion("", None),
                completion(SECOND_BLANK, None),
                completion(SECOND, None),
            ]
        };
        let turn = one_turn(script, &json!({}), streaming).await;
        assert_eq!(
            turn.requests.len(),
            2,
            "streaming={streaming}: 3回目を送らない"
        );
        assert_eq!(
            turn.saved_raw_reply(),
            SECOND_BLANK,
            "streaming={streaming}: 2回目の空を保存する"
        );
        let line = turn.parse_line();
        assert!(has_field(line, "outcome=empty"), "{line:?}");
        assert!(has_field(line, "empty_retries=1"), "{line:?}");
        if streaming {
            seen(&turn.response).assert_finished_once_at_the_end();
        }
    }
}

/// **2回目が失敗したら、1回目の空を空として保存する**（両方の経路）。
///
/// 中継では、保留していた `finish_reason` と `[DONE]` が**最後に1回だけ**流れる（「終わりの印を2回流す」実装を赤に）。
/// 清算は1回で1回目の値。失敗は `warn` に出る。
#[tokio::test]
async fn a_failed_retry_keeps_the_first_blank_reply() {
    let _serial = SERIAL.lock().await;
    /// 1回目（本文の中で閉じた思考だけ）。保存されるのはこの生の出力。
    const FIRST_BLANK: &str = "<think>\nThe user is home.\n</think>\n";
    for streaming in [false, true] {
        let first = if streaming {
            stream_content(&[FIRST_BLANK], (FIRST_PROMPT, FIRST_COMPLETION))
        } else {
            completion(FIRST_BLANK, Some((FIRST_PROMPT, FIRST_COMPLETION)))
        };
        // 2回目は台本が尽きて 500。
        let turn = one_turn(vec![first], &json!({}), streaming).await;
        assert_eq!(
            turn.requests.len(),
            2,
            "streaming={streaming}: 引き直しを送った"
        );
        assert_eq!(
            turn.saved_raw_reply(),
            FIRST_BLANK,
            "streaming={streaming}: 1回目の空を保存する"
        );
        let line = turn.parse_line();
        for field in [
            "outcome=empty",
            "empty_retries=1",
            "empty_retry_failed=true",
            "discarded_reply_chars=0",
        ] {
            assert!(
                has_field(line, field),
                "streaming={streaming}: {field}: {line:?}"
            );
        }
        let failures = turn.lines(RETRY_FAILED_LINE);
        assert_eq!(failures.len(), 1, "失敗は1本の warn: {failures:?}");
        assert!(failures[0].contains("WARN"), "{}", failures[0]);
        assert!(
            has_field(failures[0], &format!("streaming={streaming}")),
            "{}",
            failures[0]
        );
        let seals = turn.lines(SEAL_LINE);
        assert_eq!(seals.len(), 1, "清算は1回だけ: {seals:?}");
        assert!(
            has_field(
                seals[0],
                &format!("usage_prompt_tokens=Some({FIRST_PROMPT})")
            ),
            "{}",
            seals[0]
        );
        if streaming {
            let seen = seen(&turn.response);
            seen.assert_finished_once_at_the_end();
            assert_eq!(seen.content, FIRST_BLANK, "1回目の差分はそのまま流れている");
        } else {
            let returned: Value = serde_json::from_str(&turn.response).expect("JSON");
            assert_eq!(
                returned["choices"][0]["message"]["content"],
                json!(FIRST_BLANK)
            );
        }
    }
}

/// **思考を別欄で返し、本文が空のまま終わった回（実物の形）は、中継でも引き直す。**
///
/// 2回目の本文は**同じ応答の続き**として流れる: 利用者に見える増分の連結は2回目の本文、`finish_reason` と `[DONE]` は
/// 最後に1回だけ、役割も1回だけ。保存は2回目、清算は1回で2回目の値、捨てた1回目の生成数が分解の行に出る。
#[tokio::test]
async fn the_stream_is_retried_when_the_thought_came_in_a_separate_field() {
    let _serial = SERIAL.lock().await;
    let turn = one_turn(
        vec![
            stream_thought_in_a_separate_field((FIRST_PROMPT, FIRST_COMPLETION)),
            stream_content(&SECOND_PARTS, (SECOND_PROMPT, SECOND_COMPLETION)),
        ],
        &json!({}),
        true,
    )
    .await;
    assert_eq!(turn.requests.len(), 2, "1回だけ引き直すこと");
    turn.assert_requests_are_identical();
    let seen = seen(&turn.response);
    seen.assert_finished_once_at_the_end();
    assert_eq!(seen.content, SECOND, "2回目の本文が続きとして流れる");
    assert_eq!(turn.saved_raw_reply(), SECOND, "raw_content は2回目の出力");

    let line = turn.parse_line();
    for field in [
        "outcome=structured",
        "empty_retries=1",
        "empty_retry_failed=false",
        "discarded_reply_chars=0",
        "discarded_reply_unclosed=false",
        &format!("discarded_completion_tokens=Some({FIRST_COMPLETION})"),
    ] {
        assert!(has_field(line, field), "{field}: {line:?}");
    }
    let seals = turn.lines(SEAL_LINE);
    assert_eq!(seals.len(), 1, "封の清算は1回だけ: {seals:?}");
    assert!(
        has_field(
            seals[0],
            &format!("usage_prompt_tokens=Some({SECOND_PROMPT})")
        ),
        "{}",
        seals[0]
    );
    let checks = turn.lines(CROSSCHECK_LINE);
    assert_eq!(checks.len(), 1, "答え合わせは1回だけ: {checks:?}");
    assert!(
        has_field(checks[0], &format!("actual_tokens={SECOND_PROMPT}")),
        "{}",
        checks[0]
    );
}

/// **本文の中で閉じた思考だけ・空白だけの回も、中継で引き直す。** 思考の面（`low`）でも同じ。
#[tokio::test]
async fn the_stream_is_retried_after_a_closed_thought_or_whitespace() {
    let _serial = SERIAL.lock().await;
    for (first, extra) in [
        (
            vec!["<think>\nThe user", " is home.\n</think>", "\n\n"],
            json!({}),
        ),
        (vec![" ", "\n\u{3000}"], json!({})),
        (
            vec!["<think>\nThe user is home.\n</think>"],
            json!({ "sampling": { "reasoning_effort": "low" } }),
        ),
    ] {
        let raw: String = first.concat();
        let turn = one_turn(
            vec![
                stream_content(&first, (FIRST_PROMPT, FIRST_COMPLETION)),
                stream_content(&SECOND_PARTS, (SECOND_PROMPT, SECOND_COMPLETION)),
            ],
            &extra,
            true,
        )
        .await;
        assert_eq!(turn.requests.len(), 2, "{raw:?}: 引き直すこと");
        let seen = seen(&turn.response);
        seen.assert_finished_once_at_the_end();
        assert_eq!(
            seen.content,
            format!("{raw}{SECOND}"),
            "{raw:?}: 続きとして流れる"
        );
        assert_eq!(turn.saved_raw_reply(), SECOND, "{raw:?}");
        let line = turn.parse_line();
        assert!(
            has_field(
                line,
                &format!("discarded_reply_chars={}", raw.chars().count())
            ),
            "{raw:?}: {line:?}"
        );
        // **使用量は終わりの後の使用量だけのチャンクで届く形**（`stream_content`）。捨てた回の記録を、
        // 1回目の上流を読み切る前（保留した時点）に取ると、ここが `None` になる（レビューの T3）。
        assert!(
            has_field(
                line,
                &format!("discarded_completion_tokens=Some({FIRST_COMPLETION})")
            ),
            "{raw:?}: 捨てた回の生成数は、終わりの後の使用量から取ること: {line:?}"
        );
    }
}

/// **閉じない思考は、中継では引き直さない**（続けると2回目まで思考として捨てられる / ADR-0053 §決定2）。
/// **非ストリーミングでは引き直す**（対照）。
#[tokio::test]
async fn the_stream_is_not_retried_after_an_unclosed_thought() {
    let _serial = SERIAL.lock().await;
    /// 1回目（閉じない思考）。
    const UNCLOSED: &str = "<think>\nThe user is asking";

    let turn = one_turn(
        vec![
            stream_content(
                &["<think>\nThe user", " is asking"],
                (FIRST_PROMPT, FIRST_COMPLETION),
            ),
            stream_content(&SECOND_PARTS, (SECOND_PROMPT, SECOND_COMPLETION)),
        ],
        &json!({}),
        true,
    )
    .await;
    assert_eq!(turn.requests.len(), 1, "中継では閉じない思考を引き直さない");
    seen(&turn.response).assert_finished_once_at_the_end();
    assert_eq!(turn.saved_raw_reply(), UNCLOSED);
    let line = turn.parse_line();
    assert!(has_field(line, "thinking_unclosed=true"), "{line:?}");
    assert!(has_field(line, "empty_retries=0"), "{line:?}");

    let turn = one_turn(
        vec![
            completion(UNCLOSED, Some((FIRST_PROMPT, FIRST_COMPLETION))),
            completion(SECOND, Some((SECOND_PROMPT, SECOND_COMPLETION))),
        ],
        &json!({}),
        false,
    )
    .await;
    assert_eq!(turn.requests.len(), 2, "非ストリーミングでは引き直す");
    assert_eq!(turn.saved_raw_reply(), SECOND);
    let line = turn.parse_line();
    for field in [
        "empty_retries=1",
        "discarded_reply_unclosed=true",
        &format!("discarded_reply_chars={}", UNCLOSED.chars().count()),
    ] {
        assert!(has_field(line, field), "{field}: {line:?}");
    }
}
