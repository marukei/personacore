//! **試用中の軸の候補を、ターンごとに別の依頼で採点し、`T` 件で合否を記録する**
//! （[ADR-0046](../../docs/adr/0046-drift-proposes-new-axes.md) §決定3・§決定4 / 作業単位 A-9c / Issue origin#43）。
//!
//! 純粋関数（`domain::axis_trial` / `prompt::axis_trial` / `parse::drift`）に
//! **いつ・どう走らせるか**を与える層。ここだけが I/O を持つ。
//!
//! ```text
//! server::extract::run（抽出タスクの中。**変容の後・候補づくりの前に直列**。走らせ方が off なら呼ばれない）
//!   → run_axis_trial
//!        （枠の数 m = 0 なら、何も読まずに終わる）
//!        Store::axis_candidates        … 候補（名前・状態・票の内訳）。1文ずつ・トランザクションを跨がない
//!        domain::candidate_to_score   … 試用中の候補の輪番で1つ（純粋。鍵はそのターンの応答の id）
//!        domain::judge_axis_trial      … すでに T 件そろっていれば、尋ねずに判定する（純粋）
//!        prompt::axis_trial            … 依頼を組む（純粋。候補1本と直前の1往復だけ）
//!        （長すぎたら捨てる。数えない）
//!        LlmClient::complete           … **サブモデル**。トランザクションの外
//!        parse::drift::parse_drift_all_report … 候補1本を軸として分解（純粋。panic しない）
//!        Store::record_axis_candidate（TrialVote）… 票を1件（**値は動かさない**）
//!        domain::judge_axis_trial      … T 件そろったら判定（純粋）
//!        Store::record_axis_candidate（Advance）  … 合格／不合格を追記
//!        1イベント記録                 … 結末・票・件数・判定
//! ```
//!
//! # 固定の軸の振る舞いを1バイトも変えない（ADR-0046 §決定3）
//!
//! * **依頼が別**（`prompt::axis_trial` —— 固定の軸を1本も載せない）。
//! * **票の表が別**（試用の票の表 —— 変容の票の表には書かない / §決定6）。
//! * **値を動かさない**（性格ステータスの表にも累積の表にも触らない。許可証も持たない）。
//! * **本文のプロンプトに載らない**（この依頼は裏方の中にしか無い）。
//!
//! 見張り番は `the_fixed_axes_drift_the_same_with_or_without_a_trial`
//! （同じ台本を候補あり・なしで回して、固定の軸の依頼の本文・票・値・累積を突き合わせる）。
//! **代償は、試用中だけ裏方の呼び出しが1ターンに1回増えること**（§決定3）。
//!
//! # 何を「試用のターン」に数えるか
//!
//! **数えるのは票が書けた回だけ**（試用の票の件数そのものが `T` と比べる数）。
//!
//! | 回 | 票 | 数えるか |
//! |---|---|---|
//! | 読めた（`+1` / `-1` / `0`） | 1件 | **数える** |
//! | 崩れた応答・打ち切られた応答 | 捨てる | **数えない** |
//! | 上流を呼べなかった・依頼が長すぎた | 無い | 数えない |
//! | 票を書けなかった | 無い | 数えない |
//!
//! **崩れた回を `0` として数えない** —— 形式の崩れを「動いていない」に混ぜると、
//! **沈黙（`0` の率）が水増しされる**（ADR-0027 §12 が名指しする混同と同じ）。
//!
//! > **⚠ 代償: 一度も読める答えが返らない候補は、試用が終わらない。** 枠を埋めたまま判定されず、
//! > **輪番（次の節）で試用のターンのおよそ 1/k を食い続ける**（k = 試用中の件数）。
//! > 他の候補は進むが k/(k−1) 倍ほど遅くなり、**止まった候補1つで枠も1つなら、試用も候補づくりも止まる。**
//! > ありうる原因は**モデルが候補の名前を写せない**こと
//! > （ADR-0030 §決定3 の綴りの揺れ —— 採用後の完全一致の許可リストでも同じことが起きる）。
//! > **崩れた回はログの `axis_trial_outcome` に理由つきで残る**（`unknown_axis` 等）ので、
//! > §測る問い を回すときに率で読める。**打ち切る規則（崩れが続いたら不合格）は置いていない**
//! > —— 不合格の理由は3つ（`0015` の CHECK）しか無く、新しい理由は移行が要る。
//!
//! # 試用は1ターンに1つ、輪番で回す（A-9c のレビューで「最も古い1つ」から変えた）
//!
//! 試用中の候補が2つ以上あっても、**1ターンに採点するのは1つだけ**。どれにするかは**状態を持たない輪番**
//! で、鍵は**そのターンの応答の `message.id`**（`TurnJob::reply_id`）。
//! **最も古い1つに固定していた最初の版では、最も古い候補が止まると後ろの候補に票が1件も入らなかった。**
//! 鍵を混ぜる理由と代償は `domain::candidate_to_score` の doc。
//! 見張り番は `a_stuck_candidate_does_not_stop_the_others`（最も古い候補が毎回崩れる台本で、
//! 真ん中と最も新しい候補が判定まで進む）。
//!
//! # 崩れの内訳をログに残す
//!
//! 分解は変容と同じ `parse_drift_all_report` で、**捨てた理由**（`axis_trial_outcome`）と
//! **行頭から剥がした引用記号の数**（`axis_trial_quotes`）を毎回載せる —— §測る問い で崩れの内訳を読むため。
//!
//! # `m` を 0 に下げると試用も止まる
//!
//! **枠の数 `m` = 0 のキャラクターでは DB にも触らない**（候補づくりと同じ —— 毎ターン全キャラクターで
//! 走る経路を安くする）。`m` を下げても枠に居る候補は追い出されない（`AxisDiscoverySlots::free` の doc）
//! ので、**`m` = 0 にすると試用中の候補は票が止まったまま残り、`m` を戻すと続きから数える。**
//!
//! # 本文には一切影響しない / トランザクションを跨がない
//!
//! 戻り値を持たず、呼び出し側は結果を見ない。読みはどれも1文ずつで、
//! **`LlmClient::complete` の `await` はどのトランザクションの中にも無い**（CLAUDE.md 絶対制約）。
//! 書き手（`record_axis_candidate`）は応答を読み切って分解した**後**に呼ぶ。
//! 見張り番は `the_trial_request_is_sent_outside_any_transaction`。
//!
//! # LLM 出力の扱い
//!
//! 応答は**外部からの信頼できない入力**。ここから先へ渡るのは分解器が返した向き（3値）だけで、
//! **SQL・シェル・パスには渡らない。** **ログにも生出力と候補の名前を載せない**
//! （A-9a / A-9b の方針。候補の名前も LLM 由来の文字列。手掛かりは候補の id と結末の集計キー）。

use crate::domain::{
    AxisCandidateId, AxisCandidateRecord, AxisDiscoverySlots, AxisTrialRule, AxisTrialTally,
    AxisTrialVerdict, AxisTrialVote, DRIFT_SCORED_LABEL, DriftDelta, DriftDiscardReason,
    DriftScoring, StatAxis, candidate_to_score, judge_axis_trial, trial_vote,
};
use crate::llm::{ChatRequest, LlmClient, SUB_LLM_TEMPERATURE};
use crate::parse::drift::parse_drift_all_report;
use crate::parse::extraction::Completeness;
use crate::prompt::axis_trial::{AxisTrialInput, build_axis_trial_prompt, request_tokens};
use crate::store::Store;

use super::extract::{
    SubContext, TurnJob, character_vanished, record_vanished, response_completeness,
};

/// 試用の候補を1回採点した（または判定した）ことの記録に使う文言。**「走った」側。**
pub(in crate::server) const RAN_MESSAGE: &str = "軸の候補を試用で採点しました";

/// 採点しなかったことの記録に使う文言。**「走らなかった」側**（抽出・変容・候補づくりと同じ分け方）。
pub(in crate::server) const SKIPPED_MESSAGE: &str = "軸の候補を試用で採点しませんでした";

/// 欄に出す「該当なし」。
const NO_VALUE: &str = "-";

/// 書けた回の印（票・判定）。
const RECORDED: &str = "recorded";

/// **書けなかった回の印。故障**（票なら次のターンにまた尋ねる。判定なら次のターンに判定し直す）。
const RECORD_FAILED: &str = "failed_to_record";

/// 捨てた書き込みの種類（`vanished_write` 欄 / ADR-0049 §決定3）。**試用の票。**
const VANISHED_TRIAL_VOTE: &str = "axis_trial_vote";

/// 同上。**試用の判定。**
const VANISHED_TRIAL_VERDICT: &str = "axis_trial_verdict";

/// 結末：上流を呼べなかった。
const UPSTREAM_FAILED: &str = "upstream_failed";

/// 結末：**すでに `T` 件そろっていたので、尋ねずに判定した**（判定を書けなかった回の後・
/// `T` を下げて起こし直した後）。
const ALREADY_COMPLETE: &str = "already_complete";

/// **応答に許すトークン数**（1行ぶん）。
///
/// ```text
/// 1行 = 「- 」 ＋ 名前 ＋ 「 +1」 ＋ 改行
///     = 2 ＋ StatAxis::MAX_CHARS × UTF-8 の最大バイト長 ＋ 3 ＋ 1
/// ```
///
/// **変容の1行（`server::drift` の同じ式）と同じ形**（同じ分解器が読む1行）。
/// 2行以上は丸ごと捨てるので、長く受け取っても捨てる量が増えるだけ。
const AXIS_TRIAL_LINE_TOKENS: u32 = 2 + (StatAxis::MAX_CHARS as u32) * 4 + 3 + 1;

/// 応答の上限。**窓の残りで頭打ち**（依頼と足して実コンテキストに収める / 変容と同じ規則）。
///
/// **本文の数えた計画も読む**（裏方の1要求分の差し引き / `super::counted_plan::background_request`）。
pub(in crate::server) const fn response_tokens(sub_context: SubContext) -> u32 {
    let remainder = sub_context
        .tokens()
        .saturating_sub(sub_context.prompt_limit());
    if AXIS_TRIAL_LINE_TOKENS < remainder {
        AXIS_TRIAL_LINE_TOKENS
    } else {
        remainder
    }
}

/// 採点しなかった理由。**集計キーは値ごとの定数**（入力を反射しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkipReason {
    /// 候補を読めなかった（試用中かどうかも、票の件数も分からない）。
    CandidatesUnreadable,
    /// 試用中の候補が無い。
    NoTrial,
    /// 依頼が予算を超える（利用者の発言が極端に長い回）。**切り詰めない。**
    TooLong,
}

impl SkipReason {
    /// 集計キー。
    const fn label(self) -> &'static str {
        match self {
            Self::CandidatesUnreadable => "candidates_unreadable",
            Self::NoTrial => "no_trial",
            Self::TooLong => "request_too_long",
        }
    }
}

/// **試用の本体。** 抽出タスクの中で、変容の後・候補づくりの前に走る（`super::extract::run`）。
///
/// **呼ばれるのは効く走らせ方が off でないときだけ**（`super::extract::run` の doc）。
/// **枠の数 `m` = 0 なら何も読まずに終わる**（モジュールの doc）。
pub(in crate::server) async fn run_axis_trial(
    store: &Store,
    llm: &dyn LlmClient,
    job: &TurnJob,
    rule: AxisTrialRule,
) {
    // **m = 0 は DB にも触らない。** 既存のキャラクターは全部ここで終わる（既定 0）。
    if job.axis_discovery_slots == AxisDiscoverySlots::NONE {
        return;
    }

    // **候補**（名前・状態・票の内訳）。1件でも読めなければ止まる（`Store::axis_candidates` の doc）。
    let candidates = match store.axis_candidates(job.character_id, u32::MAX).await {
        Ok(candidates) => candidates,
        Err(err) => {
            record_skip(SkipReason::CandidatesUnreadable, job, 0, Some(&err));
            return;
        }
    };
    // **輪番**（鍵はそのターンの応答の id / `domain::candidate_to_score` の doc）。
    let chosen = candidate_to_score(
        candidates
            .iter()
            .map(|candidate| (candidate.id(), candidate.state())),
        job.reply_id,
    )
    .and_then(|id| candidates.iter().find(|candidate| candidate.id() == id));
    let Some(candidate) = chosen else {
        record_skip(SkipReason::NoTrial, job, 0, None);
        return;
    };
    let before = candidate.tally();

    // **すでに T 件そろっていれば、尋ねずに判定する**（判定を書けなかった回の後・T を下げた後）。
    // 尋ねると T + 1 件目の票が入り、「T 件で判定する」がずれる。
    let verdict = judge_axis_trial(rule, before);
    if verdict != AxisTrialVerdict::Pending {
        let advanced = advance(store, job, candidate.id(), verdict).await;
        record_run(
            job,
            &Turn {
                candidate: candidate.id(),
                outcome: ALREADY_COMPLETE,
                quotes: 0,
                vote: None,
                recorded: NO_VALUE,
                votes: total(before),
                rule,
                verdict,
                advanced,
                prompt_tokens: 0,
            },
            None,
        );
        return;
    }

    let messages = build_axis_trial_prompt(&AxisTrialInput {
        display_name: &job.display_name,
        candidate: candidate.name(),
        user_message: &job.user_message,
        character_message: &job.character_message,
    });
    // **長すぎたら丸ごと捨てる。切り詰めない**（変容と同じ「窓の半分」/ 同じ1本の枠の中で順に走る）。
    let prompt_tokens = request_tokens(&messages);
    if prompt_tokens > job.sub_context.prompt_limit() {
        record_skip(SkipReason::TooLong, job, prompt_tokens, None);
        return;
    }

    let request = ChatRequest {
        messages,
        // **温度は抽出・変容と同じ値**（`llm::SUB_LLM_TEMPERATURE` の doc。ここに数を書かない）。
        temperature: Some(SUB_LLM_TEMPERATURE),
        max_tokens: Some(response_tokens(job.sub_context)),
    };

    // **トランザクションの外で通信する**（CLAUDE.md 絶対制約）。
    let completion = match llm.complete(&request).await {
        Ok(completion) => completion,
        Err(err) => {
            record_run(
                job,
                &Turn {
                    candidate: candidate.id(),
                    outcome: UPSTREAM_FAILED,
                    quotes: 0,
                    vote: None,
                    recorded: NO_VALUE,
                    votes: total(before),
                    rule,
                    verdict: AxisTrialVerdict::Pending,
                    advanced: NO_VALUE,
                    prompt_tokens,
                },
                Some(&err),
            );
            return;
        }
    };

    // **崩れていても panic しない**（絶対制約）。打ち切られた回は丸ごと捨てる。
    let (score, quotes) = read_score(
        &completion.content,
        response_completeness(completion.finish_reason),
        candidate.name(),
    );
    let delta = match score {
        Ok(delta) => delta,
        Err(label) => {
            // **票を捨てる。試用のターンに数えない**（モジュールの doc の表）。
            record_run(
                job,
                &Turn {
                    candidate: candidate.id(),
                    outcome: label,
                    quotes,
                    vote: None,
                    recorded: NO_VALUE,
                    votes: total(before),
                    rule,
                    verdict: AxisTrialVerdict::Pending,
                    advanced: NO_VALUE,
                    prompt_tokens,
                },
                None,
            );
            return;
        }
    };

    let vote = trial_vote(delta);
    let record = AxisCandidateRecord::TrialVote {
        candidate: candidate.id(),
        session: job.session_id,
        vote,
    };
    // **往復の会話で縛って書く**（作業単位 A-10 / `StoreError::TurnSessionGone` の doc）。
    let (recorded, after) = match store
        .record_axis_candidate_in_session(job.character_id, job.session_id, &record)
        .await
    {
        Ok(_) => {
            let mut after = before;
            after.add(vote, 1);
            (RECORDED, after)
        }
        // **キャラクターが完全に消された回は故障ではない**（ADR-0049 §決定3）。
        Err(err) if character_vanished(&err) => {
            record_vanished(job, VANISHED_TRIAL_VOTE, &err);
            (RECORD_FAILED, before)
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                detail = ?err,
                character_id = job.character_id.get(),
                candidate_id = candidate.id().get(),
                session_id = job.session_id.get(),
                "試用の票を記録できませんでした"
            );
            (RECORD_FAILED, before)
        }
    };

    // **T 件そろったら判定する**（ADR-0046 §決定4）。
    let verdict = judge_axis_trial(rule, after);
    let advanced = if verdict == AxisTrialVerdict::Pending {
        NO_VALUE
    } else {
        advance(store, job, candidate.id(), verdict).await
    };

    record_run(
        job,
        &Turn {
            candidate: candidate.id(),
            outcome: DRIFT_SCORED_LABEL,
            quotes,
            vote: Some(vote),
            recorded,
            votes: total(after),
            rule,
            verdict,
            advanced,
            prompt_tokens,
        },
        None,
    );
}

/// **応答を候補1本の採点として読む。** 読めなければ集計キー（捨てた理由）を返す。
/// 右は**行頭から剥がした引用記号の数**（採否に関係なく数える）。
///
/// **分解器は変容の全軸採点と同じもの**（`parse::drift::parse_drift_all_report`）に、
/// 候補の名前1本を軸として渡す —— 受理集合を変容と揃えるため（写しを作らない）。
/// **打ち切られた回は、読めた中身によらず丸ごと捨てる**（ADR-0027 §打ち切り と同じ理由）。
/// **分解そのものは打ち切られた回も通す** —— 引用記号の数は捨てたかどうかと無関係な指標
/// （`server::drift::read_response` と同じ判断）。
fn read_score(
    raw: &str,
    completeness: Completeness,
    candidate: &StatAxis,
) -> (Result<Option<DriftDelta>, &'static str>, usize) {
    let report = parse_drift_all_report(raw, std::slice::from_ref(candidate));
    let quotes = report.quotes_stripped();
    if matches!(completeness, Completeness::Truncated) {
        return (Err(DriftDiscardReason::Truncated.label()), quotes);
    }
    let score = match report.into_scoring() {
        DriftScoring::Scored(scores) => match scores.as_slice() {
            [score] if score.axis() == candidate => Ok(score.delta()),
            // **到達しない**（分解器は渡した軸ぶんの採点しか返さない）。panic させず捨てる側へ。
            _ => Err(DriftDiscardReason::Malformed.label()),
        },
        DriftScoring::Discarded(reason) => Err(reason.label()),
    };
    (score, quotes)
}

/// **判定を追記する。** 書けたかどうかの印を返す（書けなければ次のターンに判定し直す）。
async fn advance(
    store: &Store,
    job: &TurnJob,
    candidate: AxisCandidateId,
    verdict: AxisTrialVerdict,
) -> &'static str {
    let Some(to) = verdict.next_state() else {
        return NO_VALUE;
    };
    let record = AxisCandidateRecord::Advance { candidate, to };
    match store
        .record_axis_candidate_in_session(job.character_id, job.session_id, &record)
        .await
    {
        Ok(_) => RECORDED,
        // **キャラクターが完全に消された回は故障ではない**（ADR-0049 §決定3）。
        Err(err) if character_vanished(&err) => {
            record_vanished(job, VANISHED_TRIAL_VERDICT, &err);
            RECORD_FAILED
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                detail = ?err,
                character_id = job.character_id.get(),
                candidate_id = candidate.get(),
                session_id = job.session_id.get(),
                "試用の判定を記録できませんでした。次のターンに判定し直します"
            );
            RECORD_FAILED
        }
    }
}

/// 票の合計。
fn total(tally: AxisTrialTally) -> u64 {
    u64::from(tally.up) + u64::from(tally.down) + u64::from(tally.zero)
}

/// 1回の採点の記録の中身（ログの欄）。
struct Turn {
    /// 採点した候補。
    candidate: AxisCandidateId,
    /// 結末（`scored` / 捨てた理由 / `upstream_failed` / `already_complete`）。
    outcome: &'static str,
    /// 行頭から剥がした引用記号の数（尋ねなかった回・上流を呼べなかった回は 0）。
    quotes: usize,
    /// 票（読めた回だけ）。
    vote: Option<AxisTrialVote>,
    /// 票を書けたか（`recorded` / `failed_to_record` / `-`）。
    recorded: &'static str,
    /// **この回の後の票の件数**（`T` と比べる数）。
    votes: u64,
    /// そのとき効いていたつまみ。
    rule: AxisTrialRule,
    /// 判定（`pending` / `passed` / 不合格の理由）。
    verdict: AxisTrialVerdict,
    /// 判定を書けたか（`recorded` / `failed_to_record` / `-`）。
    advanced: &'static str,
    /// 依頼のトークン数（見積もり。尋ねなかった回は 0）。
    prompt_tokens: u32,
}

/// **採点した回を1イベントで記録する。**
///
/// **名前も生出力も載せない**（LLM 由来の文字列 / モジュールの doc）。載せるのは集計キーと件数。
/// `cause` がある回（上流を呼べなかった）は `warn`、それ以外は `info`。
///
/// # 欄は結末によらず同じ（`server::drift::record_run` と同じ判断）
///
/// | 欄 | 何を読むか |
/// |---|---|
/// | `axis_trial_outcome` | **パース成功率**（`scored` とそれ以外の比 / CLAUDE.md「パース成功率をログに残す」）と、捨てた理由 |
/// | `axis_trial_quotes` | 行頭から剥がした引用記号の数（依頼の `> ` を答えへ持ち込む頻度 / 変容の同じ指標） |
/// | `axis_trial_vote` | 上げ・下げ・`0` の内訳（§測る問い の張り付き・沈黙） |
/// | `axis_trial_votes` / `axis_trial_turns` | あと何件で判定か |
/// | `axis_trial_verdict` | 判定（`pending` / `passed` / `pinned` / `silent` / `too_few_samples`） |
fn record_run(job: &TurnJob, turn: &Turn, cause: Option<&dyn std::error::Error>) {
    let vote = turn.vote.map_or(NO_VALUE, AxisTrialVote::label);
    match cause {
        Some(err) => tracing::warn!(
            axis_trial_outcome = %turn.outcome,
            axis_trial_quotes = turn.quotes,
            axis_trial_vote = %vote,
            axis_trial_recorded = %turn.recorded,
            axis_trial_votes = turn.votes,
            axis_trial_turns = turn.rule.turns(),
            axis_trial_verdict = %turn.verdict.label(),
            axis_trial_advanced = %turn.advanced,
            axis_trial_prompt_tokens = turn.prompt_tokens,
            error = %err,
            detail = ?err,
            candidate_id = turn.candidate.get(),
            character_id = job.character_id.get(),
            session_id = job.session_id.get(),
            "{RAN_MESSAGE}"
        ),
        None => tracing::info!(
            axis_trial_outcome = %turn.outcome,
            axis_trial_quotes = turn.quotes,
            axis_trial_vote = %vote,
            axis_trial_recorded = %turn.recorded,
            axis_trial_votes = turn.votes,
            axis_trial_turns = turn.rule.turns(),
            axis_trial_verdict = %turn.verdict.label(),
            axis_trial_advanced = %turn.advanced,
            axis_trial_prompt_tokens = turn.prompt_tokens,
            candidate_id = turn.candidate.get(),
            character_id = job.character_id.get(),
            session_id = job.session_id.get(),
            "{RAN_MESSAGE}"
        ),
    }
}

/// **採点しなかった回の記録。**
///
/// * **読めなかった回は `warn`**（故障。原因は `#[source]` 側）。
/// * 長すぎた回は `info`（設計どおりだが、並ぶなら利用者の発言が長すぎる合図）。
/// * **試用中の候補が無い回は `debug`** —— 枠を持つキャラクターで毎ターン起きる正常な状態。
fn record_skip(
    reason: SkipReason,
    job: &TurnJob,
    prompt_tokens: u32,
    cause: Option<&dyn std::error::Error>,
) {
    match (cause, reason) {
        (Some(err), _) => tracing::warn!(
            axis_trial_skipped = %reason.label(),
            error = %err,
            detail = ?err,
            character_id = job.character_id.get(),
            session_id = job.session_id.get(),
            "{SKIPPED_MESSAGE}"
        ),
        (None, SkipReason::TooLong) => tracing::info!(
            axis_trial_skipped = %reason.label(),
            axis_trial_prompt_tokens = prompt_tokens,
            axis_trial_limit_tokens = job.sub_context.prompt_limit(),
            character_id = job.character_id.get(),
            session_id = job.session_id.get(),
            "{SKIPPED_MESSAGE}"
        ),
        (None, _) => tracing::debug!(
            axis_trial_skipped = %reason.label(),
            character_id = job.character_id.get(),
            session_id = job.session_id.get(),
            "{SKIPPED_MESSAGE}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use tokio::runtime::Runtime;

    use super::*;
    use crate::domain::{
        AxisCandidateState, AxisDiscoveryMemories, AxisTrialFailure, Character, CharacterId,
        CharacterUpdate, DisplayName, DriftForm, DriftNote, DriftTuning, Handle, MemoryContent,
        MemoryId, MessageId, NewMemory, Persona, ResolvedDriftMode, SessionId, StatAlloc,
        StatDriftMode, StatScale, Subject,
    };
    use crate::llm::{ChatStream, Completion, FinishReason, LlmError, LlmFuture};
    use crate::server::extract::{Extraction, MemoryExtractor};
    use crate::testlog::{captured_under_default_filter, has_field, line_containing};

    /// 会話するキャラクター。
    const HANDLE: &str = "aoi";
    /// 呼び名（識別子とは別の綴り）。
    const DISPLAY_NAME: &str = "葵";
    /// 会話しないキャラクター（**先に作る** —— id が 1 になるのはこちら）。
    const BYSTANDER_HANDLE: &str = "kaede";
    /// 尺度（既定と別の値）。
    const SCALE: u32 = 7;
    /// 固定の軸（値 4）。
    const FIXED_AXIS: &str = "好奇心";
    /// 固定の軸（**値 0** —— 変容の依頼には載り、試用の依頼には載らない）。
    const ZERO_AXIS: &str = "臆病";
    /// 試用中の候補の名前（どの固定の軸とも違う綴り）。
    const CANDIDATE: &str = "照れ屋";
    /// 2つ目の候補の名前。
    const SECOND: &str = "負けず嫌い";
    /// 利用者の発言。
    const USER_MESSAGE: &str = "今日の発表、うまくいったよ。";
    /// キャラクターの応答（生出力）。
    const REPLY: &str = "「よかったですね」 彼女は湯呑みを置いた。";

    /// **試用の依頼だけに現れる見出し。** `prompt::axis_trial` の定数を呼ばず、期待値として持つ。
    const TRIAL_HEADING: &str = "# 採点する項目";
    /// **固定の軸の変容の依頼だけに現れる見出し**（同上）。
    const STATS_HEADING: &str = "# いまの性格";
    /// **候補づくりの依頼だけに現れる見出し**（同上）。
    const DISCOVERY_HEADING: &str = "# あなた自身について最近わかったこと";

    fn up() -> String {
        format!("- {CANDIDATE} +1")
    }

    fn down() -> String {
        format!("- {CANDIDATE} -1")
    }

    fn zero() -> String {
        format!("- {CANDIDATE} 0")
    }

    /// 台本どおりに答える上流。**依頼を全部控える。**
    struct Upstream {
        replies: Mutex<VecDeque<(String, Option<FinishReason>)>>,
        fails: bool,
        requests: Mutex<Vec<ChatRequest>>,
    }

    impl Upstream {
        fn replying(replies: &[String]) -> Self {
            Self {
                replies: Mutex::new(
                    replies
                        .iter()
                        .map(|reply| (reply.clone(), Some(FinishReason::Stop)))
                        .collect(),
                ),
                fails: false,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn with_finish(reply: &str, finish_reason: Option<FinishReason>) -> Self {
            Self {
                replies: Mutex::new(VecDeque::from([(reply.to_owned(), finish_reason)])),
                fails: false,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn failing() -> Self {
            Self {
                replies: Mutex::new(VecDeque::new()),
                fails: true,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> usize {
            self.requests.lock().expect("ロックを取れること").len()
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
                let (content, finish_reason) = self
                    .replies
                    .lock()
                    .expect("ロックを取れること")
                    .pop_front()
                    .unwrap_or_default();
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

    fn runtime() -> Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime を作れること")
    }

    fn joined(request: &ChatRequest) -> String {
        request
            .messages
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// DB とキャラクター2体（会話する方は後から作る）。
    struct Fixture {
        store: Store,
        aoi: CharacterId,
        kaede: CharacterId,
        session: SessionId,
        kaede_session: SessionId,
        source: MemoryId,
        /// 最後に配った応答の id（**往復ごとに 2 ずつ進める** —— 本番の `append_turn` と同じ歩幅）。
        turn: std::cell::Cell<i64>,
        _dir: tempfile::TempDir,
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
                        StatScale::parse(SCALE).expect("正当な尺度"),
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
        let scale = StatScale::parse(SCALE).expect("正当な尺度");
        store
            .replace_stat_allocs(
                aoi,
                &[
                    StatAlloc {
                        axis: StatAxis::parse(FIXED_AXIS).expect("正当な軸名"),
                        value: scale.value(4).expect("尺度の中"),
                    },
                    StatAlloc {
                        axis: StatAxis::parse(ZERO_AXIS).expect("正当な軸名"),
                        value: scale.value(0).expect("尺度の中"),
                    },
                ],
            )
            .await
            .expect("性格ステータスを書けること");
        // **楓の会話を2つ先に作る** —— 葵の会話の id（3）が葵の id（2）と同値にならないように。
        let kaede_session = store
            .create_session(kaede)
            .await
            .expect("セッションを作れること");
        store
            .create_session(kaede)
            .await
            .expect("セッションを作れること");
        let session = store
            .create_session(aoi)
            .await
            .expect("セッションを作れること");
        let source = store
            .add_memory(
                aoi,
                &NewMemory {
                    subject: Subject::Character,
                    content: MemoryContent::parse("人前で褒められると顔が赤くなる")
                        .expect("正当な本文"),
                    category: None,
                },
            )
            .await
            .expect("記憶を足せること");
        Fixture {
            store,
            aoi,
            kaede,
            session,
            kaede_session,
            source,
            turn: std::cell::Cell::new(0),
            _dir: dir,
        }
    }

    impl Fixture {
        fn job(&self, slots: u32) -> TurnJob {
            self.job_in(slots, self.session, USER_MESSAGE)
        }

        /// 次の往復の応答の id（試用の輪番の鍵）。
        fn next_reply(&self) -> MessageId {
            let next = self.turn.get() + 2;
            self.turn.set(next);
            MessageId::from_row(next)
        }

        fn job_in(&self, slots: u32, session: SessionId, user_message: &str) -> TurnJob {
            TurnJob {
                character_id: self.aoi,
                display_name: DisplayName::parse(DISPLAY_NAME).expect("正当な表示名"),
                stat_scale: StatScale::parse(SCALE).expect("正当な尺度"),
                session_id: session,
                reply_id: self.next_reply(),
                user_message: user_message.to_owned(),
                character_message: REPLY.to_owned(),
                sub_context: SubContext::FALLBACK,
                drift_mode: ResolvedDriftMode::resolve(
                    Some(StatDriftMode::Shadow),
                    StatDriftMode::Off,
                ),
                axis_discovery_slots: AxisDiscoverySlots::parse(slots).expect("正当な枠の数"),
            }
        }

        async fn run(&self, upstream: &Upstream, rule: AxisTrialRule) {
            run_axis_trial(&self.store, upstream, &self.job(1), rule).await;
        }

        /// 候補を1件作り、指定の状態まで進める。
        async fn seed(&self, name: &str, path: &[AxisCandidateState]) -> AxisCandidateId {
            let id = self
                .store
                .record_axis_candidate(
                    self.aoi,
                    &AxisCandidateRecord::Propose {
                        name: StatAxis::parse(name).expect("正当な軸名"),
                        source_memory: self.source,
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

        /// 試用の票を `(候補, 会話, 向き)` で古い順に。
        async fn trial_votes(&self) -> Vec<(i64, i64, String)> {
            sqlx::query_as(
                "SELECT candidate_id, session_id, vote FROM stat_axis_trial_vote ORDER BY id",
            )
            .fetch_all(self.store.pool())
            .await
            .expect("票を読めること")
        }

        /// 候補ごとのいまの状態（id の昇順）。
        async fn states(&self) -> Vec<(i64, AxisCandidateState)> {
            let mut listed: Vec<(i64, AxisCandidateState)> = self
                .store
                .axis_candidates(self.aoi, 100)
                .await
                .expect("候補を読めること")
                .iter()
                .map(|candidate| (candidate.id().get(), candidate.state()))
                .collect();
            listed.sort_by_key(|(id, _)| *id);
            listed
        }

        async fn fixed_values(&self) -> Vec<(String, u32)> {
            self.store
                .stat_allocs(self.aoi)
                .await
                .expect("読めること")
                .iter()
                .map(|alloc| (alloc.axis.as_str().to_owned(), alloc.value.get()))
                .collect()
        }

        async fn drift_vote_count(&self) -> i64 {
            sqlx::query_scalar("SELECT count(*) FROM stat_drift_vote")
                .fetch_one(self.store.pool())
                .await
                .expect("数えられること")
        }
    }

    fn rule(turns: u32, min_nonzero: u32, line: u32) -> AxisTrialRule {
        AxisTrialRule::new(turns, min_nonzero, line).expect("正当なつまみ")
    }

    // ---------------------------------------------------------------------
    // いつ尋ねるか
    // ---------------------------------------------------------------------

    /// **m = 0 のキャラクターでは DB にも触らない。** 対照（m = 1）で、この DB が本当に読めないことを確かめる。
    #[test]
    fn a_character_without_slots_touches_no_table() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        rt.block_on(fx.seed(CANDIDATE, &[]));
        rt.block_on(fx.store.pool().close());
        let upstream = Upstream::replying(&[up()]);

        let zero = captured_under_default_filter(|| {
            rt.block_on(run_axis_trial(
                &fx.store,
                &upstream,
                &fx.job(0),
                AxisTrialRule::DEFAULT,
            ));
        });
        assert!(
            !zero.contains(SKIPPED_MESSAGE) && !zero.contains(RAN_MESSAGE),
            "m = 0 は何も読まない（読めば閉じた DB で warn が出る）: {zero}"
        );

        let one = captured_under_default_filter(|| {
            rt.block_on(run_axis_trial(
                &fx.store,
                &upstream,
                &fx.job(1),
                AxisTrialRule::DEFAULT,
            ));
        });
        let line = line_containing(&one, SKIPPED_MESSAGE);
        assert!(
            has_field(line, "axis_trial_skipped=candidates_unreadable"),
            "前提: この DB は読めないこと: {line}"
        );
        assert_eq!(upstream.calls(), 0);
    }

    /// **試用中の候補が無ければ尋ねない**（候補が無い・合格した候補だけ・不合格の候補だけ）。
    #[test]
    fn no_candidate_on_trial_means_no_request() {
        runtime().block_on(async {
            let fx = fixture(2).await;
            let upstream = Upstream::replying(&[up()]);
            fx.run(&upstream, AxisTrialRule::DEFAULT).await;
            assert_eq!(upstream.calls(), 0, "候補が無い");

            fx.seed(CANDIDATE, &[AxisCandidateState::Passed]).await;
            fx.seed(
                SECOND,
                &[AxisCandidateState::Failed(AxisTrialFailure::Silent)],
            )
            .await;
            fx.run(&upstream, AxisTrialRule::DEFAULT).await;
            assert_eq!(upstream.calls(), 0, "試用中の候補が無い");
            assert!(fx.trial_votes().await.is_empty());
        });
    }

    // ---------------------------------------------------------------------
    // 依頼の中身
    // ---------------------------------------------------------------------

    /// **依頼には、採点する候補と直前の1往復だけが載る。固定の軸も他の候補も載らない。**
    ///
    /// 載らないものを同じ DB に置く: 固定の軸2本（**値 0 の軸を含む**）・選ばれなかった試用中の候補・
    /// 合格した候補。温度と応答の上限は外の値で押さえる。
    #[test]
    fn the_request_carries_only_the_candidate_and_the_turn() {
        runtime().block_on(async {
            let fx = fixture(3).await;
            fx.seed(CANDIDATE, &[]).await;
            fx.seed(SECOND, &[]).await;
            fx.seed("合格した名前", &[AxisCandidateState::Passed]).await;

            let upstream = Upstream::replying(&[up()]);
            fx.run(&upstream, AxisTrialRule::DEFAULT).await;
            assert_eq!(upstream.calls(), 1);
            let request = upstream.last_request();
            let prompt = joined(&request);
            for present in [USER_MESSAGE, REPLY, DISPLAY_NAME, TRIAL_HEADING] {
                assert!(prompt.contains(present), "{present} が載ること:\n{prompt}");
            }
            assert_eq!(
                usize::from(prompt.contains(CANDIDATE)) + usize::from(prompt.contains(SECOND)),
                1,
                "試用中の2件のうち、ちょうど1件だけが載ること:\n{prompt}"
            );
            for absent in [FIXED_AXIS, ZERO_AXIS, "合格した名前", HANDLE, STATS_HEADING] {
                assert!(
                    !prompt.contains(absent),
                    "{absent} は載らないこと:\n{prompt}"
                );
            }
            // **裏方の温度（外の値 0.8）**。期待値を定数から作らない。
            assert_eq!(request.temperature, Some(0.8));
            // **応答の上限は1行ぶん**（外の値: 2 + 20×4 + 3 + 1）。
            assert_eq!(request.max_tokens, Some(86));
        });
    }

    // ---------------------------------------------------------------------
    // 票
    // ---------------------------------------------------------------------

    /// **`+1` / `-1` / `0` は、それぞれ自分の種類の票になり、その往復の会話に帰属する。**
    ///
    /// **値は動かず、変容の票の表にも入らない**（ADR-0046 §決定3・6）。
    #[test]
    fn each_answer_becomes_its_own_kind_of_vote() {
        runtime().block_on(async {
            let fx = fixture(1).await;
            let candidate = fx.seed(CANDIDATE, &[]).await;
            let before = fx.fixed_values().await;
            let upstream = Upstream::replying(&[up(), down(), zero()]);
            for _ in 0..3 {
                fx.run(&upstream, AxisTrialRule::DEFAULT).await;
            }
            assert_eq!(upstream.calls(), 3);
            let expected: Vec<(i64, i64, String)> = ["up", "down", "zero"]
                .iter()
                .map(|vote| (candidate.get(), fx.session.get(), (*vote).to_owned()))
                .collect();
            assert_eq!(fx.trial_votes().await, expected);
            assert_ne!(
                fx.session.get(),
                fx.aoi.get(),
                "前提: 会話の id とキャラクターの id が別の値"
            );
            assert_eq!(fx.fixed_values().await, before, "固定の軸の値は動かない");
            assert_eq!(fx.drift_vote_count().await, 0, "変容の票の表には入らない");
            assert_eq!(
                fx.states().await,
                vec![(candidate.get(), AxisCandidateState::Trial)],
                "既定の T（60）には届かないので試用のまま"
            );
        });
    }

    /// **崩れた応答・打ち切られた応答・上流の失敗は panic せず、票を捨て、試用のターンに数えない。**
    ///
    /// 数えないことは、**その後に読めた1回で `T` = 1 がちょうど満たされる**ことで見る
    /// （捨てた回を数えていれば、読めた回の前に判定が済んでいる）。
    #[test]
    fn broken_answers_are_dropped_and_not_counted() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        let candidate = rt.block_on(fx.seed(CANDIDATE, &[]));
        let one = rule(1, 0, 90);
        let two_lines = format!("{}\n{}", up(), down());
        let cases: Vec<(Upstream, &str)> = vec![
            (Upstream::replying(&[String::new()]), "missing_axis"),
            (Upstream::replying(&[two_lines]), "duplicate_axis"),
            (
                Upstream::replying(&["- 照れた +1".to_owned()]),
                "unknown_axis",
            ),
            (
                Upstream::replying(&[format!("- {CANDIDATE} +2")]),
                "malformed",
            ),
            (
                Upstream::with_finish(&up(), Some(FinishReason::Length)),
                "truncated",
            ),
            (Upstream::failing(), "upstream_failed"),
        ];
        for (upstream, outcome) in &cases {
            let output = captured_under_default_filter(|| rt.block_on(fx.run(upstream, one)));
            assert_eq!(upstream.calls(), 1, "{outcome}: 尋ねたこと");
            let line = line_containing(&output, RAN_MESSAGE);
            assert!(
                has_field(line, &format!("axis_trial_outcome={outcome}")),
                "{outcome}: {line}"
            );
            for field in [
                "axis_trial_vote=-",
                "axis_trial_recorded=-",
                "axis_trial_votes=0",
                "axis_trial_verdict=pending",
            ] {
                assert!(has_field(line, field), "{outcome}: {field}: {line}");
            }
        }
        assert!(
            rt.block_on(fx.trial_votes()).is_empty(),
            "どの回も票にならない"
        );
        assert_eq!(
            rt.block_on(fx.states()),
            vec![(candidate.get(), AxisCandidateState::Trial)],
            "数えていないので判定されない"
        );

        let good = Upstream::replying(&[zero()]);
        rt.block_on(fx.run(&good, one));
        assert_eq!(rt.block_on(fx.trial_votes()).len(), 1);
        assert_eq!(
            rt.block_on(fx.states()),
            vec![(
                candidate.get(),
                AxisCandidateState::Failed(AxisTrialFailure::Silent)
            )],
            "読めた最初の1回で T = 1 がそろう"
        );
    }

    /// **票を書けなかった回は数えない**（別のキャラクターの会話の票を書き手が拒む / 申し送り2）。
    #[test]
    fn a_vote_that_cannot_be_written_is_not_counted() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        let candidate = rt.block_on(fx.seed(CANDIDATE, &[]));
        let upstream = Upstream::replying(&[up()]);
        let output = captured_under_default_filter(|| {
            rt.block_on(run_axis_trial(
                &fx.store,
                &upstream,
                &fx.job_in(1, fx.kaede_session, USER_MESSAGE),
                rule(1, 0, 90),
            ));
        });
        assert_eq!(upstream.calls(), 1);
        let line = line_containing(&output, RAN_MESSAGE);
        for field in [
            "axis_trial_outcome=scored",
            "axis_trial_vote=up",
            "axis_trial_recorded=failed_to_record",
            "axis_trial_votes=0",
            "axis_trial_verdict=pending",
            "axis_trial_advanced=-",
        ] {
            assert!(has_field(line, field), "{field}: {line}");
        }
        assert!(rt.block_on(fx.trial_votes()).is_empty());
        assert_eq!(
            rt.block_on(fx.states()),
            vec![(candidate.get(), AxisCandidateState::Trial)]
        );
        let _ = fx.kaede;
    }

    // ---------------------------------------------------------------------
    // キャラクターが消えた回（作業単位 A-10 / ADR-0049 §決定3）
    // ---------------------------------------------------------------------

    /// **答えている間に、キャラクターをしまって完全に消す上流**（門で止めている間に消す形）。
    struct DeletingUpstream {
        store: Store,
        reply: String,
        calls: Mutex<usize>,
    }

    impl LlmClient for DeletingUpstream {
        fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            *self.calls.lock().expect("ロックを取れること") += 1;
            Box::pin(async move {
                delete_aoi(&self.store).await;
                Ok(Completion {
                    content: self.reply.clone(),
                    finish_reason: Some(FinishReason::Stop),
                    usage: None,
                })
            })
        }

        fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
            Box::pin(async move { Err(LlmError::MissingContent) })
        }
    }

    /// 葵をしまって完全に消す。
    async fn delete_aoi(store: &Store) {
        let handle = Handle::parse(HANDLE).expect("正当な識別子");
        store
            .update_character(
                &handle,
                &CharacterUpdate {
                    archived: Some(true),
                    ..CharacterUpdate::default()
                },
            )
            .await
            .expect("しまえること");
        store
            .delete_character(&handle)
            .await
            .expect("しまったキャラクターは消せること");
    }

    /// **`warn` / `error` を出さず、`vanished_write=<write>` を `info` で残している**こと。
    fn assert_dropped_quietly(output: &str, write: &str) {
        assert!(
            !output.contains(" WARN ") && !output.contains(" ERROR "),
            "warn / error の行を1本も出さないこと: {output}"
        );
        assert!(
            output.lines().any(
                |line| line.contains(super::super::extract::VANISHED_MESSAGE)
                    && line.contains(" INFO ")
                    && has_field(line, &format!("vanished_write={write}"))
            ),
            "{write} を捨てたことを info で残すこと: {output}"
        );
    }

    /// **採点している最中にキャラクターが完全に消されたら、試用の票は捨てて `warn` にしない**
    /// （作業単位 A-10 のレビュー）。門で止めている間にしまう → 消す → 開ける。
    #[test]
    fn a_trial_vote_for_a_deleted_character_is_dropped_without_a_warning() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        rt.block_on(fx.seed(CANDIDATE, &[]));
        let upstream = DeletingUpstream {
            store: fx.store.clone(),
            reply: up(),
            calls: Mutex::new(0),
        };
        let output = captured_under_default_filter(|| {
            rt.block_on(run_axis_trial(
                &fx.store,
                &upstream,
                &fx.job(1),
                rule(3, 0, 90),
            ));
        });
        assert_eq!(
            *upstream.calls.lock().expect("ロック"),
            1,
            "前提: 尋ねたこと"
        );
        assert_dropped_quietly(&output, "axis_trial_vote");
        assert!(
            rt.block_on(fx.trial_votes()).is_empty(),
            "票は入っていないこと"
        );
        let _ = fx.kaede;
    }

    /// **判定を書く前にキャラクターが消えていたら、判定は捨てて `warn` にしない**
    /// （作業単位 A-10 のレビュー）。
    ///
    /// # 門で止める形が取れないので、id の使い回しで作る
    ///
    /// 判定（`advance`）に至る経路は「票が T 件そろった直後」と「すでにそろっていた回」の2つで、
    /// **前者は票の書き込みが先に縛りで落ちる**（票が増えないので判定に至らない）、
    /// **後者は候補を読んでから判定を書くまでに `await` が無い**（上流を呼ばない）。
    /// そこで本番で起きうるもう1つの形 —— **消して同じ id で作り直した**キャラクターに、
    /// 消す前の往復の仕事が届く —— で作る。新しいキャラクターには T 件そろった試用中の候補があり、
    /// 古い往復の会話はもう無いので、判定の書き込みは縛りで落ちる。
    #[test]
    fn a_verdict_for_a_deleted_character_is_dropped_without_a_warning() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        let old_job = fx.job(1);
        rt.block_on(delete_aoi(&fx.store));

        // **同じ識別子で作り直す**（id が使い回される）。枠・会話・記憶・T 件の票をそろえる。
        let reborn = rt.block_on(async {
            let handle = Handle::parse(HANDLE).expect("正当な識別子");
            let id = fx
                .store
                .create_character(
                    &handle,
                    &DisplayName::parse(DISPLAY_NAME).expect("正当な表示名"),
                    &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナ"),
                    None,
                    StatScale::parse(SCALE).expect("正当な尺度"),
                )
                .await
                .expect("作り直せること");
            fx.store
                .update_character(
                    &handle,
                    &CharacterUpdate {
                        axis_discovery_slots: Some(
                            AxisDiscoverySlots::parse(1).expect("正当な枠の数"),
                        ),
                        ..CharacterUpdate::default()
                    },
                )
                .await
                .expect("枠を開けられること");
            let session = fx.store.create_session(id).await.expect("会話を作れること");
            let source = fx
                .store
                .add_memory(
                    id,
                    &NewMemory {
                        subject: Subject::Character,
                        content: MemoryContent::parse("新しい記憶").expect("正当な本文"),
                        category: None,
                    },
                )
                .await
                .expect("記憶を足せること");
            let candidate = fx
                .store
                .record_axis_candidate(
                    id,
                    &AxisCandidateRecord::Propose {
                        name: StatAxis::parse(CANDIDATE).expect("正当な名前"),
                        source_memory: source,
                    },
                )
                .await
                .expect("候補を作れること");
            fx.store
                .record_axis_candidate(
                    id,
                    &AxisCandidateRecord::TrialVote {
                        candidate,
                        session,
                        vote: AxisTrialVote::Up,
                    },
                )
                .await
                .expect("票を足せること");
            (id, candidate)
        });
        assert_eq!(
            reborn.0, old_job.character_id,
            "前提: id が使い回されていること（されなければ、このテストは何も見ていない）"
        );

        let upstream = Upstream::replying(&[up()]);
        let output = captured_under_default_filter(|| {
            rt.block_on(run_axis_trial(
                &fx.store,
                &upstream,
                &old_job,
                rule(1, 0, 90),
            ));
        });
        assert_eq!(
            upstream.calls(),
            0,
            "前提: すでにそろっているので尋ねないこと"
        );
        assert_dropped_quietly(&output, "axis_trial_verdict");
        assert_eq!(
            rt.block_on(fx.states()),
            vec![(reborn.1.get(), AxisCandidateState::Trial)],
            "新しいキャラクターの候補は試用のまま（判定は書かれていない）"
        );
    }

    // ---------------------------------------------------------------------
    // 判定
    // ---------------------------------------------------------------------

    /// **T 件そろった回に判定して追記する。その前は判定しない。判定の後は尋ねない。**
    ///
    /// 4つの判定を1つずつ、票の台本とつまみで作る（境目の両側は `domain::axis_trial` が持つ）。
    #[test]
    fn t_votes_decide_the_trial_and_the_verdict_is_appended() {
        use AxisTrialFailure::{Pinned, Silent, TooFewSamples};
        let cases: Vec<(Vec<String>, AxisTrialRule, AxisCandidateState)> = vec![
            (
                vec![up(), up(), up()],
                rule(3, 1, 90),
                AxisCandidateState::Failed(Pinned),
            ),
            (
                vec![up(), down(), zero()],
                rule(3, 2, 90),
                AxisCandidateState::Passed,
            ),
            (
                vec![zero(), zero(), zero()],
                rule(3, 0, 90),
                AxisCandidateState::Failed(Silent),
            ),
            (
                vec![up(), zero(), zero()],
                rule(3, 2, 90),
                AxisCandidateState::Failed(TooFewSamples),
            ),
        ];
        for (answers, rule, expected) in cases {
            let rt = runtime();
            let fx = rt.block_on(fixture(1));
            let candidate = rt.block_on(fx.seed(CANDIDATE, &[]));
            let upstream = Upstream::replying(&answers);
            for turn in 1..=2 {
                rt.block_on(fx.run(&upstream, rule));
                assert_eq!(
                    rt.block_on(fx.states()),
                    vec![(candidate.get(), AxisCandidateState::Trial)],
                    "{expected:?}: {turn} 件目ではまだ試用"
                );
            }
            let output = captured_under_default_filter(|| rt.block_on(fx.run(&upstream, rule)));
            assert_eq!(
                rt.block_on(fx.states()),
                vec![(candidate.get(), expected)],
                "3 件目で判定"
            );
            let line = line_containing(&output, RAN_MESSAGE);
            for field in [
                "axis_trial_votes=3",
                "axis_trial_turns=3",
                "axis_trial_advanced=recorded",
            ] {
                assert!(has_field(line, field), "{expected:?}: {field}: {line}");
            }
            let verdict = expected.reason().map_or("passed", AxisTrialFailure::label);
            assert!(
                has_field(line, &format!("axis_trial_verdict={verdict}")),
                "{line}"
            );

            // **判定の後は尋ねない**（試用中の候補がもう無い）。
            rt.block_on(fx.run(&upstream, rule));
            assert_eq!(upstream.calls(), 3, "{expected:?}: 判定の後に尋ねない");
        }
    }

    /// **すでに T 件そろっている候補は、尋ねずに判定する**（判定を書けなかった回の後・T を下げた後）。
    #[test]
    fn an_already_complete_trial_is_judged_without_asking() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        let candidate = rt.block_on(fx.seed(CANDIDATE, &[]));
        for vote in [AxisTrialVote::Up, AxisTrialVote::Down, AxisTrialVote::Zero] {
            rt.block_on(fx.store.record_axis_candidate(
                fx.aoi,
                &AxisCandidateRecord::TrialVote {
                    candidate,
                    session: fx.session,
                    vote,
                },
            ))
            .expect("票を足せること");
        }
        let upstream = Upstream::replying(&[up()]);
        let output =
            captured_under_default_filter(|| rt.block_on(fx.run(&upstream, rule(2, 2, 90))));
        assert_eq!(upstream.calls(), 0, "尋ねない");
        assert_eq!(rt.block_on(fx.trial_votes()).len(), 3, "票は増えない");
        assert_eq!(
            rt.block_on(fx.states()),
            vec![(candidate.get(), AxisCandidateState::Passed)]
        );
        let line = line_containing(&output, RAN_MESSAGE);
        for field in [
            "axis_trial_outcome=already_complete",
            "axis_trial_votes=3",
            "axis_trial_verdict=passed",
            "axis_trial_advanced=recorded",
        ] {
            assert!(has_field(line, field), "{field}: {line}");
        }
    }

    /// **依頼に載った候補の名前ごとに答える上流**（どの候補が選ばれても、その候補の台本で答える）。
    struct ByName {
        /// (名前, 答え)。依頼に名前が載っていれば、その答えを返す。
        script: Vec<(String, String)>,
        requests: Mutex<Vec<String>>,
    }

    impl ByName {
        fn asked(&self, name: &str) -> usize {
            self.requests
                .lock()
                .expect("ロックを取れること")
                .iter()
                .filter(|prompt| prompt.contains(&format!("> - {name}")))
                .count()
        }
    }

    impl LlmClient for ByName {
        fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            let prompt = joined(request);
            let content = self
                .script
                .iter()
                .find(|(name, _)| prompt.contains(&format!("> - {name}")))
                .map(|(_, answer)| answer.clone())
                .unwrap_or_default();
            self.requests
                .lock()
                .expect("ロックを取れること")
                .push(prompt);
            Box::pin(async move {
                Ok(Completion {
                    content,
                    finish_reason: Some(FinishReason::Stop),
                    usage: None,
                })
            })
        }

        fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
            Box::pin(async move { Err(LlmError::MissingContent) })
        }
    }

    /// **読める答えが一度も返らない候補が最も古くても、他の候補は票を集めて判定まで進む**
    /// （A-9c のレビュー指摘1 / 輪番 —— `domain::candidate_to_score`）。
    ///
    /// 試用中が3件。**最も古い候補は毎回崩れる**（モデルが名前を写せない回を演じる —— 綴りの違う名前で答える）。
    /// 真ん中と最も新しい候補は `-1` で答え、`T` = 2 で合格する。**真ん中を端に置かない**（3件）。
    /// 「最も古い1つ」に固定する実装では、後ろの2件に票が1件も入らない。
    ///
    /// 上限 40 往復（1/3 ずつ当たれば、各 2 票まで平均 6 往復）。止まった候補も**実際に尋ねられていること**
    /// （崩れる経路に入っていること）まで見る。
    #[test]
    fn a_stuck_candidate_does_not_stop_the_others() {
        let rt = runtime();
        let fx = rt.block_on(fixture(4));
        let stuck_name = "写せない名前";
        let stuck = rt.block_on(fx.seed(stuck_name, &[]));
        let middle = rt.block_on(fx.seed(CANDIDATE, &[]));
        let newest = rt.block_on(fx.seed(SECOND, &[]));
        let upstream = ByName {
            script: vec![
                (stuck_name.to_owned(), "- 写せなかった名前 +1".to_owned()),
                (CANDIDATE.to_owned(), format!("- {CANDIDATE} -1")),
                (SECOND.to_owned(), format!("- {SECOND} -1")),
            ],
            requests: Mutex::new(Vec::new()),
        };
        let two = rule(2, 0, 90);

        let mut turns = 0;
        while turns < 40 {
            rt.block_on(run_axis_trial(&fx.store, &upstream, &fx.job(4), two));
            turns += 1;
            let states = rt.block_on(fx.states());
            if states
                .iter()
                .filter(|(_, state)| *state == AxisCandidateState::Passed)
                .count()
                == 2
            {
                break;
            }
        }
        assert_eq!(
            rt.block_on(fx.states()),
            vec![
                (stuck.get(), AxisCandidateState::Trial),
                (middle.get(), AxisCandidateState::Passed),
                (newest.get(), AxisCandidateState::Passed),
            ],
            "{turns} 往復で、止まった候補の後ろの2件が判定まで進むこと"
        );
        assert!(
            upstream.asked(stuck_name) >= 1,
            "前提: 止まった候補も尋ねられている（崩れる経路に入っている）"
        );
        let votes = rt.block_on(fx.trial_votes());
        assert!(
            votes
                .iter()
                .all(|(candidate, _, _)| *candidate != stuck.get()),
            "止まった候補には票が入らない: {votes:?}"
        );
        assert_eq!(votes.len(), 4, "後ろの2件に2票ずつ");
    }

    // ---------------------------------------------------------------------
    // トランザクション・長さ・記録
    // ---------------------------------------------------------------------

    /// **答えている間に DB へ書く上流**（書き込みのトランザクションを握ったまま待っていれば、
    /// この書き込みは `busy_timeout` まで待たされて失敗する）。
    struct WritingUpstream {
        store: Store,
        wrote: Mutex<Option<(Result<(), String>, Duration)>>,
    }

    impl LlmClient for WritingUpstream {
        fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            Box::pin(async move {
                let started = Instant::now();
                let written = self
                    .store
                    .update_character(
                        &Handle::parse(HANDLE).expect("正当な識別子"),
                        &CharacterUpdate {
                            axis_discovery_slots: Some(
                                AxisDiscoverySlots::parse(2).expect("正当な枠の数"),
                            ),
                            ..CharacterUpdate::default()
                        },
                    )
                    .await
                    .map(|_| ())
                    .map_err(|err| format!("{err:?}"));
                *self.wrote.lock().expect("ロックを取れること") =
                    Some((written, started.elapsed()));
                Ok(Completion {
                    content: format!("- {CANDIDATE} +1"),
                    finish_reason: Some(FinishReason::Stop),
                    usage: None,
                })
            })
        }

        fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
            Box::pin(async move { Err(LlmError::MissingContent) })
        }
    }

    /// **試用の依頼は、どのトランザクションの中でも待たない**（CLAUDE.md 絶対制約）。
    ///
    /// 上流が答えている間に、別の接続から書き込む。**書き込みのトランザクションを握っていれば、
    /// この書き込みは `busy_timeout`（5 秒）待たされて失敗する。** 通ること、待たされないことを見る。
    /// 票が入ることまで見る（上流が呼ばれない形で緑にならない）。
    #[test]
    fn the_trial_request_is_sent_outside_any_transaction() {
        let rt = runtime();
        let fx = rt.block_on(fx_with_candidate());
        let upstream = WritingUpstream {
            store: fx.store.clone(),
            wrote: Mutex::new(None),
        };
        rt.block_on(run_axis_trial(
            &fx.store,
            &upstream,
            &fx.job(1),
            AxisTrialRule::DEFAULT,
        ));
        let (written, elapsed) = upstream
            .wrote
            .lock()
            .expect("ロックを取れること")
            .clone()
            .expect("前提: 上流が呼ばれたこと");
        assert_eq!(written, Ok(()), "答えている間の書き込みが通ること");
        assert!(
            elapsed < Duration::from_secs(4),
            "書き込みが待たされないこと: {elapsed:?}"
        );
        assert_eq!(rt.block_on(fx.trial_votes()).len(), 1, "票が入ること");
    }

    async fn fx_with_candidate() -> Fixture {
        let fx = fixture(1).await;
        fx.seed(CANDIDATE, &[]).await;
        fx
    }

    /// **依頼が長すぎる回は、上流を呼ばず、数えない。**
    #[test]
    fn a_request_too_long_to_send_is_neither_sent_nor_counted() {
        let rt = runtime();
        let fx = rt.block_on(fx_with_candidate());
        let limit = SubContext::FALLBACK.prompt_limit() as usize;
        let long = "長".repeat(limit);
        let upstream = Upstream::replying(&[up()]);
        let output = captured_under_default_filter(|| {
            rt.block_on(run_axis_trial(
                &fx.store,
                &upstream,
                &fx.job_in(1, fx.session, &long),
                AxisTrialRule::DEFAULT,
            ));
        });
        let line = line_containing(&output, SKIPPED_MESSAGE);
        assert!(
            has_field(line, "axis_trial_skipped=request_too_long"),
            "{line}"
        );
        assert_eq!(upstream.calls(), 0);
        assert!(rt.block_on(fx.trial_votes()).is_empty());
    }

    /// **採点した回の記録は結末・票・件数・判定を名乗り、候補の名前も生出力も載せない。**
    #[test]
    fn the_record_names_the_outcome_but_never_the_name() {
        let rt = runtime();
        let fx = rt.block_on(fx_with_candidate());
        let upstream = Upstream::replying(&[down()]);
        let output = captured_under_default_filter(|| {
            rt.block_on(fx.run(&upstream, rule(60, 10, 90)));
        });
        let line = line_containing(&output, RAN_MESSAGE);
        for field in [
            "axis_trial_outcome=scored",
            "axis_trial_vote=down",
            "axis_trial_recorded=recorded",
            "axis_trial_votes=1",
            "axis_trial_turns=60",
            "axis_trial_verdict=pending",
            "axis_trial_advanced=-",
            "axis_trial_quotes=0",
        ] {
            assert!(has_field(line, field), "{field} が載ること: {line}");
        }
        assert!(
            !output.contains(CANDIDATE),
            "候補の名前をログに出さない: {output}"
        );
    }

    /// **引用記号を付けた答えも読み、剥がした数を記録に残す**（A-9c のレビュー提案3 /
    /// 変容と同じ `parse_drift_all_report`）。**打ち切られて捨てた回も数える**（採否と無関係な指標）。
    #[test]
    fn a_quoted_answer_is_read_and_the_quote_is_counted() {
        let rt = runtime();
        let fx = rt.block_on(fx_with_candidate());
        let quoted = Upstream::replying(&[format!("> {}", up())]);
        let output = captured_under_default_filter(|| {
            rt.block_on(fx.run(&quoted, AxisTrialRule::DEFAULT));
        });
        let line = line_containing(&output, RAN_MESSAGE);
        for field in [
            "axis_trial_outcome=scored",
            "axis_trial_vote=up",
            "axis_trial_quotes=1",
        ] {
            assert!(has_field(line, field), "{field}: {line}");
        }

        let cut_off = Upstream::with_finish(&format!("> {}", down()), Some(FinishReason::Length));
        let output = captured_under_default_filter(|| {
            rt.block_on(fx.run(&cut_off, AxisTrialRule::DEFAULT));
        });
        let line = line_containing(&output, RAN_MESSAGE);
        for field in ["axis_trial_outcome=truncated", "axis_trial_quotes=1"] {
            assert!(has_field(line, field), "{field}: {line}");
        }
        assert_eq!(
            rt.block_on(fx.trial_votes()).len(),
            1,
            "捨てた回は票にならない"
        );
    }

    // ---------------------------------------------------------------------
    // 裏方の1本の枠の中で（抽出 → 変容 → 試用 → 候補づくり）
    // ---------------------------------------------------------------------

    /// 裏方の上流の代役。**依頼の種類ごとに答える**（順番ではなく中身で —— `server::drift` のモックと同じ理由）。
    struct Background {
        requests: Mutex<Vec<ChatRequest>>,
        drift_reply: String,
        trial_reply: String,
        /// 抽出の依頼への答え（既定は何も書かない）。
        extraction_reply: String,
        /// 候補づくりの依頼への答え（既定は何も書かない）。
        discovery_reply: String,
    }

    impl Background {
        fn new(drift_reply: &str, trial_reply: &str) -> Arc<Self> {
            Self::with(drift_reply, trial_reply, "", "")
        }

        fn with(
            drift_reply: &str,
            trial_reply: &str,
            extraction_reply: &str,
            discovery_reply: &str,
        ) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                drift_reply: drift_reply.to_owned(),
                trial_reply: trial_reply.to_owned(),
                extraction_reply: extraction_reply.to_owned(),
                discovery_reply: discovery_reply.to_owned(),
            })
        }

        fn all(&self) -> Vec<ChatRequest> {
            self.requests.lock().expect("ロックを取れること").clone()
        }

        fn carrying(&self, heading: &str) -> Vec<ChatRequest> {
            self.all()
                .into_iter()
                .filter(|request| joined(request).contains(heading))
                .collect()
        }
    }

    impl LlmClient for Background {
        fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            self.requests
                .lock()
                .expect("ロックを取れること")
                .push(request.clone());
            let text = joined(request);
            let content = if text.contains(STATS_HEADING) {
                self.drift_reply.clone()
            } else if text.contains(TRIAL_HEADING) {
                self.trial_reply.clone()
            } else if text.contains(DISCOVERY_HEADING) {
                self.discovery_reply.clone()
            } else {
                self.extraction_reply.clone()
            };
            Box::pin(async move {
                Ok(Completion {
                    content,
                    finish_reason: Some(FinishReason::Stop),
                    usage: None,
                })
            })
        }

        fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
            Box::pin(async move { Err(LlmError::MissingContent) })
        }
    }

    /// 候補の置き方（腕）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Arm {
        /// 枠 0・候補なし。
        NoSlots,
        /// 枠 1・合格した候補（**試用ではない**）。
        NotOnTrial,
        /// 枠 1・試用中の候補。
        OnTrial,
    }

    /// 1つの腕を最後まで回した結果。
    struct Ran {
        calls: usize,
        drift_requests: Vec<ChatRequest>,
        trial_requests: usize,
        drift_votes: Vec<(i64, String, &'static str)>,
        values: Vec<(String, u32)>,
        charges: Vec<(String, u64)>,
        trial_votes: usize,
    }

    /// **同じ台本を、腕ごとに新しい DB で回す**（本番の裏方の入口 `Extraction::start` を通す）。
    fn run_background(arm: Arm, mode: StatDriftMode, turns: &[&str]) -> Ran {
        let rt = runtime();
        let slots = if arm == Arm::NoSlots { 0 } else { 1 };
        let fx = rt.block_on(fixture(slots));
        match arm {
            Arm::NoSlots => {}
            Arm::NotOnTrial => {
                rt.block_on(fx.seed(CANDIDATE, &[AxisCandidateState::Passed]));
            }
            Arm::OnTrial => {
                rt.block_on(fx.seed(CANDIDATE, &[]));
            }
        }
        // **全軸採点で、固定の軸を動かす答え**（値 0 の軸も採点に載る）。
        let sub = Background::new(
            &format!("- {FIXED_AXIS} +1\n- {ZERO_AXIS} 0"),
            &format!("- {CANDIDATE} +1"),
        );
        drive(
            &rt,
            &fx,
            &sub,
            mode,
            AxisDiscoveryMemories::DEFAULT,
            AxisTrialRule::DEFAULT,
            turns,
        );

        let drift_votes = rt
            .block_on(fx.store.recent_drift_votes(fx.aoi, 64))
            .expect("票を読めること")
            .iter()
            .map(|stored| {
                (
                    stored.session_id().get(),
                    stored.vote().axis().as_str().to_owned(),
                    stored.vote().delta().label(),
                )
            })
            .collect();
        let mut charges: Vec<(String, u64)> = rt
            .block_on(fx.store.drift_charges(fx.aoi))
            .expect("累積を読めること")
            .iter()
            .map(|stored| (stored.axis().as_str().to_owned(), stored.charge().to_bits()))
            .collect();
        charges.sort();
        Ran {
            calls: sub.all().len(),
            drift_requests: sub.carrying(STATS_HEADING),
            trial_requests: sub.carrying(TRIAL_HEADING).len(),
            drift_votes,
            values: rt.block_on(fx.fixed_values()),
            charges,
            trial_votes: rt.block_on(fx.trial_votes()).len(),
        }
    }

    /// **本番の裏方の入口（`Extraction::start`）で往復を回す**（1往復ずつ裏方が終わるまで待つ）。
    fn drive(
        rt: &Runtime,
        fx: &Fixture,
        sub: &Arc<Background>,
        mode: StatDriftMode,
        every: AxisDiscoveryMemories,
        rule: AxisTrialRule,
        turns: &[&str],
    ) {
        let llm: Arc<dyn LlmClient> = Arc::clone(sub) as Arc<dyn LlmClient>;
        let extractor = Arc::new(MemoryExtractor::new(
            fx.store.clone(),
            llm,
            super::super::context::SharedSubContext::fallback_for_tests(),
            mode,
            DriftForm::All,
            DriftNote::None,
            // **1票でちょうど1段動く安い K**（`server::drift` の `fixture_tuning` と同じ値）——
            // 値が動かない台本だと、試用が値へ漏れても観測できない。
            DriftTuning::new(1.0, 0.0, 0.065).expect("つまみが範囲内であること"),
            every,
            rule,
        ));
        let character: Character = rt
            .block_on(
                fx.store
                    .find_character_by_handle(&Handle::parse(HANDLE).expect("正当な識別子")),
            )
            .expect("引けること")
            .expect("在ること");
        let extraction = Extraction::new(Arc::clone(&extractor), &character);
        rt.block_on(async {
            for user_message in turns {
                extraction.start(fx.session, fx.next_reply(), user_message, REPLY);
                extractor.wait_for_the_running_task().await;
            }
        });
    }

    /// 1往復の抽出で増える、キャラクター自身の記憶（2 件）。
    const EXTRACTED_TWO: &str =
        "- あなた: 雨の日は店を早く閉める\n- あなた: 常連の名前を全部覚えている";

    /// **このターンに候補づくりが作った候補は、このターンには採点されない**
    /// （A-9c のレビュー指摘2 —— 「試用は候補づくりの前」の順序 / `server::extract::run` の doc）。
    ///
    /// 1往復目: 抽出が自分の記憶を 2 件返し（`N` = 2）、候補づくりが候補を1件作る。
    /// **そのターンの試用の依頼は 0 件・試用の票は 0 件**。2往復目には採点される（空でないこと）。
    /// 順序を入れ替えると、1往復目に生まれたばかりの候補を採点する。
    #[test]
    fn a_candidate_made_this_turn_is_not_scored_until_the_next_turn() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        let sub = Background::with(
            &format!("- {FIXED_AXIS} +1\n- {ZERO_AXIS} 0"),
            &format!("- {CANDIDATE} +1"),
            EXTRACTED_TWO,
            CANDIDATE,
        );
        let every = AxisDiscoveryMemories::parse(2).expect("正当な間隔");
        drive(
            &rt,
            &fx,
            &sub,
            StatDriftMode::Shadow,
            every,
            AxisTrialRule::DEFAULT,
            &TURNS[..1],
        );
        assert_eq!(
            sub.carrying(DISCOVERY_HEADING).len(),
            1,
            "前提: 候補づくりが尋ねた"
        );
        let states = rt.block_on(fx.states());
        assert_eq!(
            states.len(),
            1,
            "前提: 1往復目に候補が1件できた: {states:?}"
        );
        assert_eq!(states[0].1, AxisCandidateState::Trial);
        assert_eq!(
            sub.carrying(TRIAL_HEADING).len(),
            0,
            "生まれたターンには採点しない"
        );
        assert!(
            rt.block_on(fx.trial_votes()).is_empty(),
            "生まれたターンの票は無い"
        );

        drive(
            &rt,
            &fx,
            &sub,
            StatDriftMode::Shadow,
            every,
            AxisTrialRule::DEFAULT,
            &TURNS[1..2],
        );
        assert_eq!(
            sub.carrying(TRIAL_HEADING).len(),
            1,
            "次のターンには採点する"
        );
        assert_eq!(rt.block_on(fx.trial_votes()).len(), 1);
    }

    /// **このターンの判定で空いた枠を、このターンの候補づくりが使う**
    /// （A-9c のレビュー指摘2 の逆向きの腕 —— 順序を入れ替えると、候補づくりが先に走って枠が空いていない）。
    ///
    /// 枠 1 に試用中の候補（`0` が1票・`T` = 2）。この往復の試用で `0` の2票目 → 沈黙で不合格 → 枠が空く。
    /// 同じ往復の抽出で記憶が 2 件増え（`N` = 2）、候補づくりが新しい候補を作る。
    #[test]
    fn a_slot_freed_by_this_turns_verdict_is_used_by_this_turns_discovery() {
        let rt = runtime();
        let fx = rt.block_on(fixture(1));
        let first = rt.block_on(fx.seed(CANDIDATE, &[]));
        rt.block_on(fx.store.record_axis_candidate(
            fx.aoi,
            &AxisCandidateRecord::TrialVote {
                candidate: first,
                session: fx.session,
                vote: AxisTrialVote::Zero,
            },
        ))
        .expect("票を足せること");
        let sub = Background::with(
            &format!("- {FIXED_AXIS} +1\n- {ZERO_AXIS} 0"),
            &zero(),
            EXTRACTED_TWO,
            SECOND,
        );
        drive(
            &rt,
            &fx,
            &sub,
            StatDriftMode::Shadow,
            AxisDiscoveryMemories::parse(2).expect("正当な間隔"),
            rule(2, 0, 90),
            &TURNS[..1],
        );
        let names: Vec<(String, AxisCandidateState)> = rt
            .block_on(fx.store.axis_candidates(fx.aoi, 10))
            .expect("読めること")
            .iter()
            .rev()
            .map(|candidate| (candidate.name().as_str().to_owned(), candidate.state()))
            .collect();
        assert_eq!(
            names,
            vec![
                (
                    CANDIDATE.to_owned(),
                    AxisCandidateState::Failed(AxisTrialFailure::Silent)
                ),
                (SECOND.to_owned(), AxisCandidateState::Trial),
            ],
            "判定で空いた枠に、同じ往復の候補づくりが候補を入れる"
        );
    }

    /// 3往復の台本（**往復ごとに違う発言**）。
    const TURNS: [&str; 3] = [
        "今日の発表、うまくいったよ。",
        "明日は雨らしいね。",
        "新しい本を買ってきた。",
    ];

    /// **試用の有無で、固定の軸の依頼の本文・票・値・累積が1バイトも変わらない**
    /// （ADR-0046 §決定3 / Issue origin#43 の閉じる条件）。
    ///
    /// 同じ台本（3往復・`on`・全軸採点・1票で1段動くつまみ）を、候補なし・試用ではない候補・
    /// 試用中の候補の3つの腕で回して突き合わせる。
    ///
    /// | 見るもの | 何を捕まえるか |
    /// |---|---|
    /// | 固定の軸の依頼（全文・温度・上限を往復ごと） | 候補を固定の軸の依頼に混ぜる |
    /// | 変容の票（会話・軸・向き） | 試用の票を変容の票の表へ書く（重みが変わる） |
    /// | 値と累積（ビット列） | 試用が値へ漏れる |
    ///
    /// **腕が空でないこと**（試用の腕で試用の票が3件入り、値が実際に動いていること）も見る。
    #[test]
    fn the_fixed_axes_drift_the_same_with_or_without_a_trial() {
        let none = run_background(Arm::NoSlots, StatDriftMode::On, &TURNS);
        let passed = run_background(Arm::NotOnTrial, StatDriftMode::On, &TURNS);
        let trial = run_background(Arm::OnTrial, StatDriftMode::On, &TURNS);

        assert_eq!(trial.trial_votes, 3, "前提: 試用の腕で試用の票が入ること");
        assert_eq!(none.trial_votes + passed.trial_votes, 0);
        assert_ne!(
            none.values,
            vec![(FIXED_AXIS.to_owned(), 4), (ZERO_AXIS.to_owned(), 0)],
            "前提: 固定の軸の値が実際に動く台本であること"
        );
        assert_eq!(
            none.drift_requests.len(),
            TURNS.len(),
            "前提: 毎往復に固定の軸の依頼"
        );

        for (name, other) in [("試用ではない候補", &passed), ("試用中の候補", &trial)]
        {
            assert_eq!(
                other.drift_requests, none.drift_requests,
                "{name}: 固定の軸の依頼が変わらない"
            );
            assert_eq!(
                other.drift_votes, none.drift_votes,
                "{name}: 変容の票が変わらない"
            );
            assert_eq!(other.values, none.values, "{name}: 値が変わらない");
            assert_eq!(other.charges, none.charges, "{name}: 累積が変わらない");
        }
    }

    /// **裏方の呼び出しは試用中だけ1往復に1回増え、試用が無ければ増えない**（ADR-0046 §決定3 の代償）。
    #[test]
    fn a_trial_adds_exactly_one_request_per_turn_and_nothing_else_does() {
        let none = run_background(Arm::NoSlots, StatDriftMode::Shadow, &TURNS);
        let passed = run_background(Arm::NotOnTrial, StatDriftMode::Shadow, &TURNS);
        let trial = run_background(Arm::OnTrial, StatDriftMode::Shadow, &TURNS);
        assert_eq!(none.trial_requests, 0);
        assert_eq!(passed.trial_requests, 0);
        assert_eq!(trial.trial_requests, TURNS.len());
        assert_eq!(
            passed.calls, none.calls,
            "試用ではない候補は呼び出しを増やさない"
        );
        assert_eq!(
            trial.calls,
            none.calls + TURNS.len(),
            "試用中は1往復に1回だけ増える"
        );
    }

    /// **変容の走らせ方が off なら試用も走らない。shadow と on では走る**（ADR-0045 / Issue origin#43）。
    #[test]
    fn the_trial_runs_in_shadow_and_on_but_never_in_off() {
        for (mode, expected) in [
            (StatDriftMode::Off, 0),
            (StatDriftMode::Shadow, 1),
            (StatDriftMode::On, 1),
        ] {
            let ran = run_background(Arm::OnTrial, mode, &TURNS[..1]);
            assert_eq!(ran.trial_requests, expected, "{mode:?}: 試用の依頼");
            assert_eq!(ran.trial_votes, expected, "{mode:?}: 試用の票");
        }
    }
}
