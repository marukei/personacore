//! **本文が空なら、最大2回まで引き直す**（[ADR-0059](../../../docs/adr/0059-thinking-and-response-room-in-the-upstream-profile.md) §決定3 /
//! 追記（2026-11-13）の決定1〜5 / 作業単位 A-18b・A-18c）。
//!
//! # 何を空とみなすか
//!
//! 本文から漏れた思考を剥がした（ADR-0053）後が空か空白だけの回（`domain::reply_body`）。
//! 空の `「」` だけ・宙に浮いた強調の印だけは、分解では `ParseOutcome::Empty` になるが、
//! **上流が文字を返した回なので対象にしない。** **上流も面も選ばない**（思考を入れた面に限らない）。
//!
//! | 経路 | 閉じない思考を剥がして空になった回 | それ以外の空 |
//! |---|---|---|
//! | 非ストリーミング（[`complete_again_if_empty`]） | **引き直す** | 引き直す |
//! | ストリーミング（`super::relay`） | **引き直さない** —— 続きとして流すと、積み上げた本文が「閉じない思考＋次の回」になり、ADR-0053 §決定2 の規則で次の回まで捨てられる | 引き直す |
//!
//! # 何回まで送るか（[`EmptyRetry::has_room`]）
//!
//! * **引き直しは最大2回**（1回目と合わせて最大3回）。本文が空でない回が返った時点でやめる。最後の回も空なら空として保存する。
//!   どの回も**同じ要求を同じ上流（1回目と同じクライアントの写し）へ**送る。つまみにしない。
//! * **上限（`finish_reason: length`）で切れた回が1つでもあれば、そのターンの引き直しは合わせて1回まで**
//!   （`stop`→`length` でも `length`→`stop` でも3回目は送らない）。同じ `max_tokens` で送り直すと、また切れうる。
//!   `length` かは**その回の `finish_reason`**（中継で2本以上届いた回は、保留に足した後のもの）で決める。
//!   `finish_reason` が無い回は `length` ではない。
//!
//! # `finish_reason` が無いまま終わった回（[`EmptyRetry::note_ending`]）
//!
//! **`warn` で言う**（[`FINISH_MISSING_LINE`]。引き直すか・本文の有無に関わらず、両方の経路で）。分解の行の
//! `upstream_finish_missing` にも立てる。`finish_reason` がある回では言わない。
//! 中継の `[DONE]` を受けずに尽きた回（`llm` 層の `Truncated`）は「終わった回」ではない（途中で壊れた回）ので、ここを通らない。
//!
//! # 引き直しが失敗したら
//!
//! **その時点で引き直しを止め、その手前の空の回を空として保存する**（2回目で失敗すれば1回目、3回目で失敗すれば2回目）。
//! 失敗は `warn` に出す（[`log_retry_failure`]）。応答の成否は変えない —— 手前の回は上流が正常に返した回である。
//!
//! # 使用量と保存
//!
//! * 答え合わせ（ADR-0013 第3層）と封の清算（ADR-0052）は**採用した回の `usage` で1回だけ**（呼び出し側）。
//! * `raw_content` には**採用した回の生の出力**を保存する。捨てた回は保存せず、その長さ（合計）・閉じない思考だったか（どれか）・
//!   生成トークン数（すべてが返したときだけ合計）を、分解の行（`super::turn`）の欄に出す（[`EmptyRetry`]）。

use crate::domain::{ReplyBody, reply_body};
use crate::llm::{ChatRequest, Completion, FinishReason, LlmClient, LlmError, TokenUsage};

/// 引き直しの要求が失敗したことを言う行（`warn`）。**テストが綴りを照合する。**
pub(super) const RETRY_FAILED_LINE: &str =
    "空の本文の引き直しに失敗しました（手前の空の回のまま保存します）";

/// 上流が `finish_reason` を返さないまま応答を終えたことを言う行（`warn` / 作業単位 A-18c）。**テストが綴りを照合する。**
pub(super) const FINISH_MISSING_LINE: &str = "上流が finish_reason を返さないまま応答を終えました";

/// **引き直しの最大回数**（1回目と合わせて最大3回 / ADR-0059 追記（2026-11-13）の決定1）。つまみにしない。
const MAX_RETRIES: u8 = 2;

/// **上限（`length`）で切れた回があるターンの、引き直しの最大回数**（同 決定1）。
const MAX_RETRIES_AFTER_LENGTH: u8 = 1;

/// 応答の受け取り方。**閉じない思考の回を引き直すかと、ログの欄だけが違う**（モジュールの doc の表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Delivery {
    /// 応答を最後まで受け取ってから保存する（`LlmClient::complete`）。
    Complete,
    /// 受けたそばから利用者へ流す（`LlmClient::stream` / `super::relay`）。
    Stream,
}

/// **この本文は、引き直す空か**（モジュールの doc の表）。純粋関数。**回数の上限は見ない**（[`EmptyRetry::has_room`]）。
pub(super) fn retries(raw: &str, delivery: Delivery) -> bool {
    match reply_body(raw) {
        ReplyBody::Present => false,
        ReplyBody::Blank { unclosed } => !(unclosed && delivery == Delivery::Stream),
    }
}

/// **引き直しの記録**（分解の行の欄になる / `super::turn::log_outcome`）。ターンの回を重ねるごとに積み上げる。
///
/// 引き直さなかった回は [`EmptyRetry::default`]（全部 0 / `false` / `None`）—— **0 の回も欄を出す**
/// （出さないと分母が作れない）。**引き直しが0回か1回だけのターンでは、A-18b の5つの欄と同じ値になる**（決定5）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct EmptyRetry {
    /// **送った引き直しの回数**（0〜2）。開けなかった回も数える。
    retries: u8,
    /// 引き直しの**どれかが開けなかった**か（開けなかった時点で止まるので、最後に送ったもの）。
    failed: bool,
    /// 捨てた回の数（生成数の合計の分母）。`retries` から、開けなかった1回を引いた数。
    discarded: u8,
    /// 捨てた回すべての生の出力の文字数の**合計**（思考込み）。捨てていなければ 0。
    discarded_chars: usize,
    /// 捨てた回の**どれか**が、閉じない思考を剥がして空になった回か。
    discarded_unclosed: bool,
    /// 捨てた回すべての生成トークン数の**合計**。捨てた回が無い、または**1回でも返さなかったら `None`**
    /// （一部だけの合計は、欠けたことが読めないので出さない）。
    discarded_completion_tokens: Option<u32>,
    /// このターンのどれかの回が、上限（`length`）で切れたか。
    cut_by_length: bool,
    /// このターンのどれかの回が、`finish_reason` を返さないまま終わったか（`upstream_finish_missing`）。
    finish_missing: bool,
}

impl EmptyRetry {
    /// **その回の終わりを記録する**（上流が正常に終えた回ごとに1回。採用する回も捨てる回も）。
    ///
    /// `finish_reason` が無ければ [`FINISH_MISSING_LINE`] を `warn` で言う。`length` なら、以後の引き直しを1回までにする。
    pub(super) fn note_ending(&mut self, finish: Option<FinishReason>, delivery: Delivery) {
        match finish {
            None => {
                self.finish_missing = true;
                tracing::warn!(
                    streaming = delivery == Delivery::Stream,
                    attempt = u32::from(self.retries) + 1,
                    "{FINISH_MISSING_LINE}"
                );
            }
            Some(FinishReason::Length) => self.cut_by_length = true,
            Some(FinishReason::Stop | FinishReason::Other) => {}
        }
    }

    /// **まだ引き直しを送れるか**（本文は見ない / モジュールの doc の「何回まで送るか」）。
    ///
    /// その回の終わりを [`Self::note_ending`] で記録した**後**に聞くこと（その回の `length` を数えるため）。
    pub(super) const fn has_room(&self) -> bool {
        let limit = if self.cut_by_length {
            MAX_RETRIES_AFTER_LENGTH
        } else {
            MAX_RETRIES
        };
        self.retries < limit
    }

    /// **この本文で、もう一度送るか**（引き直す空で、かつ回数が残っている）。
    pub(super) fn retries_again(&self, raw: &str, delivery: Delivery) -> bool {
        retries(raw, delivery) && self.has_room()
    }

    /// **その回を捨てて、次の回を開けた**記録。捨てた回の生の出力と `usage` を足し込む。
    #[must_use]
    pub(super) fn discarded(self, raw: &str, usage: Option<TokenUsage>) -> Self {
        let tokens = usage.and_then(|usage| usage.completion_tokens);
        let discarded_completion_tokens = if self.discarded == 0 {
            tokens
        } else {
            match (self.discarded_completion_tokens, tokens) {
                (Some(sum), Some(tokens)) => Some(sum.saturating_add(tokens)),
                _ => None,
            }
        };
        Self {
            retries: self.retries.saturating_add(1),
            discarded: self.discarded.saturating_add(1),
            discarded_chars: self.discarded_chars.saturating_add(raw.chars().count()),
            discarded_unclosed: self.discarded_unclosed
                || matches!(reply_body(raw), ReplyBody::Blank { unclosed: true }),
            discarded_completion_tokens,
            ..self
        }
    }

    /// **引き直しの要求が開けず、手前の回を採用した**記録（送ったが、その回は捨てていない）。
    #[must_use]
    pub(super) const fn failed(self) -> Self {
        Self {
            retries: self.retries.saturating_add(1),
            failed: true,
            ..self
        }
    }

    /// 送った引き直しの回数（0〜2）。
    pub(super) const fn retries(self) -> u8 {
        self.retries
    }

    /// 引き直しのどれかが開けなかったか。
    pub(super) const fn failed_over(self) -> bool {
        self.failed
    }

    /// 捨てた回すべての生の出力の文字数の合計。
    pub(super) const fn discarded_chars(self) -> usize {
        self.discarded_chars
    }

    /// 捨てた回のどれかが閉じない思考だったか。
    pub(super) const fn discarded_unclosed(self) -> bool {
        self.discarded_unclosed
    }

    /// 捨てた回すべての生成トークン数の合計（1回でも返さなければ `None`）。
    pub(super) const fn discarded_completion_tokens(self) -> Option<u32> {
        self.discarded_completion_tokens
    }

    /// どれかの回が `finish_reason` を返さないまま終わったか。
    pub(super) const fn finish_missing(self) -> bool {
        self.finish_missing
    }
}

/// **引き直しの要求が失敗したことを `warn` に出す**（両方の経路が通る）。
///
/// 応答には何も載せない（手前の回は上流が正常に返した回で、その空を保存して返す）。
/// 詳細（`Debug` は `#[source]` 経由でパスを含みうる）はログにだけ出す。
pub(super) fn log_retry_failure(err: &LlmError, delivery: Delivery) {
    tracing::warn!(
        error = %err,
        detail = ?err,
        streaming = delivery == Delivery::Stream,
        "{RETRY_FAILED_LINE}"
    );
}

/// **非ストリーミングの引き直し。** 空なら、**同じ上流の写し・同じ要求**で送り直す（最大2回 / [`EmptyRetry::has_room`]）。
///
/// 返すのは採用した回と、その記録。最後の回も空ならそれを採用する（それ以上は送らない）。
/// 引き直しが失敗したら、その手前の回を採用する（[`log_retry_failure`]）。
///
/// **呼び出し側は、答え合わせと封の清算を、返した回の `usage` で1回だけ行うこと。**
pub(super) async fn complete_again_if_empty(
    llm: &dyn LlmClient,
    request: &ChatRequest,
    first: Completion,
) -> (Completion, EmptyRetry) {
    let mut record = EmptyRetry::default();
    let mut current = first;
    loop {
        record.note_ending(current.finish_reason, Delivery::Complete);
        if !record.retries_again(&current.content, Delivery::Complete) {
            return (current, record);
        }
        match llm.complete(request).await {
            Ok(next) => {
                record = record.discarded(&current.content, current.usage);
                current = next;
            }
            Err(err) => {
                log_retry_failure(&err, Delivery::Complete);
                return (current, record.failed());
            }
        }
    }
}

#[cfg(test)]
pub(super) mod test_upstream {
    //! **引き直しの相手の代役**（このモジュールと `super::relay` のテストが使う）。ネットワークには触らない。

    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use futures_util::stream;

    use crate::llm::{
        ChatChunk, ChatRequest, ChatStream, Completion, FinishReason, LlmClient, LlmError,
        LlmFuture, TokenUsage,
    };

    /// 代役が返す1回分。`Err(())` は要求の失敗（`LlmError::Status { status: 500 }` を返す）。
    pub(in crate::server) type Scripted<T> = Result<T, ()>;

    /// 中継の1本分の台本（チャンクの列）。
    pub(in crate::server) type Chunks = Vec<Result<ChatChunk, LlmError>>;

    /// **台本どおりに答える上流。** 受けた要求を控え、`complete` / `stream` の回数を数える。
    #[derive(Default)]
    pub(in crate::server) struct Again {
        completions: Mutex<VecDeque<Scripted<Completion>>>,
        streams: Mutex<VecDeque<Scripted<Chunks>>>,
        requests: Mutex<Vec<ChatRequest>>,
        completes: Mutex<usize>,
        opens: Mutex<usize>,
    }

    impl Again {
        /// `complete` が順に返すもの。
        pub(in crate::server) fn completing(replies: Vec<Scripted<Completion>>) -> Arc<Self> {
            let again = Self::default();
            *again.completions.lock().expect("ロック") = replies.into();
            Arc::new(again)
        }

        /// `stream` が順に返すもの。
        pub(in crate::server) fn streaming(replies: Vec<Scripted<Chunks>>) -> Arc<Self> {
            let again = Self::default();
            *again.streams.lock().expect("ロック") = replies.into();
            Arc::new(again)
        }

        /// `complete` を呼ばれた回数。
        pub(in crate::server) fn completes(&self) -> usize {
            *self.completes.lock().expect("ロック")
        }

        /// `stream` を呼ばれた回数。
        pub(in crate::server) fn opens(&self) -> usize {
            *self.opens.lock().expect("ロック")
        }

        /// 受けた要求（受けた順）。
        pub(in crate::server) fn requests(&self) -> Vec<ChatRequest> {
            self.requests.lock().expect("ロック").clone()
        }
    }

    impl LlmClient for Again {
        fn pinned(&self) -> Option<Arc<dyn LlmClient>> {
            None
        }

        fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
            self.requests.lock().expect("ロック").push(request.clone());
            *self.completes.lock().expect("ロック") += 1;
            let next = self.completions.lock().expect("ロック").pop_front();
            Box::pin(async move {
                match next {
                    Some(Ok(completion)) => Ok(completion),
                    Some(Err(())) | None => Err(LlmError::Status { status: 500 }),
                }
            })
        }

        fn stream<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
            self.requests.lock().expect("ロック").push(request.clone());
            *self.opens.lock().expect("ロック") += 1;
            let next = self.streams.lock().expect("ロック").pop_front();
            Box::pin(async move {
                match next {
                    Some(Ok(items)) => Ok(Box::pin(stream::iter(items)) as ChatStream),
                    Some(Err(())) | None => Err(LlmError::Status { status: 500 }),
                }
            })
        }
    }

    /// 本文 `content` と `usage` の補完（`finish_reason` は `stop`）。
    pub(in crate::server) fn completion(
        content: &str,
        usage: Option<(u32, Option<u32>)>,
    ) -> Completion {
        completion_ending(content, Some(FinishReason::Stop), usage)
    }

    /// [`completion`] の、`finish_reason` を選ぶ形（作業単位 A-18c）。
    pub(in crate::server) fn completion_ending(
        content: &str,
        finish_reason: Option<FinishReason>,
        usage: Option<(u32, Option<u32>)>,
    ) -> Completion {
        Completion {
            content: content.to_owned(),
            finish_reason,
            usage: usage.map(|(prompt_tokens, completion_tokens)| TokenUsage {
                prompt_tokens,
                completion_tokens,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_upstream::{Again, completion, completion_ending};
    use super::*;
    use crate::testlog::{captured_under_default_filter, has_field, line_containing};

    /// 同じスレッドで走らせる（ログの捕捉はスレッドローカル）。
    fn block_on<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime を作れること")
            .block_on(future)
    }

    /// 送る要求（中身は問わない。**同じ要求が2回目にも届く**ことを見る）。
    fn request() -> ChatRequest {
        let mut request = ChatRequest::new(Vec::new());
        request.max_tokens = Some(321);
        request
    }

    /// 2回目の本文。**1回目のどれとも別の文字列。**
    const SECOND: &str = "「おかえりなさい」 彼女は微笑んだ。";

    fn usage(prompt_tokens: u32, completion_tokens: Option<u32>) -> Option<TokenUsage> {
        Some(TokenUsage {
            prompt_tokens,
            completion_tokens,
        })
    }

    /// **引き直すかの表**（モジュールの doc）。閉じない思考の回だけ、経路で逆になる。
    #[test]
    fn only_an_unclosed_thought_is_left_alone_on_the_streaming_path() {
        for (raw, complete, stream) in [
            ("", true, true),
            (" \n\u{3000}\t", true, true),
            ("<think>\nThe user is home.\n</think>\n", true, true),
            ("<think>\nThe user is asking", true, false),
            ("「」", false, false),
            ("**", false, false),
            ("「おかえり」", false, false),
            ("「おかえり」<think>まだ", false, false),
        ] {
            assert_eq!(retries(raw, Delivery::Complete), complete, "{raw:?}");
            assert_eq!(retries(raw, Delivery::Stream), stream, "{raw:?}");
        }
    }

    /// **回数の上限**: 引き直しは2回まで。`length` の回が1つでもあれば（どの順でも）1回まで。
    /// `finish_reason` が無い回は `length` ではない。
    #[test]
    fn the_room_for_retries_is_two_or_one_after_a_length_cut() {
        let stop = Some(FinishReason::Stop);
        let length = Some(FinishReason::Length);
        // (各回の終わり, その回の後に送れるか)
        for (endings, expected) in [
            (vec![stop], true),
            (vec![stop, stop], true),
            (vec![stop, stop, stop], false),
            (vec![length], true),
            (vec![stop, length], false),
            (vec![length, stop], false),
            (vec![None], true),
            (vec![None, None], true),
            (vec![None, None, None], false),
            (vec![Some(FinishReason::Other), stop], true),
        ] {
            let mut record = EmptyRetry::default();
            for (index, ending) in endings.iter().enumerate() {
                if index > 0 {
                    record = record.discarded("", None);
                }
                record.note_ending(*ending, Delivery::Complete);
            }
            assert_eq!(record.has_room(), expected, "{endings:?}");
            assert_eq!(
                record.retries_again("", Delivery::Complete),
                expected,
                "{endings:?}"
            );
            assert!(
                !record.retries_again("「おかえり」", Delivery::Complete),
                "{endings:?}: 本文のある回は送らない"
            );
        }
    }

    /// **捨てた回の記録**は、生の出力の文字数（思考込み）・閉じない思考だったか・生成数を持つ。
    /// **2回捨てたら合計**（文字数は足す・閉じない思考はどれか・生成数はすべてが返したときだけ足す）。
    #[test]
    fn the_discarded_record_adds_up_every_discarded_attempt() {
        let raw = "<think>考え中";
        let once = EmptyRetry::default().discarded(raw, usage(900, Some(57)));
        assert_eq!(once.retries(), 1);
        assert!(!once.failed_over());
        assert_eq!(once.discarded_chars(), raw.chars().count());
        assert_ne!(
            once.discarded_chars(),
            raw.len(),
            "前提: バイト数と文字数が違う材料"
        );
        assert!(once.discarded_unclosed());
        assert_eq!(once.discarded_completion_tokens(), Some(57));

        // 2回目（閉じた思考だけ・生成数 23）を捨てる → 合計。閉じない思考は1回目のものが残る。
        let twice = once.discarded("<think>a</think>", usage(901, Some(23)));
        assert_eq!(twice.retries(), 2);
        assert_eq!(twice.discarded_chars(), raw.chars().count() + 16);
        assert!(twice.discarded_unclosed(), "どれかが閉じない思考なら立つ");
        assert_eq!(twice.discarded_completion_tokens(), Some(57 + 23));

        // 閉じない思考が後の回だけでも立つ。
        let later = EmptyRetry::default()
            .discarded("", None)
            .discarded("<think>考え中", None);
        assert!(later.discarded_unclosed());

        // 一部だけが `usage` を返した → 無し（どちらの順でも）。
        for (first, second) in [
            (usage(900, Some(57)), usage(901, None)),
            (usage(900, Some(57)), None),
            (None, usage(901, Some(23))),
            (usage(900, None), usage(901, Some(23))),
        ] {
            let record = EmptyRetry::default()
                .discarded("", first)
                .discarded("", second);
            assert_eq!(
                record.discarded_completion_tokens(),
                None,
                "{first:?} / {second:?}"
            );
        }

        // 閉じた思考だけ・使用量なし。
        let record = EmptyRetry::default().discarded("<think>a</think>", None);
        assert!(!record.discarded_unclosed());
        assert_eq!(record.discarded_completion_tokens(), None);

        // 失敗した記録は「送ったが、捨てていない」。
        let failed = EmptyRetry::default().failed();
        assert_eq!(failed.retries(), 1);
        assert!(failed.failed_over());
        assert_eq!(failed.discarded_chars(), 0);
        assert_eq!(failed.discarded_completion_tokens(), None);

        // 2回目は開けて3回目を開けなかった → 2回・失敗・捨てた回（1回目）の欄が残る。
        let failed_third = once.failed();
        assert_eq!(failed_third.retries(), 2);
        assert!(failed_third.failed_over());
        assert_eq!(failed_third.discarded_chars(), raw.chars().count());
        assert_eq!(failed_third.discarded_completion_tokens(), Some(57));

        // 引き直さなかった回は全部 0。
        assert_eq!(EmptyRetry::default().retries(), 0);
        assert!(!EmptyRetry::default().failed_over());
        assert!(!EmptyRetry::default().finish_missing());
    }

    /// **`finish_reason` が無いまま終わった回は `warn` で言い、記録に立てる。ある回では言わない**（決定3）。
    #[test]
    fn a_missing_finish_reason_is_warned_and_recorded() {
        for (finish, missing) in [
            (None, true),
            (Some(FinishReason::Stop), false),
            (Some(FinishReason::Length), false),
            (Some(FinishReason::Other), false),
        ] {
            for delivery in [Delivery::Complete, Delivery::Stream] {
                let mut record = EmptyRetry::default();
                let output = captured_under_default_filter(|| {
                    record.note_ending(finish, delivery);
                });
                assert_eq!(record.finish_missing(), missing, "{finish:?}");
                if missing {
                    let line = line_containing(&output, FINISH_MISSING_LINE);
                    assert!(line.contains("WARN"), "{line:?}");
                    assert!(
                        has_field(line, &format!("streaming={}", delivery == Delivery::Stream)),
                        "{line:?}"
                    );
                } else {
                    assert!(
                        !output.contains(FINISH_MISSING_LINE),
                        "{finish:?}: 言わないこと: {output:?}"
                    );
                }
            }
        }
    }

    /// **空でない回は1往復だけ**（「常に2回送る」実装を赤に）。
    #[test]
    fn a_reply_with_a_body_is_not_sent_again() {
        let again = Again::completing(vec![Ok(completion(SECOND, None))]);
        let (adopted, record) = block_on(complete_again_if_empty(
            again.as_ref(),
            &request(),
            completion("「ただいま」", None),
        ));
        assert_eq!(again.completes(), 0, "空でない回に送り直さないこと");
        assert_eq!(adopted.content, "「ただいま」");
        assert_eq!(record, EmptyRetry::default());
    }

    /// **空の回は同じ要求で送り直し、2回目が空でなければそれを採用する（3回目は送らない）。**
    /// 閉じない思考の回も（非ストリーミング）。
    #[test]
    fn a_blank_reply_is_sent_again_with_the_same_request() {
        for first in ["", "  \n", "<think>a</think>", "<think>\nunclosed"] {
            let again = Again::completing(vec![
                Ok(completion(SECOND, Some((800, Some(9))))),
                Ok(completion("余分", None)),
            ]);
            let (adopted, record) = block_on(complete_again_if_empty(
                again.as_ref(),
                &request(),
                completion(first, Some((700, Some(41)))),
            ));
            assert_eq!(again.completes(), 1, "{first:?}: 1回だけ送り直すこと");
            assert_eq!(again.requests(), vec![request()], "{first:?}: 同じ要求");
            assert_eq!(adopted.content, SECOND, "{first:?}: 2回目を採用すること");
            assert_eq!(
                adopted.usage.map(|usage| usage.prompt_tokens),
                Some(800),
                "{first:?}: 使用量も2回目のもの"
            );
            assert_eq!(
                record,
                EmptyRetry::default().discarded(first, usage(700, Some(41))),
                "{first:?}"
            );
        }
    }

    /// **2回目も空なら3回目を送り、3回目を採用する**（3回とも同じ要求）。
    #[test]
    fn a_second_blank_reply_is_followed_by_a_third_attempt() {
        let again = Again::completing(vec![
            Ok(completion("<think>b</think>", Some((801, Some(23))))),
            Ok(completion(SECOND, Some((802, Some(9))))),
            Ok(completion("余分", None)),
        ]);
        let (adopted, record) = block_on(complete_again_if_empty(
            again.as_ref(),
            &request(),
            completion(" ", Some((800, Some(41)))),
        ));
        assert_eq!(again.completes(), 2, "3回目まで送ること");
        assert_eq!(again.requests(), vec![request(), request()], "同じ要求");
        assert_eq!(adopted.content, SECOND);
        assert_eq!(adopted.usage.map(|usage| usage.prompt_tokens), Some(802));
        assert_eq!(record.retries(), 2);
        assert_eq!(record.discarded_chars(), 1 + 16);
        assert_eq!(record.discarded_completion_tokens(), Some(41 + 23));
    }

    /// **3回とも空なら3回目を採用し、4回目は送らない。**
    #[test]
    fn a_third_blank_reply_is_kept_and_not_retried_again() {
        let again = Again::completing(vec![
            Ok(completion("<think>b</think>", None)),
            Ok(completion("  ", None)),
            Ok(completion(SECOND, None)),
        ]);
        let (adopted, record) = block_on(complete_again_if_empty(
            again.as_ref(),
            &request(),
            completion("", None),
        ));
        assert_eq!(again.completes(), 2, "3回目までで止めること");
        assert_eq!(adopted.content, "  ", "3回目（空）を採用すること");
        assert_eq!(record.retries(), 2);
        assert!(!record.failed_over());
    }

    /// **`length` の空が1回でもあれば、引き直しは1回まで**（両方の順）。
    #[test]
    fn a_length_cut_allows_only_one_retry_in_either_order() {
        for (first, second) in [
            (FinishReason::Stop, FinishReason::Length),
            (FinishReason::Length, FinishReason::Stop),
        ] {
            let again = Again::completing(vec![
                Ok(completion_ending(" ", Some(second), None)),
                Ok(completion(SECOND, None)),
            ]);
            let (adopted, record) = block_on(complete_again_if_empty(
                again.as_ref(),
                &request(),
                completion_ending("", Some(first), None),
            ));
            assert_eq!(
                again.completes(),
                1,
                "{first:?}→{second:?}: 3回目を送らない"
            );
            assert_eq!(adopted.content, " ", "{first:?}→{second:?}");
            assert_eq!(record.retries(), 1);
        }
    }

    /// **2回目が失敗したら1回目を採用し、`warn` を出す。**
    #[test]
    fn a_failed_retry_keeps_the_first_reply_and_warns() {
        let again = Again::completing(vec![Err(())]);
        let first = "<think>a</think>";
        let mut outcome = None;
        let output = captured_under_default_filter(|| {
            outcome = Some(block_on(complete_again_if_empty(
                again.as_ref(),
                &request(),
                completion(first, Some((700, Some(41)))),
            )));
        });
        let (adopted, record) = outcome.expect("走ったこと");
        assert_eq!(again.completes(), 1);
        assert_eq!(adopted.content, first, "1回目を採用すること");
        assert_eq!(adopted.usage.map(|usage| usage.prompt_tokens), Some(700));
        assert_eq!(record, EmptyRetry::default().failed());
        let line = line_containing(&output, RETRY_FAILED_LINE);
        assert!(line.contains("WARN"), "{line:?}");
        assert!(has_field(line, "streaming=false"), "{line:?}");
    }

    /// **3回目が失敗したら2回目を採用する。** 記録は2回・失敗・捨てた回は1回目。
    #[test]
    fn a_failed_third_attempt_keeps_the_second_reply() {
        let again = Again::completing(vec![
            Ok(completion("<think>b</think>", Some((801, Some(23))))),
            Err(()),
        ]);
        let (adopted, record) = block_on(complete_again_if_empty(
            again.as_ref(),
            &request(),
            completion(" ", Some((800, Some(41)))),
        ));
        assert_eq!(again.completes(), 2);
        assert_eq!(adopted.content, "<think>b</think>", "2回目を採用すること");
        assert_eq!(adopted.usage.map(|usage| usage.prompt_tokens), Some(801));
        assert_eq!(record.retries(), 2);
        assert!(record.failed_over());
        assert_eq!(record.discarded_chars(), 1);
        assert_eq!(record.discarded_completion_tokens(), Some(41));
    }

    /// **`finish_reason` が無い回は、本文があっても `warn` を出し記録に立てる。引き直さない。**
    #[test]
    fn a_reply_without_a_finish_reason_is_warned_but_not_retried() {
        let again = Again::completing(vec![Ok(completion("余分", None))]);
        let mut outcome = None;
        let output = captured_under_default_filter(|| {
            outcome = Some(block_on(complete_again_if_empty(
                again.as_ref(),
                &request(),
                completion_ending(SECOND, None, None),
            )));
        });
        let (adopted, record) = outcome.expect("走ったこと");
        assert_eq!(again.completes(), 0);
        assert_eq!(adopted.content, SECOND);
        assert!(record.finish_missing());
        let line = line_containing(&output, FINISH_MISSING_LINE);
        assert!(has_field(line, "streaming=false"), "{line:?}");
    }
}
