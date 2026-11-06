//! 上流へ渡すチャットメッセージのドメイン型。
//!
//! **`llm` ではなく `domain` に置く。** `prompt` の戻り値がこの型であり、
//! `prompt` は I/O を持たない純粋モジュールなので、reqwest を抱える `llm` へ
//! 依存させたくない（`store` の sqlx も同じ理由で `domain` 経由にしてある）。
//! 綴りの決定権（JSON の `"system"` 等）は `llm::request` に残してある。

use super::message::Role;

/// メッセージの役割。OpenAI 互換の3種類。
///
/// **`String` で持たない。** 綴りを間違えると上流はエラーにせず、
/// 「システムプロンプトが効かない」という形で静かに劣化する。
///
/// `Role`（DB の2値）とは**別の型**。こちらは `System` を持つ上流の3値で、
/// 両者の変換は下の `From` 実装だけに集める。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChatRole {
    /// システムプロンプト（ペルソナ・性格・記憶）。
    System,
    /// 利用者の発言。
    User,
    /// キャラクターの発言。
    Assistant,
}

/// 保存された役割から、上流へ渡す役割へ。
///
/// **変換の置き場所をここに固定する。** 両方の enum が見える唯一の層がここで、
/// 呼び出し側（`prompt`）に手書きの `match` を置くと、
/// 変換の守り手が呼び出し側のテストだけになる。
///
/// `System` はこの経路からは作られない（システムプロンプトは `prompt` が組み立てるもので、
/// 会話履歴から生えてはいけない）。
impl From<Role> for ChatRole {
    fn from(role: Role) -> Self {
        match role {
            Role::User => Self::User,
            Role::Assistant => Self::Assistant,
        }
    }
}

/// 上流へ送る1メッセージ。
///
/// 内容の検証・整形はここでは行わない。**プロンプト注入への防御は `prompt` 層の責務**
/// （docs/06-roadmap.md）。他の層で部分的に整形すると、防御が2箇所に分散して
/// どちらも不完全になる。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// 役割。
    pub role: ChatRole,
    /// 本文。
    pub content: String,
}

impl ChatMessage {
    /// システムプロンプト。
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
        }
    }

    /// 利用者の発言。
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    /// キャラクターの発言。
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Role;

    /// **保存された役割は、対応するチャット役割へ写る。**
    ///
    /// `Role` は DB の2値、`ChatRole` は上流の3値で別物なので変換が要る。
    /// `match` で書いてあるため、`Role` に値が増えたら**このテストがコンパイルできなくなる**
    /// （網羅性の保証はここ）。取り違え（`User` → `Assistant`）は `assert_eq!` が捕まえる。
    #[test]
    fn every_stored_role_maps_to_the_matching_chat_role() {
        for role in [Role::User, Role::Assistant] {
            let expected = match role {
                Role::User => ChatRole::User,
                Role::Assistant => ChatRole::Assistant,
            };
            assert_eq!(ChatRole::from(role), expected, "role = {role:?}");
        }
    }

    /// **`System` は保存された役割からは決して作られない。**
    ///
    /// システムプロンプトは `prompt` が組み立てるものであり、会話履歴から生えてはいけない。
    /// ここが崩れると、履歴の1件がシステム指示として上流に渡る。
    #[test]
    fn a_stored_role_never_becomes_a_system_message() {
        for role in [Role::User, Role::Assistant] {
            assert_ne!(ChatRole::from(role), ChatRole::System, "role = {role:?}");
        }
    }

    /// 補助コンストラクタが役割を取り違えていないこと。
    ///
    /// `system` と `user` を取り違えても上流はエラーにせず、
    /// 「システムプロンプトが効かない」形で静かに劣化するため、値を固定する。
    #[test]
    fn constructors_set_the_matching_role() {
        assert_eq!(
            ChatMessage::system("x"),
            ChatMessage {
                role: ChatRole::System,
                content: "x".to_owned(),
            }
        );
        assert_eq!(ChatMessage::user("x").role, ChatRole::User);
        assert_eq!(ChatMessage::assistant("x").role, ChatRole::Assistant);
    }
}
