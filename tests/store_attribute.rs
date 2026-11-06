//! `character_attribute` の永続化の検証（ADR-0009 / 作業単位 12a）。
//!
//! ここで確かめるのは主に次の6点:
//!   * **往復**——保存した属性がキー・値・`subject`・**並び**まで含めて読み戻せる。
//!   * **`subject` を落とさない**——同じキーを `user` と `character` の両方に持つ
//!     キャラクターで測る（片方しか持たないフィクスチャでは取り違えを検出できない）。
//!   * **丸ごと置き換え**——要求に含まれない属性は消える（`stat_alloc` と同じ意味論）。
//!   * **件数の上限**——8 件は通り、9 件は `AttributeSet` が作らせない。
//!   * **読み出しの検証**——生 SQL で作った規格外の行は読めず、**`PUT` で直せる。**
//!   * **他のキャラクターの属性を返さない。**
//!
//! スキーマ制約そのものの検証は `tests/store_schema.rs` にある。

use personacore::domain::{
    AttributeKey, AttributeSet, AttributeValue, CharacterAttribute, CharacterId, DisplayName,
    Handle, Persona, StatScale, Subject,
};
use personacore::store::{AttributeRevision, Store, StoreError};
use sqlx::error::ErrorKind;
use tempfile::TempDir;

mod common;

use common::{
    fixture_scale, insert_attribute_revision_ignoring_checks,
    insert_character_attribute_ignoring_checks,
};

/// 一時ディレクトリ上に新規 DB を作り、マイグレーション適用済みの `Store` を返す。
async fn setup() -> (TempDir, Store) {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let store = Store::connect(dir.path().join("personacore.sqlite"))
        .await
        .expect("接続とマイグレーション適用が成功すること");
    (dir, store)
}

/// キャラクターを1件作って id を返す。**識別子・表示名・ペルソナは全部違う文字列。**
async fn character(store: &Store, handle: &str, display_name: &str) -> CharacterId {
    assert_ne!(
        handle, display_name,
        "識別子と表示名を同値にしないこと（取り違えが検出できなくなる）"
    );
    store
        .create_character(
            &Handle::parse(handle).expect("正当な識別子であること"),
            &DisplayName::parse(display_name).expect("正当な表示名であること"),
            &Persona::parse("ペルソナ本文").expect("正当なペルソナであること"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること")
}

/// 1件分の属性。**キーと値を同じ文字列にしないこと。**
fn attribute(subject: Subject, key: &str, value: &str) -> CharacterAttribute {
    assert_ne!(
        key, value,
        "キーと値を同値にしないこと（取り違えが検出できなくなる）"
    );
    CharacterAttribute {
        subject,
        key: AttributeKey::parse(key).expect("正当なキーであること"),
        value: AttributeValue::parse(value).expect("正当な値であること"),
    }
}

fn set(attributes: Vec<CharacterAttribute>) -> AttributeSet {
    AttributeSet::parse(attributes).expect("件数が上限内であること")
}

/// **このファイルの基準フィクスチャ。**
///
/// 並びと `subject` を次のように仕込んである。落とすと検出力が消える:
///
/// | # | subject | key | value |
/// |---|---|---|---|
/// | 0 | character | 一人称 | 僕 |
/// | 1 | user | 年齢 | 32 |
/// | 2 | user | 呼び方 | 先輩 |
/// | 3 | character | 年齢 | 19 |
/// | 4 | character | 種族 | 猫獣人 |
///
/// * **`user` と `character` の両方を持つ**——片方だけだと `subject` の取り違えが
///   「たまたま全部正しい」形で緑を通る。
/// * **同じキー（`年齢`）を両方の主語が持ち、値が違う**——`subject` を捨てる実装は
///   ここで必ず崩れる（ADR-0009 が `subject` を持たせた理由そのもの）。
/// * **並びと `subject` を相関させない**（C, U, U, C, C）——`subject` を位置から
///   決め打つ実装を通さないため。
/// * **先頭・中間・末尾のどれを落としても結果が変わる。**
fn fixture() -> Vec<CharacterAttribute> {
    vec![
        attribute(Subject::Character, "一人称", "僕"),
        attribute(Subject::User, "年齢", "32"),
        attribute(Subject::User, "呼び方", "先輩"),
        attribute(Subject::Character, "年齢", "19"),
        attribute(Subject::Character, "種族", "猫獣人"),
    ]
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

/// **往復。** キー・値・`subject`・並びがすべて保たれる。
#[tokio::test]
async fn replacing_then_reading_round_trips_every_attribute() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    store
        .replace_character_attributes(id, &set(fixture()))
        .await
        .expect("属性を保存できること");

    let stored = store
        .character_attributes(id)
        .await
        .expect("属性を読み出せること");

    assert_eq!(
        stored,
        fixture(),
        "キー・値・subject・並びがすべて保たれること"
    );
}

/// **`subject` の取り違えが赤になる。**
///
/// `user` と `character` の**両方**が同じキー（`年齢`）を持ち、**値が違う**状態で測る。
/// 読み出しが `subject` を捨てて `character` 固定にすると、
/// 「利用者の 32 歳」がキャラクターのものとして読まれる。
#[tokio::test]
async fn the_subject_of_every_attribute_survives_the_round_trip() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    store
        .replace_character_attributes(id, &set(fixture()))
        .await
        .expect("属性を保存できること");

    let stored = store
        .character_attributes(id)
        .await
        .expect("属性を読み出せること");

    // 主語ごとに分けて、**両方に行があること**を先に確かめる
    // （片方が空だと、以下の突き合わせが素通りしてしまう）。
    let of = |subject: Subject| -> Vec<(String, String)> {
        stored
            .iter()
            .filter(|attribute| attribute.subject == subject)
            .map(|attribute| {
                (
                    attribute.key.as_str().to_owned(),
                    attribute.value.as_str().to_owned(),
                )
            })
            .collect()
    };

    assert_eq!(
        of(Subject::User),
        vec![
            ("年齢".to_owned(), "32".to_owned()),
            ("呼び方".to_owned(), "先輩".to_owned()),
        ],
        "利用者の属性だけが利用者側に載ること"
    );
    assert_eq!(
        of(Subject::Character),
        vec![
            ("一人称".to_owned(), "僕".to_owned()),
            ("年齢".to_owned(), "19".to_owned()),
            ("種族".to_owned(), "猫獣人".to_owned()),
        ],
        "キャラクターの属性だけがキャラクター側に載ること"
    );

    // **同じキーが両方に居て、値が食い違っている**ことを明示する
    // （このフィクスチャが取り違えを検出できる根拠）。
    let ages: Vec<(Subject, &str)> = stored
        .iter()
        .filter(|attribute| attribute.key.as_str() == "年齢")
        .map(|attribute| (attribute.subject, attribute.value.as_str()))
        .collect();
    assert_eq!(
        ages,
        vec![(Subject::User, "32"), (Subject::Character, "19")],
        "同じキーでも主語で値が割れること"
    );
}

/// **丸ごと置き換え。** 要求に含まれない属性は消える（追記ではない）。
///
/// **先頭・中間・末尾の3つを同時に入れ替える**——1箇所だけ変える形にすると、
/// 「追記してから重複だけ上書きする」実装が通り抜ける余地が残る。
#[tokio::test]
async fn replacing_is_wholesale_not_an_append() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    store
        .replace_character_attributes(id, &set(fixture()))
        .await
        .expect("最初の属性を保存できること");

    let replacement = vec![
        attribute(Subject::Character, "一人称", "俺"),
        attribute(Subject::User, "出身", "北国"),
    ];
    store
        .replace_character_attributes(id, &set(replacement.clone()))
        .await
        .expect("属性を置き換えられること");

    let stored = store
        .character_attributes(id)
        .await
        .expect("属性を読み出せること");
    assert_eq!(
        stored, replacement,
        "要求に含まれない属性は消えること（追記にしない）"
    );

    // 消えたことを行数でも確かめる（`Vec` の比較だけだと、読み出し側で
    // 絞り込んでいても緑になる）。
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM character_attribute")
        .fetch_one(store.pool())
        .await
        .expect("行数を数えられること");
    assert_eq!(rows, 2, "DB にも 2 行しか残らないこと");
}

/// **空の集合を渡すと全属性が消える。**
#[tokio::test]
async fn an_empty_set_clears_every_attribute() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    store
        .replace_character_attributes(id, &set(fixture()))
        .await
        .expect("属性を保存できること");
    store
        .replace_character_attributes(id, &set(Vec::new()))
        .await
        .expect("空の集合で置き換えられること");

    assert_eq!(
        store
            .character_attributes(id)
            .await
            .expect("読み出せること"),
        Vec::new(),
        "全属性が消えること"
    );
}

/// **他のキャラクターの属性を返さない。**
///
/// 2 人とも属性を持ち、**件数も中身も違う**状態で測る
/// （同じにすると、取り違えても結果が一致してしまう）。
#[tokio::test]
async fn attributes_of_another_character_are_never_returned() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let rin = character(&store, "rin", "凛").await;

    store
        .replace_character_attributes(aoi, &set(fixture()))
        .await
        .expect("葵の属性を保存できること");
    let rin_attributes = vec![attribute(Subject::Character, "一人称", "わたし")];
    store
        .replace_character_attributes(rin, &set(rin_attributes.clone()))
        .await
        .expect("凛の属性を保存できること");

    assert_eq!(
        store.character_attributes(aoi).await.expect("読めること"),
        fixture(),
        "葵の属性が凛のもので上書きされていないこと"
    );
    assert_eq!(
        store.character_attributes(rin).await.expect("読めること"),
        rin_attributes,
        "凛には凛の属性だけが返ること"
    );

    // **置き換えも巻き添えにしない。**
    store
        .replace_character_attributes(aoi, &set(Vec::new()))
        .await
        .expect("葵の属性を消せること");
    assert_eq!(
        store.character_attributes(rin).await.expect("読めること"),
        rin_attributes,
        "他人の置き換えで凛の属性が消えないこと"
    );
}

/// 属性を1件も持たないキャラクターは空の一覧。
#[tokio::test]
async fn a_character_without_attributes_reads_as_empty() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    assert_eq!(
        store.character_attributes(id).await.expect("読めること"),
        Vec::new()
    );
}

/// **存在しないキャラクターへの置き換え**は、空なら成功し、空でなければ外部キーで止まる。
///
/// `replace_stat_allocs` と同じ扱い（「宛先が無い」ことの報告を外部キー1箇所に保つ）。
#[tokio::test]
async fn replacing_attributes_of_an_unknown_character() {
    let (_dir, store) = setup().await;
    let unknown = CharacterId::from_raw_for_tests(9_999);

    store
        .replace_character_attributes(unknown, &set(Vec::new()))
        .await
        .expect("空の置き換えは DELETE の冪等性として成功すること");

    let err = store
        .replace_character_attributes(unknown, &set(fixture()))
        .await
        .expect_err("宛先が無ければ外部キーで止まること");
    assert_query_kind(
        &err,
        ErrorKind::ForeignKeyViolation,
        "存在しない character_id",
    );

    assert_eq!(
        store
            .character_attributes(unknown)
            .await
            .expect("読めること"),
        Vec::new(),
        "1行も書かれていないこと"
    );
}

/// **同じ `(subject, key)` の重複は主キー違反**になり、置き換え全体が巻き戻る。
///
/// 判定を `AttributeSet` に持たせていないので（`stat_alloc` の軸と同じ扱い）、
/// **弾いているのが DB であること**をここで固定する。
#[tokio::test]
async fn a_duplicate_key_for_the_same_subject_rolls_the_whole_replacement_back() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    store
        .replace_character_attributes(id, &set(fixture()))
        .await
        .expect("最初の属性を保存できること");

    let duplicated = vec![
        attribute(Subject::Character, "一人称", "僕"),
        // 正規化すると上と同じキーになる。
        attribute(Subject::Character, " 一人称 ", "俺"),
    ];
    let err = store
        .replace_character_attributes(id, &set(duplicated))
        .await
        .expect_err("重複したキーは弾かれること");
    assert!(err.is_unique_violation(), "主キー違反であること: {err:?}");

    assert_eq!(
        store.character_attributes(id).await.expect("読めること"),
        fixture(),
        "巻き戻って前の設定が残ること"
    );
}

/// **主語が違えば同じキーを 2 件持てる**（主キーに `subject` が入っている根拠）。
#[tokio::test]
async fn the_same_key_is_allowed_once_per_subject() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    let both = vec![
        attribute(Subject::User, "年齢", "32"),
        attribute(Subject::Character, "年齢", "19"),
    ];
    store
        .replace_character_attributes(id, &set(both.clone()))
        .await
        .expect("主語が違えば同じキーを持てること");

    assert_eq!(
        store.character_attributes(id).await.expect("読めること"),
        both
    );
}

/// **件数の上限を拒否しているのは `AttributeSet::parse`。**
///
/// `store` は数え直さない（写しを作らない）ので、**9 件を渡す形が存在しない**ことを
/// ここで固定する。8 件ちょうどは通ることも同時に見る
/// （上限そのものを 0 にする変異を「9 が通らない」だけでは捕まえられない）。
#[tokio::test]
async fn the_count_limit_is_enforced_before_the_store_is_reached() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    let many = |count: usize| -> Vec<CharacterAttribute> {
        (0..count)
            .map(|index| {
                attribute(
                    Subject::Character,
                    &format!("キー{index}"),
                    &format!("値{index}"),
                )
            })
            .collect()
    };

    let at_limit =
        AttributeSet::parse(many(AttributeSet::MAX_LEN)).expect("上限ちょうどは受理されること");
    store
        .replace_character_attributes(id, &at_limit)
        .await
        .expect("上限ちょうどは保存できること");
    assert_eq!(
        store
            .character_attributes(id)
            .await
            .expect("読めること")
            .len(),
        AttributeSet::MAX_LEN,
        "8 件すべてが保存されること"
    );

    // **1 件超えたら集合そのものが作れない。** `store` には届かない。
    assert!(
        AttributeSet::parse(many(AttributeSet::MAX_LEN + 1)).is_err(),
        "上限を超える集合は作れないこと（store へ渡す形が存在しない）"
    );
    assert_eq!(
        store
            .character_attributes(id)
            .await
            .expect("読めること")
            .len(),
        AttributeSet::MAX_LEN,
        "拒否された要求で DB が変わらないこと"
    );
}

/// **空の値は保存できない。**
///
/// 二重の防御を両方見る。`AttributeValue::parse` が作らせず、
/// スキーマの CHECK も生 SQL を弾く。**「性格値 0 の軸」と同じ扱いにしない**
/// ——あちらは 0 が正当な保存値だが、空文字はどちらの層も通さない。
#[tokio::test]
async fn an_empty_value_can_be_stored_by_no_route() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    for raw in ["", " ", "\u{3000}", "\t\r\n"] {
        assert!(
            AttributeValue::parse(raw).is_err(),
            "アプリ層が空の値を作らせないこと: raw = {raw:?}"
        );

        let err = sqlx::query(
            "INSERT INTO character_attribute (character_id, subject, key, value) \
             VALUES (?, 'character', '種族', ?)",
        )
        .bind(id.get())
        .bind(raw)
        .execute(store.pool())
        .await
        .expect_err("スキーマも空の値を弾くこと");
        assert_eq!(
            err.as_database_error()
                .map(sqlx::error::DatabaseError::kind),
            Some(ErrorKind::CheckViolation),
            "CHECK 違反であること: raw = {raw:?}: {err}"
        );
    }

    // 空のキーも同じ（キーのない「: 猫獣人」を載せないため）。
    for raw in ["", " ", "\u{3000}"] {
        assert!(AttributeKey::parse(raw).is_err(), "raw = {raw:?}");
    }
}

/// **前後に空白のある値が格納されていたら、正規化して読む。**
#[tokio::test]
async fn a_padded_stored_key_and_value_are_normalized_on_read() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    // CHECK は「trim すると空でない」しか見ないので、素の SQL で入れられる。
    sqlx::query(
        "INSERT INTO character_attribute (character_id, subject, key, value) \
         VALUES (?, 'character', ?, ?)",
    )
    .bind(id.get())
    .bind(" 一人称 ")
    .bind("\u{3000}僕\u{3000}")
    .execute(store.pool())
    .await
    .expect("前後に空白のある行を作れること");

    assert_eq!(
        store.character_attributes(id).await.expect("読めること"),
        vec![attribute(Subject::Character, "一人称", "僕")],
        "正規化して返すこと"
    );
}

/// **上限を超えるキーが格納されていると読めない。そして `PUT` で直せる。**
///
/// 先例は `store_character` の
/// `a_broken_stat_scale_column_puts_the_replacement_out_of_reach`。
/// **「直せるはず」で書かず、実際に直す手順を測る。**
///
/// ```text
/// character_attributes(id)                 → Err(InvalidStoredValue)
/// replace_character_attributes(id, 正当な集合) → Ok        ← 既存の行を読まないので届く
/// character_attributes(id)                 → Ok(正当な集合)
/// ```
#[tokio::test]
async fn an_over_long_stored_key_stops_the_read_and_the_replacement_fixes_it() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    store
        .replace_character_attributes(id, &set(fixture()))
        .await
        .expect("正当な属性を保存できること");

    let over_long = "あ".repeat(AttributeKey::MAX_CHARS + 1);
    insert_character_attribute_ignoring_checks(&store, id.get(), "character", &over_long, "猫獣人")
        .await;

    assert!(
        matches!(
            store.character_attributes(id).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "1 行壊れると読み出しが丸ごと止まること"
    );

    // **復旧の1手目がそのまま最後の手。** 置き換えは既存の行を読まない。
    let fixed = vec![attribute(Subject::Character, "種族", "猫獣人")];
    store
        .replace_character_attributes(id, &set(fixed.clone()))
        .await
        .expect("置き換えは壊れた行があっても届くこと");

    assert_eq!(
        store
            .character_attributes(id)
            .await
            .expect("直った後は読めること"),
        fixed,
        "壊れた行ごと入れ替わること"
    );
}

/// **上限を超える値・未知の `subject` でも同じ**（読めない / 置き換えで直る）。
#[tokio::test]
async fn other_broken_columns_stop_the_read_and_are_fixed_the_same_way() {
    let over_long_value = "い".repeat(AttributeValue::MAX_CHARS + 1);
    let broken: [(&str, &str, &str); 3] = [
        // (subject, key, value)
        ("character", "種族", over_long_value.as_str()),
        ("assistant", "種族", "猫獣人"),
        ("", "種族", "猫獣人"),
    ];

    for (subject, key, value) in broken {
        let (_dir, store) = setup().await;
        let id = character(&store, "aoi", "葵").await;

        insert_character_attribute_ignoring_checks(&store, id.get(), subject, key, value).await;

        assert!(
            matches!(
                store.character_attributes(id).await,
                Err(StoreError::InvalidStoredValue)
            ),
            "読み出しが止まること: subject = {subject:?} / value = {value:?}"
        );

        let fixed = vec![attribute(Subject::Character, "一人称", "僕")];
        store
            .replace_character_attributes(id, &set(fixed.clone()))
            .await
            .unwrap_or_else(|err| panic!("置き換えが届くこと: subject = {subject:?}: {err}"));
        assert_eq!(
            store
                .character_attributes(id)
                .await
                .expect("直った後は読めること"),
            fixed
        );
    }
}

/// **キャラクターを消すと属性も消える**（`ON DELETE CASCADE`）。
///
/// ここで見るのは「会話の無いキャラクターを消したら属性が残らない」こと。
/// `store` の削除経路（`delete_character`）を通す。
#[tokio::test]
async fn deleting_a_character_takes_its_attributes_with_it() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let rin = character(&store, "rin", "凛").await;

    store
        .replace_character_attributes(aoi, &set(fixture()))
        .await
        .expect("葵の属性を保存できること");
    let kept = vec![attribute(Subject::Character, "一人称", "わたし")];
    store
        .replace_character_attributes(rin, &set(kept.clone()))
        .await
        .expect("凛の属性を保存できること");

    store
        .delete_character(&Handle::parse("aoi").expect("正当な識別子"))
        .await
        .expect("会話の無いキャラクターを削除できること");

    assert_eq!(
        store.character_attributes(aoi).await.expect("読めること"),
        Vec::new(),
        "消えたキャラクターの属性は残らないこと"
    );
    assert_eq!(
        store.character_attributes(rin).await.expect("読めること"),
        kept,
        "巻き添えにしないこと"
    );
}

// =========================================================================
// 属性の履歴（ADR-0028 §決定5 / 作業単位 A-3a）
//
// 履歴は**追記だけ**の表で、`character_attribute` はその投影。
// 書き込みの経路は `replace_character_attributes` ひとつ（走査の見張り番は
// `src/store/attribute.rs` の中）。ここで測るのは**振る舞い**。
// =========================================================================

/// 履歴1件を「主語, キー, 前, 後」に均す。**id と時刻は別に見る。**
fn shape(revision: &AttributeRevision) -> (Subject, &str, Option<&str>, Option<&str>) {
    (
        revision.subject(),
        revision.key().as_str(),
        revision.before().map(AttributeValue::as_str),
        revision.after().map(AttributeValue::as_str),
    )
}

/// このファイルで履歴を読むときの上限。**既定の定数から作らない**
/// （`server::admin` の上限を倍にする変異が緑を通る形にしないため）。
const READ_LIMIT: u32 = 100;

/// 履歴を読む。**上限は直書き。**
async fn revisions(store: &Store, id: CharacterId) -> Vec<AttributeRevision> {
    store
        .attribute_revisions(id, READ_LIMIT)
        .await
        .expect("履歴を読み出せること")
}

/// **追加・変更・削除の3形が、値ごと履歴に載る**（ADR-0028 §決定5）。
///
/// `before` と `after` を入れ替える変異は、ここで崩れる
/// （**値は全部違う**ので `assert_ne!` では防げない / CLAUDE.md「抜けるのはいつも帰属」）。
#[tokio::test]
async fn every_kind_of_change_reaches_the_history() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    // 1手目: 3件を追加。
    let first = vec![
        attribute(Subject::Character, "一人称", "僕"),
        attribute(Subject::User, "年齢", "32"),
        attribute(Subject::User, "呼び方", "先輩"),
    ];
    store
        .replace_character_attributes(id, &set(first))
        .await
        .expect("属性を保存できること");

    assert_eq!(
        revisions(&store, id)
            .await
            .iter()
            .map(shape)
            .collect::<Vec<_>>(),
        vec![
            // **新しい順**（`created_at DESC, id DESC`）なので、書いた順の逆。
            (Subject::User, "呼び方", None, Some("先輩")),
            (Subject::User, "年齢", None, Some("32")),
            (Subject::Character, "一人称", None, Some("僕")),
        ],
        "追加は before が無く after だけになること"
    );

    // 2手目: 1件を変更・1件を削除・1件を追加（1件はそのまま）。
    let second = vec![
        attribute(Subject::Character, "一人称", "俺"),
        attribute(Subject::User, "年齢", "32"),
        attribute(Subject::Character, "種族", "猫獣人"),
    ];
    store
        .replace_character_attributes(id, &set(second))
        .await
        .expect("属性を置き換えられること");

    let all = revisions(&store, id).await;
    assert_eq!(
        all.iter().take(3).map(shape).collect::<Vec<_>>(),
        vec![
            (Subject::User, "呼び方", Some("先輩"), None),
            (Subject::Character, "種族", None, Some("猫獣人")),
            (Subject::Character, "一人称", Some("僕"), Some("俺")),
        ],
        "変更は両側・削除は after が無い・値がそのまま残ること"
    );
    assert_eq!(
        all.len(),
        6,
        "変わっていない `年齢` は記録されないこと（3 + 3 件）: {:?}",
        all.iter().map(shape).collect::<Vec<_>>()
    );
}

/// **履歴の主語は、その属性自身の主語**（ADR-0028 §決定4 の見張り番の形）。
///
/// **主語だけが違う2つの属性**（同じキー・**同じ値**）を持たせ、**片方だけ**を変える。
/// `subject` を取り違える実装は、変えていない側の主語で記録する。
#[tokio::test]
async fn the_history_keeps_the_subject_of_the_attribute_it_records() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    let both = vec![
        attribute(Subject::User, "年齢", "19"),
        attribute(Subject::Character, "年齢", "19"),
    ];
    store
        .replace_character_attributes(id, &set(both))
        .await
        .expect("同じキーを両方の主語に持てること");

    // **両向きを1本で見る**（片側だけだと、その主語に決め打つ変異が通り抜ける /
    // CLAUDE.md の `D-2` の実測。実際に `Subject::Character` へ決め打つ変異は、
    // 「キャラクター側を変える」腕だけでは緑を通った）。
    let rounds: [(Subject, &str, &str, Subject, &str); 2] = [
        // (変える側の主語, その前の値, その後の値, 変えない側の主語, そちらに残る値)
        (Subject::Character, "19", "20", Subject::User, "19"),
        (Subject::User, "19", "21", Subject::Character, "20"),
    ];
    let mut expected_len = 2;
    for (round, (subject, before, new_value, other, kept)) in rounds.into_iter().enumerate() {
        let changed = vec![
            attribute(subject, "年齢", new_value),
            attribute(other, "年齢", kept),
        ];
        store
            .replace_character_attributes(id, &set(changed))
            .await
            .unwrap_or_else(|err| panic!("{round} 周目の置き換えができること: {err}"));

        expected_len += 1;
        let latest = revisions(&store, id).await;
        assert_eq!(
            shape(&latest[0]),
            (subject, "年齢", Some(before), Some(new_value)),
            "変えた側の主語で1件だけ記録されること（{round} 周目）: {:?}",
            latest.iter().map(shape).collect::<Vec<_>>()
        );
        assert_eq!(
            latest.len(),
            expected_len,
            "変えた1件だけが増えること（{round} 周目）: {:?}",
            latest.iter().map(shape).collect::<Vec<_>>()
        );
    }
}

/// **同じ集合を2回置き換えても、履歴は増えない**（ADR-0028 §決定5）。
///
/// 増えると、画面の履歴が「何も変えていない `PUT`」で埋まり、
/// **静かに変わったものを見つけるための表**という存在理由が消える。
#[tokio::test]
async fn replacing_with_the_same_set_records_nothing() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    for round in 0..3 {
        store
            .replace_character_attributes(id, &set(fixture()))
            .await
            .unwrap_or_else(|err| panic!("{round} 回目の置き換えができること: {err}"));
    }

    let all = revisions(&store, id).await;
    assert_eq!(
        all.len(),
        fixture().len(),
        "1回目の追加だけが残ること: {:?}",
        all.iter().map(shape).collect::<Vec<_>>()
    );
    assert!(
        all.iter().all(|revision| revision.before().is_none()),
        "残っているのは追加だけであること"
    );

    // **並べ替えだけの置き換えでも増えない**（履歴は並びを記録しない）。
    let mut reordered = fixture();
    reordered.reverse();
    store
        .replace_character_attributes(id, &set(reordered))
        .await
        .expect("並べ替えた集合も保存できること");
    assert_eq!(
        revisions(&store, id).await.len(),
        fixture().len(),
        "並べ替えだけでは増えないこと"
    );
}

/// **同じ時刻に入った履歴は、`id` の降順で返る**（`created_at DESC, id DESC`）。
///
/// # 同着は本番で普通に起きる
///
/// **1回の置き換えで最大 8 + 8 件が同じトランザクションで入る**ので、
/// `created_at`（ミリ秒精度）は普通に同値になる。
///
/// # 索引を落とした計画でも測る（`D-3b` の実測）
///
/// `attribute_revision_recent_idx (character_id, created_at)` を後ろから辿ると、
/// **`, id DESC` が無くても `id` の降順で出てくることがある。**
/// 索引を落として計画を変えた状態でも同じ答えになることを、同じテストで見る。
#[tokio::test]
async fn the_history_is_newest_first_even_when_the_times_tie() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    // **同じ時刻の3件**と、**前後の時刻の1件ずつ。** 入れる順は時刻の順ではない
    // （時刻を無視して `id` だけで並べる実装も、`id` を無視する実装も落ちる）。
    let rows: [(&str, &str); 5] = [
        ("2026-09-01T00:00:00.000Z", "古い"),
        ("2026-09-02T00:00:00.000Z", "同着1"),
        ("2026-09-03T00:00:00.000Z", "新しい"),
        ("2026-09-02T00:00:00.000Z", "同着2"),
        ("2026-09-02T00:00:00.000Z", "同着3"),
    ];
    let mut ids = Vec::new();
    for (created_at, value) in rows {
        let inserted = sqlx::query(
            "INSERT INTO attribute_revision (character_id, subject, key, before, after, created_at) \
             VALUES (?, 'character', '一人称', NULL, ?, ?)",
        )
        .bind(id.get())
        .bind(value)
        .bind(created_at)
        .execute(store.pool())
        .await
        .expect("履歴の行を直接入れられること");
        ids.push(inserted.last_insert_rowid());
    }
    assert_eq!(ids.len(), 5, "5件入ること");

    // 期待する並び: 新しい時刻 → 同着は id の降順 → 古い時刻。
    let expected = vec!["新しい", "同着3", "同着2", "同着1", "古い"];
    let read: Vec<String> = revisions(&store, id)
        .await
        .iter()
        .map(|revision| {
            revision
                .after()
                .map(|value| value.as_str().to_owned())
                .expect("after が入っていること")
        })
        .collect();
    assert_eq!(read, expected, "同着は id の降順で返ること");

    // **索引を落として計画を変えても同じ。**
    sqlx::query("DROP INDEX attribute_revision_recent_idx")
        .execute(store.pool())
        .await
        .expect("索引を落とせること");
    let without_index: Vec<String> = revisions(&store, id)
        .await
        .iter()
        .map(|revision| {
            revision
                .after()
                .map(|value| value.as_str().to_owned())
                .expect("after が入っていること")
        })
        .collect();
    assert_eq!(
        without_index, expected,
        "索引を辿らない計画でも同じ並びであること"
    );
}

/// **上限で切るのは新しい側から。**
///
/// 期待値は**直書き**（`server::admin` の定数から作らない / CLAUDE.md `16b`）。
#[tokio::test]
async fn only_the_newest_revisions_come_back() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    for value in ["僕", "俺", "私", "わたし"] {
        store
            .replace_character_attributes(
                id,
                &set(vec![attribute(Subject::Character, "一人称", value)]),
            )
            .await
            .expect("置き換えられること");
    }

    let two = store
        .attribute_revisions(id, 2)
        .await
        .expect("履歴を読めること");
    assert_eq!(
        two.iter().map(shape).collect::<Vec<_>>(),
        vec![
            (Subject::Character, "一人称", Some("私"), Some("わたし")),
            (Subject::Character, "一人称", Some("俺"), Some("私")),
        ],
        "新しい2件だけが返ること"
    );
    assert_eq!(
        store
            .attribute_revisions(id, 0)
            .await
            .expect("0 件でも読めること")
            .len(),
        0,
        "上限 0 なら1件も返らないこと"
    );
}

/// **他のキャラクターの履歴は返らない。**
#[tokio::test]
async fn the_history_of_another_character_is_never_returned() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let rin = character(&store, "rin", "凛").await;

    store
        .replace_character_attributes(
            aoi,
            &set(vec![attribute(Subject::Character, "一人称", "僕")]),
        )
        .await
        .expect("葵の属性を保存できること");
    store
        .replace_character_attributes(
            rin,
            &set(vec![attribute(Subject::Character, "一人称", "わたし")]),
        )
        .await
        .expect("凛の属性を保存できること");

    assert_eq!(
        revisions(&store, aoi)
            .await
            .iter()
            .map(shape)
            .collect::<Vec<_>>(),
        vec![(Subject::Character, "一人称", None, Some("僕"))],
        "葵の履歴だけが返ること"
    );
    assert_eq!(
        revisions(&store, rin)
            .await
            .iter()
            .map(shape)
            .collect::<Vec<_>>(),
        vec![(Subject::Character, "一人称", None, Some("わたし"))],
        "凛の履歴だけが返ること"
    );
}

/// **重複で落ちた置き換えは、履歴も残さない**（同じトランザクション）。
///
/// 残ると「起きていない変更」が記録に載る。
#[tokio::test]
async fn a_rejected_replacement_leaves_no_history() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    store
        .replace_character_attributes(
            id,
            &set(vec![attribute(Subject::Character, "一人称", "僕")]),
        )
        .await
        .expect("最初の属性を保存できること");
    let after_first = revisions(&store, id).await.len();
    assert_eq!(after_first, 1, "前提: 1件だけ記録されていること");

    let duplicated = vec![
        attribute(Subject::Character, "種族", "猫獣人"),
        attribute(Subject::Character, "種族", "犬獣人"),
    ];
    let err = store
        .replace_character_attributes(id, &set(duplicated))
        .await
        .expect_err("同じ主語の同じキーは主キー違反になること");
    assert_query_kind(&err, ErrorKind::UniqueViolation, "重複したキー");

    assert_eq!(
        revisions(&store, id).await.len(),
        after_first,
        "落ちた置き換えの履歴は残らないこと"
    );
    assert_eq!(
        store
            .character_attributes(id)
            .await
            .expect("投影は前の状態のままであること"),
        vec![attribute(Subject::Character, "一人称", "僕")],
    );
}

/// **空の集合は、消えた分を全部記録する。**
#[tokio::test]
async fn clearing_every_attribute_records_every_removal() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    store
        .replace_character_attributes(id, &set(fixture()))
        .await
        .expect("属性を保存できること");
    store
        .replace_character_attributes(id, &set(Vec::new()))
        .await
        .expect("空の集合で全消しできること");

    let history = revisions(&store, id).await;
    let removals: Vec<(Subject, &str, Option<&str>, Option<&str>)> =
        history.iter().take(fixture().len()).map(shape).collect();
    assert_eq!(
        removals.len(),
        fixture().len(),
        "消えた件数だけ記録されること"
    );
    assert!(
        removals
            .iter()
            .all(|(_, _, before, after)| before.is_some() && after.is_none()),
        "どれも before だけを持つこと: {removals:?}"
    );
}

/// **読めない行があると、履歴の読みは丸ごと止まる**（想起と同じ側に倒す）。
///
/// 飛ばすと**履歴から静かに1行消える。** 履歴は「静かに変わったこと」を
/// 見つけるための表なので、その表から静かに消えるのは最も悪い壊れ方。
///
/// **止まるのは履歴の読みだけ。** 属性そのもの（投影）は読めるし、置き換えも通る。
#[tokio::test]
async fn a_broken_history_row_stops_only_the_listing() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    store
        .replace_character_attributes(
            id,
            &set(vec![attribute(Subject::Character, "一人称", "僕")]),
        )
        .await
        .expect("属性を保存できること");

    let over_long = "い".repeat(AttributeValue::MAX_CHARS + 1);
    let broken: [(&str, &str, Option<&str>, Option<&str>); 4] = [
        // (subject, key, before, after)
        ("assistant", "一人称", None, Some("僕")),
        (
            "character",
            &"あ".repeat(AttributeKey::MAX_CHARS + 1),
            None,
            Some("僕"),
        ),
        ("character", "一人称", Some(over_long.as_str()), None),
        ("character", "一人称", None, Some(over_long.as_str())),
    ];

    for (subject, key, before, after) in broken {
        let (_dir, store) = setup().await;
        let id = character(&store, "aoi", "葵").await;
        insert_attribute_revision_ignoring_checks(&store, id.get(), subject, key, before, after)
            .await;

        assert!(
            matches!(
                store.attribute_revisions(id, READ_LIMIT).await,
                Err(StoreError::InvalidStoredValue)
            ),
            "履歴の読みが止まること: subject = {subject:?} / key = {key:?}"
        );
        store
            .character_attributes(id)
            .await
            .expect("属性そのものは読めること");
        store
            .replace_character_attributes(
                id,
                &set(vec![attribute(Subject::Character, "種族", "猫獣人")]),
            )
            .await
            .expect("置き換えも通ること");
    }

    // **`before` / `after` の両方が NULL の行も読めない**（作らない形）。
    insert_attribute_revision_ignoring_checks(&store, id.get(), "character", "一人称", None, None)
        .await;
    assert!(
        matches!(
            store.attribute_revisions(id, READ_LIMIT).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "前と後の両方が無い行は読めないこと"
    );
}

/// **壊れた行の修復は届く。ただし、その行の変更は履歴に残せない。**
///
/// 履歴の `key` / `before` / `after` は投影と同じ門を持つので、
/// **上限を超えた値をそのまま `before` に書くと履歴の INSERT が落ちる**
/// ——落ちると置き換えごとロールバックされ、**唯一の修復経路が閉じる。**
/// そこで**記録できない変更は記録せず、警告を残して修復を通す。**
///
/// これは「守れていないこと」であって、仕様である。
#[tokio::test]
async fn repairing_a_broken_row_is_not_recorded_but_still_works() {
    let (_dir, store) = setup().await;
    let id = character(&store, "aoi", "葵").await;

    let over_long = "い".repeat(AttributeValue::MAX_CHARS + 1);
    insert_character_attribute_ignoring_checks(&store, id.get(), "character", "種族", &over_long)
        .await;
    assert!(
        matches!(
            store.character_attributes(id).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "前提: 読めない行があること"
    );

    let fixed = vec![attribute(Subject::Character, "種族", "猫獣人")];
    store
        .replace_character_attributes(id, &set(fixed.clone()))
        .await
        .expect("置き換えが届くこと");

    assert_eq!(
        store.character_attributes(id).await.expect("直ったこと"),
        fixed,
        "投影が入れ替わること"
    );
    assert_eq!(
        revisions(&store, id)
            .await
            .iter()
            .map(shape)
            .collect::<Vec<_>>(),
        Vec::new(),
        "記録できない変更は履歴に残らないこと（残そうとすると修復ごと落ちる）"
    );
}

/// **キャラクターを消すと履歴も消える**（`ON DELETE CASCADE`）。
///
/// 孤児の履歴を残さない。**「追記のみ」が守るのはアプリからの書き換えまで**で、
/// 親の削除には従う（票の表と同じ立場）。
#[tokio::test]
async fn deleting_a_character_takes_its_history_with_it() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let rin = character(&store, "rin", "凛").await;

    for (id, value) in [(aoi, "僕"), (rin, "わたし")] {
        store
            .replace_character_attributes(
                id,
                &set(vec![attribute(Subject::Character, "一人称", value)]),
            )
            .await
            .expect("属性を保存できること");
    }

    store
        .delete_character(&Handle::parse("aoi").expect("正当な識別子"))
        .await
        .expect("会話の無いキャラクターを削除できること");

    assert_eq!(
        revisions(&store, aoi).await.len(),
        0,
        "消えたキャラクターの履歴は残らないこと"
    );
    assert_eq!(
        revisions(&store, rin)
            .await
            .iter()
            .map(shape)
            .collect::<Vec<_>>(),
        vec![(Subject::Character, "一人称", None, Some("わたし"))],
        "巻き添えにしないこと"
    );
}

/// **同時に走る2つの置き換えが、互いを落とさない**（`BEGIN IMMEDIATE` / ADR-0028 §決定5）。
///
/// # 素の `BEGIN` では落ちる
///
/// 置き換えは **現在の集合を読んでから書く**形になったので、
/// 素の `BEGIN`（DEFERRED）だと最初の読みで読み取りスナップショットが固定され、
/// 続く書きへの昇格が `SQLITE_BUSY_SNAPSHOT` になる
/// ——**`busy_timeout` の対象外で、待たずに即座に失敗する**
/// （`store::character` の `BEGIN_IMMEDIATE` の doc に 0011a-2 の実測がある）。
///
/// # 負荷のかけ方まで決める（`0006` の教訓）
///
/// **別のキャラクターを相手にした置き換えを同時に走らせる。**
/// WAL のスナップショットは DB 全体を指すので、**競合していない置き換えでも落ちる。**
/// つまり「どちらが勝つか」ではなく、**両方成功する**のが正しい結果である。
#[tokio::test]
async fn two_replacements_never_lose_their_snapshot() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let rin = character(&store, "rin", "凛").await;

    // **読む対象がある状態から始める**（現在の集合が空だと読みが軽くなる）。
    for id in [aoi, rin] {
        store
            .replace_character_attributes(id, &set(fixture()))
            .await
            .expect("初期の属性を設定できること");
    }

    for round in 0..32 {
        let first = {
            let store = store.clone();
            let next = vec![attribute(Subject::Character, "一人称", "僕")];
            tokio::spawn(async move { store.replace_character_attributes(aoi, &set(next)).await })
        };
        let second = {
            let store = store.clone();
            let next = vec![attribute(Subject::Character, "一人称", "わたし")];
            tokio::spawn(async move { store.replace_character_attributes(rin, &set(next)).await })
        };

        let first = first.await.expect("タスクが panic しないこと");
        let second = second.await.expect("タスクが panic しないこと");
        assert!(
            first.is_ok() && second.is_ok(),
            "どちらも成功すること（{round} 周目）: 葵 = {first:?} / 凛 = {second:?}"
        );

        // 次の周で「読んでから書く」を再現するため、元の集合へ戻す。
        for id in [aoi, rin] {
            store
                .replace_character_attributes(id, &set(fixture()))
                .await
                .unwrap_or_else(|err| panic!("{round} 周目の後始末ができること: {err}"));
        }
    }
}

/// **`StatScale` を取り違えていない**ことの手掛かり（フィクスチャの前提の明示）。
///
/// このファイルのキャラクターはすべて `fixture_scale()`（既定と違う値）で作る。
/// 属性はスケールと無関係なので、**混ざったら気付けるように**前提だけ書いておく。
#[test]
fn the_fixture_scale_is_not_the_default() {
    assert_ne!(fixture_scale(), StatScale::DEFAULT);
}
