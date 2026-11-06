//! **キャラクターを「しまう」「完全に消す」**の永続化の検証
//! （[ADR-0049](../docs/adr/0049-deleting-characters-and-conversations.md) §決定1 / 作業単位 A-10 / Issue origin#55）。
//!
//! ここで確かめるのは主に次の点:
//!
//!   * しまう／戻すが `character.archived_at` に書かれ、**中身（会話・記憶・性格値）を1行も変えない**こと。
//!   * **しまったキャラクターは、会話があっても完全に消せる**こと。消えるのは**そのキャラクターの行だけ**で、
//!     キャラクターを指す表は**表ごとに**1行も残らないこと（`ON DELETE CASCADE`）。
//!   * しまっていない・会話のあるキャラクターは、今までどおり `CharacterHasSessions` で1行も消えないこと。
//!   * **判定と削除が1文**であること（消す要求と戻す要求を同時に走らせても、両方は成功しない）。
//!   * 完全に消した後は、同じ識別子でもう一度作れること。
//!
//! 「会話が無いキャラクターの削除」と「会話があって消せない」の基本形は `tests/store_character.rs` にある。

use chrono::Utc;
use personacore::domain::{
    AxisCandidateRecord, AxisCandidateState, AxisDiscoveryMiss, AxisDiscoverySlots, AxisTrialVote,
    CharacterColumn, CharacterEntry, CharacterId, CharacterUpdate, DisplayName, DriftDelta,
    DriftOutcome, DriftProposal, DriftSuggestion, Handle, MemoryContent, NewMemory, Persona,
    StatAlloc, StatAxis, StatDriftMode, Subject, apply_drift,
};
use personacore::store::{Store, StoreError};
use sqlx::Row;
use tempfile::TempDir;

mod common;

use common::{CLOCK_SKEW, fixture_scale};

/// 一時ディレクトリ上に新規 DB を作り、マイグレーション適用済みの `Store` を返す。
async fn setup() -> (TempDir, Store) {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let store = Store::connect(dir.path().join("personacore.sqlite"))
        .await
        .expect("接続とマイグレーション適用が成功すること");
    (dir, store)
}

fn name(raw: &str) -> Handle {
    Handle::parse(raw).expect("テストで使う識別子は正当であること")
}

async fn create(store: &Store, handle: &str) -> CharacterId {
    store
        .create_character(
            &name(handle),
            &DisplayName::parse("葵").expect("正当な表示名"),
            &Persona::parse("静かな図書館の司書。").expect("正当なペルソナ"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること")
}

/// しまう（`archived: Some(true)`）だけの更新。
fn put_away() -> CharacterUpdate {
    CharacterUpdate {
        archived: Some(true),
        ..CharacterUpdate::default()
    }
}

/// 戻す（`archived: Some(false)`）だけの更新。
fn restore() -> CharacterUpdate {
    CharacterUpdate {
        archived: Some(false),
        ..CharacterUpdate::default()
    }
}

async fn archived_at_of(store: &Store, handle: &str) -> Option<chrono::DateTime<Utc>> {
    store
        .find_character_by_handle(&name(handle))
        .await
        .expect("検索が成功すること")
        .expect("キャラクターが存在すること")
        .archived_at
}

/// 格納値そのもの（`parse` を通さずに見る）。
async fn stored_archived_at(store: &Store, handle: &str) -> Option<String> {
    sqlx::query_scalar("SELECT archived_at FROM character WHERE handle = ?")
        .bind(handle)
        .fetch_one(store.pool())
        .await
        .expect("格納値を読めること")
}

// ======================================================================
// しまう・戻す
// ======================================================================

/// **しまうと時刻が入り、戻すと消える。** 2回しまっても**最初の時刻のまま**。
///
/// 3体並べて**真ん中だけ**をしまう（端の行だけを書く／全行を書く取り違えを、両隣が見張る）。
#[tokio::test]
async fn putting_away_records_the_time_and_restoring_clears_it() {
    let (_dir, store) = setup().await;
    for handle in ["akane", "aoi", "yuzu"] {
        create(&store, handle).await;
    }
    assert_eq!(
        archived_at_of(&store, "aoi").await,
        None,
        "作ったときはしまわれていない"
    );

    let before = Utc::now();
    store
        .update_character(&name("aoi"), &put_away())
        .await
        .expect("しまえること");
    let after = Utc::now();

    let first = archived_at_of(&store, "aoi")
        .await
        .expect("しまった時刻が入ること");
    assert!(
        first >= before - CLOCK_SKEW && first <= after + CLOCK_SKEW,
        "しまった時刻はいまであること: {first} / {before}〜{after}"
    );
    for neighbour in ["akane", "yuzu"] {
        assert_eq!(
            archived_at_of(&store, neighbour).await,
            None,
            "両隣はしまわれないこと: {neighbour}"
        );
    }

    // **2回目は時刻を書き換えない**（いつしまったかを、しまい直しで失わない）。
    let stored = stored_archived_at(&store, "aoi").await;
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    store
        .update_character(&name("aoi"), &put_away())
        .await
        .expect("しまったものをもう一度しまっても成功すること");
    assert_eq!(
        stored_archived_at(&store, "aoi").await,
        stored,
        "2回しまっても最初の時刻のまま"
    );

    store
        .update_character(&name("aoi"), &restore())
        .await
        .expect("戻せること");
    assert_eq!(
        archived_at_of(&store, "aoi").await,
        None,
        "戻すと時刻が消える"
    );
}

/// **しまう・戻すを省いた更新は、しまった状態に触らない。**
///
/// 表示名だけを変える `PATCH` のたびに戻ってしまうと、しまったキャラクターが黙って一覧へ帰ってくる。
#[tokio::test]
async fn an_update_that_does_not_mention_archiving_leaves_it_alone() {
    let (_dir, store) = setup().await;
    create(&store, "aoi").await;
    store
        .update_character(&name("aoi"), &put_away())
        .await
        .expect("しまえること");
    let stored = stored_archived_at(&store, "aoi").await;
    assert!(stored.is_some(), "前提: しまってあること");

    store
        .update_character(
            &name("aoi"),
            &CharacterUpdate {
                display_name: Some(DisplayName::parse("碧").expect("正当な表示名")),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("表示名だけを変えられること");

    assert_eq!(
        stored_archived_at(&store, "aoi").await,
        stored,
        "しまった状態は変わらないこと"
    );
}

/// **しまっても中身は1行も変わらない**（ADR-0049 §決定1「戻せる」）。
#[tokio::test]
async fn putting_away_and_restoring_change_nothing_else() {
    let (_dir, store) = setup().await;
    let id = create(&store, "aoi").await;
    populate(&store, id, "a").await;
    let before = owned_counts(&store, id).await;

    store
        .update_character(&name("aoi"), &put_away())
        .await
        .expect("しまえること");
    assert_eq!(
        owned_counts(&store, id).await,
        before,
        "しまっても中身は変わらない"
    );

    store
        .update_character(&name("aoi"), &restore())
        .await
        .expect("戻せること");
    assert_eq!(
        owned_counts(&store, id).await,
        before,
        "戻しても中身は変わらない"
    );
}

/// **一覧はしまったキャラクターも返す**（しまった時刻つき）。出し分けは読み手の仕事。
///
/// `/admin/characters` は、しまったキャラクターの一覧（画面）の元でもある。
#[tokio::test]
async fn the_listing_still_returns_put_away_characters_with_their_time() {
    let (_dir, store) = setup().await;
    for handle in ["akane", "aoi", "yuzu"] {
        create(&store, handle).await;
    }
    store
        .update_character(&name("aoi"), &put_away())
        .await
        .expect("しまえること");

    let listed: Vec<(String, bool)> = store
        .list_characters()
        .await
        .expect("一覧を取れること")
        .into_iter()
        .map(|entry| match entry {
            CharacterEntry::Readable(character) => (
                character.handle.as_str().to_owned(),
                character.archived_at.is_some(),
            ),
            CharacterEntry::Unreadable(broken) => panic!("読めない行は無いはず: {broken:?}"),
        })
        .collect();
    assert_eq!(
        listed,
        vec![
            ("akane".to_owned(), false),
            ("aoi".to_owned(), true),
            ("yuzu".to_owned(), false),
        ]
    );
}

/// **しまった時刻として読めない値は、一覧が指し示す**（他の列と同じ扱い / ADR-0016 §3）。
///
/// 読めない時刻を「しまっていない」に落とすと、しまったはずのキャラクターが黙って
/// `/v1/models` に戻ってくる。1件を指す読み出しは止める。
#[tokio::test]
async fn an_unreadable_archived_time_is_pointed_at() {
    let (_dir, store) = setup().await;
    create(&store, "akane").await;
    let id = create(&store, "aoi").await;
    sqlx::query("UPDATE character SET archived_at = 'いつか' WHERE id = ?")
        .bind(id.get())
        .execute(store.pool())
        .await
        .expect("直接書き込めること");

    let entries = store
        .list_characters()
        .await
        .expect("一覧そのものは取れること");
    let broken: Vec<_> = entries
        .iter()
        .filter_map(|entry| match entry {
            CharacterEntry::Unreadable(broken) => Some(broken),
            CharacterEntry::Readable(_) => None,
        })
        .collect();
    assert_eq!(
        broken.len(),
        1,
        "壊れた行だけが指し示されること: {entries:?}"
    );
    assert_eq!(broken[0].id, id);
    assert_eq!(broken[0].invalid_columns, vec![CharacterColumn::ArchivedAt]);

    assert!(
        matches!(
            store.find_character_by_handle(&name("aoi")).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "1件を指す読み出しは止めること"
    );

    // **戻せば直る**（`PATCH` は行を読まずに書く / ADR-0016 §4）。
    store
        .update_character(&name("aoi"), &restore())
        .await
        .expect("壊れた行でも戻せること");
    assert_eq!(archived_at_of(&store, "aoi").await, None);
}

// ======================================================================
// 完全に消す
// ======================================================================

/// **しまっていない・会話のあるキャラクターは、今までどおり消せない。** 1行も消えない。
#[tokio::test]
async fn a_character_with_a_conversation_that_is_not_put_away_cannot_be_deleted() {
    let (_dir, store) = setup().await;
    let id = create(&store, "aoi").await;
    populate(&store, id, "a").await;
    let before = owned_counts(&store, id).await;

    let err = store
        .delete_character(&name("aoi"))
        .await
        .expect_err("しまっていない・会話のあるキャラクターは消せないこと");
    assert!(
        matches!(err, StoreError::CharacterHasSessions),
        "理由は今までどおり「会話がある」: {err:?}"
    );
    assert_eq!(owned_counts(&store, id).await, before, "1行も消えないこと");

    // **戻した後も同じ**（しまって→戻したキャラクターは、しまっていないキャラクター）。
    store
        .update_character(&name("aoi"), &put_away())
        .await
        .expect("しまえること");
    store
        .update_character(&name("aoi"), &restore())
        .await
        .expect("戻せること");
    assert!(
        matches!(
            store.delete_character(&name("aoi")).await,
            Err(StoreError::CharacterHasSessions)
        ),
        "戻したキャラクターは、また消せなくなること"
    );
    assert_eq!(owned_counts(&store, id).await, before, "1行も消えないこと");
}

/// **しまったキャラクターは、会話があっても完全に消せる。キャラクターを指す表は、表ごとに全部消える。**
///
/// # 表の一覧は `sqlite_master` と突き合わせる
///
/// 数える表を手で並べるだけだと、**表が1枚増えた日に、その表を数えないまま緑**になる。
/// [`OWNED_ROWS`] の表の集合が実際の表の集合と一致することを先に確かめる
/// （表を足したら、ここが赤くなって「その表も消えてよいか」を決めることになる）。
///
/// # 隣のキャラクターにも同じ形の行を置く
///
/// 消えるのは**対象の行だけ**であること（全行を消す・隣の行を巻き込む取り違え）を、
/// 隣の行が1行も減らないことで見る。**対象の行は消す前に必ず1行以上ある**ことも確かめる
/// （0 → 0 は何も見ていない）。
#[tokio::test]
async fn a_put_away_character_is_deleted_with_every_row_that_points_at_it() {
    let (_dir, store) = setup().await;
    assert_owned_rows_cover_every_table(&store).await;

    let akane = create(&store, "akane").await;
    let aoi = create(&store, "aoi").await;
    let yuzu = create(&store, "yuzu").await;
    populate(&store, akane, "k").await;
    populate(&store, aoi, "a").await;
    populate(&store, yuzu, "y").await;

    let target_before = owned_counts(&store, aoi).await;
    for (table, count) in &target_before {
        assert!(*count > 0, "前提: 対象の行が {table} に1行以上あること");
    }
    let akane_before = owned_counts(&store, akane).await;
    let yuzu_before = owned_counts(&store, yuzu).await;

    store
        .update_character(&name("aoi"), &put_away())
        .await
        .expect("しまえること");
    store
        .delete_character(&name("aoi"))
        .await
        .expect("しまったキャラクターは会話があっても消せること");

    let target_after = owned_counts(&store, aoi).await;
    for (table, count) in &target_after {
        assert_eq!(*count, 0, "対象の行は {table} に1行も残らないこと");
    }
    assert_eq!(
        owned_counts(&store, akane).await,
        akane_before,
        "隣の行は1行も減らないこと"
    );
    assert_eq!(
        owned_counts(&store, yuzu).await,
        yuzu_before,
        "隣の行は1行も減らないこと"
    );

    // **表全体の行数も、隣の2体の分だけ**（所有の問い合わせが取りこぼした行が残っていないこと）。
    for (table, total_sql) in TOTAL_ROWS {
        let total: i64 = sqlx::query_scalar(total_sql)
            .fetch_one(store.pool())
            .await
            .expect("行数を数えられること");
        let expected = count_in(&akane_before, table) + count_in(&yuzu_before, table);
        assert_eq!(total, expected, "{table} の行は隣の2体の分だけ残ること");
    }
}

/// **会話の無いキャラクターは、しまわなくても今までどおり消せる。**
#[tokio::test]
async fn a_character_without_a_conversation_is_still_deleted_without_putting_it_away() {
    let (_dir, store) = setup().await;
    create(&store, "akane").await;
    create(&store, "aoi").await;
    create(&store, "yuzu").await;

    store
        .delete_character(&name("aoi"))
        .await
        .expect("会話の無いキャラクターは消せること");

    let handles: Vec<String> = sqlx::query_scalar("SELECT handle FROM character ORDER BY id")
        .fetch_all(store.pool())
        .await
        .expect("格納値を読めること");
    assert_eq!(handles, vec!["akane".to_owned(), "yuzu".to_owned()]);
}

/// **完全に消した後は、同じ識別子でもう一度作れる**（ADR-0049 §決定1）。
///
/// 作り直したキャラクターは**前の中身を1行も引き継がない**（指す行がカスケードで消えている）。
///
/// # ⚠ **id は同じになりうる**（作業単位 A-10 で実測）
///
/// `character.id` は AUTOINCREMENT ではない（`session` / `message` は 0003 で作り直したが、
/// `character` はそのまま）。**最大の id の行を消すと、次に作った行が同じ id を使う。**
/// ここでは消した直後に作るので、実際に同じ id になる。**中身を引き継がないことは下で見ている。**
/// 消す前から走っていた裏方が古い id のまま書いても新しいキャラクターに届かないのは、
/// 裏方の書き込みを往復の会話で縛ってあるから
/// （[`writes_bound_to_a_deleted_characters_session_never_reach_a_new_character_with_the_same_id`]）。
#[tokio::test]
async fn a_deleted_handle_can_be_used_again() {
    let (_dir, store) = setup().await;
    let old = create(&store, "aoi").await;
    populate(&store, old, "a").await;
    store
        .update_character(&name("aoi"), &put_away())
        .await
        .expect("しまえること");
    store
        .delete_character(&name("aoi"))
        .await
        .expect("消せること");

    let new = create(&store, "aoi").await;
    assert_eq!(
        archived_at_of(&store, "aoi").await,
        None,
        "作り直したものはしまわれていない"
    );
    for (table, count) in owned_counts(&store, new).await {
        let expected = i64::from(table == "character");
        assert_eq!(count, expected, "前の中身を引き継がないこと: {table}");
    }
}

/// **会話で縛った裏方の書き込みは、消したキャラクターと同じ id の新しいキャラクターに届かない**
/// （ADR-0049 §決定3 / 作業単位 A-10 のレビュー前の差し戻し）。
///
/// 裏方の書き手を全部（記憶・値・票・累積・軸の候補の3つの腕・試みの記録）、2種類の「よその会話」で呼ぶ。
///
/// | 渡す会話 | 期待する拒否 |
/// |---|---|
/// | **消したキャラクターの会話**（もう無い） | `TurnSessionGone`（キャラクターが消えた） |
/// | **別のキャラクターの、今も在る会話** | `SessionOfAnotherCharacter`（取り違え。「消えた」と読ませない） |
///
/// どちらでも、新しいキャラクターにも別のキャラクターにも**1行も入らない。**
/// **対照**として、新しいキャラクター自身の会話で呼ぶと全部書ける（「縛り」が「いつも断る」に化けていないこと）。
///
/// 軸の候補は `Propose` / `TrialVote` / `Advance` の3つの腕を全部通す —— 会話の照合は
/// `match` の手前にあるので、腕ごとに通らないと「1つの腕の中へ移した」変異を見逃す。
/// `TrialVote` の腕は、**票の会話は正しく、往復の会話だけがよそ**の形にする。
///
/// **id が実際に同じであることを前提として確かめる**（使い回されなければ何も見ていない）。
#[tokio::test]
async fn writes_bound_to_a_deleted_characters_session_never_reach_a_new_character_with_the_same_id()
{
    let (_dir, store) = setup().await;
    let akane = create(&store, "akane").await;
    let akane_session = store.create_session(akane).await.expect("会話を作れること");
    let old = create(&store, "aoi").await;
    let old_session = store.create_session(old).await.expect("会話を作れること");
    store
        .update_character(&name("aoi"), &put_away())
        .await
        .expect("しまえること");
    store
        .delete_character(&name("aoi"))
        .await
        .expect("消せること");

    let new = create(&store, "aoi").await;
    assert_eq!(new, old, "前提: id が使い回されていること");
    // 縛りが無ければ通ってしまう形にしておく: 新しいキャラクターにも会話・記憶・枠がある。
    let new_session = store.create_session(new).await.expect("会話を作れること");
    assert_ne!(
        new_session, old_session,
        "前提: 会話の id は使い回されないこと"
    );
    let source = store
        .add_memory_in_session(new, new_session, &memory("猫が好き"))
        .await
        .expect("新しいキャラクター自身の会話なら書けること（対照）");
    store
        .update_character(
            &name("aoi"),
            &CharacterUpdate {
                axis_discovery_slots: Some(AxisDiscoverySlots::parse(2).expect("正当な枠の数")),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("枠を開けられること");
    let axis = StatAxis::parse("好奇心").expect("正当な軸名");
    let allocs = vec![StatAlloc {
        axis: axis.clone(),
        value: fixture_scale().value(3).expect("尺度の中"),
    }];
    store
        .replace_stat_allocs(new, &allocs)
        .await
        .expect("性格ステータスを置けること");
    // 試用中の候補（`TrialVote` / `Advance` の腕の宛先）。**新しいキャラクター自身の会話で**作る。
    let on_trial = store
        .record_axis_candidate_in_session(
            new,
            new_session,
            &AxisCandidateRecord::Propose {
                name: StatAxis::parse("照れ屋").expect("正当な名前"),
                source_memory: source,
            },
        )
        .await
        .expect("候補を作れること（対照）");
    let outcome = apply_drift(
        &allocs,
        DriftProposal::Suggested(DriftSuggestion {
            axis: axis.clone(),
            delta: DriftDelta::Up,
        }),
    );
    let ballot = outcome.vote().expect("票になること");
    let DriftOutcome::Applied(change) = outcome else {
        panic!("前提: 値が動く提案であること: {outcome:?}");
    };
    let before_new = owned_counts(&store, new).await;
    let before_akane = owned_counts(&store, akane).await;

    let permit = || StatDriftMode::On.applying().expect("on は許可証を出すこと");
    for (origin, label, expected) in [
        (old_session, "消したキャラクターの会話", "TurnSessionGone"),
        (
            akane_session,
            "別のキャラクターの、今も在る会話",
            "SessionOfAnotherCharacter",
        ),
    ] {
        let results: Vec<(&str, Result<(), StoreError>)> = vec![
            (
                "add_memory_in_session",
                store
                    .add_memory_in_session(new, origin, &memory("よその会話の記憶"))
                    .await
                    .map(|_| ()),
            ),
            (
                "apply_stat_drift_in_session",
                store
                    .apply_stat_drift_in_session(new, origin, &change, permit())
                    .await
                    .map(|_| ()),
            ),
            (
                "record_drift_vote",
                store.record_drift_vote(new, origin, &ballot).await,
            ),
            (
                "save_drift_charges_in_session",
                store
                    .save_drift_charges_in_session(new, origin, &[(axis.clone(), 0.5)], permit())
                    .await,
            ),
            (
                "record_axis_candidate_in_session(Propose)",
                store
                    .record_axis_candidate_in_session(
                        new,
                        origin,
                        &AxisCandidateRecord::Propose {
                            name: StatAxis::parse("候補").expect("正当な名前"),
                            source_memory: source,
                        },
                    )
                    .await
                    .map(|_| ()),
            ),
            (
                "record_axis_candidate_in_session(TrialVote)",
                store
                    .record_axis_candidate_in_session(
                        new,
                        origin,
                        &AxisCandidateRecord::TrialVote {
                            candidate: on_trial,
                            session: new_session,
                            vote: AxisTrialVote::Up,
                        },
                    )
                    .await
                    .map(|_| ()),
            ),
            (
                "record_axis_candidate_in_session(Advance)",
                store
                    .record_axis_candidate_in_session(
                        new,
                        origin,
                        &AxisCandidateRecord::Advance {
                            candidate: on_trial,
                            to: AxisCandidateState::Passed,
                        },
                    )
                    .await
                    .map(|_| ()),
            ),
            (
                "record_axis_discovery_miss_in_session",
                store
                    .record_axis_discovery_miss_in_session(
                        new,
                        origin,
                        source,
                        AxisDiscoveryMiss::Malformed,
                    )
                    .await,
            ),
        ];
        for (writer, result) in results {
            let refused_as_expected = match expected {
                "TurnSessionGone" => matches!(result, Err(StoreError::TurnSessionGone)),
                _ => matches!(result, Err(StoreError::SessionOfAnotherCharacter)),
            };
            assert!(
                refused_as_expected,
                "{writer}: {label}では書かず {expected} で断ること: {result:?}"
            );
        }
        assert_eq!(
            owned_counts(&store, new).await,
            before_new,
            "{label}: 新しいキャラクターに1行も入らないこと"
        );
        assert_eq!(
            owned_counts(&store, akane).await,
            before_akane,
            "{label}: 会話の持ち主の側にも1行も入らないこと"
        );
    }

    // **対照: 新しいキャラクター自身の会話なら、どれも書ける。**
    store
        .add_memory_in_session(new, new_session, &memory("自分の会話の記憶"))
        .await
        .expect("記憶を書けること");
    assert_eq!(
        store
            .apply_stat_drift_in_session(new, new_session, &change, permit())
            .await
            .expect("値を書けること")
            .label(),
        "moved",
        "値が動くこと"
    );
    store
        .record_drift_vote(new, new_session, &ballot)
        .await
        .expect("票を書けること");
    store
        .save_drift_charges_in_session(new, new_session, &[(axis, 0.5)], permit())
        .await
        .expect("累積を書けること");
    store
        .record_axis_candidate_in_session(
            new,
            new_session,
            &AxisCandidateRecord::Propose {
                name: StatAxis::parse("候補").expect("正当な名前"),
                source_memory: source,
            },
        )
        .await
        .expect("候補を書けること");
    store
        .record_axis_candidate_in_session(
            new,
            new_session,
            &AxisCandidateRecord::TrialVote {
                candidate: on_trial,
                session: new_session,
                vote: AxisTrialVote::Up,
            },
        )
        .await
        .expect("試用の票を書けること");
    store
        .record_axis_candidate_in_session(
            new,
            new_session,
            &AxisCandidateRecord::Advance {
                candidate: on_trial,
                to: AxisCandidateState::Passed,
            },
        )
        .await
        .expect("判定を書けること");
    store
        .record_axis_discovery_miss_in_session(
            new,
            new_session,
            source,
            AxisDiscoveryMiss::Malformed,
        )
        .await
        .expect("試みを書けること");
}

fn memory(content: &str) -> NewMemory {
    NewMemory {
        subject: Subject::User,
        content: MemoryContent::parse(content).expect("正当な本文"),
        category: None,
    }
}

/// **しまっただけのあいだは、識別子は使用中のまま**（ADR-0049 §決定1）。
#[tokio::test]
async fn a_put_away_handle_is_still_taken() {
    let (_dir, store) = setup().await;
    create(&store, "aoi").await;
    store
        .update_character(&name("aoi"), &put_away())
        .await
        .expect("しまえること");

    let err = store
        .create_character(
            &name("aoi"),
            &DisplayName::parse("葵").expect("正当な表示名"),
            &Persona::parse("別のペルソナ。").expect("正当なペルソナ"),
            None,
            fixture_scale(),
        )
        .await
        .expect_err("しまってあるだけなので作れないこと");
    assert!(
        err.is_unique_violation(),
        "重複として拒否されること: {err:?}"
    );
}

/// **消す要求と戻す要求を同時に走らせても、両方は成功しない**（判定と削除が1文 / ADR-0049 §決定1）。
///
/// 数えてから消す（しまっているかを読んでから `DELETE` する）と、その隙に戻されたキャラクターを
/// **会話ごと消す**——戻したはずの会話・記憶が、エラーも無く消える。
///
/// * 削除が先に確定 → 戻す要求は `UnknownCharacter`。
/// * 戻すのが先に確定 → 削除は `CharacterHasSessions`（会話があり、もうしまっていない）。
///
/// **正しい実装では偽陽性にならない**（両方成功する経路が無い）。
/// 競合が毎回起きるとも限らないので、回数を減らさないこと。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleting_never_races_a_restore() {
    let (_dir, store) = setup().await;

    let mut restored = 0_u32;
    let mut deleted = 0_u32;
    for round in 0..64 {
        let handle = format!("aoi-{round}");
        let id = create(&store, &handle).await;
        store.create_session(id).await.expect("会話を作れること");
        store
            .update_character(&name(&handle), &put_away())
            .await
            .expect("しまえること");

        let restoring = {
            let store = store.clone();
            let target = name(&handle);
            tokio::spawn(async move { store.update_character(&target, &restore()).await })
        };
        let deleting = {
            let store = store.clone();
            let target = name(&handle);
            tokio::spawn(async move { store.delete_character(&target).await })
        };

        let restore_result = restoring.await.expect("タスクが panic しないこと");
        let delete_result = deleting.await.expect("タスクが panic しないこと");

        assert!(
            !(restore_result.is_ok() && delete_result.is_ok()),
            "戻したキャラクターを消してはならない（{round} 周目）"
        );
        if restore_result.is_ok() {
            restored += 1;
            let sessions: i64 =
                sqlx::query_scalar("SELECT count(*) FROM session WHERE character_id = ?")
                    .bind(id.get())
                    .fetch_one(store.pool())
                    .await
                    .expect("行数を数えられること");
            assert_eq!(
                sessions, 1,
                "戻したなら会話は残っていること（{round} 周目）"
            );
        }
        if delete_result.is_ok() {
            deleted += 1;
        }
    }
    assert_eq!(restored + deleted, 64, "毎周どちらか一方は成功すること");
}

// ======================================================================
// 行を置く・数える
// ======================================================================

/// **キャラクターが持ちうる行を、全部の表に置く**（`tag` は隣と値を分けるため）。
///
/// `store` に書き口の無い表（要約・挿絵・外見）もあるので、全部を直接の INSERT で置く。
/// 記憶は**訂正の鎖**（古い行が新しい行を指す）にしておく —— 自己参照の外部キーが
/// カスケードを止めないことも、この形で見る。
async fn populate(store: &Store, character: CharacterId, tag: &str) {
    let pool = store.pool();
    let id = character.get();

    let exec = |sql: &'static str| sqlx::query(sql);

    exec("INSERT INTO stat_alloc (character_id, axis, value) VALUES (?, ?, 3)")
        .bind(id)
        .bind(format!("軸{tag}"))
        .execute(pool)
        .await
        .expect("stat_alloc");
    exec("INSERT INTO character_appearance (character_id, fixed_tags) VALUES (?, ?)")
        .bind(id)
        .bind(format!("tags-{tag}"))
        .execute(pool)
        .await
        .expect("character_appearance");
    exec("INSERT INTO character_attribute (character_id, subject, key, value) VALUES (?, 'character', ?, ?)")
        .bind(id)
        .bind(format!("鍵{tag}"))
        .bind(format!("値{tag}"))
        .execute(pool)
        .await
        .expect("character_attribute");
    exec("INSERT INTO attribute_revision (character_id, subject, key, before, after) VALUES (?, 'character', ?, NULL, ?)")
        .bind(id)
        .bind(format!("鍵{tag}"))
        .bind(format!("値{tag}"))
        .execute(pool)
        .await
        .expect("attribute_revision");
    exec("INSERT INTO stat_drift_charge (character_id, axis, charge) VALUES (?, ?, 0.5)")
        .bind(id)
        .bind(format!("軸{tag}"))
        .execute(pool)
        .await
        .expect("stat_drift_charge");

    let session: i64 =
        sqlx::query_scalar("INSERT INTO session (character_id) VALUES (?) RETURNING id")
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("session");
    let message: i64 = sqlx::query_scalar(
        "INSERT INTO message (session_id, role, raw_content) VALUES (?, 'user', ?) RETURNING id",
    )
    .bind(session)
    .bind(format!("こんにちは{tag}"))
    .fetch_one(pool)
    .await
    .expect("message");
    exec("INSERT INTO summary (session_id, upto_message_id, content) VALUES (?, ?, ?)")
        .bind(session)
        .bind(message)
        .bind(format!("要約{tag}"))
        .execute(pool)
        .await
        .expect("summary");
    exec("INSERT INTO generated_image (session_id, message_id, positive_prompt, negative_prompt, seed, checkpoint, file_path) \
          VALUES (?, ?, 'p', 'n', 1, 'c', ?)")
        .bind(session)
        .bind(message)
        .bind(format!("image-{tag}.png"))
        .execute(pool)
        .await
        .expect("generated_image");
    exec("INSERT INTO stat_drift_vote (character_id, session_id, axis, delta) VALUES (?, ?, ?, 'up')")
        .bind(id)
        .bind(session)
        .bind(format!("軸{tag}"))
        .execute(pool)
        .await
        .expect("stat_drift_vote");

    // 記憶は訂正の鎖にする（古い行 → 新しい行）。
    let newer: i64 = sqlx::query_scalar(
        "INSERT INTO memory (character_id, subject, content) VALUES (?, 'user', ?) RETURNING id",
    )
    .bind(id)
    .bind(format!("新しい記憶{tag}"))
    .fetch_one(pool)
    .await
    .expect("memory");
    exec("INSERT INTO memory (character_id, subject, content, superseded_by) VALUES (?, 'user', ?, ?)")
        .bind(id)
        .bind(format!("古い記憶{tag}"))
        .bind(newer)
        .execute(pool)
        .await
        .expect("memory（上書きされた側）");

    let candidate: i64 = sqlx::query_scalar(
        "INSERT INTO stat_axis_candidate (character_id, name, source_memory_id) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(id)
    .bind(format!("候補{tag}"))
    .bind(newer)
    .fetch_one(pool)
    .await
    .expect("stat_axis_candidate");
    exec("INSERT INTO stat_axis_candidate_transition (candidate_id, state) VALUES (?, 'trial')")
        .bind(candidate)
        .execute(pool)
        .await
        .expect("stat_axis_candidate_transition");
    exec("INSERT INTO stat_axis_trial_vote (candidate_id, session_id, vote) VALUES (?, ?, 'up')")
        .bind(candidate)
        .bind(session)
        .execute(pool)
        .await
        .expect("stat_axis_trial_vote");
    exec("INSERT INTO stat_axis_discovery_attempt (character_id, through_memory_id, outcome) VALUES (?, ?, 'proposed')")
        .bind(id)
        .bind(newer)
        .execute(pool)
        .await
        .expect("stat_axis_discovery_attempt");

    // **取り決めと、その履歴**（ADR-0051 / 作業単位 A-12a）。出どころの会話も指しておく
    // —— 会話と取り決めが同じ文のカスケードで消えるとき、出どころの外部キーが止めないこと。
    let agreement: i64 = sqlx::query_scalar(
        "INSERT INTO character_agreement (character_id, content, kind, keeper, state, source_session_id) \
         VALUES (?, ?, 'promise', 'both', 'active', ?) RETURNING id",
    )
    .bind(id)
    .bind(format!("約束{tag}"))
    .bind(session)
    .fetch_one(pool)
    .await
    .expect("character_agreement");
    exec(
        "INSERT INTO agreement_transition (agreement_id, before, after) VALUES (?, NULL, 'active')",
    )
    .bind(agreement)
    .execute(pool)
    .await
    .expect("agreement_transition");
}

/// **そのキャラクターのものである行の数**を、表ごとに数える文（`(表, 文)`。表の名前順）。
///
/// 表の名前はバインドできないので、**文字列を連結せずリテラルの SQL を並べる。**
/// キャラクターを直接指さない表は、指す表を辿って数える（会話・候補を経由する）。
const OWNED_ROWS: [(&str, &str); 18] = [
    (
        "agreement_transition",
        "SELECT count(*) FROM agreement_transition \
         JOIN character_agreement ON character_agreement.id = agreement_transition.agreement_id \
         WHERE character_agreement.character_id = ?",
    ),
    (
        "attribute_revision",
        "SELECT count(*) FROM attribute_revision WHERE character_id = ?",
    ),
    ("character", "SELECT count(*) FROM character WHERE id = ?"),
    (
        "character_agreement",
        "SELECT count(*) FROM character_agreement WHERE character_id = ?",
    ),
    (
        "character_appearance",
        "SELECT count(*) FROM character_appearance WHERE character_id = ?",
    ),
    (
        "character_attribute",
        "SELECT count(*) FROM character_attribute WHERE character_id = ?",
    ),
    (
        "generated_image",
        "SELECT count(*) FROM generated_image \
         JOIN session ON session.id = generated_image.session_id WHERE session.character_id = ?",
    ),
    (
        "memory",
        "SELECT count(*) FROM memory WHERE character_id = ?",
    ),
    (
        "message",
        "SELECT count(*) FROM message \
         JOIN session ON session.id = message.session_id WHERE session.character_id = ?",
    ),
    (
        "session",
        "SELECT count(*) FROM session WHERE character_id = ?",
    ),
    (
        "stat_alloc",
        "SELECT count(*) FROM stat_alloc WHERE character_id = ?",
    ),
    (
        "stat_axis_candidate",
        "SELECT count(*) FROM stat_axis_candidate WHERE character_id = ?",
    ),
    (
        "stat_axis_candidate_transition",
        "SELECT count(*) FROM stat_axis_candidate_transition \
         JOIN stat_axis_candidate ON stat_axis_candidate.id = stat_axis_candidate_transition.candidate_id \
         WHERE stat_axis_candidate.character_id = ?",
    ),
    (
        "stat_axis_discovery_attempt",
        "SELECT count(*) FROM stat_axis_discovery_attempt WHERE character_id = ?",
    ),
    (
        "stat_axis_trial_vote",
        "SELECT count(*) FROM stat_axis_trial_vote \
         JOIN stat_axis_candidate ON stat_axis_candidate.id = stat_axis_trial_vote.candidate_id \
         WHERE stat_axis_candidate.character_id = ?",
    ),
    (
        "stat_drift_charge",
        "SELECT count(*) FROM stat_drift_charge WHERE character_id = ?",
    ),
    (
        "stat_drift_vote",
        "SELECT count(*) FROM stat_drift_vote WHERE character_id = ?",
    ),
    (
        "summary",
        "SELECT count(*) FROM summary \
         JOIN session ON session.id = summary.session_id WHERE session.character_id = ?",
    ),
];

/// 表全体の行数（`(表, 文)`。[`OWNED_ROWS`] と同じ表・同じ順）。
const TOTAL_ROWS: [(&str, &str); 18] = [
    (
        "agreement_transition",
        "SELECT count(*) FROM agreement_transition",
    ),
    (
        "attribute_revision",
        "SELECT count(*) FROM attribute_revision",
    ),
    ("character", "SELECT count(*) FROM character"),
    (
        "character_agreement",
        "SELECT count(*) FROM character_agreement",
    ),
    (
        "character_appearance",
        "SELECT count(*) FROM character_appearance",
    ),
    (
        "character_attribute",
        "SELECT count(*) FROM character_attribute",
    ),
    ("generated_image", "SELECT count(*) FROM generated_image"),
    ("memory", "SELECT count(*) FROM memory"),
    ("message", "SELECT count(*) FROM message"),
    ("session", "SELECT count(*) FROM session"),
    ("stat_alloc", "SELECT count(*) FROM stat_alloc"),
    (
        "stat_axis_candidate",
        "SELECT count(*) FROM stat_axis_candidate",
    ),
    (
        "stat_axis_candidate_transition",
        "SELECT count(*) FROM stat_axis_candidate_transition",
    ),
    (
        "stat_axis_discovery_attempt",
        "SELECT count(*) FROM stat_axis_discovery_attempt",
    ),
    (
        "stat_axis_trial_vote",
        "SELECT count(*) FROM stat_axis_trial_vote",
    ),
    (
        "stat_drift_charge",
        "SELECT count(*) FROM stat_drift_charge",
    ),
    ("stat_drift_vote", "SELECT count(*) FROM stat_drift_vote"),
    ("summary", "SELECT count(*) FROM summary"),
];

/// 表ごとの「そのキャラクターの行の数」（[`OWNED_ROWS`] の順）。
async fn owned_counts(store: &Store, character: CharacterId) -> Vec<(&'static str, i64)> {
    let mut counts = Vec::with_capacity(OWNED_ROWS.len());
    for (table, sql) in OWNED_ROWS {
        let count: i64 = sqlx::query_scalar(sql)
            .bind(character.get())
            .fetch_one(store.pool())
            .await
            .unwrap_or_else(|err| panic!("{table} を数えられること: {err}"));
        counts.push((table, count));
    }
    counts
}

fn count_in(counts: &[(&'static str, i64)], table: &str) -> i64 {
    counts
        .iter()
        .find(|(name, _)| *name == table)
        .map(|(_, count)| *count)
        .unwrap_or_else(|| panic!("{table} を数えていること"))
}

/// **数える表の集合が、実際の表の集合と一致する。**
async fn assert_owned_rows_cover_every_table(store: &Store) {
    let rows = sqlx::query(
        "SELECT name FROM sqlite_master \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> '_sqlx_migrations' \
         ORDER BY name",
    )
    .fetch_all(store.pool())
    .await
    .expect("表の一覧を取得できること");
    let actual: Vec<String> = rows
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();

    let owned: Vec<String> = OWNED_ROWS
        .iter()
        .map(|(table, _)| (*table).to_owned())
        .collect();
    let total: Vec<String> = TOTAL_ROWS
        .iter()
        .map(|(table, _)| (*table).to_owned())
        .collect();
    assert_eq!(
        owned, actual,
        "数える表が実際の表と一致すること（表を足したら、その表も消えてよいかを決めて足すこと）"
    );
    assert_eq!(total, actual, "全体を数える表も同じであること");
}
