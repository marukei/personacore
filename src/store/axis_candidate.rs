//! **軸の候補**の読み書き（[ADR-0046](../../docs/adr/0046-drift-proposes-new-axes.md) §決定1・§決定6 /
//! 作業単位 A-9a）。
//!
//! この層は**薄く保つ**。状態の規則（どこからどこへ動けるか・いまの状態の導き方・空いた枠の数え方）は
//! `domain::axis_candidate` の純粋関数にあり、ここは「読んで、規則に通して、追記する」だけを担う。
//!
//! # 表が3枚ある（ADR-0046 §決定6）
//!
//! | 表 | 何か |
//! |---|---|
//! | 候補 | 候補1件につき1行（名前・出どころの記憶・時刻）。**状態を持たない** |
//! | 移り変わり | 状態が動くたびに1行。**いまの状態は、候補ごとの最後の行** |
//! | 試用の票 | 試用中のターンごとに1行。**変容の票の表と混ぜない** |
//!
//! **3枚とも追記のみ。** 更新も削除もしない（見張り番はこのファイルの走査）。
//! **テーブル名をこのファイルの散文に綴らないこと**——走査が自分自身を落とす
//! （`store::attribute` の履歴の表と同じ作法。だからこの doc は「候補の表」「移り変わりの表」と呼ぶ）。
//!
//! # **書き込みの経路は [`Store::record_axis_candidate`] ひとつ**（ADR-0028 §決定5 と同じ形）
//!
//! 3枚への INSERT はこのファイルにだけあり、どれもあの関数の中から走る
//! （見張り番は `no_file_but_the_one_writer_appends_to_the_axis_candidate_tables`）。
//!
//! > **⚠ 走査が守っているのは「3枚へ書く文の綴りが、このファイルにだけある」まで。**
//! > **「1つの関数に閉じている」ではない**——`store::attribute` の同じ走査と同じ限界
//! > （同じファイルに、既存の定数を使い回す2つ目の `pub async fn` を足しても走査は通る）。

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::{Store, StoreError};
use crate::domain::{
    AxisCandidateId, AxisCandidateRecord, AxisCandidateState, AxisDiscoveryMiss,
    AxisDiscoveryOutcome, AxisDiscoverySlots, AxisTrialFailure, AxisTrialTally, AxisTrialVote,
    CharacterId, MemoryId, SessionId, StatAxis, current_axis_candidate_state,
};

/// **読みから始まって書きに昇格するトランザクションの開始文**（ADR-0028 §決定5 と同じ理由）。
///
/// 書き手は**いまの状態を読んでから書く**（空いた枠を数える・遷移を検査する・試用中かを見る）。
/// 素の `BEGIN`（DEFERRED）だと最初の読みでスナップショットが固定され、続く書きへの昇格が
/// `SQLITE_BUSY_SNAPSHOT` になる——**`busy_timeout` の対象外で、待たずに即座に失敗する**
/// （実測と壊れ方は `store::character` の同名の定数の doc）。
///
/// **`store::character` / `store::attribute` の定数と共有しない**（中身は同じだが、
/// 「なぜここで要るのか」は関数ごとに別。共有すると片方の理由が消えたときに一緒に外れる）。
///
/// 見張り番は `tests/store_axis_candidate.rs` の
/// `two_proposals_never_overfill_one_slot`（負けた側の理由まで照合する）と
/// `proposals_for_two_characters_never_lose_their_snapshot`（無関係な書き込み同士が両方通る）。
const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

/// そのキャラクターの枠の数（書き手がトランザクションの中で読む）。
const SELECT_SLOTS: &str = "SELECT axis_discovery_slots FROM character WHERE id = ?";

/// **候補を1件作る文。** 名前は `StatAxis`、出どころは同じキャラクターの記憶（複合外部キー）。
const INSERT_CANDIDATE: &str = "INSERT INTO stat_axis_candidate \
     (character_id, name, source_memory_id) VALUES (?, ?, ?)";

/// **移り変わりを1件足す文。** 理由は不合格のときだけ（表の CHECK が組を縛る）。
const INSERT_TRANSITION: &str = "INSERT INTO stat_axis_candidate_transition \
     (candidate_id, state, reason) VALUES (?, ?, ?)";

/// **試用の票を1件足す文。**
const INSERT_TRIAL_VOTE: &str = "INSERT INTO stat_axis_trial_vote \
     (candidate_id, session_id, vote) VALUES (?, ?, ?)";

/// **試みを1回追記する文**（作業単位 A-9b / Issue origin#42 の申し送り1）。
///
/// 書くのは [`insert_attempt`] だけで、呼ぶのは2箇所——候補を採った回（`Propose` の腕・
/// 候補と同じトランザクション）と、採らずに終わった回（[`Store::record_axis_discovery_miss`]）。
/// **結末の綴りは `AxisDiscoveryOutcome::label`**（SQL に直書きしない）。
const INSERT_ATTEMPT: &str = "INSERT INTO stat_axis_discovery_attempt \
     (character_id, through_memory_id, outcome) VALUES (?, ?, ?)";

/// **そのキャラクターの試みの目印**（これまでの試みで数え終えた記憶のうち最大の id）。
///
/// **最後の行ではなく最大値を読む**——目印は後退しない。候補を採った回の目印は
/// 候補の出どころなので、呼び出し側が古い記憶を出どころに渡しても、次の候補づくりが
/// 数え終えた記憶を数え直すことはない。
const SELECT_ATTEMPT_MARK: &str = "SELECT max(through_memory_id) FROM stat_axis_discovery_attempt \
     WHERE character_id = ?";

/// **その候補が、そのキャラクターのものか**（書き手が遷移と票の前に見る）。
///
/// **持ち主で縛る。** 縛らないと、他のキャラクターの候補を id だけで進められる。
const SELECT_OWNED_CANDIDATE: &str =
    "SELECT id FROM stat_axis_candidate WHERE id = ? AND character_id = ?";

/// **その会話が、候補の持ち主の会話か**（書き手が試用の票の前に見る / 作業単位 A-9c /
/// Issue origin#43 の申し送り2）。
///
/// 試用の票の表の外部キーは会話が**在る**ことしか見ない。縛らないと、
/// 別のキャラクターの会話のターンに入った票として書ける。
const SELECT_OWNED_SESSION: &str = "SELECT id FROM session WHERE id = ? AND character_id = ?";

/// **1件の候補の移り変わりを古い順に。** 並びは `id`（追記の順）——
/// `created_at` は同じミリ秒に並びうる（候補を作るときは同じトランザクションで2行入る）。
const SELECT_CANDIDATE_TRANSITIONS: &str = "SELECT state, reason, created_at \
     FROM stat_axis_candidate_transition WHERE candidate_id = ? ORDER BY id";

/// **そのキャラクターの全候補の移り変わり**（候補ごと・古い順）。
///
/// # 一覧の上限（`LIMIT`）で絞らない
///
/// 候補の一覧（[`SELECT_CANDIDATES`]）と別の文で読むので、ここを同じ `LIMIT` の副問い合わせで
/// 絞ると、**2つの文の間に候補が1件増えただけで**、一覧に載った最も古い候補の移り変わりが
/// 読めなくなる（その候補は「移り変わりの無い壊れた候補」に見えて一覧が止まる）。
/// 全部読んで、一覧に無い候補の行は捨てる。**候補は1件につき高々3行**なので量は候補数に比例する。
///
/// `JOIN` で書かない——走査の許可リストは `FROM` の直後だけを読みとして通す（このファイルの走査）。
const SELECT_CHARACTER_TRANSITIONS: &str = "SELECT candidate_id, state, reason, created_at \
     FROM stat_axis_candidate_transition \
     WHERE candidate_id IN (SELECT id FROM stat_axis_candidate WHERE character_id = ?) \
     ORDER BY candidate_id, id";

/// **そのキャラクターの候補を新しい順に最大 N 件。** 第2キーの `id` は同着の並びのため。
const SELECT_CANDIDATES: &str = "SELECT id, name, source_memory_id, created_at \
     FROM stat_axis_candidate WHERE character_id = ? \
     ORDER BY created_at DESC, id DESC LIMIT ?";

/// **そのキャラクターの候補の総数。上限で切らない**（返した件数とは別の値）。
const COUNT_CANDIDATES: &str =
    "SELECT count(*) AS candidates FROM stat_axis_candidate WHERE character_id = ?";

/// **そのキャラクターの候補ごと・種類ごとの試用の票の件数。**
///
/// 移り変わりと同じ理由で、一覧の上限では絞らない（1候補あたり高々3行）。
const SELECT_TALLIES: &str = "SELECT candidate_id, vote, count(*) AS votes \
     FROM stat_axis_trial_vote \
     WHERE candidate_id IN (SELECT id FROM stat_axis_candidate WHERE character_id = ?) \
     GROUP BY candidate_id, vote";

/// **移り変わり1件**（`store` の外へ出る形）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisCandidateTransition {
    /// 入った状態（不合格なら理由つき）。
    state: AxisCandidateState,
    /// いつ入ったか。
    created_at: DateTime<Utc>,
}

impl AxisCandidateTransition {
    /// 入った状態。
    #[must_use]
    pub const fn state(&self) -> AxisCandidateState {
        self.state
    }

    /// いつ入ったか（UTC）。
    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
}

/// **軸の候補1件**（`store` の外へ出る形 / ADR-0046 §決定6）。
///
/// **いまの状態は保存されていない。** [`Self::state`] は移り変わりの列から導いたもの
/// （`domain::current_axis_candidate_state`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AxisCandidate {
    id: AxisCandidateId,
    name: StatAxis,
    source_memory: MemoryId,
    created_at: DateTime<Utc>,
    transitions: Vec<AxisCandidateTransition>,
    state: AxisCandidateState,
    tally: AxisTrialTally,
}

impl AxisCandidate {
    /// 主キー。
    #[must_use]
    pub const fn id(&self) -> AxisCandidateId {
        self.id
    }

    /// 候補の名前（`StatAxis` の検証を通っている）。
    #[must_use]
    pub const fn name(&self) -> &StatAxis {
        &self.name
    }

    /// 出どころの記憶（ADR-0046 §決定2）。
    #[must_use]
    pub const fn source_memory(&self) -> MemoryId {
        self.source_memory
    }

    /// 候補が生まれた時刻（UTC）。
    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    /// 移り変わり（古い順）。**先頭は必ず試用。**
    #[must_use]
    pub fn transitions(&self) -> &[AxisCandidateTransition] {
        &self.transitions
    }

    /// **いまの状態**（移り変わりの列から導いたもの）。
    #[must_use]
    pub const fn state(&self) -> AxisCandidateState {
        self.state
    }

    /// 試用の票の内訳（上げ・下げ・`0`）。
    #[must_use]
    pub const fn tally(&self) -> AxisTrialTally {
        self.tally
    }
}

impl Store {
    /// **軸の候補の3枚の表へ書く、唯一の経路**（ADR-0046 §決定6 / ADR-0028 §決定5 と同じ形）。
    ///
    /// 1つのトランザクション（`BEGIN IMMEDIATE`）で**いまの状態を読んでから**1件を追記する。
    /// **行を書き換えない。** 状態は移り変わりの行を足して進める。
    ///
    /// | 腕 | 読むもの | 拒む条件 | 書くもの |
    /// |---|---|---|---|
    /// | `Propose` | 枠の数・全候補のいまの状態 | 空いた枠が無い → [`StoreError::AxisDiscoverySlotsFull`] | 候補1行 ＋ 試用の移り変わり1行 |
    /// | `Advance` | その候補の移り変わり | 他のキャラクターの候補 → [`StoreError::UnknownAxisCandidate`] / 規則にない遷移 → [`StoreError::AxisCandidateTransitionRefused`] | 移り変わり1行 |
    /// | `TrialVote` | その候補の移り変わり・会話の持ち主 | 他のキャラクターの候補 → [`StoreError::UnknownAxisCandidate`] / 試用中でない → [`StoreError::AxisCandidateNotOnTrial`] / **候補の持ち主の会話でない** → [`StoreError::SessionOfAnotherCharacter`]（A-9c） | 試用の票1行 |
    ///
    /// 返すのは触った候補の id（`Propose` なら新しく作った候補）。
    ///
    /// # 名前の重複は表が止める
    ///
    /// 同じキャラクターが同じ名前を二度使うと UNIQUE 違反（`StoreError::Query` /
    /// [`StoreError::is_unique_violation`] が真）。**完全一致**で、正規化しない（ADR-0046 §決定2）。
    /// 固定の軸の名前との照合は候補を出す側（A-9b）の仕事。
    ///
    /// # ⚠ 採用はまだ `stat_alloc` に何も足さない
    ///
    /// ADR-0046 §決定6 は「採用した時点で `stat_alloc` に値 0 の行として足す（書き込み経路は1つ）」
    /// と決めている。**それは第2段（採用の作業単位）がこの関数の `Advance` の腕に足す。**
    /// この作業単位の時点では採用も却下も記録するだけで、呼ぶ本番コードも無い。
    ///
    /// **トランザクションの中で外部通信を待たない**（CLAUDE.md 絶対制約）。この関数は SQL しか呼ばない。
    /// **LLM の呼び出しはトランザクションの外で済ませ、結果を上限つきの型に通してから渡すこと。**
    ///
    /// # **テスト専用**（作業単位 A-10）
    ///
    /// 会話で縛らない。本番の書き手（軸を見いだす・試用）は [`Self::record_axis_candidate_in_session`]。
    #[cfg(any(test, feature = "testing"))]
    pub async fn record_axis_candidate(
        &self,
        character_id: CharacterId,
        record: &AxisCandidateRecord,
    ) -> Result<AxisCandidateId, StoreError> {
        self.write_axis_candidate(character_id, None, record).await
    }

    /// **往復の会話で縛って**軸の候補の表へ書く（本番の書き手 / 作業単位 A-10 / ADR-0049 §決定3）。
    ///
    /// [`Self::record_axis_candidate`] の doc の約束はそのまま。加えて、`BEGIN IMMEDIATE` で
    /// 書きロックを取った**直後に** `session_id` の会話が今も在り `character_id` のものかを確かめ、
    /// 無ければ1行も書かずに [`StoreError::TurnSessionGone`]。
    ///
    /// **確かめてから書くが、隙は無い**（書きロックを握ったまま確かめ、書き、commit する。
    /// 消す側の `DELETE` も書きロックを要る）。
    ///
    /// # なぜ要るのか
    ///
    /// `Propose` の出どころの記憶（`memory.id`）も `character.id` も使い回されうる。
    /// 縛らないと、完全に消したキャラクターの候補が、同じ id で作り直したキャラクターに入る
    /// （出どころの複合外部キーは「その id の組が在るか」しか見ない）。
    /// `Advance` / `TrialVote` の候補の id は AUTOINCREMENT なので、消えた候補は
    /// `UnknownAxisCandidate` で止まるが、**入口を1つにするため全部の腕で確かめる。**
    pub async fn record_axis_candidate_in_session(
        &self,
        character_id: CharacterId,
        session_id: SessionId,
        record: &AxisCandidateRecord,
    ) -> Result<AxisCandidateId, StoreError> {
        self.write_axis_candidate(character_id, Some(session_id), record)
            .await
    }

    /// 軸の候補の表へ書く本体（`origin` が `None` なら会話で縛らない）。
    async fn write_axis_candidate(
        &self,
        character_id: CharacterId,
        origin: Option<SessionId>,
        record: &AxisCandidateRecord,
    ) -> Result<AxisCandidateId, StoreError> {
        let mut tx = self
            .pool
            .begin_with(BEGIN_IMMEDIATE)
            .await
            .map_err(StoreError::Query)?;
        if let Some(origin) = origin {
            ensure_turn_session(&mut tx, character_id, origin).await?;
        }

        let touched = match record {
            AxisCandidateRecord::Propose {
                name,
                source_memory,
            } => {
                let stored: Option<i64> = sqlx::query_scalar(SELECT_SLOTS)
                    .bind(character_id.get())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(StoreError::Query)?;
                let Some(stored) = stored else {
                    return Err(StoreError::UnknownCharacter);
                };
                let slots = slots_from_row(character_id, stored)?;

                let rows = sqlx::query(SELECT_CHARACTER_TRANSITIONS)
                    .bind(character_id.get())
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(StoreError::Query)?;
                let states = current_states(character_id, &transitions_by_candidate(&rows)?)?;
                if slots.free(states) == 0 {
                    tracing::debug!(
                        character_id = character_id.get(),
                        slots = slots.get(),
                        "軸を見いだす枠が空いていないので、候補を作りませんでした"
                    );
                    return Err(StoreError::AxisDiscoverySlotsFull);
                }

                let done = sqlx::query(INSERT_CANDIDATE)
                    .bind(character_id.get())
                    .bind(name.as_str())
                    .bind(source_memory.get())
                    .execute(&mut *tx)
                    .await
                    .map_err(StoreError::Query)?;
                let candidate = AxisCandidateId::from_row(done.last_insert_rowid());
                insert_transition(&mut tx, candidate, AxisCandidateState::Trial).await?;
                // **候補を採ったこと自体が1回の試み**（作業単位 A-9b）。同じトランザクションで
                // 目印を進める——分けると、候補だけが入って目印が残らない窓ができ、
                // 次のターンにもう一度尋ねる。
                insert_attempt(
                    &mut tx,
                    character_id,
                    *source_memory,
                    AxisDiscoveryOutcome::Proposed,
                )
                .await?;
                candidate
            }
            AxisCandidateRecord::Advance { candidate, to } => {
                let current = owned_candidate_state(&mut tx, character_id, *candidate).await?;
                if !current.can_become(*to) {
                    tracing::warn!(
                        character_id = character_id.get(),
                        candidate_id = candidate.get(),
                        from = current.label(),
                        to = to.label(),
                        "軸の候補を規則にない状態へ進めようとしたので拒みました"
                    );
                    return Err(StoreError::AxisCandidateTransitionRefused);
                }
                insert_transition(&mut tx, *candidate, *to).await?;
                *candidate
            }
            AxisCandidateRecord::TrialVote {
                candidate,
                session,
                vote,
            } => {
                let current = owned_candidate_state(&mut tx, character_id, *candidate).await?;
                if current != AxisCandidateState::Trial {
                    tracing::warn!(
                        character_id = character_id.get(),
                        candidate_id = candidate.get(),
                        state = current.label(),
                        "試用中でない軸の候補に票を足そうとしたので拒みました"
                    );
                    return Err(StoreError::AxisCandidateNotOnTrial);
                }
                // **会話も候補の持ち主のものに限る**（Issue origin#43 の申し送り2）。
                // 同じトランザクションで照合する —— 分けると、照合と書き込みの間の窓が残る。
                let session_owned: Option<i64> = sqlx::query_scalar(SELECT_OWNED_SESSION)
                    .bind(session.get())
                    .bind(character_id.get())
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(StoreError::Query)?;
                if session_owned.is_none() {
                    tracing::warn!(
                        character_id = character_id.get(),
                        candidate_id = candidate.get(),
                        session_id = session.get(),
                        "候補の持ち主ではないキャラクターの会話の試用の票を拒みました"
                    );
                    return Err(StoreError::SessionOfAnotherCharacter);
                }
                sqlx::query(INSERT_TRIAL_VOTE)
                    .bind(candidate.get())
                    .bind(session.get())
                    .bind(vote.label())
                    .execute(&mut *tx)
                    .await
                    .map_err(StoreError::Query)?;
                *candidate
            }
        };

        tx.commit().await.map_err(StoreError::Query)?;
        // **名前はログに出さない**（LLM 由来の文字列になる / 手掛かりは id）。
        tracing::debug!(
            character_id = character_id.get(),
            candidate_id = touched.get(),
            "軸の候補を記録しました"
        );
        Ok(touched)
    }

    /// **候補を採らずに終わった試みを1回記録する**（作業単位 A-9b / Issue origin#42 の申し送り1）。
    ///
    /// 完全一致で捨てた回・崩れた応答の回・上流を呼べなかった回も**「試みた」に数える**。
    /// 記録しないと、目印が進まずに**次のターンにもまた尋ねる**（採るまで毎ターン）。
    ///
    /// `through` は**その回に数え終えた記憶**（数えた記憶のうち最新）。
    /// 同じキャラクターの記憶でなければ外部キーが止める（`StoreError::Query`）。
    /// **候補を採った回はここを通らない**——`record_axis_candidate` の `Propose` が
    /// 候補と同じトランザクションで記録する。
    ///
    /// 読んでから書く検査は無い（1文）。トランザクションで囲うのは、追記の文を
    /// `Propose` の腕と同じ1本（[`insert_attempt`]）に通すためだけ。
    ///
    /// # **テスト専用**（作業単位 A-10）
    ///
    /// 会話で縛らない。本番の書き手は [`Self::record_axis_discovery_miss_in_session`]。
    #[cfg(any(test, feature = "testing"))]
    pub async fn record_axis_discovery_miss(
        &self,
        character_id: CharacterId,
        through: MemoryId,
        miss: AxisDiscoveryMiss,
    ) -> Result<(), StoreError> {
        self.write_axis_discovery_miss(character_id, None, through, miss)
            .await
    }

    /// **往復の会話で縛って**、候補を採らなかった試みを記録する
    /// （本番の書き手 / 作業単位 A-10 / ADR-0049 §決定3）。
    ///
    /// [`Self::record_axis_candidate_in_session`] と同じ形（書きロックを取ってから確かめる）。
    /// `through` の記憶の id も使い回されうるので、縛らないと、消したキャラクターの目印が
    /// 同じ id の新しいキャラクターに残る（新しいキャラクターの試みが黙って先送りされる）。
    pub async fn record_axis_discovery_miss_in_session(
        &self,
        character_id: CharacterId,
        session_id: SessionId,
        through: MemoryId,
        miss: AxisDiscoveryMiss,
    ) -> Result<(), StoreError> {
        self.write_axis_discovery_miss(character_id, Some(session_id), through, miss)
            .await
    }

    /// 採らなかった試みを記録する本体（`origin` が `None` なら会話で縛らない）。
    async fn write_axis_discovery_miss(
        &self,
        character_id: CharacterId,
        origin: Option<SessionId>,
        through: MemoryId,
        miss: AxisDiscoveryMiss,
    ) -> Result<(), StoreError> {
        // **会話で縛るときは書きロックを先に取る**（確かめる読みから始まるので、素の `BEGIN` だと
        // 昇格が `SQLITE_BUSY_SNAPSHOT` になりうる / `BEGIN_IMMEDIATE` の doc）。
        let mut tx = self
            .pool
            .begin_with(BEGIN_IMMEDIATE)
            .await
            .map_err(StoreError::Query)?;
        if let Some(origin) = origin {
            ensure_turn_session(&mut tx, character_id, origin).await?;
        }
        insert_attempt(
            &mut tx,
            character_id,
            through,
            AxisDiscoveryOutcome::Missed(miss),
        )
        .await?;
        tx.commit().await.map_err(StoreError::Query)?;
        tracing::debug!(
            character_id = character_id.get(),
            through_memory_id = through.get(),
            outcome = AxisDiscoveryOutcome::Missed(miss).label(),
            "軸の候補を採らなかった試みを記録しました"
        );
        Ok(())
    }

    /// **そのキャラクターの試みの目印**（これまでの試みで数え終えた記憶のうち最新）。
    /// **まだ一度も試みていなければ `None`**（作業単位 A-9b）。
    ///
    /// 候補づくりは、この目印より後に増えたそのキャラクター自身の記憶を数える
    /// （`domain::memories_since`）。**候補の表からは取らない**（採った回しか残らない /
    /// Issue origin#42 の申し送り1）。
    pub async fn axis_discovery_mark(
        &self,
        character_id: CharacterId,
    ) -> Result<Option<MemoryId>, StoreError> {
        let mark: Option<i64> = sqlx::query_scalar(SELECT_ATTEMPT_MARK)
            .bind(character_id.get())
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        Ok(mark.map(MemoryId::from_row))
    }

    /// **そのキャラクターの軸の候補を、新しい順に最大 `limit` 件**返す（ADR-0046 §決定6）。
    ///
    /// 候補ごとに**いまの状態（導出）・移り変わり・試用の票の内訳**を持つ。
    /// **プロンプトには載らない**（管理のための一覧）。
    ///
    /// # **1件でも読めなければ、この読みは丸ごと止まる**
    ///
    /// `attribute_revisions` / `drift_vote_history` と同じ側に倒してある。
    /// 読めないのは: 名前が `StatAxis` を通らない・状態や票の綴りが規則外・
    /// **移り変わりの列が規則に合わない**（最初が試用でない・規則にない遷移・1行も無い）。
    /// **黙って飛ばすと、どの候補がどこにいるのかが静かに1件消える。**
    ///
    /// | | |
    /// |---|---|
    /// | **止まるもの** | この一覧と、そのキャラクターの**候補づくり**（空いた枠を数えられない。**移り変わりの列が崩れた候補に限る** —— 移り変わりが1行も無い候補は候補づくりが数えず、一覧だけが止まる） |
    /// | **止まらないもの** | 総数・本文の応答・記憶・固定の軸の変容・進める／票を足す（その候補の列が読める限り） |
    ///
    /// 直す導線は `sqlite3` だけ——**3枚とも更新も削除も無い**（追記のみ）。
    ///
    /// # 3つの文を1つのトランザクションで囲わない
    ///
    /// 移り変わりと票は**一覧の上限で絞らずに**読むので、文の間に候補や行が増えても、
    /// 一覧に載った候補の列は欠けない（増えるだけ）。**欠けないので、囲う理由が無い**
    /// （[`SELECT_CHARACTER_TRANSITIONS`] の doc）。
    pub async fn axis_candidates(
        &self,
        character_id: CharacterId,
        limit: u32,
    ) -> Result<Vec<AxisCandidate>, StoreError> {
        let rows = sqlx::query(SELECT_CANDIDATES)
            .bind(character_id.get())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        let transition_rows = sqlx::query(SELECT_CHARACTER_TRANSITIONS)
            .bind(character_id.get())
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        let mut transitions = transitions_by_candidate(&transition_rows)?;
        let tally_rows = sqlx::query(SELECT_TALLIES)
            .bind(character_id.get())
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        let mut tallies = tallies_by_candidate(&tally_rows)?;

        let mut candidates = Vec::with_capacity(rows.len());
        for row in &rows {
            let id: i64 = row.try_get("id").map_err(StoreError::Query)?;
            let stored_name: String = row.try_get("name").map_err(StoreError::Query)?;
            let name = StatAxis::parse(&stored_name).map_err(|_| {
                // **格納値そのものはログにも出さない**（他の拒否と同じ。手掛かりは id）。
                tracing::warn!(
                    character_id = character_id.get(),
                    candidate_id = id,
                    "読み出せない名前を持つ軸の候補があります"
                );
                StoreError::InvalidStoredValue
            })?;
            let source_memory: i64 = row.try_get("source_memory_id").map_err(StoreError::Query)?;
            let created_at: DateTime<Utc> = row.try_get("created_at").map_err(StoreError::Query)?;

            let history = transitions.remove(&id).unwrap_or_default();
            let states: Vec<AxisCandidateState> =
                history.iter().map(AxisCandidateTransition::state).collect();
            let Some(state) = current_axis_candidate_state(&states) else {
                tracing::warn!(
                    character_id = character_id.get(),
                    candidate_id = id,
                    "移り変わりの列が規則に合わない軸の候補があります"
                );
                return Err(StoreError::InvalidStoredValue);
            };

            candidates.push(AxisCandidate {
                id: AxisCandidateId::from_row(id),
                name,
                source_memory: MemoryId::from_row(source_memory),
                created_at,
                transitions: history,
                state,
                tally: tallies.remove(&id).unwrap_or_default(),
            });
        }
        tracing::debug!(
            character_id = character_id.get(),
            candidates = candidates.len(),
            "軸の候補を読み出しました"
        );
        Ok(candidates)
    }

    /// **そのキャラクターの軸の候補の総数。上限で切らない。**
    ///
    /// 数えるだけなので名前も状態も読まない——**壊れた候補があっても総数は返る**
    /// （止まるのは一覧の側だけ）。
    pub async fn count_axis_candidates(
        &self,
        character_id: CharacterId,
    ) -> Result<u32, StoreError> {
        let counted: i64 = sqlx::query_scalar(COUNT_CANDIDATES)
            .bind(character_id.get())
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        // **`try_get::<u32>` にしない**（`count_attribute_revisions` と同じ理由）。
        u32::try_from(counted).map_err(|_| StoreError::InvalidStoredValue)
    }
}

/// 枠の数を読む（`store::character` の読み出しと同じ検証）。
fn slots_from_row(
    character_id: CharacterId,
    stored: i64,
) -> Result<AxisDiscoverySlots, StoreError> {
    u32::try_from(stored)
        .ok()
        .and_then(|raw| AxisDiscoverySlots::parse(raw).ok())
        .ok_or_else(|| {
            tracing::warn!(
                character_id = character_id.get(),
                "軸を見いだす枠の数が不正なキャラクターがあります。データベースを直接確認してください"
            );
            StoreError::InvalidStoredValue
        })
}

/// 移り変わりの状態と理由の綴りを読む。規則外は `InvalidStoredValue`。
fn state_from_row(row: &SqliteRow) -> Result<AxisCandidateState, StoreError> {
    let stored_state: String = row.try_get("state").map_err(StoreError::Query)?;
    let stored_reason: Option<String> = row.try_get("reason").map_err(StoreError::Query)?;
    AxisCandidateState::from_stored(&stored_state, stored_reason.as_deref()).ok_or_else(|| {
        // **格納値そのものはログにも出さない。**
        tracing::warn!("読み出せない状態を持つ軸の候補の移り変わりがあります");
        StoreError::InvalidStoredValue
    })
}

/// 移り変わりの行を候補ごとに畳む（行は候補ごと・古い順に並んでいる前提）。
fn transitions_by_candidate(
    rows: &[SqliteRow],
) -> Result<BTreeMap<i64, Vec<AxisCandidateTransition>>, StoreError> {
    let mut grouped: BTreeMap<i64, Vec<AxisCandidateTransition>> = BTreeMap::new();
    for row in rows {
        let candidate: i64 = row.try_get("candidate_id").map_err(StoreError::Query)?;
        let state = state_from_row(row)?;
        let created_at: DateTime<Utc> = row.try_get("created_at").map_err(StoreError::Query)?;
        grouped
            .entry(candidate)
            .or_default()
            .push(AxisCandidateTransition { state, created_at });
    }
    Ok(grouped)
}

/// 候補ごとのいまの状態。**1件でも列が崩れていれば `InvalidStoredValue`**（枠を数え違えないため）。
fn current_states(
    character_id: CharacterId,
    grouped: &BTreeMap<i64, Vec<AxisCandidateTransition>>,
) -> Result<Vec<AxisCandidateState>, StoreError> {
    grouped
        .iter()
        .map(|(candidate, history)| {
            let states: Vec<AxisCandidateState> =
                history.iter().map(AxisCandidateTransition::state).collect();
            current_axis_candidate_state(&states).ok_or_else(|| {
                tracing::warn!(
                    character_id = character_id.get(),
                    candidate_id = candidate,
                    "移り変わりの列が規則に合わない軸の候補があります"
                );
                StoreError::InvalidStoredValue
            })
        })
        .collect()
}

/// 試用の票の件数を候補ごとに畳む。
fn tallies_by_candidate(rows: &[SqliteRow]) -> Result<BTreeMap<i64, AxisTrialTally>, StoreError> {
    let mut grouped: BTreeMap<i64, AxisTrialTally> = BTreeMap::new();
    for row in rows {
        let candidate: i64 = row.try_get("candidate_id").map_err(StoreError::Query)?;
        let stored_vote: String = row.try_get("vote").map_err(StoreError::Query)?;
        let Some(vote) = AxisTrialVote::from_stored(&stored_vote) else {
            tracing::warn!(
                candidate_id = candidate,
                "読み出せない向きを持つ試用の票があります"
            );
            return Err(StoreError::InvalidStoredValue);
        };
        let votes: i64 = row.try_get("votes").map_err(StoreError::Query)?;
        let votes = u32::try_from(votes).map_err(|_| StoreError::InvalidStoredValue)?;
        grouped.entry(candidate).or_default().add(vote, votes);
    }
    Ok(grouped)
}

/// **その候補がそのキャラクターのものか**を見て、いまの状態を返す（書き手の中で使う）。
async fn owned_candidate_state(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    character_id: CharacterId,
    candidate: AxisCandidateId,
) -> Result<AxisCandidateState, StoreError> {
    let owned: Option<i64> = sqlx::query_scalar(SELECT_OWNED_CANDIDATE)
        .bind(candidate.get())
        .bind(character_id.get())
        .fetch_optional(&mut **tx)
        .await
        .map_err(StoreError::Query)?;
    if owned.is_none() {
        // **「無い」と「他のキャラクターのもの」を分けない**——どちらもこのキャラクターからは
        // 見えない候補で、読む口（一覧）も他のキャラクターの候補を出さない。
        tracing::warn!(
            character_id = character_id.get(),
            candidate_id = candidate.get(),
            "このキャラクターの軸の候補ではありません"
        );
        return Err(StoreError::UnknownAxisCandidate);
    }

    let rows = sqlx::query(SELECT_CANDIDATE_TRANSITIONS)
        .bind(candidate.get())
        .fetch_all(&mut **tx)
        .await
        .map_err(StoreError::Query)?;
    let mut states = Vec::with_capacity(rows.len());
    for row in &rows {
        states.push(state_from_row(row)?);
    }
    current_axis_candidate_state(&states).ok_or_else(|| {
        tracing::warn!(
            character_id = character_id.get(),
            candidate_id = candidate.get(),
            "移り変わりの列が規則に合わない軸の候補があります"
        );
        StoreError::InvalidStoredValue
    })
}

/// **往復の会話が今も在り、そのキャラクターのものか**を、書きロックを握ったトランザクションの中で
/// 確かめる（作業単位 A-10 / ADR-0049 §決定3）。無ければ [`StoreError::TurnSessionGone`]、
/// 別のキャラクターの会話なら [`StoreError::SessionOfAnotherCharacter`]
/// （理由の分け方は `session::turn_session_refusal`）。
///
/// **`BEGIN IMMEDIATE` で始めたトランザクションでだけ呼ぶこと**（確かめてから書くまでの隙を、
/// 書きロックで閉じている）。**どの腕よりも先に呼ぶ**（`write_axis_candidate` の `match` の手前）。
async fn ensure_turn_session(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    character_id: CharacterId,
    session_id: SessionId,
) -> Result<(), StoreError> {
    match super::session::turn_session_refusal(&mut **tx, character_id, session_id).await? {
        Some(refusal) => Err(refusal),
        None => Ok(()),
    }
}

/// 試みを1件足す（2つの書き手の中で使う / [`INSERT_ATTEMPT`]）。
async fn insert_attempt(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    character_id: CharacterId,
    through: MemoryId,
    outcome: AxisDiscoveryOutcome,
) -> Result<(), StoreError> {
    sqlx::query(INSERT_ATTEMPT)
        .bind(character_id.get())
        .bind(through.get())
        .bind(outcome.label())
        .execute(&mut **tx)
        .await
        .map_err(StoreError::Query)?;
    Ok(())
}

/// 移り変わりを1件足す（書き手の中で使う）。**理由は状態から取る**（写しを作らない）。
async fn insert_transition(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    candidate: AxisCandidateId,
    state: AxisCandidateState,
) -> Result<(), StoreError> {
    sqlx::query(INSERT_TRANSITION)
        .bind(candidate.get())
        .bind(state.label())
        .bind(state.reason().map(AxisTrialFailure::label))
        .execute(&mut **tx)
        .await
        .map_err(StoreError::Query)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // 速報（SQL 文そのものの断片）。**実挙動は tests/store_axis_candidate.rs が持つ。**
    // ---------------------------------------------------------------------

    /// **持ち主で絞る文が、全部持ち主で絞っている。**
    ///
    /// | 欠けると | |
    /// |---|---|
    /// | 一覧の `WHERE character_id = ?` | **他のキャラクターの候補が混ざる** |
    /// | 移り変わり・票の副問い合わせの `WHERE character_id = ?` | 他のキャラクターの状態で**枠を数え違える** |
    /// | 持ち主の照合の `AND character_id = ?` | **他のキャラクターの候補を id だけで進められる** |
    ///
    /// 速報でしかない——実挙動の権威は `tests/store_axis_candidate.rs::
    /// another_characters_candidate_is_out_of_reach` と
    /// `tests/server_admin_axis_candidates.rs::the_candidates_of_each_character_are_their_own`。
    #[test]
    fn every_read_is_scoped_to_one_character() {
        for statement in [
            SELECT_CANDIDATES,
            COUNT_CANDIDATES,
            SELECT_CHARACTER_TRANSITIONS,
            SELECT_TALLIES,
            SELECT_ATTEMPT_MARK,
        ] {
            assert!(
                statement.contains("WHERE character_id = ?"),
                "持ち主で絞ること: {statement}"
            );
        }
        for statement in [SELECT_OWNED_CANDIDATE, SELECT_OWNED_SESSION] {
            assert!(
                statement.contains("AND character_id = ?"),
                "持ち主を照合すること: {statement}"
            );
        }
    }

    /// **一覧は新しい順・上限つき、総数は上限を持たない。移り変わりは追記の順で読む。**
    #[test]
    fn the_listing_keeps_its_order_and_limit_and_the_count_has_none() {
        for fragment in ["ORDER BY created_at DESC, id DESC", "LIMIT ?"] {
            assert!(
                SELECT_CANDIDATES.contains(fragment),
                "{fragment} が消えています: {SELECT_CANDIDATES}"
            );
        }
        assert!(
            !COUNT_CANDIDATES.contains("LIMIT"),
            "総数を上限で切らないこと: {COUNT_CANDIDATES}"
        );
        for statement in [SELECT_CANDIDATE_TRANSITIONS, SELECT_CHARACTER_TRANSITIONS] {
            assert!(
                statement.contains("id") && statement.ends_with("id"),
                "移り変わりは id（追記の順）で並べること: {statement}"
            );
            assert!(
                !statement.contains("LIMIT"),
                "移り変わりを上限で絞らないこと（一覧に載った候補の列が欠ける）: {statement}"
            );
        }
        assert!(
            !SELECT_TALLIES.contains("LIMIT"),
            "票の内訳を上限で絞らないこと: {SELECT_TALLIES}"
        );
    }

    /// **どの文もプレースホルダで値を渡す**（CLAUDE.md 絶対制約）。
    ///
    /// 状態や票の綴り（`'trial'` 等）を SQL に直書きする形は、ここで**リテラルとして落ちる**
    /// （`label` の写しも同時に防がれる）。
    #[test]
    fn every_statement_binds_its_values_through_placeholders() {
        let quote = '\'';
        for statement in [
            SELECT_SLOTS,
            INSERT_CANDIDATE,
            INSERT_TRANSITION,
            INSERT_TRIAL_VOTE,
            INSERT_ATTEMPT,
            SELECT_ATTEMPT_MARK,
            SELECT_OWNED_CANDIDATE,
            SELECT_OWNED_SESSION,
            SELECT_CANDIDATE_TRANSITIONS,
            SELECT_CHARACTER_TRANSITIONS,
            SELECT_CANDIDATES,
            COUNT_CANDIDATES,
            SELECT_TALLIES,
        ] {
            assert!(
                !statement.contains(quote),
                "リテラルを埋め込まないこと: {statement}"
            );
        }
    }

    /// **書き手は `BEGIN IMMEDIATE` で入る。**
    ///
    /// 速報でしかない——**効いていること**の権威は `tests/store_axis_candidate.rs::
    /// two_proposals_never_overfill_one_slot`。
    #[test]
    fn the_writer_opens_an_immediate_transaction() {
        assert_eq!(BEGIN_IMMEDIATE, "BEGIN IMMEDIATE");
    }

    // ---------------------------------------------------------------------
    // SQL の走査（ADR-0046 §決定6「追記のみ」「書き込み経路は1つ」）
    //
    // 形は `store::attribute` の走査（履歴の表と投影の表）に倣う。**判定はこのモジュールの
    // 2つの関数（[`candidate_violations`] / [`candidate_writes`]）だけ**にして、
    // 見張り番も対照も同じ関数を通す（写しにすると、判定を緩める変異が両側とも緑を通る）。
    //
    // **走査は生ソースをトークンに均す。** 代償は誤検出する側に倒れることで、それは正しい。
    // ---------------------------------------------------------------------

    /// 走査対象。**`migrations` を外さないこと**（整理 SQL はマイグレーションに書かれやすい）。
    /// **Rust 側は `src` 全体**（`store` に絞ると `src/server/admin.rs` に1行書くだけで外に出る）。
    const CANDIDATE_SCAN_ROOTS: [(&str, &str); 2] = [("src", "rs"), ("migrations", "sql")];

    /// **3枚へ書いてよい唯一のファイル**（このファイル）。
    const CANDIDATE_WRITER: [&str; 3] = ["src", "store", "axis_candidate.rs"];

    /// 4枚のテーブル名（大文字）を**ソース上に綴らずに**組み立てる。
    ///
    /// この走査は自分自身のファイルも対象にするので、綴りをそのまま書くと判定の対象になる
    /// （`store::memory::table_word` と同じ作法）。
    ///
    /// **4枚目（試みの表）は作業単位 A-9b で足した。** 同じく追記のみで、書き手はこのファイル
    /// （[`insert_attempt`] の1文）。走査の判定そのものは変えていない（対象の表が1枚増えただけ）。
    fn candidate_table_words() -> [String; 4] {
        [
            ["stat_axis", "_candidate"].concat().to_uppercase(),
            ["stat_axis", "_candidate", "_transition"]
                .concat()
                .to_uppercase(),
            ["stat_axis", "_trial", "_vote"].concat().to_uppercase(),
            ["stat_axis", "_discovery", "_attempt"]
                .concat()
                .to_uppercase(),
        ]
    }

    /// **3枚のどれも、書き換える文がどこにも無い**（Issue origin#41 の閉じる条件 / 追記のみ）。
    ///
    /// # 判定は禁止語の列挙ではなく**許可リスト**
    ///
    /// 許すのは**追記（`INSERT INTO`）・読み出し（`FROM`）・DDL** だけ
    /// （[`candidate_form_is_allowed`]）。**代償は、テーブル名を Rust のコメントや識別子に
    /// 綴れないこと**——綴ると許可された形に当たらず、この見張り番が赤になる。
    #[test]
    fn no_statement_updates_or_deletes_an_axis_candidate_row() {
        let targets = candidate_scan_targets();
        assert!(
            targets.len() >= 5,
            "走査対象が少なすぎるなら、この assert は何も守っていない: {:?}",
            targets.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );

        for (table, minimum) in candidate_table_words().iter().zip([5, 4, 4, 4]) {
            let mut mentions = 0;
            let mut found = Vec::new();
            for (path, source) in &targets {
                mentions += candidate_mentions_of(source, table);
                for violation in candidate_violations(source, table) {
                    found.push(format!("{path}: {violation}"));
                }
            }
            assert!(
                found.is_empty(),
                "{table} を書き換えうる文があります:\n{}",
                found.join("\n")
            );
            // **言及を拾えていないなら、走査が壊れている。**
            assert!(
                mentions >= minimum,
                "{table} の言及を拾えていない（走査が壊れている）: {mentions} 件"
            );
        }
    }

    /// **3枚へ書く文が、唯一の書き手のファイルの外に無い。書き手のファイルには各1文ずつある。**
    ///
    /// 後半が無いと、**判定が何も見つけられなくなっても緑になる**（「見つからないこと」だけを主張するため）。
    #[test]
    fn no_file_but_the_one_writer_appends_to_the_axis_candidate_tables() {
        let writer = CANDIDATE_WRITER.join(std::path::MAIN_SEPARATOR_STR);
        let targets = candidate_scan_targets();
        assert!(
            targets.iter().any(|(path, _)| path.ends_with(&writer)),
            "唯一の書き手のファイルが走査対象に居ること"
        );

        for table in &candidate_table_words() {
            let mut outside = Vec::new();
            let mut inside = Vec::new();
            for (path, source) in &targets {
                let writes = candidate_writes(source, table);
                if path.ends_with(&writer) {
                    inside.extend(writes);
                } else {
                    outside.extend(writes.into_iter().map(|write| format!("{path}: {write}")));
                }
            }
            assert!(
                outside.is_empty(),
                "{table} へ書く文が唯一の書き手の外にあります:\n{}",
                outside.join("\n")
            );
            assert_eq!(
                inside.len(),
                1,
                "{table} へ書く文は書き手のファイルに1つだけ（INSERT）: {inside:?}"
            );
        }
    }

    /// **走査本体が、既知の回避手段を全部落とす**（植えた対照 / `15b`）。
    ///
    /// **3枚それぞれ**で、**LF と CRLF の両方で同じ答え**になることまで見る。
    #[test]
    fn the_candidate_scan_catches_every_known_evasion() {
        for table in &candidate_table_words() {
            let name = table.to_lowercase();
            let evasions = [
                format!("UPDATE {name} SET state = ? WHERE id = ?"),
                format!("UPDATE {name}\nSET vote = ?"),
                format!("UPDATE main.{name} SET name = ?"),
                format!("UPDATE {name} AS c SET name = ?"),
                format!("UPDATE OR REPLACE {name} SET name = ?"),
                format!("DELETE FROM {name} WHERE character_id = ?"),
                format!("DELETE\nFROM main.{name}"),
                format!("INSERT OR REPLACE INTO {name} (id) VALUES (?)"),
                format!("REPLACE INTO {name} (id) VALUES (?)"),
                format!(
                    "INSERT INTO {name} (id, state)\nVALUES (?, ?) \
                     ON CONFLICT (id) DO UPDATE SET state = excluded.state"
                ),
                format!("DROP TABLE {name}"),
                format!("ALTER TABLE {name} RENAME TO old"),
                format!("TRUNCATE {name}"),
                // 前の文の末尾を借りる形（記憶の走査が実際に踏んだ）。
                format!("PRAGMA foreign_keys = ON;\nDELETE FROM {name};"),
                // **表の名前が値の位置に来る形・JOIN・散文**（既定は拒否）。
                format!("SELECT 1 FROM other WHERE x = {name}"),
                format!("SELECT 1 FROM other JOIN {name} ON other.id = {name}.id"),
                format!("PRAGMA table_info({name})"),
                format!("/// `{name}` は追記のみ。"),
            ];
            for evasion in &evasions {
                let found = candidate_violations(evasion, table);
                assert!(!found.is_empty(), "この形を落とせていません: {evasion}");
                let crlf = evasion.replace('\n', "\r\n");
                assert_eq!(
                    candidate_violations(&crlf, table),
                    found,
                    "改行が CRLF だと答えが変わっています: {evasion}"
                );
            }
        }
    }

    /// **既定の枝（許可リストに無い形）が、破壊的な語なしでも効いている。**
    ///
    /// `_ => false` を `_ => true` にする変異を、ここだけが赤にする。
    #[test]
    fn the_candidate_scan_rejects_forms_that_are_not_on_the_allowlist() {
        for table in &candidate_table_words() {
            let name = table.to_lowercase();
            for statement in [
                format!("SELECT 1 FROM other WHERE x = {name}"),
                format!("SELECT 1 FROM other JOIN {name} ON other.id = x"),
                format!("PRAGMA table_info({name})"),
                format!("/// 候補の表は {name} である。"),
            ] {
                assert!(
                    candidate_destructive_in(&candidate_tokens(&statement)).is_none(),
                    "前提: 破壊的な語を含まない文であること: {statement}"
                );
                assert!(
                    !candidate_violations(&statement, table).is_empty(),
                    "許可リストに無い形が通っています: {statement}"
                );
            }
        }
    }

    /// **設計上ありうる形は落とさない**（追記・読み出し・DDL・参照）。
    ///
    /// ここを狭めると、追記と読み出しそのものが書けなくなって**走査ごと外される**羽目になる。
    #[test]
    fn the_candidate_scan_accepts_the_statements_the_design_allows() {
        for table in &candidate_table_words() {
            let name = table.to_lowercase();
            let allowed = [
                format!("INSERT INTO {name} (candidate_id, state, reason) VALUES (?, ?, ?)"),
                format!("SELECT id, name FROM {name} WHERE character_id = ? LIMIT ?"),
                format!("SELECT count(*) FROM {name}"),
                format!(
                    "SELECT a FROM b WHERE candidate_id IN (SELECT id FROM {name} WHERE character_id = ?)"
                ),
                format!(
                    "CREATE TABLE {name} (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                     character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE)"
                ),
                format!("CREATE INDEX {name}_idx\n    ON {name} (candidate_id, id)"),
                format!(
                    "CREATE TABLE other (candidate_id INTEGER NOT NULL \
                     REFERENCES {name}(id) ON DELETE CASCADE)"
                ),
                format!("PRAGMA foreign_keys = ON;\nSELECT id FROM {name} WHERE character_id = ?;"),
            ];
            for statement in &allowed {
                let violations = candidate_violations(statement, table);
                assert!(
                    violations.is_empty(),
                    "正当な文を落としています: {statement} / {violations:?}"
                );
                let crlf = statement.replace('\n', "\r\n");
                assert!(
                    candidate_violations(&crlf, table).is_empty(),
                    "CRLF の正当な文を落としています: {statement}"
                );
            }
        }
    }

    /// **書く位置の判定が、既知の書き込みの形を1件ずつ拾い、読みと DDL を拾わない。**
    #[test]
    fn the_write_scan_finds_writes_and_leaves_reads_alone() {
        for table in &candidate_table_words() {
            let name = table.to_lowercase();
            for write in [
                format!("INSERT INTO {name} (id)\nVALUES (?)"),
                format!("INSERT OR REPLACE INTO {name} (id) VALUES (?)"),
                format!("REPLACE INTO {name} (id) VALUES (?)"),
                format!("UPDATE main.{name} SET name = ?"),
                format!("DELETE FROM {name}"),
                format!("DROP TABLE {name}"),
            ] {
                assert_eq!(
                    candidate_writes(&write, table).len(),
                    1,
                    "1件の書き込みとして拾うこと: {write}"
                );
                let crlf = write.replace('\n', "\r\n");
                assert_eq!(candidate_writes(&crlf, table).len(), 1, "CRLF: {write}");
            }
            for read in [
                format!("SELECT id FROM {name} WHERE character_id = ?"),
                format!("CREATE TABLE {name} (id INTEGER)"),
                format!("CREATE INDEX x ON {name} (id)"),
                format!("CREATE TABLE other (c INTEGER REFERENCES {name}(id))"),
            ] {
                assert!(
                    candidate_writes(&read, table).is_empty(),
                    "書き込みでない形を拾わないこと: {read}"
                );
            }
        }
    }

    /// **走査は `src` 全体と `migrations` を見て、中身も読めている。**
    #[test]
    fn the_candidate_scans_cover_every_module_and_the_migrations() {
        let separator = std::path::MAIN_SEPARATOR;
        let scanned = candidate_scan_targets();
        for module in ["server", "prompt", "llm", "domain", "parse"] {
            let expected = format!("src{separator}{module}{separator}");
            assert!(
                scanned.iter().any(|(path, _)| path.contains(&expected)),
                "{module} が走査対象に入っていない"
            );
        }
        assert!(
            scanned.iter().any(|(path, _)| path.ends_with(".sql")),
            "migrations の .sql を拾えていない"
        );
        // **`server` に生の SQL を書けば掛かる**ことを、植えたソースで確かめる。
        let [candidate, _, _, _] = candidate_table_words();
        let planted = format!(
            "const BAD: &str = \"DELETE FROM {} WHERE character_id = ?\";\n",
            candidate.to_lowercase()
        );
        assert_eq!(candidate_violations(&planted, &candidate).len(), 1);
        assert_eq!(candidate_writes(&planted, &candidate).len(), 1);
    }

    /// 走査対象を「パス, ソース」で返す。**`.sql` は行コメントを落としてから。**
    fn candidate_scan_targets() -> Vec<(String, String)> {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut targets = Vec::new();
        for (root, extension) in CANDIDATE_SCAN_ROOTS {
            let mut stack = vec![manifest.join(root)];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).expect("走査対象を読めること") {
                    let path = entry.expect("ディレクトリ項目を読めること").path();
                    if path.is_dir() {
                        stack.push(path);
                        continue;
                    }
                    if !path.extension().is_some_and(|ext| ext == extension) {
                        continue;
                    }
                    let source = std::fs::read_to_string(&path).expect("ソースを読めること");
                    let text = if extension == "sql" {
                        strip_sql_comments(&source)
                    } else {
                        source
                    };
                    targets.push((path.display().to_string(), text));
                }
            }
        }
        targets
    }

    /// SQL の行コメント以降を落とす。**`lines()` に分割を委ねるので LF / CRLF で変わらない。**
    fn strip_sql_comments(source: &str) -> String {
        let marker = ["-", "-"].concat();
        source
            .lines()
            .map(|line| line.split(&marker).next().unwrap_or(""))
            .collect::<Vec<&str>>()
            .join("\n")
    }

    /// ソースをトークン列に均す（`store::attribute::attribute_tokens` と同じ規則）。
    ///
    /// 語は `[A-Za-z0-9_]` の連なりで、大文字に揃える。**それ以外は全部区切り**なので、
    /// `\r` も `.` も引用符も同じ扱いになる——「CRLF でも答えが変わらない」ことの根拠。
    fn candidate_tokens(source: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut word = String::new();
        for ch in source.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                word.push(ch.to_ascii_uppercase());
                continue;
            }
            if !word.is_empty() {
                tokens.push(std::mem::take(&mut word));
            }
        }
        if !word.is_empty() {
            tokens.push(word);
        }
        tokens
    }

    /// ソース中のテーブル名の言及数。
    fn candidate_mentions_of(source: &str, table: &str) -> usize {
        candidate_tokens(source)
            .iter()
            .filter(|token| token.as_str() == table)
            .count()
    }

    /// 違反メッセージに載せる抜粋（**判定には使わない**）。
    fn candidate_excerpt(tokens: &[String], index: usize) -> String {
        let from = index.saturating_sub(4);
        let to = (index + 9).min(tokens.len());
        tokens[from..to].join(" ")
    }

    /// 言及の近くにあってはならない語（`store::attribute` の履歴の走査と同じ6語）。
    const CANDIDATE_DESTRUCTIVE: [&str; 6] = [
        "UPDATE", "REPLACE", "DELETE", "DROP", "TRUNCATE", "CONFLICT",
    ];

    /// 文中に破壊的な語があるか。**外部キーの参照アクション（`ON DELETE CASCADE`）だけは除く。**
    fn candidate_destructive_in(tokens: &[String]) -> Option<&'static str> {
        for (index, token) in tokens.iter().enumerate() {
            let reference_action = token.as_str() == "DELETE"
                && index > 0
                && tokens[index - 1] == "ON"
                && tokens.get(index + 1).map_or("", String::as_str) != "FROM";
            if reference_action {
                continue;
            }
            if let Some(found) = CANDIDATE_DESTRUCTIVE
                .iter()
                .find(|word| **word == token.as_str())
            {
                return Some(found);
            }
        }
        None
    }

    /// 言及が**許可された形**か。判定は直前の語で行う。**既定は拒否。**
    fn candidate_form_is_allowed(tokens: &[String], index: usize) -> bool {
        let before = |back: usize| -> &str {
            index
                .checked_sub(back)
                .and_then(|at| tokens.get(at))
                .map_or("", String::as_str)
        };
        match before(1) {
            // 追記。
            "INTO" => before(2) == "INSERT",
            // 読み出し。`DELETE FROM` は `candidate_destructive_in` が先に落とす。
            "FROM" => true,
            // 以下は migrations の DDL。
            "TABLE" => before(2) == "CREATE",
            "ON" => before(3) == "INDEX",
            "REFERENCES" => true,
            _ => false,
        }
    }

    /// **3枚を書き換える文の走査本体。** 見張り番も対照も**この関数を通す**こと。
    ///
    /// 判定の前に `;` で文へ割る（前の文の末尾を現在の文の文脈として読まないため）。
    fn candidate_violations(source: &str, table: &str) -> Vec<String> {
        source
            .split(';')
            .flat_map(|statement| {
                let tokens = candidate_tokens(statement);
                let mut found = Vec::new();
                for (index, token) in tokens.iter().enumerate() {
                    if token.as_str() != table {
                        continue;
                    }
                    let excerpt = candidate_excerpt(&tokens, index);
                    if let Some(keyword) = candidate_destructive_in(&tokens) {
                        found.push(format!("同じ文に {keyword} があります: {excerpt}"));
                    } else if !candidate_form_is_allowed(&tokens, index) {
                        found.push(format!("許可されていない形です: {excerpt}"));
                    }
                }
                found
            })
            .collect()
    }

    /// **表名が「行を変える文」の位置に居るか**（`store::attribute::attribute_write_keyword` と同じ表）。
    fn candidate_write_keyword(tokens: &[String], index: usize) -> Option<&'static str> {
        const NOISE: [&str; 8] = [
            "MAIN",
            "TEMP",
            "TEMPORARY",
            "OR",
            "ROLLBACK",
            "ABORT",
            "FAIL",
            "IGNORE",
        ];
        let mut before = Vec::new();
        let mut at = index;
        while at > 0 && before.len() < 2 {
            at -= 1;
            let token = tokens[at].as_str();
            if NOISE.contains(&token) {
                continue;
            }
            before.push(token);
        }
        let first = before.first().copied().unwrap_or("");
        let second = before.get(1).copied().unwrap_or("");
        match first {
            "INTO" => Some("INSERT"),
            "UPDATE" => Some("UPDATE"),
            "REPLACE" => Some("REPLACE"),
            "TRUNCATE" => Some("TRUNCATE"),
            "FROM" if second == "DELETE" => Some("DELETE"),
            "TABLE" if second == "DROP" => Some("DROP"),
            "TABLE" if second == "ALTER" => Some("ALTER"),
            _ => None,
        }
    }

    /// **3枚へ書く文の走査本体**（書き手の外に無いことを見る）。見張り番も対照も**この関数を通す**こと。
    fn candidate_writes(source: &str, table: &str) -> Vec<String> {
        source
            .split(';')
            .flat_map(|statement| {
                let tokens = candidate_tokens(statement);
                let mut found = Vec::new();
                for (index, token) in tokens.iter().enumerate() {
                    if token.as_str() != table {
                        continue;
                    }
                    if let Some(keyword) = candidate_write_keyword(&tokens, index) {
                        found.push(format!(
                            "{keyword} で書いています: {}",
                            candidate_excerpt(&tokens, index)
                        ));
                    }
                }
                found
            })
            .collect()
    }
}
