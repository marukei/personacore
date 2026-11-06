//! **sqlx の帳簿（`_sqlx_migrations`）を、改行違いの checksum から付け替える**
//! （[ADR-0044](../../docs/adr/0044-migration-checksums-do-not-depend-on-line-endings.md) 決定3）。
//!
//! # 何を直しているのか
//!
//! `sqlx::migrate!` は各 SQL の**バイト列そのもの**の SHA-384 を checksum にする
//! （`sqlx-core 0.9.0` の `migrate/migration.rs`: `Sha384::digest(sql)`。**改行の正規化は無い**）。
//! このリポジトリは長く `.gitattributes` を持たず `core.autocrlf` が有効だったので、
//! **checkout の仕方で `migrations/*.sql` の改行が変わり**、
//! **同じコミットから組んだ2つの exe が同じ DB を**
//! `migration 5 was previously applied but has been modified` **で開けなくなった**
//! （2026-09-19 に実測）。
//!
//! 決定1（`.gitattributes` の `migrations/*.sql text eol=lf`）は**これから組む exe** を揃える。
//! **既に在る DB は揃わない** —— 帳簿には CRLF の checkout で計算された値が入っている。
//! ここが、その行だけを LF 版へ付け替える。
//!
//! # 付け替える条件は1つだけ
//!
//! **「帳簿の値が CRLF 版に一致し、LF 版に一致しない」行だけ**を動かす。
//! **本当に編集された SQL は、従来どおり `MIGRATOR.run` が止める** ——
//! 第3の値（どちらの版でもない）は、ここを素通りする。
//!
//! **⚠ 触るのは `_sqlx_migrations.checksum` だけ。** 利用者のデータには1バイトも触らない
//! （`memory` の行を DELETE / UPDATE しない絶対制約とは別の表）。
//!
//! # 逆向き（CRLF で組んだ exe が LF の帳簿を開く）は救わない
//!
//! 属性が消えた checkout で組むと、埋め込まれる SQL が CRLF になる。
//! そのとき [`as_crlf`] は**入力をそのまま返す**ので、ここは1行も付け替えない。
//! **そちらは「起きてはいけない状態」**で、止めるのは
//! `no_migration_file_carries_a_carriage_return`（決定2 の見張り番）である。
//! ここで両向きに付け替えると、**どちらが正典かを帳簿から決められなくなる。**
//!
//! **⚠ その `continue` は到達不能な防御である**（レビュー1周目の実測 / 2026-09-19）。
//! **`if false` に変えても 2180 全緑 ＝ 等価変異。**
//! 埋め込みが LF である限り成り立たず、**成り立つ世界（CRLF で組んだ exe）でも、
//! 同じ入力を後段の `continue` が落とす**（LF 版＝CRLF 版なので帳簿は必ず一致する）。
//! **逆向きを実際に止めているのは CR 走査のほう**であって、この分岐ではない。
//! **読み手への表明として残す。見張り番だと読まないこと。**

use std::borrow::Cow;
use std::collections::HashMap;

use sqlx::SqlitePool;
use sqlx::migrate::{Migration, MigrationType};
use sqlx::{AssertSqlSafe, SqlSafeStr};

use super::{MIGRATOR, StoreError};

/// 帳簿そのものが在るか。**無ければ新しい DB** で、付け替える相手が居ない。
///
/// **表の名前をプレースホルダにはできない**ので、綴りはここに直に書く
/// （`Migrator::table_name` は `#[doc(hidden)]` ＝ semver 対象外なので触らない）。
/// **綴りを取り違えると帳簿を一度も見なくなる** ——
/// `a_ledger_written_by_a_crlf_checkout_opens` が赤くなる。
const SELECT_LEDGER_EXISTS: &str =
    "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'";

/// 帳簿の版と checksum を全部読む。
const SELECT_LEDGER: &str = "SELECT version, checksum FROM _sqlx_migrations";

/// 1行の checksum を付け替える。**値も版もプレースホルダ。**
const UPDATE_CHECKSUM: &str = "UPDATE _sqlx_migrations SET checksum = ? WHERE version = ?";

/// 付け替えたことを名乗る文言。**版は本文ではなく欄（`migration=N`）に載せる**
/// ——CLAUDE.md「ログを見る見張り番は、必ず同じ行の中で見ること」。
const REPAIRED_MARKER: &str = "マイグレーションの帳簿を改行違いから付け替えました";

/// LF の SQL を、**CRLF で checkout された同じファイルのバイト列**に直す（純粋関数）。
///
/// **既に `\r\n` になっている位置を二重にしない**（`\r\r\n` を作らない）。
/// **単独の `\r`（旧 Mac 風）はそのまま**残す ——
/// checkout が作るのは `\n` → `\r\n` の置換だけなので、それ以外を真似ると別物になる。
fn as_crlf(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut previous_was_cr = false;
    for ch in sql.chars() {
        if ch == '\n' && !previous_was_cr {
            out.push('\r');
        }
        out.push(ch);
        previous_was_cr = ch == '\r';
    }
    out
}

/// **sqlx と同じ規則で** SQL の checksum を計算する（純粋関数）。
///
/// # 新しい依存を足していない
///
/// 計算そのもの（`sqlx_core::migrate::migration::checksum`）は**公開されていない**
/// （`mod migration` が私有で、再公開は `Migration` と `AppliedMigration` だけ）。
/// **`sha2` を直の依存に足す代わりに、sqlx 自身の `Migration::new` を通す** ——
/// あれは受け取った SQL から checksum を計算して持つので、**規則は定義上一致する。**
///
/// **⚠ だから「sqlx と同じ値になる」をこの関数で確かめても同義反復になる。**
/// 権威は別に置いた ——
/// * LF 側: `migrate!` が**コンパイル時に**計算した `Migration::checksum` と突き合わせる
///   （`the_checksum_matches_what_the_macro_embedded`）。
/// * CRLF 側: **実ファイルを CRLF に直して一時ディレクトリへ写し、
///   sqlx の `Migrator::new` に読ませた値**と突き合わせる
///   （`the_crlf_checksum_matches_what_sqlx_reads_from_crlf_files`）。
///   **これが「CRLF の checkout が実際に書く値」そのものである。**
///
/// 版・説明・種別・`no_tx` は checksum に入らないので、ここでは意味を持たない。
fn checksum_of(sql: &str) -> Vec<u8> {
    Migration::new(
        0,
        Cow::Borrowed(""),
        MigrationType::Simple,
        AssertSqlSafe(sql.to_owned()).into_sql_str(),
        false,
    )
    .checksum
    .into_owned()
}

/// 帳簿のうち、**改行違いでしか説明が付かない行**を LF 版へ付け替える。
///
/// `MIGRATOR.run` より**前**に呼ぶこと（呼び出しは [`super::Store::migrate`] の冒頭）。
pub(super) async fn repair_line_ending_checksums(pool: &SqlitePool) -> Result<(), StoreError> {
    let ledger_exists: Option<i64> = sqlx::query_scalar(SELECT_LEDGER_EXISTS)
        .fetch_optional(pool)
        .await
        .map_err(StoreError::Query)?;
    if ledger_exists.is_none() {
        // 新しい DB。**ここで早期に返さないと、次の `SELECT` が
        // 「そんな表は無い」で落ちて、初回の起動が丸ごと止まる。**
        return Ok(());
    }

    let recorded: HashMap<i64, Vec<u8>> = sqlx::query_as::<_, (i64, Vec<u8>)>(SELECT_LEDGER)
        .fetch_all(pool)
        .await
        .map_err(StoreError::Query)?
        .into_iter()
        .collect();

    for migration in MIGRATOR.iter() {
        let embedded = migration.sql.as_str();
        let crlf = as_crlf(embedded);
        if crlf == embedded {
            // 埋め込まれた SQL が既に CRLF（＝属性の消えた checkout で組んだ exe）。
            // **付け替える先が無い。** モジュール doc の「逆向きは救わない」。
            //
            // **⚠ ここは到達不能な防御**（モジュール doc の実測）。
            // **消しても振る舞いは変わらない。** 止めているのは CR 走査のほう。
            continue;
        }

        let Some(stored) = recorded.get(&migration.version) else {
            // まだ適用していない版。`MIGRATOR.run` が普通に適用する。
            continue;
        };

        let lf_checksum = migration.checksum.as_ref();
        if stored.as_slice() == lf_checksum {
            continue;
        }
        if stored.as_slice() != checksum_of(&crlf).as_slice() {
            // 改行違いでは説明が付かない ＝ **本当に編集された SQL。**
            // **何もしない。** `MIGRATOR.run` が今までどおり止める。
            continue;
        }

        sqlx::query(UPDATE_CHECKSUM)
            .bind(lf_checksum)
            .bind(migration.version)
            .execute(pool)
            .await
            .map_err(StoreError::Query)?;

        tracing::info!(migration = migration.version, "{REPAIRED_MARKER}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use sqlx::migrate::Migrator;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

    use super::*;
    use crate::store::Store;
    use crate::testlog::{captured_under_default_filter, has_field};

    /// 捕捉窓の中で `await` するための runtime（`src/llm/lmstudio.rs` と同じ作法）。
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime を作れること")
    }

    fn migrations_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
    }

    /// `migrations/*.sql` を**名前とバイト列**で読む。
    fn migration_files() -> Vec<(String, Vec<u8>)> {
        let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(migrations_dir())
            .expect("migrations ディレクトリを読めること")
            .map(|entry| {
                let entry = entry.expect("エントリを読めること");
                let name = entry.file_name().to_string_lossy().into_owned();
                let bytes = std::fs::read(entry.path()).expect("SQL を読めること");
                (name, bytes)
            })
            .collect();
        files.sort();
        files
    }

    /// **走査本体。** 渡された（名前, バイト列）の組から、CR を含むものを挙げる。
    ///
    /// **実ファイルと植えた文字列を同じ判定に通す**ため、引数で受け取る形にしてある
    /// （CLAUDE.md「走査本体を引数でソースを受け取る形にすれば、
    /// 実ファイルと植えたソースの両方を同じ判定に通せる」）。
    fn files_with_carriage_return(files: &[(&str, &[u8])]) -> Vec<String> {
        files
            .iter()
            .filter_map(|(name, bytes)| {
                let count = bytes.iter().filter(|byte| **byte == b'\r').count();
                (count > 0).then(|| format!("{name}: CR {count} バイト"))
            })
            .collect()
    }

    /// **CRLF で checkout した機が実際に書く checksum。**
    ///
    /// 実ファイルを CRLF に直して一時ディレクトリへ写し、**sqlx 自身の `Migrator::new`**
    /// に読ませて取る。**この関数は [`as_crlf`] も [`checksum_of`] も通らない**ので、
    /// 被験側の計算に対する独立した権威になる。
    async fn crlf_checksums_from_files() -> BTreeMap<i64, Vec<u8>> {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let files = migration_files();
        for (name, bytes) in &files {
            let text = String::from_utf8(bytes.clone()).expect("SQL は UTF-8 であること");
            // **`as_crlf` とは別の書き方**（一度 LF へ畳んでから広げる）。
            let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
            std::fs::write(dir.path().join(name), crlf.as_bytes()).expect("写せること");
        }

        let migrator = Migrator::new(dir.path())
            .await
            .expect("CRLF の写しを読めること");
        assert_eq!(
            migrator.iter().count(),
            files.len(),
            "前提: 写した数と読めた数が一致すること"
        );

        migrator
            .iter()
            .map(|migration| (migration.version, migration.checksum.to_vec()))
            .collect()
    }

    /// 埋め込まれている（＝LF の）checksum。
    fn embedded_checksums() -> BTreeMap<i64, Vec<u8>> {
        MIGRATOR
            .iter()
            .map(|migration| (migration.version, migration.checksum.to_vec()))
            .collect()
    }

    /// 帳簿の1行。**checksum 以外の欄まで持つ**のは、帰属の検査で
    /// 「その行だけが動いた」ではなく「**その行の checksum だけが動いた**」を言うため。
    #[derive(Debug, PartialEq, Eq)]
    struct LedgerRow {
        version: i64,
        checksum: Vec<u8>,
        installed_on: String,
        execution_time: i64,
        description: String,
        success: i64,
    }

    async fn ledger(pool: &SqlitePool) -> Vec<LedgerRow> {
        sqlx::query_as::<_, (i64, Vec<u8>, String, i64, String, i64)>(
            "SELECT version, checksum, CAST(installed_on AS TEXT), execution_time, \
             description, CAST(success AS INTEGER) \
             FROM _sqlx_migrations ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .expect("帳簿を読めること")
        .into_iter()
        .map(
            |(version, checksum, installed_on, execution_time, description, success)| LedgerRow {
                version,
                checksum,
                installed_on,
                execution_time,
                description,
                success,
            },
        )
        .collect()
    }

    /// 生のプールを開く（`Store` を通さずに帳簿を触るため）。
    async fn raw_pool(path: &Path) -> SqlitePool {
        SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(false),
        )
        .await
        .expect("既存の DB を開けること")
    }

    /// 指定した版の checksum を、渡した値に書き換える。
    ///
    /// **[`UPDATE_CHECKSUM`] を使い回さない。** 本番と同じ定数で植えると、
    /// **その定数を変える変異が本番と仕掛けの両方を動かして**、
    /// 赤の理由が「本番が壊れた」なのか「仕掛けが壊れた」なのか分からなくなる
    /// （CLAUDE.md「両側が動いて『赤1本』という無意味な結果」）。
    async fn plant(path: &Path, versions: &BTreeMap<i64, Vec<u8>>) {
        let pool = raw_pool(path).await;
        for (version, checksum) in versions {
            let affected =
                sqlx::query("UPDATE _sqlx_migrations SET checksum = ?1 WHERE version = ?2")
                    .bind(checksum.as_slice())
                    .bind(*version)
                    .execute(&pool)
                    .await
                    .expect("帳簿を書き換えられること")
                    .rows_affected();
            assert_eq!(affected, 1, "前提: 版 {version} の行が1行あること");
        }
        pool.close().await;
    }

    /// マイグレーション適用済みの DB を作る。
    async fn fresh(path: &Path) {
        let store = Store::connect(path).await.expect("初回の起動が通ること");
        store.close().await;
    }

    // ------------------------------------------------------------------
    // 決定2 —— `migrations/*.sql` に CR が1バイトも無い
    // ------------------------------------------------------------------

    /// **実ファイルに CR が1バイトも無い。**
    ///
    /// `.gitattributes` の `migrations/*.sql text eol=lf` が消えると、
    /// **次に Windows で checkout した機で赤くなる**（それがこの1本の存在理由）。
    ///
    /// **⚠ 「0 件だった」と「1件も読んでいない」を区別する**ため、
    /// 読めた本数を**埋め込まれたマイグレーションの数**と突き合わせる
    /// （CLAUDE.md「コーパスの大きさに下限が無く、縮めても全部緑だった」）。
    #[test]
    fn no_migration_file_carries_a_carriage_return() {
        let files = migration_files();
        assert_eq!(
            files.len(),
            MIGRATOR.iter().count(),
            "前提: 読めたファイル数が埋め込まれた版の数と一致すること: {:?}",
            files.iter().map(|(name, _)| name).collect::<Vec<_>>()
        );

        let borrowed: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        assert_eq!(
            files_with_carriage_return(&borrowed),
            Vec::<String>::new(),
            "migrations/*.sql に CR が在る。`.gitattributes` の \
             `migrations/*.sql text eol=lf` を確かめること（ADR-0044 決定1）"
        );
    }

    /// **走査本体が、植えた文字列では CR を現に見つける**（`15b` の作法）。
    ///
    /// 実ファイルだけを見ていると、**走査が壊れて何も見つけられなくなっても緑**になる。
    /// 期待値は**関数を通さないべた書き**にしてある。
    #[test]
    fn the_carriage_return_scan_finds_a_planted_one() {
        let lf: &[u8] = b"CREATE TABLE t (a INTEGER);\n";
        let crlf: &[u8] = b"CREATE TABLE t (a INTEGER);\r\n";
        let two_crlf: &[u8] = b"a\r\nb\r\n";

        assert_eq!(
            files_with_carriage_return(&[
                ("植えた_lf.sql", lf),
                ("植えた_crlf.sql", crlf),
                ("植えた_crlf2.sql", two_crlf),
            ]),
            vec![
                "植えた_crlf.sql: CR 1 バイト".to_owned(),
                "植えた_crlf2.sql: CR 2 バイト".to_owned(),
            ],
            "植えた CRLF を、名前と本数まで挙げること"
        );

        assert_eq!(
            files_with_carriage_return(&[("植えた_lf.sql", lf)]),
            Vec::<String>::new(),
            "LF だけの入力では何も挙げないこと"
        );
    }

    // ------------------------------------------------------------------
    // 純粋関数
    // ------------------------------------------------------------------

    /// **`as_crlf` は `\n` だけを広げ、既に `\r\n` の位置を二重にしない。**
    #[test]
    fn as_crlf_widens_only_bare_line_feeds() {
        assert_eq!(as_crlf("a\nb\n"), "a\r\nb\r\n", "LF は広がること");
        assert_eq!(
            as_crlf("a\r\nb\r\n"),
            "a\r\nb\r\n",
            "既に CRLF の位置は二重にならないこと"
        );
        assert_eq!(
            as_crlf("a\r\nb\nc"),
            "a\r\nb\r\nc",
            "混在でも CRLF に揃うこと"
        );
        assert_eq!(as_crlf("a\rb"), "a\rb", "単独の CR はそのまま残ること");
        assert_eq!(as_crlf(""), "", "空でも壊れないこと");
    }

    /// **LF 版と CRLF 版の checksum は違う。** これが崩れると、この作業単位に意味が無い。
    #[test]
    fn the_two_line_endings_do_not_share_a_checksum() {
        assert_ne!(
            checksum_of("a\n"),
            checksum_of("a\r\n"),
            "植えた文字列: 改行が違えば checksum も違うこと"
        );

        for migration in MIGRATOR.iter() {
            let lf = migration.sql.as_str();
            let crlf = as_crlf(lf);
            assert_ne!(
                lf,
                crlf.as_str(),
                "版 {}: LF で埋め込まれていること",
                migration.version
            );
            assert_ne!(
                checksum_of(lf),
                checksum_of(&crlf),
                "版 {}: 改行違いで checksum が動くこと",
                migration.version
            );
        }
    }

    /// **LF 側の権威** —— `migrate!` が**コンパイル時に**埋めた値と一致する。
    #[test]
    fn the_checksum_matches_what_the_macro_embedded() {
        for migration in MIGRATOR.iter() {
            assert_eq!(
                checksum_of(migration.sql.as_str()),
                migration.checksum.as_ref(),
                "版 {}: マクロが埋めた checksum と一致すること",
                migration.version
            );
        }
    }

    /// **CRLF 側の権威** —— **CRLF のファイルを sqlx 自身に読ませた値**と一致する。
    ///
    /// ここが一致しない限り、付け替えの条件は「実際に起きる形」に当たらない。
    #[test]
    fn the_crlf_checksum_matches_what_sqlx_reads_from_crlf_files() {
        let from_files = runtime().block_on(crlf_checksums_from_files());
        let embedded = embedded_checksums();
        assert_eq!(
            from_files.keys().collect::<Vec<_>>(),
            embedded.keys().collect::<Vec<_>>(),
            "前提: 版の並びが一致すること"
        );

        for migration in MIGRATOR.iter() {
            let computed = checksum_of(&as_crlf(migration.sql.as_str()));
            assert_eq!(
                Some(&computed),
                from_files.get(&migration.version),
                "版 {}: CRLF の実ファイルから sqlx が出す値と一致すること",
                migration.version
            );
            assert_ne!(
                from_files.get(&migration.version),
                embedded.get(&migration.version),
                "前提: 版 {} の CRLF 版と LF 版がずれていること",
                migration.version
            );
        }
    }

    // ------------------------------------------------------------------
    // 決定3 —— 既存 DB の帳簿の付け替え
    // ------------------------------------------------------------------

    /// **CRLF の checkout が作った帳簿を持つ DB が、そのまま開く。**
    #[test]
    fn a_ledger_written_by_a_crlf_checkout_opens() {
        runtime().block_on(async {
            let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
            let path = dir.path().join("personacore.sqlite");
            fresh(&path).await;

            let crlf = crlf_checksums_from_files().await;
            plant(&path, &crlf).await;

            // **前提: 植えた値が現に帳簿へ載っている。**
            let pool = raw_pool(&path).await;
            let planted: BTreeMap<i64, Vec<u8>> = ledger(&pool)
                .await
                .into_iter()
                .map(|row| (row.version, row.checksum))
                .collect();
            pool.close().await;
            assert_eq!(planted, crlf, "前提: 帳簿が CRLF 版になっていること");

            let store = Store::connect(&path)
                .await
                .expect("CRLF 版の帳簿を持つ DB が開けること");
            let after: BTreeMap<i64, Vec<u8>> = ledger(store.pool())
                .await
                .into_iter()
                .map(|row| (row.version, row.checksum))
                .collect();
            store.close().await;

            assert_eq!(after, embedded_checksums(), "帳簿が LF 版に戻っていること");
        });
    }

    /// **本当に編集された SQL は、今までどおり止まる。**
    ///
    /// **この対照が無いと「常に付け替える」実装が緑を通る。**
    #[test]
    fn a_genuinely_edited_migration_still_stops_the_open() {
        runtime().block_on(async {
            let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
            let path = dir.path().join("personacore.sqlite");
            fresh(&path).await;

            // **第3の値** —— LF 版でも CRLF 版でもない。
            let third = vec![0xAB_u8; 48];
            let crlf = crlf_checksums_from_files().await;
            let embedded = embedded_checksums();
            assert_ne!(
                Some(&third),
                crlf.get(&5),
                "前提: 第3の値が CRLF 版と違うこと"
            );
            assert_ne!(
                Some(&third),
                embedded.get(&5),
                "前提: 第3の値が LF 版と違うこと"
            );

            plant(&path, &BTreeMap::from([(5, third.clone())])).await;

            let outcome = Store::connect(&path).await;
            assert!(
                matches!(outcome, Err(StoreError::Migrate(_))),
                "編集された SQL は `Migrate` で止まること: {outcome:?}"
            );

            // **付け替えていないこと**まで見る（止まっただけでは、
            // 「止めてから書き換えた」を区別できない）。
            let pool = raw_pool(&path).await;
            let rows = ledger(&pool).await;
            pool.close().await;
            let fifth = rows
                .iter()
                .find(|row| row.version == 5)
                .expect("版 5 の行があること");
            assert_eq!(fifth.checksum, third, "第3の値は触られないこと");
        });
    }

    /// **新しい DB（帳簿がまだ無い）でも通る。**
    ///
    /// 帳簿の有無を見ずに読みに行く実装だと、**初回の起動が丸ごと止まる。**
    #[test]
    fn a_database_without_a_ledger_still_opens() {
        runtime().block_on(async {
            let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
            let path = dir.path().join("personacore.sqlite");

            // **前提: 触る前は帳簿が無い。**
            assert!(!path.exists(), "前提: DB ファイルがまだ無いこと");

            let store = Store::connect(&path).await.expect("初回の起動が通ること");
            let rows = ledger(store.pool()).await;
            store.close().await;
            assert_eq!(
                rows.len(),
                MIGRATOR.iter().count(),
                "全部の版が適用されること"
            );
        });
    }

    /// **帰属** —— 一致した行**だけ**が動き、他の 12 行は1バイトも触られない。
    #[test]
    fn only_the_row_that_differs_by_line_endings_is_touched() {
        runtime().block_on(async {
            let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
            let path = dir.path().join("personacore.sqlite");
            fresh(&path).await;

            let pool = raw_pool(&path).await;
            let before = ledger(&pool).await;
            pool.close().await;
            assert!(before.len() > 1, "前提: 行が複数あること");

            let crlf = crlf_checksums_from_files().await;
            let fifth_crlf = crlf.get(&5).expect("版 5 の CRLF 版があること").clone();
            plant(&path, &BTreeMap::from([(5, fifth_crlf)])).await;

            let store = Store::connect(&path).await.expect("開けること");
            let after = ledger(store.pool()).await;
            store.close().await;

            // **版 5 は LF 版に戻り、他の欄は動いていない。**
            assert_eq!(after, before, "版 5 以外は1バイトも動かないこと");
        });
    }

    /// **付け替えたことを、既定のログ設定で名乗る。版は欄で見る。**
    ///
    /// # なぜ2つ植えるのか（レビュー1周目の是正）
    ///
    /// **1つ（版 5）だけ植てていたときは、`migration = 5` の決め打ちが 2180 全緑だった。**
    /// **欄は読めていたが、帰属を一切見ていなかった**
    /// （CLAUDE.md「載っているか」の次は「どれのものか」）。
    ///
    /// **実害は `main` の作業ツリーそのもの** —— 混在の checkout で作られた帳簿は
    /// **版 1〜4 と 13 が CRLF** なので、決め打ちのままだと
    /// **`migration=5` が 5 行並ぶ**（どれが動いたのか運用者に分からない）。
    ///
    /// **端を選ばない**とも言い切れないので、**版 5（真ん中）と版 13（末尾）**を植えて、
    /// **行ごとに別の欄を読む。** 片方への決め打ちは、もう片方の行数が 2 になって落ちる。
    #[test]
    fn the_repair_names_the_migration_it_moved() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let path = dir.path().join("personacore.sqlite");
        let runtime = runtime();

        let crlf = runtime.block_on(async {
            fresh(&path).await;
            crlf_checksums_from_files().await
        });

        // **前提（否定側の先出し）: 帳簿を触っていない起動では、この行は出ない。**
        let untouched = captured_under_default_filter(|| {
            runtime.block_on(async {
                let store = Store::connect(&path).await.expect("開けること");
                store.close().await;
            });
        });
        assert!(
            !untouched.contains(REPAIRED_MARKER),
            "付け替える相手が居ないときは名乗らないこと: {untouched:?}"
        );

        let fifth = crlf.get(&5).expect("版 5 の CRLF 版があること").clone();
        let thirteenth = crlf.get(&13).expect("版 13 の CRLF 版があること").clone();
        runtime.block_on(plant(
            &path,
            &BTreeMap::from([(5, fifth), (13, thirteenth)]),
        ));

        let output = captured_under_default_filter(|| {
            runtime.block_on(async {
                let store = Store::connect(&path).await.expect("開けること");
                store.close().await;
            });
        });

        // **行に切ってから欄を見る**（CLAUDE.md「同じ行の中で」）。
        let reported: Vec<&str> = output
            .lines()
            .filter(|line| line.contains(REPAIRED_MARKER))
            .collect();
        assert_eq!(
            reported.len(),
            2,
            "植えた版のぶんだけ名乗ること: {output:?}"
        );
        for line in &reported {
            assert!(
                line.contains("INFO"),
                "既定の設定で運用者の目に入る水準であること: {line:?}"
            );
        }

        // **帰属** —— 版ごとに**別の行**から読む。
        // 片方へ決め打つと、その版の行数が 2 になり、もう片方が 0 になる。
        let fifth_lines = reported
            .iter()
            .filter(|line| has_field(line, "migration=5"))
            .count();
        let thirteenth_lines = reported
            .iter()
            .filter(|line| has_field(line, "migration=13"))
            .count();
        assert_eq!(
            (fifth_lines, thirteenth_lines),
            (1, 1),
            "版 5 と版 13 を、それぞれちょうど1行ずつ名乗ること: {reported:?}"
        );
    }
}
