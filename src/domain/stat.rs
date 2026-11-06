//! 性格ステータス（軸と配分値）のドメイン型。

use std::fmt;

use super::text::{contains_control_or_line_break, is_effectively_blank, trim_blank};

/// 検証・正規化を通った性格ステータスの軸名。
///
/// `Handle` と同じく **`parse` が唯一のコンストラクタ**。軸は固定6軸にせず
/// 任意の文字列を許すため（docs/03-data-model.md）、検証を型に載せないと
/// 空文字の軸がそのままプロンプトに「: 3/5」として載る。
///
/// `Handle` とは**別の型**にしてある。どちらも中身は正規化済みの文字列だが、
/// 取り違えると「キャラ名を軸として保存する」類の事故が静かに通る。
///
/// # 長すぎる軸名を拒否する理由
///
/// 軸名は**削れない枠**（docs/03-data-model.md「上限を持つ型にする」）に載る自由文で、
/// しかも**軸の件数に上限が無い**。1軸あたりが長いほど載せられる軸が減るので、
/// ここが無制限だと「軸を1つ足したら以後のチャットが全部 `BudgetExhausted`」が起こりうる。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatAxis(String);

impl StatAxis {
    /// 軸名の上限。**文字数**（Unicode スカラー値の個数）で数える。
    ///
    /// # なぜ 20 文字か
    ///
    /// 軸名は「パッション」「ロジック」「内なる 情熱」のような**短いラベル**で、
    /// 実際の値はどれも数文字に収まる。20 文字はその4倍で正当な軸名を弾かない。
    ///
    /// 記憶の分類（`MemoryCategory::MAX_CHARS` = 50）より小さいのは、**書き手が違う**から。
    /// 分類は抽出サブLLM の生成物で、弾くと記憶ごと失われるため寛容さが要る。
    /// 軸名は運用者が `/admin/` から書く値で、弾かれてもその場で短くして送り直せる。
    ///
    /// バイト数で数えないのは `MemoryContent::MAX_CHARS` と同じ理由
    /// （UTF-8 では日本語1文字が3バイトで、日本語だけが 1/3 しか書けなくなる）。
    ///
    /// # **この値は後から下げられない**
    ///
    /// 書き込みの検証と**読み出しの検証**（`Store::stat_allocs` →
    /// `StoreError::InvalidStoredValue`）の両方に使われる。下げると保存済みの
    /// 長い軸名がそのキャラクターの性格ステータス取得を丸ごと止め、
    /// `store` に軸単位の削除が無いので置き換え（`replace_stat_allocs`）以外に
    /// 復旧手段が無い。上げる方向は安全。
    pub const MAX_CHARS: usize = 20;

    /// 唯一のコンストラクタ。正規化してから検証する。
    ///
    /// 1. 前後の空白（`text::TRIMMABLE`）を除去する。中間の空白は保持する。
    /// 2. 空になったら拒否する。
    /// 3. **見える文字が1つも残らないなら拒否する**（作業単位 0009a）。
    /// 4. 制御文字・行区切りを含むなら拒否する。
    /// 5. `MAX_CHARS` を超えるなら拒否する。
    ///
    /// **`@` は禁止しない。** 予約語と衝突するのは `model` に載るキャラクター識別子だけで、
    /// 軸名は `model` に現れない。
    pub fn parse(raw: &str) -> Result<Self, AxisError> {
        let normalized = trim_blank(raw);

        if normalized.is_empty() {
            return Err(AxisError::Empty);
        }
        // **見えない文字だけの軸名も「空」として拒否する**（作業単位 0009a）。
        // 0009 は行区切りしか見ておらず、`StatAxis::parse("\u{2000}")` が通って
        // **名前の無い軸に数値だけ**（`> - : 5`）が載っていた（実測）。
        // 直前の「記載のない軸は考慮しません。」と矛盾する形がそのまま残っていた。
        if is_effectively_blank(normalized) {
            return Err(AxisError::Empty);
        }
        if contains_control_or_line_break(normalized) {
            return Err(AxisError::Control);
        }
        // **長さの判定は正規化の後。** 前に置くと、前後に空白を足しただけで
        // 上限ちょうどの軸名が弾かれる。
        if normalized.chars().count() > Self::MAX_CHARS {
            return Err(AxisError::TooLong);
        }

        Ok(Self(normalized.to_owned()))
    }

    /// 正規化後の文字列。SQL へのバインド値はこれを使う。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StatAxis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 軸名が受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AxisError {
    /// 正規化すると空になる（空文字・空白のみ）。
    #[error("性格ステータスの軸名が空です")]
    Empty,

    /// 制御文字を含む。
    #[error("性格ステータスの軸名に改行や制御文字は使えません")]
    Control,

    /// 正規化後の文字数が `StatAxis::MAX_CHARS` を超える。
    ///
    /// **上限値をメッセージに埋め込まない。** `Display` は定数文字列に保つ。
    #[error("性格ステータスの軸名が長すぎます")]
    TooLong,
}

/// 性格ステータスの**尺度**（そのキャラクターの値が取りうる上端）。ADR-0015。
///
/// `StatAxis` / `Handle` と同じく **`parse` が唯一のコンストラクタ**。
/// `From<u32>` も `pub fn new` も生やさない。
///
/// # なぜ下限が `1` なのか
///
/// **`0` を許すと、性格ステータスを設定してあるのに一行も載らないキャラクターが作れる。**
/// 尺度が `0` なら値が取りうるのは `0` だけで、
/// 「性格値が 0 の軸はプロンプトに含めない」（CLAUDE.md 絶対制約）が**全軸に効く**。
/// 設定は保存されているのにプロンプトには何も現れず、
/// **エラーにも警告にもならない**ので「なんとなく前と違う」としか観測できない。
///
/// # 既定値と上限の根拠は [ADR-0015](../../docs/adr/0015-stat-scale-per-character.md)
///
/// **ここには写さない。** 写すと、測り直して ADR を訂正したときに
/// こちらだけが黙って古びる（CLAUDE.md「同じ発見を2箇所に書かない」）。
///
/// # **値を作れるのはこの型だけ**（作業単位 0011a-2）
///
/// `StatValue` のコンストラクタは `StatScale::value` ひとつで、
/// **尺度を持たずに配分値を作る手段が無い。** ただし守れる範囲には限界がある
/// ——`StatValue` の doc「この型が守らないもの」を必ず読むこと。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StatScale(u32);

impl StatScale {
    /// 尺度の下限。**`0` を許さない理由は型の doc にある。**
    pub const MIN: u32 = 1;

    /// 尺度の上限。根拠は ADR-0015（**表現できる範囲であって推奨ではない**）。
    pub const MAX: u32 = 100;

    /// 尺度を指定しなかったキャラクターに配られる値。根拠は ADR-0015。
    ///
    /// **`parse` を通る値であること**（`MIN..=MAX` の中）は
    /// `the_default_is_ten_and_parses` が見張っている。
    pub const DEFAULT: Self = Self(10);

    /// 唯一のコンストラクタ。`MIN..=MAX` の外は拒否する。
    ///
    /// 理由を2つに分けるのは、**送り直しの向き**（増やすのか減らすのか）が
    /// 応答から読めるようにするため。
    pub const fn parse(raw: u32) -> Result<Self, ScaleError> {
        if raw < Self::MIN {
            return Err(ScaleError::TooSmall);
        }
        if raw > Self::MAX {
            return Err(ScaleError::TooLarge);
        }
        Ok(Self(raw))
    }

    /// 尺度の値。SQL へのバインド値と応答にはこれを使う。
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// **`StatValue` の唯一のコンストラクタ**（ADR-0015 / 作業単位 0011a-2）。
    ///
    /// `0..=self.get()` の外は拒否する。**`0` は正当な値**
    /// （落とすのは `prompt` 層の責務。CLAUDE.md 絶対制約）。
    ///
    /// 作った値は**この尺度を覚える**ので、後段（`store`）が
    /// 「保存されている尺度のものか」を突き合わせられる。
    pub const fn value(self, raw: u32) -> Result<StatValue, ValueError> {
        if raw > self.0 {
            return Err(ValueError::AboveScale);
        }
        Ok(StatValue {
            value: raw,
            scale: self,
        })
    }
}

impl fmt::Display for StatScale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 尺度が受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
///
/// **上下限の数値をメッセージに埋め込まない。** `AxisError::TooLong` と同じ扱いで、
/// 埋め込むと `MIN` / `MAX` の写しが応答の中に増える。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScaleError {
    /// `StatScale::MIN` より小さい。
    #[error("性格ステータスの尺度が小さすぎます")]
    TooSmall,

    /// `StatScale::MAX` より大きい。
    #[error("性格ステータスの尺度が大きすぎます")]
    TooLarge,
}

/// **単体テスト用の尺度。必ず `StatScale::DEFAULT` と違う値。**
///
/// 既定を使うと「要求を無視して常に既定を返す」実装が緑を通る
/// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
/// 統合テスト側の同名ヘルパは `tests/common/mod.rs`（そちらに詳しい理由がある）。
///
/// **`#[cfg(test)]` なので `cargo build` には現れない。**
#[cfg(test)]
pub(crate) fn fixture_scale() -> StatScale {
    let scale = StatScale::parse(7).expect("正当な尺度であること");
    assert_ne!(scale, StatScale::DEFAULT, "既定と違う値であること");
    scale
}

/// **ある尺度による検証を通った**性格ステータスの配分値。ADR-0015 / 作業単位 0011a-2。
///
/// `StatScale::value` が唯一のコンストラクタで、`From<u32>` も `pub fn new` も生やさない
/// （`Handle` / `StatAxis` / `StatScale` と同じ）。負値は `u32` が、
/// 尺度超過は `StatScale::value` が拒む。
///
/// **`0` は正当な値。** 落とすのは `prompt` 層の責務で、保存段階では落とさない。
///
/// # 検証に使った尺度を覚えている
///
/// `3/100` と `3/10` は**別の性格**なので、数だけを持ち回ると
/// 「どちらの尺度で決めた値か」が復元できない。`scale()` がそれを返す。
///
/// # **この型が守らないもの**（ADR-0015 の 2026-08-19 追記）
///
/// 型が守るのは「**その尺度を超える値**」までで、そこが限界。
/// **「その尺度がそのキャラクターのものか」は型では守れない。**
/// `StatScale::parse(100)` から作った `3` も `StatScale::parse(10)` から作った `3` も
/// 同じ `StatValue` 型なので、**尺度 10 のキャラクターに前者を渡すコードはコンパイルが通る**
/// （2026-08-20 に実際に書いて確認した）。
///
/// だから照合は **`store` が保存済みの尺度と突き合わせる実行時の検査**になる
/// （`Store::replace_stat_allocs` → `StoreError::StatScaleMismatch`）。
/// **`ValidTag`（[ADR-0006](../../docs/adr/0006-slot-based-tag-compilation.md)）と
/// 同じ強さだと読まないこと。** 見張っているのは型ではなくテストである。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StatValue {
    value: u32,
    scale: StatScale,
}

impl StatValue {
    /// 配分値そのもの。SQL へのバインド値と応答にはこれを使う。
    #[must_use]
    pub const fn get(self) -> u32 {
        self.value
    }

    /// **この値を検証した尺度。** `store` が保存済みの尺度と突き合わせるのに使う。
    #[must_use]
    pub const fn scale(self) -> StatScale {
        self.scale
    }
}

/// 配分値が受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
///
/// **値も尺度もメッセージに埋め込まない**（`ScaleError` / `AxisError::TooLong` と同じ扱い）。
///
/// 理由が1つしか無いのは、**下側の逸脱を `u32` が型で潰している**から
/// （負値を表現できない）。「小さすぎる」に相当する入力が存在しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    /// そのキャラクターの尺度（`StatScale`）を超えている。
    #[error("性格ステータスの値が尺度を超えています")]
    AboveScale,
}

/// 1軸分の性格ステータス配分。
///
/// **`value == 0` は正当な値。** 落とすのは `prompt` 層の責務で、保存段階では落とさない。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StatAlloc {
    /// 軸名。
    pub axis: StatAxis,
    /// 配分値。**尺度による検証を通っている**（作業単位 0011a-2）。
    pub value: StatValue,
}

impl StatAlloc {
    /// **この軸がプロンプトに載るか。**
    ///
    /// **値 0 の軸は載らない**（CLAUDE.md 絶対制約）。「パッション: 0」と書くと
    /// モデルが「情熱の欠如」を過剰演技して無感情になるので、軸ごと落とす。
    ///
    /// # なぜ述語を型の側に置くのか
    ///
    /// 条件を**2箇所以上が見る**ようになったため（作業単位 0011c）。
    ///
    /// * `prompt::sanitize_character` — 載せる軸を選ぶ（絶対制約そのもの）。
    /// * `parse::leak::detect_axis_leaks` — **載った軸だけ**を漏れの検出対象にする
    ///   （ADR-0018。載っていない軸はモデルが読んでいないので、
    ///   一致してもそれは偶然の同語）。
    ///
    /// 写しにすると片方だけ動いたときに黙ってずれる。**同じ関数を通す。**
    pub const fn appears_in_prompt(&self) -> bool {
        self.value.get() != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LINE_BREAKS;
    use crate::domain::text::INVISIBLE_SAMPLES;

    /// 正規化すると空になる文字列。`Handle` と同じ集合。
    const BLANKS: [&str; 12] = [
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
        "\u{000B}\u{000C}\u{00A0}\u{3000} \t",
    ];

    fn parsed(raw: &str) -> StatAxis {
        StatAxis::parse(raw).expect("正当な軸名として解釈できること")
    }

    /// 前後の空白は除去される。
    #[test]
    fn trims_surrounding_blanks() {
        for (raw, expected) in [
            (" パッション ", "パッション"),
            ("\u{3000}ロジック\u{3000}", "ロジック"),
            ("\tlogic\n", "logic"),
            ("\u{00A0}logic\u{00A0}", "logic"),
            (" \u{3000}\t\r\n共感\u{000B} ", "共感"),
        ] {
            assert_eq!(parsed(raw).as_str(), expected, "raw = {raw:?}");
        }
    }

    /// 中間の空白は保持される（`"内なる 情熱"` のような軸名を潰さない）。
    #[test]
    fn keeps_inner_blanks() {
        for raw in ["内なる 情熱", "内なる\u{3000}情熱", "a b"] {
            assert_eq!(parsed(raw).as_str(), raw);
        }
    }

    /// 前後の空白だけが違う軸名は同一の値になる（`stat_alloc` の主キーが分裂しない）。
    #[test]
    fn padding_does_not_create_a_distinct_axis() {
        assert_eq!(parsed(" パッション "), parsed("パッション"));
        assert_ne!(parsed("パッション"), parsed("ロジック"));
    }

    /// 正規化後が空なら拒否する（軸名のない「: 3/5」がプロンプトに載るのを防ぐ）。
    #[test]
    fn rejects_blank() {
        for raw in BLANKS {
            assert_eq!(StatAxis::parse(raw), Err(AxisError::Empty), "raw = {raw:?}");
        }
    }

    /// 制御文字を含むなら拒否する。
    #[test]
    fn rejects_control_characters() {
        for raw in [
            "a\u{0007}b",
            "パッション\u{0000}",
            "a\nb",
            "a\tb",
            "a\u{0085}b",
        ] {
            assert_eq!(
                StatAxis::parse(raw),
                Err(AxisError::Control),
                "raw = {raw:?}"
            );
        }
    }

    /// **行区切りを含むなら拒否する**（作業単位 0009）。
    ///
    /// U+2028 / U+2029 は `char::is_control` が偽（Zl / Zp）なので、
    /// 制御文字の検査だけでは素通りする。
    ///
    /// # 守っているのは「**1行のラベルは1行である**」という衛生条件
    ///
    /// **注入防御ではない。** `prompt::writer::quote_line` は `Seg::Text` を
    /// `to_inline()` で1行に畳むので、`"内なる\u{2028}情熱"` は
    /// `> - 内なる 情熱: 5` になるだけで偽の役割行にはならない。
    /// **この理由を書いておかないと**、後から「`to_inline` があるのだから要らない」
    /// として外される（`0006` の `Mutex` と同じ形）。
    /// 軸名は1行として保存し1行として読むと決めた値なので、
    /// **表示上は2行になる値を入れさせない。**
    ///
    /// 「軸名が消えて `> - : 5` になる」方は `axis_rejects_invisible_characters` の担当。
    ///
    /// **`LINE_BREAKS` の全文字を回す**（固定リストにすると、集合に文字を足した人が
    /// 気付けない）。
    #[test]
    fn axis_rejects_every_line_separator() {
        for separator in LINE_BREAKS {
            assert_eq!(
                StatAxis::parse(&format!("内なる{separator}情熱")),
                Err(AxisError::Control),
                "separator = {separator:?}"
            );
            assert!(
                StatAxis::parse(&separator.to_string()).is_err(),
                "行区切りだけの軸名も作れないこと: separator = {separator:?}"
            );
        }
    }

    /// **何も見えない軸名は `Control` ではなく `Empty`**（作業単位 0009b）。
    ///
    /// 0009a で `is_effectively_blank` を `contains_control_or_line_break` の**前**に
    /// 置いた結果、入力とエラーコードの対応が変わっている
    /// （`stat_axis_control` → `stat_axis_empty`）。**意図した対応**だが、
    /// `axis_rejects_every_line_separator` は単独の場合を `is_err()` としか見ておらず、
    /// **順序を戻す変異が緑を通る。** 詳細は
    /// `domain::character::tests::handle_reports_empty_rather_than_control_when_nothing_is_visible`。
    #[test]
    fn axis_reports_empty_rather_than_control_when_nothing_is_visible() {
        for raw in ["\u{0007}", "\u{0000}", "\u{007F}", "\u{0007}\u{0000}"] {
            assert_eq!(
                StatAxis::parse(raw),
                Err(AxisError::Empty),
                "何も見えない値は「空」として報告すること: raw = {raw:?}"
            );
        }
        for separator in LINE_BREAKS {
            assert_eq!(
                StatAxis::parse(&separator.to_string()),
                Err(AxisError::Empty),
                "行区切りだけの値も「空」として報告すること: separator = {separator:?}"
            );
        }
        for raw in ["\u{0007}情熱", "情熱\u{0007}", "内なる\u{2028}情熱"] {
            assert_eq!(
                StatAxis::parse(raw),
                Err(AxisError::Control),
                "見える文字があるなら制御文字として報告すること: raw = {raw:?}"
            );
        }
    }

    /// **見えない文字だけの軸名は作れない**（作業単位 0009a）。
    ///
    /// レビュアーの実測で `StatAxis::parse("\u{3164}")` すら `Ok` になり、
    /// プロンプトに **`> - ㅤ: 5`**（名前の見えない軸に数値だけ）が載っていた。
    /// 直前の「記載のない軸は考慮しません。」と矛盾し、
    /// **性格値 0 の軸を落としている理由と同じ形の劣化**を起こす。
    ///
    /// > **20b で U+3164 も拒否するようになった**（`domain::text` の
    /// > `MEASURED_INVISIBLE_SPELLINGS`）。`INVISIBLE_SAMPLES` に入っているので、
    /// > **このループが自動的に回している。**
    ///
    /// **それでも「見えない文字を覆い切った」わけではない**
    /// （`is_effectively_blank` の「覆えないもの」）。ここが回すのは覆えた側だけ。
    #[test]
    fn axis_rejects_invisible_characters() {
        for character in INVISIBLE_SAMPLES {
            assert_eq!(
                StatAxis::parse(&character.to_string()),
                Err(AxisError::Empty),
                "見えない文字だけの軸名が通った: U+{:04X}",
                character as u32
            );
            assert_eq!(
                StatAxis::parse(&format!(" {character}\u{3000}{character}\t")),
                Err(AxisError::Empty),
                "trim 対象の空白と混ぜても空: U+{:04X}",
                character as u32
            );
            assert!(
                StatAxis::parse(&format!("内なる{character}情熱")).is_ok(),
                "見える文字があるのに拒否した: U+{:04X}",
                character as u32
            );
        }
    }

    /// **軸名は `@` を含めてよい。** 予約語と衝突するのは `model` に載るキャラクター識別子だけ。
    #[test]
    fn accepts_at_sign() {
        for raw in ["a@b", "@", " 情熱@2 "] {
            assert!(
                StatAxis::parse(raw).is_ok(),
                "軸名の `@` は拒否しないこと: raw = {raw:?}"
            );
        }
        assert_eq!(parsed(" 情熱@2 ").as_str(), "情熱@2");
    }

    /// **貼り付け事故は軸名でも入口で止まる。**
    ///
    /// 軸名は削れない枠に**件数分**載る。1軸が長いほど載せられる軸が減るので、
    /// 無制限だと「軸を1つ足したら以後のチャットが全部 `BudgetExhausted`」が起こりうる。
    #[test]
    fn axis_rejects_a_pasted_document() {
        assert_eq!(
            StatAxis::parse(&"あ".repeat(1_000)),
            Err(AxisError::TooLong)
        );
    }

    /// **上限ちょうどは受理し、1文字超えたら拒否する。**
    #[test]
    fn axis_accepts_the_limit_and_rejects_one_more() {
        let at_limit = "あ".repeat(StatAxis::MAX_CHARS);
        assert_eq!(
            StatAxis::parse(&at_limit).map(|value| value.as_str().chars().count()),
            Ok(StatAxis::MAX_CHARS),
            "上限ちょうどは受理すること"
        );

        assert_eq!(
            StatAxis::parse(&"あ".repeat(StatAxis::MAX_CHARS + 1)),
            Err(AxisError::TooLong),
            "上限を1文字超えたら拒否すること"
        );
    }

    /// 長さは**文字数**で数える（バイト数ではない）。
    #[test]
    fn axis_length_is_counted_in_characters_not_bytes() {
        let japanese = "情".repeat(StatAxis::MAX_CHARS);
        assert!(
            japanese.len() > StatAxis::MAX_CHARS,
            "前提: 日本語はバイト数の方が大きいこと ({} バイト)",
            japanese.len()
        );
        assert!(StatAxis::parse(&japanese).is_ok(), "バイト数で弾かないこと");
        assert!(StatAxis::parse(&"\u{20BB7}".repeat(StatAxis::MAX_CHARS)).is_ok());
        assert_eq!(
            StatAxis::parse(&"🐈".repeat(StatAxis::MAX_CHARS + 1)),
            Err(AxisError::TooLong)
        );
    }

    /// 長さの判定は**正規化の後**（空白を足しただけで弾かれない）。
    #[test]
    fn axis_length_is_checked_after_trimming() {
        let padded = format!("\u{3000} {} ", "情".repeat(StatAxis::MAX_CHARS));
        assert_eq!(
            StatAxis::parse(&padded).map(|value| value.as_str().chars().count()),
            Ok(StatAxis::MAX_CHARS),
            "前後の空白は長さに数えないこと"
        );
    }

    /// 上限は短いラベルとして妥当な範囲に収まっている。
    ///
    /// **下げる方向の変更は保存済みの軸を読めなくする**（`MAX_CHARS` の doc）ので、
    /// 想定から外れたらここで気付けるようにしておく。
    #[test]
    fn axis_limit_stays_in_the_intended_range() {
        assert!(
            (10..=50).contains(&StatAxis::MAX_CHARS),
            "上限は短いラベルに見合う長さであること: {}",
            StatAxis::MAX_CHARS
        );
        // 想定している軸名（サンプルの4軸）は余裕をもって収まること。
        for axis in ["パッション", "ロジック", "共感", "ユーモア", "内なる 情熱"]
        {
            assert!(
                StatAxis::parse(axis).is_ok(),
                "現実の軸名を弾かないこと: {axis}"
            );
        }
    }

    /// **エラーの `Display` に軸名も長さも載せない**（CLAUDE.md 絶対制約）。
    #[test]
    fn axis_error_display_is_a_constant_string() {
        let over_limit = "秘密".repeat(StatAxis::MAX_CHARS);
        let err = StatAxis::parse(&over_limit).expect_err("上限超過であること");
        let message = err.to_string();
        assert!(!message.contains("秘密"), "軸名を含めないこと: {message}");
        assert!(
            !message.contains(&StatAxis::MAX_CHARS.to_string()),
            "実行時の値を埋め込まない定数文字列であること: {message}"
        );
    }

    /// 正規化してから検証する。
    #[test]
    fn normalizes_before_validating() {
        assert_eq!(StatAxis::parse("\t"), Err(AxisError::Empty));
        assert_eq!(StatAxis::parse("\r\n"), Err(AxisError::Empty));
    }

    /// エラーは理由を区別できる（空 / 制御文字 / 長すぎる）。
    #[test]
    fn error_variants_are_distinguishable() {
        let variants = [AxisError::Empty, AxisError::Control, AxisError::TooLong];

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
    }

    /// `Display` は正規化後の値をそのまま返す。
    #[test]
    fn display_matches_as_str() {
        let axis = parsed(" 内なる 情熱 ");
        assert_eq!(axis.to_string(), "内なる 情熱");
        assert_eq!(axis.to_string(), axis.as_str());
    }

    // **`stat_alloc_holds_axis_and_value` はここにあった**（作業単位 0011a-2 で移した）。
    //
    // `value` が `u32` から `StatValue` になり、尺度を通さないと作れなくなったので、
    // 尺度の話をまとめてある `value_tests` の
    // `stat_alloc_holds_an_axis_and_a_scaled_value` が引き継いでいる。
}

#[cfg(test)]
mod scale_tests {
    use super::*;

    /// 下限より小さい値は拒否する。
    ///
    /// **`0` を通すと、性格値の上限が `0` になる**（-2 で値を尺度に従わせたとき）。
    /// 全軸が `value == 0` になり、「値 0 の軸はプロンプトに含めない」（CLAUDE.md 絶対制約）が
    /// 全軸に効いて、**性格ステータスを設定してあるのに一行も載らないキャラクター**ができる。
    #[test]
    fn scale_rejects_anything_below_the_lower_bound() {
        assert_eq!(StatScale::parse(0), Err(ScaleError::TooSmall));
    }

    /// 上限より大きい値は拒否する。
    #[test]
    fn scale_rejects_anything_above_the_upper_bound() {
        for raw in [101, 1_000, u32::MAX] {
            assert_eq!(
                StatScale::parse(raw),
                Err(ScaleError::TooLarge),
                "raw = {raw}"
            );
        }
    }

    /// **両端と既定値は受理する。** 受理した値はそのまま取り出せる。
    ///
    /// 上下限を**数値で写さない**——`parse` の受理集合そのものを見る。
    /// 下限を `0` に広げる変異は上の `scale_rejects_anything_below_the_lower_bound` が、
    /// 上限を広げる変異は `scale_rejects_anything_above_the_upper_bound` が赤にする。
    #[test]
    fn scale_accepts_the_bounds_and_everything_between() {
        for raw in [1, 2, 7, 10, 99, 100] {
            assert_eq!(
                StatScale::parse(raw).map(StatScale::get),
                Ok(raw),
                "raw = {raw}"
            );
        }
    }

    /// **既定は 10。** 理由は ADR-0015（2026-08-19 の追記）にある。**ここには写さない。**
    ///
    /// 既定値が範囲の外にあると、`parse` を通らない値が既定として配られる。
    #[test]
    fn the_default_is_ten_and_parses() {
        assert_eq!(StatScale::DEFAULT.get(), 10);
        assert_eq!(
            StatScale::parse(StatScale::DEFAULT.get()),
            Ok(StatScale::DEFAULT),
            "既定値は `parse` を通ること"
        );
    }

    /// 上下限は互いに矛盾しない（下限 ≤ 既定 ≤ 上限）。
    ///
    /// **「下限が 1 以上であること」はここでは見ない。**
    /// 定数同士の比較になるので clippy が `assertions_on_constants` で止める。
    /// 見張っているのは `scale_rejects_anything_below_the_lower_bound`
    /// （`parse(0)` が拒否される＝下限は 1 以上、という振る舞いの側）。
    #[test]
    fn the_bounds_bracket_the_default() {
        assert!(StatScale::MIN <= StatScale::DEFAULT.get());
        assert!(StatScale::DEFAULT.get() <= StatScale::MAX);
    }

    /// **エラーの `Display` に上下限の数値を埋め込まない**（CLAUDE.md 絶対制約）。
    #[test]
    fn scale_error_display_is_a_constant_string() {
        for err in [ScaleError::TooSmall, ScaleError::TooLarge] {
            let message = err.to_string();
            assert!(
                !message.chars().any(|ch| ch.is_ascii_digit()),
                "実行時の値も上下限も埋め込まない定数文字列であること: {message}"
            );
            assert!(
                !message.contains(&StatScale::MAX.to_string())
                    && !message.contains(&StatScale::MIN.to_string()),
                "上下限を埋め込まないこと: {message}"
            );
        }
    }

    /// エラーは理由を区別できる（小さすぎる / 大きすぎる）。
    ///
    /// 潰すと「送り直すには増やすのか減らすのか」が応答から読めなくなる。
    #[test]
    fn scale_error_variants_are_distinguishable() {
        assert_ne!(ScaleError::TooSmall, ScaleError::TooLarge);
        assert_ne!(
            ScaleError::TooSmall.to_string(),
            ScaleError::TooLarge.to_string(),
            "文言でも区別できること"
        );
    }

    /// **`parse` 以外のコンストラクタが無い**ことの手掛かり。
    ///
    /// `From<u32>` も `pub fn new` も生やさない（`Handle` / `StatAxis` と同じ）。
    /// **型で守られているわけではない**——生やせば通る。ここは「生やしたら
    /// この doc と衝突する」ことを読み手に伝えるための場所であって、見張り番ではない。
    #[test]
    fn a_scale_round_trips_through_parse_only() {
        let scale = StatScale::parse(7).expect("正当な尺度であること");
        assert_eq!(scale.get(), 7);
        assert_eq!(StatScale::parse(scale.get()), Ok(scale));
    }
}

#[cfg(test)]
mod value_tests {
    use super::*;

    fn scale(raw: u32) -> StatScale {
        StatScale::parse(raw).expect("正当な尺度であること")
    }

    /// **尺度ちょうどは通り、1つ超えたら通らない。`0` は通る。**
    ///
    /// `0` を弾くと「値 0 の軸はプロンプトに含めない」（CLAUDE.md 絶対制約）を
    /// 実装する対象そのものが作れなくなる。落とすのは `prompt` 層の責務。
    #[test]
    fn a_value_is_accepted_up_to_the_scale_and_refused_beyond_it() {
        for raw_scale in [1, 3, 7, 10, StatScale::MAX] {
            let scale = scale(raw_scale);
            assert_eq!(
                scale.value(0).map(StatValue::get),
                Ok(0),
                "0 は正当な値であること: 尺度 = {raw_scale}"
            );
            assert_eq!(
                scale.value(raw_scale).map(StatValue::get),
                Ok(raw_scale),
                "尺度ちょうどは受理すること: 尺度 = {raw_scale}"
            );
            assert_eq!(
                scale.value(raw_scale + 1),
                Err(ValueError::AboveScale),
                "尺度を1つ超えたら拒否すること: 尺度 = {raw_scale}"
            );
            assert_eq!(
                scale.value(u32::MAX),
                Err(ValueError::AboveScale),
                "尺度 = {raw_scale}"
            );
        }
    }

    /// **値は検証に使った尺度を覚えている。**
    ///
    /// `3/100` と `3/10` は**別の性格**なので、数だけを持ち回ると
    /// 「どちらの尺度で決めた値か」が復元できない。
    #[test]
    fn a_value_remembers_the_scale_it_was_checked_against() {
        let small = scale(10).value(3).expect("尺度 10 の 3 は正当であること");
        let large = scale(100).value(3).expect("尺度 100 の 3 は正当であること");

        assert_eq!(small.get(), large.get(), "前提: 数としては同じであること");
        assert_eq!(small.scale(), scale(10));
        assert_eq!(large.scale(), scale(100));
        assert_ne!(small, large, "尺度が違えば別の値であること");
    }

    /// **`StatScale` 以外に構築の入口が無い**ことの手掛かり。
    ///
    /// `From<u32>` も `pub fn new` も生やさない（`Handle` / `StatAxis` / `StatScale` と同じ）。
    /// **型で守られているわけではない**——生やせば通る。ここは「生やしたら
    /// この doc と衝突する」ことを読み手に伝えるための場所。
    #[test]
    fn a_value_round_trips_through_its_scale_only() {
        let scale = scale(42);
        let value = scale.value(7).expect("正当な値であること");
        assert_eq!(value.get(), 7);
        assert_eq!(value.scale(), scale);
        assert_eq!(scale.value(value.get()), Ok(value));
    }

    /// **エラーの `Display` に値も尺度も埋め込まない**（CLAUDE.md 絶対制約）。
    #[test]
    fn value_error_display_is_a_constant_string() {
        let err = scale(10).value(4_294_967_295).expect_err("超過であること");
        let message = err.to_string();
        assert!(
            !message.chars().any(|ch| ch.is_ascii_digit()),
            "実行時の値も尺度も埋め込まない定数文字列であること: {message}"
        );
    }

    /// `StatAlloc` は軸と値の対。**値 0 も正当な配分**（落とすのは `prompt` 層の責務）。
    #[test]
    fn stat_alloc_holds_an_axis_and_a_scaled_value() {
        let scale = scale(5);
        let alloc = StatAlloc {
            axis: StatAxis::parse("パッション").expect("正当な軸名であること"),
            value: scale.value(0).expect("0 は正当な値であること"),
        };
        assert_eq!(alloc.axis.as_str(), "パッション");
        assert_eq!(alloc.value.get(), 0);
        assert_eq!(alloc.value.scale(), scale);
    }

    /// **プロンプトに載るかの述語は「値が 0 でない」**（CLAUDE.md 絶対制約）。
    ///
    /// `prompt` の `filter` と `parse::leak` の判定が**同じ関数**を通す根拠。
    /// 尺度の上端でも下端の 1 でも真、0 だけが偽。
    #[test]
    fn only_a_non_zero_alloc_appears_in_the_prompt() {
        let scale = scale(5);
        let axis = StatAxis::parse("パッション").expect("正当な軸名であること");
        let with = |raw: u32| StatAlloc {
            axis: axis.clone(),
            value: scale.value(raw).expect("尺度に収まる値であること"),
        };

        assert!(
            !with(0).appears_in_prompt(),
            "値 0 の軸はプロンプトに載せない（絶対制約）"
        );
        assert!(with(1).appears_in_prompt(), "1 は載る（0 だけが例外）");
        assert!(with(5).appears_in_prompt(), "尺度の上端も載る");
    }
}
