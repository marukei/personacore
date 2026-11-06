//! `character` / `stat_alloc` の永続化の検証。
//!
//! ここで確かめるのは主に次の4点:
//!   * **正規化された名前だけが DB に届く**こと（`" aoi "` と `"aoi"` が併存しない / ADR-0004）。
//!   * **照合は正規化後の値で完全一致**であること。検索側で trim しない。
//!   * `stat_alloc` の置き換えが**同一トランザクションで丸ごと差し替え**になること。
//!   * **更新は既存の行を `parse` せずに書ける**こと（ADR-0016 / 修復の入口）。
//!     **削除は会話が1件でもあれば拒否する**こと。
//!
//! スキーマ制約そのものの検証は `tests/store_schema.rs` にある。

use chrono::Utc;
use personacore::domain::{
    AxisDiscoverySlots, Character, CharacterColumn, CharacterEntry, CharacterId, CharacterUpdate,
    DisplayName, FieldUpdate, Handle, OutputFormat, Persona, StatAlloc, StatAxis, StatDriftMode,
    StatScale, UnreadableCharacter,
};
use personacore::store::{Store, StoreError};
use sqlx::error::ErrorKind;
use tempfile::TempDir;

mod common;

use common::{
    CLOCK_SKEW, delete_character_ignoring_foreign_keys, fixture_scale,
    insert_character_ignoring_checks, insert_ghost_character, insert_ghost_character_with,
    insert_stat_alloc_directly, insert_stat_alloc_ignoring_checks,
    set_axis_discovery_slots_ignoring_checks, set_stat_drift_mode_ignoring_checks,
    set_stat_scale_ignoring_checks,
};

/// 一時ディレクトリ上に新規 DB を作り、マイグレーション適用済みの `Store` を返す。
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

/// テストで使う表示名。**識別子と同じ文字列から作る**のが既定
/// （`display_name` を省いて作成したときの状態 / ADR-0011）。
fn display(raw: &str) -> DisplayName {
    DisplayName::parse(raw).expect("テストで使う表示名は正当であること")
}

fn persona(raw: &str) -> Persona {
    Persona::parse(raw).expect("テストで使うペルソナは正当であること")
}

fn output_format(raw: &str) -> OutputFormat {
    OutputFormat::parse(raw).expect("テストで使う形式指示は正当であること")
}

/// 1軸分の配分。**値は `fixture_scale()`（= 7）を通す。**
///
/// このファイルのキャラクターはほぼ全て `fixture_scale()` で作られるので、
/// **既定の組み合わせが「尺度の一致する値」になる。**
/// 取り違えを見るテストは `scaled_alloc` で**別の尺度**を明示すること。
fn alloc(axis: &str, value: u32) -> StatAlloc {
    scaled_alloc(fixture_scale(), axis, value)
}

/// 尺度を明示して1軸分の配分を作る（取り違えの検証用）。
fn scaled_alloc(scale: StatScale, axis: &str, value: u32) -> StatAlloc {
    StatAlloc {
        axis: StatAxis::parse(axis).expect("テストで使う軸名は正当であること"),
        value: scale
            .value(value)
            .expect("テストで使う値は尺度に収まること"),
    }
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

/// 作成 → 名前で取得 → 一覧、の往復。
#[tokio::test]
async fn create_then_find_then_list() {
    let (_dir, store) = setup().await;
    // `created_at` は SQLite が書くので、`Utc::now()` とは別の時計になる（`CLOCK_SKEW`）。
    let before = Utc::now() - CLOCK_SKEW;

    let id = store
        .create_character(
            &name("aoi"),
            // **識別子とは別の値**にする。同じ値だと、読み出しが `handle` を返していても
            // 往復が成立してしまい、列を分けた意味を検証できない（ADR-0011）。
            &display("葵"),
            &persona("ペルソナ本文"),
            Some(&output_format("台詞は「」で囲む")),
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    let found = store
        .find_character_by_handle(&name("aoi"))
        .await
        .expect("検索が成功すること")
        .expect("登録したキャラクターを引けること");

    assert_eq!(found.id, id);
    assert_eq!(found.handle, name("aoi"));
    assert_eq!(found.display_name, display("葵"), "表示名も往復すること");
    assert_eq!(found.base_persona, persona("ペルソナ本文"));
    assert_eq!(
        found.output_format_instruction,
        Some(output_format("台詞は「」で囲む"))
    );
    // **尺度も往復する**（作業単位 0011a-1）。`fixture_scale()` は既定と違う値なので、
    // INSERT が引数を捨てて `StatScale::DEFAULT` を書く実装はここで赤くなる。
    assert_eq!(found.stat_scale, fixture_scale(), "尺度も往復すること");
    assert!(
        found.created_at >= before && found.created_at <= Utc::now() + CLOCK_SKEW,
        "created_at に妥当な日時が入ること: {}",
        found.created_at
    );

    let listed = store.list_characters().await.expect("一覧を取得できること");
    assert_eq!(
        listed,
        vec![CharacterEntry::Readable(found)],
        "一覧にも同じ内容が現れること"
    );
}

/// `output_format_instruction` は未設定にできる（既定値はアプリ側で決める）。
#[tokio::test]
async fn output_format_instruction_may_be_absent() {
    let (_dir, store) = setup().await;
    store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("形式指示なしでも作成できること");

    let found = store
        .find_character_by_handle(&name("aoi"))
        .await
        .expect("検索が成功すること")
        .expect("登録したキャラクターを引けること");
    assert_eq!(found.output_format_instruction, None);
}

/// **識別子は書き換えられずに DB へ届く**（ADR-0017 / 作業単位 0004b-1）。
///
/// 0004b-1 より前はここが「`" aoi "` で作成したものが `"aoi"` で引ける」だった
/// （`Handle::parse` が trim していた）。**いまは正規化そのものが無い**ので、
/// 見張るのは「`parse` を通った値がそのまま格納され、そのまま引ける」ことになる。
///
/// 途中で1文字でも書き換えると、`model` からの完全一致が外れて
/// 「作れたのにチャットは 404」になる。
#[tokio::test]
async fn the_handle_reaches_the_database_unchanged() {
    let (_dir, store) = setup().await;

    // **記号入りを使う。** `"aoi"` だけだと、何を書き換えても気付けない綴りが残る。
    let handle = "aoi-v2_7";
    store
        .create_character(
            &name(handle),
            // **識別子とは別の値**にする（同値だと取り違えを検出できない）。
            &display("葵 碧"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    let stored: String = sqlx::query_scalar("SELECT handle FROM character")
        .fetch_one(store.pool())
        .await
        .expect("保存された名前を取得できること");
    assert_eq!(stored, handle, "識別子が書き換えられずに格納されること");

    assert!(
        store
            .find_character_by_handle(&name(handle))
            .await
            .expect("検索が成功すること")
            .is_some(),
        "同じ綴りで引けること"
    );

    // **正規化すれば正当になる綴りは、そもそも型を作れない**（`Handle::parse`）。
    // ここが通るようになったら `store` の手前で trim が復活している。
    assert!(
        Handle::parse(" aoi ").is_err(),
        "前後に空白のある識別子は `store` に届く型を作れないこと"
    );
}

/// 同名の重複作成は UNIQUE 違反。
#[tokio::test]
async fn duplicate_handle_is_rejected_as_unique_violation() {
    let (_dir, store) = setup().await;
    store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("1件目は作成できること");

    let err = store
        .create_character(
            &name("aoi"),
            &display("葵"),
            &persona("別のペルソナ"),
            None,
            fixture_scale(),
        )
        .await
        .expect_err("同名の作成は弾かれること");

    assert_query_kind(&err, ErrorKind::UniqueViolation, "character.handle");
    assert!(
        !err.to_string().contains("UNIQUE"),
        "Display は定数文字列で、内部情報を含めないこと: {err}"
    );
}

/// **ゴースト行は無力である**〈0004b-2 が機構を捨ててよい根拠そのもの / ADR-0017〉。
///
/// # 何を見ているのか
///
/// 正規化キーの機構（`character_name_key_idx` / `SELECT_BY_NORMALIZED_KEY` /
/// `StoreError::ConflictingUnnormalizedName`）が守っていたのは、
/// **`"aoi"` と `" aoi "` が併存して照合が両方に一致し、書き込み先が不定になる**状態だった
/// （ADR-0004）。**その状態は `Handle::parse` が trim をやめた時点で作れなくなった。**
///
/// ここで確かめるのは「機構を捨てても、ゴースト行が無害なまま」であること:
///
/// * 併存できる（索引を落としたので INSERT が通る）。
/// * それでも **`find_character_by_handle` は本物の行だけに当たる。**
/// * **一覧はゴースト行を「読めない行」として id で指し示す**（黙って消さない）。
/// * **ゴースト行があっても正当な識別子を作れる**（以前はここが塞がっていた）。
///
/// # 全 `TRIMMABLE` 文字を回す
///
/// 固定リストにすると、特定の空白だけ挙動が違う実装を見逃す。
/// **`insert_ghost_character` はゴースト行を作れることまで assert している**ので、
/// CHECK を外す抜け穴が塞がっても（＝この状況を作れなくなっても）ここが赤くなる。
#[tokio::test]
async fn a_ghost_row_is_inert_for_every_trimmable_padding() {
    for blank in personacore::domain::TRIMMABLE {
        let padded = format!("{blank}aoi{blank}");
        let (_dir, store) = setup().await;

        // store を経由しない直接の INSERT（DB の直接編集を模す）。
        // **表示名は本物と別の値**にする（取り違えを検出するため）。
        let ghost_id =
            insert_ghost_character_with(&store, &padded, "楓", "ゴーストのペルソナ").await;

        // 完全一致では引けない（= チャットからも `model` からも当たらない）。
        assert!(
            store
                .find_character_by_handle(&name("aoi"))
                .await
                .expect("検索が成功すること")
                .is_none(),
            "U+{:04X}: ゴースト行が完全一致で引けている",
            blank as u32
        );

        // **正当な識別子を作れる。** 0004b-2 より前はここが
        // `ConflictingUnnormalizedName` で塞がれ、袋小路になっていた。
        let real_id = store
            .create_character(
                &name("aoi"),
                &display("葵"),
                &persona("本物のペルソナ"),
                None,
                fixture_scale(),
            )
            .await
            .unwrap_or_else(|err| {
                panic!(
                    "U+{:04X}: ゴースト行が正当な識別子の作成を塞いでいる \
                     (格納名 = {padded:?} / 実際のエラー = {err:?})",
                    blank as u32
                )
            });
        assert_ne!(
            real_id.get(),
            ghost_id,
            "U+{:04X}: 作られたのはゴースト行とは別の行であること",
            blank as u32
        );

        // 照合は**本物の行だけ**に当たる（書き込み先が不定にならない）。
        let found = store
            .find_character_by_handle(&name("aoi"))
            .await
            .expect("検索が成功すること")
            .expect("本物の行は引けること");
        assert_eq!(
            found.id, real_id,
            "U+{:04X}: 照合がゴースト行に当たっている",
            blank as u32
        );
        assert_eq!(found.display_name, display("葵"));

        // 一覧は**両方**を返す。ゴースト行は id で指し示すだけ（黙って消さない）。
        let entries = store.list_characters().await.expect("一覧を取得できること");
        assert_eq!(
            readable(&entries)
                .iter()
                .map(|character| character.handle.as_str())
                .collect::<Vec<_>>(),
            vec!["aoi"],
            "U+{:04X}: 読める側に来るのは本物の行だけであること: {entries:?}",
            blank as u32
        );
        let broken = unreadable(&entries);
        assert_eq!(
            broken.len(),
            1,
            "U+{:04X}: ゴースト行が指し示されていない: {entries:?}",
            blank as u32
        );
        assert_eq!(broken[0].id, CharacterId::from_raw_for_tests(ghost_id));
        assert_eq!(broken[0].handle, None, "識別子は `parse` を通らないこと");
        assert_eq!(broken[0].invalid_columns, vec![CharacterColumn::Handle]);
    }
}

/// **正当な重複は今までどおり UNIQUE 違反になる**（ゴースト行の併存と混ぜない）。
#[tokio::test]
async fn a_duplicate_handle_is_still_a_unique_violation_even_next_to_a_ghost_row() {
    let (_dir, store) = setup().await;

    insert_ghost_character(&store, " rin ").await;
    store
        .create_character(
            &name("rin"),
            &display("rin"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("ゴースト行の隣でも1件目は作成できること");

    let err = store
        .create_character(
            &name("rin"),
            &display("rin"),
            &persona("別のペルソナ"),
            None,
            fixture_scale(),
        )
        .await
        .expect_err("重複する名前は弾かれること");
    assert_query_kind(&err, ErrorKind::UniqueViolation, "正当な重複作成");
    assert!(
        !err.to_string().contains("rin") && !err.to_string().contains("UNIQUE"),
        "Display は定数文字列で、格納名も内部情報も含めないこと: {err}"
    );
}

/// `is_unique_violation` は**重複だけ**に真を返す。
///
/// server 層はこの判定で 409「既に存在します」に振り分ける。
///
/// > **0004b-2 で「ゴースト行との衝突」の節が消えた。** 正規化キーの索引を落としたので、
/// > その衝突自体が起きない（`a_ghost_row_is_inert_for_every_trimmable_padding`）。
/// > 代わりに**ゴースト行があっても作成が成功する**ことをここでも押さえる。
#[tokio::test]
async fn unique_violation_is_reported_only_for_a_legitimate_duplicate() {
    let (_dir, store) = setup().await;
    store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("1件目は作成できること");

    let duplicate = store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("別のペルソナ"),
            None,
            fixture_scale(),
        )
        .await
        .expect_err("重複する名前は弾かれること");
    assert!(
        duplicate.is_unique_violation(),
        "正当な重複は UNIQUE 違反として判定されること: {duplicate:?}"
    );

    // 同一軸の重複（主キー違反）も UNIQUE 違反として扱う。
    let id = store
        .create_character(
            &name("rin"),
            &display("rin"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");
    let duplicate_axis = store
        .replace_stat_allocs(id, &[alloc("共感", 1), alloc("共感", 2)])
        .await
        .expect_err("重複した軸は弾かれること");
    assert!(
        duplicate_axis.is_unique_violation(),
        "主キー違反も UNIQUE 違反として判定されること: {duplicate_axis:?}"
    );

    // **ゴースト行は重複ではない**（作成そのものが成功する）。
    insert_ghost_character(&store, " kaede ").await;
    store
        .create_character(
            &name("kaede"),
            &display("kaede"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("ゴースト行は正当な識別子の作成を塞がないこと");

    // 制約違反以外は偽（プールを閉じて確実に別の失敗を作る）。
    store.close().await;
    let closed = store
        .list_characters()
        .await
        .expect_err("閉じたプールでは失敗すること");
    assert!(
        !closed.is_unique_violation(),
        "接続の失敗を UNIQUE 違反にしないこと: {closed:?}"
    );
}

/// **`parse` を通る識別子と、修復 API が届く行は同じ集合である**
/// 〈`normalized_name` を畳んだことの見張り番 / 0004b-2〉。
///
/// # 何が変わったのか
///
/// 0012b の `character_entry` は識別子を2段で判定していた——
/// `Handle::parse` を通したうえで、格納値が**正規化済みである**ことまで確かめ、
/// 「読める識別子」（完全一致で引ける）と「指せる識別子」（正規化キーで引ける）を
/// 別物として扱っていた。
///
/// **0004b-1 で `parse` が正規化をやめたので、後段は常に真になり死んだ検査になった。**
/// そして 0004b-2 で正規化キーの引き当てを捨てたので、**2つの概念は完全に一致した。**
///
/// # ここが赤くなる変異
///
/// * 一覧が `parse` の前に trim を挟む（「一覧には出るが `model` では引けない名前」が生まれる）
/// * 修復 API が `parse` を通る名前に届かない
///
/// **全 `TRIMMABLE` 文字と、通る綴りの両方を回す。**
#[tokio::test]
async fn a_row_that_parses_is_exactly_a_row_the_repair_api_can_reach() {
    let (_dir, store) = setup().await;

    // (格納名, `parse` を通るか)
    let mut cases: Vec<(String, bool)> = vec![
        ("aoi".to_owned(), true),
        ("aoi-v2_7".to_owned(), true),
        ("0".to_owned(), true),
        ("Aoi".to_owned(), false),
        ("葵".to_owned(), false),
        ("aoi\u{200B}".to_owned(), false),
        ("名".repeat(Handle::MAX_CHARS + 1), false),
    ];
    for blank in personacore::domain::TRIMMABLE {
        cases.push((format!("{blank}aoi{blank}"), false));
    }

    // (id, 格納名, `parse` を通るか)
    let mut rows: Vec<(i64, String, bool)> = Vec::new();
    for (index, (stored, parses)) in cases.iter().enumerate() {
        // 表示名を1件ずつ変えて、行の取り違えを検出できるようにする。
        let id =
            insert_ghost_character_with(&store, stored, &format!("表示名{index}"), "ペルソナ本文")
                .await;
        rows.push((id, stored.clone(), *parses));
    }

    let entries = store.list_characters().await.expect("一覧を取得できること");
    for (index, (id, stored, parses)) in rows.iter().enumerate() {
        // その id の行が一覧のどちら側に来ているかを見て、識別子を取り出す。
        let handle: Option<Handle> = entries
            .iter()
            .find_map(|entry| match entry {
                CharacterEntry::Readable(character) if character.id.get() == *id => {
                    Some(Some(character.handle.clone()))
                }
                CharacterEntry::Unreadable(broken) if broken.id.get() == *id => {
                    Some(broken.handle.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("id = {id} の行が一覧に現れること: {stored:?}"));

        assert_eq!(
            handle.is_some(),
            *parses,
            "一覧が返す識別子の有無は `Handle::parse` と一致すること: {stored:?}"
        );

        let Some(handle) = handle else {
            continue;
        };
        // **一覧が識別子を返したなら、修復 API は必ずその行に届く。**
        assert_eq!(
            handle.as_str(),
            stored,
            "一覧が格納値と違う綴りを返している（正規化が復活している）: {stored:?}"
        );
        let repaired = store
            .update_character(&handle, &update_display(&format!("直した{index}")))
            .await
            .unwrap_or_else(|err| panic!("一覧が返した識別子で直せること: {stored:?}: {err:?}"));
        assert_eq!(
            repaired.get(),
            *id,
            "届いたのはその行であること: {stored:?}"
        );
        assert_eq!(
            display_name_of(&store, stored).await,
            format!("直した{index}"),
            "書き換わったのはその行であること: {stored:?}"
        );
    }
}

/// 存在しない名前は `None`。
#[tokio::test]
async fn unknown_handle_is_not_found() {
    let (_dir, store) = setup().await;
    store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    assert!(
        store
            .find_character_by_handle(&name("rin"))
            .await
            .expect("検索が成功すること")
            .is_none(),
        "未登録の名前は None であること"
    );
}

/// 一覧は名前順で安定する。空の DB では空の一覧。
#[tokio::test]
async fn list_is_empty_then_sorted_by_handle() {
    let (_dir, store) = setup().await;
    assert!(
        store
            .list_characters()
            .await
            .expect("一覧を取得できること")
            .is_empty(),
        "登録前は空であること"
    );

    for raw in ["rin", "aoi", "kaede"] {
        store
            .create_character(
                &name(raw),
                &display(raw),
                &persona("ペルソナ本文"),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");
    }

    let entries = store.list_characters().await.expect("一覧を取得できること");
    let names: Vec<&str> = readable(&entries)
        .iter()
        .map(|character| character.handle.as_str())
        .collect();
    assert_eq!(names, vec!["aoi", "kaede", "rin"], "名前順で並ぶこと");
}

/// **正規化されていない名前の行は「読めた行」に混ぜない。**
///
/// 照合は完全一致なので、`" aoi "` のような行は `model` からは決して引けない。
/// 読めた行として返すと「選べるのに 404 になるモデル」になる。
///
/// **ただし黙って捨てもしない**（ADR-0016 §3 / 作業単位 0012b）。読めなかった行として
/// 指し示す。捨てると、掃除すべき相手が分からないまま正規化キーだけが塞がれ続ける。
#[tokio::test]
async fn list_points_at_rows_whose_handle_is_not_normalized_without_reading_them() {
    let (_dir, store) = setup().await;

    // store を経由しない直接の INSERT（過去データや手動編集を模す）。
    // **0004b-1 以降は CHECK を無効にしないと作れない**（0005 / ADR-0017）。
    insert_ghost_character(&store, " aoi ").await;

    store
        .create_character(
            &name("rin"),
            &display("rin"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    let entries = store.list_characters().await.expect("一覧を取得できること");
    assert_eq!(
        readable(&entries).len(),
        1,
        "正規化済みの行だけが読めた行になること: {entries:?}"
    );
    assert_eq!(readable(&entries)[0].handle, name("rin"));
    assert_eq!(
        unreadable(&entries).len(),
        1,
        "正規化されていない行を黙って捨てないこと: {entries:?}"
    );
    assert_eq!(
        unreadable(&entries)[0].invalid_columns,
        vec![CharacterColumn::Handle],
        "壊れているのは識別子の列だと分かること: {entries:?}"
    );

    // 検索側で trim しない（trim すると書き込み先が不定になる / ADR-0004）。
    assert!(
        store
            .find_character_by_handle(&name("aoi"))
            .await
            .expect("検索が成功すること")
            .is_none(),
        "正規化されていない行は完全一致で引けないこと"
    );

    // **上限を超える名前の行も同じ扱い。** `Handle` を作れない以上、
    // `model` からも `find_character_by_handle` からも決して引けないので、
    // 一覧に出すと「選べるのに 404 になるモデル」になる。
    // **CHECK は通る綴りにする**（`a` の並びなので文字集合には触れない）。
    // 引っかかるのはアプリ層の `MAX_CHARS` だけ、という状況をわざと作る。
    sqlx::query("INSERT INTO character (handle, base_persona) VALUES (?, ?)")
        .bind("a".repeat(Handle::MAX_CHARS + 1))
        .bind("ペルソナ本文")
        .execute(store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");

    let entries = store.list_characters().await.expect("一覧を取得できること");
    assert_eq!(
        readable(&entries).len(),
        1,
        "上限を超える名前の行も読めた行にはしないこと: {entries:?}"
    );
    assert_eq!(readable(&entries)[0].handle, name("rin"));
    assert_eq!(
        unreadable(&entries).len(),
        2,
        "上限を超える名前の行も指し示すこと: {entries:?}"
    );
    assert!(
        unreadable(&entries)
            .iter()
            .any(|broken| broken.handle.is_none()),
        "`Handle` を作れない行は id だけが手掛かりになること: {entries:?}"
    );
}

/// **同じ表示名で、識別子の違うキャラクターを2件持てる。**
///
/// **この作業単位の存在理由**（ADR-0011）。`name` が呼び名と識別子と一意性のキーを
/// 兼ねていたときは、`葵` の変種を並べるには `葵-実験A` のような名前を付けるしかなく、
/// **プロンプトに実験用の名前が載っていた**（比べたいのはペルソナなのに、
/// 名前という別の変数が混ざる）。
#[tokio::test]
async fn two_characters_may_share_a_display_name() {
    let (_dir, store) = setup().await;

    let first = store
        .create_character(
            &name("aoi"),
            &display("葵"),
            &persona("ペルソナA"),
            None,
            fixture_scale(),
        )
        .await
        .expect("1件目を作成できること");
    let second = store
        .create_character(
            &name("aoi-v2"),
            &display("葵"),
            &persona("ペルソナB"),
            None,
            fixture_scale(),
        )
        .await
        .expect("表示名が同じでも識別子が違えば作成できること");
    assert_ne!(first, second, "別の行として作られること");

    // **識別子ではそれぞれ引ける**（`model` からの同定は壊れていない）。
    let one = store
        .find_character_by_handle(&name("aoi"))
        .await
        .expect("検索が成功すること")
        .expect("引けること");
    let two = store
        .find_character_by_handle(&name("aoi-v2"))
        .await
        .expect("検索が成功すること")
        .expect("引けること");

    assert_eq!(one.display_name, display("葵"));
    assert_eq!(two.display_name, display("葵"));
    assert_eq!(one.display_name, two.display_name, "呼び名は同じであること");
    assert_ne!(one.handle, two.handle, "識別子は別であること");
    assert_ne!(
        one.base_persona, two.base_persona,
        "ペルソナだけが違う個体になっていること"
    );

    let entries = store.list_characters().await.expect("一覧を取得できること");
    assert_eq!(
        readable(&entries).len(),
        2,
        "どちらも一覧に出ること: {entries:?}"
    );
}

/// **表示名が不正な行も、黙って読み替えたり消したりしない。**
///
/// ペルソナ・形式指示とまったく同じ理由（`Store::list_characters` の doc）。
/// 表示名も削れない枠に載るので、上限を超えた値を通すと以後のチャットが
/// `BudgetExhausted` になり続ける。
///
/// * 1件を指す読み出し（`find_character_by_handle`）は**止まる**。
/// * 一覧は**その行を指し示す**（ADR-0016 §3。止めると他のキャラまで巻き添えになる）。
///
/// この状態は `store` 経由の作成では起こらず、DB を直接編集した場合にのみ現れる
/// （空文字はスキーマの CHECK が別途弾く）。
#[tokio::test]
async fn a_row_with_an_invalid_display_name_stops_the_read_instead_of_disappearing() {
    let (_dir, store) = setup().await;

    // 上限を超える表示名（貼り付け事故や直接編集を模す）。
    let oversized = "葵".repeat(DisplayName::MAX_CHARS + 1);
    sqlx::query("INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)")
        .bind("rin")
        .bind(&oversized)
        .bind("ペルソナ本文")
        .execute(store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること（長さはスキーマでは縛らない）");

    let entries = store
        .list_characters()
        .await
        .expect("一覧そのものは成功すること（他のキャラを巻き添えにしない）");
    let broken = unreadable(&entries);
    assert_eq!(
        broken.len(),
        1,
        "不正な表示名の行を黙って通さないこと: {entries:?}"
    );
    assert_eq!(
        broken[0].invalid_columns,
        vec![CharacterColumn::DisplayName],
        "壊れている列が分かること: {entries:?}"
    );
    assert!(
        !format!("{entries:?}").contains('葵'),
        "格納値そのものは返さないこと: {entries:?}"
    );

    let found = store
        .find_character_by_handle(&name("rin"))
        .await
        .expect_err("存在しない扱いにしないこと");
    assert!(
        matches!(found, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {found:?}"
    );
    // **`Display` に格納値を載せない**（CLAUDE.md 絶対制約。エラー応答に載りうる）。
    // 上の `{entries:?}` の検査は `CharacterEntry` の `Debug` を見ているだけで、
    // **エラー型の `Display` は見ていない**（0012b で入れ替わっていた）。
    assert!(
        !found.to_string().contains('葵'),
        "Display に格納値を含めないこと: {found}"
    );

    // 制御文字入りも同じ（`prompt` の無害化に頼らず、入口で止める）。
    sqlx::query("UPDATE character SET display_name = ? WHERE handle = ?")
        .bind("葵\u{0007}碧")
        .bind("rin")
        .execute(store.pool())
        .await
        .expect("直接の UPDATE はスキーマ上通ること");
    assert!(
        matches!(
            store.find_character_by_handle(&name("rin")).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "制御文字入りの表示名も握り潰さないこと"
    );

    // 前後に空白があるだけの行は正規化して受理する（ペルソナと同じ扱い。照合キーではない）。
    sqlx::query("UPDATE character SET display_name = ? WHERE handle = ?")
        .bind(" \u{3000}凛 ")
        .bind("rin")
        .execute(store.pool())
        .await
        .expect("直接の UPDATE はスキーマ上通ること");
    let rin = store
        .find_character_by_handle(&name("rin"))
        .await
        .expect("検索が成功すること")
        .expect("引けること");
    assert_eq!(rin.display_name, display("凛"));
}

/// **ペルソナが不正な行は、黙って除外せず読み出しを止める。**
///
/// 除外してはならない理由は `Store::list_characters` の doc にある: この行の名前は
/// 正常なので、一覧から落とすと「一覧に出ない・チャットは 404・しかし作ろうとすると
/// 『既に存在します』」という **ADR-0004 が潰した袋小路がそのまま復活する。**
/// 1件を指す読み出しは止め、一覧では**壊れた行として指し示す**（ADR-0016 §3）。
///
/// この状態は `store` 経由の作成では起こらず、DB を直接編集した場合にのみ現れる。
#[tokio::test]
async fn a_row_with_an_invalid_persona_stops_the_read_instead_of_disappearing() {
    let (_dir, store) = setup().await;
    store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    // 上限を超えるペルソナ（貼り付け事故や上限導入前のデータを模す）。
    let oversized = "あ".repeat(Persona::MAX_CHARS + 1);
    sqlx::query("INSERT INTO character (handle, base_persona) VALUES (?, ?)")
        .bind("rin")
        .bind(&oversized)
        .execute(store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");

    let entries = store
        .list_characters()
        .await
        .expect("一覧そのものは成功すること（他のキャラを巻き添えにしない）");
    let broken = unreadable(&entries);
    assert_eq!(
        broken.len(),
        1,
        "不正なペルソナの行を黙って除外しないこと: {entries:?}"
    );
    assert_eq!(
        broken[0].invalid_columns,
        vec![CharacterColumn::BasePersona],
        "壊れている列が分かること: {entries:?}"
    );
    assert_eq!(
        readable(&entries).len(),
        1,
        "正常な行は巻き添えにならないこと: {entries:?}"
    );
    assert!(
        !format!("{entries:?}").contains('あ'),
        "格納値そのものは返さないこと: {entries:?}"
    );

    // **`None` にも潰さない。**「存在しない」と返すと、作り直そうとしたときの
    // 409「既に存在します」と食い違い、原因に到達できなくなる。
    let found = store
        .find_character_by_handle(&name("rin"))
        .await
        .expect_err("存在しない扱いにしないこと");
    assert!(
        matches!(found, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {found:?}"
    );
    // **`Display` に格納値を載せない**（表示名の側と同じ理由）。
    assert!(
        !found.to_string().contains('あ'),
        "Display に格納値を含めないこと: {found}"
    );

    // 空のペルソナも同じ（`prompt` が節ごと落とすので、通すと黙って無個性になる）。
    sqlx::query("UPDATE character SET base_persona = ? WHERE handle = ?")
        .bind("   ")
        .bind("rin")
        .execute(store.pool())
        .await
        .expect("直接の UPDATE はスキーマ上通ること");
    assert!(
        matches!(
            store.find_character_by_handle(&name("rin")).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "空白だけのペルソナも握り潰さないこと"
    );

    // 前後に空白があるだけの行は正規化して受理する（軸名と同じ扱い。照合キーではない）。
    sqlx::query("UPDATE character SET base_persona = ? WHERE handle = ?")
        .bind(" \u{3000}ペルソナ本文\n")
        .bind("rin")
        .execute(store.pool())
        .await
        .expect("直接の UPDATE はスキーマ上通ること");
    let rin = store
        .find_character_by_handle(&name("rin"))
        .await
        .expect("検索が成功すること")
        .expect("引けること");
    assert_eq!(rin.base_persona, persona("ペルソナ本文"));
}

/// **形式指示が不正な行も、黙って未設定にせず読み出しを止める。**
///
/// ペルソナとまったく同じ理由（`Store::list_characters` の doc）。形式指示も
/// 削れない枠に載るので、上限を超えた値を通すと以後のチャットが
/// `BudgetExhausted` になり続ける。**`None` に潰すと「設定したのに節が出ない」**
/// という、原因に到達できない壊れ方になる。
///
/// この状態は `store` 経由の作成では起こらず、DB を直接編集した場合にのみ現れる。
#[tokio::test]
async fn a_row_with_an_invalid_output_format_stops_the_read_instead_of_disappearing() {
    let (_dir, store) = setup().await;

    // 上限を超える形式指示（上限導入前のデータや直接編集を模す）。
    let oversized = "囲".repeat(OutputFormat::MAX_CHARS + 1);
    sqlx::query(
        "INSERT INTO character (handle, base_persona, output_format_instruction) VALUES (?, ?, ?)",
    )
    .bind("rin")
    .bind("ペルソナ本文")
    .bind(&oversized)
    .execute(store.pool())
    .await
    .expect("直接の INSERT はスキーマ上通ること");

    let entries = store
        .list_characters()
        .await
        .expect("一覧そのものは成功すること（他のキャラを巻き添えにしない）");
    let broken = unreadable(&entries);
    assert_eq!(
        broken.len(),
        1,
        "不正な形式指示の行を黙って通さないこと: {entries:?}"
    );
    assert_eq!(
        broken[0].invalid_columns,
        vec![CharacterColumn::OutputFormatInstruction],
        "壊れている列が分かること: {entries:?}"
    );
    assert!(
        !format!("{entries:?}").contains('囲'),
        "格納値そのものは返さないこと: {entries:?}"
    );

    let found = store
        .find_character_by_handle(&name("rin"))
        .await
        .expect_err("未設定扱いにしないこと");
    assert!(
        matches!(found, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {found:?}"
    );
    // **`Display` に格納値を載せない**（表示名の側と同じ理由）。
    assert!(
        !found.to_string().contains('囲'),
        "Display に格納値を含めないこと: {found}"
    );

    // **空文字も `None` に潰さない。** 潰すと「未設定」の表現が NULL と '' の2通りになり、
    // `OutputFormat::parse` が空を拒否している意味が消える。
    sqlx::query("UPDATE character SET output_format_instruction = ? WHERE handle = ?")
        .bind("   ")
        .bind("rin")
        .execute(store.pool())
        .await
        .expect("直接の UPDATE はスキーマ上通ること");
    assert!(
        matches!(
            store.find_character_by_handle(&name("rin")).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "空白だけの形式指示も握り潰さないこと"
    );

    // 前後に空白があるだけの行は正規化して受理する（ペルソナと同じ扱い）。
    sqlx::query("UPDATE character SET output_format_instruction = ? WHERE handle = ?")
        .bind(" \u{3000}台詞は「」で囲む。\n")
        .bind("rin")
        .execute(store.pool())
        .await
        .expect("直接の UPDATE はスキーマ上通ること");
    let rin = store
        .find_character_by_handle(&name("rin"))
        .await
        .expect("検索が成功すること")
        .expect("引けること");
    assert_eq!(
        rin.output_format_instruction,
        Some(output_format("台詞は「」で囲む。"))
    );

    // NULL は未設定（`None`）として通る。
    sqlx::query("UPDATE character SET output_format_instruction = NULL WHERE handle = ?")
        .bind("rin")
        .execute(store.pool())
        .await
        .expect("直接の UPDATE はスキーマ上通ること");
    let rin = store
        .find_character_by_handle(&name("rin"))
        .await
        .expect("検索が成功すること")
        .expect("引けること");
    assert_eq!(rin.output_format_instruction, None);
}

/// `stat_alloc` の設定は**丸ごと差し替え**。前の軸は残らない。
#[tokio::test]
async fn replacing_stat_allocs_removes_previous_axes() {
    let (_dir, store) = setup().await;
    let id = store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    store
        .replace_stat_allocs(id, &[alloc("パッション", 3), alloc("ロジック", 5)])
        .await
        .expect("性格値を設定できること");
    store
        .replace_stat_allocs(id, &[alloc("パッション", 1), alloc("共感", 2)])
        .await
        .expect("性格値を再設定できること");

    let allocs = store.stat_allocs(id).await.expect("性格値を取得できること");
    assert_eq!(
        allocs,
        vec![alloc("パッション", 1), alloc("共感", 2)],
        "前回の軸（ロジック）が残らないこと"
    );

    // 空の配列で完全に消せる。
    store
        .replace_stat_allocs(id, &[])
        .await
        .expect("空の設定もできること");
    assert!(
        store
            .stat_allocs(id)
            .await
            .expect("性格値を取得できること")
            .is_empty(),
        "空で置き換えると軸が残らないこと"
    );
}

/// **値 0 の軸も保存・取得できる。** 落とすのは `prompt` 層の責務であり、保存段階では落とさない。
#[tokio::test]
async fn zero_valued_axis_is_persisted() {
    let (_dir, store) = setup().await;
    let id = store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    store
        .replace_stat_allocs(id, &[alloc("パッション", 0), alloc("ロジック", 5)])
        .await
        .expect("値 0 を含む設定ができること");

    let allocs = store.stat_allocs(id).await.expect("性格値を取得できること");
    assert_eq!(
        allocs,
        vec![alloc("パッション", 0), alloc("ロジック", 5)],
        "値 0 の軸が保存段階で落ちないこと"
    );
}

/// 軸名は正規化された値で保存される（前後の空白で軸が分裂しない）。
#[tokio::test]
async fn axis_name_is_normalized_before_it_reaches_the_database() {
    let (_dir, store) = setup().await;
    let id = store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    store
        .replace_stat_allocs(id, &[alloc(" \u{3000}パッション\t", 3)])
        .await
        .expect("性格値を設定できること");

    let stored: String = sqlx::query_scalar("SELECT axis FROM stat_alloc WHERE character_id = ?")
        .bind(id.get())
        .fetch_one(store.pool())
        .await
        .expect("保存された軸名を取得できること");
    assert_eq!(stored, "パッション", "DB には正規化後の軸名が入ること");
}

/// 取得順は**設定時の順序**を保つ（プロンプトへの並びを呼び出し側が決められる）。
#[tokio::test]
async fn stat_allocs_keep_the_given_order() {
    let (_dir, store) = setup().await;
    let id = store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    // 辞書順とは異なる順序で設定する。
    let given = [alloc("ロジック", 5), alloc("あいさつ", 1), alloc("共感", 2)];
    store
        .replace_stat_allocs(id, &given)
        .await
        .expect("性格値を設定できること");

    assert_eq!(
        store.stat_allocs(id).await.expect("性格値を取得できること"),
        given.to_vec(),
        "設定した順序で返ること"
    );
}

/// 置き換えは**同一トランザクション**。途中で失敗したら前の状態が保たれる。
#[tokio::test]
async fn failed_replacement_leaves_previous_allocs_intact() {
    let (_dir, store) = setup().await;
    let id = store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    let original = [alloc("パッション", 3), alloc("ロジック", 5)];
    store
        .replace_stat_allocs(id, &original)
        .await
        .expect("性格値を設定できること");

    // 同一軸の重複は主キー違反。DELETE だけが適用されて空になってはならない。
    let err = store
        .replace_stat_allocs(id, &[alloc("共感", 1), alloc("共感", 2)])
        .await
        .expect_err("重複した軸は弾かれること");
    assert_query_kind(&err, ErrorKind::UniqueViolation, "stat_alloc の主キー");

    assert_eq!(
        store.stat_allocs(id).await.expect("性格値を取得できること"),
        original.to_vec(),
        "失敗した置き換えがロールバックされること"
    );
}

/// 性格値はキャラクターごとに独立している。
#[tokio::test]
async fn stat_allocs_are_scoped_to_one_character() {
    let (_dir, store) = setup().await;
    let aoi = store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");
    let rin = store
        .create_character(
            &name("rin"),
            &display("rin"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    store
        .replace_stat_allocs(aoi, &[alloc("パッション", 3)])
        .await
        .expect("性格値を設定できること");
    store
        .replace_stat_allocs(rin, &[alloc("ロジック", 5)])
        .await
        .expect("性格値を設定できること");
    // aoi の置き換えが rin に波及しないこと。
    store
        .replace_stat_allocs(aoi, &[alloc("共感", 1)])
        .await
        .expect("性格値を再設定できること");

    assert_eq!(
        store
            .stat_allocs(aoi)
            .await
            .expect("性格値を取得できること"),
        vec![alloc("共感", 1)]
    );
    assert_eq!(
        store
            .stat_allocs(rin)
            .await
            .expect("性格値を取得できること"),
        vec![alloc("ロジック", 5)],
        "別キャラの性格値が消えないこと"
    );
}

/// 直接 INSERT された軸名の扱い（`store` 経由では起こらない状態の固定）。
///
/// * 前後に空白のある軸名は**正規化して返す**。軸は照合キーではないので、
///   取りこぼすより正規化して見せる方が安全。
/// * 制御文字入りの軸名は握り潰さず `InvalidStoredValue` にする。
/// * **上限を超える軸名も同じ**（削れない枠に載るので、通すとチャットが予算超過で死ぬ）。
#[tokio::test]
async fn stat_allocs_of_rows_inserted_outside_the_store() {
    let (_dir, store) = setup().await;
    let id = store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(id.get())
        .bind(" パッション\u{3000}")
        .bind(3_i64)
        .execute(store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");

    assert_eq!(
        store.stat_allocs(id).await.expect("性格値を取得できること"),
        vec![alloc("パッション", 3)],
        "前後に空白のある軸名は正規化して返ること"
    );

    sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(id.get())
        .bind("ロ\nジック")
        .bind(5_i64)
        .execute(store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");

    let err = store
        .stat_allocs(id)
        .await
        .expect_err("制御文字入りの軸名は握り潰さないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {err:?}"
    );
    assert!(
        !err.to_string().contains("ジック"),
        "Display に格納値を含めないこと: {err}"
    );

    // **上限超過の軸名も握り潰さない。** 軸は削れない枠に件数分載るので、
    // 通すとそのキャラクターのチャットが以後ずっと予算超過で失敗しうる。
    let (_dir, store) = setup().await;
    let id = store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");
    sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(id.get())
        .bind("情".repeat(StatAxis::MAX_CHARS + 1))
        .bind(3_i64)
        .execute(store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");

    let err = store
        .stat_allocs(id)
        .await
        .expect_err("上限を超える軸名は握り潰さないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {err:?}"
    );
    assert!(
        !err.to_string().contains('情'),
        "Display に格納値を含めないこと: {err}"
    );
}

/// 格納値が `u32` に収まらない場合は `InvalidStoredValue` に寄せる。
///
/// スキーマの CHECK は下限（0以上・整数型）しか見ないため、DB を直接編集すると
/// `4294967296` のような値が残りうる。ここを sqlx のデコードエラー（`StoreError::Query`）の
/// まま返すと、ログを見た人が「クエリが失敗した」と読んで DB の中身を疑わない。
/// 直上の軸名も同じ「DB 直接編集」由来なので、分類を揃える。
///
/// # **「`u32::MAX` は正当な値として返る」はもう成り立たない**（作業単位 0011a-2）
///
/// 値が尺度に従うようになったので、上端は `u32::MAX` ではなく**そのキャラクターの尺度**。
/// 境界（尺度ちょうど / 1つ超え）は
/// `a_stored_value_beyond_the_characters_scale_stops_the_read` が見る。
/// **ここに残っているのは「デコード失敗として返さない」という分類の話だけ。**
#[tokio::test]
async fn stat_alloc_value_beyond_u32_is_reported_as_invalid_stored_value() {
    let (_dir, store) = setup().await;
    let id = store
        .create_character(
            &name("aoi"),
            &display("aoi"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    // u32::MAX + 1 は格納値の不正として型付けされる（デコード失敗ではない）。
    sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(id.get())
        .bind("超過")
        .bind(i64::from(u32::MAX) + 1)
        .execute(store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");

    let err = store
        .stat_allocs(id)
        .await
        .expect_err("u32 に収まらない値は握り潰さないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "デコード失敗ではなく格納値の不正として分類されること: {err:?}"
    );
    assert!(
        !err.to_string().contains("4294967296"),
        "Display に格納値を含めないこと: {err}"
    );
}

/// 存在しないキャラクターの性格値は空。設定しようとすると外部キー違反。
///
/// # **キャラクターが1件も無い DB で測らないこと**（0011a-2 のレビューで実測）
///
/// 以前このテストは**空の `character` テーブル**の上で走っていた。すると
/// `SELECT_STAT_SCALE_BY_ID` の `WHERE id = ?` を
/// `WHERE id >= ? ORDER BY id LIMIT 1` に変える変異が **813/813 緑**を通る
/// ——引き当てが緩んでも、当たる行が1件も無いから。
///
/// **`unknown` より大きい id の行を1件残しておく。** 尺度も `fixture_scale()` と
/// 別の値にしてあるので、引き当てが緩めば置き換えは外部キー違反ではなく
/// `StatScaleMismatch` になり、ここで赤くなる
/// （**囮の尺度を `fixture_scale()` と同値にすると、この見張りは効かない。**
/// 尺度が一致してしまい、変異しても外部キー違反のまま通る）。
#[tokio::test]
async fn stat_allocs_of_unknown_character() {
    let (_dir, store) = setup().await;

    // **id は連番なので、消した行の id を「存在しない id」に使う。**
    // こうすると、残る囮の id は必ずそれより大きくなる。
    let vacated = store
        .create_character(
            &name("aoi-v2"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            None,
            fixture_scale(),
        )
        .await
        .expect("先に作れること");
    let decoy_scale = StatScale::parse(3).expect("正当な尺度であること");
    assert_ne!(
        decoy_scale,
        fixture_scale(),
        "囮の尺度はフィクスチャの尺度と違うこと（同値だと引き当ての緩みを検出できない）"
    );
    let decoy = store
        .create_character(
            &name("akane"),
            &display("茜音"),
            &persona("昼番の受付。"),
            None,
            decoy_scale,
        )
        .await
        .expect("囮を作れること");
    store
        .delete_character(&name("aoi-v2"))
        .await
        .expect("会話の無いキャラクターは消せること");

    let unknown = CharacterId::from_raw_for_tests(vacated.get());
    assert!(
        decoy > unknown,
        "囮の id は存在しない id より大きいこと: 囮 = {decoy:?} / 存在しない id = {unknown:?}"
    );

    assert!(
        store
            .stat_allocs(unknown)
            .await
            .expect("取得自体は成功すること")
            .is_empty(),
        "存在しないキャラクターの性格値は空であること"
    );

    let err = store
        .replace_stat_allocs(unknown, &[alloc("パッション", 3)])
        .await
        .expect_err("存在しないキャラクターには設定できないこと");
    assert_query_kind(
        &err,
        ErrorKind::ForeignKeyViolation,
        "stat_alloc.character_id",
    );

    // **空配列は存在しないキャラクターでも成功する。** DELETE が0件で成功し、
    // INSERT が1件も走らないため外部キーに触れない。
    // これは DELETE の冪等性として**許容した挙動**であり、たまたまの結果ではない。
    store
        .replace_stat_allocs(unknown, &[])
        .await
        .expect("空配列は存在しない character_id でも成功すること（DELETE の冪等性）");
}

// ======================================================================
// 更新と削除（ADR-0016 / 作業単位 0012a）
// ======================================================================

/// 更新の検証で使う値。**4つとも別の文字列にする。**
///
/// 同値にすると「表示名とペルソナを取り違える」「識別子を表示名の列に書く」といった
/// 変異が緑を通る。**このプロジェクトで最も繰り返し出ている穴**（CLAUDE.md）。
const HANDLE: &str = "aoi";
const DISPLAY: &str = "葵";
const PERSONA: &str = "静かな図書館の司書。";
const FORMAT: &str = "台詞は「」で囲む。";

/// 更新後の値。**元の値とも、互いにも別の文字列にする。**
const NEW_DISPLAY: &str = "碧";
const NEW_PERSONA: &str = "夜勤の警備員。";
const NEW_FORMAT: &str = "地の文は敬体で書く。";

/// **巻き添えを見るための「隣の行」の値。** これも他のどれとも別の文字列にする。
///
/// `NEW_DISPLAY` と同値にすると、隣の行まで書き換える実装が
/// 「元から同じ値だった」と区別できなくなる（更新の巻き添えが緑を通る）。
const NEIGHBOUR_DISPLAY: &str = "楓";
const NEIGHBOUR_PERSONA: &str = "隣町の郵便局員。";

/// 一覧（作業単位 0012b）で使う値。**壊れた行と正常な行を、値でも識別子でも分ける。**
///
/// 同値にすると「壊れた行の情報を隣の行から作る」実装が緑を通る。
const HEALTHY_HANDLE: &str = "kaede";
const HEALTHY_DISPLAY: &str = "楓花";
const HEALTHY_PERSONA: &str = "昼番の受付。";
const BROKEN_HANDLE: &str = "rin";
/// 一覧が返した識別子で直したあとの表示名。**他のどれとも別の文字列。**
const REPAIRED_DISPLAY: &str = "凛";

/// 上限超過の値を作るための埋め草。**他のフィクスチャに現れない文字**にすること
/// （「生の格納値を返していない」ことを、この文字が出てこないことで確かめるため）。
const OVERSIZED_FILLER: &str = "零";

/// **`TRIMMABLE` に入っていない、空白めいた文字。**
///
/// 「うっかり足しそうな文字」を並べてある。引き当てをここまで広げると、
/// 完全一致では別物の行を WHERE が拾い、**別のキャラクターを巻き添えにする。**
/// 使うのは `an_update_or_delete_only_ever_reaches_the_row_whose_handle_matches_exactly`
/// （**`TRIMMABLE` の8文字と併せて**回す。0004b-2 で引き当てが完全一致になったので、
/// `TRIMMABLE` の側も「拾ってはいけない集合」に移った）。
///
/// U+200B は**レビュアーが実測で巻き添えを再現した文字**なので必ず残すこと。
const PADDINGS_OUTSIDE_TRIMMABLE: [char; 8] = [
    '\u{200B}', // ZERO WIDTH SPACE（Cf）— 実測で2行に一致した
    '\u{FEFF}', // ZERO WIDTH NO-BREAK SPACE / BOM（Cf）
    '\u{2000}', // EN QUAD（White_Space）
    '\u{2003}', // EM SPACE（White_Space）
    '\u{202F}', // NARROW NO-BREAK SPACE（White_Space）
    '\u{205F}', // MEDIUM MATHEMATICAL SPACE（White_Space）
    '\u{1680}', // OGHAM SPACE MARK（White_Space）
    '\u{0085}', // NEL（`LINE_BREAKS` にはあるが `TRIMMABLE` には無い）
];

/// **照合式が「完全一致より広い」と拾ってしまう行**（0004b-2 の追補）。
///
/// `(格納名, 表示名, ゴースト行として入れるか)`。**表示名は行ごとに別の値**にする
/// （同値にすると行の取り違えがそのまま緑になる）。
///
/// # 何を並べてあるか
///
/// * **`_` を含む正当な識別子**（`a_i`）。`_` は `Handle::ALLOWED_CHARACTERS` に
///   **含まれる**ので、`WHERE name = ?` を `WHERE name LIKE ?` に変えるだけで
///   任意の1文字に化ける。`a-i` はその対照（同じ長さで `_` を含まない）。
/// * **ASCII の大文字だけが違う行**（`AOI` / `KAEDE`）。SQLite の `LIKE` は
///   ASCII について大文字小文字を区別しないので、`COLLATE NOCASE` を足したのと同じ穴が開く。
///   スキーマの CHECK は `GLOB`（大小を区別する）なのでゴーストとしてしか入らない。
/// * **`" aoi "`**（`TRIMMABLE` で埋まった行）。`trim` の復活を見る。
///
/// # 並び順に意味がある
///
/// 引き当てが広がったとき `fetch_optional` は**先に見つけた行**を返すので、
/// **完全一致する行は rowid の後ろに置く**（`AOI` → `aoi` の順）。
/// 逆順にすると、緩んでいても正しい行が返って緑になる。
const WIDENING_ROWS: [(&str, &str, bool); 7] = [
    ("AOI", "大文字の葵", true),
    (" aoi ", "空白付きの葵", true),
    ("aoi", "葵", false),
    ("a-i", "ハイフンの藍", false),
    ("a_i", "下線の藍", false),
    ("KAEDE", "大文字の楓", true),
    ("xyz", "エックス", false),
];

/// `WIDENING_ROWS` を順に挿入し、`(格納名, 表示名, id)` を返す。
async fn insert_widening_rows(store: &Store) -> Vec<(&'static str, &'static str, i64)> {
    let mut shown: Vec<&str> = WIDENING_ROWS.iter().map(|(_, shown, _)| *shown).collect();
    let all = shown.len();
    shown.sort_unstable();
    shown.dedup();
    assert_eq!(
        shown.len(),
        all,
        "前提: 表示名は行ごとに違うこと（同値にすると行の取り違えが緑になる）"
    );

    let mut inserted = Vec::with_capacity(WIDENING_ROWS.len());
    for (stored, shown, ghost) in WIDENING_ROWS {
        let id = if ghost {
            // **0004b-1 以降は CHECK を無効にしないと作れない**（0005 / ADR-0017）。
            insert_ghost_character_with(store, stored, shown, "ペルソナ本文").await
        } else {
            store
                .create_character(
                    &name(stored),
                    &display(shown),
                    &persona("ペルソナ本文"),
                    None,
                    fixture_scale(),
                )
                .await
                .expect("キャラクターを作成できること")
                .get()
        };
        inserted.push((stored, shown, id));
    }
    inserted
}

/// フィクスチャの値が互いに違うことを、使う前に確かめる。
///
/// **2つを同値にした時点で、その2つの取り違えは検出できなくなる。**
fn assert_fixture_values_are_distinct() {
    let values = [
        HANDLE,
        DISPLAY,
        PERSONA,
        FORMAT,
        NEW_DISPLAY,
        NEW_PERSONA,
        NEW_FORMAT,
        NEIGHBOUR_DISPLAY,
        NEIGHBOUR_PERSONA,
        HEALTHY_HANDLE,
        HEALTHY_DISPLAY,
        HEALTHY_PERSONA,
        BROKEN_HANDLE,
        REPAIRED_DISPLAY,
        OVERSIZED_FILLER,
    ];
    for (index, left) in values.iter().enumerate() {
        for right in &values[index + 1..] {
            assert_ne!(left, right, "フィクスチャの値は互いに違うこと");
        }
    }
}

/// 表示名・ペルソナ・形式指示がすべて別の値で埋まったキャラクターを1件作る。
async fn create_fixture(store: &Store) -> CharacterId {
    assert_fixture_values_are_distinct();
    store
        .create_character(
            &name(HANDLE),
            &display(DISPLAY),
            &persona(PERSONA),
            Some(&output_format(FORMAT)),
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること")
}

/// 表示名だけを差し替える更新。
fn update_display(value: &str) -> CharacterUpdate {
    CharacterUpdate {
        display_name: Some(display(value)),
        ..CharacterUpdate::default()
    }
}

async fn character_count(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM character")
        .fetch_one(store.pool())
        .await
        .expect("行数を数えられること")
}

async fn session_count(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM session")
        .fetch_one(store.pool())
        .await
        .expect("行数を数えられること")
}

/// 格納名で1行だけ引いて `display_name` を返す（`parse` を通さずに見る）。
///
/// **格納名は完全一致で渡す。** 正規化キーで引くと、検証したい「引き当ての広さ」を
/// テスト側でも同じ式で再現することになり、ドリフトが両側で同時に起きて緑を通る。
async fn display_name_of(store: &Store, stored_name: &str) -> String {
    sqlx::query_scalar("SELECT display_name FROM character WHERE handle = ?")
        .bind(stored_name)
        .fetch_one(store.pool())
        .await
        .expect("格納名で1行だけ引けること")
}

/// 全行の `(name, display_name)` を id 順で取り出す（`parse` を通さずに見る）。
///
/// **1行ずつ引かない。** 「巻き添えで書き換わった／消えた行」は、
/// 名指しで引く形だと**引く相手を思い付かなかった行**が観測から漏れる。
async fn stored_rows(store: &Store) -> Vec<(String, String)> {
    sqlx::query_as("SELECT handle, display_name FROM character ORDER BY id")
        .fetch_all(store.pool())
        .await
        .expect("格納値を取得できること")
}

/// 格納されている `handle` を全行ぶん取り出す（`parse` を通さずに見る）。
async fn stored_handles(store: &Store) -> Vec<String> {
    sqlx::query_scalar("SELECT handle FROM character ORDER BY id")
        .fetch_all(store.pool())
        .await
        .expect("格納名を取得できること")
}

/// **読めない行を、読まずに直せる。**〈この作業単位の要 / ADR-0016 §4〉
///
/// `find_character_by_handle` は全フィールドを `parse` するので、更新がその経路を通ると
/// **壊れた行は永久に直せない**（0009 以前に作れた U+2028 の表示名や、上限導入前の
/// 巨大なペルソナがまさにその状態）。ここが赤くなったら、この作業単位は目的を失っている。
#[tokio::test]
async fn an_update_repairs_a_row_that_cannot_be_read() {
    let (_dir, store) = setup().await;
    assert_fixture_values_are_distinct();

    // **`parse` を迂回して壊れた行を作る。** U+2028 は `char::is_control` が偽なので
    // スキーマの CHECK も通る（0009 以前は `/admin/` から作れた）。
    // ペルソナも同時に上限超過にしておく（表示名だけ直しても読めないままにするため）。
    sqlx::query("INSERT INTO character (handle, display_name, base_persona) VALUES (?, ?, ?)")
        .bind(HANDLE)
        .bind("葵\u{2028}碧")
        .bind("あ".repeat(Persona::MAX_CHARS + 1))
        .execute(store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");

    // 前提: この行は読み出せない。
    assert!(
        matches!(
            store.find_character_by_handle(&name(HANDLE)).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "前提: 壊れた行は `find_character_by_handle` では読めないこと"
    );

    // **読めない状態のまま更新が成功すること。**
    let id = store
        .update_character(
            &name(HANDLE),
            &CharacterUpdate {
                display_name: Some(display(NEW_DISPLAY)),
                base_persona: Some(persona(NEW_PERSONA)),
                output_format_instruction: FieldUpdate::Keep,
                // **尺度は触らない。** この場面で見たいのは他の列なので、
                // 「省略した列は変わらない」側に倒しておく。
                stat_scale: None,
                // 走らせ方も同じ（作業単位 A-8）。
                stat_drift_mode: FieldUpdate::Keep,
                axis_discovery_slots: None,
                archived: None,
            },
        )
        .await
        .expect("読めない行でも更新できること（parse を経由しないこと）");

    let repaired = store
        .find_character_by_handle(&name(HANDLE))
        .await
        .expect("修復後は読めること")
        .expect("行が残っていること");
    assert_eq!(repaired.id, id, "更新した行の id が返ること");
    assert_eq!(repaired.display_name, display(NEW_DISPLAY));
    assert_eq!(repaired.base_persona, persona(NEW_PERSONA));
    assert_eq!(
        repaired.handle,
        name(HANDLE),
        "識別子は更新の対象ではないこと"
    );
    assert_eq!(character_count(&store).await, 1, "行が増えていないこと");
}

/// **指定しなかった列は変わらない**（部分更新）。
///
/// 3つとも別の値にしてあるので、「省略された列を既定値で潰す」実装も
/// 「別の列に書く」実装もここで赤くなる。
#[tokio::test]
async fn an_update_leaves_the_fields_it_does_not_mention_alone() {
    let (_dir, store) = setup().await;
    let id = create_fixture(&store).await;

    // 表示名だけ変える。
    assert_eq!(
        store
            .update_character(&name(HANDLE), &update_display(NEW_DISPLAY))
            .await
            .expect("更新できること"),
        id
    );
    let after = store
        .find_character_by_handle(&name(HANDLE))
        .await
        .expect("検索が成功すること")
        .expect("引けること");
    assert_eq!(after.display_name, display(NEW_DISPLAY), "表示名は変わる");
    assert_eq!(
        after.base_persona,
        persona(PERSONA),
        "指定しなかったペルソナは元のままであること"
    );
    assert_eq!(
        after.output_format_instruction,
        Some(output_format(FORMAT)),
        "指定しなかった形式指示は元のままであること"
    );

    // ペルソナだけ変える（表示名は直前の更新結果を保つ）。
    store
        .update_character(
            &name(HANDLE),
            &CharacterUpdate {
                base_persona: Some(persona(NEW_PERSONA)),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("更新できること");
    let after = store
        .find_character_by_handle(&name(HANDLE))
        .await
        .expect("検索が成功すること")
        .expect("引けること");
    assert_eq!(after.base_persona, persona(NEW_PERSONA));
    assert_eq!(
        after.display_name,
        display(NEW_DISPLAY),
        "指定しなかった表示名は元のままであること"
    );
    assert_eq!(
        after.output_format_instruction,
        Some(output_format(FORMAT)),
        "指定しなかった形式指示は元のままであること"
    );

    // 形式指示だけ変える。
    store
        .update_character(
            &name(HANDLE),
            &CharacterUpdate {
                output_format_instruction: FieldUpdate::Set(output_format(NEW_FORMAT)),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("更新できること");
    let after = store
        .find_character_by_handle(&name(HANDLE))
        .await
        .expect("検索が成功すること")
        .expect("引けること");
    assert_eq!(
        after.output_format_instruction,
        Some(output_format(NEW_FORMAT))
    );
    assert_eq!(
        after.display_name,
        display(NEW_DISPLAY),
        "指定しなかった表示名は元のままであること"
    );
    assert_eq!(
        after.base_persona,
        persona(NEW_PERSONA),
        "指定しなかったペルソナは元のままであること"
    );

    // 何も指定しない更新は何も変えない（既定値で潰さない）。
    store
        .update_character(&name(HANDLE), &CharacterUpdate::default())
        .await
        .expect("空の更新も成功すること");
    let untouched = store
        .find_character_by_handle(&name(HANDLE))
        .await
        .expect("検索が成功すること")
        .expect("引けること");
    assert_eq!(untouched, after, "空の更新は1列も動かさないこと");
}

/// **形式指示の「変えない」と「消す」は別の意味**（3値であること）。
///
/// `Option` の列なので、2値（`Option<OutputFormat>`）で表すと
/// 「触れない」と「NULL にする」が同じ値になり、**部分更新のたびに黙って消える。**
#[tokio::test]
async fn keeping_the_output_format_is_not_the_same_as_clearing_it() {
    let (_dir, store) = setup().await;
    create_fixture(&store).await;

    // 「変えない」— 他の列を変える更新でも形式指示は残る。
    store
        .update_character(&name(HANDLE), &update_display(NEW_DISPLAY))
        .await
        .expect("更新できること");
    assert_eq!(
        store
            .find_character_by_handle(&name(HANDLE))
            .await
            .expect("検索が成功すること")
            .expect("引けること")
            .output_format_instruction,
        Some(output_format(FORMAT)),
        "「変えない」で消えないこと"
    );

    // 「消す」— NULL になる。
    store
        .update_character(
            &name(HANDLE),
            &CharacterUpdate {
                output_format_instruction: FieldUpdate::Clear,
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("更新できること");
    assert_eq!(
        store
            .find_character_by_handle(&name(HANDLE))
            .await
            .expect("検索が成功すること")
            .expect("引けること")
            .output_format_instruction,
        None,
        "「消す」で未設定になること"
    );
    let stored: Option<String> =
        sqlx::query_scalar("SELECT output_format_instruction FROM character")
            .fetch_one(store.pool())
            .await
            .expect("格納値を取得できること");
    assert_eq!(stored, None, "空文字ではなく NULL になること");

    // 未設定の行に対する「変えない」は未設定のまま。「設定する」で入る。
    store
        .update_character(&name(HANDLE), &update_display(DISPLAY))
        .await
        .expect("更新できること");
    assert_eq!(
        store
            .find_character_by_handle(&name(HANDLE))
            .await
            .expect("検索が成功すること")
            .expect("引けること")
            .output_format_instruction,
        None,
        "未設定の行が「変えない」で復活しないこと"
    );
    store
        .update_character(
            &name(HANDLE),
            &CharacterUpdate {
                output_format_instruction: FieldUpdate::Set(output_format(NEW_FORMAT)),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("更新できること");
    assert_eq!(
        store
            .find_character_by_handle(&name(HANDLE))
            .await
            .expect("検索が成功すること")
            .expect("引けること")
            .output_format_instruction,
        Some(output_format(NEW_FORMAT))
    );
}

/// **更新は識別子を動かさない**（ADR-0016 §1）。
///
/// 識別子が変わると、FE 側に保存された `model` が指さなくなる。
/// 型では「渡せないこと」しか表現できないので、**書かれないこと**はここで固定する。
///
/// # 返る id が最初の行（1）にならないようにしてある
///
/// **囮のキャラクターを先に作る。** 対象が id 1 のままだと、
/// 更新した行ではなく**決め打ちの 1 を返す実装と区別が付かない**
/// （変異検査で実測。囮を足す前は全テストが緑のまま通った）。
/// 返り値は続けて `replace_stat_allocs` に渡されうるので、
/// 取り違えると**別キャラクターの性格ステータスを書き換える。**
#[tokio::test]
async fn an_update_never_changes_the_identifier() {
    let (_dir, store) = setup().await;
    let decoy = store
        .create_character(
            &name("rin"),
            &display("凛"),
            &persona("別のペルソナ"),
            None,
            fixture_scale(),
        )
        .await
        .expect("囮のキャラクターを作成できること");
    let id = create_fixture(&store).await;
    assert_ne!(id, decoy, "対象は最初の行ではないこと");

    let updated = store
        .update_character(
            &name(HANDLE),
            &CharacterUpdate {
                display_name: Some(display(NEW_DISPLAY)),
                base_persona: Some(persona(NEW_PERSONA)),
                output_format_instruction: FieldUpdate::Set(output_format(NEW_FORMAT)),
                // **尺度は触らない。** この場面で見たいのは他の列なので、
                // 「省略した列は変わらない」側に倒しておく。
                stat_scale: None,
                // 走らせ方も同じ（作業単位 A-8）。
                stat_drift_mode: FieldUpdate::Keep,
                axis_discovery_slots: None,
                archived: None,
            },
        )
        .await
        .expect("更新できること");

    assert_eq!(
        updated, id,
        "更新した行の id が返ること（決め打ちでないこと）"
    );
    assert_eq!(
        stored_handles(&store).await,
        vec!["rin".to_owned(), HANDLE.to_owned()],
        "格納されている識別子が1文字も変わらないこと"
    );
    let found = store
        .find_character_by_handle(&name(HANDLE))
        .await
        .expect("検索が成功すること")
        .expect("更新後も同じ識別子で引けること");
    assert_eq!(found.id, id);
    assert_eq!(found.handle, name(HANDLE));
    assert_eq!(found.display_name, display(NEW_DISPLAY));
    assert_eq!(character_count(&store).await, 2, "行が増えていないこと");

    // 囮は巻き添えになっていない。
    let untouched = store
        .find_character_by_handle(&name("rin"))
        .await
        .expect("検索が成功すること")
        .expect("引けること");
    assert_eq!(untouched.id, decoy);
    assert_eq!(untouched.display_name, display("凛"));
    assert_eq!(untouched.base_persona, persona("別のペルソナ"));
}

/// 存在しないキャラクターの更新はエラー。**行を作らない。**
#[tokio::test]
async fn updating_an_unknown_character_is_an_error() {
    let (_dir, store) = setup().await;
    create_fixture(&store).await;

    let err = store
        .update_character(&name("rin"), &update_display(NEW_DISPLAY))
        .await
        .expect_err("存在しないキャラクターは更新できないこと");
    assert!(
        matches!(err, StoreError::UnknownCharacter),
        "対象が無いことを区別できるエラーになること: {err:?}"
    );
    assert!(
        !err.to_string().contains("rin"),
        "Display は定数文字列で、名前を含めないこと: {err}"
    );

    assert_eq!(character_count(&store).await, 1, "行が増えないこと");
    let untouched = store
        .find_character_by_handle(&name(HANDLE))
        .await
        .expect("検索が成功すること")
        .expect("引けること");
    assert_eq!(
        untouched.display_name,
        display(DISPLAY),
        "別のキャラクターを巻き込まないこと"
    );
}

/// **会話のあるキャラクターは削除できない**（ADR-0016 §2）。
///
/// 「エラーが返る」だけでは足りない。**行数が変化しないこと**まで見る
/// （`session` は `ON DELETE CASCADE` なので、消えたら会話ごと黙って消える）。
#[tokio::test]
async fn a_character_with_a_conversation_cannot_be_deleted() {
    let (_dir, store) = setup().await;
    let id = create_fixture(&store).await;
    store
        .create_session(id)
        .await
        .expect("セッションを作成できること");

    let err = store
        .delete_character(&name(HANDLE))
        .await
        .expect_err("会話のあるキャラクターは消せないこと");
    assert!(
        matches!(err, StoreError::CharacterHasSessions),
        "「該当なし」と区別できるエラーになること: {err:?}"
    );
    assert!(
        !err.to_string().contains(HANDLE) && !err.to_string().contains("session"),
        "Display は定数文字列で、内部情報を含めないこと: {err}"
    );

    assert_eq!(
        character_count(&store).await,
        1,
        "character の行数が変化しないこと"
    );
    assert_eq!(session_count(&store).await, 1, "会話も残っていること");
    assert!(
        store
            .find_character_by_handle(&name(HANDLE))
            .await
            .expect("検索が成功すること")
            .is_some(),
        "拒否されたキャラクターはそのまま引けること"
    );
}

/// 会話の無いキャラクターは削除できる（作り損ないの掃除）。
#[tokio::test]
async fn a_character_without_a_conversation_can_be_deleted() {
    let (_dir, store) = setup().await;
    let id = create_fixture(&store).await;
    store
        .replace_stat_allocs(id, &[alloc("パッション", 3)])
        .await
        .expect("性格値を設定できること");

    // 巻き添えを見るために、会話のある別キャラクターを1件置いておく。
    let rin = store
        .create_character(
            &name("rin"),
            &display("凛"),
            &persona("別のペルソナ"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");
    store
        .create_session(rin)
        .await
        .expect("セッションを作成できること");

    store
        .delete_character(&name(HANDLE))
        .await
        .expect("会話の無いキャラクターは消せること");

    assert_eq!(
        stored_handles(&store).await,
        vec!["rin".to_owned()],
        "対象の行だけが消えること"
    );
    assert!(
        store
            .find_character_by_handle(&name(HANDLE))
            .await
            .expect("検索が成功すること")
            .is_none(),
        "消えたキャラクターは引けないこと"
    );
    assert!(
        store
            .stat_allocs(id)
            .await
            .expect("取得自体は成功すること")
            .is_empty(),
        "性格値も残らないこと（ON DELETE CASCADE）"
    );
    assert_eq!(session_count(&store).await, 1, "他キャラの会話は残ること");
}

/// 存在しないキャラクターの削除は「会話がある」と区別できるエラー。
#[tokio::test]
async fn deleting_an_unknown_character_is_an_error() {
    let (_dir, store) = setup().await;
    create_fixture(&store).await;

    let err = store
        .delete_character(&name("rin"))
        .await
        .expect_err("存在しないキャラクターは消せないこと");
    assert!(
        matches!(err, StoreError::UnknownCharacter),
        "「会話があるので消せない」と区別できること: {err:?}"
    );
    assert_ne!(
        StoreError::UnknownCharacter.to_string(),
        StoreError::CharacterHasSessions.to_string(),
        "文言でも区別できること"
    );
    assert_eq!(character_count(&store).await, 1, "他の行を消さないこと");
}

/// **更新も削除も、綴りが完全一致する行にしか届かない**
/// 〈0004b-2 の要。`UPDATE_CHARACTER` / `DELETE_CHARACTER` / `SELECT_ID_BY_NAME` の見張り番〉。
///
/// # なぜこれが要るのか（実測が根拠）
///
/// 0012a では引き当てが**正規化キー**（`WHERE trim(name, ...) = ?`）だった。
/// 「一致する行は高々1件」が成り立っていたのは `character_name_key_idx` が
/// 併存を防いでいたからで、**0004b-2 でその索引を落とした以上、`trim` は残せない。**
/// 索引だけを落として `trim` を残すとこうなる（実測 2026-08-16 / sqlite 3.53.4）:
///
/// ```text
/// rows: 1|'aoi'（本物） 2|' aoi '（ゴースト）
/// update_character("aoi") → Ok(CharacterId(2))   ← **返るのはゴーストの id**
/// rows after update = [(1,'aoi','巻き添え'), (2,' aoi ','巻き添え')]
/// delete_character("aoi") → Ok(())
/// rows after delete = []                          ← **本物ごと消えた**
/// ```
///
/// **本物のキャラクターが黙って書き換わり、`ON DELETE CASCADE` で会話ごと消える。**
/// 索引が防いでいたどの事故よりも重い。ここが赤くなったら、その状態に戻っている。
///
/// # 何を並べるか
///
/// 隣に置くのは「**引き当てが緩めば拾ってしまう**」綴り。
/// **`TRIMMABLE` の8文字**（0012a の `trim` が拾っていた集合）と、
/// **`TRIMMABLE` の外の空白めいた文字**（`trim` を広げると拾う集合）の**両方**を回す。
/// 前者を落とすと「0012a の実装へ戻す変異」が緑を通る。
///
/// # 対象が消えたあとも見る
///
/// 隣の行だけが残った状態で削除を頼むと、正しくは `UnknownCharacter`。
/// 引き当てが緩いと隣の行を見つけてしまい、会話が1件も無いのに
/// **`CharacterHasSessions`（会話があるので消せない）という嘘**になる。
/// **返る id では観測できない失敗モードがここで捕まる**（CLAUDE.md
/// 「『観測できない』は『その観測手段では観測できない』でしかない」）。
#[tokio::test]
async fn an_update_or_delete_only_ever_reaches_the_row_whose_handle_matches_exactly() {
    let paddings: Vec<char> = personacore::domain::TRIMMABLE
        .into_iter()
        .chain(PADDINGS_OUTSIDE_TRIMMABLE)
        .collect();
    assert_eq!(
        paddings.len(),
        personacore::domain::TRIMMABLE.len() + PADDINGS_OUTSIDE_TRIMMABLE.len(),
        "前提: 両方の集合を回していること"
    );

    for padding in paddings {
        let (_dir, store) = setup().await;
        let target = create_fixture(&store).await;

        // **`handle` の完全一致 UNIQUE には触れないので併存できる行。**
        let neighbour = format!("{padding}{HANDLE}{padding}");
        assert_ne!(neighbour, HANDLE, "前提: 綴りが違うこと");
        // **0004b-1 以降は CHECK を無効にしないと作れない**（0005 / ADR-0017）。
        let neighbour_id =
            insert_ghost_character_with(&store, &neighbour, NEIGHBOUR_DISPLAY, NEIGHBOUR_PERSONA)
                .await;

        let updated = store
            .update_character(&name(HANDLE), &update_display(NEW_DISPLAY))
            .await
            .expect("完全一致する行は更新できること");
        assert_eq!(
            updated, target,
            "U+{:04X}: 更新した行の id が返ること（隣の行の id を返していないこと）",
            padding as u32
        );
        assert_ne!(
            updated.get(),
            neighbour_id,
            "U+{:04X}: 返っているのは隣の行の id",
            padding as u32
        );
        assert_eq!(
            display_name_of(&store, HANDLE).await,
            NEW_DISPLAY,
            "対象の行は更新されること"
        );
        assert_eq!(
            display_name_of(&store, &neighbour).await,
            NEIGHBOUR_DISPLAY,
            "U+{:04X} で埋まった別の行を巻き込んで書き換えている \
             （引き当てが完全一致より広い）",
            padding as u32
        );

        store
            .delete_character(&name(HANDLE))
            .await
            .expect("完全一致する行は削除できること");
        assert_eq!(
            stored_handles(&store).await,
            vec![neighbour.clone()],
            "U+{:04X} で埋まった別の行を巻き込んで消している \
             （引き当てが完全一致より広い）",
            padding as u32
        );

        // **隣の行だけが残った状態。** ここから先は返り値の**種別**で見る。
        let err = store
            .update_character(&name(HANDLE), &update_display(NEIGHBOUR_DISPLAY))
            .await
            .expect_err("完全一致する行が無いので更新は対象なしになること");
        assert!(
            matches!(err, StoreError::UnknownCharacter),
            "U+{:04X} で埋まった行を更新の対象にしている: {err:?}",
            padding as u32
        );

        let err = store
            .delete_character(&name(HANDLE))
            .await
            .expect_err("完全一致する行が無いので削除は対象なしになること");
        assert!(
            matches!(err, StoreError::UnknownCharacter),
            "U+{:04X} で埋まった行を引き当てている。\
             会話は1件も無いのに `CharacterHasSessions` を返すなら、\
             理由の引き直し（SELECT_ID_BY_NAME）が完全一致より広い: {err:?}",
            padding as u32
        );
        assert_eq!(
            display_name_of(&store, &neighbour).await,
            NEIGHBOUR_DISPLAY,
            "隣の行は最後まで無傷であること"
        );
    }
}

/// **1件を指す読み出しも、綴りが完全一致する行にしか当たらない**
/// 〈`SELECT_BY_NAME` の見張り番 / 0004b-2 の追補〉。
///
/// # なぜ別に要るのか
///
/// 直前の `an_update_or_delete_only_ever_reaches_the_row_whose_handle_matches_exactly` は
/// **更新・削除の引き当てしか見ていない。** 読み出し（`SELECT_BY_NAME`）には
/// 見張り番が1本も無く、`WHERE name = ?` を `WHERE name LIKE ?` に
/// **1トークン変えるだけでスイート全体が緑のまま通っていた**（0004b-2 のレビューで実測）。
///
/// # 無害な変異ではない
///
/// * **`_` は `Handle::ALLOWED_CHARACTERS` に含まれる。**
///   正当な識別子（`a_i`）がそのまま LIKE のワイルドカードになる。
/// * **SQLite の LIKE は ASCII について大文字小文字を区別しない**ので、
///   `"AOI"` のような（`GLOB` の CHECK を迂回して入った）行にも当たる。
///
/// ```text
/// create_character("aoi") → CharacterId(1) display="葵"
/// create_character("a_i") → CharacterId(2) display="藍"
/// find_character_by_handle("a_i") → id=CharacterId(1) display="葵"   ← 別キャラが返る
/// ```
///
/// **`model: "a_i"` で会話すると別キャラのペルソナ・記憶・セッションに繋がり、
/// エラーは出ない。** ADR-0004 が「最も危険」とした「照合が別の行に当たる」がそのまま戻る。
///
/// # `%` は回さなくてよい
///
/// LIKE のもう1つのワイルドカード `%` は `ALLOWED_CHARACTERS` の外なので、
/// **`Handle` として作れない**（この関数は `&Handle` しか受け取らない）。
/// **格納側に `%` があっても効かない**——`name LIKE ?` のパターンは右辺、
/// すなわち**バインドした引数**であって格納値ではないため。
/// 照合式が広がる変異は、`_` と大文字小文字の2つ（`WIDENING_ROWS`）で捕まる。
///
/// # 何で観測するか（`handle` では観測できない）
///
/// `find_character_by_handle` は引数の `Handle` をそのまま `Character::name` に載せる
/// （格納値を読まない）。**別の行に当たっても `handle` は要求した綴りのまま**になるので、
/// 見るのは **id と表示名**。行ごとに違う表示名にしてある（`WIDENING_ROWS`）。
///
/// 並べる行とその順序の意図は `WIDENING_ROWS` の doc にある。
#[tokio::test]
async fn a_lookup_only_ever_reaches_the_row_whose_handle_matches_exactly() {
    // (照会する識別子, 期待する行の格納名。`None` は「1件も当たらない」)
    const LOOKUPS: [(&str, Option<&str>); 7] = [
        ("aoi", Some("aoi")), // 大小を無視すると "AOI" に当たる
        ("a_i", Some("a_i")), // `_` が広がると "aoi" に当たる
        ("a-i", Some("a-i")), // ワイルドカードを含まない対照
        ("xyz", Some("xyz")), // 同上
        ("x_z", None),        // `_` が広がると "xyz" に当たる
        ("kaede", None),      // 大小を無視すると "KAEDE" に当たる
        ("rin", None),        // そもそも1行も無い
    ];

    let (_dir, store) = setup().await;
    let inserted = insert_widening_rows(&store).await;

    for (queried, expected) in LOOKUPS {
        let found = store
            .find_character_by_handle(&name(queried))
            .await
            .expect("検索が成功すること");

        let Some(expected) = expected else {
            assert!(
                found.is_none(),
                "{queried:?}: 完全一致する行は1件も無いのに引き当てている\
                 （照合が完全一致より広い）: id = {:?} 表示名 = {:?}",
                found.as_ref().map(|character| character.id.get()),
                found
                    .as_ref()
                    .map(|character| character.display_name.as_str()),
            );
            continue;
        };

        let found = found.unwrap_or_else(|| panic!("{queried:?}: 完全一致する行は引けること"));
        let (_, expected_display, expected_id) = inserted
            .iter()
            .find(|(stored, _, _)| *stored == expected)
            .copied()
            .expect("期待する行は挿入してあること");
        assert_eq!(
            (found.id.get(), found.display_name.as_str()),
            (expected_id, expected_display),
            "{queried:?}: 別のキャラクターの行を引き当てている（照合が完全一致より広い）"
        );
    }
}

/// **更新も削除も、ワイルドカードと ASCII の大小には広がらない**
/// 〈`UPDATE_CHARACTER` / `DELETE_CHARACTER` / `SELECT_ID_BY_NAME` の見張り番 / 0004b-2 の追補〉。
///
/// # `an_update_or_delete_only_ever_reaches_the_row_whose_handle_matches_exactly` では足りない
///
/// あちらが隣に置くのは**空白めいた文字で埋めた行**だけなので、
/// **`trim` の復活しか捕まえられない。** `WHERE name = ?5` を `WHERE name LIKE ?5` に
/// 変える変異は、**あちらも含めてスイート全体が緑のまま通った**（この追補で実測）。
/// 更新の側は特にたちが悪く、
///
/// * `UPDATE ... WHERE name LIKE 'aoi'` は `"AOI"` の行も**同じ値で塗り潰す**が、
/// * 返る id は `SELECT_ID_BY_NAME`（完全一致）から取るので**正しい id が返り**、
/// * 巻き添えになった行は誰も読まない
///
/// ——という形で、**成功応答のまま別のキャラクターの表示名・ペルソナが書き換わる。**
///
/// # 何を見るか
///
/// **1行ずつ名指しで引かない**（`stored_rows` で全行を突き合わせる）。
/// 巻き添えは「引く相手を思い付かなかった行」に出るので、名指しでは観測から漏れる。
///
/// 並べる行の意図は `WIDENING_ROWS` の doc にある。
#[tokio::test]
async fn an_update_or_delete_never_widens_into_a_wildcard_or_a_case_variant() {
    let (_dir, store) = setup().await;
    let inserted = insert_widening_rows(&store).await;

    // 期待する現在の姿。書き込みが成功するたびに、この表だけを進める。
    let mut expected: Vec<(String, String)> = inserted
        .iter()
        .map(|(stored, shown, _)| ((*stored).to_owned(), (*shown).to_owned()))
        .collect();
    assert_eq!(
        stored_rows(&store).await,
        expected,
        "前提: 挿入した7行がそのまま並んでいること"
    );

    // --- 更新は完全一致の行だけを書き換える ---
    for (queried, repaired) in [("aoi", "直した葵"), ("a_i", "直した藍")] {
        let updated = store
            .update_character(&name(queried), &update_display(repaired))
            .await
            .unwrap_or_else(|err| panic!("{queried:?}: 完全一致する行は更新できること: {err:?}"));
        let (_, _, id) = inserted
            .iter()
            .find(|(stored, _, _)| *stored == queried)
            .copied()
            .expect("期待する行は挿入してあること");
        assert_eq!(
            updated.get(),
            id,
            "{queried:?}: 更新した行の id が返ること（別の行の id を返していないこと）"
        );

        for row in expected.iter_mut() {
            if row.0 == queried {
                row.1 = repaired.to_owned();
            }
        }
        assert_eq!(
            stored_rows(&store).await,
            expected,
            "{queried:?}: 完全一致しない行まで書き換えている\
             （`_` がワイルドカードに化けているか、ASCII の大小を区別していない）"
        );
    }

    // --- 完全一致する行が無ければ、広げれば届く行があっても届かない ---
    for queried in ["x_z", "kaede"] {
        let err = store
            .update_character(&name(queried), &update_display("届いてはいけない表示名"))
            .await
            .expect_err("完全一致する行が無いので更新は対象なしになること");
        assert!(
            matches!(err, StoreError::UnknownCharacter),
            "{queried:?}: 完全一致しない行を更新の対象にしている: {err:?}"
        );
        assert_eq!(
            stored_rows(&store).await,
            expected,
            "{queried:?}: 拒否したのに書き換えた行がある"
        );

        let err = store
            .delete_character(&name(queried))
            .await
            .expect_err("完全一致する行が無いので削除は対象なしになること");
        assert!(
            matches!(err, StoreError::UnknownCharacter),
            "{queried:?}: 完全一致しない行を引き当てている。\
             会話は1件も無いのに `CharacterHasSessions` を返すなら、\
             理由の引き直し（SELECT_ID_BY_NAME）が完全一致より広い: {err:?}"
        );
        assert_eq!(
            stored_rows(&store).await,
            expected,
            "{queried:?}: 拒否したのに消えた行がある"
        );
    }

    // --- 削除は完全一致の行だけを消す ---
    for queried in ["a_i", "aoi"] {
        store
            .delete_character(&name(queried))
            .await
            .unwrap_or_else(|err| panic!("{queried:?}: 完全一致する行は削除できること: {err:?}"));
        expected.retain(|row| row.0 != queried);
        assert_eq!(
            stored_rows(&store).await,
            expected,
            "{queried:?}: 完全一致しない行まで消している\
             （`_` がワイルドカードに化けているか、ASCII の大小を区別していない）"
        );
    }
}

/// **削除は「数えてから消す」ではない**（判定と削除は1文で不可分）。
///
/// 数えてから消すと、その隙に始まった会話が `ON DELETE CASCADE` で**黙って消える。**
/// 逐次のテストでは差が出ないので、削除と会話開始を同時に走らせて
/// 「**両方成功する状態は存在しない**」ことを見る。
///
/// * 削除が先に確定 → 会話開始は外部キー違反で失敗する。
/// * 会話開始が先に確定 → 削除は `NOT EXISTS` に阻まれて失敗する。
///
/// **正しい実装では偽陽性にならない**（両方成功する経路が無い）。
/// 逆に競合が毎回起きるとも限らないので、回数を減らさないこと。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleting_never_races_a_starting_conversation() {
    let (_dir, store) = setup().await;

    let mut started_conversations = 0_i64;
    for round in 0..64 {
        let handle = format!("aoi-{round}");
        let id = store
            .create_character(
                &name(&handle),
                &display(DISPLAY),
                &persona(PERSONA),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");

        let starting = {
            let store = store.clone();
            tokio::spawn(async move { store.create_session(id).await })
        };
        let deleting = {
            let store = store.clone();
            let target = name(&handle);
            tokio::spawn(async move { store.delete_character(&target).await })
        };

        let started = starting.await.expect("タスクが panic しないこと");
        let deleted = deleting.await.expect("タスクが panic しないこと");

        assert!(
            !(started.is_ok() && deleted.is_ok()),
            "会話が始まったキャラクターを消してはならない（{round} 周目）"
        );
        if started.is_ok() {
            started_conversations += 1;
            assert!(
                store
                    .find_character_by_handle(&name(&handle))
                    .await
                    .expect("検索が成功すること")
                    .is_some(),
                "会話が成立したなら、そのキャラクターは残っていること（{round} 周目）"
            );
        }
    }

    // **成立した会話が1件も失われていないこと。** カスケードで消えるとここが減る。
    assert_eq!(
        session_count(&store).await,
        started_conversations,
        "成立した会話がカスケードで消えていないこと"
    );
}

// ======================================================================
// 壊れた行を指し示す一覧（ADR-0016 §3 / 作業単位 0012b）
// ======================================================================

/// 一覧から**読めた行**だけを取り出す。
fn readable(entries: &[CharacterEntry]) -> Vec<&Character> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            CharacterEntry::Readable(character) => Some(character),
            CharacterEntry::Unreadable(_) => None,
        })
        .collect()
}

/// 一覧から**読めなかった行**だけを取り出す。
fn unreadable(entries: &[CharacterEntry]) -> Vec<&UnreadableCharacter> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            CharacterEntry::Readable(_) => None,
            CharacterEntry::Unreadable(broken) => Some(broken),
        })
        .collect()
}

/// 読めた行を識別子で1件引く（一覧の中から探す）。
fn readable_named<'a>(entries: &'a [CharacterEntry], handle: &str) -> &'a Character {
    readable(entries)
        .into_iter()
        .find(|character| character.handle.as_str() == handle)
        .unwrap_or_else(|| panic!("{handle} が読める行として一覧に出ていること: {entries:?}"))
}

/// **`parse` を通らない行を生 SQL で作る**（0009 以前に作れた行や直接編集を模す）。
///
/// 返すのはその行の id。`store` の公開 API では作れない状態なので、
/// ここを通す以外に「壊れた行」を用意する手段が無い。
///
/// **CHECK 制約も外す**（`insert_character_ignoring_checks` / 0004b-1）。
/// 見本には識別子が規則外の行（`" aoi "`）も含むので、0005 の CHECK が有効なままだと
/// **そもそも見本を用意できない。** 表示名の空文字なども同じ。
async fn insert_row(
    store: &Store,
    stored_name: &str,
    display_name: &str,
    base_persona: &str,
    output_format: Option<&str>,
) -> i64 {
    insert_character_ignoring_checks(
        store,
        stored_name,
        display_name,
        base_persona,
        output_format,
    )
    .await
    .unwrap_or_else(|err| panic!("壊れた行を用意できること: name = {stored_name:?}: {err}"))
}

/// 上限を超える値（`parse` を必ず落とす）。
fn oversized(max_chars: usize) -> String {
    OVERSIZED_FILLER.repeat(max_chars + 1)
}

/// 壊れた行1件の見本（格納名・表示名・ペルソナ・形式指示・**尺度**・読めない列）。
///
/// 尺度は `None` なら触らない（スキーマの既定 10 のまま）。
/// `Some(値)` は **CHECK を外して**書き込む（範囲外の値は普通の SQL では作れない）。
type BrokenCase = (
    String,
    String,
    String,
    Option<String>,
    Option<i64>,
    Vec<CharacterColumn>,
);

/// **壊れた行が1件あっても、正常な行は一覧に出る**〈この作業単位の存在理由 / ADR-0016 §3〉。
///
/// 以前は `?` で丸ごと失敗していたので、0009 以前に作られた行が1件でもあると
/// **正常なキャラクターの一覧すら見られなかった**（修復手段は DB の直接編集だけ）。
///
/// **黙って読み飛ばすのも駄目。** それは「存在するのに見えない」という別の壊れ方で、
/// 直すべき相手が分からなくなる。
#[tokio::test]
async fn a_broken_row_does_not_take_the_healthy_ones_with_it() {
    let (_dir, store) = setup().await;
    assert_fixture_values_are_distinct();

    // 正常な行を2件。**壊れた行の前後の名前**にしてある（並びの巻き添えも見る）。
    store
        .create_character(
            &name(HANDLE),
            &display(DISPLAY),
            &persona(PERSONA),
            Some(&output_format(FORMAT)),
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");
    store
        .create_character(
            &name(HEALTHY_HANDLE),
            &display(HEALTHY_DISPLAY),
            &persona(HEALTHY_PERSONA),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    // 表示名だけが壊れた行（名前は正常なので、除外すると ADR-0004 の袋小路が復活する）。
    let broken_id = insert_row(
        &store,
        BROKEN_HANDLE,
        &oversized(DisplayName::MAX_CHARS),
        PERSONA,
        None,
    )
    .await;

    let entries = store
        .list_characters()
        .await
        .expect("壊れた行があっても一覧そのものは成功すること");
    assert_eq!(entries.len(), 3, "壊れた行も一覧に現れること: {entries:?}");

    let names: Vec<&str> = readable(&entries)
        .iter()
        .map(|character| character.handle.as_str())
        .collect();
    assert_eq!(
        names,
        vec![HANDLE, HEALTHY_HANDLE],
        "正常な行が巻き添えにならないこと: {entries:?}"
    );

    let broken = unreadable(&entries);
    assert_eq!(
        broken.len(),
        1,
        "壊れた行を黙って読み飛ばさないこと: {entries:?}"
    );
    assert_eq!(
        broken[0].id,
        CharacterId::from_raw_for_tests(broken_id),
        "壊れた行を指せる id を返すこと（決め打ちや隣の行の id ではないこと）"
    );
    assert_eq!(
        broken[0].handle.as_ref(),
        Some(&name(BROKEN_HANDLE)),
        "修復 API に渡せる識別子を返すこと"
    );
    assert_eq!(
        broken[0].invalid_columns,
        vec![CharacterColumn::DisplayName],
        "どの列が読めなかったかを返すこと"
    );

    // **生の格納値は返さない。** 上限超過の値は数千文字ありうるうえ、
    // 何が入っているか分からない（`InvalidStoredValue` の `Display` と同じ判断）。
    let shown = format!("{entries:?}");
    assert!(
        !shown.contains(OVERSIZED_FILLER),
        "壊れた列の格納値をそのまま返さないこと: {shown}"
    );
}

/// **どの列が壊れていても拾う。壊れ方が複数なら全部挙げる。**
///
/// 1種類（例: 表示名）しか見ない実装は、残りの壊れ方を**黙って読み飛ばす**か
/// 一覧を丸ごと落とすかのどちらかになる。`parse` を通る列は全部ここに並べること。
#[tokio::test]
async fn every_kind_of_broken_column_is_pointed_at() {
    let (_dir, store) = setup().await;
    assert_fixture_values_are_distinct();

    // 正常な行を1件（囮も兼ねる。壊れた行の id が 1 にならない）。
    store
        .create_character(
            &name(HANDLE),
            &display(DISPLAY),
            &persona(PERSONA),
            Some(&output_format(FORMAT)),
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    // (格納名, 表示名, ペルソナ, 形式指示, 尺度, 読めない列)
    let cases: Vec<BrokenCase> = vec![
        // 識別子が上限超過（`Handle` を作れない = どの API からも指せない）。
        (
            "名".repeat(Handle::MAX_CHARS + 1),
            HEALTHY_DISPLAY.to_owned(),
            HEALTHY_PERSONA.to_owned(),
            None,
            None,
            vec![CharacterColumn::Handle],
        ),
        // ゴースト行（`parse` は通るが、格納値が正規化されていない）。
        (
            format!(" {BROKEN_HANDLE}-ghost "),
            HEALTHY_DISPLAY.to_owned(),
            HEALTHY_PERSONA.to_owned(),
            None,
            None,
            vec![CharacterColumn::Handle],
        ),
        // 表示名（0009 以前は U+2028 入りが `/admin/` から作れた）。
        (
            format!("{BROKEN_HANDLE}-display"),
            "葵\u{2028}碧".to_owned(),
            HEALTHY_PERSONA.to_owned(),
            None,
            None,
            vec![CharacterColumn::DisplayName],
        ),
        // ペルソナ（上限導入前の巨大な値）。
        (
            format!("{BROKEN_HANDLE}-persona"),
            HEALTHY_DISPLAY.to_owned(),
            oversized(Persona::MAX_CHARS),
            None,
            None,
            vec![CharacterColumn::BasePersona],
        ),
        // 形式指示（上限導入前の巨大な値。NULL は「未設定」であって壊れていない）。
        (
            format!("{BROKEN_HANDLE}-format"),
            HEALTHY_DISPLAY.to_owned(),
            HEALTHY_PERSONA.to_owned(),
            Some(oversized(OutputFormat::MAX_CHARS)),
            None,
            vec![CharacterColumn::OutputFormatInstruction],
        ),
        // **尺度が下限より小さい**（作業単位 0011a-1）。
        // 尺度 0 は「全ての値が 0」を意味し、絶対制約（値 0 の軸は落とす）が全軸に効いて
        // **一行も載らないキャラクター**になる。スキーマの CHECK も弾くが、
        // CHECK は外せるので読み出し側でも `parse` を通している。
        (
            format!("{BROKEN_HANDLE}-scale-low"),
            HEALTHY_DISPLAY.to_owned(),
            HEALTHY_PERSONA.to_owned(),
            None,
            Some(0),
            vec![CharacterColumn::StatScale],
        ),
        // **尺度が上限より大きい。**
        (
            format!("{BROKEN_HANDLE}-scale-high"),
            HEALTHY_DISPLAY.to_owned(),
            HEALTHY_PERSONA.to_owned(),
            None,
            Some(i64::from(u32::MAX) + 1),
            vec![CharacterColumn::StatScale],
        ),
        // **2箇所壊れている行。** 片方で打ち切る実装をここで落とす。
        (
            format!("{BROKEN_HANDLE}-both"),
            oversized(DisplayName::MAX_CHARS),
            oversized(Persona::MAX_CHARS),
            None,
            None,
            vec![CharacterColumn::DisplayName, CharacterColumn::BasePersona],
        ),
        // **表示名と尺度が同時に壊れている行。** 列挙の順序（宣言順）も固定する。
        (
            format!("{BROKEN_HANDLE}-display-and-scale"),
            "葵\u{2028}碧".to_owned(),
            HEALTHY_PERSONA.to_owned(),
            None,
            Some(-1),
            vec![CharacterColumn::DisplayName, CharacterColumn::StatScale],
        ),
    ];

    let mut expected: Vec<(i64, Vec<CharacterColumn>)> = Vec::new();
    for (stored_name, display_name, base_persona, format, stat_scale, columns) in &cases {
        let id = insert_row(
            &store,
            stored_name,
            display_name,
            base_persona,
            format.as_deref(),
        )
        .await;
        if let Some(stat_scale) = *stat_scale {
            set_stat_scale_ignoring_checks(&store, id, stat_scale).await;
        }
        expected.push((id, columns.clone()));
    }
    expected.sort_by_key(|(id, _)| *id);

    let entries = store
        .list_characters()
        .await
        .expect("壊れた行があっても一覧そのものは成功すること");

    let names: Vec<&str> = readable(&entries)
        .iter()
        .map(|character| character.handle.as_str())
        .collect();
    assert_eq!(
        names,
        vec![HANDLE],
        "壊れていない行だけが読める側に来ること: {entries:?}"
    );

    let mut actual: Vec<(i64, Vec<CharacterColumn>)> = unreadable(&entries)
        .iter()
        .map(|broken| (broken.id.get(), broken.invalid_columns.clone()))
        .collect();
    actual.sort_by_key(|(id, _)| *id);
    assert_eq!(
        actual, expected,
        "壊れた列の種類と、それを指す id が一致すること: {entries:?}"
    );

    // 識別子そのものが壊れている行は、修復 API に渡せる名前が作れない
    // （**id が唯一の手掛かり**）。
    //
    // > **0004b-1 でここが1件から2件になった**（ADR-0017）。
    // > `Handle::parse` が正規化をやめたので、ゴースト行（`" rin-ghost "`）も
    // > `parse` を通らなくなり、**一覧は正規化した名前を返せなくなった。**
    // > 掃除の導線は「id で見つけて、正規化した綴りを自分で打つ」に変わっている
    // > （`a_ghost_row_the_listing_points_at_can_be_removed_by_typing_the_handle` を参照）。
    // > **実害はほぼ無い**——ゴースト行は 0005 の CHECK で新たに作れず、
    // > 既存の行を抱えた DB は移行そのものが止まるため。
    let handles: Vec<Option<String>> = unreadable(&entries)
        .iter()
        .map(|broken| {
            broken
                .handle
                .as_ref()
                .map(|handle| handle.as_str().to_owned())
        })
        .collect();
    assert_eq!(
        handles.iter().filter(|handle| handle.is_none()).count(),
        2,
        "識別子が `parse` を通らない行（上限超過とゴースト行）だけが `None` になること: {handles:?}"
    );
    // **他の列が壊れているだけの行には、必ず識別子が付く**（そこが修復の導線）。
    for suffix in ["display", "persona", "format", "both"] {
        assert!(
            handles.contains(&Some(format!("{BROKEN_HANDLE}-{suffix}"))),
            "識別子が正当な行は名前で指せること: {suffix} / {handles:?}"
        );
    }
}

/// **一覧が返した情報だけで、壊れた行を更新 API で直せる**
/// 〈この作業単位が目的を達しているかの見張り番〉。
///
/// 一覧（0012b）と更新（0012a）が1本の線で繋がっていることを見る。
/// ここが赤くなったら、壊れた行を「指し示した」だけで**修復には届いていない。**
#[tokio::test]
async fn a_broken_row_can_be_repaired_with_what_the_listing_returns() {
    let (_dir, store) = setup().await;
    assert_fixture_values_are_distinct();

    // 隣の行（巻き添えを見る）。**先に作るので id は壊れた行より小さい。**
    store
        .create_character(
            &name(HEALTHY_HANDLE),
            &display(HEALTHY_DISPLAY),
            &persona(HEALTHY_PERSONA),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    // 表示名とペルソナの両方が壊れた行。
    let broken_id = insert_row(
        &store,
        BROKEN_HANDLE,
        &oversized(DisplayName::MAX_CHARS),
        &oversized(Persona::MAX_CHARS),
        None,
    )
    .await;

    // 前提: この行は `find_character_by_handle` では読めない（= 一覧が唯一の手掛かり）。
    assert!(
        matches!(
            store.find_character_by_handle(&name(BROKEN_HANDLE)).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "前提: 壊れた行は読み出せないこと"
    );

    let entries = store.list_characters().await.expect("一覧を取得できること");
    let broken = unreadable(&entries);
    assert_eq!(broken.len(), 1, "壊れた行が指し示されること: {entries:?}");
    let handle = broken[0]
        .handle
        .clone()
        .expect("修復に使える識別子が付いていること");

    // **一覧が返した識別子だけを使って直す。**
    let repaired_id = store
        .update_character(
            &handle,
            &CharacterUpdate {
                display_name: Some(display(REPAIRED_DISPLAY)),
                base_persona: Some(persona(NEW_PERSONA)),
                output_format_instruction: FieldUpdate::Keep,
                // **尺度は触らない。** この場面で見たいのは他の列なので、
                // 「省略した列は変わらない」側に倒しておく。
                stat_scale: None,
                // 走らせ方も同じ（作業単位 A-8）。
                stat_drift_mode: FieldUpdate::Keep,
                axis_discovery_slots: None,
                archived: None,
            },
        )
        .await
        .expect("一覧が返した識別子で更新できること");
    assert_eq!(
        repaired_id, broken[0].id,
        "一覧が指した行と、実際に直った行が同じであること"
    );
    assert_eq!(
        repaired_id,
        CharacterId::from_raw_for_tests(broken_id),
        "直したのは壊れていた行であること"
    );

    let entries = store.list_characters().await.expect("一覧を取得できること");
    assert!(
        unreadable(&entries).is_empty(),
        "修復後は指し示す行が残らないこと: {entries:?}"
    );
    let repaired = readable_named(&entries, BROKEN_HANDLE);
    assert_eq!(repaired.display_name, display(REPAIRED_DISPLAY));
    assert_eq!(repaired.base_persona, persona(NEW_PERSONA));

    let neighbour = readable_named(&entries, HEALTHY_HANDLE);
    assert_eq!(
        neighbour.display_name,
        display(HEALTHY_DISPLAY),
        "隣の行が巻き添えになっていないこと"
    );
    assert_eq!(neighbour.base_persona, persona(HEALTHY_PERSONA));
}

/// **識別子が読めない行は、指し示せるが API からは触れない**
/// 〈0004b-2 で失ったものを、実行可能な形で書き留める〉。
///
/// # このテストは何度か意味が変わっている。**その履歴が要点**
///
/// 1. 0012b: `a_ghost_row_can_be_removed_with_what_the_listing_returns`。
///    一覧が返した `name`（当時は**正規化された値**）を `delete_character` へ渡していた。
/// 2. 0004b-1: `Handle::parse` が正規化をやめたので**一覧は識別子を返せなくなった**
///    （`name: None`）。名前を `..._by_typing_the_handle` に変え、
///    綴りはテスト側が打つ形にした（`DELETE_CHARACTER` の `trim` が拾っていた）。
/// 3. **0004b-2: その `trim` を捨てた。** 打った綴りでも届かない。
///
/// # なぜ届かなくてよいと判断したか
///
/// * **2 の時点で導線は既に切れていた。** 一覧が `handle: null` を返す以上、
///   運用者は掃除に打つべき綴りを API からは知りようがない。当てられたのはテストだけ。
/// * `trim` が拾えたのは `TRIMMABLE` の8文字で埋まった行だけで、
///   `"Aoi"` や `"葵"` の行には最初から届かない。**覆いが元々ごく一部だった。**
/// * `trim` を残すと、索引を落とした後は**同じ綴りの本物のキャラクターごと消す**
///   （`an_update_or_delete_only_ever_reaches_the_row_whose_handle_matches_exactly` の doc）。
///
/// **ゴースト行を作れるのは DB を直接編集した者だけで、同じ `sqlite3` で消せる。**
///
/// # ここで固定するもの
///
/// * 一覧が**その行を id で指し示す**こと（起点はここにしかない）。**黙って消さない。**
/// * 更新も削除も **`UnknownCharacter`** になること（成功したと嘘をつかない）。
/// * **隣の正常な行が最後まで無傷**であること。
/// * ゴースト行があっても**同じ識別子を作り直せる**こと（袋小路にならない）。
#[tokio::test]
async fn a_row_whose_handle_cannot_be_parsed_is_pointed_at_but_cannot_be_reached() {
    let (_dir, store) = setup().await;
    assert_fixture_values_are_distinct();

    // 隣の正常な行（巻き添えを見る）。
    store
        .create_character(
            &name(HEALTHY_HANDLE),
            &display(HEALTHY_DISPLAY),
            &persona(HEALTHY_PERSONA),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    // **ゴースト行。** 識別子以外は正常な値にしておく（壊れている列を1つに絞る）。
    let ghost = format!(" {BROKEN_HANDLE} ");
    let ghost_id = insert_row(&store, &ghost, DISPLAY, PERSONA, None).await;

    let entries = store.list_characters().await.expect("一覧を取得できること");
    let broken = unreadable(&entries);
    assert_eq!(broken.len(), 1, "ゴースト行が指し示されること: {entries:?}");
    assert_eq!(
        broken[0].id,
        CharacterId::from_raw_for_tests(ghost_id),
        "指しているのはゴースト行であること（隣の行の id ではないこと）"
    );
    assert_eq!(
        broken[0].invalid_columns,
        vec![CharacterColumn::Handle],
        "壊れているのは識別子の列だと分かること: {entries:?}"
    );
    // **一覧は識別子を返せない**（0004b-1 / ADR-0017。上の doc を参照）。
    assert_eq!(
        broken[0].handle, None,
        "正規化しない以上、ゴースト行の識別子は `parse` を通らないこと: {entries:?}"
    );

    // **正規化した綴りを打っても届かない**（0004b-2）。
    let handle = name(BROKEN_HANDLE);
    let err = store
        .update_character(&handle, &update_display(NEW_DISPLAY))
        .await
        .expect_err("正規化した綴りでは引き当てられないこと");
    assert!(
        matches!(err, StoreError::UnknownCharacter),
        "引き当てが完全一致より広い（`trim` が復活している）: {err:?}"
    );
    let err = store
        .delete_character(&handle)
        .await
        .expect_err("正規化した綴りでは掃除できないこと");
    assert!(
        matches!(err, StoreError::UnknownCharacter),
        "引き当てが完全一致より広い（`trim` が復活している）: {err:?}"
    );

    // **ゴースト行はそのまま。隣の行も無傷。**
    assert_eq!(
        display_name_of(&store, &ghost).await,
        DISPLAY,
        "ゴースト行が書き換わっていないこと"
    );
    assert_eq!(
        stored_handles(&store).await,
        vec![HEALTHY_HANDLE.to_owned(), ghost.clone()],
        "1行も消えていないこと"
    );

    // **袋小路にはならない。** 同じ識別子を作り直せる（0004b-2 で塞がりが解けた）。
    store
        .create_character(
            &name(BROKEN_HANDLE),
            &display(REPAIRED_DISPLAY),
            &persona(NEW_PERSONA),
            None,
            fixture_scale(),
        )
        .await
        .expect("ゴースト行があっても同じ識別子を作れること");
    let entries = store.list_characters().await.expect("一覧を取得できること");
    assert_eq!(
        readable_named(&entries, BROKEN_HANDLE).display_name,
        display(REPAIRED_DISPLAY),
        "作り直した行が読める側に来ること"
    );
    assert_eq!(
        unreadable(&entries).len(),
        1,
        "ゴースト行は指し示されたまま残ること（黙って消さない）: {entries:?}"
    );
    assert_eq!(
        readable_named(&entries, HEALTHY_HANDLE).display_name,
        display(HEALTHY_DISPLAY),
        "隣の行が無傷であること"
    );
}

/// **全部正常なら、指し示すものは1件も無い。**
///
/// 正常な行まで「壊れている」側に入れる実装をここで落とす
/// （そうなると一覧が丸ごと使えなくなり、`/v1/models` からキャラが消える）。
#[tokio::test]
async fn a_healthy_listing_points_at_nothing() {
    let (_dir, store) = setup().await;
    assert_fixture_values_are_distinct();

    // 形式指示のある行と無い行の両方（NULL を「壊れている」と読む実装も落とす）。
    store
        .create_character(
            &name(HANDLE),
            &display(DISPLAY),
            &persona(PERSONA),
            Some(&output_format(FORMAT)),
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");
    store
        .create_character(
            &name(HEALTHY_HANDLE),
            &display(HEALTHY_DISPLAY),
            &persona(HEALTHY_PERSONA),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");

    let entries = store.list_characters().await.expect("一覧を取得できること");
    assert!(
        unreadable(&entries).is_empty(),
        "正常な行を壊れている側に入れないこと: {entries:?}"
    );
    assert_eq!(
        readable(&entries).len(),
        2,
        "2件とも読めること: {entries:?}"
    );

    // 前後に空白があるだけの表示名・ペルソナは**正規化して受理する**（照合キーではない）。
    // ここを「壊れている」に倒すと、直す必要の無い行が延々と一覧に並ぶ。
    sqlx::query("UPDATE character SET display_name = ?, base_persona = ? WHERE handle = ?")
        .bind(format!(" \u{3000}{HEALTHY_DISPLAY} "))
        .bind(format!("\u{3000}{HEALTHY_PERSONA}\n"))
        .bind(HEALTHY_HANDLE)
        .execute(store.pool())
        .await
        .expect("直接の UPDATE はスキーマ上通ること");

    let entries = store.list_characters().await.expect("一覧を取得できること");
    assert!(
        unreadable(&entries).is_empty(),
        "正規化して受理できる値を壊れている側に入れないこと: {entries:?}"
    );
    assert_eq!(
        readable_named(&entries, HEALTHY_HANDLE).display_name,
        display(HEALTHY_DISPLAY)
    );
}

// ---------------------------------------------------------------------------
// 性格ステータスの尺度（ADR-0015 / 作業単位 0011a-1）
// ---------------------------------------------------------------------------

/// **尺度は往復し、部分更新の対象になる**（ADR-0015 / 作業単位 0011a-1）。
///
/// # ここで固定していること
///
/// * `create_character` に渡した尺度が、`find_character_by_handle` と
///   `list_characters` の**両方**から同じ値で戻る。
/// * `CharacterUpdate::stat_scale` が `None` なら**触らない**
///   （`coalesce(?, stat_scale)` を `stat_scale = ?` に変える変異はここで赤くなる）。
/// * 尺度だけを変えても他の列が動かない
///   （`UPDATE_CHARACTER` の `?N` とバインド順の取り違えを捕まえる）。
///
/// # **値が尺度に従うことは主張していない**
///
/// `StatAlloc::value` は `u32` のままで、尺度を超える値をいま保存できる
/// （それを縛るのは 0011a-2）。ここが見ているのは**尺度そのものの往復**だけ。
#[tokio::test]
async fn the_stat_scale_survives_the_round_trip_and_partial_updates() {
    let (_dir, store) = setup().await;

    // 尺度は4つとも別の値にする（囮の尺度 / 対象の初期値 / 更新後 / 既定）。
    let decoy_scale = StatScale::parse(3).expect("正当な尺度であること");
    let initial = fixture_scale();
    let updated = StatScale::parse(42).expect("正当な尺度であること");
    for (left, right) in [
        (decoy_scale, initial),
        (decoy_scale, updated),
        (initial, updated),
        (decoy_scale, StatScale::DEFAULT),
        (initial, StatScale::DEFAULT),
        (updated, StatScale::DEFAULT),
    ] {
        assert_ne!(
            left, right,
            "フィクスチャの尺度は互いに、また既定とも違うこと"
        );
    }

    // **囮を先に作る。** 尺度を「一覧の先頭の行から取る」実装と区別できるようにする。
    store
        .create_character(
            &name("akane"),
            &display("茜音"),
            &persona("昼番の受付。"),
            None,
            decoy_scale,
        )
        .await
        .expect("囮を作成できること");
    store
        .create_character(
            &name("aoi-v2"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            Some(&output_format("台詞は「」で囲むこと。")),
            initial,
        )
        .await
        .expect("対象を作成できること");

    let found = store
        .find_character_by_handle(&name("aoi-v2"))
        .await
        .expect("検索が成功すること")
        .expect("対象を引けること");
    assert_eq!(found.stat_scale, initial, "作成時の尺度が戻ること");
    assert_eq!(
        readable_named(
            &store.list_characters().await.expect("一覧を取得できること"),
            "aoi-v2"
        )
        .stat_scale,
        initial,
        "一覧からも同じ尺度が読めること"
    );

    // --- 尺度を省いた更新は尺度を変えない ---
    store
        .update_character(
            &name("aoi-v2"),
            &CharacterUpdate {
                display_name: Some(display("葵 碧")),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("表示名だけを更新できること");
    let after_display = store
        .find_character_by_handle(&name("aoi-v2"))
        .await
        .expect("検索が成功すること")
        .expect("対象を引けること");
    assert_eq!(
        after_display.stat_scale, initial,
        "尺度を省いた更新で尺度が変わらないこと（既定へ戻らないこと）"
    );
    assert_eq!(after_display.display_name, display("葵 碧"));

    // --- 尺度だけを変えても他の列は動かない ---
    store
        .update_character(
            &name("aoi-v2"),
            &CharacterUpdate {
                stat_scale: Some(updated),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("尺度だけを更新できること");
    let after_scale = store
        .find_character_by_handle(&name("aoi-v2"))
        .await
        .expect("検索が成功すること")
        .expect("対象を引けること");
    assert_eq!(after_scale.stat_scale, updated, "尺度が置き換わること");
    assert_eq!(
        (
            after_scale.id,
            after_scale.handle.clone(),
            after_scale.display_name.clone(),
            after_scale.base_persona.clone(),
            after_scale.output_format_instruction.clone(),
            after_scale.created_at,
        ),
        (
            after_display.id,
            after_display.handle,
            after_display.display_name,
            after_display.base_persona,
            after_display.output_format_instruction,
            after_display.created_at,
        ),
        "尺度以外の列が動かないこと"
    );

    // --- 囮は最後まで無傷 ---
    let decoy = store
        .find_character_by_handle(&name("akane"))
        .await
        .expect("検索が成功すること")
        .expect("囮を引けること");
    assert_eq!(
        decoy.stat_scale, decoy_scale,
        "囮の尺度が巻き添えにならないこと"
    );
    assert_eq!(decoy.display_name, display("茜音"));
}

/// **範囲外の尺度が格納された行は「読めない行」になる**（作業単位 0011a-1）。
///
/// 1件を指す読み出し（`find_character_by_handle`）は
/// `StoreError::InvalidStoredValue` で**止まる**——他の列と同じ扱いで、
/// 壊れた値を握り潰して応答を続けるとキャラクターの振る舞いだけが黙って変わる。
///
/// 一覧は落ちずに `CharacterColumn::StatScale` を指し示す
/// （どの行が壊れているかを知る唯一の導線 / ADR-0016 §3）。
/// 列の網羅そのものは `every_kind_of_broken_column_is_pointed_at` の担当で、
/// ここが見ているのは**1件読み出しとの扱いの違い**。
#[tokio::test]
async fn a_row_with_an_out_of_range_stat_scale_stops_the_read_instead_of_disappearing() {
    let (_dir, store) = setup().await;

    let healthy = store
        .create_character(
            &name("akane"),
            &display("茜音"),
            &persona("昼番の受付。"),
            None,
            fixture_scale(),
        )
        .await
        .expect("正常な行を作成できること");
    let broken = store
        .create_character(
            &name("aoi-v2"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            None,
            fixture_scale(),
        )
        .await
        .expect("対象を作成できること");
    // **CHECK を外さないと作れない値**（`set_stat_scale_ignoring_checks` の doc）。
    set_stat_scale_ignoring_checks(&store, broken.get(), 0).await;

    let err = store
        .find_character_by_handle(&name("aoi-v2"))
        .await
        .expect_err("範囲外の尺度は握り潰さずエラーになること");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値が不正であることを名乗ること: {err:?}"
    );
    assert!(
        !err.to_string().contains('0'),
        "格納値をメッセージに載せないこと: {err}"
    );

    let entries = store
        .list_characters()
        .await
        .expect("一覧そのものは成功すること");
    assert_eq!(
        readable(&entries)
            .iter()
            .map(|character| character.id)
            .collect::<Vec<_>>(),
        vec![healthy],
        "正常な行が巻き添えにならないこと: {entries:?}"
    );
    let pointed = unreadable(&entries);
    assert_eq!(pointed.len(), 1, "{entries:?}");
    assert_eq!(pointed[0].id, broken, "壊れた行を id で指せること");
    assert_eq!(
        pointed[0].handle.as_ref(),
        Some(&name("aoi-v2")),
        "修復 API に渡せる識別子を返すこと"
    );
    assert_eq!(
        pointed[0].invalid_columns,
        vec![CharacterColumn::StatScale],
        "尺度の列を指すこと"
    );
}

// ---------------------------------------------------------------------------
// 値が尺度に従う（ADR-0015 / 作業単位 0011a-2）
// ---------------------------------------------------------------------------

/// **書き込みは「保存されている尺度で作られた値」しか受け付けない。**
///
/// # なぜ「値 ≤ 尺度」の数の検査ではないのか
///
/// `3/100` と `3/10` は**別の性格**なので、数だけを見る検査では
/// **他人の尺度で決めた値を、そのまま別のキャラクターへ書ける。**
/// 型は防げない（`StatValue` の doc「この型が守らないもの」）ので、
/// **照合は保存済みの尺度との突き合わせ**になる。
///
/// # フィクスチャの作り
///
/// * 対象の尺度（`fixture_scale()` = 7）・囮の尺度（100）・値（5 と 50）は**互いに違う数**。
/// * **対象の尺度に既定（`StatScale::DEFAULT` = 10）を使わない。**
///   使うと、要求の尺度を捨てて既定を書く／読む実装と区別できない
///   （`fixture_scale` の doc。**囮の尺度が別途守ってくれるとしても、直書きしない**
///   ——「囮があるから既定でよい」という前例を作らないため）。
/// * **取り違えの本体は「値 5・尺度 100」の組。** `5 <= 7` なので、
///   数の大小だけを見る実装は**これを通す。** `50` の方は数でも弾けてしまうため、
///   それだけでは取り違えの検査にならない（両方を投げて区別する）。
#[tokio::test]
async fn a_value_built_for_another_scale_is_refused_by_the_write() {
    let (_dir, store) = setup().await;

    let target_scale = fixture_scale();
    let other_scale = StatScale::parse(100).expect("正当な尺度であること");
    assert_ne!(target_scale, other_scale, "尺度は互いに違うこと");
    assert_ne!(
        other_scale,
        StatScale::DEFAULT,
        "囮の尺度も既定と違うこと（`fixture_scale()` 側は入口で保証される）"
    );

    // **囮を先に作る**（尺度を「先頭の行から取る」実装と区別できるようにする）。
    let decoy = store
        .create_character(
            &name("akane"),
            &display("茜音"),
            &persona("昼番の受付。"),
            None,
            other_scale,
        )
        .await
        .expect("囮を作成できること");
    let id = store
        .create_character(
            &name("aoi-v2"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            None,
            target_scale,
        )
        .await
        .expect("対象を作成できること");

    // --- 尺度の一致する値は通る。**尺度ちょうども通る**（`<=` を `<` にする変異の見張り） ---
    let accepted = [
        scaled_alloc(target_scale, "パッション", 0),
        scaled_alloc(target_scale, "ロジック", 4),
        scaled_alloc(target_scale, "共感", 7),
    ];
    store
        .replace_stat_allocs(id, &accepted)
        .await
        .expect("尺度の一致する値は保存できること");
    assert_eq!(
        store.stat_allocs(id).await.expect("性格値を取得できること"),
        accepted.to_vec(),
        "尺度ちょうど（7）も 0 も往復すること"
    );

    // --- 他人の尺度で作った値は拒否される ---
    //
    // **`5` が本命。** `5 <= 7` なので、数の大小しか見ない実装はここを通す。
    for refused in [
        vec![scaled_alloc(other_scale, "ロジック", 5)],
        vec![scaled_alloc(other_scale, "ロジック", 50)],
        // 1件でも混ざっていれば全体を拒否する（置き換えは丸ごとの操作）。
        vec![
            scaled_alloc(target_scale, "パッション", 1),
            scaled_alloc(other_scale, "ロジック", 5),
        ],
    ] {
        let err = store
            .replace_stat_allocs(id, &refused)
            .await
            .expect_err("他人の尺度で作った値は拒否されること");
        assert!(
            matches!(err, StoreError::StatScaleMismatch),
            "尺度の不一致として型付けされること: {err:?} / {refused:?}"
        );
        let message = err.to_string();
        assert!(
            !message.chars().any(|ch| ch.is_ascii_digit()),
            "値も尺度も Display に載せないこと: {message}"
        );
        assert_eq!(
            store.stat_allocs(id).await.expect("性格値を取得できること"),
            accepted.to_vec(),
            "拒否されたら1行も書き換わっていないこと: {refused:?}"
        );
    }

    // --- 囮は自分の尺度で書ける（対象の尺度が全員に効いているわけではない） ---
    store
        .replace_stat_allocs(decoy, &[scaled_alloc(other_scale, "ロジック", 50)])
        .await
        .expect("囮は自分の尺度の値を保存できること");
    assert_eq!(
        store
            .stat_allocs(decoy)
            .await
            .expect("性格値を取得できること"),
        vec![scaled_alloc(other_scale, "ロジック", 50)],
        "囮の値が対象の尺度で弾かれないこと"
    );
}

/// **読み出しはそのキャラクターの尺度を通す。壊れた行は置き換えで直せる。**
///
/// # 1行壊れると丸ごと失敗するのは**既知の制約**
///
/// `stat_allocs` は1件でも範囲外なら `InvalidStoredValue` を返す
/// （`memory_from_row` と同じ方針。理由はそちらの doc）。
/// **この作業単位では直さない**（`docs/06-roadmap.md` の積み残し）。
///
/// **代わりに復旧経路を固定する。** `replace_stat_allocs` は既存の軸を読まないので、
/// 壊れた行があっても置き換えは届く。「届くはず」ではなく**実際に直す**。
/// HTTP の側（`PUT /admin/characters/{handle}/stats`）は
/// `tests/server_admin.rs::a_row_beyond_the_scale_can_be_repaired_through_the_stats_endpoint`。
///
/// # **届くのは `stat_alloc` 側が壊れている場合だけ**
///
/// `character.stat_scale` 列そのものが壊れた行には置き換えが届かない
/// （尺度は突き合わせのために読むため）。そちらは
/// `a_broken_stat_scale_column_puts_the_replacement_out_of_reach` が測っている。
#[tokio::test]
async fn a_stored_value_beyond_the_characters_scale_stops_the_read() {
    let (_dir, store) = setup().await;

    // **既定（`StatScale::DEFAULT` = 10）と違う尺度で測る。** 既定のままだと、
    // 尺度を要求の値ではなく既定から取る実装と区別できず、
    // 境界（尺度ちょうど / 1つ超え）も既定の境界と同じ数になる（`fixture_scale` の doc）。
    //
    // 実測（2026-08-20）: 尺度を 10 で直書きしていた間、読み出しを
    // `StatScale::DEFAULT.value(..)` に変える変異を**このテストは通していた**
    // （同じ変異で他の7本は赤）。`fixture_scale()`（= 7）にして初めて赤くなる。
    let scale = fixture_scale();
    let id = store
        .create_character(
            &name("aoi-v2"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            None,
            scale,
        )
        .await
        .expect("キャラクターを作成できること");

    // **境界の下側: 尺度ちょうどは読める。**
    //
    // **尺度と値を結ぶ CHECK はスキーマに無い**ので、尺度を超える行は
    // `PRAGMA ignore_check_constraints` 無しで作れる（`migrations/0001_initial.sql`）。
    // CHECK が見るのは下限（0以上・整数型）だけで、そちらに触れる値
    // （負値）は `insert_stat_alloc_ignoring_checks` を通さないと作れない。
    for (axis, value) in [("ロジック", 7_i64), ("共感", 0)] {
        sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
            .bind(id.get())
            .bind(axis)
            .bind(value)
            .execute(store.pool())
            .await
            .expect("直接の INSERT はスキーマ上通ること");
    }
    assert_eq!(
        store.stat_allocs(id).await.expect("性格値を取得できること"),
        vec![
            scaled_alloc(scale, "ロジック", 7),
            scaled_alloc(scale, "共感", 0),
        ],
        "尺度ちょうどと 0 は読めること"
    );

    // **境界の上側: 1つ超えたら読み出しが止まる。**
    sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(id.get())
        .bind("パッション")
        .bind(8_i64)
        .execute(store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");

    let err = store
        .stat_allocs(id)
        .await
        .expect_err("尺度を超える値は握り潰さないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {err:?}"
    );
    let message = err.to_string();
    assert!(
        !message.chars().any(|ch| ch.is_ascii_digit()),
        "Display に格納値を載せないこと: {message}"
    );

    // --- **復旧経路**: 置き換えは既存の行を読まないので届く ---
    let repaired = [
        scaled_alloc(scale, "パッション", 6),
        scaled_alloc(scale, "ロジック", 7),
    ];
    store
        .replace_stat_allocs(id, &repaired)
        .await
        .expect("壊れた行があっても置き換えは届くこと");
    assert_eq!(
        store
            .stat_allocs(id)
            .await
            .expect("直した後は読めるようになること"),
        repaired.to_vec(),
        "置き換えた内容がそのまま読めること"
    );
}

/// **`stat_scale` の列そのものが壊れた行には、置き換えが届かない**（0011a-2 のレビューで実測）。
///
/// # 「置き換えが唯一の復旧経路」が成り立つ範囲
///
/// `replace_stat_allocs` は**既存の軸を読まない**ので、`stat_alloc` 側の値が尺度を
/// 超えていても届く（`a_stored_value_beyond_the_characters_scale_stops_the_read`）。
/// **しかし尺度そのものは読む**（書き込む値の尺度と突き合わせるため）。
/// だから **`character.stat_scale` 列が壊れた行では、置き換えも同じ
/// `InvalidStoredValue` で止まる。**
///
/// この行を直す入口は `PATCH`（`update_character`）で、保存済みの値が大きいと**3手**になる。
/// 手順と各段の実測値は下のコード中に書いてある。
/// **`replace_stat_allocs` / `SELECT_STAT_SCALE_BY_ID` の doc がこの手順の正**
/// （ADR-0015 にも ⚠ があるが、詳細はそちらへ委譲してある）。
///
/// # 測った値であること
///
/// 2026-08-20 / 尺度の列を `0` に潰し、`stat_alloc` に `value = 50` を直接 INSERT した行。
/// **「届かないはず」ではなく、各段の返り値を実際に取ってから書いている。**
#[tokio::test]
async fn a_broken_stat_scale_column_puts_the_replacement_out_of_reach() {
    let (_dir, store) = setup().await;

    // フィクスチャの尺度は4つとも互いに違う値にする（既定 10 とも違う）。
    let original = fixture_scale(); // 7
    let too_small = StatScale::parse(20).expect("正当な尺度であること");
    let big_enough = StatScale::parse(100).expect("正当な尺度であること");
    for (left, right) in [
        (original, too_small),
        (original, big_enough),
        (too_small, big_enough),
        (too_small, StatScale::DEFAULT),
        (big_enough, StatScale::DEFAULT),
    ] {
        assert_ne!(
            left, right,
            "フィクスチャの尺度は互いに、また既定とも違うこと"
        );
    }

    let id = store
        .create_character(
            &name("aoi-v2"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            None,
            original,
        )
        .await
        .expect("キャラクターを作成できること");

    // **尺度を超える値**（尺度と値を結ぶ CHECK はスキーマに無いので素の INSERT で作れる）。
    sqlx::query("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, ?)")
        .bind(id.get())
        .bind("ロジック")
        .bind(50_i64)
        .execute(store.pool())
        .await
        .expect("直接の INSERT はスキーマ上通ること");
    // **尺度の列そのものを壊す**（CHECK を外さないと作れない値）。
    set_stat_scale_ignoring_checks(&store, id.get(), 0).await;

    // --- 実測 1: 読み出しは止まる ---
    let err = store
        .stat_allocs(id)
        .await
        .expect_err("尺度の列が壊れた行は読めないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {err:?}"
    );

    // --- 実測 2: **置き換えも届かない**（`StatScaleMismatch` ですらない） ---
    //
    // ここが doc の主張と食い違っていた点。置き換えは尺度を `parse` してから
    // 突き合わせるので、尺度が壊れていると突き合わせる前に落ちる。
    let err = store
        .replace_stat_allocs(id, &[scaled_alloc(original, "ロジック", 1)])
        .await
        .expect_err("尺度の列が壊れた行には置き換えが届かないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "尺度の不一致ではなく格納値の不正で止まること: {err:?}"
    );

    // --- 実測 3: 手順1（`PATCH`）。**保存済みの値より小さい尺度は拒否される** ---
    //
    // 値 50 が入っているので 20 では足りない。**ここが「3手になる」理由。**
    let err = store
        .update_character(
            &name("aoi-v2"),
            &CharacterUpdate {
                stat_scale: Some(too_small),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect_err("保存済みの値より小さい尺度は拒否されること");
    assert!(
        matches!(err, StoreError::StatScaleBelowStoredValue),
        "拒否の理由まで一致すること: {err:?}"
    );

    // --- 実測 4: 保存済みの値以上なら通る。**尺度は行を読まずに書ける**（ADR-0016 §4） ---
    store
        .update_character(
            &name("aoi-v2"),
            &CharacterUpdate {
                stat_scale: Some(big_enough),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("壊れた尺度の行にも PATCH は届くこと");
    assert_eq!(
        store
            .stat_allocs(id)
            .await
            .expect("尺度を直せば読めるようになること"),
        vec![scaled_alloc(big_enough, "ロジック", 50)],
        "元の値はそのまま、新しい尺度を通して読めること"
    );

    // --- 実測 5: 手順2（`PUT .../stats`）。**値は新しい尺度で作り直す** ---
    //
    // 元の尺度（7）で作った値はもう通らない。だから「尺度を下げてから入れ直す」順序が取れず、
    // **上げる → 入れ直す → 下げる**の3手になる。
    let err = store
        .replace_stat_allocs(id, &[scaled_alloc(original, "ロジック", 1)])
        .await
        .expect_err("古い尺度で作った値は通らないこと");
    assert!(
        matches!(err, StoreError::StatScaleMismatch),
        "ここでは尺度の不一致として拒否されること: {err:?}"
    );
    store
        .replace_stat_allocs(id, &[scaled_alloc(big_enough, "ロジック", 1)])
        .await
        .expect("新しい尺度で作った値なら置き換えが届くこと");

    // --- 実測 6: 手順3（`PATCH`）。値が収まったので尺度を戻せる ---
    store
        .update_character(
            &name("aoi-v2"),
            &CharacterUpdate {
                stat_scale: Some(original),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("保存済みの値が収まっていれば尺度を下げられること");
    assert_eq!(
        store.stat_allocs(id).await.expect("3手のあとは読めること"),
        vec![scaled_alloc(original, "ロジック", 1)],
        "目的の尺度で読めるようになること"
    );
}

/// **保存済みの値より小さい尺度への更新は拒否する**（ADR-0015 / 作業単位 0011a-2）。
///
/// 許すと、その `PATCH` 一回で `stat_allocs` が丸ごと失敗するようになり、
/// **以後のチャットが必ず失敗する。** ADR-0015 が閉じると宣言した穴を、
/// アプリ自身が新しい経路から開けることになる。
///
/// # 判定と更新は不可分（`DELETE_CHARACTER` と同じ形）
///
/// 数えてから書かない。0件だった理由（該当なし / 尺度が小さすぎる）は
/// **同じトランザクションの中で**引き直す。
///
/// # **等しい値は許す**
///
/// 尺度 7 に値 7 は正当なので、判定の等号を取り違える変異はここで赤くなる。
#[tokio::test]
async fn lowering_the_scale_below_a_stored_value_is_refused() {
    let (_dir, store) = setup().await;

    // **初期尺度を既定（`StatScale::DEFAULT` = 10）と同値にしない。**
    // 同値にすると、`create_character` が引数を捨てて既定を書く変異が
    // このテストでは緑を通る（実測。0011a-2 のレビュー指摘）。
    let initial = StatScale::parse(20).expect("正当な尺度であること");
    let too_small = StatScale::parse(6).expect("正当な尺度であること");
    let exactly = StatScale::parse(7).expect("正当な尺度であること");
    let highest_value = 7;
    assert_ne!(initial, too_small);
    assert_ne!(initial, exactly);
    assert_ne!(
        initial,
        StatScale::DEFAULT,
        "初期尺度は既定と違う値であること（同値だと『常に既定を書く』実装を検出できない）"
    );
    assert_eq!(
        exactly.get(),
        highest_value,
        "「尺度 = 保存済みの最大値」の組であること（等号の見張り）"
    );

    // 囮。**対象より大きい値を持つ**ので、判定が「どのキャラクターの軸を見ているか」で結果が変わる。
    let decoy = store
        .create_character(
            &name("akane"),
            &display("茜音"),
            &persona("昼番の受付。"),
            None,
            initial,
        )
        .await
        .expect("囮を作成できること");
    store
        .replace_stat_allocs(decoy, &[scaled_alloc(initial, "ロジック", 10)])
        .await
        .expect("囮に性格値を設定できること");

    let id = store
        .create_character(
            &name("aoi-v2"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            None,
            initial,
        )
        .await
        .expect("対象を作成できること");
    store
        .replace_stat_allocs(
            id,
            &[
                scaled_alloc(initial, "パッション", highest_value),
                scaled_alloc(initial, "ロジック", 2),
            ],
        )
        .await
        .expect("性格値を設定できること");

    // --- 保存済みの値（7）より小さい尺度（6）は拒否 ---
    let err = store
        .update_character(
            &name("aoi-v2"),
            &CharacterUpdate {
                display_name: Some(display("葵 碧")),
                stat_scale: Some(too_small),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect_err("保存済みの値より小さい尺度は拒否されること");
    assert!(
        matches!(err, StoreError::StatScaleBelowStoredValue),
        "「該当なし」と区別できる理由を返すこと: {err:?}"
    );
    let message = err.to_string();
    assert!(
        !message.chars().any(|ch| ch.is_ascii_digit()),
        "値も尺度も Display に載せないこと: {message}"
    );

    // **1列も書き換わっていないこと**（同じ要求に載せた表示名も含めて）。
    let after = store
        .find_character_by_handle(&name("aoi-v2"))
        .await
        .expect("検索が成功すること")
        .expect("対象を引けること");
    assert_eq!(after.stat_scale, initial, "尺度が変わっていないこと");
    assert_eq!(
        after.display_name,
        display("葵"),
        "同じ要求の他の列も書き換わっていないこと"
    );

    // --- 保存済みの値とちょうど等しい尺度（7）は通る ---
    store
        .update_character(
            &name("aoi-v2"),
            &CharacterUpdate {
                stat_scale: Some(exactly),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("等しい尺度は許すこと");
    assert_eq!(
        store
            .find_character_by_handle(&name("aoi-v2"))
            .await
            .expect("検索が成功すること")
            .expect("対象を引けること")
            .stat_scale,
        exactly
    );

    // --- 値を下げれば、尺度も下げられる（ADR-0015 の「順序で足りる」復旧経路） ---
    store
        .replace_stat_allocs(id, &[scaled_alloc(exactly, "パッション", 6)])
        .await
        .expect("値を下げられること");
    store
        .update_character(
            &name("aoi-v2"),
            &CharacterUpdate {
                stat_scale: Some(too_small),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("値を下げた後なら尺度も下げられること");

    // --- 囮は巻き添えにならない ---
    assert_eq!(
        store
            .find_character_by_handle(&name("akane"))
            .await
            .expect("検索が成功すること")
            .expect("囮を引けること")
            .stat_scale,
        initial,
        "囮の尺度が動かないこと"
    );

    // --- 軸を1つも持たないキャラクターは、どこまでも下げられる ---
    store
        .create_character(
            &name("rin"),
            &display("凛"),
            &persona("屋上でギターを弾いている。"),
            None,
            initial,
        )
        .await
        .expect("作成できること");
    store
        .update_character(
            &name("rin"),
            &CharacterUpdate {
                stat_scale: Some(StatScale::parse(StatScale::MIN).expect("正当な尺度であること")),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("軸が無いなら下限まで下げられること");
}

/// **「該当なし」と「尺度が小さすぎる」は別のエラー。**
///
/// 判定を1本に潰すと、打ち間違い（綴りを直せば通る）と仕様上の拒否
/// （綴りを直しても永久に通らない）が区別できない。`delete_character` と同じ話。
#[tokio::test]
async fn an_unknown_character_is_not_reported_as_a_scale_conflict() {
    let (_dir, store) = setup().await;

    let err = store
        .update_character(
            &name("aoi-v2"),
            &CharacterUpdate {
                stat_scale: Some(StatScale::parse(2).expect("正当な尺度であること")),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect_err("存在しないキャラクターの更新は失敗すること");
    assert!(
        matches!(err, StoreError::UnknownCharacter),
        "尺度の衝突ではなく「該当なし」を名乗ること: {err:?}"
    );
}

/// **尺度を下げる更新と、値の置き換えが同時に走っても、
/// 「保存されている値が保存されている尺度を超える」状態は作れない。**
///
/// # なぜ逐次のテストでは足りないのか
///
/// `replace_stat_allocs` が保存済みの尺度を**トランザクションの外**で読むと、
/// 読んでから書くまでの隙に `PATCH` が尺度を下げられる。
/// **検査した尺度と、書き込む先の尺度が別物**になり、
/// 書けてしまった値はそのキャラクターの `stat_allocs` を丸ごと止める
/// ——ADR-0015 が閉じると宣言した穴が、2つの正当な API の同時実行から開く。
///
/// **逐次のテストでは差が出ない**（`deleting_never_races_a_starting_conversation` と同じ形）。
///
/// # 見ているのは「どちらが勝ったか」ではなく不変条件
///
/// * **両方が成功することは無い。** 先に尺度が下がれば置き換えは
///   `StatScaleMismatch`、先に値が入れば更新は `StatScaleBelowStoredValue`。
/// * **どう転んでも読み出しは成功する。**
///
/// # **負けた側の理由まで見る**（`is_ok()` の組では足りない）
///
/// `replace_stat_allocs` が素の `BEGIN`（DEFERRED）で読みから始まっていた間の実測
/// （2026-08-20 / 直列 / 1プロセスずつ 5 回 × 64 周 = 320 周のうち、
/// 標準エラーから採取できた 315 周）:
///
/// ```text
/// replaced = Err(Query(database is locked)) / shrunk = Ok   298 周
/// replaced = Ok / shrunk = Err(StatScaleBelowStoredValue)     17 周
/// replaced = Err(StatScaleMismatch) / shrunk = Ok              0 周
/// ```
///
/// **`!(replaced.is_ok() && shrunk.is_ok())` はこの 298 周を緑のまま通す。**
/// 「先に尺度が下がれば置き換えは `StatScaleMismatch`」という主張も、
/// このときは**一度も成り立っていなかった**（0/315）。
///
/// `BEGIN IMMEDIATE` にした後の同条件の実測は
/// **`database is locked` が 0 周**（置き換えが先 314 周 / 尺度の更新が先 1 周）で、
/// **どちらの向きも実際に起きる。**
/// だから**理由まで照合する**——施錠の退行（`BEGIN IMMEDIATE` を素の `BEGIN` に戻す等）は
/// `StoreError::Query` として現れるので、ここで赤くなる。
#[tokio::test]
async fn replacing_stats_never_races_a_shrinking_scale() {
    let (_dir, store) = setup().await;

    // **既定（`StatScale::DEFAULT` = 10）を直書きしない**（`fixture_scale` の doc）。
    // 要求の尺度を捨てて既定を書く／読む実装と区別できなくなる。
    let high = fixture_scale();
    let low = StatScale::parse(5).expect("正当な尺度であること");
    assert_ne!(high, low, "2つの尺度は互いに違うこと");
    assert_ne!(
        low,
        StatScale::DEFAULT,
        "下げ先の尺度も既定と違うこと（`fixture_scale()` 側は入口で保証される）"
    );

    for round in 0..64 {
        let handle = format!("aoi-{round}");
        let id = store
            .create_character(
                &name(&handle),
                &display(DISPLAY),
                &persona(PERSONA),
                None,
                high,
            )
            .await
            .expect("キャラクターを作成できること");
        // **尺度を下げられる状態から始める**（既存の値は `low` 以下）。
        // ここが `low` を超えていると、更新は競合と無関係に必ず拒否される。
        store
            .replace_stat_allocs(id, &[scaled_alloc(high, "ロジック", 4)])
            .await
            .expect("初期値を設定できること");

        let replacing = {
            let store = store.clone();
            tokio::spawn(async move {
                // **`low` を超える値**（尺度 `high` としては正当。尺度ちょうど）。
                store
                    .replace_stat_allocs(id, &[scaled_alloc(high, "パッション", 7)])
                    .await
            })
        };
        let shrinking = {
            let store = store.clone();
            let target = name(&handle);
            tokio::spawn(async move {
                store
                    .update_character(
                        &target,
                        &CharacterUpdate {
                            stat_scale: Some(low),
                            ..CharacterUpdate::default()
                        },
                    )
                    .await
            })
        };

        let replaced = replacing.await.expect("タスクが panic しないこと");
        let shrunk = shrinking.await.expect("タスクが panic しないこと");

        // **負けた側の理由まで見る。** `!(replaced.is_ok() && shrunk.is_ok())` だけでは
        // **両方が `database is locked` で落ちた状態も緑になる**（実測はこの関数の doc）。
        match (&replaced, &shrunk) {
            // 置き換えが先に commit した → 尺度を下げる側が保存済みの値に阻まれる。
            (Ok(()), Err(StoreError::StatScaleBelowStoredValue))
            // 尺度を下げる側が先に commit した → 置き換えの値が保存済みの尺度と合わない。
            | (Err(StoreError::StatScaleMismatch), Ok(_)) => {}
            _ => panic!(
                "想定した2通り以外の結果になった（{round} 周目）: \
                 replaced = {replaced:?} / shrunk = {shrunk:?}"
            ),
        }
        store
            .stat_allocs(id)
            .await
            .unwrap_or_else(|err| panic!("どう転んでも読み出せること（{round} 周目）: {err:?}"));
    }
}

/// **軸を持つキャラクターにも、尺度を含まない部分更新が届く**（作業単位 0011a-2）。
///
/// `UPDATE_CHARACTER` の `NOT EXISTS (... AND stat_alloc.value > ?5)` は、
/// **`?5` が NULL（尺度を変えない）のとき常に真になる**ことに依存している
/// （SQL の `value > NULL` は NULL であって真にならない）。
///
/// この前提が崩れると——たとえば `?5` に既定値を与える形にすると——
/// **軸を1つでも持つキャラクターは、尺度を含まない `PATCH` が全部拒否される**
/// （`StatScaleBelowStoredValue` / HTTP では 409）。表示名も形式指示も直せなくなる。
///
/// # `the_stat_scale_survives_the_round_trip_and_partial_updates` では見張れない
///
/// あちらの対象は `stat_alloc` の行を**1つも持たない。**
/// 行が無ければ `NOT EXISTS` は NULL の扱いと関係なく真になるので、
/// `?5` を `coalesce(?5, -1)` に変える変異が**そのまま緑を通る**（実測 810/810 緑）。
/// **軸を持たせて初めて、この節が「NULL のとき真」であることを見張れる。**
#[tokio::test]
async fn a_partial_update_reaches_a_character_that_already_has_axes() {
    let (_dir, store) = setup().await;

    let scale = fixture_scale();
    let id = store
        .create_character(
            &name("aoi-v2"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            Some(&output_format("台詞は「」で囲むこと。")),
            scale,
        )
        .await
        .expect("対象を作成できること");

    // **値 0 の軸も混ぜる。** `coalesce(?5, -1)` の形の変異は
    // 「値 > -1」なので、0 の行1つでも引っ掛かる。
    let stored = [
        scaled_alloc(scale, "パッション", 7),
        scaled_alloc(scale, "ロジック", 0),
    ];
    store
        .replace_stat_allocs(id, &stored)
        .await
        .expect("軸を設定できること");

    // --- 尺度を含まない部分更新（表示名 / 形式指示の消去）は届く ---
    for update in [
        CharacterUpdate {
            display_name: Some(display("葵 碧")),
            ..CharacterUpdate::default()
        },
        CharacterUpdate {
            output_format_instruction: FieldUpdate::Clear,
            ..CharacterUpdate::default()
        },
    ] {
        assert!(
            update.stat_scale.is_none(),
            "この検証は尺度を含まない更新のためのもの: {update:?}"
        );
        assert_eq!(
            store
                .update_character(&name("aoi-v2"), &update)
                .await
                .unwrap_or_else(|err| panic!(
                    "軸を持つキャラクターにも尺度を含まない更新が届くこと: {update:?}: {err:?}"
                )),
            id,
            "更新した行の id が返ること"
        );
    }

    let after = store
        .find_character_by_handle(&name("aoi-v2"))
        .await
        .expect("検索が成功すること")
        .expect("対象を引けること");
    assert_eq!(after.display_name, display("葵 碧"), "表示名が変わること");
    assert_eq!(
        after.output_format_instruction, None,
        "形式指示が消えること"
    );
    assert_eq!(after.stat_scale, scale, "尺度は動かないこと");
    assert_eq!(
        store.stat_allocs(id).await.expect("性格値を取得できること"),
        stored.to_vec(),
        "軸も動かないこと"
    );
}

/// **親の居ない `stat_alloc` の行は、黙って消えずに読み出しを止める**（作業単位 0011a-2）。
///
/// # 「外部キーが有効だから孤児は作れない」は成り立たない
///
/// `PRAGMA foreign_keys` は**接続ごと**の設定で、プールから借りた接続の上で倒せる
/// （`delete_character_ignoring_foreign_keys` の doc）。脅威モデルは
/// ゴースト行（`insert_ghost_character`）や範囲外の尺度
/// （`set_stat_scale_ignoring_checks`）と同じ「DB を直接編集された場合の担保」。
///
/// # 内部結合にすると黙って消える
///
/// `SELECT_STAT_ALLOCS` を `JOIN` にすると孤児の行は結果から落ち、
/// **`Ok([])`**——性格ステータスを設定してあるキャラクターが「一軸も持たない」ように見える。
/// **エラーにも警告にもならないので、誰も気付けない。**
/// `LEFT JOIN` なら尺度が NULL として現れ、`InvalidStoredValue` で止まる。
#[tokio::test]
async fn stat_allocs_of_an_orphaned_row_stop_the_read_instead_of_disappearing() {
    let (_dir, store) = setup().await;

    let scale = fixture_scale();
    // 囮。**孤児にしない**ので、読み出しが巻き添えにならないことを見られる。
    let decoy = store
        .create_character(
            &name("akane"),
            &display("茜音"),
            &persona("昼番の受付。"),
            None,
            scale,
        )
        .await
        .expect("囮を作成できること");
    store
        .replace_stat_allocs(decoy, &[scaled_alloc(scale, "共感", 5)])
        .await
        .expect("囮に性格値を設定できること");

    let id = store
        .create_character(
            &name("aoi-v2"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            None,
            scale,
        )
        .await
        .expect("対象を作成できること");
    store
        .replace_stat_allocs(
            id,
            &[
                scaled_alloc(scale, "パッション", 7),
                scaled_alloc(scale, "ロジック", 0),
            ],
        )
        .await
        .expect("対象に性格値を設定できること");

    delete_character_ignoring_foreign_keys(&store, id.get()).await;

    // **仕掛けが効いていることを先に確かめる。** カスケードが走っていたら
    // 子の行は残らず、以下の検証は「軸が無いだけ」の話になってしまう。
    let orphans: i64 = sqlx::query_scalar("SELECT count(*) FROM stat_alloc WHERE character_id = ?")
        .bind(id.get())
        .fetch_one(store.pool())
        .await
        .expect("孤児の件数を数えられること");
    assert_eq!(orphans, 2, "親を消しても子の行が残っていること（孤児）");

    let err = store
        .stat_allocs(id)
        .await
        .expect_err("親の居ない行は黙って消さずにエラーになること");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {err:?}"
    );

    // --- 囮は巻き添えにならない ---
    assert_eq!(
        store
            .stat_allocs(decoy)
            .await
            .expect("囮の性格値は読めること"),
        vec![scaled_alloc(scale, "共感", 5)],
        "孤児のある DB でも、健全な行はそのまま読めること"
    );
}

/// **負の値が格納されていたら、`0` として読まずに読み出しを止める**（作業単位 0011a-2）。
///
/// # `u32::try_from` の等価性は**上側にしか成り立たない**
///
/// 「`u32` に収まらない値は尺度（上限 100）にも収まらない」——これは正しいが、
/// **負の値には当てはまらない。** 負値を弾いているのは `try_from` **だけ**で、
/// `StatScale::value` は `u32` しか受け取らないので**そこには届かない。**
///
/// 負値だけを潰して受理する形（`u32::try_from(stored_value.max(0))`）にすると、
/// `-5` の行は **`0` として読める。** そして
/// **値 0 の軸は `prompt` が絶対制約で丸ごと落とす**ので、
/// エラーにも警告にもならず**その軸だけが静かに消える**
/// ——ADR-0015 が「尺度を超える軸を消す」を却下した理由そのものの形。
///
/// # 負値だけは CHECK を外さないと作れない
///
/// `stat_alloc.value` の CHECK は下限（0以上・整数型）を見るので、
/// 尺度を超える値（`8/7`）と違って素の INSERT では入らない
/// （`insert_stat_alloc_ignoring_checks` の doc）。
#[tokio::test]
async fn a_negative_stored_value_stops_the_read_instead_of_being_read_as_zero() {
    let (_dir, store) = setup().await;

    let scale = fixture_scale();
    let id = store
        .create_character(
            &name("aoi-v2"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            None,
            scale,
        )
        .await
        .expect("対象を作成できること");
    store
        .replace_stat_allocs(id, &[scaled_alloc(scale, "ロジック", 3)])
        .await
        .expect("正常な軸を設定できること");

    insert_stat_alloc_ignoring_checks(&store, id.get(), "パッション", -5).await;

    let err = store
        .stat_allocs(id)
        .await
        .expect_err("負の値は握り潰さずエラーになること");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {err:?}"
    );
    let message = err.to_string();
    assert!(
        !message.chars().any(|ch| ch.is_ascii_digit()),
        "格納値を Display に載せないこと: {message}"
    );

    // --- 復旧経路: 置き換えは既存の行を読まないので届く ---
    let repaired = [scaled_alloc(scale, "パッション", 1)];
    store
        .replace_stat_allocs(id, &repaired)
        .await
        .expect("負の値があっても置き換えは届くこと");
    assert_eq!(
        store
            .stat_allocs(id)
            .await
            .expect("直した後は読めるようになること"),
        repaired.to_vec(),
        "置き換えた内容がそのまま読めること"
    );
}

/// 軸を1行も持たないキャラクターを1体作る（B-19 の見張り番の土台）。
///
/// **識別子と表示名は別の綴り**（ADR-0011）。尺度は `fixture_scale()`（既定と違う値）。
async fn character_without_axes(store: &Store, handle: &str) -> CharacterId {
    store
        .create_character(
            &name(handle),
            &display("葵"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること")
}

/// **正規化すると同じ軸になる行が2つあれば、その読みは丸ごと止まる**（作業単位 B-19）。
///
/// # DB は一意のつもりでいる
///
/// `stat_alloc` の主キーは `(character_id, axis)` だが、**`axis` は生のテキスト**なので
/// `"パッション"` と `" パッション"` は同居できる（`insert_stat_alloc_directly` が
/// 成功すること自体がその観測）。**一意でなくしているのは読み出し側の正規化**である。
///
/// # 是正前に何が起きていたか（B-16 のレビューの実測）
///
/// `Store::stat_allocs` が両方を受理するので、**同じ軸の行が2つ返る。**
/// その先で `GET .../drift` は**同じ集計を2回引き当て**、
/// `ups + downs` の総和が `total_votes` を超えた（実測: 総数 3 に対して総和 6）。
/// `GET .../stats` の側でも同じ軸が2行並び、**どちらの値が本当かを応答から決められない。**
///
/// # なぜ「読めない行」と同じ扱いにするのか
///
/// **落として1行にすると、消した方の値は応答のどこにも現れない**
/// ——このリポジトリが一貫して避けてきた形（孤児を `LEFT JOIN` で見せる /
/// 読めない行を `unreadable` として指し示す / 値 0 の軸を落とさない）。
/// **どちらの値を残すかを決める根拠も無い。**
///
/// # **空白の5通りが押さえているのは `trim_blank` の文字集合**
///
/// **重複の検査の分岐を5通り通っているのではない**（レビューの指摘 / 2026-08-28）。
/// 差は**全部 `StatAxis::parse`（`trim_blank`）が吸収する**ので、
/// `allocs.iter().any(...)` から見た入力は5回とも同じである。
/// **効いているのは前提の `assert_eq!` のほう**——
/// `trim_blank` が全角空白（U+3000）やタブを落とし続けることを、ここが回している。
#[tokio::test]
async fn stat_allocs_stop_when_two_rows_normalize_to_the_same_axis() {
    let (_dir, store) = setup().await;

    for (index, padded) in [
        " パッション",
        "パッション ",
        " パッション ",
        "\u{3000}パッション",
        "パッション\t",
    ]
    .into_iter()
    .enumerate()
    {
        // **前提を先に押さえる。** 綴りは違うのに、正規化すると同じ軸になること。
        assert_ne!(padded, "パッション", "生の綴りは違うこと: {padded:?}");
        assert_eq!(
            StatAxis::parse(padded),
            StatAxis::parse("パッション"),
            "正規化すると同じ軸になること: {padded:?}"
        );

        let id = character_without_axes(&store, &format!("aoi-{index}")).await;
        // **値をずらす。** 同じ値だと「どちらを返しても同じ」になり、
        // 落として1行にする実装と区別が付かない。
        insert_stat_alloc_directly(&store, id.get(), "パッション", 3).await;
        insert_stat_alloc_directly(&store, id.get(), padded, 5).await;

        let err = store
            .stat_allocs(id)
            .await
            .expect_err("同じ軸に正規化される2行は握り潰さないこと");
        assert!(
            matches!(err, StoreError::InvalidStoredValue),
            "格納値の不正として型付けされること: {padded:?}: {err:?}"
        );
        let message = err.to_string();
        assert!(
            !message.contains("パッション"),
            "Display に格納値を含めないこと: {message}"
        );
        assert!(
            !message.chars().any(|ch| ch.is_ascii_digit()),
            "Display に値も件数も含めないこと: {message}"
        );
    }
}

/// **重複は隣どうしとは限らない**（作業単位 B-19）。
///
/// # なぜ別のテストが要るのか
///
/// 直前の行とだけ突き合わせる実装（`dedup` / `windows(2)` の形）は、
/// **重複が隣接しているフィクスチャでは正しく見える。**
/// `ORDER BY rowid` は挿入順なので、**間に別の軸を1つ挟むだけ**でその実装は通り抜ける。
///
/// 3行が同じ軸に潰れる場合も見る——**2行のときだけ数える**実装を落とすため。
#[tokio::test]
async fn stat_allocs_stop_even_when_the_duplicate_is_not_the_next_row() {
    let (_dir, store) = setup().await;

    let spread = character_without_axes(&store, "aoi-spread").await;
    insert_stat_alloc_directly(&store, spread.get(), "パッション", 3).await;
    insert_stat_alloc_directly(&store, spread.get(), "ロジック", 4).await;
    insert_stat_alloc_directly(&store, spread.get(), " パッション", 5).await;

    assert!(
        matches!(
            store.stat_allocs(spread).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "間に別の軸を挟んだ重複も止めること"
    );

    let triple = character_without_axes(&store, "aoi-triple").await;
    insert_stat_alloc_directly(&store, triple.get(), "パッション", 1).await;
    insert_stat_alloc_directly(&store, triple.get(), " パッション", 2).await;
    insert_stat_alloc_directly(&store, triple.get(), "パッション\u{3000}", 3).await;

    assert!(
        matches!(
            store.stat_allocs(triple).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "3行が同じ軸に潰れる場合も止めること"
    );
}

/// **正規化しても違う軸は、そのまま全部返る**（作業単位 B-19）。
///
/// # ここが「正常な行を壊す変異」の受け皿
///
/// 重複の検査を「前後の空白を落とす前の綴りが違えば重複」のように緩めたり、
/// **`StatAxis::parse` を通した後の比較を字面の一部だけに変えたり**すると、
/// **正当な軸どうしが重複として弾かれる。** その形はここで赤になる。
///
/// # 中間の空白は保持される（`StatAxis::parse` の決定）
///
/// `"内なる 情熱"` と `"内なる\u{3000}情熱"` と `"内なる情熱"` は**3つとも別の軸**。
/// **前提として実際に確かめてから**並べる——`parse` が中間の空白まで潰すように変わると、
/// このフィクスチャは「重複を置いている」ことになり、テストの意味が裏返る。
///
/// **前後に空白のある行が1つだけなら、これまで通り正規化して受理する**
/// （`stat_allocs_of_rows_inserted_outside_the_store` の決定を変えていない）。
#[tokio::test]
async fn stat_allocs_keep_axes_that_are_still_different_after_normalizing() {
    let (_dir, store) = setup().await;

    let inner = ["内なる 情熱", "内なる\u{3000}情熱", "内なる情熱"];
    for (index, left) in inner.into_iter().enumerate() {
        for right in inner.into_iter().skip(index + 1) {
            assert_ne!(
                StatAxis::parse(left),
                StatAxis::parse(right),
                "前提: 中間の空白の違いは別の軸として残ること"
            );
        }
    }

    let id = character_without_axes(&store, "aoi-distinct").await;
    for (axis, value) in [
        ("内なる 情熱", 1),
        ("内なる\u{3000}情熱", 2),
        ("内なる情熱", 3),
        // **前後に空白のある行が1つだけなら受理する**（正規化して返る）。
        (" ロジック ", 4),
        ("パッション", 5),
    ] {
        insert_stat_alloc_directly(&store, id.get(), axis, value).await;
    }

    assert_eq!(
        store
            .stat_allocs(id)
            .await
            .expect("重複していない行は読めること"),
        vec![
            alloc("内なる 情熱", 1),
            alloc("内なる\u{3000}情熱", 2),
            alloc("内なる情熱", 3),
            alloc("ロジック", 4),
            alloc("パッション", 5),
        ],
        "5軸とも、保存順のまま正規化して返ること"
    );
}

/// **重複が止めるのは、その持ち主の読みだけ**（作業単位 B-19）。
///
/// 軸名は**キャラクターをまたいで共有される**（別人が同じ「パッション」を持つ）。
///
/// **⚠ 「持ち主で絞らずに数える実装」は書けない**（レビューの指摘 / 2026-08-28）。
/// `SELECT_STAT_ALLOCS` が既に `WHERE character_id = ?` で絞っているので、
/// **その変異を書こうとすると SQL の WHERE を落とすしかなく、無関係な数十本が赤になる。**
/// ここが残すのは**「過剰検出しない」対照**としての意味である——
/// 別人が同じ綴りの軸を持っていても、重複していない側は読めること。
#[tokio::test]
async fn a_duplicated_axis_stops_only_that_characters_read() {
    let (_dir, store) = setup().await;

    let broken = character_without_axes(&store, "aoi").await;
    let bystander = character_without_axes(&store, "rin").await;
    assert_ne!(broken, bystander, "前提: 2体の id が別であること");

    insert_stat_alloc_directly(&store, broken.get(), "パッション", 3).await;
    insert_stat_alloc_directly(&store, broken.get(), " パッション", 5).await;
    // **同じ綴りの軸を傍観者にも置く**（値は別）。
    insert_stat_alloc_directly(&store, bystander.get(), "パッション", 1).await;
    insert_stat_alloc_directly(&store, bystander.get(), "ロジック", 2).await;

    assert!(
        matches!(
            store.stat_allocs(broken).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "重複している側は止まること"
    );
    assert_eq!(
        store
            .stat_allocs(bystander)
            .await
            .expect("傍観者の読みは止まらないこと"),
        vec![alloc("パッション", 1), alloc("ロジック", 2)],
        "同じ綴りの軸を持っていても、別人の重複には巻き込まれないこと"
    );
}

/// **重複している行は `PUT .../stats` 相当の置き換えで直せる**（作業単位 B-19）。
///
/// # 「直したい行ほど読めない」に落ちていないこと（ADR-0016 §4 / `D-6` の形）
///
/// `replace_stat_allocs` は**既存の軸を読まない**（DELETE してから INSERT する）ので、
/// 重複した行があっても届く。**入れる軸は `StatAxis` を通っている**ので、
/// 置き換えた後の行は主キーの上でも正規化の上でも一意になる。
#[tokio::test]
async fn a_duplicated_axis_can_still_be_replaced() {
    let (_dir, store) = setup().await;

    let id = character_without_axes(&store, "aoi").await;
    insert_stat_alloc_directly(&store, id.get(), "パッション", 3).await;
    insert_stat_alloc_directly(&store, id.get(), " パッション", 5).await;
    assert!(
        matches!(
            store.stat_allocs(id).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "前提: 直す前は読めないこと"
    );

    let repaired = [alloc("パッション", 6), alloc("ロジック", 0)];
    store
        .replace_stat_allocs(id, &repaired)
        .await
        .expect("重複していても置き換えは届くこと");

    assert_eq!(
        store
            .stat_allocs(id)
            .await
            .expect("直した後は読めるようになること"),
        repaired.to_vec(),
        "置き換えた内容がそのまま読めること"
    );
}

// ---------------------------------------------------------------------------
// 変容の走らせ方（ADR-0045 / 作業単位 A-8）
// ---------------------------------------------------------------------------

/// **走らせ方は作成・一覧・1件読み・部分更新の全部で往復する**（ADR-0045 §決定1・§決定3）。
///
/// # 見ていること
///
/// * **作成で書いた値が、1件読みと一覧の両方から戻る。**
/// * **囮（別の値）を並べる**——「一覧の先頭の行から取る」実装と区別する。
/// * **省いた更新は変えない**（`Keep` を `Clear` に潰すと、表示名を直すたびに未設定へ戻る）。
/// * **`Set` / `Clear` はそれぞれ効き、他の列は動かない。**
/// * **従来の `create_character` は未設定（NULL）で作る**
///   ——既存の呼び出し（テストと投入スクリプト）の振る舞いが変わらない。
#[tokio::test]
async fn the_stat_drift_mode_survives_the_round_trip_and_partial_updates() {
    let (_dir, store) = setup().await;

    // 囮: 従来の入口で作る（＝未設定）。
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
    // 囮2: 対象と違う値を書く。
    store
        .create_character_with_drift_mode(
            &name("kaede"),
            &display("楓"),
            &persona("夜番の司書。"),
            None,
            fixture_scale(),
            Some(StatDriftMode::Shadow),
        )
        .await
        .expect("囮2を作成できること");
    store
        .create_character_with_drift_mode(
            &name("aoi-v2"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            Some(&output_format("台詞は「」で囲むこと。")),
            fixture_scale(),
            Some(StatDriftMode::On),
        )
        .await
        .expect("対象を作成できること");

    assert_eq!(
        found(&store, "aoi-v2").await.stat_drift_mode,
        Some(StatDriftMode::On)
    );
    assert_eq!(
        found(&store, "akane").await.stat_drift_mode,
        None,
        "従来の入口は未設定"
    );
    assert_eq!(
        found(&store, "kaede").await.stat_drift_mode,
        Some(StatDriftMode::Shadow)
    );
    let listed = store.list_characters().await.expect("一覧を取得できること");
    assert_eq!(
        [
            readable_named(&listed, "akane").stat_drift_mode,
            readable_named(&listed, "kaede").stat_drift_mode,
            readable_named(&listed, "aoi-v2").stat_drift_mode,
        ],
        [None, Some(StatDriftMode::Shadow), Some(StatDriftMode::On)],
        "一覧からも行ごとの値が読めること"
    );

    // --- 省いた更新は変えない ---
    store
        .update_character(
            &name("aoi-v2"),
            &CharacterUpdate {
                display_name: Some(display("葵 碧")),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("表示名だけを更新できること");
    let after_display = found(&store, "aoi-v2").await;
    assert_eq!(
        after_display.stat_drift_mode,
        Some(StatDriftMode::On),
        "走らせ方を省いた更新で走らせ方が変わらないこと"
    );
    assert_eq!(after_display.display_name, display("葵 碧"));

    // --- `Set` は置き換え、他の列は動かさない ---
    store
        .update_character(
            &name("aoi-v2"),
            &CharacterUpdate {
                stat_drift_mode: FieldUpdate::Set(StatDriftMode::Off),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("走らせ方だけを更新できること");
    let after_set = found(&store, "aoi-v2").await;
    assert_eq!(after_set.stat_drift_mode, Some(StatDriftMode::Off));
    assert_eq!(
        Character {
            stat_drift_mode: after_display.stat_drift_mode,
            ..after_set.clone()
        },
        after_display,
        "走らせ方以外の列が動かないこと"
    );

    // --- `Clear` は未設定へ戻す ---
    store
        .update_character(
            &name("aoi-v2"),
            &CharacterUpdate {
                stat_drift_mode: FieldUpdate::Clear,
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("走らせ方を未設定へ戻せること");
    assert_eq!(found(&store, "aoi-v2").await.stat_drift_mode, None);

    // --- 囮は最後まで無傷 ---
    assert_eq!(found(&store, "akane").await.stat_drift_mode, None);
    assert_eq!(
        found(&store, "kaede").await.stat_drift_mode,
        Some(StatDriftMode::Shadow)
    );
}

/// 識別子で1件引く（見つからなければ落とす）。
async fn found(store: &Store, handle: &str) -> Character {
    store
        .find_character_by_handle(&name(handle))
        .await
        .expect("検索が成功すること")
        .unwrap_or_else(|| panic!("{handle} を引けること"))
}

/// **6列を1回で全部違う値へ書き、どれも取り違えていない**（作業単位 A-8 / A-9a）。
///
/// `UPDATE_CHARACTER` の `?N` とバインド順は**別々に書かれている**
/// （あちらの doc）。走らせ方の2つのプレースホルダを足し、A-9a で枠の数の `?9` を
/// `handle`（`?8`）の後ろに足したので、取り違えはどの組でも赤になる形で見る。
///
/// **枠の数は 2**（既定の 0 とも上限の 4 とも違い、尺度の 42 とも違う）。
#[tokio::test]
async fn every_updatable_column_lands_in_its_own_place() {
    let (_dir, store) = setup().await;
    store
        .create_character(
            &name("aoi"),
            &display("葵"),
            &persona("落ち着いた物腰の司書。"),
            None,
            fixture_scale(),
        )
        .await
        .expect("対象を作成できること");

    let scale = StatScale::parse(42).expect("正当な尺度であること");
    store
        .update_character(
            &name("aoi"),
            &CharacterUpdate {
                display_name: Some(display("碧")),
                base_persona: Some(persona("新しいペルソナ。")),
                output_format_instruction: FieldUpdate::Set(output_format("地の文は短く。")),
                stat_scale: Some(scale),
                stat_drift_mode: FieldUpdate::Set(StatDriftMode::Shadow),
                axis_discovery_slots: Some(
                    AxisDiscoverySlots::parse(2).expect("正当な枠の数であること"),
                ),
                // **しまう（`?10` / 作業単位 A-10）。** `handle`（`?8`）・枠の数（`?9`）の後ろに足した。
                archived: Some(true),
            },
        )
        .await
        .expect("全列を更新できること");

    let after = found(&store, "aoi").await;
    assert!(after.archived_at.is_some(), "しまった時刻が入ること");
    assert_eq!(
        (
            after.display_name,
            after.base_persona,
            after.output_format_instruction,
            after.stat_scale,
            after.stat_drift_mode,
            after.axis_discovery_slots.get(),
        ),
        (
            display("碧"),
            persona("新しいペルソナ。"),
            Some(output_format("地の文は短く。")),
            scale,
            Some(StatDriftMode::Shadow),
            2,
        )
    );
}

/// **規則外の綴りを持つ行は、1件読みを止め、一覧では指し示される**（作業単位 A-8）。
///
/// CHECK は外せる（`set_stat_drift_mode_ignoring_checks`）ので、読み出し側でも
/// `StatDriftMode::parse` を通す。**環境変数の別名（`yes`）を植える**——
/// 読み出しが環境変数の語彙で読むと、ここが黙って読めてしまう。
///
/// **壊れた行を黙って「未設定」に落とさない。** 落とすと、`on` にしたつもりの
/// キャラクターが既定（多くは off）で走り、**変容は本文に出ない**（ADR-0024 §9）ので
/// 誰も気付けない。
#[tokio::test]
async fn a_row_with_an_unknown_stat_drift_mode_is_pointed_at() {
    let (_dir, store) = setup().await;

    let healthy = store
        .create_character_with_drift_mode(
            &name("akane"),
            &display("茜音"),
            &persona("昼番の受付。"),
            None,
            fixture_scale(),
            Some(StatDriftMode::On),
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
    set_stat_drift_mode_ignoring_checks(&store, broken.get(), "yes").await;

    assert!(
        matches!(
            store.find_character_by_handle(&name("aoi")).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "1件読みは止まること"
    );

    let entries = store.list_characters().await.expect("一覧は落ちないこと");
    assert_eq!(
        unreadable(&entries),
        vec![&UnreadableCharacter {
            id: broken,
            handle: Some(name("aoi")),
            invalid_columns: vec![CharacterColumn::StatDriftMode],
        }],
        "壊れた列を指し示すこと"
    );
    let healthy_row = readable_named(&entries, "akane");
    assert_eq!(healthy_row.id, healthy);
    assert_eq!(healthy_row.stat_drift_mode, Some(StatDriftMode::On));

    // **直す入口は `PATCH` と同じ更新**（読まずに書く / ADR-0016 §4）。
    store
        .update_character(
            &name("aoi"),
            &CharacterUpdate {
                stat_drift_mode: FieldUpdate::Clear,
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("壊れた行も更新で直せること");
    assert_eq!(found(&store, "aoi").await.stat_drift_mode, None);
}

/// **範囲外の枠の数を持つ行は、1件読みを止め、一覧では指し示される**（作業単位 A-9a）。
///
/// CHECK は外せる（`set_axis_discovery_slots_ignoring_checks`）ので、読み出し側でも
/// `AxisDiscoverySlots::parse` を通す。**上端のすぐ外（5）と負の値（-1）の両方**を植える
/// ——`u32` への変換だけ・上限だけを見る実装は、どちらかで黙って読めてしまう。
///
/// **壊れた値を 0 に落とさない。** 落とすと、枠を開けたはずのキャラクターで候補が黙って出なくなる。
#[tokio::test]
async fn a_row_with_out_of_range_axis_discovery_slots_is_pointed_at() {
    for stored in [5_i64, -1] {
        let (_dir, store) = setup().await;
        let healthy = store
            .create_character(
                &name("akane"),
                &display("茜音"),
                &persona("昼番の受付。"),
                None,
                fixture_scale(),
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
        set_axis_discovery_slots_ignoring_checks(&store, broken.get(), stored).await;

        assert!(
            matches!(
                store.find_character_by_handle(&name("aoi")).await,
                Err(StoreError::InvalidStoredValue)
            ),
            "{stored}: 1件読みは止まること"
        );

        let entries = store.list_characters().await.expect("一覧は落ちないこと");
        assert_eq!(
            unreadable(&entries),
            vec![&UnreadableCharacter {
                id: broken,
                handle: Some(name("aoi")),
                invalid_columns: vec![CharacterColumn::AxisDiscoverySlots],
            }],
            "{stored}: 壊れた列を指し示すこと"
        );
        assert_eq!(readable_named(&entries, "akane").id, healthy);

        // **直す入口は `PATCH` と同じ更新**（読まずに書く / ADR-0016 §4）。
        store
            .update_character(
                &name("aoi"),
                &CharacterUpdate {
                    axis_discovery_slots: Some(
                        AxisDiscoverySlots::parse(2).expect("正当な枠の数であること"),
                    ),
                    ..CharacterUpdate::default()
                },
            )
            .await
            .expect("壊れた行も更新で直せること");
        assert_eq!(found(&store, "aoi").await.axis_discovery_slots.get(), 2);
    }
}

/// **TEXT 以外の値を入れた行がどう読まれるか**（作業単位 A-8 のレビュー指摘 / 実測を固定する）。
///
/// # 実測（2026-09-26）
///
/// | 入れた値 | `typeof` | 1件読み | 一覧 |
/// |---|---|---|---|
/// | 整数 `1` | **`text`** | `InvalidStoredValue` | **読めない行として指し示す** |
/// | 小数 `1.5` | **`text`** | `InvalidStoredValue` | **読めない行として指し示す** |
/// | BLOB `b"on"` | `blob` | **`Query(ColumnDecode)`** | **丸ごと落ちる**（`Query(ColumnDecode)`） |
///
/// **列が TEXT 宣言なので、整数と小数は型アフィニティで TEXT に変換されて入る**
/// ——`try_get::<Option<String>>` の型不一致にはならず、`parse` が止める。
/// **型不一致で一覧ごと落ちるのは BLOB だけ**（TEXT アフィニティは BLOB を変換しない）。
/// `stat_scale` の「整数でない値（TEXT / REAL）は一覧ごと落ちる」と同じ種類の限界で、
/// こちらで該当するのは BLOB（`Store::list_characters` の doc）。
///
/// **BLOB の行があっても、他の行の1件読みは通る**（一覧だけが落ちる）。
#[tokio::test]
async fn a_drift_mode_that_is_not_text_is_pointed_at_unless_it_is_a_blob() {
    for (label, kind) in [("integer", 0), ("real", 1), ("blob", 2)] {
        let (_dir, store) = setup().await;
        store
            .create_character(
                &name("akane"),
                &display("茜音"),
                &persona("昼番の受付。"),
                None,
                fixture_scale(),
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
        match kind {
            0 => set_stat_drift_mode_ignoring_checks(&store, broken.get(), 1_i64).await,
            1 => set_stat_drift_mode_ignoring_checks(&store, broken.get(), 1.5_f64).await,
            _ => set_stat_drift_mode_ignoring_checks(&store, broken.get(), b"on".to_vec()).await,
        }
        let stored_type: String =
            sqlx::query_scalar("SELECT typeof(stat_drift_mode) FROM character WHERE id = ?")
                .bind(broken.get())
                .fetch_one(store.pool())
                .await
                .expect("格納された型を読めること");

        let one = store.find_character_by_handle(&name("aoi")).await;
        let listed = store.list_characters().await;
        // **他の行の1件読みは、どの場合も通る。**
        assert_eq!(
            found(&store, "akane").await.stat_drift_mode,
            None,
            "{label}: 健全な行の1件読みは巻き込まれないこと"
        );

        if kind == 2 {
            assert_eq!(stored_type, "blob", "前提: BLOB は変換されないこと");
            assert!(
                matches!(one, Err(StoreError::Query(_))),
                "{label}: 1件読みはデコードで落ちる: {one:?}"
            );
            assert!(
                matches!(listed, Err(StoreError::Query(_))),
                "{label}: 一覧は丸ごと落ちる（構造的な限界）: {listed:?}"
            );
        } else {
            assert_eq!(
                stored_type, "text",
                "前提: {label} は TEXT に変換されて入ること"
            );
            assert!(
                matches!(one, Err(StoreError::InvalidStoredValue)),
                "{label}: 1件読みは parse で止まる: {one:?}"
            );
            let entries = listed.expect("一覧は落ちないこと");
            assert_eq!(
                unreadable(&entries),
                vec![&UnreadableCharacter {
                    id: broken,
                    handle: Some(name("aoi")),
                    invalid_columns: vec![CharacterColumn::StatDriftMode],
                }],
                "{label}: 読めない行として指し示すこと"
            );
        }
    }
}

/// **全キャラクターの軸の本数の最大**（`Store::max_stat_axes` / 作業単位 B-60d-1 の再レビュー）。
///
/// * 行が無ければ 0。
/// * **値 0 の軸も数える**（変容の依頼には載る / ADR-0027 §決定4）。
/// * **しまったキャラクターの軸も数える**（しまう直前に始まった裏方は、しまった後も走りうる）。
#[tokio::test]
async fn the_max_stat_axes_counts_every_character_including_zero_values_and_archived_ones() {
    let (_dir, store) = setup().await;
    assert_eq!(store.max_stat_axes().await.expect("数えられること"), 0);

    let small = store
        .create_character(
            &name("aoi"),
            &display("葵"),
            &persona("本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("作成できること");
    let large = store
        .create_character(
            &name("bee"),
            &display("蜂"),
            &persona("本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("作成できること");
    for axis in ["勇気", "慎重"] {
        insert_stat_alloc_directly(&store, small.get(), axis, 1).await;
    }
    // 4本のうち2本は値 0。
    for (axis, value) in [("勇気", 0), ("慎重", 3), ("陽気", 0), ("几帳面", 1)] {
        insert_stat_alloc_directly(&store, large.get(), axis, value).await;
    }
    assert_eq!(
        store.max_stat_axes().await.expect("数えられること"),
        4,
        "値 0 の軸も数え、別のキャラクターの本数の最大を返すこと"
    );

    // しまったキャラクターも数える。
    sqlx::query("UPDATE character SET archived_at = '2026-10-22T00:00:00.000Z' WHERE id = ?")
        .bind(large.get())
        .execute(store.pool())
        .await
        .expect("しまえること");
    assert_eq!(store.max_stat_axes().await.expect("数えられること"), 4);
}
