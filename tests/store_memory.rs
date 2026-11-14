//! `memory` の永続化の検証。
//!
//! ここで確かめるのは主に次の点:
//!   * **`subject` を落とさない**こと。DB 上の綴りも、読み出した値も固定する。
//!     同じ `content` でも `user` / `character` が別物として残る（一人称の混線防止）。
//!   * **想起は `superseded_by IS NULL` の行だけを返す**こと（ADR-0003 の全件注入）。
//!     絞り込みを外すと、上書き済みの古い記憶が現在の事実として混ざる。
//!   * **`locked` は読み出しから落とさない**こと（フィルタもしない）。
//!   * **追加と読み出ししか無い**こと。`content` の UPDATE / 行の DELETE を提供しないのは
//!     `superseded_by` の不変条件を守るため（コンパイル時にしか確認できないので、
//!     ここでは「古い行が残り続ける」ことだけを固定する）。
//!   * **暴走した抽出結果（長すぎる本文・分類）が入口の型で弾かれる**こと。全件注入なので、
//!     1件入ると**人が訂正版を書くまで**以後の全プロンプトに載り続ける
//!     （行を消す道は無い。外すには下の「訂正版で置き換えられる」を人が叩く / ADR-0022）。
//!     **「DB に届かない」保証は型（コンパイル時）にしかない**ので、実行時に確かめられるのは
//!     「`MemoryContent` / `MemoryCategory` が組み立てを拒む」ことまで。
//!   * 想定外の格納値で **panic せず `InvalidStoredValue` に落ちる**こと。
//!   * **訂正版で置き換えられる**こと（ADR-0022 / 作業単位 14a）。古い行は消さずに
//!     `superseded_by` で後継を指し、想起から外れる。**修復のための一覧は想起ではない**ので、
//!     壊れた行があっても落ちない（想起は今まで通り落ちる）。
//!
//! スキーマ制約そのものの検証は `tests/store_schema.rs` にある。

use chrono::Utc;
use personacore::domain::{
    CategoryError, CharacterId, ContentError, DisplayName, Handle, MemoryCategory, MemoryColumn,
    MemoryContent, MemoryEntry, MemoryId, NewMemory, Persona, Subject, SupersededMemory,
    UnreadableMemory,
};
use personacore::store::{Memory, RecallScope, Store, StoreError};
use sqlx::error::ErrorKind;
use tempfile::TempDir;

mod common;

use common::{
    CLOCK_SKEW, fixture_scale, insert_memory_ignoring_checks, insert_superseded_memory_at,
};

/// 一時ディレクトリ上に新規 DB を作り、マイグレーション適用済みの `Store` を返す。
async fn setup() -> (TempDir, Store) {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let store = Store::connect(dir.path().join("personacore.sqlite"))
        .await
        .expect("接続とマイグレーション適用が成功すること");
    (dir, store)
}

fn persona(raw: &str) -> Persona {
    Persona::parse(raw).expect("テストで使うペルソナは正当であること")
}

/// テスト用のキャラクターを1件作る。
async fn character(store: &Store, raw: &str) -> CharacterId {
    let name = Handle::parse(raw).expect("テストで使う名前は正当であること");
    store
        .create_character(
            &name,
            &DisplayName::parse(raw).expect("テストで使う表示名は正当であること"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること")
}

/// 分類なしの記憶。
fn plain(subject: Subject, content: &str) -> NewMemory {
    NewMemory {
        subject,
        content: MemoryContent::parse(content).expect("テストで使う本文は正当であること"),
        category: None,
    }
}

/// 分類つきの記憶。
fn categorized(subject: Subject, content: &str, category: &str) -> NewMemory {
    NewMemory {
        subject,
        content: MemoryContent::parse(content).expect("テストで使う本文は正当であること"),
        category: Some(MemoryCategory::parse(category).expect("テストで使う分類は正当であること")),
    }
}

/// 読み出した記憶の本文だけを並べる。
fn contents(memories: &[Memory]) -> Vec<&str> {
    memories.iter().map(|m| m.content.as_str()).collect()
}

/// `memory` の行数（想起から除外された行も数える）。
async fn memory_count(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM memory")
        .fetch_one(store.pool())
        .await
        .expect("件数を取得できること")
}

/// `StoreError::Query` の中身の DB エラー種別を検証する。
fn assert_query_kind(err: &StoreError, expected: ErrorKind, context: &str) {
    match err {
        StoreError::Query(inner) => {
            let kind = inner.as_database_error().map(|db| db.kind());
            assert_eq!(kind, Some(expected), "{context}: 実際のエラー = {inner}");
        }
        other => panic!("{context}: Query エラーを期待したが {other:?} だった"),
    }
}

/// CHECK 制約を無効化した同一接続上で1文だけ実行する。
///
/// `PRAGMA ignore_check_constraints` は**接続ごと**の設定なので、
/// プールから接続を掴んだまま書き換える。
async fn force_update(store: &Store, sql: &'static str, value: &str) {
    let mut conn = store.pool().acquire().await.expect("接続を取得できること");
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *conn)
        .await
        .expect("PRAGMA が成功すること");
    sqlx::query(sql)
        .bind(value)
        .execute(&mut *conn)
        .await
        .expect("直接の UPDATE が成功すること");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *conn)
        .await
        .expect("PRAGMA が成功すること");
}

/// 追加 → 想起、の往復。全列が落ちずに戻ること。
#[tokio::test]
async fn add_then_recall_round_trips_every_column() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let before = Utc::now() - CLOCK_SKEW;

    let id = store
        .add_memory(aoi, &categorized(Subject::User, "猫が好き", "好み"))
        .await
        .expect("記憶を追加できること");

    let memories = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること");
    assert_eq!(memories.len(), 1);
    let memory = &memories[0];

    assert_eq!(memory.id, id);
    assert_eq!(memory.character_id, aoi);
    assert_eq!(memory.subject, Subject::User);
    assert_eq!(memory.content.as_str(), "猫が好き");
    assert_eq!(
        memory.category.as_ref().map(MemoryCategory::as_str),
        Some("好み")
    );
    assert!(!memory.locked, "locked の既定値は false であること");
    assert!(
        memory.created_at >= before && memory.created_at <= Utc::now() + CLOCK_SKEW,
        "created_at に妥当な日時が入ること: {}",
        memory.created_at
    );
}

/// 分類は省略できる（スキーマ上 NULL 可）。
#[tokio::test]
async fn category_is_optional() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    store
        .add_memory(aoi, &plain(Subject::Character, "紅茶をいれるのが得意"))
        .await
        .expect("分類なしでも追加できること");

    let stored: Option<String> = sqlx::query_scalar("SELECT category FROM memory")
        .fetch_one(store.pool())
        .await
        .expect("category を取得できること");
    assert_eq!(stored, None, "分類なしは NULL で格納されること");

    let memories = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること");
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].category, None);
}

/// **`subject` は DB 上でスキーマの CHECK が許す綴りで格納される。**
///
/// 往復だけを見ていると `User → 'character'` の取り違えが対称に打ち消し合って通ってしまうので、
/// 格納された文字列そのものを固定する。
#[tokio::test]
async fn subject_is_stored_with_the_spelling_the_schema_expects() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    store
        .add_memory(aoi, &plain(Subject::User, "ユーザは猫が好き"))
        .await
        .expect("記憶を追加できること");
    store
        .add_memory(aoi, &plain(Subject::Character, "葵は紅茶が好き"))
        .await
        .expect("記憶を追加できること");

    let stored: Vec<String> = sqlx::query_scalar("SELECT subject FROM memory ORDER BY id")
        .fetch_all(store.pool())
        .await
        .expect("subject を取得できること");
    assert_eq!(stored, vec!["user".to_owned(), "character".to_owned()]);

    let subjects: Vec<Subject> = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること")
        .into_iter()
        .map(|memory| memory.subject)
        .collect();
    assert_eq!(
        subjects,
        vec![Subject::User, Subject::Character],
        "往復すること"
    );
}

/// **同じ本文でも `subject` で区別される。**
///
/// 想起が `subject` を落とすと「猫が好き」がユーザの記憶かキャラの記憶か分からなくなり、
/// キャラがユーザの記憶を自分のものとして喋り出す（docs/03-data-model.md の不変条件1）。
#[tokio::test]
async fn subject_distinguishes_memories_with_identical_content() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    store
        .add_memory(aoi, &plain(Subject::User, "猫が好き"))
        .await
        .expect("記憶を追加できること");
    store
        .add_memory(aoi, &plain(Subject::Character, "猫が好き"))
        .await
        .expect("記憶を追加できること");

    let pairs: Vec<(Subject, String)> = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること")
        .into_iter()
        .map(|memory| (memory.subject, memory.content.as_str().to_owned()))
        .collect();

    assert_eq!(
        pairs,
        vec![
            (Subject::User, "猫が好き".to_owned()),
            (Subject::Character, "猫が好き".to_owned()),
        ],
        "本文が同じでも主体は取り違えられないこと"
    );
}

/// **想起は上書き済み（`superseded_by` が埋まった）記憶を返さない。**
///
/// 絞り込みを外すと「以前はコーヒーが好きだった」が現在の事実として注入される。
/// 同時に、**古い行は消さない**（時系列保持）ことも固定する。
#[tokio::test]
async fn superseded_memories_are_excluded_but_not_deleted() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let old = store
        .add_memory(aoi, &plain(Subject::User, "コーヒーが好き"))
        .await
        .expect("記憶を追加できること");
    let unrelated = store
        .add_memory(aoi, &plain(Subject::User, "猫が好き"))
        .await
        .expect("記憶を追加できること");
    let new = store
        .add_memory(aoi, &plain(Subject::User, "紅茶が好き"))
        .await
        .expect("記憶を追加できること");

    // フェーズ2の上書きを再現する。`supersede_memory` は**訂正版の INSERT と対**でしか
    // 書かないので、既存の2行だけで鎖を作るにはここで直接更新するしかない。
    sqlx::query("UPDATE memory SET superseded_by = ? WHERE id = ?")
        .bind(new.get())
        .bind(old.get())
        .execute(store.pool())
        .await
        .expect("superseded_by を設定できること");

    let memories = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること");
    assert_eq!(
        contents(&memories),
        vec!["猫が好き", "紅茶が好き"],
        "上書き済みの記憶は想起に含まれないこと"
    );
    assert!(
        memories.iter().all(|memory| memory.id != old),
        "上書き済みの id が混ざらないこと"
    );
    assert!(
        memories.iter().any(|memory| memory.id == unrelated),
        "無関係な記憶は残ること"
    );

    assert_eq!(
        memory_count(&store).await,
        3,
        "古い行は消さずに保持すること（時系列保持）"
    );

    // **件数だけでは足りない。** 行が残っていても本文が「紅茶が好き」で上書きされていたら、
    // フェーズ2の「以前はコーヒーが好きだった」は永久に失われる（不変条件2が守りたい価値そのもの）。
    // 想起の対象外になった行から、当時の本文と後継への参照が読み戻せることまで固定する。
    let (old_content, old_superseded_by): (String, Option<i64>) =
        sqlx::query_as("SELECT content, superseded_by FROM memory WHERE id = ?")
            .bind(old.get())
            .fetch_one(store.pool())
            .await
            .expect("上書き済みの行を直接引けること");
    assert_eq!(
        old_content, "コーヒーが好き",
        "古い行の本文が読み戻せること（「以前はXだった」の想起に要る）"
    );
    assert_eq!(
        old_superseded_by,
        Some(new.get()),
        "古い行が後継を指したまま残ること"
    );
}

/// **`locked` は想起から落とさない。** フィルタもしない。
#[tokio::test]
async fn locked_memories_are_recalled_with_the_flag() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let locked = store
        .add_memory(aoi, &plain(Subject::User, "手動で固定した記憶"))
        .await
        .expect("記憶を追加できること");
    store
        .add_memory(aoi, &plain(Subject::User, "通常の記憶"))
        .await
        .expect("記憶を追加できること");

    // `store` には locked を立てる API が無い（手動運用の想定）ので直接更新する。
    sqlx::query("UPDATE memory SET locked = 1 WHERE id = ?")
        .bind(locked.get())
        .execute(store.pool())
        .await
        .expect("locked を設定できること");

    let memories = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること");
    assert_eq!(
        contents(&memories),
        vec!["手動で固定した記憶", "通常の記憶"],
        "locked でフィルタしないこと"
    );
    assert_eq!(
        memories
            .iter()
            .map(|memory| memory.locked)
            .collect::<Vec<bool>>(),
        vec![true, false],
        "locked の値が読み出せること"
    );
}

/// 想起はキャラクターごとに独立している。
#[tokio::test]
async fn memories_are_scoped_to_one_character() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;

    store
        .add_memory(aoi, &plain(Subject::User, "葵に話したこと"))
        .await
        .expect("記憶を追加できること");
    store
        .add_memory(rin, &plain(Subject::User, "凛に話したこと"))
        .await
        .expect("記憶を追加できること");

    assert_eq!(
        contents(
            &store
                .recall_memories(aoi, RecallScope::Everything)
                .await
                .expect("想起が成功すること")
        ),
        vec!["葵に話したこと"],
        "別キャラの記憶が混ざらないこと"
    );
    assert_eq!(
        contents(
            &store
                .recall_memories(rin, RecallScope::Everything)
                .await
                .expect("想起が成功すること")
        ),
        vec!["凛に話したこと"]
    );
}

/// 想起は**作成順**（古い順）で返す。
///
/// **同着（同じ `created_at`）の並びはここでは見ていない。** `add_memory` は
/// `created_at` を DEFAULT に任せるので、このテストは同着を**作れるとは限らない**。
/// 第2キーの見張り番は `memories_created_in_the_same_millisecond_are_ordered_by_id`。
#[tokio::test]
async fn recall_returns_memories_in_creation_order() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    for index in 0..5 {
        store
            .add_memory(aoi, &plain(Subject::User, &format!("記憶{index}")))
            .await
            .expect("記憶を追加できること");
    }

    let memories = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること");
    assert_eq!(
        contents(&memories),
        vec!["記憶0", "記憶1", "記憶2", "記憶3", "記憶4"],
        "追加順に返ること"
    );
}

/// 記憶が1件も無ければ空。存在しないキャラクターでも panic しない。
#[tokio::test]
async fn recall_of_a_character_without_memories_is_empty() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let unknown = CharacterId::from_raw_for_tests(9_999);

    assert!(
        store
            .recall_memories(aoi, RecallScope::Everything)
            .await
            .expect("想起が成功すること")
            .is_empty(),
        "記憶が無ければ空であること"
    );
    assert!(
        store
            .recall_memories(unknown, RecallScope::Everything)
            .await
            .expect("想起が成功すること")
            .is_empty(),
        "存在しないキャラクターでも空であること"
    );
}

/// 存在しないキャラクターには記憶を追加できない（外部キー違反）。
#[tokio::test]
async fn memory_of_unknown_character_cannot_be_added() {
    let (_dir, store) = setup().await;
    let unknown = CharacterId::from_raw_for_tests(9_999);

    let err = store
        .add_memory(unknown, &plain(Subject::User, "宛先のない記憶"))
        .await
        .expect_err("存在しないキャラクターには追加できないこと");
    assert_query_kind(&err, ErrorKind::ForeignKeyViolation, "memory.character_id");
    assert_eq!(memory_count(&store).await, 0, "行が残らないこと");
}

/// 格納されている `subject` が想定外の値なら握り潰さずエラーにする。
///
/// スキーマの CHECK があるため通常は起こらないが、**panic しない**ことを固定しておく。
#[tokio::test]
async fn unknown_stored_subject_is_reported_as_invalid_stored_value() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    store
        .add_memory(aoi, &plain(Subject::User, "猫が好き"))
        .await
        .expect("記憶を追加できること");

    force_update(&store, "UPDATE memory SET subject = ?", "assistant").await;

    let err = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect_err("想定外の subject は握り潰さないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {err:?}"
    );
    assert!(
        !err.to_string().contains("assistant"),
        "Display に格納値を含めないこと: {err}"
    );
}

/// 格納されている `locked` が 0 / 1 以外なら握り潰さずエラーにする。
#[tokio::test]
async fn unknown_stored_locked_is_reported_as_invalid_stored_value() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    store
        .add_memory(aoi, &plain(Subject::User, "猫が好き"))
        .await
        .expect("記憶を追加できること");

    force_update(&store, "UPDATE memory SET locked = ?", "7").await;

    let err = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect_err("想定外の locked は握り潰さないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {err:?}"
    );
}

/// 格納されている本文が空白のみなら握り潰さずエラーにする。
///
/// `store` 経由では `MemoryContent` が防ぐため、DB を直接編集した場合にのみ現れる。
#[tokio::test]
async fn blank_stored_content_is_reported_as_invalid_stored_value() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    sqlx::query("INSERT INTO memory (character_id, subject, content) VALUES (?, 'user', ?)")
        .bind(aoi.get())
        .bind("\u{3000} \t")
        .execute(store.pool())
        .await
        .expect("直接の INSERT が成功すること");

    let err = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect_err("空白のみの本文は握り潰さないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {err:?}"
    );
}

/// **長すぎる本文は `store` の入口（型）で弾かれる。**
///
/// ADR-0003 は全件注入なので、暴走した抽出結果が1件入ると**そのキャラの以後の
/// 全プロンプトに永久に載り続ける。** フェーズ1には削除も上書きも無いため、
/// アプリ経由では二度と取り除けない。
///
/// # **「DB に届かない」ことは型でしか保証できない**
///
/// `add_memory` は `&NewMemory` しか受け取らず、その `content` は `MemoryContent`。
/// つまり「未検証の文字列が DB に届く経路が無い」のは**コンパイル時の性質**であって、
/// 実行時の assert では表現できない。
///
/// 以前はここで `memory_count == 0` を確かめていたが、`add_memory` を一度も呼ばない
/// 以上**実装が何であっても真**であり、`add_memory_unchecked(&str)` のような抜け道が
/// 生えても鳴らなかった。空振りの assert は落としてある。
/// 固定できるのは「入口の型が弾くこと」だけで、それがこのテストの主張。
/// （`src/domain/memory.rs` の単体テストと重なるが、ここは**`store` の入口の契約**を
/// 明示するために置いている。）
#[tokio::test]
async fn a_runaway_extraction_cannot_be_built_into_a_new_memory() {
    let conversation_log = "ユーザ: こんにちは\n葵: こんにちは。\n".repeat(10_000);
    assert_eq!(
        MemoryContent::parse(&conversation_log),
        Err(ContentError::TooLong),
        "会話ログ丸ごとは記憶の本文として作れないこと"
    );

    // 分類も同じ論法で守る。分類は見出しとしてプロンプトに載るため、
    // 本文と取り違えて会話ログを吐かれると同じように永久に残る。
    let single_line = conversation_log.replace('\n', " ");
    assert_eq!(
        MemoryCategory::parse(&single_line),
        Err(CategoryError::TooLong),
        "会話ログ丸ごとは記憶の分類としても作れないこと"
    );
}

/// 格納されている本文が上限を超えていたら握り潰さずエラーにする。
///
/// `store` 経由では `MemoryContent` が防ぐので、DB を直接編集した場合にのみ現れる。
/// **黙って切り詰めたりスキップしたりしない**（`memory_from_row` の方針は fail-stop）。
/// 想起が丸ごと止まるのは重いが、記憶を静かに落とすとキャラクターの振る舞いが
/// 不可視に変わるため、**気付ける方に倒している。**
#[tokio::test]
async fn overlong_stored_content_is_reported_as_invalid_stored_value() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    sqlx::query("INSERT INTO memory (character_id, subject, content) VALUES (?, 'user', ?)")
        .bind(aoi.get())
        .bind("あ".repeat(MemoryContent::MAX_CHARS + 1))
        .execute(store.pool())
        .await
        .expect("直接の INSERT が成功すること");

    let err = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect_err("上限を超えた本文は握り潰さないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {err:?}"
    );
    assert!(
        !err.to_string().contains('あ'),
        "Display に格納値を含めないこと: {err}"
    );
}

/// 上限ちょうどの本文は `store` を往復する（境界の下側）。
#[tokio::test]
async fn content_at_the_limit_round_trips() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let at_limit = "あ".repeat(MemoryContent::MAX_CHARS);

    store
        .add_memory(aoi, &plain(Subject::User, &at_limit))
        .await
        .expect("上限ちょうどの本文は追加できること");

    let memories = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること");
    assert_eq!(contents(&memories), vec![at_limit.as_str()]);
}

/// 格納されている**分類**が上限を超えていたら握り潰さずエラーにする。
///
/// 本文と同じ扱い。`store` 経由では `MemoryCategory` が防ぐので、
/// DB を直接編集した場合にのみ現れる。
#[tokio::test]
async fn overlong_stored_category_is_reported_as_invalid_stored_value() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    sqlx::query(
        "INSERT INTO memory (character_id, subject, content, category) VALUES (?, 'user', ?, ?)",
    )
    .bind(aoi.get())
    .bind("猫が好き")
    .bind("あ".repeat(MemoryCategory::MAX_CHARS + 1))
    .execute(store.pool())
    .await
    .expect("直接の INSERT が成功すること");

    let err = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect_err("上限を超えた分類は握り潰さないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {err:?}"
    );
    assert!(
        !err.to_string().contains('あ'),
        "Display に格納値を含めないこと: {err}"
    );
}

/// 上限ちょうどの分類は `store` を往復する（境界の下側）。
#[tokio::test]
async fn category_at_the_limit_round_trips() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let at_limit = "あ".repeat(MemoryCategory::MAX_CHARS);

    store
        .add_memory(aoi, &categorized(Subject::User, "猫が好き", &at_limit))
        .await
        .expect("上限ちょうどの分類は追加できること");

    let memories = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること");
    assert_eq!(
        memories[0].category.as_ref().map(MemoryCategory::as_str),
        Some(at_limit.as_str())
    );
}

/// 前後に空白のある格納値は正規化して返す（`stat_allocs` と同じ扱い）。
#[tokio::test]
async fn padded_stored_values_are_normalized_on_read() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    sqlx::query(
        "INSERT INTO memory (character_id, subject, content, category) VALUES (?, 'user', ?, ?)",
    )
    .bind(aoi.get())
    .bind("  猫が好き  ")
    .bind("\u{3000}好み\u{3000}")
    .execute(store.pool())
    .await
    .expect("直接の INSERT が成功すること");

    let memories = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること");
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].content.as_str(), "猫が好き");
    assert_eq!(
        memories[0].category.as_ref().map(MemoryCategory::as_str),
        Some("好み")
    );
}

// ---------------------------------------------------------------------------
// 訂正版で置き換える / 修復のための一覧（ADR-0022 / 作業単位 14a）
//
// **どちらも想起ではない。** 想起の入口は `recall_memories` ひとつのまま
// （ADR-0003 / ADR-0023）。ここで確かめるのは修復の導線であって、
// プロンプトに載る経路ではない。
// ---------------------------------------------------------------------------

/// 行の `superseded_by`（想起から外れた行も引ける）。
async fn superseded_by_of(store: &Store, id: i64) -> Option<i64> {
    sqlx::query_scalar("SELECT superseded_by FROM memory WHERE id = ?")
        .bind(id)
        .fetch_one(store.pool())
        .await
        .expect("行を直接引けること")
}

/// 一覧から**読めた行**の本文だけを並べる。
fn readable_contents(entries: &[MemoryEntry]) -> Vec<&str> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            MemoryEntry::Readable(memory) => Some(memory.content.as_str()),
            MemoryEntry::Unreadable(_) => None,
        })
        .collect()
}

/// 一覧から**読めなかった行**だけを並べる。
fn unreadable(entries: &[MemoryEntry]) -> Vec<&UnreadableMemory> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            MemoryEntry::Readable(_) => None,
            MemoryEntry::Unreadable(broken) => Some(broken),
        })
        .collect()
}

/// **訂正版で置き換えると、古い行は想起から消える。行そのものは残る。**
///
/// ADR-0022 の中心。`superseded_by` に後継の id を入れた瞬間、
/// 古い行は `recall_memories` の `superseded_by IS NULL` から外れる。
///
/// # フィクスチャは全部の値を違えてある
///
/// 古い行と訂正版で **`subject` も `content` も `category` も違う。**
/// 同値にすると「訂正版の列を古い行から写す」実装が緑を通る
/// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
#[tokio::test]
async fn a_correction_hides_the_old_memory_from_recall_without_deleting_it() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let old = store
        .add_memory(aoi, &categorized(Subject::User, "店を経営している", "職業"))
        .await
        .expect("記憶を追加できること");
    let unrelated = store
        .add_memory(aoi, &plain(Subject::Character, "紅茶をいれるのが得意"))
        .await
        .expect("記憶を追加できること");

    // **主体も分類も本文も、古い行と違う値にする。**
    let correction = categorized(Subject::Character, "アルバイトをしている", "生活");
    let new = store
        .supersede_memory(aoi, old, &correction)
        .await
        .expect("訂正版で置き換えられること");

    assert_ne!(new, old, "訂正版は新しい行であること");

    let memories = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること");
    assert_eq!(
        contents(&memories),
        vec!["紅茶をいれるのが得意", "アルバイトをしている"],
        "古い行が想起から消え、訂正版が現れること"
    );
    assert!(
        memories.iter().all(|memory| memory.id != old),
        "上書き済みの id が混ざらないこと"
    );
    assert!(
        memories.iter().any(|memory| memory.id == unrelated),
        "無関係な記憶は残ること"
    );

    // **訂正版の列は引数から来る**（古い行の写しではない）。
    let written = memories
        .iter()
        .find(|memory| memory.id == new)
        .expect("訂正版が想起に現れること");
    assert_eq!(written.subject, Subject::Character, "主体は引数のもの");
    assert_eq!(
        written.category.as_ref().map(MemoryCategory::as_str),
        Some("生活"),
        "分類は引数のもの"
    );
    assert_eq!(written.character_id, aoi);
    assert!(!written.locked, "訂正版の locked は既定のまま");

    // **行は消えていない。**
    assert_eq!(
        memory_count(&store).await,
        3,
        "古い行は消さずに保持すること（時系列保持）"
    );
    let (old_content, old_subject, old_superseded_by): (String, String, Option<i64>) =
        sqlx::query_as("SELECT content, subject, superseded_by FROM memory WHERE id = ?")
            .bind(old.get())
            .fetch_one(store.pool())
            .await
            .expect("上書き済みの行を直接引けること");
    assert_eq!(
        old_content, "店を経営している",
        "古い行の本文が読み戻せること（「以前はXだった」の想起に要る）"
    );
    assert_eq!(old_subject, "user", "古い行の主体も書き換えないこと");
    assert_eq!(
        old_superseded_by,
        Some(new.get()),
        "古い行が後継を指したまま残ること"
    );
}

/// **訂正は他のキャラクターに漏れない。**
#[tokio::test]
async fn a_correction_does_not_touch_another_characters_memories() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;

    let target = store
        .add_memory(aoi, &plain(Subject::User, "黒い犬を飼っている"))
        .await
        .expect("記憶を追加できること");
    store
        .add_memory(rin, &plain(Subject::User, "凛に話したこと"))
        .await
        .expect("記憶を追加できること");

    store
        .supersede_memory(aoi, target, &plain(Subject::User, "黒い猫を飼っている"))
        .await
        .expect("訂正版で置き換えられること");

    assert_eq!(
        contents(
            &store
                .recall_memories(rin, RecallScope::Everything)
                .await
                .expect("想起が成功すること")
        ),
        vec!["凛に話したこと"],
        "別キャラクターの記憶は変わらないこと"
    );
}

/// **存在しない記憶は訂正できない。** 理由は「引き当てが無い」。
#[tokio::test]
async fn superseding_a_memory_that_does_not_exist_is_rejected() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    store
        .add_memory(aoi, &plain(Subject::User, "実在する記憶"))
        .await
        .expect("記憶を追加できること");

    let ghost = MemoryId::from_raw_for_tests(9_999);
    let err = store
        .supersede_memory(aoi, ghost, &plain(Subject::User, "宛先のない訂正"))
        .await
        .expect_err("存在しない記憶は訂正できないこと");
    assert!(
        matches!(err, StoreError::UnknownMemory),
        "引き当てが無いこととして型付けされること: {err:?}"
    );
    assert!(
        !err.to_string().contains("9999"),
        "Display に id を載せないこと: {err}"
    );

    assert_eq!(
        memory_count(&store).await,
        1,
        "拒否された呼び出しは訂正版を書き残さないこと"
    );
}

/// **別のキャラクターの記憶は訂正できない。**
///
/// 複合外部キーが構造的に防いでいる（`tests/store_schema.rs`）が、
/// **理由が届かない。** 外部キーに任せると `Query` になり、
/// 「引き当てが無い」（`UnknownMemory`）と区別が付かない。
///
/// # フィクスチャは「実在する id」であることが要点
///
/// 存在しない id を渡すと `UnknownMemory` になるので、**この2つを取り違えた実装も緑になる。**
/// 指す行は実在し、ただし持ち主が違う、という状態を作る。
#[tokio::test]
async fn superseding_a_memory_of_another_character_is_rejected() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;

    let rins_memory = store
        .add_memory(rin, &plain(Subject::User, "凛に話したこと"))
        .await
        .expect("記憶を追加できること");

    let err = store
        .supersede_memory(aoi, rins_memory, &plain(Subject::User, "葵からの横取り"))
        .await
        .expect_err("別キャラクターの記憶は訂正できないこと");
    assert!(
        matches!(err, StoreError::MemoryBelongsToAnotherCharacter),
        "宛先違いとして型付けされること（引き当てが無いのとは別の理由）: {err:?}"
    );

    // **「引き当てが無い」と同じ理由に潰していないこと。**
    let missing = store
        .supersede_memory(
            aoi,
            MemoryId::from_raw_for_tests(9_999),
            &plain(Subject::User, "宛先のない訂正"),
        )
        .await
        .expect_err("存在しない記憶は訂正できないこと");
    assert_ne!(
        std::mem::discriminant(&err),
        std::mem::discriminant(&missing),
        "2つの拒否が別の理由であること: {err:?} / {missing:?}"
    );

    assert_eq!(
        memory_count(&store).await,
        1,
        "拒否された呼び出しは訂正版を書き残さないこと"
    );
    assert_eq!(
        superseded_by_of(&store, rins_memory.get()).await,
        None,
        "他人の記憶に印を付けないこと"
    );
    assert_eq!(
        contents(
            &store
                .recall_memories(rin, RecallScope::Everything)
                .await
                .expect("想起が成功すること")
        ),
        vec!["凛に話したこと"],
        "持ち主の想起は変わらないこと"
    );
}

/// **すでに上書きされた記憶は、もう一度上書きできない**（鎖を線形に保つ）。
///
/// かつ、**いま有効な方（後継）は上書きできる**——「いま有効な記憶を指定してください」が
/// 実際に通る道であることまで固定する。ここを確かめないと、
/// 「常に拒否する」実装（訂正が2度目から永久に効かない）が緑を通る。
#[tokio::test]
async fn an_already_superseded_memory_cannot_be_superseded_again() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let first = store
        .add_memory(aoi, &plain(Subject::User, "黒い犬を飼っている"))
        .await
        .expect("記憶を追加できること");
    let second = store
        .supersede_memory(aoi, first, &plain(Subject::User, "黒い猫を飼っている"))
        .await
        .expect("1度目の訂正ができること");

    let err = store
        .supersede_memory(
            aoi,
            first,
            &plain(Subject::User, "白い猫を飼っている（誤り）"),
        )
        .await
        .expect_err("上書き済みの記憶は指せないこと");
    assert!(
        matches!(err, StoreError::MemoryAlreadySuperseded),
        "上書き済みとして型付けされること: {err:?}"
    );

    assert_eq!(
        memory_count(&store).await,
        2,
        "拒否された呼び出しは訂正版を書き残さないこと"
    );
    assert_eq!(
        superseded_by_of(&store, first.get()).await,
        Some(second.get()),
        "既にある後継を差し替えないこと（鎖が分岐しない）"
    );

    // **いま有効な方は訂正できる**（鎖は伸びる）。
    let third = store
        .supersede_memory(
            aoi,
            second,
            &plain(Subject::User, "「コーヒー」という名の猫"),
        )
        .await
        .expect("いま有効な記憶は訂正できること");
    assert_eq!(
        superseded_by_of(&store, second.get()).await,
        Some(third.get()),
        "鎖が線形に伸びること"
    );
    assert_eq!(
        contents(
            &store
                .recall_memories(aoi, RecallScope::Everything)
                .await
                .expect("想起が成功すること")
        ),
        vec!["「コーヒー」という名の猫"],
        "有効なのは鎖の末端だけであること"
    );
}

/// **壊れて想起を止めている記憶も訂正できる**（ADR-0022 の存在理由そのもの）。
///
/// 引き当ては `id` だけで行い、**古い行の値を `parse` しない。**
/// 読んでから書く実装は「直したい行ほど読めない」（ADR-0016 §4）に逆戻りする。
#[tokio::test]
async fn a_broken_memory_can_still_be_superseded() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    // 空白のみの本文（`MemoryContent` が拒む値）を直接入れる。想起はここで止まる。
    let broken = insert_memory_ignoring_checks(&store, aoi, "user", "\u{3000} \t", None, 0).await;
    assert!(
        matches!(
            store.recall_memories(aoi, RecallScope::Everything).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "前提: 壊れた行があると想起は止まること"
    );

    // **一覧から id を取り、そのまま訂正する。** これが修復の導線。
    let entries = store
        .list_memories_for_repair(aoi)
        .await
        .expect("一覧は落ちないこと");
    let pointed = unreadable(&entries);
    assert_eq!(pointed.len(), 1, "壊れた行が1件指し示されること");
    assert_eq!(pointed[0].id.get(), broken);
    assert_eq!(pointed[0].character_id, aoi);

    store
        .supersede_memory(
            aoi,
            pointed[0].id,
            &plain(Subject::User, "猫を「コーヒー」と名付けている"),
        )
        .await
        .expect("壊れた行も訂正できること");

    let memories = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("訂正すると想起が回復すること");
    assert_eq!(contents(&memories), vec!["猫を「コーヒー」と名付けている"]);
    assert_eq!(
        memory_count(&store).await,
        2,
        "壊れた行も消さずに残すこと（調査の手掛かり）"
    );
}

/// **訂正版が書けなければ、古い行に印は付かない**（INSERT と UPDATE は不可分）。
///
/// 外部キーを無効にしたままキャラクターを消して**孤児になった記憶**を作ると、
/// 訂正版の INSERT だけが外部キー違反で落ちる（古い行は引ける）。
/// このとき古い行の `superseded_by` が埋まっていたら、
/// **後継のいない印**が残る——「新しい記憶は無いのに古い記憶も想起されない」状態。
///
/// **UPDATE を先に書く実装（あるいは自動コミットで2文に分ける実装）はここで赤になる。**
#[tokio::test]
async fn a_correction_that_cannot_be_written_leaves_the_old_memory_untouched() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let orphan = store
        .add_memory(aoi, &plain(Subject::User, "親を失う記憶"))
        .await
        .expect("記憶を追加できること");
    common::delete_character_ignoring_foreign_keys(&store, aoi.get()).await;

    let err = store
        .supersede_memory(aoi, orphan, &plain(Subject::User, "書けない訂正"))
        .await
        .expect_err("親のいないキャラクターには訂正版を書けないこと");
    assert_query_kind(&err, ErrorKind::ForeignKeyViolation, "memory.character_id");

    assert_eq!(
        superseded_by_of(&store, orphan.get()).await,
        None,
        "訂正版が書けなかったのに印だけ付くことがないこと"
    );
    assert_eq!(memory_count(&store).await, 1, "訂正版が半端に残らないこと");
}

/// **同時に同じ記憶を訂正しても `database is locked` にならない**
/// （`BEGIN IMMEDIATE` の見張り番）。
///
/// `supersede_memory` は「古い行を読む → 書く」形なので、素の `BEGIN`（DEFERRED）だと
/// 読み取りスナップショットが固定され、書きへの昇格が `SQLITE_BUSY_SNAPSHOT`(517) になる。
/// **これは `busy_timeout` の対象外で、待たずに即座に失敗する**
/// （実測は `src/store/character.rs::BEGIN_IMMEDIATE` の doc）。
///
/// **「両方は成功しない」だけでは足りない。** 負けた側の**理由**まで見る。
/// 素の `BEGIN` に戻すと、負けた側が `MemoryAlreadySuperseded` ではなく
/// `StoreError::Query` になって赤くなる。
#[tokio::test]
async fn two_corrections_of_the_same_memory_never_race_into_a_locked_database() {
    let (_dir, store) = setup().await;

    for round in 0..64 {
        let aoi = character(&store, &format!("aoi-{round}")).await;
        let target = store
            .add_memory(aoi, &plain(Subject::User, "訂正される記憶"))
            .await
            .expect("記憶を追加できること");

        let left = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .supersede_memory(aoi, target, &plain(Subject::User, "左からの訂正"))
                    .await
            })
        };
        let right = {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .supersede_memory(aoi, target, &plain(Subject::Character, "右からの訂正"))
                    .await
            })
        };

        let left = left.await.expect("タスクが panic しないこと");
        let right = right.await.expect("タスクが panic しないこと");

        match (&left, &right) {
            (Ok(_), Err(StoreError::MemoryAlreadySuperseded))
            | (Err(StoreError::MemoryAlreadySuperseded), Ok(_)) => {}
            _ => panic!(
                "想定した2通り以外の結果になった（{round} 周目）: left = {left:?} / right = {right:?}"
            ),
        }

        // どちらが勝っても、鎖は線形で有効な記憶は1件。
        let memories = store
            .recall_memories(aoi, RecallScope::Everything)
            .await
            .unwrap_or_else(|err| panic!("どう転んでも想起できること（{round} 周目）: {err:?}"));
        assert_eq!(
            memories.len(),
            1,
            "有効な記憶が1件だけであること（{round} 周目）"
        );
    }
}

/// **一覧は、壊れた行を指し示しつつ健全な行を返す**（ADR-0022 §2 / ADR-0016 §3 と同じ形）。
///
/// `recall_memories` は1行でも壊れていれば丸ごと止まる（**それは想起としては正しい**）。
/// 修復のための一覧は止まらない。**片方だけを見ていると、
/// `filter_map` で壊れた行を捨てる実装が緑を通る**（0012b で実測した穴）。
#[tokio::test]
async fn a_broken_memory_row_does_not_take_the_healthy_ones_with_it() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let healthy = store
        .add_memory(
            aoi,
            &categorized(Subject::Character, "紅茶をいれるのが得意", "特技"),
        )
        .await
        .expect("記憶を追加できること");
    let broken = insert_memory_ignoring_checks(&store, aoi, "user", "\u{3000} \t", None, 0).await;
    let also_healthy = store
        .add_memory(aoi, &plain(Subject::User, "猫が好き"))
        .await
        .expect("記憶を追加できること");

    let entries = store
        .list_memories_for_repair(aoi)
        .await
        .expect("壊れた行があっても一覧は落ちないこと");

    assert_eq!(entries.len(), 3, "行を落とさないこと: {entries:?}");
    assert_eq!(
        readable_contents(&entries),
        vec!["紅茶をいれるのが得意", "猫が好き"],
        "読めた行はそのまま返ること"
    );

    let pointed = unreadable(&entries);
    assert_eq!(pointed.len(), 1);
    assert_eq!(pointed[0].id.get(), broken, "壊れた行を id で指すこと");
    assert_eq!(
        pointed[0].invalid_columns,
        vec![MemoryColumn::Content],
        "壊れた列を指すこと"
    );

    // 読めた行の中身は落ちていない（全列）。
    let MemoryEntry::Readable(first) = &entries[0] else {
        panic!("先頭は読めた行であること: {entries:?}");
    };
    assert_eq!(first.id, healthy);
    assert_eq!(first.subject, Subject::Character);
    assert_eq!(
        first.category.as_ref().map(MemoryCategory::as_str),
        Some("特技")
    );
    let MemoryEntry::Readable(last) = &entries[2] else {
        panic!("末尾は読めた行であること: {entries:?}");
    };
    assert_eq!(last.id, also_healthy);
    assert_eq!(last.subject, Subject::User);

    // **並びは想起と同じ**（作成順）。壊れた行も位置を保つ。
    let ids: Vec<i64> = entries
        .iter()
        .map(|entry| match entry {
            MemoryEntry::Readable(memory) => memory.id.get(),
            MemoryEntry::Unreadable(broken) => broken.id.get(),
        })
        .collect();
    assert_eq!(ids, vec![healthy.get(), broken, also_healthy.get()]);
}

/// **壊れ方の種類ごとに、指す列が変わる。**
///
/// `memory_from_row` が `InvalidStoredValue` にする4通り（主体・本文・分類・ロック）を
/// **全部**作る。1つでも指せない種類があると、**その壊れ方で止まったチャットの
/// 原因だけが一覧から見えない。**
///
/// **複数壊れている行は全部並べる**（1つ直しても読めないことが分かる）。
#[tokio::test]
async fn every_kind_of_broken_memory_column_is_pointed_at() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let bad_subject =
        insert_memory_ignoring_checks(&store, aoi, "assistant", "主体が壊れた記憶", None, 0).await;
    let bad_content =
        insert_memory_ignoring_checks(&store, aoi, "user", "\u{3000} \t", None, 0).await;
    let bad_category = insert_memory_ignoring_checks(
        &store,
        aoi,
        "user",
        "分類が壊れた記憶",
        Some(&"あ".repeat(MemoryCategory::MAX_CHARS + 1)),
        0,
    )
    .await;
    let bad_locked =
        insert_memory_ignoring_checks(&store, aoi, "character", "ロックが壊れた記憶", None, 7)
            .await;
    // **複数壊れている行。** 1列しか報告しない実装をここで落とす。
    let doubly_broken = insert_memory_ignoring_checks(
        &store,
        aoi,
        "system",
        &"あ".repeat(MemoryContent::MAX_CHARS + 1),
        None,
        0,
    )
    .await;
    // **`locked` と他の列が同時に壊れた行。** `locked` の報告を
    // 「他の列が壊れていないときだけ」にする実装をここで落とす
    // （14a のレビューで実測。単独で壊れた行しか無いと、その変異は緑を通る）。
    let category_and_locked_broken = insert_memory_ignoring_checks(
        &store,
        aoi,
        "user",
        "分類とロックが同時に壊れた記憶",
        Some(&"あ".repeat(MemoryCategory::MAX_CHARS + 1)),
        7,
    )
    .await;

    let entries = store
        .list_memories_for_repair(aoi)
        .await
        .expect("壊れた行だけでも一覧は落ちないこと");
    let pointed = unreadable(&entries);
    assert_eq!(pointed.len(), 6, "6行とも指し示すこと: {entries:?}");

    let columns_of = |id: i64| -> Vec<MemoryColumn> {
        pointed
            .iter()
            .find(|broken| broken.id.get() == id)
            .unwrap_or_else(|| panic!("id = {id} が一覧に現れること"))
            .invalid_columns
            .clone()
    };

    assert_eq!(columns_of(bad_subject), vec![MemoryColumn::Subject]);
    assert_eq!(columns_of(bad_content), vec![MemoryColumn::Content]);
    assert_eq!(columns_of(bad_category), vec![MemoryColumn::Category]);
    assert_eq!(columns_of(bad_locked), vec![MemoryColumn::Locked]);
    assert_eq!(
        columns_of(doubly_broken),
        vec![MemoryColumn::Subject, MemoryColumn::Content],
        "壊れ方が複数なら全部並べること"
    );
    assert_eq!(
        columns_of(category_and_locked_broken),
        vec![MemoryColumn::Category, MemoryColumn::Locked],
        "locked は他の列の壊れ方と無関係に報告されること"
    );

    // **格納値そのものは持たない**（`UnreadableCharacter` と同じ判断）。
    let debug = format!("{pointed:?}");
    assert!(
        !debug.contains('あ') && !debug.contains("assistant"),
        "手掛かりは id と列名だけであること: {debug}"
    );
}

/// **一覧は想起ではない。** 想起は壊れた行があれば今まで通り止まる。
///
/// ADR-0022 が却下した選択肢（「壊れた行を想起側で読み飛ばす」）を実装に混ぜないための
/// 見張り番。**同じ DB に対して、一覧は返り、想起は止まる。**
#[tokio::test]
async fn the_repair_listing_stays_readable_where_recall_stops() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    store
        .add_memory(aoi, &plain(Subject::User, "健全な記憶"))
        .await
        .expect("記憶を追加できること");
    insert_memory_ignoring_checks(&store, aoi, "user", "ロックが壊れた記憶", None, 7).await;

    let err = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect_err("想起は壊れた行を黙って読み飛ばさないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "想起は今まで通り止まること: {err:?}"
    );

    let entries = store
        .list_memories_for_repair(aoi)
        .await
        .expect("一覧は同じ DB で返ること");
    assert_eq!(entries.len(), 2);
    assert_eq!(unreadable(&entries).len(), 1);
}

/// **一覧は上書き済みの行を返さない。**
///
/// 修復の対象は有効な行だけ。上書き済みの行はもう想起に載らないので害が無く、
/// 並べると「どれを直せばよいか」が薄まる。**履歴を見たいときは DB を直接見る**
/// （`UnreadableCharacter` と同じ立場）。
///
/// # 上書き済みの行が**壊れている**状態まで作る
///
/// 健全な行を上書きするだけでは、「読めない行は無条件に並べる」実装
/// （読めた行にだけ絞り込みを掛ける形）と区別が付かない。
#[tokio::test]
async fn the_repair_listing_leaves_out_superseded_rows() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let broken = insert_memory_ignoring_checks(&store, aoi, "user", "\u{3000} \t", None, 0).await;
    let broken_id = MemoryId::from_raw_for_tests(broken);
    let correction = store
        .supersede_memory(aoi, broken_id, &plain(Subject::User, "訂正版"))
        .await
        .expect("壊れた行を訂正できること");

    let entries = store
        .list_memories_for_repair(aoi)
        .await
        .expect("一覧が返ること");

    assert_eq!(entries.len(), 1, "有効な行だけを返すこと: {entries:?}");
    let MemoryEntry::Readable(only) = &entries[0] else {
        panic!("訂正版は読めた行であること: {entries:?}");
    };
    assert_eq!(only.id, correction);
    assert!(
        unreadable(&entries).is_empty(),
        "上書き済みの壊れた行を並べないこと（もう想起に載らない）"
    );

    // 行そのものは残っている（一覧に出ないだけ）。
    assert_eq!(memory_count(&store).await, 2);
}

/// 一覧はキャラクターごとに独立している。
#[tokio::test]
async fn the_repair_listing_is_scoped_to_one_character() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;

    store
        .add_memory(aoi, &plain(Subject::User, "葵に話したこと"))
        .await
        .expect("記憶を追加できること");
    insert_memory_ignoring_checks(&store, rin, "user", "\u{3000} \t", None, 0).await;

    let aois = store
        .list_memories_for_repair(aoi)
        .await
        .expect("一覧が返ること");
    assert_eq!(readable_contents(&aois), vec!["葵に話したこと"]);
    assert!(
        unreadable(&aois).is_empty(),
        "別キャラクターの壊れた行が混ざらないこと: {aois:?}"
    );

    let rins = store
        .list_memories_for_repair(rin)
        .await
        .expect("一覧が返ること");
    assert_eq!(unreadable(&rins).len(), 1);
    assert!(readable_contents(&rins).is_empty());
}

/// 記憶が1件も無ければ空。存在しないキャラクターでも panic しない。
#[tokio::test]
async fn the_repair_listing_of_a_character_without_memories_is_empty() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let unknown = CharacterId::from_raw_for_tests(9_999);

    assert!(
        store
            .list_memories_for_repair(aoi)
            .await
            .expect("一覧が返ること")
            .is_empty()
    );
    assert!(
        store
            .list_memories_for_repair(unknown)
            .await
            .expect("一覧が返ること")
            .is_empty(),
        "存在しないキャラクターでも空であること"
    );
}

// ---------------------------------------------------------------------------
// 14a のレビューで「緑を通り抜けた」変異を落とすための見張り番
//
// どれも**フィクスチャがその状態に一度も入っていなかった**ことが原因で、
// アサートを強めても届かない（CLAUDE.md「フィクスチャが『その分岐が効く状態』に
// 一度も入らない」）。ここで作るのは状態そのもの。
// ---------------------------------------------------------------------------

/// **`created_at` を明示して**健全な記憶を1行入れる。主キーを返す。
///
/// # なぜ要るのか
///
/// `add_memory` は `created_at` をスキーマの DEFAULT（現在時刻）に任せるので、
/// **同時刻の組をテストから作れない。** 並びの第2キー（`id`）が効くのは同着のときだけなので、
/// この状態を作らないと `ORDER BY created_at, id` の `id` は**どの向きでも緑**になる。
///
/// **1ターンで最大5件入る**ので、同着は本番で普通に起きる。
async fn insert_memory_at(
    store: &Store,
    character_id: CharacterId,
    content: &str,
    created_at: &str,
) -> i64 {
    let done = sqlx::query(
        "INSERT INTO memory (character_id, subject, content, created_at) \
         VALUES (?, 'user', ?, ?)",
    )
    .bind(character_id.get())
    .bind(content)
    .bind(created_at)
    .execute(store.pool())
    .await
    .expect("created_at を明示した INSERT が成功すること");
    done.last_insert_rowid()
}

/// **`locked = 1` の健全な**記憶を1行入れる。主キーを返す。
///
/// `store` には `locked` を立てる API が無い（ADR-0022 §4 / 手動運用の想定）ので直接入れる。
/// CHECK は 0 / 1 を許すので `PRAGMA ignore_check_constraints` は要らない
/// ——**ここで欲しいのは「壊れた行」ではなく「保護印の付いた健全な行」。**
async fn insert_locked_memory(store: &Store, character_id: CharacterId, content: &str) -> i64 {
    let done = sqlx::query(
        "INSERT INTO memory (character_id, subject, content, locked) VALUES (?, 'user', ?, 1)",
    )
    .bind(character_id.get())
    .bind(content)
    .execute(store.pool())
    .await
    .expect("locked = 1 の INSERT が成功すること");
    done.last_insert_rowid()
}

/// 一覧の並びを id で取り出す（読めた行・読めなかった行を混ぜたまま）。
fn entry_ids(entries: &[MemoryEntry]) -> Vec<i64> {
    entries
        .iter()
        .map(|entry| match entry {
            MemoryEntry::Readable(memory) => memory.id.get(),
            MemoryEntry::Unreadable(broken) => broken.id.get(),
        })
        .collect()
}

/// **`locked = 1` の行は「読めた行」として一覧に並び、`locked` は true で読める。**
///
/// ADR-0022 §4 は `locked` を**読めるかだけ見る**と決めている
/// ——**絞り込みには使わないし、書き込みもしない。**
/// この見張り番が無いと、次が全部緑を通る（14a のレビューで実測）:
///
/// * `1 => Some(true)` を `Some(false)` に — 保護印の付いた記憶が常に「未ロック」に見える
/// * `1 => Some(true)` を `None` に — **健全な行が「壊れた行」として並ぶ**
///   （運用者が壊れていない記憶を訂正しに行く）
/// * 一覧の `WHERE` に `AND locked <> 1` を足す — **その行が修復の入口から消える**
///
/// # フィクスチャは 0 と 1 を両方入れる
///
/// 片方だけだと「常に false」「常に true」と区別が付かない。
#[tokio::test]
async fn a_locked_memory_is_listed_for_repair_and_still_reads_as_locked() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let unlocked = store
        .add_memory(aoi, &plain(Subject::User, "普通の記憶"))
        .await
        .expect("記憶を追加できること");
    let locked = insert_locked_memory(&store, aoi, "保護印の付いた記憶").await;

    let entries = store
        .list_memories_for_repair(aoi)
        .await
        .expect("一覧が返ること");

    assert_eq!(
        entry_ids(&entries),
        vec![unlocked.get(), locked],
        "locked = 1 の行を一覧から外さないこと（絞り込みに使わない / ADR-0022 §4）: {entries:?}"
    );

    let flags: Vec<(i64, bool)> = entries
        .iter()
        .map(|entry| match entry {
            MemoryEntry::Readable(memory) => (memory.id.get(), memory.locked),
            MemoryEntry::Unreadable(broken) => {
                panic!("locked = 1 は壊れた値ではないこと: {broken:?}")
            }
        })
        .collect();
    assert_eq!(
        flags,
        vec![(unlocked.get(), false), (locked, true)],
        "0 と 1 が別の値として読めること"
    );
}

/// **別キャラクターの・すでに上書き済みの記憶**を指すと、返るのは**宛先違い**。
///
/// # なぜこの組み合わせが要るのか
///
/// 拒否は3種あるが、フィクスチャが**一度に1つしか成立させていなかった。**
/// そのため「別キャラクターの拒否を `&& successor.is_none()` で条件化する」変異が
/// 緑を通った（14a のレビューで実測）。実害は2つ:
///
/// * **`MemoryAlreadySuperseded` は「その記憶は存在し、訂正済みである」という
///   他キャラクターの状態の開示**になる。宛先違いは中身を一切明かさない。
/// * 運用者の調査先が変わる（「有効な方を指せばよい」と読んで、他人の鎖を辿りに行く）。
///
/// **宛先違いを先に返す。** 持ち主が違う時点で、鎖の状態はこの呼び出しには関係ない。
#[tokio::test]
async fn a_superseded_memory_of_another_character_is_still_rejected_as_wrong_owner() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;

    let rins_old = store
        .add_memory(rin, &plain(Subject::User, "凛に話したこと"))
        .await
        .expect("記憶を追加できること");
    let rins_new = store
        .supersede_memory(
            rin,
            rins_old,
            &plain(Subject::User, "凛に話したこと（訂正）"),
        )
        .await
        .expect("持ち主なら訂正できること");

    // **2つの条件が同時に成立している**ことを、フィクスチャの側で固定する。
    assert_eq!(
        superseded_by_of(&store, rins_old.get()).await,
        Some(rins_new.get()),
        "上書き済みであること"
    );

    let err = store
        .supersede_memory(aoi, rins_old, &plain(Subject::User, "葵からの横取り"))
        .await
        .expect_err("別キャラクターの記憶は訂正できないこと");
    assert!(
        matches!(err, StoreError::MemoryBelongsToAnotherCharacter),
        "宛先違いを先に返すこと（他キャラクターの鎖の状態を明かさない）: {err:?}"
    );

    assert_eq!(
        memory_count(&store).await,
        2,
        "拒否された呼び出しは訂正版を書き残さないこと"
    );
    assert_eq!(
        superseded_by_of(&store, rins_old.get()).await,
        Some(rins_new.get()),
        "他人の鎖を差し替えないこと"
    );
}

/// **同時刻に入った記憶は `id` の昇順で並ぶ**——想起（`recall_memories`）と
/// 修復の一覧（`list_memories_for_repair`）の**両方**。
///
/// # フィクスチャが作る状態
///
/// * **同じ `created_at` の組**（第2キーの `id` が効くのはこのときだけ）
/// * **`id` の昇順と `created_at` の昇順がずれている**（第1キーを落とす変異を落とす）
///
/// どちらも `add_memory` だけでは作れない（`created_at` は DEFAULT の現在時刻）。
/// この状態が無いと `ORDER BY created_at, id` を `, id DESC` にしても
/// **両側とも緑**を通る（14a のレビューで実測。想起側は 14a が作った穴ではなく、
/// 同じ形が写されていたもの）。
#[tokio::test]
async fn memories_created_in_the_same_millisecond_are_ordered_by_id() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    // **後の時刻の行を最初に入れる。** id の昇順と created_at の昇順をずらすため。
    let late = insert_memory_at(&store, aoi, "あとの時刻の記憶", "2026-08-22T10:00:01.000Z").await;
    let tie_first =
        insert_memory_at(&store, aoi, "同時刻の記憶A", "2026-08-22T10:00:00.000Z").await;
    let tie_second =
        insert_memory_at(&store, aoi, "同時刻の記憶B", "2026-08-22T10:00:00.000Z").await;

    assert!(
        late < tie_first && tie_first < tie_second,
        "id の昇順が late < A < B であること: {late} / {tie_first} / {tie_second}"
    );

    let recalled = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること");
    assert_eq!(
        recalled[0].created_at, recalled[1].created_at,
        "先頭2件が**同着**であること（この状態が無いと第2キーは観測できない）"
    );
    assert_ne!(
        recalled[0].created_at, recalled[2].created_at,
        "第1キーが効いていること（全件同着では ORDER BY created_at を落とす変異が緑になる）"
    );
    assert_eq!(
        recalled.iter().map(|m| m.id.get()).collect::<Vec<i64>>(),
        vec![tie_first, tie_second, late],
        "同着は id の昇順、時刻が違えば時刻順であること"
    );
    assert_eq!(
        contents(&recalled),
        vec!["同時刻の記憶A", "同時刻の記憶B", "あとの時刻の記憶"],
        "並びは本文の側から見ても同じであること"
    );

    let entries = store
        .list_memories_for_repair(aoi)
        .await
        .expect("一覧が返ること");
    assert_eq!(
        entry_ids(&entries),
        vec![tie_first, tie_second, late],
        "修復の一覧も同じ並びであること"
    );
}

// ---------------------------------------------------------------------------
// 上書きされた記憶を読む入口（ADR-0022 / 作業単位 D-3b）
//
// **想起でも、修復の一覧でもない3本目の読み**——「消さずに残した行」を外から見る
// ための経路。ここが無いと「時系列保持」は書き込み専用に見える。
//
// 想起（`recall_memories`）と修復の一覧（`list_memories_for_repair`）は
// **どちらも `superseded_by IS NULL` しか読まない。** そこは1バイトも触らない
// ——触ると、訂正した記憶が想起へ戻る。
// ---------------------------------------------------------------------------

/// 上書き済みの一覧を `(その行の id, 後継の id)` の組で取り出す。
///
/// **読めた行と読めなかった行を混ぜたまま**返す（並びは1本なので分けない）。
fn superseded_links(listed: &[SupersededMemory]) -> Vec<(i64, i64)> {
    listed
        .iter()
        .map(|old| {
            let id = match &old.entry {
                MemoryEntry::Readable(memory) => memory.id.get(),
                MemoryEntry::Unreadable(broken) => broken.id.get(),
            };
            (id, old.superseded_by.get())
        })
        .collect()
}

/// **上書きされた行が、そのままの中身で、後継を指したまま返る。**
///
/// # フィクスチャが同値を作っていないこと
///
/// * 古い行と訂正版は **`subject` も `content` も `category` も違う**
///   ——訂正版の列を古い行の欄に載せる実装（またはその逆）が赤になる。
/// * **後継の id は自分の id ではない**（`superseded_by` を `id` に取り違える変異）。
/// * **有効なまま残る行が別にある**——「全部返す」実装が赤になる。
#[tokio::test]
async fn the_superseded_listing_returns_the_old_row_and_the_row_that_replaced_it() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    // **有効なまま残る行。** 上書き済みの一覧には現れてはいけない。
    let active = store
        .add_memory(
            aoi,
            &categorized(Subject::Character, "図書館で働いている", "経歴"),
        )
        .await
        .expect("記憶を追加できること");
    let wrong = store
        .add_memory(
            aoi,
            &categorized(Subject::User, "黒い犬を飼っている", "家族"),
        )
        .await
        .expect("記憶を追加できること");
    let correction = store
        .supersede_memory(aoi, wrong, &plain(Subject::Character, "黒い猫を飼っている"))
        .await
        .expect("訂正できること");

    assert_ne!(wrong, correction, "前提: 古い行と訂正版が別の行であること");
    assert_ne!(wrong, active, "前提: 有効な行と別の行であること");

    let listed = store
        .list_superseded_memories(aoi, 10)
        .await
        .expect("上書き済みの一覧が返ること");

    assert_eq!(
        superseded_links(&listed),
        vec![(wrong.get(), correction.get())],
        "上書きされた行だけが、自分を置き換えた行を指して返ること"
    );

    let MemoryEntry::Readable(memory) = &listed[0].entry else {
        panic!("健全な行は読めた行として返ること: {listed:?}");
    };
    assert_eq!(memory.id, wrong);
    assert_eq!(memory.character_id, aoi);
    assert_eq!(
        memory.subject,
        Subject::User,
        "**古い行の** subject であること（訂正版は character）"
    );
    assert_eq!(
        memory.content.as_str(),
        "黒い犬を飼っている",
        "**古い行の**本文がそのまま残っていること（時系列保持）"
    );
    assert_eq!(
        memory.category.as_ref().map(MemoryCategory::as_str),
        Some("家族"),
        "**古い行の**分類であること（訂正版は分類なし）"
    );

    assert_eq!(
        store
            .count_superseded_memories(aoi)
            .await
            .expect("総数を数えられること"),
        1
    );

    // **有効な側は今までどおり。** 訂正版と、触っていない行の2件。
    let mut active_ids: Vec<i64> = entry_ids(
        &store
            .list_memories_for_repair(aoi)
            .await
            .expect("修復の一覧が返ること"),
    );
    active_ids.sort_unstable();
    assert_eq!(
        active_ids,
        vec![active.get(), correction.get()],
        "修復の一覧は有効な行だけのままであること"
    );
}

/// **上書き済みの一覧も、総数も、キャラクターで閉じている。**
///
/// 両側から見る——片方だけだと「常にこのキャラクターの行を返す」実装と区別が付かない。
/// **相手の行の方が新しい**ので、絞り込みが消えたら先頭に現れる。
#[tokio::test]
async fn the_superseded_listing_is_scoped_to_one_character() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;
    assert_ne!(aoi, rin, "前提: 2体の id が別であること");

    let aoi_successor = store
        .add_memory(aoi, &plain(Subject::User, "葵の訂正版"))
        .await
        .expect("記憶を追加できること");
    let aoi_old = insert_superseded_memory_at(
        &store,
        aoi,
        "user",
        "葵の古い記憶",
        None,
        "2026-08-27T10:00:00.000Z",
        aoi_successor.get(),
    )
    .await;

    let rin_successor = store
        .add_memory(rin, &plain(Subject::Character, "凛の訂正版"))
        .await
        .expect("記憶を追加できること");
    // **葵の行より新しい。** 混ざれば先頭に出る。
    let rin_old = insert_superseded_memory_at(
        &store,
        rin,
        "character",
        "凛の古い記憶",
        None,
        "2026-08-27T11:00:00.000Z",
        rin_successor.get(),
    )
    .await;

    assert_eq!(
        superseded_links(
            &store
                .list_superseded_memories(aoi, 10)
                .await
                .expect("一覧が返ること")
        ),
        vec![(aoi_old, aoi_successor.get())],
        "葵の一覧に凛の行が混ざらないこと"
    );
    assert_eq!(
        superseded_links(
            &store
                .list_superseded_memories(rin, 10)
                .await
                .expect("一覧が返ること")
        ),
        vec![(rin_old, rin_successor.get())],
        "凛の一覧に葵の行が混ざらないこと"
    );

    assert_eq!(
        store
            .count_superseded_memories(aoi)
            .await
            .expect("総数を数えられること"),
        1,
        "総数も絞り込みを持つこと"
    );
    assert_eq!(
        store
            .count_superseded_memories(rin)
            .await
            .expect("総数を数えられること"),
        1,
        "総数も絞り込みを持つこと"
    );
}

/// **新しい順。同着は `id` の降順**（索引が使われても、使われなくても）。
///
/// # なぜ索引を落とすのか（`tests/server_admin_drift.rs` と同じ罠）
///
/// 索引 `memory_character_idx (character_id, created_at)` を後ろから辿る計画では、
/// **同着の中が偶然 `id` の降順**になる。つまり第2キーが無くても結果は同じで、
/// 索引がある状態では**第2キーを守れない。**
/// 索引を落とすと計画が「表を走査してから並べ替える」に変わるので、
/// **同じ期待値を2回**（索引あり・なし）突き合わせる。
///
/// 落としているのは**このテスト専用の一時 DB** の索引で、マイグレーションは触らない。
///
/// # 同着は本番で普通に起きる
///
/// **1ターンで最大5件の記憶が入る**（`NewMemory::MAX_PER_TURN`）ので、
/// それらをまとめて訂正すれば `created_at` の同じ上書き済みの行が並ぶ。
///
/// # 後継は3行とも別（「常に同じ行を指す」実装が赤になる）
#[tokio::test]
async fn superseded_rows_come_back_newest_first_with_ties_broken_by_id() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let mut successors = Vec::new();
    for index in 0..3 {
        successors.push(
            store
                .add_memory(aoi, &plain(Subject::User, &format!("訂正版{index}")))
                .await
                .expect("記憶を追加できること")
                .get(),
        );
    }

    // **後の時刻の行を最初に入れる。** id の昇順と created_at の昇順をずらすため。
    let late = insert_superseded_memory_at(
        &store,
        aoi,
        "user",
        "あとの時刻の古い記憶",
        None,
        "2026-08-27T10:00:01.000Z",
        successors[0],
    )
    .await;
    let tie_first = insert_superseded_memory_at(
        &store,
        aoi,
        "user",
        "同時刻の古い記憶A",
        None,
        "2026-08-27T10:00:00.000Z",
        successors[1],
    )
    .await;
    let tie_second = insert_superseded_memory_at(
        &store,
        aoi,
        "user",
        "同時刻の古い記憶B",
        None,
        "2026-08-27T10:00:00.000Z",
        successors[2],
    )
    .await;
    assert!(
        late < tie_first && tie_first < tie_second,
        "前提: id の昇順が late < A < B であること: {late} / {tie_first} / {tie_second}"
    );

    let expected = vec![
        (late, successors[0]),
        (tie_second, successors[2]),
        (tie_first, successors[1]),
    ];

    let listed = store
        .list_superseded_memories(aoi, 10)
        .await
        .expect("一覧が返ること");
    assert_eq!(
        superseded_links(&listed),
        expected,
        "新しい順、同着は id の降順であること（索引あり）"
    );

    // **計画を変える。** 索引が使えないと、同着の中は入った順（id の昇順）で出てくる。
    sqlx::query("DROP INDEX memory_character_idx")
        .execute(store.pool())
        .await
        .expect("索引を落とせること");

    let listed = store
        .list_superseded_memories(aoi, 10)
        .await
        .expect("一覧が返ること");
    assert_eq!(
        superseded_links(&listed),
        expected,
        "索引が無くても同じ並びであること（第2キーが効いている）"
    );
}

/// **訂正の連鎖。鎖のどの環も、自分の後継を指したまま並ぶ。**
///
/// `A → B → C` を作る。**B は上書きされていて、かつ誰かを上書きしている。**
/// この状態が無いと、「後継は必ず最後に書かれた行」「後継は自分の id + 1」のような
/// 決め打ちが緑を通る。
///
/// # id を隣り合わせにしない
///
/// 鎖の環の間に**無関係な行**を挟むので、`id + 1` は後継にならない。
#[tokio::test]
async fn a_chain_of_corrections_keeps_every_link_pointing_at_its_own_successor() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let first = store
        .add_memory(aoi, &plain(Subject::User, "犬を飼っている"))
        .await
        .expect("記憶を追加できること");
    // **無関係な行**（鎖の環の id を隣り合わせにしないため）。
    store
        .add_memory(aoi, &plain(Subject::Character, "図書館で働いている"))
        .await
        .expect("記憶を追加できること");
    let second = store
        .supersede_memory(aoi, first, &plain(Subject::User, "猫を飼っている"))
        .await
        .expect("1度目の訂正ができること");
    store
        .add_memory(aoi, &plain(Subject::Character, "紅茶が好き"))
        .await
        .expect("記憶を追加できること");
    let third = store
        .supersede_memory(aoi, second, &plain(Subject::User, "黒い猫を飼っている"))
        .await
        .expect("2度目の訂正ができること");

    assert_ne!(
        second.get(),
        first.get() + 1,
        "前提: 後継が id + 1 になっていないこと"
    );
    assert_ne!(
        third.get(),
        second.get() + 1,
        "前提: 後継が id + 1 になっていないこと"
    );

    let listed = store
        .list_superseded_memories(aoi, 10)
        .await
        .expect("一覧が返ること");
    assert_eq!(
        superseded_links(&listed),
        vec![(second.get(), third.get()), (first.get(), second.get())],
        "鎖の環がそれぞれ自分の後継を指すこと（新しい順）"
    );
    assert_eq!(
        store
            .count_superseded_memories(aoi)
            .await
            .expect("総数を数えられること"),
        2
    );

    // **鎖の先頭（いま有効な行）は上書き済みではない。**
    assert!(
        !superseded_links(&listed)
            .iter()
            .any(|(id, _)| *id == third.get()),
        "いま有効な行が上書き済みの一覧に現れないこと"
    );
}

/// **上書き済みの壊れた行も「壊れている」と示され、健全な行を巻き添えにしない。**
///
/// 修復の一覧（`list_memories_for_repair`）と同じ形。
/// **黙って読み飛ばす実装（`filter_map`）はそのままコンパイルが通る**ので、
/// 守っているのはこの見張り番だけ（`MemoryEntry` の doc）。
///
/// # なぜ上書き済みの側でも要るのか
///
/// 壊れた行は**訂正して想起から外した後も残る。**
/// 読み飛ばすと、「なぜこの記憶が消えたのか」を辿るときに**壊れた環だけが鎖から落ちる**
/// ——総数には残るので、読み手には数が合わないとしか見えない。
#[tokio::test]
async fn a_broken_superseded_row_is_pointed_at_without_taking_the_healthy_ones() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let healthy_successor = store
        .add_memory(aoi, &plain(Subject::User, "健全な訂正版"))
        .await
        .expect("記憶を追加できること");
    let broken_successor = store
        .add_memory(aoi, &plain(Subject::Character, "壊れた行の訂正版"))
        .await
        .expect("記憶を追加できること");

    let healthy_old = insert_superseded_memory_at(
        &store,
        aoi,
        "character",
        "古いが読める記憶",
        Some("経歴"),
        "2026-08-27T10:00:00.000Z",
        healthy_successor.get(),
    )
    .await;
    // 本文が空白だけの行（長さ・空白の制約はスキーマに無いので素の INSERT で作れる）。
    let broken_old = insert_superseded_memory_at(
        &store,
        aoi,
        "user",
        "\u{3000} \t",
        None,
        "2026-08-27T10:00:01.000Z",
        broken_successor.get(),
    )
    .await;

    let listed = store
        .list_superseded_memories(aoi, 10)
        .await
        .expect("壊れた行があっても一覧が返ること");

    assert_eq!(
        superseded_links(&listed),
        vec![
            (broken_old, broken_successor.get()),
            (healthy_old, healthy_successor.get()),
        ],
        "壊れた行も並び、それぞれの後継を指すこと"
    );

    let MemoryEntry::Unreadable(broken) = &listed[0].entry else {
        panic!("壊れた行は読めなかった行として返ること: {listed:?}");
    };
    assert_eq!(broken.id.get(), broken_old);
    assert_eq!(broken.character_id, aoi);
    assert_eq!(broken.invalid_columns, vec![MemoryColumn::Content]);

    let MemoryEntry::Readable(memory) = &listed[1].entry else {
        panic!("健全な行は読めた行として返ること: {listed:?}");
    };
    assert_eq!(memory.content.as_str(), "古いが読める記憶");
    // **主語はこの行のもの。** もう1件（壊れた行）は `user` なので、
    // 引く文が主語を片方に決め打つ変異は、この2件のどちらかで必ず赤になる
    // （`the_superseded_listing_returns_the_old_row_and_the_row_that_replaced_it` が
    // `user` の側を持っている / 変異検査で実測 / D-3b）。
    assert_eq!(memory.subject, Subject::Character);
    assert_eq!(
        memory.category.as_ref().map(MemoryCategory::as_str),
        Some("経歴"),
        "分類もこの行のものであること"
    );

    assert_eq!(
        store
            .count_superseded_memories(aoi)
            .await
            .expect("総数を数えられること"),
        2,
        "壊れた行も総数に入ること"
    );
}

/// **上限まで返す。総数は上限で切らない。**
///
/// # 期待値を上限の定数から作らない
///
/// この読みの上限は**引数**なので、ここでは外の値（直書きの 4 と 10）で押さえる。
/// HTTP の入口が渡す既定値の権威は
/// `tests/server_admin_memories.rs::only_the_most_recent_superseded_memories_come_back_but_the_total_counts_them_all`。
///
/// # 上限は**行**に効く（読めた行だけを数えるのではない）
///
/// 窓の中に壊れた行を混ぜてあるので、「読めなかった行を上限から除く」実装は
/// 5 件返して赤になる。
#[tokio::test]
async fn the_superseded_listing_stops_at_the_limit_but_the_count_does_not() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let successor = store
        .add_memory(aoi, &plain(Subject::User, "訂正版"))
        .await
        .expect("記憶を追加できること");

    let mut planted = Vec::new();
    for index in 0..10 {
        // 窓の中に入る位置（新しい方から2番目）だけ壊す。
        let content = if index == 8 {
            "\u{3000} \t".to_owned()
        } else {
            format!("古い記憶{index}")
        };
        planted.push(
            insert_superseded_memory_at(
                &store,
                aoi,
                "user",
                &content,
                None,
                &format!("2026-08-27T10:00:{index:02}.000Z"),
                successor.get(),
            )
            .await,
        );
    }

    let listed = store
        .list_superseded_memories(aoi, 4)
        .await
        .expect("一覧が返ること");
    assert_eq!(listed.len(), 4, "上限まで返すこと（行として数える）");
    assert_eq!(
        superseded_links(&listed)
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<i64>>(),
        vec![planted[9], planted[8], planted[7], planted[6]],
        "新しい方から4件であること"
    );
    assert!(
        matches!(listed[1].entry, MemoryEntry::Unreadable(_)),
        "窓の中の壊れた行も1件として数えること: {listed:?}"
    );

    assert_eq!(
        store
            .count_superseded_memories(aoi)
            .await
            .expect("総数を数えられること"),
        10,
        "総数は上限で切らないこと"
    );
}

/// **上書きされた行が1件も無ければ空。存在しないキャラクターでも panic しない。**
///
/// 有効な記憶だけを持つキャラクターで測る——「記憶があれば何か返す」実装が赤になる。
#[tokio::test]
async fn a_character_without_superseded_memories_lists_nothing_and_counts_zero() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let unknown = CharacterId::from_raw_for_tests(9_999);

    store
        .add_memory(aoi, &plain(Subject::User, "上書きされていない記憶"))
        .await
        .expect("記憶を追加できること");

    assert!(
        store
            .list_superseded_memories(aoi, 10)
            .await
            .expect("一覧が返ること")
            .is_empty(),
        "有効な行だけのキャラクターでは空であること"
    );
    assert_eq!(
        store
            .count_superseded_memories(aoi)
            .await
            .expect("総数を数えられること"),
        0
    );
    assert!(
        store
            .list_superseded_memories(unknown, 10)
            .await
            .expect("一覧が返ること")
            .is_empty(),
        "存在しないキャラクターでも空であること"
    );
    assert_eq!(
        store
            .count_superseded_memories(unknown)
            .await
            .expect("総数を数えられること"),
        0
    );
}

/// **本番の書き込み口（`supersede_memory`）が残したものが、そのまま見える。**
///
/// 他のテストの仕込みは素の SQL なので、**書き手と読み手が繋がっていること**は
/// ここでしか観測できない（列の綴りが片方だけ変わったら赤になる）。
#[tokio::test]
async fn the_superseded_listing_shows_what_the_production_writer_left_behind() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let wrong = store
        .add_memory(
            aoi,
            &categorized(Subject::User, "店を経営している", "しごと"),
        )
        .await
        .expect("記憶を追加できること");
    let correction = store
        .supersede_memory(
            aoi,
            wrong,
            &categorized(Subject::User, "喫茶店でアルバイトをしている", "経歴"),
        )
        .await
        .expect("訂正できること");

    let listed = store
        .list_superseded_memories(aoi, 10)
        .await
        .expect("一覧が返ること");
    assert_eq!(
        superseded_links(&listed),
        vec![(wrong.get(), correction.get())]
    );

    let MemoryEntry::Readable(memory) = &listed[0].entry else {
        panic!("読めた行であること: {listed:?}");
    };
    assert_eq!(memory.content.as_str(), "店を経営している");
    assert_eq!(
        memory.category.as_ref().map(MemoryCategory::as_str),
        Some("しごと")
    );
    let now = Utc::now();
    assert!(
        memory.created_at >= now - CLOCK_SKEW && memory.created_at <= now + CLOCK_SKEW,
        "created_at が現在時刻の近傍であること: {}",
        memory.created_at
    );

    // **想起からは消えている**（この作業単位で想起を1バイトも変えていないこと）。
    let recalled = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起が成功すること");
    assert_eq!(contents(&recalled), vec!["喫茶店でアルバイトをしている"]);
}
