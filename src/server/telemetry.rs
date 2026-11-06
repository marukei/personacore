//! ログ出力（`tracing`）の初期化。
//!
//! # `warn` を落とさないこと
//!
//! `store` は「読み出せないキャラクターの行がある」等を `warn` で報告する。
//! 応答には格納値も内部の詳細も載せない方針は
//! **「詳細はログで追える」ことを前提に採用している**ので、
//! 既定フィルタが `warn` を落とすとその前提ごと崩れる（docs/06-roadmap.md）。
//!
//! `RUST_LOG` で上書きできるが、**上書きしなければ `warn` は必ず出る。**
//!
//! # `RUST_LOG` は「置き換え」ではなく「重ね書き」
//!
//! `RUST_LOG` は**全体の下限（`warn`）を土台に敷いた上に**重ねる。理由は
//! `EnvFilter` の書式で「レベル名のタイポ」が**エラーにならない**ため。
//! `RUST_LOG=infoo` は「ターゲット `infoo` を trace で」と解釈され、書式としては妥当なまま
//! **全体の下限を消し飛ばす**。しかもログ初期化前なので、その事実を警告する手段が無い。
//! 土台を敷いておけば、床を動かせるのは**明示的にグローバルレベルを書いたときだけ**になる
//! （`RUST_LOG=error` はちゃんと効く）。
//!
//! # 引き継がれるのは**床だけ**
//!
//! 土台に敷くのは `GLOBAL_LEVEL_FLOOR`（`warn` の1行）であって
//! `DEFAULT_LOG_FILTER` 全体ではない。したがって **`RUST_LOG` を1文字でも書いた時点で、
//! 既定の `personacore=info` / `tower_http=warn` は引き継がれない。**
//!
//! ```text
//! RUST_LOG=tower_http=debug
//!   → warn,tower_http=debug
//!   → personacore の起動ログ・パース成功率（info）が黙って消える
//! ```
//!
//! **無関係なターゲットを1つ追いたいだけで本クレートの info が落ちる**のは驚きだが、
//! 既定全体を土台にする方が害が大きい（`RUST_LOG=error` と書いても
//! `personacore=info` が残り、**明示的な指定が裏切られる**）。
//! 上げ直したいときは `RUST_LOG=personacore=info,tower_http=debug` のように併記する。
//! 運用者向けの説明は `docs/04-environment.md`。
//!
//! `println!` は使わない（絶対制約）。初期化そのものに失敗した場合は
//! ログを出す手段が無いので、`Result` で呼び出し側（`main`）へ返す。

use tracing_subscriber::EnvFilter;

/// ログフィルタを指定する環境変数名。
const ENV_LOG_FILTER: &str = "RUST_LOG";

/// 全体の下限。**`RUST_LOG` を指定した場合も土台として敷かれる。**
///
/// **`DEFAULT_LOG_FILTER` に差し替えないこと。** 土台が既定全体になると
/// `RUST_LOG=error` を指定しても `personacore=info` が生き残り、明示的な指定を裏切る
/// （`the_variable_can_lower_the_level` /
/// `an_unrelated_target_directive_only_inherits_the_floor` が検出する）。
const GLOBAL_LEVEL_FLOOR: &str = "warn";

/// `RUST_LOG` 未設定時のフィルタ。
///
/// * 全体は `warn`（**`store` の警告はここで拾われる**）。
/// * 本クレートは `info`（起動・停止・パース成功率などの運用ログ）。
/// * `tower_http` は `warn`（リクエストごとの `debug` ログでノイズにしない）。
///
/// **先頭の `warn`（= `GLOBAL_LEVEL_FLOOR`）を消さないこと。** 消すと `tracing` の既定
/// （`ERROR` のみ）に落ち、依存クレートの警告が黙って消える。
pub const DEFAULT_LOG_FILTER: &str = "warn,personacore=info,tower_http=warn";

/// ログの初期化に失敗した理由。
///
/// 実際に起きるのは「既に別の subscriber が設定されている」場合。
#[derive(Debug, thiserror::Error)]
#[error("ログ出力の初期化に失敗しました")]
pub struct TelemetryError(#[source] tracing::subscriber::SetGlobalDefaultError);

/// プロセス全体の subscriber を設定する。**起動時に1回だけ呼ぶ。**
pub fn init_tracing() -> Result<(), TelemetryError> {
    // **この行を `EnvFilter::from_default_env()` 等に置き換えないこと。**
    // `RUST_LOG` 未設定時に `ERROR` だけになり、`store` の警告が黙って消える。
    let filter = filter_from(|key| std::env::var(key).ok());

    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .finish();

    tracing::subscriber::set_global_default(subscriber).map_err(TelemetryError)
}

/// 参照関数からフィルタを組み立てる。**環境そのものには触らない。**
///
/// * 未設定（または空白のみ）→ 既定。
/// * 書式が壊れている → 既定（起動を止めない）。
/// * それ以外 → **全体の下限を土台に敷いた上で** `RUST_LOG` を重ねる。
///
/// 環境変数を読む以外の経路（テスト・将来の設定ファイル）はここに合流させる
/// （`AppConfig::from_lookup` と同じ形）。
fn filter_from<F>(lookup: F) -> EnvFilter
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = lookup(ENV_LOG_FILTER) else {
        return default_filter();
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return default_filter();
    }

    // 後ろの指示が前を上書きするので、`RUST_LOG=debug` / `RUST_LOG=error` は
    // 床ごと動かせる。一方でターゲット名として解釈される値（レベル名のタイポ）は
    // 床を動かせない。
    let layered = format!("{GLOBAL_LEVEL_FLOOR},{raw}");
    EnvFilter::try_new(layered).unwrap_or_else(|_| default_filter())
}

/// 既定フィルタ。**環境変数を見ない。**
fn default_filter() -> EnvFilter {
    EnvFilter::new(DEFAULT_LOG_FILTER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testlog::{
        VERBOSE_FILTER, captured_with_env_filter as captured, captured_with_filter,
    };

    /// 依存クレート相当のターゲット。**全体の下限でしか拾えない。**
    const DEPENDENCY_TARGET: &str = "some_dependency";

    /// `store` の「正規化されていない行を除外しました」に相当するターゲット。
    const STORE_TARGET: &str = "personacore::store";

    /// `TraceLayer` がリクエストごとに `debug` を出すターゲット。
    ///
    /// **既定では `warn` に抑えてある**（ノイズになるため）。
    /// 追いたい人が `RUST_LOG=tower_http=debug` と書く現実的な指定に使う。
    const TOWER_HTTP_TARGET: &str = "tower_http";

    /// マーカーイベントを流すクロージャを作る。
    ///
    /// **関数ではなくマクロにしてある。** `tracing` のコールサイトは
    /// マクロを書いた位置ごとに1つなので、関数に切り出すと
    /// 「同じコールサイトを異なるフィルタで評価する」形になり、
    /// interest キャッシュの挙動に結果が依存しうる。呼び出し側で展開すれば
    /// テストごとに別のコールサイトになる。
    macro_rules! markers {
        () => {
            || {
                tracing::warn!(target: DEPENDENCY_TARGET, "依存クレートのWARN");
                tracing::warn!(target: STORE_TARGET, "storeのWARN");
                tracing::debug!(target: TOWER_HTTP_TARGET, "tower_httpのDEBUG");
                tracing::info!("INFOのマーカー");
                tracing::debug!("DEBUGのマーカー");
                tracing::trace!("TRACEのマーカー");
            }
        };
    }

    /// `markers!()` が流す6本のマーカー。**否定側の主張の土台。**
    const EVERY_MARKER: [&str; 6] = [
        "依存クレートのWARN",
        "storeのWARN",
        "tower_httpのDEBUG",
        "INFOのマーカー",
        "DEBUGのマーカー",
        "TRACEのマーカー",
    ];

    /// **`!contains(..)` を主張する前に、その行が現に出せることを確かめる。**
    ///
    /// このモジュールのテストはほぼ全部が「このマーカーは出る／このマーカーは出ない」の
    /// 組で成り立っている。**イベント自体が出せなくなっていると否定側は全部緑**になり、
    /// `RUST_LOG` の解釈が壊れても気付けない。
    ///
    /// 具体的な壊れ方は2つ:
    ///
    /// * `markers!` から行が消える／ターゲット名が変わる。
    /// * `tracing` の callsite interest が `never` に焼き付く
    ///   （プロセス全体で1つのキャッシュ。`crate::testlog` の doc）。
    ///
    /// **同じ `markers!()` の展開を渡すこと。** 展開ごとに別の callsite になるので、
    /// 別々に展開したものを渡すと「別の行が出せる」ことしか確かめられない。
    fn assert_every_marker_is_live(emit: impl FnOnce()) {
        let verbose = captured_with_filter(VERBOSE_FILTER, emit);

        for marker in EVERY_MARKER {
            assert!(
                verbose.contains(marker),
                "前提: {marker} は trace フィルタなら現に出ること: {verbose:?}"
            );
        }
    }

    /// 与えた組だけを返す参照関数を作る（`config` の `lookup_of` と同じ形）。
    fn lookup_of(value: Option<&str>) -> impl Fn(&str) -> Option<String> {
        let value = value.map(str::to_owned);
        move |key| {
            if key == ENV_LOG_FILTER {
                value.clone()
            } else {
                None
            }
        }
    }

    /// **`RUST_LOG` 未設定なら既定。依存クレートの `warn` も `store` の `warn` も出る。**
    ///
    /// ここが消えると「黙って除外してログで気づかせる」方針の前提ごと崩れる。
    #[test]
    fn without_the_variable_it_keeps_every_warning() {
        let emit = markers!();
        assert_every_marker_is_live(emit);

        let output = captured(filter_from(lookup_of(None)), emit);

        assert!(
            output.contains("依存クレートのWARN"),
            "全体の下限 warn で依存クレートの警告を拾うこと: {output:?}"
        );
        assert!(
            output.contains("storeのWARN"),
            "store の警告を拾うこと: {output:?}"
        );
        assert!(
            output.contains("INFOのマーカー"),
            "本クレートの info は出ること: {output:?}"
        );
        assert!(
            !output.contains("DEBUGのマーカー"),
            "既定で debug は出さないこと: {output:?}"
        );
        assert!(
            !output.contains("TRACEのマーカー"),
            "既定で trace は出さないこと: {output:?}"
        );
        assert!(
            !output.contains("tower_httpのDEBUG"),
            "既定で tower_http のリクエストログは抑えること: {output:?}"
        );
    }

    /// **`RUST_LOG` を書いた時点で、引き継がれるのは床（`warn`）だけ。**
    ///
    /// `tower_http` を追うために**無関係なターゲットを1つ指定しただけ**で、
    /// 既定の `personacore=info` は引き継がれず、起動ログ・パース成功率のログが
    /// 黙って消える。驚くが、既定全体を土台にすると `RUST_LOG=error` が裏切られる
    /// （`the_variable_can_lower_the_level`）ので、こちらを選んでいる。
    ///
    /// **この副作用はコードの形からは読み取れない**ため、ここで固定する。
    /// 土台を `DEFAULT_LOG_FILTER` に広げる変更は、このテストが検出する。
    #[test]
    fn an_unrelated_target_directive_only_inherits_the_floor() {
        let emit = markers!();
        assert_every_marker_is_live(emit);

        let output = captured(filter_from(lookup_of(Some("tower_http=debug"))), emit);

        assert!(
            output.contains("tower_httpのDEBUG"),
            "指定したターゲットは debug まで出ること: {output:?}"
        );
        assert!(
            output.contains("依存クレートのWARN") && output.contains("storeのWARN"),
            "床（warn）は引き継がれること: {output:?}"
        );
        assert!(
            !output.contains("INFOのマーカー"),
            "既定の personacore=info は引き継がれないこと（引き継ぐのは床だけ）: {output:?}"
        );
    }

    /// **`RUST_LOG` で上書きできる。**
    #[test]
    fn the_variable_can_raise_the_level() {
        let emit = markers!();
        assert_every_marker_is_live(emit);

        let output = captured(filter_from(lookup_of(Some("debug"))), emit);

        assert!(
            output.contains("DEBUGのマーカー"),
            "RUST_LOG=debug で debug が出ること: {output:?}"
        );
        assert!(
            output.contains("依存クレートのWARN"),
            "上書きしても警告は消えないこと: {output:?}"
        );
        assert!(
            !output.contains("TRACEのマーカー"),
            "指定した水準を超えないこと: {output:?}"
        );
    }

    /// **`RUST_LOG` で下げることもできる**（明示的な指定は尊重する）。
    #[test]
    fn the_variable_can_lower_the_level() {
        let emit = markers!();
        assert_every_marker_is_live(emit);

        let output = captured(filter_from(lookup_of(Some("error"))), emit);

        assert!(
            !output.contains("依存クレートのWARN"),
            "明示的に error を指定したら warn は出さないこと: {output:?}"
        );
        assert!(
            !output.contains("INFOのマーカー"),
            "本クレートの info も落とすこと（既定の personacore=info を残さない）: {output:?}"
        );
    }

    /// **書式が壊れた `RUST_LOG` は既定へ落とす**（起動を止めない・黙って無効化しない）。
    #[test]
    fn a_malformed_variable_falls_back_to_the_default() {
        let emit = markers!();
        assert_every_marker_is_live(emit);

        for raw in [
            "==",
            "info,=",
            "bogus=notalevel",
            "!!!",
            "personacore=info=debug",
        ] {
            assert!(
                EnvFilter::try_new(raw).is_err(),
                "前提: {raw:?} は書式として不正であること"
            );

            let output = captured(filter_from(lookup_of(Some(raw))), emit);

            assert!(
                output.contains("依存クレートのWARN") && output.contains("storeのWARN"),
                "raw = {raw:?}: 既定へ落ちて警告が出ること: {output:?}"
            );
            assert!(
                !output.contains("DEBUGのマーカー"),
                "raw = {raw:?}: 既定へ落ちること（何でも出す状態にしない）: {output:?}"
            );
        }
    }

    /// **レベル名のタイポで警告が全部消えない。**
    ///
    /// `infoo` は「レベル」ではなく**「ターゲット `infoo` を trace で」**と解釈されるため
    /// 書式としては妥当で、単体で使うと全体の `warn` 床ごと消える。
    /// 初期化前なので警告を出す手段も無く、**黙って全ログが止まる。**
    /// 既定の床を土台に敷いてから重ねることで、この壊れ方を構造的に潰す。
    #[test]
    fn a_typo_in_the_variable_cannot_silence_warnings() {
        let emit = markers!();
        assert_every_marker_is_live(emit);

        // 対照: 素の `infoo` は妥当な書式で、しかも警告を全部消す。
        assert!(
            EnvFilter::try_new("infoo").is_ok(),
            "前提: infoo は書式として妥当（= 壊れた値として弾けない）"
        );
        let raw = captured(EnvFilter::new("infoo"), emit);
        assert!(
            !raw.contains("依存クレートのWARN") && !raw.contains("storeのWARN"),
            "前提: 素の infoo は警告を消してしまう: {raw:?}"
        );

        let output = captured(filter_from(lookup_of(Some("infoo"))), emit);

        assert!(
            output.contains("依存クレートのWARN"),
            "タイポでも依存クレートの警告は残ること: {output:?}"
        );
        assert!(
            output.contains("storeのWARN"),
            "タイポでも store の警告は残ること: {output:?}"
        );
    }

    /// 空（または空白のみ）の `RUST_LOG` は未設定と同じ扱い。
    #[test]
    fn a_blank_variable_is_treated_as_unset() {
        let emit = markers!();
        assert_every_marker_is_live(emit);

        for raw in ["", " ", "\t"] {
            let output = captured(filter_from(lookup_of(Some(raw))), emit);
            assert!(
                output.contains("INFOのマーカー") && !output.contains("DEBUGのマーカー"),
                "raw = {raw:?}: 既定と同じであること: {output:?}"
            );
        }
    }

    /// ターゲット単位の指定は**床を残したまま**効く。
    #[test]
    fn a_target_directive_keeps_the_floor() {
        let emit = markers!();
        assert_every_marker_is_live(emit);

        let output = captured(filter_from(lookup_of(Some("personacore=trace"))), emit);

        assert!(
            output.contains("TRACEのマーカー"),
            "指定したターゲットは trace まで出ること: {output:?}"
        );
        assert!(
            output.contains("依存クレートのWARN"),
            "指定していないターゲットの警告も残ること: {output:?}"
        );
    }

    /// 既定フィルタは**全体の下限として `warn`** を持つ。
    ///
    /// 先頭の `warn` を落とすと `tracing` の既定（ERROR のみ）になり、
    /// 依存クレート側の警告が黙って消える。
    #[test]
    fn default_filter_declares_a_global_warn_floor() {
        assert!(
            DEFAULT_LOG_FILTER.starts_with(GLOBAL_LEVEL_FLOOR),
            "全体の下限が {GLOBAL_LEVEL_FLOOR} であること: {DEFAULT_LOG_FILTER}"
        );
        // 書式として妥当であること（壊れていると環境変数無しでも既定に落ちない）。
        assert!(
            EnvFilter::try_new(DEFAULT_LOG_FILTER).is_ok(),
            "既定フィルタが書式として妥当であること"
        );
        assert!(
            EnvFilter::try_new(GLOBAL_LEVEL_FLOOR).is_ok(),
            "床だけでも書式として妥当であること"
        );
    }
}
