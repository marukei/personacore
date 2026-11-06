//! **UI（Svelte のビルド成果物）の配布**（ADR-0031 / 作業単位 D-0）の検証。
//! **LM Studio には繋がない。**
//!
//! ここで固定するのは3点:
//!
//!   * **UI のフォールバックが `/v1/*` と `/admin/*` の 404 を飲み込まないこと**
//!     （ADR-0031 §4）。**この1本が D-0 でいちばん壊しやすい点。**
//!   * 配り先が**設定から来ている**こと（既定値やどこかの固定パスではない）。
//!   * **配り先が無くても API は生きている**こと（`pnpm build` を忘れた状態）。

use std::net::SocketAddr;
use std::path::Path;

use personacore::server::{App, AppConfig, BindAddr, ENV_WEB_DIR, ServerError};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// UI の配り先に置く目印。**どれも互いに違う文字列**——
/// 同じにすると「どのファイルが返ったか」を区別できない。
///
/// **`v1/…` と `admin/…` にも実体を置く**のがこのテストの肝。
/// 置かないと「API のエラーが返った」と「UI 側にも何も無かった」が
/// **どちらも 404** で見分けられず、フォールバックの順序を入れ替える変異が緑を通る。
const UI_FILES: [(&str, &str); 4] = [
    (
        "index.html",
        "<!doctype html><title>UI-INDEX-MARKER</title>",
    ),
    ("v1/does-not-exist", "UI-V1-MARKER"),
    ("admin/does-not-exist", "UI-ADMIN-MARKER"),
    ("plain-ui-path", "UI-PLAIN-MARKER"),
];

/// OpenAI 互換のエラー本文（未知のパス）。**`ApiError::not_found` の写し。**
fn openai_not_found() -> Value {
    json!({
        "error": {
            "message": "指定されたエンドポイントは存在しません",
            "type": "invalid_request_error",
        }
    })
}

/// 本文を JSON として読む（読めなければ `None`）。
fn as_json(body: &str) -> Option<Value> {
    serde_json::from_str(body).ok()
}

/// 配り先に目印のファイルを書く。
fn write_ui_files(root: &Path, files: &[(&str, &str)]) {
    for (relative, contents) in files {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("配り先の親を作れること");
        }
        std::fs::write(&path, contents).expect("目印のファイルを書けること");
    }
}

/// エフェメラルポートで起動した実サーバ。**本番の入口（`App::bind`）を通る。**
struct TestServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    /// 指定した配り先で起動する。`files` が空なら**ディレクトリごと作らない**
    /// （＝ `pnpm build` を忘れた状態）。
    async fn start(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let web_dir = dir.path().join("web-dist");
        if !files.is_empty() {
            std::fs::create_dir_all(&web_dir).expect("配り先を作れること");
            write_ui_files(&web_dir, files);
        }
        Self::start_at(dir, &web_dir).await
    }

    /// **`PERSONACORE_WEB_DIR` を立てずに起動する**
    /// ——**実行ファイルに埋め込んだ画面を配る腕**（ADR-0041 §決定2 / 作業単位 F-4a）。
    async fn start_embedded() -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        Self::start_with(dir, None).await
    }

    /// 配り先を明示して起動する。
    async fn start_at(dir: TempDir, web_dir: &Path) -> Self {
        let web_dir = web_dir.to_path_buf();
        Self::start_with(dir, Some(web_dir)).await
    }

    /// 配り先の有無を明示して起動する。**`None` なら埋め込みの腕。**
    async fn start_with(dir: TempDir, web_dir: Option<std::path::PathBuf>) -> Self {
        let web_dir = web_dir.map(|path| path.display().to_string());
        let config =
            AppConfig::from_lookup(|key| (key == ENV_WEB_DIR).then(|| web_dir.clone()).flatten())
                .expect("設定を組み立てられること")
                .with_bind(BindAddr::loopback(0))
                .with_database_path(dir.path().join("personacore.sqlite"));

        let app = App::bind(&config).await.expect("待ち受けを開始できること");
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

    /// GET してステータスと本文を返す。
    async fn get(&self, path: &str) -> (reqwest::StatusCode, String) {
        let (status, _, body) = self.get_with_headers(path).await;
        (status, body)
    }

    /// GET して、**ステータスと欄と本文**を返す。
    ///
    /// **欄まで見るのは1本だけ**——`last-modified` が
    /// 「ディスクの `ServeDir` が答えた」ことの印になる
    /// （`the_disk_web_dir_is_served_only_when_it_is_set`）。
    async fn get_with_headers(
        &self,
        path: &str,
    ) -> (reqwest::StatusCode, reqwest::header::HeaderMap, String) {
        let response = reqwest::get(format!("http://{}{path}", self.addr))
            .await
            .expect("サーバへ接続できること");
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.text().await.expect("本文を読めること");
        (status, headers, body)
    }

    /// **生の要求行をそのまま送る**（ステータスコードと本文を返す）。
    ///
    /// **`reqwest` では送れない綴りがある。** `url` クレートが要求前に畳むので、
    /// `/./v1/x` は `/v1/x` になり、**正規化の穴がテストから見えなくなる。**
    /// **どの綴りが畳まれるかは `the_url_crate_folds_dot_segments_but_not_empty_ones`
    /// が実測して固定している。**
    /// ここは**生のバイト列を書く**ので、崩れた綴りをそのまま観測できる。
    async fn raw(&self, method: &str, target: &str, body: Option<&str>) -> (u16, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut request = format!("{method} {target} HTTP/1.1\r\nHost: {}\r\n", self.addr);
        // **`Connection: close`** —— 応答の終わりを EOF で判定するため。
        request.push_str("Connection: close\r\n");
        if let Some(body) = body {
            request.push_str("Content-Type: application/json\r\n");
            request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        request.push_str("\r\n");
        if let Some(body) = body {
            request.push_str(body);
        }

        let mut stream = tokio::net::TcpStream::connect(self.addr)
            .await
            .expect("サーバへ接続できること");
        stream
            .write_all(request.as_bytes())
            .await
            .expect("要求を書けること");
        stream.flush().await.expect("要求を送り切れること");

        let mut received = Vec::new();
        stream
            .read_to_end(&mut received)
            .await
            .expect("応答を読めること");
        let text = String::from_utf8_lossy(&received).into_owned();
        let (head, body) = text
            .split_once("\r\n\r\n")
            .expect("応答の頭と本文を分けられること");
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .unwrap_or_else(|| panic!("ステータス行を読めること: {head:?}"));

        (status, body.to_owned())
    }

    /// 停止シグナルを送り、サーバタスクの正常終了を待つ。
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

/// **UI のフォールバックが API の 404 を飲み込まない**（ADR-0031 §4）。
///
/// # なぜ3つを1本に置くか
///
/// **高い側（UI へ落とす）と安い側（落とさない）を1本に置いて初めて、
/// 両方向の決め打ちが赤になる**（CLAUDE.md）。
/// 「全部 API のエラー」に潰す変異も、「全部 UI へ落とす」変異も、ここで赤くなる。
///
/// # なぜ配り先に `v1/…` と `admin/…` の実体を置くか
///
/// **置かないと、この検査は「本文が違う」ことしか見ていない。**
/// 3つとも 404 なので、**フォールバックの順序を入れ替えた変異でも
/// ステータスは変わらず、本文は「空 vs JSON」の差でしか出ない。**
/// 実体を置けば、順序が入れ替わった瞬間に **200 と目印**が返る
/// ——「飲み込んだ」がそのまま観測できる形になる。
#[tokio::test]
async fn the_ui_fallback_never_swallows_an_api_error() {
    let server = TestServer::start(&UI_FILES).await;

    // --- (a) `/v1` の未知パス。**UI に同名の実体があっても** API のエラー。 ---
    let (status, body) = server.get("/v1/does-not-exist").await;
    assert_eq!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "/v1 の未知パス: {body}"
    );
    assert_eq!(
        as_json(&body),
        Some(openai_not_found()),
        "/v1 の未知パスは OpenAI 形式のエラーのままであること: {body}"
    );
    assert!(
        !body.contains("UI-V1-MARKER"),
        "/v1 の未知パスを UI が飲み込んでいる: {body}"
    );

    // --- (b) `/admin` の未知パスも同じ。 ---
    let (status, body) = server.get("/admin/does-not-exist").await;
    assert_eq!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "/admin の未知パス: {body}"
    );
    assert_eq!(
        as_json(&body),
        Some(openai_not_found()),
        "/admin の未知パスも OpenAI 形式のエラーのままであること: {body}"
    );
    assert!(
        !body.contains("UI-ADMIN-MARKER"),
        "/admin の未知パスを UI が飲み込んでいる: {body}"
    );

    // --- (c) どちらにも属さないパスは UI へ落ちる。 ---
    let (status, body) = server.get("/plain-ui-path").await;
    assert_eq!(status, reqwest::StatusCode::OK, "UI のパス: {body}");
    assert_eq!(body, "UI-PLAIN-MARKER", "UI の実体が返ること");

    // --- (d) ルートは `index.html`。 ---
    let (status, body) = server.get("/").await;
    assert_eq!(status, reqwest::StatusCode::OK, "ルート: {body}");
    assert!(
        body.contains("UI-INDEX-MARKER"),
        "ルートは index.html を返すこと: {body}"
    );

    // --- (e) UI 側にも無いパスは、**API のエラー形にはしない。** ---
    let (status, body) = server.get("/no-such-ui-file").await;
    assert_eq!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "UI の未知パス: {body}"
    );
    assert_ne!(
        as_json(&body),
        Some(openai_not_found()),
        "UI の未知パスまで API のエラー形にすると、(a)(b) との区別が消える: {body}"
    );

    server.stop().await;
}

/// **HTTP クライアント（`url` クレート）が、どの綴りを畳むか。**
///
/// **これは他所のクレートの振る舞いを固定する検査**である。置いてある理由は2つ:
///
///   * **`raw` を「`reqwest` で十分では」と読んで消させないため。**
///     **`.` / `..` / `%2e` は畳まれる**ので、その3つは `reqwest` からは観測できない。
///   * **`/../v1/x` をどちらへ寄せるかの判断の根拠**（`src/server/web.rs` のモジュール doc）。
///     **URL ライブラリを使うクライアントは、そもそも `/../v1/x` を線に乗せない。**
///     だから「`..` を飛ばさない」選択で困る現実のクライアントがいない。
///
/// **畳まれない綴り**（`//v1/x` / `/%76%31/x`）**は普通に線に乗る。**
/// —— **だから正規化が要る。**
#[test]
fn the_url_crate_folds_dot_segments_but_not_empty_ones() {
    for (spelling, on_the_wire) in [
        // **畳まれない**——ここが `starts_with` だと UI へ落ちていた形。
        ("//v1/x", "//v1/x"),
        ("/%76%31/x", "/%76%31/x"),
        // **畳まれる**——`reqwest` からは観測できない形。
        ("/./v1/x", "/v1/x"),
        ("/%2e/v1/x", "/v1/x"),
        ("/../v1/x", "/v1/x"),
        // 素の綴りは当然そのまま。
        ("/v1/x", "/v1/x"),
    ] {
        let url = reqwest::Url::parse(&format!("http://127.0.0.1:8080{spelling}"))
            .expect("URL として読めること");
        assert_eq!(url.path(), on_the_wire, "{spelling} の送られ方");
    }
}

/// **崩れた綴りの API パスも、UI に飲み込まれない**（ADR-0031 §4）。
///
/// 生パスを `starts_with` で見ていると、**ここが全部 UI へ落ちる。**
/// `/v1/does-not-exist` は 404 ＋ OpenAI 形なのに、
/// **`//v1/does-not-exist` だけ 200 ＋ `index.html`** という形になり、
/// **クライアントからは「JSON が壊れている」としか見えない。**
///
/// **⚠ `reqwest` では観測できない**（`url` が正規化する）。`raw` を使うこと。
#[tokio::test]
async fn an_unnormalised_api_path_is_still_an_api_path() {
    let server = TestServer::start(&UI_FILES).await;

    for target in [
        "//v1/does-not-exist",
        "///v1/does-not-exist",
        "/./v1/does-not-exist",
        "/%76%31/does-not-exist",
        "//admin/does-not-exist",
        "/%61dmin/does-not-exist",
    ] {
        let (status, body) = server.raw("GET", target, None).await;
        assert_eq!(status, 404, "{target}: {body}");
        assert_eq!(
            as_json(&body),
            Some(openai_not_found()),
            "{target} は OpenAI 形式のエラーのままであること: {body}"
        );
        assert!(
            !body.contains("UI-"),
            "{target} を UI が飲み込んでいる: {body}"
        );
    }

    // --- 対比: UI のパスは崩れた綴りでも UI のまま。 ---
    // **これが無いと「全部 API のエラー」に潰す変異がここで緑を通る。**
    for target in ["/plain-ui-path", "//plain-ui-path"] {
        let (status, body) = server.raw("GET", target, None).await;
        assert_eq!(status, 200, "{target}: {body}");
        assert_eq!(body, "UI-PLAIN-MARKER", "{target} は UI の実体を返すこと");
    }

    server.stop().await;
}

/// **base URL が `/` で終わるクライアントが作る形**（`base + "/v1/chat/completions"`）。
///
/// **D-0 ではここが `ServeDir` へ落ちて、本文の無い 405 になっていた**
/// （`ServeDir` は GET / HEAD しか受けない）。
/// クライアント側には **`Unexpected end of JSON input`** としか出ないので、
/// **原因（スラッシュが2つ）から最も遠い形で現れる。**
#[tokio::test]
async fn a_doubled_slash_before_the_chat_endpoint_answers_in_the_openai_shape() {
    const BROKEN_BODY: &str = "{}";

    let server = TestServer::start(&UI_FILES).await;

    let (status, body) = server
        .raw("POST", "//v1/chat/completions", Some(BROKEN_BODY))
        .await;
    assert_eq!(status, 404, "//v1/chat/completions: {body}");
    assert_eq!(
        as_json(&body),
        Some(openai_not_found()),
        "本文の無い 405 にしないこと（JSON として読めない応答になる）: {body}"
    );

    // --- 対比: 正しい綴りは**別の答え**（本文が不正なので 400）。 ---
    // 両方を1本に置く——「全部 404」に潰す変異はここで赤くなる。
    let (status, body) = server
        .raw("POST", "/v1/chat/completions", Some(BROKEN_BODY))
        .await;
    assert_eq!(
        status, 400,
        "正しい綴りは本文の検証まで届くこと（404 ではない）: {body}"
    );
    assert!(
        as_json(&body).is_some_and(|value| value.get("error").is_some()),
        "正しい綴りの答えも JSON であること: {body}"
    );

    server.stop().await;
}

/// **配り先は設定から来る**（固定パスでも既定値でもない）。
///
/// 2つのサーバを**別々の配り先**で立て、**それぞれが自分の配り先の中身を返す**ことを見る。
/// 1つだけだと「どこか固定のパスを配っている」と区別が付かない。
#[tokio::test]
async fn each_server_serves_the_directory_it_was_configured_with() {
    const FIRST: &str = "UI-FIRST-MARKER";
    const SECOND: &str = "UI-SECOND-MARKER";
    assert_ne!(FIRST, SECOND, "2つの配り先の中身は違う値にすること");

    let first = TestServer::start(&[("index.html", FIRST)]).await;
    let second = TestServer::start(&[("index.html", SECOND)]).await;

    let (status, body) = first.get("/index.html").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body, FIRST, "1つめは自分の配り先を返すこと");

    let (status, body) = second.get("/index.html").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body, SECOND, "2つめは自分の配り先を返すこと");

    first.stop().await;
    second.stop().await;
}

/// **埋め込みの腕でも、UI のフォールバックが API の 404 を飲み込まない**
/// （ADR-0031 §4 / ADR-0041 §決定2 / 作業単位 F-4a）。
///
/// # なぜディスクの腕とは別に要るのか
///
/// **`is_api_path` の判定は `unknown_route` の1箇所で、2つの腕はその下流に並ぶ。**
/// **だから「埋め込み側だけ API を飲み込む」実装は、いま1箇所を壊さないと書けない**
/// ——が、**それは「いまの形がそうである」という事実であって、
/// 守られていることではない**（CLAUDE.md「作法がある」と「見張り番がある」は別）。
/// **埋め込みの腕を通る検査が1本も無ければ、
/// 腕ごとに判定を分ける変更が黙って入る。**
///
/// # ⚠ この検査は `pnpm build` の有無に左右されない
///
/// **埋め込みの中身はコンパイル時に決まる**ので、
/// `web/dist` をビルドしていない機では表が空になる。
/// **ここが見るのは「API のパスが UI へ落ちないこと」だけ**なので、
/// **表が空でも中身入りでも同じ答えになる。**
#[tokio::test]
async fn the_embedded_ui_never_swallows_an_api_error() {
    let server = TestServer::start_embedded().await;

    for path in ["/v1/does-not-exist", "/admin/does-not-exist"] {
        let (status, body) = server.get(path).await;
        assert_eq!(status, reqwest::StatusCode::NOT_FOUND, "{path}: {body}");
        assert_eq!(
            as_json(&body),
            Some(openai_not_found()),
            "{path} は OpenAI 形式のエラーのままであること: {body}"
        );
    }

    // **崩れた綴りも同じ**（`reqwest` では送れないので生の要求行で）。
    for target in ["//v1/does-not-exist", "/%61dmin/does-not-exist"] {
        let (status, body) = server.raw("GET", target, None).await;
        assert_eq!(status, 404, "{target}: {body}");
        assert_eq!(
            as_json(&body),
            Some(openai_not_found()),
            "{target} は OpenAI 形式のエラーのままであること: {body}"
        );
    }

    // --- 対比: API でないパスは **API のエラー形にしない。** ---
    // **これが無いと「全部 API のエラー」に潰す変異がここで緑を通る。**
    let (status, body) = server.get("/no-such-ui-file").await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND, "{body}");
    assert_ne!(
        as_json(&body),
        Some(openai_not_found()),
        "UI の未知パスまで API のエラー形にすると、上の対比が消える: {body}"
    );

    // --- `..` は埋め込みからも辿れない（本文の無い 404）。 ---
    //
    // **綴りは「`..` を飛ばすと実在の名前になる」形にすること。**
    // `/assets/../index.html` も `/x/../index.html` も、
    // **飛ばすと `assets/index.html` / `x/index.html` という、どちらにせよ無い名前**になる
    // ——**飛ばす変異が素通りする**（2回とも実測 / `m3`）。
    // **先頭の `..` だけが、飛ばした瞬間に `index.html` そのものになる。**
    //
    // **⚠ `web/dist` をビルドしていない機では 404 で何も言わない**
    // （`cargo test` は FE をビルドしない）。
    // **決定的な権威は `server::web` の
    // `a_dot_dot_segment_never_resolves_in_the_embedded_ui`**（植えた表で測る）。
    let (status, body) = server.raw("GET", "/../index.html", None).await;
    assert_eq!(status, 404, "`..` を辿ってはいけない: {body}");
    assert!(body.is_empty(), "本文の無い 404 であること: {body}");

    server.stop().await;
}

/// **`PERSONACORE_WEB_DIR` が設定されたときだけディスクから配る**
/// （ADR-0041 §決定2 / 作業単位 F-4a）。
///
/// # なぜ2つの腕を1本に置くか
///
/// **片腕だけでは決め打ちが通る**（CLAUDE.md「対比は安い側だけでは足りない」）——
///
/// | 変異 | 片腕だけだと |
/// |---|---|
/// | 未設定でもディスク（`web/dist`）を読む | 設定した腕だけ見ていれば緑 |
/// | 設定されていても埋め込みを配る | 未設定の腕だけ見ていれば緑 |
///
/// **目印のファイル名は、埋め込みに絶対に無い綴りにする**
/// ——在ると「埋め込みから返った」と「ディスクから返った」が区別できない。
///
/// # ⚠ 「未設定なのにディスクを読む」を、本文だけでは見分けられない
///
/// **埋め込み元は `web/dist` そのもの**なので、
/// **「未設定でも `web/dist` をディスクから読む」実装は、本文がバイト単位で同じになる。**
/// **変異検査で実測**——目印だけを見る形では `m2a` が素通りした。
///
/// **見分けているのは `last-modified` の欄。**
/// `ServeDir` はファイルの更新時刻を載せるが、**埋め込みには更新時刻が無い**ので載らない。
/// **他所のクレートの振る舞いを固定している**ので、
/// **`ServeDir` 側（載る）と埋め込み側（載らない）を必ず対にして見る**
/// ——片方だけだと、tower-http が欄をやめた日に「守っている」と誤読する。
///
/// **⚠ `web/dist/index.html` が無い機では、未設定の腕は 404 になり、
/// この見分けは何も言わない**（`cargo test` は FE をビルドしない）。
/// **そのときの権威は起動ログの側**——`the_startup_line_says_where_the_ui_came_from`。
#[tokio::test]
async fn the_disk_web_dir_is_served_only_when_it_is_set() {
    use reqwest::header::LAST_MODIFIED;

    // **`vite` が出す名前ではない綴り**（`index.html` / `assets/…` を避ける）。
    const MARKER_PATH: &str = "/marker-that-is-never-built-by-vite";
    const MARKER_BODY: &str = "UI-FROM-DISK-MARKER";

    let on_disk = TestServer::start(&[(&MARKER_PATH[1..], MARKER_BODY)]).await;
    let (status, headers, body) = on_disk.get_with_headers(MARKER_PATH).await;
    assert_eq!(status, reqwest::StatusCode::OK, "{MARKER_PATH}: {body}");
    assert_eq!(body, MARKER_BODY, "設定した配り先の実体が返ること");
    // **対照**: `ServeDir` は更新時刻を載せる（この欄が見分けの根拠であること）。
    assert!(
        headers.contains_key(LAST_MODIFIED),
        "ServeDir が last-modified を載せていない（見分けの根拠が消えている）: {headers:?}"
    );
    on_disk.stop().await;

    let embedded = TestServer::start_embedded().await;
    let (status, _, body) = embedded.get_with_headers(MARKER_PATH).await;
    assert_eq!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "未設定のときはディスクを読まないこと: {body}"
    );
    assert_ne!(
        body, MARKER_BODY,
        "未設定なのにディスクの実体が返っている（埋め込みを配ること）"
    );

    // **埋め込み元と同じ中身を、埋め込み経由で返していること。**
    // `web/dist/index.html` が在るときだけ意味を持つ（無ければ 404 で何も言わない）。
    let index = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("web")
        .join("dist")
        .join("index.html");
    if index.is_file() {
        let (status, headers, body) = embedded.get_with_headers("/").await;
        assert_eq!(status, reqwest::StatusCode::OK, "埋め込みの `/`: {body}");
        assert!(
            !headers.contains_key(LAST_MODIFIED),
            "未設定なのに ServeDir が答えている（埋め込みには更新時刻が無い）: {headers:?}"
        );
        assert_eq!(
            body,
            std::fs::read_to_string(&index).expect("web/dist/index.html を読めること"),
            "埋め込んだ index.html が web/dist のものと違う"
        );
    }
    embedded.stop().await;
}

/// **配り先が無くても API は生きている**（`pnpm build` を忘れた状態）。
///
/// 起動が落ちたり、UI のパスが 500 になったりしないこと。
/// **気付く手段はログ**（`the_startup_line_says_where_the_ui_came_from`）であって、
/// 起動を止めることではない——止めると API だけ使いたい運用が壊れる。
#[tokio::test]
async fn a_missing_web_dir_does_not_break_the_api() {
    let server = TestServer::start(&[]).await;

    let (status, body) = server.get("/v1/models").await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "API は生きていること: {body}"
    );

    let (status, body) = server.get("/").await;
    assert_eq!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "配り先が無いときの UI は 404（500 にしない）: {body}"
    );

    let (status, body) = server.get("/v1/does-not-exist").await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
    assert_eq!(
        as_json(&body),
        Some(openai_not_found()),
        "配り先が無くても API のエラー形は変わらないこと: {body}"
    );

    server.stop().await;
}
