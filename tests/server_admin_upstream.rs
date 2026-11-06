//! `GET /admin/upstream` の検証（作業単位 G-10 /
//! [ADR-0039](../docs/adr/0039-two-upstreams-for-two-jobs.md) §決定4）。
//!
//! **実際の LM Studio には繋がない**（docs/05-development.md）。上流はこのファイルが立てる。
//!
//! # なぜ本番の入口（`App::bind`）から起動するのか
//!
//! **`App::bind_with_clients`（注入の入口）は照会そのものを行わない**ので、
//! 観測は「一度も聞いていない」で固定される。**面ごとに別の宛先・別の綴り・別の窓**が
//! 応答へ届いていることは、`bind` を通したときにしか現れない。
//!
//! # ここが見ているもの
//!
//! | 見るもの | 何が壊れたら赤になるか |
//! |---|---|
//! | 面ごとに**その面の**宛先・綴り・窓・読み手・内訳が返る | 本文と抽出の取り違え（`AppState::upstream` の入れ替えを含む） |
//! | **上流が名乗った綴り**（gguf の絶対パス）が**応答本文**に出ない | 一覧や `model_id` を観測へ持ち上げる |
//! | **設定の認証情報**が**応答本文**に出ない | `endpoint_for_view` を外す（B-71 で `redact_userinfo` から改名） |
//! | **宛先のクエリ・フラグメント**が**応答本文**に出ない（作業単位 B-71） | `endpoint_for_view` を外す・`endpoint_for_log` の `set_query(None)` を外す |
//! | **引き直しの結果**が返る（起動時の写しではない） | ハンドラが起動時の値を写し取る |
//! | 生えているのは `GET` 1本だけ | 「変える」導線が生える |
//!
//! # ⚠ 走査は `Debug` ではなく**応答本文**に当てる
//!
//! `tests/server_upstream_facts.rs` の2本は `format!("{facts:?}")` を見ている。
//! **`Debug` に出ないことは、応答本文に出ないことを1ミリも意味しない**
//! （`server::upstream` モジュールの doc の申し送り）。**ここは本文そのものを読む。**
//!
//! # ⚠ 既定で回さない
//!
//! 2つの宛先も2つのモデルの綴りも、**互いに違い、どれも既定ではない。**
//! 同値にすると「設定を読まずに本文の上流を使う」実装と区別が付かない
//! （CLAUDE.md の最頻の穴）。

use std::net::SocketAddr;

use personacore::measured::{FaceTally, ModelFace, judge_face};
use personacore::server::{
    App, AppConfig, BindAddr, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL, ENV_SUB_LLM_ENDPOINT,
    ENV_SUB_MODEL, ServerError, SharedUpstream,
};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

/// この入口のパス。
const UPSTREAM_PATH: &str = "/admin/upstream";

/// 本文生成に指定するモデル。**既定とも抽出用とも別。**
const MAIN_MODEL: &str = "ministral-3-14b-reasoning-2512";

/// 抽出に指定するモデル。**既定とも本文用とも別。**
const SUB_MODEL: &str = "ministral-3-3b-reasoning-2512";

/// 本文の上流が申告する窓。**抽出側とも既定（8192）とも落とし所（16384）とも別。**
const MAIN_TOKENS: u32 = 20_480;

/// 抽出の上流が申告する窓。**本文側とも既定とも落とし所とも別。**
const SUB_TOKENS: u32 = 6_144;

/// **`/api/v0/models` を持たない上流**が申告する窓。**上の2つとも別。**
const LLAMA_TOKENS: u32 = 24_576;

/// その上流が名乗る `id`（**gguf の絶対パス** / 設定中の綴りとは一致しない）。
///
/// **このうち応答本文へ出てよいのはファイル名だけ**（作業単位 B-45 / `server::upstream` の doc）。
const LLAMA_MODEL_ID: &str = "/models/Ministral-3-14B-Instruct-2512-Q4_K_M.gguf";

/// **同じファイル名を Windows 形の絶対パスで名乗る上流の `id`。**
///
/// **`/` しか見ない切り落としは、この形を丸ごと通す。**
const LLAMA_MODEL_ID_WINDOWS: &str = "C:\\models\\llm\\Ministral-3-14B-Instruct-2512-Q4_K_M.gguf";

/// **上の2つの、応答本文へ出てよい部分**（ファイル名）。
const LLAMA_MODEL_FILE: &str = "Ministral-3-14B-Instruct-2512-Q4_K_M.gguf";

/// **出てはならない部分**（POSIX 形のディレクトリ）。
const LLAMA_MODEL_DIR: &str = "/models/";

/// **出てはならない部分**（Windows 形のディレクトリ）。
const LLAMA_MODEL_DIR_WINDOWS: &str = "C:\\models\\llm\\";

/// 設定に書ける利用者名。**他のどのフィクスチャとも重ならない綴り。**
const ENDPOINT_USER: &str = "personacore-operator";

/// 設定に書ける合言葉。**利用者名とも別。**
const ENDPOINT_PASSWORD: &str = "s3cr3t-never-in-a-response";

/// **`/api/v0/models` に実機と同じ形で答える上流**を立てる。
async fn upstream_reporting(model: &str, loaded: u32) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(
                r#"{{"object":"list","data":[
                  {{"id":"{model}","object":"model","type":"llm","state":"loaded",
                    "max_context_length":262144,"loaded_context_length":{loaded}}}]}}"#
            ),
            "application/json",
        ))
        .mount(&server)
        .await;
    server
}

/// **`/api/v0/models` を持たず、`/v1/models` にだけ答える上流**（`llama-server` の形）。
///
/// **一覧は1件で、`id` は引数の綴り。** 同定は「一覧が1件だから」で通る。
///
/// **本文は `serde_json` に組ませる**（作業単位 B-45）——
/// **Windows 形の `\` を手で書くと JSON として壊れ、一覧が空になって検査が空振りする。**
async fn upstream_speaking_only_openai(model_id: &str) -> MockServer {
    let body = json!({
        "object": "list",
        "data": [{
            "id": model_id,
            "object": "model",
            "owned_by": "llamacpp",
            "meta": {"n_ctx": LLAMA_TOKENS, "n_ctx_train": 262_144},
        }],
    })
    .to_string();

    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;
    server
}

/// **ディレクトリの綴りが、応答本文のどこにも出ていない。**
///
/// **⚠ JSON は `\` を `\\` へ逃がす**ので、**逃がした形も一緒に探す**
/// ——生の綴りだけを見る検査は、Windows 形で**必ず空振りする。**
fn assert_no_directory(text: &str, dir: &str, what: &str) {
    let escaped = dir.replace('\\', "\\\\");
    for needle in [dir, escaped.as_str()] {
        assert!(
            !text.contains(needle),
            "{what}: ディレクトリの綴り {needle:?} が応答本文に出ないこと: {text}"
        );
    }
}

/// **観測に載る宛先の綴り**（`LmStudioConfig::endpoint` そのもの）。
fn completions_url(server: &MockServer) -> String {
    format!("{}/v1/chat/completions", server.uri())
}

/// 応答に載る内訳の形。**期待値は `FaceTally` の欄そのものから組む**
/// ——欄の取り違え（`matched` と `unanswered` の入れ替え等）はここで赤になる。
///
/// **⚠ このフィクスチャでは 4 欄のうち 2 つが 0 である。**
/// **4 欄すべてを別の値で回す腕は `server::upstream` の
/// `the_breakdown_keeps_every_counter_apart`**（純粋関数の側）。
fn measured_json(tally: FaceTally) -> Value {
    json!({
        "checked": tally.checked(),
        "matched": tally.matched,
        "drifted_other": tally.drifted_other,
        "drifted_absent": tally.drifted_absent,
        "unanswered": tally.unanswered,
    })
}

/// エフェメラルポートで起動した実サーバ。
struct TestServer {
    addr: SocketAddr,
    /// **引き直しが書く先そのもの**（`App::upstream_handle`）。
    upstream: SharedUpstream,
    /// DB ファイルの絶対パス。応答に漏れていないことの確認に使う。
    db_path: String,
    client: reqwest::Client,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    /// **本番の入口（`App::bind`）から起動する。**
    async fn start(env: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db_path = dir.path().join("personacore.sqlite");

        let pairs: Vec<(String, String)> = env
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        assert!(
            pairs.iter().any(|(name, _)| name == ENV_LLM_ENDPOINT),
            "上流は必ずモックサーバへ向けること（実機の LM Studio に繋がない）"
        );

        let config = AppConfig::from_lookup(|key| {
            pairs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(&db_path);

        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let upstream = app.upstream_handle();

        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });

        Self {
            addr,
            upstream,
            db_path: db_path.display().to_string(),
            client: reqwest::Client::new(),
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    /// **応答の生の本文と、読んだ JSON の両方**を返す。
    ///
    /// **走査は生の本文に当てる**（このファイルの doc）。
    async fn send(&self, method: Method, path: &str) -> (StatusCode, String, Value) {
        let response = self
            .client
            .request(method, format!("http://{}{path}", self.addr))
            .send()
            .await
            .expect("サーバへ接続できること");
        let status = response.status();
        let text = response.text().await.expect("本文を読めること");
        let value = serde_json::from_str(&text)
            .unwrap_or_else(|err| panic!("応答が JSON であること: {text}: {err}"));
        (status, text, value)
    }

    /// `GET /admin/upstream`。**200 であること**まで見る。
    async fn upstream_report(&self) -> (String, Value) {
        let (status, text, value) = self.send(Method::GET, UPSTREAM_PATH).await;
        assert_eq!(status, StatusCode::OK, "上流の観測を読めること: {text}");
        (text, value)
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await.expect("サーバのタスクが panic しないこと");
        }
    }
}

/// **内部情報が漏れていないこと。** 応答の**生の本文**に当てる。
fn assert_no_internals(server: &TestServer, text: &str) {
    for leak in [
        server.db_path.as_str(),
        "sqlite",
        "sqlx",
        "personacore.sqlite",
        "src\\",
        "src/",
    ] {
        assert!(
            !text.contains(leak),
            "内部情報 {leak:?} が漏れていないこと: {text}"
        );
    }
}

/// **フィクスチャが2つの面を同値にしていない。**
///
/// 同値なら以下の突き合わせは、取り違えを1つも検出しない。
#[test]
fn the_fixture_never_makes_the_two_faces_equal() {
    assert_ne!(MAIN_MODEL, SUB_MODEL);
    assert_ne!(MAIN_TOKENS, SUB_TOKENS);
    assert_ne!(MAIN_TOKENS, LLAMA_TOKENS);
    assert_ne!(SUB_TOKENS, LLAMA_TOKENS);
    assert_ne!(ENDPOINT_USER, ENDPOINT_PASSWORD);
    // **上流が名乗る綴りは、こちらが依頼する綴りとも互いとも別。**
    assert_ne!(LLAMA_MODEL_FILE, MAIN_MODEL);
    assert_ne!(LLAMA_MODEL_FILE, SUB_MODEL);
    assert_ne!(LLAMA_MODEL_ID, LLAMA_MODEL_ID_WINDOWS);
    assert_ne!(LLAMA_MODEL_DIR, LLAMA_MODEL_DIR_WINDOWS);
}

/// **2つの上流が、面ごとに別々に返る**（ADR-0039 §決定4）。
///
/// **本文全体を完全一致で突き合わせる。** 欄を1つ落とす形も、面を入れ替える形も、
/// **どちらか片方の面だけを2度返す形も**、ここで赤になる。
///
/// # 内訳の帰属
///
/// **面ごとに行数が違う**（`MeasuredRole::Sub` の定数が1つあるので 4 行 と 5 行）ので、
/// **内訳を面ごと入れ替える変異もここで赤になる。**
///
/// # ⚠ ここでは「頼んだ名前」と「答えた名前」の入れ替えは観測できない
///
/// **この上流は `/api/v0/models` に依頼した綴りをそのまま並べる**（LM Studio の形）ので、
/// **`model` と `answered_name` が同値になる。**
/// **実測（作業単位 B-45 / 2026-09-17）: `UpstreamView` で2つの欄を入れ替える変異は、
/// この1本を赤にしない。**
///
/// **捕まえているのは、2つが別の綴りになる腕のほう**
/// ——`the_response_never_carries_the_upstreams_own_model_id`（`llama-server` の形）と、
/// `server::upstream` の `a_face_that_answered_reports_who_answered_and_what_it_declared`。
/// **同値のフィクスチャは、取り違えを1つも検出しない**（CLAUDE.md の最頻の穴）。
#[tokio::test]
async fn the_two_upstreams_come_back_named_apart() {
    let main_upstream = upstream_reporting(MAIN_MODEL, MAIN_TOKENS).await;
    let sub_upstream = upstream_reporting(SUB_MODEL, SUB_TOKENS).await;

    let server = TestServer::start(&[
        (ENV_LLM_ENDPOINT, &main_upstream.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub_upstream.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
    ])
    .await;

    let (text, mut body) = server.upstream_report().await;

    // **照会した時刻は、ここで取り出して形だけ見る**（作業単位 B-66）——値は照会のたびに変わるので、
    // 本文全体の完全一致からは外す（取り出した後は `null` が入る）。時刻の帰属と値は
    // `server::upstream` の `the_report_says_when_and_how_each_face_was_checked` が見る。
    for face in ["main", "sub"] {
        let at = body[face]["checked_at"].take();
        let at = at
            .as_str()
            .unwrap_or_else(|| panic!("照会した面に時刻が載ること（{face}）: {text}"));
        chrono::DateTime::parse_from_rfc3339(at)
            .unwrap_or_else(|err| panic!("時刻は RFC 3339 であること（{face} / {at}）: {err}"));
    }

    assert_eq!(
        body,
        json!({
            "main": {
                "face": "main",
                "endpoint": completions_url(&main_upstream),
                "model": MAIN_MODEL,
                // **この上流は依頼した綴りをそのまま一覧に持っている**ので、
                // 頼んだ名前と答えた名前が一致する（作業単位 B-45）。
                "answered_name": MAIN_MODEL,
                "queried_context_tokens": MAIN_TOKENS,
                "answered_by": "native-api",
                // **鍵を断られていない**（作業単位 A-13b-1。断られた腕は `tests/server_profile_keys.rs`）。
                "unauthorized": false,
                // **時刻は上で取り出した**（作業単位 B-66）。結果は「答えた」。
                "checked_at": Value::Null,
                "check_result": "answered",
                "measured": measured_json(judge_face(
                    ModelFace::Main,
                    MAIN_MODEL,
                    Some(&[MAIN_MODEL.to_owned()]),
                )),
                // **この起動は `--upstream` を渡していない**ので、
                // **どちらの面も「外の上流」**（ADR-0042 §決定3）。
                "launch": {
                    "state": "external",
                    "since": Value::Null,
                    "answered_name": Value::Null,
                    "reason": Value::Null,
                    "log": Value::Null,
                },
            },
            "sub": {
                "face": "sub",
                "endpoint": completions_url(&sub_upstream),
                "model": SUB_MODEL,
                "answered_name": SUB_MODEL,
                "queried_context_tokens": SUB_TOKENS,
                "answered_by": "native-api",
                "unauthorized": false,
                "checked_at": Value::Null,
                "check_result": "answered",
                "measured": measured_json(judge_face(
                    ModelFace::Sub,
                    SUB_MODEL,
                    Some(&[SUB_MODEL.to_owned()]),
                )),
                "launch": {
                    "state": "external",
                    "since": Value::Null,
                    "answered_name": Value::Null,
                    "reason": Value::Null,
                    "log": Value::Null,
                },
            },
            // **テストの入口（`App::bind`）は見本を書かない**ので、書いたと言わない
            // （作業単位 F-4c。書いた起動の腕は `tests/upstream_cli.rs`）。
            "profile_sample": Value::Null,
            // **テストの入口はプロファイルを選ばない**ので、選んだと言わない
            // （作業単位 F-10b。選んだ起動の腕は `tests/upstream_cli.rs` の `assert_faces_on`）。
            "selected_profile": Value::Null,
        }),
        "面ごとの観測がそのまま返ること: {text}"
    );

    // **同値のフィクスチャで空振りしていないこと。**
    assert_ne!(
        body["main"]["measured"], body["sub"]["measured"],
        "面ごとに内訳が違うこと（同値だと内訳の取り違えを検出できない）: {text}"
    );

    assert_no_internals(&server, &text);
    server.stop().await;
}

/// **上流が名乗った綴りのうち、応答本文に出るのはファイル名だけ**
/// （絶対制約: 内部パスを外へ出さない / 作業単位 B-45）。
///
/// # ⚠ この1本は、B-45 で意味が変わった（司令塔が承認 / 2026-09-17）
///
/// **着手前は「上流が名乗った綴りが1文字も出ない」だった。**
/// **いまは「ディレクトリ部分が出ない」を見る** ——
/// **`C-18` が実測したとおり、`llama serve` は `model` の欄を無視して
/// 載っているモデルで答える**ので、**名乗るものが応答に1つも無いと、
/// 別物が答えていることを画面から読む手段が無い。**
///
/// # 空振りではないこと
///
/// **同じ応答の中で**4つを固定する:
///
/// 1. **その一覧を現に読んだ回であること**（`answered_by` と窓）。
/// 2. **答えた名前がファイル名で出ていること。**
/// 3. **頼んだ名前が、別の欄に、別の綴りで出ていること**
///    （**同値のフィクスチャでは、取り違えが観測できない**）。
/// 4. **ディレクトリは、生の形でも逃がした形でも出ていないこと。**
///
/// # ⚠ `Debug` の見張り番では捕まらない
///
/// `tests/server_upstream_facts.rs::the_held_facts_never_carry_the_upstreams_own_model_id`
/// は `format!("{facts:?}")` を見る。**手書きの `Debug` で欄を隠せば、あちらは緑のまま
/// 応答本文に綴りが出る**（`server::upstream` モジュールの doc の申し送り）。
/// **ここは応答本文そのものを読む。**
#[tokio::test]
async fn the_response_never_carries_the_upstreams_own_model_id() {
    for (model_id, dir, what) in [
        (LLAMA_MODEL_ID, LLAMA_MODEL_DIR, "POSIX 形の絶対パス"),
        (
            LLAMA_MODEL_ID_WINDOWS,
            LLAMA_MODEL_DIR_WINDOWS,
            "Windows 形の絶対パス",
        ),
    ] {
        let upstream = upstream_speaking_only_openai(model_id).await;

        let server = TestServer::start(&[
            (ENV_LLM_ENDPOINT, &upstream.uri()),
            (ENV_MAIN_MODEL, MAIN_MODEL),
            (ENV_SUB_MODEL, SUB_MODEL),
        ])
        .await;

        let (text, body) = server.upstream_report().await;

        // 前提: その一覧を実際に通した回であること。
        assert_eq!(
            body["main"]["answered_by"],
            json!("openai-sole-entry"),
            "{what}: 前提: 1件だけの OpenAI 互換の一覧から同定した回であること: {text}"
        );
        assert_eq!(
            body["main"]["queried_context_tokens"],
            json!(LLAMA_TOKENS),
            "{what}: 前提: その一覧から窓を読んでいること: {text}"
        );

        // 本題1: 頼んだ名前と答えた名前が、別々の欄に、別の綴りで載る。
        assert_eq!(
            body["main"]["model"],
            json!(MAIN_MODEL),
            "{what}: 頼んだ名前はこちらの綴りのままであること: {text}"
        );
        assert_eq!(
            body["main"]["answered_name"],
            json!(LLAMA_MODEL_FILE),
            "{what}: 答えた名前は、上流が名乗った id のファイル名であること: {text}"
        );
        assert_ne!(
            body["main"]["model"], body["main"]["answered_name"],
            "{what}: 別物が答えていることが、2つの欄の違いとして読めること: {text}"
        );

        // 本題2: ディレクトリは、応答本文に1文字も出ない。
        assert_no_directory(&text, dir, what);
        assert!(
            !text.contains(model_id),
            "{what}: 名乗った綴りを丸ごと出さないこと: {text}"
        );

        assert_no_internals(&server, &text);
        server.stop().await;
    }
}

/// **設定に書かれた認証情報は、応答本文に1文字も出ない**
/// （絶対制約: 応答に接続文字列を含めない）。
///
/// # 空振りではないこと
///
/// **その宛先で実際に往復した回であること**を、**同じ応答の中で**先に固定する。
///
/// # ⚠ 外さないこと —— この1本だけが捕まえる族が在る
///
/// 落としているのは `UpstreamObservation::new` の1行（`endpoint_for_view`。B-71 までは `redact_userinfo`）で、
/// **その1行を外す変異なら、観測の側を見る見張り番**
/// （`tests/server_upstream_facts.rs` と `server::upstream` の単体テスト）**も一緒に赤になる。**
///
/// **⚠ だからといって写しではない**（G-10 のレビューが実測 / 2026-09-08）。
/// **`UpstreamObservation` に「落とす前の綴り」を保持し、その欄の `Debug` を手書きで隠す**と、
/// **上の2本は原理的に見えず、この1本だけが赤になる**
/// （**2004 passed / 1 failed / exit 101**）。**ここが見ているのは「応答本文」である。**
#[tokio::test]
async fn the_response_never_shows_the_endpoints_credentials() {
    let upstream = upstream_reporting(MAIN_MODEL, MAIN_TOKENS).await;
    let host = upstream
        .uri()
        .strip_prefix("http://")
        .expect("モックサーバの綴りが http であること")
        .to_owned();
    let with_credentials = format!("http://{ENDPOINT_USER}:{ENDPOINT_PASSWORD}@{host}");

    let server = TestServer::start(&[
        (ENV_LLM_ENDPOINT, &with_credentials),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
    ])
    .await;

    let (text, body) = server.upstream_report().await;

    // 前提: その宛先で実際に往復していること。
    assert_eq!(
        body["main"]["answered_by"],
        json!("native-api"),
        "前提: 認証情報つきの宛先でも上流に届いていること: {text}"
    );
    assert_eq!(
        body["main"]["queried_context_tokens"],
        json!(MAIN_TOKENS),
        "前提: その往復から窓を読んでいること: {text}"
    );

    // 本題: 認証情報だけが落ちていて、宛先の同定は残る。
    assert_eq!(
        body["main"]["endpoint"],
        json!(format!("http://***@{host}/v1/chat/completions")),
        "認証情報だけを落とすこと: {text}"
    );
    for secret in [ENDPOINT_USER, ENDPOINT_PASSWORD] {
        assert!(
            !text.contains(secret),
            "設定の認証情報が応答本文に出ないこと（{secret}）: {text}"
        );
    }
    assert!(
        text.contains(&host),
        "宛先の同定は残ること（丸ごと落とさない）: {text}"
    );

    assert_no_internals(&server, &text);
    server.stop().await;
}

/// **宛先に書いたクエリとフラグメントも、応答本文に1文字も出ない**（作業単位 B-71 / Issue origin#100）。
///
/// 着手前は `***@` でユーザー情報だけを伏せ、`?token=…` はそのまま返していた。
///
/// # 空振りではないこと
///
/// **その宛先で実際に往復した回であること**を、**同じ応答の中で**先に固定する。
/// 宛先に書いたクエリは、personacore が足す経路（`/api/v0/models`）を呑む ——
/// **送信が叩く経路は書いたパス（`/pc`）なので、上流はそこで答える。**
#[tokio::test]
async fn the_response_never_shows_the_endpoints_query_or_fragment() {
    const QUERY_SECRET: &str = "q-secret-never-in-a-response";
    const FRAGMENT_SECRET: &str = "f-secret-never-in-a-response";
    let upstream = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/pc"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(
                r#"{{"object":"list","data":[
                  {{"id":"{MAIN_MODEL}","object":"model","type":"llm","state":"loaded",
                    "max_context_length":262144,"loaded_context_length":{MAIN_TOKENS}}}]}}"#
            ),
            "application/json",
        ))
        .mount(&upstream)
        .await;
    let host = upstream
        .uri()
        .strip_prefix("http://")
        .expect("モックサーバの綴りが http であること")
        .to_owned();
    let written = format!(
        "http://{ENDPOINT_USER}:{ENDPOINT_PASSWORD}@{host}/pc?token={QUERY_SECRET}#{FRAGMENT_SECRET}"
    );

    let server = TestServer::start(&[
        (ENV_LLM_ENDPOINT, &written),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
    ])
    .await;

    let (text, body) = server.upstream_report().await;

    // 前提: その宛先で実際に往復していること。
    assert_eq!(
        body["main"]["queried_context_tokens"],
        json!(MAIN_TOKENS),
        "前提: クエリつきの宛先でも上流に届き、その往復から窓を読んでいること: {text}"
    );

    // 本題: ユーザー情報は `***@` に伏せ、クエリとフラグメントは落とす。ホスト・ポート・経路は残る。
    assert_eq!(
        body["main"]["endpoint"],
        json!(format!("http://***@{host}/pc")),
        "ユーザー情報を伏せ、クエリとフラグメントを落とすこと: {text}"
    );
    for secret in [
        ENDPOINT_USER,
        ENDPOINT_PASSWORD,
        QUERY_SECRET,
        FRAGMENT_SECRET,
    ] {
        assert!(
            !text.contains(secret),
            "宛先の秘密が応答本文に出ないこと（{secret}）: {text}"
        );
    }

    assert_no_internals(&server, &text);
    server.stop().await;
}

/// **応答は、いまの観測を返す**（起動時の写しではない / ADR-0020）。
///
/// # なぜ要るのか
///
/// 上流が停止しても**予算は現在値を保つ**（`context::refresh_once`）ので、
/// **観測が「答えなし」に落ちることだけが、外から見える唯一の手掛かり**である。
/// ハンドラが起動時の値を写し取っていると、**止まった上流を永久に「生きている」と言う。**
///
/// # 反対の面は1バイトも動かない
///
/// 引き直しは**面ごとに別の往復**なので、抽出が落ちても本文の観測は動かない。
/// **本文の面を丸ごと突き合わせる**ことで、片方の失敗がもう片方を潰す形も赤になる。
#[tokio::test]
async fn the_response_follows_the_refreshed_observation() {
    let main_upstream = upstream_reporting(MAIN_MODEL, MAIN_TOKENS).await;
    let sub_upstream = upstream_reporting(SUB_MODEL, SUB_TOKENS).await;

    let server = TestServer::start(&[
        (ENV_LLM_ENDPOINT, &main_upstream.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub_upstream.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
    ])
    .await;

    let (before_text, before) = server.upstream_report().await;
    assert_eq!(
        before["sub"]["answered_by"],
        json!("native-api"),
        "前提: 抽出の上流は一度は答えていること: {before_text}"
    );
    assert_eq!(
        before["sub"]["answered_name"],
        json!(SUB_MODEL),
        "前提: 一度は名前を名乗っていること: {before_text}"
    );

    // **引き直しが「抽出の上流が答えなくなった」を書いたのと同じ効果。**
    server
        .upstream
        .refresh_for_tests(ModelFace::Sub, None, None, None, None);

    let (after_text, after) = server.upstream_report().await;

    assert_eq!(
        after["sub"]["answered_by"],
        Value::Null,
        "止まった上流は「答えなし」になること: {after_text}"
    );
    assert_eq!(
        after["sub"]["queried_context_tokens"],
        Value::Null,
        "申告された窓も「読めなかった」になること: {after_text}"
    );
    assert_eq!(
        after["sub"]["answered_name"],
        Value::Null,
        "止まった上流は名前も名乗らないこと: {after_text}"
    );
    assert_eq!(
        after["sub"]["measured"],
        measured_json(judge_face(ModelFace::Sub, SUB_MODEL, None)),
        "聞けなかった面は全行を「聞けなかった」に数えること: {after_text}"
    );
    assert_ne!(
        before["sub"], after["sub"],
        "引き直しの結果が応答に出ていること: {after_text}"
    );

    // **反対の面は1バイトも動かない。**
    assert_eq!(
        before["main"], after["main"],
        "本文の面は触らないこと: {after_text}"
    );

    server.stop().await;
}

/// **生えているのは `GET` 1本だけ。「変える」導線は無い。**
///
/// **2026-09-07 に利用者が決めている** ——「モデルは見えるだけでよい。
/// personacore はプロセスを起こさない。選ばせもしない」（ADR-0039 §却下した選択肢）。
#[tokio::test]
async fn only_get_is_exposed_for_the_upstream_path() {
    let upstream = upstream_reporting(MAIN_MODEL, MAIN_TOKENS).await;
    let server = TestServer::start(&[
        (ENV_LLM_ENDPOINT, &upstream.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
    ])
    .await;

    let not_found = json!({
        "error": {
            "message": "指定されたエンドポイントは存在しません",
            "type": "invalid_request_error",
        }
    });
    let method_not_allowed = json!({
        "error": {
            "message": "このエンドポイントは指定されたメソッドを受け付けません",
            "type": "invalid_request_error",
        }
    });

    // **1つの面を指すパスも、下位の資源も生えていない。**
    for path in [
        "/admin/upstream/main",
        "/admin/upstream/sub",
        "/admin/upstreams",
        "/admin/upstream/model",
    ] {
        for method in [Method::GET, Method::POST, Method::PUT, Method::DELETE] {
            let (status, text, body) = server.send(method.clone(), path).await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "{method} {path} が生えていないこと: {text}"
            );
            assert_eq!(body, not_found, "{method} {path}");
        }
    }

    // **この入口が受けるのは `GET` だけ**（405）。
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        let (status, text, body) = server.send(method.clone(), UPSTREAM_PATH).await;
        assert_eq!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} が生えていないこと: {text}"
        );
        assert_eq!(body, method_not_allowed, "{method}");
    }

    server.stop().await;
}

/// **`/v1/models` は触っていない**（ADR-0004: あれはキャラクターの一覧）。
///
/// 上流のモデルの綴りをあちらへ混ぜると、**`model` に書ける綴りとそうでない綴りが
/// 同じ一覧に並ぶ。** キャラクターが1件も無ければ**空**である。
#[tokio::test]
async fn the_upstream_models_never_appear_in_the_openai_model_list() {
    let main_upstream = upstream_reporting(MAIN_MODEL, MAIN_TOKENS).await;
    let sub_upstream = upstream_reporting(SUB_MODEL, SUB_TOKENS).await;

    let server = TestServer::start(&[
        (ENV_LLM_ENDPOINT, &main_upstream.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub_upstream.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
    ])
    .await;

    // 前提: 上流の観測は現に読める（＝綴りはサーバの手元にある）。
    let (report_text, _) = server.upstream_report().await;
    for model in [MAIN_MODEL, SUB_MODEL] {
        assert!(
            report_text.contains(model),
            "前提: 上流の綴りはこちらの入口に出ていること（{model}）: {report_text}"
        );
    }

    let (status, text, body) = server.send(Method::GET, "/v1/models").await;
    assert_eq!(status, StatusCode::OK, "一覧を読めること: {text}");
    assert_eq!(
        body,
        json!({"object": "list", "data": []}),
        "キャラクターが居なければ空であること: {text}"
    );

    server.stop().await;
}
