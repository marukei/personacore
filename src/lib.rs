//! personacore — ローカルLLMキャラクター会話基盤のオーケストレーション層。
//!
//! 単一クレート + モジュール分割（ADR-0002）。モジュール境界は将来 workspace へ
//! 分割できるよう crate 境界のつもりで引く。方針の詳細は CLAUDE.md と docs/ を参照。

pub mod domain;
pub mod llm;
/// 実測由来の定数の来歴（ADR-0033 §決定1 / 作業単位 E-2）。
///
/// **通信を持たない純粋関数と、その報告だけ。** 上流への照会は `llm` が行い、
/// 結果（モデル一覧）を引数で受け取る。
pub mod measured;
pub mod parse;
pub mod prompt;
pub mod server;
pub mod store;

/// ログ観測テストの共通土台。**テストビルドにしか存在しない。**
///
/// `#[cfg(feature = "testing")]` にしないこと（理由はモジュールの doc）。
#[cfg(test)]
mod testlog;

/// 集計キーの綴りが**ログの欄として読み戻せる形か**の判定（作業単位 B-40）。
/// **テストビルドにしか存在しない。**
///
/// `#[cfg(feature = "testing")]` にしないこと（`testlog` と同じ理由）。
#[cfg(test)]
mod aggkey;
