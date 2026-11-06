//! キャラクターの不変属性（キー付き）のドメイン型。ADR-0009 / 作業単位 12a。
//!
//! 「一人称: 僕」「種族: 猫獣人」のような**変わりようのない要素**を表す。
//! 動機（同じキャラクター定義から一人称が 4 通り出た実測）は ADR-0009 にある。
//! **ここには写さない**（写すと測り直したときにこちらだけ黙って古びる）。

use std::fmt;

use super::Subject;
use super::text::{contains_control_or_line_break, is_effectively_blank, trim_blank};

/// 検証・正規化を通った属性のキー。
///
/// `Handle` / `StatAxis` と同じく **`parse` が唯一のコンストラクタ**。
/// キーの語彙は固定しない（作品ごとに要る属性が違う / ADR-0009）ので、
/// 検証を型に載せないと空文字のキーがそのままプロンプトに「: 猫獣人」として載る。
///
/// # **`StatAxis` とは別の型**
///
/// どちらも「短い1行のラベル」で中身は正規化済みの文字列だが、上限が違う
/// （軸名 20 文字 / キー 12 文字）うえ、**取り違えると
/// 「性格ステータスの軸を属性として保存する」類の事故が静かに通る。**
/// `Handle` と `StatAxis` を分けているのとまったく同じ理由。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttributeKey(String);

impl AttributeKey {
    /// キーの上限。**文字数**（Unicode スカラー値の個数）で数える。
    ///
    /// # 数字の出どころ
    ///
    /// **[ADR-0009](../../docs/adr/0009-character-attributes-and-facts.md) の
    /// 「上限の数字と、その出どころ」節。ここには写さない。**
    ///
    /// バイト数で数えないのは `StatAxis::MAX_CHARS` と同じ理由
    /// （UTF-8 では日本語1文字が3バイトで、日本語だけが 1/3 しか書けなくなる）。
    ///
    /// # **この値は後から下げられない**
    ///
    /// 書き込みの検証と**読み出しの検証**（`Store::character_attributes` →
    /// `StoreError::InvalidStoredValue`）の両方に使う。下げると保存済みの長いキーが
    /// そのキャラクターの属性取得を丸ごと止める。
    /// **復旧手段はある**——置き換え（`Store::replace_character_attributes` =
    /// `PUT /admin/characters/{handle}/attributes`）は既存の行を読まないので届く
    /// （見張り番は `tests/store_attribute.rs::
    /// an_over_long_stored_key_stops_the_read_and_the_replacement_fixes_it`）。
    /// それでも**下げてよい理由にはならない**（気付くのは「読めなくなってから」で、
    /// 気付ける人はエラーの意味を知っている人だけ）。上げる方向は安全。
    ///
    /// スキーマ側にも同じ数の CHECK がある（`migrations/0009_character_attribute.sql`）。
    /// **2つは独立した定数**で、揃っていることは
    /// `tests/store_schema.rs::attribute_limits_match_the_schema_check` が見張る。
    pub const MAX_CHARS: usize = 12;

    /// 唯一のコンストラクタ。正規化してから検証する。**手順は `StatAxis::parse` と同じ。**
    ///
    /// 1. 前後の空白（`text::TRIMMABLE`）を除去する。中間の空白は保持する。
    /// 2. 空になったら拒否する。
    /// 3. 見える文字が1つも残らないなら拒否する。
    /// 4. 制御文字・行区切りを含むなら拒否する。
    /// 5. `MAX_CHARS` を超えるなら拒否する。
    ///
    /// **`@` は禁止しない**（`StatAxis` と同じ。予約語と衝突するのは `model` に載る
    /// キャラクター識別子だけで、キーは `model` に現れない）。
    pub fn parse(raw: &str) -> Result<Self, AttributeKeyError> {
        let normalized = trim_blank(raw);

        if normalized.is_empty() || is_effectively_blank(normalized) {
            return Err(AttributeKeyError::Empty);
        }
        if contains_control_or_line_break(normalized) {
            return Err(AttributeKeyError::Control);
        }
        // **長さの判定は正規化の後**（前に置くと、空白を足しただけで上限ちょうどが弾かれる）。
        if normalized.chars().count() > Self::MAX_CHARS {
            return Err(AttributeKeyError::TooLong);
        }

        Ok(Self(normalized.to_owned()))
    }

    /// 正規化後の文字列。SQL へのバインド値はこれを使う。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AttributeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// キーが受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttributeKeyError {
    /// 正規化すると空になる（空文字・空白のみ・見えない文字のみ）。
    #[error("属性のキーが空です")]
    Empty,

    /// 制御文字・行区切りを含む。
    #[error("属性のキーに改行や制御文字は使えません")]
    Control,

    /// 正規化後の文字数が `AttributeKey::MAX_CHARS` を超える。
    ///
    /// **上限値をメッセージに埋め込まない。** `Display` は定数文字列に保つ。
    #[error("属性のキーが長すぎます")]
    TooLong,
}

/// 検証・正規化を通った属性の値。
///
/// **`parse` が唯一のコンストラクタ。** `AttributeKey` と**別の型**にしてあるのは、
/// 上限が違う（12 / 20）ことと、`CharacterAttribute` の 2 フィールドが
/// **同じ型だと入れ替えても型検査を素通りする**ため。
///
/// # 空文字を拒否する
///
/// ADR-0009 が「空の値を持つ属性は行ごと落とす」と決めているが、
/// **`store` へ届く前にここで止める。**
/// 「性格値 0 の軸」と同じ扱いにはしない——あちらは `0` が**正当な保存値**で、
/// 落とすのは `prompt` 層の責務だが、**空文字はそもそも `parse` を通らない。**
/// （`種族: ` と書くとモデルが欠落を演技する。ADR-0009）
///
/// # 制御文字を拒否する
///
/// `MemoryContent`（自由文・複数行を許す）とは非対称。属性の値は
/// **`猫獣人` / `僕` / `19` のような1行の語**で、削れない枠に1行として載る。
/// 表示上2行になる値を入れさせない（`StatAxis` の行区切り拒否と同じ衛生条件）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AttributeValue(String);

impl AttributeValue {
    /// 値の上限。**文字数**（Unicode スカラー値の個数）で数える。
    ///
    /// 数字の出どころ・下げられない理由・スキーマ側の写しについては
    /// **`AttributeKey::MAX_CHARS` の doc に一本化してある。**
    /// キーより長いのは、値の方が語として長くなりうるため（`猫獣人` / `幼馴染の妹` など）。
    pub const MAX_CHARS: usize = 20;

    /// 唯一のコンストラクタ。手順は `AttributeKey::parse` と同じ。
    pub fn parse(raw: &str) -> Result<Self, AttributeValueError> {
        let normalized = trim_blank(raw);

        if normalized.is_empty() || is_effectively_blank(normalized) {
            return Err(AttributeValueError::Empty);
        }
        if contains_control_or_line_break(normalized) {
            return Err(AttributeValueError::Control);
        }
        if normalized.chars().count() > Self::MAX_CHARS {
            return Err(AttributeValueError::TooLong);
        }

        Ok(Self(normalized.to_owned()))
    }

    /// 正規化後の文字列。SQL へのバインド値はこれを使う。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AttributeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 値が受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
///
/// **`AttributeKeyError` と別の型にしてある。** 「キーが長すぎる」と
/// 「値が長すぎる」を同じ応答に潰すと、受け取った側はどちらの欄を縮めればよいか分からない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttributeValueError {
    /// 正規化すると空になる（空文字・空白のみ・見えない文字のみ）。
    #[error("属性の値が空です")]
    Empty,

    /// 制御文字・行区切りを含む。
    #[error("属性の値に改行や制御文字は使えません")]
    Control,

    /// 正規化後の文字数が `AttributeValue::MAX_CHARS` を超える。
    #[error("属性の値が長すぎます")]
    TooLong,
}

/// 1件分のキャラクター属性。
///
/// **`subject` を省略しない**（`memory` と同じ理由 / ADR-0009）。
/// 「年齢: 19」がキャラのものか利用者のものかを落とすと、モデルは取り違える。
/// `Option` にもしない——省略できる形にした時点で、省略した行が必ず生まれる。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacterAttribute {
    /// 誰についての属性か。**`memory.subject` と同じ2値。**
    pub subject: Subject,
    /// 属性のキー（`一人称` / `種族`）。
    pub key: AttributeKey,
    /// 属性の値（`僕` / `猫獣人`）。
    pub value: AttributeValue,
}

/// **件数の上限を通った属性の集合**（ADR-0009 / 作業単位 12a）。
///
/// # なぜ「型」にしたのか —— 件数の上限をどこで拒否するか
///
/// ADR-0009 は「**管理 API 側で拒否する**」と書いている。素直に読むと
/// `server::admin` の中で `if request.attributes.len() > 8 { … }` と書く形になるが、
/// **それだと `store` の公開 API が 9 件を書ける形のまま残る。**
/// `store` のモジュール doc は「公開 API は検証済みのドメイン型を受け取る。
/// `&str` や裸の `i64` を受ける入口を作らない」と宣言しており、
/// **裸の `&[CharacterAttribute]` はその宣言の外側にある。**
///
/// そこで件数の検査を**型**に載せ、`Store::replace_character_attributes` が
/// これしか受け取らない形にした。結果として
/// **9 件を保存する経路がコンパイル時に存在しない**（`parse` 以外の
/// コンストラクタを生やしていないので、`AttributeSet` は検査を通ってしか作れない）。
/// 拒否そのものは今も管理 API の中——`PUT` のハンドラが `parse` を呼んで 400 を返す——で
/// 起きるので、ADR の決定は変えていない。
///
/// > **`ValidTag`（ADR-0006）と同じ強さだと読まないこと。**
/// > 型が守るのは「**この集合を作るときに 9 件を数えなかった**」までで、
/// > `parse` を 2 回呼んで `replace_character_attributes` を 2 回叩けば
/// > **DB には 8 件しか残らない**（丸ごと置き換えなので上書きになる）。
/// > つまり守れているのは「1 回の置き換えで 9 件入らない」こと。
/// > **DB を直接編集して 9 行入れることは今も可能**で、それは読み出しでも弾かない
/// > （`character_attributes` は `Vec` を返す。理由はそちらの doc）。
///
/// # **12b への申し送り —— 予算は `MAX_LEN` から見積もる**
///
/// 属性をプロンプトの「削れない枠」に載せるとき、枠の大きさは
/// **保存済みの行数からではなく `MAX_LEN` から**見積もること。
/// 上のとおり **DB を直接編集して 9 行入った状態は読み出しで弾かれない**ので、
/// 行数を信じると枠が1行分溢れる。
///
/// # 重複は見ない
///
/// 同じ `(subject, key)` を 2 つ渡した場合はここでは弾かず、
/// スキーマの主キー違反として `store` が返す（`stat_alloc` の軸と同じ扱い）。
/// **正規化後に同じになる場合も含めて、判定を DB の1箇所に寄せてある。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeSet(Vec<CharacterAttribute>);

impl AttributeSet {
    /// 1キャラクターが持てる属性の件数の上限。
    ///
    /// 数字の出どころは **[ADR-0009](../../docs/adr/0009-character-attributes-and-facts.md)
    /// の「上限の数字と、その出どころ」節。ここには写さない。**
    ///
    /// 件数にも上限が要るのは、属性が**プロンプトの「削れない枠」に載る**うえ、
    /// `stat_alloc` のように件数が無制限だと「属性を1つ足したら以後のチャットが全部
    /// `BudgetExhausted`」が原理的に起こりうるため（ADR-0009「予算」節）。
    pub const MAX_LEN: usize = 8;

    /// 唯一のコンストラクタ。件数が `MAX_LEN` を超えるなら拒否する。
    ///
    /// **空の集合は正当**（全属性を消す置き換えに要る。`replace_stat_allocs` と同じ）。
    pub fn parse(attributes: Vec<CharacterAttribute>) -> Result<Self, AttributeSetError> {
        if attributes.len() > Self::MAX_LEN {
            return Err(AttributeSetError::TooMany);
        }
        Ok(Self(attributes))
    }

    /// 渡された並びのまま参照する。**並べ替えない**
    /// （`store` は挿入順で読み戻すので、呼び出し側が順序を決められる）。
    #[must_use]
    pub fn as_slice(&self) -> &[CharacterAttribute] {
        &self.0
    }

    /// 件数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// 空か。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// **置き換えで起きる属性1件ぶんの変更**（[ADR-0028](../../docs/adr/0028-attributes-that-change.md)
/// §決定5 / 作業単位 A-3a）。
///
/// 履歴の表（`store::attribute`）に1行として追記される形そのもの。
///
/// # `before` / `after` が `Option` である意味
///
/// | | `before` | `after` |
/// |---|---|---|
/// | 追加 | `None`（**無かった**） | `Some` |
/// | 変更 | `Some` | `Some` |
/// | 削除 | `Some` | `None`（**消えた**） |
///
/// **両方が `None` になる変更は作らない**（[`attribute_changes`] が作らない）。
/// **両方が同じ値になる変更も作らない**——変わっていないものは記録しない（§決定5 の
/// 「静かに変わると気付く手段が無い」に応えるための表なので、変わっていない行を
/// 混ぜると、変わった行がその中に埋もれる）。
///
/// # `subject` を持つ
///
/// **モデルにも人にも主語を言わせない**（§決定4）。ここに入る主語は、
/// 現在の集合または新しい集合の**その属性自身の主語**であり、
/// [`attribute_changes`] は主語をまたいで突き合わせない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeChange {
    /// 誰についての属性か。**落とさない**（CLAUDE.md 絶対制約と同じ理由）。
    pub subject: Subject,
    /// どのキーの変更か。
    pub key: AttributeKey,
    /// 変更前の値。`None` は**その主語のそのキーが無かった**こと。
    pub before: Option<AttributeValue>,
    /// 変更後の値。`None` は**消えた**こと。
    pub after: Option<AttributeValue>,
}

/// **現在の集合と新しい集合の差分**を、履歴に書く順で返す（§決定5）。**純粋関数。**
///
/// # 突き合わせるのは `(subject, key)` の組
///
/// キーだけで突き合わせると、`年齢` を利用者とキャラクターの両方が持つときに
/// **持ち主をまたいだ差分**が出る（ADR-0021 が 6% 実測した主語の取り違えを、
/// 履歴の側で作り込む形になる）。見張り番は
/// `the_same_key_under_two_subjects_never_swaps_owners`。
///
/// # 並び
///
/// **新しい集合の順（変更・追加）→ 現在の集合の順（削除）。**
/// 履歴は追記なので、**この順がそのまま同着の並び**になる（`id` の昇順）。
/// 決めておかないと、同じ置き換えが回ごとに違う並びで残る。
///
/// # 重複は判定しない
///
/// `(subject, key)` が新しい集合に2回現れたら、変更も2件出る。
/// **判定は DB の主キー**（[`AttributeSet`] の doc）で、そこで置き換え全体が落ちる。
#[must_use]
pub fn attribute_changes(
    current: &[CharacterAttribute],
    next: &[CharacterAttribute],
) -> Vec<AttributeChange> {
    let mut changes = Vec::new();

    for attribute in next {
        let stored = current
            .iter()
            .find(|held| held.subject == attribute.subject && held.key == attribute.key);
        match stored {
            // 値まで同じなら、記録するものが無い。
            Some(held) if held.value == attribute.value => {}
            Some(held) => changes.push(AttributeChange {
                subject: attribute.subject,
                key: attribute.key.clone(),
                before: Some(held.value.clone()),
                after: Some(attribute.value.clone()),
            }),
            None => changes.push(AttributeChange {
                subject: attribute.subject,
                key: attribute.key.clone(),
                before: None,
                after: Some(attribute.value.clone()),
            }),
        }
    }

    for held in current {
        let survives = next
            .iter()
            .any(|attribute| attribute.subject == held.subject && attribute.key == held.key);
        if survives {
            continue;
        }
        changes.push(AttributeChange {
            subject: held.subject,
            key: held.key.clone(),
            before: Some(held.value.clone()),
            after: None,
        });
    }

    changes
}

/// 集合が受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
///
/// **件数をメッセージに埋め込まない**（`AttributeKeyError::TooLong` と同じ扱い）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttributeSetError {
    /// `AttributeSet::MAX_LEN` を超える件数を渡された。
    #[error("属性の件数が多すぎます")]
    TooMany,
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use crate::domain::LINE_BREAKS;
    use crate::domain::text::INVISIBLE_SAMPLES;

    /// 正規化すると空になる文字列。`StatAxis` と同じ集合。
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

    fn parsed(raw: &str) -> AttributeKey {
        AttributeKey::parse(raw).expect("正当なキーとして解釈できること")
    }

    /// 前後の空白は除去し、中間の空白は保持する。
    #[test]
    fn key_trims_the_edges_and_keeps_the_middle() {
        for (raw, expected) in [
            (" 一人称 ", "一人称"),
            ("\u{3000}種族\u{3000}", "種族"),
            ("\tgender\n", "gender"),
            ("好きな 食べ物", "好きな 食べ物"),
        ] {
            assert_eq!(parsed(raw).as_str(), expected, "raw = {raw:?}");
        }
    }

    /// 前後の空白だけが違うキーは同一の値になる（主キーが分裂しない）。
    #[test]
    fn key_padding_does_not_create_a_distinct_key() {
        assert_eq!(parsed(" 一人称 "), parsed("一人称"));
        assert_ne!(parsed("一人称"), parsed("種族"));
    }

    /// 正規化後が空なら拒否する（キーのない「: 猫獣人」がプロンプトに載るのを防ぐ）。
    #[test]
    fn key_rejects_blank() {
        for raw in BLANKS {
            assert_eq!(
                AttributeKey::parse(raw),
                Err(AttributeKeyError::Empty),
                "raw = {raw:?}"
            );
        }
    }

    /// 見えない文字だけのキーも「空」として拒否する（`StatAxis` の 0009a と同じ穴）。
    #[test]
    fn key_rejects_invisible_characters() {
        for character in INVISIBLE_SAMPLES {
            assert_eq!(
                AttributeKey::parse(&character.to_string()),
                Err(AttributeKeyError::Empty),
                "見えない文字だけのキーが通った: U+{:04X}",
                character as u32
            );
            assert!(
                AttributeKey::parse(&format!("一{character}人称")).is_ok(),
                "見える文字があるのに拒否した: U+{:04X}",
                character as u32
            );
        }
    }

    /// 制御文字・行区切りを含むなら拒否する。**行区切りは全文字を回す。**
    #[test]
    fn key_rejects_control_characters_and_line_separators() {
        for raw in ["a\u{0007}b", "一人称\u{0000}", "a\nb", "a\tb", "a\u{0085}b"] {
            assert_eq!(
                AttributeKey::parse(raw),
                Err(AttributeKeyError::Control),
                "raw = {raw:?}"
            );
        }
        for separator in LINE_BREAKS {
            assert_eq!(
                AttributeKey::parse(&format!("一人{separator}称")),
                Err(AttributeKeyError::Control),
                "separator = {separator:?}"
            );
            assert_eq!(
                AttributeKey::parse(&separator.to_string()),
                Err(AttributeKeyError::Empty),
                "行区切りだけの値は「空」として報告すること: separator = {separator:?}"
            );
        }
    }

    /// **上限ちょうどは受理し、1文字超えたら拒否する。**
    #[test]
    fn key_accepts_the_limit_and_rejects_one_more() {
        let at_limit = "あ".repeat(AttributeKey::MAX_CHARS);
        assert_eq!(
            AttributeKey::parse(&at_limit).map(|key| key.as_str().chars().count()),
            Ok(AttributeKey::MAX_CHARS),
            "上限ちょうどは受理すること"
        );
        assert_eq!(
            AttributeKey::parse(&"あ".repeat(AttributeKey::MAX_CHARS + 1)),
            Err(AttributeKeyError::TooLong),
            "上限を1文字超えたら拒否すること"
        );
    }

    /// 長さは**文字数**で数える（バイト数ではない）。
    #[test]
    fn key_length_is_counted_in_characters_not_bytes() {
        let japanese = "称".repeat(AttributeKey::MAX_CHARS);
        assert!(
            japanese.len() > AttributeKey::MAX_CHARS,
            "前提: 日本語はバイト数の方が大きいこと ({} バイト)",
            japanese.len()
        );
        assert!(AttributeKey::parse(&japanese).is_ok(), "バイト数で弾かない");
        assert!(AttributeKey::parse(&"🐈".repeat(AttributeKey::MAX_CHARS)).is_ok());
        assert_eq!(
            AttributeKey::parse(&"🐈".repeat(AttributeKey::MAX_CHARS + 1)),
            Err(AttributeKeyError::TooLong)
        );
    }

    /// 長さの判定は**正規化の後**（空白を足しただけで弾かれない）。
    #[test]
    fn key_length_is_checked_after_trimming() {
        let padded = format!("\u{3000} {} ", "称".repeat(AttributeKey::MAX_CHARS));
        assert_eq!(
            AttributeKey::parse(&padded).map(|key| key.as_str().chars().count()),
            Ok(AttributeKey::MAX_CHARS),
            "前後の空白は長さに数えないこと"
        );
    }

    /// **想定しているキーは余裕をもって収まる。**
    ///
    /// 上限を下げる方向の変更は保存済みの行を読めなくするので、
    /// 想定から外れたらここで気付けるようにしておく。
    #[test]
    fn key_limit_fits_the_attributes_the_adr_names() {
        for key in ["一人称", "種族", "年齢", "性別", "身長", "出身", "所属"] {
            assert!(
                AttributeKey::parse(key).is_ok(),
                "現実のキーを弾かないこと: {key}"
            );
        }
    }

    /// **エラーの `Display` にキーも長さも載せない**（CLAUDE.md 絶対制約）。
    #[test]
    fn key_error_display_is_a_constant_string() {
        let over_limit = "秘密".repeat(AttributeKey::MAX_CHARS);
        let err = AttributeKey::parse(&over_limit).expect_err("上限超過であること");
        let message = err.to_string();
        assert!(!message.contains("秘密"), "キーを含めないこと: {message}");
        assert!(
            !message.chars().any(|ch| ch.is_ascii_digit()),
            "実行時の値も上限も埋め込まない定数文字列であること: {message}"
        );
    }

    /// エラーは理由を区別できる（空 / 制御文字 / 長すぎる）。
    #[test]
    fn key_error_variants_are_distinguishable() {
        let variants = [
            AttributeKeyError::Empty,
            AttributeKeyError::Control,
            AttributeKeyError::TooLong,
        ];
        for (index, left) in variants.iter().enumerate() {
            assert!(!left.to_string().is_empty());
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
    fn key_display_matches_as_str() {
        let key = parsed(" 好きな 食べ物 ");
        assert_eq!(key.to_string(), "好きな 食べ物");
        assert_eq!(key.to_string(), key.as_str());
    }
}

#[cfg(test)]
mod value_tests {
    use super::*;
    use crate::domain::LINE_BREAKS;
    use crate::domain::text::INVISIBLE_SAMPLES;

    fn parsed(raw: &str) -> AttributeValue {
        AttributeValue::parse(raw).expect("正当な値として解釈できること")
    }

    /// 前後の空白は除去し、中間の空白は保持する。
    #[test]
    fn value_trims_the_edges_and_keeps_the_middle() {
        assert_eq!(parsed(" 猫獣人 ").as_str(), "猫獣人");
        assert_eq!(parsed("\u{3000}僕\u{3000}").as_str(), "僕");
        assert_eq!(parsed("幼馴染 の妹").as_str(), "幼馴染 の妹");
    }

    /// **空の値は保存できない**（ADR-0009。`種族: ` はモデルが欠落を演技する）。
    ///
    /// **「性格値 0 の軸」と同じ扱いにしないこと。** あちらは `0` が正当な保存値で
    /// 落とすのは `prompt` の責務だが、空文字は `parse` を通らない。
    #[test]
    fn value_rejects_blank() {
        for raw in ["", " ", "\t", "\u{3000}", "\u{00A0}", " \u{3000}\t\r\n"] {
            assert_eq!(
                AttributeValue::parse(raw),
                Err(AttributeValueError::Empty),
                "raw = {raw:?}"
            );
        }
    }

    /// 見えない文字だけの値も「空」として拒否する（`StatAxis` の 0009a と同じ穴）。
    ///
    /// **キー側（`key_rejects_invisible_characters`）の写しではなく、値側の見張り番。**
    /// ここが抜けると `AttributeValue::parse("\u{2000}")`（EN QUAD）が通り、
    /// **`> - 一人称: ` が削れない枠を食って中身の見えない行**として載る
    /// （`StatAxis` で実測された `> - : 5` と同じ形）。
    ///
    /// **`is_effectively_blank` が覆えない文字は今も通る**
    /// （U+FFA0 HALFWIDTH HANGUL FILLER など。`domain::text::is_effectively_blank` の
    /// 「覆えないもの」）。ここが固定するのは**覆えている範囲**だけ。
    ///
    /// > **U+3164 HANGUL FILLER は 20b で覆えた側へ移った**
    /// > （`domain::text::MEASURED_INVISIBLE_SPELLINGS`）。
    /// > `INVISIBLE_SAMPLES` に入っているので、このループが自動的に回している。
    #[test]
    fn value_rejects_invisible_characters() {
        for character in INVISIBLE_SAMPLES {
            assert_eq!(
                AttributeValue::parse(&character.to_string()),
                Err(AttributeValueError::Empty),
                "見えない文字だけの値が通った: U+{:04X}",
                character as u32
            );
            assert!(
                AttributeValue::parse(&format!("猫{character}獣人")).is_ok(),
                "見える文字があるのに拒否した: U+{:04X}",
                character as u32
            );
        }
    }

    /// 制御文字・行区切りを含むなら拒否する。
    #[test]
    fn value_rejects_control_characters_and_line_separators() {
        for raw in ["猫\u{0007}獣人", "僕\u{0000}", "a\nb"] {
            assert_eq!(
                AttributeValue::parse(raw),
                Err(AttributeValueError::Control),
                "raw = {raw:?}"
            );
        }
        for separator in LINE_BREAKS {
            assert_eq!(
                AttributeValue::parse(&format!("猫{separator}獣人")),
                Err(AttributeValueError::Control),
                "separator = {separator:?}"
            );
        }
    }

    /// **上限ちょうどは受理し、1文字超えたら拒否する。**
    #[test]
    fn value_accepts_the_limit_and_rejects_one_more() {
        let at_limit = "あ".repeat(AttributeValue::MAX_CHARS);
        assert_eq!(
            AttributeValue::parse(&at_limit).map(|value| value.as_str().chars().count()),
            Ok(AttributeValue::MAX_CHARS),
            "上限ちょうどは受理すること"
        );
        assert_eq!(
            AttributeValue::parse(&"あ".repeat(AttributeValue::MAX_CHARS + 1)),
            Err(AttributeValueError::TooLong),
            "上限を1文字超えたら拒否すること"
        );
    }

    /// **キーと値の上限は別**（同じ数にすると取り違えが検出できない）。
    ///
    /// 片方の定数をもう片方で使う変異——たとえば `AttributeValue::parse` が
    /// `AttributeKey::MAX_CHARS` を見る——は、2 つが同値だと緑を通る。
    #[test]
    fn the_two_limits_are_not_the_same_number() {
        assert_ne!(
            AttributeKey::MAX_CHARS,
            AttributeValue::MAX_CHARS,
            "キーと値の上限を同値にしないこと（取り違えが検出できなくなる）"
        );
        // キーの上限を超え、値の上限には収まる長さ。**どちらの型を通したかで答えが割れる。**
        let between = "あ".repeat(AttributeKey::MAX_CHARS + 1);
        assert_eq!(
            AttributeKey::parse(&between),
            Err(AttributeKeyError::TooLong)
        );
        assert!(
            AttributeValue::parse(&between).is_ok(),
            "値としては通る長さであること"
        );
    }

    /// 長さは**文字数**で数える（バイト数ではない）。
    #[test]
    fn value_length_is_counted_in_characters_not_bytes() {
        let japanese = "獣".repeat(AttributeValue::MAX_CHARS);
        assert!(japanese.len() > AttributeValue::MAX_CHARS, "前提");
        assert!(AttributeValue::parse(&japanese).is_ok());
        assert_eq!(
            AttributeValue::parse(&"🐈".repeat(AttributeValue::MAX_CHARS + 1)),
            Err(AttributeValueError::TooLong)
        );
    }

    /// **エラーの `Display` に値も上限も載せない**（CLAUDE.md 絶対制約）。
    #[test]
    fn value_error_display_is_a_constant_string() {
        let err = AttributeValue::parse(&"秘密".repeat(AttributeValue::MAX_CHARS))
            .expect_err("上限超過であること");
        let message = err.to_string();
        assert!(!message.contains("秘密"), "値を含めないこと: {message}");
        assert!(
            !message.chars().any(|ch| ch.is_ascii_digit()),
            "定数文字列であること: {message}"
        );
    }

    /// **キーのエラーと値のエラーは別物**（どちらの欄を直すのかが分かる）。
    #[test]
    fn value_errors_do_not_read_the_same_as_key_errors() {
        for (key_error, value_error) in [
            (AttributeKeyError::Empty, AttributeValueError::Empty),
            (AttributeKeyError::Control, AttributeValueError::Control),
            (AttributeKeyError::TooLong, AttributeValueError::TooLong),
        ] {
            assert_ne!(
                key_error.to_string(),
                value_error.to_string(),
                "キーと値で別の文言であること"
            );
        }
    }

    /// エラーは理由を区別できる。
    #[test]
    fn value_error_variants_are_distinguishable() {
        let variants = [
            AttributeValueError::Empty,
            AttributeValueError::Control,
            AttributeValueError::TooLong,
        ];
        for (index, left) in variants.iter().enumerate() {
            for right in &variants[index + 1..] {
                assert_ne!(left, right);
                assert_ne!(left.to_string(), right.to_string());
            }
        }
    }
}

#[cfg(test)]
mod set_tests {
    use super::*;

    fn attribute(subject: Subject, key: &str, value: &str) -> CharacterAttribute {
        CharacterAttribute {
            subject,
            key: AttributeKey::parse(key).expect("正当なキーであること"),
            value: AttributeValue::parse(value).expect("正当な値であること"),
        }
    }

    /// `n` 件の属性。**キーは全部違う値**にする（重複違反と件数超過を混ぜないため）。
    fn many(count: usize) -> Vec<CharacterAttribute> {
        (0..count)
            .map(|index| {
                attribute(
                    Subject::Character,
                    &format!("キー{index}"),
                    &format!("値{index}"),
                )
            })
            .collect()
    }

    /// **上限ちょうどは通り、1件超えたら通らない。**
    #[test]
    fn a_set_accepts_the_limit_and_rejects_one_more() {
        let at_limit =
            AttributeSet::parse(many(AttributeSet::MAX_LEN)).expect("上限ちょうどは受理すること");
        assert_eq!(at_limit.len(), AttributeSet::MAX_LEN);

        assert_eq!(
            AttributeSet::parse(many(AttributeSet::MAX_LEN + 1)),
            Err(AttributeSetError::TooMany),
            "上限を1件超えたら拒否すること"
        );
        assert_eq!(
            AttributeSet::parse(many(100)),
            Err(AttributeSetError::TooMany)
        );
    }

    /// **空の集合は正当**（全属性を消す置き換えに要る）。
    #[test]
    fn an_empty_set_is_valid() {
        let set = AttributeSet::parse(Vec::new()).expect("空も受理すること");
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
        assert_eq!(set.as_slice(), &[]);
    }

    /// **渡した並びのまま保持する。** 並べ替えない・重複を落とさない。
    ///
    /// 並べ替えると、`store` が挿入順で読み戻す意味が消える。
    #[test]
    fn a_set_keeps_the_order_it_was_given() {
        let given = vec![
            attribute(Subject::Character, "種族", "猫獣人"),
            attribute(Subject::User, "呼び方", "先輩"),
            attribute(Subject::Character, "一人称", "僕"),
        ];
        let set = AttributeSet::parse(given.clone()).expect("3 件は上限内であること");
        assert_eq!(set.as_slice(), given.as_slice());
    }

    /// **重複はここでは弾かない**（判定は DB の主キーに寄せてある）。
    #[test]
    fn a_set_does_not_judge_duplicates() {
        let duplicated = vec![
            attribute(Subject::Character, "一人称", "僕"),
            attribute(Subject::Character, "一人称", "俺"),
        ];
        assert!(
            AttributeSet::parse(duplicated).is_ok(),
            "重複の判定は store（主キー違反）の担当であること"
        );
    }

    /// **同じキーでも主語が違えば別の属性**（ADR-0009 が `subject` を持たせた理由）。
    #[test]
    fn the_same_key_for_two_subjects_is_two_attributes() {
        let user = attribute(Subject::User, "年齢", "32");
        let character = attribute(Subject::Character, "年齢", "19");
        assert_ne!(user, character, "主語が違えば別の属性であること");
        assert_eq!(user.key, character.key, "前提: キーは同じであること");
        assert_ne!(user.value, character.value, "前提: 値は違うこと");
    }

    /// **エラーの `Display` に件数も上限も載せない**（CLAUDE.md 絶対制約）。
    #[test]
    fn set_error_display_is_a_constant_string() {
        let message = AttributeSetError::TooMany.to_string();
        assert!(
            !message.chars().any(|ch| ch.is_ascii_digit()),
            "定数文字列であること: {message}"
        );
    }
}

#[cfg(test)]
mod change_tests {
    use super::*;

    /// 1件分の属性。**キーと値を同値にしないこと**（取り違えが検出できなくなる）。
    fn attribute(subject: Subject, key: &str, value: &str) -> CharacterAttribute {
        assert_ne!(key, value, "キーと値を同値にしないこと");
        CharacterAttribute {
            subject,
            key: AttributeKey::parse(key).expect("正当なキーであること"),
            value: AttributeValue::parse(value).expect("正当な値であること"),
        }
    }

    fn value(raw: &str) -> AttributeValue {
        AttributeValue::parse(raw).expect("正当な値であること")
    }

    fn key(raw: &str) -> AttributeKey {
        AttributeKey::parse(raw).expect("正当なキーであること")
    }

    /// **無かったキーが増えたら、前の値は「無い」。**
    #[test]
    fn an_added_attribute_has_no_previous_value() {
        let changes = attribute_changes(&[], &[attribute(Subject::Character, "種族", "猫獣人")]);

        assert_eq!(changes.len(), 1, "1件だけ記録されること: {changes:?}");
        assert_eq!(changes[0].subject, Subject::Character);
        assert_eq!(changes[0].key, key("種族"));
        assert_eq!(changes[0].before, None, "前の値は無かったこと");
        assert_eq!(changes[0].after, Some(value("猫獣人")));
    }

    /// **値が変わったら、両側を残す。**
    ///
    /// `before` と `after` を入れ替える変異は、ここで必ず崩れる
    /// （**2つは違う値**なので `assert_ne!` では防げない形を、値ごと固定してある）。
    #[test]
    fn a_changed_value_keeps_both_sides_in_the_right_order() {
        let before = attribute(Subject::Character, "一人称", "僕");
        let after = attribute(Subject::Character, "一人称", "俺");
        assert_ne!(before.value, after.value, "前提: 値が違うこと");

        let changes =
            attribute_changes(std::slice::from_ref(&before), std::slice::from_ref(&after));

        assert_eq!(changes.len(), 1, "1件だけ記録されること: {changes:?}");
        assert_eq!(changes[0].before, Some(value("僕")), "古い値が before 側");
        assert_eq!(changes[0].after, Some(value("俺")), "新しい値が after 側");
    }

    /// **消えたキーは、次の値が「無い」。**
    #[test]
    fn a_removed_attribute_has_no_new_value() {
        let changes = attribute_changes(&[attribute(Subject::User, "呼び方", "先輩")], &[]);

        assert_eq!(changes.len(), 1, "1件だけ記録されること: {changes:?}");
        assert_eq!(changes[0].subject, Subject::User);
        assert_eq!(changes[0].key, key("呼び方"));
        assert_eq!(changes[0].before, Some(value("先輩")));
        assert_eq!(changes[0].after, None, "次の値は無いこと");
    }

    /// **同じ集合を渡したら、変更は1件も無い。**
    ///
    /// 並びを入れ替えただけでも「変更なし」（履歴は並びを記録しない）。
    #[test]
    fn an_unchanged_set_produces_no_change() {
        let current = vec![
            attribute(Subject::Character, "一人称", "僕"),
            attribute(Subject::User, "年齢", "32"),
        ];
        let reordered = vec![
            attribute(Subject::User, "年齢", "32"),
            attribute(Subject::Character, "一人称", "僕"),
        ];

        assert!(
            attribute_changes(&current, &current).is_empty(),
            "同じ集合で変更が出ないこと"
        );
        assert!(
            attribute_changes(&current, &reordered).is_empty(),
            "並べ替えだけでは変更が出ないこと"
        );
    }

    /// **主語だけが違う2つの属性は、取り違えられない**（ADR-0028 §決定4）。
    ///
    /// 同じキー（`年齢`）を `user` と `character` の両方に持たせ、**片方だけを変える。**
    /// `subject` を落とす／固定する実装は、
    /// 「変えていない側」の変更を記録するか、主語を取り違えるかのどちらかになる。
    ///
    /// # **2つの腕が要る**（変異検査で実測 / 2026-09-09）
    ///
    /// | 腕 | これでしか捕まらない変異 |
    /// |---|---|
    /// | **両方が同じ値** | 出力の `subject` を片方に決め打つ形 |
    /// | **値が違う** | **突き合わせを `key` だけで行う形**（`subject` を条件から落とす） |
    ///
    /// **同じ値の腕だけだと、`key` だけで突き合わせる変異が緑を通る**
    /// ——先に見つかる行の値が同じなので、`before` が偶然そろう（実測: `m15` が全緑）。
    #[test]
    fn the_same_key_under_two_subjects_never_swaps_owners() {
        // --- 腕1: 両方が同じ値。**出力の主語**を見る ---
        let same = vec![
            attribute(Subject::User, "年齢", "19"),
            attribute(Subject::Character, "年齢", "19"),
        ];
        let next = vec![
            attribute(Subject::User, "年齢", "19"),
            attribute(Subject::Character, "年齢", "20"),
        ];

        let changes = attribute_changes(&same, &next);

        assert_eq!(changes.len(), 1, "変わったのは1件だけ: {changes:?}");
        assert_eq!(
            changes[0].subject,
            Subject::Character,
            "変えた側の主語が記録されること"
        );
        assert_eq!(changes[0].before, Some(value("19")));
        assert_eq!(changes[0].after, Some(value("20")));

        // --- 腕2: 値が違う。**突き合わせに主語が効いている**ことを見る ---
        let differing = vec![
            attribute(Subject::User, "年齢", "32"),
            attribute(Subject::Character, "年齢", "19"),
        ];
        let changed = vec![
            attribute(Subject::User, "年齢", "32"),
            attribute(Subject::Character, "年齢", "20"),
        ];

        let changes = attribute_changes(&differing, &changed);

        assert_eq!(changes.len(), 1, "変わったのは1件だけ: {changes:?}");
        assert_eq!(
            changes[0].subject,
            Subject::Character,
            "変えた側の主語が記録されること"
        );
        assert_eq!(
            changes[0].before,
            Some(value("19")),
            "**キャラクター側の**古い値が載ること（利用者側の 32 ではない）"
        );
        assert_eq!(changes[0].after, Some(value("20")));

        // --- 腕3: 消える側も同じ。**主語をまたいで「残っている」と読まない** ---
        let removed = vec![attribute(Subject::User, "年齢", "32")];
        let changes = attribute_changes(&differing, &removed);
        let shape: Vec<(Subject, &str, Option<&str>, Option<&str>)> = changes
            .iter()
            .map(|change| {
                (
                    change.subject,
                    change.key.as_str(),
                    change.before.as_ref().map(AttributeValue::as_str),
                    change.after.as_ref().map(AttributeValue::as_str),
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![(Subject::Character, "年齢", Some("19"), None)],
            "同じキーが別の主語に残っていても、消えた側は消えたと記録されること"
        );
    }

    /// **並びは「新しい集合の順 → 消えた順」。**
    ///
    /// 履歴は追記なので、**書く順がそのまま同着の並び**になる（`id` が決める）。
    /// 順を決めていないと、同じ置き換えが回ごとに違う並びで残る。
    #[test]
    fn changes_follow_the_new_set_then_the_removals() {
        let current = vec![
            attribute(Subject::Character, "一人称", "僕"),
            attribute(Subject::User, "年齢", "32"),
            attribute(Subject::User, "呼び方", "先輩"),
        ];
        let next = vec![
            attribute(Subject::User, "年齢", "33"),
            attribute(Subject::Character, "種族", "猫獣人"),
        ];

        let changes = attribute_changes(&current, &next);

        let shape: Vec<(Subject, &str, Option<&str>, Option<&str>)> = changes
            .iter()
            .map(|change| {
                (
                    change.subject,
                    change.key.as_str(),
                    change.before.as_ref().map(AttributeValue::as_str),
                    change.after.as_ref().map(AttributeValue::as_str),
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                // 新しい集合の順（変更 → 追加）。
                (Subject::User, "年齢", Some("32"), Some("33")),
                (Subject::Character, "種族", None, Some("猫獣人")),
                // 消えたものは、保存されていた順。
                (Subject::Character, "一人称", Some("僕"), None),
                (Subject::User, "呼び方", Some("先輩"), None),
            ],
            "新しい集合の順に並び、消えたものが後ろに続くこと"
        );
    }

    /// **重複を渡されても panic しない**（判定は DB の主キー / `AttributeSet` の doc）。
    #[test]
    fn duplicated_keys_do_not_panic() {
        let next = vec![
            attribute(Subject::Character, "一人称", "僕"),
            attribute(Subject::Character, "一人称", "俺"),
        ];
        let changes = attribute_changes(&[], &next);
        assert_eq!(changes.len(), 2, "そのまま2件になること: {changes:?}");
    }
}
