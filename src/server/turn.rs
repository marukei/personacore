//! **会話の往復の分解と保存**（作業単位 11b-2）。
//!
//! `chat`（非ストリーミング）と `relay`（SSE 中継）の**両方がここを通る。**
//! 分けると、パース成功率のログと保存の作法が経路ごとにずれる。
//!
//! ```text
//! 上流の応答（生出力）
//!   → domain::strip_leaked_thinking … 漏れた思考を剥がす（ADR-0053。raw_content は生のまま）
//!   → parse::parse_response      … 台詞 / 地の文へ分解（純粋関数）
//!   → domain::NewMessage × 2     … 利用者の発言 ＋ 応答
//!   → Store::append_turn         … 1トランザクションで書く（11b-1）
//! ```
//!
//! # 守っていること（CLAUDE.md 絶対制約）
//!
//! * **`raw_content` を必ず入れる。** 分解結果を入れない。パース失敗時の唯一の調査手段で、
//!   プロンプトへ戻すときもこちらを読む（`prompt::history_turn`）。
//! * **利用者の発言をパースしない。** `parse_response` は**モデルの出力**を
//!   ADR-0007 の形式として読む関数で、人が書いた `「…」` を台詞として保存すると
//!   「キャラクターが言った台詞」と見分けが付かなくなる。
//! * **パース成功率をログに残す。** 載せるのは [`ParseOutcome::label`]（値ごとの定数）で、
//!   **生出力は載せない。**
//! * **トランザクションの中で `await` しない。** ここへ来る時点で上流の呼び出しは
//!   完了しており、[`TurnSink::persist`] は書き込みだけを行う。
//!
//! # 保存の失敗で応答を壊さない
//!
//! [`TurnSink::persist`] は `Result` を返さない。**返しても呼び出し側にできることが無い。**
//!
//! * 中継経路では `[DONE]` を送る直前なので、もう応答のステータスは変えられない。
//! * 非ストリーミング経路でも、生成し終えた応答を捨てて 500 を返すのは
//!   「もう一度生成させる」ことであり、失敗の代償として高すぎる。
//!
//! したがって失敗は `error!` で記録して先へ進む。**これは黙ったデータ欠落**なので、
//! 集計（破棄率のメトリクス）は docs/06-roadmap.md の持ち越しに積んである。
//!
//! なお `StoreError::InvalidTurn` は**この層では起こしようがない**。往復の `role` は
//! [`turn_messages`] が定数で組み立てるので、`(User, Assistant)` 以外にならない。
//! 種別ごとの出し分けを持たないのはそのため（起きたら実装の誤りで、`error!` に出る）。

use super::empty_retry::EmptyRetry;
use super::extract::Extraction;
use crate::domain::{NewMessage, Role, SessionId, StatAlloc, strip_leaked_thinking};
use crate::parse::leak::{AxisLeaks, detect_axis_leaks};
use crate::parse::{ParseOutcome, parse_response};
use crate::store::{Store, StoreError};

/// **生成している間にキャラクターが完全に消されて、往復を保存しなかった**ことの記録
/// （`info` / 作業単位 A-10 のレビュー）。**テストが綴りを照合する。**
///
/// 「会話の往復を保存できませんでした」（`error`）と分けるのは、利用者が自分で消した結果で、
/// 直すものが無いから。`tests/server_chat_deleted_mid_turn.rs` は綴りを直書きで持つ（写しを1つ）。
const TURN_VANISHED_MESSAGE: &str =
    "キャラクターが消えていたので、会話の往復を保存しませんでした（応答は返します）";

/// 書き込む往復の2件と、分解の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
struct TurnMessages<'a> {
    /// 利用者の発言（先に入る方）。
    user: NewMessage,
    /// 応答（後に入る方）。
    reply: NewMessage,
    /// 分解がどうなったか。**成功率の集計キー。**
    outcome: ParseOutcome,
    /// 本文から落とした Markdown の強調の**対の数**（作業単位 B-46）。
    ///
    /// **記録するだけで、保存する2件には影響しない**——`raw_content` は
    /// 生出力そのままで、落ちているのは `dialogue` / `narration` の側。
    emphasis_pairs_stripped: usize,
    /// 分解の手前で剥がした思考（ADR-0053 / 作業単位 B-61）。**記録するだけ**——
    /// `raw_content` は思考込みの生出力のまま。
    thinking: ThinkingTally,
    /// 応答に漏れた軸名（ADR-0018）。**記録するだけで、保存する2件には影響しない。**
    leaks: AxisLeaks<'a>,
}

/// 分解の手前で剥がした思考の数え（ADR-0053 §決定5）。**ログの2つの欄になる。**
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ThinkingTally {
    /// 剥がした思考のブロックの数。
    blocks: usize,
    /// 閉じないまま末尾に達したか（開きタグから末尾まで捨てた）。
    unclosed: bool,
}

/// 生出力を分解して、書き込む2件に落とす。**純粋関数**（I/O も時刻も見ない）。
///
/// 分解するのは応答だけ。利用者の発言は `raw_content` にそのまま入れる
/// （モジュールの doc を参照）。
///
/// **`stats` は判定にしか使わない。** 借りるのは軸名だけで、
/// 書き込む2件にも応答にも一切影響しない（ADR-0018「記録だけ」）。
fn turn_messages<'a>(
    user_message: &str,
    raw_reply: &str,
    stats: &'a [StatAlloc],
) -> TurnMessages<'a> {
    // **上流が漏らした思考を、分解の手前で剥がす**（ADR-0053 §決定4）。
    // `parse_response` の中で剥がさないのは、崩れた回の「生出力を丸ごと地の文」が
    // 思考込みに戻ってしまうから（ADR-0053 の却下欄）。**`raw_content` には生のまま入れる。**
    let thinking = strip_leaked_thinking(raw_reply);
    let parsed = parse_response(&thinking.text);
    // **分解の後に判定する。** 地の文と台詞を分けて数えるので、分解前には決められない。
    let leaks = detect_axis_leaks(&parsed, stats);

    TurnMessages {
        user: NewMessage {
            role: Role::User,
            // **利用者の発言はパースしない。** `「」` を含んでいても台詞にしない。
            dialogue: None,
            narration: None,
            raw_content: user_message.to_owned(),
        },
        reply: NewMessage {
            role: Role::Assistant,
            dialogue: parsed.dialogue,
            narration: parsed.narration,
            // **生出力を丸ごと。** 崩れた回もここに全部残る（絶対制約）。
            // **剥がした後の本文を入れないこと**（思考込みの生出力が唯一の調査材料 / ADR-0053 §決定3）。
            raw_content: raw_reply.to_owned(),
        },
        outcome: parsed.outcome,
        emphasis_pairs_stripped: parsed.emphasis_pairs_stripped,
        thinking: ThinkingTally {
            blocks: thinking.blocks,
            unclosed: thinking.unclosed,
        },
        leaks,
    }
}

/// 分解の結果を記録する。**パース成功率の観測点**（CLAUDE.md 絶対制約）。
///
/// 載せるのは [`ParseOutcome::label`] の定数と真偽値、それに**軸名**だけ。
/// **生出力を載せない**——ログに LLM の出力を丸ごと流すと、
/// 運用ログがそのまま会話の写しになる。軸名は運用者が `/admin/` から書いた値で、
/// LLM の出力ではない（ADR-0018）。
///
/// `info` にするのは、既定のログフィルタ（`personacore=info`）で
/// 運用者の目に入る必要があるため。成功率は「集めて初めて意味を持つ」値なので、
/// 1件も落とさずに出す。
///
/// # 軸名の漏れを**同じ1行に**載せる理由（ADR-0018 / 作業単位 0011c）
///
/// 別のイベントにすると、率を出すたびに**2種類のログを突き合わせる**ことになる。
/// 同じ行なら「このターンは分解でき、かつ軸名が漏れた」が直接数えられる。
///
/// * **漏れが無いターンでも欄を出す**（空のまま）。出さないと
///   **「欄が無い」と「0件」が区別できず**、分母が作れない。
/// * **地の文と台詞は別の欄**（ADR-0018。壊れ方が違う）。
/// * 形は `["ユーモア"]` / `[]`（`Debug`）。軸名は中間に空白を含みうる
///   （`StatAxis` は中間の空白を保つ）ので、区切りを空白やカンマに任せると
///   **集計側で軸の境界が決まらない。** `Debug` なら引用符で囲まれる。
fn log_outcome(
    outcome: ParseOutcome,
    emphasis_pairs_stripped: usize,
    thinking: ThinkingTally,
    leaks: &AxisLeaks<'_>,
    session_id: SessionId,
    retry: EmptyRetry,
) {
    // **`%`（Display）で載せる。** `&str` をそのまま渡すと `outcome="structured"` と
    // 引用符付きになり、集計キーとして扱うたびに剥がすことになる。
    tracing::info!(
        outcome = %outcome.label(),
        fell_back = outcome.fell_back(),
        // **漏れが無くても出す**（`[]`）。欄ごと消すと 0 件と欠落が区別できない。
        axis_leak_narration = ?leaks.narration(),
        axis_leak_dialogue = ?leaks.dialogue(),
        // **落とした対の数**（作業単位 B-46）。**画面には出さない**ので、
        // モデルが強調を書く頻度を知る手段はこの欄だけ。
        // **⚠ 抽出側（`server::extract`）の `emphasis_stripped` とは別の欄。**
        // あちらは**落とした行数**で、こちらは**対の数**。**どちらの行も `session_id` を持つ**ので、
        // 同じ綴りにすると単位の違う2つの数が集計で1つの列に混ざる（`parse::ParsedResponse` の欄の doc）。
        // **0 の回も出す**（出さないと分母が作れない / 上の軸名の欄と同じ理由）。
        emphasis_pairs_stripped,
        // **分解の手前で剥がした思考**（ADR-0053 §決定5 / 作業単位 B-61）。**画面には出さない。**
        // **0 / false の回も出す**（出さないと分母が作れない / 上の欄と同じ理由）。
        // 欄名は `thinking_` で始める —— `emphasis_*`（対の数）や抽出側の欄と単位が混ざらないように。
        // `thinking_unclosed=true` は「閉じタグを書き忘れて本文を失った」可能性の印
        // （ADR-0053 §結果。そのとき画面は「応答が空でした。」になる）。
        thinking_blocks_stripped = thinking.blocks,
        thinking_unclosed = thinking.unclosed,
        // **空の本文の引き直し**（ADR-0059 §決定3 / 作業単位 A-18b）。**0 / false / None の回も出す**
        // （出さないと分母が作れない）。`empty_retries` は2回目の要求を送った回数（失敗した回も 1）。
        // `empty_retry_failed=true` は2回目が失敗して1回目を保存した回（捨てた回は無いので `discarded_*` は 0）。
        // `discarded_*` は捨てた1回目のもの —— 生の出力は保存しないので、長さ（文字数・思考込み）・
        // 閉じない思考だったか・上流が数えた生成数（返さなければ `None`）だけがここに残る。
        empty_retries = retry.retries(),
        empty_retry_failed = retry.failed_over(),
        discarded_reply_chars = retry.discarded_chars(),
        discarded_reply_unclosed = retry.discarded_unclosed(),
        discarded_completion_tokens = ?retry.discarded_completion_tokens(),
        session_id = session_id.get(),
        "応答を分解しました"
    );
}

/// 1本の応答の保存先。**`chat` が組み立て、`relay` は持ち回るだけ。**
///
/// 中継は本文を出し切ってから保存する（`super::relay`）ので、
/// 保存に要るもの（DB ハンドル・着地したセッション・今回の発言）を
/// 中継の寿命のあいだ持ち歩く必要がある。
pub(super) struct TurnSink {
    /// 永続化層。`Store` は `Clone` で同じプールを指す。
    store: Store,
    /// **所有確認を通ったセッション**（`SessionId` が存在する = 確認済み）。
    session_id: SessionId,
    /// 今回の利用者の発言。**`raw_content` にそのまま入る。**
    user_message: String,
    /// そのキャラクターの性格ステータス。**軸名の漏れの判定にだけ使う**（ADR-0018）。
    ///
    /// **プロンプトを組んだときと同じ一覧**を持ち回る（`chat` が渡す）。
    /// ここで DB を引き直さない——**追加のクエリを足さない**のがこの機能の前提で、
    /// 引き直すと「プロンプトに載った軸」ではなく「今の軸」を見ることになる。
    stats: Vec<StatAlloc>,
    /// **記憶抽出の配線**（ADR-0021 §6 / `super::extract`）。
    ///
    /// **`Option` にしない。** 経路ごとに渡し忘れられる形にすると、
    /// 「中継だけ抽出が走らない」が**コンパイルの通る書き間違い**になる。
    extraction: Extraction,
}

impl TurnSink {
    /// 保存先を決める。**セッションは解決済みのものを受け取る。**
    pub(super) fn new(
        store: Store,
        session_id: SessionId,
        user_message: &str,
        stats: Vec<StatAlloc>,
        extraction: Extraction,
    ) -> Self {
        Self {
            store,
            session_id,
            user_message: user_message.to_owned(),
            stats,
            extraction,
        }
    }

    /// 往復を保存する。**応答が完全に返り切った回だけ呼ぶこと。**
    ///
    /// 途中で失敗した回に呼ぶと、部分的な応答が履歴に載って以後の文脈が汚れる。
    /// 「呼ばない」ことで守っているので、中継での呼び出し位置は `super::relay` の
    /// `RelayState::complete` の1箇所だけにしてある。そこへ来るのは2経路
    /// （`chunk_events` の**上流が尽きた枝**と、空の本文を引き直す2回目が**開けなかった後の締め**（`finish_next`） /
    /// ADR-0059 §決定3）で、どちらも `[DONE]` を流す直前。非ストリーミングは `super::chat` の1箇所。
    ///
    /// 失敗しても応答は壊さない（モジュールの doc）。
    ///
    /// **`raw_reply` は採用した回の生の出力**（空の本文を引き直した回は2回目 / ADR-0059 §決定3）。
    /// `retry` は引き直しの記録で、分解の行の欄にだけ載る（保存する2件には影響しない）。
    pub(super) async fn persist(&self, raw_reply: &str, retry: EmptyRetry) {
        let turn = turn_messages(&self.user_message, raw_reply, &self.stats);

        // **保存できてもできなくても記録する。** 成功率は保存の成否と別の話。
        //
        // **`append_turn` の後ろへ動かさないこと。** 保存に失敗した回の記録が消えると、
        // 残るのは下の `error!`（＝「保存できなかった」）だけになり、
        // **分解できたのか崩れたのかが、まさに調査したい回で失われる**——しかも
        // 分母から抜けた分だけ成功率が実態より良く見える。見張り番は
        // `tests/server_chat_parse_log.rs::a_failure_to_persist_is_recorded_as_an_error`
        // （この位置を動かす変異は、それを足すまで 686/686 緑で通り抜けた）。
        //
        // **順序（`append_turn` より前）にも別の見張り番が寄りかかっている。**
        // 事情は `super::relay` の `tests::PERSIST_TARGET` の doc にあり（ここに写さない）、
        // 動かすと `relay::tests::the_persist_log_comes_before_the_database_write` が赤くなる。
        log_outcome(
            turn.outcome,
            turn.emphasis_pairs_stripped,
            turn.thinking,
            &turn.leaks,
            self.session_id,
            retry,
        );

        // **1トランザクションで往復を書く**（11b-1）。ここに `await` する外部通信は無い。
        //
        // **`append_message` を2回呼ぶ形にしないこと。** 逐次に叩くかぎり同じ行が入るが、
        // 同時に書いたときに往復の間へ別のリクエストが割り込む
        // （`tests/server_chat.rs::concurrent_turns_on_the_same_session_are_written_indivisibly`
        // が 6/6 で赤にした変異）。
        match self
            .store
            .append_turn(self.session_id, &turn.user, &turn.reply)
            .await
        {
            // **保存に成功した直後にだけ、記憶抽出を走らせる**（ADR-0021 §6）。
            //
            // * **ここが唯一の起点。** 非ストリーミングと中継の両方がこの関数を通るので、
            //   配線が1本で済む（2箇所に置くと**片方だけが観測対象から外れる**
            //   ——11c-3b で実測した形）。
            // * **`Ok` の枝に限ること。** 保存できなかった往復は履歴に載っていない。
            //   そこから抽出すると、**どこにも残っていない会話から作られた記憶**が
            //   恒久的に残る（`memory` に更新も削除も無い）。
            // * **`await` しない。** `start` は spawn するだけで、応答の経路に
            //   待ちを1ミリ秒も足さない（CLAUDE.md 絶対制約）。中継経路では
            //   ここは `[DONE]` を送る直前なので、待てばそのぶん `[DONE]` が遅れる。
            Ok(appended) => {
                // **応答の id は試用の輪番の鍵**（`TurnJob::reply_id` / 作業単位 A-9c のレビュー）。
                self.extraction.start(
                    self.session_id,
                    appended.reply,
                    &self.user_message,
                    raw_reply,
                );
            }
            // **生成している間にキャラクターが完全に消された回は故障ではない**
            // （ADR-0049 §決定3 / 作業単位 A-10 のレビュー）。会話もカスケードで消えているので
            // 保存は `UnknownSession` で落ちる。**会話が本当に無いことを引き直して確かめてから**
            // `info` に落とす —— 他の失敗（DB が閉じている等）や、会話が在るのに落ちた回は
            // 今までどおり `error`（本物の故障を化かさない）。
            Err(err)
                if matches!(err, StoreError::UnknownSession)
                    && matches!(self.store.session_exists(self.session_id).await, Ok(false)) =>
            {
                tracing::info!(
                    error = %err,
                    session_id = self.session_id.get(),
                    "{TURN_VANISHED_MESSAGE}"
                );
            }
            Err(err) => {
                // 応答は既に返した（か、これから返す）。**ここで止めない。**
                // 詳細（`Debug` はパスを含みうる）はログにだけ出す。
                tracing::error!(
                    error = %err,
                    detail = ?err,
                    session_id = self.session_id.get(),
                    "会話の往復を保存できませんでした（応答は返します）"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{StatAxis, StatScale};
    use crate::parse::FallbackReason;
    use crate::testlog::{
        bracketed_field, captured_under_default_filter, has_field, line_containing,
    };

    /// 利用者の発言。**それ自体が `「」` を含む形**にしてある。
    ///
    /// パースすれば台詞と地の文に割れるので、`parse_response` を通す変異が
    /// 「割れる材料が無かった」だけで緑になることがない。
    const USER_MESSAGE: &str = "「ただいま」 と彼は書いた。";

    /// 応答。**台詞と地の文が両方入る**（片方だけだと入れ替えが検出できない）。
    const REPLY: &str = "「おかえりなさい」 彼女は微笑んだ。";

    /// **入れ子の応答**（`「外「内」外」`）。ADR-0014 で崩れから外れた形。
    ///
    /// 外側の台詞・内側の引用・地の文・利用者の発言が**すべて別の文字列**になるように
    /// してある（同値だと取り違えが検出できない / CLAUDE.md「最頻の穴」）。
    const NESTED_REPLY: &str = "「先生は「また明日」とおっしゃったの」 彼女は肩をすくめた。";

    /// [`NESTED_REPLY`] の外側の台詞。**内側の `「」` を含んだまま。**
    const NESTED_DIALOGUE: &str = "先生は「また明日」とおっしゃったの";

    /// [`NESTED_REPLY`] の内側の引用。**台詞本文に残る**（除去も置換もしない）。
    const INNER_QUOTE: &str = "「また明日」";

    /// [`NESTED_REPLY`] の地の文。
    const NESTED_NARRATION: &str = " 彼女は肩をすくめた。";

    /// **地の文に漏れる軸名。** 応答にも利用者の発言にも他の用途で出てこない語。
    const NARRATION_AXIS: &str = "ユーモア";

    /// **台詞に漏れる軸名。** 地の文用と**別の文字列**（同じだと欄の取り違えが検出できない）。
    const DIALOGUE_AXIS: &str = "ロジック";

    /// **値 0 の軸名。** プロンプトに載らないので、応答に出ても数えない（ADR-0018）。
    const ZERO_AXIS: &str = "パッション";

    /// **どこにも漏れない軸名。** 「常に全軸を返す」実装を緑にしないために要る。
    const CLEAN_AXIS: &str = "勇気";

    /// **中間に空白を含む軸名**（作業単位 B-12b）。
    ///
    /// `StatAxis` は前後の空白しか落とさない（`domain::StatAxis::parse`）ので、
    /// **この綴りのまま欄に載る。** つまり `axis_leak_narration=["内なる 情熱"]` は
    /// **空白で割ると値が2つに割れる**——[`has_field`] では読めない。
    ///
    /// **空白を含まない軸名だけでフィクスチャを組むと、緩い突き合わせでも緑になる。**
    /// それはフィクスチャに寄りかかった締め方なので、ここで1本だけ空白を持たせてある。
    const SPACED_AXIS: &str = "内なる 情熱";

    /// **地の文に [`NARRATION_AXIS`]、台詞に [`DIALOGUE_AXIS`] が出る応答。**
    ///
    /// 2つの欄が**別の値**で埋まるので、入れ替える変異がそのまま赤になる。
    const LEAKY_REPLY: &str = "「ロジックだけじゃ足りないよ」 ユーモアたっぷりに笑った。";

    /// **地の文に2軸が漏れる応答**（作業単位 B-12b）。
    ///
    /// 欄は `["内なる 情熱", "ユーモア"]` になる——**軸と軸のあいだにも、
    /// 軸名の中間にも空白が入る形。** 1軸だけの材料では、この状態に一度も入らない。
    ///
    /// 台詞には [`DIALOGUE_AXIS`] だけが出る（**2つの欄が別の値で埋まる**）。
    const TWO_LEAKS_REPLY: &str = "「ロジックだけじゃ足りないよ」 内なる 情熱とユーモアで笑った。";

    /// **値 0 の軸名だけが出る応答。**
    const ZERO_AXIS_REPLY: &str = "「ただいま」 パッションを込めて頷いた。";

    /// **崩れた応答**（`「` が閉じない）に軸名が混ざった形。生出力が丸ごと地の文になる。
    const BROKEN_LEAKY_REPLY: &str = "ユーモアたっぷりに笑った。「そのことは";

    /// フィクスチャの尺度。**既定と違う値**（`tests/common::fixture_scale` と同じ理由）。
    fn fixture_scale() -> StatScale {
        let scale = StatScale::parse(7).expect("正当な尺度であること");
        assert_ne!(
            scale,
            StatScale::DEFAULT,
            "フィクスチャの尺度は既定と違う値であること"
        );
        scale
    }

    /// 軸1本。
    fn alloc(axis: &str, raw: u32) -> StatAlloc {
        StatAlloc {
            axis: StatAxis::parse(axis).expect("正当な軸名であること"),
            value: fixture_scale()
                .value(raw)
                .expect("尺度に収まる値であること"),
        }
    }

    /// キャラクターの性格ステータス。**先頭は漏れない軸、末尾は漏れる軸。**
    ///
    /// * **先頭** ——「先頭の軸しか見ない」実装を緑にしない。
    /// * **末尾** ——**値 0 の軸を末尾に置かないこと。** 末尾が検出対象外だと、
    ///   「一覧の最後の軸を見ない」実装（`parse::leak` の `stats[..len - 1]`）が
    ///   「述語で落とした」結果と区別できない。**実測値は写さない**
    ///   ——`parse::leak` の `tests::stats` の doc に置いてある。
    fn stats() -> Vec<StatAlloc> {
        let axes = [
            NARRATION_AXIS,
            DIALOGUE_AXIS,
            ZERO_AXIS,
            CLEAN_AXIS,
            SPACED_AXIS,
        ];
        for (index, left) in axes.iter().enumerate() {
            for right in &axes[index + 1..] {
                assert_ne!(left, right, "フィクスチャの軸名は互いに別であること");
            }
        }

        let allocs = vec![
            alloc(CLEAN_AXIS, 4),
            // **値 0 の軸は中間に置く**（末尾に置かない理由は上）。
            alloc(ZERO_AXIS, 0),
            // **空白を含む軸**（B-12b）。漏れるのは [`TWO_LEAKS_REPLY`] のときだけで、
            // **[`NARRATION_AXIS`] より前**に置いてある——欄に載る順は渡した順
            // （`AxisLeaks` の doc）なので、期待値の並びもこの位置で決まる。
            alloc(SPACED_AXIS, 3),
            alloc(NARRATION_AXIS, 5),
            alloc(DIALOGUE_AXIS, 2),
        ];

        // **位置についての前提**なので、軸名の `assert_ne!` だけでは守れない。
        let last = allocs.last().expect("軸が1本以上あること");
        assert!(
            last.appears_in_prompt(),
            "末尾に値 0 の軸を置かないこと: {last:?}"
        );
        assert_eq!(
            last.axis.as_str(),
            DIALOGUE_AXIS,
            "末尾は実際に漏れる軸であること"
        );
        assert!(
            LEAKY_REPLY.contains(DIALOGUE_AXIS),
            "前提: 末尾の軸が現に応答へ出ること"
        );

        allocs
    }

    /// 漏れが1件も無い判定結果。
    fn no_leaks() -> AxisLeaks<'static> {
        AxisLeaks::default()
    }

    fn session() -> SessionId {
        SessionId::from_raw_for_tests(42)
    }

    /// **利用者の発言はパースしない。** 生の本文だけが残る。
    #[test]
    fn the_user_message_is_stored_verbatim_and_never_parsed() {
        // 前提: この文字列はパースすれば割れる（＝「割れないから None」ではない）。
        let parsed = parse_response(USER_MESSAGE);
        assert_eq!(parsed.dialogue.as_deref(), Some("ただいま"));

        let stats = stats();

        let turn = turn_messages(USER_MESSAGE, REPLY, &stats);

        assert_eq!(turn.user.role, Role::User);
        assert_eq!(turn.user.raw_content, USER_MESSAGE);
        assert_eq!(
            turn.user.dialogue, None,
            "モデルの出力ではないものに parse_response を通さないこと"
        );
        assert_eq!(turn.user.narration, None);
    }

    /// **応答は分解され、生出力も丸ごと残る。**
    #[test]
    fn the_reply_is_split_and_keeps_its_raw_output() {
        let stats = stats();
        let turn = turn_messages(USER_MESSAGE, REPLY, &stats);

        assert_eq!(turn.reply.role, Role::Assistant);
        assert_eq!(turn.reply.dialogue.as_deref(), Some("おかえりなさい"));
        assert_eq!(turn.reply.narration.as_deref(), Some(" 彼女は微笑んだ。"));
        assert_eq!(
            turn.reply.raw_content, REPLY,
            "`raw_content` に分解結果を入れないこと（絶対制約）"
        );
        assert_eq!(turn.outcome, ParseOutcome::Structured);
    }

    /// **往復の形は `利用者 → 応答` で固定。** 2件は別の本文を持つ。
    ///
    /// 引数を取り違えると `Store::append_turn` が `InvalidTurn` で弾くが、
    /// 「弾かれる形を組み立てていないこと」はここで見る。
    #[test]
    fn the_pair_is_a_user_turn_followed_by_an_assistant_turn() {
        assert_ne!(USER_MESSAGE, REPLY, "前提: 2つは別の文字列であること");

        let stats = stats();

        let turn = turn_messages(USER_MESSAGE, REPLY, &stats);

        assert_eq!(turn.user.role, Role::User);
        assert_eq!(turn.reply.role, Role::Assistant);
        assert_ne!(turn.user.role, turn.reply.role);
        assert_eq!(turn.user.raw_content, USER_MESSAGE);
        assert_eq!(turn.reply.raw_content, REPLY);
    }

    /// **入れ子の回でも台詞が失われない**（ADR-0014 の実利 / `parse` の外での観測点）。
    ///
    /// `parse` 側は入れ子を [`ParseOutcome::StructuredNested`] として正常系に置いているが、
    /// **それが保存する2件に届いていること**を見ているのはここだけ。
    /// 実測で、`dialogue` を入れ子の回だけ `None` に落とす変異は
    /// このテストを足すまで **699/699 緑**で通り抜けた。
    #[test]
    fn a_nested_reply_keeps_its_dialogue_and_is_marked_as_nested() {
        // 前提: 外側の台詞・内側の引用・利用者の発言がすべて別の文字列であること。
        assert_ne!(NESTED_DIALOGUE, INNER_QUOTE);
        assert_ne!(NESTED_DIALOGUE, USER_MESSAGE);
        assert_ne!(INNER_QUOTE, USER_MESSAGE);
        assert_ne!(NESTED_REPLY, REPLY);

        let stats = stats();

        let turn = turn_messages(USER_MESSAGE, NESTED_REPLY, &stats);

        assert_eq!(
            turn.reply.dialogue.as_deref(),
            Some(NESTED_DIALOGUE),
            "入れ子の回でも台詞を落とさないこと（ADR-0014 の実利そのもの）"
        );
        assert!(
            NESTED_DIALOGUE.contains(INNER_QUOTE),
            "前提: 内側の `「」` は台詞本文に残る形であること"
        );
        assert_eq!(turn.reply.narration.as_deref(), Some(NESTED_NARRATION));
        assert_eq!(
            turn.reply.raw_content, NESTED_REPLY,
            "`raw_content` に分解結果を入れないこと（絶対制約）"
        );
        assert_eq!(
            turn.outcome,
            ParseOutcome::StructuredNested,
            "入れ子を `Structured` に混ぜないこと（発生率を追えなくなる）"
        );
        assert!(
            !turn.outcome.fell_back(),
            "入れ子は崩れではない（ADR-0014）"
        );

        // 利用者の発言はここでも素通し（入れ子の回だけ扱いが変わらないこと）。
        assert_eq!(turn.user.raw_content, USER_MESSAGE);
        assert_eq!(turn.user.dialogue, None);
    }

    /// **崩れた応答でも `raw_content` は生出力そのもの。**
    #[test]
    fn a_broken_reply_keeps_the_whole_raw_output() {
        const BROKEN: &str = "彼女は言った。「おかえりなさい";

        let stats = stats();

        let turn = turn_messages(USER_MESSAGE, BROKEN, &stats);

        assert_eq!(turn.reply.raw_content, BROKEN);
        assert_eq!(turn.reply.dialogue, None, "崩れたら台詞を拾わないこと");
        assert_eq!(
            turn.reply.narration.as_deref(),
            Some(BROKEN),
            "フォールバックは生出力を丸ごと地の文にする"
        );
        assert_eq!(
            turn.outcome,
            ParseOutcome::Fallback(FallbackReason::Unclosed)
        );
    }

    /// **空の応答も往復として組み立てる**（`raw_content` は空文字のまま）。
    ///
    /// 「LLM が空を返した」こと自体が調査材料なので、握り潰さない
    /// （`domain::NewMessage` の doc）。
    #[test]
    fn an_empty_reply_is_still_a_turn() {
        let stats = stats();
        let turn = turn_messages(USER_MESSAGE, "", &stats);

        assert_eq!(turn.reply.raw_content, "");
        assert_eq!(turn.reply.dialogue, None);
        assert_eq!(turn.reply.narration, None);
        assert_eq!(turn.outcome, ParseOutcome::Empty);
        assert!(!turn.outcome.fell_back(), "空は崩れではない");
    }

    /// **分解の結果は既定のログフィルタで記録される**（パース成功率 / 絶対制約）。
    ///
    /// 載るのは `ParseOutcome::label()` の定数だけで、**生出力は載らない。**
    #[test]
    fn the_outcome_is_recorded_under_the_default_log_filter() {
        let output = captured_under_default_filter(|| {
            log_outcome(
                ParseOutcome::Structured,
                0,
                ThinkingTally::default(),
                &no_leaks(),
                session(),
                EmptyRetry::default(),
            );
        });

        // **欄は分解の記録の行の中で見る**（作業単位 B-2 / `crate::testlog` の doc）。
        let line = line_containing(&output, "応答を分解しました");
        assert!(has_field(line, "outcome=structured"), "{line:?}");
        assert!(
            has_field(line, "fell_back=false"),
            "成功率の分子になる値を載せること: {line:?}"
        );
        assert!(has_field(line, "session_id=42"), "{line:?}");
    }

    /// **6つの値がすべて、それぞれのラベルと `fell_back` で記録される。**
    ///
    /// 上の2本は `Structured` と `Fallback(Unclosed)` の**2値しか通していなかった**ので、
    /// 残り4つは「[`log_outcome`] が何を書いても」誰も見ていなかった。実測で、
    /// 入れ子のときだけラベルを `structured` へ潰す変異（ADR-0014 の決定2 ——
    /// 入れ子の発生率を追える —— が丸ごと無効になる）は **699/699 緑**で通り抜けた。
    ///
    /// 期待するラベルは**テスト側に定数で持つ。** `outcome.label()` を呼ぶと
    /// 実装の写しになり、ラベルを入れ替える変異が両側で同時に動いて緑になる。
    ///
    /// ラベルの照合は [`has_field`]（トークン一致）で行う
    /// ——`contains` だと `structured` の主張が `structured_nested` の行で満たされる。
    #[test]
    fn every_outcome_is_recorded_under_its_own_label() {
        let cases = [
            (ParseOutcome::Structured, "structured", false),
            (ParseOutcome::StructuredNested, "structured_nested", false),
            (ParseOutcome::NarrationOnly, "narration_only", false),
            (ParseOutcome::Empty, "empty", false),
            (
                ParseOutcome::Fallback(FallbackReason::Unclosed),
                "fallback_unclosed",
                true,
            ),
            (
                ParseOutcome::Fallback(FallbackReason::StrayClose),
                "fallback_stray_close",
                true,
            ),
        ];

        for (outcome, label, fell_back) in cases {
            let output = captured_under_default_filter(|| {
                log_outcome(
                    outcome,
                    0,
                    ThinkingTally::default(),
                    &no_leaks(),
                    session(),
                    EmptyRetry::default(),
                );
            });

            // **3つの欄が同じ1行に載ること**まで見る（作業単位 B-2）。
            let line = line_containing(&output, "応答を分解しました");
            assert!(
                has_field(line, &format!("outcome={label}")),
                "{outcome:?} は {label:?} で記録すること: {line:?}"
            );
            assert!(
                has_field(line, &format!("fell_back={fell_back}")),
                "{outcome:?} の `fell_back` は {fell_back} であること: {line:?}"
            );
            assert!(
                has_field(line, "session_id=42"),
                "どの値でもセッションを載せること: {line:?}"
            );
        }
    }

    /// **崩れた回は理由まで区別できる**（内訳の集計キー）。
    #[test]
    fn a_fallback_is_recorded_with_its_reason() {
        let output = captured_under_default_filter(|| {
            log_outcome(
                ParseOutcome::Fallback(FallbackReason::Unclosed),
                0,
                ThinkingTally::default(),
                &no_leaks(),
                session(),
                EmptyRetry::default(),
            );
        });

        // **欄は分解の記録の行の中で見る**（作業単位 B-2）。
        let line = line_containing(&output, "応答を分解しました");
        assert!(has_field(line, "outcome=fallback_unclosed"), "{line:?}");
        assert!(
            has_field(line, "fell_back=true"),
            "崩れた回を成功に数えないこと: {line:?}"
        );
        // 成功した回とラベルが分かれていること（同じなら成功率が計算できない）。
        //
        // **否定は塊のまま見る**（どの行にも載っていないことを主張したい /
        // [`crate::testlog::SharedBuffer::contents`] の doc）。
        // **ただし素の `contains` ではなく [`has_field`]**（作業単位 B-21d）——
        // 「**このキーの値が、この特定の値ではない**」を言うときは、
        // `contains` が **`x_outcome=structured` のようにキーの側が伸びた別の欄**を掴んで
        // **実装は正しいのに落ちる**（[`has_field`] の doc の表 / E-2 で実際に起きた形）。
        assert!(!has_field(&output, "outcome=structured"), "{output:?}");
    }

    // ---- 軸名の漏れ（ADR-0018 / 作業単位 0011c）----

    /// 漏れた軸が載った欄の**値の綴り**。**期待値はテスト側に持つ**（実装の写しにしない）。
    ///
    /// `Debug` の列なので `["A", "B"]`——**区切りにも軸名の中間にも空白が入りうる。**
    /// だから欄の照合には [`has_field`] ではなく [`bracketed_field`] を使う
    /// （作業単位 B-12b。**空の `[]` だけは空白が入らないので [`has_field`] で読める**）。
    fn axes_field(axes: &[&str]) -> String {
        let quoted: Vec<String> = axes.iter().map(|axis| format!("{axis:?}")).collect();
        format!("[{}]", quoted.join(", "))
    }

    /// **地の文と台詞の漏れが、それぞれの欄に載る**（ADR-0018）。
    ///
    /// **2つの欄は別の軸名で埋まる**ので、入れ替える変異がそのまま赤になる。
    /// **同じ1行**に載ることも見る（別イベントにすると率を出すのに突き合わせが要る）。
    #[test]
    fn a_leaked_axis_is_recorded_in_the_field_that_matches_where_it_appeared() {
        assert_ne!(
            NARRATION_AXIS, DIALOGUE_AXIS,
            "前提: 2つの欄が別の値で埋まること"
        );

        let stats = stats();
        let turn = turn_messages(USER_MESSAGE, LEAKY_REPLY, &stats);
        // 前提: 材料が地の文と台詞の両方に軸名を持つこと。
        assert_eq!(turn.leaks.narration(), [NARRATION_AXIS]);
        assert_eq!(turn.leaks.dialogue(), [DIALOGUE_AXIS]);

        let output = captured_under_default_filter(|| {
            log_outcome(
                turn.outcome,
                turn.emphasis_pairs_stripped,
                turn.thinking,
                &turn.leaks,
                session(),
                EmptyRetry::default(),
            );
        });

        // **欄は「`キー=` に続く閉じ括弧まで」を1つの値として読む**（作業単位 B-12b）。
        // `contains` では**2つとも部分文字列として当たって**しまい、実測で
        // **欄名に接頭辞を足す変異**（`x_axis_leak_narration=…`）も
        // **値の末尾に1文字足す変異**（`[…]!`）もここを通り抜けた。
        let line = line_containing(&output, "応答を分解しました");
        assert_eq!(
            bracketed_field(line, "axis_leak_narration"),
            Some(axes_field(&[NARRATION_AXIS]).as_str()),
            "地の文の漏れを地の文の欄に載せること: {line:?}"
        );
        assert_eq!(
            bracketed_field(line, "axis_leak_dialogue"),
            Some(axes_field(&[DIALOGUE_AXIS]).as_str()),
            "台詞の漏れを台詞の欄に載せること: {line:?}"
        );
        assert!(
            has_field(line, "session_id=42"),
            "どのセッションかも同じ行に載ること: {line:?}"
        );
    }

    /// **2軸が漏れた欄も、閉じ括弧までを1つの値として読める**（作業単位 B-12b）。
    ///
    /// # このテストだけが作る「状態」
    ///
    /// | 状態 | なぜ要るのか |
    /// |---|---|
    /// | **軸名の中間に空白**（[`SPACED_AXIS`]） | `StatAxis` は中間の空白を保つ。1軸でも欄の値に空白が入る |
    /// | **1つの欄に2軸**（`["A", "B"]`） | **区切りに必ず空白が入る**。1軸だけの材料では一度も通らない |
    ///
    /// どちらも [`has_field`] では読めない形で、**B-12b の前はこの状態に
    /// 一度も入っていなかった**——だから「空白を含まない軸名が1件」という
    /// フィクスチャに寄りかかったまま、緩い突き合わせが緑を通していた。
    ///
    /// **並びも固定する**（`AxisLeaks` は渡された順を保つ）。
    #[test]
    fn a_field_with_several_leaked_axes_is_read_as_one_value() {
        // **前提: `StatAxis` は中間の空白を保つ。** 落とす型なら、この道具は要らない。
        assert!(
            SPACED_AXIS.contains(' '),
            "前提: 軸名が中間に空白を含むこと: {SPACED_AXIS:?}"
        );
        assert_eq!(
            StatAxis::parse(SPACED_AXIS)
                .expect("正当な軸名であること")
                .as_str(),
            SPACED_AXIS,
            "前提: 正規化で中間の空白が落ちないこと（domain::StatAxis::parse）"
        );

        let stats = stats();
        let turn = turn_messages(USER_MESSAGE, TWO_LEAKS_REPLY, &stats);

        // 前提: 地の文に2軸、台詞に1軸。**3つとも別の軸名。**
        assert_eq!(
            turn.leaks.narration(),
            [SPACED_AXIS, NARRATION_AXIS],
            "前提: 地の文に2軸が漏れること"
        );
        assert_eq!(
            turn.leaks.dialogue(),
            [DIALOGUE_AXIS],
            "前提: 台詞に漏れるのは1軸だけであること"
        );

        let output = captured_under_default_filter(|| {
            log_outcome(
                turn.outcome,
                turn.emphasis_pairs_stripped,
                turn.thinking,
                &turn.leaks,
                session(),
                EmptyRetry::default(),
            );
        });

        let line = line_containing(&output, "応答を分解しました");
        assert_eq!(
            bracketed_field(line, "axis_leak_narration"),
            Some(axes_field(&[SPACED_AXIS, NARRATION_AXIS]).as_str()),
            "2軸を1つの値として地の文の欄に載せること: {line:?}"
        );
        assert_eq!(
            bracketed_field(line, "axis_leak_dialogue"),
            Some(axes_field(&[DIALOGUE_AXIS]).as_str()),
            "台詞の欄は1軸のままであること: {line:?}"
        );
    }

    /// **漏れが無いターンでも欄は出る**（空であって欠落ではない）。
    ///
    /// 出さないと**「欄が無い」と「0件」が区別できず**、率の分母が作れない。
    #[test]
    fn a_turn_without_a_leak_still_carries_both_fields() {
        let stats = stats();
        // 前提: 軸は現に渡っている（軸が無いから空なのではない）。
        assert!(!stats.is_empty());
        let turn = turn_messages(USER_MESSAGE, REPLY, &stats);
        assert!(turn.leaks.is_empty(), "前提: この応答は漏れないこと");

        let output = captured_under_default_filter(|| {
            log_outcome(
                turn.outcome,
                turn.emphasis_pairs_stripped,
                turn.thinking,
                &turn.leaks,
                session(),
                EmptyRetry::default(),
            );
        });

        // **空の欄もトークンとして突き合わせる**（作業単位 B-12）——`contains` だと
        // `axis_leak_narration=[]` が伸びた別の綴りにも当たる。
        // **0 件をどちらの道具で読むかの理由は、`crate::testlog::bracketed_field` の doc**
        // （作業単位 B-21b の節）に1本だけ置いてある。**ここに書き写さないこと。**
        // 要点だけ: **`[]` に限れば両者の受理集合は同値**で、こちらは同じファイルの
        // 他の欄の読み方に揃えて `has_field` を使っている。
        // 1件以上載る側は `["A", "B"]` と空白を含むので `has_field` では読めない。そちらは
        // `bracketed_field` で読む / 下の
        // [`a_field_with_several_leaked_axes_is_read_as_one_value`] 参照。
        //
        // [`a_field_with_several_leaked_axes_is_read_as_one_value`]:
        //     a_field_with_several_leaked_axes_is_read_as_one_value
        let line = line_containing(&output, "応答を分解しました");
        assert!(
            has_field(line, "axis_leak_narration=[]"),
            "漏れが無くても地の文の欄を出すこと: {line:?}"
        );
        assert!(
            has_field(line, "axis_leak_dialogue=[]"),
            "漏れが無くても台詞の欄を出すこと: {line:?}"
        );
    }

    /// **値 0 の軸はログにも出ない**（プロンプトに載っていないので読まれていない）。
    ///
    /// CLAUDE.md の絶対制約（0 の軸はプロンプトに載せない）と**対**の見張り番。
    #[test]
    fn a_zero_valued_axis_never_reaches_the_log() {
        let stats = stats();
        assert!(
            ZERO_AXIS_REPLY.contains(ZERO_AXIS),
            "前提: 応答に 0 の軸名が現に出ること"
        );

        let turn = turn_messages(USER_MESSAGE, ZERO_AXIS_REPLY, &stats);
        let output = captured_under_default_filter(|| {
            log_outcome(
                turn.outcome,
                turn.emphasis_pairs_stripped,
                turn.thinking,
                &turn.leaks,
                session(),
                EmptyRetry::default(),
            );
        });

        let line = line_containing(&output, "応答を分解しました");
        assert!(
            has_field(line, "axis_leak_narration=[]") && has_field(line, "axis_leak_dialogue=[]"),
            "プロンプトに載らなかった軸を数えないこと: {line:?}"
        );
        assert!(
            !output.contains(ZERO_AXIS),
            "0 の軸名をログに出さないこと: {output:?}"
        );
    }

    /// **崩れたターンでも判定は動く。** 生出力が丸ごと地の文なので、地の文の欄に載る。
    #[test]
    fn a_fallback_turn_is_still_inspected_for_leaks() {
        let stats = stats();
        let turn = turn_messages(USER_MESSAGE, BROKEN_LEAKY_REPLY, &stats);
        assert!(
            turn.outcome.fell_back(),
            "前提: この応答は崩れとして扱われること: {:?}",
            turn.outcome
        );

        let output = captured_under_default_filter(|| {
            log_outcome(
                turn.outcome,
                turn.emphasis_pairs_stripped,
                turn.thinking,
                &turn.leaks,
                session(),
                EmptyRetry::default(),
            );
        });

        let line = line_containing(&output, "応答を分解しました");
        assert!(
            has_field(line, "outcome=fallback_unclosed"),
            "前提: 崩れとして記録されること: {line:?}"
        );
        // **崩れた回でも欄の読み方は同じ**（作業単位 B-12b / 上の doc）。
        assert_eq!(
            bracketed_field(line, "axis_leak_narration"),
            Some(axes_field(&[NARRATION_AXIS]).as_str()),
            "崩れた回も判定の対象から外さないこと: {line:?}"
        );
        assert!(
            has_field(line, "axis_leak_dialogue=[]"),
            "崩れた回に台詞は無い（生出力が丸ごと地の文になる）: {line:?}"
        );
    }

    /// **ログに応答本文を載せない。** 載るのは軸名（運用者が書いた値）だけ。
    #[test]
    fn the_leak_fields_never_carry_the_response_text() {
        /// 応答のうち**軸名ではない**断片。1文字もログに出てはいけない。
        const NOT_AN_AXIS: &str = "だけじゃ足りないよ";
        /// 地の文のうち**軸名ではない**断片。
        const NOT_AN_AXIS_NARRATION: &str = "たっぷりに笑った";

        assert!(
            LEAKY_REPLY.contains(NOT_AN_AXIS) && LEAKY_REPLY.contains(NOT_AN_AXIS_NARRATION),
            "前提: どちらも応答の一部であること"
        );

        let stats = stats();
        let turn = turn_messages(USER_MESSAGE, LEAKY_REPLY, &stats);
        let output = captured_under_default_filter(|| {
            log_outcome(
                turn.outcome,
                turn.emphasis_pairs_stripped,
                turn.thinking,
                &turn.leaks,
                session(),
                EmptyRetry::default(),
            );
        });

        assert!(
            !output.contains(NOT_AN_AXIS) && !output.contains(NOT_AN_AXIS_NARRATION),
            "運用ログを会話の写しにしないこと: {output:?}"
        );
        assert!(
            !output.contains(USER_MESSAGE),
            "利用者の発言も載せないこと: {output:?}"
        );
    }

    /// **判定は保存する2件を1バイトも変えない**（ADR-0018「記録だけ。応答には一切影響させない」）。
    ///
    /// 軸を渡した場合と渡さなかった場合で、`user` / `reply` / `outcome` が完全に一致すること。
    #[test]
    fn the_detection_changes_nothing_that_gets_saved() {
        let stats = stats();
        let with = turn_messages(USER_MESSAGE, LEAKY_REPLY, &stats);
        let without = turn_messages(USER_MESSAGE, LEAKY_REPLY, &[]);

        assert!(
            !with.leaks.is_empty() && without.leaks.is_empty(),
            "前提: 判定結果は2つで違うこと（同じなら比較に意味が無い）"
        );
        assert_eq!(with.user, without.user, "利用者の発言を書き換えないこと");
        assert_eq!(with.reply, without.reply, "応答を書き換えないこと");
        assert_eq!(with.outcome, without.outcome);
        assert_eq!(
            with.reply.raw_content, LEAKY_REPLY,
            "生出力を丸ごと残すこと（絶対制約）"
        );
    }

    // ---- 上流が漏らした思考（ADR-0053 / 作業単位 B-61）----

    /// **思考の中で軸名を口にし、後ろに本文が続く応答**（C-20 の多数派の形を縮めたもの）。
    ///
    /// 思考の中に [`NARRATION_AXIS`] を置いてある —— **剥がさずに分解すると、
    /// 生出力が地の文に混ざって軸名の漏れとして数えられる。** 剥がせば数えない。
    /// 綴りは**空白入りの閉じタグ**（personacore の無害化が作る形 / ADR-0053 §背景）。
    const THINKING_REPLY: &str = "<think>\nThe user is home. ユーモア is high.\n</think >\n\n「おかえりなさい」 彼女は微笑んだ。";

    /// **思考だけで本文が無い応答**（B+L 5）。
    const THINKING_ONLY_REPLY: &str = "<think >\nShe lives with her dog.\n</think >";

    /// **閉じない思考**（ADR-0053 §決定3。測定では 0 件だが、起きたらログで分かる形）。
    const UNCLOSED_THINKING_REPLY: &str = "<think>\nThe user is asking";

    /// **思考2つと余った閉じタグ**（B の turn 32 の形）。
    const TWO_THOUGHTS_REPLY: &str = "<think >First.</think >\n\n<think >Second.</think >\n</think>\n\n「おかえりなさい」 彼女は微笑んだ。";

    /// **保存は思考込み、分解は思考なし**——同じ回で両方を見る（ADR-0053 §決定4）。
    ///
    /// 期待値は [`REPLY`]（思考を持たない同じ本文）を分解した結果と、手で書いた値の両方で押さえる。
    #[test]
    fn a_leaked_thought_is_stripped_before_splitting_but_kept_in_the_raw_output() {
        // 前提: 思考の後ろの本文は REPLY と同じで、思考は軸名を含む。
        assert!(THINKING_REPLY.ends_with(REPLY), "前提: 本文は REPLY と同じ");
        assert!(THINKING_REPLY.contains(NARRATION_AXIS), "前提: 思考に軸名");

        let stats = stats();
        let turn = turn_messages(USER_MESSAGE, THINKING_REPLY, &stats);
        let clean = turn_messages(USER_MESSAGE, REPLY, &stats);

        // **保存する生出力は思考込み**（絶対制約）。
        assert_eq!(
            turn.reply.raw_content, THINKING_REPLY,
            "`raw_content` に剥がした本文を入れないこと（絶対制約）"
        );
        // **分解した本文は思考なし。**
        assert_eq!(turn.reply.dialogue.as_deref(), Some("おかえりなさい"));
        assert_eq!(turn.reply.narration.as_deref(), Some(" 彼女は微笑んだ。"));
        assert_eq!(turn.reply.dialogue, clean.reply.dialogue);
        assert_eq!(turn.reply.narration, clean.reply.narration);
        assert_eq!(turn.outcome, ParseOutcome::Structured);
        // **思考の中の軸名を漏れとして数えない**（判定は剥がした本文の分解で行う）。
        assert!(
            turn.leaks.is_empty(),
            "思考の中の語を漏れに数えないこと: {:?}",
            turn.leaks
        );
        assert_eq!(
            turn.thinking,
            ThinkingTally {
                blocks: 1,
                unclosed: false
            }
        );
        // 対照: 思考の無い回は 0。
        assert_eq!(clean.thinking, ThinkingTally::default());
    }

    /// **思考だけの回は空として分解され、生出力は思考込みで残る。**
    #[test]
    fn a_reply_that_was_only_thought_is_split_as_empty() {
        let stats = stats();
        let turn = turn_messages(USER_MESSAGE, THINKING_ONLY_REPLY, &stats);

        assert_eq!(turn.reply.raw_content, THINKING_ONLY_REPLY);
        assert_eq!(turn.reply.dialogue, None);
        assert_eq!(turn.reply.narration, None);
        assert_eq!(turn.outcome, ParseOutcome::Empty);
        assert_eq!(
            turn.thinking,
            ThinkingTally {
                blocks: 1,
                unclosed: false
            }
        );
    }

    /// **閉じない思考は末尾まで捨て、空として分解する**（生出力へ戻さない / §決定3）。
    #[test]
    fn an_unclosed_thought_is_split_as_empty_and_marked_unclosed() {
        let stats = stats();
        let turn = turn_messages(USER_MESSAGE, UNCLOSED_THINKING_REPLY, &stats);

        assert_eq!(turn.reply.raw_content, UNCLOSED_THINKING_REPLY);
        assert_eq!(turn.reply.narration, None, "生出力を地の文へ戻さないこと");
        assert_eq!(turn.outcome, ParseOutcome::Empty);
        assert_eq!(
            turn.thinking,
            ThinkingTally {
                blocks: 1,
                unclosed: true
            }
        );
    }

    /// **剥がした数と閉じなかったかが、分解の記録と同じ1行に載る**（§決定5）。
    ///
    /// 3つの回（2ブロック・閉じない・思考なし）を**それぞれ別の値**で見る。
    /// 欄は [`has_field`]（トークン一致）で読む —— `contains` だと `=2` が `=20` にも当たる。
    #[test]
    fn the_stripped_thought_is_recorded_in_the_parse_line() {
        let stats = stats();

        for (reply, blocks, unclosed) in [
            (TWO_THOUGHTS_REPLY, 2, false),
            (UNCLOSED_THINKING_REPLY, 1, true),
            // **0 の回も欄を出す**（出さないと分母が作れない）。
            (REPLY, 0, false),
        ] {
            let turn = turn_messages(USER_MESSAGE, reply, &stats);
            let output = captured_under_default_filter(|| {
                log_outcome(
                    turn.outcome,
                    turn.emphasis_pairs_stripped,
                    turn.thinking,
                    &turn.leaks,
                    session(),
                    EmptyRetry::default(),
                );
            });

            let line = line_containing(&output, "応答を分解しました");
            assert!(
                has_field(line, &format!("thinking_blocks_stripped={blocks}")),
                "{reply:?}: 剥がしたブロックの数を同じ行に載せること: {line:?}"
            );
            assert!(
                has_field(line, &format!("thinking_unclosed={unclosed}")),
                "{reply:?}: 閉じなかったかを同じ行に載せること: {line:?}"
            );
            assert!(has_field(line, "session_id=42"), "{line:?}");
            // **思考の中身をログに出さない**（運用ログを会話の写しにしない）。
            assert!(
                !output.contains("First.") && !output.contains("The user"),
                "思考の中身をログに出さないこと: {output:?}"
            );
        }
    }

    // ---- 空の本文の引き直し（ADR-0059 §決定3 / 作業単位 A-18b）----

    /// **引き直しの欄が、分解の記録と同じ1行に載る。0 の回も出す**（出さないと分母が作れない）。
    ///
    /// 3つの回（引き直さなかった・捨てた・2回目が失敗した）を**それぞれ別の値**で見る。
    /// 捨てた回の文字数（11）と生成数（57）は互いにも、ほかの欄の値とも違えてある。
    #[test]
    fn the_empty_retry_is_recorded_in_the_parse_line() {
        let discarded = EmptyRetry::discarded(
            "<think>考え中のまま",
            Some(crate::llm::TokenUsage {
                prompt_tokens: 900,
                completion_tokens: Some(57),
            }),
        );
        for (retry, expected) in [
            (
                EmptyRetry::default(),
                [
                    "empty_retries=0",
                    "empty_retry_failed=false",
                    "discarded_reply_chars=0",
                    "discarded_reply_unclosed=false",
                    "discarded_completion_tokens=None",
                ],
            ),
            (
                discarded,
                [
                    "empty_retries=1",
                    "empty_retry_failed=false",
                    "discarded_reply_chars=13",
                    "discarded_reply_unclosed=true",
                    "discarded_completion_tokens=Some(57)",
                ],
            ),
            (
                EmptyRetry::failed(),
                [
                    "empty_retries=1",
                    "empty_retry_failed=true",
                    "discarded_reply_chars=0",
                    "discarded_reply_unclosed=false",
                    "discarded_completion_tokens=None",
                ],
            ),
        ] {
            let output = captured_under_default_filter(|| {
                log_outcome(
                    ParseOutcome::Structured,
                    0,
                    ThinkingTally::default(),
                    &no_leaks(),
                    session(),
                    retry,
                );
            });
            let line = line_containing(&output, "応答を分解しました");
            for field in expected {
                assert!(has_field(line, field), "{retry:?}: {field}: {line:?}");
            }
            assert!(has_field(line, "session_id=42"), "{line:?}");
        }
        // 前提: 捨てた回の文字数は材料の文字数（バイト数ではない）。
        assert_eq!("<think>考え中のまま".chars().count(), 13);
    }
}
