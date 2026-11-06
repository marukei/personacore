//! **上流への鍵**（[ADR-0056](../docs/adr/0056-a-key-for-the-upstream.md) §決定2〜4 / 作業単位 A-13b-1）を、
//! `llm` の線の上で確かめる。**実際の上流には繋がない**（`wiremock` の偽物）。
//!
//! # 何を見るか
//!
//! | 見張り番 | 決定 |
//! |---|---|
//! | [`every_send_to_a_keyed_upstream_carries_that_upstreams_key`] | §決定2: 3つの送信（照会・本文・数える）の全部に、その上流の鍵だけが付く |
//! | [`an_upstream_without_a_key_receives_no_authorization`] | §決定2: 鍵の無い上流には何も付かない |
//! | [`a_key_echoed_by_a_refusing_upstream_never_reaches_the_log`] | §決定3: 上流が写し返した鍵をログに出さない |
//! | [`a_key_straddling_the_log_limit_is_redacted_before_the_cut`] | §決定3: 伏せてから切り詰める（レビュー是正） |
//! | [`a_key_echoed_in_a_successful_answer_never_reaches_the_log`] | §決定3: 成功の応答の本文（`log_snippet`）でも伏せる（レビュー是正） |
//! | [`a_refusal_on_either_list_reader_reads_as_a_refused_key`] | §決定4: 照会がどちらの綴りで断られても「鍵を断った」と読める |
//! | [`the_debug_form_of_a_keyed_client_never_shows_the_key`] | §決定3: `Debug` に出ない |
//!
//! # 鍵は2つ・上流は2つ
//!
//! **本文と裏方を別の鍵・別の上流にする**（取り違えを見分けるため）。
//! 同じ鍵だと、片方の鍵を両方に付ける実装がここで緑になる。

mod common;

use futures_util::StreamExt;
use personacore::llm::{
    ApiKey, ChatMessage, ChatRequest, LlmError, LmStudioClient, LmStudioConfig, ModelId,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{SharedBuffer, install_global_log};

/// 本文の上流の鍵（囮）。
const MAIN_KEY: &str = "pc-main-key-A1b2C3d4E5f6-main";
/// 裏方の上流の鍵（囮）。**本文の鍵とも、その一部とも重ならない綴り。**
const SUB_KEY: &str = "pc-sub-key-Z9y8X7w6V5u4-sub";

const MODEL: &str = "planted-key-model";

fn log() -> &'static SharedBuffer {
    install_global_log("personacore=debug")
}

fn client(base_url: &str, key: Option<&str>) -> LmStudioClient {
    let model = ModelId::parse(MODEL).expect("正当なモデル識別子");
    let config = LmStudioConfig::new(base_url, model)
        .expect("正当なベース URL")
        .with_api_key(key.map(|raw| ApiKey::parse(raw).expect("正当な鍵")));
    LmStudioClient::new(config).expect("クライアントを組めること")
}

fn request() -> ChatRequest {
    ChatRequest::new(vec![
        ChatMessage::system("あなたは葵です"),
        ChatMessage::user("こんにちは"),
    ])
}

fn json(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

fn completion_body() -> String {
    format!(
        r#"{{"id":"x","object":"chat.completion","created":1,"model":"{MODEL}",
            "choices":[{{"index":0,"message":{{"role":"assistant","content":"「はい」"}},
            "finish_reason":"stop"}}]}}"#
    )
}

fn stream_body() -> String {
    format!(
        "data: {{\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"{MODEL}\",\
         \"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"「はい」\"}},\"finish_reason\":null}}]}}\n\n\
         data: [DONE]\n\n"
    )
}

/// **全部の口に答える上流**（`/api/v0/models` だけは 404 —— 2つ目の読み手まで通すため）。
async fn answering_upstream() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(json(format!(
            r#"{{"object":"list","data":[{{"id":"{MODEL}","object":"model","meta":{{"n_ctx":8192}}}}]}}"#
        )))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let streaming = String::from_utf8_lossy(&request.body).contains("\"stream\":true");
            if streaming {
                ResponseTemplate::new(200).set_body_raw(stream_body(), "text/event-stream")
            } else {
                json(completion_body())
            }
        })
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/apply-template"))
        .respond_with(json(r#"{"prompt":"組み上がり"}"#.to_owned()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/tokenize"))
        .respond_with(json(r#"{"tokens":[1,2,3]}"#.to_owned()))
        .mount(&server)
        .await;
    server
}

/// **全部の口を1回ずつ叩く**（照会・非ストリーミング・ストリーミング・封・部品）。
async fn exercise(client: &LmStudioClient) {
    client.models().await.expect("照会が通ること");
    client.complete(&request()).await.expect("本文が通ること");
    let mut stream = client.stream(&request()).await.expect("中継が始まること");
    while let Some(item) = stream.next().await {
        item.expect("中継の項目が壊れていないこと");
    }
    client
        .count_request(&request(), false)
        .await
        .expect("封が通ること");
    client
        .count_text("こんにちは")
        .await
        .expect("部品が通ること");
}

/// **届いた要求の `(パス, Authorization)` の組。**
async fn authorizations(server: &MockServer) -> Vec<(String, Option<String>)> {
    server
        .received_requests()
        .await
        .expect("記録が有効であること")
        .iter()
        .map(|request| {
            (
                request.url.path().to_owned(),
                request
                    .headers
                    .get("authorization")
                    .map(|value| value.to_str().expect("ASCII").to_owned()),
            )
        })
        .collect()
}

/// 全部の口の綴り（**叩いたのに数えない**、を防ぐ）。
const EVERY_PATH: [&str; 5] = [
    "/api/v0/models",
    "/v1/models",
    "/v1/chat/completions",
    "/apply-template",
    "/tokenize",
];

/// **3つの送信の全部に、その上流の鍵が付く。もう一方の鍵は付かない**（ADR-0056 §決定2）。
///
/// **`with_model` の複製も同じ鍵を持つ**（`sub` を書かないプロファイルの抽出の面は、本文の
/// クライアントを `with_model` で複製する —— 同じ宛先なので、同じ鍵が付くのが正しい）。
#[tokio::test]
async fn every_send_to_a_keyed_upstream_carries_that_upstreams_key() {
    let main_upstream = answering_upstream().await;
    let sub_upstream = answering_upstream().await;
    let main = client(&main_upstream.uri(), Some(MAIN_KEY));
    let sub = client(&sub_upstream.uri(), Some(SUB_KEY));

    exercise(&main).await;
    exercise(&sub).await;
    exercise(&main.with_model(ModelId::parse("another-model").expect("正当"))).await;

    for (server, own, other) in [
        (&main_upstream, MAIN_KEY, SUB_KEY),
        (&sub_upstream, SUB_KEY, MAIN_KEY),
    ] {
        let seen = authorizations(server).await;
        for wanted in EVERY_PATH {
            assert!(
                seen.iter().any(|(path, _)| path == wanted),
                "前提: {wanted} を叩いたこと: {seen:?}"
            );
        }
        for (path, header) in &seen {
            assert_eq!(
                header.as_deref(),
                Some(format!("Bearer {own}").as_str()),
                "{path} にその上流の鍵が付いていない"
            );
            assert!(
                !header.as_deref().unwrap_or_default().contains(other),
                "{path} にもう一方の鍵が付いている"
            );
        }
    }
}

/// **鍵の無い上流には、`Authorization` を1つも付けない**（ADR-0056 §決定2）。
#[tokio::test]
async fn an_upstream_without_a_key_receives_no_authorization() {
    let upstream = answering_upstream().await;
    exercise(&client(&upstream.uri(), None)).await;

    let seen = authorizations(&upstream).await;
    for wanted in EVERY_PATH {
        assert!(
            seen.iter().any(|(path, _)| path == wanted),
            "前提: {wanted} を叩いたこと: {seen:?}"
        );
    }
    for (path, header) in &seen {
        assert_eq!(header, &None, "鍵の無い上流の {path} に何か付いている");
    }
}

/// **上流が鍵を写し返しても、ログに鍵は出ない**（ADR-0056 §決定3 / `error_snippet`）。
///
/// **401 の本文を全部の口で返し**、本文がログに載る経路（本文の `warn`・照会の `debug` と
/// 両方読めなかった `warn`・数える口の `debug`）を全部通す。**本文の外の文字はそのまま出る**
/// ことを肯定側で見る（行が出ていないことで緑にならない）。
#[tokio::test]
async fn a_key_echoed_by_a_refusing_upstream_never_reaches_the_log() {
    let buffer = log();
    let upstream = MockServer::start().await;
    let echo = format!(r#"{{"error":"Invalid API key: {MAIN_KEY}","marker":"echo-marker"}}"#);
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(401).set_body_raw(echo, "application/json"))
        .mount(&upstream)
        .await;
    let keyed = client(&upstream.uri(), Some(MAIN_KEY));

    let mark = buffer.mark();
    let refused = keyed.complete(&request()).await.expect_err("断られること");
    let streamed = match keyed.stream(&request()).await {
        Ok(_) => panic!("中継も断られること"),
        Err(err) => err,
    };
    let listed = keyed.models().await.expect_err("照会も断られること");
    let _ = keyed.count_request(&request(), false).await;
    let logged = buffer.contents_since(mark);

    for err in [&refused, &streamed, &listed] {
        assert!(err.refused_the_key(), "401 は鍵を断ったと読む: {err:?}");
    }
    assert!(
        logged.contains("echo-marker"),
        "前提: 上流の本文がログに載る経路を通ったこと: {logged}"
    );
    assert!(
        !logged.contains(MAIN_KEY),
        "上流が写し返した鍵がログに出ている: {logged}"
    );
}

/// **伏せてから切り詰める**（`error_snippet` の順 / 作業単位 A-13b-1 のレビュー是正）。
///
/// **鍵がログの上限（512 文字）の境目を跨ぐ本文**を返す。先に切ると、境目の手前に残った
/// 鍵の頭（7 文字）は鍵全体に一致しないので伏せられずに出る。**その頭の綴りがログに無いこと**を見る
/// （鍵の頭は、囮の他の綴りに現れない文字列にしてある）。
#[tokio::test]
async fn a_key_straddling_the_log_limit_is_redacted_before_the_cut() {
    const EDGE_KEY: &str = "zQ7xW3vK-edge-key-Lm9P0";
    const MARKER: &str = "edge-marker";
    let buffer = log();
    let upstream = MockServer::start().await;
    // `{"error":"edge-marker ` は 22 文字。鍵の頭を 505 文字目から置く（512 − 505 = 7 文字が境目の手前）。
    let head = format!(r#"{{"error":"{MARKER} "#);
    let padding = "a".repeat(505 - head.chars().count());
    let body = format!(r#"{head}{padding}{EDGE_KEY}"}}"#);
    assert_eq!(
        body.find(EDGE_KEY),
        Some(505),
        "前提: 鍵が境目を跨ぐ位置に在ること"
    );
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(401).set_body_raw(body, "application/json"))
        .mount(&upstream)
        .await;

    let mark = buffer.mark();
    let refused = client(&upstream.uri(), Some(EDGE_KEY))
        .complete(&request())
        .await
        .expect_err("断られること");
    let logged = buffer.contents_since(mark);

    assert!(refused.refused_the_key(), "{refused:?}");
    assert!(
        logged.contains(MARKER),
        "前提: 上流の本文がログに載る経路を通ったこと: {logged}"
    );
    assert!(
        !logged.contains(&EDGE_KEY[..7]),
        "境目で切れた鍵の頭がログに出ている（切ってから伏せている）: {logged}"
    );
}

/// **成功の応答の本文をログに出すときも、鍵を伏せる**（`log_snippet` / 作業単位 A-13b-1 のレビュー是正）。
///
/// 数える口（`/apply-template` / `/tokenize`）が 200 を返しても、形が読めなければ本文を `debug` に出す。
/// **その本文が鍵を写し返していても、ログに鍵が出ないこと**を見る（2つの口とも）。
///
/// **鍵はこの腕だけの綴り**（ログの器はバイナリで1つなので、他の腕の鍵と同じだと、
/// 壊れたときに他の腕まで赤くなり、どこが漏れたのかが名前から読めない）。
#[tokio::test]
async fn a_key_echoed_in_a_successful_answer_never_reaches_the_log() {
    const ANSWER_KEY: &str = "pc-answer-key-Ra8Tb7Uc6";
    let buffer = log();
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/apply-template"))
        .respond_with(json(format!(
            r#"{{"echo":"{ANSWER_KEY}","marker":"template-marker"}}"#
        )))
        .mount(&upstream)
        .await;
    Mock::given(method("POST"))
        .and(path("/tokenize"))
        .respond_with(json(format!(
            r#"{{"echo":"{ANSWER_KEY}","marker":"tokenize-marker"}}"#
        )))
        .mount(&upstream)
        .await;
    let keyed = client(&upstream.uri(), Some(ANSWER_KEY));

    let mark = buffer.mark();
    let _ = keyed.count_request(&request(), false).await;
    let _ = keyed.count_text("こんにちは").await;
    let logged = buffer.contents_since(mark);

    for marker in ["template-marker", "tokenize-marker"] {
        assert!(
            logged.contains(marker),
            "前提: {marker} の本文がログに載る経路を通ったこと: {logged}"
        );
    }
    assert!(
        !logged.contains(ANSWER_KEY),
        "成功の応答に写し返された鍵がログに出ている: {logged}"
    );
}

/// **照会は、どちらの綴りで断られても「鍵を断った」と読める**（ADR-0056 §決定4）。
///
/// **1つ目（`/api/v0/models`）が 404 で、2つ目（`/v1/models`）が 401** の上流がある
/// （鍵で守られた中継は `/v1/*` を全部 401 にする / origin#77）。1つ目の 404 を返すと、
/// 画面は「届かない」と「断られた」を見分けられない。
#[tokio::test]
async fn a_refusal_on_either_list_reader_reads_as_a_refused_key() {
    for (native, compat, refused) in [
        (404, 401, true),
        (404, 403, true),
        (401, 404, true),
        (403, 500, true),
        // **対照**: どちらも鍵を断っていない。
        (404, 500, false),
        (404, 404, false),
    ] {
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v0/models"))
            .respond_with(ResponseTemplate::new(native))
            .mount(&upstream)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(compat))
            .mount(&upstream)
            .await;
        let err: LlmError = client(&upstream.uri(), Some(MAIN_KEY))
            .models()
            .await
            .expect_err("どちらも読めないこと");
        assert_eq!(
            err.refused_the_key(),
            refused,
            "native {native} / compat {compat}: {err:?}"
        );
    }
}

/// **鍵を持つ設定とクライアントの `Debug` に、鍵は出ない**（ADR-0056 §決定3）。
#[test]
fn the_debug_form_of_a_keyed_client_never_shows_the_key() {
    let keyed = client("http://127.0.0.1:9", Some(MAIN_KEY));
    let shown = format!("{keyed:?} {:?}", keyed.config());
    assert!(!shown.contains(MAIN_KEY), "{shown}");
    assert!(
        shown.contains("ApiKey(***)"),
        "前提: 鍵を持っていること（Debug は伏せた形を出す）: {shown}"
    );
}
