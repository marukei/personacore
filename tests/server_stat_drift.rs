//! **性格ステータスの変容の配線**（[ADR-0024](../docs/adr/0024-stat-drift.md) / 作業単位 16b）の検証。
//!
//! 見ているのは「**本番の入口（`App::bind`）から、サーバの外から見て**変容が走ったか」だけ。
//! 分解・適用規則（許可リスト・対象外・飽和）は 16a の単体テストが、
//! 走らせ方の分岐（忙しい／軸ゼロ／長すぎる／上流が落ちた）は
//! `server::drift` の単体テストが、1軸だけを動かす文は
//! `tests/store_stat_drift.rs` が持つ。
//!
//! | 見るもの | 何が壊れたら赤になるか |
//! |---|---|
//! | **スイッチを on にすると**上流へ3往復目が行き、値が動き、**票が残る** | 設定が本番の入口で読まれていない |
//! | **`shadow` では**3往復目が行き、**票は残り、値は動かない** | 3値が2値に潰れている |
//! | **既定（off）では**2往復で止まり、値も票も残らない | 既定が on になっている／スイッチを無視している |
//! | 3往復目が**サブモデル**へ行く | 本文モデルで変容を回す |
//! | 3往復目に**このキャラクターの尺度**が載る | 尺度の帰属を落とす |
//! | 3往復目に**指定した腕の注記だけ**が載る（作業単位 16m） | 注記の設定を読み捨てる／腕を取り違える |
//!
//! # 3値は本番の入口でしか見分けられない（ADR-0026 §決定1）
//!
//! `shadow` は**3値になって初めて読めるようになった綴り**で、
//! 真偽値として読む実装では**起動そのものが失敗する。**
//! 「値が動かない」だけなら off と区別が付かないので、**票の側で分ける。**
//!
//! # なぜモック注入では見られないのか
//!
//! `App::bind_with_clients` は**テスト専用の入口**で、そこだけを通るテストは
//! **本番の入口の配線を一切見ない**（CLAUDE.md「テスト専用の入口を作ると、
//! 本番の入口が観測対象から外れる」——`context::resolve_budget` で実際に起きた）。
//! ここは `wiremock` のローカルサーバを立てて `App::bind` を通す。
//!
//! **期待値を既定値にしない。** 既定は off なので、
//! 「スイッチが届くこと」を見る側は **on** で測る
//! （既定で測ると、その行が何を渡していても緑になる）。
//!
//! # 実機の LM Studio には繋がない
//!
//! 上流は `wiremock`（docs/05-development.md）。

mod common;

use std::time::{Duration, Instant};

use personacore::domain::{
    CharacterId, DisplayName, Handle, Persona, StatAlloc, StatAxis, StatScale,
};
use personacore::server::{
    App, AppConfig, BackgroundProbe, BindAddr, ENV_DRIFT_FORM, ENV_DRIFT_NOTE, ENV_LLM_ENDPOINT,
    ENV_MAIN_MODEL, ENV_STAT_DRIFT, ENV_SUB_MODEL,
};
use personacore::store::Store;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

use common::fixture_scale;

/// 会話するキャラクターの識別子（`model` に入れる値 / ADR-0004）。
const HANDLE: &str = "aoi";

/// 会話するキャラクターの呼び名。**識別子とは別の綴り**（ADR-0011）。
const DISPLAY_NAME: &str = "葵";

/// **会話しない**キャラクターの識別子。会話する方とは別の綴り。
///
/// 1体しか作らないと `character.id` と `1` が常に同値になり、
/// **持ち主を決め打ちにする実装が原理的に観測できない**（CLAUDE.md）。
const BYSTANDER_HANDLE: &str = "kaede";

/// **会話しない**キャラクターの呼び名。
const BYSTANDER_DISPLAY_NAME: &str = "楓";

/// 変容が動かす軸。**他のどの軸とも十分違う綴り。**
const MOVED_AXIS: &str = "パッション";

/// 動かない軸。
const STILL_AXIS: &str = "ロジック";

/// [`MOVED_AXIS`] の初期値。**端ではない**（飽和ばかりが見える形にしない）。
///
/// # ⚠ **中点より下に置くこと**（ADR-0027 §9 / 16e-b）
///
/// **上げの費用は中点より上で基準を超える。** 尺度 7 の中点は 3.5 なので、
/// **4 に置くと同じ票でも値が動きにくくなる。**
/// ここは「本番の入口を通ると値が動く」ことを見る場所なので、**安い側に置く**
/// （費用の係数は `1 − 1.5×0.5/7 ≒ 0.89`）。
///
/// # ⚠ **1往復で動かすには `K` を下げる**（作業単位 16g）
///
/// **票の重みが `0.1/√n` になった**（ADR-0027 訂正 3）ので、
/// **既定の `K` では 1往復では届かない。** [`Harness::start_moving`] を使うこと。
const MOVED_START: u32 = 3;

/// [`STILL_AXIS`] の初期値。[`MOVED_START`] とは別の値。
const STILL_START: u32 = 2;

/// 利用者の発言。
const USER_MESSAGE: &str = "今日の発表、うまくいったよ。褒められた。";

/// **クライアントが指定する温度**（作業単位 A-7d / [`each_path_sends_its_own_temperature`]）。
///
/// # ⚠ 裏方の `0.8` と別の値でなければならない
///
/// **同値にすると、本文の経路と裏方の経路の取り違えが原理的に観測できない**
/// （CLAUDE.md の最頻の穴 —— 「フィクスチャが2つの値を同値にすると、
/// その取り違えは検出できない」）。
///
/// # ⚠ `f32` で正確に表せない値を選んである
///
/// **`0.3` は二進で割り切れない。** 送信本文を `serde_json::to_value` 経由で組む実装に
/// 変わると、線に乗る綴りが `0.30000001192092896` になって**ここが赤くなる**
/// （A-7c が塞いだ穴 / `llm::request` の `the_wire_bytes_spell_the_request_exactly`）。
/// **`0.25` のような割り切れる値にすると、その変異は本文の欄では通り抜ける。**
///
/// **⚠ ただし、この定数はその変異を捕まえる必要条件ではない**（A-7d のレビューが実測）——
/// **`0.25` にしても、この見張り番自体は赤のまま**である。
/// **裏方の `0.8` が同じ理由で外れる**から（`Number(0.800000011920929)` ≠ `Number(0.8)`）。
/// **つまり `0.3` は「本文の欄まで見る」ための上積みであって、唯一の門ではない。**
/// **⚠ 逆に言うと、裏方の期待値を割り切れる値へ動かす日には、
/// この上積みだけが門になる。** そのときは、ここを割り切れない値のままにしておくこと。
///
/// [`each_path_sends_its_own_temperature`]: each_path_sends_its_own_temperature
const CLIENT_TEMPERATURE: f64 = 0.3;

/// 読み出す票の件数の上限。**1往復で立つ票は高々1件**なので十分に大きい。
const VOTE_LIMIT: u32 = 16;

/// **本文生成の依頼だけに現れる見出し**（振り分けに使う）。
const PERSONA_HEADING: &str = "# ペルソナ";

/// **抽出の依頼だけに現れる見出し**（振り分けに使う）。
///
/// **`# 書き出し方` は使えない。** 変容の依頼にも同じ節がある（一度そう書いて踏んだ）。
/// 会話の節の見出しは、抽出が `# 会話`、変容が `# 直前のやり取り` で別々。
const EXTRACTION_HEADING: &str = "# 会話";

/// **変容の依頼だけに現れる見出し**（振り分けに使う）。
///
/// **`prompt::drift` の定数を呼ばず、テスト側に期待値として持つ**
/// （呼ぶと綴りを変える変異が両側で同時に動いて緑になる）。
const STATS_HEADING: &str = "# いまの性格";

/// 本文生成に指定するモデル。**既定とも変容用とも別の綴り。**
const MAIN_MODEL: &str = "ministral-3-14b-reasoning-2512";

/// 裏方に指定するモデル。**既定とも本文用とも別の綴り。**
const SUB_MODEL: &str = "ministral-3-3b-reasoning-2512";

/// **変容の依頼への答え。**
///
/// # ⚠ **全軸ぶんの行を返す**（作業単位 19）
///
/// **既定の依頼の形が `all` へ移った**（ADR-0027 §決定8 / 16j）ので、
/// **1行だけの答えは「行数が足りない」として捨てられる。**
/// ここは配線を見る場所なので、**既定の形で受理される答え**を返す
/// ——保存済みの2軸を、依頼に載る順で1行ずつ。
///
/// **[`MOVED_AXIS`] だけが動く**（`+1`）。
/// [`STILL_AXIS`] は `0` なので票が立たず、薄れる側に回る。
///
/// **1行の答えに戻さないこと。** 戻すと、この節の見張り番が
/// 「票が立つまで待つ」で 20 秒待ってから落ちる形になり、
/// **原因が「既定の形が変わった」であることが読み取れない。**
fn drift_scorecard() -> String {
    format!("- {MOVED_AXIS} +1\n- {STILL_AXIS} 0")
}

/// 上流が返す本文（OpenAI 互換）。
fn upstream_body(model: &str, content: &str) -> String {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop",
        }],
    })
    .to_string()
}

/// **約束・決まりの提案の依頼か**（作業単位 A-12c-1。検出 `jd-v3` と確かめ `c1-v1` の書き方の行）。
///
/// **`prompt` の定数を呼ばず、期待値として持つ**（`tests/server_agreement_proposals.rs` と同じ綴り）。
fn is_agreement_proposal(request: &Value) -> bool {
    let messages = request["messages"].to_string();
    [
        "抜｜引用｜取り決め｜守る側｜中身",
        "点検｜番号｜行い｜分類｜守る側｜判定｜中身",
    ]
    .iter()
    .any(|marker| messages.contains(marker))
}

/// 上流へ届いた依頼が3種のどれか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Main,
    Extraction,
    Drift,
}

/// 依頼の本文から種類を決める。**分からなければ落とす**（黙って数えない形にしない）。
fn kind_of(request: &Value) -> Kind {
    let messages = request["messages"].to_string();
    // **変容から先に見る。** 抽出の依頼も本文の依頼も `# いまの性格` を持たないので
    // 排他だが、順番に寄りかからないよう1つに絞れることを下で確かめる。
    let kinds: Vec<Kind> = [
        (STATS_HEADING, Kind::Drift),
        (EXTRACTION_HEADING, Kind::Extraction),
        (PERSONA_HEADING, Kind::Main),
    ]
    .into_iter()
    .filter(|(needle, _)| messages.contains(needle))
    .map(|(_, kind)| kind)
    .collect();

    assert_eq!(
        kinds.len(),
        1,
        "依頼の種類がちょうど1つに決まること: {kinds:?} / {messages}"
    );
    kinds[0]
}

/// 起動から停止までを持つテスト用のサーバ。
struct Harness {
    upstream: MockServer,
    store: Store,
    addr: std::net::SocketAddr,
    character: CharacterId,
    bystander: CharacterId,
    /// **裏方の枠が空いたかを見る口**（作業単位 A-8 / `App::background_probe`）。
    background: BackgroundProbe,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<Result<(), personacore::server::ServerError>>>,
    _dir: tempfile::TempDir,
}

impl Harness {
    /// **本番の入口（`App::bind`）で起動する。**
    ///
    /// `stat_drift` が `None` なら環境変数を与えない（＝既定）。
    async fn start(stat_drift: Option<&str>) -> Self {
        Self::start_with(stat_drift, None).await
    }

    /// **1往復で1段動く `K`** を与えて、本番の入口で起動する。
    ///
    /// # なぜ既定のつまみで測れないのか（作業単位 16g / **19 で数を更新**）
    ///
    /// **票の重みが `0.1/√n` になった**（ADR-0027 訂正 3）ので、
    /// **既定の `K = 0.3` でも 1往復では1段も動かない**
    /// ——尺度 7 の `費用(3, 上げ)` は `(0.3 × 10/7) × (1 − 1.5 × 0.5/7) ≒ 0.3827` で、
    /// **7 往復ぶんの票**が要る（それが訂正 3 の狙いそのものである）。
    /// **`server::drift` 側の「10 往復」は、あちらの `MOVED_START` が 4 だから**
    /// ——`費用(4, 上げ) ≒ 0.4745` は 1 段ぶん高い。**混ぜないこと。**
    ///
    /// ここが見ているのは**配線**（設定が本番の入口から上流と DB まで届くか）であって
    /// 費用曲線ではないので、**`K` を下げて1往復で観測できる形にする。**
    /// 既定のつまみで曲線を見る側は `server::drift` の
    /// `the_default_pricing_never_lifts_an_axis_nobody_mentioned`。
    ///
    /// `K = 0.06` で `費用(3, 上げ) ≒ 0.0765` ——1票（0.1）でちょうど1段。
    async fn start_moving(stat_drift: Option<&str>) -> Self {
        Self::start_tuned(
            stat_drift,
            None,
            &[(personacore::server::ENV_DRIFT_STEP_COST, "0.06")],
        )
        .await
    }

    /// **走らせ方と依頼の形**を指定して、本番の入口で起動する。
    ///
    /// どちらも `None` なら環境変数を与えない（＝既定）。
    async fn start_with(stat_drift: Option<&str>, drift_form: Option<&str>) -> Self {
        Self::start_tuned(stat_drift, drift_form, &[]).await
    }

    /// **走らせ方・依頼の形・集約のつまみ**を指定して、本番の入口で起動する。
    ///
    /// `knobs` は `(環境変数名, 値)` の並び。空なら既定（ADR-0027 §決定8）。
    async fn start_tuned(
        stat_drift: Option<&str>,
        drift_form: Option<&str>,
        knobs: &[(&str, &str)],
    ) -> Self {
        let upstream = MockServer::start().await;

        // **3種の依頼を、中身で振り分けて別々の答えを返す。**
        // 1つにまとめると「変容が走っていないのに本文の呼び出しで主張が満たされる」。
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/v1/chat/completions"))
            .and(matchers::body_string_contains(STATS_HEADING))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                upstream_body(SUB_MODEL, &drift_scorecard()),
                "application/json",
            ))
            .mount(&upstream)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/v1/chat/completions"))
            .and(matchers::body_string_contains(EXTRACTION_HEADING))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(upstream_body(SUB_MODEL, ""), "application/json"),
            )
            .mount(&upstream)
            .await;
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/v1/chat/completions"))
            .and(matchers::body_string_contains(PERSONA_HEADING))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                upstream_body(MAIN_MODEL, "「よかったですね」 彼女は湯呑みを置いた。"),
                "application/json",
            ))
            .mount(&upstream)
            .await;

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db_path = dir.path().join("personacore.sqlite");
        let endpoint = upstream.uri();
        let mut env: Vec<(&str, String)> = vec![
            (ENV_LLM_ENDPOINT, endpoint.clone()),
            (ENV_MAIN_MODEL, MAIN_MODEL.to_owned()),
            (ENV_SUB_MODEL, SUB_MODEL.to_owned()),
        ];
        if let Some(value) = stat_drift {
            env.push((ENV_STAT_DRIFT, value.to_owned()));
        }
        if let Some(value) = drift_form {
            env.push((ENV_DRIFT_FORM, value.to_owned()));
        }
        for (key, value) in knobs {
            env.push((key, (*value).to_owned()));
        }
        let config = AppConfig::from_lookup(|key| {
            env.iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(&db_path);

        // **`bind_with_clients` ではない。** ここが本番と同じ入口であることが要点。
        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let store = app.store().clone();
        let background = app.background_probe();

        // **会話しない方を先に作る**（＝ `id` が 1 になるのはこちら）。
        let bystander =
            create_character(&store, BYSTANDER_HANDLE, BYSTANDER_DISPLAY_NAME, &[]).await;
        let character = create_character(
            &store,
            HANDLE,
            DISPLAY_NAME,
            &[(MOVED_AXIS, MOVED_START), (STILL_AXIS, STILL_START)],
        )
        .await;
        assert_ne!(character, bystander, "前提: 2体の id が別であること");

        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });

        Self {
            upstream,
            store,
            addr,
            character,
            bystander,
            background,
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    /// **上流の依頼が `count` 件に届いてから、裏方の枠が空くまで待つ**（作業単位 A-8）。
    /// **どちらも上限付き**で、届かなければ落とす。
    ///
    /// # なぜ2段なのか（「A が B になる」を待つ形）
    ///
    /// **空いていることだけを待つと、裏方がまだ走り出していない（あるいは一度も走らない）回にも
    /// 即座に成立してしまう。** 先に上流への依頼の件数で「そのターンの裏方が走った」ことを
    /// 押さえ、その後で「終わった」を待つ。
    ///
    /// # 何が保証されるのか
    ///
    /// 空いた＝**そのターンの抽出と変容が最後まで終わった**（`BackgroundProbe::is_idle` は
    /// 本番の `Busy` の判定の否定そのもの）。したがって
    ///
    /// * 次のターンの裏方は `Busy` で飛ばない。
    /// * **「起きていないこと」を固定の猶予なしで主張できる**——変容が走るなら、
    ///   枠が空く前に上流へ依頼が届いている。
    async fn settle_after(&self, count: usize) -> Vec<Value> {
        self.wait_for_upstream(count).await;
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.background.is_idle() {
            assert!(
                Instant::now() < deadline,
                "裏方の枠が 20 秒たっても空かない（上流の依頼は {count} 件に届いている）"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.upstream_requests().await
    }

    /// 1往復投げる。**応答を読み切らないと保存も裏方も走らない。**
    ///
    /// **クライアントは温度を指定しない**（＝FE の既定の形）。
    /// 指定した回を作るのは [`Harness::chat_at_temperature`]。
    async fn chat(&self) {
        self.post(json!({
            "model": HANDLE,
            "messages": [{"role": "user", "content": USER_MESSAGE}],
        }))
        .await;
    }

    /// **クライアントが温度を指定した1往復**（作業単位 A-7d）。
    ///
    /// **裏方の温度と別の値を渡すこと。** 同じ値だと、
    /// **本文の経路と裏方の経路の取り違えが原理的に観測できない**
    /// （CLAUDE.md の最頻の穴 / `llm::SUB_LLM_TEMPERATURE` の doc）。
    async fn chat_at_temperature(&self, temperature: f64) {
        self.post(json!({
            "model": HANDLE,
            "messages": [{"role": "user", "content": USER_MESSAGE}],
            "temperature": temperature,
        }))
        .await;
    }

    /// 本文を1つ投げて、**読み切る。**
    async fn post(&self, body: Value) {
        let response = reqwest::Client::new()
            .post(format!("http://{}/v1/chat/completions", self.addr))
            .json(&body)
            .send()
            .await
            .expect("サーバへ接続できること");
        assert!(response.status().is_success(), "{:?}", response.status());
        let _ = response.text().await.expect("本文を読めること");
    }

    /// **会話するキャラクターの走らせ方を `PATCH` で書く**（ADR-0045 / 作業単位 A-8）。
    ///
    /// **本番の入口**（`PATCH /admin/characters/{handle}`）を通す——
    /// 店を直接書くと、要求の読み・検証・更新の配線を誰も通らない。
    /// `mode` は `"on"` などの綴りか `null`（未設定へ戻す）。
    async fn patch_mode(&self, mode: Value) {
        let response = reqwest::Client::new()
            .patch(format!("http://{}/admin/characters/{HANDLE}", self.addr))
            .json(&json!({"stat_drift_mode": mode}))
            .send()
            .await
            .expect("サーバへ接続できること");
        let status = response.status();
        let body = response.text().await.expect("本文を読めること");
        assert!(status.is_success(), "{status}: {body}");
    }

    /// 保存済みの累積の件数（**off では1行も作らない**ことを見る / ADR-0026）。
    async fn charge_rows(&self, id: CharacterId) -> usize {
        self.store
            .drift_charges(id)
            .await
            .expect("累積を読めること")
            .len()
    }

    /// 上流へ届いた依頼を順に返す。
    ///
    /// **約束・決まりの提案の依頼（作業単位 A-12c-1）は数えない。** 提案は同じ裏方の列の
    /// いちばん後ろで毎往復走るが、ここが見ているのは本文・抽出・変容の3種の配線で、
    /// 提案は変容の走らせ方に依らない（`server::extract::run`）。提案の依頼は
    /// `# 会話` の見出しを持つので、数えると抽出に読まれる。提案の配線は
    /// `tests/server_agreement_proposals.rs` が見る。
    async fn upstream_requests(&self) -> Vec<Value> {
        self.upstream
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|request| request.url.path() == "/v1/chat/completions")
            .map(|request| serde_json::from_slice(&request.body).expect("JSON として読めること"))
            .filter(|request: &Value| !is_agreement_proposal(request))
            .collect()
    }

    /// 上流の依頼が `count` 件になるまで待つ。**ならなければ落とす。**
    async fn wait_for_upstream(&self, count: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let requests = self.upstream_requests().await;
            if requests.len() >= count {
                return requests;
            }
            assert!(
                Instant::now() < deadline,
                "上流の依頼が {count} 件になること: {}",
                requests.len()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// 保存済みの軸を `(軸名, 値)` の組で読む。
    async fn stats(&self, id: CharacterId) -> Vec<(String, u32)> {
        self.store
            .stat_allocs(id)
            .await
            .expect("性格ステータスを読めること")
            .into_iter()
            .map(|alloc| (alloc.axis.as_str().to_owned(), alloc.value.get()))
            .collect()
    }

    /// 保存済みの票を `(軸名, 向き)` の組で、**新しい順**に読む。
    async fn votes(&self, id: CharacterId) -> Vec<(String, &'static str)> {
        self.store
            .recent_drift_votes(id, VOTE_LIMIT)
            .await
            .expect("票を読めること")
            .iter()
            .map(|stored| {
                (
                    stored.vote().axis().as_str().to_owned(),
                    stored.vote().delta().label(),
                )
            })
            .collect()
    }

    /// 票が `count` 件になるまで待つ。**ならなければ落とす。**
    async fn wait_for_votes(&self, id: CharacterId, count: usize) -> Vec<(String, &'static str)> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let votes = self.votes(id).await;
            if votes.len() >= count {
                return votes;
            }
            assert!(
                Instant::now() < deadline,
                "票が {count} 件になること: {votes:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// 軸が `expected` になるまで待つ。**ならなければ落とす。**
    async fn wait_for_stats(&self, expected: &[(&str, u32)]) {
        let want: Vec<(String, u32)> = expected
            .iter()
            .map(|(axis, value)| ((*axis).to_owned(), *value))
            .collect();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let got = self.stats(self.character).await;
            if got == want {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "軸が {want:?} になること: {got:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
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

/// キャラクターを1件作って id を返す。
async fn create_character(
    store: &Store,
    handle: &str,
    display_name: &str,
    stats: &[(&str, u32)],
) -> CharacterId {
    let handle = Handle::parse(handle).expect("正当な識別子であること");
    let id = store
        .create_character(
            &handle,
            &DisplayName::parse(display_name).expect("正当な表示名であること"),
            &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    if !stats.is_empty() {
        let allocs: Vec<StatAlloc> = stats
            .iter()
            .map(|(axis, value)| StatAlloc {
                axis: StatAxis::parse(axis).expect("正当な軸名であること"),
                value: fixture_scale()
                    .value(*value)
                    .expect("尺度に収まる値であること"),
            })
            .collect();
        store
            .replace_stat_allocs(id, &allocs)
            .await
            .expect("性格ステータスを置けること");
    }
    id
}

/// **スイッチを on にすると、本番の入口から変容が走る**（ADR-0024 §8）。
///
/// # 期待値を既定値にしない
///
/// 既定は **off** なので、「設定が届くこと」を見る側は **on** で測る。
/// 既定で測ると、`bind_with` のその行が何を渡していても緑になる
/// （CLAUDE.md「起動時に決めるものを増やしたら、本番の入口を通るテストを
/// 必ず1本足すこと。そのテストの期待値を既定値にしないこと」）。
#[tokio::test]
async fn the_stat_drift_switch_reaches_the_upstream_through_the_real_bind() {
    assert_ne!(MAIN_MODEL, SUB_MODEL, "前提: 2つは別の綴りであること");
    assert_ne!(
        SUB_MODEL,
        personacore::llm::DEFAULT_SUB_MODEL,
        "前提: 裏方モデルは既定と別の綴りであること"
    );

    let server = Harness::start_moving(Some("on")).await;
    server.chat().await;

    // **3往復目（変容）が上流へ届くまで待つ。**
    let requests = server.wait_for_upstream(3).await;
    assert_eq!(requests.len(), 3, "本文・抽出・変容で3往復であること");
    assert_eq!(kind_of(&requests[0]), Kind::Main, "1往復目は本文");
    assert_eq!(kind_of(&requests[1]), Kind::Extraction, "2往復目は抽出");
    assert_eq!(kind_of(&requests[2]), Kind::Drift, "3往復目は変容");

    // **裏方モデルへ行くこと**（ADR-0021 §10 の配線に相乗りしている）。
    assert_eq!(
        requests[2]["model"].as_str(),
        Some(SUB_MODEL),
        "変容もサブモデルへ行くこと: {:?}",
        requests[2]
    );

    // **このキャラクターの尺度が上限として載ること**（ADR-0015 / 帰属）。
    let drift = requests[2]["messages"].to_string();
    assert!(
        drift.contains(&format!("{MOVED_AXIS}: {MOVED_START}/{}", fixture_scale())),
        "自分の尺度が上限として載ること: {drift}"
    );
    assert!(
        !drift.contains(&format!("/{}", StatScale::DEFAULT.get())),
        "既定の尺度を載せないこと: {drift}"
    );

    // **値が実際に動くこと。**
    server
        .wait_for_stats(&[(MOVED_AXIS, MOVED_START + 1), (STILL_AXIS, STILL_START)])
        .await;
    assert!(
        server.stats(server.bystander).await.is_empty(),
        "会話していないキャラクターに軸を作らないこと"
    );

    server.stop().await;
}

/// **本文・抽出・変容の3経路とも、上流に「思考しない」を頼む**
/// （[ADR-0038](adr/0038-what-personacore-requires-of-an-upstream.md) §決定3 / 作業単位 A-7b）。
///
/// # なぜここに置くか
///
/// 思考モデルは既定で予算を全部思考に使い、`content` が 0 文字になる
/// （docs/archive/04-environment-lm-studio.md の実測）。**欄が1経路でも欠けると、その経路だけが
/// 静かに空になる** —— 本文なら「応答が空でした」で見えるが、
/// **抽出と変容は本文に出ない**ので、欠けても利用者からは正常に見える。
///
/// **モック注入では見られない。** `LlmClient` が受け取る `ChatRequest` に
/// この欄は無く、**載るのは上流へ出た HTTP 本文だけ**だから
/// （`the_sub_model_reaches_the_upstream_through_the_real_bind` と同じ理由）。
///
/// # この検査が見ているもの／見ていないもの
///
/// **見ている**: **3種がちょうど1回ずつ現れ、そのどれもがこの欄を持ち、値が `none` であること。**
/// つまり**「1経路だけ欄が落ちた」「1経路だけ値が変わった」を、落ちた経路の名前まで込みで赤にする**
/// （A-7b で実測。**抽出だけ／変容だけを落とす変異が、それぞれこの1本だけを赤にした** ——
/// 失敗の左辺が `[(Main, Some("none")), (Extraction, None), (Drift, Some("none"))]` の形で出る）。
/// 3件をまとめて「どれも `none`」と見る書き方では、**1経路だけ落ちた**のか
/// **経路そのものが走らなかった**のかを区別できない。
///
/// **⚠ 見ていない**: **経路と値の対応の取り違え。**
/// **3つの期待値がすべて `Some("none")` で同値**なので、
/// **並びを入れ替える変異は原理的に観測できない**
/// （`[(Main,…),(Extraction,…),(Drift,…)]` はどう並べ替えても同じ値の列になる）。
/// CLAUDE.md の最頻の穴——「**フィクスチャが2つの値を同値にすると、その取り違えは検出できない**」。
///
/// **いまは実害ゼロ。** `llm::request::RequestPolicy` は**全経路で同一の `const`** なので、
/// 取り違えようがない（取り違えても出力が変わらない）。
///
/// **⚠ 問題になるのは C-13**（[ADR-0038](adr/0038-what-personacore-requires-of-an-upstream.md) §決定2）。
/// **サンプラーを経路ごとに違う値で持たせた瞬間、取り違えが観測可能な欠陥になる。**
/// **そのときは、経路ごとに違う値をフィクスチャに入れないと、この1本は帰属を見張らない。**
/// **`assert_ne!` を置くだけでは足りない**——足すべきなのは「**経路ごとに違う値**」そのもの。
#[tokio::test]
async fn all_three_paths_ask_the_upstream_not_to_think() {
    let server = Harness::start_moving(Some("on")).await;
    server.chat().await;

    let requests = server.wait_for_upstream(3).await;
    assert_eq!(requests.len(), 3, "本文・抽出・変容で3往復であること");

    let seen: Vec<(Kind, Option<&str>)> = requests
        .iter()
        .map(|request| (kind_of(request), request["reasoning_effort"].as_str()))
        .collect();
    assert_eq!(
        seen,
        vec![
            (Kind::Main, Some("none")),
            (Kind::Extraction, Some("none")),
            (Kind::Drift, Some("none")),
        ],
        "3経路とも思考を切ること: {requests:?}"
    );

    server.stop().await;
}

/// **3つの経路が、それぞれ自分の温度を送る**
/// （[ADR-0038](../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定2 /
/// 作業単位 A-7d）。
///
/// # ここでしか見られないもの —— **帰属**
///
/// 上の `all_three_paths_ask_the_upstream_not_to_think` は
/// **3経路に同じ値を期待している**ので、**経路と値の取り違えを見ていない**
/// （その doc が自分でそう書いている）。**温度は経路ごとに値が違う最初の欄**なので、
/// **ここが「どの値がどの経路のものか」を見る唯一の場所**になる。
///
/// **見ている**: **本文はクライアントの依頼の値**（[`CLIENT_TEMPERATURE`]）、
/// **抽出と変容は personacore が明示する `0.8`** であること。
/// **本文の値を裏方へ／裏方の値を本文へ** 取り違える変異は、ここで赤くなる。
///
/// **⚠ 見ていない**: **抽出と変容の取り違え。**
/// **2経路が同じ `0.8` を使う**ので、入れ替えても出力は1バイトも変わらない
/// （`llm::SUB_LLM_TEMPERATURE` の doc。**経路ごとに違う温度にする日には、
/// ここの期待値を違う値にすること**）。
///
/// # モック注入では見られない
///
/// **本番の入口（`App::bind`）を通って、実際の HTTP 本文を見る。**
/// `App::bind_with_clients` はテスト専用の入口なので、そこだけを通るテストは
/// **本番の配線を一切見ない**（このファイルの冒頭の doc）。
///
/// # 期待値は実装の定数から作らない
///
/// **`llm::SUB_LLM_TEMPERATURE` を呼ばない。** 呼ぶと値を動かす変異が
/// 両側で同時に動いて緑になる（CLAUDE.md「関数の外にある値で押さえる見張り番」）。
/// **`0.8` を手で書く。**
#[tokio::test]
async fn each_path_sends_its_own_temperature() {
    assert_ne!(
        json!(CLIENT_TEMPERATURE),
        json!(0.8),
        "前提: クライアントの値と裏方の値が別であること（同値だと取り違えが見えない）"
    );

    let server = Harness::start_moving(Some("on")).await;
    server.chat_at_temperature(CLIENT_TEMPERATURE).await;

    let requests = server.wait_for_upstream(3).await;
    assert_eq!(requests.len(), 3, "本文・抽出・変容で3往復であること");

    let seen: Vec<(Kind, Option<Value>)> = requests
        .iter()
        .map(|request| (kind_of(request), request.get("temperature").cloned()))
        .collect();
    assert_eq!(
        seen,
        vec![
            (Kind::Main, Some(json!(CLIENT_TEMPERATURE))),
            (Kind::Extraction, Some(json!(0.8))),
            (Kind::Drift, Some(json!(0.8))),
        ],
        "本文はクライアントの値を、裏方は明示の 0.8 を送ること: {requests:?}"
    );

    server.stop().await;
}

/// **本文・抽出・変容の3経路とも、`presence_penalty` を `0` で明示する**
/// （[ADR-0038](../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定2 の
/// 2026-11-01 追記 / 作業単位 A-14 / origin#78）。
///
/// # なぜ経路ごとに見るか
///
/// **欄が1経路でも欠けると、上流によってはその経路だけにモデルカードの既定が黙って掛かる**
/// （鍵で守られた中継の先の推論サーバは思考オフの要求に `presence_penalty 1.5` を重ねていた）。
/// **抽出と変容は本文に出ない**ので、欠けても利用者からは見えない
/// （`all_three_paths_ask_the_upstream_not_to_think` と同じ理由）。
///
/// # 「載っていること」と「値が 0 であること」を別々に見る
///
/// 1本目の `assert_eq!` は**欄の有無だけ**を経路の名前と対にして並べ、
/// 2本目は**値**を並べる。**失敗の左辺から「どの経路で・落ちたのか変わったのか」が読める。**
///
/// **⚠ 見ていない**: **経路と値の取り違え。** 3経路が同じ `0` を持つので、
/// 入れ替えても出力は変わらない（`all_three_paths_ask_the_upstream_not_to_think` の doc と同じ）。
///
/// **期待値は実装の定数から作らない。** `0.0` を手で書く。
#[tokio::test]
async fn all_three_paths_send_presence_penalty_zero() {
    let server = Harness::start_moving(Some("on")).await;
    server.chat().await;

    let requests = server.wait_for_upstream(3).await;
    assert_eq!(requests.len(), 3, "本文・抽出・変容で3往復であること");

    let carried: Vec<(Kind, bool)> = requests
        .iter()
        .map(|request| (kind_of(request), request.get("presence_penalty").is_some()))
        .collect();
    assert_eq!(
        carried,
        vec![
            (Kind::Main, true),
            (Kind::Extraction, true),
            (Kind::Drift, true),
        ],
        "3経路とも presence_penalty を欄ごと載せること: {requests:?}"
    );

    let values: Vec<(Kind, Option<Value>)> = requests
        .iter()
        .map(|request| (kind_of(request), request.get("presence_penalty").cloned()))
        .collect();
    assert_eq!(
        values,
        vec![
            (Kind::Main, Some(json!(0.0))),
            (Kind::Extraction, Some(json!(0.0))),
            (Kind::Drift, Some(json!(0.0))),
        ],
        "3経路とも presence_penalty は 0 であること: {requests:?}"
    );

    server.stop().await;
}

/// **既定（未設定）では変容が走らない**（ADR-0024 §8 の「既定 off」）。
///
/// 既定を on にすると、**抽出の質を測っている最中に性格が動いてしまい、
/// どちらの効果かを切り分けられなくなる。**
///
/// # 「起きていないこと」を主張する前に、起きる側を通してある
///
/// 上の見張り番が同じ仕掛けで3往復目を観測しているので、
/// **「そもそも配線が動いていないから 2 件」ではない**と言える。
#[tokio::test]
async fn the_default_configuration_never_asks_for_a_drift() {
    let server = Harness::start(None).await;
    server.chat().await;

    // 抽出（2往復目）までは届く。**そこから先が来ないことを主張する。**
    let requests = server.wait_for_upstream(2).await;
    assert_eq!(kind_of(&requests[1]), Kind::Extraction, "2往復目は抽出");

    // 「起きていないこと」の猶予。**肯定側には使わない**（上の見張り番は締切まで poll する）。
    tokio::time::sleep(Duration::from_millis(500)).await;

    let requests = server.upstream_requests().await;
    assert_eq!(
        requests.len(),
        2,
        "既定では変容の依頼を投げないこと: {:?}",
        requests.iter().map(kind_of).collect::<Vec<_>>()
    );
    assert_eq!(
        server.stats(server.character).await,
        vec![
            (MOVED_AXIS.to_owned(), MOVED_START),
            (STILL_AXIS.to_owned(), STILL_START),
        ],
        "既定では値が動かないこと"
    );

    server.stop().await;
}

/// **明示的に off と書いた場合も走らない。**
///
/// 「未設定なら off」と「off と書いたら off」は別の経路
/// （前者は `from_lookup` の `None` 側、後者は語彙の照合）。
#[tokio::test]
async fn an_explicit_off_never_asks_for_a_drift() {
    let server = Harness::start(Some("off")).await;
    server.chat().await;

    server.wait_for_upstream(2).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let requests = server.upstream_requests().await;
    assert_eq!(requests.len(), 2, "off と書いたら走らないこと");
    assert_eq!(
        server.stats(server.character).await,
        vec![
            (MOVED_AXIS.to_owned(), MOVED_START),
            (STILL_AXIS.to_owned(), STILL_START),
        ],
    );

    server.stop().await;
}

/// **記録専用モードは、本番の入口から走って票を残し、値を動かさない**
/// **記録専用モードは、本番の入口から走って票を残し、値を動かさない**
/// （[ADR-0026](../docs/adr/0026-drift-by-accumulation.md) §決定1 / §決定3）。
///
/// # 期待値を既定値にしない
///
/// 既定は off なので、この綴りは**明示しないと絶対に現れない。**
/// しかも `shadow` は**3値になって初めて読めるようになった値**で、
/// 「真偽値として読む実装」では起動そのものが失敗する
/// （CLAUDE.md「起動時に決めるものを増やしたら、本番の入口を通るテストを必ず1本足すこと」）。
///
/// # ここでしか見られないもの
///
/// **設定が本番の入口で3値のまま `server::drift` へ届くこと。**
/// `App::bind_with_clients` はテスト専用の入口なので、そこだけを通るテストは
/// この配線を一切見ない。
///
/// # ⚠ **`on` の腕とまったく同じ `K` で回すこと**（作業単位 19 のレビュー指摘）
///
/// **かつてここは [`Harness::start`]（既定の `K`）で起動していた。**
/// **既定の `K` では `on` でも1往復では1段も動かない**
/// （[`MOVED_START`] の doc / 1票 `0.1` < `費用(3, 上げ)`）ので、
/// **「値が動かない」は `shadow` のおかげではなく、つまみのおかげ**だった。
/// **`16g` の「対比が両方『動かない』に潰れる」形そのもの。**
///
/// **レビュアーの実測**: `server::drift::apply_scores` の `mode.applying()` を
/// `StatDriftMode::On.applying()` に差し替える変異
/// ——**記録専用モードに許可証を渡す、この doc が名指ししているまさにその変異**——で
/// **赤くなったのは単体の2本だけで、この1本は緑を通った。**
///
/// **実害:** 本番の入口で `shadow` が黙って `on` として振る舞う経路を、
/// 統合側が一切見ていなかった。**記録専用モードの存在理由は決定性**なので、
/// **測定中に性格が動いても、票の数だけを見れば正常に見える。**
///
/// > **⚠ これは 19 が持ち込んだ回帰ではない。** 票の重みが `0.1/√n` になった
/// > `16g` の時点から死んでいた（**旧既定の `K = 1.0` でも同じ**）。
/// > **19 で直したのは、19 が `K` の既定を触った作業単位だから**である。
///
/// **[`Harness::start`] へ戻さないこと。** 戻すと、この1本は
/// 「`K` が高いので動かない」しか言えなくなる。
///
/// # ⚠ **「動かない」は待ってから見る**
///
/// **票は帳簿より先に書かれる**（`server::drift::run` は `record_votes` →
/// `apply_scores` の順）。**票が立った時点では、値を動かす側はまだ走っていない。**
/// だから**否定側の主張には猶予を置く**——このファイルの
/// `the_default_configuration_never_asks_for_a_drift` と同じ作法で、
/// **肯定側（`on` の腕）は締切まで poll する。**
#[tokio::test]
async fn the_shadow_mode_records_a_vote_through_the_real_bind() {
    // **`on` の腕と同じ安い `K`。** 違うのは走らせ方1つだけ。
    let server = Harness::start_moving(Some("shadow")).await;
    let before = server.stats(server.character).await;
    assert_eq!(
        before,
        vec![
            (MOVED_AXIS.to_owned(), MOVED_START),
            (STILL_AXIS.to_owned(), STILL_START),
        ],
        "前提: 動く前の値が分かっていること"
    );

    server.chat().await;

    // **3往復目（変容）が上流へ届く。** off とはここが違う。
    let requests = server.wait_for_upstream(3).await;
    assert_eq!(kind_of(&requests[2]), Kind::Drift, "3往復目は変容");

    // **票が立つまで待つ。**
    let votes = server.wait_for_votes(server.character, 1).await;
    assert_eq!(
        votes,
        vec![(MOVED_AXIS.to_owned(), "up")],
        "提案された軸・向きの票が1件残ること"
    );
    assert!(
        server.votes(server.bystander).await.is_empty(),
        "会話していないキャラクターに票を立てないこと"
    );

    // 「起きていないこと」の猶予。**肯定側には使わない**（doc の ⚠）。
    tokio::time::sleep(Duration::from_millis(500)).await;

    // **値は1バイトも動かない。**
    // **同じ `K` の `on` は同じ台本で 1 段動く**
    // （`the_on_mode_records_a_vote_and_moves_the_value_through_the_real_bind`）ので、
    // **ここが観測しているのは走らせ方の違いだけ**である。
    assert_eq!(
        server.stats(server.character).await,
        before,
        "記録専用モードでは値が動かないこと"
    );

    server.stop().await;
}

/// **on では票と適用の両方が起きる**（ADR-0026 §決定3）。
///
/// 記録専用との差は**値が動くこと1点だけ**で、票の側は同じ形で残る
/// ——違うと、記録専用で集めた列に規則を当て直しても `on` の実際とは別物になる。
#[tokio::test]
async fn the_on_mode_records_a_vote_and_moves_the_value_through_the_real_bind() {
    let server = Harness::start_moving(Some("on")).await;
    server.chat().await;

    server.wait_for_upstream(3).await;
    assert_eq!(
        server.wait_for_votes(server.character, 1).await,
        vec![(MOVED_AXIS.to_owned(), "up")],
        "記録専用のときと同じ形の票が残ること"
    );
    server
        .wait_for_stats(&[(MOVED_AXIS, MOVED_START + 1), (STILL_AXIS, STILL_START)])
        .await;

    server.stop().await;
}

/// **依頼の形のスイッチが、本番の入口から上流の依頼まで届く**
/// （[ADR-0027](../docs/adr/0027-drift-from-what-was-not-chosen.md) §12）。
///
/// # 期待値を既定値にしない
///
/// **既定は 2026-08-25 に `all` へ移った**（ADR-0027 §決定8 / 作業単位 19）ので、
/// **「設定が届くこと」を見る側は `single` で測る。**
/// 既定で測ると、`bind_with` のその行が何を渡していても緑になる
/// （CLAUDE.md「起動時に決めるものを増やしたら、本番の入口を通るテストを
/// 必ず1本足すこと。そのテストの期待値を既定値にしないこと」）。
///
/// **⚠ 向きを戻さないこと。** 既定が `all` になったあとで `all` を明示して `all` を期待すると、
/// **設定を読み捨てる実装と区別が付かない**（`16g` の「対比が潰れる」形そのもの）。
///
/// # 既定の側も同じ仕掛けで見る
///
/// **`single` を決め打ちする実装**は、`single` だけを見ていると通り抜ける。
/// 同じ台本を既定でもう一度回して、**そちらが全軸採点になっている**ことを見る。
///
/// # 文面は**テスト側に期待値として持つ**
///
/// `prompt::drift` の定数を呼ばない——呼ぶと綴りを変える変異が両側で同時に動いて緑になる
/// （このファイルの `STATS_HEADING` と同じ作法）。
#[tokio::test]
async fn the_drift_form_reaches_the_upstream_through_the_real_bind() {
    /// **全軸採点の依頼にだけ現れる語。**
    const ALL_FORM_MARK: &str = "1項目につき1行ずつ";
    /// **0 の手本。** 全軸採点にしか載らない。
    const ZERO_EXAMPLE: &str = "- 項目名 0";
    /// **1軸を尋ねる形にだけ現れる語。**
    const SINGLE_FORM_MARK: &str = "書けるのは1行だけです";

    // ---- **明示した側は非既定の `single`** ----
    let server = Harness::start_with(Some("on"), Some("single")).await;
    server.chat().await;

    let requests = server.wait_for_upstream(3).await;
    assert_eq!(kind_of(&requests[2]), Kind::Drift, "3往復目は変容");
    let drift = requests[2]["messages"].to_string();

    assert!(
        drift.contains(SINGLE_FORM_MARK),
        "1軸を尋ねる形の依頼になっていない: {drift}"
    );
    assert!(
        !drift.contains(ALL_FORM_MARK) && !drift.contains(ZERO_EXAMPLE),
        "全軸採点の文が混ざった: {drift}"
    );

    // **保存済みの軸はどちらの形でも載る**（どこから選ぶかを見せるため）。
    for axis in [MOVED_AXIS, STILL_AXIS] {
        assert!(drift.contains(axis), "軸が載っていない: {axis} / {drift}");
    }

    server.stop().await;

    // ---- **既定（未設定）では、同じ台本が全軸採点になる** ----
    let plain = Harness::start(Some("on")).await;
    plain.chat().await;
    let requests = plain.wait_for_upstream(3).await;
    let drift = requests[2]["messages"].to_string();
    assert!(
        drift.contains(ALL_FORM_MARK),
        "既定が全軸採点でない: {drift}"
    );
    assert!(
        drift.contains(ZERO_EXAMPLE),
        "既定に 0 の手本が無い: {drift}"
    );
    assert!(
        !drift.contains(SINGLE_FORM_MARK),
        "既定なのに1軸を尋ねる形の指示が載った: {drift}"
    );
    for axis in [MOVED_AXIS, STILL_AXIS] {
        assert!(drift.contains(axis), "軸が載っていない: {axis} / {drift}");
    }
    plain.stop().await;
}

/// **書き方の注記が、本番の入口から上流の依頼まで届く**
/// （[ADR-0029](../docs/adr/0029-language-as-a-matched-bundle.md) §訂正 / 作業単位 16m）。
///
/// # **3本の腕を1本のテストに置く**（CLAUDE.md「対比は片側だけでは足りない」）
///
/// | 腕だけを見ると通り抜ける実装 | どの腕が赤にするか |
/// |---|---|
/// | 設定を読み捨てて**常に注記なし** | `copy` / `language` |
/// | 設定を読み捨てて**常に `copy`** | `language` と**既定** |
/// | 設定を読み捨てて**常に `language`** | `copy` と**既定** |
/// | `copy` と `language` の**文面を取り違える** | `copy` / `language`（互いの文が出ない） |
///
/// **既定（未設定）を必ず含める。** `16g` で「既定を通る見張り番が黙って死ぬ」形が
/// 2本出たばかりで、**期待値が既定値のテストは、その行が何を渡していても緑**になる。
/// ここでは既定の腕の期待値が「**どちらの注記も1つも載らない**」なので、
/// **既定に寄せる決め打ちも、既定から外す決め打ちも、両方赤になる。**
///
/// # 文面は**テスト側に期待値として持つ**
///
/// `prompt::drift` の定数を呼ばない——呼ぶと綴りを変える変異が両側で同時に動いて緑になる
/// （このファイルの `STATS_HEADING` / `the_drift_form_reaches_the_upstream_through_the_real_bind`
/// と同じ作法）。
#[tokio::test]
async fn the_drift_note_reaches_the_upstream_through_the_real_bind() {
    /// **腕 Y の注記。** 複写の指示。
    const COPY_NOTE: &str = "軸名は、上の「いまの性格」に書かれている字をそのまま写してください。";
    /// **腕 Z の注記。** 言語の指示。
    const LANGUAGE_NOTE: &str = "日本語で、常用漢字で書いてください。";

    for (spelling, expected) in [
        (Some("copy"), Some(COPY_NOTE)),
        (Some("language"), Some(LANGUAGE_NOTE)),
        // **既定（未設定）。** どちらの注記も載らない。
        (None, None),
    ] {
        let knobs: Vec<(&str, &str)> = spelling
            .map(|value| vec![(ENV_DRIFT_NOTE, value)])
            .unwrap_or_default();
        let server = Harness::start_tuned(Some("on"), None, &knobs).await;
        server.chat().await;

        let requests = server.wait_for_upstream(3).await;
        assert_eq!(kind_of(&requests[2]), Kind::Drift, "3往復目は変容");
        let drift = requests[2]["messages"].to_string();

        for note in [COPY_NOTE, LANGUAGE_NOTE] {
            let wanted = expected == Some(note);
            assert_eq!(
                drift.contains(note),
                wanted,
                "腕 {spelling:?} で {note:?} の有無が違う: {drift}"
            );
        }

        server.stop().await;
    }
}

/// **既定（未設定）では票も残らない**（ADR-0026 §決定3）。
///
/// 判定そのものを走らせないので、**記録の経路に入らない。**
/// 「値が動かない」だけでは記録専用モードと区別が付かないので、
/// **票の側で分ける。**
#[tokio::test]
async fn the_default_configuration_records_no_vote() {
    let server = Harness::start(None).await;
    server.chat().await;

    server.wait_for_upstream(2).await;
    // 「起きていないこと」の猶予。**肯定側には使わない。**
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(
        server.votes(server.character).await.is_empty(),
        "既定では票を残さないこと"
    );

    server.stop().await;
}

/// **集約のつまみが、本番の入口から帳簿まで届く**（ADR-0027 §決定8）。
///
/// # ⚠ **「高い `K` では動かない」だけでは何も言えない**（作業単位 16g）
///
/// 票の重みが `0.1/√n` になった（ADR-0027 訂正 3）ので、
/// **既定のつまみでもこの台本は1往復では動かない**
/// ——尺度 7 の `費用(3, 上げ) ≒ 0.3827`（既定 `K = 0.3`） に対し、1票は `0.1` である。
/// **`K = 1000` と既定が同一結果になり、つまみが読まれているかを何も見ていなかった。**
/// レビュアーの実測: `ENV_DRIFT_STEP_COST` の綴りを差し替える変異で、この形は**緑を通った。**
///
/// # だから**対比**で見る —— 違うのはつまみ1つだけ
///
/// **同じ台本を、`K` だけが違う2つの起動に通して、結果が違うこと**を見る。
/// これで「つまみが読まれていない」も「つまみを読み捨てて既定で回す」も、
/// **どちらの向きの決め打ちも**赤になる。
///
/// **[`Harness::start_moving`] を使う2本では捕まらない変異がある** ——
/// **`K` を安い側に決め打ちする**実装（設定を無視して常に `0.06`）は、
/// あちらが期待する「値が動く」を満たしてしまう。**高い側が要る。**
///
/// # 票は残る
///
/// 判定と適用が分かれている（§決定7）ことが、**本番の入口を通る形で**観測できる場所である。
///
/// CLAUDE.md「起動時に決めるものを増やしたら、本番の入口を通るテストを必ず1本足すこと」。
#[tokio::test]
async fn the_drift_tuning_reaches_the_ledger_through_the_real_bind() {
    /// **高い `K`。** 同じ提案でも値は動かない。
    const DEAR: &str = "1000";
    /// **安い `K`。** 1票（0.1）が `費用(3, 上げ) ≒ 0.0638` を越える（2段目の `0.0791` には届かない）。
    ///
    /// **[`Harness::start_moving`] の `0.06` とはわざと別の値にしてある。**
    /// 同じ値にすると「2つは揃っていなければならない」と読まれるが、**揃える理由は無い**
    /// ——ここが要るのは「**どれか安い値**への決め打ちを捕まえること」であって、
    /// あちらの値そのものではない。
    const CHEAP: &str = "0.05";

    let unchanged = vec![
        (MOVED_AXIS.to_owned(), MOVED_START),
        (STILL_AXIS.to_owned(), STILL_START),
    ];
    let moved = vec![
        (MOVED_AXIS.to_owned(), MOVED_START + 1),
        (STILL_AXIS.to_owned(), STILL_START),
    ];
    assert_ne!(unchanged, moved, "前提: 2つの着地点は別であること");
    assert_ne!(DEAR, CHEAP, "前提: 振っているつまみは別の値であること");

    // ---- 高い `K`: 票は残るが、値は動かない ----
    let dear = Harness::start_tuned(
        Some("on"),
        None,
        &[(personacore::server::ENV_DRIFT_STEP_COST, DEAR)],
    )
    .await;
    dear.chat().await;

    // **3往復目（変容）は走る。** 止まっているのは値の方だけ。
    let requests = dear.wait_for_upstream(3).await;
    assert_eq!(kind_of(&requests[2]), Kind::Drift, "3往復目は変容");

    // **票は残る**（票が残るまで待てば、変容の処理は終わっている）。
    let votes = dear.wait_for_votes(dear.character, 1).await;
    assert_eq!(
        votes,
        vec![(MOVED_AXIS.to_owned(), "up")],
        "票が残っていない: {votes:?}"
    );
    let after_dear = dear.stats(dear.character).await;
    assert_eq!(
        after_dear, unchanged,
        "K を上げたのに値が動いた（つまみが本番の入口で読まれていない）"
    );
    dear.stop().await;

    // ---- 安い `K`: **つまみだけ**を差し替えて、同じ台本で値が動く ----
    let cheap = Harness::start_tuned(
        Some("on"),
        None,
        &[(personacore::server::ENV_DRIFT_STEP_COST, CHEAP)],
    )
    .await;
    cheap.chat().await;
    cheap.wait_for_upstream(3).await;
    let votes = cheap.wait_for_votes(cheap.character, 1).await;
    assert_eq!(
        votes,
        vec![(MOVED_AXIS.to_owned(), "up")],
        "安い K でも票の形は同じであること: {votes:?}"
    );
    cheap
        .wait_for_stats(&[(MOVED_AXIS, MOVED_START + 1), (STILL_AXIS, STILL_START)])
        .await;
    let after_cheap = cheap.stats(cheap.character).await;
    cheap.stop().await;

    // **つまみ1つで結果が変わること。** ここが「届いている」の中身である。
    assert_eq!(after_cheap, moved, "安い K で値が動かない");
    assert_ne!(
        after_dear, after_cheap,
        "K を振っても結果が変わらない（つまみが帳簿へ届いていない）"
    );
}

/// **つまみの綴りが読めなければ起動そのものが止まる**（ADR-0027 §決定8）。
///
/// 黙って既定へ落とすと、**測定者は振ったつもりで既定の結果を読む。**
/// `PERSONACORE_STAT_DRIFT` / `PERSONACORE_DRIFT_FORM` と同じ立場。
#[test]
fn an_unreadable_drift_knob_stops_the_startup() {
    let config = AppConfig::from_lookup(|key| {
        (key == personacore::server::ENV_DRIFT_SLOPE).then(|| "２".to_owned())
    });
    assert!(
        config.is_err(),
        "全角の数字が既定として読み流された（起動を止めること）"
    );
}

// ---------------------------------------------------------------------------
// キャラクターごとの走らせ方（ADR-0045 / 作業単位 A-8）
// ---------------------------------------------------------------------------
//
// **どの腕も「起動時の既定と違う値」をキャラクターに書く**（CLAUDE.md
// 「実行時に決まる値を入れたら、既定と違う値で回る腕を1本置く」）。
// 既定と同じ値を書く腕だけだと、キャラクターの値を読み捨てる実装が緑を通る。
//
// **未設定は既定に従う**側は、このファイルの既存のテストが全部そうである
// （`Harness` が作るキャラクターは未設定 / `create_character`）。

/// **既定が off（未設定）でも、キャラクターを `on` にすれば値が動く**
/// （ADR-0045 §決定2 / 配布物の利用者の形そのもの）。
///
/// **起動時の既定は立てない**（`PERSONACORE_STAT_DRIFT` を与えない＝off）。
/// 利用者は環境変数を知らないので、これが画面だけで変容を動かす経路である。
#[tokio::test]
async fn a_character_set_to_on_moves_its_value_while_the_default_is_off() {
    let server = Harness::start_moving(None).await;
    server.patch_mode(json!("on")).await;

    server.chat().await;

    let requests = server.wait_for_upstream(3).await;
    assert_eq!(kind_of(&requests[2]), Kind::Drift, "3往復目は変容");
    assert_eq!(
        server.wait_for_votes(server.character, 1).await,
        vec![(MOVED_AXIS.to_owned(), "up")],
        "票が残ること"
    );
    server
        .wait_for_stats(&[(MOVED_AXIS, MOVED_START + 1), (STILL_AXIS, STILL_START)])
        .await;
    assert!(
        server.votes(server.bystander).await.is_empty(),
        "会話していないキャラクターに票を立てないこと"
    );

    server.stop().await;
}

/// **既定が on でも、キャラクターを `off` にすれば判定も票も累積も1行も作らない**
/// （ADR-0045 §決定2 / ADR-0026 §決定3）。
///
/// **環境変数がキャラクターの値に勝つ実装**（ADR-0045 の却下欄）はここで赤になる。
/// **`on` の腕と同じ安い `K`** で起動する——既定の `K` では `on` でも1往復では動かないので、
/// 「値が動かない」がつまみのおかげになる（`the_shadow_mode_records_a_vote_through_the_real_bind`
/// の ⚠ と同じ）。
#[tokio::test]
async fn a_character_set_to_off_never_asks_for_a_drift_while_the_default_is_on() {
    let server = Harness::start_moving(Some("on")).await;
    server.patch_mode(json!("off")).await;
    let before = server.stats(server.character).await;

    server.chat().await;

    // **裏方が終わるまで待つ**（固定の猶予ではない / [`Harness::settle_after`]）。
    let requests = server.settle_after(2).await;
    assert_eq!(
        requests.iter().map(kind_of).collect::<Vec<_>>(),
        vec![Kind::Main, Kind::Extraction],
        "off のキャラクターでは変容の依頼を投げないこと"
    );
    assert!(
        server.votes(server.character).await.is_empty(),
        "票を残さないこと"
    );
    assert_eq!(
        server.charge_rows(server.character).await,
        0,
        "累積も1行も作らないこと"
    );
    assert_eq!(server.stats(server.character).await, before, "値も動かない");

    server.stop().await;
}

/// **既定が on でも、キャラクターを `shadow` にすれば票だけが残り、値は動かない**
/// （ADR-0045 §決定2 / ADR-0026 §決定1）。
///
/// **「走らせるか」はキャラクターの値で決め、「動かしてよいか」は起動時の既定で決める**
/// 取り違え（許可証の出どころが割れる形）はここでしか見えない——
/// その実装は `shadow` のキャラクターの値を既定 `on` の許可証で動かす。
#[tokio::test]
async fn a_character_set_to_shadow_records_but_never_moves_while_the_default_is_on() {
    let server = Harness::start_moving(Some("on")).await;
    server.patch_mode(json!("shadow")).await;
    let before = server.stats(server.character).await;

    server.chat().await;

    let requests = server.wait_for_upstream(3).await;
    assert_eq!(kind_of(&requests[2]), Kind::Drift, "3往復目は変容");
    assert_eq!(
        server.wait_for_votes(server.character, 1).await,
        vec![(MOVED_AXIS.to_owned(), "up")],
        "票は残ること"
    );
    // **票は帳簿より先に書かれる**ので、裏方が終わるまで待ってから値を見る。
    server.settle_after(3).await;
    assert_eq!(
        server.stats(server.character).await,
        before,
        "記録専用のキャラクターでは値が動かないこと"
    );

    server.stop().await;
}

/// **同じサーバで `PATCH` すると、次のターンから効く。起こし直さない**（ADR-0045 §決定2）。
///
/// # 3ターンを1つのサーバで回す
///
/// | ターン | 走らせ方 | 期待 |
/// |---|---|---|
/// | 1 | 未設定（既定 off） | 変容の依頼なし |
/// | 2 | `PATCH` で `on` | 変容が走り、値が動く |
/// | 3 | `PATCH` で `null`（未設定へ戻す） | 変容の依頼なし（既定へ戻る） |
///
/// **1ターン目が要る**——「最初の往復で走らせ方を覚えて使い回す」実装は、
/// 1ターン目の前に `PATCH` するだけでは区別できない。
///
/// # ⚠ ターンの間で、裏方が空くのを待つ
///
/// 裏方の枠は同時に1本だけで、**前のターンの裏方が走っている間に来たターンは裏方ごと飛ぶ**
/// （`Busy` / ADR-0021 §7）。飛んだターンは変容を走らせないので、
/// **「次のターン」が成立しない。** かつては固定の 500ms を置いていた
/// （A-8 のレビュー指摘で、枠が空いたことを上限付きで待つ形 [`Harness::settle_after`] に替えた）。
#[tokio::test]
async fn switching_the_mode_takes_effect_from_the_next_turn_without_a_restart() {
    let server = Harness::start_moving(None).await;

    // --- 1ターン目: 未設定（既定 off） ---
    server.chat().await;
    assert_eq!(
        server.settle_after(2).await.len(),
        2,
        "既定 off では走らない"
    );

    // --- 2ターン目: `on` ---
    server.patch_mode(json!("on")).await;
    server.chat().await;
    server.settle_after(5).await;
    assert_eq!(
        server.stats(server.character).await,
        vec![
            (MOVED_AXIS.to_owned(), MOVED_START + 1),
            (STILL_AXIS.to_owned(), STILL_START),
        ],
        "2ターン目で値が動くこと"
    );
    assert_eq!(
        server.votes(server.character).await,
        vec![(MOVED_AXIS.to_owned(), "up")],
        "2ターン目で票が1つ立つこと"
    );

    // --- 3ターン目: 未設定へ戻す ---
    server.patch_mode(Value::Null).await;
    server.chat().await;

    assert_eq!(
        server
            .settle_after(7)
            .await
            .iter()
            .map(kind_of)
            .collect::<Vec<_>>(),
        vec![
            Kind::Main,
            Kind::Extraction,
            Kind::Main,
            Kind::Extraction,
            Kind::Drift,
            Kind::Main,
            Kind::Extraction,
        ],
        "変容は2ターン目だけで走ること"
    );
    assert_eq!(
        server.votes(server.character).await.len(),
        1,
        "3ターン目は票を足さないこと"
    );
    assert_eq!(
        server.stats(server.character).await,
        vec![
            (MOVED_AXIS.to_owned(), MOVED_START + 1),
            (STILL_AXIS.to_owned(), STILL_START),
        ],
        "未設定へ戻しても、動いた値はそのまま（切り替えは値に触らない / ADR-0045 §決定6）"
    );

    server.stop().await;
}
