//! `memory` の読み書き。
//!
//! **このモジュールが提供するのは追加（INSERT）と読み出し（SELECT）だけ。
//! `content` を UPDATE する API も、行を DELETE する API も意図的に存在しない。**
//!
//! 矛盾する記憶が出てきたときは、古い行を潰さず新しい行を追加し、古い行の
//! `superseded_by` に後継の id を入れる（docs/03-data-model.md の不変条件2）。
//! これはフェーズ2で「以前はXだったが今はY」と想起するための情報で、
//! **一度上書きすると永久に失われる。**
//!
//! > **外部キー制約はこの不変条件を守らない。** `superseded_by` の複合外部キーが防ぐのは
//! > 「参照先（後継行）が消えること」であり、守りたい**古い行の側は無防備**。
//! > `DELETE FROM memory WHERE superseded_by IS NOT NULL` は「もう上書き済みだから
//! > 消してよい行」を選ぶ最も自然なクエリでありながら、**保持したい行だけを正確に消す**
//! > （`tests/store_schema.rs::database_does_not_protect_superseded_rows_from_deletion`
//! > がその実挙動を固定している）。**DB は止めてくれないので、API を生やさないことが
//! > 唯一の防御。** 更新・削除が必要になったら、まずこの段落を疑うこと。
//!
//! > 見張り番は `no_statement_updates_or_deletes_a_memory`。**このモジュールだけでなく
//! > `src/**` と `migrations/**` 全体**を走査する（`store` に閉じない。
//! > ADR-0008 が「`server` に生の SQL を書けば走査の外に出る」と明示している）。
//! > マイグレーションを外さないのは、
//! > 上の `DELETE FROM memory WHERE superseded_by IS NOT NULL` が
//! > **データ整理のマイグレーションに書かれるのが最もありそう**だから。
//! >
//! > 判定は**禁止語の列挙ではなく許可リスト**。禁止語の列挙は
//! > `INSERT OR REPLACE INTO memory`（`DELETE` の字が出ないまま行が消える）や
//! > `UPDATE memory SET locked = 1, content = ?`（並び替えるだけで隣接しなくなる）に
//! > 原理的に弱い。許可する形は「追加」「読み出し」「フェーズ2の
//! > `UPDATE ... SET superseded_by`」「マイグレーションの DDL」だけで、
//! > **形を増やすには走査側を書き換えることになる**（そこがレビューの掛かる場所）。
//! >
//! > **走査が見るのは文字列リテラルに書かれた SQL だけ。** `format!` での組み立てや
//! > 親テーブル経由の CASCADE 削除は網の外にある（一覧は
//! > `no_statement_updates_or_deletes_a_memory` の doc）。**抽出そのものが
//! > 追いつけなくなった場合は `extraction_hazards` が落とす**（静かに無効化させない）。
//!
//! この層は薄く保つ。`&str` や裸の `i64` を受ける公開 API を作らないのは
//! `character.rs` / `session.rs` と同じ（本文は `MemoryContent`、分類は `MemoryCategory`）。

use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::{Store, StoreError};
use crate::domain::{
    CharacterId, Memory, MemoryCategory, MemoryContent, MemoryId, NewMemory, SessionId, Subject,
};

/// 記憶の追加。`created_at` / `locked` はスキーマの DEFAULT に任せる。
///
/// **`superseded_by` は書き込まない。** ここで作るのは常に**新しい行**なので後継は NULL。
/// 後継を指すのは**古い行の側**である（`memory_repair::supersede_memory`）。
///
/// **`supersede_memory` もこの文で訂正版を入れる**（作業単位 14a）ので `pub(super)`。
/// 写しを作らないのは、片方の列構成だけが黙って変わるのを避けるため
/// ——訂正版は「普通の記憶」であって、特別な列を持たない。
pub(super) const INSERT_MEMORY: &str =
    "INSERT INTO memory (character_id, subject, content, category) VALUES (?, ?, ?, ?)";

/// **抽出した記憶の追加**（作業単位 A-10 / ADR-0049 §決定3）。[`INSERT_MEMORY`] と同じ列に、
/// **それを生んだ会話が今も在り、同じキャラクターのものであること**を同じ文で足す。
///
/// 会話が消えていれば（＝キャラクターが完全に消された）0 行で、何も書かれない。
/// **判定と書き込みを分けないこと** —— 分けると、その隙に消されたキャラクターの記憶が、
/// 使い回された同じ id の新しいキャラクターに入る（`StoreError::TurnSessionGone` の doc）。
const INSERT_MEMORY_IN_SESSION: &str = "INSERT INTO memory (character_id, subject, content, category) \
     SELECT ?1, ?2, ?3, ?4 \
     WHERE EXISTS (SELECT 1 FROM session WHERE session.id = ?5 AND session.character_id = ?1)";

/// 有効な記憶の全件取得（ADR-0003: 全件注入）。
///
/// **`superseded_by IS NULL` を外さないこと。** 外すと「以前はコーヒーが好きだった」が
/// 現在の事実としてプロンプトに載り、キャラクターが矛盾したことを言い出す。
///
/// `locked` では絞り込まない（手動ロックは想起の対象から外す印ではない）。
///
/// 並びは作成順。`created_at` はミリ秒精度で同値になりうるので `id` を第2キーに置く
/// （**1ターンで最大5件入る**ので同着は普通に起きる）。
/// `memory_character_idx (character_id, created_at)` の並びに合わせてある。
///
/// **第2キーの向きの見張り番は
/// `tests/store_memory.rs::memories_created_in_the_same_millisecond_are_ordered_by_id`。**
/// 同着の組を作らないテストしか無かった間は、`, id DESC` にしても緑だった
/// （14a のレビューで実測。**これは 14a が作った穴ではなく、以前からあった穴**）。
const SELECT_ACTIVE_MEMORIES: &str = "SELECT id, subject, content, category, created_at, locked FROM memory \
     WHERE character_id = ? AND superseded_by IS NULL ORDER BY created_at, id";

impl Store {
    /// 記憶を追加し、追加した行の id を返す。
    ///
    /// 存在しないキャラクターなら外部キー違反（`StoreError::Query`）。
    ///
    /// **上書きの API はこれと対にならない。** 既存の記憶と矛盾する内容でも、
    /// 古い行を書き換えずに新しい行として追加する（モジュール doc 参照）。
    ///
    /// # **テスト専用**（作業単位 A-10）
    ///
    /// 本番の書き手（抽出）は [`Self::add_memory_in_session`] を通る。こちらは会話で縛らないので、
    /// 本番から呼べると「消したキャラクターの記憶が、同じ id の新しいキャラクターに入る」形が
    /// 作れる。**`testing` フィーチャ無しの `cargo build` では存在しない**（`Store::pool` と同じ扱い）。
    #[cfg(any(test, feature = "testing"))]
    pub async fn add_memory(
        &self,
        character_id: CharacterId,
        memory: &NewMemory,
    ) -> Result<MemoryId, StoreError> {
        let done = sqlx::query(INSERT_MEMORY)
            .bind(character_id.get())
            .bind(subject_to_db(memory.subject))
            .bind(memory.content.as_str())
            .bind(memory.category.as_ref().map(MemoryCategory::as_str))
            .execute(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let id = done.last_insert_rowid();
        tracing::debug!(
            memory_id = id,
            character_id = character_id.get(),
            "記憶を追加しました"
        );
        Ok(MemoryId::from_row(id))
    }

    /// **会話から抽出した記憶を追加する**（本番の書き手 / 作業単位 A-10 / ADR-0049 §決定3）。
    ///
    /// `session_id` は**その記憶を生んだ往復の会話**。その会話が今も在り、`character_id` の
    /// ものであるときだけ書く（判定と書き込みは1文 / [`INSERT_MEMORY_IN_SESSION`]）。
    /// 会話が無ければ何も書かずに [`StoreError::TurnSessionGone`]（キャラクターが完全に消された）、
    /// 別のキャラクターの会話なら何も書かずに [`StoreError::SessionOfAnotherCharacter`]（取り違え）。
    pub async fn add_memory_in_session(
        &self,
        character_id: CharacterId,
        session_id: SessionId,
        memory: &NewMemory,
    ) -> Result<MemoryId, StoreError> {
        let done = sqlx::query(INSERT_MEMORY_IN_SESSION)
            .bind(character_id.get())
            .bind(subject_to_db(memory.subject))
            .bind(memory.content.as_str())
            .bind(memory.category.as_ref().map(MemoryCategory::as_str))
            .bind(session_id.get())
            .execute(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        if done.rows_affected() == 0 {
            // **書けなかった理由を分ける**（消えた / 別のキャラクターの会話 / `turn_session_refusal`）。
            // 会話がそのキャラクターのものなら 0 行にはならないので、そこへ来たら「消えた」に倒す
            // （引き直すまでの間に消された、の1通りしか無い）。
            let refusal =
                super::session::turn_session_refusal(&self.pool, character_id, session_id).await?;
            return Err(refusal.unwrap_or(StoreError::TurnSessionGone));
        }
        let id = done.last_insert_rowid();
        tracing::debug!(
            memory_id = id,
            character_id = character_id.get(),
            session_id = session_id.get(),
            "記憶を追加しました"
        );
        Ok(MemoryId::from_row(id))
    }

    /// そのキャラクターの**有効な記憶を全件**、作成順（古い順）で返す。
    ///
    /// **想起の入口はこの関数だけ**（ADR-0003）。件数が増えてベクトル検索へ移るときは、
    /// **この関数のシグネチャは必ず変わる**（現在のクエリ文なり埋め込みなりを引数で
    /// 受け取る必要がある）。ADR-0003 が要求しているのは「シグネチャ不変」ではなく
    /// **入口がひとつであること**で、呼び出し側の修正箇所が1関数に閉じるのがその効能。
    /// 想起の経路を並べて増やさないこと。
    ///
    /// 「有効」は **`superseded_by` が NULL** の行。フェーズ2で上書きが入っても
    /// 読み出し側の条件は変わらない。
    ///
    /// **`locked` の行も返す**（フィルタしない）。**`subject` も落とさない。**
    /// どちらかを省くと、上位層は「誰についての記憶か」を復元できない。
    ///
    /// 前後に空白のある本文・分類が格納されていた場合は正規化して返す。
    /// 正規化しても不正な値（空白のみ等）は `StoreError::InvalidStoredValue`。
    ///
    /// **原因を「DB の直接編集」に限定しないこと**（作業単位 0009 / 0009a）。
    /// **検証を後から厳しくすると、`store` 経由で書けた行が不正になる。**
    /// 記憶については `Store::add_memory` を呼ぶ本番コードがまだ無いので
    /// 現時点の実害は無いが、抽出の書き込み経路が入った時点で
    /// `Store::list_characters` の doc に書いた話がそのまま当てはまる
    /// （こちらは**そのキャラクターの想起が丸ごと止まる**）。
    pub async fn recall_memories(
        &self,
        character_id: CharacterId,
    ) -> Result<Vec<Memory>, StoreError> {
        let rows = sqlx::query(SELECT_ACTIVE_MEMORIES)
            .bind(character_id.get())
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let mut memories = Vec::with_capacity(rows.len());
        for row in &rows {
            memories.push(memory_from_row(row, character_id)?);
        }
        tracing::debug!(
            character_id = character_id.get(),
            memories = memories.len(),
            "有効な記憶を全件読み出しました"
        );
        Ok(memories)
    }
}

/// 1行を `Memory` に組み立てる。`character_id` は WHERE で固定済みなので引数を使う。
///
/// # 不正な行を**スキップせず、想起全体を止める**（`list_characters` との非対称は意図的）
///
/// `Store::list_characters` は不正な行を**読めなかった行として並べて返す**（ADR-0016 §3）が、
/// こちらは1件でも不正なら `InvalidStoredValue` を返し、**そのキャラクターの想起を
/// 全滅させる。** 方針を割っているのは、失敗したときに起きることが違うため。
///
/// * **キャラクター一覧**は、落ちた行が**そのまま画面に出る**（指し示せば必ず気付ける）。
///   逆に1件のために一覧全体を止めると、正常なキャラクターまで見えなくなる。
/// * **記憶**は違う。1件落としてもリクエストは成功し、応答も返る。変わるのは
///   **キャラクターの振る舞いだけ**で、しかもそれは「なんとなく前と違う」としか
///   観測できない。値 0 の軸を黙って落とすとまずいのと同じ理由で、
///   **静かに減らすより止めて気付かせる方を選ぶ。**
///
/// この非対称を「片方の実装漏れ」と読んで揃えないこと。
fn memory_from_row(row: &SqliteRow, character_id: CharacterId) -> Result<Memory, StoreError> {
    let stored_subject: String = row.try_get("subject").map_err(StoreError::Query)?;
    // スキーマの CHECK があるため通常は起こらないが、**panic せずエラーに落とす**。
    let Some(subject) = subject_from_db(&stored_subject) else {
        // 格納値は Display に載せられないので、調査の手掛かりはここにしかない。
        tracing::warn!(stored_subject = ?stored_subject, "想定外の subject が格納されています");
        return Err(StoreError::InvalidStoredValue);
    };

    let stored_content: String = row.try_get("content").map_err(StoreError::Query)?;
    let content =
        MemoryContent::parse(&stored_content).map_err(|_| StoreError::InvalidStoredValue)?;

    let stored_category: Option<String> = row.try_get("category").map_err(StoreError::Query)?;
    let category = stored_category
        .as_deref()
        .map(MemoryCategory::parse)
        .transpose()
        .map_err(|_| StoreError::InvalidStoredValue)?;

    // **`try_get::<bool>` にしない。** CHECK は 0 / 1 に縛るが、DB を直接編集すれば
    // `7` も入りうる。sqlx の bool デコードは 0 以外を true にするため、
    // 「保護対象の印」が静かに増える。範囲外は握り潰さずエラーにする。
    let stored_locked: i64 = row.try_get("locked").map_err(StoreError::Query)?;
    let locked = match stored_locked {
        0 => false,
        1 => true,
        other => {
            tracing::warn!(stored_locked = other, "想定外の locked が格納されています");
            return Err(StoreError::InvalidStoredValue);
        }
    };

    Ok(Memory {
        id: MemoryId::from_row(row.try_get::<i64, _>("id").map_err(StoreError::Query)?),
        character_id,
        subject,
        content,
        category,
        created_at: row.try_get("created_at").map_err(StoreError::Query)?,
        locked,
    })
}

/// `Subject` の DB 表現。**`migrations/0001_initial.sql` の
/// `CHECK (subject IN ('user', 'character'))` と同じ綴りにすること。**
///
/// 変換を `domain` ではなくここに置いているのは、綴りの決定権を DB スキーマの隣に
/// 集めるため。`domain::Subject` は DB の都合を知らない。
///
/// **`Role` の綴りと混同しないこと。** キャラクター側は `assistant` ではなく `character`。
///
/// # **`character_attribute` と共有している**（作業単位 12a）
///
/// `migrations/0009_character_attribute.sql` の `subject` も同じ 2 値の CHECK を持つ。
/// **写しを作らずここを呼ぶこと。** 2 箇所に綴りを書くと、片方だけ直したときに
/// もう片方が黙って CHECK 違反（= 理由の分からない 500）になる。
pub(super) const fn subject_to_db(subject: Subject) -> &'static str {
    match subject {
        Subject::User => "user",
        Subject::Character => "character",
    }
}

/// DB 表現から `Subject` へ。想定外の綴りは `None`（呼び出し側でエラーに落とす）。
///
/// **trim や小文字化はしない。** 受理を広げると、CHECK を通らない値を
/// アプリ層だけが受け入れる非対称ができる。
pub(super) fn subject_from_db(stored: &str) -> Option<Subject> {
    match stored {
        "user" => Some(Subject::User),
        "character" => Some(Subject::Character),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 綴りは DB スキーマの CHECK と一致する。
    ///
    /// **往復（`subject_from_db(subject_to_db(x)) == x`）だけでは足りない。**
    /// 2値を取り違えて対応させても往復は成立してしまうため、綴りそのものを固定する。
    #[test]
    fn subject_uses_the_spelling_the_schema_expects() {
        assert_eq!(subject_to_db(Subject::User), "user");
        assert_eq!(subject_to_db(Subject::Character), "character");
    }

    /// 往復する。
    #[test]
    fn subject_round_trips() {
        for subject in [Subject::User, Subject::Character] {
            assert_eq!(subject_from_db(subject_to_db(subject)), Some(subject));
        }
    }

    /// 想定外の綴りは受け付けない（panic もしない）。
    ///
    /// **`assistant` も拒否する。** `message.role` の綴りとは別物で、
    /// 混ざると「キャラの記憶」が読めなくなる。
    #[test]
    fn unknown_subject_is_rejected() {
        for stored in [
            "",
            "assistant",
            "system",
            "User",
            "Character",
            "character ",
            " user",
        ] {
            assert_eq!(subject_from_db(stored), None, "stored = {stored:?}");
        }
    }

    /// 想起の SQL から `superseded_by IS NULL` を落とさない。
    ///
    /// 統合テスト（`tests/store_memory.rs`）が実挙動を固定しているが、
    /// 条件そのものが SQL 文から消えていないこともここで見ておく。
    #[test]
    fn recall_query_filters_superseded_rows() {
        assert!(
            SELECT_ACTIVE_MEMORIES.contains("superseded_by IS NULL"),
            "有効な記憶だけを引くこと: {SELECT_ACTIVE_MEMORIES}"
        );
    }

    // ---------------------------------------------------------------------
    // 「記憶を潰す文」の走査（不変条件2の防御）
    //
    // **禁止語の列挙をやめ、許可リストにしてある。** 経緯は
    // `no_statement_updates_or_deletes_a_memory` の doc を読むこと。
    // ---------------------------------------------------------------------

    /// 走査対象。**`migrations` を外さないこと。**
    ///
    /// 「もう上書き済みだから消してよい行」を消す整理 SQL は、コードではなく
    /// **マイグレーションに書かれるのが最もありそう**で、そこを外すと完全に無防備になる。
    ///
    /// **Rust 側は `src/store` ではなく `src` 全体。** ADR-0008 が
    /// 「`server` に生の SQL を書けば走査の外に出る」と明示しているとおり、
    /// `store` に絞ると `src/server/admin.rs` に1行書くだけで見張り番を回避できる。
    /// 見張りの範囲を人間向けのコメントで代用しない（`the_scan_covers_every_module_not_just_the_store`）。
    const SCAN_ROOTS: [(&str, &str); 2] = [("src", "rs"), ("migrations", "sql")];

    /// 違反メッセージに載せる**抜粋**の幅（トークン数）。
    ///
    /// **判定には使わない。** かつては破壊語をこの窓の中だけで探していたが、
    /// 全8列を書く upsert の `CONFLICT` がちょうど窓の外に落ちて素通りした
    /// （カンマをトークンに数える設計と列数8が噛み合った結果で、
    /// **最も素直な書き方が最も落ちやすい形**だった）。窓を広げても同じことが起きるので、
    /// 破壊語は**文全体**を見る（`destructive_in`）。ここに残っているのは表示の都合だけ。
    const LOOK_BEHIND: usize = 6;
    const LOOK_AHEAD: usize = 24;

    /// テーブル名の近傍にあってはならない語。
    ///
    /// **`REPLACE` を含めるのが要点。** SQLite の `INSERT OR REPLACE` / `REPLACE INTO` は
    /// 競合行を **DELETE してから INSERT する**ので、`DELETE` の文字がソースに一切
    /// 現れないまま行が消える。「上書き API を素直に書く」ときの最短経路がこれ。
    ///
    /// `CONFLICT` は `ON CONFLICT ... DO UPDATE SET content` の upsert を落とすため。
    /// この形は入口が `INSERT INTO memory` なので、前置きの検査だけでは素通りする。
    const DESTRUCTIVE: [&str; 5] = ["REPLACE", "DELETE", "DROP", "TRUNCATE", "CONFLICT"];

    /// テーブル名（大文字）を**ソース上に綴らずに**組み立てる。
    ///
    /// この走査は自分自身のファイルも対象にするため、テーブル名や下の失敗例を
    /// リテラルで書くと必ず自分に引っかかる。分割して `concat` すれば、
    /// ソース上には別々の文字列リテラルとして現れるだけになり、走査は語として読まない。
    fn table_word() -> String {
        ["mem", "ory"].concat().to_uppercase()
    }

    /// **走査対象のどこにも「記憶を潰しうる文」が無い**（不変条件2の防御）。
    ///
    /// # なぜ禁止語の列挙をやめたのか
    ///
    /// 「`UPDATE memory SET content` を含まない」という形の判定は、
    /// **禁止語が隣接した並びでしか一致しない。** 実際に次がすべて素通りしていた:
    /// `UPDATE memory SET locked = 1, content = ?` / `INSERT OR REPLACE INTO memory` /
    /// `REPLACE INTO memory` / `ON CONFLICT ... DO UPDATE SET content` /
    /// `UPDATE memory AS m SET content` / `UPDATE main.memory SET content` /
    /// `DELETE FROM main.memory`。列挙は原理的にこの種の回避に弱い。
    ///
    /// そこで**許可リスト**にしてある。テーブル名への言及をすべて拾い、
    /// 設計上ありうる形（追加・読み出し・フェーズ2の `superseded_by` 更新・DDL）
    /// 以外は落とす。**新しい形が要るなら `form_is_allowed` を書き換える**ことになり、
    /// そこがレビューの掛かる場所になる。
    ///
    /// # なぜ `superseded_by` の UPDATE だけ通すのか
    ///
    /// フェーズ2では `UPDATE memory SET superseded_by = ? WHERE id = ?` という
    /// **正当な文**が入る（docs/03-data-model.md 不変条件2）。ここを落とすと、
    /// フェーズ2で走査ごと外される羽目になる。**通すのはこの形だけ**で、
    /// `SET` 句に別の列を並べる形（`SET superseded_by = ?, content = ?`）も、
    /// `WHERE` 句が1行を指さない形（`WHERE character_id = ?` / `WHERE` 無し /
    /// `WHERE id = ? OR ...`）も落ちる。判定は `updates_the_successor_of_one_row`。
    ///
    /// # **この走査が「後継の印を消すこと」に対して守れる範囲**（14a で測り直した）
    ///
    /// **守れる:** 後継の印を消す文が**ソースに綴られている**形。
    ///
    /// * `superseded_by` に NULL を**綴って**代入する文
    ///   （`memory_repair` の `no_statement_clears_a_successor` が落とす。
    ///   **この doc でも語を隣接させて書かない**——書くとその走査が自分自身を落とす）
    /// * `UPDATE memory SET superseded_by = ? WHERE character_id = ?` のような、
    ///   **1行を指さない**更新（ここの許可形が落とす）
    ///
    /// **守れない:** `UPDATE memory SET superseded_by = ? WHERE id = ?` の
    /// **プレースホルダに `None` を束ねる形**（`.bind(None::<i64>)`）。
    /// SQL 文そのものは正当な上書きと**1バイトも違わない**ので、
    /// **テキスト走査では原理的に見えない。** これは「塞いだ」のではなく
    /// **「文の形は縛った、束ねる値は縛れていない」**である。
    /// 縛るとしたら型の側（`superseded_by` に束ねる値を `MemoryId` に限る）だが、
    /// 現時点では**この1文が `UPDATE_SUPERSEDED_BY` の1箇所しか無い**ことに寄りかかっている。
    ///
    /// # この網に掛からないもの（把握したうえで残している）
    ///
    /// **この走査は「文字列リテラルに書かれた SQL」しか見ない。** 以下は素通りする。
    ///
    /// * **非リテラルの組み立て。** `format!` で継ぎ足す形は、**中身が全部リテラルでも**
    ///   素通りする。接尾辞側（`format!("{INSERT_MEMORY} ON CONFLICT (id) DO UPDATE
    ///   SET content = ?")` の後半）に**テーブル名が現れない**ため、そもそも
    ///   言及として拾われない。SQL を定数から組み立てないこと。
    /// * **親テーブル経由の CASCADE 削除。** `DELETE FROM character WHERE id IN
    ///   (SELECT character_id FROM memory ...)` は、`memory` の言及だけ見れば
    ///   `FROM memory`（許可形）で、`DELETE` は別の文脈にある。キャラクター削除に
    ///   伴う記憶の消失は設計として正当ともいえるので**この網では止めない**が、
    ///   「守られている」と誤解しないこと。
    /// * **`tests/` 配下。** 統合テストは本番バイナリにリンクされないため経路が無い。
    ///
    /// 逆に、**抽出そのものが追いつけなくなったら `extraction_hazards` が落とす**。
    /// 素通りを「気づかないまま増やす」方向の穴だけは塞いである。
    /// 生文字列（`r#"UPDATE "memory" SET ..."#`）は**かつての抜け道**だったが、
    /// 今は字句として読むので落ちる（`literals_that_used_to_break_extraction_...`）。
    #[test]
    fn no_statement_updates_or_deletes_a_memory() {
        let table = table_word();
        let targets = scan_targets();

        assert!(
            targets.len() >= 5,
            "走査対象が少なすぎるなら、この assert は何も守っていない: {:?}",
            targets.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );

        let mut mentions = 0;
        let mut found = Vec::new();
        for (path, fragments) in &targets {
            for fragment in fragments {
                mentions += mentions_of(fragment, &table);
                for violation in violations(fragment, &table) {
                    found.push(format!("{path}: {violation}"));
                }
            }
        }

        assert!(
            found.is_empty(),
            "記憶を潰しうる文があります:\n{}",
            found.join("\n")
        );
        // **言及を1件も拾えていないなら、走査が壊れている。**
        // 内訳は INSERT / SELECT の2つと、マイグレーションの DDL 3つ。
        assert!(
            mentions >= 5,
            "テーブル名の言及を拾えていない（抽出が壊れている）: {mentions} 件"
        );
    }

    /// **既知の回避手段がすべて落ちる。**
    ///
    /// ここに並んでいるのは、禁止語の列挙を素通りしていた実例そのもの。
    /// 判定を書き換えるときは、この一覧が緑のままであることを必ず確かめること。
    #[test]
    fn the_scan_catches_every_known_evasion() {
        let table = table_word();
        let name = table.to_lowercase();
        let evasions = [
            // 列の順序を入れ替えただけで隣接しなくなる形。
            format!("UPDATE {name} SET locked = 1, content = ?"),
            format!("UPDATE {name} SET category = ?, content = ?"),
            format!("UPDATE {name} SET superseded_by = ?, content = ?"),
            // **`SET` 句は正当な形のまま、`WHERE` 句だけで「切る」形。**
            // 走査は WHERE の中身を見ていなかったので、次の3つは素通りしていた
            // （14a のレビューで実測）。Rust 側から `.bind(None::<i64>)` を渡せば、
            // **そのキャラクターの後継の印が一度に全部消える**——
            // 上書きしたはずの古い記憶が、現在の事実として想起へ戻る。
            format!("UPDATE {name} SET superseded_by = ? WHERE character_id = ?"),
            format!("UPDATE {name} SET superseded_by = ? WHERE superseded_by IS NOT NULL"),
            format!("UPDATE {name} SET superseded_by = ?"),
            // 1行を指す形に**足す**のも同じこと（`OR` は絞り込みを広げる）。
            format!("UPDATE {name} SET superseded_by = ? WHERE id = ? OR character_id = ?"),
            format!("UPDATE {name} SET superseded_by = ? WHERE id IN (SELECT id FROM {name})"),
            // `DELETE` の文字が現れないのに行が消える形。
            format!("INSERT OR REPLACE INTO {name} (character_id, content) VALUES (?, ?)"),
            format!("REPLACE INTO {name} (character_id, content) VALUES (?, ?)"),
            format!(
                "INSERT INTO {name} (character_id, content) VALUES (?, ?) \
                 ON CONFLICT (character_id) DO UPDATE SET content = excluded.content"
            ),
            // 別名・修飾・OR 句でテーブル名の直前直後がずれる形。
            format!("UPDATE {name} AS m SET content = ?"),
            format!("UPDATE OR REPLACE {name} SET content = ?"),
            format!("UPDATE main.{name} SET content = ?"),
            format!("DELETE FROM main.{name} WHERE id = ?"),
            // 以前から落とせていた形も回帰として残す。
            format!("UPDATE {name} SET content = ?"),
            format!("UPDATE {name}   SET   content = ?"),
            format!("UPDATE\t{name}\tSET\tcontent = ?"),
            format!("UPDATE \"{name}\" SET \"content\" = ?"),
            format!("DELETE FROM {name} WHERE superseded_by IS NOT NULL"),
            format!("DROP TABLE {name}"),
        ];

        for evasion in &evasions {
            assert!(
                !violations(evasion, &table).is_empty(),
                "この形を落とせていません: {evasion}"
            );
        }
    }

    /// **文の区切りをまたいだ回避が落ちる。**
    ///
    /// `.sql` ファイルは1断片としてトークナイズされるため、判定を「直前の語」だけで
    /// 行うと**前の文の末尾**が現在の文の文脈として読まれる。実際に次が素通りしていた:
    ///
    /// * `PRAGMA foreign_keys = ON;` の直後の `DELETE FROM memory ...`
    ///   （`ON DELETE` 除外が前の文の `ON` で成立し、残る `FROM` は無条件許可）
    /// * `ON DELETE FROM memory`（同上を1文で書いたもの）
    /// * 全列を書く upsert（`CONFLICT` がテーブル名から遠く、抜粋の窓の外に落ちる）
    ///
    /// **`PRAGMA foreign_keys = ON;` はデータ整理マイグレーションの冒頭に書かれる
    /// 最有力候補**で、走査がマイグレーションを外さない理由そのものの場所だった。
    #[test]
    fn the_scan_is_not_fooled_by_statement_boundaries() {
        let table = table_word();
        let name = table.to_lowercase();
        let evasions = [
            // 前の文の末尾の `ON` を借りる形。
            format!(
                "PRAGMA foreign_keys = ON;\nDELETE FROM {name} WHERE superseded_by IS NOT NULL;"
            ),
            format!("PRAGMA legacy_alter_table = ON;\nDELETE FROM {name};"),
            // 1文で同じことをする形。`ON DELETE` の参照アクションに `FROM` は来ない。
            format!("ON DELETE FROM {name} WHERE id = ?"),
            // 全列を書く upsert。**最も素直な書き方が最も落ちやすい形**だった。
            format!(
                "INSERT INTO {name} (id, character_id, subject, content, category, created_at, locked, superseded_by) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (id) DO UPDATE SET content = excluded.content"
            ),
        ];

        for evasion in &evasions {
            assert!(
                !violations(evasion, &table).is_empty(),
                "この形を落とせていません: {evasion}"
            );
        }
    }

    /// **設計上ありうる形は落とさない。**
    ///
    /// 特に `UPDATE ... SET superseded_by` を落とすと、フェーズ2で走査ごと外される。
    #[test]
    fn the_scan_accepts_the_statements_the_design_allows() {
        let table = table_word();
        let name = table.to_lowercase();
        let allowed = [
            format!(
                "INSERT INTO {name} (character_id, subject, content, category) VALUES (?, ?, ?, ?)"
            ),
            format!(
                "SELECT id, content FROM {name} WHERE character_id = ? AND superseded_by IS NULL"
            ),
            // フェーズ2の正当な上書き。
            format!("UPDATE {name} SET superseded_by = ? WHERE id = ?"),
            // migrations の DDL。
            format!("CREATE TABLE {name} (id INTEGER PRIMARY KEY)"),
            format!("CREATE INDEX {name}_character_idx ON {name} (character_id, created_at)"),
            format!(
                "FOREIGN KEY (superseded_by, character_id) REFERENCES {name}(id, character_id)"
            ),
            // 外部キーの参照アクション。`ON DELETE CASCADE` は DML ではないので通す。
            format!(
                "CREATE TABLE {name} (id INTEGER PRIMARY KEY, \
                 character_id INTEGER NOT NULL REFERENCES character(id) ON DELETE CASCADE)"
            ),
            // 前の文に `ON` で終わる PRAGMA があっても、読み出しは読み出しのまま。
            format!("PRAGMA foreign_keys = ON;\nSELECT id FROM {name} WHERE character_id = ?;"),
        ];

        for statement in &allowed {
            let violations = violations(statement, &table);
            assert!(
                violations.is_empty(),
                "正当な文を落としています: {statement} / {violations:?}"
            );
        }
    }

    /// **走査は Rust の文字列リテラルから SQL を取り出す。**
    ///
    /// コメントは対象外（このモジュールの doc は禁止したい文をそのまま引用している）。
    /// 行継続で語が分断されていても、Rust と同じ規則で詰めてから判定する。
    #[test]
    fn the_scan_reads_sql_out_of_rust_string_literals() {
        let table = table_word();
        let name = table.to_lowercase();

        let source = format!("const BAD: &str = \"UPDATE \\\n     {name} SET content = ?\";\n");
        let fragments = rust_sql_fragments(&source);
        assert_eq!(
            fragments.len(),
            1,
            "文字列リテラルを取り出せること: {fragments:?}"
        );
        assert!(
            !violations(&fragments[0], &table).is_empty(),
            "行継続では逃げられないこと: {fragments:?}"
        );

        let marker = line_comment_marker();
        let commented = format!("{marker} DELETE FROM {name} WHERE id = ?\nlet answer = 42;\n");
        assert!(
            rust_sql_fragments(&commented).is_empty(),
            "コメントは SQL として読まないこと"
        );
    }

    /// **かつて抽出を壊していた書き方が、今は後続を隠さない。**
    ///
    /// 走査対象が `src/**` に広がったので、これらは**禁止できない書き方**になった。
    /// URL を含むリテラル（`http:` の後ろが行コメント記号）は `llm` / `server` に
    /// 現実に何十個もあり、素朴な行コメント除去では**そこで引用符の個数パリティが
    /// 反転して以降のファイル全体がずれる。** 後続の `DELETE FROM memory` が
    /// 丸ごと消えても**テストは緑のまま**という壊れ方をしていた。
    ///
    /// **ここが赤なら、その形の後ろに書かれた SQL は走査の外にある。**
    #[test]
    fn literals_that_used_to_break_extraction_no_longer_hide_the_next_statement() {
        let table = table_word();
        let name = table.to_lowercase();
        let marker = line_comment_marker();

        // レビュアーの実測例そのもの。以前は2行目の DELETE が抽出から消えていた。
        let with_url = format!(
            "const URL: &str = \"http:{marker}127.0.0.1:1234\";\nconst BAD: &str = \"DELETE FROM {name}\";\n"
        );
        // ブロックコメント中の引用符も、以前は以降をずらしていた。
        let with_block_comment =
            format!("/* \"よくある誤り\" */\nconst BAD: &str = \"DELETE FROM {name}\";\n");
        // 生文字列は `#` で断片が割れ、識別子クォート付きの SQL がすり抜けていた。
        let with_raw_string =
            format!("const BAD: &str = r#\"UPDATE \"{name}\" SET \"content\" = ?\"#;\n");
        // 生文字列の中の行コメント記号・引用符も同じ。
        let with_raw_url = format!(
            "const URL: &str = r\"http:{marker}127.0.0.1\";\nconst BAD: &str = \"DELETE FROM {name}\";\n"
        );
        // **生識別子は生文字列ではない。** `#` だけを見て生文字列と誤読すると、
        // `r#type` が1つあるだけでファイルの残りが読み捨てられる。
        let with_raw_identifier = format!(
            "let r#type = 1;\nlet r#match = 2;\nconst BAD: &str = \"DELETE FROM {name}\";\n"
        );

        for source in [
            &with_url,
            &with_block_comment,
            &with_raw_string,
            &with_raw_url,
            &with_raw_identifier,
        ] {
            assert!(
                extraction_hazards(source).is_empty(),
                "正当な書き方を報告しないこと: {source}"
            );
            let fragments = rust_sql_fragments(source);
            assert!(
                fragments
                    .iter()
                    .any(|fragment| !violations(fragment, &table).is_empty()),
                "後続の破壊的な文を見落としています: {source} → {fragments:?}"
            );
        }
    }

    /// **閉じていない構文は、黙って無効化させずに落とす。**
    ///
    /// 走査は Rust の字句をほぼそのまま辿るので、正しいソースなら起きない。
    /// 起きるとすれば**辿り方が現実の構文に追いつけなくなった**ときで、そのとき
    /// 以降のファイル全体がずれる（後続の `DELETE FROM memory` が丸ごと消える）。
    /// 静かに無効化されるより、**うるさく落ちる**方を選ぶ。
    #[test]
    fn an_unclosed_construct_is_reported() {
        let name = table_word().to_lowercase();

        for broken in [
            format!("const BAD: &str = \"DELETE FROM {name}\n"),
            // 閉じの `#` が足りない生文字列。ファイル末尾まで中身として読まれる。
            format!("const BAD: &str = r#\"DELETE FROM {name}\";\nconst OK: &str = \"x\";\n"),
            format!("/* 閉じ忘れ\nconst BAD: &str = \"DELETE FROM {name}\";\n"),
        ] {
            assert!(
                !extraction_hazards(&broken).is_empty(),
                "閉じていない構文を報告すること: {broken}"
            );
        }
    }

    /// **実ファイルのどれも抽出を壊していない。**
    ///
    /// ここが赤なら、走査そのものが（そのファイルの途中から）効いていない。
    #[test]
    fn no_scanned_source_breaks_literal_extraction() {
        let sources = rust_sources();
        assert!(
            // `src/**` 全体が対象。`src/store` だけに戻ると 3 件程度まで落ちる。
            sources.len() >= 15,
            "走査対象の Rust ソースが少なすぎる: {:?}",
            sources.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );

        let mut found = Vec::new();
        for (path, source) in &sources {
            for hazard in extraction_hazards(source) {
                found.push(format!("{path}: {hazard}"));
            }
        }
        assert!(
            found.is_empty(),
            "抽出を壊す書き方があります（走査が静かに無効化されます）:\n{}",
            found.join("\n")
        );
    }

    /// **マイグレーションも走査する。**
    ///
    /// `DELETE FROM memory WHERE superseded_by IS NOT NULL` は「もう上書き済みだから
    /// 消してよい行」を選ぶ最も自然なクエリでありながら、**保持したい行だけを正確に
    /// 消す。** その置き場所として最もありそうなのがデータ整理のマイグレーション。
    #[test]
    fn the_scan_covers_migrations_not_just_rust() {
        let roots: Vec<&str> = SCAN_ROOTS.iter().map(|(root, _)| *root).collect();
        assert!(
            roots.contains(&"migrations"),
            "走査対象に migrations を含めること: {roots:?}"
        );

        let table = table_word();
        let name = table.to_lowercase();
        let migration = format!(
            "-- 上書き済みの記憶を整理する\nDELETE FROM {name} WHERE superseded_by IS NOT NULL;\n"
        );
        let fragment = strip_line_comments(&migration, "--");
        assert!(
            !violations(&fragment, &table).is_empty(),
            "マイグレーションの整理 SQL を落とすこと: {fragment}"
        );

        // 実ファイルが対象に入っていることも確かめる（拡張子の取り違え検出）。
        let scanned = scan_targets();
        let sql_targets: Vec<&(String, Vec<String>)> = scanned
            .iter()
            .filter(|(path, _)| path.ends_with(".sql"))
            .collect();
        assert!(
            sql_targets.len() >= 2,
            "migrations の .sql を拾えていない: {:?}",
            scanned.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );

        // **一覧に載っているだけでは足りない。** 中身がトークンとして読めていることまで
        // 確かめないと、読み取りが空文字を返していても緑になる。
        // `0001_initial.sql` の CREATE TABLE / CREATE INDEX / REFERENCES で3件。
        let mentions: usize = sql_targets
            .iter()
            .flat_map(|(_, fragments)| fragments)
            .map(|fragment| mentions_of(fragment, &table))
            .sum();
        assert!(
            mentions >= 3,
            "マイグレーションの中身を読めていない: {mentions} 件"
        );
    }

    /// **走査は `src/**` 全体を見る。`store` に閉じない。**
    ///
    /// ADR-0008 は「`server` に生の SQL を書けば走査の外に出る」と明示している。
    /// 対象が `src/store` だけだと、`src/server/admin.rs` に
    /// `DELETE FROM memory ...` を書くだけで**この見張り番を回避できる**
    /// （しかも回避したことは誰にも見えない）。人間向けのコメントで止めない。
    #[test]
    fn the_scan_covers_every_module_not_just_the_store() {
        let roots: Vec<&str> = SCAN_ROOTS.iter().map(|(root, _)| *root).collect();
        assert!(
            roots.contains(&"src"),
            "走査対象を src の一部に絞らないこと: {roots:?}"
        );

        // 実ファイルとして `store` 以外のモジュールが対象に入っていること。
        let separator = std::path::MAIN_SEPARATOR;
        let scanned = scan_targets();
        for module in ["server", "prompt", "llm", "domain", "parse"] {
            let expected = format!("src{separator}{module}{separator}");
            assert!(
                scanned.iter().any(|(path, _)| path.contains(&expected)),
                "{module} が走査対象に入っていない: {:?}",
                scanned.iter().map(|(path, _)| path).collect::<Vec<_>>()
            );
        }

        // 中身も読めている（`server` に SQL を書けば実際に掛かる）。
        let table = table_word();
        let name = table.to_lowercase();
        let planted = format!("const BAD: &str = \"DELETE FROM {name} WHERE id = ?\";\n");
        let fragments = rust_sql_fragments(&planted);
        assert!(
            fragments
                .iter()
                .any(|fragment| !violations(fragment, &table).is_empty()),
            "生の SQL を落とせること: {fragments:?}"
        );
    }

    /// 走査対象を「パス, SQL 断片の並び」で返す。
    fn scan_targets() -> Vec<(String, Vec<String>)> {
        let mut targets = Vec::new();
        for (root, extension) in SCAN_ROOTS {
            for (path, source) in source_files(root, extension) {
                let fragments = if extension == "rs" {
                    rust_sql_fragments(&source)
                } else {
                    vec![strip_line_comments(&source, "--")]
                };
                targets.push((path, fragments));
            }
        }
        targets
    }

    /// 走査対象の Rust ソースを「パス, 生ソース」で返す。
    ///
    /// **抽出後ではなく生ソース**を返す。抽出を壊す書き方の検査は、
    /// 抽出そのものを信用できない前提で行う必要がある。
    fn rust_sources() -> Vec<(String, String)> {
        SCAN_ROOTS
            .iter()
            .filter(|(_, extension)| *extension == "rs")
            .flat_map(|(root, extension)| source_files(root, extension))
            .collect()
    }

    /// `root` 配下の拡張子 `extension` のファイルを「パス, 生ソース」で返す。
    fn source_files(root: &str, extension: &str) -> Vec<(String, String)> {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        let mut stack = vec![manifest.join(root)];

        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir).expect("走査対象を読めること");
            for entry in entries {
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

    /// **抽出そのものを壊す書き方**を報告する（見つかれば走査テストが落ちる）。
    ///
    /// # 何を報告するのか
    ///
    /// 報告するのは**ファイルの末尾で閉じていない構文**（文字列・生文字列・
    /// ブロックコメント）だけ。走査は `scan_rust_source` が Rust とほぼ同じ規則で
    /// 字句を辿るので、正しい Rust ソースなら普通は起きない。
    ///
    /// 起きるとすれば**辿り方が現実の構文に追いつけなくなった**ときで、
    /// そのとき壊れ方は「その行から先が読まれない」では済まず、
    /// **以降のファイル全体がずれて後続の `DELETE FROM memory` が丸ごと消える**
    /// （テストは緑のまま）。走査が静かに無効化されるより、**うるさく落ちる**方を選ぶ。
    ///
    /// # かつて報告していたもの（今は正しく読める）
    ///
    /// * 文字列リテラル中の行コメント記号（`"http:<略>//127.0.0.1"` 等）
    /// * ブロックコメント中の引用符
    /// * 生文字列（`r#"UPDATE "memory" SET ..."#`）
    ///
    /// 走査対象が `src/**` に広がった時点で、これらは**禁止できない書き方**になった
    /// （URL を書けないソースツリーは成立しない）。報告する代わりに読めるようにしてある。
    fn extraction_hazards(source: &str) -> Vec<String> {
        scan_rust_source(source).1
    }

    /// Rust ソースを字句として辿り、`(文字列リテラルの中身, 閉じていない構文の報告)` を返す。
    ///
    /// 辿るもの: 行コメント / ブロックコメント（入れ子）/ 文字リテラル /
    /// 文字列リテラル（エスケープ・行継続）/ 生文字列（`r"..."` / `r#"..."#` / `br"..."`）。
    ///
    /// **素朴な「行コメントを落としてから引用符で区切る」実装に戻さないこと。**
    /// 文字列リテラルの中の `//`（URL）でその行の閉じ引用符が消え、
    /// **引用符の個数パリティが反転して以降のファイル全体がずれる。**
    fn scan_rust_source(source: &str) -> (Vec<String>, Vec<String>) {
        let chars: Vec<char> = source.chars().collect();
        let mut fragments = Vec::new();
        let mut hazards = Vec::new();
        let mut at = 0;
        let mut line = 1;

        while at < chars.len() {
            // 行コメント（doc コメントを含む）。行末まで読み飛ばす。
            if chars[at] == '/' && chars.get(at + 1) == Some(&'/') {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
                continue;
            }
            // ブロックコメント。入れ子を数える。**中の引用符は無視してよい。**
            if chars[at] == '/' && chars.get(at + 1) == Some(&'*') {
                let opened_at = line;
                let mut depth = 1;
                at += 2;
                while at < chars.len() && depth > 0 {
                    match (chars[at], chars.get(at + 1)) {
                        ('/', Some('*')) => {
                            depth += 1;
                            at += 2;
                        }
                        ('*', Some('/')) => {
                            depth -= 1;
                            at += 2;
                        }
                        (ch, _) => {
                            if ch == '\n' {
                                line += 1;
                            }
                            at += 1;
                        }
                    }
                }
                if depth > 0 {
                    hazards.push(format!(
                        "{opened_at} 行目: ブロックコメントが閉じていません（以降の抽出がずれます）"
                    ));
                }
                continue;
            }
            // 識別子。**まとめて読み飛ばす**（`char` の末尾の `r` を生文字列の
            // 接頭辞と読み違えないため）。`r` / `br` の直後が `#` か `"` なら生文字列。
            if is_ident_char(chars[at]) {
                let start = at;
                while at < chars.len() && is_ident_char(chars[at]) {
                    at += 1;
                }
                let word: String = chars[start..at].iter().collect();
                if !matches!(word.as_str(), "r" | "br") {
                    continue;
                }
                // **`#` があるだけでは生文字列ではない**（`r#type` のような生識別子）。
                // 引用符まで見てから判定しないと、生識別子1つでファイルの残りを
                // 読み捨ててしまう。
                let Some(hashes) = raw_string_hashes(&chars, at) else {
                    continue;
                };
                let opened_at = line;
                match raw_string_body(&chars, at + hashes + 1, hashes) {
                    Some((literal, next)) => {
                        line += literal.chars().filter(|ch| *ch == '\n').count();
                        fragments.push(literal);
                        at = next;
                    }
                    None => {
                        hazards.push(format!(
                            "{opened_at} 行目: 生文字列が閉じていません（以降の抽出がずれます）"
                        ));
                        at = chars.len();
                    }
                }
                continue;
            }
            match chars[at] {
                '\n' => {
                    line += 1;
                    at += 1;
                }
                // 文字リテラルは読み飛ばす。`'"'` を書かれると引用符の対応が
                // 丸ごとずれ、以降の SQL が読めなくなるため。
                '\'' => at += char_literal_len(&chars, at),
                '"' => {
                    let opened_at = line;
                    let mut literal = String::new();
                    at += 1;
                    let mut closed = false;
                    while at < chars.len() {
                        if chars[at] == '"' {
                            closed = true;
                            at += 1;
                            break;
                        }
                        if chars[at] != '\\' {
                            if chars[at] == '\n' {
                                line += 1;
                            }
                            literal.push(chars[at]);
                            at += 1;
                            continue;
                        }
                        match chars.get(at + 1) {
                            // 行継続。Rust は改行と次行の先頭空白を取り除くので、
                            // **同じように詰める。** ここで空白を残すと
                            // `UPDATE \<改行> memory` が別の語のまま判定される。
                            Some('\n' | '\r') => {
                                // バックスラッシュを飛ばし、続く空白（改行を含む）を詰める。
                                at += 1;
                                while chars.get(at).is_some_and(|ch| ch.is_whitespace()) {
                                    if chars[at] == '\n' {
                                        line += 1;
                                    }
                                    at += 1;
                                }
                            }
                            // それ以外のエスケープ（`\"` 等）は区切りとして空白に潰す。
                            _ => {
                                literal.push(' ');
                                at += 2;
                            }
                        }
                    }
                    if closed {
                        fragments.push(literal);
                    } else {
                        hazards.push(format!(
                            "{opened_at} 行目: 文字列リテラルが閉じていません（以降の抽出がずれます）"
                        ));
                    }
                }
                _ => at += 1,
            }
        }
        (fragments, hazards)
    }

    /// 識別子に使える文字か（生文字列の接頭辞の判定に使う）。
    fn is_ident_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    /// 生文字列の開始なら `#` の個数を返す。開始でなければ `None`。
    ///
    /// `at` は接頭辞（`r` / `br`）の直後を指す。**`#` の後ろに `"` が無ければ
    /// 生識別子**（`r#type`）なので、生文字列として扱わない。
    fn raw_string_hashes(chars: &[char], at: usize) -> Option<usize> {
        let mut hashes = 0;
        while chars.get(at + hashes) == Some(&'#') {
            hashes += 1;
        }
        (chars.get(at + hashes) == Some(&'"')).then_some(hashes)
    }

    /// 生文字列の中身と、その次の位置を返す。閉じていなければ `None`。
    ///
    /// `start` は開き引用符の直後、`hashes` は開始側の `#` の個数。
    fn raw_string_body(chars: &[char], start: usize, hashes: usize) -> Option<(String, usize)> {
        let mut index = start;
        while index < chars.len() {
            if chars[index] == '"'
                && (1..=hashes).all(|offset| chars.get(index + offset) == Some(&'#'))
            {
                let literal: String = chars[start..index].iter().collect();
                return Some((literal, index + hashes + 1));
            }
            index += 1;
        }
        None
    }

    /// Rust の行コメント記号を**ソース上に綴らずに**組み立てる。
    ///
    /// `table_word` と同じ理由。この記号をリテラルで書くと、行コメント除去が
    /// そこで切って**このファイル自身の抽出を壊す**（`extraction_hazards` が検出する形）。
    fn line_comment_marker() -> String {
        ["/", "/"].concat()
    }

    /// 行コメント（`--`）以降を落とす。**`.sql` ファイル専用。**
    ///
    /// **リテラルの中に `--` が現れるとその行の残りが落ちる。** マイグレーションに
    /// `'--'` を含む文字列リテラルを書かないこと。
    ///
    /// **Rust ソースにこれを使わないこと。** URL（`http:` + 行コメント記号）を含む
    /// リテラルで閉じ引用符が消え、**引用符の個数パリティが反転して以降のファイル全体が
    /// ずれる。** Rust 側は `scan_rust_source` が字句として辿る。
    fn strip_line_comments(source: &str, marker: &str) -> String {
        source
            .lines()
            .map(|line| line.split(marker).next().unwrap_or(""))
            .collect::<Vec<&str>>()
            .join("\n")
    }

    /// Rust ソースから SQL 断片（文字列リテラルの中身）だけを取り出す。
    ///
    /// ソース全体を走査しないのは、`memory.content` のような**変数名まで
    /// テーブル名の言及として拾ってしまう**ため。SQL は必ず文字列リテラルにあるので、
    /// そこだけを見れば許可リストが成立する。
    ///
    /// リテラルは**1つずつ別の断片**として返す。連結して1本にすると、隣り合う
    /// 無関係なリテラルが繋がって偽陽性・偽陰性の両方を生む。逆に
    /// `"DELETE FROM " + "memory"` のような分割は、単独の `memory` が
    /// 許可された形に当たらないので落ちる。
    ///
    /// **生文字列も断片として読む**（`r"..."` / `r#"..."#` / `br"..."`）。
    /// かつては `#` の位置で断片が割れ、識別子クォートを含む SQL
    /// （`UPDATE "memory" SET ...` を生文字列で書いた形）が検出をすり抜けていた。
    fn rust_sql_fragments(source: &str) -> Vec<String> {
        scan_rust_source(source).0
    }

    /// `at` から始まる文字リテラルの長さ。文字リテラルでなければ 1（ライフタイム注釈等）。
    fn char_literal_len(chars: &[char], at: usize) -> usize {
        let escaped = chars.get(at + 1) == Some(&'\\');
        if escaped && chars.get(at + 3) == Some(&'\'') {
            4
        } else if !escaped && chars.get(at + 2) == Some(&'\'') {
            3
        } else {
            1
        }
    }

    /// SQL 断片をトークン列に均す。
    ///
    /// * 語は `[A-Za-z0-9_]` の連なり。大文字に揃える（SQL は大小を区別しない）。
    /// * それ以外は区切り。**カンマだけはトークンとして残す**
    ///   （`SET a = 1, b = 2` の「ついでの列」を見分けるため）。
    fn sql_tokens(fragment: &str) -> Vec<String> {
        let mut tokens = Vec::new();
        let mut word = String::new();

        for ch in fragment.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                word.push(ch.to_ascii_uppercase());
                continue;
            }
            if !word.is_empty() {
                tokens.push(std::mem::take(&mut word));
            }
            if ch == ',' {
                tokens.push(",".to_owned());
            }
        }
        if !word.is_empty() {
            tokens.push(word);
        }
        tokens
    }

    /// 断片中のテーブル名の言及数。
    fn mentions_of(fragment: &str, table: &str) -> usize {
        sql_tokens(fragment)
            .iter()
            .filter(|token| token.as_str() == table)
            .count()
    }

    /// 断片中の「許可されていないテーブル名の言及」を理由つきで返す。
    ///
    /// **判定の前に `;` で文へ割る。** `.sql` ファイルは1断片としてトークナイズされるため、
    /// 割らないと**前の文の末尾**が現在の文の文脈として読まれる
    /// （`PRAGMA foreign_keys = ON;` の直後の `DELETE` が `ON DELETE` 扱いになった）。
    fn violations(fragment: &str, table: &str) -> Vec<String> {
        statements(fragment)
            .iter()
            .flat_map(|statement| statement_violations(statement, table))
            .collect()
    }

    /// 断片を `;` で文に割る。
    fn statements(fragment: &str) -> Vec<String> {
        fragment.split(';').map(str::to_owned).collect()
    }

    /// 1文の中の違反を返す。
    ///
    /// **破壊的な語は文全体を見る**（窓で見ない）。窓にすると、テーブル名から遠い位置に
    /// 破壊語を置くだけで逃げられる。実際、全8列を書く upsert は `CONFLICT` が
    /// ちょうど窓の外に落ちて素通りしていた。
    fn statement_violations(statement: &str, table: &str) -> Vec<String> {
        let tokens = sql_tokens(statement);
        let mut found = Vec::new();

        for (index, token) in tokens.iter().enumerate() {
            if token.as_str() != table {
                continue;
            }
            let from = index.saturating_sub(LOOK_BEHIND);
            let to = (index + LOOK_AHEAD + 1).min(tokens.len());
            let excerpt = tokens[from..to].join(" ");

            if let Some(keyword) = destructive_in(&tokens) {
                found.push(format!("同じ文に {keyword} があります: {excerpt}"));
            } else if !form_is_allowed(&tokens, index) {
                found.push(format!("許可されていない形です: {excerpt}"));
            }
        }
        found
    }

    /// 文中に破壊的な語があるか。
    ///
    /// **外部キーの参照アクション（`ON DELETE CASCADE` 等）だけは除く。** DML ではないうえ、
    /// `migrations/0001_initial.sql` の `memory` テーブル自身が持っている
    /// （`character_id ... ON DELETE CASCADE`）。ここを除かないと常に赤になる。
    ///
    /// **除外は「直前が `ON`」だけでは足りない。** 参照アクションの次に来るのは
    /// `CASCADE` / `SET` / `RESTRICT` / `NO`（ACTION）で、**`FROM` は来ない**。
    /// この条件を落とすと `ON DELETE FROM memory` が除外側に転がり込む。
    fn destructive_in(tokens: &[String]) -> Option<&'static str> {
        for (index, token) in tokens.iter().enumerate() {
            let reference_action = token.as_str() == "DELETE"
                && index > 0
                && tokens[index - 1] == "ON"
                && tokens.get(index + 1).map_or("", String::as_str) != "FROM";
            if reference_action {
                continue;
            }
            if let Some(found) = DESTRUCTIVE.iter().find(|word| **word == token.as_str()) {
                return Some(found);
            }
        }
        None
    }

    /// テーブル名の言及が**許可された形**か。
    ///
    /// 判定は直前の語で行う。`INSERT OR REPLACE INTO` / `UPDATE OR REPLACE` /
    /// `UPDATE main.memory` / `UPDATE memory AS m` はいずれもここで落ちる。
    fn form_is_allowed(tokens: &[String], index: usize) -> bool {
        let before = |back: usize| -> &str {
            index
                .checked_sub(back)
                .and_then(|at| tokens.get(at))
                .map_or("", String::as_str)
        };
        let after = |ahead: usize| -> &str { tokens.get(index + ahead).map_or("", String::as_str) };

        match before(1) {
            // 追加。
            "INTO" => before(2) == "INSERT",
            // 読み出し。`DELETE FROM` は destructive_near が先に落とす。
            "FROM" => true,
            // 以下は migrations の DDL。
            "TABLE" => before(2) == "CREATE",
            "ON" => before(3) == "INDEX",
            "REFERENCES" => true,
            // フェーズ2の上書き。**`superseded_by` を1行だけに書く形しか通さない。**
            "UPDATE" => {
                after(1) == "SET"
                    && after(2) == "SUPERSEDED_BY"
                    && updates_the_successor_of_one_row(tokens, index)
            }
            _ => false,
        }
    }

    /// `UPDATE memory` の後ろが **`SET superseded_by = ? WHERE id = ?` そのもの**か。
    ///
    /// トークン列は記号を落とすので、判定は `[SET, SUPERSEDED_BY, WHERE, ID]` との**完全一致**。
    ///
    /// # なぜ `SET` 句だけ見ていては足りなかったのか
    ///
    /// 以前は「`SET` 句にカンマが無いこと」しか見ておらず、**`WHERE` 句の中身は素通り**だった。
    /// つまり `UPDATE memory SET superseded_by = ? WHERE character_id = ?` が許可形で、
    /// Rust 側から `.bind(None::<i64>)` を渡せば
    /// **そのキャラクターの後継の印を一度に全部消せた**（14a のレビューで実測）。
    /// 消えると、上書きしたはずの古い記憶が**現在の事実として想起へ戻る。**
    ///
    /// # 完全一致にしてある理由
    ///
    /// `WHERE id = ? AND ...` のような**足す**形まで許すと、`OR` を1つ書くだけで
    /// 絞り込みが広がる（`WHERE id = ? OR character_id = ?`）。
    /// 「どこまでが安全な追加か」を走査に判定させるより、
    /// **新しい形が要るならここを書き換えさせる**方が、レビューの掛かる場所が残る。
    fn updates_the_successor_of_one_row(tokens: &[String], index: usize) -> bool {
        tokens
            .get(index + 1..)
            .is_some_and(|tail| tail == ["SET", "SUPERSEDED_BY", "WHERE", "ID"])
    }
}
