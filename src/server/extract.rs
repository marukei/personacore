//! **記憶抽出を走らせて保存する**（ADR-0021 §6〜10 / 作業単位 13b）。
//!
//! 13a の純粋関数（`prompt::extraction` / `parse::extraction`）に、
//! **いつ・どう走らせるか**を与える層。ここだけが I/O を持つ。
//!
//! ```text
//! TurnSink::persist（保存に成功した直後）
//!   → Extraction::start          … 応答の経路。**やるのは spawn だけ**
//!   → tokio::spawn
//!        Store::recall_memories    … **すでに知っていること**（§13）。**上流を呼ぶ前**
//!        prompt::extraction::build_extraction_prompt … 依頼を組む（純粋）
//!        （長すぎたら捨てて数える / ADR-0021 §8）
//!        LlmClient::complete       … **サブモデル**（§10）。トランザクションの外
//!        parse::extraction::parse_extraction … 分解（純粋。panic しない）
//!        Store::add_memory × n     … 追加のみ（`memory` に更新も削除も無い）
//!        1イベント記録             … 採った件数・理由ごとの破棄数・
//!                                     強調記号を落とした行数（ADR-0021 §11）・
//!                                     載せた記憶と入らなかった記憶の件数（§13）
//! ```
//!
//! # 記憶の読み出しは**上流を呼ぶ前**（作業単位 13c-3b）
//!
//! 読み出しは `Store::recall_memories` ——**想起の入口はこれひとつ**（ADR-0003）。
//! ここに別の読み出し関数を生やさない。
//!
//! **トランザクションを跨がない。** 読み出しは1クエリで完結し、
//! `LlmClient::complete` の `await` はその外側にある（絶対制約）。
//!
//! # 応答の経路に待ちを足さない（CLAUDE.md 絶対制約 / ADR-0021 §6）
//!
//! [`Extraction::start`] は **`async` ですらない。** 呼ばれた側で行うのは
//! 「走っている抽出があるかを見て、無ければ `tokio::spawn` する」だけで、
//! **プロンプトの組立も見積もりもタスクの中**にある。
//! 応答の経路で見積もりを回すと、会話が長い回ほど応答が遅れる
//! ——**抽出のための遅延を利用者が払う**という、この設計が避けたいものそのもの。
//!
//! # 同時に走らせるのは1件まで（§7）
//!
//! 走っていたら**捨てて数える。待ち行列にしない。** サブモデルは本文生成と
//! 同じ GPU を取り合うので、積み上がると本文の応答が遅くなる。
//!
//! # 停止時は落ちてよい（§9）
//!
//! [`MemoryExtractor::shutdown`] が走っている1件を `abort` する。
//! **タスクを leak させない**のがここの役目で、`server::serve` が停止処理の一部として呼ぶ。
//!
//! # LLM 出力の扱い
//!
//! サブLLM の出力は**外部からの信頼できない入力**（ADR-0021 §5）。
//! ここから先へ渡す経路は `parse::extraction`（純粋関数）と
//! `Store::add_memory`（プレースホルダ束縛）だけで、**SQL・シェル・パスには渡らない。**
//! **ログにも本文を載せない**——載せるのは件数と定数ラベルだけで、
//! 調査材料は `message.raw_content` の側に残っている。

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::task::JoinHandle;

use crate::domain::{
    AxisDiscoveryMemories, AxisDiscoverySlots, AxisTrialRule, Character, CharacterId, DisplayName,
    DriftForm, DriftNote, DriftTuning, MessageId, ResolvedDriftMode, SessionId, StatDriftMode,
    StatScale, strip_leaked_thinking,
};
use crate::llm::{ChatMessage, ChatRequest, FinishReason, LlmClient};
use crate::parse::extraction::{Completeness, DiscardCounts, parse_extraction};
use crate::prompt::extraction::{
    ExampleStyle, ExtractionInput, ExtractionPrompt, build_extraction_prompt, example_echoes,
    example_format_echoes, request_tokens,
};
use crate::store::{Store, StoreError};

/// **サブモデルの実コンテキスト長を、まだ一度も引けていないときの落とし所**
/// （`loaded_context_length` / トークン / 作業単位 B-7）。
///
/// 既定のサブモデルは本文と同じ 14B（ADR-0021 §10 / 実機の測定）。
/// この 16384 は LM Studio の `/api/v0/models` で実測した値（2026-08-21）。
/// **`max_context_length`（262144）ではなく、ロード時に決まる方**を見る——
/// あちらはモデルが理論上受けられる長さで、**実際に載っている窓ではない。**
///
/// # ⚠ **これは「仮定」ではなくなった**（作業単位 B-7 / [ADR-0036](../../docs/adr/0036-the-range-of-machines-is-a-constraint.md) 決定2 の第1階層）
///
/// **稼働中の上限は上流に引いた値から決まる**（[`SubContext`] / `server::context`）。
/// **ここに残っているのは「引けなかったとき」に使う値だけ:**
///
/// * 起動時の照会に失敗した（上流が停止している / `/api/v0/models` を持たない上流）。
/// * モック注入の入口（`App::bind_with_clients`）——**引く先が無い。**
///
/// **一度でも引けた後は、この値には戻らない**（引き直しに失敗しても現在値を保つ /
/// ADR-0020 §5）。以前ここに書いていた
/// 「`PERSONACORE_SUB_MODEL` を変えるとこの前提が崩れる」は**もう当てはまらない**
/// ——上限はサブモデルの申告値に付いてくる。
///
/// **⚠ ただし変容側（`super::drift` の `MAX_DRIFT_PROMPT_TOKENS`）は今もこの定数から
/// 導いている。** あちらは引き直しの対象に入っていない（B-7 の作業単位の外）。
///
/// # なぜ「定数どうしの同語反復」に見えるものを置くのか
///
/// [`FALLBACK_EXTRACTION_PROMPT_TOKENS`] は**この値の半分**という規則で決まる。
/// **その規則も 16384 という測定値も、以前は doc の散文にしかなかった。**
/// 結果、上限を `262_144`——**doc が「`max_context_length` ではなく」と
/// 名指しで警告している値そのもの**——に書き換える変異が
/// 1055 passed / 0 failed で緑を通った（実測 / 2026-08-21）。
///
/// ここに置く目的は2つ。
///
/// 1. **doc が書いた規則を実行可能にする。** 上限はこの値からの導出になり、
///    見張り番 `the_fallback_prompt_limit_is_half_of_the_sub_model_context` が
///    2つの関係と2つの実測値を固定する。
/// 2. **入力（16384）を直すべき場所をコードに与える。**
///    落とし所を測り直したときに直すのは上限ではなく**この定数**で、上限は付いてくる。
pub(in crate::server) const ASSUMED_SUB_CONTEXT_TOKENS: u32 = 16_384;

/// **サブモデルの実コンテキスト長**（トークン）。**抽出の上限はここからしか出ない。**
///
/// # なぜ生の `u32` を持ち回さないのか
///
/// 稼働中に差し替わる値（`server::context::SharedSubContext`）なので、
/// **「コンテキスト長」と「依頼の上限」が両方 `u32` のまま関数の引数に並ぶ**と、
/// 取り違えても型が通る。**この2つは 2 倍違う**ので、取り違えは
/// 「窓を超えた依頼を送る」（§8 が避けたい形。LM Studio では前方から黙って切り捨てられる /
/// ADR-0013。llama は 400 `exceed_context_size_error` で断るか、プロンプト ＋ 応答の上限が
/// 窓を超えて応答が黙って短くなる / ADR-0052 M5）か「入るはずの会話を捨てる」に
/// 直結する。**片方を型にして、もう片方は [`prompt_limit`](Self::prompt_limit) 越しにしか
/// 出てこない形にしてある。**
///
/// # 予算にならない申告値は作らせない
///
/// [`new`](Self::new) は**依頼の上限と応答の上限が同時に収まらない値**を弾く。
/// 弾かれた値は採用されず、呼び出し側（`server::context`）が現在値を保つ
/// （ADR-0020 §5 と同じ作法）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::server) struct SubContext(u32);

impl SubContext {
    /// **まだ一度も引けていないときの落とし所**（[`ASSUMED_SUB_CONTEXT_TOKENS`]）。
    pub(in crate::server) const FALLBACK: Self = Self(ASSUMED_SUB_CONTEXT_TOKENS);

    /// 上流が申告した値から作る。**依頼と応答が同時に収まらない値は `None`。**
    ///
    /// 収まらない値をそのまま採ると、**依頼だけで窓を使い切って応答が入らない**
    /// ——LM Studio では前方から黙って切り捨てられるので、消えるのは指示の側（§8 / ADR-0013）。
    /// llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。ただし、
    /// プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
    pub(in crate::server) const fn new(tokens: u32) -> Option<Self> {
        let candidate = Self(tokens);
        if candidate
            .prompt_limit()
            .saturating_add(EXTRACTION_RESPONSE_TOKENS)
            <= tokens
        {
            Some(candidate)
        } else {
            None
        }
    }

    /// 上流が申告した実コンテキスト長。**ログにはこちらを出す**（上限だけだと
    /// 「半分にした後の数」なのか「上流がそう言った」のかが読めない）。
    pub(in crate::server) const fn tokens(self) -> u32 {
        self.0
    }

    /// 抽出の依頼全体に許す上限。**規則は「半分」**
    /// （理由は [`FALLBACK_EXTRACTION_PROMPT_TOKENS`] の doc）。
    ///
    /// **ここが規則の唯一の決定点。** 落とし所の側の定数もこの関数を通る。
    pub(in crate::server) const fn prompt_limit(self) -> u32 {
        self.0 / 2
    }
}

/// **落とし所が、依頼と応答を同時に収められることをビルドで確かめる。**
///
/// [`SubContext::new`] は上流の申告値を同じ規則で弾くのに、
/// **落とし所だけはその検査を通らずに使われる**（[`SubContext::FALLBACK`] は
/// 検査を経ない構築）。ここが破れると、**引けなかった環境でだけ
/// 「依頼で窓を使い切る」状態になる**——引ける環境では再現しない。
const _: () = assert!(
    SubContext::new(ASSUMED_SUB_CONTEXT_TOKENS).is_some(),
    "落とし所のコンテキスト長では、抽出の依頼と応答が同時に収まらない"
);

/// 抽出の依頼プロンプト全体に許す上限（トークン）。**超えたら捨てる。切り詰めない**（§8）。
///
/// # ⚠ **これは「引けなかったとき」の上限**（作業単位 B-7）
///
/// **稼働中の上限は [`SubContext::prompt_limit`] が返す値**で、
/// その入力（サブモデルの実コンテキスト長）は起動時と 5 分ごとに上流へ引く
/// （`server::context` / ADR-0020 と同じ器）。
/// **この定数が実際に使われるのは、一度も引けていない環境だけ**
/// （[`ASSUMED_SUB_CONTEXT_TOKENS`] の doc）。
///
/// **`prompt::extraction` の doc が言う「上限」もこの規則のこと**であって、
/// この定数の値そのものではない。
///
/// **⚠ 本番のコードはこの定数を通らない**（引けなかった回も
/// [`SubContext::FALLBACK`] の [`prompt_limit`](SubContext::prompt_limit) を通る）ので、
/// **テストとフィーチャの下にだけ置いてある。** 残してあるのは2つの理由:
///
/// 1. **規則を実行可能な形で押さえる見張り番のため**
///    （`the_fallback_prompt_limit_is_half_of_the_sub_model_context`）。
/// 2. **実機の器具**（`tests/extraction_quality_harness.rs`）が
///    予算と門に同じ数を渡すため（[`extraction_request_for_tests`]）。
///
/// # なぜこの規則なのか（ADR-0021 は数を決めていない）
///
/// **規則は「サブモデルの実コンテキストの半分」**で、落とし所の実コンテキストが
/// [`ASSUMED_SUB_CONTEXT_TOKENS`]（16384 / 実測）なので **8192**。
/// **数ではなく規則を式で書いてある**（理由はあちらの doc）。
/// 以前は**推測**（保守的な既定 8192 の半分 = 4096）だった。変わったのは入力の側だけ。
///
/// * **半分に切る理由**: 抽出は本文生成と**同じ GPU を取り合う**（§7）。
///   依頼が長いほど prefill が伸び、本文の応答が遅くなる。
/// * **応答の分**（`EXTRACTION_RESPONSE_TOKENS` = 320）を足しても 8512 で、
///   16384 に対して十分な余裕がある（この関係も見張り番が固定する）。
/// * **見積もりは上界**（`estimate_tokens` は UTF-8 バイト数を下回らない）なので、
///   この判定が「実際は入るのに捨てた」側へ倒れることはあっても、
///   **窓を超えた依頼を送る側へは倒れない**（超えると、LM Studio では前方から黙って
///   切り捨てられる / ADR-0013。llama は 400 で断るか、応答が黙って短くなる / ADR-0052 M5）。
///
/// **4096 では足りなかった。** 依頼の削れない枠は実測 2339 あり（13c-4 で 2185 から増えた）
/// （`the_fixed_part_of_the_request_leaves_room_for_a_conversation` / 2026-08-22）、
/// 4096 の半分（2048）を超えている。
///
/// # 判定の境界は**1バイト単位**で固定してある
///
/// `tokens > limit` の両側を
/// `a_request_exactly_at_the_limit_is_sent` /
/// `a_request_one_byte_over_the_limit_is_dropped` が押さえる。
/// **以前は「上限の3倍」と「約 3.6k」の2点しか無く、
/// 判定を `.saturating_mul(2)` に緩める変異が緑を通った**（実測 / 2026-08-21）。
/// 緩んだ実装は 16384 トークンまで送るので、**まさに §8 が避けたい形**に落ちる。
///
/// **`conversation_too_long` は実機で一度も出ていない**（4096 の時点で
/// 会話の側には効いていなかった）。破棄率のログにこれが並ぶようなら、
/// **その環境のサブモデルにこの規則が合っていない**というシグナルになる
/// （§8 の但し書き）。**上限は `limit_tokens` としてその行に載る。**
#[cfg(any(test, feature = "testing"))]
pub(in crate::server) const FALLBACK_EXTRACTION_PROMPT_TOKENS: u32 =
    SubContext::FALLBACK.prompt_limit();

/// **実機で観測した、抽出の応答の最大トークン数**（ADR-0021 §17 / 作業単位 24）。
///
/// 22b の 511 回のうち**最長の応答を上流へ投げ直して測った値**（2026-08-26）。
/// 分布と測り方は ADR-0021 §17 にある。**ここに書き写さない**
/// （同じ発見を2箇所に書くと、片方を直しても、もう片方が黙って古びる / CLAUDE.md）。
///
/// # なぜ定数として置くのか
///
/// [`EXTRACTION_RESPONSE_TOKENS`] は**この値から決まる**。
/// 実測値がコードに無いと、上限を動かす変異を止めるものが
/// **定数そのものから期待値を作るテスト**しか無くなり、**両側が一緒に動いて緑を通る**
/// （CLAUDE.md「16b で定数を倍にする変異が 1283 全緑」）。
/// 見張り番は `the_response_limit_sits_just_above_the_longest_measured_response`。
///
/// # ⚠ この値は既定のサブモデルと既定の依頼の下でしか意味を持たない
///
/// **測ったのは `ministral-3-14b-instruct-2512`（LM Studio）。**
/// **綴りを機械可読で持っているのは [`crate::measured`] の一覧**で、
/// **起動時にサブモデルの綴りと突き合わせて、ずれていたら報告する**
/// （ADR-0033 §決定1 / 作業単位 E-2）。**ここを変えるなら、あちらも一緒に直すこと**
/// （`every_measured_constant_is_where_the_registry_says_it_is` が食い違いで落ちる）。
///
/// 依頼の文面（`prompt::extraction`）を変えれば応答の長さも変わる。
/// **測り直したら、直すのは上限ではなくこの定数**で、上限は付いてくる
/// （[`ASSUMED_SUB_CONTEXT_TOKENS`] と同じ形）。
pub(in crate::server) const MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS: u32 = 246;

/// **行列の実測から出した、応答の上限の上端**（ADR-0021 §17 / 作業単位 24-2）。
///
/// 抽出は次のターンの本文生成と**同じ GPU を取り合う**（§7）ので、
/// 上限まで喋る回が1度あると、**後ろに並んだ本文の最初の中身が遅れる。**
/// **どれだけ遅れるかは実測してある**（ADR-0021 §17）。
/// **この数の出し方——何秒ぶんを何トークンと見なしたか——も §17 にある。
/// ここに書き写さない**（同じ発見を2箇所に書くと片方が黙って古びる / CLAUDE.md）。
///
/// # ⚠ **この数はこの機で測った速さから出ている。守られてはいない**
///
/// **別の機・別のモデル・別の量子化では意味を持たない。**
/// ここにあるのは「実測した速さから導いた数を、上限が超えないこと」だけである。
///
/// # ⚠ **守っているのは「壊れないこと」ではなく「待たされすぎないこと」**（作業単位 B-6）
///
/// ~~この上端が守っているのは、行列の後ろに並んだ本文が中継の打ち切りに掛からないこと~~
/// **→ 前提が変わった。**
///
/// **導出のもとは「`relay::IDLE_BUDGET` の 30 秒に対する余裕」だった**（ADR-0021 §17）。
/// **その 30 秒は打ち切りの権威をやめた**
/// （[ADR-0035](../../../docs/adr/0035-the-watchdog-reports-the-human-decides.md) §決定1）
/// ——中継はもう無通信では終わらず、**`warn` が出るだけ**になった。
///
/// **つまりこの上端を外したときに起きるのは「遅い」であって「壊れる」ではない**
/// （[ADR-0036](../../../docs/adr/0036-the-range-of-machines-is-a-constraint.md) §決定1 の
/// 判定の問い）。**行列が伸びれば、次のターンの本文が遅れる。それだけ。**
/// 遅い機に載せ替えても**ターンは死なない**——死ぬのは `llm` 層の天井
/// （既定 180 秒 / `PERSONACORE_LLM_IDLE_TIMEOUT`）を超えたときだけで、
/// **そちらは利用者が伸ばせる。**
///
/// **⚠ 数は動かしていない**（B-6 は意味だけを書き直した）。
/// **測り直しは C-2 の宿題**で、**固定費を切り分けて測る**必要がある
/// （いまの導出は固定費を 0 と見なした楽観側 / docs/06-roadmap.md B-5）。
///
/// **測り直したら、直すのはこの定数**（上限は付いてくる。
/// [`MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS`] と同じ形）。
const QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS: u32 = 368;

/// 抽出の応答に許すトークン数。
///
/// 1記憶1行で上限 5 件（`NewMemory::MAX_PER_TURN`）なので、
/// **本来は数百トークンで足りる。** それでも上限を送るのは、
/// 形式を無視して喋り続けるモデルが**本文生成の GPU を握り続ける**のを防ぐため。
///
/// # なぜこの数なのか（作業単位 24 / 24-2 / ADR-0021 §17）
///
/// **窓の中の1つ**でしかない。両端はどちらも実測から来ている:
///
/// * **下端**——実測の最大（[`MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS`] = 246）に 30% の余裕。
/// * **上端**——[`QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS`]（行列の実測から導いた数）。
///
/// **以前は 512**——上端の外だった。
/// 切る根拠は**行列**にある。抽出は次のターンの本文生成と**同じ GPU を取り合う**（§7）ので、
/// 上限まで喋る回が1度あるだけで、その後ろの本文が遅れる。
/// **分布も待ち時間の実測も ADR-0021 §17 にある**——
/// **ここに書き写さない**（同じ発見を2箇所に書くと、片方が黙って古びる / CLAUDE.md）。
///
/// **⚠ 「行列が詰まらないことが守られている」とは読まないこと。**
/// 上端は**この機で測った速さから出た数**である。
/// **外したときに起きるのは「次のターンの本文が遅れる」であって「壊れる」ではない**
/// （B-6 / [`QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS`] の doc）。
///
/// # ⚠ **これは「収まることを保証する」数ではない**
///
/// 型の側の最大は `MemoryContent::MAX_CHARS`(500) ＋ `MemoryCategory::MAX_CHARS`(50) を
/// `NewMemory::MAX_PER_TURN`(5) 行ぶんで**約 2800 文字 ≒ 2000 トークン超。**
/// **512 も 320 も、もともと型の最大には遠く届いていない。**
/// これは**「暴走を止める保険」の線を実測に寄せる**変更であって、
/// **足りることを保証する変更ではない。**
///
/// # 足りなかった回に何が起きるか
///
/// **最後の行は断片になる**ので、[`Completeness::Truncated`] として
/// **捨てて `truncated` に数える**（ADR-0021 §17 / 作業単位 24）。
/// **この欄が並び始めたら、この数が現実に合っていないというシグナル**になる
/// （`conversation_too_long` と同じ読み方）。
/// **以前は「崩れとして数えられて捨てられる」と書いてあったが、事実ではなかった**
/// ——`- 主語: 本文` の形を満たす断片は**そのまま保存されていた。**
pub(in crate::server) const EXTRACTION_RESPONSE_TOKENS: u32 = 320;

/// **上限が2つの実測が挟む窓の中に居ることを、コンパイル時に確かめる。**
///
/// # なぜテストではなくビルドで止めるのか
///
/// 唯一の振る舞いの検査（`a_conversation_within_the_limit_is_sent_whole`）は
/// **この定数そのもの**と送った値を比べているので、
/// **定数を動かす変異は両側が一緒に動いて緑を通る**
/// （CLAUDE.md「16b で定数を倍にする変異が 1283 全緑」）。
/// **押さえは、関数の外にある値でなければならない。**
///
/// * **下から**: 実測の最大（[`MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS`]）に 30% の余裕。
///   割ると、上限に張り付く回が増える
///   ——断片は捨てるので（§17）、**記憶が静かに減る。**
/// * **上から**: [`QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS`]。
///   **以前はここが「実測の最大の 2 倍未満」だった**が、
///   **その窓は行列の余裕を何も守っていなかった**——
///   **491（元の 512 の 96%）が窓の中に居た**（作業単位 24 のレビューで実測。
///   素の `cargo build` も 1501 テストも全部通った）。
///   **上端の理由が行列なら、上端は行列の実測から出すこと。**
///
/// # ⚠ **秒はここに書かない**
///
/// 機の速さに依存する事実を `const assert` に埋めると、
/// **別の機でビルドが通らなくなる。** 押さえるのは**トークン数の上端だけ**で、
/// 根拠（秒）は ADR-0021 §17 にある。
///
/// **同じ窓を `the_response_limit_sits_just_above_the_longest_measured_response` も見る。**
/// 重ねてあるのは、**片方（この行）を消す変更を、もう片方が止めるため。**
const _: () = assert!(
    EXTRACTION_RESPONSE_TOKENS >= MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS * 13 / 10
        && EXTRACTION_RESPONSE_TOKENS <= QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS
);

/// **窓が空になっていない**ことを、コンパイル時に確かめる（作業単位 24-2）。
///
/// 2つの実測はそれぞれ別の理由で動く（応答の長さ / 機の速さ）ので、
/// **片方を測り直した日に、窓が反転したり1点に潰れたりしうる。**
/// **1点に潰れた窓は窓ではなく決め打ち**なので、そのときは
/// 「窓で挟んでいる」という説明の方が先に嘘になる。**気付かせるために止める。**
const _: () = assert!(
    MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS * 13 / 10 < QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS,
    "応答の上限の窓が空か1点に潰れている（両端の実測を測り直したときは ADR-0021 §17 を見ること）"
);

/// 抽出を1回行ったことの記録に使う文言。**「走った」側。**
///
/// # なぜ `pub(in crate::server)` なのか（作業単位 B-21e のレビュー指摘 / 2026-09-02）
///
/// **[`SKIPPED_MESSAGE`] と接頭辞「記憶」を共有する。**
/// だから `server::drift` 側の `extraction_skip_record` は、
/// **鍵をその共有接頭辞まで緩めてもこちらの行を掴めてしまう。**
/// **実測: 鍵を `"記憶"` へ緩める変異は 1802 全緑だった**
/// ——押さえの植えたソースに**この行が1本も無かった**ため
/// （CLAUDE.md「フィクスチャがその分岐が効く状態に一度も入らない」）。
///
/// **綴りが見えないと、その囮の行を植えられない。**
/// [`SKIPPED_MESSAGE`] / [`NO_FINISH_REASON_MESSAGE`] が同じ理由で先に開いている。
pub(in crate::server) const RAN_MESSAGE: &str = "記憶を抽出しました";

/// 抽出を**行わなかった**ことの記録に使う文言。**「走らなかった」側。**
///
/// **「0件だった」と同じ行にしないこと**（ADR-0021 §7 の「率が見えれば」が壊れる）。
/// 0 件は正常な結果で、走らなかったのは別の現象。
///
/// # なぜ `pub(in crate::server)` なのか（作業単位 B-21d のレビュー指摘 / 2026-08-29）
///
/// **`server::drift` のテストが、抽出の飛ばし記録の欄を「前提」として読む。**
/// 綴りが見えないと**行を選ぶ鍵が無く、捕捉した塊のまま欄を見るしかない**
/// ——それは「その欄がどこかの行に載っている」だけの主張になる
/// （`crate::testlog` の `has_field` の doc）。
/// **[`NO_FINISH_REASON_MESSAGE`] が同じ理由で先に開いていた**ので、こちらも揃えた。
pub(in crate::server) const SKIPPED_MESSAGE: &str = "記憶の抽出を行いませんでした";

/// 上流が**停止理由を申告しなかった**ことの記録に使う文言（作業単位 24）。
///
/// **「走った」「走らなかった」のどちらとも別のイベント**にしてある
/// （[`record_missing_finish_reason`] の doc）。
///
/// # **変容側とは別の文言**（作業単位 B-1）
///
/// `server::drift` も同じことを記録するが、**文言は共有しない。**
/// 同じにすると、**どちらのサブLLM 呼び出しに停止理由が無かったのかがログから読めない**
/// ——2つは同じターンで順に走る。
/// 見張り番は `super::drift::tests::the_two_missing_finish_reason_messages_never_contain_each_other`
/// で、**そこから読めるようにここは `pub(in crate::server)`。**
pub(in crate::server) const NO_FINISH_REASON_MESSAGE: &str = "抽出の応答に停止理由が付いていません";

/// 抽出を走らせなかった理由。**集計キーは値ごとの定数**（入力を反射しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkipReason {
    /// 前の抽出がまだ走っている（§7）。**待ち行列にしない。**
    Busy,
    /// 会話が長すぎる（§8）。**切り詰めない。**
    TooLong,
    /// **すでに知っていることを読めなかった**（§13 / 作業単位 13c-3b）。
    ///
    /// 理由は [`run`] の doc（「知らないまま走らせない」）。
    MemoriesUnreadable,
}

impl SkipReason {
    /// 集計キー。
    const fn label(self) -> &'static str {
        match self {
            Self::Busy => "busy",
            Self::TooLong => "conversation_too_long",
            Self::MemoriesUnreadable => "memories_unreadable",
        }
    }
}

/// **1往復ぶんの裏方の材料。タスクへ丸ごと移す**ので、借りものを持たない。
///
/// # 抽出と変容で共有する（ADR-0024 §2 / 作業単位 16b）
///
/// 変容は**抽出の直後に、同じ1本の枠の中で直列に**走る（`super::drift`）。
/// 材料は同じ（呼び名・直前の1往復・持ち主）なので、
/// **2つの構造体に写さない**——写すと片方だけが古びる（CLAUDE.md）。
///
/// **記憶はここに載らない。** 抽出は自分でタスクの中から読み
/// （`run` の doc）、変容は**そもそも記憶を読まない**（ADR-0024 §2）。
#[derive(Debug, Clone)]
pub(in crate::server) struct TurnJob {
    /// 記憶の持ち主。**変容が動かす軸の持ち主でもある。**
    pub(in crate::server) character_id: CharacterId,
    /// プロンプトに載る呼び名（ADR-0011。**識別子ではない**）。
    pub(in crate::server) display_name: DisplayName,
    /// **そのキャラクターの尺度**（ADR-0015）。
    ///
    /// **`StatAlloc::value` が覚えている尺度ではない。** 値の側の尺度が
    /// 「そのキャラクターのものか」は型では守れない（`StatValue` の doc）ので、
    /// 依頼に載せる上限は**キャラクターの行から来た方**を使う。
    ///
    /// **往復ごとに読み直したものが入る**（`AppState::extraction` は
    /// そのリクエストで引いた `Character` から配線を作る）ので、
    /// `PATCH` で尺度を変えても次の往復には届く。
    pub(in crate::server) stat_scale: StatScale,
    /// 記録用。**どのセッションの往復から抽出したか。**
    pub(in crate::server) session_id: SessionId,
    /// **その往復の応答の `message.id`**（作業単位 A-9c のレビュー）。
    ///
    /// 使うのは試用の輪番だけ（どの試用中の候補を採点するか / `domain::candidate_to_score`）。
    /// **往復ごとに必ず変わる値**が要る —— 票の件数のように「採点できた回だけ進む値」を鍵にすると、
    /// 読める答えが返らない候補を選んだ回に鍵が進まず、同じ候補を選び続ける。
    pub(in crate::server) reply_id: MessageId,
    /// 今回の利用者の発言。
    pub(in crate::server) user_message: String,
    /// 今回のキャラクターの応答。**生出力から、上流が漏らした思考だけを剥がしたもの**
    /// （分解結果を組み直さない / 剥がすのは [`Extraction::start`]。ADR-0053 §決定4）。
    pub(in crate::server) character_message: String,
    /// **その往復で効いているサブモデルの窓**（作業単位 B-7 / ADR-0020）。
    ///
    /// **抽出と変容の両方がここから上限を出す。** 2つは同じサブモデルの上流へ、
    /// 同じ1本の枠の中で順に依頼を出す（ADR-0024 §2）ので、
    /// **窓は往復ごとに1つ**——`stat_scale` と同じ扱いで、材料としてここに載せる。
    ///
    /// **`MemoryExtractor` の器から往復の入口で1回だけ読む**
    /// （[`Extraction::start`]）。**タスクの中で読み直さない**——
    /// 引き直しを跨いだ回だけ、抽出と変容が別の窓で組むことになる。
    ///
    /// # ⚠ **片方だけを定数のままにしないこと**
    ///
    /// 変容側は B-7 の最初の版で定数（8192）のままだった。
    /// **サブが 6144 で載っている機械では、抽出は正しく捨てるのに変容は 8192 まで送り、
    /// 窓を超える。** LM Studio では前方から黙って切り捨てられる（ADR-0013）——しかも変容は
    /// 本文に出ない決定（ADR-0024 §9）なので、**利用者にもログにも出ない。**
    /// llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5。`warn` で残る）。
    /// プロンプト ＋ 応答の上限が窓を超えて応答が短くなった回は、変容が丸ごと捨て、
    /// その旨を `server::drift` の `record_run` が出す欄に残す（綴りは向こうの doc）。
    /// **「同じ危険に入口が2つあるとき、片方だけ塞ぐと反対側から入られる」**（CLAUDE.md / 16e-b）。
    pub(in crate::server) sub_context: SubContext,
    /// **その往復に効く変容の走らせ方と、その出どころ**
    /// （[ADR-0045](../../docs/adr/0045-drift-mode-per-character.md) §決定2 / 作業単位 A-8）。
    ///
    /// **材料はチャット要求の入口（`server::chat`）で引いたキャラクターの行**と、
    /// 抽出器が持つ起動時の既定。**解決するのは応答を保存した後の [`Extraction::start`] で、1回だけ。**
    /// 行は入口で引いた写し（[`Extraction::new`] が持つ）なので、
    /// **応答を返している最中に来た `PATCH` はそのターンには効かず、次のターンから効く**
    /// （`stat_scale` と同じ扱い / 起こし直しは要らない）。
    ///
    /// **タスクの中で決め直さない**——判定を走らせるか（[`run`]）と、
    /// 値を動かしてよいか（`super::drift::run_drift`）が**同じ1つの値**を見る。
    pub(in crate::server) drift_mode: ResolvedDriftMode,
    /// **そのキャラクターの、軸を見いだす枠の数 `m`**（ADR-0046 §決定1 / 作業単位 A-9b）。
    ///
    /// `stat_scale` と同じく**チャット要求の入口で引いた行の写し**。
    /// **0 なら候補づくりは DB も上流も触らずに終わる**（`super::axis_discovery`）——
    /// Issue origin#42「`m` = 0 のキャラクターには1回も依頼しない」。
    /// **ここは入口の写しでしかない**：空いた枠の最終判定は書き手が
    /// `BEGIN IMMEDIATE` の中で読み直す（`Store::record_axis_candidate`）。
    pub(in crate::server) axis_discovery_slots: AxisDiscoverySlots,
}

/// 破棄の内訳をログの1フィールドに畳む。
///
/// **`tracing` のフィールド名は静的**なので、理由ごとに欄を生やすには
/// マクロ側に綴りを並べることになる。それをやると
/// [`DiscardCounts::iter`] が返す一覧と**二重管理**になり、理由が増えたときに
/// 片方だけ古びる。ここは「ラベル=件数」を1つの値として書き出す。
///
/// **0 件の理由も出す**（`iter` がそう返す）。欄ごと消すと
/// 「0 件だった」と「その理由が存在しない」が区別できず、分母が作れない。
struct DiscardField<'a>(&'a DiscardCounts);

impl fmt::Display for DiscardField<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, (reason, count)) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(",")?;
            }
            write!(f, "{}={count}", reason.label())?;
        }
        Ok(())
    }
}

/// 抽出を走らせる仕掛け。**`AppState` が1つだけ持ち、全リクエストで共有する。**
///
/// 共有するのは「同時に走らせるのは1件まで」（§7）がプロセス全体の約束だから。
/// リクエストごとに作ると、**同時に来た数だけ抽出が走る。**
pub(super) struct MemoryExtractor {
    /// 永続化層。`Store` は `Clone` で同じプールを指す。
    store: Store,
    /// **裏方（抽出）用の上流。** 本文生成とは別のモデルを指す（§10）。
    ///
    /// **`AppState::llm` と同じものを入れないこと。** 入れると 14B が抽出を回し、
    /// 本文生成のためのメモリを奪う。配線は `App::bind` の1箇所。
    llm: Arc<dyn LlmClient>,
    /// **サブモデルの実コンテキスト長。稼働中に差し替わる**（作業単位 B-7 / ADR-0020）。
    ///
    /// 依頼の上限（§8 の門と `prompt` へ渡す予算）は**ここからしか出ない。**
    /// **起動時の値を写し取らないこと**——写すと、上流が小さいコンテキストで
    /// 再ロードされても抽出だけが古い上限で組み続け、
    /// **窓を超えた依頼を送る**（§8 が避けたい形。LM Studio では前方から黙って切り捨てられる /
    /// ADR-0013。llama は 400 `exceed_context_size_error` で断るか、プロンプト ＋ 応答の上限が
    /// 窓を超えて応答が黙って短くなる / ADR-0052 M5）。
    ///
    /// **器は `server::context` が1つだけ作る。** ここに別の器を入れると
    /// 「引き直しは正しく WARN を出しているのに、抽出の上限だけが動かない」
    /// という壊れ方になる（`context::SharedBudget` と同じ理屈）。
    sub_context: super::context::SharedSubContext,
    /// **走っている1件。** 空か、終わったものが入っている状態が「暇」。
    ///
    /// `Semaphore` ではなく `JoinHandle` を持つのは、**同じ1つで
    /// 「1件まで」（§7）と「leak させない」（§9）の両方を担うため。**
    /// 別々の仕掛けにすると、片方だけ外しても誰も赤くならない状態が作れる。
    running: Mutex<Option<JoinHandle<()>>>,
    /// **性格ステータスの変容の走らせ方の、起動時の既定**（ADR-0024 §8 / `PERSONACORE_STAT_DRIFT`）。
    ///
    /// # ⚠ A-8 から「既定」である（ADR-0045）
    ///
    /// **効く走らせ方はキャラクターごとに、応答を保存した後の [`Extraction::start`] で決まる**
    /// （[`TurnJob::drift_mode`]）。
    /// ここにあるのは**キャラクターが未設定のときに効く値**で、
    /// [`Extraction::start`] と `admin::get_drift` だけが読む。
    ///
    /// **既定は off。** 起動設定から `App::bind_with` が1箇所で渡す。
    /// **抽出器が持つのは、変容が抽出と同じ1本の枠の中で走るから**（ADR-0024 §2）
    /// ——別の仕掛けにすると、本文生成の GPU を取り合う相手が1つ増える。
    ///
    /// **3値**（ADR-0026 §決定1）。`bool` ではないので、
    /// 「走らせるが値は動かさない」が**この1つの値で表せる。**
    default_stat_drift: StatDriftMode,
    /// **変容の依頼の形**（[ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md) §12 /
    /// `PERSONACORE_DRIFT_FORM`）。
    ///
    /// **既定は `Single`**（従来どおり1軸だけを尋ねる）。
    /// `stat_drift` と**別の値として持つ**——「走らせ方」と「尋ね方」は直交していて、
    /// 1つに畳むと `(走らせない, 全軸採点)` のような組が表現できなくなる。
    drift_form: DriftForm,
    /// **書き方の注記**（[ADR-0029](../../docs/adr/0029-language-as-a-matched-bundle.md) §訂正 /
    /// `PERSONACORE_DRIFT_NOTE`）。**既定は `None`**（注記を足さない）。
    ///
    /// `drift_form` と**別の値として持つ**——「尋ね方」と「書き方の注記」は直交していて、
    /// 1つに畳むと `2 × 3` の組を1つの綴りで表すことになり、
    /// **測定のログに何が効いたのかが書けなくなる。**
    drift_note: DriftNote,
    /// **変容の集約のつまみ**（ADR-0027 §決定8 / §9）。**既定は ADR の「当面の値」。**
    ///
    /// `stat_drift` / `drift_form` と同じ場所で受け取る——
    /// **3つとも「変容をどう走らせるか」の設定**で、渡し忘れると
    /// 帳簿が既定値で回り、**設定が効いていないことが既定値では観測できない。**
    drift_tuning: DriftTuning,
    /// **軸の候補を尋ねる間隔 `N`**（ADR-0046 §決定2 / `PERSONACORE_AXIS_DISCOVERY_MEMORIES` /
    /// 作業単位 A-9b）。
    ///
    /// **候補づくりは変容と同じ1本の枠の中、変容の直後に走る**（`run`）ので、
    /// 変容のつまみと同じ場所で受け取る。
    axis_discovery_memories: AxisDiscoveryMemories,
    /// **軸の候補の試用の判定のつまみ `T` / `K` / 線**（ADR-0046 §決定4 /
    /// `PERSONACORE_AXIS_TRIAL_*` / 作業単位 A-9c）。
    ///
    /// **試用は変容と同じ1本の枠の中、変容の直後に走る**（`run`）ので、
    /// 候補づくりの間隔と同じ場所で受け取る。
    axis_trial: AxisTrialRule,
    /// **約束・決まりの提案**（ADR-0051 §決定2 —— δ3 / 作業単位 A-12c-1 / `super::agreement_proposal`）。
    ///
    /// **同じ1本の枠の中、いちばん後ろで直列に**走る（`run`）。`Some` のときだけ走る。
    /// **本番の入口（`App::bind_with`）は必ず [`Self::with_agreement_proposals`] を通す**
    /// （見張り番は `tests/server_agreement_proposals.rs` —— 本番の入口から起こしたサーバで提案が書かれる）。
    /// `None` のまま作るのは、抽出・変容・軸の単体テストだけ（それらの台本に提案の依頼を混ぜないため）。
    agreements: Option<Arc<super::agreement_proposal::AgreementProposer>>,
    /// **走っている1本の番号**（`spawn` のたびに1つ進む。0 は「まだ1本も走らせていない」）。
    ///
    /// 提案の段の横取り（作業単位 A-12c-1 のレビュー 要修正1 / [`Self::spawn`]）で、
    /// 「いま提案の段にいるのは、枠に居る1本か」を突き合わせるのに使う。**錠（`running`）を握ったまま**読み書きする。
    generation: AtomicU64,
}

impl MemoryExtractor {
    /// 抽出器を作る。**上流はサブモデルを指すものを渡すこと**（§10）。
    ///
    /// `default_stat_drift` は ADR-0024 §8 のスイッチ。**既定は off** で、
    /// 呼び出し側（`App::bind_with`）が起動設定から渡す。
    /// **キャラクターが値を持っていれば、そちらが勝つ**（ADR-0045 §決定2）。
    ///
    /// `sub_context` は**そのサブモデルの実コンテキスト長の器**（作業単位 B-7）。
    /// **引数にしてある**のは、渡し忘れを型で止めるため——
    /// 後から差し込む形にすると、本番だけが落とし所のまま走る配線が書ける
    /// （CLAUDE.md「テスト専用の入口を作ると、本番の入口が観測対象から外れる」）。
    ///
    /// `axis_discovery_memories`（作業単位 A-9b）と `axis_trial`（作業単位 A-9c）も
    /// **同じ理由で引数**——渡し忘れを型で止める。
    #[expect(
        clippy::too_many_arguments,
        reason = "どれも起動時に1度だけ決まる別々の設定で、渡し忘れを型で止めるために引数にしている（この関数の doc）"
    )]
    pub(super) const fn new(
        store: Store,
        llm: Arc<dyn LlmClient>,
        sub_context: super::context::SharedSubContext,
        default_stat_drift: StatDriftMode,
        drift_form: DriftForm,
        drift_note: DriftNote,
        drift_tuning: DriftTuning,
        axis_discovery_memories: AxisDiscoveryMemories,
        axis_trial: AxisTrialRule,
    ) -> Self {
        Self {
            store,
            llm,
            sub_context,
            running: Mutex::new(None),
            default_stat_drift,
            drift_form,
            drift_note,
            drift_tuning,
            axis_discovery_memories,
            axis_trial,
            agreements: None,
            generation: AtomicU64::new(0),
        }
    }

    /// **約束・決まりの提案も、同じ1本の枠で走らせる**（作業単位 A-12c-1 / [`Self::agreements`] の doc）。
    #[must_use]
    pub(super) fn with_agreement_proposals(mut self) -> Self {
        self.agreements = Some(Arc::default());
        self
    }

    /// **この抽出器が持つ、変容の走らせ方の起動時の既定**（作業単位 D-5a / A-8）。
    ///
    /// `GET .../drift` の応答（`admin::DriftView::default_run_mode` と、
    /// キャラクターが未設定のときの `run_mode`）が、
    /// **`AppState::default_stat_drift` 経由でここを読む。**
    ///
    /// **写しを渡さないため**にここから読ませている——
    /// 応答が別の値を持てると、**「`on` と言うのに走らない」**が書けてしまい、
    /// **変容は本文に出ない**（ADR-0024 §9）ので画面からは永久に分からない。
    pub(super) const fn default_stat_drift(&self) -> StatDriftMode {
        self.default_stat_drift
    }

    /// 走っている抽出を落とす。**停止処理の一部**（§9 / `server::serve`）。
    ///
    /// 走っている抽出はサーバ停止で失われる。**それは受け入れる**——
    /// 上流の生成も一緒に止まるので、無駄な推論を続けない。
    /// ここで落とさないと、待ち受けが終わった後もタスクが生き残る（leak）。
    pub(super) fn shutdown(&self) {
        if let Some(handle) = self.take_slot().take() {
            handle.abort();
        }
    }

    /// 抽出を1件走らせる。**走っていたら捨てて数える**（§7）。
    ///
    /// **ここは応答の経路。** `await` を持たず、DB にも上流にも触らない。
    ///
    /// **その往復の窓は [`TurnJob::sub_context`] に載っている**
    /// （[`Extraction::start`] が入口で1回だけ読む / 作業単位 B-7）。
    ///
    /// # 提案の段だけは横取りする（作業単位 A-12c-1 のレビュー 要修正1）
    ///
    /// 走っている1本が**鎖のいちばん後ろ（約束・決まりの提案）の段にいる**なら、それを止めて（`abort`）
    /// 新しい往復の鎖を走らせる。提案の段は毎往復走るので、捨てる側に回すと、次の往復の**抽出**が
    /// `busy` で捨てられ続ける（記憶が生まれなくなる）。
    ///
    /// | 走っている1本がいる段 | 新しい往復 |
    /// |---|---|
    /// | 抽出・変容・試用・軸の候補 | **今までどおり捨てて数える**（`busy`） |
    /// | 約束・決まりの提案 | **止めて走らせる**（止めたことは `super::agreement_proposal` の記録に残る） |
    ///
    /// 止めた1本が失うのは、持ち越しの候補とその往復の確かめだけ（「失えば拾いが減るだけ」）。
    /// **書き込みは半端に残らない** —— 提案を書くのは1つのトランザクション（`Store::record_agreement_proposals`）で、
    /// 止めると `sqlx::Transaction` がコミットされずに落ち、巻き戻る
    /// （見張り番は `store::agreement` の `a_transaction_dropped_before_commit_writes_nothing`）。
    fn spawn(&self, job: TurnJob) {
        let mut slot = self.take_slot();

        // **終わっていないものがあれば走らせない。** 待ち行列にしない（§7）。
        // **ただし提案の段にいる1本は止めて、こちらを走らせる**（上の doc）。
        if slot.as_ref().is_some_and(|handle| !handle.is_finished()) {
            let running = self.generation.load(Ordering::SeqCst);
            let preemptable = self
                .agreements
                .as_ref()
                .is_some_and(|agreements| agreements.is_in_stage(running));
            if !preemptable {
                record_skip(SkipReason::Busy, &job, None, None);
                return;
            }
            if let Some(handle) = slot.take() {
                handle.abort();
            }
            super::agreement_proposal::record_preempted(&job);
        }
        let generation = self
            .generation
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);

        let store = self.store.clone();
        let llm = Arc::clone(&self.llm);
        let drift_form = self.drift_form;
        let drift_note = self.drift_note;
        let drift_tuning = self.drift_tuning;
        let axis_discovery_memories = self.axis_discovery_memories;
        let axis_trial = self.axis_trial;
        let agreements = self.agreements.clone();
        *slot = Some(tokio::spawn(async move {
            run(
                &store,
                llm.as_ref(),
                job,
                drift_form,
                drift_note,
                drift_tuning,
                axis_discovery_memories,
                axis_trial,
                agreements
                    .as_deref()
                    .map(|agreements| (agreements, generation)),
            )
            .await;
        }));
    }

    /// **いま効いているサブモデルの窓**（作業単位 B-7）。
    ///
    /// # 往復ごとに読み直す
    ///
    /// 起動時の写しを持たない（`AppState::budget` と同じ作法）。読むのは
    /// `RwLock` の読み取り1回で、`await` も通信も伴わない。
    /// **1往復の中では1回だけ読む**——タスクの中で読み直すと、
    /// 門にかけた上限と `prompt` へ渡した予算、そして変容の上限がずれうる。
    ///
    /// **本文の数えた計画も読む**（`AppState::sub_context` / ADR-0052 §決定7 の差し引きの出どころ）。
    pub(in crate::server) fn current_sub_context(&self) -> SubContext {
        self.sub_context.get()
    }

    /// 錠を取る。**毒（poison）でも進む**——中身は `JoinHandle` だけで、
    /// 壊れた不変条件が無い。ここで `unwrap` すると
    /// **本文とは無関係な裏方の都合でリクエストが panic する。**
    fn take_slot(&self) -> std::sync::MutexGuard<'_, Option<JoinHandle<()>>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// **裏方の枠が空いているか。テスト専用**（作業単位 A-8 のレビュー指摘）。
    ///
    /// **[`Self::spawn`] が `Busy` と判定する条件の否定そのもの**——
    /// ここが真なら、次の往復の裏方は `Busy` で飛ばない。
    /// 別の条件（DB の書き込みやログの行）で待つと、最後の書き込みからタスクが
    /// 終わるまでの隙が残る。
    #[cfg(any(test, feature = "testing"))]
    pub(super) fn is_idle(&self) -> bool {
        !self
            .take_slot()
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
    }

    /// **走っている抽出が終わるまで待つ。テスト専用。**
    ///
    /// 実時間の `sleep` で待ち合わせると、遅い機械でだけ落ちる見張り番になる。
    #[cfg(test)]
    pub(in crate::server) async fn wait_for_the_running_task(&self) {
        let handle = self.take_slot().take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }
}

/// **裏方（抽出と変容の1本の枠）が空いたかを外から見る口。テスト専用**
/// （作業単位 A-8 / `App::background_probe`）。
///
/// **見るだけ。** 走っているタスクを待ったり止めたりはできない
/// （`tests/` の見張り番は、本番の入口を通したサーバを外から叩くので、
/// 枠の中へ手を入れる口は要らない）。
#[cfg(any(test, feature = "testing"))]
#[derive(Clone)]
pub struct BackgroundProbe(Arc<MemoryExtractor>);

#[cfg(any(test, feature = "testing"))]
impl BackgroundProbe {
    /// 口を作る。**`App` だけが呼ぶ。**
    pub(super) const fn new(extractor: Arc<MemoryExtractor>) -> Self {
        Self(extractor)
    }

    /// **裏方の枠が空いているか**（`MemoryExtractor::is_idle`）。
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.0.is_idle()
    }
}

/// **何も書き出さない上流。テスト専用。**
///
/// `App::bind_with_client`（本文のモックだけを差す入口）が抽出側に据える。
///
/// # なぜ本文のモックを流用しないのか（実測 / 2026-08-21）
///
/// 抽出も同じモックへ行かせると、**本文の呼び出しと抽出の呼び出しが1つの列に混ざる。**
/// 既存の見張り番は「最後に上流へ渡した依頼」を見ているものが多く、
/// 実測で **14 本**（`tests/server_chat.rs` 12 / `tests/server_chat_stream.rs` 2）が
/// 赤になった。しかも抽出は非同期なので、**どちらが最後かは競走で決まる。**
///
/// **黙らせるのではなく、行き先を分ける。** 抽出のタスクは同じように spawn され、
/// 同じ経路を通り、ただ**空の応答**を受け取って 0 件で終わる。
/// **抽出そのものを見るテストは `App::bind_with_clients` で本物のモックを差す。**
#[cfg(any(test, feature = "testing"))]
pub(super) struct SilentUpstream;

#[cfg(any(test, feature = "testing"))]
impl LlmClient for SilentUpstream {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(
        &'a self,
        _request: &'a ChatRequest,
    ) -> crate::llm::LlmFuture<'a, crate::llm::Completion> {
        Box::pin(async move {
            Ok(crate::llm::Completion {
                content: String::new(),
                // **「書き切って、何も書かなかった」**（作業単位 24）。
                // `None`（申告なし）にすると、この入口を通るすべてのテストが
                // [`NO_FINISH_REASON_MESSAGE`] を吐き、**本物の申告漏れが埋もれる。**
                finish_reason: Some(crate::llm::FinishReason::Stop),
                usage: None,
            })
        })
    }

    fn stream<'a>(
        &'a self,
        _request: &'a ChatRequest,
    ) -> crate::llm::LlmFuture<'a, crate::llm::ChatStream> {
        // **抽出は逐次表示を必要としない**（`llm::LlmClient::complete` の doc）。
        Box::pin(async move { Err(crate::llm::LlmError::MissingContent) })
    }
}

/// 1つの往復に対する裏方（抽出と変容）の配線。**`TurnSink` が持ち回る。**
///
/// **`&Character` から作る。** `display_name` と `handle` を取り違えると
/// プロンプトに識別子が載る（ADR-0011 が避けたい形）ので、
/// 文字列ではなくキャラクターそのものを受け取る。
/// **尺度も同じ理由でここから取る**（ADR-0015 / `TurnJob::stat_scale`）。
pub(super) struct Extraction {
    extractor: Arc<MemoryExtractor>,
    character_id: CharacterId,
    display_name: DisplayName,
    /// **そのキャラクターの尺度**（変容の依頼に載る上限 / ADR-0024 §4）。
    stat_scale: StatScale,
    /// **そのキャラクターに書かれた変容の走らせ方**（ADR-0045）。`None` は未設定。
    ///
    /// **尺度と同じく、チャット要求の入口（`server::chat`）で引いた行から取る**ので、
    /// `PATCH` で書き換えた値は次の往復に届く（起こし直しは要らない / ADR-0045 §決定2）。
    /// **応答の最中に来た `PATCH` は、その往復には届かない**（この配線はもう作られている）。
    stat_drift_mode: Option<StatDriftMode>,
    /// **そのキャラクターの、軸を見いだす枠の数**（ADR-0046 §決定1 / 作業単位 A-9b）。
    /// 尺度と同じく入口で引いた行の写し（[`TurnJob::axis_discovery_slots`]）。
    axis_discovery_slots: AxisDiscoverySlots,
}

impl Extraction {
    /// 配線を作る。
    pub(super) fn new(extractor: Arc<MemoryExtractor>, character: &Character) -> Self {
        Self {
            extractor,
            character_id: character.id,
            display_name: character.display_name.clone(),
            stat_scale: character.stat_scale,
            stat_drift_mode: character.stat_drift_mode,
            axis_discovery_slots: character.axis_discovery_slots,
        }
    }

    /// **上流を持たない配線。テスト専用。**
    ///
    /// `super::relay` の単体テストが見ているのは保存の順序と件数で、抽出は対象外。
    /// それでも `TurnSink` は配線を必ず要求する（`Option` にしない）ので、
    /// **何も返さない上流**を差した配線をここから渡す。
    ///
    /// **本物の配線（両経路で現に抽出が走ること）はここでは見られない。**
    /// 見張るのは `tests/server_memory_extraction.rs`。
    #[cfg(test)]
    pub(super) fn quiet_for_tests(store: Store) -> Self {
        Self {
            extractor: Arc::new(MemoryExtractor::new(
                store,
                Arc::new(SilentUpstream),
                super::context::SharedSubContext::fallback_for_tests(),
                StatDriftMode::Off,
                DriftForm::Single,
                DriftNote::None,
                DriftTuning::default(),
                AxisDiscoveryMemories::DEFAULT,
                AxisTrialRule::DEFAULT,
            )),
            character_id: CharacterId::from_row(1),
            display_name: DisplayName::parse("葵").expect("正当な表示名であること"),
            stat_scale: StatScale::DEFAULT,
            stat_drift_mode: None,
            axis_discovery_slots: AxisDiscoverySlots::NONE,
        }
    }

    /// **保存に成功した往復から抽出を始める**（§6）。
    ///
    /// **呼ぶのは `TurnSink::persist` の1箇所だけ。** 非ストリーミングと中継の
    /// 両方がそこを通るので、配線が1本で済む（2箇所に置くと
    /// **片方だけが観測対象から外れる**——11c-3b で実測した形）。
    ///
    /// `reply_id` は**保存した応答の `message.id`**（試用の輪番の鍵 / [`TurnJob::reply_id`]）。
    pub(super) fn start(
        &self,
        session_id: SessionId,
        reply_id: MessageId,
        user_message: &str,
        raw_reply: &str,
    ) {
        self.extractor.spawn(TurnJob {
            character_id: self.character_id,
            display_name: self.display_name.clone(),
            stat_scale: self.stat_scale,
            session_id,
            reply_id,
            user_message: user_message.to_owned(),
            // **上流が漏らした思考を剥がしてから渡す**（ADR-0053 §決定4 / 作業単位 B-61）。
            // 抽出・変容・試用・約束の提案は、**全部この1つの値を読む**（[`TurnJob::character_message`]）。
            // 思考だけの回は空になり、各依頼は既存の「本文が空なら節を出さない」に入る。
            character_message: strip_leaked_thinking(raw_reply).text.into_owned(),
            // **窓は往復の入口で1回だけ読む**（作業単位 B-7 / [`TurnJob::sub_context`]）。
            // ここから先（抽出も変容も）は、この1つの値だけを見る。
            sub_context: self.extractor.current_sub_context(),
            // **走らせ方もここで1回だけ決める**（ADR-0045 §決定2 / [`TurnJob::drift_mode`]）。
            // **キャラクターの値（チャット要求の入口で引いた行の写し）があればそれ、無ければ起動時の既定。**
            drift_mode: ResolvedDriftMode::resolve(
                self.stat_drift_mode,
                self.extractor.default_stat_drift(),
            ),
            axis_discovery_slots: self.axis_discovery_slots,
        });
    }
}

/// **1往復ぶんの裏方。タスクの中で走る**（応答の経路ではない）。
///
/// ```text
/// extract         … 記憶抽出（ADR-0021）
/// drift           … 性格ステータスの変容（ADR-0024）。**効く走らせ方が off でないときだけ**
/// axis_trial      … 試用中の軸の候補を1つ採点する（ADR-0046 §決定3・4 / A-9c）。**同じく off でないときだけ**。
///                   さらに試用中の候補があるときだけ上流を呼ぶ（**固定の軸の依頼とは別の依頼**）
/// axis_discovery  … 軸の候補を1つ尋ねる（ADR-0046 §決定2 / A-9b）。**同じく off でないときだけ**。
///                   さらに枠が空いていて、記憶が `N` 件増えた回だけ上流を呼ぶ
/// agreements      … 約束・決まりの提案（ADR-0051 §決定2 —— δ3 / A-12c-1）。**変容の走らせ方に依らず**、
///                   検出1本と、1往復前の候補があれば確かめ1〜2本（`super::agreement_proposal`）
/// ```
///
/// # 試用は**変容の後・候補づくりの前**（作業単位 A-9c）
///
/// * **変容の後** —— 固定の軸の依頼・票・値に何も渡さないため（ADR-0046 §決定3）。
///   試用は固定の軸の表を1つも書かないので、順番が逆でも固定の軸は変わらないが、
///   **固定の軸の依頼が先に出る形**にしておけば、試用の有無で固定の軸の依頼の出る時刻さえ動かない。
/// * **候補づくりの前** —— このターンに判定して不合格になった候補の枠を、同じターンの候補づくりが使える。
///   逆にすると、**このターンに生まれた候補を、生まれた記憶とは無関係なこのターンの会話で採点する**
///   ことになる（候補は記憶から出る / §決定2）。
/// * **試用も off なら走らない**（候補づくりと同じ判断 / 下の節）。off で試用だけ走らせると
///   「変容のための GPU を使わない」（ADR-0024 §8）が崩れる。
///
/// # 候補づくりも**変容の走らせ方が off なら走らない**（作業単位 A-9b の判断）
///
/// ADR-0046 は候補づくりと走らせ方（ADR-0045）の関係を書いていない。**off では走らせない側を採る:**
///
/// * ADR-0046 の題が「**変容が**新しい軸の候補を見いだす」—— 候補づくりは変容の一部である。
/// * off は「判定そのものを走らせない」（ADR-0024 §8）＝**変容のための GPU を使わない**という決定。
/// * **試用（A-9c / Issue origin#43）は off なら走らない。** off で候補だけ作ると、判定されない試用が
///   枠を埋めて止まる（数が決まらない候補が溜まるだけ）。
///
/// 見張り番は `tests/server_axis_discovery.rs` の
/// `the_axis_discovery_interval_reaches_the_background_through_the_real_bind`（off の腕）。
///
/// # 変容は**抽出の直後に、直列に**走る（ADR-0024 §2）
///
/// 抽出タスクは同時に1本だけ（§7）。**変容をその中に置けば、
/// 本文生成の GPU を取り合う相手は増えない。**
///
/// # **抽出が飛んだ理由によって、変容の扱いが違う**
///
/// | 抽出が飛んだ理由 | 変容 | なぜ |
/// |---|---|---|
/// | `Busy`（前の1本が走っている） | **飛ぶ** | この関数そのものが呼ばれない（`spawn` が弾く / ADR-0024 §2） |
/// | 依頼が長すぎる | **走る** | 変容の依頼は記憶を載せず、会話も直前の1往復だけ。**自分の予算判定を持つ** |
/// | 記憶が読めない | **走る** | 変容は**記憶を読まない** |
///
/// **だから早期 return は [`extract`] の中に閉じてある。**
/// ここに `return` を書くと、抽出の事情が変容を巻き込む。
#[expect(
    clippy::too_many_arguments,
    reason = "どれも起動時に1度だけ決まる別々の設定を素通しで渡している（`MemoryExtractor::new` の doc）"
)]
async fn run(
    store: &Store,
    llm: &dyn LlmClient,
    job: TurnJob,
    drift_form: DriftForm,
    // **書き方の注記**（ADR-0029 §訂正 / 作業単位 16m）。素通しで `run_drift` へ渡す。
    drift_note: DriftNote,
    // **変容の集約のつまみ**（ADR-0027 §決定8 / §9）。素通しで `run_drift` へ渡す。
    drift_tuning: DriftTuning,
    // **軸の候補を尋ねる間隔**（ADR-0046 §決定2 / A-9b）。素通しで `run_axis_discovery` へ渡す。
    axis_discovery_memories: AxisDiscoveryMemories,
    // **試用の判定のつまみ**（ADR-0046 §決定4 / A-9c）。素通しで `run_axis_trial` へ渡す。
    axis_trial: AxisTrialRule,
    // **約束・決まりの提案の持ち越し**と、この1本の番号（作業単位 A-12c-1）。`None` なら提案は走らない。
    agreements: Option<(&super::agreement_proposal::AgreementProposer, u64)>,
) {
    extract(store, llm, &job).await;

    // **効く走らせ方が off なら上流を呼ばない**（ADR-0024 §8）。
    // 呼んでから結果を捨てる形にすると、off でも GPU を使う。
    //
    // **効く走らせ方はキャラクターごと**（ADR-0045 §決定2）。[`Extraction::start`] で決めた
    // [`TurnJob::drift_mode`] だけを見る——ここで起動時の既定を読み直すと、
    // `off` にしたキャラクターでも既定が `on` なら走ってしまう。
    //
    // **`off` では票も残らない**（ADR-0026 §決定3）——この関数を呼ばないので、
    // 記録の経路そのものに入らない。見張り番は
    // `super::drift::tests::the_off_mode_never_records_a_vote`。
    if job.drift_mode.mode().runs() {
        super::drift::run_drift(store, llm, &job, drift_form, drift_note, drift_tuning).await;
        // **試用は変容の後・候補づくりの前**（同じ1本の枠の中で直列 / 上の doc）。
        // 固定の軸の依頼・票・値には何も渡さない（**別の依頼** / ADR-0046 §決定3）。
        super::axis_trial::run_axis_trial(store, llm, &job, axis_trial).await;
        // **候補づくりは変容の後**（同じ1本の枠の中で直列 / 上の doc）。
        // 変容の依頼・票・値には何も渡さない（固定の軸の振る舞いを変えない）。
        super::axis_discovery::run_axis_discovery(store, llm, &job, axis_discovery_memories).await;
    }

    // **約束・決まりの提案は鎖のいちばん後ろ**（作業単位 A-12c-1 / 上の doc）。
    // **変容の走らせ方には依らない**（変容とは別の機能）。上の仕事が何で終わっても走る。
    // **この段に入ったことを印す**（この段の間だけ、次の往復が横取りできる / `MemoryExtractor::spawn`）。
    if let Some((agreements, generation)) = agreements {
        agreements.enter_stage(generation);
        super::agreement_proposal::run_agreement_proposals(store, llm, &job, agreements).await;
        agreements.leave_stage(generation);
    }
}

/// 抽出の本体。
///
/// **どんな失敗でも応答には影響しない。** 戻り値を持たず、
/// 呼び出し側は結果を見ない。**早期 return はこの関数の中に閉じる**
/// （[`run`] の doc）。
///
/// # 記憶を読めなかったら、**抽出そのものを行わない**（作業単位 13c-3b の判断）
///
/// 「記憶なしで続ける」も選べる——**重複が増えるだけで、記憶が生まれないよりまし**とも読める。
/// **それでも走らせない側を採る。** 2つの失敗の性質が対称ではないため:
///
/// * **走らせなかった回に失われるのは、その1往復ぶんの事実だけ。**
///   しかもキャラクターも利用者も**同じ事実を後の往復で言い直す**
///   （その前提の上に「新しい順に採る」が乗っている / ADR-0021 §13）。
/// * **知らないまま走らせて入った重複は、自動では消えない。**
///   行は潰さない（絶対制約）ので、外すには**人が1件ずつ訂正版で上書きする**
///   （ADR-0022）。**入る側は自動で、出す側は手作業**という非対称がここの要点。
///   §13 が塞ごうとしている重複と自己増幅の原因は
///   **「抽出器が既存の記憶を知らない」こと**そのものなので、
///   読み出しが落ちた回は**その原因に自分から入っていく**ことになる。
/// * 読めない原因（`InvalidStoredValue`）は**そのキャラクターについて持続する。**
///   一度きりの取りこぼしではなく、**以後の全往復が控除リスト無しで走る。**
///   同じ状態で `chat` の側は既に 500 を返している（`server::chat`）ので、
///   裏方だけが黙って走り続ける形にはしない。
///
/// **失敗は必ずログに残す**（`warn` ＋ 走らなかった回として数える）。
/// 握り潰すと「記憶を渡しているのに重複が減らない」ときの原因が消える。
async fn extract(store: &Store, llm: &dyn LlmClient, job: &TurnJob) {
    // **その往復の窓**（作業単位 B-7）。予算・門・ログ欄の3つが**この1つ**から出る。
    let sub_context = job.sub_context;
    // **すでに知っていること**（§13）。**上流を呼ぶ前**に、トランザクションを跨がずに読む。
    // 想起の入口は `Store::recall_memories` ひとつ（ADR-0003）。
    let known_memories = match store.recall_memories(job.character_id).await {
        Ok(memories) => memories,
        Err(err) => {
            // **知らないまま走らせない**（この関数の doc）。
            record_skip(SkipReason::MemoriesUnreadable, job, None, Some(&err));
            return;
        }
    };

    // 予算は `prompt` 側が配る（ADR-0021 §13 / CLAUDE.md 3節）。
    // **入らなかった件数はここでログに出すだけ**——選ぶのはあちらの責務。
    let prompt = build_extraction_prompt(&ExtractionInput {
        display_name: &job.display_name,
        user_message: &job.user_message,
        character_message: &job.character_message,
        known_memories: &known_memories,
        // **予算も門も同じ1つの上限から出る**（作業単位 B-7）。
        // 別々に決めると、記憶を載せる側と捨てる側が違う数を見ることになる。
        budget: sub_context.prompt_limit(),
        // **本番の手本の綴りはここが唯一の決定点**（作業単位 22a / ADR-0021 §15）。
        //
        // **既定を変えるのはこの作業単位ではない。** 22b の実機 A/B で
        // [`ExampleStyle::CharacterOnly`] が効くと分かってから差し替える。
        // **見張り番は [`a_request_exactly_at_the_limit_is_sent`] と
        // [`the_test_only_entry_points_hand_back_what_production_sends`]。**
        // どちらも [`expected_request`]（**綴りを直書き**）ないし本番が上流へ渡した
        // 依頼そのものと突き合わせるので、ここを `CharacterOnly` に変えると赤になる
        // （実測 / 2026-08-27 の B-11 のレビュー: この2本 ＋
        // `tests/server_memory_extraction.rs` の
        // `the_sub_model_reaches_the_upstream_through_the_real_bind` の計3本）。
        //
        // > **⚠ 上限を超える側（[`a_request_one_byte_over_the_limit_is_dropped`]）は
        // > 綴りを観測できない。** あちらは**依頼を送らない**回を見ているので本文が無く、
        // > `prompt_tokens=` でも区別できない——**手本の2つの綴りは枠が1トークンも
        // > 違わないことを [`every_example_style_leaves_the_same_room_for_a_conversation`]
        // > が要求している**（バイト数が同じ）。
        // > **B-11 の前は `ClosingStyle` の話で、そちらは3バイト差があったので赤になった。**
        // > **値だけ差し替えて主張を残さないこと。**
        example_style: ExampleStyle::Balanced,
    });
    let known = KnownMemoryCounts::of(&prompt);
    let messages = prompt.into_messages();

    // **長すぎたら丸ごと捨てる**（§8）。断片から抽出すると、誤った事実が恒久的に残る。
    // **記憶の有無で §8 の挙動を変えない**（ADR-0021 §13）。
    // 記憶は予算の残りからしか取らないので、ここに来る回は
    // 会話だけで枠を超えている——**丸ごと捨てる。切り詰めない。**
    let request = match extraction_request(messages, sub_context.prompt_limit()) {
        Ok(request) => request,
        Err(tokens) => {
            record_skip(SkipReason::TooLong, job, Some(tokens), None);
            return;
        }
    };

    // **トランザクションの外で通信する**（CLAUDE.md 絶対制約）。
    // 保存はこの `await` が返り切ってから。
    let completion = match llm.complete(&request).await {
        Ok(completion) => completion,
        Err(err) => {
            // 抽出だけが失敗した。**本文の応答はとうに返っている。**
            tracing::warn!(
                error = %err,
                detail = ?err,
                session_id = job.session_id.get(),
                "記憶の抽出で上流を呼べませんでした"
            );
            return;
        }
    };

    // **崩れていても panic しない**（絶対制約）。捨てた分は数に残る。
    // **打ち切られた回は、最後の1行が断片**なので分解器に捨てさせる（§17）。
    if completion.finish_reason.is_none() {
        record_missing_finish_reason(job);
    }
    let extracted = parse_extraction(
        &completion.content,
        response_completeness(completion.finish_reason),
    );
    let discards = extracted.discards();
    let parsed = extracted.memories().len();

    let mut saved = 0_usize;
    for memory in extracted.memories() {
        // **1件ずつ書く。** まとめて1トランザクションにすると、
        // 1件の失敗で全部落ちるうえ、書きロックを長く持つ（裏方が本文を待たせる）。
        // **往復の会話で縛って書く**（作業単位 A-10）。消されたキャラクターと同じ id で作り直された
        // キャラクターに、この往復の記憶を入れない（`StoreError::TurnSessionGone` の doc）。
        match store
            .add_memory_in_session(job.character_id, job.session_id, memory)
            .await
        {
            Ok(_) => saved = saved.saturating_add(1),
            // **キャラクターが完全に消された回は故障ではない**（ADR-0049 §決定3）。捨てて、記録だけ残す。
            Err(err) if character_vanished(&err) => record_vanished(job, VANISHED_MEMORY, &err),
            Err(err) => tracing::error!(
                error = %err,
                detail = ?err,
                session_id = job.session_id.get(),
                "抽出した記憶を保存できませんでした"
            ),
        }
    }

    record_run(
        job,
        parsed,
        saved,
        discards,
        extracted.emphasis_stripped(),
        extracted.quotes_stripped(),
        // **手本の文を写した回**（作業単位 B-28 / C-15）。**判定は `prompt::extraction`。**
        // **生出力を渡す**——保存された記憶だけを見ると、実測の 2 件のうち
        // 1 件（主語が無くて捨てられた側）が見えない。
        example_echoes(&completion.content),
        // **手本の「書式」が本文に居座った回**（作業単位 B-33 / C-15）。
        // **判定は `prompt::extraction`。** こちらは**採った記憶**を渡す
        // ——実害は「永久に残ること」なので、捨てた行の角括弧は数えない。
        example_format_echoes(extracted.memories()),
        known,
    );
}

/// **上流の停止理由を「最後の行が断片か」に翻訳する**（ADR-0021 §17 / 作業単位 24）。
///
/// # ここが唯一の決定点
///
/// **`parse::extraction` は `finish_reason` を見ない**（純粋関数の側に `llm` を持ち込まない）。
/// **実機の器具も写しを作らない**——[`response_completeness_for_tests`] 越しにここを通る。
///
/// # **変容も同じ関数を通る**（作業単位 B-1）
///
/// `server::drift` は**この対応付けを写さずに呼ぶ。**
/// 「どの停止理由を打ち切りと見なすか」は上流の申告の読み方であって、
/// **どちらのサブLLM 呼び出しかとは関係しない。**
/// 写しを置くと、**上流の綴りが増えた日に片方だけが古びる。**
///
/// **⚠ 打ち切られた回に何をするかは共有していない。**
/// 抽出は**最後の1行だけ**を捨て、変容は**丸ごと**捨てる
/// （ADR-0027 §打ち切り。理由は「変容は2行以上を丸ごと捨てる決まりなので、
/// 何行あったか分からない回の1行を信じる根拠が無い」）。
///
/// # 落とすのは `length` の回だけ
///
/// * [`FinishReason::Length`] —— **上限で打ち切られた。** 最後の行は断片で、
///   `- 利用者: 十年前から通って` のように**そのままパースを通ってしまう**ことがある。
/// * [`FinishReason::Stop`] —— 上流が書き終えた。**落とすと正しい事実を毎回1件捨てる。**
/// * [`FinishReason::Other`] —— `content_filter` / 未知の綴り。**打ち切りだと決めつけない。**
/// * **`None`（申告なし）—— [`Completeness::Complete`] に倒す。**
///   ここで「切れたかもしれない」側に倒すと、**停止理由を返さない上流に繋いだ瞬間、
///   毎ターン最後の1行が消える**——しかも捨てた理由は `truncated` なので、
///   ログを読んでも「上限に張り付いている」としか見えない。
///   **代わりに、申告が無いこと自体を [`record_missing_finish_reason`] が残す。**
pub(in crate::server) const fn response_completeness(
    finish_reason: Option<FinishReason>,
) -> Completeness {
    match finish_reason {
        Some(FinishReason::Length) => Completeness::Truncated,
        Some(FinishReason::Stop | FinishReason::Other) | None => Completeness::Complete,
    }
}

/// **停止理由の翻訳を、器具から本番と同じ経路で通すための入口。テスト専用**（作業単位 24）。
///
/// `tests/extraction_quality_harness.rs` は実機の応答を分解して数える。
/// **器具が `finish_reason` の対応付けを写すと、本番が変わったときに器具だけが古びる**
/// （[`extraction_request_for_tests`] と同じ理由）。
#[cfg(any(test, feature = "testing"))]
#[must_use]
pub const fn response_completeness_for_tests(finish_reason: Option<FinishReason>) -> Completeness {
    response_completeness(finish_reason)
}

/// **上流が停止理由を申告しなかったことを記録する**（作業単位 24）。
///
/// この回は「**打ち切られたか分からない**」状態で分解している
/// （[`response_completeness`] は `Complete` に倒す）。
/// **残さないと、その状態が起きていること自体に気付く手段がなくなる**——
/// 断片が恒久的な記憶になっていても、破棄の内訳は正常に見える。
///
/// **抽出を1回行ったこと自体の記録（[`record_run`]）とは別のイベントにしてある。**
/// あちらの欄は破棄率の集計に使っており、**別の意味の数を混ぜると比較が無効になる**
/// （`emphasis_stripped` / `known_included` と同じ作法）。
fn record_missing_finish_reason(job: &TurnJob) {
    tracing::warn!(
        session_id = job.session_id.get(),
        "{NO_FINISH_REASON_MESSAGE}"
    );
}

/// **裏方の書き込みが「キャラクターが消えた」で失敗したか**（ADR-0049 §決定3 / 作業単位 A-10）。
///
/// 裏方（抽出・変容・軸の候補）は本文の応答を返し切った後に走るので、その間に利用者が
/// キャラクターを**完全に消す**ことがある（しまってから `DELETE`）。
///
/// # 読むのは [`StoreError::TurnSessionGone`] だけ
///
/// 裏方の書き込みは全部、**それを生んだ会話（`session.id` は使い回されない）が今も在り、
/// 同じキャラクターのものであること**に縛ってある（`store` の `*_in_session` の関数）。
/// キャラクターが消えると会話もカスケードで消えるので、書き込みは何も書かずに
/// `TurnSessionGone` を返す。**故障ではないので `error` にしない**（[`record_vanished`] が
/// `info` で残す）。**捨てる**——消えたキャラクターへ書き直す先は無い。
///
/// **外部キー違反や `UnknownCharacter` は「消えた」と読まない**（作業単位 A-10 のレビュー）。
/// 縛りが会話を先に確かめるので、キャラクターが消えたことはそこへ届かない。
/// 届くなら本物の故障（別のキャラクターの記憶の id を渡す欠陥など）で、`info` に化かすと
/// 気づけなくなる —— **今までどおり `warn` / `error` に落ちる。**
///
/// **会話を1つだけ消す口を足す日には、この読みを見直すこと**（その日から `TurnSessionGone` は
/// 「キャラクターが消えた」だけを意味しなくなる / ADR-0049 §決定2）。
pub(in crate::server) fn character_vanished(err: &StoreError) -> bool {
    matches!(err, StoreError::TurnSessionGone)
}

/// [`record_vanished`] のメッセージ（**テストが綴りを照合する**）。
pub(in crate::server) const VANISHED_MESSAGE: &str =
    "キャラクターが消えていたので、裏方の書き込みを捨てました";

/// 捨てた書き込みの種類（`vanished_write` 欄）。**抽出した記憶。**
///
/// **綴りに表の名前そのもの（1語）を使わない** —— `store::memory` の走査は `src` 全体の
/// 文字列リテラルから表の名前を拾うので、1語で書くと「許可されていない形」として止まる。
const VANISHED_MEMORY: &str = "extracted_memory";

/// **消えたキャラクターへの書き込みを捨てたことを `info` で残す**（[`character_vanished`] の doc）。
///
/// **`warn` / `error` にしない** —— 利用者が自分で消した結果であって、直すものが無い。
/// それでも残すのは、**消した直後に走っていた裏方が何を落としたか**を後から追えるように。
pub(in crate::server) fn record_vanished(job: &TurnJob, what: &'static str, err: &StoreError) {
    tracing::info!(
        vanished_write = %what,
        error = %err,
        detail = ?err,
        character_id = job.character_id.get(),
        session_id = job.session_id.get(),
        "{VANISHED_MESSAGE}"
    );
}

/// **組み上がった依頼を、上流へ送る形にする**（§8 の門 ＋ 上流のつまみ）。
///
/// 超えていたら `Err(トークン数)` を返す。**切り詰めない**——
/// 断片から抽出すると、誤った事実が恒久的に残る（ADR-0021 §8）。
///
/// # ここが唯一の決定点
///
/// **数え方は `prompt` 側の [`request_tokens`] ひとつ。** ここに式を書くと、
/// 予算配分（`build_extraction_prompt`）と門が別の数を見ることになる。
///
/// **温度と応答の上限もここだけで決まる。** 実機の器具
/// （`tests/extraction_quality_harness.rs`）は
/// [`extraction_request_for_tests`] 越しに**この関数を通る**ので、
/// 本番が変わっても器具が黙って古びない（**写しを作らない**）。
///
/// # `limit` は呼び出し側が決める（作業単位 B-7）
///
/// **その環境のサブモデルの実コンテキストの半分**（[`SubContext::prompt_limit`]）。
/// **ここで `SubContext` を受けて自分で半分にしないこと**——
/// `prompt` へ渡す予算と門が別々に半分を計算する形になり、
/// 片方だけを緩める変異が「配分は狭いのに門は広い」状態を作る。
fn extraction_request(messages: Vec<ChatMessage>, limit: u32) -> Result<ChatRequest, u32> {
    let tokens = request_tokens(&messages);
    if tokens > limit {
        return Err(tokens);
    }

    // **温度は明示する**（ADR-0038 §決定2 / 作業単位 A-7d）。**思考させない**（ADR-0059 §決定1）。
    // **値の来歴と、変容と同じ値であることの意味は `llm::SUB_LLM_TEMPERATURE` の doc**（裏方の要求の口 /
    // `llm::ChatRequest::background`）。
    Ok(ChatRequest::background(
        messages,
        EXTRACTION_RESPONSE_TOKENS,
    ))
}

/// **実機で腕を比べる器具のための入口。テスト専用**（作業単位 21a）。
///
/// `tests/extraction_quality_harness.rs` が
/// **温度・応答の上限・§8 の門を本番と同じ経路で**通すために使う。
/// **器具の側に写しを作らせない**のが目的なので、
/// **ここで値を組み立て直さないこと**（[`extraction_request`] へ委譲するだけ）。
///
/// # ⚠ **器具は落とし所の上限で回る**（作業単位 B-7）
///
/// 器具は `App` を起動しないので、**引き直しの器を持っていない。**
/// 通るのは [`extraction_prompt_budget_for_tests`] と同じ
/// [`FALLBACK_EXTRACTION_PROMPT_TOKENS`] で、**予算と門の2つは互いに一致している。**
/// 実機のサブモデルが 16384 でロードされている限り、本番と同じ数になる。
#[cfg(any(test, feature = "testing"))]
pub fn extraction_request_for_tests(messages: Vec<ChatMessage>) -> Result<ChatRequest, u32> {
    extraction_request(messages, extraction_prompt_budget_for_tests())
}

/// **依頼全体に許すトークン数。テスト専用**（作業単位 21a）。
///
/// `build_extraction_prompt` の `budget` に渡す数。
/// **器具が数を写すと、本番が変わったときに器具だけが古びる。**
///
/// **⚠ 返るのは落とし所の側の上限**（[`extraction_request_for_tests`] の doc）。
#[cfg(any(test, feature = "testing"))]
#[must_use]
pub const fn extraction_prompt_budget_for_tests() -> u32 {
    FALLBACK_EXTRACTION_PROMPT_TOKENS
}

/// **サブモデルの落とし所のコンテキスト長。テスト専用**（作業単位 G-2 の是正）。
///
/// # なぜ入口を生やすのか
///
/// [`ASSUMED_SUB_CONTEXT_TOKENS`] は `pub(in crate::server)` なので、
/// **統合テストからは名前で呼べない。** そこで
/// 「上流の申告値は**落とし所とは別の数**であること」という前提を、
/// **たまたま同じ値を持つ別のテスト定数**と突き合わせる形が書かれていた
/// （`tests/server_context_refresh.rs` / G-2 のレビューが指摘）。
/// **そちらを別の値に変えた日に、コメントが主張している被覆が黙って消える。**
///
/// **突き合わせる相手は、この値そのものでなければならない。**
#[cfg(any(test, feature = "testing"))]
#[must_use]
pub const fn assumed_sub_context_tokens_for_tests() -> u32 {
    SubContext::FALLBACK.tokens()
}

/// **依頼に載せた記憶と、予算に入らなかった記憶の件数**（ADR-0021 §13 / 作業単位 13c-3b）。
///
/// **2つの `usize` を並べて渡さない。** 取り違えても型は通り、
/// ログの数だけが静かに入れ替わる（`memory` の側は何も変わらないので気付けない）。
#[derive(Debug, Clone, Copy)]
struct KnownMemoryCounts {
    /// 依頼の「すでに知っていること」の節に載った件数。
    included: usize,
    /// 予算に入らず落とした件数。**捨てるなら数える**（ADR-0006 と同じ作法）。
    dropped: usize,
}

impl KnownMemoryCounts {
    /// 組み上がった依頼から読む。**数えるのは `prompt` 側**（配分の責務があちらにある）。
    const fn of(prompt: &ExtractionPrompt) -> Self {
        Self {
            included: prompt.included_memories(),
            dropped: prompt.dropped_memories(),
        }
    }
}

/// **抽出1回につき1イベント**（破棄率の観測点 / ADR-0021 §3）。
///
/// 載せるのは**件数と定数ラベルだけ。** 記憶の本文もサブLLM の生出力も載せない
/// （調査材料は `message.raw_content` の側にある）。
///
/// # 水準が2つある理由
///
/// **`domain` の検証で弾いた分がある回は `warn`。**
/// `MemoryContent::parse` / `MemoryCategory::parse` は純粋関数なので `tracing` を持てず、
/// **呼び出し側が出さないと痕跡がまったく残らない**
/// （ADR-0021 §3 / docs/06-roadmap.md「メモリ抽出パイプラインの受け入れ条件」）。
/// 上限や空で弾かれた事実は「プロンプトかモデルを直すべき」というシグナルなので、
/// 既定のフィルタでも目立つ側に出す。
///
/// **欄は両方の水準でまったく同じ。** 片方にだけ欄を足すと、集計が水準で割れる。
/// **割れる先は「`domain` が弾いた行があった回」＝ いちばん読みたい回**なので、
/// この約束は doc ではなく見張り番で実行可能にしてある——
/// `markdown_emphasis_is_dropped_before_saving_and_counted_in_the_log`（`emphasis_stripped`）と
/// `a_quoted_line_is_saved_and_the_stripped_quote_is_counted_in_the_log`（`quotes_stripped`）と
/// `a_reply_that_copies_the_prompt_example_is_saved_and_counted`（`example_echoes`）と
/// `a_reply_that_copies_the_prompt_example_format_is_saved_and_counted`
/// （`example_format_echoes`）と
/// `the_known_memory_counts_are_recorded_at_the_warn_level_too`
/// （`known_included` / `known_dropped`）が `warn` 側の欄を押さえる。
/// **欄を増やしたら、`warn` 側を見る検査も一緒に増やすこと。**
///
/// # `emphasis_stripped` は破棄とは別の欄（ADR-0021 §11 / 13c-1）
///
/// Markdown の強調記号を落とした行は**捨てていない**ので、`discarded` に足さない。
/// 破棄率は既に定義された指標で、**過去の測定値と比べるために使っている**——
/// 別の意味の数を混ぜると、以後の比較が全部無効になる。
/// この欄が無いと、プロンプトに「Markdown を使うな」と書いたときに
/// **効いたかどうかが分からない**（落とした後の `memory` からは復元できない）。
///
/// # `quotes_stripped` も別の欄（作業単位 B-29 / C-15）
///
/// **行頭の `> ` を落とした行数。** 落とした行は**捨てていない**ので `discarded` に足さず、
/// 測っている現象が別なので `emphasis_stripped` にも足さない——
/// あちらは「モデルが Markdown を吐く頻度」、こちらは
/// **「モデルが依頼の引用書式を答えへ持ち込む頻度」**である。
///
/// この欄が無いと、**是正が効いたのか、モデルが書かなくなったのかが区別できない**
/// （落とした後の `memory` からは元が `> ` 付きだったか復元できない）。
/// C-15 では **272 観測のうち 15（5.5%）**でこれが起きて、
/// **そのターンの記憶が丸ごと 0 件になっていた。**
///
/// # `example_echoes` も別の欄（作業単位 B-28 / C-15）
///
/// **依頼の手本の文が、上流の生出力に現れた回数**（`prompt::extraction::example_echoes`）。
///
/// **⚠ これは防いでいない。数えているだけである。** 手本の文が混ざった記憶は
/// **いまもそのまま保存される**——捨てない理由はあちらの doc（要約すると
/// **手本は「本物の記憶に見えるように」選ばれていて、実際の葵は喫茶店の店主**なので、
/// 「苦い珈琲が好き」は本物でありうる）。
///
/// **C-15 では 816 観測のうち 2 回**で、**うち 1 件がそのまま保存された。**
/// **その 2 件は形が違う**（片方は主語が無くて捨てられている）ので、
/// **数えるのは生出力の側**——保存された記憶を見ると半分しか見えない。
///
/// # `example_format_echoes` も別の欄（作業単位 B-33 / C-15）
///
/// **手本が見せている「分類の書式」（`[好み]` の角括弧）が、
/// 保存される本文に居座った回数**（`prompt::extraction::example_format_echoes`）。
///
/// **⚠ これも防いでいない。数えているだけである。**
///
/// **`example_echoes` に混ぜない。** あちらは**手本の本文**しか見ないので、
/// `[好み]` だけが末尾に付いた行を **0 と数える**。
/// C-15 の同じデータで、**本文の漏れは 1 行、書式の居座りは 29 行**だった。
///
/// **⚠ 見る先も違う。**
///
/// | | 見る先 | 捨てられた行 |
/// |---|---|---|
/// | `example_echoes` | **生出力** | **数える** |
/// | `example_format_echoes` | **保存される本文** | **数えない** |
///
/// 後者の実害は**永久に残ること**なので、`memory` へ到達した分だけを数える
/// （理由は `prompt::extraction::example_format_echoes` の doc）。
///
/// **欄名はソースに直書きした識別子**で、LLM の出力を反射しない
/// （`DiscardReason::label` と同じ扱い）。
///
/// # `known_included` / `known_dropped` も別の欄（ADR-0021 §13 / 13c-3b）
///
/// 「すでに知っていること」の節に**載せた件数**と、**予算に入らなかった件数**。
///
/// **`discards` にも `emphasis_stripped` にも混ぜない。** どれも別の意味の指標で、
/// 混ぜると破棄率が過去の測定値と比べられなくなる（上の節と同じ理由）。
/// 落ちたのは**記憶の一覧**であって抽出の材料ではないので、
/// **破棄ではなく控除リストの充足度**を表す。
///
/// **2つとも出す。** 入らなかった件数だけでは
/// 「載せた分がどれだけあったか」が分からず、**充足度の分母が作れない**
/// （破棄の欄に 0 件の理由まで並べるのと同じ理由）。
///
/// # `effective_drift_mode` / `effective_drift_source` も別の欄（ADR-0045 / 作業単位 A-8）
///
/// **その往復に効いた変容の走らせ方と、その出どころ**（[`TurnJob::drift_mode`]）。
/// **走らせなかった回（[`record_skip`]）にも同じ2つを載せる。**
///
/// 変容は `off` の回に**1行も出さない**（ADR-0026）ので、
/// 起動時の既定が `on` なのにキャラクターが `off` で走っていない、という取り違えは
/// **変容の側のログからは「行が無い」としか読めない。**
/// **抽出の行は毎往復に1本出る**（走った回か、走らなかった回のどちらか）ので、ここに載せる。
///
/// # ⚠ 綴りは変容の記録の欄と**わざと別にしてある**
///
/// 変容の記録（`server::drift` の `record_run`）にも同じ意味の2つの欄があるが、
/// **その綴りをこのファイルに書くと `server::drift` の
/// `the_drift_fields_never_collide_with_another_module` が落ちる**——
/// 欄名で grep すると抽出の行まで数えてしまう（16b の実測）ので、あの走査は緩めない。
/// **こちらは `effective_` で始まる別の綴り**にしてある。
#[expect(
    clippy::too_many_arguments,
    reason = "欄はどれも別の意味の数で、束ねると『どれのものか』が読めなくなる（この関数の doc）"
)]
fn record_run(
    job: &TurnJob,
    extracted: usize,
    saved: usize,
    discards: &DiscardCounts,
    emphasis_stripped: usize,
    quotes_stripped: usize,
    example_echoes: usize,
    example_format_echoes: usize,
    known: KnownMemoryCounts,
) {
    let discarded = discards.total();
    let from_domain = discards
        .iter()
        .filter(|(reason, _)| reason.from_domain())
        .fold(0_usize, |sum, (_, count)| sum.saturating_add(count));

    if from_domain > 0 {
        tracing::warn!(
            extracted,
            saved,
            discarded,
            emphasis_stripped,
            quotes_stripped,
            example_echoes,
            example_format_echoes,
            known_included = known.included,
            known_dropped = known.dropped,
            discards = %DiscardField(discards),
            effective_drift_mode = %job.drift_mode.mode().label(),
            effective_drift_source = %job.drift_mode.source().label(),
            session_id = job.session_id.get(),
            "{RAN_MESSAGE}"
        );
    } else {
        tracing::info!(
            extracted,
            saved,
            discarded,
            emphasis_stripped,
            quotes_stripped,
            example_echoes,
            example_format_echoes,
            known_included = known.included,
            known_dropped = known.dropped,
            discards = %DiscardField(discards),
            effective_drift_mode = %job.drift_mode.mode().label(),
            effective_drift_source = %job.drift_mode.source().label(),
            session_id = job.session_id.get(),
            "{RAN_MESSAGE}"
        );
    }
}

/// **走らせなかった回も数える**（ADR-0021 §7 / §8）。
///
/// 「0 件だった」と**別のイベント**にしてある。同じ行にすると、
/// 破棄率の分母に「そもそも走っていない回」が混ざり、
/// **「本文が忙しすぎる」というシグナルが読めなくなる。**
///
/// `prompt_tokens` は長すぎて捨てた回だけ載る。**上限と並べて出す**
/// ——どれだけ超えたのかが分からないと、上限が現実に合っているかを判断できない。
///
/// # `cause` がある回は `warn`（作業単位 13c-3b）
///
/// 「忙しい」「長すぎる」は**設計どおりの結果**なので `info` のままにする。
/// **記憶を読めなかった回だけが故障**で、原因は `#[source]` 側にしか無い
/// （`StoreError` の `Display` は定数文字列 / 絶対制約）。
/// **`error` と `detail` はここにしか出ない**ので、落とすと調査の手掛かりが消える。
/// 見張り番は `a_store_failure_while_reading_memories_keeps_its_cause_in_the_log`。
/// **`InvalidStoredValue` では測れない**（`Display` も `Debug` も定数相当）ので、
/// あちらは `StoreError::Query`（プールを閉じる）で測っている。
///
/// **集計キー（`skipped`）と件数の欄は水準によらず同じ。** 水準で欄が割れると、
/// 走らなかった回の内訳が読めなくなる。
fn record_skip(
    reason: SkipReason,
    job: &TurnJob,
    prompt_tokens: Option<u32>,
    cause: Option<&StoreError>,
) {
    let prompt_tokens = prompt_tokens.unwrap_or_default();
    // **その回に効いていた上限**（作業単位 B-7）。**定数ではなく往復の材料から出す**
    // ——引き直しで動くので、定数を出すと「上限を超えた」と言いながら
    // 超えていない数を並べる行になる。
    // 見張り番は [`the_budget_and_the_log_follow_the_shared_window_too`]。
    let limit_tokens = job.sub_context.prompt_limit();
    match cause {
        Some(err) => tracing::warn!(
            skipped = %reason.label(),
            prompt_tokens,
            limit_tokens,
            error = %err,
            detail = ?err,
            effective_drift_mode = %job.drift_mode.mode().label(),
            effective_drift_source = %job.drift_mode.source().label(),
            session_id = job.session_id.get(),
            "{SKIPPED_MESSAGE}"
        ),
        None => tracing::info!(
            skipped = %reason.label(),
            prompt_tokens,
            limit_tokens,
            effective_drift_mode = %job.drift_mode.mode().label(),
            effective_drift_source = %job.drift_mode.source().label(),
            session_id = job.session_id.get(),
            "{SKIPPED_MESSAGE}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::runtime::Runtime;
    use tokio::sync::oneshot;

    use super::*;
    use crate::domain::{
        ChatMessage, Handle, Memory, MemoryContent, NewMemory, Persona, StatScale, Subject,
    };
    use crate::llm::{ChatStream, Completion, FinishReason, LlmError, LlmFuture};
    use crate::parse::extraction::DiscardReason;
    use crate::prompt::extraction::{
        EXAMPLE_CHARACTER_LINE, EXAMPLE_CHARACTER_ONLY_LINE, EXAMPLE_USER_LINE,
    };
    use crate::testlog::{captured_under_default_filter, has_field, has_key, line_containing};

    /// 利用者の発言。**応答とも記憶とも別の文字列。**
    const USER_MESSAGE: &str = "ただいま。今日は遅くなった。";

    /// キャラクターの応答。
    const REPLY: &str = "「おかえりなさい」 彼女は本を閉じた。";

    /// 抽出結果（利用者について）。**プロンプトの手本とは別の文**
    /// （手本をそのまま使うと「例を拾って保存する」実装と区別が付かない）。
    const USER_FACT: &str = "山あいの町で育った";

    /// 抽出結果（キャラクター自身について）。**利用者側とは別の文。**
    const CHARACTER_FACT: &str = "雨の日は店を早く閉める";

    /// **上限で切れた最後の行の中身**（作業単位 24 / ADR-0021 §17）。
    ///
    /// 「十年前から通っている」が途中で切れたもの。
    /// **崩れていない**——`- 主語: 本文` の形を完全に満たすので、
    /// 打ち切りの印を見なければ破棄にも現れずに保存される。
    /// **崩れた断片で検査すると `malformed` が偶然拾い、この穴が残ったまま緑になる。**
    const FRAGMENT_FACT: &str = "十年前から通って";

    /// **会話する**キャラクターの識別子。
    const HANDLE: &str = "aoi";

    /// **会話する**キャラクターの呼び名。**識別子とは別の綴り**（ADR-0011）。
    const DISPLAY_NAME: &str = "葵";

    /// **会話しない**キャラクターの識別子。会話する方とは別の綴り。
    const BYSTANDER_HANDLE: &str = "kaede";

    /// **会話しない**キャラクターの呼び名。他の3つとは別の綴り。
    const BYSTANDER_DISPLAY_NAME: &str = "楓";

    /// **会話しない**キャラクターが最初から持っている記憶。
    ///
    /// 抽出結果のどれとも別の本文にしてある（同値だと、どちらの行なのか区別が付かない）。
    const BYSTANDER_FACT: &str = "港の見える坂道に住んでいる";

    /// 会話するキャラクターに**あらかじめ入れておく記憶**の件数（作業単位 13c-3b）。
    ///
    /// **載る件数・落ちる件数と全部別の数**にしてある（3 / 4 / 7）。
    /// 同値にすると、2つの欄を取り違える変異が観測できない
    /// （CLAUDE.md「フィクスチャが2つの値を同値にすると…」）。
    const SEEDED_MEMORIES: usize = 7;

    /// 記憶1件あたりの詰め物の文字数。**`𠮷` は 1 文字 4 バイト**（見積もりはバイト数）。
    ///
    /// 1件 ≒ 1731 トークンになるよう選んである（下の [`FITTING_MEMORIES`] の doc）。
    const KNOWN_MEMORY_FILLER_CHARS: usize = 425;

    /// 予算に収まる記憶の件数（**実測 / 2026-08-22**）。
    ///
    /// # この数はフィクスチャの設計値であって、規則ではない
    ///
    /// 実測: 削れない枠 ＝ **2079** トークン（呼び名 `葵` ＋ [`USER_MESSAGE`] ＋ [`REPLY`]）、
    /// 取り分 ＝ 8192 − 2079 ＝ **6113**、記憶1件 ＝ **1732**、節の枠 ＝ **242**。
    /// 累計は 1974 / 3706 / 5438 / 7170 なので **3 件で止まる。**
    ///
    /// **両側に 600 トークン以上の余裕を取ってある**（5438 と 7170 の間に 6113 がある。
    /// 下へ 675 / 上へ 1057）ので、指示を数行足した程度では動かない。
    /// 動いたときは下の見張り番が「載るはずの目印が無い」で落ちるので、
    /// **黙って古びることはない。**
    ///
    /// **`prompt` 側の関数から期待値を作らない**のは意図的——
    /// 期待値を関数から作ると、**関数が緩んだときに期待値も一緒に動いて緑のまま**になる
    /// （CLAUDE.md「写しを1つにしても、その1つは自動的には守られない」）。
    const FITTING_MEMORIES: usize = 3;

    /// 予算に入らず落ちる記憶の件数。
    const DROPPED_MEMORIES: usize = SEEDED_MEMORIES - FITTING_MEMORIES;

    /// **取り分を使い切るためのフィクスチャ**の件数（作業単位 13c-3b の是正）。
    ///
    /// 上の [`SEEDED_MEMORIES`] は1件が大きすぎて（1732 トークン）、
    /// 詰め終わっても**取り分が 675 余る。** そのため
    /// 「**予算いっぱいまで詰めた依頼が §8 の門を通るか**」という状態に
    /// どのテストも一度も入っていなかった（実測 / 2026-08-22。
    /// `budget` を上限＋1000 にする変異が 1076 passed / 0 failed で緑）。
    ///
    /// **34 件は「必要な数より多い」側に取ってある。** 予算を +1000 した回に載るのは
    /// **27 件**（実測 / 依頼 9041 − 枠 2079 ＝ 6962 ＝ 節の枠 242 ＋ 24×249 ＋ 3×248）なので、
    /// 変異させても**候補が尽きない。**
    const FILLING_MEMORIES: usize = 34;

    /// [`FILLING_MEMORIES`] の1件あたりの詰め物の文字数（`𠮷` は1文字4バイト）。
    ///
    /// **1件 ＝ 248〜249 トークン**（実測 / 2026-08-22。目印の桁数で1バイト違う）。
    /// **細かさそのものが見張り番の分解能**——1件が大きいほど、
    /// 予算を動かしても載る件数が変わらない領域が広くなる。
    /// 249 は [`EXTRACTION_RESPONSE_TOKENS`]（320。**作業単位 24 までは 512**）より
    /// 小さいので、**予算をその分だけ減らす変異は必ず載る件数を動かす**
    /// （512 で測ったときは 23 件 → 21 件）。
    const FILLING_MEMORY_FILLER_CHARS: usize = 54;

    /// 「予算を使い切った」と言える余裕（トークン）。
    ///
    /// 詰め方は貪欲で、**打ち切るのは次の1件が入らないとき**なので、
    /// 余りは必ず**1件ぶんの費用**（[`FILLING_MEMORY_FILLER_CHARS`] の doc / 249）より小さい。
    /// **その1件ぶんを上回る最小の切りのよい数**にしてある——
    /// 大きくすると、予算を減らす変異を見逃す側へ緩む。
    const BUDGET_FILL_MARGIN: u32 = 256;

    /// 抽出の依頼で**利用者の発言**が載る小見出し。
    ///
    /// **`prompt::extraction` の定数を呼ばず、テスト側に期待値として持つ**
    /// （呼ぶと綴りを変える変異が両側で同時に動いて緑になる）。
    /// 綴りが変われば [`assert_each_turn_sits_under_its_own_heading`] が
    /// 「小見出しが載ること」で落ちる。
    const USER_TURN_HEADING: &str = "## 利用者の発言";

    /// 抽出の依頼で**キャラクターの応答**が載る小見出し。利用者側とは別の綴り。
    const CHARACTER_TURN_HEADING: &str = "## あなたの発言";

    /// 抽出用の上流の代役。**受けた依頼を控え、台本どおりに返す。**
    struct MockSub {
        requests: Mutex<Vec<ChatRequest>>,
        replies: Mutex<VecDeque<String>>,
        /// `Some` なら**開くまで返さない。**
        gate: Mutex<Option<oneshot::Receiver<()>>>,
        /// 上流の失敗を演じるか。
        fails: bool,
        /// 応答に添える停止理由（作業単位 24）。**`None` は「上流が申告しなかった」。**
        finish_reason: Option<FinishReason>,
        calls: AtomicUsize,
    }

    impl MockSub {
        fn replying(replies: &[&str]) -> Arc<Self> {
            Self::replying_with(replies, Some(FinishReason::Stop))
        }

        /// **停止理由まで指定して返す代役**（作業単位 24）。
        ///
        /// 既定（[`MockSub::replying`]）は `stop` ——**書き切った**回。
        /// 打ち切られた回を演じるには `Some(FinishReason::Length)` を渡す。
        fn replying_with(replies: &[&str], finish_reason: Option<FinishReason>) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                replies: Mutex::new(replies.iter().map(|text| (*text).to_owned()).collect()),
                gate: Mutex::new(None),
                fails: false,
                finish_reason,
                calls: AtomicUsize::new(0),
            })
        }

        fn failing() -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                replies: Mutex::new(VecDeque::new()),
                gate: Mutex::new(None),
                fails: true,
                finish_reason: Some(FinishReason::Stop),
                calls: AtomicUsize::new(0),
            })
        }

        fn gated() -> (Arc<Self>, oneshot::Sender<()>) {
            let (open, gate) = oneshot::channel();
            let mock = Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                replies: Mutex::new(VecDeque::new()),
                gate: Mutex::new(Some(gate)),
                finish_reason: Some(FinishReason::Stop),
                fails: false,
                calls: AtomicUsize::new(0),
            });
            (mock, open)
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        /// 最後の依頼を1つの文字列にしたもの。
        fn last_prompt(&self) -> String {
            let requests = self.requests.lock().expect("ロックを取れること");
            let last = requests.last().expect("依頼を1件以上受けていること");
            last.messages
                .iter()
                .map(|message| message.content.clone())
                .collect::<Vec<_>>()
                .join("\n")
        }

        /// 最後の依頼のメッセージ列。**送ったものそのものを数えるため**
        /// （[`MockSub::last_prompt`] の畳んだ文字列では、メッセージ1件ごとの
        /// 上乗せ分が勘定から落ちる）。
        fn last_request(&self) -> Vec<ChatMessage> {
            let requests = self.requests.lock().expect("ロックを取れること");
            requests
                .last()
                .expect("依頼を1件以上受けていること")
                .messages
                .clone()
        }

        fn last_max_tokens(&self) -> Option<u32> {
            let requests = self.requests.lock().expect("ロックを取れること");
            requests
                .last()
                .expect("依頼を1件以上受けていること")
                .max_tokens
        }

        /// 最後の依頼の温度。**21a-2 で足した**——それまで誰も見ていなかった。
        ///
        /// 温度は `extraction_request` が決める3つのつまみのうちの1つで、
        /// **実機の器具（`tests/extraction_quality_harness.rs`）の検出力の源**でもある
        /// （**温度を `0` にすると**同じ入力の繰り返しが同じ出力になり、
        /// `PERSONACORE_HARNESS_REPS` が意味を失う。
        /// **A-7d で明示にした値は 0.8 なので、そこは崩れていない**）。
        /// 見張り番は [`the_test_only_entry_points_hand_back_what_production_sends`]。
        fn last_temperature(&self) -> Option<f32> {
            let requests = self.requests.lock().expect("ロックを取れること");
            requests
                .last()
                .expect("依頼を1件以上受けていること")
                .temperature
        }
    }

    impl LlmClient for MockSub {
        fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            self.requests
                .lock()
                .expect("ロックを取れること")
                .push(request.clone());
            self.calls.fetch_add(1, Ordering::SeqCst);
            let gate = self.gate.lock().expect("ロックを取れること").take();

            Box::pin(async move {
                if let Some(gate) = gate {
                    let _ = gate.await;
                }
                if self.fails {
                    return Err(LlmError::MissingContent);
                }
                let content = self
                    .replies
                    .lock()
                    .expect("ロックを取れること")
                    .pop_front()
                    .unwrap_or_default();
                Ok(Completion {
                    content,
                    // **台本どおりの停止理由**（作業単位 24）。
                    // 決め打ちにすると、打ち切られた回を演じられない。
                    finish_reason: self.finish_reason,
                    usage: None,
                })
            })
        }

        fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
            Box::pin(async move { Err(LlmError::MissingContent) })
        }
    }

    /// **現在のスレッドで走らせる runtime。**
    ///
    /// ログの捕捉はスレッドローカルなので、別スレッドへ飛ばすと
    /// `captured_under_default_filter` から抽出タスクの記録が見えなくなる
    /// （`super::relay` の単体テストと同じ作法）。
    fn runtime() -> Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime を作れること")
    }

    /// キャラクターを1件作って読み直す。
    async fn create_character(store: &Store, handle: &str, display_name: &str) -> Character {
        let handle = Handle::parse(handle).expect("正当な名前であること");
        store
            .create_character(
                &handle,
                &DisplayName::parse(display_name).expect("正当な表示名であること"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
                None,
                StatScale::parse(7).expect("正当な尺度であること"),
            )
            .await
            .expect("キャラクターを作成できること");
        store
            .find_character_by_handle(&handle)
            .await
            .expect("キャラクターを引けること")
            .expect("作成済みであること")
    }

    /// 傍観者に最初から入れておく記憶。
    fn bystander_memory() -> NewMemory {
        NewMemory {
            subject: Subject::Character,
            content: MemoryContent::parse(BYSTANDER_FACT).expect("正当な本文であること"),
            category: None,
        }
    }

    /// **会話するキャラクターに「すでに知っていること」を [`SEEDED_MEMORIES`] 件入れる。**
    ///
    /// 戻り値は目印の一覧で、**先頭が最新**（依頼に載る順）。
    /// 1件ずつ順に書くので、`id` は入れた順に増える——`created_at` が同じ秒に並んでも
    /// 「新しい順」は決まる（`prompt::extraction::fit_known_memories`）。
    ///
    /// **本文は1件ずつ別**にしてある。同じにすると「どの1件が載ったか」が読めない。
    async fn seed_known_memories(store: &Store, character_id: CharacterId) -> Vec<String> {
        seed_memories(
            store,
            character_id,
            SEEDED_MEMORIES,
            KNOWN_MEMORY_FILLER_CHARS,
        )
        .await
    }

    /// 記憶を `count` 件、1件あたり `filler_chars` 文字の詰め物つきで入れる。
    ///
    /// **件数と大きさの両方を呼び出し側が決める。** 大きい記憶を数件入れる形
    /// （[`seed_known_memories`]）と、小さい記憶を多数入れて取り分を使い切る形
    /// （[`a_request_that_fills_its_budget_with_memories_still_passes_the_gate`]）の
    /// **両方が要る**——後者でなければ「予算が門より大きい」状態が観測できない。
    async fn seed_memories(
        store: &Store,
        character_id: CharacterId,
        count: usize,
        filler_chars: usize,
    ) -> Vec<String> {
        let mut markers = Vec::with_capacity(count);
        for index in 0..count {
            let marker = format!("{index}番目の記録");
            let body = format!("{marker}{}", "\u{20BB7}".repeat(filler_chars));
            let memory = NewMemory {
                subject: Subject::User,
                content: MemoryContent::parse(&body).expect("正当な本文であること"),
                category: None,
            };
            store
                .add_memory(character_id, &memory)
                .await
                .expect("記憶を入れられること");
            markers.push(marker);
        }
        markers.reverse();
        markers
    }

    /// **記憶の持ち主を取り違えたら分かる形の DB。**
    ///
    /// # なぜキャラクターが2件要るのか
    ///
    /// 1件しか作らないと `character.id` と `1` が**常に同値**になり、
    /// **持ち主を決め打ちにする実装**（`CharacterId::from_row(1)`）が原理的に観測できない
    /// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
    /// 実測で、`Extraction::new` の `character_id` を `1` へ決め打つ変異は、
    /// 2件目を足すまで **1029/1029 緑**で通り抜けた。
    ///
    /// 取り違えると、実機では **`aoi` との会話から抽出した記憶が別キャラの行として入る。**
    /// 想起の入口は `Store::recall_memories(character_id)` だけなので、
    /// もう一方が他人の事実を自分のこととして語り始める。
    ///
    /// > **これは修復の導線でも直しきれない**（20b でレビューと judgment をやり直した）。
    /// > `Store::supersede_memory` は**同じ持ち主の下にしか**訂正版を入れないので、
    /// > **間違って入った側の想起からは外せるが、正しい持ち主へ移す手段は無い。**
    /// > 「訂正版で上書きできる」他の誤り（中身・主語）とは**性質が違う。**
    struct Fixture {
        store: Store,
        /// **会話する方。** 後から作るので `id` は 1 ではない。
        character: Character,
        /// **会話しない方。** ここの記憶が増えたら取り違え。
        ///
        /// **あらかじめ1件入れてある。** 「0 件であること」だと、
        /// **持ち主が誰であれ空を返す**実装と区別が付かない。
        bystander: Character,
        session_id: SessionId,
    }

    /// キャラクターを**2件**持つ DB と、その配線の材料（[`Fixture`] の doc を参照）。
    async fn fresh(path: &std::path::Path) -> Fixture {
        let store = Store::connect(path).await.expect("DB を開けること");

        // **会話しない方を先に作る**（＝ `id` が 1 になるのはこちら）。
        let bystander = create_character(&store, BYSTANDER_HANDLE, BYSTANDER_DISPLAY_NAME).await;
        let character = create_character(&store, HANDLE, DISPLAY_NAME).await;

        assert_ne!(character.id, bystander.id, "前提: 2件の id が別であること");
        assert_ne!(
            character.handle.as_str(),
            bystander.handle.as_str(),
            "前提: 2件の識別子が別であること"
        );
        assert_ne!(
            character.display_name.as_str(),
            bystander.display_name.as_str(),
            "前提: 2件の呼び名が別であること"
        );
        assert_ne!(
            character.id,
            CharacterId::from_row(1),
            "前提: 会話する方の id が 1 でないこと（決め打ちと同値にしない）"
        );

        store
            .add_memory(bystander.id, &bystander_memory())
            .await
            .expect("傍観者の記憶を入れられること");

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

    /// 記憶を `(主語, 本文)` の組にして並べる。
    fn as_pairs(memories: &[Memory]) -> Vec<(Subject, String)> {
        memories
            .iter()
            .map(|memory| (memory.subject, memory.content.as_str().to_owned()))
            .collect()
    }

    /// 行の中の `キー=数` を数として読む。**欄が無ければ落とす。**
    ///
    /// `Option` を返して比較すると、**欄が消えた変異が「`None` == `None`」で緑**になる。
    ///
    /// # ⚠ **渡すのは行**（作業単位 B-21d）
    ///
    /// 理由は [`discard_count`] と同じ。**[`ran_record`] / [`skip_record`] で切ってから渡す。**
    fn field_number(line: &str, key: &str) -> usize {
        let prefix = format!("{key}=");
        line.split_whitespace()
            .find_map(|token| token.strip_prefix(&prefix))
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("{key} が数として載ること: {line:?}"))
    }

    /// **最後の1行が断片になっている応答**（作業単位 24）。
    ///
    /// 3行とも別の内容で、**1行目と3行目は主語も同じ**にしてある——
    /// 「最初の1行を落とす」実装は、残る2件の中身が変わるので赤になる。
    fn truncated_reply() -> String {
        format!("- 利用者: {USER_FACT}\n- あなた: {CHARACTER_FACT}\n- 利用者: {FRAGMENT_FACT}")
    }

    /// ログの `discards=` 欄から、その理由の件数を読む（作業単位 24）。
    ///
    /// **`contains("truncated=1")` では足りない**——`truncated=10` にも当たる。
    /// 欄は `理由=件数` をカンマで繋いだ1トークン（[`DiscardField`]）。
    ///
    /// # ⚠ **渡すのは行**（作業単位 B-21d）
    ///
    /// `split_whitespace` は**改行をまたぐ**ので、捕捉した塊を渡すと主張が
    /// 「**その欄を持つ行が、出力のどこかに1本ある**」に化ける
    /// （`crate::testlog` の `has_field` の doc / [`ran_record`] の doc）。
    /// **[`ran_record`] / [`skip_record`] で切ってから渡すこと。**
    fn discard_count(line: &str, reason: DiscardReason) -> usize {
        let prefix = format!("{}=", reason.label());
        line.split_whitespace()
            .find_map(|token| token.strip_prefix("discards="))
            .unwrap_or_else(|| panic!("破棄の欄が載ること: {line:?}"))
            .split(',')
            .find_map(|entry| entry.strip_prefix(&prefix))
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("{} の件数が載ること: {line:?}", reason.label()))
    }

    // ---------------------------------------------------------------------
    // 記録の**行**を選ぶ（作業単位 B-21d / B-21c を `server::extract` へ）
    // ---------------------------------------------------------------------

    /// **抽出を走らせた回の記録**（[`record_run`]）の**行**を1本取り出す。
    ///
    /// # なぜ捕捉した塊のまま欄を見てはいけないのか
    ///
    /// `crate::testlog` の `has_field` は `split_whitespace` で欄を切るので
    /// **改行をまたぐ。** 捕捉した出力を丸ごと渡すと、主張は
    /// 「**その欄がその値を持つ行が、出力のどこかに1本ある**」に化けており、
    /// 「**その行がこう言っている**」を一切見ていない
    /// （CLAUDE.md「その行が出たと、その行がこう言っているは別の主張である」）。
    ///
    /// **机上の話ではない。** `server::drift` で実演されている（B-21c）——
    /// 判定の記録の欄名を潰す変異は赤になるが、**飛ばし記録の枝へ同じ欄を1本足すと
    /// 緑に戻る。** ここでも同じ形が作れる：この3つのイベントは**同じターンで並んで出る**
    /// （[`a_response_without_a_finish_reason_is_recorded`] の出力は
    /// [`RAN_MESSAGE`] と [`NO_FINISH_REASON_MESSAGE`] の2本を持つ）。
    ///
    /// # 鍵は**イベント名**であって、主張したい欄ではない
    ///
    /// [`RAN_MESSAGE`] は `tracing` のメッセージ（地の文）なので `line_containing` で選ぶ。
    /// **主張したい欄そのものを鍵にすると循環する**——囮の行を掴んだまま、
    /// その行で主張が成立する。欄を鍵に選ぶ場面では
    /// `crate::testlog` の `line_with_field` を使うこと。
    ///
    /// # 同じイベントが2本あれば、**最初の1本**を返す
    ///
    /// `lines()` を先頭から探すので、**同じ捕捉窓で2回抽出を走らせた**フィクスチャでは
    /// 1回目の記録になる。**向きを未規定のままにしない**
    /// （`crate::testlog` の `line_with_field` と同じ扱い）。
    ///
    /// # 見張り番は2本
    ///
    /// | 見るもの | 見張り番 |
    /// |---|---|
    /// | 囮の行を掴まないこと（**並びのどの順でも**） | [`a_decoy_event_carrying_the_same_field_is_not_the_extraction_record`] |
    /// | 同じイベントが2本あるとき**先に出た方**を返すこと | [`the_record_tools_return_the_first_line_that_carries_their_event`] |
    ///
    /// # ⚠ **包まない場所が1種類ある**
    ///
    /// **否定**（`!has_field(塊, …)` / `!塊.contains(…)`）は塊のまま渡す
    /// ——「どの行にも無い」という強い主張になるから
    /// （権威は `crate::testlog` の `SharedBuffer::contents` と `has_field` の doc）。
    /// **その理由をここに書き写さないこと**（片方を直すと、もう片方が黙って古びる）。
    ///
    /// [`a_decoy_event_carrying_the_same_field_is_not_the_extraction_record`]:
    ///     a_decoy_event_carrying_the_same_field_is_not_the_extraction_record
    /// [`the_record_tools_return_the_first_line_that_carries_their_event`]:
    ///     the_record_tools_return_the_first_line_that_carries_their_event
    /// [`a_response_without_a_finish_reason_is_recorded`]:
    ///     a_response_without_a_finish_reason_is_recorded
    fn ran_record(output: &str) -> &str {
        line_containing(output, RAN_MESSAGE)
    }

    /// **走らせなかった回の記録**（[`record_skip`]）の**行**を1本取り出す。
    ///
    /// [`ran_record`] と同じ理由・同じ約束（**最初の1本** / 同じ見張り番2本）。
    /// **`session_id` は3つのイベントすべてに出る**ので、
    /// どの行の話かを鍵で決めないと、取り違えても緑のままになる。
    fn skip_record(output: &str) -> &str {
        line_containing(output, SKIPPED_MESSAGE)
    }

    /// **停止理由の申告が無かったことの記録**（[`record_missing_finish_reason`]）の
    /// **行**を1本取り出す。
    ///
    /// [`ran_record`] と同じ理由・同じ約束。**読み手は1本しか無いが、それでも道具にする**
    /// ——3つのイベントの文言は**「抽出」を共有する**ので、
    /// **鍵をそこまで緩める変異**は現実的な形であり、
    /// 3つを同じ見張り番に通しておかないと**緩めた側だけが観測されない。**
    fn no_finish_reason_record(output: &str) -> &str {
        line_containing(output, NO_FINISH_REASON_MESSAGE)
    }

    /// **囮のイベントが同じ欄・同じ値を出しても、そのイベントの記録としては読まない**
    /// （作業単位 B-21d）。
    ///
    /// **この作業単位の存在理由そのもの。**
    ///
    /// # 囮の形は、実演されたものと同じ
    ///
    /// **別のイベントの行が、こちらの欄を持つ**——`server::drift` の B-21c で
    /// レビュアーが実際に1本足した形をそのまま置いてある
    /// （CLAUDE.md「囮は、実際に狙っている相手と同じ形でなければ意味がない」）。
    /// **`extracted=` は本来 [`RAN_MESSAGE`] にしか出ない**ので、
    /// それを他の2本へ載せるのが「同じ形」。
    ///
    /// # ⚠ **並びのどの順でも測る**
    ///
    /// 3つの文言は**「抽出」を共有する**ので、**鍵をそこまで緩める変異**は現実的な形。
    /// **1つの順番でしか測らないと、先頭に来た側だけが赤になる。**
    /// **6通りの並びすべて**を同じ主張に通す。
    ///
    /// # 入力は**植えたソース**（`tracing` を通さない）
    ///
    /// 実際に走らせて捕まえた出力では、この囮の行は作れない
    /// ——**本番のコードが出さない行だから。**
    /// ここは道具（[`ran_record`] / [`skip_record`] / [`no_finish_reason_record`]）を、
    /// 実ログとまったく同じ判定に通すための対照である
    /// （CLAUDE.md「対照は、植えたソースを走査本体に通すこと」）。
    ///
    /// # 期待値は道具を通さないべた書き。**否定は3つの道具すべてに置く**
    ///
    /// 肯定だけだと、**塊をそのまま返す実装が囮の行で満たされて緑になる**
    /// ——B-21c のレビューが「道具の片方に権威が1本も無かった」として実測した形
    /// （`assert_ne!` は「同じ行を返している」しか見ておらず、
    /// 「**行を返している**」を見ていないので、そこも通り抜ける）。
    ///
    /// **塊から欄を取り出す道具**（[`discard_count`] / [`field_number`]）も
    /// 同じ経路に通す——あちらも `split_whitespace` で改行をまたぐ。
    #[test]
    fn a_decoy_event_carrying_the_same_field_is_not_the_extraction_record() {
        // **走った回の記録**（本物）。**囮として、飛ばした回の欄も持たせる。**
        let ran = format!(
            "2026-08-29T00:00:00.000000Z  INFO planted::ran: \
             {RAN_MESSAGE} extracted=2 discards=malformed=0,truncated=1 \
             skipped=busy session_id=1"
        );
        // **飛ばした回の記録。囮として、走った回にしか出ない `extracted=` を持たせる。**
        // **3つに共通する欄**（`session_id`）は値をわざとずらす
        // ——同じ値にすると、行を取り違えても観測できない
        // （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
        let skipped = format!(
            "2026-08-29T00:00:00.000001Z  INFO planted::skip: \
             {SKIPPED_MESSAGE} skipped=too_long extracted=9 \
             discards=malformed=9,truncated=9 session_id=2"
        );
        // **申告漏れの記録。こちらにも他の2つの欄を持たせる。**
        let missing = format!(
            "2026-08-29T00:00:00.000002Z  WARN planted::none: \
             {NO_FINISH_REASON_MESSAGE} extracted=7 skipped=no_memories \
             discards=malformed=7,truncated=7 session_id=3"
        );

        // **6通りの並びすべて。** 先頭に来た側だけが赤になる形にしない。
        for order in [
            [&ran, &skipped, &missing],
            [&ran, &missing, &skipped],
            [&skipped, &ran, &missing],
            [&skipped, &missing, &ran],
            [&missing, &ran, &skipped],
            [&missing, &skipped, &ran],
        ] {
            let planted = format!("{}\n{}\n{}\n", order[0], order[1], order[2]);

            // **前提: 塊のまま見れば、どの欄も相手の行から手に入る。**
            // ここが偽なら、下の主張は「そもそも紛れが無い」を見ているだけになる。
            for field in [
                "session_id=1",
                "session_id=2",
                "session_id=3",
                "extracted=2",
                "extracted=9",
                "extracted=7",
                "skipped=busy",
                "skipped=too_long",
                "skipped=no_memories",
            ] {
                assert!(
                    has_field(&planted, field),
                    "前提: {field} が塊の中に在ること: {planted:?}"
                );
            }

            // ---- 走った回 ----
            let observed = ran_record(&planted);
            assert!(
                has_field(observed, "session_id=1"),
                "走った回の記録の行を選べていない: {observed:?}"
            );
            assert!(
                !has_field(observed, "session_id=2") && !has_field(observed, "session_id=3"),
                "3つに共通する欄で、別のイベントの側を読んでいる: {observed:?}"
            );
            assert!(
                !has_field(observed, "extracted=9") && !has_field(observed, "extracted=7"),
                "囮の行を走った回の記録として読んでいる: {observed:?}"
            );
            assert!(
                !has_field(observed, "skipped=too_long"),
                "飛ばした回の理由が読めている（行に切れていない）: {observed:?}"
            );
            // **塊から欄を取り出す道具も、同じ行の中で答えること。**
            assert_eq!(
                field_number(observed, "extracted"),
                2,
                "件数を囮の行から読んでいる: {observed:?}"
            );
            assert_eq!(
                discard_count(observed, DiscardReason::Truncated),
                1,
                "破棄の内訳を囮の行から読んでいる: {observed:?}"
            );

            // ---- 飛ばした回 ----
            let observed = skip_record(&planted);
            assert!(
                has_field(observed, "session_id=2"),
                "飛ばした回の記録の行を選べていない: {observed:?}"
            );
            assert!(
                !has_field(observed, "session_id=1") && !has_field(observed, "session_id=3"),
                "3つに共通する欄で、別のイベントの側を読んでいる: {observed:?}"
            );
            assert!(
                !has_field(observed, "skipped=busy") && !has_field(observed, "skipped=no_memories"),
                "囮の行を飛ばした回の記録として読んでいる: {observed:?}"
            );
            assert_eq!(
                field_number(observed, "extracted"),
                9,
                "件数を囮の行から読んでいる: {observed:?}"
            );

            // ---- 申告漏れ ----
            let observed = no_finish_reason_record(&planted);
            assert!(
                has_field(observed, "session_id=3"),
                "申告漏れの記録の行を選べていない: {observed:?}"
            );
            assert!(
                !has_field(observed, "session_id=1") && !has_field(observed, "session_id=2"),
                "3つに共通する欄で、別のイベントの側を読んでいる: {observed:?}"
            );
            assert!(
                !has_field(observed, "extracted=2") && !has_field(observed, "extracted=9"),
                "囮の行を申告漏れの記録として読んでいる: {observed:?}"
            );

            // **行そのものと突き合わせる**（べた書き / 道具を通さない期待値）。
            assert_eq!(
                ran_record(&planted),
                ran,
                "走った回の記録の行そのものを返すこと: {planted:?}"
            );
            assert_eq!(
                skip_record(&planted),
                skipped,
                "飛ばした回の記録の行そのものを返すこと: {planted:?}"
            );
            assert_eq!(
                no_finish_reason_record(&planted),
                missing,
                "申告漏れの記録の行そのものを返すこと: {planted:?}"
            );
        }
    }

    /// **同じイベントが2本あれば、道具は「先に出た方」を返す**（作業単位 B-21d）。
    ///
    /// # なぜ別に要るのか
    ///
    /// 囮の見張り番は**各イベントが1本ずつ**の植えたソースを通すので、
    /// **「同じマーカーに候補が2本ある」状態に一度も入らない**
    /// （CLAUDE.md「フィクスチャがその分岐が効く状態に一度も入らない」）。
    /// **後ろから探す変異（`.rev()`）も、末尾を返す変異も、あちらでは緑を通る**
    /// ——B-21c のレビューが実測した形。
    ///
    /// 作法は `crate::testlog` の `line_with_field` の押さえと同じ ——
    /// **同じマーカーを持つ行を2本置き、期待値は道具を通さないべた書き。**
    ///
    /// # この状態は本番のフィクスチャで起きうる
    ///
    /// **1つの捕捉窓で抽出を2回走らせる入口が実在する**
    /// （[`finding_nothing_is_a_different_event_from_not_running`] の2本目は
    /// 同じ窓で [`Extraction::start`] を2回呼ぶ）。
    ///
    /// [`finding_nothing_is_a_different_event_from_not_running`]:
    ///     finding_nothing_is_a_different_event_from_not_running
    #[test]
    fn the_record_tools_return_the_first_line_that_carries_their_event() {
        let first_ran =
            format!("2026-08-29T00:00:00.000000Z  INFO planted::turn1: {RAN_MESSAGE} session_id=1");
        let first_skip = format!(
            "2026-08-29T00:00:00.000001Z  INFO planted::turn1: {SKIPPED_MESSAGE} session_id=1"
        );
        let first_missing = format!(
            "2026-08-29T00:00:00.000002Z  WARN planted::turn1: \
             {NO_FINISH_REASON_MESSAGE} session_id=1"
        );
        let second_ran =
            format!("2026-08-29T00:00:00.000003Z  INFO planted::turn2: {RAN_MESSAGE} session_id=2");
        let second_skip = format!(
            "2026-08-29T00:00:00.000004Z  INFO planted::turn2: {SKIPPED_MESSAGE} session_id=2"
        );
        let second_missing = format!(
            "2026-08-29T00:00:00.000005Z  WARN planted::turn2: \
             {NO_FINISH_REASON_MESSAGE} session_id=2"
        );
        let planted = format!(
            "{first_ran}\n{first_skip}\n{first_missing}\n\
             {second_ran}\n{second_skip}\n{second_missing}\n"
        );

        assert_eq!(
            ran_record(&planted),
            first_ran,
            "走った回の記録が2本あるとき、先に出た方を返していない: {planted:?}"
        );
        assert_eq!(
            skip_record(&planted),
            first_skip,
            "飛ばした回の記録が2本あるとき、先に出た方を返していない: {planted:?}"
        );
        assert_eq!(
            no_finish_reason_record(&planted),
            first_missing,
            "申告漏れの記録が2本あるとき、先に出た方を返していない: {planted:?}"
        );
    }

    /// 依頼の中での位置。**見つからなければ落とす**（`Option` の比較で黙って通さない）。
    fn position_of(prompt: &str, needle: &str) -> usize {
        prompt
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} が依頼に載ること: {prompt:?}"))
    }

    /// **2つの発言が、それぞれ自分の小見出しの下に載っていること。**
    ///
    /// # `contains` では取り違えを捕まえられない
    ///
    /// 入れ替えても**どちらの文字列も依頼の「どこかには」在る**ので、
    /// 「載っていること」しか見ない主張は両方とも満たされる。実測で、
    /// `TurnSink::persist` が [`Extraction::start`] に渡す2つの引数を入れ替える変異は
    /// **1029/1029 緑**で通り抜けた。だから**位置**（小見出しとの前後関係）で見る。
    ///
    /// 取り違えるとサブモデルには**役割が反転した会話**が渡り、
    /// 抽出される事実の主語が系統的に反転する（ADR-0021 §2 / CLAUDE.md
    /// 「`memory.subject` を省略しない」）。
    ///
    /// > **行は消えないが、直せる**（20b で判定をやり直した）——
    /// > 主語を正した訂正版を `Store::supersede_memory` で入れれば、
    /// > 誤った行は `superseded_by IS NULL` の外へ出て**想起から外れる。**
    /// > **ただし直すのは人で、しかも1件ずつ**。系統的に反転していれば全件が対象になる。
    /// > **「行を潰さない」と「直せない」を混同しないこと。**
    fn assert_each_turn_sits_under_its_own_heading(prompt: &str, user_message: &str, reply: &str) {
        assert_ne!(
            user_message, reply,
            "前提: 2つの発言は別の文字列であること（同値だと取り違えが観測できない）"
        );

        let user_heading = position_of(prompt, USER_TURN_HEADING);
        let character_heading = position_of(prompt, CHARACTER_TURN_HEADING);
        let user_at = position_of(prompt, user_message);
        let reply_at = position_of(prompt, reply);

        assert!(
            user_heading < user_at && user_at < character_heading,
            "利用者の発言は `{USER_TURN_HEADING}` の下に載ること: {prompt:?}"
        );
        assert!(
            character_heading < reply_at,
            "応答は `{CHARACTER_TURN_HEADING}` の下に載ること: {prompt:?}"
        );
    }

    /// **抽出が上流へ届くまで待つ。締切を超えたら落とす。**
    ///
    /// # `yield_now` 1回では届かない（作業単位 13c-3b）
    ///
    /// タスクは上流を呼ぶ前に `Store::recall_memories` を `await` する（§13）。
    /// DB の応答は別スレッドから返るので、**待つ回数は機械の速さで決まる。**
    /// 以前の `yield_now().await` 1回は「そこまでは1回で進む」という
    /// **実装ではなく機械についての主張**になっていた（実測で 0 件のまま先へ進んだ）。
    ///
    /// **肯定側（起きること）は締切まで poll する**——`tests/` の作法と同じ。
    async fn wait_until_the_upstream_is_called(sub: &MockSub) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while sub.call_count() == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "抽出が上流へ届かないまま締切を過ぎた"
            );
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }

    /// モックを差した抽出器。**`Arc<dyn LlmClient>` への変換をここに集める。**
    ///
    /// **変容は off。** このモジュールのテストが見ているのは抽出だけで、
    /// on にすると上流の呼び出し回数に変容のぶんが混ざる
    /// （変容の見張り番は `super::super::drift` の側にある）。
    fn extractor_of(store: &Store, sub: &Arc<MockSub>) -> Arc<MemoryExtractor> {
        extractor_with_sub_context(
            store,
            sub,
            super::super::context::SharedSubContext::fallback_for_tests(),
        )
    }

    /// **起動時の既定の走らせ方を指定した抽出器**（ADR-0045 / 作業単位 A-8）。
    fn extractor_defaulting_to(
        store: &Store,
        sub: &Arc<MockSub>,
        default: StatDriftMode,
    ) -> Arc<MemoryExtractor> {
        let llm: Arc<dyn LlmClient> = Arc::clone(sub) as Arc<dyn LlmClient>;
        Arc::new(MemoryExtractor::new(
            store.clone(),
            llm,
            super::super::context::SharedSubContext::fallback_for_tests(),
            default,
            DriftForm::Single,
            DriftNote::None,
            DriftTuning::default(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ))
    }

    /// **抽出の記録の行が、その往復に効いた変容の走らせ方と出どころを名乗る**
    /// （ADR-0045 §結果の ⚠ / 作業単位 A-8）。
    ///
    /// # 何を追えるようにするのか
    ///
    /// **起動時の既定が `on` でも、キャラクターが `off` なら変容は1行も出さない。**
    /// 変容の側のログからは「行が無い」としか読めないので、
    /// **毎往復に1本出る抽出の行**に、効いた値と出どころを載せる。
    ///
    /// # 腕（**どれも同じ行の中で見る**）
    ///
    /// | 既定 | キャラクター | 見る行 | 期待 |
    /// |---|---|---|---|
    /// | `on` | `off` | 走った回 | `off` / `character`（**指示された腕**） |
    /// | `shadow` | 未設定 | 走った回 | `shadow` / `default`（**定数に決め打ちする実装を止める対照**） |
    /// | `off` | `on` | 走らなかった回（長すぎる） | `on` / `character`（**走らなかった回の行にも載る**） |
    /// | `on` | `shadow` | 走った回・**`warn` の水準** | `shadow` / `character` |
    /// | `on` | 未設定 | 走らなかった回・**記憶が読めない（`warn`）** | `on` / `default` |
    ///
    /// **4本目と5本目が要る。** `record_run` と `record_skip` は、どちらも水準で分岐して
    /// 同じ欄を2箇所に書いている。**片方の枝から欄を落とす変異は、もう片方の枝だけを見る検査を緑で通る**
    /// （`the_known_memory_counts_are_recorded_at_the_warn_level_too` の doc の実測と同じ形）。
    /// **5本目は A-8 の変異 N04（原因ありの `record_skip` から2欄を落とす）が 4 本では
    /// 全緑・exit 0 を通ったので足した。**
    #[test]
    fn the_extraction_record_names_the_drift_mode_that_applied_to_its_turn() {
        /// 入力の形。
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum Turn {
            /// 普通に走る（`info` の水準）。
            Info,
            /// 長すぎて走らない（走らなかった回の行）。
            TooLong,
            /// `domain` が弾く行が混ざる（`warn` の水準）。
            Warn,
            /// 記憶が読めずに走らない（原因つきの `warn` の水準 / プールを閉じる）。
            Unreadable,
        }
        let long_message = "あ".repeat(FALLBACK_EXTRACTION_PROMPT_TOKENS as usize);
        let warn_reply = format!(
            "- 利用者: {}\n- あなた: {CHARACTER_FACT}",
            "あ".repeat(MemoryContent::MAX_CHARS + 1)
        );
        /// （既定, キャラクター, 入力の形, 期待する（走らせ方, 出どころ））。
        type Case = (
            StatDriftMode,
            Option<StatDriftMode>,
            Turn,
            (&'static str, &'static str),
        );
        let cases: [Case; 5] = [
            (
                StatDriftMode::On,
                Some(StatDriftMode::Off),
                Turn::Info,
                ("off", "character"),
            ),
            (
                StatDriftMode::Shadow,
                None,
                Turn::Info,
                ("shadow", "default"),
            ),
            (
                StatDriftMode::Off,
                Some(StatDriftMode::On),
                Turn::TooLong,
                ("on", "character"),
            ),
            (
                StatDriftMode::On,
                Some(StatDriftMode::Shadow),
                Turn::Warn,
                ("shadow", "character"),
            ),
            (StatDriftMode::On, None, Turn::Unreadable, ("on", "default")),
        ];

        for (default, chosen, turn, (mode, source)) in cases {
            let too_long = turn == Turn::TooLong;
            let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
            let runtime = runtime();
            let fixture = runtime.block_on(fresh(&dir.path().join("mode.sqlite")));
            if turn == Turn::Unreadable {
                // **プールを閉じる**（＝ `recall_memories` が `StoreError::Query` を返す状態 /
                // `a_store_failure_while_reading_memories_keeps_its_cause_in_the_log` と同じ手）。
                runtime.block_on(fixture.store.close());
            }
            let reply = if turn == Turn::Warn {
                warn_reply.as_str()
            } else {
                ""
            };
            let sub = MockSub::replying(&[reply]);
            let extractor = extractor_defaulting_to(&fixture.store, &sub, default);
            // **往復の入口が読むのはキャラクターの行**（`Extraction::new`）。
            let character = Character {
                stat_drift_mode: chosen,
                ..fixture.character.clone()
            };
            let extraction = Extraction::new(Arc::clone(&extractor), &character);
            let message = if too_long {
                long_message.as_str()
            } else {
                USER_MESSAGE
            };

            let output = captured_under_default_filter(|| {
                runtime.block_on(async {
                    extraction.start(
                        fixture.session_id,
                        crate::domain::MessageId::from_row(2),
                        message,
                        REPLY,
                    );
                    extractor.wait_for_the_running_task().await;
                });
            });
            runtime.block_on(fixture.store.close());

            let line = if too_long || turn == Turn::Unreadable {
                skip_record(&output)
            } else {
                ran_record(&output)
            };
            // **前提: 狙った水準の枝を通っていること**（同じ行の中で見る）。
            assert_eq!(
                line.contains("WARN"),
                matches!(turn, Turn::Warn | Turn::Unreadable),
                "前提: 水準が狙いどおりであること（{turn:?}）: {line:?}"
            );
            if turn == Turn::Unreadable {
                assert!(
                    has_field(line, "skipped=memories_unreadable"),
                    "前提: 記憶が読めずに走らなかった回であること: {line:?}"
                );
            }
            assert!(
                has_field(line, &format!("effective_drift_mode={mode}"))
                    && has_field(line, &format!("effective_drift_source={source}")),
                "既定 = {default:?} / キャラクター = {chosen:?}: {line:?}"
            );
        }
    }

    /// **サブモデルのコンテキスト長の器を指定した抽出器**（作業単位 B-7）。
    ///
    /// 上限（§8 の門と `prompt` へ渡す予算）が**この器から出ている**ことを見るための入口。
    fn extractor_with_sub_context(
        store: &Store,
        sub: &Arc<MockSub>,
        sub_context: super::super::context::SharedSubContext,
    ) -> Arc<MemoryExtractor> {
        let llm: Arc<dyn LlmClient> = Arc::clone(sub) as Arc<dyn LlmClient>;
        Arc::new(MemoryExtractor::new(
            store.clone(),
            llm,
            sub_context,
            StatDriftMode::Off,
            DriftForm::Single,
            DriftNote::None,
            DriftTuning::default(),
            crate::domain::AxisDiscoveryMemories::DEFAULT,
            crate::domain::AxisTrialRule::DEFAULT,
        ))
    }

    /// 1ターンの上限（`usize`）。
    fn per_turn_limit() -> usize {
        usize::try_from(NewMemory::MAX_PER_TURN).expect("上限が `usize` に収まること")
    }

    /// **1回分の抽出を最後まで走らせ、記録と保存された記憶を返す。**
    ///
    /// `Extraction::start` は spawn するだけなので、タスクの完了を待ってから読む。
    fn extract_once(sub: Arc<MockSub>, user_message: &str, reply: &str) -> (String, Vec<Memory>) {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();

        // **準備は観測窓の外で済ませる**（キャラクター作成の記録を混ぜない）。
        let fixture = runtime.block_on(fresh(&dir.path().join("extract.sqlite")));
        let extractor = extractor_of(&fixture.store, &sub);
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    user_message,
                    reply,
                );
                extractor.wait_for_the_running_task().await;
            });
        });

        let memories = runtime
            .block_on(fixture.store.recall_memories(fixture.character.id))
            .expect("記憶を読めること");

        // **会話しなかった方は1件も増えていないこと**（持ち主の決め打ちを捕まえる）。
        // ここに置くのは、この入口を通るすべての回で見るため。
        let bystander = runtime
            .block_on(fixture.store.recall_memories(fixture.bystander.id))
            .expect("傍観者の記憶を読めること");
        assert_eq!(
            as_pairs(&bystander),
            vec![(Subject::Character, BYSTANDER_FACT.to_owned())],
            "会話していないキャラクターに記憶を入れないこと（持ち主を決め打ちにしない）"
        );

        runtime.block_on(fixture.store.close());
        (output, memories)
    }

    /// **抽出している最中にキャラクターが完全に消されたら、記憶は捨てて `error` にしない**
    /// （ADR-0049 §決定3 / 作業単位 A-10）。
    ///
    /// 上流が返す前に（門で止めて）しまってから消すので、保存は必ず会話の縛りで落ちる
    /// （往復の会話がカスケードで消えている / `StoreError::TurnSessionGone`）。
    /// **故障の行（「抽出した記憶を保存できませんでした」）を出さず**、
    /// 捨てたことは `info` の [`VANISHED_MESSAGE`] に `vanished_write=extracted_memory` で残る。
    /// 抽出を1回行った記録（[`RAN_MESSAGE`]）は今までどおり出て、保存は 0 件。
    #[test]
    fn a_character_deleted_while_extracting_loses_the_memories_without_an_error() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("vanished.sqlite")));

        let (sub, open) = MockSub::gated();
        sub.replies
            .lock()
            .expect("ロックを取れること")
            .push_back(format!("- 利用者: {USER_FACT}"));
        let extractor = extractor_of(&fixture.store, &sub);
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                // **上流が依頼を受けた（＝既知の記憶は読み終わった）ところで消す。**
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                while sub.call_count() < 1 {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "抽出の依頼が上流へ届くこと"
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
                open.send(()).expect("上流を開けること");
                extractor.wait_for_the_running_task().await;
            });
        });

        assert!(
            !output.contains("抽出した記憶を保存できませんでした"),
            "消えたキャラクターへの保存の失敗を故障として出さないこと: {output}"
        );
        assert!(
            !output.contains("ERROR"),
            "error の行を1本も出さないこと: {output}"
        );
        let vanished = line_containing(&output, VANISHED_MESSAGE);
        assert!(
            vanished.contains(" INFO ") && has_field(vanished, "vanished_write=extracted_memory"),
            "捨てたことは info で残すこと: {vanished:?}"
        );
        let ran = line_containing(&output, RAN_MESSAGE);
        assert_eq!(
            field_number(ran, "extracted"),
            1,
            "前提: 1件読めていること: {ran:?}"
        );
        assert_eq!(field_number(ran, "saved"), 0, "保存は 0 件: {ran:?}");

        runtime.block_on(fixture.store.close());
    }

    /// **消したキャラクターと同じ id で作り直されても、抽出した記憶は新しいキャラクターに入らない**
    /// （ADR-0049 §決定3 / 作業単位 A-10 のレビュー前の差し戻し）。
    ///
    /// `character.id` は AUTOINCREMENT ではないので、最大の id のキャラクターを消してすぐ作ると
    /// **同じ id になる。** 外部キーは通ってしまうので、書き込みを**往復の会話**
    /// （`session.id` は使い回されない）で縛ってある（`Store::add_memory_in_session`）。
    ///
    /// **id が実際に同じであることを前提として確かめる** —— 使い回されなければ、
    /// このテストは何も見ていない。新しいキャラクターにも会話を1つ作っておく
    /// （「会話が1つでも在れば書く」形の縛りを通さないため）。
    #[test]
    fn memories_of_a_deleted_character_never_reach_a_new_character_with_the_same_id() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("reused-id.sqlite")));

        let (sub, open) = MockSub::gated();
        sub.replies
            .lock()
            .expect("ロックを取れること")
            .push_back(format!("- 利用者: {USER_FACT}"));
        let extractor = extractor_of(&fixture.store, &sub);
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
                while sub.call_count() < 1 {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "抽出の依頼が上流へ届くこと"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                // **上流が返す前に、しまって消して、同じ識別子で作り直す。**
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
                let created = create_character(&fixture.store, HANDLE, DISPLAY_NAME).await;
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
        let memories = runtime
            .block_on(fixture.store.recall_memories(reborn.id))
            .expect("記憶を読めること");
        assert!(
            memories.is_empty(),
            "消したキャラクターの記憶が、同じ id の新しいキャラクターに入らないこと: {memories:?}"
        );
        assert!(
            !output.contains("ERROR"),
            "error の行を1本も出さないこと: {output}"
        );
        let vanished = line_containing(&output, VANISHED_MESSAGE);
        assert!(
            has_field(vanished, "vanished_write=extracted_memory"),
            "捨てたことを残すこと: {vanished:?}"
        );

        runtime.block_on(fixture.store.close());
    }

    /// **抽出できた回と、走らなかった回は別のイベント**（ADR-0021 §7 / §8）。
    ///
    /// **0 件は正常な結果**であって「走らなかった」ではない。同じ行にすると
    /// 破棄率の分母に走っていない回が混ざり、
    /// **「本文が忙しすぎる」というシグナルが読めなくなる。**
    #[test]
    fn finding_nothing_is_a_different_event_from_not_running() {
        // 1. **走ったが 0 件**（上流が空を返した）。
        let (output, memories) = extract_once(MockSub::replying(&[""]), USER_MESSAGE, REPLY);

        assert!(memories.is_empty(), "0 件であること: {memories:?}");
        assert!(
            output.contains(RAN_MESSAGE),
            "走った回として記録すること: {output:?}"
        );
        // **欄は走った回の記録の行の中で見る**（作業単位 B-21d / [`ran_record`] の doc）。
        assert!(
            has_field(ran_record(&output), "extracted=0"),
            "0 件だったことが読めること: {output:?}"
        );
        assert!(
            !output.contains(SKIPPED_MESSAGE),
            "0 件を「走らなかった」と記録しないこと: {output:?}"
        );

        // 2. **走らなかった**（前の抽出が走っている / §7）。
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        // **傍観者は見ない**（このテストが見ているのは走らせ方だけ / [`Fixture`]）。
        let Fixture {
            store,
            character,
            session_id,
            ..
        } = runtime.block_on(fresh(&dir.path().join("busy.sqlite")));
        let (sub, _open) = MockSub::gated();
        let extractor = extractor_of(&store, &sub);
        let extraction = Extraction::new(Arc::clone(&extractor), &character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                // 1本目は開かないので走り続ける。
                extraction.start(
                    session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
                // 上流に届くところまで進める（そこで待ちに入る）。
                wait_until_the_upstream_is_called(&sub).await;
                extraction.start(
                    session_id,
                    crate::domain::MessageId::from_row(2),
                    USER_MESSAGE,
                    REPLY,
                );
            });
        });
        extractor.shutdown();
        runtime.block_on(store.close());

        assert_eq!(sub.call_count(), 1, "前提: 1本目だけが上流へ行くこと");
        assert!(
            output.contains(SKIPPED_MESSAGE),
            "走らなかった回として記録すること: {output:?}"
        );
        // **欄は飛ばした回の記録の行の中で見る**（作業単位 B-21d / [`ran_record`] の doc）。
        assert!(
            has_field(
                skip_record(&output),
                &format!("skipped={}", SkipReason::Busy.label())
            ),
            "理由が「忙しい」であること: {output:?}"
        );
        assert!(
            !output.contains(RAN_MESSAGE),
            "走らなかった回を「抽出しました」と記録しないこと: {output:?}"
        );
        // **否定は塊のまま見る**（どの行にも載っていないことを主張したい）。
        //
        // **⚠ 道具は [`has_key`]**（作業単位 B-21d のレビュー指摘 / 2026-08-29）。
        // ここは `!has_field(&output, "extracted=0")` と書いてあったが、
        // **それは「値が 0 ではない」しか言っていない** ——
        // **`record_skip` に `extracted = 1` を1本足す変異が全緑を通った。**
        // 文面（「件数の欄を出さないこと」）と実装がずれていた形。
        // **素の `contains("extracted=")` にも替えられない**（`x_extracted=` を掴んで
        // 実装が正しいのに落ちる / `has_field` の doc の表）。
        assert!(
            !has_key(&output, "extracted"),
            "走らなかった回に件数の欄を出さないこと（0 件と区別できなくなる）: {output:?}"
        );
    }

    /// **同時に走らせるのは1件まで。捨てて数える**（ADR-0021 §7）。
    ///
    /// **待ち行列にしない。** 並べても停止時に落ちるうえ、溜まっていることが見えない。
    #[test]
    fn a_second_extraction_while_one_runs_is_dropped_not_queued() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        // **傍観者は見ない**（このテストが見ているのは走らせ方だけ / [`Fixture`]）。
        let Fixture {
            store,
            character,
            session_id,
            ..
        } = runtime.block_on(fresh(&dir.path().join("one-at-a-time.sqlite")));
        let (sub, open) = MockSub::gated();
        let extractor = extractor_of(&store, &sub);
        let extraction = Extraction::new(Arc::clone(&extractor), &character);

        runtime.block_on(async {
            extraction.start(
                session_id,
                crate::domain::MessageId::from_row(2),
                USER_MESSAGE,
                REPLY,
            );
            wait_until_the_upstream_is_called(&sub).await;
            // 走っている間に2件・3件と来ても、増えるのは破棄の数だけ。
            extraction.start(
                session_id,
                crate::domain::MessageId::from_row(2),
                USER_MESSAGE,
                REPLY,
            );
            extraction.start(
                session_id,
                crate::domain::MessageId::from_row(2),
                USER_MESSAGE,
                REPLY,
            );

            // 1本目を開けて終わらせる。**並んでいれば、ここで2本目が走り出す。**
            open.send(()).expect("上流を開けること");
            extractor.wait_for_the_running_task().await;
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
        });
        runtime.block_on(store.close());

        assert_eq!(
            sub.call_count(),
            1,
            "捨てた分を後から走らせないこと（待ち行列にしない / §7）"
        );
    }

    /// **会話が長すぎたら、切り詰めずに捨てる**（ADR-0021 §8）。
    ///
    /// 断片から抽出すると、**誤った事実が恒久的に残る**（`memory` に更新も削除も無い）。
    /// **本文のプロンプトとは逆の判断**——あちらは削っても残りが正しい。
    ///
    /// 見ているのは「上流へ1度も行かないこと」。**切り詰めて呼ぶ実装は、
    /// 呼んだ時点でここが赤になる。**
    ///
    /// **しきい値そのものはここでは固定していない**（入力は上限の3倍の大きさ）。
    /// 判定の境界は `a_request_one_byte_over_the_limit_is_dropped` と
    /// `a_request_exactly_at_the_limit_is_sent` が1バイト単位で挟む。
    #[test]
    fn a_conversation_over_the_limit_is_dropped_whole_and_counted() {
        // 上限を確実に超える会話（`estimate_tokens` は UTF-8 バイト数以上）。
        let long_message = "あ".repeat(FALLBACK_EXTRACTION_PROMPT_TOKENS as usize);
        let sub = MockSub::replying(&[&format!("- 利用者: {USER_FACT}")]);
        let (output, memories) = extract_once(Arc::clone(&sub), &long_message, REPLY);

        assert_eq!(sub.call_count(), 0, "長すぎる会話は上流へ送らないこと");
        assert!(
            memories.is_empty(),
            "断片から記憶を作らないこと: {memories:?}"
        );
        // **理由も上限も、捨てたことを告げる行の中で見る**
        // （作業単位 B-21d / [`skip_record`] の doc）。
        let skipped = skip_record(&output);
        assert!(
            has_field(skipped, &format!("skipped={}", SkipReason::TooLong.label())),
            "捨てたことを理由つきで数えること: {skipped:?}"
        );
        assert!(
            has_field(
                skipped,
                &format!("limit_tokens={FALLBACK_EXTRACTION_PROMPT_TOKENS}")
            ),
            "上限を並べて出すこと（超過分が読めないと上限を見直せない）: {skipped:?}"
        );
    }

    /// **上限に収まる会話は、丸ごと依頼に載る**（上の対照 / 切り詰めていない証拠）。
    ///
    /// 会話の**先頭と末尾の両方**が依頼に載っていることを見る。
    /// 片方だけだと、前から切る実装と後ろから切る実装のどちらかが緑を通る。
    #[test]
    fn a_conversation_within_the_limit_is_sent_whole() {
        /// 会話の先頭に置く目印。**末尾の目印とは別の文字列。**
        const HEAD: &str = "はじめの目印";
        /// 会話の末尾に置く目印。
        const TAIL: &str = "おわりの目印";

        assert_ne!(HEAD, TAIL, "前提: 2つの目印は別の文字列であること");

        // 上限には収まるが**短くはない**会話（切り詰めが起きるなら、ここで起きる）。
        let filler = "あ".repeat(500);
        let message = format!("{HEAD}{filler}{TAIL}");
        let sub = MockSub::replying(&[""]);
        let (_output, _memories) = extract_once(Arc::clone(&sub), &message, REPLY);

        assert_eq!(sub.call_count(), 1, "上限内なら上流へ行くこと");
        let prompt = sub.last_prompt();
        assert!(
            prompt.contains(HEAD) && prompt.contains(TAIL),
            "会話を切り詰めずに載せること: {prompt:?}"
        );
        // **「応答も載ること」だけでは取り違えを捕まえられない**（ヘルパの doc）。
        // 発言ごとに、正しい側の小見出しの下に在ることまで見る。
        assert_each_turn_sits_under_its_own_heading(&prompt, &message, REPLY);
        assert_eq!(
            sub.last_max_tokens(),
            Some(EXTRACTION_RESPONSE_TOKENS),
            "抽出の応答にも上限を送ること（喋り続ける上流に GPU を握らせない）"
        );
    }

    /// [`extract_once`] が組むはずの依頼を、テスト側で同じ形に組んだもの。
    ///
    /// **呼び名はフィクスチャと同じ [`DISPLAY_NAME`]** ——
    /// 違う値にすると境界の計算が実機の依頼とずれる
    /// （`a_request_exactly_at_the_limit_is_sent` が実物と突き合わせて捕まえる）。
    fn expected_request(user_message: &str, character_message: &str) -> Vec<ChatMessage> {
        let display_name = DisplayName::parse(DISPLAY_NAME).expect("正当な表示名であること");
        build_extraction_prompt(&ExtractionInput {
            display_name: &display_name,
            user_message,
            character_message,
            known_memories: &[],
            budget: FALLBACK_EXTRACTION_PROMPT_TOKENS,
            // **本番の呼び出し側と同じ綴りを直書きする。**
            // `ExampleStyle` を本番から引くと、本番の綴りが
            // 変わったとき期待値も一緒に動いて緑のまま通る（作業単位 22a）。
            example_style: ExampleStyle::Balanced,
        })
        .into_messages()
    }

    /// 依頼を [`MockSub::last_prompt`] と同じ形に畳んだもの。
    fn joined(messages: &[ChatMessage]) -> String {
        messages
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 依頼がちょうど [`FALLBACK_EXTRACTION_PROMPT_TOKENS`] に届く利用者の発言。
    ///
    /// # **[`estimate_tokens`] が UTF-8 バイト数であることに依存している**
    ///
    /// 「会話に1バイト足すと依頼が1トークン増える」という関係を使い、
    /// **枠（会話の中身以外）を実行時に測って**残りを1バイト文字で埋める。
    /// 枠を直書きすると、指示を1行足した日に静かに嘘になる。
    ///
    /// **見積もりの定義が変わったら、この計算も一緒に変わる。**
    /// 前提は下の `assert_eq!` が固定しているので、定義が変われば**ここが赤くなる。**
    fn user_message_filling_the_limit() -> String {
        let one = tokens_of_request("a", REPLY);
        let two = tokens_of_request("aa", REPLY);
        assert_eq!(
            two,
            one + 1,
            "前提: 会話の1バイトが依頼の1トークンとして数えられること\
             （estimate_tokens は UTF-8 バイト数）"
        );

        let frame = one - 1;
        assert!(
            frame < FALLBACK_EXTRACTION_PROMPT_TOKENS,
            "前提: 枠だけで上限を超えていないこと: frame = {frame}, \
             limit = {FALLBACK_EXTRACTION_PROMPT_TOKENS}"
        );
        "a".repeat((FALLBACK_EXTRACTION_PROMPT_TOKENS - frame) as usize)
    }

    /// 依頼の大きさ（トークン）。**判定とまったく同じ関数**
    /// （[`crate::prompt::extraction::request_tokens`]）で数える。
    ///
    /// **式をここに書き写さない**——以前は `server` と `prompt` とこのテストの3箇所に
    /// 同じ式があり、どれも互いを見ていなかった（作業単位 13c-3a の是正）。
    fn tokens_of_request(user_message: &str, character_message: &str) -> u32 {
        request_tokens(&expected_request(user_message, character_message))
    }

    /// **上限ちょうどの依頼は送る**（境界の内側 / `>` であって `>=` ではない）。
    ///
    /// # なぜ1バイト単位で押さえるのか
    ///
    /// 対になる2本（「上限の3倍」で捨てる / 「約 3.6k」で送る）は、
    /// **しきい値が 3.6k〜24.7k のどこにあっても緑**だった（約7倍の遊び）。
    /// 実際、判定を `tokens > FALLBACK_EXTRACTION_PROMPT_TOKENS.saturating_mul(2)` に
    /// 緩める変異が 1055 passed / 0 failed で通った（実測 / 2026-08-21）。
    /// 緩んだ実装は 16384 トークンまで送るので、
    /// **窓を超えた依頼を送る**（§8 が避けたい形そのもの。LM Studio では前方から黙って
    /// 切り捨てられる / ADR-0013。llama は 400 `exceed_context_size_error` で断るか、
    /// プロンプト ＋ 応答の上限が窓を超えて応答が黙って短くなる / ADR-0052 M5）。
    ///
    /// **上限を 8192 に上げたぶん遊びはさらに広がった**ので、両側から1バイトで挟む。
    #[test]
    fn a_request_exactly_at_the_limit_is_sent() {
        let message = user_message_filling_the_limit();
        assert_eq!(
            tokens_of_request(&message, REPLY),
            FALLBACK_EXTRACTION_PROMPT_TOKENS,
            "前提: ちょうど上限の依頼を組めていること"
        );

        let sub = MockSub::replying(&[""]);
        let (output, _memories) = extract_once(Arc::clone(&sub), &message, REPLY);

        assert_eq!(
            sub.call_count(),
            1,
            "上限ちょうどは捨てないこと（判定は `>` であって `>=` ではない）: {output:?}"
        );
        assert!(
            !output.contains(SKIPPED_MESSAGE),
            "上限ちょうどを「走らなかった」と数えないこと: {output:?}"
        );
        // **実際に送られた依頼が、境界の計算に使ったものと1バイト違わないこと。**
        // フィクスチャの呼び名が [`DISPLAY_NAME`] と違うと枠がずれるので、ここで突き合わせる。
        assert_eq!(
            sub.last_prompt(),
            joined(&expected_request(&message, REPLY)),
            "実機の依頼が、境界の計算に使った依頼と違う"
        );
    }

    /// **上限を1トークン超えたら捨てる**（境界の外側 / 対は上の1本）。
    ///
    /// 超過分がログに出ることまで見る——**上限が現実に合っているかを判断する材料**が
    /// `prompt_tokens` と `limit_tokens` の並びしかない（`record_skip`）。
    #[test]
    fn a_request_one_byte_over_the_limit_is_dropped() {
        let message = format!("{}a", user_message_filling_the_limit());
        assert_eq!(
            tokens_of_request(&message, REPLY),
            FALLBACK_EXTRACTION_PROMPT_TOKENS + 1,
            "前提: 上限を1トークンだけ超える依頼を組めていること"
        );

        let sub = MockSub::replying(&[&format!("- 利用者: {USER_FACT}")]);
        let (output, memories) = extract_once(Arc::clone(&sub), &message, REPLY);

        assert_eq!(
            sub.call_count(),
            0,
            "1トークン超えたら送らないこと: {output:?}"
        );
        assert!(
            memories.is_empty(),
            "断片から記憶を作らないこと: {memories:?}"
        );
        // **理由も超過分も、捨てたことを告げる行の中で見る**（作業単位 B-21d）。
        let skipped = skip_record(&output);
        assert!(
            has_field(skipped, &format!("skipped={}", SkipReason::TooLong.label())),
            "捨てたことを理由つきで数えること: {skipped:?}"
        );
        // **数えた大きさが本当に上限+1であること**（枠の計算が実機と合っている証拠）。
        assert!(
            has_field(
                skipped,
                &format!("prompt_tokens={}", FALLBACK_EXTRACTION_PROMPT_TOKENS + 1)
            ),
            "超過分が読める形で出ていること: {skipped:?}"
        );
    }

    /// **上限は「サブモデルの実コンテキストの半分」**（[`SubContext::prompt_limit`]）。
    ///
    /// この規則は doc の散文にしかなく、**16384 という測定値もコード上に無かった。**
    /// そのため上限を `262_144`——**doc が「`max_context_length` ではなく」と
    /// 名指しで警告している値そのもの**——に書き換える変異が
    /// 1055 passed / 0 failed で緑を通った（実測 / 2026-08-21）。
    /// 1往復の依頼は実測 2.1k バイト程度なので、**どのテストの入力もこの上限に届かない。**
    ///
    /// # **関係だけでは閉じない——両方の実測値を直書きで押さえる**
    ///
    /// 落とし所の上限は `ASSUMED_SUB_CONTEXT_TOKENS / 2` という**式**なので、
    /// 関係を見るだけの検査は**入力の側（16384）を動かす書き換えを一切止められない**
    /// （`262_144 / 2` でも関係は成り立つ）。
    ///
    /// # **規則は2点で押さえる**（作業単位 B-7）
    ///
    /// 上限が引いた値から出るようになったので、**落とし所の1点だけでは
    /// 「規則」ではなく「その1点での値」しか固定できない**
    /// ——`|tokens| 8192` のような決め打ちが素通りする。
    /// **落とし所とは別の窓**でもう1点見る。
    ///
    /// **サブモデルの落とし所を変えるときは、ここも一緒に直すこと**
    /// （見張り番の意味を変えるので、司令塔の採否を仰ぐ）。
    #[test]
    fn the_fallback_prompt_limit_is_half_of_the_sub_model_context() {
        assert_eq!(
            ASSUMED_SUB_CONTEXT_TOKENS, 16_384,
            "既定のサブモデルの loaded_context_length（実測 / 2026-08-21）から動いた。\
             max_context_length（262144）と取り違えていないか"
        );
        assert_eq!(
            FALLBACK_EXTRACTION_PROMPT_TOKENS,
            ASSUMED_SUB_CONTEXT_TOKENS / 2,
            "上限が「実コンテキストの半分」という規則から外れた"
        );
        assert_eq!(
            FALLBACK_EXTRACTION_PROMPT_TOKENS, 8_192,
            "上限が実測から導かれる数と違う"
        );

        // **2点目。落とし所とは別の窓**（`12_288 / 2 = 6_144`）。
        // ここが無いと、`prompt_limit` を 8192 の決め打ちにする変異が緑を通る。
        let other = SubContext::new(12_288).expect("抽出に使える窓であること");
        assert_eq!(
            other.prompt_limit(),
            6_144,
            "規則は「半分」であって、落とし所での値の決め打ちではない"
        );
        assert_eq!(other.tokens(), 12_288, "窓そのものを書き換えないこと");

        assert!(
            FALLBACK_EXTRACTION_PROMPT_TOKENS.saturating_add(EXTRACTION_RESPONSE_TOKENS)
                <= ASSUMED_SUB_CONTEXT_TOKENS,
            "依頼と応答の合計が実コンテキストを超える（LM Studio は前方から黙って切り捨て、\
             llama は 400 で断るか応答が黙って短くなる）: \
             prompt = {FALLBACK_EXTRACTION_PROMPT_TOKENS}, \
             response = {EXTRACTION_RESPONSE_TOKENS}, \
             context = {ASSUMED_SUB_CONTEXT_TOKENS}"
        );
    }

    /// **抽出に使えない窓は `SubContext` にならない**（作業単位 B-7）。
    ///
    /// 依頼の上限（半分）と応答の上限が**同時に収まらない**値は弾く。
    /// 採ってしまうと、**依頼だけで窓を使い切って応答の分が残らない**
    /// ——LM Studio では前方から黙って切り捨てられるので、消えるのは指示の側（§8 / ADR-0013）。
    /// llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。ただし、
    /// プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
    ///
    /// # 境界は **639**（`640` ではない / レビュー指摘4 / 2026-08-27）
    ///
    /// **上限は切り捨ての割り算**なので、`639 / 2 = 319`、`319 + 320 = 639 <= 639`
    /// ——**奇数の側が1つ手前まで通る。** `2 × EXTRACTION_RESPONSE_TOKENS` と書くと
    /// 1つずれる。**通る最小は 639、落ちる最大は 638。**
    ///
    /// **実害はゼロ**（そんな窓のモデルは存在しない）が、
    /// **doc に書いた数がテストの主張とずれていると、次に読む人が「640 が境界」と
    /// 信じて別の場所で使う。**
    ///
    /// # 期待値を定数から作らない
    ///
    /// 境界（**639**）は**直書き**する。
    /// [`EXTRACTION_RESPONSE_TOKENS`] から組むと、応答の上限を動かす変異が
    /// 期待値を道連れにして緑を通る（CLAUDE.md「16b で定数を倍にする変異が 1283 全緑」）。
    #[test]
    fn a_window_that_cannot_hold_a_request_and_a_reply_is_rejected() {
        assert_eq!(
            EXTRACTION_RESPONSE_TOKENS, 320,
            "前提: 応答の上限が動いた（この検査の境界 639 も測り直すこと）"
        );

        for tokens in [0_u32, 1, 512, 638] {
            assert_eq!(
                SubContext::new(tokens),
                None,
                "tokens = {tokens} は依頼と応答を同時に収められないこと"
            );
        }

        // **境界そのもの。** 切り捨てのぶん、奇数の 639 が通る最小になる。
        let smallest = SubContext::new(639).expect("639 は収まること（319 + 320）");
        assert_eq!(smallest.prompt_limit(), 319);
        // 偶数側の最小も同じ上限。**1つ上が通ることまで見て、境界を挟む。**
        let even = SubContext::new(640).expect("640 も収まること（320 + 320）");
        assert_eq!(even.prompt_limit(), 320);
    }

    /// **抽出の上限は、共有の器から往復ごとに読む**（作業単位 B-7 / ADR-0020）。
    ///
    /// # 高い側と安い側を1本に置く（CLAUDE.md）
    ///
    /// 片側だけだと、**「常に落とし所」も「常に器」も同じ結果**になりうる。
    ///
    /// | 器の窓 | 上限 | この依頼 | 期待 |
    /// |---|---|---|---|
    /// | 狭い（{NARROW}） | その半分 | 超える | **捨てる**（上流を呼ばない） |
    /// | 広い（落とし所） | 8192 | 収まる | **送る** |
    ///
    /// **狭い側は落とし所（16384）とも別の窓**なので、
    /// 「器を読まずに落とし所を使う」実装はここで赤になる。
    ///
    /// # 稼働中の差し替えも同じ器で見る
    ///
    /// 起動時の値を写し取る実装（`spawn` で読まずに `new` で読む）は、
    /// **差し替えた後の1件が古い上限で組まれる**ので赤になる。
    #[tokio::test]
    async fn the_extraction_limit_is_read_from_the_shared_sub_context() {
        /// 狭い側の窓。**落とし所とも、その半分とも別の数。**
        /// 上限は 2048 で、削れない枠（実測 2339）に届かない。
        const NARROW: u32 = 4_096;

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let fixture = fresh(&dir.path().join("sub-context.sqlite")).await;
        let reply_line = format!("- 利用者: {USER_FACT}");
        let sub = MockSub::replying(&[&reply_line]);
        let sub_context = super::super::context::SharedSubContext::for_tests(
            SubContext::new(NARROW).expect("抽出に使える窓であること"),
        );
        assert_ne!(
            SubContext::new(NARROW),
            Some(SubContext::FALLBACK),
            "前提: 狭い側は落とし所と別の窓であること"
        );
        let extractor = extractor_with_sub_context(&fixture.store, &sub, sub_context.clone());

        // **狭い側**——枠だけで上限を超えるので、上流は呼ばれない。
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);
        extraction.start(
            fixture.session_id,
            crate::domain::MessageId::from_row(2),
            USER_MESSAGE,
            REPLY,
        );
        extractor.wait_for_the_running_task().await;
        assert_eq!(
            sub.call_count(),
            0,
            "狭い窓では依頼を捨てること（上流を呼ばない）"
        );

        // **広い側**——引き直しが差し替えたのと同じ効果を起こす。
        sub_context.replace_for_tests(SubContext::FALLBACK);
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);
        extraction.start(
            fixture.session_id,
            crate::domain::MessageId::from_row(2),
            USER_MESSAGE,
            REPLY,
        );
        extractor.wait_for_the_running_task().await;
        assert_eq!(
            sub.call_count(),
            1,
            "広い窓に差し替わったら、次の往復から上流を呼ぶこと"
        );
    }

    /// **応答の上限は、実測した最大の応答のすぐ上に置く**（ADR-0021 §17 / 作業単位 24）。
    ///
    /// # 期待値を [`EXTRACTION_RESPONSE_TOKENS`] から作らない
    ///
    /// 唯一の振る舞いの検査（[`a_conversation_within_the_limit_is_sent_whole`]）は
    /// **定数そのもの**と送った値を比べているので、
    /// **定数を動かす変異は両側が一緒に動いて緑を通る**
    /// （CLAUDE.md「16b で定数を倍にする変異が 1283 全緑」と同じ形）。
    ///
    /// ここは**関数の外にある値**——**2つの実測**——で両側から挟む。
    ///
    /// * **下から**: [`MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS`]（実機で 511 回測った最大）に
    ///   **30% の余裕**を足した数を下回らない。
    ///   割ると、上限に張り付く回が増える——**断片は捨てるので、記憶が静かに減る。**
    /// * **上から**: [`QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS`]（行列の待ち時間の実測から
    ///   導いた数 / ADR-0021 §17）。
    ///
    /// # **上端が「実測の最大の2倍未満」では何も守っていなかった**（作業単位 24-2）
    ///
    /// 24 の上端は 246 × 2 = 492 で、**491 が窓の中に居た**
    /// ——レビューで実測: `EXTRACTION_RESPONSE_TOKENS` を 491 にする変異は
    /// **素の `cargo build` も 1501 テストも全部通った。**
    /// **491 は元の 512 の 96%** で、§17 が測った「余裕2秒」はほぼそのまま失われる。
    /// **上端の理由が行列にあるなら、上端は行列の実測から出さなければならない。**
    ///
    /// # ⚠ これは「中継が切れないこと」の保証ではない
    ///
    /// 上端は**この機で測った速さから出た数**。
    /// **守っているのは「待たされすぎないこと」であって「壊れないこと」ではない**
    /// （B-6 / [`QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS`] の doc）。
    ///
    /// **窓の中ならどの数でもよい。** 押さえているのは規則であって、320 という数ではない。
    /// **窓が1点に潰れていないこと**も見る——潰れていれば、それは窓ではなく決め打ちで、
    /// 「窓で挟んでいる」という説明の方が先に嘘になる。
    #[test]
    fn the_response_limit_sits_just_above_the_longest_measured_response() {
        assert_eq!(
            MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS, 246,
            "実測値（22b / 511 回 / 2026-08-26）から動いた。ADR-0021 §17 と突き合わせること"
        );
        assert_eq!(
            QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS, 368,
            "行列の実測から導いた上端（ADR-0021 §17 / 2026-08-26）から動いた。\
             出し方（何秒ぶんを何トークンと見なしたか）も §17 と突き合わせること"
        );

        // 30% の余裕（整数で切り捨て）。
        let least = MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS.saturating_mul(13) / 10;
        // **窓の上端。** 一度束縛してから比べるのは、定数どうしの比較を
        // `clippy::assertions_on_constants` が「常に真」として弾くため
        // （ビルドで止める側は上の `const _: () = assert!(…)` が持っている）。
        let most = QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS;
        assert!(
            EXTRACTION_RESPONSE_TOKENS >= least,
            "実測の最大に 30% の余裕を足した数を下回っている: \
             上限 = {EXTRACTION_RESPONSE_TOKENS}, 要 {least}, \
             実測 = {MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS}"
        );
        assert!(
            EXTRACTION_RESPONSE_TOKENS <= most,
            "行列の実測から導いた上端を超えている（後ろに並んだ本文が押される）: \
             上限 = {EXTRACTION_RESPONSE_TOKENS}, \
             上端 = {most}"
        );
        assert!(
            least < most,
            "窓が空か1点に潰れている（窓ではなく決め打ちになっている）: \
             下端 = {least}, 上端 = {most}"
        );
    }

    /// **主語つきで保存される。** `利用者` と `あなた` の**両方**を含む出力で見る。
    ///
    /// 片方だけだと「常に `User` を入れる」実装がそのまま緑を通る。
    /// 2つの本文も別にしてある（同じだと入れ替えが検出できない）。
    #[test]
    fn both_subject_words_are_saved_with_the_right_subject() {
        assert_ne!(
            USER_FACT, CHARACTER_FACT,
            "前提: 2つの記憶は別の本文であること"
        );

        let reply = format!("- 利用者: {USER_FACT}\n- あなた: {CHARACTER_FACT}");
        let (output, memories) = extract_once(MockSub::replying(&[&reply]), USER_MESSAGE, REPLY);

        assert_eq!(
            as_pairs(&memories),
            vec![
                (Subject::User, USER_FACT.to_owned()),
                (Subject::Character, CHARACTER_FACT.to_owned()),
            ],
            "主語と本文が対応したまま保存されること"
        );
        let recorded = ran_record(&output);
        assert!(
            has_field(recorded, "extracted=2") && has_field(recorded, "saved=2"),
            "採った件数と書けた件数を出すこと: {recorded:?}"
        );
    }

    /// **記憶はその会話のキャラクターにだけ入る**（持ち主を決め打ちにしない）。
    ///
    /// キャラクターが2体以上いる実機で取り違えると、**`aoi` との会話から抽出した記憶が
    /// 別キャラの `memory` 行として入る。** 想起の入口は `recall_memories(character_id)`
    /// だけなので、もう一方が他人の事実を自分のこととして語り始める。
    /// **修復の導線でも持ち主は移せない**（[`Fixture`] の doc）。
    ///
    /// # 両方を読む
    ///
    /// 「会話した方に 2 件」だけでは足りない——**両方へ書く**実装が緑を通る。
    /// 会話しなかった方が**最初の1件のまま**であることも読む
    /// （0 件にしないのは、持ち主が誰であれ空を返す実装と区別するため / [`Fixture`]）。
    #[test]
    fn memories_land_on_the_character_of_the_conversation_and_nowhere_else() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("owner.sqlite")));
        let reply = format!("- 利用者: {USER_FACT}\n- あなた: {CHARACTER_FACT}");
        let sub = MockSub::replying(&[&reply]);
        let extractor = extractor_of(&fixture.store, &sub);
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        runtime.block_on(async {
            extraction.start(
                fixture.session_id,
                crate::domain::MessageId::from_row(2),
                USER_MESSAGE,
                REPLY,
            );
            extractor.wait_for_the_running_task().await;
        });

        let mine = runtime
            .block_on(fixture.store.recall_memories(fixture.character.id))
            .expect("記憶を読めること");
        let theirs = runtime
            .block_on(fixture.store.recall_memories(fixture.bystander.id))
            .expect("傍観者の記憶を読めること");
        runtime.block_on(fixture.store.close());

        assert_eq!(
            as_pairs(&mine),
            vec![
                (Subject::User, USER_FACT.to_owned()),
                (Subject::Character, CHARACTER_FACT.to_owned()),
            ],
            "会話したキャラクターに入ること"
        );
        assert_eq!(
            as_pairs(&theirs),
            vec![(Subject::Character, BYSTANDER_FACT.to_owned())],
            "会話していないキャラクターは1件も増えないこと"
        );
    }

    /// **1ターンの上限を超えた分は捨てて数える**（ADR-0021 §4 / 13a）。
    ///
    /// 上限（5）・採った件数（5）・捨てた件数（3）が**別の数**になるように
    /// 8 行の出力を与えてある。
    #[test]
    fn the_per_turn_limit_is_enforced_and_the_rest_counted() {
        let over_limit = 3_usize;
        let lines: Vec<String> = (0..per_turn_limit() + over_limit)
            .map(|index| format!("- 利用者: {index} 番目に覚えたこと"))
            .collect();
        let (output, memories) =
            extract_once(MockSub::replying(&[&lines.join("\n")]), USER_MESSAGE, REPLY);

        assert_eq!(
            memories.len(),
            per_turn_limit(),
            "上限までしか保存しないこと: {memories:?}"
        );
        let recorded = ran_record(&output);
        assert!(
            has_field(recorded, &format!("discarded={over_limit}")),
            "超えた分を数えること: {recorded:?}"
        );
        // **理由の件数は値として読む**（作業単位 B-12b）——`contains` だと
        // `over_limit=30`（10 倍の回）を「3 件だった」と読む。
        // **[`has_field`] は使えない**（内訳は `discards=…,over_limit=3,…` という
        // 1トークンの中身で、空白では割れない / [`discard_count`] の doc）。
        assert_eq!(
            discard_count(recorded, DiscardReason::OverLimit),
            over_limit,
            "理由が「上限超過」であること: {recorded:?}"
        );
        // **先頭から採る**（13a）。
        assert_eq!(
            memories
                .first()
                .map(|memory| memory.content.as_str().to_owned()),
            Some("0 番目に覚えたこと".to_owned()),
            "先頭から採ること: {memories:?}"
        );
    }

    /// **`domain` が弾いた分は `warn` に残す**（docs/06-roadmap.md の受け入れ条件）。
    ///
    /// `MemoryContent::parse` は純粋関数で `tracing` を持てないので、
    /// **ここが出さないと痕跡がまったく残らない。**
    ///
    /// # 深刻度と理由は**抽出の記録と同じ行**で見る（作業単位 B-2）
    ///
    /// `output.contains("WARN")` は、**その `WARN` がこの記録のものか**を見ていない
    /// （[`line_containing`] の doc）。理由の欄も同じで、
    /// **別のイベントに `content_too_long=1` が載っていれば満たされてしまう。**
    #[test]
    fn a_rejection_from_the_domain_layer_is_recorded_at_warn() {
        let too_long = "あ".repeat(MemoryContent::MAX_CHARS + 1);
        let reply = format!("- 利用者: {too_long}\n- あなた: {CHARACTER_FACT}");
        let (output, memories) = extract_once(MockSub::replying(&[&reply]), USER_MESSAGE, REPLY);

        assert_eq!(
            as_pairs(&memories),
            vec![(Subject::Character, CHARACTER_FACT.to_owned())],
            "崩れていない行は生き残ること（1行崩れても他は残る / ADR-0021）"
        );
        let recorded = ran_record(&output);
        // **理由の件数は値として読む**（作業単位 B-12）——`contains` だと
        // `content_too_long=10`（10 倍の回）を「1 件だった」と読む。
        //
        // **[`has_field`] は使えない。** 理由の内訳は
        // `discards=malformed=0,…,content_too_long=1,…` という**1トークンの中身**で、
        // 空白では割れない（[`DiscardField`] / [`discard_count`] の doc）。
        assert_eq!(
            discard_count(recorded, DiscardReason::ContentTooLong),
            1,
            "弾かれた理由まで数えること（同じ行の欄を見る）: {recorded:?}"
        );
        assert!(
            recorded.contains("WARN"),
            "`domain` が弾いた回は warn に残すこと（同じ行の深刻度を見る）: {recorded:?}"
        );
        // **否定は出力全体で見る**（どの行にも載っていないことを主張したい）。
        assert!(
            !output.contains(&too_long),
            "弾かれた本文をログに載せないこと: {output:?}"
        );
    }

    /// **破棄の欄には理由が全部並ぶ**（0 件の理由も含む / ADR-0021 §3）。
    ///
    /// 欄ごと消えると「0 件だった」と「その理由が存在しない」が区別できず、
    /// **破棄率の分母が作れない。**
    #[test]
    fn every_discard_reason_appears_in_the_field() {
        let (output, _memories) = extract_once(MockSub::replying(&[""]), USER_MESSAGE, REPLY);

        // **件数は値として読む**（作業単位 B-12b）——`contains` だと
        // `理由=01` にも当たり、**ラベルに接頭辞が付いた別の欄**にも当たる。
        //
        // # **いまは実害が無い。それでも締める**
        //
        // レビューで実測（2026-08-27）: ラベルに接頭辞 `x` を足す変異は、
        // **兄弟の [`discard_count`] を使う5本を赤にする一方、ここは緑のまま**だった
        // ——**緩む領域が兄弟の赤い領域に完全に含まれる**
        // （CLAUDE.md の「第4の分類 —— 権威側にマスクされた変異」）。
        //
        // **マスクは兄弟の都合であって、この検査の強さではない。**
        // 兄弟が1本減った瞬間に黙って穴になるので、ここだけで立てるようにしておく。
        let recorded = ran_record(&output);
        for reason in DiscardReason::ALL {
            assert_eq!(
                discard_count(recorded, reason),
                0,
                "{} の欄が 0 件でも出ること: {recorded:?}",
                reason.label()
            );
        }
    }

    /// **打ち切られた応答の最後の行は保存しない**（ADR-0021 §17 / 作業単位 24）。
    ///
    /// # ここが塞いでいる穴
    ///
    /// [`FRAGMENT_FACT`] は「十年前から通っている」が上限で切れたものだが、
    /// **`- 主語: 本文` の形を完全に満たす**ので、印を見なければ
    /// **破棄にも現れずに恒久的な記憶になる**（`memory` に更新も削除も無い）。
    /// ADR-0021 §8 は入力側で同じことを禁じている——**同じ理屈が出力側にも要る。**
    ///
    /// # 同じ台本を、印だけ変えて2回通す
    ///
    /// 打ち切りの回だけ 2 件になり、書き切った回は 3 件とも残る。
    /// **どちらか片方だけを見ると、「常に最後の1行を捨てる」実装と区別が付かない**
    /// （CLAUDE.md「変更する場合としない場合が同じ入力に同居するテストがあるか」）。
    #[test]
    fn the_last_line_of_a_truncated_response_is_not_saved() {
        assert_ne!(
            USER_FACT, FRAGMENT_FACT,
            "前提: 断片は他の記憶と別の本文であること"
        );
        let reply = truncated_reply();

        let (output, memories) = extract_once(
            MockSub::replying_with(&[&reply], Some(FinishReason::Length)),
            USER_MESSAGE,
            REPLY,
        );

        assert_eq!(
            as_pairs(&memories),
            vec![
                (Subject::User, USER_FACT.to_owned()),
                (Subject::Character, CHARACTER_FACT.to_owned()),
            ],
            "切れた最後の行を保存しないこと（前の行は完結しているので残す）: {output:?}"
        );
        let recorded = ran_record(&output);
        assert!(
            has_field(recorded, "extracted=2") && has_field(recorded, "saved=2"),
            "採った件数に断片を含めないこと: {recorded:?}"
        );
        assert_eq!(
            discard_count(recorded, DiscardReason::Truncated),
            1,
            "捨てた断片を数えること: {recorded:?}"
        );
        assert_eq!(
            discard_count(recorded, DiscardReason::Malformed),
            0,
            "断片を「形式が違う」に混ぜないこと: {recorded:?}"
        );

        // **同じ台本を「書き切った」回として通すと3件とも残る。**
        let (output, memories) = extract_once(MockSub::replying(&[&reply]), USER_MESSAGE, REPLY);

        assert_eq!(
            as_pairs(&memories),
            vec![
                (Subject::User, USER_FACT.to_owned()),
                (Subject::Character, CHARACTER_FACT.to_owned()),
                (Subject::User, FRAGMENT_FACT.to_owned()),
            ],
            "打ち切られていない回は1行も捨てないこと: {output:?}"
        );
        assert_eq!(
            discard_count(ran_record(&output), DiscardReason::Truncated),
            0,
            "{output:?}"
        );
    }

    /// **最後の行を落とすのは `length` の回だけ**（ADR-0021 §17 / 作業単位 24）。
    ///
    /// **4つの申告すべてを、本番の入口を通して見る。**
    /// `stop` / `content_filter` 等 / **申告なし**で落とす実装は、
    /// **上流を替えた瞬間に毎ターン1件ずつ事実を捨てはじめる**——
    /// しかも捨てた理由は `truncated` なので、**ログでは「上限に張り付いている」に見える。**
    #[test]
    fn only_the_length_reason_makes_the_last_line_a_fragment() {
        let reply = truncated_reply();

        for (finish_reason, kept, dropped) in [
            (Some(FinishReason::Stop), 3, 0),
            (Some(FinishReason::Length), 2, 1),
            (Some(FinishReason::Other), 3, 0),
            (None, 3, 0),
        ] {
            let (output, memories) = extract_once(
                MockSub::replying_with(&[&reply], finish_reason),
                USER_MESSAGE,
                REPLY,
            );

            assert_eq!(
                memories.len(),
                kept,
                "停止理由 {finish_reason:?} で残る件数が違う: {output:?}"
            );
            assert_eq!(
                discard_count(ran_record(&output), DiscardReason::Truncated),
                dropped,
                "停止理由 {finish_reason:?} で捨てた件数が違う: {output:?}"
            );
        }
    }

    /// **停止理由の対応付けは1箇所だけ**（[`response_completeness`]）。
    ///
    /// # これは速報であって権威ではない
    ///
    /// 振る舞いを見るのは [`only_the_length_reason_makes_the_last_line_a_fragment`]。
    /// ここが押さえるのは**綴りが増えたとき**——`match` で書いてあるので、
    /// `FinishReason` に値が足されると**このテストがコンパイルできなくなる。**
    /// 既定で「切れた」側へ倒す値が黙って増える経路を塞ぐ。
    #[test]
    fn every_finish_reason_maps_to_a_completeness() {
        for reason in [
            FinishReason::Stop,
            FinishReason::Length,
            FinishReason::Other,
        ] {
            let expected = match reason {
                FinishReason::Length => Completeness::Truncated,
                FinishReason::Stop | FinishReason::Other => Completeness::Complete,
            };
            assert_eq!(response_completeness(Some(reason)), expected, "{reason:?}");
        }
        assert_eq!(
            response_completeness(None),
            Completeness::Complete,
            "申告が無い回を「切れた」と決めつけないこと"
        );
    }

    /// **停止理由を申告しない上流が居ることは、ログに残す**（作業単位 24）。
    ///
    /// この回は「**打ち切られたか分からない**」まま分解している。
    /// 残さないと、**断片が恒久的な記憶になっていても破棄の内訳は正常に見える。**
    ///
    /// **申告があった回には出ないこと**まで見る。毎回出す実装は
    /// 「申告が無い上流に繋がっている」というシグナルを潰す。
    ///
    /// # 深刻度は**メッセージと同じ行**で見る（作業単位 24-2）
    ///
    /// `output.contains("WARN")` は、**その `WARN` がこのメッセージのものか**を
    /// 一切見ていない。今日は決定的（この経路に他の `WARN` が無い）だが、
    /// **`warn!` が1本増えた瞬間に空振りになる**——`info` へ落とす変異が、
    /// 無関係の行の `WARN` を拾って緑を通る（[`line_containing`] の doc）。
    #[test]
    fn a_response_without_a_finish_reason_is_recorded() {
        let reply = truncated_reply();

        let (output, memories) =
            extract_once(MockSub::replying_with(&[&reply], None), USER_MESSAGE, REPLY);

        let recorded = no_finish_reason_record(&output);
        assert!(
            recorded.contains("WARN"),
            "申告が無い回は warn に残すこと（同じ行の深刻度を見る）: {recorded:?}"
        );
        assert_eq!(memories.len(), 3, "申告が無いだけで行を捨てないこと");

        // **申告があった回には出ない。**
        let (output, _memories) = extract_once(
            MockSub::replying_with(&[&reply], Some(FinishReason::Length)),
            USER_MESSAGE,
            REPLY,
        );
        assert!(
            !output.contains(NO_FINISH_REASON_MESSAGE),
            "申告があった回に出さないこと: {output:?}"
        );
    }

    /// **強調記号を落としてから保存し、落とした行数を記録する**（ADR-0021 §11 / 13c-1）。
    ///
    /// `**コーヒー**` は `- 主語: 本文` の形を**完全に満たすので破棄には現れない。**
    /// 落とさなければ `memory` に恒久的に残る（更新も削除も無い）。
    ///
    /// 記録は**破棄率とは別の欄**に出す。破棄率は既に定義された指標で、
    /// 過去の測定値と比べるために使っている——**別の意味の数を混ぜると比較が無効になる。**
    ///
    /// 採った件数（3）と落とした行数（2）を**別の数**にしてある。
    #[test]
    fn markdown_emphasis_is_dropped_before_saving_and_counted_in_the_log() {
        let reply = "\
- あなた: 黒い猫（名前：**コーヒー**）を飼っている
- 利用者: `犬` を二匹飼っている
- あなた: 朝が弱い";
        let (output, memories) = extract_once(MockSub::replying(&[reply]), USER_MESSAGE, REPLY);

        assert_eq!(
            as_pairs(&memories),
            vec![
                (
                    Subject::Character,
                    "黒い猫（名前：コーヒー）を飼っている".to_owned()
                ),
                (Subject::User, "犬 を二匹飼っている".to_owned()),
                (Subject::Character, "朝が弱い".to_owned()),
            ],
            "強調記号を落として保存すること（落とさないと恒久的に残る）: {memories:?}"
        );
        // **欄は抽出の記録の行の中で見る**（作業単位 B-2 / [`ran_record`] の doc）。
        let recorded = ran_record(&output);
        assert!(
            has_field(recorded, "emphasis_stripped=2"),
            "落とした行数を記録すること: {recorded:?}"
        );
        assert!(
            has_field(recorded, "extracted=3"),
            "採った件数とは別の数であること: {recorded:?}"
        );
        assert!(
            has_field(recorded, "discarded=0"),
            "破棄の数に混ぜないこと: {recorded:?}"
        );

        // **`domain` が弾いた回（`warn`）にも同じ欄が出る。**
        // 片方の水準にだけ欄を足すと、集計が水準で割れる（`record_run` の doc）。
        let too_long = "あ".repeat(MemoryContent::MAX_CHARS + 1);
        let reply = format!("- 利用者: **{too_long}**\n- あなた: **朝**が弱い");
        let (output, memories) = extract_once(MockSub::replying(&[&reply]), USER_MESSAGE, REPLY);

        assert_eq!(
            as_pairs(&memories),
            vec![(Subject::Character, "朝が弱い".to_owned())],
            "落としても上限を超える本文は捨てること: {memories:?}"
        );
        let recorded = ran_record(&output);
        assert!(
            recorded.contains("WARN"),
            "warn に残すこと（同じ行の深刻度を見る）: {recorded:?}"
        );
        assert!(
            has_field(recorded, "emphasis_stripped=2"),
            "捨てた行の分も数えること（水準が変わっても欄は同じ）: {recorded:?}"
        );
        assert!(
            has_field(recorded, "extracted=1") && has_field(recorded, "discarded=1"),
            "採った件数・捨てた件数とは別の数であること: {recorded:?}"
        );
    }

    /// **引用書式を真似た行も保存し、剥がした行数を記録する**（作業単位 B-29 / C-15）。
    ///
    /// # ここが押さえているもの
    ///
    /// **C-15 の実測（2026-08-29）では、この形の回は記憶が丸ごと 0 件だった**
    /// ——Qwen3.5-9B 無検閲版の 272 観測のうち 15（5.5%）が「全行が `> ` で始まる」形で、
    /// 67 行とも `missing_subject` として捨てられていた。
    /// **保存される中身が「剥がした後」であること**を、本番の経路で見る。
    ///
    /// 採った件数（3）と剥がした行数（2）を**別の数**にしてある
    /// （同値にすると取り違えが緑になる）。**`emphasis_stripped` とも別の数**（0）。
    #[test]
    fn a_quoted_line_is_saved_and_the_stripped_quote_is_counted_in_the_log() {
        let reply = "\
> - あなた: 平日は8時に開店する
> 利用者: 犬を二匹飼っている
- あなた: 朝が弱い";
        let (output, memories) = extract_once(MockSub::replying(&[reply]), USER_MESSAGE, REPLY);

        assert_eq!(
            as_pairs(&memories),
            vec![
                (Subject::Character, "平日は8時に開店する".to_owned()),
                (Subject::User, "犬を二匹飼っている".to_owned()),
                (Subject::Character, "朝が弱い".to_owned()),
            ],
            "引用記号を剥がして保存すること（剥がさないと1件も残らない）: {memories:?}"
        );
        // **欄は抽出の記録の行の中で見る**（作業単位 B-2 / [`ran_record`] の doc）。
        let recorded = ran_record(&output);
        assert!(
            has_field(recorded, "quotes_stripped=2"),
            "剥がした行数を記録すること: {recorded:?}"
        );
        assert!(
            has_field(recorded, "extracted=3"),
            "採った件数とは別の数であること: {recorded:?}"
        );
        assert!(
            has_field(recorded, "discarded=0") && has_field(recorded, "emphasis_stripped=0"),
            "破棄にも強調の数にも混ぜないこと: {recorded:?}"
        );

        // **`domain` が弾いた回（`warn`）にも同じ欄が出る。**
        // 片方の水準にだけ欄を足すと、集計が水準で割れる（`record_run` の doc）。
        let too_long = "あ".repeat(MemoryContent::MAX_CHARS + 1);
        let reply = format!("> - 利用者: {too_long}\n> - あなた: 朝が弱い");
        let (output, memories) = extract_once(MockSub::replying(&[&reply]), USER_MESSAGE, REPLY);

        assert_eq!(
            as_pairs(&memories),
            vec![(Subject::Character, "朝が弱い".to_owned())],
            "剥がしても上限を超える本文は捨てること: {memories:?}"
        );
        let recorded = ran_record(&output);
        assert!(
            recorded.contains("WARN"),
            "warn に残すこと（同じ行の深刻度を見る）: {recorded:?}"
        );
        assert!(
            has_field(recorded, "quotes_stripped=2"),
            "捨てた行の分も数えること（水準が変わっても欄は同じ）: {recorded:?}"
        );
        assert!(
            has_field(recorded, "extracted=1") && has_field(recorded, "discarded=1"),
            "採った件数・捨てた件数とは別の数であること: {recorded:?}"
        );
    }

    /// **手本の文を写した回は、そのまま保存したうえで数える**（作業単位 B-28 / C-15）。
    ///
    /// # ⚠ この検査は「防いでいる」ことを見ていない
    ///
    /// **見ているのは、混ざったまま保存されること**（本文が1文字も変わらないこと）**と、
    /// その回に印が残ること**の2つ。**捨てていないことが、期待値の側に書いてある。**
    /// 捨てない理由は `record_run` の doc と `prompt::extraction::example_echoes` の doc。
    ///
    /// # 実測された 2 件を、両方とも1回の応答に置いてある（C-15 / 2026-08-29）
    ///
    /// 1行目が**接ぎ木**（そのまま保存された側）、2行目が**主語の無い写し**
    /// （`malformed` で捨てられた側）。**後者は保存された記憶からは見えない**ので、
    /// **欄が生出力を見ていること**がここで決まる。
    ///
    /// 採った件数（2）・捨てた件数（1）・写した回数（3）を**全部違う数**にしてある。
    #[test]
    fn a_reply_that_copies_the_prompt_example_is_saved_and_counted() {
        let reply = "\
- あなた: ホウレンソウが冬に凍死して泣いたことがある [好み] 苦い珈琲が好き
[好み] 苦い珈琲が好き <- 前の記録で既に書かれているため、今回は新しい事実として追加できない。
- 利用者: 犬を二匹飼っている";
        let (output, memories) = extract_once(MockSub::replying(&[reply]), USER_MESSAGE, REPLY);

        assert_eq!(
            as_pairs(&memories),
            vec![
                (
                    Subject::Character,
                    "ホウレンソウが冬に凍死して泣いたことがある [好み] 苦い珈琲が好き".to_owned()
                ),
                (Subject::User, "犬を二匹飼っている".to_owned()),
            ],
            "手本が混ざった行も、そのまま保存すること（捨てない・書き換えない）: {memories:?}"
        );

        let recorded = ran_record(&output);
        assert!(
            has_field(recorded, "example_echoes=3"),
            "写した回数を記録すること（捨てられた行の分も含めて）: {recorded:?}"
        );
        assert!(
            has_field(recorded, "extracted=2")
                && has_field(recorded, "saved=2")
                && has_field(recorded, "discarded=1"),
            "採った件数・保存した件数・捨てた件数とは別の数であること: {recorded:?}"
        );
        assert!(
            has_field(recorded, "emphasis_stripped=0") && has_field(recorded, "quotes_stripped=0"),
            "正規化の数に混ぜないこと: {recorded:?}"
        );

        // **`domain` が弾いた回（`warn`）にも同じ欄が出る。**
        // 片方の水準にだけ欄を足すと、集計が水準で割れる（`record_run` の doc）。
        let too_long = "あ".repeat(MemoryContent::MAX_CHARS + 1);
        let reply = format!(
            "- 利用者: {too_long} [好み] 苦い珈琲が好き\n\
             - あなた: 犬を二匹飼っている と [好み] 苦い珈琲が好き"
        );
        let (output, memories) = extract_once(MockSub::replying(&[&reply]), USER_MESSAGE, REPLY);

        assert_eq!(
            as_pairs(&memories),
            vec![(
                Subject::Character,
                "犬を二匹飼っている と [好み] 苦い珈琲が好き".to_owned()
            )],
            "上限を超える本文は捨てること（手本の写しは理由にしない）: {memories:?}"
        );
        let recorded = ran_record(&output);
        assert!(
            recorded.contains("WARN"),
            "warn に残すこと（同じ行の深刻度を見る）: {recorded:?}"
        );
        assert!(
            has_field(recorded, "example_echoes=3"),
            "捨てた行の分も数えること（水準が変わっても欄は同じ）: {recorded:?}"
        );
        assert!(
            has_field(recorded, "extracted=1") && has_field(recorded, "discarded=1"),
            "採った件数・捨てた件数とは別の数であること: {recorded:?}"
        );
    }

    /// **手本の「書式」が本文に居座った回も、そのまま保存したうえで数える**
    /// （作業単位 B-33 / C-15）。
    ///
    /// # ⚠ この検査も「防いでいる」ことを見ていない
    ///
    /// [`a_reply_that_copies_the_prompt_example_is_saved_and_counted`] とまったく同じ立場。
    /// **角括弧が残った本文が1文字も変わらずに保存されること**と、
    /// **その回に印が残ること**の2つしか見ていない。
    /// 捨てない理由は `prompt::extraction::example_format_echoes` の doc。
    ///
    /// # **`example_echoes` とは数える対象が違う**
    ///
    /// | | 見る先 | 捨てられた行 |
    /// |---|---|---|
    /// | `example_echoes` | **生出力** | **数える** |
    /// | `example_format_echoes` | **保存される本文** | **数えない** |
    ///
    /// **後者は「永久に残ること」が実害**なので、捨てられた行の角括弧は数に入らない。
    /// `warn` の側で、**上限超過で捨てられる行にも `[好み]` を置いてある**
    /// ——それが数に入っていたら、この検査が赤になる。
    #[test]
    fn a_reply_that_copies_the_prompt_example_format_is_saved_and_counted() {
        let reply = "\
- あなた: ミニトマトとネギ、犬「ぶん」を育てている [好み]
- 利用者: 鉢底に敷く石について確認している [好み] 粗いものが効果的と伝えている
- あなた: [好み] 朝が弱い
ただの前置きです。";
        let (output, memories) = extract_once(MockSub::replying(&[reply]), USER_MESSAGE, REPLY);

        assert_eq!(
            as_pairs(&memories),
            vec![
                (
                    Subject::Character,
                    "ミニトマトとネギ、犬「ぶん」を育てている [好み]".to_owned()
                ),
                (
                    Subject::User,
                    "鉢底に敷く石について確認している [好み] 粗いものが効果的と伝えている"
                        .to_owned()
                ),
                // **先頭の対は分類として読まれる**ので、本文には残らない。
                (Subject::Character, "朝が弱い".to_owned()),
            ],
            "角括弧が残った行も、そのまま保存すること（捨てない・書き換えない）: {memories:?}"
        );

        let recorded = ran_record(&output);
        assert!(
            has_field(recorded, "example_format_echoes=2"),
            "本文に残った対の数を記録すること（先頭の対は数えない）: {recorded:?}"
        );
        assert!(
            has_field(recorded, "extracted=3")
                && has_field(recorded, "saved=3")
                && has_field(recorded, "discarded=1"),
            "採った件数・保存した件数・捨てた件数とは別の数であること: {recorded:?}"
        );
        assert!(
            has_field(recorded, "example_echoes=0")
                && has_field(recorded, "emphasis_stripped=0")
                && has_field(recorded, "quotes_stripped=0"),
            "手本の本文の写しや、正規化の数に混ぜないこと: {recorded:?}"
        );

        // **`domain` が弾いた回（`warn`）にも同じ欄が出る。**
        // 片方の水準にだけ欄を足すと、集計が水準で割れる（`record_run` の doc）。
        let too_long = "あ".repeat(MemoryContent::MAX_CHARS + 1);
        let reply = format!(
            "- 利用者: {too_long} [好み]\n\
             - あなた: 底石を勧める [好み] 粗い材料が効く [失敗経験] とも言う"
        );
        let (output, memories) = extract_once(MockSub::replying(&[&reply]), USER_MESSAGE, REPLY);

        assert_eq!(
            as_pairs(&memories),
            vec![(
                Subject::Character,
                "底石を勧める [好み] 粗い材料が効く [失敗経験] とも言う".to_owned()
            )],
            "上限を超える本文は捨てること（角括弧は理由にしない）: {memories:?}"
        );
        let recorded = ran_record(&output);
        assert!(
            recorded.contains("WARN"),
            "warn に残すこと（同じ行の深刻度を見る）: {recorded:?}"
        );
        assert!(
            has_field(recorded, "example_format_echoes=2"),
            "捨てられた行の角括弧まで数えている、または回数で数えていない: {recorded:?}"
        );
        assert!(
            has_field(recorded, "extracted=1") && has_field(recorded, "discarded=1"),
            "採った件数・捨てた件数とは別の数であること: {recorded:?}"
        );
    }

    /// **上流が落ちていても、記憶が増えないだけ**（本文の応答はとうに返っている）。
    #[test]
    fn an_upstream_failure_saves_nothing_and_is_recorded() {
        let (output, memories) = extract_once(MockSub::failing(), USER_MESSAGE, REPLY);

        assert!(memories.is_empty(), "記憶を作らないこと: {memories:?}");
        assert!(
            output.contains("記憶の抽出で上流を呼べませんでした"),
            "失敗を記録すること: {output:?}"
        );
        assert!(
            !output.contains(RAN_MESSAGE),
            "呼べなかった回を「抽出しました」と記録しないこと: {output:?}"
        );
    }

    /// **崩れ切った出力でも panic せず、1件も保存しない**（絶対制約）。
    #[test]
    fn a_broken_output_is_discarded_and_counted() {
        /// 主語も区切りも無い出力。**ここから記憶を作ってはいけない。**
        const GARBAGE: &str = "承知しました！以下に事実をまとめます\n{memories: [}\n???";

        let (output, memories) = extract_once(MockSub::replying(&[GARBAGE]), USER_MESSAGE, REPLY);

        assert!(
            memories.is_empty(),
            "崩れた行から記憶を作らないこと（壊れた文字列は消せない）: {memories:?}"
        );
        let recorded = ran_record(&output);
        assert!(
            has_field(recorded, "extracted=0") && has_field(recorded, "discarded=3"),
            "捨てた分を数えること: {recorded:?}"
        );
        assert!(
            !output.contains(GARBAGE),
            "生出力をログに載せないこと: {output:?}"
        );
    }

    /// **停止で走っている抽出が落ちる**（ADR-0021 §9。タスクを leak させない）。
    #[test]
    fn shutdown_aborts_the_running_extraction() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        // **傍観者は見ない**（このテストが見ているのは走らせ方だけ / [`Fixture`]）。
        let Fixture {
            store,
            character,
            session_id,
            ..
        } = runtime.block_on(fresh(&dir.path().join("shutdown.sqlite")));
        let (sub, _open) = MockSub::gated();
        let extractor = extractor_of(&store, &sub);
        let extraction = Extraction::new(Arc::clone(&extractor), &character);

        let watch = runtime.block_on(async {
            extraction.start(
                session_id,
                crate::domain::MessageId::from_row(2),
                USER_MESSAGE,
                REPLY,
            );
            wait_until_the_upstream_is_called(&sub).await;
            // **落とす前に見張りの控えを取る。** `shutdown` は枠を空けるので、
            // 後からでは「終わったのか落ちたのか」を確かめる相手が居なくなる。
            let handle = extractor
                .take_slot()
                .take()
                .expect("走っている1件があること");
            assert!(!handle.is_finished(), "前提: まだ終わっていないこと");
            let watch = handle.abort_handle();
            *extractor.take_slot() = Some(handle);
            watch
        });
        assert_eq!(sub.call_count(), 1, "前提: 抽出が走り出していること");

        extractor.shutdown();

        // `abort` は即座には反映されないので、runtime を1周させる。
        runtime.block_on(async { tokio::task::yield_now().await });
        runtime.block_on(store.close());

        assert!(
            watch.is_finished(),
            "停止で走っている抽出が落ちること（leak させない）"
        );
        assert!(
            extractor.take_slot().is_none(),
            "落とした後に枠を空けること（次の抽出が走れる状態に戻ること）"
        );
    }

    /// **短い会話が上限に当たらないこと。**
    ///
    /// 上限（[`FALLBACK_EXTRACTION_PROMPT_TOKENS`]）が小さすぎると、
    /// **どの往復も「長すぎる」で捨てられ、記憶が一切増えない。**
    /// しかもその壊れ方は破棄率のログにしか現れないので、
    /// **依頼の固定部分が上限の半分にも満たないこと**をここで固定する。
    ///
    /// # **測るのは「実際の1往復の削れない枠」**（作業単位 13c-2 の是正）
    ///
    /// 以前は会話を両側とも空文字で測っていたが、`render_conversation` は
    /// **空の側の小見出しごと落とす**（`an_empty_side_of_the_conversation_leaves_no_empty_heading`）。
    /// つまり**実機の往復では必ず載る2本の小見出しが、勘定から抜けていた。**
    /// 実測（2026-08-22 / **単位は `request_tokens` の返り値**——
    /// 本文のバイト数にメッセージ2件分の `MESSAGE_OVERHEAD_TOKENS` = 16 を足した数）:
    ///
    /// | 何を測るか | トークン |
    /// |---|---|
    /// | 会話が両側とも空 / 呼び名 `あ`×50（旧） | 2077 |
    /// | 会話が両側とも空 / 呼び名がバイト最悪値 | 2127 |
    /// | **両側が非空で小見出しが2本載る / 呼び名がバイト最悪値（新）** | **2339**（13c-4 で 2185 から増えた） |
    ///
    /// **⚠ 表の数は締めの一言が `CLOSING_CONTENT_NOT_THE_ACT` のとき**——
    /// **B-11 で綴りの選択肢は消えたので、いまはこれしかない**（ADR-0037 §決定4）。
    ///
    /// **3つとも 13c-2 の実測（2038 / 2088 / 2146）から一律 39 増えている。**
    /// `QUOTE_NOTE` が 13c-3a で 39 バイト伸びた分で、
    /// **削れない枠は指示を1文字足すたびに実機の全往復で増える。**
    ///
    /// **黙って枠を食う書き換えが実在する**——`USER_TURN_HEADING` を 117 バイト
    /// 伸ばすと、**実機の全往復でプロンプトが 117 増える**
    /// （13c-2 で実測。当時の枠 2146 → 2263）。
    ///
    /// > **⚠ その 117 はここでは赤にならない。** 見る規則は「上限の半分」なので、
    /// > 壁は 4095。**枠が 1757 増えて初めて赤になる**（4096 − 2339）。
    /// > 13c-2 の実測では当時の枠を 2100 伸ばして 4246 で赤。
    /// > **小さな増分を捕まえる見張り番ではない。**
    /// > 上限を 8192 に上げた時点で意図してそうしてある（13c-2 / 司令塔の決定）。
    ///
    /// # **呼び名は「文字数の上限」ではなく「バイトの最悪値」で測る**
    ///
    /// [`estimate_tokens`] は **UTF-8 バイト数**だが、
    /// [`DisplayName::MAX_CHARS`] は**文字数**なので、`あ`（3バイト）は最悪値ではない。
    /// `DisplayName::parse` はアストラル面の文字（4バイト）も受けるので、
    /// 削れない枠に載りうる最大は `MAX_CHARS * 4` バイト。
    /// **このリポジトリで一度踏んだ罠**（本文プロンプトの予算で、日本語を
    /// 3バイト/文字で数えたのに見張り番は4バイトで測っていた）。
    /// **⚠ 手本の綴りは [`ExampleStyle::ALL`] を回す**（作業単位 22a）。
    ///
    /// 片方の綴りだけで測ると、**もう片方だけを太らせる書き換えが緑を通る。**
    /// ここは「上限の半分」という緩い壁なので**小さな増分は捕まえない**が、
    /// 絶対値の壁は
    /// [`every_example_style_leaves_the_same_room_for_a_conversation`] が持つ。
    #[test]
    fn the_fixed_part_of_the_request_leaves_room_for_a_conversation() {
        // `\u{20BB7}`（𠮷）はアストラル面の CJK 文字で、UTF-8 では1文字4バイト。
        // `DisplayName::parse` が受ける中で**1文字あたりのバイト数が最大**。
        let display_name = DisplayName::parse(&"\u{20BB7}".repeat(DisplayName::MAX_CHARS))
            .expect("上限ちょうどの呼び名は正当であること");
        // 前提: 本当にバイト最悪値で測っている（`あ` に戻す書き換えをここで止める）。
        assert_eq!(
            display_name.as_str().len(),
            DisplayName::MAX_CHARS * 4,
            "呼び名がバイト最悪値になっていない（estimate_tokens はバイト数）"
        );

        for example_style in ExampleStyle::ALL {
            // **会話は両側とも非空**——小見出しが2本とも載る形が実機の1往復。
            // 中身は1文字で足りる（測りたいのは中身ではなく枠）。
            // **両側を同じ文字にしない**（取り違えても緑になる）。
            let messages = build_extraction_prompt(&ExtractionInput {
                display_name: &display_name,
                user_message: "犬",
                character_message: "猫",
                // **記憶を入れない形が「削れない枠」**（ADR-0021 §13 / 13c-3a）。
                // 記憶の節は0件なら見出しごと出ず、載る分は予算の残りからしか取らない。
                known_memories: &[],
                budget: FALLBACK_EXTRACTION_PROMPT_TOKENS,
                example_style,
            })
            .into_messages();
            let fixed = request_tokens(&messages);

            assert!(
                fixed.saturating_mul(2) < FALLBACK_EXTRACTION_PROMPT_TOKENS,
                "固定部分が上限の半分に満たないこと（会話の分が残らないと何も抽出できない）: \
                 綴り = {example_style:?}, fixed = {fixed}, \
                 limit = {FALLBACK_EXTRACTION_PROMPT_TOKENS}"
            );

            // 前提: 小見出しを2本とも載せた形を測っている。
            // **空側を落とす分岐が効いた形（＝旧い測り方）より必ず大きい。**
            // `USER_TURN_HEADING` / `CHARACTER_TURN_HEADING` は `prompt::extraction` の
            // 私有定数なので、綴りを写さずに**差**で見る。
            let both_sides_empty = request_tokens(
                &build_extraction_prompt(&ExtractionInput {
                    display_name: &display_name,
                    user_message: "",
                    character_message: "",
                    known_memories: &[],
                    budget: FALLBACK_EXTRACTION_PROMPT_TOKENS,
                    example_style,
                })
                .into_messages(),
            );
            assert!(
                both_sides_empty < fixed,
                "前提: 小見出しが2本とも載る形を測っていること: \
                 綴り = {example_style:?}, 両側が空 = {both_sides_empty}, 実際の枠 = {fixed}"
            );

            // 前提: 手本の行が現に載っている（＝空のプロンプトを測っていない）。
            // **手本の1本目は綴りで変わる**ので、対応表は直書き（作業単位 22a）。
            let first_example = match example_style {
                ExampleStyle::Balanced => EXAMPLE_USER_LINE,
                ExampleStyle::CharacterOnly => EXAMPLE_CHARACTER_ONLY_LINE,
            };
            let joined = messages
                .iter()
                .map(|message| message.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                joined.contains(first_example) && joined.contains(EXAMPLE_CHARACTER_LINE),
                "前提: 依頼の固定部分を測っていること: 綴り = {example_style:?}"
            );
        }
    }

    /// **削れない枠に許す絶対の上限**（トークン / [`request_tokens`] の単位）。
    ///
    /// **`CLOSING_*` 定数の長さから作らないこと。**
    /// 定数から期待値を組むと、定数を伸ばす変異が期待値を道連れにして緑を通る
    /// （CLAUDE.md「期待値を守るべき定数から作っている」/ `16b` で2度出た形）。
    ///
    /// 実測（2026-08-25 / 呼び名はバイト最悪値・会話は両側1文字）: **2339**。
    /// **261 の余裕を持たせてある。**
    ///
    /// # 文面そのものは誰が持つのか（**21a-2 で書き直し / B-11 で1本に**）
    ///
    /// 21a はここに「文面そのものの妥当性は `..._keeps_its_carve_out` の側が持つ」と
    /// 書いた。**その主張は成り立っていなかった**——レビューの変異
    /// （締めの末尾に `聞いた・答えたことは除きます。`＝45 バイトを足す）は
    /// **絶対値 2387 ≤ 2600 で素通りした**（1480 全緑）。
    ///
    /// 現在の受け持ちは次のとおり（`..._byte_cap` は `prompt::extraction` のテスト）。
    ///
    /// | 見張り番 | 守るもの | 守らないもの |
    /// |---|---|---|
    /// | `the_closing_reminder_keeps_its_carve_out` | 主節と但し書きが**句として**残ること／前置きで範囲を狭めないこと | **文を足すこと**（既存の句は残るので鳴らない） |
    /// | `the_closing_line_stays_within_its_byte_cap` | **188 バイト**を超えないこと（**文面1文ぶんで鳴る層**） | 同じ長さのまま意味を変える書き換え |
    /// | この定数 | **枠が壊れる規模**の太り | それ未満の増分 |
    ///
    /// **「型で表現できなくした」形はどこにも無い。**
    /// 文面を書き換えるときは、**人が読んで確かめること。**
    const MAX_FIXED_FRAME_TOKENS: u32 = 2_600;

    /// **手本の綴りは、会話に残る枠を1トークンも変えない**（作業単位 22a / ADR-0021 §15）。
    ///
    /// # 「差 0」を要求する理由
    ///
    /// **手本は主語だけを差し替える**（`利用者` / `あなた` はどちらも UTF-8 で 9 バイト）ので、
    /// **枠は完全に一致するのが正しい。**
    ///
    /// 一致していれば、22b の A/B で**両腕の会話に使える枠がバイト単位で同じ**になる。
    /// ずれると、**長い往復が片方の腕でだけ §8 で捨てられ**、腕どうしを比べたことにならない
    /// （`SKIP too_long=` の行が片側に偏る）。
    ///
    /// # **枠が同じことは「分岐が潰れている」ことではない**
    ///
    /// 枠の差が 0 でないことを「分岐が生きている」の証拠には使えない
    /// （同じなのが正しいので）。
    /// だから**プロンプトの文字列そのものが違うこと**を前提として別に見る。
    ///
    /// # **絶対値の壁もここが持つ**（作業単位 B-11）
    ///
    /// 21a までは締めの綴りを回す
    /// `every_closing_style_leaves_the_same_room_for_a_conversation` と2本で
    /// [`MAX_FIXED_FRAME_TOKENS`] を見ていた。**綴りが1つになってあちらは落ちた**ので、
    /// **削れない枠の絶対値を見るのはこの1本だけ**になっている。
    ///
    /// > **⚠ [`ExampleStyle`] が将来1つになったら、この本の主張は2つに割れる。**
    /// > **`assert_eq!(smallest, largest)` と「綴りごとに別のプロンプト」は
    /// > 器具の `ARMS`（`tests/extraction_quality_harness.rs`）と
    /// > まったく同じ形で空回りする**（比べる相手が居ない）。
    /// > **残るのは絶対値の壁だけ**なので実害は小さいが、
    /// > **そのときは「枠が同じ」を守っているつもりで読まないこと。**
    #[test]
    fn every_example_style_leaves_the_same_room_for_a_conversation() {
        let display_name = DisplayName::parse(&"\u{20BB7}".repeat(DisplayName::MAX_CHARS))
            .expect("上限ちょうどの呼び名は正当であること");

        let built = |example_style: ExampleStyle| {
            build_extraction_prompt(&ExtractionInput {
                display_name: &display_name,
                user_message: "犬",
                character_message: "猫",
                known_memories: &[],
                budget: FALLBACK_EXTRACTION_PROMPT_TOKENS,
                example_style,
            })
            .into_messages()
        };

        let mut frames: Vec<(ExampleStyle, u32)> = Vec::new();
        let mut texts: Vec<(ExampleStyle, String)> = Vec::new();
        for example_style in ExampleStyle::ALL {
            let messages = built(example_style);
            let tokens = request_tokens(&messages);
            assert!(
                tokens <= MAX_FIXED_FRAME_TOKENS,
                "削れない枠が絶対の上限を超えた（実機の全往復でこの分だけ会話が削られる）: \
                 綴り = {example_style:?}, 枠 = {tokens}, 上限 = {MAX_FIXED_FRAME_TOKENS}"
            );
            frames.push((example_style, tokens));
            texts.push((example_style, joined(&messages)));
        }

        // **本題: 枠は綴りに依らず完全に一致する。**
        let smallest = frames.iter().map(|(_, tokens)| *tokens).min();
        let largest = frames.iter().map(|(_, tokens)| *tokens).max();
        let (Some(smallest), Some(largest)) = (smallest, largest) else {
            panic!("綴りが1つも無い（ExampleStyle::ALL が空）");
        };
        assert_eq!(
            smallest, largest,
            "手本の綴りで会話に残る枠が変わった（主語だけを差し替える実験の前提が崩れる。\
             長い往復が片方の腕でだけ §8 で捨てられる）: {frames:?}"
        );

        // **前提: 綴りごとに別のプロンプトが出ている**（枠が同じでも中身は違う）。
        for (index, (left_style, left)) in texts.iter().enumerate() {
            for (right_style, right) in texts.iter().skip(index + 1) {
                assert_ne!(
                    left, right,
                    "前提: 綴りごとに別の手本が載っていること（同一なら分岐が潰れている）: \
                     {left_style:?} / {right_style:?}"
                );
            }
        }
    }

    /// **すでに記録済みの事実を、予算に収まるだけ載せて依頼する**（ADR-0021 §13 / 13c-3b）。
    ///
    /// # ここが塞いでいる穴
    ///
    /// 13c-3a の時点で `run` は `known_memories: &[]` を渡していたので、
    /// **`budget` は出力に一切現れなかった。** 実測で、
    /// **`budget` を `0` にする変異が 1073 passed / 0 failed で緑を通った。**
    /// 節そのものが本番では一度も出ていなかったので、
    /// 「予算を渡している」ことを確かめる術がどこにも無かった
    /// （CLAUDE.md「テスト専用の入口を作ると、本番の入口が観測対象から外れる」の同型）。
    ///
    /// **見るのは本番の経路**——`Extraction::start` → `run` に差してあるのは
    /// 上流のモックだけで、依頼を組む側も記憶を読む側も本物。
    ///
    /// # 期待値を既定値・両端と別の数にしてある
    ///
    /// 7 件のうち **3 件が載り、4 件が落ちる**（[`FITTING_MEMORIES`] の doc）。
    ///
    /// | 渡すもの | 載る件数 | この検査 |
    /// |---|---|---|
    /// | `FALLBACK_EXTRACTION_PROMPT_TOKENS`（8192） | 3 | 緑 |
    /// | `0` | 0 | **赤**（載るはずの目印が無い） |
    /// | `ASSUMED_SUB_CONTEXT_TOKENS`（16384） | 7 | **赤**（`前提: 上流へ行っていること`） |
    /// | `&[]`（記憶を渡さない） | 0 | **赤** |
    ///
    /// **16384 の行で落ちるのは「落ちるはずの目印が載る」ではない。**
    /// 7 件とも載った依頼は 14445 トークンになり、**§8 の門が丸ごと捨てる**ので
    /// 上流には届かない。目印を見る前に、`call_count()` の前提で落ちる。
    ///
    /// # **予算と門の関係はここでは見ていない**
    ///
    /// このフィクスチャは1件が大きく（1732）、詰め終わっても**取り分が 675 余る。**
    /// そのため予算を**減らす**変異（`- EXTRACTION_RESPONSE_TOKENS` / 当時は −512）は
    /// 載る件数を変えず（4 件目は元から入らない）、**増やす**変異（`+ 1000`）でも
    /// 4 件目の累計 7170 が取り分 7113 に **57 足りない**ので、やはり 3 件のまま
    /// ——どちらも緑を通る（実測 / 2026-08-22）。
    /// 見張るのは [`a_request_that_fills_its_budget_with_memories_still_passes_the_gate`]。
    ///
    /// **持ち主も見る。** `recall_memories` を別のキャラクターで呼ぶ変異
    /// （`CharacterId::from_row(1)` = 傍観者）は、
    /// **傍観者の事実が依頼に載る**形で赤になる。
    #[test]
    fn the_request_carries_the_newest_known_memories_that_fit_the_budget() {
        // 前提はコンパイル時に見る（値がすべて定数なので実行時に見る意味が無い）。
        const {
            assert!(
                FITTING_MEMORIES > 0 && FITTING_MEMORIES < SEEDED_MEMORIES,
                "前提: 一部だけが載る形であること（全部載る／1件も載らないと予算が観測できない）"
            );
            assert!(
                FITTING_MEMORIES != DROPPED_MEMORIES,
                "前提: 載った件数と落ちた件数が別の数であること（欄の取り違えが観測できない）"
            );
        }

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("known.sqlite")));
        let newest_first =
            runtime.block_on(seed_known_memories(&fixture.store, fixture.character.id));
        let sub = MockSub::replying(&[""]);
        let extractor = extractor_of(&fixture.store, &sub);
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
        runtime.block_on(fixture.store.close());

        assert_eq!(sub.call_count(), 1, "前提: 上流へ行っていること");
        let prompt = sub.last_prompt();
        // 枠が動くと収まる件数が変わる。**落ちたときに何を見直すか**が分かるように添える。
        let frame = tokens_of_request(USER_MESSAGE, REPLY);

        for marker in newest_first.iter().take(FITTING_MEMORIES) {
            assert!(
                prompt.contains(marker),
                "新しい側から予算いっぱいまで載せること（{marker} が無い）: \
                 枠 = {frame}, 上限 = {FALLBACK_EXTRACTION_PROMPT_TOKENS}"
            );
        }
        for marker in newest_first.iter().skip(FITTING_MEMORIES) {
            assert!(
                !prompt.contains(marker),
                "予算に入らない分は載せないこと（{marker} が載っている）: \
                 枠 = {frame}, 上限 = {FALLBACK_EXTRACTION_PROMPT_TOKENS}"
            );
        }
        assert!(
            !prompt.contains(BYSTANDER_FACT),
            "会話していないキャラクターの記憶を載せないこと（持ち主を決め打ちにしない）: \
             {prompt:?}"
        );
        // **依頼が予算を超えないこと**（記憶が会話を押し出していない証拠）。
        assert!(
            request_tokens(&sub.last_request()) <= FALLBACK_EXTRACTION_PROMPT_TOKENS,
            "記憶を載せた依頼が上限を超えないこと: 枠 = {frame}"
        );

        let recorded = ran_record(&output);
        assert!(
            has_field(recorded, &format!("known_included={FITTING_MEMORIES}")),
            "載せた件数を記録すること: {recorded:?}"
        );
        assert!(
            has_field(recorded, &format!("known_dropped={DROPPED_MEMORIES}")),
            "入らなかった件数を記録すること（充足度が読めないと §13 を測れない）: {recorded:?}"
        );
        // **破棄率にも強調記号の欄にも混ぜないこと**（別の意味の指標）。
        assert!(
            has_field(recorded, "discarded=0") && has_field(recorded, "emphasis_stripped=0"),
            "落とした記憶を他の指標に混ぜないこと: {recorded:?}"
        );
    }

    /// **テスト専用の入口が、本番と1バイト違わない依頼を返す**（作業単位 21a-2）。
    ///
    /// # ここが塞いでいる穴（**21a のレビューで実測**）
    ///
    /// 21a は [`extraction_request_for_tests`] /
    /// [`extraction_prompt_budget_for_tests`] を足し、
    /// **器具の doc に「温度・応答の上限・予算・§8 の門は本番から取る。写さない」と書いた。**
    /// その主張を支える見張り番が**1本も無かった**——実測で次の2つが緑を通った
    /// （どちらも 1480 passed / 0 failed）:
    ///
    /// | 変異 | 実機の器具に起きること |
    /// |---|---|
    /// | [`extraction_request_for_tests`] が委譲をやめ、自前で `temperature: Some(0.0)` / `max_tokens: None` を組む | **本番と別の温度・別の応答上限**で 21b の両腕が回る |
    /// | [`extraction_prompt_budget_for_tests`] が上限の半分を返す | 控除リストの件数が変わり、**13c-3 が保存件数を 30% 動かした要因**が静かにずれる |
    ///
    /// **どちらも器具の出力（TSV）を見ても分からない。**
    /// CLAUDE.md「『型で表現できなくした』と書く前に、表現できてしまう実装を実際に書いてみること」。
    ///
    /// # 権威は「上流に実際に届いたもの」
    ///
    /// 突き合わせる相手は [`MockSub`] が受け取った依頼——
    /// **本番の [`extract`] を丸ごと通した結果**であって、[`extraction_request`] の
    /// 返り値ではない。**期待値を [`extraction_request`] から作らないこと**——
    /// 委譲をやめる変異が期待値を道連れにして緑になる
    /// （CLAUDE.md「写しを1つにしても、その1つは自動的には守られない」）。
    ///
    /// 期待値の側は**器具とまったく同じ経路**で組む——
    /// `build_extraction_prompt` に [`extraction_prompt_budget_for_tests`] を渡し、
    /// 出来たメッセージ列を [`extraction_request_for_tests`] に通す。
    /// **器具が写しているのはこの2行だけ**なので、ここが本番と一致していれば
    /// 器具は本番と同じつまみで回る。
    ///
    /// # 記憶を入れた状態で測る（**予算が観測できる唯一の形**）
    ///
    /// 記憶が 0 件だと**予算は出力にまったく現れない**（13c-3b で実測した穴と同型）。
    /// [`seed_known_memories`] の 7 件のうち **3 件だけが載る**状態にしてあるので、
    /// 予算を動かす変異は載る件数を動かし、依頼の文字列が変わる。
    ///
    /// # 温度は値そのものも見る（**⚠ 作業単位 A-7d で `None` から `0.8` へ変わった**）
    ///
    /// **21a 以前、温度を見る検査は1本も無かった。**
    ///
    /// **A-7d までは「`None`（上流の既定に任せる）であること」を見ていた。**
    /// **いまは [`SUB_LLM_TEMPERATURE`] を明示する**ので、見る値がそちらへ移った
    /// （[ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定2 ——
    /// **0.8 は今日の実効の既定そのもの**なので、**上流が受け取る値は変わっていない**）。
    ///
    /// **⚠ 主張のほうは変わっていない** —— 器具の `PERSONACORE_HARNESS_REPS` は
    /// **繰り返しが検出力の源**（器具の doc）なので、
    /// **温度を `0.0` に固定した日にその前提は崩れる。**
    /// **`0.8` は 0 ではない**ので崩れない。**ここが `Some(0.0)` を赤にする側でもある。**
    #[test]
    fn the_test_only_entry_points_hand_back_what_production_sends() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("parity.sqlite")));
        let _seeded = runtime.block_on(seed_known_memories(&fixture.store, fixture.character.id));
        let sub = MockSub::replying(&[""]);
        let extractor = extractor_of(&fixture.store, &sub);
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        runtime.block_on(async {
            extraction.start(
                fixture.session_id,
                crate::domain::MessageId::from_row(2),
                USER_MESSAGE,
                REPLY,
            );
            extractor.wait_for_the_running_task().await;
        });

        // **本番が上流へ渡したもの。これが権威。**
        assert_eq!(sub.call_count(), 1, "前提: 上流へ行っていること");
        let sent_prompt = sub.last_prompt();
        let sent_max_tokens = sub.last_max_tokens();
        let sent_temperature = sub.last_temperature();

        // 記憶は**本番と同じ入口**から読む（ADR-0003 / 想起の入口はこれひとつ）。
        let known = runtime
            .block_on(fixture.store.recall_memories(fixture.character.id))
            .expect("記憶を読めること");
        runtime.block_on(fixture.store.close());
        assert_eq!(
            known.len(),
            SEEDED_MEMORIES,
            "前提: 記憶を入れた状態で測っていること（0 件だと予算が観測できない）"
        );

        // **器具とまったく同じ2行を通す。** 呼び名も綴りも直書き
        // （本番から引くと、本番が変わったとき期待値も一緒に動く）。
        let display_name = DisplayName::parse(DISPLAY_NAME).expect("正当な表示名であること");
        let prompt = build_extraction_prompt(&ExtractionInput {
            display_name: &display_name,
            user_message: USER_MESSAGE,
            character_message: REPLY,
            known_memories: &known,
            budget: extraction_prompt_budget_for_tests(),
            example_style: ExampleStyle::Balanced,
        });
        assert_eq!(
            prompt.included_memories(),
            FITTING_MEMORIES,
            "前提: 予算が観測できる形であること（全部載る／1件も載らないと予算が出力に出ない）: \
             予算 = {}",
            extraction_prompt_budget_for_tests()
        );
        let request = extraction_request_for_tests(prompt.into_messages())
            .expect("器具の経路でも §8 の門を通ること");

        assert_eq!(
            joined(&request.messages),
            sent_prompt,
            "器具が組む依頼が本番と違う（器具は本番と別のプロンプトで 21b を回す）"
        );
        assert_eq!(
            request.max_tokens, sent_max_tokens,
            "器具の応答上限が本番と違う（腕どうしの比較の前提が崩れる）"
        );
        assert_eq!(
            request.temperature, sent_temperature,
            "器具の温度が本番と違う（腕どうしの比較の前提が崩れる）"
        );
        assert_eq!(
            sent_temperature,
            Some(0.8),
            "抽出の温度を明示すること（上流の既定に落ちると、どの設定で測ったかが言えなくなる / \
             ADR-0038 §決定2）。**0 にはしないこと**——器具の繰り返しが同じ出力になり、\
             検出力の源が消える"
        );
    }

    /// **記憶を読めなかったら、知らないまま走らせない**（作業単位 13c-3b の判断 / [`run`] の doc）。
    ///
    /// 走らせなかった回に失われるのは**その1往復ぶんの事実だけ**だが、
    /// 知らないまま入った重複は**自動では消えない**
    /// （外すには人が1件ずつ訂正版で上書きする / ADR-0022。[`run`] の doc）。
    /// しかも読めない原因はそのキャラクターについて持続するので、
    /// 握り潰すと**以後の全往復が控除リスト無しで走る**——§13 が塞ごうとしている状態そのもの。
    ///
    /// **失敗が必ずログに残ること**まで見る。残らないと
    /// 「記憶を渡しているのに重複が減らない」ときに原因を辿れない。
    #[test]
    fn known_memories_that_cannot_be_read_stop_the_extraction() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("unreadable.sqlite")));

        // **`store` 経由では書けない行。** 検証を後から厳しくすると現実に生まれる形
        // （`store::memory::recall_memories` の doc）。SQL は**プレースホルダ**（絶対制約）。
        runtime.block_on(async {
            sqlx::query("INSERT INTO memory (character_id, subject, content) VALUES (?, ?, ?)")
                .bind(fixture.character.id.get())
                .bind("user")
                .bind("   ")
                .execute(fixture.store.pool())
                .await
                .expect("行を入れられること");
        });

        let sub = MockSub::replying(&[&format!("- 利用者: {USER_FACT}")]);
        let extractor = extractor_of(&fixture.store, &sub);
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

        // **`recall_memories` は落ちるので、数えるのは行の側で。**
        let rows: i64 = runtime.block_on(async {
            sqlx::query_scalar("SELECT COUNT(*) FROM memory WHERE character_id = ?")
                .bind(fixture.character.id.get())
                .fetch_one(fixture.store.pool())
                .await
                .expect("行を数えられること")
        });
        runtime.block_on(fixture.store.close());

        assert_eq!(
            sub.call_count(),
            0,
            "すでに知っていることを読めないまま上流を呼ばないこと: {output:?}"
        );
        assert_eq!(rows, 1, "記憶を1件も足さないこと（入れたら消せない）");
        // **理由と深刻度は、走らなかった記録と同じ行で見る**（作業単位 B-2）。
        let skipped = skip_record(&output);
        // **綴りはテスト側に持つ**（`label()` を呼ぶと綴りを変える変異が両側で動く）。
        assert!(
            has_field(skipped, "skipped=memories_unreadable"),
            "理由が「記憶を読めなかった」であること: {skipped:?}"
        );
        assert!(
            skipped.contains("WARN"),
            "故障として目立つ水準に残すこと（握り潰さない）: {skipped:?}"
        );
        assert!(
            !output.contains(RAN_MESSAGE),
            "走らなかった回を「抽出しました」と記録しないこと: {output:?}"
        );
    }

    /// **記憶があっても、長すぎる会話は丸ごと捨てる**（ADR-0021 §13「§8 の挙動を変えない」）。
    ///
    /// # フィクスチャがこの状態に一度も入っていなかった
    ///
    /// §8 を見る2本（`a_conversation_over_the_limit_is_dropped_whole_and_counted` /
    /// `a_request_one_byte_over_the_limit_is_dropped`）は**記憶が0件**で走る。
    /// つまり「記憶があるときに §8 がどうなるか」を見ている検査が無い。
    ///
    /// # この見張り番が実際に固定していること
    ///
    /// **記憶があっても、理由は `conversation_too_long` のままで、記憶が1件も増えない。**
    ///
    /// 以前ここには「落ちる先は2つあり、どちらも実装として書けてしまう」と書いてあったが、
    /// **どちらも原理的に起きない**（作業単位 13c-3b の是正 / レビュアーの指摘）:
    ///
    /// * 「記憶を落として**入るようにしてから送る**」——`build_extraction_prompt` は
    ///   記憶を**会話を入れた残りからしか取らない**ので、会話だけで予算を超える回の
    ///   取り分は 0。**依頼は記憶ゼロの依頼と同じもの**になり、落とす余地が無い。
    /// * 「記憶があるぶん**早く門に当たる**」——同じ理由で起きない
    ///   （起きたらそれは記憶が会話を押し出している）。
    ///
    /// > **⚠ だからといって、この1本を「等価だから要らない」と読まないこと。**
    /// > 上の議論が成り立つのは **`budget` が §8 の門と同じ定数である間**だけで、
    /// > その前提を見張るのは別の1本
    /// > （[`a_request_that_fills_its_budget_with_memories_still_passes_the_gate`]）。
    /// > **2つで1組。** 片方だけを残すと、もう片方が押さえていた領域が空く。
    #[test]
    fn a_conversation_over_the_limit_is_dropped_whole_even_with_known_memories() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("too-long-with-known.sqlite")));
        runtime.block_on(seed_known_memories(&fixture.store, fixture.character.id));

        let long_message = "あ".repeat(FALLBACK_EXTRACTION_PROMPT_TOKENS as usize);
        let sub = MockSub::replying(&[&format!("- 利用者: {USER_FACT}")]);
        let extractor = extractor_of(&fixture.store, &sub);
        let extraction = Extraction::new(Arc::clone(&extractor), &fixture.character);

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                extraction.start(
                    fixture.session_id,
                    crate::domain::MessageId::from_row(2),
                    &long_message,
                    REPLY,
                );
                extractor.wait_for_the_running_task().await;
            });
        });
        let memories = runtime
            .block_on(fixture.store.recall_memories(fixture.character.id))
            .expect("記憶を読めること");
        runtime.block_on(fixture.store.close());

        assert_eq!(
            sub.call_count(),
            0,
            "記憶があっても長すぎる会話は送らないこと（記憶を削って送らない）: {output:?}"
        );
        assert_eq!(
            memories.len(),
            SEEDED_MEMORIES,
            "断片から記憶を作らないこと（元の件数のままであること）"
        );
        let skipped = skip_record(&output);
        assert!(
            has_field(skipped, &format!("skipped={}", SkipReason::TooLong.label())),
            "理由は今までどおり「会話が長すぎる」であること: {skipped:?}"
        );
    }

    /// **予算いっぱいまで記憶を詰めた依頼が、そのまま §8 の門を通る**
    /// （＝ `budget` に渡す数と門の数が同じ定数であること / 作業単位 13c-3b の是正）。
    ///
    /// # ここが塞いでいる穴
    ///
    /// `run` は `build_extraction_prompt` に `budget` を渡し、
    /// **組み上がった依頼を同じ [`FALLBACK_EXTRACTION_PROMPT_TOKENS`] で門にかける。**
    /// この2つが同じ定数であることを、着手前は**誰も見張っていなかった**（実測 / 2026-08-22）:
    ///
    /// | `budget` に渡す数 | スイート |
    /// |---|---|
    /// | `FALLBACK_EXTRACTION_PROMPT_TOKENS - EXTRACTION_RESPONSE_TOKENS`（7680） | 1076 passed / 0 failed |
    /// | `FALLBACK_EXTRACTION_PROMPT_TOKENS + 1000`（9192） | 1076 passed / 0 failed |
    ///
    /// **上振れが実害のある向き。** `build_extraction_prompt` は**取り分を使い切るまで**
    /// 記憶を詰めるので、予算が門より大きいと、記憶を十分に持つキャラクターでは
    /// **組み上がった依頼が門の上に着地する。** すると門が毎ターン丸ごと捨て、
    /// ログには `skipped=conversation_too_long` だけが並ぶ——
    /// **会話は短いのに「会話が長すぎる」と記録され、そのキャラクターの抽出が静かに止まる。**
    /// 実測（157 件保存 / ADR-0021）の規模なら確実に到達する。
    ///
    /// # 見えなかった理由は「その状態に一度も入っていない」
    ///
    /// [`the_request_carries_the_newest_known_memories_that_fit_the_budget`] は
    /// 1件 1732 トークンの記憶を 7 件しか置かないので、**取り分が 675 余る。**
    /// 余りがあるうちは、予算を動かしても依頼の大きさは門から離れたままで、
    /// **門との関係が観測にかからない**（CLAUDE.md
    /// 「フィクスチャが『その分岐が効く状態』に一度も入らない」）。
    ///
    /// **小さい記憶を多数置いて取り分を使い切る**のがこの見張り番の役目
    /// （[`FILLING_MEMORIES`] / [`FILLING_MEMORY_FILLER_CHARS`]）。
    ///
    /// # 実測（2026-08-22 / 予算 = 上限 = 8192）
    ///
    /// 削れない枠 2079、取り分 6113、記憶1件 249、節の枠 242。
    /// **23 件が載り、11 件が落ち、依頼は 8048。** 余りは 144。
    ///
    /// | `budget` に渡す数 | 依頼 | この検査 |
    /// |---|---|---|
    /// | `FALLBACK_EXTRACTION_PROMPT_TOKENS`（8192） | 8048 | 緑 |
    /// | ＋1000（9192） | 9041 | **赤**（門が捨てて上流へ行かない） |
    /// | −512（7680） | 7550 | **赤**（門から 642 も離れる＝予算を使い切っていない） |
    ///
    /// > **⚠ この表を測ったのは `EXTRACTION_RESPONSE_TOKENS` が 512 だった時点**
    /// > （作業単位 24 で 320 になった）。**−512 の行は「7680 を渡したとき」と読むこと。**
    /// > 依頼の大きさは予算だけで決まるので、上の3行はそのまま成り立つ。
    ///
    /// **門を「記憶を入れない依頼」で測る変異と組にしても赤**（実測 / 2026-08-22）。
    /// **見ているのが門の判定ではなく、現に送られた依頼の大きさ**だから
    /// ——組にすると門はすり抜けるが、9041 トークンの依頼が上流へ届いてしまう。
    /// （その変異を**単独で**当てると緑。`budget` が門と同じ定数である限り受理集合が同じで、
    /// 実害が出るのは上の +1000 と**組にしたときだけ**。だから 2 つを 1 本で捕まえる。）
    #[test]
    fn a_request_that_fills_its_budget_with_memories_still_passes_the_gate() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("budget-filled.sqlite")));
        runtime.block_on(seed_memories(
            &fixture.store,
            fixture.character.id,
            FILLING_MEMORIES,
            FILLING_MEMORY_FILLER_CHARS,
        ));
        let sub = MockSub::replying(&[""]);
        let extractor = extractor_of(&fixture.store, &sub);
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
        runtime.block_on(fixture.store.close());

        let frame = tokens_of_request(USER_MESSAGE, REPLY);
        // **短い会話なのに「会話が長すぎる」で捨てられていないこと**（上振れの壊れ方そのもの）。
        assert!(
            !output.contains(SKIPPED_MESSAGE),
            "予算いっぱいに詰めた依頼を門が捨てている（予算が門より大きい）: \
             枠 = {frame}, 上限 = {FALLBACK_EXTRACTION_PROMPT_TOKENS}, {output:?}"
        );
        assert_eq!(
            sub.call_count(),
            1,
            "予算に収めた依頼は上流へ行くこと: {output:?}"
        );

        let tokens = request_tokens(&sub.last_request());
        assert!(
            tokens <= FALLBACK_EXTRACTION_PROMPT_TOKENS,
            "組み上がった依頼が門を超えないこと（予算が門より大きい）: \
             依頼 = {tokens}, 上限 = {FALLBACK_EXTRACTION_PROMPT_TOKENS}, 枠 = {frame}"
        );
        assert!(
            tokens > FALLBACK_EXTRACTION_PROMPT_TOKENS.saturating_sub(BUDGET_FILL_MARGIN),
            "取り分を使い切っていない（予算が門より小さい）: \
             依頼 = {tokens}, 上限 = {FALLBACK_EXTRACTION_PROMPT_TOKENS}, 枠 = {frame}"
        );

        // **前提: 一部だけが載る形で測っていること。**
        // 全件載る（余りが読めない）／1件も載らない（詰めていない）と、上の2つが意味を失う。
        // **前提でも行に切ってから読む**（作業単位 B-21d / [`field_number`] の doc）。
        let recorded = ran_record(&output);
        let included = field_number(recorded, "known_included");
        let dropped = field_number(recorded, "known_dropped");
        assert!(
            included > 0 && dropped > 0,
            "前提: 予算で切れている形であること: included = {included}, dropped = {dropped}"
        );
        assert_eq!(
            included + dropped,
            FILLING_MEMORIES,
            "前提: 入れた記憶が全部どちらかに数えられていること"
        );
    }

    /// **「すでに知っていること」の2欄は、`warn` の回にも同じ形で出る**（[`record_run`] の doc）。
    ///
    /// # なぜ水準ごとに見るのか
    ///
    /// `record_run` は水準で分岐して**同じ欄を2箇所に書いている。**
    /// 片方にだけ欄を足すと、**集計が水準で割れる**——しかも割れる先は
    /// **`domain` が弾いた行があった回**、つまり「プロンプトかモデルを直すべき」と
    /// 分かる**いちばん読みたい回**の側。
    ///
    /// 実測（2026-08-22）: `warn!` 側から `known_included` / `known_dropped` を
    /// 落とす変異が **1076 passed / 0 failed** で緑を通った
    /// （`emphasis_stripped` は 13c-1 の見張り番が捕まえていた）。
    #[test]
    fn the_known_memory_counts_are_recorded_at_the_warn_level_too() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("known-at-warn.sqlite")));
        runtime.block_on(seed_known_memories(&fixture.store, fixture.character.id));

        // **`domain` が弾く行を1つ混ぜる**（＝ `warn` の水準に落ちる回）。
        let too_long = "あ".repeat(MemoryContent::MAX_CHARS + 1);
        let reply = format!("- 利用者: {too_long}\n- あなた: {CHARACTER_FACT}");
        let sub = MockSub::replying(&[&reply]);
        let extractor = extractor_of(&fixture.store, &sub);
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
        runtime.block_on(fixture.store.close());

        assert_eq!(
            sub.call_count(),
            1,
            "前提: 上流へ行っていること: {output:?}"
        );
        // **水準も欄も、抽出の記録の行の中で見る**（作業単位 B-2）。
        // ここを出力全体で見ると、**別のイベントの `WARN` で「前提」が満たされ、
        // 測っている水準が `info` へずれていても気付けない。**
        let recorded = ran_record(&output);
        // **理由の件数は値として読む**（作業単位 B-12 / 上の
        // `a_rejection_from_the_domain_layer_is_recorded_at_warn` と同じ理由で
        // [`has_field`] は使えない）。深刻度は `キー=値` ではないので `contains`。
        assert_eq!(
            discard_count(recorded, DiscardReason::ContentTooLong),
            1,
            "前提: `domain` が弾いた回を測っていること: {recorded:?}"
        );
        assert!(
            recorded.contains("WARN"),
            "前提: warn の水準で測っていること: {recorded:?}"
        );
        assert!(
            has_field(recorded, &format!("known_included={FITTING_MEMORIES}")),
            "載せた件数は水準が変わっても出ること: {recorded:?}"
        );
        assert!(
            has_field(recorded, &format!("known_dropped={DROPPED_MEMORIES}")),
            "入らなかった件数は水準が変わっても出ること: {recorded:?}"
        );
    }

    /// **想起の内訳を抽出のイベントへ混ぜない**（ADR-0023 §3 / 作業単位 15b）。
    ///
    /// `prompt::build_prompt` は `recalled_recent` / `recalled_related` / `omitted` を
    /// **別のイベント**として書き出す。ここへ足すと、
    /// **破棄率の分母に「想起が引かなかった件数」が混ざって**、
    /// 過去の測定値と比べられなくなる（`record_run` の doc）。
    ///
    /// **逆向き（想起のイベントに破棄の欄が混ざらないこと）は
    /// `prompt` 側の `the_recall_counts_do_not_leak_into_the_discard_fields`。**
    /// 混ぜ方は2通りあるので、見張り番も2本要る。
    #[test]
    fn the_extraction_event_carries_no_recall_fields() {
        let reply = format!("- 利用者: {USER_FACT}\n- あなた: {CHARACTER_FACT}");
        let (output, memories) = extract_once(MockSub::replying(&[&reply]), USER_MESSAGE, REPLY);

        assert_eq!(
            memories.len(),
            2,
            "前提: 抽出できた回であること: {output:?}"
        );
        assert!(
            output.contains(RAN_MESSAGE),
            "前提: 抽出のイベントが出ていること: {output:?}"
        );
        for field in [
            "recalled_recent",
            "recalled_related",
            "omitted",
            "excluded_own",
        ] {
            assert!(
                !output.contains(field),
                "想起の欄が抽出のイベントに混ざっている: {field} / {output:?}"
            );
        }
    }

    /// **記憶を読めなかった回は、原因を `#[source]` ごとログに残す**（[`record_skip`] の doc）。
    ///
    /// # `error` だけでは何も分からない
    ///
    /// `StoreError` の `Display` は**定数文字列**（絶対制約: 内部パスも接続文字列も
    /// レスポンスに出さない）なので、`error=` から読めるのは「DB 操作が失敗した」までで、
    /// **接続断なのか I/O なのかは `detail`（`Debug` ＝ `#[source]` 側）にしか無い。**
    ///
    /// # `InvalidStoredValue` では実害が測れない
    ///
    /// [`known_memories_that_cannot_be_read_stop_the_extraction`] が起こすのは
    /// `InvalidStoredValue` で、**`Display` も `Debug` も定数相当。**
    /// つまりあの経路では2欄を落としても失うものが無く、実測でも
    /// **`Some(err)` 腕から `error` / `detail` を落とす変異が 1076 passed / 0 failed**
    /// で緑を通った（2026-08-22）。
    ///
    /// **`StoreError::Query` を起こして測る。** プールを閉じてから走らせると
    /// `recall_memories` が `Query(PoolClosed)` を返すので、
    /// **原因が `detail` にしか現れない形**になる。
    #[test]
    fn a_store_failure_while_reading_memories_keeps_its_cause_in_the_log() {
        /// `StoreError::Query` の `Display`。**定数文字列**なので原因を含まない。
        const QUERY_DISPLAY: &str = "データベース操作に失敗しました";
        /// `Debug` にだけ現れる原因。**`detail` を落とすと消える唯一の手掛かり。**
        const CLOSED_POOL_CAUSE: &str = "PoolClosed";

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join("closed-pool.sqlite")));
        // **プールを閉じる**（＝ `recall_memories` が `StoreError::Query` を返す状態）。
        // 実機では接続断・I/O 障害がこの形で来る。
        runtime.block_on(fixture.store.close());

        let sub = MockSub::replying(&[&format!("- 利用者: {USER_FACT}")]);
        let extractor = extractor_of(&fixture.store, &sub);
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

        assert_eq!(
            sub.call_count(),
            0,
            "読めないまま上流を呼ばないこと: {output:?}"
        );
        // **深刻度も原因も、走らなかった記録と同じ行で見る**（作業単位 B-2）。
        // `error=` / `detail=` は接頭辞を持たない共通の綴り（[`record_skip`] の doc）なので、
        // **出力全体で見ると他のモジュールの同名の欄でも満たされてしまう。**
        let skipped = skip_record(&output);
        assert!(
            has_field(skipped, "skipped=memories_unreadable"),
            "前提: 記憶を読めなかった回として記録していること: {skipped:?}"
        );
        assert!(
            skipped.contains("WARN"),
            "前提: 故障として目立つ水準に出ていること: {skipped:?}"
        );
        assert!(
            has_field(skipped, &format!("error={QUERY_DISPLAY}")),
            "エラーの Display を残すこと: {skipped:?}"
        );
        assert!(
            skipped
                .split_whitespace()
                .any(|token| token.starts_with("detail=") && token.contains(CLOSED_POOL_CAUSE)),
            "原因（#[source] 側）を残すこと——Display は定数なのでここにしか無い: {skipped:?}"
        );
    }

    /// **予算・門・ログ欄の3つが、器から出た同じ1つの上限を見ている**
    /// （作業単位 B-7 のレビュー指摘1 / 2026-08-27）。
    ///
    /// # ここが塞いでいる穴
    ///
    /// [`a_request_that_fills_its_budget_with_memories_still_passes_the_gate`] は
    /// **落とし所の器**（`extractor_of`）で回っているので、そこでは
    /// `sub_context.prompt_limit()` と `SubContext::FALLBACK.prompt_limit()` が
    /// **同じ式**になる。**値ではなく式の側で同値になっている**ため、
    /// 次の2つの変異が 1554 全緑で通り抜けた（レビュアーの実測 / 2026-08-27）:
    ///
    /// | 変異 | 実害 |
    /// |---|---|
    /// | `budget` を `SubContext::FALLBACK.prompt_limit()` に固定 | **狭い窓の機械で、記憶を十分に持つキャラクターの抽出が毎ターン止まる**（予算 8192・門 3072 で組むので必ず門に当たる） |
    /// | `record_skip` の `limit_tokens` を落とし所に固定 | **「上限を超えた」と言いながら超えていない数を並べる行**（[`record_skip`] の doc が名指しで避けた形） |
    ///
    /// **どちらも引ける環境（16384）では再現しない。**
    /// **B-7 が対象にした狭い機械でだけ出る**ので、ここで測る窓は
    /// **落とし所以外**でなければ意味がない。
    ///
    /// # 2つの腕
    ///
    /// | 窓 | 上限 | 見るもの |
    /// |---|---|---|
    /// | 12288（広い / 落とし所ではない） | 6144 | 予算いっぱいに詰めた依頼が**門を通る** ＋ 取り分を使い切っている |
    /// | 4096（狭い / 落とし所ではない） | 2048 | 捨てた行が **`limit_tokens=2048`**（同じ行の中で見る） |
    ///
    /// **落とし所での振る舞いは
    /// [`a_request_that_fills_its_budget_with_memories_still_passes_the_gate`] が持つ。**
    /// ここは「窓が変われば3つとも一緒に動く」ことだけを見る。
    #[test]
    fn the_budget_and_the_log_follow_the_shared_window_too() {
        /// 落とし所ではない**広い**窓。上限は 6144。
        const WIDE: u32 = 12_288;
        /// 落とし所ではない**狭い**窓。上限は 2048 で、削れない枠（実測 2339）が入らない。
        const NARROW: u32 = 4_096;

        for window in [WIDE, NARROW] {
            assert_ne!(
                window,
                SubContext::FALLBACK.tokens(),
                "前提: 測る窓が落とし所と別であること（同値だと式の取り違えを検出できない）"
            );
        }

        // --- 広い腕: 予算と門が同じ1つの上限から出ていること ---
        let wide = SubContext::new(WIDE).expect("抽出に使える窓であること");
        let (output, sub) = run_one_turn_with_window(wide, "budget-wide.sqlite");

        assert!(
            !output.contains(SKIPPED_MESSAGE),
            "予算いっぱいに詰めた依頼を門が捨てている（予算が門より大きい）: 上限 = {}, {output:?}",
            wide.prompt_limit()
        );
        assert_eq!(
            sub.call_count(),
            1,
            "予算に収めた依頼は上流へ行くこと: {output:?}"
        );

        let tokens = request_tokens(&sub.last_request());
        assert!(
            tokens <= wide.prompt_limit(),
            "組み上がった依頼が門を超えないこと（予算が門より大きい）: 依頼 = {tokens}, 上限 = {}",
            wide.prompt_limit()
        );
        assert!(
            tokens > wide.prompt_limit().saturating_sub(BUDGET_FILL_MARGIN),
            "取り分を使い切っていない（予算が門より小さい／落とし所の予算で組んでいる）: \
             依頼 = {tokens}, 上限 = {}",
            wide.prompt_limit()
        );
        // **前提: 一部だけが載る形で測っていること**（全件載ると余りが読めない）。
        // **前提でも行に切ってから読む**（作業単位 B-21d / [`field_number`] の doc）。
        let recorded = ran_record(&output);
        let included = field_number(recorded, "known_included");
        let dropped = field_number(recorded, "known_dropped");
        assert!(
            included > 0 && dropped > 0,
            "前提: 予算で切れている形であること: included = {included}, dropped = {dropped}"
        );
        assert_eq!(
            included + dropped,
            FILLING_MEMORIES,
            "前提: 入れた記憶が全部どちらかに数えられていること"
        );

        // --- 狭い腕: 捨てた行が、その回に効いていた上限を名乗ること ---
        let narrow = SubContext::new(NARROW).expect("抽出に使える窓であること");
        let (output, sub) = run_one_turn_with_window(narrow, "budget-narrow.sqlite");

        assert_eq!(
            sub.call_count(),
            0,
            "狭い窓では依頼を捨てること: {output:?}"
        );
        // **理由も上限も、捨てたことを告げる行の中で見る**（CLAUDE.md「同じ行の中で」）。
        let skipped = skip_record(&output);
        assert!(
            has_field(skipped, &format!("skipped={}", SkipReason::TooLong.label())),
            "理由は「会話が長すぎる」であること: {skipped:?}"
        );
        assert!(
            has_field(skipped, &format!("limit_tokens={}", narrow.prompt_limit())),
            "その回に効いていた上限を載せること（落とし所の数を出さないこと）: {skipped:?}"
        );
        // **超過分と上限が同じ行に並んで初めて「どれだけ超えたか」が読める。**
        let prompt_tokens = field_number(skipped, "prompt_tokens");
        assert!(
            u32::try_from(prompt_tokens).is_ok_and(|tokens| tokens > narrow.prompt_limit()),
            "並んだ2つの数が「超えた」と読めること: {skipped:?}"
        );
    }

    /// **与えた窓で1往復ぶんの抽出を走らせる**（作業単位 B-7）。
    ///
    /// 記憶は [`FILLING_MEMORIES`] 件——**取り分を使い切る**フィクスチャなので、
    /// 予算が窓から来ているかどうかが依頼の大きさに現れる。
    fn run_one_turn_with_window(window: SubContext, db: &str) -> (String, Arc<MockSub>) {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let runtime = runtime();
        let fixture = runtime.block_on(fresh(&dir.path().join(db)));
        runtime.block_on(seed_memories(
            &fixture.store,
            fixture.character.id,
            FILLING_MEMORIES,
            FILLING_MEMORY_FILLER_CHARS,
        ));
        let sub = MockSub::replying(&[""]);
        let extractor = extractor_with_sub_context(
            &fixture.store,
            &sub,
            super::super::context::SharedSubContext::for_tests(window),
        );
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
        runtime.block_on(fixture.store.close());

        (output, sub)
    }
}
