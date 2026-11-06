//! **`Host` / `Origin` の検査**（作業単位 B-58）。**ルータの全部に効かせる**（画面・API を区別しない）。
//!
//! # 何を塞ぐか
//!
//! personacore は `127.0.0.1` にしか bind しないので、ネットワークの外からは届かない。
//! **残っていたのは「同じ機の上で動くブラウザ」経由の経路**である:
//!
//! * **DNS rebinding** —— 外のページ（`http://evil.example:<port>`）が、自分の名前の向き先を
//!   `127.0.0.1` へ差し替える。ブラウザから見ると同じオリジンのままなので、応答まで読める。
//!   **このときブラウザが送る `Host` は `evil.example:<port>`** なので、`Host` を見れば断れる。
//! * **外のオリジンからの書き込みの依頼** —— 応答は読めなくても、依頼そのものは届く。
//!   **ブラウザは `GET` / `HEAD` 以外の依頼に必ず `Origin` を付ける**ので、`Origin` を見れば断れる。
//!
//! 届いた先には、起動するコマンドを保存する口（`PUT /admin/upstream/profiles/{name}`）と、
//! それを起動する口（`POST /admin/upstream/profile`）がある。
//!
//! # 決め（司令塔の方針 / 2026-10-01）
//!
//! | 見出し | 通す | 断る（403） |
//! |---|---|---|
//! | `Host` | **無い**／`127.0.0.1:<port>`／`localhost:<port>` | それ以外の全部・**2つ以上** |
//! | `Origin` | **無い**／`http://127.0.0.1:<port>`／`http://localhost:<port>` | それ以外の全部（**`null` を含む**）・**2つ以上** |
//! | 要求の URI の authority | **無い**／`127.0.0.1:<port>`／`localhost:<port>`（`Host` と同じ規則） | それ以外の全部 |
//!
//! **`<port>` は実際に待ち受けているポート**（`App::serve` が `local_addr` から渡す）。
//! **設定値ではない** ——設定が 0（OS に選ばせる）でも正しく比べる。
//!
//! ## `Host` も authority も無い要求を通す理由
//!
//! **宛先の名前は2つの場所に載りうる。** HTTP/1.1 の普通の要求は `Host` に、
//! **HTTP/2 の要求（`:authority`）と、HTTP/1.1 の絶対形の要求行（`GET http://evil.example:<port>/`）は
//! 要求の URI に**載る（`Request::uri().authority()`）。hyper の待ち受けは h2c も受け付ける
//! （B-58 のレビューが実測）ので、**載っていれば両方とも見る。** 片方だけ見ると、もう片方に外の名前を
//! 書いた要求が通る（同じ危険に入口が2つある / CLAUDE.md）。
//!
//! **どちらも無い要求は通す。** **ブラウザは `Host` を省けない**（`fetch` でも書き換えられない禁止ヘッダ）
//! ので、`Host` の無い HTTP/1.1 の要求は DNS rebinding の経路にならない。h2c と絶対形の要求行も
//! ブラウザからは作れない。送れるのは同じ機の上のプログラムだけで、
//! **それはこの検査があっても無くても全部の口を呼べる**（認証が無い / docs/api/README.md のセキュリティの節）。
//! 断っても守れるものが無く、生の TCP で書く道具やテストを壊すだけなので通す。
//! **`Origin` は `Host` / authority の有無に関係なく見る。**
//!
//! ## 綴りの扱い
//!
//! * **名前の大文字小文字は区別しない**（`LOCALHOST:<port>` を通す）。DNS の名前は大文字小文字を
//!   区別せず、ブラウザは小文字にして送るので、通しても外のページが得るものは無い。
//! * **末尾の `.`（`localhost.`）は断る。** 正規の使い手は付けない。通す理由が無いものは通さない。
//! * **ポートを省いた形は 80 と読む**（`http` の既定）。**実際のポートが 80 のときだけ**
//!   `Host: 127.0.0.1` / `Origin: http://localhost` が通る。明示した `:80` も同じ。
//! * **ポートの綴りは、実際のポートの 10 進そのものだけ**（`08080` / `+8080` は断る）。
//! * **`[::1]` と `127.0.0.1` 以外の loopback（`127.0.0.2` 等）は断る** ——そこでは待ち受けていない
//!   （`BindAddr` は `127.0.0.1` だけ）ので、正規の要求がその名前で届くことは無い。
//! * **`Origin` は `http://` ＋ 上の `Host` と同じ規則の組だけ**。末尾の `/`・パス・`https` は断る
//!   （ブラウザが直列化するオリジンはその形にならない）。**`/` を別に探していない**のは、
//!   ポートの綴りも名前も完全一致で比べるので、`/` が混ざった綴りはそこで必ず外れるから
//!   （`/` を探す条件は等価変異だった / B-58 のレビュー）。
//!
//! # 応答
//!
//! **OpenAI の形の error**（[`ApiError::request_origin_not_allowed`]）。**届いた値は反射しない**
//! ——ログ（`tracing` の `warn`）にだけ、届いた `Host` / `Origin` を `Debug` の綴りで載せる
//! （制御文字はエスケープされるので、ログに行を差し込めない）。
//! **版のヘッダ（`x-personacore-api-version`）は付く** ——このレイヤは版のヘッダのレイヤの内側にある
//! （`server::router`。ADR-0047 §② P-2b「ルータに届いた要求への応答には付く」）。
//!
//! 見張り番は `tests/server_host_guard.rs`（実際の TCP）と、下の `mod tests`（判定の表とログ）。

use axum::extract::{Request, State};
use axum::http::HeaderValue;
use axum::http::header::{HOST, ORIGIN};
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};

use super::error::ApiError;

/// **ポートを省いた `Host` / `Origin` が指すポート**（`http` の既定）。
const HTTP_DEFAULT_PORT: u16 = 80;

/// **自分を指す名前**（`Host` / `Origin` の名前の部分。大文字小文字は区別しない）。
const LOOPBACK_NAMES: [&str; 2] = ["127.0.0.1", "localhost"];

/// **このサーバが実際に待ち受けているポート**（ミドルウェアの状態）。
#[derive(Debug, Clone, Copy)]
pub(super) struct ListeningPort(u16);

impl ListeningPort {
    pub(super) const fn new(port: u16) -> Self {
        Self(port)
    }
}

/// **断る理由**（ログに載せる）。**`Display` は定数**（CLAUDE.md §1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum Refusal {
    #[error("Host がこのサーバのものではありません")]
    Host,
    #[error("Origin がこのサーバのものではありません")]
    Origin,
    #[error("要求の宛先（authority）がこのサーバのものではありません")]
    Authority,
}

/// **`host:port` の組がこのサーバを指すか。純粋関数。**
///
/// 規則はモジュールの doc の「綴りの扱い」。
fn names_this_server(authority: &str, port: u16) -> bool {
    let (name, port_text) = match authority.rsplit_once(':') {
        Some((name, port_text)) => (name, Some(port_text)),
        None => (authority, None),
    };
    let port_matches = match port_text {
        // **10 進の綴りが一致すること**（`parse` は `+` や先頭の `0` を受け付けるので使わない）。
        Some(port_text) => port_text == port.to_string(),
        None => port == HTTP_DEFAULT_PORT,
    };
    port_matches
        && LOOPBACK_NAMES
            .iter()
            .any(|loopback| name.eq_ignore_ascii_case(loopback))
}

/// **`Origin` の値がこのサーバのオリジンか。純粋関数。** `null` は `false`。
fn is_own_origin(origin: &str, port: u16) -> bool {
    const SCHEME: &str = "http://";
    origin
        .get(..SCHEME.len())
        .filter(|scheme| scheme.eq_ignore_ascii_case(SCHEME))
        .and_then(|_| origin.get(SCHEME.len()..))
        .is_some_and(|authority| names_this_server(authority, port))
}

/// **見出しの値の並びが「無い」か「自分を指す1つ」か。純粋関数。**
///
/// **2つ以上は断る**（どちらを信じるかを決めない）。ASCII として読めない値も断る。
fn absent_or_single<'a>(
    mut values: impl Iterator<Item = &'a HeaderValue>,
    accepts: impl Fn(&str) -> bool,
) -> bool {
    match (values.next(), values.next()) {
        (None, _) => true,
        (Some(value), None) => value.to_str().is_ok_and(accepts),
        (Some(_), Some(_)) => false,
    }
}

/// **この要求を通すか。純粋関数。** 通すなら `Ok(())`、断るならその理由。
///
/// `authority` は要求の URI の authority（HTTP/2 の `:authority`・絶対形の要求行）。無ければ `None`。
pub(super) fn verdict(
    headers: &axum::http::HeaderMap,
    authority: Option<&str>,
    port: ListeningPort,
) -> Result<(), Refusal> {
    let ListeningPort(port) = port;
    if authority.is_some_and(|authority| !names_this_server(authority, port)) {
        return Err(Refusal::Authority);
    }
    if !absent_or_single(headers.get_all(HOST).iter(), |host| {
        names_this_server(host, port)
    }) {
        return Err(Refusal::Host);
    }
    if !absent_or_single(headers.get_all(ORIGIN).iter(), |origin| {
        is_own_origin(origin, port)
    }) {
        return Err(Refusal::Origin);
    }
    Ok(())
}

/// **断った応答を作り、届いた値をログに残す。**
///
/// **応答には届いた値を載せない**（[`ApiError`] は定数しか持てない）。ログには `Debug` の綴りで載せる。
fn refuse(refusal: Refusal, request: &Request) -> Response {
    let hosts: Vec<&HeaderValue> = request.headers().get_all(HOST).iter().collect();
    let origins: Vec<&HeaderValue> = request.headers().get_all(ORIGIN).iter().collect();
    let authority = request
        .uri()
        .authority()
        .map(axum::http::uri::Authority::as_str);
    tracing::warn!(
        reason = %refusal,
        host = ?hosts,
        origin = ?origins,
        authority = ?authority,
        method = %request.method(),
        path = %request.uri().path(),
        "Host / Origin がこのサーバのものではない要求を断りました"
    );
    ApiError::request_origin_not_allowed().into_response()
}

/// **ミドルウェアの本体**（`server::router` が `from_fn_with_state` で積む）。
pub(super) async fn guard(
    State(port): State<ListeningPort>,
    request: Request,
    next: Next,
) -> Response {
    let authority = request
        .uri()
        .authority()
        .map(axum::http::uri::Authority::as_str);
    match verdict(request.headers(), authority, port) {
        Ok(()) => next.run(request).await,
        Err(refusal) => refuse(refusal, &request),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, StatusCode};

    use super::*;
    use crate::testlog;

    /// **実際のポートの代わり**（既定の 8080 とも、80 とも、下の `OTHER` とも違う値）。
    const PORT: u16 = 47_123;
    const OTHER: u16 = 47_124;

    fn headers(pairs: &[(axum::http::HeaderName, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                name.clone(),
                HeaderValue::from_str(value).expect("見出しの値にできること"),
            );
        }
        map
    }

    fn judge(pairs: &[(axum::http::HeaderName, &str)]) -> Result<(), Refusal> {
        verdict(&headers(pairs), None, ListeningPort::new(PORT))
    }

    /// **フィクスチャのポートが全部違うこと**（同じだと取り違えが観測できない）。
    #[test]
    fn the_fixture_ports_all_differ() {
        let ports = [PORT, OTHER, crate::server::DEFAULT_PORT, HTTP_DEFAULT_PORT];
        for (i, a) in ports.iter().enumerate() {
            for b in &ports[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn the_host_table() {
        let own = format!("127.0.0.1:{PORT}");
        let passing = [
            own.clone(),
            format!("localhost:{PORT}"),
            format!("LOCALHOST:{PORT}"),
        ];
        for host in &passing {
            assert_eq!(judge(&[(HOST, host)]), Ok(()), "{host}");
        }
        assert_eq!(judge(&[]), Ok(()), "Host 無し");

        let refused = [
            format!("evil.example:{PORT}"),
            format!("127.0.0.1:{OTHER}"),
            format!("localhost:{OTHER}"),
            format!("127.0.0.1:{}", crate::server::DEFAULT_PORT),
            "127.0.0.1".to_owned(),
            "localhost".to_owned(),
            "127.0.0.1:80".to_owned(),
            format!("localhost.:{PORT}"),
            format!("127.0.0.2:{PORT}"),
            format!("[::1]:{PORT}"),
            format!("127.0.0.1:0{PORT}"),
            format!("127.0.0.1:+{PORT}"),
            format!("127.0.0.1:{PORT}:{PORT}"),
            format!("x.localhost:{PORT}"),
            "127.0.0.1:".to_owned(),
            String::new(),
        ];
        for host in &refused {
            assert_eq!(judge(&[(HOST, host)]), Err(Refusal::Host), "{host:?}");
        }
        assert_eq!(
            judge(&[(HOST, &own), (HOST, &own)]),
            Err(Refusal::Host),
            "Host が2つ（同じ値でも）"
        );
    }

    /// **ポートを省いた形は 80 と読む**（実際のポートが 80 のときだけ通る）。
    #[test]
    fn an_omitted_port_means_80() {
        let at_80 = ListeningPort::new(HTTP_DEFAULT_PORT);
        for host in ["127.0.0.1", "localhost", "127.0.0.1:80"] {
            assert_eq!(
                verdict(&headers(&[(HOST, host)]), None, at_80),
                Ok(()),
                "{host}"
            );
        }
        for origin in ["http://127.0.0.1", "http://localhost:80"] {
            assert_eq!(
                verdict(&headers(&[(ORIGIN, origin)]), None, at_80),
                Ok(()),
                "{origin}"
            );
        }
        assert_eq!(
            verdict(&headers(&[(HOST, "evil.example")]), None, at_80),
            Err(Refusal::Host)
        );
    }

    #[test]
    fn the_origin_table() {
        let own_host = format!("127.0.0.1:{PORT}");
        let passing = [
            format!("http://127.0.0.1:{PORT}"),
            format!("http://localhost:{PORT}"),
            format!("HTTP://LOCALHOST:{PORT}"),
        ];
        for origin in &passing {
            assert_eq!(
                judge(&[(HOST, &own_host), (ORIGIN, origin)]),
                Ok(()),
                "{origin}"
            );
        }

        let refused = [
            "null".to_owned(),
            "http://evil.example".to_owned(),
            format!("http://evil.example:{PORT}"),
            format!("http://127.0.0.1:{OTHER}"),
            format!("https://127.0.0.1:{PORT}"),
            format!("http://127.0.0.1:{PORT}/"),
            format!("http://127.0.0.1:{PORT}/x"),
            // **スキームを見ていること**（`http://` と同じ長さの別の綴り / B-58 のレビュー R2）。
            format!("xxxx://127.0.0.1:{PORT}"),
            format!("https:/127.0.0.1:{PORT}"),
            format!("127.0.0.1:{PORT}"),
            "http://".to_owned(),
            "http:/".to_owned(),
            String::new(),
        ];
        for origin in &refused {
            assert_eq!(
                judge(&[(HOST, &own_host), (ORIGIN, origin)]),
                Err(Refusal::Origin),
                "{origin:?}"
            );
            // **`Host` が無くても見る。**
            assert_eq!(
                judge(&[(ORIGIN, origin)]),
                Err(Refusal::Origin),
                "Host 無し / {origin:?}"
            );
        }
        let own = format!("http://127.0.0.1:{PORT}");
        assert_eq!(
            judge(&[(ORIGIN, &own), (ORIGIN, &own)]),
            Err(Refusal::Origin),
            "Origin が2つ"
        );
    }

    /// **要求の URI の authority も `Host` と同じ規則で見る**（HTTP/2 の `:authority`・絶対形の要求行）。
    ///
    /// **`Host` が正しくても、authority が外なら断る**（入口が2つある）。無ければ通す。
    #[test]
    fn the_authority_table() {
        let port = ListeningPort::new(PORT);
        let own_host = headers(&[(HOST, &format!("127.0.0.1:{PORT}"))]);
        for authority in [format!("127.0.0.1:{PORT}"), format!("localhost:{PORT}")] {
            assert_eq!(
                verdict(&own_host, Some(&authority), port),
                Ok(()),
                "{authority}"
            );
            assert_eq!(
                verdict(&HeaderMap::new(), Some(&authority), port),
                Ok(()),
                "Host 無し / {authority}"
            );
        }
        for authority in [
            format!("evil.example:{PORT}"),
            format!("127.0.0.1:{OTHER}"),
            "127.0.0.1".to_owned(),
            format!("user@127.0.0.1:{PORT}"),
        ] {
            assert_eq!(
                verdict(&own_host, Some(&authority), port),
                Err(Refusal::Authority),
                "{authority}"
            );
            assert_eq!(
                verdict(&HeaderMap::new(), Some(&authority), port),
                Err(Refusal::Authority),
                "Host 無し / {authority}"
            );
        }
        assert_eq!(
            verdict(&HeaderMap::new(), None, port),
            Ok(()),
            "どちらも無い"
        );
    }

    /// **ASCII として読めない値は断る**（`to_str` が失敗する形）。
    #[test]
    fn a_value_that_is_not_ascii_is_refused() {
        let mut map = HeaderMap::new();
        map.insert(
            HOST,
            HeaderValue::from_bytes(b"localhost\xff:47123").expect("不透明な値にできること"),
        );
        assert_eq!(
            verdict(&map, None, ListeningPort::new(PORT)),
            Err(Refusal::Host)
        );
    }

    /// **断った応答は 403 の定数の本文で、ログの同じ行に理由と届いた `Host` / `Origin` が載る。**
    ///
    /// **既定のフィルタで見る**（運用者の目に入ること）。**届いた値は応答に載らない。**
    #[test]
    fn a_refusal_is_logged_with_what_arrived_and_not_reflected() {
        let mut response = None;
        let output = testlog::captured_under_default_filter(|| {
            let request = Request::builder()
                .method("PUT")
                .uri("http://evil-authority.example:47123/admin/upstream/profiles/evil")
                .header(HOST, "evil-host.example:47123")
                .header(ORIGIN, "http://evil-origin.example")
                .body(axum::body::Body::empty())
                .expect("要求を組めること");
            response = Some(refuse(Refusal::Host, &request));
        });

        let line = testlog::line_containing(
            &output,
            "Host / Origin がこのサーバのものではない要求を断りました",
        );
        assert!(line.contains("WARN"), "{line}");
        assert!(line.contains("evil-host.example:47123"), "{line}");
        assert!(line.contains("http://evil-origin.example"), "{line}");
        assert!(line.contains("evil-authority.example:47123"), "{line}");
        assert!(
            line.contains(&Refusal::Host.to_string()),
            "理由が同じ行に無い: {line}"
        );
        assert!(line.contains("/admin/upstream/profiles/evil"), "{line}");

        let response = response.expect("応答があること");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime を作れること");
        let bytes = runtime
            .block_on(axum::body::to_bytes(response.into_body(), 64 * 1024))
            .expect("本文を読めること");
        let text = String::from_utf8(bytes.to_vec()).expect("UTF-8");
        assert!(!text.contains("evil"), "届いた値を反射している: {text}");
    }

    /// **理由の `Display` は定数**（届いた値を含めない）。
    #[test]
    fn the_refusal_display_is_constant() {
        assert_eq!(
            Refusal::Host.to_string(),
            "Host がこのサーバのものではありません"
        );
        assert_eq!(
            Refusal::Origin.to_string(),
            "Origin がこのサーバのものではありません"
        );
        assert_eq!(
            Refusal::Authority.to_string(),
            "要求の宛先（authority）がこのサーバのものではありません"
        );
    }
}
