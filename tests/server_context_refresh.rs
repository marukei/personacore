//! **コンテキスト長の定期的な引き直し**（ADR-0020 / 作業単位 11c-4）が
//! **本番の入口（`App::bind` → `App::serve`）に配線されている**ことの検証。
//!
//! # なぜ専用のテストバイナリなのか
//!
//! 見たいのは「背景タスクが実際に走り出したか」で、それはサーバのタスク側で起きる。
//! スレッドローカルな `with_default` では捕まらず `set_global_default` が要るが、
//! **それはプロセスにつき1回しか呼べない。** `tests/server_chat.rs` と
//! `tests/server_chat_stream.rs` は既に1本ずつ使い切っているので、
//! cargo が `tests/*.rs` ごとに別プロセスにすることを使って観測窓を1つ増やす
//! （`tests/server_chat_parse_log.rs` と同じ手）。
//!
//! # 5分を実時間で待たない
//!
//! **間隔は 5 分の定数**（ADR-0020 §2）なので、「実際に引き直した」瞬間は
//! 本番の入口からは待てない。ここで見るのは**その手前まで**:
//!
//! | 見るもの | 何が壊れたら赤になるか |
//! |---|---|
//! | 背景タスクが走り出した1行 | `serve` から `PendingRefresh::spawn` が消える |
//! | その行の `interval_seconds` | 本番へ届く間隔が 5 分でなくなる |
//! | 起動時の照会が 1 回 | `bind` の照会が消える（実測で赤になる。`server::mod` の `bind` のコメント） |
//! | 明示設定のときに「行いません」の1行 | 明示設定でも引き直す（ADR-0020 §4 違反） |
//! | 明示設定のときに本文の面への照会が、走り出した直後の1回だけ（作業単位 B-66 で 0 回から変わった） | 明示設定でも間隔を待たずに繰り返す・照会を落とす（往復として観測する側） |
//!
//! **引き直しそのものの振る舞い**（変わったら差し替える・失敗したら現在値を保つ・
//! 間隔ごとに高々1回・落とせば止まる）は `server::context` の単体テストが
//! **短い間隔を与えて**見張っている。**走り出したタスクがどの器へ書くか**も同じ理由で
//! そちら（`the_spawned_task_writes_into_the_returned_budget`）——5分を待てないので
//! **本番の入口からは見られない。** **共有の予算をハンドラが読むこと**は
//! `tests/server_chat.rs` の `the_running_budget_is_shared_with_the_handlers`。

use std::time::{Duration, Instant};

use personacore::prompt::TokenBudget;
use personacore::server::{
    App, AppConfig, BindAddr, DEFAULT_LOG_FILTER, ENV_CONTEXT_TOKENS, ENV_LLM_ENDPOINT,
    ENV_MAIN_MODEL, ENV_SUB_MODEL, ServerError, assumed_sub_context_tokens_for_tests,
};
use tempfile::TempDir;

mod common;

use common::{SharedBuffer, has_field, install_global_log};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

/// 上流が申告するコンテキスト長。**既定値とも明示設定とも別の数**にすること。
const QUERIED_CONTEXT_TOKENS: u32 = 16_384;

/// 明示設定として与える値。**上流の申告値とも既定値とも別。**
const EXPLICIT_CONTEXT_TOKENS: u32 = 12_288;

/// **サブモデルの窓**（作業単位 B-7）。**本文用の申告値とも明示設定とも既定とも別。**
const SUB_TOKENS: u32 = 6_144;

/// ADR-0020 §2 の間隔（秒）。**ここを変えるなら ADR を先に変えること。**
const EXPECTED_INTERVAL_SECONDS: u64 = 300;

/// 背景タスクが走り出したことを告げる行の手掛かり。**全文一致にしない。**
const STARTED_MARKER: &str = "定期的な引き直しを開始します";

/// 明示設定があるので引き直さない、と告げる行の手掛かり。
const DISABLED_MARKER: &str = "定期的な引き直しを行いません";

/// **本文の予算を引き直す腕だけが出す欄**（作業単位 B-7）。
///
/// ログのバッファはプロセスで1つなので、「開始します」の行は
/// **2つのテストの分が混ざる。** 自分の行を名指しで拾うために使う。
///
/// **⚠ 欄なので、読むのは [`wait_for_field`] / [`has_field`]**（作業単位 B-21）。
const MAIN_REQUERIED_FIELD: &str = "requeries_main_budget=true";

/// **明示設定の腕だけが出す欄**（本文の予算は引き直さない）。
const MAIN_NOT_REQUERIED_FIELD: &str = "requeries_main_budget=false";

/// **どちらの腕でも出る欄**（抽出のサブモデルには明示設定が無い）。
const SUB_REQUERIED_FIELD: &str = "requeries_sub_context=true";

/// **`/v1/models` にだけ答える上流が申告する窓**（作業単位 G-2）。
///
/// **既定（8192）とも、抽出の落とし所（16384）とも、上の3つの値とも違う数**にすること。
/// **落とし所と実測値が偶然一致すると、区別できるのはログの文面だけになる**
/// （G-1 が実際に踏んだ）。ここは値でも文面でも区別が付く状態にしてある。
const LLAMA_CONTEXT_TOKENS: u32 = 20_480;

/// その上流が名乗る `id`（**gguf の絶対パス** / 設定中の綴りとは一致しない）。
const LLAMA_MODEL_ID: &str = "/models/Ministral-3-14B-Instruct-2512-Q4_K_M.gguf";

/// **`-a` / `--alias` で設定中の綴りを名乗る上流が申告する窓**（作業単位 G-2 の是正）。
///
/// **この腕だけの数**にすること——他の腕と揃うと、
/// **どちらの同定を通った行なのか**がログから読めなくなる。
const ALIASED_CONTEXT_TOKENS: u32 = 24_576;

/// **同じ一覧に並ぶ、別のモデルの窓。掴んではいけない方。**
///
/// **[`ALIASED_CONTEXT_TOKENS`] と必ず違えておくこと。**
/// 同値だと「先頭の1件を決め打ちする」形と区別が付かない。
const DECOY_CONTEXT_TOKENS: u32 = 3_072;

/// その別のモデルの `id`（埋め込み。**設定中の綴りとは一致しない**）。
const DECOY_MODEL_ID: &str = "text-embedding-nomic-embed-text-v1.5";

/// **本文の予算が上流の申告値になった**ことを告げる行の手掛かり。
const USING_UPSTREAM_MARKER: &str = "上流の実コンテキスト長を使います";

/// **2つ目の読み手（OpenAI 互換）が読めた**ことを告げる行の手掛かり。
const COMPAT_READ_MARKER: &str = "OpenAI 互換のモデル一覧から実効コンテキスト長を読みました";

/// **抽出の上限が上流の申告値になった**ことを告げる行の手掛かり。
///
/// **落とし所へ落ちた行とは文面が違う**（「落とし所を使います」）。
/// G-1 では**値が偶然一致していた**ので、この文面だけが区別の手段だった。
const SUB_USING_UPSTREAM_MARKER: &str = "抽出はサブモデルの実コンテキスト長を使います";

/// ログを待つ上限。超えたら**落とす**（緑のまま何も測らない形にしない）。
const WAIT_LIMIT: Duration = Duration::from_secs(20);

/// 待ちの刻み。
const POLL_STEP: Duration = Duration::from_millis(10);

/// 実機と同じ形の `/api/v0/models` の応答本文。
const NATIVE_MODELS_BODY: &str = r#"{
  "object": "list",
  "data": [
    {"id": "PLACEHOLDER", "object": "model", "type": "llm", "state": "loaded",
     "max_context_length": 262144, "loaded_context_length": LOADED_CONTEXT}
  ]
}"#;

/// プロセスで1つだけのログバッファ。**`set_global_default` はここで1回だけ。**
///
/// 並行して走るもう一方のテストの出力も同じバッファに混ざるので、
/// **主張は「自分だけが出す文言」に対する肯定側だけ**にしてある
/// （否定側は相手の行で満たされたり壊れたりしうる）。
///
/// **⚠ 器も差し込みも、もう複製ではない**（作業単位 B-3）。
/// [`common::install_global_log`] が `src/testlog/assert.rs` の1本を読み、
/// **`OnceLock` は取り込んだバイナリごとに別の静的変数**になるので
/// 「このバイナリで1回だけ」はそちらが保証する。
/// **ここで写しを作り直さないこと**（押さえの見張り番はあちらにしか無い）。
fn shared_log() -> &'static SharedBuffer {
    install_global_log(DEFAULT_LOG_FILTER)
}

/// 述語を満たす行が出るまで待ち、その行を返す。**出なければ落とす。**
///
/// # 拾い方は2つに分けてある（作業単位 B-21）
///
/// **欄で拾うのか、文言で拾うのかは別の主張**なので、入口を分ける
/// ——[`wait_for_field`]（`キー=値`）と [`wait_for_message`]（地の文の断片）。
///
/// **⚠ ここを1本に戻して素の `contains` にしないこと。**
/// 欄を `contains` で拾うと、**キーの側が伸びた別の欄**（`x_main_model=…` /
/// `checked_sub_model=…`）に当たる。**E-2 が実際に踏んだ**——欄を1つ足しただけで
/// このファイルの肯定側と否定側が同時に狂い、欄名の改名で回避された
/// （`src/testlog/assert.rs` の [`has_field`] の doc の表がその正典）。
async fn wait_for(what: &str, matches: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + WAIT_LIMIT;
    loop {
        let logged = shared_log().contents();
        if let Some(found) = logged.lines().find(|line| matches(line)) {
            return found.to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "{what} の行が出なかった: {logged:?}"
        );
        tokio::time::sleep(POLL_STEP).await;
    }
}

/// **欄**（`キー=値`）を**空白区切りのトークンとして丸ごと**持つ行を待つ。
///
/// 行を選ぶのも主張の一種（**どの腕の行を読むか**）なので、
/// **拾う段階から [`has_field`] を通す。**
async fn wait_for_field(field: &str) -> String {
    wait_for(&format!("欄 {field:?} を持つ"), |line| {
        has_field(line, field)
    })
    .await
}

/// **地の文（メッセージ）の断片**を含む行を待つ。**欄には使わないこと。**
///
/// こちらが素の部分一致でよいのは、探すのが `キー=値` ではなく
/// **`tracing` のメッセージそのもの**だから（`キー=` の形をしていないので、
/// キーが伸びて別の欄に当たる、という壊れ方が起こらない）。
async fn wait_for_message(marker: &str) -> String {
    wait_for(&format!("文言 {marker:?} を含む"), |line| {
        line.contains(marker)
    })
    .await
}

/// `/api/v0/models` に実機と同じ形で答える上流。**実機には繋がない。**
async fn upstream_reporting(loaded: u32) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                NATIVE_MODELS_BODY
                    .replace("PLACEHOLDER", personacore::llm::DEFAULT_MAIN_MODEL)
                    .replace("LOADED_CONTEXT", &loaded.to_string()),
                "application/json",
            ),
        )
        .mount(&server)
        .await;
    server
}

/// **複数のモデルを、それぞれ別の窓で申告する上流**（作業単位 B-7 / レビュー指摘2）。
///
/// **同じ窓にしないこと。** 同値にすると、どちらの行を読んだのかが区別できない。
async fn upstream_reporting_models(models: &[(&str, u32)]) -> MockServer {
    let entries: Vec<String> = models
        .iter()
        .map(|(id, loaded)| {
            NATIVE_MODELS_BODY
                .replace("PLACEHOLDER", id)
                .replace("LOADED_CONTEXT", &loaded.to_string())
        })
        .collect();
    // 1件ずつ組んだ本文から `data` の中身だけを取り出して連結する
    // （`NATIVE_MODELS_BODY` の綴りをここに写さないため）。
    let inner: Vec<String> = entries
        .iter()
        .map(|body| {
            let start = body.find('[').expect("`data` の配列が始まること");
            let end = body.rfind(']').expect("`data` の配列が終わること");
            body[start + 1..end].to_owned()
        })
        .collect();
    let body = format!(r#"{{"object":"list","data":[{}]}}"#, inner.join(","));

    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;
    server
}

/// **`/api/v0/models` を持たず、OpenAI 互換の `/v1/models` にだけ答える上流**
/// （作業単位 G-2 / 実機の `llama-server` と同じ形）。
///
/// **`id` は gguf の絶対パス**で、設定中のモデルの綴りとは一致しない。
/// **`n_ctx`（いま載っている窓）と `n_ctx_train`（訓練時の上限）を必ず違えておくこと。**
async fn upstream_speaking_only_openai(n_ctx: u32) -> MockServer {
    let body = format!(
        r#"{{"object":"list","data":[
            {{"id":"{LLAMA_MODEL_ID}","object":"model","owned_by":"llamacpp",
              "meta":{{"n_vocab":131072,"n_ctx":{n_ctx},"n_ctx_train":262144}}}}]}}"#
    );

    let server = MockServer::start().await;
    // **1つ目は 404。** 実機の `llama-server` はここを持たない。
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

/// **`/api/v0/models` を持たず、`/v1/models` で「設定中の綴り」を名乗る上流**
/// （作業単位 G-2 の是正 / `llama-server -a ministral-3-14b-instruct-2512`）。
///
/// # これは架空の上流ではない
///
/// `llama-server` の `-a` / `--alias` は `/v1/models` の `id` を差し替える。
/// **別名を設定中のモデルに合わせて立てた上流**がこの形になり、
/// そこでは同定の根拠が「一覧が1件」ではなく**綴りの一致**になる。
///
/// # 一覧は2件で、一致する行を**後ろ**に置く
///
/// 1件だけにすると、**綴りの一致を見なくても「一覧が1件」の規則で同じ値が出る**
/// ——2つの段が区別できない。隣に別のモデル（[`DECOY_MODEL_ID`]）を置くと、
/// **綴りで引けなければ「2件以上あって一致なし」で読まない**ので、
/// 値が届いたこと自体が「1段目を通った」の証拠になる。
async fn upstream_naming_our_model(n_ctx: u32, decoy: u32) -> MockServer {
    let body = format!(
        r#"{{"object":"list","data":[
            {{"id":"{DECOY_MODEL_ID}","object":"model",
              "meta":{{"n_ctx":{decoy},"n_ctx_train":262144}}}},
            {{"id":"{}","object":"model","owned_by":"llamacpp",
              "meta":{{"n_vocab":131072,"n_ctx":{n_ctx},"n_ctx_train":262144}}}}]}}"#,
        personacore::llm::DEFAULT_MAIN_MODEL
    );

    let server = MockServer::start().await;
    // **1つ目は 404。** 実機の `llama-server` はここを持たない。
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

/// そのパスを叩かれた回数。
async fn queries_to_path(server: &MockServer, wanted: &str) -> usize {
    server.received_requests().await.map_or(0, |requests| {
        requests
            .iter()
            .filter(|request| request.url.path() == wanted)
            .count()
    })
}

/// `/api/v0/models` を叩いた回数。
async fn queries_to(server: &MockServer) -> usize {
    server.received_requests().await.map_or(0, |requests| {
        requests
            .iter()
            .filter(|request| request.url.path() == "/api/v0/models")
            .count()
    })
}

/// **`App::bind`（本番の入口）で起動した実サーバ。**
///
/// モックは注入しない。上流は必ずローカルのモックサーバへ向ける。
struct BoundServer {
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl BoundServer {
    /// 設定を与えて起動する。**プロセスの環境変数には触らない。**
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

        // **`bind_with_client` ではない。** ここが本番と同じ入口であることが要点。
        let app = App::bind(&config).await.expect("待ち受けを開始できること");

        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });

        Self {
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

/// **本番の入口を通ると、引き直しの背景タスクが実際に走り出す**（ADR-0020 §1）。
///
/// `serve` から `PendingRefresh::spawn` を消すと、この行が出ずに赤になる。
/// **単体テストは短い間隔を直に与えて回すので、配線が消えても緑のまま。**
///
/// # 間隔も同じ行で見る
///
/// 行は**タスクの中**で出しているので、`interval_seconds` は
/// **実際にそのタスクが待つ値**そのもの。ここが 300 でなくなれば、
/// 「ずれたまま気づかない窓」が ADR の想定と違っているということ。
///
/// # **自分の行を名指しで拾う**（作業単位 B-7）
///
/// ログのバッファはプロセスで1つで、**もう一方のテストも「開始します」を出す**
/// （明示設定でも抽出側の引き直しは走るため）。`STARTED_MARKER` だけで拾うと
/// **相手の行を見て緑になりうる**（CLAUDE.md「その行が出た」と「その行がこう言っている」）。
/// **この腕だけが出す欄**（`requeries_main_budget=true`）で拾う。
///
/// **⚠ 欄は「トークンとして丸ごと」で拾い、丸ごとで見る**（作業単位 B-21）。
/// 素の `contains` だと**キーが伸びた別の欄**（`x_requeries_main_budget=true`）でも
/// 拾えてしまい、**値が伸びた別の値**（`interval_seconds=3000`）も同じものと読む。
#[tokio::test]
async fn the_periodic_requery_is_wired_into_the_real_bind() {
    let _ = shared_log();

    let upstream = upstream_reporting(QUERIED_CONTEXT_TOKENS).await;
    let server = BoundServer::start(&[(ENV_LLM_ENDPOINT, &upstream.uri())]).await;

    let line = wait_for_field(MAIN_REQUERIED_FIELD).await;
    assert!(
        line.contains(STARTED_MARKER),
        "引き直しを始めた行であること: {line:?}"
    );
    // **欄はトークンとして丸ごと見る**（作業単位 B-21）。素の `contains` だと
    // `interval_seconds=3000` が「300 だった」を満たしてしまう。
    assert!(
        has_field(
            &line,
            &format!("interval_seconds={EXPECTED_INTERVAL_SECONDS}")
        ),
        "間隔は 5 分（{EXPECTED_INTERVAL_SECONDS} 秒）であること: {line:?}"
    );
    assert!(
        has_field(&line, SUB_REQUERIED_FIELD),
        "抽出側（サブモデル）も引き直すこと: {line:?}"
    );

    // 起動時の照会（ADR-0013 第2層 ＋ 作業単位 B-7）は**モデルごとに1回ずつ。**
    //
    // **⚠ 2 は「本文用とサブ用で1回ずつ」。** 以前は 1 だった
    // （抽出の上限が定数だった頃）。**どちらか片方が消えると 1 になって赤になる。**
    // **モデルの綴りは同じ GET には現れない**ので、どちらが消えたのかは
    // `tests/server_memory_extraction.rs` の
    // `the_extraction_limit_follows_the_sub_models_context_through_the_real_bind` が見る。
    assert_eq!(
        queries_to(&upstream).await,
        2,
        "起動時の照会は本文用とサブ用で1回ずつであること（引き直しの間隔はまだ経っていない）"
    );

    server.stop().await;
}

/// **`/api/v0/models` を持たない上流でも、実効コンテキスト長が本番の入口まで届く**
/// （作業単位 G-2 / 実機の `llama-server`）。
///
/// # これが作業単位の存在理由
///
/// G-1 の実測では、`llama-server` を `-c` に何を渡して立てても
/// personacore は**落とし所を使い続けた**（本文 8192 / 抽出 16384）。
/// 上流は答えられるのに、**こちらが1つの綴りしか聞いていなかった。**
///
/// # 値と文面の両方で見る
///
/// **`sub_context_length=16384` は、落とし所と LM Studio の実測値で同じ値だった**
/// ——G-1 で区別が付いたのは**ログの文面だけ**（「落とし所を使います」/
/// 「実コンテキスト長を使います」）。
///
/// ここでは [`LLAMA_CONTEXT_TOKENS`] を**既定とも落とし所とも他の腕とも違う数**にして、
/// **同じ行の中で文面と値を両方見る。** どちらかだけだと:
///
/// * 値だけ → 落とし所と一致する日に、区別が消える。
/// * 文面だけ → 「実コンテキスト長を使います」と言いながら既定を入れる形が通る。
///
/// # 往復の数も見る
///
/// **2つ目の読み手は、必要なときにモデルごとに1回だけ。**
/// 無条件に投げる形（LM Studio でも毎回2回）も、投げすぎる形もここで落ちる。
#[tokio::test]
async fn an_upstream_with_only_the_openai_model_list_still_sets_both_windows() {
    let _ = shared_log();

    assert_ne!(
        LLAMA_CONTEXT_TOKENS,
        TokenBudget::DEFAULT_CONTEXT_TOKENS,
        "前提: 上流の申告値は既定値と別の数であること（引いたのか既定なのかを区別する）"
    );
    assert_ne!(
        LLAMA_CONTEXT_TOKENS,
        assumed_sub_context_tokens_for_tests(),
        "前提: 抽出の落とし所（SubContext::FALLBACK）とも別の数であること"
    );
    assert_ne!(
        LLAMA_CONTEXT_TOKENS, QUERIED_CONTEXT_TOKENS,
        "前提: 他の腕が申告する値とも別の数であること（ログのバッファはプロセスで1つ）"
    );

    let upstream = upstream_speaking_only_openai(LLAMA_CONTEXT_TOKENS).await;
    let server = BoundServer::start(&[(ENV_LLM_ENDPOINT, &upstream.uri())]).await;

    // **本文の予算。** 文面と値を**同じ行の中で**見る。
    let line = wait_for(
        "本文の予算が上流の申告値になった行",
        |line| {
            line.contains(USING_UPSTREAM_MARKER)
                && has_field(line, &format!("context_tokens={LLAMA_CONTEXT_TOKENS}"))
        },
    )
    .await;
    assert!(
        !line.contains("既定値を使います"),
        "既定値へ落ちた行ではないこと: {line:?}"
    );

    // **抽出の上限。** こちらは落とし所と値が一致しうる側なので、文面が効く。
    let sub = wait_for(
        "抽出の上限が上流の申告値になった行",
        |line| {
            line.contains(SUB_USING_UPSTREAM_MARKER)
                && has_field(line, &format!("sub_context_length={LLAMA_CONTEXT_TOKENS}"))
        },
    )
    .await;
    assert!(
        has_field(&sub, &format!("limit_tokens={}", LLAMA_CONTEXT_TOKENS / 2)),
        "上限は申告値の半分であること: {sub:?}"
    );

    // **2つ目の読み手が「どの行を、どう同定したか」を名乗ること。**
    //
    // **`identified_by` はログにしか出ない。** そして
    // **「綴りが一致した」と「一覧が1件しか無かった」は危険の度合いが違う**
    // ——後者は推論なので、上流が2件目を差し出した日に外れうる。
    // **読み手がそれを見分けられなければ、名乗っている意味が無い。**
    let read = wait_for("2つ目の読み手が名乗った行", |line| {
        line.contains(COMPAT_READ_MARKER)
            && has_field(line, &format!("context_length={LLAMA_CONTEXT_TOKENS}"))
    })
    .await;
    assert!(
        has_field(&read, r#"identified_by="sole-entry""#),
        "同定の根拠は「一覧が1件」であること（綴りは一致していない）: {read:?}"
    );
    assert!(
        has_field(&read, &format!("upstream_model_id={LLAMA_MODEL_ID}")),
        "実際に読んだ行の id を同じ行に出すこと: {read:?}"
    );

    // **往復はモデルごとに、1つ目1回 ＋ 2つ目1回。**
    assert_eq!(
        queries_to(&upstream).await,
        2,
        "1つ目（独自 API）は本文用とサブ用で1回ずつであること"
    );
    assert_eq!(
        queries_to_path(&upstream, "/v1/models").await,
        2,
        "2つ目（OpenAI 互換）も本文用とサブ用で1回ずつであること（増やしすぎない）"
    );

    server.stop().await;
}

/// **綴りが一致する上流では、同定の根拠が「綴りの一致」になる**
/// （作業単位 G-2 の是正 / `llama-server -a <設定中のモデル>`）。
///
/// # なぜ本番の入口に要るのか
///
/// **2つの同定の根拠は危険の度合いが違う**（一致は上流が名乗った綴り / 1件は推論）。
/// `sole-entry` の側は
/// [`an_upstream_with_only_the_openai_model_list_still_sets_both_windows`] が見ていたが、
/// **`exact-id` の側は本番の入口に1本も無かった**（G-2 のレビューが指摘）。
///
/// # 一致した行の窓が届くこと（隣の値ではない）
///
/// 一覧は2件で、**一致する行は後ろ**（[`upstream_naming_our_model`]）。
/// **「常に先頭の1件」も「一覧が1件の規則を先に効かせる」も、ここで別の値になるか、
/// 何も読めなくなる。**
#[tokio::test]
async fn an_upstream_naming_our_model_is_identified_by_its_id() {
    let _ = shared_log();

    assert_ne!(
        ALIASED_CONTEXT_TOKENS, DECOY_CONTEXT_TOKENS,
        "前提: 一致する行と隣の行は別の窓を申告すること"
    );
    assert_ne!(
        ALIASED_CONTEXT_TOKENS,
        TokenBudget::DEFAULT_CONTEXT_TOKENS,
        "前提: 既定値とも別の数であること（引いたのか既定なのかを区別する）"
    );
    assert_ne!(
        ALIASED_CONTEXT_TOKENS,
        assumed_sub_context_tokens_for_tests(),
        "前提: 抽出の落とし所（SubContext::FALLBACK）とも別の数であること"
    );
    assert_ne!(
        ALIASED_CONTEXT_TOKENS, LLAMA_CONTEXT_TOKENS,
        "前提: `sole-entry` の腕とも別の数であること（ログのバッファはプロセスで1つ）"
    );

    let upstream = upstream_naming_our_model(ALIASED_CONTEXT_TOKENS, DECOY_CONTEXT_TOKENS).await;
    let server = BoundServer::start(&[(ENV_LLM_ENDPOINT, &upstream.uri())]).await;

    // **2つ目の読み手が、綴りの一致で同定したと名乗ること。**
    let read = wait_for(
        "2つ目の読み手が綴りの一致で名乗った行",
        |line| {
            line.contains(COMPAT_READ_MARKER)
                && has_field(line, &format!("context_length={ALIASED_CONTEXT_TOKENS}"))
        },
    )
    .await;
    assert!(
        has_field(&read, r#"identified_by="exact-id""#),
        "同定の根拠は綴りの一致であること（推論の `sole-entry` ではない）: {read:?}"
    );
    assert!(
        has_field(
            &read,
            &format!("upstream_model_id={}", personacore::llm::DEFAULT_MAIN_MODEL)
        ),
        "実際に読んだ行の id を同じ行に出すこと: {read:?}"
    );

    // **本文の予算まで届くこと。** 隣の行の窓（DECOY_CONTEXT_TOKENS）ではない。
    let line = wait_for(
        "本文の予算が一致した行の申告値になった行",
        |line| {
            line.contains(USING_UPSTREAM_MARKER)
                && has_field(line, &format!("context_tokens={ALIASED_CONTEXT_TOKENS}"))
        },
    )
    .await;
    assert!(
        !line.contains("既定値を使います"),
        "既定値へ落ちた行ではないこと: {line:?}"
    );

    // **抽出の上限も同じ行から来ること**（サブモデルの綴りも一致する）。
    let sub = wait_for(
        "抽出の上限が一致した行の申告値になった行",
        |line| {
            line.contains(SUB_USING_UPSTREAM_MARKER)
                && has_field(
                    line,
                    &format!("sub_context_length={ALIASED_CONTEXT_TOKENS}"),
                )
        },
    )
    .await;
    assert!(
        has_field(
            &sub,
            &format!("limit_tokens={}", ALIASED_CONTEXT_TOKENS / 2)
        ),
        "上限は申告値の半分であること: {sub:?}"
    );

    server.stop().await;
}

/// **明示設定があるときは、本文の予算を引き直さない**（ADR-0020 §4）。本番の入口で見る。
///
/// 引いても予算は変わらない以上、引くと
/// **「変わった」と言いながら予算は変えない**惑わせる記録になる。
///
/// 見るのは3つ。**どれか1つでは足りない。**
///
/// * **照会が2回飛ぶこと** —— 往復として観測する側。
///   **1回は抽出のサブモデル**の側で（作業単位 B-7）、そちらには明示設定が無い
///   （`context::resolve_sub_context` の doc）。**もう1回は本文の面の、走り出した直後の観測のための照会**
///   （作業単位 B-66 / Issue origin#94 —— 書いた面にも、生きているか・鍵が通るかを聞く。予算は動かさない）。
///   間隔（5分）はテスト中に経たないので、3回目は飛ばない。
///
///   > **⚠ 作業単位 B-66 で主張が変わった**（司令塔・利用者の決定）。着手前は「照会は1回だけ」
///   > （本文の面へは1度も聞かない）を見ていた。**本文の面へ聞いても予算が動かないこと**は
///   > `tests/server_upstream_check.rs::a_written_face_is_checked_after_the_bind_without_moving_its_budget` が見る。
/// * **「行いません」の1行** —— 本文の側の引き直しを立てたかどうかを直接観測する側。
///   `refresh_source` が明示設定を無視すると、この行が出ずに赤になる。
/// * **「開始します」の行の欄** —— **走っているのはサブ側だけ**であること。
///   `requeries_main_budget=true` になっていたら、明示設定が無視されている。
///
/// # ⚠ **「開始します」が出ること自体は正しい**（作業単位 B-7 で変わった）
///
/// 以前はこの設定で背景タスクごと立たなかったが、
/// **抽出の上限は明示設定の対象ではない**ので、サブ側だけが走る。
#[tokio::test]
async fn an_explicit_setting_disables_the_requery_on_the_real_bind() {
    let _ = shared_log();

    assert_ne!(
        EXPLICIT_CONTEXT_TOKENS,
        TokenBudget::DEFAULT_CONTEXT_TOKENS,
        "前提: 明示設定は既定値と別の数であること"
    );
    assert_ne!(
        EXPLICIT_CONTEXT_TOKENS, QUERIED_CONTEXT_TOKENS,
        "前提: 明示設定は上流の申告値と別の数であること"
    );

    let upstream = upstream_reporting(QUERIED_CONTEXT_TOKENS).await;
    let configured = EXPLICIT_CONTEXT_TOKENS.to_string();
    let server = BoundServer::start(&[
        (ENV_CONTEXT_TOKENS, &configured),
        (ENV_LLM_ENDPOINT, &upstream.uri()),
    ])
    .await;

    // **こちらは欄ではなく地の文**なので、拾うのも部分一致でよい（[`wait_for_message`]）。
    let line = wait_for_message(DISABLED_MARKER).await;
    assert!(
        !line.contains(STARTED_MARKER),
        "引き直しを始めたとは言わないこと: {line:?}"
    );

    // **走り出したタスクは、サブ側だけを引く。**
    // **この腕だけが出す欄**（`requeries_main_budget=false`）で自分の行を拾う
    // ——もう一方のテストの「開始します」と同じバッファに混ざるため。
    let started = wait_for_field(MAIN_NOT_REQUERIED_FIELD).await;
    assert!(
        started.contains(STARTED_MARKER),
        "引き直しを始めた行であること: {started:?}"
    );
    assert!(
        has_field(&started, SUB_REQUERIED_FIELD),
        "抽出側（サブモデル）は明示設定で止まらないこと: {started:?}"
    );

    // **走り出した直後の照会を待ってから数える**（作業単位 B-66）。
    let deadline = Instant::now() + WAIT_LIMIT;
    while queries_to(&upstream).await < 2 {
        assert!(
            Instant::now() < deadline,
            "書いた本文の面への、走り出した直後の照会が飛ばなかった（いま {} 回）",
            queries_to(&upstream).await
        );
        tokio::time::sleep(POLL_STEP).await;
    }
    tokio::time::sleep(POLL_STEP * 20).await;
    assert_eq!(
        queries_to(&upstream).await,
        2,
        "明示設定があるときに飛ぶ照会は、抽出の起動時の1回と、本文の走り出した直後の1回だけであること"
    );

    server.stop().await;
}

/// **2つの照会先が、それぞれ自分のモデルを引くつもりであることを、起動時に名乗る**
/// （作業単位 B-7 のレビュー指摘2 / 2026-08-27）。
///
/// # なぜログで見るのか（5分を待つのではなく）
///
/// **引き直しの照会先を本文モデルへ向ける書き換えは、スイートのどこからも観測できなかった。**
/// 実測（2026-08-27）:
///
/// | 書いてみた形 | 結果 |
/// |---|---|
/// | `let sub_refresh_source = Some(Arc::clone(&llm));` を足す | `cargo test` は緑・**clippy が `unused variable` で赤** |
/// | 分解束縛で `_` に捨ててから同じことをする | **clippy も通り 1554 全緑** |
///
/// 実害は **`PERSONACORE_SUB_MODEL` を分けた環境で、起動から5分後に静かに壊れる**
/// ——抽出の上限が本文モデルの窓の半分になり、`sub_context_length` の行は
/// もっともらしい数を出し続ける。
///
/// **待って確かめる必要は無い。** 引く相手は**起動時に決まっている事実**なので、
/// その1行を読めば足りる。
///
/// # 見るのは「同じ行の中で」2つの欄
///
/// * **サブ側の欄が、設定したサブモデルの綴りであること**（本文用でも `-` でもない）。
/// * **2つの欄が別物であること**（本文用とサブ用に別の綴りを与えてある）。
///
/// **出力全体に対して見ないこと。** このリポジトリは、
/// **別の行から答えを拾って全緑を通り抜けた実例を2件**持っている（CLAUDE.md）。
/// ここでは**もう一方のテストも「開始します」を出す**ので、なおさら効く。
#[tokio::test]
async fn the_two_requery_targets_name_their_own_models_on_the_real_bind() {
    /// 本文生成に指定するモデル。**既定ともサブ用とも別。**
    const MAIN: &str = "ministral-3-14b-reasoning-2512";
    /// 抽出に指定するモデル。**既定とも本文用とも別。**
    const SUB: &str = "ministral-3-3b-reasoning-2512";

    let _ = shared_log();

    assert_ne!(MAIN, SUB, "前提: 2つは別の綴りであること");
    assert_ne!(
        MAIN,
        personacore::llm::DEFAULT_MAIN_MODEL,
        "前提: 本文モデルは既定と別の綴りであること"
    );
    assert_ne!(
        SUB,
        personacore::llm::DEFAULT_SUB_MODEL,
        "前提: 抽出モデルは既定と別の綴りであること（既定どうしは今は同じ綴り）"
    );

    let upstream =
        upstream_reporting_models(&[(MAIN, QUERIED_CONTEXT_TOKENS), (SUB, SUB_TOKENS)]).await;
    let server = BoundServer::start(&[
        (ENV_LLM_ENDPOINT, &upstream.uri()),
        (ENV_MAIN_MODEL, MAIN),
        (ENV_SUB_MODEL, SUB),
    ])
    .await;

    // **この腕だけが出す欄で自分の行を拾う**（`sub_model` の綴りは他の腕に出ない）。
    let line = wait_for_field(&format!("sub_model={SUB}")).await;
    assert!(
        line.contains(STARTED_MARKER),
        "引き直しを始めた行であること: {line:?}"
    );
    assert!(
        has_field(&line, &format!("main_model={MAIN}")),
        "本文側の照会先は本文モデルを引くこと: {line:?}"
    );
    // **否定側も欄で見る**（作業単位 B-21）。素の `contains` だと
    // **`checked_sub_model=…` のような別の欄で誤爆する**（E-2 が実際に踏んだ）。
    //
    // **⚠ この2本が守っているのは「行のどこかに、もう1つ別の
    // `sub_model=本文モデル` / `main_model=サブモデル` を名乗るトークンが無い」ことだけ。**
    // **上の拾い方と肯定側が、それぞれの欄の値そのものは既に固定している**
    // （1行に `sub_model` は1つしか出ない）。**実測済み**——
    // 2つの照会先を入れ替える変異は、この2本ではなく**拾えないこと**で赤になる。
    assert!(
        !has_field(&line, &format!("sub_model={MAIN}")),
        "サブ側の照会先が本文モデルを引いていないこと: {line:?}"
    );
    assert!(
        !has_field(&line, &format!("main_model={SUB}")),
        "本文側の照会先がサブモデルを引いていないこと: {line:?}"
    );

    server.stop().await;
}
