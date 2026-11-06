//! **`install_global_log` に渡したフィルタが、現に効いている**ことの検証（作業単位 B-3）。
//!
//! # なぜ専用のテストバイナリが要るのか
//!
//! B-3 で subscriber の差し込みを `src/testlog/assert.rs` の1本へ畳んだとき、
//! **フィルタの綴りが引数になった。** ところが**呼び出し側4本は全部
//! `DEFAULT_LOG_FILTER` を渡す**ので、
//! **関数の中で同じ定数へ決め打つ変異は、どのテストからも区別が付かない**
//! （CLAUDE.md「実行時に決まる値を入れたら、**既定と違う状態で回る腕**を必ず1本置くこと。
//! 既定で回っている限り、引いてこようが定数だろうが区別が付かない」——`B-7` と同じ形）。
//!
//! **既存の4本には足せない。** グローバルの subscriber は**バイナリで1回きり**なので、
//! あの4本はどれも既定のフィルタを1回使い切っている。
//! **だからこのファイルが在る。** ここが「既定と違う状態で回る腕」。
//!
//! # ⚠ このバイナリに**ログを出すテスト**を増やさないこと
//!
//! バッファはバイナリで1つなので、**別のテストが出した行が
//! [`DROPPED_MARKER`] の否定側を壊しうる。**
//! **ログを1行も出さない検査（2度目の呼び出しの検査など）も、下の1本の中へ入れること**
//! ——別の `#[test]` にすると**どちらが先に走るかで、どの綴りが差し込まれるかが変わる。**

mod common;

use std::panic::catch_unwind;

use common::install_global_log;
use personacore::server::DEFAULT_LOG_FILTER;

/// 差し込むフィルタ。**既定と別であることが、このファイルの存在理由。**
///
/// 既定（`warn,personacore=info,tower_http=warn`）より**厳しい**側を選ぶ。
/// 緩い側だと「既定でも出る行」しか作れず、決め打ちの変異を区別できない。
const CUSTOM_FILTER: &str = "error";

/// **既定なら通り、[`CUSTOM_FILTER`] なら落ちる**行の手掛かり。
const DROPPED_MARKER: &str = "既定のフィルタでだけ通る行";

/// **どちらのフィルタでも通る**行の手掛かり。**先出しに使う。**
const KEPT_MARKER: &str = "どちらのフィルタでも通る行";

/// **渡したフィルタが差し込まれ、後から別の綴りで差し替えられない。**
///
/// # 3段を1本にまとめてある理由（**順番が主張の一部だから**）
///
/// | 段 | 見るもの |
/// |---|---|
/// | 1 | 渡した綴りが**現に効いている**（関数の中の決め打ちではない） |
/// | 2 | **同じ綴り**での2度目は**落ちない**（対照。同じバッファが返る） |
/// | 3 | **違う綴り**での2度目は**落ちる**（B-3 の後退をここで止める） |
///
/// **別の `#[test]` に割らないこと。** cargo はテストを並行に走らせるので、
/// **3 が先に走れば、そちらが差し込む側になってしまう**（＝どの綴りが入るか不定）。
///
/// # `should_panic` を使わない理由
///
/// **`should_panic` は「落ちた」しか言わない。**
/// 同じ作業単位で実測している——`contents_since` を「**常に**落ちる」に変える変異は、
/// `should_panic` の本体を**緑のまま通した**（赤くなったのは対照の側だけ）。
/// ここは [`catch_unwind`] で受けて、**落ちた理由（2つの綴り）まで**見る。
#[test]
fn the_filter_handed_to_the_installer_is_the_one_that_takes_effect() {
    assert_ne!(
        CUSTOM_FILTER, DEFAULT_LOG_FILTER,
        "前提: 既定と違うフィルタで回っていること（同値だと決め打ちと区別が付かない）"
    );
    assert!(
        !KEPT_MARKER.contains(DROPPED_MARKER) && !DROPPED_MARKER.contains(KEPT_MARKER),
        "前提: 2つの手掛かりが互いの部分文字列でないこと"
    );

    // --- 1. 渡した綴りが現に効いている ---------------------------------
    let buffer = install_global_log(CUSTOM_FILTER);

    // `target` を `personacore` にするのは、**既定がこのターゲットだけ `info` を通す**から。
    // ここが既定に決め打たれていれば、下の `info!` が出てしまう。
    tracing::error!(target: "personacore", "{KEPT_MARKER}");
    tracing::info!(target: "personacore", "{DROPPED_MARKER}");

    let logged = buffer.contents();

    // 先出し: **そもそも捕捉できている**こと（黙っているだけで緑にしない）。
    assert!(
        logged.contains(KEPT_MARKER),
        "前提: 差し込んだ subscriber が現に書いていること: {logged:?}"
    );

    // 本体: **既定なら通る行が、落ちている。**
    assert!(
        !logged.contains(DROPPED_MARKER),
        "渡したフィルタ（{CUSTOM_FILTER:?}）ではなく既定（{DEFAULT_LOG_FILTER:?}）が\
         効いています: {logged:?}"
    );

    // --- 2. 対照: 同じ綴りでの2度目は落ちない -------------------------
    // **これが無いと、3 は「常に落ちる」実装でも緑になる。**
    let again = catch_unwind(|| install_global_log(CUSTOM_FILTER))
        .expect("同じ綴りでの2度目は落ちないこと（呼ぶたびに落ちるなら誰も使えない）");
    assert!(
        std::ptr::eq(again, buffer),
        "同じ綴りでの2度目は、同じバッファを返すこと"
    );

    // --- 3. 違う綴りでの2度目は落ちる ---------------------------------
    // **グローバルの subscriber は差し替えられない。** 黙って最初の綴りのまま走ると、
    // 後から足したテストが「何も出なかった」で緑を通る
    // （`install_global_log` の doc の表）。
    // 戻り値を捨てる（`expect_err` は `Ok` 側に `Debug` を要求するので、
    // **共有している型に `Debug` を足さずに済ませる**）。
    let payload = catch_unwind(|| {
        let _ = install_global_log(DEFAULT_LOG_FILTER);
    })
    .expect_err("違う綴りでの2度目は落ちること（黙って別の綴りで走らせない）");
    let message = payload
        .downcast_ref::<String>()
        .map_or_else(String::new, Clone::clone);

    // **落ちた理由まで見る**（「何かで落ちた」では、別の原因の panic と区別が付かない）。
    assert!(
        message.contains(CUSTOM_FILTER) && message.contains(DEFAULT_LOG_FILTER),
        "落ちた理由に、差してある綴りと渡した綴りの両方が出ること: {message:?}"
    );
}
