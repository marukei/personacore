//! 統合テストで共有する小物。
//!
//! **統合テストはファイルごとに別バイナリ**なので、使う側に `mod common;` が要る。
//! `tests/common/` はサブディレクトリなので、それ自体はテストバイナリにならない。

// **`mod common;` は取り込んだバイナリごとに丸ごとコンパイルされる。**
// ここの小物は6本の統合テストのうち一部だけが使うので、使わないバイナリでは
// 必ず「never used」になる。`tests/common` では通常の扱い。
#![allow(dead_code, reason = "取り込むテストバイナリごとに使う小物が違う")]

/// ログの**行**と**欄**を突き合わせる道具**と、ログを溜める器**。
/// **`src/` と同じファイルを読む**（作業単位 B-3a / B-3）。
///
/// # 何が入っているかは、ここに写さない（作業単位 B-37）
///
/// **権威は `src/testlog/assert.rs` の「ここに何が入っているか」の表。**
/// **写しではなく同じファイル**なので、道具が増えたときに直す場所は1つでよい。
///
/// **⚠ この誘導の綴りは機械的に守られていない**（実測: 誘導先を
/// `src/testlog/NOWHERE.rs` に書き換えても全緑）。**見張り番は足さない** ——
/// 走査を1本足すと `15b` の自己成就の罠を自分で持ち込むので割に合わない。
///
/// **⚠ ただし「守られていない範囲」を取り違えないこと。**
/// **下の `#[path]` が同じファイルを指しているので、ファイルが消えたり移動したりすれば
/// コンパイルが止まる。** **無防備なのは「この綴りが `#[path]` と一致していること」だけ**で、
/// 起きうる事故は**「`#[path]` はコンパイラに強制されて直したが、この doc は直し忘れた」の1通り。**
/// **被害は読み手が1分損することで、振る舞いは変わらない。**
///
/// > **⚠ ここには B-37 より前、同じ表の写しがあった。**
/// > **`has_key`（B-21d）と `quoted_field`（B-32）が入っておらず、2つぶん古びていた。**
/// > **表を直すのではなく、表を消した**（CLAUDE.md「同じ発見を2箇所に書かない」）。
/// > **写しは、片方を直しても、もう片方が黙って古びる。**
///
/// **⚠ 下の `pub(crate) use` は、この表とは別物。**
/// あちらは**統合テストから見える面**で、**現に使う道具だけ**を並べてある
/// （理由は再輸出のところに書いた）。**一致していなくてよい。**
///
/// **⚠ `#[path]` は1本のまま。** 2本にすると、下の見張り番が見ているのは1本だけなので、
/// **増やした方は黙って写しへ向け替えられる**（B-12 の判断）。器も同じファイルへ入れてある。
///
/// **⚠ グローバルの subscriber は「1プロセス1回」だが、`install_global_log` は畳めた。**
/// このファイルは**取り込んだテストバイナリごとに別々にコンパイルされる**ので、
/// あちらの `OnceLock` も**バイナリごとに別の静的変数**になる。
/// **畳めないのは「バイナリを4本に分けていること」そのもの**（`src/testlog/assert.rs` の doc）。
///
/// # なぜ `use personacore::…` ではないのか
///
/// `src/testlog` は `#[cfg(test)]` なので、別クレートである統合テストからは見えない。
/// **`testing` フィーチャの下へ出す手は採らない**——dev-dependency の自己参照で
/// `cargo test` / `cargo clippy --all-targets` が `testing` を有効にするため、
/// **本番コードが呼んでも両方が通る**（CLAUDE.md 2節）。
///
/// `#[path]` なら**ライブラリの公開面は1バイトも増えない。**
/// 本番ビルドはこのファイルをコンパイルすらしない。
///
/// # 押さえ（見張り番）は写していない。**同じ文字列だから要らない**
///
/// 行と欄の意味を固定する見張り番は `src/testlog.rs` の `mod tests` にある。
/// **写しではなく同じファイル**なので、あちらが赤くなればこちらの道具も同時に壊れている。
///
/// **その「同じファイルであること」だけ**を
/// `the_integration_tests_share_this_file_instead_of_copying_it` が見張る——
/// 具体的には**2つだけ**:
///
/// 1. 下の `mod testlog_assert;` に**現に効いている `#[path]` の綴り**が共有の1本を指すこと。
/// 2. `tests/**` に、**共有している道具の定義**（関数なら宣言の語＋名前、型も同じ形）が
///    無いこと。**どの綴りを見ているかの権威は `src/testlog.rs` の `SHARED_HELPERS`**
///    ——**ここに並べ直さない**（B-37 より前は写しがあり、2本ぶん古びていた）。
///    **括弧まで読む方の呼び出し元は `tests/server_chat_parse_log.rs`**（漏れた軸の欄）。
///    **器（`SharedBuffer` と `install_global_log`）は、ログを見る統合テスト全部**が呼ぶ。
///    **⚠ ここに本数を書かない**（B-37）——**B-3 の時点の「4つ」のまま古びていた。**
///    **⚠ 欄で行を選ぶ方を `tests/` 側に書き起こさないこと**（作業単位 B-21b）——
///    本文が行を切り出す方の双子なので、**名前を変えて置くと
///    名前の走査にも本文の走査にも掛からない**（`src/testlog.rs` の `SHARED_BODIES` の表）。
///
/// **⚠ 2 は名前しか見ていない。**
/// **名前を変えた写し**（`line_with` のような名前）は**この2つでは捕まらない**
/// （実測 / B-3a のレビュー）。B-13 の時点で
/// `tests/extraction_quality_harness.rs` に**別名の写し**が1本
/// （`field_is_present`）実在した——**走査ではなく人が見つけた。**
///
/// **⚠ B-17 で、名前に依らない見張り番が1本増えた**——
/// `no_integration_test_copies_a_shared_body_under_another_name` が
/// **共有側の本文の断片**を `tests/**` の中に探す（同じく `src/testlog.rs`）。
/// **名前だけ付け替えた写しは、これで落ちる。**
///
/// **⚠ それでも「写しに戻せない」とは読まないこと。**
/// **本文の綴りまで書き直した別名**（受け側や引数の名前を変える／
/// `contains` へ弱く書き直す）は**どちらの走査でも捕まらない。**
/// 捕まえない形の一覧は `src/testlog.rs` の `SHARED_BODIES` の doc にある。
///
/// **⚠ 2 の走査は「宣言の語 ＋ 名前」の綴りを素朴に探すだけ**
/// （関数なら `fn` ＋ 名前、型なら `struct` ＋ 名前）**で、コメントと本文を区別しない。**
/// **この doc でその2語を続けて綴ると、この走査自身が赤くなる**（着手時に実際に踏んだ /
/// CLAUDE.md「走査系のテストが増えると、互いのフィクスチャで誤爆する」）。
/// **緩めない。** 言及するときは上のように**語を分けて**書くこと。
#[path = "../../src/testlog/assert.rs"]
mod testlog_assert;

// **ログを見ない統合テストの方が多い。** `#![allow(dead_code)]` は
// 再エクスポートの「未使用」までは黙らせないので、ここだけ名指しで許す
// （実測: この `allow` を外すと、道具を使わないテストバイナリが `unused imports` で
// `-D warnings` に当たって落ちる。**⚠ 本数は書かない**——打ち切りで止まるので、
// 見えた数は「そこまで進んだ数」でしかない）。
//
// **⚠ 共有ファイルにある道具が全部ここに並ぶわけではない**（作業単位 B-37）。
// **`quoted_field` は、統合テストの側にまだ呼び手がいないので並べていない。**
// **`has_key` は作業単位 B-46 の是正で呼び手ができた**（`tests/server_chat_parse_log.rs` の欄名の衝突の見張り番）。
//
// **⚠ この行は「死んだ道具の検出器」ではない。** 実測で2通りとも確かめた:
//
// * **使わない名前を足しても鳴らない** ——上の `allow` が黙らせる
//   （2本足して `cargo clippy --all-targets -- -D warnings` が exit 0）。
// * **鳴るのは「呼び手がいる名前を外したとき」だけ**で、それは lint ではなく
//   コンパイルエラー（`bracketed_field` を外すと
//   `tests/server_chat_parse_log.rs` が `E0432` で落ちる）。
//
// **呼び手ができた日に足すこと。** 先に並べても、誰も見ていない名前が増えるだけ。
#[allow(unused_imports, reason = "取り込むテストバイナリごとに使う小物が違う")]
pub(crate) use testlog_assert::{
    SharedBuffer, bracketed_field, has_field, has_key, install_global_log, line_containing,
    line_with_field,
};

/// **校正の器具の「走らせなくても確かめられる部分」**（作業単位 E-3）。
///
/// 使うのは3本:
///
/// | ファイル | 役目 |
/// |---|---|
/// | `tests/prompt_estimate_calibration.rs` | 器具の本体。**4本とも `#[ignore]`**（LM Studio が要る） |
/// | `tests/calibration_target.rs` | 対象の決め方・貼れる形の見張り番（**参照関数を渡す**ので環境に触らない） |
/// | `tests/calibration_env_wiring.rs` | **環境を読む1行**の見張り番。**テスト1本だけの専用バイナリ** |
///
/// **見張り番を器具と同じバイナリに置けない**のは、器具が**素の `cargo test` では
/// 1本も走らない**ため。**3本目が別なのは、あれだけがプロセスの環境を書き換えるから**
/// （理由はそのファイルの doc）。
pub mod calibration;

use chrono::TimeDelta;
use personacore::domain::{CharacterId, StatScale};
use personacore::store::Store;

/// DB が書いた日時と `Utc::now()` を比べるときに許すずれ。
///
/// # なぜ必要か
///
/// **SQLite の `strftime('now')` と Rust の `Utc::now()` は別の時計を読んでいる。**
///
/// | | Windows での実体 | 観測された最小ステップ |
/// |---|---|---|
/// | SQLite `'now'` | `GetSystemTimeAsFileTime`（粗い） | 1 ms |
/// | chrono `Utc::now()` | `GetSystemTimePreciseAsFileTime` | 0.0001 ms |
///
/// 実測で **SQLite 側が最大 2.73ms 遅れる**。したがって
/// `Utc::now()` を挟んで `created_at` の範囲を見る assert は、
/// マージンが無いと**確率的に落ちる**（マージン 0ms の
/// `store_character::create_then_find_then_list` は実測 200 回中 15 回失敗した）。
///
/// # なぜ 1 ms や 10 ms では足りないか
///
/// **2.73ms は上限ではない。** SQLite 側の粒度は Windows のシステムタイマ分解能に従い、
/// 1ms 刻みで観測できたのは**無関係な別プロセスが分解能を上げていたから**にすぎない。
/// 誰も上げていない既定状態では **15.6ms 刻み**になる。
/// つまり**このフレークの発生率は、テスト対象と無関係な他アプリの挙動で変わる。**
/// 「手元で通ったから安全」と言えないので、桁で上回るマージンを取る。
///
/// # 緩めても検出力は落ちない
///
/// この種の assert の目的は「`created_at` がエポックでも 2999 年でもない」という
/// 健全性の確認であり、秒単位のずれを見張ることではない
/// （日時の**順序**を見たいテストは、この定数ではなく行同士を直接比較すること）。
///
/// **上限側にも対称に足す。** 現状 SQLite 側が先行した観測は無いが、
/// 片側だけに足すと「なぜ下限だけ緩いのか」が読み手に伝わらない。
pub const CLOCK_SKEW: TimeDelta = TimeDelta::seconds(5);

/// **CHECK 制約を無効にしたまま `character` に1行入れる。**
///
/// # なぜこんな抜け穴が要るのか
///
/// 0004b-1 で `character.handle` に文字集合の CHECK が入り、
/// ゴースト行（`" aoi "` のような正規化されていない名前の行）は
/// **アプリ層からも素の SQL からも書けなくなった。**
///
/// **それでも「もう存在しない」とは書けない。**（0004b-1 のレビューで撤回）
/// 正規化キーの機構が担っていた脅威モデルは ADR-0004 の
/// **「DB を直接編集された場合の担保」**であり、**CHECK はそれを閉じていない。**
/// 実測（2026-08-16 / sqlite 3.53.4）——ゴースト行は1行で作れる:
///
/// ```text
/// sqlite3 ghost.sqlite "PRAGMA ignore_check_constraints = ON; \
///     INSERT INTO character (handle, display_name, base_persona) VALUES (' aoi ', '葵', 'p')"
/// ```
///
/// このヘルパはその1行を Rust から再現する。**ゴースト行は今も作れる**ので、
/// 「作れないから機構を捨てられる」という論法は使えない。
///
/// # **0004b-2 が機構を捨てた根拠**（ロードマップがここを正としている）
///
/// 捨ててよい理由は「作れないから」ではなく **「作られても照合が当たらないから」**。
/// `Handle::parse` が trim をやめた（ADR-0017）結果、ゴースト行は
/// **どの経路からも引き当てられない。** 実測（2026-08-16。ゴースト行 `" aoi "` を1件だけ置いた状態）:
///
/// ```text
/// Handle::parse(" aoi ")      → Err(Charset)     ← `model` の綴りとして表現できない
/// find_character_by_handle("aoi")      → None             ← チャットの引き当てに当たらない
/// list_characters()                  → Unreadable { id: 1, handle: None, invalid: [Handle] }
/// GET /v1/models                     → {"data":[]}      ← 選べない
/// GET /admin/characters              → {"characters":[],
///                                       "unreadable":[{"id":1,"handle":null,
///                                                      "invalid_columns":["handle"]}]}
/// POST /v1/chat/completions "aoi"    → 404 model_not_found
/// POST /v1/chat/completions " aoi "  → 400 character_handle_charset
/// ```
///
/// ADR-0004 が「最も危険」とした
/// **「照合で両方に一致して書き込み先が不定になる」状態が原理的に起きない。**
/// 存在する行は一覧が「読めない行」として id で指し示す（0012b）。
///
/// # **索引と `UPDATE` / `DELETE` の `trim` は一組**（片方だけ落とせない）
///
/// 索引だけを落として `trim` を残すと、実測でこうなる（同日 / 同版）:
///
/// ```text
/// rows: 1|'aoi'（本物） 2|' aoi '（ゴースト）
/// find_character_by_handle("aoi") → Some(1)              ← 照合は当たり続ける（無害）
/// update_character("aoi")       → Ok(CharacterId(2))   ← **返るのはゴーストの id**
/// rows after update = [(1,'aoi','巻き添え'), (2,' aoi ','巻き添え')]
/// delete_character("aoi")       → Ok(())
/// rows after delete = []                               ← **本物ごと消えた**
/// ```
///
/// **索引が防いでいたどの事故よりも重い。** だから 0004b-2 は
/// 索引・`SELECT_BY_NORMALIZED_KEY`・`ConflictingUnnormalizedName`・
/// `UPDATE` / `DELETE` の `trim` を**まとめて**捨てている。
///
/// # 捨てたことで失われたもの（承知のうえ）
///
/// ゴースト行を **API から掃除する手段が無くなった。**
/// ただしその導線は **0004b-1 の時点で既に切れていた**——一覧は `handle: null` を返すので、
/// 運用者は掃除に打つべき綴りを API からは知りようがない（上の実測）。
/// また `trim` が拾えたのは `TRIMMABLE` の8文字で埋まった行だけで、
/// `"Aoi"` や `"葵"` の行には最初から届いていない。
/// **ゴースト行を作れるのは DB を直接編集した者だけで、同じ `sqlite3` で消せる。**
///
/// # 仕掛け
///
/// `PRAGMA ignore_check_constraints` は**接続ごと**の設定なので、プールから1本借りて
/// その接続だけで立てて倒す（`store.pool()` に直接投げると別の接続に当たりうる）。
/// **UNIQUE 索引は無効にならない**（無効化されるのは CHECK だけ）ので、
/// `character.handle` の完全一致の重複はこの経路でも弾かれる。
///
/// 返すのは INSERT の結果そのまま。**成功を前提にしないこと。**
pub async fn insert_character_ignoring_checks(
    store: &Store,
    name: &str,
    display_name: &str,
    base_persona: &str,
    output_format_instruction: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let mut conn = store
        .pool()
        .acquire()
        .await
        .expect("プールから接続を取得できること");

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を一時的に無効にできること");

    let result = sqlx::query(
        "INSERT INTO character (handle, display_name, base_persona, output_format_instruction) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(name)
    .bind(display_name)
    .bind(base_persona)
    .bind(output_format_instruction)
    .execute(&mut *conn)
    .await;

    // **必ず戻す。** 接続はプールへ返されて他のテストが引き当てる。
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を戻せること");

    assert_checks_are_back_on(&mut conn).await;

    result.map(|done| done.last_insert_rowid())
}

/// **抜け穴を閉めたことを、閉めた接続の上で確かめる。**
///
/// # なぜ要るのか
///
/// 上の `PRAGMA ignore_check_constraints = OFF` を**丸ごと消しても、統合テストは
/// 1本も落ちなかった**（0004b-1 のレビューで実測）。消えると
/// **CHECK を切ったままの接続がプールへ返り**、以後「CHECK で弾かれること」を
/// 見ているテストが**静かに無効化される。**
/// 今日たまたま無事なのは、CHECK に依存するループがこのヘルパの呼び出しより
/// **前に置かれているから**にすぎず、**並べ替えただけで壊れる。**
///
/// # なぜ INSERT を1本投げるのか
///
/// **`PRAGMA ignore_check_constraints` は値を返さない**（読み取り形式が存在しない）ので、
/// 状態を問い合わせて確かめる手段が無い。**実際に弾かれることを見るしかない。**
///
/// 位置がここ（接続をプールへ返す直前）なのは、**この接続にしか当たらないから。**
/// 他のテストが同時に走っていても結果が揺れない＝決定的になる。
///
/// 投げるのは `character.handle` の文字集合の CHECK（0005 / ADR-0017）に触れる行。
/// **CHECK が生きていれば必ず失敗する**ので、DB には何も残らない。
async fn assert_checks_are_back_on(conn: &mut sqlx::SqliteConnection) {
    let err =
        sqlx::query("INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)")
            .bind(" CHECK が効いていない ")
            .bind("葵")
            .bind("ペルソナ本文")
            .execute(&mut *conn)
            .await
            .expect_err("CHECK を戻した接続では、規則外の識別子が弾かれること");

    assert_eq!(
        err.as_database_error()
            .map(sqlx::error::DatabaseError::kind),
        Some(sqlx::error::ErrorKind::CheckViolation),
        "抜け穴を閉めずに接続をプールへ返しています（以後の CHECK 検査が黙って無効になります）: {err}"
    );
}

/// **識別子だけが規則外の行**を1件作る。主キーを返す。
///
/// 表示名とペルソナは正当な値で埋めるので、`list_characters` が指し示す
/// `invalid_columns` は `Name` だけになる。
///
/// **識別子と表示名を同じ文字列にしないこと**（同値だと、読み出しが取り違えていても
/// 気付けない。CLAUDE.md「フィクスチャが2つの値を同値にすると…」）。
pub async fn insert_ghost_character(store: &Store, name: &str) -> i64 {
    insert_ghost_character_with(store, name, "葵", "ペルソナ本文").await
}

/// 表示名・ペルソナまで指定するゴースト行。
pub async fn insert_ghost_character_with(
    store: &Store,
    name: &str,
    display_name: &str,
    base_persona: &str,
) -> i64 {
    insert_character_ignoring_checks(store, name, display_name, base_persona, None)
        .await
        .unwrap_or_else(|err| panic!("ゴースト行を作れること: name = {name:?}: {err}"))
}

/// **CHECK 制約を無効にしたまま、既存行の `stat_scale` を書き換える。**
///
/// # なぜ要るのか
///
/// `character.stat_scale` はスキーマ側にも `BETWEEN 1 AND 100` の CHECK があるので、
/// **範囲外の値を持つ行は普通の SQL では作れない。** それでも
/// `domain::StatScale` の検証を読み出し側でも通しているのは、
/// **CHECK が `PRAGMA ignore_check_constraints` で外せる**から
/// （`insert_character_ignoring_checks` と同じ手。ゴースト行が今も作れるのと同じ話）。
///
/// **「作れないから検証は要らない」という論法は使えない。** この関数はその1行を再現する。
///
/// 仕掛けは `insert_character_ignoring_checks` と同じで、
/// 抜け穴は同じ接続の上で閉め、閉まったことを確かめてから返す。
pub async fn set_stat_scale_ignoring_checks(store: &Store, id: i64, stat_scale: i64) {
    let mut conn = store
        .pool()
        .acquire()
        .await
        .expect("プールから接続を取得できること");

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を一時的に無効にできること");

    let result = sqlx::query("UPDATE character SET stat_scale = ? WHERE id = ?")
        .bind(stat_scale)
        .bind(id)
        .execute(&mut *conn)
        .await;

    // **必ず戻す。** 接続はプールへ返されて他のテストが引き当てる。
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を戻せること");

    assert_checks_are_back_on(&mut conn).await;

    let done = result.unwrap_or_else(|err| panic!("尺度を書き換えられること: id = {id}: {err}"));
    assert_eq!(
        done.rows_affected(),
        1,
        "書き換え対象がちょうど1件であること: id = {id}"
    );
}

/// **CHECK 制約を無効にしたまま、既存行の `stat_drift_mode` を書き換える**（作業単位 A-8）。
///
/// # なぜ要るのか
///
/// `character.stat_drift_mode` には `off` / `shadow` / `on` の CHECK がある
/// （`migrations/0014_character_stat_drift_mode.sql`）ので、**規則外の綴りを持つ行は
/// 普通の SQL では作れない。** それでも読み出し側で `StatDriftMode::parse` を通しているのは、
/// **CHECK が外せる**から（[`set_stat_scale_ignoring_checks`] と同じ話）。
///
/// 仕掛けは [`set_stat_scale_ignoring_checks`] と同じで、
/// 抜け穴は同じ接続の上で閉め、閉まったことを確かめてから返す。
///
/// **値の型を選べる**（作業単位 A-8 のレビュー指摘）—— `&str` だけでなく、
/// 整数・小数・BLOB も渡せる。**ただし列は TEXT 宣言なので、整数と小数は
/// 型アフィニティで TEXT に変換されて入る**（実測 / 2026-09-26）。
/// **変換されずに残るのは BLOB だけ**（`typeof` で確かめること）。
pub async fn set_stat_drift_mode_ignoring_checks<'q, T>(store: &Store, id: i64, stored: T)
where
    T: 'q + Send + sqlx::Encode<'q, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>,
{
    let mut conn = store
        .pool()
        .acquire()
        .await
        .expect("プールから接続を取得できること");

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を一時的に無効にできること");

    let result = sqlx::query("UPDATE character SET stat_drift_mode = ? WHERE id = ?")
        .bind(stored)
        .bind(id)
        .execute(&mut *conn)
        .await;

    // **必ず戻す。** 接続はプールへ返されて他のテストが引き当てる。
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を戻せること");

    assert_checks_are_back_on(&mut conn).await;

    let done =
        result.unwrap_or_else(|err| panic!("走らせ方を書き換えられること: id = {id}: {err}"));
    assert_eq!(
        done.rows_affected(),
        1,
        "書き換え対象がちょうど1件であること: id = {id}"
    );
}

/// **CHECK 制約を無効にしたまま、既存行の `axis_discovery_slots` を書き換える**（作業単位 A-9a）。
///
/// `character.axis_discovery_slots` には整数・0〜4 の CHECK がある
/// （`migrations/0015_stat_axis_candidate.sql`）ので、**範囲外の値を持つ行は普通の SQL では作れない。**
/// それでも読み出し側で `AxisDiscoverySlots::parse` を通しているのは **CHECK が外せる**から
/// （[`set_stat_scale_ignoring_checks`] と同じ話・同じ仕掛け）。
pub async fn set_axis_discovery_slots_ignoring_checks(store: &Store, id: i64, slots: i64) {
    let mut conn = store
        .pool()
        .acquire()
        .await
        .expect("プールから接続を取得できること");

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を一時的に無効にできること");

    let result = sqlx::query("UPDATE character SET axis_discovery_slots = ? WHERE id = ?")
        .bind(slots)
        .bind(id)
        .execute(&mut *conn)
        .await;

    // **必ず戻す。** 接続はプールへ返されて他のテストが引き当てる。
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を戻せること");

    assert_checks_are_back_on(&mut conn).await;

    let done = result.unwrap_or_else(|err| panic!("枠の数を書き換えられること: id = {id}: {err}"));
    assert_eq!(
        done.rows_affected(),
        1,
        "書き換え対象がちょうど1件であること: id = {id}"
    );
}

/// テストで使う性格ステータスの尺度。**必ず既定（`StatScale::DEFAULT`）と違う値。**
///
/// # なぜ既定を使ってはいけないのか
///
/// 既定にすると、**要求の尺度を捨てて常に `StatScale::DEFAULT` を返す実装**が
/// そのまま緑を通る（CLAUDE.md「フィクスチャが2つの値を同値にすると、
/// その取り違えは検出できない」）。同じ穴は `name` と `display_name`、
/// 照会するモデル id、`plan.prompt_tokens()` と `fixed_tokens()` で繰り返し出ている。
///
/// **「省略時は既定」を確かめるテストだけが `StatScale::DEFAULT` を期待値にしてよい。**
/// それ以外のフィクスチャはこの関数を通すこと。
///
/// 入口で `assert_ne!` を置いてあるので、**この値を 10 に変えた瞬間に
/// 全テストが落ちる**（黙って既定に戻る経路が無い）。
pub fn fixture_scale() -> StatScale {
    let scale = StatScale::parse(7).expect("正当な尺度であること");
    assert_ne!(
        scale,
        StatScale::DEFAULT,
        "フィクスチャの尺度は既定と違う値であること（同値だと『常に既定を返す』実装を検出できない）"
    );
    scale
}

/// **CHECK 制約を無効にしたまま `stat_alloc` に1行入れる。**
///
/// # なぜ要るのか
///
/// `stat_alloc.value` には `typeof(value) = 'integer' AND value >= 0` の CHECK があるので、
/// **負の値を持つ行は普通の SQL では作れない**（`migrations/0001_initial.sql`）。
/// それでも読み出し側で `u32::try_from` を通しているのは、
/// **CHECK が `PRAGMA ignore_check_constraints` で外せる**から。
/// この関数はその1行を再現する。
///
/// **尺度を超えるだけの値（`11/10` など）にこの関数は要らない。**
/// 尺度と値を結ぶ制約はスキーマに無いので、素の INSERT で作れる
/// （`a_stored_value_beyond_the_characters_scale_stops_the_read` が実際にそうしている）。
/// ここが要るのは**下限の CHECK に触れる値**だけ。
///
/// 仕掛けは `insert_character_ignoring_checks` と同じで、
/// 抜け穴は同じ接続の上で閉め、閉まったことを確かめてから返す。
pub async fn insert_stat_alloc_ignoring_checks(
    store: &Store,
    character_id: i64,
    axis: &str,
    value: i64,
) {
    let mut conn = store
        .pool()
        .acquire()
        .await
        .expect("プールから接続を取得できること");

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を一時的に無効にできること");

    let result = sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(character_id)
        .bind(axis)
        .bind(value)
        .execute(&mut *conn)
        .await;

    // **必ず戻す。** 接続はプールへ返されて他のテストが引き当てる。
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を戻せること");

    assert_checks_are_back_on(&mut conn).await;

    let done = result
        .unwrap_or_else(|err| panic!("性格ステータスの行を作れること: {axis} = {value}: {err}"));
    assert_eq!(done.rows_affected(), 1, "1行だけ入ること: {axis} = {value}");
}

/// **外部キーを無効にしたまま `character` の行を1つ消す**（親の居ない `stat_alloc` を作る）。
///
/// # 「外部キーが有効だから孤児は作れない」は成り立たない
///
/// `PRAGMA foreign_keys` は**接続ごと**の設定で、`Store::connect` が
/// 接続時に立てているだけなので、**プールから借りた接続の上で倒せる**
/// （`insert_character_ignoring_checks` が `ignore_check_constraints` でやるのと同じ手）。
/// 倒したまま親を消すと `ON DELETE CASCADE` も走らないので、
/// **子だけが残る。** 脅威モデルは他の抜け穴と同じ「DB を直接編集された場合の担保」で、
/// このリポジトリが `insert_ghost_character` / `set_stat_scale_ignoring_checks` で
/// 一貫して守ってきたもの。
///
/// # 戻し方の確認が CHECK 側より厳しい
///
/// `PRAGMA foreign_keys` は**読み取り形式がある**（`ignore_check_constraints` には無い）ので、
/// 値そのものを確かめられる。**それでも実際に弾かれることまで見る**——
/// 値が 1 でも、`PRAGMA` がトランザクション中で無視されていた等の理由で
/// 実際の強制が効いていない可能性を、値の読みだけでは排除できない。
pub async fn delete_character_ignoring_foreign_keys(store: &Store, id: i64) {
    let mut conn = store
        .pool()
        .acquire()
        .await
        .expect("プールから接続を取得できること");

    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await
        .expect("外部キーを一時的に無効にできること");

    let result = sqlx::query("DELETE FROM character WHERE id = ?")
        .bind(id)
        .execute(&mut *conn)
        .await;

    // **必ず戻す。** 接続はプールへ返されて他のテストが引き当てる。
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await
        .expect("外部キーを戻せること");

    assert_foreign_keys_are_back_on(&mut conn).await;

    let done = result.unwrap_or_else(|err| panic!("親の行を消せること: id = {id}: {err}"));
    assert_eq!(
        done.rows_affected(),
        1,
        "削除対象がちょうど1件であること: id = {id}"
    );
}

/// **抜け穴を閉めたことを、閉めた接続の上で確かめる**（`assert_checks_are_back_on` と同じ役目）。
///
/// 値の読みと、実際に弾かれることの両方を見る。投げるのは
/// **親の居ない `stat_alloc`** で、外部キーが効いていれば必ず失敗するので DB には何も残らない。
async fn assert_foreign_keys_are_back_on(conn: &mut sqlx::SqliteConnection) {
    let enabled: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&mut *conn)
        .await
        .expect("PRAGMA foreign_keys を読めること");
    assert_eq!(
        enabled, 1,
        "外部キーを戻さずに接続をプールへ返しています（以後の外部キー検査が黙って無効になります）"
    );

    let err = sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(i64::MAX)
        .bind("親のいない軸")
        .bind(0_i64)
        .execute(&mut *conn)
        .await
        .expect_err("外部キーを戻した接続では、親の居ない行が弾かれること");

    assert_eq!(
        err.as_database_error()
            .map(sqlx::error::DatabaseError::kind),
        Some(sqlx::error::ErrorKind::ForeignKeyViolation),
        "外部キーの強制が効いていません（値は 1 でも弾かれていません）: {err}"
    );
}

/// **CHECK 制約を無効にしたまま `character_attribute` に1行入れる。**
///
/// # なぜ要るのか
///
/// `character_attribute` の `key` / `value` にはスキーマ側にも
/// 「空白のみでない・上限文字数以内・TEXT 型・NUL を含まない」の CHECK があるので、
/// **読み出しで弾かれる行は普通の SQL では作れない**
/// （`migrations/0009_character_attribute.sql`）。
/// それでも読み出し側で `AttributeKey::parse` / `AttributeValue::parse` を通しているのは、
/// **CHECK が `PRAGMA ignore_check_constraints` で外せる**から
/// （`insert_character_ignoring_checks` と同じ手）。
///
/// **「作れないから読み出しの検証は要らない」という論法は使えない。**
/// この関数はその1行を再現し、`PUT .../attributes` で直せることまで測れるようにする。
///
/// `subject` の綴りも検証しないので、**未知の subject を持つ行**もここから作れる。
///
/// 仕掛けは `insert_character_ignoring_checks` と同じで、
/// 抜け穴は同じ接続の上で閉め、閉まったことを確かめてから返す。
pub async fn insert_character_attribute_ignoring_checks(
    store: &Store,
    character_id: i64,
    subject: &str,
    key: &str,
    value: &str,
) {
    let mut conn = store
        .pool()
        .acquire()
        .await
        .expect("プールから接続を取得できること");

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を一時的に無効にできること");

    let result = sqlx::query(
        "INSERT INTO character_attribute (character_id, subject, key, value) VALUES (?, ?, ?, ?)",
    )
    .bind(character_id)
    .bind(subject)
    .bind(key)
    .bind(value)
    .execute(&mut *conn)
    .await;

    // **必ず戻す。** 接続はプールへ返されて他のテストが引き当てる。
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を戻せること");

    assert_checks_are_back_on(&mut conn).await;

    let done = result.unwrap_or_else(|err| panic!("属性の行を作れること: {key} = {value}: {err}"));
    assert_eq!(done.rows_affected(), 1, "1行だけ入ること: {key} = {value}");
}

/// **CHECK 制約を無効にしたまま属性の履歴に1行入れる**（作業単位 A-3a）。
///
/// 理由は `insert_character_attribute_ignoring_checks` と同じ——履歴の表にも
/// `key` / `before` / `after` の CHECK があるので、**読み出しで弾かれる行は
/// 普通の SQL では作れない。** それでも読み出しで `parse` を通しているのは
/// **CHECK が `PRAGMA` で外せる**からで、その1行をここで再現する。
///
/// `before` / `after` は `None` で NULL（**無かった / 消えた**）を作れる。
pub async fn insert_attribute_revision_ignoring_checks(
    store: &Store,
    character_id: i64,
    subject: &str,
    key: &str,
    before: Option<&str>,
    after: Option<&str>,
) {
    let mut conn = store
        .pool()
        .acquire()
        .await
        .expect("プールから接続を取得できること");

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を一時的に無効にできること");

    let result = sqlx::query(
        "INSERT INTO attribute_revision (character_id, subject, key, before, after) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(character_id)
    .bind(subject)
    .bind(key)
    .bind(before)
    .bind(after)
    .execute(&mut *conn)
    .await;

    // **必ず戻す。** 接続はプールへ返されて他のテストが引き当てる。
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を戻せること");

    assert_checks_are_back_on(&mut conn).await;

    let done = result.unwrap_or_else(|err| panic!("履歴の行を作れること: {key}: {err}"));
    assert_eq!(done.rows_affected(), 1, "1行だけ入ること: {key}");
}

/// **CHECK 制約を無効にしたまま `memory` に1行入れる。** 主キーを返す。
///
/// # なぜ要るのか
///
/// `memory` には `subject IN ('user', 'character')` と `locked IN (0, 1)` の CHECK があるので、
/// **その2列が壊れた行は普通の SQL では作れない。** それでも読み出しで検証しているのは、
/// **CHECK が `PRAGMA ignore_check_constraints` で外せる**から
/// （`insert_character_ignoring_checks` と同じ手）。
///
/// **本文・分類が壊れた行にこの関数は要らない**（長さと空白の制約はスキーマに無いので
/// 素の INSERT で作れる）。ここが要るのは CHECK に触れる値だけ。
///
/// **`locked = 1` のような正当な値にも使える**（抜け穴は開くが、入る行は健全）。
/// `store` に `locked` を立てる API が無い（ADR-0022 §4 / 手動運用の想定）ため、
/// 「保護印の付いた健全な行」を作る唯一の手段でもある。
///
/// # 置き場所が `tests/common` である理由
///
/// **写しを作らないため。** 14a では `tests/store_memory.rs` の中に置いていたが、
/// 14b で `tests/server_admin_memories.rs` も同じ行を要る。2つ持つと、
/// 片方の抜け穴の閉め方だけが黙って劣化する（この関数の後半がまさにその検査）。
///
/// 仕掛けは `insert_character_ignoring_checks` と同じで、
/// 抜け穴は同じ接続の上で閉め、閉まったことを確かめてから返す。
pub async fn insert_memory_ignoring_checks(
    store: &Store,
    character_id: CharacterId,
    subject: &str,
    content: &str,
    category: Option<&str>,
    locked: i64,
) -> i64 {
    let mut conn = store
        .pool()
        .acquire()
        .await
        .expect("プールから接続を取得できること");

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を一時的に無効にできること");

    let result = sqlx::query(
        "INSERT INTO memory (character_id, subject, content, category, locked) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(character_id.get())
    .bind(subject)
    .bind(content)
    .bind(category)
    .bind(locked)
    .execute(&mut *conn)
    .await;

    // **必ず戻す。** 接続はプールへ返されて他のテストが引き当てる。
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を戻せること");

    // **抜け穴を閉めたことを、閉めた接続の上で確かめる**（`assert_checks_are_back_on` と同じ役目）。
    // `PRAGMA ignore_check_constraints` は読み取り形式が無いので、実際に弾かれることを見る。
    // 投げるのは `memory.subject` の CHECK に触れる行なので、DB には何も残らない。
    let err = sqlx::query("INSERT INTO memory (character_id, subject, content) VALUES (?, ?, ?)")
        .bind(character_id.get())
        .bind("assistant")
        .bind("CHECK が効いていない")
        .execute(&mut *conn)
        .await
        .expect_err("CHECK を戻した接続では、規則外の subject が弾かれること");
    assert_eq!(
        err.as_database_error()
            .map(sqlx::error::DatabaseError::kind),
        Some(sqlx::error::ErrorKind::CheckViolation),
        "抜け穴を閉めずに接続をプールへ返しています（以後の CHECK 検査が黙って無効になります）: {err}"
    );

    let done = result.unwrap_or_else(|err| panic!("記憶の行を作れること: {err}"));
    done.last_insert_rowid()
}

/// **CHECK 制約を無効にしたまま `stat_drift_vote` に1行入れる。**
///
/// # なぜ要るのか
///
/// 票の表には `delta IN ('up', 'down')` と、軸名の空白・NUL・型の CHECK があるので、
/// **その2列が壊れた行は普通の SQL では作れない**
/// （`migrations/0010_stat_drift_vote.sql`）。
/// それでも `Store::recent_drift_votes` が読み出しで検証しているのは、
/// **CHECK が `PRAGMA ignore_check_constraints` で外せる**から
/// （`insert_character_ignoring_checks` と同じ手）。
///
/// **「作れないから読み出しの検証は要らない」という論法は使えない。**
/// この関数はその1行を再現し、蓄積の読みが**どう止まるか**を測れるようにする。
///
/// `created_at` を明示で受け取るので、**同着の組**も作れる
/// （並びの第2キーが効いていることを見るのに要る）。
///
/// 仕掛けは `insert_character_ignoring_checks` と同じで、
/// 抜け穴は同じ接続の上で閉め、閉まったことを確かめてから返す。
pub async fn insert_drift_vote_ignoring_checks(
    store: &Store,
    character_id: i64,
    session_id: i64,
    axis: &str,
    delta: &str,
    created_at: &str,
) -> i64 {
    let mut conn = store
        .pool()
        .acquire()
        .await
        .expect("プールから接続を取得できること");

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を一時的に無効にできること");

    let result = sqlx::query(
        "INSERT INTO stat_drift_vote (character_id, session_id, axis, delta, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(character_id)
    .bind(session_id)
    .bind(axis)
    .bind(delta)
    .bind(created_at)
    .execute(&mut *conn)
    .await;

    // **必ず戻す。** 接続はプールへ返されて他のテストが引き当てる。
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を戻せること");

    assert_checks_are_back_on(&mut conn).await;

    let done = result.unwrap_or_else(|err| panic!("票の行を作れること: {axis} = {delta}: {err}"));
    done.last_insert_rowid()
}

/// **累積の行を CHECK 制約を無視して1件作る**（`stat_drift_charge` / 作業単位 16e-b）。
///
/// **正規の経路では入らない値**（軸名が空・累積が有限でない）を植えるための穴で、
/// **「1行壊れると読みが止まる」を実挙動で見るためだけ**にある。
///
/// `insert_drift_vote_ignoring_checks` と同じ作法——
/// **`PRAGMA` は必ず戻す**（接続はプールへ返って他のテストが引き当てる）。
/// **値はプレースホルダで渡す**（CLAUDE.md 絶対制約。テストの土台でも文字列連結しない）。
pub async fn insert_drift_charge_ignoring_checks(
    store: &Store,
    character_id: i64,
    axis: &str,
    charge: f64,
) {
    let mut conn = store
        .pool()
        .acquire()
        .await
        .expect("プールから接続を取得できること");

    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を一時的に無効にできること");

    let result =
        sqlx::query("INSERT INTO stat_drift_charge (character_id, axis, charge) VALUES (?, ?, ?)")
            .bind(character_id)
            .bind(axis)
            .bind(charge)
            .execute(&mut *conn)
            .await;

    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *conn)
        .await
        .expect("CHECK 制約を戻せること");

    assert_checks_are_back_on(&mut conn).await;

    result.unwrap_or_else(|err| panic!("累積の行を作れること: {axis} = {charge}: {err}"));
}

/// **`updated_at` を明示して**累積の行を1件置く（作業単位 D-5a）。
///
/// # なぜ本番の書き込み口（`Store::save_drift_charges`）で作らないのか
///
/// あちらは `updated_at` を `strftime(..., 'now')` に任せるので、
/// **軸ごとに違う時刻も、過去の時刻も作れない。**
/// **「どの軸の時刻か」を取り違える実装は、全部の時刻が同じ瞬間だと観測できない**
/// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
///
/// **CHECK は外さない。** 軸名も累積も正当な値を渡すこと
/// ——壊れた軸名・有限でない累積が要るなら
/// [`insert_drift_charge_ignoring_checks`] を使う（そちらは時刻を選べない）。
///
/// **`updated_at` に CHECK は無い**ので、時刻として読めない綴りもこの関数で置ける
/// （**その状態を作る唯一の手段**であり、`Store::drift_charges` が止まることの権威に要る）。
///
/// **値はプレースホルダで渡す**（CLAUDE.md 絶対制約。テストの土台でも文字列連結しない）。
pub async fn insert_drift_charge_at(
    store: &Store,
    character_id: i64,
    axis: &str,
    charge: f64,
    updated_at: &str,
) {
    sqlx::query(
        "INSERT INTO stat_drift_charge (character_id, axis, charge, updated_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(character_id)
    .bind(axis)
    .bind(charge)
    .bind(updated_at)
    .execute(store.pool())
    .await
    .unwrap_or_else(|err| panic!("累積の行を作れること: {axis} = {charge} @ {updated_at}: {err}"));
}

/// **`created_at` と `superseded_by` を明示して**`memory` に1行入れる。主キーを返す。
///
/// # なぜ本番の書き込み口（`supersede_memory`）で作らないのか
///
/// あちらは `created_at` をスキーマの DEFAULT（現在時刻）に任せるので、
/// **同時刻の組も、時刻の逆順に入る行も作れない。**
/// 上書き済みの一覧の第2キー（`id`）が効くのは同着のときだけなので、
/// この状態が無いと `ORDER BY created_at DESC, id DESC` の第2キーは
/// **どの向きでも緑**になる（作業単位 D-3b）。
/// 上限を超える件数を積むのにも要る（訂正を100回叩くより速い）。
///
/// **CHECK は外さない。** `subject` は正当な綴りを渡すこと
/// ——壊れた `subject` / `locked` の行が要るなら
/// [`insert_memory_ignoring_checks`] を使う（そちらは後継を指せない）。
/// 本文・分類の壊れ方は制約がスキーマに無いので、この関数でそのまま作れる。
///
/// `superseded_by` は**同じキャラクターの実在する行**でなければならない
/// （複合外部キー `(superseded_by, character_id)`）。
///
/// # 置き場所が `tests/common` である理由
///
/// **写しを作らないため。** `tests/store_memory.rs`（読みの意味論）と
/// `tests/server_admin_memories.rs`（HTTP 越しの線）が同じ行を要る。
/// 2つ持つと、片方の仕込み方だけが黙って劣化する。
pub async fn insert_superseded_memory_at(
    store: &Store,
    character_id: CharacterId,
    subject: &str,
    content: &str,
    category: Option<&str>,
    created_at: &str,
    superseded_by: i64,
) -> i64 {
    let done = sqlx::query(
        "INSERT INTO memory (character_id, subject, content, category, created_at, superseded_by) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(character_id.get())
    .bind(subject)
    .bind(content)
    .bind(category)
    .bind(created_at)
    .bind(superseded_by)
    .execute(store.pool())
    .await
    .unwrap_or_else(|err| panic!("後継を指す INSERT が成功すること: {err}"));
    done.last_insert_rowid()
}

/// **`stat_alloc` に素の INSERT で1行入れる**（作業単位 B-19）。
///
/// # `insert_stat_alloc_ignoring_checks` との違い —— **何も外さない**
///
/// あちらは `PRAGMA ignore_check_constraints` が要る値（負値）のためにある。
/// こちらは**スキーマがそのまま受け入れる行**を置くための穴で、
/// **成功すること自体が観測**である ——
/// `stat_alloc` の主キーは `(character_id, axis)` だが、
/// **`axis` は生のテキスト**なので `"パッション"` と `" パッション"` は同居できる。
/// **一意でなくしているのは読み出し側の正規化**（`StatAxis::parse`）の方である。
///
/// **値はプレースホルダで渡す**（CLAUDE.md 絶対制約。テストの土台でも文字列連結しない）。
pub async fn insert_stat_alloc_directly(store: &Store, character_id: i64, axis: &str, value: i64) {
    let done = sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(character_id)
        .bind(axis)
        .bind(value)
        .execute(store.pool())
        .await
        .unwrap_or_else(|err| {
            panic!("素の INSERT がスキーマ上通ること: {axis:?} = {value}: {err}")
        });
    assert_eq!(
        done.rows_affected(),
        1,
        "1行だけ入ること: {axis:?} = {value}"
    );
}
