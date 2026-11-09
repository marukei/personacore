//! **上流プロファイル**（[ADR-0042](../../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)
//! §決定1 / 作業単位 F-4b-1）。
//!
//! ```text
//! personacore --upstream qwen-official
//!   ↓ %LOCALAPPDATA%\personacore\upstreams.json（PERSONACORE_UPSTREAMS で上書き可）
//!   { "qwen-official": { "main": { "endpoint": …, "model": …, "gguf": …, "args": […] } } }
//!   ↓
//!   環境変数4つと同じ入口（PERSONACORE_LLM_ENDPOINT / _MAIN_MODEL /
//!                          _SUB_LLM_ENDPOINT / _SUB_MODEL）
//!   ＋ gguf を書いた面だけ `llama serve` を起こす（`super::launch`）
//! ```
//!
//! # ここは**純粋関数**である（2つの例外を除く）
//!
//! **2つ目の例外は [`write_sample_if_missing`]**（作業単位 F-4c）——
//! **初めて叩いた機に見本を書く。在るファイルには1バイトも触らない。**
//!
//! [`parse_profiles`] は文字列と「`~` の展開先」を受け取るだけで、環境にもファイルにも触らない。
//! **ファイルを読むのは [`load_profiles`] だけ**で、そこは読んで [`parse_profiles`] へ渡す。
//! **`~` の展開先を引数にしてある**のは、**実環境のホームに寄りかからずに全規則を測る**ため。
//!
//! # `F-9` の検証規則を、そのまま引き取っている
//!
//! **写した先が無防備なら、写した瞬間に守りが消える**（CLAUDE.md `B-6`）。
//! **`scripts/start.ps1` の `Read-UpstreamFace` が見ていたものを、1つずつここへ移した:**
//!
//! | 規則 | なぜ |
//! |---|---|
//! | **知らない面**（`main` / `sub` 以外）は名前を挙げて止まる | `sub` を `third` と書くと「書いたのに効かない」になり、手掛かりが1つも出ない |
//! | **知らない鍵**も綴りを挙げて止まる | `gguf` を `ggml` と綴ると**「起こす」が静かに消え**、あとで出る「上流に繋がりません」が**別の事実**に読める |
//! | `sub` が無ければ**抽出も本文と同じ上流** | ADR-0039 §決定1 |
//! | **gguf を書いた面の `endpoint` はループバックだけ** | そのホストが `llama serve --host` へ渡る。**外から繋がる待ち受けを作らない**（CLAUDE.md §セキュリティ） |
//! | **gguf の無い面は縛らない** | 待ち受けを作ったのはこちらではない。縛ると「2台目の機に上流を置く」形（ADR-0039 §結果）が書けない |
//! | **`args` に `--host` / `--port` を書かせない**（F-4b-2） | `args` は後ろへ足されるので、後ろが勝つ実装ではループバックの検証が素通りになる（`--host 0.0.0.0`） |
//! | `~` を展開する | 見本は機をまたいで写される |
//! | **同じポートに別の gguf** は止める | `llama serve` は 1 プロセス 1 モデル |
//!
//! **`start.ps1` の後に足した鍵は [`FACE_KEYS`] の `command`（ADR-0042 §決定1）・
//! `context_tokens`（ADR-0054 §決定1）・`api_key`（ADR-0056 §決定1）の3つ。**
//! 「モデルの置き場所が変わる」のと同じ理由で、**実行ファイルの置き場所も変わる。**
//!
//! | 規則（`api_key` / 作業単位 A-13b-1） | なぜ |
//! |---|---|
//! | **値は空でない `0x21`〜`0x7E` の ASCII・4096 バイトまで**（`llm::ApiKey::parse`） | ヘッダに載せられない値を、送る段になって初めて落とさない |
//! | **鍵を書いた面の宛先にユーザー情報（`user:pass@`）を書かせない**（作業単位 A-13b-2） | `Authorization` が2本送られる（ユーザー情報の Basic 認証と、鍵の Bearer） |
//! | **面ごとの値**（`sub` に本文の鍵を貸さない） | 鍵は宛先に付く。別の宛先へ送ると漏れになる |
//! | **失敗の文面に鍵を写さない** | 文面は `Debug` と起動時の印字に出る |
//!
//! **平文の `http://` で別の機へ送る扱いは、読み込みでは決めない**（作業単位 A-13b-2 / ADR-0056 §決定5）。
//! **接続のたびの名前解決**で決めるので、判定は送る側（`llm::LmStudioClient`）にある。
//! ここが読むのは承認（`allow_key_over_plain_http`。真偽値）だけ。
//! A-13b-1 はここで「`https://` かループバックの `http://` だけ」と止めていた。
//!
//! # ⚠ ここは**プロセスを起こさない**
//!
//! 起こすのは [`super::launch`]。**この層が返すのは「起こすことになっているものの記述」**で、
//! **gguf が実在するかも見ていない**（実在確認は起こす直前 / ADR-0042 §決定2）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;

use super::config::{ENV_LLM_ENDPOINT, ENV_MAIN_MODEL, ENV_SUB_LLM_ENDPOINT, ENV_SUB_MODEL};
use crate::llm::{ApiKey, endpoint_for_log, endpoint_has_userinfo};

/// 上流プロファイルの既定のファイル名。**DB と同じディレクトリに置く**（ADR-0042 §決定1）。
///
/// **設定と DB の場所を割らない。** 割ると「どこを見ればよいか」が2つになる。
pub const UPSTREAMS_FILE_NAME: &str = "upstreams.json";

/// プロファイルの中で使える**面**の綴り。**これ以外は名前を挙げて止める。**
pub const FACE_NAMES: [&str; 2] = ["main", "sub"];

/// 面の中で使える鍵。**これ以外は綴りを挙げて止める。**
///
/// **`command` は ADR-0042 §決定1、`context_tokens` は ADR-0054 §決定1 で足した**（それ以外は `F-9` のまま）。
///
/// # ⚠ この一覧は3つの権威を兼ねる
///
/// **読み込みが受け付ける鍵**・**画面の写し（`web/src/lib/upstreamProfiles.ts` の `FACE_KEYS`。並びも）**・
/// **仕様書の閉じた綴り（`ProblemView.key` / `ProfileInvalidDetail.key`）。**
/// 足すと3つが同時に変わるので、**画面の編集（`profile_editor` の `WrittenFace`）が
/// その欄を持つまでは足さないこと**——`context_tokens` は作業単位 A-13a-1 の間だけ
/// 別の定数にしていて、A-13a-2 で画面が追いついてここへ移した。
///
/// **⚠ 綴りは直書きで並べること**（他の定数から組まない）——画面の見張り番
/// （`web/tests/upstream-profiles.test.ts` の「鍵の綴りは `FACE_KEYS` と一致する」）がこの宣言の
/// 文字列リテラルを読む。
///
/// # `api_key` は画面の欄より先に入った（作業単位 A-13b-1 / ADR-0056 §決定1）
///
/// **上の「画面が追いつくまで足さない」の例外。** 画面の編集（`WrittenFace`）は鍵の値を持たず、
/// `api_key_set` / `api_key_hint` だけを返す（ADR-0056 §決定3）。編集の依頼は鍵を
/// 「書いていない＝残す」で読むので、**欄を持たない画面が保存しても鍵は消えない。**
/// 画面の写しの `FACE_KEYS` も並びごと揃えてあり、鍵の欄そのものは作業単位 A-13b-3 で作った（値を出さない欄）。
///
/// # `allow_key_over_plain_http` も画面の欄より先に入った（作業単位 A-13b-2 / ADR-0056 §決定5）
///
/// **`api_key` と同じ例外。** A-13b-2 の間、画面の編集（`WrittenFace`）はこの欄を持たず、編集の依頼にも欄が無かった。
/// **作業単位 A-13b-3 で、一覧は書いた承認を返し、依頼は `true`（承認する）/ `false`（外す）を送れるようになった。**
/// 依頼が送らなければ、保存は**いまのファイルの、同じ名前・同じ面の値を、`endpoint` が変わらないときだけ残す**
/// （`profile_editor` の `FaceEdit::to_value`）ので、**手で書いた承認は、画面から別の欄を直しても消えない。**
/// **宛先を変えた保存では落とす**（承認は宛先に付く —— 持ち越すと、承認していない宛先へ鍵が平文で出る）。
pub const FACE_KEYS: [&str; 8] = [
    "endpoint",
    "model",
    "gguf",
    "args",
    "command",
    "context_tokens",
    "api_key",
    "allow_key_over_plain_http",
];

/// **面に書ける上流への鍵の綴り**（ADR-0056 §決定1 / 作業単位 A-13b-1。[`FACE_KEYS`] の7つ目）。
///
/// **綴りを写さず、[`FACE_KEYS`] から取る**（[`CONTEXT_TOKENS_KEY`] と同じ）。
pub const API_KEY_KEY: &str = FACE_KEYS[6];

/// **鍵を平文の `http://` でローカルネットワークの外へ送る承認**（ADR-0056 §決定5 / 作業単位 A-13b-2。
/// [`FACE_KEYS`] の8つ目）。値は真偽値で、書かなければ偽。
///
/// **綴りを写さず、[`FACE_KEYS`] から取る**（[`CONTEXT_TOKENS_KEY`] と同じ）。
pub const ALLOW_KEY_OVER_PLAIN_HTTP_KEY: &str = FACE_KEYS[7];

/// **面に書けるコンテキスト長の鍵**（ADR-0054 §決定1 / 作業単位 A-13a-1。[`FACE_KEYS`] の6つ目）。
///
/// **`main` の値は本文の予算の明示**（`PERSONACORE_CONTEXT_TOKENS` と同じ意味・同じ検証）、
/// **`sub` の値は抽出の上限の明示。** `sub` 面が無ければ `main` の値を抽出にも使う
/// （[`UpstreamProfile::settings`]）。
///
/// **綴りを写さず、[`FACE_KEYS`] から取る**（2つの綴りがずれる形を作らない）。
pub const CONTEXT_TOKENS_KEY: &str = FACE_KEYS[5];

/// `command` を書かなかったときに起こす実行ファイルの名前。
///
/// **`llama-server` ではない。** LM Studio が内側で立てるのがその綴りなので、
/// **同じ名前にすると「自分の子かどうか」を名前で語る誘惑ができる**
/// （止めるのは控えた PID だけ / ADR-0042 §決定2）。
pub const DEFAULT_COMMAND: &str = "llama";

/// データディレクトリの下の、既定のプロファイルの置き場所。**純粋関数。**
///
/// **DB と同じディレクトリ**（ADR-0041 §決定3 の `database_path_under` と対）。
#[must_use]
pub fn upstreams_path_under(data_dir: &Path) -> PathBuf {
    data_dir
        .join(super::config::APP_DATA_DIR_NAME)
        .join(UPSTREAMS_FILE_NAME)
}

/// **上流プロファイルの見本**（`scripts/upstreams.sample.json` を埋め込んだもの / 作業単位 F-4c）。
///
/// **何も無い機で初めて叩いたとき、[`write_sample_if_missing`] がこれをそのまま書く。**
///
/// # ⚠ 説明は JSON に書かない（司令塔の決定 / Issue origin#23）
///
/// **JSON にコメントは書けず、このパーサは知らない鍵で止まる**（[`FACE_KEYS`]）ので、
/// `_readme` のような説明の鍵は入れられない。**説明は起動ログと
/// `GET /admin/upstream` の `profile_sample` 欄（と、それを出す画面）が言う。**
///
/// # ⚠ 見本の gguf はこの機のもの
///
/// **他の機では「gguf が無い」で止まるのが正しい**（ADR-0042 §決定2 の「起こす前に確かめる」）。
/// **見張り番は `tests/upstream_cli.rs` の
/// `the_sample_names_ggufs_this_machine_may_not_have_and_launches_nothing`。**
pub const SAMPLE_PROFILES: &str = include_str!("../../scripts/upstreams.sample.json");

/// [`write_sample_if_missing`] が何をしたか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleOutcome {
    /// **見本を書いた。** 起動ログと `GET /admin/upstream` がその場所を言う。
    Wrote,
    /// **既に在った。1バイトも触っていない。** 「書いた」と言わない。
    AlreadyThere,
}

/// **見本を書けなかった**（作業単位 F-4c）。
///
/// `Display` は**定数文字列**。理由は `#[source]` 側（[`std::io::Error`] は置き場所を含まない）。
/// **置き場所は呼び出し側（`main`）がログに出す。**
///
/// **起動は止めない**——見本が無くても、上流無しで待ち受けられる（Issue origin#23）。
#[derive(Debug, thiserror::Error)]
#[error("上流プロファイルの見本を書けませんでした")]
pub struct SampleWriteError(#[source] std::io::Error);

/// **プロファイルが無ければ、見本を書く。在れば絶対に触らない**（作業単位 F-4c / Issue origin#23）。
///
/// # 上書きしない仕組み
///
/// **「在るか確かめてから書く」では書かない**——確かめてから書くまでの間に
/// 別の personacore（や利用者のエディタ）が書いた中身を、黙って潰しうる。
/// **`create_new`（在れば `AlreadyExists` で失敗する開き方）の1回で決める。**
///
/// # ⚠ 守れないもの
///
/// **書いている途中で失敗すると、途中までの中身が残る。** 次の起動は「在る」と読んで触らないので、
/// **直すのは利用者の手**になる（`--upstream` を付ければ、読めない理由と置き場所を挙げて止まる）。
/// **一時ファイルから改名する形は採らない**——[`std::fs::rename`] は宛先が在れば上書きする
/// （`fresh` モジュールが実際に踏んだ穴）。
pub fn write_sample_if_missing(path: &Path) -> Result<SampleOutcome, SampleWriteError> {
    use std::io::Write as _;

    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(SampleWriteError)?;
    }
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            return Ok(SampleOutcome::AlreadyThere);
        }
        Err(err) => return Err(SampleWriteError(err)),
    };
    file.write_all(SAMPLE_PROFILES.as_bytes())
        .map_err(SampleWriteError)?;
    Ok(SampleOutcome::Wrote)
}

/// **利用者のホーム。** `~` の展開先。**ここだけが環境に触る。**
///
/// Windows は `USERPROFILE`、それ以外は `HOME`。
/// **依存クレートを足していない**（`os_data_dir` と同じ判断 / ADR-0041 §決定4）。
#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let raw = std::env::var(key).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

// ----------------------------------------------------------------------
// 失敗の理由
// ----------------------------------------------------------------------

/// **読めなかったファイル**（[`ProfileError::Unreadable`] の `#[source]`）。
///
/// # ⚠ ここには置き場所を載せる
///
/// **絶対制約は「エラー*レスポンス*に内部パスを含めない」**であって、
/// **起動時にコンソールへ出す文言のことではない。**
/// **`--upstream` を渡した利用者が最初に知りたいのは「どこを見に行ったか」**である。
/// **この型が HTTP へ出る経路は無い**（`main` が `anyhow` で印字するだけ）。
#[derive(Debug, thiserror::Error)]
#[error("{path}: {source}")]
pub struct ProfileFileUnreadable {
    path: String,
    #[source]
    source: std::io::Error,
}

/// **形が不正**（[`ProfileError::Shape`] の `#[source]`）。
///
/// **綴りをここに持つ**のは、`Display` を定数文字列に保ったまま
/// 「何が駄目だったか」を運用者へ届けるため（`fresh::UnknownArgument` と同じ形）。
///
/// # 理由の種類も持つ（作業単位 F-4b-3 / ADR-0042 §決定8）
///
/// **画面から保存した内容が検証に通らなかったとき、理由を画面に返す**。
/// **`Display`（文面）は利用者が書いた綴りを含む**ので、応答へはそのまま載せない。
/// **応答に載せてよいのは [`ShapeReason`] と、面・鍵の綴り（どれも定数）だけ。**
/// **理由を決めるのは、ここで文面を組んでいるその場所**——応答の側で文面を読み解かない。
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ProfileShape {
    message: String,
    reason: ShapeReason,
    profile: Option<String>,
    face: Option<&'static str>,
    key: Option<&'static str>,
}

impl ProfileShape {
    /// **理由の種類**（定数）。
    #[must_use]
    pub const fn reason(&self) -> ShapeReason {
        self.reason
    }

    /// **どのプロファイルで止まったか**（利用者が書いた名前）。トップレベルの形なら `None`。
    #[must_use]
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// **どの面で止まったか**（[`FACE_NAMES`] のどれか）。面の外なら `None`。
    ///
    /// **知らない面（`third` 等）のときも `None`**——利用者の綴りをここへ持ち上げない。
    #[must_use]
    pub const fn face(&self) -> Option<&'static str> {
        self.face
    }

    /// **どの鍵で止まったか**（[`FACE_KEYS`] のどれか）。鍵の外なら `None`。
    ///
    /// **知らない鍵（`ggml` 等）のときも `None`**（[`Self::face`] と同じ理由）。
    #[must_use]
    pub const fn key(&self) -> Option<&'static str> {
        self.key
    }
}

/// **形が不正な理由の種類**（作業単位 F-4b-3）。**綴りも文面も定数。**
///
/// **足すときは [`Self::ALL`] にも足すこと**——`code` の一意性の見張り番はそれを回す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeReason {
    /// トップレベルがオブジェクトでない。
    TopLevelNotObject,
    /// プロファイルがオブジェクトでない。
    ProfileNotObject,
    /// 知らない面がある（`main` / `sub` 以外）。
    UnknownFace,
    /// `main` が無い。
    MissingMain,
    /// 面がオブジェクトでない。
    FaceNotObject,
    /// 面に知らない鍵がある。
    UnknownKey,
    /// 面に要る鍵（`endpoint` / `model`）が無い。
    MissingKey,
    /// 文字列であるべき値が文字列でない（`args` の要素を含む）。
    NotAString,
    /// `endpoint` が URL として読めない（ポートが無い等）。
    EndpointUnreadable,
    /// gguf を書いた面の `endpoint` がループバックでない。
    EndpointNotLoopback,
    /// `args` が配列でない。
    ArgsNotAList,
    /// `args` に `--host` / `--port` がある。
    ReservedServeFlag,
    /// `command` が空。
    CommandEmpty,
    /// `main` と `sub` が同じポートに別の gguf を載せようとしている。
    SamePortOtherGguf,
    /// `context_tokens` が予算にならない（整数でない・0・応答の余白以下 / ADR-0054 §決定1）。
    ///
    /// **鍵（[`ProfileShape::key`]）は [`CONTEXT_TOKENS_KEY`]**（作業単位 A-13a-2 で
    /// [`FACE_KEYS`] に入った。A-13a-1 の間は `None` だった）——画面はこの鍵で、理由をその欄に結びつける。
    ContextTokensUnusable,
    /// `api_key` が鍵にならない（空・4096 バイト超・`0x21`〜`0x7E` の外の字 / ADR-0056 §決定1）。
    ///
    /// **鍵（[`ProfileShape::key`]）は [`API_KEY_KEY`]。文面に鍵の綴りを写さない**（ADR-0056 §決定3）。
    ApiKeyUnusable,
    /// 鍵を書いた面の宛先にユーザー情報（`http://user:pass@host`）がある（作業単位 A-13b-2）。
    ///
    /// `reqwest` はユーザー情報を Basic 認証の `Authorization` にし、鍵の `Authorization` は足す
    /// ので、**`Authorization` が2本送られる**（上流によっては正しい鍵でも 401 になる）。
    /// **鍵は `endpoint`**（直すのはユーザー情報を外すか、鍵を外すか）。**文面にユーザー情報も鍵も写さない。**
    ///
    /// > A-13b-1 の `api_key_over_plain_http`（`https://` でもループバックの `http://` でもない宛先を
    /// > 読み込みで止める）は、A-13b-2 で外した（送る側が接続のたびに判定する / ADR-0056 §決定5）。
    ApiKeyWithUserinfo,
    /// 真偽値であるべき値が真偽値でない（`allow_key_over_plain_http` / 作業単位 A-13b-2）。
    NotABoolean,
}

impl ShapeReason {
    /// **全部の値。** 見張り番が回すための一覧。
    pub const ALL: [Self; 18] = [
        Self::TopLevelNotObject,
        Self::ProfileNotObject,
        Self::UnknownFace,
        Self::MissingMain,
        Self::FaceNotObject,
        Self::UnknownKey,
        Self::MissingKey,
        Self::NotAString,
        Self::EndpointUnreadable,
        Self::EndpointNotLoopback,
        Self::ArgsNotAList,
        Self::ReservedServeFlag,
        Self::CommandEmpty,
        Self::SamePortOtherGguf,
        Self::ContextTokensUnusable,
        Self::ApiKeyUnusable,
        Self::ApiKeyWithUserinfo,
        Self::NotABoolean,
    ];

    /// **機械が読む綴り**（画面が分岐に使う）。
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::TopLevelNotObject => "top_level_not_object",
            Self::ProfileNotObject => "profile_not_object",
            Self::UnknownFace => "unknown_face",
            Self::MissingMain => "missing_main",
            Self::FaceNotObject => "face_not_object",
            Self::UnknownKey => "unknown_key",
            Self::MissingKey => "missing_key",
            Self::NotAString => "not_a_string",
            Self::EndpointUnreadable => "endpoint_unreadable",
            Self::EndpointNotLoopback => "endpoint_not_loopback",
            Self::ArgsNotAList => "args_not_a_list",
            Self::ReservedServeFlag => "reserved_serve_flag",
            Self::CommandEmpty => "command_empty",
            Self::SamePortOtherGguf => "same_port_other_gguf",
            Self::ContextTokensUnusable => "context_tokens_unusable",
            Self::ApiKeyUnusable => "api_key_unusable",
            Self::ApiKeyWithUserinfo => "api_key_with_userinfo",
            Self::NotABoolean => "not_a_boolean",
        }
    }

    /// **人が読む文面**（画面にそのまま出してよい。**利用者の綴りを含まない**）。
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::TopLevelNotObject => {
                "トップレベルはプロファイル名のオブジェクトである必要があります"
            }
            Self::ProfileNotObject => "プロファイルがオブジェクトではありません",
            Self::UnknownFace => "知らない面があります（使えるのは main / sub だけです）",
            Self::MissingMain => "main がありません（本文の上流は省けません）",
            Self::FaceNotObject => "面がオブジェクトではありません",
            Self::UnknownKey => {
                "知らない鍵があります（使えるのは endpoint / model / gguf / args / command / context_tokens / api_key / allow_key_over_plain_http だけです）"
            }
            Self::MissingKey => "要る鍵がありません",
            Self::NotAString => "文字列であるべき値が文字列ではありません",
            Self::EndpointUnreadable => {
                "endpoint が URL として読めません（http://ホスト:ポート の形で書いてください）"
            }
            Self::EndpointNotLoopback => {
                "gguf を書いた面の endpoint はループバック（127.0.0.1 / localhost / ::1）だけです"
            }
            Self::ArgsNotAList => "args が配列ではありません",
            Self::ReservedServeFlag => {
                "args に --host / --port は書けません（endpoint から作ります）"
            }
            Self::CommandEmpty => "command が空です",
            Self::SamePortOtherGguf => {
                "main と sub が同じポートに別の gguf を載せようとしています（1 プロセス 1 モデルです）"
            }
            Self::ContextTokensUnusable => {
                "context_tokens が使えません（応答の余白より大きい整数のトークン数で書いてください）"
            }
            Self::ApiKeyUnusable => {
                "api_key が使えません（空白・改行・ASCII の外の字を含まない、4096 バイトまでの文字列で書いてください）"
            }
            Self::ApiKeyWithUserinfo => {
                "api_key を書いた面の endpoint にユーザー情報（user:pass@）は書けません（Authorization が2本送られるため）"
            }
            Self::NotABoolean => "真偽値であるべき値が真偽値ではありません",
        }
    }
}

/// **その名前のプロファイルが無い**（[`ProfileError::Unknown`] の `#[source]`）。
///
/// **在る名前を並べる。** 黙って既定へ落ちると、旗を渡した意味が消える。
#[derive(Debug, thiserror::Error)]
#[error("'{wanted}' は在りません。在るのは: {known}")]
pub struct UnknownProfile {
    wanted: String,
    known: String,
}

/// 上流プロファイルを使えない理由。
///
/// `Display` は**定数文字列**。詳細は `#[source]` 側に残る。
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// ファイルが無い／読めない。**黙って既定へ落ちない。**
    #[error("上流プロファイルを読めませんでした")]
    Unreadable(#[source] ProfileFileUnreadable),

    /// JSON として読めない。
    #[error("上流プロファイルが JSON として読めません")]
    Json(#[source] serde_json::Error),

    /// 形が不正（知らない面・知らない鍵・欠けている鍵・ループバックでない宛先など）。
    #[error("上流プロファイルの形が不正です")]
    Shape(#[source] ProfileShape),

    /// その名前のプロファイルが無い。
    #[error("指定された上流プロファイルがありません")]
    Unknown(#[source] UnknownProfile),
}

impl ProfileError {
    /// **形が不正だったときの中身**（作業単位 F-4b-3）。それ以外の理由なら `None`。
    #[must_use]
    pub const fn shape(&self) -> Option<&ProfileShape> {
        match self {
            Self::Shape(shape) => Some(shape),
            Self::Unreadable(_) | Self::Json(_) | Self::Unknown(_) => None,
        }
    }
}

/// **どこで止まったか**（[`ProfileShape`] の定数の側）。
#[derive(Clone, Copy)]
struct Fault<'a> {
    reason: ShapeReason,
    profile: Option<&'a str>,
    face: Option<&'static str>,
    key: Option<&'static str>,
}

impl<'a> Fault<'a> {
    const fn top(reason: ShapeReason) -> Self {
        Self {
            reason,
            profile: None,
            face: None,
            key: None,
        }
    }

    const fn profile(reason: ShapeReason, profile: &'a str) -> Self {
        Self {
            reason,
            profile: Some(profile),
            face: None,
            key: None,
        }
    }

    const fn face(reason: ShapeReason, profile: &'a str, face: &'static str) -> Self {
        Self {
            reason,
            profile: Some(profile),
            face: Some(face),
            key: None,
        }
    }

    const fn key(
        reason: ShapeReason,
        profile: &'a str,
        face: &'static str,
        key: &'static str,
    ) -> Self {
        Self {
            reason,
            profile: Some(profile),
            face: Some(face),
            key: Some(key),
        }
    }
}

fn shape(fault: Fault<'_>, message: impl Into<String>) -> ProfileError {
    ProfileError::Shape(ProfileShape {
        message: message.into(),
        reason: fault.reason,
        profile: fault.profile.map(str::to_owned),
        face: fault.face,
        key: fault.key,
    })
}

// ----------------------------------------------------------------------
// 読み取った形
// ----------------------------------------------------------------------

/// **1つの面のために起こすもの**（gguf を書いた面だけが持つ）。
///
/// **実在確認も起動もここでは行わない**（[`super::launch`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceLaunch {
    command: String,
    gguf: PathBuf,
    serve_host: String,
    serve_port: u16,
    extra_args: Vec<String>,
}

impl FaceLaunch {
    /// 起こす実行ファイル。**既定は [`DEFAULT_COMMAND`]。**
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// 載せる gguf。**`~` は展開済み。実在は確かめていない。**
    #[must_use]
    pub fn gguf(&self) -> &Path {
        &self.gguf
    }

    /// `--host` へ渡すホスト。**ループバックであることは検証済み。**
    #[must_use]
    pub fn serve_host(&self) -> &str {
        &self.serve_host
    }

    /// `--port` へ渡すポート。
    #[must_use]
    pub const fn serve_port(&self) -> u16 {
        self.serve_port
    }

    /// `args` に書かれた追加の引数。**`--host` / `--port` は含まない。**
    #[must_use]
    pub fn extra_args(&self) -> &[String] {
        &self.extra_args
    }
}

/// **1つの面**（`main` = 本文 / `sub` = 抽出・変容）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileFace {
    endpoint: String,
    model: String,
    launch: Option<FaceLaunch>,
    /// **書いたコンテキスト長**（ADR-0054 §決定1）。**検証済み**（`config::context_budget_of`）。
    context_tokens: Option<u32>,
    /// **この面の上流への鍵**（ADR-0056 §決定1）。**検証済み**（`ApiKey::parse`・ユーザー情報の規則）。
    api_key: Option<ApiKey>,
    /// **鍵を平文の `http://` でローカルネットワークの外へ送る承認**（ADR-0056 §決定5 /
    /// [`ALLOW_KEY_OVER_PLAIN_HTTP_KEY`]）。書かなければ偽。
    allow_key_over_plain_http: bool,
}

impl ProfileFace {
    /// **この面に書いた承認**（[`ALLOW_KEY_OVER_PLAIN_HTTP_KEY`] / ADR-0056 §決定5）。書いていなければ偽。
    ///
    /// **効くのは、鍵を書いた `http://` の面で、宛先にローカルネットワークの外が混ざるときだけ**
    /// （判定は送る側 —— `llm::LmStudioClient`）。
    #[must_use]
    pub const fn allow_key_over_plain_http(&self) -> bool {
        self.allow_key_over_plain_http
    }

    /// **この面に書いた上流への鍵**（[`API_KEY_KEY`]）。書いていなければ `None`。
    ///
    /// **面ごとの値**——`sub` 面に書いていなければ、本文の鍵を貸さない（ADR-0056 §決定1）。
    #[must_use]
    pub const fn api_key(&self) -> Option<&ApiKey> {
        self.api_key.as_ref()
    }

    /// **この面に書いたコンテキスト長**（[`CONTEXT_TOKENS_KEY`]）。書いていなければ `None`。
    ///
    /// **`PERSONACORE_CONTEXT_TOKENS` と同じ規則を通っている**（0 と応答の余白以下は読み込みで止まる）。
    /// **`sub` 面を書いていないときに抽出が何を使うかは、ここではなく
    /// [`ProfileSettings::sub_context_tokens`] が言う。**
    #[must_use]
    pub const fn context_tokens(&self) -> Option<u32> {
        self.context_tokens
    }

    /// 宛先。**設定どおりの綴り**（接続に使う。`GET /admin/upstream/profiles` もこの綴りを返す）。
    ///
    /// **外へ出す側が落とす**（作業単位 B-71）—— ログと形の誤りの文面は `llm::endpoint_for_log`、
    /// 上流の観測（`super::upstream`）は認証情報を `***@` に伏せたうえで同じものを落とす。
    /// **ログに出すときは、必ずどちらかを通すこと。**
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// 依頼するモデルの綴り。
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// **この面のために起こすもの。** `None` は「外の上流」。
    #[must_use]
    pub const fn launch(&self) -> Option<&FaceLaunch> {
        self.launch.as_ref()
    }
}

/// **1つの子プロセスと、それが担う面。**
///
/// **同じ宛先に同じ gguf を書いた2つの面は、1つの子が担う**
/// （`llama serve` は 1 プロセス 1 モデル）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileLaunch {
    launch: FaceLaunch,
    endpoint: String,
    faces: Vec<crate::measured::ModelFace>,
}

impl ProfileLaunch {
    /// 起こすものの記述。
    #[must_use]
    pub const fn launch(&self) -> &FaceLaunch {
        &self.launch
    }

    /// その子が答える宛先。
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// **この子が担う面。** 1つとは限らない。
    #[must_use]
    pub fn faces(&self) -> &[crate::measured::ModelFace] {
        &self.faces
    }
}

/// **1つのプロファイル。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamProfile {
    main: ProfileFace,
    sub: Option<ProfileFace>,
    launches: Vec<ProfileLaunch>,
}

impl UpstreamProfile {
    /// 本文の面。**省けない。**
    #[must_use]
    pub const fn main(&self) -> &ProfileFace {
        &self.main
    }

    /// 抽出・変容の面。**`None` は「本文と同じ上流」**（ADR-0039 §決定1）。
    #[must_use]
    pub const fn sub(&self) -> Option<&ProfileFace> {
        self.sub.as_ref()
    }

    /// **起こすもの一式。** 空なら1つも起こさない。
    #[must_use]
    pub fn launches(&self) -> &[ProfileLaunch] {
        &self.launches
    }

    /// **環境変数4つと同じ入口へ流す値**（ADR-0042 §決定1）。
    #[must_use]
    pub fn settings(&self) -> ProfileSettings {
        // **`sub` を省いたら本文と同じ上流**（ADR-0039 §決定1）。
        // **宛先は空文字**——サーバは空を「未設定」と読む（`AppConfig::from_lookup_with_data_dir`）。
        // **モデルの綴りは本文と同じ**。同じプロセスが答えるのだから、それが本当のこと。
        //
        // **コンテキスト長も同じ規則**（ADR-0054 §決定1）——`sub` 面が無ければ、
        // 抽出も本文と同じ上流へ行くので、**窓も同じ**（本文の値を使う）。
        // **`sub` 面があって値が無ければ写さない**——本文と別の上流かもしれない（照会する）。
        //
        // **鍵は窓と違う規則**（ADR-0056 §決定1）——`sub` 面が無ければ写さない（`None`）。
        // そのとき抽出は本文のクライアントの複製（`with_model`）を使うので、**同じ宛先へ同じ鍵が付く**
        // （`App::bind`）。ここで本文の鍵を写すと、それを別の宛先へ渡す形が書けてしまう。
        //
        // **承認は鍵と同じ規則**（ADR-0056 §決定5）——承認は鍵を送る宛先に付くので、
        // `sub` 面が無ければ写さない（抽出は本文のクライアントの複製を使い、同じ承認で送る）。
        let (sub_endpoint, sub_model, sub_context_tokens, sub_api_key, sub_allow) =
            match self.sub.as_ref() {
                Some(sub) => (
                    sub.endpoint.clone(),
                    sub.model.clone(),
                    sub.context_tokens,
                    sub.api_key.clone(),
                    sub.allow_key_over_plain_http,
                ),
                None => (
                    String::new(),
                    self.main.model.clone(),
                    self.main.context_tokens,
                    None,
                    false,
                ),
            };
        ProfileSettings {
            endpoint: self.main.endpoint.clone(),
            model: self.main.model.clone(),
            sub_endpoint,
            sub_model,
            context_tokens: self.main.context_tokens,
            sub_context_tokens,
            api_key: self.main.api_key.clone(),
            sub_api_key,
            allow_key_over_plain_http: self.main.allow_key_over_plain_http,
            sub_allow_key_over_plain_http: sub_allow,
        }
    }
}

/// **環境変数4つぶんの値と、面に書いたコンテキスト長**（ADR-0042 §決定1 / ADR-0054 §決定1）。
///
/// # つまみは2つ増えた（ADR-0054 §決定4 / 作業単位 A-13a-1）
///
/// **F-4b-1 の時点では「つまみは1つも増えていない」**——プロファイルが触るのは
/// 環境変数4つだけだった。**ADR-0054 で本文と抽出のコンテキスト長の2つが増えた**
/// （ADR-0036 §決定3 の3条件に当てはめた結果は ADR-0054 §決定4）。
///
/// **⚠ その2つは [`overlay_profile`] を通らない。** 本文の値を
/// `PERSONACORE_CONTEXT_TOKENS` に重ねると、**起動時の環境変数の値が消える**——
/// 走ったまま「値の無いプロファイル」へ切り替えたときに戻る先（ADR-0054 §決定3）が無くなる。
/// **抽出の値には、そもそも環境変数が無い。** 渡し方は `AppConfig::with_profile_context`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSettings {
    endpoint: String,
    model: String,
    sub_endpoint: String,
    sub_model: String,
    context_tokens: Option<u32>,
    sub_context_tokens: Option<u32>,
    /// **本文の上流への鍵**（ADR-0056 §決定1）。**[`overlay_profile`] を通らない**（環境変数の口を作らない）。
    api_key: Option<ApiKey>,
    /// **抽出の上流への鍵。** `sub` 面に書いたときだけ（本文の鍵を貸さない）。
    sub_api_key: Option<ApiKey>,
    /// **本文の面の承認**（ADR-0056 §決定5）。**[`overlay_profile`] を通らない**（鍵と同じ）。
    allow_key_over_plain_http: bool,
    /// **抽出の面の承認。** `sub` 面に書いたときだけ（本文の承認を貸さない —— 鍵と同じ規則）。
    sub_allow_key_over_plain_http: bool,
}

impl ProfileSettings {
    /// **本文の面の承認**（`main` 面の [`ALLOW_KEY_OVER_PLAIN_HTTP_KEY`] / ADR-0056 §決定5）。
    ///
    /// 渡し方は `AppConfig::with_profile_keys`（鍵と一緒に載せる）。
    #[must_use]
    pub const fn allow_key_over_plain_http(&self) -> bool {
        self.allow_key_over_plain_http
    }

    /// **抽出の面の承認**（`sub` 面の [`ALLOW_KEY_OVER_PLAIN_HTTP_KEY`]）。
    ///
    /// **規則は [`Self::sub_api_key`] と同じ** —— `sub` 面を書いていなければ偽（抽出は本文のクライアントの
    /// 複製を使うので、本文の承認が同じ宛先に効く）。
    #[must_use]
    pub const fn sub_allow_key_over_plain_http(&self) -> bool {
        self.sub_allow_key_over_plain_http
    }

    /// **本文の上流への鍵**（`main` 面の [`API_KEY_KEY`] / ADR-0056 §決定1）。
    ///
    /// 渡し方は `AppConfig::with_profile_keys`（環境変数の側には口が無い / ADR-0056 §決定1）。
    #[must_use]
    pub const fn api_key(&self) -> Option<&ApiKey> {
        self.api_key.as_ref()
    }

    /// **抽出の上流への鍵**（`sub` 面の [`API_KEY_KEY`]）。
    ///
    /// | `sub` 面 | 値 |
    /// |---|---|
    /// | 書いていない | **`None`**（抽出は本文のクライアントの複製を使うので、本文の鍵が同じ宛先に付く） |
    /// | 書いて、鍵を書いた | その鍵 |
    /// | 書いて、鍵を書いていない | `None`（**本文の鍵を貸さない** —— 別の宛先かもしれない） |
    #[must_use]
    pub const fn sub_api_key(&self) -> Option<&ApiKey> {
        self.sub_api_key.as_ref()
    }

    /// 本文の宛先（[`ENV_LLM_ENDPOINT`]）。
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// 本文のモデル（[`ENV_MAIN_MODEL`]）。
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// 抽出の宛先（[`ENV_SUB_LLM_ENDPOINT`]）。**空は「本文と同じ」。**
    #[must_use]
    pub fn sub_endpoint(&self) -> &str {
        &self.sub_endpoint
    }

    /// 抽出のモデル（[`ENV_SUB_MODEL`]）。
    #[must_use]
    pub fn sub_model(&self) -> &str {
        &self.sub_model
    }

    /// **本文の予算の明示**（`main` 面の [`CONTEXT_TOKENS_KEY`] / ADR-0054 §決定1）。
    ///
    /// **`PERSONACORE_CONTEXT_TOKENS` に勝つ**（ADR-0054 §決定2）。`None` なら環境変数を使う。
    #[must_use]
    pub const fn context_tokens(&self) -> Option<u32> {
        self.context_tokens
    }

    /// **抽出の上限の明示**（ADR-0054 §決定1）。
    ///
    /// | `sub` 面 | 値 |
    /// |---|---|
    /// | 書いていない | **`main` 面の値**（同じ上流なので窓も同じ） |
    /// | 書いて、値を書いた | その値 |
    /// | 書いて、値を書いていない | `None`（照会する。本文の値を写さない） |
    #[must_use]
    pub const fn sub_context_tokens(&self) -> Option<u32> {
        self.sub_context_tokens
    }

    /// **その綴りをプロファイルが決めているか。** 決めていなければ `None`。
    #[must_use]
    fn value_for(&self, key: &str) -> Option<&str> {
        if key == ENV_LLM_ENDPOINT {
            Some(&self.endpoint)
        } else if key == ENV_MAIN_MODEL {
            Some(&self.model)
        } else if key == ENV_SUB_LLM_ENDPOINT {
            Some(&self.sub_endpoint)
        } else if key == ENV_SUB_MODEL {
            Some(&self.sub_model)
        } else {
            None
        }
    }
}

/// **プロファイルを環境変数の上に重ねる。純粋関数**（ADR-0042 §決定1）。
///
/// **プロファイルが勝つ。** 旗を渡すこと自体が利用者の判断なので、
/// 環境変数に負けると「指定したはずの上流が効かない」になる（`F-9` と同じ決定）。
/// **ぶつかったことは呼び出し側が `warn` で言う**（黙って上書きしない）。
///
/// **触るのは4つだけ。** それ以外の綴りは、渡された参照関数がそのまま答える
/// ——**ポートも DB も変容のつまみも、`--upstream` では1バイトも動かない。**
///
/// **⚠ コンテキスト長（ADR-0054）もここを通さない**——`PERSONACORE_CONTEXT_TOKENS` に
/// 重ねると起動時の環境変数の値が消える（[`ProfileSettings`] の doc）。
pub fn overlay_profile<F>(settings: &ProfileSettings, base: F) -> impl Fn(&str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    let settings = settings.clone();
    move |key: &str| {
        settings
            .value_for(key)
            .map(str::to_owned)
            .or_else(|| base(key))
    }
}

/// **読み取ったプロファイル一式。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamProfiles {
    profiles: BTreeMap<String, UpstreamProfile>,
}

impl UpstreamProfiles {
    /// 在る名前（**並べ替え済み**）。
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.profiles.keys().map(String::as_str)
    }

    /// 名前で引く。**無ければ、在る名前を並べて止まる。**
    pub fn select(&self, name: &str) -> Result<&UpstreamProfile, ProfileError> {
        self.profiles.get(name).ok_or_else(|| {
            ProfileError::Unknown(UnknownProfile {
                wanted: name.to_owned(),
                known: self.profiles.keys().cloned().collect::<Vec<_>>().join(", "),
            })
        })
    }

    /// **全部のプロファイルの gguf の面が使う番号**（llama を起こす番号 / ADR-0057 §決定3 / 作業単位 F-11b）。
    /// **純粋関数。**
    ///
    /// **選ばれているかを問わない**——覚えた名前で起きた起動（ADR-0055）は上流を後で画面から起こすので、
    /// その起動の計画だけを見ると取り合いを見逃す。**外の上流の面（gguf の無い面）は入らない**
    /// （そこに待ち受けを作るのは personacore ではない / `LaunchPlan::port_conflict` と同じ線）。
    #[must_use]
    pub fn launch_ports(&self) -> BTreeSet<u16> {
        self.profiles
            .values()
            .flat_map(|profile| profile.launches.iter())
            .map(|launch| launch.launch().serve_port())
            .collect()
    }
}

// ----------------------------------------------------------------------
// 読み取り
// ----------------------------------------------------------------------

/// **待ち受けを探すときに飛ばす番号を、置き場所のファイルから読む**（ADR-0057 §決定3 / 作業単位 F-11b）。
///
/// **ファイルが無ければ空**（`Ok`。見本を書けなかった機・`PERSONACORE_UPSTREAMS` が無いファイルを指す起動）。
/// **読めない・形が不正なら `Err`**——`main` は警告を1本出して、飛ばす番号を空にして進む
/// （**上流を決めるのには使わない**。読んだ中身はここで番号だけになる）。
///
/// # Errors
///
/// 在るのに読めない、または JSON・形として読めないとき（[`load_profiles`] と同じ理由）。
pub fn launch_ports_at(path: &Path) -> Result<BTreeSet<u16>, ProfileError> {
    match std::fs::symlink_metadata(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        _ => load_profiles(path).map(|profiles| profiles.launch_ports()),
    }
}

/// **ファイルから読む。** 読めなければ置き場所を挙げて止まる。
///
/// **`~` の展開先は [`home_dir`] から引く**（ここだけが環境に触る）。
pub fn load_profiles(path: &Path) -> Result<UpstreamProfiles, ProfileError> {
    let source = std::fs::read_to_string(path).map_err(|source| {
        ProfileError::Unreadable(ProfileFileUnreadable {
            path: path.display().to_string(),
            source,
        })
    })?;
    parse_profiles(&source, home_dir().as_deref())
}

/// **JSON を読んで、形を確かめた入れ物に直す。純粋関数。**
///
/// `home` は `~` の展開先。**`None` なら `~` を展開しない**
/// （展開できない環境で黙って相対パスにしないよう、そのままの綴りで残す）。
pub fn parse_profiles(source: &str, home: Option<&Path>) -> Result<UpstreamProfiles, ProfileError> {
    let root: JsonValue = serde_json::from_str(source).map_err(ProfileError::Json)?;
    let object = root.as_object().ok_or_else(|| {
        shape(
            Fault::top(ShapeReason::TopLevelNotObject),
            "トップレベルはプロファイル名のオブジェクトである必要があります",
        )
    })?;

    let mut profiles = BTreeMap::new();
    for (name, body) in object {
        profiles.insert(name.clone(), parse_profile(name, body, home)?);
    }
    Ok(UpstreamProfiles { profiles })
}

fn parse_profile(
    name: &str,
    body: &JsonValue,
    home: Option<&Path>,
) -> Result<UpstreamProfile, ProfileError> {
    let object = body.as_object().ok_or_else(|| {
        shape(
            Fault::profile(ShapeReason::ProfileNotObject, name),
            format!("プロファイル '{name}' がオブジェクトではありません"),
        )
    })?;

    // **知らない面を黙って無視しない。** `sub` を `third` と書いても
    // 「書いたのに効かない」になり、手掛かりが1つも出ない。
    let unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !FACE_NAMES.contains(key))
        .collect();
    if !unknown.is_empty() {
        return Err(shape(
            Fault::profile(ShapeReason::UnknownFace, name),
            format!(
                "プロファイル '{name}' に知らない面があります: {}（使えるのは {} だけです）",
                unknown.join(", "),
                FACE_NAMES.join(" / ")
            ),
        ));
    }

    let main = object.get("main").ok_or_else(|| {
        shape(
            Fault::profile(ShapeReason::MissingMain, name),
            format!("プロファイル '{name}' に main がありません（本文の上流は省けません）"),
        )
    })?;
    let main = parse_face(name, "main", main, home)?;
    let sub = match object.get("sub") {
        Some(face) => Some(parse_face(name, "sub", face, home)?),
        None => None,
    };

    let launches = group_launches(name, &main, sub.as_ref())?;
    Ok(UpstreamProfile {
        main,
        sub,
        launches,
    })
}

fn parse_face(
    profile: &str,
    label: &'static str,
    body: &JsonValue,
    home: Option<&Path>,
) -> Result<ProfileFace, ProfileError> {
    let object = body.as_object().ok_or_else(|| {
        shape(
            Fault::face(ShapeReason::FaceNotObject, profile, label),
            format!("プロファイル '{profile}' の {label} がオブジェクトではありません"),
        )
    })?;

    // **知らない鍵を黙って捨てない。** `gguf` を `ggml` と綴ると
    // 「起こす」が静かに消え、あとで出る言葉が別の事実に読める。
    let unknown: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !FACE_KEYS.contains(key))
        .collect();
    if !unknown.is_empty() {
        return Err(shape(
            Fault::face(ShapeReason::UnknownKey, profile, label),
            format!(
                "プロファイル '{profile}' の {label} に知らない鍵があります: {}（使えるのは {} だけです）",
                unknown.join(", "),
                FACE_KEYS.join(" / ")
            ),
        ));
    }

    // **コンテキスト長**（ADR-0054 §決定1）。**`PERSONACORE_CONTEXT_TOKENS` と同じ規則**
    // （`config::context_budget_of`）——規則の写しをここに作らない。
    // **gguf の有無に関わらず書ける**（窓を答えない外の上流こそ、書く理由がある）。
    let context_tokens = match object.get(CONTEXT_TOKENS_KEY) {
        None => None,
        Some(raw) => {
            let usable = raw
                .as_u64()
                .and_then(|tokens| u32::try_from(tokens).ok())
                .filter(|tokens| super::config::context_budget_of(*tokens).is_ok());
            match usable {
                Some(tokens) => Some(tokens),
                None => {
                    return Err(shape(
                        Fault::key(
                            ShapeReason::ContextTokensUnusable,
                            profile,
                            label,
                            CONTEXT_TOKENS_KEY,
                        ),
                        format!(
                            "プロファイル '{profile}' の {label} の {CONTEXT_TOKENS_KEY} が使えません\
                             （応答の余白 {} より大きい整数のトークン数で書いてください）: {raw}",
                            crate::prompt::TokenBudget::DEFAULT_RESPONSE_TOKENS
                        ),
                    ));
                }
            }
        }
    };

    let text = |key: &'static str| -> Result<String, ProfileError> {
        let value = object.get(key).ok_or_else(|| {
            shape(
                Fault::key(ShapeReason::MissingKey, profile, label, key),
                format!("プロファイル '{profile}' の {label} に {key} がありません"),
            )
        })?;
        let value = value.as_str().ok_or_else(|| {
            shape(
                Fault::key(ShapeReason::NotAString, profile, label, key),
                format!("プロファイル '{profile}' の {label} の {key} が文字列ではありません"),
            )
        })?;
        Ok(value.trim().to_owned())
    };

    let endpoint = text("endpoint")?;
    let model = text("model")?;

    let (host, port) = host_and_port(&endpoint).ok_or_else(|| {
        shape(
            Fault::key(ShapeReason::EndpointUnreadable, profile, label, "endpoint"),
            format!(
                "プロファイル '{profile}' の {label} の endpoint が URL として読めません: {}",
                // **文面はログと標準エラーへ出る**ので、宛先の秘密を落とす（作業単位 B-71 / `llm::endpoint_for_log`）。
                endpoint_for_log(&endpoint)
            ),
        )
    })?;

    let api_key = parse_api_key(object, profile, label, &endpoint)?;

    // **承認**（ADR-0056 §決定5 / 作業単位 A-13b-2）。**真偽値だけ**——`"true"` の文字列を黙って
    // 偽と読むと、書いたのに効かない承認になる。**鍵や宛先との組は見ない**（効くかは送る側が決める）。
    let allow_key_over_plain_http = match object.get(ALLOW_KEY_OVER_PLAIN_HTTP_KEY) {
        None => false,
        Some(JsonValue::Bool(allowed)) => *allowed,
        Some(_) => {
            return Err(shape(
                Fault::key(
                    ShapeReason::NotABoolean,
                    profile,
                    label,
                    ALLOW_KEY_OVER_PLAIN_HTTP_KEY,
                ),
                format!(
                    "プロファイル '{profile}' の {label} の {ALLOW_KEY_OVER_PLAIN_HTTP_KEY} が\
                     真偽値（true / false）ではありません"
                ),
            ));
        }
    };

    let launch = match object.get("gguf") {
        None => None,
        Some(raw) => {
            let raw = raw.as_str().ok_or_else(|| {
                shape(
                    Fault::key(ShapeReason::NotAString, profile, label, "gguf"),
                    format!("プロファイル '{profile}' の {label} の gguf が文字列ではありません"),
                )
            })?;

            // **gguf を書いた面だけ、宛先をループバックに縛る。**
            // このホストがそのまま `llama serve --host` へ渡る ——
            // ループバック以外を書くと、**外から繋がる待ち受けを、この実行ファイルが作る。**
            if !is_loopback(&host) {
                return Err(shape(
                    Fault::key(ShapeReason::EndpointNotLoopback, profile, label, "endpoint"),
                    format!(
                        "プロファイル '{profile}' の {label} は gguf を書いているのに\
                     宛先がループバックではありません（--host に渡すため、\
                     外から繋がる待ち受けになります）: {}",
                        endpoint_for_log(&endpoint)
                    ),
                ));
            }

            let mut extra_args = Vec::new();
            if let Some(args) = object.get("args") {
                let list = args.as_array().ok_or_else(|| {
                    shape(
                        Fault::key(ShapeReason::ArgsNotAList, profile, label, "args"),
                        format!("プロファイル '{profile}' の {label} の args が配列ではありません"),
                    )
                })?;
                for item in list {
                    let item = item.as_str().ok_or_else(|| {
                        shape(Fault::key(ShapeReason::NotAString, profile, label, "args"), format!(
                            "プロファイル '{profile}' の {label} の args に文字列でない要素があります"
                        ))
                    })?;
                    // **`--host` / `--port` は `endpoint` から作る**（作業単位 F-4b-2 / Issue origin#2）。
                    // `args` は `--host <h> --port <p>` の**後ろ**へ足されるので、
                    // 同じ旗を書くと**後ろが勝つ実装では上のループバックの検証が素通りになる**
                    // （`"--host", "0.0.0.0"` で、外から繋がる待ち受けをこの実行ファイルが作る）。
                    if let Some(flag) = reserved_serve_flag(item) {
                        return Err(shape(
                            Fault::key(ShapeReason::ReservedServeFlag, profile, label, "args"),
                            format!(
                                "プロファイル '{profile}' の {label} の args に {flag} があります\
                             （{flag} は endpoint から作ります。endpoint を書き換えてください）: {item}"
                            ),
                        ));
                    }
                    extra_args.push(item.to_owned());
                }
            }

            let command = match object.get("command") {
                Some(value) => {
                    let value = value.as_str().ok_or_else(|| {
                        shape(Fault::key(ShapeReason::NotAString, profile, label, "command"), format!(
                            "プロファイル '{profile}' の {label} の command が文字列ではありません"
                        ))
                    })?;
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        return Err(shape(
                            Fault::key(ShapeReason::CommandEmpty, profile, label, "command"),
                            format!("プロファイル '{profile}' の {label} の command が空です"),
                        ));
                    }
                    trimmed.to_owned()
                }
                None => DEFAULT_COMMAND.to_owned(),
            };

            Some(FaceLaunch {
                command,
                gguf: expand_home(raw.trim(), home),
                serve_host: host,
                serve_port: port,
                extra_args,
            })
        }
    };

    Ok(ProfileFace {
        endpoint,
        model,
        launch,
        context_tokens,
        api_key,
        allow_key_over_plain_http,
    })
}

/// **面の鍵を読む**（ADR-0056 §決定1・5 / 作業単位 A-13b-1・A-13b-2）。書いていなければ `None`。
///
/// | 止める | 理由 |
/// |---|---|
/// | 文字列でない | [`ShapeReason::NotAString`] |
/// | 鍵にならない（`ApiKey::parse` が断る） | [`ShapeReason::ApiKeyUnusable`] |
/// | 宛先にユーザー情報（`user:pass@`）がある | [`ShapeReason::ApiKeyWithUserinfo`]（`Authorization` が2本送られる） |
///
/// # ⚠ 文面に鍵の綴りを写さない（ADR-0056 §決定3）
///
/// 文面（[`ProfileShape`] の `message`）は `Debug` と起動時のエラーの印字に出る。
/// **[`ShapeReason::ContextTokensUnusable`] の文面が値を写しているのと同じ形にしないこと。**
/// **ユーザー情報で止めるときは、宛先も写さない**（宛先そのものがユーザー情報を含む）。
/// 見張り番は `tests::the_key_never_appears_in_a_profile_error_or_debug_form`。
///
/// # ⚠ 平文の `http://` で別の機へ送るかは、ここで決めない（作業単位 A-13b-2）
///
/// **A-13b-1 は「`https://` かループバックの `http://` だけ」をここで止めていた**（`api_key_over_plain_http`）。
/// A-13b-2 で、**送る側が接続のたびの名前解決で判定する**形（`llm::LmStudioClient`）に置き換えた ——
/// 読み込みのときだけ判定すると、後で解決先が変わったときに承認なしで外へ出る（ADR-0056 §決定5）。
///
/// # ⚠ ユーザー情報は送信と同じパーサで読む（ADR-0056 追記5 と同じ理由）
///
/// **[`host_and_port`] を使わない。** あちらは権限部を `@` で割るだけなので、
/// `http://203.0.113.5\@127.0.0.1:46001` にユーザー情報があると読むが、送信（`reqwest` / WHATWG）は
/// `\` を `/` と読むので、ユーザー情報は無い（Basic 認証は付かない）。判定は `llm::endpoint_has_userinfo`。
fn parse_api_key(
    object: &serde_json::Map<String, JsonValue>,
    profile: &str,
    label: &'static str,
    endpoint: &str,
) -> Result<Option<ApiKey>, ProfileError> {
    let Some(raw) = object.get(API_KEY_KEY) else {
        return Ok(None);
    };
    let raw = raw.as_str().ok_or_else(|| {
        shape(
            Fault::key(ShapeReason::NotAString, profile, label, API_KEY_KEY),
            format!("プロファイル '{profile}' の {label} の {API_KEY_KEY} が文字列ではありません"),
        )
    })?;
    let key = ApiKey::parse(raw).map_err(|reason| {
        shape(
            Fault::key(ShapeReason::ApiKeyUnusable, profile, label, API_KEY_KEY),
            // **理由は定数の文面**（`ApiKeyError` の `Display`）。鍵の綴りは載せない。
            format!("プロファイル '{profile}' の {label} の {API_KEY_KEY} が使えません: {reason}"),
        )
    })?;
    if endpoint_has_userinfo(endpoint) {
        return Err(shape(
            Fault::key(ShapeReason::ApiKeyWithUserinfo, profile, label, "endpoint"),
            // **宛先を写さない**（ユーザー情報そのものを含む）。
            format!(
                "プロファイル '{profile}' の {label} は {API_KEY_KEY} を書いているのに、\
                 endpoint にユーザー情報（user:pass@）があります\
                 （Authorization が2本送られます。どちらかを外してください）"
            ),
        ));
    }
    Ok(Some(key))
}

/// **起こすものを束ねる。**
///
/// * **同じ宛先に同じ gguf** → 1つの子が2つの面を担う（1 プロセス 1 モデル）。
/// * **同じポートに別の gguf** → 止める。2つめは黙って失敗するか、
///   1つめが両方の面に答える（`llama serve` は `model` の欄を見ない / `G-1`）。
///   どちらも「設定したつもりで別のモデルが答えている」という、いちばん見えない形になる。
fn group_launches(
    profile: &str,
    main: &ProfileFace,
    sub: Option<&ProfileFace>,
) -> Result<Vec<ProfileLaunch>, ProfileError> {
    use crate::measured::ModelFace;

    let mut launches: Vec<ProfileLaunch> = Vec::new();
    let candidates = [(ModelFace::Main, Some(main)), (ModelFace::Sub, sub)];

    for (face, spec) in candidates {
        let Some(spec) = spec else { continue };
        let Some(launch) = spec.launch.as_ref() else {
            continue;
        };

        if let Some(existing) = launches
            .iter_mut()
            .find(|found| found.launch.serve_port == launch.serve_port)
        {
            if existing.launch != *launch {
                return Err(shape(
                    Fault::profile(ShapeReason::SamePortOtherGguf, profile),
                    format!(
                        "プロファイル '{profile}' の main と sub が、\
                     同じポート {} に別の gguf を載せようとしています（1 プロセス 1 モデルです）",
                        launch.serve_port
                    ),
                ));
            }
            existing.faces.push(face);
            continue;
        }

        // **`sub` を書いていないプロファイルでは、抽出も本文と同じ上流**
        // （ADR-0039 §決定1）——**つまり、この子が両方の面に答える。**
        //
        // **⚠ ここで `Sub` を入れ忘れると、起動時に抽出の面だけが
        // まだ載っていない上流へ聞きに行く**（ADR-0042 §決定3 が避けたい形）。
        // **しかも本文の面は、宛先が同じなら抽出側の一覧を借りる**
        // （`server::App::bind` の表）ので、**借りた先で `Drift::Absent` が並び直す**
        // ——**聞かない決定が丸ごと無効になる**（2026-09-19 に実測 / `m11` の是正）。
        let faces = if face == ModelFace::Main && sub.is_none() {
            vec![ModelFace::Main, ModelFace::Sub]
        } else {
            vec![face]
        };

        launches.push(ProfileLaunch {
            launch: launch.clone(),
            endpoint: spec.endpoint.clone(),
            faces,
        });
    }

    Ok(launches)
}

// ----------------------------------------------------------------------
// 綴りの道具（**すべて純粋関数**）
// ----------------------------------------------------------------------

/// 先頭の `~` を渡されたホームへ展開する。**純粋関数。**
///
/// **`home` が `None` なら展開しない。** 展開できない環境で黙って
/// `~/…` を相対パスとして扱うと、**在りもしない場所を「無い」と言う。**
fn expand_home(raw: &str, home: Option<&Path>) -> PathBuf {
    let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) else {
        return PathBuf::from(raw);
    };
    match home {
        // **区切りは両方見る**（見本は `/` で書くが、写した機は `\` を使う）。
        Some(home) => rest
            .split(['/', '\\'])
            .filter(|part| !part.is_empty())
            .fold(home.to_path_buf(), |path, part| path.join(part)),
        None => PathBuf::from(raw),
    }
}

/// `http://ホスト:ポート/…` から `(ホスト, ポート)` を取り出す。**純粋関数。**
///
/// **ポートを省いた綴りは読めない**（この家の上流はどれもポートを名乗る）。
/// **`[::1]:8080` の形も読む**（IPv6 のリテラルは角括弧に入る / RFC 3986）。
/// **認証情報は権限部から落とす**（ホストの判定に混ぜない）。
fn host_and_port(endpoint: &str) -> Option<(String, u16)> {
    let after_scheme = endpoint.split_once("://")?.1;
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    // **区切りは最後の `@`**——ホストに `@` は現れない。
    let authority = match authority.rfind('@') {
        Some(at) => &authority[at + 1..],
        None => authority,
    };

    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let close = rest.find(']')?;
        let port = rest[close + 1..].strip_prefix(':')?;
        (rest[..close].to_owned(), port)
    } else {
        let (host, port) = authority.rsplit_once(':')?;
        (host.to_owned(), port)
    };

    if host.is_empty() {
        return None;
    }
    Some((host, port.parse().ok()?))
}

/// `endpoint` から作って `llama serve` へ渡す旗（**`args` には書かせない**）。
const RESERVED_SERVE_FLAGS: [&str; 2] = ["--host", "--port"];

/// **その引数は `endpoint` から作る旗か。純粋関数**（作業単位 F-4b-2 / Issue origin#2）。
///
/// `--host` と `--host=…` の両方を見る。**値は読まない**——ループバックの値でも止める。
/// 値を読んで許す形にすると、「どの綴りなら安全か」の判定がここにもう1つ生える
/// （その判定は `endpoint` の側の [`is_loopback`] に1つあれば足りる）。
///
/// **⚠ 旗の値として現れる `--host` も止める。** どの旗が値を取るかをここは知らない
/// （知ろうとすると `llama serve` の引数の形に依存する / ADR-0042 §結果）。
fn reserved_serve_flag(arg: &str) -> Option<&'static str> {
    RESERVED_SERVE_FLAGS.into_iter().find(|flag| {
        arg == *flag
            || arg
                .strip_prefix(flag)
                .is_some_and(|rest| rest.starts_with('='))
    })
}

/// **そのホストはループバックか。純粋関数。**
///
/// **綴りで判定する。名前解決はしない**——解決に行くと、
/// **起動時にネットワークへ出る**うえ、答えが機と DNS に依存する。
fn is_loopback(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(v4) = host.parse::<std::net::Ipv4Addr>() {
        return v4.is_loopback();
    }
    if let Ok(v6) = host.parse::<std::net::Ipv6Addr>() {
        return v6.is_loopback();
    }
    false
}

/// **1つの子が担う面を、1つの綴りにする**（`super::launch` がログの欄とファイル名に使う）。
///
/// # ⚠ 区切りは [`FACE_LABEL_SEPARATOR`]。`/` を使わないこと
///
/// **この綴りは、上流の出力の落とし先のファイル名にそのまま入る。**
/// **`/` を使うと、`main/sub-….log` が「`main` というディレクトリの下」になり、
/// `File::create` が失敗して起動そのものが止まる**
/// （2026-09-19 に実測 —— `sub` を書いていないプロファイルで、
/// **待ち受けが一度も開かなかった**）。
pub(crate) fn face_labels(faces: &[crate::measured::ModelFace]) -> String {
    faces
        .iter()
        .map(|face| face.label())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(FACE_LABEL_SEPARATOR)
}

/// 面の綴りを繋ぐ字。**ファイル名に使える字だけ**（このモジュールの [`face_labels`] の doc）。
pub(crate) const FACE_LABEL_SEPARATOR: &str = "+";

#[cfg(test)]
mod tests {
    use super::*;

    /// **飛ばす番号は、全部のプロファイルの gguf の面の番号**（ADR-0057 §決定3 / 作業単位 F-11b）。
    ///
    /// 3つのプロファイルに、本文の gguf（`a`）・抽出だけの gguf（`b`）・外の上流だけ（`c`）を置く。
    /// **外の上流の番号（`c` と `b` の本文）は入らない。** 同じ番号を2つのプロファイルが使っても1つ。
    #[test]
    fn the_launch_ports_are_every_gguf_face_of_every_profile() {
        let profiles = parse_profiles(
            r#"{
              "a": { "main": { "endpoint": "http://127.0.0.1:47101", "model": "m", "gguf": "/x/a.gguf" } },
              "b": {
                "main": { "endpoint": "http://127.0.0.1:47102", "model": "m" },
                "sub":  { "endpoint": "http://localhost:47103", "model": "s", "gguf": "/x/b.gguf" }
              },
              "c": { "main": { "endpoint": "http://127.0.0.1:47104", "model": "m" } },
              "d": { "main": { "endpoint": "http://127.0.0.1:47101", "model": "m", "gguf": "/x/d.gguf" } }
            }"#,
            None,
        )
        .expect("読めること");
        assert_eq!(profiles.launch_ports(), BTreeSet::from([47101, 47103]));
    }

    /// **置き場所から読む**: 無ければ空、在れば番号、壊れていれば `Err`（`main` が警告にする）。
    #[test]
    fn the_launch_ports_at_a_path_are_empty_when_missing_and_an_error_when_broken() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let path = dir.path().join("upstreams.json");
        assert_eq!(
            launch_ports_at(&path).expect("無ければ空であること"),
            BTreeSet::new()
        );

        std::fs::write(
            &path,
            r#"{ "a": { "main": { "endpoint": "http://127.0.0.1:47201", "model": "m", "gguf": "/x/a.gguf" } } }"#,
        )
        .expect("書けること");
        assert_eq!(
            launch_ports_at(&path).expect("読めること"),
            BTreeSet::from([47201])
        );

        std::fs::write(&path, "{ not json").expect("書けること");
        assert!(matches!(launch_ports_at(&path), Err(ProfileError::Json(_))));

        std::fs::remove_file(&path).expect("消せること");
        std::fs::create_dir(&path).expect("ディレクトリを置けること");
        assert!(matches!(
            launch_ports_at(&path),
            Err(ProfileError::Unreadable(_))
        ));
    }

    /// **形が不正なら、どこで何が駄目だったかを定数で持つ**（作業単位 F-4b-3）。
    ///
    /// **1行ごとに、止まる場所と種類を固定する。** 面と鍵は**2つの値を違える**
    /// （`main` と `sub`、`endpoint` と `args` 等）——同値だと、取り違えが観測できない。
    /// **名前も `a` / `b` の2つを置き、止まる側を `b` にする**（先に読まれる `a` の名前を
    /// 持ち上げる取り違えと区別する）。
    #[test]
    fn a_malformed_profile_names_its_reason_face_and_key() {
        /// 本文・理由・プロファイル・面・鍵。
        type Case = (
            &'static str,
            ShapeReason,
            Option<&'static str>,
            Option<&'static str>,
            Option<&'static str>,
        );
        let ok = r#""a": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m" } }"#;
        let cases: [Case; 21] = [
            ("[1]", ShapeReason::TopLevelNotObject, None, None, None),
            (
                r#""b": 1"#,
                ShapeReason::ProfileNotObject,
                Some("b"),
                None,
                None,
            ),
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m" }, "third": {} }"#,
                ShapeReason::UnknownFace,
                Some("b"),
                None,
                None,
            ),
            (
                r#""b": { "sub": { "endpoint": "http://127.0.0.1:1", "model": "m" } }"#,
                ShapeReason::MissingMain,
                Some("b"),
                None,
                None,
            ),
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m" }, "sub": 3 }"#,
                ShapeReason::FaceNotObject,
                Some("b"),
                Some("sub"),
                None,
            ),
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "ggml": "x" } }"#,
                ShapeReason::UnknownKey,
                Some("b"),
                Some("main"),
                None,
            ),
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m" }, "sub": { "endpoint": "http://127.0.0.1:2" } }"#,
                ShapeReason::MissingKey,
                Some("b"),
                Some("sub"),
                Some("model"),
            ),
            (
                r#""b": { "main": { "endpoint": 5, "model": "m" } }"#,
                ShapeReason::NotAString,
                Some("b"),
                Some("main"),
                Some("endpoint"),
            ),
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1", "model": "m" } }"#,
                ShapeReason::EndpointUnreadable,
                Some("b"),
                Some("main"),
                Some("endpoint"),
            ),
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m" }, "sub": { "endpoint": "http://127.0.0.1:2", "model": "s", "gguf": 7 } }"#,
                ShapeReason::NotAString,
                Some("b"),
                Some("sub"),
                Some("gguf"),
            ),
            (
                r#""b": { "main": { "endpoint": "http://192.168.0.2:1", "model": "m", "gguf": "/x.gguf" } }"#,
                ShapeReason::EndpointNotLoopback,
                Some("b"),
                Some("main"),
                Some("endpoint"),
            ),
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m" }, "sub": { "endpoint": "http://127.0.0.1:2", "model": "s", "gguf": "/x.gguf", "args": "--ctx" } }"#,
                ShapeReason::ArgsNotAList,
                Some("b"),
                Some("sub"),
                Some("args"),
            ),
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "gguf": "/x.gguf", "args": [1] } }"#,
                ShapeReason::NotAString,
                Some("b"),
                Some("main"),
                Some("args"),
            ),
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m" }, "sub": { "endpoint": "http://127.0.0.1:2", "model": "s", "gguf": "/x.gguf", "args": ["--port=9"] } }"#,
                ShapeReason::ReservedServeFlag,
                Some("b"),
                Some("sub"),
                Some("args"),
            ),
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "gguf": "/x.gguf", "command": "  " } }"#,
                ShapeReason::CommandEmpty,
                Some("b"),
                Some("main"),
                Some("command"),
            ),
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "gguf": "/x.gguf" }, "sub": { "endpoint": "http://127.0.0.1:1", "model": "s", "gguf": "/y.gguf" } }"#,
                ShapeReason::SamePortOtherGguf,
                Some("b"),
                None,
                None,
            ),
            // **コンテキスト長**（ADR-0054 §決定1）。**止まる側を `sub` にする**（`main` には正しい値）
            // ——面を取り違える実装と、`main` だけを検証する実装を、ここで赤にする。
            // **鍵は `context_tokens`**（作業単位 A-13a-2 で `FACE_KEYS` に入った。
            // 画面はこの鍵で、理由をその欄に結びつける）。
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "context_tokens": 32768 }, "sub": { "endpoint": "http://127.0.0.1:2", "model": "s", "context_tokens": 1024 } }"#,
                ShapeReason::ContextTokensUnusable,
                Some("b"),
                Some("sub"),
                Some("context_tokens"),
            ),
            // **鍵**（ADR-0056 §決定1・5 / 作業単位 A-13b-1）。**止まる側を `sub` にし、`main` には正しい鍵**。
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "api_key": "good-key" }, "sub": { "endpoint": "http://127.0.0.1:2", "model": "s", "api_key": 7 } }"#,
                ShapeReason::NotAString,
                Some("b"),
                Some("sub"),
                Some("api_key"),
            ),
            (
                r#""b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "api_key": "good-key" }, "sub": { "endpoint": "http://127.0.0.1:2", "model": "s", "api_key": "bad key" } }"#,
                ShapeReason::ApiKeyUnusable,
                Some("b"),
                Some("sub"),
                Some("api_key"),
            ),
            // **鍵のある面の宛先にユーザー情報を書かせない**（作業単位 A-13b-2。`Authorization` が2本になる）。
            // **`main` は鍵の無い面にユーザー情報**（鍵の無い面の扱いは変えない —— 通る）。
            (
                r#""b": { "main": { "endpoint": "http://u:p@192.0.2.10:1", "model": "m" }, "sub": { "endpoint": "http://u:p@192.0.2.11:2", "model": "s", "api_key": "good-key" } }"#,
                ShapeReason::ApiKeyWithUserinfo,
                Some("b"),
                Some("sub"),
                Some("endpoint"),
            ),
            // **承認は真偽値だけ**（作業単位 A-13b-2）。**`main` には正しい承認**。
            (
                r#""b": { "main": { "endpoint": "http://192.0.2.10:1", "model": "m", "api_key": "good-key", "allow_key_over_plain_http": true }, "sub": { "endpoint": "http://192.0.2.11:2", "model": "s", "allow_key_over_plain_http": "true" } }"#,
                ShapeReason::NotABoolean,
                Some("b"),
                Some("sub"),
                Some("allow_key_over_plain_http"),
            ),
        ];

        for (body, reason, profile, face, key) in cases {
            let source = if body.starts_with('[') {
                body.to_owned()
            } else {
                format!("{{ {ok}, {body} }}")
            };
            let err = parse_profiles(&source, None).expect_err(&source);
            let found = err
                .shape()
                .unwrap_or_else(|| panic!("形の不正であること: {err:?}"));
            assert_eq!(
                (found.reason(), found.profile(), found.face(), found.key()),
                (reason, profile, face, key),
                "{source}"
            );
        }

        // **対照**: 形の不正でないもの（JSON でない）は `shape` を持たない。
        assert!(
            parse_profiles("{", None)
                .expect_err("止まる")
                .shape()
                .is_none()
        );
    }

    /// 本文の面の鍵（囮）。
    const MAIN_KEY: &str = "pc-unit-main-KEY-1111";
    /// 裏方の面の鍵（囮）。**本文の鍵と別の綴り。**
    const SUB_KEY: &str = "pc-unit-sub-KEY-2222";
    /// 宛先のユーザー情報の合言葉（囮）。**鍵とも重ならない綴り。**
    const USERINFO_SECRET: &str = "pc-unit-userinfo-3333";

    /// **鍵は面ごとに読み、本文の鍵を裏方へ貸さない**（ADR-0056 §決定1）。
    ///
    /// | プロファイル | 本文 | 裏方 |
    /// |---|---|---|
    /// | 両方に別の鍵 | 本文の鍵 | 裏方の鍵 |
    /// | 本文だけに鍵・`sub` を書いた | 本文の鍵 | **無し**（別の宛先かもしれない） |
    /// | 本文だけに鍵・`sub` を書かない | 本文の鍵 | **無し**（裏方は本文のクライアントの複製を使う —— `App::bind`） |
    /// | 裏方だけに鍵 | 無し | 裏方の鍵 |
    #[test]
    fn each_face_reads_its_own_key_and_never_lends_the_body_key() {
        let key = |raw: Option<&str>| raw.map(|raw| ApiKey::parse(raw).expect("正当"));
        let face = |port: u16, model: &str, api_key: Option<&str>| {
            let mut face = serde_json::json!({
                "endpoint": format!("http://127.0.0.1:{port}"),
                "model": model,
            });
            if let Some(raw) = api_key {
                face["api_key"] = serde_json::json!(raw);
            }
            face
        };
        let source = serde_json::json!({
            "both": { "main": face(1, "m", Some(MAIN_KEY)), "sub": face(2, "s", Some(SUB_KEY)) },
            "main-only": { "main": face(1, "m", Some(MAIN_KEY)), "sub": face(2, "s", None) },
            "no-sub": { "main": face(1, "m", Some(MAIN_KEY)) },
            "sub-only": { "main": face(1, "m", None), "sub": face(2, "s", Some(SUB_KEY)) },
        })
        .to_string();
        let profiles = parse_profiles(&source, None).expect("読めること");

        for (name, main, sub) in [
            ("both", Some(MAIN_KEY), Some(SUB_KEY)),
            ("main-only", Some(MAIN_KEY), None),
            ("no-sub", Some(MAIN_KEY), None),
            ("sub-only", None, Some(SUB_KEY)),
        ] {
            let profile = profiles.select(name).expect("在ること");
            let settings = profile.settings();
            assert_eq!(
                profile.main().api_key(),
                key(main).as_ref(),
                "{name} の本文の面"
            );
            assert_eq!(settings.api_key(), key(main).as_ref(), "{name} の本文");
            assert_eq!(settings.sub_api_key(), key(sub).as_ref(), "{name} の裏方");
        }
    }

    /// **鍵のある面の宛先を、読み込みでは止めない**（作業単位 A-13b-2 / ADR-0056 §決定5）。
    ///
    /// A-13b-1 は `https://` かループバックの `http://` 以外を読み込みで止めていた。**A-13b-2 で、
    /// 送る側が接続のたびの名前解決で判定する形に置き換えた**ので、ローカルネットワーク・外・ホスト名の
    /// `http://` も読める。**承認は面ごとに読み、書かなければ偽、`sub` 面が無ければ写さない**（鍵と同じ規則）。
    /// **鍵の無い面も今までどおり通る**（対照）。
    #[test]
    fn a_keyed_face_over_plain_http_is_read_and_its_approval_is_per_face() {
        let profile = |endpoint: &str, api_key: Option<&str>, allow: Option<bool>| {
            let mut face = serde_json::json!({ "endpoint": endpoint, "model": "m" });
            if let Some(raw) = api_key {
                face["api_key"] = serde_json::json!(raw);
            }
            if let Some(allow) = allow {
                face["allow_key_over_plain_http"] = serde_json::json!(allow);
            }
            serde_json::json!({ "p": { "main": face } }).to_string()
        };
        for endpoint in [
            "https://203.0.113.5:443",
            "http://127.0.0.1:46001",
            "http://192.168.0.10:46001",
            "http://203.0.113.5:8080",
            "http://example.com:8080",
            r"http://203.0.113.5\@127.0.0.1:46001",
        ] {
            for (api_key, allow, expected) in [
                (Some(MAIN_KEY), None, false),
                (Some(MAIN_KEY), Some(true), true),
                (Some(MAIN_KEY), Some(false), false),
                (None, Some(true), true),
                (None, None, false),
            ] {
                let profiles = parse_profiles(&profile(endpoint, api_key, allow), None)
                    .unwrap_or_else(|err| panic!("{endpoint} は通る: {err:?}"));
                let chosen = profiles.select("p").expect("在ること");
                assert_eq!(
                    chosen.main().allow_key_over_plain_http(),
                    expected,
                    "{endpoint} / {allow:?}"
                );
                let settings = chosen.settings();
                assert_eq!(settings.allow_key_over_plain_http(), expected);
                // **`sub` 面が無ければ写さない**（抽出は本文のクライアントの複製で、同じ承認が効く）。
                assert!(!settings.sub_allow_key_over_plain_http());
            }
        }

        // **`sub` 面の承認は `sub` 面のもの**（本文の承認を貸さない）。
        let source = serde_json::json!({ "p": {
            "main": { "endpoint": "http://192.0.2.1:1", "model": "m", "api_key": MAIN_KEY,
                      "allow_key_over_plain_http": true },
            "sub": { "endpoint": "http://192.0.2.2:2", "model": "s", "api_key": SUB_KEY } },
            "q": {
            "main": { "endpoint": "http://192.0.2.1:1", "model": "m", "api_key": MAIN_KEY },
            "sub": { "endpoint": "http://192.0.2.2:2", "model": "s", "api_key": SUB_KEY,
                     "allow_key_over_plain_http": true } } })
        .to_string();
        let profiles = parse_profiles(&source, None).expect("読めること");
        for (name, main, sub) in [("p", true, false), ("q", false, true)] {
            let settings = profiles.select(name).expect("在ること").settings();
            assert_eq!(
                (
                    settings.allow_key_over_plain_http(),
                    settings.sub_allow_key_over_plain_http()
                ),
                (main, sub),
                "{name}"
            );
        }
    }

    /// **宛先を埋め込む形の文面は、宛先の秘密を落としてから埋め込む**（作業単位 B-71 のレビュー是正）。
    ///
    /// 文面（[`ProfileShape`] の `Display`）は起動時の標準エラーと、画面から保存したときの info 行へ出る。
    /// **`Debug` も文面を持つ**ので両方を見る。**理由の種類**（何で止まったか）を先に固定する ——
    /// 別の理由で止まって「秘密が無い」を見ても、その文面の検査にならない。
    #[test]
    fn a_shape_message_that_names_the_endpoint_never_carries_its_secrets() {
        for (face, reason, secrets) in [
            // ポートが無い（読めない）。
            (
                serde_json::json!({ "endpoint": "http://u:pw-secret@127.0.0.1/v1?token=q-secret", "model": "m" }),
                ShapeReason::EndpointUnreadable,
                &["pw-secret", "q-secret"][..],
            ),
            // `http://` の書き忘れ（WHATWG はスキーム `user` のホストの無い URL として読める）。
            (
                serde_json::json!({ "endpoint": "user:pw-secret@127.0.0.1:8080", "model": "m" }),
                ShapeReason::EndpointUnreadable,
                &["pw-secret"][..],
            ),
            // gguf つきでループバックでない。
            (
                serde_json::json!({ "endpoint": "http://u:pw-secret@192.0.2.1:1?token=q-secret",
                                    "model": "m", "gguf": "/planted/a.gguf" }),
                ShapeReason::EndpointNotLoopback,
                &["pw-secret", "q-secret"][..],
            ),
        ] {
            let source = serde_json::json!({ "p": { "main": face } }).to_string();
            let err = parse_profiles(&source, None).expect_err(&source);
            let shape = err.shape().expect("形の不正であること");
            assert_eq!(
                shape.reason(),
                reason,
                "前提: その理由で止まること: {source}"
            );
            let shown = [err.to_string(), shape.to_string(), format!("{err:?}")];
            for secret in secrets {
                for text in &shown {
                    assert!(
                        !text.contains(secret),
                        "文面に宛先の秘密が出ている（{secret}）: {text}"
                    );
                }
            }
        }
    }

    /// **鍵のある面の宛先にユーザー情報を書かせない。判定は `reqwest` と同じ読み方**（作業単位 A-13b-2）。
    ///
    /// `reqwest` はユーザー情報を Basic 認証にし、鍵の `Authorization` を足すので、2本送られる。
    /// **`http://203.0.113.5\@127.0.0.1:46001`** は、権限部を `@` で割る読み方（[`host_and_port`]）では
    /// ユーザー情報を持つように見えるが、WHATWG は `\` を `/` と読むので持たない（通す）。
    /// **鍵の無い面のユーザー情報は今までどおり通す**（対照）。
    #[test]
    fn a_keyed_face_with_userinfo_is_refused_as_reqwest_reads_it() {
        let profile = |endpoint: &str, api_key: Option<&str>| {
            let mut face = serde_json::json!({ "endpoint": endpoint, "model": "m" });
            if let Some(raw) = api_key {
                face["api_key"] = serde_json::json!(raw);
            }
            serde_json::json!({ "p": { "main": face } }).to_string()
        };
        for endpoint in [
            "http://user:pass@127.0.0.1:46001",
            "http://user@192.168.0.10:46001",
            "https://user:pass@example.com:8443",
        ] {
            let err = parse_profiles(&profile(endpoint, Some(MAIN_KEY)), None).expect_err(endpoint);
            let found = err.shape().expect("形の不正であること");
            assert_eq!(
                (found.reason(), found.face(), found.key()),
                (
                    ShapeReason::ApiKeyWithUserinfo,
                    Some("main"),
                    Some("endpoint")
                ),
                "{endpoint}"
            );
            // **対照**: 鍵が無ければ通る（鍵の無い面の扱いは変えない）。
            parse_profiles(&profile(endpoint, None), None)
                .unwrap_or_else(|err| panic!("鍵の無い {endpoint} は通る: {err:?}"));
        }

        let backslash = r"http://203.0.113.5\@127.0.0.1:46001";
        assert!(
            host_and_port(backslash).is_some(),
            "前提: 権限部の読み方では読めること"
        );
        parse_profiles(&profile(backslash, Some(MAIN_KEY)), None).unwrap_or_else(|err| {
            panic!("WHATWG ではユーザー情報を持たない綴りは通ること: {err:?}")
        });
    }

    /// **鍵は、読み込みの失敗の文面にも `Debug` にも出ない**（ADR-0056 §決定3）。
    ///
    /// **止まる理由が鍵そのもの（使えない字）でも、宛先（ユーザー情報 / 作業単位 A-13b-2）でも、隣の欄でも**——
    /// どの文面も鍵を写さない。**読めたプロファイルの `Debug` にも出ない。**
    #[test]
    fn the_key_never_appears_in_a_profile_error_or_debug_form() {
        let bad = format!("{MAIN_KEY} with space");
        let cases = [
            serde_json::json!({ "p": { "main": {
                "endpoint": "http://127.0.0.1:1", "model": "m", "api_key": bad } } }),
            serde_json::json!({ "p": { "main": {
                "endpoint": format!("http://planted-user:{USERINFO_SECRET}@192.0.2.1:1"),
                "model": "m", "api_key": MAIN_KEY } } }),
            serde_json::json!({ "p": { "main": {
                "endpoint": "http://127.0.0.1:1", "model": "m", "api_key": MAIN_KEY,
                "context_tokens": 1 } } }),
        ];
        for source in cases {
            let err = parse_profiles(&source.to_string(), None).expect_err("止まる");
            let shown = format!("{err} {err:?} {:?}", std::error::Error::source(&err));
            assert!(!shown.contains(MAIN_KEY), "{shown}");
            // **ユーザー情報で止めた文面は、宛先ごと写さない**（作業単位 A-13b-2）。
            assert!(!shown.contains(USERINFO_SECRET), "{shown}");
        }

        let source = serde_json::json!({ "p": {
            "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "api_key": MAIN_KEY },
            "sub": { "endpoint": "http://127.0.0.1:2", "model": "s", "api_key": SUB_KEY } } })
        .to_string();
        let profiles = parse_profiles(&source, None).expect("読めること");
        let profile = profiles.select("p").expect("在ること");
        let shown = format!("{profiles:?} {:?}", profile.settings());
        assert!(
            !shown.contains(MAIN_KEY) && !shown.contains(SUB_KEY),
            "{shown}"
        );
        assert!(
            shown.contains("ApiKey(***)"),
            "前提: 鍵を持っていること: {shown}"
        );
    }

    /// **理由の綴りは互いに違い、文面は空でない**（画面が分岐と表示に使う）。
    #[test]
    fn every_shape_reason_has_its_own_code() {
        let codes: BTreeSet<&str> = ShapeReason::ALL.iter().map(|r| r.code()).collect();
        assert_eq!(codes.len(), ShapeReason::ALL.len(), "綴りが重なっている");
        let messages: BTreeSet<&str> = ShapeReason::ALL.iter().map(|r| r.message()).collect();
        assert_eq!(messages.len(), ShapeReason::ALL.len(), "文面が重なっている");
    }

    /// **埋め込んだ見本は、コミットされている見本そのものである**（作業単位 F-4c）。
    ///
    /// **期待値を [`SAMPLE_PROFILES`] から作らない**——実行時にファイルを読んで突き合わせる。
    /// **別のファイルを埋め込む変異**（例えば利用者の写し `scripts/upstreams.json`）を、
    /// ここで止める。
    #[test]
    fn the_embedded_sample_is_the_committed_sample() {
        let committed = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("scripts")
                .join("upstreams.sample.json"),
        )
        .expect("見本を読めること");
        assert_eq!(SAMPLE_PROFILES, committed);
    }

    /// **無ければ、見本をそのまま書く。親のディレクトリも作る**（作業単位 F-4c）。
    ///
    /// **何も無い機**（`%LOCALAPPDATA%\personacore\` 自体が無い）で呼ばれるので、
    /// 親が無いことは普通に起きる。
    #[test]
    fn a_missing_profile_file_is_written_with_the_sample() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let path = dir
            .path()
            .join("never")
            .join("here")
            .join(UPSTREAMS_FILE_NAME);

        let outcome = write_sample_if_missing(&path).expect("書けること");

        assert_eq!(outcome, SampleOutcome::Wrote);
        assert_eq!(
            std::fs::read_to_string(&path).expect("読めること"),
            SAMPLE_PROFILES,
            "見本がそのまま書かれていない"
        );
    }

    /// **既にあるファイルは、絶対に上書きしない**（作業単位 F-4c / Issue origin#23）。
    ///
    /// # なぜ空のファイルも並べるのか
    ///
    /// **「空なら書いてよい」は親切に見えるが、利用者の決定を消す。**
    /// 空にしたのは利用者かもしれない。**在るものには触らない、を例外なく守る。**
    #[test]
    fn an_existing_profile_file_is_never_overwritten() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        for (name, body) in [
            (
                "edited.json",
                r#"{ "mine": { "main": { "endpoint": "http://127.0.0.1:1234", "model": "m" } } }"#,
            ),
            ("broken.json", "{ これは JSON ではない"),
            ("empty.json", ""),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, body).expect("置けること");

            let outcome = write_sample_if_missing(&path).expect("在れば何もしないこと");

            assert_eq!(outcome, SampleOutcome::AlreadyThere, "{name}");
            assert_eq!(
                std::fs::read_to_string(&path).expect("読めること"),
                body,
                "{name} が書き換えられている"
            );
        }
    }

    /// **書けなければ、理由を持って返す。`Display` は定数文字列**（CLAUDE.md のエラーの作法）。
    ///
    /// **親の位置にファイルが在る**＝ディレクトリを作れない形で作る。
    #[test]
    fn a_profile_file_that_cannot_be_written_is_an_error_without_a_path() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "a file, not a directory").expect("置けること");
        let path = blocker.join(UPSTREAMS_FILE_NAME);

        let err = write_sample_if_missing(&path).expect_err("書けないこと");

        assert_eq!(err.to_string(), "上流プロファイルの見本を書けませんでした");
        assert!(
            !err.to_string().contains("blocker"),
            "表の文に置き場所が載っている: {err}"
        );
        assert!(
            std::error::Error::source(&err).is_some(),
            "理由は source 側に残ること"
        );
    }

    /// **面の綴りは、ファイル名に使える字だけで繋ぐ。**
    ///
    /// # なぜ（2026-09-19 に実測）
    ///
    /// **この綴りは上流の出力の落とし先のファイル名にそのまま入る。**
    /// **`/` で繋いでいた版では、2つの面を1つの子が担うプロファイル
    /// （`sub` を書いていない形）で `File::create` が失敗し、
    /// 待ち受けが一度も開かなかった。**
    #[test]
    fn the_face_label_never_carries_a_path_separator() {
        use crate::measured::ModelFace;

        let both = face_labels(&[ModelFace::Main, ModelFace::Sub]);
        assert_eq!(both, "main+sub", "綴りの正");
        for forbidden in ['/', '\\', ':', '*', '?', '\"', '<', '>', '|'] {
            assert!(
                !both.contains(forbidden),
                "ファイル名に使えない字が入っている（{forbidden}）: {both}"
            );
        }
        // **1つだけの面は、そのままの綴り。**
        assert_eq!(face_labels(&[ModelFace::Sub]), "sub");
    }

    /// **ループバックの綴りは3通りとも読む。そして外向きは読まない。**
    ///
    /// **両向きを1本に置く**——片側だけだと「常に真」「常に偽」の片方が通る。
    #[test]
    fn the_loopback_check_answers_both_ways() {
        for host in ["127.0.0.1", "127.9.9.9", "localhost", "LOCALHOST", "::1"] {
            assert!(is_loopback(host), "{host} はループバック");
        }
        for host in ["192.168.0.10", "example.test", "0.0.0.0", "::"] {
            assert!(!is_loopback(host), "{host} はループバックではない");
        }
    }

    /// **権限部の読み方**（認証情報・IPv6・経路つき）。
    #[test]
    fn the_authority_scan_reads_the_value_it_is_given() {
        for (endpoint, expected) in [
            (
                "http://127.0.0.1:8080",
                Some(("127.0.0.1".to_owned(), 8080u16)),
            ),
            (
                "http://127.0.0.1:8080/v1/chat/completions",
                Some(("127.0.0.1".to_owned(), 8080)),
            ),
            (
                "http://alice:s3cr3t@127.0.0.1:8080/v1",
                Some(("127.0.0.1".to_owned(), 8080)),
            ),
            ("http://[::1]:8080", Some(("::1".to_owned(), 8080))),
            // **ポートを省いた綴りは読めない。**
            ("http://127.0.0.1", None),
            // **スキームが無い綴りも読めない。**
            ("127.0.0.1:8080", None),
        ] {
            assert_eq!(host_and_port(endpoint), expected, "{endpoint}");
        }
    }

    /// **`~` は渡されたホームの下へ。渡されなければ、綴りをそのまま残す。**
    #[test]
    fn the_home_expansion_reads_the_home_it_is_given() {
        let home = Path::new("/planted");
        assert_eq!(
            expand_home("~/models/deep.gguf", Some(home)),
            Path::new("/planted").join("models").join("deep.gguf")
        );
        assert_eq!(
            expand_home("~\\models\\deep.gguf", Some(home)),
            Path::new("/planted").join("models").join("deep.gguf")
        );
        // **先頭以外の `~` は触らない。**
        assert_eq!(
            expand_home("/opt/~models/a.gguf", Some(home)),
            PathBuf::from("/opt/~models/a.gguf")
        );
        // **`~利用者名` は展開しない**（レビュー是正7 / 2026-09-19）。
        // **`strip_prefix("~")` に緩める変異は、ここが無いと緑を通る** ——
        // 緩めると `~bob/a.gguf` が `<自分のホーム>/bob/a.gguf` になり、
        // **他人のホームを指したつもりの綴りが、黙って自分の下を指す。**
        assert_eq!(
            expand_home("~bob/a.gguf", Some(home)),
            PathBuf::from("~bob/a.gguf"),
            "~利用者名 の形は展開しないこと"
        );
        // **ホームが無ければ、綴りをそのまま。**
        assert_eq!(
            expand_home("~/a.gguf", None),
            PathBuf::from("~/a.gguf"),
            "展開できないときに相対パスへ落とさないこと"
        );
    }
}
