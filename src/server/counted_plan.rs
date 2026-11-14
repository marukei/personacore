//! **数えた計画**（[ADR-0052](../../docs/adr/0052-count-with-the-upstream-tokenizer.md) §推奨2〜7 /
//! 作業単位 B-60d-1）。
//!
//! 部品を上流に数えさせて計画を立て（§推奨2）、送る直前に組み上がりを数えさせ（封 / §推奨3）、
//! **封の数が取り分以下のときだけ数えた計画を送る。** 示せなければ今のバイト数の計画（§推奨1 の (i)）で送る。
//!
//! # 1ターンの流れ
//!
//! | 段 | 何をするか | 往復（llama） |
//! |---|---|---|
//! | 0 | この世代を「数えられない」と覚えているか、**掛け金が掛かっていれば**（`封 < 実` / `super::seal` の「第3層の封の腕」）、**バイト数の計画をそのまま送る**（部品も封も聞かない） | 0 |
//! | 1 | `prompt::parts_to_count` の一覧のうち、**覚えていない部品だけを数える** | 新しい部品の数（`/tokenize` 1つずつ） |
//! | 2 | 1つでも数えた部品があれば、数えた表で計画を組む（**本文と裏方が同じ上流なら窓を差し引く** / §決定7） | 0 |
//! | 3 | 封をする。**取り分以下なら送る** | 2（`/apply-template` ＋ `/tokenize`） |
//! | 4 | 超えたら、**1回だけ**組み直して封をし直す。取り分以下なら送る | 2 |
//! | 5 | それでも超えた・数えられなかった・往復が失敗した → **バイト数の計画で送る**（封をしない） | 0 |
//!
//! **1ターンに増える往復の最大は「新しい部品の数 ＋ 4」**（封2回 × 2往復）。
//! 温まったターンの新しい部品は、ふつう今回の発言と直前の応答と、その間に増えた記憶だけである。
//! M2（ADR-0052 §測った結果）では 15k 級の封1回が約 120ms。
//!
//! 何も数えられなかった回（段1で全部が数えられない）は、**段2を飛ばしてバイト数の計画に封をする**。
//! 封は「数えられない」を覚えるため（`super::seal`）で、送る本文は今と1バイトも変わらない
//! （見張り番は `tests/server_seal.rs` の `an_upstream_that_cannot_count_is_sent_exactly_the_byte_plan`）。
//!
//! # 部品の数の記憶（§推奨6）
//!
//! [`PartMemory`]。**DB に持たない**（トークナイザに依存する数を行に残さない）。
//!
//! | 項目 | 決めたこと |
//! |---|---|
//! | 鍵 | **部品の文字列そのもの**（`prompt::CountedParts` と同じ。無害化と引用を通した後の行） |
//! | 世代 | **面の世代**（`super::context::FaceGeneration`）。世代の違う記憶は使わず、書くときに捨てる |
//! | 大きさの上限 | **キャラクターごとに、直近のターンの部品だけ**を残す（新しい定数を足さない —— 上限は「そのターンが DB から読んだ入力の大きさ」に付いてくる） |
//! | 捨て方 | 書くたびに、**そのキャラクターの表を今回の部品だけで置き換える**。前のターンにだけあった部品（落ちた履歴・置き換えた記憶）はそこで消える |
//!
//! 捨てても遅くなるだけ（次に要るときに数え直す）。**同じキャラクターの別のセッションへ移ると
//! 履歴の部品は数え直しになる**（記憶と削れない枠は共通なので残る）。
//!
//! # 未計測の部品を数える並び（§推奨6「同時に投げる数に上限を置く」）
//!
//! **最初の1つを単独で聞き、数えられたら残りを [`PART_COUNT_CONCURRENCY`] 本ずつ並べて聞く。**
//! **数えられない・往復が失敗した答えが1つでも返ったら、そこで止める**（残りはバイト数のまま）。
//!
//! * **最初の1つを単独にする理由**: 数える口を持たない上流（LM Studio）に、最初のターンで
//!   部品の数だけ往復を投げないため。**世代ごとに1往復**で「数えられない」と分かる。
//! * **止める理由**: 往復の失敗（時間切れ）を部品ごとに待つと、上限（`context_timeout` = 最大 5 秒）×
//!   部品の数 ÷ 並びの数だけターンが止まる。止めれば、待つのは並べた分の1回だけになる。
//!
//! 止めた回は、その回の計画が「同じ入力から同じプロンプト」にならない（ADR-0038 の再現性）。
//! **だから件数を封の行に出す**（`uncounted_parts` / `super::seal::report`）—— 測定ではその回を除ける。
//!
//! # 封が超えたときの「超えた分だけ狭めた予算」
//!
//! [`narrowed`]。**組み直しの取り分 = 1回目の計画の和 − 超えた分**（`超えた分 = 封 − 取り分`）。
//!
//! 窓（取り分）から超えた分を引く形にしないのは、**計画が取り分を使い切っていない回に効かない**から
//! （計画の和が取り分より小さければ、狭めた取り分にも同じ計画が収まり、同じ本文に同じ封をすることになる）。
//! 計画の和から引くと、**「封 − 計画の和」（部品の和が数え損ねた分）が一定なら、2回目の封はちょうど取り分に収まる**
//! （見張り番は `tests/server_seal.rs` の `a_prompt_is_sent_only_after_the_upstream_counted_it_within_the_allowance`）。
//!
//! # 本文と裏方が同じ上流のとき（§決定7 / M4 で「壊れる」側と決まった）
//!
//! [`WindowSharing`] と [`counted_budget`]。**数えた計画の窓からだけ**、裏方の1要求分
//! （`SubContext::prompt_limit()` ＋ 裏方の各依頼の応答の上限のうち最大 / [`background_request`]）を差し引く。
//! **バイト数の計画は差し引かない**
//! （§決定4「数えられないときは今と同じ」。バイト数の計画は実トークンで窓の約2割しか使わない）。
//!
//! ## 分け合われているかを、上流の答えから判別しない
//!
//! M4（ADR-0052 §測った結果）で、**統合 KV（`-np` 自動）のときも `-np 4` を明示したときも、
//! `/props`・`/slots`・`meta.n_ctx` は「1要求が使える上限」を言う**ことが分かっている
//! （統合 KV では 16384、`-np 4` では 4096）。**言っている値からは、その窓が他の要求と分け合われているかが読めない。**
//! 分け合わないと言えるのはスロットが1つのときだけだが、`/props` の `total_slots` の形はこの版で測っていない。
//! **だから判別できないものとして扱い、同じ上流なら差し引く**（遅くなるだけの側 / ADR-0036）。
//! 判別の材料は**第2層の照会の結果の宛先**（`super::upstream::UpstreamFacts` の面ごとの `endpoint`。
//! 差し替えで書き替わる）。綴りの一致（`server::App::bind` の `one_upstream` と同じ規則）に加えて、
//! **ループバック同士でポートが同じなら同じ上流**とみなす（[`WindowSharing::of_endpoints`]）。
//!
//! # 封と送信は同じ写し（§推奨3）
//!
//! 部品を数える・封をする・送るは、**呼び出し側（`super::chat`）が1ターンに1回だけ取った写し**
//! （`AppState::llm_for_turn`）で行う。この関数は写しを取り直さない。

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures_util::StreamExt;

use super::context::FaceGeneration;
use super::extract::{EXTRACTION_RESPONSE_TOKENS, SubContext};
use super::seal::{Outcome, PartsTally, Seal, SentPlan, TurnSeal, UncountableMemory};
use super::upstream::UpstreamFacts;
use crate::domain::DriftForm;
use crate::llm::{ChatRequest, LlmClient, ReasoningEffort, TokenCount};
use crate::prompt::{
    BudgetPlan, BuiltPrompt, CountedParts, PromptError, PromptInput, TokenBudget,
    build_prompt_quiet, parts_to_count,
};

/// **未計測の部品を同時に数える数の上限**（2本目以降。最初の1つは単独 / モジュールの doc）。
///
/// # ADR-0036 の判定: 外したとき「遅くなるだけ」
///
/// * 小さすぎる → 冷えた最初のターン（起動・プロファイルの差し替えの直後）の部品を数える時間が延びる。
/// * 大きすぎる → 上流の HTTP の処理を取り合う（M2: `/tokenize` は生成の列には並ばない）。
///   上限を持たない（全部を一度に投げる）と、記憶が数百件ある冷えたターンで数百本が同時に出る。
///
/// **どちらでも壊れない**（部品の数は計画にだけ使い、安全は封が持つ）。**設定にしない**
/// （ADR-0036 §決定3: 根拠が無くても、外して壊れない値はつまみにしない）。
/// **根拠の測定は無い**（この機・この上流で測っていない）。M3・M6 で冷えたターンの時間を測るなら、そこで見直す。
const PART_COUNT_CONCURRENCY: usize = 4;

/// **本文と裏方の窓の関係**（ADR-0052 §決定7）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowSharing {
    /// **本文と裏方が別の上流。** 窓を分け合わない（差し引かない）。
    Separate,
    /// **本文と裏方が同じ上流。** 窓が要求の間で分け合われているかは判別できない
    /// （モジュールの doc）ので、**分け合われているものとして差し引く。**
    SameUpstream,
}

impl WindowSharing {
    /// **第2層の照会の結果の宛先から決める**（[`Self::of_endpoints`]）。
    pub(super) fn of(facts: &UpstreamFacts) -> Self {
        Self::of_endpoints(facts.main().endpoint(), facts.sub().endpoint())
    }

    /// **2つの宛先が同じ上流か。** 綴りが一致するか、ループバック同士でポートが同じときだけ「同じ」とし、
    /// **それ以外（読めない綴り・ループバック以外の別名）は「別」とする。**
    ///
    /// | 宛先 | 判定 |
    /// |---|---|
    /// | 綴りが同じ（`App::bind` の `one_upstream` と同じ規則。正規化後のバイト一致） | 同じ |
    /// | **どちらもループバック**（`localhost`・`127.0.0.0/8`・`[::1]`）で、**ポートが同じ** | 同じ（パスは見ない —— 同じ口で待っているのは同じサーバ） |
    /// | それ以外（読めない綴り・ループバック以外の別名を含む） | 別 |
    ///
    /// **名前解決はしない**（`localhost` 以外の名前は綴りで比べる）。ループバック以外の
    /// 別名（同じ機の LAN の IP 等）は「別」に倒れ、差し引かない側になる —— 気付けない形として残る。
    pub(super) fn of_endpoints(main: &str, sub: &str) -> Self {
        if main == sub || same_loopback_port(main, sub) {
            Self::SameUpstream
        } else {
            Self::Separate
        }
    }
}

/// **どちらもループバックで、ポートが同じか**（[`WindowSharing::of_endpoints`]）。読めない綴りは `false`。
fn same_loopback_port(main: &str, sub: &str) -> bool {
    match (loopback_port(main), loopback_port(sub)) {
        (Some(main), Some(sub)) => main == sub,
        _ => false,
    }
}

/// **ループバックの宛先ならそのポート**（書いていなければ方式の既定 80 / 443）。それ以外・読めない綴りは `None`。
///
/// **URL のクレートを使わずに読む** —— `reqwest` の名前を `llm` の外に出すと、上流と直に話す道具の
/// 迂回路として `llm::upstream_leaks` の網3に数えられる。読むのは「方式・userinfo・ホスト・ポート」だけで、
/// 宛先の綴りは起動時の設定（`config::normalize_base` を通った後）なので、この粗さで足りる。
fn loopback_port(raw: &str) -> Option<u16> {
    let lower = raw.to_ascii_lowercase();
    let (rest, default_port) = match lower.strip_prefix("http://") {
        Some(rest) => (rest, 80),
        None => (lower.strip_prefix("https://")?, 443),
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;
    let (host, port) = if let Some(bracketed) = host_port.strip_prefix('[') {
        let (host, after) = bracketed.split_once(']')?;
        (host, after.strip_prefix(':'))
    } else {
        match host_port.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_port, None),
        }
    };
    let loopback = host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if !loopback {
        return None;
    }
    match port {
        Some(port) => port.parse().ok(),
        None => Some(default_port),
    }
}

/// **裏方の1要求分**（ADR-0052 §決定7）。**依頼の上限 ＋ 裏方の各依頼の応答の上限のうち最大のもの。**
///
/// 裏方（抽出・変容・軸の候補・軸の試用）は**1本ずつ直列に**走る（`MemoryExtractor::spawn` / `run`）ので、
/// 同時に窓を取り合うのは1要求分である。依頼の上限はどれも `SubContext::prompt_limit()`
/// で同じ。応答の上限は依頼ごとに違うので、**大きい方を取る**:
///
/// | 依頼 | 応答の上限の出どころ |
/// |---|---|
/// | 抽出 | `EXTRACTION_RESPONSE_TOKENS`（320） |
/// | 変容 | `server::drift` の `drift_response_tokens`。**全軸の形（既定）で軸の本数ぶん**（1軸あたり 86。5本で 430） |
/// | 軸の候補 / 軸の試用 | `server::axis_discovery` / `server::axis_trial` の `response_tokens`（1行ぶん） |
/// | 約束・決まりの提案（検出・確かめ） | `prompt::agreement_proposal::PROPOSAL_MAX_TOKENS`（320。抽出と同じ値なので、**差し引く量は変わらない** / 作業単位 A-12c-1） |
///
/// **変容の形（`PERSONACORE_DRIFT_FORM`）と、キャラクターの変容が止まっているかは見ない**
/// —— 全軸の形の上限は1軸の形の上限以上なので、大きい方を取れば判別を増やさずに済む（差し引き過ぎは遅くなるだけ）。
/// `axes` は**DB にある全キャラクターの性格の軸の本数の最大**（`Store::max_stat_axes`。値 0 の軸も数える ——
/// 変容の依頼には載る / ADR-0027 §決定4）。本文と重なる裏方は直前のターンのもので、
/// 別のキャラクターの仕事でありうる（裏方の枠はキャラクターをまたいで1つ）ので、話している相手の本数では足りない。
/// **試用中の候補では変容の依頼の軸は増えない**（`prompt::drift` / `the_fixed_axes_drift_the_same_with_or_without_a_trial`）。
pub(super) fn background_request(sub: SubContext, axes: usize) -> u32 {
    let response = EXTRACTION_RESPONSE_TOKENS
        .max(super::drift::drift_response_tokens(
            DriftForm::All,
            axes,
            sub,
        ))
        .max(super::axis_discovery::response_tokens(sub))
        .max(super::axis_trial::response_tokens(sub))
        .max(crate::prompt::agreement_proposal::PROPOSAL_MAX_TOKENS);
    sub.prompt_limit().saturating_add(response)
}

/// **数えた計画に使う予算と、差し引いた量。**
///
/// | 窓の関係 | 予算 | 差し引いた量 |
/// |---|---|---|
/// | 別の上流 | そのまま | 0 |
/// | 同じ上流 | 窓から裏方の1要求分を引いた予算。**予算にならなければ `None`**（数えた計画を組まず、バイト数の計画で送る） | 裏方の1要求分 |
///
/// **応答の余白は変えない**（`max_tokens` は呼び出し側が元の予算から決める）。
pub(super) fn counted_budget(
    budget: TokenBudget,
    sharing: WindowSharing,
    sub: SubContext,
    axes: usize,
) -> (Option<TokenBudget>, u32) {
    match sharing {
        WindowSharing::Separate => (Some(budget), 0),
        WindowSharing::SameUpstream => {
            let deduction = background_request(sub, axes);
            let narrowed = budget
                .context_tokens()
                .checked_sub(deduction)
                .and_then(|context| TokenBudget::new(context, budget.response_tokens()).ok());
            (narrowed, deduction)
        }
    }
}

/// **超えた分だけ狭めた予算**（モジュールの doc の「超えた分だけ狭めた予算」）。
///
/// 組み直しの取り分 = `min(計画の和, 取り分) − (封 − 取り分)`。**封が取り分以下なら `None`**
/// （組み直す理由が無い）。取り分が 0 以下になる・予算にならないときも `None`（バイト数の計画へ戻る）。
pub(super) fn narrowed(budget: TokenBudget, planned: u32, sealed: u32) -> Option<TokenBudget> {
    let allowance = budget.prompt_allowance();
    let over = sealed.checked_sub(allowance).filter(|over| *over > 0)?;
    let target = planned
        .min(allowance)
        .checked_sub(over)
        .filter(|target| *target > 0)?;
    let response = budget.response_tokens();
    TokenBudget::new(target.checked_add(response)?, response).ok()
}

/// **部品の数の記憶**（モジュールの doc の表）。キャラクターの id ごとに、直近のターンの部品だけを持つ。
///
/// **錠は `await` を跨いで持たない**（読む・書くの1手ずつ）。毒された錠でも中身を使う
/// （数の記憶が壊れる害は「数え直す」だけ）。
#[derive(Debug, Default)]
pub(super) struct PartMemory(Mutex<HashMap<i64, PartTable>>);

/// 1キャラクターぶんの部品の数。**数えたときの世代を持つ。**
#[derive(Debug)]
struct PartTable {
    generation: FaceGeneration,
    tokens: HashMap<String, u32>,
}

impl PartMemory {
    /// **覚えている数と、数えに行く部品**（`wanted` の順）。世代の違う記憶は使わない。
    fn known(
        &self,
        character: i64,
        generation: &FaceGeneration,
        wanted: &BTreeSet<String>,
    ) -> (HashMap<String, u32>, Vec<String>) {
        let tables = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let table = tables
            .get(&character)
            .filter(|table| table.generation == *generation);
        let mut known = HashMap::new();
        let mut missing = Vec::new();
        for part in wanted {
            match table.and_then(|table| table.tokens.get(part)) {
                Some(tokens) => {
                    known.insert(part.clone(), *tokens);
                }
                None => missing.push(part.clone()),
            }
        }
        (known, missing)
    }

    /// **そのキャラクターの表を、今回の部品の数だけで置き換える。** 世代の違う表はここで全部捨てる。
    fn keep(&self, character: i64, generation: &FaceGeneration, tokens: HashMap<String, u32>) {
        let mut tables = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        tables.retain(|_, table| table.generation == *generation);
        tables.insert(
            character,
            PartTable {
                generation: generation.clone(),
                tokens,
            },
        );
    }

    /// 覚えている部品の数（テスト専用）。
    #[cfg(test)]
    fn len(&self, character: i64) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&character)
            .map_or(0, |table| table.tokens.len())
    }
}

/// **未計測の部品を数える**（モジュールの doc の「並び」）。数えた部品だけを返す。
async fn count_missing(llm: &dyn LlmClient, missing: Vec<String>) -> HashMap<String, u32> {
    let mut counted = HashMap::with_capacity(missing.len());
    let mut rest = missing.into_iter();
    let Some(first) = rest.next() else {
        return counted;
    };
    match llm.count_text(&first).await {
        Ok(TokenCount::Counted(tokens)) => {
            counted.insert(first, tokens);
        }
        answer => {
            stop_counting(&answer);
            return counted;
        }
    }

    let mut answers = futures_util::stream::iter(rest)
        .map(|part| async move {
            let answer = llm.count_text(&part).await;
            (part, answer)
        })
        .buffer_unordered(PART_COUNT_CONCURRENCY);
    while let Some((part, answer)) = answers.next().await {
        match answer {
            Ok(TokenCount::Counted(tokens)) => {
                counted.insert(part, tokens);
            }
            answer => {
                // **ここで止める**（並べて飛んでいる残りは、`answers` を落として打ち切る）。
                stop_counting(&answer);
                break;
            }
        }
    }
    counted
}

/// 数えるのを止めた理由を `debug` に残す（**警告にしない** —— 止まっても、その部品がバイト数になるだけ）。
fn stop_counting(answer: &Result<TokenCount, crate::llm::LlmError>) {
    match answer {
        Ok(_) => tracing::debug!(
            "上流が部品を数えられないと答えました（残りの部品はバイト数で数えます）"
        ),
        Err(err) => tracing::debug!(
            error = %err,
            "部品を数える往復が失敗しました（残りの部品はバイト数で数えます）"
        ),
    }
}

/// 1ターンを組むのに要るもの（`super::chat` が集める）。
pub(super) struct Turn<'a> {
    /// **写しより先に読んだ**面の世代（`super::context::SharedUpstream::generation` の doc）。
    pub(super) generation: &'a FaceGeneration,
    /// **そのターンの写し**（部品・封・送信を全部これで行う）。
    pub(super) llm: &'a dyn LlmClient,
    pub(super) uncountable: &'a UncountableMemory,
    pub(super) parts: &'a PartMemory,
    /// 部品の記憶を分ける鍵（キャラクターの id）。
    pub(super) character: i64,
    /// **本文の予算**（入口で1回だけ読んだもの）。バイト数の計画はこれで組む。
    pub(super) budget: TokenBudget,
    pub(super) sharing: WindowSharing,
    /// 裏方の窓（差し引く量の出どころ）。
    pub(super) sub_context: SubContext,
    /// 性格の軸の本数の、**全キャラクターでの最大**（変容の応答の上限の出どころ / [`background_request`]）。
    pub(super) axes: usize,
    pub(super) streaming: bool,
    pub(super) temperature: Option<f32>,
    pub(super) max_tokens: u32,
    /// **本文の面に書いた思考の量**（入口で予算と一緒に1回だけ読んだもの / ADR-0059 §決定1 / 作業単位 A-18a）。
    ///
    /// **封をする本文と送る本文の両方に載る**（[`Self::request_for`] が1つの本文を作り、封もそれで数える ——
    /// チャットテンプレートの出力が変わるので、封は同じ値で作る / ADR-0052）。
    pub(super) reasoning_effort: Option<ReasoningEffort>,
}

/// 組み上がった1ターン。**`request` は封をしたものか、バイト数の計画そのもの。**
pub(super) struct Planned {
    pub(super) plan: BudgetPlan,
    pub(super) request: ChatRequest,
    pub(super) seal: TurnSeal,
}

impl Turn<'_> {
    /// 組み上がりから、送る本文（封をする本文）を作る。
    fn request_for(&self, built: &BuiltPrompt) -> ChatRequest {
        ChatRequest {
            messages: built.messages.clone(),
            temperature: self.temperature,
            max_tokens: Some(self.max_tokens),
            reasoning_effort: self.reasoning_effort,
        }
    }

    async fn seal(&self, request: &ChatRequest) -> Seal {
        self.uncountable
            .seal(self.generation, self.llm, request, self.streaming)
            .await
    }

    /// **バイト数の計画**（ADR-0052 §決定1 の (i)）。**元の予算で、何も数えていない表で組む。**
    /// 行は出さない（送ると決めたときに [`send`] が出す）。
    fn byte_plan(&self, input: &PromptInput<'_>) -> Result<BuiltPrompt, PromptError> {
        build_prompt_quiet(input, self.budget, &CountedParts::default())
    }

    /// **封で示せなかったので、バイト数の計画を封をせずに送る。**
    fn fall_back(
        &self,
        input: &PromptInput<'_>,
        mut seal: TurnSeal,
    ) -> Result<Planned, PromptError> {
        let built = self.byte_plan(input)?;
        let request = self.request_for(&built);
        seal.plan = SentPlan::Bytes;
        seal.sealed_tokens = None;
        Ok(send(built, request, seal))
    }
}

/// **送ると決めた組み上がり**。想起・削った履歴・落とした取り決めの行は**ここで1回だけ**出す
/// （作業単位 B-60d-1 のレビュー —— 組み直しの回に行が重なると、想起の行1本を1ターンと数える測定の道具が数え違える）。
fn send(built: BuiltPrompt, request: ChatRequest, seal: TurnSeal) -> Planned {
    built.report();
    Planned {
        plan: built.plan,
        request,
        seal,
    }
}

/// **1ターンを組む**（モジュールの doc の「1ターンの流れ」）。
///
/// `Err` は**バイト数の計画でも削れない枠が収まらない**ときだけ（今と同じ `BudgetExhausted`）。
/// 数えた計画が組めない・封が超えた・数えられないは、どれもバイト数の計画へ戻るだけで `Err` にしない（N3）。
pub(super) async fn plan_turn(
    turn: &Turn<'_>,
    input: &PromptInput<'_>,
) -> Result<Planned, PromptError> {
    // 0. **覚えた世代では聞かない**（B-60b の「数えられない」/ ADR-0052 §決定4。
    //    B-60d-2 の掛け金（`封 < 実` / §決定5）も同じ記憶に載る —— 理由は封の行の `seal=` に出る）。
    if let Some(skipped) = turn.uncountable.skipped(turn.generation) {
        let built = turn.byte_plan(input)?;
        let request = turn.request_for(&built);
        return Ok(send(built, request, TurnSeal::skipped(skipped)));
    }

    // 1. **部品を数える**（覚えていない部品だけ）。
    let started = Instant::now();
    let wanted = parts_to_count(input);
    let (mut tokens, missing) = turn.parts.known(turn.character, turn.generation, &wanted);
    let asked = missing.len();
    let fresh = count_missing(turn.llm, missing).await;
    let parts = PartsTally {
        counted: fresh.len(),
        uncounted: asked.saturating_sub(fresh.len()),
        elapsed: started.elapsed(),
    };
    tokens.extend(fresh);
    let anything_counted = !tokens.is_empty();
    let table: CountedParts = tokens
        .iter()
        .map(|(part, tokens)| (part.clone(), *tokens))
        .collect();
    turn.parts.keep(turn.character, turn.generation, tokens);

    let (counted_budget, shared_deduction) =
        counted_budget(turn.budget, turn.sharing, turn.sub_context, turn.axes);
    if anything_counted && counted_budget.is_none() {
        // 差し引くと窓が予算にならない（裏方の窓が本文の窓に比べて大きい）。**遅くなるだけの側**
        // （バイト数の計画）へ倒す。毎ターン起きうるので `debug`。
        tracing::debug!(
            context_tokens = turn.budget.context_tokens(),
            shared_deduction,
            "本文と裏方が同じ上流で、差し引くと予算にならないので、バイト数の計画で送ります"
        );
    }
    let mut summary = TurnSeal {
        first: Seal::remembered(),
        sealed_tokens: None,
        seal_elapsed: Duration::ZERO,
        plan: SentPlan::Bytes,
        rebuilds: 0,
        parts,
        // **数えた計画を組んだ回だけ載せる**（下で入れる。バイト数の計画は差し引かない）。
        shared_deduction: 0,
    };

    // 2. **数えた計画を組む。** 何も数えていない・予算にならない・組めないときはバイト数の計画に封をする。
    let counted = counted_budget
        .filter(|_| anything_counted)
        .and_then(|budget| {
            build_prompt_quiet(input, budget, &table)
                .ok()
                .map(|built| (budget, built))
        });
    if counted.is_some() {
        summary.shared_deduction = shared_deduction;
    }
    let Some((budget, built)) = counted else {
        // **バイト数の計画は上界で示せている**ので、封の数で送るかを決めない（封は「数えられない」の記憶と観測のため）。
        let built = turn.byte_plan(input)?;
        let request = turn.request_for(&built);
        let first = turn.seal(&request).await;
        summary.first = first;
        summary.seal_elapsed = first.elapsed();
        summary.sealed_tokens = first.sealed_tokens();
        return Ok(send(built, request, summary));
    };

    // 3. **封をする。取り分以下なら送る。**
    // **比べるのは数えた計画の予算の取り分**（同じ上流なら差し引いた後のもの）。
    let allowance = budget.prompt_allowance();
    let request = turn.request_for(&built);
    let first = turn.seal(&request).await;
    summary.first = first;
    summary.seal_elapsed = first.elapsed();
    let Outcome::Counted(sealed) = first.outcome() else {
        // 数えられない・往復の失敗 → 数えた計画は示せない（ADR-0052 §決定4）。
        return turn.fall_back(input, summary);
    };
    if sealed <= allowance {
        summary.plan = SentPlan::Counted;
        summary.sealed_tokens = Some(sealed);
        return Ok(send(built, request, summary));
    }

    // 4. **超えた分だけ狭めて、1回だけ組み直す。**
    let planned = built.plan.prompt_tokens();
    let Some(rebuilt) = narrowed(budget, planned, sealed)
        .and_then(|narrow| build_prompt_quiet(input, narrow, &table).ok())
    else {
        tracing::info!(
            sealed_tokens = sealed,
            allowance,
            planned_tokens = planned,
            "封が取り分を超え、狭めた予算では組み直せないので、バイト数の計画で送ります"
        );
        return turn.fall_back(input, summary);
    };
    let request = turn.request_for(&rebuilt);
    let second = turn.seal(&request).await;
    summary.rebuilds = 1;
    summary.seal_elapsed = summary.seal_elapsed.saturating_add(second.elapsed());
    match second.outcome() {
        Outcome::Counted(resealed) if resealed <= allowance => {
            summary.plan = SentPlan::Counted;
            summary.sealed_tokens = Some(resealed);
            Ok(send(rebuilt, request, summary))
        }
        // 5. **それでも示せなければ、バイト数の計画で送る**（`BudgetExhausted` にしない / N3）。
        outcome => {
            tracing::info!(
                first_sealed_tokens = sealed,
                second = ?outcome,
                allowance,
                "組み直しても封で取り分に収まることを示せないので、バイト数の計画で送ります"
            );
            turn.fall_back(input, summary)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::Utc;

    use super::*;
    use crate::domain::fixture_scale;
    use crate::domain::{Character, CharacterId, DisplayName, Handle, Persona};
    use crate::llm::{ChatStream, Completion, LlmError, LlmFuture};
    use crate::prompt::build_prompt;

    fn budget(context: u32) -> TokenBudget {
        TokenBudget::new(context, 1_024).expect("予算になる値であること")
    }

    fn generation(epoch: u64) -> FaceGeneration {
        FaceGeneration::for_tests(epoch, None)
    }

    fn sub(tokens: u32) -> SubContext {
        SubContext::new(tokens).expect("裏方の窓になる値であること")
    }

    /// **別の上流なら差し引かない。同じ上流なら裏方の1要求分を引く**（値は関数の外で押さえる）。
    #[test]
    fn the_counted_budget_leaves_room_for_one_background_request_only_on_a_shared_upstream() {
        assert_eq!(
            counted_budget(budget(16_384), WindowSharing::Separate, sub(16_384), 5),
            (Some(budget(16_384)), 0)
        );
        // 16384 の半分（依頼の上限）＋ 320（軸が無いので抽出の応答の上限が最大）。
        assert_eq!(
            counted_budget(budget(16_384), WindowSharing::SameUpstream, sub(16_384), 0),
            (Some(budget(16_384 - 8_192 - 320)), 8_512)
        );
        // 裏方の窓が違えば、差し引く量も違う（決め打ちしていない）。
        assert_eq!(
            counted_budget(budget(20_000), WindowSharing::SameUpstream, sub(4_096), 0),
            (Some(budget(20_000 - 2_048 - 320)), 2_368)
        );
        // 差し引くと予算にならない（窓が小さい）→ 数えた計画を組まない。
        assert_eq!(
            counted_budget(budget(8_192), WindowSharing::SameUpstream, sub(16_384), 0),
            (None, 8_512)
        );
        assert_eq!(
            counted_budget(budget(9_000), WindowSharing::SameUpstream, sub(16_384), 0),
            (None, 8_512),
            "応答の余白以下しか残らない窓も予算にしないこと"
        );
    }

    /// **裏方の1要求分は、依頼の上限 ＋ 各依頼の応答の上限のうち最大**（作業単位 B-60d-1 のレビュー 要修正2）。
    ///
    /// 期待値はリテラルで押さえる: 抽出は 320、変容は全軸の形で 1軸 86（5本で 430・4本で 344・3本で 258）、
    /// 軸の候補は 89、軸の試用は 86、約束・決まりの提案は 320（作業単位 A-12c-1 —— 抽出と同じなので、
    /// 軸が3本以下の行の 320 が「提案を足しても差し引く量が変わらない」ことも押さえる）。
    /// **軸が4本以上なら変容が抽出を上回る。**
    #[test]
    fn the_background_request_is_set_by_the_largest_response_limit() {
        for (axes, response) in [(0, 320), (3, 320), (4, 344), (5, 430), (10, 860)] {
            assert_eq!(
                background_request(sub(16_384), axes),
                8_192 + response,
                "軸 {axes} 本"
            );
        }
        // 変容の上限は窓の残りで頭打ち（依頼の上限と足して窓に収まる）。
        assert_eq!(background_request(sub(4_096), 1_000), 2_048 + 2_048);
    }

    /// **ループバック同士でポートが同じなら同じ上流。読めない綴り・ループバック以外の別名は別**
    /// （作業単位 B-60d-1 のレビューの提案）。
    #[test]
    fn loopback_endpoints_on_the_same_port_are_one_upstream() {
        let same = [
            (
                "http://127.0.0.1:8081/v1/chat/completions",
                "http://127.0.0.1:8081/v1/chat/completions",
            ),
            (
                "http://localhost:8081/v1/chat/completions",
                "http://127.0.0.1:8081/v1/chat/completions",
            ),
            (
                "http://[::1]:8081/v1/chat/completions",
                "http://LOCALHOST:8081/v1/chat/completions",
            ),
            (
                "http://127.0.0.2:8081/v1/chat/completions",
                "http://127.0.0.1:8081/proxy/v1/chat/completions",
            ),
            // ポートを書かなければ方式の既定。userinfo は見ない。
            (
                "http://localhost/v1/chat/completions",
                "http://user:secret@127.0.0.1:80/v1/chat/completions",
            ),
            // 綴りが同じなら、読めない綴りでも同じ。
            ("not a url", "not a url"),
        ];
        for (main, sub) in same {
            assert_eq!(
                WindowSharing::of_endpoints(main, sub),
                WindowSharing::SameUpstream,
                "{main} / {sub}"
            );
        }
        let separate = [
            (
                "http://127.0.0.1:8081/v1/chat/completions",
                "http://127.0.0.1:8082/v1/chat/completions",
            ),
            (
                "http://localhost:8081/v1/chat/completions",
                "http://192.0.2.10:8081/v1/chat/completions",
            ),
            (
                "http://example.com:8081/v1/chat/completions",
                "http://example.net:8081/v1/chat/completions",
            ),
            ("not a url", "http://127.0.0.1:8081/v1/chat/completions"),
            (
                "http://localhost/v1/chat/completions",
                "https://localhost/v1/chat/completions",
            ),
        ];
        for (main, sub) in separate {
            assert_eq!(
                WindowSharing::of_endpoints(main, sub),
                WindowSharing::Separate,
                "{main} / {sub}"
            );
        }
    }

    /// **組み直しの取り分 = 計画の和 − 超えた分**（モジュールの doc）。
    #[test]
    fn the_narrowed_budget_takes_the_overflow_off_the_plan() {
        // 取り分 3072、計画 3000、封 3100 → 超えた分 28 → 取り分 2972。
        assert_eq!(
            narrowed(budget(4_096), 3_000, 3_100),
            Some(budget(2_972 + 1_024))
        );
        // 計画が取り分を超えていても取り分で頭打ち（2回目の封がまた超えないように）。
        assert_eq!(
            narrowed(budget(4_096), 5_000, 3_100),
            Some(budget(3_072 - 28 + 1_024))
        );
        // 封が取り分以下なら組み直さない（境目を含む）。
        assert_eq!(narrowed(budget(4_096), 3_000, 3_072), None);
        assert_eq!(narrowed(budget(4_096), 3_000, 1), None);
        // 超えた分が計画の和以上なら組み直せない。
        assert_eq!(narrowed(budget(4_096), 100, 3_172), None);
        assert_eq!(narrowed(budget(4_096), 100, 3_173), None);
        assert_eq!(narrowed(budget(4_096), 101, 3_172), Some(budget(1 + 1_024)));
    }

    fn parts(texts: &[&str]) -> BTreeSet<String> {
        texts.iter().map(|text| (*text).to_owned()).collect()
    }

    /// **覚えるのは直近のターンの部品だけ。世代が違えば使わず、書くときに捨てる。**
    #[test]
    fn the_part_memory_keeps_the_latest_turn_per_character_and_generation() {
        let memory = PartMemory::default();
        memory.keep(
            1,
            &generation(0),
            HashMap::from([("a".to_owned(), 1), ("b".to_owned(), 2)]),
        );
        memory.keep(2, &generation(0), HashMap::from([("c".to_owned(), 3)]));

        let (known, missing) = memory.known(1, &generation(0), &parts(&["a", "c", "d"]));
        assert_eq!(known, HashMap::from([("a".to_owned(), 1)]));
        assert_eq!(
            missing,
            vec!["c".to_owned(), "d".to_owned()],
            "他のキャラクターの表は引かない"
        );

        // 置き換え: 今回の部品だけが残る（"b" は消える）。
        memory.keep(1, &generation(0), HashMap::from([("a".to_owned(), 1)]));
        assert_eq!(memory.len(1), 1);
        assert_eq!(memory.len(2), 1, "同じ世代の他のキャラクターの表は残ること");

        // 別の世代では引かない。
        let (known, missing) = memory.known(1, &generation(1), &parts(&["a"]));
        assert!(known.is_empty());
        assert_eq!(missing, vec!["a".to_owned()]);
        let (known, _) = memory.known(
            1,
            &FaceGeneration::for_tests(0, Some("another.gguf")),
            &parts(&["a"]),
        );
        assert!(known.is_empty(), "答えた名前が違えば別の世代");

        // 新しい世代で書くと、古い世代の表は全部捨てる。
        memory.keep(1, &generation(1), HashMap::from([("z".to_owned(), 9)]));
        assert_eq!(memory.len(2), 0, "古い世代の表を残さないこと");
        assert_eq!(memory.len(1), 1);
    }

    /// 部品を数える上流（答えは決めておく）。**聞かれた部品を数える。**
    struct Parts {
        answer: fn(&str) -> Result<TokenCount, LlmError>,
        asked: AtomicUsize,
        seal: Result<TokenCount, ()>,
        sealed: Mutex<Vec<ChatRequest>>,
    }

    impl Parts {
        fn new(answer: fn(&str) -> Result<TokenCount, LlmError>) -> Self {
            Self {
                answer,
                asked: AtomicUsize::new(0),
                seal: Ok(TokenCount::Uncountable),
                sealed: Mutex::new(Vec::new()),
            }
        }
    }

    impl LlmClient for Parts {
        fn pinned(&self) -> Option<Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            Box::pin(async { Err(LlmError::MissingContent) })
        }

        fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
            Box::pin(async { Err(LlmError::MissingContent) })
        }

        fn count_request<'a>(
            &'a self,
            request: &'a ChatRequest,
            _streaming: bool,
        ) -> LlmFuture<'a, TokenCount> {
            self.sealed
                .lock()
                .expect("ロックを取れること")
                .push(request.clone());
            let answer = self.seal.map_err(|()| LlmError::MissingContent);
            Box::pin(async move { answer })
        }

        /// **答える前に1回だけ譲る**（往復を模す）。すぐ答える future だと、並べた残りが
        /// 一度も poll されないまま止まるので、「最初の1つを単独で聞く」を外しても聞いた数が増えない
        /// （変異 m10 で実測 —— 譲らない形ではこのモジュールの見張り番が緑のままだった）。
        fn count_text<'a>(&'a self, text: &'a str) -> LlmFuture<'a, TokenCount> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let answer = (self.answer)(text);
            Box::pin(async move {
                tokio::task::yield_now().await;
                answer
            })
        }
    }

    fn many(count: usize) -> Vec<String> {
        (0..count).map(|index| format!("部品{index:02}")).collect()
    }

    /// **全部数えられれば全部を返す。** 数は上流の答えそのもの。
    #[tokio::test]
    async fn every_part_is_counted_when_the_upstream_can_count() {
        let llm = Parts::new(|text| {
            Ok(TokenCount::Counted(
                u32::try_from(text.len()).unwrap_or(0) + 1,
            ))
        });
        let counted = count_missing(&llm, many(9)).await;
        assert_eq!(counted.len(), 9);
        assert_eq!(counted.get("部品03"), Some(&("部品03".len() as u32 + 1)));
        assert_eq!(llm.asked.load(Ordering::SeqCst), 9);
    }

    /// **最初の1つで数えられないと分かれば、残りへは聞きに行かない**（LM Studio に部品の数だけ往復を投げない）。
    #[tokio::test]
    async fn an_upstream_that_cannot_count_is_asked_for_one_part_only() {
        for answer in [
            (|_: &str| Ok(TokenCount::Uncountable)) as fn(&str) -> Result<TokenCount, LlmError>,
            |_| Err(LlmError::MissingContent),
        ] {
            let llm = Parts::new(answer);
            let counted = count_missing(&llm, many(9)).await;
            assert!(counted.is_empty());
            assert_eq!(llm.asked.load(Ordering::SeqCst), 1, "最初の1つだけ聞くこと");
        }
        // 空なら聞かない。
        let llm = Parts::new(|_| Ok(TokenCount::Counted(1)));
        assert!(count_missing(&llm, Vec::new()).await.is_empty());
        assert_eq!(llm.asked.load(Ordering::SeqCst), 0);
    }

    /// **途中で数えられない答えが返ったら止める**（残りはバイト数のまま）。止まる前に数えたものは使う。
    #[tokio::test]
    async fn counting_stops_at_the_first_part_that_could_not_be_counted() {
        let llm = Parts::new(|text| {
            if text == "部品05" {
                Err(LlmError::MissingContent)
            } else {
                Ok(TokenCount::Counted(2))
            }
        });
        let counted = count_missing(&llm, many(40)).await;
        assert!(!counted.contains_key("部品05"));
        assert!(counted.contains_key("部品00"), "最初の1つは数えていること");
        assert!(
            counted.len() < 39,
            "止まらずに全部を数えに行かないこと: {}",
            counted.len()
        );
        assert!(
            llm.asked.load(Ordering::SeqCst) <= 6 + PART_COUNT_CONCURRENCY,
            "止めた後に残りを聞きに行かないこと: {}",
            llm.asked.load(Ordering::SeqCst)
        );
    }

    fn character() -> Character {
        Character {
            id: CharacterId::from_row(1),
            handle: Handle::parse("aoi").expect("正当な名前であること"),
            display_name: DisplayName::parse("葵").expect("正当な表示名であること"),
            base_persona: Persona::parse("落ち着いた口調で話す。")
                .expect("正当なペルソナであること"),
            output_format_instruction: None,
            stat_scale: fixture_scale(),
            stat_drift_mode: None,
            axis_discovery_slots: crate::domain::AxisDiscoverySlots::NONE,
            created_at: Utc::now(),
            archived_at: None,
        }
    }

    fn input(character: &Character) -> PromptInput<'_> {
        PromptInput {
            character,
            stats: &[],
            attributes: &[],
            agreements: &[],
            memories: &[],
            history: &[],
            history_turns_before: None,
            user_message: "こんにちは",
        }
    }

    fn turn<'a>(
        llm: &'a dyn LlmClient,
        uncountable: &'a UncountableMemory,
        parts: &'a PartMemory,
        generation: &'a FaceGeneration,
        sharing: WindowSharing,
    ) -> Turn<'a> {
        Turn {
            generation,
            llm,
            uncountable,
            parts,
            character: 1,
            budget: budget(8_192),
            sharing,
            sub_context: SubContext::FALLBACK,
            axes: 0,
            streaming: false,
            temperature: Some(0.5),
            max_tokens: 700,
            reasoning_effort: None,
        }
    }

    /// **何も数えられない回は、元の予算のバイト数の計画を送る** —— 同じ上流でも差し引かない
    /// （§決定4「今と同じ」。差し引くとこの予算では組めない）。
    #[tokio::test]
    async fn nothing_counted_sends_the_byte_plan_of_the_original_budget_even_on_a_shared_upstream()
    {
        let character = character();
        let input = input(&character);
        let llm = Parts::new(|_| Ok(TokenCount::Uncountable));
        let (uncountable, parts) = (UncountableMemory::default(), PartMemory::default());
        let generation = generation(0);
        let planned = plan_turn(
            &turn(
                &llm,
                &uncountable,
                &parts,
                &generation,
                WindowSharing::SameUpstream,
            ),
            &input,
        )
        .await
        .expect("組めること");

        let expected = build_prompt(&input, budget(8_192), &CountedParts::default())
            .expect("バイト数の計画が組めること");
        assert_eq!(planned.request.messages, expected.messages);
        assert_eq!(planned.plan, expected.plan);
        assert_eq!(planned.request.temperature, Some(0.5));
        assert_eq!(planned.request.max_tokens, Some(700));
        assert_eq!(planned.seal.plan, SentPlan::Bytes);
        assert_eq!(planned.seal.first.outcome(), Outcome::Uncountable);
        assert_eq!(
            planned.seal.shared_deduction, 0,
            "数えた計画を組まなかった回は差し引いたと言わないこと"
        );
        assert_eq!(
            llm.sealed.lock().expect("ロックを取れること").as_slice(),
            std::slice::from_ref(&planned.request),
            "送る本文に封をしたこと（「数えられない」を覚えるため）"
        );
    }

    /// **本文の面の思考の量は、送る本文と封をする本文の両方に載る**（ADR-0059 §決定1 / 作業単位 A-18a）。
    ///
    /// 封は送る本文そのもので数える（チャットテンプレートの出力が思考の有無で変わる / ADR-0052）。
    /// **対照**: 既定の [`turn`] は `None` のまま（上の腕の `planned.request` に `None` が載っている）。
    #[tokio::test]
    async fn the_reasoning_effort_rides_on_both_the_sent_and_the_sealed_request() {
        use crate::llm::ReasoningEffort;

        let character = character();
        let input = input(&character);
        let llm = Parts::new(|_| Ok(TokenCount::Uncountable));
        let (uncountable, parts) = (UncountableMemory::default(), PartMemory::default());
        let generation = generation(0);
        let thinking = Turn {
            reasoning_effort: Some(ReasoningEffort::Low),
            ..turn(
                &llm,
                &uncountable,
                &parts,
                &generation,
                WindowSharing::SameUpstream,
            )
        };
        let planned = plan_turn(&thinking, &input).await.expect("組めること");

        assert_eq!(
            planned.request.reasoning_effort,
            Some(ReasoningEffort::Low),
            "送る本文が本文の面の思考の量を持たない"
        );
        assert_eq!(
            llm.sealed.lock().expect("ロックを取れること").as_slice(),
            std::slice::from_ref(&planned.request),
            "封をした本文が送る本文と違う（思考の量を落とした？）"
        );
    }

    /// **覚えた世代では、部品も封も聞かない。**
    #[tokio::test]
    async fn a_remembered_generation_asks_nothing() {
        let character = character();
        let input = input(&character);
        let llm = Parts::new(|_| Ok(TokenCount::Counted(1)));
        let (uncountable, parts) = (UncountableMemory::default(), PartMemory::default());
        let generation = generation(0);
        let first = uncountable
            .seal(
                &generation,
                &Parts::new(|_| Ok(TokenCount::Uncountable)),
                &ChatRequest::new(Vec::new()),
                false,
            )
            .await;
        uncountable.accepted(generation.clone(), &generation, first);

        let planned = plan_turn(
            &turn(
                &llm,
                &uncountable,
                &parts,
                &generation,
                WindowSharing::Separate,
            ),
            &input,
        )
        .await
        .expect("組めること");
        assert_eq!(planned.seal, TurnSeal::remembered());
        assert_eq!(llm.asked.load(Ordering::SeqCst), 0);
        assert!(llm.sealed.lock().expect("ロックを取れること").is_empty());
    }

    /// **掛け金が掛かった世代も、部品も封も聞かずにバイト数の計画を送る**（作業単位 B-60d-2）。
    /// 封の行の `seal=` は `latched`（「数えられない」と区別できる）。**別の世代では数える。**
    #[tokio::test]
    async fn a_latched_generation_asks_nothing_and_sends_the_byte_plan() {
        let character = character();
        let input = input(&character);
        let mut llm = Parts::new(|_| Ok(TokenCount::Counted(3)));
        llm.seal = Ok(TokenCount::Counted(40));
        let (uncountable, parts) = (UncountableMemory::default(), PartMemory::default());
        let latched = generation(0);
        uncountable.latch(latched.clone(), &latched);

        let planned = plan_turn(
            &turn(
                &llm,
                &uncountable,
                &parts,
                &latched,
                WindowSharing::Separate,
            ),
            &input,
        )
        .await
        .expect("組めること");
        let expected = build_prompt(&input, budget(8_192), &CountedParts::default())
            .expect("バイト数の計画が組めること");
        assert_eq!(planned.request.messages, expected.messages);
        assert_eq!(planned.seal.plan, SentPlan::Bytes);
        assert_eq!(planned.seal.first.outcome(), Outcome::Latched);
        assert_eq!(planned.seal.first.label(), "latched");
        assert_eq!(llm.asked.load(Ordering::SeqCst), 0, "部品を聞かないこと");
        assert!(
            llm.sealed.lock().expect("ロックを取れること").is_empty(),
            "封をしないこと"
        );

        let other = generation(1);
        let planned = plan_turn(
            &turn(&llm, &uncountable, &parts, &other, WindowSharing::Separate),
            &input,
        )
        .await
        .expect("組めること");
        assert_eq!(
            planned.seal.plan,
            SentPlan::Counted,
            "別の世代では数えること"
        );
    }

    /// **数えた計画に封をして、取り分以下なら送る。2ターン目は覚えた部品を聞き直さない。**
    #[tokio::test]
    async fn a_counted_plan_within_the_allowance_is_sent_and_its_parts_are_remembered() {
        let character = character();
        let input = input(&character);
        let mut llm = Parts::new(|_| Ok(TokenCount::Counted(3)));
        llm.seal = Ok(TokenCount::Counted(40));
        let (uncountable, parts) = (UncountableMemory::default(), PartMemory::default());
        let generation = generation(0);
        let turn = turn(
            &llm,
            &uncountable,
            &parts,
            &generation,
            WindowSharing::Separate,
        );

        let planned = plan_turn(&turn, &input).await.expect("組めること");
        let wanted = parts_to_count(&input).len();
        assert_eq!(planned.seal.plan, SentPlan::Counted);
        assert_eq!(planned.seal.sealed_tokens, Some(40));
        assert_eq!(planned.seal.parts.counted, wanted);
        assert_eq!(planned.seal.parts.uncounted, 0);
        assert_eq!(planned.seal.rebuilds, 0);
        let table: CountedParts = parts_to_count(&input)
            .into_iter()
            .map(|part| (part, 3))
            .collect();
        assert_eq!(
            planned.plan,
            build_prompt(&input, budget(8_192), &table)
                .expect("数えた計画が組めること")
                .plan,
            "数えた値で組んだ計画であること"
        );

        let again = plan_turn(&turn, &input).await.expect("組めること");
        assert_eq!(again.seal.parts.counted, 0, "覚えた部品を聞き直さないこと");
        assert_eq!(llm.asked.load(Ordering::SeqCst), wanted);
        assert_eq!(
            again.request, planned.request,
            "同じ入力から同じ本文になること"
        );
    }

    /// **差し引くと予算にならないなら、部品を数えられてもバイト数の計画を送る**（作業単位 B-60d-1 のレビュー P2）。
    ///
    /// 同じ上流・既定の予算（8192）・落とし所の裏方の窓（依頼 8192 ＋ 応答 320）。差し引くと予算にならない。
    /// **元の予算で数えた計画を組む形**（`counted_budget.or(Some(turn.budget))`）を止める。
    #[tokio::test]
    async fn a_shared_window_too_small_to_deduct_sends_the_byte_plan_even_when_parts_count() {
        let character = character();
        let input = input(&character);
        let mut llm = Parts::new(|_| Ok(TokenCount::Counted(3)));
        llm.seal = Ok(TokenCount::Counted(40));
        let (uncountable, parts) = (UncountableMemory::default(), PartMemory::default());
        let generation = generation(0);
        let planned = plan_turn(
            &turn(
                &llm,
                &uncountable,
                &parts,
                &generation,
                WindowSharing::SameUpstream,
            ),
            &input,
        )
        .await
        .expect("組めること");

        let expected = build_prompt(&input, budget(8_192), &CountedParts::default())
            .expect("バイト数の計画が組めること");
        assert!(
            planned.seal.parts.counted > 0,
            "前提: 部品は数えられていること"
        );
        assert_eq!(planned.plan, expected.plan, "バイト数の計画であること");
        assert_eq!(planned.request.messages, expected.messages);
        assert_eq!(planned.seal.plan, SentPlan::Bytes);
        assert_eq!(planned.seal.shared_deduction, 0);
        // 対照: 別の上流なら同じ入力で数えた計画になる（この入力で2つの計画が見分けられる）。
        let parts = PartMemory::default();
        let separate = plan_turn(
            &turn(
                &llm,
                &uncountable,
                &parts,
                &generation,
                WindowSharing::Separate,
            ),
            &input,
        )
        .await
        .expect("組めること");
        assert_eq!(separate.seal.plan, SentPlan::Counted);
        assert_ne!(
            separate.plan, expected.plan,
            "対照: 数えた計画は計画が違うこと"
        );
    }

    /// **封が数えられない・失敗したら、数えた計画を送らない**（バイト数の計画で送る）。
    #[tokio::test]
    async fn a_counted_plan_whose_seal_is_not_a_count_falls_back_to_the_byte_plan() {
        let character = character();
        let input = input(&character);
        for seal in [Ok(TokenCount::Uncountable), Err(())] {
            let mut llm = Parts::new(|_| Ok(TokenCount::Counted(3)));
            llm.seal = seal;
            let (uncountable, parts) = (UncountableMemory::default(), PartMemory::default());
            let generation = generation(0);
            let planned = plan_turn(
                &turn(
                    &llm,
                    &uncountable,
                    &parts,
                    &generation,
                    WindowSharing::Separate,
                ),
                &input,
            )
            .await
            .expect("組めること");
            let expected = build_prompt(&input, budget(8_192), &CountedParts::default())
                .expect("バイト数の計画が組めること");
            assert_eq!(planned.request.messages, expected.messages, "{seal:?}");
            assert_eq!(planned.seal.plan, SentPlan::Bytes);
            assert_eq!(planned.seal.sealed_tokens, None);
        }
    }
}
