//! **返答の長さの目安**（[ADR-0065](../../docs/adr/0065-the-response-length-target-is-a-character-field.md) /
//! 作業単位 A-22a = marukei/personacore#71）。
//!
//! キャラクターごとに「下限と上限の字数」の1組を持つ。**既定は未設定**（`Character::response_length` が
//! `None`）で、未設定のキャラクターのプロンプトは1バイトも変わらない（ADR-0065 §決定1）。
//!
//! # ここに置くのは数だけ
//!
//! **保存する値（下限と上限）は言語に依らない**（ADR-0065 §決定2）。プロンプトに載る文と、
//! 数の書き方（漢数字）は `ja` の束の持ち物で、`prompt` の定数として置く（ADR-0029 §決定2）。
//!
//! # 片方だけ・範囲の外・下限 > 上限は表現できない
//!
//! **[`ResponseLength::parse`] が唯一のコンストラクタ**で、下限と上限を**同時に**受け取る。
//! 片方だけを持つ値は型として作れない（`Option` を2つ並べる形にしない）。
//! 保存の側も同じ1組を CHECK で縛る（`migrations/0019_character_response_length.sql`）。

/// **返答の長さの目安**（下限と上限の字数。ADR-0065 §決定1）。
///
/// `StatScale` / `AxisDiscoverySlots` と同じく **`parse` が唯一のコンストラクタ**
/// （`From` も `pub fn new` も生やさない）。
///
/// # 上限 9999 の理由
///
/// **削れない枠に載る行の長さを有限にするため**（4桁。ADR-0065 §決定1）。返答の上限
/// （`response_tokens`）とは結びつけない —— 返答の上限は壊れないための天井で、長さのつまみではない
/// （ADR-0059 §決定2）。トークンと字数の比はトークナイザに付く（ADR-0033）。
/// **上げるときはこの定数だけでは足りない** —— `migrations/0019_character_response_length.sql` の
/// CHECK が同じ 9999 で縛っているので、新しい移行で CHECK も上げないと書き込みが落ちる。
/// 最悪ケース（漢数字で最もバイト数が大きくなる数）が既定の予算に収まることは
/// `prompt` の `every_field_at_its_limit_still_fits_the_default_budget` が固定している。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResponseLength {
    min: u16,
    max: u16,
}

impl ResponseLength {
    /// 下限・上限が取れる最小の字数。
    pub const LOWEST: u16 = 1;

    /// 下限・上限が取れる最大の字数（4桁 / ADR-0065 §決定1）。
    pub const HIGHEST: u16 = 9999;

    /// 唯一のコンストラクタ。**下限と上限を同時に受け取る。**
    ///
    /// 引数が `u32` なのは、要求（`u32` で読む）と保存（`i64` を `u32` に直して読む）の
    /// 両方からそのまま渡すため。範囲の判定はここ1箇所で行う。
    ///
    /// # Errors
    ///
    /// 判定の順は「小さすぎる → 大きすぎる → 逆転」。
    ///
    /// * 下限が [`Self::LOWEST`] 未満 → [`ResponseLengthError::TooSmall`]
    /// * 下限か上限が [`Self::HIGHEST`] を超える → [`ResponseLengthError::TooLarge`]
    /// * 下限 > 上限 → [`ResponseLengthError::Inverted`]
    pub fn parse(min: u32, max: u32) -> Result<Self, ResponseLengthError> {
        if min < u32::from(Self::LOWEST) {
            return Err(ResponseLengthError::TooSmall);
        }
        if min > u32::from(Self::HIGHEST) || max > u32::from(Self::HIGHEST) {
            return Err(ResponseLengthError::TooLarge);
        }
        if min > max {
            return Err(ResponseLengthError::Inverted);
        }
        // 上の判定で `HIGHEST`（`u16` に収まる）以下と確かめてあるので、ここは失敗しない。
        // それでも `as` で切り詰めず、失敗したら「大きすぎる」に倒す（panic の経路を作らない）。
        match (u16::try_from(min), u16::try_from(max)) {
            (Ok(min), Ok(max)) => Ok(Self { min, max }),
            _ => Err(ResponseLengthError::TooLarge),
        }
    }

    /// 下限の字数（`LOWEST..=HIGHEST`）。
    #[must_use]
    pub const fn min(self) -> u16 {
        self.min
    }

    /// 上限の字数（`min()..=HIGHEST`）。
    #[must_use]
    pub const fn max(self) -> u16 {
        self.max
    }

    /// **その返答の字数が、目安の下限を割っているか**（[ADR-0066](../../docs/adr/0066-the-end-token-is-lowered-once-a-session-has-shrunk.md)
    /// §決定2 / 作業単位 A-22b）。
    ///
    /// **`<` で比べる**（下限ちょうどは割っていない）。字数は返答の `raw_content` の Unicode のスカラー値の数
    /// （`str::chars` の数）で、数えるのは呼び出し側（`store` の `shortest_reply_short_of`）。
    #[must_use]
    pub fn falls_short(self, chars: usize) -> bool {
        chars < usize::from(self.min)
    }
}

/// [`ResponseLength::parse`] が受け付けなかった理由。
///
/// **`Display` は定数文字列**（入力も上下限も反射しない / 絶対制約）。
/// 理由を3つに分けるのは、**送り直しの向き**が応答から読めるようにするため
/// （`ScaleError` と同じ判断）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResponseLengthError {
    /// 下限が小さすぎる（1 未満）。
    #[error("返答の長さの目安が小さすぎます")]
    TooSmall,
    /// 下限か上限が大きすぎる（9999 を超える）。
    #[error("返答の長さの目安が大きすぎます")]
    TooLarge,
    /// 下限が上限より大きい。
    #[error("返答の長さの目安の下限が上限を超えています")]
    Inverted,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 通った値の (下限, 上限)。
    fn pair(min: u32, max: u32) -> Result<(u16, u16), ResponseLengthError> {
        ResponseLength::parse(min, max).map(|length| (length.min(), length.max()))
    }

    /// **下限を割るのは、下限より少ないときだけ**（ADR-0066 §決定2 の「下限ちょうどは割っていない」）。
    ///
    /// 下限の前後（下限 − 1・下限・下限 + 1）を、上限と違う下限で見る（下限と上限を取り違える実装を赤にする）。
    /// 期待値は手で書く。
    #[test]
    fn a_reply_falls_short_only_below_the_lower_bound() {
        let target = ResponseLength::parse(300, 600).expect("範囲の中");
        assert!(target.falls_short(0));
        assert!(target.falls_short(299));
        assert!(!target.falls_short(300), "下限ちょうどは割っていない");
        assert!(!target.falls_short(301));
        assert!(!target.falls_short(599));
        assert!(!target.falls_short(601), "上限を超えても割ってはいない");
        let lowest = ResponseLength::parse(1, 1).expect("範囲の中");
        assert!(lowest.falls_short(0));
        assert!(!lowest.falls_short(1));
    }

    /// **端の値は通る。** 期待値は定数から作らない（外の値 1 / 9999 で押さえる）。
    #[test]
    fn the_edges_are_accepted() {
        assert_eq!(pair(1, 1), Ok((1, 1)));
        assert_eq!(pair(1, 9999), Ok((1, 9999)));
        assert_eq!(pair(9999, 9999), Ok((9999, 9999)));
        assert_eq!(pair(200, 400), Ok((200, 400)));
        assert_eq!(pair(800, 1200), Ok((800, 1200)));
        assert_eq!(pair(400, 400), Ok((400, 400)), "下限 = 上限は正当");
    }

    /// **下限 0 は小さすぎる**（上限が何であっても）。
    #[test]
    fn a_zero_minimum_is_too_small() {
        assert_eq!(pair(0, 1), Err(ResponseLengthError::TooSmall));
        assert_eq!(pair(0, 400), Err(ResponseLengthError::TooSmall));
        assert_eq!(pair(0, 0), Err(ResponseLengthError::TooSmall));
    }

    /// **10000 は大きすぎる**（下限の側でも上限の側でも）。`u32` の端まで降りる。
    #[test]
    fn ten_thousand_is_too_large_on_either_side() {
        assert_eq!(pair(1, 10_000), Err(ResponseLengthError::TooLarge));
        assert_eq!(pair(9999, 10_000), Err(ResponseLengthError::TooLarge));
        assert_eq!(pair(10_000, 10_000), Err(ResponseLengthError::TooLarge));
        assert_eq!(pair(10_000, 5), Err(ResponseLengthError::TooLarge));
        assert_eq!(pair(1, u32::MAX), Err(ResponseLengthError::TooLarge));
        assert_eq!(pair(u32::MAX, u32::MAX), Err(ResponseLengthError::TooLarge));
        // `u16` に収まるが範囲の外（`u16` への切り詰めで 9999 以下に見える変異を止める）。
        assert_eq!(pair(1, 65_536 + 400), Err(ResponseLengthError::TooLarge));
    }

    /// **下限 > 上限は逆転**（差が1でも）。
    #[test]
    fn a_minimum_above_the_maximum_is_inverted() {
        assert_eq!(pair(401, 400), Err(ResponseLengthError::Inverted));
        assert_eq!(pair(9999, 1), Err(ResponseLengthError::Inverted));
        assert_eq!(pair(2, 1), Err(ResponseLengthError::Inverted));
    }

    /// **理由ごとに文面が違う**（同じ応答に潰さない）。
    #[test]
    fn each_reason_has_its_own_message() {
        let messages = [
            ResponseLengthError::TooSmall.to_string(),
            ResponseLengthError::TooLarge.to_string(),
            ResponseLengthError::Inverted.to_string(),
        ];
        for (index, left) in messages.iter().enumerate() {
            for right in &messages[index + 1..] {
                assert_ne!(left, right);
            }
        }
    }
}
