//! 取り決め（約束・決まり）の読み書きと、**その状態の移り変わりの履歴**
//! （[ADR-0051](../../docs/adr/0051-promises-and-rules.md) §決定1 / 作業単位 A-12a）。
//!
//! この層は**薄く保つ**。中身の検証と正規化は `domain::AgreementContent::parse`、
//! 件数の判定は `domain::admits_another_active_agreement` にある。
//! ここは「検証済みの値を SQL にバインドする」ことと、**判定を書き込みと同じトランザクションの
//! 中で行う**ことだけを担う。
//!
//! # 表が2枚ある（属性の 0009 / 0013 と同じ形 / ADR-0028 §決定5）
//!
//! | 表 | 立場 |
//! |---|---|
//! | 履歴（追記のみ） | **正**。状態の変化を1件ずつ足すだけで、更新も削除もしない |
//! | 取り決め（投影） | 中身・種類・守る側と、**いまの状態**。本文が読むのはこちら |
//!
//! **投影の行は消さない**（終わった取り決めも残る / ADR-0051 §決定1）。
//! 書き換えるのは状態の列（と、**採るときの提案中の1行の中身** —— [`Store::adopt_agreement`]）だけで、
//! それも履歴の追記と**同じトランザクション**の中だけ。
//! 書く文がこのファイルの外に無いこと・消す文がどこにも無いことは、このファイルの走査が見る。
//!
//! **走査対象のファイルに履歴の表の名前を綴らないこと**——履歴の走査は許可リストなので、
//! 散文に綴ると自分自身を落とす（`store::attribute` と同じ作法。だから doc では「履歴の表」と呼ぶ）。

use chrono::{DateTime, Utc};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::{Store, StoreError};
use crate::domain::{
    Agreement, AgreementContent, AgreementEdits, AgreementEnding, AgreementId, AgreementKeeper,
    AgreementKind, AgreementState, CharacterId, NewAgreement, SessionId,
    admits_another_active_agreement,
};

/// **読みから始まって書きに昇格するトランザクションの開始文。**
///
/// 書く前に有効な件数（書く口）・いまの状態（終わらせる口）を**読む**ので、素の `BEGIN` だと
/// 昇格が `SQLITE_BUSY_SNAPSHOT` になりうる（`store::attribute` の同名の定数の doc）。
/// **共有しない**（理由は同じ doc）。
const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

/// そのキャラクターの**有効な**取り決めの件数（上限の判定に使う）。
const COUNT_ACTIVE: &str =
    "SELECT count(*) FROM character_agreement WHERE character_id = ? AND state = ?";

/// 取り決めを1件書く。**投影へ行を足す唯一の文**（手で書く・提案を書くの両方が通る）。
///
/// | 書き手 | 状態 | 出どころの会話 |
/// |---|---|---|
/// | 手で書く（[`Store::add_agreement`]） | 有効 | NULL（手で書いたものは無し / ADR-0051 §決定1） |
/// | 提案を書く（[`Store::record_agreement_proposals`]） | 提案中 | 生んだ会話 |
const INSERT_AGREEMENT: &str = "INSERT INTO character_agreement \
     (character_id, content, kind, keeper, state, source_session_id) VALUES (?, ?, ?, ?, ?, ?) \
     RETURNING id, created_at";

/// **同じ中身の提案中・有効が、そのキャラクターに既にあるか**（作業単位 A-12c-1）。
///
/// 中身は `AgreementContent` で正規化した綴りどうしで比べる（種類・守る側は見ない）。
/// 却下・終わったものは見ない。**似ているもの（dedup2）を落とすのはここではない** ——
/// `parse::agreement_proposal` の後段が、却下も含めて先に落としている。ここは綴りがまったく同じ
/// 提案を二重に積まない最後の砦。
const COUNT_SAME_CONTENT: &str = "SELECT count(*) FROM character_agreement \
     WHERE character_id = ? AND content = ? AND state IN (?, ?)";

/// **提案を採る文**（作業単位 A-12c-1）。中身・種類・守る側を直してよい（`NULL` は提案のまま）。
///
/// **状態を書き換えるのは [`UPDATE_STATE`] とこれの2つだけ**。これは**提案中の1行だけ**に縛る
/// （`AND state = ?` に提案中を束ねる）——有効・終わった取り決めの中身は、この文でも書き換わらない。
const ADOPT_AGREEMENT: &str = "UPDATE character_agreement \
     SET content = coalesce(?, content), kind = coalesce(?, kind), keeper = coalesce(?, keeper), \
     state = ? WHERE id = ? AND character_id = ? AND state = ? \
     RETURNING id, content, kind, keeper, state, source_session_id, created_at";

/// **履歴を1件足す文。追記のみ。** `id` / `created_at` はスキーマの DEFAULT に任せる。
const INSERT_TRANSITION: &str =
    "INSERT INTO agreement_transition (agreement_id, before, after) VALUES (?, ?, ?)";

/// 終わらせる前に、**その取り決めのいまの状態**を読む（持ち主で縛る）。
const SELECT_STATE: &str =
    "SELECT state FROM character_agreement WHERE id = ? AND character_id = ?";

/// **投影の状態だけを書き換える文**（終わらせる・捨てる）。書き換えるのは状態の列だけ。
///
/// `AND state = ?`（**移る前の状態** —— 終わらせるなら有効、捨てるなら提案中）を外さないこと ——
/// 外すと、終わった取り決めをもう一度終わらせる書き換えが文の形として通る（いまは同じ
/// トランザクションで先に読んで止めているが、文の側でも「その状態のものだけ」に縛っておく）。
/// 採る（中身も直しうる）は別の文 [`ADOPT_AGREEMENT`]。
const UPDATE_STATE: &str = "UPDATE character_agreement SET state = ? \
     WHERE id = ? AND character_id = ? AND state = ?";

/// そのキャラクターの取り決めを**書いた順**で全部引く（一覧の口が使う）。
///
/// **第2キーは要らない**（`id` は AUTOINCREMENT で一意）。
const SELECT_AGREEMENTS: &str = "SELECT id, content, kind, keeper, state, source_session_id, \
     created_at FROM character_agreement WHERE character_id = ? ORDER BY id";

/// そのキャラクターの**有効な**取り決めを書いた順で引く（本文が毎ターン使う）。
///
/// **終わった行を読まない**ので、終わった行が1つ壊れていても本文は止まらない
/// （CLAUDE.md「読んだ瞬間から、その列のデコードは全経路の前提になる」の逆向き）。
const SELECT_ACTIVE: &str = "SELECT id, content, kind, keeper, state, source_session_id, \
     created_at FROM character_agreement WHERE character_id = ? AND state = ? ORDER BY id";

/// **そのキャラクターの履歴を新しい順に**引く（取り決めの中身と結んで）。
///
/// 第2キーに `id` を置く —— 書いた瞬間と終わらせた瞬間が同じミリ秒になりうる
/// （`store::attribute` の履歴と同じ理由）。**上限を SQL 側で切る**（履歴は消えない）。
const SELECT_TRANSITIONS: &str = "SELECT t.id, t.agreement_id, a.content, a.kind, a.keeper, \
     t.before, t.after, t.created_at \
     FROM agreement_transition AS t \
     JOIN character_agreement AS a ON a.id = t.agreement_id \
     WHERE a.character_id = ? ORDER BY t.created_at DESC, t.id DESC LIMIT ?";

/// **そのキャラクターの履歴の総数。上限で切らない**（返した件数を総数と呼ぶと画面が嘘をつく）。
const COUNT_TRANSITIONS: &str = "SELECT count(*) FROM agreement_transition AS t \
     JOIN character_agreement AS a ON a.id = t.agreement_id WHERE a.character_id = ?";

/// **状態の変化1件**（`store` の外へ出る形）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgreementTransition {
    id: i64,
    agreement_id: AgreementId,
    content: AgreementContent,
    kind: AgreementKind,
    keeper: AgreementKeeper,
    before: Option<AgreementState>,
    after: AgreementState,
    created_at: DateTime<Utc>,
}

impl AgreementTransition {
    /// 主キー（`sqlite3` で同じ行を指せる手掛かり）。
    #[must_use]
    pub const fn id(&self) -> i64 {
        self.id
    }

    /// どの取り決めの変化か。
    #[must_use]
    pub const fn agreement_id(&self) -> AgreementId {
        self.agreement_id
    }

    /// その取り決めの中身。
    #[must_use]
    pub const fn content(&self) -> &AgreementContent {
        &self.content
    }

    /// その取り決めの種類。
    #[must_use]
    pub const fn kind(&self) -> AgreementKind {
        self.kind
    }

    /// その取り決めの守る側。**落とさない。**
    #[must_use]
    pub const fn keeper(&self) -> AgreementKeeper {
        self.keeper
    }

    /// 変化の前の状態。**`None` は「書かれた」**（無かった → `after`）。
    #[must_use]
    pub const fn before(&self) -> Option<AgreementState> {
        self.before
    }

    /// 変化の後の状態。
    #[must_use]
    pub const fn after(&self) -> AgreementState {
        self.after
    }

    /// いつ変わったか（UTC）。
    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
}

impl Store {
    /// **取り決めを手で書く**（ADR-0051 §決定2・4 / 作業単位 A-12a）。書いたものは有効で始まる。
    ///
    /// 1つのトランザクションで**有効な件数を数える → 上限なら断る → 投影に1行 → 履歴に1行**。
    ///
    /// # 上限に達していたら断る（**黙って古いものを落とさない**）
    ///
    /// 有効な取り決めが [`crate::domain::MAX_ACTIVE_AGREEMENTS`] 件あれば
    /// [`StoreError::AgreementLimitReached`]。**1行も書かれない。**
    /// 数えるのと書くのは同じ `BEGIN IMMEDIATE` の中なので、2つの書き込みが同時に来ても
    /// 上限を1件も越えない（見張り番は `tests/store_agreement.rs::
    /// concurrent_writes_never_exceed_the_limit`）。
    ///
    /// **トランザクションの中で外部通信を待たない**（CLAUDE.md 絶対制約）。SQL しか呼ばない。
    ///
    /// # 存在しないキャラクター
    ///
    /// 投影の INSERT が外部キー違反で止める（`Query`）。**`UnknownCharacter` を名乗らない**
    /// （`replace_character_attributes` と同じ判断。呼び出し側は先にキャラクターを引いている）。
    pub async fn add_agreement(
        &self,
        character_id: CharacterId,
        agreement: &NewAgreement,
    ) -> Result<Agreement, StoreError> {
        let mut tx = self
            .pool
            .begin_with(BEGIN_IMMEDIATE)
            .await
            .map_err(StoreError::Query)?;

        let active = count_active(&mut tx, character_id).await?;
        if !admits_another_active_agreement(active) {
            tracing::debug!(
                character_id = character_id.get(),
                active,
                "有効な取り決めが上限に達しているので、書きませんでした"
            );
            return Err(StoreError::AgreementLimitReached);
        }

        let (id, created_at) = insert_agreement(
            &mut tx,
            character_id,
            agreement,
            AgreementState::Active,
            None,
        )
        .await?;

        tx.commit().await.map_err(StoreError::Query)?;
        tracing::debug!(
            character_id = character_id.get(),
            agreement_id = id,
            "取り決めを書きました"
        );

        Ok(Agreement {
            id: AgreementId::from_row(id),
            content: agreement.content.clone(),
            kind: agreement.kind,
            keeper: agreement.keeper,
            state: AgreementState::Active,
            source_session: None,
            created_at,
        })
    }

    /// **有効な取り決めを手で終わらせる**（ADR-0051 §決め直し 3 —— 終わらせるのは利用者だけ）。
    ///
    /// 1つのトランザクションで**いまの状態を読む → 有効でなければ断る → 投影の状態を書き換える →
    /// 履歴に1行**。**行は消さない**（終わった取り決めは履歴と一覧に残る）。
    ///
    /// # `agreement_id` は生の値で受ける
    ///
    /// パスから来た値で、所有の確認はここでしかしない（`find_session` と同じ立場）。
    /// **その id の行が無いのも、他のキャラクターの行なのも** [`StoreError::UnknownAgreement`]
    /// （どちらもこのキャラクターからは見えない / `UnknownAxisCandidate` と同じ畳み方）。
    ///
    /// # 中身を読まない
    ///
    /// 読むのは状態の列だけ。**中身が壊れた行（DB を直接編集した等）でも終わらせられる**
    /// —— 本文を止めている行を本文から外す道になる（本文は有効な行しか読まない）。
    pub async fn end_agreement(
        &self,
        character_id: CharacterId,
        agreement_id: i64,
        ending: AgreementEnding,
    ) -> Result<AgreementId, StoreError> {
        let mut tx = self
            .pool
            .begin_with(BEGIN_IMMEDIATE)
            .await
            .map_err(StoreError::Query)?;

        let current = stored_state(&mut tx, character_id, agreement_id).await?;
        if !current.is_active() {
            return Err(StoreError::AgreementNotActive);
        }
        move_state(
            &mut tx,
            character_id,
            agreement_id,
            current,
            ending.state(),
            StoreError::AgreementNotActive,
        )
        .await?;

        tx.commit().await.map_err(StoreError::Query)?;
        tracing::debug!(
            character_id = character_id.get(),
            agreement_id,
            state = ending.state().label(),
            "取り決めを終わらせました"
        );
        Ok(AgreementId::from_row(agreement_id))
    }

    /// **提案を捨てる**（提案中 → 却下 / ADR-0051 §決定2 / 作業単位 A-12c-1）。**行は消さない。**
    ///
    /// 1つのトランザクションで**いまの状態を読む → 提案中でなければ断る → 状態を書き換える →
    /// 履歴に1行**。id の扱いと持ち主の縛りは [`Self::end_agreement`] と同じ
    /// （無い・他のキャラクターのものは [`StoreError::UnknownAgreement`]）。
    /// 提案中でなければ [`StoreError::AgreementNotProposed`]。**中身は読まない。**
    ///
    /// 捨てた行は、次からの提案の依頼の「# 提案済み」に載る（同じものを二度出さない手掛かり /
    /// `server::agreement_proposal`）。
    pub async fn reject_agreement(
        &self,
        character_id: CharacterId,
        agreement_id: i64,
    ) -> Result<AgreementId, StoreError> {
        let mut tx = self
            .pool
            .begin_with(BEGIN_IMMEDIATE)
            .await
            .map_err(StoreError::Query)?;

        let current = stored_state(&mut tx, character_id, agreement_id).await?;
        if !current.is_proposed() {
            return Err(StoreError::AgreementNotProposed);
        }
        move_state(
            &mut tx,
            character_id,
            agreement_id,
            current,
            AgreementState::Rejected,
            StoreError::AgreementNotProposed,
        )
        .await?;

        tx.commit().await.map_err(StoreError::Query)?;
        tracing::debug!(
            character_id = character_id.get(),
            agreement_id,
            "取り決めの提案を捨てました"
        );
        Ok(AgreementId::from_row(agreement_id))
    }

    /// **提案を採る**（提案中 → 有効 / ADR-0051 §決定2「採る・直してから採る」/ 作業単位 A-12c-1）。
    ///
    /// 1つのトランザクションで**いまの状態を読む → 提案中でなければ断る → 有効な件数を数える →
    /// 上限なら断る → 中身・種類・守る側（直したものだけ）と状態を書き換える → 履歴に1行**。
    ///
    /// | 断る | 返すもの |
    /// |---|---|
    /// | 無い・他のキャラクターのもの | [`StoreError::UnknownAgreement`] |
    /// | 提案中でない | [`StoreError::AgreementNotProposed`] |
    /// | 有効なものが上限に達している | [`StoreError::AgreementLimitReached`]（**提案中は数えない。採ると数える**） |
    ///
    /// 断ったときは**1行も変わらない**。採った後の姿（直した後の中身）を返す。直していない中身が
    /// 壊れていれば（DB を直接編集した等）[`StoreError::InvalidStoredValue`] で、何も書かない
    /// （壊れた行を有効にして本文を止めない）。
    pub async fn adopt_agreement(
        &self,
        character_id: CharacterId,
        agreement_id: i64,
        edits: &AgreementEdits,
    ) -> Result<Agreement, StoreError> {
        let mut tx = self
            .pool
            .begin_with(BEGIN_IMMEDIATE)
            .await
            .map_err(StoreError::Query)?;

        let current = stored_state(&mut tx, character_id, agreement_id).await?;
        if !current.is_proposed() {
            return Err(StoreError::AgreementNotProposed);
        }
        let active = count_active(&mut tx, character_id).await?;
        if !admits_another_active_agreement(active) {
            tracing::debug!(
                character_id = character_id.get(),
                active,
                "有効な取り決めが上限に達しているので、提案を採りませんでした"
            );
            return Err(StoreError::AgreementLimitReached);
        }

        let row = sqlx::query(ADOPT_AGREEMENT)
            .bind(edits.content.as_ref().map(AgreementContent::as_str))
            .bind(edits.kind.map(AgreementKind::label))
            .bind(edits.keeper.map(AgreementKeeper::label))
            .bind(AgreementState::Active.label())
            .bind(agreement_id)
            .bind(character_id.get())
            .bind(current.label())
            .fetch_optional(&mut *tx)
            .await
            .map_err(StoreError::Query)?;
        // **書きロックを握ったまま読んでいるので、ここで 0 行にはならない。** なっても panic せず断る。
        let Some(row) = row else {
            return Err(StoreError::AgreementNotProposed);
        };
        let adopted = agreement_from_row(character_id, &row)?;

        sqlx::query(INSERT_TRANSITION)
            .bind(agreement_id)
            .bind(current.label())
            .bind(AgreementState::Active.label())
            .execute(&mut *tx)
            .await
            .map_err(StoreError::Query)?;

        tx.commit().await.map_err(StoreError::Query)?;
        tracing::debug!(
            character_id = character_id.get(),
            agreement_id,
            "取り決めの提案を採りました"
        );
        Ok(adopted)
    }

    /// **裏方が見つけた取り決めを、提案中として書く**（ADR-0051 §決定2 / 作業単位 A-12c-1）。
    ///
    /// 1つのトランザクション（`BEGIN IMMEDIATE`）で:
    ///
    /// 1. **生んだ会話が今も在り、このキャラクターのものか**を確かめる（ADR-0049 §決定3）。
    ///    無ければ [`StoreError::TurnSessionGone`]、別のキャラクターのものなら
    ///    [`StoreError::SessionOfAnotherCharacter`]。**1行も書かない。**
    /// 2. 提案を1件ずつ、**同じ中身の提案中・有効が既にあれば足さず**（同じ回の中の重なりも）、
    ///    無ければ提案中で書く（出どころはこの会話）。履歴に「書かれた（無 → 提案中）」を足す。
    ///    **提案中は有効な件数の上限に数えないので、上限では断らない。**
    ///
    /// **トランザクションの中で外部通信を待たない**（CLAUDE.md 絶対制約）。依頼は全部、呼ぶ前に終わっている。
    pub async fn record_agreement_proposals(
        &self,
        character_id: CharacterId,
        session_id: SessionId,
        proposals: &[NewAgreement],
    ) -> Result<AgreementProposalsRecorded, StoreError> {
        let mut tx = self
            .pool
            .begin_with(BEGIN_IMMEDIATE)
            .await
            .map_err(StoreError::Query)?;

        if let Some(refusal) =
            super::session::turn_session_refusal(&mut *tx, character_id, session_id).await?
        {
            return Err(refusal);
        }

        let mut recorded = AgreementProposalsRecorded::default();
        for proposal in proposals {
            let same: i64 = sqlx::query_scalar(COUNT_SAME_CONTENT)
                .bind(character_id.get())
                .bind(proposal.content.as_str())
                .bind(AgreementState::Proposed.label())
                .bind(AgreementState::Active.label())
                .fetch_one(&mut *tx)
                .await
                .map_err(StoreError::Query)?;
            if same > 0 {
                recorded.duplicates += 1;
                continue;
            }
            insert_agreement(
                &mut tx,
                character_id,
                proposal,
                AgreementState::Proposed,
                Some(session_id),
            )
            .await?;
            recorded.written += 1;
        }

        tx.commit().await.map_err(StoreError::Query)?;
        tracing::debug!(
            character_id = character_id.get(),
            session_id = session_id.get(),
            written = recorded.written,
            duplicates = recorded.duplicates,
            "取り決めの提案を書きました"
        );
        Ok(recorded)
    }

    /// そのキャラクターの取り決めを**全部**（有効なものも終わったものも）書いた順で返す。
    ///
    /// **1行でも読めなければ丸ごと `InvalidStoredValue`**（`character_attributes` と同じ側）。
    /// 黙って飛ばすと、一覧から約束が静かに1件消える。
    pub async fn agreements(
        &self,
        character_id: CharacterId,
    ) -> Result<Vec<Agreement>, StoreError> {
        let rows = sqlx::query(SELECT_AGREEMENTS)
            .bind(character_id.get())
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        rows.iter()
            .map(|row| agreement_from_row(character_id, row))
            .collect()
    }

    /// そのキャラクターの**有効な**取り決めを書いた順で返す（**本文が毎ターン読む** / §決定4）。
    ///
    /// **1行でも読めなければ丸ごと `InvalidStoredValue`。** 黙って空として続けると、
    /// キャラクターが理由なく約束を忘れて応答する（属性と同じ判断）。
    /// 止まった本文を戻す道は [`Self::end_agreement`]（中身を読まない）。
    pub async fn active_agreements(
        &self,
        character_id: CharacterId,
    ) -> Result<Vec<Agreement>, StoreError> {
        let rows = sqlx::query(SELECT_ACTIVE)
            .bind(character_id.get())
            .bind(AgreementState::Active.label())
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        rows.iter()
            .map(|row| agreement_from_row(character_id, row))
            .collect()
    }

    /// **そのキャラクターの履歴を、新しい順に最大 `limit` 件**返す。**プロンプトには載らない。**
    ///
    /// **1行でも読めなければ、この読みは丸ごと止まる**（`attribute_revisions` と同じ側）。
    pub async fn agreement_transitions(
        &self,
        character_id: CharacterId,
        limit: u32,
    ) -> Result<Vec<AgreementTransition>, StoreError> {
        let rows = sqlx::query(SELECT_TRANSITIONS)
            .bind(character_id.get())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        rows.iter()
            .map(|row| transition_from_row(character_id, row))
            .collect()
    }

    /// **そのキャラクターの履歴の総数。上限で切らない。** 数えるだけなので壊れた行があっても返る。
    pub async fn count_agreement_transitions(
        &self,
        character_id: CharacterId,
    ) -> Result<u32, StoreError> {
        let counted: i64 = sqlx::query_scalar(COUNT_TRANSITIONS)
            .bind(character_id.get())
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        u32::try_from(counted).map_err(|_| StoreError::InvalidStoredValue)
    }
}

/// **提案を書いた結果**（[`Store::record_agreement_proposals`] / 作業単位 A-12c-1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AgreementProposalsRecorded {
    /// 提案中として書いた件数。
    pub written: usize,
    /// 同じ中身の提案中・有効が既にあったので足さなかった件数。
    pub duplicates: usize,
}

/// 書きロックの中で、**そのキャラクターの有効な取り決めの件数**を数える（上限の判定 / 提案中は数えない）。
async fn count_active(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    character_id: CharacterId,
) -> Result<usize, StoreError> {
    let active: i64 = sqlx::query_scalar(COUNT_ACTIVE)
        .bind(character_id.get())
        .bind(AgreementState::Active.label())
        .fetch_one(&mut **tx)
        .await
        .map_err(StoreError::Query)?;
    // **負の件数は起こりようがないが、panic しない形にしておく**（`count_attribute_revisions` と同じ）。
    usize::try_from(active).map_err(|_| StoreError::InvalidStoredValue)
}

/// 投影に1行・履歴に「書かれた（無 → `state`）」を1行足す（書きロックの中で呼ぶ）。
async fn insert_agreement(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    character_id: CharacterId,
    agreement: &NewAgreement,
    state: AgreementState,
    source_session: Option<SessionId>,
) -> Result<(i64, DateTime<Utc>), StoreError> {
    let row = sqlx::query(INSERT_AGREEMENT)
        .bind(character_id.get())
        .bind(agreement.content.as_str())
        .bind(agreement.kind.label())
        .bind(agreement.keeper.label())
        .bind(state.label())
        .bind(source_session.map(SessionId::get))
        .fetch_one(&mut **tx)
        .await
        .map_err(StoreError::Query)?;
    let id: i64 = row.try_get("id").map_err(StoreError::Query)?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(StoreError::Query)?;

    sqlx::query(INSERT_TRANSITION)
        .bind(id)
        .bind(None::<&str>)
        .bind(state.label())
        .execute(&mut **tx)
        .await
        .map_err(StoreError::Query)?;
    Ok((id, created_at))
}

/// 書きロックの中で、**その取り決めのいまの状態**を読む（持ち主で縛る）。
///
/// 行が無い・他のキャラクターのものは [`StoreError::UnknownAgreement`]、知らない綴りは
/// [`StoreError::InvalidStoredValue`]。**中身は読まない。**
async fn stored_state(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    character_id: CharacterId,
    agreement_id: i64,
) -> Result<AgreementState, StoreError> {
    let stored: Option<String> = sqlx::query_scalar(SELECT_STATE)
        .bind(agreement_id)
        .bind(character_id.get())
        .fetch_optional(&mut **tx)
        .await
        .map_err(StoreError::Query)?;
    let Some(stored) = stored else {
        return Err(StoreError::UnknownAgreement);
    };
    AgreementState::from_label(&stored).ok_or_else(|| {
        tracing::warn!(
            character_id = character_id.get(),
            agreement_id,
            "想定外の状態を持つ取り決めの行があります"
        );
        StoreError::InvalidStoredValue
    })
}

/// 書きロックの中で、状態を `from` から `to` へ書き換え、履歴に1行足す（**状態の列だけ**）。
///
/// `from` の行が無ければ（書きロックを握って読んだ後なので、起きないはず）`refused` を返す。
async fn move_state(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    character_id: CharacterId,
    agreement_id: i64,
    from: AgreementState,
    to: AgreementState,
    refused: StoreError,
) -> Result<(), StoreError> {
    let done = sqlx::query(UPDATE_STATE)
        .bind(to.label())
        .bind(agreement_id)
        .bind(character_id.get())
        .bind(from.label())
        .execute(&mut **tx)
        .await
        .map_err(StoreError::Query)?;
    if done.rows_affected() != 1 {
        return Err(refused);
    }
    sqlx::query(INSERT_TRANSITION)
        .bind(agreement_id)
        .bind(from.label())
        .bind(to.label())
        .execute(&mut **tx)
        .await
        .map_err(StoreError::Query)?;
    Ok(())
}

/// 投影の1行を [`Agreement`] に組み立てる。**panic しない。読み出しでも検証する。**
fn agreement_from_row(character_id: CharacterId, row: &SqliteRow) -> Result<Agreement, StoreError> {
    let id: i64 = row.try_get("id").map_err(StoreError::Query)?;
    let unreadable = |column: &'static str| {
        // **格納値そのものはログにも出さない**（他の拒否と同じ。手掛かりは id と列名）。
        tracing::warn!(
            character_id = character_id.get(),
            agreement_id = id,
            column,
            "読み出せない取り決めの行があります"
        );
        StoreError::InvalidStoredValue
    };

    let content: String = row.try_get("content").map_err(StoreError::Query)?;
    let content = AgreementContent::parse(&content).map_err(|_| unreadable("content"))?;
    let kind: String = row.try_get("kind").map_err(StoreError::Query)?;
    let kind = AgreementKind::from_label(&kind).ok_or_else(|| unreadable("kind"))?;
    let keeper: String = row.try_get("keeper").map_err(StoreError::Query)?;
    let keeper = AgreementKeeper::from_label(&keeper).ok_or_else(|| unreadable("keeper"))?;
    let state: String = row.try_get("state").map_err(StoreError::Query)?;
    let state = AgreementState::from_label(&state).ok_or_else(|| unreadable("state"))?;
    let source_session: Option<i64> = row
        .try_get("source_session_id")
        .map_err(StoreError::Query)?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(StoreError::Query)?;

    Ok(Agreement {
        id: AgreementId::from_row(id),
        content,
        kind,
        keeper,
        state,
        source_session: source_session.map(SessionId::from_row),
        created_at,
    })
}

/// 履歴の1行を [`AgreementTransition`] に組み立てる。**panic しない。読み出しでも検証する。**
fn transition_from_row(
    character_id: CharacterId,
    row: &SqliteRow,
) -> Result<AgreementTransition, StoreError> {
    let id: i64 = row.try_get("id").map_err(StoreError::Query)?;
    let unreadable = |column: &'static str| {
        tracing::warn!(
            character_id = character_id.get(),
            transition_id = id,
            column,
            "読み出せない取り決めの履歴の行があります"
        );
        StoreError::InvalidStoredValue
    };

    let agreement_id: i64 = row.try_get("agreement_id").map_err(StoreError::Query)?;
    let content: String = row.try_get("content").map_err(StoreError::Query)?;
    let content = AgreementContent::parse(&content).map_err(|_| unreadable("content"))?;
    let kind: String = row.try_get("kind").map_err(StoreError::Query)?;
    let kind = AgreementKind::from_label(&kind).ok_or_else(|| unreadable("kind"))?;
    let keeper: String = row.try_get("keeper").map_err(StoreError::Query)?;
    let keeper = AgreementKeeper::from_label(&keeper).ok_or_else(|| unreadable("keeper"))?;
    let before: Option<String> = row.try_get("before").map_err(StoreError::Query)?;
    let before = before
        .map(|stored| AgreementState::from_label(&stored).ok_or_else(|| unreadable("before")))
        .transpose()?;
    let after: String = row.try_get("after").map_err(StoreError::Query)?;
    let after = AgreementState::from_label(&after).ok_or_else(|| unreadable("after"))?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(StoreError::Query)?;

    Ok(AgreementTransition {
        id,
        agreement_id: AgreementId::from_row(agreement_id),
        content,
        kind,
        keeper,
        before,
        after,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // 速報（SQL 文そのものの断片）。**実挙動は tests/store_agreement.rs が持つ。**
    // ---------------------------------------------------------------------

    /// **どの文もプレースホルダで値を渡す**（CLAUDE.md 絶対制約）。
    ///
    /// 状態の綴り（`'active'`）を SQL に直書きする形は、ここで**リテラルとして落ちる**
    /// （綴りの正は `AgreementState::label` の1箇所）。
    #[test]
    fn every_statement_binds_its_values_through_placeholders() {
        let quote = '\'';
        for statement in [
            COUNT_ACTIVE,
            INSERT_AGREEMENT,
            INSERT_TRANSITION,
            SELECT_STATE,
            UPDATE_STATE,
            SELECT_AGREEMENTS,
            SELECT_ACTIVE,
            SELECT_TRANSITIONS,
            COUNT_TRANSITIONS,
            COUNT_SAME_CONTENT,
            ADOPT_AGREEMENT,
        ] {
            assert!(
                !statement.contains(quote),
                "リテラルを埋め込まないこと: {statement}"
            );
        }
    }

    /// **提案を書くトランザクションは、コミットの前に落ちれば何も残さない**（作業単位 A-12c-1 のレビュー 要修正1）。
    ///
    /// 提案の段は次の往復に横取りされうる（`server::extract::MemoryExtractor::spawn` が `abort` する）。
    /// 止まった future は、握っていた `sqlx::Transaction` をコミットせずに落とす。**その落とし方で、書いた行が
    /// 巻き戻ることを、[`Store::record_agreement_proposals`] と同じ部品（`BEGIN IMMEDIATE` → [`insert_agreement`]）で確かめる。**
    /// 投影の行も履歴の行も残らず、次の書き込みはふつうに通る（錠も残らない）。
    #[test]
    fn a_transaction_dropped_before_commit_writes_nothing() {
        use crate::domain::{DisplayName, Handle, Persona, StatScale};
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let dir = tempfile::tempdir().expect("一時ディレクトリ");
            let store = Store::connect(dir.path().join("personacore.sqlite"))
                .await
                .expect("DB を開けること");
            let aoi = store
                .create_character(
                    &Handle::parse("aoi").expect("識別子"),
                    &DisplayName::parse("葵").expect("表示名"),
                    &Persona::parse("落ち着いた口調で話す。").expect("ペルソナ"),
                    None,
                    StatScale::DEFAULT,
                )
                .await
                .expect("キャラクター");
            let session = store.create_session(aoi).await.expect("会話");
            let proposal = NewAgreement {
                content: AgreementContent::parse("利用者は黙って出かけない").expect("中身"),
                kind: AgreementKind::Rule,
                keeper: AgreementKeeper::User,
            };
            {
                let mut tx = store
                    .pool
                    .begin_with(BEGIN_IMMEDIATE)
                    .await
                    .expect("書きロックを取れること");
                insert_agreement(
                    &mut tx,
                    aoi,
                    &proposal,
                    AgreementState::Proposed,
                    Some(session),
                )
                .await
                .expect("トランザクションの中では書けること");
                // ここで落とす（コミットしない）—— 止められた future と同じ。
            }
            assert!(store.agreements(aoi).await.expect("読める").is_empty());
            assert_eq!(
                store
                    .count_agreement_transitions(aoi)
                    .await
                    .expect("数えられる"),
                0
            );
            let recorded = store
                .record_agreement_proposals(aoi, session, &[proposal])
                .await
                .expect("落とした後も書けること（錠が残っていない）");
            assert_eq!(recorded.written, 1);
        });
    }

    /// **採る文は、提案中の1行だけに縛り、直した欄だけを書き換える**（作業単位 A-12c-1）。
    ///
    /// | 欠けると | |
    /// |---|---|
    /// | `WHERE id = ?` | そのキャラクターの提案が全部採られる |
    /// | `AND character_id = ?` | 他のキャラクターの提案を採れる |
    /// | `AND state = ?` | 有効・終わった取り決めの中身を、この文で書き換えられる |
    /// | `coalesce(?, …)` | 直さなかった欄が NULL で潰れる（NOT NULL で止まるが、直しの省略が効かない） |
    #[test]
    fn the_adoption_touches_one_proposed_row_and_keeps_what_is_not_edited() {
        assert_eq!(
            ADOPT_AGREEMENT,
            format!(
                "UPDATE {} SET content = coalesce(?, content), kind = coalesce(?, kind), \
                 keeper = coalesce(?, keeper), state = ? WHERE id = ? AND character_id = ? \
                 AND state = ? RETURNING id, content, kind, keeper, state, source_session_id, \
                 created_at",
                projection_word().to_lowercase()
            )
        );
    }

    /// **状態を書き換える文は、状態の列だけを、有効な1行に縛って書き換える。**
    ///
    /// | 欠けると | |
    /// |---|---|
    /// | `WHERE id = ?` | そのキャラクターの全部の取り決めが終わる |
    /// | `AND character_id = ?` | 他のキャラクターの取り決めを終わらせられる |
    /// | `AND state = ?` | 終わった取り決めを、文の形の上では終わらせ直せる |
    ///
    /// **期待値に表の名前を綴らない**（綴ると、下の走査が書き込みを1つ多く数える）。
    #[test]
    fn the_state_update_touches_one_active_row_and_only_the_state() {
        assert_eq!(
            UPDATE_STATE,
            format!(
                "UPDATE {} SET state = ? WHERE id = ? AND character_id = ? AND state = ?",
                projection_word().to_lowercase()
            )
        );
    }

    /// **履歴を引く文が、持ち主の絞り込み・新しい順・同着の順・上限を全部持つ。**
    #[test]
    fn the_history_query_keeps_its_filter_order_limit_and_time() {
        for fragment in [
            "t.created_at",
            "WHERE a.character_id = ?",
            "ORDER BY t.created_at DESC, t.id DESC",
            "LIMIT ?",
        ] {
            assert!(
                SELECT_TRANSITIONS.contains(fragment),
                "{fragment} が消えています: {SELECT_TRANSITIONS}"
            );
        }
        assert!(COUNT_TRANSITIONS.contains("WHERE a.character_id = ?"));
        assert!(
            !COUNT_TRANSITIONS.contains("LIMIT"),
            "総数を上限で切らないこと"
        );
    }

    /// **書く口と終わらせる口は `BEGIN IMMEDIATE` で入る**（読んでから書くため）。
    #[test]
    fn the_writers_open_an_immediate_transaction() {
        assert_eq!(BEGIN_IMMEDIATE, "BEGIN IMMEDIATE");
    }

    // ---------------------------------------------------------------------
    // SQL の走査（ADR-0051 §決定1「行は消さない」「履歴は追記だけ」）
    //
    // 形は `store::attribute` / `store::axis_candidate` の走査に倣う（各モジュールが自分の写しを持つ
    // 先例どおり）。**判定はこのモジュールの関数だけ**にして、見張り番も対照も同じ関数を通す。
    // ---------------------------------------------------------------------

    /// 走査対象。**`migrations` を外さないこと**（整理する SQL はそちらに書かれるのが最もありそう）。
    const SCAN_ROOTS: [(&str, &str); 2] = [("src", "rs"), ("migrations", "sql")];

    /// **投影へ書いてよい唯一のファイル。**
    const PROJECTION_WRITER: [&str; 3] = ["src", "store", "agreement.rs"];

    /// 投影のテーブル名（大文字）を**ソース上に綴らずに**組み立てる。
    fn projection_word() -> String {
        ["character", "_agreement"].concat().to_uppercase()
    }

    /// 履歴のテーブル名（大文字）。
    fn history_word() -> String {
        ["agreement", "_transition"].concat().to_uppercase()
    }

    /// 走査対象を「パス, ソース」で返す。**`.sql` は行コメントを落としてから。**
    fn scan_targets() -> Vec<(String, String)> {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut targets = Vec::new();
        for (root, extension) in SCAN_ROOTS {
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
    fn tokens_of(source: &str) -> Vec<String> {
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

    /// 違反メッセージに載せる抜粋（**判定には使わない**）。
    fn excerpt(tokens: &[String], index: usize) -> String {
        let from = index.saturating_sub(4);
        let to = (index + 9).min(tokens.len());
        tokens[from..to].join(" ")
    }

    /// 表名の前にある**意味のある語**を近い順に返す（スキーマ修飾・競合節を読み飛ばす）。
    fn significant_before(tokens: &[String], index: usize) -> Vec<&str> {
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
        let mut found = Vec::new();
        let mut at = index;
        while at > 0 && found.len() < 3 {
            at -= 1;
            let token = tokens[at].as_str();
            if NOISE.contains(&token) {
                continue;
            }
            found.push(token);
        }
        found
    }

    /// **表名が「行を変える文」の位置に居るか。** 居るならその動詞を返す
    /// （位置の表は `store::attribute::attribute_write_keyword` と同じ）。
    fn write_keyword(tokens: &[String], index: usize) -> Option<&'static str> {
        let before = significant_before(tokens, index);
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

    /// **投影を書き換える文の走査本体**（動詞を返す）。見張り番も対照も**この関数を通す**こと。
    fn projection_writes(source: &str, table: &str) -> Vec<&'static str> {
        source
            .split(';')
            .flat_map(|statement| {
                let tokens = tokens_of(statement);
                tokens
                    .iter()
                    .enumerate()
                    .filter(|(_, token)| token.as_str() == table)
                    .filter_map(|(index, _)| write_keyword(&tokens, index))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// 履歴の言及の近くにあってはならない語（`store::attribute` の履歴の走査と同じ6語）。
    const HISTORY_DESTRUCTIVE: [&str; 6] = [
        "UPDATE", "REPLACE", "DELETE", "DROP", "TRUNCATE", "CONFLICT",
    ];

    /// 文中に破壊的な語があるか。**外部キーの参照アクション（`ON DELETE CASCADE`）だけは除く。**
    fn destructive_in(tokens: &[String]) -> Option<&'static str> {
        for (index, token) in tokens.iter().enumerate() {
            let reference_action = token.as_str() == "DELETE"
                && index > 0
                && tokens[index - 1] == "ON"
                && tokens.get(index + 1).map_or("", String::as_str) != "FROM";
            if reference_action {
                continue;
            }
            if let Some(found) = HISTORY_DESTRUCTIVE
                .iter()
                .find(|word| **word == token.as_str())
            {
                return Some(found);
            }
        }
        None
    }

    /// 履歴の言及が**許可された形**か。**既定は拒否**（追記・読み出し・DDL だけ）。
    fn history_form_is_allowed(tokens: &[String], index: usize) -> bool {
        let before = |back: usize| -> &str {
            index
                .checked_sub(back)
                .and_then(|at| tokens.get(at))
                .map_or("", String::as_str)
        };
        match before(1) {
            "INTO" => before(2) == "INSERT",
            "FROM" => true,
            "TABLE" => before(2) == "CREATE",
            "ON" => before(3) == "INDEX",
            "REFERENCES" => true,
            _ => false,
        }
    }

    /// **履歴を書き換える文の走査本体。** 見張り番も対照も**この関数を通す**こと。
    fn history_violations(source: &str, table: &str) -> Vec<String> {
        source
            .split(';')
            .flat_map(|statement| {
                let tokens = tokens_of(statement);
                let mut found = Vec::new();
                for (index, token) in tokens.iter().enumerate() {
                    if token.as_str() != table {
                        continue;
                    }
                    if let Some(keyword) = destructive_in(&tokens) {
                        found.push(format!(
                            "同じ文に {keyword} があります: {}",
                            excerpt(&tokens, index)
                        ));
                    } else if !history_form_is_allowed(&tokens, index) {
                        found.push(format!(
                            "許可されていない形です: {}",
                            excerpt(&tokens, index)
                        ));
                    }
                }
                found
            })
            .collect()
    }

    /// **投影の行を消す・書き換える文は、唯一の書き手の中の INSERT と UPDATE だけ。**
    ///
    /// * 唯一の書き手の外には、書く文が1つも無い（履歴を通らない変更を作らせない）。
    /// * 唯一の書き手の中でも、**消す文（DELETE / DROP / TRUNCATE / REPLACE）が無い**
    ///   —— 行は消さない（ADR-0051 §決定1）。`REPLACE` は競合行を消してから入れる。
    /// * 唯一の書き手は、現に INSERT を1つ・UPDATE を2つ持つ（判定が何も拾えなくなったら赤）。
    ///   UPDATE は状態だけを書き換える [`UPDATE_STATE`]（終わらせる・捨てる）と、提案中の1行を
    ///   採る [`ADOPT_AGREEMENT`]（作業単位 A-12c-1 で増えた）。
    #[test]
    fn the_projection_is_written_only_here_and_never_deleted() {
        let table = projection_word();
        let writer = PROJECTION_WRITER.join(std::path::MAIN_SEPARATOR_STR);
        let targets = scan_targets();
        assert!(targets.len() >= 5, "走査対象が少なすぎる");

        let mut outside = Vec::new();
        let mut inside = Vec::new();
        for (path, source) in &targets {
            let writes = projection_writes(source, &table);
            if path.ends_with(&writer) {
                inside.extend(writes);
            } else {
                outside.extend(writes.into_iter().map(|verb| format!("{path}: {verb}")));
            }
        }
        assert!(
            outside.is_empty(),
            "投影へ書く文が唯一の書き手の外にあります: {outside:?}"
        );
        inside.sort_unstable();
        assert_eq!(
            inside,
            vec!["INSERT", "UPDATE", "UPDATE"],
            "書き手の中の書き込みは、1件の追加・状態の書き換え・提案の採用だけであること"
        );
    }

    /// **履歴を書き換える文が、どこにも無い**（追記のみ）。
    #[test]
    fn no_statement_updates_or_deletes_a_transition() {
        let table = history_word();
        let targets = scan_targets();
        assert!(targets.len() >= 5, "走査対象が少なすぎる");

        let mut mentions = 0;
        let mut found = Vec::new();
        for (path, source) in &targets {
            mentions += tokens_of(source)
                .iter()
                .filter(|token| token.as_str() == table)
                .count();
            for violation in history_violations(source, &table) {
                found.push(format!("{path}: {violation}"));
            }
        }
        assert!(
            found.is_empty(),
            "履歴を書き換えうる文があります:\n{}",
            found.join("\n")
        );
        // 内訳は追記・読み出し（2文）・数え上げと、マイグレーションの CREATE TABLE / CREATE INDEX。
        assert!(
            mentions >= 5,
            "テーブル名の言及を拾えていない: {mentions} 件"
        );
    }

    /// **対照: 走査本体が、既知の書き込みの形を落とす**（植えたソース。LF / CRLF で同じ答え）。
    #[test]
    fn the_scans_catch_planted_writes() {
        let projection = projection_word();
        let name = projection.to_lowercase();
        for (planted, verb) in [
            (format!("DELETE FROM {name} WHERE id = ?"), "DELETE"),
            (format!("DELETE\nFROM main.{name}"), "DELETE"),
            (
                format!("INSERT OR REPLACE INTO {name} (id) VALUES (?)"),
                "INSERT",
            ),
            (format!("REPLACE INTO {name} (id) VALUES (?)"), "INSERT"),
            (
                format!("UPDATE OR REPLACE {name} SET content = ?"),
                "REPLACE",
            ),
            (format!("UPDATE {name}\nSET content = ?"), "UPDATE"),
            (format!("DROP TABLE {name}"), "DROP"),
            (
                format!("PRAGMA foreign_keys = ON;\nDELETE FROM {name};"),
                "DELETE",
            ),
        ] {
            assert_eq!(
                projection_writes(&planted, &projection),
                vec![verb],
                "{planted}"
            );
            let crlf = planted.replace('\n', "\r\n");
            assert_eq!(
                projection_writes(&crlf, &projection),
                vec![verb],
                "{planted}"
            );
        }
        for allowed in [
            format!("SELECT id FROM {name} WHERE character_id = ?"),
            format!("CREATE TABLE {name} (id INTEGER)"),
            format!("/// `{name}` は投影である。"),
        ] {
            assert!(
                projection_writes(&allowed, &projection).is_empty(),
                "{allowed}"
            );
        }

        let history = history_word();
        let name = history.to_lowercase();
        for planted in [
            format!("UPDATE {name} SET after = ? WHERE id = ?"),
            format!("DELETE FROM main.{name}"),
            format!("INSERT OR REPLACE INTO {name} (agreement_id) VALUES (?)"),
            format!("INSERT INTO {name} (id) VALUES (?) ON CONFLICT (id) DO UPDATE SET after = ?"),
            format!("DROP TABLE {name}"),
            format!("PRAGMA table_info({name})"),
            format!("/// `{name}` は追記のみ。"),
        ] {
            assert!(
                !history_violations(&planted, &history).is_empty(),
                "{planted}"
            );
            let crlf = planted.replace('\n', "\r\n");
            assert!(!history_violations(&crlf, &history).is_empty(), "{planted}");
        }
        for allowed in [
            format!("INSERT INTO {name} (agreement_id, before, after) VALUES (?, ?, ?)"),
            format!("SELECT id FROM {name} AS t WHERE t.agreement_id = ?"),
            format!(
                "CREATE TABLE {name} (agreement_id INTEGER REFERENCES x(id) ON DELETE CASCADE)"
            ),
            format!("CREATE INDEX {name}_idx ON {name} (agreement_id, id)"),
        ] {
            assert!(
                history_violations(&allowed, &history).is_empty(),
                "{allowed}"
            );
        }
    }
}
