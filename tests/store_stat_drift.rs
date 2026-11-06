//! **1軸だけを動かす経路**（[ADR-0024](../docs/adr/0024-stat-drift.md) §7 / 作業単位 16b）の検証。
//!
//! 見ているのは `Store::apply_stat_drift` の**文そのものの性質**で、
//! 「いつ呼ぶか」（`server::drift`）とは別。
//!
//! | 見るもの | 何が壊れたら赤になるか |
//! |---|---|
//! | 1軸だけが1段動く | 幅・軸の取り違え |
//! | **他の軸の更新を巻き戻さない** | 全置換（`replace_stat_allocs`）へ戻す |
//! | 現在値が違えば動かない | `AND value = ?` を落とす |
//! | 保存済みの尺度を超えたら動かない | 尺度の突き合わせを落とす |
//! | **別のキャラクターの同名の軸は動かない** | `character_id` の条件を落とす |
//! | 0 行更新は**エラーではない** | `NotMoved` を `Err` に変える |
//!
//! # `DriftChange` はここでは組み立てられない
//!
//! フィールドが private で、唯一の構築経路は `domain::apply_drift`（許可リスト）。
//! **テストも許可リストを通る**ので、「保存済みの軸だけが SQL へ届く」が
//! この経路の使い方そのものに現れている。

mod common;

use personacore::domain::{
    CharacterId, CharacterUpdate, DisplayName, DriftApply, DriftChange, DriftDelta, DriftOutcome,
    DriftProposal, DriftSuggestion, FieldUpdate, Handle, Persona, StatAlloc, StatAxis,
    StatDriftMode, StatScale,
};
use personacore::store::{DriftWrite, Store, StoreError};
use tempfile::TempDir;

use common::fixture_scale;

/// 動かす軸。**他の2軸とも、別キャラクターの軸とも十分違う綴り。**
const MOVED_AXIS: &str = "パッション";

/// 動かさない軸（同じキャラクター）。
const STILL_AXIS: &str = "ロジック";

/// **動かす軸と同じ値**を持つ、別の軸。
///
/// # なぜ「値が同じ軸」が要るのか（変異検査で見つかった穴 / 2026-08-23）
///
/// 軸の値が全部バラバラだと、**`axis = ?` の条件を落とす変異が緑を通る**
/// ——`value = ?` の条件だけで行が1つに絞られてしまうため（実測 1279 passed / 1 failed。
/// 落ちたのは文面を見る速報だけだった）。
///
/// **これは CLAUDE.md の「同値にすると取り違えが検出できない」の裏返しである。**
/// ここでは**わざと同値にしないと**、軸名で絞っていることを観測できない。
const TWIN_AXIS: &str = "共鳴";

/// **窓の中で別に更新される軸。** 全置換に戻すと、この軸の更新が巻き戻る。
const RIVAL_AXIS: &str = "好奇心";

/// 会話するキャラクターの識別子。
const HANDLE: &str = "aoi";

/// **会話しない**キャラクターの識別子。会話する方とは別の綴り。
const BYSTANDER_HANDLE: &str = "kaede";

/// 開いた DB と、そこに置いた2体のキャラクター。
struct Fixture {
    store: Store,
    /// **動かす方。** 後から作るので `id` は 1 ではない。
    character: CharacterId,
    /// **動かさない方。** 同じ綴りの軸を同じ値で持っている。
    bystander: CharacterId,
    _dir: TempDir,
}

/// キャラクターを1件作って id を返す。**表示名は識別子と別の綴り。**
async fn create(store: &Store, handle: &str, display_name: &str, scale: StatScale) -> CharacterId {
    let handle = Handle::parse(handle).expect("正当な識別子であること");
    store
        .create_character(
            &handle,
            &DisplayName::parse(display_name).expect("正当な表示名であること"),
            &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
            None,
            scale,
        )
        .await
        .expect("キャラクターを作成できること")
}

/// 軸と値の組から `StatAlloc` を作る。
fn allocs(scale: StatScale, pairs: &[(&str, u32)]) -> Vec<StatAlloc> {
    pairs
        .iter()
        .map(|(axis, value)| StatAlloc {
            axis: StatAxis::parse(axis).expect("正当な軸名であること"),
            value: scale.value(*value).expect("尺度に収まる値であること"),
        })
        .collect()
}

/// **2体のキャラクターに同じ綴りの軸を同じ値で置く。**
///
/// 同じにするのは、**持ち主の取り違えを観測できる形**にするため
/// （違う値にすると「どちらが動いたか」は分かるが、
/// 「両方動いた」を1つの assert で捕まえにくい）。
async fn fresh() -> Fixture {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let store = Store::connect(&dir.path().join("drift.sqlite"))
        .await
        .expect("DB を開けること");

    // **動かさない方を先に作る**（＝ `id` が 1 になるのはこちら）。
    let bystander = create(&store, BYSTANDER_HANDLE, "楓", fixture_scale()).await;
    let character = create(&store, HANDLE, "葵", fixture_scale()).await;
    assert_ne!(character, bystander, "前提: 2体の id が別であること");
    assert_ne!(
        character,
        CharacterId::from_raw_for_tests(1),
        "前提: 動かす方の id が 1 でないこと（決め打ちと同値にしない）"
    );

    store
        .replace_stat_allocs(
            character,
            &allocs(
                fixture_scale(),
                &[
                    (MOVED_AXIS, 4),
                    (TWIN_AXIS, 4),
                    (STILL_AXIS, 2),
                    (RIVAL_AXIS, 3),
                ],
            ),
        )
        .await
        .expect("性格ステータスを置けること");
    store
        .replace_stat_allocs(
            bystander,
            &allocs(fixture_scale(), &[(MOVED_AXIS, 4), (STILL_AXIS, 2)]),
        )
        .await
        .expect("傍観者の性格ステータスを置けること");

    Fixture {
        store,
        character,
        bystander,
        _dir: dir,
    }
}

/// 保存済みの軸を `(軸名, 値)` の組で読む。
async fn stats(store: &Store, id: CharacterId) -> Vec<(String, u32)> {
    store
        .stat_allocs(id)
        .await
        .expect("性格ステータスを読めること")
        .into_iter()
        .map(|alloc| (alloc.axis.as_str().to_owned(), alloc.value.get()))
        .collect()
}

/// **値を動かしてよいという許可証**（ADR-0026 §決定1）。
///
/// `Store::apply_stat_drift` はこれを引数に要求する。
/// **`StatDriftMode::On` からしか取れない**ので、
/// このファイルのテストが「on 相当の経路」を測っていることが呼び出しに現れる。
/// 記録専用モード（`shadow`）からこの文へ至る経路は**型として存在しない。**
fn permit() -> DriftApply {
    StatDriftMode::On
        .applying()
        .expect("on は許可証を発行すること")
}

/// **許可リストを通して**変容を組み立てる。適用できない提案なら落とす。
fn change(current: &[StatAlloc], axis: &str, delta: DriftDelta) -> DriftChange {
    let proposal = DriftProposal::Suggested(DriftSuggestion {
        axis: StatAxis::parse(axis).expect("正当な軸名であること"),
        delta,
    });
    match personacore::domain::apply_drift(current, proposal) {
        DriftOutcome::Applied(change) => change,
        other => panic!("適用できる提案であること: {other:?}"),
    }
}

/// **1軸だけが1段動く。** 他の軸も、別のキャラクターも動かない。
#[tokio::test]
async fn a_drift_moves_exactly_one_axis_by_one_step() {
    let fixture = fresh().await;
    let current = fixture
        .store
        .stat_allocs(fixture.character)
        .await
        .expect("読めること");

    let outcome = fixture
        .store
        .apply_stat_drift(
            fixture.character,
            &change(&current, MOVED_AXIS, DriftDelta::Up),
            permit(),
        )
        .await
        .expect("更新が成功すること");

    assert_eq!(outcome, DriftWrite::Moved, "1行動いたと返すこと");
    assert_eq!(
        stats(&fixture.store, fixture.character).await,
        vec![
            (MOVED_AXIS.to_owned(), 5),
            (TWIN_AXIS.to_owned(), 4),
            (STILL_AXIS.to_owned(), 2),
            (RIVAL_AXIS.to_owned(), 3),
        ],
        "動かした軸だけが1段上がること"
    );
    assert_eq!(
        stats(&fixture.store, fixture.bystander).await,
        vec![(MOVED_AXIS.to_owned(), 4), (STILL_AXIS.to_owned(), 2)],
        "同じ綴りの軸を持つ別のキャラクターは動かないこと"
    );

    fixture.store.close().await;
}

/// **下げも1段。**
#[tokio::test]
async fn a_downward_drift_moves_one_step_down() {
    let fixture = fresh().await;
    let current = fixture
        .store
        .stat_allocs(fixture.character)
        .await
        .expect("読めること");

    let outcome = fixture
        .store
        .apply_stat_drift(
            fixture.character,
            &change(&current, MOVED_AXIS, DriftDelta::Down),
            permit(),
        )
        .await
        .expect("更新が成功すること");

    assert_eq!(outcome, DriftWrite::Moved);
    assert_eq!(
        stats(&fixture.store, fixture.character).await,
        vec![
            (MOVED_AXIS.to_owned(), 3),
            (TWIN_AXIS.to_owned(), 4),
            (STILL_AXIS.to_owned(), 2),
            (RIVAL_AXIS.to_owned(), 3),
        ],
        "動かした軸だけが1段下がること"
    );

    fixture.store.close().await;
}

/// **窓の中で入った他の軸の更新を巻き戻さない**（ADR-0024 §7 の存在理由）。
///
/// 「全部読む → 1つ変える → 全部書く」（`replace_stat_allocs`）に戻すと、
/// **読んだ時点のスナップショットで書き戻す**ので、
/// その間に入った [`RIVAL_AXIS`] の更新が消える。
#[tokio::test]
async fn a_concurrent_update_to_another_axis_is_not_rolled_back() {
    let fixture = fresh().await;

    // 1. 読む（ここで変容を組み立てる）。
    let current = fixture
        .store
        .stat_allocs(fixture.character)
        .await
        .expect("読めること");
    let change = change(&current, MOVED_AXIS, DriftDelta::Up);

    // 2. **その窓の中で、別の軸が管理 API から書き換わる。**
    fixture
        .store
        .replace_stat_allocs(
            fixture.character,
            &allocs(
                fixture_scale(),
                &[
                    (MOVED_AXIS, 4),
                    (TWIN_AXIS, 4),
                    (STILL_AXIS, 2),
                    (RIVAL_AXIS, 6),
                ],
            ),
        )
        .await
        .expect("別の軸を書き換えられること");

    // 3. 書く。
    let outcome = fixture
        .store
        .apply_stat_drift(fixture.character, &change, permit())
        .await
        .expect("更新が成功すること");

    assert_eq!(outcome, DriftWrite::Moved);
    assert_eq!(
        stats(&fixture.store, fixture.character).await,
        vec![
            (MOVED_AXIS.to_owned(), 5),
            (TWIN_AXIS.to_owned(), 4),
            (STILL_AXIS.to_owned(), 2),
            (RIVAL_AXIS.to_owned(), 6),
        ],
        "窓の中で入った別の軸の更新が残ること（全置換に戻していない）"
    );

    fixture.store.close().await;
}

/// **現在値が変わっていたら動かさない。エラーにもしない**（ADR-0024 §7）。
///
/// 「読んでから書く」が1文に閉じているので、取りこぼしは**0 行更新として観測できる。**
#[tokio::test]
async fn a_stale_before_value_is_counted_as_not_moved() {
    let fixture = fresh().await;
    let current = fixture
        .store
        .stat_allocs(fixture.character)
        .await
        .expect("読めること");
    let change = change(&current, MOVED_AXIS, DriftDelta::Up);

    // **同じ軸が、窓の中で別の値へ動いた。**
    fixture
        .store
        .replace_stat_allocs(
            fixture.character,
            &allocs(
                fixture_scale(),
                &[
                    (MOVED_AXIS, 1),
                    (TWIN_AXIS, 4),
                    (STILL_AXIS, 2),
                    (RIVAL_AXIS, 3),
                ],
            ),
        )
        .await
        .expect("同じ軸を書き換えられること");

    let outcome = fixture
        .store
        .apply_stat_drift(fixture.character, &change, permit())
        .await
        .expect("0 行更新はエラーではないこと");

    assert_eq!(outcome, DriftWrite::NotMoved, "動かなかったと返すこと");
    assert_eq!(
        stats(&fixture.store, fixture.character).await,
        vec![
            (MOVED_AXIS.to_owned(), 1),
            (TWIN_AXIS.to_owned(), 4),
            (STILL_AXIS.to_owned(), 2),
            (RIVAL_AXIS.to_owned(), 3),
        ],
        "窓の中で入った値をそのまま残すこと"
    );

    fixture.store.close().await;
}

/// **尺度が下げられていたら動かさない。エラーにもしない**（ADR-0015 / ADR-0024 §7）。
///
/// `StatValue` は「どの尺度で検証したか」を覚えているが、
/// **その尺度がそのキャラクターのものかは型では守れない。**
/// だから突き合わせを**同じ文の中**で行う。
///
/// この状態は `PATCH /admin/characters/{handle}` から普通に作れる
/// （保存済みの値 4 以上の尺度なら通るので、尺度 4 への変更は受理される）。
#[tokio::test]
async fn a_value_beyond_the_stored_scale_is_counted_as_not_moved() {
    let fixture = fresh().await;
    let current = fixture
        .store
        .stat_allocs(fixture.character)
        .await
        .expect("読めること");
    // 4 → 5（尺度 7 で検証済み）。
    let change = change(&current, MOVED_AXIS, DriftDelta::Up);

    // **尺度を 4 まで下げる。** 保存済みの最大値がちょうど 4 なので受理される。
    let shrunk = StatScale::parse(4).expect("正当な尺度であること");
    assert_ne!(shrunk, fixture_scale(), "前提: 尺度が実際に変わること");
    fixture
        .store
        .update_character(
            &Handle::parse(HANDLE).expect("正当な識別子であること"),
            &CharacterUpdate {
                display_name: None,
                base_persona: None,
                output_format_instruction: FieldUpdate::Keep,
                stat_scale: Some(shrunk),
                stat_drift_mode: FieldUpdate::Keep,
                axis_discovery_slots: None,
                archived: None,
            },
        )
        .await
        .expect("尺度を下げられること");

    let outcome = fixture
        .store
        .apply_stat_drift(fixture.character, &change, permit())
        .await
        .expect("0 行更新はエラーではないこと");

    assert_eq!(outcome, DriftWrite::NotMoved, "動かなかったと返すこと");
    assert_eq!(
        stats(&fixture.store, fixture.character).await,
        vec![
            (MOVED_AXIS.to_owned(), 4),
            (TWIN_AXIS.to_owned(), 4),
            (STILL_AXIS.to_owned(), 2),
            (RIVAL_AXIS.to_owned(), 3),
        ],
        "尺度を超える値を書き込まないこと"
    );

    fixture.store.close().await;
}

/// **別のキャラクターの同名・同値の軸へは届かない**（帰属）。
///
/// 傍観者は `MOVED_AXIS` を**同じ値 4** で持っているので、
/// `character_id` の条件を落とす変異は「どちらが動いたか」ではなく
/// **「両方動いた」**として現れる。
#[tokio::test]
async fn a_change_built_for_one_character_never_moves_another() {
    let fixture = fresh().await;
    let current = fixture
        .store
        .stat_allocs(fixture.character)
        .await
        .expect("読めること");
    let change = change(&current, MOVED_AXIS, DriftDelta::Up);

    // **傍観者を宛先にして投げる。** 軸も値も一致しているが、持ち主が違う。
    let outcome = fixture
        .store
        .apply_stat_drift(fixture.bystander, &change, permit())
        .await
        .expect("エラーにしないこと");

    assert_eq!(
        outcome,
        DriftWrite::Moved,
        "宛先のキャラクターの軸が動くこと（この呼び出しは傍観者を指している）"
    );
    assert_eq!(
        stats(&fixture.store, fixture.bystander).await,
        vec![(MOVED_AXIS.to_owned(), 5), (STILL_AXIS.to_owned(), 2)],
        "指した方だけが動くこと"
    );
    assert_eq!(
        stats(&fixture.store, fixture.character).await,
        vec![
            (MOVED_AXIS.to_owned(), 4),
            (TWIN_AXIS.to_owned(), 4),
            (STILL_AXIS.to_owned(), 2),
            (RIVAL_AXIS.to_owned(), 3),
        ],
        "指していない方は動かないこと（`character_id` の条件が効いている）"
    );

    fixture.store.close().await;
}

/// **保存されていない軸へは行を作らない。**
///
/// 傍観者は [`RIVAL_AXIS`] を持たない。変容は**行を足す操作ではない**ので、
/// 0 行更新になり、軸の件数も変わらない。
#[tokio::test]
async fn a_drift_never_inserts_a_missing_axis() {
    let fixture = fresh().await;
    let current = fixture
        .store
        .stat_allocs(fixture.character)
        .await
        .expect("読めること");
    let change = change(&current, RIVAL_AXIS, DriftDelta::Up);

    let outcome = fixture
        .store
        .apply_stat_drift(fixture.bystander, &change, permit())
        .await
        .expect("エラーにしないこと");

    assert_eq!(outcome, DriftWrite::NotMoved);
    assert_eq!(
        stats(&fixture.store, fixture.bystander).await,
        vec![(MOVED_AXIS.to_owned(), 4), (STILL_AXIS.to_owned(), 2)],
        "軸を増やさないこと"
    );

    fixture.store.close().await;
}

/// **存在しないキャラクターでもエラーにしない**（0 行更新として返る）。
///
/// 「宛先が無い」を新しいエラーで名乗ると、呼び出し側（裏方のタスク）に
/// **本文と無関係な分岐**が増える。動かなかったことだけが分かればよい。
#[tokio::test]
async fn an_unknown_character_is_counted_as_not_moved() {
    let fixture = fresh().await;
    let current = fixture
        .store
        .stat_allocs(fixture.character)
        .await
        .expect("読めること");
    let change = change(&current, MOVED_AXIS, DriftDelta::Up);

    let outcome = fixture
        .store
        .apply_stat_drift(CharacterId::from_raw_for_tests(9_999), &change, permit())
        .await
        .expect("エラーにしないこと");

    assert_eq!(outcome, DriftWrite::NotMoved);

    fixture.store.close().await;
}

/// **プールが閉じていれば、それは 0 行更新ではなくエラー。**
///
/// 「動かなかった」と「そもそも書けなかった」を同じ値にすると、
/// **DB が壊れている回が「提案が適用されなかっただけ」に見える。**
#[tokio::test]
async fn a_broken_pool_is_an_error_not_a_quiet_zero() {
    let fixture = fresh().await;
    let current = fixture
        .store
        .stat_allocs(fixture.character)
        .await
        .expect("読めること");
    let change = change(&current, MOVED_AXIS, DriftDelta::Up);

    fixture.store.close().await;

    let err = fixture
        .store
        .apply_stat_drift(fixture.character, &change, permit())
        .await
        .expect_err("閉じたプールでは失敗すること");
    assert!(
        matches!(err, StoreError::Query(_)),
        "クエリの失敗として返すこと: {err:?}"
    );
}

/// **上端ちょうどへの書き込みは通る**（`?4 <= 尺度` の境界の内側）。
///
/// # なぜこの1本が要るのか（16b のレビューで見つかった穴 / 2026-08-23）
///
/// 尺度の条件を **`?4 < 尺度`**（`<=` から `<` へ）に狭める変異を当てると、
/// **赤くなったのは SQL の文面を見る速報**（`the_update_keeps_every_condition`）
/// **1本だけ**で、振る舞いを見るテストは38本すべて緑だった（実測）。
/// つまり**境界を守っていたのは、テスト側に書き写した文字列リテラルだけ**だった。
///
/// **入らなかった状態は `after == 尺度`**（上端ちょうどへ動かす書き込み）。
/// このファイルの他のフィクスチャは 4→5 しか動かさず、上端を扱う2本は
/// **`before == 尺度`** なので `apply_drift` が飽和で弾き、**SQL まで到達しない。**
///
/// # これは仮定ではない
///
/// 実機の測定（`scratchpad/m12/pairs.txt` の turn 21）で
/// **`好奇心 9->10`（尺度 10）が現に通っている。**
/// `<` に緩んだ実装ではこの書き込みが 0 行になり、
/// **「競合した」と区別が付かないまま値が上端の1つ下で永久に止まる。**
/// ADR-0024 §5 の飽和も鳴らない——飽和は `before == 尺度` のときだけなので、
/// **そこへ到達できなくなる。**
#[tokio::test]
async fn a_drift_that_lands_exactly_on_the_scale_is_written() {
    let top = fixture_scale().get();
    let fixture = fresh().await;

    // **上端の1つ下**に置く。ここから `+1` は「動ける」（飽和ではない）。
    fixture
        .store
        .replace_stat_allocs(
            fixture.character,
            &allocs(fixture_scale(), &[(MOVED_AXIS, top - 1), (STILL_AXIS, 2)]),
        )
        .await
        .expect("上端の1つ下に置けること");

    let current = fixture
        .store
        .stat_allocs(fixture.character)
        .await
        .expect("読めること");
    let change = change(&current, MOVED_AXIS, DriftDelta::Up);
    assert_eq!(
        change.after().get(),
        top,
        "前提: 上端ちょうどへ動かす変容であること（飽和ではない）"
    );

    let outcome = fixture
        .store
        .apply_stat_drift(fixture.character, &change, permit())
        .await
        .expect("更新が成功すること");

    assert_eq!(
        outcome,
        DriftWrite::Moved,
        "上端ちょうどは書けること（条件は `<=` であって `<` ではない）"
    );
    assert_eq!(
        stats(&fixture.store, fixture.character).await,
        vec![(MOVED_AXIS.to_owned(), top), (STILL_AXIS.to_owned(), 2)],
        "上端の値が保存されること"
    );

    fixture.store.close().await;
}
