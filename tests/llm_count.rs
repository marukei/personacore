//! 「数える」口（ADR-0052 §推奨 3・4 / 作業単位 B-60a）の結合検証。
//!
//! **実際の llama も LM Studio も起こさない。** 上流は `wiremock` の偽物で、
//! llama.cpp の `/apply-template`・`/tokenize` と、LM Studio の「未知の口に 200 ＋ エラー本文」を演じる。
//!
//! ここで見るのは次の4つ:
//!   * **N6** —— 封のために `/apply-template` へ送る本文が、`/v1/chat/completions` へ送る本文と
//!     **バイト単位で同じ**であること（非ストリーミング・ストリーミングの両方）。
//!   * **N8** —— 応答の**形**で判定すること。`{"tokens":[整数…]}` 以外は「数えられない」。
//!     **200 でも形が違えば数えない**（LM Studio の 200 ＋ エラー本文）。
//!   * **既定の実装は「数えられない」**（モックの上流は何も変えずにそのまま動く）。
//!   * 往復そのものが失敗したら `Err`（「数えられない上流」とは区別する）。

use std::time::Duration;

use futures_util::StreamExt;
use personacore::llm::{
    ChatMessage, ChatRequest, ChatStream, Completion, LlmClient, LlmError, LlmFuture,
    LmStudioClient, LmStudioConfig, ModelId, TokenCount,
};
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// テストで使う上流モデル。
const MODEL: &str = "planted-count-model";

/// `/apply-template` が返す、組み上がりの文字列（**植えた値**。本文のどれとも違う綴り）。
const PLANTED_TEMPLATE: &str = "<|planted_start|>system\nあなたは葵です<|planted_end|>\n<|planted_start|>user\nこんにちは<|planted_end|>\n";

fn client_for(base_url: &str) -> LmStudioClient {
    let model = ModelId::parse(MODEL).expect("テストで使うモデル識別子は正当であること");
    let config = LmStudioConfig::new(base_url, model).expect("ベース URL が正当であること");
    LmStudioClient::new(config).expect("HTTP クライアントを構築できること")
}

/// **サンプラーを全部埋めた依頼。** 落とす変異（温度や上限を落とした本文で封をする）が
/// 本文の差として現れるように、`None` の欄を残さない。
fn request() -> ChatRequest {
    let mut request = ChatRequest::new(vec![
        ChatMessage::system("あなたは葵です"),
        ChatMessage::user("こんにちは"),
    ]);
    request.temperature = Some(0.7);
    request.max_tokens = Some(300);
    request
}

fn completion_body() -> String {
    format!(
        r#"{{"id":"x","object":"chat.completion","created":1,"model":"{MODEL}",
            "choices":[{{"index":0,"message":{{"role":"assistant","content":"「はい」"}},
            "finish_reason":"stop"}}]}}"#
    )
}

fn json(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json")
}

/// llama.cpp を演じる偽の上流。チャット・組み上げ・数えるの3つの口を持つ。
async fn llama_like(tokens: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(json(&completion_body()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/apply-template"))
        .respond_with(json(
            &serde_json::json!({ "prompt": PLANTED_TEMPLATE }).to_string(),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/tokenize"))
        .respond_with(json(tokens))
        .mount(&server)
        .await;
    server
}

/// その口へ届いた要求の本文（届いた順）。
async fn bodies_to(server: &MockServer, wanted: &str) -> Vec<Vec<u8>> {
    server
        .received_requests()
        .await
        .expect("記録が有効であること")
        .into_iter()
        .filter(|request: &Request| request.url.path() == wanted)
        .map(|request| request.body)
        .collect()
}

/// 誰も待ち受けていないアドレス（上流が停止している状態）。
async fn closed_address() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("一時ポートを確保できること");
    let addr = listener.local_addr().expect("アドレスを取得できること");
    drop(listener);
    format!("http://{addr}")
}

/// ストリームを読み切る（本文がどこへ届いたかだけを見たいので中身は捨てる）。
async fn drain(mut stream: ChatStream) {
    while let Some(item) = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("ストリームが終端すること")
    {
        drop(item);
    }
}

// ---------------------------------------------------------------------- N6

/// **N6: 封の本文は、チャットへ送る本文と同じ**（ADR-0052 §推奨3 / §新しく要る見張り番 N6）。
///
/// **本文を別に組まない**（`reasoning_effort` 等がテンプレートの出力を変えうる）。
/// 比べるのは**偽の上流が実際に受け取ったバイト列**で、組み立ての関数ではない ——
/// 関数を比べると、「同じ関数を呼んだが別の引数を渡した」形が通る。
///
/// **ストリーミングの腕も回す**（`stream`・`stream_options` が本文に載るかどうかが変わる）。
/// そのうえで、**2つの腕の本文が互いに違うこと**も確かめる —— 同じなら、
/// `streaming` を無視して片方に決め打つ変異が通る。
#[tokio::test]
async fn the_seal_carries_the_same_body_as_the_chat_request() {
    let mut seen = Vec::new();
    for streaming in [false, true] {
        let server = llama_like(r#"{"tokens":[11,22,33,44]}"#).await;
        let client = client_for(&server.uri());
        let upstream: &dyn LlmClient = &client;

        let count = upstream
            .count_request(&request(), streaming)
            .await
            .expect("偽の上流は答えること");
        if streaming {
            drain(upstream.stream(&request()).await.expect("中継が始まること")).await;
        } else {
            upstream.complete(&request()).await.expect("補完が返ること");
        }

        assert_eq!(
            count,
            TokenCount::Counted(4),
            "組み上がりを数えた数を返すこと"
        );

        let chat = bodies_to(&server, "/v1/chat/completions").await;
        let sealed = bodies_to(&server, "/apply-template").await;
        assert_eq!(chat.len(), 1, "チャットへは1回だけ送ること");
        assert_eq!(sealed.len(), 1, "組み上げへは1回だけ送ること");
        assert_eq!(
            String::from_utf8_lossy(&sealed[0]),
            String::from_utf8_lossy(&chat[0]),
            "封の本文がチャットの本文と違う（streaming={streaming}）"
        );

        // **前提の確認**: 比べた本文が、落としうる欄を実際に持っていること。
        let sent: serde_json::Value =
            serde_json::from_slice(&chat[0]).expect("本文が JSON であること");
        for key in ["reasoning_effort", "temperature", "max_tokens", "messages"] {
            assert!(
                sent.get(key).is_some(),
                "比べた本文に {key} が無いなら、落とす変異を見ていない: {sent}"
            );
        }

        // **数えたのは組み上がりそのもの**（本文の JSON でも、メッセージの連結でもない）。
        let tokenized = bodies_to(&server, "/tokenize").await;
        assert_eq!(tokenized.len(), 1, "数える口へは1回だけ送ること");
        let tokenize: serde_json::Value =
            serde_json::from_slice(&tokenized[0]).expect("本文が JSON であること");
        assert_eq!(
            tokenize["content"], PLANTED_TEMPLATE,
            "組み上げが返した文字列をそのまま数えること"
        );
        assert_eq!(
            tokenize["with_pieces"], false,
            "破片付きの形を頼まないこと（返る形が数の列でなくなる）"
        );
        // **本番と同じ数え方**（`src/llm/lmstudio.rs` の `TokenizeWhat::add_special` と
        // `PARSE_SPECIAL` / ADR-0052 M1）。封は BOS を足し、制御トークンの綴りを
        // 1トークンとして読む —— この組で `usage.prompt_tokens` と一致した。
        assert_eq!(
            tokenize["add_special"], true,
            "封は BOS を足して数えること（足さないと Ministral で本番より1つ少ない / M1）"
        );
        assert_eq!(
            tokenize["parse_special"], true,
            "制御トークンの綴りは1トークンとして数えること（割ると多く数える / M1）"
        );

        seen.push(chat[0].clone());
    }
    assert_ne!(
        seen[0], seen[1],
        "2つの腕の本文が同じなら、`streaming` の取り違えを見ていない"
    );
}

/// **本文の経路が思考の量を載せた要求でも、封の本文はチャットの本文と同じ**
/// （[ADR-0059](../docs/adr/0059-thinking-and-response-room-in-the-upstream-profile.md) §決定1 の
/// 「封は本文の要求と同じ値を載せた本文で作る」/ 作業単位 A-18a）。
///
/// 上の [`the_seal_carries_the_same_body_as_the_chat_request`] は既定の腕（要求が上書きを持たない）。
/// **チャットテンプレートの出力は思考の有無で変わる**ので、封だけが既定の `none` で組まれると、数えた数が送った本文と
/// 食い違う。比べるのは偽の上流が受け取ったバイト列で、**両方に `"reasoning_effort":"low"` が載っていること**も見る
/// （両方とも落とした形が「同じ本文」で緑にならないように）。
#[tokio::test]
async fn the_seal_carries_the_reasoning_effort_the_chat_request_carries() {
    let thinking = ChatRequest {
        reasoning_effort: Some(personacore::llm::ReasoningEffort::Low),
        ..request()
    };
    for streaming in [false, true] {
        let server = llama_like(r#"{"tokens":[11,22,33,44]}"#).await;
        let client = client_for(&server.uri());
        let upstream: &dyn LlmClient = &client;

        upstream
            .count_request(&thinking, streaming)
            .await
            .expect("偽の上流は答えること");
        if streaming {
            drain(upstream.stream(&thinking).await.expect("中継が始まること")).await;
        } else {
            upstream.complete(&thinking).await.expect("補完が返ること");
        }

        let chat = bodies_to(&server, "/v1/chat/completions").await;
        let sealed = bodies_to(&server, "/apply-template").await;
        assert_eq!((chat.len(), sealed.len()), (1, 1), "streaming={streaming}");
        assert_eq!(
            String::from_utf8_lossy(&sealed[0]),
            String::from_utf8_lossy(&chat[0]),
            "封の本文がチャットの本文と違う（streaming={streaming}）"
        );
        for (label, body) in [("チャット", &chat[0]), ("封", &sealed[0])] {
            let sent: serde_json::Value =
                serde_json::from_slice(body).expect("本文が JSON であること");
            assert_eq!(
                sent["reasoning_effort"],
                serde_json::json!("low"),
                "{label}の本文が要求の思考の量を持たない（streaming={streaming}）: {sent}"
            );
        }
    }
}

/// **文字列ごとの口は、渡した文字列だけを数える**（`/apply-template` を通らない）。
#[tokio::test]
async fn counting_a_text_tokenizes_exactly_that_text() {
    let server = llama_like(r#"{"tokens":[5,6,7]}"#).await;
    let client = client_for(&server.uri());
    let upstream: &dyn LlmClient = &client;

    let text = "鍵の番号は f47ac10b-58cc-4372-a567-0e02b2c3d479";
    let count = upstream
        .count_text(text)
        .await
        .expect("偽の上流は答えること");

    assert_eq!(count, TokenCount::Counted(3));
    assert!(
        bodies_to(&server, "/apply-template").await.is_empty(),
        "文字列を数えるのに組み上げを通さないこと"
    );
    let tokenized = bodies_to(&server, "/tokenize").await;
    assert_eq!(tokenized.len(), 1);
    let tokenize: serde_json::Value =
        serde_json::from_slice(&tokenized[0]).expect("本文が JSON であること");
    assert_eq!(tokenize["content"], text);
    // **部品は全体の途中に置かれるので BOS を足さない**（封の腕と逆。取り違えを見る対）。
    assert_eq!(tokenize["add_special"], false, "部品に BOS を足さないこと");
    // **部品も封と同じく制御トークンの綴りを1トークンとして読む**（ADR-0052 決め直し1 ——
    // 本番は本文の中の綴りも制御トークンとして読んでいる）。`add_special` は封と逆、
    // `parse_special` は封と同じ —— 2つの欄の取り違えをこの組で止める。
    assert_eq!(
        tokenize["parse_special"], true,
        "部品も制御トークンの綴りを1トークンとして数えること（M1）"
    );
    assert_eq!(tokenize["with_pieces"], false);
}

// ---------------------------------------------------------------------- N8

/// **数の列ではない応答**（ADR-0052 §推奨4 / N8）。どれも 200 で返す。
///
/// 1行目は **LM Studio が未知の口に返す形を模したもの**（04-environment.md「200 ＋ エラー本文」。
/// 綴りそのものは確かめていない —— 確かめるのは M7）。
const NOT_A_TOKEN_LIST: [&str; 16] = [
    r#"{"error":"Unexpected endpoint or method. (POST /tokenize)"}"#,
    // 文字列そのもの / 数の列を文字列で。
    r#""11 22 33""#,
    r#"{"tokens":"11,22,33"}"#,
    // 負数・小数・文字列の混入・null の混入。
    r#"{"tokens":[11,-1,33]}"#,
    r#"{"tokens":[11,2.5,33]}"#,
    r#"{"tokens":[11,"22",33]}"#,
    r#"{"tokens":[11,null,33]}"#,
    // 入れ子（`with_pieces` の形 / 配列の配列）。
    r#"{"tokens":[{"id":11,"piece":"a"},{"id":22,"piece":"b"}]}"#,
    r#"{"tokens":[[11],[22]]}"#,
    // `tokens` が無い / null / オブジェクト。
    r#"{}"#,
    r#"{"tokens":null}"#,
    r#"{"tokens":{"0":11}}"#,
    // 一番外が配列（`tokens` に包まれていない）。
    r#"[11,22,33]"#,
    // JSON ではない / 空。
    "not json",
    "",
    // u64 に収まらない整数。
    r#"{"tokens":[18446744073709551616]}"#,
];

/// **N8: 数の列でない応答は、200 でも「数えられない」**（`a_count_shape_that_is_not_a_token_list_is_not_a_count`）。
///
/// **文字列ごとの口と本文ごとの口の両方**で回す（本文ごとの口は `/tokenize` まで進んでから形を見る）。
/// 「200 なら数えたことにする」変異はここで赤になる。
#[tokio::test]
async fn a_count_shape_that_is_not_a_token_list_is_not_a_count() {
    for body in NOT_A_TOKEN_LIST {
        let server = llama_like(body).await;
        let client = client_for(&server.uri());
        let upstream: &dyn LlmClient = &client;

        assert_eq!(
            upstream.count_text("こんにちは").await.ok(),
            Some(TokenCount::Uncountable),
            "数の列でない応答を数えた（文字列ごと）: {body:?}"
        );
        assert_eq!(
            upstream.count_request(&request(), false).await.ok(),
            Some(TokenCount::Uncountable),
            "数の列でない応答を数えた（本文ごと）: {body:?}"
        );
        assert_eq!(
            bodies_to(&server, "/tokenize").await.len(),
            2,
            "前提: 2つの口とも数える口まで届いていること: {body:?}"
        );
    }
}

/// **対照: 数の列なら数える**（上の表が「何でも数えない」で緑になっていないこと）。
///
/// 0 件の列は**空の文字列に対してだけ**数として受け取る。
#[tokio::test]
async fn a_token_list_is_a_count() {
    for (body, text, expected) in [
        (r#"{"tokens":[1]}"#, "a", TokenCount::Counted(1)),
        (
            r#"{"tokens":[0,151643,4294967296]}"#,
            "abc",
            TokenCount::Counted(3),
        ),
        (r#"{"tokens":[]}"#, "", TokenCount::Counted(0)),
        // **中身のある文字列を 0 と数えた応答は信じない**（窓を超えて送る向きの誤り）。
        (r#"{"tokens":[]}"#, "abc", TokenCount::Uncountable),
    ] {
        let server = llama_like(body).await;
        let client = client_for(&server.uri());
        assert_eq!(
            client.count_text(text).await.ok(),
            Some(expected),
            "{body:?} / {text:?}"
        );
    }
}

/// **組み上げの口が組み上がりを返さなければ、数えに行かない**（LM Studio の 200 ＋ エラー本文を含む）。
#[tokio::test]
async fn a_template_shape_that_is_not_a_prompt_is_not_a_count() {
    for body in [
        r#"{"error":"Unexpected endpoint or method. (POST /apply-template)"}"#,
        r#"{"prompt":42}"#,
        r#"{"prompt":null}"#,
        r#"{"prompt":["a"]}"#,
        // 空の組み上がりは数えない（0 トークンの封は、窓を超えて送る向きの誤り）。
        r#"{"prompt":""}"#,
        r#"{}"#,
        "not json",
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/apply-template"))
            .respond_with(json(body))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(json(r#"{"tokens":[1,2,3]}"#))
            .mount(&server)
            .await;
        let client = client_for(&server.uri());

        assert_eq!(
            client.count_request(&request(), false).await.ok(),
            Some(TokenCount::Uncountable),
            "組み上がりでない応答から数えた: {body:?}"
        );
        assert!(
            bodies_to(&server, "/tokenize").await.is_empty(),
            "組み上がりが無いのに数える口を叩いた: {body:?}"
        );
    }
}

/// **4xx は、形が数の列でも数えない**（llama.cpp の 404 / 断られた回）。
///
/// 判定は形で行うが、**2xx 以外の本文を数として信じるほうへは倒さない**（保守的な側）。
///
/// **⚠ B-60a では 500 / 503 もこの表にあり、`Uncountable` を期待していた。**
/// **作業単位 B-60b のレビューで 5xx を `Err` へ分けた**（下の
/// [`a_server_error_is_a_failed_round_trip_not_an_uncountable_upstream`]）。
/// 読み込み中の 503 を「数えられない上流」と取り違えると、`server::seal` がその世代の間もう数えない。
/// **この見張り番が主張するのは「4xx は数えられない」になった。**
#[tokio::test]
async fn an_unsuccessful_status_is_not_a_count_even_with_a_token_list() {
    for status in [400_u16, 404, 405] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_raw(r#"{"tokens":[1,2,3]}"#, "application/json"),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/apply-template"))
            .respond_with(
                ResponseTemplate::new(status).set_body_raw(r#"{"prompt":"x"}"#, "application/json"),
            )
            .mount(&server)
            .await;
        let client = client_for(&server.uri());

        assert_eq!(
            client.count_text("こんにちは").await.ok(),
            Some(TokenCount::Uncountable),
            "{status} を数えた（文字列ごと）"
        );
        assert_eq!(
            client.count_request(&request(), true).await.ok(),
            Some(TokenCount::Uncountable),
            "{status} を数えた（本文ごと）"
        );
    }
}

/// **5xx は「数えられない上流」ではなく、往復の失敗**（`Err(LlmError::Status)` / 作業単位 B-60b のレビュー）。
///
/// llama.cpp は読み込み中、どの口にも 503 を返す。**これを `Uncountable` に畳むと、
/// 呼び出し側は「数えられない上流だ」と覚えうる**（本当は今回聞けなかっただけ）。
///
/// **本文ごとの口は、1往復目（組み上げ）でも2往復目（数える）でも 5xx なら `Err`。**
/// 2往復目の腕は、組み上げが即答し数える口だけが 5xx を返す形で作る。
#[tokio::test]
async fn a_server_error_is_a_failed_round_trip_not_an_uncountable_upstream() {
    for status in [500_u16, 502, 503] {
        let refusing = || {
            ResponseTemplate::new(status).set_body_raw(
                r#"{"error":{"code":503,"message":"Loading model"}}"#,
                "application/json",
            )
        };

        // 1往復目（組み上げ）も数える口も 5xx。
        let server = MockServer::start().await;
        for wanted in ["/apply-template", "/tokenize"] {
            Mock::given(method("POST"))
                .and(path(wanted))
                .respond_with(refusing())
                .mount(&server)
                .await;
        }
        let client = client_for(&server.uri());
        assert!(
            matches!(
                client.count_text("こんにちは").await,
                Err(LlmError::Status { status: got }) if got == status
            ),
            "{status} を往復の失敗にすること（文字列ごと）"
        );
        assert!(
            matches!(
                client.count_request(&request(), true).await,
                Err(LlmError::Status { status: got }) if got == status
            ),
            "{status} を往復の失敗にすること（本文ごと・1往復目）"
        );

        // 2往復目（数える口）だけが 5xx。
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/apply-template"))
            .respond_with(json(
                &serde_json::json!({ "prompt": PLANTED_TEMPLATE }).to_string(),
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(refusing())
            .mount(&server)
            .await;
        let client = client_for(&server.uri());
        assert!(
            matches!(
                client.count_request(&request(), false).await,
                Err(LlmError::Status { status: got }) if got == status
            ),
            "{status} を往復の失敗にすること（本文ごと・2往復目）"
        );
        assert_eq!(
            bodies_to(&server, "/tokenize").await.len(),
            1,
            "前提: 2往復目まで進んでいること"
        );
    }
}

/// **口が無い上流**（何も載せていない偽物 = すべて 404）は「数えられない」。
#[tokio::test]
async fn an_upstream_without_the_counting_paths_cannot_count() {
    let server = MockServer::start().await;
    let client = client_for(&server.uri());
    assert_eq!(
        client.count_text("こんにちは").await.ok(),
        Some(TokenCount::Uncountable)
    );
    assert_eq!(
        client.count_request(&request(), false).await.ok(),
        Some(TokenCount::Uncountable)
    );
}

// ---------------------------------------------------------------------- 往復の失敗

/// **往復そのものが失敗したら `Err`**（「数えられない上流」とは区別する。覚えるかどうかは呼び出し側）。
#[tokio::test]
async fn a_stopped_upstream_is_an_error_not_an_uncountable_upstream() {
    let client = client_for(&closed_address().await);
    assert!(
        matches!(client.count_text("a").await, Err(LlmError::Connect(_))),
        "停止した上流は接続の失敗であること"
    );
    assert!(
        matches!(
            client.count_request(&request(), false).await,
            Err(LlmError::Connect(_))
        ),
        "停止した上流は接続の失敗であること"
    );
}

/// **数える往復は照会の上限で切る**（新しい定数を足さない / ADR-0052 §推奨4）。
///
/// 本文生成の上限（既定 180 秒の天井）を引き継ぐと、数える口が黙った回に
/// ターンが数分止まる。**切れてもバイト数に戻るだけ**なので短い上限でよい。
#[tokio::test]
async fn counting_is_bounded_by_the_query_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/tokenize"))
        .respond_with(json(r#"{"tokens":[1]}"#).set_delay(Duration::from_secs(10)))
        .mount(&server)
        .await;
    let model = ModelId::parse(MODEL).expect("正当な識別子");
    let config = LmStudioConfig::new(&server.uri(), model)
        .expect("正当な URL")
        .with_context_timeout(Duration::from_millis(300));
    let client = LmStudioClient::new(config).expect("構築できること");

    let started = std::time::Instant::now();
    let result = client.count_text("a").await;
    assert!(
        matches!(result, Err(LlmError::Timeout(_))),
        "照会の上限で切れること: {result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "照会の上限より長く待った: {:?}",
        started.elapsed()
    );
}

/// **本文ごとの口の2往復目（数える口）が失敗しても `Err`**（「数えられない上流」に畳まない）。
///
/// 組み上げの口は即答し、数える口だけを照会の上限より遅らせる。
/// 2往復目の失敗を `Uncountable` に畳むと、呼び出し側は「数えられない上流だ」と覚えうる
/// （本当は今回聞けなかっただけ）。
#[tokio::test]
async fn a_failure_on_the_second_round_trip_of_a_request_count_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/apply-template"))
        .respond_with(json(
            &serde_json::json!({ "prompt": PLANTED_TEMPLATE }).to_string(),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/tokenize"))
        .respond_with(json(r#"{"tokens":[1]}"#).set_delay(Duration::from_secs(10)))
        .mount(&server)
        .await;
    let model = ModelId::parse(MODEL).expect("正当な識別子");
    let config = LmStudioConfig::new(&server.uri(), model)
        .expect("正当な URL")
        .with_context_timeout(Duration::from_millis(300));
    let client = LmStudioClient::new(config).expect("構築できること");

    let result = client.count_request(&request(), false).await;
    assert!(
        matches!(result, Err(LlmError::Timeout(_))),
        "数える口の時間切れは往復の失敗であること: {result:?}"
    );
    assert_eq!(
        bodies_to(&server, "/apply-template").await.len(),
        1,
        "前提: 1往復目（組み上げ）は届いていること"
    );
}

// ---------------------------------------------------------------------- 既定の実装

/// 数える口を実装しないモック（既存のモックと同じ形）。
struct ChatOnly;

impl LlmClient for ChatOnly {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        Box::pin(async {
            Ok(Completion {
                content: String::new(),
                finish_reason: None,
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        Box::pin(async { Ok(Box::pin(futures_util::stream::empty()) as ChatStream) })
    }
}

/// **既定の実装は「数えられない」**（ADR-0052 §作業単位 B-60a）。
///
/// 既存のモックや他の上流は、何も足さずに今のバイト数の計画のまま動く。
/// **数えたふり（0 や何かの数を返す）をすると、呼び出し側は封をしたと信じる。**
#[tokio::test]
async fn the_default_implementation_cannot_count() {
    let upstream: &dyn LlmClient = &ChatOnly;
    assert_eq!(
        upstream.count_text("こんにちは").await.ok(),
        Some(TokenCount::Uncountable)
    );
    for streaming in [false, true] {
        assert_eq!(
            upstream.count_request(&request(), streaming).await.ok(),
            Some(TokenCount::Uncountable)
        );
    }
}
