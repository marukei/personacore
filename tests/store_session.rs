//! `session` / `message` の永続化の検証。
//!
//! ここで確かめるのは主に次の6点:
//!   * **「最新セッション」は `updated_at` 基準**であること（`created_at` ではない / ADR-0004）。
//!     ただし**中身のあるセッションを先に見る**（B-2b）。上流が開けなかった回・
//!     打ち切られた回に残る空セッションが「最新」を横取りしない。
//!   * **メッセージ追加が `session.updated_at` を必ず進める**こと。片方だけ実行できない。
//!   * **`updated_at` が同一キャラ内で単調増加する**こと。時計が進まなくても、
//!     既存行が未来時刻でも順序が壊れない。押し上げは `INSERT` 側にも要る
//!     （無いと `aoi@new` の直後に旧セッションを継続する）。
//!   * **`latest_or_create_session` が同時呼び出しで壊れない**こと。
//!     読み→書きの昇格は `busy_timeout` が効かないため `BEGIN IMMEDIATE` が要る。
//!   * **`aoi@<id>` の照合が、そのキャラのセッションであることを確認する**こと。
//!     他キャラのセッション id を渡したら `None`（緩めると別キャラの記憶が混ざる）。
//!   * **`raw_content` を落とさない**こと。`dialogue` / `narration` が両方 `None` でも、
//!     空文字でも保存できる。
//!
//! スキーマ制約そのものの検証は `tests/store_schema.rs` にある。

use chrono::Utc;
use personacore::domain::{CharacterId, DisplayName, Handle, NewMessage, Persona, Role, SessionId};
use personacore::store::{Store, StoreError};
use sqlx::error::ErrorKind;
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

fn persona(raw: &str) -> Persona {
    Persona::parse(raw).expect("テストで使うペルソナは正当であること")
}

/// テスト用のキャラクターを1件作る。
async fn character(store: &Store, raw: &str) -> CharacterId {
    let name = Handle::parse(raw).expect("テストで使う名前は正当であること");
    store
        .create_character(
            &name,
            &DisplayName::parse(raw).expect("テストで使う表示名は正当であること"),
            &persona("ペルソナ本文"),
            None,
            fixture_scale(),
        )
        .await
        .expect("キャラクターを作成できること")
}

/// パース前の生出力だけを持つメッセージ（パース失敗時の形）。
fn raw_only(role: Role, raw_content: &str) -> NewMessage {
    NewMessage {
        role,
        dialogue: None,
        narration: None,
        raw_content: raw_content.to_owned(),
    }
}

/// 往復1組のフィクスチャ（利用者の発言 → 応答）。
///
/// **利用者側と応答側で、`role` も3つの本文もすべて別の値にしてある。**
/// どこか2つを同値にすると、その2つの取り違え（応答側に利用者側の `raw_content` を書く、
/// `dialogue` に `narration` を書く等）が**テストを緑のまま通り抜ける。**
/// 入口で全部が相異なることを固定しておく。
fn turn() -> (NewMessage, NewMessage) {
    let user = NewMessage {
        role: Role::User,
        dialogue: Some("今日はどこへ行こうか".to_owned()),
        narration: Some("利用者は地図を広げた".to_owned()),
        raw_content: "「今日はどこへ行こうか」\n利用者は地図を広げた".to_owned(),
    };
    let reply = NewMessage {
        role: Role::Assistant,
        dialogue: Some("海が見たいな".to_owned()),
        narration: Some("葵は窓の外を指した".to_owned()),
        raw_content: "「海が見たいな」\n葵は窓の外を指した".to_owned(),
    };

    assert_ne!(
        user.role, reply.role,
        "role が同値だと取り違えを検出できない"
    );
    let texts = [
        user.dialogue.as_deref(),
        user.narration.as_deref(),
        Some(user.raw_content.as_str()),
        reply.dialogue.as_deref(),
        reply.narration.as_deref(),
        Some(reply.raw_content.as_str()),
    ];
    let mut unique = texts.to_vec();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        texts.len(),
        "本文6つがすべて相異なること（同値があるとその取り違えが検出できない）: {texts:?}"
    );

    (user, reply)
}

/// `session` の行数。
async fn session_count(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM session")
        .fetch_one(store.pool())
        .await
        .expect("件数を取得できること")
}

/// `message` の行数。
async fn message_count(store: &Store) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM message")
        .fetch_one(store.pool())
        .await
        .expect("件数を取得できること")
}

/// そのセッションに入っている `message` の行数。
///
/// 「中身のあるセッション」を見張るテストの**前提**を明示するために使う
/// （空のつもりのセッションに実は行が入っていた、という取り違えを潰す）。
async fn message_count_of(store: &Store, session_id: SessionId) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM message WHERE session_id = ?")
        .bind(session_id.get())
        .fetch_one(store.pool())
        .await
        .expect("件数を取得できること")
}

/// `updated_at` を直接書き換える（**時計に依存せず順序を組み立てるため**）。
///
/// 同着の組を作るのは `store` 経由では原理的にできない（書き込み側が単調増加を
/// 保証している）ので、直接書き込む。**同着は DB を直接触ったときだけの状況ではない** ——
/// `latest_session` の `id DESC` はその状況でしか通らない枝なので、
/// 全件が別の値だとその枝を一度も踏まないまま緑になる。
async fn set_updated_at(store: &Store, session_id: SessionId, at: &str) {
    let done = sqlx::query("UPDATE session SET updated_at = ? WHERE id = ?")
        .bind(at)
        .bind(session_id.get())
        .execute(store.pool())
        .await
        .expect("直接の UPDATE が成功すること");
    assert_eq!(done.rows_affected(), 1, "対象のセッションが1行あること");
}

/// 格納されている `updated_at` の生の文字列。
async fn stored_updated_at(store: &Store, session_id: SessionId) -> String {
    sqlx::query_scalar("SELECT updated_at FROM session WHERE id = ?")
        .bind(session_id.get())
        .fetch_one(store.pool())
        .await
        .expect("updated_at を取得できること")
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

/// 作成 → id で取得、の往復。
#[tokio::test]
async fn create_then_find_by_id() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    // `created_at` は SQLite が書くので、`Utc::now()` とは別の時計になる（`CLOCK_SKEW`）。
    let before = Utc::now() - CLOCK_SKEW;

    let id = store
        .create_session(aoi)
        .await
        .expect("セッションを作成できること");

    let found = store
        .find_session(aoi, id.get())
        .await
        .expect("検索が成功すること")
        .expect("作成したセッションを引けること");

    assert_eq!(found.id, id);
    assert_eq!(found.character_id, aoi);
    assert_eq!(found.label, None, "ラベルは未設定（フェーズ1では付けない）");
    assert!(
        found.created_at >= before && found.created_at <= Utc::now() + CLOCK_SKEW,
        "created_at に妥当な日時が入ること: {}",
        found.created_at
    );
    assert!(
        found.updated_at >= found.created_at,
        "updated_at が created_at を下回らないこと: {} / {}",
        found.updated_at,
        found.created_at
    );
}

/// **`aoi@<id>` の照合は、そのセッションが本当にそのキャラのものかを確認する**（ADR-0004）。
///
/// ここを緩めると「別キャラの記憶が混ざる」が API 経由で成立する。
#[tokio::test]
async fn session_of_another_character_is_not_found() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;

    let aoi_session = store
        .create_session(aoi)
        .await
        .expect("セッションを作成できること");

    assert!(
        store
            .find_session(aoi, aoi_session.get())
            .await
            .expect("検索が成功すること")
            .is_some(),
        "本人のセッションは引けること"
    );
    assert!(
        store
            .find_session(rin, aoi_session.get())
            .await
            .expect("検索が成功すること")
            .is_none(),
        "他キャラのセッション id は引けないこと"
    );
    assert!(
        store
            .latest_session(rin)
            .await
            .expect("検索が成功すること")
            .is_none(),
        "他キャラのセッションは最新セッションにもならないこと"
    );
}

/// 存在しないセッション id は `None`。
#[tokio::test]
async fn unknown_session_id_is_not_found() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    assert!(
        store
            .find_session(aoi, 9_999)
            .await
            .expect("検索が成功すること")
            .is_none(),
        "存在しないセッション id は None であること"
    );
    assert!(
        store
            .latest_session(aoi)
            .await
            .expect("検索が成功すること")
            .is_none(),
        "1件も無ければ最新セッションは None であること"
    );
}

/// **`find_session` は `aoi@42` の `42`（未検証の生の `i64`）を受け取る入口**（ADR-0004）。
///
/// 所有確認を通ったときだけ `SessionId` が得られる、という不変条件をここで固定する。
/// 範囲外・負値・存在しない id を渡しても panic せず `None` になること。
#[tokio::test]
async fn find_session_takes_an_untrusted_raw_id() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");

    let found = store
        .find_session(aoi, session.get())
        .await
        .expect("検索が成功すること")
        .expect("所有していれば引けること");
    assert_eq!(
        found.id, session,
        "所有確認を通った id だけが SessionId になる"
    );

    for raw in [0_i64, -1, i64::MIN, i64::MAX, 9_999] {
        assert!(
            store
                .find_session(aoi, raw)
                .await
                .expect("検索が成功すること")
                .is_none(),
            "未検証の値でも panic せず None であること: raw = {raw}"
        );
    }
}

/// `aoi` の意味論: 最新セッションが無ければ作り、あればそれを継続する。
#[tokio::test]
async fn latest_or_create_creates_once_then_continues() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let created = store
        .latest_or_create_session(aoi)
        .await
        .expect("最初の呼び出しで作成できること");
    assert_eq!(session_count(&store).await, 1, "1件だけ作られること");

    let continued = store
        .latest_or_create_session(aoi)
        .await
        .expect("2回目も成功すること");
    assert_eq!(continued.id, created.id, "同じセッションを継続すること");
    assert_eq!(
        session_count(&store).await,
        1,
        "2回目は新しいセッションを作らないこと"
    );

    // 明示的に作った新規セッションがあれば、そちらを継続する（`aoi@new` の直後）。
    let fresh = store
        .create_session(aoi)
        .await
        .expect("セッションを作成できること");
    let continued = store
        .latest_or_create_session(aoi)
        .await
        .expect("継続できること");
    assert_eq!(continued.id, fresh, "新しく作った方を継続すること");
    assert_eq!(session_count(&store).await, 2, "余分に作らないこと");
}

/// **`aoi` の経路は同時に叩かれても落ちず、セッションを2つ作らない。**
///
/// `latest_or_create_session` は「読んでから書く」ため、素の `BEGIN`（deferred）だと
/// 書きへの昇格が `SQLITE_BUSY_SNAPSHOT`(517) になる。これは
/// **`busy_timeout` が一切効かず即座に失敗する**ので、FE が2本同時に投げただけで
/// 片方が落ちる。`BEGIN IMMEDIATE` で開始すれば書き↔書きの待ちに変わって待てる
/// （ADR-0004「書き込みの順序」）。
///
/// `BEGIN IMMEDIATE` を素の `BEGIN` に戻すと、失敗するか空セッションが複数できる。
///
/// **タスク数を減らさないこと。** 競合は毎回起きるとは限らないので、少ないと
/// 回帰を見逃す。実測（素の `BEGIN` に戻した状態での検出率）:
///
/// | タスク数 | 検出 | 見逃し |
/// |---|---|---|
/// | 8 | 30/35 | **5/35（約14%）** |
/// | 32 | 20/20 | 0 |
///
/// 正しい実装での偽陽性は 32 本でも 0/20。実行時間もほぼ変わらない
/// （プール既定の `max_connections` は 10 なので接続も足りる）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn latest_or_create_survives_concurrent_callers() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let tasks: Vec<_> = (0..32)
        .map(|_| {
            let store = store.clone();
            tokio::spawn(async move { store.latest_or_create_session(aoi).await })
        })
        .collect();

    let mut ids = Vec::new();
    for task in tasks {
        let session = task
            .await
            .expect("タスクが panic しないこと")
            .expect("同時に呼ばれても失敗しないこと（読み→書きの昇格で落ちない）");
        ids.push(session.id);
    }

    assert_eq!(
        session_count(&store).await,
        1,
        "同時に呼ばれてもセッションは1つしか作られないこと"
    );
    assert!(
        ids.iter().all(|id| *id == ids[0]),
        "全員が同じセッションを継続すること: {ids:?}"
    );
}

/// セッションはキャラクターごとに独立している。
#[tokio::test]
async fn sessions_are_scoped_to_one_character() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;

    let aoi_session = store
        .latest_or_create_session(aoi)
        .await
        .expect("作成できること");
    let rin_session = store
        .latest_or_create_session(rin)
        .await
        .expect("作成できること");

    assert_ne!(aoi_session.id, rin_session.id);
    assert_eq!(
        store
            .latest_session(aoi)
            .await
            .expect("検索が成功すること")
            .expect("最新セッションがあること")
            .id,
        aoi_session.id,
        "別キャラのセッションが最新として返らないこと"
    );
}

/// 存在しないキャラクターにはセッションを作れない（外部キー違反）。
#[tokio::test]
async fn session_of_unknown_character_cannot_be_created() {
    let (_dir, store) = setup().await;
    let unknown = CharacterId::from_raw_for_tests(9_999);

    let err = store
        .create_session(unknown)
        .await
        .expect_err("存在しないキャラクターには作成できないこと");
    assert_query_kind(&err, ErrorKind::ForeignKeyViolation, "session.character_id");

    let err = store
        .latest_or_create_session(unknown)
        .await
        .expect_err("暗黙の作成でも同じこと");
    assert_query_kind(&err, ErrorKind::ForeignKeyViolation, "session.character_id");
}

/// **メッセージ追加は `session.updated_at` を進め、最新セッションの解決結果を動かす**（ADR-0004）。
///
/// これが「最新 = `updated_at`」の実質的な意味。`created_at` 基準の実装では
/// 後から作ったセッションが最新のままになり、このテストが落ちる。
#[tokio::test]
async fn appending_a_message_moves_the_latest_session() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let first = store.create_session(aoi).await.expect("作成できること");
    let second = store.create_session(aoi).await.expect("作成できること");

    let latest = store
        .latest_session(aoi)
        .await
        .expect("検索が成功すること")
        .expect("最新セッションがあること");
    assert_eq!(latest.id, second, "後から作った方が最新であること");

    let first_before = store
        .find_session(aoi, first.get())
        .await
        .expect("検索が成功すること")
        .expect("セッションがあること")
        .updated_at;
    let second_before = stored_updated_at(&store, second).await;

    store
        .append_message(first, &raw_only(Role::User, "こんにちは"))
        .await
        .expect("メッセージを追加できること");

    let latest = store
        .latest_session(aoi)
        .await
        .expect("検索が成功すること")
        .expect("最新セッションがあること");
    assert_eq!(
        latest.id, first,
        "メッセージを追加した側が最新セッションになること"
    );
    assert!(
        latest.updated_at > first_before,
        "updated_at が進むこと: {} → {}",
        first_before,
        latest.updated_at
    );
    assert_eq!(
        stored_updated_at(&store, second).await,
        second_before,
        "他のセッションの updated_at は動かないこと"
    );
}

/// **同一ミリ秒でも `updated_at` は必ず進む。**
///
/// `0001` の `updated_at` はミリ秒精度なので、時計が進まないと同値になり、
/// 最新セッションの解決が不定になる。`store` は「そのキャラの最大値 + 1ms」まで
/// 押し上げることで順序を決定的にしている。ここではシステム時刻より未来の値を
/// 直接書き込むことで、時計に依存せずその挙動を固定する。
///
/// 単純に `strftime('now')` を代入する実装に戻すと、2999年の値より小さくなり
/// 最新セッションが動かないため、このテストが落ちる。
#[tokio::test]
async fn updated_at_advances_even_when_the_clock_does_not() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let first = store.create_session(aoi).await.expect("作成できること");
    let second = store.create_session(aoi).await.expect("作成できること");

    // 2つのセッションの updated_at を同値（かつシステム時刻より未来）に揃える。
    sqlx::query("UPDATE session SET updated_at = ?")
        .bind("2999-01-01T00:00:00.000Z")
        .execute(store.pool())
        .await
        .expect("直接の UPDATE が成功すること");

    store
        .append_message(first, &raw_only(Role::User, "こんにちは"))
        .await
        .expect("メッセージを追加できること");

    assert_eq!(
        stored_updated_at(&store, first).await,
        "2999-01-01T00:00:00.001Z",
        "同値・未来の時刻でも 1ms 進むこと"
    );
    assert_eq!(
        stored_updated_at(&store, second).await,
        "2999-01-01T00:00:00.000Z",
        "他のセッションは動かないこと"
    );

    let latest = store
        .latest_session(aoi)
        .await
        .expect("検索が成功すること")
        .expect("最新セッションがあること");
    assert_eq!(
        latest.id, first,
        "同一ミリ秒でも、追加した側へ最新セッションが移ること"
    );
}

/// **セッション作成（`INSERT`）にも `updated_at` の押し上げが要る**（ADR-0004）。
///
/// 既存セッションの `updated_at` がシステム時刻より未来にある状態で `aoi@new` を打つと、
/// 押し上げの無い実装（`strftime('now')` をそのまま代入）では新しい行の方が小さくなり、
/// **新規セッションを作った直後に `aoi` が旧セッションを継続する。**
/// ADR-0004 が最も避けたい「気づきにくい記憶の混線」そのもの。
///
/// 通常の作成順を見るテストではこの変異を検出できない（未来時刻の既存行が無い状況では
/// 押し上げの有無で結果が変わらず、同一ミリ秒でも `ORDER BY … id DESC` が正解を拾う）。
/// そのため未来時刻を直接書き込んで固定する。
///
/// # ⚠ **末尾の主張が成り立つのは「旧セッションが空のとき」だけ**（B-2b で狭まった）
///
/// `latest_session` は**中身のあるセッションを先に見る**ようになったので、
/// **旧セッションに往復が1組でも入っていると、`aoi` は新規ではなく旧セッションを返す。**
/// ここが緑なのは、このフィクスチャの `old` に `message` が1件も無いからである。
///
/// **`updated_at` の押し上げそのものは、この狭まりと無関係に要る**
/// （途中の2つのアサートが押し上げを直接見ている。並び順の話ではない）。
///
/// 狭まった側の状態——**中身のある旧セッション ＋ それより新しい空の `@new`**——は
/// `a_fresh_session_loses_to_an_older_conversation_until_its_first_message_lands`
/// が作って固定している。**散文だけにしないこと。**
#[tokio::test]
async fn create_session_becomes_latest_even_against_a_future_timestamp() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let old = store.create_session(aoi).await.expect("作成できること");

    // 既存セッションをシステム時刻より未来に置く（`aoi@42` で復旧した後などに
    // `append_message` の押し上げで実際に到達しうる状態）。
    sqlx::query("UPDATE session SET updated_at = ? WHERE id = ?")
        .bind("2999-01-01T00:00:00.000Z")
        .bind(old.get())
        .execute(store.pool())
        .await
        .expect("直接の UPDATE が成功すること");

    let fresh = store
        .create_session(aoi)
        .await
        .expect("セッションを作成できること");

    assert_eq!(
        stored_updated_at(&store, fresh).await,
        "2999-01-01T00:00:00.001Z",
        "新規セッションの updated_at が既存の最大値より 1ms 大きいこと"
    );
    assert_eq!(
        stored_updated_at(&store, old).await,
        "2999-01-01T00:00:00.000Z",
        "既存セッションの updated_at は動かないこと"
    );
    assert_eq!(
        store
            .latest_session(aoi)
            .await
            .expect("検索が成功すること")
            .expect("最新セッションがあること")
            .id,
        fresh,
        "未来時刻の既存セッションがあっても、新規作成が最新になること"
    );
}

/// 別キャラクターの未来時刻には引きずられない（押し上げはキャラクター単位）。
///
/// `character_id` の条件を外す変異を入れると、`rin` の未来時刻に合わせて
/// `aoi` の `updated_at` が 2999 年へ飛ぶため、このテストが落ちる。
#[tokio::test]
async fn create_session_is_not_pushed_by_another_characters_timestamp() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;

    let rin_session = store.create_session(rin).await.expect("作成できること");
    sqlx::query("UPDATE session SET updated_at = ? WHERE id = ?")
        .bind("2999-01-01T00:00:00.000Z")
        .bind(rin_session.get())
        .execute(store.pool())
        .await
        .expect("直接の UPDATE が成功すること");

    // `updated_at` は `store` が組み立てるが、比較相手の `created_at` 系と同じく
    // 別の時計と突き合わせることになるため `CLOCK_SKEW` を使う。
    let before = Utc::now() - CLOCK_SKEW;
    let aoi_session = store.create_session(aoi).await.expect("作成できること");
    let updated_at = store
        .find_session(aoi, aoi_session.get())
        .await
        .expect("検索が成功すること")
        .expect("セッションがあること")
        .updated_at;

    assert!(
        updated_at >= before && updated_at <= Utc::now() + CLOCK_SKEW,
        "他キャラの未来時刻に引きずられず実時刻が入ること: {updated_at}"
    );
}

// ----------------------------------------------------------------------
// 「最新」は中身のあるセッションを先に見る（作業単位 B-2b）
// ----------------------------------------------------------------------

/// そのセッションを「中身のあるセッション」にする（往復を1組書く）。
async fn fill(store: &Store, session: SessionId) {
    let (user, reply) = turn();
    store
        .append_turn(session, &user, &reply)
        .await
        .expect("往復を追加できること");
}

/// **メッセージが1件も入っていないセッションは「最新」の座を取らない**（B-2b）。
///
/// `server::chat` は上流を開くより前にセッションを作るので、上流が開けなかった回は
/// **`session` の行だけが残る**（`message` は0件）。中断された回も同じで、
/// [ADR-0019](docs/adr/0019-relay-idle-watchdog.md) §4 により1行も保存されない。
/// **つまり空セッションは異常系だけの現象ではなく、正常系の中にも居る。**
///
/// `INSERT` でも `updated_at` は押し上がる（ADR-0004）ので、`updated_at` だけで選ぶと
/// **その空セッションが最新になり、素の `aoi` が会話ではなく空の続きに着地する。**
/// 画面上は普通に見えるため、履歴が分断されたことに気付けない。
///
/// **空の方を「新しく」しておくのがこのテストの要**。逆だと `updated_at` だけを見る
/// 実装でも正解を返してしまい、何も見ていないことになる。
#[tokio::test]
async fn a_session_with_messages_outranks_a_newer_empty_one() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let talked = store.create_session(aoi).await.expect("作成できること");
    fill(&store, talked).await;
    // 上流が開けなかった回の残骸（`chat` はここまで進んでから 502 を返す）。
    let abandoned = store.create_session(aoi).await.expect("作成できること");

    // 前提を明示する。この2つが崩れると、このテストは何も見ていない。
    assert!(
        stored_updated_at(&store, abandoned).await > stored_updated_at(&store, talked).await,
        "空セッションの方が新しいこと: 空 = {} / 会話 = {}",
        stored_updated_at(&store, abandoned).await,
        stored_updated_at(&store, talked).await
    );
    assert_eq!(
        message_count_of(&store, abandoned).await,
        0,
        "残骸に行が入っていないこと"
    );
    assert_eq!(
        message_count_of(&store, talked).await,
        2,
        "会話の方には往復が入っていること"
    );

    let latest = store
        .latest_session(aoi)
        .await
        .expect("検索が成功すること")
        .expect("最新セッションがあること");
    assert_eq!(
        latest.id, talked,
        "中身のある方が最新であること（空の残骸に着地しない）"
    );

    // `aoi` の経路（暗黙の作成つき）も同じ判断をし、余分な行も作らない。
    let resolved = store
        .latest_or_create_session(aoi)
        .await
        .expect("解決できること");
    assert_eq!(
        resolved.id, talked,
        "aoi の経路も中身のある方を継続すること"
    );
    assert_eq!(
        session_count(&store).await,
        2,
        "解決のために新しいセッションを作らないこと"
    );
}

/// **`aoi@new` で作った直後の空セッションは、まだ「最新」にならない**（B-2b で狭まった保証）。
///
/// `create_session_becomes_latest_even_against_a_future_timestamp` は
/// 「`@new` の直後に `aoi` が旧セッションへ戻らないこと」を固定しているが、
/// **その主張が成り立つのは旧セッションが空のときだけ**になった。
/// **中身のある旧セッションがあると、最初のメッセージが保存されるまでの窓では
/// 素の `aoi` が旧セッションを返す。**
///
/// # 司令塔の判断: **この狭まりを受け入れる**（2026-08-27）
///
/// * **空セッションには継続すべき中身が無い。**
/// * **`@new` を出したクライアントは応答の `model`（`aoi@<id>`）を受け取って固定する**ので、
///   成功した `@new` がこの窓に依存することはない（下で `find_session` が届くことも見る）。
/// * 窓に入るのは**その回が1行も保存されなかったとき**か、**2つ目のタブ**。
///
/// **ロードマップが却下した「失敗したら `@new` を降ろす」とは別物。**
/// あちらは `@new` の意味を変える話で、こちらは「最新」の意味を変えた結果、
/// **中身の無いセッションが継続の対象から外れた**という話である。
/// **`@new` は今も必ず新規を作るし、明示した `aoi@<id>` は今も届く。**
#[tokio::test]
async fn a_fresh_session_loses_to_an_older_conversation_until_its_first_message_lands() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let talked = store.create_session(aoi).await.expect("作成できること");
    fill(&store, talked).await;
    // `aoi@new` が作った直後の行（まだ1件も入っていない）。
    let fresh = store.create_session(aoi).await.expect("作成できること");

    // 前提: 押し上げは効いていて `fresh` の方が新しい。
    // **この狭まりは並び順の話であって、押し上げが壊れた結果ではない。**
    assert!(
        stored_updated_at(&store, fresh).await > stored_updated_at(&store, talked).await,
        "@new で作った方が新しいこと: 新 = {} / 旧 = {}",
        stored_updated_at(&store, fresh).await,
        stored_updated_at(&store, talked).await
    );
    assert_eq!(
        message_count_of(&store, fresh).await,
        0,
        "作った直後は空であること"
    );
    assert_eq!(
        message_count_of(&store, talked).await,
        2,
        "旧セッションには中身があること（この前提が崩れると窓に入らない）"
    );

    // **窓の中** —— 素の `aoi` は中身のある方を返す。
    assert_eq!(
        store
            .latest_session(aoi)
            .await
            .expect("検索が成功すること")
            .expect("最新セッションがあること")
            .id,
        talked,
        "最初のメッセージが入るまでは、中身のある旧セッションが返ること"
    );

    // **明示すれば届く。** `@new` の応答は `aoi@<id>` を返すので、実クライアントはこちらを通る。
    assert_eq!(
        store
            .find_session(aoi, fresh.get())
            .await
            .expect("検索が成功すること")
            .expect("id で引けること")
            .id,
        fresh,
        "@new が返した id は、窓の中でも届くこと"
    );

    // **窓は最初のメッセージまで。** 往復が1組入れば `fresh` が最新になる。
    fill(&store, fresh).await;
    assert_eq!(
        store
            .latest_session(aoi)
            .await
            .expect("検索が成功すること")
            .expect("最新セッションがあること")
            .id,
        fresh,
        "1組でも入れば、以後は新しい方を継続すること"
    );
}

/// **空セッションしか無いときは、いちばん新しい空セッションをそのまま使う**（B-2b の決定）。
///
/// 空セッションを「除外」ではなく「順位を下げる」形にしたのはこのため。除外すると、
/// 空しか無い状態で `latest_or_create_session` が**呼ばれるたびに新しい行を作る**ので、
/// 上流が落ちている間じゅうセッションが増え続ける（しかも中身は全部空）。
/// 空セッションは履歴が空なので**新規に作るのと振る舞いが区別できない。**増やさない方を採る。
///
/// **`updated_at` が同着の組を作ってある。** 全件が別の値だと `id DESC` の枝を
/// 一度も通らない（同一ミリ秒の作成は本番でも起こりうる）。
/// さらに「id は最大だが古い」1件を混ぜてあるので、`id` を先に見る変異もここで落ちる。
#[tokio::test]
async fn when_every_session_is_empty_the_newest_empty_one_is_reused() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let tied_older = store.create_session(aoi).await.expect("作成できること");
    let tied_newer = store.create_session(aoi).await.expect("作成できること");
    let stale = store.create_session(aoi).await.expect("作成できること");
    assert!(
        tied_older.get() < tied_newer.get() && tied_newer.get() < stale.get(),
        "id は作成順に増えること（この前提の上で並び順を組み立てている）"
    );

    set_updated_at(&store, tied_older, "2999-01-01T00:00:00.007Z").await;
    set_updated_at(&store, tied_newer, "2999-01-01T00:00:00.007Z").await;
    set_updated_at(&store, stale, "2999-01-01T00:00:00.004Z").await;

    let latest = store
        .latest_session(aoi)
        .await
        .expect("検索が成功すること")
        .expect("空でも最新セッションは返ること");
    assert_eq!(
        latest.id, tied_newer,
        "同着なら id が大きい方、古い方は id が最大でも選ばれないこと"
    );

    let resolved = store
        .latest_or_create_session(aoi)
        .await
        .expect("解決できること");
    assert_eq!(resolved.id, tied_newer, "aoi の経路も同じ答えを返すこと");
    assert_eq!(
        session_count(&store).await,
        3,
        "空しか無くても新しいセッションを作らないこと"
    );
}

/// **中身のあるものが複数あるときの並びは ADR-0004 のまま**（`updated_at DESC, id DESC`）。
///
/// 「中身があること」を先に見る変更を入れても、その後ろの並びが変わっていないことを固定する。
/// 4件の `updated_at` を `id` の順とわざと食い違わせ、**同着の組も混ぜてある**ので、
/// `ASC` にする変異・`id` を先に見る変異・同着の向きを変える変異が、すべてここで落ちる。
///
/// # **4件の `message` の件数をすべて違える**（レビュー指摘 / 2026-08-27）
///
/// ここを揃えると、第1キーが**真偽**（`EXISTS`）でも**件数**（`COUNT(*)`）でも
/// 4件が同順位になり、**第1キーの値域を一度も区別しない**。
/// 実際 `EXISTS` → `COUNT(*)` の変異が 1571 全緑を通り抜けていた。
///
/// **その変異は等価ではない。** 件数で並べると
/// **「新しいが短い会話」が「古いが長い会話」に負ける** ——
/// 利用者が `aoi@new` で始めた会話を数ターン続けたあと素の `aoi` へ戻ると、
/// **その会話ではなく古い長い会話の続きに着地する。**
/// 履歴は普通に載るので**画面上は正常に見え、ログにも出ない。**
/// この作業単位が潰した壊れ方そのものなので、**勝者を最少件数にしてある。**
#[tokio::test]
async fn among_sessions_with_messages_the_most_recently_updated_wins() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let oldest = store.create_session(aoi).await.expect("作成できること");
    let tied_older = store.create_session(aoi).await.expect("作成できること");
    let tied_newer = store.create_session(aoi).await.expect("作成できること");
    let highest_id = store.create_session(aoi).await.expect("作成できること");
    // **件数をすべて違える。** 勝者（`tied_newer`）が**最少**、最古（`oldest`）が最多。
    for (session, turns) in [
        (oldest, 4),
        (tied_older, 3),
        (tied_newer, 1),
        (highest_id, 2),
    ] {
        for _ in 0..turns {
            fill(&store, session).await;
        }
    }

    // id 順:            oldest < tied_older < tied_newer < highest_id
    // updated_at 順:    oldest < highest_id < (tied_older == tied_newer)
    set_updated_at(&store, oldest, "2999-01-01T00:00:00.003Z").await;
    set_updated_at(&store, tied_older, "2999-01-01T00:00:00.009Z").await;
    set_updated_at(&store, tied_newer, "2999-01-01T00:00:00.009Z").await;
    set_updated_at(&store, highest_id, "2999-01-01T00:00:00.005Z").await;

    // 前提: 4件の件数がすべて違い、勝者が最少であること。
    // **これが崩れると、第1キーが「件数」に化けても緑になる。**
    let mut counts = Vec::new();
    for session in [oldest, tied_older, tied_newer, highest_id] {
        counts.push(message_count_of(&store, session).await);
    }
    let mut unique = counts.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        counts.len(),
        "4件の message 件数がすべて相異なること: {counts:?}"
    );
    assert_eq!(
        message_count_of(&store, tied_newer).await,
        *counts.iter().min().expect("件数が4つあること"),
        "勝者が最少件数であること（＝件数で並べると別の答えになる）: {counts:?}"
    );

    let latest = store
        .latest_session(aoi)
        .await
        .expect("検索が成功すること")
        .expect("最新セッションがあること");
    assert_eq!(
        latest.id, tied_newer,
        "updated_at が最大の組から、id が大きい方が選ばれること"
    );
}

/// セッションが1行も無ければ `None`。`aoi` の経路はそこで初めて作る。
///
/// **作った直後のセッションは空**なので、B-2b の変更後もそれが返り続けること
/// （空を除外する実装だと、`aoi` を叩くたびに新しい行が増える）。
#[tokio::test]
async fn with_no_sessions_at_all_the_aoi_path_creates_exactly_one() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    assert!(
        store
            .latest_session(aoi)
            .await
            .expect("検索が成功すること")
            .is_none(),
        "1行も無ければ None であること"
    );

    let created = store
        .latest_or_create_session(aoi)
        .await
        .expect("作成できること");
    assert_eq!(session_count(&store).await, 1);
    assert_eq!(
        message_count_of(&store, created.id).await,
        0,
        "作った直後は空であること（この状態で次の解決を測る）"
    );

    let again = store
        .latest_or_create_session(aoi)
        .await
        .expect("解決できること");
    assert_eq!(again.id, created.id, "空のまま同じセッションを継続すること");
    assert_eq!(
        session_count(&store).await,
        1,
        "空だからといって作り直さないこと"
    );
}

/// **`aoi@<id>` は空セッションにも届く。** 順位を下げただけで、明示した指定は殺さない。
///
/// 空セッションを WHERE で弾く実装にすると、`find_session` まで巻き込んで
/// 「明示的に指したのに 404」になりうる。そこは ADR-0004 の復旧経路なので塞がない。
/// **届くだけでなく、書き込めば普通に最新へ戻れる**ところまで見る
/// （読み出し経路から外れたままなら、届いても意味が無い）。
#[tokio::test]
async fn an_explicit_id_still_reaches_an_empty_session() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    let talked = store.create_session(aoi).await.expect("作成できること");
    fill(&store, talked).await;
    let empty = store.create_session(aoi).await.expect("作成できること");

    let found = store
        .find_session(aoi, empty.get())
        .await
        .expect("検索が成功すること")
        .expect("空セッションも id で引けること");
    assert_eq!(found.id, empty);

    fill(&store, empty).await;
    assert_eq!(
        store
            .latest_session(aoi)
            .await
            .expect("検索が成功すること")
            .expect("最新セッションがあること")
            .id,
        empty,
        "中身が入れば最新として返ること"
    );
    assert_eq!(
        message_count_of(&store, talked).await,
        2,
        "もう一方のセッションに書かれていないこと"
    );
}

/// `updated_at` に書き込む日時の書式が、スキーマの DEFAULT と一致していること。
///
/// ずれると文字列比較による大小関係が壊れ、最新セッションの解決が静かに狂う。
/// `created_at` はスキーマの DEFAULT、`updated_at` は `store` が組み立てた値なので、
/// 同じ行の2列を比べればカップリングを固定できる。
#[tokio::test]
async fn updated_at_uses_the_same_timestamp_format_as_the_schema_default() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let id = store.create_session(aoi).await.expect("作成できること");
    store
        .append_message(id, &raw_only(Role::Assistant, "生出力"))
        .await
        .expect("メッセージを追加できること");

    let (created_at, updated_at): (String, String) =
        sqlx::query_as("SELECT created_at, updated_at FROM session WHERE id = ?")
            .bind(id.get())
            .fetch_one(store.pool())
            .await
            .expect("日時を取得できること");

    assert_eq!(
        updated_at.len(),
        created_at.len(),
        "書式の桁数が DEFAULT と一致すること: {updated_at} / {created_at}"
    );
    let shape = |value: &str| -> Vec<Option<char>> {
        // 区切り文字の位置だけを取り出す（数字部分は当然違う）。
        value
            .chars()
            .map(|c| if c.is_ascii_digit() { None } else { Some(c) })
            .collect()
    };
    assert_eq!(
        shape(&updated_at),
        shape(&created_at),
        "区切り文字の並びが DEFAULT と一致すること: {updated_at} / {created_at}"
    );
}

/// **`dialogue` / `narration` が両方 `None` でも保存・取得できる。**
///
/// パース失敗時のフォールバックの受け皿。`raw_content` は必須引数なので省略できない。
#[tokio::test]
async fn raw_content_survives_when_parsing_failed() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");

    let broken = "「こんにちは（閉じ括弧が無い";
    let id = store
        .append_message(session, &raw_only(Role::Assistant, broken))
        .await
        .expect("パースできなかったメッセージも保存できること");

    let messages = store
        .recent_messages(session, 10)
        .await
        .expect("メッセージを取得できること");
    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message.id, id);
    assert_eq!(message.session_id, session);
    assert_eq!(message.role, Role::Assistant);
    assert_eq!(message.dialogue, None);
    assert_eq!(message.narration, None);
    assert_eq!(message.raw_content, broken, "生出力が丸ごと残ること");
}

/// パースできた場合も `raw_content` は保持する（「成功したから生は要らない」としない）。
#[tokio::test]
async fn raw_content_is_kept_even_when_parsing_succeeded() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");

    store
        .append_message(
            session,
            &NewMessage {
                role: Role::Assistant,
                dialogue: Some("こんにちは".to_owned()),
                narration: Some("葵は微笑んだ".to_owned()),
                raw_content: "「こんにちは」\n葵は微笑んだ".to_owned(),
            },
        )
        .await
        .expect("メッセージを追加できること");

    let messages = store
        .recent_messages(session, 10)
        .await
        .expect("メッセージを取得できること");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].dialogue.as_deref(), Some("こんにちは"));
    assert_eq!(messages[0].narration.as_deref(), Some("葵は微笑んだ"));
    assert_eq!(
        messages[0].raw_content, "「こんにちは」\n葵は微笑んだ",
        "分解できても生出力を残すこと"
    );
}

/// **空の `raw_content` は許す。**
///
/// 「LLM が空を返した」こと自体が調査材料であり、拒否すると**その事象を記録できなくなる**
/// （`raw_content` の存在理由に反する）。`character.handle` に空を禁じる CHECK があるのと
/// 非対称だが、これは意図した非対称。
#[tokio::test]
async fn empty_raw_content_is_accepted() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");

    store
        .append_message(session, &raw_only(Role::Assistant, ""))
        .await
        .expect("空の生出力も記録できること");

    let messages = store
        .recent_messages(session, 10)
        .await
        .expect("メッセージを取得できること");
    assert_eq!(messages.len(), 1, "空でも1件として残ること");
    assert_eq!(
        messages[0].raw_content, "",
        "空文字がそのまま読み出せること"
    );
}

/// `role` は DB 上でスキーマの CHECK が許す綴りで格納される。
///
/// 往復だけを見ていると `User → 'assistant'` の取り違えが対称に打ち消し合って通ってしまうので、
/// 格納された文字列そのものを固定する。
#[tokio::test]
async fn role_is_stored_with_the_spelling_the_schema_expects() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");

    store
        .append_message(session, &raw_only(Role::User, "ユーザ発言"))
        .await
        .expect("メッセージを追加できること");
    store
        .append_message(session, &raw_only(Role::Assistant, "キャラ発言"))
        .await
        .expect("メッセージを追加できること");

    let stored: Vec<String> =
        sqlx::query_scalar("SELECT role FROM message WHERE session_id = ? ORDER BY id")
            .bind(session.get())
            .fetch_all(store.pool())
            .await
            .expect("role を取得できること");
    assert_eq!(stored, vec!["user".to_owned(), "assistant".to_owned()]);

    let roles: Vec<Role> = store
        .recent_messages(session, 10)
        .await
        .expect("メッセージを取得できること")
        .into_iter()
        .map(|message| message.role)
        .collect();
    assert_eq!(roles, vec![Role::User, Role::Assistant], "往復すること");
}

/// 直近 N 件を**時系列順**で返す（プロンプト組立が使う並び）。
#[tokio::test]
async fn recent_messages_returns_the_last_n_in_chronological_order() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");

    for index in 0..5 {
        store
            .append_message(session, &raw_only(Role::User, &format!("発言{index}")))
            .await
            .expect("メッセージを追加できること");
    }

    let raw = |messages: Vec<personacore::store::Message>| -> Vec<String> {
        messages
            .into_iter()
            .map(|message| message.raw_content)
            .collect()
    };

    assert_eq!(
        raw(store
            .recent_messages(session, 3)
            .await
            .expect("メッセージを取得できること")),
        vec!["発言2", "発言3", "発言4"],
        "直近3件が古い順に返ること"
    );
    assert_eq!(
        raw(store
            .recent_messages(session, 99)
            .await
            .expect("メッセージを取得できること"))
        .len(),
        5,
        "件数より大きい上限でも全件返ること"
    );
    assert!(
        store
            .recent_messages(session, 0)
            .await
            .expect("メッセージを取得できること")
            .is_empty(),
        "上限 0 なら空であること"
    );
}

// ---------------------------------------------------------------------- 履歴の窓（ADR-0063 / A-17）

/// 通し番号 `turn` の往復（本文に番号を書く。利用者と応答で別の綴り）。
async fn append_numbered_turn(store: &Store, session: SessionId, turn: u32) {
    store
        .append_turn(
            session,
            &raw_only(Role::User, &format!("依頼{turn}")),
            &raw_only(Role::Assistant, &format!("返事{turn}")),
        )
        .await
        .expect("往復を書けること");
}

/// 窓の最初の行（利用者の発言）の通し番号。
fn first_turn_of(history: &personacore::store::RecentHistory) -> u32 {
    let first = &history
        .messages
        .first()
        .expect("窓が空でないこと")
        .raw_content;
    first
        .strip_prefix("依頼")
        .and_then(|number| number.parse().ok())
        .unwrap_or_else(|| panic!("窓の最初の行が利用者の発言ではない: {first}"))
}

/// **往復を 1〜60 と伸ばしながら読むと、読み始めは 10 往復に1回だけ変わり、N ≥ 25 では 25〜34 往復が返る**
/// （ADR-0063 §決定2）。
///
/// **同じキャラクターの別のセッションにも毎回1往復書く**（数える範囲がセッションに閉じていなければ、
/// 番号がずれる）。変わり目の前後はリテラルで固定する。
#[tokio::test]
async fn the_recent_history_starts_on_a_boundary_that_moves_once_every_ten_turns() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let other = store.create_session(aoi).await.expect("作成できること");
    let session = store.create_session(aoi).await.expect("作成できること");

    let mut firsts = Vec::new();
    for turn in 1..=60_u32 {
        append_numbered_turn(&store, other, 1_000 + turn).await;
        append_numbered_turn(&store, session, turn).await;

        let history = store
            .recent_history(session)
            .await
            .expect("履歴を読めること");
        let first = first_turn_of(&history);
        assert_eq!(history.turns_before, first - 1, "往復 {turn}: 前の発言の数");
        let expected: Vec<String> = (first..=turn)
            .flat_map(|number| [format!("依頼{number}"), format!("返事{number}")])
            .collect();
        let raw: Vec<String> = history
            .messages
            .iter()
            .map(|message| message.raw_content.clone())
            .collect();
        assert_eq!(raw, expected, "往復 {turn}: 読み始めから最新まで、欠けずに");
        let window = turn - first + 1;
        if turn >= 25 {
            assert!(
                (25..=34).contains(&window),
                "往復 {turn}: 窓は 25〜34 往復（いま {window}）"
            );
        } else {
            assert_eq!(first, 1, "往復 {turn}: 25 未満は全部読む");
        }
        firsts.push(first);
    }

    // 変わり目の前後（添字は往復の数 − 1）。
    assert_eq!(firsts[33], 1, "34 往復までは 1 から");
    assert_eq!(firsts[34], 11, "35 往復で 11 へ");
    assert_eq!(firsts[44], 21, "45 往復で 21 へ");
    assert_eq!(firsts[54], 31, "55 往復で 31 へ");
    assert_eq!(firsts[59], 31, "60 往復はまだ 31 から");
    let changes = firsts.windows(2).filter(|pair| pair[0] != pair[1]).count();
    assert_eq!(
        changes, 3,
        "60 往復までに読み始めが変わるのは 3 回: {firsts:?}"
    );
}

/// **返事の無い利用者の発言も1つの番号を持つ**（DB の利用者の発言を数える / `Store::turn_number` と同じ）。
///
/// 1〜29 の往復の後に返事の無い依頼（30）、続けて 31〜35 の往復。往復の数は 35 なので、読み始めは 11。
/// 返事の無い依頼を数えない形だと往復の数が 34 になり、読み始めは 1 のまま。
#[tokio::test]
async fn a_user_message_without_a_reply_still_takes_a_turn_number() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");
    for turn in 1..=29 {
        append_numbered_turn(&store, session, turn).await;
    }
    store
        .append_message(session, &raw_only(Role::User, "依頼30"))
        .await
        .expect("発言を書けること");
    for turn in 31..=35 {
        append_numbered_turn(&store, session, turn).await;
    }

    let history = store
        .recent_history(session)
        .await
        .expect("履歴を読めること");
    assert_eq!(first_turn_of(&history), 11);
    assert_eq!(history.turns_before, 10);
    assert_eq!(
        history.messages.len(),
        2 * 25 - 1,
        "11〜35 の往復と、返事の無い依頼1件"
    );
}

/// **読む件数には天井がある**（[`personacore::domain::HISTORY_READ_CAP`]）。天井で頭が切れても、
/// `turns_before` は**返した最初の行から数え直す**ので、行との対応は崩れない。
///
/// 30 往復（読み始めは 1）の後に、往復の形でない応答だけの行を 80 件。読み始めからは 140 件あるが、
/// 返るのは新しい側の 100 件で、最初の行は通し番号 21 の依頼（前に 20 件）。
#[tokio::test]
async fn the_recent_history_is_capped_and_still_counts_from_its_first_row() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");
    for turn in 1..=30 {
        append_numbered_turn(&store, session, turn).await;
    }
    for index in 0..80 {
        store
            .append_message(
                session,
                &raw_only(Role::Assistant, &format!("応答だけ{index}")),
            )
            .await
            .expect("発言を書けること");
    }

    let history = store
        .recent_history(session)
        .await
        .expect("履歴を読めること");
    assert_eq!(
        personacore::domain::HISTORY_READ_CAP,
        100,
        "前提: 天井は 100 件"
    );
    assert_eq!(history.messages.len(), 100);
    assert_eq!(first_turn_of(&history), 21);
    assert_eq!(history.turns_before, 20);
    assert_eq!(
        history
            .messages
            .last()
            .map(|message| message.raw_content.as_str()),
        Some("応答だけ79"),
        "新しい側を返す"
    );
}

/// **天井で頭が応答の行に当たっても、`turns_before` はその応答の依頼まで数える**（レビューの提案 B）。
///
/// 30 往復の後に応答だけの行を 81 件。読み始め（通し番号 1）からは 141 件で、返るのは新しい側の 100 件。
/// 最初の行は通し番号 21 の応答（`返事21`）で、その手前の利用者の発言は 1〜21 の 21 件。
/// 窓の中の最初の依頼は `依頼22`（通し番号 22 = 21 + 1）。
#[tokio::test]
async fn a_cap_that_cuts_at_a_reply_still_counts_its_request() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");
    for turn in 1..=30 {
        append_numbered_turn(&store, session, turn).await;
    }
    for index in 0..81 {
        store
            .append_message(
                session,
                &raw_only(Role::Assistant, &format!("応答だけ{index}")),
            )
            .await
            .expect("発言を書けること");
    }

    let history = store
        .recent_history(session)
        .await
        .expect("履歴を読めること");
    assert_eq!(history.messages.len(), 100);
    let first = history.messages.first().expect("窓が空でないこと");
    assert_eq!(first.role, Role::Assistant, "先頭は応答の行");
    assert_eq!(first.raw_content, "返事21");
    assert_eq!(history.turns_before, 21, "依頼21 まで数える");
    assert_eq!(
        history
            .messages
            .iter()
            .find(|message| message.role == Role::User)
            .map(|message| message.raw_content.as_str()),
        Some("依頼22"),
        "窓の中の最初の依頼"
    );
}

/// **空のセッションは空の窓**（前の発言は 0）。
#[tokio::test]
async fn an_empty_session_has_an_empty_recent_history() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");

    let history = store
        .recent_history(session)
        .await
        .expect("履歴を読めること");
    assert!(history.messages.is_empty());
    assert_eq!(history.turns_before, 0);
}

/// メッセージはセッションごとに独立している。
#[tokio::test]
async fn messages_are_scoped_to_one_session() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;
    let aoi_session = store.create_session(aoi).await.expect("作成できること");
    let aoi_other = store.create_session(aoi).await.expect("作成できること");
    let rin_session = store.create_session(rin).await.expect("作成できること");

    store
        .append_message(aoi_session, &raw_only(Role::User, "葵への発言"))
        .await
        .expect("メッセージを追加できること");
    store
        .append_message(aoi_other, &raw_only(Role::User, "葵の別セッション"))
        .await
        .expect("メッセージを追加できること");
    store
        .append_message(rin_session, &raw_only(Role::User, "凛への発言"))
        .await
        .expect("メッセージを追加できること");

    let contents: Vec<String> = store
        .recent_messages(aoi_session, 10)
        .await
        .expect("メッセージを取得できること")
        .into_iter()
        .map(|message| message.raw_content)
        .collect();
    assert_eq!(
        contents,
        vec!["葵への発言"],
        "同一セッションのメッセージだけが返ること"
    );
}

/// 存在しないセッションにはメッセージを追加できない。
///
/// **メッセージだけが入って `updated_at` が付いてこない状態を作らせない**ため、
/// 追加と更新は同一トランザクションで、更新対象が無ければ全体が失敗する。
#[tokio::test]
async fn appending_to_unknown_session_fails_without_writing_anything() {
    let (_dir, store) = setup().await;
    let unknown = SessionId::from_raw_for_tests(9_999);

    let err = store
        .append_message(unknown, &raw_only(Role::User, "宛先のない発言"))
        .await
        .expect_err("存在しないセッションには追加できないこと");
    assert!(
        matches!(err, StoreError::UnknownSession),
        "セッション不明として型付けされること: {err:?}"
    );
    assert_eq!(
        message_count(&store).await,
        0,
        "メッセージが書き込まれていないこと"
    );
}

/// **往復は「利用者 → 応答」の順で入る。**
///
/// 並びが逆になると、次のターンのプロンプトで会話が入れ替わって見える。
/// フィクスチャは `role` も本文も左右で別の値なので、
/// 順序の入れ替え・値の取り違えのどちらでも落ちる。
///
/// **別キャラクターのセッションを1つ用意しておく。** 用意しないと「応答だけ別セッションへ
/// 書く」取り違えを止めているのが外部キー制約だけになり、
/// **`PRAGMA foreign_keys` を有効化し忘れた瞬間に黙って素通りする**
/// （`src/store/mod.rs` の `UnknownSession` が FK に頼っていないのと同じ理由）。
/// 実在する別セッションを宛先にすれば FK は通ってしまうので、
/// **そちらが空であることをテスト側で見る。**
#[tokio::test]
async fn append_turn_writes_the_pair_in_order() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;
    let session = store.create_session(aoi).await.expect("作成できること");
    let other = store.create_session(rin).await.expect("作成できること");
    let (user, reply) = turn();

    let appended = store
        .append_turn(session, &user, &reply)
        .await
        .expect("往復を追加できること");

    // **長さの検査より先に置く。** 宛先を取り違えた実装は「対象セッションが1件」でも
    // 落ちるが、それだと「どこへ行ったか」が分からない。先にこちらで落とす。
    assert!(
        store
            .recent_messages(other, 10)
            .await
            .expect("メッセージを取得できること")
            .is_empty(),
        "別のセッションには1行も書かれないこと（宛先の取り違え）"
    );

    let messages = store
        .recent_messages(session, 10)
        .await
        .expect("メッセージを取得できること");
    assert_eq!(messages.len(), 2, "2件だけ入ること");

    assert_eq!(
        messages[0].id, appended.user,
        "1件目が利用者の発言であること"
    );
    assert_eq!(messages[0].role, user.role);
    assert_eq!(messages[0].dialogue, user.dialogue);
    assert_eq!(messages[0].narration, user.narration);
    assert_eq!(messages[0].raw_content, user.raw_content);

    assert_eq!(messages[1].id, appended.reply, "2件目が応答であること");
    assert_eq!(messages[1].role, reply.role);
    assert_eq!(messages[1].dialogue, reply.dialogue);
    assert_eq!(messages[1].narration, reply.narration);
    assert_eq!(messages[1].raw_content, reply.raw_content);

    assert!(
        appended.user < appended.reply,
        "利用者の発言が先に採番されること: {} / {}",
        appended.user,
        appended.reply
    );
}

/// **往復の形（利用者 → 応答）でない組は拒否される。**
///
/// `role` を見ない実装は、`append_turn` が防ぐと宣言している状態そのもの
/// —— **「利用者の発言が2つ並ぶ履歴」を不可分に書き込める。**
/// `prompt` は `raw_content` を読むだけなので後段の誰も赤にせず、
/// **モデルの応答が静かに崩れるだけ**になる。呼び出し側が引数順を取り違えた
/// `(Assistant, User)` も同じくここで止まる。
///
/// `Role` は `User` / `Assistant` の2値なので、往復でない組み合わせはこの3通りで尽きる。
///
/// **弾いたときに1行も書かれず `updated_at` も動かないこと**まで見る。
///
/// **この2つで止められるのは、検証より前の書き込みがトランザクションの外へ出た場合だけ。**
/// 検証を `touch_session` の後（トランザクションの中）へ下げる変異は早期 return で
/// ロールバックされるため、**行数も `updated_at` も外からは同一に見えて観測できない**
/// （変異検査で実測）。その変異は**宛先も形も不正なときの返り値**で観測する
/// （`append_turn_checks_the_shape_before_touching_the_database`）。
#[tokio::test]
async fn append_turn_refuses_a_pair_that_is_not_user_then_assistant() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");
    let (user, reply) = turn();
    let updated_at_before = stored_updated_at(&store, session).await;

    let broken = [
        (&user, &user, "利用者の発言が2つ並ぶ"),
        (&reply, &reply, "応答が2つ並ぶ"),
        (&reply, &user, "利用者と応答が入れ替わっている"),
    ];

    for (first, second, what) in broken {
        let Err(err) = store.append_turn(session, first, second).await else {
            panic!("{what}: 往復の形でない組は拒否されること");
        };
        assert!(
            matches!(err, StoreError::InvalidTurn),
            "{what}: 往復の形が不正として型付けされること: {err:?}"
        );
        assert_eq!(
            message_count(&store).await,
            0,
            "{what}: 1行も書き込まれていないこと"
        );
        assert_eq!(
            stored_updated_at(&store, session).await,
            updated_at_before,
            "{what}: updated_at も動いていないこと"
        );
    }

    // **拒否が広すぎないこと。** 正しい形は通り、渡した `role` がそのまま格納される
    // （綴りを位置から決め打つ実装をここで落とす）。
    store
        .append_turn(session, &user, &reply)
        .await
        .expect("往復の形なら追加できること");
    let stored: Vec<String> =
        sqlx::query_scalar("SELECT role FROM message WHERE session_id = ? ORDER BY id")
            .bind(session.get())
            .fetch_all(store.pool())
            .await
            .expect("role を取得できること");
    assert_eq!(
        stored,
        vec!["user".to_owned(), "assistant".to_owned()],
        "渡した role がそのまま格納されること"
    );
}

/// **往復の形の検証は、DB に触るより前に行われる。**
///
/// 「1行も書かれない」「`updated_at` が動かない」ではこの位置を固定できない。
/// 検証を `touch_session` の後（トランザクションの中）へ下げても、早期 return で
/// tx がロールバックされるため、**行数も `updated_at` も外からは同一に見える**
/// （変異検査で実測。この2つで止められるのは、検証より前の書き込みが
/// **トランザクションの外へ出た**場合だけ）。
///
/// 位置が外から観測できるのは**宛先も形も不正な呼び出し**。検証が先なら `InvalidTurn`、
/// 後なら `touch_session` が先に 0 件を返して `UnknownSession` になる。
///
/// # 下げた場合に何が起きるか（次に検証位置を動かす人向け）
///
/// * 形が不正な呼び出しが**毎回 SQLite の書きロックを取ってから**弾かれる。
///   裏方の抽出書き込みと本文処理の書き込みが重なる設計なので、`busy_timeout` を無駄に食う。
/// * `server` から見えるエラー種別が `InvalidTurn`（呼び出し側のバグ）から
///   `UnknownSession`（404 相当）へ**静かに化ける。**
///
/// # このテストが固定していない範囲（実測）
///
/// 見張っているのは「**`touch_session` より前**」であって「`begin()` より前」ではない。
/// 検証を `self.pool.begin()` と `touch_session` の間へ移す変異は**緑のまま通る**。
/// deferred な `BEGIN` は最初の文を実行するまでロックを取らないので、
/// 上の2つの失敗モードはどちらも起きない（＝無害な位置）。
/// **書きロックを取る位置は `touch_session`** なので、見張る境界もそこに置いてある。
#[tokio::test]
async fn append_turn_checks_the_shape_before_touching_the_database() {
    let (_dir, store) = setup().await;
    let (user, reply) = turn();
    // 宛先も存在しない。形の検証が先なら、この id には一度も触れられない。
    let unknown = SessionId::from_raw_for_tests(9_999);

    let broken = [
        (&user, &user, "利用者の発言が2つ並ぶ"),
        (&reply, &reply, "応答が2つ並ぶ"),
        (&reply, &user, "利用者と応答が入れ替わっている"),
    ];

    for (first, second, what) in broken {
        let err = store
            .append_turn(unknown, first, second)
            .await
            .expect_err("宛先も形も不正なら失敗すること");
        assert!(
            matches!(err, StoreError::InvalidTurn),
            "{what}: 宛先を引くより先に形で弾かれること\
             （UnknownSession なら検証がトランザクションの中へ下がっている）: {err:?}"
        );
    }

    // 形が正しければ、宛先が無いことは通常どおり `UnknownSession` になる
    // （上の検査が「常に InvalidTurn を返す」実装でも緑にならないようにする）。
    let err = store
        .append_turn(unknown, &user, &reply)
        .await
        .expect_err("宛先が無ければ失敗すること");
    assert!(
        matches!(err, StoreError::UnknownSession),
        "形が正しければ宛先の不在として型付けされること: {err:?}"
    );
}

/// **往復の書き込みの間に、他の書き込みが割り込まない。**
///
/// 不可分性の見張り番は失敗注入（`append_turn_leaves_nothing_behind_when_the_reply_cannot_be_written`）
/// にもあるが、あちらは単スレッドで DB トリガに依存する。
/// **実運用で起きる失敗モードは失敗注入ではなく並行実行**なので、こちらを恒久で置く。
///
/// 判定は「同一往復の2件の rowid が連続していること」。単一の書きトランザクション内で
/// 採番された rowid は連続するので、`append_message` を2回呼ぶ実装
/// （＝往復が不可分でない実装）では他タスクの書き込みが間に挟まって崩れる。
/// 実測では非不可分な実装に対して**同一往復の間に23件が割り込んだ**。
///
/// **この判定は2つの前提に依存する。**
///   1. **WAL の単一ライタ** —— 書きトランザクションは直列化されるので、
///      他タスクの `message` への挿入がこのトランザクションの中に混ざらない。
///   2. **`message` への挿入を引き起こすトリガ・カスケードが存在しないこと。**
///      `message` に `AFTER INSERT` トリガを足して同じテーブルへ書くと、
///      往復の2件の間に別の rowid が採番され、**この見張り番が静かに壊れる**
///      （不可分性は保たれているのに赤くなる／緩めれば非不可分な実装を通す）。
///      `message` へ書くトリガを追加するなら、この判定を作り直すこと。
///
/// **`>=` や「順序だけ」に緩めないこと。** 緩めると非不可分な実装が緑を通る。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn append_turn_is_not_interleaved_with_concurrent_writers() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");
    let (user, reply) = turn();

    let tasks: Vec<_> = (0..32)
        .map(|_| {
            let store = store.clone();
            let (user, reply) = (user.clone(), reply.clone());
            tokio::spawn(async move { store.append_turn(session, &user, &reply).await })
        })
        .collect();

    for task in tasks {
        let pair = task
            .await
            .expect("タスクが panic しないこと")
            .expect("同時に呼ばれても失敗しないこと");
        assert_eq!(
            pair.reply.get(),
            pair.user.get() + 1,
            "往復の間に他の書き込みが割り込んでいないこと: {pair:?}"
        );
    }

    assert_eq!(
        message_count(&store).await,
        64,
        "32往復ぶんが欠けずに入ること"
    );
}

/// **応答側の書き込みが失敗したら、利用者側も残らない。**
///
/// この作業単位の存在理由そのもの。別々に書くと、応答の書き込みだけが失敗したときに
/// **「返答の無い利用者ターン」が履歴に残り**、次のターンのプロンプトに利用者発言が
/// 2つ並ぶ。`prompt` は `raw_content` を読むだけなので、そこでは誰も赤にしない。
///
/// 失敗は DB 側のトリガで注入する（`store` の実装を書き換えずに、
/// 「1件目は成功し2件目で落ちる」状況を決定的に作れる唯一の手段）。
/// `append_message` を2回呼ぶ実装に差し替えると、1件目が確定してこのテストが落ちる。
#[tokio::test]
async fn append_turn_leaves_nothing_behind_when_the_reply_cannot_be_written() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");
    let (user, reply) = turn();
    let updated_at_before = stored_updated_at(&store, session).await;

    // 応答側（`assistant`）の INSERT だけを失敗させる。RAISE(ABORT) は
    // その文だけを巻き戻してトランザクションは生かしたままにするので、
    // 「利用者側は書けたが応答側で落ちた」状態を正確に再現できる。
    sqlx::query(
        "CREATE TRIGGER reject_assistant_message BEFORE INSERT ON message \
         WHEN NEW.role = 'assistant' \
         BEGIN SELECT RAISE(ABORT, 'テスト用の失敗'); END",
    )
    .execute(store.pool())
    .await
    .expect("トリガを作成できること");

    let err = store
        .append_turn(session, &user, &reply)
        .await
        .expect_err("応答側が書けなければ失敗すること");
    assert!(
        matches!(err, StoreError::Query(_)),
        "DB エラーとして型付けされること: {err:?}"
    );

    assert_eq!(
        message_count(&store).await,
        0,
        "利用者側の発言だけが残らないこと（返答の無いターンを作らない）"
    );
    assert_eq!(
        stored_updated_at(&store, session).await,
        updated_at_before,
        "updated_at も巻き戻ること"
    );
}

/// 存在しないセッションへの往復は**1行も書かない**。
///
/// 「エラーが返る」だけでは足りないので、`message` の行数を前後で数えて比べる。
/// 既存の行がある状態で試すことで、「たまたま両方 0」を通さない。
#[tokio::test]
async fn append_turn_writes_nothing_for_an_unknown_session() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");
    let (user, reply) = turn();

    store
        .append_turn(session, &user, &reply)
        .await
        .expect("往復を追加できること");
    let count_before = message_count(&store).await;
    assert_eq!(count_before, 2, "先に往復が1組入っていること");
    let updated_at_before = stored_updated_at(&store, session).await;

    let unknown = SessionId::from_raw_for_tests(9_999);
    let err = store
        .append_turn(unknown, &user, &reply)
        .await
        .expect_err("存在しないセッションには追加できないこと");
    assert!(
        matches!(err, StoreError::UnknownSession),
        "セッション不明として型付けされること: {err:?}"
    );

    assert_eq!(
        message_count(&store).await,
        count_before,
        "message の行数が変化しないこと"
    );
    assert_eq!(
        stored_updated_at(&store, session).await,
        updated_at_before,
        "他のセッションの updated_at も動かないこと"
    );
}

/// **`updated_at` の押し上げは往復1組につき1回。**
///
/// 押し上げは「そのキャラクターの最大値 + 1ms」なので、`append_message` を2回呼ぶ実装は
/// 2ms 進む。両方を同じテストで測ることで、**1回と2回を区別できる形**にしてある
/// （同じ値を期待していると、トランザクションを分けた実装が緑のまま通る）。
///
/// 押し上げはキャラクター単位なので、2つの計測は別キャラクターに分ける。
///
/// **`aoi` 側にはセッションを2つ置く。** 1つしか無いと `latest_session(aoi)` の候補が
/// 1つきりで、末尾の「往復を追加した側が最新セッションになること」が**何を書いても緑**になる。
/// 追加しない方（`aoi_other`）を追加前の時点で最新にしておき、
/// **押し上げが起きて初めて逆転する**形にする。
#[tokio::test]
async fn append_turn_advances_updated_at_exactly_once() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let rin = character(&store, "rin").await;
    let aoi_session = store.create_session(aoi).await.expect("作成できること");
    let aoi_other = store.create_session(aoi).await.expect("作成できること");
    let rin_session = store.create_session(rin).await.expect("作成できること");
    let (user, reply) = turn();

    // 時計に依存せず ms 単位で測れるよう、システム時刻より未来の値に揃える。
    // **押し上げ後の期待値は `aoi` 側でも `rin` 側でも同じ基準**（`.000Z` + N ms）になる。
    // `aoi_session` だけを1年前に置くのは、押し上げが無ければ `aoi_other` が最新のままで、
    // 逆転したことが最後のアサートで観測できるようにするため。
    for (session, at) in [
        (aoi_session, "2998-01-01T00:00:00.000Z"),
        (aoi_other, "2999-01-01T00:00:00.000Z"),
        (rin_session, "2999-01-01T00:00:00.000Z"),
    ] {
        sqlx::query("UPDATE session SET updated_at = ? WHERE id = ?")
            .bind(at)
            .bind(session.get())
            .execute(store.pool())
            .await
            .expect("直接の UPDATE が成功すること");
    }

    assert_eq!(
        store
            .latest_session(aoi)
            .await
            .expect("検索が成功すること")
            .expect("最新セッションがあること")
            .id,
        aoi_other,
        "追加前は往復を書かない方が最新であること（この前提が崩れると末尾の検査が無意味になる）"
    );

    store
        .append_turn(aoi_session, &user, &reply)
        .await
        .expect("往復を追加できること");

    store
        .append_message(rin_session, &user)
        .await
        .expect("メッセージを追加できること");
    store
        .append_message(rin_session, &reply)
        .await
        .expect("メッセージを追加できること");

    assert_eq!(
        stored_updated_at(&store, aoi_session).await,
        "2999-01-01T00:00:00.001Z",
        "往復1組では 1ms だけ進むこと"
    );
    assert_eq!(
        stored_updated_at(&store, rin_session).await,
        "2999-01-01T00:00:00.002Z",
        "append_message を2回呼んだ場合は 2ms 進む（＝上と区別が付く）"
    );

    assert_eq!(
        store
            .latest_session(aoi)
            .await
            .expect("検索が成功すること")
            .expect("最新セッションがあること")
            .id,
        aoi_session,
        "往復を追加した側が最新セッションになること"
    );
}

/// **パースできなかった応答も往復として書ける。**
///
/// パーサはフォールバックするので `dialogue` / `narration` が両方 `None` の応答は
/// 正常な状態。`raw_content` は空文字も正当な値（`domain::NewMessage`）。
/// 利用者側は分解済みの値を持たせて、応答側の `None` / 空文字が
/// 利用者側の値で埋められていないことも同時に見る。
#[tokio::test]
async fn append_turn_accepts_a_reply_that_could_not_be_parsed() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");
    let (user, _) = turn();
    let reply = raw_only(Role::Assistant, "");

    store
        .append_turn(session, &user, &reply)
        .await
        .expect("空の応答も記録できること");

    let messages = store
        .recent_messages(session, 10)
        .await
        .expect("メッセージを取得できること");
    assert_eq!(messages.len(), 2, "空でも1件として残ること");
    assert_eq!(messages[0].raw_content, user.raw_content);
    assert_eq!(messages[0].dialogue, user.dialogue);
    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[1].dialogue, None);
    assert_eq!(messages[1].narration, None);
    assert_eq!(
        messages[1].raw_content, "",
        "空文字がそのまま読み出せること"
    );
}

/// ラベルは読み出せる（フェーズ1では `store` から設定しないため直接 INSERT で確認する）。
#[tokio::test]
async fn label_is_read_back() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;

    sqlx::query("INSERT INTO session (character_id, label) VALUES (?, ?)")
        .bind(aoi.get())
        .bind("初対面")
        .execute(store.pool())
        .await
        .expect("直接の INSERT が成功すること");

    let latest = store
        .latest_session(aoi)
        .await
        .expect("検索が成功すること")
        .expect("最新セッションがあること");
    assert_eq!(latest.label.as_deref(), Some("初対面"));
}

/// 格納されている `role` が想定外の値なら握り潰さずエラーにする。
///
/// スキーマの CHECK があるため `store` 経由でも直接 INSERT でも起こらないが、
/// パーサ同様に**panic しない**ことを固定しておく（`unwrap` で書くと落ちる）。
#[tokio::test]
async fn unknown_stored_role_is_reported_as_invalid_stored_value() {
    let (_dir, store) = setup().await;
    let aoi = character(&store, "aoi").await;
    let session = store.create_session(aoi).await.expect("作成できること");

    store
        .append_message(session, &raw_only(Role::User, "発言"))
        .await
        .expect("メッセージを追加できること");

    // CHECK 制約があるため通常は書けない。`PRAGMA ignore_check_constraints` は
    // **接続ごと**の設定なので、同一接続を掴んだまま書き換える。
    {
        let mut conn = store.pool().acquire().await.expect("接続を取得できること");
        sqlx::query("PRAGMA ignore_check_constraints = ON")
            .execute(&mut *conn)
            .await
            .expect("PRAGMA が成功すること");
        sqlx::query("UPDATE message SET role = ? WHERE session_id = ?")
            .bind("system")
            .bind(session.get())
            .execute(&mut *conn)
            .await
            .expect("直接の UPDATE が成功すること");
        sqlx::query("PRAGMA ignore_check_constraints = OFF")
            .execute(&mut *conn)
            .await
            .expect("PRAGMA が成功すること");
    }

    let err = store
        .recent_messages(session, 10)
        .await
        .expect_err("想定外の role は握り潰さないこと");
    assert!(
        matches!(err, StoreError::InvalidStoredValue),
        "格納値の不正として型付けされること: {err:?}"
    );
    assert!(
        !err.to_string().contains("system"),
        "Display に格納値を含めないこと: {err}"
    );
}
