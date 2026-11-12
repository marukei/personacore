//! **性格ステータスの変容を走らせる**（[ADR-0024](../../docs/adr/0024-stat-drift.md) / 作業単位 16b）。
//!
//! 16a の純粋関数（`prompt::drift` / `parse::drift` / `domain::drift`）と
//! 16b の書き込み経路（`store::drift`）に、**いつ・どう走らせるか**を与える層。
//!
//! ```text
//! server::extract::run（抽出タスクの中。**抽出の直後に直列**）
//!   → run_drift                     … スイッチが on のときだけ呼ばれる（§8）
//!        Store::stat_allocs          … **保存済みの軸**（＝許可リスト）。上流を呼ぶ前
//!        prompt::drift::build_drift_prompt … 依頼を組む（純粋。載る軸が 0 なら `None`）
//!        （長すぎたら捨てて数える）
//!        LlmClient::complete         … **サブモデル**。トランザクションの外
//!        （打ち切られていたら丸ごと捨てて数える / ADR-0027 §打ち切り）
//!        parse::drift::parse_drift   … 分解（純粋。panic しない）
//!        domain::apply_drift         … 許可リスト・対象外・飽和（純粋）
//!        Store::record_drift_vote    … **票を1件足す**（ADR-0026 §決定3。**追記のみ**）
//!        Store::apply_stat_drift     … **1軸だけを動かす単文**（§7）。**許可証が要る**
//!        1イベント記録               … 結末・軸・前後の値・向き・票・書き込みの結果
//! ```
//!
//! # **判定と適用は分かれている**（ADR-0027 §決定7 / 作業単位 16e-b）
//!
//! **モデルの答えが、そのまま値になることはもう無い。**
//! 分解した採点は帳簿（`domain::drift_ledger`）へ入り、
//! **累積が費用に達したターンだけ**値が動く。
//!
//! ```text
//!   read_response      … 依頼の形ごとの分解器（純粋）。**採点・票・集計キーにそろえる**
//!   axis_counts        … 票の表から「0 でなかった回数」を数える（＝希少度の重み）
//!   record_votes       … **0 でない採点だけ**を票にする（記録専用モードでも走る）
//!   apply_scores       … 帳簿を回して、動いた軸を書く（**許可証が要る**）
//! ```
//!
//! **`all` で呼ぶのは `parse_drift_all_report` の側である**（`parse_drift_all` ではない）。
//! あちらは採点だけを返す薄い包みで、**引用記号を剥がした行数（16e-c）が落ちる。**
//!
//! # ⚠ **`drift_outcome=applied` は「値が動いた」ではない**
//!
//! `single` の `applied` は「**提案が許可リストを通り、票になった**」、
//! `saturated` は「**端の軸への提案だった**」である。
//! **値が動いたかどうかは `drift_steps` の欄**を見ること。
//! 16b〜16e-a の測定と綴りは同じだが、**意味が変わっている。**
//!
//! # 記録専用モード（ADR-0026 §決定1）
//!
//! `PERSONACORE_STAT_DRIFT=shadow` は**判定して票を残すが、値を動かさない。**
//!
//! **分岐そのものは在る**（`let write = match (&outcome, mode.applying())`）。
//! 型が消しているのは**分岐ではなく、分岐の中身を間違って書ける余地**である
//! ——`Store::apply_stat_drift` は `domain::DriftApply`（許可証）を引数に要求し、
//! その許可証は `StatDriftMode::applying` からしか手に入らない。
//! つまり**このモジュールの現状のコードには、記録専用モードから書き込みの文へ至る枝が
//! 存在しない。**
//!
//! > **⚠ 「型が守るから見張り番は要らない」と読まないこと**（16d のレビューで実測）。
//! > **`StatDriftMode::On.applying()` はどこからでも呼べる。**
//! > `mode.applying()` をそう書き換える変異は**コンパイルが通り**、
//! > `domain` の `only_the_applying_mode_hands_out_a_permit` は**緑のまま**だった。
//! > 赤くなったのは**振る舞いを見る2本**——
//! > `the_shadow_mode_never_touches_the_stored_values` と
//! > `tests/server_stat_drift.rs::the_shadow_mode_records_a_vote_through_the_real_bind`。
//! > **権威はそちらにある。**
//! >
//! > **⚠ 統合の1本は、2026-08-25 まで実際には権威になっていなかった**（作業単位 19 のレビュー）。
//! > **`shadow` の腕だけが既定の `K` で回っており、既定の `K` では `on` でも
//! > 1往復では1段も動かない**ので、**「値が動かない」はモードではなくつまみの結果**だった。
//! > **同じ変異で、統合のこの1本は緑を通っていた**（単体の2本だけが赤）。
//! > **両腕を同じ安い `K` に揃えて直した。** 実測: 直す前は緑・直した後は赤。
//!
//! **票の経路は3値で共通**（`record_vote`）。`shadow` のときだけ別の道を通る形にすると、
//! 測定で集めた票と `on` で集まる票が同じものだと言えなくなる。
//!
//! # なぜ抽出と同じ枠の中なのか（ADR-0024 §2）
//!
//! 抽出タスクは**同時に1本だけ**走る（`MemoryExtractor`）。
//! **変容をその中に置けば、本文生成の GPU を取り合う相手は増えない。**
//!
//! 帰結として、**抽出が `Busy` で飛んだターンは変容も飛ぶ。**
//! 変容は1ターン1段なので、飛んだターンは「動かなかった」だけである。
//!
//! **飛ぶのは `Busy` のときだけ。** 抽出が別の理由（依頼が長すぎる／記憶が読めない）で
//! 飛んだターンでも変容は走る——変容は**記憶を読まず**、会話も直前の1往復しか載せない。
//! **変容は自分の予算判定を持つ**（[`FALLBACK_MAX_DRIFT_PROMPT_TOKENS`]）。
//!
//! # 本文には一切影響しない
//!
//! 上流が落ちていても、崩れた出力を返しても、DB が書けなくても、
//! **起きるのは「そのターンは動かなかった」だけ**である。
//! 戻り値を持たず、呼び出し側は結果を見ない。
//!
//! # トランザクションを持たない
//!
//! 読み（`stat_allocs`）も書き（`apply_stat_drift`）も1文で完結するので、
//! **`LlmClient::complete` の `await` はどのトランザクションの中にも無い**
//! （CLAUDE.md 絶対制約）。**そもそも張らない形にしてある。**
//!
//! # LLM 出力の扱い
//!
//! サブLLM の出力は**外部からの信頼できない入力**（CLAUDE.md 絶対制約 / ADR-0024 §1）。
//! ここから先へ渡る経路は `parse::drift`（純粋）→ `domain::apply_drift`（許可リスト）→
//! `Store::apply_stat_drift`（プレースホルダ束縛）だけで、
//! **SQL・シェル・パスには渡らない。**
//! **ログにも生出力を載せない**——載せるのは定数ラベルと、
//! **保存済みの軸名**（管理 API から書かれた検証済みの値）と数だけ。
//!
//! # `character.rs` の全置換をここから呼ばない（ADR-0024 §7）
//!
//! 「全部読む → 1つ変える → 全部書く」は、その窓で他の軸の更新を巻き戻す。
//! **見張り番は `the_drift_path_never_reaches_for_the_wholesale_replacement`**
//! （このファイルと `store::drift` のソースを走査する）。
//! **走査対象のファイルにその関数名を綴らないこと**——綴ると走査が自分自身を落とす
//! （`store::memory::table_word` と同じ作法）。

use crate::domain::{
    DriftAxisLedger, DriftDelta, DriftDiscardReason, DriftForm, DriftLedger, DriftNote, DriftScore,
    DriftScoring, DriftTuning, DriftVote, StatAlloc, StatAxis, StatDriftMode, apply_drift,
};
use crate::llm::{ChatRequest, LlmClient};
use crate::parse::drift::{parse_drift, parse_drift_all_report};
// **打ち切りの語彙は抽出と共有する**（作業単位 B-1）。
// 言っているのは封筒のこと（「上流が最後まで書いたか」）で、
// **何を捨てるかは呼ぶ側が決める**——写しを作ると片方だけ古びる。
use crate::parse::extraction::Completeness;
use crate::prompt::drift::{DriftInput, build_drift_prompt, request_tokens};
use crate::store::{DriftWrite, Store, StoredDriftCharge};

use super::extract::{
    SubContext, TurnJob, character_vanished, record_vanished, response_completeness,
};

/// 変容の依頼プロンプト全体に許す上限（トークン）。**超えたら捨てる。切り詰めない。**
///
/// # なぜ抽出と同じ規則で、なぜ別の定数なのか
///
/// **規則は抽出と同じ「サブモデルの実コンテキストの半分」**
/// （[`SubContext`] の doc）。理由もそのまま通用する:
///
/// * 半分に切るのは、裏方が本文生成と**同じ GPU を取り合う**から。
/// * **抽出と変容は同時ではなく順に走る**（ADR-0024 §2 / 同じ1本の枠の中）ので、
///   **2つを足して半分にする必要は無い。** どの瞬間も上流に載っているのは一方だけである。
///   足して分け合う形にすると、**どちらの依頼も理由なく細くなる。**
///
/// **それでも定数は別に持つ。** 変容の依頼は削れない枠が抽出よりずっと小さく
/// （記憶を載せない / 会話は直前の1往復だけ）、**片方だけを動かして測りたくなる。**
/// 抽出の定数を直接使うと、**変容の実験が抽出の予算を巻き込む。**
///
/// **写しにはしない。** 数（8192）ではなく**式**を書いてあるので、
/// サブモデルが変わったときに直す場所は増えない。
///
/// # ⚠ **これは「引けなかったとき」の上限**（作業単位 B-7 / レビュー指摘3 / 2026-08-27）
///
/// **稼働中の上限は [`drift_prompt_limit`] が返す値**で、入力は
/// **その往復の窓**（[`TurnJob::sub_context`]）。抽出とまったく同じ器から来る。
///
/// **⚠ ここを定数のままにすると、反対側の入口が開いたままになる。**
/// サブが 6144 で載っている機械では:
///
/// | 裏方 | 上限（定数のままだったとき） | 何が起きるか |
/// |---|---|---|
/// | 抽出 | 3072（窓の半分） | 正しく捨てる |
/// | 変容 | **8192**（窓より大きい） | **超えた依頼がそのまま出る**（LM Studio では前方から黙って切り捨てられる / ADR-0013。llama は 400 `exceed_context_size_error` で断る / ADR-0052 M5。ただし、プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）） |
///
/// **しかも変容は本文に出ない**（ADR-0024 §9）ので、LM Studio で黙って切り捨てられても
/// **利用者にもログにも出ない。**（llama の 400 は `record_skip` の `warn` で残り、応答が
/// 短くなった回は丸ごと捨てて `drift_outcome=truncated` がログに出る。）
/// **「同じ危険に入口が2つあるとき、片方だけ塞ぐと反対側から入られる」**（CLAUDE.md / 16e-b）。
///
/// # 実機でここに当たることは、まず無い
///
/// 変容の依頼の削れない枠は**軸の本数ぶんの短い行と指示だけ**で、
/// 会話は直前の1往復。**当たるとしたら利用者の発言が極端に長い回**である。
/// 破棄率のログに `too_long` が並ぶようなら、この数が現実に合っていない合図になる。
#[cfg(test)]
const FALLBACK_MAX_DRIFT_PROMPT_TOKENS: u32 = drift_prompt_limit(SubContext::FALLBACK);

/// **変容の依頼に許す上限**（トークン）。**規則は抽出と同じ「窓の半分」。**
///
/// **抽出の [`SubContext::prompt_limit`] をそのまま使う。**
/// 別の式を書くと、**同じサブモデルの同じ窓に対して2つの上限**ができる。
const fn drift_prompt_limit(sub_context: SubContext) -> u32 {
    sub_context.prompt_limit()
}

/// **変容の応答に許す上限の頭打ち**（トークン）。**窓の残り**。
///
/// 依頼（[`drift_prompt_limit`]）と足して**サブモデルの実コンテキストに収まる**
/// ——超えると、LM Studio では前方から黙って切り捨てられ、最初に消えるのは指示の側になる
/// （ADR-0013）。llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。ただし、
/// プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
const fn drift_response_cap(sub_context: SubContext) -> u32 {
    sub_context
        .tokens()
        .saturating_sub(drift_prompt_limit(sub_context))
}

/// **応答の1行に見込むトークン数**（ADR-0027 §12 / 作業単位 B-1）。
///
/// # 式
///
/// ```text
/// 1行 = 「- 」 ＋ 軸名 ＋ 「 +1」 ＋ 改行
///     = 2 ＋ StatAxis::MAX_CHARS × UTF-8 の最大バイト長 ＋ 3 ＋ 1
/// ```
///
/// **見積もりは UTF-8 のバイト数**（`prompt::estimate` の doc——バイトレベル BPE の
/// 証明できる上界）。軸名は**文字数**で上限が決まっている（`StatAxis::MAX_CHARS` = 20）ので、
/// **1文字が最大4バイト**のアストラル面の文字で埋められた最悪の軸名を見込む。
///
/// **数を直書きしない。** `StatAxis::MAX_CHARS` が動いたら一緒に動く。
/// 見張り番は `the_response_limit_covers_the_worst_case_scorecard`
/// ——**この定数から期待値を作らず、実際に最悪の応答を組み立てて測る。**
///
/// # ⚠ **1軸を尋ねる形も、これで測る**（作業単位 B-1 で直した）
///
/// **かつて `single` には別の定数（`DRIFT_RESPONSE_TOKENS`）があり、この式を下回っていた**
/// ——`all` と `single` は同じ1行の答えを受け取るのに、送っていた量が違った。
/// 収まらない回は行末の符号が落ちて `malformed` になり、
/// **読んだ人はプロンプトを直しに行く。打ち手が正反対。**
///
/// **答えの形は同じ1行なので、見込む量も1つでよい。**
///
/// **実際に何トークンだったのかは
/// [ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md) §打ち切り 1 の表**
/// ——**散文の数はあそこ1箇所にしか置かない**（CLAUDE.md「同じ発見を2箇所に書かない。
/// 片方を直しても、もう片方が黙って古びる」）。
const DRIFT_LINE_TOKENS: u32 = 2 + (StatAxis::MAX_CHARS as u32) * 4 + 3 + 1;

/// **応答に許すトークン数の上限**（全軸採点で軸が多いとき）。
///
/// 依頼（[`FALLBACK_MAX_DRIFT_PROMPT_TOKENS`]）と足して**サブモデルの実コンテキストに収める。**
/// 超えると、LM Studio では**前方から黙って切り捨てられる**ので、最初に消えるのは指示の側になる
/// （ADR-0013）。llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。ただし、
/// プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
///
/// # **軸の件数には上限が無い**（`StatAxis` の doc）
///
/// 軸を **96 本以上**持つキャラクターでは、この上限に当たって**末尾が切れる**
/// （95 本ちょうどまでは最悪の応答が収まり、96 本で溢れる）。
///
/// **この境目を押さえている見張り番は
/// [`the_axis_count_where_the_worst_case_scorecard_stops_fitting`]**
/// ——**期待値を [`DRIFT_LINE_TOKENS`] から作らず**、
/// `StatAxis::MAX_CHARS` 文字の軸名で**最悪の応答を実際に組み立てて測る。**
/// 掛け算の途中経過をここに書かないのは、**`StatAxis::MAX_CHARS` が動いた日に
/// 静かに嘘になる**からである（散文の数は
/// [ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md) §打ち切り 1 に寄せた）。
///
/// 切れた回は**打ち切りの申告つきで返る**ので `truncated` として丸ごと捨てられ、
/// **破棄率のログに現れる**——黙って「動いていない」に化けることは無い
/// （作業単位 B-1 より前は `missing_axis` に化けていた）。
///
/// # **下からも押さえてある**（レビュー指摘A / 16e-a）
///
/// **この値を小さくする変異は、上の「実コンテキストに収まる」だけでは止まらない**
/// ——半分（4096）にする変異が**全緑を通り抜けた。**
/// 実害は静かで重い: 軸を 48 本持つキャラクターの応答が毎回末尾で切れ、
/// **16e-b で分解を繋いだ後は毎ターン捨てられ続ける**（値も票も一切動かない）。
///
/// 下界の見張り番は `the_response_limit_covers_a_request_that_fills_the_prompt_budget`
/// ——**依頼の予算を最長の軸名で埋め切ったときの本数**を実際に組み立てて数え、
/// **その本数ぶんの最悪の応答が収まること**を要求する（定数から期待値を作らない）。
/// **⚠ これも「引けなかったとき」の値**（作業単位 B-7）。
/// 稼働中の頭打ちは [`drift_response_cap`] が窓から出す。
#[cfg(test)]
const FALLBACK_MAX_DRIFT_RESPONSE_TOKENS: u32 = drift_response_cap(SubContext::FALLBACK);

/// **依頼の形と軸の本数から、応答の上限を出す。** 純粋関数。
///
/// | 形 | 見込む行数 | 上限 |
/// |---|---|---|
/// | [`DriftForm::Single`] | **1行**（軸が何本でも / ADR-0024 §3） | [`DRIFT_LINE_TOKENS`] |
/// | [`DriftForm::All`] | 軸の本数（ADR-0027 §12） | 1行ぶん × 本数 |
///
/// どちらも [`FALLBACK_MAX_DRIFT_RESPONSE_TOKENS`] で頭打ちにするので、
/// **依頼と足して実コンテキストを超えることは構造上ありえない。**
///
/// # なぜ `None` を送らないのか（**上限の大きさの理由ではない**）
///
/// `EXTRACTION_RESPONSE_TOKENS` とまったく同じ理由:
/// **形式を無視して喋り続けるモデルが、本文生成の GPU を握り続ける**のを防ぐため。
/// 上限が無いと、崩れた回ほど長く GPU を占める。
///
/// **ここで言っているのは「上限を送るかどうか」だけである。**
/// **どの大きさにするかは次の節**（＝答えの形）で、
/// **行列の余裕からは出ていない**（下の ⚠）。
///
/// # なぜ「最悪の答えちょうど」で足りるのか
///
/// **それより長い応答からは、採れる情報が1つも増えない。**
/// `single` は 2 行以上を丸ごと捨て、`all` は軸の本数と合わない採点表を丸ごと捨てる
/// （`parse::drift`）。**長く受け取っても捨てる量が増えるだけ**である。
///
/// # ⚠ **上端は「行列の余裕」から出ていない**（作業単位 B-1）
///
/// 抽出側には行列の実測がある（`QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS`）が、
/// **変容側にはまだ無い。無い数を作らない**ので、上端は上の「答えの形」から素直に出す。
///
/// **⚠ それでも壊れはしない**（作業単位 B-6）。行列が伸びて中継が黙っても、
/// **中継は打ち切らず `warn` を出すだけ**になった
/// （[ADR-0035](../../../docs/adr/0035-the-watchdog-reports-the-human-decides.md) §決定1）。
/// **起きるのは「次のターンの本文が遅れる」であって「ターンが死ぬ」ではない。**
///
/// **足りずに切れた回**は `truncated` として**丸ごと捨てられる**
/// （ADR-0027 §打ち切り）。**その欄が並び始めたら、この見込みが現実に合っていない合図。**
///
/// **本文の数えた計画も読む**（裏方の1要求分の差し引き / `super::counted_plan::background_request`）。
pub(in crate::server) fn drift_response_tokens(
    form: DriftForm,
    axes: usize,
    sub_context: SubContext,
) -> u32 {
    // **見込む行数。** ここだけが形で分かれる——1行に見込む量は共通である。
    let lines = match form {
        DriftForm::Single => 1,
        DriftForm::All => u32::try_from(axes).unwrap_or(u32::MAX),
    };
    DRIFT_LINE_TOKENS
        .saturating_mul(lines)
        .min(drift_response_cap(sub_context))
}

/// **重みを数えるために読み戻す票の件数**
/// （[ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md) §決定1）。
///
/// # なぜ上限が要るのか
///
/// **票は消えない**（追記のみ / ADR-0026 §決定3）。全件読むと、
/// **読む量が会話の長さに比例して増える。** 変容は毎ターン走る裏方なので、
/// ここが効いてくる。
///
/// # なぜこの大きさなのか
///
/// **全軸採点は1ターンに軸の本数ぶんの票を立てる。**
/// 軸5本なら 1024 件で **200 ターンぶん**、1本なら 1024 ターンぶんが窓に入る。
/// ADR-0027 の測定は 72 ターン（＝ 360 票）なので、**測定は丸ごと窓に収まる。**
///
/// # ⚠ **窓を超えると重みが下げ止まる**
///
/// 「その軸のそれまでの `0` でなかった回数」は**窓の中の件数**なので、
/// 重みは `1/1024` より軽くならない。
/// **これは ADR §決定1 の「代償: 早い票が重い」を緩める向き**に働くが、
/// **ADR が測ったのは窓の無い当て直し**である。長い会話では規則が少しずれる。
const DRIFT_WEIGHT_VOTES: u32 = 1024;

/// 変容を1回判定したことの記録に使う文言。**「走った」側。**
const RAN_MESSAGE: &str = "性格の変容を判定しました";

/// 変容を**行わなかった**ことの記録に使う文言。**「走らなかった」側。**
///
/// **「提案なし」と同じ行にしないこと。** 「提案が出なかった」は正常な結果で、
/// ADR-0024 の「危険 3. 提案が出ない」を読む側の値そのものである。
/// 走らなかった回を混ぜると、**提案率の分母が壊れる。**
const SKIPPED_MESSAGE: &str = "性格の変容を判定しませんでした";

/// 上流が**停止理由を申告しなかった**ことの記録に使う文言（作業単位 B-1）。
///
/// # **抽出の文言と別にしてある**（写しではなく、別のイベント）
///
/// 変容と抽出は**同じターンで順に走る**ので、同じ文言にすると
/// **どちらのサブLLM 呼び出しに停止理由が無かったのかがログから読めない。**
/// 見張り番は `the_two_missing_finish_reason_messages_never_contain_each_other`。
///
/// **「判定した」「判定しなかった」のどちらとも別のイベント**にしてある
/// （[`record_missing_finish_reason`] の doc）。
const NO_FINISH_REASON_MESSAGE: &str = "変容の応答に停止理由が付いていません";

/// 軸の欄に出す「該当なし」。**提案が無い・破棄された回。**
const NO_AXIS: &str = "-";

/// 向きの欄に出す「該当なし」。**向きが決まらなかった回**（破棄・提案なし）。
const NO_DELTA: &str = "-";

/// 値の欄に出す「該当なし」。
///
/// # ⚠ **`0` は番兵に使えない**（ADR-0027 §決定4 / §決定6）
///
/// **値 0 の軸も変容の対象になった**ので、`before = 0`（＝ゼロから育て始めた回）が
/// 現実に起きる。番兵を `0` のままにすると、
/// **ゼロから 1 へ上がった回の `before` が「該当なし」と同じ綴りでログに出る**
/// ——ADR-0027 §11 が「いちばん実用に効く」と書いた配置が、**測れなくなる。**
///
/// 尺度の上限は `StatScale::MAX`（= 100）で、
/// **配分値がそれを超えることはない**（`StatScale::value` が唯一のコンストラクタ）。
/// 見張り番は `the_absent_value_marker_can_never_be_a_real_value`。
const NO_VALUE: u32 = u32::MAX;

/// 書き込みまで行かなかった回の印（提案なし・破棄・飽和）。
const NO_WRITE: &str = "-";

/// **書き込みそのものが失敗した回の印。**
///
/// **`DriftWrite::NotMoved`（＝「動かなかった」）と同じ値にしない。**
/// 前者は設計どおりの結果（競合・尺度が下げられた）で、こちらは**故障**である。
/// 同じ値にすると、**DB が書けない回が「提案が適用されなかっただけ」に見える。**
const WRITE_FAILED: &str = "failed";

/// **票にならなかった回の印**（提案なし・破棄）。
///
/// 「票を残さなかった」と「票を残そうとして失敗した」を分けるのは、
/// [`NO_WRITE`] と [`WRITE_FAILED`] を分けているのと同じ理由。
const NO_VOTE: &str = "-";

/// **票を1件残した回の印**（ADR-0026 §決定3）。
const VOTE_RECORDED: &str = "recorded";

/// **票を残せなかった回の印。故障。**
///
/// **記録専用モードではこれが唯一の成果物**なので、失敗が
/// 「そもそも票にならなかった」に紛れると、**測定の分母が静かに減る。**
const VOTE_FAILED: &str = "failed_to_record";

/// 捨てた書き込みの種類（`vanished_write` 欄 / ADR-0049 §決定3）。**変容の票。**
const VANISHED_DRIFT_VOTE: &str = "drift_vote";

/// 同上。**変容の累積。**
const VANISHED_DRIFT_CHARGE: &str = "drift_charge";

/// 同上。**性格ステータスの値（1軸1段）。**
const VANISHED_STAT_VALUE: &str = "stat_value";

/// **ログに載せる生出力の上限（文字数）。**
///
/// # なぜ生出力をログに残すのか（腕F の測定で見つかった穴）
///
/// 絶対制約「`message.raw_content` を必ず保持する」の理由は
/// **「パース失敗時の唯一の調査手段」**だが、**その理由が変容の経路には効いていなかった**
/// ——腕F では破棄が 4/40 出たのに、**何を書かれて捨てたのかが残っていなかった。**
///
/// そして [ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md) §12 の問い
/// （**`0` は実際に出るか**）は、**破棄されなかった回の中身**を数える問いである。
/// **成功した回も載せる**のはそのため——破棄時だけでは答えられない。
///
/// # バイトではなく**文字数**で切る
///
/// バイト添字で切ると多バイト文字の途中で割れる（`panic!` の温床）。
/// `chars().take(..)` なら**どんな入力でも落ちない**（絶対制約）。
///
/// # 上限が要る理由
///
/// 全軸採点の応答上限は軸の本数ぶん伸びる（[`drift_response_tokens`]）ので、
/// **そのままログへ流すと1イベントが数千文字になる。**
/// 切り詰めたら [`DRIFT_RAW_TRUNCATED`] を付けて、
/// **「全部載っている」と誤読させない。**
const DRIFT_RAW_LOG_CHARS: usize = 512;

/// **切り詰めた印。** これが付いている行は**生出力の途中まで**しか載っていない。
///
/// **無印と区別が付く綴りにする。** 付け忘れると、
/// 途中で切れた応答が「モデルがそこまでしか書かなかった」ように読める。
const DRIFT_RAW_TRUNCATED: &str = "…(truncated)";

/// 生出力をログに載せられる形にする。**純粋関数。panic しない。**
///
/// **改行は畳まない。** 呼び出し側が `Debug`（`?`）で書くので、
/// `\n` はエスケープされて**1イベント1行が保たれる**
/// （`record_run` の doc / 見張り番は `a_multi_line_response_stays_on_one_event_line`）。
fn loggable_raw(raw: &str) -> String {
    let mut text: String = raw.chars().take(DRIFT_RAW_LOG_CHARS).collect();
    if raw.chars().nth(DRIFT_RAW_LOG_CHARS).is_some() {
        text.push_str(DRIFT_RAW_TRUNCATED);
    }
    text
}

/// 変容を走らせなかった理由。**集計キーは値ごとの定数**（入力を反射しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriftSkipReason {
    /// **性格ステータスを読めなかった。**
    ///
    /// `stat_allocs` は1行壊れると丸ごと失敗する（既存の仕様）。
    /// 許可リストそのものが手に入らないので、**その回は走らせない**——
    /// 軸ゼロの依頼を投げても、モデルは名前を捏造するだけである。
    StatsUnreadable,
    /// **軸が1本も定義されていない**（ADR-0027 §決定6）。
    ///
    /// **そもそも性格ステータスを持たないキャラクターだけ**がここに来る。
    /// **全軸が値 0 でも走る**——ゼロから育てるのがこの機構の想定される配置である
    /// （ADR-0027 §11）。`build_drift_prompt` が `None` を返すので、**上流を呼ばない**
    /// ——答えの決まっている問いに GPU を使わない。
    NoAxes,
    /// 依頼が長すぎる。**切り詰めない**（[`FALLBACK_MAX_DRIFT_PROMPT_TOKENS`]）。
    TooLong,
    /// **上流を呼べなかった。**
    ///
    /// **抽出側はこれを数えていない**（`warn` を出すだけ）が、変容では数える。
    /// ADR-0024 の危険3つは**すべて率で読む**ので、
    /// 「呼べなかった回」が分母から漏れると提案率が実際より高く出る。
    UpstreamFailed,
}

impl DriftSkipReason {
    /// 集計キー。
    const fn label(self) -> &'static str {
        match self {
            Self::StatsUnreadable => "stats_unreadable",
            Self::NoAxes => "no_axes",
            Self::TooLong => "request_too_long",
            Self::UpstreamFailed => "upstream_failed",
        }
    }
}

/// **変容の本体**（ADR-0024）。**抽出タスクの中で、抽出の直後に走る。**
///
/// `super::extract::run` が**効く走らせ方（[`TurnJob::drift_mode`]）が off でないときだけ**呼ぶ。
/// off のときにここへ入って結果を捨てる形にしないのは、**off でも GPU を使う**から。
///
/// # 走らせ方は引数で受けない（作業単位 A-8）
///
/// **`Extraction::start`（応答を保存した後）で決めた [`TurnJob::drift_mode`] だけを見る**（ADR-0045 §決定2）。
/// 引数で別に受けると、「走らせるか」を決めた値と「動かしてよいか」を決める値が
/// 別々の出どころを持てる——キャラクターが `shadow` なのに起動時の既定 `on` の許可証で
/// 値が動く、が書けてしまう。
///
/// # 許可リストは**保存済みの軸**（ADR-0024 §1）
///
/// `Store::stat_allocs` が返した軸だけが動かせる。
/// **依頼に載せないことは、それだけでは §4 を守らない**（モデルは載っていない語も書く）ので、
/// 受け取る側でも `domain::apply_drift` が落とす。**見張り番は2本立っている。**
pub(in crate::server) async fn run_drift(
    store: &Store,
    llm: &dyn LlmClient,
    job: &TurnJob,
    form: DriftForm,
    // **書き方の注記**（ADR-0029 §訂正 / `PERSONACORE_DRIFT_NOTE`）。素通しで依頼へ渡す。
    note: DriftNote,
    tuning: DriftTuning,
) {
    // **その往復に効く走らせ方**（ADR-0045 §決定2）。判定・票・許可証・記録の全部がこれを見る。
    let mode = job.drift_mode.mode();
    // **保存済みの軸**（＝許可リスト）。**上流を呼ぶ前**に、トランザクションを跨がずに読む。
    let allocs = match store.stat_allocs(job.character_id).await {
        Ok(allocs) => allocs,
        Err(err) => {
            record_skip(
                DriftSkipReason::StatsUnreadable,
                job,
                None,
                Some(&err),
                mode,
            );
            return;
        }
    };

    // **載る軸が1本も無ければ依頼そのものを作らない**（`build_drift_prompt` が `Option`）。
    let Some(prompt) = build_drift_prompt(&DriftInput {
        display_name: &job.display_name,
        stats: &allocs,
        // **依頼の形**（ADR-0027 §12 / `PERSONACORE_DRIFT_FORM`）。
        form,
        // **書き方の注記**（ADR-0029 §訂正 / `PERSONACORE_DRIFT_NOTE`）。
        note,
        // **キャラクターの尺度**（値の側が覚えている尺度ではない / `TurnJob::stat_scale`）。
        scale: job.stat_scale,
        user_message: &job.user_message,
        character_message: &job.character_message,
    }) else {
        record_skip(DriftSkipReason::NoAxes, job, None, None, mode);
        return;
    };
    let axes = prompt.included_axes();
    let messages = prompt.into_messages();

    // **長すぎたら丸ごと捨てる。切り詰めない。**
    // 数え方は `prompt` 側の1関数だけ（抽出と共有 / `prompt::drift::request_tokens`）。
    let tokens = request_tokens(&messages);
    if tokens > drift_prompt_limit(job.sub_context) {
        record_skip(DriftSkipReason::TooLong, job, Some(tokens), None, mode);
        return;
    }

    // **温度は明示する**（ADR-0038 §決定2 / 作業単位 A-7d。抽出と同じ値）。**思考させない**（ADR-0059 §決定1）。
    // **値の来歴と、抽出と同じ値であることの意味は `llm::SUB_LLM_TEMPERATURE` の doc**（裏方の要求の口 /
    // `llm::ChatRequest::background`）。
    // **上限は形と軸の本数から出す**（全軸採点は N 行返る / ADR-0027 §12）。
    let request =
        ChatRequest::background(messages, drift_response_tokens(form, axes, job.sub_context));

    // **トランザクションの外で通信する**（CLAUDE.md 絶対制約）。
    // このモジュールはそもそもトランザクションを張らない。
    let completion = match llm.complete(&request).await {
        Ok(completion) => completion,
        Err(err) => {
            record_skip(
                DriftSkipReason::UpstreamFailed,
                job,
                Some(tokens),
                Some(&err),
                mode,
            );
            return;
        }
    };

    // **申告が無いことは、それ自体を残す**（作業単位 B-1 / 24 と同じ作法）。
    // **捨てはしない**（[`response_completeness`] は「切れていない」に倒す）。
    if completion.finish_reason.is_none() {
        record_missing_finish_reason(job);
    }

    // **崩れていても panic しない**（絶対制約）。捨てた分は理由つきで数に残る。
    //
    // **依頼の形で分解器が違う**（ADR-0027 §12）。どちらも
    // 「集計キー・票・採点表」という同じ形にそろえて返る（[`DriftReading`]）。
    //
    // **打ち切られた回は丸ごと捨てる**（ADR-0027 §打ち切り）。
    // 翻訳（停止理由 → 打ち切られたか）は**抽出と同じ関数**を通る。
    let reading = read_response(
        form,
        &allocs,
        &completion.content,
        response_completeness(completion.finish_reason),
    );

    // **票を先に残す**（ADR-0026 §決定3）。**記録専用モードでもここは走る。**
    //
    // 票になるのは「許可リストを通った、`0` でない採点」だけで、
    // その選別は `domain` 側（`DriftOutcome::vote` / `DriftScore::vote`）に閉じている。
    // **飽和も票になる**——記録の時点で落とすと「上げようとし続けている」が消える。
    //
    // **ここは帳簿より先。** 帳簿は「その軸のそれまでの `0` でなかった回数」を
    // 票の表から数えるので、**この回の票を数に入れてはいけない**
    // ——重みは「その票を含めて数える」（ADR-0027 §決定1）が、
    // その1つは帳簿が自分で足す。
    let counts = axis_counts(store, job, &allocs).await;
    let recorded = record_votes(store, job, &reading.votes).await;

    // **値を動かすのは帳簿**（ADR-0027 §決定1〜3・§決定7・§9）。
    //
    // **`mode.applying()` が `None` なら、書き込みの文へ至る枝そのものが無い**
    // （`Store::apply_stat_drift` / `Store::save_drift_charges` は許可証を引数に要求する）。
    let applied = apply_scores(
        store,
        job,
        &allocs,
        counts,
        reading.scores.as_deref(),
        form,
        tuning,
        mode,
    )
    .await;

    // **生出力もそのまま渡す**（上限つきで載る / `loggable_raw`）。
    record_run(
        job,
        &reading,
        &applied,
        axes,
        recorded,
        mode,
        &completion.content,
    );
}

/// **応答から読み取ったもの。** 依頼の形が違っても、ここから先は同じ形で扱う。
///
/// # なぜ1つの形にそろえるのか
///
/// そろえないと、**帳簿・票の記録・ログの3箇所が形ごとに分岐する。**
/// 分岐が3つに散ると、片方の形にだけ効く変異が
/// 「もう片方の形のテストが緑だから大丈夫」に見える
/// （CLAUDE.md「片方だけのテストにしない」）。
struct DriftReading {
    /// ログの `drift_outcome` に出る綴り。
    ///
    /// | 形 | 値 |
    /// |---|---|
    /// | `single` | `DriftOutcome::label`（`applied` / `saturated` / `nothing` / 破棄の理由） |
    /// | `all` | `DriftScoring::label`（`scored` / 破棄の理由） |
    ///
    /// # ⚠ **`applied` は「値が動いた」ではない**（16e-b で意味が変わった）
    ///
    /// ADR-0027 §決定7 で**判定と適用が分かれた。**
    /// `single` の `applied` は「**提案が許可リストを通り、票になった**」で、
    /// `saturated` は「**端の軸への提案だった**」である。
    /// **値が動いたかどうかは `drift_steps` の欄**を見ること。
    label: &'static str,
    /// 記録する票。**`single` は 0〜1 件、`all` は 0〜N 件。**
    votes: Vec<DriftVote>,
    /// 帳簿へ渡す1ターンぶんの採点。**並びは `allocs` と同じ。**
    ///
    /// **読めなかった回は `None`。** 帳簿を回さない——
    /// 破棄された回に「全軸が `0` と採点された」ことにすると、
    /// **形式の崩れが下向きの供給源になる**（ADR-0027 §12 が名指しする混同）。
    scores: Option<Vec<Option<DriftDelta>>>,
    /// ログの `drift_axis` に出す軸（`single` で提案が通った回だけ `Some`）。
    axis: Option<StatAxis>,
    /// ログの `drift_delta` に出す向き（同上）。
    delta: Option<DriftDelta>,
    /// **行頭から剥がした引用記号の数**（作業単位 16e-c / `all` だけ）。
    quotes_stripped: usize,
}

impl DriftReading {
    /// **打ち切られた回**（ADR-0027 §打ち切り / 作業単位 B-1）。
    ///
    /// **読めた中身は1つも使わない。** 票も採点も持たない値をここで**作り直す**
    /// ——読めた値を書き換える形（`..reading` で埋める形）にしないのは、
    /// **後から効果を運ぶ欄が増えたときに、コンパイラが判断を求めるようにするため。**
    ///
    /// **引用記号の数だけは持ち越す。** あれは「捨てたかどうか」とは無関係に
    /// 「モデルが依頼の記号を答えへ持ち込む頻度」を測る指標である
    /// （`parse::drift::DriftScoringReport::quotes_stripped` の doc）。
    const fn truncated(quotes_stripped: usize) -> Self {
        Self {
            label: DriftDiscardReason::Truncated.label(),
            votes: Vec::new(),
            scores: None,
            axis: None,
            delta: None,
            quotes_stripped,
        }
    }
}

/// **応答を分解して [`DriftReading`] にする。** DB にも上流にも触らない。
///
/// # `all` で呼ぶのは `parse_drift_all_report` の方（`parse_drift_all` ではない）
///
/// あちらは薄い包みで、**剥がした引用記号の数が落ちる**——
/// ADR-0027 が「31% の破棄」を測った指標そのものなので、本番の経路から落とさない。
///
/// # **打ち切られた回は、読めた中身によらず丸ごと捨てる**（ADR-0027 §打ち切り）
///
/// **抽出（最後の1行だけ捨てる）とは意図的に違う。**
/// 1軸を尋ねる形は**2行以上を丸ごと捨てる**決まりなので、
/// **打ち切りが2行目を消した回は「1行の正当な提案」に化ける**
/// ——捨てるはずの回が値を動かし、**ログは正常な `applied` に見える。**
/// **何行あったのか分からない以上、残った1行を信じる根拠が無い。**
///
/// **分解そのものは通す。** 引用記号の数（16e-c の指標）は
/// **採否に関係なく数える**決まりなので、ここで落とさない。
fn read_response(
    form: DriftForm,
    allocs: &[StatAlloc],
    raw: &str,
    completeness: Completeness,
) -> DriftReading {
    let reading = read_whole_response(form, allocs, raw);
    match completeness {
        Completeness::Truncated => DriftReading::truncated(reading.quotes_stripped),
        Completeness::Complete => reading,
    }
}

/// **最後まで書かれた応答として分解する。** [`read_response`] の中身。
fn read_whole_response(form: DriftForm, allocs: &[StatAlloc], raw: &str) -> DriftReading {
    match form {
        DriftForm::Single => {
            let outcome = apply_drift(allocs, parse_drift(raw));
            let label = outcome.label();
            let vote = outcome.vote();
            // **選ばれた軸だけが `Some`。残りは「選ばれなかった軸」**（ADR-0027 §決定2）。
            let scores = vote.as_ref().map(|vote| {
                allocs
                    .iter()
                    .map(|alloc| (alloc.axis == *vote.axis()).then(|| vote.delta()))
                    .collect()
            });
            DriftReading {
                label,
                axis: vote.as_ref().map(|vote| vote.axis().clone()),
                delta: vote.as_ref().map(DriftVote::delta),
                votes: vote.into_iter().collect(),
                scores,
                quotes_stripped: 0,
            }
        }
        DriftForm::All => {
            // **依頼に載せた軸の綴りと並び**（`prompt::drift::stat_lines` と同じ順）。
            let axes: Vec<StatAxis> = allocs.iter().map(|alloc| alloc.axis.clone()).collect();
            let report = parse_drift_all_report(raw, &axes);
            let quotes_stripped = report.quotes_stripped();
            let scoring = report.into_scoring();
            let label = scoring.label();
            let scores = match &scoring {
                DriftScoring::Scored(scores) => aligned_scores(allocs, scores),
                DriftScoring::Discarded(_) => None,
            };
            // **票も同じ門を通す**（レビュー指摘4 / 16e-b）。
            //
            // 以前はここが `scores.len() == allocs.len()` しか見ておらず、
            // **帳簿へ渡す採点（`aligned_scores`）より門が弱かった。**
            // いまは到達しない差だが、**2つの門が同じ前提に寄りかかっている**ことに
            // 気付ける形ではなかった——**強い方1つに寄せる。**
            let votes = match (&scoring, scores.is_some()) {
                (DriftScoring::Scored(scores), true) => {
                    scores.iter().filter_map(DriftScore::vote).collect()
                }
                // **並びが合わない採点表は丸ごと使わない。**
                _ => Vec::new(),
            };
            DriftReading {
                label,
                votes,
                scores,
                // **全軸採点に「提案された1軸」は無い。**
                axis: None,
                delta: None,
                quotes_stripped,
            }
        }
    }
}

/// **採点表を、保存済みの軸の並びに突き合わせる。**
///
/// `parse::drift` は依頼に載せた順で返すので、通常はそのまま一致する。
/// **それでも位置ごとに軸名を照合する**——
/// 一致しなければ `None`（＝帳簿を回さない）。
///
/// # なぜ「たぶん合っている」で通さないのか
///
/// ずれた採点表をそのまま帳簿へ渡すと、**軸 A の採点が軸 B に入る。**
/// 値も件数も整合したまま**中身だけが入れ替わる**ので、
/// ログにも票にも現れない（CLAUDE.md「抜けるのはいつも帰属」）。
/// 見張り番は `a_scorecard_whose_axes_do_not_line_up_is_not_applied`。
fn aligned_scores(allocs: &[StatAlloc], scores: &[DriftScore]) -> Option<Vec<Option<DriftDelta>>> {
    if scores.len() != allocs.len() {
        return None;
    }
    let mut aligned = Vec::with_capacity(allocs.len());
    for (alloc, score) in allocs.iter().zip(scores) {
        if alloc.axis != *score.axis() {
            return None;
        }
        aligned.push(score.delta());
    }
    Some(aligned)
}

/// **票を1件残す**（ADR-0026 §決定3）。**残したかどうかの印**を返す。
///
/// # 値を動かす経路とは独立している
///
/// ここは `Store::record_drift_vote`（追記のみ）しか呼ばず、**許可証を持たない。**
/// したがって**記録専用モードでも `on` でも、まったく同じ経路を通る**
/// ——「shadow のときだけ別の道を通る」形にすると、
/// 測定で集めた票と、`on` で集まる票が同じものだと言えなくなる。
///
/// # 書けなくても本文には影響しない
///
/// 応答はとうに返っている。**故障は `error` で別に残す**
/// （`DriftWrite::NotMoved` と `WRITE_FAILED` を分けているのと同じ判断）。
async fn record_votes(store: &Store, job: &TurnJob, votes: &[DriftVote]) -> &'static str {
    if votes.is_empty() {
        return NO_VOTE;
    }

    // **1件でも書けなければ「残せなかった」側に倒す。**
    // 部分的に残った票は**そのまま重みになる**（ADR-0027 §決定1）ので、
    // 「一部だけ成功」を成功として数えると、**次のターン以降の重みが静かにずれる。**
    let mut all_recorded = true;
    for vote in votes {
        if let Err(err) = store
            .record_drift_vote(job.character_id, job.session_id, vote)
            .await
        {
            // **キャラクターが完全に消された回は故障ではない**（ADR-0049 §決定3）。
            // 「残せなかった」には数える（票は残っていない）が、`error` にはしない。
            if character_vanished(&err) {
                record_vanished(job, VANISHED_DRIFT_VOTE, &err);
            } else {
                tracing::error!(
                    error = %err,
                    detail = ?err,
                    session_id = job.session_id.get(),
                    "性格の変容の票を記録できませんでした"
                );
            }
            all_recorded = false;
        }
    }
    if all_recorded {
        VOTE_RECORDED
    } else {
        VOTE_FAILED
    }
}

/// **軸ごとの「`0` でなかった回数」を票の表から数える**（ADR-0027 §決定1 / §12 の危険3）。
///
/// 票は**非ゼロの採点にだけ立つ**ので、**票の件数がそのまま回数**である。
///
/// # 読めなかったら全部 `0` として進む
///
/// ここが失敗しても**変容そのものは止めない**——重みが「初回の票」として振る舞う
/// （＝いつもより重い）だけで、**値が壊れるわけではない。**
/// 止めると、票の表が1行壊れただけでそのキャラクターの変容が永久に凍る。
///
/// # ⚠ **数えるのは直近 [`DRIFT_WEIGHT_VOTES`] 件まで**
///
/// 全件読むと、票は消えないので**読む量が会話の長さに比例して増える。**
/// 窓を超えた古い票は数から落ちるので、**重みは 1/窓 より軽くならない。**
/// ADR-0027 §決定1 の「代償: 早い票が重い」は、この窓のぶんだけ緩和される。
async fn axis_counts(store: &Store, job: &TurnJob, allocs: &[StatAlloc]) -> Vec<u32> {
    let votes = match store
        .recent_drift_votes(job.character_id, DRIFT_WEIGHT_VOTES)
        .await
    {
        Ok(votes) => votes,
        Err(err) => {
            tracing::warn!(
                error = %err,
                detail = ?err,
                session_id = job.session_id.get(),
                "性格の変容の票を読めませんでした（重みを初回として進めます）"
            );
            Vec::new()
        }
    };

    allocs
        .iter()
        .map(|alloc| {
            let count = votes
                .iter()
                .filter(|stored| stored.vote().axis() == &alloc.axis)
                .count();
            u32::try_from(count).unwrap_or(u32::MAX)
        })
        .collect()
}

/// **帳簿を回して、動いた軸を書き込む**（ADR-0027 §決定1〜3・§決定7・§9）。
///
/// # 記録専用モードでは**何もしない**（ADR-0026 §決定1）
///
/// 累積すら書かない。溜めておくと、`on` に切り替えた瞬間に
/// **溜まっていた分がまとめて放出される**——
/// 記録専用モードの存在理由（値が動かないので決定性が保たれる）を、
/// **切り替えた直後だけ荒れる**という形で裏から壊す。
///
/// # トランザクションの中で外部通信を `await` しない（CLAUDE.md 絶対制約）
///
/// 上流との往復は**この関数を呼ぶ前に終わっている。**
/// ここで張るトランザクションは `Store::save_drift_charges` の中の1つだけで、
/// 中身は同じ表への `INSERT` である。
#[allow(clippy::too_many_arguments)]
async fn apply_scores(
    store: &Store,
    job: &TurnJob,
    allocs: &[StatAlloc],
    counts: Vec<u32>,
    scores: Option<&[Option<DriftDelta>]>,
    form: DriftForm,
    tuning: DriftTuning,
    mode: StatDriftMode,
) -> LedgerReport {
    // **採点が読めなかった回は帳簿を回さない**（[`DriftReading::scores`] の doc）。
    let Some(scores) = scores else {
        return LedgerReport::untouched();
    };
    // **許可証が無ければ書き込みの文へ至る枝が無い**（ADR-0026 §決定1）。
    let Some(permit) = mode.applying() else {
        return LedgerReport::untouched();
    };

    let charges = match store.drift_charges(job.character_id).await {
        Ok(charges) => charges,
        Err(err) => {
            // **累積が読めない回は値を動かさない。**
            // 0 として進めると、**溜まっていた端数が消えて静かに巻き戻る。**
            tracing::error!(
                error = %err,
                detail = ?err,
                session_id = job.session_id.get(),
                "性格の変容の累積を読めませんでした"
            );
            return LedgerReport::failed();
        }
    };

    let entries = allocs
        .iter()
        .zip(counts)
        .map(|(alloc, scored)| {
            let charge = charges
                .iter()
                .find(|stored| stored.axis() == &alloc.axis)
                // **行が無い軸は累積 0**（初回のターン）。
                // **時刻はここでは読まない**——帳簿は「いつ」を見ない（ADR-0027 §決定7）。
                .map_or(0.0, StoredDriftCharge::charge);
            DriftAxisLedger::new(alloc.axis.clone(), alloc.value, charge, scored)
        })
        .collect();

    let mut ledger = DriftLedger::new(form, tuning, entries);
    let effect = match ledger.absorb(scores) {
        Ok(effect) => effect,
        Err(err) => {
            tracing::error!(
                error = %err,
                detail = ?err,
                session_id = job.session_id.get(),
                "性格の変容の採点を帳簿に通せませんでした"
            );
            return LedgerReport::failed();
        }
    };

    let mut report = LedgerReport {
        write: NO_WRITE,
        // **数えるのは「段」**（動いた軸の数ではない / [`LedgerReport::steps`] の doc）。
        steps: effect.ups().saturating_add(effect.downs()),
        ups: effect.ups(),
        downs: effect.downs(),
        saturations: effect.saturations().len(),
        moved: None,
    };

    // **動いた軸を1つずつ書く。** `store` の文は現在値を条件に含めるので、
    // 取りこぼしは 0 行更新として観測できる（ADR-0024 §7）。
    for change in effect.changes() {
        // **許可証は書き込み1回につき1枚**（`DriftApply` は `Copy` ではない）。
        let Some(step_permit) = mode.applying() else {
            // **到達しない**（上で1枚取れている）。`expect` を置かずに
            // 安全側（書かない）へ倒す（CLAUDE.md 絶対制約）。
            report.write = NO_WRITE;
            continue;
        };
        match store
            .apply_stat_drift_in_session(job.character_id, job.session_id, change, step_permit)
            .await
        {
            Ok(write) => report.note_write(write.label()),
            // **キャラクターが完全に消された回は故障ではない**（ADR-0049 §決定3 / 作業単位 A-10）。
            Err(err) if character_vanished(&err) => {
                record_vanished(job, VANISHED_STAT_VALUE, &err);
                report.note_write(WRITE_FAILED);
            }
            Err(err) => {
                // **本文には影響しない。** 応答はとうに返っている。
                // 原因は `#[source]` 側にしか無い（`StoreError` の `Display` は定数文字列）。
                tracing::error!(
                    error = %err,
                    detail = ?err,
                    session_id = job.session_id.get(),
                    "性格ステータスの変容を書き込めませんでした"
                );
                report.note_write(WRITE_FAILED);
            }
        }
    }

    // **ちょうど1軸だけ動いた回は、前後の値をログに載せる**（[`record_run`] の doc）。
    if let [change] = effect.changes() {
        report.moved = Some((
            change.axis().clone(),
            change.delta(),
            change.before().get(),
            change.after().get(),
        ));
    }

    // **累積は動いた軸だけでなく全軸ぶん書き戻す。**
    // 薄れた軸の端数も持ち越さないと、**次のターンで下向きが消える。**
    let updated: Vec<(StatAxis, f64)> = ledger
        .axes()
        .iter()
        .map(|entry| (entry.axis().clone(), entry.charge()))
        .collect();
    if let Err(err) = store
        .save_drift_charges_in_session(job.character_id, job.session_id, &updated, permit)
        .await
    {
        // **キャラクターが完全に消された回は故障ではない**（ADR-0049 §決定3 / `record_votes` と同じ）。
        if character_vanished(&err) {
            record_vanished(job, VANISHED_DRIFT_CHARGE, &err);
        } else {
            tracing::error!(
                error = %err,
                detail = ?err,
                session_id = job.session_id.get(),
                "性格の変容の累積を書き戻せませんでした"
            );
        }
        report.write = WRITE_FAILED;
    }

    report
}

/// **帳簿を回した結果のうち、ログに出すぶん。**
struct LedgerReport {
    /// 書き込みの欄（[`NO_WRITE`] / `moved` / `not_moved` / [`WRITE_FAILED`]）。
    write: &'static str,
    /// **実際に値が動いた「段」の数**（＝ [`crate::domain::DriftTurnEffect::ups`] ＋ `downs`）。
    ///
    /// # ⚠ **「動いた軸の数」ではない**（レビュー指摘D / 16e-b）
    ///
    /// **費用が安い領域では1軸が1ターンに2段以上動く**（ADR-0027 §9）ので、
    /// 2つは食い違う。**ADR の表は「段」を数えている**ので、こちらに揃えた。
    /// 揃えないと、**`all.awk` の再現値とログの値が静かにずれる**
    /// ——次の実機測定はこの欄をログから数える。
    steps: usize,
    /// **上がった段の数。**
    ///
    /// # なぜ向きを別の欄に出すのか
    ///
    /// `drift_delta` は**ちょうど1軸だけ動いた回**しか埋まらない
    /// （2軸以上動いた回に載せる向きが1つに決まらない）。
    /// **ADR-0024 の「危険 2. 一方向へ流れる」はログだけから数える計画**なので、
    /// この2欄が無いと**2軸以上動いたターンの向きが完全に落ちる。**
    ups: usize,
    /// **下がった段の数。**
    downs: usize,
    /// **端で動かせなかった段の数**（ADR-0024 の「危険 1. 端に貼り付く」）。
    saturations: usize,
    /// ちょうど1軸だけ動いた回の `(軸, 向き, 前, 後)`。
    moved: Option<(StatAxis, DriftDelta, u32, u32)>,
}

impl LedgerReport {
    /// 帳簿を回さなかった回（採点が読めない／記録専用モード）。
    const fn untouched() -> Self {
        Self {
            write: NO_WRITE,
            steps: 0,
            ups: 0,
            downs: 0,
            saturations: 0,
            moved: None,
        }
    }

    /// 帳簿そのものが失敗した回。**「動かなかった」と混ぜない**（故障である）。
    const fn failed() -> Self {
        Self {
            write: WRITE_FAILED,
            steps: 0,
            ups: 0,
            downs: 0,
            saturations: 0,
            moved: None,
        }
    }

    /// 書き込みの結果を畳む。**故障は上書きされない。**
    ///
    /// 1ターンに2軸以上動きうる（ADR-0027 §9 の安い領域）ので、
    /// **一番悪い結果を残す**——`failed` > `not_moved` > `moved`。
    /// 良い方を残すと、**1件でも書けた回が「全部書けた」に見える。**
    fn note_write(&mut self, write: &'static str) {
        if self.write == WRITE_FAILED {
            return;
        }
        if self.write == NO_WRITE || write != DriftWrite::Moved.label() {
            self.write = write;
        }
    }
}

/// **変容1回につき1イベント**（ADR-0024 の「危険」3つの観測点）。
///
/// # 欄は結末によらず**まったく同じ**
///
/// 分岐ごとに欄を増やすと、集計が分岐で割れる（`super::extract::record_run` と同じ判断）。
/// 該当しない回は番兵（[`NO_AXIS`] / [`NO_DELTA`] / [`NO_VALUE`] / [`NO_WRITE`] /
/// [`NO_VOTE`]）を出す。
///
/// # ADR-0024 の「危険」3つが、この1行から数えられる
///
/// | 危険 | 数え方 |
/// |---|---|
/// | 1. 端に貼り付く | `outcome=saturated` の件数 |
/// | 2. 一方向へ流れる | `delta=up` と `delta=down` の比 |
/// | 3. 提案が出ない | `outcome=nothing` と、それ以外の比（提案率） |
///
/// # 走らせ方と票の欄（ADR-0026 §決定1 / §決定3）
///
/// | 欄 | 何を読むか |
/// |---|---|
/// | 走らせ方 | `shadow` と `on` の区別。**後から測定を読むときに、
///   適用があったのか無かったのかが分からないと困る** |
/// | 票 | 票を1件残したか（[`VOTE_RECORDED`]）・残せなかったか（[`VOTE_FAILED`]） |
///
/// **記録専用モードでは書き込みの欄が常に [`NO_WRITE`] になる。**
/// **`outcome=applied` でも値は動いていない**——動くかどうかは走らせ方の欄が決める。
///
/// # 破棄の内訳を1つの値に畳まない（抽出の `discards=` とは形が違う）
///
/// **変容は1ターンに結末が1つしか無い。** 抽出は1回の依頼から複数行を採るので
/// 「ラベル=件数」の畳み込みが要るが、こちらで同じことをすると
/// **毎行に 0 が6つ並ぶだけ**で、`outcome=` を見るより数えにくくなる。
///
/// **語彙が二重管理になる心配は無い。** ここが出すのは
/// `DriftOutcome::label()` がそのまま返す綴りで、
/// **破棄の理由も結末も1つの語彙**（`DriftOutcome::label` の doc）。
/// 綴りが重ならないことは `domain` の `every_outcome_label_is_distinct` が見張る。
///
/// # 水準は `info` の1つだけ
///
/// 抽出は「`domain` の検証で弾いた行があった回」を `warn` に上げるが、
/// **変容では上げない。** 捨てても失われるものが無いから
/// （同じ話題が続けば次のターンでまた提案できる / `parse::drift` の doc）。
/// 一方、**書き込みが落ちた回は故障**なので、そちらは別に `tracing::error!` を出す。
///
/// # 欄名は **`drift_` で始める**（16b のレビューで実測）
///
/// ADR-0024 は危険3つを「**ログの数だけで判別できる形**」にすると約束している。
/// **接頭辞が無いと、素朴な grep が別のイベントを一緒に数える。**
/// 実測（`scratchpad/m12`）:
///
/// | 綴り | 混ざったもの |
/// |---|---|
/// | `outcome=` | `server::turn` の分解ログ（`narration_only` 18 / `structured` 12） |
/// | `axes=` | `server::admin` の「性格ステータスを設定しました」1件 |
/// | `skipped=` / `prompt_tokens=` / `limit_tokens=` | **`server::extract` の同名の欄** |
///
/// **数え方が測定を歪める形そのもの**（docs/05-development.md「判定は読んで行う」）で、
/// **この測定は作業単位 17 でも繰り返す。**
///
/// **`session_id` / `error` / `detail` は接頭辞を付けない。**
/// あれらは横断して引くための共通の綴りで、集計の分母にはならない。
///
/// 綴りが他のモジュールへ漏れていないことは
/// `the_drift_fields_never_collide_with_another_module` が走査で見張る。
///
/// # **生出力を1欄に載せる**（`drift_raw` / 腕F の測定で見つかった穴）
///
/// **成功した回も破棄した回も載せる。** ADR-0027 §12 の問い（`0` は実際に出るか）は
/// **破棄されなかった回の中身**を数える問いなので、破棄時だけでは答えられない。
///
/// **上限は [`DRIFT_RAW_LOG_CHARS`]**、切り詰めたら [`DRIFT_RAW_TRUNCATED`] を付ける。
/// **`Debug`（`?`）で書く**ので改行はエスケープされ、**1イベント1行**が保たれる
/// （`server_plain.log` を行単位で数える道具が既にある）。
///
/// # 軸名だけ `Debug` で出す
///
/// 軸名は**空白を含みうる自由文**（`StatAxis` の doc）なので、
/// `Display` だと `axis=内なる 情熱` になって欄の切れ目が読めない。
/// `Debug` なら引用符で囲まれる。**載せるのは保存済みの綴り**
/// （管理 API から書かれた検証済みの値）であって、サブLLM の生出力ではない。
fn record_run(
    job: &TurnJob,
    reading: &DriftReading,
    applied: &LedgerReport,
    axes: usize,
    recorded: &'static str,
    mode: StatDriftMode,
    raw: &str,
) {
    // **軸と向きの欄は「モデルが何を言ったか」**（`single` のときだけ埋まる）。
    let axis = match (&applied.moved, &reading.axis) {
        // **動いた軸があればそちらを出す**（`all` では提案された1軸が存在しない）。
        (Some((axis, ..)), _) => axis.as_str(),
        (None, Some(axis)) => axis.as_str(),
        (None, None) => NO_AXIS,
    };
    let delta = match (&applied.moved, reading.delta) {
        (Some((_, delta, ..)), _) => delta.label(),
        (None, Some(delta)) => delta.label(),
        (None, None) => NO_DELTA,
    };
    // **前後の値は「実際に動いた回」だけ**（ADR-0027 §決定7 で判定と適用が分かれた）。
    let (before, after) = match &applied.moved {
        Some((_, _, before, after)) => (*before, *after),
        None => (NO_VALUE, NO_VALUE),
    };

    tracing::info!(
        drift_outcome = %reading.label,
        drift_moved = %applied.write,
        drift_steps = applied.steps,
        drift_ups = applied.ups,
        drift_downs = applied.downs,
        drift_saturations = applied.saturations,
        drift_quotes = reading.quotes_stripped,
        drift_recorded = %recorded,
        drift_run_mode = %mode.label(),
        // **走らせ方の出どころ**（ADR-0045 §結果の ⚠ / 作業単位 A-8）——
        // 既存のキャラクターに値を書いたまま測定を回したとき、環境変数が
        // 効かなかったことを**ログだけで**追えるように。
        drift_mode_source = %job.drift_mode.source().label(),
        drift_axes = axes,
        drift_axis = ?axis,
        drift_before = before,
        drift_after = after,
        drift_delta = %delta,
        // **生出力**（上限つき）。**`Debug` で書く**ので改行はエスケープされ、
        // 1イベントが1行のまま保たれる。**サブLLM の出力そのもの**なので、
        // ログ以外へは渡さない（レスポンスにも SQL にも出さない / 絶対制約）。
        drift_raw = ?loggable_raw(raw),
        session_id = job.session_id.get(),
        "{RAN_MESSAGE}"
    );
}

/// **上流が停止理由を申告しなかったことを記録する**（作業単位 B-1 / 24 と同じ作法）。
///
/// この回は「**打ち切られたか分からない**」まま読んでいる
/// （[`response_completeness`] は「切れていない」に倒す）。
/// **残さないと、その状態が起きていること自体に気付く手段がなくなる**——
/// 打ち切りが黙って `applied` に化けていても、`drift_outcome` の内訳は正常に見える。
///
/// # なぜ「切れたかもしれない」側に倒さないのか
///
/// **停止理由を返さない上流に繋いだ瞬間、変容が丸ごと止まる。**
/// しかもログには `truncated` と出るので、**「上限に張り付いている」としか読めない**
/// ——上限を上げても直らない（抽出側とまったく同じ判断 / ADR-0021 §17）。
///
/// # 変容を1回判定したこと自体の記録（[`record_run`]）とは別のイベント
///
/// あちらの欄は結末の集計に使っており、**別の意味の数を混ぜると比較が無効になる**
/// （`drift_quotes` / `drift_axes` と同じ作法）。
fn record_missing_finish_reason(job: &TurnJob) {
    tracing::warn!(
        session_id = job.session_id.get(),
        "{NO_FINISH_REASON_MESSAGE}"
    );
}

/// **走らせなかった回も数える**（ADR-0024 の危険3つは、すべて率で読む）。
///
/// 「判定した」と**別のイベント**にしてある。同じ行にすると、
/// 提案率の分母に「そもそも判定していない回」が混ざる。
///
/// `prompt_tokens` は長すぎて捨てた回と上流が落ちた回だけ載る。
/// **上限と並べて出す**——どれだけ超えたのかが分からないと、上限を見直せない。
///
/// # `cause` がある回は `warn`（`super::extract::record_skip` と同じ作法）
///
/// 「軸が無い」「長すぎる」は**設計どおりの結果**なので `info` のまま。
/// **性格ステータスが読めなかった回と上流が落ちた回だけが故障**で、
/// 原因は `#[source]` 側にしか無い（`Display` は定数文字列 / 絶対制約）。
///
/// **見張り番は2本**（作業単位 B-2-2 / どちらも**手掛かりと同じ行の中で**水準を見る）:
/// `every_drift_record_names_its_turn_and_keeps_its_level`（`info` の2枝と `warn` の枝）と
/// `stats_that_cannot_be_read_skip_the_drift_and_keep_the_cause`（故障の枝）。
/// **B-2 でこの表を書いたときは1本も無く、`warn!` と `info!` を入れ替える変異が全緑だった。**
///
/// **集計キー（`skipped`）と件数の欄は水準によらず同じ。**
fn record_skip(
    reason: DriftSkipReason,
    job: &TurnJob,
    prompt_tokens: Option<u32>,
    cause: Option<&dyn std::error::Error>,
    mode: StatDriftMode,
) {
    let prompt_tokens = prompt_tokens.unwrap_or_default();
    // **その回に効いていた上限**（作業単位 B-7）。**定数ではなく往復の材料から出す**
    // ——抽出側の limit_tokens とまったく同じ理由（super::extract::record_skip）。
    let drift_limit_tokens = drift_prompt_limit(job.sub_context);
    match cause {
        Some(err) => tracing::warn!(
            drift_skipped = %reason.label(),
            drift_run_mode = %mode.label(),
            drift_mode_source = %job.drift_mode.source().label(),
            drift_prompt_tokens = prompt_tokens,
            drift_limit_tokens,
            error = %err,
            detail = ?err,
            session_id = job.session_id.get(),
            "{SKIPPED_MESSAGE}"
        ),
        None => tracing::info!(
            drift_skipped = %reason.label(),
            drift_run_mode = %mode.label(),
            drift_mode_source = %job.drift_mode.source().label(),
            drift_prompt_tokens = prompt_tokens,
            drift_limit_tokens,
            session_id = job.session_id.get(),
            "{SKIPPED_MESSAGE}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use tokio::runtime::Runtime;
    use tokio::sync::oneshot;

    use super::*;
    use crate::domain::{
        Character, CharacterId, ChatMessage, DRIFT_FLOOR, DisplayName, DriftDelta, Handle,
        MemoryContent, NewMemory, Persona, SessionId, StatAlloc, StatAxis, StatScale, Subject,
    };
    use crate::llm::{ChatStream, Completion, FinishReason, LlmError, LlmFuture};
    use crate::server::extract::{Extraction, MemoryExtractor};
    use crate::store::DriftWrite;
    use crate::testlog::{
        captured_under_default_filter, has_field, has_key, line_containing, quoted_field,
    };

    /// 利用者の発言。**応答とも、どの軸名とも別の文字列。**
    const USER_MESSAGE: &str = "今日の発表、うまくいったよ。褒められた。";

    /// キャラクターの応答。
    const REPLY: &str = "「よかったですね」 彼女は湯呑みを置いた。";

    /// **動かす軸。** 他のどの軸とも十分違う綴り（許可リストの照合が観測対象）。
    const MOVED_AXIS: &str = "パッション";

    /// **動かさない軸。** 値 0 ではないので依頼には載る。
    const STILL_AXIS: &str = "ロジック";

    /// **動かす軸と同じ値**を持つ、別の軸。
    ///
    /// # なぜ「値が同じ軸」が要るのか（変異検査で見つかった穴 / 2026-08-23）
    ///
    /// 軸の値が全部バラバラだと、**`store` の `axis = ?` の条件を落とす変異が緑を通る**
    /// ——`value = ?` の条件だけで行が1つに絞られてしまうため（実測）。
    /// 権威は `tests/store_stat_drift.rs` の同名の軸だが、
    /// **配線の側でも同じ状態を作っておく。**
    const TWIN_AXIS: &str = "共鳴";

    /// **値 0 の軸。**
    ///
    /// **依頼には載る**（ADR-0027 §決定4 / 見張り番は
    /// `the_drift_request_carries_every_stored_axis_including_the_silent_one`）。
    /// **本文プロンプトには載らない**——落とすのは `prompt::sanitize_character` の側で、
    /// 変容の依頼はその述語を見ない。
    ///
    /// **上げれば動く**（`0` → `1`。`a_character_whose_axes_are_all_zero_is_still_asked`）。
    /// 下げようとすると飽和する（下端は `DRIFT_FLOOR` = 1）。
    const SILENT_AXIS: &str = "冷徹さ";

    /// **保存されていない軸。** モデルが作った名前の代役（ADR-0024 §1）。
    const UNKNOWN_AXIS: &str = "破壊衝動";

    /// **会話する**キャラクターの識別子。
    const HANDLE: &str = "aoi";

    /// **会話する**キャラクターの呼び名。**識別子とは別の綴り**（ADR-0011）。
    const DISPLAY_NAME: &str = "葵";

    /// **会話しない**キャラクターの識別子。会話する方とは別の綴り。
    const BYSTANDER_HANDLE: &str = "kaede";

    /// **会話しない**キャラクターの呼び名。他の3つとは別の綴り。
    const BYSTANDER_DISPLAY_NAME: &str = "楓";

    /// **会話するキャラクターの尺度。** 既定とも傍観者とも別の値。
    const SCALE: u32 = 7;

    /// **会話しないキャラクターの尺度。** 会話する方とも既定とも別の値。
    ///
    /// # なぜ3つとも違う値が要るのか
    ///
    /// 依頼に載る上限が「どのキャラクターのものか」を見るため。
    /// 既定と同値だと `StatScale::DEFAULT` を返す実装が緑を通り、
    /// 傍観者と同値だと**別のキャラクターの尺度を読む**実装が緑を通る
    /// （CLAUDE.md「抜けるのはいつも帰属」）。
    const BYSTANDER_SCALE: u32 = 5;

    /// [`MOVED_AXIS`] の初期値。**端ではない**（飽和ばかりが見える形にしない）。
    const MOVED_START: u32 = 4;

    /// [`STILL_AXIS`] の初期値。[`MOVED_START`] とは別の値。
    const STILL_START: u32 = 2;

    /// 抽出の上流が返す本文。**1件も採らせない**
    /// （この単体テストが見ているのは変容だけ）。
    const EXTRACTION_REPLY: &str = "";

    /// **変容の依頼だけに現れる見出し。**
    ///
    /// **`prompt::drift` の定数を呼ばず、テスト側に期待値として持つ**
    /// （呼ぶと綴りを変える変異が両側で同時に動いて緑になる）。
    /// 綴りが変われば [`MockSub::drift_prompt`] が「変容の依頼が届いていない」で落ちる。
    const STATS_HEADING: &str = "# いまの性格";

    /// 裏方の上流の代役。**受けた依頼を控え、依頼の種類ごとに答える。**
    ///
    /// # なぜ「順番」ではなく「中身」で答え分けるのか
    ///
    /// **抽出は飛ぶことがある**（記憶が読めない／長すぎる）。順番で答えると、
    /// **抽出が飛んだ回に変容が抽出用の答えを受け取る**——
    /// そして「抽出が飛んでも変容は走る」という見張り番が、
    /// **走ったのに何も起きない形で緑になる**（実測。この形で一度書いて踏んだ）。
    struct MockSub {
        requests: Mutex<Vec<ChatRequest>>,
        /// **変容の依頼への答え。**
        drift_reply: String,
        /// `Some` なら、[`MockSub::gate_drift`] が指す方の依頼を**開くまで返さない。**
        gate: Mutex<Option<oneshot::Receiver<()>>>,
        /// **門をどちらの依頼に掛けるか。**
        ///
        /// * `false` … **抽出**を止める。タスクが走り続ける状態を作る（`Busy` の観測）。
        /// * `true` … **変容**を止める。**変容が軸を読んでから書くまでの窓**を開ける
        ///   （ADR-0024 §7 の競合の観測）。
        ///
        /// **この2つを1つにできない。** 抽出を止めると変容はまだ軸を読んでおらず、
        /// その窓で値を書き換えても**変容は新しい値を読んで普通に成功する**
        /// （実測。この形で一度書いて踏んだ）。
        gate_drift: bool,
        /// **変容の依頼にだけ**失敗を返すか。
        fails_the_drift: bool,
        /// **変容の応答にだけ**付ける停止理由（作業単位 B-1）。
        ///
        /// # なぜ抽出の応答と分けるのか
        ///
        /// **抽出も同じモックから答えを受け取る。** 両方に同じ値を付けると、
        /// `None` を演じた回に**抽出側の「停止理由が付いていません」まで一緒に出る**
        /// ——変容の記録を見ているつもりで、抽出の記録を読むことになる
        /// （`MockSub::is_drift` の doc「順番ではなく中身で答え分ける」と同じ理由）。
        drift_finish_reason: Option<FinishReason>,
        calls: AtomicUsize,
    }

    impl MockSub {
        /// 変容の依頼に `drift_reply` を返すモック。**停止理由は `stop`。**
        fn replying(drift_reply: &str) -> Arc<Self> {
            Self::replying_with(drift_reply, Some(FinishReason::Stop))
        }

        /// **停止理由まで指定して**変容の依頼に答えるモック（作業単位 B-1）。
        ///
        /// 打ち切られた回を演じるには `Some(FinishReason::Length)` を渡す。
        fn replying_with(
            drift_reply: &str,
            drift_finish_reason: Option<FinishReason>,
        ) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                drift_reply: drift_reply.to_owned(),
                gate: Mutex::new(None),
                gate_drift: false,
                fails_the_drift: false,
                drift_finish_reason,
                calls: AtomicUsize::new(0),
            })
        }

        /// **変容の依頼にだけ失敗を返す**（抽出は普通に終わる）。
        fn failing_the_drift() -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                drift_reply: String::new(),
                gate: Mutex::new(None),
                gate_drift: false,
                fails_the_drift: true,
                drift_finish_reason: Some(FinishReason::Stop),
                calls: AtomicUsize::new(0),
            })
        }

        /// **抽出の応答を開くまで返さないモック**（タスクが走り続ける状態を作る）。
        fn gating_the_extraction() -> (Arc<Self>, oneshot::Sender<()>) {
            Self::gating(false, "なし")
        }

        /// **変容の応答を開くまで返さないモック**（軸を読んでから書くまでの窓を開ける）。
        fn gating_the_drift(drift_reply: &str) -> (Arc<Self>, oneshot::Sender<()>) {
            Self::gating(true, drift_reply)
        }

        fn gating(gate_drift: bool, drift_reply: &str) -> (Arc<Self>, oneshot::Sender<()>) {
            let (open, gate) = oneshot::channel();
            let mock = Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                drift_reply: drift_reply.to_owned(),
                gate: Mutex::new(Some(gate)),
                gate_drift,
                fails_the_drift: false,
                drift_finish_reason: Some(FinishReason::Stop),
                calls: AtomicUsize::new(0),
            });
            (mock, open)
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        /// 受けた依頼が**変容の依頼**か。
        fn is_drift(request: &ChatRequest) -> bool {
            request
                .messages
                .iter()
                .any(|message| message.content.contains(STATS_HEADING))
        }

        /// **変容の依頼**を1つの文字列に畳んだもの。**無ければ落とす。**
        ///
        /// **「最後の依頼」で採らない。** 変容が走らなかった回に
        /// **抽出の依頼を変容の依頼として読む**ことになる。
        fn drift_prompt(&self) -> String {
            let requests = self.requests.lock().expect("ロックを取れること");
            let request = requests
                .iter()
                .find(|request| Self::is_drift(request))
                .expect("変容の依頼が届いていること");
            request
                .messages
                .iter()
                .map(|message| message.content.clone())
                .collect::<Vec<_>>()
                .join("\n")
        }

        /// 変容の依頼の応答上限。**無ければ落とす。**
        fn drift_max_tokens(&self) -> Option<u32> {
            let requests = self.requests.lock().expect("ロックを取れること");
            requests
                .iter()
                .find(|request| Self::is_drift(request))
                .expect("変容の依頼が届いていること")
                .max_tokens
        }

        /// 変容の依頼の温度。**無ければ落とす。**
        ///
        /// **「最後の依頼」で採らない**（[`MockSub::drift_prompt`] と同じ理由 ——
        /// 変容が走らなかった回に**抽出の依頼の温度**を読むことになる）。
        fn drift_temperature(&self) -> Option<f32> {
            let requests = self.requests.lock().expect("ロックを取れること");
            requests
                .iter()
                .find(|request| Self::is_drift(request))
                .expect("変容の依頼が届いていること")
                .temperature
        }
    }

    impl LlmClient for MockSub {
        fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let is_drift = Self::is_drift(request);
            self.requests
                .lock()
                .expect("ロックを取れること")
                .push(request.clone());
            // **門は片方の依頼にだけ掛ける**（[`MockSub::gate_drift`]）。
            let gate = if is_drift == self.gate_drift {
                self.gate.lock().expect("ロックを取れること").take()
            } else {
                None
            };

            Box::pin(async move {
                if let Some(gate) = gate {
                    let _ = gate.await;
                }
                // **変容の依頼だけを落とす**（抽出は普通に終わる）。
                if self.fails_the_drift && is_drift {
                    return Err(LlmError::MissingContent);
                }
                let (content, finish_reason) = if is_drift {
                    (self.drift_reply.clone(), self.drift_finish_reason)
                } else {
                    // **抽出の応答は常に `stop`。** 変容の停止理由をここへ漏らさない
                    // （[`MockSub::drift_finish_reason`] の doc）。
                    (EXTRACTION_REPLY.to_owned(), Some(FinishReason::Stop))
                };
                Ok(Completion {
                    content,
                    finish_reason,
                    usage: None,
                })
            })
        }

        fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
            Box::pin(async move { Err(LlmError::MissingContent) })
        }
    }

    /// **現在のスレッドで走らせる runtime**（ログの捕捉はスレッドローカル）。
    fn runtime() -> Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime を作れること")
    }

    /// キャラクターを1件作って読み直す。
    async fn create_character(
        store: &Store,
        handle: &str,
        display_name: &str,
        scale: u32,
    ) -> Character {
        let handle = Handle::parse(handle).expect("正当な識別子であること");
        store
            .create_character(
                &handle,
                &DisplayName::parse(display_name).expect("正当な表示名であること"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
                None,
                StatScale::parse(scale).expect("正当な尺度であること"),
            )
            .await
            .expect("キャラクターを作成できること");
        store
            .find_character_by_handle(&handle)
            .await
            .expect("キャラクターを引けること")
            .expect("作成済みであること")
    }

    /// 軸と値の組から `StatAlloc` を作る。
    fn allocs(scale: u32, pairs: &[(&str, u32)]) -> Vec<StatAlloc> {
        let scale = StatScale::parse(scale).expect("正当な尺度であること");
        pairs
            .iter()
            .map(|(axis, value)| StatAlloc {
                axis: StatAxis::parse(axis).expect("正当な軸名であること"),
                value: scale.value(*value).expect("尺度に収まる値であること"),
            })
            .collect()
    }

    /// **2体のキャラクターを持つ DB。**
    ///
    /// # なぜ2体要るのか
    ///
    /// 1体しか作らないと `character.id` と `1` が常に同値になり、
    /// **持ち主を決め打ちにする実装が原理的に観測できない**（CLAUDE.md）。
    /// **尺度も別**にしてあるので、**別のキャラクターの尺度を依頼に載せる**実装も落ちる。
    ///
    /// 傍観者は [`MOVED_AXIS`] を**同じ値**で持っている——
    /// `character_id` の条件を落とす変異が「両方動いた」として現れる。
    struct Fixture {
        store: Store,
        /// **会話する方。** 後から作るので `id` は 1 ではない。
        character: Character,
        /// **会話しない方。**
        bystander: Character,
        session_id: SessionId,
    }

    async fn fresh(path: &std::path::Path) -> Fixture {
        let store = Store::connect(path).await.expect("DB を開けること");

        // **会話しない方を先に作る**（＝ `id` が 1 になるのはこちら）。
        let bystander = create_character(
            &store,
            BYSTANDER_HANDLE,
            BYSTANDER_DISPLAY_NAME,
            BYSTANDER_SCALE,
        )
        .await;
        let character = create_character(&store, HANDLE, DISPLAY_NAME, SCALE).await;

        assert_ne!(character.id, bystander.id, "前提: 2体の id が別であること");
        assert_ne!(
            character.id,
            CharacterId::from_row(1),
            "前提: 会話する方の id が 1 でないこと（決め打ちと同値にしない）"
        );
        assert_ne!(
            character.stat_scale, bystander.stat_scale,
            "前提: 2体の尺度が別であること（帰属を観測できる形）"
        );
        assert_ne!(
            character.stat_scale,
            StatScale::DEFAULT,
            "前提: 尺度が既定と別であること（常に既定を返す実装を検出できる形）"
        );

        put_stats(
            &store,
            character.id,
            &[
                (MOVED_AXIS, MOVED_START),
                (TWIN_AXIS, MOVED_START),
                (STILL_AXIS, STILL_START),
                (SILENT_AXIS, 0),
            ],
        )
        .await;
        put_stats(&store, bystander.id, &[(MOVED_AXIS, MOVED_START)]).await;

        let session_id = store
            .create_session(character.id)
            .await
            .expect("セッションを作成できること");

        Fixture {
            store,
            character,
            bystander,
            session_id,
        }
    }

    /// **性格ステータスを置く。テストの土台。**
    ///
    /// # なぜ `store` の全置換 API を呼ばないのか
    ///
    /// **このファイルが `the_drift_path_never_reaches_for_the_wholesale_replacement` の
    /// 走査対象だから。** あの走査は**生ソースを見る**ので、
    /// テストの土台であってもその関数名をここに綴れない
    /// （`store::memory::table_word` と同じ制約。走査は綴りしか見ない）。
    ///
    /// **代わりに素の SQL で置く。** 入るのは `StatAxis` / `StatValue` を通した値と
    /// 同じ形の行なので、土台としての性質は変わらない
    /// （`tests/common` のヘルパ群と同じ立場）。
    async fn put_stats(store: &Store, id: CharacterId, pairs: &[(&str, u32)]) {
        sqlx::query("DELETE FROM stat_alloc WHERE character_id = ?")
            .bind(id.get())
            .execute(store.pool())
            .await
            .expect("既存の軸を消せること");
        for (axis, value) in pairs {
            // **軸も値もドメイン型を通してから束ねる**（土台でも検証を素通りさせない）。
            let axis = StatAxis::parse(axis).expect("正当な軸名であること");
            let value = StatScale::parse(SCALE.max(BYSTANDER_SCALE))
                .expect("正当な尺度であること")
                .value(*value)
                .expect("尺度に収まる値であること");
            sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
                .bind(id.get())
                .bind(axis.as_str())
                .bind(value.get())
                .execute(store.pool())
                .await
                .expect("軸を置けること");
        }
    }

    /// 保存済みの軸を `(軸名, 値)` の組で読む。
    async fn stats(store: &Store, id: CharacterId) -> Vec<(String, u32)> {
        store
            .stat_allocs(id)
            .await
            .expect("性格ステータスを読めること")
            .into_iter()
            .map(|alloc| (alloc.axis.as_str().to_owned(), alloc.value.get()))
            .collect()
    }

    /// **この単体テスト群が使うつまみ。値付けを外した（傾き 0）既定。**
    ///
    /// # なぜ既定（傾き 1.5）で回さないのか
    ///
    /// このファイルが見ているのは**配線**（誰の軸を、どの表へ、どの順で）であって、
    /// 費用曲線ではない。値付けが効いていると**値によって同じ提案が動いたり動かなかったりする**ので、
    /// 「配線が壊れて動かなかった」と「曲線が高くて動かなかった」が混ざる。
    ///
    /// 曲線そのものの権威は `domain::drift_ledger`（ADR の表を再現する側）。
    ///
    /// # ⚠ **既定が観測対象から外れないように、別の見張り番を立ててある**
    ///
    /// CLAUDE.md「テスト専用の入口を作ると、本番の入口が観測対象から外れる」。
    /// 既定のつまみを通る見張り番は
    /// [`the_default_pricing_never_lifts_an_axis_nobody_mentioned`]。
    ///
    /// **つまみが現に帳簿へ届いていることは、このヘルパを使うテスト群そのものが見ている**
    /// ——`K = 0.065` は既定では届かない安さなので、
    /// **つまみを丸ごと捨てて `DriftTuning::default()` で回す変異は、
    /// 「値が動く」と書いた 17 本を一斉に赤にする**（レビュアーの実測 / 16g）。
    ///
    /// > **⚠ かつてここに `a_larger_step_cost_keeps_the_value_still_but_still_records_the_vote`
    /// > という専用の見張り番を挙げていた。16g で消した。**
    /// > **票の重みが `0.1/√n` になったので「既定でも動かない」状態になり、
    /// > `K = 1000` と既定が同一結果になって、つまみを何も見なくなっていた**
    /// > （上の 17 本の変異でこの1本だけが緑だった）。
    /// > **「値が動かない回でも票は残る」**（§決定7）の側は
    /// > [`the_third_vote_weighs_a_third_because_every_past_vote_is_counted`] の
    /// > `K = 1_000` の一巡が引き継いでいる。
    ///
    /// # ⚠ **`K` は `0.065`**（作業単位 16g）
    ///
    /// **票の重みが `1` から `0.1/√n` になった**（ADR-0027 訂正 3）ので、
    /// **`K = 1` では初回の票でも1段も動かない。**
    /// 配線を見るには「**1票でちょうど1段**」でなければならない。
    ///
    /// **1段の費用は `K × (既定尺度/尺度)`**（ADR-0027 訂正 2）なので、
    /// このキャラクターの尺度（[`SCALE`] = 7）では `K × 10/7`。
    /// `K = 0.065` で **費用 ≒ 0.0929** ——初回の票（0.1）でちょうど1段動き、
    /// **2段目には届かない**（端数 0.0071）。
    fn fixture_tuning() -> DriftTuning {
        let tuning = DriftTuning::new(1.0, 0.0, 0.065).expect("つまみが範囲内であること");
        assert_ne!(
            tuning,
            DriftTuning::default(),
            "既定と違うつまみで測ること（既定を返す実装を検出できる形）"
        );
        tuning
    }

    /// 変容を差した抽出器（**スイッチ on**）。
    fn extractor_with_drift(store: &Store, sub: &Arc<MockSub>) -> Arc<MemoryExtractor> {
        let llm: Arc<dyn LlmClient> = Arc::clone(sub) as Arc<dyn LlmClient>;
        Arc::new(MemoryExtractor::new(
            store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::On,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ))
    }

    /// 保存済みの票を `(セッション, 軸名, 向き)` の組で、**新しい順**に読む。
    ///
    /// **セッションまで読む。** 「どの往復の票か」を落とすと、
    /// 帰属を取り違える実装が観測できない（CLAUDE.md「抜けるのはいつも帰属」）。
    async fn votes(store: &Store, id: CharacterId) -> Vec<(i64, String, &'static str)> {
        store
            .recent_drift_votes(id, VOTE_LIMIT)
            .await
            .expect("票を読めること")
            .iter()
            .map(|stored| {
                (
                    stored.session_id().get(),
                    stored.vote().axis().as_str().to_owned(),
                    stored.vote().delta().label(),
                )
            })
            .collect()
    }

    /// 読み出す票の件数の上限。**1往復で立つ票は高々1件**なので十分に大きい。
    const VOTE_LIMIT: u32 = 16;

    /// **1往復ぶんの裏方を最後まで走らせ、記録・軸・票を返す。**
    struct Ran {
        output: String,
        stats: Vec<(String, u32)>,
        bystander_stats: Vec<(String, u32)>,
        /// **会話したキャラクターの票。**
        votes: Vec<(i64, String, &'static str)>,
        /// **会話していないキャラクターの票。** 帰属を取り違えるとここに現れる。
        bystander_votes: Vec<(i64, String, &'static str)>,
        /// **会話したキャラクターの累積**（軸名順）。
        ///
        /// 帳簿の持ち越しは**値にも票にもログにも現れない**——
        /// 表を直接読まないと、「溜まっている」と「消えている」が区別できない。
        charges: Vec<(String, f64)>,
        /// 走らせた往復のセッション。**票の帰属の期待値。**
        session_id: i64,
        /// 走らせたキャラクター。**[`session_id`](Ran::session_id) と取り違えたことを
        /// 観測するためだけに持つ**（作業単位 B-1-2）。
        ///
        /// [`fresh`] が**会話しない方を先に作る**ので `character.id` は 1 ではなく、
        /// セッションは各テストで作り直す最初の1件なので `session_id` は 1 になる。
        /// **2つは必ず別の値**だが、それは `fresh` の並びに寄りかかった性質なので、
        /// **使う側で `assert_ne!` を置いて前提を明示すること**
        /// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
        character_id: i64,
    }

    /// スイッチ on で1往復走らせる。
    fn run_once(sub: &Arc<MockSub>, user_message: &str) -> Ran {
        run_once_with(sub, user_message, StatDriftMode::On)
    }

    /// スイッチを指定して1往復走らせる。
    ///
    /// # ⚠ **渡す形は [`DriftForm::Single`]。既定ではない**（作業単位 19）
    ///
    /// **既定は `all` へ移った**（ADR-0027 §決定8 / 16j）ので、
    /// **このファイルの単体テストの大半は非既定の形を通っている。**
    /// **「単体テストは既定の形を通っている」と読まないこと。**
    ///
    /// ここを既定へ寄せると、境界の2本
    /// （[`a_drift_request_exactly_at_the_limit_is_sent`] /
    /// [`a_drift_request_one_byte_over_the_limit_is_dropped`]）が
    /// **実機の依頼と1バイト違わないこと**を主張できなくなる
    /// ——あちらは [`expected_drift_request`] が組む依頼と突き合わせており、
    /// **両側が同じ形でなければならない。**
    ///
    /// **既定の形（`all`）を通る観測は別にある** ——
    /// [`the_default_request_form_leaves_room_for_the_conversation`]（枠の大きさ）と
    /// `tests/server_stat_drift.rs` の本番の入口を通る5本。
    fn run_once_with(sub: &Arc<MockSub>, user_message: &str, stat_drift: StatDriftMode) -> Ran {
        run_once_in(sub, user_message, stat_drift, DriftForm::Single)
    }

    /// スイッチと**依頼の形**を指定して1往復走らせる（**つまみは [`fixture_tuning`]**）。
    fn run_once_in(
        sub: &Arc<MockSub>,
        user_message: &str,
        stat_drift: StatDriftMode,
        form: DriftForm,
    ) -> Ran {
        run_once_tuned(sub, user_message, stat_drift, form, fixture_tuning())
    }

    /// スイッチ・依頼の形・**集約のつまみ**を指定して1往復走らせる。
    fn run_once_tuned(
        sub: &Arc<MockSub>,
        user_message: &str,
        stat_drift: StatDriftMode,
        form: DriftForm,
        tuning: DriftTuning,
    ) -> Ran {
        run_turns_tuned(sub, &[user_message], stat_drift, form, tuning)
    }

    /// **同じ DB で往復を続けて走らせる。**
    ///
    /// # なぜ複数ターンが要るのか
    ///
    /// 帳簿は**累積を次のターンへ持ち越す**（ADR-0027 §決定7）。
    /// 1ターンしか回さないフィクスチャでは、
    /// **持ち越しを捨てる実装（毎ターン累積を 0 に戻す）が緑を通る**
    /// ——1ターン目の結果はどちらでも同じである
    /// （CLAUDE.md「フィクスチャがその分岐が効く状態に一度も入らない」）。
    fn run_turns_tuned(
        sub: &Arc<MockSub>,
        user_messages: &[&str],
        stat_drift: StatDriftMode,
        form: DriftForm,
        tuning: DriftTuning,
    ) -> Ran {
        // **既定は落とし所の窓**（この配線は引き直しを持たない）。
        run_turns_in_window(
            sub,
            user_messages,
            stat_drift,
            None,
            form,
            tuning,
            SubContext::FALLBACK,
        )
    }

    /// **キャラクターの走らせ方を書いて**1往復走らせる（ADR-0045 / 作業単位 A-8）。
    ///
    /// `default` は抽出器に渡す起動時の既定、`chosen` はキャラクターに書いた値。
    fn run_once_as(
        sub: &Arc<MockSub>,
        user_message: &str,
        default: StatDriftMode,
        chosen: Option<StatDriftMode>,
    ) -> Ran {
        run_turns_in_window(
            sub,
            &[user_message],
            default,
            chosen,
            DriftForm::Single,
            fixture_tuning(),
            SubContext::FALLBACK,
        )
    }

    /// **サブモデルの窓を指定して1往復走らせる**（作業単位 B-7 / レビュー指摘3）。
    ///
    /// 変容の上限が**その往復の窓**から出ていることを見るための入口。
    fn run_once_with_window(sub: &Arc<MockSub>, user_message: &str, window: SubContext) -> Ran {
        run_turns_in_window(
            sub,
            &[user_message],
            StatDriftMode::On,
            None,
            DriftForm::Single,
            fixture_tuning(),
            window,
        )
    }

    /// 窓まで指定して往復を続けて走らせる実体。
    #[allow(clippy::too_many_arguments)]
    fn run_turns_in_window(
        sub: &Arc<MockSub>,
        user_messages: &[&str],
        stat_drift: StatDriftMode,
        // **キャラクターに書いた走らせ方**（ADR-0045）。`None` は未設定＝`stat_drift` に従う。
        chosen: Option<StatDriftMode>,
        form: DriftForm,
        tuning: DriftTuning,
        window: SubContext,
    ) -> Ran {
        assert!(!user_messages.is_empty(), "1往復は走らせること");
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();

        // **準備は観測窓の外で済ませる**（キャラクター作成の記録を混ぜない）。
        let fixture = runtime.block_on(fresh(&dir.path().join("drift.sqlite")));
        let llm: Arc<dyn LlmClient> = Arc::clone(sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **窓は呼び出し側が決める**（既定は落とし所 / 作業単位 B-7）。
            super::super::context::SharedSubContext::for_tests(window),
            stat_drift,
            form,
            DriftNote::None,
            tuning,
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        // **往復の入口が読むのはキャラクターの行**（`Extraction::new`）なので、
        // 書いた値はここで行に載せる（DB の列を経由する経路は統合テストが見る）。
        let character = Character {
            stat_drift_mode: chosen,
            ..fixture.character.clone()
        };
        let extraction = Extraction::new(Arc::clone(&extractor), &character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                for user_message in user_messages {
                    extraction.start(
                        fixture.session_id,
                        crate::domain::MessageId::from_row(2),
                        user_message,
                        REPLY,
                    );
                    // **1本ずつ待つ。** 待たないと2本目が `Busy` で飛ぶ（§7）。
                    extractor.wait_for_the_running_task().await;
                }
            });
        });

        let mine = runtime.block_on(stats(&fixture.store, fixture.character.id));
        let theirs = runtime.block_on(stats(&fixture.store, fixture.bystander.id));
        let my_votes = runtime.block_on(votes(&fixture.store, fixture.character.id));
        let their_votes = runtime.block_on(votes(&fixture.store, fixture.bystander.id));
        let my_charges = runtime.block_on(charges(&fixture.store, fixture.character.id));
        runtime.block_on(fixture.store.close());

        Ran {
            output,
            stats: mine,
            bystander_stats: theirs,
            votes: my_votes,
            bystander_votes: their_votes,
            charges: my_charges,
            session_id: fixture.session_id.get(),
            character_id: fixture.character.id.get(),
        }
    }

    /// 保存済みの累積を `(軸名, 値)` の組で、**軸名順**に読む。
    async fn charges(store: &Store, id: CharacterId) -> Vec<(String, f64)> {
        let mut rows: Vec<(String, f64)> = store
            .drift_charges(id)
            .await
            .expect("累積を読めること")
            .into_iter()
            .map(|stored| (stored.axis().as_str().to_owned(), stored.charge()))
            .collect();
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        rows
    }

    /// 累積を軸名で引く。**行が無ければ `None`。**
    fn charge_of(ran: &Ran, axis: &str) -> Option<f64> {
        ran.charges
            .iter()
            .find(|(name, _)| name == axis)
            .map(|(_, charge)| *charge)
    }

    /// 変容の記録に出る軸の欄の綴り（`Debug` なので引用符が付く）。
    fn axis_field(axis: &str) -> String {
        format!("drift_axis={axis:?}")
    }

    // ---------------------------------------------------------------------
    // 記録の**行**を選ぶ（作業単位 B-21c）
    // ---------------------------------------------------------------------

    /// **変容を判定した回の記録**（[`record_run`]）の**行**を1本取り出す。
    ///
    /// # なぜ捕捉した塊のまま欄を見てはいけないのか
    ///
    /// [`has_field`] は `split_whitespace` で欄を切るので**改行をまたぐ。**
    /// 捕捉した出力を丸ごと渡すと、主張は
    /// 「**その欄がその値を持つ行が、出力のどこかに1本ある**」に化けており、
    /// 「**その行がこう言っている**」を一切見ていない
    /// （CLAUDE.md「その行が出たと、その行がこう言っているは別の主張である」）。
    ///
    /// **机上の話ではない。実演されている**（2026-08-29 / B-21c の出発点）——
    /// [`record_run`] の欄名を潰す変異は赤になるが、
    /// **飛ばし記録の枝へ変容の結末の欄を1本足すと緑に戻る。**
    /// 変容の記録は壊れたまま、別のイベントの行が主張を満たす。
    ///
    /// # 鍵は**イベント名**であって、主張したい欄ではない
    ///
    /// [`RAN_MESSAGE`] は `tracing` のメッセージ（地の文）なので
    /// [`line_containing`] で選ぶ。**主張したい欄そのものを鍵にすると循環する**
    /// ——囮の行を掴んだまま、その行で主張が成立する。
    /// 欄を鍵に選ぶ場面では `crate::testlog` の `line_with_field` を使うこと。
    ///
    /// # 同じイベントが2本あれば、**最初の1本**を返す
    ///
    /// `lines()` を先頭から探すので、**多ターンのフィクスチャ**
    /// （[`run_turns_tuned`] に発言を複数渡す形）では**1ターン目の記録**になる。
    /// **いまこの道具を使う多ターンのテストは無い**が、
    /// **向きを未規定のままにしない**（`crate::testlog` の `line_with_field` と同じ扱い）。
    ///
    /// # 見張り番は2本
    ///
    /// | 見るもの | 見張り番 |
    /// |---|---|
    /// | 囮の行を掴まないこと（**両方の順番で**） | [`a_decoy_event_carrying_the_same_field_is_not_the_drift_record`] |
    /// | 同じイベントが2本あるとき**先に出た方**を返すこと | [`the_record_tools_return_the_first_line_that_carries_their_event`] |
    ///
    /// # ⚠ **包まない場所が2種類ある**（B-21c で1箇所ずつ判断した）
    ///
    /// | 形 | 塊のまま渡す理由 |
    /// |---|---|
    /// | **否定**（`!has_field(塊, …)`） | `crate::testlog` の `SharedBuffer::contents` と `has_field` の doc が権威 |
    /// | **他のモジュールの欄を見る前提**（`skipped=…` は `server::extract` の欄） | あちらの `SKIPPED_MESSAGE` が `pub` ではないので、いま鍵にできる綴りが無い。前提なので広げていない |
    ///
    /// **どちらの理由もここに書き写さないこと**（片方を直すと、もう片方が黙って古びる）。
    fn drift_record(output: &str) -> &str {
        line_containing(output, RAN_MESSAGE)
    }

    /// **走らせなかった回の記録**（[`record_skip`]）の**行**を1本取り出す。
    ///
    /// [`drift_record`] と同じ理由・同じ約束（**最初の1本** / 同じ見張り番2本）。
    /// **`drift_run_mode` は判定した回にも飛ばした回にも出る**ので、
    /// どちらの行の話かを鍵で決めないと、取り違えても緑のままになる。
    ///
    /// **⚠ 飛ばした回の欄を鍵にして [`line_containing`] で選ばないこと**
    /// ——あちらは素の部分一致なので、**キーの側が伸びた行**や
    /// **値の側が伸びた行**を掴む。**同じファイルの中で接頭辞が伸びる分には、
    /// [`the_drift_fields_never_collide_with_another_module`] は掛からない。**
    fn skip_record(output: &str) -> &str {
        line_containing(output, SKIPPED_MESSAGE)
    }

    /// **`server::extract` の飛ばし記録**の**行**を1本取り出す（作業単位 B-21d）。
    ///
    /// # B-21c が「鍵にできる綴りが無い」と書いた理由は、もう無い
    ///
    /// あちらは `skipped=` を**塊のまま**見て「前提だから広げていない」と書いていたが、
    /// **`server::extract` の `SKIPPED_MESSAGE` を `pub(in crate::server)` にすれば届く**
    /// （`NO_FINISH_REASON_MESSAGE` に前例があった / B-21d のレビュー指摘 / 2026-08-29）。
    ///
    /// **前提であっても、塊のまま欄を見れば「どこかの行に載っている」しか言っていない**
    /// ——変容の記録の側にその欄が1本載れば、抽出が飛んでいなくても満たされる。
    ///
    /// # ⚠ このファイルの [`skip_record`] とは**別のイベント**
    ///
    /// あちらは**変容**を飛ばした記録、こちらは**抽出**を飛ばした記録。
    /// **どちらも `drift_run_mode` や `session_id` を持つ同じターンの行なので、
    /// 名前で取り違えないこと。**
    ///
    /// # 見張り番（**B-21e で足した。B-21d の時点では1本も無かった**）
    ///
    /// [`the_record_tools_return_the_first_line_that_carries_their_event`] ——
    /// **「行を返していること」「先に出た方を返すこと」「鍵を緩めていないこと」を同時に固定する。**
    ///
    /// **実測**（2026-09-02）。**塞ぐまでに2段あった:**
    ///
    /// | 変異 | 見張り番が無かった頃 |
    /// |---|---|
    /// | 本体を `output`（塊）に置き換える | **1802 全緑**（[`drift_record`] / [`skip_record`] の同じ変異は 2 赤） |
    /// | 鍵を共有接頭辞 `"記憶"` へ**緩める** | **1802 全緑**（行を2本置いただけの版でも緑のまま） |
    /// | 鍵を変容側の [`SKIPPED_MESSAGE`] へ**取り違える** | **3 赤**（元から塞がっていた） |
    ///
    /// 1つ目は、呼び出し側が `has_field(.., "skipped=…")` だけで、
    /// **`split_whitespace` が改行をまたぐ**ため塊でも通っていたから。
    /// 2つ目の理由と塞ぎ方は、見張り番の doc に書いてある。
    fn extraction_skip_record(output: &str) -> &str {
        line_containing(output, super::super::extract::SKIPPED_MESSAGE)
    }

    /// **囮のイベントが同じ欄・同じ値を出しても、変容の記録としては読まない**（B-21c）。
    ///
    /// **この作業単位の存在理由そのもの。**
    ///
    /// # 囮の形は、実演されたものと同じ
    ///
    /// **飛ばし記録の行が、変容の結末の欄を持つ**——
    /// 2026-08-29 にレビュアーが実際に1本足した形をそのまま置いてある
    /// （CLAUDE.md「囮は、実際に狙っている相手と同じ形でなければ意味がない」）。
    ///
    /// # ⚠ **順番の両方で測る**（B-21c のレビュー指摘2）
    ///
    /// [`RAN_MESSAGE`] と [`SKIPPED_MESSAGE`] は
    /// **接頭辞「性格の変容を判定し」を共有する**ので、
    /// **鍵をその接頭辞まで緩める変異**は現実的な形である。
    /// **1つの順番でしか測らないと片側しか赤にならない** ——
    /// 囮が先なら [`drift_record`] の緩みだけが、記録が先なら
    /// [`skip_record`] の緩みだけが捕まる。**両方の順番を同じ主張に通す。**
    ///
    /// # 入力は**植えたソース**（`tracing` を通さない）
    ///
    /// 実際に走らせて捕まえた出力では、この囮の行は作れない
    /// ——**本番のコードが出さない行だから。**
    /// ここは道具（[`drift_record`] / [`skip_record`]）を、
    /// 実ログとまったく同じ判定に通すための対照である
    /// （CLAUDE.md「対照は、植えたソースを走査本体に通すこと」）。
    ///
    /// # 期待値は道具を通さないべた書き。**否定は両方の道具に置く**
    ///
    /// 肯定だけだと、**塊をそのまま返す実装が囮の行で満たされて緑になる**
    /// ——B-21c のレビューが [`skip_record`] の側でそれを実測した
    /// （`assert_ne!` は「同じ行を返している」しか見ておらず、
    /// 「**行を返している**」を見ていないので、そこも通り抜ける）。
    #[test]
    fn a_decoy_event_carrying_the_same_field_is_not_the_drift_record() {
        // **判定の記録**（本物）。
        let record = format!(
            "2026-08-29T00:00:00.000001Z  INFO planted::record: \
             {RAN_MESSAGE} drift_outcome=nothing drift_run_mode=shadow session_id=1"
        );
        // **囮** —— 飛ばし記録の行が、判定の結末の欄を持つ。
        // **両方のイベントに出る欄**（`drift_run_mode`）は、値をわざとずらす
        // ——同じ値にすると、行を取り違えても観測できない
        // （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
        let decoy = format!(
            "2026-08-29T00:00:00.000000Z  WARN planted::decoy: \
             {SKIPPED_MESSAGE} drift_skipped=upstream_failed drift_outcome=applied \
             drift_run_mode=on session_id=1"
        );

        for planted in [
            format!("{decoy}\n{record}\n"),
            format!("{record}\n{decoy}\n"),
        ] {
            // **前提: 塊のまま見れば、どちらの欄も相手の行から手に入る。**
            // ここが偽なら、下の主張は「そもそも紛れが無い」を見ているだけになる。
            assert!(
                has_field(&planted, "drift_outcome=applied"),
                "前提: 囮の欄が塊の中に在ること: {planted:?}"
            );
            assert!(
                has_field(&planted, "drift_outcome=nothing"),
                "前提: 判定の欄が塊の中に在ること: {planted:?}"
            );

            let ran = drift_record(&planted);
            assert!(
                !has_field(ran, "drift_outcome=applied"),
                "囮の行を変容の記録として読んでいる: {ran:?}"
            );
            assert!(
                has_field(ran, "drift_outcome=nothing"),
                "変容の記録の行を選べていない: {ran:?}"
            );
            assert!(
                has_field(ran, "drift_run_mode=shadow"),
                "両方に出る欄で、飛ばし記録の側を読んでいる: {ran:?}"
            );

            // **逆向きも同じ強さで見る。** 判定の行にしか無い欄で否定を1本置く
            // ——これが無いと、塊をそのまま返す実装が肯定2本を囮の行で満たして通る。
            let skipped = skip_record(&planted);
            assert!(
                has_field(skipped, "drift_skipped=upstream_failed"),
                "飛ばし記録の行を選べていない: {skipped:?}"
            );
            assert!(
                has_field(skipped, "drift_run_mode=on"),
                "両方に出る欄で、判定の記録の側を読んでいる: {skipped:?}"
            );
            assert!(
                !has_field(skipped, "drift_outcome=nothing"),
                "判定の行にしか無い欄が読めている（行に切れていない）: {skipped:?}"
            );

            // **行そのものと突き合わせる**（べた書き / 道具を通さない期待値）。
            assert_eq!(ran, record, "判定の記録の行そのものを返すこと: {planted:?}");
            assert_eq!(
                skipped, decoy,
                "飛ばし記録の行そのものを返すこと: {planted:?}"
            );
        }
    }

    /// **同じイベントが2本あれば、道具は「先に出た方」を返す**（B-21c のレビュー指摘2）。
    ///
    /// # なぜ別に要るのか
    ///
    /// 囮の見張り番は**各イベントが1本ずつ**の植えたソースを通すので、
    /// **「同じマーカーに候補が2本ある」状態に一度も入らない**
    /// （CLAUDE.md「フィクスチャがその分岐が効く状態に一度も入らない」）。
    /// **後ろから探す変異（`.rev()`）も、末尾を返す変異も、あちらでは緑を通る。**
    ///
    /// 作法は `crate::testlog` の `line_with_field` の押さえと同じ ——
    /// **同じマーカーを持つ行を2本置き、期待値は道具を通さないべた書き。**
    ///
    /// # この状態は本番のフィクスチャで起きうる
    ///
    /// 帳簿は累積を次のターンへ持ち越すので、**多ターンの入口が実在する**
    /// （[`run_turns_tuned`]）。**いまこの道具を使う多ターンのテストは無い**が、
    /// **足す人が「どちらのターンを読んでいるか」を調べずに済むように、向きを固定する。**
    ///
    /// # ⚠ **3本目の道具は、ここに写されていなかった**（作業単位 B-21e で実測 / 2026-09-02）
    ///
    /// B-21d が [`extraction_skip_record`] を足したとき、
    /// **この見張り番にも囮の見張り番にも写らなかった。**
    /// **実測: あの道具を「塊をそのまま返す」に変える変異は 1802 全緑**だった
    /// （同じ変異を [`drift_record`] / [`skip_record`] に当てると、どちらも 2 赤）。
    /// 呼び出し側は2箇所とも `has_field(.., "skipped=…")` で、
    /// **`split_whitespace` は改行をまたぐ**ので、塊でも同じ判定が通る。
    /// CLAUDE.md「写しを作ったら、列・分岐・枝の1つずつに当てること。代表1つでは足りない」。
    ///
    /// # ⚠ **鍵を「緩める」側は、行を2本置いただけでは塞がらない**（同じ回のレビュー指摘）
    ///
    /// **`記憶の抽出を行いませんでした` と `記憶を抽出しました` は接頭辞「記憶」を共有する。**
    /// **実測: 鍵を `"記憶"` へ緩める変異は、この見張り番に2本足した後でも 1802 全緑**だった
    /// ——**植えた行のうち「記憶」を含むのが抽出の飛ばし記録だけ**なら、
    /// **その族に収まる鍵は何を書いても同じ行を返す。**
    ///
    /// **だから「抽出が走った記録」の行を、飛ばし記録より前に1本置いてある**
    /// （[`super::super::extract::RAN_MESSAGE`] の doc が可視性の理由を書いている）。
    /// **鍵の取り違え**（変容側の [`SKIPPED_MESSAGE`] へ）**は元から 3 赤**で、
    /// **塞がっていなかったのは緩めの側だけ。**
    #[test]
    fn the_record_tools_return_the_first_line_that_carries_their_event() {
        let first_record = format!(
            "2026-08-29T00:00:00.000000Z  INFO planted::turn1: \
             {RAN_MESSAGE} drift_outcome=nothing session_id=1"
        );
        let first_skip = format!(
            "2026-08-29T00:00:00.000001Z  INFO planted::turn1: \
             {SKIPPED_MESSAGE} drift_skipped=no_axes session_id=1"
        );
        // **抽出が「走った」記録**（囮）。**抽出の飛ばし記録より前に置く。**
        // 抽出の2つの文言は接頭辞「記憶」を共有するので、
        // **鍵をそこまで緩めるとこちらの行が先に見つかる。**
        // **後ろに置くと、緩めても飛ばし記録の方が先に見つかって緑になる**
        // ——`a_decoy_event_carrying_the_same_field_is_not_the_drift_record` が
        // 変容側で「両方の順番」を測っているのと同じ理由。
        // **こちらの道具は1本だけ**（走った側を選ぶ道具は無い）なので、この向き1つでよい。
        let extraction_ran = format!(
            "2026-08-29T00:00:00.000002Z  INFO planted::turn1: {} \
             extracted=1 saved=1 session_id=1",
            super::super::extract::RAN_MESSAGE
        );
        // **抽出の飛ばし記録**（[`extraction_skip_record`] が選ぶ相手）。
        // **変容の2つのイベントとは別のメッセージ**なので、鍵を取り違えれば選べない。
        let first_extraction_skip = format!(
            "2026-08-29T00:00:00.000003Z  INFO planted::turn1: {} \
             skipped=memories_unreadable session_id=1",
            super::super::extract::SKIPPED_MESSAGE
        );
        let second_record = format!(
            "2026-08-29T00:00:00.000004Z  INFO planted::turn2: \
             {RAN_MESSAGE} drift_outcome=applied session_id=2"
        );
        let second_skip = format!(
            "2026-08-29T00:00:00.000005Z  WARN planted::turn2: \
             {SKIPPED_MESSAGE} drift_skipped=upstream_failed session_id=2"
        );
        let second_extraction_skip = format!(
            "2026-08-29T00:00:00.000006Z  INFO planted::turn2: {} \
             skipped=conversation_too_long session_id=2",
            super::super::extract::SKIPPED_MESSAGE
        );
        let planted = format!(
            "{first_record}\n{first_skip}\n{extraction_ran}\n{first_extraction_skip}\n\
             {second_record}\n{second_skip}\n{second_extraction_skip}\n"
        );

        assert_eq!(
            drift_record(&planted),
            first_record,
            "判定の記録が2本あるとき、先に出た方を返していない: {planted:?}"
        );
        assert_eq!(
            skip_record(&planted),
            first_skip,
            "飛ばし記録が2本あるとき、先に出た方を返していない: {planted:?}"
        );
        assert_eq!(
            extraction_skip_record(&planted),
            first_extraction_skip,
            "抽出の飛ばし記録が2本あるとき、先に出た方を返していない: {planted:?}"
        );
    }

    /// 初期状態の軸（何も動かなかったときの期待値）。
    fn untouched() -> Vec<(String, u32)> {
        vec![
            (MOVED_AXIS.to_owned(), MOVED_START),
            (TWIN_AXIS.to_owned(), MOVED_START),
            (STILL_AXIS.to_owned(), STILL_START),
            (SILENT_AXIS.to_owned(), 0),
        ]
    }

    /// 傍観者の初期状態。
    fn bystander_untouched() -> Vec<(String, u32)> {
        vec![(MOVED_AXIS.to_owned(), MOVED_START)]
    }

    // ---------------------------------------------------------------------
    // 番兵と語彙
    // ---------------------------------------------------------------------

    /// **「値なし」の番兵が、本物の値と衝突しない**（ADR-0024 §5）。
    ///
    /// # ⚠ **`0` はもう番兵に使えない**（ADR-0027 §決定4 / §決定6）
    ///
    /// 以前は「変容の下端は `1` なので動いた回の値は必ず 1 以上」を根拠に
    /// `0` を番兵にしていた。**値 0 の軸が変容の対象になった**ので、
    /// `before = 0`（ゼロから育て始めた回）が現実に起きる。
    ///
    /// いまの根拠は**尺度の上限**——配分値は `StatScale::value` でしか作れず、
    /// **`StatScale::MAX` を超えることはない。**
    #[test]
    fn the_absent_value_marker_can_never_be_a_real_value() {
        const { assert!(NO_VALUE > crate::domain::StatScale::MAX) };

        // **実際に上端の値を作って、番兵と衝突しないことを見る**（定数だけを比べない）。
        let scale = StatScale::parse(StatScale::MAX).expect("正当な尺度であること");
        let top = scale.value(StatScale::MAX).expect("上端の値が作れること");
        assert_ne!(top.get(), NO_VALUE, "上端の値が番兵と衝突した");

        // **`0` は番兵に使えない**——ゼロから育て始めた回の `before` がこれである。
        let floor = scale.value(0).expect("値 0 が作れること");
        assert_ne!(floor.get(), NO_VALUE, "値 0 が番兵と衝突した");
    }

    /// **書き込みの4つの印が、1つも重ならない。**
    ///
    /// 「動いた」「動かなかった」「書き込みまで行っていない」「書き込みが落ちた」は
    /// **全部違う話**である。重なると、故障が正常な結果に見える。
    #[test]
    fn every_write_marker_is_distinct() {
        let markers = [
            DriftWrite::Moved.label(),
            DriftWrite::NotMoved.label(),
            NO_WRITE,
            WRITE_FAILED,
        ];
        for (index, left) in markers.iter().enumerate() {
            for right in &markers[index + 1..] {
                assert_ne!(left, right, "書き込みの印が重なっています: {markers:?}");
            }
        }
    }

    /// **走らせなかった4つの理由が、1つも重ならない。**
    #[test]
    fn every_skip_reason_is_distinct() {
        let reasons = [
            DriftSkipReason::StatsUnreadable,
            DriftSkipReason::NoAxes,
            DriftSkipReason::TooLong,
            DriftSkipReason::UpstreamFailed,
        ];
        for (index, left) in reasons.iter().enumerate() {
            for right in &reasons[index + 1..] {
                assert_ne!(left.label(), right.label());
            }
        }
    }

    /// **「判定した」と「判定しなかった」を同じ行にしない。**
    ///
    /// 混ぜると提案率の分母が壊れる（[`SKIPPED_MESSAGE`] の doc）。
    #[test]
    fn the_two_event_messages_never_contain_each_other() {
        assert!(!RAN_MESSAGE.contains(SKIPPED_MESSAGE));
        assert!(!SKIPPED_MESSAGE.contains(RAN_MESSAGE));
    }

    /// **上限そのものを、式の外にある値で押さえる**（ADR-0024 / 16b のレビュー）。
    ///
    /// # なぜ「比較の側」だけでは閉じないのか（実測 / 2026-08-23）
    ///
    /// 定数を `ASSUMED_SUB_CONTEXT_TOKENS / 2` → **`ASSUMED_SUB_CONTEXT_TOKENS`**
    /// （8192 → 16384）に変える変異は **1283 passed / 0 failed で緑を通った。**
    ///
    /// 境界の2本（[`a_drift_request_exactly_at_the_limit_is_sent`] /
    /// [`a_drift_request_one_byte_over_the_limit_is_dropped`]）は、
    /// **期待値を定数から作っている**（[`user_message_filling_the_drift_limit`] が
    /// `FALLBACK_MAX_DRIFT_PROMPT_TOKENS` を読む）ので、
    /// **定数が動けば期待値も一緒に動いて緑のまま**になる。
    /// [`the_long_fixture_sits_in_the_band_where_only_the_extraction_is_rejected`] も
    /// 抽出との**相対比較**なので、両方が倍になれば帯は動かない。
    ///
    /// CLAUDE.md が名指ししている形そのもの ——
    /// **「一本化は『食い違いを消す』までで、『値が正しいこと』は別に要る。
    /// 必要だったのは関数の外にある値で押さえる見張り番1本。」**
    ///
    /// # 押さえる3つ
    ///
    /// 1. **入力（16384）** —— サブモデルの `loaded_context_length` の実測値。
    ///    `max_context_length`（262144）と取り違えると、上限が16倍になる。
    /// 2. **規則（半分）** と **導かれる数（8192）を直書きで**。
    ///    関係だけでは入力側の書き換えを止められず、
    ///    直書きだけでは規則から外れた書き換えを止められない。**両方要る。**
    /// 3. **依頼＋応答が実コンテキストに収まること。**
    ///    崩れると、LM Studio では**前方から黙って切り捨てられる** —— 最初に消えるのは指示の側なので、
    ///    **変容の依頼が「1行だけ」を言わなくなる**（`FALLBACK_MAX_DRIFT_PROMPT_TOKENS` の doc / ADR-0013）。
    ///    llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。ただし、
    ///    プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
    ///
    /// **サブモデルの落とし所を変えるときはここも一緒に直すこと**
    /// （見張り番の意味を変えるので、司令塔の採否を仰ぐ）。
    /// 抽出側の対は `super::super::extract::tests::
    /// the_fallback_prompt_limit_is_half_of_the_sub_model_context`。
    ///
    /// # **規則は2点で押さえる**（作業単位 B-7 / レビュー指摘3 / 2026-08-27）
    ///
    /// 上限が**その往復の窓**から出るようになったので、落とし所の1点だけでは
    /// **「規則」ではなく「その1点での値」しか固定できない**
    /// ——`|_| 8192` のような決め打ちが素通りする。
    /// **落とし所とは別の窓**でもう1点見る。
    #[test]
    fn the_drift_limit_is_half_of_the_sub_model_context() {
        assert_eq!(
            SubContext::FALLBACK.tokens(),
            16_384,
            "既定のサブモデルの loaded_context_length（実測）から動いた。\
             max_context_length（262144）と取り違えていないか"
        );
        assert_eq!(
            FALLBACK_MAX_DRIFT_PROMPT_TOKENS,
            SubContext::FALLBACK.tokens() / 2,
            "上限が「実コンテキストの半分」という規則から外れた"
        );
        assert_eq!(
            FALLBACK_MAX_DRIFT_PROMPT_TOKENS, 8_192,
            "上限が実測から導かれる数と違う"
        );

        // **2点目。落とし所とは別の窓**（`6144 / 2 = 3072`）。
        // **抽出と同じ窓・同じ規則**であることも、ここで一緒に押さえる
        // ——2つの上限が食い違うと、片方が捨てる依頼をもう片方が上流へ送る。
        let narrow = SubContext::new(6_144).expect("抽出に使える窓であること");
        assert_eq!(
            drift_prompt_limit(narrow),
            3_072,
            "規則は「窓の半分」であって、落とし所での値の決め打ちではない"
        );
        assert_eq!(
            drift_prompt_limit(narrow),
            narrow.prompt_limit(),
            "変容と抽出が同じ窓から同じ上限を出すこと"
        );

        // **応答の頭打ちも窓から出る。** ここが定数のままだと、
        // 狭い窓では**依頼と応答の合計が窓を超える**
        // （変異検査で実測 / 2026-08-27。この2本を足すまで緑を通った）。
        assert_eq!(
            drift_response_cap(narrow),
            3_072,
            "頭打ちは窓の残り（6144 − 3072）であること"
        );
        // **軸が多い回**は頭打ちが効く。**落とし所の 8192 を出さないこと。**
        assert_eq!(
            drift_response_tokens(DriftForm::All, 1_000, narrow),
            3_072,
            "軸が多くても、その窓の残りを超えて要求しないこと"
        );

        // **依頼＋応答が窓に収まること。** 軸が1本の回と、頭打ちが効く回の両方で見る
        // ——1本だけだと `86` トークンしか要求しないので、頭打ちの取り違えが観測できない。
        for window in [SubContext::FALLBACK, narrow] {
            for (form, axes) in [(DriftForm::Single, 1), (DriftForm::All, 1_000)] {
                assert!(
                    drift_prompt_limit(window)
                        .saturating_add(drift_response_tokens(form, axes, window))
                        <= window.tokens(),
                    "依頼と応答の合計が実コンテキストを超える（LM Studio は前方から黙って切り捨て、\
                     llama は 400 で断るか応答が黙って短くなる）: \
                     形 = {form:?}, prompt = {}, response = {}, context = {}",
                    drift_prompt_limit(window),
                    drift_response_tokens(form, axes, window),
                    window.tokens()
                );
            }
        }
    }

    // ---------------------------------------------------------------------
    // 走らせ方（ADR-0024 §2）
    // ---------------------------------------------------------------------

    /// **スイッチが off なら、上流を1回しか呼ばない**（ADR-0024 §8）。
    ///
    /// 呼んでから結果を捨てる形だと、**off でも GPU を使う。**
    ///
    /// **票も残らない**（ADR-0026 §決定3）——`run_drift` そのものが呼ばれないので、
    /// 記録の経路に入らない。3値のうちここだけが「何も起きない」側。
    #[test]
    fn the_switch_being_off_never_calls_the_upstream_a_second_time() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let ran = run_once_with(&sub, USER_MESSAGE, StatDriftMode::Off);

        assert_eq!(sub.call_count(), 1, "抽出の1回だけであること");
        assert!(
            !ran.output.contains(RAN_MESSAGE) && !ran.output.contains(SKIPPED_MESSAGE),
            "変容の記録を出さないこと: {:?}",
            ran.output
        );
        assert_eq!(ran.stats, untouched(), "値が動かないこと");
        assert!(ran.votes.is_empty(), "票も残らないこと: {:?}", ran.votes);
    }

    /// **スイッチが on なら、抽出の直後に2回目を呼ぶ**（ADR-0024 §2）。
    #[test]
    fn the_switch_being_on_asks_the_upstream_a_second_time() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let ran = run_once(&sub, USER_MESSAGE);

        assert_eq!(sub.call_count(), 2, "抽出と変容で2回であること");
        assert!(
            ran.output.contains(RAN_MESSAGE),
            "判定した回として記録すること: {:?}",
            ran.output
        );
    }

    /// **抽出が `Busy` で飛んだターンは、変容も飛ぶ**（ADR-0024 §2）。
    ///
    /// 走っている1本があるとき、2本目のターンは `spawn` が弾く。
    /// **変容はそのタスクの中にあるので、一緒に飛ぶ**——ここが
    /// 「飛ぶのは `Busy` のときだけ」の唯一の例外。
    #[test]
    fn a_busy_turn_skips_the_drift_as_well() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("busy.sqlite")));
        let (sub, _open) = MockSub::gating_the_extraction();
        let extractor = extractor_with_drift(&fixture.store, &sub);
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                // 1本目は開かないので走り続ける（抽出の応答待ちで止まる）。
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while sub.call_count() == 0 {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "1本目が上流へ届くこと"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                // 2本目。**`Busy` で弾かれる。**
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
            });
        });
        extractor.shutdown();
        runtime.block_on(fixture.store.close());

        assert_eq!(sub.call_count(), 1, "前提: 1本目の抽出だけが上流へ行くこと");
        assert!(
            !output.contains(RAN_MESSAGE) && !output.contains(SKIPPED_MESSAGE),
            "飛んだターンでは変容の記録が出ないこと: {output:?}"
        );
    }

    /// **記憶が読めなくて抽出が飛んだターンでも、変容は走る**（ADR-0024 §2）。
    ///
    /// **変容は記憶を読まない。** 抽出の事情に巻き込む理由が無い。
    ///
    /// 壊れた行は「本文が空白のみ」で作る（長さと空白の制約はスキーマに無いので
    /// 素の INSERT で作れる。`tests/common` の同種のヘルパと同じ判断）。
    #[test]
    fn a_turn_whose_memories_cannot_be_read_still_drifts() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("unreadable.sqlite")));

        // **読み出しが落ちる行を1件置く**（`recall_memories` が `InvalidStoredValue`）。
        runtime.block_on(async {
            sqlx::query("INSERT INTO memory (character_id, subject, content) VALUES (?, ?, ?)")
                .bind(fixture.character.id.get())
                .bind("user")
                .bind("   ")
                .execute(fixture.store.pool())
                .await
                .expect("壊れた行を作れること");
        });
        assert!(
            runtime
                .block_on(fixture.store.recall_memories(fixture.character.id))
                .is_err(),
            "前提: 記憶が読めない状態であること"
        );

        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::On,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                extractor.wait_for_the_running_task().await;
            });
        });

        let after = runtime.block_on(stats(&fixture.store, fixture.character.id));
        runtime.block_on(fixture.store.close());

        assert_eq!(
            sub.call_count(),
            1,
            "前提: 抽出は上流へ行かないこと（記憶が読めない）: {output:?}"
        );
        // **前提でも行に切る**（作業単位 B-21d のレビュー指摘 / 2026-08-29）。
        // B-21c は「`skipped=` は `server::extract` の欄で、鍵にできる綴りが無い」と
        // 書いて塊のまま渡していたが、**あちらの `SKIPPED_MESSAGE` を
        // `pub(in crate::server)` にすれば届く**（`NO_FINISH_REASON_MESSAGE` に前例があった）。
        assert!(
            has_field(
                extraction_skip_record(&output),
                "skipped=memories_unreadable"
            ),
            "前提: 抽出が記憶を読めずに飛んだ回であること: {output:?}"
        );
        assert!(
            output.contains(RAN_MESSAGE)
                && has_field(drift_record(&output), "drift_outcome=applied"),
            "それでも変容は走ること: {output:?}"
        );
        assert_eq!(
            after,
            vec![
                (MOVED_AXIS.to_owned(), MOVED_START + 1),
                (TWIN_AXIS.to_owned(), MOVED_START),
                (STILL_AXIS.to_owned(), STILL_START),
                (SILENT_AXIS.to_owned(), 0),
            ],
            "値が実際に動くこと"
        );
    }

    /// **抽出の依頼が長すぎて飛んだターンでも、変容は走る**（ADR-0024 §2）。
    ///
    /// # フィクスチャの設計
    ///
    /// **変容の依頼は抽出より削れない枠が小さい**（記憶の節も手本の行数も無い）ので、
    /// **抽出だけが上限を超える会話の長さが存在する。**
    /// 帯の広さは [`the_drift_request_is_smaller_than_the_extraction_request`] が
    /// 実測で押さえており、その中央あたりを取ってある。
    /// **帯が消えたらそちらが赤になる**ので、ここが黙って古びることはない。
    #[test]
    fn a_turn_whose_extraction_request_is_too_long_still_drifts() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let ran = run_once(&sub, &long_message_only_the_extraction_rejects());

        assert_eq!(
            sub.call_count(),
            1,
            "前提: 抽出は上流へ行かないこと（長すぎる）: {:?}",
            ran.output
        );
        // **前提でも行に切る**（上と同じ / B-21d のレビュー指摘）。
        assert!(
            has_field(
                extraction_skip_record(&ran.output),
                "skipped=conversation_too_long"
            ),
            "前提: 抽出が長さで飛んだ回であること: {:?}",
            ran.output
        );
        assert!(
            ran.output.contains(RAN_MESSAGE)
                && has_field(drift_record(&ran.output), "drift_outcome=applied"),
            "それでも変容は走ること: {:?}",
            ran.output
        );
        assert_eq!(
            ran.stats,
            vec![
                (MOVED_AXIS.to_owned(), MOVED_START + 1),
                (TWIN_AXIS.to_owned(), MOVED_START),
                (STILL_AXIS.to_owned(), STILL_START),
                (SILENT_AXIS.to_owned(), 0),
            ],
            "値が実際に動くこと"
        );
    }

    /// **変容の依頼が長すぎたら、変容も捨てる**（変容は自分の予算判定を持つ）。
    ///
    /// **しきい値そのものはここでは固定していない**（入力は上限の3倍の大きさ）。
    /// 判定の境界は [`a_drift_request_exactly_at_the_limit_is_sent`] と
    /// [`a_drift_request_one_byte_over_the_limit_is_dropped`] が1バイト単位で挟む。
    #[test]
    fn a_request_over_the_drift_limit_is_dropped_and_counted() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        // 上限を確実に超える会話（`estimate_tokens` は UTF-8 バイト数以上）。
        let ran = run_once(
            &sub,
            &"あ".repeat(FALLBACK_MAX_DRIFT_PROMPT_TOKENS as usize),
        );

        assert_eq!(sub.call_count(), 0, "長すぎる会話は上流へ送らないこと");
        assert!(
            has_field(
                skip_record(&ran.output),
                &format!("drift_skipped={}", DriftSkipReason::TooLong.label())
            ),
            "捨てたことを理由つきで数えること: {:?}",
            ran.output
        );
        assert!(
            has_field(
                skip_record(&ran.output),
                &format!("drift_limit_tokens={FALLBACK_MAX_DRIFT_PROMPT_TOKENS}")
            ),
            "上限を並べて出すこと: {:?}",
            ran.output
        );
        assert_eq!(ran.stats, untouched(), "値が動かないこと");
    }

    /// [`run_once`] が組むはずの変容の依頼を、テスト側で同じ形に組んだもの。
    ///
    /// **呼び名・軸・尺度はフィクスチャと同じ**——違う値にすると
    /// 境界の計算が実機の依頼とずれる
    /// （[`a_drift_request_exactly_at_the_limit_is_sent`] が実物と突き合わせて捕まえる）。
    fn expected_drift_request(user_message: &str) -> Vec<ChatMessage> {
        // **[`run_once`] が渡す形**（[`run_once_in`] を見よ）。ここを既定に寄せない
        // ——境界の2本は「実機が送った依頼と1バイト違わないこと」を見るので、
        // **実機の側が使っている形と揃っていなければならない。**
        expected_drift_request_in(DriftForm::Single, user_message)
    }

    /// 形を指定して同じ依頼を組む。**枠の大きさを形ごとに測るために要る**（作業単位 19）。
    fn expected_drift_request_in(form: DriftForm, user_message: &str) -> Vec<ChatMessage> {
        let display_name = DisplayName::parse(DISPLAY_NAME).expect("正当な表示名であること");
        build_drift_prompt(&DriftInput {
            display_name: &display_name,
            stats: &allocs(
                SCALE,
                &[
                    (MOVED_AXIS, MOVED_START),
                    (TWIN_AXIS, MOVED_START),
                    (STILL_AXIS, STILL_START),
                    (SILENT_AXIS, 0),
                ],
            ),
            form,
            note: DriftNote::None,
            scale: StatScale::parse(SCALE).expect("正当な尺度であること"),
            user_message,
            character_message: REPLY,
        })
        .expect("軸が載るので依頼が作られること")
        .into_messages()
    }

    /// 依頼を [`MockSub::drift_prompt`] と同じ形に畳んだもの。
    fn joined(messages: &[ChatMessage]) -> String {
        messages
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 変容の依頼の大きさ（トークン）。**判定とまったく同じ関数**で数える。
    ///
    /// **式をここに書き写さない**（13c-3a の是正と同じ理由）。
    fn tokens_of_drift_request(user_message: &str) -> u32 {
        request_tokens(&expected_drift_request(user_message))
    }

    /// 依頼がちょうど [`FALLBACK_MAX_DRIFT_PROMPT_TOKENS`] に届く利用者の発言。
    ///
    /// # **`estimate_tokens` が UTF-8 バイト数であることに依存している**
    ///
    /// 「会話に1バイト足すと依頼が1トークン増える」という関係を使い、
    /// **枠（会話の中身以外）を実行時に測って**残りを1バイト文字で埋める。
    /// 枠を直書きすると、指示を1行足した日に静かに嘘になる。
    /// 前提は下の `assert_eq!` が固定しているので、**定義が変われば赤くなる。**
    fn user_message_filling_the_drift_limit() -> String {
        let one = tokens_of_drift_request("a");
        let two = tokens_of_drift_request("aa");
        assert_eq!(
            two,
            one + 1,
            "前提: 会話の1バイトが依頼の1トークンとして数えられること"
        );

        let frame = one - 1;
        assert!(
            frame < FALLBACK_MAX_DRIFT_PROMPT_TOKENS,
            "前提: 枠だけで上限を超えていないこと: frame = {frame}, \
             limit = {FALLBACK_MAX_DRIFT_PROMPT_TOKENS}"
        );
        "a".repeat((FALLBACK_MAX_DRIFT_PROMPT_TOKENS - frame) as usize)
    }

    /// 形ごとの**枠**（会話の中身以外）の大きさ。[`user_message_filling_the_drift_limit`] と同じ数え方。
    fn drift_frame_tokens(form: DriftForm) -> u32 {
        request_tokens(&expected_drift_request_in(form, "a")) - 1
    }

    /// **既定の形（`all`）でも、枠だけで予算を食い切らない**（作業単位 19）。
    ///
    /// # なぜ境界の2本では足りないのか
    ///
    /// [`a_drift_request_exactly_at_the_limit_is_sent`] と
    /// [`a_drift_request_one_byte_over_the_limit_is_dropped`] は
    /// **[`run_once`] が渡す [`DriftForm::Single`] の枠**で残りを埋める。
    /// **既定が `all` へ移った**（ADR-0027 §決定8 / 16j）ので、
    /// **あの2本は非既定の形の枠を測っている。**
    ///
    /// **`all` の依頼は長い**——全軸を並べ、全軸ぶんの行を書かせ、`0` の手本まで載る。
    /// **枠が上限を食い切ると、その形のすべてのターンが `TooLong` で捨てられ、
    /// 運用者からは「変容が動かない」としか見えない**（`drift_skipped` はログにしか出ない）。
    ///
    /// # 2つの形が別の大きさであることも見る
    ///
    /// 同値だと、**形を読み捨てて常に同じ依頼を組む実装**が観測できない
    /// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
    /// **`all` の方が大きい**ことまで向きを固定する。
    #[test]
    fn the_default_request_form_leaves_room_for_the_conversation() {
        let single = drift_frame_tokens(DriftForm::Single);
        let all = drift_frame_tokens(DriftForm::All);

        assert!(
            all > single,
            "全軸採点の枠が1軸を尋ねる形より大きくない（形が読み捨てられている）: \
             all = {all} / single = {single}"
        );
        assert!(
            all < FALLBACK_MAX_DRIFT_PROMPT_TOKENS,
            "既定の形の枠だけで上限を超えた（全ターンが捨てられる）: \
             all = {all} / limit = {FALLBACK_MAX_DRIFT_PROMPT_TOKENS}"
        );

        // **「収まる」だけでは弱い。会話の側に実用的な余地が残ること。**
        // **半分は関数の外から来た数**——上限の半分を会話に使えなければ、
        // **枠が予算を食い過ぎている**と判断する。
        assert!(
            all < FALLBACK_MAX_DRIFT_PROMPT_TOKENS / 2,
            "既定の形の枠が予算の半分以上を占めた: all = {all} / limit = {FALLBACK_MAX_DRIFT_PROMPT_TOKENS}"
        );
    }

    /// **上限ちょうどの依頼は送る**（境界の内側 / `>` であって `>=` ではない）。
    ///
    /// # なぜ1バイト単位で押さえるのか（変異検査で見つかった穴 / 2026-08-23）
    ///
    /// 「上限の3倍で捨てる」1本しか無かったとき、判定を
    /// `tokens > FALLBACK_MAX_DRIFT_PROMPT_TOKENS.saturating_mul(2)` に緩める変異が
    /// **1280 passed / 0 failed で通った**（実測）。
    /// **抽出側で 2026-08-21 に同じ変異が通ったのとまったく同じ形**である
    /// （`super::super::extract::FALLBACK_EXTRACTION_PROMPT_TOKENS` の doc）——
    /// **doc がその穴を名指ししているのに、写したのは判定だけで見張り番を写さなかった。**
    ///
    /// 緩んだ実装は 16384 トークンまで送るので、**窓を超えた依頼を送る。**
    /// LM Studio では前方から黙って切り捨てられ（ADR-0013）、最初に消えるのは指示の側なので、
    /// **変容の依頼が「1行だけ」を言わなくなる。** llama は 400 `exceed_context_size_error` で
    /// 断るか、プロンプト ＋ 応答の上限が窓を超えて応答が黙って短くなる（ADR-0052 M5）。
    #[test]
    fn a_drift_request_exactly_at_the_limit_is_sent() {
        let message = user_message_filling_the_drift_limit();
        assert_eq!(
            tokens_of_drift_request(&message),
            FALLBACK_MAX_DRIFT_PROMPT_TOKENS,
            "前提: ちょうど上限の依頼を組めていること"
        );

        let sub = MockSub::replying("なし");
        let ran = run_once(&sub, &message);

        assert!(
            !ran.output.contains(SKIPPED_MESSAGE),
            "上限ちょうどを「判定しなかった」と数えないこと: {:?}",
            ran.output
        );
        assert!(
            ran.output.contains(RAN_MESSAGE),
            "上限ちょうどは上流へ行くこと（判定は `>` であって `>=` ではない）: {:?}",
            ran.output
        );
        // **実際に送られた依頼が、境界の計算に使ったものと1バイト違わないこと。**
        assert_eq!(
            sub.drift_prompt(),
            joined(&expected_drift_request(&message)),
            "実機の依頼が、境界の計算に使った依頼と違う"
        );
    }

    /// **上限を1トークン超えたら捨てる**（境界の外側 / 対は上の1本）。
    ///
    /// 超過分がログに出ることまで見る——**上限が現実に合っているかを判断する材料**が
    /// `prompt_tokens` と `limit_tokens` の並びしかない（[`record_skip`]）。
    #[test]
    fn a_drift_request_one_byte_over_the_limit_is_dropped() {
        let message = format!("{}a", user_message_filling_the_drift_limit());
        assert_eq!(
            tokens_of_drift_request(&message),
            FALLBACK_MAX_DRIFT_PROMPT_TOKENS + 1,
            "前提: 上限を1トークンだけ超える依頼を組めていること"
        );

        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let ran = run_once(&sub, &message);

        assert!(
            has_field(
                skip_record(&ran.output),
                &format!("drift_skipped={}", DriftSkipReason::TooLong.label())
            ),
            "1トークン超えたら捨てること: {:?}",
            ran.output
        );
        // **数えた大きさが本当に上限+1であること**（枠の計算が実機と合っている証拠）。
        assert!(
            has_field(
                skip_record(&ran.output),
                &format!(
                    "drift_prompt_tokens={}",
                    FALLBACK_MAX_DRIFT_PROMPT_TOKENS + 1
                )
            ),
            "超過分が読める形で出ていること: {:?}",
            ran.output
        );
        assert_eq!(ran.stats, untouched(), "値が動かないこと");
    }

    // ---------------------------------------------------------------------
    // 依頼の中身（帰属）
    // ---------------------------------------------------------------------

    /// **依頼に載る上限は、このキャラクター自身の尺度**（ADR-0015 / 帰属）。
    ///
    /// 3つの尺度（このキャラクター 7 / 傍観者 5 / 既定 10）を**わざとずらして**あるので、
    /// 「別のキャラクターの尺度を読む」も「常に既定を返す」も落ちる。
    #[test]
    fn the_drift_request_shows_this_characters_own_scale() {
        let sub = MockSub::replying("なし");
        let _ran = run_once(&sub, USER_MESSAGE);

        let prompt = sub.drift_prompt();
        assert!(
            prompt.contains(&format!("{MOVED_AXIS}: {MOVED_START}/{SCALE}")),
            "自分の尺度が上限として載ること: {prompt:?}"
        );
        assert!(
            !prompt.contains(&format!("/{BYSTANDER_SCALE}")),
            "別のキャラクターの尺度を載せないこと: {prompt:?}"
        );
        assert!(
            !prompt.contains(&format!("/{}", StatScale::DEFAULT.get())),
            "既定の尺度を載せないこと: {prompt:?}"
        );
    }

    /// **値 0 の軸も依頼に載る**（ADR-0027 §決定4）。
    ///
    /// # ⚠ このテストは意味が変わった（16e-a）
    ///
    /// 旧名は `the_drift_request_leaves_out_the_silent_axis`。
    /// **本文プロンプトの絶対制約はそのまま**で、変わったのは**変容の依頼だけ**
    /// ——`prompt::drift::stat_lines` の doc に理由がある
    /// （制約は「演技する側」の問題で、変容のサブLLM は演技していない）。
    ///
    /// **載る軸の本数もログに出る**（提案率を読むときの分母の材料）。
    #[test]
    fn the_drift_request_carries_every_stored_axis_including_the_silent_one() {
        let sub = MockSub::replying("なし");
        let ran = run_once(&sub, USER_MESSAGE);

        let prompt = sub.drift_prompt();
        for axis in [MOVED_AXIS, STILL_AXIS, SILENT_AXIS] {
            assert!(prompt.contains(axis), "軸が載っていない: {axis} {prompt:?}");
        }
        assert!(
            prompt.contains(&format!("{SILENT_AXIS}: 0/{SCALE}")),
            "値 0 の軸が「値/上限」の形で載っていない: {prompt:?}"
        );
        assert!(
            has_field(drift_record(&ran.output), "drift_axes=4"),
            "載せた軸の本数が読めること: {:?}",
            ran.output
        );
    }

    /// **2つの発言が、それぞれ自分の小見出しの下に載る**（話者の帰属）。
    ///
    /// 入れ替えると、変容サブLLM は**利用者の発言を自分の発言として読み、
    /// その内容で自分の性格を動かす**（CLAUDE.md「抜けるのはいつも帰属」）。
    /// **変容は本文に出ない**（ADR-0024 §9）ので、ログの数だけを見ても正常に見える。
    ///
    /// **`contains` では捕まえられない**——入れ替えてもどちらの文字列も
    /// 依頼の「どこかには」在る。だから**位置**で見る。
    #[test]
    fn each_turn_sits_under_its_own_heading_in_the_drift_request() {
        /// 利用者の発言が載る小見出し。**`prompt::drift` の定数を呼ばず期待値として持つ**
        /// （呼ぶと綴りを変える変異が両側で同時に動いて緑になる）。
        const USER_TURN_HEADING: &str = "## 利用者の発言";
        /// キャラクターの応答が載る小見出し。利用者側とは別の綴り。
        const CHARACTER_TURN_HEADING: &str = "## あなたの発言";

        assert_ne!(USER_MESSAGE, REPLY, "前提: 2つの発言が別の文字列であること");

        let sub = MockSub::replying("なし");
        let _ran = run_once(&sub, USER_MESSAGE);
        let prompt = sub.drift_prompt();

        let position = |needle: &str| {
            prompt
                .find(needle)
                .unwrap_or_else(|| panic!("{needle:?} が依頼に載ること: {prompt:?}"))
        };
        let user_heading = position(USER_TURN_HEADING);
        let character_heading = position(CHARACTER_TURN_HEADING);
        let user_at = position(USER_MESSAGE);
        let reply_at = position(REPLY);

        assert!(
            user_heading < user_at && user_at < character_heading,
            "利用者の発言は `{USER_TURN_HEADING}` の下に載ること: {prompt:?}"
        );
        assert!(
            character_heading < reply_at,
            "応答は `{CHARACTER_TURN_HEADING}` の下に載ること: {prompt:?}"
        );
    }

    /// **依頼が実際に運ぶ上限は、最悪の1行ちょうど**（作業単位 B-1）。
    ///
    /// 上限が無いと、形式を無視して喋り続けるモデルが本文生成の GPU を握り続ける。
    ///
    /// # 期待値を定数から作らない（CLAUDE.md / 16b で踏んだ形）
    ///
    /// [`DRIFT_LINE_TOKENS`] から期待値を組むと、
    /// **その定数を動かす変異は両側が一緒に動いて緑になる。**
    /// ここは**関数の外にある値**——`StatAxis::MAX_CHARS` 文字ちょうどの軸名で
    /// **最悪の1行を実際に組み立てて測ったバイト数**——と突き合わせる。
    ///
    /// # ⚠ **かつてここには抽出との比があった**（作業単位 B-1 で外した）
    ///
    /// `DRIFT_RESPONSE_TOKENS <= EXTRACTION_RESPONSE_TOKENS / 4` という `const assert` が
    /// **「小さい上限を送っている」を、定数の写しではない形で守っていた。**
    /// **その比そのものが、上限を最悪の1行より小さい値に縛り付けていた**
    /// ——**正しい値にするとビルドが通らなかった**
    /// （実際の数は ADR-0027 §打ち切り 1 の表。**散文の数はあそこ1箇所**）。
    ///
    /// **押さえは、抽出の都合ではなく答えの形から出す。**
    /// 抽出の上限は**あちらの実測**（応答の長さ / 行列の待ち時間）で動く数で、
    /// **変容の答えが1行に収まるかどうかとは何の関係も無い。**
    /// いまの押さえ（最悪の1行ちょうど）は比より狭い——
    /// **大きくする変異も小さくする変異も、どちらもここで赤くなる。**
    #[test]
    fn the_drift_request_carries_a_small_response_limit() {
        let sub = MockSub::replying("なし");
        let _ran = run_once(&sub, USER_MESSAGE);

        assert_eq!(
            sub.drift_max_tokens(),
            Some(worst_case_response_tokens(1)),
            "変容の依頼が運ぶ上限が、最悪の1行ちょうどではない"
        );
    }

    /// **変容の依頼は温度を明示する**
    /// （[ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定2 /
    /// 作業単位 A-7d）。
    ///
    /// # ここが見ているもの
    ///
    /// **依頼が上流へ渡るときに温度が載っていること**と、**その値**の2つ。
    /// **`None`（上流の既定に任せる）に戻す変異**も、**別の値にする変異**も、
    /// ここで赤くなる。
    ///
    /// # 期待値を [`SUB_LLM_TEMPERATURE`] から作らない
    ///
    /// **定数から作ると、値を動かす変異が両側で同時に動いて緑になる**
    /// （CLAUDE.md「写しを1つにしても、その1つは自動的には守られない」）。**`0.8` を手で書く。**
    ///
    /// # ⚠ 抽出との取り違えは、ここでは観測できない
    ///
    /// **抽出も同じ値を送る**ので、2つを入れ替えても出力は1バイトも変わらない
    /// （[`SUB_LLM_TEMPERATURE`] の doc）。**ここが言えるのは「変容の依頼に 0.8 が載る」まで。**
    /// **本文の経路との取り違え**は `tests/server_stat_drift.rs` の
    /// `each_path_sends_its_own_temperature` が見る。
    #[test]
    fn the_drift_request_carries_the_explicit_sub_temperature() {
        let sub = MockSub::replying("なし");
        let _ran = run_once(&sub, USER_MESSAGE);

        assert_eq!(
            sub.drift_temperature(),
            Some(0.8),
            "変容の依頼が温度を明示していない（上流の既定に落ちると、\
             どの設定で測ったかが言えなくなる / ADR-0038 §決定2）"
        );
    }

    /// **最悪の応答を実際に組み立てて、その見積もりトークン数を返す**（作業単位 16e-a / B-1）。
    ///
    /// `StatAxis::MAX_CHARS` 文字ちょうどの軸名を**アストラル面の文字**（1文字 = 4 バイト）で
    /// 埋める。`prompt::estimate` は UTF-8 バイト数を上界として数えるので、これが最悪。
    ///
    /// **[`DRIFT_LINE_TOKENS`] からも [`FALLBACK_MAX_DRIFT_RESPONSE_TOKENS`] からも導かない**
    /// ——導くと、定数を動かす変異が両側で同時に動いて緑を通る。
    ///
    /// # ⚠ **「最悪」に引用記号（`> `）は含めない**（作業単位 B-1-2 / レビュー指摘5）
    ///
    /// **16e-c は「31% の応答が依頼の引用記号を持ち込む」を実測している**ので、
    /// 実際に届く行は `> - {軸名} +1` になりうる。**それでもここでは数えない。**
    ///
    /// **現実の軸名は 5〜10 文字**で、ここが見込んでいる
    /// 「`StatAxis::MAX_CHARS` 文字すべてがアストラル面の文字」との差は**桁で開いている**
    /// ——記号の 2 バイトはその余裕に吸収される。
    /// **枠が本当に足りなくなるのは「20 文字すべてが 4 バイト文字の軸名」という
    /// 病的な場合だけ**で、そのときは記号の有無に関わらず切れる。
    ///
    /// **足りずに切れた回は `truncated` として丸ごと捨てられ、破棄率のログに現れる。**
    /// **その欄が並び始めたら、ここに記号を足す前に、まず軸名の実物を見ること。**
    fn worst_case_response_tokens(lines: usize) -> u32 {
        let widest = "\u{20BB7}".repeat(StatAxis::MAX_CHARS);
        assert_eq!(
            StatAxis::parse(&widest).map(|axis| axis.as_str().chars().count()),
            Ok(StatAxis::MAX_CHARS),
            "前提: 上限ちょうどの軸名が作れること"
        );
        let worst: String = (0..lines)
            .map(|_| format!("- {widest} +1\n"))
            .collect::<Vec<String>>()
            .concat();
        crate::prompt::estimate_tokens(&worst)
    }

    /// **応答の上限は「最悪の1行 × 見込む行数」ちょうど**（ADR-0027 §12 / 作業単位 B-1）。
    ///
    /// # **期待値を定数から作らない**（CLAUDE.md / 16b で踏んだ形）
    ///
    /// `DRIFT_LINE_TOKENS` を使って期待値を組むと、
    /// **その定数を倍にする変異も半分にする変異も両側が一緒に動いて緑になる。**
    /// ここは**関数の外にある値**——`StatAxis::MAX_CHARS` 文字ちょうどの軸名を
    /// N 本ぶん並べた**最悪の応答を実際に組み立てて、その見積もりバイト数**
    /// （[`worst_case_response_tokens`]）——で押さえる。
    ///
    /// 両側から挟む:
    ///
    /// * **下から**: 最悪の応答が上限に収まること（**足りないと末尾が切れる**）。
    /// * **上から**: **それを1トークンも超えないこと。**
    ///
    /// # ⚠ **上端の理由は「行列」ではない**（作業単位 B-1 で直した）
    ///
    /// **かつてここは「最悪の応答の2倍未満」で、理由を
    /// 「喋り続けるモデルが本文生成の GPU を握り続ける」（＝行列）と書いていた。**
    /// **理由と出どころがずれている**——出どころは応答の長さで、行列は一度も測っていない。
    /// 24 が抽出側で踏んだのと同じ構図で、あちらは**窓の中に元の値が居座った。**
    ///
    /// **変容側には行列の実測が無い**ので、**無い数を作らない。**
    /// 上端は**答えの形**から素直に出す——
    /// 受け付ける答えは `single` なら1行、`all` なら軸の本数ぶんの行しか無い
    /// （それ以上は `parse::drift` が丸ごと捨てる）ので、
    /// **それより長い上限を送っても採れる情報が1つも増えない。**
    ///
    /// **⚠ 「行列の余裕が守られている」とは読まないこと。**
    /// **行列が伸びたときに起きるのは「遅れる」であって「壊れる」ではない**
    /// （B-6 / [`FALLBACK_MAX_DRIFT_RESPONSE_TOKENS`] の doc / 抽出側の
    /// `QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS` と同じ但し書き）。
    #[test]
    fn the_response_limit_covers_the_worst_case_scorecard() {
        for axes in [1_usize, 2, 5, 20] {
            let needed = worst_case_response_tokens(axes);
            let limit = drift_response_tokens(DriftForm::All, axes, SubContext::FALLBACK);

            assert!(
                limit >= needed,
                "最悪の応答が上限に収まらない（軸 {axes} 本 / 要 {needed} / 上限 {limit}）"
            );
            assert!(
                limit <= needed,
                "上限が最悪の応答より長い（軸 {axes} 本 / 要 {needed} / 上限 {limit}）"
            );
        }

        // **軸を増やせば上限も増える**（本数を無視する実装と区別する）。
        for axes in [1_usize, 2, 5] {
            assert!(
                drift_response_tokens(DriftForm::All, axes + 1, SubContext::FALLBACK)
                    > drift_response_tokens(DriftForm::All, axes, SubContext::FALLBACK),
                "軸を増やしても上限が変わらない: {axes}"
            );
        }

        // **1軸を尋ねる形は、軸が何本でも「最悪の1行」ちょうど**（ADR-0024 §3）。
        //
        // # ⚠ かつてここは「本数に追随しないこと」しか見ていなかった（作業単位 B-1）
        //
        // **`all` には課していた「最悪の1行が収まること」を、`single` には
        // 一度も課していなかった。** その結果 `single` は `all` より小さい上限を送り続けており、
        // **アストラル面の軸名でも、ふつうの3バイト日本語の軸名でも収まらなかった**
        // （実際の数は ADR-0027 §打ち切り 1 の表。**散文の数はあそこ1箇所**）。
        // 収まらない回は行末の符号が落ちて `malformed` になる
        // ——**ログには出るが、読んだ人はプロンプトを直しに行く。打ち手が正反対。**
        let single = worst_case_response_tokens(1);
        for axes in [1_usize, 2, 5, 100] {
            assert_eq!(
                drift_response_tokens(DriftForm::Single, axes, SubContext::FALLBACK),
                single,
                "1軸を尋ねる形の上限が、最悪の1行ちょうどではない: 軸 {axes} 本"
            );
        }

        // **軸が極端に多くても、依頼と足して実コンテキストに収まる**（超えると、LM Studio は
        // 前方から黙って切り捨てる / ADR-0013。llama は 400 で断るか、応答が黙って短くなる / ADR-0052 M5）。
        for form in [DriftForm::Single, DriftForm::All] {
            for axes in [1_usize, 1_000, usize::MAX] {
                let total = FALLBACK_MAX_DRIFT_PROMPT_TOKENS.saturating_add(drift_response_tokens(
                    form,
                    axes,
                    SubContext::FALLBACK,
                ));
                assert!(
                    total <= SubContext::FALLBACK.tokens(),
                    "依頼と応答の合計が実コンテキストを超える（{form:?} / 軸 {axes} 本 / 合計 {total}）"
                );
            }
        }
    }

    /// **doc が名指しする「軸 96 本から末尾が切れる」の境目を、実際に確かめる**
    /// （作業単位 B-1-2 / レビュー指摘3）。
    ///
    /// # なぜ要るのか —— **算術が散文でしか書かれていなかった**
    ///
    /// [`FALLBACK_MAX_DRIFT_RESPONSE_TOKENS`] の doc は、かつて境目の掛け算を
    /// **手で計算した数として直接載せていた**（いまは ADR-0027 §打ち切り 1 に寄せてある）。
    /// **その算術を確かめている検査は、どこにも存在しなかった。**
    /// `StatAxis::MAX_CHARS` が動いた日に、**doc だけが静かに嘘になる。**
    ///
    /// [`the_response_limit_covers_a_request_that_fills_the_prompt_budget`] は
    /// **下界**（＝上限が小さすぎないこと）しか見ておらず、**境目がどこかは言っていない。**
    /// [`the_response_limit_covers_the_worst_case_scorecard`] が回す軸数は **1 / 2 / 5 / 20**
    /// で、**頭打ちに一度も触れていない。**
    ///
    /// # 期待値は「関数の外にある値」から作る（CLAUDE.md / 16b で踏んだ形）
    ///
    /// **[`DRIFT_LINE_TOKENS`] を掛け算して期待値を組まない。**
    /// 組むと、**その定数を動かす変異は両側が一緒に動いて緑になる。**
    /// ここは `StatAxis::MAX_CHARS` 文字ちょうどの軸名で
    /// **最悪の応答を N 行ぶん実際に組み立てて測った値**（[`worst_case_response_tokens`]）
    /// と、`drift_response_tokens` が返す上限を突き合わせる。
    ///
    /// **直書きするのは境目の本数だけ**——それが doc の主張そのものだからである。
    ///
    /// # 両側から挟む
    ///
    /// * **95 本**: 最悪の応答と上限が**一致**する（＝頭打ちに当たっていない）。
    /// * **96 本**: 上限が最悪の応答を**下回る**（＝頭打ちが効き、末尾が切れる）。
    ///
    /// 片側だけだと、境目を動かす変異の**半分しか止まらない。**
    #[test]
    fn the_axis_count_where_the_worst_case_scorecard_stops_fitting() {
        /// doc が名指ししている「**最後に丸ごと収まる**軸の本数」。
        const LAST_THAT_FITS: usize = 95;

        let fitting = worst_case_response_tokens(LAST_THAT_FITS);
        assert_eq!(
            drift_response_tokens(DriftForm::All, LAST_THAT_FITS, SubContext::FALLBACK),
            fitting,
            "doc は「軸 {LAST_THAT_FITS} 本までは収まる」と書いているが、\
             上限が最悪の応答（{fitting}）と一致しない"
        );

        let overflowing = worst_case_response_tokens(LAST_THAT_FITS + 1);
        let limit = drift_response_tokens(DriftForm::All, LAST_THAT_FITS + 1, SubContext::FALLBACK);
        assert!(
            limit < overflowing,
            "doc は「軸 {} 本以上で末尾が切れる」と書いているが、\
             最悪の応答（{overflowing}）が上限（{limit}）に収まっている",
            LAST_THAT_FITS + 1
        );
    }

    /// **依頼の予算いっぱいの依頼に対して、応答の上限が答えを包む**
    /// （[`FALLBACK_MAX_DRIFT_RESPONSE_TOKENS`] を**下から**押さえる / レビュー指摘A）。
    ///
    /// # なぜ [`the_response_limit_covers_the_worst_case_scorecard`] では足りなかったのか
    ///
    /// あちらが回す軸数は **1 / 2 / 5 / 20** で、**頭打ちが効き始めるのは 96 本から。**
    /// `1_000` と `usize::MAX` のループは `total <= SubContext::FALLBACK.tokens()`（**上から**）
    /// しか見ていないので、**頭打ちを半分にする変異が全緑を通り抜けた**（実測）。
    ///
    /// # 押さえは「関数の外にある値」から作る
    ///
    /// 1. **依頼の側**——軸名が上限の長さ（`StatAxis::MAX_CHARS` 文字）の軸を
    ///    1本ずつ増やして**実際に依頼を組み**、`request_tokens` が
    ///    [`FALLBACK_MAX_DRIFT_PROMPT_TOKENS`] を超える手前の本数 `n` を数える。
    /// 2. **応答の側**——その `n` 行ぶんの**最悪の応答を実際に組み立て**、
    ///    `estimate_tokens` で測る。
    /// 3. **上限がそれを包むこと。**
    ///
    /// どちらの数も `FALLBACK_MAX_DRIFT_RESPONSE_TOKENS` からも `DRIFT_LINE_TOKENS` からも
    /// 導いていないので、**上限を小さくする変異はここで赤くなる。**
    ///
    /// # ⚠ **これは「どんな依頼でも包む」ではない**
    ///
    /// 軸名が短ければ、依頼の予算にはもっと多くの軸が載る（`n` は軸名の長さで決まる）。
    /// **短い軸名を数百本持つキャラクターでは、いまも応答が切れる**
    /// ——その回は `missing_axis` として**ログに現れる**（`FALLBACK_MAX_DRIFT_RESPONSE_TOKENS` の doc）。
    /// ここが主張するのは「**最長の軸名で予算を埋めた依頼までは包む**」までである。
    #[test]
    fn the_response_limit_covers_a_request_that_fills_the_prompt_budget() {
        let widest = "\u{20BB7}".repeat(StatAxis::MAX_CHARS);
        let axis = StatAxis::parse(&widest).expect("正当な軸名であること");
        let display_name = DisplayName::parse(DISPLAY_NAME).expect("正当な表示名であること");
        let scale = StatScale::parse(StatScale::MAX).expect("正当な尺度であること");
        let value = scale
            .value(StatScale::MAX)
            .expect("尺度に収まる値であること");

        // **依頼の予算に収まる最大の本数を、実際に組んで数える。**
        let mut stats: Vec<StatAlloc> = Vec::new();
        let mut fits = 0_usize;
        loop {
            stats.push(StatAlloc {
                axis: axis.clone(),
                value,
            });
            let messages = build_drift_prompt(&DriftInput {
                display_name: &display_name,
                stats: &stats,
                form: DriftForm::All,
                note: DriftNote::None,
                scale,
                user_message: USER_MESSAGE,
                character_message: REPLY,
            })
            .expect("軸があるので依頼が作られること")
            .into_messages();

            if request_tokens(&messages) > FALLBACK_MAX_DRIFT_PROMPT_TOKENS {
                break;
            }
            fits = stats.len();
        }

        assert!(
            fits > 20,
            "前提: 予算にはそれなりの本数が載ること（{fits} 本では下界が緩すぎる）"
        );

        // **その本数ぶんの最悪の応答**を実際に組み立てて測る。
        let worst: String = (0..fits)
            .map(|_| format!("- {widest} +1\n"))
            .collect::<Vec<String>>()
            .concat();
        let needed = crate::prompt::estimate_tokens(&worst);

        assert!(
            FALLBACK_MAX_DRIFT_RESPONSE_TOKENS >= needed,
            "予算いっぱいの依頼（軸 {fits} 本）の答えが応答の上限に収まらない: \
             要 {needed} / 上限 {FALLBACK_MAX_DRIFT_RESPONSE_TOKENS}"
        );
        // **頭打ちが実際にその本数で効いていないこと**（＝上限がそのまま出ること）。
        assert_eq!(
            drift_response_tokens(DriftForm::All, fits, SubContext::FALLBACK),
            DRIFT_LINE_TOKENS.saturating_mul(u32::try_from(fits).unwrap_or(u32::MAX)),
            "予算いっぱいの依頼で、もう頭打ちに当たっている"
        );
    }

    /// **全軸採点で起動すると、依頼の形と応答の上限の両方が変わる**（ADR-0027 §12）。
    ///
    /// # 設定が `server::drift` まで届いていることを、**既定でない値**で見る
    ///
    /// 本番の入口（`App::bind`）を通る側は
    /// `tests/server_stat_drift.rs::the_drift_form_reaches_the_upstream_through_the_real_bind`。
    /// ここは**依頼の中身と上限**を直接見る。
    ///
    /// # ⚠ 応答の分解はまだ繋がっていない（16e-a）
    ///
    /// N 行の応答は `multiple_lines` として捨てられる（このモジュールの doc）。
    /// **そこまで含めて、いまの姿を固定してある**——16e-b が繋いだときに、
    /// この見張り番が「変わった」と言う。
    #[test]
    fn the_all_axis_form_changes_the_request_and_the_limit() {
        let reply =
            format!("- {MOVED_AXIS} +1\n- {TWIN_AXIS} 0\n- {STILL_AXIS} 0\n- {SILENT_AXIS} 0");
        let sub = MockSub::replying(&reply);
        let ran = run_once_in(&sub, USER_MESSAGE, StatDriftMode::On, DriftForm::All);

        let prompt = sub.drift_prompt();
        assert!(
            prompt.contains("1項目につき1行ずつ"),
            "全軸採点の指示になっていない: {prompt:?}"
        );
        assert!(
            prompt.contains("- 項目名 0"),
            "0 の手本が載っていない: {prompt:?}"
        );
        assert!(
            !prompt.contains("書けるのは1行だけです"),
            "1軸を尋ねる形の指示が混ざった: {prompt:?}"
        );

        // **上限は軸の本数から出ている**（1軸を尋ねる形の「1行ぶん」ではない）。
        let axes = untouched().len();
        assert_eq!(
            sub.drift_max_tokens(),
            Some(drift_response_tokens(
                DriftForm::All,
                axes,
                SubContext::FALLBACK
            )),
            "全軸採点の上限を送っていない"
        );
        assert_ne!(
            sub.drift_max_tokens(),
            Some(drift_response_tokens(
                DriftForm::Single,
                axes,
                SubContext::FALLBACK
            )),
            "1軸を尋ねる形の上限のままである"
        );

        // **16e-b で分解が繋がった。** N 行の応答は採点表として読まれる。
        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=scored"),
            "N 行の応答が採点表として読まれていない: {:?}",
            ran.output
        );
        assert!(
            !has_field(&ran.output, "drift_outcome=multiple_lines"),
            "1軸を尋ねる形の分解器に掛かっている: {:?}",
            ran.output
        );

        // **`0` の軸は票にならない**（ADR-0027 §決定1 の「0 でなかった回数」）。
        assert_eq!(
            ran.votes,
            vec![(ran.session_id, MOVED_AXIS.to_owned(), "up")],
            "0 と採点された軸まで票になっている"
        );

        // **値は帳簿が動かす。** 1票（重み 1）＝費用 1 なので1段。
        assert_eq!(
            ran.stats,
            vec![
                (MOVED_AXIS.to_owned(), MOVED_START + 1),
                (TWIN_AXIS.to_owned(), MOVED_START),
                (STILL_AXIS.to_owned(), STILL_START),
                (SILENT_AXIS.to_owned(), 0),
            ],
            "採点表から値が動いていない"
        );
        assert!(has_field(drift_record(&ran.output), "drift_steps=1"));
        assert_eq!(
            ran.bystander_stats,
            bystander_untouched(),
            "別のキャラクターの軸が動いた"
        );
    }

    /// **フィクスチャが本当に「抽出だけが落ちる」帯に居る**
    /// （`a_turn_whose_extraction_request_is_too_long_still_drifts` の前提）。
    ///
    /// # なぜ別の見張り番が要るのか
    ///
    /// あちらは「抽出が飛んで変容が走る」という**結果**を見るので、
    /// **帯から外れると「両方飛ぶ」に変わって、前提の assert で落ちる。**
    /// 落ちること自体は正しいが、**どちら側へ外れたのかが読めない。**
    /// ここは2つの依頼のトークン数を直接測り、**上限との余裕も一緒に出す。**
    ///
    /// # 帯の正体
    ///
    /// 抽出の依頼は手本4行・規則の節・記憶の節の枠・`# 会話` の前置きを持ち、
    /// 変容の依頼は**軸の短い行しか持たない。** 上限は両者とも
    /// [`SubContext`] の半分で同じなので、
    /// **削れない枠の差がそのまま「抽出だけが落ちる」帯の広さ**になる。
    #[test]
    fn the_long_fixture_sits_in_the_band_where_only_the_extraction_is_rejected() {
        use crate::prompt::extraction::{ExampleStyle, ExtractionInput, build_extraction_prompt};

        let display_name = DisplayName::parse(DISPLAY_NAME).expect("正当な表示名であること");
        let long = long_message_only_the_extraction_rejects();

        let extraction = build_extraction_prompt(&ExtractionInput {
            display_name: &display_name,
            user_message: &long,
            character_message: REPLY,
            known_memories: &[],
            budget: super::super::extract::FALLBACK_EXTRACTION_PROMPT_TOKENS,
            // **本番の呼び出し側と同じ綴り**（帯の広さは削れない枠で決まる / 22a）。
            example_style: ExampleStyle::Balanced,
        })
        .into_messages();
        let drift = build_drift_prompt(&DriftInput {
            display_name: &display_name,
            stats: &allocs(
                SCALE,
                &[(MOVED_AXIS, MOVED_START), (STILL_AXIS, STILL_START)],
            ),
            form: DriftForm::Single,
            note: DriftNote::None,
            scale: StatScale::parse(SCALE).expect("正当な尺度であること"),
            user_message: &long,
            character_message: REPLY,
        })
        .expect("軸が載るので依頼が作られること")
        .into_messages();

        let extraction_tokens = request_tokens(&extraction);
        let drift_tokens = request_tokens(&drift);

        assert!(
            extraction_tokens > super::super::extract::FALLBACK_EXTRACTION_PROMPT_TOKENS,
            "抽出は上限を超えること: {extraction_tokens} <= {}",
            super::super::extract::FALLBACK_EXTRACTION_PROMPT_TOKENS
        );
        assert!(
            drift_tokens <= FALLBACK_MAX_DRIFT_PROMPT_TOKENS,
            "変容は上限に収まること: {drift_tokens} > {FALLBACK_MAX_DRIFT_PROMPT_TOKENS}"
        );
        // **両側の余裕を出す。** 片側が痩せてきたら、この数字で気付ける。
        assert!(
            extraction_tokens - super::super::extract::FALLBACK_EXTRACTION_PROMPT_TOKENS >= 100
                && FALLBACK_MAX_DRIFT_PROMPT_TOKENS - drift_tokens >= 100,
            "帯の余裕が薄い（指示を数行足しただけで動く）: 抽出 {extraction_tokens} / 変容 {drift_tokens} / 上限 {FALLBACK_MAX_DRIFT_PROMPT_TOKENS}"
        );
    }

    /// **抽出だけが上限を超える長さの発言。**
    ///
    /// # この数はフィクスチャの設計値であって、規則ではない
    ///
    /// `あ` は UTF-8 で3バイト。`estimate_tokens` はバイト数以上なので、
    /// 文字数 × 3 がそのまま会話ぶんの費用の下界になる。
    /// **帯の中に居ることは
    /// [`the_long_fixture_sits_in_the_band_where_only_the_extraction_is_rejected`] が
    /// 両側の余裕つきで測る**ので、この数が黙って古びることはない。
    fn long_message_only_the_extraction_rejects() -> String {
        "あ".repeat(2_050)
    }

    // ---------------------------------------------------------------------
    // 結末ごとの記録（ADR-0024 の「危険」3つ）
    // ---------------------------------------------------------------------

    /// **適用された回は、軸と before → after と向きが読める。**
    ///
    /// 動かすのは**そのキャラクターの軸だけ**。傍観者は同じ綴りの軸を
    /// **同じ値**で持っているので、持ち主を落とす変異は「両方動いた」として現れる。
    #[test]
    fn an_applied_drift_records_the_axis_and_both_values() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let ran = run_once(&sub, USER_MESSAGE);

        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=applied"),
            "{:?}",
            ran.output
        );
        assert!(
            has_field(
                drift_record(&ran.output),
                &format!("drift_moved={}", DriftWrite::Moved.label())
            ),
            "書き込みが届いたことが読めること: {:?}",
            ran.output
        );
        assert!(
            has_field(drift_record(&ran.output), &axis_field(MOVED_AXIS)),
            "動いた軸が読めること: {:?}",
            ran.output
        );
        assert!(
            has_field(
                drift_record(&ran.output),
                &format!("drift_before={MOVED_START}")
            ) && has_field(
                drift_record(&ran.output),
                &format!("drift_after={}", MOVED_START + 1)
            ),
            "前後の値が読めること: {:?}",
            ran.output
        );
        assert!(
            has_field(
                drift_record(&ran.output),
                &format!("drift_delta={}", DriftDelta::Up.label())
            ),
            "向きが読めること: {:?}",
            ran.output
        );

        assert_eq!(
            ran.stats,
            vec![
                (MOVED_AXIS.to_owned(), MOVED_START + 1),
                (TWIN_AXIS.to_owned(), MOVED_START),
                (STILL_AXIS.to_owned(), STILL_START),
                (SILENT_AXIS.to_owned(), 0),
            ],
            "指した軸だけが1段動くこと"
        );
        assert_eq!(
            ran.bystander_stats,
            bystander_untouched(),
            "同じ綴り・同じ値の軸を持つ別のキャラクターは動かないこと"
        );
    }

    /// **下げも同じように記録される。** 向きの欄が上げと別の値になる。
    #[test]
    fn a_downward_drift_records_the_other_direction() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} -1"));
        let ran = run_once(&sub, USER_MESSAGE);

        assert!(
            has_field(
                drift_record(&ran.output),
                &format!("drift_delta={}", DriftDelta::Down.label())
            ),
            "下げとして読めること: {:?}",
            ran.output
        );
        assert_ne!(
            DriftDelta::Up.label(),
            DriftDelta::Down.label(),
            "前提: 2つの向きが別の綴りであること（危険 2 の比が作れる形）"
        );
        assert!(
            has_field(
                drift_record(&ran.output),
                &format!("drift_after={}", MOVED_START - 1)
            ),
            "1段下がること: {:?}",
            ran.output
        );
        assert_eq!(
            ran.stats,
            vec![
                (MOVED_AXIS.to_owned(), MOVED_START - 1),
                (TWIN_AXIS.to_owned(), MOVED_START),
                (STILL_AXIS.to_owned(), STILL_START),
                (SILENT_AXIS.to_owned(), 0),
            ],
        );
    }

    /// **提案が無かった回は「なし」として記録される**（ADR-0024 の危険 3）。
    ///
    /// **破棄と同じ値にしない。** 混ぜると、
    /// 「モデルが黙っている」と「モデルが形式を守れない」が区別できなくなる。
    #[test]
    fn a_turn_with_no_suggestion_is_recorded_as_nothing() {
        let sub = MockSub::replying("なし");
        let ran = run_once(&sub, USER_MESSAGE);

        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=nothing"),
            "{:?}",
            ran.output
        );
        assert!(
            has_field(
                drift_record(&ran.output),
                &format!("drift_moved={NO_WRITE}")
            ),
            "書き込みまで行っていないこと: {:?}",
            ran.output
        );
        assert!(
            has_field(drift_record(&ran.output), &axis_field(NO_AXIS))
                && has_field(
                    drift_record(&ran.output),
                    &format!("drift_delta={NO_DELTA}")
                ),
            "軸も向きも該当なしであること: {:?}",
            ran.output
        );
        assert!(
            has_field(
                drift_record(&ran.output),
                &format!("drift_before={NO_VALUE}")
            ) && has_field(
                drift_record(&ran.output),
                &format!("drift_after={NO_VALUE}")
            ),
            "値の欄も該当なしであること: {:?}",
            ran.output
        );
        assert_eq!(ran.stats, untouched());
    }

    /// **端では飽和する。破棄と混ぜない**（ADR-0024 §5）。
    ///
    /// 飽和は**提案が正しく、適用できないだけ**である。
    /// 「毎ターン上げようとしているが上端で止まっている」と
    /// 「提案が出ていない」は**まったく違う状態**なので、区別が付く値にする。
    ///
    /// # 下端は `0`（ADR-0027 §決定5 / 16e-b で `1` から変わった）
    ///
    /// **値 0 の軸から `-1` が飽和**になる。値 1 からは `0` へ落ちる
    /// ——「変容が軸を本文プロンプトから消す」経路は、
    /// **塞ぐのをやめた**（§決定4 で 0 の軸も依頼に載るので片道ではない）。
    ///
    /// # 帳簿の飽和と、提案の飽和は別の欄で数える
    ///
    /// `drift_outcome=saturated` は**提案が端の軸に来た**こと
    /// （`domain::apply_drift`）で、`drift_saturations` は
    /// **累積が費用に届いたのに端で動けなかった段の数**（`domain::drift_ledger`）である。
    /// 1票（重み 1）＝費用 1 なので、この回は**両方が立つ。**
    #[test]
    fn saturation_at_the_bottom_is_recorded_separately_from_a_discard() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("floor.sqlite")));

        // **下端（`DRIFT_FLOOR` = 0）に置く。** ここから `-1` は飽和。
        runtime.block_on(put_stats(
            &fixture.store,
            fixture.character.id,
            &[(MOVED_AXIS, DRIFT_FLOOR), (STILL_AXIS, STILL_START)],
        ));

        let sub = MockSub::replying(&format!("- {MOVED_AXIS} -1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::On,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                extractor.wait_for_the_running_task().await;
            });
        });
        let after = runtime.block_on(stats(&fixture.store, fixture.character.id));
        runtime.block_on(fixture.store.close());

        assert!(
            has_field(drift_record(&output), "drift_outcome=saturated"),
            "{output:?}"
        );
        assert!(
            has_field(drift_record(&output), &axis_field(MOVED_AXIS))
                && has_field(
                    drift_record(&output),
                    &format!("drift_delta={}", DriftDelta::Down.label())
                ),
            "飽和した軸と向きが読めること（どちらの端で詰まっているか）: {output:?}"
        );
        assert!(
            has_field(drift_record(&output), &format!("drift_moved={NO_WRITE}")),
            "書き込みまで行っていないこと: {output:?}"
        );
        // **帳簿の側でも段が飽和している**（累積は費用に届いた）。
        assert!(
            has_field(drift_record(&output), "drift_saturations=1")
                && has_field(drift_record(&output), "drift_steps=0"),
            "帳簿の飽和が数えられていないか、値が動いてしまっている: {output:?}"
        );
        assert_eq!(
            after,
            vec![
                (MOVED_AXIS.to_owned(), DRIFT_FLOOR),
                (STILL_AXIS.to_owned(), STILL_START)
            ],
            "負の値を作らないこと"
        );
    }

    /// **上端でも飽和する。**
    #[test]
    fn saturation_at_the_top_is_recorded_as_saturated() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("ceiling.sqlite")));

        runtime.block_on(put_stats(
            &fixture.store,
            fixture.character.id,
            &[(MOVED_AXIS, SCALE), (STILL_AXIS, STILL_START)],
        ));

        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::On,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                extractor.wait_for_the_running_task().await;
            });
        });
        let after = runtime.block_on(stats(&fixture.store, fixture.character.id));
        runtime.block_on(fixture.store.close());

        assert!(
            has_field(drift_record(&output), "drift_outcome=saturated"),
            "{output:?}"
        );
        assert!(
            has_field(
                drift_record(&output),
                &format!("drift_delta={}", DriftDelta::Up.label())
            ),
            "上端で詰まっていることが読めること: {output:?}"
        );
        assert_eq!(
            after,
            vec![
                (MOVED_AXIS.to_owned(), SCALE),
                (STILL_AXIS.to_owned(), STILL_START)
            ],
            "尺度を超えないこと"
        );
    }

    /// **破棄の理由が、理由ごとに違う値で記録される**（ADR-0024 §6）。
    ///
    /// 1つに潰すと、「モデルが形式を守れない」「列挙する癖がある」
    /// 「知らない軸を作る」が区別できず、
    /// **プロンプトを直すべきかモデルを替えるべきかの判断材料にならない。**
    ///
    /// # `not_in_prompt` はもう出ない（ADR-0027 §決定4 / §決定6）
    ///
    /// 値 0 の軸も依頼に載るようになったので、その理由は**到達不能になった**
    /// ——語彙ごと消してある（`domain::DriftDiscardReason`）。
    #[test]
    fn every_discard_reason_is_recorded_under_its_own_value() {
        let cases = [
            // 形式が読めない（幅が ±1 でない）。
            (format!("- {MOVED_AXIS} +2"), "malformed"),
            // 2行以上。**先頭だけを採らない。**
            (
                format!("- {MOVED_AXIS} +1\n- {STILL_AXIS} -1"),
                "multiple_lines",
            ),
            // 保存されていない軸。
            (format!("- {UNKNOWN_AXIS} +1"), "unknown_axis"),
        ];

        for (reply, expected) in cases {
            let sub = MockSub::replying(&reply);
            let ran = run_once(&sub, USER_MESSAGE);

            assert!(
                has_field(
                    drift_record(&ran.output),
                    &format!("drift_outcome={expected}")
                ),
                "{reply:?} は {expected} として数えること: {:?}",
                ran.output
            );
            assert!(
                has_field(
                    drift_record(&ran.output),
                    &format!("drift_moved={NO_WRITE}")
                ),
                "破棄では書き込みまで行かないこと: {:?}",
                ran.output
            );
            assert_eq!(ran.stats, untouched(), "{reply:?} で値が動かないこと");
            assert_eq!(ran.bystander_stats, bystander_untouched());
        }
    }

    /// **飽和・破棄・適用・提案なしが、すべて別の値になる**（ADR-0024 の危険を数える前提）。
    ///
    /// 4つを1本にまとめると、危険1（端に貼り付く）と危険3（提案が出ない）が
    /// **同じ数に見える。**
    #[test]
    fn the_four_kinds_of_outcome_never_share_a_value() {
        let seen = [
            (format!("- {MOVED_AXIS} +1"), "applied"),
            ("なし".to_owned(), "nothing"),
            (format!("- {UNKNOWN_AXIS} +1"), "unknown_axis"),
        ];
        let mut values = Vec::new();
        for (reply, expected) in seen {
            let sub = MockSub::replying(&reply);
            let ran = run_once(&sub, USER_MESSAGE);
            assert!(
                has_field(
                    drift_record(&ran.output),
                    &format!("drift_outcome={expected}")
                ),
                "{reply:?}: {:?}",
                ran.output
            );
            values.push(expected);
        }
        // 飽和は上の2本（上端・下端）が別に見ている。ここでは語彙の重なりだけを押さえる。
        values.push("saturated");
        for (index, left) in values.iter().enumerate() {
            for right in &values[index + 1..] {
                assert_ne!(left, right, "結末の値が重なっています: {values:?}");
            }
        }
    }

    // ---------------------------------------------------------------------
    // 打ち切られた応答（作業単位 B-1 / ADR-0027 §打ち切り）
    // ---------------------------------------------------------------------

    /// **打ち切られた回は、届いた中身によらず丸ごと捨てる**（ADR-0027 §打ち切り）。
    ///
    /// # これが**静かな**方の経路である
    ///
    /// `parse_drift` は**2行以上を丸ごと捨てる**（`multiple_lines`）。
    /// **モデルが2行書いた回で、打ち切りが2行目を丸ごと消すと、
    /// 受け取る側には「1行の正当な提案」しか届かない。**
    /// 捨てるはずの回がそのまま値を動かし、**ログは正常な `applied` に見える。**
    ///
    /// **何行あったのかは分からない**ので、残った1行を信じる根拠が無い
    /// ——`multiple_lines` と同じ扱いにする（抽出の「最後の1行だけ捨てる」とは
    /// **意図的に違う判断**。理由は ADR-0027）。
    ///
    /// # 対照を同じ本文で置く
    ///
    /// **同じ応答が、打ち切られていなければ通る。**
    /// 対照が無いと「常に捨てる」実装が緑を通る（CLAUDE.md「高い側と安い側を1本に置く」）。
    #[test]
    fn a_truncated_turn_is_dropped_whole_even_when_what_arrived_reads_as_one_valid_line() {
        let reply = format!("- {MOVED_AXIS} +1");

        let cut = MockSub::replying_with(&reply, Some(FinishReason::Length));
        let ran = run_once(&cut, USER_MESSAGE);
        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=truncated"),
            "打ち切られた回を打ち切りとして数えていない: {:?}",
            ran.output
        );
        assert!(
            !has_field(&ran.output, "drift_outcome=applied"),
            "打ち切られた回の提案が通った: {:?}",
            ran.output
        );
        assert!(
            ran.votes.is_empty(),
            "打ち切られた回が票になった: {:?}",
            ran.votes
        );
        assert_eq!(ran.stats, untouched(), "打ち切られた回で値が動いた");
        assert_eq!(ran.bystander_stats, bystander_untouched());

        // **対照——同じ本文が、打ち切られていなければ通る。**
        let whole = MockSub::replying(&reply);
        let ran = run_once(&whole, USER_MESSAGE);
        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=applied"),
            "打ち切られていない回まで捨てている: {:?}",
            ran.output
        );
        assert_eq!(
            ran.stats,
            vec![
                (MOVED_AXIS.to_owned(), MOVED_START + 1),
                (TWIN_AXIS.to_owned(), MOVED_START),
                (STILL_AXIS.to_owned(), STILL_START),
                (SILENT_AXIS.to_owned(), 0),
            ],
            "打ち切られていない回で値が動かない"
        );
    }

    /// **打ち切られ方が違っても、行き着く先は同じ1つの値**（作業単位 B-1）。
    ///
    /// 上流が切った位置によって、届く形は3通りある:
    ///
    /// | 届いた形 | 印が無ければどうなるか |
    /// |---|---|
    /// | **2行目が丸ごと消えた**（完結した1行だけが届く） | `applied` —— **値が動く** |
    /// | **2行目が途中で切れた**（2行届く） | `multiple_lines` |
    /// | **1行が途中で切れた** | `malformed` |
    ///
    /// **受け取った側から3つを区別する手立ては無い。**
    /// 何行あったのかが分からない以上、**残った行を信じる根拠が無い。**
    /// だから**3つとも `truncated` に落とす。**
    ///
    /// # ⚠ **これが「最後の1行だけ捨てる」を止める見張り番**
    ///
    /// **抽出の扱い（最後の1行だけ捨てる）をここへ写すと、
    /// 2行目が途中で切れた回は「1行の正当な提案」になって値を動かす**
    /// ——**捨てるはずの回が適用される向き**で、**ログは正常な `applied` に見える。**
    /// 上の表の1行目と2行目が、その形を作ってある。
    ///
    /// # 印が無ければ3つが**別の**結末になることも、ここで見る
    ///
    /// 同じ値に潰れていたら、上のループは何も主張していない
    /// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
    #[test]
    fn every_shape_of_a_truncated_response_lands_on_the_same_value() {
        // **2行目が丸ごと消えた**（見えているのは完結した1行）。
        let line_gone = format!("- {MOVED_AXIS} +1");
        // **2行目が途中で切れた**（2行届いており、2行目は符号だけ）。
        let second_line_cut = format!("- {MOVED_AXIS} +1\n- {STILL_AXIS} -");
        // **1行が途中で切れた。**
        let only_line_cut = format!("- {MOVED_AXIS} +");

        // **印が無ければ、3つは別の結末になる**（前提。同値だと以下が空振りする）。
        let without_the_mark = [
            (&line_gone, "applied"),
            (&second_line_cut, "multiple_lines"),
            (&only_line_cut, "malformed"),
        ];
        let mut seen: Vec<&str> = Vec::new();
        for (reply, expected) in without_the_mark {
            let sub = MockSub::replying(reply);
            let ran = run_once(&sub, USER_MESSAGE);
            assert!(
                has_field(
                    drift_record(&ran.output),
                    &format!("drift_outcome={expected}")
                ),
                "前提: 印が無ければ {reply:?} は {expected} になること: {:?}",
                ran.output
            );
            assert!(!seen.contains(&expected), "前提: 3つが別の結末であること");
            seen.push(expected);
        }

        // **印が付くと、3つとも同じ値へ落ちる。**
        //
        // **`second_line_cut` を先頭に置く**（作業単位 B-1-2 / レビュー指摘2）——
        // 「最後の1行だけ捨てる」を素直に移植した変異で**実際に値が動くのはこの形**で、
        // `line_gone` から回すと**無害な形で先に止まり、失敗メッセージが
        // 何も残らない側（`nothing`）を指す。**
        // **この見張り番が止めているものが、失敗メッセージからそのまま読める並びにする。**
        for reply in [&second_line_cut, &line_gone, &only_line_cut] {
            let sub = MockSub::replying_with(reply, Some(FinishReason::Length));
            let ran = run_once(&sub, USER_MESSAGE);
            assert!(
                has_field(drift_record(&ran.output), "drift_outcome=truncated"),
                "{reply:?} が打ち切りとして数えられていない: {:?}",
                ran.output
            );
            assert_eq!(ran.stats, untouched(), "{reply:?} で値が動いた");
            assert!(
                ran.votes.is_empty(),
                "{reply:?} が票になった: {:?}",
                ran.votes
            );
        }
    }

    /// **`truncated` を `malformed` に混ぜない**（打ち手が正反対）。
    ///
    /// 崩れなら**プロンプトを直しに行く**が、打ち切りなら**上限を上げる**側である。
    /// 混ぜると、`drift_outcome=malformed` を読んだ人が
    /// **「モデルが形式を崩している」と誤診する**——ログには出ているのに、打ち手が逆になる。
    #[test]
    fn a_truncated_turn_is_not_counted_as_malformed() {
        let fragment = format!("- {MOVED_AXIS} +");
        let sub = MockSub::replying_with(&fragment, Some(FinishReason::Length));
        let ran = run_once(&sub, USER_MESSAGE);

        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=truncated"),
            "{:?}",
            ran.output
        );
        assert!(
            !has_field(&ran.output, "drift_outcome=malformed"),
            "打ち切りを崩れとして数えている（打ち手が正反対）: {:?}",
            ran.output
        );
    }

    /// **捨てるのは `length` の回だけ**（作業単位 B-1 / 24 と同じ判断）。
    ///
    /// `stop` / それ以外 / **申告なし**で捨てると、
    /// **停止理由を返さない上流に繋いだ瞬間、変容が永久に動かなくなる。**
    /// しかもログには `truncated` と出るので、**「上限に張り付いている」としか読めない。**
    #[test]
    fn only_the_length_reason_throws_the_whole_drift_response_away() {
        let reply = format!("- {MOVED_AXIS} +1");
        let moved = vec![
            (MOVED_AXIS.to_owned(), MOVED_START + 1),
            (TWIN_AXIS.to_owned(), MOVED_START),
            (STILL_AXIS.to_owned(), STILL_START),
            (SILENT_AXIS.to_owned(), 0),
        ];
        assert_ne!(
            moved,
            untouched(),
            "前提: 動いた形と動かない形が別であること"
        );

        for finish_reason in [
            Some(FinishReason::Length),
            Some(FinishReason::Stop),
            Some(FinishReason::Other),
            None,
        ] {
            let dropped = finish_reason == Some(FinishReason::Length);
            let sub = MockSub::replying_with(&reply, finish_reason);
            let ran = run_once(&sub, USER_MESSAGE);

            let expected = if dropped { "truncated" } else { "applied" };
            assert!(
                has_field(
                    drift_record(&ran.output),
                    &format!("drift_outcome={expected}")
                ),
                "{finish_reason:?} は {expected} になること: {:?}",
                ran.output
            );
            assert_eq!(
                ran.stats,
                if dropped { untouched() } else { moved.clone() },
                "{finish_reason:?} で値の動きが違う"
            );
        }
    }

    /// **全軸採点でも丸ごと捨てる**（作業単位 B-1）。
    ///
    /// # なぜ `missing_axis` のままではいけないのか
    ///
    /// 行が足りない採点表は `missing_axis` として捨てられる（`parse::drift`）。
    /// **数としては捨てられているが、理由が嘘になる**——
    /// あれは「**モデルが列挙を最後まで書かない**」の欄で、打ち手は指示の側である。
    /// **打ち切りの回がそこへ混ざると、上限を上げるべき回が指示の問題に見える。**
    ///
    /// **行が揃っている回**も一緒に測る。あちらは**捨てられずに値を動かしていた**
    /// ——最後の1行がちょうど収まっただけで、後ろに何が続くはずだったかは分からない。
    #[test]
    fn a_truncated_scorecard_is_dropped_whole_whether_or_not_a_line_is_missing() {
        let whole =
            format!("- {MOVED_AXIS} +1\n- {TWIN_AXIS} 0\n- {STILL_AXIS} 0\n- {SILENT_AXIS} 0");
        // **末尾の軸の行が届かなかった採点表。**
        let short = format!("- {MOVED_AXIS} +1\n- {TWIN_AXIS} 0\n- {STILL_AXIS} 0");
        assert_ne!(whole, short, "前提: 2つの応答が別の文字列であること");

        for reply in [&whole, &short] {
            let sub = MockSub::replying_with(reply, Some(FinishReason::Length));
            let ran = run_once_in(&sub, USER_MESSAGE, StatDriftMode::On, DriftForm::All);

            assert!(
                has_field(drift_record(&ran.output), "drift_outcome=truncated"),
                "{reply:?} が打ち切りとして数えられていない: {:?}",
                ran.output
            );
            assert!(
                !has_field(&ran.output, "drift_outcome=missing_axis"),
                "打ち切りを「列挙をやめた」として数えている: {:?}",
                ran.output
            );
            assert_eq!(ran.stats, untouched(), "{reply:?} で値が動いた");
            assert!(
                ran.votes.is_empty(),
                "{reply:?} が票になった: {:?}",
                ran.votes
            );
            // **崩れが薄れの供給源にならない**（`a_discarded_scorecard_never_fades_anything`）。
            assert!(
                ran.charges.is_empty(),
                "打ち切られた回に累積が動いた: {:?}",
                ran.charges
            );
        }

        // **対照——揃った採点表は、打ち切られていなければ読まれる。**
        let sub = MockSub::replying(&whole);
        let ran = run_once_in(&sub, USER_MESSAGE, StatDriftMode::On, DriftForm::All);
        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=scored"),
            "打ち切られていない採点表まで捨てている: {:?}",
            ran.output
        );
    }

    /// **打ち切られた回でも、引用記号を剥がした数は数える**（作業単位 16e-c の指標）。
    ///
    /// 測っているのは「**モデルが依頼の引用記号を答えへ持ち込む頻度**」なので、
    /// **その回が最終的に捨てられたかどうかは関係しない**
    /// （`DriftScoringReport::quotes_stripped` の doc「採否に関係なく数える」）。
    /// 丸ごと捨てるついでにこの数まで落とすと、**是正の効き目を測る指標が
    /// 打ち切りの回だけ静かに欠ける。**
    #[test]
    fn a_truncated_scorecard_still_reports_the_quotes_it_stripped() {
        let quoted =
            format!("> - {MOVED_AXIS} +1\n> - {TWIN_AXIS} 0\n- {STILL_AXIS} 0\n- {SILENT_AXIS} 0");
        let sub = MockSub::replying_with(&quoted, Some(FinishReason::Length));
        let ran = run_once_in(&sub, USER_MESSAGE, StatDriftMode::On, DriftForm::All);

        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=truncated"),
            "{:?}",
            ran.output
        );
        assert!(
            has_field(drift_record(&ran.output), "drift_quotes=2"),
            "打ち切られた回で剥がした記号の数が落ちている: {:?}",
            ran.output
        );
    }

    /// **停止理由を申告しない上流が居ることは、ログに残す**（24 と同じ作法）。
    ///
    /// この回は「**打ち切られたか分からない**」まま読んでいる
    /// （`response_completeness` は「切れていない」に倒す）。
    /// 残さないと、**その状態が起きていること自体に気付く手段がなくなる。**
    ///
    /// **申告があった回には出ないこと**まで見る。毎回出す実装は
    /// 「申告が無い上流に繋がっている」というシグナルを潰す。
    ///
    /// # 深刻度は**メッセージと同じ行**で見る（作業単位 24-2）
    ///
    /// `output.contains("WARN")` は、**その `WARN` がこのメッセージのものか**を
    /// 一切見ていない。この経路には**別の `warn`** が在りうるので、行ごと取り出して見る。
    ///
    /// # **どの往復のものか**も同じ行で見る（作業単位 B-1-2 / レビュー指摘1）
    ///
    /// **`session_id` を `character_id` に取り違える変異が、1510 全緑を通り抜けた**（実測）。
    /// この行は上に書いたとおり**唯一の気付く手段**なので、
    /// **id が違うと `drift_outcome` の行と突き合わせられず、「気付く手段」が成立しない。**
    /// CLAUDE.md「**抜けるのはいつも帰属**——載っているかではなく、どれのものか」。
    ///
    /// **`assert_ne!` を先に置く**——`fresh` が2つを別の値にしていることに
    /// 寄りかかっているので、その前提が崩れたら**ここが先に落ちる**形にする。
    #[test]
    fn a_drift_response_without_a_finish_reason_is_recorded() {
        let reply = format!("- {MOVED_AXIS} +1");

        let sub = MockSub::replying_with(&reply, None);
        let ran = run_once(&sub, USER_MESSAGE);

        let recorded = line_containing(&ran.output, NO_FINISH_REASON_MESSAGE);
        assert!(
            recorded.contains("WARN"),
            "申告が無い回は warn に残すこと（同じ行の深刻度を見る）: {recorded:?}"
        );
        // **どの往復のものかが、同じ行から読めること**（作業単位 B-1-2 / レビュー指摘1）。
        //
        // この行は「その状態が起きていること自体に気付く唯一の手段」なので、
        // **id を取り違えると `drift_outcome` の行と突き合わせられない。**
        // CLAUDE.md「抜けるのはいつも帰属——載っているかではなく、どれのものか」。
        assert_ne!(
            ran.session_id, ran.character_id,
            "前提: 往復とキャラクターの id が別であること（同値だと取り違えを観測できない）"
        );
        assert!(
            has_field(recorded, &format!("session_id={}", ran.session_id)),
            "申告が無い回の記録に、どの往復のものかが載っていない\
             （期待 session_id={} / character_id={} と取り違えていないか）: {recorded:?}",
            ran.session_id,
            ran.character_id
        );
        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=applied"),
            "申告が無いだけで捨てないこと: {:?}",
            ran.output
        );

        // **申告があった回には出ない。**
        let sub = MockSub::replying_with(&reply, Some(FinishReason::Length));
        let ran = run_once(&sub, USER_MESSAGE);
        assert!(
            !ran.output.contains(NO_FINISH_REASON_MESSAGE),
            "申告があった回に出さないこと: {:?}",
            ran.output
        );
    }

    /// **変容の記録は、どの往復のものかを自分の行に載せる**（作業単位 B-2）。
    ///
    /// # 何を捕まえるためのものか
    ///
    /// `record_run` / `record_skip` の `session_id` を `character_id` に取り違える変異は、
    /// **B-1-2 の是正の後もこのファイルのどのテストも見ていなかった**
    /// （B-1 のレビューで実測。塞いだのは「唯一の気付く手段」と名指しした
    /// [`a_drift_response_without_a_finish_reason_is_recorded`] の1行だけ）。
    ///
    /// **`session_id` は接頭辞を持たない共通の綴り**（[`record_run`] の doc）で、
    /// **同じ捕捉に抽出の記録が必ず並んでいる**——あちらも同じ往復の `session_id` を出す。
    /// つまり**出力全体に対して `has_field` で見ると、変容の行が何を書いていても
    /// 抽出の行が答えを供給して緑を通る。**
    /// CLAUDE.md「**抜けるのはいつも帰属**——載っているかではなく、どれのものか」。
    ///
    /// # 3つの枝を全部通す
    ///
    /// `record_skip` は**水準で分岐して同じ欄を2箇所に書いている**ので、
    /// 片方だけ見ると、もう片方を落とす変異が緑になる。
    ///
    /// | 枝 | 台本 | 水準 |
    /// |---|---|---|
    /// | [`record_run`] | 提案どおり1段動く | `info` |
    /// | [`record_skip`]（`cause` 無し） | 依頼が上限を超える | `info` |
    /// | [`record_skip`]（`cause` 有り） | 上流が落ちる | `warn` |
    ///
    /// # 水準も同じ行で見る（作業単位 B-2-2 / レビュー指摘 R-1）
    ///
    /// **上の表は B-2 で書いたのに、水準を見る assert がこのファイルに1本も無かった。**
    /// `record_skip` の `warn!` と `info!` を**入れ替える**変異が **1535 全緑**を通り抜けている
    /// （B-2 のレビューで実測）。
    /// **既定フィルタ（`warn,personacore=info`）ではどちらも出る**ので、
    /// **出力を眺めても違いが見えない**——故障（上流が落ちた回・性格ステータスが
    /// 読めない回）が設計どおりの結果に紛れて、率でしか読めなくなる。
    ///
    /// **深刻度の紛れは枝によって在ったり無かったりする**（2026-08-26 に捕捉を実測）:
    ///
    /// | 枝 | 同じ捕捉に並ぶ抽出の記録 | 出力全体で見たときの紛れ |
    /// |---|---|---|
    /// | 判定した回 | `INFO`（記憶を抽出しました） | **在る** |
    /// | 長すぎた回 | `INFO`（抽出も長すぎ） | **在る** |
    /// | 上流が落ちた回 | `INFO`（記憶を抽出しました） | **いまは無い** |
    ///
    /// **在る2つのために行へ絞ってある。** 3つめは、
    /// **この捕捉に `warn` が1本増えた瞬間に同じ形**になる
    /// （現に [`stats_that_cannot_be_read_skip_the_drift_and_keep_the_cause`] の
    /// 台本では抽出の側も `warn` を出す）。
    ///
    /// # 前提を明示する
    ///
    /// `fresh` が**会話しない方のキャラクターを先に作る**ので
    /// `session_id != character_id` になっているが、
    /// **それはフィクスチャの並びに寄りかかった性質**なので毎回確かめる
    /// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
    ///
    /// [`stats_that_cannot_be_read_skip_the_drift_and_keep_the_cause`]:
    ///     stats_that_cannot_be_read_skip_the_drift_and_keep_the_cause
    #[test]
    fn every_drift_record_names_its_turn_and_keeps_its_level() {
        /// 記録に出る深刻度の綴り。**この2つのどちらか**でしかない。
        const LEVELS: [&str; 2] = ["INFO", "WARN"];

        /// 1回ぶんの記録を、**手掛かりの行の中で**確かめる。
        ///
        /// `level` は[`LEVELS`]のどれか。**期待した水準が載っていること**と
        /// **もう一方が載っていないこと**の両方を見る——片側だけだと、
        /// 2つの枝を入れ替える変異が片方の枝で緑を通る。
        fn check(ran: &Ran, marker: &str, level: &str, what: &str) {
            assert!(LEVELS.contains(&level), "水準の綴りは {LEVELS:?} のどれか");
            assert_ne!(
                ran.session_id, ran.character_id,
                "前提: 往復とキャラクターの id が別であること（同値だと取り違えを観測できない）"
            );
            // **前提: 出力全体で見れば、間違った答えが手に入る。**
            // ここが偽なら、下の主張は「そもそも紛れが無い」を見ているだけになる。
            assert!(
                ran.output
                    .lines()
                    .filter(|line| has_field(line, &format!("session_id={}", ran.session_id)))
                    .count()
                    >= 2,
                "前提: 同じ欄を持つ行が他にもあること（{what}）: {:?}",
                ran.output
            );

            let recorded = line_containing(&ran.output, marker);
            assert!(
                has_field(recorded, &format!("session_id={}", ran.session_id)),
                "{what}に、どの往復のものかが載っていない\
                 （期待 session_id={} / character_id={} と取り違えていないか）: {recorded:?}",
                ran.session_id,
                ran.character_id
            );
            // **水準はこの行の中で見る**（doc の表 / 作業単位 B-2-2）。
            assert!(
                has_field(recorded, level),
                "{what}の水準が {level} でない: {recorded:?}"
            );
            for other in LEVELS.iter().filter(|other| **other != level) {
                assert!(
                    !has_field(recorded, other),
                    "{what}に別の水準（{other}）が載っている: {recorded:?}"
                );
            }
        }

        // 1. 判定した回（`record_run`）。
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let ran = run_once(&sub, USER_MESSAGE);
        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=applied"),
            "前提: 実際に判定した回であること: {:?}",
            ran.output
        );
        check(&ran, RAN_MESSAGE, "INFO", "判定した回の記録");

        // 2. 走らせなかった回・`info` の枝（依頼が長すぎる）。
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let ran = run_once(
            &sub,
            &"あ".repeat(FALLBACK_MAX_DRIFT_PROMPT_TOKENS as usize),
        );
        assert!(
            has_field(
                skip_record(&ran.output),
                &format!("drift_skipped={}", DriftSkipReason::TooLong.label())
            ),
            "前提: 長すぎて捨てた回であること: {:?}",
            ran.output
        );
        check(&ran, SKIPPED_MESSAGE, "INFO", "長すぎて捨てた回の記録");

        // 3. 走らせなかった回・`warn` の枝（上流が落ちた）。
        let sub = MockSub::failing_the_drift();
        let ran = run_once(&sub, USER_MESSAGE);
        assert!(
            has_field(
                skip_record(&ran.output),
                &format!("drift_skipped={}", DriftSkipReason::UpstreamFailed.label())
            ),
            "前提: 上流が落ちた回であること: {:?}",
            ran.output
        );
        check(&ran, SKIPPED_MESSAGE, "WARN", "上流が落ちた回の記録");
    }

    /// **停止理由が無いことの記録は、抽出のものと別の文言である**（作業単位 B-1）。
    ///
    /// **同じ文言にすると、どちらのサブLLM 呼び出しに停止理由が無かったのかが
    /// ログから読めない。** 変容と抽出は**同じターンで順に走る**ので、
    /// 両方が同じ行を出したら区別する手掛かりが残らない。
    ///
    /// **翻訳の関数（`response_completeness`）は共有する**が、**文言は共有しない。**
    #[test]
    fn the_two_missing_finish_reason_messages_never_contain_each_other() {
        let extraction = super::super::extract::NO_FINISH_REASON_MESSAGE;
        assert_ne!(
            NO_FINISH_REASON_MESSAGE, extraction,
            "変容と抽出の文言が同じでは、どちらの応答だったのか読めない"
        );
        assert!(
            !NO_FINISH_REASON_MESSAGE.contains(extraction)
                && !extraction.contains(NO_FINISH_REASON_MESSAGE),
            "片方がもう片方を含むと、素朴な grep が両方を数える"
        );
    }

    // ---------------------------------------------------------------------
    // 書き込みが届かなかった回
    // ---------------------------------------------------------------------

    /// **0 行更新は「動かなかった」として数えられ、エラーにならない**（ADR-0024 §7）。
    ///
    /// # 窓は「変容が軸を読んでから、書くまで」
    ///
    /// 門は**変容の依頼に掛ける。** そこで初めて
    /// 「軸は読み終わっている・まだ書いていない」状態が作れる。
    /// **抽出の依頼に掛けても観測できない**——変容はまだ軸を読んでいないので、
    /// 書き換えた後の値をそのまま読んで**普通に成功する**（実測。一度そう書いて踏んだ）。
    ///
    /// **提案そのものは正しい**（`outcome=applied`）ので、破棄とも飽和とも別の値になる。
    /// 区別が要る理由は、**「モデルが提案を出していない」と
    /// 「提案は出たが書き込みが届かなかった」で打ち手が違う**から。
    #[test]
    fn a_lost_race_is_counted_as_not_moved_and_is_not_an_error() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("race.sqlite")));

        let (sub, open) = MockSub::gating_the_drift(&format!("- {MOVED_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::On,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                // **変容の依頼**が門で止まるまで待つ（＝軸は読み終わっている）。
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while sub.call_count() < 2 {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "変容の依頼が上流へ届くこと"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                // **この窓で、管理 API が同じ軸を別の値へ動かす。**
                put_stats(
                    &fixture.store,
                    fixture.character.id,
                    &[(MOVED_AXIS, 6), (STILL_AXIS, STILL_START)],
                )
                .await;
                // 門を開けると、変容の依頼が投げられて答えが返る。
                open.send(()).expect("上流を開けること");
                extractor.wait_for_the_running_task().await;
            });
        });
        let after = runtime.block_on(stats(&fixture.store, fixture.character.id));
        runtime.block_on(fixture.store.close());

        assert_eq!(
            sub.call_count(),
            2,
            "前提: 変容も上流へ行くこと: {output:?}"
        );
        assert!(
            has_field(drift_record(&output), "drift_outcome=applied"),
            "提案そのものは正しいこと: {output:?}"
        );
        assert!(
            has_field(
                drift_record(&output),
                &format!("drift_moved={}", DriftWrite::NotMoved.label())
            ),
            "動かなかったこととして数えること: {output:?}"
        );
        assert!(
            !has_field(&output, &format!("drift_moved={WRITE_FAILED}")),
            "故障として数えないこと: {output:?}"
        );
        assert_eq!(
            after,
            vec![
                (MOVED_AXIS.to_owned(), 6),
                (STILL_AXIS.to_owned(), STILL_START)
            ],
            "窓の中で入った値をそのまま残すこと"
        );
    }

    /// **書き込みそのものが落ちた回を、「動かなかった」と同じ値で数えない。**
    ///
    /// # 変異検査で見つかった穴（2026-08-23）
    ///
    /// [`WRITE_FAILED`] を `DriftWrite::NotMoved.label()` に差し替える変異が
    /// **1282 passed / 0 failed で通った**（実測）——
    /// **「提案が適用された」「かつ書き込みが落ちた」という状態に
    /// どのテストも一度も入っていなかった。**
    /// 値は全部違っていて、欠けていたのは**状態**である
    /// （CLAUDE.md「フィクスチャが『その分岐が効く状態』に一度も入らない」）。
    ///
    /// 実害は軽くない。**その値は 4 つの印の中で唯一の故障の合図**であり、
    /// `not_moved` に化けると **DB が書けない回が「競合しただけ」に見える**
    /// （[`WRITE_FAILED`] の doc がまさにそう書いている）。
    ///
    /// # 状態の作り方
    ///
    /// **軸を読み終えた後、書き込む前に**プールを閉じる。
    /// 門を変容の依頼に掛けるのは [`a_lost_race_is_counted_as_not_moved_and_is_not_an_error`]
    /// と同じで、窓の中でやることだけが違う。
    #[test]
    fn a_write_that_fails_is_not_counted_as_a_quiet_zero() {
        /// `StoreError::Query` の `Display`。**定数文字列**なので原因を含まない。
        const QUERY_DISPLAY: &str = "データベース操作に失敗しました";
        /// `Debug` にだけ現れる原因。**`detail` を落とすと消える唯一の手掛かり。**
        const CLOSED_POOL_CAUSE: &str = "PoolClosed";

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("write-fails.sqlite")));

        let (sub, open) = MockSub::gating_the_drift(&format!("- {MOVED_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::On,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                // **変容の依頼**が門で止まるまで待つ（＝軸は読み終わっている）。
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while sub.call_count() < 2 {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "変容の依頼が上流へ届くこと"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                // **この窓で永続化層が落ちる。** 読みは済み、書きはこれから。
                fixture.store.close().await;
                open.send(()).expect("上流を開けること");
                extractor.wait_for_the_running_task().await;
            });
        });

        assert!(
            has_field(drift_record(&output), "drift_outcome=applied"),
            "前提: 提案そのものは適用できる形であること: {output:?}"
        );
        assert!(
            has_field(
                drift_record(&output),
                &format!("drift_moved={WRITE_FAILED}")
            ),
            "書き込みが落ちた回として数えること: {output:?}"
        );
        assert!(
            !has_field(
                &output,
                &format!("drift_moved={}", DriftWrite::NotMoved.label())
            ),
            "「動かなかった」と同じ値にしないこと（故障が競合に見える）: {output:?}"
        );
        assert!(
            output.contains(QUERY_DISPLAY),
            "`Display` をログに残すこと: {output:?}"
        );
        assert!(
            output.contains(CLOSED_POOL_CAUSE),
            "原因（`#[source]` 側）をログに残すこと: {output:?}"
        );
    }

    /// **票を残せなかった回を、「残した」と同じ値で数えない。**
    ///
    /// # 変異検査で見つかった穴（16d のレビュー / 2026-08-23）
    ///
    /// [`VOTE_FAILED`] を [`VOTE_RECORDED`] に差し替える変異が
    /// **1338 passed / 0 failed で通った**——
    /// **`the_absent_value_marker...` も `every_vote_marker_is_distinct` も
    /// 綴りが重ならないことしか見ておらず、
    /// 「票を残そうとして失敗した」という状態にどのテストも一度も入っていなかった。**
    /// 書き込みの側（[`a_write_that_fails_is_not_counted_as_a_quiet_zero`]）には
    /// 振る舞いを見るテストがあるのに、**票の側だけ対が欠けていた。**
    ///
    /// # 実害 —— **測定の分母が黙って埋まる**
    ///
    /// 記録専用モードでは**票が唯一の成果物**である（[`VOTE_FAILED`] の doc）。
    /// ADR-0026 の実測表「票の記録 72/72。失敗 0」は**この欄を数えて書いた。**
    /// この変異が入っていれば、**書けなかった回も `recorded` として 72/72 に数えられ、
    /// ログの数だけを見ても永久に気づけない。**
    ///
    /// # 状態の作り方
    ///
    /// [`a_write_that_fails_is_not_counted_as_a_quiet_zero`] と同じ手。
    /// **軸を読み終えた後、票を書く前に**プールを閉じる。
    /// **票の記録は書き込みより先に走る**ので、同じ窓で両方が落ちる。
    #[test]
    fn a_vote_that_cannot_be_recorded_is_not_counted_as_recorded() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("vote-fails.sqlite")));

        let (sub, open) = MockSub::gating_the_drift(&format!("- {MOVED_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        // **記録専用モードで測る。** 票だけが成果物である状態そのもの。
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::Shadow,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                // **変容の依頼**が門で止まるまで待つ（＝軸は読み終わっている）。
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while sub.call_count() < 2 {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "変容の依頼が上流へ届くこと"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                // **この窓で永続化層が落ちる。** 読みは済み、票の追記はこれから。
                fixture.store.close().await;
                open.send(()).expect("上流を開けること");
                extractor.wait_for_the_running_task().await;
            });
        });

        assert!(
            has_field(drift_record(&output), "drift_outcome=applied"),
            "前提: 票になる結末であること: {output:?}"
        );
        assert!(
            has_field(
                drift_record(&output),
                &format!("drift_recorded={VOTE_FAILED}")
            ),
            "票を残せなかった回として数えること: {output:?}"
        );
        assert!(
            !has_field(&output, &format!("drift_recorded={VOTE_RECORDED}")),
            "「残した」と同じ値にしないこと（故障が成果に見える）: {output:?}"
        );
        assert!(
            !has_field(&output, &format!("drift_recorded={NO_VOTE}")),
            "「票にならなかった」と同じ値にもしないこと: {output:?}"
        );
        assert!(
            output.contains("性格の変容の票を記録できませんでした"),
            "故障を別のイベントとして残すこと: {output:?}"
        );
    }

    /// **採点している最中にキャラクターが完全に消されたら、票は捨てて `error` にしない**
    /// （ADR-0049 §決定3 / 作業単位 A-10）。
    ///
    /// [`a_vote_that_cannot_be_recorded_is_not_counted_as_recorded`] と同じ窓（軸は読み終え、
    /// 票はこれから）で、プールを閉じる代わりに**しまってから消す。** 票は会話の縛りで落ちる
    /// （往復の会話がカスケードで消えている / `StoreError::TurnSessionGone`）。
    /// **残せなかった回としては数える**（票は残っていない）が、故障の行は出さず、
    /// `info` の [`super::super::extract::VANISHED_MESSAGE`] に `vanished_write=drift_vote` で残る。
    #[test]
    fn a_vote_for_a_deleted_character_is_dropped_without_an_error() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("vote-vanished.sqlite")));

        let (sub, open) = MockSub::gating_the_drift(&format!("- {MOVED_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::Shadow,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while sub.call_count() < 2 {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "変容の依頼が上流へ届くこと"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                // **この窓でキャラクターを完全に消す。** 読みは済み、票の追記はこれから。
                fixture
                    .store
                    .update_character(
                        &fixture.character.handle,
                        &crate::domain::CharacterUpdate {
                            archived: Some(true),
                            ..crate::domain::CharacterUpdate::default()
                        },
                    )
                    .await
                    .expect("しまえること");
                fixture
                    .store
                    .delete_character(&fixture.character.handle)
                    .await
                    .expect("しまったキャラクターは消せること");
                open.send(()).expect("上流を開けること");
                extractor.wait_for_the_running_task().await;
            });
        });

        assert!(
            has_field(
                drift_record(&output),
                &format!("drift_recorded={VOTE_FAILED}")
            ),
            "票を残せなかった回として数えること: {output:?}"
        );
        assert!(
            !output.contains("性格の変容の票を記録できませんでした"),
            "消えたキャラクターへの票の失敗を故障として出さないこと: {output}"
        );
        assert!(
            !output.contains("ERROR"),
            "error の行を1本も出さないこと: {output}"
        );
        let vanished = line_containing(&output, super::super::extract::VANISHED_MESSAGE);
        assert!(
            vanished.contains(" INFO ") && has_field(vanished, "vanished_write=drift_vote"),
            "捨てたことは info で残すこと: {vanished:?}"
        );
    }

    /// **消したキャラクターと同じ id で作り直されても、変容は新しいキャラクターに何も書かない**
    /// （ADR-0049 §決定3 / 作業単位 A-10 のレビュー前の差し戻し）。
    ///
    /// `on` で回す（票・値・累積の3つの書き込みが全部走る形）。採点の依頼が上流へ届いた
    /// （＝軸は読み終えた）ところで、しまって消して、**同じ識別子・同じ尺度・同じ軸と値**で
    /// 作り直す。縛りが無ければ、値の文は「同じ軸・同じ値」に当たって新しいキャラクターの
    /// 値を動かし、累積も新しいキャラクターの行として入る。
    ///
    /// **id が実際に同じであることを前提として確かめる**（使い回されなければ何も見ていない）。
    #[test]
    fn drift_of_a_deleted_character_never_reaches_a_new_character_with_the_same_id() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("drift-reused-id.sqlite")));

        let (sub, open) = MockSub::gating_the_drift(&format!("- {MOVED_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::On,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let mut reborn: Option<Character> = None;
        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while sub.call_count() < 2 {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "変容の依頼が上流へ届くこと"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                fixture
                    .store
                    .update_character(
                        &fixture.character.handle,
                        &crate::domain::CharacterUpdate {
                            archived: Some(true),
                            ..crate::domain::CharacterUpdate::default()
                        },
                    )
                    .await
                    .expect("しまえること");
                fixture
                    .store
                    .delete_character(&fixture.character.handle)
                    .await
                    .expect("しまったキャラクターは消せること");
                let created = create_character(&fixture.store, HANDLE, DISPLAY_NAME, SCALE).await;
                put_stats(
                    &fixture.store,
                    created.id,
                    &[
                        (MOVED_AXIS, MOVED_START),
                        (TWIN_AXIS, MOVED_START),
                        (STILL_AXIS, STILL_START),
                        (SILENT_AXIS, 0),
                    ],
                )
                .await;
                fixture
                    .store
                    .create_session(created.id)
                    .await
                    .expect("新しいキャラクターにも会話を作れること");
                reborn = Some(created);
                open.send(()).expect("上流を開けること");
                extractor.wait_for_the_running_task().await;
            });
        });

        let reborn = reborn.expect("作り直したキャラクターがあること");
        assert_eq!(
            reborn.id, fixture.character.id,
            "前提: id が使い回されていること（されなければ、このテストは何も見ていない）"
        );
        assert!(
            has_field(drift_record(&output), "drift_outcome=applied"),
            "前提: 値を動かす結末であること: {output:?}"
        );
        assert_eq!(
            runtime.block_on(stats(&fixture.store, reborn.id)),
            vec![
                (MOVED_AXIS.to_owned(), MOVED_START),
                (TWIN_AXIS.to_owned(), MOVED_START),
                (STILL_AXIS.to_owned(), STILL_START),
                (SILENT_AXIS.to_owned(), 0),
            ],
            "新しいキャラクターの値は1つも動かないこと"
        );
        assert!(
            runtime
                .block_on(votes(&fixture.store, reborn.id))
                .is_empty(),
            "新しいキャラクターに票が入らないこと"
        );
        assert!(
            runtime
                .block_on(charges(&fixture.store, reborn.id))
                .is_empty(),
            "新しいキャラクターに累積が入らないこと"
        );
        assert!(
            !output.contains("ERROR"),
            "error の行を1本も出さないこと: {output}"
        );
        for write in ["drift_vote", "stat_value", "drift_charge"] {
            assert!(
                output.lines().any(
                    |line| line.contains(super::super::extract::VANISHED_MESSAGE)
                        && has_field(line, &format!("vanished_write={write}"))
                ),
                "{write} を捨てたことを残すこと: {output}"
            );
        }
        runtime.block_on(fixture.store.close());
    }

    // ---------------------------------------------------------------------
    // 走らなかった回
    // ---------------------------------------------------------------------

    /// **軸が1本も定義されていなければ、上流を呼ばない**（ADR-0027 §決定6）。
    ///
    /// 空の節を出すとモデルは名前を捏造し、節ごと落として投げると
    /// **答えの決まっている問いに GPU を使う。**
    ///
    /// # ⚠ **「全軸が 0」はもうここに来ない**（16e-a / ADR-0027 §決定6）
    ///
    /// 以前はこのテストが全軸 0 のキャラクターで測っていた。
    /// **いまはそれが「走る」側**である（[`a_character_whose_axes_are_all_zero_is_still_asked`]）
    /// ——ゼロ始まりを止めると、育て始めることが原理的にできない。
    #[test]
    fn a_character_without_any_stored_axis_is_never_asked() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("no-axes.sqlite")));

        // **軸を1本も持たせない。**
        runtime.block_on(put_stats(&fixture.store, fixture.character.id, &[]));

        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::On,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                extractor.wait_for_the_running_task().await;
            });
        });
        let after = runtime.block_on(stats(&fixture.store, fixture.character.id));
        runtime.block_on(fixture.store.close());

        assert_eq!(sub.call_count(), 1, "変容の依頼を投げないこと: {output:?}");
        assert!(
            has_field(
                skip_record(&output),
                &format!("drift_skipped={}", DriftSkipReason::NoAxes.label())
            ),
            "軸が無かった回として数えること: {output:?}"
        );
        assert!(
            !output.contains(RAN_MESSAGE),
            "判定した回として数えないこと（提案率の分母に入れない）: {output:?}"
        );
        assert!(after.is_empty(), "軸が生えないこと: {after:?}");
    }

    /// **全軸が 0 のキャラクターにも依頼を投げる**（ADR-0027 §決定4 / §決定6）。
    ///
    /// # ここを止めると、ゼロ始まりが黙って動かない
    ///
    /// ADR-0027 §11 の実測——**中間から始めると均され、ゼロから始めると分化する。**
    /// 全部ゼロは使い方の1つではなく、**この機構が実際に分化する配置**である。
    ///
    /// **上流を呼ぶだけでなく、値が実際に 0 から 1 へ動く**ところまで見る
    /// ——依頼だけ投げて `domain` の側で捨てる形（旧 §4 の門）が残っていたら、
    /// **提案は出ているのに一度も動かない**という、いちばん気づきにくい壊れ方になる。
    #[test]
    fn a_character_whose_axes_are_all_zero_is_still_asked() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("all-zero.sqlite")));

        runtime.block_on(put_stats(
            &fixture.store,
            fixture.character.id,
            &[(MOVED_AXIS, 0), (STILL_AXIS, 0)],
        ));

        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::On,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                extractor.wait_for_the_running_task().await;
            });
        });
        let after = runtime.block_on(stats(&fixture.store, fixture.character.id));
        runtime.block_on(fixture.store.close());

        assert_eq!(sub.call_count(), 2, "抽出と変容で2回であること: {output:?}");
        assert!(
            sub.drift_prompt()
                .contains(&format!("{MOVED_AXIS}: 0/{SCALE}")),
            "値 0 の軸が依頼に載ること: {:?}",
            sub.drift_prompt()
        );
        assert!(
            !has_field(
                &output,
                &format!("drift_skipped={}", DriftSkipReason::NoAxes.label())
            ),
            "軸が無かった回として数えないこと: {output:?}"
        );
        assert_eq!(
            after,
            vec![(MOVED_AXIS.to_owned(), 1), (STILL_AXIS.to_owned(), 0)],
            "ゼロから 1 段育つこと"
        );
    }

    /// **性格ステータスが読めない回は、変容を飛ばして数える。**
    ///
    /// 許可リストそのものが手に入らないので走らせない。
    /// **原因は `#[source]` 側にしか無い**（`StoreError::Display` は定数文字列）ので、
    /// `error` と `detail` を落とすと調査の手掛かりが消える。
    #[test]
    fn stats_that_cannot_be_read_skip_the_drift_and_keep_the_cause() {
        /// `StoreError::Query` の `Display`。**定数文字列**なので原因を含まない。
        const QUERY_DISPLAY: &str = "データベース操作に失敗しました";
        /// `Debug` にだけ現れる原因。**`detail` を落とすと消える唯一の手掛かり。**
        const CLOSED_POOL_CAUSE: &str = "PoolClosed";

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("closed.sqlite")));
        // **プールを閉じる**（＝ `stat_allocs` が `StoreError::Query` を返す状態）。
        runtime.block_on(fixture.store.close());

        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::On,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                extractor.wait_for_the_running_task().await;
            });
        });

        assert_eq!(sub.call_count(), 0, "上流を呼ばないこと: {output:?}");

        // **原因は変容の記録と同じ行で見る**（作業単位 B-2）。
        //
        // **`error` / `detail` は接頭辞を持たない共通の綴り**（`record_skip` の doc）で、
        // **この台本では抽出の側も同じプールで転んで同じ2欄を出す。**
        // 出力全体で見ると、**変容の記録から `error` / `detail` を落としても、
        // 抽出の行がそのまま答えを供給して緑を通る。**
        let skipped = skip_record(&output);
        assert!(
            has_field(
                skipped,
                &format!("drift_skipped={}", DriftSkipReason::StatsUnreadable.label())
            ),
            "読めなかった回として数えること: {skipped:?}"
        );
        // **深刻度も同じ行の中で見る**（作業単位 B-2-2 / レビュー指摘 R-1）。
        //
        // `record_skip` は水準で分岐しており、**故障だけが `warn`**（あちらの doc の表）。
        // ここが `info` に落ちても**既定フィルタ（`warn,personacore=info`）では
        // どちらも出る**ので、ログを眺めても気付けない。
        //
        // **前提: 出力全体で見れば、間違った答えが手に入る。**
        // この台本では抽出の側も同じ閉じたプールで転んで `warn` を出すので、
        // `output.contains("WARN")` は変容の記録が何を書いていても満たされる。
        assert!(
            output
                .lines()
                .filter(|line| line.contains("WARN") && !line.contains(SKIPPED_MESSAGE))
                .count()
                >= 1,
            "前提: 変容の記録以外にも WARN の行があること（紛れが無いと行に絞った意味が測れない）: {output:?}"
        );
        assert!(
            has_field(skipped, "WARN"),
            "故障の枝は warn に残すこと（同じ行の深刻度を見る）: {skipped:?}"
        );
        assert!(
            skipped.contains(QUERY_DISPLAY),
            "`Display` をログに残すこと: {skipped:?}"
        );
        assert!(
            skipped.contains(CLOSED_POOL_CAUSE),
            "原因（`#[source]` 側）をログに残すこと: {skipped:?}"
        );
    }

    /// **上流が落ちた回も数える。本文には影響しない。**
    ///
    /// 数えないと、**提案率の分母から漏れて提案率が実際より高く出る。**
    #[test]
    fn an_upstream_failure_is_counted_and_leaves_the_values_alone() {
        let sub = MockSub::failing_the_drift();
        let ran = run_once(&sub, USER_MESSAGE);

        assert_eq!(
            sub.call_count(),
            2,
            "変容の依頼は投げること: {:?}",
            ran.output
        );
        assert!(
            has_field(
                skip_record(&ran.output),
                &format!("drift_skipped={}", DriftSkipReason::UpstreamFailed.label())
            ),
            "上流が落ちた回として数えること: {:?}",
            ran.output
        );
        assert!(
            !ran.output.contains(RAN_MESSAGE),
            "判定した回として数えないこと: {:?}",
            ran.output
        );
        assert_eq!(ran.stats, untouched(), "値が動かないこと");
    }

    // ---------------------------------------------------------------------
    // 全置換を呼ばないことの走査
    // ---------------------------------------------------------------------

    /// **変容の経路が、全置換の関数名をどこにも綴っていない**（ADR-0024 §7）。
    ///
    /// # なぜテキスト走査なのか
    ///
    /// 型では止まらない——`Store` は同じ1つで、全置換の関数は `pub` である。
    /// **呼べてしまう。** 呼ぶと「全部読む → 1つ変える → 全部書く」に戻り、
    /// その窓で入った他の軸の更新が**エラーにならずに巻き戻る。**
    ///
    /// # この走査が守れる範囲
    ///
    /// **[`scanned_paths`] のファイルに、その綴りが1文字でも現れたら落ちる。**
    /// 呼び出しでも、`use` でも、コメントでも同じ。
    ///
    /// # 守れないもの（把握したうえで残している）
    ///
    /// * **走査対象の外に変容の処理を書くこと。** 走査の外側そのもの。
    /// * **`store` 側に別名を作ること**（`fn replace_all(..) { self.全置換(..) }`）。
    ///   `store::drift` に入口を足せば、外側は綴らずに全置換へ届く。
    /// * **`server::extract` から呼ぶこと。** あちらは変容の経路ではないが、
    ///   同じタスクの中にいる。**走査対象に入れてある**ので現時点では落ちる。
    #[test]
    fn the_drift_path_never_reaches_for_the_wholesale_replacement() {
        let (scanned, found) = files_spelling(&scanned_sources(), &wholesale_spelling());

        assert!(
            scanned >= 3,
            "走査対象が少なすぎるなら、この assert は何も守っていない: {scanned} 件"
        );
        assert!(
            found.is_empty(),
            "変容の経路から全置換へ届いています（他の軸の更新が巻き戻ります）:\n{}",
            found.join("\n")
        );
    }

    /// **走査本体が現に綴りを見つけられる**ことの対照。
    ///
    /// 「1件も見つからないから緑」と「そもそも探せていないから緑」を区別する。
    ///
    /// # 対照は**植えたソース**を読む（15b の実測）
    ///
    /// 実ファイルを読む対照は**自己成就する**——走査本体の綴りを別の語に変える変異は、
    /// **その語を走査本体のあるファイル（＝このファイル）に書き込む**ので、
    /// 実ファイル走査は「1件見つけた」と言って緑のまま通る。
    ///
    /// # ここで塞いでいる3つ
    ///
    /// 1. **綴りが本物の関数名を指しているか**（定数だけを書き換える変異）。
    /// 2. **植えた呼び出しを、見張り番と同じ本体が見つけるか。**
    /// 3. **無関係な行を見つけないか**（無条件 true への変異）。
    #[test]
    fn the_scan_would_notice_the_wholesale_replacement() {
        let spelling = wholesale_spelling();

        // 1. 綴りが本物の関数名を指していること。
        let owner = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("store")
                .join("character.rs"),
        )
        .expect("全置換の定義を読めること");
        assert!(
            owner.contains(&format!("pub async fn {spelling}(")),
            "その綴りの関数が実在しない（走査の綴りが古い）: {spelling}"
        );

        // 2 と 3. 植えたソースを、見張り番と同じ本体に通す。
        let planted = planted_sources(&spelling);
        let (scanned, found) = files_spelling(&planted, &spelling);
        assert_eq!(scanned, planted.len(), "植えた分を全部見ること");
        assert_eq!(
            found.len(),
            1,
            "植えた呼び出しをちょうど1件見つけること（探し方が壊れている）: {found:?}"
        );
        assert!(
            found[0].ends_with("planted-call.rs"),
            "見つけたのが呼び出しを植えたファイルではない: {found:?}"
        );
    }

    /// 全置換の関数名。**ソース上にそのまま綴らない**
    /// （綴ると、この走査が自分自身を落とす。`store::memory::table_word` と同じ作法）。
    fn wholesale_spelling() -> String {
        ["replace_stat", "_allocs"].concat()
    }

    /// **走査本体。** 見張り番も対照も**この関数を通す**こと。
    ///
    /// **ソースを引数で受け取る**——実ファイルと植えたソースの両方を同じ判定に通すため
    /// （`prompt::relevance` の `files_spelling_the_accessor` と同じ形）。
    fn files_spelling(sources: &[(String, String)], needle: &str) -> (usize, Vec<String>) {
        let mut scanned = 0;
        let mut found = Vec::new();
        for (path, source) in sources {
            scanned += 1;
            if source.contains(needle) {
                found.push(path.clone());
            }
        }
        (scanned, found)
    }

    /// 走査対象。**変容が通るファイルだけ。**
    ///
    /// 全置換そのもの（`src/store/character.rs`）と管理 API（`src/server/admin.rs`）は
    /// **正当な呼び出し元**なので入れない。
    const SCANNED_PATHS: [[&str; 3]; 3] = [
        ["src", "server", "drift.rs"],
        ["src", "store", "drift.rs"],
        ["src", "server", "extract.rs"],
    ];

    /// 走査対象のパス一覧。
    fn scanned_paths() -> Vec<std::path::PathBuf> {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        SCANNED_PATHS
            .iter()
            .map(|parts| {
                let mut path = manifest.to_path_buf();
                for part in parts {
                    path.push(part);
                }
                path
            })
            .collect()
    }

    /// 走査対象を「パス, 生ソース」で返す。**読めなければ落とす。**
    fn scanned_sources() -> Vec<(String, String)> {
        scanned_paths()
            .into_iter()
            .map(|path| {
                let source = std::fs::read_to_string(&path)
                    .unwrap_or_else(|err| panic!("{} を読めること: {err}", path.display()));
                (path.display().to_string(), source)
            })
            .collect()
    }

    /// 対照のために**植えた**ソース一式。**実ファイルは読まない。**
    ///
    /// 綴りは引数で受け取り、直接は書かない
    /// ——写しを作ると、綴りを壊す変異が対照ごと道連れになって緑になる。
    ///
    /// > **⚠ ここのリテラルに `mem` + `ory` の語を入れないこと。**
    /// > `store::memory` の走査（`no_statement_updates_or_deletes_a_memory`）は
    /// > `src/**` の文字列リテラルからテーブル名の言及を拾うので、
    /// > 植えたパスにその名前を書いた時点で**そちらが赤くなる**。
    fn planted_sources(spelling: &str) -> Vec<(String, String)> {
        vec![
            (
                "planted-call.rs".to_owned(),
                format!("store.{spelling}(id, &allocs).await?;"),
            ),
            (
                "planted-single.rs".to_owned(),
                "store.apply_stat_drift(id, &change).await?;".to_owned(),
            ),
            (
                "planted-unrelated.rs".to_owned(),
                "let axes = prompt.included_axes();".to_owned(),
            ),
        ]
    }

    // ---------------------------------------------------------------------
    // ログの欄名の走査（ADR-0024「ログの数だけで判別できる形」）
    // ---------------------------------------------------------------------

    /// 変容のイベントが出す欄名。**接頭辞つきの綴りを1箇所に置く。**
    ///
    /// **`session_id` / `error` / `detail` は入れない。** あれらは横断して引くための
    /// 共通の綴りで、集計の分母にはならない（`record_run` の doc）。
    const DRIFT_FIELDS: [&str; 19] = [
        "drift_outcome",
        "drift_moved",
        // **16e-b で増えた5つ。** 追加時にここへ足し忘れると、
        // 「欄が本当に出ている」検査にも「他のモジュールと重ならない」走査にも掛からない。
        "drift_steps",
        "drift_ups",
        "drift_downs",
        "drift_saturations",
        "drift_quotes",
        "drift_recorded",
        "drift_run_mode",
        // **A-8 で増えた1つ**（走らせ方の出どころ / ADR-0045）。
        "drift_mode_source",
        "drift_axes",
        "drift_axis",
        "drift_before",
        "drift_after",
        "drift_delta",
        "drift_raw",
        "drift_skipped",
        "drift_prompt_tokens",
        "drift_limit_tokens",
    ];

    /// **欄名が本当に出ている。** 綴りだけ直して出し忘れる形を塞ぐ。
    ///
    /// 出るのは「判定した」側と「判定しなかった」側で分かれるので、
    /// **両方のイベントを1回ずつ起こして、[`DRIFT_FIELDS`] の全部が
    /// どちらかの行に揃うことを見る。**
    ///
    /// # 見るのは**行**であって、捕まえた出力の塊ではない（作業単位 B-21e）
    ///
    /// **名前が「a log line」と言っている通りに、行で見る。**
    /// B-21e より前は塊に `contains("キー=")` を当てており、主張が2つの意味で緩んでいた:
    ///
    /// | 緩み方 | 通り抜ける変異 |
    /// |---|---|
    /// | **キーがトークンの先頭にあることを見ない** | 欄名を `x_` で伸ばす（`x_drift_raw=` は `drift_raw=` を含む） |
    /// | **どの行に出たかを見ない** | 変容の2つのイベント以外の行がその欄を持てば満たされる |
    ///
    /// **1つ目は実測してある**（2026-09-02 / B-21e）——[`record_run`] の
    /// `drift_raw` を `x_drift_raw` へ伸ばす変異は、**着手前は 1802 全緑**だった。
    /// **他の見張り番も1本も赤くならない** ——
    /// [`the_drift_fields_never_collide_with_another_module`] は**他のモジュールしか走査せず**、
    /// [`the_field_scan_would_notice_a_collision`] は [`DRIFT_FIELDS`] の定数そのものを読むので
    /// **自己成就する**（あちらの doc が自分でそう書いている）。
    /// **マスクされていたのではなく、誰も見ていなかった。**
    ///
    /// 2つ目は、いまは別の見張り番が塞いでいる ——
    /// `src/**` の他のモジュールがこの綴りを持てば、上の走査が落ちる。
    /// **それでも行で見る。** あちらは**ソースの走査**であって、
    /// **出た行の話を一切していない**（`tests/**` も走査していない）。
    ///
    /// # 道具は [`has_key`]（値は見ない）
    ///
    /// 言いたいのは「**そのキーが、値によらず在る**」なので、
    /// 値まで名指しする [`has_field`] では書けない。
    /// **使い分けの権威は `crate::testlog` の `has_field` の doc の表。**
    #[test]
    fn every_drift_field_actually_appears_in_a_log_line() {
        // 判定した側（適用）。
        let applied = run_once(
            &MockSub::replying(&format!("- {MOVED_AXIS} +1")),
            USER_MESSAGE,
        );
        // 判定しなかった側（長すぎる）。
        let skipped = run_once(
            &MockSub::replying("なし"),
            &"あ".repeat(FALLBACK_MAX_DRIFT_PROMPT_TOKENS as usize),
        );
        // **変容の2つのイベントの行そのもの。** 塊ではない。
        let records = [drift_record(&applied.output), skip_record(&skipped.output)];

        for field in DRIFT_FIELDS {
            assert!(
                records.iter().any(|line| has_key(line, field)),
                "{field} がどちらのイベントの行にも出ていない: {records:?}"
            );
        }
    }

    /// **記録の行**から `drift_raw=` の**値そのもの**を取り出す（`Debug` の引用符の中）。
    ///
    /// # 判定は共有の1本に預けてある（作業単位 B-32）
    ///
    /// **B-32 より前は、ここに素の `find("drift_raw=\"")` を書いていた。**
    /// **キーがトークンの先頭にあることを見ないので、
    /// 欄名に接頭辞を足す変異が、この道具を使う4本を全部緑のまま通した**
    /// （2026-09-02 / B-21e の変異検査。**呼び出しは9箇所、テストは4本**）。
    /// **是正後は同じ変異が 5 赤**（その4本と
    /// [`every_drift_field_actually_appears_in_a_log_line`]）。
    ///
    /// **`crate::testlog` の道具の使い分けに寄せた** ——
    /// 値に空白が入る（`Debug` の文字列）ので [`has_field`] では読めず、
    /// `[` で始まらないので `bracketed_field` でも読めない。
    /// **`quoted_field` がその形の権威**で、規則も見張り番もあちらにある。
    ///
    /// **引数は行。** 塊を渡すと、[`quoted_field`] の規則 3（閉じ引用符でトークンが終わる）が
    /// 改行をまたいだ判定になる（[`drift_record`] の doc と同じ理由）。
    ///
    /// **読めなければ落とす。** `Option` を返して呼び出し側が畳むと、
    /// **欄が丸ごと消えた変異が「読めなかったので何も主張しない」で緑**になる。
    fn raw_field(line: &str) -> String {
        quoted_field(line, "drift_raw")
            .unwrap_or_else(|| panic!("生出力の欄を読めること: {line:?}"))
            .to_owned()
    }

    /// **生出力の欄を、名前が伸びた別の欄から読まない**（作業単位 B-32）。
    ///
    /// # なぜ要るのか（**実測された穴**）
    ///
    /// B-21e の変異検査で、[`record_run`] の `drift_raw` を `x_drift_raw` へ伸ばす変異が
    /// **[`raw_field`] を使う4本を全部緑のまま通した。**
    /// 素の `find` は `x_drift_raw="` の中に `drift_raw="` を見つけ、
    /// **囮の値をそのまま返していた**（`crate::testlog` の `has_field` の doc の
    /// 「キーの側が別物なのに当たる」形）。
    ///
    /// **「誰も見ていなかった」ではない** ——
    /// [`every_drift_field_actually_appears_in_a_log_line`] が **1 赤**で捕まえていた。
    /// **あちらは「その欄が在る」を見る道具**で、**値を取り出す側は無防備**だった。
    ///
    /// # 入力は**植えた行**（`tracing` を通さない）
    ///
    /// 本番のイベントに囮の欄は作れない。**行も期待値もべた書き**で、
    /// [`raw_field`] を通して期待値を組み立てない。
    ///
    /// # ⚠ 囮の位置は**前後の両方**で測る
    ///
    /// 素の `find` は**先に現れた一致**を返すので、
    /// **囮が後ろにあるだけの入力では、壊れた実装でも偶然正しく答える。**
    ///
    /// # 落ちる側の対照は別の1本
    ///
    /// [`the_raw_field_refuses_a_line_that_only_has_a_longer_name`] ——
    /// `should_panic` は「落ちた」しか言わないので、
    /// **落ちてはいけない入力を見るこちら**と対にしてある。
    #[test]
    fn the_raw_field_is_read_from_its_own_column() {
        // **囮が先**——素の `find` はここで囮の中身を返す。
        let decoy_first = r#"INFO 記録 x_drift_raw="囮" drift_raw="本物" session_id=7"#;
        // **囮が後ろ**——壊れた実装でも偶然正しく答える形（対照）。
        let decoy_last = r#"INFO 記録 drift_raw="本物" x_drift_raw="囮" session_id=7"#;

        assert_eq!(
            raw_field(decoy_first),
            "本物",
            "名前が伸びた欄を飛び越えて、自分の欄を読むこと"
        );
        assert_eq!(
            raw_field(decoy_last),
            "本物",
            "囮が後ろにあっても、自分の欄を読むこと"
        );

        // **エスケープした引用符は値の一部**（`Debug` の書き方そのもの）。
        assert_eq!(
            raw_field(r#"INFO 記録 drift_raw="a\"b" session_id=7"#),
            r#"a\"b"#,
            "エスケープした引用符で値を切らないこと"
        );
        // **行末にあっても読める**（閉じ引用符の後ろに何も無い）。
        assert_eq!(
            raw_field(r#"INFO 記録 session_id=7 drift_raw="末尾""#),
            "末尾",
            "行末の欄を読めること"
        );
    }

    /// **名前が伸びた欄しか無い行は、値を捏造せずに落ちる**（作業単位 B-32）。
    ///
    /// 落ちない側の対照は [`the_raw_field_is_read_from_its_own_column`]
    /// （CLAUDE.md「`should_panic` は落ちたしか言わない」）。
    #[test]
    #[should_panic(expected = "生出力の欄")]
    fn the_raw_field_refuses_a_line_that_only_has_a_longer_name() {
        let _ = raw_field(r#"INFO 記録 x_drift_raw="囮" session_id=7"#);
    }

    /// **生出力が、成功した回にも破棄した回にもログへ載る**（腕F の穴 / ADR-0027 §12）。
    ///
    /// # 破棄時だけでは足りない
    ///
    /// §12 の問いは「**破棄されなかった回に `0` が何個あったか**」なので、
    /// **提案が通った回の中身**が要る。破棄時だけ載せる実装はここで赤くなる。
    ///
    /// # 期待値は「上流に書かせた文字列」そのもの
    ///
    /// 定数からも上限からも作らない。**モックへ渡した綴りが、そのままログに出ること**を見る。
    #[test]
    fn the_raw_response_is_logged_on_both_the_kept_and_the_discarded_turn() {
        let kept = format!("- {MOVED_AXIS} +1");
        let broken = "これは形式に合わない文章です";
        assert_ne!(kept, broken, "2つを同じ文字列にしない");

        let applied = run_once(&MockSub::replying(&kept), USER_MESSAGE);
        assert!(
            has_field(drift_record(&applied.output), "drift_outcome=applied"),
            "前提: 提案が通る回であること: {:?}",
            applied.output
        );
        assert_eq!(
            raw_field(drift_record(&applied.output)),
            kept,
            "通った回の生出力が載っていない: {:?}",
            applied.output
        );

        let discarded = run_once(&MockSub::replying(broken), USER_MESSAGE);
        assert!(
            has_field(drift_record(&discarded.output), "drift_outcome=malformed"),
            "前提: 捨てられる回であること: {:?}",
            discarded.output
        );
        assert_eq!(
            raw_field(drift_record(&discarded.output)),
            broken,
            "捨てた回の生出力が載っていない: {:?}",
            discarded.output
        );

        // **「なし」の回も載る**（提案が無かったことと、何と書かれたかは別の情報）。
        let nothing = run_once(&MockSub::replying("なし"), USER_MESSAGE);
        assert!(has_field(
            drift_record(&nothing.output),
            "drift_outcome=nothing"
        ));
        assert_eq!(raw_field(drift_record(&nothing.output)), "なし");
    }

    /// **長い応答は切り詰められ、印が付く**（[`DRIFT_RAW_TRUNCATED`]）。
    ///
    /// # ⚠ **期待値を上限の定数から作らない**（CLAUDE.md / 16b と 16e-a で2回踏んでいる）
    ///
    /// `DRIFT_RAW_LOG_CHARS` を使って期待値を組むと、
    /// **上限を動かす変異は両側が一緒に動いて緑になる。** ここは3つの外側の値で挟む:
    ///
    /// 1. **桁の違う2つの長い応答**（1万文字と10万文字）で、
    ///    **載った長さが同じ**になること —— 入力の長さに追随したら切り詰めていない。
    /// 2. **1欄が 1024 文字を超えない**という、**この見張り番が独立に置く上限。**
    ///    `all` の応答上限は軸の本数ぶん伸びる（数千文字になりうる）ので、
    ///    「切り詰めている」を「少しだけ削っている」の言い換えにさせない。
    /// 3. **短い応答は丸ごと載り、印が付かない** —— 上限を極端に小さくする変異を止める。
    #[test]
    fn a_long_response_is_truncated_and_marked() {
        let long = "あ".repeat(10_000);
        let longer = "あ".repeat(100_000);

        let short_ran = run_once(&MockSub::replying(&long), USER_MESSAGE);
        let long_ran = run_once(&MockSub::replying(&longer), USER_MESSAGE);

        let first = raw_field(drift_record(&short_ran.output));
        let second = raw_field(drift_record(&long_ran.output));

        assert!(
            first.contains(DRIFT_RAW_TRUNCATED),
            "切り詰めた印が無い: {first:?}"
        );
        assert!(
            second.contains(DRIFT_RAW_TRUNCATED),
            "切り詰めた印が無い: {second:?}"
        );
        assert_eq!(
            first, second,
            "応答が10倍になったのに載る長さが変わった（切り詰めていない）"
        );
        assert!(
            first.chars().count() <= 1_024,
            "1欄が長すぎる（{}文字）: {first:?}",
            first.chars().count()
        );
        assert!(
            first.chars().count() < long.chars().count() / 4,
            "元の応答に対して削れていない（{}文字）",
            first.chars().count()
        );

        // **短い応答は丸ごと載り、印が付かない。**
        let kept = format!("- {MOVED_AXIS} +1");
        let ran = run_once(&MockSub::replying(&kept), USER_MESSAGE);
        let value = raw_field(drift_record(&ran.output));
        assert_eq!(value, kept, "短い応答が削られた");
        assert!(
            !value.contains(DRIFT_RAW_TRUNCATED),
            "削っていないのに印が付いた: {value:?}"
        );
    }

    /// **切り詰めの印は、上限を1文字でも超えたら付く**（境界）。
    ///
    /// # ⚠ 変異検査で見つかった穴（16e-a の追加分 / n8）
    ///
    /// 判定を `nth(上限 + 1)` に1つずらす変異は、
    /// **ちょうど「上限 + 1 文字」の応答だけを、印なしで1文字削る。**
    /// 上の見張り番は 1万文字と1文字の入力しか通らないので、
    /// **その1点を一度も踏んでいなかった**（CLAUDE.md「フィクスチャがその分岐が効く状態に
    /// 一度も入らない」）。
    ///
    /// # ここでは上限の**値**を守っていない。守るのは**境目**である
    ///
    /// 入力の長さは上限から作る（境界に立つにはそれしかない）が、
    /// **期待値は上限から作らない** ——「ちょうどなら丸ごと・無印」「1文字超えたら印」は
    /// 上限がいくつであっても成り立つ主張なので、
    /// **上限を動かす変異はここでは緑のまま**でよい。
    /// 値の側は [`a_long_response_is_truncated_and_marked`] が外側の数で挟んでいる。
    #[test]
    fn the_truncation_mark_appears_as_soon_as_the_limit_is_passed() {
        // **ちょうど上限**——丸ごと載り、印は付かない。
        let exact = "あ".repeat(DRIFT_RAW_LOG_CHARS);
        let ran = run_once(&MockSub::replying(&exact), USER_MESSAGE);
        let value = raw_field(drift_record(&ran.output));
        assert_eq!(value, exact, "上限ちょうどの応答が削られた");
        assert!(
            !value.contains(DRIFT_RAW_TRUNCATED),
            "削っていないのに印が付いた: {}文字",
            value.chars().count()
        );

        // **1文字超え**——印が付き、載る本文は上限ちょうどまで。
        let over = "あ".repeat(DRIFT_RAW_LOG_CHARS + 1);
        let ran = run_once(&MockSub::replying(&over), USER_MESSAGE);
        let value = raw_field(drift_record(&ran.output));
        assert!(
            value.contains(DRIFT_RAW_TRUNCATED),
            "1文字超えたのに印が付かない: {}文字",
            value.chars().count()
        );
        assert!(
            value.starts_with(&exact),
            "載った本文が、上限ちょうどまでの写しになっていない"
        );
        assert_eq!(
            value,
            format!("{exact}{DRIFT_RAW_TRUNCATED}"),
            "印の付き方が期待と違う"
        );
    }

    /// **改行を含む応答でも、イベントは1行のまま**（`server_plain.log` を行で数える道具がある）。
    ///
    /// 全軸採点の応答は **N 行**返る（ADR-0027 §12）ので、そのまま流すと
    /// **1回の判定が複数行のログになり、件数を数える道具が壊れる。**
    #[test]
    fn a_multi_line_response_stays_on_one_event_line() {
        let reply = format!("- {MOVED_AXIS} +1\n- {TWIN_AXIS} 0\n- {STILL_AXIS} 0");
        assert!(reply.contains('\n'), "前提: 改行を含む応答であること");

        let ran = run_once_in(
            &MockSub::replying(&reply),
            USER_MESSAGE,
            StatDriftMode::On,
            DriftForm::All,
        );

        let events: Vec<&str> = ran
            .output
            .lines()
            .filter(|line| line.contains(RAN_MESSAGE))
            .collect();
        assert_eq!(
            events.len(),
            1,
            "判定のイベントが1行になっていない: {:?}",
            ran.output
        );

        // **中身は全部その1行に載っている**（改行はエスケープされる）。
        let value = raw_field(events[0]);
        assert!(!value.contains('\n'), "生の改行がログ本文に出た: {value:?}");
        for line in reply.lines() {
            assert!(
                value.contains(line),
                "応答の一部が載っていない: {line:?} / {value:?}"
            );
        }
        // **改行はエスケープされた2文字として残っている**（消えていない）。
        assert!(
            value.contains(&format!("{}n", '\u{5C}')),
            "改行がエスケープされた形で残っていない: {value:?}"
        );
    }

    /// **変容の欄名が、他のモジュールのどこにも綴られていない。**
    ///
    /// # なぜ要るのか（16b のレビューで実測 / `scratchpad/m12`）
    ///
    /// 接頭辞が無かったころ、素朴な grep が別のイベントを一緒に数えた ——
    /// `outcome=` は `server::turn` の分解ログ（`narration_only` / `structured`）を、
    /// `axes=` は `server::admin` の1件を拾い、
    /// `skipped=` / `prompt_tokens=` / `limit_tokens=` は
    /// **`server::extract` の同名の欄とそのまま重なっていた。**
    ///
    /// ADR-0024 は危険3つを「**ログの数だけで判別できる形**」にすると約束しており、
    /// **この測定は作業単位 17 でも繰り返す。**
    ///
    /// # この走査が守れる範囲
    ///
    /// **`src/**` のうち `server::drift` 以外に、その綴りが1文字でも現れたら落ちる。**
    /// コメントでも `use` でも同じ。
    ///
    /// **守れないもの:** ログの**値**の重なり（`applied` という語は他所にも出うる）。
    /// 数えるときは `drift_outcome=applied` のように**欄名ごと**照合すること。
    ///
    /// # ⚠ **この走査は、他のモジュールの doc にも当たる**（着手直後に自分で踏んだ）
    ///
    /// **引用しただけで赤くなる。** 実際、`store::drift` の doc に
    /// 「ログの `drift_moved=` が反転する」と書いた時点でここが落ちた
    /// （CLAUDE.md「走査系のテストが増えると、互いのフィクスチャで誤爆する」）。
    ///
    /// **緩めない。** 他所から言及したいときは、
    /// **接頭辞つきの綴りをそのまま書かず**「`record_run` が出す欄」のように指すこと
    /// （`store::memory::table_word` が語を分けて書くのと同じ作法）。
    #[test]
    fn the_drift_fields_never_collide_with_another_module() {
        let sources = other_modules();
        for field in DRIFT_FIELDS {
            let (scanned, found) = files_spelling(&sources, field);
            assert!(
                scanned >= 20,
                "走査対象が少なすぎるなら、この assert は何も守っていない: {scanned} 件"
            );
            assert!(
                found.is_empty(),
                "{field} が他のモジュールにも綴られています（grep が両方を数えます）:\n{}",
                found.join("\n")
            );
        }
    }

    /// **走査本体が現に綴りを見つけられる**ことの対照。
    ///
    /// 「1件も見つからないから緑」と「そもそも探せていないから緑」を区別する。
    /// **対照は植えたソースを読む**（実ファイルを読むと自己成就する / 15b の実測）。
    ///
    /// ここで塞いでいる2つ（**3つ目は塞げないので、別の見張り番に預けてある**）:
    ///
    /// 1. **植えた綴りを、見張り番と同じ本体が見つけるか。**
    /// 2. **無関係な行を見つけないか**（無条件 true への変異）。
    ///
    /// # **「綴りが実在するか」はここでは確かめられない**（実測 / 2026-08-23）
    ///
    /// 下の 1. は「除外したファイル自身が全部の綴りを持つこと」を見るが、
    /// **これは自己成就する** —— `DRIFT_FIELDS` の定数そのものがこのファイルにあるので、
    /// 綴りを `drift_outcomes` のような**実在しない欄名へ変える変異**を当てても、
    /// **その綴りは定数として書かれている**から `contains` は真になる。
    /// 15b で `prompt::relevance` が踏んだ形とまったく同じ。
    ///
    /// **本物の対照は [`every_drift_field_actually_appears_in_a_log_line`]** ——
    /// あちらは**実際に出たログの本文**を見るので、ソースに綴りがあるだけでは緑にならない。
    /// 実測: 綴りを壊す変異で赤くなったのは**あちら1本だけ**だった。
    /// 下の 1. は「定数と実装のファイルが揃っていること」を言うだけの弱い検査として残す。
    #[test]
    fn the_field_scan_would_notice_a_collision() {
        // 1. 除外したファイル自身は、全部の綴りを持っている。
        let mine = std::fs::read_to_string(this_module()).expect("このファイルを読めること");
        for field in DRIFT_FIELDS {
            assert!(
                mine.contains(field),
                "その綴りの欄が実在しない（走査の綴りが古い）: {field}"
            );
        }

        // 2 と 3. 植えたソースを、見張り番と同じ本体に通す。
        for field in DRIFT_FIELDS {
            let planted = vec![
                ("planted-collision.rs".to_owned(), format!("{field} = 1,")),
                (
                    "planted-clean.rs".to_owned(),
                    "session_id = job.session_id.get(),".to_owned(),
                ),
            ];
            let (scanned, found) = files_spelling(&planted, field);
            assert_eq!(scanned, 2, "植えた分を全部見ること");
            assert_eq!(
                found.len(),
                1,
                "{field}: 植えた衝突をちょうど1件見つけること: {found:?}"
            );
            assert!(found[0].ends_with("planted-collision.rs"), "{found:?}");
        }
    }

    /// このファイル自身のパス。
    fn this_module() -> std::path::PathBuf {
        let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for part in ["src", "server", "drift.rs"] {
            path.push(part);
        }
        path
    }

    /// `src/**` の Rust ソースから、**このファイルを除いた**もの。
    fn other_modules() -> Vec<(String, String)> {
        let mine = this_module();
        let mut files = Vec::new();
        let mut stack = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("走査対象を読めること") {
                let path = entry.expect("ディレクトリ項目を読めること").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !path.extension().is_some_and(|ext| ext == "rs") || path == mine {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("ソースを読めること");
                files.push((path.display().to_string(), source));
            }
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }
    /// **走査対象が実在する。** パスの綴りが古びたら落ちる。
    #[test]
    fn every_scanned_path_exists() {
        for path in scanned_paths() {
            assert!(
                path.is_file(),
                "走査対象が実在しないこと: {}",
                path.display()
            );
        }
    }

    /// 傍観者の記憶を1件入れておく（他のテストで使う土台の確認）。
    ///
    /// **持ち主を決め打ちにする実装**は、傍観者の軸が動くことで捕まる。
    /// ここでは前提（2体が別々の性格ステータスを持つこと）だけを固定する。
    #[test]
    fn the_fixture_gives_the_two_characters_different_stats() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("fixture.sqlite")));

        let mine = runtime.block_on(stats(&fixture.store, fixture.character.id));
        let theirs = runtime.block_on(stats(&fixture.store, fixture.bystander.id));
        runtime.block_on(fixture.store.close());

        assert_eq!(mine, untouched());
        assert_eq!(theirs, bystander_untouched());
        assert_ne!(mine, theirs, "2体の軸の並びが別であること");
    }

    /// 傍観者に記憶を1件入れられること（`NewMemory` の経路が生きていることの前提）。
    #[test]
    fn the_bystander_can_hold_a_memory_without_affecting_the_drift() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("bystander.sqlite")));

        runtime
            .block_on(
                fixture.store.add_memory(
                    fixture.bystander.id,
                    &NewMemory {
                        subject: Subject::Character,
                        content: MemoryContent::parse("港の見える坂道に住んでいる")
                            .expect("正当な本文であること"),
                        category: None,
                    },
                ),
            )
            .expect("傍観者の記憶を入れられること");

        let theirs = runtime.block_on(stats(&fixture.store, fixture.bystander.id));
        runtime.block_on(fixture.store.close());
        assert_eq!(theirs, bystander_untouched(), "記憶は軸に影響しないこと");
    }

    // ---------------------------------------------------------------------
    // 3値の走らせ方と票（ADR-0026 §決定1 / §決定3）
    // ---------------------------------------------------------------------

    /// **記録専用モードでは `stat_alloc` が1バイトも変わらない**（ADR-0026 §決定1）。
    ///
    /// # このモードの存在理由がこれである
    ///
    /// 「適用しないのでプロンプトが動かず、決定性が保たれる」——
    /// ここが緩むと、**同じ会話を2回投げて比べる測定が黙って成立しなくなる。**
    /// しかも**票は同じように溜まる**ので、集計だけ見ても気付けない。
    ///
    /// # 上流は呼ぶ（off とは違う）
    ///
    /// 記録専用は「走らせるが動かさない」。上流を呼ばないと票が立たないので、
    /// **2回目の呼び出しが行われることも一緒に見る。**
    #[test]
    fn the_shadow_mode_never_touches_the_stored_values() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let ran = run_once_with(&sub, USER_MESSAGE, StatDriftMode::Shadow);

        assert_eq!(sub.call_count(), 2, "抽出と変容で2回呼ぶこと");
        assert_eq!(
            ran.stats,
            untouched(),
            "値が1つも動かないこと: {:?}",
            ran.stats
        );
        assert_eq!(
            ran.bystander_stats,
            bystander_untouched(),
            "傍観者の値も動かないこと"
        );
        assert!(
            has_field(drift_record(&ran.output), "drift_moved=-"),
            "書き込みまで行っていない印を出すこと: {:?}",
            ran.output
        );
    }

    /// **記録専用モードでも票は残る**（ADR-0026 §決定3）。
    ///
    /// # 帰属を3つとも見る
    ///
    /// 持ち主（傍観者の側が空であること）・往復（`session_id`）・向き（提案どおり）。
    /// **「票が1件ある」だけでは、どれのものかを何も言っていない**
    /// （CLAUDE.md「抜けるのはいつも帰属」）。
    #[test]
    fn the_shadow_mode_records_a_vote_for_this_character_and_this_turn() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let ran = run_once_with(&sub, USER_MESSAGE, StatDriftMode::Shadow);

        assert_eq!(
            ran.votes,
            vec![(
                ran.session_id,
                MOVED_AXIS.to_owned(),
                DriftDelta::Up.label()
            )],
            "この往復の、この軸の、上げの票が1件だけ残ること"
        );
        assert!(
            ran.bystander_votes.is_empty(),
            "会話していないキャラクターに票を立てないこと: {:?}",
            ran.bystander_votes
        );
        assert!(
            has_field(drift_record(&ran.output), "drift_recorded=recorded"),
            "票を残したことが記録に出ること: {:?}",
            ran.output
        );
    }

    /// **下げの提案は下げの票になる。**
    ///
    /// 上げだけで測ると、**向きを決め打ちにする実装が緑を通る**
    /// （`DriftDelta::Up` を書き込む変異）。
    #[test]
    fn a_downward_suggestion_is_recorded_as_a_downward_vote() {
        let sub = MockSub::replying(&format!("- {STILL_AXIS} -1"));
        let ran = run_once_with(&sub, USER_MESSAGE, StatDriftMode::Shadow);

        assert_eq!(
            ran.votes,
            vec![(
                ran.session_id,
                STILL_AXIS.to_owned(),
                DriftDelta::Down.label()
            )],
            "下げの票が、提案された軸に立つこと"
        );
    }

    /// **on では票の記録と適用の両方が起きる**（ADR-0026 §決定3）。
    ///
    /// 記録専用との差は**値が動くこと1点だけ**で、票の側は同じでなければならない
    /// ——違うと、記録専用で集めた提案列に規則を当て直しても
    /// **`on` の実際とは別物**になる。
    #[test]
    fn the_on_mode_records_a_vote_and_moves_the_value() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let ran = run_once_with(&sub, USER_MESSAGE, StatDriftMode::On);

        assert_eq!(
            ran.votes,
            vec![(
                ran.session_id,
                MOVED_AXIS.to_owned(),
                DriftDelta::Up.label()
            )],
            "票は記録専用のときと同じ形で残ること"
        );
        assert_eq!(
            ran.stats,
            vec![
                (MOVED_AXIS.to_owned(), MOVED_START + 1),
                (TWIN_AXIS.to_owned(), MOVED_START),
                (STILL_AXIS.to_owned(), STILL_START),
                (SILENT_AXIS.to_owned(), 0),
            ],
            "提案された軸だけが1段上がること"
        );
        assert!(
            has_field(
                drift_record(&ran.output),
                &format!("drift_moved={}", DriftWrite::Moved.label())
            ),
            "書き込みが届いたことも記録に出ること: {:?}",
            ran.output
        );
    }

    /// **3値それぞれが、記録から見分けられる**（16d の指示）。
    ///
    /// 後から測定を読むときに「適用があったのか無かったのか」が分からないと、
    /// **記録専用で集めた列と `on` で集めた列が混ざる。**
    ///
    /// `off` の行は出ない（判定そのものを走らせない）ので、
    /// **「その綴りが出ないこと」**を見る。
    #[test]
    fn the_run_mode_is_readable_from_the_log() {
        let reply = format!("- {MOVED_AXIS} +1");
        let shadow = run_once_with(
            &MockSub::replying(&reply),
            USER_MESSAGE,
            StatDriftMode::Shadow,
        );
        let on = run_once_with(&MockSub::replying(&reply), USER_MESSAGE, StatDriftMode::On);
        let off = run_once_with(&MockSub::replying(&reply), USER_MESSAGE, StatDriftMode::Off);

        assert!(
            has_field(drift_record(&shadow.output), "drift_run_mode=shadow"),
            "記録専用と読めること: {:?}",
            shadow.output
        );
        assert!(
            has_field(drift_record(&on.output), "drift_run_mode=on"),
            "on と読めること: {:?}",
            on.output
        );
        assert!(
            !has_field(&shadow.output, "drift_run_mode=on"),
            "記録専用の行が on を名乗らないこと: {:?}",
            shadow.output
        );
        assert!(
            !has_field(&on.output, "drift_run_mode=shadow"),
            "on の行が記録専用を名乗らないこと: {:?}",
            on.output
        );
        // **言いたいのは「そのキーが、値によらず一切現れない」。**
        // **何で書くかは `crate::testlog` の `has_field` の doc の表が権威**（2行目）。
        // **素の `contains("キー=")` は、実装が正しいのに落ちる形を持つ**——
        // **作業単位 B-31 で `x_drift_run_mode=` の囮を1本足して実測した**
        // （是正前は 2 赤・うち1本がここ / 是正後はここが緑）。
        assert!(
            !has_key(&off.output, "drift_run_mode"),
            "off では判定そのものが走らないこと: {:?}",
            off.output
        );
    }

    /// **記録に、走らせ方の出どころが載る**（ADR-0045 §結果の ⚠ / 作業単位 A-8）。
    ///
    /// 既存のキャラクターに値を書いたまま測定を回すと、環境変数が効かない。
    /// **それをログだけで追えるように**、判定した回にも飛ばした回にも出どころを載せる。
    ///
    /// # 腕
    ///
    /// | 既定 | キャラクター | 期待 |
    /// |---|---|---|
    /// | `on` | 未設定 | 判定の行が `on` / `default` |
    /// | `off` | `shadow` | 判定の行が `shadow` / `character`（**既定と違う値で回る腕**） |
    /// | `on` | `off` | **行が1本も出ない**（判定そのものを走らせない） |
    /// | `off` | `on` | 飛ばした行が `on` / `character` |
    ///
    /// **同じ行の中で見る**（`drift_record` / `skip_record`）。
    #[test]
    fn the_drift_record_names_where_its_run_mode_came_from() {
        let reply = format!("- {MOVED_AXIS} +1");

        let by_default = run_once_as(
            &MockSub::replying(&reply),
            USER_MESSAGE,
            StatDriftMode::On,
            None,
        );
        let line = drift_record(&by_default.output);
        assert!(
            has_field(line, "drift_run_mode=on") && has_field(line, "drift_mode_source=default"),
            "既定から来た回: {line:?}"
        );

        let by_character = run_once_as(
            &MockSub::replying(&reply),
            USER_MESSAGE,
            StatDriftMode::Off,
            Some(StatDriftMode::Shadow),
        );
        let line = drift_record(&by_character.output);
        assert!(
            has_field(line, "drift_run_mode=shadow")
                && has_field(line, "drift_mode_source=character"),
            "キャラクターから来た回: {line:?}"
        );

        let silenced = run_once_as(
            &MockSub::replying(&reply),
            USER_MESSAGE,
            StatDriftMode::On,
            Some(StatDriftMode::Off),
        );
        assert!(
            !has_key(&silenced.output, "drift_run_mode")
                && !has_key(&silenced.output, "drift_mode_source"),
            "キャラクターが off なら判定そのものが走らないこと: {:?}",
            silenced.output
        );
        assert!(silenced.votes.is_empty(), "票も残らないこと");

        let skipped = run_once_as(
            &MockSub::replying("なし"),
            &"あ".repeat(FALLBACK_MAX_DRIFT_PROMPT_TOKENS as usize),
            StatDriftMode::Off,
            Some(StatDriftMode::On),
        );
        let line = skip_record(&skipped.output);
        assert!(
            has_field(line, "drift_run_mode=on") && has_field(line, "drift_mode_source=character"),
            "飛ばした回にも出どころが載ること: {line:?}"
        );
    }

    /// **走らせなかった回にも走らせ方が載る。**
    ///
    /// 提案率は「走った回 ÷ 走らせようとした回」で読むので、
    /// **飛んだ回がどのモードのものか分からないと分母が作れない。**
    #[test]
    fn a_skipped_turn_also_says_which_mode_it_was_running_in() {
        let ran = run_once_with(
            &MockSub::replying("なし"),
            &"あ".repeat(FALLBACK_MAX_DRIFT_PROMPT_TOKENS as usize),
            StatDriftMode::Shadow,
        );

        assert!(
            ran.output.contains(SKIPPED_MESSAGE),
            "飛んだ回として記録すること: {:?}",
            ran.output
        );
        assert!(
            has_field(skip_record(&ran.output), "drift_run_mode=shadow"),
            "飛んだ回にも走らせ方が載ること: {:?}",
            ran.output
        );
    }

    /// **飽和した提案も票になる**（ADR-0026 §決定3）。
    ///
    /// # 上端にいる軸を実際に作る
    ///
    /// フィクスチャの軸はどれも端にいないので、
    /// **この状態はテストが作らないと存在しない**
    /// （CLAUDE.md「フィクスチャがその分岐が効く状態に一度も入っていない」）。
    /// ここでは尺度の上端まで押し上げてから `+1` を提案させる。
    ///
    /// # 実害
    ///
    /// 記録の時点で落とすと、「上端で上げようとし続けている」が票から消える。
    /// **蓄積（16e）はその傾向を二度と見られない。**
    #[test]
    fn a_saturated_suggestion_is_still_recorded_as_a_vote() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("drift.sqlite")));

        // **上端に貼り付いた軸を作る**（この分岐に入る唯一の状態）。
        let top = fixture.character.stat_scale.get();
        runtime.block_on(put_stats(
            &fixture.store,
            fixture.character.id,
            &[(MOVED_AXIS, top), (STILL_AXIS, STILL_START)],
        ));

        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::Shadow,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                extractor.wait_for_the_running_task().await;
            });
        });

        assert!(
            has_field(drift_record(&output), "drift_outcome=saturated"),
            "前提: 飽和として記録される回であること: {output:?}"
        );

        let recorded = runtime.block_on(votes(&fixture.store, fixture.character.id));
        assert_eq!(
            recorded,
            vec![(
                fixture.session_id.get(),
                MOVED_AXIS.to_owned(),
                DriftDelta::Up.label()
            )],
            "飽和しても、提案どおりの票が残ること"
        );
        assert_eq!(
            runtime.block_on(stats(&fixture.store, fixture.character.id)),
            vec![
                (MOVED_AXIS.to_owned(), top),
                (STILL_AXIS.to_owned(), STILL_START),
            ],
            "値は動かないこと（クランプもしない）"
        );

        runtime.block_on(fixture.store.close());
    }

    /// **破棄された提案は票にならない**（ADR-0026 §決定3）。**1軸を尋ねる形で出る3種すべて。**
    ///
    /// 破棄は「許可リストの門を通っていない」——
    /// 票に入れると、**モデルが作った軸名が蓄積へ混ざる。**
    ///
    /// | 上流の答え | 理由 |
    /// |---|---|
    /// | 形にならない行 | `malformed` |
    /// | 2行 | `multiple_lines` |
    /// | 保存されていない軸 | `unknown_axis` |
    ///
    /// **`missing_axis` / `duplicate_axis` は全軸採点でしか出ない**（ADR-0027 §12）ので、
    /// ここには来ない——分解の側の見張り番は `parse::drift`。
    #[test]
    fn every_discarded_suggestion_stays_out_of_the_votes() {
        let cases = [
            ("これは形式に合わない文章です", "malformed"),
            (
                &format!("- {MOVED_AXIS} +1\n- {STILL_AXIS} -1"),
                "multiple_lines",
            ),
            (&format!("- {UNKNOWN_AXIS} +1"), "unknown_axis"),
        ];

        for (reply, expected) in cases {
            let ran = run_once_with(
                &MockSub::replying(reply),
                USER_MESSAGE,
                StatDriftMode::Shadow,
            );

            assert!(
                has_field(
                    drift_record(&ran.output),
                    &format!("drift_outcome={expected}")
                ),
                "前提: {expected} として捨てられる回であること: {:?}",
                ran.output
            );
            assert!(
                ran.votes.is_empty(),
                "{expected} は票にならないこと: {:?}",
                ran.votes
            );
            assert!(
                has_field(drift_record(&ran.output), "drift_recorded=-"),
                "票にならなかった印を出すこと: {:?}",
                ran.output
            );
        }
    }

    /// **「提案なし」も票にならない。**
    ///
    /// 破棄とは別の経路（分解が正常に「なし」と読んだ回）なので、別に見る。
    #[test]
    fn a_turn_with_no_suggestion_records_no_vote() {
        let ran = run_once_with(
            &MockSub::replying("なし"),
            USER_MESSAGE,
            StatDriftMode::Shadow,
        );

        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=nothing"),
            "前提: 提案なしとして記録される回であること: {:?}",
            ran.output
        );
        assert!(ran.votes.is_empty(), "票が立たないこと: {:?}", ran.votes);
    }

    /// **票の3つの印が、1つも重ならない。**
    ///
    /// 「票にならなかった」「残した」「残せなかった（故障）」は全部違う話である。
    ///
    /// # ⚠ **「書き込みの印とも重ねない」とは書けない**（16d のレビューで指摘）
    ///
    /// **[`NO_VOTE`] と [`NO_WRITE`] はどちらも `"-"` で、実際に重なっている。**
    /// 当初この doc は「書き込みの印とも重ねない」と書いていたが**誤り**だった。
    ///
    /// **重なったままにする。** 「該当なし」の番兵は
    /// [`NO_AXIS`] / [`NO_DELTA`] / [`NO_WRITE`] と揃った1つの綴りである方が読みやすく、
    /// **`tracing` は `キー=値` で出すので欄名で区別が付く**（実害が無い）。
    ///
    /// **重ねてはいけないのは故障の側だけ**——`failed` と `failed_to_record` が
    /// 同じ綴りになると、**票が書けなかった回と値が書けなかった回が
    /// 1つの数に潰れる。** そこは下で押さえる。
    #[test]
    fn every_vote_marker_is_distinct() {
        let markers = [NO_VOTE, VOTE_RECORDED, VOTE_FAILED];
        for (index, left) in markers.iter().enumerate() {
            for right in &markers[index + 1..] {
                assert_ne!(left, right, "票の印が重なっています: {markers:?}");
            }
        }
        // **書き込みの「失敗」と票の「失敗」を同じ綴りにしない。**
        assert_ne!(
            VOTE_FAILED, WRITE_FAILED,
            "票の故障と書き込みの故障が同じ綴りになっている"
        );
        // **「該当なし」は揃えてある**（上の doc）。変えるなら doc も一緒に直すこと。
        assert_eq!(
            NO_VOTE, NO_WRITE,
            "「該当なし」の番兵を揃える判断を変えたなら、この doc も直すこと"
        );
    }

    // ---------------------------------------------------------------------
    // 集約規則の配線（ADR-0027 §決定1〜3・§決定7・§9 / 作業単位 16e-b）
    // ---------------------------------------------------------------------

    /// **累積は次のターンへ持ち越される**（ADR-0027 §決定7「溜めて消費する」）。
    ///
    /// # 1ターンでは観測できない
    ///
    /// 毎ターン累積を `0` から始める実装は、**1ターン目の結果が同じ**なので
    /// このファイルの他のテストを全部通り抜ける
    /// （CLAUDE.md「フィクスチャがその分岐が効く状態に一度も入らない」）。
    ///
    /// `K = 0.1`（尺度 7 なので **費用 ≒ 0.1429**）にすると
    /// **1票目（重み 0.1）では届かず、2票目（重み 0.1/√2、累計 ≒ 0.1707）で越える。**
    /// 持ち越しが消えていれば、**何ターン回しても動かない。**
    #[test]
    fn the_accumulator_carries_over_to_the_next_turn() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let tuning = DriftTuning::new(1.0, 0.0, 0.1).expect("つまみが範囲内であること");

        let after_one = run_turns_tuned(
            &sub,
            &[USER_MESSAGE],
            StatDriftMode::On,
            DriftForm::Single,
            tuning,
        );
        assert_eq!(
            after_one.stats,
            untouched(),
            "前提: 1票では費用に届かないこと"
        );

        let after_two = run_turns_tuned(
            &sub,
            &[USER_MESSAGE, USER_MESSAGE],
            StatDriftMode::On,
            DriftForm::Single,
            tuning,
        );
        assert_eq!(
            after_two.stats.first(),
            Some(&(MOVED_AXIS.to_owned(), MOVED_START + 1)),
            "2ターン目で持ち越しが効いていない: {:?}",
            after_two.stats
        );
        assert_eq!(after_two.votes.len(), 2, "票は2件残ること");
    }

    /// **票の件数がそのまま重みになる**（ADR-0027 §決定1 / §12 の危険3）。
    ///
    /// # `0` を数に入れる変異が、ここで赤くなる
    ///
    /// 2票目の重みは `0.1/√2 ≒ 0.0707`。`K = 0.1`（尺度 7 なので費用 ≒ 0.1429）にすると
    /// **1票目（0.1）＋2票目（0.0707）＝ 0.1707 で越える。**
    /// もし重みを数え間違えて2票目も `0.1` なら累計 `0.2` になり、
    /// 逆に票を数え落として初回の重みを毎回使っても同じである。
    /// **残った累積**が、重みが `0.1/√2` だったことの証拠になる
    /// ——段の数は両方とも 1 なので、**そこだけ見ても区別が付かない。**
    #[test]
    fn the_weight_of_the_second_vote_comes_from_the_recorded_votes() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let tuning = DriftTuning::new(1.0, 0.0, 0.1).expect("つまみが範囲内であること");
        let ran = run_turns_tuned(
            &sub,
            &[USER_MESSAGE, USER_MESSAGE],
            StatDriftMode::On,
            DriftForm::Single,
            tuning,
        );

        assert_eq!(
            ran.stats.first(),
            Some(&(MOVED_AXIS.to_owned(), MOVED_START + 1)),
            "2ターンで1段だけ動くこと: {:?}",
            ran.stats
        );
        // 0.1 + 0.1/√2 − 10/7×0.1 ≒ 0.02785 が残る。
        // **2票目も 0.1 だと 0.2 − 0.14286 ≒ 0.05714** になり、ここで割れる。
        let charge = charge_of(&ran, MOVED_AXIS).expect("累積の行があること");
        assert!(
            (charge - 0.027_853_5).abs() < 1e-6,
            "2票目の重みが 0.1/√2 になっていない（残った累積 {charge}）: {:?}",
            ran.charges
        );
    }

    /// **`K` だけを振った対比のつまみ**（[`the_default_pricing_never_lifts_an_axis_nobody_mentioned`]）。
    ///
    /// # `0.5` / `1.5` を直書きし、**既定と一致していることをここで確かめる**
    ///
    /// この2つが `DEFAULT_DRIFT_DECAY` / `DEFAULT_DRIFT_SLOPE` と一致しているからこそ、
    /// **`0.33` / `0.14` の対比が「`K` だけを振った A/B」になっている。**
    ///
    /// **既定から引いて組み立てない**（`DriftTuning::default()` の λ と傾きを使う形にしない）。
    /// それだと**既定が動いたときに対比も黙って一緒に動き**、
    /// **窓の縁（`0.3174566` / `0.1447229`）を測り直す機会が失われる。**
    /// 直書き＋一致の検査にしてあるので、**既定が動いた瞬間にここが赤になる**
    /// ——**失敗の向きは「赤くなる」側**なので、見落とし（偽陰性）にはならない。
    fn contrast_tuning(step_cost: f64) -> DriftTuning {
        let base = DriftTuning::default();
        assert!(
            (base.decay() - 0.5).abs() < 1e-12 && (base.slope() - 1.5).abs() < 1e-12,
            "対比の 0.5 / 1.5 が既定と一致しなくなった。\
             `0.33` / `0.14` の窓（doc の表）を測り直すこと: λ={} 傾き={}",
            base.decay(),
            base.slope()
        );
        DriftTuning::new(0.5, 1.5, step_cost).expect("つまみが範囲内であること")
    }

    /// **既定のつまみでは、言われなかった軸は上がらない**
    /// （ADR-0027「訂正（2026-08-24）」/ 作業単位 16e-b）。
    ///
    /// # ⚠ この見張り番は、以前は**逆のこと**を言っていた
    ///
    /// `the_default_pricing_lifts_a_zero_valued_axis_on_its_own` を置き換えたもの
    /// （司令塔の採否済み）。**傾き `2` は `v = 0` の上げの費用を厳密に `0` にする**ので、
    /// **値 0 の軸が会話で何も言われなくても毎ターン 1 に上がっていた。**
    /// 既定を `1.5` に下げて（`費用(0, 上げ) = K × 0.25`）塞いだ。
    ///
    /// **実害の向きは軽くない**——運用者が「語らない」と宣言した軸が、
    /// 変容が走っている限り本文プロンプトに現れていた。
    ///
    /// # ここは**既定のつまみを通る唯一の配線テスト**である
    ///
    /// 他は [`fixture_tuning`]（値付けを外した組）で回っている
    /// （CLAUDE.md「テスト専用の入口を作ると、本番の入口が観測対象から外れる」）。
    ///
    /// # 「何も動かない」だけを見ない
    ///
    /// 全部止まっている実装でも緑になってしまう。
    /// **言われた軸が動き、言われなかった軸は動かない**ことを一緒に見る。
    ///
    /// # ⚠ **10 往復回す**（作業単位 16g → **19 で 75 から下げた**）
    ///
    /// 既定のつまみでの `費用(4, 上げ)` は、尺度 7 では
    /// `(0.3 × 10/7) × (1 + 1.5 × 0.5/7) ≒ 0.4745`（ADR-0027 訂正 2 で尺度の正規化が入った）。
    /// **票の重みは `0.1/√n`**（訂正 3）なので、累積がそこへ届くのは **10 票目**
    /// ——`0.1 × Σ1/√k` は 9 票で `0.4705`（**まだ届かない**）、10 票で `0.5021`。
    ///
    /// **`K` が `1.0` から `0.3` へ下がった**（16j / 作業単位 19）ので、
    /// **1段に要る票が 75 → 10 になった。**
    ///
    /// # ⚠ **この往復数は「ちょうど1段」の窓の下端に置いてある**
    ///
    /// **9 票では 0 段、35 票目で 2 段目に入る**（実測）。
    /// **10 はその窓の下端**なので、**両方向の決め打ちがここで赤になる**
    /// （CLAUDE.md「対比は『安い側』だけでは足りない」）。
    ///
    /// | `K` の決め打ち | ここでの結果 |
    /// |---|---|
    /// | **`0.3174566` より高い側** | 10 票では費用に届かず、**「言われた軸が動かない」で赤** |
    /// | 既定（`0.3`） | **ちょうど 1 段** |
    /// | **`0.1447229` 以下** | 10 票で 2 段動き、**赤** |
    ///
    /// > **⚠ 窓は片側だけ狭い。** **上端は `0.3174566`**
    /// > （＝ `0.5020998 / 1.5816327`。既定はそこから **5.5% 下**）で、
    /// > **下端は `0.1447229`**（＝ `0.5020998 / (1.5816327 + 1.8877551)`。
    /// > 既定はそこから **107% 上**）。**安い側は `0.2` でもこの1本では緑を通る。**
    /// >
    /// > **⚠ ここに一度 `0.3199` と書いてあったが、実測とずれている**
    /// > （19 の測定値の写し。**`K = 0.317` はまだ 1 段動く**）。
    /// > **上端は `0.3175` 付近**なので、**余裕を 3.1% と読まないこと（実際は 3.9%）。**
    ///
    /// **往復数を窓の真ん中へ動かさないこと。** 20 に置くと、
    /// **高い側の決め打ちまで緑を通るようになる。**
    ///
    /// # ⚠ **段数だけを見ると、この見張り番は `K` の値を見ていない**（作業単位 20a）
    ///
    /// 上の窓は `0.1447229 < K ≤ 0.3174566` と広いので、**既定を `±10%` 動かす変異**
    /// （`0.3 → 0.27`）**は段数では捕まらない。** 16g のレビューの指摘どおり、
    /// **往復数そのものが遊びを吸収していた**（実測: `0.27` で赤になったのは
    /// 値を直書きした3本だけ——`the_default_tuning_is_the_one_the_adr_settled_on` /
    /// `the_cheapest_step_still_costs_a_tenth_of_the_base` /
    /// `the_drift_tuning_defaults_to_the_values_the_adr_settled_on`。
    /// **振る舞いを見る側は1本も赤くならなかった**）。
    ///
    /// **往復数を増やして窓を狭める道は採らない。** 脆くなるだけで、
    /// **`K` を見たことにはならない**（司令塔の指示）。**代わりに残った累積を見る。**
    ///
    /// ## **残った累積は `K` の連続な関数である**
    ///
    /// 10 往復ぶんの票（`0.1 × Σ_{k=1..10} 1/√k = 0.5020998`）から
    /// `費用(4, 上げ) = K × (10/7) × (1 + 1.5 × 0.5/7) = K × 1.5816327` を1回だけ引いた分が残る。
    ///
    /// | `K` | 段数 | 残った累積 |
    /// |---|---|---|
    /// | `0.27`（既定 −10%） | 1 | `0.0750590` |
    /// | **既定（`0.3`）** | **1** | **`0.0276100`** |
    /// | `0.33`（既定 +10%） | 0 | `0.5020998` |
    ///
    /// **段数は3つのうち2つで同じでも、累積は必ず違う。**
    /// 許容 `1e-6` で押さえるので、**`K` が `1e-6` 動けばここが赤になる**
    /// （赤の下端は `1e-6 / 1.5816327 ≒ 6.3e-7`）。
    /// 期待値は `DriftTuning::default()` からではなく**上の算術から直書き**する
    /// （CLAUDE.md「関数の外にある値で押さえる」）。
    ///
    /// ## それでも「つまみが届いているか」は別に要る
    ///
    /// 累積だけでは、**`K` を無視して `0.3` を決め打つ実装**が緑を通る。
    /// **高い側（`0.33` で動かない）と安い側（`0.14` で2段動く）を同じテストに置く**
    /// ——CLAUDE.md「対比は『安い側』だけでは足りない」。
    /// **どちらも絶対値で直書き**してあるので、既定を動かしても道連れにならない。
    ///
    /// ### ⚠ **対比は窓の縁から 4% 弱しか離れていない**
    ///
    /// `0.33` は上端（`0.3174566`）の **+3.95%**、`0.14` は下端（`0.1447229`）の **−3.26%**。
    /// **票の重みの係数（`0.1`）・往復数（`10`）・値付け曲線を触ると、
    /// この2つが窓の内側へ入って対比が黙って潰れる**
    /// （CLAUDE.md「速さのつまみを触ると、既定を通る見張り番が黙って死ぬ」）。
    /// **触る作業単位では、上の2つの数を必ず測り直すこと。**
    ///
    /// ### ⚠ **対比は「`K` だけを振ったもの」でなければならない**
    ///
    /// `DriftTuning::new(0.5, 1.5, …)` の `0.5` / `1.5` は
    /// **`DEFAULT_DRIFT_DECAY` / `DEFAULT_DRIFT_SLOPE` と一致している**——
    /// 一致していないと、これは `K` の A/B ではなく**3つを同時に振った比較**になる。
    /// 既定の傾きを動かせば残累積のアサートが先に赤くなるが、
    /// **それを直したあとも対比だけが古い傾きで回り続ける**ので、
    /// **前提そのものをテストの中で確かめる**（[`contrast_tuning`]）。
    #[test]
    fn the_default_pricing_never_lifts_an_axis_nobody_mentioned() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        // **既定のつまみで1段動くまでの往復数**（doc の ⚠）。
        let turns = vec![USER_MESSAGE; 10];
        let ran = run_turns_tuned(
            &sub,
            &turns,
            StatDriftMode::On,
            DriftForm::Single,
            DriftTuning::default(),
        );

        let value_of = |axis: &str| {
            ran.stats
                .iter()
                .find(|(name, _)| name == axis)
                .map(|(_, value)| *value)
                .unwrap_or_else(|| panic!("{axis} があること: {:?}", ran.stats))
        };

        // **値 0 の軸は、何も言われていないので上がらない。**
        assert_eq!(
            value_of(SILENT_AXIS),
            0,
            "言われていない値 0 の軸が上がった（費用 0 の段が戻っている）: {:?}",
            ran.stats
        );
        // **他の「選ばれなかった軸」も動かない。**
        assert_eq!(value_of(STILL_AXIS), STILL_START, "{:?}", ran.stats);
        assert_eq!(value_of(TWIN_AXIS), MOVED_START, "{:?}", ran.stats);

        // **言われた軸は動く**（全部止まっている実装と区別する）。
        assert_eq!(
            value_of(MOVED_AXIS),
            MOVED_START + 1,
            "既定のつまみで、2票あっても提案された軸が動かない: {:?}",
            ran.stats
        );
        assert!(
            ran.votes.iter().all(|(_, axis, _)| axis == MOVED_AXIS),
            "何も言われていない軸に票が立った: {:?}",
            ran.votes
        );

        // **ここからが `K` の値そのものを見る側**（doc の「残った累積」）。
        // `0.1 × Σ_{k=1..10} 1/√k − 0.3 × 10/7 × (1 + 1.5 × 0.5/7)`
        // = `0.5020998 − 0.4744898`。**`K` が動けば必ずここが動く。**
        let charge = charge_of(&ran, MOVED_AXIS).expect("累積の行があること");
        assert!(
            (charge - 0.027_610_0).abs() < 1e-6,
            "既定の `K` が 0.3 でない（残った累積 {charge}）: {:?}",
            ran.charges
        );

        // **高い側**——同じ 10 往復で1段も動かない。`K` を安い側へ決め打つ実装が赤になる。
        let dearer = run_turns_tuned(
            &sub,
            &turns,
            StatDriftMode::On,
            DriftForm::Single,
            contrast_tuning(0.33),
        );
        assert_eq!(
            dearer
                .stats
                .iter()
                .find(|(name, _)| name == MOVED_AXIS)
                .map(|(_, value)| *value),
            Some(MOVED_START),
            "`K = 0.33` で動いた（つまみが届いていない）: {:?}",
            dearer.stats
        );

        // **安い側**——同じ 10 往復で2段動く。`K` を高い側へ決め打つ実装が赤になる。
        let cheaper = run_turns_tuned(
            &sub,
            &turns,
            StatDriftMode::On,
            DriftForm::Single,
            contrast_tuning(0.14),
        );
        assert_eq!(
            cheaper
                .stats
                .iter()
                .find(|(name, _)| name == MOVED_AXIS)
                .map(|(_, value)| *value),
            Some(MOVED_START + 2),
            "`K = 0.14` で2段動かない（つまみが届いていない）: {:?}",
            cheaper.stats
        );
    }

    /// **記録専用モードは累積にも触らない**（ADR-0026 §決定1）。
    ///
    /// # 値だけを見ても足りない
    ///
    /// 累積を溜めておくと、**`on` に切り替えた瞬間に溜まっていた分が放出される。**
    /// 記録専用モードの存在理由は「値が動かないので決定性が保たれる」ことなので、
    /// **切り替えた直後だけ荒れる**のはその前提を裏から壊す。
    /// **値が動かないことだけを見る既存の見張り番では観測できない。**
    #[test]
    fn the_shadow_mode_leaves_the_accumulator_empty() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let ran = run_turns_tuned(
            &sub,
            &[USER_MESSAGE, USER_MESSAGE, USER_MESSAGE],
            StatDriftMode::Shadow,
            DriftForm::Single,
            fixture_tuning(),
        );

        assert_eq!(ran.stats, untouched(), "記録専用モードで値が動いた");
        assert!(
            ran.charges.is_empty(),
            "記録専用モードで累積が溜まった: {:?}",
            ran.charges
        );
        assert_eq!(ran.votes.len(), 3, "票は残ること: {:?}", ran.votes);
    }

    /// **全軸採点では、`0` でない採点だけが票になる**（ADR-0027 §決定1）。
    ///
    /// # 票の件数がそのまま重みの分母になる
    ///
    /// `0` まで票にすると、**全軸の件数が毎ターン同じになり、重みが一律に戻る**
    /// （§12 の危険3）。**上げと下げの両方**を同じターンに入れて、
    /// 向きが提案どおりに残ることも一緒に見る。
    #[test]
    fn the_all_axis_form_votes_only_for_the_scores_that_were_not_zero() {
        let reply =
            format!("- {MOVED_AXIS} +1\n- {TWIN_AXIS} -1\n- {STILL_AXIS} 0\n- {SILENT_AXIS} 0");
        let sub = MockSub::replying(&reply);
        let ran = run_once_in(&sub, USER_MESSAGE, StatDriftMode::On, DriftForm::All);

        let mut got: Vec<(String, &'static str)> = ran
            .votes
            .iter()
            .map(|(session, axis, delta)| {
                assert_eq!(*session, ran.session_id, "票の往復が違う");
                (axis.clone(), *delta)
            })
            .collect();
        got.sort();
        let mut want = vec![
            (MOVED_AXIS.to_owned(), "up"),
            (TWIN_AXIS.to_owned(), "down"),
        ];
        want.sort();
        assert_eq!(got, want, "0 の軸まで票になっているか、向きが違う");

        assert!(
            ran.bystander_votes.is_empty(),
            "別のキャラクターに票が立った: {:?}",
            ran.bystander_votes
        );
    }

    /// **`0` と採点された軸は薄れる**（ADR-0027 §決定2 / §決定3）。
    ///
    /// # 累積を読まないと観測できない
    ///
    /// 1ターンでは値が動くほど溜まらないので、
    /// **「薄れた」は表の中にしか現れない。** 値も票もログも動かない。
    ///
    /// **値の違う2本**で見る（比例していることの観測）。
    /// **値 0 の軸は薄れない**——ADR §決定3 が「ゼロから育つのを邪魔しない」と
    /// 書いている当のものなので、同じ一巡で確かめる。
    #[test]
    fn the_all_axis_form_fades_the_axes_that_were_scored_zero() {
        let reply =
            format!("- {MOVED_AXIS} +1\n- {TWIN_AXIS} 0\n- {STILL_AXIS} 0\n- {SILENT_AXIS} 0");
        let sub = MockSub::replying(&reply);
        let ran = run_once_in(&sub, USER_MESSAGE, StatDriftMode::On, DriftForm::All);

        let twin = charge_of(&ran, TWIN_AXIS).expect("薄れた軸の累積があること");
        let still = charge_of(&ran, STILL_AXIS).expect("薄れた軸の累積があること");
        let silent = charge_of(&ran, SILENT_AXIS).expect("値 0 の軸の累積があること");

        assert!(twin < 0.0, "0 と採点された軸が薄れていない: {twin}");
        assert!(still < 0.0, "0 と採点された軸が薄れていない: {still}");
        assert!(
            twin < still,
            "薄れる量が現在値に比例していない（{TWIN_AXIS}={MOVED_START} / \
             {STILL_AXIS}={STILL_START}）: {twin} vs {still}"
        );
        assert!(
            silent.abs() < 1e-12,
            "値 0 の軸が薄れた（ゼロから育つ道を塞いでいる）: {silent}"
        );
    }

    /// **引用記号を持ち込んだ採点表も読める。剥がした行数はログに出る**（作業単位 16e-c）。
    ///
    /// # 繋いだのは `parse_drift_all_report` の側である
    ///
    /// `parse_drift_all` は薄い包みで、**剥がした行数が落ちる。**
    /// ADR-0027 が「31% の破棄」を測った指標そのものなので、本番の経路から落とさない。
    /// **`> ` だけでなく `> - ` の形**（腕E で 2 件出た）も混ぜてある。
    #[test]
    fn a_quoted_scorecard_is_read_and_the_quotes_are_counted() {
        let reply =
            format!("> {MOVED_AXIS} +1\n> - {TWIN_AXIS} 0\n- {STILL_AXIS} 0\n> {SILENT_AXIS} 0");
        let sub = MockSub::replying(&reply);
        let ran = run_once_in(&sub, USER_MESSAGE, StatDriftMode::On, DriftForm::All);

        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=scored"),
            "引用記号つきの採点表が捨てられた: {:?}",
            ran.output
        );
        assert!(
            has_field(drift_record(&ran.output), "drift_quotes=3"),
            "剥がした行数が出ていない: {:?}",
            ran.output
        );
        assert_eq!(
            ran.votes,
            vec![(ran.session_id, MOVED_AXIS.to_owned(), "up")],
            "剥がした行から票が立っていない: {:?}",
            ran.votes
        );
    }

    /// **並びが合わない採点表は帳簿へ渡さない**（CLAUDE.md「抜けるのはいつも帰属」）。
    ///
    /// # なぜ純粋関数として直接見るのか
    ///
    /// `parse::drift` は**依頼に載せた順**で採点表を返すので、
    /// **上流の応答をどう崩してもこの枝には入らない。**
    /// それでも門を置いてあるのは、ずれた採点表が帳簿へ届くと
    /// **軸 A の採点が軸 B に入る**（値も件数も整合したまま中身だけ入れ替わる）から。
    ///
    /// **枝が到達不能なまま緑になるのを避けるため、ここで直接通す**
    /// ——`aligned_scores` はこのモジュールの関数である。
    #[test]
    fn a_scorecard_whose_axes_do_not_line_up_is_not_applied() {
        let stats = allocs(
            SCALE,
            &[(MOVED_AXIS, MOVED_START), (STILL_AXIS, STILL_START)],
        );
        let axis = |raw: &str| StatAxis::parse(raw).expect("正当な軸名であること");

        // **並びも本数も合っていれば通る**（対照）。
        let lined_up = [
            DriftScore::new(axis(MOVED_AXIS), Some(DriftDelta::Up)),
            DriftScore::new(axis(STILL_AXIS), None),
        ];
        assert_eq!(
            aligned_scores(&stats, &lined_up),
            Some(vec![Some(DriftDelta::Up), None]),
            "正しい採点表が落ちた"
        );

        // **入れ替わっている。**
        let swapped = [
            DriftScore::new(axis(STILL_AXIS), None),
            DriftScore::new(axis(MOVED_AXIS), Some(DriftDelta::Up)),
        ];
        assert_eq!(
            aligned_scores(&stats, &swapped),
            None,
            "並びが入れ替わった採点表が通った"
        );

        // **本数が違う。**
        let short = [DriftScore::new(axis(MOVED_AXIS), Some(DriftDelta::Up))];
        assert_eq!(aligned_scores(&stats, &short), None, "短い採点表が通った");

        // **知らない軸が混ざっている。**
        let unknown = [
            DriftScore::new(axis(MOVED_AXIS), Some(DriftDelta::Up)),
            DriftScore::new(axis(UNKNOWN_AXIS), None),
        ];
        assert_eq!(
            aligned_scores(&stats, &unknown),
            None,
            "依頼に無い軸の採点表が通った"
        );
    }

    /// **崩れた採点表は帳簿を回さない**（ADR-0027 §12 が名指しする混同）。
    ///
    /// 破棄された回に「全軸が `0` と採点された」ことにすると、
    /// **形式の崩れがそのまま下向きの供給源になる。**
    /// 累積が1行も増えていないことで観測する——値も票も動かない回なので、
    /// **表を読まないと区別が付かない。**
    #[test]
    fn a_discarded_scorecard_never_fades_anything() {
        // 1行足りない（`missing_axis`）。
        let reply = format!("- {MOVED_AXIS} +1\n- {TWIN_AXIS} 0\n- {STILL_AXIS} 0");
        let sub = MockSub::replying(&reply);
        let ran = run_once_in(&sub, USER_MESSAGE, StatDriftMode::On, DriftForm::All);

        assert!(
            has_field(drift_record(&ran.output), "drift_outcome=missing_axis"),
            "行が欠けた採点表が読まれてしまった: {:?}",
            ran.output
        );
        assert_eq!(ran.stats, untouched(), "捨てた回に値が動いた");
        assert!(
            ran.votes.is_empty(),
            "捨てた回に票が立った: {:?}",
            ran.votes
        );
        assert!(
            ran.charges.is_empty(),
            "捨てた回に累積が動いた（崩れが薄れる源になっている）: {:?}",
            ran.charges
        );
    }

    /// **提案が無い回は、`single` では何も薄れない**（ADR-0027 §決定2）。
    ///
    /// 「なし」と読める回に**全軸が薄れる**実装にすると、
    /// **モデルが黙った回が下向きの供給源になる**——§決定2 は
    /// 「**選ばれなかった**軸が薄れる」であって「誰も選ばれなかったら全部薄れる」ではない。
    #[test]
    fn a_turn_with_no_suggestion_never_fades_anything() {
        let sub = MockSub::replying("なし");
        let ran = run_once(&sub, USER_MESSAGE);

        assert!(has_field(
            drift_record(&ran.output),
            "drift_outcome=nothing"
        ));
        assert_eq!(ran.stats, untouched());
        assert!(
            ran.charges.is_empty(),
            "提案が無い回に累積が動いた: {:?}",
            ran.charges
        );
    }

    /// **1ターンに2軸動いた回は、書き込みの欄に「いちばん悪い結果」が出る。**
    ///
    /// # なぜ「良い方」ではいけないのか
    ///
    /// 全軸採点では**1ターンに2軸以上動きうる**（ADR-0027 §9 の安い領域）。
    /// 良い方を残すと、**1件でも書けた回が「全部書けた」に見える**——
    /// ADR-0024 §7 の「取りこぼしたら 0 行更新として観測できる」が
    /// **半分だけ嘘になる。**
    ///
    /// # この状態は「窓」を開けないと作れない
    ///
    /// 片方の軸だけを、**変容が軸を読んでから書くまでの窓**で外から動かす。
    /// **2軸とも動く回**そのものが、`single` では原理的に起きない
    /// （CLAUDE.md「フィクスチャがその分岐が効く状態に一度も入らない」）。
    #[test]
    fn a_turn_that_moves_two_axes_reports_the_worst_write() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("two.sqlite")));

        // **2軸だけにする。** どちらも同じ値から1段動く。
        runtime.block_on(put_stats(
            &fixture.store,
            fixture.character.id,
            &[(MOVED_AXIS, MOVED_START), (TWIN_AXIS, MOVED_START)],
        ));

        let (sub, open) =
            MockSub::gating_the_drift(&format!("- {MOVED_AXIS} +1\n- {TWIN_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::On,
            DriftForm::All,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while sub.call_count() < 2 {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "変容の依頼が上流へ届くこと"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                // **片方の軸だけを窓の中で動かす**（そちらの書き込みが 0 行になる）。
                put_stats(
                    &fixture.store,
                    fixture.character.id,
                    &[(MOVED_AXIS, MOVED_START), (TWIN_AXIS, MOVED_START + 2)],
                )
                .await;
                open.send(()).expect("上流を開けること");
                extractor.wait_for_the_running_task().await;
            });
        });
        let after = runtime.block_on(stats(&fixture.store, fixture.character.id));
        runtime.block_on(fixture.store.close());

        assert!(
            has_field(drift_record(&output), "drift_steps=2"),
            "前提: 帳簿が2軸とも動かしたこと: {output:?}"
        );
        assert!(
            has_field(
                drift_record(&output),
                &format!("drift_moved={}", DriftWrite::NotMoved.label())
            ),
            "取りこぼした方が書き込みの欄に出ていない: {output:?}"
        );
        assert_eq!(
            after,
            vec![
                (MOVED_AXIS.to_owned(), MOVED_START + 1),
                (TWIN_AXIS.to_owned(), MOVED_START + 2),
            ],
            "書けた方は動き、窓で入った値はそのまま残ること"
        );
    }

    /// **累積が読めない回は、値を動かさず「故障」として記録する**（レビュー指摘A / 16e-b）。
    ///
    /// # `0` として続けると、静かに巻き戻る
    ///
    /// `stat_drift_charge` に1行でも読めない行が入ると、
    /// **空の累積で続行する実装はそのキャラクターの帳簿を毎ターン `0` から始め直す。**
    /// `K` に届かないはずのターンで動き、届くはずのターンで動かない。
    ///
    /// **しかもログでは区別が付かない** —— `drift_moved` が
    /// [`WRITE_FAILED`] ではなく `moved` / [`NO_WRITE`] になるので、
    /// **`WRITE_FAILED` と `NO_WRITE` を分けた理由が半分無効になる。**
    ///
    /// # `store` 側の見張り番では足りない
    ///
    /// `tests/store_drift_charge.rs::one_unreadable_row_stops_the_whole_read` は
    /// **`Store::drift_charges` が止まること**までしか押さえていない。
    /// **止まった後にこの層が何をするか**は、ここでしか観測できない。
    ///
    /// docs/03-data-model.md の「止まるもの／止まらないもの」の表が、そのまま検査になっている。
    #[test]
    fn a_charge_that_cannot_be_read_stops_the_values_and_is_recorded_as_a_failure() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("broken.sqlite")));

        // **読めない累積を1行植える。**
        //
        // **軸名の側を壊す**（`StatAxis::parse` が `MAX_CHARS` で落とす長さ）。
        // 表の CHECK は**長さを見ない**——`stat_alloc` が許す形より狭い制約を置くと
        // **正当な行だけが静かに落ちる**ので、そう決めてある
        // （`migrations/0011_stat_drift_charge.sql`）。
        // したがって**この行は正規の CHECK を全部通り、読み出しでだけ落ちる。**
        let unreadable_axis = "あ".repeat(StatAxis::MAX_CHARS * 2);
        assert!(
            StatAxis::parse(&unreadable_axis).is_err(),
            "前提: 読み出しの検証で落ちる軸名であること"
        );
        runtime.block_on(async {
            sqlx::query(
                "INSERT INTO stat_drift_charge (character_id, axis, charge) VALUES (?, ?, ?)",
            )
            .bind(fixture.character.id.get())
            .bind(&unreadable_axis)
            .bind(0.5_f64)
            .execute(fixture.store.pool())
            .await
            .expect("読めない累積を植えられること");
        });

        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let llm: Arc<dyn LlmClient> = Arc::clone(&sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fixture.store.clone(),
            llm,
            // **抽出の上限は落とし所**（この配線は引き直しを持たない / 作業単位 B-7）。
            super::super::context::SharedSubContext::fallback_for_tests(),
            StatDriftMode::On,
            DriftForm::Single,
            DriftNote::None,
            fixture_tuning(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ));
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                extractor.wait_for_the_running_task().await;
            });
        });
        let after = runtime.block_on(stats(&fixture.store, fixture.character.id));
        let recorded = runtime.block_on(votes(&fixture.store, fixture.character.id));
        runtime.block_on(fixture.store.close());

        // **止まるもの: 値の適用。**
        assert_eq!(after, untouched(), "累積が読めないのに値が動いた");
        assert!(
            has_field(drift_record(&output), "drift_steps=0"),
            "段が動いていないことがログに出ていない: {output:?}"
        );
        // **止まらないもの: 判定と票。**
        assert!(
            has_field(drift_record(&output), "drift_outcome=applied"),
            "判定まで止まっている: {output:?}"
        );
        assert_eq!(
            recorded,
            vec![(
                fixture.session_id.get(),
                MOVED_AXIS.to_owned(),
                DriftDelta::Up.label()
            )],
            "票まで止まっている: {recorded:?}"
        );
        // **故障として記録される**（「動かなかった」と混ぜない）。
        assert!(
            has_field(
                drift_record(&output),
                &format!("drift_moved={WRITE_FAILED}")
            ),
            "故障ではなく「動かなかった」として数えている: {output:?}"
        );
    }

    /// **3票目の重みは `0.1/√3`。過去の票を全部数えている**
    /// （ADR-0027 §決定1 / 訂正 3 / レビュー指摘B）。
    ///
    /// # 2ターンでは、窓を潰す変異が観測できない
    ///
    /// 票を読み戻す窓（[`DRIFT_WEIGHT_VOTES`]）を **`1`** にすると、
    /// 重みは `0.1` と `0.1/√2` を往復するだけになる
    /// ——**`1/√n` が3票目以降まったく効かない。**
    /// **2ターンでは両者が一致する**（どちらも `0.1, 0.1/√2`）ので、
    /// `the_weight_of_the_second_vote_comes_from_the_recorded_votes` では捕まらない
    /// （CLAUDE.md「その分岐が効く状態に一度も入らない」）。
    ///
    /// ADR-0027 §決定1 の表（一律なら飽和22 / 希少度なら飽和0）が測ったのは
    /// **この希少度の重みそのもの**である。
    ///
    /// # 累積そのものと、閾値の両側で見る
    ///
    /// * 累積は `0.1 + 0.1/√2 + 0.1/√3 ≒ 0.22845`
    ///   （窓 1 なら `0.1 + 0.1/√2 + 0.1/√2 ≒ 0.24142`）
    /// * 尺度 7 なので費用は `K × 10/7`。`K = 0.164`（費用 ≒ 0.23429）を
    ///   **2つの間**に置くと、**正しい側だけが動かない**
    #[test]
    fn the_third_vote_weighs_a_third_because_every_past_vote_is_counted() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));

        // **段を動かさない費用**にして、累積そのものを読む。
        let quiet = run_turns_tuned(
            &sub,
            &[USER_MESSAGE, USER_MESSAGE, USER_MESSAGE],
            StatDriftMode::On,
            DriftForm::Single,
            DriftTuning::new(1.0, 0.0, 1_000.0).expect("つまみが範囲内であること"),
        );
        assert_eq!(quiet.votes.len(), 3, "前提: 3票立つこと");
        let charge = charge_of(&quiet, MOVED_AXIS).expect("累積の行があること");
        let want = 0.1 * (1.0 + 1.0 / 2.0_f64.sqrt() + 1.0 / 3.0_f64.sqrt());
        assert!(
            (charge - want).abs() < 1e-9,
            "3票目の重みが 0.1/√3 になっていない（累積 {charge} / 期待 {want}）: {:?}",
            quiet.charges
        );

        // **閾値の後ろ**（`0.22845 < 0.23429 < 0.24142`）。
        // 窓が潰れていると、ここで動いてしまう。
        let above = run_turns_tuned(
            &sub,
            &[USER_MESSAGE, USER_MESSAGE, USER_MESSAGE],
            StatDriftMode::On,
            DriftForm::Single,
            DriftTuning::new(1.0, 0.0, 0.164).expect("つまみが範囲内であること"),
        );
        assert_eq!(
            above.stats,
            untouched(),
            "3票の重みが 0.1 + 0.1/√2 + 0.1/√3 を超えている（過去の票を数え落としている）"
        );

        // **閾値の手前**（費用 ≒ 0.22143 < 0.22845）。全部止まっている実装と区別する。
        let below = run_turns_tuned(
            &sub,
            &[USER_MESSAGE, USER_MESSAGE, USER_MESSAGE],
            StatDriftMode::On,
            DriftForm::Single,
            DriftTuning::new(1.0, 0.0, 0.155).expect("つまみが範囲内であること"),
        );
        assert_eq!(
            below.stats.first(),
            Some(&(MOVED_AXIS.to_owned(), MOVED_START + 1)),
            "3票の累積が費用に届いていない: {:?}",
            below.stats
        );
    }

    /// **`drift_steps` は「動いた軸の数」ではなく「段の数」**（レビュー指摘D / 16e-b）。
    ///
    /// # 2つは1軸が2段以上動いたときにだけ食い違う
    ///
    /// **ADR-0027 の表は「段」を数えている**ので、こちらに揃える。
    /// 揃えないと、**`all.awk` の再現値と、次の実機測定がログから数える値が
    /// 静かにずれる。**
    ///
    /// # 向きの欄も一緒に見る
    ///
    /// `drift_delta` は**ちょうど1軸だけ動いた回**しか埋まらないので、
    /// **2軸以上動いたターンの向きはログから完全に落ちる。**
    /// `drift_ups` / `drift_downs` がその穴を埋める。
    #[test]
    fn the_step_count_in_the_log_counts_steps_not_axes() {
        // **`K = 0.03`（尺度 7 なので費用 ≒ 0.0429）/ 1票（重み 0.1）で 2 段**上がる。
        // 動くのは1軸だけ。
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let ran = run_once_tuned(
            &sub,
            USER_MESSAGE,
            StatDriftMode::On,
            DriftForm::Single,
            DriftTuning::new(1.0, 0.0, 0.03).expect("つまみが範囲内であること"),
        );

        assert_eq!(
            ran.stats.first(),
            Some(&(MOVED_AXIS.to_owned(), MOVED_START + 2)),
            "前提: 1軸が2段動くこと: {:?}",
            ran.stats
        );
        assert!(
            has_field(drift_record(&ran.output), "drift_steps=2"),
            "段ではなく軸の数を数えている: {:?}",
            ran.output
        );
        assert!(
            !has_field(&ran.output, "drift_steps=1"),
            "動いた軸の数（1）が出ている: {:?}",
            ran.output
        );
        assert!(
            has_field(drift_record(&ran.output), "drift_ups=2")
                && has_field(drift_record(&ran.output), "drift_downs=0"),
            "向きごとの段数が出ていない: {:?}",
            ran.output
        );
        // **書き込みは1回**（1軸ぶんは1件にまとめる / `DriftTurnEffect::changes` の doc）。
        assert!(
            has_field(
                drift_record(&ran.output),
                &format!("drift_moved={}", DriftWrite::Moved.label())
            ),
            "書き込みが1件にまとまっていない: {:?}",
            ran.output
        );
    }

    /// **下がった段も、向きごとの欄に出る。**
    ///
    /// 上げだけを見ていると、**`drift_downs` を `ups` の写しにする変異**が緑を通る
    /// （ADR-0024 の「危険 2. 一方向へ流れる」は**比**で読む）。
    #[test]
    fn a_downward_turn_shows_up_in_the_downward_step_count() {
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} -1"));
        let ran = run_once_tuned(
            &sub,
            USER_MESSAGE,
            StatDriftMode::On,
            DriftForm::Single,
            DriftTuning::new(1.0, 0.0, 0.03).expect("つまみが範囲内であること"),
        );

        assert_eq!(
            ran.stats.first(),
            Some(&(MOVED_AXIS.to_owned(), MOVED_START - 2)),
            "前提: 1軸が2段下がること: {:?}",
            ran.stats
        );
        assert!(
            has_field(drift_record(&ran.output), "drift_downs=2")
                && has_field(drift_record(&ran.output), "drift_ups=0"),
            "下がった段が上がった段の欄に出ている: {:?}",
            ran.output
        );
        assert!(
            has_field(drift_record(&ran.output), "drift_steps=2"),
            "{:?}",
            ran.output
        );
    }

    /// **変容の上限も、その往復の窓から出る**（作業単位 B-7 / レビュー指摘3 / 2026-08-27）。
    ///
    /// # ここが塞いでいる穴（**反対側の入口**）
    ///
    /// B-7 の最初の版は**抽出だけ**を窓から出し、変容は定数（8192）のままだった。
    /// **サブが 6144 で載っている機械では、こうなる:**
    ///
    /// | 裏方 | 上限 | 何が起きるか |
    /// |---|---|---|
    /// | 抽出 | 3072 | 正しく捨てる |
    /// | 変容 | **8192**（窓より大きい） | **窓を超えた依頼が上流へ出る**（LM Studio では前方から黙って切り捨てられる / ADR-0013。llama は 400 `exceed_context_size_error` で断る / ADR-0052 M5。ただし、プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）） |
    ///
    /// **変容は本文に出ない**（ADR-0024 §9）ので、LM Studio で黙って切り捨てられても
    /// **利用者にもログにも出ない。**
    /// **「同じ危険に入口が2つあるとき、片方だけ塞ぐと反対側から入られる」**（CLAUDE.md / 16e-b）。
    ///
    /// # 2つの腕（高い側と安い側を1本に）
    ///
    /// **同じ会話**を2つの窓で通す。会話の長さは**わざと2つの上限の間**に置いてある。
    ///
    /// | 窓 | 変容の上限 | この会話 | 期待 |
    /// |---|---|---|---|
    /// | 落とし所（16384） | 8192 | 収まる | **走る** |
    /// | 4096（落とし所ではない） | 2048 | 超える | **捨てる** ＋ `drift_limit_tokens=2048` |
    ///
    /// **定数に固定する変異は、狭い側で「走ってしまう」ので赤になる。**
    /// **安い側だけでは足りない**——「常に捨てる」実装が緑になる（CLAUDE.md）。
    #[test]
    fn the_drift_limit_follows_the_shared_window_too() {
        /// 落とし所ではない狭い窓。**変容の上限は 2048。**
        const NARROW: u32 = 4_096;
        /// 会話の長さ（バイト）。**2048 を超え、8192 には収まる。**
        const CONVERSATION_BYTES: usize = 3_000;

        assert_ne!(
            NARROW,
            SubContext::FALLBACK.tokens(),
            "前提: 測る窓が落とし所と別であること（同値だと定数への固定を検出できない）"
        );

        let narrow = SubContext::new(NARROW).expect("抽出に使える窓であること");
        let message = "a".repeat(CONVERSATION_BYTES);

        // --- 高い側: 落とし所の窓なら、この会話で変容は走る ---
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let wide_run = run_once_with_window(&sub, &message, SubContext::FALLBACK);
        assert!(
            wide_run.output.contains(RAN_MESSAGE),
            "前提: 落とし所の窓ならこの会話で変容は走ること: {:?}",
            wide_run.output
        );

        // --- 安い側: 窓が狭ければ、同じ会話を捨てる ---
        let sub = MockSub::replying(&format!("- {MOVED_AXIS} +1"));
        let narrow_run = run_once_with_window(&sub, &message, narrow);

        assert_eq!(
            sub.call_count(),
            0,
            "狭い窓では上流へ送らないこと: {:?}",
            narrow_run.output
        );
        // **理由も上限も、捨てたことを告げる行の中で見る**（CLAUDE.md「同じ行の中で」）。
        // **抽出側の飛ばし記録が同じ出力に、しかも先に出る**ので、
        // [`skip_record`] で**自分のイベントの行**を選ぶ。
        //
        // **⚠ 欄の綴りを鍵にして `line_containing` で選ばないこと**（B-21c のレビュー指摘3）。
        // あちらは素の部分一致なので、**キーの側が伸びた行**（`x_` を足した綴り）を掴む。
        // **実測**（2026-08-29）——抽出の飛ばし記録へ `x_` を足した欄を1本置くと、
        // 欄で選ぶ形は**抽出の行を掴み、実装が正しいままこの見張り番が赤**になった。
        // **掴んだ行がたまたま同じ欄を持てば、逆に壊れたまま緑になる**（こちらは未実測）。
        // **同じファイルの中で接頭辞が伸びる分には、欄名の衝突検査は掛からない。**
        let skipped = skip_record(&narrow_run.output);
        assert!(
            has_field(
                skipped,
                &format!("drift_limit_tokens={}", narrow.prompt_limit())
            ),
            "その回に効いていた上限を載せること（落とし所の数を出さないこと）: {skipped:?}"
        );
        assert!(
            !has_field(
                skipped,
                &format!("drift_limit_tokens={FALLBACK_MAX_DRIFT_PROMPT_TOKENS}")
            ),
            "落とし所の上限を出していないこと: {skipped:?}"
        );
        assert_eq!(narrow_run.stats, untouched(), "値が動かないこと");
    }
}
