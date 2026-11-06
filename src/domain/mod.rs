//! ドメイン型と不変条件。
//!
//! **I/O を持たない純粋関数のみ**を置く（CLAUDE.md 3節）。
//! 値は「検証を通った状態でしか作れない」形にする。ADR-0006 の `ValidTag` と同じ発想で、
//! 不正な値を「防ぐ」のではなく**型で表現できなくする**。
//!
//! # 行の型（`Character` / `Memory` / `Message`）がここにある理由
//!
//! これらは DB の1行を表すが、**中身は検証済みのドメイン型と `String` と日時だけ**で、
//! sqlx の derive も serde も持たない。`store` に置くと、純粋モジュール
//! （`prompt` / `parse`）がそれらを読むために **sqlx を抱える I/O モジュールへ依存する**
//! ことになる。同じ理由で `ChatMessage` / `ChatRole` もここに置く（`llm` は reqwest を抱える）。
//! `store` / `llm` は `pub use` で再公開するので、呼び出し側の名前は変わらない。

mod agreement;
mod attribute;
mod axis_candidate;
mod axis_discovery;
mod axis_trial;
mod character;
mod chat;
mod drift;
mod drift_ledger;
mod memory;
mod message;
mod selector;
mod session;
mod stat;
mod text;
mod thinking;

pub use agreement::{
    Agreement, AgreementContent, AgreementContentError, AgreementEdits, AgreementEnding,
    AgreementId, AgreementKeeper, AgreementKind, AgreementState, MAX_ACTIVE_AGREEMENTS,
    NewAgreement, admits_another_active_agreement,
};
pub use attribute::{
    AttributeChange, AttributeKey, AttributeKeyError, AttributeSet, AttributeSetError,
    AttributeValue, AttributeValueError, CharacterAttribute, attribute_changes,
};
pub use axis_candidate::{
    AxisCandidateId, AxisCandidateRecord, AxisCandidateState, AxisDiscoverySlots,
    AxisDiscoverySlotsError, AxisTrialFailure, AxisTrialTally, AxisTrialVote,
    current_axis_candidate_state,
};
pub use axis_discovery::{
    AXIS_CANDIDATE_PLACEHOLDER, AxisDiscoveryDue, AxisDiscoveryMemories,
    AxisDiscoveryMemoriesError, AxisDiscoveryMiss, AxisDiscoveryOutcome, AxisNameClash,
    axis_discovery_due, axis_name_clash, memories_since,
};
pub use axis_trial::{
    AxisTrialRule, AxisTrialRuleError, AxisTrialVerdict, candidate_to_score, judge_axis_trial,
    trial_vote,
};
pub use character::{
    Character, CharacterColumn, CharacterEntry, CharacterId, CharacterUpdate, DisplayName,
    DisplayNameError, FieldUpdate, Handle, HandleError, OutputFormat, OutputFormatError, Persona,
    PersonaError, UnreadableCharacter,
};
pub use chat::{ChatMessage, ChatRole};
pub use drift::{
    DRIFT_FLOOR, DRIFT_SCORED_LABEL, DriftApply, DriftChange, DriftDelta, DriftDiscardReason,
    DriftForm, DriftModeSource, DriftNote, DriftOutcome, DriftProposal, DriftScore, DriftScoring,
    DriftSuggestion, DriftVote, NO_CHANGE_SIGN, ResolvedDriftMode, StatDriftMode,
    StatDriftModeError, apply_drift,
};
pub use drift_ledger::{
    DEFAULT_DRIFT_DECAY, DEFAULT_DRIFT_SLOPE, DEFAULT_DRIFT_STEP_COST, DriftAxisLedger,
    DriftLedger, DriftLedgerError, DriftTuning, DriftTuningError, DriftTurnEffect, MAX_DRIFT_SLOPE,
};
pub use memory::{
    CategoryError, ContentError, Memory, MemoryCategory, MemoryColumn, MemoryContent, MemoryEntry,
    MemoryId, NewMemory, Subject, SupersededMemory, UnreadableMemory,
};
pub use message::{Message, MessageId, NewMessage, Role};
pub use selector::{ModelSelector, SelectorError, SessionRef};
pub use session::SessionId;
pub use stat::{AxisError, ScaleError, StatAlloc, StatAxis, StatScale, StatValue, ValueError};
// **単体テスト用の尺度フィクスチャ。** `#[cfg(test)]` なので `cargo build` には現れない。
// 理由は `stat::fixture_scale` の doc（統合テスト側は `tests/common/mod.rs`）。
#[cfg(test)]
pub(crate) use stat::fixture_scale;
pub use text::{LINE_BREAKS, TRIMMABLE};
pub use thinking::{StrippedThinking, strip_leaked_thinking};
