//! **第2の上流のつまみ**（`PERSONACORE_SUB_LLM_ENDPOINT`）が
//! **本番の入口（`App::bind`）から上流まで届いている**ことの検証
//! （作業単位 G-3a / [ADR-0039](../docs/adr/0039-two-upstreams-for-two-jobs.md) §決定1）。
//!
//! **実際の LM Studio には繋がない**（docs/05-development.md）。上流は2種類、
//! **どちらもこのファイルが自分で立てる:**
//!
//! | 上流 | 見るもの |
//! |---|---|
//! | **`wiremock` を2つ** | **どちらの往復がどちらへ行くか**（宛先とモデルの帰属） |
//! | **接続を数える生の TCP を1つ**（[`CountingUpstream`]） | **2つのクライアントが接続プールを共有しているか** |
//!
//! **後者が要る理由は、前者では原理的に見えないから**——
//! **未設定の枝は宛先もモデルも同じ**なので、`wiremock` から見える往復は1バイトも変わらない。
//!
//! # なぜ本番の入口を通す必要があるのか
//!
//! **つまみは無料ではない。** 起動時に決めるものを増やすと、
//! **本番の入口を通るテストを足さない限り配線が観測対象から外れる**
//! （CLAUDE.md 4節「テスト専用の入口を作ると、本番の入口が観測対象から外れる」）。
//! **足すつまみ1つにつき、`App::bind` を通る見張り番を1本。** これがその1本。
//!
//! **`App::bind_with_clients`（注入の入口）は、この配線を1ミリも見ない**——
//! あちらは `Arc<dyn LlmClient>` を2つ受け取るだけで、
//! **どこへ繋ぐかを決める行そのものを通らない。**
//!
//! # ⚠ **既定で回してはいけない**（B-7 の穴）
//!
//! 2つのエンドポイントも2つのモデルの綴りも、**互いに違い、どれも既定ではない。**
//! 同値にすると「設定を読まずに本文の上流を使う」実装と区別が付かず、
//! **`assert_ne!` にも掛からない**（`llm.with_model` は base URL を共有するので、
//! **既定のままだと2つの経路が文字どおり同じ宛先になる**）。
//!
//! # 何を見ていないか
//!
//! * **綴りの読み方**（空文字列＝未設定 / 不正な値で起動を止める）は
//!   `server::config` のユニット（`an_empty_sub_llm_endpoint_reads_as_unset` /
//!   `a_malformed_sub_llm_endpoint_stops_the_startup`）の担当。
//! * **起動ログがどちらの上流かを名乗ること**（ADR-0039 §決定4）は
//!   `server::tests::the_startup_line_names_the_extraction_upstream_apart_from_the_body_one`
//!   の担当（**捕捉には `set_global_default` が要る**ので、ライブラリ側に置いてある）。
//! * **実測由来の定数の突き合わせ**（`measured`）が2つの上流でどうなるかは
//!   `tests/server_measured_model.rs` の
//!   `two_upstreams_are_each_judged_against_their_own_catalogue` と
//!   `one_upstream_named_twice_still_judges_the_body_face` の担当。

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::future::Either;

use personacore::domain::{DisplayName, Handle, Persona};
use personacore::llm::{DEFAULT_ENDPOINT, DEFAULT_MAIN_MODEL, DEFAULT_SUB_MODEL};
use personacore::server::{
    App, AppConfig, BackgroundProbe, BindAddr, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL,
    ENV_SUB_LLM_ENDPOINT, ENV_SUB_MODEL,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Barrier, Notify};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

mod common;

use common::fixture_scale;

/// 本文生成に指定するモデル。**既定とも抽出用とも別。**
const MAIN_MODEL: &str = "ministral-3-14b-reasoning-2512";

/// 抽出に指定するモデル。**既定とも本文用とも別。**
const SUB_MODEL: &str = "ministral-3-3b-reasoning-2512";

/// テストで使うキャラクターの識別子。**表示名とは別の値。**
const HANDLE: &str = "aoi";

/// 表示名。**識別子と同値にしない。**
const DISPLAY_NAME: &str = "葵";

/// 利用者の発言。
const USER_MESSAGE: &str = "ただいま";

/// 上流が返す本文（OpenAI 互換）。**抽出として読める行**を1件だけ含む。
const UPSTREAM_BODY: &str = r#"{
  "id": "chatcmpl-1",
  "object": "chat.completion",
  "created": 1,
  "model": "PLACEHOLDER",
  "choices": [{"index": 0, "message": {"role": "assistant", "content": "- 利用者: 山あいの町で育った"},
               "finish_reason": "stop"}]
}"#;

/// 上流が届くのを待つ上限。**超えたら落とす**（緑のまま何も測らない形にしない）。
const WAIT_LIMIT: Duration = Duration::from_secs(20);

/// 待ちの刻み。
const POLL_STEP: Duration = Duration::from_millis(10);

/// 「起きていないこと」を主張する前に置く猶予。
///
/// **肯定側の主張にこれを使わないこと**——肯定側は締切まで poll する
/// （`tests/server_memory_extraction.rs` の `SETTLE` の doc が権威）。
const SETTLE: Duration = Duration::from_millis(500);

/// **本文と抽出の両方を受けられる上流**を1つ立てる。
///
/// `/api/v0/models` は**そのモデルだけ**を持っていると答える
/// （コンテキスト長の照会が通る＝落とし所へ落ちない）。
async fn upstream_for(model: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            UPSTREAM_BODY.replace("PLACEHOLDER", model),
            "application/json",
        ))
        .mount(&server)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            format!(
                r#"{{"object":"list","data":[
                  {{"id":"{model}","object":"model","type":"llm","state":"loaded",
                    "max_context_length":262144,"loaded_context_length":16384}}]}}"#
            ),
            "application/json",
        ))
        .mount(&server)
        .await;
    server
}

/// その上流へ届いた `/v1/chat/completions` の本文（**照会は数えない**）。
///
/// **約束・決まりの提案の依頼（作業単位 A-12c-1）も数えない** —— ここが見ているのは本文と抽出の
/// 行き先で、提案は抽出の後ろの同じ裏方の列で毎往復走る。提案の依頼の行き先は
/// [`proposals_to`] で別に数える（サブの上流へだけ行くこと）。
async fn completions_to(server: &MockServer) -> Vec<Value> {
    all_completions_to(server)
        .await
        .into_iter()
        .filter(|request| !is_agreement_proposal(request))
        .collect()
}

/// その上流へ届いた、**約束・決まりの提案の依頼**の本数（作業単位 A-12c-1）。
async fn proposals_to(server: &MockServer) -> usize {
    all_completions_to(server)
        .await
        .iter()
        .filter(|request| is_agreement_proposal(request))
        .count()
}

async fn all_completions_to(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|request| request.url.path() == "/v1/chat/completions")
        .map(|request| serde_json::from_slice(&request.body).expect("JSON として読めること"))
        .collect()
}

/// **約束・決まりの提案の依頼か**（検出 `jd-v3` と確かめ `c1-v1` の書き方の行 —— `prompt` の定数を呼ばない）。
fn is_agreement_proposal(request: &Value) -> bool {
    let messages = request["messages"].to_string();
    [
        "抜｜引用｜取り決め｜守る側｜中身",
        "点検｜番号｜行い｜分類｜守る側｜判定｜中身",
    ]
    .iter()
    .any(|marker| messages.contains(marker))
}

/// その上流へ届いた `/api/v0/models` の回数。
async fn context_queries_to(server: &MockServer) -> usize {
    server.received_requests().await.map_or(0, |requests| {
        requests
            .iter()
            .filter(|request| request.url.path() == "/api/v0/models")
            .count()
    })
}

/// 非ストリーミングの依頼の本文。
///
/// **`handle` を受け取る。** 接続プールの見張り番
/// （`the_unset_extraction_client_shares_the_body_clients_connection_pool`）は
/// **2人のキャラクターへ同時に**投げる——同じ相手へ2本重ねると、
/// **セッションへの書き込みが取り合いになる**ので、測りたいものが接続の話でなくなる。
fn non_streaming_body(handle: &str, message: &str) -> Value {
    json!({
        "model": handle,
        "messages": [{"role": "user", "content": message}],
        "stream": false,
    })
}

/// **本番の入口（`App::bind`）で1往復させ、両方の上流に何が届いたかを返す。**
///
/// `sub_endpoint` が `Some` なら `PERSONACORE_SUB_LLM_ENDPOINT` を与える。
/// **`None` の腕が対照**——未設定の既定の振る舞い（`with_model` の複製）を通る。
///
/// `settle` は「これ以上は届かない」と言い切る前の猶予。
/// **肯定側は `WAIT_LIMIT` まで poll する**（呼び出し側が `want_sub_calls` で指定）。
async fn roundtrip(
    main: &MockServer,
    sub: &MockServer,
    sub_endpoint: Option<&str>,
    want_sub_calls: usize,
) {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let main_uri = main.uri();
    let sub_uri = sub_endpoint.map(str::to_owned);

    // **前提: 2つの上流は別の宛先で、どちらも実機ではないこと。**
    assert_ne!(main.uri(), sub.uri(), "前提: 2つの上流は別の宛先であること");
    for uri in [main.uri(), sub.uri()] {
        assert_ne!(
            uri.trim_end_matches('/'),
            DEFAULT_ENDPOINT,
            "実機（{DEFAULT_ENDPOINT}）へ向けないこと（docs/05-development.md）"
        );
    }

    let config = AppConfig::from_lookup(|key| match key {
        ENV_LLM_ENDPOINT => Some(main_uri.clone()),
        ENV_SUB_LLM_ENDPOINT => sub_uri.clone(),
        ENV_MAIN_MODEL => Some(MAIN_MODEL.to_owned()),
        ENV_SUB_MODEL => Some(SUB_MODEL.to_owned()),
        _ => None,
    })
    .expect("設定を組み立てられること")
    .with_bind(BindAddr::loopback(0))
    .with_database_path(dir.path().join("personacore.sqlite"));

    // **`bind_with_clients` ではない。** ここが本番と同じ入口であることが要点。
    let app = App::bind(&config).await.expect("待ち受けを開始できること");
    let addr = app.local_addr();
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

    let (shutdown, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        app.serve(async move {
            let _ = rx.await;
        })
        .await
    });

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/v1/chat/completions"))
        .json(&non_streaming_body(HANDLE, USER_MESSAGE))
        .send()
        .await
        .expect("サーバへ接続できること");
    assert!(response.status().is_success(), "{:?}", response.status());
    let _ = response.text().await.expect("本文を読めること");

    // **肯定側は締切まで poll し、否定側は猶予だけ置く**（`SETTLE` の doc）。
    let limit = Instant::now()
        + if want_sub_calls == 0 {
            SETTLE
        } else {
            WAIT_LIMIT
        };
    while completions_to(sub).await.len() < want_sub_calls && Instant::now() < limit {
        tokio::time::sleep(POLL_STEP).await;
    }
    if want_sub_calls == 0 {
        tokio::time::sleep(SETTLE).await;
    }

    shutdown.send(()).expect("停止シグナルを送れること");
    handle
        .await
        .expect("サーバタスクが panic しないこと")
        .expect("正常に終了すること");
}

/// **抽出だけが2つ目の上流へ行く**（ADR-0039 §決定1 / 作業単位 G-3a）。
///
/// # なぜ「モデルを2つ」では足りないのか
///
/// **`llama serve` は 1 プロセス 1 モデルで、`model` の欄を見ない**（`G-1` の実測）。
/// `LmStudioClient::with_model` は **base URL を共有した複製**なので、
/// **「2つのモデル」は表現できても「2つの上流」は表現できない。**
/// **`C-12` が測った配置**（抽出だけ別モデル / 持ち主の取り違え 64% → 8%）は、
/// **このつまみが無いと1台の機の上では表現できない。**
///
/// # 2つの腕を1本に置く理由
///
/// | 腕 | `PERSONACORE_SUB_LLM_ENDPOINT` | 本文の上流 | 抽出の上流 |
/// |---|---|---|---|
/// | 2つの上流 | 2つ目を指す | **本文だけ**（1回） | **抽出だけ**（1回） |
/// | 1つの上流 | 未設定 | **本文と抽出の両方**（2回） | **0回** |
///
/// **腕ごとに捕まえる相手が違う。**
/// 上の腕は「**2つ目のつまみが無視される**」形（未設定と同じ経路を通る）を、
/// 下の腕は「**未設定なのに本文とは別の宛先へ送る**」形（`DEFAULT_ENDPOINT` へ送る等）を捕まえる。
/// **下の腕でしか捕まらない形が実在する**ので、外さないこと。
///
/// # ⚠ 言えること・言えないこと（**2周にわたって、2つとも訂正されている**）
///
/// **1周目のこの節は「未設定の腕と一緒に置いて初めて、両方向の決め打ちが赤になる」と
/// 書いていた。G-3a のレビューが実測して否定した**（2026-09-07）。
///
/// | 決め打ち | **このテスト**で赤になるか |
/// |---|---|
/// | **常に `with_model` の複製**（2つ目のつまみを無視する） | **赤** |
/// | **未設定でも `LmStudioClient::new` で別のクライアントを組む**<br>（同じ宛先・同じモデル・**プールだけ分かれる**） | **緑**（`m14`） |
///
/// **下の行は、いまも「このテストでは緑」である。**
/// 宛先もモデルも同じなので、**`wiremock` から見える往復は1バイトも変わらない。**
///
/// # ⚠ しかし「観測できない」ではない（**2周目の訂正**）
///
/// **1周目はここに「TCP の本数でも割れない」と書いていた。偽だった。**
/// 根拠にした「起動時の2本の照会は**同時に走る**ので、共有していても接続は2本開く」は
/// **正しい。そこで話を終えたのが誤り**——**重なる往復をもう一段作れば割れる。**
///
/// **見張り番は同じファイルの
/// `the_unset_extraction_client_shares_the_body_clients_connection_pool`**
/// （接続を数える上流へ、本文の依頼を2本重ねる。
/// **`m14` を当てて 1 赤・exit 101 / 10 回とも**）。
///
/// **[ADR-0039](../docs/adr/0039-two-upstreams-for-two-jobs.md) §決定1 の
/// 「未設定なら本文と同じクライアントを使う」は、いまは見張り番を持つ。**
/// **ただし、このテストが持っているのではない。** ここが守っているのは
/// **宛先とモデルの帰属**まで。
///
/// # モデルの綴りも同じ行で見る（**帰属**）
///
/// 「どの上流へ行ったか」だけを見ると、**2つの上流に本文用のクライアントを2つ組む**
/// 取り違えが素通りする。**届いた本文の `model` まで見る**ので、
/// **上流とモデルの組み合わせが入れ替わった形**も赤になる。
#[tokio::test]
async fn the_sub_llm_endpoint_sends_only_the_extraction_to_the_second_upstream() {
    assert_ne!(
        MAIN_MODEL, SUB_MODEL,
        "前提: 2つのモデルは別の綴りであること"
    );
    assert_ne!(
        MAIN_MODEL, DEFAULT_MAIN_MODEL,
        "前提: 本文モデルは既定と別の綴りであること"
    );
    assert_ne!(
        SUB_MODEL, DEFAULT_SUB_MODEL,
        "前提: 抽出モデルは既定と別の綴りであること"
    );

    // ---- 腕1: 2つの上流。**本文は1つ目へ、抽出は2つ目へ。** ----
    let main = upstream_for(MAIN_MODEL).await;
    let sub = upstream_for(SUB_MODEL).await;
    let sub_uri = sub.uri();
    roundtrip(&main, &sub, Some(&sub_uri), 1).await;

    let to_main = completions_to(&main).await;
    let to_sub = completions_to(&sub).await;

    assert_eq!(
        to_main.len(),
        1,
        "本文の上流へ行くのは本文の1回だけであること（抽出まで来ている）: {to_main:?}"
    );
    assert_eq!(
        to_sub.len(),
        1,
        "抽出の上流へ抽出が1回届くこと（2つ目のエンドポイントが無視されている）: {to_sub:?}"
    );
    assert_eq!(
        to_main[0]["model"].as_str(),
        Some(MAIN_MODEL),
        "1つ目の上流へ行くのは本文モデルであること: {:?}",
        to_main[0]
    );
    assert_eq!(
        to_sub[0]["model"].as_str(),
        Some(SUB_MODEL),
        "2つ目の上流へ行くのは抽出モデルであること: {:?}",
        to_sub[0]
    );
    // **2つ目へ行ったのが抽出の依頼であること**（本文をもう一度投げているのではない）。
    assert!(
        to_sub[0]["messages"]
            .to_string()
            .contains(personacore::prompt::extraction::EXAMPLE_USER_LINE),
        "2つ目の上流へ行ったのは抽出の依頼であること: {:?}",
        to_sub[0]
    );

    // **約束・決まりの提案の依頼も、本文の上流へは行かない**（裏方の列の上流 / 作業単位 A-12c-1）。
    assert_eq!(
        proposals_to(&main).await,
        0,
        "提案の依頼が本文の上流へ行っている"
    );

    // **サブのコンテキスト長の照会も2つ目の上流へ行くこと**（`resolve_sub_context`）。
    // ここが1つ目のままだと、**抽出の上限が本文モデルの窓から決まる。**
    assert!(
        context_queries_to(&sub).await >= 1,
        "サブのコンテキスト長は2つ目の上流へ照会すること"
    );

    // ---- 腕2: 未設定。**既定の振る舞い**（1つの上流に2往復）。 ----
    let only = upstream_for(MAIN_MODEL).await;
    let unused = upstream_for(SUB_MODEL).await;
    roundtrip(&only, &unused, None, 0).await;

    let to_only = completions_to(&only).await;
    assert_eq!(
        to_only.len(),
        2,
        "未設定なら本文も抽出も同じ上流へ行くこと: {to_only:?}"
    );
    assert_eq!(
        to_only[0]["model"].as_str(),
        Some(MAIN_MODEL),
        "1往復目は本文モデルであること: {:?}",
        to_only[0]
    );
    assert_eq!(
        to_only[1]["model"].as_str(),
        Some(SUB_MODEL),
        "2往復目は抽出モデルであること（`with_model` の複製）: {:?}",
        to_only[1]
    );
    assert_eq!(
        all_completions_to(&unused).await.len(),
        0,
        "未設定なのに2つ目の上流へ行っている（常に2つ目のクライアントを組んでいる）"
    );
    assert_eq!(
        context_queries_to(&unused).await,
        0,
        "未設定なのに2つ目の上流へ照会している"
    );
}

// ---------------------------------------------------------------------------
// **接続プールの見張り番**（G-3a の是正2周目 / 2026-09-07）
//
// レビュー1周目まで、この作業単位の doc は「未設定でも別のクライアントを組む変異は
// **原理的に観測できない**」と書いていた。**偽である。**
// **レビュアーが接続を数える上流を立てて実際に割った**ので、以下はその形を見張り番にしたもの。
// ---------------------------------------------------------------------------

/// 2人目のキャラクターの識別子。**1人目とも表示名とも別の値。**
const SECOND_HANDLE: &str = "midori";

/// 2人目の表示名。**識別子と同値にしない。**
const SECOND_DISPLAY_NAME: &str = "翠";

/// **同時に投げる本文の依頼の本数。**
///
/// **上流はこの数がそろうまで、どちらの本文にも答えない**（[`CountingUpstream`]）。
/// そろったら**1本目にだけ答え、2本目はテストが [`CountingUpstream::release_held_body`] を
/// 呼ぶまで待たせる**（1本目の裏方が終わるのを待つため /
/// `the_unset_extraction_client_shares_the_body_clients_connection_pool` の doc）。
/// **待たせる側が1本だけなのは、この値が 2 だから**——増やすなら、待たせた本文どうしが
/// また同時に返り、同じ取り合いに戻る。
/// **時計で待たないこと**——`sleep` で重なりを作ると、機が重い回に重ならず、
/// **「重なっていないから接続が増えない」を「共有できている」と読む**形になる
/// （CLAUDE.md「緑のまま何も測らない形にしない」）。
const OVERLAPPING_BODY_TURNS: usize = 2;

/// 上流が数えたもの。**この見張り番の本体は [`Seen::connections`]。**
#[derive(Debug, Default)]
struct Seen {
    /// **`accept` した TCP 接続の総数。**
    connections: AtomicUsize,
    /// 届いた `/api/v0/models` の総数。
    catalogue_queries: AtomicUsize,
    /// 届いた `/v1/chat/completions` の総数（本文＋抽出）。**約束・決まりの提案の依頼は数えない**
    /// （[`Seen::proposals`] に別に数える / 作業単位 A-12c-1）。
    completions: AtomicUsize,
    /// 届いた**約束・決まりの提案の依頼**の数（抽出の後ろの同じ列で走る。接続の本数の主張には関わらない）。
    proposals: AtomicUsize,
    /// いま処理中の**本文**の依頼の本数（抽出は数えない）。
    body_in_flight: AtomicUsize,
    /// [`Seen::body_in_flight`] の最大値。**重なりが実際に起きたか。**
    peak_body_in_flight: AtomicUsize,
    /// **来るはずのない綴りへの要求**（届いたら 404 を返して数える）。
    unexpected: AtomicUsize,
}

/// **接続を数える生の上流。**
///
/// # なぜ `wiremock` では書けないのか
///
/// **`wiremock` は「どんな要求が届いたか」しか見せない。**
/// ここで測りたいのは**その下の層**——**2つのクライアントが同じ `reqwest::Client` を
/// 共有しているか**である。**宛先もモデルも同じ**なので、
/// **要求そのものは1バイトも変わらない。**
/// 差が出るのは**開いた TCP 接続の本数**だけで、それは `accept` を自分で数えるしかない。
///
/// # 何をする上流か
///
/// * `GET /api/v0/models` — 与えられた綴りを並べて即答する。
/// * `POST /v1/chat/completions` — **本文の依頼なら
///   [`OVERLAPPING_BODY_TURNS`] 本そろうまで待ち、そろったら1本目にだけ答える。**
///   **2本目は [`CountingUpstream::release_held_body`] まで待たせる**（どちらも上限つき）。
///   抽出と提案は待たずに答える。
/// * `POST /apply-template` — 404 で断る（封のための組み上げの口 / 作業単位 B-60b。数えない）。
/// * `POST /tokenize` — 404 で断る（部品を数える口 / 作業単位 B-60d-1。最初の部品が1回来る。数えない）。
/// * それ以外 — 404 を返して [`Seen::unexpected`] を数える。
///
/// **`Connection: close` は返さない。** 返すと在庫が毎回捨てられ、
/// **プールを共有していても接続が要求の数だけ開く**——
/// **共有しているかどうかが観測できなくなる。**
struct CountingUpstream {
    addr: SocketAddr,
    seen: Arc<Seen>,
    /// **待たせている2本目の本文を放す合図**（[`CountingUpstream::release_held_body`]）。
    release: Arc<Notify>,
    accepting: tokio::task::JoinHandle<()>,
}

impl CountingUpstream {
    /// `models` を並べて答える上流を立てる。
    async fn start(models: &[&str]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("待ち受けを開けること");
        let addr = listener.local_addr().expect("待ち受け先を読めること");
        let seen = Arc::new(Seen::default());
        let catalogue = models_wire(models);
        let barrier = Arc::new(Barrier::new(OVERLAPPING_BODY_TURNS));
        let release = Arc::new(Notify::new());
        let accepting = tokio::spawn({
            let seen = Arc::clone(&seen);
            let release = Arc::clone(&release);
            async move {
                loop {
                    let Ok((socket, _)) = listener.accept().await else {
                        return;
                    };
                    // **数えるのはここ1箇所。** 以後この接続で何往復しても増えない。
                    seen.connections.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(serve_connection(
                        socket,
                        Arc::clone(&seen),
                        catalogue.clone(),
                        Arc::clone(&barrier),
                        Arc::clone(&release),
                    ));
                }
            }
        });
        Self {
            addr,
            seen,
            release,
            accepting,
        }
    }

    /// **待たせている2本目の本文に答えさせる。**
    ///
    /// `Notify::notify_one` は待ち手がまだいなくても許可を1つ残すので、
    /// 2本目が待ちに入る前に呼んでも取りこぼさない。
    fn release_held_body(&self) {
        self.release.notify_one();
    }

    /// 設定に渡すベース URL。
    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// 待ち受けを畳む。
    fn stop(self) {
        self.accepting.abort();
    }
}

/// `/api/v0/models` の応答本文（実機と同じ形）。**どれも `loaded`。**
fn models_wire(models: &[&str]) -> String {
    let entries: Vec<String> = models
        .iter()
        .map(|model| {
            format!(
                r#"{{"id":"{model}","object":"model","type":"llm","state":"loaded",
                   "max_context_length":262144,"loaded_context_length":16384}}"#
            )
        })
        .collect();
    format!(r#"{{"object":"list","data":[{}]}}"#, entries.join(","))
}

/// `needle` が最初に現れる位置。
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// ヘッダから `Content-Length` を読む。無ければ 0。
fn content_length(head: &str) -> usize {
    for line in head.lines() {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
            && let Ok(length) = value.trim().parse::<usize>()
        {
            return length;
        }
    }
    0
}

/// **1本の接続で、keep-alive のまま要求を順に捌く。**
async fn serve_connection(
    mut socket: TcpStream,
    seen: Arc<Seen>,
    catalogue: String,
    barrier: Arc<Barrier>,
    release: Arc<Notify>,
) {
    let mut buffered: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        // ---- ヘッダの終わりまで読む。相手が畳んだら抜ける。 ----
        let head_end = loop {
            if let Some(at) = find(&buffered, b"\r\n\r\n") {
                break at + 4;
            }
            match socket.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(read) => buffered.extend_from_slice(&chunk[..read]),
            }
        };
        let head = String::from_utf8_lossy(&buffered[..head_end]).into_owned();
        let length = content_length(&head);
        while buffered.len() < head_end + length {
            match socket.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(read) => buffered.extend_from_slice(&chunk[..read]),
            }
        }
        let body = String::from_utf8_lossy(&buffered[head_end..head_end + length]).into_owned();
        buffered.drain(..head_end + length);

        let request_line = head.lines().next().unwrap_or_default().to_owned();
        let (status, response) = if request_line.contains(" /api/v0/models ") {
            seen.catalogue_queries.fetch_add(1, Ordering::SeqCst);
            ("200 OK", catalogue.clone())
        } else if request_line.contains(" /v1/chat/completions ")
            && serde_json::from_str::<Value>(&body)
                .is_ok_and(|request| is_agreement_proposal(&request))
        {
            seen.proposals.fetch_add(1, Ordering::SeqCst);
            ("200 OK", UPSTREAM_BODY.replace("PLACEHOLDER", SUB_MODEL))
        } else if request_line.contains(" /v1/chat/completions ") {
            seen.completions.fetch_add(1, Ordering::SeqCst);
            let model = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|request| request["model"].as_str().map(str::to_owned))
                .unwrap_or_default();
            // **本文の依頼だけを待たせる。** 抽出まで待たせると、
            // **抽出は本文が返り切った後に始まる**（ADR-0021）ので2本目が来ず、止まる。
            if model == MAIN_MODEL {
                let now = seen.body_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                seen.peak_body_in_flight.fetch_max(now, Ordering::SeqCst);
                // **待つ側に上限を置く**（CLAUDE.md / 作業単位 G-4）。
                // そろわなかったら答えてしまい、**赤は `peak_body_in_flight` で出す。**
                let answer_now = tokio::time::timeout(WAIT_LIMIT, barrier.wait())
                    .await
                    .map_or(true, |waited| waited.is_leader());
                seen.body_in_flight.fetch_sub(1, Ordering::SeqCst);
                // **そろった後は1本目にだけ答え、2本目はテストの合図まで待たせる**
                // （1本目の裏方を終わらせてから2本目を保存させる / marukei/personacore#42）。
                // **ここにも上限を置く。** 合図が来なければ答えてしまい、
                // 赤は待つ側（`wait_for_background`）が「時間切れ」で出す。
                if !answer_now {
                    let _ = tokio::time::timeout(WAIT_LIMIT, release.notified()).await;
                }
            }
            ("200 OK", UPSTREAM_BODY.replace("PLACEHOLDER", &model))
        } else if request_line.contains(" /apply-template ") {
            // **封のための組み上げの口**（作業単位 B-60b / ADR-0052 §推奨3）。本文の前に毎回来る。
            // **口を持たない llama と同じく 404 で断る**（＝数えられない）。
            // **「来るはずのない綴り」には数えない** ——来るのが正しい。
            ("404 Not Found", "{}".to_owned())
        } else if request_line.contains(" /tokenize ") {
            // **部品を数える口**（作業単位 B-60d-1 / ADR-0052 §推奨2）。封の前に、最初の部品が
            // 1回来る（`server::counted_plan`）。**404 で断る**（＝数えられない。残りの部品は来ない）。
            // **「来るはずのない綴り」には数えない** ——来るのが正しい。
            ("404 Not Found", "{}".to_owned())
        } else {
            seen.unexpected.fetch_add(1, Ordering::SeqCst);
            ("404 Not Found", "{}".to_owned())
        };

        let wire = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{response}",
            response.len()
        );
        if socket.write_all(wire.as_bytes()).await.is_err() || socket.flush().await.is_err() {
            return;
        }
    }
}

/// **裏方が終わるまで待ち、その時点で届いた本数を判定する**（marukei/personacore#42）。
///
/// **呼ぶのは、その往復の HTTP 応答を読み切った後だけ。** 非ストリーミングの経路は
/// 保存（＝裏方を走らせるか捨てるかの決定）を済ませてから応答を返すので、
/// 読み切った時点で決定は済んでいる。**読み切る前に呼ぶと、裏方がまだ始まっていないのを
/// 「空いている」と読む。**
///
/// 返す言い分は2つに分かれる:
///
/// | 言い分 | 状態 |
/// |---|---|
/// | **時間切れ** | [`WAIT_LIMIT`] まで待っても裏方が終わらなかった |
/// | **届かなかった** | 裏方は終わったのに本数が足りない（送られなかった） |
///
/// **本数だけを時間で待つ形にしないこと。** それだと、送られない要求を
/// 上限まで待ってから「本数が違う」とだけ言い、遅れと欠落の区別が付かない（以前の形）。
async fn wait_for_background(
    seen: &Seen,
    background: &BackgroundProbe,
    want: usize,
    what: &str,
) -> Result<(), String> {
    let limit = Instant::now() + WAIT_LIMIT;
    loop {
        // **枠を先に見て、本数を後に読む。** 本数は上流が要求を受けた時点で数えるので、
        // 空いていたのを見た後に読んだ本数は、その裏方が送った分を全部含む。
        let idle = background.is_idle();
        let arrived = seen.completions.load(Ordering::SeqCst);
        if idle {
            return if arrived >= want {
                Ok(())
            } else {
                Err(format!(
                    "{what}: 届かなかった —— 裏方は終わったのに、届いた本数が足りない\
                     （{arrived} / {want}。走っている間に来た往復の裏方は捨てられる / ADR-0021 §7）"
                ))
            };
        }
        if Instant::now() >= limit {
            return Err(format!(
                "{what}: 時間切れ —— {WAIT_LIMIT:?} 待っても裏方が終わらなかった\
                 （届いた本数 {arrived} / {want}）"
            ));
        }
        tokio::time::sleep(POLL_STEP).await;
    }
}

/// **未設定のとき、抽出のクライアントは本文のクライアントと接続プールを共有する**
/// （[ADR-0039](../docs/adr/0039-two-upstreams-for-two-jobs.md) §決定1 /
/// 作業単位 G-3a の是正2周目）。
///
/// # ⚠ 「観測できない」は偽だった
///
/// **1周目のこの作業単位は、`AppConfig::sub_llm` と `App::bind` と
/// このファイルの3箇所に「TCP の本数でも割れない」と書いていた。**
/// 根拠は「起動時の2本の照会は同時に走るので、共有していても接続は2本開く」——
/// **前半は正しく、そこから「割れない」と断定したのが誤り。**
/// **重なる往復をもう一段作れば割れる**（レビュアーが実測 / 2026-09-07）。
///
/// # 何が違うのか
///
/// | 枝 | 起動時 | **本文を2本重ねたあと** |
/// |---|---|---|
/// | **共有**（`llm.with_model` の複製） | 2 | **2**（起動時の2本がどちらも本文側に使える） |
/// | **分ける**（`LmStudioClient::new`） | 2 | **3**（本文側の手持ちは1本しか無い） |
///
/// **宛先もモデルも同じなので、`wiremock` から見える往復は1バイトも変わらない。**
/// **差は「開いた TCP 接続の本数」にしか出ない**ので、[`CountingUpstream`] が要る。
///
/// # ⚠ 重なりは時計ではなく数で作る
///
/// **重ならなければ、プールを分けても在庫の1本を順に使い回すだけで接続は増えない。**
/// つまり**この見張り番は「重ならなかった回」に何も測らずに緑になる。**
/// だから上流は [`OVERLAPPING_BODY_TURNS`] 本そろうまで答えず、
/// **重なりが実際に起きたことを [`Seen::peak_body_in_flight`] で別に主張する。**
///
/// # ⚠ 2人のキャラクターへ投げる
///
/// 同じ相手へ2本重ねると**セッションへの書き込みが取り合いになる**ので、
/// 測っているものが接続の話でなくなる。**別々のキャラクターなら別々のセッション。**
///
/// # ⚠ 2本目の本文は、1本目の裏方が終わるまで返させない（marukei/personacore#42）
///
/// **裏方の枠は全キャラクターで1本だけで、走っている間に来た往復の裏方は捨てられる**
/// （ADR-0021 §7 / `server::extract` の `MemoryExtractor::spawn`。提案の段にいれば横取りする）。
/// 以前は2本の本文を同時に返していたので、**2本目の保存が1本目の抽出の最中に当たると、
/// 2本目の抽出は `skipped=busy` で送られない**——本番の設計どおりの振る舞いで、
/// 「本文2本と抽出2本が届く」はタイミングに頼った前提だった。
/// 実測: 上流が抽出に答えるのを 1 ミリ秒遅らせるだけで、毎回 3 本で止まり、
/// `skipped=busy` の行が1本出た（CI では同じことが機の遅さで起きていた）。
///
/// だから上流は**重なりを作った後、1本目にだけ答え**、テストは
/// **1本目の抽出が届き、裏方が空いた**のを見てから2本目を放す（[`wait_for_background`]）。
/// **重なり（`peak_body_in_flight`）も、接続の本数の主張も変わらない**——
/// 重なっている間は本文側の2本の接続が両方塞がる（プールを分けた枝では、ここで3本目が開く）。
/// 2本目を待たせている間に塞がっているのは1本で、1本目の抽出は空いた1本
/// （分けた枝なら抽出側の起動時の1本）を使う。
#[tokio::test]
async fn the_unset_extraction_client_shares_the_body_clients_connection_pool() {
    let upstream = CountingUpstream::start(&[MAIN_MODEL, SUB_MODEL]).await;
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let base = upstream.base_url();
    assert_ne!(
        base.trim_end_matches('/'),
        DEFAULT_ENDPOINT,
        "実機（{DEFAULT_ENDPOINT}）へ向けないこと（docs/05-development.md）"
    );

    let config = AppConfig::from_lookup(|key| match key {
        ENV_LLM_ENDPOINT => Some(base.clone()),
        ENV_MAIN_MODEL => Some(MAIN_MODEL.to_owned()),
        ENV_SUB_MODEL => Some(SUB_MODEL.to_owned()),
        // **`ENV_SUB_LLM_ENDPOINT` は与えない。** ここで測るのは未設定の枝。
        _ => None,
    })
    .expect("設定を組み立てられること")
    .with_bind(BindAddr::loopback(0))
    .with_database_path(dir.path().join("personacore.sqlite"));

    // **`bind_with_clients` ではない。** 未設定の枝を通る本番の入口はここだけ。
    let app = App::bind(&config).await.expect("待ち受けを開始できること");
    let addr = app.local_addr();
    // **裏方の枠が空いたかを見る口**（`wait_for_background`）。`serve` が `app` を食う前に取る。
    let background = app.background_probe();

    // ---- 起動時。**ここだけでは割れない**（1周目が「割れない」と書いた理由）。 ----
    assert_eq!(
        upstream.seen.catalogue_queries.load(Ordering::SeqCst),
        2,
        "起動時は2つの面がそれぞれ照会すること（往復は面ごとに1回）"
    );
    let at_startup = upstream.seen.connections.load(Ordering::SeqCst);
    assert_eq!(
        at_startup, 2,
        "起動時の2本の照会は同時に走るので、プールを共有していても接続は2本開く"
    );

    for (handle, display) in [(HANDLE, DISPLAY_NAME), (SECOND_HANDLE, SECOND_DISPLAY_NAME)] {
        app.store()
            .create_character(
                &Handle::parse(handle).expect("正当な名前であること"),
                &DisplayName::parse(display).expect("正当な表示名であること"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");
    }

    let (shutdown, rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        app.serve(async move {
            let _ = rx.await;
        })
        .await
    });

    // ---- **本文を2本、同時に。** 上流は2本そろうまでどちらにも答えず、そろったら1本目にだけ答える。 ----
    let client = reqwest::Client::new();
    let endpoint = format!("http://{addr}/v1/chat/completions");
    let turn = |handle: &str| {
        let request = client
            .post(&endpoint)
            .json(&non_streaming_body(handle, USER_MESSAGE));
        tokio::spawn(async move {
            let response = request.send().await.expect("サーバへ接続できること");
            let status = response.status();
            let _ = response.text().await.expect("本文を読めること");
            status
        })
    };
    let (first, second) = (turn(HANDLE), turn(SECOND_HANDLE));

    // **先に返った1本（＝上流が答えた方）を読み切る。** これでその往復の裏方の決定は済んでいる
    // （`wait_for_background` の doc）。**上限は `WAIT_LIMIT` の2倍**——重ならなかった回は、
    // 上流がそろうのを `WAIT_LIMIT` まで待ってから答えるので、それより先に諦めない
    // （赤はすぐ下の `peak_body_in_flight` で出す）。
    let (answered, held) =
        match tokio::time::timeout(WAIT_LIMIT * 2, futures_util::future::select(first, second))
            .await
            .expect("時間切れ —— 1本目の本文の応答が返らなかった")
        {
            Either::Left((status, held)) | Either::Right((status, held)) => (status, held),
        };
    let answered = answered.expect("往復のタスクが panic しないこと");
    assert!(answered.is_success(), "{answered:?}");
    // **重なったかは、ここで確かめる。** 上流が1本目に答えた時点で2本はそろっているので、
    // 重なった回はもう 2 に確定している。重ならなかった回をここで止めないと、下の
    // `wait_for_background` が「届かなかった（裏方が捨てた）」と、原因と別のことを言う。
    assert_eq!(
        upstream.seen.peak_body_in_flight.load(Ordering::SeqCst),
        OVERLAPPING_BODY_TURNS,
        "本文の2本が上流で重ならなかった（重ならないと、プールの共有と分割を割れない）"
    );

    // **1本目の抽出が届き、1本目の裏方が終わるまで待つ**（本文2本＋抽出1本）。
    // ここで2本目を放すと、2本目の保存が1本目の抽出の最中に当たり、2本目の抽出が捨てられうる
    // （このテストの doc / marukei/personacore#42）。
    if let Err(why) = wait_for_background(
        &upstream.seen,
        &background,
        OVERLAPPING_BODY_TURNS + 1,
        "1本目の往復の抽出",
    )
    .await
    {
        panic!("{why}");
    }

    // ---- **2本目の本文を放す。** 裏方の枠は空いているので、2本目の抽出は捨てられない。 ----
    upstream.release_held_body();
    let held = tokio::time::timeout(WAIT_LIMIT * 2, held)
        .await
        .expect("時間切れ —— 2本目の本文の応答が返らなかった")
        .expect("往復のタスクが panic しないこと");
    assert!(held.is_success(), "{held:?}");

    // **抽出まで届き切り、裏方が終わるのを待つ**（本文2本＋抽出2本）。**待つ側に上限を置く。**
    let want_completions = OVERLAPPING_BODY_TURNS * 2;
    if let Err(why) = wait_for_background(
        &upstream.seen,
        &background,
        want_completions,
        "2本目の往復の抽出",
    )
    .await
    {
        panic!("{why}");
    }
    // **「これ以上は開かない」と言い切る前の猶予**（`SETTLE` の doc）。
    tokio::time::sleep(SETTLE).await;

    shutdown.send(()).expect("停止シグナルを送れること");
    handle
        .await
        .expect("サーバタスクが panic しないこと")
        .expect("正常に終了すること");

    let connections = upstream.seen.connections.load(Ordering::SeqCst);
    let completions = upstream.seen.completions.load(Ordering::SeqCst);
    let peak = upstream.seen.peak_body_in_flight.load(Ordering::SeqCst);
    let unexpected = upstream.seen.unexpected.load(Ordering::SeqCst);
    upstream.stop();

    assert_eq!(unexpected, 0, "この上流へ来るはずのない綴りが届いている");
    assert_eq!(
        completions, want_completions,
        "本文2本と抽出2本が届くこと（届いていないなら、下の本数は何も意味しない）"
    );
    assert_eq!(
        peak, OVERLAPPING_BODY_TURNS,
        "前提: 本文の往復が実際に重なったこと\
         ——重ならなければ、プールを分けても在庫の1本を使い回すだけで接続は増えない"
    );
    assert_eq!(
        connections, at_startup,
        "未設定なら抽出のクライアントは本文のクライアントと接続プールを共有すること\
         （分けると、重なった2本目の本文が {at_startup} 本目の接続を開く）"
    );
}
