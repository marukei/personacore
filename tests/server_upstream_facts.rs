//! **上流について観測できたことが、本番の入口（`App::bind`）から
//! `AppState` まで届いている**ことの検証（作業単位 G-3b /
//! [ADR-0039](../docs/adr/0039-two-upstreams-for-two-jobs.md)）。
//!
//! **実際の LM Studio には繋がない**（docs/05-development.md）。上流はこのファイルが立てる。
//!
//! # なぜ本番の入口を通す必要があるのか
//!
//! **`App::bind_with_clients`（注入の入口）は、この配線を1ミリも見ない**
//! ——あちらは照会そのものを行わないので、観測は「一度も聞いていない」で固定される。
//! **`bind` だけが `resolve_budget` / `resolve_sub_context` を通り、
//! `measured::report` の内訳を器へ入れる。**
//!
//! # ⚠ 既定で回さない
//!
//! 2つのエンドポイントも2つのモデルの綴りも、**互いに違い、どれも既定ではない。**
//! 同値にすると「設定を読まずに本文の上流を使う」実装と区別が付かない
//! （CLAUDE.md の最頻の穴）。
//!
//! # ここで見ていないもの
//!
//! * **引き直しで観測が更新されること**は `server::context` の単体テストの担当
//!   （5分を実時間で待てないので、短い間隔を与えられるそちらでしか見られない）。
//! * **突き合わせの規則そのもの**は `measured` の単体テストの担当。

use personacore::llm::AnsweredBy;
use personacore::measured::{ModelFace, judge_face};
use personacore::server::{
    App, AppConfig, BindAddr, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL, ENV_SUB_LLM_ENDPOINT,
    ENV_SUB_MODEL, UpstreamFacts,
};
use serde_json::json;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

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
/// **このうち保持してよいのはファイル名だけ**（作業単位 B-45 / `server::upstream` の doc）。
const LLAMA_MODEL_ID: &str = "/models/Ministral-3-14B-Instruct-2512-Q4_K_M.gguf";

/// **同じファイル名を Windows 形の絶対パスで名乗る上流の `id`。**
///
/// **`/` しか見ない切り落としは、この形を丸ごと通す。**
const LLAMA_MODEL_ID_WINDOWS: &str = "C:\\models\\llm\\Ministral-3-14B-Instruct-2512-Q4_K_M.gguf";

/// **上の2つの、外へ出てよい部分**（ファイル名）。
const LLAMA_MODEL_FILE: &str = "Ministral-3-14B-Instruct-2512-Q4_K_M.gguf";

/// **出てはならない部分**（POSIX 形のディレクトリ）。
const LLAMA_MODEL_DIR: &str = "/models/";

/// **出てはならない部分**（Windows 形のディレクトリ）。
const LLAMA_MODEL_DIR_WINDOWS: &str = "C:\\models\\llm\\";

/// **依頼した綴りと同じ一覧に並ぶ、別のモデルの `id`。**
///
/// **一覧を丸ごと保持していないこと**を見るための囮
/// ——`/models/` をそのまま含むので、抱え込めばディレクトリごと外へ出る。
const OTHER_MODEL_ID: &str = "/models/some-other-model.gguf";

/// **`vendor/model` 形で依頼するモデルの綴り**（作業単位 B-45 のレビュー是正）。
///
/// **`/` を含むが、パスではない。** 上流はこの綴りをそのまま一覧に並べる。
/// **継ぎ目が一覧の各要素をファイル名に切ると、この綴りは一覧から消え、
/// 突き合わせが `Drift::Other` ではなく `Drift::Absent` に倒れる。**
const VENDOR_MAIN_MODEL: &str = "vendor/main-model-for-tests";

/// **上を、継ぎ目が切り落としたとしたら残る綴り。** **手で書く**——
/// 切り落としの実装から作ると、緩めたときに期待値も一緒に動く（CLAUDE.md）。
const VENDOR_MAIN_MODEL_TRUNCATED: &str = "main-model-for-tests";

/// 同上、囮のほう（[`OTHER_MODEL_ID`] を切り落とした形）。
const OTHER_MODEL_ID_TRUNCATED: &str = "some-other-model.gguf";

/// **依頼した綴りがそのまま一覧に在る上流**が申告する窓。**他のどの窓とも別。**
const EXACT_TOKENS: u32 = 12_288;

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
/// **本文は `serde_json` に組ませる**（作業単位 B-45）。
/// **Windows 形の `\` を手で書くと JSON として壊れ、一覧が空になって検査が空振りする。**
async fn openai_only_upstream(body: &serde_json::Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body.to_string(), "application/json"))
        .mount(&server)
        .await;
    server
}

/// **一覧は1件で、`id` は引数の綴り。** 同定は「一覧が1件だから」で通る。
async fn upstream_speaking_only_openai(model_id: &str) -> MockServer {
    openai_only_upstream(&json!({
        "object": "list",
        "data": [{
            "id": model_id,
            "object": "model",
            "owned_by": "llamacpp",
            "meta": {"n_ctx": LLAMA_TOKENS, "n_ctx_train": 262_144},
        }],
    }))
    .await
}

/// **依頼した綴りを、別の行と一緒に並べる上流。**
///
/// **一覧は2件**なので「1件だから」では通らない
/// ——同定は**依頼した綴りとの完全一致**で決まる（`AnsweredBy::CompatExactId`）。
async fn upstream_listing_several_including(model: &str) -> MockServer {
    openai_only_upstream(&json!({
        "object": "list",
        "data": [
            {"id": OTHER_MODEL_ID, "object": "model", "owned_by": "llamacpp"},
            {
                "id": model,
                "object": "model",
                "owned_by": "llamacpp",
                "meta": {"n_ctx": EXACT_TOKENS, "n_ctx_train": 262_144},
            },
        ],
    }))
    .await
}

/// **どちらの綴りにも答えない上流**（停止・別実装）。**往復そのものが失敗する。**
async fn upstream_that_never_answers() -> MockServer {
    let server = MockServer::start().await;
    for path in ["/api/v0/models", "/v1/models"] {
        Mock::given(matchers::method("GET"))
            .and(matchers::path(path))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
    }
    server
}

/// **ディレクトリの綴りが、どこにも出ていない。**
///
/// **⚠ `Debug` も JSON も `\` を `\\` へ逃がす**ので、**逃がした形も一緒に探す**
/// ——生の綴りだけを見る検査は、Windows 形で**必ず空振りする。**
fn assert_no_directory(rendered: &str, dir: &str, what: &str) {
    let escaped = dir.replace('\\', "\\\\");
    for needle in [dir, escaped.as_str()] {
        assert!(
            !rendered.contains(needle),
            "{what}: ディレクトリの綴り {needle:?} を保持しないこと: {rendered}"
        );
    }
}

/// 設定に書ける利用者名。**他のどのフィクスチャとも重ならない綴り。**
const ENDPOINT_USER: &str = "personacore-operator";

/// 設定に書ける合言葉。**利用者名とも別。**
const ENDPOINT_PASSWORD: &str = "s3cr3t-never-in-a-response";

/// **観測に載る宛先の綴り**（`LmStudioConfig::endpoint` そのもの）。
///
/// **base URL ではなく、実際に叩く URL。** 起動ログの `llm_endpoint` と同じ綴りで、
/// **`server::App::bind` が「上流は1つか」を判定するのにも使っている値**である。
fn completions_url(server: &MockServer) -> String {
    format!("{}/v1/chat/completions", server.uri())
}

/// **本番の入口で起動し、保持された観測を読む。** サーバは応答させない
/// （見たいのは `bind` が作った器の中身であって、HTTP の振る舞いではない）。
async fn facts_from_real_bind(env: &[(&str, &str)]) -> UpstreamFacts {
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

    // **`bind_with_clients` ではない。** ここが本番と同じ入口であることが要点。
    let app = App::bind(&config).await.expect("待ち受けを開始できること");
    let facts = app.upstream_handle().get();
    drop(app);
    facts
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
    for tokens in [MAIN_TOKENS, SUB_TOKENS, LLAMA_TOKENS] {
        assert_ne!(tokens, EXACT_TOKENS);
    }
    // **上流が名乗る綴りは、こちらが依頼する綴りとも互いとも別。**
    assert_ne!(LLAMA_MODEL_FILE, MAIN_MODEL);
    assert_ne!(LLAMA_MODEL_FILE, SUB_MODEL);
    assert_ne!(LLAMA_MODEL_ID, LLAMA_MODEL_ID_WINDOWS);
    assert_ne!(LLAMA_MODEL_DIR, LLAMA_MODEL_DIR_WINDOWS);
    assert_ne!(OTHER_MODEL_ID, LLAMA_MODEL_ID);
    // **`vendor/model` 形と、それを切り落とした形は別の綴り**
    // ——同値なら、継ぎ目の切り落としを1つも検出しない。
    assert_ne!(VENDOR_MAIN_MODEL, VENDOR_MAIN_MODEL_TRUNCATED);
    assert_ne!(OTHER_MODEL_ID, OTHER_MODEL_ID_TRUNCATED);
    assert_ne!(VENDOR_MAIN_MODEL, MAIN_MODEL);
    assert_ne!(VENDOR_MAIN_MODEL, SUB_MODEL);
}

/// **2つの上流について観測できたことが、面ごとに保持されている**（作業単位 G-3b）。
///
/// **宛先・依頼するモデル・申告された窓・どの読み手が答えたか・突き合わせの内訳**を、
/// **本文と抽出で別々に**見る。
///
/// # 取り違えはここで赤くなる
///
/// **5つの欄がすべて面ごとに別の値**になるようにフィクスチャを組んである
/// ——2つの上流は別のポート、別のモデル、別の窓を申告し、
/// **突き合わせの行数も面ごとに違う**（`MeasuredRole::Sub` の定数が1つあるため）。
///
/// # ⚠ ここでは「頼んだ名前」と「答えた名前」の入れ替えは観測できない
///
/// **この上流は `/api/v0/models` に依頼した綴りをそのまま並べる**（LM Studio の形）ので、
/// **`model` と `answered_name` が同値**になる。
/// **2つが別の綴りになる腕は
/// `the_held_facts_never_carry_the_upstreams_own_model_id`**（`llama-server` の形）。
/// **同値のフィクスチャは、取り違えを1つも検出しない**（CLAUDE.md の最頻の穴）。
#[tokio::test]
async fn the_two_upstreams_are_observed_apart_through_the_real_bind() {
    let main_upstream = upstream_reporting(MAIN_MODEL, MAIN_TOKENS).await;
    let sub_upstream = upstream_reporting(SUB_MODEL, SUB_TOKENS).await;

    let facts = facts_from_real_bind(&[
        (ENV_LLM_ENDPOINT, &main_upstream.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub_upstream.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
    ])
    .await;

    // ---- 本文の面 ----
    let main = facts.main();
    assert_eq!(
        main.endpoint(),
        completions_url(&main_upstream),
        "本文の宛先"
    );
    assert_eq!(main.model(), MAIN_MODEL, "本文が依頼するモデルの綴り");
    assert_eq!(
        main.queried_context_tokens(),
        Some(MAIN_TOKENS),
        "本文の上流が申告した窓"
    );
    assert_eq!(
        main.answered_by(),
        Some(AnsweredBy::NativeApi),
        "本文の上流は独自 API で答えた"
    );
    // **答えた名前は、この面の上流が名乗ったもの**（作業単位 B-45）。
    // **面ごとに別の綴り**なので、流用する実装はここで赤くなる。
    assert_eq!(
        main.answered_name(),
        Some(MAIN_MODEL),
        "本文の上流が名乗った名前"
    );

    // ---- 抽出の面 ----
    let sub = facts.sub();
    assert_eq!(sub.endpoint(), completions_url(&sub_upstream), "抽出の宛先");
    assert_eq!(sub.model(), SUB_MODEL, "抽出が依頼するモデルの綴り");
    assert_eq!(
        sub.queried_context_tokens(),
        Some(SUB_TOKENS),
        "抽出の上流が申告した窓"
    );
    assert_eq!(
        sub.answered_by(),
        Some(AnsweredBy::NativeApi),
        "抽出の上流は独自 API で答えた"
    );
    assert_eq!(
        sub.answered_name(),
        Some(SUB_MODEL),
        "抽出の上流が名乗った名前"
    );

    // ---- 突き合わせの内訳も面ごと ----
    //
    // **どちらの綴りも自分の上流の一覧に在る**ので `Absent` は 0
    // （ここが 0 でないなら、一覧を面をまたいで流用している / ADR-0039 §決定1）。
    // **測ったときの相手ではない**ので、全部 `Other` に落ちる。
    for (what, observation) in [("本文", main), ("抽出", sub)] {
        let measured = observation.measured();
        assert!(measured.checked() > 0, "{what}: 見る行があること");
        assert_eq!(
            measured.drifted_absent, 0,
            "{what}: 自分の上流に在る綴りを「一覧に無い」と言わないこと"
        );
        assert_eq!(measured.unanswered, 0, "{what}: 照会は通っていること");
        assert_eq!(
            measured.drifted_other,
            measured.checked(),
            "{what}: 測ったときの相手ではないので、全行がずれること"
        );
    }
    assert_ne!(
        main.measured(),
        sub.measured(),
        "面ごとに行数が違うこと（同値だと内訳の取り違えを検出できない）"
    );

    // **内訳の帰属**——「載っているか」ではなく「どちらの面のものか」。
    //
    // **面ごとに行数が違う**（`MeasuredRole::Sub` の定数が1つあるので 4 行 と 5 行）ので、
    // **`report` の返り値を面ごと入れ替える変異はここで赤くなる。**
    // 上の `drifted_other == checked()` は入れ替えても成り立つので、そちらでは捕まらない。
    assert_eq!(
        main.measured(),
        judge_face(ModelFace::Main, MAIN_MODEL, Some(&[MAIN_MODEL.to_owned()])),
        "本文の内訳は、本文の面を本文の一覧で見たものであること"
    );
    assert_eq!(
        sub.measured(),
        judge_face(ModelFace::Sub, SUB_MODEL, Some(&[SUB_MODEL.to_owned()])),
        "抽出の内訳は、抽出の面を抽出の一覧で見たものであること"
    );
}

/// **上流が名乗った綴りのうち、保持するのはファイル名だけ**
/// （絶対制約: 内部パスを外へ出さない / 作業単位 B-45 / `server::upstream` の doc）。
///
/// # ⚠ この1本は、B-45 で意味が変わった（司令塔が承認 / 2026-09-17）
///
/// **着手前は「上流が名乗った綴りを1文字も持たない」だった。**
/// **いまは「ディレクトリ部分を持たない」を見る** ——
/// **`C-18` が実測したとおり、`llama serve` は `model` の欄を無視して
/// 載っているモデルで答える**ので、**名乗るものが1つも無いと、
/// 別物が答えていることを読む手段が無い。**
///
/// # 空振りではないこと
///
/// 3つとも、**同じ回の中で**固定する:
///
/// 1. **その綴りを現に読んだ回であること**（`answered_by` が `CompatSoleEntry` になるのは
///    **1件だけの一覧からその行を同定した**ときだけ。窓の値もその一覧から来ている）。
/// 2. **ファイル名は現に保持されていること**（落とす実装なら赤）。
/// 3. **頼んだ名前とは違う綴りであること**（同値なら、この1本は何も守っていない）。
///
/// # ⚠ `/` と `\` の両方で回す
///
/// **片方しか見ない切り落としは、もう片方の形の絶対パスを丸ごと通す。**
/// **そして `Debug` は `\` を `\\` へ逃がす**ので、**逃がした形も一緒に探す**
/// （[`assert_no_directory`]）——生の綴りだけを見る検査は Windows 形で空振りする。
///
/// # ⚠ 「型で表現できなくした」とは書けない
///
/// **止めているのは `server::upstream` の `name_that_answered` の1行**で、
/// **外せば絶対パスがそのまま入る。**
#[tokio::test]
async fn the_held_facts_never_carry_the_upstreams_own_model_id() {
    for (model_id, dir, what) in [
        (LLAMA_MODEL_ID, LLAMA_MODEL_DIR, "POSIX 形の絶対パス"),
        (
            LLAMA_MODEL_ID_WINDOWS,
            LLAMA_MODEL_DIR_WINDOWS,
            "Windows 形の絶対パス",
        ),
    ] {
        let upstream = upstream_speaking_only_openai(model_id).await;

        let facts = facts_from_real_bind(&[
            (ENV_LLM_ENDPOINT, &upstream.uri()),
            (ENV_MAIN_MODEL, MAIN_MODEL),
            (ENV_SUB_MODEL, SUB_MODEL),
        ])
        .await;

        // 前提: その一覧を実際に通していること。
        assert_eq!(
            facts.main().answered_by(),
            Some(AnsweredBy::CompatSoleEntry),
            "{what}: 前提: 1件だけの OpenAI 互換の一覧から同定した回であること"
        );
        assert_eq!(
            facts.main().queried_context_tokens(),
            Some(LLAMA_TOKENS),
            "{what}: 前提: その一覧から窓を読んでいること"
        );

        // 本題1: 答えた名前は、ファイル名だけ。
        assert_eq!(
            facts.main().answered_name(),
            Some(LLAMA_MODEL_FILE),
            "{what}: 上流が名乗った id のファイル名だけを持つこと"
        );
        assert_ne!(
            facts.main().model(),
            LLAMA_MODEL_FILE,
            "{what}: 頼んだ名前と答えた名前が同値でないこと（同値なら何も守っていない）"
        );

        // 本題2: ディレクトリは、保持したものに1文字も入らない。
        let rendered = format!("{facts:?}");
        assert_no_directory(&rendered, dir, what);
        assert!(
            !rendered.contains(model_id),
            "{what}: 名乗った綴りを丸ごと保持しないこと: {rendered}"
        );
        assert!(
            rendered.contains(LLAMA_MODEL_FILE),
            "{what}: ファイル名は現に保持されていること（空振り防止）: {rendered}"
        );
    }
}

/// **依頼した綴りがそのまま一覧に在った回は、答えた名前も同じ綴りになる**（作業単位 B-45）。
///
/// # なぜ切り落とさないのか
///
/// **上流が「その綴りで答える」と名乗っている**ので、推測が1つも入っていない。
/// **ここで `/` の後ろを取ると、`mistralai/devstral-small-2-2512` のような綴りが
/// 勝手に短くなり、「頼んだ名前と違うものが答えた」と画面が嘘をつく。**
///
/// # 同じ一覧の別の行は、1文字も残らない
///
/// 囮（[`OTHER_MODEL_ID`]）を同じ一覧に並べてある
/// ——**一覧を丸ごと抱え込む実装なら、そのディレクトリごと外へ出る。**
///
/// # 名指しできない面も、同じ回の中で見る
///
/// **抽出の綴りはこの一覧に無く、一覧は2件**なので、
/// **どれが答えたかを決められない**——そのときは名乗らない（`None`）。
#[tokio::test]
async fn a_face_answered_by_its_own_spelling_names_it_unchanged() {
    let upstream = upstream_listing_several_including(MAIN_MODEL).await;

    let facts = facts_from_real_bind(&[
        (ENV_LLM_ENDPOINT, &upstream.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
    ])
    .await;

    // 前提: 完全一致で同定した回であること。
    assert_eq!(
        facts.main().answered_by(),
        Some(AnsweredBy::CompatExactId),
        "前提: 依頼した綴りがそのまま一覧に在った回であること"
    );
    assert_eq!(
        facts.main().queried_context_tokens(),
        Some(EXACT_TOKENS),
        "前提: その行から窓を読んでいること"
    );

    assert_eq!(
        facts.main().answered_name(),
        Some(MAIN_MODEL),
        "上流が依頼した綴りを名乗ったなら、そのままの綴りで出すこと"
    );
    assert_eq!(facts.main().model(), MAIN_MODEL, "頼んだ名前は動かないこと");

    // **同じ上流でも、抽出の綴りは一覧に無い**ので、どれが答えたか決められない。
    assert_eq!(
        facts.sub().answered_by(),
        Some(AnsweredBy::CompatCatalogueOnly),
        "前提: 綴りの一覧だけが読めた回であること"
    );
    assert_eq!(
        facts.sub().answered_name(),
        None,
        "どれが答えたかを決められないなら、名乗らないこと"
    );

    // 一覧の別の行は、1文字も残らない。
    let rendered = format!("{facts:?}");
    assert!(
        !rendered.contains(OTHER_MODEL_ID),
        "同じ一覧に並んでいた別の綴りを保持しないこと: {rendered}"
    );
    assert_no_directory(&rendered, LLAMA_MODEL_DIR, "同じ一覧の別の行");
}

/// **何も答えなかった面は、名前も名乗らない**（作業単位 B-45）。
///
/// **空文字にしないこと。** 画面は `null` を「上流は名乗っていません」と出す
/// ——`Some("")` にすると、**名乗ったが空だった**という在りえない状態が画面に出る。
#[tokio::test]
async fn an_upstream_that_never_answered_names_nothing() {
    let upstream = upstream_that_never_answers().await;

    let facts = facts_from_real_bind(&[
        (ENV_LLM_ENDPOINT, &upstream.uri()),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
    ])
    .await;

    for (what, observation) in [("本文", facts.main()), ("抽出", facts.sub())] {
        assert_eq!(
            observation.answered_by(),
            None,
            "{what}: 前提: 往復そのものが失敗していること"
        );
        assert_eq!(
            observation.answered_name(),
            None,
            "{what}: 答えが無ければ、名前も無いこと"
        );
    }
}

/// **設定に書かれた認証情報は、保持したものに1文字も入らない**
/// （絶対制約: 応答に接続文字列を含めない / `server::upstream` の doc）。
///
/// # なぜ本番の入口を通すのか
///
/// **`normalize_base` は userinfo を弾かない**ので、
/// `PERSONACORE_LLM_ENDPOINT=http://user:pass@host:port` は**現に設定できる。**
/// **その綴りが `LmStudioConfig::endpoint` を経由して観測へ入る経路**は、
/// `App::bind` を通したときにしか現れない。
///
/// # 空振りではないこと
///
/// **その宛先で実際に往復していること**を先に固定する
/// ——認証情報つきの URL で上流に届かなければ、
/// この検査は「届かなかったから漏れなかった」を見ているだけになる。
///
/// # ⚠ 「型で表現できなくした」とは書けない
///
/// **止めているのは `UpstreamObservation::new` の1行**（`endpoint_for_view`。B-71 までは `redact_userinfo`）で、
/// **外せば綴りはそのまま入る。** 実際に外して測ってある
/// ——**この1本と `server::upstream` の `credentials_are_not_kept_in_the_endpoint` の 2 赤。**
#[tokio::test]
async fn the_held_facts_never_show_the_endpoints_credentials() {
    let upstream = upstream_reporting(MAIN_MODEL, MAIN_TOKENS).await;
    let host = upstream
        .uri()
        .strip_prefix("http://")
        .expect("モックサーバの綴りが http であること")
        .to_owned();
    let with_credentials = format!("http://{ENDPOINT_USER}:{ENDPOINT_PASSWORD}@{host}");

    let facts = facts_from_real_bind(&[
        (ENV_LLM_ENDPOINT, &with_credentials),
        (ENV_MAIN_MODEL, MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
    ])
    .await;

    // 前提: その宛先で実際に往復していること。
    assert_eq!(
        facts.main().answered_by(),
        Some(AnsweredBy::NativeApi),
        "前提: 認証情報つきの宛先でも上流に届いていること"
    );
    assert_eq!(
        facts.main().queried_context_tokens(),
        Some(MAIN_TOKENS),
        "前提: その往復から窓を読んでいること"
    );

    // 本題: 認証情報だけが落ちていて、宛先の同定は残る。
    assert_eq!(
        facts.main().endpoint(),
        format!("http://***@{host}/v1/chat/completions"),
        "認証情報だけを落とすこと"
    );
    let rendered = format!("{facts:?}");
    for secret in [ENDPOINT_USER, ENDPOINT_PASSWORD] {
        assert!(
            !rendered.contains(secret),
            "設定の認証情報を保持しないこと（{secret}）: {rendered}"
        );
    }
    assert!(
        rendered.contains(&host),
        "宛先の同定は残ること（丸ごと落とさない）: {rendered}"
    );
}

/// **継ぎ目は、上流が並べた綴りのままの一覧を突き合わせへ渡す**
/// （作業単位 B-45 のレビュー是正 / 2026-09-17）。
///
/// # なぜ要るのか —— **実測で、無防備だった**
///
/// **`server::context::probe_upstream` が一覧の各要素をファイル名に切って
/// [`UpstreamProbe::catalogue`] へ入れる変異は、着手前 2053 全緑だった。**
/// **等価変異ではない**——`measured::check` は**完全一致**で見るので、
/// **`vendor/model` 形の綴りは切られた一覧から消え、
/// 判定が `Drift::Other`（測り直す先が決まっている）から
/// `Drift::Absent`（誰が答えるかまだ分からない）へ倒れる。**
/// **調査先がまるで違う**（`measured::report` の doc）。
///
/// # 空振りではないこと
///
/// **切り落とした一覧で判定した答えが、実際の答えと違うこと**まで見る
/// ——同値なら、この1本は何も守っていない。
///
/// # ⚠ 答えた名前の側も、同じ回の中で見る
///
/// **`vendor/model` 形は切らない**（[`name_that_answered`] のレビュー是正）。
/// **単体テストはこの規則を見ているが、本番の入口を通る腕はここだけである。**
#[tokio::test]
async fn the_seam_hands_the_measured_check_the_catalogue_as_the_upstream_listed_it() {
    let upstream = upstream_listing_several_including(VENDOR_MAIN_MODEL).await;

    let facts = facts_from_real_bind(&[
        (ENV_LLM_ENDPOINT, &upstream.uri()),
        (ENV_MAIN_MODEL, VENDOR_MAIN_MODEL),
        (ENV_SUB_MODEL, SUB_MODEL),
    ])
    .await;

    // 前提: その一覧を実際に通していること。
    assert_eq!(
        facts.main().answered_by(),
        Some(AnsweredBy::CompatExactId),
        "前提: 依頼した綴りがそのまま一覧に在った回であること"
    );

    // 本題1: 答えた名前は、`vendor/model` 形のまま（本番の入口を通る腕）。
    assert_eq!(
        facts.main().answered_name(),
        Some(VENDOR_MAIN_MODEL),
        "パスに見えない綴りは切らないこと（切ると、違うものが答えたと画面が嘘をつく）"
    );

    // 本題2: 突き合わせは、上流が並べた綴りのままの一覧で行われている。
    let as_listed = judge_face(
        ModelFace::Main,
        VENDOR_MAIN_MODEL,
        Some(&[OTHER_MODEL_ID.to_owned(), VENDOR_MAIN_MODEL.to_owned()]),
    );
    let as_truncated = judge_face(
        ModelFace::Main,
        VENDOR_MAIN_MODEL,
        Some(&[
            OTHER_MODEL_ID_TRUNCATED.to_owned(),
            VENDOR_MAIN_MODEL_TRUNCATED.to_owned(),
        ]),
    );
    assert_ne!(
        as_listed, as_truncated,
        "空振り防止: 切り落とした一覧なら、別の答えになること"
    );
    assert_eq!(
        facts.main().measured(),
        as_listed,
        "継ぎ目は、上流が並べた綴りのままの一覧を突き合わせへ渡すこと"
    );
}
