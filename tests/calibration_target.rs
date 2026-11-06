//! **校正の器具が「どのモデルを測るか」**を決める部分の見張り番（作業単位 E-3）。
//!
//! # なぜ専用のテストバイナリが要るのか
//!
//! 校正の器具（`tests/prompt_estimate_calibration.rs`）は**4本とも `#[ignore]`** で、
//! **LM Studio の起動が要る**（`~/.claude/CLAUDE.md` / docs/04-environment.md）。
//! つまり**「本当に別のモデルを測れるか」は、素の `cargo test` では確かめられない。**
//!
//! **だから、走らせずに確かめられる部分だけを `tests/common/calibration.rs` へ切り出し、
//! ここがそれを見張る。** 見張れるのは次の4つ:
//!
//! | 見るもの | 何を止めるか |
//! |---|---|
//! | 未設定なら `DEFAULT_MAIN_MODEL` | 既定が今までどおりであること |
//! | `PERSONACORE_CALIBRATION_MODEL` だけが対象を動かす | **常に既定を返す**（＝E-3 以前の欠陥）|
//! | 本番の振り分け（`PERSONACORE_SUB_MODEL` / `PERSONACORE_MAIN_MODEL`）は対象を動かさない | **勝手に切り替わる**（逆向きの同じ事故）|
//! | 読めない綴りは**止まる** | 黙って既定へ落ちる |
//!
//! **⚠ ここが緑でも「サブモデルを測れた」とは言えない。**
//! **言えるのは「対象を指せる」までで、実機で測った者はまだ居ない**
//! （2026-08-28 時点。LM Studio は落ちている）。
//!
//! # 環境変数は1つも触らない
//!
//! 判定はすべて**参照関数を受け取る純粋関数**（`calibration_target`）に対して行う。
//! `src/server/config.rs` の `from_lookup` と同じ作法で、
//! **`std::env::set_var` は並行するテストを壊す**ため使わない。
//!
//! **⚠ その代わり、ここの 11 本はどれも「環境を読む1行」を通らない。**
//! **そこは `tests/calibration_env_wiring.rs`（テスト1本だけの専用バイナリ）が見張る**
//! ——分けた理由はあちらの doc（環境を書き換えるので、同じプロセスに他のテストを置けない）。

mod common;

use common::calibration::{
    AssembledShape, COMPARED_SHAPE_CATEGORY, CONTROL_LISTS, ControlListKind, ControlProbe,
    ENV_CALIBRATION_MODEL, LookalikeProbe, LookalikeSide, ShapeProbe, ShapeVerdict,
    assembled_shape, calibration_body, calibration_target, compared_shape_spelling, golden_block,
    lookalike_side, measured_on_note, overhead_block, probe_control, probe_shape, shape_block,
};
// **行を取り出す道具は共有の1本を使う**（`tests/common/mod.rs`）。
// **別名で写さないこと** —— CLAUDE.md が「`line_with` のような名前の写しは走査に捕まらない」と
// 名指ししている形になる。
use common::line_containing;
use personacore::llm::{DEFAULT_MAIN_MODEL, DEFAULT_SUB_MODEL, ModelId, ModelIdError};
use personacore::prompt::{
    MEASURED_CONTROL_TOKEN_SPELLINGS, MEASURED_MODEL_DEPENDENT_SPELLINGS,
    MEASURED_ORDINARY_LOOKALIKES,
};
use personacore::server::{ENV_MAIN_MODEL, ENV_SUB_MODEL};
use serde_json::json;

/// 校正の対象として与える綴り。**既定のどちらとも違うこと**がこのファイルの存在理由
/// （CLAUDE.md「実行時に決まる値を入れたら、**既定と違う状態で回る腕**を必ず1本置くこと」）。
const TARGET_MODEL: &str = "ministral-3-3b-instruct-2512";

/// **本番の振り分け（サブ）**に入っている綴り。**対象を動かしてはならない側。**
const SUB_MODEL_SPELLING: &str = "ministral-3-3b-reasoning-2512";

/// **本番の振り分け（メイン）**に入っている綴り。`/` を含む識別子は正当（`ModelId` の doc）。
const MAIN_MODEL_SPELLING: &str = "mistralai/devstral-small-2-2512";

/// 貼れる形の検査に使うベースラインの**フィクスチャ**。
///
/// **⚠ 本物の `GOLDEN_BASELINE`（536）と違う値であること。**
/// 同値にすると、**引数を無視して 536 を書く決め打ちが素通りする**
/// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
/// **表の行（4 / 20）は2件あるので同じ穴は無い。**
const FIXTURE_BASELINE: u32 = 537;

/// 表を測ったモデルの**フィクスチャ**。
///
/// **実物（`prompt_estimate_calibration.rs` の `MEASURED_ON_MODEL`）を持ち込まない。**
/// 持ち込むと、判定が「対象と既定の関係」に寄りかかる。
/// ここが見るのは「違えば言う / 同じなら黙る」だけ。
const MEASURED_ON_FIXTURE: &str = "ministral-3-14b-reasoning-2512";

/// 与えた組だけを返す参照関数（`src/server/config.rs` の作法）。
fn lookup_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let pairs: Vec<(String, String)> = pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    move |key| {
        pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    }
}

fn model(spelling: &str) -> ModelId {
    ModelId::parse(spelling).expect("フィクスチャの綴りは正当であること")
}

/// フィクスチャが既定と同値でないこと。**同値だと決め打ちの変異を検出できない。**
fn assert_fixtures_differ() {
    for spelling in [TARGET_MODEL, SUB_MODEL_SPELLING, MAIN_MODEL_SPELLING] {
        assert_ne!(
            spelling, DEFAULT_MAIN_MODEL,
            "フィクスチャは既定のメインと違う綴りであること"
        );
        assert_ne!(
            spelling, DEFAULT_SUB_MODEL,
            "フィクスチャは既定のサブと違う綴りであること"
        );
    }
    assert_ne!(
        TARGET_MODEL, SUB_MODEL_SPELLING,
        "対象と囮は違う綴りであること"
    );
    assert_ne!(
        TARGET_MODEL, MAIN_MODEL_SPELLING,
        "対象と囮は違う綴りであること"
    );
}

/// **何も指定しなければ、今までどおり `DEFAULT_MAIN_MODEL` を測る。**
#[test]
fn nothing_specified_still_measures_the_default_main_model() {
    let target = calibration_target(lookup_of(&[])).expect("既定だけで決まること");

    assert_eq!(
        target.as_str(),
        DEFAULT_MAIN_MODEL,
        "未設定のときの対象が既定のメインから動いている"
    );
}

/// **`PERSONACORE_CALIBRATION_MODEL` が対象を動かす。**
///
/// **これが E-3 の本体。** 変異「対象の選択を無視して常に既定を返す」はここで赤くなる。
#[test]
fn the_calibration_variable_selects_the_model_to_measure() {
    assert_fixtures_differ();

    let target = calibration_target(lookup_of(&[(ENV_CALIBRATION_MODEL, TARGET_MODEL)]))
        .expect("正当な綴りであること");

    assert_eq!(
        target.as_str(),
        TARGET_MODEL,
        "指した綴りが対象になっていない"
    );
}

/// **前後の空白は落とす**（`ModelId::parse` を通していることの現れ）。
#[test]
fn the_calibration_variable_is_trimmed_by_the_model_id_parser() {
    let target = calibration_target(lookup_of(&[(
        ENV_CALIBRATION_MODEL,
        "  ministral-3-3b-instruct-2512\t",
    )]))
    .expect("空白を除けば正当であること");

    assert_eq!(target.as_str(), TARGET_MODEL, "前後の空白が残っている");
}

/// **本番の振り分けは、校正の対象を動かさない。**
///
/// # なぜこの囮の形なのか
///
/// `PERSONACORE_SUB_MODEL` は**本番でサブモデル（抽出・変容）を差し替えるつまみ**であって、
/// **校正の対象ではない。** 校正が黙って別のモデルを測り始めるのは、
/// E-3 以前の欠陥（常にメインを測る）と**向きが逆なだけの同じ事故**である。
///
/// **2段に分けてある**——「囮だけ」と「囮 ＋ 本命」で、
/// **読んでしまう変異と、囮を優先する変異の両方**を赤にするため。
#[test]
fn the_production_model_variables_do_not_move_the_calibration_target() {
    assert_fixtures_differ();

    // 1. 囮だけ。**対象は既定のまま。**
    let decoys_only = calibration_target(lookup_of(&[
        (ENV_SUB_MODEL, SUB_MODEL_SPELLING),
        (ENV_MAIN_MODEL, MAIN_MODEL_SPELLING),
    ]))
    .expect("既定だけで決まること");

    assert_eq!(
        decoys_only.as_str(),
        DEFAULT_MAIN_MODEL,
        "本番の振り分けが校正の対象を動かした（黙って別のモデルを測っている）"
    );

    // 2. 囮 ＋ 本命。**本命が勝つ。**
    let with_target = calibration_target(lookup_of(&[
        (ENV_SUB_MODEL, SUB_MODEL_SPELLING),
        (ENV_MAIN_MODEL, MAIN_MODEL_SPELLING),
        (ENV_CALIBRATION_MODEL, TARGET_MODEL),
    ]))
    .expect("正当な綴りであること");

    assert_eq!(
        with_target.as_str(),
        TARGET_MODEL,
        "校正の変数より本番の振り分けが優先されている"
    );
}

/// **読むのは `PERSONACORE_CALIBRATION_MODEL` ただ1つ。**
///
/// 上の囮は「思いついた2つ」しか置けない。ここは**それ以外の綴りを1つでも読んだら落ちる**
/// 参照関数を渡して、**こちらが思いつかなかった変数も**まとめて止める。
#[test]
fn only_the_calibration_variable_is_ever_read() {
    let target = calibration_target(|key| {
        assert_eq!(
            key, ENV_CALIBRATION_MODEL,
            "校正の対象を決めるのに別の環境変数を読んでいる: {key}"
        );
        None
    })
    .expect("既定だけで決まること");

    assert_eq!(target.as_str(), DEFAULT_MAIN_MODEL);
}

/// **モデルを差し替える人が読む doc の実物。** 写しではない。
///
/// `include_str!` はビルドの入力なので、**この doc を書き換えればこのテストバイナリは
/// 必ず再コンパイルされる**（先例は `tests/server_admin.rs` の `SAMPLE_SEED_JSON`）。
const ENVIRONMENT_DOC: &str = include_str!("../docs/04-environment.md");

/// 校正の器具の実物（手順の doc がこの中にある）。
const CALIBRATION_INSTRUMENT: &str = include_str!("prompt_estimate_calibration.rs");

/// **doc に書かれた変数名が、実装の綴りと1文字も違わないこと。**
///
/// # なぜ要るのか
///
/// **ずれたときの失敗が、E-3 が直した欠陥そのものになる** ——
/// **doc どおりに環境変数を設定した人が、指したつもりで `DEFAULT_MAIN_MODEL` を測る。**
/// **綴り間違いは `Err` で止まるが、「別の名前の変数」は設定しても誰も読まないので止まらない。**
///
/// **実測**（E-3 のレビュー / 2026-08-28）——
/// **この検査が無い状態で `ENV_CALIBRATION_MODEL` の綴りを変える変異は、1642 件すべて緑を通った。**
/// 値の写しが3箇所（この doc 2箇所と手順1）あり、**どれも定数を参照していない**ため。
///
/// **⚠ `PERSONACORE_MAIN_MODEL` / `PERSONACORE_SUB_MODEL` も同じく無防備**だが、
/// **E-3 が作った欠陥ではないので触っていない。**
#[test]
fn every_written_copy_of_the_variable_name_matches_the_constant() {
    // **走査対象が空・短すぎるなら、この検査は何も守っていない**（`src/testlog.rs` の床と同じ発想）。
    for (label, source, floor) in [
        ("docs/04-environment.md", ENVIRONMENT_DOC, 10_000),
        (
            "tests/prompt_estimate_calibration.rs",
            CALIBRATION_INSTRUMENT,
            5_000,
        ),
    ] {
        assert!(
            source.len() > floor,
            "{label} を読めていない（{} バイト）",
            source.len()
        );
        assert!(
            source.contains(ENV_CALIBRATION_MODEL),
            "{label} が {ENV_CALIBRATION_MODEL} を1度も書いていない\
             （doc の綴りと実装がずれると、設定しても誰も読まない）"
        );
    }
}

/// **読めない綴りは、黙って既定へ落ちずに止まる。**
///
/// 落ちなければ「別のモデルを指したつもりで、既定を測り続ける」——
/// **E-3 が直した欠陥と同じ形が、綴り間違いのたびに再現する。**
#[test]
fn an_unreadable_spelling_stops_instead_of_falling_back() {
    assert_eq!(
        calibration_target(lookup_of(&[(ENV_CALIBRATION_MODEL, "mo\ndel")])),
        Err(ModelIdError::Control),
        "制御文字を含む綴りが通った（または既定へ落ちた）"
    );
    assert_eq!(
        calibration_target(lookup_of(&[(ENV_CALIBRATION_MODEL, "   ")])),
        Err(ModelIdError::Empty),
        "空白だけの綴りが通った（または既定へ落ちた）"
    );
    assert_eq!(
        calibration_target(lookup_of(&[(ENV_CALIBRATION_MODEL, "")])),
        Err(ModelIdError::Empty),
        "空の綴りが通った（または既定へ落ちた）"
    );
}

/// **上流へ送る本文の `model` が、選んだ対象そのものであること。**
///
/// 対象を決める関数が正しくても、**依頼の組み立てが既定を書いていれば何も変わらない**
/// （それが E-3 以前の `prompt_tokens_of` だった）。
#[test]
fn the_request_body_names_the_selected_target() {
    assert_fixtures_differ();

    let body = calibration_body(
        &model(TARGET_MODEL),
        json!([{"role": "user", "content": "a"}]),
    );

    assert_eq!(
        body["model"], TARGET_MODEL,
        "依頼の model が対象と違う（決め打ちに戻っている）"
    );
    assert_eq!(
        body["messages"],
        json!([{"role": "user", "content": "a"}]),
        "渡したメッセージ列がそのまま載ること"
    );
    assert_eq!(body["stream"], json!(false), "校正は逐次で受け取らない");
    assert_eq!(
        body["max_tokens"],
        json!(1),
        "本文は要らない（測るのは usage.prompt_tokens だけ）"
    );
}

/// **表を測ったモデルと、いま測る相手が違うなら言う**（ADR-0033 §決定1）。
#[test]
fn the_instrument_says_when_the_table_was_measured_on_another_model() {
    assert_fixtures_differ();
    assert_ne!(
        MEASURED_ON_FIXTURE, TARGET_MODEL,
        "前提: 表のモデルと対象がずれた状態で見ていること"
    );

    let note = measured_on_note(MEASURED_ON_FIXTURE, &model(TARGET_MODEL))
        .expect("違うなら注意書きが出ること");

    assert!(
        note.contains(MEASURED_ON_FIXTURE),
        "表を測ったモデルを名乗っていない: {note}"
    );
    assert!(
        note.contains(TARGET_MODEL),
        "いま測っている相手を名乗っていない: {note}"
    );

    assert_eq!(
        measured_on_note(MEASURED_ON_FIXTURE, &model(MEASURED_ON_FIXTURE)),
        None,
        "同じモデルなら黙ること（毎回鳴ると読まれなくなる）"
    );
}

/// **`GOLDEN` の実測値が、そのまま貼れる形で出る**（ADR-0033 §決定2）。
#[test]
fn the_golden_block_is_pasteable_and_names_the_model_it_measured() {
    assert_fixtures_differ();

    let block = golden_block(
        &model(TARGET_MODEL),
        MEASURED_ON_FIXTURE,
        FIXTURE_BASELINE,
        &[("猫が好き", 4), ("a b c", 20)],
    );

    assert!(
        block.contains("const GOLDEN_BASELINE: Option<u32> = Some(537);"),
        "ベースラインが貼れる形で出ていない: {block}"
    );
    assert!(
        block.contains(r#"    ("猫が好き", Some(4)),"#),
        "表の行が貼れる形で出ていない: {block}"
    );
    assert!(
        block.contains(r#"    ("a b c", Some(20)),"#),
        "2件目の行が貼れる形で出ていない: {block}"
    );

    // **同じ行の中で見る**（CLAUDE.md「その行が出た」と「その行がこう言っている」は別の主張）。
    let header = block
        .lines()
        .find(|line| line.contains("ここから"))
        .expect("貼り始めの目印があること");
    assert!(
        header.contains(TARGET_MODEL),
        "貼り始めの行が、測った相手を名乗っていない: {header}"
    );

    // 表のモデルと違うので、注意書きが**貼る範囲の外**に出ていること。
    let note_at = block
        .find(MEASURED_ON_FIXTURE)
        .expect("表を測ったモデルを名乗ること");
    let header_at = block.find("ここから").expect("貼り始めの目印があること");
    assert!(
        note_at < header_at,
        "注意書きが貼る範囲の中にある（貼るとコンパイルできない）: {block}"
    );
}

/// **同じモデルを測っているなら、貼る範囲の前に余計な行を出さない。**
#[test]
fn the_golden_block_stays_quiet_when_the_model_matches() {
    let block = golden_block(
        &model(MEASURED_ON_FIXTURE),
        MEASURED_ON_FIXTURE,
        FIXTURE_BASELINE,
        &[("猫が好き", 4)],
    );

    let first = block.lines().next().expect("空でないこと");
    assert!(
        first.contains("ここから"),
        "同じモデルなのに前置きが出ている: {first}"
    );
}

/// **`MESSAGE_OVERHEAD_TOKENS` も貼れる形で出る。**
///
/// **⚠ 下げる向きには貼らせない。** 実測が定数を下回っているときに実測値を貼ると、
/// **余裕が消える**（この定数は上界であって、実測そのものではない）。
///
/// # ⚠ 数は必ず「行ごと・語つき」で見る
///
/// **1文字の `contains` は空振りする** —— レビューで実測（E-3 / 2026-08-28）:
/// **`contains('5')` は注意書きの行にある `2512` を拾う**ので、
/// **実測値が出力から丸ごと消えても緑だった。**
/// **`overhead_block` の doc が「実測そのものは別の行に出すので判断の材料は失われない」と
/// 明記しているのに、その主張を見ている見張り番が無かった。**
#[test]
fn the_overhead_block_never_suggests_lowering_the_constant() {
    let target = model(TARGET_MODEL);

    // 1. 実測が定数を上回る → **実測値**を貼る。
    let raised = overhead_block(&target, MEASURED_ON_FIXTURE, 10, 34, 12, 8);
    assert!(
        raised.contains("pub const MESSAGE_OVERHEAD_TOKENS: u32 = 12;"),
        "実測が定数を上回ったのに、貼る値が実測値でない: {raised}"
    );

    // 2. 実測が定数を下回る → **いまの定数**のまま。
    let kept = overhead_block(&target, MEASURED_ON_FIXTURE, 10, 22, 5, 8);
    assert!(
        kept.contains("pub const MESSAGE_OVERHEAD_TOKENS: u32 = 8;"),
        "実測が定数を下回ったのに、貼る値が下がっている: {kept}"
    );
    assert!(
        !kept.contains("= 5;"),
        "下げる向きの値を貼れる形で出している: {kept}"
    );

    // **実測そのものが、貼る値と別に読めること。**
    // **同じ行の中で、語と一緒に見る**（CLAUDE.md「その行が出た」と「その行がこう言っている」は別）。
    let kept_increment = line_containing(&kept, "増分");
    assert!(
        kept_increment.contains("増分 = 5 トークン"),
        "実測の増分が出力から消えている（判断の材料が無い）: {kept_increment}"
    );
    assert!(
        kept_increment.contains("MESSAGE_OVERHEAD_TOKENS = 8"),
        "いまの定数が読めない（比べられない）: {kept_increment}"
    );

    let raised_increment = line_containing(&raised, "増分");
    assert!(
        raised_increment.contains("増分 = 12 トークン"),
        "実測の増分が、いまの定数と取り違えられている: {raised_increment}"
    );

    // **件数の帰属。** 1件と3件を入れ替えた報告は、器具の報告そのものが嘘になる。
    let measured_line = line_containing(&raised, "メッセージ1件 =");
    assert!(
        measured_line.contains("メッセージ1件 = 10 トークン / 3件 = 34 トークン"),
        "1件と3件の実測が入れ替わっている（または消えている）: {measured_line}"
    );
    assert!(
        measured_line.contains(TARGET_MODEL),
        "実測の行が、測った相手を名乗っていない: {measured_line}"
    );

    // **「足りている」は、足りているときだけ出す。**
    assert!(
        kept.lines().any(|line| line.contains("足りている")),
        "定数で足りているのに、そう言っていない: {kept}"
    );
    assert!(
        !raised.lines().any(|line| line.contains("足りている")),
        "足りていないのに「足りている」と出している: {raised}"
    );
}

// ------------------------------------------------------------------
// **崩した形の比較**（作業単位 B-25）。
//
// 器具の本体（`a_category_named_after_a_control_token_is_not_special_in_the_prompt`）は
// `#[ignore]` なので、**中に何を書いても素の `cargo test` は1行も通らない。**
// 行の組み立ても、測った値の配線も、判定も、報告の塊も
// `tests/common/calibration.rs` の純粋関数に置き、ここが見張る。
// ------------------------------------------------------------------

/// 比較に使う綴りの**フィクスチャ**。
///
/// **実物（器具が使う制御トークンの綴り）を持ち込まない**——持ち込むと、
/// 報告の中で綴りを決め打つ変異が「たまたま同じ文字列」で素通りする
/// （`MEASURED_ON_FIXTURE` と同じ作法）。
const SHAPE_FIXTURE_SPELLING: &str = "[FIXTURE_SHAPE]";

/// 崩した形を含む行の**フィクスチャ**（`SafeText::sanitize` が入れる空白の位置に注意）。
const SHAPE_FIXTURE_LINE: &str = "> - [FIXTURE_SHAPE ] ねこ";

/// 判定の材料を組み立てる。**綴りは常にフィクスチャ。**
fn shape_probe(raw: u32, guarded: u32, unguarded: u32) -> ShapeProbe<'static> {
    ShapeProbe {
        spelling: SHAPE_FIXTURE_SPELLING,
        raw,
        guarded,
        unguarded,
    }
}

/// 組み上がった行のフィクスチャから [`AssembledShape`] を作る。
fn shape_fixture() -> AssembledShape {
    assembled_shape(SHAPE_FIXTURE_LINE, SHAPE_FIXTURE_SPELLING)
        .expect("フィクスチャは崩れた行であること")
}

/// **崩した形の比較は、その綴りがこの上流で特殊なときにしか判定しない**（作業単位 B-25）。
///
/// # 何が起きたのか
///
/// 器具は「防御あり（崩した形）の方がトークン数が多い」ことで無害化を確かめていた。
/// **これは綴りが特殊トークンであるモデルでしか観測できない。**
/// **`qwen/qwen3.5-9b` では `[INST]` がただの文字列**（2〜7 トークン）なので、
/// 崩しても数はほとんど動かず、**`8` 対 `8` で落ちた**（C-14 が 2026-08-29 に実測）。
/// **落ちたときの指示は「規則と一覧を見直すこと」だが、規則は正しく崩し続けている。**
/// **偽の警報は、鳴らないことより悪い**（`MEASURED_ORDINARY_LOOKALIKES` の doc / B-22）。
///
/// # 数はここに書く
///
/// 期待値を定数から作らない（写しになる / CLAUDE.md）。
/// **1 は「特殊トークンは定義上1トークン」そのもの**なので、literal で書いてある。
#[test]
fn the_shape_comparison_is_judged_only_where_the_spelling_is_special() {
    // 1. この上流で特殊（1 トークン）で、崩した側の方が高い → **崩せている。**
    //    11 / 9 は `ministral-3-14b-instruct-2512` の実測（器具の doc）。
    assert_eq!(shape_probe(1, 11, 9).verdict(), ShapeVerdict::Broken);
    assert_eq!(
        shape_probe(1, 11, 9).failure(),
        None,
        "崩せているのに失敗として扱っている"
    );

    // 2. 特殊なのに崩した側が高くならない → **本物の失敗**（同数と、低い側の2通り）。
    assert_eq!(shape_probe(1, 9, 9).verdict(), ShapeVerdict::StillSpecial);
    assert_eq!(shape_probe(1, 8, 9).verdict(), ShapeVerdict::StillSpecial);
    assert!(
        shape_probe(1, 9, 9).failure().is_some(),
        "特殊トークンのままなのに失敗として扱っていない"
    );

    // 3. **この上流では特殊でない** → 大小に関わらず判定しない。
    //    `(8, 8, 8)` は C-14 が `qwen/qwen3.5-9b` で実際に踏んだ値。
    //    **崩した側が低い組（8, 7, 9）まで入れてある**——ここを「低ければ失敗」に
    //    緩める変異は、それが無いと素通りする。
    for (raw, guarded, unguarded) in [(8, 8, 8), (8, 11, 9), (8, 7, 9), (0, 5, 4), (2, 9, 9)] {
        let probe = shape_probe(raw, guarded, unguarded);
        assert_eq!(
            probe.verdict(),
            ShapeVerdict::NotObservable,
            "素の綴りが {raw} トークン（＝特殊でない）なのに判定している"
        );
        assert_eq!(
            probe.failure(),
            None,
            "観測できない比較を失敗として扱っている: {raw} / {guarded} / {unguarded}"
        );
    }
}

/// **測った値が、入れ替わらずに欄へ入る**（要修正1 / レビューが実測した穴）。
///
/// # なぜ配線を見るのか
///
/// **器具の中で `guarded` と `unguarded` を入れ替える変異は、1749 全緑を通った。**
/// `ministral` の実測（11 / 9）が入れ替わると `9 > 11` が false になり、
/// **判定は `StillSpecial` に化けて「規則を見直せ」と言う**
/// ——**B-25 が消したはずの偽警報が、判定の中身ではなく配線から戻る。**
///
/// **入力ごとに違う値を返す偽の `measure`** を渡すので、どの2つを入れ替えても赤になる。
#[test]
fn the_wiring_cannot_swap_the_two_sides() {
    let shape = shape_fixture();

    // 3つとも違う値。**同値にすると入れ替えを検出できない**（CLAUDE.md の最頻の穴）。
    assert_ne!(shape.spelling(), shape.guarded_line());
    assert_ne!(shape.guarded_line(), shape.unguarded_line());

    let probe = probe_shape(&shape, |text| {
        if text == shape.spelling() {
            1
        } else if text == shape.guarded_line() {
            11
        } else if text == shape.unguarded_line() {
            9
        } else {
            panic!("測っていない文字列を判定に使っている: {text:?}")
        }
    });

    assert_eq!(probe.raw, 1, "素の綴りの実測が別の欄から来ている");
    assert_eq!(probe.guarded, 11, "防御ありの実測が別の欄から来ている");
    assert_eq!(probe.unguarded, 9, "防御なしの実測が別の欄から来ている");
    assert_eq!(probe.spelling, SHAPE_FIXTURE_SPELLING);
    assert_eq!(probe.verdict(), ShapeVerdict::Broken);
}

/// **防御なしの行は、崩した形から空白を1つ抜いて作る**（要修正5 / 上流を呼ばない主張）。
#[test]
fn the_unguarded_line_puts_the_spelling_back() {
    let shape = shape_fixture();

    assert_eq!(shape.spelling(), SHAPE_FIXTURE_SPELLING);
    assert_eq!(shape.neutralized(), "[FIXTURE_SHAPE ]");
    assert_eq!(shape.guarded_line(), SHAPE_FIXTURE_LINE);
    assert_eq!(
        shape.unguarded_line(),
        "> - [FIXTURE_SHAPE] ねこ",
        "防御なしの形が作れていない（無害化の形が変わった可能性）"
    );
    assert_ne!(
        shape.unguarded_line(),
        shape.guarded_line(),
        "防御あり／なしが同じ行になっている（比較が意味を持たない）"
    );

    // **測る対象は3つとも違う文字列**（同値だと配線の入れ替えが見えない）。
    let texts = shape.texts();
    assert_eq!(texts.len(), 3);
    for (index, text) in texts.iter().enumerate() {
        assert!(
            !texts[..index].contains(text),
            "測る対象に同じ文字列が2度ある: {text:?}"
        );
    }
}

/// **組み立ての3つの壊れ方が、それぞれ別の `Err` になる**（要修正5）。
///
/// **どれも `build_prompt` と `SafeText::sanitize` だけで決まる**ので、
/// **実機は1回も要らない。** 器具の中に置いていた頃は、どれも誰も見ていなかった。
#[test]
fn a_line_that_lost_its_defence_is_reported_before_any_measurement() {
    // 1. **無害化が素通りした**（この器具の存在理由そのもの）。
    let raw_line = "> - [FIXTURE_SHAPE] ねこ";
    let error = assembled_shape(raw_line, SHAPE_FIXTURE_SPELLING)
        .expect_err("原形が載った行は受け付けないこと");
    assert!(error.contains("原形"), "何が起きたか読めない: {error}");
    assert!(error.contains(raw_line), "どの行の話か読めない: {error}");

    // 2. 崩した形が行に無い（無害化の形が変わった）。
    let other_line = "> - [FIXTURE_SHAPE  ] ねこ";
    let error = assembled_shape(other_line, SHAPE_FIXTURE_SPELLING)
        .expect_err("崩した形が無い行は受け付けないこと");
    assert!(
        error.contains("[FIXTURE_SHAPE ]"),
        "探した形を名乗っていない: {error}"
    );

    // 3. **規則がその綴りを崩さない**（比較の前提が無い）。
    //    `>>…<<` は括弧が `[` / `<` でないので規則の外（`safe.rs` の対照と同じ軸）。
    //
    //    **⚠ 行に綴りを置かないこと。** 置くと 1 の枝（原形が載った）が先に鳴るので、
    //    **この枝を落としても同じ `Err` になり、区別が付かない**
    //    （着手時に実測: 綴りを置いた版では、この枝を潰す変異が **1756 全緑**を通った）。
    let outside = ">>FIXTURE<<";
    let error = assembled_shape("> - ねこ", outside).expect_err("規則の外の綴りは受け付けないこと");
    assert!(error.contains(outside), "どの綴りの話か読めない: {error}");
    assert!(
        error.contains("規則"),
        "規則の外だと言っていない（別の枝の文面が出ている）: {error}"
    );
}

/// **観測できないときの報告が、直す先の無い場所へ人を送らない**（作業単位 B-25 の本題）。
///
/// B-22 が `<|im_start|>` を対照から出した理由と同じ——
/// **「規則と一覧を見直すこと」と言われた人は、直す先が無い場所を探すことになる。**
///
/// # ⚠ フィクスチャは3つとも違う値（要修正3）
///
/// **`(8, 8, 8)` で見ていた頃は、防御あり／なしの入れ替えも、
/// 素の実測を防御ありに差し替える変異も、どちらも 0 赤だった。**
/// **Qwen で回した人が読む行はこれ1行だけ**なので、帰属が落ちると読み手は何も分からない。
/// C-14 が踏んだ `(8, 8, 8)` は判定側（`the_shape_comparison_…`）に残してある。
#[test]
fn the_unobservable_report_does_not_send_anyone_to_fix_the_rule() {
    let target = model(TARGET_MODEL);
    let report = shape_probe(8, 11, 9).report(&target);

    // **同じ行の中で**名乗ること（CLAUDE.md「その行が出た」と「その行がこう言っている」は別）。
    let line = line_containing(&report, "判定しない");
    assert!(
        line.contains(SHAPE_FIXTURE_SPELLING),
        "どの綴りの話か名乗っていない: {line}"
    );
    assert!(
        line.contains("= 8 トークン"),
        "素の実測が読めない（判断の材料が無い）: {line}"
    );
    assert!(
        line.contains("防御あり 11"),
        "防御ありの実測が読めない（または入れ替わっている）: {line}"
    );
    assert!(
        line.contains("防御なし 9"),
        "防御なしの実測が読めない（または入れ替わっている）: {line}"
    );
    assert!(
        line.contains(TARGET_MODEL),
        "どの上流に当てた結果か名乗っていない: {line}"
    );

    // **直す先の無い指示を出さない。** ここが B-25 が消した文面そのもの。
    for forbidden in ["見直すこと", "無害化が効いていない", "特殊トークンのまま"]
    {
        assert!(
            !report.contains(forbidden),
            "観測できないだけなのに、直しに行かせる文面が出ている（{forbidden}）: {report}"
        );
    }
}

/// **崩せていないときは、直す先を名乗る**（観測できるモデルでは判定を弱めない）。
#[test]
fn a_shape_that_stays_special_says_where_to_look() {
    // **防御ありと防御なしを違う値にする**——同値にすると取り違えが検出できない。
    let failure = shape_probe(1, 8, 9)
        .failure()
        .expect("特殊トークンのままなら失敗であること");

    assert!(
        failure.contains(SHAPE_FIXTURE_SPELLING),
        "どの綴りの話か名乗っていない: {failure}"
    );
    assert!(
        failure.contains("src/prompt/safe.rs"),
        "直す先を名乗っていない: {failure}"
    );

    // **どちらがどちらか**まで読めること（帰属）。
    let line = line_containing(&failure, "防御あり");
    assert!(
        line.contains("防御あり 8"),
        "防御ありの実測が読めない（または入れ替わっている）: {line}"
    );
    assert!(
        line.contains("防御なし 9"),
        "防御なしの実測が読めない（または入れ替わっている）: {line}"
    );
}

/// **3つの判定は、3つとも違う文面で出る**（要修正4 / doc がそう書いている）。
///
/// **`StillSpecial` の報告は、これが無いと一度も観測されていなかった**
/// （失敗の文面を見ているのは `failure()` の側）。
/// **実害**: 実機で落ちるとき、panic の直前に出る行が「崩せている」と言える——
/// **器具の出力を転記した人は、失敗の記録に成功の文面が並んだものを残す。**
#[test]
fn each_verdict_reads_differently() {
    let target = model(TARGET_MODEL);
    let broken = shape_probe(1, 11, 9).report(&target);
    let still_special = shape_probe(1, 8, 9).report(&target);
    let unobservable = shape_probe(8, 11, 9).report(&target);

    assert!(
        broken.contains("崩せている"),
        "崩せている判定がそう言っていない: {broken}"
    );
    assert!(
        still_special.contains("崩せていない"),
        "崩せていない判定がそう言っていない: {still_special}"
    );
    assert!(
        unobservable.contains("判定しない"),
        "観測できない判定がそう言っていない: {unobservable}"
    );

    assert_ne!(broken, still_special, "崩せている／いないが同じ文面");
    assert_ne!(broken, unobservable, "崩せている／判定しないが同じ文面");
    assert_ne!(
        still_special, unobservable,
        "崩せていない／判定しないが同じ文面"
    );

    // **崩せていない側の帰属**（要修正4 —— 両側を入れ替える変異はここで赤になる）。
    let line = line_containing(&still_special, "崩せていない");
    assert!(
        line.contains("防御あり 8"),
        "防御ありの実測が読めない（または入れ替わっている）: {line}"
    );
    assert!(
        line.contains("防御なし 9"),
        "防御なしの実測が読めない（または入れ替わっている）: {line}"
    );
}

/// **崩せているときの報告も、両側を取り違えずに出す。**
#[test]
fn the_report_tells_the_two_sides_apart() {
    let target = model(TARGET_MODEL);
    let report = shape_probe(1, 11, 9).report(&target);

    let line = line_containing(&report, "防御あり");
    assert!(
        line.contains("防御あり 11"),
        "防御ありの実測が読めない（または入れ替わっている）: {line}"
    );
    assert!(
        line.contains("防御なし 9"),
        "防御なしの実測が読めない（または入れ替わっている）: {line}"
    );
    assert!(
        line.contains(TARGET_MODEL),
        "どの上流に当てた結果か名乗っていない: {line}"
    );
}

/// **標準出力の塊が、実測2行と判定を全部持っている**（要修正1 / 報告を落とす変異）。
///
/// **「観測できないときは判定しない」を選べる根拠は「報告はする」の一点。**
/// 器具の `println!` から判定の行を落とす変異は 1749 全緑だった——
/// **塊の組み立てをここへ出したので、いまは落とせば赤になる。**
#[test]
fn the_block_shows_both_lines_and_the_verdict() {
    let target = model(TARGET_MODEL);
    let shape = shape_fixture();
    let probe = probe_shape(&shape, |text| {
        if text == shape.spelling() {
            8
        } else if text == shape.guarded_line() {
            11
        } else {
            9
        }
    });
    let block = shape_block(&target, &shape, &probe);

    // 実測の2行は、**行と数が同じ行の中で**読めること。
    let guarded = line_containing(&block, "防御あり");
    assert!(
        guarded.contains(shape.guarded_line()),
        "防御ありの行が載っていない: {guarded}"
    );
    assert!(
        guarded.contains("= 11 トークン"),
        "防御ありの実測が読めない（または入れ替わっている）: {guarded}"
    );
    assert!(
        guarded.contains(TARGET_MODEL),
        "どの上流に当てた結果か名乗っていない: {guarded}"
    );

    let unguarded = line_containing(&block, "防御なし ");
    assert!(
        unguarded.contains(shape.unguarded_line()),
        "防御なしの行が載っていない: {unguarded}"
    );
    assert!(
        unguarded.contains("= 9 トークン"),
        "防御なしの実測が読めない（または入れ替わっている）: {unguarded}"
    );

    // **判定の行が塊の中にあること**（落とすと、なぜ何も起きないのかが読めなくなる）。
    assert!(
        block.contains(&probe.report(&target)),
        "判定が塊から落ちている: {block}"
    );
    assert!(
        block.lines().count() >= 3,
        "塊が3行に満たない（どれかが落ちている）: {block}"
    );
}

/// **器具が比較に使う綴りは、兄弟のループも同じ上流で測っている**（判断を仰いだ点）。
///
/// [`ShapeProbe`] は**素の綴りが特殊でない上流では何も判定しない**ので、
/// **「崩した綴りが 1 トークンでないこと」を当てる口がそこには無い**
/// （B-22 の形の片方しか持っていない）。
/// **持っていないほうは、兄弟のループ**
/// （`measured_control_tokens_are_still_special_after_neutralizing`）
/// **が `MEASURED_CONTROL_TOKEN_SPELLINGS` の全件で当てている。**
///
/// **その「同じ綴りである」という前提だけが、ここで押さえられる。**
/// 器具の中に書くと、器具ごと走らない。
#[test]
fn the_compared_spelling_is_also_measured_by_the_sibling_loop() {
    let spelling = compared_shape_spelling();

    assert!(
        MEASURED_CONTROL_TOKEN_SPELLINGS.contains(&spelling.as_str()),
        "比較に使う綴りが実測データの一覧に無い（兄弟のループが同じ綴りを測っていない）: {spelling:?}"
    );
    assert_eq!(
        spelling,
        format!("[{COMPARED_SHAPE_CATEGORY}]"),
        "分類から組み上がる綴りが変わっている"
    );
    assert!(
        !COMPARED_SHAPE_CATEGORY.contains('[') && !COMPARED_SHAPE_CATEGORY.contains(']'),
        "分類が括弧を含んでいる（括弧は Seg::Lit 側から来る）: {COMPARED_SHAPE_CATEGORY}"
    );
}

/// ソースから**コメントを落とす**（走査の本体 / 要修正2）。
///
/// **ソースは引数で受け取る**——実ファイルだけを読む対照は自己成就する
/// （CLAUDE.md / `15b`）。
///
/// # ⚠ 見えないもの
///
/// * ブロックコメント（`/* … */`）。このリポジトリでは使っていない。
/// * 文字列リテラルの中の `//`。**この走査が探すのは識別子だけ**なので害は無い。
fn code_without_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// **走査は、コードを読んでいて散文を読んでいない**（要修正2 の対照）。
///
/// **植えたソースを走査本体に通す。** 実ファイルだけを見る検査は
/// 「1件も無いから緑」と「そもそも探せていないから緑」を区別できない。
#[test]
fn the_scan_reads_code_and_not_prose() {
    let planted = "/// shape_block と書いてあるだけの doc\n\
                   let probe = probe_shape(&shape, measure); // shape_block も末尾コメント\n";
    let code = code_without_comments(planted);

    assert!(
        !code.contains("shape_block"),
        "散文の中の名前を拾っている（doc に書くだけで検査が満たされる）: {code}"
    );
    assert!(
        code.contains("probe_shape(&shape"),
        "コードの側まで落としている: {code}"
    );
}

/// **器具が、比較を自前で書き直していないこと。**
///
/// # なぜ走査なのか
///
/// 器具の本体は `#[ignore]` で、**素の `cargo test` では1行も走らない。**
/// 判定・配線・報告を共有の純粋関数へ出しても、
/// **器具の中でそれらを使わずに書き戻せば、上の見張り番を全部素通りして B-25 が再発する。**
///
/// # ⚠ 見えないもの
///
/// **見ているのは下の綴りだけ。** 別の綴りで書き直した比較（`match` で書く等）は見えない。
/// **散文は見ない**（[`code_without_comments`] を通す）ので、
/// **doc に名前を書いただけでは、この検査は満たされない**——要修正2 で塞いだ穴。
#[test]
fn the_instrument_asks_the_shared_judgment_instead_of_comparing_by_itself() {
    assert!(
        CALIBRATION_INSTRUMENT.len() > 5_000,
        "器具を読めていない（{} バイト）",
        CALIBRATION_INSTRUMENT.len()
    );

    let code = code_without_comments(CALIBRATION_INSTRUMENT);
    for needle in [
        "assembled_shape(",
        "probe_shape(",
        "shape_block(",
        "failure()",
    ] {
        assert!(
            code.contains(needle),
            "器具が共有の判定を使っていない（自前で書いている可能性）: {needle}"
        );
    }
    // **器具は測った数に触らない。** 触れる場所が無ければ、比較の書き戻しに材料が無い
    // （別名に写す形——`let lhs = probe.guarded;`——もここで止まる）。
    for forbidden in [
        "guarded > unguarded",
        "probe.guarded",
        "probe.unguarded",
        "probe.raw",
    ] {
        assert!(
            !code.contains(forbidden),
            "器具が測った数を自分で読んでいる（判定を書き戻す材料になる）: {forbidden}"
        );
    }
}

// ------------------------------------------------------------------
// **対照の失敗の文面**（作業単位 B-27）。
//
// 器具の対照ループ（`measured_control_tokens_are_still_special_after_neutralizing`）は
// `#[ignore]` なので、**中に何を書いても素の `cargo test` は1行も通らない。**
// 側の判定も、報告も、失敗の文面も `tests/common/calibration.rs` に置き、ここが見張る。
// ------------------------------------------------------------------

/// **規則の内側**の対照のフィクスチャ（規則がすでに崩している形）。
///
/// **実物（`MEASURED_ORDINARY_LOOKALIKES` の綴り）を持ち込まない**
/// ——持ち込むと、文面の中で綴りを決め打つ変異が「たまたま同じ文字列」で素通りする
/// （`SHAPE_FIXTURE_SPELLING` と同じ作法）。
const INSIDE_LOOKALIKE_FIXTURE: &str = "[FIXTURE_LOOKALIKE]";

/// **規則の外側**の対照のフィクスチャ（開き括弧が `[` / `<` でない形）。
const OUTSIDE_LOOKALIKE_FIXTURE: &str = ">>FIXTURE_LOOKALIKE<<";

/// 判定の材料を組み立てる。**綴りは常にフィクスチャ。**
fn lookalike(spelling: &'static str, raw: u32) -> LookalikeProbe<'static> {
    LookalikeProbe { spelling, raw }
}

/// **側のラベルそのもの**（人が読む行で「どちらの針か」を名乗る語）。
///
/// # ⚠ `内側` / `外側` の2文字では足りない（B-27 のレビューが実測）
///
/// **内側の直す先の文面は「内側の針が0本にならないよう…」と書く。**
/// **`内側` だけを探すアサートは、その文面で満たされる**ので、
/// **側のラベルを誤ラベルする変異が 1811 全緑を通った。**
/// **⚠ 外側は当時も赤だった**（外側の文面に `外側` の語が無い）——
/// **片側だけが落ちる `D-6` の形。**
const INSIDE_LABEL: &str = "規則の内側の針";
const OUTSIDE_LABEL: &str = "規則の外側の針";

/// **側は、無害化そのものから引く**（作業単位 B-27）。
///
/// 期待値は**直に書く** ——`SafeText::sanitize` から作ると
/// **同じ式で両側を作る**ことになり、何も見ていない（CLAUDE.md）。
/// **`Inside` / `Outside` を入れ替える変異は、ここで赤になる。**
#[test]
fn the_side_of_a_lookalike_is_read_out_of_the_neutralizer() {
    for (spelling, expected) in [
        // 規則が崩す形。
        ("[FIXTURE_LOOKALIKE]", LookalikeSide::Inside),
        // `|` を本体に許した後の形（作業単位 B-22）。
        ("<FIXTURE|LOOKALIKE>", LookalikeSide::Inside),
        ("[]", LookalikeSide::Inside),
        // 開き括弧が `[` / `<` でない。
        (">>FIXTURE_LOOKALIKE<<", LookalikeSide::Outside),
        // 本体が非 ASCII（全角の `\u{FF5C}`）。
        ("<\u{FF5C}FIXTURE\u{FF5C}>", LookalikeSide::Outside),
        // 本体に空白があれば形ではない。
        ("[FIXTURE LOOKALIKE]", LookalikeSide::Outside),
        ("ねこ", LookalikeSide::Outside),
    ] {
        assert_eq!(
            lookalike_side(spelling),
            expected,
            "側の判定が規則と食い違っている: {spelling:?}"
        );
    }

    // **2つのフィクスチャは必ず別の側**（同じ側だと、下の2本が同じ主張になる）。
    assert_ne!(
        lookalike_side(INSIDE_LOOKALIKE_FIXTURE),
        lookalike_side(OUTSIDE_LOOKALIKE_FIXTURE),
        "内側と外側のフィクスチャが同じ側になっている"
    );

    // **実物の一覧にも当てる**（手書きのフィクスチャだけで回さない / B-27 のレビュー）。
    // **両側がそろっていること**まで見る——`lookalike_side` を片側へ決め打つ変異は、
    // フィクスチャだけでなくここでも鳴る。
    //
    // **⚠ 綴りごとの答えが規則（`control_token_shape_len`）と一致することは、
    // `src/prompt/safe.rs` の `the_two_side_judgements_agree_on_every_lookalike` が見ている**
    // ——規則は `#[cfg(test)]` なのでこの crate からは呼べない。**ここに写しを作らない。**
    let inside = MEASURED_ORDINARY_LOOKALIKES
        .into_iter()
        .filter(|spelling| lookalike_side(spelling) == LookalikeSide::Inside)
        .count();
    assert!(
        inside >= 1,
        "実物の一覧に内側の針が1本も無い（判定器が片側へ決め打たれている可能性）"
    );
    assert!(
        MEASURED_ORDINARY_LOOKALIKES.len() - inside >= 1,
        "実物の一覧に外側の針が1本も無い（判定器が片側へ決め打たれている可能性）"
    );
}

/// **規則の内側の針が特殊になっても、規則を直しに行かせない**（作業単位 B-27 の本題）。
///
/// # 何が誤っていたのか
///
/// 対照が特殊トークンになったときの文面は
/// **「src/prompt/safe.rs の規則と一覧を見直すこと」**の1つだけだった。
///
/// * **規則の外側の針**（`>>ABSTRACT<<` 型）→ **正しい。規則を広げる先がある**（B-22 の前例）。
/// * **規則の内側の針**（`[control_8]` 型）→ **規則側に直す先が無い。**
///   **規則はすでにその綴りを崩している**（それが「内側」の意味）。
///
/// **B-25 とは形が違う。** あちらは比較そのものが**原理的に無意味**になるので
/// **判定を止めた。** こちらは**警報そのものが正しい事実を言っている**
/// （対照が特殊になった＝その一覧の帰属が崩れた）ので、**判定は止めない。**
/// **誤っているのは指示の半分だけ**なので、**文面を内外で分ける。**
#[test]
fn an_inside_lookalike_is_not_sent_to_fix_the_rule() {
    let probe = lookalike(INSIDE_LOOKALIKE_FIXTURE, 1);
    let failure = probe
        .failure()
        .expect("対照が特殊トークンになったら、内側でも失敗であること（判定は止めない）");

    // **同じ行の中で**名乗ること（CLAUDE.md「その行が出た」と「その行がこう言っている」は別）。
    let line = line_containing(&failure, "対照が特殊トークンになった");
    assert!(
        line.contains(INSIDE_LOOKALIKE_FIXTURE),
        "どの綴りの話か名乗っていない: {line}"
    );
    assert!(
        line.contains("が 1 トークン"),
        "実測が読めない（判断の材料が無い）: {line}"
    );
    // **⚠ ラベルそのものを見る**（`内側` の2文字では足りない / B-27 のレビューが実測）。
    // 内側の直す先の文面は「**内側の針**が0本にならないよう…」と書くので、
    // **`内側` だけを探すと、側のラベルを誤ラベルする変異が 1811 全緑を通る。**
    // **反対側が出ていないこと**まで見て、両向きの取り違えを止める。
    assert!(
        line.contains(INSIDE_LABEL),
        "内側の針だと名乗っていない: {line}"
    );
    assert!(
        !line.contains(OUTSIDE_LABEL),
        "内側の針なのに外側だと名乗っている: {line}"
    );

    // **直す先は「一覧の帰属」。** 移す先まで名乗る（B-22 が `<|im_start|>` でやった形）。
    assert!(
        failure.contains("MEASURED_ORDINARY_LOOKALIKES"),
        "どの一覧から外すのか名乗っていない: {failure}"
    );
    assert!(
        failure.contains("MEASURED_CONTROL_TOKEN_SPELLINGS"),
        "移す先を名乗っていない: {failure}"
    );
    assert!(
        failure.contains("MEASURED_MODEL_DEPENDENT_SPELLINGS"),
        "モデルで答えが変わる場合の移す先を名乗っていない: {failure}"
    );

    // **規則に直す先が無いのに、規則を直しに行かせない。**
    for forbidden in ["規則と一覧を見直すこと", "広げる", "is_shape_body"] {
        assert!(
            !failure.contains(forbidden),
            "規則には直す先が無いのに、規則を直しに行かせている（{forbidden}）: {failure}"
        );
    }
}

/// **規則の外側の針が特殊になったら、規則を広げに行かせる**（作業単位 B-27）。
///
/// **こちらは直す先が実在する** ——`<|im_start|>` が Qwen で 1 トークンだったとき、
/// **一覧を増やすのではなく規則を広げた**（B-22 / `is_shape_body` に `|`）。
#[test]
fn an_outside_lookalike_is_sent_to_widen_the_rule() {
    let probe = lookalike(OUTSIDE_LOOKALIKE_FIXTURE, 1);
    let failure = probe
        .failure()
        .expect("対照が特殊トークンになったら失敗であること");

    let line = line_containing(&failure, "対照が特殊トークンになった");
    assert!(
        line.contains(OUTSIDE_LOOKALIKE_FIXTURE),
        "どの綴りの話か名乗っていない: {line}"
    );
    assert!(
        line.contains("が 1 トークン"),
        "実測が読めない（判断の材料が無い）: {line}"
    );
    assert!(
        line.contains(OUTSIDE_LABEL),
        "外側の針だと名乗っていない: {line}"
    );
    assert!(
        !line.contains(INSIDE_LABEL),
        "外側の針なのに内側だと名乗っている: {line}"
    );

    // **直す先は規則。** どこを触るのかまで名乗る。
    assert!(
        failure.contains("src/prompt/safe.rs"),
        "直す先のファイルを名乗っていない: {failure}"
    );
    assert!(
        failure.contains("is_shape_body"),
        "規則のどこを広げるのか名乗っていない: {failure}"
    );
    assert!(
        failure.contains("広げる"),
        "規則を広げる方向だと言っていない: {failure}"
    );

    // **一覧へ移す方向へ誘導しない**（B-22 は一覧を1件も増やしていない）。
    assert!(
        !failure.contains("移すこと"),
        "規則を広げる場面で、一覧を動かしに行かせている: {failure}"
    );

    // **内側と同じ文面にならない**（分けたことの実体）。
    assert_ne!(
        failure,
        lookalike(INSIDE_LOOKALIKE_FIXTURE, 1)
            .failure()
            .expect("内側も失敗であること"),
        "内側と外側で同じ文面が出ている（分かれていない）"
    );
}

/// **特殊トークンでない対照は、失敗ではない**（判定を広げすぎない）。
#[test]
fn a_lookalike_that_is_still_ordinary_is_not_a_failure() {
    let target = model(TARGET_MODEL);

    for spelling in [INSIDE_LOOKALIKE_FIXTURE, OUTSIDE_LOOKALIKE_FIXTURE] {
        // `0` は測れなかった場合、`4` / `5` / `7` は実測にある値。
        for raw in [0, 2, 4, 5, 7] {
            let probe = lookalike(spelling, raw);
            assert_eq!(
                probe.failure(),
                None,
                "特殊トークンでない対照を失敗として扱っている: {spelling:?} / {raw}"
            );

            let report = probe.report(&target);
            assert!(
                report.contains(&format!("= {raw} トークン")),
                "実測が読めない: {report}"
            );
            for forbidden in ["見直す", "広げる", "移すこと"] {
                assert!(
                    !report.contains(forbidden),
                    "何も起きていないのに直しに行かせている（{forbidden}）: {report}"
                );
            }
        }
    }
}

/// **4通りの報告が、4通りとも違う文面で出る**（側 × 特殊かどうか）。
///
/// **人が読む行はこれしかない。** 側が落ちると、Qwen で回した人は
/// **どちらを直せばよいのか分からないまま「規則」と書かれた指示を読む。**
#[test]
fn each_lookalike_report_reads_differently() {
    let target = model(TARGET_MODEL);

    // **4通りとも「どちらのラベルが出るべきか」を書く。**
    // **⚠ `内側` または `外側` を含むこと、では帰属を一切見ていない**
    // （`||` なので両方が外側でも通る / B-27 のレビューが実測）。
    let reports = [
        (
            lookalike(INSIDE_LOOKALIKE_FIXTURE, 1).report(&target),
            INSIDE_LABEL,
            OUTSIDE_LABEL,
        ),
        (
            lookalike(OUTSIDE_LOOKALIKE_FIXTURE, 1).report(&target),
            OUTSIDE_LABEL,
            INSIDE_LABEL,
        ),
        (
            lookalike(INSIDE_LOOKALIKE_FIXTURE, 5).report(&target),
            INSIDE_LABEL,
            OUTSIDE_LABEL,
        ),
        (
            lookalike(OUTSIDE_LOOKALIKE_FIXTURE, 5).report(&target),
            OUTSIDE_LABEL,
            INSIDE_LABEL,
        ),
    ];

    for (index, (report, expected, forbidden)) in reports.iter().enumerate() {
        assert!(
            !reports[..index].iter().any(|(other, _, _)| other == report),
            "別の状態が同じ文面で出ている: {report}"
        );
        assert!(
            report.contains(TARGET_MODEL),
            "どの上流に当てた結果か名乗っていない: {report}"
        );

        // **側のラベルは、特殊になった側でもならなかった側でも正しく出る。**
        assert!(
            report.contains(expected),
            "側のラベルが読めない（期待: {expected}）: {report}"
        );
        assert!(
            !report.contains(forbidden),
            "反対側のラベルが出ている（誤ラベル: {forbidden}）: {report}"
        );
    }
}

/// **器具が、対照の判定と報告を共有のものから出していること**（作業単位 B-27）。
///
/// 器具の中に `raw == SPECIAL` を書き戻せば、**文面を分けた意味がその1行で消える。**
///
/// # ⚠ 報告の行も見る（B-27 のレビューが実測）
///
/// **`probe.report()` の `println!` を丸ごと落とす変異は 1811 全緑で、clippy も 0 だった。**
/// **実害**: 新しい上流で器具を回した人が、**対照6件の実測行を1行も見られない。**
/// **判定に届かない値**（`= 4 トークン` など）**と側のラベルは、この `println!` にしか出ない。**
/// 兄弟の `shape_block` については、この針が最初から入っていた。
///
/// # ⚠ 見ているのは下の綴りだけ（**別の書き方で書き直した判定は見えない**）
///
/// **肯定側の針は、ループ本体にしか現れない形で書く**
/// ——`use` 行だけで満たされる綴り（`LookalikeProbe` 単体）にすると、
/// **器具を直書きに戻しても `use` が残っている限り緑になる**（実測。
/// 当時止めていたのは clippy の unused import であって、このテストではなかった）。
///
/// **否定側は綴りの一覧でしかない。** `SPECIAL == raw` のように**左右を入れ替えた**書き方や、
/// **別の定数に写してから比べる**書き方は、ここでは見えない。
/// **そこは肯定側の3本**（`LookalikeProbe {` / `probe.report(` / `probe.failure()`）
/// **が「共有の判定を通っていること」を要求することで塞ぐ**
/// ——通したうえで**さらに**自前の判定を足す形だけが、この一覧の網の目である。
/// 兄弟の `the_instrument_asks_the_shared_judgment_instead_of_comparing_by_itself` と同じ但し書き。
#[test]
fn the_instrument_uses_the_shared_lookalike_judgment() {
    let code = code_without_comments(CALIBRATION_INSTRUMENT);

    // **ループ本体にしか現れない綴り**（`use` 行では満たされない）。
    for needle in ["LookalikeProbe {", "probe.report(", "probe.failure()"] {
        assert!(
            code.contains(needle),
            "器具が共有の判定・報告を使っていない（自前で書いている可能性）: {needle}"
        );
    }

    for forbidden in ["raw == SPECIAL", "SPECIAL == raw"] {
        assert!(
            !code.contains(forbidden),
            "器具が対照の判定を自前で書いている（文面を分けた意味が消える）: {forbidden}"
        );
    }
}

// ------------------------------------------------------------------
// **制御トークンの当て直しの判定と文面**（作業単位 B-35）。
//
// 器具の2つのループ（`MEASURED_CONTROL_TOKEN_SPELLINGS` と
// `MEASURED_MODEL_DEPENDENT_SPELLINGS`）は `#[ignore]` の中にあり、
// **中に書いた判定も文面も素の `cargo test` では1行も走らない。**
// 判定は `tests/common/calibration.rs` の `ControlProbe` に置き、ここが見張る
// （`ShapeProbe` / `LookalikeProbe` と同じ作法）。
// ------------------------------------------------------------------

/// 制御トークンの判定のフィクスチャ。**規則が崩す形**で、対照の綴りとは別物。
const CONTROL_FIXTURE_SPELLING: &str = "[FIXTURE_CONTROL]";

/// 上の綴りを無害化した形。
///
/// **`SafeText::sanitize` を呼んで作らない。** 呼んで作ると、
/// `probe_control` が崩した形を求めるのをやめる変異と**同時に動いて緑のまま**になる
/// （`15b` の「対照が自己成就する」形）。
const CONTROL_FIXTURE_NEUTRALIZED: &str = "[FIXTURE_CONTROL ]";

/// 実測2点を与えて [`ControlProbe`] を組む。
///
/// **知らない文字列を測ろうとしたら落ちる**——`probe_control` が
/// 崩した形を求めなくなったら、ここで止まる。
fn control_probe(kind: ControlListKind, raw: u32, broken: u32) -> ControlProbe<'static> {
    probe_control(kind, CONTROL_FIXTURE_SPELLING, move |text| {
        if text == CONTROL_FIXTURE_SPELLING {
            raw
        } else if text == CONTROL_FIXTURE_NEUTRALIZED {
            broken
        } else {
            panic!("測っていない文字列を判定に使っている: {text:?}")
        }
    })
}

/// **どの実測がどの欄へ入るかの配線**（B-25 のレビューが `ShapeProbe` で実測した穴）。
#[test]
fn the_control_probe_wiring_cannot_swap_the_two_sides() {
    // **2つを同値にすると入れ替えを検出できない**（CLAUDE.md の最頻の穴）。
    assert_ne!(CONTROL_FIXTURE_SPELLING, CONTROL_FIXTURE_NEUTRALIZED);

    let probe = control_probe(ControlListKind::Measured, 1, 7);

    assert_eq!(probe.raw, 1, "素の綴りの実測が別の欄から来ている");
    assert_eq!(probe.broken, 7, "無害化後の実測が別の欄から来ている");
    assert_eq!(probe.spelling, CONTROL_FIXTURE_SPELLING);
    assert_eq!(
        probe.neutralized, CONTROL_FIXTURE_NEUTRALIZED,
        "崩した形が SafeText::sanitize から来ていない"
    );
    assert_eq!(probe.kind, ControlListKind::Measured);
    assert!(probe.failures().is_empty(), "{:?}", probe.failures());
}

/// **素の綴りが特殊でなくなったことを当てるのは、実測の一覧の側だけ**（B-22 の形）。
#[test]
fn a_spelling_that_stopped_being_special_fails_only_on_the_measured_list() {
    let measured = control_probe(ControlListKind::Measured, 2, 7).failures();
    assert_eq!(measured.len(), 1, "{measured:?}");
    assert!(
        measured[0].contains("特殊トークンでなくなった"),
        "{measured:?}"
    );
    assert!(
        measured[0].contains("MEASURED_CONTROL_TOKEN_SPELLINGS"),
        "直す先を名乗っていない: {measured:?}"
    );

    // **モデル依存の一覧では、同じ数でも何も言わない**（どちらの答えも正しいから）。
    let dependent = control_probe(ControlListKind::ModelDependent, 2, 7).failures();
    assert!(
        dependent.is_empty(),
        "モデル依存の綴りで素の値を判定している: {dependent:?}"
    );
}

/// **無害化が空振りしていることは、どちらの一覧でも当てる。**
#[test]
fn a_neutralized_spelling_that_stays_one_token_fails_on_both_lists() {
    for kind in [ControlListKind::Measured, ControlListKind::ModelDependent] {
        let failures = control_probe(kind, 1, 1).failures();
        assert_eq!(failures.len(), 1, "{kind:?} / {failures:?}");
        assert!(failures[0].contains("無害化が効いていない"), "{failures:?}");
        assert!(
            failures[0].contains("control_token_shape_len"),
            "直す先を名乗っていない: {failures:?}"
        );
    }
}

/// **無害化で文字が落ちたら言う**（空白を除けば元の綴りに戻るはず）。
///
/// 綴りの中の水平タブは `\u{9}` で書く。**生の不可視文字を置かない**（CLAUDE.md）。
#[test]
fn a_neutralizing_that_drops_characters_is_reported() {
    let lossy = "[A\u{9}B]";
    let probe = probe_control(ControlListKind::ModelDependent, lossy, |_| 5);
    let failures = probe.failures();

    assert_eq!(probe.neutralized, "[A B]", "前提が変わった");
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].contains("無害化で文字が失われた"),
        "{failures:?}"
    );
    // 文面は `{:?}` で綴りを出すので、タブはここでも `\t` に見える。
    assert!(failures[0].contains("\"[A\\tB]\""), "{failures:?}");
    assert!(failures[0].contains("\"[A B]\""), "{failures:?}");
}

/// **一覧ごとに文面が違い、2つの実測はそれぞれの場所に出る。**
#[test]
fn each_control_list_kind_reads_differently() {
    let measured = control_probe(ControlListKind::Measured, 1, 7);
    let dependent = control_probe(ControlListKind::ModelDependent, 1, 7);

    assert_eq!(
        measured.report(),
        "\"[FIXTURE_CONTROL]\" = 1 トークン / 無害化後 \"[FIXTURE_CONTROL ]\" = 7 トークン"
    );
    assert_eq!(
        dependent.report(),
        "モデル依存 \"[FIXTURE_CONTROL]\" = 1 トークン / 無害化後 \"[FIXTURE_CONTROL ]\" = 7 トークン（素の値は判定しない）"
    );
    assert_ne!(measured.report(), dependent.report());
}

/// **どちらの一覧でも、健全なら何も言わない**（毎回鳴る警報は読まれなくなる）。
#[test]
fn a_healthy_control_probe_says_nothing() {
    for kind in [ControlListKind::Measured, ControlListKind::ModelDependent] {
        assert!(
            control_probe(kind, 1, 7).failures().is_empty(),
            "{kind:?} で偽の警報が出た"
        );
    }
}

/// **器具が、制御トークンの判定を自前で書き直していないこと**（作業単位 B-35）。
///
/// 兄弟の `the_instrument_asks_the_shared_judgment_instead_of_comparing_by_itself` と同じ但し書き
/// （見ているのは下の綴りだけ / 散文は見ない）。
#[test]
fn the_instrument_uses_the_shared_control_judgment() {
    let code = code_without_comments(CALIBRATION_INSTRUMENT);

    for needle in ["probe_control(", "probe.report()", "probe.failures()"] {
        assert!(
            code.contains(needle),
            "器具が共有の判定・報告を使っていない（自前で書いている可能性）: {needle}"
        );
    }

    // **器具は測った数にも、崩した形の判定にも触らない。**
    for forbidden in [
        "probe.broken",
        "probe.neutralized",
        "broken < 2",
        "!= SPECIAL",
        "SPECIAL_TOKEN_COUNT",
    ] {
        assert!(
            !code.contains(forbidden),
            "器具が制御トークンの判定を自前で書いている: {forbidden}"
        );
    }
}

// ------------------------------------------------------------------
// **B-35 のレビューが「緑を通り抜けた」と名指しした穴**（要修正1 / 2 / 5）。
// ------------------------------------------------------------------

/// **E-1 の器具**（`tests/control_token_survey.rs`）のソース。
///
/// **兄弟の [`CALIBRATION_INSTRUMENT`] と同じ形** ——このファイルは
/// **すでに別のテストファイルを読み込んで走査している**ので、写しは1つも増えない。
const SURVEY_INSTRUMENT: &str = include_str!("control_token_survey.rs");

/// **一覧と判定の対が、表として固定されていること**（要修正1）。
///
/// # なぜ位置ではなく中身で当てるのか
///
/// 表の2行を入れ替える変異を、**順番ではなく「どちらの一覧がどちらの判定か」で捕まえる。**
/// 位置で当てると、表を並べ替えただけの無害な変更でも赤くなる。
#[test]
fn each_control_list_is_paired_with_its_own_judgement() {
    // **2つの一覧が同じものだと、入れ替えを検出できない**（CLAUDE.md の最頻の穴）。
    assert_ne!(
        MEASURED_CONTROL_TOKEN_SPELLINGS.as_slice(),
        MEASURED_MODEL_DEPENDENT_SPELLINGS.as_slice()
    );

    let mut measured = 0;
    let mut dependent = 0;
    for (kind, spellings) in CONTROL_LISTS {
        match kind {
            ControlListKind::Measured => {
                measured += 1;
                assert_eq!(
                    spellings,
                    MEASURED_CONTROL_TOKEN_SPELLINGS.as_slice(),
                    "実測の一覧が別の判定へ回っている（素の綴りの検査が消える）"
                );
            }
            ControlListKind::ModelDependent => {
                dependent += 1;
                assert_eq!(
                    spellings,
                    MEASURED_MODEL_DEPENDENT_SPELLINGS.as_slice(),
                    "モデル依存の一覧が別の判定へ回っている（ministral で偽の警報が出る）"
                );
            }
        }
    }

    // **どちらの一覧も、ちょうど1回ずつ回る。**
    assert_eq!((measured, dependent), (1, 1), "{CONTROL_LISTS:?}");
}

/// **器具が対を自分で決めていないこと**（要修正1 の走査側）。
///
/// 兄弟の `the_instrument_asks_the_shared_judgment_instead_of_comparing_by_itself` と同じ但し書き
/// （見ているのは下の綴りだけ / 散文は見ない）。
#[test]
fn the_instrument_reads_the_pairing_from_the_shared_table() {
    let code = code_without_comments(CALIBRATION_INSTRUMENT);

    assert!(
        code.contains("CONTROL_LISTS"),
        "器具が共有の表を回していない（自分で対を決めている可能性）"
    );
    assert!(
        !code.contains("ControlListKind::"),
        "器具が自分で kind を選んでいる（2行を入れ替える変異が誰にも見えなくなる）"
    );
}

/// **無害化後がちょうど 2 トークンなら健全**（要修正5 / 境界）。
///
/// **`broken < 2` を `< 3` へ緩める変異は、この1本でだけ赤くなる。**
/// **緩んだ側の実害は偽の警報**——`[]` → `[ ]` のような短い形は実際に 2 になりえて、
/// **落ちても直す先が無い**（B-25）。
#[test]
fn a_neutralized_spelling_of_exactly_two_tokens_is_healthy() {
    for kind in [ControlListKind::Measured, ControlListKind::ModelDependent] {
        let failures = control_probe(kind, 1, 2).failures();
        assert!(
            failures.is_empty(),
            "{kind:?} で境界（無害化後 2 トークン）に偽の警報が出た: {failures:?}"
        );
    }

    // **危ない向きは、いまも鳴る。**（1 トークンのまま＝無害化が空振り）
    assert_eq!(
        control_probe(ControlListKind::Measured, 1, 1)
            .failures()
            .len(),
        1
    );
}

/// **E-1 の器具が、報告を出して、穴で落ちること**（要修正2）。
///
/// # なぜ走査なのか
///
/// 器具は **`#[ignore]`** で、**残っているのは `println!` と `assert!` の2行だけ。**
/// **その2行はどのテストからも観測できない**ので、
/// **`assert!` を消しても `println!` を消しても全緑だった**（レビュアーが実測 / 2026-09-02）。
/// **`survey_all` の戻り値の権威**（`the_whole_report_names_every_file_including_the_unreadable_one`）
/// **は、器具がそれを見ているかの権威ではない。**
///
/// # ⚠ 見えないもの
///
/// **見ているのは下の綴りだけ。** 別の綴りで書き直した器具は見えない。
/// **散文は見ない**（[`code_without_comments`] を通す）ので、
/// **doc に名前を書いただけでは、この検査は満たされない。**
#[test]
fn the_survey_instrument_prints_the_block_and_fails_on_the_gaps() {
    assert!(
        SURVEY_INSTRUMENT.len() > 5_000,
        "器具を読めていない（{} バイト）",
        SURVEY_INSTRUMENT.len()
    );

    // **⚠ ファイル全体ではなく、器具の本体だけを見る。**
    // `survey_block` / `survey_failure` はこのファイルの中で**定義され、見張り番からも呼ばれる**ので、
    // ファイル全体に「使っていないこと」を当てると、必ず落ちる。
    let body = function_body(
        SURVEY_INSTRUMENT,
        "fn every_installed_gguf_is_covered_by_the_neutralizing_rule()",
    );

    // **⚠ 切り出しが関数の終わりで止まっていることを、ここでも当てる。**
    // 止まらずにファイル末尾まで伸びると、**後ろの定義から `survey_block(` を拾って
    // 「器具が自前で組み立てている」という誤った失敗**になる（`main` の CRLF で実際に起きた）。
    // **行数で見るのは、改行の綴りに依らないから。**
    let lines = body.lines().count();
    // **⚠ この閾値 40 を固定しているものは何も無い**（B-35 のレビュー / 2026-09-02）。
    // 器具の本体は 20 行ほどで、切り出しが止まらないと 170 行を超えるので幅は広い。
    // **いまは下の禁止綴りの検査にマスクされていて、単独では観測できない**（CLAUDE.md「第4の分類」）。
    // **置いてあるのは、器具がファイル末尾へ移った日にはこの門だけが権威になるから。**
    assert!(
        lines < 40,
        "本体の切り出しが関数の終わりで止まっていない（{lines} 行）。改行の綴りを疑うこと"
    );

    let code = code_without_comments(body);

    for needle in [
        "survey_all(&read)",
        "println!(\"{block}\")",
        "assert!(failures.is_empty()",
        // **器具の配線も固定する**（要修正2 / B-35 のレビュー）。
        // **1本目が消えると、0 件を調べて「未覆いゼロ」で成功する** ——
        // **「7つの gguf で未覆いゼロ」と「0 件で未覆いゼロ」は、終了コードでは区別できない。**
        // 残り2本は**純粋関数を実際の環境・ファイルへ繋ぐ唯一の場所**。
        // **同じ形なので3本まとめて固定する**（1本だけ塞ぐのは D-6 の再演）。
        "assert!(!files.is_empty()",
        "gguf_search_root(|key| std::env::var(key).ok())",
        "BufReader::with_capacity(1 << 20, file)",
    ] {
        assert!(
            code.contains(needle),
            "器具が組み立て・報告・判定のどれかをやめている: {needle}"
        );
    }

    // **器具は結果を自分で作り直さない。** 触れる場所が無ければ、判定の書き戻しに材料が無い。
    for forbidden in ["survey_block(", "survey_failure(", ".uncovered"] {
        assert!(
            !code.contains(forbidden),
            "器具が報告・判定を自前で組み立てている（survey_all を通していない）: {forbidden}"
        );
    }
}

/// ソースから**その関数の本体だけ**を切り出す（走査の本体 / 要修正2）。
///
/// **ソースは引数で受け取る**——実ファイルだけを読む対照は自己成就する
/// （CLAUDE.md / `15b`）。見張り番は [`the_body_slice_stops_at_the_function_end`]。
///
/// # ⚠ 改行の綴りに依存しないこと（**`main` で実際に落ちた**）
///
/// 以前は `rest.find` に `\n}\n` を渡して終端を探していた。
/// **`main` のツリーは CRLF なので `\r\n}\r\n` になり、一致せず、
/// 切り出しがファイル末尾まで伸びて後ろの定義を拾った**
/// （`器具が報告・判定を自前で組み立てている: survey_block(` で落ちる）。
/// **隔離ワークツリーの新規ファイルは LF なので、そこでは緑だった**
/// （`.gitattributes` が無く `core.autocrlf = true` / CLAUDE.md）。
/// **つまり、誰がクローンしても落ちる形だった。**
///
/// いまは**行単位**で見る。`split_inclusive` に `'\n'` を渡す形は
/// **`\n` と `\r\n` のどちらでも同じところで行を割る**ので、両方の改行で同じ答えを返す。
///
/// # ⚠ 見えないもの
///
/// 終端は**列 0 の `}` で始まる行**で決める。このリポジトリの整形（`rustfmt`）では
/// それが関数の終わりだが、**1行に畳まれた関数**は切り出せない。
fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("関数が見つからない: {signature}"));
    let rest = &source[start..];

    let mut end = 0;
    for line in rest.split_inclusive('\n') {
        end += line.len();
        // 列 0 の `}` ＝ 関数の終わり。CRLF でも LF でも同じところで止まる。
        if line.starts_with('}') {
            break;
        }
    }

    &rest[..end]
}

/// **切り出しは、その関数の終わりで、ちょうど終わる**（要修正2 の対照 / 植えたソースを本体に通す）。
///
/// # ⚠ 両方の改行で測る
///
/// **以前の対照は LF だけだった。** 実ファイルは `main` では CRLF になるので、
/// **対照が実ファイルより弱く、改行に依存する終端探しの穴を作り直せた**
/// （`F-2` と同じ形 —— 本命だけ赤・植えた対照は緑）。
/// **同じ本体に、同じ植えたソースを、改行だけ変えて通す。**
///
/// # ⚠ 「どこで終わったか」まで固定する（**含む／含まないでは足りない**）
///
/// 「本体の最初の文を含む」「前後の関数を含まない」だけを見ていたとき、
/// **次の2つが両方とも通り抜けた**（B-35 のレビューが実測 / 2026-09-02）:
///
/// | 変異 | 何が起きるか | 緩い検査では |
/// |---|---|---|
/// | **字下げされた `}` で止まる**（`trim_start().starts_with('}')`） | 本体が途中で切れる | **通る** |
/// | **`lines()` ＋ `end += line.len() + 1`** | **CRLF で1行あたり1バイト足りず、末尾が切れる** | **通る** |
///
/// **⚠ 2つ目は、今日直した欠陥のちょうど逆向きである** ——
/// **「CRLF 安全にしよう」と `lines()` へ書き換えて改行を1バイトで足し戻すと、
/// LF では厳密に等価、CRLF でだけ壊れる。**
/// **だから期待値そのものを、両方の改行で `assert_eq!` に固定する。**
///
/// # ⚠ 入力も期待値も、両方の改行でリテラルに書く
///
/// 片方をもう片方から作る（`replace` で導く）と、**作り方が壊れた日に期待値も一緒に動く。**
#[test]
fn the_body_slice_stops_at_the_function_end() {
    let planted_lf = "fn other() {\n    let a = 1;\n}\n\
                      fn target() {\n    let inside = 2;\n    if x {\n    }\n}\n\
                      fn after() {\n    let outside = 3;\n}\n";
    let planted_crlf = "fn other() {\r\n    let a = 1;\r\n}\r\n\
                        fn target() {\r\n    let inside = 2;\r\n    if x {\r\n    }\r\n}\r\n\
                        fn after() {\r\n    let outside = 3;\r\n}\r\n";

    let expected_lf = "fn target() {\n    let inside = 2;\n    if x {\n    }\n}\n";
    let expected_crlf = "fn target() {\r\n    let inside = 2;\r\n    if x {\r\n    }\r\n}\r\n";

    // **同値だと、改行の違いを一度も測っていない。**
    assert_ne!(planted_lf, planted_crlf);
    assert_ne!(expected_lf, expected_crlf);

    assert_eq!(
        function_body(planted_lf, "fn target()"),
        expected_lf,
        "LF: 切り出しが関数の終わりで、ちょうど終わっていない"
    );
    assert_eq!(
        function_body(planted_crlf, "fn target()"),
        expected_crlf,
        "CRLF: 切り出しが関数の終わりで、ちょうど終わっていない"
    );
}
