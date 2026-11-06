//! **変容の票の表**（[ADR-0026](../docs/adr/0026-drift-by-accumulation.md) §決定3 /
//! 作業単位 16d）の検証。
//!
//! 見ているのは `Store::record_drift_vote` と `Store::recent_drift_votes` の**実挙動**で、
//! 「いつ票を立てるか」（`server::drift`）とは別。
//! 文の形そのものと追記のみの走査は `src/store/drift.rs` の単体テストが持つ。
//!
//! | 見るもの | 何が壊れたら赤になるか |
//! |---|---|
//! | 票が**この持ち主・この往復**に立つ | `character_id` / `session_id` の取り違え |
//! | 向きが**提案どおり**に残る | 符号の反転・決め打ち |
//! | **新しい順**に、上限まで返る | 並び・`LIMIT` を落とす |
//! | 同着は `id` で決まる | 第2キーを落とす |
//! | **1行壊れると読みが止まる**（記録は止まらない） | 読み出しの検証を握り潰す |
//! | スキーマが向きの綴りを縛る | CHECK を落とす／綴りがずれる |
//! | 宛先の無い票は入らない | 外部キーを落とす |
//!
//! # 追記のみ（ADR-0026 §決定3）
//!
//! `store` に**票を更新する API も削除する API も無い。**
//! ここにその種のテストが無いのは、**呼ぶものが存在しないから**である
//! （そこを塞いでいるのは走査 `no_statement_updates_or_deletes_a_vote`）。

mod common;

use personacore::domain::{
    CharacterId, DisplayName, DriftDelta, DriftOutcome, DriftProposal, DriftSuggestion, DriftVote,
    Handle, Persona, SessionId, StatAlloc, StatAxis,
};
use personacore::store::{Store, StoreError};
use tempfile::TempDir;

use common::{fixture_scale, insert_drift_vote_ignoring_checks};

/// 票が立つ軸。**他のどの軸とも十分違う綴り。**
const VOTED_AXIS: &str = "パッション";

/// もう1本の軸。[`VOTED_AXIS`] とは別の綴り。
const OTHER_AXIS: &str = "ロジック";

/// 会話するキャラクターの識別子。
const HANDLE: &str = "aoi";

/// **会話しない**キャラクターの識別子。会話する方とは別の綴り。
const BYSTANDER_HANDLE: &str = "kaede";

/// 読み出す件数の上限。**入れる票より大きい**（`LIMIT` の効き目は別のテストで見る）。
const PLENTY: u32 = 32;

/// 開いた DB と、そこに置いた2体のキャラクター・2つのセッション。
struct Fixture {
    store: Store,
    /// **票が立つ方。** 後から作るので `id` は 1 ではない。
    character: CharacterId,
    /// **票が立たない方。** 同じ綴りの軸を持っている。
    bystander: CharacterId,
    /// 票を立てる往復。
    session: SessionId,
    /// **同じキャラクターの別の往復。** `session_id` の決め打ちを観測する。
    other_session: SessionId,
    /// 傍観者の往復。
    bystander_session: SessionId,
    _dir: TempDir,
}

/// キャラクターを1件作って id を返す。**表示名は識別子と別の綴り。**
async fn create(store: &Store, handle: &str, display_name: &str) -> CharacterId {
    let handle = Handle::parse(handle).expect("正当な識別子であること");
    store
        .create_character(
            &handle,
            &DisplayName::parse(display_name).expect("正当な表示名であること"),
            &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること")
}

/// 軸と値の組から `StatAlloc` を作る。
fn allocs(pairs: &[(&str, u32)]) -> Vec<StatAlloc> {
    pairs
        .iter()
        .map(|(axis, value)| StatAlloc {
            axis: StatAxis::parse(axis).expect("正当な軸名であること"),
            value: fixture_scale()
                .value(*value)
                .expect("尺度に収まる値であること"),
        })
        .collect()
}

/// **2体のキャラクターと3つのセッション。**
///
/// # なぜ往復が3つ要るのか
///
/// 「どの往復の票か」を見るには、**同じキャラクターに別の往復**が要る
/// （1つしか無いと `session_id` を決め打ちにする実装が観測できない）。
/// 傍観者の側にも往復を作るのは、**持ち主と往復を取り違える**実装
/// （傍観者の往復に自分の票を立てる）を観測するため。
async fn fresh() -> Fixture {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let store = Store::connect(&dir.path().join("vote.sqlite"))
        .await
        .expect("DB を開けること");

    // **票が立たない方を先に作る**（＝ `id` が 1 になるのはこちら）。
    let bystander = create(&store, BYSTANDER_HANDLE, "楓").await;
    let character = create(&store, HANDLE, "葵").await;
    assert_ne!(character, bystander, "前提: 2体の id が別であること");
    assert_ne!(
        character,
        CharacterId::from_raw_for_tests(1),
        "前提: 票が立つ方の id が 1 でないこと（決め打ちと同値にしない）"
    );

    for id in [character, bystander] {
        store
            .replace_stat_allocs(id, &allocs(&[(VOTED_AXIS, 4), (OTHER_AXIS, 2)]))
            .await
            .expect("性格ステータスを置けること");
    }

    // **傍観者の往復を先に作る**（＝ `id` が 1 になるのはこちら）。
    let bystander_session = store
        .create_session(bystander)
        .await
        .expect("セッションを作成できること");
    let session = store
        .create_session(character)
        .await
        .expect("セッションを作成できること");
    let other_session = store
        .create_session(character)
        .await
        .expect("セッションを作成できること");
    assert_ne!(session, other_session, "前提: 2つの往復が別であること");
    assert_ne!(
        session, bystander_session,
        "前提: 傍観者の往復とも別であること"
    );

    Fixture {
        store,
        character,
        bystander,
        session,
        other_session,
        bystander_session,
        _dir: dir,
    }
}

/// **許可リストを通して**票を組み立てる。門を通らない提案なら落とす。
///
/// **`DriftVote` はここでは構造体リテラルで作れない**（フィールドが private で、
/// 唯一の構築経路が `DriftOutcome::vote`）。テストも同じ門を通る。
fn vote(current: &[StatAlloc], axis: &str, delta: DriftDelta) -> DriftVote {
    let proposal = DriftProposal::Suggested(DriftSuggestion {
        axis: StatAxis::parse(axis).expect("正当な軸名であること"),
        delta,
    });
    let outcome = personacore::domain::apply_drift(current, proposal);
    assert!(
        matches!(
            outcome,
            DriftOutcome::Applied(_) | DriftOutcome::Saturated { .. }
        ),
        "門を通る提案であること: {outcome:?}"
    );
    outcome.vote().expect("票になること")
}

/// 保存済みの票を `(セッション, 軸名, 向き)` の組で、**新しい順**に読む。
async fn votes(store: &Store, id: CharacterId, limit: u32) -> Vec<(i64, String, &'static str)> {
    store
        .recent_drift_votes(id, limit)
        .await
        .expect("票を読めること")
        .iter()
        .map(|stored| {
            (
                stored.session_id().get(),
                stored.vote().axis().as_str().to_owned(),
                stored.vote().delta().label(),
            )
        })
        .collect()
}

/// そのキャラクターの軸を読む。
async fn stats(store: &Store, id: CharacterId) -> Vec<StatAlloc> {
    store
        .stat_allocs(id)
        .await
        .expect("性格ステータスを読めること")
}

/// **票は、書いた持ち主と書いた往復に立つ**（ADR-0026 §決定3）。
///
/// # 帰属を3方向から見る
///
/// * **持ち主** —— 傍観者の側は空のまま。
/// * **往復** —— 同じキャラクターの2つの往復に別々の票を立て、**入れ替わらない**こと。
/// * **向き** —— 上げと下げが、それぞれ提案どおりに残ること。
///
/// CLAUDE.md「抜けるのはいつも帰属。『載っているか』ではなく『どれのものか』」。
#[tokio::test]
async fn a_vote_lands_on_the_character_and_the_turn_that_produced_it() {
    let fixture = fresh().await;
    let current = stats(&fixture.store, fixture.character).await;

    fixture
        .store
        .record_drift_vote(
            fixture.character,
            fixture.session,
            &vote(&current, VOTED_AXIS, DriftDelta::Up),
        )
        .await
        .expect("票を記録できること");
    fixture
        .store
        .record_drift_vote(
            fixture.character,
            fixture.other_session,
            &vote(&current, OTHER_AXIS, DriftDelta::Down),
        )
        .await
        .expect("票を記録できること");

    assert_eq!(
        votes(&fixture.store, fixture.character, PLENTY).await,
        vec![
            (
                fixture.other_session.get(),
                OTHER_AXIS.to_owned(),
                DriftDelta::Down.label()
            ),
            (
                fixture.session.get(),
                VOTED_AXIS.to_owned(),
                DriftDelta::Up.label()
            ),
        ],
        "2件が、それぞれの往復・軸・向きのまま新しい順に返ること"
    );
    assert!(
        votes(&fixture.store, fixture.bystander, PLENTY)
            .await
            .is_empty(),
        "別のキャラクターの票にならないこと"
    );

    fixture.store.close().await;
}

/// **票は、他のキャラクターの読みに混ざらない。**
///
/// 2体とも同じ綴りの軸に票を立てる。**`character_id = ?` を落とすと、
/// どちらの読みにも4件返る**——蓄積の閾値が2倍の速さで埋まる。
#[tokio::test]
async fn each_character_only_sees_its_own_votes() {
    let fixture = fresh().await;
    let mine = stats(&fixture.store, fixture.character).await;
    let theirs = stats(&fixture.store, fixture.bystander).await;

    for _ in 0..2 {
        fixture
            .store
            .record_drift_vote(
                fixture.character,
                fixture.session,
                &vote(&mine, VOTED_AXIS, DriftDelta::Up),
            )
            .await
            .expect("票を記録できること");
        fixture
            .store
            .record_drift_vote(
                fixture.bystander,
                fixture.bystander_session,
                &vote(&theirs, VOTED_AXIS, DriftDelta::Down),
            )
            .await
            .expect("票を記録できること");
    }

    let mine = votes(&fixture.store, fixture.character, PLENTY).await;
    let theirs = votes(&fixture.store, fixture.bystander, PLENTY).await;

    assert_eq!(mine.len(), 2, "自分の票だけが返ること: {mine:?}");
    assert_eq!(theirs.len(), 2, "相手の票だけが返ること: {theirs:?}");
    assert!(
        mine.iter()
            .all(|(session, _, delta)| *session == fixture.session.get()
                && *delta == DriftDelta::Up.label()),
        "自分の票が相手のものに置き換わっていないこと: {mine:?}"
    );
    assert!(
        theirs.iter().all(
            |(session, _, delta)| *session == fixture.bystander_session.get()
                && *delta == DriftDelta::Down.label()
        ),
        "相手の票が自分のものに置き換わっていないこと: {theirs:?}"
    );

    fixture.store.close().await;
}

/// **直近 N 件だけが、新しい順に返る**（ADR-0026 §決定2 の入力）。
///
/// `LIMIT` を落とすと、票は消えないので**読む量が会話の長さに比例して増え続ける。**
/// 並びを落とすと「直近」でなくなり、**蓄積が古い票を数える。**
///
/// # ⚠ **並びを回文にしないこと**（変異検査で見つかった穴 / 2026-08-23）
///
/// 最初は5件を `[軸A, 軸B, 軸A, 軸B, 軸A]` で積んでいた。
/// **この並びは先頭3件と末尾3件が同じ**（どちらも `[A, B, A]`）なので、
/// **`DESC` を落として古い順にする変異が、このテストでは緑を通った。**
///
/// # ⚠ **時刻と `id` を同じ順に積まないこと**（16d のレビューで実測）
///
/// 是正後も `created_at` を `10:00 → 10:04` と**挿入順どおり**に積んでいたので、
/// **第1キーを `ORDER BY id DESC` に落とす変異が 1338 全緑で通った**
/// （文面を写した速報も一緒に緩めて測定）。
/// **`created_at` 順と `id` 順が一度も食い違わなければ、時刻は観測されていない。**
///
/// **いまは時刻を入れ替えて積んである** ——
/// 3件目と5件目が、2件目・4件目より**古い時刻**で入る。
///
/// | 入った順（= `id`） | 時刻 |
/// |---|---|
/// | 1 | 10:00 |
/// | 2 | **10:03** |
/// | 3 | **10:01** |
/// | 4 | **10:04** |
/// | 5 | **10:02** |
///
/// 時刻で並べると `4, 2, 5, 3, 1`、`id` で並べると `5, 4, 3, 2, 1`。
/// **上限3件では集合そのものが違う**（`{4,2,5}` と `{5,4,3}`）。
///
/// # 実害
///
/// `SELECT_RECENT_VOTES` の doc が名指ししている「復元・移行・測定用の一括投入」で、
/// **時刻が前後した票が入ると「直近 N 件」が別の集合を指し、16e は違う票を消費する。**
/// `Store::recent_drift_votes` の doc いわく、**1票の食い違いは
/// 「変容が1回起きない／余計に起きる」に直結し、しかもどこにも現れない。**
#[tokio::test]
async fn only_the_most_recent_votes_come_back_and_they_come_back_newest_first() {
    let fixture = fresh().await;

    // **時刻を明示して積む。** 実時間に頼ると、同じミリ秒に入って順が決まらない。
    // **時刻の順と入った順（`id`）をわざと食い違わせる**（doc の表）。
    let ballots = [
        (fixture.session, VOTED_AXIS, DriftDelta::Up, "10:00"),
        (fixture.other_session, VOTED_AXIS, DriftDelta::Down, "10:03"),
        (fixture.session, OTHER_AXIS, DriftDelta::Up, "10:01"),
        (fixture.other_session, OTHER_AXIS, DriftDelta::Down, "10:04"),
        (fixture.session, OTHER_AXIS, DriftDelta::Down, "10:02"),
    ];
    for (session, axis, delta, clock) in ballots {
        insert_drift_vote_ignoring_checks(
            &fixture.store,
            fixture.character.get(),
            session.get(),
            axis,
            delta.label(),
            &format!("2026-08-23T{clock}:00.000Z"),
        )
        .await;
    }

    let recent = votes(&fixture.store, fixture.character, 3).await;
    assert_eq!(recent.len(), 3, "上限どおりの件数であること: {recent:?}");
    assert_eq!(
        recent,
        vec![
            // 10:04（4件目に入った）
            (
                fixture.other_session.get(),
                OTHER_AXIS.to_owned(),
                DriftDelta::Down.label()
            ),
            // 10:03（2件目に入った）
            (
                fixture.other_session.get(),
                VOTED_AXIS.to_owned(),
                DriftDelta::Down.label()
            ),
            // 10:02（5件目に入った＝最後に入ったが、時刻は3番目に新しい）
            (
                fixture.session.get(),
                OTHER_AXIS.to_owned(),
                DriftDelta::Down.label()
            ),
        ],
        "**時刻**の新しい順に返ること（入った順ではない）"
    );

    assert_eq!(
        votes(&fixture.store, fixture.character, PLENTY).await.len(),
        5,
        "上限を上げれば全部返ること（消えてはいない）"
    );

    fixture.store.close().await;
}

/// **同着は `id` で決まる**（`SELECT_ACTIVE_MEMORIES` と同じ形）。
///
/// # この状態はテストが作らないと存在しない
///
/// `created_at` はミリ秒精度なので、同着の組は**明示で作る**しかない。
/// 第2キーを落としても、**同着の組が無いテストしか無ければ緑のまま**通る
/// （14a で `memory` が実際に踏んだ穴）。
///
/// # ⚠ **これだけでは第2キーを守れない**（変異検査で実測 / 2026-08-23）
///
/// `, id DESC` を落とす変異を当てても、**このテストは緑のまま**通った。
/// 索引 `(character_id, created_at)` を後ろから辿る計画では、
/// **同着の中が偶然 `id` の降順になる**からである。
/// **権威は次のテスト**（`ties_are_ordered_by_id_even_when_the_index_is_not_used`）で、
/// あちらは索引を外して**計画を変えてから**測る。
#[tokio::test]
async fn votes_created_in_the_same_millisecond_are_ordered_by_id() {
    let fixture = fresh().await;
    const SAME_INSTANT: &str = "2026-08-23T10:00:00.000Z";

    let first = insert_drift_vote_ignoring_checks(
        &fixture.store,
        fixture.character.get(),
        fixture.session.get(),
        VOTED_AXIS,
        DriftDelta::Up.label(),
        SAME_INSTANT,
    )
    .await;
    let second = insert_drift_vote_ignoring_checks(
        &fixture.store,
        fixture.character.get(),
        fixture.other_session.get(),
        OTHER_AXIS,
        DriftDelta::Down.label(),
        SAME_INSTANT,
    )
    .await;
    assert!(first < second, "前提: 後の行の id が大きいこと");

    assert_eq!(
        votes(&fixture.store, fixture.character, 1).await,
        vec![(
            fixture.other_session.get(),
            OTHER_AXIS.to_owned(),
            DriftDelta::Down.label()
        )],
        "同じ時刻なら、後から入った票が新しい側になること"
    );

    fixture.store.close().await;
}

/// **同着の順が、問い合わせ計画に依存しない**（`, id DESC` の権威）。
///
/// # なぜ索引を外すのか
///
/// 索引 `(character_id, created_at)` を後ろから辿る計画では、
/// **同着の中が偶然 `id` の降順になる。** つまり第2キーが無くても結果は同じで、
/// **上のテストは第2キーを守れていない**（変異検査で実測 / 2026-08-23）。
///
/// 索引を外すと計画が「表を走査してから並べ替える」に変わり、
/// **同着の中は `id` の昇順になる**（`sqlite3` で実測: `1 2 3 4` と `4 3 2 1`）。
/// **順が逆になるので、第2キーが無いと必ず落ちる。**
///
/// # 索引を落としても本番の意味は変わらない
///
/// 索引は速さのためだけにある（`migrations/0010_stat_drift_vote.sql`）。
/// ここで落としているのは**このテスト専用の一時 DB** で、
/// マイグレーションには手を触れていない。
#[tokio::test]
async fn ties_are_ordered_by_id_even_when_the_index_is_not_used() {
    let fixture = fresh().await;
    const SAME_INSTANT: &str = "2026-08-23T11:00:00.000Z";

    // **同着を4件。** 中身は全部違う組にして、並びが読めるようにする。
    let ballots = [
        (fixture.session, VOTED_AXIS, DriftDelta::Up),
        (fixture.other_session, VOTED_AXIS, DriftDelta::Down),
        (fixture.session, OTHER_AXIS, DriftDelta::Up),
        (fixture.other_session, OTHER_AXIS, DriftDelta::Down),
    ];
    for (session, axis, delta) in ballots {
        insert_drift_vote_ignoring_checks(
            &fixture.store,
            fixture.character.get(),
            session.get(),
            axis,
            delta.label(),
            SAME_INSTANT,
        )
        .await;
    }

    // **計画を変える。** 索引が使えないと、同着の中は `id` の昇順で出てくる。
    sqlx::query("DROP INDEX stat_drift_vote_recent_idx")
        .execute(fixture.store.pool())
        .await
        .expect("索引を落とせること");

    let expected: Vec<(i64, String, &str)> = ballots
        .iter()
        .rev()
        .map(|(session, axis, delta)| (session.get(), (*axis).to_owned(), delta.label()))
        .collect();
    assert_eq!(
        votes(&fixture.store, fixture.character, PLENTY).await,
        expected,
        "索引が無くても、同着は入った順の逆（= id の降順）で返ること"
    );

    fixture.store.close().await;
}

/// **スキーマが向きの綴りを縛る**（`migrations/0010_stat_drift_vote.sql`）。
///
/// # 綴りは `DriftDelta::label` の側が正
///
/// **写しを作らない**という 16d の判断が、ここで実際に書き込んで確かめられる
/// ——`label()` を変えると、この書き込みが CHECK 違反で落ちる。
///
/// 同時に、**CHECK が本当に効いている**ことも見る
/// （効いていないと、読み出しの検証だけが最後の砦になる）。
#[tokio::test]
async fn every_direction_is_accepted_by_the_schema() {
    let fixture = fresh().await;
    let current = stats(&fixture.store, fixture.character).await;

    for delta in DriftDelta::ALL {
        fixture
            .store
            .record_drift_vote(
                fixture.character,
                fixture.session,
                &vote(&current, VOTED_AXIS, delta),
            )
            .await
            .unwrap_or_else(|err| panic!("{} を書き込めること: {err}", delta.label()));
    }

    // **語彙の外の綴りは CHECK が弾く。**
    let err = sqlx::query(
        "INSERT INTO stat_drift_vote (character_id, session_id, axis, delta) VALUES (?, ?, ?, ?)",
    )
    .bind(fixture.character.get())
    .bind(fixture.session.get())
    .bind(VOTED_AXIS)
    .bind("sideways")
    .execute(fixture.store.pool())
    .await
    .expect_err("語彙の外の向きは弾かれること");
    assert_eq!(
        err.as_database_error()
            .map(sqlx::error::DatabaseError::kind),
        Some(sqlx::error::ErrorKind::CheckViolation),
        "CHECK 違反として弾かれること: {err}"
    );

    assert_eq!(
        votes(&fixture.store, fixture.character, PLENTY).await.len(),
        2,
        "弾かれた行は入っていないこと"
    );

    fixture.store.close().await;
}

/// **1行壊れると蓄積の読みが止まる。記録は止まらない**（`recent_drift_votes` の doc）。
///
/// # 何が止まって、何が止まらないか
///
/// 止まるのは**そのキャラクターの票の読みだけ**。
/// **票の記録（追記）は既存の行を読まない**ので届き続ける。
///
/// # なぜ黙って飛ばさないのか
///
/// 蓄積は**数を数えて閾値と比べる**仕掛けなので、
/// 1票減ることは「変容が1回起きない／余計に起きる」に直結し、
/// **しかもどこにも現れない。**
#[tokio::test]
async fn a_broken_direction_stops_the_read_but_not_the_recording() {
    let fixture = fresh().await;
    let current = stats(&fixture.store, fixture.character).await;

    fixture
        .store
        .record_drift_vote(
            fixture.character,
            fixture.session,
            &vote(&current, VOTED_AXIS, DriftDelta::Up),
        )
        .await
        .expect("正当な票を記録できること");

    insert_drift_vote_ignoring_checks(
        &fixture.store,
        fixture.character.get(),
        fixture.session.get(),
        VOTED_AXIS,
        "sideways",
        "2026-08-23T10:00:00.000Z",
    )
    .await;

    assert!(
        matches!(
            fixture
                .store
                .recent_drift_votes(fixture.character, PLENTY)
                .await,
            Err(StoreError::InvalidStoredValue)
        ),
        "1行壊れると読み出しが丸ごと止まること"
    );

    // **追記は届き続ける。**
    fixture
        .store
        .record_drift_vote(
            fixture.character,
            fixture.other_session,
            &vote(&current, OTHER_AXIS, DriftDelta::Down),
        )
        .await
        .expect("壊れた行があっても票は記録できること");

    // **他のキャラクターの読みは止まらない。**
    assert!(
        votes(&fixture.store, fixture.bystander, PLENTY)
            .await
            .is_empty(),
        "別のキャラクターの読みは止まらないこと"
    );

    fixture.store.close().await;
}

/// **軸名が壊れていても同じ**（読めない行で止まる）。
///
/// 上限を超える軸名は `StatAxis::parse` が拒否するので、
/// **書き込みの検証を後から厳しくしたときと同じ形**になる
/// （`Store::list_characters` の doc）。
#[tokio::test]
async fn a_broken_axis_stops_the_read_the_same_way() {
    let fixture = fresh().await;
    let over_long = "あ".repeat(StatAxis::MAX_CHARS + 1);

    for broken in [over_long.as_str(), "   ", ""] {
        let fixture = &fixture;
        insert_drift_vote_ignoring_checks(
            &fixture.store,
            fixture.character.get(),
            fixture.session.get(),
            broken,
            DriftDelta::Up.label(),
            "2026-08-23T10:00:00.000Z",
        )
        .await;

        assert!(
            matches!(
                fixture
                    .store
                    .recent_drift_votes(fixture.character, PLENTY)
                    .await,
                Err(StoreError::InvalidStoredValue)
            ),
            "軸名 {broken:?} の行で読み出しが止まること"
        );
    }

    fixture.store.close().await;
}

/// **宛先の無い票は入らない。**
///
/// 持ち主も往復も、**存在しない id を書けない。**
/// 落とすと、票が「誰の・どの往復のものでもない」まま溜まる。
///
/// **作業単位 A-10 から、止めるのは外部キーではなく同じ文の会話の照合**（`INSERT_VOTE` の
/// `WHERE EXISTS`）。理由は、会話が無ければ `TurnSessionGone`、在るが別のキャラクターのものなら
/// `SessionOfAnotherCharacter`。**別のキャラクターの往復**にも書けなくなった
/// （外部キーだけでは書けていた / 下の3つ目）。
#[tokio::test]
async fn a_vote_without_a_home_is_rejected() {
    let fixture = fresh().await;
    let current = stats(&fixture.store, fixture.character).await;
    let ballot = vote(&current, VOTED_AXIS, DriftDelta::Up);

    let missing_character = fixture
        .store
        .record_drift_vote(
            CharacterId::from_raw_for_tests(9_999),
            fixture.session,
            &ballot,
        )
        .await;
    // 会話は在るが、その持ち主は 9999 ではない —— **取り違え**として返る（消えた、ではない）。
    assert!(
        matches!(
            missing_character,
            Err(StoreError::SessionOfAnotherCharacter)
        ),
        "存在しない持ち主には書けないこと: {missing_character:?}"
    );

    let missing_session = fixture
        .store
        .record_drift_vote(
            fixture.character,
            SessionId::from_raw_for_tests(9_999),
            &ballot,
        )
        .await;
    assert!(
        matches!(missing_session, Err(StoreError::TurnSessionGone)),
        "存在しない往復には書けないこと: {missing_session:?}"
    );

    // **別のキャラクターの往復**（作業単位 A-10）。会話は在るので、外部キーだけでは止まらない。
    let someone_elses_session = fixture
        .store
        .record_drift_vote(fixture.character, fixture.bystander_session, &ballot)
        .await;
    assert!(
        matches!(
            someone_elses_session,
            Err(StoreError::SessionOfAnotherCharacter)
        ),
        "別のキャラクターの往復には書けないこと（消えた、ではなく取り違え）: {someone_elses_session:?}"
    );

    assert!(
        votes(&fixture.store, fixture.character, PLENTY)
            .await
            .is_empty(),
        "弾かれた票は1件も入っていないこと"
    );
    assert!(
        votes(&fixture.store, fixture.bystander, PLENTY)
            .await
            .is_empty(),
        "別のキャラクターの側にも入っていないこと"
    );

    fixture.store.close().await;
}

/// **票を記録しても、性格ステータスは1バイトも変わらない**（ADR-0026 §決定1）。
///
/// 記録専用モードの土台がここにある——**この関数には許可証が要らない**ので、
/// 「票を書くついでに値も動かす」実装になっていないことを実挙動で押さえる。
#[tokio::test]
async fn recording_a_vote_never_moves_a_value() {
    let fixture = fresh().await;
    let before = stats(&fixture.store, fixture.character).await;

    for delta in DriftDelta::ALL {
        for axis in [VOTED_AXIS, OTHER_AXIS] {
            fixture
                .store
                .record_drift_vote(
                    fixture.character,
                    fixture.session,
                    &vote(&before, axis, delta),
                )
                .await
                .expect("票を記録できること");
        }
    }

    assert_eq!(
        stats(&fixture.store, fixture.character).await,
        before,
        "票を4件入れても値が動かないこと"
    );
    assert_eq!(
        votes(&fixture.store, fixture.character, PLENTY).await.len(),
        4,
        "前提: 票そのものは入っていること（何も起きていないのではない）"
    );

    fixture.store.close().await;
}
