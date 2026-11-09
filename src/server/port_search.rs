//! **待ち受けのポートが使われていたら、空いている次の番号で待ち受ける**
//! （[ADR-0057](../../docs/adr/0057-the-listening-port-shifts-when-taken-and-is-set-from-the-screen.md)
//! §決定1・3・4 / 作業単位 F-11b / marukei/personacore#14）。
//!
//! # 探し方（[`PortSearch::plan`] が作る候補の列。**純粋関数**）
//!
//! | 出どころ | 候補 |
//! |---|---|
//! | `environment`（`PERSONACORE_PORT`。明示） | **元の番号だけ**（ずらさない。塞がっていれば B-76 の文で止まる） |
//! | `saved` / `default` | **元の番号から `+`[`PORT_SHIFT_SPAN`] まで**（65535 で打ち切り）。順に試す |
//! | 元の番号が `0`（OS に選ばせる） | **`0` だけ**（ずらしの対象ではない。出どころを問わない） |
//!
//! * **飛ばす番号**は、`upstreams.json` の全部のプロファイルの `gguf` の面が使う番号
//!   （`AppConfig::with_face_ports` / `main` が読む）。**明示していなければ元の番号も飛ばす。**
//!   **飛ばした番号も範囲の 10 に数える**（範囲は元の番号で決まり、飛ばしても伸びない）。
//! * **ずらす失敗は `AddrInUse` と `PermissionDenied`**（Windows の除外ポート範囲）だけ（[`shifts_past`]）。
//!   ほかの失敗では、ずらさずに止まる。
//! * **試す番号は [`BindAddr::loopback`] で作る**（`127.0.0.1` 以外を開く道を作らない）。
//! * **ずらした番号は覚えない**——次の起動は、また元の番号から試す。
//!
//! # 開く処理（[`open_first`]）
//!
//! **開き方（`TcpListener::bind`）は呼び出し側が渡す**——`App::bind_with` は本物の `bind` を、単体テストは
//! 作り物の結果（`PermissionDenied` など、本物では作りにくい失敗）を渡す。**列の歩き方は1つだけ。**
//! **`Store::connect` より前で完結する**（`App::bind` の doc の約束 —— 開けなくても閉じるものが無い）。
//!
//! # 知らせる
//!
//! **ずらしたら、`App::bind_with` が警告を1本出す**（[`shifted_message`] と、元の番号・実際の番号・出どころ・
//! 飛ばした番号の欄）。`GET /admin/server` は [`OpenedPort`] を `listening_port_requested` /
//! `listening_port_shifted` / `listening_port_skipped` として言う（`listening_port` モジュール）。
//! **起動ログ `設定を読み込みました` の `bind=` は、元の番号（出どころの番号。試したとは限らない）のまま**
//! （`main` の `log_settings` / ADR-0057 §決定4 の追記）。

use std::collections::BTreeSet;
use std::future::Future;

use super::config::{BindAddr, PortSource};

/// **元の番号から、いくつ先まで試すか**（ADR-0057 §決定3）。
///
/// **実測由来の値ではない。** 外れたとき（範囲が全部埋まっている機）は、止まって範囲と直し方を言うだけで、
/// 壊れる側には倒れない（ADR-0036）。**設定できるようにもしない。**
pub const PORT_SHIFT_SPAN: u16 = 10;

/// **候補の列の1つ。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PortStep {
    /// この番号を開いてみる。
    Try(u16),
    /// **開かずに飛ばす**（llama の面が使う番号）。範囲の数には入る。
    Skip(u16),
}

impl PortStep {
    const fn port(self) -> u16 {
        match self {
            Self::Try(port) | Self::Skip(port) => port,
        }
    }
}

/// **この起動が待ち受けを探す計画**（候補の列）。**作るのは [`Self::plan`] だけ。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PortSearch {
    requested: u16,
    source: PortSource,
    /// **ずらすか**（明示でも `0` でもない）。**列の長さからは決めない**——保存した番号が 65535 なら、
    /// ずらす起動でも列は1つしか無い（そこで明示と取り違えると、範囲でなく1つの番号の文で止まる）。
    shifts: bool,
    steps: Vec<PortStep>,
}

impl PortSearch {
    /// **候補の列を作る。純粋関数**（モジュールの doc の表）。
    ///
    /// `face_ports` は `upstreams.json` の全部のプロファイルの `gguf` の面が使う番号。
    /// **明示（`environment`）と `0` では見ない**——明示した番号が面と同じなら、今までどおり
    /// `LaunchPlan::port_conflict`（B-52）が止める。
    pub(crate) fn plan(requested: u16, source: PortSource, face_ports: &BTreeSet<u16>) -> Self {
        let shifts = source != PortSource::Environment && requested != 0;
        let steps = if shifts {
            // **`RangeInclusive<u16>` は `u16::MAX` を含めて止まる**（`saturating_add` で上端を 65535 に打ち切る）。
            (requested..=requested.saturating_add(PORT_SHIFT_SPAN))
                .map(|port| {
                    if face_ports.contains(&port) {
                        PortStep::Skip(port)
                    } else {
                        PortStep::Try(port)
                    }
                })
                .collect()
        } else {
            vec![PortStep::Try(requested)]
        };
        Self {
            requested,
            source,
            shifts,
            steps,
        }
    }

    /// 範囲の最後の番号（試した範囲を言う文に使う）。
    fn last(&self) -> u16 {
        self.steps.last().map_or(self.requested, |step| step.port())
    }

    #[cfg(test)]
    fn steps(&self) -> &[PortStep] {
        &self.steps
    }
}

/// **この起動が待ち受けを開けた結果**（`GET /admin/server` と警告が言う）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenedPort {
    /// **元の番号**（出どころの番号。試したとは限らない。`PERSONACORE_PORT=0` なら `0`）。
    requested: u16,
    /// 元の番号の出どころ。
    source: PortSource,
    /// **開けた候補**（`0` で開いたなら `0`。実際の番号は `local_addr` で読む）。
    opened: u16,
    /// **開けるまでに飛ばした番号**（llama の面が使う番号。開けた番号より後ろの番号は入らない）。
    skipped: Vec<u16>,
}

impl OpenedPort {
    pub(crate) const fn requested(&self) -> u16 {
        self.requested
    }

    pub(crate) const fn source(&self) -> PortSource {
        self.source
    }

    /// **元の番号とは別の番号で開けたか。**
    pub(crate) const fn shifted(&self) -> bool {
        self.opened != self.requested
    }

    pub(crate) fn skipped(&self) -> &[u16] {
        &self.skipped
    }

    /// **元の番号そのものを、llama の面が使う番号として飛ばしたか**（塞がっていたのではない）。
    ///
    /// ずらしの警告（[`shifted_message`]）はこれで文を選ぶ——「使えなかった」「次はまた元の番号から試す」は、
    /// 面の番号で飛ばした元の番号には偽になる（次の起動でも試さない）。
    pub(crate) fn requested_was_skipped(&self) -> bool {
        self.skipped.first() == Some(&self.requested)
    }
}

/// **範囲の番号をどれも開けなかった**（`ServerError::PortsTaken` の `#[source]`）。
///
/// **番号は置き場所でも秘密でもない**ので、文面に載せる（`main` は利用者向けの文を
/// `config::ports_in_use_message` で別に足す）。最後に試した番号の OS の失敗を `#[source]` に持つ
/// （全部を飛ばしたときは無い）。**範囲の中で飛ばした番号**も持つ——利用者向けの文は、それが在るときだけ
/// 「面の番号は試していない」と言い、1つも試せなかったときはそう言う（読めない `upstreams.json` で飛ばす番号が
/// 空の起動に、飛ばしたと言わない）。
///
/// **この型の文**（`main` が出す鎖の `Caused by` の行）も、試した数と範囲の幅で偽にならない形にする
/// （[`ports_taken_text`]）。**`ServerError::PortsTaken` の `Display` は定数のまま**で、番号を含むのはこちらだけ。
#[derive(Debug, thiserror::Error)]
#[error("{}", ports_taken_text(*.first, *.last, .skipped))]
pub struct PortsTaken {
    first: u16,
    last: u16,
    skipped: Vec<u16>,
    #[source]
    source: Option<std::io::Error>,
}

/// **[`PortsTaken`] の文。純粋関数。**
///
/// | 範囲の中 | 文 |
/// |---|---|
/// | 1つも試していない（全部を飛ばした） | 「ポート {範囲} は、どれも llama の面が使う番号として飛ばしたので、1つも試していません」 |
/// | 範囲が1つで、それを試した | 「ポート {番号} を試しましたが、開けませんでした」 |
/// | それ以外 | 「ポート {範囲} のうち {試した数} 個を順に試しましたが、どれも開けませんでした」 |
///
/// 範囲が1つの番号なら「〜」を書かない。
fn ports_taken_text(first: u16, last: u16, skipped: &[u16]) -> String {
    let range = if first == last {
        first.to_string()
    } else {
        format!("{first}〜{last}")
    };
    let total = usize::from(last.saturating_sub(first)) + 1;
    let tried = total.saturating_sub(skipped.len());
    if tried == 0 {
        // **「どれも」の後は空白**（続く語が英字の `llama`）。
        let every = if total > 1 { "どれも " } else { "" };
        format!(
            "ポート {range} は、{every}llama の面が使う番号として飛ばしたので、1つも試していません"
        )
    } else if total == 1 {
        format!("ポート {range} を試しましたが、開けませんでした")
    } else {
        format!("ポート {range} のうち {tried} 個を順に試しましたが、どれも開けませんでした")
    }
}

impl PortsTaken {
    /// 元の番号（出どころの番号。試したとは限らない）。
    #[must_use]
    pub const fn first(&self) -> u16 {
        self.first
    }

    /// 範囲の最後の番号。
    #[must_use]
    pub const fn last(&self) -> u16 {
        self.last
    }

    /// **範囲の中で、llama の面が使う番号として飛ばした番号**（試していない）。昇順。
    #[must_use]
    pub fn skipped(&self) -> &[u16] {
        &self.skipped
    }
}

/// **開けなかった理由**（`App::bind_with` が `ServerError` に写す）。
#[derive(Debug)]
pub(crate) enum SearchFailure {
    /// **ずらさない失敗**（明示した番号が使われていた・ずらす対象でない失敗）。**今までどおり `ServerError::Bind`。**
    Stopped(std::io::Error),
    /// **範囲を全部試して、どれも開けなかった。**
    Exhausted(PortsTaken),
}

/// **この失敗なら次の番号へ進む**（`AddrInUse` と、Windows の除外ポート範囲で返る `PermissionDenied`）。**純粋関数。**
pub(crate) fn shifts_past(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
    )
}

/// **候補の列を順に開き、最初に開けたものを返す**（モジュールの doc）。
///
/// `open` は1つの番号を開く処理（`App::bind_with` では `TcpListener::bind`）。**宛先は必ず
/// [`BindAddr::loopback`] で作って渡す。**
///
/// # Errors
///
/// ずらさない失敗（[`SearchFailure::Stopped`]）か、範囲を全部試して開けなかったとき（[`SearchFailure::Exhausted`]）。
pub(crate) async fn open_first<T, F, Fut>(
    search: &PortSearch,
    mut open: F,
) -> Result<(T, OpenedPort), SearchFailure>
where
    F: FnMut(BindAddr) -> Fut,
    Fut: Future<Output = std::io::Result<T>>,
{
    let mut skipped = Vec::new();
    let mut last_error = None;
    for step in &search.steps {
        match *step {
            PortStep::Skip(port) => skipped.push(port),
            PortStep::Try(port) => match open(BindAddr::loopback(port)).await {
                Ok(listener) => {
                    return Ok((
                        listener,
                        OpenedPort {
                            requested: search.requested,
                            source: search.source,
                            opened: port,
                            skipped,
                        },
                    ));
                }
                Err(err) if search.shifts && shifts_past(&err) => last_error = Some(err),
                Err(err) => return Err(SearchFailure::Stopped(err)),
            },
        }
    }
    Err(SearchFailure::Exhausted(PortsTaken {
        first: search.requested,
        last: search.last(),
        skipped,
        source: last_error,
    }))
}

/// **ずらしたときの警告の本文**（ADR-0057 §決定4）。**番号を2つ言う。**
///
/// | 元の番号 | 文の頭 |
/// |---|---|
/// | 塞がっていた（`requested_was_skipped` が偽） | 「ポート {元} は使えなかったので、{実際} で待ち受けます」。次の起動はまた元の番号から試す |
/// | **llama の面が使う番号として飛ばした**（真） | 「ポート {元} は上流プロファイルの llama の面が使う番号なので、{実際} で待ち受けます」。次の起動でも試さないので「また試す」と言わない |
///
/// **写しは `tests/listening_port_cli.rs` の `shifted_head`（文の頭だけ）**——括弧の中を変えても、そちらは
/// 赤にならない（括弧の中の「画面の「設定」」は、このモジュールの単体テストが見る）。元の番号・実際の番号・
/// 出どころ・飛ばした番号は、同じ行の欄にも出る。
pub(crate) fn shifted_message(requested: u16, port: u16, requested_was_skipped: bool) -> String {
    if requested_was_skipped {
        format!(
            "ポート {requested} は上流プロファイルの llama の面が使う番号なので、{port} で待ち受けます\
             （ずらした番号は覚えません。\
             ブックマークや外部のクライアントに書いた番号は、実際の番号に合わせてください。\
             番号は画面の「設定」で変えられます）"
        )
    } else {
        format!(
            "ポート {requested} は使えなかったので、{port} で待ち受けます\
             （ずらした番号は覚えず、次の起動ではまた {requested} から試します。\
             ブックマークや外部のクライアントに書いた番号は、実際の番号に合わせてください。\
             番号は画面の「設定」で変えられます）"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PortStep::{Skip, Try};

    fn faces(ports: &[u16]) -> BTreeSet<u16> {
        ports.iter().copied().collect()
    }

    fn tries(range: std::ops::RangeInclusive<u16>) -> Vec<PortStep> {
        range.map(Try).collect()
    }

    /// **保存・既定は、元の番号から +10 まで（11 個）を順に試す。**
    #[test]
    fn a_saved_or_default_port_tries_the_next_ten_in_order() {
        for source in [PortSource::Saved, PortSource::Default] {
            let search = PortSearch::plan(8080, source, &faces(&[]));
            assert_eq!(search.steps(), tries(8080..=8090).as_slice(), "{source:?}");
            assert_eq!(search.last(), 8090);
            assert!(search.shifts);
        }
    }

    /// **明示（`PERSONACORE_PORT`）は元の番号だけ。** 面と同じ番号でも飛ばさない（B-52 が止める）。
    #[test]
    fn an_explicit_port_is_the_only_candidate_even_if_a_face_uses_it() {
        let search = PortSearch::plan(8081, PortSource::Environment, &faces(&[8081, 8082]));
        assert_eq!(search.steps(), [Try(8081)].as_slice());
        assert_eq!(search.last(), 8081);
        assert!(!search.shifts);
    }

    /// **`0`（OS に選ばせる）はずらさない**——出どころを問わない（テストが `with_bind(0)` で組む形も）。
    #[test]
    fn port_zero_is_never_shifted() {
        for source in PortSource::ALL {
            let search = PortSearch::plan(0, source, &faces(&[0, 1]));
            assert_eq!(search.steps(), [Try(0)].as_slice(), "{source:?}");
            assert!(!search.shifts);
        }
    }

    /// **範囲は 65535 で打ち切る**（あふれて 0 や小さい番号へ回り込まない）。
    #[test]
    fn the_range_stops_at_65535() {
        let search = PortSearch::plan(65530, PortSource::Saved, &faces(&[]));
        assert_eq!(search.steps(), tries(65530..=65535).as_slice());
        assert_eq!(search.last(), 65535);

        let at_top = PortSearch::plan(65535, PortSource::Default, &faces(&[]));
        assert_eq!(at_top.steps(), [Try(65535)].as_slice());
        assert_eq!(at_top.last(), 65535);
    }

    /// **面の番号は飛ばし、飛ばした番号も 10 に数える**（範囲は伸びない）。範囲の外の面の番号は列に出ない。
    #[test]
    fn face_ports_are_skipped_and_still_count() {
        let search = PortSearch::plan(8080, PortSource::Default, &faces(&[8081, 8085, 8091, 9000]));
        let mut expected = tries(8080..=8090);
        expected[1] = Skip(8081);
        expected[5] = Skip(8085);
        assert_eq!(search.steps(), expected.as_slice());
        assert_eq!(search.last(), 8090, "飛ばした分だけ範囲を伸ばしている");
    }

    /// **明示していなければ、元の番号も面と同じなら飛ばす。**
    #[test]
    fn the_requested_port_itself_is_skipped_when_a_face_uses_it() {
        let search = PortSearch::plan(9100, PortSource::Saved, &faces(&[9100]));
        assert_eq!(search.steps()[0], Skip(9100));
        assert_eq!(search.steps()[1], Try(9101));
        assert_eq!(search.steps().len(), 11);
    }

    fn io(kind: std::io::ErrorKind) -> std::io::Error {
        std::io::Error::from(kind)
    }

    /// **作り物の開き方**: `taken` の番号は `kind` で断り、それ以外は開けたことにして番号を返す。
    /// 試した番号を順に `seen` へ書く。
    async fn walk(
        search: &PortSearch,
        taken: &[(u16, std::io::ErrorKind)],
        seen: &mut Vec<BindAddr>,
    ) -> Result<(u16, OpenedPort), SearchFailure> {
        open_first(search, |addr| {
            seen.push(addr);
            let refused = taken
                .iter()
                .find(|(port, _)| *port == addr.port())
                .map(|(_, kind)| *kind);
            async move {
                match refused {
                    Some(kind) => Err(io(kind)),
                    None => Ok(addr.port()),
                }
            }
        })
        .await
    }

    /// **元の番号が開ければ、ずらさない**（飛ばした番号も無い。次の番号を試しもしない）。
    #[tokio::test]
    async fn a_free_requested_port_opens_without_shifting() {
        let search = PortSearch::plan(8080, PortSource::Default, &faces(&[8081]));
        let mut seen = Vec::new();
        let (port, opened) = walk(&search, &[], &mut seen).await.expect("開けること");
        assert_eq!(port, 8080);
        assert_eq!(seen, vec![BindAddr::loopback(8080)]);
        assert!(!opened.shifted());
        assert_eq!(opened.requested(), 8080);
        assert_eq!(opened.source(), PortSource::Default);
        assert!(opened.skipped().is_empty());
    }

    /// **使われていたら次へ。面の番号は開きに行かずに飛ばし、飛ばした番号を言う。**
    /// **見本の 8081 の形**——既定の 8080 が塞がると 8081 を飛ばして 8082 で開く。
    #[tokio::test]
    async fn a_taken_port_moves_to_the_next_free_one_past_the_face_ports() {
        let search = PortSearch::plan(8080, PortSource::Default, &faces(&[8081]));
        let mut seen = Vec::new();
        let (port, opened) = walk(&search, &[(8080, std::io::ErrorKind::AddrInUse)], &mut seen)
            .await
            .expect("開けること");
        assert_eq!(port, 8082);
        assert_eq!(
            seen,
            vec![BindAddr::loopback(8080), BindAddr::loopback(8082)],
            "飛ばす番号を開きに行っている"
        );
        assert!(opened.shifted());
        assert_eq!(opened.requested(), 8080);
        assert_eq!(opened.skipped(), [8081].as_slice());
    }

    /// **`PermissionDenied`（Windows の除外ポート範囲）でもずれる。**
    #[tokio::test]
    async fn permission_denied_also_moves_to_the_next_port() {
        let search = PortSearch::plan(9200, PortSource::Saved, &faces(&[]));
        let mut seen = Vec::new();
        let (port, opened) = walk(
            &search,
            &[
                (9200, std::io::ErrorKind::PermissionDenied),
                (9201, std::io::ErrorKind::AddrInUse),
            ],
            &mut seen,
        )
        .await
        .expect("開けること");
        assert_eq!(port, 9202);
        assert!(opened.shifted());
        assert_eq!(opened.source(), PortSource::Saved);
        assert!(opened.skipped().is_empty());
    }

    /// **元の番号を飛ばして開いたら「ずらした」**（元の番号は塞がっていなくても、面が使う）。
    #[tokio::test]
    async fn skipping_the_requested_port_counts_as_shifted() {
        let search = PortSearch::plan(9100, PortSource::Saved, &faces(&[9100]));
        let mut seen = Vec::new();
        let (port, opened) = walk(&search, &[], &mut seen).await.expect("開けること");
        assert_eq!(port, 9101);
        assert_eq!(seen, vec![BindAddr::loopback(9101)]);
        assert!(opened.shifted());
        assert_eq!(opened.skipped(), [9100].as_slice());
    }

    /// **ほかの失敗では、ずらさずに止まる**（次の番号を試さない）。
    #[tokio::test]
    async fn another_failure_stops_without_shifting() {
        let search = PortSearch::plan(8080, PortSource::Default, &faces(&[]));
        let mut seen = Vec::new();
        let outcome = walk(&search, &[(8080, std::io::ErrorKind::Other)], &mut seen).await;
        assert!(
            matches!(&outcome, Err(SearchFailure::Stopped(err)) if err.kind() == std::io::ErrorKind::Other),
            "{outcome:?}"
        );
        assert_eq!(seen, vec![BindAddr::loopback(8080)]);
    }

    /// **明示した番号が使われていたら、ずらさずに止まる**（今までどおりの `Bind` の失敗）。
    #[tokio::test]
    async fn an_explicit_port_in_use_stops_as_before() {
        let search = PortSearch::plan(8080, PortSource::Environment, &faces(&[]));
        let mut seen = Vec::new();
        let outcome = walk(&search, &[(8080, std::io::ErrorKind::AddrInUse)], &mut seen).await;
        assert!(
            matches!(&outcome, Err(SearchFailure::Stopped(err)) if err.kind() == std::io::ErrorKind::AddrInUse),
            "{outcome:?}"
        );
        assert_eq!(seen, vec![BindAddr::loopback(8080)]);
    }

    /// **範囲が全部だめなら、試した範囲（元の番号〜最後）を言って止まる。** 最後の OS の失敗を持つ。
    #[tokio::test]
    async fn every_port_taken_names_the_range() {
        let search = PortSearch::plan(8080, PortSource::Default, &faces(&[8081]));
        let taken: Vec<(u16, std::io::ErrorKind)> = (8080..=8090)
            .map(|port| (port, std::io::ErrorKind::AddrInUse))
            .collect();
        let mut seen = Vec::new();
        let outcome = walk(&search, &taken, &mut seen).await;
        let Err(SearchFailure::Exhausted(taken)) = outcome else {
            panic!("範囲を言って止まっていない: {outcome:?}");
        };
        assert_eq!((taken.first(), taken.last()), (8080, 8090));
        assert!(
            std::error::Error::source(&taken).is_some(),
            "OS の失敗を落とした"
        );
        assert_eq!(seen.len(), 10, "飛ばす 8081 を除く 10 個を試すこと");
    }

    /// **保存した番号が 65535（範囲が1つの番号しか無い）でも、明示とは扱わない**——塞がっていれば
    /// 範囲（65535〜65535）を言って止まる（`ports_in_use_message`）。1つの番号の文（B-76 の明示の文）にしない。
    ///
    /// **列の長さで「ずらすか」を決めると、ここで明示と取り違える**（作業単位 F-11b の変異の準備中に見つけた）。
    #[tokio::test]
    async fn a_saved_port_at_the_top_is_still_a_range_of_one() {
        let search = PortSearch::plan(65535, PortSource::Saved, &faces(&[]));
        let mut seen = Vec::new();
        let outcome = walk(
            &search,
            &[(65535, std::io::ErrorKind::AddrInUse)],
            &mut seen,
        )
        .await;
        let Err(SearchFailure::Exhausted(taken)) = outcome else {
            panic!("範囲として止まっていない（明示と取り違えた）: {outcome:?}");
        };
        assert_eq!((taken.first(), taken.last()), (65535, 65535));

        // **`PermissionDenied` も同じ**（ずらす失敗なので、明示の `Bind` にしない）。
        let outcome = walk(
            &search,
            &[(65535, std::io::ErrorKind::PermissionDenied)],
            &mut seen,
        )
        .await;
        assert!(
            matches!(outcome, Err(SearchFailure::Exhausted(_))),
            "{outcome:?}"
        );
    }

    /// **保存した番号が 65535 で、それが面の番号なら、1つも試さずに範囲（65535）で止まり、飛ばした番号を持つ**
    /// ——利用者向けの文はこれで「試せる番号が無かった」と言う（`config::ports_in_use_message`）。
    #[tokio::test]
    async fn a_saved_top_port_that_a_face_uses_tries_nothing_and_says_so() {
        let search = PortSearch::plan(65535, PortSource::Saved, &faces(&[65535]));
        assert_eq!(search.steps(), [Skip(65535)].as_slice());
        let mut seen = Vec::new();
        let outcome = walk(&search, &[], &mut seen).await;
        let Err(SearchFailure::Exhausted(taken)) = outcome else {
            panic!("範囲として止まっていない: {outcome:?}");
        };
        assert!(seen.is_empty(), "飛ばす番号を開きに行っている");
        assert_eq!((taken.first(), taken.last()), (65535, 65535));
        assert_eq!(taken.skipped(), [65535].as_slice());
        let message = super::super::config::ports_in_use_message(
            taken.first(),
            taken.last(),
            taken.skipped(),
        );
        assert!(
            message.contains("試せる番号がありませんでした"),
            "{message}"
        );
        assert!(!message.contains("他のプログラム"), "{message}");
    }

    /// **`PortsTaken` の文（鎖の `Caused by` の行）は、試した数と範囲の幅で偽にならない**（F-11 の再レビュー是正）。
    #[test]
    fn the_ports_taken_text_never_claims_more_than_was_tried() {
        assert_eq!(
            ports_taken_text(8080, 8090, &[]),
            "ポート 8080〜8090 のうち 11 個を順に試しましたが、どれも開けませんでした"
        );
        assert_eq!(
            ports_taken_text(8080, 8090, &[8081, 8085]),
            "ポート 8080〜8090 のうち 9 個を順に試しましたが、どれも開けませんでした"
        );
        assert_eq!(
            ports_taken_text(65535, 65535, &[]),
            "ポート 65535 を試しましたが、開けませんでした"
        );
        assert_eq!(
            ports_taken_text(65535, 65535, &[65535]),
            "ポート 65535 は、llama の面が使う番号として飛ばしたので、1つも試していません"
        );
        let all: Vec<u16> = (9300..=9310).collect();
        assert_eq!(
            ports_taken_text(9300, 9310, &all),
            "ポート 9300〜9310 は、どれも llama の面が使う番号として飛ばしたので、1つも試していません"
        );
        // **`Display` がこの文を使う**（型の文そのもの）。
        let taken = PortsTaken {
            first: 65535,
            last: 65535,
            skipped: vec![65535],
            source: None,
        };
        assert_eq!(taken.to_string(), ports_taken_text(65535, 65535, &[65535]));
    }

    /// **元の番号を飛ばした後に、続けて別の面の番号も飛ばした形でも、元の番号を飛ばしたと読む**
    /// （飛ばした番号の「先頭」が元の番号か / F-11 の再レビュー是正。面が1つだけの腕では、先頭と末尾を取り違える
    /// 実装が見分けられなかった）。
    #[tokio::test]
    async fn a_skipped_requested_port_followed_by_another_skip_is_still_the_requested_one() {
        let mut seen = Vec::new();
        let (port, opened) = walk(
            &PortSearch::plan(9100, PortSource::Saved, &faces(&[9100, 9101])),
            &[],
            &mut seen,
        )
        .await
        .expect("開けること");
        assert_eq!(port, 9102);
        assert_eq!(opened.skipped(), [9100, 9101].as_slice());
        assert!(opened.requested_was_skipped());
    }

    /// **全部を飛ばしたときも範囲を言って止まる**（OS の失敗は無い）。
    #[tokio::test]
    async fn every_port_skipped_names_the_range_too() {
        let all: Vec<u16> = (9300..=9310).collect();
        let search = PortSearch::plan(9300, PortSource::Saved, &faces(&all));
        let mut seen = Vec::new();
        let outcome = walk(&search, &[], &mut seen).await;
        let Err(SearchFailure::Exhausted(taken)) = outcome else {
            panic!("範囲を言って止まっていない: {outcome:?}");
        };
        assert_eq!((taken.first(), taken.last()), (9300, 9310));
        assert!(std::error::Error::source(&taken).is_none());
        assert!(seen.is_empty(), "飛ばす番号を開きに行っている");
    }

    /// **同梱の見本 `upstreams.json` の llama の面は 8081 なので、既定の 8080 が塞がると 8081 を飛ばして
    /// 8082 で待ち受ける**（ADR-0057 §決定3「見本の 8081 もここで飛ぶ」）。
    ///
    /// **利用者の 8080 に触らずに見る**（exe のテストは既定の番号を経由しない）——見本を本番と同じ読み
    /// （`parse_profiles` → `launch_ports`）に通し、作り物の開き方で 8080 を塞ぐ。
    #[tokio::test]
    async fn the_sample_profiles_make_a_taken_default_port_skip_8081() {
        let sample = super::super::upstreams::parse_profiles(
            super::super::upstreams::SAMPLE_PROFILES,
            Some(std::path::Path::new("home")),
        )
        .expect("見本が読めること");
        let face_ports = sample.launch_ports();
        assert_eq!(face_ports, faces(&[8081]), "前提: 見本の llama の面の番号");

        let search = PortSearch::plan(
            super::super::config::DEFAULT_PORT,
            PortSource::Default,
            &face_ports,
        );
        let mut seen = Vec::new();
        let (port, opened) = walk(&search, &[(8080, std::io::ErrorKind::AddrInUse)], &mut seen)
            .await
            .expect("開けること");
        assert_eq!(port, 8082);
        assert_eq!(opened.skipped(), [8081].as_slice());
    }

    /// **ずらす失敗は2つだけ。**
    #[test]
    fn only_addr_in_use_and_permission_denied_shift() {
        assert!(shifts_past(&io(std::io::ErrorKind::AddrInUse)));
        assert!(shifts_past(&io(std::io::ErrorKind::PermissionDenied)));
        for kind in [
            std::io::ErrorKind::AddrNotAvailable,
            std::io::ErrorKind::Other,
            std::io::ErrorKind::InvalidInput,
        ] {
            assert!(!shifts_past(&io(kind)), "{kind:?}");
        }
    }

    /// **警告は元の番号と実際の番号を、その順で言う。**
    #[test]
    fn the_warning_names_both_ports_in_order() {
        let message = shifted_message(8080, 8082, false);
        assert!(
            message.starts_with("ポート 8080 は使えなかったので、8082 で待ち受けます"),
            "{message}"
        );
        assert!(
            message.contains("次の起動ではまた 8080 から試します"),
            "{message}"
        );
        assert!(message.contains("画面の「設定」"), "{message}");
    }

    /// **元の番号を面の番号として飛ばしたときは、「使えなかった」「また試す」と言わない**
    /// （塞がっていたのではなく、次の起動でも試さない）。
    #[test]
    fn the_warning_for_a_skipped_requested_port_says_why_and_promises_no_retry() {
        let message = shifted_message(9100, 9101, true);
        assert!(
            message.starts_with(
                "ポート 9100 は上流プロファイルの llama の面が使う番号なので、9101 で待ち受けます"
            ),
            "{message}"
        );
        assert!(!message.contains("使えなかった"), "{message}");
        assert!(!message.contains("また 9100 から試します"), "{message}");
        assert!(message.contains("画面の「設定」"), "{message}");
    }

    /// **元の番号を飛ばしたかは、飛ばした番号の先頭が元の番号か**で決まる（塞がっていた元の番号の後で面の番号を
    /// 飛ばした形は偽）。範囲が尽きたときは、範囲の中で飛ばした番号を全部持つ。
    #[tokio::test]
    async fn the_skipped_ports_tell_apart_a_skipped_requested_port_and_reach_the_failure() {
        let mut seen = Vec::new();
        let (_, skipped_first) = walk(
            &PortSearch::plan(9100, PortSource::Saved, &faces(&[9100])),
            &[],
            &mut seen,
        )
        .await
        .expect("開けること");
        assert!(skipped_first.requested_was_skipped());

        let (_, taken_first) = walk(
            &PortSearch::plan(9100, PortSource::Saved, &faces(&[9101])),
            &[(9100, std::io::ErrorKind::AddrInUse)],
            &mut seen,
        )
        .await
        .expect("開けること");
        assert!(!taken_first.requested_was_skipped());
        assert_eq!(taken_first.skipped(), [9101].as_slice());

        let taken: Vec<(u16, std::io::ErrorKind)> = (9100..=9110)
            .map(|port| (port, std::io::ErrorKind::AddrInUse))
            .collect();
        let outcome = walk(
            &PortSearch::plan(9100, PortSource::Saved, &faces(&[9103, 9107, 9200])),
            &taken,
            &mut seen,
        )
        .await;
        let Err(SearchFailure::Exhausted(exhausted)) = outcome else {
            panic!("範囲として止まっていない: {outcome:?}");
        };
        assert_eq!(exhausted.skipped(), [9103, 9107].as_slice());
    }
}
