//! **切断時キャンセル**（作業単位 11c-3b）の検証。
//!
//! **実際の LM Studio には繋がない**（docs/05-development.md）。上流は `LlmClient` の
//! モックか、**このファイルが自分で立てる生 TCP の偽 LM Studio**。
//!
//! # 何を固定しているか
//!
//! docs/02-architecture.md は「FE 側が切断したら上流へのリクエストもキャンセルする
//! （無駄な推論を続けない）」と決めている。その実現は**2段の連鎖**で、
//! 本番コードには「切断」を扱う分岐が1つも無い。
//!
//! | 段 | 主張 | 見張り番 |
//! |---|---|---|
//! | 下流 | **FE が切ると axum が SSE の本文を drop する**（＝ `ChatStream` が落ちる） | 前半（モック注入） |
//! | 上流 | `ChatStream` を drop すると上流への TCP が閉じる | `tests/llm_lmstudio.rs` |
//! | **両端** | **本番の入口から偽 LM Studio まで、切断がそのまま届く** | **後半**（`App::bind`） |
//!
//! **分岐が無いので、壊れても「何かを消した」形にはならない。** 中継が
//! `ChatStream` をどこかに抱え込む実装（別タスクへ move して `spawn` する、
//! チャンネル越しに読み溜める等）へ変わった瞬間、切断しても上流は生成を続ける。
//! **その状態でも下流に見える SSE は変わらない。**
//! 実測した内訳は [`disconnecting_while_the_upstream_is_streaming_drops_it`] の
//! 「実測」節にある（**このファイルの外では `relay` の無通信打ち切りだけが巻き添えで赤くなり、
//! 他の統合テストは全件緑**）。
//!
//! # 前半だけでは**応答が1バイトも出る前の窓**を見ていない（後半の存在理由）
//!
//! 前半（モック注入）が観測を始めるのは**中継が始まった後**——
//! `open_and_read_one_event` が最初の `data:` を読んでから切るので、その時点で
//! `ChatStream` は既に `RelayState` の中にいる。しかも `MockLlm::stream` は
//! 即座に解決するので、**`chat` が `llm.stream(&upstream).await`
//! （`llm` は `AppState::llm_for_turn` で取ったそのターンの写し / 作業単位 B-60b）で
//! 待っている時間帯そのものが再現されない。**
//!
//! **そこがいちばん無駄が大きい時間帯。** LM Studio の JIT ロードは数十秒かかる。
//! FE が「送信 → 気が変わって即キャンセル」を繰り返すと、キャンセルを上流へ
//! 伝えない実装では**そのたびにモデルをロードして、誰も読まない応答を最後まで作り続ける。**
//! GPU を専有する構成なので、後続の正規のリクエストが待たされる。
//!
//! **`tokio::spawn` を1つ足すだけで壊れる。** 上流のオープンを切り離したタスクへ出す
//! 変異（レビュー指摘 C1）は、**前半の3本を含む全件を素通りした**（実測）。
//! 後半の [`disconnecting_before_the_first_byte_closes_the_upstream`] が赤にする。
//!
//! # ここで**やらない**こと
//!
//! * **保存の意味論**（切断した回を保存しない）は既存の振る舞いで、11b-2 の担当。
//! * `ChatStream` の drop が上流の TCP を閉じること**単体**は
//!   `tests/llm_lmstudio.rs` の担当（上表）。後半はそれを含む経路全体を通す。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt as _;
use futures_util::stream;
use personacore::domain::{DisplayName, Handle, Persona};
use personacore::llm::{
    ChatChunk, ChatRequest, ChatStream, Completion, DEFAULT_ENDPOINT, FinishReason, LlmClient,
    LlmError, LlmFuture,
};
use personacore::server::{App, AppConfig, BindAddr, ENV_LLM_ENDPOINT, ServerError};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

mod common;

use common::fixture_scale;

/// drop を待つ上限。**実測（15〜23ms）に対して桁で余裕を取る。**
///
/// drop されれば即座に返るので、**緑の側ではこの時間を待たない。**
///
/// ~~**中継の無通信予算（30秒 / 11c-3a）より必ず短くすること。**~~
/// ~~長くすると、切断をまったく検出できていない実装でも**打ち切りが上流を落とす**ため、~~
/// ~~その drop を「切断で落ちた」と読み違えて緑になる。~~
///
/// > **⚠ この縛りは B-6 で消えた**（[ADR-0035](../docs/adr/0035-the-watchdog-reports-the-human-decides.md)）。
/// > **中継は無通信では上流を落とさなくなった**ので、
/// > **どれだけ長く待っても「打ち切りの drop」と混ざる余地が無い。**
/// > **落とすのは切断か、`llm` 層の天井（既定 180 秒）に当たった回だけ。**
/// > **それでも短いままにしてある**——長くする理由が無く、
/// > **天井（180秒）を跨がない限り、混ざらない性質は保たれる。**
///
/// **要求の応答を待つ上限（[`RESPONSE_LIMIT`]）と兼ねないこと。** 兼ねると、
/// ここを短くしたときに「応答が届くのを待つ時間」まで黙って厳しくなる。
const DROP_LIMIT: Duration = Duration::from_secs(10);

/// **要求の応答が届くのを待つ読み取り上限。** 詰まったときにテストを吊るさないための値。
///
/// [`DROP_LIMIT`] と違い、ここで測っているのは
/// 「上流を落としたか」ではなく「サーバが応答を始めたか」。負荷時に落ちないよう長く取る。
const RESPONSE_LIMIT: Duration = Duration::from_secs(30);

/// **`FakeUpstream::observe` の待ちの上限**（Issue origin#34）。
///
/// 上流へ依頼が届かない変異（例: キャラクターを読む `SELECT` から列を1つ落とす）の下では、
/// `FakeUpstream::start` の `accept().await` が永遠に空振りし、観測結果を送る側が
/// 一度も送信しない。ここに上限が無いと `observe` の `recv().await` が無期限に待ち続け、
/// テストバイナリが終わらない（2026-09-26 に 10 分以上・46 分の2回、実測）。
///
/// [`RESPONSE_LIMIT`] と**値は揃えるが定数は分ける**。ここで測っているのは
/// 「上流へ要求が届いたか」で、[`RESPONSE_LIMIT`] の「サーバが応答を始めたか」とは別の関心事
/// （[`DROP_LIMIT`] を [`RESPONSE_LIMIT`] と兼ねない理由と同じ）。
const OBSERVE_LIMIT: Duration = Duration::from_secs(30);

/// **落ちていないことを確かめる観測窓。**
///
/// **短くしすぎると意味を失う**ので、使う側が「同じ長さの窓で、切断すれば落ちるのが
/// 見える」ことを併せて主張する（[`staying_connected_keeps_the_upstream_alive`]）。
/// **この値を 0 にすると、その第2相が観測できなくなって赤くなる。**
const ALIVE_WINDOW: Duration = Duration::from_millis(500);

/// 印を見に行く間隔。
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// テストで使うキャラクターの識別子。**表示名とは別の値にすること。**
const HANDLE: &str = "aoi";

/// 表示名。**識別子と同値にしない**（どちらが載ったか区別できなくなる）。
const DISPLAY_NAME: &str = "葵";

/// **drop されたら記録する印。** `ChatStream` に相乗りさせる。
struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// ストリームに「drop されたら記録する」印を相乗りさせる。
///
/// **印は `unfold` の状態に入る**ので、ストリーム本体が落ちたときにだけ落ちる。
/// 中継が中身を別の器へ移し替えても、**その器が生きている限り印は立たない。**
fn watched(inner: ChatStream, flag: &Arc<AtomicBool>) -> ChatStream {
    let guard = DropFlag(Arc::clone(flag));
    Box::pin(stream::unfold(
        (inner, guard),
        |(mut inner, guard)| async move { inner.next().await.map(|item| (item, (inner, guard))) },
    ))
}

fn chunk(delta: &str) -> Result<ChatChunk, LlmError> {
    Ok(ChatChunk {
        delta: delta.to_owned(),
        finish_reason: None,
        usage: None,
    })
}

/// **1件だけ流して、その後は永久に黙る**上流（接続は開いたまま）。
///
/// LM Studio が長考している最中に FE が切った状況。**下流への書き込みが無いので、
/// 切断は「書けなくなった」ではなく「相手が閉じた」側からしか気づけない。**
fn silent_after_one(flag: &Arc<AtomicBool>) -> ChatStream {
    watched(
        Box::pin(stream::iter(vec![chunk("「おかえり")]).chain(stream::pending())),
        flag,
    )
}

/// **止まらずに流し続ける**上流（生成の真っ最中に切られた状況）。
fn endless(flag: &Arc<AtomicBool>) -> ChatStream {
    watched(
        Box::pin(stream::unfold(0_u32, |index| async move {
            tokio::time::sleep(POLL_INTERVAL).await;
            Some((chunk(&format!("片{index}")), index + 1))
        })),
        flag,
    )
}

/// 上流（LM Studio）の代役。**ネットワークには一切触らない。**
struct MockLlm {
    /// `stream` が返すストリーム。**1本だけ。**
    source: Mutex<Option<ChatStream>>,
}

impl LlmClient for MockLlm {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        Box::pin(async move {
            Ok(Completion {
                content: "非ストリーミングの本文".to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        Box::pin(async move {
            self.source
                .lock()
                .expect("ロックを取れること")
                .take()
                .ok_or(LlmError::MissingContent)
        })
    }
}

/// エフェメラルポートで起動した実サーバ。
struct TestServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    /// **モックを注入して**起動する（`bind_with_client`）。
    ///
    /// ここが実際の LM Studio に繋がらないことの根拠。
    async fn with_mock(source: ChatStream) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let config = Self::config(&dir, None);
        let llm = Arc::new(MockLlm {
            source: Mutex::new(Some(source)),
        });
        let app = App::bind_with_client(&config, llm)
            .await
            .expect("待ち受けを開始できること");
        Self::serve(app, dir).await
    }

    /// **本番の入口**（`App::bind`）で起動し、上流を `endpoint` へ向ける。
    ///
    /// **`bind_with_client` を通らない**ことがこの経路の要点（CLAUDE.md 4節
    /// 「テスト専用の入口を作ると、本番の入口が観測対象から外れる」）。
    /// 上流は呼び出し側が立てた偽 LM Studio なので、実機には繋がらない。
    async fn bound_to(endpoint: &str) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let config = Self::config(&dir, Some(endpoint));
        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        Self::serve(app, dir).await
    }

    /// 設定を組み立てる。**プロセスの環境変数には触らない**（`from_lookup` に関数を渡す）。
    fn config(dir: &TempDir, endpoint: Option<&str>) -> AppConfig {
        if let Some(url) = endpoint {
            assert_ne!(
                url.trim().trim_end_matches('/'),
                DEFAULT_ENDPOINT,
                "実機（{DEFAULT_ENDPOINT}）へ向けないこと（docs/05-development.md）"
            );
        }
        let endpoint = endpoint.map(str::to_owned);
        AppConfig::from_lookup(|key| {
            if key == ENV_LLM_ENDPOINT {
                endpoint.clone()
            } else {
                None
            }
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(dir.path().join("personacore.sqlite"))
    }

    /// キャラクターを1体作り、停止シグナルつきで走らせる。
    async fn serve(app: App, dir: TempDir) -> Self {
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

/// **生の TCP で**ストリーミング要求を投げる（**応答は読まない**）。
///
/// `reqwest` では「途中で切る」を確実には表現できない（本文を捨てても
/// 接続がプールへ戻るだけのことがある）ので、こちらは自分でソケットを握る。
///
/// 戻り値の `TcpStream` を drop することが、このファイルで言う「FE の切断」。
async fn send_request(addr: SocketAddr) -> TcpStream {
    let mut socket = TcpStream::connect(addr)
        .await
        .expect("サーバへ接続できること");
    let body = format!(
        "{{\"model\":\"{HANDLE}\",\"messages\":\
         [{{\"role\":\"user\",\"content\":\"ただいま、葵\"}}],\"stream\":true}}"
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

/// ストリーミング要求を投げ、**1件目の `data:` が届くまで読む。**
async fn open_and_read_one_event(addr: SocketAddr) -> TcpStream {
    let mut socket = send_request(addr).await;

    let mut received = Vec::new();
    let mut scratch = [0_u8; 4096];
    loop {
        let read = tokio::time::timeout(RESPONSE_LIMIT, socket.read(&mut scratch))
            .await
            .expect("応答が時間内に届くこと")
            .expect("応答を読めること");
        assert_ne!(read, 0, "サーバ側から先に閉じないこと");
        received.extend_from_slice(&scratch[..read]);
        if String::from_utf8_lossy(&received).contains("data:") {
            return socket;
        }
    }
}

/// 印が立つまで待つ（実時間・上限つき）。**立たなければ `None`。**
///
/// 仮想時計（`tokio::time::pause`）は使わない。**この経路は DB を触る**ので、
/// 自動前進を開けると sqlx のプール取得が `PoolTimedOut` で落ちる
/// （docs/05-development.md）。待つのは実測 15〜23ms の事象なので実時間で足りる。
async fn wait_for_drop(flag: &AtomicBool, limit: Duration) -> Option<Duration> {
    let started = Instant::now();
    loop {
        if flag.load(Ordering::SeqCst) {
            return Some(started.elapsed());
        }
        if started.elapsed() >= limit {
            return None;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// **生成の真っ最中に FE が切ったら、上流のストリームを落とす。**
///
/// 落とさなければ、上流は**誰も読まない応答を生成し続ける**
/// （docs/02-architecture.md「無駄な推論を続けない」）。
///
/// # 実測: **この向きだけを捕まえる失敗モードは、まだ見つかっていない**
///
/// `ChatStream` を先読みタスクへ move する変異（`relay_response` でチャンネルへ
/// 読み溜める形 / M2）を当てた実測（2026-08-20）:
///
/// | バイナリ | 結果 |
/// |---|---|
/// | lib（`relay` のユニット） | 553 passed / **6 failed**（すべて 11c-3a の無通信打ち切り。先読みタスクが沈黙を吸うので打ち切りが起きない） |
/// | | > **⚠ この行は B-6 の前の数**（ADR-0035 で打ち切りは撤回された）。**同じ内訳にはもうならない** |
/// | このファイル | 2 passed / **3 failed** |
/// | `tests/server_chat_stream.rs` | 15 passed / 0 failed |
/// | `tests/llm_lmstudio.rs` | 27 passed / 0 failed |
///
/// **このファイルで赤くなった3本に、この検証は入っていない**——
/// [`disconnecting_while_the_upstream_is_silent_drops_it_too`]、
/// [`disconnecting_after_the_first_byte_closes_the_upstream`]、
/// [`staying_connected_keeps_the_upstream_alive`]（第2相）で、
/// **こちらと [`disconnecting_before_the_first_byte_closes_the_upstream`] は緑を通り抜けた。**
///
/// 理由も分かっている。上流が喋り続けていると先読みタスクは `send().await` で止まるので、
/// **受信側が落ちた瞬間に `Err` を受けて `break` し、結果として上流を落とす。**
/// 黙っている場合は `upstream.next()` で止まっているので気づけない。
///
/// **それでも残してある。** 「見つかっていない」は「無い」ではない
/// （**喋っている最中だけ検出できない**実装を、まだ誰も探し切っていない）。
/// **冗長かもしれない1本**として読むこと。
#[tokio::test]
async fn disconnecting_while_the_upstream_is_streaming_drops_it() {
    let flag = Arc::new(AtomicBool::new(false));
    let server = TestServer::with_mock(endless(&flag)).await;

    let socket = open_and_read_one_event(server.addr).await;
    assert!(
        !flag.load(Ordering::SeqCst),
        "前提: 中継の最中はまだ drop されていないこと"
    );

    // **ここが「FE の切断」。**
    drop(socket);

    let dropped = wait_for_drop(&flag, DROP_LIMIT).await;
    server.stop().await;

    assert!(
        dropped.is_some(),
        "FE が切断したら上流のストリームを落とすこと（{DROP_LIMIT:?} 以内）"
    );
}

/// **上流が黙っている最中に切られても落とす。**
///
/// 下流への書き込みが無い状態なので、**「書けなくなった」では気づけない。**
/// 上の検証だけだと、書き込みの失敗にしか反応しない実装が緑を通り抜ける。
///
/// **抱え込む実装を捕まえているのはこちら**
/// （[`disconnecting_while_the_upstream_is_streaming_drops_it`] の「実測」節）。
#[tokio::test]
async fn disconnecting_while_the_upstream_is_silent_drops_it_too() {
    let flag = Arc::new(AtomicBool::new(false));
    let server = TestServer::with_mock(silent_after_one(&flag)).await;

    let socket = open_and_read_one_event(server.addr).await;
    assert!(
        !flag.load(Ordering::SeqCst),
        "前提: 中継の最中はまだ drop されていないこと"
    );

    drop(socket);

    let dropped = wait_for_drop(&flag, DROP_LIMIT).await;
    server.stop().await;

    assert!(
        dropped.is_some(),
        "上流が黙っていても、切断で上流のストリームを落とすこと（{DROP_LIMIT:?} 以内）"
    );
}

/// **対照**: 繋がっている間は落とさない。
///
/// これが無いと、上の2つは**印が最初から立っているだけ**でも緑になる。
/// 同時に「読むのが遅いだけの FE を勝手に切らない」ことの確認でもある。
///
/// # 観測窓を第2相で縛る
///
/// 「[`ALIVE_WINDOW`] の間は落ちなかった」だけなら、**窓を 0ms にしても緑のまま**
/// （実測 / レビュー指摘）。そこで**同じ長さの窓で切断も観測する。**
/// 窓が短すぎれば第2相が観測できず、**この対照そのものが赤くなる。**
#[tokio::test]
async fn staying_connected_keeps_the_upstream_alive() {
    let flag = Arc::new(AtomicBool::new(false));
    let server = TestServer::with_mock(silent_after_one(&flag)).await;

    // 第1相: **落とさずに保持したまま**観測窓が尽きるのを待つ。
    let socket = open_and_read_one_event(server.addr).await;
    let while_held = wait_for_drop(&flag, ALIVE_WINDOW).await;

    // 第2相: **同じ長さの窓**で切断を観測する。ここが窓の長さを縛っている。
    drop(socket);
    let after_drop = wait_for_drop(&flag, ALIVE_WINDOW).await;

    server.stop().await;

    assert!(
        while_held.is_none(),
        "繋がっている間は上流のストリームを落とさないこと: {while_held:?}"
    );
    assert!(
        after_drop.is_some(),
        "第2相で切断を観測できなかった。**観測窓（{ALIVE_WINDOW:?}）が短すぎる**か、\
         **切断が上流へ伝わっていない**かのどちらか。\
         いずれにせよ第1相の `None` は「落ちない」ではなく「何も見ていない」を意味する"
    );
}

// ------------------------------------- 端から端まで（`App::bind` → 偽 LM Studio）
//
// ここから下は**モックを注入しない。** FE も上流も生の TCP で、間に立つのは
// **本番の入口（`App::bind`）で起動した personacore そのもの。**
//
// | 見るもの | 前半（モック注入） | 後半（ここ） |
// |---|---|---|
// | 上流オープン待ちの窓 | **見ない**（即座に解決する） | **見る** |
// | `App::bind` の配線 | 通らない（`bind_with_client`） | **通る** |
// | 上流が閉じたか | `ChatStream` の drop で代用 | **TCP を上流側から観測** |
//
// **偽 LM Studio は `/api/v0/models` に 404 を返す。** 起動時のコンテキスト長照会
// （ADR-0013 第2層）は失敗しても既定値へ落ちるだけなので、これで起動できる。

/// 上流側から見た接続の様子。
///
/// 中身（原因・バイト数）は**失敗したときの手掛かり**としてしか読まない。
/// 判定に使うのはどのバリアントかだけなので、`Debug` 経由の読みしか無い。
#[derive(Debug)]
#[expect(dead_code, reason = "中身は失敗時の `Debug` 出力としてだけ読む")]
enum Liveness {
    /// 読みが 0 バイト（FIN）＝ **personacore が閉じた。**
    Closed,
    /// 読みが失敗した（RST 等）。**これも「閉じた」側。**
    Reset(String),
    /// 観測窓の間なにも起きなかった ＝ **接続は生きている。**
    Open,
    /// 予期しない追加データ。要求は本文まで読み切っているので本来は来ない。
    Unexpected(usize),
}

/// 偽 LM Studio の観測窓。**両相で同じ長さを使う**ので、縮めると
/// 第2相（切断の観測）が成立せずに赤くなる。
const UPSTREAM_WINDOW: Duration = Duration::from_millis(500);

/// SSE の応答ヘッダ（チャンク転送符号化）。
const SSE_HEAD: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                          Transfer-Encoding: chunked\r\n\r\n";

/// 起動時の照会（`/api/v0/models`）への返事。**接続ごと閉じる**ので、
/// 本文生成は必ず別の接続で来る（観測対象が混ざらない）。
const NOT_FOUND: &[u8] =
    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// 上流が送る増分1件。
fn sse_delta(content: &str) -> String {
    format!(
        "data: {{\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1,\
         \"model\":\"m\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{content}\"}},\
         \"finish_reason\":null}}]}}\n\n"
    )
}

/// 要求を**本文まで読み切って**先頭行を返す。
///
/// **ヘッダの終わりで止めないこと。** 残った本文は、この後の「接続の生死」の
/// 観測に `Unexpected` として現れる（＝観測が壊れる）。
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
        let read = socket.read(&mut scratch).await.expect("要求を読めること");
        assert_ne!(read, 0, "ヘッダの途中で閉じないこと");
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
        let read = socket.read(&mut scratch).await.expect("本文を読めること");
        assert_ne!(read, 0, "本文の途中で閉じないこと");
        buffer.extend_from_slice(&scratch[..read]);
    }

    head.lines().next().unwrap_or_default().to_owned()
}

/// 生 TCP の偽 LM Studio。**personacore の上流として `App::bind` から向けられる。**
struct FakeUpstream {
    url: String,
    liveness: mpsc::Receiver<Liveness>,
}

impl FakeUpstream {
    /// 立ち上げる。`first` が `Some` なら本文生成の要求に**その1件だけ**返し、
    /// `None` なら**1バイトも返さない**（JIT ロード中の窓の再現）。
    ///
    /// 本文生成の接続を掴んだら、[`UPSTREAM_WINDOW`] の窓で**2回**生死を観測して報告する。
    /// 1回目は「繋がっている間は生きている」の対照、2回目が「切断で閉じる」の本題。
    async fn start(first: Option<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("一時ポートを確保できること");
        let addr = listener.local_addr().expect("アドレスを取得できること");
        let (report, liveness) = mpsc::channel(2);

        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.expect("接続を受け付けられること");
                let line = read_request(&mut socket).await;

                // 起動時のコンテキスト長照会。**断って閉じる。**
                //
                // **本文生成以外の `POST` も断って閉じる**（作業単位 B-60b）。本文の前に
                // **封のための数える口**（`/apply-template`）が来るようになったので、
                // 断らないとそちらの接続を本文の接続と取り違えて観測する
                // ——**見ているのが本文の接続であること**がこのファイルの主張の前提である
                // （実測: 断らない形では「1バイト目の後」の腕が、数える往復に SSE を返して時間切れで赤。
                // 「1バイトも出る前」の腕は**緑のまま**だった —— コードを読む限り、観測していたのは
                // 数える往復の接続で、**意味が変わったまま緑だった**ことになる）。
                if line.starts_with("GET ") || !line.contains(" /v1/chat/completions ") {
                    socket.write_all(NOT_FOUND).await.expect("断れること");
                    socket.flush().await.expect("送出できること");
                    continue;
                }

                if let Some(first) = first.as_deref() {
                    socket.write_all(SSE_HEAD).await.expect("頭を書けること");
                    socket
                        .write_all(format!("{:X}\r\n{first}\r\n", first.len()).as_bytes())
                        .await
                        .expect("本文を書けること");
                    socket.flush().await.expect("送出できること");
                }

                let mut scratch = [0_u8; 1024];
                for _ in 0..2 {
                    let started = Instant::now();
                    let result =
                        tokio::time::timeout(UPSTREAM_WINDOW, socket.read(&mut scratch)).await;
                    // **窓の外で起きたことは「観測できなかった」に倒す。**
                    // `tokio::time::timeout` は**内側の future を先にポーリングする**ので、
                    // 観測を始める前に届いていた FIN は**窓が 0 でも `Closed` として通る。**
                    // それでは窓が短すぎることを検出できない（実測 / [`UPSTREAM_WINDOW`]）。
                    let liveness = if started.elapsed() > UPSTREAM_WINDOW {
                        Liveness::Open
                    } else {
                        match result {
                            Ok(Ok(0)) => Liveness::Closed,
                            Ok(Ok(read)) => Liveness::Unexpected(read),
                            Ok(Err(err)) => Liveness::Reset(err.to_string()),
                            Err(_elapsed) => Liveness::Open,
                        }
                    };
                    if report.send(liveness).await.is_err() {
                        break;
                    }
                }

                // **観測を送り終えてから閉じる**（先に落とすと、こちら発の FIN と混ざる）。
                drop(socket);
                return;
            }
        });

        Self {
            url: format!("http://{addr}"),
            liveness,
        }
    }

    /// 次の観測結果を受け取る。
    ///
    /// **上限つき**（[`OBSERVE_LIMIT`]）。上流へ依頼が届かない変異の下では
    /// 観測結果が一度も送られてこないため、上限が無いとテストバイナリが終わらない
    /// （[`OBSERVE_LIMIT`] の doc / Issue origin#34）。
    async fn observe(&mut self) -> Liveness {
        tokio::time::timeout(OBSERVE_LIMIT, self.liveness.recv())
            .await
            .unwrap_or_else(|_| panic!("上流へ依頼が届かなかった（{OBSERVE_LIMIT:?} 待った）"))
            .expect("観測結果を受け取れること")
    }
}

/// **応答が1バイトも出る前に切られたら、上流への要求も落とす**（レビュー指摘 C1）。
///
/// LM Studio が JIT ロードしている数十秒がこの窓。ここでキャンセルが伝わらないと、
/// **FE が投げ捨てた要求のたびにモデルがロードされ、誰も読まない応答が作られる。**
/// GPU を専有する構成なので、後続の正規のリクエストがそのぶん待たされる。
///
/// **前半（モック注入）はこの窓を再現できない**（モジュールの doc）。
/// `chat` の `llm.stream(&upstream).await`（そのターンの写し / `AppState::llm_for_turn`）を
/// 切り離したタスクへ出す変異は、
/// **この1本が無いと全件緑で通る。**
#[tokio::test]
async fn disconnecting_before_the_first_byte_closes_the_upstream() {
    // **上流は1バイトも返さない。** personacore は応答ヘッダを待ち続ける。
    let mut upstream = FakeUpstream::start(None).await;
    let server = TestServer::bound_to(&upstream.url).await;

    let socket = send_request(server.addr).await;

    // 第1相: 要求が上流まで届き、**繋いでいる間は保たれている**こと。
    let while_held = upstream.observe().await;
    assert!(
        matches!(while_held, Liveness::Open),
        "前提: 要求が上流へ届き、繋いでいる間は保たれていること: {while_held:?}"
    );
    // **窓の分だけ実時間が経った後**なので、届くはずのものはもう届いている。
    let mut scratch = [0_u8; 1024];
    assert!(
        matches!(
            socket.try_read(&mut scratch),
            Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock
        ),
        "前提: FE にはまだ1バイトも出ていないこと（この窓を測っている）"
    );

    // **ここが「FE の切断」。**
    drop(socket);

    let after_drop = upstream.observe().await;
    server.stop().await;

    assert!(
        matches!(after_drop, Liveness::Closed | Liveness::Reset(_)),
        "応答を1バイトも出す前に切られたら、上流への要求も落とすこと: {after_drop:?}"
    );
}

/// **中継が始まった後に切られても、上流への TCP が閉じる**（端から端まで）。
///
/// 前半（モック注入）が見ているのと同じ失敗モードを、**本番の入口と実物の
/// `LmStudioClient` を通して**もう一度見る。前半は `ChatStream` の drop で代用して
/// いるので、**「drop したのに TCP が閉じない」形は前半では観測できない。**
#[tokio::test]
async fn disconnecting_after_the_first_byte_closes_the_upstream() {
    let mut upstream = FakeUpstream::start(Some(sse_delta("「おかえり"))).await;
    let server = TestServer::bound_to(&upstream.url).await;

    // **1件目が下流へ出るまで読む**（＝中継が始まった状態）。
    let socket = open_and_read_one_event(server.addr).await;

    let while_held = upstream.observe().await;
    assert!(
        matches!(while_held, Liveness::Open),
        "前提: 中継の最中は上流への接続を保つこと: {while_held:?}"
    );

    drop(socket);

    let after_drop = upstream.observe().await;
    server.stop().await;

    assert!(
        matches!(after_drop, Liveness::Closed | Liveness::Reset(_)),
        "FE が切断したら上流への TCP を閉じること: {after_drop:?}"
    );
}
