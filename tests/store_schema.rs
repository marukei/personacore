//! `store` モジュールの接続・マイグレーション・スキーマ制約の検証。
//!
//! ここで確かめるのは主に「正しいデータが通ること」ではなく、**「不正なデータを弾くこと」**。
//! CLAUDE.md の絶対制約（`memory.subject` / `message.raw_content` /
//! `generated_image` の再現情報 / キャラクター識別子の `@` 禁止）を、アプリ層だけでなく
//! スキーマ側でも強制できているかを確認する。
//! 併せて、**別キャラ・別セッションの記憶の混線**を複合外部キーで防げているか、
//! 接続の `journal_mode` が WAL であるかも見る。
//!
//! 例外的に「通ること」を固定しているのは次の2種類:
//!   * `message.dialogue` / `narration` の NULL — パース失敗時のフォールバックの受け皿。
//!     ここに NOT NULL が付くと、最も落ちてほしくない経路だけが実行時に落ちる。
//!   * `ON DELETE CASCADE` — 落としても正常系では気づけないため、明示的に固定する。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use personacore::domain::{
    AgreementContent, AgreementKeeper, AgreementKind, AgreementState, AttributeKey, AttributeSet,
    AttributeValue, AxisDiscoverySlots, CharacterAttribute, CharacterColumn, CharacterEntry,
    CharacterId, DisplayName, DisplayNameError, DriftModeSource, Handle, HandleError, LINE_BREAKS,
    MemoryColumn, ResolvedDriftMode, StatAxis, StatDriftMode, Subject, TRIMMABLE,
};
use personacore::store::{Store, StoreError};
use sqlx::Row;
use sqlx::error::ErrorKind;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tempfile::TempDir;

mod common;

use common::insert_character_ignoring_checks;

/// 一時ディレクトリ上に新規 DB を作り、マイグレーション適用済みの `Store` を返す。
///
/// `TempDir` を落とすとファイルごと消えるため、呼び出し側で保持する。
async fn setup() -> (TempDir, Store) {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let store = Store::connect(dir.path().join("personacore.sqlite"))
        .await
        .expect("接続とマイグレーション適用が成功すること");
    (dir, store)
}

/// DB エラーの種別を検証する。単に「失敗した」ではなく「意図した制約で落ちた」ことを見る。
fn assert_kind(err: &sqlx::Error, expected: ErrorKind, context: &str) {
    let kind = err.as_database_error().map(|db| db.kind());
    assert_eq!(kind, Some(expected), "{context}: 実際のエラー = {err}");
}

async fn insert_character(store: &Store, handle: &str) -> i64 {
    sqlx::query("INSERT INTO character (handle, base_persona) VALUES (?, ?)")
        .bind(handle)
        .bind("ペルソナ本文")
        .execute(store.pool())
        .await
        .expect("正当なキャラクターは挿入できること")
        .last_insert_rowid()
}

async fn insert_session(store: &Store, character_id: i64) -> i64 {
    sqlx::query("INSERT INTO session (character_id, label) VALUES (?, ?)")
        .bind(character_id)
        .bind("テストセッション")
        .execute(store.pool())
        .await
        .expect("正当なセッションは挿入できること")
        .last_insert_rowid()
}

async fn insert_message(store: &Store, session_id: i64) -> i64 {
    sqlx::query("INSERT INTO message (session_id, role, raw_content) VALUES (?, 'assistant', ?)")
        .bind(session_id)
        .bind("生出力")
        .execute(store.pool())
        .await
        .expect("正当なメッセージは挿入できること")
        .last_insert_rowid()
}

/// 生 SQL でのセッション削除。
///
/// **`store` にセッションを削除する API は無い**
/// （0012a で入ったのは `delete_character` だけ。セッションの側は今も入口が無い）。
/// id の再利用を確かめるには最大 id の行を実際に消す必要があるため、
/// `testing` フィーチャの抜け穴（`Store::pool`）から直接消す。
async fn delete_session(store: &Store, id: i64) {
    let done = sqlx::query("DELETE FROM session WHERE id = ?")
        .bind(id)
        .execute(store.pool())
        .await
        .expect("セッションを削除できること");
    assert_eq!(done.rows_affected(), 1, "削除対象がちょうど1件であること");
}

/// 生 SQL でのメッセージ削除。理由は `delete_session` と同じ。
async fn delete_message(store: &Store, id: i64) {
    let done = sqlx::query("DELETE FROM message WHERE id = ?")
        .bind(id)
        .execute(store.pool())
        .await
        .expect("メッセージを削除できること");
    assert_eq!(done.rows_affected(), 1, "削除対象がちょうど1件であること");
}

async fn insert_memory(store: &Store, character_id: i64, content: &str) -> i64 {
    sqlx::query("INSERT INTO memory (character_id, subject, content) VALUES (?, 'user', ?)")
        .bind(character_id)
        .bind(content)
        .execute(store.pool())
        .await
        .expect("正当な記憶は挿入できること")
        .last_insert_rowid()
}

async fn insert_summary(
    store: &Store,
    session_id: i64,
    upto_message_id: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query("INSERT INTO summary (session_id, upto_message_id, content) VALUES (?, ?, ?)")
        .bind(session_id)
        .bind(upto_message_id)
        .bind("要約本文")
        .execute(store.pool())
        .await
        .map(|done| done.last_insert_rowid())
}

/// `generated_image` の INSERT。`message_id` は NULL 可（メッセージに紐付かない生成）。
async fn insert_generated_image(
    store: &Store,
    session_id: i64,
    message_id: Option<i64>,
) -> Result<i64, sqlx::Error> {
    sqlx::query(
        "INSERT INTO generated_image \
         (session_id, message_id, positive_prompt, negative_prompt, seed, checkpoint, file_path) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(session_id)
    .bind(message_id)
    .bind("1girl, smile")
    .bind("worst quality")
    .bind(42_i64)
    .bind("some-checkpoint.safetensors")
    .bind("images/0001.png")
    .execute(store.pool())
    .await
    .map(|done| done.last_insert_rowid())
}

/// 「空白のみ」とみなすべき文字列の一覧。
///
/// SQLite の `trim(X)` は**半角スペースしか除かない**ため、`length(trim(X)) > 0` では
/// TAB / 改行 / 全角空白 / NBSP のみの文字列が素通りする。
/// 日本語環境では **U+3000（全角空白）** が現実に混入しうるので、除去対象に含める。
const BLANK_STRINGS: [&str; 11] = [
    "",
    " ",
    "   ",
    "\t",              // U+0009 TAB
    "\n",              // U+000A LF
    "\u{000B}",        // U+000B VT
    "\u{000C}",        // U+000C FF
    "\r",              // U+000D CR
    "\u{3000}",        // 全角空白
    "\u{00A0}",        // NBSP
    " \u{3000}\t\r\n", // 混在
];

/// テーブルごとの行数カウント。テーブル名はバインドできないため、
/// 文字列連結でクエリを組まず**リテラルの SQL を列挙する**。
const COUNT_QUERIES: [(&str, &str); 17] = [
    ("character", "SELECT COUNT(*) FROM character"),
    (
        "character_agreement",
        "SELECT COUNT(*) FROM character_agreement",
    ),
    (
        "agreement_transition",
        "SELECT COUNT(*) FROM agreement_transition",
    ),
    ("stat_alloc", "SELECT COUNT(*) FROM stat_alloc"),
    ("stat_drift_vote", "SELECT COUNT(*) FROM stat_drift_vote"),
    (
        "stat_drift_charge",
        "SELECT COUNT(*) FROM stat_drift_charge",
    ),
    (
        "character_attribute",
        "SELECT COUNT(*) FROM character_attribute",
    ),
    (
        "attribute_revision",
        "SELECT COUNT(*) FROM attribute_revision",
    ),
    (
        "stat_axis_candidate",
        "SELECT COUNT(*) FROM stat_axis_candidate",
    ),
    (
        "stat_axis_candidate_transition",
        "SELECT COUNT(*) FROM stat_axis_candidate_transition",
    ),
    (
        "stat_axis_trial_vote",
        "SELECT COUNT(*) FROM stat_axis_trial_vote",
    ),
    (
        "character_appearance",
        "SELECT COUNT(*) FROM character_appearance",
    ),
    ("session", "SELECT COUNT(*) FROM session"),
    ("message", "SELECT COUNT(*) FROM message"),
    ("memory", "SELECT COUNT(*) FROM memory"),
    ("summary", "SELECT COUNT(*) FROM summary"),
    ("generated_image", "SELECT COUNT(*) FROM generated_image"),
];

/// 索引の宣言（`(索引名, 対象テーブル, &[(列名, DESC なら 1)])`）。
type ExpectedIndex = (&'static str, &'static str, &'static [(&'static str, i64)]);

/// 索引の実測（`(索引名, 対象テーブル, [(列名, DESC なら 1)])`）。
type IndexShape = (String, String, Vec<(String, i64)>);

/// 明示的に作った索引の一覧（`(索引名, 対象テーブル, &[(列名, DESC なら 1)])`。名前順）。
///
/// **テーブル再構築で最も静かに落ちるのが索引。** 落ちても正常系のテストは緑のままで、
/// 変わるのは走査計画だけなので、実運用でも「なんとなく遅い」としか観測できない。
/// `sqlite_autoindex_*`（UNIQUE / PRIMARY KEY の自動生成）はここに載せない。
/// そちらは複合外部キーのテストが実挙動で見ている。
///
/// **名前だけでは足りない。** 列構成・列順・`DESC` を書き換えても索引名は変わらないので、
/// `session_character_recent_idx` を `(updated_at, character_id)` に作り直しても
/// 名前だけの照合は緑のまま通る。この索引は ADR-0004 の
/// 「`aoi` = 最新セッションを継続」を引く本体なので、列順と `DESC` まで固定する。
///
/// 列名 `<expr>` は式索引の印（`pragma_index_xinfo` は式の列名を NULL で返す）。
/// **いまは式索引が1つも無い**が、印そのものは残す
/// （式索引を足したときに `None` を握り潰さないため）。
///
/// > **`character_name_key_idx`（正規化キーの UNIQUE 索引 / 0002）はここにあった。**
/// > 0004b-2 で `migrations/0006_drop_character_name_key_idx.sql` が落としている。
/// > **`character.handle` の一意性そのものは残っている**（列の `UNIQUE` =
/// > `sqlite_autoindex_character_1`。この表には載せない自動生成の索引で、
/// > `character_handle_must_be_unique` が実挙動で見ている）。
const EXPECTED_INDEXES: [ExpectedIndex; 12] = [
    // **取り決めの履歴は取り決めごとに追記の順で辿る**（ADR-0051 / 作業単位 A-12a）。
    (
        "agreement_transition_agreement_idx",
        "agreement_transition",
        &[("agreement_id", 0), ("id", 0)],
    ),
    // **属性の履歴は追記のみで消えない**ので、主キー（`id`）の自動索引だけだと
    // 「新しい順に N 件」の読みが全表走査になる（ADR-0028 §決定5 / 作業単位 A-3a）。
    // 票の索引と同じ形——**キーは列に含めない**（履歴は「新しい順の並び」として読む）。
    (
        "attribute_revision_recent_idx",
        "attribute_revision",
        &[("character_id", 0), ("created_at", 0)],
    ),
    // **有効な取り決めは毎ターン読む**（ADR-0051 §決定4）。キャラクターと状態で引く。
    (
        "character_agreement_character_idx",
        "character_agreement",
        &[("character_id", 0), ("state", 0)],
    ),
    (
        "generated_image_session_idx",
        "generated_image",
        &[("session_id", 0), ("id", 0)],
    ),
    (
        "memory_character_idx",
        "memory",
        &[("character_id", 0), ("created_at", 0)],
    ),
    (
        "message_session_idx",
        "message",
        &[("session_id", 0), ("id", 0)],
    ),
    (
        "session_character_recent_idx",
        "session",
        &[("character_id", 0), ("updated_at", 1)],
    ),
    // **軸の候補の移り変わりは候補ごとに追記の順で読む**（ADR-0046 §決定6 / 作業単位 A-9a）。
    // いまの状態は候補ごとの最後の行なので、`id` まで列に含める。
    (
        "stat_axis_candidate_transition_candidate_idx",
        "stat_axis_candidate_transition",
        &[("candidate_id", 0), ("id", 0)],
    ),
    // **試みの目印はキャラクターごとの最大値**（`max(through_memory_id)` / 作業単位 A-9b）。
    (
        "stat_axis_discovery_attempt_character_idx",
        "stat_axis_discovery_attempt",
        &[("character_id", 0), ("through_memory_id", 0)],
    ),
    // 試用の票は候補ごと・種類ごとに数える（`GROUP BY candidate_id, vote`）。
    (
        "stat_axis_trial_vote_candidate_idx",
        "stat_axis_trial_vote",
        &[("candidate_id", 0), ("vote", 0)],
    ),
    // **票は追記のみで消えない**ので、主キーの自動索引だけだと
    // 「直近 N 件」の読みが毎ターン全表走査になる（ADR-0026 §決定2）。
    // 軸は列に含めない——蓄積は「直近 N 件の並び」として読む。
    (
        "stat_drift_vote_recent_idx",
        "stat_drift_vote",
        &[("character_id", 0), ("created_at", 0)],
    ),
    (
        "summary_session_idx",
        "summary",
        &[("session_id", 0), ("upto_message_id", 0)],
    ),
];

/// 行数を取得する。`sql` はリテラルのみを渡す。
async fn count_rows(store: &Store, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(store.pool())
        .await
        .expect("行数を取得できること")
}

/// マイグレーションが適用でき、18テーブルが過不足なく存在する。
///
/// **`character_agreement` / `agreement_transition` は 0018 で増えた**（ADR-0051 §決定1 / 作業単位 A-12a）。
///
/// **`character_attribute` は 0009 で増えた**（ADR-0009 / 作業単位 12a）。
/// **`stat_drift_vote` は 0010 で増えた**（ADR-0026 §決定3 / 作業単位 16d）。
/// **`stat_drift_charge` は 0011 で増えた**（ADR-0027 §決定7 / 作業単位 16e-b）。
/// **`attribute_revision` は 0013 で増えた**（ADR-0028 §決定5 / 作業単位 A-3a）。
/// **`stat_axis_candidate` / `stat_axis_candidate_transition` / `stat_axis_trial_vote` は 0015 で増えた**
/// （ADR-0046 §決定6 / 作業単位 A-9a）。
/// **`stat_axis_discovery_attempt` は 0016 で増えた**（ADR-0046 §決定2 / 作業単位 A-9b）。
/// docs/03-data-model.md の 8 テーブル + 10 で、`character_fact`（散文）はまだ無い。
#[tokio::test]
async fn migration_creates_exactly_eighteen_tables() {
    let (_dir, store) = setup().await;

    let rows = sqlx::query(
        "SELECT name FROM sqlite_master \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations' \
         ORDER BY name",
    )
    .fetch_all(store.pool())
    .await
    .expect("テーブル一覧を取得できること");

    let names: Vec<String> = rows
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();

    assert_eq!(
        names,
        vec![
            "agreement_transition",
            "attribute_revision",
            "character",
            "character_agreement",
            "character_appearance",
            "character_attribute",
            "generated_image",
            "memory",
            "message",
            "session",
            "stat_alloc",
            "stat_axis_candidate",
            "stat_axis_candidate_transition",
            "stat_axis_discovery_attempt",
            "stat_axis_trial_vote",
            "stat_drift_charge",
            "stat_drift_vote",
            "summary",
        ],
        "docs/03-data-model.md の8テーブル + character_attribute + stat_drift_vote + \
         stat_drift_charge + attribute_revision + 軸の候補の3枚 + 試みの表 + 取り決めの2枚が\
         過不足なく作られること"
    );
}

/// **一覧が指し示す列名は、`character` に実在する列名と一致する**
/// （ADR-0016 §3 / 作業単位 0012b）。
///
/// `CharacterColumn::as_str` は `GET /admin/characters` の `invalid_columns` に
/// そのまま載る。**綴りがずれると、応答を見た運用者が `sqlite3` で
/// `SELECT <その名前> FROM character` を打って `no such column` で止まる**
/// ——手掛かりが「存在しない列」を指すという、最も避けたい壊れ方になる。
/// 実際 `OutputFormatInstruction` の綴りを `"output_format"` に変えても
/// **全テストが緑だった**（0012b のレビューで実測）。
///
/// # 綴りを1つずつ書き並べない
///
/// 期待値を手書きすると `as_str` の写しが増えるだけで、
/// **列名を変えるマイグレーション**（`0004` が `display_name` を足したような変更）には
/// 何も効かない。`PRAGMA table_info` と**集合として**突き合わせれば、
///
/// * variant の綴りを1つ変える → 集合がずれて赤
/// * 列名を変えるマイグレーション → 集合がずれて赤
/// * 列を1つ足す → 赤になり、「この列は `parse` を通すのか」を必ず決めることになる
///
/// のすべてを1本で見張れる。
#[tokio::test]
async fn every_pointed_at_column_is_a_real_character_column() {
    let (_dir, store) = setup().await;

    let rows = sqlx::query("SELECT name FROM pragma_table_info('character')")
        .fetch_all(store.pool())
        .await
        .expect("character の列一覧を取得できること");
    let actual: BTreeSet<String> = rows
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(
        !actual.is_empty(),
        "前提: PRAGMA が列を返すこと（空なら突き合わせが無意味になる）"
    );

    let pointed: BTreeSet<String> = CharacterColumn::ALL
        .iter()
        .map(|column| column.as_str().to_owned())
        .collect();
    assert_eq!(
        pointed.len(),
        CharacterColumn::ALL.len(),
        "2つの variant が同じ列名を指していないこと: {pointed:?}"
    );

    // **`parse` を通さない列。** id は主キー、`created_at` は DB が書く日時で、
    // どちらも壊れていても `CharacterColumn` では指し示せない
    // （`created_at` のデコード失敗は今でも一覧を丸ごと落とす。
    //   理由は `Store::list_characters` の doc）。
    const NOT_PARSED: [&str; 2] = ["id", "created_at"];
    let expected: BTreeSet<String> = pointed
        .iter()
        .cloned()
        .chain(NOT_PARSED.iter().map(|name| (*name).to_owned()))
        .collect();

    assert_eq!(
        actual, expected,
        "`CharacterColumn` の列名と `character` の実在の列が一致すること \
         （応答の invalid_columns が、そのまま sqlite3 で打てる列名であること）"
    );
}

/// **`MemoryColumn` が指す列名が、`memory` に実在する**（ADR-0022 §2 / 作業単位 14a）。
///
/// `CharacterColumn` と同じ理由。修復のための一覧が返す列名は、
/// **そのまま `sqlite3` で `SELECT <その名前> FROM memory` と打てなければ意味が無い。**
/// 綴りがずれた瞬間、手掛かりは「存在しない列」を指す。
///
/// # `parse` を通さない列を書き並べてある意味
///
/// `NOT_PARSED` に並ぶのは「壊れていても `MemoryColumn` では指し示せない列」。
/// **`memory` に列を足すと、この突き合わせが赤くなる**ので、
/// 「その列は `parse` を通すのか（＝壊れた行として指し示せるべきか）」を必ず決めることになる。
///
/// **`locked` は `NOT_PARSED` 側ではない。** `memory_from_row` は 0 / 1 以外の
/// `locked` を `InvalidStoredValue` で弾く（＝その1行でチャットも抽出も止まる）ので、
/// 指し示せなければ修復の入口から見えなくなる（`MemoryColumn::Locked` の doc）。
#[tokio::test]
async fn every_pointed_at_column_is_a_real_memory_column() {
    let (_dir, store) = setup().await;

    let rows = sqlx::query("SELECT name FROM pragma_table_info('memory')")
        .fetch_all(store.pool())
        .await
        .expect("memory の列一覧を取得できること");
    let actual: BTreeSet<String> = rows
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(
        !actual.is_empty(),
        "前提: PRAGMA が列を返すこと（空なら突き合わせが無意味になる）"
    );

    let pointed: BTreeSet<String> = MemoryColumn::ALL
        .iter()
        .map(|column| column.as_str().to_owned())
        .collect();
    assert_eq!(
        pointed.len(),
        MemoryColumn::ALL.len(),
        "2つの variant が同じ列名を指していないこと: {pointed:?}"
    );

    // **`parse` を通さない列。** `id` / `character_id` は主キーと外部キー、
    // `created_at` は DB が書く日時、`superseded_by` は一覧が絞り込みに使う列
    // （読めない値なら行がそもそも一覧に載らない）。
    const NOT_PARSED: [&str; 4] = ["id", "character_id", "created_at", "superseded_by"];
    let expected: BTreeSet<String> = pointed
        .iter()
        .cloned()
        .chain(NOT_PARSED.iter().map(|name| (*name).to_owned()))
        .collect();

    assert_eq!(
        actual, expected,
        "`MemoryColumn` の列名と `memory` の実在の列が一致すること \
         （一覧の invalid_columns が、そのまま sqlite3 で打てる列名であること）"
    );
}

/// 明示的に作った索引が、**列構成・列順・`DESC` まで含めて**過不足なく存在する。
///
/// **テーブル再構築（`0003`）で落ちても正常系のテストは気づけない**ため、ここで固定する。
/// 名前と対象テーブルだけでは「同じ名前で中身が違う索引」を見逃すので、
/// `pragma_index_xinfo` で実際のキー列まで読む（`key = 1` が宣言したキー列。
/// `key = 0` は SQLite が末尾に足す rowid なので除く）。
#[tokio::test]
async fn migration_creates_every_declared_index() {
    let (_dir, store) = setup().await;

    let rows = sqlx::query(
        "SELECT m.name AS index_name, m.tbl_name AS table_name, \
                x.name AS column_name, x.\"desc\" AS is_desc \
         FROM sqlite_master AS m \
         JOIN pragma_index_xinfo(m.name) AS x \
         WHERE m.type = 'index' AND m.name NOT LIKE 'sqlite_%' AND x.key = 1 \
         ORDER BY m.name, x.seqno",
    )
    .fetch_all(store.pool())
    .await
    .expect("索引一覧を取得できること");

    // 索引名ごとにキー列を畳み込む。`ORDER BY m.name, x.seqno` で並んでいるので、
    // 直前の要素と索引名を比べるだけでよい。
    let mut indexes: Vec<IndexShape> = Vec::new();
    for row in &rows {
        let index_name = row.get::<String, _>("index_name");
        let table_name = row.get::<String, _>("table_name");
        // 式索引の列名は NULL で返る。定数の印に寄せて比較可能にする。
        let column = row
            .get::<Option<String>, _>("column_name")
            .unwrap_or_else(|| "<expr>".to_owned());
        let is_desc = row.get::<i64, _>("is_desc");

        match indexes.last_mut() {
            Some((name, _, columns)) if *name == index_name => columns.push((column, is_desc)),
            _ => indexes.push((index_name, table_name, vec![(column, is_desc)])),
        }
    }

    let expected: Vec<IndexShape> = EXPECTED_INDEXES
        .iter()
        .map(|(name, table, columns)| {
            (
                (*name).to_owned(),
                (*table).to_owned(),
                columns
                    .iter()
                    .map(|(column, is_desc)| ((*column).to_owned(), *is_desc))
                    .collect(),
            )
        })
        .collect();

    assert_eq!(
        indexes, expected,
        "宣言した索引が列順と DESC まで含めて作られること（再構築で落としていないこと）"
    );
}

/// **`session.id` は行を削除しても再利用されない**（ADR-0010 / `0003`）。
///
/// `session.id` は `model` フィールドに `aoi@42` の形で **DB の外へ出る**唯一の id。
/// 再利用されると、利用者のクライアント設定に残った `aoi@42` や、応答の `model` として
/// 返した値が**別のセッションを指す**。ADR-0004 が最も避けたい「気づきにくい記憶の混線」
/// そのものになる。
///
/// **スキーマ文字列に `AUTOINCREMENT` があることを見ても足りない。**
/// 宣言だけあって効いていない状態を検出できないので、実際の採番で確かめる。
#[tokio::test]
async fn session_id_is_never_reused_after_deleting_the_highest_row() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    let first = insert_session(&store, character_id).await;
    let highest = insert_session(&store, character_id).await;
    assert!(
        highest > first,
        "id が単調増加すること: {first} → {highest}"
    );

    // 最大 id の行を消す。`AUTOINCREMENT` が無ければ、次の行が同じ id を取る。
    delete_session(&store, highest).await;

    let next = insert_session(&store, character_id).await;
    assert!(
        next > highest,
        "最大 id の行を消しても id を再利用しないこと: 消した id = {highest} / 次の id = {next}"
    );

    // **全行を消しても 1 に戻らない。** `AUTOINCREMENT` が無ければ必ず 1 から振り直す。
    delete_session(&store, first).await;
    delete_session(&store, next).await;
    let empty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session")
        .fetch_one(store.pool())
        .await
        .expect("行数を取得できること");
    assert_eq!(empty, 0, "前提: セッションが空になっていること");

    let after_empty = insert_session(&store, character_id).await;
    assert!(
        after_empty > next,
        "全行を消しても採番が巻き戻らないこと: 直前の最大 id = {next} / 次の id = {after_empty}"
    );
}

/// **`message.id` は行を削除しても再利用されない**（ADR-0010 / `0003`）。
///
/// メッセージの時系列は `id` 昇順で表現している（`SELECT_RECENT_MESSAGES` の
/// `ORDER BY id DESC`）。再利用されると、後から入れたメッセージが**古い位置に紛れ込み**、
/// プロンプトの会話履歴が入れ替わる。
#[tokio::test]
async fn message_id_is_never_reused_after_deleting_the_highest_row() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_id = insert_session(&store, character_id).await;

    let first = insert_message(&store, session_id).await;
    let highest = insert_message(&store, session_id).await;
    assert!(
        highest > first,
        "id が単調増加すること: {first} → {highest}"
    );

    delete_message(&store, highest).await;

    let next = insert_message(&store, session_id).await;
    assert!(
        next > highest,
        "最大 id の行を消しても id を再利用しないこと: 消した id = {highest} / 次の id = {next}"
    );

    delete_message(&store, first).await;
    delete_message(&store, next).await;
    let empty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM message")
        .fetch_one(store.pool())
        .await
        .expect("行数を取得できること");
    assert_eq!(empty, 0, "前提: メッセージが空になっていること");

    let after_empty = insert_message(&store, session_id).await;
    assert!(
        after_empty > next,
        "全行を消しても採番が巻き戻らないこと: 直前の最大 id = {next} / 次の id = {after_empty}"
    );
}

/// 再接続をまたいでも採番が巻き戻らない。
///
/// **`AUTOINCREMENT` の状態は `sqlite_sequence` テーブルに永続化される。**
/// 「接続中だけメモリ上で最大値を覚えている」実装だと、プロセスを跨いだ時点で
/// 再利用が復活する。`aoi@42` は再起動を跨いで残る値なので、ここが要。
#[tokio::test]
async fn session_and_message_ids_do_not_rewind_across_reconnects() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("personacore.sqlite");

    let (session_id, message_id) = {
        let store = Store::connect(&path).await.expect("初回接続が成功すること");
        let character_id = insert_character(&store, "aoi").await;
        let session_id = insert_session(&store, character_id).await;
        let message_id = insert_message(&store, session_id).await;

        delete_message(&store, message_id).await;
        delete_session(&store, session_id).await;
        store.close().await;
        (session_id, message_id)
    };

    let store = Store::connect(&path).await.expect("再接続が成功すること");
    let character_id: i64 = sqlx::query_scalar("SELECT id FROM character WHERE handle = 'aoi'")
        .fetch_one(store.pool())
        .await
        .expect("キャラクターを取得できること");

    let next_session = insert_session(&store, character_id).await;
    assert!(
        next_session > session_id,
        "再接続後もセッション id を再利用しないこと: 消した id = {session_id} / 次の id = {next_session}"
    );

    let next_message = insert_message(&store, next_session).await;
    assert!(
        next_message > message_id,
        "再接続後もメッセージ id を再利用しないこと: 消した id = {message_id} / 次の id = {next_message}"
    );
}

/// **`0003` の再構築が既存データを保全する。**
///
/// 他のテストはすべて `0001`〜`0003` を一度に適用した**空の DB** を見るため、
/// 「再構築で行が消える」を誰も検出できない。`0003` は外部キーを切れない都合で
/// **4テーブルを作り直す**（理由は `0003` の冒頭）ので、消える範囲は広い。
///
/// ここでは `0002` までを適用した DB に行を入れ、そこへ `0003` を当てて確かめる。
/// **id まで含めて往復すること**が要点で、`session.id` が変われば外に出た `aoi@42` が
/// 別のセッションを指す（ADR-0010 が止めたかったことそのものが移行で起きる）。
///
/// **時刻列も同格で見る。** 復元 INSERT から `created_at` / `updated_at` を落とすと
/// `DEFAULT (strftime(...,'now'))` が効くのでエラーにならず、**静かに移行時刻へ**書き換わる。
/// `session.updated_at` が全行同じ値になると `session_character_recent_idx` で引く
/// 「`aoi` = 最新セッションを継続」（ADR-0004）が**どのセッションに繋がるか不定**になり、
/// `session.id` の再利用を塞いだ意味が抜ける。
#[tokio::test]
async fn migration_preserves_existing_rows_and_their_ids() {
    // **時刻は DEFAULT に任せず明示的に入れる。** DEFAULT 任せだと、復元 INSERT が
    // その列を落としても「移行時刻で入り直した値」と区別が付かずアサートが素通りする。
    // 行ごと・列ごとに違う値にして、**列の取り違え**（created_at と updated_at の入替）も
    // 検出できるようにする。現在時刻と紛れないよう、いずれも十分に過去の値にする。
    const SESSION_CREATED_AT: &str = "2024-01-02T03:04:05.678Z";
    const SESSION_UPDATED_AT: &str = "2024-05-06T07:08:09.012Z";
    const MESSAGE_CREATED_AT: &str = "2023-11-12T13:14:15.161Z";
    const SUMMARY_CREATED_AT: &str = "2022-07-08T09:10:11.121Z";
    const IMAGE_CREATED_AT: &str = "2021-03-04T05:06:07.089Z";

    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("legacy.sqlite");
    let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");

    // --- `0002` までを適用した「旧スキーマの DB」を作る ---
    {
        let pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .foreign_keys(true),
        )
        .await
        .expect("旧スキーマ用のプールを開けること");

        // 版 2 までのファイルだけを一時ディレクトリへ写し、そこから `Migrator` を作る。
        // `Migrator::migrations` は `#[doc(hidden)]`（semver 対象外）なので触らない。
        // `Migrator::new` は `&Path` を受け取れるため、公開 API だけで部分適用にできる。
        // **チェックサムはファイル内容から計算される**ので、後段の `Store::connect` が
        // `_sqlx_migrations` と照合するときもそのまま整合する。
        let subset_dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let mut copied: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&migrations).expect("migrations ディレクトリを読めること")
        {
            let entry = entry.expect("マイグレーションのエントリを読めること");
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("0001_") || name.starts_with("0002_") {
                std::fs::copy(entry.path(), subset_dir.path().join(&name))
                    .expect("マイグレーションをコピーできること");
                copied.push(name);
            }
        }
        // **前提の確認を省かない。** 抽出が壊れたときに黙って全適用になるのを防ぐ。
        assert_eq!(
            copied.len(),
            2,
            "前提: 版 2 までがちょうど2件であること: {copied:?}"
        );

        let migrator = Migrator::new(subset_dir.path())
            .await
            .expect("マイグレーションを読めること");
        assert_eq!(
            migrator.iter().count(),
            2,
            "前提: 適用対象が版 2 までの2件であること"
        );
        migrator
            .run(&pool)
            .await
            .expect("版 2 までを適用できること");

        // **id は 1 始まりにしない。** 連番で振り直しても気づけないため、飛ばした値を使う。
        sqlx::query("INSERT INTO character (id, name, base_persona) VALUES (3, 'aoi', ?)")
            .bind("ペルソナ本文")
            .execute(&pool)
            .await
            .expect("キャラクターを入れられること");
        sqlx::query(
            "INSERT INTO session (id, character_id, label, created_at, updated_at) \
             VALUES (7, 3, ?, ?, ?)",
        )
        .bind("移行前のセッション")
        .bind(SESSION_CREATED_AT)
        .bind(SESSION_UPDATED_AT)
        .execute(&pool)
        .await
        .expect("セッションを入れられること");
        sqlx::query(
            "INSERT INTO message \
             (id, session_id, role, dialogue, narration, raw_content, created_at) \
             VALUES (11, 7, 'assistant', ?, ?, ?, ?)",
        )
        .bind("こんにちは")
        .bind("彼女は微笑んだ")
        .bind("「こんにちは」彼女は微笑んだ")
        .bind(MESSAGE_CREATED_AT)
        .execute(&pool)
        .await
        .expect("メッセージを入れられること");
        sqlx::query(
            "INSERT INTO summary (id, session_id, upto_message_id, content, created_at) \
             VALUES (5, 7, 11, ?, ?)",
        )
        .bind("要約本文")
        .bind(SUMMARY_CREATED_AT)
        .execute(&pool)
        .await
        .expect("要約を入れられること");
        sqlx::query(
            "INSERT INTO generated_image \
             (id, session_id, message_id, positive_prompt, negative_prompt, seed, \
              checkpoint, file_path, created_at) \
             VALUES (9, 7, 11, ?, ?, ?, ?, ?, ?)",
        )
        .bind("1girl, smile")
        .bind("worst quality")
        .bind(4_242_i64)
        .bind("some-checkpoint.safetensors")
        .bind("images/0001.png")
        .bind(IMAGE_CREATED_AT)
        .execute(&pool)
        .await
        .expect("画像記録を入れられること");

        pool.close().await;
    }

    // --- `0003` を当てる ---
    let store = Store::connect(&path)
        .await
        .expect("既存データのある DB にも 0003 を適用できること");

    let session =
        sqlx::query("SELECT character_id, label, created_at, updated_at FROM session WHERE id = 7")
            .fetch_one(store.pool())
            .await
            .expect("移行前のセッションが id ごと残っていること");
    assert_eq!(session.get::<i64, _>("character_id"), 3);
    assert_eq!(
        session.get::<Option<String>, _>("label").as_deref(),
        Some("移行前のセッション"),
        "セッションの列が保全されること"
    );
    assert_eq!(
        session.get::<String, _>("created_at"),
        SESSION_CREATED_AT,
        "セッションの作成時刻が移行時刻へ書き換わらないこと"
    );
    assert_eq!(
        session.get::<String, _>("updated_at"),
        SESSION_UPDATED_AT,
        "セッションの更新時刻が移行時刻へ書き換わらないこと\
         （潰れると ADR-0004 の「最新セッションを継続」が不定になる）"
    );

    let message = sqlx::query(
        "SELECT session_id, role, dialogue, narration, raw_content, created_at \
         FROM message WHERE id = 11",
    )
    .fetch_one(store.pool())
    .await
    .expect("移行前のメッセージが id ごと残っていること");
    assert_eq!(message.get::<i64, _>("session_id"), 7);
    assert_eq!(message.get::<String, _>("role"), "assistant");
    assert_eq!(
        message.get::<Option<String>, _>("dialogue").as_deref(),
        Some("こんにちは")
    );
    assert_eq!(
        message.get::<Option<String>, _>("narration").as_deref(),
        Some("彼女は微笑んだ")
    );
    assert_eq!(
        message.get::<String, _>("raw_content"),
        "「こんにちは」彼女は微笑んだ",
        "生出力が保全されること"
    );
    assert_eq!(
        message.get::<String, _>("created_at"),
        MESSAGE_CREATED_AT,
        "メッセージの作成時刻が移行時刻へ書き換わらないこと"
    );

    let summary = sqlx::query(
        "SELECT session_id, upto_message_id, content, created_at FROM summary WHERE id = 5",
    )
    .fetch_one(store.pool())
    .await
    .expect("要約が id ごと残っていること");
    assert_eq!(summary.get::<i64, _>("session_id"), 7);
    assert_eq!(
        summary.get::<i64, _>("upto_message_id"),
        11,
        "複合外部キーの参照先を指したまま残ること"
    );
    assert_eq!(summary.get::<String, _>("content"), "要約本文");
    assert_eq!(
        summary.get::<String, _>("created_at"),
        SUMMARY_CREATED_AT,
        "要約の作成時刻が移行時刻へ書き換わらないこと"
    );

    let image = sqlx::query(
        "SELECT session_id, message_id, positive_prompt, negative_prompt, seed, checkpoint, \
         file_path, created_at, typeof(seed) AS seed_type FROM generated_image WHERE id = 9",
    )
    .fetch_one(store.pool())
    .await
    .expect("画像記録が id ごと残っていること");
    assert_eq!(image.get::<i64, _>("session_id"), 7);
    assert_eq!(image.get::<Option<i64>, _>("message_id"), Some(11));
    assert_eq!(image.get::<String, _>("positive_prompt"), "1girl, smile");
    assert_eq!(image.get::<String, _>("negative_prompt"), "worst quality");
    assert_eq!(
        image.get::<i64, _>("seed"),
        4_242,
        "再現情報が保全されること"
    );
    assert_eq!(
        image.get::<String, _>("seed_type"),
        "integer",
        "退避と復元で seed の型が崩れないこと"
    );
    assert_eq!(
        image.get::<String, _>("checkpoint"),
        "some-checkpoint.safetensors"
    );
    assert_eq!(image.get::<String, _>("file_path"), "images/0001.png");
    assert_eq!(
        image.get::<String, _>("created_at"),
        IMAGE_CREATED_AT,
        "画像記録の作成時刻が移行時刻へ書き換わらないこと"
    );

    // 採番は移行前の最大値の続きから。**1 に巻き戻ると `aoi@7` が別物を指す。**
    let next_session = insert_session(&store, 3).await;
    assert!(
        next_session > 7,
        "移行後の採番が既存 id を踏まないこと: 次の session.id = {next_session}"
    );
    let next_message = insert_message(&store, next_session).await;
    assert!(
        next_message > 11,
        "移行後の採番が既存 id を踏まないこと: 次の message.id = {next_message}"
    );
}

/// **`0004` が既存行の `display_name` を `name` の値で埋める。**
///
/// 移行の要点はここだけ。バックフィルを落としても DDL は成功し、
/// **空の DB では誰も気づけない**（既存行が無いので差が出ない）ため、
/// `0003` までを適用した DB に行を入れてから `0004` を当てて確かめる。
///
/// 埋め損なうと、そのキャラクターは表示名が既定値のまま残り、
/// **プロンプトに載る呼び名が黙って別物になる**
/// （0012a 以降は `PATCH` で直せるが、**呼び名がずれていることに気付く手段が無い**）。
#[tokio::test]
async fn migration_backfills_display_name_from_the_existing_handle() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("legacy.sqlite");
    let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");

    // --- `0003` までを適用した「旧スキーマの DB」を作る ---
    {
        let pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .foreign_keys(true),
        )
        .await
        .expect("旧スキーマ用のプールを開けること");

        // 版 3 までのファイルだけを一時ディレクトリへ写して部分適用する
        // （`migration_preserves_existing_rows_and_their_ids` と同じ手口）。
        let subset_dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let mut copied: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&migrations).expect("migrations ディレクトリを読めること")
        {
            let entry = entry.expect("マイグレーションのエントリを読めること");
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("0001_") || name.starts_with("0002_") || name.starts_with("0003_") {
                std::fs::copy(entry.path(), subset_dir.path().join(&name))
                    .expect("マイグレーションをコピーできること");
                copied.push(name);
            }
        }
        // **前提の確認を省かない。** 抽出が壊れたときに黙って全適用になるのを防ぐ。
        assert_eq!(
            copied.len(),
            3,
            "前提: 版 3 までがちょうど3件であること: {copied:?}"
        );

        let migrator = Migrator::new(subset_dir.path())
            .await
            .expect("マイグレーションを読めること");
        assert_eq!(
            migrator.iter().count(),
            3,
            "前提: 適用対象が版 3 までの3件であること"
        );
        migrator
            .run(&pool)
            .await
            .expect("版 3 までを適用できること");

        // **`display_name` 列がまだ無いこと**を確かめてから入れる
        // （列が既にあると、このテストは何も検証していない）。
        let err = sqlx::query("SELECT display_name FROM character")
            .fetch_all(&pool)
            .await
            .expect_err("移行前は display_name 列が存在しないこと");
        assert!(
            err.to_string().contains("display_name"),
            "実際のエラー = {err}"
        );

        // 名前の異なる複数行。1件だけだと「全行を同じ値で埋める」実装を見逃す。
        for (id, name) in [(3_i64, "aoi"), (8, "aoi-v2"), (11, "rin")] {
            sqlx::query("INSERT INTO character (id, name, base_persona) VALUES (?, ?, ?)")
                .bind(id)
                .bind(name)
                .bind("ペルソナ本文")
                .execute(&pool)
                .await
                .expect("キャラクターを入れられること");
        }

        pool.close().await;
    }

    // --- `0004` を当てる ---
    let store = Store::connect(&path)
        .await
        .expect("既存データのある DB にも 0004 を適用できること");

    for (id, name) in [(3_i64, "aoi"), (8, "aoi-v2"), (11, "rin")] {
        let row = sqlx::query("SELECT handle, display_name FROM character WHERE id = ?")
            .bind(id)
            .fetch_one(store.pool())
            .await
            .expect("移行前のキャラクターが id ごと残っていること");
        assert_eq!(
            row.get::<String, _>("handle"),
            name,
            "識別子が保全されること"
        );
        assert_eq!(
            row.get::<String, _>("display_name"),
            name,
            "表示名が移行前の名前で埋まること（id = {id}）"
        );
    }
}

/// 版 `upto` までのマイグレーションだけを適用した DB を作り、そのプールを返す。
///
/// **`Migrator::migrations` は `#[doc(hidden)]`（semver 対象外）なので触らない。**
/// 対象のファイルだけを一時ディレクトリへ写し、公開 API の `Migrator::new(&Path)` から
/// 部分適用する。**チェックサムはファイル内容から計算される**ので、
/// 後段の `Store::connect` が `_sqlx_migrations` と照合するときもそのまま整合する。
///
/// 戻り値の `TempDir` は写し先。落とすと消えるので呼び出し側で保持する。
async fn apply_migrations_up_to(path: &Path, upto: usize) -> (TempDir, SqlitePool) {
    let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let subset_dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");

    let mut copied: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&migrations).expect("migrations ディレクトリを読めること")
    {
        let entry = entry.expect("マイグレーションのエントリを読めること");
        let name = entry.file_name().to_string_lossy().into_owned();
        let version: usize = name
            .split('_')
            .next()
            .and_then(|prefix| prefix.parse().ok())
            .unwrap_or_else(|| panic!("版番号で始まるファイル名であること: {name}"));
        if version <= upto {
            std::fs::copy(entry.path(), subset_dir.path().join(&name))
                .expect("マイグレーションをコピーできること");
            copied.push(name);
        }
    }
    // **前提の確認を省かない。** 抽出が壊れたときに黙って全適用になるのを防ぐ。
    assert_eq!(
        copied.len(),
        upto,
        "前提: 版 {upto} までがちょうど {upto} 件であること: {copied:?}"
    );

    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true),
    )
    .await
    .expect("旧スキーマ用のプールを開けること");

    let migrator = Migrator::new(subset_dir.path())
        .await
        .expect("マイグレーションを読めること");
    assert_eq!(
        migrator.iter().count(),
        upto,
        "前提: 適用対象が版 {upto} までであること"
    );
    migrator
        .run(&pool)
        .await
        .expect("指定した版までを適用できること");

    (subset_dir, pool)
}

/// **`0005` の再構築が8テーブル全部の行を保全する。**
///
/// `0005` は `character.handle` に CHECK を足すためだけの移行で、**作り直すのは
/// `character` 1表だけ**（`-- no-transaction` で外部キーを切れるようにしてある。
/// 詳細は `0005` の冒頭）。**それでも8テーブル全部を見る。**
///
/// # なぜ「触らない7表」まで見るのか
///
/// **外部キーを切り損ねると、この移行は7表を全滅させる。**
/// `DROP TABLE character` は暗黙の `DELETE FROM` を伴うので、`PRAGMA foreign_keys` が
/// 効いていないと stat_alloc / session / memory / character_appearance が
/// `ON DELETE CASCADE` で消え、session 経由で message / summary / generated_image まで
/// 連鎖する。**エラーは出ない**（実測 2026-08-16: memory 2 行 → 0 行）。
///
/// **他のテストはすべて空の DB を見る**ので、これを誰も検出できない。
///
/// # 見るもの
///
/// * **id まで含めて往復すること。** `session.id` が変われば外に出た `aoi@42` が
///   別のセッションを指す（ADR-0010 が止めたかったことが移行で起きる）。
/// * **時刻列も同格。** 写す INSERT の列から落とすと `DEFAULT (strftime(...,'now'))` が
///   効くのでエラーにならず、**静かに移行時刻へ**書き換わる。
/// * **`memory.superseded_by`**（絶対制約「時系列保持」）。`memory` を落とさない形に
///   したので繋がりは自然に残るが、**それは「外部キーが切れていれば」の話。**
///   切れていなければ古い行ごと消えるので、ここが最初に赤くなる。
/// * **`memory.locked` / `generated_image` の再現情報**（seed の型まで）。
#[tokio::test]
async fn the_character_rebuild_preserves_every_table() {
    // **時刻は DEFAULT に任せず明示的に入れる**（`0003` の同種テストと同じ理由）。
    const CHARACTER_CREATED_AT: &str = "2020-01-02T03:04:05.678Z";
    const MEMORY_CREATED_AT: &str = "2020-02-03T04:05:06.789Z";
    const SESSION_CREATED_AT: &str = "2024-01-02T03:04:05.678Z";
    const SESSION_UPDATED_AT: &str = "2024-05-06T07:08:09.012Z";
    const MESSAGE_CREATED_AT: &str = "2023-11-12T13:14:15.161Z";
    const SUMMARY_CREATED_AT: &str = "2022-07-08T09:10:11.121Z";
    const IMAGE_CREATED_AT: &str = "2021-03-04T05:06:07.089Z";

    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("legacy.sqlite");

    // --- `0004` までを適用した「旧スキーマの DB」を作る ---
    {
        let (_subset, pool) = apply_migrations_up_to(&path, 4).await;

        // **id は 1 始まりにしない。** 連番で振り直しても気づけないため、飛ばした値を使う。
        sqlx::query(
            "INSERT INTO character \
             (id, name, display_name, base_persona, output_format_instruction, created_at) \
             VALUES (3, 'aoi', ?, ?, ?, ?)",
        )
        .bind("葵")
        .bind("静かな司書。")
        .bind("台詞は「」で囲む。")
        .bind(CHARACTER_CREATED_AT)
        .execute(&pool)
        .await
        .expect("キャラクターを入れられること");
        sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (3, ?, 4)")
            .bind("ロジック")
            .execute(&pool)
            .await
            .expect("性格ステータスを入れられること");
        sqlx::query(
            "INSERT INTO character_appearance \
             (character_id, fixed_tags, lora_name, lora_strength) VALUES (3, ?, ?, 0.75)",
        )
        .bind("1girl, blue hair")
        .bind("some-lora")
        .execute(&pool)
        .await
        .expect("外見定義を入れられること");
        // **後継行を先に入れる**（旧スキーマでも外部キーは有効なので順序が要る）。
        for (id, content, superseded_by, locked) in [
            (22_i64, "いまは京都に住んでいる", None::<i64>, 1_i64),
            (21, "東京に住んでいた", Some(22), 0),
        ] {
            sqlx::query(
                "INSERT INTO memory \
                 (id, character_id, subject, content, category, created_at, superseded_by, locked) \
                 VALUES (?, 3, 'user', ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(content)
            .bind("居住地")
            .bind(MEMORY_CREATED_AT)
            .bind(superseded_by)
            .bind(locked)
            .execute(&pool)
            .await
            .expect("記憶を入れられること");
        }
        sqlx::query(
            "INSERT INTO session (id, character_id, label, created_at, updated_at) \
             VALUES (7, 3, ?, ?, ?)",
        )
        .bind("移行前のセッション")
        .bind(SESSION_CREATED_AT)
        .bind(SESSION_UPDATED_AT)
        .execute(&pool)
        .await
        .expect("セッションを入れられること");
        sqlx::query(
            "INSERT INTO message \
             (id, session_id, role, dialogue, narration, raw_content, created_at) \
             VALUES (11, 7, 'assistant', ?, ?, ?, ?)",
        )
        .bind("こんにちは")
        .bind("彼女は微笑んだ")
        .bind("「こんにちは」彼女は微笑んだ")
        .bind(MESSAGE_CREATED_AT)
        .execute(&pool)
        .await
        .expect("メッセージを入れられること");
        sqlx::query(
            "INSERT INTO summary (id, session_id, upto_message_id, content, created_at) \
             VALUES (5, 7, 11, ?, ?)",
        )
        .bind("要約本文")
        .bind(SUMMARY_CREATED_AT)
        .execute(&pool)
        .await
        .expect("要約を入れられること");
        sqlx::query(
            "INSERT INTO generated_image \
             (id, session_id, message_id, positive_prompt, negative_prompt, seed, \
              checkpoint, file_path, created_at) \
             VALUES (9, 7, 11, ?, ?, ?, ?, ?, ?)",
        )
        .bind("1girl, smile")
        .bind("worst quality")
        .bind(4_242_i64)
        .bind("some-checkpoint.safetensors")
        .bind("images/0001.png")
        .bind(IMAGE_CREATED_AT)
        .execute(&pool)
        .await
        .expect("画像記録を入れられること");

        pool.close().await;
    }

    // --- `0005` を当てる ---
    let store = Store::connect(&path)
        .await
        .expect("既存データのある DB にも 0005 を適用できること");

    let character = sqlx::query(
        "SELECT handle, display_name, base_persona, output_format_instruction, created_at \
         FROM character WHERE id = 3",
    )
    .fetch_one(store.pool())
    .await
    .expect("キャラクターが id ごと残っていること");
    assert_eq!(character.get::<String, _>("handle"), "aoi");
    assert_eq!(character.get::<String, _>("display_name"), "葵");
    assert_eq!(character.get::<String, _>("base_persona"), "静かな司書。");
    assert_eq!(
        character
            .get::<Option<String>, _>("output_format_instruction")
            .as_deref(),
        Some("台詞は「」で囲む。"),
        "形式指示が保全されること"
    );
    assert_eq!(
        character.get::<String, _>("created_at"),
        CHARACTER_CREATED_AT,
        "キャラクターの作成時刻が移行時刻へ書き換わらないこと"
    );

    let stat = sqlx::query("SELECT axis, value FROM stat_alloc WHERE character_id = 3")
        .fetch_one(store.pool())
        .await
        .expect("性格ステータスが残っていること");
    assert_eq!(stat.get::<String, _>("axis"), "ロジック");
    assert_eq!(stat.get::<i64, _>("value"), 4);

    let appearance = sqlx::query(
        "SELECT fixed_tags, lora_name, lora_strength FROM character_appearance \
         WHERE character_id = 3",
    )
    .fetch_one(store.pool())
    .await
    .expect("外見定義が残っていること");
    assert_eq!(
        appearance.get::<String, _>("fixed_tags"),
        "1girl, blue hair"
    );
    assert_eq!(
        appearance.get::<Option<String>, _>("lora_name").as_deref(),
        Some("some-lora")
    );
    assert!((appearance.get::<f64, _>("lora_strength") - 0.75).abs() < f64::EPSILON);

    // **`superseded_by` は絶対制約。** 復元の UPDATE を落とすと、ここだけが NULL になる。
    let superseded = sqlx::query(
        "SELECT content, category, created_at, superseded_by, locked FROM memory WHERE id = 21",
    )
    .fetch_one(store.pool())
    .await
    .expect("古い記憶が id ごと残っていること");
    assert_eq!(superseded.get::<String, _>("content"), "東京に住んでいた");
    assert_eq!(
        superseded.get::<Option<String>, _>("category").as_deref(),
        Some("居住地")
    );
    assert_eq!(
        superseded.get::<String, _>("created_at"),
        MEMORY_CREATED_AT,
        "記憶の作成時刻が移行時刻へ書き換わらないこと"
    );
    assert_eq!(
        superseded.get::<Option<i64>, _>("superseded_by"),
        Some(22),
        "「以前はXだったが今はY」の繋がりが保全されること（絶対制約）"
    );
    assert_eq!(superseded.get::<i64, _>("locked"), 0);

    let latest = sqlx::query("SELECT superseded_by, locked FROM memory WHERE id = 22")
        .fetch_one(store.pool())
        .await
        .expect("新しい記憶が id ごと残っていること");
    assert_eq!(latest.get::<Option<i64>, _>("superseded_by"), None);
    assert_eq!(
        latest.get::<i64, _>("locked"),
        1,
        "保護フラグが保全されること"
    );

    let session =
        sqlx::query("SELECT character_id, label, created_at, updated_at FROM session WHERE id = 7")
            .fetch_one(store.pool())
            .await
            .expect("セッションが id ごと残っていること");
    assert_eq!(session.get::<i64, _>("character_id"), 3);
    assert_eq!(
        session.get::<Option<String>, _>("label").as_deref(),
        Some("移行前のセッション")
    );
    assert_eq!(session.get::<String, _>("created_at"), SESSION_CREATED_AT);
    assert_eq!(
        session.get::<String, _>("updated_at"),
        SESSION_UPDATED_AT,
        "更新時刻が潰れると ADR-0004 の「最新セッションを継続」が不定になる"
    );

    let message = sqlx::query(
        "SELECT session_id, role, dialogue, narration, raw_content, created_at \
         FROM message WHERE id = 11",
    )
    .fetch_one(store.pool())
    .await
    .expect("メッセージが id ごと残っていること");
    assert_eq!(message.get::<i64, _>("session_id"), 7);
    assert_eq!(message.get::<String, _>("role"), "assistant");
    assert_eq!(
        message.get::<Option<String>, _>("dialogue").as_deref(),
        Some("こんにちは")
    );
    assert_eq!(
        message.get::<Option<String>, _>("narration").as_deref(),
        Some("彼女は微笑んだ")
    );
    assert_eq!(
        message.get::<String, _>("raw_content"),
        "「こんにちは」彼女は微笑んだ",
        "生出力が保全されること（絶対制約）"
    );
    assert_eq!(message.get::<String, _>("created_at"), MESSAGE_CREATED_AT);

    let summary = sqlx::query(
        "SELECT session_id, upto_message_id, content, created_at FROM summary WHERE id = 5",
    )
    .fetch_one(store.pool())
    .await
    .expect("要約が id ごと残っていること");
    assert_eq!(summary.get::<i64, _>("session_id"), 7);
    assert_eq!(
        summary.get::<i64, _>("upto_message_id"),
        11,
        "複合外部キーの参照先を指したまま残ること"
    );
    assert_eq!(summary.get::<String, _>("content"), "要約本文");
    assert_eq!(summary.get::<String, _>("created_at"), SUMMARY_CREATED_AT);

    let image = sqlx::query(
        "SELECT session_id, message_id, positive_prompt, negative_prompt, seed, checkpoint, \
         file_path, created_at, typeof(seed) AS seed_type FROM generated_image WHERE id = 9",
    )
    .fetch_one(store.pool())
    .await
    .expect("画像記録が id ごと残っていること");
    assert_eq!(image.get::<i64, _>("session_id"), 7);
    assert_eq!(image.get::<Option<i64>, _>("message_id"), Some(11));
    assert_eq!(image.get::<String, _>("positive_prompt"), "1girl, smile");
    assert_eq!(image.get::<String, _>("negative_prompt"), "worst quality");
    assert_eq!(
        image.get::<i64, _>("seed"),
        4_242,
        "再現情報が保全されること（絶対制約）"
    );
    assert_eq!(
        image.get::<String, _>("seed_type"),
        "integer",
        "退避と復元で seed の型が崩れないこと"
    );
    assert_eq!(
        image.get::<String, _>("checkpoint"),
        "some-checkpoint.safetensors"
    );
    assert_eq!(image.get::<String, _>("file_path"), "images/0001.png");
    assert_eq!(image.get::<String, _>("created_at"), IMAGE_CREATED_AT);

    // 採番は移行前の最大値の続きから。**1 に巻き戻ると `aoi@7` が別物を指す。**
    let next_session = insert_session(&store, 3).await;
    assert!(
        next_session > 7,
        "移行後の採番が既存 id を踏まないこと: 次の session.id = {next_session}"
    );
    let next_message = insert_message(&store, next_session).await;
    assert!(
        next_message > 11,
        "移行後の採番が既存 id を踏まないこと: 次の message.id = {next_message}"
    );
}

/// **`0005` の再構築が `sqlite_sequence`（採番状態）を巻き戻さない**（ADR-0010）。
///
/// # 上のテストでは検出できない
///
/// 行が1件でも残っていれば、その行の id が `sqlite_sequence` を最大値まで押し上げる。
/// 差が出るのは**全行を消した後**（`sqlite_sequence` にだけ値が残っている状態）で、
/// このとき `session` / `message` を落とす移行は**採番が 1 に巻き戻る。**
///
/// `session.id` は `model` フィールドに `aoi@42` の形で DB の外へ出る唯一の id なので、
/// 巻き戻ると**外に残った `aoi@42` が別のセッションを指す**（ADR-0004 の記憶の混線）。
///
/// # **いまの `0005` はそもそも `session` / `message` を落とさない**
///
/// `character` 1表しか作り直さないので、`sqlite_sequence` に触れる余地が無い
/// （`0005` の冒頭 2）。**それでもこのテストは残す。**
/// 外部キーの切り忘れは `DROP TABLE character` を7表のカスケードに変え、
/// **`session` の行が消えた状態を作る**——そこから先はこのテストが見ている状況そのもの。
/// 「触らないから安全」は、触らないことが守られている間しか成り立たない。
#[tokio::test]
async fn the_character_rebuild_does_not_rewind_the_autoincrement_counters() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("legacy.sqlite");

    let (highest_session, highest_message) = {
        let (_subset, pool) = apply_migrations_up_to(&path, 4).await;

        sqlx::query("INSERT INTO character (id, name, base_persona) VALUES (3, 'aoi', ?)")
            .bind("ペルソナ本文")
            .execute(&pool)
            .await
            .expect("キャラクターを入れられること");
        sqlx::query("INSERT INTO session (id, character_id, label) VALUES (70, 3, ?)")
            .bind("消えるセッション")
            .execute(&pool)
            .await
            .expect("セッションを入れられること");
        sqlx::query(
            "INSERT INTO message (id, session_id, role, raw_content) VALUES (110, 70, 'user', ?)",
        )
        .bind("生出力")
        .execute(&pool)
        .await
        .expect("メッセージを入れられること");

        // **全部消す。** `sqlite_sequence` にだけ 70 / 110 が残る状態を作る。
        sqlx::query("DELETE FROM message")
            .execute(&pool)
            .await
            .expect("メッセージを消せること");
        sqlx::query("DELETE FROM session")
            .execute(&pool)
            .await
            .expect("セッションを消せること");

        let session_seq: i64 =
            sqlx::query_scalar("SELECT seq FROM sqlite_sequence WHERE name = 'session'")
                .fetch_one(&pool)
                .await
                .expect("前提: 採番状態が残っていること");
        let message_seq: i64 =
            sqlx::query_scalar("SELECT seq FROM sqlite_sequence WHERE name = 'message'")
                .fetch_one(&pool)
                .await
                .expect("前提: 採番状態が残っていること");
        assert_eq!(
            session_seq, 70,
            "前提: session の採番が 70 まで進んでいること"
        );
        assert_eq!(
            message_seq, 110,
            "前提: message の採番が 110 まで進んでいること"
        );

        pool.close().await;
        (session_seq, message_seq)
    };

    // --- `0005` を当てる ---
    let store = Store::connect(&path)
        .await
        .expect("空のテーブルがあっても 0005 を適用できること");

    let next_session = insert_session(&store, 3).await;
    assert!(
        next_session > highest_session,
        "移行で session の採番が巻き戻らないこと: 移行前 = {highest_session} / 次 = {next_session}"
    );
    let next_message = insert_message(&store, next_session).await;
    assert!(
        next_message > highest_message,
        "移行で message の採番が巻き戻らないこと: 移行前 = {highest_message} / 次 = {next_message}"
    );
}

/// **規則を満たさない識別子がある DB では、`0005` は黙って直さずに落ちる**（ADR-0017）。
///
/// 識別子は不変（ADR-0016）なので **API では直せない。** `Aoi` を `aoi` に、
/// `" aoi "` を `"aoi"` に勝手に直すと、**FE 側やクライアント設定に残った `model` が
/// 別のものを指す**（あるいは何も指さなくなる）。ADR-0004 が最も避けたい壊れ方なので、
/// 移行を止めて運用者に判断させる。
///
/// # メッセージまで見る理由
///
/// CHECK 違反の素のエラー（`CHECK constraint failed: length(name) > 0 AND ...`）では
/// **どの行が悪いのかも、何をすればよいのかも分からない。**
/// `0005` は先に明示的なトリガを置いて、
/// **該当行を洗い出す SQL ごと**エラーに載せている。ここが空文句に痩せたら赤くする。
///
/// # 直したら通ること
///
/// 「落ちる」だけを見ると、**何をしても落ちる実装**（無条件の RAISE）が緑を通る。
/// 直した後に適用できることまで見る。
#[tokio::test]
async fn the_migration_refuses_to_convert_handles_it_cannot_rewrite() {
    for bad in [
        " aoi ",
        "Aoi",
        "葵",
        "aoi.v2",
        "aoi\u{200B}",
        "aoi\u{0000}AOI",
    ] {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let path = dir.path().join("legacy.sqlite");

        {
            let (_subset, pool) = apply_migrations_up_to(&path, 4).await;
            // 正当な行も混ぜる。**1件でも規則外があれば全体が止まる**ことを見る。
            sqlx::query("INSERT INTO character (id, name, base_persona) VALUES (3, 'rin', ?)")
                .bind("ペルソナ本文")
                .execute(&pool)
                .await
                .expect("正当なキャラクターを入れられること");
            sqlx::query("INSERT INTO character (id, name, base_persona) VALUES (8, ?, ?)")
                .bind(bad)
                .bind("ペルソナ本文")
                .execute(&pool)
                .await
                .unwrap_or_else(|err| panic!("旧スキーマでは入ること: {bad:?}: {err}"));
            pool.close().await;
        }

        let err = Store::connect(&path)
            .await
            .err()
            .unwrap_or_else(|| panic!("規則外の識別子がある DB では移行が失敗すること: {bad:?}"));

        // **`Display` は定数文字列**（CLAUDE.md 絶対制約）。詳細は `source` 側。
        assert_eq!(err.to_string(), "マイグレーションの適用に失敗しました");
        let detail = std::error::Error::source(&err)
            .expect("source に理由が残っていること")
            .to_string();
        assert!(
            detail.contains("personacore 0005"),
            "どの移行で止まったかが分かること: {detail}"
        );
        assert!(
            detail.contains("SELECT id, name FROM character"),
            "該当行を洗い出す手順が載っていること: {detail}"
        );
        assert!(
            detail.contains("ADR-0016"),
            "API では直せない理由に辿れること: {detail}"
        );
        // **格納値そのものは載せない**（何が入っているか分からない値をそのまま反射しない）。
        assert!(
            !detail.contains(bad) || bad.chars().all(|c| c.is_ascii_alphanumeric()),
            "格納値をそのまま載せないこと: {detail}"
        );

        // --- 直せば通る（無条件に落ちる実装を赤にする） ---
        {
            // **版 4 のスキーマを触るので列は `name`**（改名は `0007`）。
            // ここを `handle` に「直す」と no such column で落ちる。
            let (_subset, pool) = apply_migrations_up_to(&path, 4).await;
            sqlx::query("UPDATE character SET name = 'aoi' WHERE id = 8")
                .execute(&pool)
                .await
                .expect("識別子を直せること");
            pool.close().await;
        }
        let store = Store::connect(&path)
            .await
            .unwrap_or_else(|err| panic!("直した後は適用できること: {bad:?}: {err}"));
        let names = sqlx::query_scalar::<_, String>("SELECT handle FROM character ORDER BY id")
            .fetch_all(store.pool())
            .await
            .expect("識別子を読めること");
        assert_eq!(names, vec!["rin".to_owned(), "aoi".to_owned()]);
    }
}

/// **`0005` が途中で止まっても、DB は移行前のまま使える**（作業単位 0004b-1 追補）。
///
/// # なぜ確かめる必要があるのか
///
/// `0005` は `-- no-transaction` で sqlx の包みを外し、**自分で `BEGIN` / `COMMIT` を書く**
/// （`PRAGMA foreign_keys = OFF` はトランザクションの中では無視されるため）。
/// その結果、**失敗したときの巻き戻しを sqlx に頼れない。**
///
/// 巻き戻し自体は明示 `BEGIN` が担うが、**`_sqlx_migrations` への記録だけは
/// 自分のトランザクションの外**に落ちる（sqlx は `no_tx` のとき、移行 SQL を流し終えてから
/// 別の文で記録する）。したがって「行は戻ったのに版だけ進んだ」あるいはその逆が
/// 起きていないかを、実際に落として確かめる。
///
/// # 見るもの
///
/// * **データが1行も欠けていない**（部分的に作り直した残骸が残っていない）。
/// * **版が進んでいない**（`_sqlx_migrations` に 5 が無い）。だから再実行できる。
/// * **作業用の表とトリガが残っていない**（残ると再実行が `already exists` で詰む）。
#[tokio::test]
async fn a_migration_that_stops_leaves_the_database_where_it_was() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("legacy.sqlite");

    {
        let (_subset, pool) = apply_migrations_up_to(&path, 4).await;
        // **移行を止める行。** 識別子が規則外なので `0005` のゲートが落とす。
        sqlx::query("INSERT INTO character (id, name, base_persona) VALUES (8, 'Aoi', ?)")
            .bind("ペルソナ本文")
            .execute(&pool)
            .await
            .expect("旧スキーマでは入ること");
        // **巻き添えを見るための行。** 部分的に作り直して落ちると、ここが消える。
        sqlx::query(
            "INSERT INTO memory (id, character_id, subject, content) VALUES (21, 8, 'user', ?)",
        )
        .bind("東京に住んでいた")
        .execute(&pool)
        .await
        .expect("記憶を入れられること");
        sqlx::query("INSERT INTO session (id, character_id, label) VALUES (70, 8, ?)")
            .bind("移行前のセッション")
            .execute(&pool)
            .await
            .expect("セッションを入れられること");
        pool.close().await;
    }

    assert!(
        Store::connect(&path).await.is_err(),
        "前提: 規則外の識別子があるので移行が止まること"
    );

    // 移行前の版（`0004`）で開き直して中身を見る。**`0005` は当てない。**
    let (_subset, pool) = apply_migrations_up_to(&path, 4).await;

    let applied: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .expect("適用済みの版を読めること");
    assert_eq!(
        applied,
        vec![1, 2, 3, 4],
        "止まった移行が版だけ進めていないこと（進むと再実行できない）"
    );

    // **版 4 のスキーマを読むので列は `name`**（改名は `0007`）。
    let handle: String = sqlx::query_scalar("SELECT name FROM character WHERE id = 8")
        .fetch_one(&pool)
        .await
        .expect("キャラクターが残っていること");
    assert_eq!(handle, "Aoi", "移行が識別子を勝手に書き換えていないこと");

    let memories: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory")
        .fetch_one(&pool)
        .await
        .expect("記憶の行数を読めること");
    assert_eq!(
        memories, 1,
        "止まった移行が記憶を消していないこと（絶対制約）"
    );

    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM session")
        .fetch_one(&pool)
        .await
        .expect("セッションの行数を読めること");
    assert_eq!(sessions, 1, "止まった移行がセッションを消していないこと");

    // 作業用の表・トリガが残っていると、再実行が `already exists` で詰む。
    let leftovers: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE name LIKE '%\\_0005' ESCAPE '\\' \
         OR name LIKE '%\\_0005\\_%' ESCAPE '\\' ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .expect("残骸を検索できること");
    assert!(
        leftovers.is_empty(),
        "作業用の表・トリガが残っていないこと: {leftovers:?}"
    );

    pool.close().await;

    // --- 直せば通る。**巻き戻ったことの裏取り**（残骸があるとここで落ちる） ---
    {
        // **版 4 のスキーマを触るので列は `name`**（改名は `0007`）。
        let (_subset, pool) = apply_migrations_up_to(&path, 4).await;
        sqlx::query("UPDATE character SET name = 'aoi' WHERE id = 8")
            .execute(&pool)
            .await
            .expect("識別子を直せること");
        pool.close().await;
    }
    let store = Store::connect(&path)
        .await
        .expect("直した後は同じ DB に 0005 を当てられること");
    let memories: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory")
        .fetch_one(store.pool())
        .await
        .expect("記憶の行数を読めること");
    assert_eq!(memories, 1, "やり直した移行でも記憶が残ること");
}

/// **親を失った子の行が残っていたら、`0005` は完了せずに止まる**（作業単位 0004b-1 追補）。
///
/// # なぜこの検査が要るのか
///
/// `0005` は SQLite 公式の 12 ステップに従い、`PRAGMA foreign_keys = OFF` を立ててから
/// `character` を作り直す。**その間、外部キーは一切検査されない。**
/// 公式手順の第 10 段が `PRAGMA foreign_key_check` を要求しているのはこのためで、
/// 落とすと**参照整合性が壊れた DB を、壊れたまま次の版へ運ぶ。**
///
/// 壊れた行は DB の直接編集で作れる（`PRAGMA foreign_keys = OFF` を立てて INSERT すれば
/// 済む）。移行を通した後では、`memory` の孤児は
/// **どのキャラクターの想起にも現れないまま残り続ける**——`recall_memories` は
/// `character_id` で引くので、誰も読まないし誰も気づかない。
///
/// # トリガで落とす理由
///
/// `PRAGMA foreign_key_check` は違反を**行として返すだけでエラーにならない。**
/// SQL だけで移行を止めるには RAISE を使うしかなく、RAISE が書けるのはトリガの中だけ。
/// **`pragma_foreign_key_check` 関数はトリガの中では使えない**
/// （`unsafe use of virtual table` / 実測 2026-08-16 sqlite 3.53.4）ので、
/// `character` の子4テーブルを素の副問い合わせで見ている。
///
/// # 「落ちた」だけでは足りない（0004b-1 追補）
///
/// このゲートは**作り直し（DROP / RENAME / CREATE INDEX）を済ませた後**に落ちる。
/// `a_migration_that_stops_leaves_the_database_where_it_was` が見ているのは
/// 識別子ゲート、つまり**作り直しの前**に落ちる場合で、**巻き戻すものが何も無い。**
/// DDL を済ませた後で落ちる経路は、ここでしか押さえられない。
///
/// そこでエラー文字列に加えて、あちらと同じ3点を見る:
///
/// * **作業用の表・トリガが残っていない**（残ると再実行が `already exists` /
///   `no such table` で詰み、**運用者が原因を正しく直しても二度と移行できない**）。
/// * **DROP / RENAME が巻き戻っている**（`character` の行が移行前のまま生きている）。
/// * **孤児を消せば `Store::connect` が通る**——つまり**復旧できる。** ここが本体。
///
/// 実測（2026-08-16）: `COMMIT;` を FK ゲートの**前**へ移すと1点目と3点目が赤くなる。
/// エラー文字列だけを見ていた頃は、この変異が 779 テスト全部を通り抜けていた。
#[tokio::test]
async fn the_rebuild_stops_when_a_child_row_has_lost_its_parent() {
    // 子テーブルごとに、孤児を1行だけ作って確かめる。**1つでも見落とせば穴になる。**
    // 3列目は孤児を数える手、4列目は**孤児を取り除く手**
    // （運用者が DB を直接編集して直す想定）。**表名を差し込んだ動的 SQL は組まない。**
    let orphans: [(&str, &str, &str, &str); 4] = [
        (
            "memory",
            "INSERT INTO memory (id, character_id, subject, content) VALUES (21, 404, 'user', '孤児')",
            "SELECT COUNT(*) FROM memory WHERE character_id = 404",
            "DELETE FROM memory WHERE character_id = 404",
        ),
        (
            "session",
            "INSERT INTO session (id, character_id, label) VALUES (70, 404, '孤児')",
            "SELECT COUNT(*) FROM session WHERE character_id = 404",
            "DELETE FROM session WHERE character_id = 404",
        ),
        (
            "stat_alloc",
            "INSERT INTO stat_alloc (character_id, axis, value) VALUES (404, 'ロジック', 4)",
            "SELECT COUNT(*) FROM stat_alloc WHERE character_id = 404",
            "DELETE FROM stat_alloc WHERE character_id = 404",
        ),
        (
            "character_appearance",
            "INSERT INTO character_appearance (character_id, fixed_tags) VALUES (404, '1girl')",
            "SELECT COUNT(*) FROM character_appearance WHERE character_id = 404",
            "DELETE FROM character_appearance WHERE character_id = 404",
        ),
    ];

    for (table, insert, count_orphan, remove_orphan) in orphans {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let path = dir.path().join("legacy.sqlite");

        {
            let (_subset, pool) = apply_migrations_up_to(&path, 4).await;
            // 正当な行も混ぜる。**識別子は規則を満たす**ので、止まる理由は孤児だけ。
            sqlx::query("INSERT INTO character (id, name, base_persona) VALUES (3, 'aoi', ?)")
                .bind("ペルソナ本文")
                .execute(&pool)
                .await
                .expect("正当なキャラクターを入れられること");

            // **DB の直接編集の再現。** 外部キーは接続ごとの設定なので、
            // 1本の接続で切ってから入れる（プールへ投げると別の接続に当たりうる）。
            let mut conn = pool.acquire().await.expect("接続を取得できること");
            sqlx::query("PRAGMA foreign_keys = OFF")
                .execute(&mut *conn)
                .await
                .expect("外部キーを切れること");
            sqlx::query(insert)
                .execute(&mut *conn)
                .await
                .unwrap_or_else(|err| panic!("{table} に孤児を作れること: {err}"));
            drop(conn);
            pool.close().await;
        }

        let err = Store::connect(&path)
            .await
            .err()
            .unwrap_or_else(|| panic!("{table} の孤児が残った DB では移行が止まること"));

        assert_eq!(err.to_string(), "マイグレーションの適用に失敗しました");
        let detail = std::error::Error::source(&err)
            .expect("source に理由が残っていること")
            .to_string();
        assert!(
            detail.contains("personacore 0005"),
            "どの移行で止まったかが分かること: {detail}"
        );
        assert!(
            detail.contains("PRAGMA foreign_key_check"),
            "調べ直す手順が載っていること: {detail}"
        );

        // --- 止まった後の DB を、移行前の版で開き直して見る。**`0005` は当てない。** ---
        let (_subset, pool) = apply_migrations_up_to(&path, 4).await;

        // 1. 作業用の表・トリガが残っていないこと。
        //    **このゲートはトランザクションの後半で作られる**ので、`COMMIT` の位置が
        //    ずれると確定して残り、再実行が詰む。
        let leftovers: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE name LIKE '%\\_0005' ESCAPE '\\' \
             OR name LIKE '%\\_0005\\_%' ESCAPE '\\' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .expect("残骸を検索できること");
        assert!(
            leftovers.is_empty(),
            "{table}: 作業用の表・トリガが残っていないこと: {leftovers:?}"
        );

        // 2. DROP / RENAME が巻き戻っていること。**作り直しの途中で確定していない。**
        //    **版 4 のスキーマを読むので列は `name`**（改名は `0007`）。
        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM character ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("キャラクターを読めること");
        assert_eq!(
            names,
            vec!["aoi".to_owned()],
            "{table}: 止まった移行が character の行を落としていないこと"
        );
        // 孤児も勝手に消えていないこと（黙って「直して」進むのではなく、運用者に委ねる）。
        let orphan_rows: i64 = sqlx::query_scalar(count_orphan)
            .fetch_one(&pool)
            .await
            .expect("孤児の行数を読めること");
        assert_eq!(
            orphan_rows, 1,
            "{table}: 止まった移行が孤児を黙って消していないこと"
        );

        // 3. **原因を取り除けば通ること。** ここが本体——止まった後に復旧できなければ、
        //    「止める」ことに意味が無い（DB を手で開いて残骸を掃除する以外に道が無くなる）。
        sqlx::query(remove_orphan)
            .execute(&pool)
            .await
            .unwrap_or_else(|err| panic!("{table}: 孤児を取り除けること: {err}"));
        pool.close().await;

        let store = Store::connect(&path)
            .await
            .unwrap_or_else(|err| panic!("{table}: 孤児を消した後は適用できること: {err}"));
        // **`0007` まで当たった後なので列は `handle`。** 上の巻き戻し確認（`0004` の
        // プールで読む方）は移行前のスキーマを見ているので `name` のまま。
        let names: Vec<String> = sqlx::query_scalar("SELECT handle FROM character ORDER BY id")
            .fetch_all(store.pool())
            .await
            .expect("移行後にキャラクターを読めること");
        assert_eq!(
            names,
            vec!["aoi".to_owned()],
            "{table}: やり直した移行でも character の行が残ること"
        );
    }
}

/// **`0005` は `character` 以外のテーブル定義に指一本触れない**（作業単位 0004b-1 追補）。
///
/// # 何を防いでいるのか
///
/// 「`character` を `character_old` へ改名してから新しい `character` を作る」書き方は
/// 一見自然だが、**外部キーが有効なまま改名すると、他テーブルの `REFERENCES` 句が
/// `character_old` へ書き換わる**（SQLite の仕様。実測 2026-08-16 / sqlite 3.53.4）。
/// 書き換わっても**エラーは出ない**し行も消えないので、正常系のテストは全部緑のまま。
/// 気づくのは「キャラクターを消したのに記憶が残っている」を誰かが見つけたときになる。
///
/// `0005` はこれを避けるため、**改名するのは新しく作った表の側だけ**にしてある
/// （作る → 写す → 旧表を落とす → 新表を改名）。旧表の名前は最後まで動かない。
///
/// # 引用符の話
///
/// `ALTER TABLE ... RENAME TO` を通った表の DDL は `CREATE TABLE "character"` と
/// **引用符付き**になる（SQLite が書き戻すときの整形）。テーブル名そのものは変わらないので
/// `sqlite_master.name` も `PRAGMA table_info` も従来どおり。
/// **DDL 文字列を突き合わせているテストが無い**ことを、ここで併せて固定しておく。
#[tokio::test]
async fn the_rebuild_leaves_the_other_table_definitions_untouched() {
    let (_dir, store) = setup().await;

    for table in ["memory", "session", "stat_alloc", "character_appearance"] {
        let ddl: String =
            sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?")
                .bind(table)
                .fetch_one(store.pool())
                .await
                .unwrap_or_else(|err| panic!("{table} の DDL を読めること: {err}"));
        assert!(
            ddl.contains("REFERENCES character(id)"),
            "{table} の参照先が書き換わっていないこと: {ddl}"
        );
        assert!(
            !ddl.contains("character_old") && !ddl.contains("character_new"),
            "{table} が作業用の表を指していないこと: {ddl}"
        );
    }

    // 参照が生きていること自体も見る（DDL の字面だけでは効いているか分からない）。
    let character_id = insert_character(&store, "aoi").await;
    insert_memory(&store, character_id, "東京に住んでいた").await;
    sqlx::query("DELETE FROM character WHERE id = ?")
        .bind(character_id)
        .execute(store.pool())
        .await
        .expect("キャラクターを消せること");
    assert_eq!(
        count_rows(&store, "SELECT COUNT(*) FROM memory").await,
        0,
        "参照が生きていれば ON DELETE CASCADE が届くこと"
    );
}

/// **表示名は空にできない**（`0004` の CHECK）。
///
/// 空だと `prompt` が `# 名前` の下に**空の引用行**を書く。
/// アプリ層（`DisplayName::parse`）だけでなくスキーマ側でも書けなくする。
///
/// **ここは固定リストなのでドリフト検出にはならない**（U+000B / U+000C を落としても
/// このテストは緑のまま通る）。`TRIMMABLE` との一致は
/// `trimmable_set_matches_the_schema_check` が両方向で見ている。
#[tokio::test]
async fn character_display_name_cannot_be_blank() {
    let (_dir, store) = setup().await;

    // 正常系の対照。CHECK が何でも弾いているわけではないことを確かめる。
    sqlx::query("INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)")
        .bind("aoi")
        .bind("葵")
        .bind("ペルソナ本文")
        .execute(store.pool())
        .await
        .expect("正当な表示名は挿入できること");

    for (index, blank) in ["", " ", "   ", "\u{3000}", "\u{00A0}", " \u{3000}\t\r\n"]
        .into_iter()
        .enumerate()
    {
        let err = sqlx::query(
            "INSERT INTO character (handle, display_name, base_persona) \
                               VALUES (?, ?, ?)",
        )
        .bind(format!("handle{index}"))
        .bind(blank)
        .bind("ペルソナ本文")
        .execute(store.pool())
        .await
        .expect_err("空白のみの表示名は CHECK 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("character.display_name = {blank:?}"),
        );
    }

    // **UPDATE でも空にできない。**（CHECK は列に付くので INSERT だけではない）
    let err = sqlx::query("UPDATE character SET display_name = '' WHERE handle = 'aoi'")
        .execute(store.pool())
        .await
        .expect_err("既存行を空の表示名に書き換えられないこと");
    assert_kind(&err, ErrorKind::CheckViolation, "UPDATE display_name = ''");
}

/// **表示名は一意でなくてよい**（ADR-0011 の存在理由）。
///
/// `name` に張った UNIQUE / 正規化キー索引が `display_name` にも波及していたら、
/// 同じ呼び名で別ペルソナの個体を共存させられない。
#[tokio::test]
async fn character_display_name_need_not_be_unique() {
    let (_dir, store) = setup().await;

    for handle in ["aoi", "aoi-v2"] {
        sqlx::query("INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)")
            .bind(handle)
            .bind("葵")
            .bind("ペルソナ本文")
            .execute(store.pool())
            .await
            .expect("同じ表示名でも識別子が違えば挿入できること");
    }

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM character WHERE display_name = '葵'")
        .fetch_one(store.pool())
        .await
        .expect("件数を取得できること");
    assert_eq!(count, 2, "同じ表示名の行が2件並ぶこと");
}

/// プールから取り出したどの接続でも `PRAGMA foreign_keys` が ON になっている。
///
/// SQLite は既定で外部キー制約を無効にするため、接続ごとの有効化を忘れると
/// 制約が黙って効かなくなる。
#[tokio::test]
async fn foreign_keys_pragma_is_on_for_every_connection() {
    let (_dir, store) = setup().await;

    // 同時に保持して別々の物理接続を確実に開かせる。
    let mut conns = Vec::new();
    for _ in 0..3 {
        conns.push(
            store
                .pool()
                .acquire()
                .await
                .expect("プールから接続を取得できること"),
        );
    }

    for conn in conns.iter_mut() {
        let enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&mut **conn)
            .await
            .expect("PRAGMA を読めること");
        assert_eq!(enabled, 1, "接続ごとに外部キー制約が有効であること");
    }
}

/// 存在しない外部キーを参照する INSERT は失敗する（＝ FK が実際に効いている）。
#[tokio::test]
async fn insert_with_unknown_foreign_key_is_rejected() {
    let (_dir, store) = setup().await;

    let err = sqlx::query("INSERT INTO session (character_id, label) VALUES (?, ?)")
        .bind(9_999_i64)
        .bind("存在しないキャラのセッション")
        .execute(store.pool())
        .await
        .expect_err("存在しない character_id は外部キー制約で弾かれること");

    assert_kind(&err, ErrorKind::ForeignKeyViolation, "session.character_id");
}

/// キャラクター識別子に `@` を含められない（ADR-0004: `model` フィールドの予約語と衝突するため）。
#[tokio::test]
async fn character_handle_cannot_contain_at_sign() {
    let (_dir, store) = setup().await;

    // 正常系の対照。CHECK が何でも弾いているわけではないことを確かめる。
    insert_character(&store, "aoi").await;

    for name in ["ao@i", "@aoi", "aoi@", "aoi@new", "a@o@i"] {
        let err = sqlx::query("INSERT INTO character (handle, base_persona) VALUES (?, ?)")
            .bind(name)
            .bind("ペルソナ本文")
            .execute(store.pool())
            .await
            .expect_err("`@` を含むキャラクター識別子は CHECK 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("character.handle = {name}"),
        );
    }
}

/// キャラクター識別子は一意（`model` フィールドからの同定が壊れるため）。
#[tokio::test]
async fn character_handle_must_be_unique() {
    let (_dir, store) = setup().await;
    insert_character(&store, "aoi").await;

    let err = sqlx::query("INSERT INTO character (handle, base_persona) VALUES (?, ?)")
        .bind("aoi")
        .bind("別のペルソナ")
        .execute(store.pool())
        .await
        .expect_err("重複するキャラクター識別子は UNIQUE 制約で弾かれること");

    assert_kind(&err, ErrorKind::UniqueViolation, "character.handle");
}

/// **正規化キーの UNIQUE 索引はもう無い**（`0006` / 作業単位 0004b-2）。
///
/// # このテストは意味が反転している。**それが要点**
///
/// 以前は `character_name_is_unique_after_normalization` という名前で、
/// `"aoi"` と `" aoi "` の**併存を索引が弾くこと**を固定していた（0002 / ADR-0004）。
///
/// **索引の存在理由は「DB を直接編集された場合の担保」だった。** 併存すると
/// アプリ層が `model` を trim して照合した瞬間に両方へ一致し、書き込み先が不定になる——
/// **その前提が ADR-0017 で消えた。** `Handle::parse` が trim をやめたので、
/// 照合は格納値との完全一致だけになり、ゴースト行はどの経路からも引き当てられない。
///
/// **残すと害の方が大きい。** ゴースト行が1件あるだけで正当な識別子 `aoi` の作成が
/// 塞がれ、ADR-0004 が潰したはずの袋小路（一覧に出ない・チャットは 404・
/// しかし作れもしない）そのものになっていた。
///
/// # ここで固定するもの
///
/// * **併存できる**こと（索引が復活していたら弾かれて赤くなる）。
/// * それでも **`handle` の完全一致 UNIQUE は生きている**こと。
///
/// 無害であること（照合が当たらない・一覧が指し示す）は `store` 側の
/// `tests/store_character.rs::a_ghost_row_is_inert_for_every_trimmable_padding` が見る。
#[tokio::test]
async fn rows_that_only_differ_by_padding_may_now_coexist() {
    let (_dir, store) = setup().await;
    insert_character(&store, "aoi").await;

    for name in [
        " aoi ",
        "\u{3000}aoi",
        "aoi\t",
        "\u{00A0}aoi\r\n",
        "\u{000B}\u{000C}aoi ",
        "a oi",
        "葵\u{3000}碧",
    ] {
        insert_character_ignoring_checks(&store, name, "葵", "別のペルソナ", None)
            .await
            .unwrap_or_else(|err| {
                panic!(
                    "正規化キーの索引が残っている（0006 が効いていない）: name = {name:?}: {err}"
                )
            });
    }

    // **完全一致の UNIQUE は生きている。** CHECK を外しても索引は外れない。
    let err = insert_character_ignoring_checks(&store, "aoi", "葵", "別のペルソナ", None)
        .await
        .expect_err("まったく同じ綴りは弾かれること");
    assert_kind(
        &err,
        ErrorKind::UniqueViolation,
        "character.handle の UNIQUE",
    );
}

/// **ゴースト行は素の SQL では書けない**（0005 の CHECK / ADR-0017）。
///
/// 上の `rows_that_only_differ_by_padding_may_now_coexist` が
/// 「索引はもう防がない」ことを固定しているのに対し、こちらは
/// **文字集合の CHECK が同じ行を入口で止める**ことを固定する。
///
/// # **「本番ではもう作れない」ではない**（0004b-1 のレビューで撤回）
///
/// この CHECK が閉じるのは「`Store::pool` を掴んで素の SQL を書く」経路までで、
/// **正規化キーの索引が担っていた脅威モデル——DB の直接編集——は閉じていない。**
/// 実測（2026-08-16 / sqlite 3.53.4）——ゴースト行は1行で作れる:
///
/// ```text
/// sqlite3 ghost.sqlite "PRAGMA ignore_check_constraints = ON; \
///     INSERT INTO character (handle, display_name, base_persona) VALUES (' aoi ', '葵', 'p')"
/// ```
///
/// **0004b-2（正規化の機構を捨てる）の根拠は、この事実の側には置けなかった。**
/// 置いたのは「`Handle::parse` が trim をやめた（ADR-0017）ので、
/// **ゴースト行が存在しても `model` からも完全一致の `find_character_by_handle` からも
/// 照合が当たらない**」という別の事実の方
/// （実測は `tests/common/mod.rs::insert_character_ignoring_checks` の doc が正）。
///
/// # INSERT だけでなく UPDATE も見る
///
/// CHECK は列に付くので**書き換えにも効く**。ここを INSERT だけで見ていると、
/// 「入口は塞いだが、既存行を規則外へ書き換えられる」状態を検出できない
/// （識別子は不変・ADR-0016 §1 なのでアプリ層には UPDATE の経路が無いが、
///  **その不変性を DB 側でも担保していること**がこのテストの守備範囲）。
#[tokio::test]
async fn a_lone_unnormalized_row_can_no_longer_be_written() {
    const UNNORMALIZED: [&str; 8] = [
        " aoi ",
        "aoi ",
        " aoi",
        "\u{3000}aoi",
        "aoi\t",
        "a oi",
        "aoi\u{200B}",
        "Aoi",
    ];

    let (_dir, store) = setup().await;

    for name in UNNORMALIZED {
        let err = sqlx::query("INSERT INTO character (handle, base_persona) VALUES (?, ?)")
            .bind(name)
            .bind("ペルソナ本文")
            .execute(store.pool())
            .await
            .expect_err("正規化されていない識別子は CHECK で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("INSERT character.handle = {name:?}"),
        );
    }

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM character")
        .fetch_one(store.pool())
        .await
        .expect("行数を取得できること");
    assert_eq!(rows, 0, "1件も書けていないこと");

    // --- **既存行を規則外へ書き換えることもできない。** ---
    let id = insert_character(&store, "aoi").await;

    for name in UNNORMALIZED {
        let err = sqlx::query("UPDATE character SET handle = ? WHERE id = ?")
            .bind(name)
            .bind(id)
            .execute(store.pool())
            .await
            .expect_err("正規化されていない識別子への書き換えは CHECK で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("UPDATE character.handle = {name:?}"),
        );
    }

    let stored: String = sqlx::query_scalar("SELECT handle FROM character WHERE id = ?")
        .bind(id)
        .fetch_one(store.pool())
        .await
        .expect("識別子を読めること");
    assert_eq!(stored, "aoi", "1件も書き換わっていないこと");
}

/// `domain::TRIMMABLE` とスキーマ側の除去対象の**ドリフト検出**。
///
/// 写しの台帳は `src/domain/text.rs` の `TRIMMABLE` の doc が正（**0004b-2 で数え直した**）。
/// ここで対にして回すのは、いま効いている写しのうち**2つ**:
///
/// * `TRIMMABLE` と `0005` の `character.handle` の trim CHECK — ただしこれは
///   **文字集合の CHECK（`'*[^a-z0-9_-]*'`）に完全に含まれる**ので、
///   このループが赤くなったときは両方が同時に緩んでいる。
///   識別子の側の本命の見張り番は `handle_alphabet_matches_the_schema_check`。
/// * `TRIMMABLE` と `0005` の `display_name` の CHECK — **広い方向がより痛い。**
///   CHECK にだけ余分な文字（例: `char(8203)`）が入ると、`DisplayName::parse` は
///   通すのに DB が `CheckViolation` で弾く。`server` の `classify_create` は
///   `is_unique_violation()` が偽なので `internal()` に落ち、
///   **利用者には理由の分からない 500 しか返らない。**
///
/// `stat_alloc.axis` の CHECK は `stat_alloc_axis_cannot_be_blank` が見ているが、
/// あちらは `TRIMMABLE` から導出していない固定リストなので、**この検出には入らない**。
///
/// **固定リストで代用しないこと。** `character_display_name_cannot_be_blank` は
/// 固定リストなので、`0004` の CHECK から `char(11)` / `char(12)` を落としても緑のまま
/// 通り抜ける（実測）。**必ず `TRIMMABLE` を回すこと。**
#[tokio::test]
async fn trimmable_set_matches_the_schema_check() {
    let (_dir, store) = setup().await;

    // 順方向 (1): アプリが「空」として拒否する文字は、DB も CHECK で弾く。
    for blank in TRIMMABLE {
        let raw = blank.to_string();
        assert_eq!(
            Handle::parse(&raw),
            Err(HandleError::Empty),
            "アプリ層が空として拒否すること: U+{:04X}",
            blank as u32
        );

        let err = sqlx::query("INSERT INTO character (handle, base_persona) VALUES (?, ?)")
            .bind(&raw)
            .bind("ペルソナ本文")
            .execute(store.pool())
            .await
            .expect_err("スキーマも空白のみとして弾くこと");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("name = U+{:04X}", blank as u32),
        );

        // 同じ文字を `display_name` にも通す（`0004` の CHECK / ADR-0011）。
        // **`handle` は正当な値で埋める。** 埋めないと `character.handle` の CHECK が
        // 先に落ち、`display_name` の CHECK には一度も到達しない。
        assert_eq!(
            DisplayName::parse(&raw),
            Err(DisplayNameError::Empty),
            "アプリ層が空の表示名として拒否すること: U+{:04X}",
            blank as u32
        );

        let err = sqlx::query(
            "INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)",
        )
        .bind(format!("display-{:04X}", blank as u32))
        .bind(&raw)
        .bind("ペルソナ本文")
        .execute(store.pool())
        .await
        .expect_err("スキーマも空白のみの表示名として弾くこと");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("display_name = U+{:04X}", blank as u32),
        );
    }

    // > **順方向 (2)——`0002` の索引との対——はここにあった。**〈0004b-2 で削除〉
    // > 正規化済みの行を1件置いてから `{blank}aoi{blank}` を INSERT し、
    // > UNIQUE 違反になることで索引の `trim` 列挙を評価させていた。
    // > **`0006` が索引を落としたので、この対は存在しない**
    // > （`rows_that_only_differ_by_padding_may_now_coexist` が併存を固定している）。
    // > 識別子の側の写しは `0005` の trim CHECK が1つ残るだけで、
    // > **それは文字集合の CHECK に完全に含まれる**ため単独では受理集合を変えない。

    // 逆方向: **アプリが通す「空白めいた文字」は、DB も通す。**
    // スキーマ側の空白集合だけを広げると、アプリ層を通った表示名が
    // `CheckViolation` で弾かれて**理由の分からない 500** になる。
    //
    // **文字は作業単位 0009a で入れ替えた。** 以前は U+2000 / U+200B / U+FEFF を
    // 使っていたが、`domain::text::is_effectively_blank` が White_Space ∪ Cc ∪ Cf を
    // 見えない側に含めるようになり、**これらだけの値はアプリ層が拒否する**。
    // 「アプリが通すのに DB が弾く」状況を作れないので、検出力がゼロになる。
    //
    // 代わりに使うのは **`is_effectively_blank` が覆えない「見えないが文字」**
    // （`is_effectively_blank` の「覆えないもの」を参照）。
    // ただしこれは3文字の**スポットチェック**であり、網羅ではない。
    //
    // > **文字は作業単位 20b でもう一度入れ替えた。** U+3164 / U+2800 / U+115F は
    // > `MEASURED_INVISIBLE_SPELLINGS` に入り、**アプリ層が拒否するようになった**ので、
    // > 0009a のときとまったく同じ理由（検出力がゼロになる）で使えない。
    // > いまは U+FFA0（HALFWIDTH HANGUL FILLER）/ U+034F（COMBINING GRAPHEME JOINER）/
    // > U+FE00（VARIATION SELECTOR-1）。**測って足すたびにここは材料を1組失う。**
    //
    // > **`handle` の側はもう回せない**（0004b-1）。これらの文字は
    // > `Handle::parse` も 0005 の CHECK も拒否するので、
    // > 「アプリが通すのに DB が弾く」状況を `handle` では作れない。
    // > **識別子の側の両方向のドリフト検出は
    // > `handle_alphabet_matches_the_schema_check` が引き継いでいる。**
    //
    // **`0004` の `display_name` の CHECK だけを広げた場合はここで落ちる。**
    // アプリ層を通った表示名が DB で弾かれて 500 になるので実害が重い。
    for (index, extra) in ['\u{FFA0}', '\u{034F}', '\u{FE00}'].into_iter().enumerate() {
        let raw = extra.to_string();
        assert_eq!(
            Handle::parse(&raw),
            Err(HandleError::Charset),
            "識別子としては 0004b-1 以降は拒否されること: U+{:04X}",
            extra as u32
        );

        // **`handle` には衝突しない正当な値**を入れて、
        // 落ちるとしたら `display_name` の CHECK だけになるようにする。
        assert!(
            DisplayName::parse(&raw).is_ok(),
            "アプリ層は除去対象外の空白を表示名として通すこと: U+{:04X}",
            extra as u32
        );

        sqlx::query("INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)")
            .bind(format!("extra-display-{index}"))
            .bind(&raw)
            .bind("ペルソナ本文")
            .execute(store.pool())
            .await
            .unwrap_or_else(|err| {
                panic!(
                    "スキーマも表示名として通すこと: U+{:04X}: {err}",
                    extra as u32
                );
            });
    }
}

/// **`Handle::ALLOWED_CHARACTERS` と `0005` の CHECK のドリフト検出**
/// （ADR-0017 / 作業単位 0004b-1）。
///
/// 写しは3箇所ある（`Handle::ALLOWED_CHARACTERS` の doc）。**ここが突き合わせるのはそのうちの2つ** ——
/// 画面の説明文（3つ目）は `web/tests/character-form-examples.test.ts` が見る。
/// **Rust 側は 38 文字の列挙、SQL 側は範囲記法**（`'*[^a-z0-9_-]*'`）で、
/// 綴りをわざと変えてある。ここで**両方向**を機械的に突き合わせる。
///
/// * **アプリが通す値を DB も通す** —— 片側だけ狭いと、`create_character` が
///   `CheckViolation` で落ちる。`server::classify_create` は UNIQUE 違反ではないので
///   `internal()` に落ち、
///   利用者には**理由の分からない 500** しか返らない（`display_name` のときと同じ形）。
/// * **アプリが弾く値を DB も弾く** —— 片側だけ広いと、DB の直接編集で
///   `" aoi "` や `"Aoi"` の行が復活する。ADR-0017 が消したはずの欠陥クラスが戻る。
///
/// # 固定リストにしない
///
/// **代表を並べると「大文字だけ通す」「`.` だけ通す」といった漏れを検出できない。**
/// 全 ASCII（128 点）に加え、`TRIMMABLE` / `LINE_BREAKS` /
/// 非 ASCII の見本を機械的に回す（domain 側の
/// `every_character_outside_the_alphabet_is_rejected` は全スカラー値を回すが、
/// **DB へ 110 万件 INSERT はできない**のでここは代表点まで）。
///
/// # 埋め込み NUL を必ず入れること
///
/// SQLite の `length()` / `GLOB` は**テキストを最初の NUL までしか見ない。**
/// `"aoi\u{0000}AOI"` は GLOB の否定クラスを素通りするので、
/// CHECK 側に `length(cast(name AS BLOB)) = length(name)` が無いと
/// **アプリが弾く値を DB が受け入れる**（実測）。
#[tokio::test]
async fn handle_alphabet_matches_the_schema_check() {
    let (_dir, store) = setup().await;

    // 回す文字。**ASCII 全域を必ず含める**（`-` / `_` / 大文字 / 記号がここに入る）。
    let mut probes: Vec<char> = (0_u32..128).filter_map(char::from_u32).collect::<Vec<_>>();
    probes.extend(TRIMMABLE);
    probes.extend(LINE_BREAKS);
    probes.extend([
        '\u{00A0}',
        '\u{200B}',
        '\u{FEFF}',
        '\u{3000}',
        '\u{3164}',
        '\u{2800}',
        '\u{115F}',
        '葵',
        '🐈',
        'Ａ',
        'ｏ',
        '\u{20BB7}',
        '\u{0661}',
        'é',
        'Ω',
    ]);

    let mut accepted_by_both = 0_u32;
    for (index, probe) in probes.iter().enumerate() {
        // **正当な文字に挟む。** 単独だと、長さ1の識別子しか見ない実装を通す。
        let raw = format!("a{probe}b{index}");
        let accepted_by_the_app = Handle::parse(&raw).is_ok();

        let result = sqlx::query("INSERT INTO character (handle, base_persona) VALUES (?, ?)")
            .bind(&raw)
            .bind("ペルソナ本文")
            .execute(store.pool())
            .await;

        match (accepted_by_the_app, result) {
            (true, Ok(_)) => accepted_by_both += 1,
            (false, Err(err)) => assert_kind(
                &err,
                ErrorKind::CheckViolation,
                &format!("U+{:04X} は CHECK で弾くこと", *probe as u32),
            ),
            (true, Err(err)) => panic!(
                "アプリが通した識別子を DB が弾いた（CHECK が狭い / 理由不明の 500 になる）: \
                 U+{:04X}: {err}",
                *probe as u32
            ),
            (false, Ok(_)) => panic!(
                "アプリが弾いた識別子を DB が受け入れた（CHECK が広い / ゴースト行が復活する）: \
                 U+{:04X}",
                *probe as u32
            ),
        }
    }

    assert_eq!(
        accepted_by_both, 38,
        "両側が受理する文字はちょうど 38 種であること（`ALLOWED_CHARACTERS` と同数）"
    );

    // **埋め込み NUL。** SQLite の `length()` / `GLOB` は最初の NUL までしか見ないので、
    // CHECK にバイト長の照合が無いとここが素通りする。
    for raw in ["aoi\u{0000}AOI", "aoi\u{0000} aoi ", "\u{0000}Aoi"] {
        assert!(
            Handle::parse(raw).is_err(),
            "アプリは NUL 入りの識別子を拒否すること: raw = {raw:?}"
        );
        let err = sqlx::query("INSERT INTO character (handle, base_persona) VALUES (?, ?)")
            .bind(raw)
            .bind("ペルソナ本文")
            .execute(store.pool())
            .await
            .expect_err("DB も NUL 入りの識別子を拒否すること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("character.handle = {raw:?}（NUL の先に隠した文字を見ていない）"),
        );
    }
}

/// `memory.subject` は `user` / `character` のみ。NULL も不可。
#[tokio::test]
async fn memory_subject_is_restricted_to_known_values() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    for subject in ["user", "character"] {
        sqlx::query("INSERT INTO memory (character_id, subject, content) VALUES (?, ?, ?)")
            .bind(character_id)
            .bind(subject)
            .bind("猫が好き")
            .execute(store.pool())
            .await
            .expect("既知の subject は挿入できること");
    }

    for subject in ["User", "system", "", "assistant"] {
        let err =
            sqlx::query("INSERT INTO memory (character_id, subject, content) VALUES (?, ?, ?)")
                .bind(character_id)
                .bind(subject)
                .bind("猫が好き")
                .execute(store.pool())
                .await
                .expect_err("未知の subject は CHECK 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("subject = {subject}"),
        );
    }

    let err = sqlx::query("INSERT INTO memory (character_id, subject, content) VALUES (?, ?, ?)")
        .bind(character_id)
        .bind(Option::<String>::None)
        .bind("猫が好き")
        .execute(store.pool())
        .await
        .expect_err("subject の省略は NOT NULL で弾かれること");

    assert_kind(&err, ErrorKind::NotNullViolation, "memory.subject = NULL");
}

/// `message.role` は `user` / `assistant` のみ。
#[tokio::test]
async fn message_role_is_restricted_to_known_values() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_id = insert_session(&store, character_id).await;

    for role in ["user", "assistant"] {
        sqlx::query("INSERT INTO message (session_id, role, raw_content) VALUES (?, ?, ?)")
            .bind(session_id)
            .bind(role)
            .bind("生出力")
            .execute(store.pool())
            .await
            .expect("既知の role は挿入できること");
    }

    for role in ["system", "tool", "Assistant", ""] {
        let err =
            sqlx::query("INSERT INTO message (session_id, role, raw_content) VALUES (?, ?, ?)")
                .bind(session_id)
                .bind(role)
                .bind("生出力")
                .execute(store.pool())
                .await
                .expect_err("未知の role は CHECK 制約で弾かれること");

        assert_kind(&err, ErrorKind::CheckViolation, &format!("role = {role}"));
    }
}

/// `message.raw_content` は NULL 不可（パース失敗時の唯一の調査手段）。
#[tokio::test]
async fn message_raw_content_cannot_be_null() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_id = insert_session(&store, character_id).await;

    let err = sqlx::query(
        "INSERT INTO message (session_id, role, dialogue, narration, raw_content) \
         VALUES (?, 'assistant', ?, ?, ?)",
    )
    .bind(session_id)
    .bind("こんにちは")
    .bind("彼女は微笑んだ")
    .bind(Option::<String>::None)
    .execute(store.pool())
    .await
    .expect_err("raw_content の NULL は NOT NULL 制約で弾かれること");

    assert_kind(&err, ErrorKind::NotNullViolation, "message.raw_content");
}

/// `generated_image` の再現情報のうち、1つだけを NULL にした INSERT のケース。
struct NullCase {
    /// NULL にした列名（アサートのメッセージ用）。
    column: &'static str,
    positive: Option<&'static str>,
    negative: Option<&'static str>,
    seed: Option<i64>,
}

/// `generated_image` の再現情報（プロンプト・シード）は NULL 不可。
#[tokio::test]
async fn generated_image_reproduction_columns_cannot_be_null() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_id = insert_session(&store, character_id).await;
    let message_id = insert_message(&store, session_id).await;

    const INSERT: &str = "INSERT INTO generated_image \
         (session_id, message_id, positive_prompt, negative_prompt, seed, checkpoint, file_path) \
         VALUES (?, ?, ?, ?, ?, ?, ?)";

    // 正常系の対照。
    sqlx::query(INSERT)
        .bind(session_id)
        .bind(message_id)
        .bind("1girl, smile")
        .bind("worst quality")
        .bind(42_i64)
        .bind("some-checkpoint.safetensors")
        .bind("images/0001.png")
        .execute(store.pool())
        .await
        .expect("再現情報が揃っていれば挿入できること");

    let cases = [
        NullCase {
            column: "positive_prompt",
            positive: None,
            negative: Some("worst quality"),
            seed: Some(42),
        },
        NullCase {
            column: "negative_prompt",
            positive: Some("1girl, smile"),
            negative: None,
            seed: Some(42),
        },
        NullCase {
            column: "seed",
            positive: Some("1girl, smile"),
            negative: Some("worst quality"),
            seed: None,
        },
    ];

    for NullCase {
        column,
        positive,
        negative,
        seed,
    } in cases
    {
        let err = sqlx::query(INSERT)
            .bind(session_id)
            .bind(message_id)
            .bind(positive)
            .bind(negative)
            .bind(seed)
            .bind("some-checkpoint.safetensors")
            .bind("images/0002.png")
            .execute(store.pool())
            .await
            .expect_err("再現情報の NULL は NOT NULL 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::NotNullViolation,
            &format!("generated_image.{column}"),
        );
    }
}

/// `memory.superseded_by` に有効な id を入れられる（フェーズ2で使う列が機能する）。
/// 併せて、存在しない id は外部キー制約で弾かれることも確認する。
#[tokio::test]
async fn memory_superseded_by_links_to_another_memory() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    let old_id = insert_memory(&store, character_id, "コーヒーが好き").await;
    let new_id = insert_memory(&store, character_id, "紅茶が好き").await;

    // 新規行を追加し、古い行に「後継」を指させる（古い行は消さない）。
    sqlx::query("UPDATE memory SET superseded_by = ? WHERE id = ?")
        .bind(new_id)
        .bind(old_id)
        .execute(store.pool())
        .await
        .expect("superseded_by を設定できること");

    let row = sqlx::query("SELECT content, superseded_by, locked FROM memory WHERE id = ?")
        .bind(old_id)
        .fetch_one(store.pool())
        .await
        .expect("古い行が残っていること");

    assert_eq!(
        row.get::<String, _>("content"),
        "コーヒーが好き",
        "古い記憶の本文が保持されていること"
    );
    assert_eq!(
        row.get::<Option<i64>, _>("superseded_by"),
        Some(new_id),
        "superseded_by が新しい行を指すこと"
    );
    assert_eq!(
        row.get::<i64, _>("locked"),
        0,
        "locked の既定値は 0 であること"
    );

    let err = sqlx::query("UPDATE memory SET superseded_by = ? WHERE id = ?")
        .bind(9_999_i64)
        .bind(old_id)
        .execute(store.pool())
        .await
        .expect_err("存在しない id への superseded_by は外部キー制約で弾かれること");

    assert_kind(&err, ErrorKind::ForeignKeyViolation, "memory.superseded_by");
}

/// `stat_alloc` は (character_id, axis) が主キー。軸名は任意の文字列。
#[tokio::test]
async fn stat_alloc_rejects_duplicate_axis_per_character() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    for axis in ["パッション", "ロジック", "任意に増やせる軸"] {
        sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
            .bind(character_id)
            .bind(axis)
            .bind(3_i64)
            .execute(store.pool())
            .await
            .expect("任意の軸名を登録できること");
    }

    let err = sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(character_id)
        .bind("パッション")
        .bind(5_i64)
        .execute(store.pool())
        .await
        .expect_err("同一キャラの同一軸は主キー制約で弾かれること");

    assert_kind(&err, ErrorKind::UniqueViolation, "stat_alloc の主キー");
}

/// 同じ DB へ再接続してもマイグレーションが冪等に適用され、データが残る。
#[tokio::test]
async fn reconnecting_reapplies_migrations_idempotently() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("personacore.sqlite");

    {
        let store = Store::connect(&path).await.expect("初回接続が成功すること");
        insert_character(&store, "aoi").await;
        store.close().await;
    }

    let store = Store::connect(&path).await.expect("再接続が成功すること");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM character")
        .fetch_one(store.pool())
        .await
        .expect("再接続後もテーブルを参照できること");

    assert_eq!(count, 1, "既存データが保持されていること");
}

/// 開けないパスへの接続は `StoreError::Connect` になり、
/// メッセージに内部パスを漏らさない（エラーレスポンスに載りうるため）。
#[tokio::test]
async fn connect_to_unopenable_path_reports_connect_error_without_leaking_path() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir
        .path()
        .join("存在しない下位ディレクトリ")
        .join("db.sqlite");

    let err = Store::connect(&path)
        .await
        .expect_err("存在しないディレクトリ配下には作成できないこと");

    assert!(
        matches!(err, StoreError::Connect(_)),
        "接続エラーとして型付けされること: {err:?}"
    );
    assert!(
        !err.to_string().contains("存在しない下位ディレクトリ"),
        "エラーメッセージに内部パスを含めないこと: {err}"
    );
}

/// SQLite の副次ファイル（`-wal` / `-shm`）のパス。
///
/// **接続が開いている間だけ存在し、最後の接続が閉じるときに片付けられる。**
/// つまりプールを閉じたかどうかを外から観測できる。
fn side_files(db_path: &Path) -> [PathBuf; 2] {
    let base = db_path.display().to_string();
    [
        PathBuf::from(format!("{base}-wal")),
        PathBuf::from(format!("{base}-shm")),
    ]
}

/// **マイグレーションに失敗したら、開いたプールを閉じてから `Err` を返す。**
///
/// `Store::connect` はプールを開いてから `migrate` を走らせるので、`?` で素通しすると
/// プールが宙に浮く。失われるユーザデータは無い（適用に失敗した DB なので）が、
/// **`-wal` / `-shm` とファイルロックがプロセス終了まで残る。**
/// 呼び出し側は `Store` を受け取れないため、**構造的にここでしか閉じられない。**
///
/// マイグレーション失敗は `_sqlx_migrations` のチェックサムを壊して作る。
/// **マイグレーション SQL は編集しない**（チェックサムが変わり、他の DB が開けなくなる）。
#[tokio::test]
async fn failed_migration_closes_the_pool() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("tampered.sqlite");

    {
        let store = Store::connect(&path).await.expect("初回接続が成功すること");
        // 適用済みの記録を書き換える。SQL ファイル側は触らない。
        let done = sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = 1")
            .bind(vec![0_u8; 4])
            .execute(store.pool())
            .await
            .expect("チェックサムを書き換えられること");
        assert_eq!(done.rows_affected(), 1, "前提: 版 1 の記録が存在すること");
        store.close().await;
    }

    let err = Store::connect(&path)
        .await
        .expect_err("チェックサムが合わない DB は開けないこと");
    assert!(
        matches!(err, StoreError::Migrate(_)),
        "マイグレーションの失敗として型付けされること: {err:?}"
    );
    assert!(
        !err.to_string().contains("tampered"),
        "エラーメッセージに内部パスを含めないこと: {err}"
    );

    for side in side_files(&path) {
        assert!(
            !side.exists(),
            "マイグレーション失敗でもプールを閉じること（{} が残っている）",
            side.display()
        );
    }
}

/// **新しい版で開いた DB を、古い exe の形で開くと、それと分かる型で止まる**（B-55）。
///
/// 「古い exe の形」は、`_sqlx_migrations` に**その exe が知らない版**を1行足した DB。
/// （適用済みなのに、解決できた一覧に無い = sqlx の `VersionMissing`）
///
/// **止まるだけでなく、DB に1バイトも書かない**ことまで見る——帳簿の全行の写しが
/// 開く前後で同じであること。`Display` は定数の日本語で、版の番号を含まない。
#[tokio::test]
async fn a_database_opened_by_a_newer_version_stops_with_a_typed_error() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("newer.sqlite");

    {
        let store = Store::connect(&path).await.expect("初回接続が成功すること");
        sqlx::query(
            "INSERT INTO _sqlx_migrations \
             (version, description, success, checksum, execution_time) \
             VALUES (?, 'from a newer version', 1, ?, 0)",
        )
        .bind(987_654_i64)
        .bind(vec![7_u8; 48])
        .execute(store.pool())
        .await
        .expect("未知の版を1行足せること");
        store.close().await;
    }

    let before = ledger_snapshot(&path).await;
    assert!(
        before.iter().any(|row| row.0 == 987_654),
        "前提: 未知の版が帳簿にあること"
    );

    let err = Store::connect(&path)
        .await
        .expect_err("知らない版を持つ DB は開けないこと");
    assert!(
        matches!(err, StoreError::DatabaseFromNewerVersion(_)),
        "それと分かる型で止まること: {err:?}"
    );
    let shown = err.to_string();
    assert!(
        shown.contains("新しい版") && shown.contains("写しておいた保存先に戻して"),
        "利用者に向けた日本語であること: {shown}"
    );
    assert!(
        !shown.contains("987654") && !shown.contains("newer.sqlite"),
        "版の番号やパスは Display に載せない: {shown}"
    );
    assert!(
        matches!(
            std::error::Error::source(&err)
                .and_then(|s| s.downcast_ref::<sqlx::migrate::MigrateError>()),
            Some(sqlx::migrate::MigrateError::VersionMissing(987_654))
        ),
        "詳細は source 側に VersionMissing(987654) として残ること"
    );

    assert_eq!(
        ledger_snapshot(&path).await,
        before,
        "止まるとき、帳簿に1行も書かないこと"
    );
    for side in side_files(&path) {
        assert!(!side.exists(), "プールを閉じること（{}）", side.display());
    }
}

/// **対比: 編集された SQL（チェックサム不一致）は、これまでどおり `Migrate`。**
///
/// 「何でも `DatabaseFromNewerVersion` にする」実装を通さない。
/// （`failed_migration_closes_the_pool` も同じことを `Migrate` で見ているが、
/// ここでは「新しい版」の文言が出ないことまで見る。）
#[tokio::test]
async fn a_tampered_checksum_is_not_reported_as_a_newer_version() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("tampered2.sqlite");
    {
        let store = Store::connect(&path).await.expect("初回接続が成功すること");
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = 1")
            .bind(vec![0_u8; 4])
            .execute(store.pool())
            .await
            .expect("チェックサムを書き換えられること");
        store.close().await;
    }
    let err = Store::connect(&path).await.expect_err("開けないこと");
    assert!(matches!(err, StoreError::Migrate(_)), "{err:?}");
    assert!(!err.to_string().contains("新しい版"), "{err}");
}

/// 帳簿の全行（版 / 説明 / 成否 / チェックサム）の写し。**書かずに開く**ための別プール。
async fn ledger_snapshot(path: &Path) -> Vec<(i64, String, bool, Vec<u8>)> {
    let pool = SqlitePool::connect_with(SqliteConnectOptions::new().filename(path))
        .await
        .expect("読み用に開けること");
    let rows = sqlx::query(
        "SELECT version, description, success, checksum FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .expect("帳簿を読めること");
    pool.close().await;
    rows.iter()
        .map(|row| {
            (
                row.get::<i64, _>("version"),
                row.get::<String, _>("description"),
                row.get::<bool, _>("success"),
                row.get::<Vec<u8>, _>("checksum"),
            )
        })
        .collect()
}

/// プールから取り出したどの接続でも `journal_mode` が WAL になっている。
///
/// sqlx 0.9 は `journal_mode` を明示しない限り設定しない（既定の `delete` のまま）。
/// `delete` では読みトランザクション中の書き込みが `database is locked` で落ちるが、
/// メモリ抽出は本文レスポンス返却後に非同期で走る（= 読みと書きが常時同時に走る）ため、
/// WAL でないと通常動作で衝突する。
#[tokio::test]
async fn journal_mode_is_wal_for_every_connection() {
    let (_dir, store) = setup().await;

    let mut conns = Vec::new();
    for _ in 0..3 {
        conns.push(
            store
                .pool()
                .acquire()
                .await
                .expect("プールから接続を取得できること"),
        );
    }

    for conn in conns.iter_mut() {
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&mut **conn)
            .await
            .expect("PRAGMA を読めること");
        assert_eq!(
            mode.to_ascii_lowercase(),
            "wal",
            "接続ごとに journal_mode が WAL であること"
        );
    }
}

/// `memory.superseded_by` は**同一キャラクターの記憶しか指せない**。
///
/// 別キャラの記憶を指せると、フェーズ2の遡行がキャラをまたいで記憶を混線させる
/// （ADR-0004 が「最も避けたい壊れ方」とする状態）。
#[tokio::test]
async fn memory_superseded_by_cannot_point_to_another_character() {
    let (_dir, store) = setup().await;
    let aoi = insert_character(&store, "aoi").await;
    let rin = insert_character(&store, "rin").await;

    let aoi_old = insert_memory(&store, aoi, "コーヒーが好き").await;
    let aoi_new = insert_memory(&store, aoi, "紅茶が好き").await;
    let rin_memory = insert_memory(&store, rin, "犬が好き").await;

    // 正常系の対照: 同一キャラ内なら指せる。
    sqlx::query("UPDATE memory SET superseded_by = ? WHERE id = ?")
        .bind(aoi_new)
        .bind(aoi_old)
        .execute(store.pool())
        .await
        .expect("同一キャラの記憶は指せること");

    let err = sqlx::query("UPDATE memory SET superseded_by = ? WHERE id = ?")
        .bind(rin_memory)
        .bind(aoi_old)
        .execute(store.pool())
        .await
        .expect_err("別キャラの記憶を指す superseded_by は複合外部キーで弾かれること");

    assert_kind(
        &err,
        ErrorKind::ForeignKeyViolation,
        "memory.superseded_by が別キャラを指す",
    );
}

/// `memory.superseded_by` は自分自身を指せない（遡行が無限ループになるため）。
#[tokio::test]
async fn memory_superseded_by_cannot_be_self() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let memory_id = insert_memory(&store, character_id, "コーヒーが好き").await;

    let err = sqlx::query("UPDATE memory SET superseded_by = ? WHERE id = ?")
        .bind(memory_id)
        .bind(memory_id)
        .execute(store.pool())
        .await
        .expect_err("自己参照の superseded_by は CHECK 制約で弾かれること");

    assert_kind(&err, ErrorKind::CheckViolation, "memory.superseded_by = id");

    // INSERT 時に明示指定した場合も同様に弾かれる。
    let err = sqlx::query(
        "INSERT INTO memory (id, character_id, subject, content, superseded_by) \
         VALUES (?, ?, 'user', ?, ?)",
    )
    .bind(4_242_i64)
    .bind(character_id)
    .bind("紅茶が好き")
    .bind(4_242_i64)
    .execute(store.pool())
    .await
    .expect_err("INSERT 時の自己参照も CHECK 制約で弾かれること");

    assert_kind(
        &err,
        ErrorKind::CheckViolation,
        "INSERT 時の memory.superseded_by = id",
    );
}

/// `summary.upto_message_id` は**同一セッションのメッセージしか指せない**。
#[tokio::test]
async fn summary_upto_message_must_belong_to_same_session() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_one = insert_session(&store, character_id).await;
    let session_two = insert_session(&store, character_id).await;

    let message_in_one = insert_message(&store, session_one).await;
    let message_in_two = insert_message(&store, session_two).await;

    // 正常系の対照。
    insert_summary(&store, session_one, message_in_one)
        .await
        .expect("同一セッションのメッセージなら要約を作れること");
    insert_summary(&store, session_two, message_in_two)
        .await
        .expect("同一セッションのメッセージなら要約を作れること");

    let err = insert_summary(&store, session_two, message_in_one)
        .await
        .expect_err("別セッションのメッセージを指す要約は複合外部キーで弾かれること");

    assert_kind(
        &err,
        ErrorKind::ForeignKeyViolation,
        "summary.upto_message_id が別セッションを指す",
    );
}

/// `generated_image.message_id` は**同一セッションのメッセージしか指せない**。
/// `message_id` が NULL のときは（MATCH SIMPLE により）制約が満たされる。
#[tokio::test]
async fn generated_image_message_must_belong_to_same_session() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_one = insert_session(&store, character_id).await;
    let session_two = insert_session(&store, character_id).await;

    let message_in_one = insert_message(&store, session_one).await;

    // 正常系の対照: 同一セッション、および message_id なし。
    insert_generated_image(&store, session_one, Some(message_in_one))
        .await
        .expect("同一セッションのメッセージには紐付けられること");
    insert_generated_image(&store, session_two, None)
        .await
        .expect("メッセージに紐付かない生成も記録できること");

    let err = insert_generated_image(&store, session_two, Some(message_in_one))
        .await
        .expect_err("別セッションのメッセージを指す画像は複合外部キーで弾かれること");

    assert_kind(
        &err,
        ErrorKind::ForeignKeyViolation,
        "generated_image.message_id が別セッションを指す",
    );
}

/// キャラクター識別子は空文字・空白のみを許さない。全角空白・TAB・NBSP なども空白として扱う。
///
/// ADR-0004 では `model` フィールドをキャラクター識別子の許可リストと照合するため、
/// 識別子が `''` のキャラが1件でもあると `model` が空のリクエストが 404 にならず、
/// そのキャラに書き込み始める。
/// 半角スペースだけを除く `trim()` では、日本語環境で現実に混入する
/// **全角空白のみの識別子**が素通りしてしまう。
#[tokio::test]
async fn character_handle_cannot_be_blank() {
    let (_dir, store) = setup().await;

    // 正常系の対照。**この CHECK が何でも弾いているわけではない**ことを確かめる。
    //
    // > 0004b-1 より前は「空白を**含む**識別子は通ること」も併せて固定していたが、
    // > `character.handle` の文字集合の CHECK（0005 / ADR-0017）が**空白を含む識別子ごと
    // > 弾く**ようになったので、その対照はもう置けない。
    // > **この CHECK 単体が空白のみだけを弾くこと**は
    // > `trimmable_set_matches_the_schema_check` が `display_name` 側で見ている。
    insert_character(&store, "aoi").await;
    insert_character(&store, "aoi-v2_7").await;

    for name in BLANK_STRINGS {
        let err = sqlx::query("INSERT INTO character (handle, base_persona) VALUES (?, ?)")
            .bind(name)
            .bind("ペルソナ本文")
            .execute(store.pool())
            .await
            .expect_err("空白のみのキャラクター識別子は CHECK 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("character.handle = {name:?}"),
        );
    }
}

/// `stat_alloc.axis` は空文字・空白のみを許さない。
///
/// 軸名が空のまま通ると、プロンプトに**軸名のない「: 3/5」**が載る。
#[tokio::test]
async fn stat_alloc_axis_cannot_be_blank() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    // 正常系の対照。空白を含む軸名は通ること。
    for axis in ["パッション", "内なる\u{3000}情熱"] {
        sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
            .bind(character_id)
            .bind(axis)
            .bind(3_i64)
            .execute(store.pool())
            .await
            .expect("空白でない軸名は挿入できること");
    }

    for axis in BLANK_STRINGS {
        let err =
            sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
                .bind(character_id)
                .bind(axis)
                .bind(3_i64)
                .execute(store.pool())
                .await
                .expect_err("空白のみの軸名は CHECK 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("stat_alloc.axis = {axis:?}"),
        );
    }
}

/// `stat_alloc.value` は**整数以外の型**を受け付けない。
///
/// SQLite の型アフィニティでは、INTEGER 宣言の列に `'abc'` を入れても
/// 数値に変換できないため TEXT のまま格納される。さらに比較規則上
/// TEXT > INTEGER なので `CHECK (value >= 0)` は真になり、**そのまま通ってしまう**。
/// 小数（REAL）も同様に素通りする。`typeof` で型そのものを縛る。
#[tokio::test]
async fn stat_alloc_value_must_be_integer_typed() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    // 正常系の対照。
    for (axis, value) in [("ゼロ軸", 0_i64), ("パッション", 5)] {
        sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
            .bind(character_id)
            .bind(axis)
            .bind(value)
            .execute(store.pool())
            .await
            .expect("整数は挿入できること");
    }

    let stored: String =
        sqlx::query_scalar("SELECT typeof(value) FROM stat_alloc WHERE axis = 'パッション'")
            .fetch_one(store.pool())
            .await
            .expect("typeof を取得できること");
    assert_eq!(stored, "integer", "格納された値が整数型であること");

    // 文字列。アフィニティで数値化できず TEXT のまま入る。
    for value in ["abc", "３", ""] {
        let err =
            sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
                .bind(character_id)
                .bind("ロジック")
                .bind(value)
                .execute(store.pool())
                .await
                .expect_err("文字列の性格値は CHECK 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("stat_alloc.value = {value:?}"),
        );
    }

    // 小数。REAL のまま入る。
    for value in [2.5_f64, -0.5] {
        let err =
            sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
                .bind(character_id)
                .bind("ロジック")
                .bind(value)
                .execute(store.pool())
                .await
                .expect_err("小数の性格値は CHECK 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("stat_alloc.value = {value}"),
        );
    }

    // 負値の拒否（既存の `value >= 0`）が typeof 追加後も生きていること。
    let err = sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(character_id)
        .bind("ロジック")
        .bind(-3_i64)
        .execute(store.pool())
        .await
        .expect_err("負の性格値は CHECK 制約で弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "stat_alloc.value = -3");
}

/// `generated_image.seed` は**整数以外の型**を受け付けない。
///
/// `seed` は「再現性のために必ず保持する」絶対制約の列であり、壊れたことに気づくのは
/// **読み出し時＝再現したくなった瞬間**。NOT NULL だけでは型アフィニティにより
/// `'abc'` が TEXT のまま通る。
#[tokio::test]
async fn generated_image_seed_must_be_integer_typed() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_id = insert_session(&store, character_id).await;

    const INSERT: &str = "INSERT INTO generated_image \
         (session_id, positive_prompt, negative_prompt, seed, checkpoint, file_path) \
         VALUES (?, ?, ?, ?, ?, ?)";

    // 正常系の対照。負のシードや大きな値も整数なら通る。
    for seed in [0_i64, 42, -1, i64::MAX] {
        sqlx::query(INSERT)
            .bind(session_id)
            .bind("1girl, smile")
            .bind("worst quality")
            .bind(seed)
            .bind("some-checkpoint.safetensors")
            .bind("images/0001.png")
            .execute(store.pool())
            .await
            .expect("整数のシードは挿入できること");
    }

    let stored: i64 =
        sqlx::query_scalar("SELECT seed FROM generated_image ORDER BY id DESC LIMIT 1")
            .fetch_one(store.pool())
            .await
            .expect("シードを取得できること");
    assert_eq!(stored, i64::MAX, "大きなシードが欠落せず往復すること");

    for seed in ["abc", "42abc", ""] {
        let err = sqlx::query(INSERT)
            .bind(session_id)
            .bind("1girl, smile")
            .bind("worst quality")
            .bind(seed)
            .bind("some-checkpoint.safetensors")
            .bind("images/0002.png")
            .execute(store.pool())
            .await
            .expect_err("文字列のシードは CHECK 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("generated_image.seed = {seed:?}"),
        );
    }

    for seed in [1.5_f64, -0.25] {
        let err = sqlx::query(INSERT)
            .bind(session_id)
            .bind("1girl, smile")
            .bind("worst quality")
            .bind(seed)
            .bind("some-checkpoint.safetensors")
            .bind("images/0003.png")
            .execute(store.pool())
            .await
            .expect_err("小数のシードは CHECK 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("generated_image.seed = {seed}"),
        );
    }
}

/// `stat_alloc.value` は負値を許さない。
///
/// 「性格値0の軸はプロンプトに含めない」を `value != 0` で実装したとき、
/// 負値が残っていると「パッション: -3/5」がそのままプロンプトに載る。
#[tokio::test]
async fn stat_alloc_value_cannot_be_negative() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    // 正常系の対照。0 は「軸ごと落とす」対象として有効な値なので通す。
    for (axis, value) in [("ゼロ軸", 0_i64), ("パッション", 5)] {
        sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
            .bind(character_id)
            .bind(axis)
            .bind(value)
            .execute(store.pool())
            .await
            .expect("0 以上の値は挿入できること");
    }

    for value in [-1_i64, -3] {
        let err =
            sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
                .bind(character_id)
                .bind("ロジック")
                .bind(value)
                .execute(store.pool())
                .await
                .expect_err("負の性格値は CHECK 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("stat_alloc.value = {value}"),
        );
    }
}

/// `memory.locked` は 0 / 1 のみ。
///
/// `WHERE locked = 1` で保護対象を選ぶ実装では、`locked = 7` の行が
/// 静かに保護対象から漏れる。
#[tokio::test]
async fn memory_locked_is_restricted_to_zero_or_one() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    for locked in [0_i64, 1] {
        sqlx::query(
            "INSERT INTO memory (character_id, subject, content, locked) VALUES (?, 'user', ?, ?)",
        )
        .bind(character_id)
        .bind("猫が好き")
        .bind(locked)
        .execute(store.pool())
        .await
        .expect("0 / 1 は挿入できること");
    }

    for locked in [-1_i64, 2, 7] {
        let err = sqlx::query(
            "INSERT INTO memory (character_id, subject, content, locked) \
             VALUES (?, 'user', ?, ?)",
        )
        .bind(character_id)
        .bind("猫が好き")
        .bind(locked)
        .execute(store.pool())
        .await
        .expect_err("0 / 1 以外の locked は CHECK 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("memory.locked = {locked}"),
        );
    }
}

/// **DB は「上書き済みの古い行」を保護しない**ことを明示するテスト。
///
/// `superseded_by` の外部キーが防ぐのは**後継行**（参照先）の消失だけで、
/// 保持したい**古い行**（参照元）は素通しで消える。
/// `DELETE FROM memory WHERE superseded_by IS NOT NULL` は「もう上書き済みだから
/// 消してよい行」を選ぶ最も自然なクエリでありながら、保持したい行だけを正確に消す。
/// 「DB が止めてくれる」と誤解しないよう、実際の挙動をここで固定する。
#[tokio::test]
async fn database_does_not_protect_superseded_rows_from_deletion() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    let old_id = insert_memory(&store, character_id, "コーヒーが好き").await;
    let new_id = insert_memory(&store, character_id, "紅茶が好き").await;

    sqlx::query("UPDATE memory SET superseded_by = ? WHERE id = ?")
        .bind(new_id)
        .bind(old_id)
        .execute(store.pool())
        .await
        .expect("superseded_by を設定できること");

    // 後継行（参照先）の削除は外部キーで弾かれる。守れるのはこちらだけ。
    let err = sqlx::query("DELETE FROM memory WHERE id = ?")
        .bind(new_id)
        .execute(store.pool())
        .await
        .expect_err("参照されている後継行は外部キー制約で弾かれること");
    assert_kind(
        &err,
        ErrorKind::ForeignKeyViolation,
        "後継行の削除は防がれる",
    );

    // 一方で古い行は消せてしまう。**これはアプリ層の責務**。
    let done = sqlx::query("DELETE FROM memory WHERE superseded_by IS NOT NULL")
        .execute(store.pool())
        .await
        .expect("古い行の削除は DB では防げないこと（アプリ層の責務）");

    assert_eq!(
        done.rows_affected(),
        1,
        "「上書き済みの行」を選ぶクエリが、保持したい古い行だけを消してしまうこと"
    );

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory WHERE id = ?")
        .bind(old_id)
        .fetch_one(store.pool())
        .await
        .expect("行数を取得できること");
    assert_eq!(remaining, 0, "古い行は DB 側では保護されないこと");
}

/// `message.dialogue` / `message.narration` は NULL 可。
///
/// これは**パース失敗時に生出力だけを保存するフォールバックの受け皿**。
/// ここに NOT NULL が付くと、最も落ちてほしくない経路だけが実行時に落ちる。
#[tokio::test]
async fn message_dialogue_and_narration_may_be_null() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_id = insert_session(&store, character_id).await;

    sqlx::query(
        "INSERT INTO message (session_id, role, dialogue, narration, raw_content) \
         VALUES (?, 'assistant', NULL, NULL, ?)",
    )
    .bind(session_id)
    .bind("パースできなかった生出力")
    .execute(store.pool())
    .await
    .expect("パース失敗時は dialogue / narration が NULL でも保存できること");

    let row = sqlx::query(
        "SELECT dialogue, narration, raw_content FROM message WHERE session_id = ? ORDER BY id",
    )
    .bind(session_id)
    .fetch_one(store.pool())
    .await
    .expect("保存した行を取得できること");

    assert_eq!(row.get::<Option<String>, _>("dialogue"), None);
    assert_eq!(row.get::<Option<String>, _>("narration"), None);
    assert_eq!(
        row.get::<String, _>("raw_content"),
        "パースできなかった生出力",
        "生出力は必ず保持されること"
    );
}

/// `generated_image.checkpoint` / `file_path` は NULL 不可。
///
/// チェックポイントが違えば同じ seed でも再現しないため、`checkpoint` は `seed` と同格。
#[tokio::test]
async fn generated_image_checkpoint_and_file_path_cannot_be_null() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_id = insert_session(&store, character_id).await;

    const INSERT: &str = "INSERT INTO generated_image \
         (session_id, positive_prompt, negative_prompt, seed, checkpoint, file_path) \
         VALUES (?, ?, ?, ?, ?, ?)";

    for (column, checkpoint, file_path) in [
        ("checkpoint", None, Some("images/0001.png")),
        ("file_path", Some("some-checkpoint.safetensors"), None),
    ] {
        let err = sqlx::query(INSERT)
            .bind(session_id)
            .bind("1girl, smile")
            .bind("worst quality")
            .bind(42_i64)
            .bind(checkpoint)
            .bind(file_path)
            .execute(store.pool())
            .await
            .expect_err("再現に必要な列の NULL は NOT NULL 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::NotNullViolation,
            &format!("generated_image.{column}"),
        );
    }
}

/// `summary` の必須列のうち、1つだけを NULL にした INSERT のケース。
struct SummaryNullCase {
    /// NULL にした列名（アサートのメッセージ用）。
    column: &'static str,
    session: Option<i64>,
    message: Option<i64>,
    content: Option<&'static str>,
}

/// `summary` の列定義（NOT NULL と参照）を固定する。
#[tokio::test]
async fn summary_requires_session_message_and_content() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_id = insert_session(&store, character_id).await;
    let message_id = insert_message(&store, session_id).await;

    let summary_id = insert_summary(&store, session_id, message_id)
        .await
        .expect("正当な要約は挿入できること");

    let row = sqlx::query(
        "SELECT session_id, upto_message_id, content, created_at FROM summary WHERE id = ?",
    )
    .bind(summary_id)
    .fetch_one(store.pool())
    .await
    .expect("保存した要約を取得できること");

    assert_eq!(row.get::<i64, _>("session_id"), session_id);
    assert_eq!(row.get::<i64, _>("upto_message_id"), message_id);
    assert_eq!(row.get::<String, _>("content"), "要約本文");
    assert!(
        !row.get::<String, _>("created_at").is_empty(),
        "created_at に既定値が入ること"
    );

    const INSERT: &str =
        "INSERT INTO summary (session_id, upto_message_id, content) VALUES (?, ?, ?)";

    let cases = [
        SummaryNullCase {
            column: "session_id",
            session: None,
            message: Some(message_id),
            content: Some("要約本文"),
        },
        SummaryNullCase {
            column: "upto_message_id",
            session: Some(session_id),
            message: None,
            content: Some("要約本文"),
        },
        SummaryNullCase {
            column: "content",
            session: Some(session_id),
            message: Some(message_id),
            content: None,
        },
    ];

    for SummaryNullCase {
        column,
        session,
        message,
        content,
    } in cases
    {
        let err = sqlx::query(INSERT)
            .bind(session)
            .bind(message)
            .bind(content)
            .execute(store.pool())
            .await
            .expect_err("要約の必須列の NULL は NOT NULL 制約で弾かれること");

        assert_kind(
            &err,
            ErrorKind::NotNullViolation,
            &format!("summary.{column}"),
        );
    }
}

/// `character_appearance` はキャラクターにつき1行。`lora_*` は NULL 可。
#[tokio::test]
async fn character_appearance_is_one_row_per_character() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    sqlx::query(
        "INSERT INTO character_appearance (character_id, fixed_tags, lora_name, lora_strength) \
         VALUES (?, ?, NULL, NULL)",
    )
    .bind(character_id)
    .bind("blue hair, long hair, green eyes")
    .execute(store.pool())
    .await
    .expect("LoRA 未設定の外見定義を登録できること");

    let row = sqlx::query(
        "SELECT fixed_tags, lora_name, lora_strength FROM character_appearance \
         WHERE character_id = ?",
    )
    .bind(character_id)
    .fetch_one(store.pool())
    .await
    .expect("保存した外見定義を取得できること");

    assert_eq!(
        row.get::<String, _>("fixed_tags"),
        "blue hair, long hair, green eyes"
    );
    assert_eq!(row.get::<Option<String>, _>("lora_name"), None);
    assert_eq!(row.get::<Option<f64>, _>("lora_strength"), None);

    let err =
        sqlx::query("INSERT INTO character_appearance (character_id, fixed_tags) VALUES (?, ?)")
            .bind(character_id)
            .bind("別の固定タグ")
            .execute(store.pool())
            .await
            .expect_err("キャラクターにつき2行目は主キー制約で弾かれること");
    assert_kind(
        &err,
        ErrorKind::UniqueViolation,
        "character_appearance.character_id",
    );

    let err =
        sqlx::query("INSERT INTO character_appearance (character_id, fixed_tags) VALUES (?, ?)")
            .bind(insert_character(&store, "rin").await)
            .bind(Option::<String>::None)
            .execute(store.pool())
            .await
            .expect_err("fixed_tags の NULL は NOT NULL 制約で弾かれること");
    assert_kind(
        &err,
        ErrorKind::NotNullViolation,
        "character_appearance.fixed_tags",
    );
}

/// キャラクターを削除すると、依存する行がすべて CASCADE で消える。
///
/// CASCADE が落ちても正常系のテストでは気づけないため、明示的に固定する。
#[tokio::test]
async fn deleting_character_cascades_to_all_dependent_rows() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_id = insert_session(&store, character_id).await;
    let message_id = insert_message(&store, session_id).await;

    sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, 'パッション', 3)")
        .bind(character_id)
        .execute(store.pool())
        .await
        .expect("性格値を登録できること");
    sqlx::query(
        "INSERT INTO character_attribute (character_id, subject, key, value) \n         VALUES (?, 'character', '一人称', '僕')",
    )
    .bind(character_id)
    .execute(store.pool())
    .await
    .expect("属性を登録できること");
    sqlx::query("INSERT INTO character_appearance (character_id, fixed_tags) VALUES (?, ?)")
        .bind(character_id)
        .bind("blue hair")
        .execute(store.pool())
        .await
        .expect("外見定義を登録できること");
    let memory_id = insert_memory(&store, character_id, "猫が好き").await;
    // **取り決めとその履歴も CASCADE の対象**（ADR-0051 §決定1 / ADR-0049）。
    // 出どころの会話も指しておく（会話と同じ文のカスケードで消えるとき、外部キーが止めないこと）。
    let agreement_id = sqlx::query(
        "INSERT INTO character_agreement (character_id, content, kind, keeper, state, source_session_id) \
         VALUES (?, '毎朝あいさつする', 'rule', 'both', 'active', ?)",
    )
    .bind(character_id)
    .bind(session_id)
    .execute(store.pool())
    .await
    .expect("取り決めを登録できること")
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO agreement_transition (agreement_id, before, after) VALUES (?, NULL, 'active')",
    )
    .bind(agreement_id)
    .execute(store.pool())
    .await
    .expect("取り決めの履歴を登録できること");
    // **軸の候補の3枚も CASCADE の対象**（ADR-0046 §決定6 / 作業単位 A-9a）。追記のみだが、
    // 孤児を作らないために親の削除には従う。**行を入れてから消す**（空のままだと 0 件は自明）。
    let candidate_id = sqlx::query(
        "INSERT INTO stat_axis_candidate (character_id, name, source_memory_id) VALUES (?, ?, ?)",
    )
    .bind(character_id)
    .bind("好奇心")
    .bind(memory_id)
    .execute(store.pool())
    .await
    .expect("候補を登録できること")
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO stat_axis_candidate_transition (candidate_id, state) VALUES (?, 'trial')",
    )
    .bind(candidate_id)
    .execute(store.pool())
    .await
    .expect("移り変わりを登録できること");
    sqlx::query(
        "INSERT INTO stat_axis_trial_vote (candidate_id, session_id, vote) VALUES (?, ?, 'zero')",
    )
    .bind(candidate_id)
    .bind(session_id)
    .execute(store.pool())
    .await
    .expect("試用の票を登録できること");
    // **票も CASCADE の対象**（ADR-0026 §決定3）。追記のみだが、
    // 孤児を作らないために**親の削除には従う**。
    sqlx::query(
        "INSERT INTO stat_drift_vote (character_id, session_id, axis, delta) \
         VALUES (?, ?, 'パッション', 'up')",
    )
    .bind(character_id)
    .bind(session_id)
    .execute(store.pool())
    .await
    .expect("票を登録できること");
    insert_summary(&store, session_id, message_id)
        .await
        .expect("要約を登録できること");
    insert_generated_image(&store, session_id, Some(message_id))
        .await
        .expect("画像記録を登録できること");

    sqlx::query("DELETE FROM character WHERE id = ?")
        .bind(character_id)
        .execute(store.pool())
        .await
        .expect("キャラクターを削除できること");

    for (table, sql) in COUNT_QUERIES {
        assert_eq!(
            count_rows(&store, sql).await,
            0,
            "{table} がキャラクター削除で CASCADE されること"
        );
    }
}

/// 上書き関係のある記憶を持つキャラクターでも、CASCADE 削除できる。
///
/// `superseded_by` の外部キーは削除時アクションを持たないため、
/// CASCADE と衝突しないことを確認しておく。
#[tokio::test]
async fn deleting_character_cascades_even_with_superseded_memories() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    let old_id = insert_memory(&store, character_id, "コーヒーが好き").await;
    let new_id = insert_memory(&store, character_id, "紅茶が好き").await;
    sqlx::query("UPDATE memory SET superseded_by = ? WHERE id = ?")
        .bind(new_id)
        .bind(old_id)
        .execute(store.pool())
        .await
        .expect("superseded_by を設定できること");

    sqlx::query("DELETE FROM character WHERE id = ?")
        .bind(character_id)
        .execute(store.pool())
        .await
        .expect("上書き関係があってもキャラクターを削除できること");

    assert_eq!(
        count_rows(&store, "SELECT COUNT(*) FROM memory").await,
        0,
        "記憶が残らないこと"
    );
}

/// `character.stat_scale` は **`1`〜`100` の整数だけ**を受け付ける（ADR-0015 / 作業単位 0011a-1）。
///
/// # 「拒否される値」と「受理される値」の両方を見る
///
/// `stat_alloc.value` の CHECK と**同じ罠**がある。SQLite の型アフィニティにより、
/// INTEGER 宣言の列に入れた `'3'` や `3.0` は**整数 3 に変換されてから** CHECK を通る
/// ので、`typeof` を縛っても**受理される**。拒否できるのは変換できない値
/// （`'abc'` / `'3abc'` / `'３'` / `2.5`）だけ。
///
/// **片側だけを見ると「文字列を一律に弾いている」と誤読され、
/// アプリ層の検証（`domain::StatScale`）を省いてよい根拠にされる。** 両方を並べる。
#[tokio::test]
async fn character_stat_scale_only_accepts_integers_inside_the_bounds() {
    let (_dir, store) = setup().await;

    const INSERT: &str = "INSERT INTO character (handle, display_name, base_persona, stat_scale) VALUES (?, ?, ?, ?)";

    // --- 範囲外の整数は拒否する ---
    for value in [0_i64, 101, -1, i64::MAX] {
        let err = sqlx::query(INSERT)
            .bind(format!("out-{}", value.unsigned_abs()))
            .bind("葵")
            .bind("ペルソナ本文")
            .bind(value)
            .execute(store.pool())
            .await
            .expect_err("範囲外の尺度は CHECK 制約で弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("character.stat_scale = {value}"),
        );
    }

    // --- 整数に変換できない値は拒否する ---
    for value in ["abc", "3abc", "３", ""] {
        let err = sqlx::query(INSERT)
            .bind("text-scale")
            .bind("葵")
            .bind("ペルソナ本文")
            .bind(value)
            .execute(store.pool())
            .await
            .expect_err("整数でない尺度は CHECK 制約で弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("character.stat_scale = {value:?}"),
        );
    }

    // --- 小数は拒否する（REAL のまま入る） ---
    for value in [2.5_f64, 0.5, 100.5] {
        let err = sqlx::query(INSERT)
            .bind("real-scale")
            .bind("葵")
            .bind("ペルソナ本文")
            .bind(value)
            .execute(store.pool())
            .await
            .expect_err("小数の尺度は CHECK 制約で弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("character.stat_scale = {value}"),
        );
    }

    // --- 両端は受理する ---
    for value in [1_i64, 7, 100] {
        sqlx::query(INSERT)
            .bind(format!("ok-{value}"))
            .bind("葵")
            .bind("ペルソナ本文")
            .bind(value)
            .execute(store.pool())
            .await
            .unwrap_or_else(|err| panic!("範囲内の尺度は挿入できること: {value}: {err}"));
    }

    // --- **アフィニティ変換で通る値**（拒否できていない側。ここが罠） ---
    sqlx::query(INSERT)
        .bind("affinity-text")
        .bind("葵")
        .bind("ペルソナ本文")
        .bind("3")
        .execute(store.pool())
        .await
        .expect("`'3'` はアフィニティ変換で整数になってから CHECK を通る（拒否できない）");
    sqlx::query(INSERT)
        .bind("affinity-real")
        .bind("葵")
        .bind("ペルソナ本文")
        .bind(3.0_f64)
        .execute(store.pool())
        .await
        .expect("`3.0` も同様に整数へ変換されてから通る（拒否できない）");

    let types: Vec<String> = sqlx::query_scalar(
        "SELECT typeof(stat_scale) FROM character WHERE handle IN ('affinity-text', 'affinity-real')",
    )
    .fetch_all(store.pool())
    .await
    .expect("typeof を取得できること");
    assert_eq!(
        types,
        vec!["integer".to_owned(), "integer".to_owned()],
        "アフィニティ変換された値は整数として格納されていること \
         （＝この CHECK は「文字列を一律に弾く」制約ではない）"
    );

    // --- 既定値は指定しない INSERT に配られる ---
    sqlx::query("INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)")
        .bind("no-scale")
        .bind("葵")
        .bind("ペルソナ本文")
        .execute(store.pool())
        .await
        .expect("尺度を指定しない INSERT も通ること");
    let default_scale: i64 =
        sqlx::query_scalar("SELECT stat_scale FROM character WHERE handle = 'no-scale'")
            .fetch_one(store.pool())
            .await
            .expect("尺度を読めること");
    assert_eq!(default_scale, 10, "スキーマの既定値");
}

/// **既存行のある DB に `0008` を実際に当てて、尺度が埋まることを確かめる**
/// （`migrations/0008_character_stat_scale.sql` / ADR-0015 / 作業単位 0011a-1）。
///
/// # なぜ「行のある DB」で当てる必要があるのか
///
/// **`ALTER TABLE ADD COLUMN` の CHECK は、既存行がある場合にだけ落ちうる**
/// （`migrations/0004_character_display_name.sql` の実測）。
/// **空の開発用 DB では通り、行のある本番 DB でだけ落ちる**——しかも落ちた DB は
/// `Store::connect` ごと起動不能になる。`setup()` は空の DB を作るので、
/// あちらでは**この壊れ方を1度も踏まない。**
///
/// # 尺度を引き上げる UPDATE が本体
///
/// **落としても正常系は1つも赤くならない。** 気付くのは 0011a-2 で
/// 値を尺度に従わせた瞬間で、**11 以上の値を持つ既存キャラクターが丸ごと読めなくなる。**
///
/// 版 7 までを部分適用してから `Store::connect` に残りを流す手口は
/// `migration_preserves_existing_rows_and_their_ids` と同じ
/// （`Migrator::new(&Path)` は公開 API で、チェックサムはファイル内容から計算される）。
#[tokio::test]
async fn the_migration_lifts_existing_rows_to_the_largest_value_they_actually_use() {
    // (識別子, その行が持つ性格値, 移行後に期待する尺度)
    //
    // # **閾値そのものを縛るための行が混ざっている**
    //
    // WHERE 句の `value > 10` の右辺を閾値 `t` と呼ぶ。ある行の最大値を `m` とすると、
    // `m > t` なら `min(100, m)` に、そうでなければ既定の 10 のまま残る。
    // 下の一覧は **`t` を 9..=10 まで絞り込む**ように選んである:
    //
    // * `f-nine`（最大 9 → 期待 10）が `t >= 9` を要求する
    //   （`t <= 8` だと 9 まで引き上げられてしまう）
    // * `g-eleven`（最大 11 → 期待 11）が `t <= 10` を要求する
    //   （`t >= 11` だと既定の 10 に据え置かれてしまう）
    //
    // **`t = 9` と `t = 10` は区別できない（等価）。** 差が出るのは最大値が
    // ちょうど 10 の行だけで、`t = 9` なら更新されて `min(100, 10) = 10`、
    // `t = 10` なら更新されずに既定の 10。**どちらも同じ値になる。**
    // したがって**ここまでしか絞れない**——この一覧は閾値を「一意に固定」してはいない。
    const CASES: [(&str, &[i64], i64); 7] = [
        // 10 以下しか使っていない → 既定のまま。
        ("a-small", &[5, 3], 10),
        // 11 以上を使っている → その最大値まで引き上げる。
        ("b-mid", &[42, 7], 42),
        // **上限で頭打ち。この行は救えていない**（value = 150 は 0011a-2 で読めなくなる）。
        // マイグレーションの ★ 印のコメントが言っているのがこの行。
        ("c-huge", &[150, 20], 100),
        // 軸が1件も無い → 相関サブクエリの NULL に触れない（EXISTS が偽）。
        ("d-none", &[], 10),
        // ちょうど 10 → `value > 10` に当たらないので既定のまま。
        ("e-exact", &[10], 10),
        // **閾値の下側の見張り番。** 閾値を 8 以下へ下げると 9 に引き上がって赤になる。
        ("f-nine", &[9, 2], 10),
        // **閾値の上側の見張り番。** 閾値を 11 以上へ上げると 10 のまま残って赤になる。
        // ここが緑を通ると、最大値 11〜20 の既存キャラクターだけが既定のまま取り残される。
        ("g-eleven", &[11, 6], 11),
    ];

    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("legacy.sqlite");
    let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");

    // --- 版 7 までを適用した「尺度の無い DB」を作り、行を入れる ---
    {
        let pool = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true)
                .foreign_keys(true),
        )
        .await
        .expect("旧スキーマ用のプールを開けること");

        let subset_dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let mut copied: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&migrations).expect("migrations ディレクトリを読めること")
        {
            let entry = entry.expect("マイグレーションのエントリを読めること");
            let name = entry.file_name().to_string_lossy().into_owned();
            // **版 7 までだけを写す。** 版が増えてもここは「7 まで」を指し続ける。
            //
            // > **かつては `!name.starts_with("0008_")` で書いてあった**（作業単位 12a で直した）。
            // > `0009` を足した瞬間にそれが素通りし、下の `copied.len() == 7` で赤くなった。
            // > **前提の確認を置いていたから気付けた**——置いていなければ
            // > 「尺度の無い DB」と言い張りながら 0009 まで当たった DB で測っていた。
            let version: usize = name
                .split('_')
                .next()
                .and_then(|prefix| prefix.parse().ok())
                .unwrap_or_else(|| panic!("版番号で始まるファイル名であること: {name}"));
            if version <= 7 {
                std::fs::copy(entry.path(), subset_dir.path().join(&name))
                    .expect("マイグレーションをコピーできること");
                copied.push(name);
            }
        }
        // **前提の確認を省かない。** 抽出が壊れたときに黙って全適用になるのを防ぐ。
        assert_eq!(
            copied.len(),
            7,
            "前提: 版 7 までが7件であること: {copied:?}"
        );

        let migrator = Migrator::new(subset_dir.path())
            .await
            .expect("マイグレーションを読めること");
        migrator
            .run(&pool)
            .await
            .expect("版 7 までを適用できること");

        // **この時点で stat_scale 列はまだ無い。** 無いことを確かめてから行を入れる
        // （既に在ったら、後段の「移行が埋めた」という主張が成り立たない）。
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('character')")
                .fetch_all(&pool)
                .await
                .expect("列一覧を取得できること");
        assert!(
            !columns.contains(&"stat_scale".to_owned()),
            "前提: 版 7 の時点では尺度の列が無いこと: {columns:?}"
        );

        for (handle, values, _) in CASES {
            let character_id = sqlx::query(
                "INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)",
            )
            .bind(handle)
            .bind(format!("呼び名: {handle}"))
            .bind("ペルソナ本文")
            .execute(&pool)
            .await
            .expect("移行前のキャラクターを入れられること")
            .last_insert_rowid();
            for (index, value) in values.iter().enumerate() {
                sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
                    .bind(character_id)
                    .bind(format!("軸{index}"))
                    .bind(value)
                    .execute(&pool)
                    .await
                    .expect("移行前の性格値を入れられること");
            }
        }

        pool.close().await;
    }

    // --- 残り（`0008`）を本番の経路で当てる ---
    let store = Store::connect(&path)
        .await
        .expect("既存行のある DB にも 0008 を適用できること（ADD COLUMN が CHECK で落ちないこと）");

    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT handle, stat_scale FROM character ORDER BY handle")
            .fetch_all(store.pool())
            .await
            .expect("尺度を読めること");

    let expected: Vec<(String, i64)> = CASES
        .iter()
        .map(|(handle, _, scale)| ((*handle).to_owned(), *scale))
        .collect();
    assert_eq!(rows, expected, "移行が既存行の尺度を埋めること");
}

// ============================================================================
// character_attribute（ADR-0009 / 作業単位 12a）
// ============================================================================

/// `character_attribute` に1行入れる（CHECK を通る素の SQL）。
async fn insert_attribute(
    store: &Store,
    character_id: i64,
    subject: &str,
    key: &str,
    value: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO character_attribute (character_id, subject, key, value) VALUES (?, ?, ?, ?)",
    )
    .bind(character_id)
    .bind(subject)
    .bind(key)
    .bind(value)
    .execute(store.pool())
    .await
    .map(|_| ())
}

/// `character_attribute.subject` は `user` / `character` のみ。
///
/// **`memory.subject` と同じ2値**（ADR-0009。綴りの写し元は
/// `src/store/memory.rs` の `subject_to_db` ひとつ）。落とすと
/// 「年齢: 19」がキャラのものか利用者のものかが DB の中で曖昧になる。
#[tokio::test]
async fn attribute_subject_is_restricted_to_known_values() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    // **受理される値**（両方。片方だけだと「常に character を通す」実装と区別が付かない）。
    for (subject, key) in [("user", "呼び方"), ("character", "一人称")] {
        insert_attribute(&store, character_id, subject, key, "僕")
            .await
            .unwrap_or_else(|err| panic!("既知の subject は挿入できること: {subject}: {err}"));
    }

    // **拒否される値。** `assistant` は `message.role` の綴りで、取り違えの実例。
    for subject in ["User", "Character", "assistant", "system", ""] {
        let err = insert_attribute(&store, character_id, subject, "種族", "猫獣人")
            .await
            .expect_err("未知の subject は CHECK 制約で弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("subject = {subject:?}"),
        );
    }

    let err = sqlx::query(
        "INSERT INTO character_attribute (character_id, subject, key, value) \
         VALUES (?, ?, '種族', '猫獣人')",
    )
    .bind(character_id)
    .bind(Option::<String>::None)
    .execute(store.pool())
    .await
    .expect_err("subject の省略は NOT NULL で弾かれること");
    assert_kind(
        &err,
        ErrorKind::NotNullViolation,
        "character_attribute.subject = NULL",
    );
}

/// `character_attribute` のキー・値は**空白のみでは書けない**。
///
/// 空のキーはプロンプトに**キーのない「: 猫獣人」**を載せ、空の値は
/// `種族: ` を載せてモデルに欠落を演技させる（ADR-0009）。
/// 空白の扱いは `stat_alloc.axis` と同じ（半角スペースだけでは足りない理由もそちら）。
#[tokio::test]
async fn attribute_key_and_value_cannot_be_blank() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    // 正常系の対照。**中間の空白を含む値は通る**（潰すと「好きな 食べ物」が書けない）。
    insert_attribute(
        &store,
        character_id,
        "character",
        "好きな 食べ物",
        "焼き 芋",
    )
    .await
    .expect("空白でないキー・値は挿入できること");

    for blank in BLANK_STRINGS {
        let err = insert_attribute(&store, character_id, "character", blank, "猫獣人")
            .await
            .expect_err("空白のみのキーは CHECK 制約で弾かれること");
        assert_kind(&err, ErrorKind::CheckViolation, &format!("key = {blank:?}"));

        let err = insert_attribute(&store, character_id, "character", "種族", blank)
            .await
            .expect_err("空白のみの値は CHECK 制約で弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("value = {blank:?}"),
        );
    }
}

/// **`TRIMMABLE` の全文字について、アプリが「空」として拒否する値は DB も弾く。**
///
/// `trimmable_set_matches_the_schema_check` の `character_attribute` 版。
/// スキーマ側の空白集合だけを狭めると、アプリ層を通らない値が DB に入る。
/// 逆にスキーマ側だけを広げると、アプリを通った値が
/// `CheckViolation` で弾かれて**理由の分からない 500** になる。
///
/// # **`key` と `value` を両方見る**（作業単位 12a のレビュー指摘で広げた）
///
/// `character_attribute` の CHECK 式は**キー側と値側で丸ごと写し**になっている。
/// 片側しか見ていなかったときの実測（2026-08-20 / `cargo test --no-fail-fast` /
/// 全 949 件）:
///
/// | `migrations/0009` への変異 | このテスト | スイート全体 |
/// |---|---|---|
/// | `key` 側の trim 集合から `char(160)` を落とす | **赤** | 947 passed / 2 failed |
/// | `value` 側の trim 集合から `char(160)` を落とす | 緑 | 948 passed / 1 failed |
/// | `value` 側の trim 集合に `char(12644)` を足す | **赤** | 948 passed / 1 failed |
/// | `key` 側の trim 集合に `char(12644)` を足す | 緑 | **949 passed / 0 failed** |
///
/// つまり**順方向はキー側だけ、逆方向は値側だけ**しか見ていなかった。
///
/// * 順方向の値側は、固定リストの `attribute_key_and_value_cannot_be_blank` が
///   拾っていた（上表2行目の 1 failed はそれ）。**ただしそれは偶然で**、
///   `BLANK_STRINGS` は `TRIMMABLE` から導出されていない別の配列なので、
///   **`TRIMMABLE` に文字を1つ足せばその被覆は黙って消える。**
/// * 逆方向のキー側は**どの見張り番も見ていなかった**（上表4行目）。
///   スポットチェックが key を `{raw}{index}` にしていたため、
///   trim 集合を広げても末尾の数字が残って CHECK を通ってしまう。
///
/// **どちらの向きも、どちらの列も、ここで見る。**
/// 台帳（`src/domain/text.rs` の `TRIMMABLE`）の 5 / 6 はこの形を指している。
#[tokio::test]
async fn attribute_blank_set_matches_the_schema_check() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    for blank in TRIMMABLE {
        let raw = blank.to_string();
        assert!(
            AttributeKey::parse(&raw).is_err(),
            "アプリ層が空のキーとして拒否すること: U+{:04X}",
            blank as u32
        );
        assert!(
            AttributeValue::parse(&raw).is_err(),
            "アプリ層が空の値として拒否すること: U+{:04X}",
            blank as u32
        );

        // **キー側。** 値には正当な語を置く（キーと値を同値にすると取り違えが隠れる）。
        let err = insert_attribute(&store, character_id, "character", &raw, "猫獣人")
            .await
            .expect_err("スキーマも空白のみのキーとして弾くこと");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("key = U+{:04X}", blank as u32),
        );

        // **値側。** キーには正当な語を置く。
        let err = insert_attribute(&store, character_id, "character", "種族", &raw)
            .await
            .expect_err("スキーマも空白のみの値として弾くこと");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("value = U+{:04X}", blank as u32),
        );
    }

    // 逆方向: **アプリが通す「空白めいた文字」は DB も通す。**
    // `is_effectively_blank` が覆えない「見えないが文字」でスポットチェックする
    // （`trimmable_set_matches_the_schema_check` と同じ3文字）。
    //
    // **キー側と値側を別の行に分ける。** 1行にまとめると、その文字を含まない方の列に
    // 正当な語が入ってしまい、片側だけを広げる変異が隠れる（上表4行目が実測）。
    for (index, extra) in ['\u{FFA0}', '\u{034F}', '\u{FE00}'].into_iter().enumerate() {
        let raw = extra.to_string();
        assert!(
            AttributeKey::parse(&raw).is_ok(),
            "アプリ層は除去対象外の空白をキーとして通すこと: U+{:04X}",
            extra as u32
        );
        assert!(
            AttributeValue::parse(&raw).is_ok(),
            "アプリ層は除去対象外の空白を値として通すこと: U+{:04X}",
            extra as u32
        );

        // **値側**（キーは正当な語）。
        insert_attribute(
            &store,
            character_id,
            "character",
            &format!("キー{index}"),
            &raw,
        )
        .await
        .unwrap_or_else(|err| {
            panic!("スキーマも値として通すこと: U+{:04X}: {err}", extra as u32);
        });

        // **キー側**（値は正当な語）。
        insert_attribute(
            &store,
            character_id,
            "character",
            &raw,
            &format!("値{index}"),
        )
        .await
        .unwrap_or_else(|err| {
            panic!(
                "スキーマもキーとして通すこと: U+{:04X}: {err}",
                extra as u32
            );
        });
    }
}

/// **スキーマ側の文字数 CHECK が、`domain` の上限と揃っている。**
///
/// 2つは**独立した定数**（適用済みのマイグレーションは編集できないので、
/// SQL 側の 12 / 20 は今後もこの値のまま）。片方だけ動かすと:
///
/// * アプリ側だけ緩める → 通った値が `CheckViolation` になり**理由の分からない 500**
/// * スキーマ側だけ緩い → 直接 INSERT で読めない行が作れる（読み出しの検証は残るので
///   壊れ方は軽いが、CHECK を置いた意味が消える）
///
/// **受理される値と拒否される値の両方**を見る（境界の片側だけだと、
/// CHECK を丸ごと落とす変異は「拒否される値」でしか捕まらない）。
#[tokio::test]
async fn attribute_limits_match_the_schema_check() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    // --- キー: 上限ちょうどは通り、1文字超えたら通らない ---
    let key_at_limit = "あ".repeat(AttributeKey::MAX_CHARS);
    assert!(AttributeKey::parse(&key_at_limit).is_ok(), "前提");
    insert_attribute(&store, character_id, "character", &key_at_limit, "猫獣人")
        .await
        .expect("上限ちょうどのキーは DB も通すこと");

    let key_over = "あ".repeat(AttributeKey::MAX_CHARS + 1);
    assert!(AttributeKey::parse(&key_over).is_err(), "前提");
    let err = insert_attribute(&store, character_id, "character", &key_over, "猫獣人")
        .await
        .expect_err("上限を超えるキーは CHECK 制約で弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "key が上限超過");

    // --- 値: 同じく両側 ---
    let value_at_limit = "い".repeat(AttributeValue::MAX_CHARS);
    assert!(AttributeValue::parse(&value_at_limit).is_ok(), "前提");
    insert_attribute(&store, character_id, "character", "種族", &value_at_limit)
        .await
        .expect("上限ちょうどの値は DB も通すこと");

    let value_over = "い".repeat(AttributeValue::MAX_CHARS + 1);
    assert!(AttributeValue::parse(&value_over).is_err(), "前提");
    let err = insert_attribute(&store, character_id, "character", "出身", &value_over)
        .await
        .expect_err("上限を超える値は CHECK 制約で弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "value が上限超過");

    // --- **キーと値で上限が違う**（片方の定数をもう片方に使う変異を落とす） ---
    let between = "う".repeat(AttributeKey::MAX_CHARS + 1);
    // **「キーの方が短い」を定数同士で比較しない**（clippy が
    // `assertions_on_constants` で止める）。見張っているのは `domain` 側の
    // `the_two_limits_are_not_the_same_number`（振る舞いとして両者を分ける）。
    insert_attribute(&store, character_id, "character", "一人称", &between)
        .await
        .expect("キーの上限を超える長さでも、値としては通ること");
    let err = insert_attribute(&store, character_id, "character", &between, "僕")
        .await
        .expect_err("同じ長さがキーとしては弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "キーと値の上限は別");

    // --- 文字数で数える（バイト数ではない） ---
    let japanese = "獣".repeat(AttributeValue::MAX_CHARS);
    assert!(japanese.len() > AttributeValue::MAX_CHARS, "前提");
    insert_attribute(&store, character_id, "character", "口癖", &japanese)
        .await
        .expect("日本語をバイト数で弾かないこと");
}

/// **TEXT 列でも型を縛る**（`stat_alloc.value` の型アフィニティの罠の TEXT 版）。
///
/// BLOB は TEXT アフィニティで変換されないので、`typeof` を縛らないと
/// `length()` がバイト数を返す行が作れる（読み出しは `ColumnDecode` で落ち、
/// 原因が SQL 側に見える）。整数は `'19'` へ変換されてから通る——
/// **これは意図した受理**で、`19` は属性値として正当。
#[tokio::test]
async fn attribute_key_and_value_must_be_text_typed() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    // **受理される値**: 整数はアフィニティ変換で TEXT になる。
    sqlx::query(
        "INSERT INTO character_attribute (character_id, subject, key, value) \
         VALUES (?, 'character', '年齢', 19)",
    )
    .bind(character_id)
    .execute(store.pool())
    .await
    .expect("整数は TEXT へ変換されてから通ること");

    let stored: (String, String) =
        sqlx::query_as("SELECT typeof(value), value FROM character_attribute WHERE key = '年齢'")
            .fetch_one(store.pool())
            .await
            .expect("読めること");
    assert_eq!(stored, ("text".to_owned(), "19".to_owned()));

    // **拒否される値**: BLOB は TEXT アフィニティで変換されない。
    //
    // # BLOB は **NUL を含まないもの**で試すこと（作業単位 12a の変異検査で判明）
    //
    // 最初は `x'00ff'` で書いていたが、**`typeof` を丸ごと落としても全テストが緑だった**
    // （948/948）。`x'00ff'` は NUL を含むので `instr(key, char(0)) = 0` に、
    // また `length(trim(x'00ff', ' '))` が 0 になるので空白の CHECK にも掛かり、
    // **`typeof` に到達する前に 2 つの別の節が弾いていた。**
    // 実測（2026-08-20 / sqlite3 3.53.4）——他の節だけを残すと `x'414243'` は**通る**:
    //
    // ```text
    // typeof(x'414243') = blob / instr(…, char(0)) = 0 / length(trim(…, ' ')) = 3
    // ```
    //
    // **`typeof` は死んだ検査ではない。テストの穴だった。**
    // **BLOB も必ずプレースホルダで渡す**（`Vec<u8>` を bind すると BLOB になる）。
    // `x'…'` リテラルを `format!` で組み立てると、テストの側だけ
    // 「SQL を文字列連結で作る」形になる（CLAUDE.md 絶対制約）。
    for (label, blob) in [
        ("NUL を含まない", vec![0x41_u8, 0x42, 0x43]),
        ("NUL を含む", vec![0x00_u8, 0xFF]),
    ] {
        let err = sqlx::query(
            "INSERT INTO character_attribute (character_id, subject, key, value) \
             VALUES (?, 'character', ?, ?)",
        )
        .bind(character_id)
        .bind(format!("blob-{label}"))
        .bind(blob.clone())
        .execute(store.pool())
        .await
        .expect_err("BLOB の値は弾かれること");
        assert_kind(&err, ErrorKind::CheckViolation, &format!("value = {label}"));

        let err = sqlx::query(
            "INSERT INTO character_attribute (character_id, subject, key, value) \
             VALUES (?, 'character', ?, '猫獣人')",
        )
        .bind(character_id)
        .bind(blob)
        .execute(store.pool())
        .await
        .expect_err("BLOB のキーは弾かれること");
        assert_kind(&err, ErrorKind::CheckViolation, &format!("key = {label}"));
    }
}

/// **埋め込み NUL は弾く**（`length()` の CHECK だけでは足りない）。
///
/// SQLite の `length()` はテキストを最初の NUL までしか数えないので、
/// `length(key) <= 12` は NUL を混ぜた任意長の値を素通しする（実測は
/// `migrations/0009_character_attribute.sql` のコメント）。
/// アプリ層は `AttributeKey::parse` が制御文字として弾くので、
/// **アプリが通す値を DB が弾く**非対称は生まれない。
#[tokio::test]
async fn attribute_key_and_value_reject_embedded_nul() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    let smuggled = format!("あ\u{0000}{}", "い".repeat(50));
    assert!(
        AttributeKey::parse(&smuggled).is_err() && AttributeValue::parse(&smuggled).is_err(),
        "アプリ層は NUL を拒否すること"
    );

    let err = insert_attribute(&store, character_id, "character", &smuggled, "猫獣人")
        .await
        .expect_err("NUL を含むキーは弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "key に NUL");

    let err = insert_attribute(&store, character_id, "character", "種族", &smuggled)
        .await
        .expect_err("NUL を含む値は弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "value に NUL");
}

/// **主キーは `(character_id, subject, key)`。**
///
/// * 同じ主語で同じキーを2件 → 主キー違反（どちらが載るかが挿入順まかせにならない）
/// * **主語が違えば同じキーを持てる** → `subject` を主キーから外す変異が赤になる
/// * キャラクターが違えば当然持てる
#[tokio::test]
async fn attribute_primary_key_is_character_subject_and_key() {
    let (_dir, store) = setup().await;
    let aoi = insert_character(&store, "aoi").await;
    let rin = insert_character(&store, "rin").await;

    insert_attribute(&store, aoi, "character", "年齢", "19")
        .await
        .expect("1件目は入ること");

    let err = insert_attribute(&store, aoi, "character", "年齢", "20")
        .await
        .expect_err("同じ主語の同じキーは弾かれること");
    assert_kind(&err, ErrorKind::UniqueViolation, "同一 (character, key)");

    insert_attribute(&store, aoi, "user", "年齢", "32")
        .await
        .expect("主語が違えば同じキーを持てること");
    insert_attribute(&store, rin, "character", "年齢", "17")
        .await
        .expect("キャラクターが違えば同じキーを持てること");

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM character_attribute")
        .fetch_one(store.pool())
        .await
        .expect("行数を数えられること");
    assert_eq!(rows, 3, "弾かれた1件を除いて3行あること");
}

/// **親のいない属性は書けない**（外部キー）。
#[tokio::test]
async fn attribute_requires_an_existing_character() {
    let (_dir, store) = setup().await;

    let err = insert_attribute(&store, 9_999, "character", "一人称", "僕")
        .await
        .expect_err("存在しないキャラクターの属性は書けないこと");
    assert_kind(&err, ErrorKind::ForeignKeyViolation, "未知の character_id");
}

// ============================================================================
// attribute_revision（ADR-0028 §決定5 / 作業単位 A-3a）
// ============================================================================

/// 属性の履歴に1行入れる（CHECK を通る素の SQL）。**`before` / `after` は NULL 可。**
async fn insert_revision(
    store: &Store,
    character_id: i64,
    subject: &str,
    key: &str,
    before: Option<&str>,
    after: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO attribute_revision (character_id, subject, key, before, after) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(character_id)
    .bind(subject)
    .bind(key)
    .bind(before)
    .bind(after)
    .execute(store.pool())
    .await
    .map(|_| ())
}

/// **履歴の `key` / `before` / `after` の CHECK が、0009 と同じ4条件を持っている**
/// （`migrations/0013_attribute_revision.sql` / ADR-0028 §決定5）。
///
/// # なぜ写しごと測るのか
///
/// 0013 の CHECK は 0009 の**写し**である（適用済みのマイグレーションは編集できないので、
/// 共有する手段が無い）。**写しは、片方を直してももう片方が黙って古びる。**
/// 4条件それぞれが**実挙動として**効いていることを、ここで固定する
/// （票の側の `drift_vote_axis_check_rejects_blank_blob_and_nul` と同じ形）。
///
/// # 条件ごとに効く領域
///
/// | 条件 | ここで入る値 |
/// |---|---|
/// | 空白のみを弾く | `BLANK_STRINGS`（TAB / 全角空白 / NBSP を含む） |
/// | `typeof(...) = 'text'` | **NUL を含まない BLOB**（`x'414243'`）。他の条件を素通りする |
/// | `instr(..., char(0)) = 0` | **NUL の後ろに長い文字列**（`length()` は最初の NUL まで） |
/// | `length(...) <= 12 / 20` | 上限ちょうどは通り、1文字超えたら通らない |
///
/// **3列とも見る**（1列だけ測ると、他の2列の条件を落とす変異が緑を通る）。
/// **`before` / `after` は NULL を許す**——「無かった」「消えた」を表す唯一の形なので、
/// ここを NOT NULL にする変異は逆向きの対照で赤になる。
#[tokio::test]
async fn attribute_revision_checks_reject_blank_blob_nul_and_over_long() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    // 正常系の対照。**NULL の側と、中間に空白を含む値。**
    insert_revision(
        &store,
        character_id,
        "character",
        "一人称",
        None,
        Some("僕"),
    )
    .await
    .expect("追加（before が NULL）は入ること");
    insert_revision(&store, character_id, "user", "呼び方", Some("先輩"), None)
        .await
        .expect("削除（after が NULL）は入ること");
    insert_revision(
        &store,
        character_id,
        "character",
        "内なる\u{3000}声",
        Some("僕"),
        Some("俺"),
    )
    .await
    .expect("中間の空白を含むキーは入ること");

    // 1. 空白のみ。**3列とも。**
    for blank in BLANK_STRINGS {
        let err = insert_revision(&store, character_id, "character", blank, None, Some("僕"))
            .await
            .expect_err("空白のみのキーは弾かれること");
        assert_kind(&err, ErrorKind::CheckViolation, &format!("key = {blank:?}"));

        let err = insert_revision(
            &store,
            character_id,
            "character",
            "一人称",
            Some(blank),
            Some("僕"),
        )
        .await
        .expect_err("空白のみの before は弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("before = {blank:?}"),
        );

        let err = insert_revision(
            &store,
            character_id,
            "character",
            "一人称",
            Some("僕"),
            Some(blank),
        )
        .await
        .expect_err("空白のみの after は弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("after = {blank:?}"),
        );
    }

    // 2. NUL を含まない BLOB。**`typeof` だけが弾ける値。**
    for column in ["key", "before", "after"] {
        let blob = vec![0x41_u8, 0x42, 0x43];
        let query = match column {
            "key" => sqlx::query(
                "INSERT INTO attribute_revision (character_id, subject, key, before, after) \
                 VALUES (?, 'character', ?, NULL, '僕')",
            )
            .bind(character_id)
            .bind(blob),
            "before" => sqlx::query(
                "INSERT INTO attribute_revision (character_id, subject, key, before, after) \
                 VALUES (?, 'character', '一人称', ?, '僕')",
            )
            .bind(character_id)
            .bind(blob),
            _ => sqlx::query(
                "INSERT INTO attribute_revision (character_id, subject, key, before, after) \
                 VALUES (?, 'character', '一人称', '僕', ?)",
            )
            .bind(character_id)
            .bind(blob),
        };
        let err = query
            .execute(store.pool())
            .await
            .expect_err("BLOB は弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("{column} = NUL を含まない BLOB"),
        );
    }

    // 3. 埋め込み NUL。**`length()` は最初の NUL までしか数えない。**
    let smuggled = format!("あ\u{0000}{}", "い".repeat(50));
    assert!(
        AttributeKey::parse(&smuggled).is_err(),
        "アプリ層も NUL を拒否すること"
    );
    let err = insert_revision(
        &store,
        character_id,
        "character",
        &smuggled,
        None,
        Some("僕"),
    )
    .await
    .expect_err("NUL を含むキーは弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "key に NUL");
    let err = insert_revision(
        &store,
        character_id,
        "character",
        "一人称",
        Some(&smuggled),
        Some("僕"),
    )
    .await
    .expect_err("NUL を含む before は弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "before に NUL");
    let err = insert_revision(
        &store,
        character_id,
        "character",
        "一人称",
        Some("僕"),
        Some(&smuggled),
    )
    .await
    .expect_err("NUL を含む after は弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "after に NUL");

    // 4. 長さ。**キーと値で上限が違う**（片方の定数をもう片方に使う変異を落とす）。
    let key_at_limit = "あ".repeat(AttributeKey::MAX_CHARS);
    insert_revision(
        &store,
        character_id,
        "character",
        &key_at_limit,
        None,
        Some("僕"),
    )
    .await
    .expect("上限ちょうどのキーは通ること");
    let key_over = "あ".repeat(AttributeKey::MAX_CHARS + 1);
    let err = insert_revision(
        &store,
        character_id,
        "character",
        &key_over,
        None,
        Some("僕"),
    )
    .await
    .expect_err("上限を超えるキーは弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "key が上限超過");

    let value_at_limit = "い".repeat(AttributeValue::MAX_CHARS);
    insert_revision(
        &store,
        character_id,
        "character",
        "出身",
        Some(&value_at_limit),
        Some(&value_at_limit),
    )
    .await
    .expect("上限ちょうどの値は通ること");
    let value_over = "い".repeat(AttributeValue::MAX_CHARS + 1);
    let err = insert_revision(
        &store,
        character_id,
        "character",
        "出身",
        Some(&value_over),
        None,
    )
    .await
    .expect_err("上限を超える before は弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "before が上限超過");
    let err = insert_revision(
        &store,
        character_id,
        "character",
        "出身",
        None,
        Some(&value_over),
    )
    .await
    .expect_err("上限を超える after は弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "after が上限超過");

    // **キーとして弾かれる長さでも、値としては通る**（上限が別の定数であること）。
    insert_revision(
        &store,
        character_id,
        "character",
        "一人称",
        Some(&key_over),
        None,
    )
    .await
    .expect("キーの上限を超える長さでも、値としては通ること");

    // 弾かれた行は1件も入っていない（通した6件だけ）。
    assert_eq!(
        count_rows(&store, "SELECT COUNT(*) FROM attribute_revision").await,
        6,
        "CHECK を通った行だけが入っていること"
    );
}

/// **履歴の `subject` も `user` / `character` のみ**（0009 と同じ2値）。
#[tokio::test]
async fn attribute_revision_subject_is_restricted_to_known_values() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    for subject in ["assistant", "User", "", " "] {
        let err = insert_revision(&store, character_id, subject, "一人称", None, Some("僕"))
            .await
            .expect_err("想定外の主語は弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("subject = {subject:?}"),
        );
    }
}

/// **親のいない履歴は書けない**（外部キー）。**親を消すと履歴も消える**（CASCADE）。
#[tokio::test]
async fn attribute_revision_follows_its_character() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    let err = insert_revision(&store, 9_999, "character", "一人称", None, Some("僕"))
        .await
        .expect_err("存在しないキャラクターの履歴は書けないこと");
    assert_kind(&err, ErrorKind::ForeignKeyViolation, "未知の character_id");

    insert_revision(
        &store,
        character_id,
        "character",
        "一人称",
        None,
        Some("僕"),
    )
    .await
    .expect("正当な履歴は入ること");
    sqlx::query("DELETE FROM character WHERE id = ?")
        .bind(character_id)
        .execute(store.pool())
        .await
        .expect("キャラクターを消せること");
    assert_eq!(
        count_rows(&store, "SELECT COUNT(*) FROM attribute_revision").await,
        0,
        "親を消すと履歴も消えること"
    );
}

/// **既存の DB に 0013 を当てられる。属性は読めたまま、履歴は空**（作業単位 A-3a）。
///
/// # なぜ要るのか
///
/// 履歴は**空の DB から作る**のが普通の経路だが、**既にあるキャラクターと属性を持つ DB**に
/// 当たるのが現実の経路である。**新しい表を足すだけの移行でも、既存行が読めなくなれば
/// その DB は起動不能になる**（CLAUDE.md の D-5a の実測）。
///
/// **「履歴が空である」ことまで見る。** 0013 は過去の属性を履歴へ埋め戻さない
/// ——埋め戻すと、**誰も行っていない変更**が「いつ変わったか」の表に載る
/// （しかも時刻は移行の時刻になる）。**属性が既にあることと、変更が記録されていることは別。**
#[tokio::test]
async fn the_history_migration_lands_on_an_existing_database() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("personacore.sqlite");

    // --- 0012 までの DB を作り、属性を2件入れる ---
    {
        let (_subset, pool) = apply_migrations_up_to(&path, 12).await;

        sqlx::query(
            "INSERT INTO character (id, handle, display_name, base_persona) VALUES (?, ?, ?, ?)",
        )
        .bind(7_i64)
        .bind("aoi")
        .bind("葵")
        .bind("ペルソナ本文")
        .execute(&pool)
        .await
        .expect("旧スキーマにキャラクターを入れられること");
        for (subject, key, value) in [("character", "一人称", "僕"), ("user", "年齢", "32")] {
            sqlx::query(
                "INSERT INTO character_attribute (character_id, subject, key, value) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(7_i64)
            .bind(subject)
            .bind(key)
            .bind(value)
            .execute(&pool)
            .await
            .expect("旧スキーマに属性を入れられること");
        }

        // **履歴の表がまだ無いこと**（前提。あると、この移行を測っていない）。
        sqlx::query("SELECT count(*) FROM attribute_revision")
            .fetch_one(&pool)
            .await
            .expect_err("0012 の時点では履歴の表が無いこと");

        pool.close().await;
    }

    // --- 0013 を当てる ---
    let store = Store::connect(&path)
        .await
        .expect("既存データのある DB にも 0013 を適用できること");

    let attributes = store
        .character_attributes(CharacterId::from_raw_for_tests(7))
        .await
        .expect("移行後も属性を読めること");
    assert_eq!(
        attributes
            .iter()
            .map(|attribute| (attribute.key.as_str(), attribute.value.as_str()))
            .collect::<Vec<_>>(),
        vec![("一人称", "僕"), ("年齢", "32")],
        "既存の属性がそのまま読めること"
    );
    assert_eq!(
        count_rows(&store, "SELECT COUNT(*) FROM attribute_revision").await,
        0,
        "移行は履歴を埋め戻さないこと（誰も行っていない変更を記録しない）"
    );

    // **移行の直後から履歴は書ける。**
    store
        .replace_character_attributes(
            CharacterId::from_raw_for_tests(7),
            &AttributeSet::parse(vec![CharacterAttribute {
                subject: Subject::Character,
                key: AttributeKey::parse("一人称").expect("正当なキー"),
                value: AttributeValue::parse("俺").expect("正当な値"),
            }])
            .expect("件数が上限内であること"),
        )
        .await
        .expect("移行後の DB でも置き換えられること");
    assert_eq!(
        count_rows(&store, "SELECT COUNT(*) FROM attribute_revision").await,
        2,
        "変更（一人称）と削除（年齢）が記録されること"
    );
}

/// **票の `axis` の CHECK 3条件が、全部効いている**
/// （`migrations/0010_stat_drift_vote.sql` / ADR-0026 §決定3）。
///
/// # なぜ要るのか（16d のレビューで実測）
///
/// **3条件（`typeof` / NUL / 空白のみ）を全部無効化しても 1338 全緑だった。**
/// マイグレーションのコメントは「0009 が実測した穴（`length()` は最初の NUL までしか
/// 数えない）」を根拠に条件を挙げているのに、**その実測が守られていることを
/// 誰も確かめていなかった。**
///
/// しかも `tests/common::insert_drift_vote_ignoring_checks` は
/// 「CHECK は `PRAGMA` で外せるから読み出しの検証が要る」と書いている。
/// **その裏返し（普通の経路では入らない）を、ここで測る。**
///
/// # 3条件それぞれが効く領域
///
/// | 条件 | ここで入る値 |
/// |---|---|
/// | 空白のみを弾く | `BLANK_STRINGS`（TAB / 全角空白 / NBSP を含む） |
/// | `typeof(axis) = 'text'` | **NUL を含まない BLOB**（`x'414243'`）。他の2条件を素通りする |
/// | `instr(axis, char(0)) = 0` | **NUL の後ろに長い文字列**（`length()` は最初の NUL までしか数えない） |
///
/// **BLOB は NUL を含まないもので試すこと。** 含むと `instr` の節が先に弾き、
/// `typeof` に到達しない（0009 の変異検査で判明した形）。
#[tokio::test]
async fn drift_vote_axis_check_rejects_blank_blob_and_nul() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_id = insert_session(&store, character_id).await;

    // 正常系の対照。**中間の空白を含む軸名は通ること**（`stat_alloc.axis` と同じ）。
    for axis in ["パッション", "内なる\u{3000}情熱"] {
        sqlx::query(
            "INSERT INTO stat_drift_vote (character_id, session_id, axis, delta) \
             VALUES (?, ?, ?, 'up')",
        )
        .bind(character_id)
        .bind(session_id)
        .bind(axis)
        .execute(store.pool())
        .await
        .expect("空白でない軸名は挿入できること");
    }

    // 1. 空白のみ。
    for axis in BLANK_STRINGS {
        let err = sqlx::query(
            "INSERT INTO stat_drift_vote (character_id, session_id, axis, delta) \
             VALUES (?, ?, ?, 'up')",
        )
        .bind(character_id)
        .bind(session_id)
        .bind(axis)
        .execute(store.pool())
        .await
        .expect_err("空白のみの軸名は CHECK 制約で弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("stat_drift_vote.axis = {axis:?}"),
        );
    }

    // 2. NUL を含まない BLOB。**`typeof` だけが弾ける値。**
    let err = sqlx::query(
        "INSERT INTO stat_drift_vote (character_id, session_id, axis, delta) \
         VALUES (?, ?, ?, 'up')",
    )
    .bind(character_id)
    .bind(session_id)
    .bind(vec![0x41_u8, 0x42, 0x43])
    .execute(store.pool())
    .await
    .expect_err("BLOB の軸名は弾かれること");
    assert_kind(
        &err,
        ErrorKind::CheckViolation,
        "axis = NUL を含まない BLOB",
    );

    // 3. 埋め込み NUL。**`length()` は最初の NUL までしか数えない。**
    let smuggled = format!("あ\u{0000}{}", "い".repeat(50));
    assert!(
        StatAxis::parse(&smuggled).is_err(),
        "アプリ層も NUL を拒否すること"
    );
    let err = sqlx::query(
        "INSERT INTO stat_drift_vote (character_id, session_id, axis, delta) \
         VALUES (?, ?, ?, 'up')",
    )
    .bind(character_id)
    .bind(session_id)
    .bind(&smuggled)
    .execute(store.pool())
    .await
    .expect_err("NUL を含む軸名は弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "axis に NUL");

    // **弾かれた行は1件も入っていない**（対照の2件だけ）。
    assert_eq!(
        count_rows(&store, "SELECT COUNT(*) FROM stat_drift_vote").await,
        2,
        "CHECK を通った行だけが入っていること"
    );
}

/// **`stat_drift_charge.charge` の CHECK が、有限でない値と型違いを弾く**
/// （作業単位 16e-b のレビュー指摘3 / `migrations/0012_stat_drift_charge_finite.sql`）。
///
/// # ±Inf が入ると、そのキャラクターの変容が永久に止まる
///
/// `typeof(charge) = 'real'` だけでは **±Inf が通る**（`9e999` は REAL である）。
/// 入った瞬間に `Store::drift_charges` が丸ごと `InvalidStoredValue` になり、
/// **累積には削除の導線が無い**ので `sqlite3` でしか直せない。
///
/// アプリ側は三重に塞いである（`DriftTuning` / `save_drift_charges` /
/// `DriftAxisLedger::new`）が、**表の側にも門を置く**
/// ——0009 / 0010 が `axis` に3条件を重ねているのと同じ判断。
///
/// # NaN は試さない
///
/// **SQLite に NaN の表現が無い。** 束縛すると NULL になり `NOT NULL` で弾かれるので、
/// `CheckViolation` ではなく `NotNullViolation` になる（別の門の仕事）。
#[tokio::test]
async fn drift_charge_check_rejects_an_infinite_or_mistyped_value() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    // 正常系の対照。**負も、極端に大きい有限値も通ること。**
    for (axis, charge) in [("パッション", -1.5_f64), ("ロジック", 1e308), ("共鳴", 0.0)]
    {
        sqlx::query("INSERT INTO stat_drift_charge (character_id, axis, charge) VALUES (?, ?, ?)")
            .bind(character_id)
            .bind(axis)
            .bind(charge)
            .execute(store.pool())
            .await
            .unwrap_or_else(|err| panic!("有限な累積は挿入できること: {charge:e}: {err}"));
    }

    // 1. ±Inf。**`typeof` だけでは弾けない値。**
    for (axis, charge) in [("上", f64::INFINITY), ("下", f64::NEG_INFINITY)] {
        let err = sqlx::query(
            "INSERT INTO stat_drift_charge (character_id, axis, charge) VALUES (?, ?, ?)",
        )
        .bind(character_id)
        .bind(axis)
        .bind(charge)
        .execute(store.pool())
        .await
        .expect_err("有限でない累積は CHECK 制約で弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("stat_drift_charge.charge = {charge}"),
        );
    }

    // 2. SQL リテラルの `9e999`（`sqlite3` から手で書ける形そのもの）。
    let err = sqlx::query(
        "INSERT INTO stat_drift_charge (character_id, axis, charge) VALUES (?, '手書き', 9e999)",
    )
    .bind(character_id)
    .execute(store.pool())
    .await
    .expect_err("9e999 は弾かれること");
    assert_kind(
        &err,
        ErrorKind::CheckViolation,
        "stat_drift_charge.charge = 9e999",
    );

    // 3. 数に見えない TEXT と BLOB。
    //
    // **`'1.5'` は弾けない**——REAL の型アフィニティが数に見える TEXT を
    // REAL へ変換するので、`typeof` は `'real'` になる（実測 / 16e-b）。
    // `typeof` が捕まえるのは**変換されずに残る形**だけである。
    let err =
        sqlx::query("INSERT INTO stat_drift_charge (character_id, axis, charge) VALUES (?, ?, ?)")
            .bind(character_id)
            .bind("文字列")
            .bind("たくさん")
            .execute(store.pool())
            .await
            .expect_err("数に見えない TEXT の累積は弾かれること");
    assert_kind(
        &err,
        ErrorKind::CheckViolation,
        "stat_drift_charge.charge = 'たくさん'",
    );

    let err =
        sqlx::query("INSERT INTO stat_drift_charge (character_id, axis, charge) VALUES (?, ?, ?)")
            .bind(character_id)
            .bind("バイト列")
            .bind(vec![0x41_u8, 0x42, 0x43])
            .execute(store.pool())
            .await
            .expect_err("BLOB の累積は弾かれること");
    assert_kind(
        &err,
        ErrorKind::CheckViolation,
        "stat_drift_charge.charge = BLOB",
    );

    // **`'1.5'` は通る**（アフィニティが REAL へ直す）。**読み出しでは落ちない。**
    sqlx::query("INSERT INTO stat_drift_charge (character_id, axis, charge) VALUES (?, ?, ?)")
        .bind(character_id)
        .bind("数に見える文字列")
        .bind("1.5")
        .execute(store.pool())
        .await
        .expect("数に見える TEXT はアフィニティで REAL になること");

    // 4. 軸名の側は 0011 の3条件がそのまま残っている（作り直しで落ちていないこと）。
    for axis in BLANK_STRINGS {
        let err = sqlx::query(
            "INSERT INTO stat_drift_charge (character_id, axis, charge) VALUES (?, ?, 0.0)",
        )
        .bind(character_id)
        .bind(axis)
        .execute(store.pool())
        .await
        .expect_err("空白のみの軸名は弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("stat_drift_charge.axis = {axis:?}"),
        );
    }
}

/// **`0014` より前に作った行は「未設定」で読め、起動時の既定に従う**（ADR-0045 §決定1 / 作業単位 A-8）。
///
/// # なぜ要るのか
///
/// ADR-0045 は「**既存のキャラクターは全部未設定から始まるので、
/// いまの DB の振る舞いは1つも変わらない**」と言っている。
/// **それを言えるのは、移行が既存の行に値を書かない場合だけ**で、
/// `DEFAULT 'off'` を置く変異では、`PERSONACORE_STAT_DRIFT=on` で回している
/// 既存の測定が**黙って止まる**（キャラクターの値は環境変数に勝つ）。
///
/// **他のテストはすべて空の DB から作った行を見る**ので、この移行が既存の行に何を書くかは
/// ここでしか観測できない。
///
/// # 読み出しは両方の経路で見る
///
/// **新しい列は、キャラクターを読む全経路のデコードの前提になる**（CLAUDE.md §2 の ⚠）。
/// 1件読み（`find_character_by_handle`）と一覧（`list_characters`）の両方で読む。
#[tokio::test]
async fn a_character_written_before_0014_reads_as_unset_and_follows_the_default() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("legacy.sqlite");

    // --- `0013` までを適用した「旧スキーマの DB」を作る ---
    {
        let (_subset, pool) = apply_migrations_up_to(&path, 13).await;

        // **列がまだ無いこと**を確かめてから入れる（あると、このテストは何も見ていない）。
        let err = sqlx::query("SELECT stat_drift_mode FROM character")
            .fetch_all(&pool)
            .await
            .expect_err("移行前は stat_drift_mode 列が存在しないこと");
        assert!(
            err.to_string().contains("stat_drift_mode"),
            "実際のエラー = {err}"
        );

        for (id, handle) in [(3_i64, "aoi"), (8, "rin")] {
            sqlx::query(
                "INSERT INTO character (id, handle, display_name, base_persona, stat_scale) \
                 VALUES (?, ?, ?, ?, 7)",
            )
            .bind(id)
            .bind(handle)
            .bind("葵")
            .bind("ペルソナ本文")
            .execute(&pool)
            .await
            .expect("キャラクターを入れられること");
        }
        pool.close().await;
    }

    // --- `0014` を当てる ---
    let store = Store::connect(&path)
        .await
        .expect("既存データのある DB にも 0014 を適用できること");

    // **生の値が NULL であること**（`None` に読めるだけなら、壊れた値を握り潰す実装でも通る）。
    let stored: Vec<Option<String>> =
        sqlx::query_scalar("SELECT stat_drift_mode FROM character ORDER BY id")
            .fetch_all(store.pool())
            .await
            .expect("列を読めること");
    assert_eq!(stored, vec![None, None], "既存の行は NULL のまま");

    let found = store
        .find_character_by_handle(&Handle::parse("aoi").expect("正当な識別子"))
        .await
        .expect("移行前の行を1件読みで読めること")
        .expect("対象を引けること");
    assert_eq!(found.id.get(), 3);
    assert_eq!(found.stat_drift_mode, None, "1件読みで未設定");

    let listed = store.list_characters().await.expect("一覧を取得できること");
    let modes: Vec<(i64, Option<StatDriftMode>)> = listed
        .iter()
        .map(|entry| match entry {
            CharacterEntry::Readable(character) => (character.id.get(), character.stat_drift_mode),
            CharacterEntry::Unreadable(broken) => {
                panic!("移行前の行が読めない行になった: {broken:?}")
            }
        })
        .collect();
    assert_eq!(modes, vec![(3, None), (8, None)], "一覧でも未設定");

    // **既定に従う。** 既定を off 以外にして、既定の値そのものが返ることを見る
    // （off で見ると「常に off」の実装と区別が付かない）。
    for default in [StatDriftMode::Shadow, StatDriftMode::On] {
        let resolved = ResolvedDriftMode::resolve(found.stat_drift_mode, default);
        assert_eq!(
            (resolved.mode(), resolved.source()),
            (default, DriftModeSource::Default)
        );
    }
}

// ---------------------------------------------------------------------------
// 軸の候補（ADR-0046 §決定1・§決定6 / 作業単位 A-9a / `0015`）
// ---------------------------------------------------------------------------

/// **`0015` より前に作った行は枠 0 で読め、候補は 0 件**（Issue origin#41「既存の DB の振る舞いが変わらない」）。
///
/// **他のテストはすべて空の DB から作った行を見る**ので、この移行が既存の行に何を書くかは
/// ここでしか観測できない（`DEFAULT` を 0 以外にする変異はここで赤になる）。
///
/// # 読み出しは両方の経路で見る
///
/// **新しい列は、キャラクターを読む全経路のデコードの前提になる**（CLAUDE.md §2 の ⚠）。
/// 1件読み（`find_character_by_handle`）と一覧（`list_characters`）の両方で読み、
/// **候補を作ろうとしても枠が無い**（`m` = 0 の振る舞い）ことまで見る。
#[tokio::test]
async fn a_character_written_before_0015_has_no_slots_and_no_candidates() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let path = dir.path().join("legacy.sqlite");

    // --- `0014` までを適用した「旧スキーマの DB」を作る ---
    {
        let (_subset, pool) = apply_migrations_up_to(&path, 14).await;

        // **列がまだ無いこと**を確かめてから入れる（あると、このテストは何も見ていない）。
        let err = sqlx::query("SELECT axis_discovery_slots FROM character")
            .fetch_all(&pool)
            .await
            .expect_err("移行前は axis_discovery_slots 列が存在しないこと");
        assert!(
            err.to_string().contains("axis_discovery_slots"),
            "実際のエラー = {err}"
        );

        for (id, handle) in [(3_i64, "aoi"), (8, "rin")] {
            sqlx::query(
                "INSERT INTO character (id, handle, display_name, base_persona, stat_scale, \
                 stat_drift_mode) VALUES (?, ?, ?, ?, 7, 'shadow')",
            )
            .bind(id)
            .bind(handle)
            .bind("葵")
            .bind("ペルソナ本文")
            .execute(&pool)
            .await
            .expect("キャラクターを入れられること");
        }
        sqlx::query(
            "INSERT INTO memory (character_id, subject, content) VALUES (3, 'character', ?)",
        )
        .bind("夜に星を見るのが好き")
        .execute(&pool)
        .await
        .expect("記憶を入れられること");
        pool.close().await;
    }

    // --- `0015` を当てる ---
    let store = Store::connect(&path)
        .await
        .expect("既存データのある DB にも 0015 を適用できること");

    // **生の値が 0 であること**（読み出しが既定を返すだけなら、壊れた値を握り潰す実装でも通る）。
    let stored: Vec<i64> =
        sqlx::query_scalar("SELECT axis_discovery_slots FROM character ORDER BY id")
            .fetch_all(store.pool())
            .await
            .expect("列を読めること");
    assert_eq!(stored, vec![0, 0], "既存の行は 0");

    let found = store
        .find_character_by_handle(&Handle::parse("aoi").expect("正当な識別子"))
        .await
        .expect("移行前の行を1件読みで読めること")
        .expect("対象を引けること");
    assert_eq!(
        found.axis_discovery_slots,
        AxisDiscoverySlots::NONE,
        "1件読みで 0"
    );
    assert_eq!(
        found.stat_drift_mode,
        Some(StatDriftMode::Shadow),
        "既存の列は変わらないこと"
    );

    let listed = store.list_characters().await.expect("一覧を取得できること");
    let slots: Vec<(i64, u32)> = listed
        .iter()
        .map(|entry| match entry {
            CharacterEntry::Readable(character) => {
                (character.id.get(), character.axis_discovery_slots.get())
            }
            CharacterEntry::Unreadable(broken) => {
                panic!("移行前の行が読めない行になった: {broken:?}")
            }
        })
        .collect();
    assert_eq!(slots, vec![(3, 0), (8, 0)], "一覧でも 0");

    assert!(
        store
            .axis_candidates(found.id, 100)
            .await
            .expect("候補を読めること")
            .is_empty(),
        "候補は 0 件"
    );
    let memory_id: i64 = sqlx::query_scalar("SELECT id FROM memory WHERE character_id = 3")
        .fetch_one(store.pool())
        .await
        .expect("記憶の id を引けること");
    let err = sqlx::query(
        "INSERT INTO stat_axis_candidate (character_id, name, source_memory_id) VALUES (8, ?, ?)",
    )
    .bind("好奇心")
    .bind(memory_id)
    .execute(store.pool())
    .await
    .expect_err("他のキャラクターの記憶を出どころにできないこと（複合外部キー）");
    assert_kind(
        &err,
        ErrorKind::ForeignKeyViolation,
        "stat_axis_candidate.source_memory_id",
    );
}

/// **枠の数の CHECK が、範囲外と整数でない値を全部止める**（`0015`）。
///
/// 整数に見える TEXT（`'3'`）と整数値の REAL（`3.0`）は**型アフィニティで整数へ直って**入る
/// （止める必要が無い / 読み出しでも落ちない）。止めるのは直らずに残る形。
#[tokio::test]
async fn axis_discovery_slots_check_rejects_out_of_range_and_non_integers() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    for (label, result) in [
        (
            "5",
            sqlx::query("UPDATE character SET axis_discovery_slots = 5 WHERE id = ?"),
        ),
        (
            "-1",
            sqlx::query("UPDATE character SET axis_discovery_slots = -1 WHERE id = ?"),
        ),
        (
            "2.5",
            sqlx::query("UPDATE character SET axis_discovery_slots = 2.5 WHERE id = ?"),
        ),
        (
            "'abc'",
            sqlx::query("UPDATE character SET axis_discovery_slots = 'abc' WHERE id = ?"),
        ),
        (
            "X'01'",
            sqlx::query("UPDATE character SET axis_discovery_slots = X'01' WHERE id = ?"),
        ),
        (
            "NULL",
            sqlx::query("UPDATE character SET axis_discovery_slots = NULL WHERE id = ?"),
        ),
    ] {
        let err = result
            .bind(character_id)
            .execute(store.pool())
            .await
            .expect_err("範囲外・整数でない値は弾かれること");
        let expected = if label == "NULL" {
            ErrorKind::NotNullViolation
        } else {
            ErrorKind::CheckViolation
        };
        assert_kind(&err, expected, &format!("axis_discovery_slots = {label}"));
    }

    for (label, sql, expected) in [
        (
            "4",
            "UPDATE character SET axis_discovery_slots = 4 WHERE id = ?",
            4_i64,
        ),
        (
            "0",
            "UPDATE character SET axis_discovery_slots = 0 WHERE id = ?",
            0,
        ),
        (
            "'3'",
            "UPDATE character SET axis_discovery_slots = '3' WHERE id = ?",
            3,
        ),
        (
            "3.0",
            "UPDATE character SET axis_discovery_slots = 3.0 WHERE id = ?",
            3,
        ),
    ] {
        sqlx::query(sql)
            .bind(character_id)
            .execute(store.pool())
            .await
            .unwrap_or_else(|err| panic!("{label} は入ること: {err}"));
        let (stored, kind): (i64, String) = sqlx::query_as(
            "SELECT axis_discovery_slots, typeof(axis_discovery_slots) FROM character WHERE id = ?",
        )
        .bind(character_id)
        .fetch_one(store.pool())
        .await
        .expect("読めること");
        assert_eq!((stored, kind.as_str()), (expected, "integer"), "{label}");
    }
}

/// 候補の行を1件入れる（名前だけを変えて CHECK を試す）。
async fn insert_axis_candidate<'q, T>(
    store: &Store,
    character_id: i64,
    memory_id: i64,
    name: T,
) -> Result<i64, sqlx::Error>
where
    T: 'q + Send + sqlx::Encode<'q, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO stat_axis_candidate (character_id, name, source_memory_id) VALUES (?, ?, ?)",
    )
    .bind(character_id)
    .bind(name)
    .bind(memory_id)
    .execute(store.pool())
    .await
    .map(|done| done.last_insert_rowid())
}

/// **候補の名前の CHECK が、空白のみ・BLOB・NUL・長すぎるを止める**（`0015` / `0013` の key と同じ4条件）。
///
/// 長さは `StatAxis::MAX_CHARS`（20）の写し。**20 は通り、21 は止まる**（両側を見る）。
#[tokio::test]
async fn axis_candidate_name_check_rejects_blank_blob_nul_and_over_long() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let memory_id = insert_memory(&store, character_id, "猫が好き").await;

    for blank in BLANK_STRINGS {
        let err = insert_axis_candidate(&store, character_id, memory_id, blank)
            .await
            .expect_err("空白のみの名前は弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("name = {blank:?}"),
        );
    }
    let err = insert_axis_candidate(&store, character_id, memory_id, "好奇\u{0}心")
        .await
        .expect_err("NUL を含む名前は弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "name に NUL");
    let err = insert_axis_candidate(
        &store,
        character_id,
        memory_id,
        "好奇心".as_bytes().to_vec(),
    )
    .await
    .expect_err("BLOB の名前は弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "name が BLOB");
    let over_long = "あ".repeat(StatAxis::MAX_CHARS + 1);
    let err = insert_axis_candidate(&store, character_id, memory_id, over_long.as_str())
        .await
        .expect_err("21 文字の名前は弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "name が 21 文字");

    let at_limit = "あ".repeat(20);
    insert_axis_candidate(&store, character_id, memory_id, at_limit.as_str())
        .await
        .expect("20 文字の名前は入ること");
    assert_eq!(
        StatAxis::MAX_CHARS,
        20,
        "前提: 移行の写し（20）と StatAxis の上限が一致していること"
    );
}

/// **移り変わりの状態と理由は組でしか入らない**（`0015`）。票は3つの綴りだけ。
#[tokio::test]
async fn axis_candidate_transition_and_vote_spellings_are_closed() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let memory_id = insert_memory(&store, character_id, "猫が好き").await;
    let session_id = insert_session(&store, character_id).await;
    let candidate_id = insert_axis_candidate(&store, character_id, memory_id, "好奇心")
        .await
        .expect("候補を入れられること");

    for (state, reason) in [
        ("failed", None),
        ("trial", Some("pinned")),
        ("passed", Some("silent")),
        ("failed", Some("unknown")),
        ("Trial", None),
        ("paused", None),
        ("", None),
    ] {
        let err = sqlx::query(
            "INSERT INTO stat_axis_candidate_transition (candidate_id, state, reason) VALUES (?, ?, ?)",
        )
        .bind(candidate_id)
        .bind(state)
        .bind(reason)
        .execute(store.pool())
        .await
        .expect_err("規則外の組は弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("({state:?}, {reason:?})"),
        );
    }
    for (state, reason) in [
        ("trial", None),
        ("passed", None),
        ("failed", Some("pinned")),
        ("failed", Some("silent")),
        ("failed", Some("too_few_samples")),
        ("adopted", None),
        ("rejected", None),
    ] {
        sqlx::query(
            "INSERT INTO stat_axis_candidate_transition (candidate_id, state, reason) VALUES (?, ?, ?)",
        )
        .bind(candidate_id)
        .bind(state)
        .bind(reason)
        .execute(store.pool())
        .await
        .unwrap_or_else(|err| panic!("({state:?}, {reason:?}) は入ること: {err}"));
    }

    for vote in ["UP", " up", "0", "neutral", ""] {
        let err = sqlx::query(
            "INSERT INTO stat_axis_trial_vote (candidate_id, session_id, vote) VALUES (?, ?, ?)",
        )
        .bind(candidate_id)
        .bind(session_id)
        .bind(vote)
        .execute(store.pool())
        .await
        .expect_err("規則外の票は弾かれること");
        assert_kind(&err, ErrorKind::CheckViolation, &format!("vote = {vote:?}"));
    }
    for vote in ["up", "down", "zero"] {
        sqlx::query(
            "INSERT INTO stat_axis_trial_vote (candidate_id, session_id, vote) VALUES (?, ?, ?)",
        )
        .bind(candidate_id)
        .bind(session_id)
        .bind(vote)
        .execute(store.pool())
        .await
        .unwrap_or_else(|err| panic!("{vote} は入ること: {err}"));
    }
}

// ======================================================================
// 取り決め（ADR-0051 §決定1 / 作業単位 A-12a / migrations/0018_agreement.sql）
// ======================================================================

/// 取り決めを1行置く（中身・種類・守る側・状態を選べる）。
async fn insert_agreement<'q>(
    store: &Store,
    character_id: i64,
    content: impl sqlx::Encode<'q, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite> + 'q,
    kind: &'q str,
    keeper: &'q str,
    state: &'q str,
) -> Result<i64, sqlx::Error> {
    sqlx::query(
        "INSERT INTO character_agreement (character_id, content, kind, keeper, state) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(character_id)
    .bind(content)
    .bind(kind)
    .bind(keeper)
    .bind(state)
    .execute(store.pool())
    .await
    .map(|done| done.last_insert_rowid())
}

/// **中身の上限は domain とスキーマで揃っている**（`AgreementContent::MAX_CHARS` の doc）。
///
/// 受理される値と拒否される値の両方を見る（境界の片側だけだと、CHECK を丸ごと落とす変異は
/// 「拒否される値」でしか捕まらない）。**文字数で数える**（日本語をバイト数で弾かない）。
#[tokio::test]
async fn agreement_content_limit_matches_the_schema_check() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    let at_limit = "約".repeat(AgreementContent::MAX_CHARS);
    assert!(AgreementContent::parse(&at_limit).is_ok(), "前提");
    assert!(
        at_limit.len() > AgreementContent::MAX_CHARS,
        "前提: 多バイト"
    );
    insert_agreement(&store, character_id, at_limit, "promise", "user", "active")
        .await
        .expect("上限ちょうどの中身は DB も通すこと");

    let over = "約".repeat(AgreementContent::MAX_CHARS + 1);
    assert!(AgreementContent::parse(&over).is_err(), "前提");
    let err = insert_agreement(&store, character_id, over, "promise", "user", "active")
        .await
        .expect_err("上限を超える中身は CHECK 制約で弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "content が上限超過");
}

/// **中身の CHECK の4条件**（空白のみ・BLOB・NUL を混ぜた長さのごまかし）が全部効いている。
#[tokio::test]
async fn agreement_content_checks_reject_blank_blob_and_nul() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    for blank in BLANK_STRINGS {
        let err = insert_agreement(&store, character_id, blank, "rule", "both", "active")
            .await
            .expect_err("空白だけの中身は弾かれること");
        assert_kind(&err, ErrorKind::CheckViolation, &format!("{blank:?}"));
    }
    let err = insert_agreement(
        &store,
        character_id,
        "約束".as_bytes().to_vec(),
        "rule",
        "both",
        "active",
    )
    .await
    .expect_err("BLOB は弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "BLOB");
    // **length() は最初の NUL までしか数えない**ので、NUL そのものを見ている CHECK が要る。
    let smuggled = format!("ab{}{}", '\0', "c".repeat(AgreementContent::MAX_CHARS * 2));
    let err = insert_agreement(&store, character_id, smuggled, "rule", "both", "active")
        .await
        .expect_err("NUL を混ぜた中身は弾かれること");
    assert_kind(&err, ErrorKind::CheckViolation, "NUL");
}

/// **種類・守る側・状態は閉じた綴りだけ**（状態は「提案中」「却下」の綴りも受ける）。
///
/// 受ける綴りは domain の定義から作る（**写しの食い違いは、ここが赤にする**）。
/// 状態の予約の2つ（`proposed` / `rejected`）は domain に無いので、ここに直書きする。
#[tokio::test]
async fn agreement_kind_keeper_and_state_are_restricted() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;

    for kind in AgreementKind::ALL.map(AgreementKind::label) {
        insert_agreement(&store, character_id, "約束", kind, "user", "active")
            .await
            .unwrap_or_else(|err| panic!("kind = {kind} は入ること: {err}"));
    }
    for keeper in AgreementKeeper::ALL.map(AgreementKeeper::label) {
        insert_agreement(&store, character_id, "約束", "rule", keeper, "active")
            .await
            .unwrap_or_else(|err| panic!("keeper = {keeper} は入ること: {err}"));
    }
    let reserved = ["proposed", "rejected"];
    for state in AgreementState::ALL
        .map(AgreementState::label)
        .into_iter()
        .chain(reserved)
    {
        insert_agreement(&store, character_id, "約束", "rule", "user", state)
            .await
            .unwrap_or_else(|err| panic!("state = {state} は入ること: {err}"));
    }

    for (kind, keeper, state) in [
        ("Promise", "user", "active"),
        ("", "user", "active"),
        ("rule", "assistant", "active"),
        ("rule", "both ", "active"),
        ("rule", "user", "Active"),
        ("rule", "user", "done"),
        ("rule", "user", ""),
    ] {
        let err = insert_agreement(&store, character_id, "約束", kind, keeper, state)
            .await
            .expect_err("規則外の綴りは弾かれること");
        assert_kind(
            &err,
            ErrorKind::CheckViolation,
            &format!("({kind:?}, {keeper:?}, {state:?})"),
        );
    }
}

/// **履歴の前後の状態も閉じた綴りだけ。前は NULL（書かれた）を許し、後は許さない。**
#[tokio::test]
async fn agreement_transition_states_are_restricted() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let agreement_id = insert_agreement(&store, character_id, "約束", "rule", "user", "active")
        .await
        .expect("取り決めを置けること");
    let insert = |before: Option<&'static str>, after: Option<&'static str>| {
        let store = store.clone();
        async move {
            sqlx::query(
                "INSERT INTO agreement_transition (agreement_id, before, after) VALUES (?, ?, ?)",
            )
            .bind(agreement_id)
            .bind(before)
            .bind(after)
            .execute(store.pool())
            .await
        }
    };

    insert(None, Some("active"))
        .await
        .expect("書かれた行は入ること");
    insert(Some("active"), Some("fulfilled"))
        .await
        .expect("終わった行は入ること");
    for (before, after) in [
        (None, None),
        (Some("Active"), Some("fulfilled")),
        (Some("active"), Some("done")),
        (Some(""), Some("broken")),
    ] {
        let err = insert(before, after)
            .await
            .expect_err("規則外の履歴は弾かれること");
        let expected = if after.is_none() {
            ErrorKind::NotNullViolation
        } else {
            ErrorKind::CheckViolation
        };
        assert_kind(&err, expected, &format!("({before:?}, {after:?})"));
    }

    let err = insert(None, Some("active")).await;
    assert!(err.is_ok(), "同じ取り決めに何行でも足せること（追記）");
    let err = sqlx::query(
        "INSERT INTO agreement_transition (agreement_id, before, after) VALUES (?, NULL, 'active')",
    )
    .bind(agreement_id + 1000)
    .execute(store.pool())
    .await
    .expect_err("存在しない取り決めの履歴は書けないこと");
    assert_kind(&err, ErrorKind::ForeignKeyViolation, "未知の agreement_id");
}

/// **出どころの会話は在る会話だけを指せる**（NULL は手で書いたもの）。
#[tokio::test]
async fn agreement_source_session_must_exist() {
    let (_dir, store) = setup().await;
    let character_id = insert_character(&store, "aoi").await;
    let session_id = insert_session(&store, character_id).await;

    for source in [None, Some(session_id)] {
        sqlx::query(
            "INSERT INTO character_agreement (character_id, content, kind, keeper, state, source_session_id) \
             VALUES (?, '約束', 'rule', 'user', 'active', ?)",
        )
        .bind(character_id)
        .bind(source)
        .execute(store.pool())
        .await
        .unwrap_or_else(|err| panic!("{source:?} は入ること: {err}"));
    }
    let err = sqlx::query(
        "INSERT INTO character_agreement (character_id, content, kind, keeper, state, source_session_id) \
         VALUES (?, '約束', 'rule', 'user', 'active', ?)",
    )
    .bind(character_id)
    .bind(session_id + 1000)
    .execute(store.pool())
    .await
    .expect_err("存在しない会話は指せないこと");
    assert_kind(
        &err,
        ErrorKind::ForeignKeyViolation,
        "未知の source_session_id",
    );
}
