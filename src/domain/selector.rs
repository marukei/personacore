//! OpenAI 互換リクエストの `model` フィールドの解釈（ADR-0004）。
//!
//! `model` は**クライアントから来る信頼できない入力**である。ここは書式を解釈するだけで、
//! **キャラクターの実在確認（許可リスト照合）もセッションの所有確認も行わない。**
//! それらは `store` に問い合わせる後段（作業単位 11a-2）の責務。
//!
//! ADR-0004 の「`model` の解釈は単一の関数に閉じ込める」がこのモジュール。
//! ヘッダ優先（`X-Personacore-Session`）を後から足すときも、差込口はここ1箇所になる。

use super::character::{Handle, HandleError};

/// 新規セッションを指す予約接尾辞。
///
/// **完全一致（小文字のみ）で判定する。** `NEW` / `New` / ` new` を通すあいまい一致は
/// 入れない。`aoi@NEW` のつもりが「`NEW` という名前のセッション指定」に化けるより、
/// 明確に拒否して書き直させる方が事故が少ない（ADR-0004 は確実性を最優先する）。
const NEW_SUFFIX: &str = "new";

/// `model` フィールドを解釈した結果。
///
/// **`ModelSelector::parse` が唯一のコンストラクタ。** フィールドは私有にしてあり、
/// 構造体リテラルでは組み立てられない（`Handle` / `ValidTag` と同じ発想）。
/// これにより「書式の解釈が2箇所に分かれて片方だけ直る」経路が型レベルで無くなる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSelector {
    /// 検証済みの識別子。**実在確認はまだ通っていない。**
    handle: Handle,
    /// どのセッションを指しているか。
    session: SessionRef,
}

impl ModelSelector {
    /// `model` フィールドを解釈する。**唯一のコンストラクタ。**
    ///
    /// | `model` | 結果 |
    /// |---|---|
    /// | `aoi` | `SessionRef::Latest` |
    /// | `aoi@new` | `SessionRef::New` |
    /// | `aoi@42` | `SessionRef::Existing(42)` |
    ///
    /// **最初の `@` で1回だけ分割する。** 多重 `@` の特別扱いは書かない。
    /// `a@o@i` は接尾辞が `o@i` になり、`new` でも数値でもないので自然に拒否される
    /// （最後の `@` で分割すると識別子部に `@` が残り、別の理由で拒否されるため、
    /// **どちらで分割したかはエラーの理由まで見ないと分からない**）。
    ///
    /// 識別子部の検証は `Handle::parse` に委ねる。
    /// **識別子部も接尾辞も正規化しない**（`aoi@ new` も ` aoi@new` も拒否）。
    ///
    /// > 0004b-1 より前は識別子部だけを trim していた（「名前は照合キー、接尾辞は閉じた語彙」
    /// > という非対称）。ADR-0017 で識別子から正規化そのものを外したので、
    /// > **いまは両側とも入力そのままを見る。**
    pub fn parse(raw: &str) -> Result<Self, SelectorError> {
        // `split_once` は**最初の**区切りで1回だけ分ける。`rsplit_once` にしないこと。
        let (handle_part, suffix) = match raw.split_once('@') {
            Some((handle_part, suffix)) => (handle_part, Some(suffix)),
            None => (raw, None),
        };

        let handle = Handle::parse(handle_part).map_err(SelectorError::Handle)?;

        let session = match suffix {
            None => SessionRef::Latest,
            Some(suffix) if suffix == NEW_SUFFIX => SessionRef::New,
            Some("") => return Err(SelectorError::EmptySuffix),
            Some(suffix) => SessionRef::Existing(parse_session_number(suffix)?),
        };

        Ok(Self { handle, session })
    }

    /// 検証済みの識別子。許可リストとの照合はこの値で完全一致させる。
    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    /// どのセッションを指しているか。
    pub const fn session(&self) -> SessionRef {
        self.session
    }

    /// 2つの部品へ分解する（識別子を move で取り出したいとき用）。
    pub fn into_parts(self) -> (Handle, SessionRef) {
        (self.handle, self.session)
    }
}

/// `model` の接尾辞が指すセッション。
///
/// # `Existing` が `SessionId` ではない理由
///
/// **`SessionId` が存在する = そのキャラクターの所有確認を通った**、が crate 全体の
/// 不変条件（`domain::session` の doc）。`aoi@42` の `42` は**クライアントから来る
/// 未検証の値**なので、ここで `SessionId` を作ると他人のセッションを指す値が
/// 「DB から復元した値」と見分けが付かなくなる。
/// `Store::find_session` が生の `i64` を受け取るのはこのため。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionRef {
    /// 接尾辞なし。最新セッション（`session.updated_at` が最大）を継続する。
    /// 1つも無ければ暗黙に作成する（解決は `store` の責務）。
    Latest,
    /// `@new`。新規セッションを作成する。
    New,
    /// `@<id>`。**未検証の生の id。** 所有確認を通すまで `SessionId` にしない。
    Existing(i64),
}

/// 数値の接尾辞を解釈する。
///
/// 検査は次の順。**どれか1つを外すと通ってしまう入力がある**（各行のコメント参照）。
fn parse_session_number(suffix: &str) -> Result<i64, SelectorError> {
    // 1. ASCII 数字のみ。**`str::parse::<i64>` に頼らないこと。**
    //    `parse` は `+5` を受理し、`-1` も（値としては）解釈してしまう。
    if !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(SelectorError::InvalidSuffix);
    }
    // 2. 冗長な先頭 `0` を持たない。`0042` と `42` が同じセッションを指す表記ゆれを
    //    作らないため。**`0` 単体はここでは弾かない**（1文字なら冗長ではない）。
    //    弾くのは 4 の下限検査で、**2 と 4 は片方だけでは足りない。**
    if suffix.len() > 1 && suffix.starts_with('0') {
        return Err(SelectorError::InvalidSuffix);
    }
    // 3. `i64` に収まる。溢れたら拒否する（panic しない）。
    let Ok(id) = suffix.parse::<i64>() else {
        return Err(SelectorError::InvalidSuffix);
    };
    // 4. 1 以上。`0` は主キーとして存在しない値なので入口で落とす。
    if id < 1 {
        return Err(SelectorError::InvalidSuffix);
    }

    Ok(id)
}

/// `model` フィールドを解釈できない理由。
///
/// `Display` は**定数文字列**にする（エラーレスポンスに載るため。CLAUDE.md 絶対制約）。
/// **入力そのものを埋め込まない。** `model` はクライアントから来る値なので、
/// 反射すると情報漏れ・注入の経路になる。詳細は `#[source]` 側に押し込む。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SelectorError {
    /// 識別子部が `Handle` として不正（空 / `@` / 制御文字 / 長すぎる / 規則外の文字）。
    ///
    /// 理由は `source` に残る。`Display` は変えない。
    ///
    /// **文言は「名前」ではなく「識別子」で書く**（作業単位 0004b-3 / `HandleError` の doc）。
    #[error("model のキャラクター識別子を解釈できません")]
    Handle(#[source] HandleError),

    /// `@` の後ろが空（`aoi@`）。**`aoi` と同じ扱いにしない。**
    #[error("model の @ の後ろが空です")]
    EmptySuffix,

    /// `@` の後ろが `new` でも正の整数でもない。
    #[error("model の @ の後ろを解釈できません")]
    InvalidSuffix,
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use crate::domain::{Handle, HandleError, ModelSelector, SelectorError, SessionRef};

    fn selector(raw: &str) -> ModelSelector {
        ModelSelector::parse(raw).expect("正当な model として解釈できること")
    }

    fn error(raw: &str) -> SelectorError {
        ModelSelector::parse(raw).expect_err("拒否されること")
    }

    /// 接尾辞が無ければ「最新セッションを継続」。
    #[test]
    fn a_bare_handle_selects_the_latest_session() {
        for raw in ["aoi", "s10", "aoi-2", "aoi_2", "42", "new", "-", "_"] {
            let selected = selector(raw);
            assert_eq!(selected.session(), SessionRef::Latest, "raw = {raw:?}");
            assert_eq!(selected.handle().as_str(), raw, "raw = {raw:?}");
        }
    }

    /// `aoi@new` は新規セッション。
    #[test]
    fn the_new_suffix_selects_a_fresh_session() {
        let selected = selector("aoi@new");
        assert_eq!(selected.session(), SessionRef::New);
        assert_eq!(selected.handle().as_str(), "aoi");

        // 識別子が `new` でも接尾辞と混ざらない。
        assert_eq!(selector("new@new").handle().as_str(), "new");
        assert_eq!(selector("new@new").session(), SessionRef::New);
    }

    /// `aoi@42` は指定セッションの継続。
    #[test]
    fn a_numeric_suffix_selects_an_explicit_session() {
        for (raw, expected) in [
            ("aoi@42", 42_i64),
            ("aoi@1", 1),
            ("s10@7", 7),
            ("aoi@9223372036854775807", i64::MAX),
        ] {
            assert_eq!(
                selector(raw).session(),
                SessionRef::Existing(expected),
                "raw = {raw:?}"
            );
        }
    }

    /// **`SessionRef::Existing` は生の `i64`。`SessionId` にしない。**
    ///
    /// `SessionId` が存在する = そのキャラクターの所有確認を通った、が crate 全体の
    /// 不変条件（`domain::session` の doc）。ここは未検証の入力なので作ってはならない。
    /// 型が変わったらこの束縛がコンパイルエラーになる。
    #[test]
    fn existing_holds_an_unverified_raw_id() {
        let SessionRef::Existing(raw) = selector("aoi@42").session() else {
            panic!("Existing であること");
        };
        let raw_i64: i64 = raw;
        assert_eq!(raw_i64, 42);
    }

    /// **識別子部も正規化しない**（ADR-0017 / 作業単位 0004b-1。**0009 以前と真逆**）。
    ///
    /// 以前は識別子部だけを trim していたので、` aoi @new` が `aoi@new` と同じ選択になった。
    /// いまは `Handle::parse` が空白を拒否するので、**別の選択にはならず落ちる。**
    ///
    /// ここが緑のまま識別子部の trim が復活すると、`model` から
    /// 「照合では一致するのに完全一致では引けない」値が再び入ってくる。
    #[test]
    fn the_handle_part_is_never_normalized() {
        // 空白（`Charset`）で落ちる形。
        for raw in [
            " aoi ",
            "\u{3000}aoi\u{3000}",
            " aoi @new",
            "\u{3000}aoi\u{3000}@42",
            "aoi @new",
            "aoi\u{00A0}",
        ] {
            assert_eq!(
                error(raw),
                SelectorError::Handle(HandleError::Charset),
                "識別子部を trim して受理してはいけない: raw = {raw:?}"
            );
        }

        // TAB / LF は制御文字でもあるので理由は `Control`。**受理しないことは同じ。**
        for raw in ["\taoi\n", "\raoi", "aoi\n@new"] {
            assert_eq!(
                error(raw),
                SelectorError::Handle(HandleError::Control),
                "識別子部を trim して受理してはいけない: raw = {raw:?}"
            );
        }

        // 受理した識別子部は**入力そのまま**。
        assert_eq!(selector("aoi@new").handle().as_str(), "aoi");
        assert_eq!(selector("aoi-v2@42").handle().as_str(), "aoi-v2");
    }

    /// 空・空白のみは名前が空として拒否する。
    #[test]
    fn rejects_a_blank_model() {
        for raw in [
            "",
            " ",
            "   ",
            "\t",
            "\n",
            "\u{000B}",
            "\u{000C}",
            "\r",
            "\u{3000}",
            "\u{00A0}",
            " \u{3000}\t\r\n",
        ] {
            assert_eq!(
                error(raw),
                SelectorError::Handle(HandleError::Empty),
                "raw = {raw:?}"
            );
        }
    }

    /// 識別子部が空なら拒否する（接尾辞が正当でも）。
    #[test]
    fn rejects_a_missing_handle() {
        for raw in ["@", "@new", "@42", " @new", "\u{3000}@1"] {
            assert_eq!(
                error(raw),
                SelectorError::Handle(HandleError::Empty),
                "raw = {raw:?}"
            );
        }
    }

    /// `aoi@` は接尾辞が空。**`aoi` と同じ扱いにしない。**
    #[test]
    fn rejects_an_empty_suffix() {
        assert_eq!(error("aoi@"), SelectorError::EmptySuffix);
        assert_eq!(error("aoi-v2@"), SelectorError::EmptySuffix);
    }

    /// **最初の `@` で1回だけ分割する。** 多重 `@` の特別扱いは書かない。
    ///
    /// 最後の `@` で分割すると識別子部に `@` が残り、`HandleError::AtSign` になる。
    /// **理由まで見ないとこの取り違えは検出できない**ので、変異が変わる先の
    /// `Name(AtSign)` ではないことを明示的に固定する。
    #[test]
    fn splits_on_the_first_at_sign_only() {
        for raw in ["a@o@i", "a@o@new", "a@1@2", "aoi@new@new", "aoi@42@"] {
            let err = error(raw);
            assert_eq!(err, SelectorError::InvalidSuffix, "raw = {raw:?}");
            assert_ne!(
                err,
                SelectorError::Handle(HandleError::AtSign),
                "最後の `@` で分割していないこと: raw = {raw:?}"
            );
        }
    }

    /// **`new` は完全一致（小文字のみ）。** あいまい一致を入れない。
    #[test]
    fn the_new_suffix_is_matched_exactly() {
        for raw in [
            "aoi@NEW",
            "aoi@New",
            "aoi@nEw",
            "aoi@nEW",
            "aoi@ｎｅｗ", // 全角
            "aoi@news",
            "aoi@newnew",
            "aoi@ne",
        ] {
            assert_eq!(error(raw), SelectorError::InvalidSuffix, "raw = {raw:?}");
        }
    }

    /// **接尾辞は trim しない。** 識別子部との非対称は意図したもの。
    #[test]
    fn the_suffix_is_never_trimmed() {
        for raw in [
            "aoi@ new",
            "aoi@new ",
            "aoi@\tnew",
            "aoi@ 1",
            "aoi@1 ",
            "aoi@\u{3000}1",
            "aoi@1\n",
        ] {
            assert_eq!(error(raw), SelectorError::InvalidSuffix, "raw = {raw:?}");
        }
    }

    /// 数値の接尾辞は **ASCII 数字のみ**。`str::parse::<i64>` の寛容さに頼らない。
    #[test]
    fn a_numeric_suffix_must_be_ascii_digits_only() {
        for raw in [
            "aoi@+1",       // `str::parse::<i64>` は受理してしまう
            "aoi@-1",       // 負値
            "aoi@+42",      //
            "aoi@1_000",    //
            "aoi@1.0",      //
            "aoi@4 2",      //
            "aoi@１",       // 全角数字
            "aoi@\u{0661}", // アラビア数字 U+0661
            "aoi@0x2a",     //
            "aoi@42abc",    //
            "aoi@abc",      //
        ] {
            assert_eq!(error(raw), SelectorError::InvalidSuffix, "raw = {raw:?}");
        }
    }

    /// 冗長な先頭 `0` を拒否する（`0042` と `42` の表記ゆれを作らない）。
    #[test]
    fn a_numeric_suffix_rejects_redundant_leading_zeros() {
        for raw in ["aoi@01", "aoi@0042", "aoi@00", "aoi@000000000000000001"] {
            assert_eq!(error(raw), SelectorError::InvalidSuffix, "raw = {raw:?}");
        }
    }

    /// **`0` は拒否する。** id は 1 以上。
    #[test]
    fn a_numeric_suffix_must_be_at_least_one() {
        assert_eq!(error("aoi@0"), SelectorError::InvalidSuffix);
        assert_eq!(error("aoi@-0"), SelectorError::InvalidSuffix);
        // 下限ちょうどは受理する。
        assert_eq!(selector("aoi@1").session(), SessionRef::Existing(1));
    }

    /// `i64` に収まらない数値は拒否する（panic もしない）。
    #[test]
    fn a_numeric_suffix_beyond_i64_is_rejected() {
        for raw in [
            "aoi@9223372036854775808", // i64::MAX + 1
            "aoi@9999999999999999999",
            "aoi@18446744073709551616", // u64::MAX + 1
        ] {
            assert_eq!(error(raw), SelectorError::InvalidSuffix, "raw = {raw:?}");
        }
        let long_digits = format!("aoi@1{}", "0".repeat(1_000));
        assert_eq!(error(&long_digits), SelectorError::InvalidSuffix);
        // 上限ちょうどは受理する。
        assert_eq!(
            selector("aoi@9223372036854775807").session(),
            SessionRef::Existing(i64::MAX)
        );
    }

    /// 識別子部の制御文字は `Handle` が弾く（理由も保持する）。
    #[test]
    fn rejects_control_characters_in_the_handle() {
        for raw in ["a\u{0007}b", "a\u{0007}b@new", "a\nb@42", "aoi\u{007F}"] {
            assert_eq!(
                error(raw),
                SelectorError::Handle(HandleError::Control),
                "raw = {raw:?}"
            );
        }
    }

    /// 識別子部の長さ上限も `Handle` に委ねる。
    #[test]
    fn rejects_a_too_long_handle() {
        let over_limit = "a".repeat(Handle::MAX_CHARS + 1);
        assert_eq!(
            error(&over_limit),
            SelectorError::Handle(HandleError::TooLong),
            "上限を1文字超えた名前は拒否すること"
        );
        assert_eq!(
            error(&format!("{over_limit}@new")),
            SelectorError::Handle(HandleError::TooLong)
        );
        // 上限ちょうどは受理する。
        let at_limit = "a".repeat(Handle::MAX_CHARS);
        assert_eq!(selector(&at_limit).handle().as_str(), at_limit);
        assert_eq!(
            selector(&format!("{at_limit}@42")).session(),
            SessionRef::Existing(42)
        );
    }

    /// エラーは理由を区別できる（`AxisError` のテストと同じ形）。
    #[test]
    fn error_variants_are_distinguishable() {
        let variants = [
            SelectorError::Handle(HandleError::Empty),
            SelectorError::EmptySuffix,
            SelectorError::InvalidSuffix,
        ];

        for (index, left) in variants.iter().enumerate() {
            assert!(
                !left.to_string().is_empty(),
                "理由を説明するメッセージがあること"
            );
            for right in &variants[index + 1..] {
                assert_ne!(left, right);
                assert_ne!(
                    left.to_string(),
                    right.to_string(),
                    "文言でも区別できること"
                );
            }
        }

        // 他のドメイン型のエラーと文言が被らない（応答で取り違えない）。
        assert_ne!(
            SelectorError::Handle(HandleError::Empty).to_string(),
            HandleError::Empty.to_string()
        );

        // **「名前」ではなく「識別子」と名乗る**（作業単位 0004b-3）。
        // 上の `assert_ne!` は互いに違うことしか見ないので、ここを
        // 「model のキャラクター名を解釈できません」に戻す変異は**緑を通る**（実測）。
        // `model` に書くのは識別子で、表示名（ADR-0011）はそこに現れない。
        let message = SelectorError::Handle(HandleError::Empty).to_string();
        assert!(
            message.contains("識別子"),
            "識別子の解釈に失敗したと名乗ること: {message}"
        );
        assert!(
            !message.contains("キャラクター名"),
            "「名前」と呼ばないこと（識別子と表示名のどちらか読めなくなる）: {message}"
        );
    }

    /// 識別子部の詳細は `source` 側に残る（`Display` は定数のまま）。
    #[test]
    fn the_handle_error_is_kept_in_the_source() {
        let err = error("a@o@");
        assert_eq!(err, SelectorError::InvalidSuffix);
        assert!(err.source().is_none(), "接尾辞のエラーに source は無い");

        let err = error("aoi\u{0007}");
        assert!(
            matches!(err, SelectorError::Handle(HandleError::Control)),
            "理由が保たれること"
        );
        let source = err.source().expect("source があること");
        assert_eq!(source.to_string(), HandleError::Control.to_string());

        // 値としては区別できるが、`Display` は同一の定数文字列。
        assert_ne!(
            SelectorError::Handle(HandleError::Empty),
            SelectorError::Handle(HandleError::TooLong)
        );
        assert_eq!(
            SelectorError::Handle(HandleError::Empty).to_string(),
            SelectorError::Handle(HandleError::TooLong).to_string(),
            "詳細は Display ではなく source に置くこと"
        );
    }

    /// **`Display` に入力を埋め込まない**（CLAUDE.md 絶対制約）。
    ///
    /// `model` はクライアントから来る値なので、そのまま反射するとエラー応答が
    /// 情報漏れ・注入の経路になる。
    #[test]
    fn error_display_is_a_constant_string() {
        let secrets = [
            "aoi@秘密の接尾辞",
            "秘密のキャラ\u{0007}@new",
            "秘密@",
            "秘密@0042",
            "秘密@9223372036854775808",
            "a@秘密@i",
        ];
        for raw in secrets {
            let message = error(raw).to_string();
            assert!(
                !message.contains("秘密"),
                "入力を含めないこと: raw = {raw:?} / message = {message}"
            );
        }

        let long = "秘密".repeat(Handle::MAX_CHARS);
        let message = error(&long).to_string();
        assert!(!message.contains("秘密"), "message = {message}");
        assert!(
            !message.contains(&Handle::MAX_CHARS.to_string()),
            "上限値も埋め込まない定数文字列であること: {message}"
        );

        // 数値もそのまま載せない。
        let message = error("aoi@0042").to_string();
        assert!(!message.contains("0042"), "message = {message}");
    }

    /// 解釈した2つの部品を取り出せる（`session` は `Copy`、`handle` は借用と move の両方）。
    #[test]
    fn exposes_both_parts() {
        let selected = selector("aoi@42");
        assert_eq!(selected.handle().as_str(), "aoi");
        assert_eq!(selected.session(), SessionRef::Existing(42));

        let (handle, session) = selected.into_parts();
        assert_eq!(handle.as_str(), "aoi");
        assert_eq!(session, SessionRef::Existing(42));
    }

    /// `SessionRef` は値で比較できる（取り違えが `assert_eq!` で見える）。
    #[test]
    fn session_ref_compares_by_value() {
        assert_eq!(SessionRef::Latest, SessionRef::Latest);
        assert_ne!(SessionRef::Latest, SessionRef::New);
        assert_ne!(SessionRef::New, SessionRef::Existing(1));
        assert_ne!(SessionRef::Existing(1), SessionRef::Existing(2));
        assert_eq!(SessionRef::Existing(42), SessionRef::Existing(42));
    }
}
