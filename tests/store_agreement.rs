//! 取り決め（約束・決まり）の永続化の検証（ADR-0051 §決定1・4 / 作業単位 A-12a）。
//!
//! ここで確かめるのは主に次の点:
//!   * **往復**——書いた取り決めが中身・種類・守る側まで読み戻せる。
//!   * **終わらせる**——状態が変わり、**行は消えず**、有効な一覧（本文が読む）から外れる。
//!   * **履歴は追記**——「書かれた（無→有効）」「有効→果たした」が残り、新しい順に読める。
//!   * **上限**——有効なものが上限に達したら書けず、**何も保存されない**。終わらせれば書ける。
//!   * **持ち主**——他のキャラクターの取り決めは読めず、終わらせられない。
//!   * **壊れた行**——終わった行が壊れていても本文は止まらない。有効な行が壊れたら止まり、
//!     終わらせる口（中身を読まない）で戻せる。
//!   * **提案**（作業単位 A-12c-1）——提案中として書かれ、上限に数えず、採ると有効・捨てると却下。
//!     生んだ会話に縛って書く。
//!
//! スキーマ制約そのものの検証は `tests/store_schema.rs` にある。

use personacore::domain::{
    AgreementContent, AgreementEdits, AgreementEnding, AgreementKeeper, AgreementKind,
    AgreementState, CharacterId, DisplayName, Handle, MAX_ACTIVE_AGREEMENTS, MessageId,
    NewAgreement, NewMessage, Persona, Role, SessionId,
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

fn new_agreement(content: &str, kind: AgreementKind, keeper: AgreementKeeper) -> NewAgreement {
    NewAgreement {
        content: AgreementContent::parse(content).expect("正当な中身であること"),
        kind,
        keeper,
    }
}

/// **このファイルの基準フィクスチャ**（3件 —— 真ん中を選ぶため）。
///
/// **守る側も種類も、3件で互いに違う値の組にする**（同じ値が並ぶと取り違えが検出できない）。
fn fixture() -> [NewAgreement; 3] {
    [
        new_agreement(
            "毎朝あいさつを交わす",
            AgreementKind::Rule,
            AgreementKeeper::Both,
        ),
        new_agreement(
            "週末に海へ連れて行く",
            AgreementKind::Promise,
            AgreementKeeper::Character,
        ),
        new_agreement(
            "名前を呼び捨てにしない",
            AgreementKind::Rule,
            AgreementKeeper::User,
        ),
    ]
}

/// 書いた順の (中身, 種類, 守る側, 状態)。
fn shapes(
    agreements: &[personacore::domain::Agreement],
) -> Vec<(String, AgreementKind, AgreementKeeper, AgreementState)> {
    agreements
        .iter()
        .map(|agreement| {
            (
                agreement.content.as_str().to_owned(),
                agreement.kind,
                agreement.keeper,
                agreement.state,
            )
        })
        .collect()
}

async fn count(store: &Store, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql)
        .fetch_one(store.pool())
        .await
        .expect("数えられること")
}

/// **書いた取り決めが、中身・種類・守る側まで書いた順に読み戻せる。** 書いたものは有効で始まる。
#[tokio::test]
async fn written_agreements_round_trip_in_order_and_start_active() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;

    let mut written = Vec::new();
    for agreement in fixture() {
        written.push(
            store
                .add_agreement(aoi, &agreement)
                .await
                .expect("書けること"),
        );
    }
    assert!(
        written
            .iter()
            .all(|agreement| agreement.state == AgreementState::Active
                && agreement.source_session.is_none()),
        "手で書いたものは有効で始まり、出どころが無いこと: {written:?}"
    );

    let listed = store.agreements(aoi).await.expect("読めること");
    assert_eq!(listed, written, "書いた戻り値と読み戻した姿が一致すること");
    assert_eq!(
        shapes(&listed),
        vec![
            (
                "毎朝あいさつを交わす".to_owned(),
                AgreementKind::Rule,
                AgreementKeeper::Both,
                AgreementState::Active
            ),
            (
                "週末に海へ連れて行く".to_owned(),
                AgreementKind::Promise,
                AgreementKeeper::Character,
                AgreementState::Active
            ),
            (
                "名前を呼び捨てにしない".to_owned(),
                AgreementKind::Rule,
                AgreementKeeper::User,
                AgreementState::Active
            ),
        ]
    );
    assert_eq!(
        store.active_agreements(aoi).await.expect("読めること"),
        written
    );
}

/// **真ん中を終わらせると、その1件だけが状態を変え、行は残り、有効な一覧から外れる。**
/// **履歴に「書かれた」3件と「有効→果たした」1件が、新しい順に残る。**
#[tokio::test]
async fn ending_the_middle_one_keeps_the_row_and_records_the_transition() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let mut ids = Vec::new();
    for agreement in fixture() {
        ids.push(
            store
                .add_agreement(aoi, &agreement)
                .await
                .expect("書けること")
                .id,
        );
    }

    let ended = store
        .end_agreement(aoi, ids[1].get(), AgreementEnding::Fulfilled)
        .await
        .expect("終わらせられること");
    assert_eq!(ended, ids[1]);

    let listed = store.agreements(aoi).await.expect("読めること");
    assert_eq!(
        listed.iter().map(|a| a.state).collect::<Vec<_>>(),
        vec![
            AgreementState::Active,
            AgreementState::Fulfilled,
            AgreementState::Active
        ],
        "真ん中だけが終わり、行は消えていないこと"
    );
    let active: Vec<_> = store
        .active_agreements(aoi)
        .await
        .expect("読めること")
        .iter()
        .map(|a| a.id)
        .collect();
    assert_eq!(
        active,
        vec![ids[0], ids[2]],
        "終わったものは有効な一覧に出ないこと"
    );

    let transitions = store
        .agreement_transitions(aoi, 50)
        .await
        .expect("履歴を読めること");
    let read: Vec<_> = transitions
        .iter()
        .map(|t| (t.agreement_id(), t.before(), t.after(), t.keeper()))
        .collect();
    assert_eq!(
        read,
        vec![
            (
                ids[1],
                Some(AgreementState::Active),
                AgreementState::Fulfilled,
                AgreementKeeper::Character
            ),
            (ids[2], None, AgreementState::Active, AgreementKeeper::User),
            (
                ids[1],
                None,
                AgreementState::Active,
                AgreementKeeper::Character
            ),
            (ids[0], None, AgreementState::Active, AgreementKeeper::Both),
        ],
        "新しい順に、書かれた3件と終わった1件が残ること"
    );
    assert_eq!(
        transitions[0].content().as_str(),
        "週末に海へ連れて行く",
        "履歴はその取り決めの中身と結んで返ること"
    );
    assert_eq!(
        store
            .count_agreement_transitions(aoi)
            .await
            .expect("数えられること"),
        4
    );
}

/// **3つの終わらせ方が、それぞれの状態として残る**（取り違えない）。
#[tokio::test]
async fn each_ending_is_recorded_as_its_own_state() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let mut ids = Vec::new();
    for agreement in fixture() {
        ids.push(
            store
                .add_agreement(aoi, &agreement)
                .await
                .expect("書けること")
                .id,
        );
    }
    // **並びと終わらせ方を相関させない**（真ん中を「破った」にする）。
    for (id, ending) in [
        (ids[2], AgreementEnding::Withdrawn),
        (ids[0], AgreementEnding::Fulfilled),
        (ids[1], AgreementEnding::Broken),
    ] {
        store
            .end_agreement(aoi, id.get(), ending)
            .await
            .expect("終わらせられること");
    }
    let listed = store.agreements(aoi).await.expect("読めること");
    assert_eq!(
        listed.iter().map(|a| a.state).collect::<Vec<_>>(),
        vec![
            AgreementState::Fulfilled,
            AgreementState::Broken,
            AgreementState::Withdrawn
        ]
    );
    assert!(
        store
            .active_agreements(aoi)
            .await
            .expect("読めること")
            .is_empty()
    );
}

/// **終わった取り決めは、もう終わらせられない。何も書かれない。**
#[tokio::test]
async fn an_ended_agreement_cannot_be_ended_again() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let [first, ..] = fixture();
    let id = store
        .add_agreement(aoi, &first)
        .await
        .expect("書けること")
        .id;
    store
        .end_agreement(aoi, id.get(), AgreementEnding::Broken)
        .await
        .expect("終わらせられること");

    let again = store
        .end_agreement(aoi, id.get(), AgreementEnding::Fulfilled)
        .await;
    assert!(
        matches!(again, Err(StoreError::AgreementNotActive)),
        "{again:?}"
    );
    let listed = store.agreements(aoi).await.expect("読めること");
    assert_eq!(
        listed[0].state,
        AgreementState::Broken,
        "状態は最初の終わり方のまま"
    );
    assert_eq!(
        store
            .count_agreement_transitions(aoi)
            .await
            .expect("数えられること"),
        2,
        "履歴は増えないこと"
    );
}

/// **有効なものが上限に達したら書けず、何も保存されない。** 1件終わらせれば、また書ける。
#[tokio::test]
async fn the_limit_refuses_a_write_and_stores_nothing() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;

    let mut ids = Vec::new();
    for index in 0..MAX_ACTIVE_AGREEMENTS {
        let agreement = new_agreement(
            &format!("約束その{index}"),
            AgreementKind::Promise,
            AgreementKeeper::Character,
        );
        ids.push(
            store
                .add_agreement(aoi, &agreement)
                .await
                .unwrap_or_else(|err| panic!("{index} 件目は書けること: {err:?}"))
                .id,
        );
    }
    // **上限の値は関数の外の値で押さえる**（定数を倍にする変異を通さない）。
    assert_eq!(ids.len(), 16);

    let before_rows = count(&store, "SELECT count(*) FROM character_agreement").await;
    let before_history = count(&store, "SELECT count(*) FROM agreement_transition").await;
    let over = store
        .add_agreement(
            aoi,
            &new_agreement(
                "上限を越える約束",
                AgreementKind::Rule,
                AgreementKeeper::User,
            ),
        )
        .await;
    assert!(
        matches!(over, Err(StoreError::AgreementLimitReached)),
        "{over:?}"
    );
    assert_eq!(
        count(&store, "SELECT count(*) FROM character_agreement").await,
        before_rows,
        "取り決めは1行も増えないこと"
    );
    assert_eq!(
        count(&store, "SELECT count(*) FROM agreement_transition").await,
        before_history,
        "履歴も1行も増えないこと"
    );
    assert!(
        store
            .agreements(aoi)
            .await
            .expect("読めること")
            .iter()
            .all(|a| a.content.as_str() != "上限を越える約束"),
        "断った取り決めはどこにも無いこと"
    );

    // **数えるのは有効なものだけ**（終わったものは枠を食わない）。
    store
        .end_agreement(aoi, ids[7].get(), AgreementEnding::Withdrawn)
        .await
        .expect("終わらせられること");
    store
        .add_agreement(
            aoi,
            &new_agreement("空いた枠の約束", AgreementKind::Rule, AgreementKeeper::User),
        )
        .await
        .expect("1件終わらせれば書けること");
}

/// **上限は他のキャラクターの件数を数えない。**
#[tokio::test]
async fn the_limit_counts_only_this_characters_agreements() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let mio = character(&store, "mio", "澪").await;
    for index in 0..MAX_ACTIVE_AGREEMENTS {
        store
            .add_agreement(
                aoi,
                &new_agreement(
                    &format!("約束その{index}"),
                    AgreementKind::Promise,
                    AgreementKeeper::Character,
                ),
            )
            .await
            .expect("書けること");
    }
    store
        .add_agreement(
            mio,
            &new_agreement("澪の約束", AgreementKind::Promise, AgreementKeeper::User),
        )
        .await
        .expect("隣のキャラクターは書けること");
}

/// **同時に書いても、上限を1件も越えない**（数えるのと書くのが同じ `BEGIN IMMEDIATE` の中）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writes_never_exceed_the_limit() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;

    let tasks: Vec<_> = (0..(MAX_ACTIVE_AGREEMENTS + 8))
        .map(|index| {
            let store = store.clone();
            tokio::spawn(async move {
                store
                    .add_agreement(
                        aoi,
                        &new_agreement(
                            &format!("同時の約束{index}"),
                            AgreementKind::Promise,
                            AgreementKeeper::Both,
                        ),
                    )
                    .await
            })
        })
        .collect();
    let mut written = 0;
    let mut refused = 0;
    for task in tasks {
        match task.await.expect("タスクが panic しないこと") {
            Ok(_) => written += 1,
            Err(StoreError::AgreementLimitReached) => refused += 1,
            Err(other) => panic!("想定外の失敗: {other:?}"),
        }
    }
    assert_eq!((written, refused), (16, 8));
    assert_eq!(
        store
            .active_agreements(aoi)
            .await
            .expect("読めること")
            .len(),
        16
    );
}

/// **他のキャラクターの取り決めは読めず、終わらせられない。存在しない id も同じ。**
#[tokio::test]
async fn another_characters_agreement_is_neither_listed_nor_ended() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let mio = character(&store, "mio", "澪").await;
    let [first, second, _] = fixture();
    let aois = store.add_agreement(aoi, &first).await.expect("書けること");
    let mios = store.add_agreement(mio, &second).await.expect("書けること");

    assert_eq!(
        store.agreements(aoi).await.expect("読めること"),
        vec![aois.clone()]
    );
    assert_eq!(
        store.agreements(mio).await.expect("読めること"),
        vec![mios.clone()]
    );

    let crossed = store
        .end_agreement(aoi, mios.id.get(), AgreementEnding::Withdrawn)
        .await;
    assert!(
        matches!(crossed, Err(StoreError::UnknownAgreement)),
        "{crossed:?}"
    );
    let missing = store
        .end_agreement(aoi, mios.id.get() + 1000, AgreementEnding::Withdrawn)
        .await;
    assert!(
        matches!(missing, Err(StoreError::UnknownAgreement)),
        "{missing:?}"
    );
    assert_eq!(
        store.agreements(mio).await.expect("読めること")[0].state,
        AgreementState::Active,
        "隣のキャラクターの取り決めは有効のまま"
    );
    assert_eq!(
        store
            .count_agreement_transitions(aoi)
            .await
            .expect("数えられること"),
        1
    );
    assert_eq!(
        store
            .count_agreement_transitions(mio)
            .await
            .expect("数えられること"),
        1,
        "履歴も持ち主ごとに数えること"
    );
}

/// **履歴は上限で切って返し、総数は切らない。**
#[tokio::test]
async fn only_the_newest_transitions_come_back_but_the_total_counts_them_all() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let mut ids = Vec::new();
    for agreement in fixture() {
        ids.push(
            store
                .add_agreement(aoi, &agreement)
                .await
                .expect("書けること")
                .id,
        );
    }
    store
        .end_agreement(aoi, ids[0].get(), AgreementEnding::Broken)
        .await
        .expect("終わらせられること");

    let newest = store
        .agreement_transitions(aoi, 2)
        .await
        .expect("読めること");
    assert_eq!(
        newest
            .iter()
            .map(|t| (t.agreement_id(), t.after()))
            .collect::<Vec<_>>(),
        vec![
            (ids[0], AgreementState::Broken),
            (ids[2], AgreementState::Active)
        ]
    );
    assert_eq!(
        store
            .count_agreement_transitions(aoi)
            .await
            .expect("数えられること"),
        4
    );
}

/// 生の SQL で行を置く（**スキーマの CHECK は通る形**。`store` の書き手は通らない）。
async fn insert_raw(store: &Store, character: CharacterId, content: &str, state: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO character_agreement (character_id, content, kind, keeper, state) \
         VALUES (?, ?, 'rule', 'user', ?) RETURNING id",
    )
    .bind(character.get())
    .bind(content)
    .bind(state)
    .fetch_one(store.pool())
    .await
    .expect("行を置けること")
}

/// **終わった行が壊れていても、本文が読む「有効な一覧」は止まらない。一覧の口は止まる。**
#[tokio::test]
async fn a_broken_ended_row_does_not_stop_the_active_read() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let [first, ..] = fixture();
    let kept = store.add_agreement(aoi, &first).await.expect("書けること");
    let too_long = "長".repeat(AgreementContent::MAX_CHARS + 1);
    insert_raw_ignoring_checks(&store, aoi, &too_long, "fulfilled").await;

    assert_eq!(
        store
            .active_agreements(aoi)
            .await
            .expect("有効な一覧は読めること"),
        vec![kept],
        "終わった壊れた行は読まないこと"
    );
    assert!(
        matches!(
            store.agreements(aoi).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "一覧の口は黙って飛ばさず止まること"
    );
}

/// **有効な行が壊れたら本文の読みは止まり、終わらせる口（中身を読まない）で戻せる。**
#[tokio::test]
async fn a_broken_active_row_stops_the_read_and_ending_it_restores_it() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let [first, ..] = fixture();
    let kept = store.add_agreement(aoi, &first).await.expect("書けること");
    let too_long = "長".repeat(AgreementContent::MAX_CHARS + 1);
    let raw = insert_raw_ignoring_checks(&store, aoi, &too_long, "active").await;

    assert!(
        matches!(
            store.active_agreements(aoi).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "壊れた有効な行を黙って飛ばさないこと"
    );
    store
        .end_agreement(aoi, raw, AgreementEnding::Withdrawn)
        .await
        .expect("中身を読まずに終わらせられること");
    assert_eq!(
        store.active_agreements(aoi).await.expect("戻ること"),
        vec![kept]
    );
}

/// **提案中・却下は一覧に載るが、本文が読む「有効な一覧」には載らない**（作業単位 A-12c-1）。
///
/// A-12a の間は「読めない」（一覧の口が `InvalidStoredValue`）を固定していた。読み手を広げた日に
/// 主張を書き換えた（`AgreementState::from_label` の doc の予告どおり）。
#[tokio::test]
async fn a_proposed_row_is_listed_but_not_read_as_active() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    insert_raw(&store, aoi, "提案された約束", "proposed").await;
    insert_raw(&store, aoi, "捨てられた約束", "rejected").await;

    assert!(
        store
            .active_agreements(aoi)
            .await
            .expect("読めること")
            .is_empty(),
        "提案中・却下は本文に載らないこと"
    );
    assert_eq!(
        shapes(&store.agreements(aoi).await.expect("一覧は読めること")),
        vec![
            (
                "提案された約束".to_owned(),
                AgreementKind::Rule,
                AgreementKeeper::User,
                AgreementState::Proposed
            ),
            (
                "捨てられた約束".to_owned(),
                AgreementKind::Rule,
                AgreementKeeper::User,
                AgreementState::Rejected
            ),
        ]
    );
}

// ---------------------------------------------------------------------------
// 提案（作業単位 A-12c-1 / ADR-0051 §決定2）
// ---------------------------------------------------------------------------

/// 会話を1つ作り、`turns` 往復を書く。**応答の id を書いた順に返す。**
async fn conversation(
    store: &Store,
    character: CharacterId,
    turns: usize,
) -> (SessionId, Vec<MessageId>) {
    let session = store
        .create_session(character)
        .await
        .expect("会話を作れること");
    let mut replies = Vec::new();
    for index in 0..turns {
        let written = store
            .append_turn(
                session,
                &message(Role::User, &format!("発言{index}")),
                &message(Role::Assistant, &format!("応答{index}")),
            )
            .await
            .expect("往復を書けること");
        replies.push(written.reply);
    }
    (session, replies)
}

fn message(role: Role, raw: &str) -> NewMessage {
    NewMessage {
        role,
        dialogue: None,
        narration: None,
        raw_content: raw.to_owned(),
    }
}

/// **提案を書くと、提案中として一覧に載り（出どころの会話つき）、本文には載らない。**
/// 履歴には「書かれた（無 → 提案中）」が残る。
#[tokio::test]
async fn recorded_proposals_are_listed_as_proposed_with_their_conversation() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let (session, _) = conversation(&store, aoi, 1).await;

    let [first, second, _] = fixture();
    let recorded = store
        .record_agreement_proposals(aoi, session, &[first.clone(), second.clone()])
        .await
        .expect("書けること");
    assert_eq!((recorded.written, recorded.duplicates), (2, 0));

    let listed = store.agreements(aoi).await.expect("読めること");
    assert_eq!(
        shapes(&listed),
        vec![
            (
                first.content.as_str().to_owned(),
                first.kind,
                first.keeper,
                AgreementState::Proposed
            ),
            (
                second.content.as_str().to_owned(),
                second.kind,
                second.keeper,
                AgreementState::Proposed
            ),
        ]
    );
    assert!(
        listed
            .iter()
            .all(|agreement| agreement.source_session == Some(session)),
        "出どころの会話を持つこと"
    );
    assert!(
        store
            .active_agreements(aoi)
            .await
            .expect("読める")
            .is_empty()
    );
    let transitions = store
        .agreement_transitions(aoi, 10)
        .await
        .expect("読めること");
    assert_eq!(
        transitions
            .iter()
            .map(|transition| (transition.before(), transition.after()))
            .collect::<Vec<_>>(),
        vec![
            (None, AgreementState::Proposed),
            (None, AgreementState::Proposed)
        ]
    );
}

/// **提案中は有効な件数の上限に数えない。採ると数える**（上限に達していたら採れない）。
#[tokio::test]
async fn proposals_do_not_count_toward_the_limit_but_adopting_does() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let (session, _) = conversation(&store, aoi, 1).await;
    for index in 0..MAX_ACTIVE_AGREEMENTS - 1 {
        store
            .add_agreement(
                aoi,
                &new_agreement(
                    &format!("決まり{index}"),
                    AgreementKind::Rule,
                    AgreementKeeper::User,
                ),
            )
            .await
            .expect("上限の手前までは書けること");
    }
    let proposals: Vec<NewAgreement> = (0..3)
        .map(|index| {
            new_agreement(
                &format!("提案{index}"),
                AgreementKind::Promise,
                AgreementKeeper::Both,
            )
        })
        .collect();
    store
        .record_agreement_proposals(aoi, session, &proposals)
        .await
        .expect("有効が上限の手前でも提案は何件でも書けること");
    // **手で書く口も、提案中を数えない**（有効 15 + 提案中 3 でも、もう1件書ける）。
    let filler = store
        .add_agreement(
            aoi,
            &new_agreement("16件目", AgreementKind::Rule, AgreementKeeper::User),
        )
        .await
        .expect("提案中は上限に数えないこと");
    // **有効が上限ちょうどでも、提案は書ける**（提案中は上限に数えない）。
    store
        .record_agreement_proposals(
            aoi,
            session,
            &[new_agreement(
                "上限の後の提案",
                AgreementKind::Rule,
                AgreementKeeper::User,
            )],
        )
        .await
        .expect("有効が上限に達していても提案は書けること");

    let listed = store.agreements(aoi).await.expect("読める");
    let proposed: Vec<i64> = listed
        .iter()
        .filter(|agreement| agreement.state == AgreementState::Proposed)
        .map(|agreement| agreement.id.get())
        .collect();
    assert_eq!(proposed.len(), 4);
    assert!(matches!(
        store
            .adopt_agreement(aoi, proposed[1], &AgreementEdits::default())
            .await,
        Err(StoreError::AgreementLimitReached)
    ));
    assert_eq!(
        store.agreements(aoi).await.expect("読める"),
        listed,
        "断ったら何も変わらないこと"
    );

    store
        .end_agreement(aoi, filler.id.get(), AgreementEnding::Withdrawn)
        .await
        .expect("終わらせられること");
    let adopted = store
        .adopt_agreement(aoi, proposed[1], &AgreementEdits::default())
        .await
        .expect("枠が空けば採れること");
    assert_eq!(adopted.state, AgreementState::Active);
    assert_eq!(adopted.content.as_str(), "提案1");
    assert_eq!(
        store.active_agreements(aoi).await.expect("読める").len(),
        MAX_ACTIVE_AGREEMENTS
    );
}

/// **採るときに中身・種類・守る側を直せる。直さなかった欄は提案のまま。**
/// 履歴に「提案中 → 有効」が残り、本文の読み（有効な一覧）に載る。
#[tokio::test]
async fn adopting_can_edit_each_field_and_records_the_transition() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let (session, _) = conversation(&store, aoi, 1).await;
    let proposals = [
        new_agreement("扉を閉める", AgreementKind::Rule, AgreementKeeper::User),
        new_agreement("海へ行く", AgreementKind::Rule, AgreementKeeper::Character),
        new_agreement("名前で呼ぶ", AgreementKind::Rule, AgreementKeeper::User),
    ];
    store
        .record_agreement_proposals(aoi, session, &proposals)
        .await
        .expect("書けること");
    let ids: Vec<i64> = store
        .agreements(aoi)
        .await
        .expect("読める")
        .iter()
        .map(|agreement| agreement.id.get())
        .collect();

    // 真ん中だけを、3つとも直して採る。
    let edited = store
        .adopt_agreement(
            aoi,
            ids[1],
            &AgreementEdits {
                content: Some(AgreementContent::parse("週末に海へ連れて行く").expect("中身")),
                kind: Some(AgreementKind::Promise),
                keeper: Some(AgreementKeeper::Both),
            },
        )
        .await
        .expect("採れること");
    // 3つ目は守る側だけを直す。
    let partly = store
        .adopt_agreement(
            aoi,
            ids[2],
            &AgreementEdits {
                keeper: Some(AgreementKeeper::Character),
                ..AgreementEdits::default()
            },
        )
        .await
        .expect("採れること");

    assert_eq!(
        shapes(&store.agreements(aoi).await.expect("読める")),
        vec![
            (
                "扉を閉める".to_owned(),
                AgreementKind::Rule,
                AgreementKeeper::User,
                AgreementState::Proposed
            ),
            (
                "週末に海へ連れて行く".to_owned(),
                AgreementKind::Promise,
                AgreementKeeper::Both,
                AgreementState::Active
            ),
            (
                "名前で呼ぶ".to_owned(),
                AgreementKind::Rule,
                AgreementKeeper::Character,
                AgreementState::Active
            ),
        ]
    );
    assert_eq!(
        store.active_agreements(aoi).await.expect("読める"),
        vec![edited.clone(), partly]
    );
    assert_eq!(edited.source_session, Some(session));
    let newest = store.agreement_transitions(aoi, 1).await.expect("読める");
    assert_eq!(
        (
            newest[0].before(),
            newest[0].after(),
            newest[0].agreement_id().get()
        ),
        (
            Some(AgreementState::Proposed),
            AgreementState::Active,
            ids[2]
        )
    );
}

/// **捨てると却下になり、行は残る。** 却下・有効なものは採れも捨てられもしない。
/// 提案中は終わらせられない（終わらせる口は有効なものだけ）。
#[tokio::test]
async fn rejecting_keeps_the_row_and_only_proposals_can_be_decided() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let (session, _) = conversation(&store, aoi, 1).await;
    let [first, second, third] = fixture();
    store
        .record_agreement_proposals(aoi, session, &[first, second, third])
        .await
        .expect("書けること");
    let ids: Vec<i64> = store
        .agreements(aoi)
        .await
        .expect("読める")
        .iter()
        .map(|agreement| agreement.id.get())
        .collect();

    assert_eq!(
        store
            .reject_agreement(aoi, ids[1])
            .await
            .expect("捨てられること")
            .get(),
        ids[1]
    );
    let listed = store.agreements(aoi).await.expect("読める");
    assert_eq!(
        listed
            .iter()
            .map(|agreement| agreement.state)
            .collect::<Vec<_>>(),
        vec![
            AgreementState::Proposed,
            AgreementState::Rejected,
            AgreementState::Proposed
        ]
    );
    let newest = store.agreement_transitions(aoi, 1).await.expect("読める");
    assert_eq!(
        (newest[0].before(), newest[0].after()),
        (Some(AgreementState::Proposed), AgreementState::Rejected)
    );

    // 却下は採れない・捨て直せない。
    assert!(matches!(
        store
            .adopt_agreement(aoi, ids[1], &AgreementEdits::default())
            .await,
        Err(StoreError::AgreementNotProposed)
    ));
    assert!(matches!(
        store.reject_agreement(aoi, ids[1]).await,
        Err(StoreError::AgreementNotProposed)
    ));
    // 提案中は終わらせられない。
    assert!(matches!(
        store
            .end_agreement(aoi, ids[0], AgreementEnding::Fulfilled)
            .await,
        Err(StoreError::AgreementNotActive)
    ));
    // 有効なものは捨てられない。
    store
        .adopt_agreement(aoi, ids[2], &AgreementEdits::default())
        .await
        .expect("採れること");
    assert!(matches!(
        store.reject_agreement(aoi, ids[2]).await,
        Err(StoreError::AgreementNotProposed)
    ));
    assert_eq!(
        store
            .count_agreement_transitions(aoi)
            .await
            .expect("数えられる"),
        5,
        "書かれた3つ・却下1つ・採用1つ"
    );
}

/// **同じ中身の提案中・有効が既にあれば足さない**（同じ回の中の重なりも）。却下・終わったものは見ない。
///
/// 似ているもの（dedup2）を落とすのは `parse::agreement_proposal` の後段で、ここは
/// **綴りがまったく同じもの**だけを見る最後の砦（後段が通した綴りの同じ提案を、二重に積まない）。
#[tokio::test]
async fn proposals_equal_to_a_proposed_or_active_one_are_not_added() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let (session, _) = conversation(&store, aoi, 2).await;
    let active = new_agreement("有効なもの", AgreementKind::Rule, AgreementKeeper::User);
    let ended = new_agreement("終わったもの", AgreementKind::Rule, AgreementKeeper::User);
    store.add_agreement(aoi, &active).await.expect("書ける");
    let ended_id = store.add_agreement(aoi, &ended).await.expect("書ける").id;
    store
        .end_agreement(aoi, ended_id.get(), AgreementEnding::Fulfilled)
        .await
        .expect("終わらせられる");
    let proposed = new_agreement("提案中のもの", AgreementKind::Rule, AgreementKeeper::Both);
    let rejected = new_agreement("却下したもの", AgreementKind::Rule, AgreementKeeper::Both);
    store
        .record_agreement_proposals(aoi, session, &[proposed.clone(), rejected.clone()])
        .await
        .expect("書ける");
    let rejected_id = store
        .agreements(aoi)
        .await
        .expect("読める")
        .iter()
        .find(|agreement| agreement.content.as_str() == "却下したもの")
        .expect("ある")
        .id;
    store
        .reject_agreement(aoi, rejected_id.get())
        .await
        .expect("捨てられる");

    let fresh = new_agreement("新しいもの", AgreementKind::Rule, AgreementKeeper::User);
    // **守る側や種類が違っても、中身が同じなら同じもの**として数える。
    let active_again = new_agreement("有効なもの", AgreementKind::Promise, AgreementKeeper::Both);
    let recorded = store
        .record_agreement_proposals(
            aoi,
            session,
            &[
                active_again,
                proposed,
                rejected.clone(),
                ended.clone(),
                fresh.clone(),
                fresh,
            ],
        )
        .await
        .expect("書ける");
    assert_eq!((recorded.written, recorded.duplicates), (3, 3));
    let contents: Vec<(String, AgreementState)> = store
        .agreements(aoi)
        .await
        .expect("読める")
        .iter()
        .map(|agreement| (agreement.content.as_str().to_owned(), agreement.state))
        .collect();
    assert_eq!(
        contents,
        vec![
            ("有効なもの".to_owned(), AgreementState::Active),
            ("終わったもの".to_owned(), AgreementState::Fulfilled),
            ("提案中のもの".to_owned(), AgreementState::Proposed),
            ("却下したもの".to_owned(), AgreementState::Rejected),
            ("却下したもの".to_owned(), AgreementState::Proposed),
            ("終わったもの".to_owned(), AgreementState::Proposed),
            ("新しいもの".to_owned(), AgreementState::Proposed),
        ]
    );
}

/// **別のキャラクターの会話・消えた会話に縛った提案は、1行も書かない**（ADR-0049 §決定3）。
#[tokio::test]
async fn proposals_bound_to_another_or_a_missing_conversation_write_nothing() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let ren = character(&store, "ren", "蓮").await;
    let (aoi_session, _) = conversation(&store, aoi, 1).await;
    let (ren_session, _) = conversation(&store, ren, 1).await;
    let [first, ..] = fixture();

    assert!(matches!(
        store
            .record_agreement_proposals(aoi, ren_session, std::slice::from_ref(&first))
            .await,
        Err(StoreError::SessionOfAnotherCharacter)
    ));
    // 無い会話（使い回されない id の先）は「消えた」。
    let missing = SessionId::from_raw_for_tests(aoi_session.get() + ren_session.get() + 100);
    assert!(matches!(
        store
            .record_agreement_proposals(aoi, missing, std::slice::from_ref(&first))
            .await,
        Err(StoreError::TurnSessionGone)
    ));
    for character in [aoi, ren] {
        assert!(
            store
                .agreements(character)
                .await
                .expect("読める")
                .is_empty()
        );
    }
}

/// **別のキャラクターの提案は採れず、捨てられない**（どちらも「無い」）。
#[tokio::test]
async fn another_characters_proposal_is_neither_adopted_nor_rejected() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let ren = character(&store, "ren", "蓮").await;
    let (session, _) = conversation(&store, aoi, 1).await;
    let [first, ..] = fixture();
    store
        .record_agreement_proposals(aoi, session, &[first])
        .await
        .expect("書ける");
    let id = store.agreements(aoi).await.expect("読める")[0].id.get();

    assert!(matches!(
        store
            .adopt_agreement(ren, id, &AgreementEdits::default())
            .await,
        Err(StoreError::UnknownAgreement)
    ));
    assert!(matches!(
        store.reject_agreement(ren, id).await,
        Err(StoreError::UnknownAgreement)
    ));
    assert!(matches!(
        store.reject_agreement(aoi, id + 100).await,
        Err(StoreError::UnknownAgreement)
    ));
    assert_eq!(
        store.agreements(aoi).await.expect("読める")[0].state,
        AgreementState::Proposed
    );
}

/// **同時に採っても上限を1件も越えない**（数えるのと書くのが同じ `BEGIN IMMEDIATE` の中）。
#[tokio::test]
async fn concurrent_adoptions_never_exceed_the_limit() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let (session, _) = conversation(&store, aoi, 1).await;
    for index in 0..MAX_ACTIVE_AGREEMENTS - 2 {
        store
            .add_agreement(
                aoi,
                &new_agreement(
                    &format!("決まり{index}"),
                    AgreementKind::Rule,
                    AgreementKeeper::User,
                ),
            )
            .await
            .expect("書ける");
    }
    let proposals: Vec<NewAgreement> = (0..8)
        .map(|index| {
            new_agreement(
                &format!("提案{index}"),
                AgreementKind::Rule,
                AgreementKeeper::User,
            )
        })
        .collect();
    store
        .record_agreement_proposals(aoi, session, &proposals)
        .await
        .expect("書ける");
    let ids: Vec<i64> = store
        .agreements(aoi)
        .await
        .expect("読める")
        .iter()
        .filter(|agreement| agreement.state == AgreementState::Proposed)
        .map(|agreement| agreement.id.get())
        .collect();

    let handles: Vec<_> = ids
        .iter()
        .map(|id| {
            let store = store.clone();
            let id = *id;
            tokio::spawn(async move {
                store
                    .adopt_agreement(aoi, id, &AgreementEdits::default())
                    .await
            })
        })
        .collect();
    let mut adopted = 0;
    let mut refused = 0;
    for handle in handles {
        match handle.await.expect("タスクが終わること") {
            Ok(_) => adopted += 1,
            Err(StoreError::AgreementLimitReached) => refused += 1,
            Err(other) => panic!("想定外の失敗: {other:?}"),
        }
    }
    assert_eq!((adopted, refused), (2, 6));
    assert_eq!(
        store.active_agreements(aoi).await.expect("読める").len(),
        MAX_ACTIVE_AGREEMENTS
    );
}

/// **往復の通し番号は、その応答までの利用者の発言の数**（提案の依頼の「往復 n」/ 作業単位 A-12c-1）。
///
/// 会話ごとに1から数える。**真ん中の往復を指して**、端の取り違え（全件を数える・次の往復まで数える）を通さない。
#[tokio::test]
async fn the_turn_number_counts_the_user_messages_through_the_reply() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi", "葵").await;
    let (first_session, first_replies) = conversation(&store, aoi, 3).await;
    let (second_session, second_replies) = conversation(&store, aoi, 2).await;

    let numbers = [
        store.turn_number(first_session, first_replies[0]).await,
        store.turn_number(first_session, first_replies[1]).await,
        store.turn_number(first_session, first_replies[2]).await,
        store.turn_number(second_session, second_replies[0]).await,
        store.turn_number(second_session, second_replies[1]).await,
    ]
    .map(|number| number.expect("数えられること"));
    assert_eq!(numbers, [1, 2, 3, 1, 2]);
}

/// CHECK を外して行を置く（**同じ接続で外して・置いて・戻す**）。
async fn insert_raw_ignoring_checks(
    store: &Store,
    character: CharacterId,
    content: &str,
    state: &str,
) -> i64 {
    let mut connection = store.pool().acquire().await.expect("接続を取れること");
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *connection)
        .await
        .expect("CHECK を外せること");
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO character_agreement (character_id, content, kind, keeper, state) \
         VALUES (?, ?, 'rule', 'user', ?) RETURNING id",
    )
    .bind(character.get())
    .bind(content)
    .bind(state)
    .fetch_one(&mut *connection)
    .await
    .expect("行を置けること");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *connection)
        .await
        .expect("CHECK を戻せること");
    id
}
