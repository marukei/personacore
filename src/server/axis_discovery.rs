//! **記憶が溜まったら、裏方が軸の候補を1つ出す**
//! （[ADR-0046](../../docs/adr/0046-drift-proposes-new-axes.md) §決定2 / 作業単位 A-9b / Issue origin#42）。
//!
//! 純粋関数（`domain::axis_discovery` / `prompt::axis_discovery` / `parse::axis_candidate`）に
//! **いつ・どう走らせるか**を与える層。ここだけが I/O を持つ。
//!
//! ```text
//! server::extract::run（抽出タスクの中。**変容の直後に直列**。走らせ方が off なら呼ばれない）
//!   → run_axis_discovery
//!        （枠の数 m = 0 なら、何も読まずに終わる）
//!        Store::axis_candidates      … 既存の候補（名前と状態）。空いた枠を数える
//!        Store::axis_discovery_mark  … 前回の試みで数え終えた記憶
//!        Store::recall_memories      … **想起の入口**（ADR-0003）。目印より後の自分の記憶を数える
//!        domain::axis_discovery_due  … 空いた枠があり、N 件増えたか（純粋）
//!        Store::stat_allocs          … 固定の軸の名前
//!        prompt::axis_discovery      … 依頼を組む（純粋。予算に入る記憶と、入るだけの使えない名前）
//!        LlmClient::complete         … **サブモデル**。トランザクションの外
//!        parse::axis_candidate       … 名前を1つ取り出す（純粋。panic しない）
//!        domain::axis_name_clash     … 既存の名前と**完全一致**なら捨てる（正規化しない）
//!        Store::record_axis_candidate（Propose）… 採ったら試用として追記（目印も同じトランザクション）
//!        Store::record_axis_discovery_miss      … 採らなかった回も「試みた」として記録
//!        1イベント記録               … 結末・何と一致したか／なぜ崩れていたか・件数
//! ```
//!
//! # 「試みた」を必ず残す（Issue origin#42 の申し送り1）
//!
//! 上流を呼んだ回は、**結末が何であれ試みの表に1行残す**（採った・完全一致で捨てた・
//! 崩れていた・打ち切られた・上流を呼べなかった・書けなかった）。
//! 残さないと目印が進まず、**次のターンもまた尋ねる**（採るまで毎ターン）。
//! 上流を呼ぶ前に止まった回（枠が無い・記憶が足りない・読めない・長すぎる）は残さない
//! ——尋ねていないので、次に条件が揃った回に尋ねればよい。
//!
//! # 本文には一切影響しない / トランザクションを跨がない
//!
//! 戻り値を持たず、呼び出し側は結果を見ない。読みはどれも1文で完結し、
//! **`LlmClient::complete` の `await` はどのトランザクションの中にも無い**（CLAUDE.md 絶対制約）。
//! 書き手（`record_axis_candidate`）は応答を読み切って `StatAxis` に通した**後**に呼ぶ。
//!
//! # LLM 出力の扱い
//!
//! 応答は**外部からの信頼できない入力**。ここから先へ渡る経路は `parse::axis_candidate`
//! （`StatAxis` を通す）→ 照合 → `record_axis_candidate`（プレースホルダ束縛）だけで、
//! **SQL・シェル・パスには渡らない。**
//! **ログにも名前と生出力を載せない**（A-9a の方針。手掛かりは結末の集計キーと候補の id）。
//!
//! > **⚠ 代償**: 崩れた応答の中身はどこにも残らない（変容は生出力をログに載せている）。
//! > 崩れ方の内訳は `axis_discovery_detail`（空・2行以上・手本の写し・「なし」系・文字を含まない・型を通らない）
//! > までしか分からない。

use crate::domain::{
    AxisCandidateRecord, AxisCandidateState, AxisDiscoveryDue, AxisDiscoveryMemories,
    AxisDiscoveryMiss, AxisDiscoveryOutcome, AxisDiscoverySlots, StatAxis, axis_discovery_due,
    axis_name_clash, memories_since,
};
use crate::llm::{ChatRequest, LlmClient, SUB_LLM_TEMPERATURE};
use crate::parse::axis_candidate::{AxisCandidateReading, parse_axis_candidate};
use crate::prompt::axis_discovery::{
    AxisDiscoveryInput, build_axis_discovery_prompt, request_tokens,
};
use crate::store::{AxisCandidate, Store, StoreError};

use super::extract::{
    SubContext, TurnJob, character_vanished, record_vanished, response_completeness,
};

/// 候補を1回尋ねたことの記録に使う文言。**「尋ねた」側。**
pub(in crate::server) const RAN_MESSAGE: &str = "軸の候補を尋ねました";

/// 尋ねなかったことの記録に使う文言。**「尋ねなかった」側**（抽出・変容と同じ分け方）。
pub(in crate::server) const SKIPPED_MESSAGE: &str = "軸の候補を尋ねませんでした";

/// 詳細の欄に出す「該当なし」。
const NO_DETAIL: &str = "-";

/// 試みを記録できた回の印。
const ATTEMPT_RECORDED: &str = "recorded";

/// **試みを記録できなかった回の印。故障**（次のターンにもう一度尋ねることになる）。
const ATTEMPT_FAILED: &str = "failed_to_record";

/// 捨てた書き込みの種類（`vanished_write` 欄 / ADR-0049 §決定3）。**軸の候補。**
const VANISHED_AXIS_CANDIDATE: &str = "axis_candidate";

/// 同上。**軸の候補を尋ねた試みの記録。**
const VANISHED_DISCOVERY_ATTEMPT: &str = "axis_discovery_attempt";

/// 採った名前を候補として書けなかった理由（`axis_discovery_detail`）：枠が先に埋まった。
const NOT_RECORDED_SLOTS_FULL: &str = "slots_full";

/// 同：同じ名前の候補が先に入った（照合の後に入った / 表の UNIQUE が止めた）。
const NOT_RECORDED_TAKEN: &str = "name_taken";

/// 同：DB の失敗。
const NOT_RECORDED_STORE_FAILED: &str = "store_failed";

/// **応答に許すトークン数**（名前1行ぶん）。
///
/// ```text
/// 1行 = 「- 」 ＋ 名前 ＋ 囲み（「」で 6 バイト） ＋ 改行
///     = 2 ＋ StatAxis::MAX_CHARS × UTF-8 の最大バイト長 ＋ 6 ＋ 1
/// ```
///
/// 見積もりは UTF-8 のバイト数（`prompt::estimate` の上界）。**それより長い応答からは
/// 採れる情報が増えない** —— 2行以上は丸ごと捨てるので（`parse::axis_candidate`）、
/// 長く受け取っても捨てる量が増えるだけ。上限を送るのは、形式を無視して喋り続けるモデルが
/// 本文生成の GPU を握り続けるのを防ぐため（変容の `drift_response_tokens` と同じ理由）。
/// 切れた回は `truncated` として丸ごと捨てて数える。
const AXIS_CANDIDATE_LINE_TOKENS: u32 = 2 + (StatAxis::MAX_CHARS as u32) * 4 + 6 + 1;

/// 応答の上限。**窓の残りで頭打ち**（依頼と足して実コンテキストに収める / 変容と同じ規則）。
///
/// **本文の数えた計画も読む**（裏方の1要求分の差し引き / `super::counted_plan::background_request`）。
pub(in crate::server) const fn response_tokens(sub_context: SubContext) -> u32 {
    let remainder = sub_context
        .tokens()
        .saturating_sub(sub_context.prompt_limit());
    if AXIS_CANDIDATE_LINE_TOKENS < remainder {
        AXIS_CANDIDATE_LINE_TOKENS
    } else {
        remainder
    }
}

/// 尋ねなかった理由。**集計キーは値ごとの定数**（入力を反射しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkipReason {
    /// 候補を読めなかった（空いた枠を数えられない・照合の相手が分からない）。
    CandidatesUnreadable,
    /// 試みの目印を読めなかった。
    AttemptsUnreadable,
    /// 記憶を読めなかった。
    MemoriesUnreadable,
    /// 固定の軸を読めなかった（照合の相手が分からない）。
    StatsUnreadable,
    /// 空いた枠が無い。
    NoFreeSlot,
    /// 記憶がまだ `N` 件増えていない。
    NotEnoughMemories,
    /// 記憶を1件載せただけで依頼が予算を超える（名前の一覧が長すぎる）。
    TooLong,
}

impl SkipReason {
    /// 集計キー。
    const fn label(self) -> &'static str {
        match self {
            Self::CandidatesUnreadable => "candidates_unreadable",
            Self::AttemptsUnreadable => "attempts_unreadable",
            Self::MemoriesUnreadable => "memories_unreadable",
            Self::StatsUnreadable => "stats_unreadable",
            Self::NoFreeSlot => "no_free_slot",
            Self::NotEnoughMemories => "not_enough_memories",
            Self::TooLong => "request_too_long",
        }
    }
}

/// **候補づくりの本体。** 抽出タスクの中で、変容の直後に走る（`super::extract::run`）。
///
/// **呼ばれるのは効く走らせ方が off でないときだけ**（判断の理由は `super::extract::run` の doc）。
/// **枠の数 `m` = 0 なら何も読まずに終わる**（Issue origin#42「1回も依頼しない」）。
pub(in crate::server) async fn run_axis_discovery(
    store: &Store,
    llm: &dyn LlmClient,
    job: &TurnJob,
    every: AxisDiscoveryMemories,
) {
    let slots = job.axis_discovery_slots;
    // **m = 0 は DB にも触らない。** 既存のキャラクターは全部ここで終わる（既定 0）。
    if slots == AxisDiscoverySlots::NONE {
        return;
    }

    // **既存の候補**（名前と状態）。1件でも読めなければ止まる（`Store::axis_candidates` の doc）。
    let candidates = match store.axis_candidates(job.character_id, u32::MAX).await {
        Ok(candidates) => candidates,
        Err(err) => {
            record_skip(SkipReason::CandidatesUnreadable, job, Some(&err));
            return;
        }
    };
    let states: Vec<AxisCandidateState> = candidates.iter().map(AxisCandidate::state).collect();
    // **枠が無ければ、記憶を読む前に終わる**（空いていないキャラクターの毎ターンを安くする）。
    if slots.free(states.iter().copied()) == 0 {
        record_skip(SkipReason::NoFreeSlot, job, None);
        return;
    }

    let mark = match store.axis_discovery_mark(job.character_id).await {
        Ok(mark) => mark,
        Err(err) => {
            record_skip(SkipReason::AttemptsUnreadable, job, Some(&err));
            return;
        }
    };
    // **想起の入口はひとつ**（ADR-0003）。読み出しは1クエリで、上流を呼ぶ前。
    let memories = match store.recall_memories(job.character_id).await {
        Ok(memories) => memories,
        Err(err) => {
            record_skip(SkipReason::MemoriesUnreadable, job, Some(&err));
            return;
        }
    };
    let fresh = memories_since(&memories, mark);
    let free_slots = match axis_discovery_due(slots, states.iter().copied(), fresh.len(), every) {
        AxisDiscoveryDue::Due { free_slots } => free_slots,
        AxisDiscoveryDue::NoFreeSlot => {
            record_skip(SkipReason::NoFreeSlot, job, None);
            return;
        }
        AxisDiscoveryDue::NotEnoughMemories => {
            record_skip(SkipReason::NotEnoughMemories, job, None);
            return;
        }
    };
    // **数え終えた記憶**（数えた記憶のうち最新）。候補の出どころと、試みの目印を兼ねる。
    // `Due` は 1 件以上で成り立つ（`N` ≥ 1）ので `None` にはならないが、panic させない。
    let Some(through) = fresh.last().map(|memory| memory.id) else {
        record_skip(SkipReason::NotEnoughMemories, job, None);
        return;
    };

    let allocs = match store.stat_allocs(job.character_id).await {
        Ok(allocs) => allocs,
        Err(err) => {
            record_skip(SkipReason::StatsUnreadable, job, Some(&err));
            return;
        }
    };

    // **いまある名前**（固定・試用中・合格・採用）と**使えない名前**（却下・不合格）。
    //
    // **使えない名前は新しい順**（`Store::axis_candidates` の並びのまま）。依頼には予算に入るだけ
    // 新しい方から載り、古い名前は落ちる —— **落ちても照合（下の `axis_name_clash`）が全候補と照らす**
    // （Issue origin#43 の申し送り1 / `prompt::axis_discovery` の doc）。
    let fixed: Vec<&StatAxis> = allocs.iter().map(|alloc| &alloc.axis).collect();
    let mut current = fixed.clone();
    let mut refused = Vec::new();
    for candidate in &candidates {
        if candidate.state().occupies_slot() {
            current.push(candidate.name());
        } else {
            refused.push(candidate.name());
        }
    }

    // **載せる記憶は新しい方から最大 `N` 件**（溜まっていた分が多くても、依頼を太らせない）。
    let every_count = usize::try_from(every.get()).unwrap_or(usize::MAX);
    let offered = fresh
        .get(fresh.len().saturating_sub(every_count)..)
        .unwrap_or(&fresh);
    let Some(prompt) = build_axis_discovery_prompt(&AxisDiscoveryInput {
        display_name: &job.display_name,
        current_names: &current,
        refused_names: &refused,
        memories: offered,
        // **抽出・変容と同じ「窓の半分」**（同じ1本の枠の中で順に走る / `SubContext` の doc）。
        budget: job.sub_context.prompt_limit(),
    }) else {
        record_skip(SkipReason::TooLong, job, None);
        return;
    };
    let sent_memories = prompt.included_memories();
    let sent_refused = prompt.included_refused_names();
    let messages = prompt.into_messages();
    let prompt_tokens = request_tokens(&messages);

    let request = ChatRequest {
        messages,
        // **温度は抽出・変容と同じ値**（`llm::SUB_LLM_TEMPERATURE` の doc。ここに数を書かない）。
        temperature: Some(SUB_LLM_TEMPERATURE),
        max_tokens: Some(response_tokens(job.sub_context)),
    };

    let counts = Counts {
        fresh: fresh.len(),
        sent: sent_memories,
        free_slots,
        every,
        prompt_tokens,
        refused_sent: sent_refused,
        refused_total: refused.len(),
    };

    // **トランザクションの外で通信する**（CLAUDE.md 絶対制約）。
    let completion = match llm.complete(&request).await {
        Ok(completion) => completion,
        Err(err) => {
            let miss = AxisDiscoveryMiss::UpstreamFailed;
            let attempt = record_miss(store, job, through, miss).await;
            record_run(
                job,
                AxisDiscoveryOutcome::Missed(miss),
                NO_DETAIL,
                attempt,
                &counts,
                Some(&err),
            );
            return;
        }
    };

    // **崩れていても panic しない**（絶対制約）。打ち切られた回は丸ごと捨てる。
    let reading = parse_axis_candidate(
        &completion.content,
        response_completeness(completion.finish_reason),
    );
    let (outcome, detail, attempt) = match reading {
        AxisCandidateReading::Truncated => {
            let miss = AxisDiscoveryMiss::Truncated;
            (
                AxisDiscoveryOutcome::Missed(miss),
                NO_DETAIL,
                record_miss(store, job, through, miss).await,
            )
        }
        AxisCandidateReading::Malformed(reason) => {
            let miss = AxisDiscoveryMiss::Malformed;
            (
                AxisDiscoveryOutcome::Missed(miss),
                reason.label(),
                record_miss(store, job, through, miss).await,
            )
        }
        AxisCandidateReading::Named(name) => {
            // **完全一致で捨てる**（正規化しない / `domain::axis_name_clash`）。
            let clash = axis_name_clash(
                &name,
                fixed.iter().copied(),
                candidates
                    .iter()
                    .map(|candidate| (candidate.name(), candidate.state())),
            );
            if let Some(clash) = clash {
                let miss = AxisDiscoveryMiss::Duplicate;
                (
                    AxisDiscoveryOutcome::Missed(miss),
                    clash.label(),
                    record_miss(store, job, through, miss).await,
                )
            } else {
                propose(store, job, name, through).await
            }
        }
    };

    record_run(job, outcome, detail, attempt, &counts, None);
}

/// **採った名前を試用として追記する**（候補と試みの目印は同じトランザクション）。
///
/// 書けなかった回は「採らなかった試み」として別に記録する（目印を進める）。
async fn propose(
    store: &Store,
    job: &TurnJob,
    name: StatAxis,
    through: crate::domain::MemoryId,
) -> (AxisDiscoveryOutcome, &'static str, &'static str) {
    let record = AxisCandidateRecord::Propose {
        name,
        source_memory: through,
    };
    // **往復の会話で縛って書く**（作業単位 A-10 / `StoreError::TurnSessionGone` の doc）。
    match store
        .record_axis_candidate_in_session(job.character_id, job.session_id, &record)
        .await
    {
        Ok(_) => (AxisDiscoveryOutcome::Proposed, NO_DETAIL, ATTEMPT_RECORDED),
        Err(err) => {
            // 照合の後・書く前に状況が変わった（枠が埋まった・同名が入った）か、DB の失敗。
            let detail = if matches!(err, StoreError::AxisDiscoverySlotsFull) {
                NOT_RECORDED_SLOTS_FULL
            } else if err.is_unique_violation() {
                NOT_RECORDED_TAKEN
            } else if character_vanished(&err) {
                // **キャラクターが完全に消された回は故障ではない**（ADR-0049 §決定3）。
                record_vanished(job, VANISHED_AXIS_CANDIDATE, &err);
                NOT_RECORDED_STORE_FAILED
            } else {
                tracing::warn!(
                    error = %err,
                    detail = ?err,
                    character_id = job.character_id.get(),
                    session_id = job.session_id.get(),
                    "軸の候補を書けませんでした"
                );
                NOT_RECORDED_STORE_FAILED
            };
            let miss = AxisDiscoveryMiss::NotRecorded;
            (
                AxisDiscoveryOutcome::Missed(miss),
                detail,
                record_miss(store, job, through, miss).await,
            )
        }
    }
}

/// 採らなかった試みを記録する。**失敗しても応答には影響しない**（印を返してログに残す）。
async fn record_miss(
    store: &Store,
    job: &TurnJob,
    through: crate::domain::MemoryId,
    miss: AxisDiscoveryMiss,
) -> &'static str {
    match store
        .record_axis_discovery_miss_in_session(job.character_id, job.session_id, through, miss)
        .await
    {
        Ok(()) => ATTEMPT_RECORDED,
        // **キャラクターが完全に消された回は故障ではない**（ADR-0049 §決定3）。次のターンは来ない。
        Err(err) if character_vanished(&err) => {
            record_vanished(job, VANISHED_DISCOVERY_ATTEMPT, &err);
            ATTEMPT_FAILED
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                detail = ?err,
                character_id = job.character_id.get(),
                session_id = job.session_id.get(),
                "軸の候補を尋ねた試みを記録できませんでした。次のターンにもう一度尋ねます"
            );
            ATTEMPT_FAILED
        }
    }
}

/// 1回の試みの件数（ログの欄）。
struct Counts {
    /// 目印より後に増えた、そのキャラクター自身の記憶の件数。
    fresh: usize,
    /// 依頼に載せた記憶の件数。
    sent: usize,
    /// 空いていた枠の数。
    free_slots: u32,
    /// `N`。
    every: AxisDiscoveryMemories,
    /// 依頼のトークン数（見積もり）。
    prompt_tokens: u32,
    /// **依頼に載せた使えない名前の件数**（新しい方から / 作業単位 A-9c）。
    refused_sent: usize,
    /// **使えない名前の総数**（載せきれなかった分も照合には使う）。
    refused_total: usize,
}

/// **尋ねた回を1イベントで記録する。**
///
/// **名前も生出力も載せない**（LLM 由来の文字列 / A-9a の方針）。載せるのは集計キーと件数。
/// `cause` がある回（上流を呼べなかった）は `warn`、それ以外は `info`。
fn record_run(
    job: &TurnJob,
    outcome: AxisDiscoveryOutcome,
    detail: &'static str,
    attempt: &'static str,
    counts: &Counts,
    cause: Option<&dyn std::error::Error>,
) {
    match cause {
        Some(err) => tracing::warn!(
            axis_discovery_outcome = %outcome.label(),
            axis_discovery_detail = %detail,
            axis_discovery_attempt = %attempt,
            axis_discovery_fresh_memories = counts.fresh,
            axis_discovery_sent_memories = counts.sent,
            axis_discovery_free_slots = counts.free_slots,
            axis_discovery_every = counts.every.get(),
            axis_discovery_prompt_tokens = counts.prompt_tokens,
            axis_discovery_refused_sent = counts.refused_sent,
            axis_discovery_refused_total = counts.refused_total,
            error = %err,
            detail = ?err,
            character_id = job.character_id.get(),
            session_id = job.session_id.get(),
            "{RAN_MESSAGE}"
        ),
        None => tracing::info!(
            axis_discovery_outcome = %outcome.label(),
            axis_discovery_detail = %detail,
            axis_discovery_attempt = %attempt,
            axis_discovery_fresh_memories = counts.fresh,
            axis_discovery_sent_memories = counts.sent,
            axis_discovery_free_slots = counts.free_slots,
            axis_discovery_every = counts.every.get(),
            axis_discovery_prompt_tokens = counts.prompt_tokens,
            axis_discovery_refused_sent = counts.refused_sent,
            axis_discovery_refused_total = counts.refused_total,
            character_id = job.character_id.get(),
            session_id = job.session_id.get(),
            "{RAN_MESSAGE}"
        ),
    }
}

/// **尋ねなかった回の記録。**
///
/// * **読めなかった回は `warn`**（故障。原因は `#[source]` 側）。
/// * 長すぎた回は `info`（設計どおりだが、並ぶなら名前の一覧が長すぎる合図）。
/// * **枠が無い・記憶が足りない回は `debug`** —— 毎ターン起きる正常な状態で、
///   既定のフィルタで毎ターン1行出すと他の記録が読めなくなる。
fn record_skip(reason: SkipReason, job: &TurnJob, cause: Option<&dyn std::error::Error>) {
    match (cause, reason) {
        (Some(err), _) => tracing::warn!(
            axis_discovery_skipped = %reason.label(),
            error = %err,
            detail = ?err,
            character_id = job.character_id.get(),
            session_id = job.session_id.get(),
            "{SKIPPED_MESSAGE}"
        ),
        (None, SkipReason::TooLong) => tracing::info!(
            axis_discovery_skipped = %reason.label(),
            character_id = job.character_id.get(),
            session_id = job.session_id.get(),
            "{SKIPPED_MESSAGE}"
        ),
        (None, _) => tracing::debug!(
            axis_discovery_skipped = %reason.label(),
            character_id = job.character_id.get(),
            session_id = job.session_id.get(),
            "{SKIPPED_MESSAGE}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use tokio::runtime::Runtime;

    use super::*;
    use crate::domain::{
        AxisCandidateId, AxisTrialFailure, CharacterId, CharacterUpdate, DisplayName, Handle,
        MemoryContent, MemoryId, NewMemory, Persona, ResolvedDriftMode, SessionId, StatAlloc,
        StatDriftMode, StatScale, Subject,
    };
    use crate::llm::{ChatStream, Completion, FinishReason, LlmError, LlmFuture};
    use crate::testlog::{captured_under_default_filter, has_field, line_containing};

    /// 会話するキャラクター。
    const HANDLE: &str = "aoi";
    /// 呼び名（識別子とは別の綴り）。
    const DISPLAY_NAME: &str = "葵";
    /// 会話しないキャラクター（**先に作る** —— id が 1 になるのはこちら）。
    const BYSTANDER_HANDLE: &str = "kaede";
    /// 固定の軸。
    const FIXED_AXIS: &str = "落ち着き";
    /// 候補として返させる、どの既存の名前とも違う名前。
    const NEW_NAME: &str = "照れ屋";

    /// 台本どおりに答える上流。**依頼を全部控える。**
    struct Upstream {
        replies: Mutex<VecDeque<String>>,
        finish_reason: Option<FinishReason>,
        fails: bool,
        requests: Mutex<Vec<ChatRequest>>,
    }

    impl Upstream {
        fn replying(replies: &[&str]) -> Self {
            Self::with(replies, Some(FinishReason::Stop), false)
        }

        fn with(replies: &[&str], finish_reason: Option<FinishReason>, fails: bool) -> Self {
            Self {
                replies: Mutex::new(replies.iter().map(|reply| (*reply).to_owned()).collect()),
                finish_reason,
                fails,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> usize {
            self.requests.lock().expect("ロックを取れること").len()
        }

        fn last_prompt(&self) -> String {
            let requests = self.requests.lock().expect("ロックを取れること");
            requests
                .last()
                .expect("依頼を1件以上受けていること")
                .messages
                .iter()
                .map(|message| message.content.clone())
                .collect::<Vec<_>>()
                .join("\n")
        }

        fn last_request(&self) -> ChatRequest {
            self.requests
                .lock()
                .expect("ロックを取れること")
                .last()
                .expect("依頼を1件以上受けていること")
                .clone()
        }
    }

    impl LlmClient for Upstream {
        fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            self.requests
                .lock()
                .expect("ロックを取れること")
                .push(request.clone());
            Box::pin(async move {
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
                    finish_reason: self.finish_reason,
                    usage: None,
                })
            })
        }

        fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
            Box::pin(async move { Err(LlmError::MissingContent) })
        }
    }

    /// DB とキャラクター2体（会話する方は後から作る）。
    struct Fixture {
        store: Store,
        aoi: CharacterId,
        kaede: CharacterId,
        session: SessionId,
        _dir: tempfile::TempDir,
    }

    fn runtime() -> Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime を作れること")
    }

    async fn fixture(slots: u32) -> Fixture {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let store = Store::connect(dir.path().join("personacore.sqlite"))
            .await
            .expect("DB を開けること");
        let create = |handle: &'static str, name: &'static str| {
            let store = store.clone();
            async move {
                store
                    .create_character(
                        &Handle::parse(handle).expect("正当な識別子"),
                        &DisplayName::parse(name).expect("正当な表示名"),
                        &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナ"),
                        None,
                        StatScale::parse(7).expect("正当な尺度"),
                    )
                    .await
                    .expect("キャラクターを作れること")
            }
        };
        let kaede = create(BYSTANDER_HANDLE, "楓").await;
        let aoi = create(HANDLE, DISPLAY_NAME).await;
        assert_ne!(
            aoi,
            CharacterId::from_row(1),
            "前提: 会話する方の id が 1 でない"
        );
        store
            .update_character(
                &Handle::parse(HANDLE).expect("正当な識別子"),
                &CharacterUpdate {
                    axis_discovery_slots: Some(
                        AxisDiscoverySlots::parse(slots).expect("正当な枠の数"),
                    ),
                    ..CharacterUpdate::default()
                },
            )
            .await
            .expect("枠の数を書けること");
        let scale = StatScale::parse(7).expect("正当な尺度");
        store
            .replace_stat_allocs(
                aoi,
                &[StatAlloc {
                    axis: StatAxis::parse(FIXED_AXIS).expect("正当な軸名"),
                    // **値 0 の軸も照合の相手**（本文プロンプトに出ないだけ）。
                    value: scale.value(0).expect("尺度の中"),
                }],
            )
            .await
            .expect("性格ステータスを書けること");
        let session = store
            .create_session(aoi)
            .await
            .expect("セッションを作れること");
        Fixture {
            store,
            aoi,
            kaede,
            session,
            _dir: dir,
        }
    }

    impl Fixture {
        /// キャラクター自身の記憶を `count` 件足し、最後の1件の id を返す。
        async fn remember(&self, owner: CharacterId, count: usize, tag: &str) -> MemoryId {
            let mut last = None;
            for index in 0..count {
                last = Some(
                    add(
                        &self.store,
                        owner,
                        Subject::Character,
                        &format!("{tag}{index}"),
                    )
                    .await,
                );
            }
            last.expect("1件以上足すこと")
        }

        fn job(&self, slots: u32) -> TurnJob {
            TurnJob {
                character_id: self.aoi,
                display_name: DisplayName::parse(DISPLAY_NAME).expect("正当な表示名"),
                stat_scale: StatScale::parse(7).expect("正当な尺度"),
                session_id: self.session,
                reply_id: crate::domain::MessageId::from_row(2),
                user_message: "ただいま。".to_owned(),
                character_message: "「おかえりなさい」".to_owned(),
                sub_context: SubContext::FALLBACK,
                drift_mode: ResolvedDriftMode::resolve(
                    Some(StatDriftMode::Shadow),
                    StatDriftMode::Off,
                ),
                axis_discovery_slots: AxisDiscoverySlots::parse(slots).expect("正当な枠の数"),
            }
        }

        async fn run(&self, upstream: &Upstream, slots: u32, every: u32) {
            run_axis_discovery(
                &self.store,
                upstream,
                &self.job(slots),
                AxisDiscoveryMemories::parse(every).expect("正当な間隔"),
            )
            .await;
        }

        /// 候補の (名前, いまの状態, 出どころ) を古い順に。
        async fn candidates(&self) -> Vec<(String, AxisCandidateState, MemoryId)> {
            let mut listed: Vec<(String, AxisCandidateState, MemoryId)> = self
                .store
                .axis_candidates(self.aoi, 100)
                .await
                .expect("候補を読めること")
                .iter()
                .map(|candidate| {
                    (
                        candidate.name().as_str().to_owned(),
                        candidate.state(),
                        candidate.source_memory(),
                    )
                })
                .collect();
            listed.reverse();
            listed
        }

        /// 試みの結末を古い順に。
        async fn outcomes(&self) -> Vec<String> {
            sqlx::query_scalar(
                "SELECT outcome FROM stat_axis_discovery_attempt WHERE character_id = ? ORDER BY id",
            )
            .bind(self.aoi.get())
            .fetch_all(self.store.pool())
            .await
            .expect("試みを読めること")
        }

        /// 候補を1件作り、指定の状態まで進める（**試みの目印も `source` まで進む**）。
        async fn seed_candidate(
            &self,
            name: &str,
            source: MemoryId,
            path: &[AxisCandidateState],
        ) -> AxisCandidateId {
            let id = self
                .store
                .record_axis_candidate(
                    self.aoi,
                    &AxisCandidateRecord::Propose {
                        name: StatAxis::parse(name).expect("正当な軸名"),
                        source_memory: source,
                    },
                )
                .await
                .expect("候補を作れること");
            for to in path {
                self.store
                    .record_axis_candidate(
                        self.aoi,
                        &AxisCandidateRecord::Advance {
                            candidate: id,
                            to: *to,
                        },
                    )
                    .await
                    .expect("状態を進められること");
            }
            id
        }
    }

    async fn add(store: &Store, owner: CharacterId, subject: Subject, content: &str) -> MemoryId {
        store
            .add_memory(
                owner,
                &NewMemory {
                    subject,
                    content: MemoryContent::parse(content).expect("正当な本文"),
                    category: None,
                },
            )
            .await
            .expect("記憶を足せること")
    }

    // ---------------------------------------------------------------------
    // いつ尋ねるか（Issue origin#42 の閉じる条件）
    // ---------------------------------------------------------------------

    /// **m = 0 のキャラクターには、記憶が何件あっても1回も尋ねない。試みも残らない。**
    #[test]
    fn a_character_without_slots_is_never_asked() {
        runtime().block_on(async {
            let fx = fixture(0).await;
            fx.remember(fx.aoi, 20, "記録").await;
            let upstream = Upstream::replying(&[NEW_NAME]);
            fx.run(&upstream, 0, 1).await;
            assert_eq!(upstream.calls(), 0, "m = 0 は尋ねない");
            assert!(fx.outcomes().await.is_empty());
            assert_eq!(
                fx.store
                    .axis_discovery_mark(fx.aoi)
                    .await
                    .expect("読めること"),
                None
            );
        });
    }

    /// **m = 0 のキャラクターでは DB にも触らない**（毎ターン全キャラクターで走る経路を安くする）。
    ///
    /// **観測のしかた**: 接続を閉じて、どの読みも失敗する DB にする。m = 0 で何かを読めば
    /// 「読めなかった」の `warn` が出る。**対照（m = 1）で、この DB が本当に読めないことを確かめる。**
    ///
    /// > 変異 `m16`（m = 0 の早期 return を外す）は、この1本を足すまで全緑だった ——
    /// > 空いた枠の判定（`slots.free(..) == 0`）が同じ回を止めるので、**上流の呼び出し回数では観測できない。**
    #[test]
    fn a_character_without_slots_touches_no_table() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        rt.block_on(fx.store.pool().close());
        let upstream = Upstream::replying(&[NEW_NAME]);

        let zero = captured_under_default_filter(|| rt.block_on(fx.run(&upstream, 0, 1)));
        assert!(
            !zero.contains(SKIPPED_MESSAGE) && !zero.contains(RAN_MESSAGE),
            "m = 0 は何も読まない（読めば閉じた DB で warn が出る）: {zero}"
        );

        let one = captured_under_default_filter(|| rt.block_on(fx.run(&upstream, 1, 1)));
        let line = line_containing(&one, SKIPPED_MESSAGE);
        assert!(
            has_field(line, "axis_discovery_skipped=candidates_unreadable"),
            "前提: この DB は読めないこと（m = 1 なら読みに行って止まる）: {line}"
        );
        assert_eq!(upstream.calls(), 0);
    }

    /// **空きがあり、記憶が N 件増えたときだけ1回尋ね、採った名前は試用になる。**
    ///
    /// N − 1 件 → 尋ねない。N 件目 → 1回。**増えていなければ次は尋ねない。**
    /// 出どころは**数えた記憶のうち最新**（Issue origin#42 の申し送り2）。
    #[test]
    fn a_free_slot_and_n_new_memories_ask_exactly_once() {
        runtime().block_on(async {
            let fx = fixture(2).await;
            let upstream = Upstream::replying(&[NEW_NAME]);

            fx.remember(fx.aoi, 2, "前半").await;
            fx.run(&upstream, 2, 3).await;
            assert_eq!(upstream.calls(), 0, "N − 1 件では尋ねない");

            let newest = fx.remember(fx.aoi, 1, "三件目").await;
            fx.run(&upstream, 2, 3).await;
            assert_eq!(upstream.calls(), 1, "N 件ちょうどで1回");
            assert_eq!(
                fx.candidates().await,
                vec![(NEW_NAME.to_owned(), AxisCandidateState::Trial, newest)],
                "採った名前は試用。出どころは最新の記憶"
            );
            assert_eq!(fx.outcomes().await, vec!["proposed"]);

            fx.run(&upstream, 2, 3).await;
            assert_eq!(upstream.calls(), 1, "増えていなければ尋ねない");
        });
    }

    /// **枠が埋まっていれば、記憶が何件増えても尋ねない。**
    #[test]
    fn a_full_set_of_slots_is_never_asked() {
        runtime().block_on(async {
            let fx = fixture(1).await;
            let early = fx.remember(fx.aoi, 1, "最初").await;
            fx.seed_candidate("負けず嫌い", early, &[]).await;
            fx.remember(fx.aoi, 10, "後").await;

            let upstream = Upstream::replying(&[NEW_NAME]);
            fx.run(&upstream, 1, 3).await;
            assert_eq!(upstream.calls(), 0, "試用で埋まっている");
        });
    }

    /// **他のキャラクターの記憶は数に入らない**（帰属）。
    #[test]
    fn another_characters_memories_do_not_count() {
        runtime().block_on(async {
            let fx = fixture(2).await;
            fx.remember(fx.kaede, 10, "楓の記録").await;
            fx.remember(fx.aoi, 2, "葵の記録").await;
            add(
                &fx.store,
                fx.aoi,
                Subject::User,
                "利用者は山あいの町で育った",
            )
            .await;

            let upstream = Upstream::replying(&[NEW_NAME]);
            fx.run(&upstream, 2, 3).await;
            assert_eq!(
                upstream.calls(),
                0,
                "楓の記憶と利用者についての記憶は、葵の N 件に数えない"
            );
        });
    }

    // ---------------------------------------------------------------------
    // 捨てた回も「試みた」に数える（申し送り1）
    // ---------------------------------------------------------------------

    /// **完全一致で捨てた回の後、次のターンで再び尋ねない。N 件増えたらまた尋ねる。**
    ///
    /// 候補の表からは「前回」が取れない（捨てた回は候補を作らない）ことの見張り番。
    #[test]
    fn a_discarded_answer_still_counts_as_an_attempt() {
        runtime().block_on(async {
            let fx = fixture(2).await;
            let upstream = Upstream::replying(&[FIXED_AXIS, NEW_NAME]);
            fx.remember(fx.aoi, 3, "記録").await;

            fx.run(&upstream, 2, 3).await;
            assert_eq!(upstream.calls(), 1);
            assert!(
                fx.candidates().await.is_empty(),
                "固定の軸と同じ名前は捨てる"
            );

            fx.run(&upstream, 2, 3).await;
            fx.run(&upstream, 2, 3).await;
            assert_eq!(
                upstream.calls(),
                1,
                "捨てた回の後も、増えていなければ尋ねない"
            );

            fx.remember(fx.aoi, 3, "続き").await;
            fx.run(&upstream, 2, 3).await;
            assert_eq!(upstream.calls(), 2, "N 件増えたらまた尋ねる");
            assert_eq!(fx.outcomes().await, vec!["duplicate", "proposed"]);
        });
    }

    /// **既存のどの種類の名前とも、完全一致なら捨てる**（固定・試用・合格・採用・却下・不合格）。
    ///
    /// 1つの DB に全種類を並べ、1件ずつ当てる（真ん中の候補にも当たる）。
    #[test]
    fn an_exact_match_is_discarded_for_every_kind_of_existing_name() {
        let rt = runtime();
        // 枠 4: 試用・合格・採用の3つが埋め、却下・不合格は空ける → 空き 1。
        let fx = rt.block_on(fixture(4));
        use AxisCandidateState::{Adopted, Failed, Passed, Rejected};
        let before = rt.block_on(async {
            let early = fx.remember(fx.aoi, 1, "最初").await;
            fx.seed_candidate("試用中の名前", early, &[]).await;
            fx.seed_candidate("合格した名前", early, &[Passed]).await;
            fx.seed_candidate("採用した名前", early, &[Passed, Adopted])
                .await;
            fx.seed_candidate("却下した名前", early, &[Passed, Rejected])
                .await;
            fx.seed_candidate("不合格の名前", early, &[Failed(AxisTrialFailure::Silent)])
                .await;
            fx.candidates().await
        });

        let names = [
            (FIXED_AXIS, "fixed"),
            ("試用中の名前", "trial"),
            ("合格した名前", "passed"),
            ("採用した名前", "adopted"),
            ("却下した名前", "rejected"),
            ("不合格の名前", "failed"),
        ];
        for (name, kind) in names {
            rt.block_on(fx.remember(fx.aoi, 1, name));
            let upstream = Upstream::replying(&[name]);
            let output = captured_under_default_filter(|| {
                rt.block_on(fx.run(&upstream, 4, 1));
            });
            assert_eq!(upstream.calls(), 1, "{name}: 尋ねたこと");
            let line = line_containing(&output, RAN_MESSAGE);
            assert!(
                has_field(line, "axis_discovery_outcome=duplicate"),
                "{name}: {line}"
            );
            assert!(
                has_field(line, &format!("axis_discovery_detail={kind}")),
                "{name} は {kind} と一致したと名乗ること: {line}"
            );
            assert!(!line.contains(name), "名前をログに出さない: {line}");
        }
        assert_eq!(rt.block_on(fx.candidates()), before, "候補は1件も増えない");
    }

    /// **送り仮名違いは捨てずに、別の候補として採る**（正規化しない）。
    #[test]
    fn a_differently_spelled_name_is_taken_as_a_new_candidate() {
        runtime().block_on(async {
            let fx = fixture(1).await;
            let newest = fx.remember(fx.aoi, 1, "記録").await;
            let upstream = Upstream::replying(&["- 落着き"]);
            fx.run(&upstream, 1, 1).await;
            assert_eq!(
                fx.candidates().await,
                vec![("落着き".to_owned(), AxisCandidateState::Trial, newest)],
                "固定の軸「落ち着き」と別の綴りなので採る"
            );
        });
    }

    /// **崩れた応答・打ち切られた応答・上流の失敗でも panic せず、どれも「試みた」に数える。**
    #[test]
    fn broken_answers_never_panic_and_count_as_attempts() {
        runtime().block_on(async {
            let fx = fixture(1).await;
            let too_long = "長".repeat(StatAxis::MAX_CHARS + 1);
            let cases: Vec<(Upstream, &str)> = vec![
                (Upstream::replying(&[""]), "malformed"),
                (Upstream::replying(&["- 照れ屋\n- 負けず嫌い"]), "malformed"),
                (Upstream::replying(&[too_long.as_str()]), "malformed"),
                (Upstream::replying(&["- 項目名"]), "malformed"),
                (Upstream::replying(&["なし"]), "malformed"),
                (
                    Upstream::with(&["- 照れ"], Some(FinishReason::Length), false),
                    "truncated",
                ),
                (
                    Upstream::with(&[], Some(FinishReason::Stop), true),
                    "upstream_failed",
                ),
            ];
            let mut expected = Vec::new();
            for (upstream, outcome) in &cases {
                fx.remember(fx.aoi, 1, outcome).await;
                fx.run(upstream, 1, 1).await;
                assert_eq!(upstream.calls(), 1, "{outcome}: 尋ねたこと");
                // **次のターン（増えていない）は尋ねない。**
                fx.run(upstream, 1, 1).await;
                assert_eq!(upstream.calls(), 1, "{outcome}: 捨てた後に尋ね直さない");
                expected.push(*outcome);
            }
            assert!(fx.candidates().await.is_empty(), "どれも候補にならない");
            assert_eq!(fx.outcomes().await, expected);
        });
    }

    /// **答えている間に枠の数を 0 へ落とす上流**（`PATCH` が応答の最中に来た回を演じる）。
    ///
    /// 名前は照合を通るが、書き手が `BEGIN IMMEDIATE` の中で枠を数え直して拒む
    /// （`StoreError::AxisDiscoverySlotsFull`）——**採った名前を書けなかった回**。
    struct ShrinkingUpstream {
        store: Store,
        calls: Mutex<usize>,
    }

    impl LlmClient for ShrinkingUpstream {
        fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            *self.calls.lock().expect("ロックを取れること") += 1;
            Box::pin(async move {
                self.store
                    .update_character(
                        &Handle::parse(HANDLE).expect("正当な識別子"),
                        &CharacterUpdate {
                            axis_discovery_slots: Some(AxisDiscoverySlots::NONE),
                            ..CharacterUpdate::default()
                        },
                    )
                    .await
                    .expect("枠の数を書けること");
                Ok(Completion {
                    content: NEW_NAME.to_owned(),
                    finish_reason: Some(FinishReason::Stop),
                    usage: None,
                })
            })
        }

        fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
            Box::pin(async move { Err(LlmError::MissingContent) })
        }
    }

    /// **採った名前を書けなかった回も「試みた」に数え、次のターンは尋ねない**（レビュー指摘 / A-9b）。
    ///
    /// 経路は `propose` の失敗 → `record_miss(.., NotRecorded)`。**入口の写し（`TurnJob` の `m` = 1）は
    /// 空きがあると言い、書き手は DB の `m` = 0 を読んで拒む**——この食い違いがこの経路の唯一の入口。
    #[test]
    fn a_name_that_could_not_be_written_still_counts_as_an_attempt() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        rt.block_on(fx.remember(fx.aoi, 1, "記録"));
        let upstream = ShrinkingUpstream {
            store: fx.store.clone(),
            calls: Mutex::new(0),
        };
        let run = || {
            rt.block_on(run_axis_discovery(
                &fx.store,
                &upstream,
                &fx.job(1),
                AxisDiscoveryMemories::parse(1).expect("正当な間隔"),
            ));
        };

        let output = captured_under_default_filter(run);
        assert_eq!(*upstream.calls.lock().expect("ロック"), 1, "尋ねたこと");
        let line = line_containing(&output, RAN_MESSAGE);
        assert!(
            has_field(line, "axis_discovery_outcome=not_recorded"),
            "{line}"
        );
        assert!(
            has_field(line, "axis_discovery_detail=slots_full"),
            "{line}"
        );
        assert!(
            rt.block_on(fx.candidates()).is_empty(),
            "候補は書かれていない"
        );
        assert_eq!(rt.block_on(fx.outcomes()), vec!["not_recorded"]);

        // **次のターン（記憶は増えていない・入口の写しはまだ空きがあると言う）は尋ねない。**
        run();
        assert_eq!(
            *upstream.calls.lock().expect("ロック"),
            1,
            "書けなかった回の後に尋ね直さない"
        );
    }

    /// **答えている間に、キャラクターをしまって完全に消す上流**（作業単位 A-10）。
    ///
    /// 上流が依頼を受けて返すまでの窓（＝門で止めている間）に、しまう → 消す。
    /// 読みはもう済んでいて、書き込みはこれから —— 本番で起きうる窓そのもの。
    struct DeletingUpstream {
        store: Store,
        calls: Mutex<usize>,
    }

    impl LlmClient for DeletingUpstream {
        fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            *self.calls.lock().expect("ロックを取れること") += 1;
            Box::pin(async move {
                let handle = Handle::parse(HANDLE).expect("正当な識別子");
                self.store
                    .update_character(
                        &handle,
                        &CharacterUpdate {
                            archived: Some(true),
                            ..CharacterUpdate::default()
                        },
                    )
                    .await
                    .expect("しまえること");
                self.store
                    .delete_character(&handle)
                    .await
                    .expect("しまったキャラクターは消せること");
                Ok(Completion {
                    content: NEW_NAME.to_owned(),
                    finish_reason: Some(FinishReason::Stop),
                    usage: None,
                })
            })
        }

        fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
            Box::pin(async move { Err(LlmError::MissingContent) })
        }
    }

    /// **軸を見いだしている最中にキャラクターが完全に消されたら、候補も試みも捨てて
    /// `warn` / `error` にしない**（ADR-0049 §決定3 / 作業単位 A-10 のレビュー）。
    ///
    /// 経路は `propose`（候補の書き込みが会話の縛りで落ちる）→ `record_miss`（試みの記録も落ちる）。
    /// **2つの腕をこの1本で通す**——どちらかの「消えた」の腕を外すと、その腕の `warn` が出る。
    /// 捨てたことは `info` の `VANISHED_MESSAGE` に `vanished_write=axis_candidate` と
    /// `vanished_write=axis_discovery_attempt` で残る。
    #[test]
    fn a_character_deleted_while_discovering_loses_the_candidate_without_a_warning() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        rt.block_on(fx.remember(fx.aoi, 1, "記録"));
        let upstream = DeletingUpstream {
            store: fx.store.clone(),
            calls: Mutex::new(0),
        };

        let output = captured_under_default_filter(|| {
            rt.block_on(run_axis_discovery(
                &fx.store,
                &upstream,
                &fx.job(1),
                AxisDiscoveryMemories::parse(1).expect("正当な間隔"),
            ));
        });

        assert_eq!(
            *upstream.calls.lock().expect("ロック"),
            1,
            "前提: 尋ねたこと"
        );
        assert!(
            !output.contains(" WARN ") && !output.contains(" ERROR "),
            "warn / error の行を1本も出さないこと: {output}"
        );
        for write in ["axis_candidate", "axis_discovery_attempt"] {
            assert!(
                output.lines().any(
                    |line| line.contains(super::super::extract::VANISHED_MESSAGE)
                        && line.contains(" INFO ")
                        && has_field(line, &format!("vanished_write={write}"))
                ),
                "{write} を捨てたことを info で残すこと: {output}"
            );
        }
        let count: i64 = rt
            .block_on(
                sqlx::query_scalar("SELECT count(*) FROM stat_axis_candidate")
                    .fetch_one(fx.store.pool()),
            )
            .expect("数えられること");
        assert_eq!(count, 0, "候補は1件も書かれていないこと");
    }

    /// **依頼が長すぎる回は、上流を呼ばず、試みも記録しない**（レビュー指摘 / A-9b）。
    ///
    /// 名前の一覧（削れない枠）だけで窓の半分を超える形を作る：最長（20 文字・4 バイト字）の
    /// 固定の軸を 150 本。**記録しないので、条件が揃い直した次の回に尋ねる**（尋ねていない回は試みではない）。
    #[test]
    fn a_request_too_long_to_send_is_neither_sent_nor_counted() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        let scale = StatScale::parse(7).expect("正当な尺度");
        let axes: Vec<StatAlloc> = (0..150)
            .map(|index| StatAlloc {
                axis: StatAxis::parse(&format!("{index:03}{}", "\u{20BB7}".repeat(17)))
                    .expect("正当な軸名（20 文字）"),
                value: scale.value(0).expect("尺度の中"),
            })
            .collect();
        rt.block_on(fx.store.replace_stat_allocs(fx.aoi, &axes))
            .expect("性格ステータスを書けること");
        rt.block_on(fx.remember(fx.aoi, 1, "記録"));
        let upstream = Upstream::replying(&[NEW_NAME]);

        let output = captured_under_default_filter(|| rt.block_on(fx.run(&upstream, 1, 1)));
        let line = line_containing(&output, SKIPPED_MESSAGE);
        assert!(
            has_field(line, "axis_discovery_skipped=request_too_long"),
            "前提: 長すぎる経路に入ったこと: {line}"
        );
        assert_eq!(upstream.calls(), 0, "上流を呼ばない");
        assert!(rt.block_on(fx.outcomes()).is_empty(), "試みを記録しない");
        assert_eq!(
            rt.block_on(fx.store.axis_discovery_mark(fx.aoi))
                .expect("読めること"),
            None,
            "目印も進まない"
        );
    }

    /// **却下・不合格の名前がいくら溜まっても、候補づくりは止まらない**（Issue origin#43 の申し送り1 / A-9c）。
    ///
    /// 最長（20 文字・4 バイト字）の名前の不合格の候補を 150 件（A-9b の形なら「長すぎる」で止まる量）。
    /// **依頼は送られ、新しい名前は載り、古い名前は落ちる。** 落ちた名前をモデルが返しても、
    /// **照合が捨てる**（最後の判定は依頼ではなく照合 / ADR-0046 §決定2）。
    #[test]
    fn a_long_list_of_refused_names_never_stops_the_discovery() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        let name_of = |index: usize| format!("{index:03}{}", "\u{20BB7}".repeat(17));
        rt.block_on(async {
            let early = fx.remember(fx.aoi, 1, "最初").await;
            for index in 0..150 {
                fx.seed_candidate(
                    &name_of(index),
                    early,
                    &[AxisCandidateState::Failed(AxisTrialFailure::Pinned)],
                )
                .await;
            }
            fx.remember(fx.aoi, 1, "記録").await;
        });
        let oldest = name_of(0);
        let newest = name_of(149);
        let upstream = Upstream::replying(&[oldest.as_str()]);

        let output = captured_under_default_filter(|| rt.block_on(fx.run(&upstream, 1, 1)));
        assert_eq!(upstream.calls(), 1, "名前が溜まっても尋ねること: {output}");
        let prompt = upstream.last_prompt();
        assert!(prompt.contains(&newest), "新しい名前は載ること");
        assert!(!prompt.contains(&oldest), "古い名前は落ちること");
        let line = line_containing(&output, RAN_MESSAGE);
        for field in [
            "axis_discovery_outcome=duplicate",
            "axis_discovery_detail=failed",
            "axis_discovery_refused_total=150",
        ] {
            assert!(has_field(line, field), "{field}: {line}");
        }
        let sent: usize = line
            .split_whitespace()
            .find_map(|token| token.strip_prefix("axis_discovery_refused_sent="))
            .and_then(|value| value.parse().ok())
            .expect("載せた名前の件数の欄があること");
        assert!(0 < sent && sent < 150, "一部だけ載ること: {sent}");
        assert_eq!(
            rt.block_on(fx.store.count_axis_candidates(fx.aoi))
                .expect("数えられること"),
            150,
            "落ちた名前と同じ答えは候補にならない"
        );
    }

    // ---------------------------------------------------------------------
    // 依頼の中身
    // ---------------------------------------------------------------------

    /// **依頼には、いまある名前・使えない名前・目印より後の自分の記憶だけが載る。**
    ///
    /// 載らないもの（目印以前の記憶・利用者の記憶・他のキャラクターの記憶）を同じ DB に置く。
    #[test]
    fn the_request_carries_the_names_and_only_the_new_memories_of_this_character() {
        runtime().block_on(async {
            let fx = fixture(3).await;
            let old = add(&fx.store, fx.aoi, Subject::Character, "目印より前の記録").await;
            fx.seed_candidate("試用中の名前", old, &[]).await;
            fx.seed_candidate(
                "却下した名前",
                old,
                &[AxisCandidateState::Passed, AxisCandidateState::Rejected],
            )
            .await;
            add(&fx.store, fx.kaede, Subject::Character, "楓の記録").await;
            add(
                &fx.store,
                fx.aoi,
                Subject::User,
                "利用者は山あいの町で育った",
            )
            .await;
            add(
                &fx.store,
                fx.aoi,
                Subject::Character,
                "雨の日は店を早く閉める",
            )
            .await;
            add(
                &fx.store,
                fx.aoi,
                Subject::Character,
                "常連の名前を全部覚えている",
            )
            .await;

            let upstream = Upstream::replying(&[NEW_NAME]);
            fx.run(&upstream, 3, 2).await;
            assert_eq!(upstream.calls(), 1);
            let prompt = upstream.last_prompt();
            for present in [
                FIXED_AXIS,
                "試用中の名前",
                "却下した名前",
                "雨の日は店を早く閉める",
                "常連の名前を全部覚えている",
                DISPLAY_NAME,
            ] {
                assert!(prompt.contains(present), "{present} が載ること:\n{prompt}");
            }
            for absent in ["目印より前の記録", "楓の記録", "山あいの町", HANDLE] {
                assert!(
                    !prompt.contains(absent),
                    "{absent} は載らないこと:\n{prompt}"
                );
            }

            let request = upstream.last_request();
            // **裏方の温度（外の値 0.8）**。期待値を定数から作らない。
            assert_eq!(request.temperature, Some(0.8));
            // **応答の上限は名前1行ぶん**（外の値で押さえる: 2 + 20×4 + 6 + 1）。
            assert_eq!(request.max_tokens, Some(89));
        });
    }

    /// **尋ねた回の記録は結末と件数を名乗り、名前も生出力も載せない。**
    #[test]
    fn the_record_names_the_outcome_but_never_the_name() {
        let output = captured_under_default_filter(|| {
            runtime().block_on(async {
                let fx = fixture(2).await;
                fx.remember(fx.aoi, 4, "記録").await;
                let upstream = Upstream::replying(&[NEW_NAME]);
                fx.run(&upstream, 2, 3).await;
                assert_eq!(upstream.calls(), 1);
            });
        });
        let line = line_containing(&output, RAN_MESSAGE);
        for field in [
            "axis_discovery_outcome=proposed",
            "axis_discovery_detail=-",
            "axis_discovery_attempt=recorded",
            "axis_discovery_fresh_memories=4",
            "axis_discovery_sent_memories=3",
            "axis_discovery_free_slots=2",
            "axis_discovery_every=3",
        ] {
            assert!(has_field(line, field), "{field} が載ること: {line}");
        }
        assert!(!output.contains(NEW_NAME), "名前をログに出さない: {output}");
    }
}
