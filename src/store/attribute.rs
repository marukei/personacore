//! `character_attribute` の読み書きと、**その変更の履歴**
//! （ADR-0009 / 作業単位 12a ＋ [ADR-0028](../../docs/adr/0028-attributes-that-change.md)
//! §決定5 / 作業単位 A-3a）。
//!
//! この層は**薄く保つ**。キー・値の検証と正規化、件数の上限は `domain` 側の
//! `AttributeKey::parse` / `AttributeValue::parse` / `AttributeSet::parse` で済んでおり、
//! 差分の取り方も `domain::attribute_changes`（純粋関数）にある。
//! ここは「検証済みの値を SQL にバインドする」だけを担う。
//!
//! **`&str` も裸の `Vec<CharacterAttribute>` も受け取らない。**
//! 前者は未検証の文字列が DB に届く経路になり、後者は
//! **件数の上限を通っていない集合を書ける経路**になる（`AttributeSet` の doc）。
//!
//! # 表が2枚ある（ADR-0028 §決定5）
//!
//! | 表 | 立場 |
//! |---|---|
//! | 履歴（追記のみ） | **正**。変更を1件ずつ足すだけで、更新も削除もしない |
//! | 現在値 | **投影**。履歴の「いま」を写したもの。プロンプトが読むのはこちら |
//!
//! **書き込みの経路は [`Store::replace_character_attributes`] ひとつ。**
//! 2箇所に同じ事実が載る形なので（CLAUDE.md「同じ発見を2箇所に書かない」）、
//! **投影を導出のままに保つ見張り番**が要る——このファイルの
//! `no_statement_outside_the_one_writer_touches_the_projection` が
//! `src/**` と `migrations/**` を走査して、書く文がここ以外に無いことを見る。

use chrono::{DateTime, Utc};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::memory::{subject_from_db, subject_to_db};
use super::{Store, StoreError};
use crate::domain::{
    AttributeKey, AttributeSet, AttributeValue, CharacterAttribute, CharacterId, Subject,
    attribute_changes,
};

/// そのキャラクターの属性を**設定時の順序**で引く。
///
/// `ORDER BY rowid` は挿入順（`stat_alloc` と同じ）。置き換えが丸ごと入れ直すので、
/// 呼び出し側が渡した並びがそのまま返る。
///
/// # **`stat_alloc` と違って `LEFT JOIN` しない**
///
/// あちらは値の検証に**親の列**（`character.stat_scale`）が要るため、
/// 親を引けないこと自体を `InvalidStoredValue` にする必要があった。
/// 属性は親の列を1つも使わないので、結合する理由が無い。
///
/// **結合しない代わりに、孤児は「見えない」のではなく「引かれない」。**
/// 外部キーを倒して親を消せば行だけが残るが、`character_id` で引く限り
/// **その id を持つキャラクターが再び現れない限り誰にも読まれない**
/// （`character.id` は `AUTOINCREMENT` ではないので id の再利用は起こりうる。
/// これは `stat_alloc` / `memory` と共通の性質で、この作業単位で変えていない）。
const SELECT_ATTRIBUTES: &str = "SELECT subject, key, value FROM character_attribute \
     WHERE character_id = ? ORDER BY rowid";

/// 丸ごと消す（置き換えの前半）。**`character_id` で縛る**——縛らないと他のキャラクターの
/// 属性まで消える。
const DELETE_ATTRIBUTES: &str = "DELETE FROM character_attribute WHERE character_id = ?";

/// 1件挿入する（置き換えの後半）。
const INSERT_ATTRIBUTE: &str =
    "INSERT INTO character_attribute (character_id, subject, key, value) VALUES (?, ?, ?, ?)";

/// **読みから始まって書きに昇格するトランザクションの開始文**（ADR-0028 §決定5）。
///
/// 置き換えは**現在の集合を読んでから書く**形になったので、素の `BEGIN`（DEFERRED）だと
/// 最初の読みで読み取りスナップショットが固定され、続く書きへの昇格が
/// `SQLITE_BUSY_SNAPSHOT`(517) になる。**`busy_timeout` の対象外で、待たずに即座に失敗する。**
///
/// **WAL のスナップショットは DB 全体を指す**ので、
/// 「同じキャラクターへの置き換えと競合する場合だけ」の話ではない
/// ——**無関係な書き込みが1つ commit されただけで昇格が落ちる**
/// （実測と壊れ方は `store::character` の同名の定数の doc。0011a-2 で 315 回中 298 回）。
///
/// **`store::character` / `store::session` の定数と共有しない。** 中身は同じ文字列だが、
/// 「なぜここで要るのか」は関数ごとに別で、共有すると片方の理由が消えたときに
/// もう片方も一緒に外れる。
///
/// 見張り番は `tests/store_attribute.rs::two_replacements_never_lose_their_snapshot`
/// （**別のキャラクターを相手にした置き換えを同時に走らせ、両方が成功する**ことを見る）。
const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

/// **履歴を1件足す文**（ADR-0028 §決定5）。**追記のみ。**
///
/// `id` / `created_at` はスキーマの DEFAULT に任せる。
///
/// # 更新も削除も対になる文を持たない
///
/// 履歴は「属性がいつ何から何に変わったか」の記録で、後から書き換えると
/// **記録としての意味が消える**（票の表と同じ立場 / ADR-0026 §決定3）。
/// **見張り番は `no_statement_updates_or_deletes_a_revision`**
/// （`src/**` と `migrations/**` を走査する）。
/// **走査対象のファイルにテーブル名を綴らないこと**——綴ると走査が自分自身を落とす
/// （`store::memory::table_word` と同じ作法。だから doc では「履歴の表」と呼んでいる）。
const INSERT_REVISION: &str = "INSERT INTO attribute_revision \
     (character_id, subject, key, before, after) VALUES (?, ?, ?, ?, ?)";

/// **そのキャラクターの履歴を新しい順に**引く（ADR-0028 §決定5）。
///
/// # 第2キーに `id` を置く（`SELECT_ACTIVE_MEMORIES` / 票の履歴と同じ理由）
///
/// `created_at` はミリ秒精度で**同値になりうる**。
/// **1回の置き換えで最大 16 件が同じトランザクションで入る**ので、同着は本番で普通に起きる。
/// 同着の順が挿入順まかせになると、同じ置き換えの履歴が回ごとに違う並びで読める。
///
/// # 上限を SQL 側で切る
///
/// 履歴は消えないので、全件読むと**読む量が編集回数に比例して増える。**
/// 並べ替えを呼び出し側の仕事にしない——**どちらの順で返すかを1箇所に決める。**
const SELECT_REVISIONS: &str = "SELECT id, subject, key, before, after, created_at \
     FROM attribute_revision WHERE character_id = ? ORDER BY created_at DESC, id DESC LIMIT ?";

/// **そのキャラクターの履歴の総数**（作業単位 A-3a のレビューの是正）。
///
/// # なぜ「返した件数」で代用しないのか
///
/// 一覧は上限で切られている（[`SELECT_REVISIONS`] の `LIMIT`）ので、
/// 件数を総数と呼ぶと**画面が「これで全部」と嘘をつく。**
/// **見えていない件数は、総数との差でしか読めない。**
///
/// **上限で切らない。** 読めない行も数に入る（数え上げは主語も値も時刻も読まない）
/// ——一覧が 500 で止まっていても、この数は返る。
const COUNT_REVISIONS: &str =
    "SELECT count(*) AS revisions FROM attribute_revision WHERE character_id = ?";

/// **属性の変更1件**（`store` の外へ出る形 / ADR-0028 §決定5）。
///
/// `domain::AttributeChange` に `id` と `created_at` が付いたもの。
/// **包んでいるのは、`store` の外へ「保存済みの1行」として出すため**で、
/// 差分の意味（`before` / `after` の `None` が何を指すか）はあちらの doc が正。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeRevision {
    /// 主キー。**人が `sqlite3` で同じ行を指せる唯一の手掛かり。**
    id: i64,
    /// 誰についての属性か。**落とさない**（ADR-0028 §決定4）。
    subject: Subject,
    /// どのキーの変更か。**読み出しの検証を通っている。**
    key: AttributeKey,
    /// 変更前の値。`None` は「無かった」。
    before: Option<AttributeValue>,
    /// 変更後の値。`None` は「消えた」。
    after: Option<AttributeValue>,
    /// いつ変わったか。**この表の存在理由**（いつ静かに変わったのかを読む）。
    created_at: DateTime<Utc>,
}

impl AttributeRevision {
    /// 主キー。
    #[must_use]
    pub const fn id(&self) -> i64 {
        self.id
    }

    /// 誰についての属性か。
    #[must_use]
    pub const fn subject(&self) -> Subject {
        self.subject
    }

    /// どのキーの変更か。
    #[must_use]
    pub const fn key(&self) -> &AttributeKey {
        &self.key
    }

    /// 変更前の値（`None` は「無かった」）。
    #[must_use]
    pub const fn before(&self) -> Option<&AttributeValue> {
        self.before.as_ref()
    }

    /// 変更後の値（`None` は「消えた」）。
    #[must_use]
    pub const fn after(&self) -> Option<&AttributeValue> {
        self.after.as_ref()
    }

    /// いつ変わったか（UTC）。
    #[must_use]
    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
}

/// **投影に保存されている行を、差分に使える形へ均したもの。**
///
/// 置き換えは**既存の行を読む**ようになったが、**読めない行があっても届かせる**
/// （修復の唯一の入口を塞がない / [`Store::replace_character_attributes`] の doc）。
/// そのため「読めた行」と「読めなかった行」を分けて持つ。
#[derive(Debug, Default)]
struct StoredProjection {
    /// 主語・キー・値の全部が読めた行。**差分の突き合わせに使う。**
    readable: Vec<CharacterAttribute>,
    /// 主語とキーは読めたが**値が読めない**行。
    /// この組に触る変更は履歴に書けない（`before` に入れる値が門を通らない）。
    opaque: Vec<(Subject, AttributeKey)>,
    /// **主語かキーが読めない**行の数。組として指せないので、数だけ持つ。
    unnameable: usize,
}

impl StoredProjection {
    /// その組の値が読めなかったか（＝履歴に書けないか）。
    fn is_opaque(&self, subject: Subject, key: &AttributeKey) -> bool {
        self.opaque
            .iter()
            .any(|(held, name)| *held == subject && name == key)
    }

    /// 読めなかった行の総数。
    fn unreadable(&self) -> usize {
        self.opaque.len() + self.unnameable
    }
}

impl Store {
    /// キャラクターの属性を**丸ごと置き換える**（`replace_stat_allocs` と同じ意味論）。
    ///
    /// 要求に含まれない属性は消える。空の集合を渡すと全属性が消える。
    /// 全消し → 挿入を**1つのトランザクション**で行うので、途中の状態は観測されない。
    ///
    /// # 件数の上限は型が通っている
    ///
    /// `AttributeSet` は `parse` を通ってしか作れず、`parse` が
    /// `AttributeSet::MAX_LEN` を超える集合を拒否する。**ここで数え直さない。**
    /// 数え直すと上限の写しが 2 箇所になり、片方だけ変えたときに黙ってずれる。
    ///
    /// # **ここが唯一の書き手**（ADR-0028 §決定5）
    ///
    /// 1つのトランザクションで
    /// **現在の集合を読む → 差分を履歴へ追記する → 投影を差し替える**を行う。
    ///
    /// > **⚠ 守れていないこと —— 原子性には、この綴り以上の権威が無い。**
    /// > 走査の見張り番（`no_statement_outside_the_one_writer_touches_the_projection`）が
    /// > 見ているのは**投影へ書く文が、このファイルの外に無いこと**であって、
    /// > **履歴と投影が同じトランザクションで動くこと**ではない。
    /// > **実測（2026-09-09）: 履歴の追記を `tx.commit()` の後ろへ出し、
    /// > プールに対する自動コミットで実行しても全緑だった**
    /// > （レビューが 2046 全緑、是正の回に測り直して **2049 全緑・exit 0**）。
    /// > **殺す見張り番は作れていない**——履歴の INSERT だけを失敗させる公開経路が無く、
    /// > ロック競合で作る形は**時間に寄りかかる**（時間に寄りかかる見張り番は足さないと決めた）。
    /// > **窓が開くと何が起きるか**: 投影だけが新しくなり、
    /// > **その変更の行が履歴に無い**——「静かに変わったもの」が1件、記録から落ちる。
    ///
    /// **差分ゼロなら履歴に1行も書かない。** 何も変えていない `PUT` で履歴が埋まると、
    /// 「静かに変わったもの」を見つけるための表という存在理由が消える。
    ///
    /// **トランザクションの中で外部通信を待たない**（CLAUDE.md 絶対制約）。
    /// この関数は SQL しか呼ばない。
    ///
    /// # **既存の行を読むが、読めなくても届く**（修復の入口を塞がない）
    ///
    /// 上限を超えるキーや未知の `subject` が既に保存されていても、この関数は届く
    /// ——読めない行は**差分の対象から外す**だけで、置き換えそのものは止めない。
    /// `character_attributes` は 1 行壊れると丸ごと失敗するので、
    /// **壊れた行の復旧経路はここだけ**（見張り番は `tests/store_attribute.rs::
    /// an_over_long_stored_key_stops_the_read_and_the_replacement_fixes_it`）。
    ///
    /// > **⚠ 守れていないこと —— 壊れた行の修復は履歴に残らない。**
    /// > 履歴の `key` / `before` / `after` は投影と**同じ門**（長さ・NUL・空白・型）を持つので、
    /// > 上限を超えた値をそのまま `before` に書くと**履歴の INSERT が落ちる。**
    /// > 落ちれば置き換えごとロールバックされ、**唯一の修復経路が閉じる。**
    /// > そこで**記録できない変更は記録せず、`warn` を残して修復を通す。**
    /// > 見張り番は `tests/store_attribute.rs::
    /// > repairing_a_broken_row_is_not_recorded_but_still_works`。
    ///
    /// # `BEGIN IMMEDIATE` で入る
    ///
    /// 現在の集合を**読んでから書く**形なので、素の `BEGIN` では
    /// `SQLITE_BUSY_SNAPSHOT` になる（実測と壊れ方は [`BEGIN_IMMEDIATE`] の doc）。
    /// **A-3a より前は素の `BEGIN` で正しかった**——最初の文が DELETE だったので
    /// 昇格が起きなかった。**読みを足した時点でその前提が消えている。**
    ///
    /// # 存在しないキャラクター
    ///
    /// 集合が空なら成功する（読みが 0 件、DELETE が 0 件で成功し、INSERT が 1 件も走らない）。
    /// 空でなければ INSERT が外部キー違反で止める。**ここで `UnknownCharacter` を
    /// 名乗らない**のは、「宛先が無い」ことの報告を 1 箇所（外部キー）に保つため
    /// ——`replace_stat_allocs` と揃えてある。
    pub async fn replace_character_attributes(
        &self,
        character_id: CharacterId,
        attributes: &AttributeSet,
    ) -> Result<(), StoreError> {
        let mut tx = self
            .pool
            .begin_with(BEGIN_IMMEDIATE)
            .await
            .map_err(StoreError::Query)?;

        let rows = sqlx::query(SELECT_ATTRIBUTES)
            .bind(character_id.get())
            .fetch_all(&mut *tx)
            .await
            .map_err(StoreError::Query)?;
        let current = projection_from_rows(&rows);

        // **差分の取り方は `domain` の純粋関数。** ここで組み直さない。
        let mut changes = attribute_changes(&current.readable, attributes.as_slice());
        // **履歴に書けない組は落とす**（上の ⚠ / 修復を通すため）。
        changes.retain(|change| !current.is_opaque(change.subject, &change.key));

        for change in &changes {
            sqlx::query(INSERT_REVISION)
                .bind(character_id.get())
                // **綴りは `store::memory` と共有する**（写しを作らない）。
                .bind(subject_to_db(change.subject))
                .bind(change.key.as_str())
                .bind(change.before.as_ref().map(AttributeValue::as_str))
                .bind(change.after.as_ref().map(AttributeValue::as_str))
                .execute(&mut *tx)
                .await
                .map_err(StoreError::Query)?;
        }

        sqlx::query(DELETE_ATTRIBUTES)
            .bind(character_id.get())
            .execute(&mut *tx)
            .await
            .map_err(StoreError::Query)?;

        for attribute in attributes.as_slice() {
            sqlx::query(INSERT_ATTRIBUTE)
                .bind(character_id.get())
                // **綴りは `store::memory` と共有する**（写しを作らない）。
                .bind(subject_to_db(attribute.subject))
                .bind(attribute.key.as_str())
                .bind(attribute.value.as_str())
                .execute(&mut *tx)
                .await
                .map_err(StoreError::Query)?;
        }

        tx.commit().await.map_err(StoreError::Query)?;
        tracing::debug!(
            character_id = character_id.get(),
            attributes = attributes.len(),
            revisions = changes.len(),
            "キャラクター属性を置き換えました"
        );
        if current.unreadable() > 0 {
            // **格納値そのものはログにも出さない**（他の拒否と同じ。手掛かりは id と件数）。
            tracing::warn!(
                character_id = character_id.get(),
                unreadable = current.unreadable(),
                "読めない属性の行を置き換えました。その行の変更は履歴に残せません"
            );
        }
        Ok(())
    }

    /// **そのキャラクターの属性の履歴を、新しい順に最大 `limit` 件**返す
    /// （ADR-0028 §決定5 / 作業単位 A-3a）。
    ///
    /// 使うのは管理 API（**人が読む**）。**プロンプトには一切載らない**
    /// ——載せると、削れない枠が編集回数に比例して膨らむ。
    ///
    /// # **1行でも読めなければ、この読みは丸ごと止まる**
    ///
    /// `recall_memories` / `drift_vote_history` と同じ側に倒してある。
    /// **黙って飛ばすと、履歴から静かに1行消える**——
    /// **「静かに変わったことに気付く」ための表**（§決定5）なのだから、
    /// その表から静かに消えるのが最も悪い壊れ方である。
    ///
    /// | | |
    /// |---|---|
    /// | **止まるもの** | この一覧だけ |
    /// | **止まらないもの** | 属性そのものの読み・**置き換え**・本文の応答・記憶・変容 |
    ///
    /// **止まっても履歴は溜まり続ける**ので、壊れた行を直せば続きから読める。
    /// 直す導線は `sqlite3` だけ——**履歴に更新も削除も無い**（追記のみ）。
    ///
    /// 見張り番は `tests/store_attribute.rs::a_broken_history_row_stops_only_the_listing`。
    pub async fn attribute_revisions(
        &self,
        character_id: CharacterId,
        limit: u32,
    ) -> Result<Vec<AttributeRevision>, StoreError> {
        let rows = sqlx::query(SELECT_REVISIONS)
            .bind(character_id.get())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let mut revisions = Vec::with_capacity(rows.len());
        for row in &rows {
            revisions.push(revision_from_row(character_id, row)?);
        }
        tracing::debug!(
            character_id = character_id.get(),
            revisions = revisions.len(),
            "キャラクター属性の履歴を読み出しました"
        );
        Ok(revisions)
    }

    /// **そのキャラクターの履歴の総数**（作業単位 A-3a のレビューの是正）。**上限で切らない。**
    ///
    /// 数えるだけなので、主語も値も時刻も読まない
    /// ——**壊れた行があっても総数は返る**（止まるのは一覧の側だけ）。
    ///
    /// 実挙動の権威は `tests/server_admin_attributes.rs::
    /// only_the_newest_revisions_come_back_but_the_total_counts_them_all`
    /// （**上限を超える履歴を積んだ状態**でしか、総数と件数のすり替えは観測できない）。
    pub async fn count_attribute_revisions(
        &self,
        character_id: CharacterId,
    ) -> Result<u32, StoreError> {
        let counted: i64 = sqlx::query_scalar(COUNT_REVISIONS)
            .bind(character_id.get())
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        // **`try_get::<u32>` にしない**（`count_drift_votes` と同じ理由）。
        // 収まらない状態はこの表では起こりようがないが、**panic しない形**にしておく。
        u32::try_from(counted).map_err(|_| StoreError::InvalidStoredValue)
    }

    /// そのキャラクターの属性を**設定時の順序**で返す。
    ///
    /// **`subject` を落とさない**（CLAUDE.md 絶対制約と同じ理由が属性にも効く /
    /// ADR-0009）。想定外の綴りが格納されていたら `InvalidStoredValue`。
    ///
    /// 前後に空白のあるキー・値が格納されていた場合は正規化して返す
    /// （`stat_allocs` の軸名と同じ扱い）。正規化しても不正なら `InvalidStoredValue`。
    ///
    /// # **1 行壊れると丸ごと失敗する**
    ///
    /// `memory_from_row` / `stat_allocs` と同じ方針。復旧は
    /// `replace_character_attributes`（= `PUT /admin/characters/{handle}/attributes`）で、
    /// **あちらは読めない行があっても止まらないので届く。**
    ///
    /// # **件数はここでは数えない**
    ///
    /// DB を直接編集して `AttributeSet::MAX_LEN` を超える行を入れることは今も可能だが、
    /// 読み出しでは弾かない。弾くと**何件入っているのかを API から見る手段が消え**、
    /// 直したい人ほど状況を確認できなくなる。上限が守っているのはプロンプトの予算で、
    /// **属性をプロンプトに載せるのは 12b の担当**（この時点では誰も載せていない）。
    pub async fn character_attributes(
        &self,
        character_id: CharacterId,
    ) -> Result<Vec<CharacterAttribute>, StoreError> {
        let rows = sqlx::query(SELECT_ATTRIBUTES)
            .bind(character_id.get())
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let mut attributes = Vec::with_capacity(rows.len());
        for row in rows {
            let stored_subject: String = row.try_get("subject").map_err(StoreError::Query)?;
            let Some(subject) = subject_from_db(&stored_subject) else {
                // **格納値そのものはログにも出さない**（他の拒否と同じ。手掛かりは id）。
                tracing::warn!(
                    character_id = character_id.get(),
                    "想定外の subject を持つ属性の行があります。置き換えで直せます"
                );
                return Err(StoreError::InvalidStoredValue);
            };

            let stored_key: String = row.try_get("key").map_err(StoreError::Query)?;
            let key = AttributeKey::parse(&stored_key).map_err(|_| {
                tracing::warn!(
                    character_id = character_id.get(),
                    "読み出せないキーを持つ属性の行があります。置き換えで直せます"
                );
                StoreError::InvalidStoredValue
            })?;

            let stored_value: String = row.try_get("value").map_err(StoreError::Query)?;
            let value = AttributeValue::parse(&stored_value).map_err(|_| {
                tracing::warn!(
                    character_id = character_id.get(),
                    "読み出せない値を持つ属性の行があります。置き換えで直せます"
                );
                StoreError::InvalidStoredValue
            })?;

            attributes.push(CharacterAttribute {
                subject,
                key,
                value,
            });
        }

        Ok(attributes)
    }
}

/// 投影の行を、差分に使える形へ均す。**読めない行があってもエラーにしない。**
///
/// `character_attributes`（読み出しの入口）とは**わざと方針が逆**である。
///
/// | | 読み出し | ここ |
/// |---|---|---|
/// | 読めない行 | **丸ごと `InvalidStoredValue`** | **数えて、差分から外す** |
/// | 理由 | 欠けた属性で喋らせない | **修復の唯一の入口を塞がない** |
///
/// **デコードの失敗（BLOB 等）も同じ扱い。** `?` で外へ返すと、
/// `PRAGMA` で作られた1行が置き換えごと止めてしまう。
///
/// **`Result` を返さない。** 返す形にすると、後から「ここで止める」枝が生えて
/// 修復の入口が静かに閉じる。**この関数は失敗しない**ことを型で言っておく。
fn projection_from_rows(rows: &[SqliteRow]) -> StoredProjection {
    let mut projection = StoredProjection::default();

    for row in rows {
        // **主語とキーが読めない行は、組として指せない。** 数だけ持つ。
        let Ok(stored_subject) = row.try_get::<String, _>("subject") else {
            projection.unnameable += 1;
            continue;
        };
        let Some(subject) = subject_from_db(&stored_subject) else {
            projection.unnameable += 1;
            continue;
        };
        let Ok(stored_key) = row.try_get::<String, _>("key") else {
            projection.unnameable += 1;
            continue;
        };
        let Ok(key) = AttributeKey::parse(&stored_key) else {
            projection.unnameable += 1;
            continue;
        };

        // 値だけが読めない行は、**組は分かる**ので履歴の対象から外すために覚えておく。
        let value = row
            .try_get::<String, _>("value")
            .ok()
            .and_then(|stored| AttributeValue::parse(&stored).ok());
        match value {
            Some(value) => projection.readable.push(CharacterAttribute {
                subject,
                key,
                value,
            }),
            None => projection.opaque.push((subject, key)),
        }
    }

    projection
}

/// 履歴の1行を [`AttributeRevision`] に組み立てる。**panic しない。**
///
/// **読み出しでも検証する**（`memory_from_row` / `vote_from_row` と同じ作法）。
/// 弾くのは4つ:
///
/// * 想定外の `subject`（CHECK があるので通常の経路では入らない）
/// * 読めない `key`
/// * 読めない `before` / `after`
/// * **前も後も無い行**——「無かったものが消えた」は変更ではない。
///   `CHECK` では止められない（列ごとの CHECK は片方しか見ない）ので、ここで落とす。
fn revision_from_row(
    character_id: CharacterId,
    row: &SqliteRow,
) -> Result<AttributeRevision, StoreError> {
    let id: i64 = row.try_get("id").map_err(StoreError::Query)?;

    let stored_subject: String = row.try_get("subject").map_err(StoreError::Query)?;
    let Some(subject) = subject_from_db(&stored_subject) else {
        // **格納値そのものはログにも出さない**（他の拒否と同じ。手掛かりは id）。
        tracing::warn!(
            character_id = character_id.get(),
            revision_id = id,
            "想定外の subject を持つ属性の履歴の行があります"
        );
        return Err(StoreError::InvalidStoredValue);
    };

    let stored_key: String = row.try_get("key").map_err(StoreError::Query)?;
    let key = AttributeKey::parse(&stored_key).map_err(|_| {
        tracing::warn!(
            character_id = character_id.get(),
            revision_id = id,
            "読み出せないキーを持つ属性の履歴の行があります"
        );
        StoreError::InvalidStoredValue
    })?;

    let before = revision_value(character_id, id, row, "before")?;
    let after = revision_value(character_id, id, row, "after")?;
    if before.is_none() && after.is_none() {
        tracing::warn!(
            character_id = character_id.get(),
            revision_id = id,
            "前の値も後の値も無い属性の履歴の行があります"
        );
        return Err(StoreError::InvalidStoredValue);
    }

    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(StoreError::Query)?;

    Ok(AttributeRevision {
        id,
        subject,
        key,
        before,
        after,
        created_at,
    })
}

/// 履歴の `before` / `after` を読む。**NULL は `None`**（無かった / 消えた）。
///
/// 列名を引数で受けるのは、**同じ判定を2回書かない**ため
/// （片方だけ検証が抜ける形を作らない）。
fn revision_value(
    character_id: CharacterId,
    revision_id: i64,
    row: &SqliteRow,
    column: &'static str,
) -> Result<Option<AttributeValue>, StoreError> {
    let stored: Option<String> = row.try_get(column).map_err(StoreError::Query)?;
    let Some(stored) = stored else {
        return Ok(None);
    };
    let value = AttributeValue::parse(&stored).map_err(|_| {
        tracing::warn!(
            character_id = character_id.get(),
            revision_id,
            column,
            "読み出せない値を持つ属性の履歴の行があります"
        );
        StoreError::InvalidStoredValue
    })?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // 速報（SQL 文そのものの断片）。**実挙動は tests/store_attribute.rs が持つ。**
    // ---------------------------------------------------------------------

    /// **履歴を引く文が、絞り込みと並びと上限を全部持っている。**
    ///
    /// | 欠けると | |
    /// |---|---|
    /// | `WHERE character_id = ?` | **別のキャラクターの履歴が混ざる** |
    /// | `ORDER BY created_at DESC` | 「新しい順」でなくなる |
    /// | `, id DESC` | 同着の順が挿入順まかせになる（1回の置き換えで最大 16 件が同着） |
    /// | `LIMIT ?` | 編集回数に比例して応答が伸び続ける |
    /// | `created_at` | **いつ変わったのかが読めない**（この表の存在理由が消える） |
    #[test]
    fn the_history_query_keeps_its_filter_order_limit_and_time() {
        for fragment in [
            "created_at",
            "WHERE character_id = ?",
            "ORDER BY created_at DESC, id DESC",
            "LIMIT ?",
        ] {
            assert!(
                SELECT_REVISIONS.contains(fragment),
                "{fragment} が消えています: {SELECT_REVISIONS}"
            );
        }
    }

    /// **総数を数える文が、持ち主で絞っていて、上限を持たない。**
    ///
    /// | 欠けると | |
    /// |---|---|
    /// | `WHERE character_id = ?` | **他のキャラクターの編集回数が総数に混ざる** |
    /// | `LIMIT` が付く | 総数が「返した件数」に潰れ、**画面が「これで全部」と嘘をつく** |
    ///
    /// 速報でしかない——実挙動の権威は `tests/server_admin_attributes.rs` の
    /// `the_history_of_each_character_is_its_own`（**両方に履歴がある状態**で測る）と
    /// `only_the_newest_revisions_come_back_but_the_total_counts_them_all`。
    #[test]
    fn the_history_count_is_scoped_to_one_character_and_never_limited() {
        assert!(
            COUNT_REVISIONS.contains("WHERE character_id = ?"),
            "持ち主で絞ること: {COUNT_REVISIONS}"
        );
        assert!(
            !COUNT_REVISIONS.contains("LIMIT"),
            "総数を上限で切らないこと: {COUNT_REVISIONS}"
        );
    }

    /// **どの文もプレースホルダで値を渡す**（CLAUDE.md 絶対制約）。
    ///
    /// 主語の綴り（`'user'` / `'character'`）を SQL に直書きする形は、
    /// ここで**リテラルとして落ちる**（`subject_to_db` の写しも同時に防がれる）。
    #[test]
    fn every_statement_binds_its_values_through_placeholders() {
        let quote = '\'';
        for statement in [
            SELECT_ATTRIBUTES,
            DELETE_ATTRIBUTES,
            INSERT_ATTRIBUTE,
            INSERT_REVISION,
            SELECT_REVISIONS,
            COUNT_REVISIONS,
        ] {
            assert!(
                !statement.contains(quote),
                "リテラルを埋め込まないこと: {statement}"
            );
        }
    }

    /// **置き換えは `BEGIN IMMEDIATE` で入る**（ADR-0028 §決定5）。
    ///
    /// 速報でしかない——**効いていること**の権威は
    /// `tests/store_attribute.rs::two_replacements_never_lose_their_snapshot`。
    #[test]
    fn the_replacement_opens_an_immediate_transaction() {
        assert_eq!(
            BEGIN_IMMEDIATE, "BEGIN IMMEDIATE",
            "読んでから書くので、素の BEGIN では BUSY_SNAPSHOT になる"
        );
    }

    // ---------------------------------------------------------------------
    // SQL の走査（ADR-0028 §決定5 の「書き込み経路は1つ」「履歴は追記のみ」）
    //
    // 形は `store::memory` / `store::drift` の走査に倣う。**判定はこのモジュールの
    // 2つの関数だけ**にして、見張り番も対照も同じ関数を通す
    // （写しにすると、判定を緩める変異が両側とも緑を通る / 14a の実測）。
    //
    // **走査は生ソースをトークンに均す**（`store::drift` と同じ側）。
    // 字句解析を写すと「片方だけが直る写し」がもう1つ増えるため。
    // 代償は**誤検出する側に倒れる**ことで、それは正しい振る舞いである。
    // ---------------------------------------------------------------------

    /// 走査対象。**`migrations` を外さないこと。**
    ///
    /// 「もう古い属性だから消してよい」と整理する SQL は、コードではなく
    /// **マイグレーションに書かれるのが最もありそう**である。
    ///
    /// **Rust 側は `src/store` ではなく `src` 全体**（`store` に絞ると
    /// `src/server/admin.rs` に1行書くだけで走査の外に出る / ADR-0008）。
    const ATTRIBUTE_SCAN_ROOTS: [(&str, &str); 2] = [("src", "rs"), ("migrations", "sql")];

    /// **投影へ書いてよい唯一のファイル**（ADR-0028 §決定5）。
    ///
    /// ここを増やすときは、**履歴を書かない書き手が増えていないか**を必ず見ること
    /// ——投影だけを触る文が1つでもあると、履歴は「いま」の投影を説明しなくなる。
    ///
    /// **マイグレーションでの表の作り直し**（`0005` / `0007` が `character` に対して
    /// 行った形）が必要になったら、そのときここに足すことになる。
    /// **そこがレビューの掛かる場所である。**
    /// > **⚠ 位置の表に `TO` が無い。** [`attribute_write_keyword`] が見るのは
    /// > `INTO` / `UPDATE` / `REPLACE` / `TRUNCATE` / `FROM`(DELETE) / `TABLE`(DROP・ALTER) だけで、
    /// > **`ALTER TABLE x RENAME TO character_attribute` は単独では捕まらない**
    /// > （投影の名前が `TO` の直後に来るため）。
    /// > **表を丸ごとすげ替える形なら、古い名前を触る `DROP` / `ALTER` の側で落ちる。**
    /// > **走査は変えない**——`TO` を足すと当たる面が散文まで広がる。
    const PROJECTION_WRITER: [&str; 3] = ["src", "store", "attribute.rs"];

    /// 投影のテーブル名（大文字）を**ソース上に綴らずに**組み立てる。
    ///
    /// この走査は自分自身のファイルも対象にするため、綴りをそのまま書くと
    /// 判定の対象になる（`store::memory::table_word` と同じ作法）。
    fn projection_table_word() -> String {
        ["character", "_attribute"].concat().to_uppercase()
    }

    /// 履歴のテーブル名（大文字）。理由は [`projection_table_word`] と同じ。
    fn revision_table_word() -> String {
        ["attribute", "_revision"].concat().to_uppercase()
    }

    /// **投影へ書く文が、唯一の書き手のファイル以外に無い**（ADR-0028 §決定5）。
    ///
    /// # なぜ「書く文」だけを見るのか（履歴の走査と形が違う理由）
    ///
    /// 投影のテーブル名は**散文にも出る**（`docs` を指す doc コメントや、
    /// 「この表は何か」の説明）。履歴の側のように
    /// **すべての言及を許可リストに掛ける**と、コメントを1行書くたびに赤くなる。
    ///
    /// そこで判定を「**書き込みの位置に居るか**」に絞ってある。
    /// SQL でこの表の行を変えるには `INSERT` / `UPDATE` / `DELETE` / `REPLACE` /
    /// `DROP` / `TRUNCATE` のどれかが要り、**そのとき表名は必ず決まった位置に来る**
    /// （`INTO <表>` / `UPDATE <表>` / `DELETE FROM <表>` / `DROP TABLE <表>` …）。
    /// 位置を [`attribute_write_keyword`] に列挙し、そこに当たったものだけを違反とする。
    ///
    /// # この網に掛からないもの（把握したうえで残している）
    ///
    /// * **非リテラルの組み立て**（`format!` で表名と動詞を別々に継ぎ足す形）。
    ///   記憶の走査と同じ穴で、SQL を定数から組み立てないこと。
    /// * **`tests/` 配下。** 統合テストは本番バイナリにリンクされない
    ///   ——現に `tests/store_schema.rs` は素の `INSERT` で行を作る。
    /// * **親テーブル経由の CASCADE 削除**（`character` を消せば属性も消える）。
    ///   設計として正当なので止めない。
    #[test]
    fn no_statement_outside_the_one_writer_touches_the_projection() {
        let table = projection_table_word();
        let writer = PROJECTION_WRITER.join(std::path::MAIN_SEPARATOR_STR);
        let targets = attribute_scan_targets();

        assert!(
            targets.len() >= 5,
            "走査対象が少なすぎるなら、この assert は何も守っていない: {:?}",
            targets.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );

        let mut mentions = 0;
        let mut found = Vec::new();
        for (path, source) in &targets {
            mentions += attribute_mentions_of(source, &table);
            if path.ends_with(&writer) {
                continue;
            }
            for violation in projection_writes(source, &table) {
                found.push(format!("{path}: {violation}"));
            }
        }

        assert!(
            found.is_empty(),
            "投影へ書く文が唯一の書き手の外にあります（履歴を通らない変更が作れます）:\n{}",
            found.join("\n")
        );
        // **言及を1件も拾えていないなら、走査が壊れている。**
        assert!(
            mentions >= 4,
            "テーブル名の言及を拾えていない（走査が壊れている）: {mentions} 件"
        );
    }

    /// **唯一の書き手は、現に書いている。**
    ///
    /// 上のテストは「見つからないこと」を主張するので、
    /// **判定が何も見つけられなくなっても緑になる。**
    /// 書き手のファイルには DELETE と INSERT があり、判定がそれを**書き込みとして**
    /// 拾えることを別に確かめる。
    #[test]
    fn the_one_writer_really_writes_the_projection() {
        let table = projection_table_word();
        let writer = PROJECTION_WRITER.join(std::path::MAIN_SEPARATOR_STR);
        let (_, source) = attribute_scan_targets()
            .into_iter()
            .find(|(path, _)| path.ends_with(&writer))
            .expect("唯一の書き手のファイルが走査対象に居ること");

        let writes = projection_writes(&source, &table);
        assert_eq!(
            writes.len(),
            2,
            "全消しと1件挿入の2文が書き込みとして拾われること: {writes:?}"
        );
    }

    /// **走査本体が、既知の書き込みの形を全部落とす。**
    ///
    /// **対照は植えたソース**（実ファイルを読む対照は自己成就する / `15b` の実測）。
    /// **改行を変えても同じ答えになること**まで見る
    /// ——このリポジトリは checkout の時点で CRLF になり、
    /// 作業ツリーで書いた新しいファイルは LF になる（`E-1` の実測）。
    #[test]
    fn the_projection_scan_catches_every_known_write() {
        let table = projection_table_word();
        let name = table.to_lowercase();
        let writes = [
            format!("INSERT INTO {name} (character_id, subject, key, value)\nVALUES (?, ?, ?, ?)"),
            format!("INSERT OR REPLACE INTO {name} (character_id)\nVALUES (?)"),
            format!("REPLACE INTO {name} (character_id) VALUES (?)"),
            format!("INSERT INTO main.{name} (character_id) VALUES (?)"),
            format!("UPDATE {name}\nSET value = ? WHERE key = ?"),
            format!("UPDATE main.{name} SET value = ?"),
            format!("UPDATE {name} AS a SET value = ?"),
            format!("UPDATE OR REPLACE {name} SET value = ?"),
            format!("DELETE FROM {name} WHERE character_id = ?"),
            format!("DELETE\nFROM main.{name}"),
            format!("DROP TABLE {name}"),
            format!("TRUNCATE {name}"),
            // 前の文の末尾を借りる形（記憶の走査が実際に踏んだ）。
            format!("PRAGMA foreign_keys = ON;\nDELETE FROM {name};"),
        ];

        for write in &writes {
            let found = projection_writes(write, &table);
            assert_eq!(
                found.len(),
                1,
                "この形を1件の書き込みとして落とせていません: {write}"
            );
            // **CRLF でも同じ答え。**
            let crlf = write.replace('\n', "\r\n");
            assert_eq!(
                projection_writes(&crlf, &table),
                found,
                "改行が CRLF だと答えが変わっています: {write}"
            );
        }
    }

    /// **読み出しと散文は落とさない。**
    ///
    /// ここを狭めると、doc コメントに表の名前を書くたびに赤くなり、
    /// **走査ごと外される**羽目になる（記憶の走査が `store` だけを見ていた頃の失敗）。
    #[test]
    fn the_projection_scan_leaves_reads_and_prose_alone() {
        let table = projection_table_word();
        let name = table.to_lowercase();
        let allowed = [
            format!("SELECT subject, key, value FROM {name} WHERE character_id = ?"),
            format!("SELECT count(*) FROM {name}"),
            format!("CREATE TABLE {name} (character_id INTEGER NOT NULL)"),
            format!("PRAGMA table_info({name})"),
            // 散文（doc コメント）。**動詞が同じ文の中にあっても、位置が違えば落とさない。**
            format!("/// `{name}` は投影である。INSERT と DELETE は1箇所に閉じている。"),
            format!("//! `{name}` の読み書き（ADR-0009）。"),
            format!(
                "/// 置き換えは DELETE と INSERT を1つのトランザクションで行う。`{name}` を読む。"
            ),
        ];

        for statement in &allowed {
            assert_eq!(
                projection_writes(statement, &table),
                Vec::<String>::new(),
                "書き込みでない形を落としています: {statement}"
            );
        }
    }

    /// **履歴を書き換える文が、どこにも無い**（ADR-0028 §決定5 / 追記のみ）。
    ///
    /// # 判定は禁止語の列挙ではなく**許可リスト**
    ///
    /// 票の走査（`store::drift`）と同じ形。新しい形が要るなら
    /// [`revision_form_is_allowed`] を書き換えることになり、**そこがレビューの掛かる場所**になる。
    ///
    /// **代償は、履歴の表の名前を Rust のコメントや識別子に綴れないこと。**
    /// 綴ると許可された形に当たらず、この見張り番が赤になる。
    /// **それは正しい振る舞いである**——静かに通り抜けるより、うるさく落ちる方を選ぶ。
    /// （だからこのモジュールの doc は「履歴の表」と呼んでいる。）
    #[test]
    fn no_statement_updates_or_deletes_a_revision() {
        let table = revision_table_word();
        let targets = attribute_scan_targets();

        assert!(
            targets.len() >= 5,
            "走査対象が少なすぎるなら、この assert は何も守っていない: {:?}",
            targets.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );

        let mut mentions = 0;
        let mut found = Vec::new();
        for (path, source) in &targets {
            mentions += attribute_mentions_of(source, &table);
            for violation in revision_violations(source, &table) {
                found.push(format!("{path}: {violation}"));
            }
        }

        assert!(
            found.is_empty(),
            "履歴を書き換えうる文があります:\n{}",
            found.join("\n")
        );
        // 内訳は追記と読み出しの2つと、マイグレーションの
        // `CREATE TABLE` / `CREATE INDEX ... ON` の2つ。
        assert!(
            mentions >= 4,
            "テーブル名の言及を拾えていない（走査が壊れている）: {mentions} 件"
        );
    }

    /// **走査本体が、既知の回避手段を全部落とす。**
    ///
    /// 対照は植えたソース（`15b`）。**LF と CRLF の両方で同じ答えになること**まで見る。
    #[test]
    fn the_revision_scan_catches_every_known_evasion() {
        let table = revision_table_word();
        let name = table.to_lowercase();
        let evasions = [
            format!("UPDATE {name} SET after = ? WHERE id = ?"),
            format!("UPDATE {name}\nSET before = ?"),
            format!("UPDATE main.{name} SET after = ?"),
            format!("UPDATE {name} AS r SET after = ?"),
            format!("UPDATE OR REPLACE {name} SET after = ?"),
            format!("DELETE FROM {name} WHERE character_id = ?"),
            format!("DELETE FROM main.{name}"),
            format!("INSERT OR REPLACE INTO {name} (character_id) VALUES (?)"),
            format!("REPLACE INTO {name} (character_id) VALUES (?)"),
            format!(
                "INSERT INTO {name} (character_id, subject, key)\nVALUES (?, ?, ?) \
                 ON CONFLICT (id) DO UPDATE SET after = excluded.after"
            ),
            format!("DROP TABLE {name}"),
            // 前の文の末尾の `ON` を借りる形（記憶の走査が実際に踏んだ）。
            format!("PRAGMA foreign_keys = ON;\nDELETE FROM {name};"),
            format!("ON DELETE FROM {name} WHERE id = ?"),
            // **表の名前が値の位置に来る形**（既定は拒否）。
            format!("SELECT 1 FROM other WHERE x = {name}"),
            format!("PRAGMA table_info({name})"),
            // **散文に綴る形も落ちる**（上の doc の「代償」そのもの）。
            format!("/// `{name}` は追記のみ。"),
        ];

        for evasion in &evasions {
            let found = revision_violations(evasion, &table);
            assert!(!found.is_empty(), "この形を落とせていません: {evasion}");
            let crlf = evasion.replace('\n', "\r\n");
            assert_eq!(
                revision_violations(&crlf, &table),
                found,
                "改行が CRLF だと答えが変わっています: {evasion}"
            );
        }
    }

    /// **既定の枝（許可リストに無い形）が、破壊的な語なしでも効いている。**
    ///
    /// 16d のレビューが票の側で実測した形——`_ => false` を `_ => true` にしても
    /// 全緑になる、という穴を先に塞いでおく。
    /// ここに並ぶのは**破壊的な語を1つも含まない、許可されていない形**である。
    #[test]
    fn the_revision_scan_rejects_forms_that_are_not_on_the_allowlist() {
        let table = revision_table_word();
        let name = table.to_lowercase();

        for statement in [
            format!("SELECT 1 FROM other WHERE x = {name}"),
            format!("PRAGMA table_info({name})"),
            format!("/// 履歴の表は {name} である。"),
        ] {
            assert!(
                revision_destructive_in(&attribute_tokens(&statement)).is_none(),
                "前提: 破壊的な語を含まない文であること（既定の枝でしか落ちない）: {statement}"
            );
            assert!(
                !revision_violations(&statement, &table).is_empty(),
                "許可リストに無い形が通っています: {statement}"
            );
        }
    }

    /// **設計上ありうる形は落とさない。**
    ///
    /// ここを狭めると、追記と読み出しそのものが書けなくなって
    /// **走査ごと外される**羽目になる。
    #[test]
    fn the_revision_scan_accepts_the_statements_the_design_allows() {
        let table = revision_table_word();
        let name = table.to_lowercase();
        let allowed = [
            format!(
                "INSERT INTO {name} (character_id, subject, key, before, after) VALUES (?, ?, ?, ?, ?)"
            ),
            format!(
                "SELECT id, subject, key, before, after, created_at FROM {name} WHERE character_id = ? LIMIT ?"
            ),
            format!("SELECT COUNT(*) FROM {name}"),
            // migrations の DDL。参照アクションの `ON DELETE CASCADE` を含む形も通す。
            format!(
                "CREATE TABLE {name} (id INTEGER PRIMARY KEY AUTOINCREMENT, \
                 character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE)"
            ),
            format!("CREATE INDEX {name}_recent_idx ON {name} (character_id, created_at)"),
            format!("PRAGMA foreign_keys = ON;\nSELECT id FROM {name} WHERE character_id = ?;"),
        ];

        for statement in &allowed {
            let violations = revision_violations(statement, &table);
            assert!(
                violations.is_empty(),
                "正当な文を落としています: {statement} / {violations:?}"
            );
            let crlf = statement.replace('\n', "\r\n");
            assert!(
                revision_violations(&crlf, &table).is_empty(),
                "CRLF の正当な文を落としています: {statement}"
            );
        }
    }

    /// **走査は `src` 全体と `migrations` を見る。**
    ///
    /// `store` に絞ると、`src/server/admin.rs` に1行書くだけで回避できる
    /// （ADR-0008 が「`server` に生の SQL を書けば走査の外に出る」と明示している）。
    #[test]
    fn the_attribute_scans_cover_every_module_and_the_migrations() {
        let roots: Vec<&str> = ATTRIBUTE_SCAN_ROOTS.iter().map(|(root, _)| *root).collect();
        assert!(roots.contains(&"src"), "src 全体を見ること: {roots:?}");
        assert!(
            roots.contains(&"migrations"),
            "migrations を外さないこと: {roots:?}"
        );

        let separator = std::path::MAIN_SEPARATOR;
        let scanned = attribute_scan_targets();
        for module in ["server", "prompt", "llm", "domain", "parse"] {
            let expected = format!("src{separator}{module}{separator}");
            assert!(
                scanned.iter().any(|(path, _)| path.contains(&expected)),
                "{module} が走査対象に入っていない: {:?}",
                scanned.iter().map(|(path, _)| path).collect::<Vec<_>>()
            );
        }
        assert!(
            scanned.iter().any(|(path, _)| path.ends_with(".sql")),
            "migrations の .sql を拾えていない: {:?}",
            scanned.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );

        // **中身も読めている。** `server` に生の SQL を書けば実際に掛かることを、
        // 走査本体（判定と同じ関数）に植えたソースを通して確かめる。
        let projection = projection_table_word();
        let planted = format!(
            "const BAD: &str = \"DELETE FROM {} WHERE character_id = ?\";\n",
            projection.to_lowercase()
        );
        assert_eq!(
            projection_writes(&planted, &projection).len(),
            1,
            "植えた書き込みを落とせること"
        );
    }

    /// 走査対象を「パス, ソース」で返す。**`.sql` は行コメントを落としてから。**
    fn attribute_scan_targets() -> Vec<(String, String)> {
        let mut targets = Vec::new();
        for (root, extension) in ATTRIBUTE_SCAN_ROOTS {
            for (path, source) in attribute_source_files(root, extension) {
                let text = if extension == "sql" {
                    strip_sql_comments(&source)
                } else {
                    source
                };
                targets.push((path, text));
            }
        }
        targets
    }

    /// `root` 配下の拡張子 `extension` のファイルを「パス, 生ソース」で返す。
    fn attribute_source_files(root: &str, extension: &str) -> Vec<(String, String)> {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
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
                files.push((path.display().to_string(), source));
            }
        }
        files
    }

    /// SQL の行コメント以降を落とす。**`.sql` 専用。**
    ///
    /// **`lines()` に分割を委ねている**ので、LF と CRLF で結果は変わらない。
    fn strip_sql_comments(source: &str) -> String {
        let marker = ["-", "-"].concat();
        source
            .lines()
            .map(|line| line.split(&marker).next().unwrap_or(""))
            .collect::<Vec<&str>>()
            .join("\n")
    }

    /// ソースをトークン列に均す（`store::drift::vote_tokens` と同じ規則）。
    ///
    /// 語は `[A-Za-z0-9_]` の連なりで、大文字に揃える（SQL は大小を区別しない）。
    /// **それ以外は全部区切り**なので、`\r` も `.` も引用符も同じ扱いになる
    /// ——ここが「CRLF でも答えが変わらない」ことの根拠である。
    fn attribute_tokens(source: &str) -> Vec<String> {
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
    fn attribute_mentions_of(source: &str, table: &str) -> usize {
        attribute_tokens(source)
            .iter()
            .filter(|token| token.as_str() == table)
            .count()
    }

    /// 違反メッセージに載せる抜粋の幅（トークン数）。**判定には使わない。**
    const ATTRIBUTE_LOOK_BEHIND: usize = 4;
    const ATTRIBUTE_LOOK_AHEAD: usize = 8;

    /// 抜粋を作る。
    fn attribute_excerpt(tokens: &[String], index: usize) -> String {
        let from = index.saturating_sub(ATTRIBUTE_LOOK_BEHIND);
        let to = (index + ATTRIBUTE_LOOK_AHEAD + 1).min(tokens.len());
        tokens[from..to].join(" ")
    }

    /// 表名の前にある**意味のある語**を、近い順に返す。
    ///
    /// スキーマ修飾（`main.`）と競合節（`INSERT OR REPLACE` / `UPDATE OR ROLLBACK`）は
    /// **読み飛ばす**——飛ばさないと、`main.` を1つ付けるだけで判定を外せる。
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

    /// **表名が「行を変える文」の位置に居るか。** 居るならその動詞を返す。
    ///
    /// SQLite でこの表の行を変えるには次のどれかが要り、
    /// **そのとき表名の直前に来る語は決まっている。**
    ///
    /// | 形 | 表名の直前 |
    /// |---|---|
    /// | `INSERT INTO` / `INSERT OR REPLACE INTO` / `REPLACE INTO` | `INTO` |
    /// | `UPDATE` / `UPDATE OR REPLACE` / `UPDATE ... AS` | `UPDATE` |
    /// | `DELETE FROM` | `FROM`（その前が `DELETE`） |
    /// | `DROP TABLE` / `ALTER TABLE` | `TABLE`（その前が `DROP` / `ALTER`） |
    /// | `TRUNCATE`（SQLite には無いが、写して持ち込まれる） | `TRUNCATE` |
    ///
    /// **`REPLACE` を `NOISE` に入れていない**のは、`REPLACE INTO` が `INTO` で捕まり、
    /// `UPDATE OR REPLACE` は `OR` を飛ばした先の `REPLACE` をここで拾うため。
    fn attribute_write_keyword(tokens: &[String], index: usize) -> Option<&'static str> {
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

    /// **投影を書き換える文の走査本体。** 見張り番も対照も**この関数を通す**こと。
    ///
    /// 判定の前に `;` で文へ割る。割らないと**前の文の末尾**が現在の文の文脈として
    /// 読まれる（記憶の走査が `PRAGMA foreign_keys = ON;` で実際に踏んだ）。
    fn projection_writes(source: &str, table: &str) -> Vec<String> {
        source
            .split(';')
            .flat_map(|statement| {
                let tokens = attribute_tokens(statement);
                let mut found = Vec::new();
                for (index, token) in tokens.iter().enumerate() {
                    if token.as_str() != table {
                        continue;
                    }
                    if let Some(keyword) = attribute_write_keyword(&tokens, index) {
                        found.push(format!(
                            "{keyword} で投影を書いています: {}",
                            attribute_excerpt(&tokens, index)
                        ));
                    }
                }
                found
            })
            .collect()
    }

    /// 履歴の言及の近くにあってはならない語。
    ///
    /// **`UPDATE` を含める**（票の表と同じ。履歴に更新の用途は1つも無い）。
    /// `CONFLICT` は `INSERT INTO ... ON CONFLICT ... DO UPDATE` を落とすため
    /// ——この形は表名が `INTO` の直後なので、許可リストだけでは素通りする。
    const REVISION_DESTRUCTIVE: [&str; 6] = [
        "UPDATE", "REPLACE", "DELETE", "DROP", "TRUNCATE", "CONFLICT",
    ];

    /// 文中に破壊的な語があるか。
    ///
    /// **外部キーの参照アクション（`ON DELETE CASCADE` 等）だけは除く。**
    /// 履歴の表自身が持っているので、除かないと常に赤になる。
    /// 除外は「直前が `ON`」だけでは足りない——参照アクションの次に `FROM` は来ない。
    fn revision_destructive_in(tokens: &[String]) -> Option<&'static str> {
        for (index, token) in tokens.iter().enumerate() {
            let reference_action = token.as_str() == "DELETE"
                && index > 0
                && tokens[index - 1] == "ON"
                && tokens.get(index + 1).map_or("", String::as_str) != "FROM";
            if reference_action {
                continue;
            }
            if let Some(found) = REVISION_DESTRUCTIVE
                .iter()
                .find(|word| **word == token.as_str())
            {
                return Some(found);
            }
        }
        None
    }

    /// 履歴の言及が**許可された形**か。判定は直前の語で行う。**既定は拒否。**
    ///
    /// 許すのは**追記・読み出し・DDL** だけ。`UPDATE` を通す形は1つも無い。
    fn revision_form_is_allowed(tokens: &[String], index: usize) -> bool {
        let before = |back: usize| -> &str {
            index
                .checked_sub(back)
                .and_then(|at| tokens.get(at))
                .map_or("", String::as_str)
        };

        match before(1) {
            // 追記。
            "INTO" => before(2) == "INSERT",
            // 読み出し。`DELETE FROM` は `revision_destructive_in` が先に落とす。
            "FROM" => true,
            // 以下は migrations の DDL。
            "TABLE" => before(2) == "CREATE",
            "ON" => before(3) == "INDEX",
            "REFERENCES" => true,
            _ => false,
        }
    }

    /// **履歴を書き換える文の走査本体。** 見張り番も対照も**この関数を通す**こと。
    fn revision_violations(source: &str, table: &str) -> Vec<String> {
        source
            .split(';')
            .flat_map(|statement| {
                let tokens = attribute_tokens(statement);
                let mut found = Vec::new();
                for (index, token) in tokens.iter().enumerate() {
                    if token.as_str() != table {
                        continue;
                    }
                    let excerpt = attribute_excerpt(&tokens, index);
                    if let Some(keyword) = revision_destructive_in(&tokens) {
                        found.push(format!("同じ文に {keyword} があります: {excerpt}"));
                    } else if !revision_form_is_allowed(&tokens, index) {
                        found.push(format!("許可されていない形です: {excerpt}"));
                    }
                }
                found
            })
            .collect()
    }
}
