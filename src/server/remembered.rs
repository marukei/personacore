//! **最後に選んだ上流プロファイルを覚える**（[ADR-0042](../../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)
//! §決定9 / 作業単位 F-4f / Issue origin#28）。
//!
//! ```text
//! %LOCALAPPDATA%\personacore\
//!   upstreams.json          … 利用者のファイル（ここには書かない）
//!   selected-upstream.txt   … 覚えた名前 1 行（このモジュール）
//! ```
//!
//! # 覚えるのは名前だけ
//!
//! **中身は `upstreams.json` が持つ**（写しを作らない）。**`upstreams.json` には書かない**——
//! 知らない鍵は読み込みで止まる（`upstreams` の検証規則）。**DB にも置かない**——
//! `--fresh` は DB を退避するが、どのモデルで起きるかは設定の側（利用者の決定）。
//!
//! # 置き場所は `upstreams.json` の隣（**既定の置き場所のときだけ**）
//!
//! **[`memory_for`] が [`ProfilesLocation::Editable`] からしか場所を作らない。**
//! `PERSONACORE_UPSTREAMS` が指す先（[`ProfilesLocation::ReadOnly`]）では覚えない
//! ——開発中の `start.ps1 -Upstream` の選択を、利用者の遊びの設定へ持ち込まない。
//! **画面から `upstreams.json` を書けるかと同じ1つの値から決める**（ADR-0042 §決定8）ので、
//! 「書けないのに覚える」「書けるのに覚えない」の配線が書けない。
//!
//! # 読む側と書く側
//!
//! | 誰が | いつ | どこで |
//! |---|---|---|
//! | 読む | 引数なしの起動 | `src/main.rs`（[`read_remembered`]） |
//! | 書く | `--upstream <名前>` で**起こせた**とき | `App::run_with_upstreams` |
//! | 書く | 画面の切り替えが受け付けられたとき | `switch::Switcher` |
//!
//! **書けなくても止めない**（警告だけ / [`remember_or_warn`]）。**置き換えで書く**
//! （`profile_editor::replace_file_with`。途中で落ちても、前の名前が読める形で残る）。
//!
//! # ⚠ 守れないもの
//!
//! * **2つの personacore が同時に書くと、後に置き換えた方が残る。** この実行ファイルの中では
//!   切り替えの錠（`Switcher` の `Supervisor`）の下で書くので、最後に受け付けた切り替えが残る。
//!
//! # 覚えた名前のプロファイルを起こせなかったら（ADR-0055 / 作業単位 F-10a）
//!
//! **覚えた名前で起きた起動は、gguf の面を起こさない**（`LaunchTiming::WhenAsked` / `not_started` で待つ）。
//! **起こす前に確かめられるのは gguf の実在と待ち受けとの取り合いまで**（`LaunchPlan::check_before_starting`）で、
//! 使えなければ今までどおり環境変数の既定で起きる（覚えた名前は消さない）。
//! **`command` を起こせない等は起こして初めて分かる**ので、**同じ名前で起こしたときに、その面の `failed`
//! として出る**（`switch` / 見張り番は `tests/upstream_cli.rs` の
//! `a_remembered_profile_whose_command_is_missing_waits_and_fails_only_when_asked`）。
//!
//! > **F-10a より前は、起動の中で起こして失敗し、`main` が `ServerError::Launch` を受けて
//! > 環境変数の既定で起き直していた**（F-4f のレビュー1周目の是正1）。起動時に起こさなくなり、その枝は消えた。

use std::path::{Path, PathBuf};

use super::profile_editor::{
    ProfilesLocation, RealSteps, ReplaceError, accepts_name, replace_file_with,
};

/// **覚えた名前を置くファイルの名前**（ADR-0042 §決定9）。
pub const REMEMBERED_FILE_NAME: &str = "selected-upstream.txt";

/// **覚えた名前の置き場所**——`upstreams.json` と同じディレクトリ。**純粋関数。**
///
/// **親の無い綴り**（`upstreams.json` だけ）ならカレントの下（`profile_editor::replace_file_with` と同じ扱い）。
#[must_use]
pub fn remembered_path_beside(profiles: &Path) -> PathBuf {
    profiles
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join(REMEMBERED_FILE_NAME)
}

/// **この起動が覚える先。** 既定の置き場所（画面から書ける）のときだけ `Some`。**純粋関数。**
pub(crate) fn memory_for(location: &ProfilesLocation) -> Option<PathBuf> {
    match location {
        ProfilesLocation::Editable(path) => Some(remembered_path_beside(path)),
        ProfilesLocation::ReadOnly(_) | ProfilesLocation::None => None,
    }
}

/// **覚えた名前を読めない**（`main` が警告にして、環境変数の既定で起きる）。
///
/// `Display` は**定数文字列**。理由は `#[source]` 側（[`std::io::Error`] は置き場所を含まない）。
/// **置き場所は呼び出し側がログに出す。**
#[derive(Debug, thiserror::Error)]
pub enum RememberedUnreadable {
    /// 読めない（権限・その名前がディレクトリ・UTF-8 でない）。
    #[error("覚えた上流プロファイルの名前を読めませんでした")]
    Io(#[source] std::io::Error),
    /// **1行ではない**（手で書き換えた）。どの行を採るかを推測しない。
    #[error("覚えた上流プロファイルの名前が1行ではありません")]
    NotOneLine,
    /// **制御文字を含む**（`--upstream` で選べない綴り）。
    #[error("覚えた上流プロファイルの名前に制御文字が含まれています")]
    ControlCharacter,
}

/// **ファイルの中身から名前を取り出す。純粋関数。**
///
/// * **前後の空白と BOM は落とす**（手で書き換えたときのメモ帳の癖を吸う）。
/// * **空なら「覚えていない」**（`None`）。
/// * **2行以上なら読まない**（[`RememberedUnreadable::NotOneLine`]）。
pub(crate) fn parse_remembered(text: &str) -> Result<Option<String>, RememberedUnreadable> {
    let name = text.trim_start_matches('\u{feff}').trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.contains(['\n', '\r']) {
        return Err(RememberedUnreadable::NotOneLine);
    }
    // **改行以外の制御文字（ESC・TAB 等）も読まない**——`--upstream` が受け付けない綴りで、
    // 名前はログに載る（作業単位 F-4f / レビュー1周目の是正3）。
    if name.chars().any(char::is_control) {
        return Err(RememberedUnreadable::ControlCharacter);
    }
    Ok(Some(name.to_owned()))
}

/// **覚えた名前を読む。** 無ければ `Ok(None)`（何も覚えていない）。
///
/// # Errors
///
/// 読めない、または1行でないとき。
pub fn read_remembered(path: &Path) -> Result<Option<String>, RememberedUnreadable> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_remembered(&text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(RememberedUnreadable::Io(err)),
    }
}

/// **覚えられなかった。** `Display` は定数。
#[derive(Debug, thiserror::Error)]
pub(crate) enum RememberError {
    /// **`--upstream <名前>` で選べない綴り**（覚えても、次の起動で同じ名前として引けない）。
    #[error("この名前は覚えられません")]
    Unrememberable,
    /// 置き換えで書けなかった。
    #[error("覚えた上流プロファイルの名前を書けませんでした")]
    Write(#[source] ReplaceError),
}

/// **ファイルに書く中身。純粋関数。** 名前1行（LF で終える）。
///
/// **`--upstream` で選べる綴りだけ**を覚える（[`accepts_name`] / 写しを作らない）——
/// 前後に空白を持つ名前や改行を含む名前は、読み戻すと別の名前になる。
/// **改行・制御文字も [`accepts_name`] が断る**（判定は `fresh::parse_args` / レビュー1周目の是正3）。
/// ここに写しの判定を置かない。
pub(crate) fn remembered_text(name: &str) -> Result<String, RememberError> {
    if !accepts_name(name) {
        return Err(RememberError::Unrememberable);
    }
    Ok(format!("{name}\n"))
}

/// **覚える。置き換えで書く。**
pub(crate) fn remember(path: &Path, name: &str) -> Result<(), RememberError> {
    let text = remembered_text(name)?;
    replace_file_with(&RealSteps, path, text.as_bytes()).map_err(RememberError::Write)
}

/// **覚える。書けなければ警告だけ出して続ける**（ADR-0042 §決定9「書けなくても止めない」）。
pub(crate) fn remember_or_warn(path: &Path, name: &str) {
    match remember(path, name) {
        Ok(()) => tracing::info!(
            upstream_profile = ?name,
            remembered_in = %path.display(),
            "上流プロファイルの名前を覚えました（次に引数なしで起動すると、これで起きます）"
        ),
        Err(err) => tracing::warn!(
            upstream_profile = ?name,
            remembered_in = %path.display(),
            error = %err,
            cause = ?std::error::Error::source(&err),
            "上流プロファイルの名前を覚えられませんでした（起動と切り替えは続けます）"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_memory_sits_beside_the_profiles_file() {
        let profiles = Path::new("root").join("personacore").join("upstreams.json");
        assert_eq!(
            remembered_path_beside(&profiles),
            Path::new("root")
                .join("personacore")
                .join("selected-upstream.txt")
        );
        // **親の無い綴り**はカレントの下。
        assert_eq!(
            remembered_path_beside(Path::new("upstreams.json")),
            Path::new(".").join("selected-upstream.txt")
        );
    }

    /// **覚えるのは既定の置き場所だけ**（`PERSONACORE_UPSTREAMS` が指す先では覚えない）。
    #[test]
    fn only_the_editable_location_remembers() {
        let path = Path::new("dir").join("upstreams.json");
        assert_eq!(
            memory_for(&ProfilesLocation::Editable(path.clone())),
            Some(Path::new("dir").join("selected-upstream.txt"))
        );
        assert_eq!(memory_for(&ProfilesLocation::ReadOnly(path)), None);
        assert_eq!(memory_for(&ProfilesLocation::None), None);
    }

    #[test]
    fn a_single_line_is_the_remembered_name() {
        assert_eq!(
            parse_remembered("beta\n").ok().flatten().as_deref(),
            Some("beta")
        );
        // **CRLF・BOM・前後の空白**（メモ帳で書き換えた形）。
        assert_eq!(
            parse_remembered("\u{feff}  beta \r\n\r\n")
                .ok()
                .flatten()
                .as_deref(),
            Some("beta")
        );
        // **名前の中の空白は残す。**
        assert_eq!(
            parse_remembered("my profile\n").ok().flatten().as_deref(),
            Some("my profile")
        );
    }

    #[test]
    fn an_empty_file_remembers_nothing() {
        for text in ["", "\n", "  \r\n", "\u{feff}"] {
            assert!(
                matches!(parse_remembered(text), Ok(None)),
                "{text:?} を名前として読んでいる"
            );
        }
    }

    /// **2行以上は推測しない**（どちらの行を採っても、利用者の意図と違いうる）。
    #[test]
    fn more_than_one_line_is_not_read() {
        for text in ["beta\ngamma\n", "beta\r\ngamma", "beta\rgamma"] {
            assert!(
                matches!(
                    parse_remembered(text),
                    Err(RememberedUnreadable::NotOneLine)
                ),
                "{text:?} を1行として読んでいる"
            );
        }
    }

    /// **改行以外の制御文字を含む名前も読まない**（ESC で色を偽る・TAB で欄を割る）。
    #[test]
    fn a_name_with_a_control_character_is_not_read() {
        for text in ["x\u{1b}[31m\n", "a\tb\n", "x\u{7f}"] {
            assert!(
                matches!(
                    parse_remembered(text),
                    Err(RememberedUnreadable::ControlCharacter)
                ),
                "{text:?} を名前として読んでいる"
            );
        }
    }

    /// **書く形は名前1行（LF）。読み戻すと同じ名前。**
    #[test]
    fn the_written_text_reads_back_as_the_same_name() {
        for name in ["beta", "my profile", "qwen-3.6"] {
            let text = remembered_text(name).expect("覚えられること");
            assert_eq!(text, format!("{name}\n"));
            assert_eq!(
                parse_remembered(&text).ok().flatten().as_deref(),
                Some(name)
            );
        }
    }

    /// **`--upstream` で選べない綴りは覚えない**（読み戻すと別の名前になる／選べない）。
    #[test]
    fn a_name_the_flag_cannot_select_is_not_remembered() {
        for name in [
            "",
            "  ",
            " beta",
            "beta ",
            "--fresh",
            "be\nta",
            "be\rta",
            "be\u{1b}ta",
        ] {
            assert!(
                matches!(remembered_text(name), Err(RememberError::Unrememberable)),
                "{name:?} を覚えている"
            );
        }
    }

    #[test]
    fn a_missing_file_remembers_nothing_and_a_written_one_reads_back() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let path = dir.path().join(REMEMBERED_FILE_NAME);
        assert!(matches!(read_remembered(&path), Ok(None)));

        remember(&path, "beta").expect("書けること");
        assert_eq!(std::fs::read(&path).expect("読めること"), b"beta\n");
        assert_eq!(
            read_remembered(&path).ok().flatten().as_deref(),
            Some("beta")
        );

        // **置き換える**（前の名前を残さない）。一時ファイルも残さない。
        remember(&path, "gamma").expect("書けること");
        assert_eq!(std::fs::read(&path).expect("読めること"), b"gamma\n");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("読めること")
            .flatten()
            .map(|entry| entry.file_name())
            .filter(|name| name != REMEMBERED_FILE_NAME)
            .collect();
        assert!(
            leftovers.is_empty(),
            "一時ファイルが残っている: {leftovers:?}"
        );
    }

    /// **その名前がディレクトリなら、読めない・書けない。どちらも panic しない。**
    #[test]
    fn a_directory_in_its_place_is_unreadable_and_unwritable() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let path = dir.path().join(REMEMBERED_FILE_NAME);
        std::fs::create_dir(&path).expect("置けること");

        assert!(matches!(
            read_remembered(&path),
            Err(RememberedUnreadable::Io(_))
        ));
        assert!(matches!(
            remember(&path, "beta"),
            Err(RememberError::Write(_))
        ));
        assert!(path.is_dir(), "置いたディレクトリが消えている");
    }
}
