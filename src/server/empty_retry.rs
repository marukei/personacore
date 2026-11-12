//! **本文が空なら、1回だけ引き直す**（[ADR-0059](../../../docs/adr/0059-thinking-and-response-room-in-the-upstream-profile.md) §決定3 /
//! 作業単位 A-18b）。
//!
//! # 何を空とみなすか
//!
//! 本文から漏れた思考を剥がした（ADR-0053）後が空か空白だけの回（`domain::reply_body`）。
//! 空の `「」` だけ・宙に浮いた強調の印だけは、分解では `ParseOutcome::Empty` になるが、
//! **上流が文字を返した回なので対象にしない。** **上流も面も選ばない**（思考を入れた面に限らない）。
//! 回数は1回で固定し、2回目も空なら空として保存する（つまみにしない）。
//!
//! | 経路 | 閉じない思考を剥がして空になった回 | それ以外の空 |
//! |---|---|---|
//! | 非ストリーミング（[`complete_again_if_empty`]） | **引き直す** | 引き直す |
//! | ストリーミング（`super::relay`） | **引き直さない** —— 続きとして流すと、積み上げた本文が「閉じない思考＋2回目」になり、ADR-0053 §決定2 の規則で2回目まで捨てられる | 引き直す |
//!
//! # 2回目が失敗したら
//!
//! **引き直しが無かったときと同じ結果にする**（1回目の空を空として保存する）。失敗は `warn` に出す
//! （[`log_retry_failure`]）。応答の成否は変えない —— 1回目は上流が正常に返した回である。
//!
//! # 使用量と保存
//!
//! * 答え合わせ（ADR-0013 第3層）と封の清算（ADR-0052）は**採用した回の `usage` で1回だけ**（呼び出し側）。
//! * `raw_content` には**採用した回の生の出力**を保存する。捨てた1回目は保存せず、その長さ・閉じない思考だったか・
//!   生成トークン数を、分解の行（`super::turn`）の欄に出す（[`EmptyRetry`]）。

use crate::domain::{ReplyBody, reply_body};
use crate::llm::{ChatRequest, Completion, LlmClient, LlmError, TokenUsage};

/// 2回目の要求が失敗したことを言う行（`warn`）。**テストが綴りを照合する。**
pub(super) const RETRY_FAILED_LINE: &str =
    "空の本文の引き直しに失敗しました（1回目の空のまま保存します）";

/// 応答の受け取り方。**閉じない思考の回を引き直すかだけが違う**（モジュールの doc の表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Delivery {
    /// 応答を最後まで受け取ってから保存する（`LlmClient::complete`）。
    Complete,
    /// 受けたそばから利用者へ流す（`LlmClient::stream` / `super::relay`）。
    Stream,
}

/// **この本文を引き直すか**（モジュールの doc の表）。純粋関数。
pub(super) fn retries(raw: &str, delivery: Delivery) -> bool {
    match reply_body(raw) {
        ReplyBody::Present => false,
        ReplyBody::Blank { unclosed } => !(unclosed && delivery == Delivery::Stream),
    }
}

/// **引き直しの記録**（分解の行の欄になる / `super::turn::log_outcome`）。
///
/// 引き直さなかった回は [`EmptyRetry::default`]（全部 0 / `false` / `None`）—— **0 の回も欄を出す**
/// （出さないと分母が作れない）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct EmptyRetry {
    /// **2回目の要求を送った回数**（0 か 1）。失敗した回も 1。
    retries: u8,
    /// 2回目の要求が失敗し、**1回目を採用した**か。
    failed: bool,
    /// 捨てた1回目の生の出力の文字数（思考込み）。捨てていなければ 0。
    discarded_chars: usize,
    /// 捨てた1回目が、閉じない思考を剥がして空になった回か。
    discarded_unclosed: bool,
    /// 捨てた1回目の生成トークン数（上流が数えた値。返さなければ `None`）。
    discarded_completion_tokens: Option<u32>,
}

impl EmptyRetry {
    /// **1回目を捨てて2回目を採用した**記録。捨てた回の生の出力と `usage` から作る。
    pub(super) fn discarded(first_raw: &str, first_usage: Option<TokenUsage>) -> Self {
        Self {
            retries: 1,
            failed: false,
            discarded_chars: first_raw.chars().count(),
            discarded_unclosed: matches!(
                reply_body(first_raw),
                ReplyBody::Blank { unclosed: true }
            ),
            discarded_completion_tokens: first_usage.and_then(|usage| usage.completion_tokens),
        }
    }

    /// **2回目の要求が失敗し、1回目を採用した**記録（送ったが、何も捨てていない）。
    pub(super) const fn failed() -> Self {
        Self {
            retries: 1,
            failed: true,
            discarded_chars: 0,
            discarded_unclosed: false,
            discarded_completion_tokens: None,
        }
    }

    /// 2回目の要求を送った回数（0 か 1）。
    pub(super) const fn retries(self) -> u8 {
        self.retries
    }

    /// 2回目が失敗して1回目を採用したか。
    pub(super) const fn failed_over(self) -> bool {
        self.failed
    }

    /// 捨てた1回目の生の出力の文字数。
    pub(super) const fn discarded_chars(self) -> usize {
        self.discarded_chars
    }

    /// 捨てた1回目が閉じない思考だったか。
    pub(super) const fn discarded_unclosed(self) -> bool {
        self.discarded_unclosed
    }

    /// 捨てた1回目の生成トークン数。
    pub(super) const fn discarded_completion_tokens(self) -> Option<u32> {
        self.discarded_completion_tokens
    }
}

/// **2回目の要求が失敗したことを `warn` に出す**（両方の経路が通る）。
///
/// 応答には何も載せない（1回目は上流が正常に返した回で、その空を保存して返す）。
/// 詳細（`Debug` は `#[source]` 経由でパスを含みうる）はログにだけ出す。
pub(super) fn log_retry_failure(err: &LlmError, delivery: Delivery) {
    tracing::warn!(
        error = %err,
        detail = ?err,
        streaming = delivery == Delivery::Stream,
        "{RETRY_FAILED_LINE}"
    );
}

/// **非ストリーミングの引き直し。** 1回目が空なら、**同じ上流の写し・同じ要求**でもう1回だけ送る。
///
/// 返すのは採用した回と、その記録。2回目も空なら2回目を採用する（それ以上は送らない）。
/// 2回目が失敗したら1回目を採用する（[`log_retry_failure`]）。
///
/// **呼び出し側は、答え合わせと封の清算を、返した回の `usage` で1回だけ行うこと。**
pub(super) async fn complete_again_if_empty(
    llm: &dyn LlmClient,
    request: &ChatRequest,
    first: Completion,
) -> (Completion, EmptyRetry) {
    if !retries(&first.content, Delivery::Complete) {
        return (first, EmptyRetry::default());
    }
    match llm.complete(request).await {
        // **2回目も空でも2回目を採用する**（それ以上は送らない）。
        Ok(second) => {
            let record = EmptyRetry::discarded(&first.content, first.usage);
            (second, record)
        }
        Err(err) => {
            log_retry_failure(&err, Delivery::Complete);
            (first, EmptyRetry::failed())
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

    /// 本文 `content` と `usage` の補完。
    pub(in crate::server) fn completion(
        content: &str,
        usage: Option<(u32, Option<u32>)>,
    ) -> Completion {
        Completion {
            content: content.to_owned(),
            finish_reason: Some(FinishReason::Stop),
            usage: usage.map(|(prompt_tokens, completion_tokens)| TokenUsage {
                prompt_tokens,
                completion_tokens,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_upstream::{Again, completion};
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

    /// **捨てた回の記録**は、生の出力の文字数（思考込み）・閉じない思考だったか・生成数を持つ。
    #[test]
    fn the_discarded_record_carries_the_first_attempt() {
        let raw = "<think>考え中";
        let record = EmptyRetry::discarded(
            raw,
            Some(TokenUsage {
                prompt_tokens: 900,
                completion_tokens: Some(57),
            }),
        );
        assert_eq!(record.retries(), 1);
        assert!(!record.failed_over());
        assert_eq!(record.discarded_chars(), raw.chars().count());
        assert_ne!(
            record.discarded_chars(),
            raw.len(),
            "前提: バイト数と文字数が違う材料"
        );
        assert!(record.discarded_unclosed());
        assert_eq!(record.discarded_completion_tokens(), Some(57));

        // 閉じた思考だけ・使用量なし。
        let record = EmptyRetry::discarded("<think>a</think>", None);
        assert!(!record.discarded_unclosed());
        assert_eq!(record.discarded_completion_tokens(), None);

        // 失敗した記録は「送ったが、捨てていない」。
        let failed = EmptyRetry::failed();
        assert_eq!(failed.retries(), 1);
        assert!(failed.failed_over());
        assert_eq!(failed.discarded_chars(), 0);
        assert_eq!(failed.discarded_completion_tokens(), None);

        // 引き直さなかった回は全部 0。
        assert_eq!(EmptyRetry::default().retries(), 0);
        assert!(!EmptyRetry::default().failed_over());
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

    /// **空の回は同じ要求で1回だけ送り直し、2回目を採用する。** 閉じない思考の回も（非ストリーミング）。
    #[test]
    fn a_blank_reply_is_sent_again_once_with_the_same_request() {
        for first in ["", "  \n", "<think>a</think>", "<think>\nunclosed"] {
            let again = Again::completing(vec![Ok(completion(SECOND, Some((800, Some(9)))))]);
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
                EmptyRetry::discarded(
                    first,
                    Some(TokenUsage {
                        prompt_tokens: 700,
                        completion_tokens: Some(41),
                    })
                ),
                "{first:?}"
            );
        }
    }

    /// **2回目も空なら2回目を採用し、3回目は送らない。**
    #[test]
    fn a_second_blank_reply_is_kept_and_not_retried_again() {
        let again = Again::completing(vec![
            Ok(completion("<think>b</think>", None)),
            Ok(completion(SECOND, None)),
        ]);
        let (adopted, record) = block_on(complete_again_if_empty(
            again.as_ref(),
            &request(),
            completion("", None),
        ));
        assert_eq!(again.completes(), 1, "2回目までで止めること");
        assert_eq!(
            adopted.content, "<think>b</think>",
            "2回目（空）を採用すること"
        );
        assert_eq!(record.retries(), 1);
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
        assert_eq!(record, EmptyRetry::failed());
        let line = line_containing(&output, RETRY_FAILED_LINE);
        assert!(line.contains("WARN"), "{line:?}");
        assert!(has_field(line, "streaming=false"), "{line:?}");
    }
}
