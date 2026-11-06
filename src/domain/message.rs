//! メッセージのドメイン型。

use std::fmt;

use chrono::{DateTime, Utc};

/// `message` テーブルの主キー。
///
/// `CharacterId` / `SessionId` と**別の型**にしてある。どれも中身は `i64` だが、
/// 取り違えても外部キーは黙って通る（`message.id = 3` と `session.id = 3` は
/// ほぼ常に両方存在する）。
///
/// `From<i64>` も `pub fn new` も生やさない。得る手段は `Store::append_message` の
/// 戻り値と `Message::id` だけ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MessageId(i64);

impl MessageId {
    /// DB から復元する内部経路。**crate 外からは呼べない。**
    pub(crate) const fn from_row(raw: i64) -> Self {
        Self(raw)
    }

    /// 生の主キー値。SQL へのバインド値はこれを使う。
    pub const fn get(self) -> i64 {
        self.0
    }

    /// **テスト専用の抜け穴。** 「存在しない id」を作る用途にのみ使う。
    #[cfg(any(test, feature = "testing"))]
    pub const fn from_raw_for_tests(raw: i64) -> Self {
        Self(raw)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// メッセージの発話者。
///
/// **`String` で持たない。** スキーマの `CHECK (role IN ('user', 'assistant'))` と
/// 対応する2値であり、文字列で持つと `"System"` や `"assistant "` のような値が
/// アプリ層を通り抜けて CHECK 違反として初めて表面化する。
///
/// **DB との相互変換は `store` に閉じ込める**（`src/store/session.rs`）。
/// ここに `as_str` を生やすと、綴りの決定権が `domain` と `store` に分散する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// 利用者の発言。
    User,
    /// キャラクターの発言。
    Assistant,
}

impl Role {
    /// **全部の値**（作業単位 P-2a —— 仕様書が応答の綴りの `enum` をここから作る）。
    ///
    /// **綴りはここに無い**（HTTP の綴りは `server::admin` の `role_to_wire`、DB の綴りは `store`）。
    /// 数え漏れは `every_role_variant_is_listed_in_all` が見る。
    pub const ALL: [Self; 2] = [Self::User, Self::Assistant];
}

/// これから書き込むメッセージ。
///
/// **`raw_content` は `Option` ではない。** パース前の生出力はパース失敗時の
/// 唯一の調査手段であり（CLAUDE.md 絶対制約）、「分解できたから生は要らない」という
/// 呼び出し方を型で表現できなくするために必須フィールドにしてある。
/// 構造体リテラルでしか作れないので、うっかり省略するとコンパイルが通らない。
///
/// 逆に `dialogue` / `narration` は `Option`。パーサは形式が崩れても panic せず
/// フォールバックするため、**両方 `None`（生出力だけ）が正常な状態として存在する**。
///
/// ここでは内容の検証・正規化を一切しない。中身は LLM の出力そのもので、
/// 加工すると調査手段としての価値が失われる。SQL へはプレースホルダで渡すので、
/// 値が SQL として解釈されることはない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    /// 発話者。
    pub role: Role,
    /// 構造化出力の分解結果（台詞）。パースできなければ `None`。
    pub dialogue: Option<String>,
    /// 構造化出力の分解結果（地の文）。パースできなければ `None`。
    pub narration: Option<String>,
    /// パース前の生出力。**必ず保持する。**
    ///
    /// **空文字は正当な値として受け入れる。** 「LLM が空を返した」こと自体が調査材料であり、
    /// 拒否するとその事象を記録できなくなる（`raw_content` の存在理由に反する）。
    /// `character.handle` に空を禁じる CHECK があるのとは非対称だが、これは意図した非対称。
    pub raw_content: String,
}

/// `message` テーブルの1行。
///
/// **読み書きは `store`、置き場所はここ**（理由は `super` の doc）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// 主キー。
    pub id: MessageId,
    /// 所属セッション。
    pub session_id: super::SessionId,
    /// 発話者。
    pub role: Role,
    /// 構造化出力の分解結果（台詞）。パースできなければ `None`。
    pub dialogue: Option<String>,
    /// 構造化出力の分解結果（地の文）。パースできなければ `None`。
    pub narration: Option<String>,
    /// **パース前の生出力。** パース失敗時の唯一の調査手段。
    ///
    /// プロンプトへ戻すときもこの値を使う（`prompt`）。分解後を組み直すと、
    /// モデルが直前に自分で出した形式と食い違って出力形式が回を追うごとに崩れる。
    pub raw_content: String,
    /// 作成日時（UTC）。
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **`Role::ALL` に数え漏れが無い**（`domain::memory` の
    /// `every_subject_variant_is_listed_in_all` と同じ仕掛け —— 網羅の `match`）。
    #[test]
    fn every_role_variant_is_listed_in_all() {
        // **ワイルドカードの腕を足さないこと。** `ALL` からは回さない（`contains` が必ず真になる）。
        let listed = |role: Role| match role {
            Role::User | Role::Assistant => Role::ALL.contains(&role),
        };
        for role in [Role::User, Role::Assistant] {
            assert!(listed(role), "{role:?} が ALL に無い");
        }
        assert_eq!(
            Role::ALL.len(),
            2,
            "役割を足したなら ALL の要素数も増えていること"
        );
    }

    /// 生の値を往復させる（`store` のバインド値として使うため）。
    #[test]
    fn message_id_round_trips_the_raw_key() {
        for raw in [1_i64, 42, i64::MAX] {
            assert_eq!(MessageId::from_row(raw).get(), raw);
        }
        assert_eq!(MessageId::from_row(42).to_string(), "42");
    }

    /// 値が違えば別の id。同じなら等しい。
    #[test]
    fn message_id_compares_by_value() {
        assert_eq!(MessageId::from_row(3), MessageId::from_row(3));
        assert_ne!(MessageId::from_row(3), MessageId::from_row(4));
        assert!(MessageId::from_row(3) < MessageId::from_row(4));
    }

    /// テスト専用の抜け穴も同じ表現を作る。
    #[test]
    fn test_only_constructor_matches_the_internal_one() {
        assert_eq!(
            MessageId::from_raw_for_tests(9_999),
            MessageId::from_row(9_999)
        );
    }

    /// 発話者は2値。取り違えを検出できるよう比較可能にしてある。
    #[test]
    fn role_has_two_distinct_values() {
        assert_eq!(Role::User, Role::User);
        assert_ne!(Role::User, Role::Assistant);
    }

    /// パース失敗時は `dialogue` / `narration` が両方 `None` で、生出力だけが残る。
    #[test]
    fn new_message_may_carry_only_the_raw_output() {
        let message = NewMessage {
            role: Role::Assistant,
            dialogue: None,
            narration: None,
            raw_content: "「こんにちは（閉じ括弧が無い".to_owned(),
        };
        assert_eq!(message.dialogue, None);
        assert_eq!(message.narration, None);
        assert!(
            !message.raw_content.is_empty(),
            "生出力は常に持っていること"
        );
    }
}
