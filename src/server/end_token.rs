//! **終わりのトークンを下げるかを、本文の1往復ごとに決める**
//! （[ADR-0066](../../docs/adr/0066-the-end-token-is-lowered-once-a-session-has-shrunk.md) §決定2 / 作業単位 A-22b）。
//!
//! # 3つが揃ったときだけ掛ける
//!
//! 1. 本文の面に `sampling.end_token_bias` がある（`config::BodyFace::end_token_bias`）。
//! 2. キャラクターに返答の長さの目安がある（`Character::response_length` / ADR-0065）。
//! 3. **その会話の保存済みの返答のうち、どれか1つでも字数が目安の下限より少ない**
//!    （`Store::shortest_reply_short_of`。直前の返答だけを見ない —— 見ると振動する / ADR-0066 §背景）。
//!
//! **1か2が欠けたら DB を読まない**（欄を書かない面・目安の無いキャラクターの往復は、着手前と同じクエリだけを流す）。
//!
//! # 呼ぶ場所
//!
//! **`super::chat` が、要求を組む前（上流を呼ぶ前）に1回だけ呼ぶ。** 読みはトランザクションを取らないので、
//! 「トランザクションの中で外部通信を待たない」（絶対制約）とは関わらない。決めた値は送る本文にも封にも
//! 空の本文の引き直しにも同じものが載る（`super::counted_plan::Turn::end_token_bias` → `llm::ChatRequest`）。
//!
//! **裏方の経路はここを通らない**（`llm::ChatRequest::background` は常に `None`）。

use crate::domain::{Character, SessionId};
use crate::llm::EndTokenBias;
use crate::store::{Store, StoreError};

/// **掛けた往復で出す `info` の行**（ADR-0066 §決定2）。テストがこの綴りで引く。
pub(super) const LOWERED_LINE: &str =
    "この会話は返答の長さの目安の下限を割ったので、終わりのトークンを下げて送ります";

/// **この往復で送る値**（掛けないなら `None`）。モジュールの doc の3つの条件。
///
/// 掛けた往復では `info` で1行出す（会話の識別子・下限・その会話の最短の返答の字数・送った値 / ADR-0066 §決定2）。
///
/// # Errors
///
/// 保存済みの返答を読めなかったとき（`StoreError`）。**黙って「掛けない」に落とさない** —— 読めない DB の上で
/// 往復を続けると、保存も失敗する（呼び出し側は他の読みと同じく 500 にする）。
pub(super) async fn for_turn(
    store: &Store,
    character: &Character,
    session_id: SessionId,
    written: Option<EndTokenBias>,
) -> Result<Option<EndTokenBias>, StoreError> {
    let (Some(written), Some(target)) = (written, character.response_length) else {
        return Ok(None);
    };
    let Some(shortest) = store.shortest_reply_short_of(session_id, target).await? else {
        return Ok(None);
    };
    tracing::info!(
        character = %character.handle,
        session_id = session_id.get(),
        response_length_min = target.min(),
        shortest_reply_chars = shortest,
        logit_bias = %written.wire_spelling(),
        "{LOWERED_LINE}"
    );
    Ok(Some(written))
}
