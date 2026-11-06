//! **変容の累積の表**（[ADR-0027](../docs/adr/0027-drift-from-what-was-not-chosen.md)
//! §決定7 / 作業単位 16e-b）の検証。
//!
//! 見ているのは `Store::drift_charges` と `Store::save_drift_charges` の**実挙動**で、
//! 「何を溜めるか」（`domain::drift_ledger`）とは別。
//!
//! | 見るもの | 何が壊れたら赤になるか |
//! |---|---|
//! | 累積が**この持ち主・この軸**に置かれる | `character_id` / `axis` の取り違え |
//! | 同じ軸を2回書くと**上書き**される | `ON CONFLICT` を落とす／行が増える |
//! | **行が無い軸は返らない**（初回のターン） | 空の行を作る |
//! | **1行壊れると読みが止まる** | 読み出しの検証を握り潰す |
//! | 有限でない値は**書かない** | `is_finite` の門を落とす |
//! | 宛先の無い累積は入らない | 外部キーを落とす |
//!
//! # なぜ票の表と分けたのか
//!
//! **票は追記のみ**（ADR-0026 §決定3 / 走査が UPDATE を禁じている）で、
//! こちらは**1軸1行の現在値**として毎ターン上書きされる。
//! 「何がいつ動いたか」の履歴は票とログの側が持つ。

mod common;

use chrono::{DateTime, Utc};
use personacore::domain::{
    CharacterId, DisplayName, DriftApply, Handle, Persona, StatAlloc, StatAxis, StatDriftMode,
};
use personacore::store::{Store, StoreError};
use tempfile::TempDir;

use common::{fixture_scale, insert_drift_charge_at, insert_drift_charge_ignoring_checks};

/// 累積が置かれる軸。**他のどの軸とも十分違う綴り。**
const CHARGED_AXIS: &str = "パッション";

/// もう1本の軸。[`CHARGED_AXIS`] とは別の綴り。
const OTHER_AXIS: &str = "ロジック";

/// 累積を置かない軸。**行が無い状態を作る。**
const UNTOUCHED_AXIS: &str = "共鳴";

/// 会話するキャラクターの識別子。
const HANDLE: &str = "aoi";

/// **会話しない**キャラクターの識別子。会話する方とは別の綴り。
const BYSTANDER_HANDLE: &str = "kaede";

/// 開いた DB と、そこに置いた2体のキャラクター。
struct Fixture {
    store: Store,
    /// **累積が置かれる方。** 後から作るので `id` は 1 ではない。
    character: CharacterId,
    /// **置かれない方。** 同じ綴りの軸を持っている。
    bystander: CharacterId,
    _dir: TempDir,
}

/// **値を動かしてよい許可証。**
///
/// `StatDriftMode::On` からしか取れない（ADR-0026 §決定1）。
fn permit() -> DriftApply {
    StatDriftMode::On.applying().expect("on は許可証を出すこと")
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

fn axis(raw: &str) -> StatAxis {
    StatAxis::parse(raw).expect("正当な軸名であること")
}

/// 軸と値の組から `StatAlloc` を作る。
fn allocs(pairs: &[(&str, u32)]) -> Vec<StatAlloc> {
    pairs
        .iter()
        .map(|(name, value)| StatAlloc {
            axis: axis(name),
            value: fixture_scale()
                .value(*value)
                .expect("尺度に収まる値であること"),
        })
        .collect()
}

/// **2体のキャラクター。**
///
/// 1体しか作らないと `character.id` と `1` が常に同値になり、
/// **持ち主を決め打ちにする実装が原理的に観測できない**（CLAUDE.md）。
async fn fresh() -> Fixture {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let store = Store::connect(&dir.path().join("charge.sqlite"))
        .await
        .expect("DB を開けること");

    // **累積が置かれない方を先に作る**（＝ `id` が 1 になるのはこちら）。
    let bystander = create(&store, BYSTANDER_HANDLE, "楓").await;
    let character = create(&store, HANDLE, "葵").await;
    assert_ne!(character, bystander, "前提: 2体の id が別であること");
    assert_ne!(
        character,
        CharacterId::from_raw_for_tests(1),
        "前提: 累積が置かれる方の id が 1 でないこと（決め打ちと同値にしない）"
    );

    for id in [character, bystander] {
        store
            .replace_stat_allocs(
                id,
                &allocs(&[(CHARGED_AXIS, 4), (OTHER_AXIS, 2), (UNTOUCHED_AXIS, 3)]),
            )
            .await
            .expect("性格ステータスを置けること");
    }

    Fixture {
        store,
        character,
        bystander,
        _dir: dir,
    }
}

/// 累積を `(軸名, 値)` の組で読む。**並びは軸名でそろえる**（SQL の並びに寄りかからない）。
async fn charges(store: &Store, id: CharacterId) -> Vec<(String, f64)> {
    let mut rows: Vec<(String, f64)> = charge_rows(store, id)
        .await
        .into_iter()
        .map(|(name, charge, _)| (name, charge))
        .collect();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    rows
}

/// 累積を `(軸名, 値, 更新時刻)` の組で読む（作業単位 D-5a）。**並びは軸名でそろえる。**
///
/// **時刻を落とさない読みが要る理由**は `Store::drift_charges` の doc にある——
/// 累積は履歴を持たないので、**表から取れる時間の手掛かりはこの1列だけ**である。
async fn charge_rows(store: &Store, id: CharacterId) -> Vec<(String, f64, DateTime<Utc>)> {
    let mut rows: Vec<(String, f64, DateTime<Utc>)> = store
        .drift_charges(id)
        .await
        .expect("累積を読めること")
        .into_iter()
        .map(|charge| {
            (
                charge.axis().as_str().to_owned(),
                charge.charge(),
                charge.updated_at(),
            )
        })
        .collect();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    rows
}

/// テストが植える時刻を `DateTime<Utc>` にする。**綴りは呼び出し側の直書き。**
fn at(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .expect("テストで使う時刻は RFC 3339 であること")
        .with_timezone(&Utc)
}

/// **書いた累積が、この持ち主のこの軸から読み戻せる。**
///
/// # 2本の軸に**別の値**を置く
///
/// 同じ値だと、**軸を取り違える実装**が緑を通る
/// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
/// **符号も別**にしてある——薄れた軸は負、票が入った軸は正になるのが本番の形である。
#[tokio::test]
async fn a_charge_round_trips_for_this_character_and_this_axis() {
    let fixture = fresh().await;

    fixture
        .store
        .save_drift_charges(
            fixture.character,
            &[(axis(CHARGED_AXIS), 0.75), (axis(OTHER_AXIS), -0.25)],
            permit(),
        )
        .await
        .expect("累積を書けること");

    assert_eq!(
        charges(&fixture.store, fixture.character).await,
        vec![
            (CHARGED_AXIS.to_owned(), 0.75),
            (OTHER_AXIS.to_owned(), -0.25),
        ]
    );

    // **別のキャラクターには何も入らない。**
    assert!(
        charges(&fixture.store, fixture.bystander).await.is_empty(),
        "別のキャラクターに累積が入った"
    );
    fixture.store.close().await;
}

/// **同じ軸を2回書くと上書きされる。行は増えない**（1軸1行）。
///
/// 行が増えると、読み出しが**同じ軸を2回返す**ので、
/// 帳簿がどちらを採るかが挿入順まかせになる。
#[tokio::test]
async fn writing_the_same_axis_twice_replaces_the_row() {
    let fixture = fresh().await;

    for value in [0.5_f64, -1.5] {
        fixture
            .store
            .save_drift_charges(fixture.character, &[(axis(CHARGED_AXIS), value)], permit())
            .await
            .expect("累積を書けること");
    }

    assert_eq!(
        charges(&fixture.store, fixture.character).await,
        vec![(CHARGED_AXIS.to_owned(), -1.5)],
        "上書きされていないか、行が増えている"
    );
    fixture.store.close().await;
}

/// **書いていない軸の行は返らない**（初回のターン）。
///
/// 呼び出し側（`server::drift`）が**保存済みの軸に合わせて `0` で埋める**ので、
/// ここで空の行を作る必要は無い。作ると、
/// **一度も走っていないキャラクターと、累積が 0 に戻ったキャラクターが
/// 表の上で区別できなくなる。**
#[tokio::test]
async fn an_axis_that_was_never_charged_has_no_row() {
    let fixture = fresh().await;

    fixture
        .store
        .save_drift_charges(fixture.character, &[(axis(CHARGED_AXIS), 0.25)], permit())
        .await
        .expect("累積を書けること");

    let rows = charges(&fixture.store, fixture.character).await;
    assert_eq!(rows.len(), 1, "書いていない軸の行まで返った: {rows:?}");
    assert!(
        !rows.iter().any(|(name, _)| name == UNTOUCHED_AXIS),
        "書いていない軸が返った: {rows:?}"
    );
    fixture.store.close().await;
}

/// **空のスライスを渡しても何も起きない**（文を1つも投げない）。
#[tokio::test]
async fn writing_nothing_leaves_the_table_alone() {
    let fixture = fresh().await;

    fixture
        .store
        .save_drift_charges(fixture.character, &[], permit())
        .await
        .expect("空でも成功すること");

    assert!(charges(&fixture.store, fixture.character).await.is_empty());
    fixture.store.close().await;
}

/// **有限でない累積は書かない**（`inf` / `nan`）。
///
/// # 書けてしまうと、そのキャラクターの変容が永久に止まる
///
/// 次の読み出しが `InvalidStoredValue` になり、**累積には削除の導線が無い。**
/// 直す手段は `sqlite3` だけになる。
///
/// **同じ呼び出しに正しい軸が混ざっていても、まとめて拒否する**
/// ——トランザクションなので、部分的に書けた状態を残さない。
#[tokio::test]
async fn a_charge_that_is_not_finite_is_refused() {
    let fixture = fresh().await;

    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let err = fixture
            .store
            .save_drift_charges(
                fixture.character,
                &[(axis(CHARGED_AXIS), 0.5), (axis(OTHER_AXIS), bad)],
                permit(),
            )
            .await
            .err()
            .unwrap_or_else(|| panic!("{bad} が書けてしまった"));
        assert!(
            matches!(err, StoreError::InvalidStoredValue),
            "{bad} が別の理由で落ちた: {err}"
        );
        assert!(
            charges(&fixture.store, fixture.character).await.is_empty(),
            "拒否した呼び出しの一部が書き込まれた: {bad}"
        );
    }
    fixture.store.close().await;
}

/// **1行でも読めなければ、この読みは丸ごと止まる。**
///
/// `Store::recent_drift_votes` と同じ側に倒してある。黙って飛ばすと
/// **その軸の累積だけが 0 に戻る**——「その軸だけ変容が遅い」としか観測できない。
///
/// **壊れうるのは2つ**（軸名と累積）ので、両方を回す。
#[tokio::test]
async fn one_unreadable_row_stops_the_whole_read() {
    for (axis_name, charge) in [
        // 軸名が空（`StatAxis::parse` を通らない）。
        ("   ", 0.5_f64),
        // 累積が有限でない。
        (CHARGED_AXIS, f64::INFINITY),
    ] {
        let fixture = fresh().await;
        // **正しい行も置く**（壊れた行だけを飛ばす実装を落とす）。
        fixture
            .store
            .save_drift_charges(fixture.character, &[(axis(OTHER_AXIS), 0.25)], permit())
            .await
            .expect("累積を書けること");
        insert_drift_charge_ignoring_checks(
            &fixture.store,
            fixture.character.get(),
            axis_name,
            charge,
        )
        .await;

        let err = fixture
            .store
            .drift_charges(fixture.character)
            .await
            .err()
            .unwrap_or_else(|| panic!("壊れた行を読み飛ばした: {axis_name:?} {charge}"));
        assert!(
            matches!(err, StoreError::InvalidStoredValue),
            "別の理由で落ちた: {err}"
        );

        // **書き込みの側は止まらない**（追記も上書きもできる）。
        fixture
            .store
            .save_drift_charges(fixture.character, &[(axis(OTHER_AXIS), 0.5)], permit())
            .await
            .expect("読めなくても書けること");

        // **別のキャラクターの読みは止まらない。**
        assert!(
            charges(&fixture.store, fixture.bystander).await.is_empty(),
            "壊れた行が別のキャラクターの読みを止めた"
        );
        fixture.store.close().await;
    }
}

/// **宛先の無い累積は入らない**（外部キー）。
#[tokio::test]
async fn a_charge_for_a_character_that_does_not_exist_is_refused() {
    let fixture = fresh().await;

    let ghost = CharacterId::from_raw_for_tests(9_999);
    let err = fixture
        .store
        .save_drift_charges(ghost, &[(axis(CHARGED_AXIS), 0.5)], permit())
        .await
        .expect_err("宛先の無い累積は入らないこと");
    assert!(
        matches!(err, StoreError::Query(_)),
        "別の理由で落ちた: {err}"
    );
    fixture.store.close().await;
}

/// **キャラクターを消すと累積も消える**（`ON DELETE CASCADE`）。
///
/// 残ると、**同じ id が再利用されたときに他人の累積を引き継ぐ。**
#[tokio::test]
async fn deleting_the_character_takes_its_charges_with_it() {
    let fixture = fresh().await;

    fixture
        .store
        .save_drift_charges(fixture.character, &[(axis(CHARGED_AXIS), 0.5)], permit())
        .await
        .expect("累積を書けること");
    assert_eq!(charges(&fixture.store, fixture.character).await.len(), 1);

    sqlx::query("DELETE FROM character WHERE id = ?")
        .bind(fixture.character.get())
        .execute(fixture.store.pool())
        .await
        .expect("キャラクターを消せること");

    assert!(
        charges(&fixture.store, fixture.character).await.is_empty(),
        "累積が残った"
    );
    fixture.store.close().await;
}

/// **上書きは `updated_at` を進める**（レビュー提案1 / 16e-b）。
///
/// # この列が唯一の時間の手掛かりである
///
/// 累積は**履歴を持たない**（何がいつ動いたかは票とログの側）。
/// 進めないと、**そのキャラクターの変容がいつまで走っていたのかが表から読めない。**
/// `UPSERT_CHARGE` から `updated_at = ...` を落とす変異は、ここで赤くなる。
///
/// # 時刻を「今」どうしで比べない
///
/// `strftime` はミリ秒精度なので、**2回続けて書くと同じ値になりうる**
/// （遅い機械でだけ落ちる見張り番になる）。
/// **古い時刻を植えてから上書きする**ので、判定は決定的である。
#[tokio::test]
async fn writing_again_moves_the_timestamp_forward() {
    let fixture = fresh().await;

    /// 植える「昔」。**現在時刻とは絶対に一致しない。**
    const LONG_AGO: &str = "2000-01-01T00:00:00.000Z";

    sqlx::query(
        "INSERT INTO stat_drift_charge (character_id, axis, charge, updated_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(fixture.character.get())
    .bind(CHARGED_AXIS)
    .bind(0.125_f64)
    .bind(LONG_AGO)
    .execute(fixture.store.pool())
    .await
    .expect("古い累積を植えられること");

    fixture
        .store
        .save_drift_charges(fixture.character, &[(axis(CHARGED_AXIS), 0.5)], permit())
        .await
        .expect("累積を書けること");

    let stamp: String =
        sqlx::query_scalar("SELECT updated_at FROM stat_drift_charge WHERE character_id = ?")
            .bind(fixture.character.get())
            .fetch_one(fixture.store.pool())
            .await
            .expect("時刻を読めること");
    assert_ne!(stamp, LONG_AGO, "上書きしたのに時刻が古いまま");
    assert!(
        stamp.as_str() > LONG_AGO,
        "時刻が進んでいない: {stamp} <= {LONG_AGO}"
    );

    // **値の方も上書きされている**（時刻だけ進めて中身を落とす実装を落とす）。
    assert_eq!(
        charges(&fixture.store, fixture.character).await,
        vec![(CHARGED_AXIS.to_owned(), 0.5)]
    );
    fixture.store.close().await;
}

/// **読み出しが、軸ごとの `updated_at` を持って返る**（作業単位 D-5a）。
///
/// # ここが是正の半分である
///
/// 列は 16e-b から実在し、[`writing_again_moves_the_timestamp_forward`] が
/// **書き込みの側だけ**を見張っていた。**読む口がどこにも無かった**ので、
/// 「いつまで変容が走っていたのか」は `sqlite3` でしか取れなかった。
///
/// # 3軸を**全部違う時刻**で植えて、**真ん中**を押さえる
///
/// 全部同じ時刻だと、**どの軸の時刻かを取り違える実装が観測できない**
/// （CLAUDE.md の最頻の穴）。
/// **2件だと「常に先頭」「常に末尾」の片方が通り抜ける**ので3件にしてある
/// （`D-2` の実測）。
///
/// **値（累積）と時刻の取り違え**も落ちる——軸名の昇順と時刻の昇順を
/// **わざと逆**にしてあるので、並び順に寄りかかった引き当ては合わない。
#[tokio::test]
async fn each_axis_carries_the_time_its_own_charge_was_written() {
    let fixture = fresh().await;

    // **軸名の昇順（`パ` U+30D1 < `ロ` U+30ED < `共` U+5171）と、
    // 時刻の昇順（2024 → 2025 → 2026）がわざと食い違う並び。**
    const PASSION_AT: &str = "2026-08-26T01:02:03.400Z";
    const OTHER_AT: &str = "2024-05-04T03:02:01.100Z";
    const UNTOUCHED_AT: &str = "2025-12-31T23:59:59.999Z";

    for (name, charge, stamp) in [
        (CHARGED_AXIS, 0.75_f64, PASSION_AT),
        (OTHER_AXIS, -0.25, OTHER_AT),
        (UNTOUCHED_AXIS, 2.5, UNTOUCHED_AT),
    ] {
        insert_drift_charge_at(&fixture.store, fixture.character.get(), name, charge, stamp).await;
    }
    // **傍観者には同じ軸名で別の時刻を置く**（持ち主の絞り込みが消えたら赤になる）。
    insert_drift_charge_at(
        &fixture.store,
        fixture.bystander.get(),
        CHARGED_AXIS,
        9.5,
        "2019-01-01T00:00:00.000Z",
    )
    .await;

    assert_eq!(
        charge_rows(&fixture.store, fixture.character).await,
        vec![
            // 軸名の昇順（`パッション` / `ロジック` / `共鳴`）。
            (CHARGED_AXIS.to_owned(), 0.75, at(PASSION_AT)),
            (OTHER_AXIS.to_owned(), -0.25, at(OTHER_AT)),
            (UNTOUCHED_AXIS.to_owned(), 2.5, at(UNTOUCHED_AT)),
        ],
        "軸ごとに、その軸の累積とその軸の時刻が返ること"
    );

    fixture.store.close().await;
}

/// **書き込みの経路が押した綴りを、この読みがデコードできる**（作業単位 D-5a）。
///
/// # ⚠ 何を押さえているのかを、正確に言う（レビューの実測で訂正 / 2026-08-28）
///
/// **「本番が押した時刻が正しい」ではない。**
/// ここは**新しい行を作る**ので、`updated_at` は
/// `UPSERT_CHARGE` の `ON CONFLICT` 枝ではなく、
/// **`migrations/0012` の列の `DEFAULT` から来ている。**
/// **上書きの枝が押す綴りは、[`writing_again_moves_the_timestamp_forward`] の側**である。
///
/// **押さえているのは書式の往復** ——
/// **マイグレーションが書く綴りを、この読み（`charge_from_row` の
/// `try_get::<DateTime<Utc>>`）がデコードできること。**
/// 値そのものはテストから壊せない（`strftime` が決める）ので、
/// **値の正しさはここでは主張しない。**
///
/// # 綴りを写さない
///
/// 期待値は**この処理を挟んだ現在時刻の窓**で作る。
/// `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')` の書式をテストへ写すと、
/// **書式が変わったとき両方が一緒に動いて緑のまま**になる。
///
/// **[`each_axis_carries_the_time_its_own_charge_was_written`] だけでは足りない。**
/// あちらは**テストが植えた綴り**しか読んでいないので、
/// **スキーマが書く綴りを読めない実装（書式の食い違い）が通り抜ける。**
///
/// # ⚠ 窓を「厳密にこの2点の間」にしないこと（**このワークツリーで実測 / 2026-08-28**）
///
/// 最初は `[before - 1ms, after]` で書いて、**8本並列 × 6ラウンドで 48 回中 10 回落ちた。**
/// 落ち方は `2026-08-28 08:53:15.679 UTC が
/// [2026-08-28 08:53:15.680502500 UTC, …] の外` ——**SQLite の時刻の方が古い。**
///
/// 理由は2つ重なっている:
///
/// * **`strftime` はミリ秒で切り捨てる**（`Utc::now()` はナノ秒まで持つ）。
/// * **SQLite が読む OS の時計は粗い。** Windows の既定のティックは 15.6ms あり、
///   高分解能の `Utc::now()` より**最大でその分だけ後ろに見える。**
///
/// **どちらも「速い機械／遅い機械」の話ではないので、待っても消えない。**
/// **窓を広く取る**——狙いは「**いまの時刻が書かれている**」ことであって、
/// ミリ秒の一致ではない。[`STAMP_SLACK`] の幅なら、
/// **固定の定数・エポック・書式違いはどれも外れる。**
#[tokio::test]
async fn the_time_the_production_writer_stamps_is_readable() {
    let fixture = fresh().await;

    let before = Utc::now();
    fixture
        .store
        .save_drift_charges(fixture.character, &[(axis(CHARGED_AXIS), 0.5)], permit())
        .await
        .expect("累積を書けること");
    let after = Utc::now();

    let rows = charge_rows(&fixture.store, fixture.character).await;
    assert_eq!(rows.len(), 1, "1軸だけ書いたこと: {rows:?}");
    let (name, charge, stamp) = &rows[0];
    assert_eq!(name, CHARGED_AXIS);
    assert!((charge - 0.5).abs() < f64::EPSILON, "累積: {charge}");
    assert!(
        *stamp >= before - STAMP_SLACK && *stamp <= after + STAMP_SLACK,
        "書いた瞬間の時刻でないこと: {stamp} が [{before}, {after}] の外（許容 {STAMP_SLACK}）"
    );

    fixture.store.close().await;
}

/// **本番が押した時刻の許容幅**（作業単位 D-5a / 上の ⚠ の実測から）。
///
/// **時計の粗さと切り捨てを吸収するだけの幅**で、
/// **「いまではない時刻」は1つも通さない**
/// ——固定の定数（2023〜2026 の綴り）もエポックも、この幅の外にある。
const STAMP_SLACK: chrono::TimeDelta = chrono::TimeDelta::seconds(60);

/// **時刻として読めない行が1つあれば、この読みは丸ごと止まる**（作業単位 D-5a）。
///
/// # ⚠ **止まる理由が1つ増えた**
///
/// 是正前、`updated_at` は**どの `SELECT` にも載っていなかった**ので、
/// この列が壊れていても累積の読みは通っていた。
/// 読むようにした以上、**壊れた時刻はこの読みを止める**——
/// つまり**そのキャラクターの値の適用も止まる**（`server::drift::apply_scores` は
/// 累積が読めない回に値を動かさない）。
///
/// **黙って飛ばす形は採らない。** 飛ばすと**その軸の累積だけが 0 に戻り**、
/// 「その軸だけ変容が遅い」としか観測できない（`Store::drift_charges` の doc）。
///
/// **`updated_at` に CHECK は無い**（`migrations/0012`）ので、
/// この状態は `sqlite3` から作れる。だから見張り番が要る。
#[tokio::test]
async fn a_stored_time_that_is_not_a_time_stops_the_whole_read() {
    for broken in ["", "きのう", "2026-13-45T99:99:99Z", "1756000000"] {
        let fixture = fresh().await;
        // **正しい行も置く**（壊れた行だけを飛ばす実装を落とす）。
        insert_drift_charge_at(
            &fixture.store,
            fixture.character.get(),
            OTHER_AXIS,
            0.25,
            "2026-08-26T01:02:03.400Z",
        )
        .await;
        insert_drift_charge_at(
            &fixture.store,
            fixture.character.get(),
            CHARGED_AXIS,
            0.5,
            broken,
        )
        .await;

        let err = fixture
            .store
            .drift_charges(fixture.character)
            .await
            .err()
            .unwrap_or_else(|| panic!("壊れた時刻を読み飛ばした: {broken:?}"));
        assert!(
            matches!(err, StoreError::Query(_)),
            "別の理由で落ちた: {err}"
        );

        // **別のキャラクターの読みは止まらない。**
        assert!(
            charges(&fixture.store, fixture.bystander).await.is_empty(),
            "壊れた行が別のキャラクターの読みを止めた"
        );
        fixture.store.close().await;
    }
}
