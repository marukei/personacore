//! **抽出プロンプトを、実機で腕どうし比べるための道具**
//! （作業単位 21a / 22a で腕の表に一般化 / **B-11 で表が1行になった** /
//! ADR-0021 §14・§15・§16 / ADR-0037 §決定3・§決定4）。
//!
//! # 通常は走らない
//!
//! **実機を触るテストは `#[ignore]`**。LM Studio は常時稼働ではない
//! （`~/.claude/CLAUDE.md` / docs/04-environment.md）ので、素の `cargo test` で
//! 落ちてはいけない。**落ちている状態を「壊れている」と誤診しないこと。**
//!
//! 例外は [`the_two_labels_can_be_told_apart_by_grep`]（21a-2）、
//! [`the_arm_table_really_produces_distinct_arms`]（22a）、
//! [`the_arm_name_carries_its_own_spellings`]（22a-2）、
//! [`a_truncated_completion_loses_only_its_last_line`]（24-2）、
//! [`the_rows_come_out_as_raw_then_memories_then_discard`]（A-1a / B-11 で縮めた）の
//! 5本で、**どれも上流にも台本にも触らない**（純粋関数と定数しか通らない）。
//! **素の `cargo test` で回る見張り番はこの5本だけ**なので、
//! `#[ignore]` を付けないこと。
//!
//! # 何を比べるのか
//!
//! **腕は [`ARMS`] の表が決める**（作業単位 22a）。表の1行が [`ExampleStyle`] で、
//! **同じ台本・同じ既知の記憶・同じモデルで、表の順に交互に回す。**
//!
//! **出力の `arm` 欄は綴りから導出する**（[`arm_name`] / 22a-2 の是正）。
//! **表は名前を持たない**ので、名前と綴りがずれた状態は書けない。
//!
//! # ⚠ **表はいま1行しかない**（作業単位 B-11 / ADR-0037 §決定4）
//!
//! | 腕（`arm` 欄） | 何 |
//! |---|---|
//! | `Balanced` | **本番の形。** `- 主語: 事実` を1本の依頼で尋ねる |
//!
//! **文面の実験を当面やめた**ので、比べる相手が無い（ADR-0037 §決定1——
//! 7回試して質を上げたものが1つも無かった）。
//! **器具は残してある**——次に使うのは **C-11「別のモデルで同じ台本を回す」**で、
//! そのときは**腕どうしではなくモデルどうしを比べる**（ADR-0037 §決定3）。
//!
//! > **腕を比べる実験に戻すなら、[`ARMS`] に行を足すだけでよい。**
//! > **[`assert_the_arms_differ`] が全ペアを突き合わせる**ので、
//! > 同じ組を2行並べたり分岐が潰れていたりすれば、測定を始める前に落ちる。
//!
//! # 使い方
//!
//! 1. LM Studio を起動する（`http://127.0.0.1:1234`）。モデルは JIT ロードされる。
//! 2. 台本と既知の記憶を用意する（下記の形式）。**リポジトリには置かない。**
//! 3. 走らせる:
//!
//! ```text
//! PERSONACORE_HARNESS_TRANSCRIPT=/path/transcript.jsonl \
//! PERSONACORE_HARNESS_KNOWN=/path/known.jsonl \
//! PERSONACORE_HARNESS_REPS=3 \
//! cargo test --test extraction_quality_harness -- --ignored --nocapture
//! ```
//!
//! ## 入力の形式（どちらも JSONL / 1行1件）
//!
//! * `PERSONACORE_HARNESS_TRANSCRIPT` — `{"turn": 1, "user": "…", "character": "…"}`
//!   `character` は**生出力**（`message.raw_content` 相当）。
//! * `PERSONACORE_HARNESS_KNOWN` — `{"subject": "user"|"character", "content": "…", "category": null|"…"}`
//!
//! ## つまみ
//!
//! | 環境変数 | 既定 | 何 |
//! |---|---|---|
//! | `PERSONACORE_HARNESS_REPS` | 1 | 同じ入力を何回回すか |
//! | [`ENV_LLM_ENDPOINT`] | [`DEFAULT_ENDPOINT`] | 上流 |
//! | [`ENV_SUB_MODEL`] | [`DEFAULT_SUB_MODEL`] | 裏方のモデル |
//!
//! **繰り返しに意味がある。** 抽出の温度は **`llm::SUB_LLM_TEMPERATURE`**
//! （`server::extract` が明示する / 作業単位 A-7d）で、**0 ではない**ので
//! **同じ入力でも出力は毎回変わる。** これが検出力の源。
//!
//! **⚠ かつてここは「温度を指定していない」と書いていた。**
//! **A-7d で明示になったが、採った値は今日の実効の既定そのもの**なので、
//! **この器具が受け取る出力の散らばりは変わっていない**
//! （[ADR-0038](../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定2）。
//! **⚠ その値を `0` にすると、この器具は何も測れなくなる。**
//!
//! # 出力（標準出力 / `--nocapture` で読む前提）
//!
//! タブ区切りで、1行が1つの観測。`#` で始まる行は覚書。
//!
//! ```text
//! #<TAB>覚書                                       … 見出し・腕ごとの枠のトークン数など
//! 腕<TAB>ターン<TAB>回<TAB>RAW<TAB>{"…"}          … サブLLM の生出力（JSON 文字列）
//! 腕<TAB>ターン<TAB>回<TAB>利用者<TAB>本文        … 保存されるはずの記憶1件
//! 腕<TAB>ターン<TAB>回<TAB>DISCARD<TAB>理由=件数 …   … 捨てた内訳（1ターン1行）
//! 腕<TAB>ターン<TAB>回<TAB>SKIP<TAB>too_long=… … **上流を呼ばなかった回**（§8 の門）
//! 腕<TAB>ターン<TAB>回<TAB>SKIP<TAB>upstream_error=… … **上流が失敗した回**
//! ```
//!
//! **記憶の行に分類（`memory.category`）は出さない。** 欄を増やすと集計側の列が動くので、
//! **分類が要るときは `RAW` の行から読む**（生出力には角括弧のまま残っている）。
//!
//! ## **1ターンに `RAW` → 記憶 → `DISCARD` の組を1つ出す**
//!
//! **欄は増やさない**（5欄のまま）。**`RAW` の行は直後の `DISCARD` と同じターンのもの。**
//! 見張り番は [`the_rows_come_out_as_raw_then_memories_then_discard`]。
//!
//! > **⚠ `ask=` 欄は B-11 で落とした**（ADR-0037 §決定4）。
//! > 依頼が1本しか無いので、**名乗るべき相手が居ない。**
//!
//! ## **`DISCARD` と `SKIP` は別の目印**（21a-2 の是正）
//!
//! 21a は3つの別物——「§8 の門で捨てた回」「上流が失敗した回」「毎ターンの内訳」——を
//! **同じ `DISCARD` の欄に出していた。** そうすると `grep DISCARD | wc -l` が
//! **成功したターンも数える。** `16l` で実際に踏んだ形そのもの
//! （集計の道具が本番と別の受理集合を持ち、5つの測定を数え直した）。
//!
//! * [`DISCARD_LABEL`] — **上流が答えた回にだけ出る。** **1ターンにつき1行。**
//!   分解器の内訳と `parsed` / `emphasis_stripped` / `quotes_stripped` /
//!   `example_echoes` / `example_format_echoes` / 控除リストの件数。
//!   （`quotes_stripped` は**行頭の `> ` を落とした行数** / 作業単位 B-29。
//!   C-15 で 5.5% の観測がこの形だった。**是正が効いたかは、この欄でしか測れない。**
//!   `example_echoes` は**依頼の手本の文を写した回数** / 作業単位 B-28。
//!   **捨てていない**ので破棄には現れず、**この欄でしか見えない。**
//!   `example_format_echoes` は**手本の「書式」（`[好み]` の角括弧）が
//!   保存される本文に居座った回数** / 作業単位 B-33。
//!   **`example_echoes` はこれを 0 と数える**ので、混ぜずに別の欄で出す。）
//!   **欄そのものの見張り番は [`the_normalisation_counts_ride_in_their_own_fields`]。**
//! * [`SKIP_LABEL`] — **上流を呼ばなかった／呼べなかった回。** この行が出たターンには
//!   `RAW` も記憶の行も無い。
//!
//! ## **`DISCARD` を必ず出す**
//!
//! CLAUDE.md:「道具の出力に、サーバが出した判定の内訳を必ず一緒に載せること」。
//! `16l` で実際に踏んだ——**集計の道具だけが本番のパーサより厳しく、
//! 正しく処理されたターンを「壊れている」と数えていた。**
//! 気付けたのは**道具の数とサーバの判定を突き合わせたから**にすぎない。
//! **0 件の理由も含めて全部出す**（欄が消えると集計の列が揃わない）。
//!
//! ## **生出力も必ず出す**
//!
//! `parse_extraction` が捨てた行は、この道具の他の欄には一切現れない。
//! **絶対制約「`message.raw_content` を必ず保持する」と同じ理由**で、
//! 崩れたときの唯一の調査手段になる。改行を含むので **JSON 文字列**にして1行に畳む。
//!
//! ## **集計はしない**
//!
//! 数えるのは司令塔の仕事。**この道具は観測を並べるだけ**にしてある——
//! 数え方を道具側に持つと、本番のパーサと受理集合がずれても気付けない（`16l`）。
//!
//! # ⚠ **21b の数を ADR-0021 §14 の表と並べないこと**（21a-2 で足した）
//!
//! §14 の表（列挙した語 5 → 1 / 列挙していない語 3 → 4）は**本番のサーバ**で出た数で、
//! **控除リスト（すでに知っていること）が会話の進行につれて増えていく**状態の値。
//! 13c-3 はその機構が**保存件数を 4 → 8 に動かした**ことを観測している。
//!
//! **この道具は既知の記憶を全ターン・全腕で固定する**（下の「守っていること」）ので、
//! **その機構が再現していない。** つまり:
//!
//! * **腕どうしの比較**（[`ARMS`] の行どうし）は成立する——全腕が同じ固定条件。
//! * **13c-4 / §14 の数との比較は成立しない。** 条件が違う。
//!   並べて「良くなった／悪くなった」と読むと、**測っていないものを測ったことにする。**
//!
//! > **⚠ 21b / 22b / A-1 の数とも、直接は並べられない。**
//! > 台本と既知の記憶を同じにすれば条件は揃うが、**揃えたことを確かめてから並べること。**
//!
//! CLAUDE.md:「表は、どの設定で出したかを書かないと写しですらない」。
//!
//! # 守っていること
//!
//! * **全腕を同じプロセスの中で交互に回す。並行させない。**
//!   `16k` で実測: 2つの版が同時に走ると1ターンが 23s → 31.5s に伸び、
//!   **腕どうしを比べるための測定が、腕どうしの取り合いで汚れた**（CLAUDE.md）。
//! * **温度・応答の上限・予算・§8 の門は、本番から取る。写さない。**
//!   通るのは [`extraction_request_for_tests`] /
//!   [`extraction_prompt_budget_for_tests`]（どちらも `server::extract` の実体へ委譲）。
//!   **一致は単体テストが押さえている**——
//!   `server::extract::tests::the_test_only_entry_points_hand_back_what_production_sends`
//!   が、本番の `extract()` が上流へ渡した依頼と**この道具とまったく同じ経路**で
//!   組んだ依頼を突き合わせる（21a-2 の是正。それまで**何も守っていなかった**）。
//! * **測定を始める前に、腕どうしのプロンプトが実際に違うことを確かめる**
//!   （[`assert_the_arms_differ`]）。**表の全ペアを突き合わせ、1組でも同一なら即座に落ちる。**
//!   `ExampleStyle::first_example_line` の分岐が潰れると、
//!   この道具は**同一のプロンプトで複数の腕を回し、司令塔は「差が無かった」と読む**
//!   ——結論が反転する。
//!   単体テストは今日その変異を赤にするが、**本体は `#[ignore]` なので、
//!   実機で回す当日に何かが崩れていても誰も見ていない。**
//!   **黙って回り続けるのがいちばん悪い。**
//!   **だから同じ判定を、植えた台本で素の `cargo test` からも通す**
//!   （[`the_arm_table_really_produces_distinct_arms`] / 22a）。
//!   **⚠ 表が1行のあいだ、全ペアの突き合わせは空回りする**（B-11）——
//!   **鳴っているのは「どの腕も依頼が空でない」だけ。**
//!   **行を足した日に、初めて元の強さに戻る。**
//! * **腕の名前と依頼は、同じ組から一度に作る**（[`build_arm`] / 22a-2）。
//!   **確かめる側（[`assert_the_arms_differ`]）と送る側（[`run_once`]）が同じ関数を通る**ので、
//!   **確かめた依頼と送る依頼が食い違う**形にはならない。
//!   **名前を手書きしないのは、22a がそこで反転しうる形だったから**（[`ARMS`] の doc）。
//! * **上流が答えた回の翻訳は [`observations`] に切り出してある**（作業単位 24-2）。
//!   **`println!` する側には判断が残っていない**ので、
//!   **素の `cargo test` から同じ関数を通せる**（[`a_truncated_completion_loses_only_its_last_line`]）。
//!   切り出す前は、**打ち切りの申告を捨てる書き換えが 1501 全緑**だった（レビューで実測）。
//! * **既知の記憶は全ターン・全腕で同じ一覧。** この道具では固定する
//!   （切り詰めは本番の `build_extraction_prompt` に任せる）。
//! * `println!` を使っているのは、**観測の転記こそがこの実行の成果物**だから。
//!   本番コードのログは `tracing`（CLAUDE.md 5節）。

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use personacore::domain::{
    CharacterId, DisplayName, Memory, MemoryCategory, MemoryContent, MemoryId, Subject,
};
use personacore::llm::{
    ChatMessage, Completion, DEFAULT_ENDPOINT, DEFAULT_SUB_MODEL, FinishReason, LlmClient,
    LmStudioClient, LmStudioConfig, ModelId,
};
use personacore::parse::extraction::parse_extraction;
use personacore::prompt::extraction::{
    ExampleStyle, ExtractionInput, ExtractionPrompt, build_extraction_prompt, example_echoes,
    example_format_echoes, request_tokens,
};
use personacore::server::{
    ENV_LLM_ENDPOINT, ENV_SUB_MODEL, extraction_prompt_budget_for_tests,
    extraction_request_for_tests, response_completeness_for_tests,
};
use serde::Deserialize;

mod common;

/// **欄の突き合わせは共有の1本を使う**（作業単位 B-13）。
///
/// ここには**同じ本体の写しが2本**あった（`has_field` と、別名の `field_is_present`）。
/// 写しのままだと、`src/testlog.rs` に置いてある押さえ
/// （`has_field_matches_a_whole_token_and_nothing_longer`）が
/// **このファイルには届かない**——`contains` へ退化させても誰も赤くならない。
///
/// **`testing` フィーチャの下へ出す形は採らない**（CLAUDE.md 2節）。
/// 共有は `tests/common/mod.rs` の `#[path]` 1本を通す。
use common::has_field;

/// **腕の表**（作業単位 22a / 22a-2 で表示名を落とした / **B-11 で1行になった**）。
///
/// # **表示名を表に持たない**（22a-2 の是正）
///
/// 出力の `arm` 欄は [`arm_name`] が**綴りから導出する。**
/// 22a は表の第1要素に手書きの表示名を置いていたが、
/// **名前と綴りの対応を守るものが何も無かった**——表示名2つを入れ替えるだけの変異が
/// **1488 全緑**（レビューで実測）。**TSV の欄も目印も正常なまま、
/// 司令塔が読む結論だけが左右反転する。** 22a の存在理由そのものを横から無効化する形。
///
/// **見張り番で守るのをやめて、書けなくした。** 表に名前が無いので、
/// **名前が綴りとずれた状態は、この表では作れない。**
///
/// > **⚠ 「型で表現できなくした」ではない**（CLAUDE.md / `0012b`）。
/// > **寄りかかっているのは [`ExampleStyle`] の `Debug` の綴り**で、
/// > **`Debug` を手で実装すれば嘘の名前を返せる**（そのときは `arm_name` も
/// > [`the_arm_name_carries_its_own_spellings`] も一緒に嘘を向くので、
/// > **素の `cargo test` は緑のまま**）。
/// > **`derive(Debug)` を手書きの実装に置き換えないこと**が、この形の前提。
///
/// # 出力の `arm` 欄になる
///
/// **司令塔の集計 script はこの綴りで行を振り分ける。**
/// **腕どうしで必ず別の綴りにすること**——同じ綴りが2行あると、
/// **TSV の上で2つの腕が1つに混ざる**（[`assert_the_arms_differ`] が赤にする）。
///
/// # ⚠ **いまは1腕**（作業単位 B-11 / ADR-0037 §決定4）
///
/// | 腕（`arm` 欄） | 何 |
/// |---|---|
/// | `Balanced` | **本番の形。** 比べる相手は無い |
///
/// **文面の実験をやめたので、対の綴りが1つも残っていない**
/// （21b で `Category` は、22b で `CharacterOnly` は、A-1 で `Split` は不採用）。
/// **次にこの器具を使うのは C-11「別のモデルで同じ台本を回す」**で、
/// そこで比べるのは**腕どうしではなくモデルどうし**（ADR-0037 §決定3）。
///
/// > **⚠ 表が1行のあいだ、[`assert_the_arms_differ`] の
/// > 「全ペアが別物」は空回りする**（ペアが存在しない）。
/// > **腕を比べる実験に戻すときは、行を足すだけでよい**——
/// > 足した瞬間に、あの検査は元の強さに戻る。
const ARMS: [ExampleStyle; 1] = [ExampleStyle::Balanced];

/// **表を空にできないことを、テストターゲットのコンパイル時に止める**（作業単位 B-11）。
///
/// # なぜ実行時の `assert!` から移したのか
///
/// B-11 は [`assert_the_arms_differ`] の下限を `>= 2` から `>= 1` へ緩めた
/// （対の綴りが1つも残らなかったため / ADR-0037 §決定4）。
/// **緩めた結果、その `assert!` は死んだ**——[`ARMS`] はコンパイル時定数なので
/// **今のコードにはその行を発火させる状態が存在しない**（変異検査 M11 で実測: 1579 全緑）。
///
/// **`const _` なら、`[ExampleStyle; 0]` に書き換えた瞬間に
/// `error[E0080]: evaluation panicked` で止まる**（実測 / 2026-08-27 の B-11 のレビュー）。
/// **空の表は「片腕だけ測る」より悪い**——`#[ignore]` の本体が1行も測らないまま緑で終わる。
///
/// # ⚠ **「ビルド時に止まる」とは書けない**
///
/// **[`ARMS`] は `tests/` にあるので、この `const _` を評価するのは
/// `cargo test` と `cargo clippy --all-targets` のときだけ。**
/// **素の `cargo build` は test target を作らないので止まらない。**
/// CLAUDE.md が「素の `cargo build` だけが検出できる」と強く書いている手前、
/// **ここを「ビルド時」と書くと必ず読み違えられる。**
///
/// # 守っていないこと
///
/// **「腕が2つ以上あること」は、もうどこも守っていない。**
/// 表が1行なのが今の正しい状態なので、下限は 1 が限界
/// （[`assert_the_arms_differ`] の doc）。
const _: () = assert!(
    !ARMS.is_empty(),
    "腕が1つも無い（`#[ignore]` の本体が1行も測らないまま緑で終わる）"
);

/// **腕の表示名を綴りから導出する**（作業単位 22a-2 / B-11 で綴りが1つになった）。
/// 例 `Balanced`。
///
/// **導出はここ1箇所だけ。** [`ARMS`] にも [`run_once`] にも名前の実体を置かない——
/// 置いた瞬間、**名前と綴りがずれた状態が書けるようになる**（22a が実際にそうだった）。
///
/// 寄りかかっている前提は [`ARMS`] の doc を参照（`Debug` の綴り）。
fn arm_name(example: ExampleStyle) -> String {
    format!("{example:?}")
}

/// 台本のパスを指す環境変数。
const ENV_TRANSCRIPT: &str = "PERSONACORE_HARNESS_TRANSCRIPT";

/// 既知の記憶のパスを指す環境変数。
const ENV_KNOWN: &str = "PERSONACORE_HARNESS_KNOWN";

/// 繰り返し回数を指す環境変数。**既定は 1。**
const ENV_REPS: &str = "PERSONACORE_HARNESS_REPS";

/// キャラクターの呼び名。**台本の中身と揃える必要はない**——
/// 依頼の枠の大きさにしか効かない。
const DISPLAY_NAME: &str = "葵";

/// 既知の記憶に振る `created_at` の起点（Unix 秒）。
///
/// **行の並び順がそのまま「古い順」になる。** 本番の `Store::recall_memories` も
/// 古い順に返すので、**ファイルの末尾がいちばん新しい。**
const KNOWN_EPOCH_SECONDS: i64 = 1_700_000_000;

/// 生出力の欄の目印。
const RAW_LABEL: &str = "RAW";

/// 破棄の内訳の欄の目印。**上流が答えた回にだけ出る**（モジュールの doc）。
const DISCARD_LABEL: &str = "DISCARD";

/// **上流を呼ばなかった／呼べなかった回**の目印（21a-2）。
///
/// [`DISCARD_LABEL`] と分けてある——同じ目印にすると
/// `grep DISCARD | wc -l` が**成功したターンも数える**（モジュールの doc / `16l`）。
const SKIP_LABEL: &str = "SKIP";

/// 台本の1ターン。
#[derive(Debug, Deserialize)]
struct Turn {
    /// ターン番号。**出力の欄になるだけ**で、順序は行の並びが決める。
    turn: u32,
    /// 利用者の発言。
    user: String,
    /// キャラクターの応答。**生出力**（`message.raw_content` 相当）。
    character: String,
}

/// 既知の記憶1件。
#[derive(Debug, Deserialize)]
struct KnownRow {
    /// `user` か `character`。**DB の綴り**（`Subject::extraction_word` の日本語ではない）。
    subject: String,
    content: String,
    category: Option<String>,
}

/// 環境変数を読む。無ければ `None`。
fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|raw| !raw.trim().is_empty())
}

/// 台本を読む。
fn read_transcript(path: &PathBuf) -> Vec<Turn> {
    let raw = std::fs::read_to_string(path).expect("台本を読めること");
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Turn>(line).expect("台本の1行が JSON であること"))
        .collect()
}

/// 既知の記憶を読む。**並び順がそのまま古い順になる。**
fn read_known(path: &PathBuf) -> Vec<Memory> {
    let raw = std::fs::read_to_string(path).expect("既知の記憶を読めること");
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| {
            let row: KnownRow =
                serde_json::from_str(line).expect("既知の記憶の1行が JSON であること");
            let subject = match row.subject.as_str() {
                "user" => Subject::User,
                "character" => Subject::Character,
                other => panic!("subject は user か character のいずれか: {other:?}"),
            };
            let index = i64::try_from(index).expect("件数が i64 に収まること");
            let created_at: DateTime<Utc> =
                DateTime::from_timestamp(KNOWN_EPOCH_SECONDS + index, 0).expect("正当な時刻");

            Memory {
                // **`id` も並び順に合わせる。** 同じ秒に並んだときの順序は
                // `id` の大きい方が新しい（`prompt::extraction::fit_known_memories`）。
                id: MemoryId::from_raw_for_tests(index + 1),
                character_id: CharacterId::from_raw_for_tests(1),
                subject,
                content: MemoryContent::parse(&row.content).expect("正当な記憶本文であること"),
                category: row
                    .category
                    .as_deref()
                    .map(|raw| MemoryCategory::parse(raw).expect("正当な分類であること")),
                created_at,
                locked: false,
            }
        })
        .collect()
}

/// 上流のクライアント。**モデルは裏方の既定**（本番と同じ定数から取る）。
fn sub_client() -> LmStudioClient {
    let endpoint = env(ENV_LLM_ENDPOINT).unwrap_or_else(|| DEFAULT_ENDPOINT.to_owned());
    let model = env(ENV_SUB_MODEL).unwrap_or_else(|| DEFAULT_SUB_MODEL.to_owned());
    let model = ModelId::parse(&model).expect("モデル識別子が正当であること");
    let config = LmStudioConfig::new(&endpoint, model).expect("エンドポイントが正当であること");

    LmStudioClient::new(config).expect("上流クライアントを作れること")
}

/// TSV の欄に入れられる形へ均す。**タブと行区切りだけを空白にする。**
///
/// `parse_extraction` は行単位で読むので本文に改行は入らないはずだが、
/// **道具の側で欄が壊れると集計が黙ってずれる**ので、ここで潰しておく。
fn one_line(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character == '\t' || character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

/// **腕1つぶんの「名前」と「依頼」を、同じ組から一度に作る**（作業単位 22a-2）。
///
/// # 名前と依頼を別々に組まない
///
/// 別々に組むと、**片方だけ別の綴りから作る書き換え**が通る——
/// 22a の表がまさにその形で、**依頼は綴りから、名前は手書き**だった。
/// ここを通せば、**`arm` 欄に出る名前と上流へ送る依頼は、必ず同じ組から出る。**
///
/// [`assert_the_arms_differ`]（測定前の突き合わせ）と [`run_once`]（実際に送る側）が
/// **どちらもこの関数を通る。** 別々に書くと、**確かめた依頼と送る依頼が食い違いうる**
/// ——予算や既知の記憶の渡し方が片方だけ変わっても、誰も鳴らない。
///
/// > **⚠ 「取り違えを書けなくした」ではない。**
/// > [`run_once`] は `#[ignore]` の本体からしか呼ばれないので、
/// > **その中で名前だけを作り直す書き換えは、素の `cargo test` では観測できない**
/// > （22a-2 の変異検査で実測 —— 1489 全緑）。**塞げたのは「写しが2つある」ことまで。**
///
/// # **1腕につき依頼は1本**（B-11 で `AskStyle` を落とした / ADR-0037 §決定4）
fn build_arm(
    display_name: &DisplayName,
    known: &[Memory],
    turn: &Turn,
    example_style: ExampleStyle,
) -> (String, ExtractionPrompt) {
    let prompt = build_extraction_prompt(&ExtractionInput {
        display_name,
        user_message: &turn.user,
        character_message: &turn.character,
        known_memories: known,
        // **予算は本番から取る。写さない。**
        budget: extraction_prompt_budget_for_tests(),
        example_style,
    });
    (arm_name(example_style), prompt)
}

/// **上流が答えた1ターンぶん。**
struct Answer {
    /// 上流の答え。
    completion: Completion,
    /// その依頼に載った控除リストの件数。
    known: KnownCounts,
}

/// 1回分（**腕1つ・ターン1つ**）を回して、観測を標準出力へ並べる。
///
/// **並行させない**——腕どうしが GPU を取り合うと、腕どうしを比べるための測定が
/// 取り合いで汚れる（`16k` で実測 / CLAUDE.md）。
async fn run_once(
    client: &dyn LlmClient,
    display_name: &DisplayName,
    known: &[Memory],
    turn: &Turn,
    rep: u32,
    example_style: ExampleStyle,
) {
    // **`arm` 欄は綴りから導出する。手書きの名前を受け取らない**（22a-2 / [`arm_name`]）。
    // **名前と依頼は同じ綴りから一度に作る**（[`build_arm`]）。
    let (arm, prompt) = build_arm(display_name, known, turn, example_style);
    let known_counts = KnownCounts {
        included: prompt.included_memories(),
        dropped: prompt.dropped_memories(),
    };
    let messages: Vec<ChatMessage> = prompt.into_messages();

    // **温度・応答の上限・§8 の門も本番から取る。**
    let request = match extraction_request_for_tests(messages) {
        Ok(request) => request,
        Err(tokens) => {
            // **本番なら丸ごと捨てる回**（ADR-0021 §8）。上流を呼ばずに記録する。
            println!(
                "{arm}\t{}\t{rep}\t{SKIP_LABEL}\ttoo_long={tokens}",
                turn.turn
            );
            return;
        }
    };

    let answer = match client.complete(&request).await {
        Ok(completion) => Answer {
            completion,
            known: known_counts,
        },
        Err(err) => {
            println!(
                "{arm}\t{}\t{rep}\t{SKIP_LABEL}\tupstream_error={}",
                turn.turn,
                one_line(&err.to_string())
            );
            return;
        }
    };

    // **観測の組み立ては [`observations`]。ここは並べるだけ**（作業単位 24-2）。
    for line in observations(&arm, turn.turn, rep, &answer) {
        println!("{line}");
    }
}

/// **依頼に載った既知の記憶と、予算に入らなかった件数。**
///
/// **2つの `usize` を裸で並べて渡さない。** 取り違えても型は通り、
/// TSV の `known_included` / `known_dropped` だけが静かに入れ替わる
/// （本番の `KnownMemoryCounts` と同じ理由）。
#[derive(Debug, Clone, Copy)]
struct KnownCounts {
    /// 依頼の「すでに知っていること」の節に載った件数。
    included: usize,
    /// 予算に入らず落とした件数。
    dropped: usize,
}

/// **上流が答えた1回ぶんを、TSV の行に翻訳する。純粋関数**（作業単位 24-2）。
///
/// # ここが塞いでいる穴
///
/// [`run_once`] は `#[ignore]` の本体からしか呼ばれない。
/// **つまり打ち切りの翻訳を器具が捨てても、素の `cargo test` は何も言わない**
/// ——レビューで実測: 引数を `completion.finish_reason` → `None` に変える変異は
/// **1501 全緑**だった。**器具だけが甘くなり、断片を採った記憶を
/// 「モデルが出した記憶」として数える**（`16l` の裏返し）。
///
/// **上流にも `println!` にも触らない形へ切り出して、素の `cargo test` から通す**
/// ——見張り番は [`a_truncated_completion_loses_only_its_last_line`]。
/// **`println!` する側（[`run_once`]）には、もう判断が残っていない。**
///
/// # 欄を変えないこと
///
/// `arm/turn/rep/kind/value` の5欄と [`RAW_LABEL`] / [`DISCARD_LABEL`] /
/// [`SKIP_LABEL`] の目印は**司令塔の集計 script が依存している。**
///
/// # **1ターンに `RAW` → 記憶 → `DISCARD` の組を1つ出す**
///
/// **欄は増やさない。** `RAW` の行は**その組の `DISCARD` と同じターンのもの。**
/// **並びが帰属を決めている**（`RAW` の行には目印以外の欄が無い）ので、
/// 順序を入れ替えないこと。
/// 見張り番は [`the_rows_come_out_as_raw_then_memories_then_discard`]。
///
/// > **⚠ `ask=` 欄は B-11 で落とした**（ADR-0037 §決定4）——
/// > 依頼が1本しか無いので、**名乗るべき相手が居ない。**
fn observations(arm: &str, turn: u32, rep: u32, answer: &Answer) -> Vec<String> {
    let mut lines = Vec::new();

    // **生出力を先に出す。** ここから先で何件残ろうと、調べる材料は必ず残る。
    let raw =
        serde_json::to_string(&answer.completion.content).expect("文字列を JSON にできること");
    lines.push(format!("{arm}\t{turn}\t{rep}\t{RAW_LABEL}\t{raw}"));

    // **打ち切られた回の最後の行は断片**（ADR-0021 §17 / 作業単位 24）。
    // **翻訳の規則を写さない**——本番と同じ [`response_completeness_for_tests`] を通す。
    let extracted = parse_extraction(
        &answer.completion.content,
        response_completeness_for_tests(answer.completion.finish_reason),
    );
    for memory in extracted.memories() {
        lines.push(format!(
            "{arm}\t{turn}\t{rep}\t{}\t{}",
            memory.subject.extraction_word(),
            one_line(memory.content.as_str())
        ));
    }

    // **サーバが出した判定の内訳。0 件の理由も出す**（欄を消さない / モジュールの doc）。
    let discards = extracted
        .discards()
        .iter()
        .map(|(reason, count)| format!("{}={count}", reason.label()))
        .collect::<Vec<_>>()
        .join(" ");
    lines.push(format!(
        "{arm}\t{turn}\t{rep}\t{DISCARD_LABEL}\t{discards} parsed={} \
         emphasis_stripped={} quotes_stripped={} example_echoes={} \
         example_format_echoes={} known_included={} known_dropped={}",
        extracted.memories().len(),
        extracted.emphasis_stripped(),
        extracted.quotes_stripped(),
        // **手本の写しは生出力から数える**（作業単位 B-28）。
        // **本番と同じ関数**——道具が別の受理集合を持つと `16l` の形になる。
        example_echoes(&answer.completion.content),
        // **手本の「書式」の居座りは、採った記憶から数える**（作業単位 B-33）。
        // **見る先が違うのは本番と同じ**（`server::extract::record_run` の doc）。
        example_format_echoes(extracted.memories()),
        answer.known.included,
        answer.known.dropped
    ));

    lines
}

/// **腕どうしのプロンプトが実際に違うことを、測定を始める前に確かめる**
/// （作業単位 21a-2 / 22a で表に一般化）。
///
/// # なぜ器具の側にも置くのか
///
/// `ExampleStyle::first_example_line` の分岐が潰れると、
/// この道具は**同一のプロンプトで腕を回す。** 出力の TSV は腕の名前だけが違う
/// 正常な形になり、**司令塔は「差が無かった」と読む**——結論が反転する。
///
/// 単体テスト（`prompt::extraction::tests::all_lists_every_example_style_exactly_once` /
/// `server::extract::tests::every_example_style_leaves_the_same_room_for_a_conversation`）は
/// 今日その変異を赤にする。**それでも足りない**——この道具の本体は `#[ignore]` なので、
/// **実機で回す当日に何かが崩れていても誰も見ていない。**
/// **素の `cargo test` で回る対照は [`the_arm_table_really_produces_distinct_arms`]。**
///
/// **落とすのは測定を1回も行う前。** 途中で気付いても、
/// **黙って回り続けるのがいちばん悪い**（`16k` の測定を2腕ぶん破棄した形）。
///
/// # **返り値が腕の表**（作業単位 22a）
///
/// **本体はこの関数が返した表を回す。[`ARMS`] を直接は見ない。**
/// 「呼び出しを1行消す」だけの書き換えは、**回すものが無くなってコンパイルが止まる。**
/// 21a-2 の形（`assert` を呼ぶだけ）は、**その1行を消すと何も鳴らずに
/// 検査だけが消える**——実際に対照として測って緑だった。
/// **見張り番は消せる。消したら止まる形にしておく方が強い。**
///
/// > **⚠ 「コンパイルが止まる」のは、変異が呼び出し1箇所だけのときに限る**（22a-2 で訂正）。
/// > **呼び出しを消して `&ARMS` を直に回す**ように**2箇所を同時に**書き換えれば、
/// > コンパイルは通り、この関数の中の `assert` は誰にも呼ばれなくなる。
/// > **それでも実害は無い**——[`the_arm_table_really_produces_distinct_arms`] が
/// > **素の `cargo test` で、同じ判定を植えた台本に対して実行している**からで、
/// > 判定は引数だけに依存する純粋関数なので**実台本と植えた台本で結果が分かれる状態は作れない**
/// > （CLAUDE.md の第4の分類＝他の見張り番にマスクされた変異）。
/// > **止まる形にしてあるのは「消し忘れ」を防ぐためで、権威は素の `cargo test` 側にある。**
///
/// # 3つを見る（**表に一般化したので、どれも表の全ペアに掛かる**）
///
/// 1. **どの腕も依頼が空でない。** 空の依頼を投げる腕は、TSV に1行も出ないまま
///    「差が無かった」と読まれる。
/// 2. **導出後の腕名が全ペアで違う**（[`arm_name`]）。同じ綴りが並ぶと
///    **TSV の上で2つの腕が1つに混ざる**（集計は `arm` 欄で振り分ける）。
/// 3. **プロンプトが全ペアで違う。** 同じものを2回測ることになる。
///
/// > **⚠ B-11 で「腕が2つ以上ある」が落ちた**（ADR-0037 §決定4 で対の綴りが全部消えたため）。
/// > **表が1行のあいだ、2 と 3 は二重ループが 0 回で空回りする。**
/// > **いま実際に鳴っているのは 1 だけ。**
/// > **腕を比べる実験に戻して行を足せば、その日から 2 と 3 は元の強さに戻る。**
/// > **「表が空でないこと」は実行時ではなく [`ARMS`] の直下の `const _` が持つ**
/// > ——ここに実行時の `assert!` として置いても、[`ARMS`] がコンパイル時定数なので
/// > **発火させる状態が存在しない**（B-11 の変異検査 M11 で実測: 1579 全緑）。
///
/// # 覚書に腕ごとの枠を出す
///
/// [`request_tokens`] は本番の §8 の門とまったく同じ関数。
/// **枠が腕ごとにどれだけ違うか**は、長い往復が片方の腕でだけ捨てられていないかを
/// 後から確かめる唯一の材料になる（`SKIP too_long=` の行と突き合わせる）。
/// **表の全腕ぶん出す。**
fn assert_the_arms_differ(
    display_name: &DisplayName,
    known: &[Memory],
    turn: &Turn,
) -> &'static [ExampleStyle] {
    let mut prompts: Vec<(String, String)> = Vec::new();

    for example_style in ARMS {
        // **本体が実際に送るのとまったく同じ組み立て**（[`build_arm`]）。
        // 名前も依頼もここから出るので、**確かめた依頼と送る依頼はずれない。**
        let (arm, prompt) = build_arm(display_name, known, turn, example_style);
        let messages = prompt.messages();
        let tokens = request_tokens(messages);
        println!("# 枠 = {arm}: {tokens} トークン（ターン {}）", turn.turn);

        let mut whole = String::new();
        for message in messages {
            whole.push_str(&message.content);
            whole.push('\n');
        }
        assert!(
            !whole.trim().is_empty(),
            "依頼が空の腕がある（TSV に1行も出ないまま「差が無かった」と読まれる）: {arm}"
        );
        prompts.push((arm, whole));
    }

    for (index, (left_arm, left)) in prompts.iter().enumerate() {
        for (right_arm, right) in prompts.iter().skip(index + 1) {
            assert_ne!(
                left_arm, right_arm,
                "導出後の腕名が同じ（集計の TSV で2つの腕が1つに混ざる。\
                 表に同じ綴りを2行並べていないか見ること）"
            );
            assert_ne!(
                left, right,
                "腕どうしのプロンプトが同一（同じものを2回測ることになる。\
                 ExampleStyle の分岐が潰れていないか、\
                 表に同じ腕を2つ並べていないか見ること）: {left_arm} / {right_arm}"
            );
        }
    }

    &ARMS
}

/// **[`ARMS`] の全腕を、同じプロセスの中で交互に回す**（作業単位 21a / 22a）。
///
/// 並びは `ターン1: 腕1 → ターン1: 腕2 → … → ターン2: 腕1 → …`。
/// **並行させないこと**——腕どうしが GPU を取り合うと、
/// 腕どうしを比べるための測定が取り合いで汚れる（`16k` で実測 / CLAUDE.md）。
#[tokio::test]
#[ignore = "LM Studio の起動と台本のパスが要る。手順はこのファイルの doc を参照"]
async fn every_arm_runs_alternately_over_the_transcript() {
    // **環境変数が無いときは panic しない。** `--ignored` で回したときに
    // 「LM Studio が落ちている」と誤診させないため、理由を出して早期 return する。
    let (Some(transcript_path), Some(known_path)) = (env(ENV_TRANSCRIPT), env(ENV_KNOWN)) else {
        println!(
            "# 走らせるには台本と既知の記憶のパスが要る: {ENV_TRANSCRIPT} / {ENV_KNOWN}\n\
             # 形式はこのファイルの doc を参照（どちらも JSONL）。**LM Studio の状態とは無関係。**"
        );
        return;
    };

    let reps = env(ENV_REPS)
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);

    let transcript = read_transcript(&PathBuf::from(transcript_path));
    let known = read_known(&PathBuf::from(known_path));
    let display_name = DisplayName::parse(DISPLAY_NAME).expect("正当な表示名であること");

    println!("# arm\tturn\trep\tkind\tvalue");
    println!(
        "# ターン数 = {} / 既知の記憶 = {} 件 / 繰り返し = {reps} / 予算 = {} トークン",
        transcript.len(),
        known.len(),
        extraction_prompt_budget_for_tests()
    );
    // **腕名と綴りの対応を1行ずつ出す**（22a-2）。名前は導出なので表には無い——
    // **後から TSV だけを見た人が、どの腕がどの綴りだったかを引けるようにする。**
    for example in ARMS {
        println!("# 腕 = {} ← ExampleStyle::{example:?}", arm_name(example));
    }

    // **1ターン目で腕どうしを突き合わせる。同じなら即座に落ちる**（21a-2）。
    // **上流を作る前**に見るので、崩れていたら1回も測らずに止まる。
    let Some(first_turn) = transcript.first() else {
        println!("# 台本が空。回すものが無い。");
        return;
    };
    // **回すのはこの返り値。`ARMS` を直接見ない**（22a / この関数の doc）——
    // 呼び出しを消すと回すものが無くなり、コンパイルが止まる。
    let arms = assert_the_arms_differ(&display_name, &known, first_turn);

    let client = sub_client();

    for turn in &transcript {
        for rep in 1..=reps {
            // **腕は必ず交互。** まとめて片方を回すと、上流の状態
            //（JIT ロード・キャッシュ）の違いが腕の差に化ける。
            for arm in arms {
                run_once(&client, &display_name, &known, turn, rep, *arm).await;
            }
        }
    }
}

/// **[`ARMS`] の表が、実際に別々の腕を作る**（作業単位 22a）。
///
/// # ここが塞いでいる穴
///
/// [`assert_the_arms_differ`] は**本体（`#[ignore]`）からしか呼ばれない。**
/// つまり**表を壊しても素の `cargo test` は何も言わない**——
/// 気付くのは実機を回す当日で、しかも
/// **腕が1つに潰れていても TSV は正常な形で出る**（21a-2 と同じ形）。
///
/// **植えた台本を、器具の判定そのものに通す。**
/// 「実ファイルを読む対照は自己成就しうる」（CLAUDE.md / `15b`）の裏返しで、
/// **判定本体を引数で受ける形にしてあるから、テストが用意した入力を同じ判定に通せる。**
///
/// # ⚠ **いまこのテストが実際に見ているのは「依頼が空でない」だけ**（作業単位 B-11）
///
/// **[`ARMS`] が1行なので、[`assert_the_arms_differ`] の
/// 「腕名が全ペアで違う」「プロンプトが全ペアで違う」は二重ループが 0 回で空回りする。**
/// **上の「ここが塞いでいる穴」は、表が2行以上に戻った日から再び成立する。**
/// **このテストだけを読んで「別物であることが守られている」と読まないこと。**
///
/// # 上流にも台本にも触らない
///
/// [`build_extraction_prompt`] は純粋関数で、
/// [`extraction_prompt_budget_for_tests`] も定数を返すだけ。
/// **LM Studio の状態とは無関係**なので `#[ignore]` を付けないこと。
#[test]
fn the_arm_table_really_produces_distinct_arms() {
    let display_name = DisplayName::parse(DISPLAY_NAME).expect("正当な表示名であること");
    // **利用者側とキャラクター側を別の文字列にする**（取り違えても緑になる）。
    let turn = Turn {
        turn: 1,
        user: "犬を二匹飼っているんだ".to_owned(),
        character: "「そうなんだ」と葵は笑った".to_owned(),
    };

    // **記憶ありの形でも通す**——記憶の節が入っても腕どうしは別物のまま。
    assert_the_arms_differ(&display_name, &[], &turn);
    assert_the_arms_differ(&display_name, &planted_known(), &turn);
}

/// **植えた既知の記憶。主語ごとに1件ずつ、別の本文**（作業単位 A-1a）。
///
/// **片方の主語しか無いと、控除リストの絞り込み（ADR-0034 §決定4）が観測できない**
/// ——反対の主語の記憶が「載っていない」ことを、載る記憶が無いから言えなくなる。
fn planted_known() -> Vec<Memory> {
    [
        (Subject::User, USER_FACT),
        (Subject::Character, CHARACTER_FACT),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (subject, content))| {
        let index = i64::try_from(index).expect("件数が i64 に収まること");
        Memory {
            id: MemoryId::from_raw_for_tests(index + 1),
            character_id: CharacterId::from_raw_for_tests(1),
            subject,
            content: MemoryContent::parse(content).expect("正当な記憶本文であること"),
            category: None,
            created_at: DateTime::from_timestamp(KNOWN_EPOCH_SECONDS + index, 0)
                .expect("正当な時刻"),
            locked: false,
        }
    })
    .collect()
}

/// 植えた**利用者の**記憶の本文。**[`CHARACTER_FACT`] と別の文字列にすること。**
const USER_FACT: &str = "海の近くで働いてみたい";

/// 植えた**キャラクターの**記憶の本文。
const CHARACTER_FACT: &str = "雨の日は店を早く閉める";

/// **2つの目印は `grep` で切り分けられる**（作業単位 21a-2）。
///
/// # ここが塞いでいる穴
///
/// 21a は「上流を呼ばなかった回」「上流が失敗した回」「毎ターンの内訳」の
/// **3つを同じ [`DISCARD_LABEL`] に出していた。**
/// **`grep DISCARD | wc -l` が成功したターンも数える**——
/// `16l` で実際に踏んだ形そのもの（集計の道具が本番と別の受理集合を持ち、
/// 5つの測定を数え直した）。
///
/// 分けたことを doc に書くだけでは、**片方を書き換えたときに何も鳴らない。**
/// **この道具の本体は `#[ignore]`** なので、目印を見ているのは素の `cargo test` では
/// この1本だけ（本体を回す当日まで誰も見ない）。
///
/// # 「違うこと」では足りない
///
/// 集計は行を `grep` で拾う。**片方がもう片方を部分文字列として含むと、
/// 違う綴りのままでも切り分けが壊れる**（`DISCARD` と `DISCARDED` のような形）。
/// **どちらの向きにも含まないこと**まで見る。
#[test]
fn the_two_labels_can_be_told_apart_by_grep() {
    assert_ne!(
        DISCARD_LABEL, SKIP_LABEL,
        "捨てた回と内訳の目印が同じ（grep が成功したターンも数える / 16l）"
    );
    assert!(
        !DISCARD_LABEL.contains(SKIP_LABEL) && !SKIP_LABEL.contains(DISCARD_LABEL),
        "一方が他方の部分文字列（grep で切り分けられない）: \
         {DISCARD_LABEL} / {SKIP_LABEL}"
    );
}

/// **打ち切られた回は、最後の1行だけが観測から落ちる**（作業単位 24-2 / ADR-0021 §17）。
///
/// # ここが塞いでいる穴
///
/// [`observations`] を呼ぶのは [`run_once`] だけで、それは `#[ignore]` の本体からしか
/// 走らない。**器具が打ち切りの申告を捨てても、素の `cargo test` は何も言わなかった**
/// ——レビューで実測: `response_completeness_for_tests(completion.finish_reason)` の引数を
/// `None` に変える変異が **1501 全緑**。
/// **道具だけが甘くなり、断片を採った記憶を「モデルが出した記憶」として数える。**
/// `16l` の裏返し（あのときは道具だけが厳しすぎて、正しい回を「壊れている」と数えた）。
///
/// # 同じ生出力を、申告だけ変えて2回通す
///
/// **`length` と `stop` で結果が分かれること**を1本で見る。
/// 片方だけを見ると、**どちらかへの決め打ち**（常に落とす／常に落とさない）が通り抜ける。
///
/// # 欄そのものも見る
///
/// 司令塔の集計 script は `arm/turn/rep/kind/value` の5欄で振り分ける。
/// **欄が増減した状態は、実機を回す当日まで誰も見ない**ので、ここで固定する。
/// `known_included` / `known_dropped` は**違う数**にしてある（入れ替えが緑にならない）。
#[test]
fn a_truncated_completion_loses_only_its_last_line() {
    /// 最後の行が**断片**（「十年前から通っている」が切れたもの / ADR-0021 §17）。
    const RAW: &str = "- 利用者: 犬を二匹飼っている\n\
                       - あなた: 苦い珈琲が好き\n\
                       - 利用者: 十年前から通って";
    const ARM: &str = "腕A+腕B";
    const TURN: u32 = 7;
    const REP: u32 = 3;

    /// 生出力と停止理由から、上流の答えを組む。
    fn answered(finish_reason: Option<FinishReason>, known: KnownCounts) -> Answer {
        Answer {
            completion: Completion {
                content: RAW.to_owned(),
                finish_reason,
                usage: None,
            },
            known,
        }
    }

    /// 1行を欄に割って、`(kind, value)` を返す。**欄の数と前3欄もここで見る。**
    fn rows(lines: &[String]) -> Vec<(String, String)> {
        lines
            .iter()
            .map(|line| {
                let fields: Vec<&str> = line.split('\t').collect();
                assert_eq!(
                    fields.len(),
                    5,
                    "欄が5つでない（集計 script の列がずれる）: {line:?}"
                );
                assert_eq!(fields[0], ARM, "腕の欄: {line:?}");
                assert_eq!(fields[1], TURN.to_string(), "ターンの欄: {line:?}");
                assert_eq!(fields[2], REP.to_string(), "回の欄: {line:?}");
                (fields[3].to_owned(), fields[4].to_owned())
            })
            .collect()
    }

    /// 記憶の行だけを拾う（[`RAW_LABEL`] / [`DISCARD_LABEL`] を除く）。
    fn memories(rows: &[(String, String)]) -> Vec<(String, String)> {
        rows.iter()
            .filter(|(kind, _)| kind != RAW_LABEL && kind != DISCARD_LABEL)
            .cloned()
            .collect()
    }

    /// 目印の行の `value` 欄を1つ取り出す。**無ければ落とす。**
    fn value_of(rows: &[(String, String)], label: &str) -> String {
        rows.iter()
            .find(|(kind, _)| kind == label)
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| panic!("{label} の行があること: {rows:?}"))
    }

    // `キー=値` を**丸ごと1語**として探すのは共有の1本（`common::has_field`）。
    // **写しをここへ戻さないこと**（B-13 / ファイル先頭の `use` の doc）。

    let known = KnownCounts {
        included: 2,
        dropped: 1,
    };
    assert_ne!(
        known.included, known.dropped,
        "前提: 2つの件数を別の数にすること（入れ替えが緑になる）"
    );

    let truncated = rows(&observations(
        ARM,
        TURN,
        REP,
        &answered(Some(FinishReason::Length), known),
    ));
    let complete = rows(&observations(
        ARM,
        TURN,
        REP,
        &answered(Some(FinishReason::Stop), known),
    ));

    assert_eq!(
        memories(&truncated),
        vec![
            ("利用者".to_owned(), "犬を二匹飼っている".to_owned()),
            ("あなた".to_owned(), "苦い珈琲が好き".to_owned()),
        ],
        "打ち切られた回は最後の1行だけを落とすこと（前の行は完結している）"
    );
    assert_eq!(
        memories(&complete),
        vec![
            ("利用者".to_owned(), "犬を二匹飼っている".to_owned()),
            ("あなた".to_owned(), "苦い珈琲が好き".to_owned()),
            ("利用者".to_owned(), "十年前から通って".to_owned()),
        ],
        "書き終えた回は1行も落とさないこと（正しい事実を毎回1件捨てることになる）"
    );

    let truncated_discard = value_of(&truncated, DISCARD_LABEL);
    assert!(
        has_field(&truncated_discard, "truncated=1") && has_field(&truncated_discard, "parsed=2"),
        "落とした1行を truncated に数えること: {truncated_discard:?}"
    );
    let complete_discard = value_of(&complete, DISCARD_LABEL);
    assert!(
        has_field(&complete_discard, "truncated=0") && has_field(&complete_discard, "parsed=3"),
        "書き終えた回は truncated に数えないこと: {complete_discard:?}"
    );

    for discard in [&truncated_discard, &complete_discard] {
        assert!(
            has_field(discard, "known_included=2") && has_field(discard, "known_dropped=1"),
            "控除リストの件数が、渡した組のまま載ること: {discard:?}"
        );
    }

    // **断片は捨てても、生出力には残る**（唯一の調査手段 / モジュールの doc）。
    for observed in [&truncated, &complete] {
        assert!(
            value_of(observed, RAW_LABEL).contains("十年前から通って"),
            "落とした断片も RAW の行には残ること: {observed:?}"
        );
    }
}

/// **正規化の数と手本の写しは、それぞれ自分の欄に乗る**（作業単位 B-28 / B-29）。
///
/// # ここが塞いでいる穴（レビューの実測 / 2026-09-02）
///
/// **B-29 が `quotes_stripped` を足した時点で、器具の欄には見張り番が1本も無かった**
/// ——`extracted.quotes_stripped()` を `extracted.emphasis_stripped()` に変える取り違えが
/// **1811 全緑**を通った。**`emphasis_stripped` も同じく無防備で、
/// B-29 はその穴を新しい欄に引き継いだ形。**
///
/// **⚠ 重いのは、これが「効いたかどうかを測る唯一の欄」だから。**
/// [`observations`] を呼ぶのは [`run_once`] だけで、それは `#[ignore]` の本体からしか走らない
/// ——**実機を回す当日に別の数が載っていても、緑のままで誰も気付けない。**
/// [`a_truncated_completion_loses_only_its_last_line`] の doc が
/// 「**欄が増減した状態は、実機を回す当日まで誰も見ない**ので、ここで固定する」と
/// 書いていたのに、**足した欄がその約束の外に落ちていた。**
///
/// # **5つとも違う数にしてある**
///
/// | 欄 | 数 | その数を作っている行 |
/// |---|---|---|
/// | `parsed` | **4** | 4行とも `- 主語: 本文` の形を満たす |
/// | `quotes_stripped` | **1** | 1行目だけが `> ` で始まる |
/// | `emphasis_stripped` | **2** | 2行目と3行目に `**` がある |
/// | `example_echoes` | **3** | 4行目に手本の本文が3回（`苦い珈琲` 2 ＋ `犬を二匹` 1） |
/// | `example_format_echoes` | **5** | 各行の末尾に1つずつ ＋ 4行目の途中と末尾で2つ |
///
/// **どれか2つが同じ数だと、その2つの取り違えは緑のまま**
/// （[`a_truncated_completion_loses_only_its_last_line`] が
/// `known_included=2` / `known_dropped=1` でやっているのと同じ作法）。
/// **`RAW` に `>` も `**` も無い腕では、両方 0 で同値になって何も言わない。**
///
/// > **⚠ `example_format_echoes` が `parsed` を超える数になっているのは意図的**
/// > （作業単位 B-33）。**行数ではなく回数**を数えるので、
/// > **1件の本文に2つ残っていれば 2** になる。4行目がその形。
/// > **`parsed` と同じ 4 にすると、行数へ潰す実装と区別が付かない。**
#[test]
fn the_normalisation_counts_ride_in_their_own_fields() {
    /// 引用記号1行・強調2行・手本の写し3回・手本の書式の居座り5回。
    ///
    /// **4行目の先頭の `[好み]` は分類として読まれる**ので、書式の数には入らない
    /// ——**入れてしまう実装は、この 5 を 6 にする。**
    const RAW: &str = "> - あなた: 平日は8時に開店する [営業]\n\
                       - 利用者: **犬**を二匹飼っている [家族]\n\
                       - あなた: **朝**が弱い [体質]\n\
                       - 利用者: [好み] 苦い珈琲が好き と [好み] 苦い珈琲が好き と 犬を二匹飼っている [好み]";
    const ARM: &str = "腕A+腕B";
    const TURN: u32 = 9;
    const REP: u32 = 2;

    let answer = Answer {
        completion: Completion {
            content: RAW.to_owned(),
            // **打ち切りではない**——断片として最後の1行が落ちると数が動く。
            finish_reason: Some(FinishReason::Stop),
            usage: None,
        },
        known: KnownCounts {
            included: 2,
            dropped: 1,
        },
    };

    let lines = observations(ARM, TURN, REP, &answer);
    let discard = lines
        .iter()
        .find_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            (fields.len() == 5 && fields[3] == DISCARD_LABEL).then(|| fields[4].to_owned())
        })
        .unwrap_or_else(|| panic!("{DISCARD_LABEL} の行があること: {lines:?}"));

    // **5つとも違う数**（前提が崩れたら、取り違えが緑になる）。
    let expected = [
        ("parsed", 4),
        ("quotes_stripped", 1),
        ("emphasis_stripped", 2),
        ("example_echoes", 3),
        ("example_format_echoes", 5),
    ];
    let mut seen: Vec<u32> = expected.iter().map(|(_, count)| *count).collect();
    seen.sort_unstable();
    let unique = {
        let mut unique = seen.clone();
        unique.dedup();
        unique
    };
    assert_eq!(
        seen, unique,
        "前提: 5つの数を別の値にすること（同値だと入れ替えが緑になる）"
    );

    for (field, count) in expected {
        assert!(
            has_field(&discard, &format!("{field}={count}")),
            "{field} が {count} で載っていない（欄の取り違え・落とし・二重計上）: {discard:?}"
        );
    }

    // **捨てていない。** 手本を写した行も引用記号の行も、記憶として出ている。
    let memories: Vec<String> = lines
        .iter()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            (fields.len() == 5 && fields[3] != RAW_LABEL && fields[3] != DISCARD_LABEL)
                .then(|| fields[4].to_owned())
        })
        .collect();
    assert_eq!(memories.len(), 4, "記憶の行が4本出ていない: {lines:?}");
}

/// **腕名は、自分の綴りを名乗る**（作業単位 22a-2）。
///
/// # ここが塞いでいる穴
///
/// 22a は腕名を [`ARMS`] の手書きの第1要素から取っていたので、
/// **表示名2つを入れ替えるだけの変異が 1488 全緑**だった（レビューで実測）。
/// **TSV の欄も目印も正常、腕名も2種類、プロンプトも別物**で、
/// **司令塔が読む結論だけが左右反転する。**
///
/// 22a-2 は名前を表から落として [`arm_name`] の導出に一本化したが、
/// **導出そのものが嘘をつく形は残る**——`arm_name` の中で綴りを差し替えれば、
/// 「別々の名前が2つある」も「表の組が2つある」も成立したまま反転する。
/// **[`assert_the_arms_differ`] は名前どうしが違うことしか見ないので、
/// この形には掛からない**（CLAUDE.md「抜けるのはいつも帰属」）。
///
/// **だから「違う」ではなく「どれのものか」を見る**——
/// 腕名が**自分の綴りを名乗っている**ことを [`ARMS`] の外まで、
/// [`ExampleStyle::ALL`] の全綴りで確かめる。
/// **表に載っていない綴り（`CharacterOnly`）でも導出が壊れていないこと**まで見るので、
/// 表を書き換えた日に初めて壊れる形にならない。
///
/// > **⚠ 22a-2 は `ClosingStyle::ALL` × `ExampleStyle::ALL` × `ArmAsks::ALL` の
/// > 全組を回していた。** B-11 で前2つの型を落としたので、
/// > **いま回っているのは [`ExampleStyle`] の1次元だけ**（ADR-0037 §決定4）。
///
/// # 寄りかかっている前提
///
/// **`Debug` の綴り**（[`ARMS`] の doc）。**`Debug` を手で実装して嘘の綴りを返せば、
/// 期待値もその嘘を向くのでこの検査は緑のまま**——ここは型が守っていない。
#[test]
fn the_arm_name_carries_its_own_spellings() {
    let mut names: Vec<String> = Vec::new();

    for example in ExampleStyle::ALL {
        let name = arm_name(example);
        assert!(
            name.contains(&format!("{example:?}")),
            "腕名が手本の綴りを名乗っていない（別の腕の名前が付く / 帰属の取り違え）: \
             {name} / {example:?}"
        );
        names.push(name);
    }

    // **全組が別の名前になる。** 綴りを名前から落とすと、
    // **表に無い組まで含めて**ここで潰れる（`Balanced` と `CharacterOnly`）。
    for (index, left) in names.iter().enumerate() {
        for right in names.iter().skip(index + 1) {
            assert_ne!(
                left, right,
                "別の綴りが同じ腕名になった（TSV の上で腕が混ざる）: {names:?}"
            );
        }
    }
}

/// TSV の1行を欄に割る。**欄が5つであることもここで見る。**
///
/// 集計 script は `arm/turn/rep/kind/value` の5欄で振り分けるので、
/// **欄が増減した状態は実機を回す当日まで誰も見ない。**
fn split_row(line: &str) -> (String, String) {
    let fields: Vec<&str> = line.split('\t').collect();
    assert_eq!(
        fields.len(),
        5,
        "欄が5つでない（集計 script の列がずれる）: {line:?}"
    );
    (fields[3].to_owned(), fields[4].to_owned())
}

/// 記憶の行（`kind` が主語の語）だけを `(主語, 本文)` で拾う。
fn memory_rows(lines: &[String]) -> Vec<(String, String)> {
    lines
        .iter()
        .map(|line| split_row(line))
        .filter(|(kind, _)| kind != RAW_LABEL && kind != DISCARD_LABEL)
        .collect()
}

/// [`DISCARD_LABEL`] の行の `value` 欄を、出た順に拾う。
fn discard_values(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|line| split_row(line))
        .filter(|(kind, _)| kind == DISCARD_LABEL)
        .map(|(_, value)| value)
        .collect()
}

// `キー=値` を**丸ごと1語**として探すのは共有の1本（`common::has_field`）。
// ここには `field_is_present` という**別名の写し**が居た（本体は共有版と同一）。
// **別名は走査に捕まらない**ので、B-13 で名前ごと畳んだ。

/// **1ターンの行は `RAW` → 記憶 → `DISCARD` の順に並ぶ**
/// （作業単位 A-1a / **B-11 で1組ぶんに縮めた**）。
///
/// # ここが塞いでいる穴
///
/// **`RAW` の行を `DISCARD` の後ろへ移す変異。**
/// `RAW` の行には目印以外の欄が無く、**どのターンの生出力かは並びだけが決めている。**
/// 司令塔は 22b と同じ形で「利用者の行を 50 行読む」ので、
/// **どの応答から出た行かを追えないと読めない。**
///
/// # ⚠ **A-1a の版より弱い。承知のうえ**
///
/// A-1a は `every_ask_gets_its_own_group_of_rows_in_the_order_it_was_sent` として
/// **2本の依頼を渡し、`ask=` 欄の帰属**（内訳の行が1本目の依頼を名乗る変異）も
/// 一緒に見ていた。**B-11 で依頼が1本になったので、その主張は消えた**
/// （名乗るべき相手が居ない / ADR-0037 §決定4）。
/// **残したのは「並びが帰属を決めている」ほうだけ。**
///
/// **依頼を複数投げる形へ戻す日には、あの版も一緒に戻すこと**
/// （`git` の履歴にある）。
///
/// # フィクスチャは全部の欄を食い違わせる
///
/// 生出力・件数・控除リストの数——**どれか1つでも揃えると、
/// その欄の取り違えが観測できなくなる。**
#[test]
fn the_rows_come_out_as_raw_then_memories_then_discard() {
    const ARM: &str = "腕A";
    const TURN: u32 = 4;
    const REP: u32 = 2;
    /// **2件返す**（記憶の行が1本だけだと「並び」が観測しにくい）。
    const RAW: &str = "- 利用者: 犬を二匹飼っている\n- あなた: 苦い珈琲が好き";

    let answer = Answer {
        completion: Completion {
            content: RAW.to_owned(),
            finish_reason: Some(FinishReason::Stop),
            usage: None,
        },
        known: KnownCounts {
            included: 3,
            dropped: 1,
        },
    };
    // **前提: 控除リストの2つの数が違う**（入れ替えが緑にならない）。
    assert_ne!(
        answer.known.included, answer.known.dropped,
        "前提: 控除リストの件数を別の数にすること"
    );

    let lines = observations(ARM, TURN, REP, &answer);
    let rows: Vec<(String, String)> = lines.iter().map(|line| split_row(line)).collect();
    let kinds: Vec<&str> = rows.iter().map(|(kind, _)| kind.as_str()).collect();

    // **組の形と並び。** `RAW` → 記憶 → `DISCARD`。
    // **`RAW` を `DISCARD` の後ろへ移すと、ここで落ちる。**
    assert_eq!(
        kinds,
        vec![
            RAW_LABEL,
            Subject::User.extraction_word(),
            Subject::Character.extraction_word(),
            DISCARD_LABEL,
        ],
        "1ターンの行が `RAW` → 記憶 → `DISCARD` で並んでいない: {rows:?}"
    );

    // **記憶の行は、生出力の順のまま。**
    assert_eq!(
        memory_rows(&lines),
        vec![
            ("利用者".to_owned(), "犬を二匹飼っている".to_owned()),
            ("あなた".to_owned(), "苦い珈琲が好き".to_owned()),
        ],
        "記憶の行が生出力の順に並んでいない: {rows:?}"
    );

    // **生出力は `RAW` の行にだけ載る。**
    assert!(
        rows[0].1.contains("犬を二匹飼っている"),
        "生出力が RAW の行に載っていない: {:?}",
        rows[0]
    );

    // **内訳の行は自分の件数を名乗る。**
    let discards = discard_values(&lines);
    assert_eq!(discards.len(), 1, "内訳の行は1ターンに1本: {lines:?}");
    for field in ["parsed=2", "known_included=3", "known_dropped=1"] {
        assert!(
            has_field(&discards[0], field),
            "内訳の行が自分の数を名乗っていない: 欲しい欄 = {field}, {:?}",
            discards[0]
        );
    }
}
