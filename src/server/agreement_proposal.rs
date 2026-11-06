//! **約束・決まりの提案を、毎往復の裏方の列で走らせて、提案中として保存する**
//! （[ADR-0051](../../docs/adr/0051-promises-and-rules.md) §決定2・§軽い形の模索の結果と、実装する形 ——
//! **δ3 ＋ dedup2** / 作業単位 A-12c-1 / Issue origin#71）。
//!
//! 純粋関数（`prompt::agreement_proposal` / `parse::agreement_proposal`）に
//! **いつ・どう走らせるか**を与える層。ここだけが I/O を持つ。
//!
//! ```text
//! server::extract::run（抽出タスクの中。**抽出・変容・試用・候補づくりの後ろに直列**）
//!   → run_agreement_proposals
//!        AgreementProposer::take        … 1往復前の候補（プロセスの中の持ち越し）
//!        Store::turn_number             … この往復の番号（会話ごとに 1 から）
//!        ── 検出 ──
//!        prompt::detection_request      … この往復だけ。「# 提案済み」は載せない（純粋）
//!        （上限を超えたら送らない）
//!        LlmClient::complete            … **サブモデル**。トランザクションの外
//!        parse::read_detection → detection_candidates … 引用がこの往復の本文にある行だけ（純粋）
//!        ── 確かめ（持ち越した候補があるときだけ） ──
//!        Store::agreements              … 提案中・却下・有効の行（「# 提案済み」と dedup の相手）
//!        prompt::fit_check_request      … 印の往復〜この往復。上限に収める（純粋）
//!        LlmClient::complete × 1〜2     … 票が割れたらもう1本（2of3）
//!        parse::decide                  … 票・mind=2・dedup2・主語の照合（純粋）
//!        Store::record_agreement_proposals … 提案中として書く（生んだ会話に縛る）
//!        AgreementProposer::keep        … この往復の候補を次の往復へ持ち越す
//! ```
//!
//! # 抽出と同じ鎖の、いちばん後ろ（ADR-0051 §軽い形の模索の結果 —— 新しい形の条件）
//!
//! 裏方は `MemoryExtractor` の1本の枠で**直列に**走る（`extract::run`）。ここもその中に入るので、
//! **本文の数えた計画が裏方のために空けている1要求分**（`counted_plan::background_request`）に収まる ——
//! 依頼の上限は抽出と同じ `SubContext::prompt_limit()`、応答の上限 320 は抽出と同じ値。
//! **新しい調停は要らない。** 前の往復の裏方がまだ走っていれば、この往復の裏方は丸ごと飛ぶ
//! （`spawn` の `Busy`）—— 提案もその往復は飛ぶ。
//!
//! **ただし前の往復がこの段（提案）にいるなら、次の往復はそれを止めて走る**（`MemoryExtractor::spawn` の横取り /
//! 作業単位 A-12c-1 のレビュー 要修正1）。この段は毎往復走るので、止めずに捨てる側に回すと、次の往復の
//! **抽出**が `busy` で捨てられ続ける。止めた1本が失うのは持ち越しの候補とその往復の確かめだけで、書きかけの
//! トランザクションはコミットされずに巻き戻る。止めたことは [`record_preempted`] が残す。
//!
//! **変容（と試用・候補づくり）の後ろ**に置くのは、変容の依頼・票・値に何も渡さないため
//! （固定の軸の依頼が出る時刻も動かさない）。**変容の走らせ方が off でも走る**（変容とは別の機能）。
//!
//! # 候補の持ち越し（1往復）はプロセスの中（[`AgreementProposer`]）
//!
//! δ3 は「往復 t の検出で生まれた候補を、往復 t+1 の後で確かめる」（`k=1`）。
//! 持ち越しは**キャラクターごとに1つ**で、プロセスの中にだけある。**失っても拾いが減るだけ**
//! （再起動・飛んだ往復）で、壊れるものは無いので DB には持たない。
//!
//! | 次に来た往復 | 確かめに添える往復 |
//! |---|---|
//! | 同じ会話の、すぐ次の往復 | 印の往復と、この往復（`rp3.py` の `session_turns(q, q+1)`） |
//! | 別の会話・番号が飛んだ往復 | **印の往復だけ**（測定で会話の最後の往復をその場で確かめた形と同じ依頼） |
//!
//! 本番は「会話の最後の往復」を知らないので、**最後の往復の候補は、次にそのキャラクターと話したときに確かめる**
//! （測定はその往復の中で確かめていた。依頼の中身は同じで、出る時刻が1往復遅れる）。
//!
//! # 失敗したら、その往復の検出か確かめを飛ばす（鎖の他の仕事を止めない）
//!
//! | 起きたこと | 扱い |
//! |---|---|
//! | 依頼が上限を超える | 送らない（検出: 候補 0 件。確かめ: 「# 提案済み」→ 添える往復の順に削ってから） |
//! | 上流を呼べない | その依頼の側を飛ばす（確かめの1本目なら、持ち越した候補は捨てる。**2本目だけなら、1本目までの票で決める**） |
//! | 次の往復に横取りされた | 何も書かずに落ちる（持ち越しの候補とその往復の確かめを失う） |
//! | 番号・取り決めを読めない | 同上 |
//! | キャラクターが消えていた | 書かずに捨てる（`extract::record_vanished`） |
//!
//! **どの失敗も panic しない**し、戻り値も無い（呼び出し側は結果を見ない）。
//!
//! # LLM 出力の扱い
//!
//! 検出と確かめの出力は**外部からの信頼できない入力**。ここから先へ渡す経路は
//! `parse::agreement_proposal`（純粋関数）と、`AgreementContent` を通した中身の
//! `Store::record_agreement_proposals`（プレースホルダ束縛）だけで、**SQL・シェル・パスには渡らない。**
//! 依頼に戻すとき（引用・提案済みの中身）は `prompt` が無害化する。**ログにも本文を載せない**
//! （件数と定数ラベルだけ）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use super::extract::{TurnJob, character_vanished, record_vanished};
use crate::domain::{Agreement, AgreementState, CharacterId, NewAgreement, SessionId};
use crate::llm::{ChatMessage, ChatRequest, LlmClient, SUB_LLM_TEMPERATURE};
use crate::parse::agreement_proposal::{
    Candidate, CheckReading, Outcome, PRODUCTION_DEDUP, Unsaveable, decide, detection_candidates,
    needs_another_draw, proposal_to_save, read_check, read_detection,
};
use crate::prompt::agreement_proposal::{
    ListedProposal, Mark, PROPOSAL_MAX_TOKENS, ProposalTurn, detection_request, fit_check_request,
    fits,
};
use crate::prompt::extraction::request_tokens;
use crate::store::{Store, StoreError};

/// 検出を1回行ったことの記録に使う文言。
pub(in crate::server) const DETECTED_MESSAGE: &str = "約束・決まりの候補を探しました";

/// 確かめを1回行ったことの記録に使う文言（**提案を出したか・何件落としたか**もこの行に載る）。
pub(in crate::server) const CHECKED_MESSAGE: &str = "約束・決まりの候補を確かめました";

/// 検出か確かめを**行わなかった**ことの記録に使う文言（理由は `agreement_skipped` の欄）。
pub(in crate::server) const SKIPPED_MESSAGE: &str = "約束・決まりの候補の依頼を送りませんでした";

/// 捨てた書き込みの種類（`vanished_write` 欄）。**取り決めの提案。**
///
/// **綴りに表の名前そのものを使わない**（`store::agreement` の走査が表の名前の語を数える）。
const VANISHED_PROPOSAL: &str = "proposed_promise_or_rule";

/// 飛ばした理由。**集計キーは値ごとの定数**（入力を反射しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkipReason {
    /// この往復の番号を読めなかった（検出も確かめも飛ばす）。
    TurnUnreadable,
    /// 検出の依頼が上限を超えた。
    DetectionTooLong,
    /// 検出で上流を呼べなかった。
    DetectionUpstream,
    /// 提案済みの取り決めを読めなかった（確かめを飛ばす）。
    AgreementsUnreadable,
    /// 確かめの依頼が、削っても上限に収まらなかった。
    CheckTooLong,
    /// 確かめで上流を呼べなかった。
    CheckUpstream,
    /// **次の往復が来たので、この段を止めた**（`MemoryExtractor::spawn` の横取り / 作業単位 A-12c-1 のレビュー 要修正1）。
    PreemptedByNextTurn,
}

impl SkipReason {
    /// 集計キー。
    const fn label(self) -> &'static str {
        match self {
            Self::TurnUnreadable => "turn_unreadable",
            Self::DetectionTooLong => "detection_too_long",
            Self::DetectionUpstream => "detection_upstream_failed",
            Self::AgreementsUnreadable => "agreements_unreadable",
            Self::CheckTooLong => "check_too_long",
            Self::CheckUpstream => "check_upstream_failed",
            Self::PreemptedByNextTurn => "preempted_by_next_turn",
        }
    }
}

/// **提案の段を、次の往復のために止めたことを記録する**（`MemoryExtractor::spawn` が呼ぶ）。
///
/// **設計どおりの結果なので `info`**（`busy` と同じ扱い）。`session_id` は**次の往復の会話**
/// （止めた1本の会話は、spawn の時点では手元に無い —— 止めた1本が失ったのは持ち越しの候補とその往復の確かめ）。
pub(in crate::server) fn record_preempted(next: &TurnJob) {
    tracing::info!(
        agreement_skipped = %SkipReason::PreemptedByNextTurn.label(),
        session_id = next.session_id.get(),
        "{SKIPPED_MESSAGE}"
    );
}

/// **1往復前に生まれた候補**（持ち越し / モジュールの doc）。
#[derive(Debug, Clone)]
struct Carry {
    /// 候補が生まれた会話（**提案はこの会話に縛って書く**）。
    session_id: SessionId,
    /// 候補が生まれた往復（確かめに添える）。
    turn: ProposalTurn,
    /// 候補（1件以上）。
    candidates: Vec<Candidate>,
}

/// **約束・決まりの提案の、往復をまたぐ状態**（候補の持ち越し）。`MemoryExtractor` が1つだけ持つ。
///
/// **プロセスの中にしか無い**（モジュールの doc —— 失えば拾いが減るだけ）。
#[derive(Debug, Default)]
pub(in crate::server) struct AgreementProposer {
    /// キャラクターごとの持ち越し（多くて1つ）。
    carry: Mutex<HashMap<CharacterId, Carry>>,
    /// **いまこの段にいる裏方の1本の番号**（0 は「誰もいない」/ 作業単位 A-12c-1 のレビュー 要修正1）。
    ///
    /// `MemoryExtractor::spawn` が、走っている1本の番号と突き合わせて「横取りしてよいか」を決める。
    /// 番号で持つのは、止めた1本が後から印を消しても、次の1本の印を消さないため（[`Self::leave_stage`]）。
    stage: AtomicU64,
}

impl AgreementProposer {
    /// **番号 `generation` の1本が、この段に入った**（`extract::run` が呼ぶ）。
    pub(in crate::server) fn enter_stage(&self, generation: u64) {
        self.stage.store(generation, Ordering::SeqCst);
    }

    /// **番号 `generation` の1本が、この段を出た**。**別の1本の印は消さない。**
    pub(in crate::server) fn leave_stage(&self, generation: u64) {
        let _ = self
            .stage
            .compare_exchange(generation, 0, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// **番号 `generation` の1本が、いまこの段にいるか**（0 は誰でもない）。
    pub(in crate::server) fn is_in_stage(&self, generation: u64) -> bool {
        generation != 0 && self.stage.load(Ordering::SeqCst) == generation
    }

    /// 錠を取る。**毒（poison）でも進む** —— 中身は持ち越しの写しだけで、壊れた不変条件が無い。
    fn carry(&self) -> std::sync::MutexGuard<'_, HashMap<CharacterId, Carry>> {
        self.carry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// そのキャラクターの持ち越しを取り出す（**取り出したら空になる**）。
    fn take(&self, character_id: CharacterId) -> Option<Carry> {
        self.carry().remove(&character_id)
    }

    /// この往復の候補を次の往復へ持ち越す（**0 件なら持ち越さない**）。
    fn keep(&self, character_id: CharacterId, carry: Carry) {
        if carry.candidates.is_empty() {
            return;
        }
        self.carry().insert(character_id, carry);
    }

    /// **持ち越している候補の数。テスト専用。**
    #[cfg(test)]
    fn carried(&self, character_id: CharacterId) -> usize {
        self.carry()
            .get(&character_id)
            .map_or(0, |carry| carry.candidates.len())
    }
}

/// **1往復ぶんの提案の仕事**（検出 → 1往復前の候補の確かめ → 持ち越し）。**タスクの中で走る。**
///
/// 戻り値を持たない。**どんな失敗でも本文にも鎖の他の仕事にも影響しない**（モジュールの doc）。
pub(in crate::server) async fn run_agreement_proposals(
    store: &Store,
    llm: &dyn LlmClient,
    job: &TurnJob,
    proposer: &AgreementProposer,
) {
    let carried = proposer.take(job.character_id);
    let number = match store.turn_number(job.session_id, job.reply_id).await {
        Ok(number) => number,
        Err(err) => {
            record_unreadable(job, SkipReason::TurnUnreadable, &err);
            return;
        }
    };
    let current = ProposalTurn {
        number,
        user: job.user_message.clone(),
        character: job.character_message.clone(),
    };

    // ---- 検出（この往復だけ。「# 提案済み」は載せない） ----
    let candidates = detect(llm, job, &current).await;

    // ---- 1往復前の候補の確かめ ----
    if let Some(carry) = carried {
        check(store, llm, job, &current, carry).await;
    }

    proposer.keep(
        job.character_id,
        Carry {
            session_id: job.session_id,
            turn: current,
            candidates,
        },
    );
}

/// 提案の依頼の形（**温度は裏方と同じ・応答の上限は 320**）。
fn proposal_request(messages: Vec<ChatMessage>) -> ChatRequest {
    ChatRequest {
        messages,
        // **温度は抽出・変容と同じ値**（`llm::SUB_LLM_TEMPERATURE` の doc。ここに数を書かない）。
        temperature: Some(SUB_LLM_TEMPERATURE),
        max_tokens: Some(PROPOSAL_MAX_TOKENS),
    }
}

/// **検出**を1本送り、候補を返す（送れなかった・読めなかったら 0 件）。
async fn detect(llm: &dyn LlmClient, job: &TurnJob, current: &ProposalTurn) -> Vec<Candidate> {
    let limit = job.sub_context.prompt_limit();
    let messages = detection_request(&job.display_name, current);
    if !fits(&messages, limit) {
        record_too_long(job, SkipReason::DetectionTooLong, request_tokens(&messages));
        return Vec::new();
    }
    // **トランザクションの外で通信する**（CLAUDE.md 絶対制約）。
    let completion = match llm.complete(&proposal_request(messages)).await {
        Ok(completion) => completion,
        Err(err) => {
            tracing::warn!(
                agreement_skipped = %SkipReason::DetectionUpstream.label(),
                error = %err,
                detail = ?err,
                session_id = job.session_id.get(),
                "{SKIPPED_MESSAGE}"
            );
            return Vec::new();
        }
    };
    // **崩れていても panic しない**（絶対制約）。打ち切られた回も、測定の道具と同じく読めた行は読む。
    let name = job.display_name.as_str();
    let reading = read_detection(&completion.content, name);
    let found = detection_candidates(&reading, &current.user, &current.character);
    tracing::info!(
        agreement_turn = current.number,
        agreement_lines = reading.items.len(),
        agreement_candidates = found.candidates.len(),
        agreement_quote_missing = found.quote_missing,
        agreement_not_agreement = found.not_agreement,
        agreement_merged = found.merged,
        agreement_unread_lines = reading.unread_lines,
        agreement_nothing = reading.nothing,
        session_id = job.session_id.get(),
        "{DETECTED_MESSAGE}"
    );
    found.candidates
}

/// **1往復前の候補を確かめ、通ったものを提案中として書く。**
async fn check(
    store: &Store,
    llm: &dyn LlmClient,
    job: &TurnJob,
    current: &ProposalTurn,
    carry: Carry,
) {
    // **「# 提案済み」と dedup の相手**: そのキャラクターの提案中・却下・有効の行（新しい順）。
    let agreements = match store.agreements(job.character_id).await {
        Ok(agreements) => agreements,
        Err(err) => {
            record_unreadable(job, SkipReason::AgreementsUnreadable, &err);
            return;
        }
    };
    let listed_rows: Vec<&Agreement> = agreements
        .iter()
        .rev()
        .filter(|agreement| {
            matches!(
                agreement.state,
                AgreementState::Proposed | AgreementState::Rejected | AgreementState::Active
            )
        })
        .collect();
    let listed: Vec<ListedProposal<'_>> = listed_rows
        .iter()
        .map(|agreement| ListedProposal {
            keeper: agreement.keeper,
            content: agreement.content.as_str(),
        })
        .collect();
    let existing: Vec<&str> = listed_rows
        .iter()
        .map(|agreement| agreement.content.as_str())
        .collect();

    // **添える往復**: 同じ会話のすぐ次の往復なら、印の往復とこの往復。そうでなければ印の往復だけ。
    let adjacent = carry.session_id == job.session_id
        && carry.turn.number.checked_add(1) == Some(current.number);
    let mut window = vec![carry.turn.clone()];
    if adjacent {
        window.push(current.clone());
    }
    let marks: Vec<Mark<'_>> = carry
        .candidates
        .iter()
        .map(|candidate| Mark {
            turn: carry.turn.number,
            from_user: candidate.from_user,
            quote: &candidate.quote,
        })
        .collect();
    let limit = job.sub_context.prompt_limit();
    let Some(fitted) = fit_check_request(&job.display_name, &window, &marks, &listed, limit) else {
        let smallest = crate::prompt::agreement_proposal::check_request(
            &job.display_name,
            window.get(..1).unwrap_or(&[]),
            &marks,
            &[],
        );
        record_too_long(job, SkipReason::CheckTooLong, request_tokens(&smallest));
        return;
    };

    let name = job.display_name.as_str();
    let request = proposal_request(fitted.messages);
    let mut draws: Vec<CheckReading> = Vec::new();
    for _ in 0..2 {
        // **トランザクションの外で通信する**（CLAUDE.md 絶対制約）。
        let completion = match llm.complete(&request).await {
            Ok(completion) => completion,
            Err(err) => {
                tracing::warn!(
                    agreement_skipped = %SkipReason::CheckUpstream.label(),
                    agreement_draws = draws.len(),
                    error = %err,
                    detail = ?err,
                    session_id = job.session_id.get(),
                    "{SKIPPED_MESSAGE}"
                );
                // **1本目も送れなかったら確かめを飛ばす。** 2本目だけが失敗したなら、1本目までの票で決める
                // （検出と1本目がそろった候補は決まっている。割れた候補は多数が無いので出ない /
                // 作業単位 A-12c-1 のレビューの提案2）。
                if draws.is_empty() {
                    return;
                }
                break;
            }
        };
        draws.push(read_check(
            &completion.content,
            name,
            carry.candidates.len(),
        ));
        // **割れた候補が残ったときだけ、もう1本**（2of3 / `rp3.py` の `Delta.check`）。
        if draws
            .first()
            .is_none_or(|first| !needs_another_draw(&carry.candidates, first))
        {
            break;
        }
    }

    let decided = decide(&carry.candidates, &draws, name, &existing, PRODUCTION_DEDUP);
    let mut tally = CheckTally::default();
    let mut proposals: Vec<NewAgreement> = Vec::new();
    for outcome in &decided.outcomes {
        match outcome {
            Outcome::NotKept => tally.not_kept += 1,
            Outcome::Deduped => tally.deduped += 1,
            Outcome::SubjectMismatch => tally.subject_mismatch += 1,
            Outcome::Proposed { keeper, content } => match proposal_to_save(keeper, content) {
                Ok(proposal) => proposals.push(proposal),
                Err(Unsaveable::UnclearKeeper) => tally.unclear_keeper += 1,
                Err(Unsaveable::InvalidContent) => tally.invalid_content += 1,
            },
        }
    }

    let (written, duplicates) = if proposals.is_empty() {
        (0, 0)
    } else {
        // **生んだ会話に縛って書く**（ADR-0049 §決定3）。消えていれば何も書かれない。
        match store
            .record_agreement_proposals(job.character_id, carry.session_id, &proposals)
            .await
        {
            Ok(recorded) => (recorded.written, recorded.duplicates),
            Err(err) if character_vanished(&err) => {
                record_vanished(job, VANISHED_PROPOSAL, &err);
                (0, 0)
            }
            Err(err) => {
                tracing::error!(
                    error = %err,
                    detail = ?err,
                    session_id = carry.session_id.get(),
                    "約束・決まりの提案を保存できませんでした"
                );
                (0, 0)
            }
        }
    };

    tracing::info!(
        agreement_turn = carry.turn.number,
        agreement_checked = carry.candidates.len(),
        agreement_draws = draws.len(),
        agreement_listed = fitted.listed,
        agreement_window_turns = fitted.turns,
        agreement_proposed = written,
        agreement_duplicates = duplicates,
        agreement_deduped = tally.deduped,
        agreement_subject_mismatch = tally.subject_mismatch,
        agreement_not_kept = tally.not_kept,
        agreement_mind_flips = decided.mind_flips,
        agreement_unclear_keeper = tally.unclear_keeper,
        agreement_invalid_content = tally.invalid_content,
        session_id = carry.session_id.get(),
        "{CHECKED_MESSAGE}"
    );
}

/// 確かめで落とした件数の内訳（ログの欄）。
#[derive(Debug, Default)]
struct CheckTally {
    not_kept: usize,
    deduped: usize,
    subject_mismatch: usize,
    unclear_keeper: usize,
    invalid_content: usize,
}

/// **上限を超えたので送らなかった回を記録する。** 設計どおりの結果なので `info`。
/// 見積もり（`agreement_prompt_tokens`）と上限を並べて出す。
fn record_too_long(job: &TurnJob, reason: SkipReason, prompt_tokens: u32) {
    tracing::info!(
        agreement_skipped = %reason.label(),
        agreement_prompt_tokens = prompt_tokens,
        agreement_limit_tokens = job.sub_context.prompt_limit(),
        session_id = job.session_id.get(),
        "{SKIPPED_MESSAGE}"
    );
}

/// **読めなかったので飛ばした回を記録する。** 故障なので `warn`。
///
/// **見積もりの欄は出さない**（依頼を組む前に止まっていて、測っていない —— 0 と書くと「0 トークンの依頼」に
/// 読める / 作業単位 A-12c-1 のレビューの提案3）。
fn record_unreadable(job: &TurnJob, reason: SkipReason, err: &StoreError) {
    tracing::warn!(
        agreement_skipped = %reason.label(),
        agreement_limit_tokens = job.sub_context.prompt_limit(),
        error = %err,
        detail = ?err,
        session_id = job.session_id.get(),
        "{SKIPPED_MESSAGE}"
    );
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use tokio::runtime::Runtime;

    use super::*;
    use crate::domain::{
        AgreementContent, AgreementEnding, AgreementKeeper, AgreementKind, AxisDiscoverySlots,
        DisplayName, Handle, MessageId, NewMessage, Persona, ResolvedDriftMode, Role,
        StatDriftMode, StatScale,
    };
    use crate::llm::{ChatStream, Completion, FinishReason, LlmError, LlmFuture};
    use crate::server::extract::SubContext;
    use crate::testlog::{captured_under_default_filter, has_field, line_containing};

    const DISPLAY_NAME: &str = "葵";

    /// 1往復目の利用者の発言（**引用はこの中にある**）。
    const FIRST_USER: &str = "わかったよ。もう黙って出かけない。";
    /// 1往復目の応答。
    const FIRST_REPLY: &str = "「約束ですよ」 彼女は笑った。";
    /// 2往復目。
    const SECOND_USER: &str = "ただいま。";
    const SECOND_REPLY: &str = "「おかえりなさい」";

    /// 検出の答え（1往復目の利用者の発言から1件）。
    const DETECTED: &str =
        "抜｜「もう黙って出かけない」｜取り決め｜利用者｜利用者は黙って出かけない";
    /// 何も無い検出。
    const NOTHING: &str = "なし";
    /// 確かめの答え（残す・利用者 —— 検出とそろう）。
    const KEPT: &str = "点検｜1｜黙って出かける｜取り決め｜利用者｜残す｜利用者は黙って出かけない";
    /// 確かめの答え（落とす —— 検出と割れる）。
    const DROPPED: &str = "点検｜1｜黙って出かける｜その場｜利用者｜落とす｜-";

    /// **検出の依頼だけに現れる綴り**（`jd-v3` の書き方の行）。**`prompt` の定数を呼ばず、期待値として持つ。**
    const DETECTION_MARKER: &str = "抜｜引用｜取り決め｜守る側｜中身";
    /// 確かめの依頼だけに現れる綴り（`c1-v1` の書き方の行）。
    const CHECK_MARKER: &str = "点検｜番号｜行い｜分類｜守る側｜判定｜中身";

    /// 提案の依頼の上流の代役。**受けた依頼を控え、台本どおりに返す**（尽きたら空）。
    struct MockSub {
        requests: Mutex<Vec<ChatRequest>>,
        replies: Mutex<VecDeque<Result<String, ()>>>,
    }

    impl MockSub {
        fn replying(replies: &[Result<&str, ()>]) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                replies: Mutex::new(
                    replies
                        .iter()
                        .map(|reply| reply.map(str::to_owned))
                        .collect(),
                ),
            })
        }

        fn requests(&self) -> Vec<ChatRequest> {
            self.requests.lock().expect("ロック").clone()
        }

        fn users(&self) -> Vec<String> {
            self.requests()
                .iter()
                .map(|request| {
                    request
                        .messages
                        .last()
                        .map(|message| message.content.clone())
                        .unwrap_or_default()
                })
                .collect()
        }
    }

    impl LlmClient for MockSub {
        fn pinned(&self) -> Option<Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            self.requests.lock().expect("ロック").push(request.clone());
            let reply = self
                .replies
                .lock()
                .expect("ロック")
                .pop_front()
                .unwrap_or(Ok(String::new()));
            Box::pin(async move {
                match reply {
                    Ok(content) => Ok(Completion {
                        content,
                        finish_reason: Some(FinishReason::Stop),
                        usage: None,
                    }),
                    Err(()) => Err(LlmError::MissingContent),
                }
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

    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        aoi: CharacterId,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("一時ディレクトリ");
        let store = Store::connect(dir.path().join("personacore.sqlite"))
            .await
            .expect("DB を開けること");
        let aoi = store
            .create_character(
                &Handle::parse("aoi").expect("識別子"),
                &DisplayName::parse(DISPLAY_NAME).expect("表示名"),
                &Persona::parse("落ち着いた口調で話す。").expect("ペルソナ"),
                None,
                StatScale::DEFAULT,
            )
            .await
            .expect("キャラクターを作れること");
        Fixture {
            _dir: dir,
            store,
            aoi,
        }
    }

    /// 往復を1つ書いて、その往復の裏方の材料を返す。
    async fn turn(
        fixture: &Fixture,
        session: SessionId,
        user: &str,
        reply: &str,
        sub_context: SubContext,
    ) -> TurnJob {
        let message = |role, raw: &str| NewMessage {
            role,
            dialogue: None,
            narration: None,
            raw_content: raw.to_owned(),
        };
        let written = fixture
            .store
            .append_turn(
                session,
                &message(Role::User, user),
                &message(Role::Assistant, reply),
            )
            .await
            .expect("往復を書けること");
        job(
            fixture.aoi,
            session,
            written.reply,
            user,
            reply,
            sub_context,
        )
    }

    fn job(
        character_id: CharacterId,
        session_id: SessionId,
        reply_id: MessageId,
        user: &str,
        reply: &str,
        sub_context: SubContext,
    ) -> TurnJob {
        TurnJob {
            character_id,
            display_name: DisplayName::parse(DISPLAY_NAME).expect("表示名"),
            stat_scale: StatScale::DEFAULT,
            session_id,
            reply_id,
            user_message: user.to_owned(),
            character_message: reply.to_owned(),
            sub_context,
            drift_mode: ResolvedDriftMode::resolve(None, StatDriftMode::Off),
            axis_discovery_slots: AxisDiscoverySlots::NONE,
        }
    }

    async fn proposals(
        fixture: &Fixture,
    ) -> Vec<(String, AgreementKeeper, AgreementState, Option<SessionId>)> {
        fixture
            .store
            .agreements(fixture.aoi)
            .await
            .expect("読めること")
            .into_iter()
            .map(|agreement| {
                (
                    agreement.content.as_str().to_owned(),
                    agreement.keeper,
                    agreement.state,
                    agreement.source_session,
                )
            })
            .collect()
    }

    /// **1往復目で候補が生まれ、2往復目の後で確かめて、提案中として書く**（δ3 の流れ）。
    ///
    /// * 依頼は 検出（1往復目）→ 検出（2往復目）→ 確かめ の3本。**確かめは票がそろったので1本だけ。**
    /// * 確かめには印の往復と次の往復を添える（「# 会話（往復 1〜2）」）。
    /// * 提案は生んだ会話に縛られ、提案中で、**本文の読み（有効な一覧）に載らない。**
    /// * サンプラは裏方の温度と、応答の上限 320。
    #[test]
    fn a_candidate_from_one_turn_is_checked_after_the_next_and_saved_as_proposed() {
        runtime().block_on(async {
            let fixture = fixture().await;
            let session = fixture
                .store
                .create_session(fixture.aoi)
                .await
                .expect("会話");
            let sub = MockSub::replying(&[Ok(DETECTED), Ok(NOTHING), Ok(KEPT)]);
            let proposer = AgreementProposer::default();

            let first = turn(
                &fixture,
                session,
                FIRST_USER,
                FIRST_REPLY,
                SubContext::FALLBACK,
            )
            .await;
            run_agreement_proposals(&fixture.store, sub.as_ref(), &first, &proposer).await;
            assert_eq!(
                proposer.carried(fixture.aoi),
                1,
                "1往復目の候補を持ち越すこと"
            );
            assert!(proposals(&fixture).await.is_empty(), "確かめる前は書かない");

            let second = turn(
                &fixture,
                session,
                SECOND_USER,
                SECOND_REPLY,
                SubContext::FALLBACK,
            )
            .await;
            run_agreement_proposals(&fixture.store, sub.as_ref(), &second, &proposer).await;

            let users = sub.users();
            assert_eq!(users.len(), 3, "検出・検出・確かめの3本: {users:#?}");
            assert!(users[0].contains(DETECTION_MARKER) && users[0].contains("# 会話（往復 1）\n"));
            assert!(users[1].contains(DETECTION_MARKER) && users[1].contains("# 会話（往復 2）\n"));
            assert!(users[2].contains(CHECK_MARKER));
            assert!(users[2].contains("# 会話（往復 1〜2）\n"), "{}", users[2]);
            assert!(users[2].contains("1. [往復 1][利用者の発言から] 「もう黙って出かけない」"));
            for request in sub.requests() {
                assert_eq!(request.max_tokens, Some(320));
                assert_eq!(request.temperature, Some(SUB_LLM_TEMPERATURE));
            }

            assert_eq!(
                proposals(&fixture).await,
                vec![(
                    "利用者は黙って出かけない".to_owned(),
                    AgreementKeeper::User,
                    AgreementState::Proposed,
                    Some(session)
                )]
            );
            assert!(
                fixture
                    .store
                    .active_agreements(fixture.aoi)
                    .await
                    .expect("読める")
                    .is_empty(),
                "提案中は本文に載らないこと"
            );
            assert_eq!(proposer.carried(fixture.aoi), 0, "2往復目には候補が無い");
        });
    }

    /// **検出と1本目の確かめが割れたら、確かめをもう1本送る**（2of3）。2本目が残せば出す。
    #[test]
    fn a_split_vote_draws_a_second_check() {
        runtime().block_on(async {
            let fixture = fixture().await;
            let session = fixture
                .store
                .create_session(fixture.aoi)
                .await
                .expect("会話");
            let sub = MockSub::replying(&[Ok(DETECTED), Ok(NOTHING), Ok(DROPPED), Ok(KEPT)]);
            let proposer = AgreementProposer::default();
            for (user, reply) in [(FIRST_USER, FIRST_REPLY), (SECOND_USER, SECOND_REPLY)] {
                let job = turn(&fixture, session, user, reply, SubContext::FALLBACK).await;
                run_agreement_proposals(&fixture.store, sub.as_ref(), &job, &proposer).await;
            }
            let users = sub.users();
            assert_eq!(users.len(), 4, "{users:#?}");
            assert_eq!(users[2], users[3], "2本目は同じ依頼を送り直す");
            assert_eq!(proposals(&fixture).await.len(), 1);
        });
    }

    /// **「# 提案済み」は、そのキャラクターの提案中・却下・有効の行から、新しい順に作る**（終わったものは載せない）。
    /// **検出には載らない。** 提案済みと同じ取り決め（dedup2）は出さない。
    #[test]
    fn the_proposed_list_is_built_from_proposed_rejected_and_active_rows_newest_first() {
        runtime().block_on(async {
            let fixture = fixture().await;
            let session = fixture
                .store
                .create_session(fixture.aoi)
                .await
                .expect("会話");
            let write = |content: &str| NewAgreement {
                content: AgreementContent::parse(content).expect("中身"),
                kind: AgreementKind::Rule,
                keeper: AgreementKeeper::User,
            };
            // 書いた順: 有効 → 終わった → 提案中 → 却下。
            fixture
                .store
                .add_agreement(fixture.aoi, &write("有効な決まり"))
                .await
                .expect("書ける");
            let ended = fixture
                .store
                .add_agreement(fixture.aoi, &write("終わった決まり"))
                .await
                .expect("書ける");
            fixture
                .store
                .end_agreement(fixture.aoi, ended.id.get(), AgreementEnding::Fulfilled)
                .await
                .expect("終わらせられる");
            fixture
                .store
                .record_agreement_proposals(
                    fixture.aoi,
                    session,
                    &[write("提案中の決まり"), write("利用者は黙って出かけない")],
                )
                .await
                .expect("書ける");
            let rejected = fixture
                .store
                .agreements(fixture.aoi)
                .await
                .expect("読める")
                .into_iter()
                .find(|agreement| agreement.content.as_str() == "利用者は黙って出かけない")
                .expect("ある");
            fixture
                .store
                .reject_agreement(fixture.aoi, rejected.id.get())
                .await
                .expect("捨てられる");

            let sub = MockSub::replying(&[Ok(DETECTED), Ok(NOTHING), Ok(KEPT)]);
            let proposer = AgreementProposer::default();
            for (user, reply) in [(FIRST_USER, FIRST_REPLY), (SECOND_USER, SECOND_REPLY)] {
                let job = turn(&fixture, session, user, reply, SubContext::FALLBACK).await;
                run_agreement_proposals(&fixture.store, sub.as_ref(), &job, &proposer).await;
            }
            let users = sub.users();
            assert!(
                users[2].starts_with(
                    "# 提案済み\n\
                     1. [守る側: 利用者] 利用者は黙って出かけない\n\
                     2. [守る側: 利用者] 提案中の決まり\n\
                     3. [守る側: 利用者] 有効な決まり\n\n"
                ),
                "{}",
                users[2]
            );
            assert!(!users[2].contains("終わった決まり"));
            for detection in &users[..2] {
                assert!(!detection.contains("# 提案済み"), "{detection}");
                assert!(!detection.contains("有効な決まり"), "{detection}");
            }
            // **捨てたものと同じ取り決めは、二度出さない**（dedup2 —— 却下の行も相手）。
            assert_eq!(
                proposals(&fixture)
                    .await
                    .iter()
                    .filter(|(content, ..)| content == "利用者は黙って出かけない")
                    .count(),
                1
            );
        });
    }

    /// **本番の dedup は dedup2**（ADR-0051 の決定 —— δ3 に dedup2 を足した形）。
    ///
    /// 捨てた提案「利用者は声を出してはいけない」と、新しい候補「利用者は許しが出るまで部屋を出てはいけない」は、
    /// δ3 で測った dedup（文末の「〜してはいけない」で重なる）では同じと読まれて出ない。dedup2 では別の取り決めなので出る。
    /// 期待値は測定の道具（`rp2.same_content` → 真 / `rp4.same_content2` → 偽）に同じ2文を通して確かめた。
    #[test]
    fn the_production_dedup_keeps_a_different_rule_with_the_same_ending() {
        runtime().block_on(async {
            let fixture = fixture().await;
            let session = fixture.store.create_session(fixture.aoi).await.expect("会話");
            let earlier = NewAgreement {
                content: AgreementContent::parse("利用者は声を出してはいけない").expect("中身"),
                kind: AgreementKind::Rule,
                keeper: AgreementKeeper::User,
            };
            fixture
                .store
                .record_agreement_proposals(fixture.aoi, session, &[earlier])
                .await
                .expect("書ける");
            let rejected = fixture.store.agreements(fixture.aoi).await.expect("読める")[0].id;
            fixture
                .store
                .reject_agreement(fixture.aoi, rejected.get())
                .await
                .expect("捨てられる");

            let sub = MockSub::replying(&[
                Ok("抜｜「許しが出るまで部屋を出ない」｜取り決め｜利用者｜利用者は許しが出るまで部屋を出てはいけない"),
                Ok(NOTHING),
                Ok("点検｜1｜部屋を出る｜取り決め｜利用者｜残す｜利用者は許しが出るまで部屋を出てはいけない"),
            ]);
            let proposer = AgreementProposer::default();
            for (user, reply) in [
                ("わかった、許しが出るまで部屋を出ないよ。", FIRST_REPLY),
                (SECOND_USER, SECOND_REPLY),
            ] {
                let job = turn(&fixture, session, user, reply, SubContext::FALLBACK).await;
                run_agreement_proposals(&fixture.store, sub.as_ref(), &job, &proposer).await;
            }
            assert_eq!(
                proposals(&fixture)
                    .await
                    .into_iter()
                    .map(|(content, _, state, _)| (content, state))
                    .collect::<Vec<_>>(),
                vec![
                    ("利用者は声を出してはいけない".to_owned(), AgreementState::Rejected),
                    (
                        "利用者は許しが出るまで部屋を出てはいけない".to_owned(),
                        AgreementState::Proposed
                    ),
                ]
            );
        });
    }

    /// **依頼の大きさが上限を超える往復は飛ばす**（検出を送らない。候補も持ち越さない）。飛ばしたことは記録する。
    #[test]
    fn a_detection_over_the_limit_is_not_sent() {
        let output = captured_under_default_filter(|| {
            runtime().block_on(async {
                let fixture = fixture().await;
                let session = fixture
                    .store
                    .create_session(fixture.aoi)
                    .await
                    .expect("会話");
                let sub = MockSub::replying(&[Ok(DETECTED)]);
                let proposer = AgreementProposer::default();
                // 依頼の上限 1000（窓 2000 の半分）。検出の文面だけで 2451 バイトあるので収まらない。
                let small = SubContext::new(2_000).expect("窓になる値");
                let job = turn(&fixture, session, FIRST_USER, FIRST_REPLY, small).await;
                run_agreement_proposals(&fixture.store, sub.as_ref(), &job, &proposer).await;
                assert!(sub.requests().is_empty(), "上限を超える依頼は送らないこと");
                assert_eq!(proposer.carried(fixture.aoi), 0);
            });
        });
        let line = line_containing(&output, SKIPPED_MESSAGE);
        assert!(
            has_field(line, "agreement_skipped=detection_too_long"),
            "{line}"
        );
        assert!(has_field(line, "agreement_limit_tokens=1000"), "{line}");
    }

    /// **確かめが上限を超えるなら、提案済みを減らしてから送る。それでも超えれば飛ばす**（候補は捨てる）。
    #[test]
    fn a_check_over_the_limit_is_skipped_and_the_candidates_are_dropped() {
        runtime().block_on(async {
            let fixture = fixture().await;
            let session = fixture
                .store
                .create_session(fixture.aoi)
                .await
                .expect("会話");
            let sub = MockSub::replying(&[Ok(DETECTED), Ok(NOTHING)]);
            let proposer = AgreementProposer::default();
            // 検出（約 2.7k）は入るが、確かめ（指示だけで 2.7k ＋ 会話）は入らない上限。
            let limit_of = |messages: &[ChatMessage]| request_tokens(messages);
            let detection = detection_request(
                &DisplayName::parse(DISPLAY_NAME).expect("表示名"),
                &ProposalTurn {
                    number: 1,
                    user: FIRST_USER.to_owned(),
                    character: FIRST_REPLY.to_owned(),
                },
            );
            let tokens = limit_of(&detection) + 20;
            let window = SubContext::new(tokens * 2).expect("窓になる値");
            let first = turn(&fixture, session, FIRST_USER, FIRST_REPLY, window).await;
            run_agreement_proposals(&fixture.store, sub.as_ref(), &first, &proposer).await;
            let second = turn(&fixture, session, SECOND_USER, SECOND_REPLY, window).await;
            run_agreement_proposals(&fixture.store, sub.as_ref(), &second, &proposer).await;
            assert_eq!(sub.users().len(), 2, "確かめは送らないこと");
            assert!(proposals(&fixture).await.is_empty());
            assert_eq!(proposer.carried(fixture.aoi), 0);
        });
    }

    /// **崩れた出力・上流の失敗では、候補 0 件で終わり、panic しない。** 次の往復の仕事は続く。
    #[test]
    fn broken_or_failed_replies_leave_no_candidate_and_do_not_panic() {
        runtime().block_on(async {
            let fixture = fixture().await;
            let session = fixture
                .store
                .create_session(fixture.aoi)
                .await
                .expect("会話");
            let sub = MockSub::replying(&[
                Ok("抜｜｜\n点検｜x\n\u{0}｜「」"),
                Err(()),
                Ok(DETECTED),
                Ok(NOTHING),
                Err(()),
            ]);
            let proposer = AgreementProposer::default();
            for (user, reply) in [
                (FIRST_USER, FIRST_REPLY),
                (SECOND_USER, SECOND_REPLY),
                (FIRST_USER, FIRST_REPLY),
                (SECOND_USER, SECOND_REPLY),
            ] {
                let job = turn(&fixture, session, user, reply, SubContext::FALLBACK).await;
                run_agreement_proposals(&fixture.store, sub.as_ref(), &job, &proposer).await;
            }
            // 4往復目の確かめが失敗した —— 書かず、候補も捨てる。
            assert_eq!(sub.users().len(), 5);
            assert!(proposals(&fixture).await.is_empty());
            assert_eq!(proposer.carried(fixture.aoi), 0);
        });
    }

    /// **次の往復が別の会話なら、印の往復だけを添えて確かめ、提案は生んだ会話に縛る。**
    #[test]
    fn a_candidate_followed_by_another_conversation_is_checked_alone() {
        runtime().block_on(async {
            let fixture = fixture().await;
            let first_session = fixture
                .store
                .create_session(fixture.aoi)
                .await
                .expect("会話");
            let second_session = fixture
                .store
                .create_session(fixture.aoi)
                .await
                .expect("会話");
            let sub = MockSub::replying(&[Ok(DETECTED), Ok(NOTHING), Ok(KEPT)]);
            let proposer = AgreementProposer::default();
            let first = turn(
                &fixture,
                first_session,
                FIRST_USER,
                FIRST_REPLY,
                SubContext::FALLBACK,
            )
            .await;
            run_agreement_proposals(&fixture.store, sub.as_ref(), &first, &proposer).await;
            let other = turn(
                &fixture,
                second_session,
                SECOND_USER,
                SECOND_REPLY,
                SubContext::FALLBACK,
            )
            .await;
            run_agreement_proposals(&fixture.store, sub.as_ref(), &other, &proposer).await;

            let users = sub.users();
            assert!(users[2].contains("# 会話（往復 1）\n"), "{}", users[2]);
            assert!(
                !users[2].contains(SECOND_USER),
                "別の会話の往復を添えないこと"
            );
            assert_eq!(
                proposals(&fixture).await,
                vec![(
                    "利用者は黙って出かけない".to_owned(),
                    AgreementKeeper::User,
                    AgreementState::Proposed,
                    Some(first_session)
                )]
            );
        });
    }

    /// **2本目の確かめが失敗しても、1本目で決まった候補は捨てない**（作業単位 A-12c-1 のレビューの提案2）。
    ///
    /// 候補は2つ。1つ目は検出と1本目がそろう（決まり）、2つ目は割れる（2本目が要る）。2本目が失敗すると、
    /// 1つ目だけが出て、2つ目は多数が無いので出ない。
    #[test]
    fn a_failed_second_check_keeps_what_the_first_one_decided() {
        runtime().block_on(async {
            let fixture = fixture().await;
            let session = fixture
                .store
                .create_session(fixture.aoi)
                .await
                .expect("会話");
            let sub = MockSub::replying(&[
                Ok(
                    "抜｜「もう黙って出かけない」｜取り決め｜利用者｜利用者は黙って出かけない\n\
                    抜｜「夜は台所に入らない」｜取り決め｜利用者｜利用者は夜に台所へ入らない",
                ),
                Ok(NOTHING),
                Ok(
                    "点検｜1｜黙って出かける｜取り決め｜利用者｜残す｜利用者は黙って出かけない\n\
                    点検｜2｜台所に入る｜その場｜利用者｜落とす｜-",
                ),
                Err(()),
            ]);
            let proposer = AgreementProposer::default();
            for (user, reply) in [
                ("もう黙って出かけない。夜は台所に入らないよ。", FIRST_REPLY),
                (SECOND_USER, SECOND_REPLY),
            ] {
                let job = turn(&fixture, session, user, reply, SubContext::FALLBACK).await;
                run_agreement_proposals(&fixture.store, sub.as_ref(), &job, &proposer).await;
            }
            assert_eq!(sub.users().len(), 4, "2本目は送って失敗した");
            assert_eq!(
                proposals(&fixture)
                    .await
                    .into_iter()
                    .map(|(content, ..)| content)
                    .collect::<Vec<_>>(),
                vec!["利用者は黙って出かけない".to_owned()]
            );
        });
    }

    /// **横取りしたことは、既定のフィルタで見える `info` に残る**（`agreement_skipped=preempted_by_next_turn`）。
    #[test]
    fn a_preemption_is_recorded() {
        let output = captured_under_default_filter(|| {
            runtime().block_on(async {
                let fixture = fixture().await;
                let session = fixture
                    .store
                    .create_session(fixture.aoi)
                    .await
                    .expect("会話");
                let next = turn(
                    &fixture,
                    session,
                    SECOND_USER,
                    SECOND_REPLY,
                    SubContext::FALLBACK,
                )
                .await;
                record_preempted(&next);
            });
        });
        let line = line_containing(&output, SKIPPED_MESSAGE);
        assert!(
            has_field(line, "agreement_skipped=preempted_by_next_turn"),
            "{line}"
        );
        assert!(line.contains("INFO"), "{line}");
    }

    /// **番号の印は、自分の番号のときだけ消す**（止めた1本が後から出ても、次の1本の印を消さない）。
    #[test]
    fn leaving_the_stage_never_clears_another_runs_mark() {
        let proposer = AgreementProposer::default();
        assert!(!proposer.is_in_stage(0), "0 は誰でもない");
        proposer.enter_stage(7);
        assert!(proposer.is_in_stage(7));
        assert!(!proposer.is_in_stage(8));
        proposer.leave_stage(6);
        assert!(proposer.is_in_stage(7), "別の番号では消えない");
        proposer.leave_stage(7);
        assert!(!proposer.is_in_stage(7));
    }

    /// **読めなかったので飛ばした回には、見積もりの欄を出さない**（測っていない / レビューの提案3）。
    /// 上限を超えた回には出す（`a_detection_over_the_limit_is_not_sent`）。
    #[test]
    fn a_skip_before_any_request_was_built_names_no_prompt_tokens() {
        let output = captured_under_default_filter(|| {
            runtime().block_on(async {
                let fixture = fixture().await;
                let session = fixture
                    .store
                    .create_session(fixture.aoi)
                    .await
                    .expect("会話");
                let job = turn(
                    &fixture,
                    session,
                    FIRST_USER,
                    FIRST_REPLY,
                    SubContext::FALLBACK,
                )
                .await;
                fixture.store.close().await;
                let sub = MockSub::replying(&[Ok(DETECTED)]);
                run_agreement_proposals(
                    &fixture.store,
                    sub.as_ref(),
                    &job,
                    &AgreementProposer::default(),
                )
                .await;
                assert!(sub.requests().is_empty(), "番号を読めなければ送らない");
            });
        });
        let line = line_containing(&output, SKIPPED_MESSAGE);
        assert!(
            has_field(line, "agreement_skipped=turn_unreadable"),
            "{line}"
        );
        assert!(!line.contains("agreement_prompt_tokens"), "{line}");
        assert!(line.contains("WARN"), "{line}");
    }

    /// **同じ会話でも、間の往復が飛んでいたら（前の裏方が走っていて `Busy`）、印の往復だけを添える**
    /// （添えるのは印の往復とその次の往復だけ —— `rp3.py` の `session_turns(q, q + 1)`。飛んだ往復の本文は持っていない）。
    #[test]
    fn a_candidate_followed_by_a_skipped_turn_is_checked_alone() {
        runtime().block_on(async {
            let fixture = fixture().await;
            let session = fixture
                .store
                .create_session(fixture.aoi)
                .await
                .expect("会話");
            let sub = MockSub::replying(&[Ok(DETECTED), Ok(NOTHING), Ok(KEPT)]);
            let proposer = AgreementProposer::default();
            let first = turn(
                &fixture,
                session,
                FIRST_USER,
                FIRST_REPLY,
                SubContext::FALLBACK,
            )
            .await;
            run_agreement_proposals(&fixture.store, sub.as_ref(), &first, &proposer).await;
            // 2往復目は書かれるが、裏方は走らない（前の裏方がまだ走っていた形）。
            let _skipped = turn(
                &fixture,
                session,
                "飛んだ往復",
                "飛んだ応答",
                SubContext::FALLBACK,
            )
            .await;
            let third = turn(
                &fixture,
                session,
                SECOND_USER,
                SECOND_REPLY,
                SubContext::FALLBACK,
            )
            .await;
            run_agreement_proposals(&fixture.store, sub.as_ref(), &third, &proposer).await;

            let users = sub.users();
            assert!(users[1].contains("# 会話（往復 3）\n"), "{}", users[1]);
            assert!(users[2].contains("# 会話（往復 1）\n"), "{}", users[2]);
            assert!(
                !users[2].contains(SECOND_USER),
                "飛んだ後の往復を印の次の往復として添えないこと"
            );
            assert_eq!(proposals(&fixture).await.len(), 1);
        });
    }

    /// **提案を出したことと、落とした件数は `info` で残る**（既定のフィルタで見える）。本文は載せない。
    #[test]
    fn the_check_records_what_it_proposed_and_dropped() {
        let output = captured_under_default_filter(|| {
            runtime().block_on(async {
                let fixture = fixture().await;
                let session = fixture
                    .store
                    .create_session(fixture.aoi)
                    .await
                    .expect("会話");
                let sub = MockSub::replying(&[Ok(DETECTED), Ok(NOTHING), Ok(KEPT)]);
                let proposer = AgreementProposer::default();
                for (user, reply) in [(FIRST_USER, FIRST_REPLY), (SECOND_USER, SECOND_REPLY)] {
                    let job = turn(&fixture, session, user, reply, SubContext::FALLBACK).await;
                    run_agreement_proposals(&fixture.store, sub.as_ref(), &job, &proposer).await;
                }
            });
        });
        let line = line_containing(&output, CHECKED_MESSAGE);
        for field in [
            "agreement_checked=1",
            "agreement_draws=1",
            "agreement_proposed=1",
            "agreement_deduped=0",
            "agreement_subject_mismatch=0",
            "agreement_not_kept=0",
            "agreement_mind_flips=0",
            "agreement_unclear_keeper=0",
            "agreement_invalid_content=0",
        ] {
            assert!(has_field(line, field), "{field}: {line}");
        }
        assert!(line.contains("INFO"), "{line}");
        assert!(
            !output.contains("黙って出かけない"),
            "本文をログに載せないこと: {output}"
        );
        let detected = line_containing(&output, DETECTED_MESSAGE);
        assert!(has_field(detected, "agreement_candidates=1"), "{detected}");
    }
}
