//! 上流へ投げるリクエストの型と、OpenAI 互換 JSON への写し取り。
//!
//! **`ChatRequest` はモデル識別子を持たない。** 上流のモデルを決めるのは
//! `LmStudioConfig`（＝設定）だけで、リクエスト側から差し込めない。
//! これは意図的な制約で、FE から来る `model` フィールドは
//! **キャラクター／セッションの選択子であって推論モデルの指定ではない**（ADR-0004）。
//! `ChatRequest` に `model` を生やすと、信頼できない入力をそのまま上流へ中継する
//! 経路が最短距離で生えてしまう。**その経路を型として存在させない。**
//!
//! キャラクターごとにモデルを変えたい場合は `LmStudioClient::with_model` で
//! 差し替えたクライアントを作る（接続プールは共有される）。

use serde::Serialize;

use crate::domain::{ChatMessage, ChatRole};

/// 上流のモデル識別子（例: `ministral-3-14b-instruct-2512`）。
///
/// **`ModelId::parse` が唯一のコンストラクタ。** `From<String>` も `new` も生やさない
/// （`Handle` と同じ発想）。空文字や改行入りの識別子を設定に書いてしまうと、
/// 上流からは意味の分からない 400 が返るだけで原因に到達しづらい。
///
/// なお `/` を含む識別子（`mistralai/devstral-small-2-2512`）は正当なので拒否しない。
/// この値は**パスやシェルには渡らない**（JSON の値としてのみ使う）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModelId(String);

impl ModelId {
    /// 唯一のコンストラクタ。前後の空白を除去し、空と制御文字を拒否する。
    pub fn parse(raw: &str) -> Result<Self, ModelIdError> {
        let normalized = raw.trim();

        if normalized.is_empty() {
            return Err(ModelIdError::Empty);
        }
        if normalized.chars().any(char::is_control) {
            return Err(ModelIdError::Control);
        }

        Ok(Self(normalized.to_owned()))
    }

    /// JSON へ載せる文字列。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// モデル識別子が受け付けられない理由。`Display` は定数文字列（入力を反射しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ModelIdError {
    /// 正規化すると空になる。
    #[error("モデル識別子が空です")]
    Empty,

    /// 制御文字を含む。
    #[error("モデル識別子に制御文字は使えません")]
    Control,
}

/// [`ApiKey`] の長さの上限（バイト。ADR-0056 §決定1）。
pub const API_KEY_MAX_BYTES: usize = 4096;

/// **伏せた後の綴り**（[`redact_key`]）。
const REDACTED: &str = "***";

/// [`ApiKey::hint`] が見せる末尾の文字数。
const HINT_CHARS: usize = 4;

/// **見せてよい鍵の最短の長さ。** これより短い鍵は、末尾4文字でも鍵の半分を超えて出るので見せない。
const HINT_MIN_CHARS: usize = HINT_CHARS * 2;

/// **上流へ送る鍵**（ADR-0056 §決定1〜3 / 作業単位 A-13b-1）。
///
/// **[`ApiKey::parse`] が唯一のコンストラクタ**（`ModelId` と同じ発想）。
///
/// # 外へ出さない（ADR-0056 §決定3）
///
/// * **`Debug` は定数**（`ApiKey(***)`）。`LmStudioConfig` などは `Debug` を derive しているので、
///   包まないと `{:?}` から漏れる。
/// * **`Display` を実装しない**（`%key` でログへ載せる書き方を、型で書けなくする）。
/// * **`Serialize` を実装しない**（応答に載らない）。
/// * 値を取り出す口は [`Self::expose_secret`] だけ（ファイルへ書くとき・本文から伏せるとき）。
///
/// # 送る形は作っておく
///
/// **`Authorization: Bearer <鍵>` のヘッダ値を、検証と同じ場所で作る**（[`Self::bearer`]）。
/// ヘッダに載せられない値を、送る段になって初めて落とさない。**`sensitive` を立てる**
/// （`reqwest` / `hyper` が値をログ・`Debug` に出さない）。
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey {
    raw: String,
    bearer: reqwest::header::HeaderValue,
}

impl ApiKey {
    /// 唯一のコンストラクタ。**整えない**（前後の空白も鍵の一部として読み、止める）。
    ///
    /// | 入力 | 結果 |
    /// |---|---|
    /// | 空 | [`ApiKeyError::Empty`] |
    /// | [`API_KEY_MAX_BYTES`] を超える | [`ApiKeyError::TooLong`] |
    /// | `0x21`〜`0x7E` の外の字（空白・改行・制御文字・非 ASCII） | [`ApiKeyError::Character`] |
    pub fn parse(raw: &str) -> Result<Self, ApiKeyError> {
        if raw.is_empty() {
            return Err(ApiKeyError::Empty);
        }
        if raw.len() > API_KEY_MAX_BYTES {
            return Err(ApiKeyError::TooLong);
        }
        if !raw.bytes().all(|byte| (0x21..=0x7E).contains(&byte)) {
            return Err(ApiKeyError::Character);
        }
        // **上の検査を通った値は必ず載る**（可視の ASCII だけ）。それでも `unwrap` に寄りかからない。
        let mut bearer = reqwest::header::HeaderValue::from_str(&format!("Bearer {raw}"))
            .map_err(|_| ApiKeyError::Character)?;
        bearer.set_sensitive(true);
        Ok(Self {
            raw: raw.to_owned(),
            bearer,
        })
    }

    /// **鍵そのもの。** ファイルへ書くときと、上流の本文から伏せるときだけ使う。
    ///
    /// **ログ・応答・エラーの文面に渡さないこと**（ADR-0056 §決定3）。
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.raw
    }

    /// **画面に見せる末尾4文字**（鍵で守られた中継の先の推論サーバの画面と同じ見せ方 / ADR-0056 §決定3）。
    ///
    /// **8 文字より短い鍵は見せない**（`None`）—— 末尾4文字でも鍵の半分以上が出る。
    #[must_use]
    pub fn hint(&self) -> Option<String> {
        // **字は全部 ASCII**（[`Self::parse`] が見ている）ので、バイトの位置で切ってよい。
        (self.raw.len() >= HINT_MIN_CHARS)
            .then(|| self.raw[self.raw.len() - HINT_CHARS..].to_owned())
    }

    /// **送るヘッダの値**（`Bearer <鍵>`。`sensitive` が立っている）。
    pub(super) const fn bearer(&self) -> &reqwest::header::HeaderValue {
        &self.bearer
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

/// 鍵が受け付けられない理由。`Display` は定数文字列（**鍵を反射しない**）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ApiKeyError {
    /// 空。
    #[error("鍵が空です")]
    Empty,

    /// 長すぎる。
    #[error("鍵が長すぎます")]
    TooLong,

    /// `0x21`〜`0x7E` の外の字を含む。
    #[error("鍵に使えない字があります（空白・改行・制御文字・ASCII の外の字は使えません）")]
    Character,
}

// ----------------------------------------------------------------------
// 鍵を平文の `http://` で別の機へ送るとき（ADR-0056 §決定5 / 作業単位 A-13b-2）
// ----------------------------------------------------------------------

/// **宛先のアドレスの種類**（ADR-0056 §決定5 の表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AddressClass {
    /// ループバック（IPv4 `127.0.0.0/8`・IPv6 `::1`）。**そのまま送る。**
    Loopback,
    /// ローカルネットワーク（IPv4 `10.0.0.0/8`・`172.16.0.0/12`・`192.168.0.0/16`・`169.254.0.0/16`、
    /// IPv6 `fc00::/7`・`fe80::/10`）。**警告して送る。**
    LocalNetwork,
    /// それ以外（`100.64.0.0/10` を含む）。**承認があるときだけ、警告して送る。**
    Elsewhere,
}

/// **アドレスの種類を決める。純粋関数**（ADR-0056 §決定5）。
///
/// # `100.64.0.0/10`（CGNAT）は外
///
/// 通信事業者の網を通りうる（Tailscale などで使っていても、personacore からは見分けられない）。
///
/// # IPv4 を写した IPv6（`::ffff:a.b.c.d`）は外
///
/// **元の IPv4 の種類に読み替えない**（ADR は IPv6 の範囲を `fc00::/7`・`fe80::/10` と `::1` で書いている）。
/// 読み替えないほうは「承認が要る」側に倒れるので、鍵が承認なしで外へ出る向きには割れない。
pub(super) fn address_class(ip: std::net::IpAddr) -> AddressClass {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let [first, second, _, _] = v4.octets();
            if first == 127 {
                AddressClass::Loopback
            } else if first == 10
                || (first == 172 && (16..=31).contains(&second))
                || (first == 192 && second == 168)
                || (first == 169 && second == 254)
            {
                AddressClass::LocalNetwork
            } else {
                AddressClass::Elsewhere
            }
        }
        std::net::IpAddr::V6(v6) => {
            let head = v6.segments()[0];
            if v6.is_loopback() {
                AddressClass::Loopback
            } else if head & 0xfe00 == 0xfc00 || head & 0xffc0 == 0xfe80 {
                AddressClass::LocalNetwork
            } else {
                AddressClass::Elsewhere
            }
        }
    }
}

/// **鍵を平文の `http://` で別の機へ送るときの扱い**（ADR-0056 §決定5 / 作業単位 A-13b-2）。
///
/// **ループバック・`https://`・鍵の無い面にはこの値が無い**（`None` で表す。扱いは変えない）。
///
/// | 値 | 送るか | 綴り（[`Self::label`]） |
/// |---|---|---|
/// | [`Self::LocalNetwork`] | **警告して送る** | `local_network` |
/// | [`Self::Approved`] | **警告して送る**（面に承認がある） | `approved` |
/// | [`Self::NotApproved`] | **送らない**（接続する前に止める） | `not_approved` |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOverPlainHttp {
    /// 宛先が全部ループバックかローカルネットワークで、1つ以上ローカルネットワーク。
    LocalNetwork,
    /// ローカルネットワークの外が混ざるが、面に承認（`allow_key_over_plain_http: true`）がある。
    Approved,
    /// ローカルネットワークの外が混ざり、承認が無い。
    NotApproved,
}

impl KeyOverPlainHttp {
    /// **全部の値**（仕様書の閉じた綴りを作るため）。
    pub const ALL: [Self; 3] = [Self::LocalNetwork, Self::Approved, Self::NotApproved];

    /// **線に載せる綴り**（`GET /admin/upstream` の面の `key_over_plain_http`）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LocalNetwork => "local_network",
            Self::Approved => "approved",
            Self::NotApproved => "not_approved",
        }
    }
}

/// **解決した（または書かれた）アドレスの全部から、扱いを決める。純粋関数**（ADR-0056 §決定5）。
///
/// | アドレス | 返り値 |
/// |---|---|
/// | 全部ループバック（空を含む） | `None`（そのまま送る） |
/// | ループバックとローカルネットワークだけで、ローカルネットワークが1つ以上 | [`KeyOverPlainHttp::LocalNetwork`] |
/// | **1つでも**それ以外が混ざる | 承認があれば [`KeyOverPlainHttp::Approved`]、無ければ [`KeyOverPlainHttp::NotApproved`] |
///
/// **空は「送れる」側**（`None`）。どこへも繋がらないので、鍵は出ない（接続の失敗になる）。
pub(super) fn judge_addresses<I>(addresses: I, approved: bool) -> Option<KeyOverPlainHttp>
where
    I: IntoIterator<Item = std::net::IpAddr>,
{
    let mut local = false;
    for address in addresses {
        match address_class(address) {
            AddressClass::Loopback => {}
            AddressClass::LocalNetwork => local = true,
            // **1つでも外が混ざれば、そこで決まる**（残りを見ても変わらない）。
            AddressClass::Elsewhere => {
                return Some(if approved {
                    KeyOverPlainHttp::Approved
                } else {
                    KeyOverPlainHttp::NotApproved
                });
            }
        }
    }
    local.then_some(KeyOverPlainHttp::LocalNetwork)
}

/// **鍵を付けた面の宛先を、判定の仕方で分ける**（ADR-0056 §決定5 / 作業単位 A-13b-2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum KeyDestination {
    /// **判定しない**: `https://`・ループバック（`127.0.0.0/8`・`::1`・`localhost`）・
    /// `reqwest` が読めない綴り（送信そのものが起きない）。
    Unguarded,
    /// **IP アドレスの宛先。** `reqwest` の `dns_resolver` を通らないので、組むときに判定する。
    Address(std::net::IpAddr),
    /// **ホスト名の宛先**（綴りはログに出す用 —— 権限部の認証情報は含まない）。
    /// **接続のたびの名前解決**で判定する。
    Name(String),
}

/// **宛先の判定の仕方を決める。純粋関数**（ADR-0056 §決定5 / 作業単位 A-13b-2）。
///
/// # ホストは、送信が実際に繋ぐものを読む（ADR-0056 追記5）
///
/// **送信と同じ URL のパーサ（`reqwest::Url` / WHATWG）で読む。** 権限部を `@` で割るだけの読み方だと、
/// `http://203.0.113.5\@127.0.0.1:46001` はループバックに見えるが、WHATWG は `\` を `/` と読むので
/// **実際に繋ぐのは `203.0.113.5:80`** —— 判定と送信が割れて、鍵が平文で外へ出る。
/// **ここに置くのは、送信の道具（`reqwest`）を `llm` の外で綴らないため**（`upstream_leaks` の網3）。
pub(super) fn key_destination(endpoint: &str) -> KeyDestination {
    let Ok(url) = reqwest::Url::parse(endpoint.trim()) else {
        return KeyDestination::Unguarded;
    };
    if url.scheme() != "http" {
        return KeyDestination::Unguarded;
    }
    let Some(host) = url.host_str() else {
        return KeyDestination::Unguarded;
    };
    if host.eq_ignore_ascii_case("localhost") {
        return KeyDestination::Unguarded;
    }
    // **依存（`url` クレート）を直に足さない**ので、`Host` の型ではなく綴りを読む
    // （IPv6 は角括弧つきで来る / WHATWG は IPv4 を正規化した綴りで返す）。
    let address = host
        .parse::<std::net::Ipv4Addr>()
        .map(std::net::IpAddr::V4)
        .ok()
        .or_else(|| {
            host.strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .and_then(|inner| inner.parse::<std::net::Ipv6Addr>().ok())
                .map(std::net::IpAddr::V6)
        });
    match address {
        Some(address) if address_class(address) == AddressClass::Loopback => {
            KeyDestination::Unguarded
        }
        Some(address) => KeyDestination::Address(address),
        None => KeyDestination::Name(host.to_owned()),
    }
}

/// **宛先にユーザー情報（`http://user:pass@host`）があるか。純粋関数**（作業単位 A-13b-2）。
///
/// `reqwest` は宛先のユーザー情報を Basic 認証の `Authorization` にし、面の鍵の `Authorization` は
/// 足す（`append`）ので、**同じ面に両方を書くと `Authorization` が2本送られる。** 読み込みはこの形を止める。
///
/// **送信と同じパーサ（`reqwest::Url`）で読む**（[`key_destination`] と同じ理由）——
/// `http://203.0.113.5\@127.0.0.1:46001` は、WHATWG ではユーザー情報を持たない。
#[must_use]
pub fn endpoint_has_userinfo(endpoint: &str) -> bool {
    reqwest::Url::parse(endpoint.trim())
        .is_ok_and(|url| !url.username().is_empty() || url.password().is_some())
}

/// **ログ（と画面の観測）に出す宛先。ユーザー情報（`user:pass@`）・クエリ・フラグメントを落とす。純粋関数**
/// （作業単位 B-68 で `lmstudio` に置き、B-71 / Issue origin#100 でここへ移して公開した）。
///
/// クエリは `?api_key=…` のように秘密を運びうる。**ホスト・ポート・パスは残す**（どこへ繋ぎに行ったかが
/// 読めないと、別々に落ちる2つの上流を切り分けられない）。
/// 読めない綴りは宛先を出さない（`<読めない宛先>`）—— 伏せ損ねた綴りをそのまま出さない。
///
/// **宛先をログに出す箇所は、全部これを通す。2つ目を作らない。** 画面の観測（`server::upstream`）も
/// これを通してから、ユーザー情報があった印（`***@`）を足す。
///
/// **送信と同じパーサ（`reqwest::Url` / WHATWG）で読む**（[`endpoint_has_userinfo`] と同じ理由）。
/// **ここに置くのは、`llm` の外から呼ぶため**（`upstream_leaks` の網は継ぎ目のファイルの名前を数えない）。
///
/// # 落とすものが無ければ、書いたままの綴りを返す
///
/// WHATWG の綴りに直すと `http://127.0.0.1:9` が `http://127.0.0.1:9/` になる（既定のポートも消える）。
/// **秘密を持たない宛先の行は、着手前と1バイトも変えない**（ログの値で行を拾う見張り番がある ——
/// `tests/upstream_launch.rs` の「起こした上流を止めます」の `upstream_endpoint` 等）。
///
/// # ホストを持たない URL は出さない（B-71 のレビュー是正）
///
/// `user:pw-secret@127.0.0.1:8080`（`http://` の書き忘れ）は、WHATWG ではスキーム `user`・
/// パス `pw-secret@127.0.0.1:8080` の URL として**読めてしまう**。ユーザー情報もクエリも空なので、
/// 下の「書いたまま」の枝に入ってパスワードが出ていた。**判定はスキームの一覧ではなく `has_host`** ——
/// 宛先の同定はホストで、ホストの無い URL には出す値打ちのある部分が無い（`file:`・`mailto:` 等も同じ扱い）。
/// スキームを `http` / `https` に絞る形も考えたが、`ftp://h` のようなホストを持つ誤記まで隠すと、
/// 「宛先のスキームが不正」という直す手掛かりが行から消える。
#[must_use]
pub fn endpoint_for_log(endpoint: &str) -> String {
    match reqwest::Url::parse(endpoint) {
        Ok(url) if !url.has_host() => "<読めない宛先>".to_owned(),
        Ok(url)
            if url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none() =>
        {
            endpoint.to_owned()
        }
        Ok(mut url) => {
            strip_for_log(&mut url);
            url.to_string()
        }
        Err(_) => "<読めない宛先>".to_owned(),
    }
}

/// **`reqwest::Error` が持つ URL から、[`endpoint_for_log`] と同じものを落とす**（作業単位 B-71）。
///
/// `reqwest::Error` の `Display` は `… for url (<叩いた URL>)`、`Debug` は `url` の欄を持つ。
/// **ユーザー情報は `reqwest` が組むときに Basic 認証へ移す**ので残らないが、**クエリとフラグメントは残る。**
/// **`LlmError` は `#[source]` にこのエラーを持つ**ので、ログに出す前ではなく**受け取ったその場で**落とす
/// （呼び出し側が `?err` やエラーの連鎖で書いても漏れないように）。
///
/// **URL ごと消す（`without_url`）のではなく、落とすものだけ落とす** —— 叩いた経路
/// （`/api/v0/models` 等）は、どの読み手が失敗したかの手掛かりで、見張り番もそれを読んでいる。
#[must_use]
pub(super) fn error_for_log(mut err: reqwest::Error) -> reqwest::Error {
    if let Some(url) = err.url_mut() {
        strip_for_log(url);
    }
    err
}

/// [`endpoint_for_log`] と [`error_for_log`] の中身（**落とすものの一覧はここ1箇所**）。
fn strip_for_log(url: &mut reqwest::Url) {
    // **どちらも失敗するのは、ユーザー情報を持てない URL（ホストの無い等）だけ** —— そのときは
    // 落とすものが無い。
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
}

/// **名前解決の差し替え口**（作業単位 A-13b-2）。
///
/// **本番は OS の名前解決**（`lmstudio` の `SystemResolver`）。テストは
/// `LmStudioConfig::with_host_resolver` で差し替え、**実際の DNS に依らずに**
/// 「解決先が後から外へ変わる」を作る。
///
/// **呼ばれるのは、鍵を付けた `http://` のホスト名の宛先だけ**（[`KeyDestination::Name`]）。
/// 鍵の無い面・`https://`・ループバック・IP アドレスの宛先は、今までどおり `reqwest` の既定で繋ぐ。
pub trait HostResolver: Send + Sync + std::fmt::Debug {
    /// `host` を解決する。**ポートは宛先の URL が決める**（返すのはアドレスだけ）。
    fn lookup(&self, host: &str) -> HostLookup;
}

/// [`HostResolver::lookup`] が返す future。
pub type HostLookup = std::pin::Pin<
    Box<dyn std::future::Future<Output = std::io::Result<Vec<std::net::IpAddr>>> + Send>,
>;

/// **`reqwest` が実際に繋ぐホストの綴り**（[`key_destination`] の前提を、見張り番が確かめるため）。
#[cfg(test)]
fn dialed_host(endpoint: &str) -> Option<String> {
    reqwest::Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
}

/// **本文から鍵を伏せる。純粋関数**（ADR-0056 §決定3）。
///
/// **上流のエラー本文をログに出す前に通す**（`lmstudio::error_snippet`）。上流が鍵を写し返すかは
/// 上流次第で、鍵で守られた中継は写し返さないことを確かめたが、ほかは確かめていない。
/// **切り詰める前に伏せること**（後で切ると、鍵の途中で切れた断片が伏せられずに残る）。
///
/// # 伏せる綴りは3つ（作業単位 A-13b-1 のレビュー是正）
///
/// 鍵は `"` と `\` を含みうる（`0x21`〜`0x7E`）。上流が JSON の本文に写し返すと、生の綴りでは現れない。
///
/// | 形 | 例（鍵 `a"b\c/d`） |
/// |---|---|
/// | JSON の文字列としてエスケープし、`/` も `\/` にした形 | `a\"b\\c\/d` |
/// | JSON の文字列としてエスケープした形 | `a\"b\\c/d` |
/// | 生の綴り | `a"b\c/d` |
///
/// **長い形から先に置き換える。** 短い形を先にしても鍵の字は残らない（足されたエスケープの `\` が `***` の隣に残るだけ）が、
/// 1つの写し返しが1つの `***` になるほうがログを読みやすい（レビューの指摘で、残りが出うるという前の主張を外した）。
/// **`\u` に4桁の16進を続けた綴り（引用符を `\u` と `0022` で書く形など）での写し返しは伏せない**（守れないもの）。
#[must_use]
pub(super) fn redact_key(body: &str, key: Option<&ApiKey>) -> String {
    let Some(key) = key else {
        return body.to_owned();
    };
    let raw = key.expose_secret();
    // **鍵は可視の ASCII だけ**なので、JSON のエスケープが要るのは `"` と `\` だけ。
    let escaped = raw.replace('\\', "\\\\").replace('"', "\\\"");
    let slashed = escaped.replace('/', "\\/");
    let mut forms = vec![slashed, escaped, raw.to_owned()];
    forms.sort_by_key(|form| std::cmp::Reverse(form.len()));
    forms.dedup();
    forms.iter().fold(body.to_owned(), |text, form| {
        text.replace(form.as_str(), REDACTED)
    })
}

/// 1回の補完要求。
///
/// ここのサンプラーは `None` なら**送らない**（上流の既定に任せる）。
/// 0 と「未指定」を取り違えないよう `Option` で表す。
///
/// **⚠ personacore が必ず載せるサンプラーは、この型ではなく [`RequestPolicy`] にある**
/// （`top_k` / `top_p` / `min_p` / `repeat_penalty` / `presence_penalty` / ADR-0038 §決定2）。
/// **ここに在るのは「依頼ごとに変わる値」だけ。**
/// **⚠ `temperature` がここに在るのは、経路ごとに値が違うから**である。
/// **本文の経路はクライアントの依頼の値を通し、裏方（抽出・変容）の経路は
/// [`SUB_LLM_TEMPERATURE`] を明示する**（作業単位 A-7d で、
/// **最後に残っていた「観測できない上流の既定」が消えた**）。
/// **⚠ `None` が残るのは「クライアントが `temperature` を送ってこなかった本文の回」だけ**
/// （未決 / [`SUB_LLM_TEMPERATURE`] の doc）。
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    /// 送るメッセージ列。並び順がそのまま文脈になる。
    pub messages: Vec<ChatMessage>,
    /// 温度。未指定なら上流の既定。
    pub temperature: Option<f32>,
    /// 生成トークンの上限。未指定なら上流の既定。
    pub max_tokens: Option<u32>,
}

impl ChatRequest {
    /// サンプラー未指定の要求を作る。
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            temperature: None,
            max_tokens: None,
        }
    }
}

/// **裏方（抽出・変容）が上流へ明示する温度**
/// （[ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定2 /
/// 作業単位 A-7d）。
///
/// # なぜ [`RequestPolicy`] に載せないのか
///
/// **あれは全経路で同じ1つ**で、**本文の経路はクライアントの依頼の値をそのまま通す**
/// （[`ChatRequest::temperature`]）。**あそこへ載せると経路差になる**うえ、
/// **[`WireRequest`] と同じ名前なので重複キーがそのまま線に乗る**
/// （`#[serde(flatten)]` はマップ直列化 / 見張り番は
/// `the_wire_bytes_carry_each_top_level_key_exactly_once`）。
/// **だから欄は1つのままにして、値を入れる側**（`server::extract` / `server::drift`）**が持つ。**
///
/// # 値の来歴 —— **今日の実効の既定そのもの**（C-13 の実測 / 2026-08-29）
///
/// **測ったこと**: **記録プロキシと `/slots` の突き合わせで、20 サンプルとも
/// `"temperature":0.800000011920929`** ——これは **`0.8_f32` を `f64` へ広げた綴り**である。
/// **同じ回の wire ログ 6 本すべてで、`temperature` の欄は1件も出ていない**
/// （腕あたり 108 POST × 6）ので、**その 0.8 は上流が当てていた既定**にほかならない。
///
/// # ⚠ ここから先は推論である（**実測ではない**）
///
/// **温度を明示で送った腕は1本も無い。** **`0.8_f32` は JSON へ `0.8` と綴られ、
/// 上流が float 化すれば同じビットに戻る**ので、
/// **上流が受け取る値は今日と同じはず**であり、**だから抽出と変容の質も変わらないはず**である。
/// **この「はず」を落とさないこと**——**利用者が採否の根拠にしたのは、この一文**である。
///
/// **⚠ サンプラー4欄は、ここが一段強い。**
/// **あちらは明示で送った値が `/slots` に入ることを実際に確かめてある**
/// （腕 C の `repeat_penalty: 1.0` がスロットに `1.0` として現れ、
/// 何も送らない腕 A は `1.100000023841858` だった / [`RequestPolicy::top_k`] の doc）。
/// **温度についてだけ、その確認が抜けている。**
/// **確かめるなら「明示で送った回の `/slots`」を読むこと**（`/props` ではない）。
///
/// **変わるのが「観測できるようになること」だけ、という狙いはどちらも同じ**
/// （[`RequestPolicy`] の doc）。
///
/// **⚠ この `0.8` の出どころは原理的に区別できない**（C-13）——
/// **LM Studio のプロキシが入れているのか engine の既定なのか、どちらも 0.8 なので帰属できない。**
/// **明示にすると、その区別が要らなくなる**（personacore が持つ値になる）。
///
/// **⚠ 測り直すときに `/props` を読まないこと。** 権威は `/slots` だけ
/// （理由は [`RequestPolicy::top_k`] の doc）。
///
/// **⚠ この定数は [`crate::measured::MEASURED_CONSTANTS`] には登録しない**
/// （サンプラー4欄と同じ）。**理由はあちらの表が持つ**（「入れなかったもの」の行 /
/// **ここに写さない**）。
///
/// # ⚠ 抽出と変容の取り違えは、この形では観測できない
///
/// **2つの経路が同じ1つの値を使う**ので、**両者を入れ替える変異は出力が1バイトも変わらない。**
/// CLAUDE.md の最頻の穴——「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」——が
/// **実装の側で同値になっている形**なので、**`assert_ne!` をどこに置いても防げない。**
/// **経路ごとに違う温度が要ると判断した日には、先に
/// 「経路と値の取り違えを見る見張り番」を置くこと**（[`RequestPolicy`] の doc と同じ申し送り）。
///
/// **⚠ 「本文の経路との取り違え」のほうは観測できる。**
/// クライアントの値と 0.8 は別の値なので、見張り番は
/// `tests/server_stat_drift.rs` の `each_path_sends_its_own_temperature`
/// （**3経路の値を、経路の名前と対にして並べて見る**）。
///
/// # ⚠ `0.0` にしないこと
///
/// **温度を 0 に固定すると、同じ入力の繰り返しが同じ出力になる。**
/// 実機の器具（`tests/extraction_quality_harness.rs`）は
/// **繰り返しが検出力の源**（`PERSONACORE_HARNESS_REPS`）なので、
/// **0 にした日に、その器具は何も測れなくなる。**
///
/// # ⚠ 未決 —— **クライアントが `temperature` を送ってこない回**
///
/// **本文の経路はいまも `None` のまま上流の既定で走る**（A-7d の範囲外）。
/// **そこに既定を置くかどうかは別の判断が要る** ——
/// 置けば「クライアントが省略した」と「personacore が決めた」が区別できなくなり、
/// 置かなければ本文だけが観測できない既定の上に残る。
pub const SUB_LLM_TEMPERATURE: f32 = 0.8;

/// `ChatRole` の JSON 表現。**OpenAI 互換の綴りと一致させること。**
///
/// 変換をここに置くのは、綴りの決定権を「上流と話す層」に集めるため
/// （`store` が DB の綴りを持つのと同じ配置）。
/// **`ChatRole` 自体が `domain` に移っても、この関数は一緒に引っ越さない。**
const fn role_to_wire(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    }
}

/// **personacore が、上流へ出るすべての要求に必ず載せる欄**
/// （[ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定3）。
///
/// # モデルで分岐しない。設定でも分岐しない
///
/// **これは「このモデルにはこの欄が要る」という一覧ではなく、personacore の立場である。**
/// personacore は `content` を読み、裏方の予算はトークン単位で決めている
/// （抽出の応答上限は `server::extract::EXTRACTION_RESPONSE_TOKENS`）ので、
/// **上流に思考させたくない**——それはどのモデルを載せていても変わらない。
///
/// **⚠ 値を書かないのは意図的。** `server::extract` には**似た綴りの定数が3つ**あり
/// （`MEASURED_MAX_EXTRACTION_RESPONSE_TOKENS` /
/// `QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS` / `EXTRACTION_RESPONSE_TOKENS`）、
/// **値を写すとどれの値か分からなくなるうえ、動いたときに黙って古びる。**
/// **名前だけなら追随する。**
///
/// **⚠ この doc は A-7b で2回続けて踏んだ。** まず「368」と書き
/// （それは `QUEUE_SAFE_MAX_EXTRACTION_RESPONSE_TOKENS` の値で、ここの話ではなかった）、
/// **それを直すときに今度は「320」を転記した。**
/// **間違った値を、別の値で直しても同じ形である。**
/// **他モジュールの定数は、名前で指すこと。**
///
/// **思考しないモデルに送っても何も変わらない**（`ministral-3-14b-instruct-2512` で実測。
/// status・`completion_tokens`・本文がバイト単位で一致した / ADR-0038 §決定3）。
/// **だからモデルごとの束を持たない。**
/// 一覧を持てば黙って古びる（[ADR-0030](../../docs/adr/0030-choosing-the-axes.md)）し、
/// 上流固有の型が3つ目になる（[ADR-0036](../../docs/adr/0036-the-range-of-machines-is-a-constraint.md) 決定4）。
///
/// **⚠ [`StreamOptions`] の doc が「載せると拒否する上流がありうる」と書いているのに、
/// こちらは条件を付けずに載せている。** **その食い違いは意図的で、まだ測っていない** ——
/// **知らない欄で 400 を返す上流を一度も踏んでいない**（LM Studio は無視した）。
/// **何を測っていないかは
/// [ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定3 の
/// 「測っていないこと」に書いてある。ここに写さない。**
///
/// **⚠ A-7c で、この食い違いの重さが上がった。**
/// 標準でない欄が1つ（`reasoning_effort`）から**5つ**（＋サンプラー4欄）になったので、
/// **未知の欄を 400 で拒否する上流では「測定が再現しない」ではなく
/// 「その上流ではすべての要求が失敗する」になる。**
/// （A-14 で足した `presence_penalty` は OpenAI 標準の欄なので、標準でない欄の数は増えていない。）
/// **どの層の話になるかは
/// [ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定1 の表が持つ**
/// （**ここに写さない**）。
///
/// # サンプラーもここに載る（**ADR-0038 §決定2 / 値は C-13 が測って決まった**）
///
/// **目的は「測定が再現すること」であって、出力を良くすることではない。**
/// **指定しない欄は上流の既定が当たり、その既定は観測できず、上流ごとに違う**
/// （[ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §背景 ——
/// **サンプラーを揃えるまで `temperature 0` でも出力が 6 文字目で分岐した**）。
///
/// **⚠ 明示にしても振る舞いは変わらない**（C-13 の実測 / 2026-08-29）——
/// **何も送らない腕と、いまの既定を明示した腕は、3種類の固定依頼・全部の長さの帯で
/// 見分けられなかった**（長さ・トークン数・繰り返し・失敗率の符号検定 9 通りとも有意でない）。
/// **変わるのは「観測できるようになる」ことだけで、それがこの決定そのものである。**
///
/// **⚠ `temperature` はここに無い。経路ごとに値が違うから**である
/// （[`ChatRequest`] の欄 —— 本文はクライアントの依頼の値、裏方は [`SUB_LLM_TEMPERATURE`]）。
/// **⚠ ここに載せると、経路差になるうえ [`WireRequest`] と重複キーになる**
/// （申し送り2 / `the_wire_bytes_carry_each_top_level_key_exactly_once`）。
/// **⚠ A-7c の時点で残っていた未決は、A-7d で閉じた** ——
/// **抽出と変容も明示で送る**（値と来歴は [`SUB_LLM_TEMPERATURE`] の doc）。
///
/// **⚠ `top_k` / `min_p` / `repeat_penalty` は OpenAI 標準の欄ではない。**
/// **それを理由に「上流固有の欄」へ格下げしないこと**——`reasoning_effort` と同じ理由で、
/// **送る理由は上流の都合ではなく personacore の立場**（測定が再現すること）だから。
/// **ADR-0038 §決定1 は「要求ごとのサンプラの指定を尊重すること」を必須の層に置いている。**
///
/// **⚠ C-13 が測った4つ（`top_k` / `top_p` / `min_p` / `repeat_penalty`）は
/// [`crate::measured::MEASURED_CONSTANTS`] には登録しない。**
/// **理由はあちらの表が持つ**（「入れなかったもの」の行 / **ここに写さない**）。
/// **`presence_penalty` は実測由来ではない**（利用者の決定 / A-14）**ので、そもそも候補ではない。**
///
/// **⚠ `Eq` を derive しない。** `top_p` / `min_p` / `repeat_penalty` は `f32` なので、
/// `Eq` があるとそもそも欄を足せない（A-7b で実測）。比較する相手も居ない。
///
/// **⚠ 「経路ごとに違う値」は、この形のままでは持てない。**
/// ここは全経路で同じ1つの定数で、経路差が要るなら
/// `wire_request` の引数として受け取る形へ変えることになる。
/// **その前に「経路と値の取り違えを見る見張り番」が要る** ——
/// いまの `tests/server_stat_drift.rs` の `all_three_paths_ask_the_upstream_not_to_think` は
/// **3経路に同じ値を期待している**ので、取り違えを見ていない
/// （[ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定3 の表）。
///
/// **⚠ 作法だけは A-7d が先に作った。** `temperature` は**経路ごとに違う値**になったので、
/// **同じファイルの `each_path_sends_its_own_temperature` が
/// 「経路の名前と値を対にして並べる」形で帰属を見ている。**
/// **ここの欄を経路ごとに変える日は、あの形を、経路ごとに変える欄の1つずつに写すこと** ——
/// **ただし写すのは形であって、値ではない**（**3経路が同じ値を持つ限り、取り違えは観測できない**）。
///
/// # ⚠ A-7b の申し送り3つは、C-13 と A-7c で全部片が付いた
///
/// 1. **「意図しない欄が混ざっていないこと」の権威は、1本では足りなかった。**
///    サンプラーを足すと `wire_request_carries_model_and_messages_in_order` の
///    期待値も一緒に書き換わるので、**書き換えた日には、その1本は自分の主張を守れない。**
///    **いまは3本で見ている** ——
///    その1本（本文全体との等値）と、
///    `the_wire_bytes_carry_each_top_level_key_exactly_once`（**線に乗ったキーの多重集合**）と、
///    `the_wire_bytes_spell_the_request_exactly`（**バイト列そのものとの等値**）。
///    **どれも期待値をこの構造体から作っていない**（手で書いた文字列リテラル）。
/// 2. **名前の衝突は実在した**（C-13 が実測）。`#[serde(flatten)]` はマップ直列化なので、
///    **ここに [`WireRequest`] と同じ名前の欄を足すと、`serde_json::to_string` は
///    重複キーをそのまま線に載せる**（`…,"temperature":9.5,…,"temperature":0.5`）。
///    **`serde_json::to_value` は畳む**ので、**`to_value` を通す検査からは原理的に見えない。**
///    **上流（LM Studio）は 200 を返し、後ろ勝ちだった。**
///    **JSON の仕様上は未定義なので、どちらが勝つかは上流ごとに違いうる。**
///    **見張り番は `the_wire_bytes_carry_each_top_level_key_exactly_once`**
///    （**同じ値の重複でも赤になる**。A-7c で実測 ——
///    **是正前はその変異が 1731 全緑を通った**）。
/// 3. **⚠ 「`f32` は綴りどおりに出ない」は誤りだった**
///    （C-13 が記録プロキシの生バイトで実測 / 2026-08-29）。
///    **serde は最短表記を書く** —— `"top_p":0.95` / `"min_p":0.05` / `"repeat_penalty":1.1`。
///    **`0.949999988079071` が出るのは `serde_json::Value` を経由したときだけ**で、
///    **それは線に乗るバイト列ではない。**
///    **そして上流が受け取った値は、上流自身の既定とビット単位で同一だった。**
///    **綴りと受け取る値は一致している。**
#[derive(Debug, Clone, Copy, Serialize)]
pub(super) struct RequestPolicy {
    /// **思考モデルに思考をさせない。**
    ///
    /// 思考モデル（`Qwen3.5-9B` / `ministral-3-14b-reasoning-2512`）は既定で
    /// 予算を全部思考に使い、**`content` が 0 文字**になる
    /// （personacore からは「応答が空でした」/ docs/archive/04-environment-lm-studio.md の実測）。
    /// `/no_think` も `chat_template_kwargs` も効かず、**この欄だけが効いた。**
    ///
    /// # ⚠ 欄の名前は標準、値は拡張
    ///
    /// **`"none"` は OpenAI 標準の値ではない。**
    /// **それを理由に「上流固有の欄」へ格下げしないこと** ——
    /// 送る理由は上流の都合ではなく personacore の立場（この型の doc）だから。
    ///
    /// **標準が何を定めているかと、なぜ格下げしないかは
    /// [ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定3 の
    /// 「測っていないこと」に書いてある。ここに写さない**（上の ⚠ と同じ理由）。
    reasoning_effort: &'static str,

    /// **上位 K 個からしか選ばない。**
    ///
    /// # ⚠ 4つのサンプラーは「今日の実効の既定」そのもの
    ///
    /// **`top_k 40` / `top_p 0.95` / `min_p 0.05` / `repeat_penalty 1.1` は、
    /// この機の上流が現に使っていた値**（C-13 の実測 / 2026-08-29）。
    /// **だから明示にしても振る舞いは変わらない。変わるのは観測できることだけ**
    /// （理由はこの型の doc）。
    ///
    /// # ⚠ 測り直すときに `/props` を読まないこと
    ///
    /// **権威は `/slots`（その要求が実際に何を使ったか）であって、
    /// `/props`（起動時の既定）ではない。** **2つは食い違う** ——
    /// **`/props` は `repeat_penalty 1.0` と言い、`/slots` は `1.1` だった。**
    /// **`llama-server` の起動引数にサンプラーのフラグは1つも無い**ので、
    /// **その 1.1 は LM Studio のプロキシが本文に入れて送っていたことになる。**
    /// **`/props` を読むと間違った答えが返る。**
    top_k: u32,

    /// **確率の上位からどこまでを候補にするか**（累積確率）。値の来歴は [`Self::top_k`]。
    top_p: f32,

    /// **最有力の候補に対する相対的な足切り。** 値の来歴は [`Self::top_k`]。
    min_p: f32,

    /// **繰り返しへの罰。** 値の来歴は [`Self::top_k`]。
    ///
    /// **⚠ ここだけは「守りが薄くなる」向きの疑いがあった**
    /// （[ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定2 ——
    /// 司令塔が比較で使った `1.0` は「抑制なし」で、
    /// **小型モデルの繰り返しを既定が抑えていたかもしれない**）。
    /// **C-13 が測って、その疑いは要らなくなった** ——
    /// **明示する値が既定と同じ `1.1` なので、抑制は一切外れていない。**
    /// **`1.0` を送る腕（中立）は別に測ってあり、長い依頼でわずかに繰り返しが増えた**が、
    /// **多重比較を補正すると1本も残らず、退化は 576 応答で1件も起きていない。**
    /// **つまり「1.0 が危ない」とも言えていない。**採らない理由は
    /// **既定と揃えるほうが線が1本少ないから**である。
    ///
    /// # ⚠ この欄だけ、綴りが上流系統に依存する（**未確認**）
    ///
    /// **`repeat_penalty` は llama.cpp 系の綴りで、vLLM / TGI は
    /// `repetition_penalty` と綴る**（レビュアーの指摘。**確信度は高いが、
    /// この機では実機で確かめていない**）。
    ///
    /// **⚠ 当たっているなら、上流を llama.cpp 系の外へ替えた瞬間に
    /// この欄だけが静かに無視され、その上流の既定が当たる** ——
    /// **[ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定2 が
    /// 消そうとした「観測できない既定」が、ここだけ戻る。**
    /// **他の3欄と同格に扱わないこと。**
    /// **綴りを増やす（両方送る・上流ごとに選ぶ）判断は、まだしていない。**
    repeat_penalty: f32,

    /// **既に出たトークンへの一律の罰。`0`（罰なし）を明示する**
    /// （[ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定2 の
    /// 2026-11-01 追記 / 作業単位 A-14 / origin#78）。
    ///
    /// # なぜ送るのか —— **上流がモデルカードの既定を黙って重ねうる**
    ///
    /// **鍵で守られた中継の先の推論サーバ（C-21 で遊ぶ用にした第三者の推論サーバ）は、要求に書いていない欄に
    /// モデルカードの既定を合成し、要求に書いた欄だけを優先する。**
    /// **既定には「思考あり」と「思考オフ」の2組があり、`presence_penalty 1.5` は思考オフの組にだけある。**
    ///
    /// **⚠ 訂正（2026-11-01、A-14 を取り込んだ後の実機確認）**: C-21 と A-14 の着手時には
    /// 「毎回 1.5 が掛かっていた」と書いたが、**誤りだった。** その推論サーバは `reasoning_effort: "none"` を
    /// 思考オフとして扱わない（思考の有無は `chat_template_kwargs.enable_thinking` が `false` かだけで決まり、
    /// `reasoning_effort` の値はこの判定に使われない。その製品のソースで確かめた）ので、personacore の要求には**思考ありの組**
    /// （`temperature 1.0` / `top_p 0.95` / `top_k 20`。`presence_penalty` は無い）が重なっていた。
    /// 実機でも、欄なしと 0 の明示が同じ出力になり、1.5 の明示だけが違った
    /// （`<measurements>` の 2026-11-01 の、中継越しの測定の `presence_probe2.txt`）。
    /// **それでも 0 を明示するのは、上流の設定や判定が変わっても値を personacore が持つため**
    /// （ADR-0038 §決定2）。
    ///
    /// # 値の来歴 —— **llama-server の既定**（**LM Studio は未確認**）
    ///
    /// **llama-server は、この欄を送っていなかった時期の `/slots` に
    /// `"presence_penalty":0.0` が出ている**（B-59 の記録）。**だから 0 を明示しても
    /// そちらの振る舞いは変わらないはず**（過去の測定と線がつながる）。
    /// **その推論サーバでも、いまの要求の形（`reasoning_effort: "none"`）では振る舞いは変わらない**（上の訂正）。
    ///
    /// **⚠ LM Studio は確かめていない**（アンインストール済みで、もう測れない）。
    /// **「プロキシは penalty を足さない」とも言えない** —— LM Studio のプロキシは
    /// `repeat_penalty 1.1` を本文に入れていた（[`Self::top_k`] の doc）。
    ///
    /// **⚠ 上の4欄と違い、この機で「明示で送った値が `/slots` に入る」ことはまだ確かめていない。**
    ///
    /// **⚠ OpenAI 標準の欄**なので、未知の欄を拒む上流の問題（この型の doc）は増やさない。
    presence_penalty: f32,
}

impl RequestPolicy {
    /// personacore の立場。**全経路で同じ1つ。**
    ///
    /// **⚠ 経路ごとに変えないこと。** 変えるなら、先に
    /// 「経路と値の取り違えを見る見張り番」が要る（この型の doc）。
    ///
    /// # 面に書いた値は、この1組の上に重なる（ADR-0058 / 作業単位 A-16）
    ///
    /// **上流プロファイルの面に書いた欄（いまは `repeat_penalty` だけ）は、その面のクライアントが
    /// [`Self::overridden_by`] で置き換えて送る。** 値が付くのは経路ではなく面（上流）で、`sub` 面を書いた
    /// プロファイルでは本文と裏方が違う値を送りうる。**その帰属の見張り番は `tests/server_profile_sampling.rs`**
    /// （本文と裏方に違う値を書き、どちらの要求にどちらが載るかを見る）。**何も書いていない面は、この1組のまま。**
    pub(super) const PERSONACORE: Self = Self {
        reasoning_effort: "none",
        top_k: 40,
        top_p: 0.95,
        min_p: 0.05,
        repeat_penalty: 1.1,
        presence_penalty: 0.0,
    };

    /// **面に書いた値で置き換えた1組**（ADR-0058 §決定1 / 作業単位 A-16）。
    ///
    /// **書いた欄だけを置き換える。** 何も書いていなければ（[`Sampling::default`]）、
    /// [`Self::PERSONACORE`] と1ビットも違わない —— 書かない面の要求のバイト列は変わらない
    /// （見張り番は `tests::the_wire_bytes_spell_the_request_exactly` が既定のまま通ること）。
    #[must_use]
    const fn overridden_by(self, sampling: Sampling) -> Self {
        let repeat_penalty = match sampling.repeat_penalty {
            Some(written) => written.0,
            None => self.repeat_penalty,
        };
        Self {
            repeat_penalty,
            ..self
        }
    }
}

/// **[`RepeatPenalty`] の下限**（含む / ADR-0058 §決定1 の「範囲は実装で決める」）。
///
/// # 範囲を「中立の 1.0 から2倍以内」にした理由
///
/// llama.cpp 系の繰り返しの罰は、既に出たトークンの logit を**この値で割る（正）・掛ける（負）**。
/// **1.0 が中立**で、1.0 より大きいと繰り返しを抑え、小さいと繰り返しを促す。
///
/// | 値 | 何が起きるか | 扱い |
/// |---|---|---|
/// | NaN・無限 | 送る本文に書けない（JSON は数として表せない） | **止める** |
/// | 0 | logit を 0 で割る | **止める** |
/// | 負 | 罰の向きが裏返る（正の logit を負にする） | **止める** |
/// | 0.5 未満 | 繰り返しを2倍より強く促す。会話がループに落ちる側の値で、測る理由が見当たらない | **止める** |
/// | 0.5〜2.0 | 測る意味のある範囲（モデルカードや上流の既定は 1.0〜1.3 程度。ADR-0058 の実測は 1.0 と 1.1） | 通す |
/// | 2.0 超 | 既に出た語を全部2倍より強く罰する。`1.1` を `11` と打ち損じた値もここに落ちる | **止める** |
///
/// **⚠ この範囲は測って決めたものではない**（ADR-0036 §決定3 の「根拠が無い」値の、壊れた値を作らせないための柵）。
/// 範囲の外に正当な値があると分かったら、ここを広げる。
pub const REPEAT_PENALTY_MIN: f64 = 0.5;

/// **[`RepeatPenalty`] の上限**（含む）。理由は [`REPEAT_PENALTY_MIN`] の doc。
pub const REPEAT_PENALTY_MAX: f64 = 2.0;

/// **面に書いた繰り返しの罰**（ADR-0058 §決定1 / 作業単位 A-16）。
///
/// **[`Self::parse`] が唯一のコンストラクタ**（`ModelId` と同じ発想）。範囲は [`REPEAT_PENALTY_MIN`] の表。
///
/// # `Eq` を手で書く
///
/// 中身は `f32` だが、**NaN を作れない**（[`Self::parse`] が止める）ので、ビット列の比較が値の比較になる
/// （`+0` と `-0` も作れない —— 下限が正）。**`Eq` が要るのは、面の比較（走ったままの切り替えの
/// 「変わらない面は触らない」）と、プロファイルの型がそれを derive しているから**である。
#[derive(Debug, Clone, Copy)]
pub struct RepeatPenalty(f32);

impl PartialEq for RepeatPenalty {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for RepeatPenalty {}

impl RepeatPenalty {
    /// 唯一のコンストラクタ。**範囲の外は `None`**（[`REPEAT_PENALTY_MIN`] の表）。
    ///
    /// **判定は受け取った `f64` のまま行い、通った値だけを `f32` へ落とす**
    /// （送る本文の欄は `f32` —— [`RequestPolicy`]）。範囲の端は `f32` でも正確に表せる。
    #[must_use]
    pub fn parse(value: f64) -> Option<Self> {
        if !value.is_finite() || !(REPEAT_PENALTY_MIN..=REPEAT_PENALTY_MAX).contains(&value) {
            return None;
        }
        // **範囲の中の `f64` は `f32` に収まる**（落ちるのは精度だけ。送るのは `f32` の最短表記）。
        let narrowed = value as f32;
        Some(Self(narrowed))
    }

    /// 送る値。
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

/// **面に書いたサンプラの値**（ADR-0058 §決定1 / 作業単位 A-16）。**書いた欄だけが `Some`。**
///
/// **既定（[`Self::default`]）は「何も書いていない」** —— 送る1組は [`RequestPolicy::PERSONACORE`] のまま。
/// **値を持つのは `LmStudioConfig`（1つの上流クライアントの設定）**で、本文と裏方のクライアントが
/// それぞれ自分の面の値を持つ（`sub` 面が無ければ、裏方は本文のクライアントの複製なので本文の値を持つ）。
///
/// # いま書けるのは `repeat_penalty` だけ
///
/// ADR-0036 §決定3 の3条件を実測で示せているのが、サンプラの5つのうちこの1つだけだから
/// （ADR-0058 §決定1・§決定4）。ほかの4つを足すときは、この型に欄を足し、ADR に追記する。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sampling {
    repeat_penalty: Option<RepeatPenalty>,
}

impl Sampling {
    /// **繰り返しの罰を置き換える1組。**
    #[must_use]
    pub const fn with_repeat_penalty(repeat_penalty: RepeatPenalty) -> Self {
        Self {
            repeat_penalty: Some(repeat_penalty),
        }
    }

    /// 書いた繰り返しの罰。書いていなければ `None`。
    #[must_use]
    pub const fn repeat_penalty(&self) -> Option<RepeatPenalty> {
        self.repeat_penalty
    }

    /// **実際に送るサンプラの5つ**（ログに出す値 / ADR-0058 §決定3）。
    ///
    /// **送る本文と同じ関数（[`RequestPolicy::overridden_by`]）から作る** —— ログと線が食い違わない。
    /// **言えるのは「送った値」まで**で、上流で効いた値ではない（効いた値の権威は上流の側の観測）。
    #[must_use]
    pub const fn sent_samplers(self) -> SentSamplers {
        SentSamplers(RequestPolicy::PERSONACORE.overridden_by(self))
    }

    /// **面の値で置き換えた欄の名前**（送る本文の綴り。何も置き換えていなければ空）。
    #[must_use]
    pub fn overridden(self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        if self.repeat_penalty.is_some() {
            keys.push("repeat_penalty");
        }
        keys
    }
}

/// **実際に送るサンプラの5つ**（[`Sampling::sent_samplers`] / ADR-0058 §決定3）。ログに出すための読み口。
///
/// **小数は `Display`（`f32` の最短表記）で出すこと** —— 線に乗る綴りと同じになる。`f64` へ広げて記録すると
/// `0.949999988079071` のような、線に無い綴りがログに出る（`RequestPolicy` の doc の申し送り3）。
#[derive(Debug, Clone, Copy)]
pub struct SentSamplers(RequestPolicy);

impl SentSamplers {
    /// `top_k`。
    #[must_use]
    pub const fn top_k(&self) -> u32 {
        self.0.top_k
    }

    /// `top_p`。
    #[must_use]
    pub const fn top_p(&self) -> f32 {
        self.0.top_p
    }

    /// `min_p`。
    #[must_use]
    pub const fn min_p(&self) -> f32 {
        self.0.min_p
    }

    /// `repeat_penalty`。
    #[must_use]
    pub const fn repeat_penalty(&self) -> f32 {
        self.0.repeat_penalty
    }

    /// `presence_penalty`。
    #[must_use]
    pub const fn presence_penalty(&self) -> f32 {
        self.0.presence_penalty
    }
}

/// **線と同じ綴りで5つを並べる**（`top_k=40 top_p=0.95 …`）。
impl std::fmt::Display for SentSamplers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "top_k={} top_p={} min_p={} repeat_penalty={} presence_penalty={}",
            self.top_k(),
            self.top_p(),
            self.min_p(),
            self.repeat_penalty(),
            self.presence_penalty()
        )
    }
}

/// 送信本文。`stream` は呼び出し方（補完か中継か）で決まるので引数で受ける。
#[derive(Debug, Serialize)]
pub(super) struct WireRequest<'a> {
    /// **設定から来るモデル識別子。** リクエスト側からは差し込めない。
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    stream: bool,
    /// **どの経路でも必ず載る欄**（[`RequestPolicy`]）。**分岐の外側に置くこと。**
    #[serde(flatten)]
    policy: RequestPolicy,
    /// **ストリーミングのときだけ載せる**（`wire_request` の doc）。
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

/// ストリーミングの追加要求（OpenAI 互換）。
///
/// **今のところ `include_usage` だけ。** 増やすときは、載せると拒否する上流が
/// ありうることを前提に「本当に要るか」を先に確かめること。
#[derive(Debug, Serialize)]
struct StreamOptions {
    /// 最終チャンクに `usage` を載せるよう要求する。
    include_usage: bool,
}

#[derive(Debug, Serialize)]
struct WireMessage<'a> {
    role: &'static str,
    content: &'a str,
}

/// 送信本文を組み立てる。**純粋関数**（I/O も乱数も時刻も見ない）。
///
/// # ここが、上流へ出る本文を組み立てる唯一の場所
///
/// 本文・抽出・変容の3経路はどれも `LlmClient` を通り、
/// その唯一の実装（`LmStudioClient::post`）がこの関数を1度だけ呼ぶ。
/// **経路ごとに必ず載せたい欄は [`RequestPolicy`] に置く** ——
/// 3経路それぞれの組み立て地点（`server::chat` / `server::extract` / `server::drift`）に
/// 写しを置くと、**1つだけ落ちても他の2つが正しいので気づけない**
/// （CLAUDE.md「写しを作ったら、列・分岐・枝の1つずつに当てること」）。
///
/// **⚠ 「唯一」は型が守っているのではない。** 守っているのは次の2つだけ:
///
/// 1. **可視性** —— [`WireRequest`] も `wire_request` も `pub(super)` なので、
///    `llm` の外からは組み立てられない。
/// 2. **`LlmClient` の実装が本番に1つしか無いこと** ——
///    `LmStudioClient` だけ。**2つ目を足したら、この「唯一」は消える。**
///
/// **どちらもコンパイラは見張っていない。**
/// `llm` の中に別の本文組み立てを書けば、そのまま通る。
/// **[`RequestPolicy`] を「型で表現できなくした」とは書けない**（CLAUDE.md）。
///
/// **⚠ ソース走査の見張り番は置かない。** `15b` で「走査の対照が自己成就する」
/// （変異が探している綴りを走査対象そのものに書き込むと緑になる）ことと、
/// **既存の走査系テストのフィクスチャに誤爆する**ことの両方を踏むため。
/// **`LlmClient` の実装を2つ目にするときは、この段を読むこと。**
///
/// # ストリーミングでは使用量を明示的に要求する
///
/// **`stream_options.include_usage` を送らないと、LM Studio は最終チャンクに
/// `usage` を載せない**（2026-08-15 実測。送れば
/// `{"choices":[],"usage":{"prompt_tokens":537,…}}` が最後に届く）。
/// つまり送らない限り、答え合わせ（ADR-0013 第3層）は中継経路で
/// **材料を一度も受け取れないまま黙り続ける。**
///
/// **非ストリーミングでは送らない。** `usage` は本文と一緒に返るので不要で、
/// 未知の項目を拒否する上流がありうる。
///
/// # 面に書いたサンプラの値（ADR-0058 §決定1 / 作業単位 A-16）
///
/// **`sampling` は、そのクライアントの面に書いた値**（`LmStudioConfig` が持つ）。書いた欄だけが
/// [`RequestPolicy::PERSONACORE`] を置き換える。**何も書いていなければ、本文は1バイトも変わらない。**
pub(super) fn wire_request<'a>(
    model: &'a ModelId,
    request: &'a ChatRequest,
    stream: bool,
    sampling: Sampling,
) -> WireRequest<'a> {
    WireRequest {
        model: model.as_str(),
        messages: request
            .messages
            .iter()
            .map(|message| WireMessage {
                role: role_to_wire(message.role),
                content: &message.content,
            })
            .collect(),
        stream,
        // **条件を付けない。** 本文・抽出・変容の3経路はどれもここを通るので、
        // ここに書けば1経路だけ欠けることが起こらない（A-7b の狙い）。
        // **面に書いた値は、ここで置き換える**（ADR-0058 / 経路ではなくクライアントの面に付く値）。
        policy: RequestPolicy::PERSONACORE.overridden_by(sampling),
        stream_options: stream.then_some(StreamOptions {
            include_usage: true,
        }),
        temperature: request.temperature,
        max_tokens: request.max_tokens,
    }
}

#[cfg(test)]
mod api_key_tests {
    use super::*;

    /// **囮の鍵。** どの出口にも、この綴りが現れないことを見る。
    const PLANTED: &str = "sk-planted-KEY-0123456789-zz9Q";

    /// **受け付けるのは、空でない `0x21`〜`0x7E` だけ**（ADR-0056 §決定1）。両端の字も通す。
    #[test]
    fn a_key_admits_exactly_the_visible_ascii() {
        for raw in [PLANTED, "!", "~", "!~", &"a".repeat(API_KEY_MAX_BYTES)] {
            let key = ApiKey::parse(raw).unwrap_or_else(|err| panic!("{raw:?} は通る: {err}"));
            assert_eq!(key.expose_secret(), raw, "書いたまま持つ（整えない）");
        }
        for (raw, expected) in [
            ("", ApiKeyError::Empty),
            (" ", ApiKeyError::Character),
            ("ab cd", ApiKeyError::Character),
            (" abcd", ApiKeyError::Character),
            ("abcd\n", ApiKeyError::Character),
            ("ab\tcd", ApiKeyError::Character),
            ("ab\u{7f}cd", ApiKeyError::Character),
            ("鍵abcd", ApiKeyError::Character),
            ("abcd\u{a0}", ApiKeyError::Character),
        ] {
            assert_eq!(ApiKey::parse(raw), Err(expected), "{raw:?}");
        }
        // **上限は 4096 バイト**（1つ越えたら止める。境界の両側を1本に置く）。
        assert_eq!(
            ApiKey::parse(&"a".repeat(API_KEY_MAX_BYTES + 1)),
            Err(ApiKeyError::TooLong)
        );
    }

    /// **`Debug` は定数**（ADR-0056 §決定3）。包んだ型（`Option`・構造体）からも漏れない。
    #[test]
    fn the_debug_form_never_shows_the_key() {
        let key = ApiKey::parse(PLANTED).expect("通る");
        let shown = format!("{key:?} {:?} {:#?}", Some(&key), vec![key.clone()]);
        assert!(!shown.contains(PLANTED), "{shown}");
        assert!(!shown.contains("0123456789"), "一部も出さない: {shown}");
        assert!(shown.contains("ApiKey(***)"), "伏せたことは言う: {shown}");
    }

    /// **ヘッダは `Bearer <鍵>` で、`sensitive` が立っている**（ADR-0056 §決定2）。
    #[test]
    fn the_bearer_header_is_marked_sensitive() {
        let key = ApiKey::parse(PLANTED).expect("通る");
        let header = key.bearer();
        assert_eq!(header.to_str().expect("ASCII"), format!("Bearer {PLANTED}"));
        assert!(header.is_sensitive(), "set_sensitive(true) が要る");
        assert!(
            !format!("{header:?}").contains(PLANTED),
            "sensitive のヘッダは Debug に値を出さない"
        );
    }

    /// **末尾4文字だけを見せる。短い鍵は見せない**（見せると鍵の大半が出る）。
    #[test]
    fn the_hint_is_the_last_four_characters_of_a_long_enough_key() {
        assert_eq!(
            ApiKey::parse(PLANTED).expect("通る").hint().as_deref(),
            Some("zz9Q")
        );
        assert_eq!(
            ApiKey::parse("abcdefgh").expect("通る").hint().as_deref(),
            Some("efgh"),
            "8 文字なら見せる（境界）"
        );
        assert_eq!(
            ApiKey::parse("abcdefg").expect("通る").hint(),
            None,
            "7 文字は見せない（境界）"
        );
    }

    /// **本文から鍵を伏せる。** 鍵が無ければ触らない。
    #[test]
    fn redaction_replaces_every_occurrence_of_the_key() {
        let key = ApiKey::parse(PLANTED).expect("通る");
        let body = format!(r#"{{"error":"bad key {PLANTED}","echo":"{PLANTED}"}}"#);
        let redacted = redact_key(&body, Some(&key));
        assert!(!redacted.contains(PLANTED), "{redacted}");
        assert_eq!(
            redacted, r#"{"error":"bad key ***","echo":"***"}"#,
            "鍵の外は1文字も変えない"
        );
        assert_eq!(redact_key(&body, None), body, "鍵の無い面は触らない");
    }

    /// **鍵を付けた面の宛先は、`reqwest` が実際に繋ぐホストで判定する**
    /// （作業単位 A-13b-1 のレビュー是正 / ADR-0056 追記5 / A-13b-2 で「止める」から「判定の仕方」へ）。
    ///
    /// **`\` を挟んだ綴りは、権限部を `@` で割る読み方と WHATWG で宛先が割れる。** どちらの向きも、
    /// 前提として `reqwest` が読むホストを確かめてから判定を見る。
    #[test]
    fn the_key_destination_is_the_host_reqwest_will_dial() {
        use std::net::IpAddr;

        let address = |raw: &str| KeyDestination::Address(raw.parse::<IpAddr>().expect("正当"));
        let name = |raw: &str| KeyDestination::Name(raw.to_owned());
        for (endpoint, dialed, expected) in [
            (
                r"http://203.0.113.5\@127.0.0.1:46001",
                "203.0.113.5",
                address("203.0.113.5"),
            ),
            (
                r"http://127.0.0.1\@203.0.113.5:1",
                "127.0.0.1",
                KeyDestination::Unguarded,
            ),
            (
                "http://127.0.0.1:46001",
                "127.0.0.1",
                KeyDestination::Unguarded,
            ),
            ("http://127.9.9.9:1", "127.9.9.9", KeyDestination::Unguarded),
            ("http://LOCALHOST:1", "localhost", KeyDestination::Unguarded),
            ("http://[::1]:1", "[::1]", KeyDestination::Unguarded),
            (
                "https://203.0.113.5:443",
                "203.0.113.5",
                KeyDestination::Unguarded,
            ),
            (
                "https://example.com:443",
                "example.com",
                KeyDestination::Unguarded,
            ),
            (
                "http://192.168.0.10:1",
                "192.168.0.10",
                address("192.168.0.10"),
            ),
            ("http://0.0.0.0:1", "0.0.0.0", address("0.0.0.0")),
            ("http://[::]:1", "[::]", address("::")),
            ("http://[fe80::1]:1", "[fe80::1]", address("fe80::1")),
            // **WHATWG は 10 進の1語も IPv4 と読む**（綴りの見た目で判定しない）。
            (
                "http://3232235530:1",
                "192.168.0.10",
                address("192.168.0.10"),
            ),
            ("http://example.com:1", "example.com", name("example.com")),
            (
                "http://u:p@upstream.test:1",
                "upstream.test",
                name("upstream.test"),
            ),
        ] {
            assert_eq!(
                dialed_host(endpoint).as_deref(),
                Some(dialed),
                "前提: {endpoint}"
            );
            assert_eq!(key_destination(endpoint), expected, "{endpoint}");
        }
        // **読めない綴り・ほかのスキームは判定しない**（`reqwest` が送信を組めない。
        // `LmStudioConfig::new` も `http` / `https` 以外を止める）。
        for endpoint in ["", "127.0.0.1:1", "ftp://203.0.113.5:1", "http://"] {
            assert_eq!(
                key_destination(endpoint),
                KeyDestination::Unguarded,
                "{endpoint:?}"
            );
        }
    }

    /// **範囲の端を、内と外の両側で見る**（ADR-0056 §決定5 / Issue origin#85 の見張り番）。
    ///
    /// **`100.64.0.0/10`（CGNAT）は外**（通信事業者の網を通りうる）。IPv4 を写した IPv6 も外。
    #[test]
    fn the_local_network_ranges_hold_at_both_edges() {
        use std::net::IpAddr;

        for (raw, expected) in [
            // ループバック
            ("127.0.0.0", AddressClass::Loopback),
            ("127.255.255.255", AddressClass::Loopback),
            ("126.255.255.255", AddressClass::Elsewhere),
            ("128.0.0.0", AddressClass::Elsewhere),
            ("::1", AddressClass::Loopback),
            // 10.0.0.0/8
            ("9.255.255.255", AddressClass::Elsewhere),
            ("10.0.0.0", AddressClass::LocalNetwork),
            ("10.255.255.255", AddressClass::LocalNetwork),
            ("11.0.0.0", AddressClass::Elsewhere),
            // 172.16.0.0/12
            ("172.15.255.255", AddressClass::Elsewhere),
            ("172.16.0.0", AddressClass::LocalNetwork),
            ("172.31.255.255", AddressClass::LocalNetwork),
            ("172.32.0.0", AddressClass::Elsewhere),
            // 192.168.0.0/16
            ("192.167.255.255", AddressClass::Elsewhere),
            ("192.168.0.0", AddressClass::LocalNetwork),
            ("192.168.255.255", AddressClass::LocalNetwork),
            ("192.169.0.0", AddressClass::Elsewhere),
            // 169.254.0.0/16
            ("169.253.255.255", AddressClass::Elsewhere),
            ("169.254.0.0", AddressClass::LocalNetwork),
            ("169.254.255.255", AddressClass::LocalNetwork),
            ("169.255.0.0", AddressClass::Elsewhere),
            // 100.64.0.0/10 は含めない（端の内側も外）
            ("100.63.255.255", AddressClass::Elsewhere),
            ("100.64.0.0", AddressClass::Elsewhere),
            ("100.127.255.255", AddressClass::Elsewhere),
            ("100.128.0.0", AddressClass::Elsewhere),
            // そのほか
            ("0.0.0.0", AddressClass::Elsewhere),
            ("203.0.113.5", AddressClass::Elsewhere),
            ("255.255.255.255", AddressClass::Elsewhere),
            // fc00::/7
            (
                "fbff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
                AddressClass::Elsewhere,
            ),
            ("fc00::", AddressClass::LocalNetwork),
            (
                "fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
                AddressClass::LocalNetwork,
            ),
            ("fe00::", AddressClass::Elsewhere),
            // fe80::/10
            (
                "fe7f:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
                AddressClass::Elsewhere,
            ),
            ("fe80::", AddressClass::LocalNetwork),
            (
                "febf:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
                AddressClass::LocalNetwork,
            ),
            ("fec0::", AddressClass::Elsewhere),
            // IPv6 のそのほか
            ("::", AddressClass::Elsewhere),
            ("2001:db8::1", AddressClass::Elsewhere),
            ("::ffff:10.0.0.1", AddressClass::Elsewhere),
            ("::ffff:127.0.0.1", AddressClass::Elsewhere),
        ] {
            let ip: IpAddr = raw.parse().expect("正当なアドレス");
            assert_eq!(address_class(ip), expected, "{raw}");
        }
    }

    /// **1つでも外が混ざれば承認が要る**（ADR-0056 §決定5）。全部ローカル・全部ループバック・空も見る。
    ///
    /// **外を列の先頭にも末尾にも置く**（先頭だけ・末尾だけを見る実装を赤にする）。
    #[test]
    fn one_outside_address_among_many_needs_the_approval() {
        use std::net::IpAddr;

        let ips = |raw: &[&str]| -> Vec<IpAddr> {
            raw.iter()
                .map(|one| one.parse().expect("正当なアドレス"))
                .collect()
        };
        for (addresses, approved, expected) in [
            (ips(&[]), false, None),
            (ips(&["127.0.0.1", "::1"]), false, None),
            (ips(&["127.0.0.1", "::1"]), true, None),
            (
                ips(&["127.0.0.1", "192.168.0.7", "fe80::1"]),
                false,
                Some(KeyOverPlainHttp::LocalNetwork),
            ),
            (
                ips(&["10.0.0.5"]),
                true,
                Some(KeyOverPlainHttp::LocalNetwork),
            ),
            (
                ips(&["127.0.0.1", "10.0.0.5", "203.0.113.5"]),
                false,
                Some(KeyOverPlainHttp::NotApproved),
            ),
            (
                ips(&["203.0.113.5", "127.0.0.1", "10.0.0.5"]),
                false,
                Some(KeyOverPlainHttp::NotApproved),
            ),
            (
                ips(&["127.0.0.1", "100.64.0.1"]),
                true,
                Some(KeyOverPlainHttp::Approved),
            ),
            (
                ips(&["2001:db8::1"]),
                false,
                Some(KeyOverPlainHttp::NotApproved),
            ),
        ] {
            assert_eq!(
                judge_addresses(addresses.iter().copied(), approved),
                expected,
                "{addresses:?} / 承認 {approved}"
            );
        }
    }

    /// **ユーザー情報は、送信と同じパーサで読む**（作業単位 A-13b-2）。
    #[test]
    fn userinfo_is_read_the_way_reqwest_reads_it() {
        for (endpoint, has) in [
            ("http://user:pass@192.0.2.1:1", true),
            ("http://user@192.0.2.1:1", true),
            ("http://:pass@192.0.2.1:1", true),
            ("https://user:pass@example.com", true),
            ("http://192.0.2.1:1", false),
            ("http://@192.0.2.1:1", false),
            // **WHATWG では `\` が経路の区切りなので、`@` の手前は権限部ではない。**
            (r"http://203.0.113.5\@127.0.0.1:46001", false),
            ("http://192.0.2.1:1/p@th", false),
            ("", false),
        ] {
            assert_eq!(endpoint_has_userinfo(endpoint), has, "{endpoint:?}");
        }
    }

    /// **ログに出す宛先は、ユーザー情報・クエリ・フラグメントを落とし、ホスト・ポート・パスは残す**
    /// （作業単位 B-71 / Issue origin#100。純粋関数の表）。
    ///
    /// **期待値は手で書く**（関数から作ると、緩めたときに期待値も一緒に動く）。
    #[test]
    fn the_endpoint_for_log_drops_userinfo_query_and_fragment_but_keeps_the_destination() {
        for (endpoint, expected) in [
            (
                "http://127.0.0.1:1234/v1/chat/completions",
                "http://127.0.0.1:1234/v1/chat/completions",
            ),
            // **落とすものが無ければ書いたまま**（WHATWG の `/` を足さない・既定のポートを消さない）。
            ("http://127.0.0.1:9", "http://127.0.0.1:9"),
            ("http://127.0.0.1:80", "http://127.0.0.1:80"),
            // **落とすものがあれば WHATWG の綴り**（`/` が付く）。空のクエリも落とす。
            ("http://127.0.0.1:9?k=q-secret", "http://127.0.0.1:9/"),
            ("http://127.0.0.1:9/v1?", "http://127.0.0.1:9/v1"),
            ("http://127.0.0.1:9/v1#", "http://127.0.0.1:9/v1"),
            // **ホストを持たない URL は出さない**（B-71 のレビュー是正）。`http://` の書き忘れは WHATWG では
            // スキーム `user`・パス `pw-secret@…` と読めて、ユーザー情報もクエリも空になる ——
            // 「書いたまま」の枝に入ってパスワードが出ていた。
            ("user:pw-secret@127.0.0.1:8080", "<読めない宛先>"),
            ("localhost:1234", "<読めない宛先>"),
            (
                "http://user:pw-secret@127.0.0.1:47941/v1",
                "http://127.0.0.1:47941/v1",
            ),
            (
                "http://user@127.0.0.1:47941/v1",
                "http://127.0.0.1:47941/v1",
            ),
            (
                "http://:pw-secret@127.0.0.1:47941/v1",
                "http://127.0.0.1:47941/v1",
            ),
            (
                "http://127.0.0.1:47941/v1?token=q-secret",
                "http://127.0.0.1:47941/v1",
            ),
            (
                "http://127.0.0.1:47941/v1#f-secret",
                "http://127.0.0.1:47941/v1",
            ),
            (
                "http://user:pw-secret@127.0.0.1:47941/v1?token=q-secret#f-secret",
                "http://127.0.0.1:47941/v1",
            ),
            // **宛先に付け足したパスはクエリに呑まれる**（送信が実際に叩く経路は `/v1`）。
            (
                "http://user:pw-secret@127.0.0.1:47941/v1?token=q-secret/v1/chat/completions",
                "http://127.0.0.1:47941/v1",
            ),
            (
                "https://user:pw-secret@example.com:8443/api?k=q-secret",
                "https://example.com:8443/api",
            ),
            ("http://[::1]:1234/v1?k=q-secret", "http://[::1]:1234/v1"),
            // **経路の中の `@` は利用者名ではない。** 触らない。
            (
                "http://127.0.0.1:1234/mail@host",
                "http://127.0.0.1:1234/mail@host",
            ),
            // **読めない綴りは宛先を出さない**（伏せ損ねた綴りをそのまま出さない）。
            ("127.0.0.1:1234", "<読めない宛先>"),
            ("http://user:pw-secret@[::1", "<読めない宛先>"),
        ] {
            assert_eq!(endpoint_for_log(endpoint), expected, "{endpoint:?}");
        }
    }

    /// **綴りは閉じていて、互いに違う**（画面と仕様書が読む）。
    #[test]
    fn every_key_over_plain_http_label_is_distinct() {
        let labels: std::collections::BTreeSet<&str> = KeyOverPlainHttp::ALL
            .iter()
            .map(|value| value.label())
            .collect();
        assert_eq!(
            labels,
            ["approved", "local_network", "not_approved"]
                .into_iter()
                .collect()
        );
    }

    /// **画面の宛先の分類（`web/src/lib/upstreamKey.ts` の `keyDestination`）と同じ表を、ここでも読む**
    /// （作業単位 A-13b-3）。
    ///
    /// 画面は保存の前に「承認を求めるか」「ローカルネットワークの警告を出すか」を決めるために、
    /// この関数と同じ分類を TypeScript で持つ（写し）。**両側が同じ表**
    /// （`tests/fixtures/key_destination_corpus.tsv`）**を読む**ので、Rust の判定を変えればここが、
    /// 画面が追随していなければ `web/tests/upstream-key.test.ts` が赤になる。
    ///
    /// **ここは判定を変えない**（読むだけのテスト）。表の4つの分類が全部出ること・行が空でないことも見る
    /// （読めていないから緑、を通さない）。
    #[test]
    fn the_key_destination_corpus_matches_the_judgment() {
        const CORPUS: &str = include_str!("../../tests/fixtures/key_destination_corpus.tsv");
        let class = |endpoint: &str| match key_destination(endpoint) {
            KeyDestination::Unguarded => "unguarded",
            KeyDestination::Address(address) => match address_class(address) {
                // **ループバックは `Unguarded` で返る**ので、ここへ来たら判定の側の食い違い（表に無い綴り）。
                AddressClass::Loopback => "address_loopback",
                AddressClass::LocalNetwork => "local_network",
                AddressClass::Elsewhere => "elsewhere",
            },
            KeyDestination::Name(_) => "name",
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut rows = 0;
        for line in CORPUS.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let (endpoint, expected) = line.split_once('\t').expect("宛先<TAB>分類 の形");
            assert_eq!(class(endpoint), expected, "{endpoint:?}");
            seen.insert(expected);
            rows += 1;
        }
        assert!(rows >= 40, "表が痩せている（{rows} 行）");
        assert_eq!(
            seen.into_iter().collect::<Vec<_>>(),
            ["elsewhere", "local_network", "name", "unguarded"]
        );
    }

    /// **JSON の文字列としてエスケープされた綴りも伏せる**（作業単位 A-13b-1 のレビュー是正）。
    ///
    /// 鍵は `"` と `\` を含みうる（`0x21`〜`0x7E`）。上流が鍵を JSON の本文に写し返すと、
    /// `"` は `\"`、`\` は `\\` になり、`/` を `\/` と書く上流もある。**生の綴りの置換だけでは、
    /// どれも伏せられずに残る。** 3つの形を1つの本文に並べ、どれも残らないこと・鍵の外は変えないことを見る。
    #[test]
    fn redaction_also_replaces_the_json_escaped_forms_of_the_key() {
        let raw = r#"k"ey\path/seg-9a8b"#;
        let key = ApiKey::parse(raw).expect("通る");
        let escaped = r#"k\"ey\\path/seg-9a8b"#;
        let slashed = r#"k\"ey\\path\/seg-9a8b"#;
        let body = format!(r#"raw={raw} json={escaped} slash={slashed} end"#);
        let redacted = redact_key(&body, Some(&key));
        assert_eq!(redacted, "raw=*** json=*** slash=*** end", "{redacted}");
        // **前提**: 3つの形は互いに違う（同じなら1つの置換で全部消え、何も見ていない）。
        assert!(raw != escaped && escaped != slashed && raw != slashed);
        // **前提**: エスケープした形は、生の綴りを部分に含まない（生の置換だけでは消えない）。
        assert!(!escaped.contains(raw) && !slashed.contains(raw));
    }

    /// **断られた（401 / 403）と、それ以外の失敗を分ける**（ADR-0056 §決定4）。
    #[test]
    fn only_401_and_403_read_as_a_refused_key() {
        use super::super::LlmError;
        for status in [401, 403] {
            assert!(LlmError::Status { status }.refused_the_key(), "{status}");
        }
        for status in [400, 402, 404, 407, 429, 500, 502, 503] {
            assert!(!LlmError::Status { status }.refused_the_key(), "{status}");
        }
        assert!(!LlmError::Truncated.refused_the_key());
        // **承認が無くて送らなかったのは、上流が断ったのではない**（作業単位 A-13b-2）。
        assert!(!LlmError::KeyNotApproved.refused_the_key());
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn model() -> ModelId {
        ModelId::parse("ministral-3-14b-instruct-2512").expect("正当な識別子であること")
    }

    fn to_value(request: &ChatRequest, stream: bool) -> Value {
        serde_json::to_value(wire_request(&model(), request, stream, Sampling::default()))
            .expect("送信本文は JSON にできること")
    }

    /// **線に実際に乗るバイト列。** `reqwest` の `.json()` が通るのはこちら。
    ///
    /// **[`to_value`] とは見えるものが違う。** `serde_json::to_value` は
    /// `serde_json::Map` へ畳むので、**重複キーが消える。**
    /// `#[serde(flatten)]` はマップ直列化なので、[`RequestPolicy`] に
    /// [`WireRequest`] と同じ名前の欄を足すと**同じキーが2度出た本文がそのまま線に乗る**
    /// （C-13 が実測 / `RequestPolicy` の doc の申し送り2）。
    /// **その形は `to_value` を通す検査からは原理的に見えない。**
    fn to_wire(request: &ChatRequest, stream: bool) -> String {
        to_wire_with(request, stream, Sampling::default())
    }

    /// **面に書いた値を持つクライアントが線に載せるバイト列**（ADR-0058 / 作業単位 A-16）。
    fn to_wire_with(request: &ChatRequest, stream: bool, sampling: Sampling) -> String {
        serde_json::to_string(&wire_request(&model(), request, stream, sampling))
            .expect("送信本文は JSON にできること")
    }

    fn penalty(value: f64) -> RepeatPenalty {
        RepeatPenalty::parse(value).unwrap_or_else(|| panic!("{value} は範囲の中であること"))
    }

    /// **書いた `repeat_penalty` だけが線の上で置き換わる**（ADR-0058 §決定1 / 作業単位 A-16）。
    ///
    /// **期待値は手で書いたバイト列**（[`the_wire_bytes_spell_the_request_exactly`] と同じ形）。
    /// 既定の `1.1` と違う値（`1.05`）にし、**`f32` で割り切れない値**にしてある —— `to_value` を経由する
    /// 実装に変わると `1.0499999523162842` になって赤くなる。
    ///
    /// 対照: 何も書いていない [`Sampling::default`] は既定の綴りのまま（[`the_wire_bytes_spell_the_request_exactly`]
    /// が同じ依頼で見ている。ここでも同じ依頼で並べ、置き換わったのがその1欄だけであることを見る）。
    #[test]
    fn a_written_repeat_penalty_replaces_only_that_field_on_the_wire() {
        let request = ChatRequest {
            messages: vec![ChatMessage::user("こんにちは")],
            temperature: Some(0.5),
            max_tokens: None,
        };
        let written = to_wire_with(
            &request,
            false,
            Sampling::with_repeat_penalty(penalty(1.05)),
        );
        assert_eq!(
            written,
            concat!(
                r#"{"model":"ministral-3-14b-instruct-2512","#,
                r#""messages":[{"role":"user","content":"こんにちは"}],"#,
                r#""stream":false,"reasoning_effort":"none","#,
                r#""top_k":40,"top_p":0.95,"min_p":0.05,"repeat_penalty":1.05,"#,
                r#""presence_penalty":0.0,"temperature":0.5}"#,
            )
        );
        // **対照**: 書いていなければ既定の 1.1 —— 違うのはその1欄だけ。
        assert_eq!(
            to_wire_with(&request, false, Sampling::default()),
            written.replace(r#""repeat_penalty":1.05"#, r#""repeat_penalty":1.1"#)
        );
    }

    /// **範囲**（[`REPEAT_PENALTY_MIN`] の表）。端は通し、外は止める。NaN・無限・0・負も止める。
    ///
    /// **期待値は定数から作らない**（`0.5` / `2.0` を手で書く）。
    #[test]
    fn a_repeat_penalty_admits_exactly_the_documented_range() {
        for value in [0.5, 0.9, 1.0, 1.05, 1.1, 1.3, 2.0] {
            let parsed =
                RepeatPenalty::parse(value).unwrap_or_else(|| panic!("{value} は通ること"));
            // 送るのは `f32` に落とした値（端は正確に表せる）。
            assert_eq!(f64::from(parsed.get()), f64::from(value as f32), "{value}");
        }
        for value in [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -0.0,
            -1.1,
            f64::from_bits(1),
            0.499_999_999,
            2.000_000_001,
            11.0,
            f64::MAX,
        ] {
            assert_eq!(RepeatPenalty::parse(value), None, "{value} は止めること");
        }
    }

    /// **送るサンプラの5つと、置き換えた欄の名前**（ログの形 / ADR-0058 §決定3）。
    ///
    /// 書いていなければ既定の5つで置き換えた欄は無く、書いたら `repeat_penalty` だけが変わる。
    #[test]
    fn the_sent_samplers_name_the_values_on_the_wire() {
        assert_eq!(
            Sampling::default().sent_samplers().to_string(),
            "top_k=40 top_p=0.95 min_p=0.05 repeat_penalty=1.1 presence_penalty=0"
        );
        assert!(Sampling::default().overridden().is_empty());
        let written = Sampling::with_repeat_penalty(penalty(1.05));
        assert_eq!(
            written.sent_samplers().to_string(),
            "top_k=40 top_p=0.95 min_p=0.05 repeat_penalty=1.05 presence_penalty=0"
        );
        assert_eq!(written.overridden(), vec!["repeat_penalty"]);
    }

    /// **同じ値なら等しく、違う値なら等しくない**（手で書いた `Eq` / 切り替えの比較が使う）。
    #[test]
    fn repeat_penalties_compare_by_value() {
        assert_eq!(penalty(1.05), penalty(1.05));
        assert_ne!(penalty(1.05), penalty(1.0));
        assert_ne!(
            Sampling::with_repeat_penalty(penalty(1.1)),
            Sampling::default(),
            "既定と同じ値を書いた面も「書いた」面である"
        );
    }

    /// **線に乗ったバイト列を読み直した値。**
    ///
    /// [`to_value`] と違い、**数は綴りどおりに読める** ——
    /// `0.95_f32` を `to_value` に通すと `0.949999988079071` になるが、
    /// **線に乗るのは `0.95` の 4 バイト**なので、読み直せば `json!(0.95)` と等しい
    /// （`RequestPolicy` の doc の申し送り3）。
    ///
    /// **重複キーはここでも畳まれる**（読み直しはマップを作る）。
    /// **重複を見るのは [`to_wire`] を直に見る側の仕事。**
    fn wire_value(request: &ChatRequest, stream: bool) -> Value {
        serde_json::from_str(&to_wire(request, stream)).expect("線に乗った本文は JSON であること")
    }

    /// 最上位のキーを**出現順のまま、重複も畳まずに**拾う。
    ///
    /// `serde_json::Value` へ読むと `Map::insert` が後勝ちで畳んでしまうので、
    /// **`MapAccess` を自分で回す**（こちらは本文に現れた順に1件ずつ渡してくる）。
    struct TopLevelKeys(Vec<String>);

    impl<'de> serde::Deserialize<'de> for TopLevelKeys {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            struct Keys;

            impl<'de> serde::de::Visitor<'de> for Keys {
                type Value = Vec<String>;

                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("JSON オブジェクト")
                }

                fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                where
                    A: serde::de::MapAccess<'de>,
                {
                    let mut keys = Vec::new();
                    while let Some(key) = map.next_key::<String>()? {
                        map.next_value::<serde::de::IgnoredAny>()?;
                        keys.push(key);
                    }
                    Ok(keys)
                }
            }

            deserializer.deserialize_map(Keys).map(Self)
        }
    }

    /// 線に乗ったバイト列の最上位キーを、**重複を畳まずに**並べ替えて返す。
    fn sorted_top_level_keys(wire: &str) -> Vec<String> {
        let TopLevelKeys(mut keys) =
            serde_json::from_str(wire).expect("線に乗った本文は JSON オブジェクトであること");
        keys.sort();
        keys
    }

    /// 前後の空白は落ちる。中間は保持する（識別子に空白は無い想定だが正規化しすぎない）。
    #[test]
    fn model_id_trims_surrounding_blanks() {
        assert_eq!(
            ModelId::parse("  ministral-3-3b-instruct-2512\t")
                .expect("正当")
                .as_str(),
            "ministral-3-3b-instruct-2512"
        );
        // `/` 入りの識別子は正当（LM Studio に実在する）。
        assert_eq!(
            ModelId::parse("mistralai/devstral-small-2-2512")
                .expect("正当")
                .as_str(),
            "mistralai/devstral-small-2-2512"
        );
    }

    /// 空と制御文字は拒否する。理由は区別できる。
    #[test]
    fn model_id_rejects_blank_and_control() {
        for raw in ["", " ", "\t", "\r\n", "   \n "] {
            assert_eq!(
                ModelId::parse(raw),
                Err(ModelIdError::Empty),
                "raw = {raw:?}"
            );
        }
        for raw in ["a\nb", "a\u{0000}b", "mo\u{0007}del", "a\tb"] {
            assert_eq!(
                ModelId::parse(raw),
                Err(ModelIdError::Control),
                "raw = {raw:?}"
            );
        }
        assert_ne!(
            ModelIdError::Empty.to_string(),
            ModelIdError::Control.to_string()
        );
    }

    /// 綴りは OpenAI 互換。**往復だけでは取り違えを検出できない**ので値を固定する。
    #[test]
    fn role_uses_the_openai_spelling() {
        assert_eq!(role_to_wire(ChatRole::System), "system");
        assert_eq!(role_to_wire(ChatRole::User), "user");
        assert_eq!(role_to_wire(ChatRole::Assistant), "assistant");
    }

    /// 送信本文の形。モデルは設定由来、メッセージは順序どおり。
    ///
    /// **本文全体との等値で見ている。** 欄が1つ落ちても、余計な欄が1つ増えても赤になる。
    ///
    /// **⚠ 見ているのは線に乗ったバイト列を読み直した値**（[`wire_value`]）**で、
    /// `to_value` ではない**（A-7c で切り替えた）。`to_value` は `f32` を `f64` へ広げるので、
    /// **`0.95_f32` が `0.949999988079071` になり、期待値に書いた綴りが線の綴りと食い違う。**
    /// **どちらも重複キーは畳む**ので、重複を見るのは
    /// [`the_wire_bytes_carry_each_top_level_key_exactly_once`] の仕事。
    #[test]
    fn wire_request_carries_model_and_messages_in_order() {
        let request = ChatRequest::new(vec![
            ChatMessage::system("ペルソナ"),
            ChatMessage::user("こんにちは"),
            ChatMessage::assistant("やあ"),
        ]);

        assert_eq!(
            wire_value(&request, false),
            json!({
                "model": "ministral-3-14b-instruct-2512",
                "stream": false,
                "reasoning_effort": "none",
                "top_k": 40,
                "top_p": 0.95,
                "min_p": 0.05,
                "repeat_penalty": 1.1,
                "presence_penalty": 0.0,
                "messages": [
                    {"role": "system", "content": "ペルソナ"},
                    {"role": "user", "content": "こんにちは"},
                    {"role": "assistant", "content": "やあ"},
                ],
            })
        );
    }

    /// **線に乗ったバイト列の最上位キーは、1つずつしか出ない。**
    ///
    /// # なぜ `to_value` を通す検査では足りないのか
    ///
    /// [`RequestPolicy`] は `#[serde(flatten)]` で載る。**マップ直列化なので、
    /// [`WireRequest`] と同じ名前の欄をこちらに足すと、`serde_json::to_string` は
    /// 重複キーをそのまま線に載せる**（C-13 が実測。
    /// `…,"temperature":9.5,…,"temperature":0.5` という線を実際に作れている）。
    /// **`serde_json::to_value` は畳む**ので、`to_value` を通す検査は
    /// **この形を原理的に見られない。**
    ///
    /// **上流（LM Studio）は 200 を返し、後ろ勝ちだった。**
    /// **JSON の仕様上は未定義**なので、**どちらが勝つかは上流ごとに違いうる。**
    ///
    /// # 同じ値の重複でも赤にする
    ///
    /// **値が同じなら害が無い、ではない** ——
    /// 同じ欄を2箇所が書いている状態そのものが、片方だけ動いたときに
    /// 出力から気づけない形（CLAUDE.md「写しから列が1つだけ落ちる」）。
    /// **A-7c で `reasoning_effort` を [`WireRequest`] 側にも同じ値で載せる変異を当てて、
    /// 是正前は 1731 全緑・是正後は2本が赤**であることを実測した
    /// （**この1本と [`the_wire_bytes_spell_the_request_exactly`]**。
    /// **「1本」ではない** —— あちらを緩めるときに、ここが唯一の権威だと誤読しないこと）。
    ///
    /// **⚠ この道具（[`TopLevelKeys`]）自体の対照は
    /// [`the_key_collector_keeps_duplicates`] が持つ。**
    /// **無いと、道具を「畳む」形に壊す変異が 1734 全緑を通る**（レビューが実測）。
    ///
    /// **⚠ 期待値は [`RequestPolicy`] からも [`WireRequest`] からも作らない**
    /// （CLAUDE.md「関数の外にある値で押さえる見張り番」）。**綴りを手で書く。**
    #[test]
    fn the_wire_bytes_carry_each_top_level_key_exactly_once() {
        // **どの依頼にも必ず出る欄。** 経路にも分岐にも依存しない。
        let always = [
            "model",
            "messages",
            "stream",
            "reasoning_effort",
            "top_k",
            "top_p",
            "min_p",
            "repeat_penalty",
            "presence_penalty",
        ];

        let cases = [
            (
                "既定",
                ChatRequest::new(vec![ChatMessage::user("x")]),
                &[][..],
            ),
            ("メッセージ無し", ChatRequest::new(Vec::new()), &[][..]),
            (
                "クライアントの指定あり",
                ChatRequest {
                    messages: vec![ChatMessage::system("ペルソナ"), ChatMessage::user("x")],
                    temperature: Some(0.5),
                    max_tokens: Some(512),
                },
                &["temperature", "max_tokens"][..],
            ),
        ];

        for (label, request, extra) in &cases {
            for stream in [false, true] {
                let wire = to_wire(request, stream);

                let mut expected: Vec<String> = always
                    .iter()
                    .chain(extra.iter())
                    .map(|key| (*key).to_owned())
                    .collect();
                if stream {
                    // ストリーミングのときだけ載る（`wire_request` の doc）。
                    expected.push("stream_options".to_owned());
                }
                expected.sort();

                assert_eq!(
                    sorted_top_level_keys(&wire),
                    expected,
                    "{label} / stream={stream}: 最上位のキーは1つずつであること（重複キーは to_value では見えない）: {wire}"
                );
            }
        }
    }

    /// **道具そのものの対照 —— [`TopLevelKeys`] は重複を畳まない。**
    ///
    /// # なぜ本文を通すだけでは足りないか
    ///
    /// [`the_wire_bytes_carry_each_top_level_key_exactly_once`] は
    /// **正しい実装なら重複の無い本文しか通らない。**
    /// つまり **「重複が無いから緑」と「道具が畳んでいるから緑」が区別できない。**
    /// 実測: `keys.push(key)` を `if !keys.contains(&key) { keys.push(key) }` に変えると
    /// **1734 全緑**を通り、**そのとき残る赤は
    /// [`the_wire_bytes_spell_the_request_exactly`] の1本だけ**（しかも
    /// **6通りのうち1通りしか見ていない**）。
    /// **「`serde_json::Map` を使って簡単にする」変更が、塞いだ穴の 5/6 を静かに開ける形。**
    ///
    /// **CLAUDE.md `15b` と同じ形** ——
    /// **走査の対照は、テストが植えた入力を走査本体に通すこと。**
    #[test]
    fn the_key_collector_keeps_duplicates() {
        // **手で書いた本文。** `wire_request` は通さない（正しい実装では重複を作れない）。
        assert_eq!(
            sorted_top_level_keys(r#"{"a":1,"b":2,"a":3}"#),
            vec!["a".to_owned(), "a".to_owned(), "b".to_owned()],
            "同じキーが2度出た本文は、2度のまま拾うこと"
        );
        // 値が同じ重複でも畳まない（線に乗った以上、2度出たことが事実）。
        assert_eq!(
            sorted_top_level_keys(r#"{"a":1,"a":1}"#),
            vec!["a".to_owned(), "a".to_owned()],
            "値まで同じ重複も、2度のまま拾うこと"
        );
        // 対照: 重複が無ければ、そのまま並べ替えて返す（数を水増ししていない）。
        assert_eq!(
            sorted_top_level_keys(r#"{"b":1,"a":2}"#),
            vec!["a".to_owned(), "b".to_owned()],
            "重複が無い本文を膨らませないこと"
        );
    }

    /// **線に乗るバイト列そのもの。** 綴りも順序も、これが全部。
    ///
    /// # ここでしか言えないこと —— **綴りと、上流が受け取る値が一致すること**
    ///
    /// [ADR-0038](../../docs/adr/0038-what-personacore-requires-of-an-upstream.md) §決定2 の
    /// 目的は**測定が再現すること**なので、
    /// **こちらが書いた綴りがそのまま線に乗るか**は、この決定の中身そのものである。
    ///
    /// **serde は最短表記を書く**（C-13 が記録プロキシの生バイトで実測 / 2026-08-29）——
    /// `"top_p":0.95` / `"min_p":0.05` / `"repeat_penalty":1.1`。
    /// **`0.949999988079071` が出るのは `serde_json::Value` を経由したときだけ**で、
    /// **それは線に乗るバイト列ではない**（A-7b の申し送り3 はここを取り違えていた）。
    ///
    /// **⚠ 期待値は構造体から作らない。** 1文字ずつ手で書いた文字列リテラル。
    #[test]
    fn the_wire_bytes_spell_the_request_exactly() {
        let request = ChatRequest {
            messages: vec![
                ChatMessage::system("ペルソナ"),
                ChatMessage::user("こんにちは"),
            ],
            temperature: Some(0.5),
            max_tokens: Some(512),
        };

        assert_eq!(
            to_wire(&request, true),
            concat!(
                r#"{"model":"ministral-3-14b-instruct-2512","#,
                r#""messages":[{"role":"system","content":"ペルソナ"},"#,
                r#"{"role":"user","content":"こんにちは"}],"#,
                r#""stream":true,"reasoning_effort":"none","#,
                r#""top_k":40,"top_p":0.95,"min_p":0.05,"repeat_penalty":1.1,"#,
                r#""presence_penalty":0.0,"#,
                r#""stream_options":{"include_usage":true},"#,
                r#""temperature":0.5,"max_tokens":512}"#,
            ),
            "線に乗るバイト列が変わったら、測定はその日で切れる（ADR-0038 §決定2）"
        );
    }

    /// **サンプラーは、どんな要求にも必ず5つとも載る**（ADR-0038 §決定2 /
    /// `presence_penalty` は A-14 で足した）。
    ///
    /// 指定しない欄は上流の既定が当たり、**その既定は観測できず、上流ごとに違う。**
    /// **明示にして初めて「どの設定で測ったか」が言える**——それがこの決定の目的で、
    /// **出力を良くするためではない**（C-13 の実測: 何も送らない腕と、いまの既定を
    /// 明示した腕は、3種類の固定依頼・全部の長さの帯で見分けられなかった）。
    ///
    /// **⚠ 5欄を1つずつ見る。代表1つで済ませない**
    /// （CLAUDE.md「写しから列が1つだけ落ちて 262 全緑」）。
    /// **⚠ そして分岐の外側にあることを、`reasoning_effort` と同じ形で見る** ——
    /// ストリーミングの別・クライアント指定の有無・メッセージの有無で変わらないこと。
    #[test]
    fn wire_request_always_sends_the_explicit_samplers() {
        let cases = [
            ("既定", ChatRequest::new(vec![ChatMessage::user("x")])),
            ("メッセージ無し", ChatRequest::new(Vec::new())),
            (
                "クライアントの指定あり",
                ChatRequest {
                    messages: vec![ChatMessage::system("ペルソナ"), ChatMessage::user("x")],
                    temperature: Some(0.5),
                    max_tokens: Some(512),
                },
            ),
        ];

        for (label, request) in &cases {
            for stream in [false, true] {
                // **線から読み直す。** `to_value` は `f32` を広げるので綴りを見られない。
                let body = wire_value(request, stream);
                for (field, expected) in [
                    ("top_k", json!(40)),
                    ("top_p", json!(0.95)),
                    ("min_p", json!(0.05)),
                    ("repeat_penalty", json!(1.1)),
                    ("presence_penalty", json!(0.0)),
                ] {
                    assert_eq!(
                        body[field], expected,
                        "{label} / stream={stream}: {field} を明示すること: {body}"
                    );
                }
            }
        }
    }

    /// **`presence_penalty` は、どんな要求にも `0` で明示して載る**
    /// （ADR-0038 §決定2 の 2026-11-01 追記 / 作業単位 A-14）。
    ///
    /// # なぜ要るか
    ///
    /// **上流によっては、要求に書いていない欄にモデルカードの既定を重ねる**
    /// （鍵で守られた中継の先の推論サーバは思考オフの要求に `presence_penalty 1.5` を重ねる。personacore の要求は
    /// その推論サーバには思考ありと読まれるので実際には掛かっていなかった —— `RequestPolicy::presence_penalty` の doc の訂正 / origin#78）。
    /// **llama-server の既定は 0**（`/slots` で確認。LM Studio は未確認）なので、0 を明示してもそちらは変わらない。
    ///
    /// # 「載っていること」と「値が 0 であること」を別々に見る
    ///
    /// **欄が落ちると、上流によっては 1.5 が戻る**（値の検査だけだと
    /// `Value::Null` との比較で落ちるので結局赤にはなるが、**何が壊れたかを言い分けたい**）。
    /// **値の検査は「`0` の整数」ではなく「`0.0` の浮動小数」と比べる** ——
    /// serde は `0.0_f32` を `0.0` と綴る（`the_wire_bytes_spell_the_request_exactly`）。
    ///
    /// **⚠ 期待値は [`RequestPolicy`] から作らない。** 手で書く。
    #[test]
    fn wire_request_always_sends_presence_penalty_zero() {
        let cases = [
            ("既定", ChatRequest::new(vec![ChatMessage::user("x")])),
            ("メッセージ無し", ChatRequest::new(Vec::new())),
            (
                "クライアントの指定あり",
                ChatRequest {
                    messages: vec![ChatMessage::system("ペルソナ"), ChatMessage::user("x")],
                    temperature: Some(0.5),
                    max_tokens: Some(512),
                },
            ),
        ];

        for (label, request) in &cases {
            for stream in [false, true] {
                let wire = to_wire(request, stream);
                let body = wire_value(request, stream);

                // 載っていること（欄の有無）。
                assert!(
                    body.get("presence_penalty").is_some(),
                    "{label} / stream={stream}: presence_penalty を欄ごと落とさないこと: {wire}"
                );
                // 値が 0 であること（読み直した値）。
                assert_eq!(
                    body["presence_penalty"],
                    json!(0.0),
                    "{label} / stream={stream}: presence_penalty は 0 であること: {wire}"
                );
                // 綴り（線に乗るバイト列）。値の直後が区切りであることまで見る。
                assert!(
                    wire.contains(r#""presence_penalty":0.0,"#)
                        || wire.contains(r#""presence_penalty":0.0}"#),
                    "{label} / stream={stream}: 線には 0.0 の綴りで乗ること: {wire}"
                );
            }
        }
    }

    /// **思考を切る欄は、どんな要求にも必ず載る**（ADR-0038 §決定3）。
    ///
    /// 思考モデルは既定で予算を全部思考に使い、`content` が 0 文字になる
    /// （personacore からは「応答が空でした」/ docs/archive/04-environment-lm-studio.md の実測）。
    /// **この欄はモデルにも経路にも依存しない**ので、`wire_request` の分岐の
    /// 外側に置いてある。ここで見ているのは「外側にあること」そのもの ——
    /// **ストリーミングの別・サンプラーの有無・メッセージの有無のどれを変えても
    /// 同じ値が載る**ことを、同じ1本で押さえる。
    #[test]
    fn wire_request_always_asks_for_no_reasoning() {
        let cases = [
            ("既定", ChatRequest::new(vec![ChatMessage::user("x")])),
            ("メッセージ無し", ChatRequest::new(Vec::new())),
            (
                "サンプラー指定あり",
                ChatRequest {
                    messages: vec![ChatMessage::system("ペルソナ"), ChatMessage::user("x")],
                    temperature: Some(0.5),
                    max_tokens: Some(512),
                },
            ),
        ];

        for (label, request) in &cases {
            for stream in [false, true] {
                let body = to_value(request, stream);
                assert_eq!(
                    body["reasoning_effort"],
                    json!("none"),
                    "{label} / stream={stream} で思考を切ること: {body}"
                );
            }
        }
    }

    /// `stream` は引数どおりに載る（中継と補完で取り違えると SSE が来ない）。
    #[test]
    fn wire_request_reflects_the_stream_flag() {
        let request = ChatRequest::new(vec![ChatMessage::user("x")]);
        assert_eq!(to_value(&request, true)["stream"], json!(true));
        assert_eq!(to_value(&request, false)["stream"], json!(false));
    }

    /// **使用量はストリーミングのときだけ要求する**（ADR-0013 第3層）。
    ///
    /// 送らないと、LM Studio は最終チャンクに `usage` を載せない（2026-08-15 実測）。
    /// つまり中継経路の答え合わせが**材料を一度も受け取れないまま黙り続ける。**
    ///
    /// 逆に非ストリーミングでは不要（`usage` は本文と一緒に返る）。
    /// **不要なものを送らない**——拒否する上流がありうる。
    #[test]
    fn wire_request_asks_for_usage_only_when_streaming() {
        let request = ChatRequest::new(vec![ChatMessage::user("x")]);

        assert_eq!(
            to_value(&request, true)["stream_options"],
            json!({"include_usage": true}),
            "ストリーミングでは使用量を要求すること"
        );
        assert!(
            to_value(&request, false).get("stream_options").is_none(),
            "非ストリーミングでは項目ごと送らないこと"
        );
    }

    /// 未指定のサンプラーは**キーごと落とす**（`null` を送らない）。
    ///
    /// **⚠ ここで言う「サンプラー」は [`ChatRequest`] の欄**——
    /// **クライアントの依頼から来る `temperature` / `max_tokens`** のことで、
    /// **personacore が必ず載せる欄**（[`RequestPolicy`]）**とは別**である。
    /// あちらは未指定という状態を持たない（`wire_request_always_sends_the_explicit_samplers`）。
    #[test]
    fn wire_request_omits_unset_samplers() {
        let request = ChatRequest::new(vec![ChatMessage::user("x")]);
        let body = to_value(&request, false);
        assert!(body.get("temperature").is_none(), "既定に任せること");
        assert!(body.get("max_tokens").is_none(), "既定に任せること");
    }

    /// 指定したサンプラーは載る。
    #[test]
    fn wire_request_includes_set_samplers() {
        let request = ChatRequest {
            messages: vec![ChatMessage::user("x")],
            // f32 → f64 の変換で誤差が出ない値を使う（0.8 は二進で割り切れない）。
            temperature: Some(0.5),
            max_tokens: Some(512),
        };
        let body = to_value(&request, true);
        assert_eq!(body["temperature"], json!(0.5));
        assert_eq!(body["max_tokens"], json!(512));
    }

    /// **裏方の温度は `0.8` で、線にもその綴りで乗る**（ADR-0038 §決定2 / 作業単位 A-7d）。
    ///
    /// # 期待値は定数から作らない
    ///
    /// 作ると**値を動かす変異が両側で同時に動いて緑になる**
    /// （CLAUDE.md「関数の外にある値で押さえる見張り番」）。**`0.8` を手で書く。**
    ///
    /// # なぜ綴りまで見るのか
    ///
    /// **上流が受け取る値が今日と同じであること**が、この決定の前提そのもの
    /// （C-13 の `/slots` は `0.800000011920929` ＝ **`0.8_f32` を `f64` へ広げた綴り**）。
    /// **`serde_json::to_value` を経由すると線に乗るのも `0.800000011920929` になる**ので、
    /// **値だけを読み直す検査ではその差が見えない**（A-7c が塞いだ穴と同じ形）。
    ///
    /// **⚠ ここが見ているのは「型と綴り」まで。**
    /// **どの経路がこの値を使うか**は、ここからは一切見えない
    /// （`server::extract` / `server::drift` の単体と
    /// `tests/server_stat_drift.rs` の `each_path_sends_its_own_temperature` が持つ）。
    #[test]
    fn the_sub_llm_temperature_is_zero_point_eight_on_the_wire() {
        assert_eq!(
            SUB_LLM_TEMPERATURE, 0.8,
            "裏方の温度は、今日の実効の既定そのものであること（C-13 / ADR-0038 §決定2）"
        );

        let request = ChatRequest {
            messages: vec![ChatMessage::user("x")],
            temperature: Some(SUB_LLM_TEMPERATURE),
            max_tokens: Some(320),
        };
        let wire = to_wire(&request, false);

        assert!(
            wire.ends_with(r#""temperature":0.8,"max_tokens":320}"#),
            "線には 0.8 の綴りで乗ること（to_value を経由すると 0.800000011920929 になる）: {wire}"
        );
    }

    /// メッセージが空でも組み立てられる（拒否は上流の仕事。ここで panic しない）。
    #[test]
    fn wire_request_accepts_an_empty_message_list() {
        let body = to_value(&ChatRequest::new(Vec::new()), false);
        assert_eq!(body["messages"], json!([]));
    }
}
