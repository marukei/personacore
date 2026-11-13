//! **本文が空なら最大2回引き直す**（[ADR-0059](../docs/adr/0059-thinking-and-response-room-in-the-upstream-profile.md)
//! §決定3 / §決定4 の腕5 / 追記（2026-11-13）の決定1〜6 / 作業単位 A-18b・A-18c）が、**本番の入口（`App::bind` → `App::serve`）を通って**
//! 両方の経路で効くことの検証。**実際の上流には繋がない**（`wiremock` の偽物）。
//!
//! # 何を見るか
//!
//! | 見張り番 | 経路 | 観測すること |
//! |---|---|---|
//! | [`a_blank_reply_is_retried_once_on_a_face_without_reasoning_effort`] | 非ストリーミング | 思考を書いていない面でも空なら同じ要求で引き直し、2回目を返して保存する。2回目が空でなければ3回目は送らない。使用量の清算は1回で2回目の値。分解の行の欄 |
//! | [`a_reply_with_a_body_is_sent_once`] | 両方 | 空でない回は1往復だけ。分解の行の欄は 0。`finish_reason` がある回では `warn` を出さない |
//! | [`a_failed_retry_keeps_the_first_blank_reply`] | 両方 | 2回目が失敗したら1回目の空を保存する。中継では `finish_reason` と `[DONE]` が最後に1回だけ流れる |
//! | [`the_stream_is_retried_when_the_thought_came_in_a_separate_field`] | ストリーミング | 思考を別欄で返し本文が空のまま終わった回（実物の形）は引き直し、2回目を同じ応答の続きとして流す。2回目が空でなければ3回目は送らない |
//! | [`the_stream_is_retried_after_a_closed_thought_or_whitespace`] | ストリーミング | 本文の中で閉じた思考だけ・空白だけの回も引き直す。思考の面（`low`）でも |
//! | [`the_stream_is_not_retried_after_an_unclosed_thought`] | 両方 | 閉じない思考は、中継では引き直さない（非ストリーミングでは引き直す） |
//! | [`a_third_attempt_is_sent_when_the_second_is_also_blank`] | 両方 | 2回目も空なら3回目を送り、3回目を採る。3回とも同じ要求。捨てた2回の欄は合計 |
//! | [`three_blank_replies_are_kept_without_a_fourth_request`] | 両方 | 3回とも空なら3回目の空を保存し、4回目は送らない |
//! | [`a_failed_third_attempt_keeps_the_second_blank_reply`] | 両方 | 3回目を開けなければ2回目の空を保存する（`empty_retries=2`・`failed=true`・捨てた回の欄は1回目） |
//! | [`a_third_stream_that_breaks_midway_saves_nothing`] | ストリーミング | 3回目の途中で壊れたら error で終え、何も保存しない |
//! | [`a_blank_cut_by_the_length_limit_allows_only_one_retry_in_either_order`] | 両方 | `length` の空が1回でもあれば、`stop`→`length`・`length`→`stop` のどちらでも3回目を送らない |
//! | [`a_stream_that_ends_blank_without_a_finish_reason_is_retried`] | ストリーミング | `finish_reason` 無しで `[DONE]` で終わった空も引き直す。`warn` と `upstream_finish_missing` |
//! | [`a_stream_without_a_finish_reason_is_not_retried_outside_the_rule`] | ストリーミング | 対照: 閉じない思考は引き直さない／開けなければ `[DONE]` だけで空を保存／`Truncated` は引き直さず保存もしない |
//! | [`a_reply_with_a_body_but_no_finish_reason_is_warned_and_not_retried`] | 両方 | `finish_reason` 無しで本文がある回は `warn` を出すが引き直さない |
//! | [`both_held_closing_tags_are_relayed_before_the_third_attempt`] | ストリーミング | 捨てる2回の終わりのチャンクの閉じタグが、両方とも下流に流れる |
//! | [`the_discarded_completion_tokens_are_absent_when_only_some_attempts_report_usage`] | 両方 | 捨てた回の一部だけが `usage` を返したら、生成数の欄は無し |
//! | [`a_runaway_stream_is_cut_and_stored_but_a_complete_reply_is_not_cut`] | 両方 | 暴走の区間の合計が 256 文字に達したチャンクで中継を打ち切り、`length` と `[DONE]` で終えて保存する（引き直さない / ADR-0062 §決定4 / 作業単位 A-21b）。非ストリーミングは打ち切らない |
//!
//! 打ち切りの境目（255 / 256）・切れ目を変えたときの一致・引き直しで合計を捨てることは `server::relay` の単体テストが見る。
//!
//! 3回目を開く待ちを番人が報告することは、仮想時計の要る見張り番なので `server::relay` の単体テストが見る。
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
/// `finish_reason` が無いまま終わった回を言う行（`server::empty_retry::FINISH_MISSING_LINE` の一部 / 作業単位 A-18c）。
const FINISH_MISSING_LINE: &str = "finish_reason を返さないまま";

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

/// 非ストリーミングの答え（`finish_reason` は `stop`）。
fn completion(content: &str, usage: Option<(u32, u32)>) -> ResponseTemplate {
    completion_ending(content, Some("stop"), usage)
}

/// 非ストリーミングの答え。**`finish` が `None` なら `finish_reason` の欄ごと持たない**（作業単位 A-18c）。
fn completion_ending(
    content: &str,
    finish: Option<&str>,
    usage: Option<(u32, u32)>,
) -> ResponseTemplate {
    let mut choice = json!({
        "index": 0,
        "message": {"role": "assistant", "content": content},
    });
    if let Some(finish) = finish {
        choice["finish_reason"] = json!(finish);
    }
    let mut body = json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": MAIN_MODEL,
        "choices": [choice],
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
    sse_closing(chunks, true)
}

/// SSE の答え。**`done` が偽なら `[DONE]` を送らずに閉じる**（`llm` 層の `Truncated` になる形 / 作業単位 A-18c）。
fn sse_closing(chunks: &[Value], done: bool) -> ResponseTemplate {
    let mut text = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        text.push_str(&format!("data: {chunk}\n\n"));
        if index == 0 {
            text.push_str(": keep-alive\n\n");
        }
    }
    if done {
        text.push_str("data: [DONE]\n\n");
    }
    ResponseTemplate::new(200).set_body_raw(text, "text/event-stream")
}

/// 役割だけの先頭チャンク（実物の形）。
fn role_chunk() -> Value {
    chunk(json!({"role": "assistant", "content": ""}), None, None)
}

/// 使用量だけのチャンク（`choices` が空）。
fn usage_only_chunk(usage: (u32, u32)) -> Value {
    json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1,
    "model": MAIN_MODEL, "choices": [], "usage": usage_json(usage)})
}

/// **思考を別欄（`reasoning_content`）で返し、本文が空のまま終わる**（測定で記録した実物の形。使用量は終わりのチャンク）。
fn stream_thought_in_a_separate_field(usage: (u32, u32)) -> ResponseTemplate {
    stream_thought_ending(Some("stop"), Some(usage))
}

/// [`stream_thought_in_a_separate_field`] の、終わりの `finish_reason` と使用量を選ぶ形。
///
/// `finish` が `None` なら **`finish_reason` 付きのチャンクを1本も送らずに `[DONE]` で閉じる**（使用量は、あれば使用量だけのチャンクで）。
fn stream_thought_ending(finish: Option<&str>, usage: Option<(u32, u32)>) -> ResponseTemplate {
    let mut chunks = vec![
        role_chunk(),
        chunk(json!({"reasoning_content": "The"}), None, None),
        chunk(json!({"reasoning_content": " user is home."}), None, None),
    ];
    match finish {
        Some(finish) => chunks.push(chunk(json!({}), Some(finish), usage)),
        None => chunks.extend(usage.map(usage_only_chunk)),
    }
    sse(&chunks)
}

/// **本文を差分で流す**（使用量は終わりの後の使用量だけのチャンク）。
fn stream_content(parts: &[&str], usage: (u32, u32)) -> ResponseTemplate {
    let mut chunks = vec![role_chunk()];
    chunks.extend(
        parts
            .iter()
            .map(|part| chunk(json!({"content": part}), None, None)),
    );
    chunks.push(chunk(json!({}), Some("stop"), None));
    chunks.push(usage_only_chunk(usage));
    sse(&chunks)
}

/// **本文を差分で流し、`finish_reason` 付きのチャンクを1本も送らずに閉じる**（marukei/personacore#47 の形）。
///
/// `done` が偽なら `[DONE]` も送らない（`Truncated`）。使用量は、あれば使用量だけのチャンクで。
fn stream_content_without_finish(
    parts: &[&str],
    usage: Option<(u32, u32)>,
    done: bool,
) -> ResponseTemplate {
    let mut chunks = vec![role_chunk()];
    chunks.extend(
        parts
            .iter()
            .map(|part| chunk(json!({"content": part}), None, None)),
    );
    chunks.extend(usage.map(usage_only_chunk));
    sse_closing(&chunks, done)
}

/// **閉じタグが `finish_reason` のチャンクに載って届く**形（本文の中で閉じた思考だけ）。
fn stream_closing_in_the_finish(thought: &str, close: &str, usage: (u32, u32)) -> ResponseTemplate {
    sse(&[
        role_chunk(),
        chunk(json!({"content": thought}), None, None),
        chunk(json!({"content": close}), Some("stop"), None),
        usage_only_chunk(usage),
    ])
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
        self.assert_finished_once_at_the_end_with("stop");
    }

    /// [`Self::assert_finished_once_at_the_end`] の、`finish_reason` の値を選ぶ形。
    fn assert_finished_once_at_the_end_with(&self, reason: &str) {
        assert_eq!(
            self.dones,
            vec![self.events - 1],
            "[DONE] は最後に1回だけ: {self:?}"
        );
        assert_eq!(
            self.finishes,
            vec![(self.events - 2, reason.to_owned())],
            "finish_reason は [DONE] の直前に1回だけ: {self:?}"
        );
        assert_eq!(self.roles, 1, "役割は1回だけ: {self:?}");
    }

    /// **`finish_reason` は一度も流れず、`[DONE]` が最後に1回だけ**（`finish_reason` 無しで終わった回を採用した形）。
    fn assert_done_once_without_a_finish(&self) {
        assert_eq!(
            self.dones,
            vec![self.events - 1],
            "[DONE] は最後に1回だけ: {self:?}"
        );
        assert!(
            self.finishes.is_empty(),
            "finish_reason は流れない: {self:?}"
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
            // **3回目の台本も置く**（2回目が空でなければ3回目を送らないこと / 作業単位 A-18c）。
            completion("余分", None),
        ],
        &json!({}),
        false,
    )
    .await;

    assert_eq!(
        turn.requests.len(),
        2,
        "2回目が空でなければ3回目を送らないこと"
    );
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
        "upstream_finish_missing=false",
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
            // **`finish_reason` がある回では言わない**（対照 / 作業単位 A-18c）。
            "upstream_finish_missing=false",
        ] {
            assert!(
                has_field(line, field),
                "streaming={streaming}: {field}: {line:?}"
            );
        }
        assert!(
            turn.lines(FINISH_MISSING_LINE).is_empty(),
            "streaming={streaming}: finish_reason がある回では warn を出さないこと"
        );
        if streaming {
            let seen = seen(&turn.response);
            seen.assert_finished_once_at_the_end();
            assert_eq!(seen.content, SECOND);
        }
    }
}

// ------------------------------------------- 引き直しを最大2回にする（作業単位 A-18c）

/// 1回目の空（空白1文字）。**2回目（[`SECOND_BLANK`]）と文字数が違う** —— 捨てた回の文字数の合計を見分ける。
const FIRST_BLANK_SPACE: &str = " ";
/// 2回目の空（本文の中で閉じた思考だけ。16 文字）。
const SECOND_BLANK: &str = "<think>b</think>";
/// 2回目の生成数。**1回目（[`FIRST_COMPLETION`]）・3回目（[`SECOND_COMPLETION`]）と違える。**
const MIDDLE_COMPLETION: u32 = 23;
/// 2回目のプロンプトの数（清算がどの回の値かを見分ける）。
const MIDDLE_PROMPT: u32 = 702;

/// 1回目と2回目が空の台本（使用量は両方あり）。3回目は `third`、4回目は余分の本文。
fn two_blanks_then(third: ResponseTemplate, streaming: bool) -> Vec<ResponseTemplate> {
    if streaming {
        vec![
            stream_content(&[FIRST_BLANK_SPACE], (FIRST_PROMPT, FIRST_COMPLETION)),
            stream_content(&[SECOND_BLANK], (MIDDLE_PROMPT, MIDDLE_COMPLETION)),
            third,
            stream_content(&["余分"], (1, 1)),
        ]
    } else {
        vec![
            completion(FIRST_BLANK_SPACE, Some((FIRST_PROMPT, FIRST_COMPLETION))),
            completion(SECOND_BLANK, Some((MIDDLE_PROMPT, MIDDLE_COMPLETION))),
            third,
            completion("余分", None),
        ]
    }
}

/// **2回目も空なら3回目を送り、3回目が空でなければそれを採る**（両方の経路 / ADR-0059 追記（2026-11-13）の決定1・決定6）。
///
/// 3回とも同じ要求。保存・清算は3回目、捨てた2回の欄は**合計**（文字数 1＋16、生成数 41＋23）。
#[tokio::test]
async fn a_third_attempt_is_sent_when_the_second_is_also_blank() {
    let _serial = SERIAL.lock().await;
    for streaming in [false, true] {
        let third = if streaming {
            stream_content(&SECOND_PARTS, (SECOND_PROMPT, SECOND_COMPLETION))
        } else {
            completion(SECOND, Some((SECOND_PROMPT, SECOND_COMPLETION)))
        };
        let turn = one_turn(two_blanks_then(third, streaming), &json!({}), streaming).await;
        assert_eq!(
            turn.requests.len(),
            3,
            "streaming={streaming}: 3回目を送ること"
        );
        turn.assert_requests_are_identical();
        assert_eq!(turn.saved_raw_reply(), SECOND, "streaming={streaming}");

        let line = turn.parse_line();
        for field in [
            "outcome=structured",
            "empty_retries=2",
            "empty_retry_failed=false",
            &format!(
                "discarded_reply_chars={}",
                FIRST_BLANK_SPACE.chars().count() + SECOND_BLANK.chars().count()
            ),
            "discarded_reply_unclosed=false",
            &format!(
                "discarded_completion_tokens=Some({})",
                FIRST_COMPLETION + MIDDLE_COMPLETION
            ),
            "upstream_finish_missing=false",
        ] {
            assert!(
                has_field(line, field),
                "streaming={streaming}: {field}: {line:?}"
            );
        }
        let seals = turn.lines(SEAL_LINE);
        assert_eq!(seals.len(), 1, "清算は1回だけ: {seals:?}");
        assert!(
            has_field(
                seals[0],
                &format!("usage_prompt_tokens=Some({SECOND_PROMPT})")
            ),
            "清算は3回目の値: {}",
            seals[0]
        );
        let checks = turn.lines(CROSSCHECK_LINE);
        assert_eq!(checks.len(), 1, "答え合わせは1回だけ: {checks:?}");
        if streaming {
            let seen = seen(&turn.response);
            seen.assert_finished_once_at_the_end();
            assert_eq!(
                seen.content,
                format!("{FIRST_BLANK_SPACE}{SECOND_BLANK}{SECOND}"),
                "3回目が同じ応答の続きとして流れる"
            );
        } else {
            let returned: Value = serde_json::from_str(&turn.response).expect("JSON");
            assert_eq!(returned["choices"][0]["message"]["content"], json!(SECOND));
        }
    }
}

/// **3回とも空なら空として保存し、4回目は送らない**（両方の経路）。保存と清算は3回目。
///
/// 中継の1回目・2回目は思考を別欄で返す実物の形。
#[tokio::test]
async fn three_blank_replies_are_kept_without_a_fourth_request() {
    let _serial = SERIAL.lock().await;
    /// 3回目の空（1回目・2回目と別の綴り）。
    const THIRD_BLANK: &str = "<think>\nStill thinking.\n</think>";
    /// 3回目のプロンプトの数。
    const THIRD_PROMPT: u32 = 704;
    for streaming in [false, true] {
        let script = if streaming {
            vec![
                stream_thought_in_a_separate_field((FIRST_PROMPT, FIRST_COMPLETION)),
                stream_thought_in_a_separate_field((MIDDLE_PROMPT, MIDDLE_COMPLETION)),
                stream_content(&[THIRD_BLANK], (THIRD_PROMPT, 5)),
                stream_content(&SECOND_PARTS, (1, 1)),
            ]
        } else {
            vec![
                completion("", Some((FIRST_PROMPT, FIRST_COMPLETION))),
                completion(" ", Some((MIDDLE_PROMPT, MIDDLE_COMPLETION))),
                completion(THIRD_BLANK, Some((THIRD_PROMPT, 5))),
                completion(SECOND, None),
            ]
        };
        let turn = one_turn(script, &json!({}), streaming).await;
        assert_eq!(
            turn.requests.len(),
            3,
            "streaming={streaming}: 4回目を送らない"
        );
        turn.assert_requests_are_identical();
        assert_eq!(
            turn.saved_raw_reply(),
            THIRD_BLANK,
            "streaming={streaming}: 3回目の空を保存する"
        );
        let line = turn.parse_line();
        for field in [
            "outcome=empty",
            "empty_retries=2",
            "empty_retry_failed=false",
        ] {
            assert!(
                has_field(line, field),
                "streaming={streaming}: {field}: {line:?}"
            );
        }
        let seals = turn.lines(SEAL_LINE);
        assert_eq!(seals.len(), 1, "清算は1回だけ: {seals:?}");
        assert!(
            has_field(
                seals[0],
                &format!("usage_prompt_tokens=Some({THIRD_PROMPT})")
            ),
            "{}",
            seals[0]
        );
        if streaming {
            seen(&turn.response).assert_finished_once_at_the_end();
        }
    }
}

/// **3回目を開けなければ、2回目の空を保存する**（両方の経路）。`empty_retries=2`・`failed=true`、捨てた回の欄は1回目の値。
///
/// 中継では、2回目の保留チャンクと `[DONE]` が最後に1回ずつ流れる。清算は1回で2回目の値。
/// **非ストリーミングでは「3回目の途中で壊れた」も同じ扱い**（応答を最後まで受け取ってから保存するので、
/// 壊れた回は開けなかった回と区別されない / 作業単位 A-18c の解釈）—— 本文が壊れた JSON の腕で見る。
#[tokio::test]
async fn a_failed_third_attempt_keeps_the_second_blank_reply() {
    let _serial = SERIAL.lock().await;
    for (streaming, third) in [
        (false, ResponseTemplate::new(500)),
        (
            false,
            ResponseTemplate::new(200).set_body_raw("{\"choices\":[{\"mess", "application/json"),
        ),
        (true, ResponseTemplate::new(500)),
    ] {
        let mut script = two_blanks_then(third, streaming);
        script.truncate(3);
        let turn = one_turn(script, &json!({}), streaming).await;
        assert_eq!(
            turn.requests.len(),
            3,
            "streaming={streaming}: 3回目を送った"
        );
        assert_eq!(
            turn.saved_raw_reply(),
            SECOND_BLANK,
            "streaming={streaming}: 2回目の空を保存する"
        );
        let line = turn.parse_line();
        for field in [
            "outcome=empty",
            "empty_retries=2",
            "empty_retry_failed=true",
            &format!(
                "discarded_reply_chars={}",
                FIRST_BLANK_SPACE.chars().count()
            ),
            &format!("discarded_completion_tokens=Some({FIRST_COMPLETION})"),
        ] {
            assert!(
                has_field(line, field),
                "streaming={streaming}: {field}: {line:?}"
            );
        }
        let failures = turn.lines(RETRY_FAILED_LINE);
        assert_eq!(failures.len(), 1, "失敗は1本の warn: {failures:?}");
        let seals = turn.lines(SEAL_LINE);
        assert_eq!(seals.len(), 1, "清算は1回だけ: {seals:?}");
        assert!(
            has_field(
                seals[0],
                &format!("usage_prompt_tokens=Some({MIDDLE_PROMPT})")
            ),
            "清算は2回目の値: {}",
            seals[0]
        );
        if streaming {
            let seen = seen(&turn.response);
            seen.assert_finished_once_at_the_end();
            assert_eq!(seen.content, format!("{FIRST_BLANK_SPACE}{SECOND_BLANK}"));
        }
    }
}

/// **中継で3回目の途中で上流が壊れたら、error で終えて何も保存しない**（A-18b の追記のまま）。
///
/// 3回目は本文を1つ流した後、`[DONE]` を送らずに閉じる（`Truncated`）。
#[tokio::test]
async fn a_third_stream_that_breaks_midway_saves_nothing() {
    let _serial = SERIAL.lock().await;
    let third = sse_closing(
        &[
            role_chunk(),
            chunk(json!({"content": SECOND_PARTS[0]}), None, None),
        ],
        false,
    );
    let mut script = two_blanks_then(third, true);
    script.truncate(3);
    let turn = one_turn(script, &json!({}), true).await;
    assert_eq!(turn.requests.len(), 3, "前提: 3回目を開いたこと");
    assert!(turn.saved.is_empty(), "何も保存しない: {:?}", turn.saved);
    assert!(
        turn.lines(PARSE_LINE).is_empty(),
        "分解の行も出ない（保存の入口に来ない）"
    );
    let lines = data_lines(&turn.response);
    assert!(
        !lines.iter().any(|data| data == "[DONE]"),
        "[DONE] を流さない: {lines:?}"
    );
    let last: Value = serde_json::from_str(lines.last().expect("1件はあること")).expect("JSON");
    assert!(
        last.get("error").is_some(),
        "最後は error オブジェクト: {last}"
    );
}

/// **上限（`length`）で切れた空が1回でもあれば、引き直しは合わせて1回まで**（両方の経路・両方の順）。
///
/// `stop`→`length` でも `length`→`stop` でも3回目を送らず、2回目の空を保存する。中継の空は思考を別欄で返す形
/// （本文に漏れる上流の `length` は閉じない思考になり、もともと引き直さない / 決定1）。
#[tokio::test]
async fn a_blank_cut_by_the_length_limit_allows_only_one_retry_in_either_order() {
    let _serial = SERIAL.lock().await;
    for streaming in [false, true] {
        for (first, second) in [("stop", "length"), ("length", "stop")] {
            let script = if streaming {
                vec![
                    stream_thought_ending(Some(first), Some((FIRST_PROMPT, FIRST_COMPLETION))),
                    stream_thought_ending(Some(second), Some((MIDDLE_PROMPT, MIDDLE_COMPLETION))),
                    stream_content(&SECOND_PARTS, (SECOND_PROMPT, SECOND_COMPLETION)),
                ]
            } else {
                vec![
                    completion_ending("", Some(first), Some((FIRST_PROMPT, FIRST_COMPLETION))),
                    completion_ending(" ", Some(second), Some((MIDDLE_PROMPT, MIDDLE_COMPLETION))),
                    completion(SECOND, Some((SECOND_PROMPT, SECOND_COMPLETION))),
                ]
            };
            let turn = one_turn(script, &json!({}), streaming).await;
            let case = format!("streaming={streaming} {first}→{second}");
            assert_eq!(turn.requests.len(), 2, "{case}: 3回目を送らない");
            let expected = if streaming { "" } else { " " };
            assert_eq!(
                turn.saved_raw_reply(),
                expected,
                "{case}: 2回目の空を保存する"
            );
            let line = turn.parse_line();
            for field in [
                "outcome=empty",
                "empty_retries=1",
                "empty_retry_failed=false",
            ] {
                assert!(has_field(line, field), "{case}: {field}: {line:?}");
            }
            if streaming {
                seen(&turn.response).assert_finished_once_at_the_end_with(second);
            }
        }
    }
}

/// **中継で、`finish_reason` 付きのチャンクが一度も無いまま `[DONE]` で終わった空も引き直す**（決定2 / marukei/personacore#47）。
///
/// その回の `[DONE]` は流さず、2回目を続きとして流す。下流の `finish_reason` と `[DONE]` は2回目のもの。
/// `finish_reason` が無いまま終わった回は `warn` で言い、分解の行の `upstream_finish_missing` が立つ。
#[tokio::test]
async fn a_stream_that_ends_blank_without_a_finish_reason_is_retried() {
    let _serial = SERIAL.lock().await;
    let turn = one_turn(
        vec![
            stream_content_without_finish(&[], None, true),
            stream_content(&SECOND_PARTS, (SECOND_PROMPT, SECOND_COMPLETION)),
            stream_content(&["余分"], (1, 1)),
        ],
        &json!({}),
        true,
    )
    .await;
    assert_eq!(turn.requests.len(), 2, "引き直すこと");
    turn.assert_requests_are_identical();
    let seen = seen(&turn.response);
    seen.assert_finished_once_at_the_end();
    assert_eq!(seen.content, SECOND);
    assert_eq!(turn.saved_raw_reply(), SECOND);
    let line = turn.parse_line();
    for field in [
        "outcome=structured",
        "empty_retries=1",
        "empty_retry_failed=false",
        "discarded_reply_chars=0",
        // **使用量を返さなかった回を捨てた**ので無し。
        "discarded_completion_tokens=None",
        "upstream_finish_missing=true",
    ] {
        assert!(has_field(line, field), "{field}: {line:?}");
    }
    let warns = turn.lines(FINISH_MISSING_LINE);
    assert_eq!(warns.len(), 1, "1回目だけを言う: {warns:?}");
    assert!(warns[0].contains("WARN"), "{}", warns[0]);
    assert!(has_field(warns[0], "streaming=true"), "{}", warns[0]);
}

/// **`finish_reason` 無しで `[DONE]` で終わった回の対照**（中継）: 閉じない思考は引き直さない／
/// 引き直しを開けなければ `[DONE]` だけを流して空を保存する／`[DONE]` 無しで尽きた回（`Truncated`）は引き直さず保存もしない。
#[tokio::test]
async fn a_stream_without_a_finish_reason_is_not_retried_outside_the_rule() {
    let _serial = SERIAL.lock().await;
    /// 閉じない思考。
    const UNCLOSED: &str = "<think>\nThe user is asking";

    // 閉じない思考 → 引き直さない。
    let turn = one_turn(
        vec![
            stream_content_without_finish(&["<think>\nThe user", " is asking"], None, true),
            stream_content(&SECOND_PARTS, (SECOND_PROMPT, SECOND_COMPLETION)),
        ],
        &json!({}),
        true,
    )
    .await;
    assert_eq!(turn.requests.len(), 1, "閉じない思考は引き直さない");
    seen(&turn.response).assert_done_once_without_a_finish();
    assert_eq!(turn.saved_raw_reply(), UNCLOSED);
    let line = turn.parse_line();
    assert!(has_field(line, "empty_retries=0"), "{line:?}");
    assert!(has_field(line, "upstream_finish_missing=true"), "{line:?}");

    // 引き直しを開けない → `[DONE]` だけを流して、その空の回を保存する。
    let turn = one_turn(
        vec![stream_content_without_finish(&[], None, true)],
        &json!({}),
        true,
    )
    .await;
    assert_eq!(turn.requests.len(), 2, "引き直しを試みたこと");
    let seen_failed = seen(&turn.response);
    seen_failed.assert_done_once_without_a_finish();
    assert_eq!(seen_failed.content, "");
    assert_eq!(turn.saved_raw_reply(), "");
    let line = turn.parse_line();
    for field in [
        "outcome=empty",
        "empty_retries=1",
        "empty_retry_failed=true",
        "upstream_finish_missing=true",
    ] {
        assert!(has_field(line, field), "{field}: {line:?}");
    }

    // `[DONE]` 無しで尽きた（`Truncated`）→ 引き直さず、error で終えて保存しない。
    let turn = one_turn(
        vec![
            stream_content_without_finish(&[], None, false),
            stream_content(&SECOND_PARTS, (SECOND_PROMPT, SECOND_COMPLETION)),
        ],
        &json!({}),
        true,
    )
    .await;
    assert_eq!(turn.requests.len(), 1, "Truncated は引き直さない");
    assert!(turn.saved.is_empty(), "保存しない: {:?}", turn.saved);
    // 途中で壊れた回は「終わった回」ではないので、`finish_reason` 無しの `warn` は出さない（壊れたことは別の行が言う）。
    assert!(
        turn.lines(FINISH_MISSING_LINE).is_empty(),
        "Truncated では warn しない"
    );
    let lines = data_lines(&turn.response);
    assert!(!lines.iter().any(|data| data == "[DONE]"), "{lines:?}");
    let last: Value = serde_json::from_str(lines.last().expect("1件はあること")).expect("JSON");
    assert!(
        last.get("error").is_some(),
        "最後は error オブジェクト: {last}"
    );
}

/// **`finish_reason` が無いまま終わったが本文がある回は、`warn` は出るが引き直さない**（両方の経路 / 決定3）。
#[tokio::test]
async fn a_reply_with_a_body_but_no_finish_reason_is_warned_and_not_retried() {
    let _serial = SERIAL.lock().await;
    for streaming in [false, true] {
        let script = if streaming {
            vec![
                stream_content_without_finish(
                    &SECOND_PARTS,
                    Some((SECOND_PROMPT, SECOND_COMPLETION)),
                    true,
                ),
                stream_content(&["余分"], (1, 1)),
            ]
        } else {
            vec![
                completion_ending(SECOND, None, Some((SECOND_PROMPT, SECOND_COMPLETION))),
                completion("余分", None),
            ]
        };
        let turn = one_turn(script, &json!({}), streaming).await;
        assert_eq!(
            turn.requests.len(),
            1,
            "streaming={streaming}: 引き直さない"
        );
        assert_eq!(turn.saved_raw_reply(), SECOND, "streaming={streaming}");
        let line = turn.parse_line();
        for field in ["empty_retries=0", "upstream_finish_missing=true"] {
            assert!(
                has_field(line, field),
                "streaming={streaming}: {field}: {line:?}"
            );
        }
        let warns = turn.lines(FINISH_MISSING_LINE);
        assert_eq!(warns.len(), 1, "streaming={streaming}: {warns:?}");
        assert!(warns[0].contains("WARN"), "{}", warns[0]);
        assert!(
            has_field(warns[0], &format!("streaming={streaming}")),
            "{}",
            warns[0]
        );
        if streaming {
            let seen = seen(&turn.response);
            seen.assert_done_once_without_a_finish();
            assert_eq!(seen.content, SECOND);
        }
    }
}

/// **捨てる回が2つとも終わりのチャンクに閉じタグを持つとき、両方の閉じタグが下流に流れる**（中継 / 決定4）。
///
/// どちらかを捨てると、使い手の側で「閉じない思考＋続き」になり、3回目の本文まで思考として剥がされる。
#[tokio::test]
async fn both_held_closing_tags_are_relayed_before_the_third_attempt() {
    let _serial = SERIAL.lock().await;
    const CLOSE: &str = "</think>";
    const FIRST_THOUGHT: &str = "<think>\nFirst.\n";
    const SECOND_THOUGHT: &str = "<think>\nSecond try.\n";
    let turn = one_turn(
        vec![
            stream_closing_in_the_finish(FIRST_THOUGHT, CLOSE, (FIRST_PROMPT, FIRST_COMPLETION)),
            stream_closing_in_the_finish(SECOND_THOUGHT, CLOSE, (MIDDLE_PROMPT, MIDDLE_COMPLETION)),
            stream_content(&SECOND_PARTS, (SECOND_PROMPT, SECOND_COMPLETION)),
        ],
        &json!({}),
        true,
    )
    .await;
    assert_eq!(turn.requests.len(), 3);
    let seen = seen(&turn.response);
    seen.assert_finished_once_at_the_end();
    assert_eq!(
        seen.content,
        format!("{FIRST_THOUGHT}{CLOSE}{SECOND_THOUGHT}{CLOSE}{SECOND}"),
        "両方の閉じタグを落とさずに、3回目を続けること"
    );
    assert_eq!(turn.saved_raw_reply(), SECOND);
    let line = turn.parse_line();
    assert!(
        has_field(
            line,
            &format!(
                "discarded_reply_chars={}",
                format!("{FIRST_THOUGHT}{CLOSE}{SECOND_THOUGHT}{CLOSE}")
                    .chars()
                    .count()
            )
        ),
        "{line:?}"
    );
}

/// **捨てた回の一部だけが `usage` を返したら、`discarded_completion_tokens` は無し**（両方の経路・両方の順 / 決定5）。
///
/// 一部だけの合計（返した回だけ・最後の回だけ）を出す実装は、どちらかの順で赤になる。
#[tokio::test]
async fn the_discarded_completion_tokens_are_absent_when_only_some_attempts_report_usage() {
    let _serial = SERIAL.lock().await;
    for streaming in [false, true] {
        for usage_on_first in [true, false] {
            let usage_of = |reported: bool, usage: (u32, u32)| reported.then_some(usage);
            let first = usage_of(usage_on_first, (FIRST_PROMPT, FIRST_COMPLETION));
            let middle = usage_of(!usage_on_first, (MIDDLE_PROMPT, MIDDLE_COMPLETION));
            let script = if streaming {
                vec![
                    stream_thought_ending(Some("stop"), first),
                    stream_thought_ending(Some("stop"), middle),
                    stream_content(&SECOND_PARTS, (SECOND_PROMPT, SECOND_COMPLETION)),
                ]
            } else {
                vec![
                    completion("", first),
                    completion(" ", middle),
                    completion(SECOND, Some((SECOND_PROMPT, SECOND_COMPLETION))),
                ]
            };
            let turn = one_turn(script, &json!({}), streaming).await;
            let case = format!("streaming={streaming} usage_on_first={usage_on_first}");
            assert_eq!(turn.requests.len(), 3, "{case}: 前提");
            assert_eq!(turn.saved_raw_reply(), SECOND, "{case}");
            let line = turn.parse_line();
            for field in ["empty_retries=2", "discarded_completion_tokens=None"] {
                assert!(has_field(line, field), "{case}: {field}: {line:?}");
            }
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
            // **3回目の台本も置く**（2回目が空でなければ3回目を送らないこと / 作業単位 A-18c）。
            stream_content(&["余分"], (1, 1)),
        ],
        &json!({}),
        true,
    )
    .await;
    assert_eq!(
        turn.requests.len(),
        2,
        "2回目が空でなければ3回目を送らないこと"
    );
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

// ------------------------------------------- 暴走の打ち切り（ADR-0062 §決定4 / 作業単位 A-21b）

/// 打ち切りの行（`server::relay::RUNAWAY_CUT_LINE` の先頭）。
const RUNAWAY_CUT_LINE: &str = "返答の暴走が続いたので、上流の生成を打ち切りました";

/// **暴走した本文の中継は、区間の合計が 256 文字に達したチャンクで打ち切られ、`length` と `[DONE]` で終わって保存される。
/// 非ストリーミングは打ち切らない**（返す本文も保存も全文 / ADR-0062 §決定4 / 作業単位 A-21b）。
///
/// どこで打ち切るかの期待値は、差分ごとに積み上げた全文へ `fold_runaway_repetition` を当てて決める（権威は純粋関数）。
/// 手で数えると、単位（`「好き」……`、6 文字）の 43 回目で 258 文字（42 回目は 252 文字）。打ち切った回は引き直さず
/// （本文がある）、`usage` が届かないので封の清算は `usage` 無し。保存した返答の分解は畳んだ本文で行う（ADR-0062 §決定2）。
#[tokio::test]
async fn a_runaway_stream_is_cut_and_stored_but_a_complete_reply_is_not_cut() {
    const OPENING: &str = "「おかえり」";
    const UNIT: &str = "「好き」……";
    let _serial = SERIAL.lock().await;

    let mut parts = vec![OPENING];
    parts.extend(std::iter::repeat_n(UNIT, 60));
    let whole = parts.concat();
    let total = |text: &str| -> usize {
        personacore::domain::fold_runaway_repetition(text)
            .runs
            .iter()
            .map(|run| run.length)
            .sum()
    };
    let mut prefix = String::new();
    let mut expected = None;
    for part in &parts {
        prefix.push_str(part);
        if total(&prefix) >= personacore::domain::RUNAWAY_CUT_CHARS {
            expected = Some(prefix.clone());
            break;
        }
    }
    let expected = expected.expect("前提: どこかで 256 に達する");
    assert_eq!(
        expected,
        format!("{OPENING}{}", UNIT.repeat(43)),
        "前提: 手で数えた位置と一致する"
    );

    // ---- ストリーミング: 打ち切る。
    let turn = one_turn(
        vec![
            stream_content(&parts, (FIRST_PROMPT, FIRST_COMPLETION)),
            stream_content(&SECOND_PARTS, (SECOND_PROMPT, SECOND_COMPLETION)),
        ],
        &json!({}),
        true,
    )
    .await;
    assert_eq!(turn.requests.len(), 1, "打ち切った回は引き直さない");
    let shown = seen(&turn.response);
    shown.assert_finished_once_at_the_end_with("length");
    assert_eq!(shown.content, expected, "検出したチャンクまでを流すこと");
    assert_eq!(
        turn.saved_raw_reply(),
        expected,
        "保存は流した生の出力と同じ"
    );
    let cut = turn.lines(RUNAWAY_CUT_LINE);
    assert_eq!(cut.len(), 1, "打ち切りの行は1本: {cut:?}");
    for field in [
        "runs=1",
        "longest_run_period=6",
        "longest_run_chars=258",
        "runaway_total_chars=258",
    ] {
        assert!(has_field(cut[0], field), "{field}: {}", cut[0]);
    }
    let seals = turn.lines(SEAL_LINE);
    assert_eq!(seals.len(), 1, "封の清算は1回: {seals:?}");
    assert!(
        has_field(seals[0], "usage_prompt_tokens=None"),
        "打ち切った回は usage が届かない: {}",
        seals[0]
    );
    let line = turn.parse_line();
    for field in ["runaway_runs_folded=1", "empty_retries=0"] {
        assert!(has_field(line, field), "{field}: {line:?}");
    }

    // ---- 非ストリーミング: 打ち切らない（返す本文は変えない）。
    let turn = one_turn(
        vec![completion(&whole, Some((FIRST_PROMPT, FIRST_COMPLETION)))],
        &json!({}),
        false,
    )
    .await;
    let returned: Value = serde_json::from_str(&turn.response).expect("JSON");
    assert_eq!(returned["choices"][0]["message"]["content"], json!(whole));
    assert_eq!(turn.saved_raw_reply(), whole, "保存も全文");
    assert!(turn.lines(RUNAWAY_CUT_LINE).is_empty());
}
