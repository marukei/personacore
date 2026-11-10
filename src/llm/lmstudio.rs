//! LM Studio（OpenAI 互換）クライアント。
//!
//! この層は**薄く保つ**（CLAUDE.md 3節）。やることは
//! 「設定どおりに投げる」「HTTP の失敗を型に落とす」「SSE をチャンク列に均す」の3つだけで、
//! 判断は持たせない。応答の解釈は `response`、送信本文の組立は `request` にある。
//!
//! # ストリーミング
//!
//! **チャンクを溜め込まない**（docs/02-architecture.md）。`stream` は本文の到着を待たず、
//! ヘッダとステータスを確認した時点で `ChatStream` を返す。受けたチャンクはそのまま流す。
//!
//! **本文全体の蓄積はこの層では行わず、呼び出し側に委ねる。** 理由:
//!
//! - ここで蓄積すると `Arc<Mutex<String>>` のような共有状態と「終わったら受け取る」経路が
//!   必要になり、この層に状態管理という判断が入り込む。
//! - `server` は「FE が切断したら途中まででも保存する」判断を持つ（docs/02-architecture.md）。
//!   蓄積を持つ側と保存を決める側は同じであるべきで、分かれていると
//!   **どちらが `raw_content` の権威か**が曖昧になる。
//! - 蓄積は `while let Some(chunk) = stream.next().await` の中で1行足すだけで済み、
//!   その場所は中継の処理と同じループなので**中継を止めない**ことが自明になる。

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use eventsource_stream::{Event, EventStreamError, Eventsource};
use futures_util::{Stream, StreamExt, stream};

use super::error::LlmError;
use super::request::{
    ApiKey, ChatRequest, HostLookup, HostResolver, KeyDestination, KeyOverPlainHttp, ModelId,
    Sampling, WireRequest, endpoint_for_log, error_for_log, judge_addresses, key_destination,
    redact_key, wire_request,
};
use super::response::{
    AnsweredBy, ChatChunk, Completion, is_done, parse_applied_template, parse_chunk,
    parse_completion, parse_effective_context_length, parse_loaded_context_length,
    parse_model_catalogue, parse_token_count,
};
use super::{ChatStream, LlmClient, LlmFuture, TokenCount};

/// LM Studio の既定のエンドポイント（docs/archive/04-environment-lm-studio.md）。**外部公開しない。**
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:1234";

/// 本文生成に使う既定モデル（docs/archive/04-environment-lm-studio.md の実測値）。
pub const DEFAULT_MAIN_MODEL: &str = "ministral-3-14b-instruct-2512";

/// 抽出・要約（裏方処理）に使う既定モデル。**本文と同じ綴り**（ADR-0021 §10）。
///
/// 以前は 3B 級を既定にしていたが、2026-08-21 の実測で**撤回した**。
/// 3B は 2.2 倍速い代わりに**主語を約 10/41 で間違え、形式も 41% 崩れる**
/// （14B は 0/65・0%）。**`memory` に更新も削除も無い**ので、
/// 遅さはその回だけの費用だが、間違った記憶は永久に残る。
/// 同じ綴りなら上流が 2 つ目のモデルを載せずに済む（12GB に 14B と 3B を同居させない）。
///
/// **定数は本文用と分けたまま残す。** `PERSONACORE_SUB_MODEL` で別のモデルへ振り分ける
/// 経路（`server::config`）は今も生きており、既定が一致しているのは
/// **今の実測がそう出た**からにすぎない。
pub const DEFAULT_SUB_MODEL: &str = "ministral-3-14b-instruct-2512";

/// OpenAI 互換の補完エンドポイントのパス。
const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";

/// **LM Studio 独自**のモデル一覧のパス。**1つ目の読み手。**
///
/// **LM Studio の** OpenAI 互換の `/v1/models` は `id` と `owned_by` しか返さず、
/// **コンテキスト長を返さない**（2026-08-14 実測 / ADR-0013）。
/// LM Studio に対してはここだけが実コンテキスト長の入手経路なので、独自 API を承知で叩く。
/// **持たない上流では 404 になるだけ**で、そのときは [`OPENAI_MODELS_PATH`] を試す。
///
/// **⚠ 「`/v1/models` は長さを返さない」は LM Studio についての事実**であって、
/// OpenAI 互換のすべてについての事実ではない（2026-09-04 に `llama-server` で実測 /
/// 作業単位 G-2）。**この一般化が、`-c` を何にしても 8192 を使い続ける原因だった。**
const NATIVE_MODELS_PATH: &str = "/api/v0/models";

/// **OpenAI 互換**のモデル一覧のパス。**2つ目の読み手**（作業単位 G-2）。
///
/// `llama-server` は `/api/v0/models` を持たない（404 / 2026-09-04 実測）が、
/// **答えられないのではなく綴りが違うだけ**で、こちらには
/// `data[].meta.n_ctx`（いま載っている窓）が載っている。
///
/// **⚠ 上流によっては 200 を返しながら何も答えない。**
/// LM Studio のこのパスは `id` の一覧だけ（`meta` を持たない）で、
/// **未知のパスにも 404 ではなく 200 ＋ エラー本文を返す**（G-1 実測）。
/// **ステータスで分岐しない**——判定は本文が読めたかどうかで行う。
const OPENAI_MODELS_PATH: &str = "/v1/models";

/// **llama.cpp の組み上げの口**（ADR-0052 §推奨3 / 作業単位 B-60a）。
///
/// チャットへ送るのと同じ本文を受け、チャットテンプレートを当てた文字列を
/// `{"prompt":"…"}` で返す（**llama.cpp の server README の記憶。この版では見ていない** ——
/// ADR-0052 §背景「上流が数える口を持っているか」）。
///
/// **⚠ 未確認: この口が `/v1/chat/completions` と同じテンプレート処理を通るか**
/// （`reasoning_effort` 等で出力が変わるか）。**確かめるのは M1**（封の数と
/// `usage.prompt_tokens` が等しいか）。
const APPLY_TEMPLATE_PATH: &str = "/apply-template";

/// **llama.cpp の数える口**（ADR-0052 §推奨3・4 / 作業単位 B-60a）。
///
/// `{"tokens":[…]}` を返す（記憶。**この版では見ていない**）。
/// **LM Studio は未知の口に 200 ＋ エラー本文を返す**ので、判定は形で行う
/// （`response::parse_token_count`）。
const TOKENIZE_PATH: &str = "/tokenize";

/// ログに出す応答本文の上限（文字数）。**エラーレスポンスには出さない。**
const ERROR_BODY_LOG_LIMIT: usize = 512;

/// 接続の既定上限。
///
/// 停止中の LM Studio はすぐ拒否されるので短くてよい。**それでも省略しない。**
/// 拒否ではなく無反応（経路が黙って捨てる）の場合、OS 任せだと数十秒待たされる
/// （Windows の既定は約21秒。**手動で実測した値**であり、
/// `connect_timeout_bounds_an_unresponsive_host` はこれを観測しない
/// — あちらは上限を設定した側の経過時間しか測らないため）。
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// 1回の読み取りが返るまでの既定上限（`reqwest` の `read_timeout`）。
///
/// **全体の上限ではなく「読めない時間」の上限。** ストリーミングに全体上限をかけると
/// 長い応答が途中で切られるので（`request_timeout` の項）、こちらで滞留を検出する。
///
/// **短くしすぎないこと。** `read_timeout` は本文のチャンク間だけでなく
/// **応答ヘッダの到着待ちにも効く**。JIT ロードが有効なので初回は 8.24GB のモデルの
/// ロード待ちがまるごとこの1回の読み取りに乗る（docs/archive/04-environment-lm-studio.md）。
/// ここを切り詰めると、**本文が1文字も出ないまま `Timeout` になる。**
///
/// 裏を返すと、この値は「トークン間の無通信」と「初回トークンまでの待ち」の
/// **両方を兼ねた一つの予算**であり、前者だけを厳しくすることはできない
/// （`reqwest` のこの上限はクライアント単位でしか設定できず、途中で緩められない）。
/// 分けるには層内に別の予算と別のエラー分類を持ち込むことになるため、
/// **この層は薄く保つ**方（CLAUDE.md 3節）を選んでいる。
///
/// **180秒は暫定値。** リポジトリ内に根拠となるロード時間の計測値は無い
/// （docs/04-environment.md にあるのはモデルサイズ 8.24GB だけ）。
/// CLAUDE.md 6節「メイン/サブモデルの常駐 vs JIT 切替は実測して決める」が
/// 決まった時点で、**この値も実測に合わせて見直すこと。**
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(180);

/// 非ストリーミング要求の既定の全体上限。**ストリーミングには適用しない。**
///
/// 「少しずつ延々と送られ続ける」を止めるための外側の歯止め。
/// `reqwest` はこの上限を**応答ヘッダ待ちだけでなく本文の読み取りにも引き継ぐ**ので、
/// `complete()` には性質の違う2つの上限が同時にかかる:
///
/// - 1回の読み取りが `idle_timeout` を超えて返らなければ打ち切り。
/// - 通信の有無に関わらず、要求開始からの経過が `request_timeout` を超えたら打ち切り。
///
/// **危険の向きを取り違えないこと。** この値を `DEFAULT_IDLE_TIMEOUT` より短くしても
/// 「全体上限が効かなくなる」のではない。**逆に全体上限が支配的になり、
/// 到達不能になるのは `idle_timeout` の方**（沈黙がそこまで伸びる前に全体上限が来る）。
/// 結果として `idle_timeout` に持たせた JIT ロード待ちの予算が丸ごと切り詰められ、
/// **本文が1文字も出ないまま `Timeout` になる。**
/// この関係は `defaults_keep_the_total_deadline_outside_the_read_budget` で固定する。
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// コンテキスト長の照会の既定の全体上限。**本文生成の上限とは別物。**
///
/// **短くてよいし、短くなければならない。** この照会は**起動時に走る**ので、
/// ここが `DEFAULT_REQUEST_TIMEOUT`（300秒）や `DEFAULT_IDLE_TIMEOUT`（180秒）を
/// 引き継ぐと、黙り込んだ上流1台でサーバが数分間起動しなくなる
/// （待ち受けもデータベース接続もこの照会の後ろにある）。
///
/// 照会先は一覧を返すだけでモデルのロードを伴わないため、JIT ロード待ちの
/// 予算（`DEFAULT_IDLE_TIMEOUT` の項）は要らない。**取れなくても既定値へ落ちるだけ。**
const DEFAULT_CONTEXT_TIMEOUT: Duration = Duration::from_secs(5);

/// **天井（`idle_timeout`）に足して全体上限を作る猶予**（作業単位 B-6）。
///
/// # なぜ差で導くのか（[`LmStudioConfig::with_idle_budget`] の導出）
///
/// 全体上限は「**少しずつ延々と送られ続ける**」を止めるための外側の歯止めで、
/// **天井そのものより一定量だけ広ければ役目を果たす。**
/// 差で導くと次の2つが同時に成り立つ:
///
/// * **`request >= idle` が構造的に保証される**（`saturating_add` は減らない）。
///   `DEFAULT_REQUEST_TIMEOUT` の項が言う「向きの取り違え」が**書けなくなる。**
/// * **既定の天井を入れると、出荷している全体上限がそのまま出る**（下の `const _`）。
///
/// **比で導く案（`idle × 5/3`）は採らなかった。** 天井を 1 秒にしたときの
/// 全体上限が 1.7 秒になり、**猶予という意味を失う**（歯止めが天井と実質同じになる）。
const REQUEST_TIMEOUT_MARGIN: Duration = Duration::from_secs(120);

/// **導出が既定を再現することを、ビルドで固定する。**
///
/// ここが破れると、**つまみを一度も触っていない利用者の全体上限が黙って変わる。**
/// 実行時の見張り番（`the_default_ceiling_reproduces_the_shipped_defaults`）と
/// 二重になっているが、**片方はビルドを止める**ので、テストを消しても残る。
const _: () = assert!(
    DEFAULT_IDLE_TIMEOUT.as_secs() + REQUEST_TIMEOUT_MARGIN.as_secs()
        == DEFAULT_REQUEST_TIMEOUT.as_secs(),
    "全体上限の導出（天井 ＋ 猶予）が、出荷している既定値と食い違っている"
);

/// **既定の照会・接続の上限は、既定の天井の半分より短い。**
///
/// [`LmStudioConfig::with_idle_budget`] は両者を `min(既定, 天井の半分)` で導くので、
/// **この関係が成り立っていれば、既定の天井では既定値がそのまま出る。**
const _: () = assert!(
    DEFAULT_CONTEXT_TIMEOUT.as_secs() * 2 <= DEFAULT_IDLE_TIMEOUT.as_secs()
        && DEFAULT_CONNECT_TIMEOUT.as_secs() * 2 <= DEFAULT_IDLE_TIMEOUT.as_secs(),
    "照会・接続の既定が天井の半分を超えている（既定の天井で導出が既定を再現しない）"
);

/// **⚠ 照会と接続の既定が同値である間、導出の2行は交換可能で観測できない**
/// （作業単位 B-6 / レビュー指摘 P-1）。
///
/// `LmStudioConfig::with_idle_budget` の
/// `context_timeout = DEFAULT_CONTEXT_TIMEOUT.min(half)` と
/// `connect_timeout = DEFAULT_CONNECT_TIMEOUT.min(half)` は、
/// **2つの既定がどちらも 5 秒なので文字どおり入れ替えても同じ結果**になる。
/// **どのテストも取り違えを表現できない**
/// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」の**定数版**）。
///
/// **いま実害はゼロ。** 危ないのは**片方の既定を変えた日**で、
/// そのとき黙って間違った側に付く。
/// **その日にビルドを止めて理由を言うのが、この覚え書きの役目である。**
///
/// **止まったらやること: 取り違えを赤にする見張り番を1本足してから、この `assert` を外す。**
const _: () = assert!(
    DEFAULT_CONTEXT_TIMEOUT.as_secs() == DEFAULT_CONNECT_TIMEOUT.as_secs(),
    "照会と接続の既定が分かれた。`with_idle_budget` の2行の取り違えが\
     観測できるようになったので、それを赤にする見張り番を1本足すこと"
);

/// 接続先とモデルの設定。
///
/// **エンドポイントもモデル識別子もハードコードしない。** キャラクターごとに
/// モデルを変える可能性があるため、モデルは `LmStudioClient::with_model` で
/// 差し替えられる（接続プールは共有される）。
#[derive(Debug, Clone)]
pub struct LmStudioConfig {
    /// `/v1/chat/completions` までを含む完全な URL。
    endpoint: String,
    /// **LM Studio 独自**の `/api/v0/models` までを含む完全な URL。
    models_endpoint: String,
    /// **OpenAI 互換**の `/v1/models` までを含む完全な URL（2つ目の読み手 / G-2）。
    compat_models_endpoint: String,
    /// **組み上げの口**（`/apply-template` / ADR-0052）。
    apply_template_endpoint: String,
    /// **数える口**（`/tokenize` / ADR-0052）。
    tokenize_endpoint: String,
    model: ModelId,
    connect_timeout: Duration,
    request_timeout: Duration,
    idle_timeout: Duration,
    context_timeout: Duration,
    /// **この上流への鍵**（ADR-0056 §決定1・2 / 作業単位 A-13b-1）。`None` なら何も付けない。
    ///
    /// **`Debug` は伏せる**（[`ApiKey`] の手書きの `Debug`）——この型は `Debug` を derive している。
    api_key: Option<ApiKey>,
    /// **鍵を平文の `http://` でローカルネットワークの外へ送る承認**（ADR-0056 §決定5 / 作業単位 A-13b-2）。
    allow_key_over_plain_http: bool,
    /// **名前解決の差し替え**（テストだけが入れる / [`HostResolver`]）。`None` は OS の名前解決。
    ///
    /// **使うのは鍵を付けた `http://` のホスト名の宛先だけ**（`LmStudioClient::new`）。
    host_resolver: Option<Arc<dyn HostResolver>>,
    /// **この上流の面に書いたサンプラの値**（ADR-0058 §決定1 / 作業単位 A-16）。既定は「何も書いていない」。
    ///
    /// **[`LmStudioClient::with_model`] の複製にも写る** —— `sub` 面の無いプロファイルの裏方は本文の
    /// クライアントの複製なので、本文の値で送る（ADR-0058 §決定1 の「`sub` 面が無ければ `main` の値」）。
    sampling: Sampling,
}

impl LmStudioConfig {
    /// ベース URL（例: `http://127.0.0.1:1234`）とモデルから設定を作る。
    ///
    /// 末尾の `/` は除いて `/v1/chat/completions` を足す。
    /// **ベース URL は設定由来の値**（LLM 出力でもクライアント入力でもない）だが、
    /// 綴り間違いが「接続できない」以外の形で表面化しないよう、ここで最低限を検証する。
    ///
    /// # 末尾が `/v1` の宛先は、受け付けたまま警告する（作業単位 B-68 / Issue origin#97）
    ///
    /// `http://127.0.0.1:46001/v1` は `…/v1/v1/chat/completions` に送ることになる。**止めない**
    /// （`upstreams.json` に既にそう書いた利用者の起動を壊さない。パスの前置きを本当に `/v1` で終える
    /// 上流も区別できない）。**警告を1行出すだけ**（[`endpoint_ends_with_v1`]）。画面は保存の前に止める。
    ///
    /// **ここで出す理由**——宛先は全部ここを通る（環境変数の本文・裏方、プロファイルの切り替えの両面。
    /// 検証が [`normalize_base`] の1箇所なのと同じ理由）。**`normalize_base` の受理集合は変えない。**
    pub fn new(base_url: &str, model: ModelId) -> Result<Self, EndpointError> {
        let base = normalize_base(base_url)?;
        if endpoint_ends_with_v1(&base) {
            tracing::warn!(
                endpoint = %endpoint_for_log(&base),
                "上流の宛先の末尾が /v1 です。personacore が /v1 を足すので /v1/v1/… に送ります\
                 （宛先はホストとポートまで書いてください）"
            );
        }
        Ok(Self {
            endpoint: format!("{base}{CHAT_COMPLETIONS_PATH}"),
            models_endpoint: format!("{base}{NATIVE_MODELS_PATH}"),
            compat_models_endpoint: format!("{base}{OPENAI_MODELS_PATH}"),
            apply_template_endpoint: format!("{base}{APPLY_TEMPLATE_PATH}"),
            tokenize_endpoint: format!("{base}{TOKENIZE_PATH}"),
            model,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            context_timeout: DEFAULT_CONTEXT_TIMEOUT,
            api_key: None,
            allow_key_over_plain_http: false,
            host_resolver: None,
            sampling: Sampling::default(),
        })
    }

    /// **この上流の面に書いたサンプラの値を決める**（ADR-0058 §決定1 / 作業単位 A-16）。
    ///
    /// **書いた欄だけが送る1組を置き換える**（`request::wire_request`）。[`Sampling::default`] は
    /// 「何も書いていない」で、送る本文は1バイトも変わらない。
    #[must_use]
    pub const fn with_sampling(mut self, sampling: Sampling) -> Self {
        self.sampling = sampling;
        self
    }

    /// **この上流の面に書いたサンプラの値。**
    #[must_use]
    pub const fn sampling(&self) -> Sampling {
        self.sampling
    }

    /// **鍵を平文の `http://` でローカルネットワークの外へ送る承認を決める**（ADR-0056 §決定5 /
    /// 作業単位 A-13b-2）。面の `allow_key_over_plain_http`。
    ///
    /// **効くのは鍵を付けた `http://` の宛先で、ローカルネットワークの外が混ざるときだけ**
    /// （[`LmStudioClient::key_over_plain_http`]）。鍵の無い面・`https://`・ループバックには効かない。
    #[must_use]
    pub const fn with_key_over_plain_http_allowed(mut self, allowed: bool) -> Self {
        self.allow_key_over_plain_http = allowed;
        self
    }

    /// **鍵を平文の `http://` でローカルネットワークの外へ送る承認があるか。**
    #[must_use]
    pub const fn key_over_plain_http_allowed(&self) -> bool {
        self.allow_key_over_plain_http
    }

    /// **名前解決を差し替える**（テスト専用 / 作業単位 A-13b-2）。
    ///
    /// **実際の DNS に依らずに**「全部ローカル」「1つだけ外が混ざる」「解決先が後から外へ変わる」を作る。
    /// 使われるのは鍵を付けた `http://` のホスト名の宛先だけ。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn with_host_resolver(mut self, resolver: Arc<dyn HostResolver>) -> Self {
        self.host_resolver = Some(resolver);
        self
    }

    /// **名前解決の差し替えを `like` から写す**（走ったままの差し替え / `server::config::retarget_llm`）。
    ///
    /// 待ち時間と同じく、**宛先ではなく「どう繋ぐか」の設定**なので、切り替えても引き継ぐ。
    /// 本番では `like` も `None`（OS の名前解決）なので、何も変わらない。
    #[must_use]
    pub fn with_host_resolver_of(mut self, like: &Self) -> Self {
        self.host_resolver.clone_from(&like.host_resolver);
        self
    }

    /// **この上流への鍵を決める**（ADR-0056 §決定2）。`None` は「付けない」。
    ///
    /// **3つの送信（照会・本文・数える）の全部に `Authorization: Bearer <鍵>` が付く**
    /// （[`LmStudioClient`] の `authorized`）。**鍵は宛先に付く**ので、本文と裏方が別の上流なら
    /// 別々に決める（ADR-0056 §決定1）。
    #[must_use]
    pub fn with_api_key(mut self, api_key: Option<ApiKey>) -> Self {
        self.api_key = api_key;
        self
    }

    /// **この上流への鍵。** 無ければ `None`。
    #[must_use]
    pub const fn api_key(&self) -> Option<&ApiKey> {
        self.api_key.as_ref()
    }

    /// 接続確立までの上限を変える。**確立後の待ちには効かない。**
    #[must_use]
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// 非ストリーミング要求の全体上限を変える。**ストリーミングには適用されない。**
    ///
    /// この上限は本文の読み取りにも引き継がれる。`idle_timeout` を下回らせると
    /// **到達不能になるのは `idle_timeout` の方**なので、下回らせないこと
    /// （`DEFAULT_REQUEST_TIMEOUT` の項）。
    #[must_use]
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// 1回の読み取りが返るまでの上限を変える（滞留の検出）。
    ///
    /// **応答ヘッダの到着待ちにも効く**ので、`complete()` の実効上限でもあり、
    /// `stream()` の初回トークンまでの上限でもある（`DEFAULT_IDLE_TIMEOUT` の項）。
    #[must_use]
    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// コンテキスト長の照会の全体上限を変える。**起動を止めないための上限。**
    #[must_use]
    pub fn with_context_timeout(mut self, timeout: Duration) -> Self {
        self.context_timeout = timeout;
        self
    }

    /// **打ち切りの天井を1つ決めて、残りを導く**（ADR-0035 §決定3 / 作業単位 B-6）。
    ///
    /// # つまみは1つだけ（ADR-0036 §決定3）
    ///
    /// 設定可能にしてよいのは「**根拠が無い・外すと破壊的・導出も観測もできない**」の
    /// 3つが揃った値だけで、**つまみ1つにつき `App::bind` を通る見張り番が1本要る。**
    /// だから外へ出すのは天井（`idle_timeout`）だけにして、
    /// 残りの3つは**ここで導く。**
    ///
    /// | 導く先 | 式 | なぜ |
    /// |---|---|---|
    /// | `request_timeout` | 天井 ＋ [`REQUEST_TIMEOUT_MARGIN`]（飽和加算） | **`request >= idle` を構造的に保つ**（`DEFAULT_REQUEST_TIMEOUT` の項の「向き」） |
    /// | `context_timeout` | `min(既定, 天井 / 2)` | **起動を止めない**上に、**天井より必ず短い** |
    /// | `connect_timeout` | `min(既定, 天井 / 2)` | 同上（接続の失敗は即座に分かる） |
    ///
    /// **`/ 2` は「必ず真に短くする」ための最小の仕掛け。**
    /// `min(既定, 天井)` だと天井が 5 秒ちょうどのときに**等しくなり**、
    /// 「照会の上限は天井より短い」という関係が破れる。
    ///
    /// # ⚠ `timeout` は 0 にしないこと
    ///
    /// 0 を渡すと導出した照会・接続も 0 になり、**すべての要求が即座に失敗する。**
    /// 呼び出し側（`server::config`）が**起動を止める**ので、ここへは届かない。
    /// **この型は「使える値かどうか」を判断しない**——
    /// テストが極端な値を通すための入口でもある。
    #[must_use]
    pub fn with_idle_budget(mut self, timeout: Duration) -> Self {
        // **天井の半分。** `min` と組み合わせて「必ず天井より短い」を作る。
        let half = timeout / 2;

        self.idle_timeout = timeout;
        // **減らない加算。** 天井が極端に大きくても `request < idle` にならない。
        self.request_timeout = timeout.saturating_add(REQUEST_TIMEOUT_MARGIN);
        self.context_timeout = DEFAULT_CONTEXT_TIMEOUT.min(half);
        self.connect_timeout = DEFAULT_CONNECT_TIMEOUT.min(half);
        self
    }

    /// 実際に叩く URL。
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// コンテキスト長を照会する URL（**LM Studio 独自** / 1つ目の読み手）。
    pub fn models_endpoint(&self) -> &str {
        &self.models_endpoint
    }

    /// コンテキスト長を照会する URL（**OpenAI 互換** / 2つ目の読み手 / G-2）。
    pub fn compat_models_endpoint(&self) -> &str {
        &self.compat_models_endpoint
    }

    /// 使うモデル識別子。
    pub const fn model(&self) -> &ModelId {
        &self.model
    }

    /// 接続確立までの上限。
    #[must_use]
    pub const fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    /// 非ストリーミング要求の全体上限。**ストリーミングには適用されない。**
    #[must_use]
    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// **1回の読み取りが返るまでの上限**（＝打ち切りの天井 / ADR-0035 §決定3）。
    #[must_use]
    pub const fn idle_timeout(&self) -> Duration {
        self.idle_timeout
    }

    /// コンテキスト長の照会の全体上限。**起動を止めないための上限。**
    #[must_use]
    pub const fn context_timeout(&self) -> Duration {
        self.context_timeout
    }
}

/// ベース URL が受け付けられない理由。`Display` は定数文字列（入力を反射しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    /// 空文字・空白のみ。
    #[error("エンドポイントが空です")]
    Empty,

    /// `http` / `https` 以外。
    #[error("エンドポイントのスキームが不正です")]
    UnsupportedScheme,

    /// ホスト名が無い（`http://` だけ等）。
    #[error("エンドポイントにホストがありません")]
    MissingHost,

    /// 空白・制御文字を含む。
    #[error("エンドポイントに空白または制御文字は使えません")]
    Malformed,
}

/// ベース URL を検証し、末尾の `/` を落とす。**純粋関数。**
///
/// パスを足すのは呼び出し側（`LmStudioConfig::new` が補完と独自 API の2本を組み立てる）。
/// **検証はここ1箇所**なので、エンドポイントを増やしても検証の抜けが生じない。
fn normalize_base(base_url: &str) -> Result<String, EndpointError> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(EndpointError::Empty);
    }
    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(EndpointError::Malformed);
    }

    let rest = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .ok_or(EndpointError::UnsupportedScheme)?;
    if rest.is_empty() || rest.starts_with('/') {
        return Err(EndpointError::MissingHost);
    }

    Ok(trimmed.trim_end_matches('/').to_owned())
}

/// **宛先のパス（末尾の `/` を除く）の最後の区切りが、ちょうど `v1` か。純粋関数**（作業単位 B-68）。
///
/// 大文字小文字は問わない（`/V1`）。**`/v1beta`・`/api/v1x`・`/v1/api` は止めない**（最後の区切りが
/// `v1` そのものではない）。パスが無い宛先・URL として読めない綴りは `false`。
///
/// **WHATWG の URL として読む**（送るときと同じ `reqwest::Url`）—— `\` を `/` と読み、`.` と `..` を畳む。
/// 画面の写し（`web/src/lib/upstreamProfiles.ts` の `endpointEndsWithV1`）も同じ読み方をし、
/// **両側が同じ表**（`tests/fixtures/endpoint_v1_suffix_corpus.tsv`）**で見張られる。**
#[must_use]
pub fn endpoint_ends_with_v1(endpoint: &str) -> bool {
    reqwest::Url::parse(endpoint.trim()).is_ok_and(|url| {
        url.path()
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .is_some_and(|last| last.eq_ignore_ascii_case("v1"))
    })
}

/// **モデル一覧の照会1回ぶんの読み取り**（作業単位 E-2）。
///
/// `/api/v0/models` の同じ本文から2つを取り出す。**往復は1回。**
///
/// # 2つは失敗の仕方が違う
///
/// * [`context_length`](Self::context_length) は**設定中のモデル1件**についての読み取りなので、
///   一覧に居ない・未ロード・値が不正のいずれでも `Err`。
/// * [`catalogue`](Self::catalogue) は**一覧そのもの**なので、読めなければ空になるだけ。
///
/// **だから片方が `Err` でも、もう片方は使える**——
/// 「設定中のモデルが未ロード」のとき、一覧は取れている。
#[derive(Debug)]
pub struct ModelsReport {
    /// 設定中のモデルにロードされているコンテキスト長（ADR-0013 第2層）。
    context_length: Result<u32, LlmError>,
    /// **上流が持っているモデルの綴り**（`state` で絞らない）。
    catalogue: Vec<String>,
    /// **どの読み手が答えたか**（作業単位 G-3b）。
    answered_by: AnsweredBy,
}

impl ModelsReport {
    /// 設定中のモデルのコンテキスト長。
    pub fn context_length(self) -> Result<u32, LlmError> {
        self.context_length
    }

    /// **上流が持っているモデルの綴り。** 読めなければ空。
    ///
    /// **依頼した綴りがここに無ければ、別のモデルが黙って答える**
    /// （`crate::measured::check`）。
    pub fn catalogue(&self) -> &[String] {
        &self.catalogue
    }

    /// **どの読み手が答えたか**（作業単位 G-3b）。
    ///
    /// **往復が成功した回にしか意味を持たない。** 「答えなし」は
    /// `models()` が `Err` を返すことで表す（この型は作られない）。
    pub const fn answered_by(&self) -> AnsweredBy {
        self.answered_by
    }

    /// 3つに割る（呼び出し側が全部を使う場合）。
    ///
    /// **`loaded_context_length()` の代わりにこれを使うこと**——
    /// あちらは**同じ往復の一覧と読み手を捨てる。**
    pub fn into_parts(self) -> (Result<u32, LlmError>, Vec<String>, AnsweredBy) {
        (self.context_length, self.catalogue, self.answered_by)
    }
}

/// **読み手1つぶんの失敗**（作業単位 G-5）。**ここでは深刻度を決めない。**
///
/// # なぜ失敗そのものではなく、この器で返すのか
///
/// **1つの読み手が失敗したことは、それ自体では良いことでも悪いことでもない。**
/// `llama-server` を上流にすると1つ目は**必ず** 404 になり、
/// **そのあと2つ目が成功する**（G-2 が実測 / 2026-09-04）。
/// **深刻度を決められるのは「どちらの読み手も何も返さなかったか」を知っている側**
/// ——[`LmStudioClient::models`] だけである。
///
/// だから [`LmStudioClient::fetch_models`] は**事実を集めて返すだけ**にし、
/// 応答本文（調査の手掛かり）も一緒に持ち上げる。
/// **本文を捨てると、本物の問題のときに `warn` へ載せられない。**
///
/// **⚠ 本文はログにだけ出す。** エラーレスポンスには載せない（CLAUDE.md）。
#[derive(Debug)]
struct ModelsFetchError {
    /// 失敗そのもの。
    error: LlmError,
    /// エラー応答の本文の先頭。**ステータス異常のときだけ在る。**
    snippet: Option<String>,
}

/// **2つ目の読み手が、どう終わったか**（作業単位 G-5 の是正）。
///
/// # なぜ1語を別に持つのか
///
/// [`LmStudioClient::read_compat_models`] が返す誤りは
/// **1つ目の [`LlmError::Status`]**（調査先を1つ目に保つため / そちらの doc）なので、
/// **2つ目がどう失敗したかは、そのままでは1文字も残らない。**
///
/// **404 か・5xx か・タイムアウトか・「答えたが中身が無い」かで、
/// 読み手が調べに行く先がまるで違う。**
/// 警告の行の値打ちは「止めること」だけでなく**「どこを直せと言うか」**でもある
/// （CLAUDE.md / 作業単位 B-38）。**その欄がこれ。**
///
/// 見張り番は [`tests::neither_reader_answering_is_warned_with_both_paths_on_one_line`]
/// （**腕を2本回して、定数に潰れていないことまで見る**）。
///
/// [`tests::neither_reader_answering_is_warned_with_both_paths_on_one_line`]:
///     tests::neither_reader_answering_is_warned_with_both_paths_on_one_line
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompatOutcome {
    /// 2xx 以外が返った。**その綴りは在るが、断られた。**
    Status(u16),
    /// 接続できなかった。**上流が停止している**（docs/04-environment.md）。
    Unreachable,
    /// 制限時間内に完了しなかった。
    Timeout,
    /// 接続後の送受信に失敗した。
    Transport,
    /// **2xx なのに、長さも綴りの一覧も取れなかった。** 上流は答えたが中身が無い。
    Unusable,
    /// **送らなかった**（鍵を平文で外へ送る承認が無い / 作業単位 A-13b-2）。上流は何も見ていない。
    KeyNotApproved,
}

impl CompatOutcome {
    /// 読み手1つぶんの失敗から、1語を決める。
    ///
    /// **`_` で畳まないこと。** [`LlmError`] にバリアントが増えたら
    /// ここをコンパイルエラーにして、**畳んでよいかを人に決めさせる**
    /// （CLAUDE.md「実測由来の値を、外れたら壊れる場所に置かない」と同じ形で、
    /// 分類の決定を黙って既定へ流さない）。
    ///
    /// # 到達する腕と、しない腕（**混ぜて数えないこと**）
    ///
    /// [`LmStudioClient::fetch_models`] が作るのは [`LlmError::Status`] と
    /// [`map_request_error`] の3つ（`Connect` / `Timeout` / `Transport`）**だけ**である。
    /// **作業単位 A-13b-2 で [`LlmError::KeyNotApproved`] が5つ目に加わった**（鍵を平文で外へ送る
    /// 承認が無い面。見張り番は `tests/llm_plain_http_key.rs` —— 下の表には入れていない）。
    ///
    /// **⚠ 下の 6 つ**（`Decode` 以下）**はここへ来ない。**
    /// **到達しないので、そこに見張り番を置いても永久に緑になる。**
    /// **「守られている」と数えないこと。**
    ///
    /// **⚠ 到達する4つのほうは、4つとも見張ってある**（作業単位 G-5 の是正 / 2026-09-06）。
    /// **是正前は `Status` の1つだけで、残り3つは値を潰す変異が 1945 全緑**だった
    /// ——**そして上の断り書きは「6つは到達しない」としか言っていなかったので、
    /// 読み手は「残りは守られている」と読む。**
    ///
    /// | 腕 | 見張り番 | 上流の作り方 |
    /// |---|---|---|
    /// | `Status` | [`tests::neither_reader_answering_is_warned_with_both_paths_on_one_line`] | 2つ目も 404／2つ目が 500 |
    /// | `Timeout` | 同上 | 2つ目を [`LmStudioConfig::with_context_timeout`] より遅くする |
    /// | `Connect` | [`tests::a_dying_upstream_is_named_for_what_it_did`] | 1つ目に答えた直後に**待ち受けごと畳む** |
    /// | `Transport` | 同上 | `Content-Length` より**短い本文**で切る |
    ///
    /// # ⚠ 「腕が見張ってある」は「値が見張ってある」ではない（4周目の是正 / 2026-09-06）
    ///
    /// **上の表は、3周目まで「腕」しか言っていなかった。**
    /// `Status` の腕を通す上流が **404 の1本きり**だったので、
    /// **`Self::Status(*status)` を `Self::Status(404)` に決め打つ変異が 1949 全緑**だった。
    ///
    /// **実害は「警報は出ており、指示だけが誤っている」形**（CLAUDE.md / 作業単位 B-38）——
    /// **2つ目が 500 を返した回まで `compat_outcome=status_404` と名乗る。**
    /// この欄の存在理由は
    /// **「読み手が調べに行く先がまるで違う」**（[`LmStudioClient::read_compat_models`] の doc）
    /// なので、**`status_404` を読んだ人は「綴りが違う・エンドポイントが無い」を疑いに行く。
    /// 実際は「上流が壊れている」。**
    ///
    /// **だから `Status` の腕は 404 と 500 の2本で回す。**
    /// **1本に戻さないこと**——戻した瞬間、値は再び決め打ちに潰せる。
    ///
    /// 下2つは wiremock では作れないので、**生の TCP で筋書きを演じる上流**を使う
    /// （[`tests::WireScript`]）。
    ///
    /// **`_` で畳まないこと**（上の項）——畳むと、この表の「到達する腕」が
    /// **増えたことに誰も気付けなくなる。**
    ///
    /// [`tests::neither_reader_answering_is_warned_with_both_paths_on_one_line`]:
    ///     tests::neither_reader_answering_is_warned_with_both_paths_on_one_line
    /// [`tests::a_dying_upstream_is_named_for_what_it_did`]:
    ///     tests::a_dying_upstream_is_named_for_what_it_did
    /// [`tests::WireScript`]: tests::WireScript
    fn from_error(error: &LlmError) -> Self {
        match error {
            LlmError::Status { status } => Self::Status(*status),
            LlmError::Connect(_) => Self::Unreachable,
            LlmError::Timeout(_) => Self::Timeout,
            // **到達する**（作業単位 A-13b-2）—— 照会も [`LmStudioClient::authorized`] と名前解決を通る。
            LlmError::KeyNotApproved => Self::KeyNotApproved,
            LlmError::Transport(_)
            | LlmError::Decode(_)
            | LlmError::MalformedEvent
            | LlmError::EmptyPayload
            | LlmError::MissingContent
            | LlmError::MissingContextLength
            | LlmError::Truncated => Self::Transport,
        }
    }
}

/// **どちらの読み手も読めなかった回に返すステータス。純粋関数**（作業単位 A-13b-1）。
///
/// **原則は1つ目のステータス**（[`LmStudioClient::read_compat_models`] の doc —— 調査先を1つ目に保つ）。
/// **例外は「2つ目だけが鍵を断った」とき**で、そのときは2つ目のステータスを返す。
///
/// 鍵で守られた中継は `/v1/*` を全部 401 にする（origin#77）。1つ目（`/api/v0/models`）は
/// その上流に無い綴りなので 404 になりうる。**1つ目を返すと、上流の画面が
/// 「届かない」と「鍵を断られた」を見分けられない**（ADR-0056 §決定4）。
/// 見張り番は `tests/llm_api_key.rs` の `a_refusal_on_either_list_reader_reads_as_a_refused_key`。
fn refusal_first(native_status: u16, compat_outcome: CompatOutcome) -> u16 {
    let native = LlmError::Status {
        status: native_status,
    };
    match compat_outcome {
        CompatOutcome::Status(status)
            if !native.refused_the_key() && LlmError::Status { status }.refused_the_key() =>
        {
            status
        }
        _ => native_status,
    }
}

impl fmt::Display for CompatOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // **`=` を値に入れないこと。** ログの欄は `キー=値` で読まれるので、
            // 区切りと衝突して `has_field` が当たらなくなる。
            Self::Status(status) => write!(f, "status_{status}"),
            Self::Unreachable => f.write_str("unreachable"),
            Self::Timeout => f.write_str("timeout"),
            Self::Transport => f.write_str("transport"),
            Self::Unusable => f.write_str("unusable"),
            Self::KeyNotApproved => f.write_str("key_not_approved"),
        }
    }
}

/// LM Studio へ話すクライアント。
///
/// `reqwest::Client` は内部で `Arc` を持つので、`with_model` の複製は接続プールを共有する。
#[derive(Debug, Clone)]
pub struct LmStudioClient {
    http: reqwest::Client,
    config: LmStudioConfig,
    /// **鍵を平文の `http://` で別の機へ送る扱い**（[`KeyGuard`] / 作業単位 A-13b-2）。
    /// `with_model` の複製と共有する（同じ宛先・同じ接続プール・同じ名前解決）。
    guard: Arc<KeyGuard>,
}

// ----------------------------------------------------------------------
// 鍵を平文の `http://` で別の機へ送るとき（ADR-0056 §決定5 / 作業単位 A-13b-2）
// ----------------------------------------------------------------------

/// **1つのクライアントの、鍵を平文で別の機へ送る扱い。**
///
/// | 宛先（[`key_destination`]） | いつ判定するか | 承認が無くて外なら |
/// |---|---|---|
/// | 判定しない（鍵が無い・`https://`・ループバック） | —— | —— |
/// | IP アドレス | **組んだとき**（`dns_resolver` を通らないので、変わらない） | 送信の前に [`LlmError::KeyNotApproved`]（[`LmStudioClient::authorized`]） |
/// | ホスト名 | **接続のたびの名前解決**（[`GuardedResolver`]） | 名前解決が [`KeyNotApprovedMarker`] で失敗し、接続しない |
///
/// **最後の判定**（[`Self::last`]）が `GET /admin/upstream` の面に出る（`server::upstream`）。
/// **判定が変わったときだけ `warn` を1本出す**（接続のたびに出すと、同じ事実が何本も並ぶ）。
#[derive(Debug)]
struct KeyGuard {
    /// **IP アドレスの宛先の判定**（組んだときに決まる）。ホスト名・判定しない宛先は `None`。
    fixed: Option<KeyOverPlainHttp>,
    /// **最後の判定。** IP アドレスの宛先は組んだときの判定、ホスト名は最後の名前解決の判定。
    last: Mutex<Option<KeyOverPlainHttp>>,
    /// ログに出す宛先のホスト（**ユーザー情報を含まない** —— `reqwest::Url::host_str`）。
    host: String,
}

impl KeyGuard {
    /// **判定しないクライアント**（鍵が無い・`https://`・ループバック）。
    fn unguarded() -> Self {
        Self {
            fixed: None,
            last: Mutex::new(None),
            host: String::new(),
        }
    }

    /// **判定を記録する。変わったときだけ `warn` を出す。**
    ///
    /// 錠が毒されていても中身を使う（中身は `Option` の置き換えだけで壊れない）。
    fn record(&self, verdict: Option<KeyOverPlainHttp>, addresses: &[IpAddr]) {
        let changed = {
            let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            let changed = *last != verdict;
            *last = verdict;
            changed
        };
        if !changed {
            return;
        }
        let Some(verdict) = verdict else {
            return;
        };
        let host = self.host.as_str();
        let label = verdict.label();
        match verdict {
            KeyOverPlainHttp::LocalNetwork => tracing::warn!(
                upstream_host = %host,
                ?addresses,
                key_over_plain_http = label,
                "鍵を暗号化せずに（平文の http:// で）ローカルネットワークの機へ送ります"
            ),
            KeyOverPlainHttp::Approved => tracing::warn!(
                upstream_host = %host,
                ?addresses,
                key_over_plain_http = label,
                "鍵を暗号化せずに（平文の http:// で）ローカルネットワークの外へ送ります（面に承認があります）"
            ),
            KeyOverPlainHttp::NotApproved => tracing::warn!(
                upstream_host = %host,
                ?addresses,
                key_over_plain_http = label,
                "宛先にローカルネットワークの外が混ざるので、鍵を送りません（平文の http:// で送る承認が面にありません）"
            ),
        }
    }

    /// **最後の判定。**
    fn last(&self) -> Option<KeyOverPlainHttp> {
        *self.last.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// **接続する前に止めるか**（IP アドレスの宛先で、承認が無くて外）。
    fn refuses_before_connecting(&self) -> bool {
        self.fixed == Some(KeyOverPlainHttp::NotApproved)
    }
}

/// **名前解決の失敗として返す印**（承認が無くて外が混ざった / 作業単位 A-13b-2）。
///
/// `reqwest` は名前解決の失敗を接続の失敗（`is_connect`）に包む。**包まれた中からこの型を探して**
/// [`LlmError::KeyNotApproved`] へ戻す（[`map_request_error`]）。`Display` は定数。
#[derive(Debug, thiserror::Error)]
#[error("鍵を平文でローカルネットワークの外へ送る承認がありません")]
struct KeyNotApprovedMarker;

/// **その失敗が、承認の無い送信を止めたものか**（`reqwest` が包んだ中を辿る）。
fn carries_key_not_approved(err: &reqwest::Error) -> bool {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(layer) = current {
        if layer.is::<KeyNotApprovedMarker>() {
            return true;
        }
        current = layer.source();
    }
    false
}

/// **OS の名前解決**（本番の [`HostResolver`]）。`getaddrinfo` の並びをそのまま返す。
#[derive(Debug)]
struct SystemResolver;

impl HostResolver for SystemResolver {
    fn lookup(&self, host: &str) -> HostLookup {
        let host = host.to_owned();
        Box::pin(async move {
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            Ok(found.map(|address| address.ip()).collect())
        })
    }
}

/// **接続のたびの名前解決で判定する口**（`reqwest` の `dns_resolver` / ADR-0056 §決定5）。
///
/// **`reqwest` は新しい接続を張るたびにここを呼ぶ**（接続プールの使い回しでは呼ばない ——
/// 使い回すのは、前に判定を通った接続である）。**IP アドレスの宛先では呼ばれない**
/// （そちらは [`KeyGuard::fixed`]）。
#[derive(Debug)]
struct GuardedResolver {
    inner: Arc<dyn HostResolver>,
    approved: bool,
    guard: Arc<KeyGuard>,
}

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let lookup = self.inner.lookup(name.as_str());
        let approved = self.approved;
        let guard = Arc::clone(&self.guard);
        Box::pin(async move {
            let addresses = lookup.await?;
            let verdict = judge_addresses(addresses.iter().copied(), approved);
            guard.record(verdict, &addresses);
            if verdict == Some(KeyOverPlainHttp::NotApproved) {
                return Err(
                    Box::new(KeyNotApprovedMarker) as Box<dyn std::error::Error + Send + Sync>
                );
            }
            // **ポートは 0**（宛先の URL のポートが上書きする / `reqwest::dns::Resolve` の doc）。
            let found: reqwest::dns::Addrs = Box::new(
                addresses
                    .into_iter()
                    .map(|address| SocketAddr::new(address, 0)),
            );
            Ok(found)
        })
    }
}

impl LmStudioClient {
    /// 設定から構築する。
    ///
    /// **全体タイムアウトはクライアントに設定しない。** ストリーミングの応答は
    /// 本文が終わるまで数分かかりうるため、滞留の検出は `idle_timeout` に任せ、
    /// 全体上限は非ストリーミング要求にだけ個別に付ける
    /// （`streaming_ignores_the_total_request_timeout` が固定している）。
    ///
    /// **`read_timeout` を外さないこと。** 上流が接続を開いたまま黙り込んだとき、
    /// これが無いと `stream()` の `next()` は永久に返らず、下流の FE も axum の
    /// ハンドラも宙吊りになる（docs/02-architecture.md）。ストリーミングには
    /// 全体上限を付けない方針なので、**これが唯一の防波堤**
    /// （`stalled_stream_is_cut_by_the_idle_timeout` が固定している）。
    ///
    /// # 鍵を平文の `http://` で別の機へ送るとき（ADR-0056 §決定5 / 作業単位 A-13b-2）
    ///
    /// **鍵を付けた `http://` の宛先がループバックでなければ、ここで判定の仕掛けを組む**（[`KeyGuard`]）。
    ///
    /// | 宛先 | 組むもの |
    /// |---|---|
    /// | IP アドレス | その場で判定する（変わらない） |
    /// | ホスト名 | **`dns_resolver` を差し替える**（[`GuardedResolver`] —— 接続のたびに判定） |
    ///
    /// **どちらもプロキシを使わない**（`no_proxy`）。プロキシを通すと、`reqwest` が名前解決するのは
    /// プロキシのホストで、鍵はプロキシから先へ平文で流れる —— 判定した先と、鍵が届く先が割れる。
    /// **鍵の無い面・`https://`・ループバックは今までどおり**（プロキシの扱いも変えない）。
    /// 見張り番は `tests/llm_plain_http_key_proxy.rs`（環境変数を書き換えるので、テスト1本だけの専用のバイナリ）。
    pub fn new(config: LmStudioConfig) -> Result<Self, LlmError> {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.idle_timeout);

        let destination = match config.api_key {
            Some(_) => key_destination(&config.endpoint),
            None => KeyDestination::Unguarded,
        };
        let approved = config.allow_key_over_plain_http;
        let guard = match destination {
            KeyDestination::Unguarded => Arc::new(KeyGuard::unguarded()),
            KeyDestination::Address(address) => {
                let verdict = judge_addresses([address], approved);
                let guard = Arc::new(KeyGuard {
                    fixed: verdict,
                    last: Mutex::new(None),
                    host: address.to_string(),
                });
                guard.record(verdict, &[address]);
                builder = builder.no_proxy();
                guard
            }
            KeyDestination::Name(host) => {
                let guard = Arc::new(KeyGuard {
                    fixed: None,
                    last: Mutex::new(None),
                    host,
                });
                let inner = config
                    .host_resolver
                    .clone()
                    .unwrap_or_else(|| Arc::new(SystemResolver));
                builder = builder
                    .dns_resolver(GuardedResolver {
                        inner,
                        approved,
                        guard: Arc::clone(&guard),
                    })
                    .no_proxy();
                guard
            }
        };

        let http = builder.build().map_err(|err| {
            tracing::error!(error = %err, "HTTP クライアントを構築できませんでした");
            LlmError::Transport(err)
        })?;

        Ok(Self {
            http,
            config,
            guard,
        })
    }

    /// **鍵を平文の `http://` で別の機へ送る扱いの、最後の判定**（ADR-0056 §決定5 / 作業単位 A-13b-2）。
    ///
    /// | 返り値 | 意味 |
    /// |---|---|
    /// | `None` | 判定しない宛先（鍵が無い・`https://`・ループバック・全部ループバックに解決した）か、ホスト名をまだ解決していない |
    /// | `Some(_)` | IP アドレスの宛先なら組んだときの判定、ホスト名なら最後の名前解決の判定 |
    ///
    /// **`GET /admin/upstream` の面に出る**（`server::context::probe_upstream` が照会のあとに読む）。
    #[must_use]
    pub fn key_over_plain_http(&self) -> Option<KeyOverPlainHttp> {
        self.guard.last()
    }

    /// モデルだけを差し替えた複製を返す（本文生成と裏方処理の切り替え）。
    ///
    /// LM Studio は JIT ロードが有効なので、**識別子を変えるだけで切り替わる**
    /// （docs/archive/04-environment-lm-studio.md）。明示的なロード API は要らない。
    #[must_use]
    pub fn with_model(&self, model: ModelId) -> Self {
        Self {
            http: self.http.clone(),
            config: LmStudioConfig {
                model,
                ..self.config.clone()
            },
            // **同じ判定を共有する**（同じ宛先・同じ名前解決の口 —— `http` の複製が同じ口を使う）。
            guard: Arc::clone(&self.guard),
        }
    }

    /// 現在の設定。
    pub const fn config(&self) -> &LmStudioConfig {
        &self.config
    }

    /// **鍵があれば `Authorization: Bearer <鍵>` を付ける**（ADR-0056 §決定2）。
    ///
    /// **3つの送信（[`Self::fetch_models`]・[`Self::post`]・[`Self::post_for_count`]）が全部ここを通る。**
    /// 送信を足したら、ここを通すこと（通さないと、その口だけ鍵の無い要求になり 401 が返る）。
    /// ヘッダの値は `sensitive` が立っている（[`ApiKey`] が作った値をそのまま使う）。
    ///
    /// # 承認の無い送信はここで止める（IP アドレスの宛先 / 作業単位 A-13b-2）
    ///
    /// **宛先が IP アドレスで、ローカルネットワークの外で、承認が無ければ、要求を組まずに
    /// [`LlmError::KeyNotApproved`]**（接続しない）。ホスト名の宛先は、接続のたびの名前解決が止める
    /// （[`GuardedResolver`]）。**`voice` は失敗の行の深刻度**（[`map_request_error`] と同じ）。
    fn authorized(
        &self,
        builder: reqwest::RequestBuilder,
        voice: RequestFailureVoice,
    ) -> Result<reqwest::RequestBuilder, LlmError> {
        if self.guard.refuses_before_connecting() {
            return Err(key_not_approved(voice));
        }
        Ok(match &self.config.api_key {
            Some(key) => builder.header(reqwest::header::AUTHORIZATION, key.bearer().clone()),
            None => builder,
        })
    }

    /// **エラー応答の本文を、ログ用に取り出す**（鍵を伏せてから切り詰める / [`error_snippet`]）。
    async fn error_snippet(&self, response: reqwest::Response) -> String {
        error_snippet(response, self.config.api_key.as_ref()).await
    }

    /// 非ストリーミングの補完。
    pub async fn complete(&self, request: &ChatRequest) -> Result<Completion, LlmError> {
        let response = self.post(request, false).await?;
        let body = response
            .text()
            .await
            .map_err(|err| map_request_error(err, RequestFailureVoice::Speak))?;
        parse_completion(&body)
    }

    /// **設定中のモデルに実際にロードされているコンテキスト長**を照会する
    /// （ADR-0013 第2層）。
    ///
    /// # トレイトに載せない理由
    ///
    /// 照会先の `/api/v0/models` は **LM Studio 独自** で、OpenAI 互換の `/v1/models` は
    /// コンテキスト長を返さない。`LlmClient` に載せると
    /// **他の実装が実装できない項目**になるので、具象型の固有メソッドに留める。
    ///
    /// # 失敗は「取れなかった」以上の意味を持たない
    ///
    /// 独自 API を持たない上流では 404、停止していれば `Connect`、
    /// 一覧に居なければ `MissingContextLength` になる。**どれも起動を止める理由にはならない**
    /// ので、呼び出し側（`server::context`）は既定値へ落として `warn` を出す。
    pub async fn loaded_context_length(&self) -> Result<u32, LlmError> {
        self.models().await?.context_length
    }

    /// **モデル一覧の照会1回ぶん**（作業単位 E-2）。
    ///
    /// [`loaded_context_length`](Self::loaded_context_length) と**同じ1回の往復**から、
    /// コンテキスト長と**上流が持っているモデルの綴り**の両方を取り出す。
    ///
    /// # なぜ1回にまとめるのか
    ///
    /// 別のメソッドとして往復を足すと、**起動時の照会が1つ増える。**
    /// 引くのは推論を起こさないメタデータだが、
    /// **「起動時の照会はモデルごとに1回」を見張っているテストがある**
    /// （`tests/server_context_refresh.rs`）ので、増やせばその主張が変わる。
    /// **同じ本文から2つ読むだけなので、増やす理由が無い。**
    ///
    /// # **読み手は2つある**（作業単位 G-2）
    ///
    /// 1つ目が [`NATIVE_MODELS_PATH`]、2つ目が [`OPENAI_MODELS_PATH`]。
    /// **2つ目を投げるのは、1つ目のエンドポイントが無かったときだけ**（2xx 以外）。
    ///
    /// | 1つ目の結果 | 2つ目 | なぜ |
    /// |---|---|---|
    /// | 2xx（読めても読めなくても） | **投げない** | LM Studio 経由の往復は今までどおり1回 |
    /// | 2xx 以外（404 等） | **投げる** | `llama-server` はここに来る |
    /// | 応答が1つも来ない（タイムアウト / 接続失敗） | **投げない** | **起動時の待ちを倍にしない。** 黙っている上流は2つ目にも黙っている |
    ///
    /// # ⚠ 「200 だが長さが読めない」で2つ目を投げないこと（**変異検査の結果**）
    ///
    /// 着手時はそちらでも投げていた。**変異検査で、その枝を丸ごと消す変異が
    /// 1931 全緑で通った**（2026-09-05 / 作業単位 G-2）。見張り番が無いのは当然で、
    /// **得をする上流が存在しない** ——
    ///
    /// * **LM Studio の `/v1/models` は `meta` を持たない**ので、
    ///   「一覧に居ない・未ロード」で投げ直しても**必ず空振りする。**
    /// * **JIT ロードなので「未ロード」は普通の状態**（docs/archive/04-environment-lm-studio.md）。
    ///   投げると**冷えた起動のたびに、モデルごとに往復が1つ増える。**
    ///
    /// **空振りする往復を増やすだけの枝だったので、条件を「2xx 以外」に狭めた。**
    ///
    /// **「起動時の照会はモデルごとに1回」という主張は、上流が LM Studio である限り
    /// そのまま**（冷えていても・別のモデルを設定していても変わらない）。
    /// 増えるのは**独自 API を持たない上流に対してだけ**で、
    /// そこでは1回目が即座に 404 で返る。
    pub async fn models(&self) -> Result<ModelsReport, LlmError> {
        self.models_voiced(ListFailureVoice::Warn).await
    }

    /// **載せている最中の上流へ聞く**（作業単位 B-51 / ADR-0042 §決定3）。
    /// 往復も読み方も返り値も [`Self::models`] と同じで、**違うのは1行の水準だけ**。
    ///
    /// **どちらの綴りでも読めなかったことを `debug` で言う**（[`Self::models`] は `warn`）。
    /// 自分で起こした上流は、載り終わるまで**どの綴りにも 503 を返す**（`llama-server`）。
    /// **それは期待している状態**なので、警告にすると2秒ごとに鳴り（Issue origin#20 の実測で
    /// 20 秒の載せ替えに 8 行）、**本物の警告が埋もれる**（ADR-0035）。
    /// 経過は呼び出し側（`server::launch` の番人）が `info` で言い、
    /// 緩い天井を超えたら `warn` を1本出す。
    ///
    /// # ⚠ 呼ぶのは番人の「待つ間」だけ
    ///
    /// **起こしていない面（外の上流）・載った後の面・5分ごとの引き直しは [`Self::models`]**
    /// ——そちらの 503 は期待していない状態で、**今のまま警告である。**
    /// 見張り番は `server::context::tests::an_unreadable_list_is_progress_for_a_launching_face_but_a_warning_for_an_external_one`
    /// （同じ 503・同じ捕捉で、**面ごとに水準が割れること**を同じ行の中で見る）と、
    /// `tests/upstream_launch.rs` の
    /// `a_list_the_launching_face_cannot_read_yet_is_not_a_warning_but_an_external_one_is`
    /// （**番人が本当にこちらを呼んでいること**を、偽の `llama` で見る）。
    ///
    /// # ⚠ 水準を落とすのは「どちらの綴りでも読めなかった」1行だけ
    ///
    /// **通信そのものの失敗（接続できない・時間内に終わらない）は [`Self::models`] と同じ `info`**
    /// （[`RequestFailureVoice::Murmur`]）。**触っていない。**
    ///
    /// # なぜ引数ではなくメソッドを分けたか
    ///
    /// **水準を表す型を `pub` にすると、`llm` の外に上流固有の綴りが1つ増える**
    /// （`upstream_leaks` の網1が数える。メソッドは数えない）。
    /// **呼び出し側が選ぶのは「どちらを呼ぶか」で足りる。**
    /// **`llm::LlmClient` のトレイトは触っていない**（この照会はもともとトレイトに無い）。
    pub async fn models_while_loading(&self) -> Result<ModelsReport, LlmError> {
        self.models_voiced(ListFailureVoice::Progress).await
    }

    /// **観測のためだけに聞く**（作業単位 B-66 のレビュー是正 / Issue origin#94 / ADR-0020 の追記）。
    /// 往復も読み方も返り値も [`Self::models`] と同じで、**違うのは「どちらの綴りでも読めなかった」1行の水準だけ**
    /// （`debug`。[`Self::models_while_loading`] と同じ水準）。
    ///
    /// **呼ぶのは、コンテキスト長を書いた面（予算に使わない照会）だけ**（`server::context::refresh_once` /
    /// `refresh_sub_once`）。B-66 の1周目まで、その面には照会しなかったので、この `warn` は1度も出ていなかった。
    /// **モデル一覧は上流に「任意」の能力**（ADR-0038 §決定1）なので、一覧の口を持たない上流にコンテキスト長を
    /// 書いて繋ぐ構成では、警告にすると**会話は通るのに5分ごとに鳴る**。結果は観測の `check_result` が言う
    /// （`no_model_list`）。**通信そのものの失敗の水準（`info`）は触っていない。**
    ///
    /// 見張り番は `server::context::tests::an_unreadable_list_is_not_a_warning_for_a_written_face_but_stays_one_otherwise`。
    pub async fn models_for_observation(&self) -> Result<ModelsReport, LlmError> {
        self.models_voiced(ListFailureVoice::Progress).await
    }

    /// [`Self::models`] / [`Self::models_while_loading`] の本体。
    /// **`voice` が効くのは「どちらの綴りでも読めなかった」1行の水準だけ**（[`ListFailureVoice`]）。
    async fn models_voiced(&self, voice: ListFailureVoice) -> Result<ModelsReport, LlmError> {
        tracing::debug!(
            model = self.config.model.as_str(),
            "推論サーバへコンテキスト長を照会します"
        );

        match self
            .fetch_models(
                &self.config.models_endpoint,
                NATIVE_MODELS_PATH,
                // **1つ目が黙ったら往復はここで終わる**（下の枝）ので、
                // **綴りを名乗れる側が他に居ない**——だから `Hush` にはしない。
                //
                // **⚠ それでも `Speak` ではない**（作業単位 G-8 ①）。
                // この往復は失敗しても何も止めない（「取れたら得をする」経路）ので、
                // **警告にすると上流が停止しているだけで5分ごとに鳴る。**
                // 落とし所へ落ちた事実は呼び出し側（`server::context`）が `warn` で言う。
                RequestFailureVoice::Murmur,
            )
            .await
        {
            // **1回で足りた。** ここが LM Studio の通る道（往復は増えない）。
            Ok(body) => Ok(ModelsReport {
                context_length: parse_loaded_context_length(&body, self.config.model.as_str()),
                catalogue: parse_model_catalogue(&body),
                // **この綴りが 2xx を返した**（長さが読めたかどうかとは別）。
                answered_by: AnsweredBy::NativeApi,
            }),
            Err(ModelsFetchError { error, snippet }) => match error {
                // **上流は応答した。この綴りのエンドポイントを持っていないだけ。**
                LlmError::Status { status } => {
                    self.read_compat_models(status, snippet, voice).await
                }
                // **応答が1つも来なかった。** 2つ目を投げても待ちが増えるだけ。
                //
                // **ここは黙らない**——上流の停止・タイムアウト・通信の失敗は
                // `map_request_error` が**1本だけ**出している。
                //
                // **⚠ 3つとも `info`**（`RequestFailureVoice::Murmur` / 作業単位 G-8 ①）。
                // 着手前は接続失敗だけが `info` で、残り2つが `warn` だった——
                // **同じ「上流が答えない」が、断られ方で割れていた形**
                // （拒否されれば `Connect`、接続だけ受けて黙れば `Timeout`）。
                // 後者は**起動時に2本・5分ごとに2本**鳴っていた（G-5 が消しにきた形）。
                //
                // 見張り番は `tests::a_best_effort_query_names_its_failure_without_warning`
                // ——**`Speak` へ戻すと2腕が `warn` になって赤、
                // `Hush` へ潰すと1本ごと消えて赤。**
                error => Err(error),
            },
        }
    }

    /// **2つ目の読み手**（OpenAI 互換 / 作業単位 G-2）。
    /// **1つ目のエンドポイントが無かったときだけ走る。**
    ///
    /// `native_status` / `native_body` は1つ目が返したもの。
    /// **長さが読めなければ、失敗の理由は1つ目のものを保つ**
    /// （2つ目の 200 で塗り潰さない——調査先が「別の上流を向いている」から
    /// 「その上流が答えなかった」へすり替わる）。
    /// **例外は「2つ目だけが鍵を断った（401 / 403）」回**で、そのときは2つ目のステータスを返す
    /// （[`refusal_first`] / 作業単位 A-13b-1）。
    ///
    /// # **2つ目の読み手について `warn` を出すのは、ここだけ**（作業単位 G-5）
    ///
    /// **どちらの読み手も何も返さなかったときだけ鳴る。**
    /// 読み手1つぶんの失敗は [`Self::fetch_models`] が `debug` で出すだけで、
    /// **`llama-server` の 404 はここまで来ない**（2つ目が答えるので）。
    ///
    /// # ⚠ 「ここが唯一の `warn` の出どころ」ではない（G-5 の是正 / 2026-09-06）
    ///
    /// **見出しはかつてそう書いてあった。測ると偽だった。**
    /// `native 404` ＋ `compat` を [`LmStudioConfig::with_context_timeout`] より遅くすると、
    /// **同じ1つの事実に対して `warn` が2本出た** ——
    /// `map_request_error` の「時間内に完了しませんでした」と、下の1行。
    /// **G-5 が消しにきた「同じ事実が2本」そのものだった。**
    ///
    /// **直したのは見出しではなく振る舞いのほう**——2つ目の読み手は
    /// [`RequestFailureVoice::Hush`] で引くので、**この経路の `warn` はこの1行だけになった。**
    /// 見張り番は [`tests::neither_reader_answering_is_warned_with_both_paths_on_one_line`]
    /// の `timeout` の腕で、**本数ではなく「出した層の一覧」を `assert_eq!` で固定してある。**
    ///
    /// **⚠ 訂正: いまは「この往復に `warn` は1本きり」と書ける**（作業単位 G-9 / 2026-09-07）。
    /// **G-5 の時点では書けなかった** ——**2つ目が 200 を返し、綴りは在るが長さが無い回**
    /// （`meta` を持たない上流）に、`llm::response` が
    /// 「実効コンテキスト長がありません」を `warn` で1本出していた（G-2 からの持ち越し）。
    /// **それが `debug` へ落ちた**（`parse_effective_context_length` の doc
    /// 「読めなかったことを、この層は警告にしない」）。
    /// 見張り番は [`tests::a_list_without_a_length_is_not_a_broken_upstream`]
    /// ——**出した層の一覧が空であることを固定してある。**
    ///
    /// [`tests::a_list_without_a_length_is_not_a_broken_upstream`]:
    ///     tests::a_list_without_a_length_is_not_a_broken_upstream
    ///
    /// **⚠ 「1つ目が失敗したら鳴らす」に戻さないこと。** それが
    /// **起動のたびに2本・5分ごとに2本**という形（G-2 / G-6 の実測）だった。
    /// 見張り番は [`tests::a_native_404_is_not_a_warning_when_the_openai_list_answers`]
    /// と [`tests::neither_reader_answering_is_warned_with_both_paths_on_one_line`]
    /// ——**両向きを1本ずつ押さえてある。**
    ///
    /// # ⚠ この設計が意図的に静かにする、唯一のクラス
    ///
    /// **「1つ目が 5xx で、2つ目が使える 200 を返した」回**は、既定のフィルタに
    /// 何も出なくなった（1つ目の失敗は [`Self::fetch_models`] の `debug` だけ）。
    /// **上流が半分壊れているのに、往復は成功するので黙る。**
    ///
    /// **それでよいと決めた**——`llama-server` の 404 と区別する手段が
    /// ステータスの値しか無く、**値で深刻度を分けると「実測由来の値を、
    /// 外れたら壊れる場所に置く」形**になる（CLAUDE.md / ADR-0036）。
    /// **上流の一覧は現に読めているので、この経路の振る舞いは正しい。**
    ///
    /// **⚠ 記録として残す。** ここを直したくなったときに読む段である。
    ///
    /// # ⚠ `native_body` が `None` の腕は到達しない
    ///
    /// この関数へ入るのは [`Self::models`] が [`LlmError::Status`] を受けたときだけで、
    /// [`Self::fetch_models`] は**ステータス異常のときだけ `snippet` を詰める。**
    /// つまり `native_body` は常に `Some`。
    /// **そこに見張り番を書いても永久に緑なので、「守られている」と数えないこと**
    /// （CLAUDE.md「死んだ検査は変異検査でしか見つからない」）。
    ///
    /// [`tests::a_native_404_is_not_a_warning_when_the_openai_list_answers`]:
    ///     tests::a_native_404_is_not_a_warning_when_the_openai_list_answers
    /// [`tests::neither_reader_answering_is_warned_with_both_paths_on_one_line`]:
    ///     tests::neither_reader_answering_is_warned_with_both_paths_on_one_line
    async fn read_compat_models(
        &self,
        native_status: u16,
        native_body: Option<String>,
        voice: ListFailureVoice,
    ) -> Result<ModelsReport, LlmError> {
        let compat_outcome = match self.compat_models(native_status).await {
            Ok(report) => return Ok(report),
            Err(outcome) => outcome,
        };

        // **ここが「本物の問題」。** 1つ目が答えず、2つ目からも何も得られなかった。
        // **1行の中で、どちらの綴りをどのステータスで失ったかまで読めること**
        // （CLAUDE.md「その行が出た」と「その行がこう言っている」は別）。
        //
        // **⚠ ただし、載せている最中の上流では期待している状態**（作業単位 B-51）——
        // そのときだけ `debug` へ落とす（[`LmStudioClient::models_while_loading`]）。
        // **2つの腕は水準以外を1文字も変えないこと**（文面と欄が割れると、
        // 同じ事実を面ごとに別の綴りで探すことになる）。
        //
        // `model` は**どの面の照会かを同じ行で読むため**（B-51 で足した。依頼する綴りで、
        // `server` の面ごとに別のクライアントが持つ）。
        let model = self.config.model.as_str();
        let native_body = native_body.as_deref().unwrap_or_default();
        match voice {
            ListFailureVoice::Warn => tracing::warn!(
                model = %model,
                native_path = NATIVE_MODELS_PATH,
                native_status,
                // 本文はログにだけ出す。**エラーレスポンスには載せない。**
                native_body = ?native_body,
                compat_path = OPENAI_MODELS_PATH,
                // **2つ目の側**（どう失敗したか）。**1つ目のステータスと取り違えないこと。**
                compat_outcome = %compat_outcome,
                "推論サーバのモデル一覧を、どちらの綴りでも読めませんでした"
            ),
            ListFailureVoice::Progress => tracing::debug!(
                model = %model,
                native_path = NATIVE_MODELS_PATH,
                native_status,
                native_body = ?native_body,
                compat_path = OPENAI_MODELS_PATH,
                compat_outcome = %compat_outcome,
                "推論サーバのモデル一覧を、どちらの綴りでも読めませんでした"
            ),
        }
        Err(LlmError::Status {
            status: refusal_first(native_status, compat_outcome),
        })
    }

    /// 2つ目の読み手そのもの。**何も得られなければ [`CompatOutcome`]**
    /// （深刻度は呼び出し側が決める。**ここが返すのは事実だけ**）。
    async fn compat_models(&self, native_status: u16) -> Result<ModelsReport, CompatOutcome> {
        let native_error = || LlmError::Status {
            status: native_status,
        };

        let body = match self
            .fetch_models(
                &self.config.compat_models_endpoint,
                OPENAI_MODELS_PATH,
                // **ここで深刻度を名乗らせない**（G-5 の是正）——
                // 2つ目の失敗は呼び出し側が `compat_outcome` として1行に載せる。
                RequestFailureVoice::Hush,
            )
            .await
        {
            Ok(body) => body,
            Err(failure) => {
                tracing::debug!(error = %failure.error, "OpenAI 互換のモデル一覧も読めませんでした");
                return Err(CompatOutcome::from_error(&failure.error));
            }
        };

        // **`state` を持たないだけで、綴りの一覧としては同じ形**（`data[].id`）。
        let catalogue = parse_model_catalogue(&body);

        match parse_effective_context_length(&body, self.config.model.as_str()) {
            Ok(found) => {
                // **どの行をどう同定したかを同じ行に出す。**
                // `upstream_model_id` は gguf の絶対パスになりうる——
                // **ログには出してよいが、エラーレスポンスには載せない**（CLAUDE.md）。
                tracing::info!(
                    model = self.config.model.as_str(),
                    upstream_model_id = %found.model_id,
                    identified_by = found.matched.label(),
                    context_length = found.tokens,
                    "OpenAI 互換のモデル一覧から実効コンテキスト長を読みました"
                );
                Ok(ModelsReport {
                    context_length: Ok(found.tokens),
                    catalogue,
                    // **どう同定したかだけを持ち上げる。** 綴り（gguf の絶対パス）は
                    // 上の `info` の1行に残し、この型には載せない（`AnsweredBy` の doc）。
                    answered_by: AnsweredBy::from_match(found.matched),
                })
            }
            Err(err) => {
                tracing::debug!(
                    error = %err,
                    "OpenAI 互換のモデル一覧からも実効コンテキスト長を読めませんでした"
                );
                if catalogue.is_empty() {
                    // **2つ目からも何も得られなかった。** 1つ目の失敗をそのまま返す
                    // （呼び出し側は「取れなかった」として既定値へ落ちる）。
                    //
                    // **⚠ この3行は、`llm` の外の呼び出し側からは観測できない**
                    // （変異検査で実測 / G-2: `if false` にしても 1932 全緑）——
                    // `loaded_context_length` はどちらでも同じ `Err` を返し、
                    // `server::context::resolve_sub_context` の2つの枝は**同じ文言を出す**うえ、
                    // `measured::check` は**空の一覧を `None` と同じ「聞けなかった」に倒す。**
                    // **寄りかかっているのはその2つ。**
                    //
                    // **⚠ 等価変異ではない。`models()` からは区別できる**
                    // （G-2 のレビューが複製ツリーで実測 / 2026-09-05）——
                    // `if false` にすると、**空の一覧を積んだ `Ok` が返る。**
                    // 見張り番は `tests/llm_lmstudio.rs` の
                    // `an_empty_openai_list_fails_the_whole_round_trip`。
                    // **「観測できない」は「その観測手段では観測できない」でしかない**（CLAUDE.md）。
                    return Err(CompatOutcome::Unusable);
                }
                // **一覧だけは取れた**（`measured::check` はこれで足りる）。
                //
                // **⚠ ここで `warn` を出さないこと**（作業単位 G-5）。
                // 長さが読めなかったことは呼び出し側（`server::context`）が
                // 「落とし所を使います」で1本出す。**出すと同じ事実が2本になる。**
                //
                // **⚠ 訂正の訂正**（作業単位 G-9 / 2026-09-07）——
                // G-5 の時点では `parse_effective_context_length` が `llm::response` で
                // 「実効コンテキスト長がありません」を **`warn` で1本**出しており
                // （G-2 からの持ち越し）、**ここで足すと3本目**になる状態だった。
                // **その1本は `debug` へ落ちた**ので、いま `warn` は呼び出し側の1本だけ。
                // **ここで足すと、また2本になる。**
                //
                // 見張り番は `tests::a_list_without_a_length_is_not_a_broken_upstream`
                // ——**本数ではなく、出した層の一覧を固定してある**
                // （本数で見ると、上の1本と足し引きして黙る）。
                Ok(ModelsReport {
                    context_length: Err(native_error()),
                    catalogue,
                    // **綴りの一覧だけが取れた。** 同定はできていない。
                    answered_by: AnsweredBy::CompatCatalogueOnly,
                })
            }
        }
    }

    /// モデル一覧を1回引いて、本文を返す。**ステータスまで確認する。**
    ///
    /// `path` は**ログに出す綴り**（どちらの読み手が失敗したのかを読めるようにする）。
    ///
    /// # ⚠ ここでステータス異常の深刻度を決めない（作業単位 G-5）
    ///
    /// **1つの読み手が失敗したことは、それ自体では問題ではない**
    /// （[`ModelsFetchError`] の doc）。**`warn` に戻さないこと。**
    /// 判定は [`Self::read_compat_models`] が「どちらも駄目だったか」で行う。
    ///
    /// # ⚠ 通信そのものの失敗は `voice` 次第（G-5 の是正 / 2026-09-06）
    ///
    /// **1つ目の読み手が黙ったら、そこで往復は終わる**（2つ目は投げない /
    /// [`Self::models`] の表）ので、**深刻度を決められる側が他に居ない。**
    /// だから1つ目は [`RequestFailureVoice::Speak`]。
    ///
    /// **2つ目は [`RequestFailureVoice::Hush`]。** あちらの失敗は
    /// [`Self::read_compat_models`] が `compat_outcome` として1行に載せるので、
    /// **ここでも出すと同じ事実が2本になる。**
    async fn fetch_models(
        &self,
        url: &str,
        path: &'static str,
        voice: RequestFailureVoice,
    ) -> Result<String, ModelsFetchError> {
        let response = self
            .authorized(self.http.get(url), voice)
            .map_err(|error| ModelsFetchError {
                error,
                snippet: None,
            })?
            // **起動時に走る照会なので、必ず短い上限で切る。**
            .timeout(self.config.context_timeout)
            .send()
            .await
            .map_err(|err| ModelsFetchError {
                error: map_request_error(err, voice),
                snippet: None,
            })?;

        let status = response.status();
        if !status.is_success() {
            // 本文はログにだけ出す。**エラーレスポンスには載せない。**
            let snippet = self.error_snippet(response).await;
            tracing::debug!(
                status = status.as_u16(),
                path,
                body = ?snippet,
                "推論サーバがモデル一覧にエラーを返しました"
            );
            return Err(ModelsFetchError {
                error: LlmError::Status {
                    status: status.as_u16(),
                },
                snippet: Some(snippet),
            });
        }

        response.text().await.map_err(|err| ModelsFetchError {
            error: map_request_error(err, voice),
            snippet: None,
        })
    }

    /// ストリーミングの補完。**本文の到着を待たずに返る。**
    ///
    /// HTTP のステータス異常はストリームの項目ではなく**この関数の失敗**として返す
    /// （下流が「ストリームは開いたのに中身がエラー」を扱わずに済む）。
    pub async fn stream(&self, request: &ChatRequest) -> Result<ChatStream, LlmError> {
        let response = self.post(request, true).await?;
        // `bytes_stream` は `Unpin` とは限らないので、ここで固定してから SSE に載せる。
        let events = Box::pin(response.bytes_stream()).eventsource();
        Ok(Box::pin(chunk_stream(events)))
    }

    /// 要求を投げ、ステータスまで確認した応答を返す。
    async fn post(
        &self,
        request: &ChatRequest,
        streaming: bool,
    ) -> Result<reqwest::Response, LlmError> {
        let body = self.chat_body(request, streaming);
        let mut builder = self.authorized(
            self.http.post(&self.config.endpoint).json(&body),
            RequestFailureVoice::Speak,
        )?;
        builder = if streaming {
            builder.header(reqwest::header::ACCEPT, "text/event-stream")
        } else {
            // 全体上限は非ストリーミングにだけ付ける（ストリーミングに付けると
            // 長い応答が本文の途中で切られる）。
            builder.timeout(self.config.request_timeout)
        };

        tracing::debug!(
            model = self.config.model.as_str(),
            messages = request.messages.len(),
            streaming,
            "推論サーバへ要求を送ります"
        );

        let response = builder
            .send()
            .await
            .map_err(|err| map_request_error(err, RequestFailureVoice::Speak))?;
        let status = response.status();
        if !status.is_success() {
            // 本文はログにだけ出す。**エラーレスポンスには載せない。**
            let snippet = self.error_snippet(response).await;
            tracing::warn!(
                status = status.as_u16(),
                body = ?snippet,
                "推論サーバがエラーを返しました"
            );
            return Err(LlmError::Status {
                status: status.as_u16(),
            });
        }

        Ok(response)
    }

    /// **チャットへ送る本文**。[`Self::post`] と [`Self::count_request`] の**両方がここを通る**
    /// （封の本文を別に組まない / ADR-0052 §推奨3）。
    fn chat_body<'a>(&'a self, request: &'a ChatRequest, streaming: bool) -> WireRequest<'a> {
        wire_request(&self.config.model, request, streaming, self.config.sampling)
    }

    /// **本文ごとに数える**（封 / ADR-0052 §推奨3 / 作業単位 B-60a）。
    ///
    /// **チャットへ送るのと同じ本文**（[`Self::chat_body`]）を [`APPLY_TEMPLATE_PATH`] へ送り、
    /// 返った組み上がりを [`TOKENIZE_PATH`] で数える。往復は2つ。
    /// 返り値の読み方は [`LlmClient::count_request`] の表。
    ///
    /// # 見張り番
    ///
    /// `tests/llm_count.rs` の `the_seal_carries_the_same_body_as_the_chat_request`（N6）——
    /// **偽の上流が受け取った2つの本文をバイト列で比べる。**
    ///
    /// # 時間の上限は照会の上限（[`LmStudioConfig::context_timeout`]）
    ///
    /// **新しい定数を足さない**（ADR-0052 §推奨4 / ADR-0035）。数える往復はモデルを載せず、
    /// **切れてもその回がバイト数に戻るだけ**なので、本文生成の天井（既定 180 秒）を引き継がない
    /// —— 引き継ぐと、数える口が黙った回にターンが数分止まる。上限は**往復ごと**に掛かる
    /// （2往復なので、最悪はその2倍）。
    ///
    /// **生成中も `/tokenize` は生成の列に並ばない**（ADR-0052 M2 / 2026-10-22・この機の llama）。
    /// 15k 級の本文（`/apply-template` ＋ `/tokenize`）で、空いているとき中央値 約 117ms、
    /// 別のスロットが前処理・生成している最中も中央値 120〜129ms だった
    /// （4スロットが埋まり5本目が列で待っている間も約 123ms）。
    /// もし待たされる上流なら、上限で切れて**バイト数に戻る**（遅くなるのではなく、窓が狭くなる）。
    pub async fn count_request(
        &self,
        request: &ChatRequest,
        streaming: bool,
    ) -> Result<TokenCount, LlmError> {
        let body = self.chat_body(request, streaming);
        let Some(answer) = self
            .post_for_count(
                &self.config.apply_template_endpoint,
                APPLY_TEMPLATE_PATH,
                &body,
            )
            .await?
        else {
            return Ok(TokenCount::Uncountable);
        };
        let Some(prompt) = parse_applied_template(&answer) else {
            tracing::debug!(
                model = self.config.model.as_str(),
                path = APPLY_TEMPLATE_PATH,
                body = ?log_snippet(&answer, self.config.api_key.as_ref()),
                "組み上げの口が組み上がりを返しませんでした（数えられない）"
            );
            return Ok(TokenCount::Uncountable);
        };
        self.tokenize(&prompt, TokenizeWhat::WholePrompt).await
    }

    /// **文字列ごとに数える**（計画の部品 / ADR-0052 §推奨2 / 作業単位 B-60a）。
    ///
    /// [`TOKENIZE_PATH`] へ1往復。**BOS を足さずに数える**（[`TokenizeWhat::Fragment`]）。
    /// 返り値の読み方は [`LlmClient::count_request`] の表。
    pub async fn count_text(&self, text: &str) -> Result<TokenCount, LlmError> {
        self.tokenize(text, TokenizeWhat::Fragment).await
    }

    /// [`TOKENIZE_PATH`] で数え、**形で判定する**（`response::parse_token_count`）。
    ///
    /// **中身のある文字列を 0 と数えた答えは信じない**（`usage` の 0 を信じないのと同じ向き ——
    /// 0 トークンの封は窓を超えて送る向きの誤りになる）。
    async fn tokenize(&self, content: &str, what: TokenizeWhat) -> Result<TokenCount, LlmError> {
        let body = TokenizeRequest {
            content,
            add_special: what.add_special(),
            parse_special: PARSE_SPECIAL,
            with_pieces: false,
        };
        let Some(answer) = self
            .post_for_count(&self.config.tokenize_endpoint, TOKENIZE_PATH, &body)
            .await?
        else {
            return Ok(TokenCount::Uncountable);
        };
        match parse_token_count(&answer) {
            Some(0) if !content.is_empty() => {
                tracing::debug!(
                    model = self.config.model.as_str(),
                    path = TOKENIZE_PATH,
                    "数える口が、中身のある文字列を 0 トークンと答えました（数えられない）"
                );
                Ok(TokenCount::Uncountable)
            }
            Some(tokens) => {
                tracing::debug!(
                    model = self.config.model.as_str(),
                    path = TOKENIZE_PATH,
                    tokens,
                    bytes = content.len(),
                    "上流がトークンを数えました"
                );
                Ok(TokenCount::Counted(tokens))
            }
            None => {
                tracing::debug!(
                    model = self.config.model.as_str(),
                    path = TOKENIZE_PATH,
                    body = ?log_snippet(&answer, self.config.api_key.as_ref()),
                    "数える口の応答が数の列ではありませんでした（数えられない）"
                );
                Ok(TokenCount::Uncountable)
            }
        }
    }

    /// 数える口へ1往復し、**成功のときだけ**本文を返す。
    ///
    /// | 結果 | 返り値 |
    /// |---|---|
    /// | 2xx | `Ok(Some(本文))` —— **数えたかどうかは、まだ決まっていない**（形は呼び出し側が見る） |
    /// | 4xx など（llama.cpp の 404） | `Ok(None)` —— 上流は答えたが、数えない |
    /// | **5xx**（llama.cpp の読み込み中の 503 など） | **`Err(LlmError::Status)`** —— 今回は聞けなかった |
    /// | 往復の失敗 | `Err` |
    ///
    /// **2xx は数えた証拠にならない**（LM Studio は未知の口に 200 ＋ エラー本文を返す）。
    /// **逆に、2xx 以外の本文を数として信じるほうへは倒さない**（保守的な側）。
    ///
    /// # 5xx を「数えられない」にしない（作業単位 B-60b のレビュー）
    ///
    /// **5xx は上流の性質ではなく、上流のそのときの状態**である（読み込み中・混雑・内部の失敗）。
    /// `Ok(Uncountable)` に畳むと、呼び出し側は「数えられない上流だ」と覚えうる
    /// （`server::seal` は覚えた世代の間もう数えない）。**口が無いことは 4xx（404 等）で言われる**ので、
    /// 5xx は往復の失敗の側（`Err`）に入れる。
    /// **429 / 408 も「そのときの状態」だが、いまは `Uncountable` のまま**（レビューの提案。
    /// 数える口がそれを返す上流をまだ見ていない —— 推測で分類を増やさない。覚えるのは本文が通った回だけなので、害は狭い）。
    /// 見張り番は `tests/llm_count.rs` の `a_server_error_is_a_failed_round_trip_not_an_uncountable_upstream`。
    ///
    /// # ここで警告を出さない
    ///
    /// **LM Studio では毎ターン「数えられない」になる**ので、`warn` にすると毎ターン鳴る。
    /// 往復そのものの失敗は [`RequestFailureVoice::Murmur`]（`info`）——
    /// **失敗しても何も止まらない**（その回がバイト数に戻るだけ）往復の声である。
    /// **5xx はここでは `debug` だけ**で、既定のログでは封の行の `seal=failed`（`info`）で見える。
    async fn post_for_count<B: serde::Serialize + ?Sized>(
        &self,
        url: &str,
        path: &'static str,
        body: &B,
    ) -> Result<Option<String>, LlmError> {
        let response = self
            .authorized(self.http.post(url), RequestFailureVoice::Murmur)?
            .json(body)
            .timeout(self.config.context_timeout)
            .send()
            .await
            .map_err(|err| map_request_error(err, RequestFailureVoice::Murmur))?;

        let status = response.status();
        if status.is_server_error() {
            let snippet = self.error_snippet(response).await;
            tracing::debug!(
                model = self.config.model.as_str(),
                status = status.as_u16(),
                path,
                body = ?snippet,
                "推論サーバの数える口がサーバ側のエラーを返しました（今回は聞けなかった）"
            );
            return Err(LlmError::Status {
                status: status.as_u16(),
            });
        }
        if !status.is_success() {
            let snippet = self.error_snippet(response).await;
            tracing::debug!(
                model = self.config.model.as_str(),
                status = status.as_u16(),
                path,
                body = ?snippet,
                "推論サーバの数える口が成功以外を返しました（数えられない）"
            );
            return Ok(None);
        }

        response
            .text()
            .await
            .map(Some)
            .map_err(|err| map_request_error(err, RequestFailureVoice::Murmur))
    }
}

/// **数える口の `parse_special`**（ADR-0052 / 作業単位 B-60a・B-60b2）。**封（全体）も部品も同じ値。**
///
/// # M1 で確かめた（2026-10-22 / この機の llama・Qwen 公式と Ministral 14B）
///
/// 本番のチャットの経路は、組み上がりの中の制御トークンの綴り（Qwen の `<|im_start|>`、
/// Ministral の `[INST]` 等）を**1トークンとして読む**。
/// * **`false` で数えた封は毎回多く数えた。** その綴りが普通の文字列として数トークンに割れ、
///   差はメッセージ数で決まった（Qwen 約 10 × メッセージ数 ＋ 3、Ministral 4 × メッセージ数 ＋ 11）。
/// * **同じ本文を `true` で数え直すと、`usage.prompt_tokens` と Qwen 9/9・Ministral 8/8 で
///   完全に一致した**（Ministral は `add_special = true` のとき。[`TokenizeWhat::add_special`]）。
///   非ストリーミングと中継で違いは無かった。
/// * **部品も `true`。** M1 の一致は、利用者の発言を含む全体を `true` で数えたもの ——
///   本番は本文の中の綴りも制御トークンとして読んでいる（ADR-0052「測った後の決め直し」1）。
///
/// **⚠ 測ったのはこの機・この2系統だけ**（ADR-0036）。本番の数え方と食い違えば、
/// 封の数で送るかを決めている（B-60d-1 / `server::counted_plan`）ので、その回は窓を超えうる
/// （ADR-0052 §整合 ADR-0036）。第3層が `封 < 実` に気付いてバイト数へ戻す仕組みは作業単位 B-60d-2 で、まだ無い。
/// 既定値に任せず必ず送る（上流の既定が変わっても数え方が動かない）。
const PARSE_SPECIAL: bool = true;

/// **何を数えるか**で `add_special`（BOS 等を足すか）が変わる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenizeWhat {
    /// **送る本文の組み上がり全体**（封）。
    WholePrompt,
    /// **計画の部品**（記憶の1行など）。全体の途中に置かれる断片。
    Fragment,
}

impl TokenizeWhat {
    /// `add_special` の値。
    ///
    /// # M1 で確かめた範囲（ADR-0052 §背景「BOS の数え方」/ 2026-10-22）
    ///
    /// * **封（全体）は `true`。** Ministral 14B は全体に BOS を足すと `usage.prompt_tokens` と
    ///   一致し（8/8）、**`false` だと BOS の分だけ1つ少なく数えた**（壊れる側）。
    ///   Qwen 公式は `add_special` に依らず一致した（この旗が効かない）。
    ///   どちらも [`PARSE_SPECIAL`] が `true` のときの話。
    /// * **部品は `false`。** 部品は全体の途中に置かれるので、BOS は付かない。
    ///   **部品の数は安全を担わない**（外れたら封が組み直させるだけ / ADR-0052 §推奨2）ので、
    ///   部品ごとに BOS を足して全部の部品を1つずつ多く数える理由が無い。
    const fn add_special(self) -> bool {
        match self {
            Self::WholePrompt => true,
            Self::Fragment => false,
        }
    }
}

/// [`TOKENIZE_PATH`] へ送る本文。**欄は全部明示する**（上流の既定に任せない）。
#[derive(Debug, serde::Serialize)]
struct TokenizeRequest<'a> {
    content: &'a str,
    add_special: bool,
    parse_special: bool,
    /// **`true` にすると返る形が `[{"id":…,"piece":…}]` になり、数の列ではなくなる。**
    with_pieces: bool,
}

/// ログに出す本文の先頭（[`ERROR_BODY_LOG_LIMIT`] 文字まで）。**エラーレスポンスには載せない。**
///
/// **鍵を伏せてから切り詰める**（[`error_snippet`] と同じ順 / 作業単位 A-13b-1 のレビュー是正）——
/// 成功の応答（数える口の 200）が鍵を写し返しても、ログに出さない。
fn log_snippet(body: &str, key: Option<&ApiKey>) -> String {
    redact_key(body, key)
        .chars()
        .take(ERROR_BODY_LOG_LIMIT)
        .collect()
}

impl LlmClient for LmStudioClient {
    /// **具象の上流は差し替わらない**ので、自分自身をそのまま使えばよい（[`LlmClient::pinned`]）。
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        // 固有メソッドを明示して呼ぶ（トレイト経由だと自分を再帰呼び出しになる）。
        Box::pin(LmStudioClient::complete(self, request))
    }

    fn stream<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        Box::pin(LmStudioClient::stream(self, request))
    }

    fn count_request<'a>(
        &'a self,
        request: &'a ChatRequest,
        streaming: bool,
    ) -> LlmFuture<'a, TokenCount> {
        Box::pin(LmStudioClient::count_request(self, request, streaming))
    }

    fn count_text<'a>(&'a self, text: &'a str) -> LlmFuture<'a, TokenCount> {
        Box::pin(LmStudioClient::count_text(self, text))
    }
}

/// SSE のイベント列をチャンク列に均す。
///
/// **溜め込まない。** 1イベント受けるごとに1項目返す（`unfold` は次が要求されるまで
/// 上流を読まないので、下流の消費速度がそのまま反映される）。
///
/// 終わり方は3通り。いずれも**必ず `None` で終端する**（下流を宙吊りにしない）。
///
/// | 上流の様子 | 下流に見えるもの |
/// |---|---|
/// | `data: [DONE]` を受けた | そのまま終端（`[DONE]` は流さない） |
/// | `[DONE]` 前にストリームが尽きた | `LlmError::Truncated` を1件流してから終端 |
/// | 途中で失敗した | 対応する `LlmError` を1件流してから終端 |
///
/// 失敗の後に読み進めないのは、フレーミングが壊れた後の内容を本文として混ぜないため。
/// **そこまでに流したチャンクは有効**なので、呼び出し側は蓄積分を保存してよい。
///
/// # `fuse` を外さないこと
///
/// **`stream::unfold` は終端後にもう一度 poll されると panic する。**
/// `ChatStream` は公開 API なので、**受け取った側が終端後にもう一度 `next()` を
/// 呼ぶだけで本番コードパスの panic になる**（ループを抜けた後の駄目押しの1回、
/// 二重の `drain`、`select!` の他分岐が先に決着した後の再読み込み、など）。
/// 公開 API から到達できることが、`fuse` を残す理由として十分。
/// `fuse` を噛ませると以後は `None` を返し続けるだけになる。
///
/// **「axum に載せる限り安全」ではない**（そう読める書き方をしないこと）。
/// axum 0.8 の SSE 応答は終端後に再 poll せず、`chain` / `select` / `merge` /
/// `timeout` といった組み合わせ子も再 poll しないことを実測で確認しているが、
/// 危ないのは組み合わせ子ではなく**呼び出し側の余分な1回**の方。
/// `polling_after_termination_is_safe` がその1回を固定している。
fn chunk_stream<S>(events: S) -> impl Stream<Item = Result<ChatChunk, LlmError>> + Send
where
    S: Stream<Item = Result<Event, EventStreamError<reqwest::Error>>> + Send + Unpin + 'static,
{
    // 状態が `None` になったら終端。`[DONE]` と失敗の後はここに落とす。
    stream::unfold(Some(events), |state| async move {
        let mut events = state?;

        loop {
            match events.next().await {
                Some(Ok(event)) => {
                    if is_done(&event.data) {
                        tracing::debug!("上流のストリームが完了しました");
                        return None;
                    }
                    match parse_chunk(&event.data) {
                        Ok(Some(chunk)) => return Some((Ok(chunk), Some(events))),
                        // 中身の無いチャンクは読み飛ばして次を待つ（ここで止めない）。
                        Ok(None) => continue,
                        Err(err) => return Some((Err(err), None)),
                    }
                }
                Some(Err(err)) => return Some((Err(map_event_error(err)), None)),
                None => {
                    // **`[DONE]` を受けずに尽きた。** 黙って正常終了させると、
                    // 途中で切れた応答が「完結した応答」として保存される。
                    tracing::warn!("上流のストリームが完了前に終了しました");
                    return Some((Err(LlmError::Truncated), None));
                }
            }
        }
    })
    .fuse()
}

/// **「どちらの綴りでも読めなかった」1行を、どの水準で言うか**（作業単位 B-51）。
///
/// **呼び出し側が「どちらのメソッドを呼ぶか」で選ぶ**（[`LmStudioClient::models`] /
/// [`LmStudioClient::models_while_loading`]）。**`pub` にしないこと** ——
/// `llm` の外へ出すと、上流固有の綴りが1つ増える（`upstream_leaks` の網1）。
///
/// **⚠ [`RequestFailureVoice`] とは別物。** あちらは往復そのものの失敗
/// （接続・タイムアウト・通信）の1行の水準で、**こちらはそれに触らない。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListFailureVoice {
    /// **警告**（[`LmStudioClient::models`]）。起こしていない面・載った後・引き直し。
    Warn,
    /// **経過**（`debug` / [`LmStudioClient::models_while_loading`]）。自分で起こした上流が載るのを待つ間。
    /// **観測のためだけの照会**（[`LmStudioClient::models_for_observation`] / 作業単位 B-66）も同じ水準。
    Progress,
}

/// **この失敗について、その場で深刻度を名乗ってよいか**（作業単位 G-5 の是正）。
///
/// # なぜ呼び出し側が決めるのか
///
/// **「1つの読み手が失敗した」は事実であって深刻度ではない**（[`ModelsFetchError`] の doc）。
/// [`map_request_error`] が無条件に `warn` を出していたので、
/// **2つ目の読み手がタイムアウト／通信失敗した回は、同じ1つの事実に対して
/// `map_request_error` と [`LmStudioClient::read_compat_models`] が2本出していた**
/// （G-5 の是正で実測 / 2026-09-06）——**G-5 が消しにきた「同じ事実が2本」そのもの。**
///
/// **⚠ 黙らせてよいのは2つ目の読み手だけ。** 本文の中継（[`LmStudioClient::complete`] /
/// [`LmStudioClient::stream`]）と**1つ目の読み手**は、失敗したらそこで往復が終わるので、
/// **その場で名乗ってよい**（綴りを持っている側が他に居ない）。
///
/// # ⚠ 「名乗ってよい」と「警告にしてよい」は別だった（作業単位 G-8 ① / 2026-09-07）
///
/// **3つの声がある。** 分けたのは、**1つ目の読み手が「名乗るべきだが、警告ではない」**
/// という3つ目の場所だったから。
///
/// | 声 | 水準 | どういう失敗に付けるか |
/// |---|---|---|
/// | [`Speak`](Self::Speak) | 接続失敗は `info`、他は `warn` | **失敗が下流に届く**（本文の中継。利用者が 503 を受け取る） |
/// | [`Murmur`](Self::Murmur) | 3腕とも `info` | **失敗しても何も止まらない**（コンテキスト長の照会。落とし所へ落ちるだけ） |
/// | [`Hush`](Self::Hush) | 3腕とも `debug` | **同じ事実を、呼び出し側が1行にまとめて出す**（2つ目の読み手） |
///
/// **問いは「この失敗で何が失われるか」。** 何も失われないなら警告にしない
/// ——**上流が普段停止しているこの家では、それが5分ごとに鳴る**（ADR-0020 の引き直し）。
///
/// # 呼び出しの一覧と、その1つずつの見張り番（4周目の是正 / 2026-09-06）
///
/// **3周目は、この型を渡す口を5箇所へ新しく生やしたのに、
/// 見張り番が2つしか無かった**（残り3つは `Hush` へ潰す変異が 1949 全緑）。
/// **今日の値が正しいことと、次の編集からそれが守られることは別**である。
///
/// | 渡す場所 | 声 | 見張り番 |
/// |---|---|---|
/// | [`LmStudioClient::models`]（1つ目の読み手） | `Murmur` | [`tests::a_best_effort_query_names_its_failure_without_warning`] / [`tests::a_stopped_upstream_is_not_a_warning`] |
/// | [`LmStudioClient::compat_models`]（2つ目の読み手） | `Hush` | [`tests::the_hushed_reader_still_speaks_at_debug`] / [`tests::neither_reader_answering_is_warned_with_both_paths_on_one_line`] |
/// | [`LmStudioClient::post`]（要求の送信） | `Speak` | [`tests::a_relay_that_never_gets_an_answer_names_its_own_severity`] / [`tests::a_relay_to_a_stopped_upstream_is_not_a_warning`] |
/// | [`LmStudioClient::complete`]（本文の読み取り） | `Speak` | [`tests::a_relay_cut_off_mid_body_names_its_own_severity`] |
/// | [`map_event_error`]（SSE 中継中） | `Speak` | [`tests::a_stream_cut_off_mid_relay_names_its_own_severity`] |
///
/// **⚠ 実害は「完全な沈黙」ではない**（是正のときに測って言い直した）。
/// 本文の中継の [`LlmError`] は全経路が `server::chat` の
/// **`upstream_error` を通り、そこで `error!` が出る。**
/// **消えるのはこの層の1行だけ**——`reqwest` が言った中身
/// （**どの綴りへ・どう失敗したか**）を持っているのはそちらである。
///
/// [`tests::a_best_effort_query_names_its_failure_without_warning`]:
///     tests::a_best_effort_query_names_its_failure_without_warning
/// [`tests::a_stopped_upstream_is_not_a_warning`]: tests::a_stopped_upstream_is_not_a_warning
/// [`tests::the_hushed_reader_still_speaks_at_debug`]:
///     tests::the_hushed_reader_still_speaks_at_debug
/// [`tests::neither_reader_answering_is_warned_with_both_paths_on_one_line`]:
///     tests::neither_reader_answering_is_warned_with_both_paths_on_one_line
/// [`tests::a_relay_that_never_gets_an_answer_names_its_own_severity`]:
///     tests::a_relay_that_never_gets_an_answer_names_its_own_severity
/// [`tests::a_relay_cut_off_mid_body_names_its_own_severity`]:
///     tests::a_relay_cut_off_mid_body_names_its_own_severity
/// [`tests::a_stream_cut_off_mid_relay_names_its_own_severity`]:
///     tests::a_stream_cut_off_mid_relay_names_its_own_severity
/// [`tests::a_relay_to_a_stopped_upstream_is_not_a_warning`]:
///     tests::a_relay_to_a_stopped_upstream_is_not_a_warning
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestFailureVoice {
    /// **その場で深刻度を名乗る**（接続失敗は `info`、それ以外は `warn`）。
    ///
    /// **⚠ 接続失敗が `info` である側の見張り番は
    /// [`tests::a_relay_to_a_stopped_upstream_is_not_a_warning`] だけ**
    /// （G-8/G-9 のレビューの是正 / 2026-09-07）。
    /// **[`Murmur`](Self::Murmur) と同じ腕に居ることは押さえではない**
    /// ——理由は [`map_request_error`] の doc の訂正の節。
    ///
    /// [`tests::a_relay_to_a_stopped_upstream_is_not_a_warning`]:
    ///     tests::a_relay_to_a_stopped_upstream_is_not_a_warning
    Speak,
    /// **名乗るが、警告にはしない**（3腕とも `info` / 作業単位 G-8 ①）。
    ///
    /// **失敗しても何も止まらない往復**のための声。
    /// コンテキスト長の照会は「取れたら得をする」経路（`server::context` のモジュール doc）で、
    /// **失われるのは上流の実測値だけ**であり、**その結果は呼び出し側が必ず言う**
    /// （起動時は `choose_budget` / `choose_sub_context` が `warn`、
    /// 5分ごとの引き直しは `refresh_once` / `refresh_sub_once` が `info`）。
    ///
    /// **⚠ [`Hush`](Self::Hush) との違いは「消えるかどうか」。**
    /// **既定のフィルタは `personacore=info`** なので、`Murmur` の1行は運用者に届く。
    /// `reqwest` が言った中身（**どの綴りへ・どう失敗したか**）を持っているのはこの行だけで、
    /// 呼び出し側の `info` は [`LlmError`] の定数文字列しか持たない。**落とすと綴りが消える。**
    ///
    /// **⚠ 3腕をまとめて `info` にすること。** 着手前は接続失敗だけが `info` で、
    /// **同じ「上流が答えない」が、断られ方で `info` と `warn` に割れていた**
    /// （**拒否されれば `Connect`、接続だけ受けて黙れば `Timeout`** / 実測）。
    /// 見張り番は [`tests::a_best_effort_query_names_its_failure_without_warning`]。
    ///
    /// [`tests::a_best_effort_query_names_its_failure_without_warning`]:
    ///     tests::a_best_effort_query_names_its_failure_without_warning
    Murmur,
    /// **`debug` へ落とす。** 深刻度は [`LmStudioClient::read_compat_models`] が決める。
    ///
    /// **握り潰すのではない**——`RUST_LOG=personacore=debug` なら、
    /// `reqwest` が言った中身（`Speak` と同じ文面・同じ詳細）がそのまま出る。
    ///
    /// **⚠ この主張は3腕ぶんある**（接続失敗・タイムアウト・通信失敗）。
    /// **押さえも3腕要る**（CLAUDE.md / 作業単位 D-6「写しを作ったら、
    /// 列・分岐・枝の1つずつに当てること。代表1つでは足りない」）——
    /// 3周目は接続失敗の1腕しか回しておらず、
    /// **残り2腕を `trace!` へ落とす変異が 1949 全緑**だった。
    /// **どちらの腕も現に通っている。通っているのに、見ていなかった**
    /// （「分岐に入った」と「入ったことを見た」は別）。
    /// 見張り番は [`tests::the_hushed_reader_still_speaks_at_debug`]。
    Hush,
}

/// `reqwest` の失敗を型に落とす。**接続失敗を他と混ぜない。**
///
/// 深刻度は [`RequestFailureVoice`]（呼び出し側が決める / 作業単位 G-5 の是正）。
/// **分類の枝は1本のまま**にしてある——水準ごとに枝を割ると、
/// **分類の権威が2箇所に分かれて片方が観測されなくなる**（CLAUDE.md / 作業単位 B-6）。
///
/// # 8つの水準の、1つずつの見張り番（作業単位 G-8 ① で1列足した / 2026-09-07）
///
/// | 分類 | `Speak` | `Murmur` | `Hush` |
/// |---|---|---|---|
/// | 接続失敗 | `info` / [`tests::a_relay_to_a_stopped_upstream_is_not_a_warning`] | `info` / [`tests::a_stopped_upstream_is_not_a_warning`] | `debug` / [`tests::the_hushed_reader_still_speaks_at_debug`] |
/// | タイムアウト | `warn` / [`tests::a_relay_that_never_gets_an_answer_names_its_own_severity`] | `info` / [`tests::a_best_effort_query_names_its_failure_without_warning`] | `debug` / 同上 |
/// | 通信失敗 | `warn` / [`tests::a_relay_cut_off_mid_body_names_its_own_severity`] | `info` / 同上 | `debug` / 同上 |
///
/// # ⚠ 接続失敗の `Speak` と `Murmur` は同じ腕だが、それは押さえではない（**訂正**）
///
/// **2026-09-07 まで、ここには「腕を1本にまとめてあるので、水準を上げる変異は
/// [`tests::a_stopped_upstream_is_not_a_warning`] が必ず赤にする」と書いてあった。
/// 偽だった。**
///
/// あのテストは [`LmStudioClient::models`] を通る。**その口は G-8 ① で
/// [`RequestFailureVoice::Speak`] から [`RequestFailureVoice::Murmur`] へ移っている**ので、
/// **あのテストが押さえているのは `Murmur` の列だけ**である。
/// **実測: 腕を割って `Speak` だけ `warn!` へ上げる変異が passed=1957 failed=0 / exit=0**
/// （レビュアーの `m8` / 同じ変異を着手前の版に当てると 1 赤）。
///
/// **列ごとに、その列を実際に通る見張り番が要る**
/// （CLAUDE.md / 作業単位 D-6「代表1つでは足りない」）。
/// **`Speak` の接続失敗を通すのは
/// [`tests::a_relay_to_a_stopped_upstream_is_not_a_warning`] だけ**
/// （停止した上流へ本文を中継する筋書き / `tests::RelayScript::NeverListens`）。
///
/// **腕を1本のままにしてよい理由は「観測できるから」ではなく「同じ行を出すから」。**
/// **割っても構わないが、割った先の水準を見る者は上の表の列にしか居ない。**
///
/// **`Hush` の3つは既定のフィルタでは元から出ない側**なので、
/// **否定側の見張り番には一切掛からない**（水準を下げる変異も、行ごと消す変異も通る）。
/// 肯定側を [`crate::testlog::DEBUG_FILTER`] で回すのが唯一の押さえである。
///
/// **`Murmur` の3つは既定のフィルタに出る側**（`personacore=info`）なので、
/// **肯定側も否定側も同じ1回の捕捉で見られる。**
///
/// [`tests::a_stopped_upstream_is_not_a_warning`]: tests::a_stopped_upstream_is_not_a_warning
/// [`tests::the_hushed_reader_still_speaks_at_debug`]:
///     tests::the_hushed_reader_still_speaks_at_debug
/// [`tests::a_best_effort_query_names_its_failure_without_warning`]:
///     tests::a_best_effort_query_names_its_failure_without_warning
/// [`tests::a_relay_that_never_gets_an_answer_names_its_own_severity`]:
///     tests::a_relay_that_never_gets_an_answer_names_its_own_severity
/// [`tests::a_relay_cut_off_mid_body_names_its_own_severity`]:
///     tests::a_relay_cut_off_mid_body_names_its_own_severity
/// [`tests::a_relay_to_a_stopped_upstream_is_not_a_warning`]:
///     tests::a_relay_to_a_stopped_upstream_is_not_a_warning
fn map_request_error(err: reqwest::Error, voice: RequestFailureVoice) -> LlmError {
    // **宛先のクエリ・フラグメントを、行に出す前・`LlmError` に包む前に落とす**（作業単位 B-71 /
    // `request::error_for_log`）。ここが送信の失敗の入口なので、ここで落とせば以降の全部に効く。
    let err = error_for_log(err);
    // **承認の無い送信を名前解決が止めた**（作業単位 A-13b-2）。`reqwest` はこれを接続の失敗に包むので、
    // **`is_connect` より先に見る**（後に置くと「推論サーバへ接続できませんでした」に化け、
    // 直す場所が鍵の承認であることが応答から消える）。
    if carries_key_not_approved(&err) {
        tracing::debug!(error = %err, "名前解決の段で送信を止めました");
        return key_not_approved(voice);
    }
    if err.is_connect() {
        // LM Studio は普段停止している。異常ではなく想定された分岐。
        match voice {
            // **接続失敗は `Speak` でも `info`。** LM Studio は普段停止している。
            //
            // **⚠ 2つの列を、別々の見張り番が見ている**（この関数の doc の表）——
            // `Speak` は `tests::a_relay_to_a_stopped_upstream_is_not_a_warning`、
            // `Murmur` は `tests::a_stopped_upstream_is_not_a_warning`。
            // **実測（2026-09-07 / 全体は 1958 本）:**
            // **腕を割って `Speak` だけを `warn!` へ上げると 1 赤**（前者 / `m8`）。
            // **`Murmur` だけを上げると 2 赤**——後者に加えて
            // `tests::a_best_effort_query_names_its_failure_without_warning` も鳴る（`m14`）。
            // **`Speak` だけを `debug!` へ落としても前者が 1 赤**（`m13`）。
            // **腕を1本にまとめてあることは押さえではない**——理由はこの関数の doc の訂正の節。
            RequestFailureVoice::Speak | RequestFailureVoice::Murmur => {
                tracing::info!(error = %err, "推論サーバへ接続できませんでした");
            }
            RequestFailureVoice::Hush => {
                tracing::debug!(error = %err, "推論サーバへ接続できませんでした");
            }
        }
        LlmError::Connect(err)
    } else if err.is_timeout() {
        match voice {
            RequestFailureVoice::Speak => {
                tracing::warn!(error = %err, "推論サーバの応答が時間内に完了しませんでした");
            }
            RequestFailureVoice::Murmur => {
                tracing::info!(error = %err, "推論サーバの応答が時間内に完了しませんでした");
            }
            RequestFailureVoice::Hush => {
                tracing::debug!(error = %err, "推論サーバの応答が時間内に完了しませんでした");
            }
        }
        LlmError::Timeout(err)
    } else {
        match voice {
            RequestFailureVoice::Speak => {
                tracing::warn!(error = %err, "推論サーバとの通信に失敗しました");
            }
            RequestFailureVoice::Murmur => {
                tracing::info!(error = %err, "推論サーバとの通信に失敗しました");
            }
            RequestFailureVoice::Hush => {
                tracing::debug!(error = %err, "推論サーバとの通信に失敗しました");
            }
        }
        LlmError::Transport(err)
    }
}

/// **承認が無いので送らなかった**（作業単位 A-13b-2）。行の深刻度は `voice` に従う
/// （[`map_request_error`] の表と同じ —— 本文は `warn`、裏方の照会と数える口は `info`、2つ目の読み手は `debug`）。
///
/// **どのアドレスが外だったかは、判定が変わったときの行**（[`KeyGuard::record`]）が言う。
fn key_not_approved(voice: RequestFailureVoice) -> LlmError {
    const LINE: &str = "鍵を平文の http:// でローカルネットワークの外へ送る承認が無いので、推論サーバへ送りませんでした";
    match voice {
        RequestFailureVoice::Speak => tracing::warn!("{LINE}"),
        RequestFailureVoice::Murmur => tracing::info!("{LINE}"),
        RequestFailureVoice::Hush => tracing::debug!("{LINE}"),
    }
    LlmError::KeyNotApproved
}

/// SSE 層の失敗を型に落とす。
///
/// **通信の失敗を [`RequestFailureVoice::Hush`] にしないこと**（4周目の是正 / 2026-09-06）。
/// ここは**本文を中継している最中**で、深刻度を決められる側が他に居ない。
/// 見張り番は [`tests::a_stream_cut_off_mid_relay_names_its_own_severity`]。
///
/// [`tests::a_stream_cut_off_mid_relay_names_its_own_severity`]:
///     tests::a_stream_cut_off_mid_relay_names_its_own_severity
fn map_event_error(err: EventStreamError<reqwest::Error>) -> LlmError {
    match err {
        // 本文の中継の途中なので、**深刻度を決められる側は他に居ない。**
        EventStreamError::Transport(err) => map_request_error(err, RequestFailureVoice::Speak),
        // 詳細（不正なバイト列・崩れた行）は Display に載せられないのでログへ。
        EventStreamError::Utf8(err) => {
            tracing::warn!(error = %err, "上流のストリームが UTF-8 として不正です");
            LlmError::MalformedEvent
        }
        EventStreamError::Parser(err) => {
            tracing::warn!(error = ?err, "上流のストリームの書式が不正です");
            LlmError::MalformedEvent
        }
    }
}

/// エラー応答の本文を**ログ用に**切り詰めて取り出す。取れなければ空。
///
/// **鍵を伏せてから切り詰める**（ADR-0056 §決定3 / [`redact_key`]）——上流が鍵を写し返しても、
/// ログに出さない。**順を逆にしない**（切ってから伏せると、境目で切れた鍵の断片が残る）。
async fn error_snippet(response: reqwest::Response, key: Option<&ApiKey>) -> String {
    match response.text().await {
        Ok(body) => redact_key(&body, key)
            .chars()
            .take(ERROR_BODY_LOG_LIMIT)
            .collect(),
        Err(err) => {
            // **宛先のクエリ・フラグメントを落としてから出す**（作業単位 B-71 / `request::error_for_log`）。
            tracing::debug!(error = %error_for_log(err), "エラー応答の本文を読めませんでした");
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::string::FromUtf8Error;

    use super::*;
    use crate::testlog::{DEBUG_FILTER, has_field, line_containing, quoted_field};

    fn model() -> ModelId {
        ModelId::parse(DEFAULT_MAIN_MODEL).expect("既定モデルは正当であること")
    }

    /// SSE イベントを1つ作る。
    fn event(data: &str) -> Result<Event, EventStreamError<reqwest::Error>> {
        Ok(Event {
            event: "message".to_owned(),
            data: data.to_owned(),
            id: String::new(),
            retry: None,
        })
    }

    /// UTF-8 として不正なバイト列から作った失敗（上流のフレーミング破損の代役）。
    fn utf8_failure() -> Result<Event, EventStreamError<reqwest::Error>> {
        let err: FromUtf8Error = String::from_utf8(vec![0xff]).expect_err("不正なバイト列");
        Err(EventStreamError::Utf8(err))
    }

    /// 増分を1つ載せたイベント。
    fn delta(content: &str) -> Result<Event, EventStreamError<reqwest::Error>> {
        event(&format!(
            r#"{{"choices":[{{"delta":{{"content":"{content}"}}}}]}}"#
        ))
    }

    /// 与えたイベント列を最後まで流す。
    async fn collect(
        events: Vec<Result<Event, EventStreamError<reqwest::Error>>>,
    ) -> Vec<Result<ChatChunk, LlmError>> {
        chunk_stream(stream::iter(events)).collect().await
    }

    /// ベース URL は末尾の `/` を吸収して補完エンドポイントになる。
    ///
    /// **検証と組み立てを分けた**（`normalize_base` ＋ `LmStudioConfig::new`）ので、
    /// 組み立て結果は設定から観測する。見ている入力と期待値は分ける前と同じ。
    #[test]
    fn normalizes_the_endpoint() {
        for base in [
            "http://127.0.0.1:1234",
            "http://127.0.0.1:1234/",
            "http://127.0.0.1:1234///",
            "  http://127.0.0.1:1234  ",
        ] {
            assert_eq!(
                LmStudioConfig::new(base, model())
                    .expect("ベース URL が正当であること")
                    .endpoint(),
                "http://127.0.0.1:1234/v1/chat/completions",
                "base = {base:?}"
            );
        }
        assert_eq!(
            LmStudioConfig::new("https://example.test", model())
                .expect("ベース URL が正当であること")
                .endpoint(),
            "https://example.test/v1/chat/completions"
        );
    }

    /// 受け付けられないベース URL は、理由を区別できる形で拒否する。
    #[test]
    fn rejects_unusable_endpoints() {
        assert_eq!(normalize_base(""), Err(EndpointError::Empty));
        assert_eq!(normalize_base("   "), Err(EndpointError::Empty));
        assert_eq!(
            normalize_base("127.0.0.1:1234"),
            Err(EndpointError::UnsupportedScheme)
        );
        assert_eq!(
            normalize_base("ftp://127.0.0.1"),
            Err(EndpointError::UnsupportedScheme)
        );
        assert_eq!(
            normalize_base("file:///etc/passwd"),
            Err(EndpointError::UnsupportedScheme)
        );
        assert_eq!(normalize_base("http://"), Err(EndpointError::MissingHost));
        assert_eq!(
            normalize_base("http:///v1"),
            Err(EndpointError::MissingHost)
        );
        assert_eq!(
            normalize_base("http://127.0.0.1 :1234"),
            Err(EndpointError::Malformed)
        );
        assert_eq!(
            normalize_base("http://127.0.0.1\n:1234"),
            Err(EndpointError::Malformed)
        );

        // **どちらのエンドポイントも作られない**（片方だけ検証を素通りしないこと）。
        assert!(LmStudioConfig::new("127.0.0.1:1234", model()).is_err());
    }

    /// **末尾が `/v1` の宛先の判定が、画面と同じ表で一致する**（作業単位 B-68 / Issue origin#97）。
    ///
    /// 画面（`web/src/lib/upstreamProfiles.ts` の `endpointEndsWithV1`）は保存の前に止めるために、
    /// この関数と同じ判定を TypeScript で持つ（写し）。**両側が同じ表**
    /// （`tests/fixtures/endpoint_v1_suffix_corpus.tsv`）**を読む**ので、ここを変えればこのテストが、
    /// 画面が追随していなければ `web/tests/upstream-profiles.test.ts` が赤になる。
    ///
    /// **表が痩せていない・2つの分類が両方出る**ことも見る（読めていないから緑、を通さない）。
    #[test]
    fn the_v1_suffix_corpus_matches_the_judgment() {
        const CORPUS: &str = include_str!("../../tests/fixtures/endpoint_v1_suffix_corpus.tsv");
        let mut seen = std::collections::BTreeSet::new();
        let mut rows = 0;
        for line in CORPUS.lines() {
            // **`\r` を落としてから割る**（作業ツリーは CRLF になりうる）。
            let line = line.trim_end_matches('\r');
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let (endpoint, expected) = line.split_once('\t').expect("宛先<TAB>分類 の形");
            let class = if endpoint_ends_with_v1(endpoint) {
                "v1"
            } else {
                "ok"
            };
            assert_eq!(class, expected, "{endpoint:?}");
            seen.insert(expected);
            rows += 1;
        }
        assert!(rows >= 30, "表が痩せている（{rows} 行）");
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), ["ok", "v1"]);
    }

    /// **末尾が `/v1` の宛先は受け付けたまま、既定のログフィルタで警告を1行出す**（作業単位 B-68）。
    ///
    /// * **受理集合は変えない** —— 組んだ宛先は今までどおり `…/v1/v1/chat/completions`
    ///   （既に `upstreams.json` にそう書いた利用者の起動を止めない）。
    /// * **警告の行に宛先は出すが、ユーザー情報（`user:pass@`）・クエリ・フラグメントは出さない**
    ///   （後の2つは作業単位 B-68 のレビュー是正。`?api_key=…` のような綴りがありうる）。
    /// * **止めない側の対照**（`/v1beta`・パスの前置き・末尾の `/` だけ）は警告しない。
    ///   否定側は、同じ捕捉の中で肯定側が出ていることを先に確かめてから見る（モジュール `testlog` の doc）。
    #[test]
    fn a_v1_suffixed_endpoint_is_accepted_and_warned_without_its_secrets() {
        const MARKER: &str = "宛先の末尾が /v1 です";
        let mut built = Vec::new();
        let output =
            crate::testlog::captured_with_filter(crate::server::DEFAULT_LOG_FILTER, || {
                for base in [
                    "http://127.0.0.1:46001/v1beta",
                    "http://user:secret@127.0.0.1:46001/v1/",
                    "https://example.com/api",
                    "https://example.org/relay/v1?token=secret2#frag-secret3",
                    "http://127.0.0.1:46001/",
                ] {
                    built.push(
                        LmStudioConfig::new(base, model())
                            .expect("末尾が /v1 でも受け付けること")
                            .endpoint()
                            .to_owned(),
                    );
                }
            });
        assert_eq!(
            built,
            [
                "http://127.0.0.1:46001/v1beta/v1/chat/completions",
                "http://user:secret@127.0.0.1:46001/v1/v1/chat/completions",
                "https://example.com/api/v1/chat/completions",
                "https://example.org/relay/v1?token=secret2#frag-secret3/v1/chat/completions",
                "http://127.0.0.1:46001/v1/chat/completions",
            ]
        );
        let warned: Vec<&str> = output.lines().filter(|l| l.contains(MARKER)).collect();
        assert_eq!(warned.len(), 2, "警告は /v1 の宛先ごとに1行: {output}");
        // **ユーザー情報の腕**（`user:secret@` を落とす）。
        let line = line_containing(&output, "http://127.0.0.1:46001/v1");
        assert!(line.contains(MARKER), "{line}");
        assert!(line.contains("WARN"), "{line}");
        assert!(!line.contains("secret"), "{line}");
        assert!(!line.contains("user"), "{line}");
        // **クエリとフラグメントの腕**（作業単位 B-68 のレビュー是正 —— `?api_key=…` のような綴りを出さない）。
        let line = line_containing(&output, "https://example.org/relay/v1");
        assert!(line.contains(MARKER), "{line}");
        assert!(line.contains("WARN"), "{line}");
        assert!(!line.contains("secret2"), "{line}");
        assert!(!line.contains("token"), "{line}");
        assert!(!line.contains("secret3"), "{line}");
    }

    /// 設定はエンドポイントとモデルを保持する。
    #[test]
    fn config_keeps_endpoint_and_model() {
        let config =
            LmStudioConfig::new(DEFAULT_ENDPOINT, model()).expect("既定値は正当であること");
        assert_eq!(
            config.endpoint(),
            "http://127.0.0.1:1234/v1/chat/completions"
        );
        assert_eq!(config.model().as_str(), DEFAULT_MAIN_MODEL);
    }

    /// **コンテキスト長の照会先は同じベース URL の独自 API。**
    ///
    /// 末尾の `/` の扱いが補完エンドポイントと食い違わないこと（`//api/v0/models`
    /// になると 404 になり、**取れないのに「上流が持っていない」と読める**）。
    ///
    /// **既定と違うベースを必ず1つ混ぜること。** `127.0.0.1:1234` の表記違いだけだと、
    /// 「ベース URL から導出している」と「既定にハードコードされている」を
    /// **原理的に区別できない**（`normalizes_the_endpoint` と同じ形に揃えてある）。
    #[test]
    fn config_derives_the_native_models_endpoint_from_the_same_base() {
        for (base, expected) in [
            (
                "http://127.0.0.1:1234",
                "http://127.0.0.1:1234/api/v0/models",
            ),
            (
                "http://127.0.0.1:1234/",
                "http://127.0.0.1:1234/api/v0/models",
            ),
            (
                "http://127.0.0.1:1234///",
                "http://127.0.0.1:1234/api/v0/models",
            ),
            (
                "  http://127.0.0.1:1234  ",
                "http://127.0.0.1:1234/api/v0/models",
            ),
            // **既定とは別のベース。** ここが無いと導出とハードコードが区別できない。
            ("https://example.test", "https://example.test/api/v0/models"),
        ] {
            let config = LmStudioConfig::new(base, model()).expect("ベース URL が正当であること");
            assert_eq!(config.models_endpoint(), expected, "base = {base:?}");
            // OpenAI 互換の `/v1/models` ではない（あちらは長さを返さない）。
            assert!(
                !config.models_endpoint().contains("/v1/models"),
                "OpenAI 互換の一覧はコンテキスト長を返さない: {}",
                config.models_endpoint()
            );
        }
    }

    /// **2つ目の照会先も、同じベース URL から導く**（作業単位 G-2）。
    ///
    /// 末尾の `/` の扱いが他の2つと食い違うと `//v1/models` になり、
    /// **上流は 404 を返す**——**「答えられない上流だ」と読める失敗**になる。
    ///
    /// **既定と違うベースを必ず1つ混ぜること**（導出とハードコードの区別 /
    /// [`config_derives_the_native_models_endpoint_from_the_same_base`] と同じ形）。
    #[test]
    fn config_derives_the_openai_models_endpoint_from_the_same_base() {
        for (base, expected) in [
            ("http://127.0.0.1:1234", "http://127.0.0.1:1234/v1/models"),
            ("http://127.0.0.1:1234/", "http://127.0.0.1:1234/v1/models"),
            (
                "http://127.0.0.1:1234///",
                "http://127.0.0.1:1234/v1/models",
            ),
            (
                "  http://127.0.0.1:1234  ",
                "http://127.0.0.1:1234/v1/models",
            ),
            // **既定とは別のベース。** ここが無いと導出とハードコードが区別できない。
            ("https://example.test", "https://example.test/v1/models"),
        ] {
            let config = LmStudioConfig::new(base, model()).expect("ベース URL が正当であること");
            assert_eq!(config.compat_models_endpoint(), expected, "base = {base:?}");
            // **3つは別の宛先。** 取り違えると、補完の宛先へ GET を投げることになる。
            assert_ne!(config.compat_models_endpoint(), config.models_endpoint());
            assert_ne!(config.compat_models_endpoint(), config.endpoint());
        }
    }

    /// **照会の上限は起動を止めない長さ。** 本文生成の上限を引き継がないこと。
    ///
    /// ここが 180〜300 秒側に揃うと、黙り込んだ上流1台で
    /// **サーバが数分間起動しなくなる**（待ち受けも DB 接続も照会の後ろにある）。
    #[test]
    fn the_context_query_does_not_inherit_the_generation_timeouts() {
        // **既定の設定が実際に掴む値を見る。** 定数同士の比較だけだと、
        // `LmStudioConfig::new` が `DEFAULT_REQUEST_TIMEOUT`（300秒）を掴むように
        // 変わっても気付けない（`with_context_timeout` で上書きしてから測る
        // `a_slow_native_api_is_bounded_by_its_own_timeout` も既定を通らない）。
        assert_eq!(
            LmStudioConfig::new(DEFAULT_ENDPOINT, model())
                .expect("既定値は正当であること")
                .context_timeout,
            DEFAULT_CONTEXT_TIMEOUT,
            "既定の設定は照会専用の上限を掴むこと（本文生成の上限を引き継がない）"
        );

        assert!(
            DEFAULT_CONTEXT_TIMEOUT < DEFAULT_IDLE_TIMEOUT,
            "照会の上限（{DEFAULT_CONTEXT_TIMEOUT:?}）は無通信の上限\
             （{DEFAULT_IDLE_TIMEOUT:?}）より十分短いこと"
        );
        assert!(
            DEFAULT_CONTEXT_TIMEOUT <= Duration::from_secs(10),
            "起動を待たせない長さであること: {DEFAULT_CONTEXT_TIMEOUT:?}"
        );
    }

    /// **モデルは差し替えられる。** 接続先と上限はそのまま。
    ///
    /// # `DEFAULT_SUB_MODEL` を使わない理由
    ///
    /// **既定どうしが同じ綴りになった**（ADR-0021 §10）。差し替え先に
    /// `DEFAULT_SUB_MODEL` を渡すと、元の値（`DEFAULT_MAIN_MODEL`）と一致するので
    /// **引数を無視して自分のモデルを使い回す実装が緑を通る**（実測: 既定を揃えた直後に
    /// その変異を当てると、赤はこの見張り番を除いた
    /// `the_sub_model_reaches_the_upstream_through_the_real_bind` の1本だけになった）。
    /// 差し替え先には**既定のどちらとも違う綴り**を与える。
    #[test]
    fn with_model_only_replaces_the_model() {
        /// 差し替え先。**既定の本文モデルとも既定の抽出モデルとも別の綴り。**
        const OTHER_MODEL: &str = "ministral-3-3b-reasoning-2512";

        assert_ne!(
            OTHER_MODEL, DEFAULT_MAIN_MODEL,
            "前提: 差し替え先は元のモデルと別の綴りであること"
        );
        assert_ne!(
            OTHER_MODEL, DEFAULT_SUB_MODEL,
            "前提: 差し替え先は既定の抽出モデルとも別の綴りであること"
        );

        let config = LmStudioConfig::new(DEFAULT_ENDPOINT, model())
            .expect("既定値は正当であること")
            .with_request_timeout(Duration::from_secs(7));
        let client = LmStudioClient::new(config).expect("構築できること");

        let sub = ModelId::parse(OTHER_MODEL).expect("正当であること");
        let switched = client.with_model(sub);

        assert_eq!(switched.config().model().as_str(), OTHER_MODEL);
        assert_eq!(switched.config().endpoint(), client.config().endpoint());
        assert_eq!(switched.config().request_timeout, Duration::from_secs(7));
        assert_eq!(client.config().model().as_str(), DEFAULT_MAIN_MODEL);
    }

    /// **既定値の役割分担を固定する。**
    ///
    /// `request_timeout` は応答ヘッダ待ちだけでなく**本文の読み取りにも引き継がれる**。
    /// したがって `request_timeout` が `idle_timeout` を下回ると、
    /// **到達不能になるのは全体上限ではなく `idle_timeout` の方**で、
    /// JIT ロード待ち（8.24GB）に割いたはずの予算が全体上限に切り詰められる。
    ///
    /// **この不変条件が破れていた実例は今のところ無い。** 旧既定（`request` 180秒 /
    /// `idle` 120秒）も `request >= idle` を満たしていた。当時の問題は不変条件の
    /// 違反ではなく **`idle` の絶対値が JIT ロードに足りなかった**ことで、
    /// `idle` を 180秒へ、あわせて `request` を 300秒へ引き上げて解消している。
    /// **片方だけを縮めると黙って破れる**関係なので、ここで固定する。
    #[test]
    fn defaults_keep_the_total_deadline_outside_the_read_budget() {
        assert!(
            DEFAULT_REQUEST_TIMEOUT >= DEFAULT_IDLE_TIMEOUT,
            "全体上限（{DEFAULT_REQUEST_TIMEOUT:?}）が無通信の上限（{DEFAULT_IDLE_TIMEOUT:?}）を\
             下回ると、無通信の上限の方が到達不能になり、JIT ロード待ちの予算が切り詰められる"
        );
        assert!(
            DEFAULT_CONNECT_TIMEOUT < DEFAULT_IDLE_TIMEOUT,
            "接続の上限は無通信の上限より短いこと（接続の失敗は即座に分かる）"
        );
    }

    /// **天井を1つ動かすと、残りの3つが導出でついてくる**（作業単位 B-6 / ADR-0036 §決定3）。
    ///
    /// つまみは [`LmStudioConfig::with_idle_budget`] の1つだけで、
    /// `request` / `context` / `connect` は**そこから導く。**
    /// **2つ目のつまみを足さないための仕掛け**なので、
    /// 「導出をやめて既定を掴む」変異はここで落ちる。
    ///
    /// # 何を確かめるか
    ///
    /// **既存の不変条件が、どの天井でも保たれること:**
    /// `connect < idle`、`context < idle`、`idle <= request`。
    ///
    /// **格子は極端まで降りる**——1秒（照会の既定 5 秒より短い）と、
    /// `u64::MAX` 秒（加算が溢れる形）を必ず含める。
    #[test]
    fn one_knob_derives_every_other_deadline() {
        for secs in [1_u64, 2, 5, 9, 10, 11, 30, 180, 3_600, 86_400, u64::MAX] {
            let idle = Duration::from_secs(secs);
            let config = LmStudioConfig::new(DEFAULT_ENDPOINT, model())
                .expect("既定値は正当であること")
                .with_idle_budget(idle);

            assert_eq!(
                config.idle_timeout(),
                idle,
                "天井そのものは渡した値: {idle:?}"
            );
            assert!(
                config.context_timeout() < config.idle_timeout(),
                "照会の上限（{:?}）は天井（{idle:?}）より必ず短いこと",
                config.context_timeout()
            );
            assert!(
                config.connect_timeout() < config.idle_timeout(),
                "接続の上限（{:?}）は天井（{idle:?}）より必ず短いこと",
                config.connect_timeout()
            );
            assert!(
                config.request_timeout() >= config.idle_timeout(),
                "全体上限（{:?}）が天井（{idle:?}）を下回ると、天井の方が到達不能になる",
                config.request_timeout()
            );
            assert!(
                config.context_timeout() <= DEFAULT_CONTEXT_TIMEOUT,
                "照会の上限を既定より伸ばさないこと（起動を待たせない / {:?}）",
                config.context_timeout()
            );
        }
    }

    /// **導出は、既定の天井を渡すと出荷している既定値をそのまま再現する。**
    ///
    /// # なぜこれが要るのか（B-7 の穴）
    ///
    /// 導出が既定と食い違っていると、**つまみを触っていない利用者の振る舞いが
    /// 黙って変わる。** しかも既定で回る見張り番はそれを見ない
    /// （CLAUDE.md「実行時に決まる値を入れたら、既定と違う状態で回る腕を必ず1本置く」——
    /// **その腕は上の [`one_knob_derives_every_other_deadline`] と
    /// `tests/server_llm_idle_timeout.rs`**）。
    ///
    /// **期待値は導出の式から作らない。** 出荷している4つの定数と直に比べる。
    #[test]
    fn the_default_ceiling_reproduces_the_shipped_defaults() {
        let derived = LmStudioConfig::new(DEFAULT_ENDPOINT, model())
            .expect("既定値は正当であること")
            .with_idle_budget(DEFAULT_IDLE_TIMEOUT);

        assert_eq!(derived.idle_timeout(), DEFAULT_IDLE_TIMEOUT);
        assert_eq!(
            derived.request_timeout(),
            DEFAULT_REQUEST_TIMEOUT,
            "既定の天井からは、出荷している全体上限がそのまま出ること"
        );
        assert_eq!(
            derived.context_timeout(),
            DEFAULT_CONTEXT_TIMEOUT,
            "既定の天井からは、出荷している照会の上限がそのまま出ること"
        );
        assert_eq!(
            derived.connect_timeout(),
            DEFAULT_CONNECT_TIMEOUT,
            "既定の天井からは、出荷している接続の上限がそのまま出ること"
        );
    }

    /// **天井を縮めると、照会と接続も一緒に縮む。**
    ///
    /// [`the_default_ceiling_reproduces_the_shipped_defaults`] だけだと
    /// 「4つとも既定を返すだけ」の実装が緑を通る（**式が文字どおり同じになる** / B-7）。
    /// **既定より短い天井**で、実際に別の値が出ることをここで見る。
    #[test]
    fn a_short_ceiling_pulls_the_query_and_connect_deadlines_down_with_it() {
        /// 既定の照会の上限（5秒）より短い天井。**そこが分かれ目。**
        const TIGHT: Duration = Duration::from_secs(4);

        assert!(
            TIGHT < DEFAULT_CONTEXT_TIMEOUT,
            "前提: 既定の照会の上限より短い天井であること（{TIGHT:?} / {DEFAULT_CONTEXT_TIMEOUT:?}）"
        );

        let derived = LmStudioConfig::new(DEFAULT_ENDPOINT, model())
            .expect("既定値は正当であること")
            .with_idle_budget(TIGHT);

        assert_eq!(derived.idle_timeout(), TIGHT);
        assert_ne!(
            derived.context_timeout(),
            DEFAULT_CONTEXT_TIMEOUT,
            "天井が既定の照会の上限より短いのに、照会だけ既定のまま居座っている"
        );
        assert_ne!(
            derived.connect_timeout(),
            DEFAULT_CONNECT_TIMEOUT,
            "天井が既定の接続の上限より短いのに、接続だけ既定のまま居座っている"
        );
        assert_ne!(
            derived.request_timeout(),
            DEFAULT_REQUEST_TIMEOUT,
            "全体上限が天井についてきていない（導出ではなく既定を掴んでいる）"
        );
    }

    /// `[DONE]` で終端し、番兵そのものは下流へ出さない。
    #[tokio::test]
    async fn terminates_at_the_done_marker() {
        let items = collect(vec![
            delta("こん"),
            delta("にちは"),
            event("[DONE]"),
            // 番兵の後ろは読まない。
            delta("読まれない"),
        ])
        .await;

        assert_eq!(items.len(), 2, "[DONE] 以降は流さないこと: {items:?}");
        assert_eq!(items[0].as_ref().expect("正常").delta, "こん");
        assert_eq!(items[1].as_ref().expect("正常").delta, "にちは");
    }

    /// **終端した後にもう一度読んでも panic しない。**
    ///
    /// `stream::unfold` は素のままだと終端後の poll で panic する。下流が
    /// 一度でも余分に poll すれば本番で落ちるので、`fuse` を外さないこと。
    #[tokio::test]
    async fn polling_after_termination_is_safe() {
        let mut chunks = Box::pin(chunk_stream(stream::iter(vec![
            delta("こん"),
            event("[DONE]"),
        ])));

        assert!(chunks.next().await.is_some(), "1件目は流れること");
        assert!(chunks.next().await.is_none(), "[DONE] で終端すること");
        for _ in 0..3 {
            assert!(
                chunks.next().await.is_none(),
                "終端後は None のままであること"
            );
        }
    }

    /// **`[DONE]` を受けずに尽きたら `Truncated` を流して終端する。**
    #[tokio::test]
    async fn reports_truncation_when_the_upstream_ends_early() {
        let items = collect(vec![delta("こん")]).await;

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].as_ref().expect("正常").delta, "こん");
        assert!(
            matches!(items[1], Err(LlmError::Truncated)),
            "打ち切りとして区別されること: {:?}",
            items[1]
        );
    }

    /// 何も受け取らずに尽きた場合も終端する（宙吊りにしない）。
    #[tokio::test]
    async fn reports_truncation_for_an_empty_stream() {
        let items = collect(Vec::new()).await;
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0], Err(LlmError::Truncated)));
    }

    /// 失敗を受けたら1件流して終端し、**その後ろは読まない。**
    #[tokio::test]
    async fn stops_after_a_failure() {
        let items = collect(vec![delta("こん"), utf8_failure(), delta("読まれない")]).await;

        assert_eq!(items.len(), 2, "失敗の後ろは流さないこと: {items:?}");
        assert_eq!(items[0].as_ref().expect("正常").delta, "こん");
        assert!(
            matches!(items[1], Err(LlmError::MalformedEvent)),
            "書式の破損として区別されること: {:?}",
            items[1]
        );
    }

    /// 中身の無いチャンクは読み飛ばし、**ストリームは止めない。**
    #[tokio::test]
    async fn skips_chunks_without_choices() {
        let items = collect(vec![
            event(r#"{"choices":[]}"#),
            delta("こん"),
            event(r#"{"usage":{"total_tokens":9}}"#),
            event("[DONE]"),
        ])
        .await;

        assert_eq!(items.len(), 1, "読み飛ばした分は出さないこと: {items:?}");
        assert_eq!(items[0].as_ref().expect("正常").delta, "こん");
    }

    /// 壊れたチャンクはエラーとして流れ、そこで終端する（panic しない）。
    #[tokio::test]
    async fn broken_chunk_becomes_an_error_item() {
        let items = collect(vec![event("{\"choices\":"), event("[DONE]")]).await;

        assert_eq!(items.len(), 1);
        assert!(
            matches!(items[0], Err(LlmError::Decode(_))),
            "JSON の破損として区別されること: {:?}",
            items[0]
        );
    }

    // ------------------------------- 照会の深刻度（作業単位 G-5）
    //
    // **`llama-server` を上流にすると、1つ目の読み手は必ず 404 を返す**
    // （`/api/v0/models` を持たない / G-2 が実測 / 2026-09-04）。
    // **そして2つ目が成功する。** つまり **404 は正常な経路の一部**である。
    //
    // 着手前はここが `warn!` だったので、**起動のたびに2本、5分ごとに2本**鳴っていた
    // （G-2 / G-6 が実測 / 2026-09-05）。
    // **毎回鳴る警告は読まれなくなる**（`crate::measured` の doc）。
    //
    // **深刻度を決める材料は「1つ目が失敗したか」ではなく
    // 「どちらの読み手も何も返さなかったか」。** それを知っているのは
    // [`LmStudioClient::models`] だけなので、[`LmStudioClient::fetch_models`] から
    // 深刻度の決定を取り上げてある。

    /// 読み手1つぶんの失敗を告げる行の手掛かり。**既定のフィルタでは出ない側。**
    const ONE_READER_MARKER: &str = "モデル一覧にエラーを返しました";

    // **肯定側の先出しに使うフィルタは [`crate::testlog::DEBUG_FILTER`]。**
    // **G-8/G-9 の是正で引き上げた**——`llm::response::tests` に
    // **同じ値と同じ理由の写しが別に置かれていた**（CLAUDE.md「同じ発見を2箇所に書かない」）。
    // **`trace` で捕まえてはいけない理由も、値の綴りが `personacore=debug` である理由も、
    // 向こうの doc が1箇所で持っている。**

    /// **どちらの読み手も読めなかった**ことを告げる行の手掛かり。**既定で出る側。**
    const NEITHER_READER_MARKER: &str = "どちらの綴りでも読めませんでした";

    /// [`map_request_error`] のタイムアウトの行の手掛かり（**1つ目の読み手の側**）。
    const TIMED_OUT_MARKER: &str = "推論サーバの応答が時間内に完了しませんでした";

    /// [`map_request_error`] の接続失敗の行の手掛かり。**`warn` ではなく `info`。**
    const CONNECT_FAILED_MARKER: &str = "推論サーバへ接続できませんでした";

    /// [`map_request_error`] の通信失敗の行の手掛かり（**本文が途中で切れた側**）。
    const TRANSPORT_FAILED_MARKER: &str = "推論サーバとの通信に失敗しました";

    /// 2つ目の読み手が長さを答えたことを告げる行の手掛かり（作業単位 G-2）。
    ///
    /// **⚠ この文面は `tests/server_context_refresh.rs` が権威**（G-1 / G-2）。
    /// あちらは `sub_context_length=16384` が落とし所と同値になる腕を持っていて、
    /// **区別が付くのはこの文面だけ**である。**触らないこと。**
    const COMPAT_READ_MARKER: &str = "OpenAI 互換のモデル一覧から実効コンテキスト長を読みました";

    /// `llama-server` の `/v1/models` が申告する窓。
    ///
    /// **既定の予算（8192）とも落とし所（16384）とも違う数**にすること。
    const G5_COMPAT_N_CTX: u32 = 20_480;

    /// `llama-server` の `id`（**gguf の絶対パス** / 依頼する綴りとは一致しない）。
    const G5_COMPAT_MODEL_ID: &str = "/models/Ministral-3-14B-Instruct-2512-Q4_K_M.gguf";

    /// **現在のスレッドで走らせる runtime。** ログの捕捉はスレッドローカルなので、
    /// `#[tokio::test]` ではなく捕捉窓の内側で `block_on` する。
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime を作れること")
    }

    /// 実機の `llama-server` と同じ形の `/v1/models` の応答（`meta.n_ctx` を持つ）。
    fn g5_compat_body() -> String {
        format!(
            r#"{{"object":"list","data":[{{"id":"{G5_COMPAT_MODEL_ID}","object":"model",
               "owned_by":"llamacpp","meta":{{"n_ctx":{G5_COMPAT_N_CTX},"n_ctx_train":262144}}}}]}}"#
        )
    }

    /// **その行を出した層**（`… WARN personacore::llm::response: …` の真ん中）。
    ///
    /// 本数だけを数えると「誰が言ったか」を見ないことになる。
    fn emitter_of(line: &str) -> &str {
        let mut tokens = line.split_whitespace().skip_while(|token| *token != "WARN");
        assert_eq!(tokens.next(), Some("WARN"), "WARN の行だけを渡すこと");
        tokens.next().expect("水準の次に層の綴りが来ること")
    }

    /// 2つの読み手に別々の応答を積んだ上流を立て、`models()` を1回叩く。
    ///
    /// 捕捉窓の中で `block_on` するので、**出力はこの1回の往復のものだけ。**
    async fn mount_models(
        server: &wiremock::MockServer,
        path: &'static str,
        response: wiremock::ResponseTemplate,
    ) {
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(path))
            .respond_with(response)
            .mount(server)
            .await;
    }

    /// 上流を立てて `models()` を1回叩き、**指定したフィルタで捕捉した出力**を返す。
    fn query_models(
        filter: &str,
        native: wiremock::ResponseTemplate,
        compat: wiremock::ResponseTemplate,
    ) -> (String, Result<ModelsReport, LlmError>) {
        query_models_bounded(filter, native, compat, DEFAULT_CONTEXT_TIMEOUT)
    }

    /// 同じものを、**照会の上限を縮めて**叩く。
    ///
    /// **遅い上流の腕にはこちらを使う。** 既定（[`DEFAULT_CONTEXT_TIMEOUT`]）のままだと
    /// 1本につき 5 秒待つことになる。
    fn query_models_bounded(
        filter: &str,
        native: wiremock::ResponseTemplate,
        compat: wiremock::ResponseTemplate,
        context_timeout: Duration,
    ) -> (String, Result<ModelsReport, LlmError>) {
        let runtime = runtime();
        let mut outcome = None;
        let output = crate::testlog::captured_with_filter(filter, || {
            runtime.block_on(async {
                let server = wiremock::MockServer::start().await;
                mount_models(&server, NATIVE_MODELS_PATH, native).await;
                mount_models(&server, OPENAI_MODELS_PATH, compat).await;
                let client = LmStudioClient::new(
                    LmStudioConfig::new(&server.uri(), model())
                        .expect("ベース URL は正当")
                        .with_context_timeout(context_timeout),
                )
                .expect("クライアントを作れること");
                outcome = Some(client.models().await);
            });
        });
        (output, outcome.expect("往復が済んでいること"))
    }

    // -------------------------- 生の TCP で演じる上流（作業単位 G-5 の是正）
    //
    // **wiremock では作れない失敗が2つある** ——
    // 「2つ目の読み手が**接続できない**」と「本文が**途中で切れる**」。
    // どちらも [`CompatOutcome::from_error`] の**到達する腕**なので、
    // **作れないままだと、その腕を潰す変異が永久に緑になる**
    // （是正前は実際そうだった / 1945 全緑）。

    /// 1つ目の読み手が返す本文。**wiremock の腕と生の TCP の腕で同じものを使う**
    /// （片方だけ直すと、`native_body` の主張が腕ごとに別物になる）。
    const NATIVE_FAILURE_BODY: &str = "upstream is on fire";

    /// 1つ目の読み手へ返す 503 の、**生の応答そのもの。**
    ///
    /// # ⚠ `Connection: close` は、腕を分けている当のものではない（実測）
    ///
    /// **書いた本人は「落とすと2つ目が同じ接続を使い回して腕が化ける」と doc に書いた。**
    /// **測ったら偽だった** ——`X-Ignored: 1` に置き換える変異が **1948 全緑**
    /// （G-5 の是正の変異検査 `m17` / 2026-09-06）。
    ///
    /// **実際に接続を切っているのは [`serve_once`] のほう**である
    /// （応答を書いて返った時点でソケットを落とす）。**この見出しは宣言にすぎない。**
    ///
    /// **⚠ だから「ここを守れ」と読まないこと。**
    /// **腕を分けているのは [`serve_once`] がソケットを手放すこと**で、
    /// **そちらは腕の期待値が現に見張っている**（`m23` で実測）——
    /// `std::mem::forget(socket)` を足して握り続けさせると、
    /// 2つ目の要求が誰にも読まれずに滞留し、
    /// **`compat_outcome=unreachable` が `timeout` になって赤くなる。**
    /// **見出しも一緒に落とした `m24` でも同じ**（2箇所が互いを隠してはいない）。
    ///
    /// 見出しを残してあるのは、**実機の HTTP サーバが実際にそう名乗るから**
    /// （現実の形に寄せる）であって、テストがそれに寄りかかっているからではない。
    fn native_503_wire() -> String {
        format!(
            "HTTP/1.1 503 Service Unavailable\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n{NATIVE_FAILURE_BODY}",
            NATIVE_FAILURE_BODY.len()
        )
    }

    /// 2つ目の読み手へ返す**壊れた** 200。
    ///
    /// **`Content-Length` が本文より長い。** 送り切らずに閉じるので、
    /// `reqwest` は**本文の読み取り**で失敗する（= [`CompatOutcome::Transport`]）。
    /// **ステータスは 200 なので、[`LmStudioClient::fetch_models`] の
    /// 「ステータス異常」の枝は通らない。**
    fn truncated_200_wire() -> String {
        let body = r#"{"object":"list","data":[]}"#;
        format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len() + 64
        )
    }

    /// 1接続ぶんだけ応答して閉じる。
    ///
    /// **要求の綴りが `expect_path` であることまで見る** ——
    /// 順番が入れ替わったら、その腕は別のものを測っている。
    async fn serve_once(listener: &tokio::net::TcpListener, expect_path: &str, response: &str) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut socket, _) = listener.accept().await.expect("接続を受けられること");
        let mut request = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            let read = socket.read(&mut buf).await.expect("要求を読めること");
            assert_ne!(read, 0, "要求が途中で切れないこと: {request:?}");
            request.extend_from_slice(&buf[..read]);
            // GET なので、ヘッダの終わりが要求の終わり。
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&request);
        assert!(
            head.starts_with(&format!("GET {expect_path} ")),
            "要求の順番: 次に来るのは {expect_path} であること: {head:?}"
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("応答を書けること");
        socket.flush().await.expect("送り切れること");
    }

    /// 生の上流が演じる筋書き（作業単位 G-5 の是正）。
    #[derive(Debug, Clone, Copy)]
    enum WireScript {
        /// **誰も待ち受けていない。** 1つ目からして接続できない
        /// （= LM Studio が停止している普段の状態 / docs/04-environment.md）。
        NeverListens,
        /// 1つ目に 503 で答えた**直後に、待ち受けごと畳む。**
        /// 2つ目は接続できない（= [`CompatOutcome::Unreachable`]）。
        DiesAfterTheFirstReader,
        /// 2つ目に**途中で切れる 200** を返す（= [`CompatOutcome::Transport`]）。
        TruncatesTheSecondReader,
        /// **1つ目**に**途中で切れる 200** を返す（作業単位 G-8 ①）。
        ///
        /// ステータスは 200 なので [`LmStudioClient::fetch_models`] の
        /// 「ステータス異常」の枝は通らず、**失敗するのは `text()` の側**
        /// （= [`LlmError::Transport`]）。
        ///
        /// **2つ目は投げない**——[`LmStudioClient::models`] が2つ目へ進むのは
        /// [`LlmError::Status`] を受けたときだけだからである。
        /// **[`RequestFailureVoice`] の通信失敗の腕を、1つ目の読み手で通す唯一の筋書き。**
        TruncatesTheFirstReader,
    }

    /// 筋書きを演じ切るのを待つ上限（4周目の是正 / 2026-09-06）。
    ///
    /// # ⚠ 待つ側に上限が要る（**実測で踏んだ**）
    ///
    /// **`upstream.await` に上限が無いと、上流が想定どおりに叩かれなかった回、
    /// テストは赤にならずに「止まる」。**
    ///
    /// **実測**: [`serve_once`] にソケットを握らせる変異（3周目の `m23`）と、
    /// [`native_503_wire`] から `Connection: close` を落とす変異（3周目の `m17`）を
    /// **組にすると**、2つ目の要求が同じ接続を使い回して誰も `accept` せず、
    /// **[`the_hushed_reader_still_speaks_at_debug`] が 24 分走り続けた**
    /// （**どちらの変異も単独では1分で決着する** ——
    /// CLAUDE.md「第5の分類 —— 2つの変異が互いを隠し合う」）。
    ///
    /// **上限は「働く側」ではなく「待つ側」に要る**（CLAUDE.md / 作業単位 G-4）。
    ///
    /// **⚠ 速さの主張ではない。** 通常は即座に終わる（いちばん長い腕でも
    /// [`DEFAULT_CONTEXT_TIMEOUT`] の 5 秒）ので十分に緩く、
    /// 外れたときに言うのは**「上流が想定どおりに叩かれなかった」だけ**である。
    ///
    /// [`the_hushed_reader_still_speaks_at_debug`]: the_hushed_reader_still_speaks_at_debug
    const UPSTREAM_JOIN_LIMIT: Duration = Duration::from_secs(30);

    /// 筋書きを演じ切るのを、上限つきで待つ。**掛かったら名前のある赤にする。**
    async fn join_upstream(upstream: tokio::task::JoinHandle<()>) {
        let abort = upstream.abort_handle();
        match tokio::time::timeout(UPSTREAM_JOIN_LIMIT, upstream).await {
            Ok(joined) => joined.expect("筋書きを演じ切れること"),
            Err(_) => {
                abort.abort();
                panic!(
                    "上流の筋書きが {UPSTREAM_JOIN_LIMIT:?} 以内に終わらなかった\
                     （要求が届いていない。接続の使い回しを疑うこと）"
                );
            }
        }
    }

    /// 生の上流を1つ立てて `models()` を1回叩く。
    fn query_models_over_wire(
        filter: &str,
        script: WireScript,
    ) -> (String, Result<ModelsReport, LlmError>) {
        let runtime = runtime();
        let mut outcome = None;
        let output = crate::testlog::captured_with_filter(filter, || {
            runtime.block_on(async {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("待ち受けを開けること");
                let base = format!(
                    "http://{}",
                    listener.local_addr().expect("待ち受け先を読めること")
                );
                let upstream = match script {
                    WireScript::NeverListens => {
                        // **要求が来る前に畳む。** 以後この綴りへは接続できない。
                        drop(listener);
                        None
                    }
                    WireScript::DiesAfterTheFirstReader => Some(tokio::spawn(async move {
                        serve_once(&listener, NATIVE_MODELS_PATH, &native_503_wire()).await;
                        // **ここで畳む。** 2つ目の接続は拒否される。
                        drop(listener);
                    })),
                    WireScript::TruncatesTheSecondReader => Some(tokio::spawn(async move {
                        serve_once(&listener, NATIVE_MODELS_PATH, &native_503_wire()).await;
                        serve_once(&listener, OPENAI_MODELS_PATH, &truncated_200_wire()).await;
                    })),
                    WireScript::TruncatesTheFirstReader => Some(tokio::spawn(async move {
                        // **1つ目で切る。** 2つ目は投げられないので、演じるのはこれだけ。
                        serve_once(&listener, NATIVE_MODELS_PATH, &truncated_200_wire()).await;
                    })),
                };
                let client = LmStudioClient::new(
                    LmStudioConfig::new(&base, model()).expect("ベース URL は正当"),
                )
                .expect("クライアントを作れること");
                outcome = Some(client.models().await);
                if let Some(upstream) = upstream {
                    join_upstream(upstream).await;
                }
            });
        });
        (output, outcome.expect("往復が済んでいること"))
    }

    /// **この層が「言っていること」で行を選ぶ**（作業単位 G-5 の是正 / 2026-09-06）。
    ///
    /// # ⚠ 文面だけで [`crate::testlog::line_containing`] を呼ばないこと
    ///
    /// **`LlmError` の `Display` は定数文字列**（CLAUDE.md / エラーレスポンスに詳細を出さないため）。
    /// つまり**同じ文面が、別の行の `error=` の値としてそのまま載る。**
    /// 実際にこの捕捉の中には、こういう2本が並んでいる:
    ///
    /// ```text
    /// DEBUG …::lmstudio: 推論サーバへ接続できませんでした error=error sending request for url (…/v1/models)
    /// DEBUG …::lmstudio: OpenAI 互換のモデル一覧も読めませんでした error=推論サーバへ接続できませんでした
    /// ```
    ///
    /// **どちらも文面を含み、どちらも DEBUG。** だから文面と水準だけで見ると、
    /// **1本目が消えても2本目に当たって緑のまま**になる ——
    /// **実測: [`RequestFailureVoice::Hush`] の腕を `trace!` へ落とす変異が 1949 全緑**
    /// （是正前の `m22`）。**まさに「その行が出た」と「その行がこう言っている」の取り違え。**
    ///
    /// **層の綴りと文面を繋いで、`TARGET: メッセージ` の位置に固定する。**
    ///
    /// # ⚠ 訂正: この錨は、いまのところ**単独で赤にしているものが1つも無い**
    ///
    /// **4周目に、反対向きを測った**（3周目は測っていなかった / 2026-09-06）。
    /// **錨を素の [`line_containing`] へ戻したうえで `m22` を当て直すと、
    /// 依然として赤**である（3腕とも。`m11` / `m12`）。
    ///
    /// **赤にしているのは錨ではなく、その次の行の
    /// `contains(OPENAI_MODELS_PATH)`** ——
    /// **囮は [`LlmError`] の `Display`（定数文字列）を `error=` に載せているだけなので、
    /// どの綴りで失敗したかを持っていない。**
    ///
    /// **⚠ だから「この錨が m22 を捕まえる」と読まないこと。**
    /// 上の「1949 全緑」は**是正前の形**の実測であって、いまの形の実測ではない。
    /// **いま錨がしていることは1つ ——「囮ではなく本命の行を、失敗の文面に出す」。**
    /// **止める場所ではなく、名指しする名前のほうの値打ちである**
    /// （CLAUDE.md / 作業単位 B-38）。
    ///
    /// **⚠ それでも外さないこと。** 詳細を見ない主張
    /// （[`a_best_effort_query_names_its_failure_without_warning`] の
    /// `has_field(_, "INFO")` のように）を後から足した日に、
    /// **囮がその主張に答えてしまう。**
    /// **いま単独で効いていないことは、「要らない」ではなく「まだ試されていない」である。**
    ///
    /// [`a_best_effort_query_names_its_failure_without_warning`]:
    ///     a_best_effort_query_names_its_failure_without_warning
    fn line_said_by_this_layer<'a>(output: &'a str, message: &str) -> &'a str {
        line_containing(output, &format!("{THIS_LAYER} {message}"))
    }

    /// ログに出るこの層の綴り（`TARGET:` の位置）。
    const THIS_LAYER: &str = "personacore::llm::lmstudio:";

    /// その回に **WARN を出した層の一覧**（出た順）。
    ///
    /// **本数で見ないこと。** 層をまたいで1本移す変異（片方を消して片方に足す）は
    /// **本数だと足し引きで黙る**（G-5 の是正でレビュアーが実証 / 1945 全緑）。
    fn warning_emitters(output: &str) -> Vec<&str> {
        output
            .lines()
            .filter(|line| has_field(line, "WARN"))
            .map(emitter_of)
            .collect()
    }

    /// 「どちらの読み手も駄目だった」1行を、**腕ごとに同じ強さで**検査する。
    ///
    /// 上流の作り方（wiremock / 生の TCP）は腕ごとに違うが、
    /// **主張は1つ**——1行の中で、どちらの綴りを・どう失ったかまで読めること。
    fn assert_neither_reader_line(
        output: &str,
        report: Result<ModelsReport, LlmError>,
        expected_outcome: &str,
        expected_emitters: &[&str],
    ) {
        let line = line_containing(output, NEITHER_READER_MARKER);
        assert!(
            has_field(line, "WARN"),
            "深刻度は warn であること（本物の問題）: {line:?}"
        );
        assert_eq!(
            quoted_field(line, "native_path"),
            Some(NATIVE_MODELS_PATH),
            "1つ目の綴りを同じ行に載せること: {line:?}"
        );
        assert!(
            has_field(line, "native_status=503"),
            "1つ目のステータスを同じ行に載せること: {line:?}"
        );
        assert_eq!(
            quoted_field(line, "compat_path"),
            Some(OPENAI_MODELS_PATH),
            "2つ目の綴りを同じ行に載せること: {line:?}"
        );
        assert!(
            has_field(line, &format!("compat_outcome={expected_outcome}")),
            "2つ目がどう失敗したかを同じ行に載せること: {line:?}"
        );
        assert!(
            !has_field(line, "compat_outcome=status_503"),
            "2つ目の欄に1つ目のステータスを載せないこと（帰属）: {line:?}"
        );
        assert_eq!(
            quoted_field(line, "native_body"),
            Some(NATIVE_FAILURE_BODY),
            "調査の手掛かり（1つ目の応答本文）を同じ行に残すこと: {line:?}"
        );

        // **同じ事実を2本にしないこと**（G-5 の是正）。
        assert_eq!(
            warning_emitters(output),
            expected_emitters,
            "この回に WARN を出した層が食い違っている: {output:?}"
        );

        assert_eq!(
            report.expect_err("往復ごと失敗すること").status(),
            Some(503),
            "失敗の理由は1つ目のものを保つこと"
        );
    }

    /// **どの読み手が答えたかを、報告そのものが持つ**（作業単位 G-3b）。
    ///
    /// # なぜログでは足りないのか
    ///
    /// **`server` はこれを実行中も持ち続ける**（`AppState` / 作業単位 G-10 と D-9）。
    /// **ログにしか無いと、5分ごとの引き直しで変わった後の状態を誰も読めない。**
    ///
    /// # ⚠ 上流が名乗った綴りは持ち上げない
    ///
    /// `llama-server` の `id` は **gguf の絶対パス**。
    /// **[`AnsweredBy`] は値を持たない列挙**なので、
    /// **同定した行の綴りはこの型に載らない**（絶対制約: 応答へ内部パスを出さない）。
    /// **一覧（[`ModelsReport::catalogue`]）には載る**——そちらは `measured::check` の
    /// 中で使い切られ、[`crate::server::UpstreamObservation`] へは持ち上がらない。
    /// 見張り番は `tests/server_upstream_facts.rs::
    /// the_held_facts_never_carry_the_upstreams_own_model_id`。
    ///
    /// # 4つの腕は互いに区別が付くこと
    ///
    /// **一致した綴りで同定した腕は、一覧を2件にしてある**——1件だと
    /// [`AnsweredBy::CompatSoleEntry`] と区別が付かず、
    /// **完全一致の段を丸ごと消す変異が緑になる。**
    #[test]
    fn the_report_names_which_reader_answered() {
        let native_ok = || {
            wiremock::ResponseTemplate::new(200).set_body_raw(
                format!(
                    r#"{{"object":"list","data":[{{"id":"{DEFAULT_MAIN_MODEL}","object":"model",
                       "type":"llm","state":"loaded","max_context_length":262144,
                       "loaded_context_length":16384}}]}}"#
                ),
                "application/json",
            )
        };
        // **完全一致の行があり、かつ一覧は2件**（＝1件だから同定した、ではない）。
        let compat_exact = || {
            wiremock::ResponseTemplate::new(200).set_body_raw(
                format!(
                    r#"{{"object":"list","data":[
                       {{"id":"{DEFAULT_MAIN_MODEL}","object":"model",
                         "meta":{{"n_ctx":{G5_COMPAT_N_CTX}}}}},
                       {{"id":"text-embedding-nomic-embed-text-v1.5","object":"model"}}]}}"#
                ),
                "application/json",
            )
        };
        // **一致は無く、一覧は2件**（同定できない。綴りだけが取れる）。
        let compat_catalogue_only = || {
            wiremock::ResponseTemplate::new(200).set_body_raw(
                r#"{"object":"list","data":[{"id":"a-model","object":"model"},
                   {"id":"b-model","object":"model"}]}"#,
                "application/json",
            )
        };

        let arms: [(
            &str,
            wiremock::ResponseTemplate,
            wiremock::ResponseTemplate,
            AnsweredBy,
        ); 4] = [
            (
                "独自 API が 2xx を返した",
                native_ok(),
                compat_exact(),
                AnsweredBy::NativeApi,
            ),
            (
                "OpenAI 互換で、綴りが完全一致した",
                wiremock::ResponseTemplate::new(404),
                compat_exact(),
                AnsweredBy::CompatExactId,
            ),
            (
                "OpenAI 互換で、一覧が1件だった",
                wiremock::ResponseTemplate::new(404),
                wiremock::ResponseTemplate::new(200)
                    .set_body_raw(g5_compat_body(), "application/json"),
                AnsweredBy::CompatSoleEntry,
            ),
            (
                "OpenAI 互換だが、同定できず綴りだけが取れた",
                wiremock::ResponseTemplate::new(404),
                compat_catalogue_only(),
                AnsweredBy::CompatCatalogueOnly,
            ),
        ];

        for (what, native, compat, expected) in arms {
            let (_, report) = query_models(crate::server::DEFAULT_LOG_FILTER, native, compat);
            let report = report.expect("往復そのものは成功すること");
            assert_eq!(
                report.answered_by(),
                expected,
                "{what}: どの読み手が答えたかを取り違えないこと"
            );
            // **同定した行の綴りは、報告のどこにも載らない**（一覧を除く）。
            assert!(
                !report.answered_by().label().contains(G5_COMPAT_MODEL_ID),
                "{what}: 読み手の綴りに上流の `id` を混ぜないこと"
            );
        }
    }

    /// **4つの綴りは互いに違う**（同値なら上の突き合わせは何も守っていない）。
    #[test]
    fn every_reader_label_is_its_own_spelling() {
        let labels = [
            AnsweredBy::NativeApi.label(),
            AnsweredBy::CompatExactId.label(),
            AnsweredBy::CompatSoleEntry.label(),
            AnsweredBy::CompatCatalogueOnly.label(),
        ];
        let mut sorted = labels.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), labels.len(), "綴りが重なっている: {labels:?}");
    }

    /// **1つ目の 404 は、2つ目が答えたなら警告ではない**（作業単位 G-5 の存在理由）。
    ///
    /// `llama-server` を上流にしたときの、まさにその経路。
    ///
    /// # 否定側の前に肯定側を出す
    ///
    /// 「既定のフィルタで出ない」を主張する前に、
    /// **そもそもそのイベントが出ていること**を `trace` で確かめる
    /// （確かめないと、行ごと消す変異でも「何も出なかった」まま緑になる /
    /// `crate::testlog` の doc）。
    #[test]
    fn a_native_404_is_not_a_warning_when_the_openai_list_answers() {
        let compat = || {
            wiremock::ResponseTemplate::new(200).set_body_raw(g5_compat_body(), "application/json")
        };

        // 肯定側——`RUST_LOG=debug` 相当なら、読み手1つぶんの失敗は現に出ている
        // （**`trace` で見ないこと**——[`DEBUG_FILTER`] の doc）。
        let (verbose, _) =
            query_models(DEBUG_FILTER, wiremock::ResponseTemplate::new(404), compat());
        assert!(
            verbose.contains(ONE_READER_MARKER),
            "前提: 読み手1つぶんの失敗は、debug のフィルタでは出ること: {verbose:?}"
        );

        // 否定側——既定のフィルタでは出ない。
        let (output, report) = query_models(
            crate::server::DEFAULT_LOG_FILTER,
            wiremock::ResponseTemplate::new(404),
            compat(),
        );
        assert!(
            !output.contains(ONE_READER_MARKER),
            "2つ目が答えた回に、1つ目の 404 を既定のフィルタへ出さないこと: {output:?}"
        );
        assert!(
            !output.contains(NEITHER_READER_MARKER),
            "読めているのに「どちらも読めなかった」と言わないこと: {output:?}"
        );
        assert!(
            !output.lines().any(|line| has_field(line, "WARN")),
            "正常な経路に WARN の行を1本も出さないこと: {output:?}"
        );

        // **正常な経路であることは、成功の1行で読める**（G-2 / 文面は触らない）。
        let line = line_containing(&output, COMPAT_READ_MARKER);
        assert!(
            has_field(line, &format!("context_length={G5_COMPAT_N_CTX}")),
            "2つ目が読んだ窓を同じ行に載せること: {line:?}"
        );

        assert_eq!(
            report.expect("往復は成功すること").context_length().ok(),
            Some(G5_COMPAT_N_CTX),
            "落とし所ではなく、上流の申告値を返すこと"
        );
    }

    /// **どちらの読み手も何も返さなかったら、それは警告である**（作業単位 G-5）。
    ///
    /// **深刻度を下げた向きの副作用（本物の問題まで静かになる）を、ここで押さえる。**
    /// 1本の行で「どちらの綴りを、どのステータスで失ったか」まで読めること。
    ///
    /// # 2つ目の失敗の中身も、同じ行に載せる（G-5 の是正）
    ///
    /// 返る誤りは1つ目の [`LlmError::Status`] なので、**2つ目の理由は捨てられる。**
    /// **404 か・5xx か・タイムアウトか・「答えたが中身が無い」かで、
    /// 読み手が調べに行く先がまるで違う**（CLAUDE.md / B-38「見張り番の値打ちは
    /// どこを直せと言うか」）。
    ///
    /// **腕を複数回すのは、その欄が定数に潰れていないことを見るため。**
    /// 1本だけだと `compat_outcome` を `status_404` に決め打つ変異が通る。
    ///
    /// # `status_500` の腕が要る理由（4周目の是正 / 2026-09-06）
    ///
    /// **腕を4つ回しても、`Status` の腕を通す上流が 404 の1本きりだった。**
    /// **`CompatOutcome::from_error` の `Self::Status(*status)` を
    /// `Self::Status(404)` に潰す変異が 1949 全緑**——
    /// **腕は見張られていたが、値は見張られていなかった。**
    /// **`status_404` を読んだ人は「綴りが違う」を疑いに行く。実際は「上流が壊れている」**
    /// （CLAUDE.md / 作業単位 B-38「警報は出ており、指示だけが誤っている」）。
    ///
    /// # `timeout` の腕が要る理由（作業単位 G-5 の是正 / 2026-09-06）
    ///
    /// **是正前、[`CompatOutcome::from_error`] で見張られていたのは `Status` だけ**
    /// だった（`unusable` は [`LmStudioClient::compat_models`] が直に作るので、
    /// あの関数を一度も通らない）。
    /// **`Timeout` の腕を潰す変異は 1945 全緑**で、doc は
    /// **「下の6つは到達しない」としか断っていなかった。**
    ///
    /// **そしてこの腕は、同時に「同じ事実が2本にならないこと」の権威でもある。**
    /// 是正前は `map_request_error` が無条件に `warn` を出していたので、
    /// **この腕だけ WARN が2本**出ていた（G-5 が消しにきた形そのもの）。
    #[test]
    fn neither_reader_answering_is_warned_with_both_paths_on_one_line() {
        /// 2つ目の読み手が返さない時間。**上限より十分に長いこと。**
        const COMPAT_STALL: Duration = Duration::from_secs(30);
        /// 上限。**待たされるのはここまで**（既定の 5 秒を待たない）。
        const BOUNDED: Duration = Duration::from_millis(200);

        for (compat, timeout, expected_outcome, expected_emitters) in [
            // 2つ目のエンドポイントも断った。
            (
                wiremock::ResponseTemplate::new(404),
                DEFAULT_CONTEXT_TIMEOUT,
                "status_404",
                &["personacore::llm::lmstudio:"][..],
            ),
            // 2つ目は**応答したが、壊れている**（4周目の是正 / 2026-09-06）。
            //
            // **この腕が無いと `Status` の腕は 404 の1本きり**になり、
            // **`CompatOutcome::from_error` の `Self::Status(*status)` を
            // `Self::Status(404)` に決め打つ変異が 1949 全緑**を通る
            // （[`CompatOutcome::from_error`] の doc「腕が見張ってあるは
            // 値が見張ってあるではない」）。
            //
            // **`status_404` と `status_500` で、読み手が向かう先が違う** ——
            // 前者は「綴りが違う・エンドポイントが無い」、後者は「上流が壊れている」。
            (
                wiremock::ResponseTemplate::new(500),
                DEFAULT_CONTEXT_TIMEOUT,
                "status_500",
                &["personacore::llm::lmstudio:"][..],
            ),
            // 2つ目は **200 を返したのに、長さも綴りも取れなかった。**
            //
            // **⚠ `llm::response` の1本は、この腕では出なくなった**（作業単位 G-8 ②）。
            // あちらの「同定できません」は `debug` へ落ちたので、
            // **この事実を `warn` で言うのはこの層の1本だけ**である。
            // **一覧で固定してあるので、どちらの層が1本足しても必ず赤になる。**
            (
                wiremock::ResponseTemplate::new(200)
                    .set_body_raw(r#"{"object":"list","data":[]}"#, "application/json"),
                DEFAULT_CONTEXT_TIMEOUT,
                "unusable",
                &["personacore::llm::lmstudio:"][..],
            ),
            // 2つ目は**上限まで何も返さなかった**（G-5 の是正）。
            // **答えられる上流でも、遅ければここに来る。**
            (
                wiremock::ResponseTemplate::new(200)
                    .set_body_raw(g5_compat_body(), "application/json")
                    .set_delay(COMPAT_STALL),
                BOUNDED,
                "timeout",
                &["personacore::llm::lmstudio:"][..],
            ),
        ] {
            let (output, report) = query_models_bounded(
                crate::server::DEFAULT_LOG_FILTER,
                wiremock::ResponseTemplate::new(503).set_body_string(NATIVE_FAILURE_BODY),
                compat,
                timeout,
            );

            assert_neither_reader_line(&output, report, expected_outcome, expected_emitters);
        }
    }

    /// **上流が死ぬ・本文が切れる**——wiremock では作れない2つ（G-5 の是正）。
    ///
    /// [`CompatOutcome::from_error`] の到達する4腕のうち、
    /// **`Connect` と `Transport` はここでしか通らない。**
    /// 是正前はどちらも潰す変異が 1945 全緑だった。
    ///
    /// # 現実に起きる形である
    ///
    /// **`llama-server` を上流にすると1つ目は必ず 404 になる**（G-2 が実測）ので、
    /// **「1つ目に答えた直後に上流が落ちる」は、そのまま起動時の一場面。**
    /// そのとき読み手が向かう先は「上流を起動し直す」であって、
    /// 「綴りが違う」でも「遅い」でもない——**`compat_outcome` の1語がそれを言う。**
    #[test]
    fn a_dying_upstream_is_named_for_what_it_did() {
        for (script, expected_outcome) in [
            (WireScript::DiesAfterTheFirstReader, "unreachable"),
            (WireScript::TruncatesTheSecondReader, "transport"),
        ] {
            let (output, report) =
                query_models_over_wire(crate::server::DEFAULT_LOG_FILTER, script);

            assert_neither_reader_line(
                &output,
                report,
                expected_outcome,
                &["personacore::llm::lmstudio:"],
            );
        }
    }

    /// **コンテキスト長の照会は、失敗を名乗るが警告にはしない**（作業単位 G-8 ①）。
    ///
    /// # なぜ黙らせてよいのか
    ///
    /// この往復は**「取れたら得をする」経路**（`server::context` のモジュール doc）で、
    /// **失敗しても何も止まらない。** 失われるのは「上流の実測値」だけで、
    /// **その結果は呼び出し側が必ず言う** ——
    ///
    /// | いつ | 呼び出し側 | 水準 |
    /// |---|---|---|
    /// | 起動時 | `choose_budget` / `choose_sub_context` の「既定値を使います / 落とし所を使います」 | `warn` |
    /// | 5分ごとの引き直し | `refresh_once` / `refresh_sub_once` の「現在の値を保ちます」 | `info`（ADR-0020 §5 —— 値は変わっていない） |
    ///
    /// **`warn` に戻すと、上流が停止しているだけで5分ごとに鳴る**
    /// （**この家では LM Studio は普段落としてある** / docs/04-environment.md）。
    ///
    /// # ⚠ 3つの分類を分けて残さないこと（**実機で測った** / 2026-09-07）
    ///
    /// **「上流が答えない」は、断られ方で `Connect` にも `Timeout` にもなる。**
    /// 実測: **誰も待ち受けていない綴りへ向けると `Connect`**（接続を拒否される）、
    /// **接続は受けるが何も返さない上流へ向けると `Timeout`**
    /// （モデルをロード中の LM Studio・詰まった上流・受けてから止まる中継が、この形）。
    /// **同じ事実の深刻度が、断られ方で変わってはいけない。**
    /// 着手前は接続失敗だけが `info` で、残り2つが `warn` だったので、
    /// **後者では起動時に2本・5分ごとに2本鳴っていた**（実測 / 作業単位 G-8 ①）。
    ///
    /// # 名乗りは消さない
    ///
    /// **`Hush` へ落とさないのは、`reqwest` が言った中身（どの綴りへ・どう失敗したか）を
    /// 持っているのがこの1行だけ**だから。`LlmError` の `Display` は定数文字列なので、
    /// 呼び出し側の `info` は綴りを持たない。**既定のフィルタは `personacore=info`** なので、
    /// 運用者はこの行をそのまま受け取る。
    ///
    /// # 3腕とも回すこと
    ///
    /// [`RequestFailureVoice::Murmur`] の主張は3腕ぶんある
    /// （CLAUDE.md / 作業単位 D-6「代表1つでは足りない」）。
    /// **接続失敗の腕だけだと、着手前の実装でも緑になる**（あちらも `info` だった）。
    #[test]
    fn a_best_effort_query_names_its_failure_without_warning() {
        /// 1つ目の読み手が返さない時間。**上限より十分に長いこと。**
        const QUERY_STALL: Duration = Duration::from_secs(30);
        /// 上限。**待たされるのはここまで**（既定の 5 秒を待たない）。
        const QUERY_BOUNDED: Duration = Duration::from_millis(200);

        // **接続失敗と通信失敗は wiremock では作れない**（[`WireScript`] の doc）。
        let over_wire = [
            (WireScript::NeverListens, CONNECT_FAILED_MARKER),
            (WireScript::TruncatesTheFirstReader, TRANSPORT_FAILED_MARKER),
        ]
        .map(|(script, marker)| {
            (
                query_models_over_wire(crate::server::DEFAULT_LOG_FILTER, script),
                marker,
            )
        });

        // **タイムアウトは遅い上流**（生の TCP では作りづらいので wiremock）。
        let stalled = query_models_bounded(
            crate::server::DEFAULT_LOG_FILTER,
            wiremock::ResponseTemplate::new(200)
                .set_body_raw(g5_compat_body(), "application/json")
                .set_delay(QUERY_STALL),
            // **2つ目は即答できる状態にしておく**（投げないことも主張のうち）。
            wiremock::ResponseTemplate::new(200).set_body_raw(g5_compat_body(), "application/json"),
            QUERY_BOUNDED,
        );

        for ((output, report), marker) in over_wire
            .iter()
            .map(|((output, report), marker)| ((output.as_str(), report), *marker))
            .chain(std::iter::once((
                (stalled.0.as_str(), &stalled.1),
                TIMED_OUT_MARKER,
            )))
        {
            // 肯定側——**その事実は現に1行出ている**（`Hush` へ潰す変異と区別が付くこと）。
            //
            // **文面だけで選ばないこと**——理由は [`line_said_by_this_layer`] の doc。
            let line = line_said_by_this_layer(output, marker);
            assert!(
                has_field(line, "INFO"),
                "照会の失敗は名乗るが、警告にはしないこと: {line:?}"
            );
            assert!(
                line.contains(NATIVE_MODELS_PATH),
                "reqwest が言った中身（どの綴りで失敗したか）まで残っていること: {line:?}"
            );

            // 否定側——既定のフィルタに WARN を1本も出さない。
            assert!(
                warning_emitters(output).is_empty(),
                "5分ごとに鳴る警告にしないこと: {output:?}"
            );
            assert!(
                !output.contains(NEITHER_READER_MARKER),
                "2つ目は投げていないので、そちらの綴りを名乗らないこと: {output:?}"
            );
            assert!(
                report.is_err(),
                "往復ごと失敗していること（黙ったのは深刻度だけ）: {report:?}"
            );
        }

        assert!(
            matches!(stalled.1, Err(LlmError::Timeout(_))),
            "タイムアウトとして区別されること: {:?}",
            stalled.1
        );
    }

    /// **黙らせた読み手も、`debug` では同じことを同じ文面で言う**（作業単位 G-5 の是正）。
    ///
    /// [`RequestFailureVoice::Hush`] の doc は「**握り潰すのではない**」と書いている。
    /// **その doc を真にするのはこの1本だけ** ——
    /// `Hush` の腕から行を丸ごと落とす変異も、`debug` 以外の水準へ動かす変異も、
    /// **既定のフィルタでは元から出ない側**なので、否定側の見張り番には一切掛からない。
    ///
    /// # この文面の行は、2つ目の読み手のものに限られる
    ///
    /// どの腕も**1つ目には 503 で答える**（ステータス異常であって通信の失敗ではない）ので、
    /// `map_request_error` を通るのは2つ目だけ。**`Speak` の行と取り違えようがない。**
    ///
    /// # ⚠ 3腕とも回すこと（4周目の是正 / 2026-09-06）
    ///
    /// **3周目は接続失敗の1腕しか回していなかった。**
    /// **残り2腕（タイムアウト・通信失敗）を `trace!` へ落とす変異は 1949 全緑**だった
    /// ——**どちらの腕も現に通っている**（タイムアウトは
    /// [`neither_reader_answering_is_warned_with_both_paths_on_one_line`] の `timeout` の腕、
    /// 通信失敗は [`WireScript::TruncatesTheSecondReader`]）。
    /// **通っているのに、どちらも [`crate::server::DEFAULT_LOG_FILTER`] で捕捉していたので
    /// `debug` が原理的に見えなかった** ——
    /// **「分岐に入った」と「入ったことを見た」は別**（CLAUDE.md）。
    ///
    /// [`RequestFailureVoice::Hush`] の doc は3腕ぶんの主張をしているので、
    /// **押さえも3腕要る**（CLAUDE.md / 作業単位 D-6）。
    ///
    /// [`neither_reader_answering_is_warned_with_both_paths_on_one_line`]:
    ///     neither_reader_answering_is_warned_with_both_paths_on_one_line
    #[test]
    fn the_hushed_reader_still_speaks_at_debug() {
        /// 2つ目の読み手が返さない時間。**上限より十分に長いこと。**
        const COMPAT_STALL: Duration = Duration::from_secs(30);
        /// 上限。**待たされるのはここまで。**
        const BOUNDED: Duration = Duration::from_millis(200);

        // **接続失敗と通信失敗は wiremock では作れない**（[`WireScript`] の doc）。
        let over_wire = [
            (WireScript::DiesAfterTheFirstReader, CONNECT_FAILED_MARKER),
            (
                WireScript::TruncatesTheSecondReader,
                TRANSPORT_FAILED_MARKER,
            ),
        ]
        .map(|(script, marker)| (query_models_over_wire(DEBUG_FILTER, script).0, marker));

        // **タイムアウトは遅い上流**（生の TCP では作りづらいので wiremock）。
        let stalled = query_models_bounded(
            DEBUG_FILTER,
            wiremock::ResponseTemplate::new(503).set_body_string(NATIVE_FAILURE_BODY),
            wiremock::ResponseTemplate::new(200)
                .set_body_raw(g5_compat_body(), "application/json")
                .set_delay(COMPAT_STALL),
            BOUNDED,
        )
        .0;

        for (verbose, marker) in over_wire
            .iter()
            .map(|(verbose, marker)| (verbose.as_str(), *marker))
            .chain(std::iter::once((stalled.as_str(), TIMED_OUT_MARKER)))
        {
            // **文面だけで選ばないこと**——理由は [`line_said_by_this_layer`] の doc。
            // **3腕とも、同じ文面を `error=` に載せた行がこの層から別に出ている。**
            let line = line_said_by_this_layer(verbose, marker);
            assert!(
                has_field(line, "DEBUG"),
                "2つ目の読み手の失敗も、debug には同じ文面で残っていること: {line:?}"
            );
            // **「同じ詳細」までを主張する**（`Hush` の doc がそう書いている）。
            // `reqwest` の言い分には**どの綴りで失敗したかそのもの**が入っているので、
            // **`LlmError` の定数文字列に潰す変異はここで赤になる。**
            assert!(
                line.contains(OPENAI_MODELS_PATH),
                "reqwest が言った中身（どの綴りで失敗したか）まで残っていること: {line:?}"
            );
        }
    }

    /// **停止している上流は警告ではない**（作業単位 G-5 の是正）。
    ///
    /// **LM Studio は普段落としてある**（docs/04-environment.md / CLAUDE.md）。
    /// ここを `warn` にすると、**起動のたびと5分ごとに鳴る** ——
    /// G-5 が消しにきたノイズを、上流が停止しているだけで作り直すことになる。
    ///
    /// # ⚠ 見ているのは [`RequestFailureVoice::Murmur`] の列だけ（**訂正 / 2026-09-07**）
    ///
    /// **着手前は「`map_request_error` の接続失敗の腕を `warn` へ上げる変異は、
    /// ここで赤になる」と書いてあった。G-8 ① の後は偽である。**
    ///
    /// この筋書きは [`LmStudioClient::models`] を通る。**その口の声は G-8 ① で
    /// [`RequestFailureVoice::Speak`] から [`RequestFailureVoice::Murmur`] へ移った**ので、
    /// **腕を割って `Speak` だけを `warn!` へ上げる変異は、ここでは赤にならない**
    /// （**実測: passed=1957 failed=0 / exit=0** / レビュアーの `m8`）。
    ///
    /// **`Speak` の列は [`a_relay_to_a_stopped_upstream_is_not_a_warning`] が見る。**
    ///
    /// [`a_relay_to_a_stopped_upstream_is_not_a_warning`]:
    ///     a_relay_to_a_stopped_upstream_is_not_a_warning
    #[test]
    fn a_stopped_upstream_is_not_a_warning() {
        // 肯定側——その事実は現に1行出ている（出ない変異と区別が付くこと）。
        //
        // **ここでも文面だけで選ばないこと**——[`line_said_by_this_layer`] の doc。
        // いまは1つ目で往復が終わるので紛れる行は無いが、
        // **紛れが「無い」ことに寄りかかった検査は、行が1本増えた日に黙って弱くなる。**
        let (verbose, _) = query_models_over_wire(DEBUG_FILTER, WireScript::NeverListens);
        let announced = line_said_by_this_layer(&verbose, CONNECT_FAILED_MARKER);
        assert!(
            has_field(announced, "INFO"),
            "接続できなかったことは info で1本出ていること: {announced:?}"
        );
        assert!(
            announced.contains(NATIVE_MODELS_PATH),
            "1つ目の読み手の失敗であること（2つ目は投げていない）: {announced:?}"
        );

        // 否定側——既定のフィルタには1本も出さない。
        let (output, report) =
            query_models_over_wire(crate::server::DEFAULT_LOG_FILTER, WireScript::NeverListens);
        assert!(
            warning_emitters(&output).is_empty(),
            "普段どおり停止しているだけの上流に WARN を出さないこと: {output:?}"
        );
        assert!(
            !output.contains(NEITHER_READER_MARKER),
            "2つ目は投げていないので、そちらの綴りを名乗らないこと: {output:?}"
        );
        assert!(
            matches!(report, Err(LlmError::Connect(_))),
            "接続失敗として区別されること: {report:?}"
        );
    }

    /// **送信の失敗の行にも、返す `LlmError` の中にも、宛先のクエリ・フラグメントが残らない**
    /// （作業単位 B-71 / Issue origin#100）。
    ///
    /// `reqwest::Error` の `Display` は `… for url (<叩いた URL>)` を含み、`Debug` は `url` の欄を持つ。
    /// **ユーザー情報は `reqwest` 自身が Basic 認証へ移すので URL に残らない**が、クエリとフラグメントは残る。
    /// **`LlmError` は `#[source]` にその `reqwest::Error` を持つ**ので、呼び出し側が `?err` で書いても漏れる。
    ///
    /// # 空振りではないこと
    ///
    /// **同じ捕捉の中で、宛先のホストとポート・叩いた経路が出ていること**を先に見る
    /// （`without_url()` で URL ごと消す実装は、ここで赤になる —— 切り分けの手掛かりを消さない）。
    #[test]
    fn a_failed_send_never_logs_the_query_or_fragment_of_the_endpoint() {
        const QUERY_SECRET: &str = "pc-b71-query-secret";
        const FRAGMENT_SECRET: &str = "pc-b71-fragment-secret";
        let runtime = runtime();
        let mut outcome = None;
        let mut host = String::new();
        let output = crate::testlog::captured_with_filter(DEBUG_FILTER, || {
            runtime.block_on(async {
                let listener =
                    std::net::TcpListener::bind("127.0.0.1:0").expect("待ち受けを開けること");
                host = listener
                    .local_addr()
                    .expect("待ち受け先を読めること")
                    .to_string();
                // **要求が来る前に畳む。** 以後この綴りへは接続できない。
                drop(listener);
                let base = format!("http://{host}/pc?token={QUERY_SECRET}#{FRAGMENT_SECRET}");
                let client = LmStudioClient::new(
                    LmStudioConfig::new(&base, model()).expect("ベース URL は正当"),
                )
                .expect("クライアントを作れること");
                outcome = Some(client.models().await);
            });
        });
        let report = outcome.expect("往復が済んでいること");

        // 肯定側: 失敗の行は出ていて、宛先のホストとポート・叩いた経路を名指している。
        let announced = line_said_by_this_layer(&output, CONNECT_FAILED_MARKER);
        assert!(
            announced.contains(&format!("http://{host}/pc")),
            "前提: 失敗の行が宛先のホスト・ポート・経路を名指していること: {announced:?}"
        );
        assert!(
            matches!(report, Err(LlmError::Connect(_))),
            "前提: 接続の失敗であること: {report:?}"
        );

        // 本題: どの行にも、返したエラーの `Debug`（`#[source]` を含む）にも、秘密は無い。
        let rendered = format!("{report:?}");
        for secret in [QUERY_SECRET, FRAGMENT_SECRET] {
            assert!(
                !output.contains(secret),
                "ログに宛先の秘密が出ている（{secret}）: {output}"
            );
            assert!(
                !rendered.contains(secret),
                "返したエラーに宛先の秘密が残っている（{secret}）: {rendered}"
            );
        }
    }

    /// **2つ目が綴りの一覧だけを返した回は、警告ではない**（作業単位 G-5）。
    ///
    /// 長さが読めなかったことは、**呼び出し側**（`server::context`）が
    /// 「落とし所を使います」で1本出す。**ここで出すと同じ事実が2本になる。**
    #[test]
    fn a_list_without_a_length_is_not_a_broken_upstream() {
        let (output, report) = query_models(
            crate::server::DEFAULT_LOG_FILTER,
            wiremock::ResponseTemplate::new(404),
            // **200 で、綴りは在る。`meta` だけが無い**（LM Studio の `/v1/models` の形）。
            wiremock::ResponseTemplate::new(200).set_body_raw(
                r#"{"object":"list","data":[{"id":"some-model","object":"model"}]}"#,
                "application/json",
            ),
        );

        assert!(
            !output.contains(NEITHER_READER_MARKER),
            "綴りは取れているのに「どちらも読めなかった」と言わないこと: {output:?}"
        );
        // **「その文面が出ていない」は「警告が出ていない」ではない。**
        //
        // G-5 の是正で実測: 上の否定だけだと、**この枝の直前に `warn!` を1本足す変異が
        // 1945 全緑**を通る（`NEITHER_READER_MARKER` しか見ていないので）。
        // 兄弟の `a_native_404_is_not_a_warning_when_the_openai_list_answers` は
        // 最初から強い形を持っていて、**写す先が1本足りなかった**という形。
        //
        // ここに入るのは「1つ目が 2xx 以外・2つ目が 200 で綴りは在るが `meta` が無い」上流
        // （vLLM / Ollama の OpenAI 互換シム / 古い `llama-server` ビルド）。
        // **1本足すと、起動のたびに2本＋5分ごとの引き直しでも鳴る。**
        //
        // **⚠ 訂正: この腕はいま完全に静かである**（作業単位 G-9 / 2026-09-07）。
        //
        // **G-5 の時点では、この腕にも警告が1本あった**——`llm::response` の
        // 「OpenAI 互換のモデル一覧に実効コンテキスト長がありません」（作業単位 G-2）。
        // **それが「5分ごとに鳴っている族」そのもの**で、G-9 が `debug` へ落とした。
        //
        // **`llm::response` の doc がその判断の権威**
        // （`parse_effective_context_length` の「読めなかったことを、この層は警告にしない」）。
        // ここが期待するのは**空の一覧**であって、
        // **どちらの層が1本足しても、この `assert_eq!` は必ず赤くなる。**
        //
        // **本数ではなく、出した層の一覧を丸ごと固定する**のは変えていない
        // ——**「WARN が何本か」ではなく「どの層が言っているか」を見ること**
        // （CLAUDE.md「その行が出た」と「その行がこう言っている」は別）。
        let warned: Vec<&str> = output
            .lines()
            .filter(|line| has_field(line, "WARN"))
            .map(emitter_of)
            .collect();
        assert_eq!(
            warned,
            Vec::<&str>::new(),
            "綴りだけが取れた回に、どの層からも警告を出さないこと: {output:?}"
        );

        let report = report.expect("一覧だけは取れているので往復は成功すること");
        assert_eq!(
            report.catalogue(),
            ["some-model".to_owned()],
            "綴りの一覧は持ち帰ること"
        );
    }

    // ------------------- 本文の中継の深刻度（4周目の是正 / 2026-09-06）
    //
    // **[`RequestFailureVoice`] を渡す口は5つある**（あの型の表）。
    // 3周目に見張られていたのは**読み手の2つだけ**で、
    // **本文の中継の3つは `Speak` を `Hush` へ潰す変異が 1949 全緑**だった。
    //
    // **⚠ 実害は「完全な沈黙」ではない。** [`LlmError`] は全経路が
    // `server::chat` の `upstream_error` を通り、そこで `error!` が出る。
    // **消えるのはこの層の1行だけ**——だが、`reqwest` が言った中身
    // （**どの綴りへ・どう失敗したか**）を持っているのはそちらである。
    //
    // **それでも押さえが要る理由は、3周目が「その場で深刻度を名乗ってよいか」の判断を
    // 新しく5箇所へ生やしたこと**そのもの。
    // **今日の値は正しいが、doc が書いた境界を次の編集から守るものが何も無い。**

    /// 本文の中継が失敗する筋書き。**どれも wiremock では作れない。**
    #[derive(Debug, Clone, Copy)]
    enum RelayScript {
        /// **誰も待ち受けていない。** 要求が来る前に畳む
        /// （= LM Studio が停止している普段の状態 / docs/04-environment.md）。
        ///
        /// [`LmStudioClient::post`] の `send` が**接続失敗**になる
        /// ——**[`RequestFailureVoice::Speak`] の接続失敗の腕を通す唯一の筋書き**
        /// （作業単位 G-8/G-9 のレビューの是正 / 2026-09-07）。
        ///
        /// # ⚠ この腕にだけ [`RELAY_BOUNDED`] を掛けないこと（**実測**）
        ///
        /// **この機では、待ち受けの無い loopback への接続が拒否されるまで約 2 秒かかる**
        /// （Windows は RST を即座に返さず SYN を再送する / `python` の素の
        /// `socket.connect` で 2054 / 2032 / 2025 ms を実測 / 2026-09-07）。
        /// **200 ms を掛けると、接続が失敗する前に要求の上限が切れる** ——
        /// 実際に `warn` の「応答が時間内に完了しませんでした」が出て、
        /// **接続失敗の腕ではなくタイムアウトの腕を測っていた。**
        ///
        /// **接続が失敗する限り、待たされる上限は
        /// [`super::DEFAULT_CONNECT_TIMEOUT`]（5 秒）**なので、
        /// **上限を被せなければ、それが天井になる。**
        /// **実測はこの1本で 2.05 / 2.04 / 2.03 秒**（OS の接続拒否が先に来る / 最終レビュー）。
        ///
        /// **⚠ 「接続が失敗する限り」は外せない条件である**（最終レビューの指摘 / 2026-09-07）。
        /// **畳んだ綴りを誰かが掴めば接続は成立し、そのときこの筋書きに天井は無い** ——
        /// **同じプロセスの [`RELAY_HOLD`]（30 秒）に掴まれれば 30 秒、
        /// 別プロセスなら [`super::DEFAULT_IDLE_TIMEOUT`]（180 秒）側。**
        /// **⚠ `CLAUDE.md` の「上限は働く側ではなく待つ側に要る」とは向きが逆である。**
        /// **それでも定数を置き直さないのは、落ち方が赤であって無限待機ではないから**
        /// （下の「見分け方」はその 30 秒の経路でも効く）。**頻度は測っていない。**
        ///
        /// **⚠ 姉妹の [`query_models_over_wire`] は「答えを持っていた」わけではない**
        /// （最終レビューの訂正）。**あちらはどの筋書きにも
        /// [`LmStudioConfig::with_request_timeout`] を一度も呼んでいない**ので、
        /// **「接続失敗の腕にだけ被せない」を選んだ前例ではなく、被せる仕組みが無いだけ。**
        /// **一致しているのは結果の形だけである**
        /// （3周目の是正 / 実測由来の 10 秒の定数を1つ消した / ADR-0036）。
        NeverListens,
        /// 要求は受け取るが、**上限まで何も返さない。**
        /// [`LmStudioClient::post`] の `send` が時間切れになる。
        NeverAnswers,
        /// ヘッダは 200 で返し、**本文を送り切らずに閉じる。**
        /// [`LmStudioClient::complete`] の `text` が失敗する。
        TruncatesTheBody,
        /// SSE を開いて増分を1つ流し、**送り切らずに閉じる。**
        /// [`map_event_error`] の `Transport` が拾う（**中継の途中**）。
        TruncatesTheEventStream,
    }

    /// 応答を返さない筋書きが接続を握っておく時間。**上限より十分に長いこと。**
    const RELAY_HOLD: Duration = Duration::from_secs(30);

    /// 本文の応答を待つ上限。**待たされるのはここまで。**
    const RELAY_BOUNDED: Duration = Duration::from_millis(200);

    // **[`RelayScript::NeverListens`] 専用の上限は置かない**（3周目の是正 / 2026-09-07）。
    //
    // **2周目までは `RELAY_CONNECT_BOUNDED = 10 秒` を置いていた**が、
    // **doc が「待たされる本当の上限は `DEFAULT_CONNECT_TIMEOUT`（5 秒）なので
    // 実際には効かない」と書きながら、その関係を守るものが無かった**
    // （G-9 のレビューの指摘）。**効かない定数は消した。**
    // **⚠ ただし「効かない」が成り立つのは接続が失敗する限りで、
    // 姉妹の `query_models_over_wire` は前例ではない**（どちらも最終レビューの訂正）。
    // **条件と、姉妹との違いは [`RelayScript::NeverListens`] の doc。**

    /// 中継に使う要求。**中身は問わない**（失敗するのは本文が返る前後）。
    fn relay_request() -> ChatRequest {
        ChatRequest::new(vec![
            crate::domain::ChatMessage::system("あなたは葵です"),
            crate::domain::ChatMessage::user("こんにちは"),
        ])
    }

    /// **本文が `Content-Length` に足りない** 200（送り切らずに閉じる）。
    ///
    /// **ステータスは 200 なので [`LmStudioClient::post`] の
    /// 「ステータス異常」の枝は通らない**——失敗するのは `text()` の側だけ。
    fn truncated_completion_wire() -> String {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"こんにちは"}}]}"#;
        format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len() + 64
        )
    }

    /// SSE を開いて**増分を1つ流してから**、送り切らずに閉じる。
    ///
    /// **1つ流すのは、中継が始まっていることを主張の一部にするため**
    /// （開く前に落ちたのなら、それは [`LmStudioClient::post`] の腕である）。
    fn truncated_event_stream_wire() -> String {
        let body = concat!(r#"data: {"choices":[{"delta":{"content":"こ"}}]}"#, "\n\n");
        format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/event-stream\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len() + 64
        )
    }

    /// ヘッダの終わり（`\r\n\r\n` の**直後**）の位置。
    fn head_end(request: &[u8]) -> Option<usize> {
        request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|at| at + 4)
    }

    /// 要求が申告した本文の長さ（無ければ 0）。
    fn declared_length(head: &str) -> usize {
        head.lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse().ok())
            .unwrap_or(0)
    }

    /// 補完の要求を1つ**本文まで**受け取り、指定の応答を返して閉じる。
    ///
    /// `response` が `None` なら**何も返さずに握ったまま**（[`RELAY_HOLD`]）。
    /// **閉じてしまうと通信の失敗になり、測りたいタイムアウトの腕を通らない。**
    async fn serve_chat_once(listener: &tokio::net::TcpListener, response: Option<&str>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut socket, _) = listener.accept().await.expect("接続を受けられること");
        let mut request = Vec::new();
        let mut buf = [0u8; 1024];
        let head = loop {
            let read = socket.read(&mut buf).await.expect("要求を読めること");
            assert_ne!(read, 0, "要求が途中で切れないこと: {request:?}");
            request.extend_from_slice(&buf[..read]);
            if let Some(end) = head_end(&request) {
                let head = String::from_utf8_lossy(&request[..end]).into_owned();
                // **POST なので、本文まで受け切って初めて要求が終わる。**
                if request.len() >= end + declared_length(&head) {
                    break head;
                }
            }
        };
        assert!(
            head.starts_with(&format!("POST {CHAT_COMPLETIONS_PATH} ")),
            "中継の綴りであること: {head:?}"
        );

        match response {
            Some(response) => {
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("応答を書けること");
                socket.flush().await.expect("送り切れること");
            }
            None => tokio::time::sleep(RELAY_HOLD).await,
        }
    }

    /// ストリームを最後まで飲む。**最後の項目がこの往復の結末。**
    async fn drain_stream(client: &LmStudioClient) -> Result<(), LlmError> {
        let mut stream = LmStudioClient::stream(client, &relay_request()).await?;
        let mut last = Ok(());
        while let Some(item) = stream.next().await {
            last = item.map(|_| ());
        }
        last
    }

    /// 生の上流を1つ立てて、**本文の中継**を1回行う。
    fn relay_over_wire(filter: &str, script: RelayScript) -> (String, Result<(), LlmError>) {
        let runtime = runtime();
        let mut outcome = None;
        let output = crate::testlog::captured_with_filter(filter, || {
            runtime.block_on(async {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("待ち受けを開けること");
                let base = format!(
                    "http://{}",
                    listener.local_addr().expect("待ち受け先を読めること")
                );
                let response = match script {
                    RelayScript::NeverListens | RelayScript::NeverAnswers => None,
                    RelayScript::TruncatesTheBody => Some(truncated_completion_wire()),
                    RelayScript::TruncatesTheEventStream => Some(truncated_event_stream_wire()),
                };
                let upstream = match script {
                    RelayScript::NeverListens => {
                        // **要求が来る前に畳む。** 以後この綴りへは接続できない。
                        drop(listener);
                        None
                    }
                    _ => Some(tokio::spawn(async move {
                        serve_chat_once(&listener, response.as_deref()).await
                    })),
                };

                let config = LmStudioConfig::new(&base, model()).expect("ベース URL は正当");
                let client = LmStudioClient::new(match script {
                    // **接続そのものが失敗する筋書きには、要求の上限を被せない。**
                    // 被せると**接続が拒否される前に切れて、タイムアウトの腕を測る**
                    // ——理由は [`RelayScript::NeverListens`] の doc。
                    // 天井は `LmStudioConfig` の既定（`DEFAULT_CONNECT_TIMEOUT` / 5 秒）。
                    RelayScript::NeverListens => config,
                    _ => config.with_request_timeout(RELAY_BOUNDED),
                })
                .expect("クライアントを作れること");

                outcome = Some(match script {
                    RelayScript::TruncatesTheEventStream => drain_stream(&client).await,
                    _ => LmStudioClient::complete(&client, &relay_request())
                        .await
                        .map(|_| ()),
                });

                // **待ち受けを立てていない筋書きには、待つものが無い。**
                if let Some(upstream) = upstream {
                    match script {
                        // **握ったままなので、待たずに畳む**（[`RELAY_HOLD`] を待たない）。
                        RelayScript::NeverAnswers => upstream.abort(),
                        // **上限つきで待つ**——理由は [`UPSTREAM_JOIN_LIMIT`]。
                        _ => join_upstream(upstream).await,
                    }
                }
            });
        });
        (output, outcome.expect("往復が済んでいること"))
    }

    /// **停止している上流への中継も、警告ではない**（G-8/G-9 のレビューの是正 / 2026-09-07）。
    ///
    /// # ⚠ この1本が無いと、`Speak` の接続失敗は完全に無防備だった（**実測**）
    ///
    /// 着手前は [`LmStudioClient::models`] の呼び出し口が
    /// [`RequestFailureVoice::Speak`] だったので、
    /// [`a_stopped_upstream_is_not_a_warning`] が**接続失敗 × `Speak`** を押さえていた。
    /// **G-8 ① がその口を [`RequestFailureVoice::Murmur`] へ替えた瞬間、
    /// あのテストは `Murmur` の側へ移り、`Speak` の側を見る者が誰も居なくなった。**
    ///
    /// **実測（レビュアーの `m8`）: 接続失敗の `Speak | Murmur` の腕を割って
    /// `Speak` だけを `warn!` へ上げる変異が passed=1957 failed=0 / exit=0。**
    /// **同じ変異を着手前の版に当てると 1 赤**（[`a_stopped_upstream_is_not_a_warning`]）。
    ///
    /// # 実害は「WARN が1本増える」では済まない
    ///
    /// **LM Studio は普段落としてある**（docs/04-environment.md）。その状態で
    /// `POST /v1/chat/completions` が来ると、**チャット1本ごとに**この行が出る。
    /// **G-8/G-9 が消しにきた族そのもので、頻度は「起動時と5分ごと」より桁違いに高い。**
    ///
    /// # ⚠ この1本は、変異と無関係に赤くなりうる（**見分け方つき** / 3周目の是正）
    ///
    /// [`relay_over_wire`] は `TcpListener` の綴りを控えてから畳むので、
    /// **窓の間に別の待ち受けが同じポートを掴む**と接続が成立してしまう
    /// （**この機は SYN 再送で約 2 秒かかるので窓が広い**）。
    /// **G-9 のレビュアーが、無変異の 22 回中 1 回で観測した**
    /// ——結末が [`LlmError::Connect`] ではなく通信失敗になり、
    /// `Speak` なので `warn` が1本出る。
    ///
    /// **⚠ ポートの取り合いそのものは直していない**
    /// （**既存の [`WireScript::NeverListens`] が同じ形**なので、直すなら別の作業単位）。
    /// **代わりに、外から見分けられるようにした** ——
    /// **結末の検査を先頭に置いてある**ので、取り合いで落ちたときは
    /// **「接続失敗として区別されること」**が最初に鳴る。
    /// **深刻度の検査（`推論サーバへ接続できませんでした` を含む行）が鳴ったなら、
    /// それは取り合いではなく、[`map_request_error`] の側が変わっている。**
    ///
    /// **見分けの補助**: 出力に
    /// 「**推論サーバとの通信に失敗しました**」の行が在れば、それは取り合いの側。
    ///
    /// [`a_stopped_upstream_is_not_a_warning`]: a_stopped_upstream_is_not_a_warning
    #[test]
    fn a_relay_to_a_stopped_upstream_is_not_a_warning() {
        let (output, outcome) =
            relay_over_wire(crate::server::DEFAULT_LOG_FILTER, RelayScript::NeverListens);

        // **⚠ ここを最初に置くこと**（3周目の是正）——理由はこのテストの doc。
        // **後ろに置くと、ポートの取り合いで落ちた回と
        // 「接続失敗の水準が変わった」回が、同じ文面で落ちて区別が付かない。**
        assert!(
            matches!(outcome, Err(LlmError::Connect(_))),
            "接続失敗として区別されること。\
             ここで落ちたなら、まず**ポートの取り合い**を疑うこと\
             （下の出力に「推論サーバとの通信に失敗しました」が在れば、\
             畳んだ綴りを別の待ち受けが掴んでいる）: {outcome:?} / {output:?}"
        );

        // 肯定側——**その事実は現に1行出ている**（行ごと落とす変異と区別が付くこと）。
        //
        // **文面だけで選ばないこと**——理由は [`line_said_by_this_layer`] の doc。
        let line = line_said_by_this_layer(&output, CONNECT_FAILED_MARKER);
        assert!(
            has_field(line, "INFO"),
            "停止している上流への中継は info であること: {line:?}"
        );
        assert!(
            line.contains(CHAT_COMPLETIONS_PATH),
            "reqwest が言った中身（どの綴りで失敗したか）まで残っていること: {line:?}"
        );

        // 否定側——既定のフィルタには、どの層からも1本も出さない。
        assert_eq!(
            warning_emitters(&output),
            Vec::<&str>::new(),
            "停止しているだけの上流への中継で WARN を出さないこと: {output:?}"
        );
    }

    /// **応答が1つも返らない中継は、この層がその場で深刻度を名乗る**（4周目の是正）。
    ///
    /// [`LmStudioClient::post`] の `send`。**`complete` と `stream` の両方が通る唯一の口**で、
    /// 3周目は `Hush` へ潰す変異が 1949 全緑だった。
    #[test]
    fn a_relay_that_never_gets_an_answer_names_its_own_severity() {
        let (output, outcome) =
            relay_over_wire(crate::server::DEFAULT_LOG_FILTER, RelayScript::NeverAnswers);

        assert_eq!(
            warning_emitters(&output),
            ["personacore::llm::lmstudio:"],
            "中継が時間切れになったら、この層が1本言うこと: {output:?}"
        );
        // **文面だけで選ばないこと**——理由は [`line_said_by_this_layer`] の doc。
        let line = line_said_by_this_layer(&output, TIMED_OUT_MARKER);
        assert!(
            has_field(line, "WARN"),
            "本文が返らないのは warn であること: {line:?}"
        );
        assert!(
            line.contains(CHAT_COMPLETIONS_PATH),
            "reqwest が言った中身（どの綴りで失敗したか）まで残っていること: {line:?}"
        );
        assert!(
            matches!(outcome, Err(LlmError::Timeout(_))),
            "タイムアウトとして区別されること: {outcome:?}"
        );
    }

    /// **本文の途中で切れた中継も、この層がその場で深刻度を名乗る**（4周目の是正）。
    ///
    /// [`LmStudioClient::complete`] の `text`。
    /// **ステータスは 200 なので、[`LmStudioClient::post`] の警告とは別の腕。**
    #[test]
    fn a_relay_cut_off_mid_body_names_its_own_severity() {
        let (output, outcome) = relay_over_wire(
            crate::server::DEFAULT_LOG_FILTER,
            RelayScript::TruncatesTheBody,
        );

        assert_eq!(
            warning_emitters(&output),
            ["personacore::llm::lmstudio:"],
            "本文が途中で切れたら、この層が1本言うこと: {output:?}"
        );
        let line = line_said_by_this_layer(&output, TRANSPORT_FAILED_MARKER);
        assert!(
            has_field(line, "WARN"),
            "本文が途中で切れるのは warn であること: {line:?}"
        );
        assert!(
            matches!(outcome, Err(LlmError::Transport(_))),
            "通信の失敗として区別されること: {outcome:?}"
        );
    }

    /// **SSE の中継中に切れたときも、この層がその場で深刻度を名乗る**（4周目の是正）。
    ///
    /// [`map_event_error`] の `Transport`。**中継が始まった後**なので、
    /// 深刻度を決められる側は他に居ない。
    ///
    /// # `Truncated` の腕と取り違えないこと
    ///
    /// 上流が**きれいに**終わったなら `chunk_stream` の
    /// 「完了前に終了しました」（[`LlmError::Truncated`]）である。
    /// ここで測っているのは**フレーミングごと壊れた**側で、
    /// `Content-Length` を満たさずに閉じることで作り分けている。
    #[test]
    fn a_stream_cut_off_mid_relay_names_its_own_severity() {
        let (output, outcome) = relay_over_wire(
            crate::server::DEFAULT_LOG_FILTER,
            RelayScript::TruncatesTheEventStream,
        );

        assert_eq!(
            warning_emitters(&output),
            ["personacore::llm::lmstudio:"],
            "中継中に切れたら、この層が1本言うこと: {output:?}"
        );
        let line = line_said_by_this_layer(&output, TRANSPORT_FAILED_MARKER);
        assert!(
            has_field(line, "WARN"),
            "中継が壊れるのは warn であること: {line:?}"
        );
        assert!(
            matches!(outcome, Err(LlmError::Transport(_))),
            "通信の失敗として区別されること（`Truncated` ではない）: {outcome:?}"
        );
    }
}
