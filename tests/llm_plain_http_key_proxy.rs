//! **鍵を付けた、ループバック以外の `http://` の面は、環境変数のプロキシを通さない**
//! （ADR-0056 §決定5 / 作業単位 A-13b-2 のレビュー是正 / Issue origin#85）。
//!
//! # なぜ要るのか
//!
//! プロキシを通すと、`reqwest` が名前解決するのは**プロキシのホスト**で、宛先のホスト名は解決しない
//! （判定の口 `dns_resolver` が呼ばれない）。**鍵はプロキシから先へ平文で流れ、判定した先と鍵が届く先が割れる。**
//! だから `LmStudioClient::new` は、判定する面だけ `no_proxy` で組む。**この1本は、その `no_proxy` を外す変異を赤にする**
//! （是正前は外しても全緑だった —— 変異 `m17`）。
//!
//! # プロキシは環境変数からだけ読まれる
//!
//! この crate の `reqwest` は `default-features = false` で `system-proxy` を持たないので、プロキシは
//! `HTTP_PROXY` / `http_proxy` などの環境変数だけから、**クライアントを組む時点で**読まれる。
//! だからこのテストは、**環境変数を書き換えてからクライアントを組む。**
//!
//! # ⚠ このバイナリにテストを増やさないこと
//!
//! **このファイルのテストは1本だけである。それが安全の根拠。**
//! 下の1本は `std::env::set_var` / `remove_var` で**プロセス全体の**環境を書き換える。
//! 同じプロセスで別のテストが並行に走ると、そのテストのクライアントまでプロキシを読む（あるいは
//! 読んでいる最中に書き換わって未定義動作になる）。**統合テストはファイルごとに別プロセス**なので、
//! 他のテストバイナリには届かない（先例は `tests/calibration_env_wiring.rs`）。
//! **2本目を足した瞬間、その隔離が消える。** 対照も、別のテストに割らず下の1本の中に置く。

use std::io::{Read as _, Write as _};
use std::net::{IpAddr, TcpListener};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use personacore::llm::{
    ApiKey, ChatMessage, ChatRequest, HostLookup, HostResolver, LlmError, LmStudioClient,
    LmStudioConfig, ModelId,
};

/// 面の鍵（囮）。
const KEY: &str = "pc-plain-proxy-KEY-1f2e3d4c";
/// 偽の名前（`.test` は実在しない）。**実際の DNS には聞かない。**
const ENDPOINT: &str = "http://upstream-proxy-a13b2.test:1";

/// **返すアドレスを決めた名前解決。** 呼ばれた回数を数える。
#[derive(Debug)]
struct OutsideResolver {
    answer: Vec<IpAddr>,
    lookups: AtomicUsize,
}

impl HostResolver for OutsideResolver {
    fn lookup(&self, _host: &str) -> HostLookup {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        let answer = self.answer.clone();
        Box::pin(async move { Ok(answer) })
    }
}

/// **偽のプロキシ**（`127.0.0.1`）。受け取ったバイト数を足し、届いた中身を控え、502 を返して閉じる。
struct FakeProxy {
    port: u16,
    received: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<u8>>>,
}

impl FakeProxy {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("偽のプロキシを立てられること");
        let port = listener.local_addr().expect("ポート").port();
        let received = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (counter, copy) = (Arc::clone(&received), Arc::clone(&seen));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let mut buffer = [0u8; 4096];
                if let Ok(read) = stream.read(&mut buffer) {
                    counter.fetch_add(read, Ordering::SeqCst);
                    copy.lock().expect("錠").extend_from_slice(&buffer[..read]);
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        Self {
            port,
            received,
            seen,
        }
    }

    fn received(&self) -> usize {
        self.received.load(Ordering::SeqCst)
    }

    fn seen(&self) -> String {
        String::from_utf8_lossy(&self.seen.lock().expect("錠")).into_owned()
    }
}

fn client(endpoint: &str, key: Option<&str>, resolver: &Arc<OutsideResolver>) -> LmStudioClient {
    let model = ModelId::parse("planted-proxy-model").expect("正当なモデル識別子");
    let resolver: Arc<dyn HostResolver> = Arc::clone(resolver) as Arc<dyn HostResolver>;
    let config = LmStudioConfig::new(endpoint, model)
        .expect("正当なベース URL")
        .with_api_key(key.map(|raw| ApiKey::parse(raw).expect("正当な鍵")))
        .with_host_resolver(resolver)
        .with_connect_timeout(Duration::from_secs(2))
        // **繋がってしまった相手に、既定の上限（180 秒）まで待たせない**（`10.0.0.5:9` は利用者の LAN で
        // 受け付けられうる）。
        .with_idle_timeout(Duration::from_secs(10))
        .with_request_timeout(Duration::from_secs(10));
    LmStudioClient::new(config).expect("クライアントを組めること")
}

fn request() -> ChatRequest {
    ChatRequest::new(vec![ChatMessage::user("こんにちは")])
}

/// **環境変数のプロキシがあっても、判定する面はプロキシへ1バイトも送らず、宛先を名前解決して止まる。**
///
/// | 段 | クライアント | 期待 |
/// |---|---|---|
/// | 1 | 鍵あり・承認なし・宛先はホスト名（外に解決） | `KeyNotApproved`。名前解決の口が呼ばれ、**偽のプロキシは何も受け取らない** |
/// | 2 | 鍵あり・宛先は IP アドレスのローカルネットワーク（送ってよい） | 宛先へ直に繋ぎに行って失敗する（`Connect` / `Timeout`）。**偽のプロキシは何も受け取らない** |
/// | 3（対照） | 鍵なし（判定しない面） | **偽のプロキシに届く**（プロキシの環境変数が本当に読まれていること —— これが無いと段1・2は空振りでも緑） |
///
/// `no_proxy` を外した実装は、段1・2でプロキシへ繋ぎ、鍵の付いた要求を平文で渡す（段1では名前解決の口も呼ばれない）。
/// 段1はホスト名の枝の `no_proxy`、段2は IP アドレスの枝の `no_proxy` を見る。
#[test]
fn a_judged_face_never_goes_through_the_environment_proxy() {
    let proxy = FakeProxy::start();
    let proxy_url = format!("http://127.0.0.1:{}", proxy.port);

    // SAFETY: このバイナリのテストは1本だけで、環境を読む別のスレッドが居ない（モジュールの doc）。
    // 偽のプロキシのスレッドは環境を読まない。実行時（tokio）はこの後で組む。
    unsafe {
        for name in [
            "NO_PROXY",
            "no_proxy",
            "ALL_PROXY",
            "all_proxy",
            "REQUEST_METHOD",
        ] {
            std::env::remove_var(name);
        }
        std::env::set_var("HTTP_PROXY", &proxy_url);
        std::env::set_var("http_proxy", &proxy_url);
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("実行時を組めること");

    // --- 1. 判定する面 ------------------------------------------------------
    let resolver = Arc::new(OutsideResolver {
        answer: vec!["203.0.113.5".parse().expect("正当なアドレス")],
        lookups: AtomicUsize::new(0),
    });
    let keyed = client(ENDPOINT, Some(KEY), &resolver);
    let err = runtime
        .block_on(keyed.complete(&request()))
        .expect_err("承認が無いので止まること");
    assert!(
        matches!(err, LlmError::KeyNotApproved),
        "承認が無くて送らなかったと読めること: {err:?}"
    );
    assert!(
        resolver.lookups.load(Ordering::SeqCst) >= 1,
        "宛先のホスト名を名前解決していない（プロキシへ回した）"
    );
    // 偽のプロキシのスレッドが読み終える時間を置く（届いていれば、ここまでに数えられる）。
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        proxy.received(),
        0,
        "判定する面の要求がプロキシに届いている: {}",
        proxy.seen()
    );

    // --- 2. IP アドレスの宛先（送ってよい判定） -----------------------------
    let literal = client("http://10.0.0.5:9", Some(KEY), &resolver);
    let err = runtime
        .block_on(literal.complete(&request()))
        .expect_err("届かないアドレスなので失敗すること");
    assert!(
        matches!(err, LlmError::Connect(_) | LlmError::Timeout(_)),
        "宛先へ直に繋ぎに行っていない: {err:?}"
    );
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        proxy.received(),
        0,
        "IP アドレスの宛先の要求がプロキシに届いている: {}",
        proxy.seen()
    );

    // --- 3. 対照: 判定しない面はプロキシを通る ------------------------------
    let keyless = client(ENDPOINT, None, &resolver);
    let _ = runtime.block_on(keyless.complete(&request()));
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while proxy.received() == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        proxy.received() > 0,
        "前提: プロキシの環境変数が読まれていること（鍵の無い面がプロキシへ届かない）"
    );
    assert!(
        proxy.seen().contains("upstream-proxy-a13b2.test"),
        "前提: プロキシに届いたのはこの宛先への要求であること: {}",
        proxy.seen()
    );
    assert!(
        !proxy.seen().contains(KEY),
        "鍵の無い面の要求に鍵が載っている"
    );

    // SAFETY: 同上。
    unsafe {
        std::env::remove_var("HTTP_PROXY");
        std::env::remove_var("http_proxy");
    }
}
