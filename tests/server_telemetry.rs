//! `init_tracing` の**結線**の検証。
//!
//! フィルタそのものの挙動は `server::telemetry` のユニットテストで固定してある。
//! ここで見るのは **`init_tracing` が実際にそのフィルタを使ってグローバルへ設定しているか**
//! という1点だけ。`init_tracing` の中身を `EnvFilter::from_default_env()` に取り替えると、
//! `RUST_LOG` 未設定時に `ERROR` だけになり `store` の警告が黙って消えるが、
//! ユニットテストはすべて緑のままになる（呼ばれない関数を検査しても意味が無いため）。
//!
//! # なぜ専用のテストバイナリなのか
//!
//! `tracing` の「現在の最大レベル」は**プロセス全体で1つ**の値で、
//! 他のテストが `with_default` で差した一時的な subscriber に引きずられる。
//!
//!   * **押し上げられる** — 並行して走る `with_default(EnvFilter::new("trace"))` が
//!     観測値を上げてしまい、既定より緩いフィルタでも緑になる（偽陰性）。
//!   * **下げられる側にも壊れる** — `with_default` を抜けても最大レベルは復元されず、
//!     **最後に走った scoped subscriber の値が残る。** 隣に
//!     `EnvFilter::new("error")` 相当を差すテストがあると、
//!     このテストは理由なく赤くなる（偽陽性）。
//!
//! どちらも観測値が他のテストに依存する形なので、
//! **このファイルにはテストを1つしか置かない**（統合テストはファイルごとに別プロセス）。

use tracing::level_filters::LevelFilter;

/// `init_tracing` が設定した subscriber は、**少なくとも `info` を通す。**
///
/// `store` の「正規化されていないキャラ名の行を除外しました」等の `warn` も、
/// 起動ログ・パース成功率などの `info` もここで拾われる。
#[test]
fn init_tracing_installs_a_subscriber_that_keeps_warnings() {
    // このバイナリはテスト1つ（モジュール doc 参照）。**ここより前に環境変数へ触る
    // スレッドは存在しない**ので、`remove_var` の安全条件はその制約がそのまま根拠になる。
    // 端末側の `RUST_LOG` に結果を左右されると、結線の検証にならない
    // （`RUST_LOG=warn` を敷いた端末では `info` の水準に届かず落ちる）。
    unsafe { std::env::remove_var("RUST_LOG") };

    personacore::server::init_tracing().expect("1回目は成功すること");

    assert!(
        personacore::server::init_tracing().is_err(),
        "2回目は失敗すること（= グローバルに設定している証拠）"
    );

    let current = LevelFilter::current();
    assert!(
        current >= LevelFilter::WARN,
        "既定フィルタの床（warn）が効いていること（実際の水準 = {current}）"
    );
    assert!(
        current >= LevelFilter::INFO,
        "既定フィルタの personacore=info が効いていること（実際の水準 = {current}）"
    );
}
