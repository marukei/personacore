//! **鍵を平文の `http://` で別の機へ送るとき**（[ADR-0056](../docs/adr/0056-a-key-for-the-upstream.md)
//! §決定5 / 作業単位 A-13b-2 / Issue origin#85）を、`llm` の線の上で確かめる。
//!
//! **実際の上流にも、実際の DNS にも繋がない。** 上流は `wiremock` の偽物（`127.0.0.1`）、
//! 名前解決は [`ScriptedResolver`]（返すアドレスを途中で差し替えられる）。
//!
//! # 何を見るか
//!
//! | 見張り番 | 決定 |
//! |---|---|
//! | [`a_name_resolving_only_to_loopback_and_local_network_is_sent_with_a_warning`] | 名前解決が全部ローカルなら、警告して送る |
//! | [`one_outside_address_among_the_resolved_stops_every_send_before_it_reaches_the_upstream`] | 1つでも外が混ざれば、承認が無い送信は偽の上流に届かない（3つの送信の全部） |
//! | [`an_approval_lets_the_mixed_resolution_through_with_a_warning`] | 承認があれば、外が混ざっても警告して送る |
//! | [`a_resolution_that_turns_outside_after_the_first_send_stops_the_next_connection`] | **解決先が後から外へ変わる** —— 読み込みのときだけ判定する実装はここで赤になる |
//! | [`an_address_literal_is_judged_without_the_resolver`] | IP アドレスの宛先は名前解決を通らないので、別に判定する |
//! | [`an_address_literal_that_may_be_sent_is_actually_dialled`] | IP アドレスの宛先で、送ってよい判定（ローカル・承認あり）は本当に接続しに行く |
//! | [`keyless_https_and_loopback_faces_are_not_judged`] | 鍵の無い面・`https://`・ループバックは今までどおり |
//!
//! # 偽の上流は `127.0.0.1`、外のアドレスは `203.0.113.0/24`（文書用）
//!
//! **外が混ざる名前解決は「先頭に `127.0.0.1`、後ろに外」** にしてある。判定しない実装・先頭だけを見る
//! 実装は `127.0.0.1` へ繋いで偽の上流に届くので、**「届かない」がそのまま判定の効きになる。**
//! 外のアドレスへは接続しない（判定が止める）。

mod common;

use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use personacore::llm::{
    ApiKey, ChatMessage, ChatRequest, HostLookup, HostResolver, KeyOverPlainHttp, LlmError,
    LmStudioClient, LmStudioConfig, ModelId,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{SharedBuffer, install_global_log};

/// 面の鍵（囮）。
const KEY: &str = "pc-plain-http-KEY-5a6b7c8d";
const MODEL: &str = "planted-plain-http-model";
/// 偽の名前（`.test` は実在しない）。**実際の DNS には聞かない**（[`ScriptedResolver`] が答える）。
const HOST: &str = "upstream-a13b2.test";

fn log() -> &'static SharedBuffer {
    install_global_log("personacore=debug")
}

/// **返すアドレスを途中で差し替えられる名前解決。** 呼ばれた回数と、聞かれた名前を数える。
#[derive(Debug, Default)]
struct ScriptedResolver {
    answer: Mutex<Vec<IpAddr>>,
    lookups: AtomicUsize,
    asked: Mutex<Vec<String>>,
}

impl ScriptedResolver {
    fn answering(addresses: &[&str]) -> Arc<Self> {
        let resolver = Arc::new(Self::default());
        resolver.set(addresses);
        resolver
    }

    fn set(&self, addresses: &[&str]) {
        *self.answer.lock().expect("錠") = addresses
            .iter()
            .map(|raw| raw.parse().expect("正当なアドレス"))
            .collect();
    }

    fn lookups(&self) -> usize {
        self.lookups.load(Ordering::SeqCst)
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("錠").clone()
    }
}

impl HostResolver for ScriptedResolver {
    fn lookup(&self, host: &str) -> HostLookup {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.asked.lock().expect("錠").push(host.to_owned());
        let answer = self.answer.lock().expect("錠").clone();
        Box::pin(async move { Ok(answer) })
    }
}

/// **偽の名前で、偽の上流のポートを指す宛先。**
fn named(upstream: &MockServer) -> String {
    format!("http://{HOST}:{}", upstream.address().port())
}

fn config(
    endpoint: &str,
    key: Option<&str>,
    approved: bool,
    resolver: &Arc<ScriptedResolver>,
) -> LmStudioConfig {
    let model = ModelId::parse(MODEL).expect("正当なモデル識別子");
    let resolver: Arc<dyn HostResolver> = Arc::clone(resolver) as Arc<dyn HostResolver>;
    LmStudioConfig::new(endpoint, model)
        .expect("正当なベース URL")
        .with_api_key(key.map(|raw| ApiKey::parse(raw).expect("正当な鍵")))
        .with_key_over_plain_http_allowed(approved)
        .with_host_resolver(resolver)
        // **外のアドレスへ繋ぎに行く実装を、長く待たせない**（判定が効いていれば繋ぎに行かない）。
        .with_connect_timeout(Duration::from_secs(2))
        // **繋がってしまった相手に、既定の上限（180 秒）まで待たせない**。`10.0.0.5:9` は利用者の LAN で
        // 受け付けられうる。偽の上流と話すテストも同じ設定を使うので、負荷で揺れない幅にする。
        .with_idle_timeout(Duration::from_secs(10))
        .with_request_timeout(Duration::from_secs(10))
}

fn client(
    endpoint: &str,
    key: Option<&str>,
    approved: bool,
    resolver: &Arc<ScriptedResolver>,
) -> LmStudioClient {
    LmStudioClient::new(config(endpoint, key, approved, resolver))
        .expect("クライアントを組めること")
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

/// **全部の口に答える上流。** `close` なら、どの応答も接続を閉じる（次の送信は新しい接続 ＝ 新しい名前解決）。
async fn answering_upstream(close: bool) -> MockServer {
    let server = MockServer::start().await;
    let finish = move |template: ResponseTemplate| {
        if close {
            template.insert_header("connection", "close")
        } else {
            template
        }
    };
    Mock::given(method("GET"))
        .and(path("/api/v0/models"))
        .respond_with(finish(ResponseTemplate::new(404)))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(finish(json(format!(
            r#"{{"object":"list","data":[{{"id":"{MODEL}","object":"model","meta":{{"n_ctx":8192}}}}]}}"#
        ))))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(move |request: &wiremock::Request| {
            let streaming = String::from_utf8_lossy(&request.body).contains("\"stream\":true");
            let template = if streaming {
                ResponseTemplate::new(200).set_body_raw(stream_body(), "text/event-stream")
            } else {
                json(completion_body())
            };
            if close {
                template.insert_header("connection", "close")
            } else {
                template
            }
        })
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/apply-template"))
        .respond_with(finish(json(r#"{"prompt":"組み上がり"}"#.to_owned())))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/tokenize"))
        .respond_with(finish(json(r#"{"tokens":[1,2,3]}"#.to_owned())))
        .mount(&server)
        .await;
    server
}

/// **全部の口を1回ずつ叩く**（照会・非ストリーミング・ストリーミング・封・部品）。全部通ること。
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

/// **届いた要求の `Authorization`**（届いた順）。
async fn authorizations(server: &MockServer) -> Vec<Option<String>> {
    server
        .received_requests()
        .await
        .expect("記録が有効であること")
        .iter()
        .map(|request| {
            request
                .headers
                .get("authorization")
                .map(|value| value.to_str().expect("ASCII").to_owned())
        })
        .collect()
}

/// **名前解決が全部ループバックかローカルネットワークなら、警告して送る**（ADR-0056 §決定5）。
///
/// **名前解決は差し替えた口が答えている**（`.test` は実際の DNS では解決しない —— 口を通らない実装は
/// 繋がらずに赤になる）。警告の行は同じ行の中で `local_network` と宛先のホストを見る。
#[tokio::test]
async fn a_name_resolving_only_to_loopback_and_local_network_is_sent_with_a_warning() {
    let buffer = log();
    let upstream = answering_upstream(false).await;
    let resolver = ScriptedResolver::answering(&["127.0.0.1", "10.0.0.5", "fe80::1"]);
    let keyed = client(&named(&upstream), Some(KEY), false, &resolver);

    let mark = buffer.mark();
    exercise(&keyed).await;
    let logged = buffer.contents_since(mark);

    let seen = authorizations(&upstream).await;
    assert!(seen.len() >= 5, "前提: 全部の口が届いたこと: {seen:?}");
    for header in &seen {
        assert_eq!(header.as_deref(), Some(format!("Bearer {KEY}").as_str()));
    }
    assert!(resolver.lookups() >= 1, "差し替えた名前解決を通ったこと");
    assert!(
        resolver.asked().iter().all(|asked| asked == HOST),
        "聞かれた名前は宛先のホスト: {:?}",
        resolver.asked()
    );
    assert_eq!(
        keyed.key_over_plain_http(),
        Some(KeyOverPlainHttp::LocalNetwork)
    );
    assert!(
        logged.lines().any(|line| line.contains("WARN")
            && line.contains("key_over_plain_http=\"local_network\"")
            && line.contains(HOST)),
        "警告の行（同じ行に local_network と宛先のホスト）: {logged}"
    );
    assert!(!logged.contains(KEY), "鍵がログに出ている: {logged}");
}

/// **1つでも外が混ざれば、承認の無い送信は接続の前に止まり、偽の上流に届かない**（ADR-0056 §決定5）。
///
/// **3つの送信の全部**（照会・本文の2通り・数える口）を叩く。**外は列の末尾**にも先頭にも置く
/// （先頭の `127.0.0.1` だけを見る実装・接続先だけを見る実装を赤にする）。
#[tokio::test]
async fn one_outside_address_among_the_resolved_stops_every_send_before_it_reaches_the_upstream() {
    let buffer = log();
    for answer in [
        &["127.0.0.1", "10.0.0.5", "203.0.113.5"][..],
        &["127.0.0.1", "100.64.0.1"][..],
        &["203.0.113.5", "127.0.0.1"][..],
    ] {
        let upstream = answering_upstream(false).await;
        let resolver = ScriptedResolver::answering(answer);
        let keyed = client(&named(&upstream), Some(KEY), false, &resolver);

        let mark = buffer.mark();
        let listed = keyed.models().await.expect_err("照会は止まること");
        let completed = keyed
            .complete(&request())
            .await
            .expect_err("本文は止まること");
        let streamed = match keyed.stream(&request()).await {
            Ok(_) => panic!("中継も止まること"),
            Err(err) => err,
        };
        let counted = keyed
            .count_request(&request(), false)
            .await
            .expect_err("封も止まること");
        let logged = buffer.contents_since(mark);

        for err in [&listed, &completed, &streamed, &counted] {
            assert!(
                matches!(err, LlmError::KeyNotApproved),
                "{answer:?}: 承認が無くて送らなかったと読めること: {err:?}"
            );
            assert!(!err.refused_the_key(), "上流が断ったのではない: {err:?}");
        }
        assert_eq!(
            authorizations(&upstream).await,
            Vec::<Option<String>>::new(),
            "{answer:?}: 偽の上流に要求が届いている"
        );
        assert!(resolver.lookups() >= 1, "前提: 名前解決を通ったこと");
        assert_eq!(
            keyed.key_over_plain_http(),
            Some(KeyOverPlainHttp::NotApproved),
            "{answer:?}"
        );
        assert!(
            logged.lines().any(|line| line.contains("WARN")
                && line.contains("key_over_plain_http=\"not_approved\"")
                && line.contains("203.0.113.5") == answer.contains(&"203.0.113.5")),
            "{answer:?}: 外が混ざったと言う行: {logged}"
        );
    }
}

/// **承認があれば、外が混ざっても警告して送る**（ADR-0056 §決定5）。
#[tokio::test]
async fn an_approval_lets_the_mixed_resolution_through_with_a_warning() {
    let buffer = log();
    let upstream = answering_upstream(false).await;
    let resolver = ScriptedResolver::answering(&["127.0.0.1", "203.0.113.5"]);
    let keyed = client(&named(&upstream), Some(KEY), true, &resolver);

    let mark = buffer.mark();
    exercise(&keyed).await;
    let logged = buffer.contents_since(mark);

    let seen = authorizations(&upstream).await;
    assert!(seen.len() >= 5, "全部の口が届いたこと: {seen:?}");
    assert_eq!(
        keyed.key_over_plain_http(),
        Some(KeyOverPlainHttp::Approved)
    );
    assert!(
        logged.lines().any(|line| line.contains("WARN")
            && line.contains("key_over_plain_http=\"approved\"")
            && line.contains("203.0.113.5")),
        "承認で送ると言う行: {logged}"
    );
}

/// **解決先が後から外へ変わったら、次の接続から止まる**（ADR-0056 §決定5 / Issue origin#85 の見張り番）。
///
/// **読み込みのとき（クライアントを組んだとき）だけ判定する実装は、ここで赤になる** ——
/// 2回目も `127.0.0.1`（列の先頭）へ繋いで、偽の上流に2本目が届く。
/// 偽の上流は応答のたびに接続を閉じる（`Connection: close`）ので、2回目は必ず新しい接続 ＝ 新しい名前解決になる。
#[tokio::test]
async fn a_resolution_that_turns_outside_after_the_first_send_stops_the_next_connection() {
    let upstream = answering_upstream(true).await;
    let resolver = ScriptedResolver::answering(&["127.0.0.1"]);
    let keyed = client(&named(&upstream), Some(KEY), false, &resolver);

    keyed.complete(&request()).await.expect("1回目は届くこと");
    assert_eq!(
        keyed.key_over_plain_http(),
        None,
        "全部ループバックなら判定しない"
    );
    assert_eq!(authorizations(&upstream).await.len(), 1, "前提: 1本届いた");

    resolver.set(&["127.0.0.1", "203.0.113.5"]);
    let err = keyed
        .complete(&request())
        .await
        .expect_err("2回目は止まること");

    assert!(matches!(err, LlmError::KeyNotApproved), "{err:?}");
    assert_eq!(
        authorizations(&upstream).await.len(),
        1,
        "解決先が外へ変わった後の送信が、偽の上流に届いている"
    );
    assert_eq!(
        resolver.lookups(),
        2,
        "接続のたびに名前解決していること（2回の送信で2回）"
    );
    assert_eq!(
        keyed.key_over_plain_http(),
        Some(KeyOverPlainHttp::NotApproved)
    );

    // **戻れば、また届く**（判定は最後の名前解決のもの）。
    resolver.set(&["127.0.0.1", "192.168.0.7"]);
    keyed
        .complete(&request())
        .await
        .expect("ローカルへ戻れば届くこと");
    assert_eq!(authorizations(&upstream).await.len(), 2);
    assert_eq!(
        keyed.key_over_plain_http(),
        Some(KeyOverPlainHttp::LocalNetwork)
    );
}

/// **IP アドレスの宛先は名前解決を通らないので、組んだときに判定する**（ADR-0056 §決定5 / Issue origin#85 のコメント）。
///
/// **外のアドレスで承認が無ければ、どの送信も接続せずに止まる。** 判定しない実装は
/// `203.0.113.5:9` へ繋ぎに行き、接続の上限（2 秒）で `Connect` / `Timeout` になる（`KeyNotApproved` ではない）。
/// **ローカルネットワークのアドレス・承認のある外は、送らずに判定だけを見る**（実際の LAN へ繋がない）。
#[tokio::test]
async fn an_address_literal_is_judged_without_the_resolver() {
    let resolver = ScriptedResolver::answering(&["127.0.0.1"]);

    let outside = client("http://203.0.113.5:9", Some(KEY), false, &resolver);
    assert_eq!(
        outside.key_over_plain_http(),
        Some(KeyOverPlainHttp::NotApproved),
        "組んだ時点で判定が決まっていること（送る前）"
    );
    let listed = outside.models().await.expect_err("照会は止まること");
    let completed = outside
        .complete(&request())
        .await
        .expect_err("本文は止まること");
    let streamed = match outside.stream(&request()).await {
        Ok(_) => panic!("中継も止まること"),
        Err(err) => err,
    };
    let counted = outside
        .count_text("こんにちは")
        .await
        .expect_err("部品も止まること");
    for err in [&listed, &completed, &streamed, &counted] {
        assert!(matches!(err, LlmError::KeyNotApproved), "{err:?}");
    }

    for (endpoint, approved, expected) in [
        (
            "http://10.0.0.5:9",
            false,
            Some(KeyOverPlainHttp::LocalNetwork),
        ),
        (
            "http://172.31.255.255:9",
            false,
            Some(KeyOverPlainHttp::LocalNetwork),
        ),
        (
            "http://172.32.0.0:9",
            false,
            Some(KeyOverPlainHttp::NotApproved),
        ),
        (
            "http://[fd00::5]:9",
            false,
            Some(KeyOverPlainHttp::LocalNetwork),
        ),
        (
            "http://203.0.113.5:9",
            true,
            Some(KeyOverPlainHttp::Approved),
        ),
        (
            "http://100.64.0.1:9",
            true,
            Some(KeyOverPlainHttp::Approved),
        ),
        // **WHATWG で読んだホスト**（`\` は経路の区切り）—— 外へ繋ぐ綴りは外として判定する。
        (
            r"http://203.0.113.5\@127.0.0.1:46001",
            false,
            Some(KeyOverPlainHttp::NotApproved),
        ),
    ] {
        assert_eq!(
            client(endpoint, Some(KEY), approved, &resolver).key_over_plain_http(),
            expected,
            "{endpoint} / 承認 {approved}"
        );
    }
    assert_eq!(
        resolver.lookups(),
        0,
        "IP アドレスの宛先は名前解決を通らない"
    );
}

/// **IP アドレスの宛先で「送る」側の判定は、本当に接続しに行く**（作業単位 A-13b-2 のレビュー是正）。
///
/// ローカルネットワークのアドレス（承認なし）と、外のアドレス（承認あり）へ `complete` を送ると、
/// **接続を試みて失敗する**（`Connect` / `Timeout`）—— `KeyNotApproved` ではない。
/// **判定が付いた宛先を全部止める実装**（`refuses_before_connecting` を `fixed.is_some()` にする変異）は
/// ここで赤になる（是正前は判定だけを見ていたので緑だった）。
/// どちらも届かないアドレスなので、接続の上限（2 秒）で終わる。
#[tokio::test]
async fn an_address_literal_that_may_be_sent_is_actually_dialled() {
    let resolver = ScriptedResolver::answering(&["127.0.0.1"]);
    for (endpoint, approved, expected) in [
        ("http://10.0.0.5:9", false, KeyOverPlainHttp::LocalNetwork),
        ("http://203.0.113.5:9", true, KeyOverPlainHttp::Approved),
    ] {
        let literal = client(endpoint, Some(KEY), approved, &resolver);
        assert_eq!(
            literal.key_over_plain_http(),
            Some(expected),
            "前提: {endpoint} の判定"
        );
        let err = literal
            .complete(&request())
            .await
            .expect_err("届かないアドレスなので失敗すること");
        assert!(
            matches!(err, LlmError::Connect(_) | LlmError::Timeout(_)),
            "{endpoint} / 承認 {approved}: 接続を試みずに止めている: {err:?}"
        );
    }
    assert_eq!(
        resolver.lookups(),
        0,
        "IP アドレスの宛先は名前解決を通らない"
    );
}

/// **鍵の無い面・`https://`・ループバックは判定しない**（扱いを変えない / ADR-0056 §決定5）。
///
/// **鍵の無い外の `http://` は今までどおり繋ぎに行く**（承認を求めない）—— 偽の上流（`127.0.0.1`）に届く。
#[tokio::test]
async fn keyless_https_and_loopback_faces_are_not_judged() {
    let resolver = ScriptedResolver::answering(&["203.0.113.5"]);
    for (endpoint, key) in [
        ("http://203.0.113.5:9", None),
        ("http://192.168.0.7:9", None),
        ("https://203.0.113.5:9", Some(KEY)),
        ("https://example.com:9", Some(KEY)),
        ("http://127.0.0.1:9", Some(KEY)),
        ("http://localhost:9", Some(KEY)),
        ("http://[::1]:9", Some(KEY)),
    ] {
        assert_eq!(
            client(endpoint, key, false, &resolver).key_over_plain_http(),
            None,
            "{endpoint}"
        );
    }

    // **鍵の無いループバックの面は、今までどおり届く**（`Authorization` も付かない）。
    let upstream = answering_upstream(false).await;
    let keyless = client(&upstream.uri(), None, false, &resolver);
    exercise(&keyless).await;
    assert!(
        authorizations(&upstream).await.iter().all(Option::is_none),
        "鍵の無い面に何か付いている"
    );
    assert_eq!(
        resolver.lookups(),
        0,
        "判定しない宛先は差し替えた名前解決を通らない"
    );
}
