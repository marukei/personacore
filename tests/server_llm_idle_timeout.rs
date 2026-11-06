//! **打ち切りの天井のつまみ**（`PERSONACORE_LLM_IDLE_TIMEOUT`）が
//! **本番の入口（`App::bind`）から上流まで届いている**ことの検証
//! （作業単位 B-6 / [ADR-0035](../docs/adr/0035-the-watchdog-reports-the-human-decides.md) §決定3）。
//!
//! **実際の LM Studio には繋がない**（docs/05-development.md）。
//! 上流は**このファイルが自分で立てる生 TCP の偽 LM Studio**で、
//! **本文生成の要求には、返しても増分1件までで、そこから黙る。**
//!
//! # 天井に当たった回が下流にどう見えるかも、ここでしか見ていない
//!
//! B-6 で中継が打ち切りをやめたので、
//! **ストリーミングの途中で止まった上流を切るのは `llm` 層の天井だけ**になった。
//! **`LlmError::Timeout` が `upstream_timeout` に写ること**は
//! **`reqwest::Error` を作れないためユニットで表現できない**
//! （[`a_ceiling_hit_mid_stream_tells_the_client_it_was_a_timeout`]）。
//!
//! # なぜ本番の入口を通す必要があるのか
//!
//! **つまみは無料ではない。** 起動時に決めるものを増やすと、
//! **本番の入口を通るテストを足さない限り配線が観測対象から外れる**
//! ——`App::bind_with_client`（モック注入用）を足した結果、
//! **予算を丸ごと捨てる変異が2つとも緑を通った**という実例がある（CLAUDE.md 4節）。
//! **足すつまみ1つにつき、`App::bind` を通る見張り番を1本。** これがその1本。
//!
//! # ⚠ **既定で回してはいけない**（B-7 の穴）
//!
//! 期待値が既定（180秒）だと、**「読んだ値」と「定数」が文字どおり同じ式**になり、
//! **つまみを丸ごと無視する実装と区別が付かない。** `assert_ne!` にも掛からない。
//! **ここは既定と違う2つの天井で回す**——短い側（切れる）と長い側（切れない）。
//!
//! # 何を見ていないか
//!
//! **導出そのもの**（`request` / `context` / `connect` が天井から出ていること）は
//! `llm::lmstudio` の `one_knob_derives_every_other_deadline` と
//! `the_default_ceiling_reproduces_the_shipped_defaults` の担当。
//! **ここが見るのは「読んだ秒数が、上流を待つ長さとして実際に効く」ところだけ。**

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use personacore::domain::{DisplayName, Handle, Persona};
use personacore::llm::DEFAULT_ENDPOINT;
use personacore::server::{App, AppConfig, BindAddr, ENV_LLM_ENDPOINT, ENV_LLM_IDLE_TIMEOUT};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

mod common;

use common::fixture_scale;

/// **短い側の天井。** これを超えたら上流を待つのをやめる。
///
/// **既定（180秒）とは別の値**でなければならない（モジュールの doc）。
/// その前提は [`the_ceiling_setting_reaches_the_upstream_through_the_real_bind`] が
/// 既定の設定から実際に読んで確かめる。
const SHORT_CEILING: Duration = Duration::from_secs(2);

/// **長い側の天井。** 観測窓の間は切れない。
///
/// **既定とも短い側とも別の値。** 既定（180秒）にすると、
/// 「つまみを読まず既定を使う」実装がこちらの腕で緑になる。
const LONG_CEILING: Duration = Duration::from_secs(60);

/// **両方の腕で使う観測窓。** 短い側より長く、長い側より短いこと。
///
/// 縮めすぎると短い側が観測できず、伸ばしすぎると長い側が切れてしまう。
/// **その2つは下の前提で縛ってある。**
const OBSERVE: Duration = Duration::from_secs(8);

/// テストで使うキャラクターの識別子。**表示名とは別の値にすること。**
const HANDLE: &str = "aoi";

/// 表示名。**識別子と同値にしない**（どちらが載ったか区別できなくなる）。
const DISPLAY_NAME: &str = "葵";

/// 起動時の照会（`/api/v0/models`）への返事。**接続ごと閉じる**ので、
/// 本文生成は必ず別の接続で来る（観測対象が混ざらない）。
const NOT_FOUND: &[u8] =
    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// **1バイトも返さない偽 LM Studio。**
///
/// 起動時のコンテキスト長照会（`GET`）だけは 404 で断る——
/// あれは失敗しても既定値へ落ちるだけなので、これで `App::bind` は通る。
/// **本文生成（`POST`）の接続は掴んだまま黙る。**
struct SilentUpstream {
    url: String,
    _task: JoinHandle<()>,
}

impl SilentUpstream {
    /// **1バイトも返さない。** 応答ヘッダすら出さない。
    async fn start() -> Self {
        Self::start_with(None).await
    }

    /// `first` が `Some` なら、本文生成の要求に **SSE のヘッダと増分1件だけ**返して
    /// **そこから黙る**（＝中継が始まった後に上流が止まった形）。
    ///
    /// **`stream()` を成功させてから止めるのが要点。** 1バイトも返さないと
    /// `chat` が中継に入る前に失敗するので、**`relay` の `error_data` を通らない。**
    async fn start_with(first: Option<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("一時ポートを確保できること");
        let addr = listener.local_addr().expect("アドレスを取得できること");

        let task = tokio::spawn(async move {
            // **掴んだまま黙る**ので、握り続ける入れ物が要る（drop すると FIN が飛ぶ）。
            let mut held: Vec<TcpStream> = Vec::new();
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let line = read_request(&mut socket).await;
                // **本文生成以外の `POST`（封のための数える口 / 作業単位 B-60b）も断って閉じる。**
                // 黙って掴むと、数える往復が照会の上限まで待ってから本文が来るので、
                // **測っている待ち時間に、数える往復の上限が混ざる。**
                if line.starts_with("GET ") || !line.contains(" /v1/chat/completions ") {
                    // 起動時の照会。**断って閉じる。**
                    let _ = socket.write_all(NOT_FOUND).await;
                    let _ = socket.flush().await;
                    continue;
                }
                if let Some(first) = first.as_deref() {
                    let _ = socket.write_all(SSE_HEAD).await;
                    let _ = socket
                        .write_all(format!("{:X}\r\n{first}\r\n", first.len()).as_bytes())
                        .await;
                    let _ = socket.flush().await;
                }
                held.push(socket);
            }
        });

        Self {
            url: format!("http://{addr}"),
            _task: task,
        }
    }
}

/// SSE の応答ヘッダ（チャンク転送符号化）。
const SSE_HEAD: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                          Transfer-Encoding: chunked\r\n\r\n";

/// 上流が送る増分1件。**`llm::parse_chunk` が読める形。**
fn sse_delta(content: &str) -> String {
    format!(
        "data: {{\"id\":\"chatcmpl-upstream\",\"object\":\"chat.completion.chunk\",\
         \"created\":1760000000,\"model\":\"upstream-model\",\
         \"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{content}\"}},\
         \"finish_reason\":null}}]}}\n\n"
    )
}

/// 要求を**本文まで読み切って**先頭行を返す。
///
/// **ヘッダの終わりで止めないこと。** 残った本文が残っていると、
/// この後「黙っている」つもりの接続に読み残しが混ざる。
async fn read_request(socket: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut scratch = [0_u8; 4096];
    let head_end = loop {
        if let Some(at) = buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|at| at + 4)
        {
            break at;
        }
        let Ok(read) = socket.read(&mut scratch).await else {
            return String::new();
        };
        if read == 0 {
            return String::new();
        }
        buffer.extend_from_slice(&scratch[..read]);
    };

    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let length: usize = head
        .lines()
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())
                    .flatten()
            })
        })
        .unwrap_or(0);
    while buffer.len() < head_end + length {
        let Ok(read) = socket.read(&mut scratch).await else {
            break;
        };
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&scratch[..read]);
    }

    head.lines().next().unwrap_or_default().to_owned()
}

/// **本番の入口**（`App::bind`）で起動したサーバ。
struct TestServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), personacore::server::ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    /// 上流を `endpoint` に向け、天井を `ceiling` にして**本番の入口で**起動する。
    ///
    /// **`bind_with_client` を通らない**ことがこの経路の要点。
    async fn start(endpoint: &str, ceiling: Duration) -> Self {
        assert_ne!(
            endpoint.trim().trim_end_matches('/'),
            DEFAULT_ENDPOINT,
            "実機（{DEFAULT_ENDPOINT}）へ向けないこと（docs/05-development.md）"
        );

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let endpoint = endpoint.to_owned();
        let secs = ceiling.as_secs().to_string();
        // **プロセスの環境変数には触らない**（`from_lookup` に関数を渡す）。
        let config = AppConfig::from_lookup(|key| match key {
            ENV_LLM_ENDPOINT => Some(endpoint.clone()),
            ENV_LLM_IDLE_TIMEOUT => Some(secs.clone()),
            _ => None,
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(dir.path().join("personacore.sqlite"));

        // 前提: 設定の側で、渡した天井がそのまま載っていること。
        assert_eq!(
            config.llm().idle_timeout(),
            ceiling,
            "前提: 設定が天井を掴んでいること"
        );

        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        let addr = app.local_addr();

        assert_ne!(
            HANDLE, DISPLAY_NAME,
            "識別子と表示名は別の値にすること（同値だとどちらが載ったか区別できない）"
        );
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

/// **生の TCP で**非ストリーミングの要求を投げる。
async fn send_request(addr: SocketAddr) -> TcpStream {
    send(addr, false).await
}

/// **生の TCP で**ストリーミングの要求を投げる。
async fn send_streaming_request(addr: SocketAddr) -> TcpStream {
    send(addr, true).await
}

async fn send(addr: SocketAddr, stream: bool) -> TcpStream {
    let mut socket = TcpStream::connect(addr)
        .await
        .expect("サーバへ接続できること");
    let body = format!(
        "{{\"model\":\"{HANDLE}\",\"messages\":\
         [{{\"role\":\"user\",\"content\":\"ただいま、葵\"}}],\"stream\":{stream}}}"
    );
    // **`Host` は実際の待ち受け先**（作業単位 B-58 / `server::host_guard`）——
    // ポートの無い `localhost` はポート 80 と読まれて 403 になる。
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {addr}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    socket
        .write_all(request.as_bytes())
        .await
        .expect("要求を書けること");
    socket.flush().await.expect("送出できること");
    socket
}

/// [`OBSERVE`] の間に届いた応答。**届かなければ `None`。**
async fn response_within(socket: &mut TcpStream) -> Option<String> {
    let mut scratch = [0_u8; 8192];
    let read = tokio::time::timeout(OBSERVE, socket.read(&mut scratch))
        .await
        .ok()?
        .expect("応答を読めること");
    Some(String::from_utf8_lossy(&scratch[..read]).into_owned())
}

/// [`OBSERVE`] の間、`marker` が現れるまで読み続ける。**現れなければ `None`。**
///
/// **1回の読みで済ませない。** ストリーミングの応答は
/// 「ヘッダ → 増分 → …（天井まで待つ）… → error オブジェクト」と**別々に届く**ので、
/// 最初の読みだけを見ると増分しか見えない。
async fn read_until(socket: &mut TcpStream, marker: &str) -> Option<String> {
    let deadline = Instant::now() + OBSERVE;
    let mut received = Vec::new();
    let mut scratch = [0_u8; 8192];
    loop {
        let left = deadline.checked_duration_since(Instant::now())?;
        let read = tokio::time::timeout(left, socket.read(&mut scratch))
            .await
            .ok()?
            .expect("応答を読めること");
        if read == 0 {
            // サーバが閉じた。**そこまでに届いた分で判定する。**
            let body = String::from_utf8_lossy(&received).into_owned();
            return body.contains(marker).then_some(body);
        }
        received.extend_from_slice(&scratch[..read]);
        let body = String::from_utf8_lossy(&received);
        if body.contains(marker) {
            return Some(body.into_owned());
        }
    }
}

/// **天井のつまみが、本番の入口から上流を待つ長さとして効く。**
///
/// # 2つの腕を1本に置く理由
///
/// **安い側（短い天井で切れる）だけでは足りない。**
/// 「常に短い値で切る」実装もそれを満たしてしまう。
/// **高い側（長い天井では切れない）と一緒に置いて初めて、両方向の決め打ちが赤くなる**
/// （CLAUDE.md「対比は安い側だけでは足りない」）。
///
/// | 腕 | 天井 | [`OBSERVE`] の間に |
/// |---|---|---|
/// | 短い | [`SHORT_CEILING`] | **応答が届く**（上流を諦めた） |
/// | 長い | [`LONG_CEILING`] | **何も届かない**（まだ待っている） |
///
/// **どちらの天井も既定（180秒）ではない。**
#[tokio::test]
async fn the_ceiling_setting_reaches_the_upstream_through_the_real_bind() {
    // **前提: 観測窓が2つの天井の間にあること。**
    assert!(
        SHORT_CEILING < OBSERVE && OBSERVE < LONG_CEILING,
        "前提: 観測窓（{OBSERVE:?}）は短い天井（{SHORT_CEILING:?}）より長く、\
         長い天井（{LONG_CEILING:?}）より短いこと"
    );

    // **前提: どちらの天井も既定ではないこと**（B-7 の穴）。
    // 既定で回すと「読んだ値」と「定数」が文字どおり同じ式になり、
    // つまみを丸ごと無視する実装と区別できない。
    let shipped = AppConfig::from_lookup(|_| None)
        .expect("既定値だけで組み立てられること")
        .llm()
        .idle_timeout();
    assert_ne!(
        shipped, SHORT_CEILING,
        "前提: 短い側の天井が既定と同じでは、つまみが効いているか分からない"
    );
    assert_ne!(
        shipped, LONG_CEILING,
        "前提: 長い側の天井が既定と同じでは、つまみが効いているか分からない"
    );

    // ---- 腕1: 短い天井。**上流を諦めて応答が返る。** ----
    let upstream = SilentUpstream::start().await;
    let server = TestServer::start(&upstream.url, SHORT_CEILING).await;

    let started = Instant::now();
    let mut socket = send_request(server.addr).await;
    let answered = response_within(&mut socket).await;
    let waited = started.elapsed();
    drop(socket);
    server.stop().await;

    let answered = answered.unwrap_or_else(|| {
        panic!(
            "短い天井（{SHORT_CEILING:?}）を設定したのに、{OBSERVE:?} 以内に応答が返らない\
             ——つまみが上流まで届いていない"
        )
    });
    assert!(
        answered.starts_with("HTTP/1.1"),
        "応答の先頭が読めること: {answered:?}"
    );
    // **時間切れとして返っていること。** 接続できない・即座に失敗する形と区別する
    // （導出した接続の上限が 0 に潰れると `upstream_unavailable` になる）。
    assert!(
        answered.contains("upstream_timeout"),
        "上流を待ち切って諦めた形で返すこと: {answered:?}"
    );
    assert!(
        waited >= SHORT_CEILING,
        "天井まで待たずに返している（{waited:?} < {SHORT_CEILING:?}）\
         ——待つのをやめたのは天井のせいではない"
    );

    // ---- 腕2: 長い天井。**同じ窓では何も返らない。** ----
    let upstream = SilentUpstream::start().await;
    let server = TestServer::start(&upstream.url, LONG_CEILING).await;

    let mut socket = send_request(server.addr).await;
    let quiet = response_within(&mut socket).await;
    drop(socket);
    server.stop().await;

    assert!(
        quiet.is_none(),
        "長い天井（{LONG_CEILING:?}）を設定したのに {OBSERVE:?} で応答が返った\
         ——つまみを読まずに短い値で切っている: {quiet:?}"
    );
}

/// **天井に当たったストリーミングは、下流へ `upstream_timeout` を届ける**
/// （作業単位 B-6 / レビュー指摘 R-1）。
///
/// # なぜ統合テストなのか —— **ユニットでは作れない**
///
/// この経路の分かれ目は `relay::error_data(&err)` →
/// `chat::upstream_error(err)` の `match` で、
/// **`LlmError::Timeout` だけが `upstream_timeout` に写る。**
/// `LlmError::Timeout(#[source] reqwest::Error)` で、
/// **`reqwest::Error` は公開 API から作れない。**
/// ユニットで組み立てられる `Truncated` / `Status{429}` は**どれも `upstream_failed` に落ちる**ので、
/// 「別の符号に写ること」を `server::relay` のユニットでは表現できない。
///
/// **だから本物の `reqwest` に天井を踏ませる。**
///
/// # ⚠ B-6 で、ここが**新しく打ち切りの権威になった**
///
/// 中継の 30 秒が打ち切りをやめたので、
/// **ストリーミングの途中で止まった上流を切るのは `llm` 層の天井だけ**になった
/// （`relay.rs` のモジュール doc「天井に当たると `LlmError::Timeout` が返り、
/// 『上流が壊れた』枝へ合流する」）。**その1行を見る見張り番はここしか無い。**
///
/// **実害**: `upstream_failed` にすり替わると、
/// **時間切れだけを投げ直すクライアントが二度と投げ直さない。**
///
/// # 上流の台本
///
/// **増分を1件だけ返してから黙る。** 1バイトも返さないと `stream()` が
/// 中継に入る前に失敗し、**`error_data` を1度も通らない**（それでは何も測っていない）。
#[tokio::test]
async fn a_ceiling_hit_mid_stream_tells_the_client_it_was_a_timeout() {
    /// 上流が1件だけ流す本文。**下流にそのまま出るはず。**
    const FIRST: &str = "「おかえり";

    let upstream = SilentUpstream::start_with(Some(sse_delta(FIRST))).await;
    let server = TestServer::start(&upstream.url, SHORT_CEILING).await;

    let started = Instant::now();
    let mut socket = send_streaming_request(server.addr).await;

    // **`data:` が2件届くまで読む。** 1件目が上流の増分、2件目が終わり方。
    let body = read_until(&mut socket, "\"code\"").await;
    let waited = started.elapsed();
    drop(socket);
    server.stop().await;

    let body = body.unwrap_or_else(|| {
        panic!(
            "天井（{SHORT_CEILING:?}）に当たったのに、{OBSERVE:?} 以内に\
             終わり方を告げるイベントが届かない"
        )
    });

    // 前提: 中継が現に始まっていること（＝ `error_data` を通る枝に居ること）。
    assert!(
        body.contains("text/event-stream"),
        "前提: SSE として応答が始まっていること: {body:?}"
    );
    assert!(
        body.contains(FIRST),
        "前提: 天井に当たる前に届いた増分は捨てないこと: {body:?}"
    );
    // 前提: 待ったのは天井のぶんであること（接続の失敗と区別する）。
    assert!(
        waited >= SHORT_CEILING,
        "前提: 天井まで待ってから終わること（{waited:?} < {SHORT_CEILING:?}）"
    );

    // **本題。** 時間切れは時間切れとして届く。
    assert!(
        body.contains("upstream_timeout"),
        "天井に当たった回は `upstream_timeout` として届けること\
         （`upstream_failed` にすり替わると、時間切れだけを投げ直す\
         クライアントが二度と投げ直さない）: {body:?}"
    );
    // **完結していないので `[DONE]` は送らない**（ADR-0019 §5）。
    assert!(
        !body.contains("[DONE]"),
        "天井に当たった回で [DONE] を流さないこと: {body:?}"
    );
}

/// **不正な天井では起動しない**（黙って既定へ落とさない / B-6）。
///
/// **`0` も含む**——書式としては妥当だが、**導出した接続の上限まで 0 に潰れて、
/// 上流への接続を1度も試みなくなる。**
/// **線は「使える値かどうか」ではなく「即座に失敗する／待って失敗する」の境目にある**
/// （`1` は遅い機では役に立たないが、待ってから失敗するので通す）。
///
/// 綴りごとの網羅は `server::config` のユニット
/// （`an_unusable_llm_idle_timeout_stops_the_startup`）が見る。
/// **ここが見ているのは「本番の入口の手前で止まる」ことと、
/// エラーが値を反射しないこと。**
#[tokio::test]
async fn an_unusable_ceiling_never_reaches_the_real_bind() {
    for raw in ["0", "とても長く"] {
        let err =
            AppConfig::from_lookup(|key| (key == ENV_LLM_IDLE_TIMEOUT).then(|| raw.to_owned()))
                .expect_err("不正な天井で起動を止めること");

        let message = err.to_string();
        assert!(
            !message.contains(raw),
            "指定された値が Display に漏れている（{raw:?}）: {message}"
        );
    }

    // **対照: 正当な値なら、ここは通る。**
    // これが無いと「何を渡しても失敗する」実装が上を満たして緑になる。
    let usable = LONG_CEILING.as_secs().to_string();
    let built = AppConfig::from_lookup(|key| (key == ENV_LLM_IDLE_TIMEOUT).then(|| usable.clone()))
        .expect("正当な天井なら組み立てられること");
    assert_eq!(
        built.llm().idle_timeout(),
        LONG_CEILING,
        "対照が、渡した天井をそのまま掴んでいること"
    );
}
