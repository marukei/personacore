//! 会話セッションのドメイン型。

use std::fmt;

/// `session` テーブルの主キー。
///
/// **`i64` の裸のままにしない。** `CharacterId` / `MessageId` と同型の識別子が並ぶため、
/// `store.stat_allocs(session_id)` のような取り違えが型検査を素通りする。
/// 外部キーは「存在しない id」しか弾けず、`session.id = 3` と `character.id = 3` が
/// **両方存在する状況（ほぼ常に成立）では静かに別のデータを返す**。
///
/// `CharacterId` と同じく **`From<i64>` も `pub fn new` も生やさない**。
/// 得る手段は `Store::create_session` の戻り値と `Session::id` だけで、
/// DB からの復元経路は `pub(crate)` に閉じてある。
///
/// > `aoi@42` の `42` は**クライアントから来る信頼できない入力**である。
/// > 数値として解釈できても、それが `SessionId` になってよいのは
/// > `Store::find_session` がそのキャラクターの所有だと確認した後だけ（ADR-0004）。
/// >
/// > そのため **`Store::find_session` は生の `i64` を受け取る。** 呼び出し側が
/// > 未検証の値から `SessionId` を組み立てる経路を残すと、その一行は
/// > 「DB から復元した値」と見分けが付かず、レビューで見落とされる。
/// > **`SessionId` が存在する = 所有確認を通った**、が crate 全体の不変条件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(i64);

impl SessionId {
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

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生の値を往復させる（`store` のバインド値として使うため）。
    #[test]
    fn session_id_round_trips_the_raw_key() {
        for raw in [1_i64, 42, i64::MAX] {
            assert_eq!(SessionId::from_row(raw).get(), raw);
        }
        assert_eq!(SessionId::from_row(42).to_string(), "42");
    }

    /// 値が違えば別の id。同じなら等しい。
    #[test]
    fn session_id_compares_by_value() {
        assert_eq!(SessionId::from_row(3), SessionId::from_row(3));
        assert_ne!(SessionId::from_row(3), SessionId::from_row(4));
        assert!(SessionId::from_row(3) < SessionId::from_row(4));
    }

    /// テスト専用の抜け穴も同じ表現を作る。
    #[test]
    fn test_only_constructor_matches_the_internal_one() {
        assert_eq!(
            SessionId::from_raw_for_tests(9_999),
            SessionId::from_row(9_999)
        );
    }
}
