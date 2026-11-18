//! **その会話の返答が目安の下限を割ったか**（`Store::shortest_reply_short_of`）の検証
//! （[ADR-0066](../docs/adr/0066-the-end-token-is-lowered-once-a-session-has-shrunk.md) §決定2 / 作業単位 A-22b）。
//!
//! ここで固定するのは次の点:
//!
//!   * 返すのは、**下限より少ない字数の返答のうち最も短いものの字数**。割った返答が無ければ `None`。
//!   * **字数は Unicode のスカラー値の数**（バイト数でも UTF-16 の単位でもない）。日本語・絵文字・NUL を含む返答で見る。
//!   * **下限ちょうどは割っていない**（`<`）。
//!   * **数えるのは応答だけ**（利用者の短い発言は数えない）・**このセッションのものだけ**（別の会話の短い返答は数えない）。
//!   * **全部の返答を見る**（古い返答で割っていれば、後の返答が長くても割ったまま）。
//!
//! 期待値は手で書く（定数から作らない）。

use personacore::domain::{
    CharacterId, DisplayName, Handle, NewMessage, Persona, ResponseLength, Role, SessionId,
};
use personacore::store::Store;
use tempfile::TempDir;

mod common;

use common::fixture_scale;

/// 一時ディレクトリ上に新規 DB を作り、マイグレーション適用済みの `Store` を返す。
async fn setup() -> (TempDir, Store, CharacterId) {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let store = Store::connect(dir.path().join("personacore.sqlite"))
        .await
        .expect("接続とマイグレーション適用が成功すること");
    let character = store
        .create_character(
            &Handle::parse("kaede").expect("正当な名前であること"),
            &DisplayName::parse("楓").expect("正当な表示名であること"),
            &Persona::parse("ペルソナ本文").expect("正当なペルソナであること"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること");
    (dir, store, character)
}

fn message(role: Role, text: &str) -> NewMessage {
    NewMessage {
        role,
        dialogue: None,
        narration: None,
        raw_content: text.to_owned(),
    }
}

/// 往復を1つ書く（利用者の発言 `user` と、その返答 `reply`）。
async fn turn(store: &Store, session: SessionId, user: &str, reply: &str) {
    store
        .append_turn(
            session,
            &message(Role::User, user),
            &message(Role::Assistant, reply),
        )
        .await
        .expect("往復を書けること");
}

/// 目安（下限 `min`・上限 600）。
fn target(min: u32) -> ResponseLength {
    ResponseLength::parse(min, 600).expect("範囲の中")
}

async fn shortest(store: &Store, session: SessionId, min: u32) -> Option<u32> {
    store
        .shortest_reply_short_of(session, target(min))
        .await
        .expect("読めること")
}

/// **返答が無ければ `None`。割った返答の中で最も短い字数を返す。古い返答で割っていれば、後が長くても割ったまま。**
#[tokio::test]
async fn the_shortest_reply_below_the_lower_bound_is_reported_over_the_whole_session() {
    let (_dir, store, character) = setup().await;
    let session = store
        .create_session(character)
        .await
        .expect("会話を作れること");
    assert_eq!(shortest(&store, session, 10).await, None, "返答が無い会話");

    turn(&store, session, "a", "あいうえおかきくけこさ").await; // 11 字
    assert_eq!(
        shortest(&store, session, 10).await,
        None,
        "11 字は下限 10 を割らない"
    );
    turn(&store, session, "a", "あいうえおかき").await; // 7 字
    turn(&store, session, "a", "あいうえおかきくけこさしすせそ").await; // 15 字
    turn(&store, session, "a", "あいうえお").await; // 5 字
    turn(&store, session, "a", "あいうえおかきくけこさしすせそたち").await; // 17 字
    assert_eq!(
        shortest(&store, session, 10).await,
        Some(5),
        "割った返答（7 字・5 字）のうち最も短いもの。後の返答が長くても割ったまま"
    );
    assert_eq!(shortest(&store, session, 6).await, Some(5));
    assert_eq!(
        shortest(&store, session, 5).await,
        None,
        "下限ちょうどは割っていない"
    );
}

/// **字数は Unicode のスカラー値の数**（バイト数でも UTF-16 の単位でもない）。**NUL の後ろも数える。**
///
/// | 返答 | 字数 | バイト数 | UTF-16 の単位 |
/// |---|---|---|---|
/// | `あいう`（日本語3字） | 3 | 9 | 3 |
/// | `😀😀a`（絵文字2つと a） | 3 | 9 | 5 |
/// | `あ` NUL `いうえお` | 6 | 16 | 6 |
/// | `😀😀😀`（4 バイトの字だけ） | 3 | 12 | 6 |
///
/// 下限 4 なら3つとも4字未満かを問う: 前の2つは割り（3 字）、NUL を含む返答は割らない（6 字。**NUL の手前までしか
/// 数えない実装は 1 字と読んで割ったことにする**）。
///
/// **4 バイトの字だけの返答は、バイト数での絞り込みの係数（`UTF8_MAX_BYTES_PER_CHAR` = 4）を押さえる** —— 3 字・12 バイトで
/// 下限 4 なら、係数 4 の上限は 16 で読まれ、係数 3 の上限は 12（`12 < 12` が偽）で落ちて「割っていない」になる。
#[tokio::test]
async fn characters_are_counted_as_unicode_scalar_values_including_past_a_nul() {
    for (reply, min, expected) in [
        ("あいう", 4, Some(3)),
        ("あいう", 3, None),
        ("😀😀a", 4, Some(3)),
        ("😀😀a", 3, None),
        ("あ\u{0}いうえお", 4, None),
        ("あ\u{0}いうえお", 7, Some(6)),
        ("😀😀😀", 4, Some(3)),
    ] {
        let (_dir, store, character) = setup().await;
        let session = store
            .create_session(character)
            .await
            .expect("会話を作れること");
        turn(&store, session, "a", reply).await;
        assert_eq!(
            shortest(&store, session, min).await,
            expected,
            "{reply:?} / 下限 {min}"
        );
    }
}

/// **数えるのは応答だけ・このセッションのものだけ。**
///
/// 会話 A に短い返答（2 字）、会話 B には短い利用者の発言（1 字）と長い返答（12 字）だけ。
/// **B は割っていない**（利用者の発言を数える実装・会話を区別しない実装は B で `Some` を返して赤になる）。
/// **対照**: A は割っている。
#[tokio::test]
async fn only_this_sessions_replies_are_counted() {
    let (_dir, store, character) = setup().await;
    let other = store
        .create_session(character)
        .await
        .expect("会話を作れること");
    let session = store
        .create_session(character)
        .await
        .expect("会話を作れること");
    turn(&store, other, "長い利用者の発言です", "はい").await;
    turn(&store, session, "a", "あいうえおかきくけこさし").await;
    assert_eq!(
        shortest(&store, session, 10).await,
        None,
        "B は割っていない"
    );
    assert_eq!(
        shortest(&store, other, 10).await,
        Some(2),
        "対照: A は割っている"
    );
}
