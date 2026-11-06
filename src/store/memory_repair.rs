//! 記憶の**修復**（[ADR-0022] / 作業単位 14a）。
//!
//! **想起ではない。** 想起の入口は `Store::recall_memories` ひとつだけで
//! （ADR-0003 / ADR-0023）、このモジュールが返すものは**プロンプトに一切載らない。**
//! 名前（`list_memories_for_repair` / `supersede_memory`）も置き場所（このファイル）も
//! 想起と分けてあるのは、**混同されると「想起の入口が2本ある」状態に静かに変わる**ため。
//!
//! # 何を解いているのか
//!
//! **`memory` には更新も削除も無い**（CLAUDE.md 絶対制約）。それは意図した設計だが、
//! 13c-3b で「直せない故障」に変わった:
//!
//! * 記憶が1行でも `InvalidStoredValue` になると、そのキャラクターの `chat` が 500 になる。
//! * 抽出も記憶を読むので、抽出も止まる。
//! * **アプリ経由で直す手段が無かった**（**14a 以前の話。それを解いたのがこのモジュール**）。
//!
//! ADR-0022 の答えは「消す道は作らない。**いま何が正しいか**を書かせ、古い行にそれを指させる」:
//!
//! ```text
//! 新しい行を INSERT   →   古い行の superseded_by に、その id を入れる
//! ```
//!
//! 想起は `superseded_by IS NULL` の行しか読まないので、
//! **古い行を指させた瞬間に読み出し経路から外れる。行は消えない**（時系列保持）。
//!
//! # このモジュールが足す SQL は、既存の走査が許す形だけ
//!
//! `src/store/memory.rs` の `no_statement_updates_or_deletes_a_memory` は
//! **`src/**` と `migrations/**` の全体**を走査し、記憶を潰しうる文を落とす。
//! 許されているのは追加・読み出し・`UPDATE memory SET superseded_by = ? WHERE id = ?`・DDL だけで、
//! **`SET` 句に列をもう1つ並べても、`WHERE` 句が1行を指さなくても赤くなる**
//! （`SET superseded_by = ?, content = ?` も `WHERE character_id = ?` も落ちる）。
//! ここの文はその許可形の中に収めてある。**走査側を書き換えて通すことはしない。**
//!
//! [ADR-0022]: ../../../docs/adr/0022-repairing-memories-without-deleting.md

use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::memory::{INSERT_MEMORY, subject_from_db, subject_to_db};
use super::{Store, StoreError};
use crate::domain::{
    CharacterId, Memory, MemoryCategory, MemoryColumn, MemoryContent, MemoryEntry, MemoryId,
    NewMemory, SupersededMemory, UnreadableMemory,
};

/// **読みから始まって書きに昇格するトランザクションの開始文**（ADR-0022 §3）。
///
/// `supersede_memory` は「古い行の存在と持ち主を確かめる → INSERT / UPDATE」の形なので、
/// 素の `BEGIN`（DEFERRED）だと**最初の読みで読み取りスナップショットが固定され**、
/// 続く書きへの昇格が `SQLITE_BUSY_SNAPSHOT`(517) になる。
/// これは **`busy_timeout` の対象外で、待たずに即座に失敗する**。
///
/// **WAL のスナップショットは DB 全体を指す**ので、「同じ記憶を同時に訂正した場合だけ」の
/// 話ではない。**無関係な書き込みが1つ commit されただけで昇格が落ちる。**
/// `replace_stat_allocs` で実測済み（`src/store/character.rs::BEGIN_IMMEDIATE` の doc /
/// 315 周のうち 298 周が `database is locked`）。壊れ方が重いのは
/// **そこが唯一の復旧経路**だからで、**直したいときほど詰まる。**
///
/// **見張り番はこの定数を読むテストではなく、同時実行のテスト**
/// （`tests/store_memory.rs::two_corrections_of_the_same_memory_never_race_into_a_locked_database`）。
/// 定数と期待値を突き合わせるだけの検査は、**定数が緩めば期待値も一緒に動く**ので、
/// ここでは置かない。あちらは負けた側の**理由**（`MemoryAlreadySuperseded` か
/// `Query` か）まで照合するので、素の `BEGIN` に戻すと赤くなる。
///
/// **`character.rs` / `session.rs` の同名の定数と共有しない。** 中身は同じ文字列だが、
/// 「なぜここで要るのか」は関数ごとに別で、共有すると片方の理由が消えたときに
/// もう片方も一緒に外れる。
const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

/// 訂正の対象になる行の**持ち主と、既に付いている後継**だけを引く。
///
/// # **本文も分類も読まない**（この文の存在理由）
///
/// 修復の対象は**壊れた行**である。`MemoryContent::parse` を通す読み方をすると
/// **直したい行ほど読めない**（ADR-0016 §4 で `update_character` が同じ結論に達している）。
/// 引き当ては主キーだけで済ませ、書く値は引数の検証済みの型から取る。
///
/// 読む2列はどちらも**拒否の理由を決めるためだけ**に要る:
///
/// * `character_id` — 別のキャラクターの記憶を指していないか。
/// * `superseded_by` — 既に上書きされていないか（鎖を線形に保つ）。
///
/// **`locked` は読まない**（ADR-0022 §4。意味がまだ確定していないので、
/// 訂正の可否の判断に混ぜない）。
const SELECT_SUPERSEDE_TARGET: &str = "SELECT character_id, superseded_by FROM memory WHERE id = ?";

/// 古い行に後継を指させる**唯一の文**（docs/03-data-model.md 不変条件2 の「正当な上書き」）。
///
/// **`SET` 句に別の列を並べないこと。** 走査（`no_statement_updates_or_deletes_a_memory`）は
/// `superseded_by` の単独更新だけを通し、`SET superseded_by = ?, content = ?` は落とす。
/// **後者は「時系列保持」を破る**——古い行の本文が失われると
/// 「以前はXだったが今はY」が永久に復元できない。
///
/// **`WHERE` 句も1行を指す形しか通らない。** `WHERE character_id = ?` に変えると走査が落とす
/// ——その形は**そのキャラクターの後継の印を一度に全部書き換えられる**からで、
/// `.bind(None::<i64>)` を渡せば「切る」文になる（14a のレビューで実測）。
///
/// **`superseded_by` を NULL に戻す文は作らない**（後継の無い取り消しを作らない /
/// ADR-0022 §1）。この規則の見張り番は `no_statement_clears_a_successor`。
///
/// > **走査が守れない形が1つ残っている。** この文の**プレースホルダに `None` を束ねる**と、
/// > SQL 文は正当な上書きと同一のまま「切る」動作になる。
/// > **テキスト走査では原理的に見えない**ので、守っているのは
/// > 「後継を `correction_id`（直前に INSERT した行の id）でしか束ねていない」という
/// > `supersede_memory` の中身だけである。**塞いだのではなく、1箇所に閉じてある。**
const UPDATE_SUPERSEDED_BY: &str = "UPDATE memory SET superseded_by = ? WHERE id = ?";

/// 修復のための一覧が引く行。**有効な行だけ**（`superseded_by IS NULL`）。
///
/// # なぜ上書き済みの行を並べないのか
///
/// **修復の対象は有効な行だけ**である。上書き済みの行はもう想起に載らないので
/// 害が無く、並べると「どれを直せばよいか」が薄まる。
///
/// > **⚠ 「履歴を見たいときは DB を直接見る」は D-3b で事実でなくなった**（2026-08-27）。
/// > **読む入口は [`SELECT_SUPERSEDED_MEMORY_ROWS`] にある。**
/// > ここが有効な行だけを引く理由は「履歴を見せない」ではなく
/// > **「修復の対象に、もう害の無い行を混ぜない」**である。**条件は変えないこと。**
///
/// # `SELECT_ACTIVE_MEMORIES`（想起の文）と共有しない
///
/// 現時点では中身が同じだが、**理由が別**である。あちらは「上書き済みの記憶を
/// 現在の事実としてプロンプトに載せない」ため、こちらは「もう害の無い行を
/// 修復対象に混ぜない」ため。共有すると、片方の理由が消えたときに
/// もう片方も一緒に外れる（`BEGIN_IMMEDIATE` を各所で分けてあるのと同じ判断）。
///
/// 並びは想起と同じ作成順。`created_at` はミリ秒精度で同値になりうるので
/// `id` を第2キーに置く（**1ターンで最大5件入る**ので同着は普通に起きる）。
/// 見張り番は想起側と同じ
/// `tests/store_memory.rs::memories_created_in_the_same_millisecond_are_ordered_by_id`
/// （**同じフィクスチャで両方の文を見る**——同着の組は `add_memory` からは作れない）。
const SELECT_ACTIVE_MEMORY_ROWS: &str = "SELECT id, subject, content, category, created_at, locked FROM memory \
     WHERE character_id = ? AND superseded_by IS NULL ORDER BY created_at, id";

/// **上書きされた記憶を引く文**（ADR-0022 / 作業単位 D-3b）。**新しい順に最大 `?2` 件。**
///
/// # なぜ [`SELECT_ACTIVE_MEMORY_ROWS`] を組み替えないのか
///
/// あの文は**想起から外れていない行**を引く。条件を問い合わせで組み替えられる形にすると、
/// **修復の一覧の絞り込みと、この読みの絞り込みが1つの文の中で綱引き**になる
/// ——片方を直すともう片方が黙って変わる。
/// **`WHERE` を実行時に組み立てないこと**（絶対制約: SQL は文字列連結で組まない）。
/// 別の文にしてあるので、**どちらの条件も定数のまま**である。
///
/// # `superseded_by` を**読む**（`IS NOT NULL` で絞るだけでは足りない）
///
/// **どの行に置き換わったかが分からなければ、それは「時系列」ではなく
/// ただの「消えた記憶の山」**である。鎖を辿れることがこの読みの存在理由。
///
/// # 並びは**新しい順**（`created_at DESC, id DESC`）
///
/// 修復の一覧（古い順）と**逆**なのは、用途が違うから——あちらは
/// 「上から順に直していく」一覧、こちらは「**さっき何を直したか**」を見る一覧で、
/// 上限で切るのも新しい側を残すためである。
///
/// **第2キー（`id`）を落とさないこと。** `created_at` はミリ秒精度で同値になりうるうえ、
/// **1ターンで最大5件の記憶が入る**（`NewMemory::MAX_PER_TURN`）ので、
/// それらをまとめて訂正すれば同着が普通に起きる。
/// 見張り番は `tests/store_memory.rs::superseded_rows_come_back_newest_first_with_ties_broken_by_id`
/// （**索引を落とした状態でも同じ並び**であることまで見る。
/// `memory_character_idx (character_id, created_at)` を後ろから辿る計画では
/// 同着の中が偶然 `id` の降順になるので、索引があるうちは第2キーを守れない）。
///
/// # 並ぶのは「訂正した時刻」ではない
///
/// **いつ上書きされたかは保存していない**（`superseded_by` を書くだけで、
/// `updated_at` の類は持たない）。並ぶのは**その記憶が書かれた時刻**である。
const SELECT_SUPERSEDED_MEMORY_ROWS: &str = "SELECT id, subject, content, category, created_at, locked, superseded_by FROM memory \
     WHERE character_id = ? AND superseded_by IS NOT NULL ORDER BY created_at DESC, id DESC LIMIT ?";

/// **上書きされた記憶の総数**（作業単位 D-3b）。**上限で切らない。**
///
/// # なぜ「返した件数」で代用しないのか
///
/// 一覧は上限で切られている（[`SELECT_SUPERSEDED_MEMORY_ROWS`] の `LIMIT`）ので、
/// 件数を総数と呼ぶと**画面が「これで全部」と嘘をつく。**
/// **行は消えないので単調増加する**（ADR-0022 §結果）——上限に当たるのは異常事態ではない。
///
/// **読めない行も数に入る。** 数えるだけで、主語も本文も分類も読まないため
/// ——一覧の側が壊れた行を指し示している間も、総数は返り続ける。
const COUNT_SUPERSEDED_MEMORIES: &str =
    "SELECT count(*) AS memories FROM memory WHERE character_id = ? AND superseded_by IS NOT NULL";

impl Store {
    /// **訂正版を書き、古い行にそれを指させる**（ADR-0022）。追加した行の id を返す。
    ///
    /// ```text
    /// 新しい行を INSERT   →   古い行の superseded_by に、その id を入れる
    /// ```
    ///
    /// 古い行は**消えない**。想起（`recall_memories`）から外れるだけで、
    /// 本文も分類も作成日時もそのまま残る（「以前はXだったが今はY」の材料）。
    ///
    /// # **「消す」道は作らない**（ADR-0022 §1）
    ///
    /// 後継は必須の引数で、**後継の無い取り消しは表現できない。**
    /// 壊れた行（本文が空白のみ等）でも訂正版を書かせる立場を取る
    /// ——「消したい」は「今は何が正しいか」を言えるはず、という判断。
    ///
    /// > **循環は構造的に作れない。** 後継は**必ずこの呼び出しの中で新しく作る行**なので、
    /// > 後継が古い行を先に指していることはありえない。
    /// > スキーマが防げるのは自己参照（`CHECK (superseded_by <> id)`）までで、
    /// > `1→2` かつ `2→1` は CHECK では止まらない
    /// > （docs/03-data-model.md）。**ここで作れないことが唯一の担保。**
    ///
    /// # 書き込みは1つのトランザクション（ADR-0022 §3）
    ///
    /// INSERT と UPDATE は**両方入るか、どちらも入らないか**。片方だけ通ると
    /// **後継のない訂正版**（同じ事実が2件想起される）か、
    /// **存在しない行を指す古い行**（想起から消えたきり戻らない）が残る。
    /// 開始文が `BEGIN IMMEDIATE` である理由はその定数の doc。
    ///
    /// # 拒否の3種を**別の理由**で返す
    ///
    /// | 状況 | 返す理由 | なぜ分けるのか |
    /// |---|---|---|
    /// | 古い行が無い | `UnknownMemory` | 呼び出し側が id を間違えている |
    /// | 別キャラクターの行 | `MemoryBelongsToAnotherCharacter` | 宛先違い。**FK に任せると「引き当てが無い」と区別が付かない** |
    /// | すでに上書き済み | `MemoryAlreadySuperseded` | 鎖を線形に保つ。**いま有効な方を指せば通る** |
    ///
    /// # **同時に成立したら、宛先違いが勝つ**（上から順に見る）
    ///
    /// 別キャラクターの・すでに上書き済みの行を指した場合に
    /// `MemoryAlreadySuperseded` を返すと、**「その記憶は存在し、訂正済みである」という
    /// 他キャラクターの状態の開示**になる。宛先違いは中身を一切明かさない。
    /// 運用者の調査先も変わる（「有効な方を指せばよい」と読んで他人の鎖を辿りに行く）。
    /// 持ち主が違う時点で、鎖の状態はこの呼び出しには関係ない。
    ///
    /// 見張り番は
    /// `tests/store_memory.rs::a_superseded_memory_of_another_character_is_still_rejected_as_wrong_owner`
    /// （**2つの条件が同時に成立する行**を作る。1つずつしか成立しないフィクスチャでは、
    /// 宛先違いを `&& successor.is_none()` で条件化する変異が緑を通る / 14a のレビューで実測）。
    ///
    /// 複合外部キー
    /// （`FOREIGN KEY (superseded_by, character_id) REFERENCES memory(id, character_id)`）が
    /// 別キャラクターの行を構造的に潰していても、**ここで先に弾く。**
    /// 任せると `StoreError::Query`（"データベース操作に失敗しました"）になり、
    /// 呼び出し側は原因を区別できない。しかも FK は接続ごとの `PRAGMA foreign_keys` に
    /// 依存する（`UnknownSession` が外部キー違反を名乗らないのと同じ理由）。
    ///
    /// # **`locked` は読みも書きもしない**（ADR-0022 §4）
    ///
    /// 意味がまだ確定していないので、訂正の可否の判断にも使わないし、
    /// 訂正版にも書かない（DB の既定値 0 のまま入る）。
    ///
    /// # 自動化はしない（ADR-0022 §5）
    ///
    /// これは**人が叩く導線**。抽出が自分で「これは前の記憶の訂正だ」と判断して
    /// 呼ぶ道は別の判断（フェーズ2）で、**誤って上書きすると正しい記憶が読めなくなる。**
    pub async fn supersede_memory(
        &self,
        character_id: CharacterId,
        superseded: MemoryId,
        correction: &NewMemory,
    ) -> Result<MemoryId, StoreError> {
        let mut tx = self
            .pool
            .begin_with(BEGIN_IMMEDIATE)
            .await
            .map_err(StoreError::Query)?;

        // **検査はトランザクションの中で行う。** 外に出すと、その間に別の訂正が
        // commit して「有効だと確かめた行」が上書き済みになり、鎖が分岐する。
        let target: Option<(i64, Option<i64>)> = sqlx::query_as(SELECT_SUPERSEDE_TARGET)
            .bind(superseded.get())
            .fetch_optional(&mut *tx)
            .await
            .map_err(StoreError::Query)?;

        let Some((owner, successor)) = target else {
            // **id はログにだけ出す**（`Display` には載せない / 他の拒否と同じ）。
            tracing::warn!(
                memory_id = superseded.get(),
                character_id = character_id.get(),
                "存在しない記憶を訂正しようとしました"
            );
            return Err(StoreError::UnknownMemory);
        };

        if owner != character_id.get() {
            tracing::warn!(
                memory_id = superseded.get(),
                character_id = character_id.get(),
                "別のキャラクターの記憶を訂正しようとしました"
            );
            return Err(StoreError::MemoryBelongsToAnotherCharacter);
        }

        if let Some(successor) = successor {
            tracing::warn!(
                memory_id = superseded.get(),
                successor,
                "すでに上書きされた記憶を訂正しようとしました。いま有効な記憶を指定してください"
            );
            return Err(StoreError::MemoryAlreadySuperseded);
        }

        // **追加の文は `add_memory` と同じもの**（`INSERT_MEMORY`）。
        // 訂正版は「普通の記憶」であって、特別な列を持たない
        // ——`superseded_by` も `locked` も書かない。写しを作ると、
        // 片方の列構成だけが黙って変わる。
        let inserted = sqlx::query(INSERT_MEMORY)
            .bind(character_id.get())
            .bind(subject_to_db(correction.subject))
            .bind(correction.content.as_str())
            .bind(correction.category.as_ref().map(MemoryCategory::as_str))
            .execute(&mut *tx)
            .await
            .map_err(StoreError::Query)?;
        let correction_id = inserted.last_insert_rowid();

        // **更新件数は確かめない。** 同じトランザクションの中で行の存在を
        // 確かめたうえ、`BEGIN IMMEDIATE` で書き込みロックを取っているので、
        // ここが 0 件になる入力が存在しない（到達不能な検査を置かない）。
        sqlx::query(UPDATE_SUPERSEDED_BY)
            .bind(correction_id)
            .bind(superseded.get())
            .execute(&mut *tx)
            .await
            .map_err(StoreError::Query)?;

        tx.commit().await.map_err(StoreError::Query)?;

        tracing::info!(
            character_id = character_id.get(),
            superseded = superseded.get(),
            correction = correction_id,
            "記憶を訂正版で置き換えました"
        );
        Ok(MemoryId::from_row(correction_id))
    }

    /// **修復のための一覧。** 有効な記憶を、読めた行と読めなかった行に分けて返す。
    ///
    /// # **これは想起ではない**（ADR-0022 §2 / ADR-0003 の「入口はひとつ」に反しない）
    ///
    /// 返り値は**プロンプトに載らない。** `prompt` からこの関数を呼ばないこと。
    /// 呼びたくなったら、それは想起の入口が2本になろうとしている合図である。
    ///
    /// # なぜ `recall_memories` で代用できないのか
    ///
    /// あちらは**1行でも壊れていれば丸ごと `InvalidStoredValue`** で止まる。
    /// それは想起としては正しい（静かに減らすとキャラクターの振る舞いだけが
    /// 不可視に変わる）が、**修復には使えない**——直したい行がある状態では読めない。
    ///
    /// # **壊れた行を黙って落とさない**
    ///
    /// 行ごとに `MemoryEntry::Readable` / `Unreadable` を返す。
    /// **これは型が強制するものではない**（`filter_map` で `Unreadable` を捨てる実装は
    /// そのままコンパイルが通る。0012b で実測）。守っているのは
    /// `tests/store_memory.rs::a_broken_memory_row_does_not_take_the_healthy_ones_with_it` /
    /// `every_kind_of_broken_memory_column_is_pointed_at` /
    /// `the_repair_listing_stays_readable_where_recall_stops` の3本。
    ///
    /// # **`locked` は「読めるかだけ」見る**（ADR-0022 §4）
    ///
    /// 0 / 1 以外なら**その行を読めなかった行にする**が、
    /// **絞り込みには一切使わない**（`SELECT_ACTIVE_MEMORY_ROWS` に `locked` の条件は無い）。
    /// `locked = 1` の行は「保護印の付いた健全な行」であって、修復の対象から外す行ではない
    /// ——外すと**保護したい記憶ほど修復の入口から消える。**
    ///
    /// 見張り番は
    /// `tests/store_memory.rs::a_locked_memory_is_listed_for_repair_and_still_reads_as_locked`
    /// （`locked = 1` の健全な行を作る唯一のテスト。この状態が無いと
    /// `1 => Some(true)` を `Some(false)` にしても `None` にしても、
    /// `WHERE` に `AND locked <> 1` を足しても緑だった / 14a のレビューで実測）。
    ///
    /// # 返すのは**有効な行だけ**
    ///
    /// 上書き済みの行はもう想起に載らないので害が無い。理由は
    /// `SELECT_ACTIVE_MEMORY_ROWS` の doc。
    ///
    /// # 行ごとに救えるのは `parse` 由来の壊れ方だけ
    ///
    /// `try_get` のデコード失敗（`created_at` が日時として読めない等）は
    /// **今でも一覧を丸ごと落とす**（`StoreError::Query`）。
    /// `list_characters` と同じ構造的な限界で、`docs/06-roadmap.md` に積んである。
    pub async fn list_memories_for_repair(
        &self,
        character_id: CharacterId,
    ) -> Result<Vec<MemoryEntry>, StoreError> {
        let rows = sqlx::query(SELECT_ACTIVE_MEMORY_ROWS)
            .bind(character_id.get())
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let mut entries = Vec::with_capacity(rows.len());
        for row in &rows {
            entries.push(memory_entry(row, character_id)?);
        }

        tracing::debug!(
            character_id = character_id.get(),
            entries = entries.len(),
            "修復のための記憶一覧を読み出しました"
        );
        Ok(entries)
    }

    /// **上書きされた記憶を、新しい順に最大 `limit` 件**返す（ADR-0022 / 作業単位 D-3b）。
    ///
    /// # なぜこの読みが要るのか
    ///
    /// ADR-0022 の要は「**消さずに時系列で残す**」ことなのに、
    /// **読み出す入口がどれも `superseded_by IS NULL` しか見ていなかった**
    /// （想起も、修復の一覧も）。**外から見ると「消さずに残す」は書き込み専用**で、
    /// 「以前はXだったが今はY」の X の側が API のどこにも現れない。
    ///
    /// # **想起ではない**（ADR-0003 の「入口はひとつ」に反しない）
    ///
    /// 返り値は**プロンプトに載らない。** `prompt` からこの関数を呼ばないこと。
    /// ここが返すのは**想起の条件から外れた行**——つまり
    /// **想起に載ることが二度と無い行**である。
    ///
    /// # **修復の一覧でもない**
    ///
    /// [`Store::list_memories_for_repair`] は「いま直せる行」を返す。
    /// **上書き済みの行はもう直せない**（`supersede_memory` は
    /// `MemoryAlreadySuperseded` で拒む）。ここが返すのは**読むためだけの記録**である。
    ///
    /// # 上限を持つ理由
    ///
    /// **行は消えないので単調増加する**（ADR-0022 §結果）。
    /// 上限が無ければ、応答は訂正の回数に比例して伸び続ける。
    /// **既定値を決めるのは呼び出し側**（`src/server/admin.rs` の
    /// `RECENT_SUPERSEDED_MEMORIES`）——`store` は「いくつまで」を決めない。
    ///
    /// # **1行でも `try_get` に失敗すれば、この読みは丸ごと止まる**
    ///
    /// [`Store::list_memories_for_repair`] と同じ構造的な限界
    /// （`created_at` が日時として読めない等）。
    /// **`parse` 由来の壊れ方は行ごとに救う**——`MemoryEntry::Unreadable` として並べる。
    /// 読み飛ばすと**壊れた環だけが鎖から落ち、総数にだけ残る。**
    pub async fn list_superseded_memories(
        &self,
        character_id: CharacterId,
        limit: u32,
    ) -> Result<Vec<SupersededMemory>, StoreError> {
        let rows = sqlx::query(SELECT_SUPERSEDED_MEMORY_ROWS)
            .bind(character_id.get())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let mut listed = Vec::with_capacity(rows.len());
        for row in &rows {
            listed.push(superseded_memory(row, character_id)?);
        }

        tracing::debug!(
            character_id = character_id.get(),
            memories = listed.len(),
            "上書きされた記憶を読み出しました"
        );
        Ok(listed)
    }

    /// **そのキャラクターの上書きされた記憶の総数**（作業単位 D-3b）。**上限で切らない。**
    ///
    /// 数えるだけなので、主語も本文も分類も読まない
    /// ——**壊れた行があっても総数は返る**（一覧の側がその行を指し示す）。
    pub async fn count_superseded_memories(
        &self,
        character_id: CharacterId,
    ) -> Result<u32, StoreError> {
        let counted: i64 = sqlx::query_scalar(COUNT_SUPERSEDED_MEMORIES)
            .bind(character_id.get())
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        // **`try_get::<u32>` にしない**（`count_drift_votes` と同じ理由）。
        // `u32` に収まらない状態はこの表では起こりようがないが、
        // **収まらなかったときに panic しない形**にしておく。
        u32::try_from(counted).map_err(|_| StoreError::InvalidStoredValue)
    }
}

/// 1行を `MemoryEntry` に組み立てる。`character_id` は WHERE で固定済みなので引数を使う。
///
/// # `memory_from_row`（想起側）との非対称は意図的
///
/// あちらは1列でも `parse` を通らなければ **`InvalidStoredValue` で想起全体を止める。**
/// こちらは**その行を「読めなかった行」として並べて返す。**
/// 失敗したときに起きることが違うため方針を割ってある
/// （`memory_from_row` の doc に想起側の理由がある）:
///
/// * **想起**は、落とすとキャラクターの振る舞いだけが「なんとなく前と違う」形で変わる。
/// * **修復の一覧**は、落とすと**直す相手が分からなくなる。** 1件のために一覧全体を
///   止めると、健全な記憶まで見えなくなり、修復の入口そのものが消える。
///
/// この非対称を「片方の実装漏れ」と読んで揃えないこと。
///
/// **`Err` を返すのは `try_get` が失敗したときだけ**（`parse` の失敗では返さない）。
fn memory_entry(row: &SqliteRow, character_id: CharacterId) -> Result<MemoryEntry, StoreError> {
    let id = MemoryId::from_row(row.try_get::<i64, _>("id").map_err(StoreError::Query)?);
    let mut invalid_columns = Vec::new();

    let stored_subject: String = row.try_get("subject").map_err(StoreError::Query)?;
    let subject = subject_from_db(&stored_subject);
    if subject.is_none() {
        // **格納値はログにも出さない**（何が入っているか分からない）。手掛かりは id。
        invalid_columns.push(MemoryColumn::Subject);
    }

    let stored_content: String = row.try_get("content").map_err(StoreError::Query)?;
    let content = MemoryContent::parse(&stored_content).ok();
    if content.is_none() {
        invalid_columns.push(MemoryColumn::Content);
    }

    // **NULL は「未設定」であって不正ではない**（`Some(None)` = 読めた・値は無し）。
    let stored_category: Option<String> = row.try_get("category").map_err(StoreError::Query)?;
    let category = match &stored_category {
        None => Some(None),
        Some(stored) => MemoryCategory::parse(stored).ok().map(Some),
    };
    if category.is_none() {
        invalid_columns.push(MemoryColumn::Category);
    }

    // **`try_get::<bool>` にしない**（`memory_from_row` と同じ理由）。CHECK は 0 / 1 に
    // 縛るが `PRAGMA ignore_check_constraints` で外せるので `7` も入りうる。
    // sqlx の bool デコードは 0 以外を true にするため、範囲外を通すと
    // 「保護対象の印」が静かに増える。
    //
    // **報告は他の列の壊れ方と無関係に行う。** 「他が読めたときだけ `locked` を報告する」
    // 形にすると、**`locked` と他列が同時に壊れた行が `locked` を報告しない**
    // ——1つ直しても読めない行が残る（見張り番は
    // `every_kind_of_broken_memory_column_is_pointed_at` の
    // `category_and_locked_broken`）。
    let stored_locked: i64 = row.try_get("locked").map_err(StoreError::Query)?;
    let locked = match stored_locked {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    };
    if locked.is_none() {
        invalid_columns.push(MemoryColumn::Locked);
    }

    let created_at = row.try_get("created_at").map_err(StoreError::Query)?;

    match (subject, content, category, locked) {
        (Some(subject), Some(content), Some(category), Some(locked)) => {
            Ok(MemoryEntry::Readable(Memory {
                id,
                character_id,
                subject,
                content,
                category,
                created_at,
                locked,
            }))
        }
        _ => {
            // **格納値は出さない。列名だけを出す**（定数なので安全）。
            tracing::warn!(
                memory_id = id.get(),
                character_id = character_id.get(),
                invalid_columns = ?invalid_columns,
                "読み出せない記憶の行があります。一覧が返す id で訂正できます"
            );
            Ok(MemoryEntry::Unreadable(UnreadableMemory {
                id,
                character_id,
                invalid_columns,
            }))
        }
    }
}

/// 1行を `SupersededMemory` に組み立てる（作業単位 D-3b）。
///
/// # 行の組み立ては [`memory_entry`] に任せる（写さない）
///
/// 主語・本文・分類・`locked` の検証も、壊れた行の指し示し方も、
/// **修復の一覧とまったく同じもの**を通す。写しを作ると、
/// **片方の列の読み方だけが黙って変わる**（`locked` の範囲検査が片側から消える等）。
/// ここが足すのは `superseded_by` の1列だけ。
///
/// # `Option<i64>` で読まない
///
/// 引く文が `superseded_by IS NOT NULL` で絞っているので、**NULL は届かない。**
/// `Option` にすると、呼び出し側に**「後継の分からない上書き済みの行」という
/// 存在しない状態**を扱わせることになる。
///
/// > **⚠ 型は守らない。ここは測って確かめた**（D-3b の変異検査 / 2026-08-27）。
/// > 当初この doc は「絞り込みが消えたら型のデコードで落ちる」と書いていたが、**嘘だった。**
/// > **sqlx の SQLite は NULL を `i64` に `0` としてデコードする**——エラーにならない。
/// > `WHERE` から `AND superseded_by IS NOT NULL` を外す変異は、
/// > **有効な行を「後継が 0 の上書き済みの行」として静かに並べた**（実測: `[(3, 0), (2, 3), (1, 0)]`）。
/// >
/// > **守っているのは `WHERE` 句と見張り番だけ**である（同じ変異で 13 本が赤 / うち
/// > `tests/store_memory.rs::a_character_without_superseded_memories_lists_nothing_and_counts_zero`
/// > は**有効な行しか無いキャラクター**でそれを捕まえる）。**「型で表現できなくした」と読まないこと。**
fn superseded_memory(
    row: &SqliteRow,
    character_id: CharacterId,
) -> Result<SupersededMemory, StoreError> {
    let entry = memory_entry(row, character_id)?;
    let superseded_by: i64 = row.try_get("superseded_by").map_err(StoreError::Query)?;

    Ok(SupersededMemory {
        entry,
        superseded_by: MemoryId::from_row(superseded_by),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一覧は**有効な行だけ**を引く。
    ///
    /// 実挙動は `tests/store_memory.rs::the_repair_listing_leaves_out_superseded_rows` が
    /// 固定しているが、条件そのものが SQL 文から消えていないこともここで見ておく
    /// （想起側の `recall_query_filters_superseded_rows` と同じ形）。
    #[test]
    fn the_repair_listing_query_filters_superseded_rows() {
        assert!(
            SELECT_ACTIVE_MEMORY_ROWS.contains("superseded_by IS NULL"),
            "有効な記憶だけを引くこと: {SELECT_ACTIVE_MEMORY_ROWS}"
        );
    }

    /// **2つの一覧の絞り込みが、逆のまま別々の文に載っている**（作業単位 D-3b）。
    ///
    /// # なぜ速報がここに要るのか
    ///
    /// **実挙動の権威は `tests/store_memory.rs` と `tests/server_admin_memories.rs`** で、
    /// ここは「**1つの文を問い合わせで組み替える形へ寄せていない**」ことだけを見る。
    /// 寄せると、修復の一覧の絞り込みとこの読みの絞り込みが同じ文の中で綱引きになり、
    /// **片方を直すともう片方が黙って変わる**（さらに `WHERE` を実行時に組む誘惑が生まれる。
    /// 絶対制約: SQL は文字列連結で組まない）。
    ///
    /// **`ORDER BY` の向きはここで見ない。** 見ると、向きを反転する変異に対して
    /// **文面を写したこの検査だけが赤くなり、振る舞いを見るテストが無くても
    /// 守れているように読める**（CLAUDE.md「赤くなったのが速報だけのことがある」）。
    #[test]
    fn the_two_listings_ask_for_opposite_halves_of_the_table() {
        assert!(
            SELECT_SUPERSEDED_MEMORY_ROWS.contains("superseded_by IS NOT NULL"),
            "上書き済みの行だけを引くこと: {SELECT_SUPERSEDED_MEMORY_ROWS}"
        );
        assert!(
            COUNT_SUPERSEDED_MEMORIES.contains("superseded_by IS NOT NULL"),
            "総数も上書き済みの行だけを数えること: {COUNT_SUPERSEDED_MEMORIES}"
        );
        assert!(
            !SELECT_ACTIVE_MEMORY_ROWS.contains("superseded_by IS NOT NULL"),
            "修復の一覧に上書き済みの行を混ぜないこと: {SELECT_ACTIVE_MEMORY_ROWS}"
        );
        assert_ne!(
            SELECT_ACTIVE_MEMORY_ROWS, SELECT_SUPERSEDED_MEMORY_ROWS,
            "1つの文を組み替える形へ寄せないこと"
        );

        // **後継の列を読んでいること。** 絞るだけでは鎖が辿れない
        // （「時系列」ではなく「消えた記憶の山」になる）。
        let selected = SELECT_SUPERSEDED_MEMORY_ROWS
            .split_once(" FROM ")
            .map(|(columns, _)| columns)
            .unwrap_or_default();
        assert!(
            selected.contains("superseded_by"),
            "後継の id を選択していること: {selected}"
        );

        // **上限を持つこと。** 行は消えないので単調増加する。
        assert!(
            SELECT_SUPERSEDED_MEMORY_ROWS.contains("LIMIT ?"),
            "上限を持つこと: {SELECT_SUPERSEDED_MEMORY_ROWS}"
        );
        assert!(
            !COUNT_SUPERSEDED_MEMORIES.contains("LIMIT"),
            "総数は上限で切らないこと: {COUNT_SUPERSEDED_MEMORIES}"
        );
    }

    /// **引き当ての文が、直したい行の値を読まない。**
    ///
    /// 修復の対象は壊れた行なので、`content` や `category` を読む形にすると
    /// **直したい行ほど届かない**（ADR-0016 §4 と同じ罠）。
    /// 読んでよいのは拒否の理由を決める2列だけ。
    #[test]
    fn the_target_lookup_reads_nothing_that_needs_parsing() {
        for column in ["content", "category", "subject", "locked"] {
            assert!(
                !SELECT_SUPERSEDE_TARGET.contains(column),
                "{column} を読むと壊れた行に届かなくなる: {SELECT_SUPERSEDE_TARGET}"
            );
        }
        assert!(SELECT_SUPERSEDE_TARGET.contains("character_id"));
        assert!(SELECT_SUPERSEDE_TARGET.contains("superseded_by"));
    }

    /// **後継の印を消す文が、どこにも書かれていない**（ADR-0022 §1）。
    ///
    /// # なぜ既存の走査では足りないのか
    ///
    /// `no_statement_updates_or_deletes_a_memory` は
    /// 「`superseded_by` の単独更新」を**正当な形として通す。**
    /// つまり**「繋ぐ」と「切る」を区別できない**（`docs/06-roadmap.md` の既知の穴）。
    /// 印を消す文は許可形にぴったり収まるので、あの走査は素通りする。
    ///
    /// **失敗シナリオはロードマップが名指ししている**——将来の整理マイグレーションが
    /// 「もう使わないリンクを掃除する」つもりで印を消すと、
    /// **上書きしたはずの古い記憶が現在の事実として想起へ戻る。**
    /// ADR-0022 §1 の「後継の無い取り消しは作らない」の裏側でもある
    /// （取り消しは「印を付けない」だけでなく「印を外す」形でも作れてしまう）。
    ///
    /// # 判定
    ///
    /// 走査対象は `src/**` の Rust と `migrations/**` の SQL。
    /// **コメントも含む生のソースを見る**（判定を軽くするため。
    /// 禁止したい文をコメントで引用したくなったら、この doc のように語を分けて書くこと）。
    ///
    /// **判定そのものは `clears_a_successor` にある。** ここで組み直さないのは、
    /// 「植え込んだ文を実際に落とせる」と主張する
    /// `the_successor_scan_actually_reads_the_tree` が**同じ関数を叩くため**
    /// ——判定を書き直せば、あちらが赤くなる。
    /// 写しにしていた頃は、判定を `NULLIF` に変えても**両方とも緑**だった
    /// （14a のレビューで実測。CLAUDE.md「呼び出し側が全部その関数で期待値を作る」）。
    ///
    /// # **この走査が守れない形**
    ///
    /// `UPDATE memory SET superseded_by = ? WHERE id = ?` の
    /// **プレースホルダに `None` を束ねる**書き方は、**どんなテキスト走査でも見えない**
    /// ——SQL 文が正当な上書きと同一だからである。
    /// 詳しくは `memory.rs::no_statement_updates_or_deletes_a_memory` の doc。
    #[test]
    fn no_statement_clears_a_successor() {
        let found: Vec<String> = scanned_sources()
            .into_iter()
            .filter(|(_, source)| clears_a_successor(source))
            .map(|(path, _)| path)
            .collect();

        assert!(
            found.is_empty(),
            "後継の印を消す文があります（上書きしたはずの記憶が想起へ戻ります）: {found:?}"
        );
    }

    /// **走査が実ファイルを読めている。かつ、判定が実際に落とせる。**
    ///
    /// `no_statement_clears_a_successor` は「見つからないこと」を主張するので、
    /// **1ファイルも読めていなくても緑になる。** 対象が実在することと、
    /// 植え込んだ文を**あの走査が使う判定そのもので**落とせることを別に確かめる。
    #[test]
    fn the_successor_scan_actually_reads_the_tree() {
        let sources = scanned_sources();
        assert!(
            sources.len() >= 15,
            "走査対象が少なすぎるなら、あの assert は何も守っていない: {:?}",
            sources
                .iter()
                .map(|(path, _)| path)
                .collect::<Vec<&String>>()
        );
        assert!(
            sources.iter().any(|(path, _)| path.ends_with(".sql")),
            "マイグレーションを外さないこと（整理 SQL が最もありそうな置き場所）"
        );

        // 植え込んだ文が落ちること。空白の詰め方・大小・改行をまたぐ形も同じ扱い。
        // **テーブル名もソース上に綴らない**（`memory.rs` の `table_word` と同じ理由。
        // 綴ると、記憶を潰す文の走査に自分自身が引っ掛かる）。
        let table = ["mem", "ory"].concat();
        let column = ["superseded", "_by"].concat();
        for planted in [
            format!("UPDATE {table} SET {column} = NULL WHERE id = ?"),
            format!("update {table} set {column}=null;"),
            format!("UPDATE {table}\n     SET {column} = NULL\n     WHERE character_id = ?"),
        ] {
            assert!(
                clears_a_successor(&planted),
                "この形を落とせていません: {planted}"
            );
        }

        // **落としてはいけない形。** 判定を広げる変異（列名を見ずに NULL 代入だけを
        // 探す等）は、ここが赤になる。
        for allowed in [
            format!("UPDATE {table} SET {column} = ? WHERE id = ?"),
            format!("SELECT id FROM {table} WHERE {column} IS NULL"),
            format!("UPDATE {table} SET content = NULL WHERE id = ?"),
        ] {
            assert!(
                !clears_a_successor(&allowed),
                "正当な文を落としています: {allowed}"
            );
        }
    }

    /// **後継の印を消す文がソースに綴られているか**（`no_statement_clears_a_successor` の判定）。
    ///
    /// 空白を全部落として大文字に揃えてから、`superseded_by` への NULL 代入を探す。
    /// 空白の詰め方・大小・改行をまたぐ形を同じ扱いにするため。
    ///
    /// **判定はこの1箇所だけ。** 呼び出し側で組み直さないこと
    /// （組み直すと、判定を緩める変異が両側とも緑を通る）。
    fn clears_a_successor(source: &str) -> bool {
        // **この語をソース上にそのまま綴らない**（自分自身が走査対象なので必ず引っ掛かる）。
        let column = ["superseded", "_by"].concat();
        let cleared = format!("{column}=NULL").to_ascii_uppercase();

        let squeezed: String = source
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>()
            .to_ascii_uppercase();
        squeezed.contains(&cleared)
    }

    /// 走査対象を「パス, 生ソース」で返す（`src/**` の Rust と `migrations/**` の SQL）。
    fn scanned_sources() -> Vec<(String, String)> {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();

        for (root, extension) in [("src", "rs"), ("migrations", "sql")] {
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
        }
        files
    }
}
