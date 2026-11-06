//! **軸の候補を尋ねた「試み」の記録**の検証（[ADR-0046](../docs/adr/0046-drift-proposes-new-axes.md)
//! §決定2 / 作業単位 A-9b / Issue origin#42 の申し送り1）。
//!
//! ここで固定するのは:
//!
//!   * **目印は、まだ一度も試みていなければ `None`。**
//!   * **採らなかった試みも、採った試みも目印を進める**（採った回は候補と同じトランザクション）。
//!   * **拒まれた候補（枠が無い）は、目印を進めない**（何も書かれない）。
//!   * **目印は後退しない**（最大値）・**キャラクターごと**・**他のキャラクターの記憶は指せない。**
//!
//! 「目印から先を数えて尋ねるか決める」は `server::axis_discovery` の単体テストが持つ。

use personacore::domain::{
    AxisCandidateRecord, AxisDiscoveryMiss, AxisDiscoverySlots, CharacterId, CharacterUpdate,
    DisplayName, Handle, MemoryContent, MemoryId, NewMemory, Persona, StatAxis, Subject,
};
use personacore::store::{Store, StoreError};
use tempfile::TempDir;

mod common;

use common::fixture_scale;

async fn setup() -> (TempDir, Store) {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let store = Store::connect(dir.path().join("personacore.sqlite"))
        .await
        .expect("接続とマイグレーション適用が成功すること");
    (dir, store)
}

async fn character(store: &Store, handle: &str, display_name: &str) -> CharacterId {
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

async fn set_slots(store: &Store, handle: &str, slots: u32) {
    store
        .update_character(
            &Handle::parse(handle).expect("正当な識別子であること"),
            &CharacterUpdate {
                axis_discovery_slots: Some(
                    AxisDiscoverySlots::parse(slots).expect("正当な枠の数であること"),
                ),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("枠の数を書けること");
}

async fn memory(store: &Store, id: CharacterId, content: &str) -> MemoryId {
    store
        .add_memory(
            id,
            &NewMemory {
                subject: Subject::Character,
                content: MemoryContent::parse(content).expect("正当な本文であること"),
                category: None,
            },
        )
        .await
        .expect("記憶を追加できること")
}

async fn attempts(store: &Store) -> Vec<(i64, i64, String)> {
    sqlx::query_as(
        "SELECT character_id, through_memory_id, outcome FROM stat_axis_discovery_attempt ORDER BY id",
    )
    .fetch_all(store.pool())
    .await
    .expect("試みを読めること")
}

/// **採らなかった試みも、採った試みも目印を進め、結末の綴りで残る。**
///
/// 記憶を3件作り、**真ん中**を採らなかった試みの目印に、**最後**を採った試みの出どころにする。
#[tokio::test]
async fn both_kinds_of_attempt_move_the_mark_and_keep_their_outcome() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    set_slots(&store, "aoi", 2).await;
    let first = memory(&store, aoi, "一つ目").await;
    let second = memory(&store, aoi, "二つ目").await;
    let third = memory(&store, aoi, "三つ目").await;
    assert!(
        first < second && second < third,
        "前提: id が作った順に増える"
    );

    assert_eq!(
        store.axis_discovery_mark(aoi).await.expect("読めること"),
        None,
        "まだ一度も試みていない"
    );

    store
        .record_axis_discovery_miss(aoi, second, AxisDiscoveryMiss::Malformed)
        .await
        .expect("採らなかった試みを書けること");
    assert_eq!(
        store.axis_discovery_mark(aoi).await.expect("読めること"),
        Some(second)
    );

    store
        .record_axis_candidate(
            aoi,
            &AxisCandidateRecord::Propose {
                name: StatAxis::parse("照れ屋").expect("正当な軸名"),
                source_memory: third,
            },
        )
        .await
        .expect("候補を作れること");
    assert_eq!(
        store.axis_discovery_mark(aoi).await.expect("読めること"),
        Some(third),
        "候補を採った回も目印を進める"
    );

    assert_eq!(
        attempts(&store).await,
        vec![
            (aoi.get(), second.get(), "malformed".to_owned()),
            (aoi.get(), third.get(), "proposed".to_owned()),
        ]
    );
}

/// **拒まれた候補（空いた枠が無い）は、試みの表にも何も書かない**（候補と同じトランザクション）。
#[tokio::test]
async fn a_refused_proposal_leaves_no_attempt_behind() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    // **m = 0**（既定）。
    let only = memory(&store, aoi, "一つ目").await;

    let refused = store
        .record_axis_candidate(
            aoi,
            &AxisCandidateRecord::Propose {
                name: StatAxis::parse("照れ屋").expect("正当な軸名"),
                source_memory: only,
            },
        )
        .await;
    assert!(
        matches!(refused, Err(StoreError::AxisDiscoverySlotsFull)),
        "{refused:?}"
    );
    assert_eq!(
        store.axis_discovery_mark(aoi).await.expect("読めること"),
        None
    );
    assert!(attempts(&store).await.is_empty());
}

/// **目印は後退しない**（古い記憶を指す試みが後から来ても、最大値のまま）。
#[tokio::test]
async fn the_mark_never_moves_backwards() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let older = memory(&store, aoi, "古い").await;
    let newer = memory(&store, aoi, "新しい").await;

    store
        .record_axis_discovery_miss(aoi, newer, AxisDiscoveryMiss::Duplicate)
        .await
        .expect("書けること");
    store
        .record_axis_discovery_miss(aoi, older, AxisDiscoveryMiss::UpstreamFailed)
        .await
        .expect("書けること");
    assert_eq!(
        store.axis_discovery_mark(aoi).await.expect("読めること"),
        Some(newer)
    );
}

/// **目印はキャラクターごと。他のキャラクターの記憶は指せない**（複合外部キー）。
///
/// **会話しない方を先に作る**（`id` が 1 のキャラクターを取り違える実装を観測できる形）。
#[tokio::test]
async fn the_mark_belongs_to_one_character_and_cannot_point_elsewhere() {
    let (_dir, store) = setup().await;
    let kaede = character(&store, "kaede", "楓").await;
    let aoi = character(&store, "aoi", "葵").await;
    let kaedes = memory(&store, kaede, "楓の記憶").await;
    let aois = memory(&store, aoi, "葵の記憶").await;

    store
        .record_axis_discovery_miss(aoi, aois, AxisDiscoveryMiss::Truncated)
        .await
        .expect("書けること");
    assert_eq!(
        store.axis_discovery_mark(kaede).await.expect("読めること"),
        None,
        "他のキャラクターの試みは目印にならない"
    );

    let crossed = store
        .record_axis_discovery_miss(aoi, kaedes, AxisDiscoveryMiss::NotRecorded)
        .await;
    assert!(matches!(crossed, Err(StoreError::Query(_))), "{crossed:?}");
    assert_eq!(
        store.axis_discovery_mark(aoi).await.expect("読めること"),
        Some(aois),
        "拒まれた試みは何も書かない"
    );
}

/// **結末の綴りは SQL の CHECK に全部通る**（`AxisDiscoveryOutcome::label` の写しが古びていない）。
#[tokio::test]
async fn every_miss_spelling_is_accepted_by_the_table() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let only = memory(&store, aoi, "一つ目").await;
    for miss in [
        AxisDiscoveryMiss::Duplicate,
        AxisDiscoveryMiss::Malformed,
        AxisDiscoveryMiss::Truncated,
        AxisDiscoveryMiss::UpstreamFailed,
        AxisDiscoveryMiss::NotRecorded,
    ] {
        store
            .record_axis_discovery_miss(aoi, only, miss)
            .await
            .unwrap_or_else(|err| panic!("{miss:?} を書けること: {err:?}"));
    }
    let outcomes: Vec<String> = attempts(&store)
        .await
        .into_iter()
        .map(|(_, _, outcome)| outcome)
        .collect();
    assert_eq!(
        outcomes,
        vec![
            "duplicate",
            "malformed",
            "truncated",
            "upstream_failed",
            "not_recorded"
        ]
    );
}
