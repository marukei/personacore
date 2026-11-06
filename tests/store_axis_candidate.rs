//! **軸の候補の永続化**の検証（[ADR-0046](../docs/adr/0046-drift-proposes-new-axes.md) §決定1・§決定6 /
//! 作業単位 A-9a / Issue origin#41）。
//!
//! ここで固定するのは主に次の点:
//!
//!   * **枠の数 `m` は既定 0 で、`update_character` で書けて、読む2経路の両方に出る。**
//!   * **候補は空いた枠があるときだけ作れる**（`m` = 0 では1件も作れない）。
//!   * **状態は追記で進み、いまの状態は導出**（3件以上の真ん中の候補で見る）。
//!     拒まれた遷移は1行も書かない。
//!   * **試用の票は候補ごと・種類ごとに数えられ、試用中にだけ書ける。**
//!     **変容の票の表には1行も入らない**（ADR-0046 §決定6）。
//!   * **他のキャラクターの候補は、読めも書けもしない。**
//!   * **書き手は `BEGIN IMMEDIATE`**（同時の2件が枠を溢れさせず、別のキャラクター同士は両方通る）。
//!
//! スキーマ制約（CHECK・外部キー・既存 DB への移行）は `tests/store_schema.rs` にある。

use personacore::domain::{
    AxisCandidateId, AxisCandidateRecord, AxisCandidateState, AxisDiscoverySlots, AxisTrialFailure,
    AxisTrialTally, AxisTrialVote, CharacterEntry, CharacterId, CharacterUpdate, DisplayName,
    Handle, MemoryContent, MemoryId, NewMemory, Persona, SessionId, StatAxis, Subject,
};
use personacore::store::{AxisCandidate, Store, StoreError};
use tempfile::TempDir;

mod common;

use common::fixture_scale;

/// 一時ディレクトリ上に新規 DB を作り、マイグレーション適用済みの `Store` を返す。
async fn setup() -> (TempDir, Store) {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let store = Store::connect(dir.path().join("personacore.sqlite"))
        .await
        .expect("接続とマイグレーション適用が成功すること");
    (dir, store)
}

/// キャラクターを1件作って id を返す。
async fn character(store: &Store, handle: &str, display_name: &str) -> CharacterId {
    assert_ne!(handle, display_name, "識別子と表示名を同値にしないこと");
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

/// 枠の数を書く（`update_character` が唯一の書き手）。
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

/// そのキャラクター自身の記憶を1件作る（候補の出どころ / ADR-0046 §決定2）。
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

fn axis(name: &str) -> StatAxis {
    StatAxis::parse(name).expect("正当な軸名であること")
}

/// 候補を作る。
async fn propose(
    store: &Store,
    id: CharacterId,
    name: &str,
    source: MemoryId,
) -> Result<AxisCandidateId, StoreError> {
    store
        .record_axis_candidate(
            id,
            &AxisCandidateRecord::Propose {
                name: axis(name),
                source_memory: source,
            },
        )
        .await
}

/// 状態を1つ進める。
async fn advance(
    store: &Store,
    id: CharacterId,
    candidate: AxisCandidateId,
    to: AxisCandidateState,
) -> Result<AxisCandidateId, StoreError> {
    store
        .record_axis_candidate(id, &AxisCandidateRecord::Advance { candidate, to })
        .await
}

/// 試用の票を1件足す。
async fn vote(
    store: &Store,
    id: CharacterId,
    candidate: AxisCandidateId,
    session: SessionId,
    vote: AxisTrialVote,
) -> Result<AxisCandidateId, StoreError> {
    store
        .record_axis_candidate(
            id,
            &AxisCandidateRecord::TrialVote {
                candidate,
                session,
                vote,
            },
        )
        .await
}

async fn candidates(store: &Store, id: CharacterId) -> Vec<AxisCandidate> {
    store
        .axis_candidates(id, 100)
        .await
        .expect("候補を読めること")
}

/// 生の行数（テーブル名はバインドできないので、リテラルの SQL だけを渡す）。
async fn count(store: &Store, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(store.pool())
        .await
        .expect("行数を数えられること")
}

/// 候補の「名前・いまの状態・移り変わり」を比べやすい形にする。
fn shape(candidate: &AxisCandidate) -> (String, AxisCandidateState, Vec<AxisCandidateState>) {
    (
        candidate.name().as_str().to_owned(),
        candidate.state(),
        candidate
            .transitions()
            .iter()
            .map(|transition| transition.state())
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// 枠の数
// ---------------------------------------------------------------------------

/// **作ったばかりのキャラクターは枠 0・候補 0**（ADR-0046 §決定1「足しただけでは何も起きない」）。
#[tokio::test]
async fn a_new_character_has_no_slots_and_no_candidates() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;

    let found = store
        .find_character_by_handle(&Handle::parse("aoi").expect("正当な識別子"))
        .await
        .expect("読めること")
        .expect("引けること");
    assert_eq!(found.axis_discovery_slots, AxisDiscoverySlots::NONE);
    assert!(candidates(&store, aoi).await.is_empty());
    assert_eq!(
        store
            .count_axis_candidates(aoi)
            .await
            .expect("数えられること"),
        0
    );
}

/// **枠の数は `update_character` で書け、1件読みと一覧の両方に出る。触らない更新では変わらない。**
///
/// 値は既定（0）とも上限（4）とも違う 3 で見る——0 だと「読まずに既定を返す」実装が、
/// 4 だと「上限を返す」実装が緑になる。
#[tokio::test]
async fn slots_round_trip_through_the_update_and_both_readers() {
    let (_dir, store) = setup().await;
    character(&store, "aoi", "葵").await;
    character(&store, "rin", "凛").await;
    set_slots(&store, "aoi", 3).await;

    // **触らない更新（尺度だけを変える）では変わらない。**
    store
        .update_character(
            &Handle::parse("aoi").expect("正当な識別子"),
            &CharacterUpdate {
                display_name: Some(DisplayName::parse("あおい").expect("正当な表示名")),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("更新できること");

    let found = store
        .find_character_by_handle(&Handle::parse("aoi").expect("正当な識別子"))
        .await
        .expect("読めること")
        .expect("引けること");
    assert_eq!(found.axis_discovery_slots.get(), 3, "1件読み");
    assert_eq!(
        found.display_name.as_str(),
        "あおい",
        "他の列は書けていること"
    );

    let listed: Vec<(String, u32)> = store
        .list_characters()
        .await
        .expect("一覧を読めること")
        .into_iter()
        .map(|entry| match entry {
            CharacterEntry::Readable(character) => (
                character.handle.as_str().to_owned(),
                character.axis_discovery_slots.get(),
            ),
            CharacterEntry::Unreadable(broken) => panic!("読めない行: {broken:?}"),
        })
        .collect();
    assert_eq!(
        listed,
        vec![("aoi".to_owned(), 3), ("rin".to_owned(), 0)],
        "一覧でも、書いたキャラクターだけが 3 であること"
    );
}

// ---------------------------------------------------------------------------
// 候補を作る
// ---------------------------------------------------------------------------

/// **`m` = 0 では候補を作れない。何も書かれない。**
#[tokio::test]
async fn no_candidate_can_be_proposed_without_a_slot() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let source = memory(&store, aoi, "夜に星を見るのが好き").await;

    let err = propose(&store, aoi, "好奇心", source)
        .await
        .expect_err("枠が無ければ作れないこと");
    assert!(
        matches!(err, StoreError::AxisDiscoverySlotsFull),
        "実際 = {err:?}"
    );
    assert_eq!(
        count(&store, "SELECT count(*) FROM stat_axis_candidate").await,
        0
    );
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM stat_axis_candidate_transition"
        )
        .await,
        0
    );
}

/// **空いた枠の分だけ作れ、不合格と却下は枠を空ける。**
///
/// 枠を埋める状態（試用・合格・採用）と空ける状態（不合格・却下）を両方通る。
#[tokio::test]
async fn failed_and_rejected_candidates_free_their_slot() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let source = memory(&store, aoi, "夜に星を見るのが好き").await;
    set_slots(&store, "aoi", 1).await;

    let first = propose(&store, aoi, "好奇心", source)
        .await
        .expect("1つ目は作れること");
    let err = propose(&store, aoi, "几帳面さ", source)
        .await
        .expect_err("試用中の候補が枠を埋めていること");
    assert!(matches!(err, StoreError::AxisDiscoverySlotsFull), "{err:?}");

    advance(&store, aoi, first, AxisCandidateState::Passed)
        .await
        .expect("合格にできること");
    let err = propose(&store, aoi, "几帳面さ", source)
        .await
        .expect_err("合格した候補も枠を埋めていること");
    assert!(matches!(err, StoreError::AxisDiscoverySlotsFull), "{err:?}");

    advance(&store, aoi, first, AxisCandidateState::Rejected)
        .await
        .expect("却下できること");
    let second = propose(&store, aoi, "几帳面さ", source)
        .await
        .expect("却下で枠が空くこと");

    advance(
        &store,
        aoi,
        second,
        AxisCandidateState::Failed(AxisTrialFailure::Pinned),
    )
    .await
    .expect("不合格にできること");
    propose(&store, aoi, "茶目っ気", source)
        .await
        .expect("不合格で枠が空くこと");
}

/// **同じキャラクターの中で同じ名前は二度使えない**（ADR-0046 §決定2・5）。**他のキャラクターは使える。**
///
/// 却下した後でも使えない——却下・不合格の名前を二度出させないため。
#[tokio::test]
async fn a_name_is_never_used_twice_by_the_same_character() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let rin = character(&store, "rin", "凛").await;
    let aoi_source = memory(&store, aoi, "夜に星を見るのが好き").await;
    let rin_source = memory(&store, rin, "朝は紅茶を淹れる").await;
    set_slots(&store, "aoi", 2).await;
    set_slots(&store, "rin", 1).await;

    let first = propose(&store, aoi, "好奇心", aoi_source)
        .await
        .expect("作れること");
    advance(
        &store,
        aoi,
        first,
        AxisCandidateState::Failed(AxisTrialFailure::Silent),
    )
    .await
    .expect("不合格にできること");

    let err = propose(&store, aoi, "好奇心", aoi_source)
        .await
        .expect_err("不合格の名前はもう使えないこと");
    assert!(err.is_unique_violation(), "UNIQUE で止まること: {err:?}");

    propose(&store, rin, "好奇心", rin_source)
        .await
        .expect("他のキャラクターは同じ名前を使えること");
}

/// **出どころの記憶は、そのキャラクターのものに限る**（複合外部キー）。
#[tokio::test]
async fn the_source_memory_must_belong_to_the_same_character() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let rin = character(&store, "rin", "凛").await;
    let rin_source = memory(&store, rin, "朝は紅茶を淹れる").await;
    set_slots(&store, "aoi", 1).await;

    let err = propose(&store, aoi, "好奇心", rin_source)
        .await
        .expect_err("他のキャラクターの記憶は出どころにできないこと");
    assert!(matches!(err, StoreError::Query(_)), "{err:?}");
    assert_eq!(
        count(&store, "SELECT count(*) FROM stat_axis_candidate").await,
        0
    );
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM stat_axis_candidate_transition"
        )
        .await,
        0,
        "候補の行が落ちたら、試用の移り変わりも残らないこと（同じトランザクション）"
    );
}

// ---------------------------------------------------------------------------
// 状態の移り変わり
// ---------------------------------------------------------------------------

/// **状態は追記で進み、いまの状態は移り変わりの列から導かれる**（Issue origin#41 の閉じる条件）。
///
/// # 3件にして真ん中を見る
///
/// 端の候補だけを見ると、「先頭（最新）の状態を全員に配る」「末尾だけ正しい」実装が緑になる。
/// 真ん中の候補（几帳面さ）だけを採用まで進め、両隣を別の状態に置く。
///
/// # 行を書き換えていないことも生の行で見る
///
/// 導出が正しくても、**書き手が行を UPDATE して結果だけ合わせている**なら閉じる条件を満たさない。
/// 移り変わりの行数が遷移の回数と一致し、最初の行がまだ試用のままであることを見る。
#[tokio::test]
async fn the_current_state_is_derived_from_appended_transitions() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let source = memory(&store, aoi, "夜に星を見るのが好き").await;
    set_slots(&store, "aoi", 3).await;

    let oldest = propose(&store, aoi, "好奇心", source)
        .await
        .expect("作れること");
    let middle = propose(&store, aoi, "几帳面さ", source)
        .await
        .expect("作れること");
    let newest = propose(&store, aoi, "茶目っ気", source)
        .await
        .expect("作れること");

    advance(&store, aoi, middle, AxisCandidateState::Passed)
        .await
        .expect("合格にできること");
    advance(&store, aoi, middle, AxisCandidateState::Adopted)
        .await
        .expect("採用にできること");
    advance(
        &store,
        aoi,
        oldest,
        AxisCandidateState::Failed(AxisTrialFailure::TooFewSamples),
    )
    .await
    .expect("不合格にできること");

    let listed = candidates(&store, aoi).await;
    assert_eq!(
        listed.iter().map(shape).collect::<Vec<_>>(),
        vec![
            (
                "茶目っ気".to_owned(),
                AxisCandidateState::Trial,
                vec![AxisCandidateState::Trial],
            ),
            (
                "几帳面さ".to_owned(),
                AxisCandidateState::Adopted,
                vec![
                    AxisCandidateState::Trial,
                    AxisCandidateState::Passed,
                    AxisCandidateState::Adopted,
                ],
            ),
            (
                "好奇心".to_owned(),
                AxisCandidateState::Failed(AxisTrialFailure::TooFewSamples),
                vec![
                    AxisCandidateState::Trial,
                    AxisCandidateState::Failed(AxisTrialFailure::TooFewSamples),
                ],
            ),
        ],
        "新しい順に、それぞれの移り変わりといまの状態が載ること"
    );
    assert_eq!(
        listed.iter().map(AxisCandidate::id).collect::<Vec<_>>(),
        vec![newest, middle, oldest],
        "書き手が返した id と一致すること"
    );
    assert!(
        listed
            .iter()
            .all(|candidate| candidate.source_memory() == source),
        "出どころの記憶を落とさないこと"
    );

    // **追記であって書き換えではない。** 3件の試用 + 合格 + 採用 + 不合格 = 6 行。
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM stat_axis_candidate_transition"
        )
        .await,
        6
    );
    let middle_rows: Vec<String> = sqlx::query_scalar(
        "SELECT state FROM stat_axis_candidate_transition WHERE candidate_id = ? ORDER BY id",
    )
    .bind(middle.get())
    .fetch_all(store.pool())
    .await
    .expect("移り変わりの生の行を読めること");
    assert_eq!(
        middle_rows,
        vec!["trial", "passed", "adopted"],
        "最初の行が試用のまま残っていること（書き換えていない）"
    );
}

/// **許されない遷移は拒まれ、1行も書かれない。**
#[tokio::test]
async fn a_refused_transition_writes_nothing() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let source = memory(&store, aoi, "夜に星を見るのが好き").await;
    set_slots(&store, "aoi", 2).await;
    let on_trial = propose(&store, aoi, "好奇心", source)
        .await
        .expect("作れること");
    let failed = propose(&store, aoi, "几帳面さ", source)
        .await
        .expect("作れること");
    advance(
        &store,
        aoi,
        failed,
        AxisCandidateState::Failed(AxisTrialFailure::Pinned),
    )
    .await
    .expect("不合格にできること");
    let before = count(
        &store,
        "SELECT count(*) FROM stat_axis_candidate_transition",
    )
    .await;

    for (candidate, to) in [
        (on_trial, AxisCandidateState::Adopted),
        (on_trial, AxisCandidateState::Rejected),
        (on_trial, AxisCandidateState::Trial),
        (failed, AxisCandidateState::Passed),
        (failed, AxisCandidateState::Trial),
    ] {
        let err = advance(&store, aoi, candidate, to)
            .await
            .expect_err("許されない遷移は拒まれること");
        assert!(
            matches!(err, StoreError::AxisCandidateTransitionRefused),
            "{to:?}: {err:?}"
        );
    }
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM stat_axis_candidate_transition"
        )
        .await,
        before,
        "拒まれた遷移は1行も書かないこと"
    );
    assert_eq!(
        shape(&candidates(&store, aoi).await[1]).1,
        AxisCandidateState::Trial,
        "試用のまま"
    );
}

// ---------------------------------------------------------------------------
// 試用の票
// ---------------------------------------------------------------------------

/// **票は候補ごと・種類ごとに数えられる。変容の票の表には入らない**（ADR-0046 §決定6）。
///
/// 2つの候補に**件数の違う**票を入れる——候補で絞らない実装・種類を取り違える実装は、
/// どちらかの内訳で必ず崩れる。
#[tokio::test]
async fn trial_votes_are_tallied_per_candidate_and_kept_apart_from_drift_votes() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let source = memory(&store, aoi, "夜に星を見るのが好き").await;
    let session = store.create_session(aoi).await.expect("会話を作れること");
    set_slots(&store, "aoi", 2).await;
    let curious = propose(&store, aoi, "好奇心", source)
        .await
        .expect("作れること");
    let tidy = propose(&store, aoi, "几帳面さ", source)
        .await
        .expect("作れること");

    for (candidate, kind, times) in [
        (curious, AxisTrialVote::Up, 2),
        (curious, AxisTrialVote::Down, 1),
        (curious, AxisTrialVote::Zero, 3),
        (tidy, AxisTrialVote::Zero, 5),
        (tidy, AxisTrialVote::Up, 4),
    ] {
        for _ in 0..times {
            vote(&store, aoi, candidate, session, kind)
                .await
                .expect("試用中の候補には票を足せること");
        }
    }

    let listed = candidates(&store, aoi).await;
    let tallies: Vec<(String, AxisTrialTally)> = listed
        .iter()
        .map(|candidate| (candidate.name().as_str().to_owned(), candidate.tally()))
        .collect();
    assert_eq!(
        tallies,
        vec![
            (
                "几帳面さ".to_owned(),
                AxisTrialTally {
                    up: 4,
                    down: 0,
                    zero: 5
                }
            ),
            (
                "好奇心".to_owned(),
                AxisTrialTally {
                    up: 2,
                    down: 1,
                    zero: 3
                }
            ),
        ]
    );
    assert_eq!(
        count(&store, "SELECT count(*) FROM stat_drift_vote").await,
        0,
        "変容の票の表には1行も入らないこと"
    );
}

/// **試用中でない候補には票を足せない。何も書かれない。**
#[tokio::test]
async fn votes_are_refused_once_the_trial_is_over() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let source = memory(&store, aoi, "夜に星を見るのが好き").await;
    let session = store.create_session(aoi).await.expect("会話を作れること");
    set_slots(&store, "aoi", 1).await;
    let curious = propose(&store, aoi, "好奇心", source)
        .await
        .expect("作れること");
    vote(&store, aoi, curious, session, AxisTrialVote::Up)
        .await
        .expect("試用中は足せること");
    advance(&store, aoi, curious, AxisCandidateState::Passed)
        .await
        .expect("合格にできること");

    let err = vote(&store, aoi, curious, session, AxisTrialVote::Down)
        .await
        .expect_err("合格した候補には足せないこと");
    assert!(
        matches!(err, StoreError::AxisCandidateNotOnTrial),
        "{err:?}"
    );
    assert_eq!(
        count(&store, "SELECT count(*) FROM stat_axis_trial_vote").await,
        1
    );
}

/// **試用の票の会話は、候補の持ち主の会話でなければならない**（Issue origin#43 の申し送り2 / 作業単位 A-9c）。
///
/// 外部キーは会話が**在る**ことしか見ないので、別のキャラクターの会話を渡しても書けていた。
/// 書き手が同じトランザクションで `session.character_id` を照合する。
///
/// 3体目（持ち主でも会話の持ち主でもないキャラクター）を作り、**会話の id とキャラクターの id が
/// 同値にならない**形にしてある（取り違えを観測できる）。**対照として持ち主の会話では書ける。**
#[tokio::test]
async fn a_trial_vote_must_come_from_this_characters_session() {
    let (_dir, store) = setup().await;
    let rin = character(&store, "rin", "凛").await;
    let kaede = character(&store, "kaede", "楓").await;
    let aoi = character(&store, "aoi", "葵").await;
    let rin_session = store.create_session(rin).await.expect("会話を作れること");
    store.create_session(rin).await.expect("会話を作れること");
    let kaede_session = store.create_session(kaede).await.expect("会話を作れること");
    let aoi_session = store.create_session(aoi).await.expect("会話を作れること");
    assert_ne!(
        aoi_session.get(),
        aoi.get(),
        "前提: 葵の会話の id と葵の id が別の値"
    );
    assert_eq!(
        kaede_session.get(),
        aoi.get(),
        "前提: 楓の会話の id が葵の id と同じ値（照合の束ね方を取り違える実装を観測する）"
    );
    let source = memory(&store, aoi, "夜に星を見るのが好き").await;
    set_slots(&store, "aoi", 1).await;
    let curious = propose(&store, aoi, "好奇心", source)
        .await
        .expect("作れること");

    for foreign in [rin_session, kaede_session] {
        let err = vote(&store, aoi, curious, foreign, AxisTrialVote::Up)
            .await
            .expect_err("別のキャラクターの会話の票は拒むこと");
        assert!(
            matches!(err, StoreError::SessionOfAnotherCharacter),
            "{err:?}"
        );
    }
    assert_eq!(
        count(&store, "SELECT count(*) FROM stat_axis_trial_vote").await,
        0,
        "拒んだ票は1行も書かれない"
    );

    vote(&store, aoi, curious, aoi_session, AxisTrialVote::Up)
        .await
        .expect("持ち主の会話なら書けること");
    assert_eq!(
        count(&store, "SELECT count(*) FROM stat_axis_trial_vote").await,
        1
    );
}

// ---------------------------------------------------------------------------
// 持ち主
// ---------------------------------------------------------------------------

/// **他のキャラクターの候補は、進められず、票も足せず、一覧にも出ない。**
#[tokio::test]
async fn another_characters_candidate_is_out_of_reach() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let rin = character(&store, "rin", "凛").await;
    let source = memory(&store, aoi, "夜に星を見るのが好き").await;
    let rin_session = store.create_session(rin).await.expect("会話を作れること");
    set_slots(&store, "aoi", 1).await;
    let curious = propose(&store, aoi, "好奇心", source)
        .await
        .expect("作れること");

    let err = advance(&store, rin, curious, AxisCandidateState::Passed)
        .await
        .expect_err("他のキャラクターの候補は進められないこと");
    assert!(matches!(err, StoreError::UnknownAxisCandidate), "{err:?}");
    let err = vote(&store, rin, curious, rin_session, AxisTrialVote::Up)
        .await
        .expect_err("他のキャラクターの候補には票を足せないこと");
    assert!(matches!(err, StoreError::UnknownAxisCandidate), "{err:?}");

    assert!(candidates(&store, rin).await.is_empty(), "凛の一覧は空");
    assert_eq!(
        store
            .count_axis_candidates(rin)
            .await
            .expect("数えられること"),
        0,
        "凛の総数も 0"
    );
    assert_eq!(
        shape(&candidates(&store, aoi).await[0]).1,
        AxisCandidateState::Trial,
        "葵の候補は試用のまま"
    );
    assert_eq!(
        count(&store, "SELECT count(*) FROM stat_axis_trial_vote").await,
        0
    );
}

// ---------------------------------------------------------------------------
// 読み
// ---------------------------------------------------------------------------

/// **上限を渡すと新しい側だけが返り、総数は全部を数える。**
#[tokio::test]
async fn the_listing_is_limited_but_the_count_is_not() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let source = memory(&store, aoi, "夜に星を見るのが好き").await;
    set_slots(&store, "aoi", 3).await;
    for name in ["好奇心", "几帳面さ", "茶目っ気"] {
        propose(&store, aoi, name, source)
            .await
            .expect("作れること");
    }

    let listed = store.axis_candidates(aoi, 2).await.expect("読めること");
    assert_eq!(
        listed
            .iter()
            .map(|candidate| candidate.name().as_str())
            .collect::<Vec<_>>(),
        vec!["茶目っ気", "几帳面さ"],
        "新しい側の2件"
    );
    assert_eq!(
        store
            .count_axis_candidates(aoi)
            .await
            .expect("数えられること"),
        3
    );
}

/// **移り変わりの列が壊れた候補があれば、一覧は止まる。総数は止まらない。**
///
/// 黙って飛ばすと、**どの候補がどこにいるのかが静かに1件消える**。
#[tokio::test]
async fn a_broken_chain_stops_the_listing_but_not_the_count() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let source = memory(&store, aoi, "夜に星を見るのが好き").await;
    set_slots(&store, "aoi", 1).await;
    let curious = propose(&store, aoi, "好奇心", source)
        .await
        .expect("作れること");

    // **CHECK を満たすが遷移の規則に反する行**（試用 → 採用）を直接足す。
    sqlx::query("INSERT INTO stat_axis_candidate_transition (candidate_id, state) VALUES (?, ?)")
        .bind(curious.get())
        .bind("adopted")
        .execute(store.pool())
        .await
        .expect("行を足せること");

    let err = store
        .axis_candidates(aoi, 100)
        .await
        .expect_err("壊れた列は一覧を止めること");
    assert!(matches!(err, StoreError::InvalidStoredValue), "{err:?}");
    assert_eq!(
        store
            .count_axis_candidates(aoi)
            .await
            .expect("数えられること"),
        1
    );
}

// ---------------------------------------------------------------------------
// BEGIN IMMEDIATE
// ---------------------------------------------------------------------------

/// **同じキャラクターへの同時の2件は、枠を溢れさせない。負けた側は「枠が無い」で落ちる。**
///
/// 書き手は「いまの状態を読む → 空きを数える → 書く」なので、素の `BEGIN` だと
/// 昇格が `SQLITE_BUSY_SNAPSHOT`（`Query`）で落ち、**負けた側の理由が変わる。**
/// 空きを数えずに書く実装なら、**両方が通って枠を溢れさせる。**
#[tokio::test]
async fn two_proposals_never_overfill_one_slot() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let source = memory(&store, aoi, "夜に星を見るのが好き").await;
    set_slots(&store, "aoi", 1).await;

    for round in 0..16 {
        let names = [format!("好奇心{round}"), format!("几帳面さ{round}")];
        let tasks: Vec<_> = names
            .iter()
            .map(|name| {
                let store = store.clone();
                let name = axis(name);
                tokio::spawn(async move {
                    store
                        .record_axis_candidate(
                            aoi,
                            &AxisCandidateRecord::Propose {
                                name,
                                source_memory: source,
                            },
                        )
                        .await
                })
            })
            .collect();
        let mut won = Vec::new();
        let mut lost = Vec::new();
        for task in tasks {
            match task.await.expect("タスクが panic しないこと") {
                Ok(id) => won.push(id),
                Err(err) => lost.push(err),
            }
        }
        assert_eq!(won.len(), 1, "{round} 周目: 1件だけが通ること: {lost:?}");
        assert!(
            lost.iter()
                .all(|err| matches!(err, StoreError::AxisDiscoverySlotsFull)),
            "{round} 周目: 負けた側は「枠が無い」で落ちること: {lost:?}"
        );

        // 次の周のために枠を空ける。
        advance(
            &store,
            aoi,
            won[0],
            AxisCandidateState::Failed(AxisTrialFailure::Silent),
        )
        .await
        .unwrap_or_else(|err| panic!("{round} 周目の後始末ができること: {err}"));
    }
}

/// **別のキャラクターへの同時の書き込みは、両方通る**（WAL のスナップショットは DB 全体を指す）。
#[tokio::test]
async fn proposals_for_two_characters_never_lose_their_snapshot() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let rin = character(&store, "rin", "凛").await;
    let aoi_source = memory(&store, aoi, "夜に星を見るのが好き").await;
    let rin_source = memory(&store, rin, "朝は紅茶を淹れる").await;
    set_slots(&store, "aoi", 1).await;
    set_slots(&store, "rin", 1).await;

    for round in 0..16 {
        let tasks: Vec<_> = [(aoi, aoi_source), (rin, rin_source)]
            .into_iter()
            .map(|(id, source)| {
                let store = store.clone();
                let name = axis(&format!("好奇心{round}"));
                tokio::spawn(async move {
                    store
                        .record_axis_candidate(
                            id,
                            &AxisCandidateRecord::Propose {
                                name,
                                source_memory: source,
                            },
                        )
                        .await
                })
            })
            .collect();
        let mut made = Vec::new();
        for task in tasks {
            made.push(task.await.expect("タスクが panic しないこと"));
        }
        assert!(
            made.iter().all(Result::is_ok),
            "{round} 周目: どちらも成功すること: {made:?}"
        );
        for ((id, _), result) in [(aoi, aoi_source), (rin, rin_source)].iter().zip(made) {
            let candidate = result.unwrap_or_else(|err| panic!("{round} 周目: {err}"));
            advance(
                &store,
                *id,
                candidate,
                AxisCandidateState::Failed(AxisTrialFailure::Silent),
            )
            .await
            .unwrap_or_else(|err| panic!("{round} 周目の後始末ができること: {err}"));
        }
    }
}
