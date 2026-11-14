//! **本文の想起の境の時刻**（`Store::recall_boundary`）と、**境で絞った想起**
//! （`Store::recall_memories` の `RecallScope::AsOf`）の検証（ADR-0064 / 作業単位 A-15b）。
//!
//! ここで固定するのは次の点:
//!
//!   * 境の時刻は、往復の数 N 以下で最大の区切り r（5, 15, 25, …）の **r 番目の往復の最後のメッセージ**の
//!     `created_at`。r が無い（N < 5）ならセッションの `created_at`。
//!     **返事の無い依頼も1往復**と数え、そのときの「最後のメッセージ」は依頼そのもの。
//!     **別のセッションの行は混ざらない**（数え方にも、選ぶ行にも）。
//!   * 境で絞った想起は、`created_at` が境の時刻**以前**（ちょうどを含む）の有効な記憶だけを返す。
//!     **訂正で入った行**（ほかの行の `superseded_by` に指されている行）は、境より新しくても素通りする。
//!     訂正を重ねても、最新の行が素通りする。
//!   * `RecallScope::Everything`（裏方の要求）は今までどおり有効な記憶の全件を返す。
//!
//! # 時刻の作り方
//!
//! `created_at` はミリ秒で同着しうるので、**どの行を選んだか**を値で見分けるために、
//! 行を書いた後で `created_at` を**行ごとに別の過去の時刻**へ直に書き換える（テストだけの細工）。
//! 過去（2000 年）にしてあるのは、`supersede_memory` が書く訂正の行（いまの時刻）が
//! 必ず境より新しくなるようにするため。

use personacore::domain::{
    CharacterId, CharacterUpdate, DisplayName, Handle, MemoryContent, MemoryId, NewMemory,
    NewMessage, Persona, Role, SessionId, Subject,
};
use personacore::store::{Memory, RecallScope, Store};
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

/// テスト用のキャラクターを1件作る。
async fn character(store: &Store, raw: &str) -> CharacterId {
    store
        .create_character(
            &Handle::parse(raw).expect("テストで使う名前は正当であること"),
            &DisplayName::parse(&format!("{raw}さん")).expect("正当な表示名であること"),
            &Persona::parse("ペルソナ本文").expect("正当なペルソナであること"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること")
}

/// 行ごとに別の過去の時刻（`2000-01-01T00:MM:SS.mmmZ`）。`index` が大きいほど新しい。
///
/// 形式は本番の DEFAULT（`strftime('%Y-%m-%dT%H:%M:%fZ','now')`）と同じ綴り。
fn at(index: u32) -> String {
    let minutes = index / 60;
    let seconds = index % 60;
    format!("2000-01-01T00:{minutes:02}:{seconds:02}.000Z")
}

fn message(role: Role, text: &str) -> NewMessage {
    NewMessage {
        role,
        dialogue: None,
        narration: None,
        raw_content: text.to_owned(),
    }
}

/// 1行の `message.created_at` を直に書き換える（テストだけの細工）。
async fn set_message_time(store: &Store, id: i64, stamp: &str) {
    let done = sqlx::query("UPDATE message SET created_at = ? WHERE id = ?")
        .bind(stamp)
        .bind(id)
        .execute(store.pool())
        .await
        .expect("時刻を書き換えられること");
    assert_eq!(done.rows_affected(), 1, "1行だけ書き換えること");
}

/// セッションの `created_at` を直に書き換える（テストだけの細工）。
async fn set_session_time(store: &Store, session: SessionId, stamp: &str) {
    let done = sqlx::query("UPDATE session SET created_at = ? WHERE id = ?")
        .bind(stamp)
        .bind(session.get())
        .execute(store.pool())
        .await
        .expect("時刻を書き換えられること");
    assert_eq!(done.rows_affected(), 1, "1行だけ書き換えること");
}

/// 往復 `turn` の時刻の番号。依頼は `2 * turn - 1`、応答は `2 * turn`（セッションは 0）。
fn user_index(turn: u32) -> u32 {
    2 * turn - 1
}
fn reply_index(turn: u32) -> u32 {
    2 * turn
}

/// 境の時刻の求め方（往復の数 0〜36、返事の無い依頼、別のセッションの行が間に挟まる形）。
///
/// 見る側のセッション A には、往復を1つ書くたびに**別のセッション B の往復を1つ挟む**（id が交互に並ぶ）。
/// B の行の時刻は A のどれよりも新しくしてあるので、選ぶ行の絞りから `session_id` が落ちると
/// B の行の時刻（2001 年）が返る。数え方から落ちると、区切りの位置がずれる。
///
/// A の往復 7 は**返事の無い依頼**（応答を書かない）。そのとき N = 7 の区切りは 5 のままで、
/// N = 15 で区切りになる往復 15 は普通の往復。返事の無い依頼が区切りの往復になる形は、
/// 下の [`an_unanswered_request_on_the_boundary_is_its_own_last_message`] で見る。
#[tokio::test]
async fn the_recall_boundary_is_the_end_of_the_latest_boundary_turn() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;
    let watched = store
        .create_session(aoi)
        .await
        .expect("セッションを作れること");
    let other_of_same = store
        .create_session(aoi)
        .await
        .expect("セッションを作れること");
    let other_of_another = store
        .create_session(rin)
        .await
        .expect("セッションを作れること");
    set_session_time(&store, watched, &at(0)).await;
    set_session_time(&store, other_of_same, "1999-12-31T23:59:59.000Z").await;
    set_session_time(&store, other_of_another, "1999-12-31T23:59:58.000Z").await;

    // (往復の数, 期待する境の時刻)。**期待値は関数の外の表**（区切りの往復をリテラルで書く）。
    let expected = |turns: u32| -> String {
        match turns {
            0..=4 => at(0),
            5..=14 => at(reply_index(5)),
            15..=24 => at(reply_index(15)),
            25..=34 => at(reply_index(25)),
            35..=36 => at(reply_index(35)),
            _ => unreachable!("この表は 36 往復まで"),
        }
    };

    let mut noise = 0;
    for turns in 0..=36 {
        let boundary = store
            .recall_boundary(watched)
            .await
            .expect("境の時刻を求められること");
        assert_eq!(
            boundary.as_stored(),
            expected(turns),
            "往復 {turns} 個のときの境の時刻"
        );
        if turns == 36 {
            break;
        }

        // 往復 `turns + 1` を書く。往復 7 だけ返事が無い。
        let turn = turns + 1;
        let user = store
            .append_message(watched, &message(Role::User, &format!("依頼{turn}")))
            .await
            .expect("依頼を書けること");
        set_message_time(&store, user.get(), &at(user_index(turn))).await;
        if turn != 7 {
            let reply = store
                .append_message(watched, &message(Role::Assistant, &format!("返事{turn}")))
                .await
                .expect("応答を書けること");
            set_message_time(&store, reply.get(), &at(reply_index(turn))).await;
        }

        // **別のセッションの往復を挟む**（同じキャラクターと、別のキャラクター）。
        for other in [other_of_same, other_of_another] {
            let appended = store
                .append_turn(
                    other,
                    &message(Role::User, "ほかの依頼"),
                    &message(Role::Assistant, "ほかの返事"),
                )
                .await
                .expect("ほかの往復を書けること");
            for id in [appended.user, appended.reply] {
                noise += 1;
                set_message_time(
                    &store,
                    id.get(),
                    &format!("2001-01-01T00:00:{:02}.{noise:03}Z", noise % 60),
                )
                .await;
            }
        }
    }
}

/// **返事の無い依頼が区切りの往復なら、境の時刻はその依頼の時刻**（r == N のとき・r < N のとき）。
#[tokio::test]
async fn an_unanswered_request_on_the_boundary_is_its_own_last_message() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store
        .create_session(aoi)
        .await
        .expect("セッションを作れること");
    set_session_time(&store, session, &at(0)).await;

    for turn in 1..=4 {
        let appended = store
            .append_turn(
                session,
                &message(Role::User, &format!("依頼{turn}")),
                &message(Role::Assistant, &format!("返事{turn}")),
            )
            .await
            .expect("往復を書けること");
        set_message_time(&store, appended.user.get(), &at(user_index(turn))).await;
        set_message_time(&store, appended.reply.get(), &at(reply_index(turn))).await;
    }
    // 往復 5 は返事の無い依頼（区切りの往復）。
    let fifth = store
        .append_message(session, &message(Role::User, "依頼5"))
        .await
        .expect("依頼を書けること");
    set_message_time(&store, fifth.get(), &at(user_index(5))).await;

    // r == N == 5: セッションの最後のメッセージ（＝ 返事の無い依頼）。
    let boundary = store
        .recall_boundary(session)
        .await
        .expect("求められること");
    assert_eq!(boundary.as_stored(), at(user_index(5)), "N = 5（r == N）");

    // 往復 6 を書いても、区切りの往復 5 の最後のメッセージ（依頼）のまま。
    let sixth = store
        .append_turn(
            session,
            &message(Role::User, "依頼6"),
            &message(Role::Assistant, "返事6"),
        )
        .await
        .expect("往復を書けること");
    set_message_time(&store, sixth.user.get(), &at(user_index(6))).await;
    set_message_time(&store, sixth.reply.get(), &at(reply_index(6))).await;
    let boundary = store
        .recall_boundary(session)
        .await
        .expect("求められること");
    assert_eq!(
        boundary.as_stored(),
        at(user_index(5)),
        "N = 6（往復 6 の依頼の手前で最も新しいメッセージ）"
    );
}

/// 記憶を1行入れ、`created_at` を指定の時刻へ直に書き換える（テストだけの細工）。主キーを返す。
async fn insert_memory_at(
    store: &Store,
    character_id: CharacterId,
    subject: Subject,
    content: &str,
    stamp: &str,
) -> MemoryId {
    let id = store
        .add_memory(character_id, &plain(subject, content))
        .await
        .expect("記憶を入れられること");
    let done = sqlx::query("UPDATE memory SET created_at = ? WHERE id = ?")
        .bind(stamp)
        .bind(id.get())
        .execute(store.pool())
        .await
        .expect("時刻を書き換えられること");
    assert_eq!(done.rows_affected(), 1, "1行だけ書き換えること");
    id
}

fn contents(memories: &[Memory]) -> Vec<&str> {
    memories
        .iter()
        .map(|memory| memory.content.as_str())
        .collect()
}

fn plain(subject: Subject, content: &str) -> NewMemory {
    NewMemory {
        subject,
        content: MemoryContent::parse(content).expect("正当な本文であること"),
        category: None,
    }
}

/// **境の時刻ちょうどの記憶は載り、1ミリ秒後の記憶は載らない**（主語に依らず・裏方は全件）。
///
/// 境はセッションの `created_at`（往復 0 個）。キャラクター自身の記憶も同じく絞る
/// （主語の絞りは `prompt` の仕事で、`store` は主語を見ない）。
#[tokio::test]
async fn recall_as_of_the_boundary_keeps_memories_up_to_and_including_it() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;
    let session = store
        .create_session(aoi)
        .await
        .expect("セッションを作れること");
    set_session_time(&store, session, "2000-01-01T00:00:10.000Z").await;

    // **書く順と時刻の順を食い違わせる**（並びが id ではなく作成時刻であることも一緒に見る）。
    for (who, subject, content, stamp) in [
        (aoi, Subject::User, "ずっと前", "2000-01-01T00:00:09.999Z"),
        (aoi, Subject::User, "ちょうど", "2000-01-01T00:00:10.000Z"),
        (aoi, Subject::User, "直後", "2000-01-01T00:00:10.001Z"),
        (
            aoi,
            Subject::Character,
            "自分の前",
            "2000-01-01T00:00:09.000Z",
        ),
        (
            aoi,
            Subject::Character,
            "自分の後",
            "2000-01-01T00:00:11.000Z",
        ),
        // 別のキャラクターの記憶は、どちらの形でも混ざらない。
        (rin, Subject::User, "よその前", "2000-01-01T00:00:01.000Z"),
    ] {
        insert_memory_at(&store, who, subject, content, stamp).await;
    }

    let boundary = store
        .recall_boundary(session)
        .await
        .expect("求められること");
    assert_eq!(boundary.as_stored(), "2000-01-01T00:00:10.000Z");

    let recalled = store
        .recall_memories(aoi, RecallScope::AsOf(&boundary))
        .await
        .expect("想起できること");
    assert_eq!(
        contents(&recalled),
        vec!["自分の前", "ずっと前", "ちょうど"],
        "境の時刻以前だけ（作成順）"
    );

    let everything = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起できること");
    assert_eq!(
        contents(&everything),
        vec!["自分の前", "ずっと前", "ちょうど", "直後", "自分の後"],
        "裏方の要求は全件"
    );
}

/// **訂正で入った行は、境より新しくても素通りする**（訂正を2回重ねた最新の行も）。
///
/// 訂正の行は `supersede_memory` が書く（いまの時刻 ＝ 境より新しい）。
/// 訂正された古い行は今までどおり外れる。**境より新しい普通の記憶は外れたまま**
/// （素通しを「新しい行は全部通す」に広げる変異はここで赤）。
#[tokio::test]
async fn corrections_pass_the_boundary_even_when_they_are_newer() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store
        .create_session(aoi)
        .await
        .expect("セッションを作れること");
    set_session_time(&store, session, "2000-01-01T00:00:10.000Z").await;

    let once = insert_memory_at(
        &store,
        aoi,
        Subject::User,
        "一度直す前",
        "2000-01-01T00:00:01.000Z",
    )
    .await;
    let twice = insert_memory_at(
        &store,
        aoi,
        Subject::User,
        "二度直す前",
        "2000-01-01T00:00:02.000Z",
    )
    .await;
    insert_memory_at(
        &store,
        aoi,
        Subject::User,
        "そのまま",
        "2000-01-01T00:00:03.000Z",
    )
    .await;
    // 境より新しい普通の記憶（抽出で入った形）。
    store
        .add_memory(aoi, &plain(Subject::User, "境の後の抽出"))
        .await
        .expect("記憶を入れられること");

    store
        .supersede_memory(aoi, once, &plain(Subject::User, "一度直した後"))
        .await
        .expect("訂正できること");
    let middle = store
        .supersede_memory(aoi, twice, &plain(Subject::User, "二度直す途中"))
        .await
        .expect("訂正できること");
    store
        .supersede_memory(aoi, middle, &plain(Subject::User, "二度直した後"))
        .await
        .expect("訂正を重ねられること");

    let boundary = store
        .recall_boundary(session)
        .await
        .expect("求められること");
    let recalled = store
        .recall_memories(aoi, RecallScope::AsOf(&boundary))
        .await
        .expect("想起できること");
    assert_eq!(
        contents(&recalled),
        vec!["そのまま", "一度直した後", "二度直した後"],
        "訂正の行は素通り、境の後の抽出は外れる"
    );

    let everything = store
        .recall_memories(aoi, RecallScope::Everything)
        .await
        .expect("想起できること");
    assert_eq!(
        contents(&everything),
        vec!["そのまま", "境の後の抽出", "一度直した後", "二度直した後"],
        "裏方の要求は有効な全件"
    );
}

/// **存在しないセッションの境は求められない**（`UnknownSession`。panic しない）。
///
/// 消える前に往復を 5 つ置く（区切りがある形）。会話が消えるのは**しまったキャラクターを削除したとき**
/// （`DELETE_CHARACTER` は、しまったキャラクターなら会話があっても消す。会話もメッセージも連鎖で消える）。
/// 本番と同じく、しまってから削除する。数えた往復は 0 になり、セッションの読みが `UnknownSession` を返す。
#[tokio::test]
async fn the_boundary_of_a_vanished_session_is_an_unknown_session() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store
        .create_session(aoi)
        .await
        .expect("セッションを作れること");
    for turn in 1..=5 {
        store
            .append_turn(
                session,
                &message(Role::User, &format!("依頼{turn}")),
                &message(Role::Assistant, &format!("返事{turn}")),
            )
            .await
            .expect("往復を書けること");
    }
    let handle = Handle::parse("aoi").expect("正当な名前であること");
    store
        .update_character(
            &handle,
            &CharacterUpdate {
                archived: Some(true),
                ..CharacterUpdate::default()
            },
        )
        .await
        .expect("しまえること");
    store
        .delete_character(&handle)
        .await
        .expect("しまったキャラクターは会話ごと削除できること");
    assert!(
        !store
            .session_exists(session)
            .await
            .expect("存在を確かめられること"),
        "前提: 会話が消えたこと"
    );
    let err = store
        .recall_boundary(session)
        .await
        .expect_err("消えたセッションの境は求められないこと");
    assert!(
        matches!(err, personacore::store::StoreError::UnknownSession),
        "{err:?}"
    );
}
