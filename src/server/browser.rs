//! **待ち受けが開いたら、既定のブラウザで画面を開く**（作業単位 F-4c / Issue origin#23）。
//!
//! ```text
//! personacore（引数なし）
//!   ↓ App::bind が済んだ（待ち受け・DB まで開いた）
//!   ↓ 画面の URL（http://127.0.0.1:<実際のポート>/）
//!   Windows: cmd /C start "" <URL>
//!   Mac:     open <URL>
//!   その他:  xdg-open <URL>
//! ```
//!
//! **`--no-browser` で開かない**（`main` が [`crate::server::Startup`] に渡す）。
//! **`--no-ui` でも開かない**（開く頁が無い。決めるのは `App::run_with_upstreams` /
//! 作業単位 P-1。見張り番は `tests/upstream_cli.rs::the_no_ui_flag_never_opens_the_browser_and_says_so_on_one_line`）。
//! **`scripts/start.ps1` の `Start-Process $Url` はここへ移した**——同じことをする実装を2つ持たない
//! （持つと二重に開く。見張り番は `tests/start_script.rs::
//! the_start_script_leaves_opening_the_browser_to_the_executable`）。
//!
//! # 失敗しても止めない
//!
//! **開けなくても、待ち受けは続ける**（WARN に URL を出す。利用者が自分で開ける）。
//! **ブラウザが無い機（サーバ・CI）で起動そのものが落ちる形にしない。**
//!
//! # ⚠ テストは本物のブラウザを絶対に開かない（**司令塔の決定 / この機の EDR**）
//!
//! **`testing` フィーチャ付きのビルド**（`cargo test` が起こす実行ファイルはこれ）では、
//! **開くコマンドの名前だけを [`ENV_TEST_BROWSER`] から引く。**
//! **立っていなければ、何も起こさない**（INFO で「開かない」と言う）。
//! **つまり、テストを書いた人が偽物を渡し忘れても、本物は開かない。**
//! **見張り番は `tests/upstream_cli.rs::a_test_build_without_the_fake_opens_nothing`**
//! （偽物も `--no-browser` も渡さずに起こし、「開きません」の INFO を同じ行の中で見る）。
//! > **⚠ 初版はこの主張に見張り番が無かった**（レビュー1周目 / r10）——
//! > **環境変数が無いときに何かを起こす変異が 2197 全緑を通った。**
//!
//! ## なぜ `PATH` の先頭に偽物を置く形（偽の `llama` と同じ形）にしなかったか
//!
//! **Windows では届かない。** 本番の開き方は `cmd` を名前で起こすが、
//! **[`std::process::Command`] は Windows で `PATH` より先に「実行ファイルの置き場所」と
//! `System32` / Windows ディレクトリを探す**ので、`PATH` の先頭に置いた偽の `cmd.exe` は
//! **本物の `cmd.exe` に負ける**（`llama` は `System32` に無いので、`PATH` の偽物が勝つ）。
//! **Windows で開き方を `PATH` から引けるのは、`System32` に無い名前だけ**で、
//! URL を既定のブラウザへ渡す標準の手段はどれも `System32` か Windows ディレクトリに居る。
//!
//! ## 本番と何が違い、何が同じか
//!
//! | | 本番（`testing` 無し） | `testing` 付き |
//! |---|---|---|
//! | 開くかどうか（`--no-browser`） | 同じ | 同じ |
//! | いつ開くか（`bind` の後、`serve` の前） | 同じ | 同じ |
//! | URL（実際に掴んだポート） | 同じ | 同じ |
//! | **渡す引数**（`/C start "" <URL>` 等） | 同じ | **同じ**（偽物がそのまま受け取る） |
//! | **起こすコマンドの名前** | [`opener`] の名前（`cmd` 等） | [`ENV_TEST_BROWSER`] の値 |
//!
//! **違うのは最後の1行だけ**で、その名前は [`opener`] の単体テストが綴りで固定している。
//! **本物のブラウザが開くことそのものは、司令塔が実機で確かめる**（作業者・レビュアーは開かない）。
//!
//! ## ⚠ 守れないもの
//!
//! * **`testing` 無しの枝（[`program_for_this_build`] の本番の腕）は、どのテストからも走らない。**
//!   **そこを「何も起こさない」に変える変異は全緑を通る。** 守るのは実機の確認だけ。
//! * **`cargo test` の後の `target/debug/personacore.exe` は `testing` 付き**
//!   （`CARGO_BIN_EXE_personacore` がそれを指すので、テストのたびに上書きされる）。
//!   **その exe を直に叩いても、ブラウザは開かない**（INFO にそう出る）。
//!   **実機で確かめるときは、直前に素の `cargo build`（か `cargo run`）で組み直すこと。**
//!
//! # ⚠ つまみではない
//!
//! [`ENV_TEST_BROWSER`] を**読むのは `testing` 付きのビルドだけ**
//! （素の `cargo build` の実行ファイルは、この環境変数を1度も読まない）。
//! **⚠ 「存在しない」ではない**——綴りそのものは、[`open`] の INFO の文面として
//! 本番の実行ファイルにも1箇所入っている（本番では到達しない枝の文面。レビュー1周目の指摘4）。
//! **CLAUDE.md §1 の「つまみは最後の手段」に当たる設定項目を、本番には1つも足していない。**

use std::net::SocketAddr;

/// **`testing` 付きのビルドで、ブラウザの代わりに起こすコマンド**（作業単位 F-4c）。
///
/// **本番のビルドは読まない綴り**（[`program_for_this_build`] の doc）。
/// **`PERSONACORE_` で始める**——統合テストは親から `PERSONACORE_*` を1つも継がせないので、
/// **開発者のシェルに立っていても、テストへ漏れない。**
#[cfg_attr(not(any(test, feature = "testing")), allow(dead_code))]
pub const ENV_TEST_BROWSER: &str = "PERSONACORE_TEST_BROWSER";

/// **開き方を選ぶための OS の区別。** 引数で受けるので、**全部の腕をこの機で測れる**
/// （`config::data_dir_from` は `cfg` で1本しかコンパイルされない形だった）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    /// `cmd /C start "" <URL>`
    Windows,
    /// `open <URL>`
    Mac,
    /// `xdg-open <URL>`
    OtherUnix,
}

impl Platform {
    /// **この実行ファイルが組まれた OS。**
    pub(crate) const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::OtherUnix
        }
    }
}

/// **画面の URL。純粋関数。**
///
/// **実際に掴んだ宛先から作る**——設定のポートから作ると、
/// `PERSONACORE_PORT=0`（OS に選ばせる）で `:0` を開く。
///
/// # ⚠ `cmd` の特殊文字を入れないこと
///
/// Windows では [`opener`] が URL を **`cmd /C start` へそのまま渡す。**
/// **`&` / `|` / `^` / `%` 等が入ると `cmd` がそれを解釈する**（問い合わせ文字列を足した日に踏む）。
/// **いまの URL は `http://127.0.0.1:<数字>/` だけ**で、見張り番は
/// `the_ui_url_never_carries_characters_cmd_would_interpret`。
pub(crate) fn ui_url(addr: SocketAddr) -> String {
    format!("http://{addr}/")
}

/// **その OS で、URL を既定のブラウザへ渡すコマンドと引数。純粋関数。**
///
/// **Windows の `start` は、1つ目の引用された引数を窓の題名として読む**ので、
/// **空の題名（`""`）を先に渡す**——渡さないと、URL を引用した日に題名として食われる。
pub(crate) fn opener(platform: Platform, url: &str) -> (&'static str, Vec<String>) {
    match platform {
        Platform::Windows => (
            "cmd",
            vec![
                "/C".to_owned(),
                "start".to_owned(),
                String::new(),
                url.to_owned(),
            ],
        ),
        Platform::Mac => ("open", vec![url.to_owned()]),
        Platform::OtherUnix => ("xdg-open", vec![url.to_owned()]),
    }
}

/// **このビルドで起こすコマンドの名前。** `None` は「何も起こさない」。
///
/// **本番**（`testing` 無し）: [`opener`] の名前そのもの。
/// **`testing` 付き**: [`ENV_TEST_BROWSER`] の値。**立っていなければ `None`**
/// ——**テストが本物のブラウザを開く経路を、このビルドには作らない。**
#[cfg(not(any(test, feature = "testing")))]
fn program_for_this_build(default: &'static str) -> Option<std::ffi::OsString> {
    Some(default.into())
}

/// **このビルドで起こすコマンドの名前。** `None` は「何も起こさない」
/// （本番の腕の doc を参照）。
#[cfg(any(test, feature = "testing"))]
fn program_for_this_build(_default: &'static str) -> Option<std::ffi::OsString> {
    std::env::var_os(ENV_TEST_BROWSER).filter(|value| !value.is_empty())
}

/// **既定のブラウザで `url` を開く。失敗しても止めない。**
///
/// **起こしたコマンドの終わりは待たない**（別のタスクで待ち、失敗なら WARN）。
/// `cmd /C start` も `open` もすぐ返るが、**待ち受けの開始を、他人のプロセスの都合で遅らせない。**
pub(crate) fn open(url: &str) {
    let (default_program, args) = opener(Platform::current(), url);
    let Some(program) = program_for_this_build(default_program) else {
        tracing::info!(
            url,
            "テスト用のビルドなので、ブラウザは開きません（{ENV_TEST_BROWSER} が未設定）"
        );
        return;
    };

    match tokio::process::Command::new(&program)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            tracing::info!(url, "既定のブラウザで画面を開きます");
            let url = url.to_owned();
            tokio::spawn(async move {
                match child.wait().await {
                    Ok(status) if status.success() => {}
                    Ok(status) => tracing::warn!(
                        url = %url,
                        %status,
                        "ブラウザを開けなかったかもしれません（url の画面を手で開いてください）"
                    ),
                    Err(err) => tracing::warn!(
                        url = %url,
                        error = %err,
                        "ブラウザを開くコマンドの終わりを確かめられませんでした（url の画面を手で開いてください）"
                    ),
                }
            });
        }
        Err(err) => tracing::warn!(
            url,
            error = %err,
            "ブラウザを開けませんでした（url の画面を手で開いてください）"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **開き方の綴りを、OS ごとに固定する**（作業単位 F-4c）。
    ///
    /// **期待値を直に書く**（[`opener`] から作らない / CLAUDE.md `16b`）。
    /// **本番で起こすコマンドの名前を見ているのは、ここだけ**
    /// （統合テストは名前を偽物に差し替える / モジュールの doc の表）。
    #[test]
    fn each_platform_opens_the_url_with_its_own_command() {
        let url = "http://127.0.0.1:54321/";
        assert_eq!(
            opener(Platform::Windows, url),
            (
                "cmd",
                vec![
                    "/C".to_owned(),
                    "start".to_owned(),
                    String::new(),
                    url.to_owned()
                ]
            )
        );
        assert_eq!(opener(Platform::Mac, url), ("open", vec![url.to_owned()]));
        assert_eq!(
            opener(Platform::OtherUnix, url),
            ("xdg-open", vec![url.to_owned()])
        );
    }

    /// **この機が名乗る OS**（`cfg!` の取り違えを見る）。
    #[test]
    fn the_current_platform_is_the_one_this_was_built_for() {
        let expected = if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::OtherUnix
        };
        assert_eq!(Platform::current(), expected);
    }

    /// **URL は実際に掴んだ宛先から作る。**
    #[test]
    fn the_ui_url_is_the_root_of_the_address_actually_bound() {
        let addr: SocketAddr = "127.0.0.1:54321".parse().expect("宛先として読めること");
        assert_eq!(ui_url(addr), "http://127.0.0.1:54321/");
    }

    /// **URL に `cmd` が解釈する字を入れない**（Windows では `cmd /C start` へそのまま渡る）。
    ///
    /// **許す字の集合を直に書く**——`&` 等の禁止字を並べる形だと、並べ忘れた字が通る。
    #[test]
    fn the_ui_url_never_carries_characters_cmd_would_interpret() {
        for port in [1_u16, 8080, 65_535] {
            let url = ui_url(SocketAddr::from(([127, 0, 0, 1], port)));
            assert!(
                url.chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '/' | '.')),
                "cmd が解釈しうる字が入っている: {url}"
            );
        }
    }
}
