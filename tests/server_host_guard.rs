//! **`Host` / `Origin` の検査**（作業単位 B-58）を、**実際の TCP で待ち受けたサーバに**当てて確かめる。
//! **LM Studio には繋がない**（偽の上流を注入するか、閉じたポートを宛先にする）。
//!
//! # 何を固定するか
//!
//! | 主張 | 破れると |
//! |---|---|
//! | 外の `Host` の保存の依頼は 403 で、**ファイルは1バイトも変わらない** | ブラウザで開いた外のページが、DNS rebinding で起動するコマンドを書ける |
//! | `Host` のポートは**実際に待ち受けているポート**と比べる | 設定値（0 / 8080）と比べると、正しい要求まで断るか、別のポートの名を通す |
//! | `127.0.0.1:<port>` / `localhost:<port>` / `Host` 無しは通る | 画面も API も使えなくなる |
//! | `Origin` は自分のオリジンか、無いときだけ通る（`null` は断る） | 外のページからの書き込みの依頼が届く |
//! | 画面（静的ファイル）にも効く | 画面の資産だけが外の名前に答える |
//! | 403 にも版のヘッダが付く（ADR-0047 §② P-2b） | ルータに届いた応答のうち、この 403 だけが版を名乗らない |
//!
//! # 要求は生の TCP で書く
//!
//! **HTTP クライアントは `Host` を自分で足す**ので、「`Host` が無い要求」を作れない。
//! 何がソケットに乗ったかを曖昧にしないため、このファイルの要求は全部 [`raw`] で書く。
//!
//! # フィクスチャのポートは全部違える
//!
//! **実際のポート**（OS が選ぶ）・**既定のポート**（[`DEFAULT_PORT`]）・**`Host` に書く別のポート**
//! （[`other_port`]）は、毎回違う値であることを先に確かめる（同じだと取り違えが観測できない）。
//! **設定のポートは 0**（[`TestServer::start`]）——設定値と比べる変異は、正しい要求まで断って赤になる。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use personacore::llm::{
    ChatRequest, ChatStream, Completion, FinishReason, LlmClient, LlmError, LlmFuture,
};
use personacore::server::{
    API_VERSION, App, AppConfig, BindAddr, DEFAULT_PORT, ENV_LLM_ENDPOINT, ENV_WEB_DIR, LaunchPlan,
    ServerError, Startup,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// 断ったときの `error.code`（**司令塔の決定の綴りを写す**——定数から作らない）。
const REFUSED_CODE: &str = "request_origin_not_allowed";

/// 版のヘッダの名前（小文字。生の応答の見出しは小文字にして探す）。
const VERSION_HEADER: &str = "x-personacore-api-version";

/// 画面の配り先に置く目印（`vite` が出さない綴り）。
const UI_INDEX: &str = "<!doctype html><title>HOST-GUARD-INDEX</title>";
const UI_ASSET_PATH: &str = "assets/host-guard-marker.js";
const UI_ASSET: &str = "/* HOST-GUARD-ASSET */";

/// 1回の依頼の上限（待つ側に置く / CLAUDE.md §4）。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// 本文を返すだけの偽の上流（このファイルは本文生成を見ない）。
struct MockLlm;

impl LlmClient for MockLlm {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        Box::pin(async move {
            Ok(Completion {
                content: "「こんにちは」".to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        Box::pin(async move { Err(LlmError::MissingContent) })
    }
}

// ----------------------------------------------------------------------
// 生の要求
// ----------------------------------------------------------------------

/// 1回の応答（状態・見出し・本文）。
struct Reply {
    status: u16,
    /// 見出し（名前は小文字）。
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header_values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .collect()
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|err| panic!("JSON として読めること: {err} / {}", self.body))
    }

    /// **断られた応答の形**（403・OpenAI の形の error・新しい `code`・版のヘッダ）を確かめる。
    fn assert_refused(&self, context: &str) {
        assert_eq!(self.status, 403, "{context}: {}", self.body);
        let body = self.json();
        assert_eq!(body["error"]["code"], REFUSED_CODE, "{context}: {body}");
        assert_eq!(
            body["error"]["type"], "invalid_request_error",
            "{context}: {body}"
        );
        assert!(
            body["error"]["message"].is_string(),
            "{context}: message が無い: {body}"
        );
        assert_eq!(
            self.header_values(VERSION_HEADER),
            vec![API_VERSION],
            "{context}: 断った応答にも版のヘッダが1つだけ付くこと"
        );
    }
}

/// **生の HTTP/1.1 で1往復する。** `headers` に `Host` を書かなければ、`Host` の無い要求になる。
///
/// `Connection: close` を付け、応答の終わりを `read_to_end` の終端で知る。
async fn raw(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: Option<&str>,
) -> Reply {
    let mut head = format!("{method} {path} HTTP/1.1\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(body) = body {
        head.push_str("Content-Type: application/json\r\n");
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("Connection: close\r\n\r\n");
    head.push_str(body.unwrap_or(""));

    let exchange = async {
        let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
        socket.write_all(head.as_bytes()).await?;
        let mut answer = Vec::new();
        socket.read_to_end(&mut answer).await?;
        Ok::<_, std::io::Error>(answer)
    };
    let answer = tokio::time::timeout(REQUEST_TIMEOUT, exchange)
        .await
        .expect("上限の中で答えること")
        .expect("サーバと往復できること");
    let text = String::from_utf8(answer).expect("応答は UTF-8 であること");
    let (head, body) = text
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("見出しと本文の境界が無い: {text}"));
    let mut lines = head.split("\r\n");
    let status_line = lines.next().expect("状態の行があること");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("状態の行を読めない: {status_line}"));
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    Reply {
        status,
        headers,
        body: body.to_owned(),
    }
}

fn host(value: impl Into<String>) -> (&'static str, String) {
    ("Host", value.into())
}

fn origin(value: impl Into<String>) -> (&'static str, String) {
    ("Origin", value.into())
}

/// **`Host` に書く別のポート。** 実際のポートとも既定のポートとも違う値。
fn other_port(actual: u16) -> u16 {
    let candidate = if actual == u16::MAX {
        actual - 1
    } else {
        actual + 1
    };
    if candidate == DEFAULT_PORT {
        candidate + 1
    } else {
        candidate
    }
}

/// **フィクスチャの3つのポートが全部違うこと**（同じだと取り違えが観測できない）。
fn assert_ports_differ(actual: u16) -> u16 {
    let other = other_port(actual);
    assert_ne!(actual, DEFAULT_PORT, "前提: 実際のポートが既定と同じ");
    assert_ne!(other, actual, "前提: 別のポートが実際のポートと同じ");
    assert_ne!(other, DEFAULT_PORT, "前提: 別のポートが既定と同じ");
    assert_ne!(
        actual, 80,
        "前提: ポートを省いた `Host` の腕が意味を持たない"
    );
    other
}

// ----------------------------------------------------------------------
// 注入の入口（ポート 0）で起こすサーバ
// ----------------------------------------------------------------------

struct TestServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    /// **設定のポートは 0**（OS に選ばせる）。**画面も配る**（配り先は目印入りの一時ディレクトリ）。
    async fn start() -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let web_dir = dir.path().join("web-dist");
        std::fs::create_dir_all(web_dir.join("assets")).expect("配り先を作れること");
        std::fs::write(web_dir.join("index.html"), UI_INDEX).expect("置けること");
        std::fs::write(web_dir.join(UI_ASSET_PATH), UI_ASSET).expect("置けること");
        let web_dir = web_dir.display().to_string();
        let config = AppConfig::from_lookup(|key| (key == ENV_WEB_DIR).then(|| web_dir.clone()))
            .expect("設定を組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(dir.path().join("personacore.sqlite"));
        let llm = Arc::new(MockLlm);
        let app = App::bind_with_client(&config, llm)
            .await
            .expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });
        Self {
            addr,
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    const fn port(&self) -> u16 {
        self.addr.port()
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).expect("停止シグナルを送れること");
        }
        if let Some(handle) = self.handle.take() {
            handle
                .await
                .expect("サーバタスクが panic しないこと")
                .expect("正常に終了すること");
        }
    }
}

// ----------------------------------------------------------------------
// 本番の入口（編集できるプロファイルの置き場所つき）で起こすサーバ
// ----------------------------------------------------------------------

/// 上流の宛先（閉じている loopback のポート）。**LM Studio に繋ぎに行かせない。**
const CLOSED_ENDPOINT: &str = "http://127.0.0.1:9";

/// 待つ側の上限（回数）と1回あたりの待ち時間。
const MAX_WAITS: usize = 300;
const WAIT_STEP: Duration = Duration::from_millis(100);

/// ポートを取られて待ち受けを開けなかったとき、別のポートで起こし直す回数の上限。
const MAX_BINDS: usize = 5;

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("ポートを取れること");
    let port = listener.local_addr().expect("宛先を読めること").port();
    drop(listener);
    port
}

struct Serving {
    port: u16,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), ServerError>>>,
}

impl Serving {
    /// **本番の入口でサーバを起こし、一覧の口が答えるまで待つ。**
    ///
    /// **環境変数は1つも読まない。** ポートを取られたら別のポートで起こし直す
    /// （`tests/upstream_profile_editor.rs` の `serve` と同じ理由）。
    async fn start(dir: &Path, profiles: PathBuf) -> Self {
        for _ in 0..MAX_BINDS {
            let port = free_port();
            let config = AppConfig::from_lookup_with_data_dir(
                |key: &str| (key == ENV_LLM_ENDPOINT).then(|| CLOSED_ENDPOINT.to_owned()),
                Some(dir.to_path_buf()),
            )
            .expect("設定を組めること")
            .with_bind(BindAddr::loopback(port))
            .with_database_path(dir.join("db").join("personacore.sqlite"));
            let plan =
                Startup::new(LaunchPlan::none()).with_editable_profiles_file(profiles.clone());
            let (shutdown, wait) = oneshot::channel();
            let task = tokio::spawn(async move {
                App::run_with_upstreams(&config, plan, async move {
                    let _ = wait.await;
                })
                .await
            });
            let mut serving = Self {
                port,
                shutdown: Some(shutdown),
                task: Some(task),
            };
            if serving.answers().await {
                return serving;
            }
            serving.task = None;
        }
        panic!("{MAX_BINDS} 回起こし直しても待ち受けを開けなかった");
    }

    /// **この待ち受けが一覧の口に答えるまで待つ。** サーバのタスクが終わったら `false`。
    async fn answers(&self) -> bool {
        for _ in 0..MAX_WAITS {
            if self
                .task
                .as_ref()
                .is_none_or(tokio::task::JoinHandle::is_finished)
            {
                return false;
            }
            let answered = tokio::time::timeout(Duration::from_secs(2), async {
                let socket = tokio::net::TcpStream::connect(("127.0.0.1", self.port)).await;
                socket.is_ok()
            })
            .await
            .unwrap_or(false);
            if answered {
                let reply = raw(
                    self.port,
                    "GET",
                    "/admin/upstream/profiles",
                    &[host(format!("127.0.0.1:{}", self.port))],
                    None,
                )
                .await;
                if reply.status == 200
                    && serde_json::from_str::<Value>(&reply.body)
                        .is_ok_and(|body| body.get("editable").is_some())
                {
                    return true;
                }
            }
            tokio::time::sleep(WAIT_STEP).await;
        }
        panic!("待ち受けを {MAX_WAITS} 回待っても答えなかった");
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.await
                .expect("サーバのタスクが落ちていないこと")
                .expect("サーバが正常に止まること");
        }
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// 手で書いたプロファイル（1つ）。
fn one_profile() -> Value {
    json!({
        "north": {
            "main": { "endpoint": "http://127.0.0.1:47301", "model": "north-main" }
        }
    })
}

/// **保存の依頼の本文**（起動するコマンドを書き込む形 —— 塞ぐ対象そのもの）。
fn evil_save_body(revision: &Value) -> String {
    json!({
        "revision": revision,
        "main": {
            "endpoint": "http://127.0.0.1:47302",
            "model": "evil-main",
            "gguf": null,
            "args": null,
            "command": "C:/evil/payload.exe"
        }
    })
    .to_string()
}

// ----------------------------------------------------------------------
// 本命
// ----------------------------------------------------------------------

/// **外の `Host` から保存の口を叩いても 403 で、ファイルは1バイトも変わらない。**
///
/// ポートだけ違う `Host`・外の `Origin` の腕も同じ保存の口で見る。
/// **最後に正しい `Host` で同じ依頼を送り、実際に書けることを確かめる**
/// ——書けないフィクスチャで「変わらなかった」を見ても何も言えない。
#[tokio::test]
async fn a_foreign_host_cannot_save_a_profile_and_nothing_is_written() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let profiles_dir = dir.path().join("app");
    std::fs::create_dir_all(&profiles_dir).expect("作れること");
    let path = profiles_dir.join("upstreams.json");
    std::fs::write(&path, one_profile().to_string()).expect("置けること");
    let original = std::fs::read(&path).expect("読めること");

    let server = Serving::start(dir.path(), path.clone()).await;
    let port = server.port;
    let other = assert_ports_differ(port);
    let own = format!("127.0.0.1:{port}");

    let listing = raw(port, "GET", "/admin/upstream/profiles", &[host(&own)], None).await;
    assert_eq!(listing.status, 200, "{}", listing.body);
    let revision = listing.json()["revision"].clone();
    let body = evil_save_body(&revision);

    let refused_arms: [(&str, Vec<(&str, String)>); 4] = [
        ("外の Host", vec![host(format!("evil.example:{port}"))]),
        (
            "ポートだけ違う Host",
            vec![host(format!("127.0.0.1:{other}"))],
        ),
        (
            "外の Origin",
            vec![host(&own), origin(format!("http://evil.example:{port}"))],
        ),
        ("Origin: null", vec![host(&own), origin("null")]),
    ];
    for (label, headers) in &refused_arms {
        let reply = raw(
            port,
            "PUT",
            "/admin/upstream/profiles/evil",
            headers,
            Some(&body),
        )
        .await;
        reply.assert_refused(label);
        assert_eq!(
            std::fs::read(&path).expect("読めること"),
            original,
            "{label}: 断ったのにファイルが変わった"
        );
    }

    // **一覧も変わっていない**（印が同じ）。
    let again = raw(port, "GET", "/admin/upstream/profiles", &[host(&own)], None).await;
    assert_eq!(again.json()["revision"], revision, "{}", again.body);

    // **対照: 正しい `Host`（と自分の `Origin`）なら同じ依頼で書ける。**
    let saved = raw(
        port,
        "PUT",
        "/admin/upstream/profiles/evil",
        &[host(&own), origin(format!("http://{own}"))],
        Some(&body),
    )
    .await;
    assert_eq!(saved.status, 200, "{}", saved.body);
    assert_ne!(
        std::fs::read(&path).expect("読めること"),
        original,
        "対照: 正しい Host でも書けていない（フィクスチャが書けない形になっている）"
    );

    server.stop().await;
}

/// **外の `Host` / `Origin` から待ち受けのポートの保存の口を叩いても 403 で、保存のファイルは1バイトも変わらない**
/// （ADR-0057 §決定6「`PUT` は設定を書く口なので、DNS rebinding の検査が守る経路そのもの」/ 作業単位 F-12a）。
///
/// **最後に正しい `Host` で同じ依頼を送り、実際に書けることを確かめる**（上の保存の口の腕と同じ理由）。
#[tokio::test]
async fn a_foreign_host_cannot_save_the_listening_port_and_nothing_is_written() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let profiles_dir = dir.path().join("app");
    std::fs::create_dir_all(&profiles_dir).expect("作れること");
    let path = profiles_dir.join("upstreams.json");
    std::fs::write(&path, one_profile().to_string()).expect("置けること");
    let saved = profiles_dir.join("listening-port.txt");
    std::fs::write(&saved, "8090\n").expect("置けること");
    let original = std::fs::read(&saved).expect("読めること");

    let server = Serving::start(dir.path(), path.clone()).await;
    let port = server.port;
    let other = assert_ports_differ(port);
    let own = format!("127.0.0.1:{port}");
    let body = json!({ "port": 9123 }).to_string();

    let refused_arms: [(&str, Vec<(&str, String)>); 4] = [
        ("外の Host", vec![host(format!("evil.example:{port}"))]),
        (
            "ポートだけ違う Host",
            vec![host(format!("127.0.0.1:{other}"))],
        ),
        (
            "外の Origin",
            vec![host(&own), origin(format!("http://evil.example:{port}"))],
        ),
        ("Origin: null", vec![host(&own), origin("null")]),
    ];
    for (label, headers) in &refused_arms {
        let reply = raw(port, "PUT", "/admin/server/port", headers, Some(&body)).await;
        reply.assert_refused(label);
        assert_eq!(
            std::fs::read(&saved).expect("読めること"),
            original,
            "{label}: 断ったのに保存のファイルが変わった"
        );
    }
    // **読む口も同じ検査を通る**（`GET` だけ素通しする形を作らない）。
    raw(
        port,
        "GET",
        "/admin/server",
        &[host(format!("evil.example:{port}"))],
        None,
    )
    .await
    .assert_refused("外の Host の GET");

    // **対照: 正しい `Host`（と自分の `Origin`）なら同じ依頼で書ける。**
    let accepted = raw(
        port,
        "PUT",
        "/admin/server/port",
        &[host(&own), origin(format!("http://{own}"))],
        Some(&body),
    )
    .await;
    assert_eq!(accepted.status, 200, "{}", accepted.body);
    assert_eq!(
        std::fs::read_to_string(&saved).expect("読めること"),
        "9123\n",
        "対照: 正しい Host でも書けていない（フィクスチャが書けない形になっている）"
    );

    server.stop().await;
}

/// **`Host` は `127.0.0.1` / `localhost` と、実際に待ち受けているポートの組だけが通る。**
///
/// **設定のポートは 0** なので、設定値と比べる実装は通るべき行まで断って赤になる。
#[tokio::test]
async fn only_loopback_names_with_the_listening_port_pass_the_host_check() {
    let server = TestServer::start().await;
    let port = server.port();
    let other = assert_ports_differ(port);

    let passing: Vec<Option<String>> = vec![
        Some(format!("127.0.0.1:{port}")),
        Some(format!("localhost:{port}")),
        // **名前の大文字小文字は区別しない**（DNS の名前は区別しない。ブラウザは小文字にして送る）。
        Some(format!("LOCALHOST:{port}")),
        Some(format!("LocalHost:{port}")),
        // **`Host` の無い要求は通す**（ブラウザは `Host` を省けない）。
        None,
    ];
    for value in &passing {
        let headers: Vec<(&str, String)> = value.iter().map(host).collect();
        let reply = raw(port, "GET", "/v1/models", &headers, None).await;
        assert_eq!(
            reply.status, 200,
            "Host {value:?} が通らない: {}",
            reply.body
        );
        assert!(
            reply.json()["data"].is_array(),
            "Host {value:?}: 一覧が返っていない: {}",
            reply.body
        );
    }

    let refused: Vec<String> = vec![
        format!("evil.example:{port}"),
        "evil.example".to_owned(),
        format!("127.0.0.1:{other}"),
        format!("localhost:{other}"),
        format!("127.0.0.1:{DEFAULT_PORT}"),
        format!("localhost:{DEFAULT_PORT}"),
        // **ポートを省いた形は 80 と読む**（実際のポートは 80 ではない）。
        "127.0.0.1".to_owned(),
        "localhost".to_owned(),
        // **末尾の `.` は断る**（正規の使い手は付けない）。
        format!("localhost.:{port}"),
        // **待ち受けていない名前**（`127.0.0.1` 以外の loopback・IPv6）。
        format!("127.0.0.2:{port}"),
        format!("[::1]:{port}"),
        // **ポートの綴りは実際のポートの10進そのものだけ。**
        format!("127.0.0.1:0{port}"),
        format!("127.0.0.1:+{port}"),
        format!("127.0.0.1:{port}:{port}"),
        format!("localhost:{port}.evil.example"),
        format!("evil.example.localhost:{port}"),
        format!("user@127.0.0.1:{port}"),
        "127.0.0.1:".to_owned(),
    ];
    for value in &refused {
        let reply = raw(port, "GET", "/v1/models", &[host(value)], None).await;
        reply.assert_refused(&format!("Host {value}"));
    }

    // **`Host` が2つある要求も断る**（どちらを信じるかを決めない）。
    let reply = raw(
        port,
        "GET",
        "/v1/models",
        &[
            host(format!("127.0.0.1:{port}")),
            host(format!("evil.example:{port}")),
        ],
        None,
    )
    .await;
    reply.assert_refused("Host が2つ");

    server.stop().await;
}

/// **`Origin` は、無いか、自分のオリジンのときだけ通る。** `null` は断る。
#[tokio::test]
async fn the_origin_must_be_this_server_or_absent() {
    let server = TestServer::start().await;
    let port = server.port();
    let other = assert_ports_differ(port);
    let own = format!("127.0.0.1:{port}");

    let passing = [
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ];
    for value in &passing {
        let reply = raw(
            port,
            "GET",
            "/v1/models",
            &[host(&own), origin(value)],
            None,
        )
        .await;
        assert_eq!(
            reply.status, 200,
            "Origin {value} が通らない: {}",
            reply.body
        );
    }
    // **無い腕**（同じ `Host` で）。
    let reply = raw(port, "GET", "/v1/models", &[host(&own)], None).await;
    assert_eq!(reply.status, 200, "Origin 無しが通らない: {}", reply.body);

    let refused = [
        "http://evil.example".to_owned(),
        format!("http://evil.example:{port}"),
        "null".to_owned(),
        format!("http://127.0.0.1:{other}"),
        format!("http://127.0.0.1:{DEFAULT_PORT}"),
        format!("https://127.0.0.1:{port}"),
        format!("http://127.0.0.1:{port}/"),
        format!("http://localhost.:{port}"),
        format!("127.0.0.1:{port}"),
        // **スキームを見ていること**（`http://` と同じ長さの別の綴り / B-58 のレビュー R2）。
        format!("xxxx://127.0.0.1:{port}"),
        format!("https:/127.0.0.1:{port}"),
        String::new(),
    ];
    for value in &refused {
        let reply = raw(
            port,
            "GET",
            "/v1/models",
            &[host(&own), origin(value)],
            None,
        )
        .await;
        reply.assert_refused(&format!("Origin {value:?}"));
    }

    // **`Host` が無くても `Origin` は見る**（`Origin` だけが外の腕）。
    let reply = raw(
        port,
        "GET",
        "/v1/models",
        &[origin("http://evil.example")],
        None,
    )
    .await;
    reply.assert_refused("Host 無し・外の Origin");

    server.stop().await;
}

/// **画面（静的ファイル）にも効く。** 断った応答に画面の中身が載らない。
#[tokio::test]
async fn the_screen_is_guarded_as_well() {
    let server = TestServer::start().await;
    let port = server.port();
    let other = assert_ports_differ(port);
    let own = format!("127.0.0.1:{port}");
    let asset = format!("/{UI_ASSET_PATH}");

    for (path, marker) in [("/", UI_INDEX), (asset.as_str(), UI_ASSET)] {
        // **対照: 正しい `Host` なら中身が返る。**
        let reply = raw(port, "GET", path, &[host(&own)], None).await;
        assert_eq!(reply.status, 200, "{path}: {}", reply.body);
        assert!(reply.body.contains(marker), "{path}: 対照の中身が無い");

        for (label, headers) in [
            ("外の Host", vec![host(format!("evil.example:{port}"))]),
            ("別のポート", vec![host(format!("localhost:{other}"))]),
            (
                "外の Origin",
                vec![host(&own), origin("http://evil.example")],
            ),
        ] {
            let reply = raw(port, "GET", path, &headers, None).await;
            reply.assert_refused(&format!("{path} / {label}"));
            assert!(
                !reply.body.contains(marker),
                "{path} / {label}: 断った応答に画面の中身が載っている"
            );
        }
    }

    server.stop().await;
}

/// **要求の URI の authority も見る**（B-58 のレビュー P1）。HTTP/1.1 の**絶対形の要求行**で確かめる。
///
/// **`Host` は正しいまま**にして、authority だけを外にする ——`Host` しか見ない実装はここで通してしまう。
#[tokio::test]
async fn an_absolute_request_line_naming_another_host_is_refused() {
    let server = TestServer::start().await;
    let port = server.port();
    let other = assert_ports_differ(port);
    let own = format!("127.0.0.1:{port}");

    // **対照: 自分を指す絶対形は通る**（`Host` 有り・無しの両方）。
    for target in [
        format!("http://127.0.0.1:{port}/v1/models"),
        format!("http://localhost:{port}/v1/models"),
    ] {
        let reply = raw(port, "GET", &target, &[host(&own)], None).await;
        assert_eq!(reply.status, 200, "{target}: {}", reply.body);
        let reply = raw(port, "GET", &target, &[], None).await;
        assert_eq!(reply.status, 200, "Host 無し / {target}: {}", reply.body);
    }

    for target in [
        format!("http://evil.example:{port}/v1/models"),
        format!("http://127.0.0.1:{other}/v1/models"),
    ] {
        let reply = raw(port, "GET", &target, &[host(&own)], None).await;
        reply.assert_refused(&format!("{target}（Host は自分）"));
        let reply = raw(port, "GET", &target, &[], None).await;
        reply.assert_refused(&format!("{target}（Host 無し）"));
    }

    server.stop().await;
}

/// **HTTP/2（h2c）の `:authority` も見る**（B-58 のレビュー P1 / 待ち受けは h2c も受け付ける）。
///
/// クライアントに名前の向き先を `127.0.0.1` へ固定させ（`resolve`）、`:authority` に外の名前を載せる
/// ——DNS rebinding と同じ形を、HTTP/2 で作る。
#[tokio::test]
async fn an_http2_authority_naming_another_host_is_refused() {
    let server = TestServer::start().await;
    let port = server.port();
    let addr = server.addr;

    let client = reqwest::Client::builder()
        .http2_prior_knowledge()
        .resolve("evil.example", addr)
        .resolve("localhost", addr)
        .build()
        .expect("クライアントを組めること");

    // **対照: 自分の名前なら HTTP/2 でも通る**（HTTP/2 で話せていることもここで見る）。
    for url in [
        format!("http://127.0.0.1:{port}/v1/models"),
        format!("http://localhost:{port}/v1/models"),
    ] {
        let response = client
            .get(&url)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .expect("HTTP/2 で往復できること");
        assert_eq!(response.version(), reqwest::Version::HTTP_2, "{url}");
        assert_eq!(response.status().as_u16(), 200, "{url}");
    }

    let response = client
        .get(format!("http://evil.example:{port}/v1/models"))
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .expect("HTTP/2 で往復できること");
    assert_eq!(response.version(), reqwest::Version::HTTP_2);
    assert_eq!(response.status().as_u16(), 403);
    let versions: Vec<String> = response
        .headers()
        .get_all(VERSION_HEADER)
        .iter()
        .map(|value| value.to_str().unwrap_or("<not ascii>").to_owned())
        .collect();
    assert_eq!(versions, vec![API_VERSION.to_owned()]);
    let body: Value = response.json().await.expect("JSON であること");
    assert_eq!(body["error"]["code"], REFUSED_CODE, "{body}");

    server.stop().await;
}

/// **検査は本文の上限より外にある**——外の `Host` の要求は、**本文を1バイトも読まずに** 403 で断る
/// （B-58 のレビュー P2）。
///
/// 上限（2 MiB）を超える `Content-Length` を名乗り、**本文は送らない。**
/// 検査が上限の内側にあると、上限のミドルウェアが本文を待ち続けるので、応答が来ない（ここが時間切れで赤）。
///
/// **本文を実際に送る形にしない理由**: 送ると、サーバが読まずに閉じた接続は RST になり、
/// Windows では既に受け取った 403 まで捨てられうる（`server::body` の doc の表と同じ壊れ方）。
/// 正しい実装の側で揺れる見張り番は置かない。
#[tokio::test]
async fn a_foreign_host_is_refused_before_the_body_is_read() {
    /// 上限（axum の既定と同じ 2 MiB / `server::body::MAX_BODY_BYTES`）を超える大きさ。
    const DECLARED: usize = 3 * 1024 * 1024;
    /// 応答を待つ上限。**正しい実装ならすぐ返る**ので、短くてよい。
    const WAIT: Duration = Duration::from_secs(20);

    let server = TestServer::start().await;
    let port = server.port();

    let head = format!(
        "PUT /admin/upstream/profiles/evil HTTP/1.1\r\n\
         Host: evil.example:{port}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {DECLARED}\r\n\
         Connection: close\r\n\r\n"
    );
    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("繋がること");
    socket.write_all(head.as_bytes()).await.expect("書けること");
    let mut answer = Vec::new();
    tokio::time::timeout(WAIT, socket.read_to_end(&mut answer))
        .await
        .expect("本文を送る前に応答が来ること（検査が本文の上限の内側にあると、ここで待ち続ける）")
        .expect("読めること");
    let text = String::from_utf8(answer).expect("UTF-8");
    assert!(
        text.starts_with("HTTP/1.1 403"),
        "413 ではなく 403 であること: {text}"
    );
    assert!(text.contains(REFUSED_CODE), "{text}");

    server.stop().await;
}
