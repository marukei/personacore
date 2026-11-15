//! **返答の長さの目安**（下限と上限の字数）の永続化の検証
//! （ADR-0065 §決定1 / 作業単位 A-22a = marukei/personacore#71）。
//!
//! 見ていること:
//!   * 作成・1件読み・一覧・部分更新（省く／置き換える／未設定に戻す）で往復する。
//!   * **下限と上限を取り違えない**（下限 ≠ 上限の値で見る）。
//!   * 既存の入口（`create_character` / `create_character_with_drift_mode`）は未設定で作る。
//!   * 1組が読めない行（片方だけ・範囲外・逆転）は、1件読みを止め、一覧では2列とも指し示す。
//!   * スキーマの CHECK が1組の条件を守る。

use personacore::domain::{
    Character, CharacterColumn, CharacterEntry, CharacterUpdate, DisplayName, FieldUpdate, Handle,
    Persona, ResponseLength, StatDriftMode, UnreadableCharacter,
};
use personacore::store::{CharacterSettings, Store, StoreError};
use sqlx::error::ErrorKind;
use tempfile::TempDir;

mod common;

use common::{fixture_scale, set_response_length_ignoring_checks};

async fn setup() -> (TempDir, Store) {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let store = Store::connect(dir.path().join("personacore.sqlite"))
        .await
        .expect("接続とマイグレーション適用が成功すること");
    (dir, store)
}

fn name(raw: &str) -> Handle {
    Handle::parse(raw).expect("テストで使う名前は正当であること")
}

fn display(raw: &str) -> DisplayName {
    DisplayName::parse(raw).expect("テストで使う表示名は正当であること")
}

fn persona(raw: &str) -> Persona {
    Persona::parse(raw).expect("テストで使うペルソナは正当であること")
}

fn length(min: u32, max: u32) -> ResponseLength {
    ResponseLength::parse(min, max).expect("正当な目安であること")
}

async fn found(store: &Store, handle: &str) -> Character {
    store
        .find_character_by_handle(&name(handle))
        .await
        .expect("検索が成功すること")
        .unwrap_or_else(|| panic!("{handle} を引けること"))
}

fn readable_named<'a>(entries: &'a [CharacterEntry], handle: &str) -> &'a Character {
    entries
        .iter()
        .find_map(|entry| match entry {
            CharacterEntry::Readable(character) if character.handle.as_str() == handle => {
                Some(character)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("{handle} が読める行にあること: {entries:?}"))
}

fn unreadable(entries: &[CharacterEntry]) -> Vec<&UnreadableCharacter> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            CharacterEntry::Unreadable(broken) => Some(broken),
            CharacterEntry::Readable(_) => None,
        })
        .collect()
}

/// 生の2列（`typeof` つき）。読み出しが既定を返すだけの実装と区別するため、DB の値そのものを見る。
async fn stored_columns(store: &Store, handle: &str) -> (Option<i64>, Option<i64>) {
    sqlx::query_as(
        "SELECT response_length_min, response_length_max FROM character WHERE handle = ?",
    )
    .bind(handle)
    .fetch_one(store.pool())
    .await
    .expect("2列を読めること")
}

/// **作成・1件読み・一覧・部分更新の全部で往復する**（ADR-0065 §決定1）。
///
/// * **囮を2つ並べる**（未設定のものと、別の値のもの）—— 一覧の先頭から取る実装と区別する。
/// * **下限 ≠ 上限**（200 / 400）—— 2列を取り違える実装は値が入れ替わって赤になる。
/// * **省いた更新は変えない**（`Keep` を `Clear` に潰すと、表示名を直すたびに未設定へ戻る）。
/// * **`Set` は置き換え、`Clear` は未設定へ戻す。他の列は動かない。**
#[tokio::test]
async fn the_response_length_survives_the_round_trip_and_partial_updates() {
    let (_dir, store) = setup().await;

    // 囮1: 従来の入口（＝未設定）。
    store
        .create_character(
            &name("akane"),
            &display("茜音"),
            &persona("昼番の受付。"),
            None,
            fixture_scale(),
        )
        .await
        .expect("囮を作成できること");
    // 囮2: 別の値。
    store
        .create_character_with_settings(
            &name("kaede"),
            &display("楓"),
            &persona("夜番の司書。"),
            None,
            fixture_scale(),
            CharacterSettings {
                stat_drift_mode: None,
                response_length: Some(length(800, 1200)),
            },
        )
        .await
        .expect("囮2を作成できること");
    store
        .create_character_with_settings(
            &name("aoi"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            None,
            fixture_scale(),
            CharacterSettings {
                stat_drift_mode: Some(StatDriftMode::On),
                response_length: Some(length(200, 400)),
            },
        )
        .await
        .expect("対象を作成できること");

    assert_eq!(stored_columns(&store, "aoi").await, (Some(200), Some(400)));
    assert_eq!(stored_columns(&store, "akane").await, (None, None));
    let aoi = found(&store, "aoi").await;
    assert_eq!(aoi.response_length, Some(length(200, 400)));
    assert_eq!(
        aoi.stat_drift_mode,
        Some(StatDriftMode::On),
        "同時に渡した走らせ方も書かれること"
    );
    assert_eq!(found(&store, "akane").await.response_length, None);

    let listed = store.list_characters().await.expect("一覧を取得できること");
    assert_eq!(
        [
            readable_named(&listed, "akane").response_length,
            readable_named(&listed, "kaede").response_length,
            readable_named(&listed, "aoi").response_length,
        ],
        [None, Some(length(800, 1200)), Some(length(200, 400))],
        "一覧からも行ごとの値が読めること"
    );

    // --- 省いた更新は変えない ---
    store
        .update_character(
            &name("aoi"),
            &CharacterUpdate {
                display_name: Some(display("葵 碧")),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("表示名だけを更新できること");
    let after_display = found(&store, "aoi").await;
    assert_eq!(after_display.response_length, Some(length(200, 400)));

    // --- `Set` は置き換え、他の列は動かさない ---
    store
        .update_character(
            &name("aoi"),
            &CharacterUpdate {
                response_length: FieldUpdate::Set(length(400, 800)),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("目安だけを更新できること");
    let after_set = found(&store, "aoi").await;
    assert_eq!(after_set.response_length, Some(length(400, 800)));
    assert_eq!(
        Character {
            response_length: after_display.response_length,
            ..after_set.clone()
        },
        after_display,
        "目安以外の列が動かないこと"
    );

    // --- `Clear` は未設定へ戻す（2列とも NULL） ---
    store
        .update_character(
            &name("aoi"),
            &CharacterUpdate {
                response_length: FieldUpdate::Clear,
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("目安を未設定へ戻せること");
    assert_eq!(found(&store, "aoi").await.response_length, None);
    assert_eq!(stored_columns(&store, "aoi").await, (None, None));

    // --- 囮は最後まで無傷 ---
    assert_eq!(found(&store, "akane").await.response_length, None);
    assert_eq!(
        found(&store, "kaede").await.response_length,
        Some(length(800, 1200))
    );
}

/// **既存の作成の入口は未設定で作る**（呼び出し側の振る舞いが変わらない）。
#[tokio::test]
async fn the_existing_entry_points_create_without_a_target() {
    let (_dir, store) = setup().await;
    store
        .create_character_with_drift_mode(
            &name("aoi"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            None,
            fixture_scale(),
            Some(StatDriftMode::Shadow),
        )
        .await
        .expect("作成できること");
    assert_eq!(stored_columns(&store, "aoi").await, (None, None));
    let aoi = found(&store, "aoi").await;
    assert_eq!(aoi.response_length, None);
    assert_eq!(aoi.stat_drift_mode, Some(StatDriftMode::Shadow));
}

/// **1組が読めない行は、1件読みを止め、一覧では2列とも指し示す**（`CharacterColumn::ResponseLengthMin` の doc）。
///
/// 植える壊れ方: 片方だけ（下限だけ・上限だけ）・下限 0・上限 10000・逆転・負の値。
/// **壊れた値を「未設定」に落とさない**（落とすと、目安を入れたはずのキャラクターから行が黙って消える）。
#[tokio::test]
async fn a_row_whose_pair_cannot_be_read_is_pointed_at_by_both_columns() {
    let cases: [(Option<i64>, Option<i64>); 7] = [
        (Some(200), None),
        (None, Some(400)),
        (Some(0), Some(400)),
        (Some(200), Some(10_000)),
        (Some(401), Some(400)),
        (Some(-1), Some(400)),
        (Some(i64::from(u32::MAX) + 1), Some(i64::from(u32::MAX) + 2)),
    ];
    for (min, max) in cases {
        let (_dir, store) = setup().await;
        let healthy = store
            .create_character_with_settings(
                &name("akane"),
                &display("茜音"),
                &persona("昼番の受付。"),
                None,
                fixture_scale(),
                CharacterSettings {
                    stat_drift_mode: None,
                    response_length: Some(length(200, 400)),
                },
            )
            .await
            .expect("正常な行を作成できること");
        let broken = store
            .create_character(
                &name("aoi"),
                &display("葵"),
                &persona("落ち着いた物腰の司書。"),
                None,
                fixture_scale(),
            )
            .await
            .expect("対象を作成できること");
        set_response_length_ignoring_checks(&store, broken.get(), min, max).await;

        assert!(
            matches!(
                store.find_character_by_handle(&name("aoi")).await,
                Err(StoreError::InvalidStoredValue)
            ),
            "1件読みは止まること: {min:?} / {max:?}"
        );

        let entries = store.list_characters().await.expect("一覧は落ちないこと");
        assert_eq!(
            unreadable(&entries),
            vec![&UnreadableCharacter {
                id: broken,
                handle: Some(name("aoi")),
                invalid_columns: vec![
                    CharacterColumn::ResponseLengthMin,
                    CharacterColumn::ResponseLengthMax,
                ],
            }],
            "2列とも指し示すこと: {min:?} / {max:?}"
        );
        let healthy_row = readable_named(&entries, "akane");
        assert_eq!(healthy_row.id, healthy);
        assert_eq!(healthy_row.response_length, Some(length(200, 400)));

        // **直す入口は `PATCH` と同じ更新**（読まずに書く / ADR-0016 §4）。
        store
            .update_character(
                &name("aoi"),
                &CharacterUpdate {
                    response_length: FieldUpdate::Clear,
                    ..CharacterUpdate::default()
                },
            )
            .await
            .expect("壊れた行も更新で直せること");
        assert_eq!(found(&store, "aoi").await.response_length, None);
    }
}

/// スキーマの CHECK の種類を確かめる。
fn assert_check_violation(err: &sqlx::Error, context: &str) {
    let kind = err
        .as_database_error()
        .map(sqlx::error::DatabaseError::kind);
    assert_eq!(kind, Some(ErrorKind::CheckViolation), "{context}: {err}");
}

/// **CHECK が1組の条件を守る**（`0019`）。片方だけ・範囲外・逆転・整数でない値を止め、
/// 端の値と、整数に直って入る形（`'200'` / `3.0`）は通す。
#[tokio::test]
async fn the_schema_check_keeps_the_pair_whole() {
    let (_dir, store) = setup().await;
    let id = store
        .create_character(
            &name("aoi"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            None,
            fixture_scale(),
        )
        .await
        .expect("対象を作成できること")
        .get();

    for sql in [
        "UPDATE character SET response_length_min = 0, response_length_max = 5 WHERE id = ?",
        "UPDATE character SET response_length_min = 5, response_length_max = 10000 WHERE id = ?",
        "UPDATE character SET response_length_min = 401, response_length_max = 400 WHERE id = ?",
        "UPDATE character SET response_length_min = NULL, response_length_max = 5 WHERE id = ?",
        "UPDATE character SET response_length_min = 5, response_length_max = NULL WHERE id = ?",
        "UPDATE character SET response_length_min = 2.5, response_length_max = 4 WHERE id = ?",
        "UPDATE character SET response_length_min = 'abc', response_length_max = 5 WHERE id = ?",
        "UPDATE character SET response_length_min = X'01', response_length_max = 5 WHERE id = ?",
        "UPDATE character SET response_length_min = 2, response_length_max = 4.5 WHERE id = ?",
        // 両方 NULL の行に、片方だけを書く。
        "UPDATE character SET response_length_min = 5 WHERE id = ?",
        "UPDATE character SET response_length_max = 5 WHERE id = ?",
    ] {
        let err = sqlx::query(sql)
            .bind(id)
            .execute(store.pool())
            .await
            .expect_err("1組の条件を破る値は弾かれること");
        assert_check_violation(&err, sql);
    }

    for (sql, expected) in [
        (
            "UPDATE character SET response_length_min = 1, response_length_max = 1 WHERE id = ?",
            (1_i64, 1_i64),
        ),
        (
            "UPDATE character SET response_length_min = 9999, response_length_max = 9999 WHERE id = ?",
            (9999, 9999),
        ),
        (
            "UPDATE character SET response_length_min = '200', response_length_max = '400' WHERE id = ?",
            (200, 400),
        ),
        (
            "UPDATE character SET response_length_min = 3.0, response_length_max = 4.0 WHERE id = ?",
            (3, 4),
        ),
    ] {
        sqlx::query(sql)
            .bind(id)
            .execute(store.pool())
            .await
            .unwrap_or_else(|err| panic!("{sql} は入ること: {err}"));
        let (min, max, min_kind, max_kind): (i64, i64, String, String) = sqlx::query_as(
            "SELECT response_length_min, response_length_max, \
             typeof(response_length_min), typeof(response_length_max) FROM character WHERE id = ?",
        )
        .bind(id)
        .fetch_one(store.pool())
        .await
        .expect("読めること");
        assert_eq!(
            ((min, max), min_kind.as_str(), max_kind.as_str()),
            (expected, "integer", "integer"),
            "{sql}"
        );
    }

    // 両方 NULL へは戻せる。
    sqlx::query(
        "UPDATE character SET response_length_min = NULL, response_length_max = NULL WHERE id = ?",
    )
    .bind(id)
    .execute(store.pool())
    .await
    .expect("両方 NULL へは戻せること");
}
