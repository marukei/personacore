//! **実測由来の定数と、上流が応答するモデルの突き合わせが、本番の入口に配線されている**
//! ことの検証（[ADR-0033](../docs/adr/0033-measured-constants-name-what-they-were-measured-on.md)
//! §決定1 / 作業単位 E-2）。
//!
//! # なぜ専用のテストバイナリなのか
//!
//! 見たいのは **`App::bind`（本番の入口）が出す行**で、観測には
//! `set_global_default` が要る。**それはプロセスにつき1回しか呼べない**ので、
//! cargo が `tests/*.rs` ごとに別プロセスにすることを使って観測窓を1つ増やす
//! （`tests/server_context_refresh.rs` と同じ手）。
//!
//! # ⚠ フィクスチャは、綴りを1つも同値にしない
//!
//! **メインとサブに別の綴りを与える。** 既定どうしは同じ綴り
//! （`DEFAULT_MAIN_MODEL` == `DEFAULT_SUB_MODEL`）なので、
//! **既定のまま回すと「サブの定数をメインの綴りと突き合わせる」取り違えが観測できない**
//! （CLAUDE.md の最頻の穴 / `personacore::measured::requested_for` の doc）。
//!
//! **測ったときの相手（`ministral-3-14b-instruct-2512`）とも別の綴りにする**腕を置く。
//! 同値にすると「ずれ」と「一致」が区別できない。
//!
//! # ⚠ ここで見ていないこと
//!
//! * **実機に繋がない。** 上流は必ずローカルのモックサーバ。
//!   実機で本当に一覧が返るかは `#[ignore]` の器具の側
//!   （`tests/prompt_estimate_calibration.rs` と同じ作法）。
//! * **定数の値が正しいこと**は、この配線とは無関係（`measured` モジュールの doc）。
//!
//! # ⚠ 数はべた書きにしてある
//!
//! まとめの行の `measured_checked` / `measured_matched` / `measured_drifted` /
//! `measured_unanswered` は**一覧から計算せずに書いてある**（9 / 4 / 5 / 0 など）。
//! **一覧から作ると、一覧が緩んだときに期待値も一緒に動いて緑のまま**になる
//! （CLAUDE.md「関数の外にある値で押さえる見張り番」）。
//!
//! **`measured::MEASURED_CONSTANTS` に項目を足したら、ここも直すこと。**
//! 内訳は「両方の面で見る定数 × 2 ＋ サブだけの定数 × 1」。

use std::time::{Duration, Instant};

use personacore::llm::DEFAULT_MAIN_MODEL;
use personacore::server::{
    App, AppConfig, BindAddr, DEFAULT_LOG_FILTER, ENV_CONTEXT_TOKENS, ENV_LLM_ENDPOINT,
    ENV_MAIN_MODEL, ENV_SUB_LLM_ENDPOINT, ENV_SUB_MODEL, ServerError,
};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

mod common;

use common::{SharedBuffer, has_field, install_global_log};

/// 突き合わせのまとめの行の手掛かり。**全文一致にしない。**
const SUMMARY_MARKER: &str = "突き合わせました";

/// ずれを告げる行の手掛かり。
const DRIFT_MARKER: &str = "別のモデルで測ったものです";

/// 依頼した綴りが上流に無いことを告げる行の手掛かり。
const ABSENT_MARKER: &str = "モデル一覧にありません";

/// **メインの面の1本が代弁する定数の綴り**（作業単位 G-5）。
///
/// `Drift::Absent` の行は**面 × 測ったときの相手ごとに1本**へ畳んである
/// （`personacore::measured::absent_groups`）ので、
/// **どの定数を代弁しているかは、この欄にしか出ない。**
///
/// **⚠ べた書き**（このファイルの冒頭の doc と同じ理由——
/// `MEASURED_CONSTANTS` から作ると、一覧が緩んだときに期待値も一緒に動く）。
/// **一覧に項目を足したら、ここも直すこと。**
const ABSENT_MAIN_NAMES: &str = "measured_constant_names=MEASURED_CONTROL_TOKEN_SPELLINGS,\
     MESSAGE_OVERHEAD_TOKENS,GOLDEN,GOLDEN_BASELINE";

/// **サブの面の1本が代弁する定数の綴り。** メインの4件 ＋ サブ専用の1件。
const ABSENT_SUB_NAMES: &str = "measured_constant_names=MEASURED_CONTROL_TOKEN_SPELLINGS,\
     MESSAGE_OVERHEAD_TOKENS,GOLDEN,GOLDEN_BASELINE,MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS";

/// **一覧の全項目 × 全面**を、報告の行の上で1つずつ当てるための表（E-2 の是正 / 2026-08-28）。
///
/// 列は `(定数名, 場所, 何に依存するか, 面, その面が依頼する綴り)`。
/// **9行あるのは「両方の面で見る定数 4 件 × 2 ＋ サブだけの定数 1 件」**だから。
///
/// # ⚠ すべてべた書きにしてある
///
/// `personacore::measured::MEASURED_CONSTANTS` から作ると、
/// **一覧が緩んだときに期待値も一緒に動いて緑のまま**になる
/// （CLAUDE.md「関数の外にある値で押さえる見張り番」）。
///
/// # ⚠ 代表1つで止めないこと（**一度そうなっていた**）
///
/// 是正前はサブ専用の1件しか見ておらず、
/// **`measured_at` を `"src/server/extract.rs"` に固定する変異が 1685 全緑**を通った。
/// **落ちていた先に `MEASURED_CONTROL_TOKEN_SPELLINGS` が含まれていた**のが重い——
/// ADR-0033 §決定3 が「**最優先。ほかは後でよい。これだけは性質が違う**」と名指しした行で、
/// **上流を替えると実際に間違いになることが実機で確かめられている**
/// （Qwen3.5-9B では `<|im_start|>` が 1 トークン ＝ 本物の制御トークン / 2026-08-28。
/// **当時は `prompt::safe` の規則が `|` を許しておらず、無害化が素通りしていた**
/// —— **規則は B-22 で広げた**が、**この一覧が間違った場所を指す危険は消えていない**）。
/// **その警告を読んだ人が、間違った `measured_at` に従って別のファイルを開くことになる。**
///
/// **一覧に項目を足したら、ここにも行を足すこと**（足さないと、その定数の欄は誰も見ない）。
const EXPECTED_ROWS: [(&str, &str, &str, &str, &str); 9] = [
    (
        "MEASURED_CONTROL_TOKEN_SPELLINGS",
        "src/prompt/safe.rs",
        "トークナイザ",
        "main",
        OTHER_MAIN,
    ),
    (
        "MEASURED_CONTROL_TOKEN_SPELLINGS",
        "src/prompt/safe.rs",
        "トークナイザ",
        "sub",
        OTHER_SUB,
    ),
    (
        "MESSAGE_OVERHEAD_TOKENS",
        "src/prompt/estimate.rs",
        "チャットテンプレート",
        "main",
        OTHER_MAIN,
    ),
    (
        "MESSAGE_OVERHEAD_TOKENS",
        "src/prompt/estimate.rs",
        "チャットテンプレート",
        "sub",
        OTHER_SUB,
    ),
    (
        "GOLDEN",
        "tests/prompt_estimate_calibration.rs",
        "トークナイザ",
        "main",
        OTHER_MAIN,
    ),
    (
        "GOLDEN",
        "tests/prompt_estimate_calibration.rs",
        "トークナイザ",
        "sub",
        OTHER_SUB,
    ),
    (
        "GOLDEN_BASELINE",
        "tests/prompt_estimate_calibration.rs",
        "チャットテンプレート",
        "main",
        OTHER_MAIN,
    ),
    (
        "GOLDEN_BASELINE",
        "tests/prompt_estimate_calibration.rs",
        "チャットテンプレート",
        "sub",
        OTHER_SUB,
    ),
    (
        "MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS",
        "src/server/extract.rs",
        "モデルの饒舌さ",
        "sub",
        OTHER_SUB,
    ),
];

/// **サブの下でだけ意味を持つ定数**の欄（帰属を見る本命）。
///
/// 一覧の中で**唯一 `MeasuredRole::Sub` の項目**なので、
/// 「サブの綴りで突き合わせているか」を単独で観測できる。
const SUB_ONLY_CONSTANT: &str = "measured_constant=MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS";

/// ログを待つ上限。超えたら**落とす**（緑のまま何も測らない形にしない）。
const WAIT_LIMIT: Duration = Duration::from_secs(20);

/// 待ちの刻み。
const POLL_STEP: Duration = Duration::from_millis(10);

/// **一致の腕**で使うメインの綴り。**測ったときの相手そのもの。**
const MEASURED_MAIN: &str = DEFAULT_MAIN_MODEL;

/// **ずれの腕**で使うメインの綴り。**測ったときの相手とも、下のサブとも別。**
const OTHER_MAIN: &str = "arm-b-main-model";

/// **ずれの腕**で使うサブの綴り。**メインと必ず違えること**（帰属の観測点）。
const OTHER_SUB: &str = "arm-b-sub-model";

/// **一致の腕**で使うサブの綴り。**メインと違え、かつ上流の一覧には載せる。**
const MEASURED_ARM_SUB: &str = "arm-a-sub-model";

/// **「上流に無い綴り」の腕**で使うメイン。
const UNLISTED_MAIN: &str = "arm-c-main-model";

/// **「上流に無い綴り」の腕**で使うサブ。
const UNLISTED_SUB: &str = "arm-c-sub-model";

/// **「聞けなかった」の腕**で使うメイン。
const SILENT_MAIN: &str = "arm-d-main-model";

/// **「聞けなかった」の腕**で使うサブ。
const SILENT_SUB: &str = "arm-d-sub-model";

/// **「一覧が空」の腕**で使うメイン。
const EMPTY_MAIN: &str = "arm-e-main-model";

/// **「一覧が空」の腕**で使うサブ。
const EMPTY_SUB: &str = "arm-e-sub-model";

/// **「上流が2つ」の腕**で使うメイン（作業単位 G-3a / ADR-0039 §決定1）。
const SPLIT_MAIN: &str = "arm-f-main-model";

/// **「上流が2つ」の腕**で使うサブ。**メインと必ず違えること。**
const SPLIT_SUB: &str = "arm-f-sub-model";

/// **「同じ URL を2つの欄で名乗った」腕**で使うメイン（G-3a の是正）。
const SAME_MAIN: &str = "arm-g-main-model";

/// **「同じ URL を2つの欄で名乗った」腕**で使うサブ。
const SAME_SUB: &str = "arm-g-sub-model";

/// **「別々の URL ＋ 明示設定」の腕**で使うメイン（G-3a の是正）。
const APART_MAIN: &str = "arm-h-main-model";

/// **「別々の URL ＋ 明示設定」の腕**で使うサブ。
const APART_SUB: &str = "arm-h-sub-model";

/// **本文の側の照会を止めるための明示設定**（`PERSONACORE_CONTEXT_TOKENS`）。
///
/// **既定の予算（`TokenBudget::DEFAULT_CONTEXT_TOKENS`）とも、
/// 上流が申告する値とも別の綴りにしてある**——同値だと
/// 「設定を読まずに落とし所を使う」実装と区別が付かない
/// （CLAUDE.md / B-7「実行時に決まる値を入れたら、既定と違う状態で回る腕を1本置く」）。
const EXPLICIT_CONTEXT_TOKENS: &str = "12288";

/// 実機と同じ形の `/api/v0/models` の1件分。
///
/// **`state` は `not-loaded`。** 冷えた上流でも判定が変わらないことが要点
/// （未ロードは「これから載る」であって「別のモデルが答える」ではない）。
const ENTRY: &str = r#"{"id": "PLACEHOLDER", "object": "model", "type": "llm",
     "state": "not-loaded", "max_context_length": 262144}"#;

/// プロセスで1つだけのログバッファ。**`set_global_default` はここで1回だけ。**
fn shared_log() -> &'static SharedBuffer {
    install_global_log(DEFAULT_LOG_FILTER)
}

/// **欄が全部そろっている行**を1本選ぶ。**無ければ `None`**（待ちの1周ぶん）。
///
/// # 判定は `common::has_field` に預けてある（作業単位 B-36）
///
/// **選ぶ側と主張する側の受理集合を揃える**ため
/// （`src/testlog/assert.rs` の `line_with_field` の doc が権威。ここに写さない）。
///
/// **⚠ ここは `contains` にしないこと。** B-36 より前は、欄を鍵にする待ちが
/// **素の部分一致**で行を選んでいた。**`measured_constant=GOLDEN` は
/// `measured_constant=GOLDEN_BASELINE` の行にも当たり**、
/// **`checked_main=…` は `x_checked_main=…` の行にも当たる。**
/// **一覧には接頭辞を共有する2つの定数が実在する。**
///
/// **⚠ `all` を `any` にしないこと。** ログのバッファはプロセスで1つで、
/// **5つの腕の行が混ざる**——1つの欄だけで拾うと**別の腕の行を掴んで緑になる**
/// （実測 / E-2 の着手時に実際に踏んだ: サブ専用の定数の行を、隣の腕のものと取り違えた）。
///
/// 見張り番は [`the_line_picker_matches_every_field_as_a_whole_token`]。
fn line_with_all_fields<'a>(logged: &'a str, fields: &[&str]) -> Option<&'a str> {
    logged
        .lines()
        .find(|line| fields.iter().all(|field| has_field(line, field)))
}

/// **行を選ぶ側が、欄をトークンとして丸ごと見ている**（作業単位 B-36）。
///
/// # 入力は**植えた行**（`tracing` も上流も通さない）
///
/// 本番のログに囮の欄は作れない。**行も期待値もべた書き。**
///
/// # 囮は3つとも、実際に起こりうる形
///
/// | 囮 | 素の `contains` だと |
/// |---|---|
/// | **キーの側が伸びた**（`x_checked_main=…`） | 掴む。**E-2 が現に踏んだ形**（`checked_sub_model=` を足したら赤くなった） |
/// | **値の側が伸びた**（`…=arm-a-longer`） | 掴む。**`GOLDEN` と `GOLDEN_BASELINE` が実在する** |
/// | **片方の欄しか合わない** | `all` を `any` へ緩める変異が掴む |
///
/// **囮を全部、狙いの行より手前に置いてある**——後ろに置くと、
/// 壊れた実装でも偶然正しく答える。
///
/// # **向きも同時に固定する**（作業単位 B-36 のレビュー指摘 / 2026-09-02）
///
/// **欄が全部そろう行を2本**置き、**期待値は先に出た方**にしてある。
///
/// | 向きを変える変異 | 1本しか無い版 | いま（2本） |
/// |---|---|---|
/// | `.find(..)` → `.filter(..).last()` | **1831 全緑** | **1 赤**（この見張り番だけ） |
/// | `.lines()` → `.lines().rev()` | **1831 全緑** | **1 赤**（同上） |
///
/// **2026-09-02 に4回とも実測。候補が1本なら、どの向きでも同じ答えになる。**
///
/// **⚠ いま実害はゼロ**（腕ごとに綴りを違えてあるので、本番の候補は常に1本）。
/// **だがそれはフィクスチャの一意性に寄りかかった無防備**で、
/// `report()` が1プロセスで2回走る／再試行が入る／腕が増えて値が衝突する、
/// のどれかが起きた瞬間に向きが効き始める。
/// **`src/testlog/assert.rs` の `line_with_field` が同じ理由で同じ形を置いている**
/// （**あちらが権威。理由をここに書き写さない**）。
#[test]
fn the_line_picker_matches_every_field_as_a_whole_token() {
    const PLANTED: &str = "\
2026-09-02T00:00:00Z  INFO personacore::m: 囮 x_checked_main=arm-a measured_face=main
2026-09-02T00:00:01Z  INFO personacore::m: 囮 checked_main=arm-a-longer measured_face=main
2026-09-02T00:00:02Z  INFO personacore::m: 囮 checked_main=arm-a measured_face=sub
2026-09-02T00:00:03Z  INFO personacore::m: 狙い checked_main=arm-a measured_face=main
2026-09-02T00:00:04Z  INFO personacore::m: 二本目 checked_main=arm-a measured_face=main";

    // **前提**——囮と狙いが互いに別の行であること（同値だと何も測れない）。
    let lines: Vec<&str> = PLANTED.lines().collect();
    assert_eq!(lines.len(), 5, "前提: 5本の行を植えてあること");
    for (at, line) in lines.iter().enumerate() {
        for other in &lines[at + 1..] {
            assert_ne!(line, other, "前提: 植えた行が互いに違うこと");
        }
    }
    // **前提**——欄が全部そろう行が**2本**あること（1本だと向きを観測できない）。
    // **数えるのに `has_field` を使わない**（見張る相手そのものなので自己成就する）。
    // **先頭の空白まで含めて数える**——付けないと `x_checked_main=…` の囮も数に入る。
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains(" checked_main=arm-a measured_face=main"))
            .count(),
        2,
        "前提: 欄が全部そろう行が2本あること（向きを観測するため）"
    );

    assert_eq!(
        line_with_all_fields(PLANTED, &["checked_main=arm-a", "measured_face=main"]),
        Some(
            "2026-09-02T00:00:03Z  INFO personacore::m: 狙い checked_main=arm-a measured_face=main"
        ),
        "囮を飛び越えて、欄が全部そろった行のうち先に出た方を返すこと"
    );

    // 否定: その欄を持つ行が無ければ、行を捏造しない。
    assert_eq!(
        line_with_all_fields(PLANTED, &["checked_main=arm-b"]),
        None,
        "無い欄に対して行を返さないこと"
    );
}

/// **欄が全部そろっている**行が出るまで待つ。**出なければ落とす。**
///
/// 行を選ぶのは [`line_with_all_fields`]。ここは**待つだけ。**
async fn wait_for_fields(fields: &[&str]) -> String {
    let deadline = Instant::now() + WAIT_LIMIT;
    loop {
        let logged = shared_log().contents();
        if let Some(line) = line_with_all_fields(&logged, fields) {
            return line.to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "{fields:?} を全部欄として持つ行が出なかった: {logged:?}"
        );
        tokio::time::sleep(POLL_STEP).await;
    }
}

/// **その綴りを持っていると答える上流。**
async fn upstream_listing(models: &[&str]) -> MockServer {
    let entries: Vec<String> = models
        .iter()
        .map(|id| ENTRY.replace("PLACEHOLDER", id))
        .collect();
    let body = format!(r#"{{"object":"list","data":[{}]}}"#, entries.join(","));

    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(&server)
        .await;
    server
}

/// **モデル一覧を持たない上流**（LM Studio 以外の実装 / 停止中に相当）。
///
/// `/api/v0/models` は **LM Studio 独自**なので、他の上流では 404 になる。
/// **それは異常ではない**——「聞けなかった」に落ちること自体が主張。
async fn upstream_without_the_native_api() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    server
}

/// **`App::bind`（本番の入口）で起動した実サーバ。** モックは注入しない。
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

/// 突き合わせのまとめの行を、**この腕の綴りで**名指しで拾う。
///
/// ログのバッファはプロセスで1つで、**5つの腕のまとめが混ざる**
/// （**「4つ」と書いてあったのを B-36 で数え直した。腕は 5 本ある**）。
/// 綴りは腕ごとに違えてあるので、`checked_main` の値で自分の行が引ける。
///
/// **鍵は欄なので [`wait_for_fields`] で拾う**（作業単位 B-36）。
async fn summary_for(main: &str) -> String {
    let checked_main = format!("checked_main={main}");
    let line = wait_for_fields(&[checked_main.as_str()]).await;
    assert!(
        line.contains(SUMMARY_MARKER),
        "突き合わせのまとめの行であること: {line:?}"
    );
    line
}

/// **測ったときの相手がメインなら、メインの面は全部「一致」になる**（本番の入口）。
///
/// # 何を押さえているか
///
/// * 突き合わせが**実際に走った**こと（まとめの行が出る）。
/// * **「ずれ」に潰す変異が赤くなる**——この腕のメインの綴りでは警告が出てはいけない。
/// * **サブの綴りは別**なので、サブ専用の定数は「ずれ」として数えられる
///   ——**一致とずれが同じ回に両方出る**ので、片側へ潰す変異はどちらの向きも捕まる。
#[tokio::test]
async fn the_check_runs_on_the_real_bind_and_matches_the_model_it_was_measured_on() {
    let _ = shared_log();

    // **依頼する2つの綴りは、どちらも上流の一覧に在る。**
    let upstream = upstream_listing(&[MEASURED_MAIN, MEASURED_ARM_SUB]).await;
    let server = BoundServer::start(&[
        (ENV_LLM_ENDPOINT, &upstream.uri()),
        (ENV_MAIN_MODEL, MEASURED_MAIN),
        (ENV_SUB_MODEL, MEASURED_ARM_SUB),
    ])
    .await;

    let line = summary_for(MEASURED_MAIN).await;

    assert!(
        has_field(&line, &format!("checked_sub={MEASURED_ARM_SUB}")),
        "サブの綴りも同じ行に載ること: {line:?}"
    );
    assert!(
        has_field(&line, "measured_unanswered=0"),
        "一覧が取れているのに「聞けなかった」を数えないこと: {line:?}"
    );
    // **メインの面は一致、サブの面はずれ**（サブの綴りは測ったときの相手ではない）。
    assert!(
        has_field(&line, "measured_matched=4"),
        "メインの面（4つ）が一致であること: {line:?}"
    );
    assert!(
        has_field(&line, "measured_drifted=5"),
        "サブの面（5つ）がずれであること: {line:?}"
    );
    assert!(
        has_field(&line, "measured_checked=9"),
        "突き合わせた総数であること: {line:?}"
    );

    // **この腕のメインの綴りは、ずれの警告に出てはいけない。**
    let logged = shared_log().contents();
    for line in logged.lines().filter(|line| line.contains(DRIFT_MARKER)) {
        assert!(
            !has_field(line, &format!("upstream_model={MEASURED_MAIN}")),
            "測ったときの相手を「ずれ」と報告しないこと: {line:?}"
        );
    }

    server.stop().await;
}

/// **測ったときの相手と違うモデルなら、定数ごとにずれを報告する**（本番の入口）。
///
/// # 帰属を同じ行で見る
///
/// **定数の側（何で測ったか）と上流の側（いま何が答えるか）を取り違える変異**は、
/// 値がどちらも実在するので**存在の検査では捕まらない。**
/// **同じ行の中で、どちらの欄に何が載っているか**を見る。
#[tokio::test]
async fn a_model_that_was_never_measured_on_is_reported_per_constant() {
    let _ = shared_log();

    let upstream = upstream_listing(&[OTHER_MAIN, OTHER_SUB]).await;
    let server = BoundServer::start(&[
        (ENV_LLM_ENDPOINT, &upstream.uri()),
        (ENV_MAIN_MODEL, OTHER_MAIN),
        (ENV_SUB_MODEL, OTHER_SUB),
    ])
    .await;

    let summary = summary_for(OTHER_MAIN).await;
    assert!(
        has_field(&summary, "measured_matched=0"),
        "どの面も一致しないこと: {summary:?}"
    );
    assert!(
        has_field(&summary, "measured_drifted=9"),
        "全部の面がずれであること: {summary:?}"
    );

    // **メインの面の行**を名指しで拾う（サブの面の行と混ざる）。
    let main_upstream = format!("upstream_model={OTHER_MAIN}");
    let main_line = wait_for_fields(&[main_upstream.as_str()]).await;
    assert!(
        main_line.contains(DRIFT_MARKER),
        "ずれを告げる行であること: {main_line:?}"
    );
    assert!(
        has_field(&main_line, "measured_face=main"),
        "メインの面であること: {main_line:?}"
    );
    assert!(
        has_field(&main_line, "measured_on=ministral-3-14b-instruct-2512"),
        "定数の側は測ったときの相手であること: {main_line:?}"
    );
    assert!(
        !has_field(&main_line, &format!("measured_on={OTHER_MAIN}")),
        "定数の側に上流の綴りを載せないこと: {main_line:?}"
    );

    // **サブの面の行。** 依頼する綴りはサブのもの——ここがメインの綴りになっていたら、
    // 帰属を取り違えている（既定どうしが同じ綴りなら観測できない形）。
    let sub_upstream = format!("upstream_model={OTHER_SUB}");
    let sub_line = wait_for_fields(&[sub_upstream.as_str()]).await;
    assert!(
        has_field(&sub_line, "measured_face=sub"),
        "サブの面であること: {sub_line:?}"
    );
    assert!(
        has_field(
            &sub_line,
            "measured_constant=MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS"
        ) || has_field(
            &sub_line,
            "measured_constant=MEASURED_CONTROL_TOKEN_SPELLINGS"
        ) || has_field(&sub_line, "measured_constant=MESSAGE_OVERHEAD_TOKENS")
            || has_field(&sub_line, "measured_constant=GOLDEN")
            || has_field(&sub_line, "measured_constant=GOLDEN_BASELINE"),
        "一覧に在る定数を名乗ること: {sub_line:?}"
    );

    // **サブ専用の定数は、サブの綴りとだけ突き合わせる**（帰属の本命）。
    //
    // **既定どうしが同じ綴りだと、この取り違えは原理的に観測できない**
    // （`measured::requested_for` の doc）。**この腕は2つを違えてある**ので観測できる。
    //
    // 肯定側（サブの綴りで出ている）と否定側（メインの綴りでは出ていない）を**両方**見る。
    // 片方だけだと、「常にメインを見る」と「常にサブを見る」のどちらか一方しか捕まらない。
    let sub_only = SUB_ONLY_CONSTANT;
    let line = wait_for_fields(&[sub_only, sub_upstream.as_str()]).await;
    assert!(
        has_field(&line, "measured_face=sub"),
        "サブの面として報告すること: {line:?}"
    );

    // **一覧の全項目 × 全面**を、1行ずつ当てる（[`EXPECTED_ROWS`] / E-2 の是正）。
    //
    // **代表1つで止めない。** 以前はサブ専用の1件しか見ておらず、
    // **`measured_at` を `"src/server/extract.rs"` に固定する変異が 1685 全緑**を通った
    // （`measured_depends_on` を `"モデルの饒舌さ"` に固定する変異も同じ）。
    // **落ちていた先には `MEASURED_CONTROL_TOKEN_SPELLINGS` が含まれていた**——
    // ADR-0033 §決定3 が「最優先」と名指しした行である。
    for (name, location, depends_on, face, upstream) in EXPECTED_ROWS {
        let constant_field = format!("measured_constant={name}");
        let face_field = format!("measured_face={face}");
        let upstream_field = format!("upstream_model={upstream}");
        // **欄として完全一致で拾う。** `contains` だと
        // `measured_constant=GOLDEN` が `GOLDEN_BASELINE` の行にも当たる。
        let line = wait_for_fields(&[&constant_field, &face_field, &upstream_field]).await;

        assert!(
            line.contains(DRIFT_MARKER),
            "ずれを告げる行であること: {name} / {face} / {line:?}"
        );
        // **どこを直しに行けばよいか**が、その定数の行に載ること。
        assert!(
            has_field(&line, &format!("measured_at={location}")),
            "定数のある場所を、その定数の行に載せること: {name} / {face} / {line:?}"
        );
        // **何に依存しているか**（測り直す手順を選ぶ手掛かり / 04-environment.md）。
        assert!(
            has_field(&line, &format!("measured_depends_on={depends_on}")),
            "何に依存する値なのかを、その定数の行に載せること: {name} / {face} / {line:?}"
        );
        assert!(
            has_field(&line, "measured_on=ministral-3-14b-instruct-2512"),
            "定数の側は測ったときの相手であること: {name} / {face} / {line:?}"
        );
    }

    let logged = shared_log().contents();
    for line in logged.lines().filter(|line| line.contains(sub_only)) {
        assert!(
            !has_field(line, &format!("upstream_model={OTHER_MAIN}")),
            "サブ専用の定数をメインの綴りと突き合わせないこと: {line:?}"
        );
    }

    server.stop().await;
}

/// **依頼する綴りが上流の一覧に無ければ、その形で報告する**（本番の入口）。
///
/// LM Studio は**存在しないモデル名でもエラーにせず、載っているモデルが黙って答える**
/// ので、この形は「誰が答えるか分からない」状態そのもの。
#[tokio::test]
async fn a_spelling_the_upstream_does_not_have_is_reported_as_such() {
    let _ = shared_log();

    // 上流は**別のモデルだけ**を持っている。
    let upstream = upstream_listing(&["someone-else-entirely"]).await;
    let server = BoundServer::start(&[
        (ENV_LLM_ENDPOINT, &upstream.uri()),
        (ENV_MAIN_MODEL, UNLISTED_MAIN),
        (ENV_SUB_MODEL, UNLISTED_SUB),
    ])
    .await;

    let summary = summary_for(UNLISTED_MAIN).await;
    assert!(
        has_field(&summary, "measured_drifted=9"),
        "全部の面がずれであること: {summary:?}"
    );
    assert!(
        has_field(&summary, "measured_unanswered=0"),
        "一覧は取れているので「聞けなかった」ではないこと: {summary:?}"
    );

    let requested = format!("requested_model={UNLISTED_MAIN}");
    let line = wait_for_fields(&[requested.as_str()]).await;
    assert!(
        line.contains(ABSENT_MARKER),
        "一覧に無いことを告げる行であること: {line:?}"
    );
    assert!(
        !has_field(&line, "requested_model=someone-else-entirely"),
        "上流が持っている綴りを「依頼した綴り」として載せないこと: {line:?}"
    );

    // **1つの事実は1本**（作業単位 G-5）。**`llama-server` を上流にすると、
    // 9行が全部この形になる**（G-2 / G-6 の実測）。**判定は正確でも、
    // 9本は「1つの事実 × 9」であって「9つの事実」ではない。**
    //
    // **⚠ 数はべた書き**（このファイルの冒頭の doc と同じ理由——
    // 一覧から計算すると、一覧が緩んだときに期待値も一緒に動く）。
    // 内訳は「両方の面で見る定数 × 2 ＋ サブだけの定数 × 1」なので **4 / 5。**
    assert!(
        has_field(&line, "measured_face=main"),
        "どちらの面の話かを同じ行に載せること: {line:?}"
    );
    assert!(
        has_field(&line, "measured_constants=4"),
        "メインの面が代弁する定数の件数を同じ行に載せること: {line:?}"
    );
    // **畳んだ以上、代弁している範囲は行の上に無ければ意味がない。**
    // **⚠ 綴りもべた書き**（一覧から作ると、一覧が緩んだときに期待値も一緒に動く）。
    assert!(
        has_field(&line, ABSENT_MAIN_NAMES),
        "メインの面が代弁する定数の綴りを、並んだ順に同じ行へ載せること: {line:?}"
    );

    let requested_sub = format!("requested_model={UNLISTED_SUB}");
    let sub_line = wait_for_fields(&[requested_sub.as_str()]).await;
    assert!(
        sub_line.contains(ABSENT_MARKER),
        "サブの面も1本出ること: {sub_line:?}"
    );
    assert!(
        has_field(&sub_line, "measured_face=sub"),
        "サブの面として報告すること: {sub_line:?}"
    );
    assert!(
        has_field(&sub_line, "measured_constants=5"),
        "サブの面が代弁する定数の件数を同じ行に載せること: {sub_line:?}"
    );
    assert!(
        has_field(&sub_line, ABSENT_SUB_NAMES),
        "サブの面が代弁する定数の綴りを、並んだ順に同じ行へ載せること: {sub_line:?}"
    );

    // **面ごとに1本。** 定数ごとに1本へ戻ると、ここが 9 になる。
    //
    // **⚠ 共有のバッファには他のテストの行も混ざる**ので、
    // **この腕でしか使わない綴り**（`UNLISTED_MAIN` / `UNLISTED_SUB`）で数える。
    let logged = shared_log().contents();
    let absent = logged
        .lines()
        .filter(|line| line.contains(ABSENT_MARKER))
        .filter(|line| has_field(line, &requested) || has_field(line, &requested_sub))
        .count();
    assert_eq!(
        absent, 2,
        "面ごとに1本であること（定数ごとに1本ではない）: {logged}"
    );

    server.stop().await;
}

/// **上流がモデル一覧を持たないときは「聞けなかった」。ずれではない**（本番の入口）。
///
/// `/api/v0/models` は LM Studio 独自なので、**他の上流では必ずこうなる**
/// （ADR-0036 §決定4）。**ここで警告を出すと、毎回鳴って読まれなくなる。**
#[tokio::test]
async fn an_upstream_without_the_model_list_is_not_reported_as_drift() {
    let _ = shared_log();

    let upstream = upstream_without_the_native_api().await;
    let server = BoundServer::start(&[
        (ENV_LLM_ENDPOINT, &upstream.uri()),
        (ENV_MAIN_MODEL, SILENT_MAIN),
        (ENV_SUB_MODEL, SILENT_SUB),
    ])
    .await;

    let summary = summary_for(SILENT_MAIN).await;
    assert!(
        has_field(&summary, "measured_unanswered=9"),
        "全部の面が「聞けなかった」であること: {summary:?}"
    );
    assert!(
        has_field(&summary, "measured_drifted=0"),
        "聞けなかったをずれに数えないこと: {summary:?}"
    );
    assert!(
        has_field(&summary, "measured_matched=0"),
        "聞けなかったを一致に数えないこと: {summary:?}"
    );

    // **この腕の綴りは、どの警告にも出てはいけない。**
    let logged = shared_log().contents();
    for line in logged
        .lines()
        .filter(|line| line.contains(DRIFT_MARKER) || line.contains(ABSENT_MARKER))
    {
        assert!(
            !has_field(line, &format!("upstream_model={SILENT_MAIN}")),
            "聞けなかった回に警告を出さないこと: {line:?}"
        );
        assert!(
            !has_field(line, &format!("requested_model={SILENT_MAIN}")),
            "聞けなかった回に警告を出さないこと: {line:?}"
        );
        assert!(
            !has_field(line, &format!("requested_model={SILENT_SUB}")),
            "聞けなかった回に警告を出さないこと: {line:?}"
        );
    }

    server.stop().await;
}

/// **一覧そのものが空でも「聞けなかった」。ずれではない**（本番の入口）。
///
/// # なぜ 404 の腕とは別に要るのか
///
/// **通る経路が違う。** 404 は往復が失敗して**一覧が `None`** になるが、
/// こちらは**往復は成功していて一覧が空**（`Some(vec![])`）。
/// **空を「その綴りは上流に無い」と読むと、何も言っていない上流に対して警告が出る。**
///
/// **着手時の変異検査で、この腕が無いと単体テスト1本しか赤にならなかった**
/// ——本番の入口では、空の一覧という状態にどのフィクスチャも入っていなかった
/// （CLAUDE.md「フィクスチャが『その分岐が効く状態』に一度も入らない」）。
#[tokio::test]
async fn an_empty_model_list_is_not_reported_as_drift() {
    let _ = shared_log();

    // **一覧は返るが、中身が無い。**
    let upstream = upstream_listing(&[]).await;
    let server = BoundServer::start(&[
        (ENV_LLM_ENDPOINT, &upstream.uri()),
        (ENV_MAIN_MODEL, EMPTY_MAIN),
        (ENV_SUB_MODEL, EMPTY_SUB),
    ])
    .await;

    let summary = summary_for(EMPTY_MAIN).await;
    assert!(
        has_field(&summary, "measured_unanswered=9"),
        "全部の面が「聞けなかった」であること: {summary:?}"
    );
    assert!(
        has_field(&summary, "measured_drifted=0"),
        "空の一覧をずれに数えないこと: {summary:?}"
    );

    let logged = shared_log().contents();
    for line in logged
        .lines()
        .filter(|line| line.contains(DRIFT_MARKER) || line.contains(ABSENT_MARKER))
    {
        assert!(
            !has_field(line, &format!("requested_model={EMPTY_MAIN}")),
            "空の一覧に対して警告を出さないこと: {line:?}"
        );
        assert!(
            !has_field(line, &format!("requested_model={EMPTY_SUB}")),
            "空の一覧に対して警告を出さないこと: {line:?}"
        );
    }

    server.stop().await;
}

/// **上流が2つのとき、それぞれの面は自分の上流の一覧で判定される**
/// （作業単位 G-3a / [ADR-0039](../docs/adr/0039-two-upstreams-for-two-jobs.md) §決定3）。
///
/// # なぜこの腕が要るのか —— **嘘の警告を鳴らさないため**
///
/// **G-3a より前、モデルの一覧はサブ側の照会の本文からしか来ていなかった**
/// （`server::context::resolve_sub_context`）。**一覧はモデルに依らない**という前提は
/// **上流が1つのときだけ**正しい。
///
/// **⚠ 上流が2つになると崩れる。**
/// **本文のモデルの綴りを、抽出側の上流の一覧の中から探す**ことになるので、
/// **在るはずのものが `Drift::Absent`（「依頼する綴りが上流の一覧にありません」）として鳴る。**
///
/// **実測（G-3a の着手時 / 2026-09-07）:** この腕で
/// **`requested_model=arm-f-main-model` の `Absent` の警告が実際に出ていた**
/// （`measured_drifted=9` / `measured_unanswered=0`）。**本文の上流には在る綴りである。**
///
/// # ⚠ 「本文には一度も聞いていない」は偽だった（**G-3a のレビューが実測 / 是正済み**）
///
/// 是正前のこのテストは **`measured_unanswered=4`** を期待値にしていて、
/// 根拠として doc が「**本文の側には一度も一覧を聞いていない**」と書いていた。
/// **偽である**——`server::context::resolve_budget` は
/// `loaded_context_length()` → `models()` を通るので、**本文の上流にも現に照会していた**
/// （レビュアーがカウンタを植えて実測: `main /api/v0/models hits = 1`）。
/// **`ModelsReport` は一覧も持っていて、その層が捨てていただけ。**
///
/// **いまは捨てない。** 本文の面は**本文の上流の一覧**で判定する。
/// **往復は1回も増えていない**（同じ照会の本文から2つ読む / `ModelsReport::into_parts`）。
///
/// # 期待する内訳
///
/// | 面 | 一覧 | 判定 | 件数 |
/// |---|---|---|---|
/// | 本文 | **本文の上流のもの** | **ずれ**（綴りは在るが、測ったときの相手ではない） | 4 |
/// | 抽出 | **抽出の上流のもの** | **ずれ**（同上） | 5 |
///
/// **⚠ 数はべた書き**（このファイルの冒頭の doc と同じ理由）。
///
/// # ⚠ 「載っているか」ではなく「どれのものか」を見る（**帰属**）
///
/// 件数だけを見ると、**一覧を取り違える変異**（本文の面へ抽出の一覧を渡す）は
/// `Absent` を 9 本鳴らして `measured_drifted=9` を満たしうる。
/// **だから面ごとに、鳴っている行の中身まで見る:**
///
/// * `Absent` の行が**1本も無い**こと（**取り違えると必ず出る**）。
/// * `measured_face=main` の行が名乗る `upstream_model` は**本文の綴り**であること。
/// * `measured_face=sub` の行が名乗る `upstream_model` は**抽出の綴り**であること。
#[tokio::test]
async fn two_upstreams_are_each_judged_against_their_own_catalogue() {
    let _ = shared_log();

    // **本文の上流は本文の綴りだけ、抽出の上流は抽出の綴りだけ**を持っている。
    // **同じ一覧を持たせないこと**——同値だと、どちらから読んでも同じ答えになる。
    let main_upstream = upstream_listing(&[SPLIT_MAIN]).await;
    let sub_upstream = upstream_listing(&[SPLIT_SUB]).await;
    assert_ne!(
        main_upstream.uri(),
        sub_upstream.uri(),
        "前提: 2つの上流は別の宛先であること"
    );
    assert_ne!(SPLIT_MAIN, SPLIT_SUB, "前提: 2つの綴りは別であること");

    let server = BoundServer::start(&[
        (ENV_LLM_ENDPOINT, &main_upstream.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub_upstream.uri()),
        (ENV_MAIN_MODEL, SPLIT_MAIN),
        (ENV_SUB_MODEL, SPLIT_SUB),
    ])
    .await;

    let summary = summary_for(SPLIT_MAIN).await;
    assert!(
        has_field(&summary, &format!("checked_sub={SPLIT_SUB}")),
        "サブの綴りも同じ行に載ること: {summary:?}"
    );
    assert!(
        has_field(&summary, "measured_unanswered=0"),
        "どちらの面も自分の上流へ照会できている\
         ——本文の面の一覧を捨てないこと: {summary:?}"
    );
    assert!(
        has_field(&summary, "measured_drifted=9"),
        "9つとも「ずれ」であること（どちらの綴りも測ったときの相手ではない）: {summary:?}"
    );
    assert!(
        has_field(&summary, "measured_matched=0"),
        "どちらの綴りも測ったときの相手ではないこと: {summary:?}"
    );
    assert!(
        has_field(&summary, "measured_checked=9"),
        "突き合わせた総数であること: {summary:?}"
    );

    // **`Absent` は1本も出てはいけない。**
    // **これが直す前に実際に出ていた行である**（`requested_model=arm-f-main-model`）。
    // **一覧を取り違えると必ずここが鳴る**ので、件数の主張より強い。
    let logged = shared_log().contents();
    for line in logged.lines().filter(|line| line.contains(ABSENT_MARKER)) {
        for spelling in [SPLIT_MAIN, SPLIT_SUB] {
            assert!(
                !has_field(line, &format!("requested_model={spelling}")),
                "自分の上流に在る綴りを「一覧に無い」と報告しないこと\
                 （もう一方の上流の一覧で探している）: {line:?}"
            );
        }
    }

    // **面ごとに、鳴っている行が名乗る綴りまで見る**（帰属）。
    // **`upstream_model` は「その面が依頼する綴り」**なので、
    // **面と綴りの組が入れ替わった形**はここで赤になる。
    for (face, own, other) in [
        ("main", SPLIT_MAIN, SPLIT_SUB),
        ("sub", SPLIT_SUB, SPLIT_MAIN),
    ] {
        let own_field = format!("upstream_model={own}");
        let face_field = format!("measured_face={face}");
        let line = wait_for_fields(&[own_field.as_str(), face_field.as_str()]).await;
        assert!(
            line.contains(DRIFT_MARKER),
            "面 {face} はずれとして鳴ること: {line:?}"
        );
        assert!(
            !has_field(&line, &format!("upstream_model={other}")),
            "面 {face} にもう一方の綴りを載せないこと: {line:?}"
        );
    }

    server.stop().await;
}

/// **同じ上流を2つの欄で名乗った起動は、上流が1つのときと同じ扱いになる**
/// （作業単位 G-3a の是正 / [ADR-0039](../docs/adr/0039-two-upstreams-for-two-jobs.md) §決定1）。
///
/// # 何を見ているのか —— **`one_upstream` を「何から作るか」**
///
/// **`App::bind` は「上流が1つか」をクライアントの宛先の綴りで判定する**
/// （設定の有無ではない）。**この2つは同じではない**——
/// **`PERSONACORE_SUB_LLM_ENDPOINT` に本文と同じ URL を書いた起動**は、
/// **設定としては `Some`** だが、**宛先としては1つ**である。
///
/// **是正前は、この判定を `config.sub_llm().is_none()` に差し替えても
/// 1966 本すべてが緑だった**（G-3a のレビューが実測 / 2026-09-07）。
/// **値そのものは効いていたが、「何から作るか」を見る者が1人も居なかった。**
///
/// # ⚠ この判定が効くのは、本文の面が一覧を持たないときだけ
///
/// **`App::bind` が見ているのは「本文の面が一覧を持っていない」ことだけ**で、
/// そうなる道は2つある——**明示設定で照会そのものを行わなかった**ときと、
/// **往復そのものが失敗した**とき（表は `server::context::resolve_budget` の doc）。
///
/// **この2つの腕は、前者で作る。** `PERSONACORE_CONTEXT_TOKENS` を明示すると
/// `resolve_budget` は早期 return するので、**決定的に、その状態を作れる**からである
/// （結果を捨てる往復を増やさないための早期 return / ADR-0013）。
/// **後者（往復の失敗）でも同じ分岐を通るが、ここでは作っていない。**
///
/// **⚠ そして「この2つを区別しない」ことは、どこからも観測されていない**——
/// **実測（`m17`: 下の腕を `None if one_upstream && explicit.is_some() =>` に絞る）は
/// 1969 全緑・exit 0**（G-3a の是正2周目 / 2026-09-07）。
/// **区別しないのは決めたことであって、測っているものではない。**
///
/// **一覧を持つ腕（＝明示設定が無い起動）では、この判定は結果に何も影響しない。**
///
/// # 2つの腕を1本に置く理由
///
/// | 腕 | 2つの欄 | 宛先 | 本文の面 |
/// |---|---|---|---|
/// | 同じ URL を2度 | どちらも同じモックサーバ | **1つ** | **判定される**（`measured_unanswered=0`） |
/// | 別々の URL | 別のモックサーバ | **2つ** | **聞けなかった**（`measured_unanswered=4`） |
///
/// **片方だけでは足りない**（CLAUDE.md「対比は安い側だけでは足りない」）——
/// `true` の決め打ちは下の腕で、`false` の決め打ちは上の腕で赤になる。
///
/// # ⚠ 「同じ URL」は**正規化後にバイト一致する綴り**の意味
///
/// 比べているのは `LmStudioConfig::endpoint()`——`normalize_base` が
/// **末尾の `/` を落とすだけ**なので、**`http://LOCALHOST:1234` と
/// `http://localhost:1234` は別扱いになり、本文の面は黙る。**
/// **ホスト名を畳んだり名前解決したりはしていない。**
#[tokio::test]
async fn one_upstream_named_twice_still_judges_the_body_face() {
    let _ = shared_log();

    // ---- 腕1: 同じモックサーバの URI を2つの欄へ与える。**宛先は1つ。** ----
    let shared = upstream_listing(&[SAME_MAIN, SAME_SUB]).await;
    assert_ne!(SAME_MAIN, SAME_SUB, "前提: 2つの綴りは別であること");
    let shared_uri = shared.uri();
    let server = BoundServer::start(&[
        (ENV_LLM_ENDPOINT, &shared_uri),
        // **同じ綴りを2度**。設定としては `Some` だが、宛先としては1つ。
        (ENV_SUB_LLM_ENDPOINT, &shared_uri),
        (ENV_MAIN_MODEL, SAME_MAIN),
        (ENV_SUB_MODEL, SAME_SUB),
        // **本文の側の照会を止める**（この判定が効く唯一の状態を作る）。
        (ENV_CONTEXT_TOKENS, EXPLICIT_CONTEXT_TOKENS),
    ])
    .await;

    let summary = summary_for(SAME_MAIN).await;
    assert!(
        has_field(&summary, "measured_unanswered=0"),
        "宛先が1つなら、本文の面もその一覧で判定すること: {summary:?}"
    );
    assert!(
        has_field(&summary, "measured_drifted=9"),
        "9つとも「ずれ」であること（どちらの綴りも測ったときの相手ではない）: {summary:?}"
    );
    // **本文の面が現に鳴っていること**（数だけでなく、行の上でも見る）。
    let main_field = format!("upstream_model={SAME_MAIN}");
    let main_line = wait_for_fields(&[main_field.as_str(), "measured_face=main"]).await;
    assert!(
        main_line.contains(DRIFT_MARKER),
        "本文の面がずれとして鳴ること: {main_line:?}"
    );
    server.stop().await;

    // ---- 腕2: 別々の URL。**宛先は2つ**なので、本文の面は「聞けなかった」。 ----
    let main_upstream = upstream_listing(&[APART_MAIN]).await;
    let sub_upstream = upstream_listing(&[APART_SUB]).await;
    assert_ne!(
        main_upstream.uri(),
        sub_upstream.uri(),
        "前提: 2つの上流は別の宛先であること"
    );
    let server = BoundServer::start(&[
        (ENV_LLM_ENDPOINT, &main_upstream.uri()),
        (ENV_SUB_LLM_ENDPOINT, &sub_upstream.uri()),
        (ENV_MAIN_MODEL, APART_MAIN),
        (ENV_SUB_MODEL, APART_SUB),
        (ENV_CONTEXT_TOKENS, EXPLICIT_CONTEXT_TOKENS),
    ])
    .await;

    let summary = summary_for(APART_MAIN).await;
    assert!(
        has_field(&summary, "measured_unanswered=4"),
        "宛先が2つで本文へ照会していないなら、本文の面は「聞けなかった」であること\
         ——抽出側の一覧を流用しないこと: {summary:?}"
    );
    assert!(
        has_field(&summary, "measured_drifted=5"),
        "抽出の面（5つ）は自分の上流の一覧で判定すること: {summary:?}"
    );

    // **本文の綴りを「一覧に無い」と言わないこと**（流用した形の署名）。
    let logged = shared_log().contents();
    for line in logged.lines().filter(|line| line.contains(ABSENT_MARKER)) {
        assert!(
            !has_field(line, &format!("requested_model={APART_MAIN}")),
            "聞いていない上流の綴りを「一覧に無い」と報告しないこと: {line:?}"
        );
    }

    server.stop().await;
}
