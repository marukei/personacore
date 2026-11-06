//! リクエスト本文の読み取り。**上限と、超過したときの読み捨て。**
//!
//! # なぜ axum 既定の本文上限をそのまま使わないか
//!
//! axum 既定の 2MB 制限は `http_body_util::Limited` で実装されており、
//! **Content-Length ではなく実際に読んだバイト数**で打ち切る。打ち切った時点で
//! 抽出器は拒否を返し、サーバは 413 を書いて接続を閉じる。
//! **ところがクライアントはまだ本文を書いている。**
//! 未読データを残したまま閉じると OS が RST を送るので、クライアントの write が失敗し、
//! **既に受信していた 413 まで破棄される。**
//!
//! 2026-08-14 の実測（docs/06-roadmap.md「0005 の範囲」）:
//!
//! | 本文 | 413 を受け取れた | 応答を失った |
//! |---|---|---|
//! | 2.86 MB | 6 | **9** |
//! | 2.10 MB | 15 | 0 |
//!
//! **超過量が大きいほど壊れる。** 2.10MB で壊れないのは、超過分が
//! ソケットの受信バッファに収まっており、hyper が接続を閉じる直前に行う
//! **1回だけの読み取り**（`Conn::poll_drain_or_close_read`）で吸い切れるから。
//! hyper 1.x にこれを「最後まで読み捨てる」に変える設定は無い
//! （`poll_read_body` を1度呼び、終わっていなければ `close_read()` する実装）。
//!
//! # 対処
//!
//! **413 を返すと決めた後、残りの本文を上限付きで読み捨ててから応答する。**
//! 全部読んでから閉じれば未読データが無いので RST は起きない。
//!
//! **Content-Length で即座に弾く案は逆効果。** 未読が本文まるごとになり、悪化する。
//!
//! # 読み捨てにも上限を置く
//!
//! 無制限に読むと上限を設けた意味が消える（`127.0.0.1` 限定なので DoS の危険は
//! 小さいが、設計として無制限は持たない）。`MAX_DRAIN_BYTES` まで読んで
//! まだ終わらなければ、そこでやめて応答する。**その場合は従来どおり RST が起こりうる**
//! ——「必ず届く」ではなく「予算の範囲では必ず届く」が保証の内容。
//!
//! # 上限を2箇所に持つことについて
//!
//! axum 既定の上限は**外さない。** このミドルウェアが先に（内側で）本文を読むので、
//! `MAX_BODY_BYTES` が axum の既定と同じ値である限り、判定は必ずこちらが先に行う。
//! axum 側は「このミドルウェアが外れたときに上限が消える」ことを防ぐ控えとして残す。
//!
//! **`MAX_BODY_BYTES` を axum の既定より大きくしたときの代償は次の2つ。**
//! 2026-08-14 に 4MB で実測したところ **raw socket テストは緑のままだった**ので、
//! 「大きくすると読み捨てが走らず元の壊れ方に戻る」は誤り
//! （ミドルウェアが 3MB を先に全部読み切るため、抽出器が 413 を返す時点で
//! 未読データが無い）:
//!
//! * **1リクエストあたり最大 `MAX_BODY_BYTES` までメモリに積む。**
//! * **413 が「読み捨ての無い抽出器経路」から返る。**
//!   axum 既定と `MAX_BODY_BYTES` の間の大きさの本文は、このミドルウェアではなく
//!   `ApiError::from_json_rejection` の 413 分岐が返す。読み捨てを担うこのモジュールの
//!   経路に入るのは**本文が `MAX_BODY_BYTES` を超えた場合だけ**なので、
//!   「413 は予算の範囲で必ず届く」という保証はその範囲でしか成り立たない。

use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use futures_util::{Stream, StreamExt as _};

use super::error::ApiError;

/// 受け付けるリクエスト本文の上限。
///
/// **axum の既定（`axum_core` の `DEFAULT_LIMIT` = 2MB）と同じ値にしてある。**
/// 大きくしたときの代償（メモリと、413 を返す経路が変わること）はモジュールの doc。
pub(crate) const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// 上限を超えた後に読み捨てる量の上限。
///
/// これを超えて送り続けるクライアントには 413 が届かないことがある。
/// **「無制限に読む」にしないこと**（上限を設けた意味が消える）。
///
/// # 上げ下げのコストは非対称
///
/// 読み捨ては予算を減らすだけで**バッファに積まない**ので、
/// **メモリコストは塊1個ぶん（O(1)）。** 上げても増えるのは
/// 「接続を掴んでいる時間」だけで、実質ほぼ無料。
///
/// 一方**下げる方向は実害に直結する。** 413 が届く保証の実効閾値は
/// **`MAX_BODY_BYTES` + `MAX_DRAIN_BYTES` = 10MB** で、これを超える本文は
/// `Drained::CapReached` になり **413 を失いうる。**
/// 下限側には事実上の見張りが効いていて、**1MB 前後を下回ると
/// raw socket テスト（`tests/server_chat.rs` の
/// `an_oversized_chat_body_receives_the_whole_413_over_a_raw_socket`）が赤になる**
/// ——あのテストの超過量が約 0.9MB（3MB の本文 − 2MB の上限）だから。
/// 2026-08-14 に 512KB で実測して赤を確認した。**迷ったら下げずに上げること。**
pub(crate) const MAX_DRAIN_BYTES: usize = 8 * 1024 * 1024;

/// 本文の読み取り結果。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BodyRead {
    /// 上限内で読み切った。
    Complete(Vec<u8>),
    /// 上限を超えた。**残りは `Drained` の内容まで読み捨て済み。**
    TooLarge(Drained),
    /// 本文を読んでいる途中で失敗した（クライアントの切断など）。
    Failed,
}

/// 上限超過後、残りをどこまで読み捨てられたか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Drained {
    /// **終端まで読み捨てた。** 未読が残らないので RST は起きない。
    Ended,
    /// 読み捨ての上限に達したので途中でやめた。**RST が起こりうる。**
    CapReached,
}

/// 本文を `limit` まで読み、超えたら残りを `drain_cap` まで読み捨てる。
///
/// **超過を検出しても、そこで読むのをやめない。** やめると未読データが残り、
/// 応答を書いて閉じた瞬間に RST でクライアントごと巻き添えにする
/// （モジュールの doc）。
pub(crate) async fn read_capped<S, E>(mut body: S, limit: usize, drain_cap: usize) -> BodyRead
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    let mut buffered: Vec<u8> = Vec::new();

    while let Some(frame) = body.next().await {
        let Ok(chunk) = frame else {
            return BodyRead::Failed;
        };
        if buffered.len() + chunk.len() > limit {
            // **上限超過。ここで return しない。** 読み捨ての予算からは、
            // 上限を跨いだこの塊の分も差し引く。
            let budget = drain_cap.saturating_sub(chunk.len());
            return BodyRead::TooLarge(drain(body, budget).await);
        }
        buffered.extend_from_slice(&chunk);
    }

    BodyRead::Complete(buffered)
}

/// 残りを `budget` バイトまで読み捨てる。
///
/// 読み捨て中の失敗は `Ended` として扱う。**その先を読む相手がもういない**ので、
/// 未読データが残る心配も無い。
async fn drain<S, E>(mut body: S, mut budget: usize) -> Drained
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
{
    loop {
        if budget == 0 {
            return Drained::CapReached;
        }
        match body.next().await {
            None | Some(Err(_)) => return Drained::Ended,
            Some(Ok(chunk)) => budget = budget.saturating_sub(chunk.len()),
        }
    }
}

/// 上限超過を記録する。**`Ended` と `CapReached` で深刻度を分ける。**
///
/// 既定のログフィルタは `warn,personacore=info` なので `debug` は出ない。
/// 両方を `debug` に混ぜると、**413 が届かなかった側の手掛かりが
/// 運用中のサーバに一切残らなくなる。**
///
/// **ミドルウェアから切り出してあるのは、ここを単体テストで観測するため。**
/// `axum::middleware::Next` はテストから構築できず、
/// `limit_request_body` ごと呼ぶ形にすると深刻度の分岐を確かめられない。
fn log_oversized_body(path: &str, drained: Drained) {
    match drained {
        // 終端まで読み捨てた＝未読が残らないので 413 は届く。
        // 「大きすぎる本文が来た」だけの情報なので `debug` のまま。
        Drained::Ended => tracing::debug!(
            path = %path,
            drained = ?drained,
            "リクエスト本文が上限を超えました"
        ),
        // **413 がクライアントに届かなかったかもしれない唯一のケース。**
        // クライアント側には「接続がリセットされた」としか見えず、
        // こちらのログだけが手掛かりになる。
        Drained::CapReached => tracing::warn!(
            path = %path,
            drained = ?drained,
            "リクエスト本文が読み捨ての上限を超えました（413 が届かない可能性があります）"
        ),
    }
}

/// 全ルートの前段に入るミドルウェア。**本文を読み切ってから先へ渡す。**
///
/// ここで 413 を返すので、`ApiError::from_json_rejection` の 413 分岐は
/// 実運用では通らなくなる。**それでも消さないこと**——このミドルウェアが外れたとき、
/// axum 既定の上限が同じ状況を作る（そのときの応答が 400 に化けるのは別の劣化）。
pub(crate) async fn limit_request_body(request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();

    match read_capped(body.into_data_stream(), MAX_BODY_BYTES, MAX_DRAIN_BYTES).await {
        BodyRead::Complete(bytes) => {
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await
        }
        BodyRead::TooLarge(drained) => {
            // **応答の内容は `ApiError` の定数のみ。** ここには入力を載せない。
            log_oversized_body(parts.uri.path(), drained);
            ApiError::body_too_large().into_response()
        }
        BodyRead::Failed => {
            tracing::debug!(path = %parts.uri.path(), "リクエスト本文を読み取れませんでした");
            ApiError::invalid_body().into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures_util::stream;

    use super::*;
    use crate::testlog::{
        VERBOSE_FILTER, captured_under_default_filter, captured_with_filter, has_field,
        line_containing,
    };

    /// 1塊のバイト数（テスト用）。
    const CHUNK: usize = 1_024;

    /// `CapReached` のログだけが持つ手掛かり。
    ///
    /// **全文一致にしないこと。** 文言を推敲しただけで赤くなると、
    /// 見張りたい「既定フィルタで残るか」と無関係なところでテストが割れる。
    const CAP_REACHED_MARKER: &str = "413 が届かない可能性";

    /// `Ended` 側の手掛かり。**文言ではなく構造化フィールドの値を見る。**
    ///
    /// 「既定フィルタに出ないこと」を主張する前に、
    /// **そもそもイベントが出ている**ことをこれで確かめる。
    const ENDED_MARKER: &str = "drained=Ended";

    /// `CHUNK` バイトの塊を `chunks` 個返すストリーム。
    ///
    /// **実際に取り出された総バイト数を数える。** 読み捨ての上限が効いているかは
    /// 「どこまで読んだか」でしか観測できない。
    fn counted(
        chunks: usize,
    ) -> (
        impl Stream<Item = Result<Bytes, Infallible>> + Unpin,
        Arc<AtomicUsize>,
    ) {
        let produced = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&produced);
        let body = stream::iter((0..chunks).map(move |_| {
            counter.fetch_add(CHUNK, Ordering::SeqCst);
            Ok(Bytes::from_static(&[b'x'; CHUNK]))
        }));
        (Box::pin(body), produced)
    }

    /// 上限内なら全部読める。
    #[tokio::test]
    async fn a_body_within_the_limit_is_read_whole() {
        let (body, produced) = counted(4);

        let outcome = read_capped(body, 4 * CHUNK, 16 * CHUNK).await;

        assert_eq!(outcome, BodyRead::Complete(vec![b'x'; 4 * CHUNK]));
        assert_eq!(produced.load(Ordering::SeqCst), 4 * CHUNK);
    }

    /// 上限ちょうどは通し、1バイトでも超えたら弾く。
    #[tokio::test]
    async fn the_limit_is_inclusive() {
        let inside = read_capped(
            Box::pin(stream::iter([Ok::<_, Infallible>(Bytes::from_static(
                b"abcd",
            ))])),
            4,
            16,
        )
        .await;
        assert_eq!(inside, BodyRead::Complete(b"abcd".to_vec()));

        let outside = read_capped(
            Box::pin(stream::iter([Ok::<_, Infallible>(Bytes::from_static(
                b"abcde",
            ))])),
            4,
            16,
        )
        .await;
        assert_eq!(outside, BodyRead::TooLarge(Drained::Ended));
    }

    /// **超過を見つけても読むのをやめない。** 残りを終端まで読み捨てる。
    ///
    /// これが本作業単位の中心。読み捨てを外すと、未読データを残したまま
    /// 応答して RST を招き、**413 がクライアントに届かなくなる。**
    #[tokio::test]
    async fn the_rest_of_an_oversized_body_is_drained_to_the_end() {
        let (body, produced) = counted(16);

        let outcome = read_capped(body, 4 * CHUNK, 64 * CHUNK).await;

        assert_eq!(
            outcome,
            BodyRead::TooLarge(Drained::Ended),
            "終端まで読み捨てること（途中でやめると RST になる）"
        );
        assert_eq!(
            produced.load(Ordering::SeqCst),
            16 * CHUNK,
            "上限で読むのをやめないこと"
        );
    }

    /// **読み捨てにも上限がある。** 無制限に読まない。
    ///
    /// 上限を外すと `produced` が本文全体になり、このテストが落ちる。
    #[tokio::test]
    async fn draining_stops_at_the_cap() {
        let total = 256;
        let limit = 4 * CHUNK;
        let drain_cap = 8 * CHUNK;
        let (body, produced) = counted(total);

        let outcome = read_capped(body, limit, drain_cap).await;

        assert_eq!(
            outcome,
            BodyRead::TooLarge(Drained::CapReached),
            "読み切れなかったことを区別できること"
        );
        let read = produced.load(Ordering::SeqCst);
        assert!(
            read <= limit + drain_cap + CHUNK,
            "読み捨ての上限を超えて読まないこと: {read} バイト"
        );
        assert!(
            read < total * CHUNK,
            "本文全体を読んでしまっている（上限が効いていない）: {read} バイト"
        );
    }

    /// 上限を跨いだ塊が読み捨て予算より大きければ、1塊も追加で読まない。
    #[tokio::test]
    async fn a_chunk_larger_than_the_drain_budget_stops_immediately() {
        let (body, produced) = counted(16);

        let outcome = read_capped(body, 0, CHUNK / 2).await;

        assert_eq!(outcome, BodyRead::TooLarge(Drained::CapReached));
        assert_eq!(
            produced.load(Ordering::SeqCst),
            CHUNK,
            "予算を使い切ったら追加で読まないこと"
        );
    }

    /// 本文の読み取り失敗は上限超過と区別する。
    #[tokio::test]
    async fn a_read_error_is_not_reported_as_too_large() {
        let body = stream::iter([
            Ok(Bytes::from_static(b"ab")),
            Err("接続が切れた"),
            Ok(Bytes::from_static(b"cd")),
        ]);

        let outcome = read_capped(Box::pin(body), 1_024, 1_024).await;

        assert_eq!(outcome, BodyRead::Failed);
    }

    /// 空の本文は空として読める（GET などが通る道）。
    #[tokio::test]
    async fn an_empty_body_is_complete() {
        let body = stream::iter(Vec::<Result<Bytes, Infallible>>::new());

        let outcome = read_capped(Box::pin(body), 1_024, 1_024).await;

        assert_eq!(outcome, BodyRead::Complete(Vec::new()));
    }

    /// 上限は axum の既定（2MB）と同じ。
    ///
    /// **大きくすると、この範囲の 413 が「読み捨ての無い抽出器経路」から返るようになり、
    /// 1リクエストあたりのメモリも `MAX_BODY_BYTES` まで伸びる**（モジュールの doc）。
    #[test]
    fn the_limit_matches_the_axum_default() {
        assert_eq!(
            MAX_BODY_BYTES, 2_097_152,
            "axum_core の DEFAULT_LIMIT と同じ値にすること"
        );
    }

    /// **`CapReached` の手掛かりは既定のログ設定で残る。**
    ///
    /// これが `debug` に戻されると、**413 を届けられなかった唯一の証拠**が
    /// 既定フィルタ（`warn,personacore=info`）に落とされて消える。
    /// クライアント側には「接続がリセットされた」としか見えないので、
    /// **消えたことは運用中にしか露見しない。**
    #[test]
    fn a_cap_reached_drain_is_visible_under_the_default_log_filter() {
        let output = captured_under_default_filter(|| {
            log_oversized_body("/v1/chat/completions", Drained::CapReached);
        });

        assert!(
            output.contains(CAP_REACHED_MARKER),
            "413 を失いうる側は既定の設定で残すこと: {output:?}"
        );
    }

    /// **`Ended` は既定のログ設定に出さない。**
    ///
    /// こちらは 413 が届いているので、運用者に見せる必要が無い。
    /// 両方を同じ深刻度に揃える変更（どちらの向きでも）をここで止める。
    ///
    /// # 「出ないこと」の前に「出ること」を確かめる
    ///
    /// 否定だけを主張すると、**イベントが丸ごと消えても緑のまま**になる
    /// （`debug!` を削除する変更、`tracing` の callsite interest が
    /// `never` に焼き付く事故のどちらでも）。
    #[test]
    fn an_ended_drain_stays_out_of_the_default_log_filter() {
        // **まず「記録自体はしている」ことを確かめる。**
        let verbose = captured_with_filter(VERBOSE_FILTER, || {
            log_oversized_body("/v1/chat/completions", Drained::Ended);
        });
        // **経路の欄は `Ended` の行の中で見る**（作業単位 B-2 / `crate::testlog` の doc）。
        // **欄はトークンとして突き合わせる**（作業単位 B-12）——`contains` だと
        // `path=/v1/chat/completions/extra` のような**別の経路**にも当たる。
        let ended = line_containing(&verbose, ENDED_MARKER);
        assert!(
            has_field(ended, "path=/v1/chat/completions"),
            "前提: どの経路で起きたかを載せること: {ended:?}"
        );

        let output = captured_under_default_filter(|| {
            log_oversized_body("/v1/chat/completions", Drained::Ended);
        });

        assert!(
            !output.contains(CAP_REACHED_MARKER),
            "413 が届いた側を「届かないかもしれない」と報告しないこと: {output:?}"
        );
        assert!(
            output.is_empty(),
            "既定の設定では debug ごと落ちること: {output:?}"
        );
    }

    /// 既定の読み捨て予算は、上限を跨いだ塊を吸ってもなお残る。
    ///
    /// **予算が 0 だと超過分を一切吸えず、直す前と同じ壊れ方に戻る。**
    #[tokio::test]
    async fn the_default_drain_budget_absorbs_an_overshoot() {
        let (body, produced) = counted(16);

        let outcome = read_capped(body, 4 * CHUNK, MAX_DRAIN_BYTES).await;

        assert_eq!(outcome, BodyRead::TooLarge(Drained::Ended));
        assert_eq!(produced.load(Ordering::SeqCst), 16 * CHUNK);
    }
}
