//! HTTP サーバ（axum）。OpenAI 互換のルーティング。
//!
//! この層は**薄く保つ**（CLAUDE.md 3節）。判断は `domain` / `prompt` / `parse` に置き、
//! ここは「受ける・呼ぶ・返す」だけを担う。SQL は書かない（必要なら `store` に足す）。
//!
//! # 起動と停止
//!
//! ```text
//! init_tracing → AppConfig::from_env → App::run
//!   ├ bind  … 待ち受け開始（明示していない番号が塞がっていれば次の空きへ / `port_search`）
//!   │         → 失敗しうる構築 → **最後に** DB接続＋マイグレーション
//!   └ serve … 引き直しの開始 → graceful shutdown まで応答
//!             → 引き直しの停止 → Store::close
//! ```
//!
//! **コンテキスト長の引き直し（ADR-0020）は `serve` が持つ。** 背景タスクなので
//! 応答の経路には入らず、`serve` を抜けるときに落ちる（タスクを leak させない）。
//!
//! **停止時に `Store::close()` を必ず通す。** WAL のデータは `-wal` 側に残るため、
//! 閉じずに落とすと失われる（docs/04-environment.md）。`serve` は正常終了でも
//! 異常終了でも閉じる。
//!
//! `bind` に「閉じてから返す」分岐は無い。**`Store` を最後に開くので、
//! それより前で `?` を書いても閉じる対象が存在しない。**
//! 失敗しうる構築（上流クライアント等）は `Store::connect` より前に足すこと。
//!
//! **本番の起動経路（`main`）は `App` を手に持たない。** `bind` と `serve` の間で
//! 早期 return を書くと、待ち受け済みの `App` を落としたまま関数を抜ける
//! （= `Store` を閉じない）経路ができる。`App::run` に閉じ込めて、
//! そもそも書けないようにしてある。
//!
//! # バインドアドレス
//!
//! **`127.0.0.1` 固定**（絶対制約）。設定できるのはポートだけで、
//! ループバック以外は `BindAddr` の型として表現できない（`config` モジュールの doc）。

mod admin;
mod agreement_proposal;
mod axis_discovery;
mod axis_trial;
mod body;
mod browser;
mod chat;
mod config;
mod console;
mod context;
mod counted_plan;
mod drift;
mod error;
mod extract;
mod fresh;
mod host_guard;
mod instance_lock;
mod launch;
mod listening_port;
mod models;
mod openapi;
mod port_search;
mod profile_editor;
mod relay;
mod remembered;
mod seal;
mod switch;
mod telemetry;
mod turn;
mod upstream;
mod upstreams;
mod usage;
mod web;

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;

pub use config::{
    APP_DATA_DIR_NAME, AppConfig, BindAddr, ConfigError, DATABASE_FILE_NAME, DEFAULT_PORT,
    DEFAULT_WEB_DIR, ENV_AXIS_DISCOVERY_MEMORIES, ENV_AXIS_TRIAL_LINE_PERCENT,
    ENV_AXIS_TRIAL_MIN_NONZERO, ENV_AXIS_TRIAL_TURNS, ENV_CONTEXT_TOKENS, ENV_DATABASE_PATH,
    ENV_DRIFT_DECAY, ENV_DRIFT_FORM, ENV_DRIFT_NOTE, ENV_DRIFT_SLOPE, ENV_DRIFT_STEP_COST,
    ENV_LLM_ENDPOINT, ENV_LLM_IDLE_TIMEOUT, ENV_MAIN_MODEL, ENV_PORT, ENV_STAT_DRIFT,
    ENV_SUB_LLM_ENDPOINT, ENV_SUB_MODEL, ENV_UPSTREAMS, ENV_WEB_DIR, PortSource, SAVED_PORT_MIN,
    SavedPort, database_path_from, database_path_from_env, database_path_under, os_data_dir,
    port_in_use_message, ports_in_use_message, resolve_port,
};
/// **起動に失敗したとき、ダブルクリックで開いた窓を理由と一緒に消さない**（作業単位 B-76 / `console` モジュール）。
pub use console::{PRESS_ENTER_TO_CLOSE, wait_before_closing_if_alone};
pub use error::ApiError;
/// **同じデータを使う personacore を2つ動かさない**（ADR-0057 §決定2 / 作業単位 F-11a / `instance_lock` モジュール）。
///
/// **`main` が起動の最初に [`acquire_beside`] を呼び、取れたら [`Startup::holding`] で渡す。**
/// **URL を書く口（`InstanceLock::announce`）は公開しない**——書くのは待ち受けを開いた後の
/// [`App::run_with_upstreams`] だけ。
pub use instance_lock::{
    Acquired, InstanceLock, InstanceLockError, LOCK_FILE_NAME, OtherInstance, RUNNING_FILE_NAME,
    acquire_beside, guide_second_start, lock_dir_of,
};
/// **待ち受けのポートが使われていたら、空いている次の番号で待ち受ける**（ADR-0057 §決定1・3 /
/// 作業単位 F-11b / `port_search` モジュール）。**探すのは `App::bind` の中だけ**（公開するのは範囲の幅と、
/// 範囲を全部試しても開けなかったときの `#[source]`）。
pub use port_search::{PORT_SHIFT_SPAN, PortsTaken};

/// **失敗の応答の形の一覧**（API の形の指紋のため / 作業単位 O-7 / `error::error_catalog`）。
#[cfg(any(test, feature = "testing"))]
pub use error::{ErrorShape, error_catalog};

/// **API の仕様書（OpenAPI 3.2）**（ADR-0047 §② / 作業単位 P-2a / `openapi` モジュール）。
pub use openapi::{API_VERSION, API_VERSION_HEADER, DocumentError, api_document};

/// **仕様書のキーワードを、名前の位置を避けて落とす**（API の形の指紋のため / 作業単位 O-7）。
#[cfg(any(test, feature = "testing"))]
pub use openapi::strip_keywords_for_tests;

/// **初期化の退避**（ADR-0041 §決定4 / `fresh` モジュール）。
///
/// `main` はここを結線するだけ。**DB を開く前**に呼ぶこと。
pub use fresh::{
    Args, ArgsError, FRESH_FLAG, FreshError, NO_BROWSER_FLAG, NO_UI_FLAG, UPSTREAM_FLAG,
    VERSION_FLAG, VERSION_LINE, parse_args, stash_database,
};
pub use telemetry::{DEFAULT_LOG_FILTER, TelemetryError, init_tracing};

/// **上流について観測できたこと**（作業単位 G-3b / `upstream` モジュール）。
///
/// `GET /admin/upstream`（作業単位 G-10）と画面（D-9）がここを読む。
///
/// **いつ・どう確かめたか**（`CheckOutcome` / `UpstreamCheck`）は作業単位 B-66 で足した。
pub use upstream::{CheckOutcome, UpstreamCheck, UpstreamFacts, UpstreamObservation};

/// **起こした上流の面ごとの状態**（ADR-0042 §決定3 / `launch` モジュール）。
///
/// `GET /admin/upstream` がこれを言う。**観測（`UpstreamFacts`）とは別の主張**——
/// あちらは「上流が何と答えたか」、こちらは「この実行ファイルが何をしたか」。
///
/// # ⚠ 共有ハンドルは再公開していない（レビュー是正5 / 作業単位 F-4b-1）
///
/// **`App::upstream_handle` にあたるものを生やしていない。**
/// **読む口は `GET /admin/upstream` だけ**で、統合テストもそこを通る
/// （`tests/upstream_launch.rs`）——**本物の起動を通さずに状態だけ覗く道を作らない。**
///
/// **⚠ 初版は `SharedLaunchStates` を `testing` の下で再公開し、doc が**
/// **`App::launch_handle` と対だと書いていたが、そのメソッドは存在せず、利用者も0だった。**
/// **要るようになったら、そのとき対のアクセサごと足すこと。**
pub use launch::{FaceState, LaunchError, LaunchPlan, LaunchStates, UPSTREAM_LOG_DIR_NAME};

/// **上流プロファイル**（ADR-0042 §決定1 / 作業単位 F-4b-1 / `upstreams` モジュール）。
///
/// `main` が上流を選ぶために読むのは、`--upstream <名前>` か覚えた名前があるときだけ。
/// **番号を明示していない起動は、待ち受けを探すときに飛ばす番号のためにも読む**（[`launch_ports_at`] /
/// ADR-0057 §決定3 / 作業単位 F-11b）——**その中身で上流を決めることはしない。**
pub use upstreams::{
    DEFAULT_COMMAND, FACE_KEYS, FACE_NAMES, FaceLaunch, ProfileError, ProfileFace, ProfileLaunch,
    ProfileSettings, SAMPLE_PROFILES, SampleOutcome, SampleWriteError, UPSTREAMS_FILE_NAME,
    UpstreamProfile, UpstreamProfiles, home_dir, launch_ports_at, load_profiles, overlay_profile,
    parse_profiles, upstreams_path_under, write_sample_if_missing,
};

/// **最後に選んだ上流プロファイルの名前**（ADR-0042 §決定9 / 作業単位 F-4f / `remembered` モジュール）。
///
/// **`main` が読むだけ。** 書くのはこのモジュールの中（`App::run_with_upstreams` と差し替え）で、
/// **書く口は公開しない**——覚える先は [`Startup::with_editable_profiles_file`] の置き場所からしか作られない。
pub use remembered::{
    REMEMBERED_FILE_NAME, RememberedUnreadable, read_remembered, remembered_path_beside,
};

/// **画面で保存した待ち受けのポート**（ADR-0057 §決定5 / 作業単位 F-12a / `listening_port` モジュール）。
///
/// **`main` が読むだけ。** 書くのは `PUT /admin/server/port`（このモジュールの中）で、
/// **書く口は公開しない**——保存する先は [`Startup::with_editable_profiles_file`] の置き場所からしか作られない。
pub use listening_port::{
    LISTENING_PORT_FILE_NAME, SavedPortUnreadable, read_saved_port, saved_port_path_beside,
};

/// 上流の観測の共有ハンドル。**テストからだけ見える**（`App::upstream_handle` と対）。
///
/// 本番から観測を書き換える手段は生やさない（`context::SharedUpstream` の doc）。
#[cfg(any(test, feature = "testing"))]
pub use context::SharedUpstream;

/// 共有のトークン予算。**テストからだけ見える**（`App::budget_handle` と対）。
///
/// 本番から予算を書き換える手段は生やさない（`context::SharedBudget` の doc）。
#[cfg(any(test, feature = "testing"))]
pub use context::SharedBudget;

/// 抽出の窓の共有ハンドル。**テストからだけ見える**（`App::sub_context_handle` と対 / 作業単位 A-13a-1）。
///
/// **読むだけ**（`tokens_for_tests`）。本番から抽出の上限を書き換える手段は生やさない。
#[cfg(any(test, feature = "testing"))]
pub use context::SharedSubContext;

/// 裏方の枠が空いたかを外から見る口。**テストからだけ見える**（`App::background_probe` と対 / 作業単位 A-8）。
#[cfg(any(test, feature = "testing"))]
pub use extract::BackgroundProbe;

/// 抽出の依頼の**予算と上流のつまみ**。**テストからだけ見える**（作業単位 21a）。
///
/// `tests/extraction_quality_harness.rs`（実機で締めの言い方の腕を比べる器具）が、
/// **温度・応答の上限・予算・§8 の門を本番と同じ経路で通す**ために使う。
/// **器具が値を写すと、本番が変わったときに器具だけが黙って古びる。**
///
/// `response_completeness_for_tests` も同じ理由（作業単位 24）——
/// **上流の停止理由を「最後の行が断片か」に翻訳する規則**を器具に写させない。
#[cfg(any(test, feature = "testing"))]
pub use extract::{
    assumed_sub_context_tokens_for_tests, extraction_prompt_budget_for_tests,
    extraction_request_for_tests, response_completeness_for_tests,
};

use crate::domain::{Character, StatDriftMode};
use crate::llm::{LlmClient, LlmError, LmStudioClient};
use crate::measured;
use crate::prompt::TokenBudget;
use crate::store::{Store, StoreError};

/// 起動・実行に失敗した理由。
///
/// `Display` は**定数文字列**。詳細は `#[source]` 側に残る（ログには出せる）。
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// DB に接続できない、またはマイグレーションが通らない。
    #[error("データベースを初期化できませんでした")]
    Store(#[source] StoreError),

    /// **DB を置くディレクトリを作れない**（ADR-0041 §決定3 / 作業単位 F-4a）。
    ///
    /// **[`Self::Store`] と分けてある。** 既定は OS の作法の場所なので、
    /// **初回はディレクトリごと作る**——そこで落ちたときの直す手
    /// （置き場所か権限）は、DB を開けなかったときとは違う。
    #[error("データベースの置き場所を作れませんでした")]
    DatabaseDir(#[source] std::io::Error),

    /// 待ち受けアドレスを開けない（ポートが使用中など）。
    ///
    /// **番号を明示した起動（`PERSONACORE_PORT`）か、ずらす対象でない失敗**（ADR-0057 §決定1・3）。
    /// 明示していない起動が範囲を全部試して開けなかったときは [`Self::PortsTaken`]。
    #[error("待ち受けアドレスを開けませんでした")]
    Bind(#[source] std::io::Error),

    /// **番号を明示していない起動が、ずらす範囲（元の番号〜`+`[`PORT_SHIFT_SPAN`]）を全部試しても
    /// 待ち受けを開けなかった**（ADR-0057 §決定3 / 作業単位 F-11b）。
    ///
    /// **`Display` は [`Self::Bind`] と同じ定数文字列**（どちらも「待ち受けを開けなかった」）。範囲は
    /// [`Self::tried_ports`] で読む——`main` はそれで [`ports_in_use_message`] を足す。
    #[error("待ち受けアドレスを開けませんでした")]
    PortsTaken(#[source] PortsTaken),

    /// 実行中に HTTP サーバが失敗した。
    #[error("HTTP サーバの実行に失敗しました")]
    Serve(#[source] std::io::Error),

    /// 上流（推論サーバ）クライアントを構築できない。
    ///
    /// **通信の失敗とは別物。** これは TLS / リゾルバの初期化など**起動時の構成エラー**で、
    /// 「LM Studio を起動してください」ではなく「環境を確認してください」の案内になる
    /// （docs/06-roadmap.md の `llm` からの持ち越し）。
    #[error("推論サーバのクライアントを構築できませんでした")]
    LlmClient(#[source] LlmError),

    /// **名指しされた上流を起こせない**（ADR-0042 §決定2 / 作業単位 F-4b-1）。
    ///
    /// **`--upstream` を渡した起動でしか出ない。** 渡していない起動には
    /// このバリアントへ来る道が無い（`launch::LaunchPlan::none`）。
    ///
    /// **[`Self::LlmClient`] と分けてある。** あちらは「クライアントを組めない」で、
    /// こちらは「gguf が無い／`llama` が PATH に無い」——直す手がまるで違う。
    #[error("名指しされた上流を起こせませんでした")]
    Launch(#[source] launch::LaunchError),
}

impl ServerError {
    /// **待ち受けのポートを、他のプログラムが使っていたので開けなかったか**
    /// （作業単位 B-76 / marukei/personacore#13）。
    ///
    /// **`Display` は変えない**（定数文字列のまま）。ポートの番号と直し方を足すのは `main`
    /// （[`port_in_use_message`]）で、**この判定が真のときだけ**足す——権限などの他の失敗の文は変わらない。
    ///
    /// Windows の `WSAEADDRINUSE`（10048）は、標準ライブラリが [`std::io::ErrorKind::AddrInUse`] に写す
    /// （単体テストの `windows_reports_an_occupied_port_as_addr_in_use` が確かめる）。
    pub fn is_port_in_use(&self) -> bool {
        matches!(self, Self::Bind(source) if source.kind() == std::io::ErrorKind::AddrInUse)
    }

    /// **ずらす範囲を全部試して開けなかったなら、その範囲**（試し始めた番号, 範囲の最後の番号）
    /// （ADR-0057 §決定3 / 作業単位 F-11b）。
    ///
    /// **[`Self::is_port_in_use`] とは重ならない**——あちらは明示した1つの番号の話（B-76 の文）、
    /// こちらは範囲の話（[`ports_in_use_message`]）。`main` はこちらを先に見る。
    #[must_use]
    pub const fn tried_ports(&self) -> Option<(u16, u16)> {
        match self {
            Self::PortsTaken(taken) => Some((taken.first(), taken.last())),
            _ => None,
        }
    }
}

/// ハンドラが共有する状態。
///
/// **`Store` は `Clone` で同じプールを指す。** 上流クライアントは
/// `Arc<dyn LlmClient>`（トレイト越し / CLAUDE.md 5節）で持つので、
/// テストではモックを注入できる。
#[derive(Clone)]
pub(crate) struct AppState {
    store: Store,
    llm: Arc<dyn LlmClient>,
    budget: context::SharedBudget,
    /// **記憶抽出の仕掛け**（ADR-0021 §6〜10 / `extract` モジュール）。
    ///
    /// **1つを全リクエストで共有する。** 「同時に走らせるのは1件まで」（§7）は
    /// プロセス全体の約束なので、リクエストごとに作ると
    /// **同時に来た数だけ抽出が走り、本文生成の GPU を食い合う。**
    extraction: Arc<extract::MemoryExtractor>,
    /// **UI の配り先**（ADR-0031 / `web` モジュール）。
    ///
    /// **未知パスのハンドラだけが使う。** 他のハンドラから触らないこと
    /// ——`/v1` / `/admin` の応答が UI の有無に左右される形は作らない。
    ui: web::UiFiles,
    /// **上流について観測できたこと**（作業単位 G-3b / `upstream` モジュール）。
    ///
    /// **起動時の写しではない。** 5分ごとの引き直しがここを書き替える
    /// （`context::refresh_once` / `context::refresh_sub_once`）。
    upstream: context::SharedUpstream,
    /// **上流へいま照会する口**（`POST /admin/upstream/check` / 作業単位 B-66 / `context::UpstreamChecker`）。
    ///
    /// **5分ごとの背景タスクと同じ器・同じ照会先・同じ門**（`context::PendingRefresh::checker` の複製）。
    /// **別に組まない** —— 組むと、照会が [`Self::upstream`] ではない器へ書く配線が書ける。
    checker: context::UpstreamChecker,
    /// **自分で起こした上流の、面ごとの状態**（ADR-0042 §決定3 / `launch` モジュール）。
    ///
    /// **[`Self::upstream`] とは別の主張である。** あちらは「上流が何と答えたか」、
    /// こちらは「この実行ファイルが何をしたか」——
    /// **畳むと『まだ載せている最中』と『上流が黙っている』が区別できなくなる。**
    launch_states: launch::SharedLaunchStates,
    /// **この起動が上流プロファイルの見本を書いた先**（作業単位 F-4c）。
    ///
    /// **`None` は「書いていない」。** 起動時に `main` が決めて、以後は変わらない
    /// （見本を書くのは起動時の1回だけ）。**読む口は `GET /admin/upstream`。**
    profile_sample: Option<Arc<std::path::Path>>,
    /// **走ったままの差し替え**（ADR-0042 §決定4 / 作業単位 F-4b-2 / `switch` モジュール）。
    ///
    /// **起こした子の束（`launch::Supervisor`）はこの中に在る。** `serve` の終わりに止める側と
    /// 差し替える側が**同じ錠**を通る（`switch::Switcher` の doc）。
    switcher: Arc<switch::Switcher>,
    /// **上流プロファイルの一覧と保存**（ADR-0042 §決定8 / 作業単位 F-4b-3 / `profile_editor` モジュール）。
    ///
    /// **置き場所は [`Self::switcher`] と同じ1つの値から作る**（`serve`）——
    /// 画面が直したファイルと、差し替えが読むファイルを別々にしない。
    profile_editor: Arc<profile_editor::ProfileEditor>,
    /// **直近のターンで、取り決めが何件本文に載り、何件落ちたか**（キャラクターごと /
    /// 作業単位 A-12a の決め直し）。**DB に持たない**（起動し直すと空 —— 「まだこの起動で
    /// ターンが無い」と言う）。書くのは本文の組み立て（`chat`）、読むのは `GET .../agreements`。
    agreement_fits: Arc<std::sync::Mutex<HashMap<i64, AgreementFit>>>,
    /// **「数えられない」と覚えた本文の面の世代**（ADR-0052 §推奨4 / 作業単位 B-60b / `seal` モジュール）。
    ///
    /// **1つを全リクエストで共有する**（覚えた世代の間は、どのターンも数えに行かない）。
    /// **プロセスの中にしか無い**（DB に持たない / ADR-0052 §推奨6）。
    uncountable: Arc<seal::UncountableMemory>,
    /// **上流が数えた部品の数**（ADR-0052 §推奨6 / 作業単位 B-60d-1 / `counted_plan` モジュール）。
    ///
    /// **1つを全リクエストで共有する**（キャラクターごとに直近のターンの部品を持つ）。
    /// **プロセスの中にしか無い**（DB に持たない）。
    parts: Arc<counted_plan::PartMemory>,
    /// **待ち受けのポートの、いま・保存・次の起動**（ADR-0057 §決定5・6 / 作業単位 F-12a / `listening_port`）。
    ///
    /// **保存の置き場所は [`Self::switcher`] と同じ1つの値（`profiles_file`）から作る**（`serve`）。
    listening: Arc<listening_port::Listening>,
}

/// **直近のターンで載った取り決めの件数**（作業単位 A-12a の決め直し / [`AppState::agreement_fit`]）。
///
/// `character_created_at` は**同じ id で作り直したキャラクター**の取り違えを避ける印
/// （`character.id` は使い回されうる）。読むときにいまのキャラクターと突き合わせる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AgreementFit {
    pub(crate) character_created_at: chrono::DateTime<chrono::Utc>,
    pub(crate) included: usize,
    pub(crate) dropped: usize,
    pub(crate) at: chrono::DateTime<chrono::Utc>,
}

impl AppState {
    /// 直近のターンの取り決めの載り方を覚える（**上書き**。履歴は持たない）。
    ///
    /// 錠が毒されていても panic しない（中身を取り出して続ける —— 件数の記録が壊れる害は小さい）。
    pub(crate) fn record_agreement_fit(
        &self,
        character: &Character,
        included: usize,
        dropped: usize,
    ) {
        let mut fits = match self.agreement_fits.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        fits.insert(
            character.id.get(),
            AgreementFit {
                character_created_at: character.created_at,
                included,
                dropped,
                at: chrono::Utc::now(),
            },
        );
    }

    /// 直近のターンの取り決めの載り方。**この起動でまだターンが無ければ `None`**
    /// （同じ id で作り直したキャラクターのものも `None`）。
    pub(crate) fn agreement_fit(&self, character: &Character) -> Option<AgreementFit> {
        let fits = match self.agreement_fits.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        fits.get(&character.id.get())
            .copied()
            .filter(|fit| fit.character_created_at == character.created_at)
    }
    /// 永続化層。
    pub(crate) const fn store(&self) -> &Store {
        &self.store
    }

    /// **待ち受けのポートについて、この起動が知っていること**（`GET /admin/server` / `PUT /admin/server/port`）。
    pub(crate) fn listening(&self) -> &listening_port::Listening {
        &self.listening
    }

    /// **この往復のあいだ使い続ける上流の写し**（ADR-0052 §推奨3 / 作業単位 B-60b）。
    /// **具象型ではなくトレイト越しに使う。**
    ///
    /// **1ターンに1回だけ呼び、封と送信をその写しで行うこと**（`chat`）。
    /// 2回呼ぶと、間の差し替えで**数えた上流と送った上流が食い違いうる**
    /// （見張り番は `tests/server_seal.rs` の `the_seal_and_the_send_use_one_copy_of_the_upstream`）。
    ///
    /// 写しを返さない実装（[`LlmClient::pinned`] が `None` —— テストのモック）は、そのものを使う。
    ///
    /// **⚠ 先に [`Self::main_generation`] を読むこと**（`context::SharedUpstream::generation` の doc）。
    pub(crate) fn llm_for_turn(&self) -> Arc<dyn LlmClient> {
        self.llm.pinned().unwrap_or_else(|| Arc::clone(&self.llm))
    }

    /// **本文の面の世代**の現在値（ADR-0052 §推奨4 / `context::FaceGeneration`）。
    ///
    /// **[`Self::llm_for_turn`] より先に読むこと**（順の理由は `context::SharedUpstream::generation` の doc）。
    pub(in crate::server) fn main_generation(&self) -> context::FaceGeneration {
        self.upstream.generation(measured::ModelFace::Main)
    }

    /// **「数えられない」と覚えた世代**（`seal` モジュール。B-60d-2 から掛け金も載る）。
    pub(in crate::server) fn uncountable(&self) -> &seal::UncountableMemory {
        &self.uncountable
    }

    /// [`Self::uncountable`] の共有の持ち手（中継の終わりで掛け金を掛けるため / `seal::Sealed`）。
    pub(in crate::server) fn uncountable_shared(&self) -> Arc<seal::UncountableMemory> {
        Arc::clone(&self.uncountable)
    }

    /// **本文の面のいまの世代を、後で読む口**（[`Self::main_generation`] と同じ読み方 / `seal::Sealed`）。
    ///
    /// 応答の後（中継なら流れの終わり）に、記憶へ書く世代がまだいまの世代かを確かめるために読む。
    pub(in crate::server) fn main_generation_source(&self) -> seal::CurrentGeneration {
        let upstream = self.upstream.clone();
        Arc::new(move || upstream.generation(measured::ModelFace::Main))
    }

    /// **上流が数えた部品の数の記憶**（`counted_plan` モジュール）。
    pub(in crate::server) fn parts(&self) -> &counted_plan::PartMemory {
        &self.parts
    }

    /// **本文と裏方の窓の関係**（ADR-0052 §決定7 / `counted_plan::WindowSharing`）。
    ///
    /// **第2層の照会の結果（[`Self::upstream`]）の宛先から決める**ので、走ったままの差し替えに付いてくる。
    /// **[`Self::main_generation`] の直後、[`Self::llm_for_turn`] の前に読む**（`chat`）。
    pub(in crate::server) fn main_window_sharing(&self) -> counted_plan::WindowSharing {
        counted_plan::WindowSharing::of(&self.upstream.get())
    }

    /// **いま効いている裏方の窓**（数えた計画の差し引きの出どころ / ADR-0052 §決定7）。
    /// 抽出器が往復ごとに読むのと同じ器を読む（写しを持たない）。
    pub(in crate::server) fn sub_context(&self) -> extract::SubContext {
        self.extraction.current_sub_context()
    }

    /// トークン予算の**現在値**（ADR-0013 第2層 / ADR-0020 / `context` モジュール）。
    ///
    /// **起動時に決めた値とは限らない。** 明示設定が無いときは背景タスクが
    /// 5分ごとに上流へ引き直しており、上流が小さいコンテキストで再ロードされたら
    /// ここが差し替わる（差し替えなければ窓を超える。LM Studio ではプロンプトの先頭が
    /// 黙って切り捨てられる / ADR-0013。llama は 400 `exceed_context_size_error` で断る /
    /// ADR-0052 M5。ただし、プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる
    /// （`finish_reason=length`））。
    ///
    /// **`TokenBudget::default()` をハンドラから直に呼ばないこと。** 呼ぶと設定も
    /// 上流の実コンテキスト長も無視され、実測 16384 に対して 8192 しか使わなくなる。
    ///
    /// **1リクエストの中では入口で1回だけ読むこと**（`chat`）。組立と答え合わせで
    /// 別の値を読むと、途中で差し替わった回だけ辻褄が合わなくなる。
    ///
    /// **この「1回だけ」に見張り番は無い**（現に本番の呼び出しは `chat` の1箇所だけだが、
    /// リクエストの途中で予算を差し替えるテストがスイートに無いので、`usage` 側で
    /// 読み直す変更を入れても緑のまま）。破れたときの実害は、**引き直しを跨いだ
    /// その1リクエストの答え合わせ**（`usage::report`）が別のコンテキスト長と
    /// 突き合わさり、**その回だけ警告が余分に出る／出ない**ことに留まる。
    pub(crate) fn budget(&self) -> TokenBudget {
        self.budget.get()
    }

    /// 記憶抽出の配線を、このキャラクター向けに作る（ADR-0021 §6）。
    ///
    /// **`TurnSink` に渡す。** 走り出すのは `TurnSink::persist` が
    /// 保存に成功した直後の1箇所だけで、非ストリーミングと中継の両方がそこを通る。
    pub(in crate::server) fn extraction(&self, character: &Character) -> extract::Extraction {
        extract::Extraction::new(Arc::clone(&self.extraction), character)
    }

    /// **上流について観測できたこと**の現在値（作業単位 G-3b）。
    ///
    /// **起動時に決めた値とは限らない。** 明示設定が無い面は背景タスクが
    /// 5分ごとに引き直しており、上流が別のモデルを載せ直したり停止したりすれば
    /// ここが差し替わる（`upstream` モジュールの doc）。
    ///
    /// **読む口は `GET /admin/upstream`**（`upstream::get_upstream` / 作業単位 G-10）。
    /// **書いているのは引き直しだけ**（`context::refresh_once` /
    /// `context::refresh_sub_once`）で、**本番から観測を書き換える手段は無い。**
    ///
    /// **採用しているコンテキスト長はここには無い。**
    /// 本文は [`Self::budget`]、抽出は `MemoryExtractor` が持つ
    /// ——**写しを作らない**（`upstream` モジュールの doc）。
    ///
    /// # `allow(dead_code)` は外してある（作業単位 G-10）
    ///
    /// **G-3b は読む口が無いまま `AppState` へ載せる作業単位だった**ので、
    /// 読み手が来るまでの間だけ黙らせてあった。**`get_upstream` が呼び始めたので外した。**
    ///
    /// # 「別の器を渡す」道が、構文上ここに無い
    ///
    /// **⚠ 「配線そのものは観測されている」とは書けなかった**
    /// （作業単位 G-3b のレビュー指摘 / 2026-09-08）。
    /// **観測されていたのは `App` 側の器**（`App::upstream_handle` が渡す `Arc`）**だけ**で、
    /// **この口を通る見張り番は1本も無かった**——**本文と抽出の面を入れ替える変異が
    /// 1995 全緑・exit 0**（実測 / 2026-09-08）。
    ///
    /// **それでも「別の器」を渡す配線は書けない。** 器を作る
    /// [`context::SharedUpstream::new`] は `context` の private で、
    /// `serve` が `AppState` を組む時点では `self.refresh` が
    /// 直前の `spawn()` で**消費済み**である。**新しい器を作る式が1つも書けない**
    /// ——`SharedSubContext` と同じ論法（`context::PendingRefresh` の doc）。
    ///
    /// **⚠ そちらの穴は G-10 で塞いだ。** 面を入れ替える変異（`m2`）は
    /// **3 赤・exit 101**（`tests/server_admin_upstream.rs` の
    /// `the_two_upstreams_come_back_named_apart` /
    /// `the_response_follows_the_refreshed_observation` /
    /// `the_response_never_shows_the_endpoints_credentials`）。
    /// **G-10 が足した見張り番を全部外すと exit 0 に戻る**（`m2_noguard` / 実測 2026-09-08。
    /// 表は `upstream` モジュールの doc）。
    ///
    /// **⚠ 上の「1995 全緑」は G-3b の回の数で、G-10 では測り直していない。**
    /// **G-10 が自分で測ったのは「いま赤になる」と「見張り番を外すと緑に戻る」の2つ**である。
    pub(crate) fn upstream(&self) -> UpstreamFacts {
        self.upstream.get()
    }

    /// **上流へいま照会する口**（`POST /admin/upstream/check` / 作業単位 B-66）。
    pub(in crate::server) const fn checker(&self) -> &context::UpstreamChecker {
        &self.checker
    }

    /// **起こした上流の、面ごとの状態**の現在値（ADR-0042 §決定3 / 作業単位 F-4b-1）。
    ///
    /// **起動時の写しを持たない。** 載るのを待つ背景タスクがここを書き替えるので、
    /// 写すと**「載せている」と言ったまま永久に固まる**（[`Self::upstream`] と同じ理屈）。
    ///
    /// **読む口は `GET /admin/upstream`**（`upstream::get_upstream`）。
    /// **`--upstream` を渡していない起動では、どちらの面も `external`** のままである。
    pub(crate) fn launch_states(&self) -> launch::LaunchStates {
        self.launch_states.get()
    }

    /// **この起動が上流プロファイルの見本を書いた先**（作業単位 F-4c）。`None` は「書いていない」。
    ///
    /// **読む口は `GET /admin/upstream`**（`upstream::get_upstream`）。
    pub(crate) fn profile_sample(&self) -> Option<&std::path::Path> {
        self.profile_sample.as_deref()
    }

    /// **走ったままの差し替え**（`POST /admin/upstream/profile` / 作業単位 F-4b-2）。
    ///
    /// **`Arc` で返す** —— 差し替えは依頼の接続から切り離したタスクで走る
    /// （`switch::run_detached` / 作業単位 B-56）ので、タスクへ複製を持たせる。
    pub(in crate::server) const fn switcher(&self) -> &Arc<switch::Switcher> {
        &self.switcher
    }

    /// **上流プロファイルの一覧と保存**（`GET` / `PUT /admin/upstream/profiles…` / 作業単位 F-4b-3）。
    pub(in crate::server) fn profile_editor(&self) -> &profile_editor::ProfileEditor {
        &self.profile_editor
    }

    /// **変容の走らせ方の、起動時の既定**（ADR-0026 §決定1 / 作業単位 D-5a / **A-8 で「既定」になった**）。
    ///
    /// `GET /admin/characters/{handle}/drift` が応答に載せる
    /// （`admin::DriftView::default_run_mode` と、キャラクターが未設定のときの `run_mode`）。
    ///
    /// # ⚠ 効く走らせ方ではない（ADR-0045 §決定2）
    ///
    /// **キャラクターが値を持っていれば、そちらが勝つ。** 効く値を決めるのは
    /// `domain::ResolvedDriftMode::resolve` で、変容を走らせる側
    /// （`extract::Extraction::start`）と応答（`admin::drift_view`）の両方が通る。
    ///
    /// # **写しを持たない**
    ///
    /// 返すのは**抽出器が実際に判定に使っている値そのもの**である。
    /// `AppState` に別のフィールドとして持たせると、
    /// **「応答は `on` と言うのに変容は走っていない」**という配線が書ける
    /// ——しかも**変容は本文に出ない**（ADR-0024 §9）ので、
    /// **画面だけを見ている人には永久に分からない。**
    ///
    /// # 起動時に決まる値なので、本番の入口を通る見張り番が要る
    ///
    /// **`bind_with` に新しい行を足していない**（既にある `stat_drift` が
    /// `MemoryExtractor` へ渡り、そこから読むだけ）ので、
    /// 「配線だけの場所」は増えていない。それでも
    /// **既定（`off`）以外で回る腕**を置いてある——既定で回っている限り、
    /// 引いてこようが決め打ちだろうが区別が付かない（CLAUDE.md / `B-7` の実測）。
    /// 権威は `tests/server_admin_drift.rs::
    /// the_response_names_the_run_mode_the_server_was_started_with`
    /// （**3値を1本のテストで回す**ので、既定への決め打ちも綴りの入れ替えも赤になる）。
    pub(crate) fn default_stat_drift(&self) -> StatDriftMode {
        self.extraction.default_stat_drift()
    }
}

/// `LlmClient` は `Debug` を要求しないので手で書く（`derive` できない）。
impl fmt::Debug for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppState")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

/// **起動時の照会で決まったもの一式**（`App::bind_with` の引数を束ねたもの）。
///
/// # なぜ束ねるのか
///
/// **数が増えたから**（作業単位 G-3b で 8 個目になり `clippy::too_many_arguments` に触れた）
/// **だけではない。** 5 つのうち 2 つが `Option<Arc<LmStudioClient>>`、
/// 2 つが「本文用 / サブ用」の対で、**位置で渡す限り取り違えが型では止まらない。**
/// **名前で渡す形にすると、取り違えは綴りの間違いになる。**
///
/// **注入の入口（`App::bind_with_clients`）は「引く先が無い」ものを詰める**
/// ——落とし所・`None`・「一度も聞いていない」。
struct Resolved {
    /// 本文のトークン予算（ADR-0013 第2層）と、**それが明示か**（作業単位 A-13a-1）。
    budget: context::Chosen<TokenBudget>,
    /// 本文の予算の引き直しの照会先。**`None` は照会先を持たない注入の入口だけ。**
    ///
    /// **明示の間も持つ**（作業単位 A-13a-1）——引くかどうかは器の「明示か」が毎回決める
    /// （走ったまま値の無いプロファイルへ切り替えたら引き直す / ADR-0054 §決定3）。
    refresh_source: Option<context::FaceClient>,
    /// **サブモデルの実コンテキスト長**（作業単位 B-7）と、**それが明示か**（ADR-0054 §決定1）。
    sub_context: context::Chosen<extract::SubContext>,
    /// **サブモデルの引き直しの照会先。** 本文用と取り違えないこと
    /// （`context::resolve_sub_context` の doc）。
    sub_refresh_source: Option<context::FaceClient>,
    /// **上流について観測できたこと**（作業単位 G-3b）。
    upstream_facts: upstream::UpstreamFacts,
    /// **起動の直後に照会する面**（作業単位 B-66 / `context::CheckAtStart`）。
    ///
    /// **`bind` の中で照会しなかった面のうち、照会してよい面**（コンテキスト長を書いた外の上流の面）。
    /// **`bind` の中では待たない** —— 背景タスクの最初の1回として `serve` の直後に走る。
    check_at_start: context::CheckAtStart,
}

/// **この起動が自分で起こすもの**（ADR-0042 / 作業単位 F-4b-1 / `launch` モジュール）。
///
/// **`--upstream` を渡していない起動は計画が空**で、子は1つも作られない。
/// **⚠ F-4b-1 までは `Launching::none` を通っていた**が、いまは本番の入口は
/// **差し替え口（`clients`）を必ず持つ**（作業単位 F-4b-2 / 走ったまま gguf へ差し替えるため）。
/// `Launching::none` はモックを注入する入口だけが通る。
///
/// **3つを別々の引数で渡さない**——渡すときに順番を取り違えても型が通る形にしない。
struct Launching {
    /// 起こすもの。**空なら1つも起こさない。**
    plan: launch::LaunchPlan,
    /// **計画の面をいつ起こすか**（ADR-0055 / 作業単位 F-10a / [`LaunchTiming`]）。
    ///
    /// **[`LaunchTiming::WhenAsked`] なら、計画の面は起こさずに `not_started` にする**（`bind_with`）。
    timing: LaunchTiming,
    /// **面ごとの差し替え口**（載ったかを聞く相手でもある）。
    ///
    /// **本番の入口（`App::bind`）では、起こすものが無くても `Some`**（作業単位 F-4b-2）——
    /// 環境変数で起動してから、gguf のプロファイルへ差し替えられるように。
    /// **`None` はモックを注入する入口だけ**（差し替えられない起動）。
    ///
    /// **`App` の `llm` からは作れない**——あちらは `Arc<dyn LlmClient>` で、
    /// `/api/v0/models` はトレイトに載っていない（`llm` モジュールの doc）。
    clients: Option<context::FaceClients>,
    /// **起動時の `PERSONACORE_CONTEXT_TOKENS`**（ADR-0054 §決定3 / 作業単位 A-13a-1）。
    ///
    /// **走ったまま値の無いプロファイルへ切り替えたときに戻る先**（`launch::LoadSinks`）。
    /// **「いま明示か」は器が持つ**——F-4b-2 までここにあった `main_budget_is_explicit: bool` は
    /// 起動時に決めた値で、切り替えに追従しなかった。
    context_environment: Option<TokenBudget>,
    /// **画面を配るか**（`--no-ui` / ADR-0047 §① / 作業単位 P-1）。
    ///
    /// **ここで運ぶのは、`bind_with` が配り先（`web::UiFiles`）を決める場所だから**——
    /// 待ち受けた後で差し替えると、起動ログの `ui_source` が `embedded` と嘘をつく。
    /// **モック注入の入口は常に [`Ui::Serve`]**（今までどおり）。
    ui: Ui,
}

impl Launching {
    /// **1つも起こさず、差し替えもできない。** モック注入の入口だけが通る。
    ///
    /// **⚠ F-4b-1 までは `--upstream` の無い本番の起動もここを通った。**
    /// **いまは本番の入口は常に差し替え口を持つ**（作業単位 F-4b-2 / `bind_launching`）ので、
    /// テスト専用の入口にしか要らない。
    #[cfg(any(test, feature = "testing"))]
    fn none() -> Self {
        Self {
            plan: launch::LaunchPlan::none(),
            timing: LaunchTiming::AtStartup,
            clients: None,
            context_environment: None,
            ui: Ui::Serve,
        }
    }
}

/// **待ち受けが開いたら、ブラウザを開くか**（作業単位 F-4c / `browser` モジュール）。
///
/// **`bool` にしない**——`Startup` を組む式で `true` / `false` が何を言っているか読めなくなる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    /// **既定のブラウザで画面を開く**（引数なしで叩いたとき）。
    Open,
    /// **開かない**（`--no-browser` / テストの入口）。URL は INFO に出る。
    Stay,
}

/// **画面を配るか**（`--no-ui` / [ADR-0047](../../docs/adr/0047-the-api-is-the-product.md) §① / 作業単位 P-1）。
///
/// **`bool` にしない**（[`Browser`] と同じ理由）。
///
/// # [`Ui::Withhold`] のとき
///
/// | もの | どうなるか |
/// |---|---|
/// | `/` と画面の資産 | **本文の無い 404**（`PERSONACORE_WEB_DIR` が立っていても） |
/// | `/v1` / `/admin` | **1バイトも変わらない**（未知パスの判定 `web::is_api_path` が先に取る） |
/// | ブラウザ | **開かない**（[`Browser::Open`] を渡されても。開く頁が無い） |
/// | 起動ログ | 待ち受けの行が `ui_source=none`、**[`NO_UI_LINE`] の1行**が同じ欄を持つ。画面が空だという警告は出さない |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ui {
    /// **画面を配る**（既定。埋め込みか `PERSONACORE_WEB_DIR`）。
    Serve,
    /// **配らない**（`--no-ui`）。
    Withhold,
}

/// **`--no-ui` の起動が言う1行の本文**（作業単位 P-1）。
///
/// **写しが `tests/upstream_cli.rs` の `NO_UI_LINE_COPY` に在る**
/// （文面を変えたら、そちらは「言っていない」で赤になる）。
pub const NO_UI_LINE: &str =
    "画面は配りません（--no-ui）。ブラウザも開きません。/v1 と /admin だけに答えます";

/// **計画の面を、いつ起こすか**（[ADR-0055](../../docs/adr/0055-do-not-start-the-remembered-upstream-at-launch.md)
/// §決定1・2 / 作業単位 F-10a）。
///
/// **`bool` にしない**（[`Browser`] と同じ理由）。
///
/// | 値 | いつ使う | 計画の面 |
/// |---|---|---|
/// | [`Self::AtStartup`] | `--upstream <名前>` を明示した起動（既定） | **待ち受けを開いた直後に起こす**（今までどおり / ADR-0042 §決定3） |
/// | [`Self::WhenAsked`] | **覚えた名前で起きた起動**（`main`） | **起こさない。** `not_started` で待ち、同じ名前の `POST /admin/upstream/profile` で起こす |
///
/// **どちらでも、宛先・モデル・窓はプロファイルのものを読み込む**（ADR-0055 §決定1「起こさないだけ」）。
/// **起こす面には、どちらでも起動時に上流を聞かない**——[`Self::WhenAsked`] の面も、そこにはまだ何も居ない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchTiming {
    /// **待ち受けを開いた直後に起こす**（既定）。
    AtStartup,
    /// **頼まれるまで起こさない**（覚えた名前で起きた起動）。
    WhenAsked,
}

/// **起動時に `main` が決めて、[`App::run_with_upstreams`] へ渡すもの**（作業単位 F-4c）。
///
/// | 欄 | 既定（`Startup::new` / `From<LaunchPlan>`） |
/// |---|---|
/// | 起こす上流（[`LaunchPlan`]） | 渡されたもの |
/// | いつ起こすか（[`LaunchTiming`]） | **[`LaunchTiming::AtStartup`]**（今までどおり） |
/// | 見本を書いた先 | **無し**（書いたと言わない） |
/// | ブラウザ | **[`Browser::Stay`]**（開かない） |
/// | 画面 | **[`Ui::Serve`]**（配る。今までどおり） |
/// | 選んだプロファイルの名前（[`Self::selecting`]） | **無し**（選んだと言わない） |
///
/// **既定を「開かない」にした理由**: **この入口を直に呼ぶのはテスト**（`tests/upstream_launch.rs`）で、
/// **開く側を既定にすると、`Startup` を足し忘れた呼び出しがブラウザを開く形になる。**
/// **開くのは `main` が [`Self::browser`] で明示したときだけ。**
#[derive(Debug)]
pub struct Startup {
    plan: launch::LaunchPlan,
    /// **計画の面をいつ起こすか**（ADR-0055 / 作業単位 F-10a / [`Self::launch_timing`]）。
    timing: LaunchTiming,
    profile_sample: Option<std::path::PathBuf>,
    profiles_file: profile_editor::ProfilesLocation,
    browser: Browser,
    /// **画面を配るか**（`--no-ui` / 作業単位 P-1 / [`Self::ui`]）。
    ui: Ui,
    /// **`--upstream <名前>` で選んだ名前**（ADR-0042 §決定9 / [`Self::remembering`]）。
    remember: Option<String>,
    /// **この起動が選んだプロファイルの名前**（作業単位 F-10b / [`Self::selecting`]）。
    selected: Option<String>,
    /// **`main` が起動の最初に取った、データのディレクトリのロック**（ADR-0057 §決定2 / [`Self::holding`]）。
    instance: Option<instance_lock::InstanceLock>,
}

impl Startup {
    /// **計画だけを持つ起動。** 見本の話はせず、ブラウザも開かず、**差し替えもできない。**
    #[must_use]
    pub const fn new(plan: launch::LaunchPlan) -> Self {
        Self {
            plan,
            timing: LaunchTiming::AtStartup,
            profile_sample: None,
            profiles_file: profile_editor::ProfilesLocation::None,
            browser: Browser::Stay,
            ui: Ui::Serve,
            remember: None,
            selected: None,
            instance: None,
        }
    }

    /// **この起動が持っている、データのディレクトリのロック**（ADR-0057 §決定2 / 作業単位 F-11a）。
    ///
    /// **待ち受けを開いた後に、実際の URL と画面を配るか（[`Self::ui`]）をロックの隣へ書き、
    /// [`App::run_with_upstreams`] が返るまで持ち続ける**——同じデータで起こした2つ目は、それを読んで案内する。
    /// **渡さなければ何も書かない**（テストの入口の既定）。
    #[must_use]
    pub fn holding(mut self, lock: instance_lock::InstanceLock) -> Self {
        self.instance = Some(lock);
        self
    }

    /// **この起動はプロファイル `name` で起きた**（作業単位 F-10b / ADR-0055 §決定3）。
    ///
    /// **`GET /admin/upstream` の `selected_profile` の最初の値**になる（以後は差し替えが書き換える）。
    /// 画面の「起こす」はこの名前を `POST /admin/upstream/profile` へ送る。
    ///
    /// **`main` は `--upstream <名前>` と覚えた名前のどちらで起きたときも渡す。**
    /// [`Self::remembering`]（覚える）とは別のこと —— 覚えた名前で起きた起動は覚え直さないが、選んではいる。
    /// **環境変数の既定で起きた起動では呼ばない**（選んだと言わない）。
    #[must_use]
    pub fn selecting(mut self, name: impl Into<String>) -> Self {
        self.selected = Some(name.into());
        self
    }

    /// **計画の面をいつ起こすか**（ADR-0055 §決定1・2 / 作業単位 F-10a）。
    ///
    /// **`main` は覚えた名前で起きるときだけ [`LaunchTiming::WhenAsked`] を渡す。**
    /// **`--upstream` を明示した起動では渡さない**（既定の [`LaunchTiming::AtStartup`] のまま）。
    #[must_use]
    pub const fn launch_timing(mut self, timing: LaunchTiming) -> Self {
        self.timing = timing;
        self
    }

    /// **この起動は `--upstream <名前>` で選んだ。起こせたら、その名前を覚える**
    /// （ADR-0042 §決定9 / 作業単位 F-4f）。
    ///
    /// **覚えるのは既定の置き場所（[`Self::with_editable_profiles_file`]）のときだけ**——
    /// [`Self::with_profiles_file`]（`PERSONACORE_UPSTREAMS`）では何も書かない。
    /// **覚えるのは起こせた後**（`bind` が通った後）——gguf が無くて止まった名前は覚えない。
    /// **覚えた名前から起こした起動では呼ばない**（同じ名前を書き直すだけ）。
    #[must_use]
    pub fn remembering(mut self, name: impl Into<String>) -> Self {
        self.remember = Some(name.into());
        self
    }

    /// **走ったまま差し替えるときに読む、上流プロファイルの置き場所**
    /// （ADR-0042 §決定1・4 / 作業単位 F-4b-2）。**画面からは読むだけ**（作業単位 F-4b-3）。
    ///
    /// **`POST /admin/upstream/profile` のたびにここを読み直す**——
    /// 利用者が gguf のパスを書き換えた直後でも、選び直せば効く。
    /// **渡さなければ、その起動は差し替えを断る**（409 `upstream_switch_unavailable`）。
    ///
    /// **`main` は `PERSONACORE_UPSTREAMS` が指した先をここで渡す**——
    /// `PUT /admin/upstream/profiles/{name}` は 409 `upstream_profiles_read_only` で断る（ADR-0042 §決定8）。
    #[must_use]
    pub fn with_profiles_file(mut self, path: std::path::PathBuf) -> Self {
        self.profiles_file = profile_editor::ProfilesLocation::ReadOnly(path);
        self
    }

    /// **[`Self::with_profiles_file`] と同じ。そのうえ画面から書ける**（ADR-0042 §決定8 / 作業単位 F-4b-3）。
    ///
    /// **`main` が既定の置き場所（DB と同じディレクトリ）を渡すときだけ使う。**
    /// **`PERSONACORE_UPSTREAMS` が指した先を渡さないこと**——画面の保存が
    /// リポジトリのファイル（コミットされた見本を含む）を書き換える経路になる。
    #[must_use]
    pub fn with_editable_profiles_file(mut self, path: std::path::PathBuf) -> Self {
        self.profiles_file = profile_editor::ProfilesLocation::Editable(path);
        self
    }

    /// **この起動が上流プロファイルの見本を書いた**（`GET /admin/upstream` の `profile_sample`）。
    ///
    /// **書いたときだけ呼ぶこと**——既に在ったファイルの場所を渡すと、
    /// 画面が「見本を置いた」と嘘をつく。
    #[must_use]
    pub fn with_profile_sample(mut self, written_to: std::path::PathBuf) -> Self {
        self.profile_sample = Some(written_to);
        self
    }

    /// **待ち受けが開いたら、ブラウザを開くか。**
    ///
    /// **[`Ui::Withhold`] のときは、ここに何を渡しても開かない**（[`Self::ui`]）。
    #[must_use]
    pub const fn browser(mut self, browser: Browser) -> Self {
        self.browser = browser;
        self
    }

    /// **画面を配るか**（`--no-ui` / ADR-0047 §① / 作業単位 P-1）。
    ///
    /// **[`Ui::Withhold`] は [`Browser::Open`] に勝つ**——開く頁が無い。
    #[must_use]
    pub const fn ui(mut self, ui: Ui) -> Self {
        self.ui = ui;
        self
    }
}

impl From<launch::LaunchPlan> for Startup {
    fn from(plan: launch::LaunchPlan) -> Self {
        Self::new(plan)
    }
}

/// 待ち受けを開始したサーバ。**まだ応答はしていない**（`serve` で走り出す）。
///
/// `bind` と `serve` を分けているのは、テストが**実際のポートを知ってから**
/// リクエストを投げられるようにするため（エフェメラルポートで起動できる）。
pub struct App {
    store: Store,
    llm: Arc<dyn LlmClient>,
    listener: TcpListener,
    local_addr: SocketAddr,
    /// **待ち受けをどう開けたか**——元の番号・その出どころ・ずらしたか・飛ばした番号
    /// （ADR-0057 §決定4・5 / 作業単位 F-11b / `port_search::OpenedPort`）。`serve` が `GET /admin/server` へ渡す。
    opened: port_search::OpenedPort,
    /// **共有の予算と、まだ走り出していない引き直しを束ねたもの**（ADR-0020）。
    ///
    /// **予算と照会先を別々のフィールドで持たない。** 別々に持つと `serve` で
    /// 「引き直しが書く先」と「ハンドラが読む先」を取り違えて配線でき、
    /// **WARN は正しく出るのに予算は古いまま**という壊れ方が書ける。
    ///
    /// **`serve` からその取り違えを書く道は、実測で2つとも塞がっている**
    /// ——`context::SharedBudget::new` は private（`E0624`）、
    /// `context::PendingRefresh::new` を2度呼ぶ形は
    /// `the_running_budget_is_shared_with_the_handlers` が赤にする。
    /// **`context` モジュールの中にはまだ書ける形が1つ残っている**
    /// （`context::PendingRefresh` の doc の表。**測った結果はそこにだけ書く**）。
    ///
    /// 照会先は具象型で持つ。`/api/v0/models` は LM Studio 独自で `LlmClient` に
    /// 載っていない（載せると他の実装が実装できない項目になる / `llm` モジュールの doc）。
    refresh: context::PendingRefresh,
    /// **記憶抽出の仕掛け**（ADR-0021 / `extract` モジュール）。
    ///
    /// `bind` の時点で作るのは、**`Store` を握る必要があるから**（抽出は DB に書く）。
    /// `serve` はこれを `AppState` へ載せ、**停止時に `shutdown()` を呼ぶ**
    /// ——走っている抽出を落とす場所は、止められる `serve` にしかない。
    extraction: Arc<extract::MemoryExtractor>,
    /// **UI の配り先**（ADR-0031 / `web` モジュール）。
    ///
    /// `bind_with` で作る（**本番の入口とモック注入の入口の両方がそこを通る**）ので、
    /// この1行が観測対象から外れる形が構造的に作れない。
    /// `serve` がこれを `AppState` へ載せる。
    ui: web::UiFiles,
    /// **上流の観測の器**（作業単位 G-3b）。
    ///
    /// **`refresh` が持つ器の複製そのもの**（`context::PendingRefresh::upstream`）。
    /// 別のフィールドで持たない——別々に持つと `serve` で
    /// 「引き直しが書く先」と「ハンドラが読む先」を取り違えて配線できる
    /// （`refresh` の doc と同じ理屈）。
    upstream: context::SharedUpstream,
    /// **起こした上流の、面ごとの状態**（ADR-0042 §決定3 / `launch` モジュール）。
    ///
    /// **`--upstream` を渡していない起動では、どちらの面も `External`** のままで、
    /// **応答にその1語が載るだけ**（振る舞いは何も変わらない）。
    launch_states: launch::SharedLaunchStates,
    /// **起こした子の控え。** `serve` が番人を付け、差し替えの持ち主へ渡し、終了時に止める。
    ///
    /// **`bind` の中で起こして、ここに持つ。** 途中で `?` に当たれば
    /// `kill_on_drop` が殺す（`launch` モジュールの doc）。
    /// **起こすものが無くても空の束を持つ**（作業単位 F-4b-2）。
    supervisor: launch::Supervisor,
    /// **面ごとの差し替え口**（載ったかを聞く相手でもある）。
    /// **本番の入口では常に `Some`、モックを注入する入口では `None`。**
    face_clients: Option<context::FaceClients>,
    /// **起動時の `PERSONACORE_CONTEXT_TOKENS`**（切り替えで戻る先 / [`Launching`] の同名の欄）。
    context_environment: Option<TokenBudget>,
    /// **この起動が上流プロファイルの見本を書いた先**（作業単位 F-4c / [`Startup`]）。
    profile_sample: Option<Arc<std::path::Path>>,
    /// **走ったまま差し替えるときに読む置き場所**（作業単位 F-4b-2 / [`Startup::with_profiles_file`]）。
    /// **画面から書けるか**も持つ（作業単位 F-4b-3 / [`Startup::with_editable_profiles_file`]）。
    profiles_file: profile_editor::ProfilesLocation,
    /// **起動時に選んだプロファイルの名前**（作業単位 F-10b / [`Startup::selecting`]）。
    /// `serve` が差し替えの持ち主へ渡す（以後は差し替えが書き換える）。
    selected_profile: Option<String>,
}

/// `LlmClient` は `Debug` を要求しないので手で書く（`derive` できない）。
impl fmt::Debug for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App")
            .field("store", &self.store)
            .field("local_addr", &self.local_addr)
            .finish_non_exhaustive()
    }
}

impl App {
    /// 待ち受けから停止までを一本で実行する。**本番の起動経路はこれを使う。**
    ///
    /// `App` を呼び出し側へ渡さないので、**待ち受けと応答の間に失敗しうる処理を
    /// 差し込めない**（差し込めると、待ち受け済みの `App` を落としたまま
    /// 早期 return する経路ができ、`Store` が閉じられずに漏れる）。
    ///
    /// テストは実際のポートを知る必要があるので、`bind` と `serve` を直接使う。
    pub async fn run<F>(config: &AppConfig, shutdown: F) -> Result<(), ServerError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Self::run_with_upstreams(config, launch::LaunchPlan::none(), shutdown).await
    }

    /// **自分で上流を起こす起動**（ADR-0042 / 作業単位 F-4b-1）。
    ///
    /// `main` が `--upstream <名前>` を受けたときだけ、空でない計画が来る。
    /// **計画が空なら [`Self::run`] と1バイトも変わらない。**
    ///
    /// **`bind` と `serve` を分けて書かないのは [`Self::run`] と同じ理由**
    /// ——分けると、待ち受け済みの `App`（＝起こした子を持つ `App`）を
    /// 落としたまま抜ける経路ができる。
    ///
    /// # 起動時に `main` が決めることは [`Startup`] で渡す（作業単位 F-4c）
    ///
    /// **`LaunchPlan` をそのまま渡してもよい**（`From<LaunchPlan>`）——
    /// **そのときは見本の話をせず、ブラウザも開かない**（テストの入口の既定）。
    ///
    /// # ブラウザを開くのは `bind` の後・`serve` の前
    ///
    /// **`bind` は DB まで開いてから返る**ので、開いた画面はすぐ使える
    /// （待ち受けは開いているので、`serve` が受け始めるまでの接続は待たされるだけ）。
    /// **開くことは失敗しない**（`browser::open` は WARN に落とす）ので、
    /// **`bind` と `serve` の間に `?` を置かない約束**（[`Self::run`] の doc）は破れていない。
    pub async fn run_with_upstreams<F>(
        config: &AppConfig,
        startup: impl Into<Startup>,
        shutdown: F,
    ) -> Result<(), ServerError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let startup = startup.into();
        // **ロックは `serve` が返るまで持つ**（下の `drop`）。`bind` が失敗して `?` で抜けても、ここで手放す。
        let instance = startup.instance;
        let mut app =
            Self::bind_launching(config, startup.plan, startup.timing, startup.ui).await?;

        // **起こせたので、選んだ名前を覚える**（ADR-0042 §決定9）。**書けなくても止めない**
        // （`remember_or_warn` は警告だけ）——`bind` と `serve` の間に `?` を置かない約束を破らない。
        if let (Some(name), Some(memory)) = (
            startup.remember.as_deref(),
            remembered::memory_for(&startup.profiles_file),
        ) {
            remembered::remember_or_warn(&memory, name);
        }

        app.profile_sample = startup.profile_sample.map(Arc::from);
        app.profiles_file = startup.profiles_file;
        app.selected_profile = startup.selected;

        // **待ち受けを開いた後に、実際の URL と画面を配るかを、ロックの隣へ書く**（ADR-0057 §決定2 / 作業単位 F-11a）。
        // **`local_addr` から書く**（設定の番号ではない。`PERSONACORE_PORT=0` でも実際の番号を言う）。
        // **書けなくても止めない**（`announce` は警告だけ）——`bind` と `serve` の間に `?` を置かない約束を破らない。
        // **下のブラウザの行より前に書く**——`tests/instance_lock_cli.rs` は、その行を「書いた後」の印として待つ。
        if let Some(lock) = &instance {
            lock.announce(app.local_addr, startup.ui);
        }

        // **画面を配らないなら、ブラウザの指定を見ない**（作業単位 P-1）——開く頁が無い。
        // **この1行が「画面は配らない」を言う唯一の行**（`Ui` の doc の表）。
        // `ui_source` は待ち受けの行と同じ欄・同じ綴り（集計はこの欄で引く）。
        let url = browser::ui_url(app.local_addr);
        match (startup.ui, startup.browser) {
            (Ui::Withhold, _) => {
                // **`%` で書く**——素で渡すと `ui_source="none"`（引用符つき）になり、
                // 待ち受けの行の `ui_source=none` と綴りが割れて集計で引けない（実測）。
                tracing::info!(ui_source = %web::UI_SOURCE_NONE, "{NO_UI_LINE}");
            }
            (Ui::Serve, Browser::Open) => {
                // 開けなかったことは `browser::open` が WARN で言う（ここでは止めない）。
                let _opened = browser::open(&url);
            }
            (Ui::Serve, Browser::Stay) => tracing::info!(url = %url, "ブラウザは開きません"),
        }

        let served = app.serve(shutdown).await;
        // **`serve` が返る（`Store` を閉じ、起こした子を止めた）まで、ロックを持つ。**
        drop(instance);
        served
    }

    /// 待ち受けを開始し、**最後に** DB へ接続する（マイグレーション適用込み）。
    ///
    /// # `Store::connect` を最後に置く理由
    ///
    /// 先に開くと、以降のすべての失敗に「開いた `Store` を閉じてから返す」分岐が要る。
    /// 分岐を足し忘れた `?` が1つ混じるだけでプールが漏れ、`-wal` / `-shm` と
    /// ファイルロックがプロセス終了まで残る。しかも**漏れた経路にテストが無ければ緑のまま**で、
    /// 守っているのはコメントだけになる。
    ///
    /// **開く順序を最後にすれば、それより前の `?` には閉じる対象が存在しない。**
    /// 「閉じ忘れないように分岐を足す」ではなく「閉じる必要が無い形にする」。
    ///
    /// **`Store::connect` より下に `?` を書かないこと。** 失敗しうる構築
    /// （上流クライアント等）は、すべて `Store::connect` より前に置く。
    ///
    /// # 待ち受けを探すのも、`Store::connect` より前（ADR-0057 §決定3 / 作業単位 F-11b）
    ///
    /// **番号を明示していない起動は、塞がっていれば次の空きへずらす**（`port_search` モジュール）。
    /// 範囲を全部試して開けなければ [`ServerError::PortsTaken`] で止まる——そこでも閉じるものはまだ無い。
    pub async fn bind(config: &AppConfig) -> Result<Self, ServerError> {
        Self::bind_launching(
            config,
            launch::LaunchPlan::none(),
            LaunchTiming::AtStartup,
            Ui::Serve,
        )
        .await
    }

    /// **待ち受けの本体。計画があれば、自分で上流を起こす**（ADR-0042 / 作業単位 F-4b-1）。
    ///
    /// # ⚠ 起こす面には、起動時に上流を聞かない（ADR-0042 §決定3）
    ///
    /// **まだ載っていないのだから、聞けば必ず「答えなし」になる。**
    /// そのまま `measured::report` へ渡すと、
    /// **`llama-server` を上流にしたときの 9 行の `Absent`**（作業単位 G-5 が
    /// 「1つの事実 × 9」と診断したもの）**が、毎回の起動で鳴る。**
    /// **深刻度の判定を壊さないために、聞かない面は `unqueried` のままにする**
    /// ——`measured::check` はそれを `Unanswered` に倒すので、
    /// **`Absent` と混ざらない**（`measured` モジュールの doc の3値）。
    ///
    /// **測り直すのは「準備完了」になってから**（`launch::FaceWatcher` →
    /// `context::adopt_face_probe`）。**その往復の観測をそのまま採る。**
    ///
    /// **[`LaunchTiming::WhenAsked`] の面も聞かない**（ADR-0055 / 作業単位 F-10a）——
    /// 起こすのを待っている面の宛先にも、まだ何も居ない。測るのは、頼まれて起こし、載ってから。
    ///
    /// # ⚠ 起こさない面は1バイトも変わらない
    ///
    /// `plan.launches_face(face)` が偽の面は、**いままでどおり起動時に聞く。**
    /// `split` のように**片面だけ起こす**形があるので、面ごとに判定する。
    async fn bind_launching(
        config: &AppConfig,
        plan: launch::LaunchPlan,
        timing: LaunchTiming,
        ui: Ui,
    ) -> Result<Self, ServerError> {
        // --- ここから: 失敗しても閉じるものが無い区間。新しい構築はこの中に足す。 ---

        // 上流クライアント。**`Store::connect` より前**（docs/06-roadmap.md）。
        // ここでの失敗は起動時の構成エラーなので、通信失敗とは別のバリアントに落とす。
        //
        // **`Arc` で持つ。** 本文生成は `Arc<dyn LlmClient>` として、
        // コンテキスト長の引き直しは具象型として、同じ1つを使う。
        let llm =
            Arc::new(LmStudioClient::new(config.llm().clone()).map_err(ServerError::LlmClient)?);

        // **裏方（記憶抽出）用の上流**（ADR-0021 §10）。**モデルだけを差し替えた複製**で、
        // 接続プールは本文用と共有される（`LmStudioClient::with_model`）。
        //
        // **`llm` をそのまま使わないこと。** 既定どうしは同じ綴りなので
        // 既定のままでは違いが出ないが、使うと **`PERSONACORE_SUB_MODEL` が黙って無視される**
        // （抽出だけ別のモデルへ振り分ける経路が消える）。**この1行の見張り番は
        // `tests/server_memory_extraction.rs` の2本**——
        // `the_sub_model_reaches_the_upstream_through_the_real_bind`（依頼の宛先）と
        // `the_extraction_limit_follows_the_sub_models_context_through_the_real_bind`
        // （**コンテキスト長の照会先** / 作業単位 B-7）。他のサーバテストは
        // `bind_with_clients`（注入の入口）を通るので、ここが潰れても赤くならない。
        //
        // **⚠ 上流が2つになりうる**（ADR-0039 §決定1 / 作業単位 G-3a）。
        // **`PERSONACORE_SUB_LLM_ENDPOINT` が明示されたときだけ、別のクライアントを組む。**
        // **未設定なら `with_model` の経路をそのまま通る**——既定の振る舞いを1バイトも変えない
        // （接続プールの共有もそのまま）。**この分岐の見張り番は
        // `tests/server_sub_llm_endpoint.rs` の2本。**
        //
        // | 見張り番 | 見ているもの |
        // |---|---|
        // | `the_sub_llm_endpoint_sends_only_the_extraction_to_the_second_upstream` | **宛先とモデルの帰属**（本文用と抽出用に別のモックサーバを立てる） |
        // | `the_unset_extraction_client_shares_the_body_clients_connection_pool` | **接続プールの共有**（接続を数える上流へ、本文を2本重ねる） |
        //
        // **⚠ 1周目は「プールの共有は観測できない」と書いていた。偽だった**
        // （G-3a の是正2周目 / 2026-09-07）。
        // **未設定の枝を `LmStudioClient::new(llm.with_model(…).config().clone())` へ
        // 差し替える変異（＝同じ宛先のままプールだけ分ける / `m14`）は、
        // `wiremock` から見える往復を1バイトも変えない**ので1本目は緑のままだが、
        // **開く TCP 接続が 2 本から 3 本に増える**ので2本目が赤になる
        // （**1 赤・exit 101 / 10 回とも。その1本を外すと 1969 全緑に戻る**）。
        // 観測の仕方は `AppConfig::sub_llm` の doc。
        let sub_client = Arc::new(match config.sub_llm() {
            Some(sub) => LmStudioClient::new(sub.clone()).map_err(ServerError::LlmClient)?,
            None => llm.with_model(config.sub_model().clone()),
        });

        // **面ごとの差し替え口**（ADR-0042 §決定4 / 作業単位 F-4b-2 / `context::FaceClient`）。
        //
        // **作るのはここだけ、面につき1つ。** 本文（`AppState::llm`）・抽出（`MemoryExtractor`）・
        // 5分ごとの引き直し・載るのを待つ番人が、**同じ口を共有する**——
        // 走ったまま差し替えたとき、全員が次の往復から新しい上流へ行く。
        // **2つ作ると「本文は新しい上流、引き直しは古い上流」が書ける。**
        let faces = context::FaceClients::new(Arc::clone(&llm), Arc::clone(&sub_client));
        let main_face = faces.face(crate::measured::ModelFace::Main).clone();
        let sub_face = faces.face(crate::measured::ModelFace::Sub).clone();
        let sub: Arc<dyn LlmClient> = Arc::new(sub_face.clone());

        // **実コンテキスト長の照会（ADR-0013 第2層）。`Store::connect` より前。**
        // 失敗しても `Result` にしない——ここは「取れたら得をする」経路で、
        // 取れなければ既定値へ落ちて `warn` を出すだけ。**起動は止めない**
        // （`/api/v0/models` は LM Studio 独自なので、他の上流では必ず失敗する）。
        //
        // **この2行の見張り番（実測 / 11c-4 の仕上げ・2026-08-20）。**
        // 他のサーバテストは `bind_with_client`（照会を行わない入口）を通るので、
        // ここが潰れても赤くならない。**変異の向きごとに捕まえる本数が違う:**
        //
        // * 照会を落とす（`context::choose_budget(explicit, None)` に潰す）→ **3本が赤**。
        //   `the_queried_context_length_survives_the_real_bind` と
        //   `the_running_budget_is_shared_with_the_handlers`（`tests/server_chat.rs`）、
        //   `the_periodic_requery_is_wired_into_the_real_bind`
        //   （`tests/server_context_refresh.rs`。起動時の照会が 1 回であることを見ている）。
        // * 明示設定を落とす（`context::resolve_budget(None, &llm)` に潰す）→ **2本が赤**。
        //   `the_explicit_context_setting_survives_the_real_bind`（`tests/server_chat.rs`）と
        //   `an_explicit_setting_disables_the_requery_on_the_real_bind`
        //   （`tests/server_context_refresh.rs`）。
        //
        // **`the_explicit_context_setting_survives_the_real_bind` は、照会を落とす変異では
        // 原理的に赤にできない。** 明示設定があるとき `resolve_budget` は
        // `choose_budget(explicit, None)` そのものを返すので、その経路では**等価変異**。
        // 11c-3b の「この2行の見張り番は今も上の2本だけである」は**誤りだった**
        // （数も、内訳も、向きの区別も合っていない）。
        //
        // **明示は2つの出どころから決まる**（ADR-0054 §決定1・2 / 作業単位 A-13a-1）——
        // 上流プロファイルの面に書いた値が `PERSONACORE_CONTEXT_TOKENS` に勝ち、ぶつかったら `warn` を1本。
        // **抽出の上限の明示は面の値だけ**（環境変数は抽出を止めない / `context::resolve_sub_context` の doc）。
        // 見張り番は `tests/server_profile_context.rs`（つまみ1つにつき1本 / ADR-0036 §決定3）。
        let profile_context = config.profile_context();
        let explicit = context::explicit_main_budget(config.context_budget(), profile_context.main);
        let sub_explicit = profile_context.sub;

        // **2つの照会を同時に投げる**（ADR-0036 決定2 の第1階層 / 作業単位 B-7）。
        //
        // **直列にすると、黙り込んだ上流での起動待ちが2倍になる**
        // （`llm::lmstudio` の `DEFAULT_CONTEXT_TIMEOUT` は照会1回ぶんの予算）。
        // どちらも失敗してよい経路なので、順序に意味は無い。
        //
        // **サブ側は `explicit` を見ない。** `PERSONACORE_CONTEXT_TOKENS` が優先するのは
        // 本文の予算であって、抽出の上限ではない（`context::resolve_sub_context` の doc）。
        // **サブ側が見るのは `sub_explicit`（上流プロファイルの面に書いた値）だけ**（作業単位 A-13a-1）。
        //
        // **サブ側は引き直しの照会先も一緒に返す。** ここで別々に組み立てると、
        // **引き直しだけを本文モデルへ向ける形**が書け、**それはスイートで観測できない**
        // （変異検査で実測 / 1554 全緑。5分の間隔を待てるテストが無い）。
        //
        // **⚠ 自分で起こす面は、ここでは聞かない**（ADR-0042 §決定3 / この関数の doc）。
        // **その面はまだ載っていない**ので、聞けば必ず「答えなし」になる。
        // **測り直すのは載ってから**（`launch::FaceWatcher`）。
        let launching_main = plan.launches_face(crate::measured::ModelFace::Main);
        let launching_sub = plan.launches_face(crate::measured::ModelFace::Sub);
        let ((budget, main_probe), (sub_context, sub_refresh_source, sub_probe)) = tokio::join!(
            async {
                if launching_main {
                    // **落とし所で始める。** 載ったら `adopt_face_probe` が採り直す。
                    (
                        context::choose_budget(explicit, None),
                        upstream::UpstreamProbe::unanswered(),
                    )
                } else {
                    context::resolve_budget(explicit, &llm).await
                }
            },
            async {
                if launching_sub {
                    // **引き直しの照会先は、聞かない回でも渡す**
                    // ——5分後には載っている（`context::resolve_sub_context` の doc）。
                    (
                        context::choose_sub_context(sub_explicit, None),
                        Some(sub_face.clone()),
                        upstream::UpstreamProbe::unanswered(),
                    )
                } else {
                    context::resolve_sub_context(&sub_face, sub_explicit).await
                }
            },
        );

        // **実測由来の定数が名乗るモデルと、いま上流が応答するモデルを突き合わせる**
        // （ADR-0033 §決定1 / 作業単位 E-2）。**報告するだけで、起動は止めない**
        // （ADR-0035 と同じ形 —— 見張りは報告する。決めるのは人）。
        //
        // **一覧は面ごとに、その面の上流の照会から来る**（ADR-0039 §決定1 / 作業単位 G-3a）。
        // **どちらの照会も、コンテキスト長を読むのと同じ1回の往復**なので、
        // **往復は1本も増えていない**（`LmStudioClient::models` ＋ `ModelsReport::into_parts`）。
        //
        // **依頼する綴りはクライアントから取る。** 設定を読み直すと、
        // 「設定はこう書いてあるが、クライアントは別のモデルを向いている」形が観測できない。
        //
        // **この1行の見張り番は `tests/server_measured_model.rs`**
        // （**メインとサブに別の綴り**を与えて、本番の入口で見る）。
        //
        // **⚠ 「本文の側には一度も聞いていない」は偽だった**（G-3a のレビューが実測 / 2026-09-07）。
        // 是正前のこの行は本文の面へ `None` を渡し、コメントがその理由を
        // 「照会するのは抽出側だけ」と書いていた。**`context::resolve_budget` は
        // `loaded_context_length()` → `models()` を通るので、現に照会していた**
        // （カウンタを植えて実測: 上流が2つの起動でも `main /api/v0/models hits = 1`）。
        // **一覧を捨てていただけ。** その結果、**本文の面の 4 件が `Unanswered` に落ち**、
        // 落ちる先に **`MEASURED_CONTROL_TOKEN_SPELLINGS`**（ADR-0033 §決定3 が
        // 「最優先」と名指しした行）が含まれていた。
        //
        // **⚠ 「一度も聞いていない」が真になる条件は1つだけ**——
        // **`PERSONACORE_CONTEXT_TOKENS` を明示した起動**（`resolve_budget` が早期 return する）。
        // **既定（未設定）では偽。**
        //
        // **⚠ ただし、下の分岐が見ているのは「一度も聞いていない」ではない**
        // （G-3a の是正2周目 / 2026-09-07。**doc がコードと違うことを言っていた**）。
        // **見ているのは `main_catalogue` が `None` であること**——すなわち
        // **本文の面が一覧を持っていない**という結果だけで、
        // **それは「明示設定で照会そのものを行わなかった」ときと
        // 「往復そのものが失敗した」ときの2つで真になる**
        // （表は `context::resolve_budget` の doc が権威）。
        //
        // **2つを区別しないのが正しい。** 宛先が同じなら、
        // **本文の照会が落ちて抽出の照会が通った回**に抽出側の一覧を使うのは妥当である
        // ——**同じ上流が並べた、同じ一覧**だから。
        //
        // **⚠ そして、区別する形との差はスイートのどこからも観測されていない。**
        // **実測（`m17`: 下の腕を `None if one_upstream && explicit.is_some() =>` に絞る）は
        // 1969 全緑・exit 0。** **これは決めたことであって、測っているものではない。**
        let one_upstream = llm.config().endpoint() == sub_client.config().endpoint();

        // **本文の面が突き合わせる一覧。**
        //
        // | 本文の面の一覧 | 宛先 | 使う一覧 |
        // |---|---|---|
        // | **持っている**（照会が通った） | どちらでも | **本文の上流のもの** |
        // | **持っていない**（明示設定／往復が失敗） | **1つ** | **抽出側の照会の答え**（同じ上流なので同じ一覧） |
        // | **持っていない**（同上） | 2つ | **無し**（`Unanswered`。別の上流の一覧で探さない） |
        //
        // **流用してよいのは宛先が同じときだけ。** 別の上流の一覧で本文の綴りを探すと、
        // **本文の上流には在るものが `Drift::Absent` として鳴る**（＝**嘘の警告**）。
        // **ADR-0039 §決定3（警告は鳴らせたままにする）の前提は「鳴っているのが本当のこと」**である。
        //
        // **判定は設定ではなくクライアントの宛先で行う**——
        // 設定を読み直すと、「設定はこう書いてあるが、クライアントは別の上流を向いている」
        // 形が観測できない（すぐ上の「依頼する綴りはクライアントから取る」と同じ理由）。
        //
        // **⚠ 「同じ URL」は「正規化後にバイト一致する綴り」の意味である。**
        // `LmStudioConfig::endpoint()` を比べているだけで、`normalize_base` は
        // **末尾の `/` を落とすほかは何もしない**——**`http://LOCALHOST:1234` と
        // `http://localhost:1234` は別扱いになり、本文の面は黙る**（`Unanswered` に落ちる）。
        // **ホスト名の畳み込みも名前解決もしていない。**
        //
        // **見張り番は `tests/server_measured_model.rs` の2本**——
        // `two_upstreams_are_each_judged_against_their_own_catalogue`（既定の経路）と
        // `one_upstream_named_twice_still_judges_the_body_face`（**この `one_upstream` を
        // 何から作るか**。明示設定つきの2腕。**`config.sub_llm().is_none()` から作る変異は
        // 是正前は 1966 全緑だった**）。
        let main_catalogue = match main_probe.catalogue.clone() {
            Some(read) => Some(read),
            None if one_upstream => sub_probe.catalogue.clone(),
            None => None,
        };

        // **突き合わせは1回だけ。** 返る内訳をそのまま観測へ入れる
        // （作業単位 G-3b）——**判定を作る場所を2つにしない**（CLAUDE.md）。
        let (main_tally, sub_tally) = measured::report(
            llm.config().model().as_str(),
            sub_client.config().model().as_str(),
            main_catalogue.as_deref(),
            sub_probe.catalogue.as_deref(),
        );

        // **上流について観測できたことを、捨てずに持つ**（作業単位 G-3b）。
        //
        // **宛先も綴りもクライアントから取る**（設定を読み直さない。すぐ上と同じ理由）。
        // **面ごとに、その面の上流のもの**——取り違えると
        // 「抽出が死んでいるのに本文の数を見て正常と読む」形になる。
        //
        // **上流が名乗った綴りは、ファイル名だけが載る**（作業単位 B-45）
        // ——gguf の絶対パスのディレクトリ部分は `context::probe_upstream` が落としている。
        // 理由と見張り番は `upstream` モジュールの doc。
        let upstream_facts = upstream::UpstreamFacts::new(
            upstream::UpstreamObservation::new(
                llm.config().endpoint(),
                llm.config().model().as_str(),
                &main_probe,
                main_tally,
            ),
            upstream::UpstreamObservation::new(
                sub_client.config().endpoint(),
                sub_client.config().model().as_str(),
                &sub_probe,
                sub_tally,
            ),
        );

        // **定期的な引き直しの照会先**（ADR-0020）。**明示の間も持つ**（作業単位 A-13a-1）——
        // 明示が優先される以上、引き直しても予算は変わらないので引かない（§4）が、
        // **それを決めるのは器の「明示か」を毎回読む引き直し**（`context::refresh_once`）。
        // **起動時に照会先を捨てると、走ったまま値の無いプロファイルへ切り替えたときに戻れない**
        // （ADR-0054 §決定3）。実際に走らせるのは `serve`（**タスクを止められる場所**）。
        //
        // **この行の見張り番は `tests/server_context_refresh.rs` の2本だけ。**
        // 他のサーバテストは `bind_with_client`（引き直しを持たない入口）を通る。
        let refresh_source = Some(main_face.clone());

        // **値と「明示か」を一緒に器へ渡す**（`context::Chosen` / 作業単位 A-13a-1）。
        let budget = match explicit {
            Some(_) => context::Chosen::explicit(budget),
            None => context::Chosen::measured(budget),
        };
        let sub_context = match sub_explicit {
            Some(_) => context::Chosen::explicit(sub_context),
            None => context::Chosen::measured(sub_context),
        };

        // **コンテキスト長を書いた面も、生きているか・鍵が通るかを照会する**（作業単位 B-66 / Issue origin#94）。
        //
        // **上の `join!` では照会しない**（`resolve_budget` / `resolve_sub_context` の明示の枝はそのまま）
        // ——**ここで待つと、上流が黙っているときに起動が照会の上限（5秒）ぶん遅くなる。**
        // 背景タスクの最初の1回として `serve` の直後に走らせる（`context::CheckAtStart`）。
        //
        // **自分で起こす面は入れない** —— 起動時に照会しないのと同じ理由（まだ載っていない /
        // 載るのを待つ番人が聞く）。書いていない面は上の `join!` で照会済み。
        //
        // **見張り番は `tests/server_upstream_check.rs`**（`bind` が返った時点で、書いた面の上流が
        // 1度も聞かれていないこと・`serve` の後に聞かれること）。
        let check_at_start = context::CheckAtStart {
            main: explicit.is_some() && !launching_main,
            sub: sub_explicit.is_some() && !launching_sub,
        };

        // --- ここまで ---

        // **載ったかを聞く相手は、面ごとのクライアントそのもの**（ADR-0042 §決定3）。
        // **設定から作り直さない**——作り直すと「設定はこう書いてあるが、
        // クライアントは別の上流を向いている」形が観測できない（この関数の他の箇所と同じ理由）。
        //
        // **起こすものが無くても渡す**（作業単位 F-4b-2）——環境変数で起動してから、
        // gguf のプロファイルへ走ったまま差し替えられるように。
        let launching = Launching {
            clients: Some(faces),
            context_environment: config.context_budget(),
            plan,
            timing,
            ui,
        };

        // 待ち受け開始と DB 接続は共通の実体へ委譲する。
        // **本文も差し替え口越しに渡す**（上の `main_face` の doc）。
        Self::bind_with(
            config,
            Arc::new(main_face),
            sub,
            Resolved {
                budget,
                refresh_source,
                sub_context,
                sub_refresh_source,
                upstream_facts,
                check_at_start,
            },
            launching,
        )
        .await
    }

    /// 待ち受けの実体。**`bind` とテスト専用の注入版が両方ここへ委譲する。**
    ///
    /// # なぜ private の実体を挟むか
    ///
    /// モックを注入する入口はテストにしか要らないが、**本番コードパスが
    /// `#[cfg(feature = "testing")]` の API を呼ぶ形にしてはならない**（CLAUDE.md 2節）。
    /// そうすると素の `cargo build` が宣言と実装の乖離を検出できなくなる。
    /// この関数にはフィーチャを付けず、`bind`（本番）と `bind_with_client`（テスト）が
    /// どちらもここを通る形にしてある。
    async fn bind_with(
        config: &AppConfig,
        llm: Arc<dyn LlmClient>,
        sub: Arc<dyn LlmClient>,
        resolved: Resolved,
        launching: Launching,
    ) -> Result<Self, ServerError> {
        let Resolved {
            budget,
            refresh_source,
            sub_context,
            sub_refresh_source,
            upstream_facts,
            check_at_start,
        } = resolved;

        // --- ここから: 失敗しても閉じるものが無い区間。新しい構築はこの中に足す。 ---

        // **待ち受けを開く。番号を明示していなければ、塞がっているとき次の空きへずらす**
        // （ADR-0057 §決定1・3 / 作業単位 F-11b / `port_search` モジュール）。
        //
        // **ここで完結させる**——`Store::connect` より前なので、範囲を全部試して開けなくても閉じるものが無い
        // （`App::bind` の doc の約束）。**試す番号は `BindAddr::loopback` で作られる**（`open_first` が渡す）。
        let (listener, opened) = port_search::open_first(&config.port_search(), |addr| {
            TcpListener::bind(addr.socket_addr())
        })
        .await
        .map_err(|failure| match failure {
            port_search::SearchFailure::Stopped(err) => ServerError::Bind(err),
            port_search::SearchFailure::Exhausted(taken) => ServerError::PortsTaken(taken),
        })?;
        let local_addr = listener.local_addr().map_err(ServerError::Bind)?;

        // **ずらしたら、警告を1本**（ADR-0057 §決定4）——元の番号・実際の番号・出どころ・飛ばした番号。
        // **`設定を読み込みました` の `bind=` は試し始めた番号のまま**（`main`）。実際の番号はこの行と、
        // 下の待ち受けの行の `addr=` が言う。見張り番は `tests/listening_port_cli.rs` の
        // `a_taken_saved_port_shifts_past_a_face_port_and_the_log_and_the_api_say_so`。
        if opened.shifted() {
            tracing::warn!(
                requested_port = opened.requested(),
                port = local_addr.port(),
                port_source = %opened.source().label(),
                skipped_ports = ?opened.skipped(),
                "{}",
                port_search::shifted_message(opened.requested(), local_addr.port())
            );
        }

        // **⚠ 上流を起こすのは、待ち受けを開いた「後」**（ADR-0042 §決定3）。
        //
        // **逆にすると、`bind` に失敗した起動が上流を残す** ——
        // 起こした後で `TcpListener::bind` が失敗すると（ポートが埋まっている等）、
        // **その子を止める人がもう居ない。**
        // **見張り番は `tests/upstream_launch.rs` の
        // `a_failed_bind_leaves_no_child_behind`**（偽の `llama` が一度も呼ばれないこと）。
        //
        // **`?` で抜けても子は死ぬ**（`kill_on_drop(true)` / `launch` モジュールの doc）——
        // この下の `Store::connect` が失敗する経路がそれである。
        let launch_states = launch::SharedLaunchStates::external();
        let supervisor = if launching.plan.is_empty() {
            // **起こすものが無くても、空の束を持つ**（作業単位 F-4b-2 / 走ったままの差し替え）。
            // **ファイルには触らない**（`upstream-logs/` は起こすときに作る）。
            launch::Supervisor::idle(
                upstream_log_dir(config.database_path()),
                launch_states.clone(),
            )
        } else if launching.timing == LaunchTiming::WhenAsked {
            // **覚えた名前で起きた起動は、gguf の面を起こさない**（ADR-0055 §決定1 / 作業単位 F-10a）。
            //
            // **空の束を持ち、計画の面を `not_started` にするだけ。** 子も `upstream-logs/` も作らない。
            // **起こすのは同じ名前の `POST /admin/upstream/profile`**（`switch`）——束に走っている子が
            // 無いので、差し替えの計画（`switch::plan_switch`）がその面を「新しく起こす」に倒す。
            //
            // **gguf の実在と待ち受けとの取り合いは、ここでは見ない**——`main` が覚えた名前を採る前に
            // 同じ関数（`LaunchPlan::check_before_starting`）で見ており、起こすときは差し替えがもう一度見る。
            for face in crate::measured::ModelFace::ALL {
                if launching.plan.launches_face(face) {
                    launch_states.set(face, launch::FaceState::NotStarted);
                }
            }
            launch::Supervisor::idle(
                upstream_log_dir(config.database_path()),
                launch_states.clone(),
            )
        } else {
            // **⚠ 起こす前に、待ち受けとの取り合いを止める**（作業単位 B-52 / Issue origin#21）。
            //
            // **起こしてしまうと、`llama` は同じポートを取れずに 2 秒で終わる。**
            // そのとき外から言えるのは**「失敗（exit code: 1）」だけ**で、
            // **本当の理由は `llama` の err ログの中にしか無い**（利用者の実機 / 2026-09-19）。
            // **この実行ファイルは両方の値を知っている**のだから、起こす前に両方を挙げて言える。
            //
            // **判定は純粋関数**（`launch::LaunchPlan::port_conflict`）で、
            // **渡すのは実際に開いた番号**（`local_addr` / ADR-0057 §決定3 / 作業単位 F-11b）——
            // `DEFAULT_PORT` も設定の番号（`config.bind()`）も見に行かない。ずらした起動では設定の番号と
            // 違い、`PERSONACORE_PORT=0` では OS が選んだ番号になる。見張り番は `tests/upstream_launch.rs` の
            // `the_conflict_check_sees_the_port_actually_opened_after_shifting`。
            if let Some(collision) = launching
                .plan
                .port_conflict(BindAddr::loopback(local_addr.port()))
            {
                return Err(ServerError::Launch(launch::LaunchError::PortCollision(
                    collision,
                )));
            }
            launch::Supervisor::start(
                &launching.plan,
                &upstream_log_dir(config.database_path()),
                &launch_states,
            )
            .await
            .map_err(ServerError::Launch)?
        };

        // --- ここまで ---

        // **DB の置き場所を作る**（ADR-0041 §決定3 / 作業単位 F-4a）。
        //
        // **既定は OS の作法の場所**で、初回はそのディレクトリがまだ無い。
        // **`Store::connect` の直前でやる**——ここも `bind`（本番）と
        // `bind_with_clients`（モック注入）の両方が通る1箇所である。
        //
        // **`?` ではなく `Store::connect` に任せる形にしない。**
        // sqlx は「親ディレクトリが無い」を `Connect` の中に畳むので、
        // **「作れなかった」と「開けなかった」が同じ理由になる。**
        if let Some(parent) = config.database_path().parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(ServerError::DatabaseDir)?;
        }

        // **`bind` の中で失敗しうる最後の処理。この下に `?` を書かない。**
        let store = Store::connect(config.database_path())
            .await
            .map_err(ServerError::Store)?;

        // **性格ステータスの変容のスイッチ**（ADR-0024 §8 / `PERSONACORE_STAT_DRIFT`）。
        //
        // **`bind` ではなくここで読む。** `bind`（本番）と `bind_with_clients`
        // （モック注入）の**両方がここを通る**ので、この1行が観測対象から外れる形が
        // 構造的に作れない（CLAUDE.md「テスト専用の入口を作ると、本番の入口が
        // 観測対象から外れる」——`context::resolve_budget` はまさにそうなった）。
        //
        // **それでも本番の入口を通る見張り番を置いてある**——
        // `tests/server_stat_drift.rs::
        // the_stat_drift_switch_reaches_the_upstream_through_the_real_bind`
        // （**期待値は on。既定の off ではない**）と、
        // 同ファイルの `the_shadow_mode_records_a_vote_through_the_real_bind`
        // （**3値になったので、記録専用も本番の入口で観測する** / ADR-0026 §決定1）。
        let stat_drift = config.stat_drift();

        // **変容の依頼の形**（ADR-0027 §12 / `PERSONACORE_DRIFT_FORM`）。
        //
        // **`stat_drift` とまったく同じ場所で読む**（同じ理由——本番の入口と
        // モックの入口の両方がここを通る）。**本番の入口を通る見張り番も置いてある**——
        // `tests/server_stat_drift.rs::
        // the_drift_form_reaches_the_upstream_through_the_real_bind`
        // （**期待値は `single`。既定の `all` ではない** ——作業単位 19 で既定が入れ替わった）。
        let drift_form = config.drift_form();

        // **変容の依頼に足す注記**（ADR-0029 §訂正 / `PERSONACORE_DRIFT_NOTE`）。
        //
        // **`drift_form` とまったく同じ場所で読む**（同じ理由——本番の入口と
        // モックの入口の両方がここを通る）。**本番の入口を通る見張り番も置いてある**——
        // `tests/server_stat_drift.rs::
        // the_drift_note_reaches_the_upstream_through_the_real_bind`
        // （**3本の腕を同じ台本で回して、既定と既定以外の対比を1本に置いてある**）。
        let drift_note = config.drift_note();

        // **変容の集約のつまみ**（ADR-0027 §決定8 / §9 / `PERSONACORE_DRIFT_*`）。
        //
        // **`stat_drift` とまったく同じ場所で読む**（同じ理由——本番の入口と
        // モックの入口の両方がここを通る）。**本番の入口を通る見張り番も置いてある**——
        // `tests/server_stat_drift.rs::
        // the_drift_tuning_reaches_the_ledger_through_the_real_bind`
        // （**期待値は既定ではない値**）。
        let drift_tuning = config.drift_tuning();

        // **軸の候補を尋ねる間隔**（ADR-0046 §決定2 / `PERSONACORE_AXIS_DISCOVERY_MEMORIES` /
        // 作業単位 A-9b）。
        //
        // **`drift_tuning` とまったく同じ場所で読む**（同じ理由——本番の入口と
        // モックの入口の両方がここを通る）。**本番の入口を通る見張り番も置いてある**——
        // `tests/server_axis_discovery.rs::
        // the_axis_discovery_interval_reaches_the_background_through_the_real_bind`
        // （**期待値は既定ではない値**。既定の腕と並べてある）。
        let axis_discovery_memories = config.axis_discovery_memories();

        // **軸の候補の試用の判定のつまみ**（ADR-0046 §決定4 / `PERSONACORE_AXIS_TRIAL_*` /
        // 作業単位 A-9c）。
        //
        // **`axis_discovery_memories` とまったく同じ場所で読む**（同じ理由——本番の入口と
        // モックの入口の両方がここを通る）。**本番の入口を通る見張り番も置いてある**——
        // `tests/server_axis_trial.rs::
        // the_axis_trial_rule_reaches_the_background_through_the_real_bind`
        // （**3つとも既定ではない値の腕**と、既定の腕を並べてある）。
        let axis_trial = config.axis_trial();

        // **UI をどこから配るか**（ADR-0031 §3 / ADR-0041 §決定2 / `PERSONACORE_WEB_DIR`）。
        //
        // **`stat_drift` とまったく同じ場所で読む**（同じ理由——本番の入口と
        // モックの入口の両方がここを通る）。**本番の入口を通る見張り番も置いてある**——
        // `tests/server_web_ui.rs`（**配り先はどれも既定ではない**。
        // **未設定＝埋め込みの腕も同じファイルが持つ**）。
        //
        // **`--no-ui` なら、設定を見ずに「配らない」**（ADR-0047 §① / 作業単位 P-1）。
        // **見張り番は本物の実行ファイルを通る**——
        // `tests/upstream_cli.rs::the_no_ui_flag_withholds_the_screen_and_leaves_the_api_byte_for_byte`
        // （`PERSONACORE_WEB_DIR` を立てたまま旗の有無で比べる）。
        let ui = match launching.ui {
            Ui::Serve => web::UiFiles::new(config.web_dir()),
            Ui::Withhold => web::UiFiles::Withheld,
        };

        tracing::info!(
            addr = %local_addr,
            // **上流は2つありうる**（ADR-0039 §決定4 / 作業単位 G-3a）。
            // **どちらの話かを、常に名乗る。**
            //
            // **本文だけ生きていて抽出が死んでいる状態は、いまも起きる**（記憶が育たない）。
            // **新しいのは「別々に落ちる」こと**で、そのとき
            // **どこへ繋ぎに行っていたのかがログのどこにも無いと、切り分けようがない。**
            //
            // **未設定なら2つは同じ綴りになる**——「未設定」とは書かない。
            // **運用者が知りたいのは「抽出はどこへ行くのか」**であって、設定の有無ではない
            // （`AppConfig::sub_llm_endpoint` の doc）。
            //
            // **ユーザー情報・クエリ・フラグメントは落として出す**（作業単位 B-71 / Issue origin#100 /
            // `llm::endpoint_for_log`）。見張り番は `tests/upstream_cli.rs` の
            // `the_secrets_written_in_an_endpoint_never_reach_the_log_or_the_upstream_report`。
            llm_endpoint = %crate::llm::endpoint_for_log(config.llm().endpoint()),
            sub_llm_endpoint = %crate::llm::endpoint_for_log(config.sub_llm_endpoint()),
            context_tokens = budget.value().context_tokens(),
            // **3値になった**（ADR-0026 §決定1）ので綴りで出す。
            // `true` / `false` のままだと、記録専用モードが on と区別できない。
            stat_drift = %stat_drift.label(),
            // **どちらの形で測ったか**が後からログだけで分かること（ADR-0027 §12）。
            drift_form = %drift_form.label(),
            // **どの腕で測ったか**が後からログだけで分かること（ADR-0029 §訂正）。
            // **3本の腕は依頼が1行違うだけ**なので、ログに出さないと
            // **生ログを読み比べるまで区別が付かない。**
            drift_note = %drift_note.label(),
            // **どのつまみで釣り合わせたか**も同じ理由で残す（ADR-0027 §決定8 の ⚠
            // 「これは決めたではなく出発点である」）。既定のまま起動した回と、
            // 振って起動した回が、**ログだけで区別できないと測定を並べられない。**
            drift_decay = drift_tuning.decay(),
            drift_slope = drift_tuning.slope(),
            drift_step_cost = drift_tuning.step_cost(),
            // **軸の候補を尋ねる間隔**（ADR-0046 §決定2 / 作業単位 A-9b）。測定の器具なので、
            // **どの間隔で回した測定か**をログだけで区別できるように残す（つまみと同じ理由）。
            axis_discovery_memories = axis_discovery_memories.get(),
            // **軸の候補の試用の判定のつまみ**（ADR-0046 §決定4 / 作業単位 A-9c）。
            // 測定の器具なので、**どの `T` / `K` / 線で回した測定か**をログだけで区別できるように残す。
            axis_trial_turns = axis_trial.turns(),
            axis_trial_min_nonzero = axis_trial.min_nonzero(),
            axis_trial_line_percent = axis_trial.line_percent(),
            // **画面がどこから来たか・その出どころ・何件配れるか**
            // （ADR-0031 §結果 / ADR-0041 §決定2 / 作業単位 D-0 / F-4a）。
            //
            // **`web/dist` はビルド成果物で `.gitignore` に入る。**
            // ビルドし忘れれば画面は出ないし、古い成果物が残っていれば古いまま出る。
            // **埋め込みでも同じ**——`pnpm build` より後に `cargo build` しないと、
            // **実行ファイルの中の画面が古いまま**になる（ADR-0041 §結果）。
            // **「Finished in 0.4s は直っていない」と同じ罠を FE 側に作らない**ため、
            // 起動のたびに**どちらの経路か**と**そこに何件あるか**を言わせる。
            ui_source = %ui.source(),
            ui_origin = %ui.origin().display(),
            ui_files = ui.file_count(),
            "待ち受けを開始しました"
        );

        // **1件も配れないときは黙らない。** 上の行にも出ているが、
        // **十いくつの欄の中の `0` は読み飛ばされる。**
        // これは「起動に失敗した」ではないので `warn`（起動は続ける / `ENV_WEB_DIR` の doc）。
        //
        // **文面は出どころで違う**——直す手が違う（`pnpm build` か、
        // `pnpm build` のあとの `cargo build` か）。
        //
        // **配らない形（`--no-ui`）は文面を持たない**ので黙る——配らないと決めた起動に
        // `pnpm build` を探させない（作業単位 P-1）。
        if ui.file_count() == 0
            && let Some(message) = ui.missing_message()
        {
            tracing::warn!(ui_origin = %ui.origin().display(), "{message}");
        }

        // **ここで共有の器を作る。** 稼働中に引き直しが差し替えるので、
        // ハンドラも抽出器も起動時の値を写し取る形にしない（ADR-0020 / 作業単位 B-7）。
        //
        // **器と照会先をここで束ねる。** `serve` は束を `spawn` するだけで、
        // 書き先と読み先を選ぶ余地が無い（`context::PendingRefresh` の doc）。
        let refresh = context::PendingRefresh::new(
            budget,
            refresh_source,
            sub_context,
            sub_refresh_source,
            upstream_facts,
            check_at_start,
        );

        // **抽出器が読む先は、引き直しが書く先そのもの**（`Arc` の複製）。
        // **`SharedSubContext` を新しく作る道はここには無い**（`context` の private）。
        let sub_context = refresh.sub_context();

        // **ハンドラが読む先も、引き直しが書く先そのもの**（作業単位 G-3b）。
        // **`SharedUpstream` を新しく作る道はここには無い**（`context` の private）。
        let upstream = refresh.upstream();

        Ok(Self {
            // **抽出器はここで作る。** `Store` を握るので、開いた後でしか作れない。
            // **`llm` ではなく `sub` を渡す**（ADR-0021 §10）。
            // **約束・決まりの提案も同じ枠で走らせる**（作業単位 A-12c-1）。本番の入口とモック注入の入口の
            // 両方がここを通る（見張り番は `tests/server_agreement_proposals.rs`）。
            extraction: Arc::new(
                extract::MemoryExtractor::new(
                    store.clone(),
                    sub,
                    sub_context,
                    stat_drift,
                    drift_form,
                    drift_note,
                    drift_tuning,
                    axis_discovery_memories,
                    axis_trial,
                )
                .with_agreement_proposals(),
            ),
            store,
            llm,
            listener,
            local_addr,
            // **元の番号と出どころは探した結果から**（実際の番号は `local_addr` / ADR-0057 §決定4・5）。
            opened,
            ui,
            refresh,
            upstream,
            launch_states,
            supervisor,
            face_clients: launching.clients,
            context_environment: launching.context_environment,
            // **見本の話は `main` だけが持つ**（`run_with_upstreams` が載せる）。
            // **テストの入口（`bind` / `bind_with_client*`）は、書いたと言わない。**
            profile_sample: None,
            // **置き場所も `main` だけが持つ**（`run_with_upstreams` が載せる）。
            profiles_file: profile_editor::ProfilesLocation::None,
            // **選んだ名前も `main` だけが持つ**（`run_with_upstreams` が載せる / 作業単位 F-10b）。
            selected_profile: None,
        })
    }

    /// 上流クライアントを差し替えて待ち受ける。**テスト専用。**
    ///
    /// テストは**実際の LM Studio に繋がない**（docs/05-development.md）ので、
    /// `LlmClient` のモックをここから注入する。`Store::pool` と同じくフィーチャで
    /// ゲートするのは、本番から「上流を差し替える手段」が見えないようにするため。
    ///
    /// **コンテキスト長の照会は行わない。** 注入されるのは `LlmClient` の実装であり、
    /// 照会先の `/api/v0/models` は LM Studio 独自でトレイトに載っていないため。
    /// 予算は「明示設定 → 既定値」（照会の段だけが抜けた形）で決まる。
    ///
    /// **定期的な引き直し（ADR-0020）も走らない。** 同じ理由——引く先が無い。
    /// したがって**この入口を通るテストは引き直しの配線を一切見ていない。**
    /// 見張るのは `tests/server_context_refresh.rs`（本番の入口を通る）。
    /// **抽出の上流は「何も返さないもの」に据える**（`extract::SilentUpstream`）。
    ///
    /// 抽出のタスクは同じように spawn され、同じ経路を通り、0 件で終わる。
    /// **注入されたモックへは行かない**——行かせると本文の呼び出しと混ざり、
    /// 「最後に上流へ渡した依頼」を見ている既存の見張り番が**競走で赤くなる**
    /// （実測 14 本 / `extract::SilentUpstream` の doc）。
    /// 抽出そのものを見るテストは [`App::bind_with_clients`] を使うこと。
    #[cfg(any(test, feature = "testing"))]
    pub async fn bind_with_client(
        config: &AppConfig,
        llm: Arc<dyn LlmClient>,
    ) -> Result<Self, ServerError> {
        Self::bind_with_clients(config, llm, Arc::new(extract::SilentUpstream)).await
    }

    /// **本文用と抽出用に別の上流を注入して待ち受ける。テスト専用。**
    ///
    /// 抽出（ADR-0021）が走ったことを、**本文の呼び出しと混ざらない形**で
    /// 観測するための入口。1つのモックに両方を受けさせると、
    /// 「抽出が走っていないのに本文の呼び出しで主張が満たされる」形になる。
    ///
    /// **モデル識別子の配線（§10）はここでは見られない。**
    /// `LlmClient` は綴りを持たない（`ChatRequest` に項目が無い / ADR-0004）ので、
    /// **本番の入口 `App::bind` を通して上流の HTTP 本文を見るしかない**
    /// （`tests/server_memory_extraction.rs` の
    /// `the_sub_model_reaches_the_upstream_through_the_real_bind`）。
    #[cfg(any(test, feature = "testing"))]
    pub async fn bind_with_clients(
        config: &AppConfig,
        llm: Arc<dyn LlmClient>,
        sub: Arc<dyn LlmClient>,
    ) -> Result<Self, ServerError> {
        // **明示の決め方は本番の入口と同じ関数**（作業単位 A-13a-1）。引く先が無いので、
        // 明示が無ければ既定値・落とし所になる。
        //
        // **器の「明示か」も本番の入口と同じく事実どおりに付ける**（レビュー是正）——
        // この入口では引き直しが走らないので今は効かないが、印が事実と食い違う器を作らない。
        let profile_context = config.profile_context();
        let explicit = context::explicit_main_budget(config.context_budget(), profile_context.main);
        let budget = context::choose_budget(explicit, None);
        let budget = match explicit {
            Some(_) => context::Chosen::explicit(budget),
            None => context::Chosen::measured(budget),
        };
        // **サブモデルのコンテキスト長も照会できない**（同じ理由——引く先が無い）。
        // 通るのは「引けなかった」経路で、抽出の上限は落とし所になる。
        let sub_context = context::choose_sub_context(profile_context.sub, None);
        let sub_context = match profile_context.sub {
            Some(_) => context::Chosen::explicit(sub_context),
            None => context::Chosen::measured(sub_context),
        };
        // **一度も聞いていない**（照会先が無い）。判定は全部「聞けなかった」になる。
        let upstream_facts = upstream::UpstreamFacts::new(
            upstream::UpstreamObservation::unqueried(
                measured::ModelFace::Main,
                config.llm().endpoint(),
                config.llm().model().as_str(),
            ),
            upstream::UpstreamObservation::unqueried(
                measured::ModelFace::Sub,
                config.sub_llm_endpoint(),
                config.sub_model().as_str(),
            ),
        );
        Self::bind_with(
            config,
            llm,
            sub,
            Resolved {
                budget,
                refresh_source: None,
                sub_context,
                sub_refresh_source: None,
                upstream_facts,
                // **照会先が無い**ので、起動の直後にも照会しない。
                check_at_start: context::CheckAtStart::NONE,
            },
            // **注入の入口は1つも起こさない。** 上流そのものがモックである。
            Launching::none(),
        )
        .await
    }

    /// 共有のトークン予算のハンドル。**テスト専用。**
    ///
    /// 走っているサーバのハンドラが**現在値**を読むこと（起動時の写しではないこと）を、
    /// 5分を実時間で待たずに確かめるための口。`Store::pool` と同じくゲートする
    /// ——本番から予算を書き換える手段は生やさない。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn budget_handle(&self) -> SharedBudget {
        self.refresh.shared()
    }

    /// 上流の観測の共有ハンドル。**テスト専用**（作業単位 G-3b）。
    ///
    /// **`serve` が `AppState` へ載せるのと同じ器**（`Arc` の複製）。
    /// **`G-10` が読む口ができるまで、本番の読み手はここだけ**である
    /// ——`AppState::upstream` の doc に、その状態を書いてある。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn upstream_handle(&self) -> SharedUpstream {
        self.upstream.clone()
    }

    /// 抽出の窓（サブモデルのコンテキスト長）の共有ハンドル。**テスト専用**（作業単位 A-13a-1）。
    ///
    /// **抽出器が読むのと同じ器**（`Arc` の複製 / `context::PendingRefresh::sub_context`）。
    /// **読むだけ**（`tokens_for_tests`）——本番から抽出の上限を書き換える手段は生やさない。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn sub_context_handle(&self) -> SharedSubContext {
        self.refresh.sub_context()
    }

    /// **走ったまま差し替えるときに読む置き場所を渡す。テスト専用**（作業単位 A-13a-1）。
    ///
    /// **[`Self::run_with_upstreams`] が [`Startup::with_profiles_file`] から写すのと同じ1行**
    /// （`app.profiles_file = …`）。**`run_with_upstreams` は `App` を手渡さない**ので、
    /// 予算の器（[`Self::budget_handle`]）を控えたまま切り替えを試すテストはここを通る
    /// （`tests/server_profile_context.rs`）。**読むだけの置き場所**（画面からは書かない）。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn with_profiles_file_for_tests(mut self, path: std::path::PathBuf) -> Self {
        self.profiles_file = profile_editor::ProfilesLocation::ReadOnly(path);
        self
    }

    /// **裏方（抽出と変容の1本の枠）が空いたかを見る口。テスト専用**（作業単位 A-8）。
    ///
    /// **`serve` が `AppState` へ載せるのと同じ抽出器**（`Arc` の複製）を見る。
    /// 往復を続けて投げるテストが、前の往復の裏方が終わったことを
    /// **固定の時間ではなく状態で待つ**ために使う（裏方は同時に1本だけで、
    /// 走っている間に来た往復の裏方は飛ぶ / ADR-0021 §7）。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn background_probe(&self) -> BackgroundProbe {
        BackgroundProbe::new(Arc::clone(&self.extraction))
    }

    /// 実際に開いたアドレス（ポート `0` を指定した場合の解決結果）。
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// 永続化層のハンドル。**テスト専用。**
    ///
    /// テストが停止後の状態（プールが閉じたか）を観測するために使う。
    /// 用途がテストなので `Store::pool` と同じくフィーチャでゲートする
    /// （宣言と実装が乖離していると、素の `cargo build` が乖離を検出できなくなる）。
    /// `&Store` は `Clone` でき `close()` も公開なので、本番から見えると
    /// 「稼働中のサーバのプールを外から閉じられるハンドル」になる。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub const fn store(&self) -> &Store {
        &self.store
    }

    /// `shutdown` が解決するまで応答し、**終了時に必ず `Store` を閉じる。**
    pub async fn serve<F>(self, shutdown: F) -> Result<(), ServerError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        // **コンテキスト長の定期的な引き直しをここで始める**（ADR-0020）。
        //
        // `bind` ではなく `serve` に置くのは、**止められる場所がここだから。**
        // 応答の経路には一切入らないので、本文の待ちは増えない（§3）。
        //
        // **予算はここで受け取る。** 引き直しが書く先そのものが返ってくるので、
        // 「引き直しには別の器を渡し、ハンドラには起動時の器を渡す」配線を
        // **書きようがない**（`context::PendingRefresh` の doc）。
        // **載るのを待つ側が読む器を、`spawn` の前に控える。**
        // `spawn` は `self.refresh` を食べるので、後からは取れない。
        let load_sinks_parts = (self.refresh.sub_context(), self.upstream.clone());
        // **「いま確かめる」の口も `spawn` の前に控える**（作業単位 B-66）——背景タスクと同じ器・同じ門。
        let checker = self.refresh.checker();

        let (refresh, budget) = self.refresh.spawn();

        // **自分で起こした上流が載るのを、背景で待つ**（ADR-0042 §決定3）。
        //
        // **`bind` ではなく `serve` に置くのは、止められる場所がここだから**
        // （`context::PendingRefresh::spawn` と同じ理由）。
        // **書く先は、引き直しが書く先そのもの**（`Arc` の複製）——
        // 別の器を作ると、**載った直後の観測だけが誰にも読まれない場所へ落ちる。**
        //
        // **差し替え（作業単位 F-4b-2）も同じ番人と器を使う**（`launch::Armed`）。
        let mut supervisor = self.supervisor;
        if let Some(clients) = self.face_clients.clone() {
            supervisor.arm(
                clients,
                launch::LoadSinks::new(
                    budget.clone(),
                    load_sinks_parts.0,
                    load_sinks_parts.1,
                    self.context_environment,
                ),
            );
        }
        // **差し替えの持ち主**（ADR-0042 §決定4）。**子の束はここへ移る。**
        // **ポートは実際に開いたもの**——`0`（OS に選ばせる）で起動しても、起こす面との取り合いを見られる。
        //
        // **差し替えが読む置き場所と、画面が読み書きする置き場所は、同じ1つの値から作る**
        // （作業単位 F-4b-3）——別々の値を渡す配線を書かない。
        //
        // **覚える先（ADR-0042 §決定9）も同じ値から作る**——既定の置き場所のときだけ覚える。
        //
        // **いま選ばれている名前（作業単位 F-10b）は起動時の選択から始め、以後は差し替えが書き換える。**
        let switcher = Arc::new(switch::Switcher::new(
            supervisor,
            self.profiles_file.path().map(std::path::Path::to_path_buf),
            remembered::memory_for(&self.profiles_file),
            self.local_addr.port(),
            self.selected_profile,
        ));
        // **保存の置き場所は、差し替え・画面の保存と同じ1つの値から**（ADR-0057 §決定5 / `listening_port::memory_for`）。
        // **番号は実際に開いた番号**（`local_addr`。設定値ではない）。
        let listening = Arc::new(listening_port::Listening::new(
            self.local_addr.port(),
            self.opened,
            listening_port::memory_for(&self.profiles_file),
        ));
        let profile_editor = Arc::new(profile_editor::ProfileEditor::new(self.profiles_file));

        // **抽出器はここで手元にも残す。** 停止時に走っている抽出を落とすため
        // （ADR-0021 §9）。`AppState` の側は各ハンドラが複製して持ち回る。
        let extraction = Arc::clone(&self.extraction);

        let state = AppState {
            store: self.store,
            llm: self.llm,
            budget,
            extraction: self.extraction,
            ui: self.ui,
            // **引き直しが書く先そのもの**（`bind_with` が `refresh` から受け取った複製）。
            upstream: self.upstream,
            checker,
            // **起こした上流の状態も、書き手と同じ器**（`launch` モジュールの doc）。
            launch_states: self.launch_states,
            profile_sample: self.profile_sample,
            switcher: Arc::clone(&switcher),
            profile_editor,
            agreement_fits: Arc::default(),
            // **起動のたびに空から始める**（ADR-0052 §推奨4「起動」で世代が変わる）。
            uncountable: Arc::new(seal::UncountableMemory::default()),
            // **部品の数の記憶も空から始める**（ADR-0052 §推奨6。冷えた最初のターンが少し遅くなるだけ）。
            parts: Arc::new(counted_plan::PartMemory::default()),
            listening,
        };
        let store = state.store.clone();

        // **`Host` / `Origin` と比べるのは実際に開いたポート**（作業単位 B-58 / `host_guard` の doc）
        // ——設定値（`config.bind()`）ではない。0（OS に選ばせる）で起動しても正しく比べる。
        let port = host_guard::ListeningPort::new(self.local_addr.port());
        let result = axum::serve(self.listener, router(state, port))
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(ServerError::Serve);

        // **サーバが止まるときに残さない**（ADR-0020 §3）。落とすと abort される。
        // スコープの終わりでも落ちるが、**停止処理の一部として明示する。**
        drop(refresh);

        // **走っている記憶抽出も落とす**（ADR-0021 §9）。上流の生成も一緒に止まる。
        //
        // **`Store::close` より前に置いてある。** 逆にすると、閉じたプールへ
        // 書きに行くタスクが残る分だけ `run` の `add_memory` が失敗し、
        // 停止処理の最中に `error!`（「抽出した記憶を保存できませんでした」）が
        // 出るだけで、得るものが無い。
        //
        // # **この順序を守っているのはこのコメントだけ**（13b のレビューで判明）
        //
        // 2文を入れ替える変異は **1030/1030 緑**で通り抜ける（この作業単位で実測）。
        // **見張り番は無い。**
        // 足していないのは、**入れ替えたときの実害が「停止時のログが1行増える」
        // だけ**だから——`Store::close` は冪等で、`add_memory` の失敗は
        // `run` の中で握り潰される（応答はとうに返っている）。
        // **順序に寄りかかった主張をこの下に足すなら、そのときに見張り番も足すこと。**
        extraction.shutdown();

        // **控えた子だけを止め、終了を待つ**（ADR-0042 §決定2）。
        //
        // **載るのを待っていた番人は、子ごとに先に止まる**（`launch::Supervisor::stop`）
        // ——止めないと、止めた子を見て「失敗」と書きに来る。
        //
        // **`Store::close` より前**——GPU を掴んだままにしない。
        // **止めるのは自分が起こしたものだけ**で、名前で探す経路はこの家に無い
        // （`launch` モジュールの doc / `F-9` が実機で踏んだ形）。
        // **差し替え（作業単位 F-4b-2）で起こし直した子も、同じ束に在る。**
        switcher.shutdown().await;

        // **成功・失敗のどちらでも閉じる。** ここを通らないと WAL のデータが残る。
        store.close().await;
        tracing::info!("データベースを閉じました");

        result
    }
}

/// ルーティング。
///
/// `/v1/` は OpenAI 互換の境界（ADR-0001）、`/admin/` は本サービス独自の管理 API
/// （ADR-0008）。**混ぜない。** OpenAI が将来 `/v1/characters` 等を定義しても衝突しない。
///
/// **`port` は実際に待ち受けているポート**（`Host` / `Origin` の検査が比べる先 / `host_guard`）。
fn router(state: AppState, port: host_guard::ListeningPort) -> Router {
    Router::new()
        .route("/v1/models", get(models::list_models))
        // 本文生成。**`stream: true` なら SSE 中継**（`chat` → `relay`）。
        .route("/v1/chat/completions", post(chat::chat_completions))
        // **上流について観測できたこと**（ADR-0039 §決定4 / 作業単位 G-10）。**読むだけ。**
        //
        // **この入口には書き込みの口を生やさない。** 2026-09-07 の利用者の決定
        // （「見えるだけでよい。起こさない。選ばせもしない」）は、**ADR-0042 §決定5 で
        // 「自分が書いたプロファイルから選ぶ」まで置き換わった**（2026-09-18）——
        // **選ぶ口は下の `/admin/upstream/profile` の1本だけ**で、ここは今も `GET` だけ。
        // **1つの面を指すパス（`.../upstream/main`）も生やさない**
        // ——生やせば `PUT`（面の綴りの書き換え）が自然な形になる。
        //
        // **⚠ `/v1/models` へ混ぜないこと。** あちらはキャラクターの一覧（ADR-0004）で、
        // 上流のモデルを並べた瞬間に「`model` に書ける綴り」と「書けない綴り」が同じ一覧に混ざる。
        // 見張り番は `tests/server_admin_upstream.rs`。
        .route("/admin/upstream", get(upstream::get_upstream))
        // **両方の面の上流に、いま照会する**（作業単位 B-66 / Issue origin#94）。上流モデルの画面の「いま確かめる」。
        //
        // **観測を書き換える口ではない** —— 照会するのは5分ごとと同じ中身（`context::UpstreamChecker`）で、
        // 宛先も綴りも本文で受け取らない（本文は読まない）。**面を指すパスにもしない**（両方の面をまとめて照会する）。
        .route("/admin/upstream/check", post(upstream::check_upstream))
        // **走ったまま上流を差し替える**（ADR-0042 §決定4・5 / 作業単位 F-4b-2）。
        //
        // **`/admin/upstream` そのものには書き込みの口を生やさない**（`GET` だけ / 405）。
        // **差し替えるのは「利用者が `upstreams.json` に書いたプロファイルの名前」だけ**で、
        // 宛先やモデルの綴りを本文で受け取る口は作らない（ADR-0042 §決定5 ——
        // **探さない・落とさない。自分が書いたものから選ぶ**）。
        // **1つの面を指すパス（`.../upstream/main`）も生やさない**——差し替えは面ごとに
        // 新旧を比べて決める（`switch::plan_switch`）もので、面を名指しして書き換えるものではない。
        .route("/admin/upstream/profile", post(switch::switch_profile))
        // **上流プロファイルを読む・足す・直す**（ADR-0042 §決定5・8 / 作業単位 F-4b-3）。
        //
        // **書く口は `PUT`（1つのプロファイルを丸ごと置き換える）だけ。`DELETE` は生やさない**
        // （ADR-0042 §決定8「消す操作はまだ置かない」）。**書けるのは既定の置き場所だけ**
        // （`PERSONACORE_UPSTREAMS` が指す先は 409）。**面を指すパスも生やさない。**
        .route(
            "/admin/upstream/profiles",
            get(profile_editor::list_profiles),
        )
        .route(
            "/admin/upstream/profiles/{name}",
            put(profile_editor::save_profile),
        )
        // **personacore 自身の待ち受けのポート**（ADR-0057 §決定5・6 / 作業単位 F-12a）。
        //
        // **上流の口（`/admin/upstream`）に混ぜない**——推論サーバの話と、personacore 自身の待ち受けの話を分ける。
        // **書く口は `PUT .../port` の1本だけ**で、保存は次の起動から効く（待ち受けは移さない）。
        // **この `router` に足す**（`host_guard` を通る。設定を書く口なので DNS rebinding の検査が守る経路そのもの）。
        .route("/admin/server", get(listening_port::get_server))
        .route(
            "/admin/server/port",
            put(listening_port::save_listening_port),
        )
        // 管理 API。**`memory` を消す・書き換える経路はここに生やさない**
        // （ADR-0022 / `admin` の doc）。**生えているのは下の2本だけ**——
        // 読む入口（`GET .../memories`）と、行を追加する入口（`POST .../corrections`）。
        // `DELETE` も `PUT` も無く、1件の記憶を指すパスすら無い。
        // 見張り番は `tests/server_admin.rs::no_admin_endpoint_deletes_or_rewrites_a_memory`。
        .route(
            "/admin/characters",
            get(admin::list_characters).post(admin::create_character),
        )
        // **1件を指すパス。** 受けるのは `PATCH` と `DELETE` だけで、
        // `PUT`（丸ごと置き換え = 識別子ごと差し替える形）は生やさない（ADR-0016 §1）。
        .route(
            "/admin/characters/{handle}",
            patch(admin::update_character).delete(admin::delete_character),
        )
        .route(
            "/admin/characters/{handle}/stats",
            put(admin::replace_stats).get(admin::get_stats),
        )
        // **変容の履歴**（ADR-0024 §9 / 作業単位 D-4a）。**読むだけ。**
        //
        // 変容は本文にもユーザーにも見せない決定なので、**ログを読まないと
        // 起きたことが一切見えない。** ここはその唯一の入口である。
        //
        // **書き込みの口を足さない。** 票は追記のみ（ADR-0026 §決定3）で、
        // 書くのは本文処理だけ。外から書き換えられる記録は記録ではない。
        // **1件を指すパス（`.../drift/{id}`）も生やさない**——生やせば
        // `PUT`（置き換え）と `DELETE`（削除）が自然な形になり、
        // 記憶の側で塞いだのとまったく同じ穴が開く。
        // 見張り番は `tests/server_admin_drift.rs::only_get_is_exposed_for_the_drift_path`。
        .route("/admin/characters/{handle}/drift", get(admin::get_drift))
        // **属性も `PUT`**（`store` の意味論が丸ごと置き換えだから / ADR-0009 / 作業単位 12a）。
        // `PATCH` に見せると部分更新だと誤解され、「消したはずの属性が残る」と読まれる。
        .route(
            "/admin/characters/{handle}/attributes",
            put(admin::replace_attributes).get(admin::get_attributes),
        )
        // **属性の変更の履歴**（ADR-0028 §決定5 / 作業単位 A-3a）。**読むだけ。**
        //
        // 属性は削れない枠に毎ターン載るので、**静かに変わると気付く手段が無い。**
        // ここはその唯一の入口である（変容における `.../drift` と同じ立場）。
        //
        // **書き込みの口を足さない。** 履歴は追記のみで、書くのは上の `PUT` ひとつ
        // （§決定5「書き込み経路は1つ」）。**1件を指すパス（`.../revisions/{id}`）も
        // 生やさない**——生やせば `PUT` と `DELETE` が自然な形になり、
        // 記憶と票の側で塞いだのと同じ穴が開く。
        // 見張り番は `tests/server_admin_attributes.rs::only_get_is_exposed_for_the_revisions_path`。
        .route(
            "/admin/characters/{handle}/attributes/revisions",
            get(admin::get_attribute_revisions),
        )
        // **取り決め（約束・決まり）**（ADR-0051 §決定5・6 / 作業単位 A-12a）。
        //
        // **書く口は2本だけ**——手で書く（`POST .../agreements`）と、手で終わらせる
        // （`POST .../{agreement_id}/transitions` ＝ 履歴の行の追加）。**`PUT` も `DELETE` も生やさない**
        // （行は消さない / ADR-0051 §決定1）。**1件の取り決めを指すパス（`.../agreements/{id}`）も
        // 生やさない**——生やせば `PUT` と `DELETE` が自然な形になる。
        // 見張り番は `tests/server_admin_agreements.rs::only_the_listed_methods_are_exposed`。
        .route(
            "/admin/characters/{handle}/agreements",
            get(admin::list_agreements).post(admin::create_agreement),
        )
        .route(
            "/admin/characters/{handle}/agreements/transitions",
            get(admin::list_agreement_transitions),
        )
        .route(
            "/admin/characters/{handle}/agreements/{agreement_id}/transitions",
            post(admin::end_agreement),
        )
        // **提案の採否**（ADR-0051 §決定2 / 作業単位 A-12c-1）。提案は裏方が毎往復書く（`agreement_proposal`）。
        // 採る（`.../adoption`）・捨てる（`.../rejection`）も履歴の行の追加（提案中 → 有効／却下）で、行は消えない。
        // **提案中の一覧は `GET .../agreements` の `state` が `proposed` の行**（口を増やさない）。
        .route(
            "/admin/characters/{handle}/agreements/{agreement_id}/adoption",
            post(admin::adopt_agreement),
        )
        .route(
            "/admin/characters/{handle}/agreements/{agreement_id}/rejection",
            post(admin::reject_agreement),
        )
        // **軸の候補**（ADR-0046 §決定6 / 作業単位 A-9a）。**読むだけ。**
        //
        // **書き込みの口を足さない。** 候補を書くのは裏方（A-9b / A-9c）と第2段の採用・却下だけ。
        // **1件を指すパス（`.../axis-candidates/{id}`）も生やさない**（属性の履歴と同じ理由）。
        // 見張り番は `tests/server_admin_axis_candidates.rs::only_get_is_exposed_for_the_candidates_path`。
        .route(
            "/admin/characters/{handle}/axis-candidates",
            get(admin::get_axis_candidates),
        )
        // **修復のための記憶一覧**（ADR-0022 §2 / 作業単位 14b）。**読むだけ。**
        // 想起（`recall_memories`）とは別の入口で、返すものはプロンプトに載らない。
        //
        // **上書きされた記憶もここから読む**（`?include=superseded` / 作業単位 D-3b）。
        // **経路を増やしていない**——`.../memories/history` のような2本目を生やすと、
        // 「そのキャラクターの記憶を読む入口」が2つになり、どちらが正かが分かれる。
        // **既定の応答は 14b から1バイトも変わらない**（見張り番は
        // `tests/server_admin_memories.rs::by_default_the_listing_says_nothing_about_superseded_memories`）。
        .route(
            "/admin/characters/{handle}/memories",
            get(admin::list_memories),
        )
        // **訂正版で置き換える**（ADR-0022）。**`POST`＝行の追加**であり、
        // 起きることもそれ（新しい行を INSERT し、古い行にそれを指させる）。
        //
        // **1件の記憶を指すパス（`.../memories/{memory_id}`）は生やさない。**
        // 生やせば `PUT`（置き換え）と `DELETE`（削除）が自然な形になり、
        // **ADR-0022 が却下した2つの操作**がパスとして表現できてしまう。
        // 消す道はここに存在しない（`memory` の行は消さない / 絶対制約）。
        .route(
            "/admin/characters/{handle}/memories/{memory_id}/corrections",
            post(admin::correct_memory),
        )
        // **会話履歴を読む口**（作業単位 D-12c-1 / Issue origin#35）。**読むだけ。**
        //
        // 会話そのものは DB に残っていたのに、**読む入口が1本も無かった**
        // （画面から過去の文章を遡る方法が無かった）。
        //
        // **書き込みの口を足さない。** 会話を書くのは本文処理だけで、
        // `session.updated_at` の押し上げと不可分に行う（ADR-0004）。
        // **1件のセッション・1件のメッセージを指すパスも生やさない**——生やせば
        // `PUT` と `DELETE` が自然な形になり、`message.raw_content` を消す道が表現できてしまう。
        // 見張り番は `tests/server_admin_sessions.rs::only_get_is_exposed_for_the_session_paths`。
        .route(
            "/admin/characters/{handle}/sessions",
            get(admin::list_sessions),
        )
        .route(
            "/admin/characters/{handle}/sessions/{session_id}/messages",
            get(admin::list_session_messages),
        )
        // **API の仕様書を配る口**（ADR-0047 §② P-2b）。**`--no-ui` でも配る**——画面ではなく
        // API の一部。**この口自身も仕様書に載っている**（ルート表との突き合わせの対象のまま）。
        // `/v1` / `/admin` の下に置かないのは、どちらの境界にも属さない（OpenAI 互換でも管理でもない）から。
        .route("/openapi.json", get(openapi::get_document))
        // 未知の**パス**はここへ来る。`fallback` はここにしか効かない。
        //
        // **UI（ADR-0031）へ落ちるのもここ。** ただし `/v1` / `/admin` の下は
        // **404 のまま**——`unknown_route` の doc と `web` モジュールの doc。
        //
        // **`ServeDir` を `fallback_service` に直接据えないこと。**
        // 据えると `/v1/typo` が UI に飲み込まれ、
        // **綴りを間違えた API 呼び出しに `index.html` が 200 で返る。**
        .fallback(unknown_route)
        // パスは合っているがメソッドが違う場合は 405。**別に指定しないと空ボディになる。**
        .method_not_allowed_fallback(unsupported_method)
        // **本文の上限はここで見る。** 抽出器に任せると、上限に達した時点で
        // 読むのをやめて応答を書くため、まだ書いているクライアントを RST で
        // 巻き添えにして 413 が届かない（`body` モジュールの doc / docs/06-roadmap.md）。
        .layer(axum::middleware::from_fn(body::limit_request_body))
        // **`Host` / `Origin` の検査**（作業単位 B-58 / `host_guard` の doc）。**画面も API も区別しない。**
        //
        // **本文の上限より外**——断る要求の本文を読まない（見張り番は `tests/server_host_guard.rs` の
        // `a_foreign_host_is_refused_before_the_body_is_read`）。**版のヘッダより内**——この 403 にも
        // 版のヘッダが付く（ADR-0047 §② P-2b「ルータに届いた要求への応答には付く」）。
        // **フォールバック（画面・API の 404）と 405 にも効く**のは、版のヘッダと同じく
        // `.layer` がそれより前に足したルートとフォールバックの全部を包むから。
        .layer(axum::middleware::from_fn_with_state(
            port,
            host_guard::guard,
        ))
        .layer(TraceLayer::new_for_http())
        // **API の版のヘッダはいちばん外**（ADR-0047 §② P-2b）。内側のレイヤ（本文の上限）が
        // 返す 413 にも、フォールバック・405 にも付く。**`.layer` はそれより前に足した
        // ルートとフォールバックにしか効かない**ので、この行は最後に置く。
        .layer(axum::middleware::map_response(openapi::stamp_api_version))
        .with_state(state)
}

/// **上流の出力の落とし先**（ADR-0042 §決定2 / 作業単位 F-4b-1）。**純粋関数。**
///
/// **DB と同じディレクトリの下**（`upstream-logs/`）。**見る場所を2つにしない。**
/// DB のパスに親が無い綴り（`personacore.sqlite` だけ）ならカレントの下に作る
/// ——`AppConfig` はそこまでは縛らない。
fn upstream_log_dir(database_path: &std::path::Path) -> std::path::PathBuf {
    database_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(launch::UPSTREAM_LOG_DIR_NAME)
}

/// **ディスクの配り先から1件も配れないときの警告の本文**（ADR-0031 §結果）。
///
/// 定数にしてあるのは、見張り番（`the_startup_line_says_where_the_ui_came_from`）が
/// **綴りを写さずに参照できる**ようにするため。
const WEB_DIR_MISSING_MESSAGE: &str =
    "UI の配り先が見つかりません。`pnpm build` を実行するか PERSONACORE_WEB_DIR を確認してください";

/// **埋め込んだ画面が1件も無いときの警告の本文**（ADR-0041 §決定2 / 作業単位 F-4a）。
///
/// **`WEB_DIR_MISSING_MESSAGE` と別にしてあるのは、直す手が違うから。**
/// ディスクなら `pnpm build` だけでよいが、**埋め込みは組み直しが要る**
/// ——埋め込みはコンパイル時に決まるので、`pnpm build` しても
/// **`cargo build` し直さない限り実行ファイルの中の画面は変わらない。**
const UI_EMBED_EMPTY_MESSAGE: &str = "画面が実行ファイルに1件も埋め込まれていません。\
     `pnpm build` の後に `cargo build` し直すか、PERSONACORE_WEB_DIR を設定してください";

/// 未知のパスのハンドラ。**リクエストの内容を反射しない。**
///
/// # ⚠ **API の 404 を UI が飲み込まないこと**（ADR-0031 §4）
///
/// `/v1/*` と `/admin/*` は**必ず OpenAI 形式のエラー**で返す。
/// UI へ落ちるのは、その2つに属さないパスだけ。
/// 判定は `web::is_api_path`（純粋関数）、見張り番は
/// `tests/server_web_ui.rs::the_ui_fallback_never_swallows_an_api_error`。
///
/// **`Request` は最後の引数**（本文を消費する抽出器は1つだけ、かつ末尾）。
async fn unknown_route(State(state): State<AppState>, request: Request) -> Response {
    if web::is_api_path(request.uri().path()) {
        return ApiError::not_found().into_response();
    }
    state.ui.respond(request).await
}

/// 未対応メソッドのハンドラ。**リクエストの内容を反射しない。**
async fn unsupported_method() -> ApiError {
    ApiError::method_not_allowed()
}

/// Windows のコンソール制御イベントを1種類待つ。
///
/// **登録に失敗した経路は永久に待たせる。** 即座に解決させると起動直後に
/// サーバが止まる（1つでも登録に失敗したら全体が落ちる、という壊れ方になる）。
///
/// 種類ごとに戻り値の型が違う（`CtrlBreak` / `CtrlClose` / …）ので、
/// 関数ではなくマクロで受ける。
#[cfg(windows)]
macro_rules! windows_console_signal {
    ($register:path, $label:literal) => {
        async {
            match $register() {
                Ok(mut signal) => {
                    signal.recv().await;
                    tracing::info!(signal = $label, "終了要求を受け取りました");
                }
                Err(err) => {
                    tracing::error!(
                        error = %err,
                        signal = $label,
                        "シグナルハンドラを登録できませんでした"
                    );
                    std::future::pending::<()>().await;
                }
            }
        }
    };
}

/// Ctrl+C（と OS の終了要求）を待つ。
///
/// **ハンドラを登録できなかった場合は、その経路だけを永久に待たせる。**
/// 即座に解決させると起動直後にサーバが止まる。
///
/// Windows では **`Ctrl+C` と `Ctrl+Break` は別のイベント**で、コンソールを閉じた場合・
/// ログオフ・シャットダウンもそれぞれ独立している。1つだけ拾うと、残りは
/// 「猶予時間を待たされた末に強制終了」＝ `Store::close()` を通らない停止になる。
/// **`taskkill /F` と違い、これらは救える経路なので全部拾う。**
pub async fn shutdown_signal() {
    let ctrl_c = async {
        match tokio::signal::ctrl_c().await {
            Ok(()) => tracing::info!("Ctrl+C を受け取りました"),
            Err(err) => {
                tracing::error!(error = %err, "Ctrl+C ハンドラを登録できませんでした");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(windows)]
    {
        use tokio::signal::windows;

        tokio::select! {
            () = ctrl_c => {}
            () = windows_console_signal!(windows::ctrl_break, "ctrl_break") => {}
            () = windows_console_signal!(windows::ctrl_close, "ctrl_close") => {}
            () = windows_console_signal!(windows::ctrl_logoff, "ctrl_logoff") => {}
            () = windows_console_signal!(windows::ctrl_shutdown, "ctrl_shutdown") => {}
        }
    }

    #[cfg(unix)]
    {
        let terminate = async {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut signal) => {
                    signal.recv().await;
                    tracing::info!("SIGTERM を受け取りました");
                }
                Err(err) => {
                    tracing::error!(error = %err, "SIGTERM ハンドラを登録できませんでした");
                    std::future::pending::<()>().await;
                }
            }
        };

        tokio::select! {
            () = ctrl_c => {}
            () = terminate => {}
        }
    }

    #[cfg(not(any(windows, unix)))]
    {
        ctrl_c.await;
    }

    tracing::info!("停止処理を開始します");
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use super::*;
    use crate::testlog;

    /// 起動ログの本文。**この綴りが変わったら、下の検査は行を見つけられずに落ちる。**
    const STARTUP_MESSAGE: &str = "待ち受けを開始しました";

    /// 起動ログに載せる設定。**どの欄も互いに違う値**（下の doc の「前提」）。
    ///
    /// **どれも既定ではない**——既定のままだと「その欄が何を写していても緑」になる
    /// （CLAUDE.md「テスト専用の入口を作ると、本番の入口が観測対象から外れる」の実例）。
    ///
    /// **`ENV_WEB_DIR` の値に空白を入れないこと。** `fields_of` はトークンで割るので、
    /// 空白を含むパスは組の列が崩れる（**ここで測りたいのは帰属であって
    /// パスの書式ではない**ので、崩れない値を選ぶ）。
    const STARTUP_SETTINGS: [(&str, &str); 14] = [
        // **2つの上流には別の綴りを与える**（ADR-0039 §決定4 / 作業単位 G-3a）。
        // 同値だと、2つの欄を取り違える変異が観測できない。
        (ENV_LLM_ENDPOINT, "http://127.0.0.1:19311"),
        (ENV_SUB_LLM_ENDPOINT, "http://127.0.0.1:19322"),
        (ENV_CONTEXT_TOKENS, "12345"),
        (ENV_STAT_DRIFT, "shadow"),
        (ENV_DRIFT_FORM, "single"),
        (ENV_DRIFT_NOTE, "copy"),
        (ENV_DRIFT_DECAY, "0.25"),
        (ENV_DRIFT_SLOPE, "1.25"),
        (ENV_DRIFT_STEP_COST, "0.75"),
        (ENV_AXIS_DISCOVERY_MEMORIES, "7"),
        // **3つとも既定（60 / 10 / 90）ではなく、互いにも他の欄とも違う値**（作業単位 A-9c）。
        (ENV_AXIS_TRIAL_TURNS, "45"),
        (ENV_AXIS_TRIAL_MIN_NONZERO, "6"),
        (ENV_AXIS_TRIAL_LINE_PERCENT, "85"),
        (ENV_WEB_DIR, "web-dist-for-the-startup-line"),
    ];

    /// 指定した本文を含む行から `キー=値` を**書かれた順に**取り出す。
    ///
    /// **`contains("drift_note=copy")` で1件ずつ見ない。** それだと
    /// **欄が増えても減っても緑**で、並びも見ていない。
    /// トークンを丸ごと組にして取り出し、**組の列そのもの**を突き合わせる。
    ///
    /// # 行を選ぶのは共有の1本（作業単位 B-36）
    ///
    /// **B-36 より前は、ここに [`testlog::line_containing`] の本文を写していた**
    /// （`lines` → `find(contains)` → `unwrap_or_else(panic!)` の3行）。
    /// **写しは `tests/**` を読む走査の外に居た**ので、誰も気付かなかった
    /// （`crate::testlog` の `SHARED_BODIES` の doc「置き場所の限界」）。
    ///
    /// **手掛かりは `tracing` のメッセージ**（地の文）なので、
    /// 欄で選ぶ `line_with_field` ではなく[`testlog::line_containing`] が正しい
    /// ——**使い分けの権威は `crate::testlog` の `has_field` の doc の表。**
    ///
    /// [`testlog::line_containing`]: crate::testlog::line_containing
    fn fields_of(output: &str, message: &str) -> Vec<(String, String)> {
        testlog::line_containing(output, message)
            .split_whitespace()
            .filter_map(|token| token.split_once('='))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }

    /// 待ち受けから停止までを1往復させ、その間のログと実際のアドレスを返す。
    ///
    /// **`serve` まで通して閉じる。** `Store` を開いたまま落とさない。
    fn captured_startup(config: &AppConfig) -> (String, SocketAddr) {
        let mut local_addr = None;
        let output = testlog::captured_under_default_filter(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime を作れること");
            runtime.block_on(async {
                let app = App::bind_with_clients(
                    config,
                    Arc::new(extract::SilentUpstream),
                    Arc::new(extract::SilentUpstream),
                )
                .await
                .expect("待ち受けられること");
                local_addr = Some(app.local_addr());
                app.serve(std::future::ready(()))
                    .await
                    .expect("停止まで通ること");
            });
        });

        (output, local_addr.expect("待ち受けたアドレスがあること"))
    }

    /// **起動ログの欄が、それぞれ自分の値の隣にある**（16m のレビュー / 作業単位 20a）。
    ///
    /// # 何を捕まえるためのものか
    ///
    /// **`drift_note = %drift_form.label()` のような入れ替えは、これまで全緑で通っていた**
    /// （実測 / 20a の着手前 —— 1470 passed / 0 failed）。
    /// **`assert_ne!` では防げない。値は全部違っていて、組み合わせだけが間違っている**
    /// （CLAUDE.md「抜けるのはいつも帰属」）。
    ///
    /// # 前提: どの欄も違う値であること
    ///
    /// 1つでも同値だと、その2つの取り違えが観測できなくなる。
    /// **テストの中で実際に確かめる**（フィクスチャを書き換えたときに黙って死なせない）。
    ///
    /// # 並びまで見る
    ///
    /// 運用者はこの1行を読み比べて腕を区別する（ADR-0027 §12 / ADR-0029 §訂正）ので、
    /// **欄の増減も並び替えもここで赤になる。**
    ///
    /// # ⚠ 綴りで探さない
    ///
    /// `tracing` はフィールド名と `=` の間に ANSI エスケープを挿入しうる。
    /// `testlog` の捕捉は `with_ansi(false)` なので素の綴りで出るが、
    /// **それに寄りかからずトークンとして割る**（`fields_of`）。
    #[test]
    fn the_startup_line_shows_every_setting_next_to_its_own_name() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let config = AppConfig::from_lookup(|key| {
            STARTUP_SETTINGS
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned())
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(dir.path().join("startup.sqlite"));

        let (output, local_addr) = captured_startup(&config);
        let want: Vec<(String, String)> = [
            ("addr", local_addr.to_string()),
            // **2つの上流は、それぞれ自分の綴りの隣に出ること**（ADR-0039 §決定4）。
            (
                "llm_endpoint",
                "http://127.0.0.1:19311/v1/chat/completions".to_owned(),
            ),
            (
                "sub_llm_endpoint",
                "http://127.0.0.1:19322/v1/chat/completions".to_owned(),
            ),
            ("context_tokens", "12345".to_owned()),
            ("stat_drift", "shadow".to_owned()),
            ("drift_form", "single".to_owned()),
            ("drift_note", "copy".to_owned()),
            ("drift_decay", "0.25".to_owned()),
            ("drift_slope", "1.25".to_owned()),
            ("drift_step_cost", "0.75".to_owned()),
            // **既定（10）ではない値**（作業単位 A-9b）。
            ("axis_discovery_memories", "7".to_owned()),
            // **既定（60 / 10 / 90）ではない値**（作業単位 A-9c）。
            ("axis_trial_turns", "45".to_owned()),
            ("axis_trial_min_nonzero", "6".to_owned()),
            ("axis_trial_line_percent", "85".to_owned()),
            // **配り先が設定されているので、画面はディスクから来る**（ADR-0041 §決定2）。
            // 埋め込み側との対比は `the_startup_line_says_where_the_ui_came_from`。
            ("ui_source", "disk".to_owned()),
            // **設定した綴りがそのまま出ること**（`.` を前置きした写しではない）。
            ("ui_origin", "web-dist-for-the-startup-line".to_owned()),
            // **この設定の配り先は実在しない**（作っていない）ので 0 件。
            ("ui_files", "0".to_owned()),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect();

        // **前提**——どの欄も違う値（同値だと取り違えが観測できない）。
        let mut distinct: Vec<&str> = want.iter().map(|(_, value)| value.as_str()).collect();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            want.len(),
            "前提: 起動ログの欄が互いに違う値であること: {want:?}"
        );

        assert_eq!(
            fields_of(&output, STARTUP_MESSAGE),
            want,
            "起動ログの欄と値の対応が違う: {output:?}"
        );
    }

    /// **起動ログが、本文の上流と抽出の上流を別々に名乗る**
    /// （[ADR-0039](../../docs/adr/0039-two-upstreams-for-two-jobs.md) §決定4 / 作業単位 G-3a）。
    ///
    /// # なぜ要るのか
    ///
    /// **本文だけ生きていて抽出が死んでいる状態は、いまも起きる**（記憶が育たない）。
    /// **新しいのは「別々に落ちる」こと**で、`map_request_error` は
    /// **両方から呼ばれる**ので、あの行だけでは**どちらの往復の話か分からない。**
    /// 起動時に2つの綴りが出ていれば、後から突き合わせられる。
    ///
    /// # なぜ2つの腕を1本に置くか
    ///
    /// **設定した腕だけでは足りない。** 「常に抽出の綴りを出す」実装も満たす。
    /// **未設定の腕（2つが同じ綴りになる）と一緒に置いて初めて**、
    /// 「常に本文の綴りを出す」実装も赤になる
    /// （CLAUDE.md「対比は安い側だけでは足りない」）。
    ///
    /// | 腕 | `PERSONACORE_SUB_LLM_ENDPOINT` | 期待 |
    /// |---|---|---|
    /// | 2つの上流 | 本文とは別の綴り | **2つの欄が別々の値** |
    /// | 1つの上流 | 未設定 | **2つの欄が同じ値**（＝本文の綴り） |
    ///
    /// **どちらの腕のエンドポイントも既定ではない**——既定だと
    /// 「設定を読まずに `DEFAULT_ENDPOINT` を出す」実装と区別が付かない。
    #[test]
    fn the_startup_line_names_the_extraction_upstream_apart_from_the_body_one() {
        const MAIN: &str = "http://127.0.0.1:19411";
        const SUB: &str = "http://127.0.0.1:19422";
        const MAIN_WIRE: &str = "http://127.0.0.1:19411/v1/chat/completions";
        const SUB_WIRE: &str = "http://127.0.0.1:19422/v1/chat/completions";

        assert_ne!(MAIN, SUB, "前提: 2つは別の綴りであること");
        // **既定の綴りは、既定で組んだ設定から読む**（定数をここへ写さない——
        // 写すと、定数を変える変異と一緒に期待値が動く）。
        let shipped = AppConfig::from_lookup(|_| None)
            .expect("既定値だけで組み立てられること")
            .llm()
            .endpoint()
            .to_owned();
        for wire in [MAIN_WIRE, SUB_WIRE] {
            assert_ne!(
                wire, shipped,
                "前提: 既定の上流とは別の綴りであること（既定だと設定を読まない実装と区別できない）"
            );
        }

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");

        for (sub_setting, want_sub) in [(Some(SUB), SUB_WIRE), (None, MAIN_WIRE)] {
            let config = AppConfig::from_lookup(|key| match key {
                ENV_LLM_ENDPOINT => Some(MAIN.to_owned()),
                ENV_SUB_LLM_ENDPOINT => sub_setting.map(str::to_owned),
                _ => None,
            })
            .expect("設定を組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(dir.path().join("startup-upstreams.sqlite"));

            let (output, _) = captured_startup(&config);
            let fields = fields_of(&output, STARTUP_MESSAGE);

            assert!(
                fields.contains(&("llm_endpoint".to_owned(), MAIN_WIRE.to_owned())),
                "本文の上流を名乗ること（{sub_setting:?}）: {fields:?} / {output:?}"
            );
            assert!(
                fields.contains(&("sub_llm_endpoint".to_owned(), want_sub.to_owned())),
                "抽出の上流を名乗ること（{sub_setting:?} → {want_sub}）: {fields:?} / {output:?}"
            );
        }
    }

    /// **起動ログが、画面がどこから来たかを正直に言う**
    /// （ADR-0031 §結果 / 作業単位 D-0 / ADR-0041 §決定2 / 作業単位 F-4a）。
    ///
    /// **`the_startup_line_says_whether_the_web_dir_exists` の後継。**
    /// **配り方が2通りになった**ので、欄が `web_dir` / `web_dir_exists` から
    /// `ui_source` / `ui_origin` / `ui_files` へ移った。
    ///
    /// # なぜこれが要るのか
    ///
    /// **`web/dist` はビルド成果物で `.gitignore` に入る。**
    /// ビルドし忘れると画面が出ない／古いまま出るが、
    /// **サーバは何事もなく起動する**（画面の有無は起動条件ではない）。
    /// **埋め込みになって危険が増えた**——`pnpm build` しても
    /// **`cargo build` し直すまで実行ファイルの中の画面は変わらない**（ADR-0041 §結果）。
    /// **「Finished in 0.4s は直っていない」と同じ罠**を FE 側に作らないための1行。
    ///
    /// # なぜ3つの腕を1本に置くか
    ///
    /// **高い側と安い側を1本に置いて初めて、両方向の決め打ちが赤になる**（CLAUDE.md）。
    ///
    /// | 腕 | `PERSONACORE_WEB_DIR` | 期待 |
    /// |---|---|---|
    /// | ディスク・中身あり | 実在するディレクトリ（ファイル2件） | `disk` / その綴り / `2` / 黙る |
    /// | ディスク・中身なし | 無いディレクトリ | `disk` / その綴り / `0` / **配り先の警告** |
    /// | 埋め込み | 未設定 | `embedded` / 埋め込み元の綴り / 実物の件数 |
    ///
    /// **`ui_source` を逆に言う変異は、1本目と3本目の対比で赤になる。**
    /// **警告の文面も出どころで分かれる**ので、取り違えもここで赤になる。
    ///
    /// # ⚠ 埋め込みの件数は、実物の `web/dist` から作る
    ///
    /// **埋め込みはコンパイル時に決まるのでテストから植えられない。**
    /// 期待値を埋め込みの表そのものから作ると**式で同値**になる（CLAUDE.md `B-7`）ので、
    /// **`web/dist` を実際に数えて突き合わせる**——
    /// **埋め込み元が別の場所を向く変異は、ここでも赤くなる。**
    /// **`pnpm build` していなければ両方 0 になり、そのときこの腕は
    /// 「0 と 0 が合う」しか言わない**（それでも `ui_source` の腕は生きている）。
    #[test]
    fn the_startup_line_says_where_the_ui_came_from() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");

        let present = dir.path().join("present-web-dist");
        std::fs::create_dir(&present).expect("配り先を作れること");
        std::fs::write(present.join("index.html"), "<!doctype html>").expect("書けること");
        std::fs::write(present.join("app.js"), "//").expect("書けること");
        let missing = dir.path().join("missing-web-dist");

        // **前提**——片方だけが実在すること（両方同じだと対比が潰れる）。
        assert!(present.is_dir(), "前提: 実在する側は作ってあること");
        assert!(!missing.exists(), "前提: 実在しない側は無いこと");

        // **実物の `web/dist` を数える**（埋め込みの表からは作らない）。
        let embedded_origin = Path::new(DEFAULT_WEB_DIR).display().to_string();
        let dist = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("web")
            .join("dist");
        let dist_files = count_files_under(&dist);

        /// 起動ログについて、1つの腕が言うはずのこと。
        struct Arm<'a> {
            /// `PERSONACORE_WEB_DIR` に与える値（`None` なら未設定＝埋め込み）。
            web_dir: Option<&'a Path>,
            want_source: &'a str,
            want_origin: String,
            want_files: usize,
            /// 出るはずの警告の文面（`None` なら黙るはず）。
            want_warning: Option<&'a str>,
        }

        let arms = [
            Arm {
                web_dir: Some(&present),
                want_source: "disk",
                want_origin: present.display().to_string(),
                want_files: 2,
                want_warning: None,
            },
            Arm {
                web_dir: Some(&missing),
                want_source: "disk",
                want_origin: missing.display().to_string(),
                want_files: 0,
                want_warning: Some(WEB_DIR_MISSING_MESSAGE),
            },
            Arm {
                web_dir: None,
                want_source: "embedded",
                want_origin: embedded_origin,
                want_files: dist_files,
                want_warning: (dist_files == 0).then_some(UI_EMBED_EMPTY_MESSAGE),
            },
        ];

        for Arm {
            web_dir,
            want_source,
            want_origin,
            want_files,
            want_warning,
        } in arms
        {
            let web_dir_value = web_dir.map(|path| path.display().to_string());
            let config = AppConfig::from_lookup(|key| {
                (key == ENV_WEB_DIR)
                    .then(|| web_dir_value.clone())
                    .flatten()
            })
            .expect("設定を組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(dir.path().join("startup-web.sqlite"));

            let (output, _) = captured_startup(&config);
            let fields = fields_of(&output, STARTUP_MESSAGE);

            assert!(
                fields.contains(&("ui_source".to_owned(), want_source.to_owned())),
                "画面の出どころが違う（期待 {want_source}）: {fields:?} / {output:?}"
            );
            // **どこを見て言っているのか**（値だけでなく帰属を見る）。
            assert!(
                fields.contains(&("ui_origin".to_owned(), want_origin.clone())),
                "出どころの綴りが違う（期待 {want_origin}）: {fields:?} / {output:?}"
            );
            assert!(
                fields.contains(&("ui_files".to_owned(), want_files.to_string())),
                "配れる件数が違う（期待 {want_files}）: {fields:?} / {output:?}"
            );

            // **1件も無いときだけ声を上げる。** あるときに出ると狼少年になる。
            // **文面は出どころで違う**（直す手が違う）ので、両方について見る。
            for message in [WEB_DIR_MISSING_MESSAGE, UI_EMBED_EMPTY_MESSAGE] {
                assert_eq!(
                    output.contains(message),
                    want_warning == Some(message),
                    "警告の有無が違う（{want_source} / 期待 {want_warning:?}）: {output:?}"
                );
            }
            if let Some(message) = want_warning {
                // **行を選ぶのは共有の1本**（作業単位 B-36。上の [`fields_of`] と同じ理由）。
                let warning = testlog::line_containing(&output, message);
                assert!(
                    warning.contains(&want_origin),
                    "警告にも出どころを載せること（起動行を遡らずに直せる）: {warning}"
                );
            }
        }
    }

    /// **注入の入口も、器の「明示か」を事実どおりに付ける**（作業単位 A-13a-1 のレビュー是正）。
    ///
    /// **3つの腕**（明示なし・環境変数だけ・環境変数とプロファイルの両方）。**値はどれも違える**
    /// ——「常に measured」「常に explicit」「抽出に環境変数を写す」のどれも、どこかの腕で赤になる。
    /// **抽出の明示は面の値だけ**（環境変数は抽出を止めない）。
    #[tokio::test]
    async fn the_injection_entry_marks_explicit_values_as_explicit() {
        let dir = tempfile::tempdir().expect("一時ディレクトリ");
        let profiles = upstreams::parse_profiles(
            r#"{ "p": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "context_tokens": 28672 },
                        "sub":  { "endpoint": "http://127.0.0.1:2", "model": "s", "context_tokens": 10240 } } }"#,
            None,
        )
        .expect("読めること");
        let settings = profiles.select("p").expect("在ること").settings();

        for (arm, environment, profile, want) in [
            ("明示なし", None, false, (None, None)),
            ("環境変数だけ", Some("12288"), false, (Some(12_288), None)),
            ("両方", Some("12288"), true, (Some(28_672), Some(10_240))),
        ] {
            let mut config = AppConfig::from_lookup(|key| {
                (key == ENV_CONTEXT_TOKENS)
                    .then(|| environment.map(str::to_owned))
                    .flatten()
            })
            .expect("設定を組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(dir.path().join(format!("{arm}.sqlite")));
            if profile {
                config = config
                    .with_profile_context(&settings)
                    .expect("載せられること");
            }

            let app = App::bind_with_clients(
                &config,
                Arc::new(extract::SilentUpstream),
                Arc::new(extract::SilentUpstream),
            )
            .await
            .expect("待ち受けられること");
            assert_eq!(
                (
                    app.refresh.shared().explicit_tokens(),
                    app.refresh.sub_context().explicit_tokens()
                ),
                want,
                "{arm}"
            );
            app.serve(std::future::ready(()))
                .await
                .expect("停止まで通ること");
        }
    }

    /// ディレクトリの下のファイルを数える（**テストが自分で数える側**）。
    ///
    /// **`web::count_disk_files` を呼ばない**——呼ぶと、
    /// 数え方を変える変異と一緒に期待値が動く（CLAUDE.md `16b`）。
    fn count_files_under(root: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(root) else {
            return 0;
        };
        entries
            .flatten()
            .map(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    count_files_under(&path)
                } else {
                    1
                }
            })
            .sum()
    }

    // ------------------------------------------------------------------
    // 使用中のポート（作業単位 B-76 / marukei/personacore#13）
    // ------------------------------------------------------------------

    /// **使用中と言うのは、待ち受けの失敗が `AddrInUse` のときだけ。**
    ///
    /// 権限（`PermissionDenied`）や、同じ kind でも待ち受け以外の失敗（`Serve`）には言わない
    /// ——言うと「ポートを変えれば直る」と、違う直し方を案内する。
    #[test]
    fn only_a_bind_failure_of_kind_addr_in_use_is_reported_as_port_in_use() {
        use std::io::{Error, ErrorKind};

        let cases = [
            (
                ServerError::Bind(Error::from(ErrorKind::AddrInUse)),
                true,
                "待ち受け・使用中",
            ),
            (
                ServerError::Bind(Error::from(ErrorKind::PermissionDenied)),
                false,
                "待ち受け・権限",
            ),
            (
                ServerError::Bind(Error::from(ErrorKind::AddrNotAvailable)),
                false,
                "待ち受け・使えないアドレス",
            ),
            (
                ServerError::Serve(Error::from(ErrorKind::AddrInUse)),
                false,
                "実行中の失敗（kind は同じ）",
            ),
            (
                ServerError::DatabaseDir(Error::from(ErrorKind::AddrInUse)),
                false,
                "DB の置き場所（kind は同じ）",
            ),
        ];
        for (err, expected, what) in cases {
            assert_eq!(err.is_port_in_use(), expected, "{what}: {err:?}");
        }
    }

    /// **Windows の `WSAEADDRINUSE`（10048）は `AddrInUse` に写る**（実際に出る os error の番号）。
    #[cfg(windows)]
    #[test]
    fn windows_reports_an_occupied_port_as_addr_in_use() {
        let err = ServerError::Bind(std::io::Error::from_raw_os_error(10048));
        assert!(err.is_port_in_use(), "{err:?}");
    }

    /// **本物の待ち受けを2つ重ねると、使用中と判定される**（この OS の実際の失敗の形）。
    #[tokio::test]
    async fn binding_an_occupied_port_is_reported_as_port_in_use() {
        let held = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("ポートを取れること");
        let addr = held.local_addr().expect("宛先を読めること");
        let source = TcpListener::bind(addr)
            .await
            .expect_err("使用中のポートは開けないこと");
        let err = ServerError::Bind(source);
        assert!(err.is_port_in_use(), "{err:?}");
    }

    /// **文はポートの番号と、変える環境変数の名前を言う。** 期待の綴りは定数から作らない。
    #[test]
    fn the_port_in_use_message_names_the_port_and_the_setting() {
        assert_eq!(
            port_in_use_message(18_093),
            "ポート 18093 は他のプログラムが使っています。\
             環境変数 PERSONACORE_PORT で別のポートを指定して起動し直してください"
        );
    }

    /// **`Display` は定数のまま**（ポートの番号を載せない / CLAUDE.md「エラー型の `Display` は定数文字列」）。
    #[test]
    fn a_port_in_use_keeps_the_constant_display() {
        let err = ServerError::Bind(std::io::Error::from(std::io::ErrorKind::AddrInUse));
        assert_eq!(err.to_string(), "待ち受けアドレスを開けませんでした");
    }
}
