//! `llm` モジュール（LM Studio クライアント）の結合検証。
//!
//! **実際の LM Studio には一切繋がない**（docs/05-development.md）。HTTP レベルは
//! `wiremock`、「本文を少しずつ送る」挙動が要る検証だけは生の TCP サーバを立てる。
//!
//! ここで確かめるのは主に次の8点:
//!   * 非ストリーミングの正常系と、**上流の応答が壊れていても panic しない**こと。
//!   * **接続失敗 / タイムアウト / HTTP エラー / 壊れた JSON / 空のペイロード**が
//!     それぞれ区別できるエラーになること。**LM Studio の停止は想定された分岐**であり、
//!     他のエラーと混ざってはいけない。
//!   * エラーの `Display` に**エンドポイントも応答本文も漏れない**こと（CLAUDE.md 絶対制約）。
//!     `#[source]` に `reqwest::Error` を持つ3つ（`Connect` / `Timeout` / `Transport`）は
//!     その `reqwest::Error` を外から作れないため、**文言の検査はここでしかできない。**
//!     （4つ目の `Decode` は `serde_json::Error` なので `src/llm/error.rs` 側で検査する。）
//!   * **チャンクが逐次流れる**こと。上流が本文を送り終える前に下流へ届く
//!     （全部受信してから返す実装だと `chunks_arrive_before_the_upstream_finishes` が落ちる）。
//!   * **`[DONE]` で正常終端し、上流が途中で切れたら終端が下流へ伝わる**こと。
//!     どちらの場合もストリームは必ず `None` で終わる（宙吊りにしない）。
//!   * **3つの上限がそれぞれ効く場所**（`llm_timeouts` 節）。ここが一番壊れやすい:
//!     接続の上限 / 無通信の上限 / 非ストリーミングだけの全体上限、の役割分担。
//!   * **読み切らずに `ChatStream` を drop すると、上流への TCP が閉じる**こと（11c-3b）。
//!     切断時キャンセルの上流側の1段。**測ってからでないと言えない**ので生の TCP で見る。
//!   * **トレイト越しにモックを注入できる**こと（`Arc<dyn LlmClient>` で使えること）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use futures_util::stream;
use personacore::llm::{
    ChatChunk, ChatMessage, ChatRequest, ChatRole, ChatStream, Completion, FinishReason, LlmClient,
    LlmError, LlmFuture, LmStudioClient, LmStudioConfig, ModelId, TokenUsage,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// テストで使う上流モデル。docs/archive/04-environment-lm-studio.md の実測値。
const MODEL: &str = "ministral-3-14b-instruct-2512";

/// フィクスチャの `usage.prompt_tokens`。**`completion_tokens` / `total_tokens` とは
/// 必ず違う値にすること**（どれを読んでいるかを区別するため / ADR-0013 第3層）。
const PROMPT_TOKENS: u32 = 37;

/// ストリームの1項目を待つときの上限。
///
/// **これが逐次性の検証の核。** 上流が本文を送り終えていない時点で `next()` が
/// 返らなければ、実装が全部溜め込んでいることになる。
const NEXT_ITEM_LIMIT: Duration = Duration::from_secs(5);

fn model() -> ModelId {
    ModelId::parse(MODEL).expect("テストで使うモデル識別子は正当であること")
}

/// 指定のベース URL を向くクライアント（モデルは `MODEL`）。
fn client_for(base_url: &str) -> LmStudioClient {
    client_for_model(base_url, MODEL)
}

/// 指定のベース URL と**モデル**を向くクライアント。
///
/// `PERSONACORE_MAIN_MODEL` で既定以外のモデルを設定した状態の再現に使う。
fn client_for_model(base_url: &str, model_id: &str) -> LmStudioClient {
    let model = ModelId::parse(model_id).expect("テストで使うモデル識別子は正当であること");
    let config = LmStudioConfig::new(base_url, model).expect("ベース URL が正当であること");
    LmStudioClient::new(config).expect("HTTP クライアントを構築できること")
}

fn request() -> ChatRequest {
    ChatRequest::new(vec![
        ChatMessage::system("あなたは葵です"),
        ChatMessage::user("こんにちは"),
    ])
}

/// 正常系の非ストリーミング応答。
///
/// **`usage` の3項目は必ず互いに違う値にしておくこと**（ADR-0013 第3層）。
/// 揃っていると、`prompt_tokens` を読んでいるのか `completion_tokens` /
/// `total_tokens` を読んでいるのかがテストから区別できない。
fn completion_body(content: &str, finish_reason: &str) -> String {
    format!(
        r#"{{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"{MODEL}",
            "choices":[{{"index":0,"message":{{"role":"assistant","content":"{content}"}},
            "finish_reason":"{finish_reason}"}}],
            "usage":{{"prompt_tokens":{PROMPT_TOKENS},"completion_tokens":11,
                "total_tokens":48}}}}"#
    )
}

/// SSE の1イベント分のテキスト。
fn sse_delta(content: &str) -> String {
    format!(
        "data: {{\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1,\
         \"model\":\"{MODEL}\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{content}\"}},\
         \"finish_reason\":null}}]}}\n\n"
    )
}

/// `POST /v1/chat/completions` に固定応答を返すモックサーバ。
async fn mock_server(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

/// SSE 形式の本文を一括で返すモックサーバ。
async fn sse_server(body: String) -> MockServer {
    mock_server(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")).await
}

/// 誰も待ち受けていないアドレス。**LM Studio が停止している状態の再現。**
async fn closed_address() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("一時ポートを確保できること");
    let addr = listener.local_addr().expect("アドレスを取得できること");
    drop(listener);
    format!("http://{addr}")
}

/// `stream()` の失敗を取り出す。
///
/// `ChatStream` は `Debug` を実装しないので `expect_err` が使えない
/// （ストリームの中身を覗ける `Debug` を生やすと、上流の出力がログに滲む）。
fn stream_error(result: Result<ChatStream, LlmError>, reason: &str) -> LlmError {
    match result {
        Ok(_) => panic!("{reason}"),
        Err(error) => error,
    }
}

/// `#[source]` に `reqwest::Error` を持つバリアントの `Display`。
/// **文言は定数**（CLAUDE.md 絶対制約）。
///
/// `reqwest::Error` は外から構築できないので、**この3つの文言を検査できるのはここだけ**
/// （`src/llm/error.rs` の単体テストからは、実際に通信しないと作れない）。
/// 4つ目の `Decode` は `serde_json::Error` なのであちらで完全一致を検査している
/// （`decode_display_does_not_reflect_the_upstream_body`）。
const CONNECT_MESSAGE: &str = "推論サーバへ接続できませんでした";
const TIMEOUT_MESSAGE: &str = "推論サーバの応答が時間内に完了しませんでした";
const TRANSPORT_MESSAGE: &str = "推論サーバとの通信に失敗しました";

/// `#[source]` の中身が `Display` に滲み出していないこと。
///
/// **「URL を含まない」程度の検査では足りない。** `#[error("...: {0}")]` を1つ足したとき、
/// 漏れる内容は `source` の種類次第（body 側の失敗は URL を持たない）で、
/// 緩い検査は素通りする。**定数と完全一致することだけが確実な歯止め。**
/// 文言を変えるなら、それは意図的な変更としてここも直すこと。
fn assert_constant_message(error: &LlmError, expected: &str) {
    assert_eq!(
        error.to_string(),
        expected,
        "Display は定数文字列であること（source を混ぜない）: {error:?}"
    );
}

/// ストリームを最後まで読み切る。**`None` で終わらなければここで止まる**ので、
/// 宙吊りを見逃さないよう `NEXT_ITEM_LIMIT` で打ち切る。
async fn drain(stream: &mut ChatStream) -> Vec<Result<ChatChunk, LlmError>> {
    let mut items = Vec::new();
    loop {
        let next = tokio::time::timeout(NEXT_ITEM_LIMIT, stream.next())
            .await
            .expect("ストリームが終端すること（宙吊りにならないこと）");
        match next {
            Some(item) => items.push(item),
            None => return items,
        }
    }
}

// ---------------------------------------------------------------- 非ストリーミング

/// 正常系。応答の中身と、**上流へ送ったリクエストの中身**の両方を確かめる。
#[tokio::test]
async fn completes_a_non_streaming_request() {
    let server = mock_server(
        ResponseTemplate::new(200)
            .set_body_raw(completion_body("こんにちは", "stop"), "application/json"),
    )
    .await;

    let completion = client_for(&server.uri())
        .complete(&request())
        .await
        .expect("正常な応答を解釈できること");

    assert_eq!(completion.content, "こんにちは");
    assert_eq!(completion.finish_reason, Some(FinishReason::Stop));
    // **答え合わせの真値が HTTP の往復を越えて届くこと**（ADR-0013 第3層）。
    // `parse_completion` の単体テストは、`complete` がその結果を捨てても緑のままになる。
    assert_eq!(
        completion.usage,
        Some(TokenUsage {
            prompt_tokens: PROMPT_TOKENS,
            // フィクスチャの `completion_tokens`（A-18b から読む / 捨てた1回目の生成数）。
            completion_tokens: Some(11),
        }),
        "上流が数えた prompt_tokens を落とさないこと"
    );

    let requests = server
        .received_requests()
        .await
        .expect("記録が有効であること");
    assert_eq!(requests.len(), 1, "上流へは1回だけ投げること");
    let sent: serde_json::Value =
        serde_json::from_slice(&requests[0].body).expect("送信本文が JSON であること");
    assert_eq!(sent["model"], MODEL, "モデル識別子は設定から来ること");
    assert_eq!(sent["stream"], false, "非ストリーミングであること");
    // **使用量の追加要求は送らない。** 非ストリーミングでは `usage` が本文と一緒に
    // 返るので不要で、拒否する上流がありうる。
    assert!(
        sent.get("stream_options").is_none(),
        "非ストリーミングでは stream_options を送らないこと: {sent}"
    );
    assert_eq!(sent["messages"][0]["role"], "system");
    assert_eq!(sent["messages"][0]["content"], "あなたは葵です");
    assert_eq!(sent["messages"][1]["role"], "user");
    assert_eq!(sent["messages"][1]["content"], "こんにちは");
}

/// HTTP エラーはステータスを保持したまま区別できる。
///
/// **`Display` に URL も応答本文も載せない**（CLAUDE.md 絶対制約）。
#[tokio::test]
async fn http_error_status_is_distinguishable() {
    for status in [400_u16, 404, 429, 500, 503] {
        let server = mock_server(
            ResponseTemplate::new(status)
                .set_body_raw(r#"{"error":"model not found: secret"}"#, "application/json"),
        )
        .await;

        let error = client_for(&server.uri())
            .complete(&request())
            .await
            .expect_err("HTTP エラーは失敗として返ること");

        match &error {
            LlmError::Status { status: got } => assert_eq!(*got, status),
            other => panic!("HTTP エラーとして区別されること: {other:?}"),
        }
        assert_eq!(error.status(), Some(status));

        let shown = error.to_string();
        assert!(!shown.is_empty(), "説明があること");
        assert!(!shown.contains("http"), "URL が漏れないこと: {shown}");
        assert!(
            !shown.contains("127.0.0.1"),
            "ホストが漏れないこと: {shown}"
        );
        assert!(!shown.contains("secret"), "応答本文が漏れないこと: {shown}");
        assert!(
            !shown.contains(&status.to_string()),
            "Display は定数文字列であること: {shown}"
        );
    }
}

/// 壊れた JSON は `Decode`。**panic しない。**
#[tokio::test]
async fn broken_json_is_a_decode_error() {
    let server =
        mock_server(ResponseTemplate::new(200).set_body_raw("{\"choices\": [", "application/json"))
            .await;

    let error = client_for(&server.uri())
        .complete(&request())
        .await
        .expect_err("壊れた JSON は失敗として返ること");

    assert!(
        matches!(error, LlmError::Decode(_)),
        "JSON の破損として区別されること: {error:?}"
    );
    // **この検査は緩い。** 漏れる内容は上流が何を返したか次第なので、特定の語では
    // 捕まえきれない（`{0}` を足しても素通りする）。定数との完全一致による本当の歯止めは
    // `src/llm/error.rs` の `decode_display_does_not_reflect_the_upstream_body`。
    assert!(
        !error.to_string().contains("choices"),
        "応答本文が Display に漏れないこと"
    );
}

/// 本文の無い応答（`choices` が空）は `MissingContent`。`Decode` とは別物。
#[tokio::test]
async fn empty_choices_is_missing_content() {
    let server = mock_server(
        ResponseTemplate::new(200).set_body_raw(r#"{"id":"1","choices":[]}"#, "application/json"),
    )
    .await;

    let error = client_for(&server.uri())
        .complete(&request())
        .await
        .expect_err("本文が無ければ失敗として返ること");

    assert!(
        matches!(error, LlmError::MissingContent),
        "空の応答として区別されること: {error:?}"
    );
}

/// **LM Studio が起動していないのは正常な状態。** 接続失敗として区別できること。
#[tokio::test]
async fn unreachable_upstream_is_a_connect_error() {
    let error = client_for(&closed_address().await)
        .complete(&request())
        .await
        .expect_err("接続できなければ失敗として返ること");

    assert!(
        matches!(error, LlmError::Connect(_)),
        "接続失敗として区別されること: {error:?}"
    );
    assert_constant_message(&error, CONNECT_MESSAGE);
}

/// 応答が返らないケースはタイムアウトとして区別できる。
#[tokio::test]
async fn slow_upstream_is_a_timeout() {
    let server = mock_server(
        ResponseTemplate::new(200)
            .set_body_raw(completion_body("遅い", "stop"), "application/json")
            .set_delay(Duration::from_secs(30)),
    )
    .await;

    let config = LmStudioConfig::new(&server.uri(), model())
        .expect("ベース URL が正当であること")
        .with_request_timeout(Duration::from_millis(200));
    let error = LmStudioClient::new(config)
        .expect("HTTP クライアントを構築できること")
        .complete(&request())
        .await
        .expect_err("時間内に返らなければ失敗として返ること");

    assert!(
        matches!(error, LlmError::Timeout(_)),
        "タイムアウトとして区別されること: {error:?}"
    );
    assert_constant_message(&error, TIMEOUT_MESSAGE);
}

// ---------------------------------------------------------------- ストリーミング

/// チャンクが順序どおり流れ、`[DONE]` で終端する。**`[DONE]` 自体は下流へ出さない。**
#[tokio::test]
async fn streams_chunks_in_order_and_ends_at_done() {
    let body = format!(
        "{}{}{}data: [DONE]\n\n",
        sse_delta("こん"),
        sse_delta("にち"),
        format_args!(
            "data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n"
        ),
    );
    let server = sse_server(body).await;

    let mut chat_stream = client_for(&server.uri())
        .stream(&request())
        .await
        .expect("ストリームを開けること");
    let items = drain(&mut chat_stream).await;

    let chunks: Vec<ChatChunk> = items
        .into_iter()
        .map(|item| item.expect("正常系ではエラーが混ざらないこと"))
        .collect();
    assert_eq!(chunks.len(), 3, "[DONE] は下流へ出さないこと: {chunks:?}");
    assert_eq!(chunks[0].delta, "こん");
    assert_eq!(chunks[1].delta, "にち");
    assert_eq!(chunks[2].delta, "");
    assert_eq!(chunks[2].finish_reason, Some(FinishReason::Stop));

    // 終端後にもう一度読んでも `None` のまま（宙吊りにならない）。
    assert!(
        tokio::time::timeout(NEXT_ITEM_LIMIT, chat_stream.next())
            .await
            .expect("終端後も待たされないこと")
            .is_none()
    );

    // **上流に SSE を要求していること。** ここが `false` だと、本物の LM Studio は
    // 一括の JSON を返す（モックは気にしないので、この assert だけが違いを見る）。
    let requests = server
        .received_requests()
        .await
        .expect("記録が有効であること");
    let sent: serde_json::Value =
        serde_json::from_slice(&requests[0].body).expect("送信本文が JSON であること");
    assert_eq!(sent["stream"], true, "上流にストリーミングを要求すること");
    // **使用量を明示的に要求していること**（ADR-0013 第3層 / ストリーミング）。
    // 送らないと LM Studio は最終チャンクに `usage` を載せない（2026-08-15 実測）ので、
    // 中継経路の答え合わせが**材料を一度も受け取れないまま黙り続ける。**
    assert_eq!(
        sent["stream_options"],
        serde_json::json!({"include_usage": true}),
        "上流に使用量を要求すること: {sent}"
    );
}

/// **どちらの呼び方でも、上流へ出た HTTP 本文に「思考しない」が載る**
/// （ADR-0038 §決定3 / 作業単位 A-7b）。
///
/// # なぜ単体テストでは足りないか
///
/// `wire_request` の単体テストは、`post` がその結果を捨てて別の本文を組み立てても
/// 緑のままになる（`stream_options` の見張り番が既に同じ理由でここに居る）。
/// **本当に上流へ届いたバイト列**を見るのは、この層だけ。
///
/// # 綴りも値も、ここでは定数から作らない
///
/// 期待値は `"reasoning_effort"` と `"none"` を**直に書く。**
/// 実装側の定数から作ると、定数を書き換える変異と一緒に期待値も動いて緑になる
/// （CLAUDE.md「関数の外にある値で押さえる見張り番」）。
#[tokio::test]
async fn both_call_shapes_ask_the_upstream_not_to_think() {
    // 非ストリーミング（裏方の抽出・変容が通る形）。
    let plain = mock_server(
        ResponseTemplate::new(200)
            .set_body_raw(completion_body("はい", "stop"), "application/json"),
    )
    .await;
    client_for(&plain.uri())
        .complete(&request())
        .await
        .expect("正常な応答を解釈できること");

    // ストリーミング（本文生成の中継が通る形）。
    let streaming = sse_server(format!("{}data: [DONE]\n\n", sse_delta("こん"))).await;
    let mut chat_stream = client_for(&streaming.uri())
        .stream(&request())
        .await
        .expect("ストリームを開けること");
    drain(&mut chat_stream).await;

    for (label, server, expected_stream) in [
        ("非ストリーミング", &plain, false),
        ("ストリーミング", &streaming, true),
    ] {
        let requests = server
            .received_requests()
            .await
            .expect("記録が有効であること");
        assert_eq!(requests.len(), 1, "{label}: 上流へは1回だけ投げること");
        let sent: serde_json::Value =
            serde_json::from_slice(&requests[0].body).expect("送信本文が JSON であること");
        // **同じ本文の中で**、呼び方と欄の両方を見る（別の往復の欄を拾わない）。
        assert_eq!(
            sent["stream"],
            serde_json::json!(expected_stream),
            "{label}: 呼び方を取り違えていないこと: {sent}"
        );
        assert_eq!(
            sent["reasoning_effort"],
            serde_json::json!("none"),
            "{label}: 上流に思考しないよう頼むこと: {sent}"
        );
    }
}

/// **要求が思考の量を載せたら、どちらの呼び方でもその値が上流へ出る**
/// （[ADR-0059](../docs/adr/0059-thinking-and-response-room-in-the-upstream-profile.md) §決定1 / 作業単位 A-18a）。
///
/// 上の `both_call_shapes_ask_the_upstream_not_to_think` が既定の腕（要求が上書きを持たない＝裏方の形）で、
/// これは書いた面の腕（本文の経路の形）。**`post` が要求の上書きを捨てて別の本文を組む実装は、この層でしか見えない。**
/// 期待値は `"low"` を直に書く。
#[tokio::test]
async fn both_call_shapes_carry_the_requested_reasoning_effort() {
    let thinking = ChatRequest {
        reasoning_effort: Some(personacore::llm::ReasoningEffort::Low),
        ..request()
    };
    let plain = mock_server(
        ResponseTemplate::new(200)
            .set_body_raw(completion_body("はい", "stop"), "application/json"),
    )
    .await;
    client_for(&plain.uri())
        .complete(&thinking)
        .await
        .expect("正常な応答を解釈できること");
    let streaming = sse_server(format!("{}data: [DONE]\n\n", sse_delta("こん"))).await;
    let mut chat_stream = client_for(&streaming.uri())
        .stream(&thinking)
        .await
        .expect("ストリームを開けること");
    drain(&mut chat_stream).await;

    for (label, server) in [("非ストリーミング", &plain), ("ストリーミング", &streaming)]
    {
        let requests = server
            .received_requests()
            .await
            .expect("記録が有効であること");
        assert_eq!(requests.len(), 1, "{label}: 上流へは1回だけ投げること");
        let sent: serde_json::Value =
            serde_json::from_slice(&requests[0].body).expect("送信本文が JSON であること");
        assert_eq!(
            sent["reasoning_effort"],
            serde_json::json!("low"),
            "{label}: 要求が載せた思考の量を送ること: {sent}"
        );
    }
}

/// **上流へ出た HTTP 本文に、サンプラー5欄が綴りどおりに載る**
/// （ADR-0038 §決定2 / 作業単位 A-7c。`presence_penalty` は A-14 で足した）。
///
/// # なぜ単体テストでは足りないのか
///
/// `wire_request` の単体テストが見ているのは**構造体を直列化した文字列**までで、
/// **`post` がそれをどう送るか**は見ていない。
/// 実測（A-7c）: `.json(&body)` を `.json(&serde_json::to_value(&body))` に変えると、
/// **線に乗るのは `"top_p":0.949999988079071` になり、`#[serde(flatten)]` の重複キーも畳まれる。**
/// **つまりこの作業単位の成果（綴りの一致・重複の検出）が本番経路でだけ失われるのに、
/// 単体テストは全部緑だった。**
///
/// **`reasoning_effort` について A-7b が「モック注入では見られない」と書いた理由が、
/// そのままサンプラー4欄にも当たる**（`both_call_shapes_ask_the_upstream_not_to_think`）。
///
/// # 見るのは受信した生のバイト列
///
/// `serde_json::from_slice` で読み直すと `0.95` と `0.949999988079071` の区別は付くが、
/// **重複キーは畳まれる。** ここでは**受け取ったバイト列そのもの**を見る。
///
/// **期待値は実装側の定数から作らない**（CLAUDE.md「関数の外にある値で押さえる見張り番」）。
/// **欄名も値も直に書く。**
#[tokio::test]
async fn both_call_shapes_carry_the_explicit_samplers_on_the_wire() {
    // 非ストリーミング（裏方の抽出・変容が通る形）。
    let plain = mock_server(
        ResponseTemplate::new(200)
            .set_body_raw(completion_body("はい", "stop"), "application/json"),
    )
    .await;
    client_for(&plain.uri())
        .complete(&request())
        .await
        .expect("正常な応答を解釈できること");

    // ストリーミング（本文生成の中継が通る形）。
    let streaming = sse_server(format!("{}data: [DONE]\n\n", sse_delta("こん"))).await;
    let mut chat_stream = client_for(&streaming.uri())
        .stream(&request())
        .await
        .expect("ストリームを開けること");
    drain(&mut chat_stream).await;

    for (label, server, expected_stream) in [
        ("非ストリーミング", &plain, false),
        ("ストリーミング", &streaming, true),
    ] {
        let requests = server
            .received_requests()
            .await
            .expect("記録が有効であること");
        assert_eq!(requests.len(), 1, "{label}: 上流へは1回だけ投げること");
        let raw = std::str::from_utf8(&requests[0].body).expect("送信本文が UTF-8 であること");

        // **同じ本文の中で**、呼び方と欄の両方を見る（別の往復の欄を拾わない）。
        assert!(
            has_json_field(
                raw,
                "stream",
                if expected_stream { "true" } else { "false" }
            ),
            "{label}: 呼び方を取り違えていないこと: {raw}"
        );

        // **5欄を1つずつ。代表1つで済ませない**
        // （CLAUDE.md「写しから列が1つだけ落ちて 262 全緑」）。
        for (field, value) in [
            ("top_k", "40"),
            ("top_p", "0.95"),
            ("min_p", "0.05"),
            ("repeat_penalty", "1.1"),
            // A-14: 上流がモデルカードの既定（鍵で守られた中継の先の推論サーバは 1.5）を重ねないよう 0 を明示する。
            ("presence_penalty", "0.0"),
        ] {
            assert!(
                has_json_field(raw, field, value),
                "{label}: {field} が {value} の綴りで線に乗ること: {raw}"
            );
            // **同じ欄が2度出ていない**こと（`#[serde(flatten)]` の重複キーは
            // 読み直すと畳まれるので、バイト列の側でしか見えない）。
            assert_eq!(
                raw.matches(&format!("\"{field}\"")).count(),
                1,
                "{label}: {field} は1度だけ載ること: {raw}"
            );
        }
    }
}

/// **依頼が持つ温度は、線に綴りどおり1度だけ乗る**（ADR-0038 §決定2 / 作業単位 A-7d）。
///
/// # なぜ単体テストでは足りないのか
///
/// `wire_request` の単体テストが見ているのは**構造体を直列化した文字列**までで、
/// **`post` がそれをどう送るか**は見ていない（A-7c が
/// `both_call_shapes_carry_the_explicit_samplers_on_the_wire` で実測した穴）。
/// **裏方の温度はサンプラー4欄より後に足した欄**なので、
/// **その穴に乗っているかどうかは別に測る必要がある。**
///
/// 実測（A-7d）: `.json(&body)` を `.json(&serde_json::to_value(&body))` に変えると、
/// **線に乗るのは `"temperature":0.800000011920929` になる。**
/// **`0.8_f32` と同じ値ではあるが、綴りは別物**で、
/// **[ADR-0038] §決定2 の目的（測定が再現すること）はバイト列の話**である。
///
/// # ここが見ているもの／見ていないもの
///
/// **見ている**: **`llm` の層が、受け取った温度を綴りどおりに運ぶこと**と、
/// **重複キーになっていないこと**（`#[serde(flatten)]` は同じ名前の欄を畳まない）。
///
/// **⚠ 見ていない**: **どの経路がどの値を使うか。** ここは
/// [`ChatRequest`] に入れた値をそのまま観測しているだけなので、
/// **帰属の権威は `tests/server_stat_drift.rs` の
/// `each_path_sends_its_own_temperature`**（本番の入口を通る）。
///
/// [ADR-0038]: ../docs/adr/0038-what-personacore-requires-of-an-upstream.md
#[tokio::test]
async fn both_call_shapes_carry_the_requested_temperature_on_the_wire() {
    // **裏方（抽出・変容）が送る値そのもの。** 期待値は実装の定数から作らない。
    let request_at = |temperature: f32| ChatRequest {
        messages: vec![ChatMessage::user("こんにちは")],
        temperature: Some(temperature),
        max_tokens: None,
        reasoning_effort: None,
    };

    // 非ストリーミング（裏方の抽出・変容が通る形）。
    let plain = mock_server(
        ResponseTemplate::new(200)
            .set_body_raw(completion_body("はい", "stop"), "application/json"),
    )
    .await;
    client_for(&plain.uri())
        .complete(&request_at(0.8))
        .await
        .expect("正常な応答を解釈できること");

    // ストリーミング（本文生成の中継が通る形）。
    let streaming = sse_server(format!("{}data: [DONE]\n\n", sse_delta("こん"))).await;
    let mut chat_stream = client_for(&streaming.uri())
        .stream(&request_at(0.8))
        .await
        .expect("ストリームを開けること");
    drain(&mut chat_stream).await;

    for (label, server) in [("非ストリーミング", &plain), ("ストリーミング", &streaming)]
    {
        let requests = server
            .received_requests()
            .await
            .expect("記録が有効であること");
        assert_eq!(requests.len(), 1, "{label}: 上流へは1回だけ投げること");
        let raw = std::str::from_utf8(&requests[0].body).expect("送信本文が UTF-8 であること");

        assert!(
            has_json_field(raw, "temperature", "0.8"),
            "{label}: 温度が 0.8 の綴りで線に乗ること（to_value を経由すると \
             0.800000011920929 になる）: {raw}"
        );
        assert_eq!(
            raw.matches("\"temperature\"").count(),
            1,
            "{label}: temperature は1度だけ載ること（重複キーは読み直すと畳まれる）: {raw}"
        );
    }
}

/// JSON の本文に `"欄":値` が**その値ちょうどで**載っているか。
///
/// # ⚠ 素の `contains` を使わない理由
///
/// **`"top_k":40` は `"top_k":400` の一部**であり、
/// **`"repeat_penalty":1.1` は `"repeat_penalty":1.15` の一部**である。
/// 素の部分一致だと、**値を伸ばす変異が通り抜ける**
/// （CLAUDE.md「`sub_model=` の値が1文字でも伸びた綴りは別物」と同じ形）。
/// **値の直後が `,` か `}` であることまで見る。**
///
/// ここで扱うのは `serde_json` が書いた本文なので、
/// **欄名と値の間に空白は入らない**（整形されていたら、それはもう別の話である）。
fn has_json_field(body: &str, field: &str, value: &str) -> bool {
    let needle = format!("\"{field}\":{value}");
    body.match_indices(&needle).any(|(at, found)| {
        matches!(
            body[at + found.len()..].chars().next(),
            Some(',') | Some('}')
        )
    })
}

/// 中身の無いペイロードは `EmptyPayload`。壊れた JSON とは区別する。
#[tokio::test]
async fn empty_sse_payload_is_distinguishable() {
    // `data:` に空白だけを載せる（SSE は本当に空の data はイベント化しない）。
    let server = sse_server("data:  \n\ndata: [DONE]\n\n".to_owned()).await;

    let mut chat_stream = client_for(&server.uri())
        .stream(&request())
        .await
        .expect("ストリームを開けること");
    let items = drain(&mut chat_stream).await;

    assert_eq!(items.len(), 1, "エラー1件で終端すること: {items:?}");
    assert!(
        matches!(items[0], Err(LlmError::EmptyPayload)),
        "空のペイロードとして区別されること: {:?}",
        items[0]
    );
}

/// 壊れた JSON のチャンクはエラーになり、そこで終端する（panic しない）。
#[tokio::test]
async fn broken_chunk_json_terminates_the_stream() {
    let server = sse_server(format!("{}data: {{\"choices\":\n\n", sse_delta("こん"))).await;

    let mut chat_stream = client_for(&server.uri())
        .stream(&request())
        .await
        .expect("ストリームを開けること");
    let items = drain(&mut chat_stream).await;

    assert_eq!(items.len(), 2, "受け取れた分は流してから終端すること");
    assert_eq!(
        items[0].as_ref().expect("1件目は正常").delta,
        "こん",
        "壊れる前のチャンクは失わないこと"
    );
    assert!(
        matches!(items[1], Err(LlmError::Decode(_))),
        "JSON の破損として区別されること: {:?}",
        items[1]
    );
}

/// HTTP エラーは**ストリームの項目ではなく `stream()` 自体の失敗**として返る。
#[tokio::test]
async fn stream_reports_http_error_before_yielding() {
    let server = mock_server(ResponseTemplate::new(500).set_body_raw("boom", "text/plain")).await;

    let error = stream_error(
        client_for(&server.uri()).stream(&request()).await,
        "HTTP エラーはストリームを開く前に返ること",
    );

    assert!(
        matches!(error, LlmError::Status { status: 500 }),
        "HTTP エラーとして区別されること: {error:?}"
    );
}

/// 接続できないときも `stream()` の時点で分かる。
#[tokio::test]
async fn stream_reports_connect_failure_before_yielding() {
    let error = stream_error(
        client_for(&closed_address().await).stream(&request()).await,
        "接続できなければ失敗として返ること",
    );

    assert!(
        matches!(error, LlmError::Connect(_)),
        "接続失敗として区別されること: {error:?}"
    );
}

// ------------------------------------------------- ストリーミング（生 TCP・逐次性）

/// 上流の応答の終わり方。
#[derive(Debug, Clone, Copy)]
enum Ending {
    /// `[DONE]` を送り、チャンク終端まで書いて閉じる（正常終了）。
    Done,
    /// `[DONE]` を送らないまま接続を閉じる（上流が途中で落ちた）。
    CloseWithoutDone,
    /// チャンク枠の途中で接続を破棄する（フレーミングが壊れる）。
    AbortMidFrame,
}

/// 生の TCP で SSE を返す最小の HTTP サーバを立て、ベース URL と「続きを送る合図」を返す。
///
/// **`wiremock` では本文を分割して送れない**（`ResponseTemplate` は本文を一括で返す）ため、
/// 「上流がまだ送り終えていない時点で下流に届くか」を検証するにはこちらが要る。
///
/// `first` を書いた時点で**接続を開いたまま停止する**。合図を受けてから `rest` と終端を送る。
async fn spawn_sse_server(
    first: String,
    rest: Vec<String>,
    ending: Ending,
) -> (String, oneshot::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("一時ポートを確保できること");
    let addr = listener.local_addr().expect("アドレスを取得できること");
    let (release, wait) = oneshot::channel::<()>();

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("接続を受け付けられること");
        read_request(&mut socket).await;

        // `CloseWithoutDone` だけは長さも終端も示さない（本文は EOF で終わる）。
        // こうすると「上流が黙って落ちた」を再現できる。
        let head = match ending {
            Ending::CloseWithoutDone => {
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
            }
            _ => {
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                  Transfer-Encoding: chunked\r\n\r\n"
            }
        };
        socket
            .write_all(head.as_bytes())
            .await
            .expect("頭を書けること");
        write_part(&mut socket, &first, ending).await;

        // **ここで止まる。** 下流がこの時点で1件目を受け取れなければ、
        // 実装が全部溜め込んでから返している。
        let _ = wait.await;

        for part in rest {
            write_part(&mut socket, &part, ending).await;
        }
        match ending {
            Ending::Done => {
                write_part(&mut socket, "data: [DONE]\n\n", ending).await;
                socket
                    .write_all(b"0\r\n\r\n")
                    .await
                    .expect("終端を書けること");
            }
            Ending::CloseWithoutDone => {}
            // チャンク枠の途中で切る。長さだけ宣言して中身を送らない。
            Ending::AbortMidFrame => {
                socket
                    .write_all(b"40\r\ndata: {\"choi")
                    .await
                    .expect("途中まで書けること");
            }
        }
        let _ = socket.flush().await;
        drop(socket);
    });

    (format!("http://{addr}"), release)
}

/// 要求を読み切る（読まずに書くと接続が壊れる環境がある）。
async fn read_request(socket: &mut tokio::net::TcpStream) {
    let mut buffer = Vec::new();
    let mut scratch = [0_u8; 1024];
    loop {
        let read = socket.read(&mut scratch).await.expect("要求を読めること");
        if read == 0 || buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            return;
        }
        buffer.extend_from_slice(&scratch[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            return;
        }
    }
}

/// 本文の1片を書く。`CloseWithoutDone` 以外はチャンク転送符号化で包む。
async fn write_part(socket: &mut tokio::net::TcpStream, part: &str, ending: Ending) {
    let bytes = match ending {
        Ending::CloseWithoutDone => part.as_bytes().to_vec(),
        _ => format!("{:X}\r\n{part}\r\n", part.len()).into_bytes(),
    };
    socket.write_all(&bytes).await.expect("本文を書けること");
    socket.flush().await.expect("送出できること");
}

/// **逐次性の検証。** 上流が本文を送り終える前に、1件目が下流へ届くこと。
///
/// 全部受信してから返す実装だと、サーバが停止している間 `next()` が返らず
/// `NEXT_ITEM_LIMIT` で落ちる。**蓄積しながら中継できる**ことも同時に確かめる。
#[tokio::test]
async fn chunks_arrive_before_the_upstream_finishes() {
    let (base_url, release) = spawn_sse_server(
        sse_delta("こん"),
        vec![sse_delta("にち"), sse_delta("は")],
        Ending::Done,
    )
    .await;

    // **`stream()` 自体が本文の到着を待たないこと。** 上流は1件目を送った時点で
    // 止まっているので、ここで待つ実装（全部受信してから返す）はここで落ちる。
    // 時間で縛らないと、溜め込む実装は無通信上限（既定180秒）まで走ってしまう。
    let mut chat_stream =
        tokio::time::timeout(NEXT_ITEM_LIMIT, client_for(&base_url).stream(&request()))
            .await
            .expect("本文の到着を待たずにストリームが返ること")
            .expect("ストリームを開けること");

    // 上流はまだ2件目以降を送っていない。ここで返らなければ実装が溜め込んでいる。
    let first = tokio::time::timeout(NEXT_ITEM_LIMIT, chat_stream.next())
        .await
        .expect("上流が送り終える前に1件目が届くこと")
        .expect("1件目があること")
        .expect("1件目は正常なチャンクであること");
    assert_eq!(first.delta, "こん");

    // 蓄積は呼び出し側で行える（ストリームを止めない）。
    let mut accumulated = first.delta;
    release.send(()).expect("続きを送らせられること");
    for item in drain(&mut chat_stream).await {
        accumulated.push_str(&item.expect("残りも正常なチャンクであること").delta);
    }
    assert_eq!(accumulated, "こんにちは", "本文を落とさず蓄積できること");
}

/// **上流が `[DONE]` を送らずに切れた場合**、下流へ終端が伝わる（宙吊りにしない）。
#[tokio::test]
async fn upstream_eof_without_done_is_truncated() {
    let (base_url, release) =
        spawn_sse_server(sse_delta("こん"), vec![], Ending::CloseWithoutDone).await;
    release.send(()).expect("続きを送らせられること");

    let mut chat_stream = client_for(&base_url)
        .stream(&request())
        .await
        .expect("ストリームを開けること");
    let items = drain(&mut chat_stream).await;

    assert_eq!(items.len(), 2, "受け取れた分は流すこと: {items:?}");
    assert_eq!(items[0].as_ref().expect("1件目は正常").delta, "こん");
    assert!(
        matches!(items[1], Err(LlmError::Truncated)),
        "打ち切りとして区別されること: {:?}",
        items[1]
    );
}

/// フレーミングが壊れた場合も、エラーを1件出して終端する。
#[tokio::test]
async fn broken_framing_terminates_the_stream() {
    let (base_url, release) =
        spawn_sse_server(sse_delta("こん"), vec![], Ending::AbortMidFrame).await;
    release.send(()).expect("続きを送らせられること");

    let mut chat_stream = client_for(&base_url)
        .stream(&request())
        .await
        .expect("ストリームを開けること");
    let items = drain(&mut chat_stream).await;

    assert_eq!(items.len(), 2, "受け取れた分は流すこと: {items:?}");
    assert_eq!(items[0].as_ref().expect("1件目は正常").delta, "こん");
    let error = match &items[1] {
        Err(error) => error,
        Ok(chunk) => panic!("フレーミングの破損は失敗として終端すること: {chunk:?}"),
    };
    assert!(
        matches!(error, LlmError::Transport(_)),
        "上流の切断として区別されること: {error:?}"
    );
    assert_constant_message(error, TRANSPORT_MESSAGE);
}

// ------------------------------------------------------------------ 上限（llm_timeouts）
//
// 上限は3つあり、**効く場所がそれぞれ違う**。取り違えると「永久に返らない」か
// 「長い応答が途中で切れる」のどちらかになるので、3つとも実挙動で固定する。
//
// | 設定 | どこに効くか |
// |---|---|
// | `connect_timeout` | 接続確立まで |
// | `idle_timeout`（`read_timeout`） | **1回の読み取りが返るまで。** ヘッダ待ちも含む |
// | `request_timeout` | **非ストリーミングの全体。** ストリーミングには付けない |

/// 頭と1件だけ送り、**その後は何も送らずに接続を開いたままにする**サーバ。
///
/// LM Studio が生成の途中で固まった状態（接続は生きているが無通信）の再現。
/// `spawn_sse_server` は合図を受けたら必ず続きを送るので、この状態は作れない。
///
/// 戻り値のハンドルは検証後に `abort` する（黙って居座るタスクを残さない）。
async fn spawn_stalled_sse_server(first: String) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("一時ポートを確保できること");
    let addr = listener.local_addr().expect("アドレスを取得できること");

    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("接続を受け付けられること");
        read_request(&mut socket).await;

        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                  Transfer-Encoding: chunked\r\n\r\n",
            )
            .await
            .expect("頭を書けること");
        write_part(&mut socket, &first, Ending::Done).await;

        // **ここから永久に黙る。** ソケットは開いたまま保持する
        // （drop すると EOF になり、無通信ではなく `Truncated` の検証になってしまう）。
        std::future::pending::<()>().await;
    });

    (format!("http://{addr}"), handle)
}

/// **無通信の検出。** 上流が接続を開いたまま黙り込んだら `idle_timeout` で打ち切る。
///
/// これが効かないと `next()` は永久に返らず、**FE も axum のハンドラも宙吊りになる**
/// （docs/02-architecture.md「接続を宙吊りにしない」）。ストリーミングには全体上限を
/// 付けない方針なので、**この上限が唯一の防波堤**。
#[tokio::test]
async fn stalled_stream_is_cut_by_the_idle_timeout() {
    let (base_url, server) = spawn_stalled_sse_server(sse_delta("こん")).await;

    let config = LmStudioConfig::new(&base_url, model())
        .expect("ベース URL が正当であること")
        .with_idle_timeout(Duration::from_millis(300));
    let mut chat_stream = LmStudioClient::new(config)
        .expect("HTTP クライアントを構築できること")
        .stream(&request())
        .await
        .expect("ストリームを開けること");

    let items = drain(&mut chat_stream).await;

    assert_eq!(
        items.len(),
        2,
        "黙り込む前に受け取れた分は流すこと: {items:?}"
    );
    assert_eq!(
        items[0].as_ref().expect("1件目は正常").delta,
        "こん",
        "打ち切りの前に届いたチャンクは失わないこと"
    );
    let error = match &items[1] {
        Err(error) => error,
        Ok(chunk) => panic!("無通信は失敗として終端すること: {chunk:?}"),
    };
    assert!(
        matches!(error, LlmError::Timeout(_)),
        "無通信の打ち切りとして区別されること: {error:?}"
    );
    assert_constant_message(error, TIMEOUT_MESSAGE);

    server.abort();
}

/// **応答しないホストへの接続は `connect_timeout` で打ち切る。**
///
/// 宛先は RFC 5737 の TEST-NET-1（`192.0.2.1`）。文書用に予約されていて実在の
/// ホストには届かず、**SYN が黙って捨てられる**（拒否されるのとは別物で、
/// `closed_address` では再現できない）。この状態を OS 任せにすると
/// **Windows の既定では約 21 秒**待たされる（手動での実測値。この検証は
/// 上限を設定した側しか走らせないので、既定の21秒はここでは観測されない）。
///
/// 経路の都合で即座に「到達不能」が返る環境では、この検証は素通りする
/// （それでも誤って落ちることはない）。
#[tokio::test]
async fn connect_timeout_bounds_an_unresponsive_host() {
    let config = LmStudioConfig::new("http://192.0.2.1:1234", model())
        .expect("ベース URL が正当であること")
        .with_connect_timeout(Duration::from_millis(200));
    let client = LmStudioClient::new(config).expect("HTTP クライアントを構築できること");

    let started = Instant::now();
    let error = tokio::time::timeout(Duration::from_secs(5), client.complete(&request()))
        .await
        .expect("接続の上限で打ち切られること（OS 任せにしないこと）")
        .expect_err("接続できなければ失敗として返ること");
    let elapsed = started.elapsed();

    assert!(
        matches!(error, LlmError::Connect(_)),
        "接続失敗として区別されること: {error:?}"
    );
    assert_constant_message(&error, CONNECT_MESSAGE);
    assert!(
        elapsed < Duration::from_secs(3),
        "設定した上限で打ち切ること（OS 既定の約21秒を待たないこと）: {elapsed:?}"
    );
}

/// **`idle_timeout` は非ストリーミングにも効く。**
///
/// `read_timeout` は「1回の読み取り」の上限で、**応答ヘッダの到着待ちも含む**。
/// 生成中の LM Studio は何も送ってこないので、`complete()` の実効上限は
/// `min(request_timeout, idle_timeout)` になる。
/// ここでは全体上限（30秒）を十分に長く取り、**無通信の上限（300ms）が先に効く**
/// ことを固定する。既定値の役割分担（`DEFAULT_IDLE_TIMEOUT` が JIT ロード待ちを
/// 飲む側）はこの挙動が前提。
#[tokio::test]
async fn complete_is_bounded_by_the_idle_timeout_as_well() {
    let server = mock_server(
        ResponseTemplate::new(200)
            .set_body_raw(completion_body("遅い", "stop"), "application/json")
            .set_delay(Duration::from_secs(3)),
    )
    .await;

    let config = LmStudioConfig::new(&server.uri(), model())
        .expect("ベース URL が正当であること")
        .with_idle_timeout(Duration::from_millis(300))
        .with_request_timeout(Duration::from_secs(30));

    let started = Instant::now();
    let error = LmStudioClient::new(config)
        .expect("HTTP クライアントを構築できること")
        .complete(&request())
        .await
        .expect_err("無通信の上限で打ち切られること");
    let elapsed = started.elapsed();

    assert!(
        matches!(error, LlmError::Timeout(_)),
        "タイムアウトとして区別されること: {error:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "全体上限（30秒）ではなく無通信の上限（300ms）で打ち切ること: {elapsed:?}"
    );
}

/// **ストリーミングに全体上限を付けないこと。**
///
/// 付けると**長い応答が本文の途中でぶつ切りになる**。しかも切れた理由は
/// 上流にもモデルにも痕跡が残らないので、原因不明の欠落として現れる。
/// ここでは全体上限（300ms）を大きくまたぐ本文を流し、**最後まで届く**ことを固定する。
#[tokio::test]
async fn streaming_ignores_the_total_request_timeout() {
    let (base_url, release) = spawn_sse_server(
        sse_delta("こん"),
        vec![sse_delta("にち"), sse_delta("は")],
        Ending::Done,
    )
    .await;

    let config = LmStudioConfig::new(&base_url, model())
        .expect("ベース URL が正当であること")
        .with_request_timeout(Duration::from_millis(300));
    let mut chat_stream = LmStudioClient::new(config)
        .expect("HTTP クライアントを構築できること")
        .stream(&request())
        .await
        .expect("ストリームを開けること");

    let first = tokio::time::timeout(NEXT_ITEM_LIMIT, chat_stream.next())
        .await
        .expect("1件目が届くこと")
        .expect("1件目があること")
        .expect("1件目は正常なチャンクであること");
    assert_eq!(first.delta, "こん");

    // **全体上限を確実にまたぐ。** 実際の生成もこの長さを軽く超える。
    tokio::time::sleep(Duration::from_millis(600)).await;
    release.send(()).expect("続きを送らせられること");

    let mut accumulated = first.delta;
    for item in drain(&mut chat_stream).await {
        let chunk = item.expect("全体上限で打ち切られないこと（ストリーミングには付けない）");
        accumulated.push_str(&chunk.delta);
    }
    assert_eq!(
        accumulated, "こんにちは",
        "上限をまたいでも本文を落とさないこと"
    );
}

// -------------------------------------------------------- 切断時キャンセル（11c-3b）
//
// **FE が切断したら上流の推論も止める**（docs/02-architecture.md「無駄な推論を続けない」）。
// この主張は2段の連鎖でできている。
//
// | 段 | 主張 | 見張り番 |
// |---|---|---|
// | 下流 | FE が切ると axum が SSE の本文（`ChatStream` を持つ状態）を drop する | `tests/server_chat_cancel.rs` |
// | 上流 | **`ChatStream` を drop すると上流への TCP が閉じる** | **ここ** |
//
// **上の段は「そうなっているはず」で通してはいけない。** `reqwest`（hyper）は接続を
// 再利用するために本文を読み切ろうとすることがあり、そうなると **TCP は生き続けて
// 上流は生成を続ける。** 一般論ではなく**この構成で**測る。
//
// **`wiremock` では測れない**（接続が閉じたかを教えてくれない）ので、生の TCP を立てて
// 上流側から読み、**EOF が来るか**を見る。
//
// 2026-08-20 の実測（`reqwest` 0.13 / この構成）: drop から EOF まで **0.45〜0.60ms**（5回とも）。
// 保持した側は3秒観測して**一度も閉じなかった。**

/// 上流側から見た接続の様子。
///
/// 中身（原因・バイト数）は**失敗したときの手掛かり**としてしか読まない。
/// 判定に使うのはどのバリアントかだけなので、`Debug` 経由の読みしか無い。
#[derive(Debug)]
#[expect(dead_code, reason = "中身は失敗時の `Debug` 出力としてだけ読む")]
enum Liveness {
    /// 読みが 0 バイト（FIN）＝ **クライアントが閉じた。**
    Closed,
    /// 読みが失敗した（RST 等）。**これも「閉じた」側。**
    Reset(String),
    /// 観測窓の間なにも起きなかった ＝ **接続は生きている。**
    Open,
    /// 予期しない追加データ。要求は既に読み切っているので本来は来ない。
    Unexpected(usize),
}

/// 接続が閉じるのを待つ上限。**実測（0.6ms）に対して桁で余裕を取る。**
///
/// 閉じれば即座に返るので、**緑の側ではこの時間を待たない。**
const CLOSE_LIMIT: Duration = Duration::from_secs(5);

/// **閉じないことを確かめる観測窓。** こちらは必ず待ち切るので短く取る。
///
/// **短くしすぎると意味を失う。** 「この窓の間は閉じなかった」だけなら
/// **0ms でも緑になる**（実測 / レビュー指摘）。そこで
/// [`holding_the_stream_keeps_the_upstream_connection_open`] は
/// **同じ長さの窓で drop も観測する。** 窓が短すぎればその第2相が成立せず、
/// 対照そのものが赤くなる。
const OPEN_WINDOW: Duration = Duration::from_millis(500);

/// `spawn_liveness_watching_server` の観測結果を受け取る待ちの上限
/// （`tests/server_chat_cancel.rs` の `OBSERVE_LIMIT` / Issue origin#34 と同種）。
///
/// 送信側（`spawn_liveness_watching_server` 内のバックグラウンドタスク）は
/// `window`（呼び出し側が渡す [`CLOSE_LIMIT`] か [`OPEN_WINDOW`]、大きくても5秒）
/// 以内に必ず `report.send` を呼ぶ設計である。**本番コードの変異でここが終わらなく
/// なる経路は無い** —— 上流へ要求が届かなければ、テストはここに届く前に
/// `client.stream(...)` で止まる（本番のクライアントの `read_timeout` = 既定 180 秒で赤）。
/// ここに届いた時点で1件目を受け取っているので、送り手はもう `accept` を過ぎている。
/// **この上限は、ヘルパー自身が壊れたとき（送り手が送らずに止まる / B-57 / Issue origin#34 の変異 M2）の保険**
/// である（B-57 のレビュー / 2026-09-27。`tests/server_chat_cancel.rs` の上限とは役割が違う）。
///
/// 送信側の `window` の最大値（[`CLOSE_LIMIT`] = 5秒）より確実に大きく、かつ
/// `tests/server_chat_cancel.rs::OBSERVE_LIMIT` と値を揃える。
const OBSERVE_LIMIT: Duration = Duration::from_secs(30);

/// `spawn_liveness_watching_server` が返す `mpsc::Receiver` から次の観測結果を
/// 受け取る。**上限つき**（[`OBSERVE_LIMIT`]）。
async fn observe(receiver: &mut mpsc::Receiver<Liveness>) -> Liveness {
    tokio::time::timeout(OBSERVE_LIMIT, receiver.recv())
        .await
        .unwrap_or_else(|_| panic!("上流へ依頼が届かなかった（{OBSERVE_LIMIT:?} 待った）"))
        .expect("観測結果を受け取れること")
}

/// 頭と1件だけ送り、その後は**上流側から読み続けて**接続の生死を報告するサーバ。
///
/// `spawn_stalled_sse_server` との違いは**読むこと。** あちらは黙って居座るだけなので、
/// クライアント側が閉じたかを観測できない。
///
/// 観測結果を送るまでソケットを保持する（先に落とすと、こちら発の FIN と混ざる）。
///
/// **観測は同じ長さの窓で2回**行う。1回目は「保持している間は閉じない」の対照、
/// 2回目は「drop したら閉じる」。**対照側が窓を縮められないための仕掛け**
/// （[`OPEN_WINDOW`] の doc）なので、1回に減らさないこと。
async fn spawn_liveness_watching_server(
    first: String,
    window: Duration,
) -> (String, mpsc::Receiver<Liveness>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("一時ポートを確保できること");
    let addr = listener.local_addr().expect("アドレスを取得できること");
    let (report, observed) = mpsc::channel(2);

    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("接続を受け付けられること");
        read_request(&mut socket).await;

        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                  Transfer-Encoding: chunked\r\n\r\n",
            )
            .await
            .expect("頭を書けること");
        write_part(&mut socket, &first, Ending::Done).await;

        let mut scratch = [0_u8; 1024];
        for _ in 0..2 {
            let started = Instant::now();
            let result = tokio::time::timeout(window, socket.read(&mut scratch)).await;
            // **窓の外で起きたことは「観測できなかった」に倒す。**
            // `tokio::time::timeout` は**内側の future を先にポーリングする**ので、
            // 観測を始める前に届いていた FIN は**窓が 0 でも `Closed` として通る。**
            // それでは「窓が短すぎる」ことを第2相で検出できない
            // （実測: この実時間の締めが無いと `OPEN_WINDOW` を 0ms にしても緑のままだった）。
            let liveness = if started.elapsed() > window {
                Liveness::Open
            } else {
                match result {
                    Ok(Ok(0)) => Liveness::Closed,
                    Ok(Ok(read)) => Liveness::Unexpected(read),
                    Ok(Err(err)) => Liveness::Reset(err.to_string()),
                    Err(_elapsed) => Liveness::Open,
                }
            };
            // 受け手が1回しか読まないテストもある。**閉じていたら黙って終わる。**
            if report.send(liveness).await.is_err() {
                break;
            }
        }

        drop(socket);
    });

    (format!("http://{addr}"), observed)
}

/// **読み切らずに `ChatStream` を drop すると、上流への接続が閉じる。**
///
/// 中継の打ち切り（11c-3a の `state.upstream = None`）も FE の切断（11c-3b で
/// axum が本文を落とすこと）も、**最後は「`ChatStream` を落とす」という同じ1手**に
/// 集約されている。ここが効かなければ、**上流は誰も読まない応答を生成し続ける。**
#[tokio::test]
async fn dropping_the_stream_closes_the_upstream_connection() {
    let (base_url, mut observed) =
        spawn_liveness_watching_server(sse_delta("こん"), CLOSE_LIMIT).await;
    let client = client_for(&base_url);

    let mut chat_stream = client
        .stream(&request())
        .await
        .expect("ストリームを開けること");
    let first = tokio::time::timeout(NEXT_ITEM_LIMIT, chat_stream.next())
        .await
        .expect("1件目が届くこと")
        .expect("1件目があること")
        .expect("1件目は正常なチャンクであること");
    assert_eq!(first.delta, "こん", "前提: 途中まで読んだ状態にすること");

    // **ここが観測対象。** 上流はまだ `[DONE]` を送っていない。
    drop(chat_stream);

    let liveness = observe(&mut observed).await;

    // **クライアントは観測の後まで生かす。** 先に落とすと、閉じた原因が
    // 「ストリームの drop」なのか「クライアントごと消えたこと」なのか区別できない。
    drop(client);

    assert!(
        matches!(liveness, Liveness::Closed | Liveness::Reset(_)),
        "読み切らずに drop したら上流への接続を閉じること: {liveness:?}"
    );
}

/// **対照**: 保持している間は閉じない。
///
/// これが無いと、上の検証は**観測手段が常に「閉じた」と言うだけ**でも緑になる。
/// 同時に「読むのが遅いだけの下流を勝手に切らない」ことの確認でもある。
///
/// # 観測窓を第2相で縛る
///
/// 「[`OPEN_WINDOW`] の間は閉じなかった」だけなら、**窓を 0ms にしても緑のまま**
/// （実測 / レビュー指摘）。そこで**同じ長さの窓で drop も観測する。**
/// 窓が短すぎれば第2相が観測できず、**この対照そのものが赤くなる。**
#[tokio::test]
async fn holding_the_stream_keeps_the_upstream_connection_open() {
    let (base_url, mut observed) =
        spawn_liveness_watching_server(sse_delta("こん"), OPEN_WINDOW).await;
    let client = client_for(&base_url);

    let mut chat_stream = client
        .stream(&request())
        .await
        .expect("ストリームを開けること");
    let first = tokio::time::timeout(NEXT_ITEM_LIMIT, chat_stream.next())
        .await
        .expect("1件目が届くこと")
        .expect("1件目があること")
        .expect("1件目は正常なチャンクであること");
    assert_eq!(first.delta, "こん", "前提: 途中まで読んだ状態にすること");

    // 第1相: **落とさずに保持したまま**観測窓が尽きるのを待つ。
    let while_held = observe(&mut observed).await;

    // 第2相: **同じ長さの窓**で drop を観測する。ここが窓の長さを縛っている。
    drop(chat_stream);
    let after_drop = observe(&mut observed).await;

    // **クライアントは観測の後まで生かす**（上の検証と同じ理由）。
    drop(client);

    assert!(
        matches!(while_held, Liveness::Open),
        "保持している間は上流への接続を閉じないこと: {while_held:?}"
    );
    assert!(
        matches!(after_drop, Liveness::Closed | Liveness::Reset(_)),
        "第2相で drop を観測できなかった。**観測窓（{OPEN_WINDOW:?}）が短すぎる**か、\
         **drop が上流へ伝わっていない**かのどちらか。\
         いずれにせよ第1相の `Open` は「閉じない」ではなく「何も見ていない」を意味する: \
         {after_drop:?}"
    );
}

// ------------------------------------------------ 実コンテキスト長の照会（ADR-0013 第2層）
//
// **`/api/v0/models` は LM Studio 独自 API。** OpenAI 互換の `/v1/models` は
// コンテキスト長を返さない（実測: `id` と `owned_by` のみ）ので、
// `LlmClient` トレイトには載せず `LmStudioClient` の固有メソッドにしてある。
//
// ここで確かめるのは HTTP の経路だけ（本文の解釈は `src/llm/response.rs` の単体テスト）。
// **どの失敗でも起動を止めないこと**が要点なので、失敗が区別できる形で返ることを見る。

/// 実機で観測した `/api/v0/models` の応答（2026-08-14）。
///
/// **`max_context_length` と `loaded_context_length` の値は必ず違えておくこと。**
const NATIVE_MODELS_BODY: &str = r#"{
    "object": "list",
    "data": [
        {"id": "ministral-3-14b-instruct-2512", "object": "model", "type": "llm",
         "state": "loaded", "max_context_length": 262144, "loaded_context_length": 16384}
    ]
}"#;

/// **既定ではない**モデル。`PERSONACORE_MAIN_MODEL` で実際に設定しうる値
/// （docs/archive/04-environment-lm-studio.md の実測値）。
const OTHER_MODEL: &str = "ministral-3-3b-instruct-2512";

/// `MODEL` に対応する、一覧上のロード済みコンテキスト長。
const MODEL_LOADED: u32 = 16_384;

/// `OTHER_MODEL` に対応する長さ。**`MODEL_LOADED` と必ず違えておくこと**
/// （同じ値だと、どちらの行を掴んだかが区別できない）。
const OTHER_MODEL_LOADED: u32 = 4_096;

/// 2つのモデルが**別々の長さで**同時にロードされている一覧。
///
/// 実機でも起こる状態（14B を 16384、3B を 4096 でロード）。
fn two_loaded_models_body() -> String {
    format!(
        r#"{{"object":"list","data":[
            {{"id":"{MODEL}","object":"model","type":"llm","state":"loaded",
              "max_context_length":262144,"loaded_context_length":{MODEL_LOADED}}},
            {{"id":"{OTHER_MODEL}","object":"model","type":"llm","state":"loaded",
              "max_context_length":262144,"loaded_context_length":{OTHER_MODEL_LOADED}}}]}}"#
    )
}

/// `GET /api/v0/models` に固定応答を返すモックサーバ。
async fn native_models_server(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v0/models"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

/// **ロード済みのコンテキスト長を独自 API から取れる。**
#[tokio::test]
async fn the_loaded_context_length_comes_from_the_native_models_endpoint() {
    let server = native_models_server(
        ResponseTemplate::new(200).set_body_raw(NATIVE_MODELS_BODY, "application/json"),
    )
    .await;

    let tokens = client_for(&server.uri())
        .loaded_context_length()
        .await
        .expect("実機と同じ応答なら取れること");

    assert_eq!(
        tokens, 16_384,
        "max_context_length（262144）ではなく loaded_context_length を読むこと"
    );
}

/// **照会するのは「設定中の」モデル。既定のモデルではない。**
///
/// `PERSONACORE_MAIN_MODEL=ministral-3-3b-instruct-2512`（4096 でロード）で起動し、
/// 一覧に 14B が 16384 でロード済みで並んでいると、**4096 のモデルに 16384 の予算**を
/// 張ることになる。LM Studio はエラーも出さずに前方から切り捨てるので気付けない（ADR-0013）。
/// llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。ただし、
/// プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
///
/// `src/llm/response.rs` の `the_model_id_is_matched_exactly` が押さえているのは
/// **パーサ側の照合規則**だけで、他の結合テストはすべて既定モデルを使うため、
/// クライアントが渡す id が **`DEFAULT_MAIN_MODEL` のハードコードでも区別が付かない。**
/// ここだけが「**設定中の**モデルを渡すこと」を見ている。
#[tokio::test]
async fn the_context_length_is_queried_for_the_configured_model() {
    assert_ne!(OTHER_MODEL, MODEL, "既定と別のモデルで確かめること");

    let server = native_models_server(
        ResponseTemplate::new(200).set_body_raw(two_loaded_models_body(), "application/json"),
    )
    .await;

    let tokens = client_for_model(&server.uri(), OTHER_MODEL)
        .loaded_context_length()
        .await
        .expect("設定中のモデルは一覧にあること");
    assert_eq!(
        tokens, OTHER_MODEL_LOADED,
        "設定中のモデル（{OTHER_MODEL}）の行を読むこと（既定モデルの {MODEL_LOADED} ではない）"
    );

    // 逆向き: 既定モデルを設定したなら既定モデルの行を掴む
    //（別の id への固定にすり替わっていないこと）。
    let tokens = client_for(&server.uri())
        .loaded_context_length()
        .await
        .expect("設定中のモデルは一覧にあること");
    assert_eq!(
        tokens, MODEL_LOADED,
        "設定中のモデル（{MODEL}）の行を読むこと"
    );
}

/// **独自 API を持たない上流でも壊れない。** 404 はステータスとして区別できる。
///
/// vLLM / Ollama などに差し替えるとこの経路に来る（ADR-0013）。
#[tokio::test]
async fn an_upstream_without_the_native_api_reports_a_status() {
    // **何も mount しないサーバ**は未知のパスに 404 を返す（独自 API が無い上流の再現）。
    let server = MockServer::start().await;

    let error = client_for(&server.uri())
        .loaded_context_length()
        .await
        .expect_err("独自 API が無ければ失敗すること");

    assert_eq!(
        error.status(),
        Some(404),
        "ステータスとして区別されること: {error:?}"
    );
    assert_constant_message(&error, "推論サーバがエラーを返しました");
}

/// **停止している上流でも壊れない。** 接続失敗として区別できる。
#[tokio::test]
async fn a_stopped_upstream_cannot_report_its_context_length() {
    let base_url = closed_address().await;

    let error = client_for(&base_url)
        .loaded_context_length()
        .await
        .expect_err("接続できなければ失敗すること");

    assert!(
        matches!(error, LlmError::Connect(_)),
        "接続失敗として区別されること: {error:?}"
    );
    assert_constant_message(&error, CONNECT_MESSAGE);
}

/// **照会は起動時に走るので、上限で必ず打ち切る。**
///
/// ここに上限が無いと、黙り込んだ上流1台で**サーバが起動しなくなる**
/// （待ち受けもデータベース接続も、この照会の後ろにある）。
#[tokio::test]
async fn a_slow_native_api_is_bounded_by_its_own_timeout() {
    let server = native_models_server(
        ResponseTemplate::new(200)
            .set_body_raw(NATIVE_MODELS_BODY, "application/json")
            .set_delay(Duration::from_secs(30)),
    )
    .await;

    let config = LmStudioConfig::new(&server.uri(), model())
        .expect("ベース URL が正当であること")
        .with_context_timeout(Duration::from_millis(200));

    let started = Instant::now();
    let error = LmStudioClient::new(config)
        .expect("HTTP クライアントを構築できること")
        .loaded_context_length()
        .await
        .expect_err("時間内に返らなければ失敗として返ること");

    assert!(
        matches!(error, LlmError::Timeout(_)),
        "タイムアウトとして区別されること: {error:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "上限で打ち切ること（本文生成側の 300 秒を引き継がないこと）: {:?}",
        started.elapsed()
    );
    assert_constant_message(&error, TIMEOUT_MESSAGE);
}

// ------------------------------------- 2つ目の読み手（作業単位 G-2 / `/v1/models`）
//
// **`llama-server` には `/api/v0/models` が無い**（404 / 2026-09-04 実測）。
// そのままだと `-c` に何を渡しても personacore は落とし所を使い続ける。
// **答えられないのではなく、綴りが違うだけ**なので、OpenAI 互換の `/v1/models` を
// 2つ目の読み手として持つ。
//
// **往復は増やさない方向に倒してある**——1つ目が答えたら2つ目は投げない。
// ここで見るのはその条件そのもの（「投げる」「投げない」の両側）。

/// `llama-server` の `/v1/models` が申告する窓。
///
/// **落とし所（`SubContext::FALLBACK` の 16384）とも、既定の予算（8192）とも、
/// このファイルの `MODEL_LOADED`（16384）とも違う数**にすること。
/// 揃っていると、**引いてきたのか決め打ちなのかが原理的に区別できない**
/// （CLAUDE.md「実行時に決まる値を入れたら、既定と違う状態で回る腕を必ず1本置く」）。
const COMPAT_N_CTX: u32 = 20_480;

/// 同じ応答に並ぶ**訓練時の上限**。**`COMPAT_N_CTX` と必ず違えておくこと。**
const COMPAT_N_CTX_TRAIN: u32 = 262_144;

/// `llama-server` の `id`（**gguf の絶対パス** / 設定中の綴りとは一致しない）。
const COMPAT_MODEL_ID: &str = r"C:\models\Ministral-3-14B-Instruct-2512-Q4_K_M.gguf";

/// 実機の `llama-server` と同じ形の `/v1/models` の応答。
fn compat_models_body() -> String {
    format!(
        r#"{{"object":"list","data":[
            {{"id":"{}","object":"model","owned_by":"llamacpp",
              "meta":{{"n_vocab":131072,"n_ctx":{COMPAT_N_CTX},
                       "n_ctx_train":{COMPAT_N_CTX_TRAIN},"n_embd":5120}}}}]}}"#,
        COMPAT_MODEL_ID.replace('\\', r"\\")
    )
}

/// `GET /v1/models` に固定応答を足す。
async fn mount_compat_models(server: &MockServer, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(response)
        .mount(server)
        .await;
}

/// そのパスを叩かれた回数。
async fn hits(server: &MockServer, wanted: &str) -> usize {
    server.received_requests().await.map_or(0, |requests| {
        requests
            .iter()
            .filter(|request| request.url.path() == wanted)
            .count()
    })
}

/// **独自 API を持たない上流でも、OpenAI 互換の一覧から実効の窓が取れる**
/// （作業単位 G-2 / `llama-server`）。
///
/// **これがこの作業単位の存在理由。** 2つ目の読み手を消すと、
/// `-c 20480` で立てた上流に対して personacore は既定の 8192 を使い続ける
/// ——**エラーも出ず応答も返る**ので、ログの文面以外に手掛かりが無い。
#[tokio::test]
async fn the_effective_context_length_falls_back_to_the_openai_models_endpoint() {
    // **1つ目は 404。** 実機の `llama-server` と同じ（`/api/v0/models` を持たない）。
    let server = native_models_server(ResponseTemplate::new(404)).await;
    mount_compat_models(
        &server,
        ResponseTemplate::new(200).set_body_raw(compat_models_body(), "application/json"),
    )
    .await;

    let tokens = client_for(&server.uri())
        .loaded_context_length()
        .await
        .expect("OpenAI 互換の一覧から取れること");

    assert_eq!(
        tokens, COMPAT_N_CTX,
        "n_ctx_train（{COMPAT_N_CTX_TRAIN}）ではなく n_ctx を読むこと"
    );
    assert_eq!(
        hits(&server, "/v1/models").await,
        1,
        "2つ目の読み手は1回だけ投げること"
    );
}

/// **1つ目が答えたら、2つ目は投げない**（作業単位 G-2 / 回帰）。
///
/// LM Studio 経由の往復は**今までどおり1回。**
/// 「起動時の照会はモデルごとに1回」を見張っているテスト
/// （`tests/server_context_refresh.rs`）の主張を変えないための条件。
///
/// **2つの一覧はわざと違う値を申告する。** 同じ値だと、どちらを読んだのか
/// 区別が付かない（この家の最頻の穴）。
#[tokio::test]
async fn the_openai_endpoint_is_not_queried_when_the_native_one_answers() {
    assert_ne!(
        MODEL_LOADED, COMPAT_N_CTX,
        "前提: 2つの一覧は違う値を申告すること"
    );

    let server = native_models_server(
        ResponseTemplate::new(200).set_body_raw(NATIVE_MODELS_BODY, "application/json"),
    )
    .await;
    mount_compat_models(
        &server,
        ResponseTemplate::new(200).set_body_raw(compat_models_body(), "application/json"),
    )
    .await;

    let tokens = client_for(&server.uri())
        .loaded_context_length()
        .await
        .expect("独自 API が答えれば取れること");

    assert_eq!(tokens, MODEL_LOADED, "独自 API の側の値を使うこと");
    assert_eq!(
        hits(&server, "/api/v0/models").await,
        1,
        "1つ目は1回だけ投げること"
    );
    assert_eq!(
        hits(&server, "/v1/models").await,
        0,
        "1つ目が答えたら2つ目は投げないこと（往復を増やさない）"
    );
}

/// **綴りが一致する行があるなら、そちらを読む**（作業単位 G-2 / HTTP 越し）。
///
/// # これは架空の上流ではない
///
/// `llama-server` の `-a` / `--alias` は `/v1/models` の `id` を差し替える。
/// **別名を設定中のモデルに合わせて立てた上流**がここに来る。
///
/// # 2件のうち**真ん中でも端でもない**方を当てる
///
/// 一致する行を**後ろ**に置いてある。「常に最初の1件」も「一覧が1件のときの規則を
/// 先に効かせる」も、ここで別の値を返して赤になる
/// （CLAUDE.md「選ぶ操作を見るテストは、選ばれる対象を端に置かない」の趣旨——
/// **2件しか無いので、せめて一致の側を先頭から外す**）。
#[tokio::test]
async fn an_id_that_matches_our_model_wins_over_the_other_entries() {
    /// 一致しない側の窓。**一致する側とは必ず違う値。**
    const DECOY_N_CTX: u32 = 3_072;

    assert_ne!(
        DECOY_N_CTX, COMPAT_N_CTX,
        "前提: 2つの行は違う窓を申告すること"
    );

    let body = format!(
        r#"{{"object":"list","data":[
            {{"id":"{COMPAT_MODEL_ID_ESCAPED}","object":"model",
              "meta":{{"n_ctx":{DECOY_N_CTX},"n_ctx_train":{COMPAT_N_CTX_TRAIN}}}}},
            {{"id":"{MODEL}","object":"model",
              "meta":{{"n_ctx":{COMPAT_N_CTX},"n_ctx_train":{COMPAT_N_CTX_TRAIN}}}}}]}}"#,
        COMPAT_MODEL_ID_ESCAPED = COMPAT_MODEL_ID.replace('\\', r"\\")
    );

    let server = native_models_server(ResponseTemplate::new(404)).await;
    mount_compat_models(
        &server,
        ResponseTemplate::new(200).set_body_raw(body, "application/json"),
    )
    .await;

    let tokens = client_for(&server.uri())
        .loaded_context_length()
        .await
        .expect("綴りが一致する行があること");

    assert_eq!(
        tokens, COMPAT_N_CTX,
        "設定中のモデルの行を読むこと（先頭の {DECOY_N_CTX} ではない）"
    );
}

/// **どれとも一致せず、一覧が2件以上あるなら読まない**（作業単位 G-2 / HTTP 越し）。
///
/// **「1件しか無ければそれを採る」の危険を抑えているのがここ。**
/// 間違ったモデルの窓を本文の予算にすると壊れる。この経路の上流（llama / `meta`）は、窓を大きく
/// 取り違えれば 400 `exceed_context_size_error` で断るか、プロンプト ＋ 応答の上限が窓を超えて
/// 応答が黙って短くなる（`finish_reason=length` / ADR-0052 M5）。小さく取り違えれば余るだけ。
/// どちらでも前方からの切り捨ては起きない（「エラーも出ず応答も返る」前方切り捨ては
/// LM Studio の話 / ADR-0013）。
/// **読まなければ既定値へ落ちるだけ**で、遅くなるだけ・壊れない側に倒れる。
///
/// **単体テスト（`a_list_with_several_models_and_no_match_is_refused`）と対にしてある。**
/// あちらが規則そのもの、こちらが**その規則が本当に HTTP の経路に載っているか。**
#[tokio::test]
async fn an_ambiguous_openai_list_yields_no_context_length() {
    /// 一覧に並ぶ2つの窓。**どちらも掴んではいけない。**
    const FIRST_N_CTX: u32 = 3_072;
    const SECOND_N_CTX: u32 = 5_120;

    let body = format!(
        r#"{{"object":"list","data":[
            {{"id":"someone-else","object":"model","meta":{{"n_ctx":{FIRST_N_CTX}}}}},
            {{"id":"another-one","object":"model","meta":{{"n_ctx":{SECOND_N_CTX}}}}}]}}"#
    );

    let server = native_models_server(ResponseTemplate::new(404)).await;
    mount_compat_models(
        &server,
        ResponseTemplate::new(200).set_body_raw(body, "application/json"),
    )
    .await;

    let report = client_for(&server.uri())
        .models()
        .await
        .expect("一覧そのものは取れること");
    assert_eq!(
        report.catalogue(),
        ["someone-else".to_owned(), "another-one".to_owned()],
        "綴りの一覧は読めること（読めないから失敗した、ではない）"
    );

    let error = report
        .context_length()
        .expect_err("どれが答えるのか分からないなら読まないこと");
    assert_eq!(
        error.status(),
        Some(404),
        "1つ目の失敗の理由を保つこと: {error:?}"
    );
}

/// **1つ目が 200 を返したなら、長さが読めなくても2つ目は投げない**（作業単位 G-2）。
///
/// # なぜこの腕が要るのか（**変異検査で分かった**）
///
/// 着手時は「長さが読めなければ2つ目」にしていたが、**その枝を消す変異が 1931 全緑**だった。
/// **得をする上流が無い**——LM Studio の `/v1/models` は `meta` を持たないので、
/// 「一覧に居ない・未ロード」で投げ直しても必ず空振りする。
///
/// **そして LM Studio は JIT ロードなので「未ロード」は普通の状態。**
/// 投げると**冷えた起動のたびに、モデルごとに往復が1つ増える。**
/// ここはその「増えない」を見張る側。
#[tokio::test]
async fn the_openai_endpoint_is_not_queried_when_the_native_list_lacks_our_model() {
    // **1つ目は 200。** ただし設定中のモデルは一覧に居ない（冷えた起動 / 別のモデル設定）。
    let server = native_models_server(
        ResponseTemplate::new(200).set_body_raw(NATIVE_MODELS_BODY, "application/json"),
    )
    .await;
    // **2つ目は答えられる状態**にしておく（投げないことが主張なので）。
    mount_compat_models(
        &server,
        ResponseTemplate::new(200).set_body_raw(compat_models_body(), "application/json"),
    )
    .await;

    let error = client_for_model(&server.uri(), OTHER_MODEL)
        .loaded_context_length()
        .await
        .expect_err("一覧に居ないモデルの長さは取れないこと");

    assert!(
        matches!(error, LlmError::MissingContextLength),
        "1つ目の失敗の理由がそのまま残ること: {error:?}"
    );
    assert_eq!(
        hits(&server, "/v1/models").await,
        0,
        "1つ目が 200 を返したなら、2つ目は投げないこと（空振りの往復を増やさない）"
    );
}

/// **黙り込んだ上流に対して、起動時の待ちを倍にしないこと**（作業単位 G-2）。
///
/// 2つ目の読み手を無条件に投げると、**上限（`context_timeout`）を2回ぶん待つ。**
/// 待たされるのは起動そのもの（待ち受けも DB 接続もこの照会の後ろにある）。
///
/// **投げるのは「上流は応答したが、この綴りでは答えられなかった」ときだけ。**
/// 応答が1つも返らなかった（タイムアウト / 接続失敗）なら、2つ目も返らない。
#[tokio::test]
async fn a_silent_native_api_does_not_double_the_startup_stall() {
    let server = native_models_server(
        ResponseTemplate::new(200)
            .set_body_raw(NATIVE_MODELS_BODY, "application/json")
            .set_delay(Duration::from_secs(30)),
    )
    .await;
    // **2つ目は即座に答えられる状態**にしておく（投げないことが主張なので）。
    mount_compat_models(
        &server,
        ResponseTemplate::new(200).set_body_raw(compat_models_body(), "application/json"),
    )
    .await;

    let config = LmStudioConfig::new(&server.uri(), model())
        .expect("ベース URL が正当であること")
        .with_context_timeout(Duration::from_millis(200));

    let started = Instant::now();
    let error = LmStudioClient::new(config)
        .expect("HTTP クライアントを構築できること")
        .loaded_context_length()
        .await
        .expect_err("時間内に返らなければ失敗として返ること");

    assert!(
        matches!(error, LlmError::Timeout(_)),
        "タイムアウトとして区別されること: {error:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "上限で打ち切ること: {:?}",
        started.elapsed()
    );
    assert_eq!(
        hits(&server, "/v1/models").await,
        0,
        "応答が1つも返らなかったなら、2つ目は投げないこと（待ちを倍にしない）"
    );
}

/// **一覧は取れて、長さだけが取れない上流**（作業単位 G-2）。
///
/// 実機の LM Studio の `/v1/models` がこの形（**200 だが `meta` を持たない**）。
/// **「200 が返った」は「答えた」ではない**ので、長さは取れないまま。
/// **それでも綴りの一覧は使える**——`measured::check` はそれで足りる。
#[tokio::test]
async fn the_catalogue_comes_from_the_openai_endpoint_when_the_native_one_is_absent() {
    let server = native_models_server(ResponseTemplate::new(404)).await;
    mount_compat_models(
        &server,
        // **実機の LM Studio と同じ形**（`meta` を持たない一覧）。
        ResponseTemplate::new(200).set_body_raw(
            format!(r#"{{"object":"list","data":[{{"id":"{MODEL}","object":"model"}}]}}"#),
            "application/json",
        ),
    )
    .await;

    let report = client_for(&server.uri())
        .models()
        .await
        .expect("一覧が取れたなら往復は成功していること");
    assert_eq!(
        report.catalogue(),
        [MODEL.to_owned()],
        "綴りの一覧は OpenAI 互換の側から取れること"
    );

    let error = report
        .context_length()
        .expect_err("`meta` が無ければ長さは取れないこと");
    assert_eq!(
        error.status(),
        Some(404),
        "1つ目の失敗の理由を保つこと（2つ目の 200 で塗り潰さない）: {error:?}"
    );
}

/// **どちらの綴りも持たない上流でも壊れない**（作業単位 G-2 の後も同じ）。
///
/// 返るのは**1つ目の失敗**。`an_upstream_without_the_native_api_reports_a_status` と
/// 同じ主張だが、あちらは**2つ目を明示的に置いていない**ので、
/// **ここが「2つとも 404」を名指しで作っている状態。**
#[tokio::test]
async fn an_upstream_with_neither_list_reports_the_native_status() {
    let server = native_models_server(ResponseTemplate::new(404)).await;
    mount_compat_models(&server, ResponseTemplate::new(404)).await;

    let error = client_for(&server.uri())
        .loaded_context_length()
        .await
        .expect_err("どちらも無ければ失敗すること");

    assert_eq!(error.status(), Some(404), "{error:?}");
    assert_eq!(
        hits(&server, "/v1/models").await,
        1,
        "1つ目が 404 なら2つ目は投げること"
    );
}

/// **2つ目の読み手が長さを読めたときも、綴りの一覧を持ち帰る**（作業単位 G-2 の是正）。
///
/// # ここが塞いでいる穴（**レビューが実測 / その変異は 1934 全緑だった**）
///
/// 成功の枝の `catalogue` を `Vec::new()` にする変異が、**1本も赤にならなかった。**
/// このファイルは `catalogue` を**失敗の枝でしか見ていなかった**のに、
/// **`llama-server` が必ず通るのは成功の枝**（1つ目が 404 で、2つ目が長さを答える）。
///
/// # 落とすと何が起きるか
///
/// **`catalogue` は `measured::report` の唯一の入力**で、
/// `measured::check` は**空の一覧を `Unanswered` に倒す。**
/// つまり **`Drift::Absent` の warn が丸ごと消え**、まとめの行が
/// `measured_drifted=N` から `measured_unanswered=N` へ**静かに移る。**
/// **ADR-0036 §決定4 の引き金がその報告**なので、
/// 消えたことに気付ける場所が他に無い。
#[tokio::test]
async fn the_catalogue_survives_when_the_openai_endpoint_answers_with_a_length() {
    let server = native_models_server(ResponseTemplate::new(404)).await;
    mount_compat_models(
        &server,
        ResponseTemplate::new(200).set_body_raw(compat_models_body(), "application/json"),
    )
    .await;

    let report = client_for(&server.uri())
        .models()
        .await
        .expect("2つ目の読み手が答えること");

    assert_eq!(
        report.catalogue(),
        [COMPAT_MODEL_ID.to_owned()],
        "長さを読めた枝でも、綴りの一覧を持ち帰ること（`measured::check` の唯一の入力）"
    );
    assert_eq!(
        report
            .context_length()
            .expect("同じ応答から長さも読めること"),
        COMPAT_N_CTX,
        "一覧だけが取れて長さが消える形ではないこと"
    );
}

/// **2つ目の読み手にも、1つ目と同じ短い上限が掛かること**（作業単位 G-2 の是正）。
///
/// # 失敗の筋道（**レビューが実測 / その変異は 1934 全緑だった**）
///
/// 上限をパスで分岐させる変異——**1つ目にだけ `context_timeout` を掛け、
/// 2つ目には掛けない**——を、誰も赤にしなかった。
/// `LmStudioClient::new` は `connect_timeout` と `idle_timeout`（180 秒）しか持たず、
/// **往復全体の上限が無い。** つまり
/// **`/api/v0/models` を 404 で即返し、`/v1/models` でだらだら返す上流**に当たると、
/// **起動がそこで止まる**——待ち受けも DB 接続もこの照会の後ろにある。
///
/// # 既存の腕では観測できない
///
/// `a_silent_native_api_does_not_double_the_startup_stall` は**1つ目を遅くする**腕で、
/// **そのテスト自身が「2つ目は投げていない」と主張している。**
/// **「1つ目が 404 ＋ 2つ目が遅い」腕はここにしか無い。**
#[tokio::test]
async fn a_slow_openai_models_endpoint_is_bounded_by_the_same_timeout() {
    let server = native_models_server(ResponseTemplate::new(404)).await;
    mount_compat_models(
        &server,
        ResponseTemplate::new(200)
            .set_body_raw(compat_models_body(), "application/json")
            .set_delay(Duration::from_secs(30)),
    )
    .await;

    let config = LmStudioConfig::new(&server.uri(), model())
        .expect("ベース URL が正当であること")
        .with_context_timeout(Duration::from_millis(200));

    let started = Instant::now();
    let error = LmStudioClient::new(config)
        .expect("HTTP クライアントを構築できること")
        .loaded_context_length()
        .await
        .expect_err("時間内に返らなければ失敗として返ること");

    assert!(
        started.elapsed() < Duration::from_secs(5),
        "2つ目の読み手も上限で打ち切ること: {:?}",
        started.elapsed()
    );
    assert_eq!(
        error.status(),
        Some(404),
        "失敗の理由は1つ目のものを保つこと（2つ目の遅さで塗り潰さない）: {error:?}"
    );
    assert_eq!(
        hits(&server, "/v1/models").await,
        1,
        "2つ目は実際に投げていること（投げないから速い、ではない）"
    );
}

/// **2つ目も綴りを1件も返さなかったら、往復ごと失敗として返る**（作業単位 G-2 の是正）。
///
/// # なぜ要るのか（**等価変異ではなかった**）
///
/// `src/llm/lmstudio.rs` の「一覧が空なら1つ目の失敗を返す」枝は、
/// **`llm` の外の呼び出し側からは観測できない**（`if false` にしても全緑）。
/// **`models()` からは観測できる**——`if false` だと、
/// **空の一覧を積んだ `Ok` が返る。**
///
/// **「観測できない」は「その観測手段では観測できない」でしかない**（CLAUDE.md）。
#[tokio::test]
async fn an_empty_openai_list_fails_the_whole_round_trip() {
    let server = native_models_server(ResponseTemplate::new(404)).await;
    mount_compat_models(
        &server,
        // **200 で、JSON としても読める。それでも綴りは1件も無い。**
        ResponseTemplate::new(200)
            .set_body_raw(r#"{"object":"list","data":[]}"#, "application/json"),
    )
    .await;

    let error = client_for(&server.uri())
        .models()
        .await
        .expect_err("2つ目からも何も得られなければ往復ごと失敗として返ること");

    assert_eq!(
        error.status(),
        Some(404),
        "失敗の理由は1つ目のものを保つこと: {error:?}"
    );
}

// ---------------------------------------------------------------- トレイト注入

/// トレイトレベルのモック。**HTTP を一切使わない実装を注入できることの証明。**
#[derive(Debug)]
struct MockLlm {
    reply: String,
    deltas: Vec<String>,
}

impl LlmClient for MockLlm {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        // 受け取ったリクエストを見て応答を変えられること（モックとして最低限の要件）。
        let echoed = request
            .messages
            .last()
            .map(|message| message.content.clone())
            .unwrap_or_default();
        let reply = format!("{}/{echoed}", self.reply);
        Box::pin(async move {
            Ok(Completion {
                content: reply,
                finish_reason: Some(FinishReason::Stop),
                // トレイト注入の実証が目的なので、答え合わせの材料は持たない
                // （**`usage` を返さない上流**の再現でもある / ADR-0013 第3層）。
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        let items: Vec<Result<ChatChunk, LlmError>> = self
            .deltas
            .iter()
            .map(|delta| {
                Ok(ChatChunk {
                    delta: delta.clone(),
                    finish_reason: None,
                    // 上と同じく**`usage` を返さない上流**の再現（ADR-0013 第3層）。
                    usage: None,
                })
            })
            .collect();
        Box::pin(async move {
            let stream: ChatStream = Box::pin(stream::iter(items));
            Ok(stream)
        })
    }
}

/// 上位層が具象型ではなくトレイトに依存できること（`dyn` で使えること）。
async fn relay(client: &dyn LlmClient) -> String {
    let mut chat_stream = client.stream(&request()).await.expect("モックは失敗しない");
    let mut accumulated = String::new();
    while let Some(item) = chat_stream.next().await {
        accumulated.push_str(&item.expect("モックは失敗しない").delta);
    }
    accumulated
}

#[tokio::test]
async fn mock_client_can_be_injected_through_the_trait() {
    let mock = MockLlm {
        reply: "もっく".to_owned(),
        deltas: vec!["こん".to_owned(), "にちは".to_owned()],
    };

    let completion = mock
        .complete(&request())
        .await
        .expect("モックは失敗しないこと");
    assert_eq!(completion.content, "もっく/こんにちは");

    // `Arc<dyn LlmClient>` として持ち回せること（server 層が使う形）。
    let shared: Arc<dyn LlmClient> = Arc::new(mock);
    assert_eq!(relay(shared.as_ref()).await, "こんにちは");
}

/// 役割の綴りは OpenAI 互換の3種類。取り違えるとペルソナが崩れる。
#[test]
fn chat_message_helpers_set_the_role() {
    assert_eq!(ChatMessage::system("x").role, ChatRole::System);
    assert_eq!(ChatMessage::user("x").role, ChatRole::User);
    assert_eq!(ChatMessage::assistant("x").role, ChatRole::Assistant);
}
