//! **`calibration_target_from_env` が、現にプロセスの環境を読んでいる**ことの検証
//! （作業単位 E-3 / 変異9 の是正）。
//!
//! # なぜ専用のテストバイナリが要るのか
//!
//! E-3 は対象の決め方を**参照関数を受け取る純粋関数**（`calibration_target`）へ切り出し、
//! `tests/calibration_target.rs` の 11 本がそちらを見張っている。
//! **ところが 11 本は全部「テスト専用の入口」を通っていて、
//! 実際に使われる1行（`std::env::var` を差し込む側）を誰も通っていなかった。**
//!
//! **実測**（E-3 / 2026-08-28）——
//! **`calibration_target(|key| std::env::var(key).ok())` を `calibration_target(|_| None)` に
//! 変える変異が、1641 件すべて緑を通った。**
//! **環境を読まなくなっても、誰も気付かない状態だった。**
//!
//! CLAUDE.md が名指ししている形そのものである:
//!
//! > **テスト専用の入口を作ると、本番の入口が観測対象から外れる。**
//! > `App::bind_with_client` を足した結果、**`App::bind` の配線だけが誰も通らない状態**になった。
//!
//! **だからこのファイルが在る。** ここが「実際の入口を通る腕」。
//! 先例は `B-3` の `tests/log_filter_wiring.rs`（同じ理由で作られた）。
//!
//! # ⚠ このバイナリにテストを増やさないこと
//!
//! **このファイルのテストは1本だけである。それが安全の根拠。**
//! 下の1本は **`std::env::set_var` / `remove_var` でプロセスの環境を書き換える**ので、
//! **同じプロセスで別のテストが並行に走っていたら壊れる**
//! （`src/server/config.rs` の doc:「`std::env::set_var` は並行するテストを壊す」）。
//!
//! **統合テストはファイルごとに別プロセス**なので、他のテストバイナリには届かない。
//! **⚠ ここに2本目を足した瞬間、その隔離が消える。**
//! **両向き（設定が効く / 未設定なら既定）も、別のテストに割らず、下の1本の中へ入れること**
//! ——割ると、どちらが先に走るかで環境の状態が変わる。

mod common;

use common::calibration::{ENV_CALIBRATION_MODEL, calibration_target_from_env};
use personacore::llm::DEFAULT_MAIN_MODEL;

/// 環境へ入れる綴り。**既定と違うことが、このファイルの存在理由**
/// （CLAUDE.md「実行時に決まる値を入れたら、**既定と違う状態で回る腕**を必ず1本置くこと」）。
const TARGET_MODEL: &str = "ministral-3-3b-instruct-2512";

/// **環境変数が現に読まれている**——設定すれば効き、外せば既定へ戻る。
///
/// # 3段を1本にまとめてある理由（**両向きが1本に要るから**）
///
/// | 段 | 見るもの | これが無いと通る変異 |
/// |---|---|---|
/// | 1 | **未設定なら既定** | 環境の値を常に使う／別の綴りへ決め打つ |
/// | 2 | **設定した綴りが効く** | **環境を読まない**（＝変異9。`\|_\| None`）|
/// | 3 | **外せばまた既定へ戻る** | 1度目だけ読む／読んだ値を握り続ける |
///
/// **⚠ 別の `#[test]` に割らないこと。** cargo はテストを並行に走らせるので、
/// 割ると**環境をどちらが先に書き換えるかで結果が変わる。**
///
/// # なぜ `unsafe` が安全と言えるのか
///
/// `std::env::set_var` / `remove_var` が unsafe なのは、
/// **別のスレッドが同時に環境を読んでいると未定義動作になる**ため。
///
/// **このバイナリのテストは1本だけ**なので、書き換えている間に環境を読む
/// テストスレッドが存在しない（走っているのはこのテストだけ）。
/// **上のとおり、ここにテストを増やすとこの根拠が消える。**
///
/// **外から `PERSONACORE_CALIBRATION_MODEL` が設定されていても壊れない**——
/// 段1で必ず外してから見るため（校正を回している最中に `cargo test` しても落ちない）。
#[test]
fn the_calibration_target_is_read_from_the_process_environment() {
    assert_ne!(
        TARGET_MODEL, DEFAULT_MAIN_MODEL,
        "前提: 既定と違う綴りで回っていること（同値だと決め打ちと区別が付かない）"
    );

    // --- 1. 未設定なら既定 ------------------------------------------------
    // SAFETY: このバイナリのテストは1本だけで、環境を読む別スレッドが居ない（doc 参照）。
    unsafe { std::env::remove_var(ENV_CALIBRATION_MODEL) };

    let unset = calibration_target_from_env().expect("未設定なら既定で決まること");
    assert_eq!(
        unset.as_str(),
        DEFAULT_MAIN_MODEL,
        "未設定なのに既定以外を測ろうとしている"
    );

    // --- 2. 設定した綴りが効く --------------------------------------------
    // SAFETY: 同上。
    unsafe { std::env::set_var(ENV_CALIBRATION_MODEL, TARGET_MODEL) };

    let set = calibration_target_from_env().expect("正当な綴りであること");
    assert_eq!(
        set.as_str(),
        TARGET_MODEL,
        "環境に入れた綴りが対象になっていない（環境を読んでいない）"
    );

    // --- 3. 外せばまた既定へ戻る ------------------------------------------
    // SAFETY: 同上。
    unsafe { std::env::remove_var(ENV_CALIBRATION_MODEL) };

    let cleared = calibration_target_from_env().expect("未設定なら既定で決まること");
    assert_eq!(
        cleared.as_str(),
        DEFAULT_MAIN_MODEL,
        "外したのに前の綴りを測ろうとしている（読んだ値を握り続けている）"
    );
}
