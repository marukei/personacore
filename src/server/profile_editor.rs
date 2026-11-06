//! **画面から上流プロファイルを読む・足す・直す**（[ADR-0042](../../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)
//! §決定1・5・8 / 作業単位 F-4b-3 / Issue origin#24）。
//!
//! ```text
//! GET /admin/upstream/profiles            … 一覧（書いたままの値）・書けるか・置き場所・内容の印
//! PUT /admin/upstream/profiles/{name}     … 1つのプロファイルを足す・直す（読んだときの印を添える）
//!   0. 書けるのは既定の置き場所だけ（PERSONACORE_UPSTREAMS が指す先は読むだけ / 409）
//!   1. 名前は起動時の `--upstream` が受け付ける綴りだけ（`fresh::parse_args` を通す / 400）
//!   2. いまのファイルの印が、画面が読んだときの印と違えば 409（手で書き換えられていた）
//!   3. いまのファイルが検証に通らなければ 422（画面からは直せない。手で直す）
//!   4. 直した後のファイル全体を `parse_profiles` にもう一度通す（通らなければ 422 と理由）
//!   5. 同じディレクトリの一時ファイルに書いて差し替える（置き換えで書く）
//!   6. 読み直して、書いたバイト列そのものであることを確かめる
//! ```
//!
//! **消す口は置かない**（ADR-0042 §決定8）。
//!
//! # 書いた値と観測した値を混ぜない（Issue origin#24 の閉じる条件）
//!
//! **ここが返す gguf のパスは、利用者が書いたままの綴り**（`~` を含むなら `~` のまま）。
//! `GET /admin/upstream` が絶対パスを落としているのは、あちらが**上流が名乗った（観測した）値**だから。
//! **こちらは `upstreams.json` の生の値を読むだけで、展開した `FaceLaunch::gguf` を1度も使わない。**
//!
//! # 検証の写しを作らない
//!
//! **保存してよいかを決めるのは [`parse_profiles`]（起動時と差し替えのたびに通る関数）だけ。**
//! 名前の規則は **[`parse_args`]（`--upstream <名前>` を読む関数）** に通して決める
//! ——「画面では作れたのに、`--upstream` では選べない名前」を作らない。
//!
//! # ⚠ 守れないもの
//!
//! * **印を見てから差し替えるまでの間に、別の書き手（利用者のエディタ）が書いた内容は上書きしうる。**
//!   この実行ファイルの中の書き手は錠で1つにしてあり、差し替えの直前に印をもう一度見るので、
//!   窓は「2度目の読みから改名まで」に縮めてあるが、**ゼロではない**（OS のファイル錠は使っていない）。
//! * **書き直すと、手で整えた並びや空白は失われる**（ADR-0042 §決定8 が受け入れた）。
//! * **印の確認は2度ある（読んだ直後と、差し替える直前）が、2度目にしか無い守りを観測する継ぎ目は無い**
//!   （レビュー1周目）。錠の下では、1度目を通った依頼の印は2度目でも同じなので、
//!   どちらか片方を外しても緑のまま（条件付き等価）。**両方を外すと赤になる**
//!   （`tests/upstream_profile_editor.rs::a_stale_revision_is_refused_and_the_file_is_untouched`）。
//!   2度目が守るのは「外のエディタが間に書いた」窓だけで、それを作る継ぎ目をテストは持たない。
//! * **書いた後に読み直した値が依頼と一致するかの確認（`save_with` の 6 の2つ目）は観測されていない。**
//!   1つ目（バイト列の一致）は、改名の直後に書く継ぎ目で観測している
//!   （`a_writer_after_the_replacement_is_reported_as_verify_failed`）。
//! * **保存の錠（[`ProfileEditor`] の `lock`）を外しても、テストは緑のまま**（レビュー1周目）。
//!   統合テストは `current_thread` の実行時で、同時に2つの `PUT` を走らせる腕が無い。
//!   **本番はマルチスレッドの実行時**なので、錠が無いと2つの保存が同じ印を見て互いを上書きしうる。

use std::path::{Path, PathBuf};

use axum::Json;
use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path as UrlPath, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;
use utoipa::openapi::schema::Object;

use super::error::{ApiError, ErrorBody};
use super::fresh::{UPSTREAM_FLAG, parse_args};
use super::upstreams::ShapeReason;
use super::upstreams::{FACE_KEYS, FACE_NAMES, ProfileError, home_dir, parse_profiles};
use super::{AppState, openapi};
use crate::llm::ApiKey;

/// ログに出す入口の名前（`ApiError::from_json_rejection`）。
const ENDPOINT: &str = "admin/upstream/profiles";

// ----------------------------------------------------------------------
// 置き場所
// ----------------------------------------------------------------------

/// **上流プロファイルの置き場所と、そこへ書いてよいか**（ADR-0042 §決定1・8）。
///
/// **書いてよいのは既定の置き場所（DB と同じディレクトリ）だけ。**
/// `PERSONACORE_UPSTREAMS` が指すファイルは、コミットされた見本かもしれない
/// （`start.ps1 -Upstream` は `scripts/upstreams.json` か `scripts/upstreams.sample.json` を指す）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProfilesLocation {
    /// **置き場所が無い**（OS のデータディレクトリが読めない／テストの入口）。
    None,
    /// **読むだけ**（`PERSONACORE_UPSTREAMS` が指す先）。
    ReadOnly(PathBuf),
    /// **読み書きできる**（既定の置き場所）。
    Editable(PathBuf),
}

impl ProfilesLocation {
    /// 置き場所（書けるかに関わらず）。
    pub(crate) fn path(&self) -> Option<&Path> {
        match self {
            Self::None => None,
            Self::ReadOnly(path) | Self::Editable(path) => Some(path),
        }
    }

    const fn editable(&self) -> bool {
        matches!(self, Self::Editable(_))
    }
}

// ----------------------------------------------------------------------
// 書いたままの値（**純粋関数**）
// ----------------------------------------------------------------------

/// **1つの面の、利用者が書いたままの値**（**応答**の形 / 依頼の形は [`WrittenFaceRequest`]）。
///
/// **`gguf` は展開しない**（`~` を含むなら `~` のまま）。**`endpoint` / `model` も整えない**
/// （読み込みは前後の空白を落として使うが、ここは書いた綴りを返す）。
///
/// # 応答と依頼で型を分けた（作業単位 P-2a / 司令塔の決定）
///
/// **線の上の約束が向きで違う**——応答は9つの欄を必ず載せ（書いていない鍵は `null`。鍵は値の代わりに
/// `api_key_set` / `api_key_hint`。承認は書いていなければ `false`）、依頼は `gguf` / `args` / `command` /
/// `context_tokens` / `api_key` / `allow_key_over_plain_http` を省ける。
/// 1つの型だと、仕様書（`utoipa`）は **どちらか片方の向きでしか正しく書けない。**
/// 依頼を読んだら [`From`] で [`FaceEdit`]（この型と [`KeyChange`]）へ移す。
///
/// # `context_tokens`（作業単位 A-13a-2 / ADR-0054 §決定5）
///
/// **型は `u64`**（読み込みの検証は `u32` に入る・応答の余白より大きい、を見る）。
/// 依頼を `u32` で読むと、`u32` に入らない値は本文として読めない 400 になり、
/// **理由（`context_tokens_unusable`）と欄（`key`）が画面に届かない。** `u64` で受けて、
/// **使えるかは読み込みと同じ関数（[`parse_profiles`]）に決めさせる**（検証の写しを作らない）。
/// 負の値・整数でない値は 400 のまま（画面は送る前に止める）。
///
/// # 鍵（`api_key` / 作業単位 A-13b-1 / ADR-0056 §決定3）
///
/// **値は線に載せない**（`api_key` は `#[serde(skip)]`）。応答に載るのは
/// **`api_key_set`（書いてあるか）と `api_key_hint`（末尾4文字。書いていない・短い鍵は `null`）だけ。**
/// 3つは [`Self::with_api_key`] だけが揃えて決める（食い違った組を作らない）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct WrittenFace {
    endpoint: String,
    model: String,
    #[schema(required = true)]
    gguf: Option<String>,
    #[schema(required = true)]
    args: Option<Vec<String>>,
    #[schema(required = true)]
    command: Option<String>,
    /// **書いたコンテキスト長**（ADR-0054 §決定1）。`null` は「書いていない」（上流に聞く）。
    #[schema(required = true)]
    context_tokens: Option<u64>,
    /// **書いた鍵。線に載せない**（`Debug` も伏せる / `llm::ApiKey`）。
    #[serde(skip)]
    api_key: Option<ApiKey>,
    /// **鍵を書いてあるか**（ADR-0056 §決定3）。
    api_key_set: bool,
    /// **鍵の末尾4文字**（ADR-0056 §決定3）。書いていない・8 文字より短い鍵は `null`。
    #[schema(required = true)]
    api_key_hint: Option<String>,
    /// **書いた承認**（`allow_key_over_plain_http` / ADR-0056 §決定5 / 作業単位 A-13b-3）。
    /// 書いていなければ `false`。**判定の結果ではない**（実際に送るかは送る側が接続のたびに決める）。
    ///
    /// **ファイルへ書くのは [`FaceEdit::to_value`] だけ**（[`Self::to_value`] はこの欄を書かない）。
    allow_key_over_plain_http: bool,
}

/// **鍵をどうするか**（依頼の面の `api_key` / ADR-0056 §決定3）。
///
/// | 依頼 | 値 | 保存 |
/// |---|---|---|
/// | 書いていない | [`Self::Keep`] | **いまのファイルの、同じ名前・同じ面の鍵を残す**（画面は鍵の値を持たない） |
/// | `null` | [`Self::Clear`] | 鍵ごと消す |
/// | 文字列 | [`Self::Set`] | その鍵に置き換える（使えるかは読み込みと同じ検証が決める） |
///
/// **`Debug` は伏せる**（[`Self::Set`] は鍵そのものを持つ）。
#[derive(Clone, PartialEq, Eq, Default)]
pub(crate) enum KeyChange {
    /// 残す。
    #[default]
    Keep,
    /// 消す。
    Clear,
    /// 置き換える。**検証前の綴り**（検証は [`parse_profiles`] が保存の前に行う）。
    Set(String),
}

impl std::fmt::Debug for KeyChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Keep => "Keep",
            Self::Clear => "Clear",
            Self::Set(_) => "Set(***)",
        })
    }
}

/// **省いた `api_key` と `null` を分けて読む**（[`KeyChange`]）。省いたときは `#[serde(default)]` が
/// [`KeyChange::Keep`] を入れ、ここは呼ばれない。
fn key_change<'de, D>(deserializer: D) -> Result<KeyChange, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Option::<String>::deserialize(deserializer)? {
        None => KeyChange::Clear,
        Some(raw) => KeyChange::Set(raw),
    })
}

/// **1つの面の、利用者が書いたままの値**（**依頼**の形 / `PUT /admin/upstream/profiles/{name}`）。
///
/// **`deny_unknown_fields`**（`/admin/` の全入口で共通の方針）。
/// **省いた鍵と `null` は同じ意味**（書かない）。応答の形は [`WrittenFace`]。
///
/// **⚠ `api_key` だけは省略と `null` の意味が違う**（[`KeyChange`] / ADR-0056 §決定3）。
///
/// # 承認（`allow_key_over_plain_http` / 作業単位 A-13b-3）
///
/// | 依頼 | 保存 |
/// |---|---|
/// | 書いていない・`null` | **今の規則**（いまの面の `endpoint` と文字列として同じときだけ、いまの承認を残す / [`FaceEdit::to_value`]） |
/// | `true` | 承認を書く（宛先を変えた保存でも書く —— 画面が新しい宛先について承認を取った） |
/// | `false` | 承認を外す（鍵ごと消す。`false` を書き残さない） |
///
/// **`null` を「外す」と読まない**（`api_key` と違い、省略と `null` を分けない）。外すのは `false` だけ。
///
/// **保存の後に鍵の無い面には、どの依頼でも承認を書かない**（作業単位 A-13b-3 のレビュー是正 / [`FaceEdit::to_value`]）。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct WrittenFaceRequest {
    endpoint: String,
    model: String,
    #[serde(default)]
    gguf: Option<String>,
    #[serde(default)]
    args: Option<Vec<String>>,
    #[serde(default)]
    command: Option<String>,
    /// **書くコンテキスト長。** 使えるか（応答の余白より大きく、`u32` に入るか）は保存の検証が決める。
    #[serde(default)]
    context_tokens: Option<u64>,
    /// **鍵**（書いていない＝残す / `null`＝消す / 文字列＝置き換える / [`KeyChange`]）。
    #[serde(default, deserialize_with = "key_change")]
    #[schema(value_type = Option<String>)]
    api_key: KeyChange,
    /// **承認**（書いていない・`null`＝今の規則 / `true`＝承認する / `false`＝外す）。
    #[serde(default)]
    allow_key_over_plain_http: Option<bool>,
}

/// **読んだ依頼の1面ぶん**（鍵と承認以外の欄と、鍵と承認をどうするか）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FaceEdit {
    /// 鍵と承認以外の欄（**`api_key` は常に `None`、`allow_key_over_plain_http` は常に `false`**）。
    face: WrittenFace,
    key: KeyChange,
    /// **承認をどうするか**（`None` は今の規則 / [`WrittenFaceRequest`] の表）。
    approval: Option<bool>,
}

/// **読んだ依頼を、応答と同じ形へ移す**（欄はそのまま / 何も整えない）。
impl From<WrittenFaceRequest> for FaceEdit {
    fn from(request: WrittenFaceRequest) -> Self {
        Self {
            face: WrittenFace {
                endpoint: request.endpoint,
                model: request.model,
                gguf: request.gguf,
                args: request.args,
                command: request.command,
                context_tokens: request.context_tokens,
                api_key: None,
                api_key_set: false,
                api_key_hint: None,
                allow_key_over_plain_http: false,
            },
            key: request.api_key,
            approval: request.allow_key_over_plain_http,
        }
    }
}

/// **1つのプロファイルの、書いたままの値。**
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct WrittenProfile {
    name: String,
    main: WrittenFace,
    #[schema(required = true)]
    sub: Option<WrittenFace>,
}

/// **検証には通ったが、画面の形（[`WrittenFace`]）に写せない値がある**。
///
/// **起こりうるのは1つだけ**: gguf を書いていない面の `args` / `command` は、読み込みが見ない
/// （使わないので型も確かめない）。そこに配列でない `args` 等を手で書くと、ここに来る。
///
/// **A-13a-1 の間はもう1つあった**——`context_tokens` を書いた面（画面の形がその欄を
/// 持たなかった。理由 `not_yet_editable`）。**作業単位 A-13a-2 で画面が追いついたので、
/// その起こり方ごと外した。**
///
/// **`face` / `key` は定数**（どちらも [`FACE_NAMES`] / [`FACE_KEYS`] の綴り）。
/// **前提（検証済み）が崩れた入力では `None` になる**（その場合も panic しない）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Unrepresentable {
    face: Option<&'static str>,
    key: Option<&'static str>,
}

impl Unrepresentable {
    const WHOLE: Self = Self {
        face: None,
        key: None,
    };
}

/// **検証済みの JSON から、書いたままの値を取り出す。純粋関数。**
///
/// **前提: `source` は [`parse_profiles`] を通っている。** 通っていない入力を渡したら、
/// 写せない値として返す（panic しない）。**並びは名前の順。**
pub(crate) fn written_profiles(source: &str) -> Result<Vec<WrittenProfile>, Unrepresentable> {
    let Ok(root) = serde_json::from_str::<Map<String, Value>>(source) else {
        return Err(Unrepresentable::WHOLE);
    };
    let [main_label, sub_label] = FACE_NAMES;
    let mut profiles = Vec::with_capacity(root.len());
    for (name, body) in &root {
        let Value::Object(body) = body else {
            return Err(Unrepresentable::WHOLE);
        };
        let Some(main) = body.get(main_label) else {
            return Err(Unrepresentable::WHOLE);
        };
        profiles.push(WrittenProfile {
            name: name.clone(),
            main: written_face(main, main_label)?,
            sub: body
                .get(sub_label)
                .map(|sub| written_face(sub, sub_label))
                .transpose()?,
        });
    }
    // **名前の順**（`serde_json` の既定の `Map` は並べ替え済みだが、それに寄りかからない）。
    profiles.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(profiles)
}

/// **1つの面の、書いたままの値。** 型が画面の形に合わなければ、その鍵を名指しする。
fn written_face(value: &Value, face: &'static str) -> Result<WrittenFace, Unrepresentable> {
    let at = |key: &'static str| Unrepresentable {
        face: Some(face),
        key: Some(key),
    };
    let Value::Object(object) = value else {
        return Err(Unrepresentable {
            face: Some(face),
            key: None,
        });
    };
    if object.keys().any(|key| !FACE_KEYS.contains(&key.as_str())) {
        return Err(Unrepresentable {
            face: Some(face),
            key: None,
        });
    }
    // **承認（`allow_key_over_plain_http`）は書いた値を写す**（作業単位 A-13b-3。A-13b-2 の間は画面の形に欄が無く、写さなかった）。
    let [
        endpoint,
        model,
        gguf,
        args,
        command,
        context_tokens,
        api_key,
        allow_key_over_plain_http,
    ] = FACE_KEYS;
    let required = |key: &'static str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| at(key))
    };
    let optional = |key: &'static str| match object.get(key) {
        None => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(at(key)),
    };
    let args_value = match object.get(args) {
        None => None,
        Some(Value::Array(items)) => Some(
            items
                .iter()
                .map(|item| item.as_str().map(str::to_owned).ok_or_else(|| at(args)))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(_) => return Err(at(args)),
    };
    // **検証済みなら必ず整数**（読み込みが `u32` に入る・応答の余白より大きい、を見ている）。
    // 整数でなければ、前提が崩れた入力なので写せないと言う（panic しない）。
    let window = match object.get(context_tokens) {
        None => None,
        Some(value) => Some(value.as_u64().ok_or_else(|| at(context_tokens))?),
    };
    // **検証済みなら必ず使える鍵**（読み込みが `ApiKey::parse` を通している）。
    let key = match object.get(api_key) {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .and_then(|raw| ApiKey::parse(raw).ok())
                .ok_or_else(|| at(api_key))?,
        ),
    };
    // **検証済みなら必ず真偽値**（読み込みが真偽値でない値を止めている）。
    let approved = match object.get(allow_key_over_plain_http) {
        None => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| at(allow_key_over_plain_http))?,
    };
    Ok(WrittenFace {
        endpoint: required(endpoint)?,
        model: required(model)?,
        gguf: optional(gguf)?,
        args: args_value,
        command: optional(command)?,
        context_tokens: window,
        api_key: None,
        api_key_set: false,
        api_key_hint: None,
        allow_key_over_plain_http: approved,
    }
    .with_api_key(key))
}

// ----------------------------------------------------------------------
// 内容の印（**純粋関数**）
// ----------------------------------------------------------------------

/// **ファイルの内容の印**（ADR-0042 §決定8）。**バイト列そのものから作る。**
///
/// **この実行ファイルの中でだけ比べる印**なので、暗号学的な強さは要らない
/// （[`std::hash::DefaultHasher::new`] は同じビルドの中で決定的）。
/// **長さも混ぜる**——64 bit の衝突を、長さの違う書き換えでは起こさない。
pub(crate) fn revision_of(bytes: &[u8]) -> String {
    use std::hash::{DefaultHasher, Hasher as _};

    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    format!("{:x}-{:016x}", bytes.len(), hasher.finish())
}

// ----------------------------------------------------------------------
// 1つのプロファイルを足す・直す（**純粋関数**）
// ----------------------------------------------------------------------

/// **保存したい1つのプロファイル**（名前を除く）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileEdit {
    main: FaceEdit,
    sub: Option<FaceEdit>,
}

/// **保存できない理由**（純粋な部分）。
#[derive(Debug)]
pub(crate) enum EditError {
    /// **いまのファイルが検証に通らない**（画面からは直せない。手で直す）。
    CurrentInvalid(ProfileError),
    /// **いまのファイルは検証に通るが、画面の形に写せない**（一覧の `unrepresentable`。手で直す）。
    CurrentUnrepresentable,
    /// **`--upstream` が受け付けない名前。**
    NameRejected,
    /// **直した後のファイルが検証に通らない。**
    Rejected(ProfileError),
    /// JSON に書き出せない（`Value` からは起きない）。
    Serialize(serde_json::Error),
}

/// **その名前は `--upstream <名前>` で選べるか。** 判定は [`parse_args`] に任せる（写しを作らない）。
///
/// **「選んだ名前が、そのままの綴りで選ばれる」ことまで見る**——`--upstream` は前後の空白を落とすので、
/// `" x"` は `x` を選びに行く（画面で作れたのに、起動時には別の名前を探す）。
pub(crate) fn accepts_name(name: &str) -> bool {
    parse_args([UPSTREAM_FLAG, name])
        .ok()
        .is_some_and(|args| args.upstream() == Some(name))
}

impl WrittenFace {
    /// **鍵と、応答に載せる2つ（`api_key_set` / `api_key_hint`）を揃えて決める。**
    ///
    /// **3つを別々に書く場所を作らない**（印だけ立って末尾が無い、のような組を作らない）。
    #[must_use]
    fn with_api_key(self, api_key: Option<ApiKey>) -> Self {
        Self {
            api_key_set: api_key.is_some(),
            api_key_hint: api_key.as_ref().and_then(ApiKey::hint),
            api_key,
            ..self
        }
    }

    /// **ファイルに書く形。** 書かない鍵は `null` で残さない（読み込みは `"gguf": null` を止める）。
    ///
    /// **`context_tokens` を書いていない面は、A-13a-2 の前と同じバイト列になる**
    /// （鍵を足さない / 見張り番は `a_face_without_context_tokens_is_written_byte_for_byte_as_before`）。
    /// **鍵（`api_key`）を書いていない面も、A-13b-1 の前と同じバイト列になる**（同じ理由で鍵を足さない）。
    /// **承認（`allow_key_over_plain_http`）はここでは書かない**（[`FaceEdit::to_value`] が依頼といまのファイルから決める）。
    fn to_value(&self) -> Value {
        let [
            endpoint,
            model,
            gguf,
            args,
            command,
            context_tokens,
            api_key,
            _allow_key_over_plain_http,
        ] = FACE_KEYS;
        let mut object = Map::new();
        object.insert(endpoint.to_owned(), Value::from(self.endpoint.as_str()));
        object.insert(model.to_owned(), Value::from(self.model.as_str()));
        if let Some(value) = &self.gguf {
            object.insert(gguf.to_owned(), Value::from(value.as_str()));
        }
        if let Some(values) = &self.args {
            object.insert(
                args.to_owned(),
                Value::Array(values.iter().map(|arg| Value::from(arg.as_str())).collect()),
            );
        }
        if let Some(value) = &self.command {
            object.insert(command.to_owned(), Value::from(value.as_str()));
        }
        if let Some(value) = self.context_tokens {
            object.insert(context_tokens.to_owned(), Value::from(value));
        }
        if let Some(key) = &self.api_key {
            object.insert(api_key.to_owned(), Value::from(key.expose_secret()));
        }
        Value::Object(object)
    }
}

impl FaceEdit {
    /// **ファイルに書く形**（鍵は [`KeyChange`] に従う）。
    ///
    /// `current` は**いまのファイルの、同じ名前・同じ面**（無ければ `None`）。
    /// [`KeyChange::Keep`] はそこから鍵の値を**そのまま**写す（整えない・検証は保存の前に全体で行う）。
    /// **別の名前・別の面の鍵は見ない**（鍵は宛先に付く / ADR-0056 §決定1）。
    ///
    /// # 承認（`allow_key_over_plain_http`）
    ///
    /// **依頼が `true` / `false` を送れば、それに従う**（作業単位 A-13b-3）。`true` は書き、`false` は鍵ごと消す
    /// （`false` を書き残さない —— 書いていないのと同じ意味なので、バイト列を増やさない）。
    /// `true` は宛先を変えた保存でも書く（画面が新しい宛先について危険を示し、承認を取った）。
    ///
    /// ## 送らなければ（`None`）、宛先が変わらないときだけ残す（作業単位 A-13b-2 とそのレビュー是正）
    ///
    /// 残さないと、手で書いた承認が、画面から別の欄を直しただけで消え、次の送信から
    /// `upstream_key_not_approved` になる。
    /// **残す先は鍵と同じ**（同じ名前・同じ面。別の名前・別の面の承認は見ない）。
    ///
    /// **ただし、いまの面の `endpoint` と保存後の `endpoint` が文字列として一致するときだけ**。
    /// 承認は「この宛先へ鍵を平文で送ってよい」という判断で、宛先に付く（`config::retarget_llm` と同じ立場）。
    /// 宛先を変えた保存で持ち越すと、**利用者が承認していない新しい外の宛先へ鍵が平文で出る**
    /// （ADR-0056 §決定5 は危険を示したうえで承認を取る）。いまの面に `endpoint` の文字列が無ければ落とす。
    /// **正規化しない**（末尾の `/` が違うだけでも落とす —— 落とす側は送らないだけで、鍵は出ない）。
    /// 見張り番は `tests/upstream_profile_editor.rs` の `a_hand_written_approval_survives_a_save_from_the_screen`。
    fn to_value(&self, current: Option<&Map<String, Value>>) -> Value {
        let mut value = self.face.to_value();
        let key = match &self.key {
            KeyChange::Keep => current
                .and_then(|face| face.get(super::upstreams::API_KEY_KEY))
                .cloned(),
            KeyChange::Clear => None,
            KeyChange::Set(raw) => Some(Value::from(raw.as_str())),
        };
        let same_endpoint = current
            .and_then(|face| face.get("endpoint"))
            .and_then(Value::as_str)
            .is_some_and(|endpoint| endpoint == self.face.endpoint);
        // **保存の後に鍵の無い面には、承認を書かない**（作業単位 A-13b-3 のレビュー是正）。
        // 残すと、後で同じ宛先に鍵を足したとき、確認なしで承認済みとして送られる。断らずに落とす。
        let approval = match self.approval {
            _ if key.is_none() => None,
            Some(true) => Some(Value::Bool(true)),
            Some(false) => None,
            None => current
                .filter(|_| same_endpoint)
                .and_then(|face| face.get(super::upstreams::ALLOW_KEY_OVER_PLAIN_HTTP_KEY))
                .cloned(),
        };
        if let Value::Object(object) = &mut value {
            if let Some(key) = key {
                object.insert(super::upstreams::API_KEY_KEY.to_owned(), key);
            }
            if let Some(approval) = approval {
                object.insert(
                    super::upstreams::ALLOW_KEY_OVER_PLAIN_HTTP_KEY.to_owned(),
                    approval,
                );
            }
        }
        value
    }

    /// **書いた後に読み直した面が、この依頼どおりか。**
    ///
    /// 鍵と承認以外の欄は全部一致すること。鍵は [`KeyChange::Clear`] なら無いこと・[`KeyChange::Set`] なら
    /// その綴りであること。**[`KeyChange::Keep`] は見ない**（残した値はいまのファイルが決める）。
    /// 承認も同じ —— 送った値なら一致すること、**送らなければ見ない**（今の規則といまのファイルが決める）。
    fn is_reflected_in(&self, listed: &WrittenFace) -> bool {
        let key_matches = match &self.key {
            KeyChange::Keep => true,
            KeyChange::Clear => listed.api_key.is_none(),
            KeyChange::Set(raw) => listed
                .api_key
                .as_ref()
                .is_some_and(|key| key.expose_secret() == raw),
        };
        // **鍵の無い面では、`true` を送っても書かれない**（[`Self::to_value`]）。
        let approval_matches = self.approval.is_none_or(|wanted| {
            listed.allow_key_over_plain_http == (wanted && listed.api_key.is_some())
        });
        let others = WrittenFace {
            allow_key_over_plain_http: false,
            ..listed.clone().with_api_key(None)
        };
        key_matches && approval_matches && others == self.face
    }
}

impl ProfileEdit {
    /// **ファイルに書く形。** `sub` が無ければ鍵ごと書かない（本文と同じ上流 / ADR-0039 §決定1）。
    ///
    /// `current` は**いまのファイルの、同じ名前のプロファイル**（無ければ `None` / [`FaceEdit::to_value`]）。
    fn to_value(&self, current: Option<&Value>) -> Value {
        let [main_label, sub_label] = FACE_NAMES;
        let current_face = |label: &str| {
            current
                .and_then(Value::as_object)
                .and_then(|profile| profile.get(label))
                .and_then(Value::as_object)
        };
        let mut object = Map::new();
        object.insert(
            main_label.to_owned(),
            self.main.to_value(current_face(main_label)),
        );
        if let Some(sub) = &self.sub {
            object.insert(sub_label.to_owned(), sub.to_value(current_face(sub_label)));
        }
        Value::Object(object)
    }
}

/// **いまのファイルに1つのプロファイルを足す・直した、ファイル全体を返す。純粋関数。**
///
/// * `current` が `None` なら、無いファイルに書く（そのプロファイル1つだけのファイル）。
/// * **他のプロファイルの値は1つも変えない**（JSON として組み直すので、並びと空白は変わる）。
/// * **返す前に、ファイル全体を [`parse_profiles`] に通す。**
pub(crate) fn apply_edit(
    current: Option<&str>,
    name: &str,
    edit: &ProfileEdit,
    home: Option<&Path>,
) -> Result<String, EditError> {
    if !accepts_name(name) {
        return Err(EditError::NameRejected);
    }

    let mut root = match current {
        None => Map::new(),
        Some(source) => {
            // **いまのファイルが検証に通らなければ、画面からは触らない**——
            // 通らない行を抱えたまま書き直すと、保存の検証が「どこが悪いか」を別の行で言う。
            parse_profiles(source, home).map_err(EditError::CurrentInvalid)?;
            // **画面の形に写せないファイルにも書かない**（レビュー1周目の是正1）。
            // 一覧はそれを `invalid` と言うので、保存も同じく断る——ここを通すと、
            // 置き換えた後の読み直し（`save_with` の 6）で初めて気付き、**書き換えてから 500** になる。
            if written_profiles(source).is_err() {
                return Err(EditError::CurrentUnrepresentable);
            }
            serde_json::from_str::<Map<String, Value>>(source)
                .map_err(|err| EditError::CurrentInvalid(ProfileError::Json(err)))?
        }
    };
    // **送らなかった鍵は、いまのファイルの同じ名前から残す**（[`KeyChange::Keep`]）。
    let written = edit.to_value(root.get(name));
    root.insert(name.to_owned(), written);

    let mut text =
        serde_json::to_string_pretty(&Value::Object(root)).map_err(EditError::Serialize)?;
    text.push('\n');

    // **保存してよいかを決めるのは、読み込みと同じ関数だけ**（ADR-0042 §決定8）。
    parse_profiles(&text, home).map_err(EditError::Rejected)?;
    Ok(text)
}

// ----------------------------------------------------------------------
// 置き換えで書く（I/O）
// ----------------------------------------------------------------------

/// **置き換えの手順**（失敗を注入する継ぎ目）。**本番は [`RealSteps`] だけ。**
pub(crate) trait ReplaceSteps {
    /// **一時ファイルを新しく作って書き切る**（在れば失敗する）。
    fn write_new(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()>;
    /// **一時ファイルで宛先を置き換える。**
    fn replace(&self, from: &Path, to: &Path) -> std::io::Result<()>;
}

/// 本物の手順。
pub(crate) struct RealSteps;

impl ReplaceSteps for RealSteps {
    fn write_new(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write as _;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(bytes)?;
        // **差し替える前に、中身をディスクへ落とす**——落とす前に電源が落ちると、
        // 差し替えた先が空のファイルになりうる。
        file.sync_all()
    }

    fn replace(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        // **宛先が在れば置き換える**（Windows でも `MoveFileExW` の `REPLACE_EXISTING`）。
        std::fs::rename(from, to)
    }
}

/// **置き換えで書けなかった。** `Display` は定数。
#[derive(Debug, thiserror::Error)]
#[error("上流プロファイルを書き換えられませんでした")]
pub(crate) struct ReplaceError(#[source] std::io::Error);

/// 一時ファイルの名前を、同じプロセスの中で重ねないための番号。
static NEXT_TEMPORARY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// **同じディレクトリの一時ファイルに書いてから、宛先を置き換える。**
///
/// **失敗したら一時ファイルを消す**（消せなければログに言う）。**宛先は元のまま。**
///
/// **同じディレクトリに置く**のは、置き換えを同じボリュームの中の改名にするため
/// （ボリュームをまたぐ改名は置き換えにならない）。
pub(crate) fn replace_file_with(
    steps: &impl ReplaceSteps,
    path: &Path,
    bytes: &[u8],
) -> Result<(), ReplaceError> {
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().map_or_else(
        || super::upstreams::UPSTREAMS_FILE_NAME.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    let temporary = dir.join(format!(
        ".{file_name}.{}-{}.tmp",
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));

    if let Err(err) = steps.write_new(&temporary, bytes) {
        // **在った一時ファイルは他人のもの**（`create_new` が断った）。消さない。
        if err.kind() != std::io::ErrorKind::AlreadyExists {
            discard(&temporary);
        }
        return Err(ReplaceError(err));
    }
    if let Err(err) = steps.replace(&temporary, path) {
        discard(&temporary);
        return Err(ReplaceError(err));
    }
    Ok(())
}

/// **一時ファイルを消す。** 消せなければ、場所を挙げてログに言う（応答には載せない）。
fn discard(temporary: &Path) {
    if let Err(err) = std::fs::remove_file(temporary)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            temporary = %temporary.display(),
            error = %err,
            "上流プロファイルの一時ファイルを消せませんでした"
        );
    }
}

// ----------------------------------------------------------------------
// 応答の形
// ----------------------------------------------------------------------

/// **読めない**（無いのとは別。権限・文字コード）。
const UNREADABLE: (&str, &str) = (
    "unreadable",
    "上流プロファイルを読めませんでした（置き場所の権限か文字コードを確かめてください）",
);

/// **JSON として読めない。**
const NOT_JSON: (&str, &str) = ("not_json", "上流プロファイルが JSON として読めません");

/// **検証には通るが、画面の形に写せない**（[`Unrepresentable`]）。
const UNREPRESENTABLE: (&str, &str) = (
    "unrepresentable",
    "gguf の無い面に、画面では扱えない形の値が書かれています（手で直してください）",
);

/// **何が駄目だったか**（定数だけ。プロファイルの名前だけは利用者がファイルに書いた綴り）。
///
/// | 欄 | 中身 |
/// |---|---|
/// | `reason` | `unreadable` / `not_json` / `unrepresentable` / `ShapeReason::code` のどれか |
/// | `message` | その理由の定数の文面 |
/// | `profile` | **一覧（`GET`）でだけ**、止まったプロファイルの名前。保存（`PUT`）の応答には載せない |
/// | `face` / `key` | `main` / `sub`、[`FACE_KEYS`] の綴り（分からなければ `null`） |
///
/// **A-13a-1 の間は `reason` に `not_yet_editable` があった**（`context_tokens` を書いたファイル）。
/// **作業単位 A-13a-2 で画面が追いついたので外した**（見張り番は
/// `the_listing_reasons_are_three_constants_and_the_shape_reasons`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct ProblemView {
    #[schema(schema_with = problem_reason_schema)]
    reason: &'static str,
    message: &'static str,
    #[schema(required = true)]
    profile: Option<String>,
    #[schema(schema_with = problem_face_schema, required = true)]
    face: Option<&'static str>,
    #[schema(schema_with = problem_key_schema, required = true)]
    key: Option<&'static str>,
}

/// **`ProblemView::reason` に載りうる綴り全部**（作業単位 P-2a —— 仕様書の `enum` の元）。
///
/// **定数の綴りを写さない**——3つの定数と [`ShapeReason::ALL`] を通して作る。
/// **⚠ `ProblemView::constant` に4つ目の定数を渡すようにしたら、ここにも足すこと**
/// （足し忘れは、その理由を返す応答を `tests/openapi_responses.rs` で検証すれば赤になるが、
/// いまその応答を作る腕は無い）。
pub(super) fn problem_reasons() -> Vec<&'static str> {
    [UNREADABLE.0, NOT_JSON.0, UNREPRESENTABLE.0]
        .into_iter()
        .chain(ShapeReason::ALL.map(ShapeReason::code))
        .collect()
}

fn problem_reason_schema() -> Object {
    openapi::closed_string(problem_reasons())
}

fn problem_face_schema() -> Object {
    openapi::nullable_closed_string(FACE_NAMES)
}

fn problem_key_schema() -> Object {
    openapi::nullable_closed_string(FACE_KEYS)
}

/// **一覧の状態**（`ProfilesView::state` の綴り / 作業単位 P-2a で型にした —— 振る舞いは不変）。
///
/// **以前は綴りを `view_of` の中に直に書いていた。** 仕様書の `enum` を綴りの写しでなく
/// 定義から作るために、ここへ集めた（表は [`view_of`] の doc）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProfilesState {
    /// 置き場所が無い。
    None,
    /// 置き場所はあるが、ファイルが無い。
    Missing,
    /// 読めない（UTF-8 でない等）。
    Unreadable,
    /// 読めたが、検証に通らない／画面の形に写せない。
    Invalid,
    /// 読めた。
    Ok,
}

impl ProfilesState {
    /// **全部の値。** 数え漏れは `every_profiles_state_is_listed_in_all` が見る。
    pub(super) const ALL: [Self; 5] = [
        Self::None,
        Self::Missing,
        Self::Unreadable,
        Self::Invalid,
        Self::Ok,
    ];

    /// 応答の綴り。**画面が読む正**（`web/src/lib/api.ts` の `ProfilesView::state`）。
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Missing => "missing",
            Self::Unreadable => "unreadable",
            Self::Invalid => "invalid",
            Self::Ok => "ok",
        }
    }
}

fn profiles_state_schema() -> Object {
    openapi::closed_string(ProfilesState::ALL.map(ProfilesState::label))
}

/// **`GET /admin/upstream/profiles` の応答**（`PUT` の成功も同じ形）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct ProfilesView {
    #[schema(required = true)]
    path: Option<String>,
    editable: bool,
    #[schema(schema_with = profiles_state_schema)]
    state: &'static str,
    #[schema(required = true)]
    revision: Option<String>,
    #[schema(required = true)]
    problem: Option<ProblemView>,
    profiles: Vec<WrittenProfile>,
}

/// **読んだ結果**（[`view_of`] へ渡す）。
enum Reading {
    NoLocation,
    Missing,
    Unreadable,
    Bytes(Vec<u8>),
}

impl ProblemView {
    const fn constant((reason, message): (&'static str, &'static str)) -> Self {
        Self {
            reason,
            message,
            profile: None,
            face: None,
            key: None,
        }
    }

    /// **読み込みが止まった理由から組む。** 文面（`Display`）は使わない——利用者の綴りを含む。
    fn of(err: &ProfileError) -> Self {
        match err.shape() {
            Some(shape) => Self {
                reason: shape.reason().code(),
                message: shape.reason().message(),
                profile: shape.profile().map(str::to_owned),
                face: shape.face(),
                key: shape.key(),
            },
            None => match err {
                ProfileError::Json(_) => Self::constant(NOT_JSON),
                ProfileError::Unreadable(_) | ProfileError::Shape(_) | ProfileError::Unknown(_) => {
                    Self::constant(UNREADABLE)
                }
            },
        }
    }

    fn unrepresentable(at: Unrepresentable) -> Self {
        Self {
            face: at.face,
            key: at.key,
            ..Self::constant(UNREPRESENTABLE)
        }
    }
}

/// **読んだ結果から応答を組む。純粋関数**（読むのは [`read_location`]）。
///
/// | `state` | いつ | `revision` | `problem` | `profiles` |
/// |---|---|---|---|---|
/// | `none` | 置き場所が無い | `null` | `null` | 空 |
/// | `missing` | ファイルが無い | `null` | `null` | 空 |
/// | `unreadable` | 読めない（権限） | `null` | `unreadable` | 空 |
/// | `unreadable` | UTF-8 でない | 印 | `unreadable` | 空 |
/// | `invalid` | 読み込みの検証に通らない／画面の形に写せない | 印 | 理由 | 空 |
/// | `ok` | 読めた | 印 | `null` | 名前の順 |
fn view_of(location: &ProfilesLocation, reading: Reading, home: Option<&Path>) -> ProfilesView {
    let view = |state: ProfilesState, revision, problem, profiles| ProfilesView {
        path: location.path().map(|path| path.display().to_string()),
        editable: location.editable(),
        state: state.label(),
        revision,
        problem,
        profiles,
    };
    let bytes = match reading {
        Reading::NoLocation => return view(ProfilesState::None, None, None, Vec::new()),
        Reading::Missing => return view(ProfilesState::Missing, None, None, Vec::new()),
        Reading::Unreadable => {
            return view(
                ProfilesState::Unreadable,
                None,
                Some(ProblemView::constant(UNREADABLE)),
                Vec::new(),
            );
        }
        Reading::Bytes(bytes) => bytes,
    };
    let revision = revision_of(&bytes);
    // **UTF-8 でなければ「読めない」**（`load_profiles` の `read_to_string` と同じ判定）。
    //
    // **印は返す**（レビュー1周目の是正2）。`null` を返すと、画面がそれを添えた保存は
    // 「無いと思って送った」と読まれて 409 `changed` になり、**読み直しても抜けられない。**
    // 印が合えば、保存は UTF-8 でないことを見て 422 `unreadable` で断る。
    let Ok(text) = String::from_utf8(bytes) else {
        return view(
            ProfilesState::Unreadable,
            Some(revision),
            Some(ProblemView::constant(UNREADABLE)),
            Vec::new(),
        );
    };
    if let Err(err) = parse_profiles(&text, home) {
        return view(
            ProfilesState::Invalid,
            Some(revision),
            Some(ProblemView::of(&err)),
            Vec::new(),
        );
    }
    match written_profiles(&text) {
        Ok(profiles) => view(ProfilesState::Ok, Some(revision), None, profiles),
        Err(at) => view(
            ProfilesState::Invalid,
            Some(revision),
            Some(ProblemView::unrepresentable(at)),
            Vec::new(),
        ),
    }
}

fn read_location(location: &ProfilesLocation) -> Reading {
    let Some(path) = location.path() else {
        return Reading::NoLocation;
    };
    match std::fs::read(path) {
        Ok(bytes) => Reading::Bytes(bytes),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Reading::Missing,
        Err(err) => {
            tracing::warn!(
                upstream_profiles_path = %path.display(),
                error = %err,
                "上流プロファイルを読めませんでした"
            );
            Reading::Unreadable
        }
    }
}

// ----------------------------------------------------------------------
// 持ち主
// ----------------------------------------------------------------------

/// **一覧と保存の持ち主**（`AppState` が `Arc` で持つ）。
///
/// **書き手はこの実行ファイルの中で1つだけ**（錠）。
pub(crate) struct ProfileEditor {
    location: ProfilesLocation,
    lock: tokio::sync::Mutex<()>,
}

impl ProfileEditor {
    pub(crate) fn new(location: ProfilesLocation) -> Self {
        Self {
            location,
            lock: tokio::sync::Mutex::new(()),
        }
    }

    /// **いまのファイルを読んで、一覧を組む。**
    fn view(&self) -> ProfilesView {
        view_of(
            &self.location,
            read_location(&self.location),
            home_dir().as_deref(),
        )
    }

    /// **1つのプロファイルを足す・直す**（本物の手順で置き換える）。
    async fn save(&self, name: &str, request: SaveRequest) -> Result<ProfilesView, SaveError> {
        self.save_with(&RealSteps, name, request).await
    }

    /// **[`Self::save`] の本体。** 置き換えの手順だけを差し替えられる（失敗を注入する継ぎ目）。
    ///
    /// **錠は最初から最後まで持つ**——この実行ファイルの中で、2つの保存が
    /// 同じ印を見て互いを上書きする形を作らない。**錠の中で `await` するのは錠を取る1回だけ。**
    async fn save_with(
        &self,
        steps: &impl ReplaceSteps,
        name: &str,
        request: SaveRequest,
    ) -> Result<ProfilesView, SaveError> {
        let _writer = self.lock.lock().await;

        // 0. **書けるのは既定の置き場所だけ**（ADR-0042 §決定8）。
        let ProfilesLocation::Editable(path) = &self.location else {
            tracing::info!(
                upstream_profile = ?name,
                "この起動では上流プロファイルを画面から直せません（読むだけの置き場所）"
            );
            return Err(SaveError::Api(ApiError::upstream_profiles_read_only()));
        };

        // 1. **名前は `--upstream` が選べる綴りだけ。**
        if !accepts_name(name) {
            return Err(SaveError::Api(ApiError::upstream_profile_name_invalid()));
        }

        // 2. **画面が読んだときと同じ内容か。** 違えば上書きしない。
        let current = read_current(path)?;
        if current.as_deref().map(revision_of) != request.revision {
            tracing::info!(
                upstream_profiles_path = %path.display(),
                "上流プロファイルは画面が読んだ後に書き換えられていました（上書きしていません）"
            );
            return Err(SaveError::Api(ApiError::upstream_profiles_changed()));
        }
        let current_text = match current {
            None => None,
            Some(bytes) => Some(String::from_utf8(bytes).map_err(|_| {
                tracing::warn!(
                    upstream_profiles_path = %path.display(),
                    "上流プロファイルが UTF-8 ではありません（画面からは直しません）"
                );
                SaveError::Api(ApiError::upstream_profiles_unreadable())
            })?),
        };

        // 3〜4. **いまのファイルと、直した後のファイル全体を、読み込みと同じ関数に通す。**
        let home = home_dir();
        let edit = ProfileEdit {
            main: FaceEdit::from(request.main),
            sub: request.sub.map(FaceEdit::from),
        };
        let text = apply_edit(current_text.as_deref(), name, &edit, home.as_deref()).map_err(
            |err| match err {
                EditError::CurrentInvalid(err) => {
                    tracing::warn!(
                        upstream_profiles_path = %path.display(),
                        error = %err,
                        cause = ?std::error::Error::source(&err),
                        "いまの上流プロファイルが検証に通りません（画面からは直しません）"
                    );
                    SaveError::Api(ApiError::upstream_profiles_unreadable())
                }
                EditError::CurrentUnrepresentable => {
                    tracing::warn!(
                        upstream_profiles_path = %path.display(),
                        "いまの上流プロファイルに画面で扱えない値があります（画面からは直しません）"
                    );
                    SaveError::Api(ApiError::upstream_profiles_unreadable())
                }
                EditError::NameRejected => {
                    SaveError::Api(ApiError::upstream_profile_name_invalid())
                }
                EditError::Rejected(err) => {
                    tracing::info!(
                        upstream_profile = ?name,
                        error = %err,
                        cause = ?std::error::Error::source(&err),
                        "直した上流プロファイルが検証に通りません（保存していません）"
                    );
                    // **保存の応答にはプロファイルの名前を載せない**（依頼の反射になる）。
                    // **止まるのは必ず直したプロファイル**——他の行は 3 で検証済み。
                    SaveError::Invalid(ProblemView {
                        profile: None,
                        ..ProblemView::of(&err)
                    })
                }
                EditError::Serialize(err) => {
                    tracing::error!(error = %err, "上流プロファイルを JSON に書き出せませんでした");
                    SaveError::Api(ApiError::internal())
                }
            },
        )?;

        // 5. **置き換えで書く。** 差し替える直前に、印をもう一度見る（窓を縮める）。
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|err| write_failed(path, &err))?;
        }
        if read_current(path)?.as_deref().map(revision_of) != request.revision {
            tracing::info!(
                upstream_profiles_path = %path.display(),
                "上流プロファイルは保存の途中で書き換えられました（上書きしていません）"
            );
            return Err(SaveError::Api(ApiError::upstream_profiles_changed()));
        }
        replace_file_with(steps, path, text.as_bytes()).map_err(|err| write_failed(path, &err))?;

        // 6. **読み直して、書いたものそのものであることを確かめる。**
        // **ここから先の失敗は「書き換わった後」**——元のファイルが残っているとは言わない。
        let reread = std::fs::read(path).map_err(|err| {
            tracing::error!(
                upstream_profiles_path = %path.display(),
                error = %err,
                "書いた直後の上流プロファイルを読み直せませんでした"
            );
            SaveError::Api(ApiError::upstream_profiles_verify_failed())
        })?;
        if reread != text.as_bytes() {
            tracing::error!(
                upstream_profiles_path = %path.display(),
                "書いた直後の上流プロファイルが、書いた内容と違います"
            );
            return Err(SaveError::Api(ApiError::upstream_profiles_verify_failed()));
        }
        let view = view_of(&self.location, Reading::Bytes(reread), home.as_deref());
        let saved = view.profiles.iter().any(|profile| {
            profile.name == name
                && edit.main.is_reflected_in(&profile.main)
                && match (&edit.sub, &profile.sub) {
                    (None, None) => true,
                    (Some(wanted), Some(listed)) => wanted.is_reflected_in(listed),
                    _ => false,
                }
        });
        // **⚠ この確認を外しても、テストは緑のまま**（レビュー1周目の是正の後に実測）。
        // ここへ来るのは、書き出し（`WrittenFace::to_value`）と読み込み（`written_profiles`）の
        // 形が食い違ったときだけで、その食い違いは保存 → 読み直しの見張り番
        // （`a_saved_profile_reads_back_as_it_was_sent` 等）が応答の中身で先に見つける。
        // **是正の前は、画面に写せない元のファイルを抱えた保存がここへ届いていた**（是正1で入口の 422 に移した）。
        if !saved {
            tracing::error!(
                upstream_profile = ?name,
                "書いた直後の上流プロファイルを読み直すと、依頼と違います"
            );
            return Err(SaveError::Api(ApiError::upstream_profiles_verify_failed()));
        }
        tracing::info!(
            upstream_profile = ?name,
            upstream_profiles_path = %path.display(),
            "上流プロファイルを保存しました"
        );
        Ok(view)
    }
}

/// **いまのファイルのバイト列。** 無ければ `None`。読めなければ 422（原因はログ）。
fn read_current(path: &Path) -> Result<Option<Vec<u8>>, SaveError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => {
            tracing::warn!(
                upstream_profiles_path = %path.display(),
                error = %err,
                "上流プロファイルを読めませんでした（保存していません）"
            );
            Err(SaveError::Api(ApiError::upstream_profiles_unreadable()))
        }
    }
}

/// **書けなかった**（原因と置き場所はログにだけ出す）。
fn write_failed(path: &Path, err: &dyn std::error::Error) -> SaveError {
    tracing::warn!(
        upstream_profiles_path = %path.display(),
        error = %err,
        cause = ?err.source(),
        "上流プロファイルを書き換えられませんでした（元のファイルは残っています）"
    );
    SaveError::Api(ApiError::upstream_profiles_write_failed())
}

// ----------------------------------------------------------------------
// ハンドラ
// ----------------------------------------------------------------------

/// **保存の依頼**（`PUT /admin/upstream/profiles/{name}`）。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SaveRequest {
    /// **読んだときの印**（`GET` の `revision`）。**ファイルが無かったなら `null`。**
    #[serde(default)]
    revision: Option<String>,
    main: WrittenFaceRequest,
    #[serde(default)]
    sub: Option<WrittenFaceRequest>,
}

/// **保存を断った理由**（応答）。
#[derive(Debug)]
pub(super) enum SaveError {
    Api(ApiError),
    /// **直した後のファイルが検証に通らない**（422 と、どこが駄目か）。
    Invalid(ProblemView),
}

impl IntoResponse for SaveError {
    fn into_response(self) -> Response {
        match self {
            Self::Api(error) => error.into_response(),
            // **載るのは定数だけ**（`ApiError::into_response_with` は `&'static str` しか受けない）。
            //
            // **⚠ 鍵の綴りは仕様書の [`ProfileInvalidDetail`] の欄名と同じでなければならない**
            // （写し / 実際の応答を仕様書で検証する `tests/openapi_responses.rs` の
            // `the_profile_endpoints_answer_as_documented_when_the_file_is_editable` が見る）。
            Self::Invalid(problem) => ApiError::upstream_profile_invalid().into_response_with(&[
                ("reason", Some(problem.reason)),
                ("reason_message", Some(problem.message)),
                ("face", problem.face),
                ("key", problem.key),
            ]),
        }
    }
}

// ----------------------------------------------------------------------
// 仕様書: 保存を断った 422（作業単位 P-2a のレビュー是正）
// ----------------------------------------------------------------------

/// **保存の 422 の本文**（仕様書のためだけに在る / `PUT /admin/upstream/profiles/{name}`）。
///
/// # 2つの形がある
///
/// | `code` | 形 | いつ |
/// |---|---|---|
/// | `upstream_profile_invalid` | [`ProfileInvalidBody`]（`error` の下に `reason` / `reason_message` / `face` / `key` が足される） | 直した後が検証に通らない |
/// | `upstream_profiles_unreadable` | `ErrorBody` のまま | いまのファイルが読めない・検証に通らない |
///
/// **`oneOf` で区別できるのは、`ErrorDetail` が余計な欄を許さないから**（`error` モジュールの doc）。
/// 許すと、足された形も `ErrorBody` の腕に当たり、「ちょうど1つ」を満たさない。
pub(super) struct SaveRejection;

impl utoipa::PartialSchema for SaveRejection {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::OneOfBuilder;
        use utoipa::openapi::{Ref, RefOr};
        OneOfBuilder::new()
            .item(RefOr::Ref(
                Ref::from_schema_name(ProfileInvalidBody::name()),
            ))
            .item(RefOr::Ref(Ref::from_schema_name(ErrorBody::name())))
            .into()
    }
}

impl ToSchema for SaveRejection {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("ProfileSaveRejection")
    }

    fn schemas(
        schemas: &mut Vec<(
            String,
            utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
        )>,
    ) {
        use utoipa::PartialSchema as _;
        schemas.push((
            ProfileInvalidBody::name().into_owned(),
            ProfileInvalidBody::schema(),
        ));
        schemas.push((ErrorBody::name().into_owned(), ErrorBody::schema()));
        ProfileInvalidBody::schemas(schemas);
        ErrorBody::schemas(schemas);
    }
}

/// **直した後が検証に通らなかった 422 の本文**（[`SaveRejection`] の1つ目の形）。**写し**
/// （本文を組むのは `ApiError::into_response_with` と [`SaveError`] の `into_response`）。
#[derive(ToSchema)]
#[expect(dead_code, reason = "仕様書のためだけの型")]
struct ProfileInvalidBody {
    error: ProfileInvalidDetail,
}

/// [`ProfileInvalidBody`] の `error`。**値の集合は定義から実行時に作る**（綴りを写さない）。
#[derive(ToSchema)]
#[expect(dead_code, reason = "仕様書のためだけの型")]
struct ProfileInvalidDetail {
    message: String,
    #[schema(rename = "type")]
    kind: String,
    #[schema(schema_with = invalid_code_schema)]
    code: String,
    /// **何が駄目だったか**（`ShapeReason::code` の綴り）。
    #[schema(schema_with = shape_reason_schema)]
    reason: String,
    /// その理由の定数の文面。
    reason_message: String,
    #[schema(schema_with = problem_face_schema, required = true)]
    face: Option<String>,
    #[schema(schema_with = problem_key_schema, required = true)]
    key: Option<String>,
}

/// `code` は1つだけ（**値は `ApiError::upstream_profile_invalid` の定数から**）。
fn invalid_code_schema() -> Object {
    openapi::closed_string(ApiError::upstream_profile_invalid().code())
}

/// 直した後の検証が止まる理由（`ShapeReason` の綴り）。
///
/// **`ProblemView::of` の、形の理由が無いときの落とし所（`not_json` / `unreadable`）は載せない** ——
/// 直した後のファイルは保存が `Value` から書いた JSON なので、JSON として読めない・読み込みが
/// 形でない理由で止まる、は起こらない（司令塔の指示 / レビュー 2026-09-30）。
fn shape_reason_schema() -> Object {
    openapi::closed_string(ShapeReason::ALL.map(ShapeReason::code))
}

/// `GET /admin/upstream/profiles` — **一覧・書けるか・置き場所・印。**
///
/// **失敗しない**（無い・読めない・検証に通らないは `state` と `problem` で言う / [`view_of`] の表）。
/// **gguf は書いたままの綴り**（`GET /admin/upstream` の観測した値とは別物）。
#[utoipa::path(
    get,
    path = "/admin/upstream/profiles",
    summary = "List the saved upstream profiles",
    tag = "upstream",
    responses(
        (status = 200, body = ProfilesView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn list_profiles(State(state): State<AppState>) -> Json<ProfilesView> {
    Json(state.profile_editor().view())
}

/// `PUT /admin/upstream/profiles/{name}` — **1つのプロファイルを足す・直す**（ADR-0042 §決定8）。
///
/// **本文**: `{ "revision": <GET の revision | null>, "main": 面, "sub": 面 | null }`
/// （面は `{ "endpoint", "model", "gguf"?, "args"?, "command"?, "context_tokens"?, "api_key"?, "allow_key_over_plain_http"? }`。
/// **省いた `sub` は「無い」**。**省いた `api_key` は「いまの鍵を残す」、`null` は「消す」**（[`KeyChange`]）。
/// **省いた（か `null` の）`allow_key_over_plain_http` は今の規則、`true` は承認する、`false` は外す**（[`WrittenFaceRequest`]））。
/// **返すのは `GET /admin/upstream/profiles` と同じ形**（保存した直後に読み直したもの）。
///
/// | 応答 | いつ |
/// |---|---|
/// | 200 | 保存した |
/// | 400 `invalid_body` | 本文が読めない・知らない欄 |
/// | 400 `upstream_profile_name_invalid` | `--upstream` で選べない名前 |
/// | 409 `upstream_profiles_read_only` | 読むだけの置き場所（`PERSONACORE_UPSTREAMS`）／置き場所が無い |
/// | 409 `upstream_profiles_changed` | 読んだ後に書き換えられていた（`revision` が違う） |
/// | 422 `upstream_profiles_unreadable` | いまのファイルが読めない・検証に通らない（手で直す） |
/// | 422 `upstream_profile_invalid` | 直した後が検証に通らない（`reason` / `reason_message` / `face` / `key`） |
/// | 500 `upstream_profiles_write_failed` | 書けなかった（**元のファイルは残る**） |
/// | 500 `upstream_profiles_verify_failed` | 書いた後に読み直すと違った（**ファイルは書き換わっている**。改名の直後に別の書き手が書いたとき等） |
///
/// **4xx と `upstream_profiles_write_failed` では、ファイルは1バイトも変わっていない。**
#[utoipa::path(
    put,
    path = "/admin/upstream/profiles/{name}",
    summary = "Create or replace one upstream profile",
    tag = "upstream",
    params(("name" = String, Path)),
    request_body = SaveRequest,
    responses(
        (status = 200, body = ProfilesView),
        // **本文の形が `ErrorBody` と違う回がある**（`SaveRejection` の doc）。
        (status = 422, body = SaveRejection),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn save_profile(
    State(state): State<AppState>,
    name: Result<UrlPath<String>, PathRejection>,
    body: Result<Json<SaveRequest>, JsonRejection>,
) -> Result<Json<ProfilesView>, SaveError> {
    let UrlPath(name) = name.map_err(|_| SaveError::Api(ApiError::invalid_path()))?;
    let Json(request) = body
        .map_err(|rejection| SaveError::Api(ApiError::from_json_rejection(rejection, ENDPOINT)))?;
    state.profile_editor().save(&name, request).await.map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(endpoint: &str, model: &str) -> WrittenFace {
        WrittenFace {
            endpoint: endpoint.to_owned(),
            model: model.to_owned(),
            gguf: None,
            args: None,
            command: None,
            context_tokens: None,
            api_key: None,
            api_key_set: false,
            api_key_hint: None,
            allow_key_over_plain_http: false,
        }
    }

    /// 本文の面に書く `context_tokens`。**既定（8192）とも落とし所（16384）とも、裏方の値とも違う。**
    const MAIN_WINDOW: u64 = 28_672;

    /// 裏方の面に書く `context_tokens`。**本文の値とも既定とも落とし所とも違う。**
    const SUB_WINDOW: u64 = 10_240;

    fn launched(
        endpoint: &str,
        model: &str,
        gguf: &str,
        args: &[&str],
        command: &str,
    ) -> WrittenFace {
        WrittenFace {
            endpoint: endpoint.to_owned(),
            model: model.to_owned(),
            gguf: Some(gguf.to_owned()),
            args: Some(args.iter().map(|arg| (*arg).to_owned()).collect()),
            command: Some(command.to_owned()),
            context_tokens: None,
            api_key: None,
            api_key_set: false,
            api_key_hint: None,
            allow_key_over_plain_http: false,
        }
    }

    /// **2つのプロファイル × 2面。値は全部違える**（帰属の見張り番の土台）。
    ///
    /// **`context_tokens` は `alpha` の本文と `beta` の裏方にだけ書く**（作業単位 A-13a-2）——
    /// プロファイルを取り違える実装も、面を取り違える実装も、値が出る場所が変わる。
    const TWO_BY_TWO: &str = r#"{
        "alpha": {
            "main": { "endpoint": "http://127.0.0.1:41001", "model": "alpha-main",
                      "gguf": "~/models/alpha-main.gguf", "args": ["--ctx-size", "4096"],
                      "command": "C:/tools/llama-a.exe", "context_tokens": 28672 },
            "sub":  { "endpoint": "http://127.0.0.1:41002", "model": "alpha-sub",
                      "gguf": "~/models/alpha-sub.gguf", "args": ["-ngl", "10"],
                      "command": "C:/tools/llama-b.exe" }
        },
        "beta": {
            "main": { "endpoint": "http://127.0.0.1:41003", "model": "beta-main" },
            "sub":  { "endpoint": "http://127.0.0.1:41004", "model": "beta-sub",
                      "gguf": "/opt/beta-sub.gguf", "context_tokens": 10240 }
        },
        "gamma": {
            "main": { "endpoint": "http://127.0.0.1:41005", "model": "gamma-main" }
        }
    }"#;

    // ---------------- 書いたままの値 ----------------

    /// **gguf は書いたままの綴りで返る**（`~` は `~` のまま。展開した値と混ぜない）。
    #[test]
    fn the_listing_returns_the_gguf_as_written_with_its_tilde() {
        let profiles = written_profiles(TWO_BY_TWO).expect("写せること");
        let alpha = profiles
            .iter()
            .find(|profile| profile.name == "alpha")
            .expect("alpha が在ること");
        assert_eq!(
            alpha.main.gguf.as_deref(),
            Some("~/models/alpha-main.gguf"),
            "書いたままの綴りでない"
        );
        assert_eq!(
            alpha.sub.as_ref().and_then(|s| s.gguf.as_deref()),
            Some("~/models/alpha-sub.gguf")
        );

        // **対照**: 同じ綴りを読み込みに通すと、`~` は展開される（＝ここは読み込みの値を使っていない）。
        let parsed =
            parse_profiles(TWO_BY_TWO, Some(Path::new("/planted-home"))).expect("読めること");
        let expanded = parsed
            .select("alpha")
            .expect("在ること")
            .main()
            .launch()
            .expect("起こす面であること")
            .gguf()
            .to_path_buf();
        assert_ne!(
            expanded,
            PathBuf::from("~/models/alpha-main.gguf"),
            "前提: 読み込みは ~ を展開すること"
        );
    }

    /// **全部の欄が、その面・そのプロファイルのものとして返る**（帰属）。
    #[test]
    fn every_field_comes_back_under_its_own_profile_and_face() {
        let profiles = written_profiles(TWO_BY_TWO).expect("写せること");
        assert_eq!(
            profiles,
            vec![
                WrittenProfile {
                    name: "alpha".to_owned(),
                    main: WrittenFace {
                        context_tokens: Some(MAIN_WINDOW),
                        ..launched(
                            "http://127.0.0.1:41001",
                            "alpha-main",
                            "~/models/alpha-main.gguf",
                            &["--ctx-size", "4096"],
                            "C:/tools/llama-a.exe",
                        )
                    },
                    sub: Some(launched(
                        "http://127.0.0.1:41002",
                        "alpha-sub",
                        "~/models/alpha-sub.gguf",
                        &["-ngl", "10"],
                        "C:/tools/llama-b.exe"
                    )),
                },
                WrittenProfile {
                    name: "beta".to_owned(),
                    main: face("http://127.0.0.1:41003", "beta-main"),
                    sub: Some(WrittenFace {
                        gguf: Some("/opt/beta-sub.gguf".to_owned()),
                        context_tokens: Some(SUB_WINDOW),
                        ..face("http://127.0.0.1:41004", "beta-sub")
                    }),
                },
                WrittenProfile {
                    name: "gamma".to_owned(),
                    main: face("http://127.0.0.1:41005", "gamma-main"),
                    sub: None,
                },
            ]
        );
    }

    /// **前後の空白も書いたまま**（読み込みは落として使う。ここは落とさない）。
    #[test]
    fn the_listing_keeps_the_spelling_as_written() {
        let source =
            r#"{ "p": { "main": { "endpoint": " http://127.0.0.1:1 ", "model": " m " } } }"#;
        parse_profiles(source, None).expect("前提: 読み込みは通ること");
        let profiles = written_profiles(source).expect("写せること");
        assert_eq!(profiles[0].main, face(" http://127.0.0.1:1 ", " m "));
    }

    /// **gguf の無い面の `args` は読み込みが見ないので、配列でなくても通る。そこは写せないと言う。**
    #[test]
    fn an_inert_value_that_the_screen_cannot_show_is_named() {
        let source = r#"{ "p": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m" },
                                 "sub": { "endpoint": "http://127.0.0.1:2", "model": "s", "args": "x" } } }"#;
        parse_profiles(source, None).expect("前提: 読み込みは通ること");
        assert_eq!(
            written_profiles(source),
            Err(Unrepresentable {
                face: Some("sub"),
                key: Some("args"),
            })
        );
        let command = r#"{ "p": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "command": 3 } } }"#;
        parse_profiles(command, None).expect("前提: 読み込みは通ること");
        assert_eq!(
            written_profiles(command),
            Err(Unrepresentable {
                face: Some("main"),
                key: Some("command"),
            })
        );
    }

    /// **`context_tokens` を書いたファイルも、一覧に出る**（作業単位 A-13a-2 / ADR-0054 §決定5）。
    ///
    /// **A-13a-1 の間は `not_yet_editable` で一覧ごと扱えなかった。** いまは `ok` で、
    /// 値は書いた面に、書いていない面は `None` で出る（帰属は
    /// `every_field_comes_back_under_its_own_profile_and_face` が丸ごと見る）。
    #[test]
    fn a_file_with_context_tokens_is_listed_with_the_value_on_its_face() {
        parse_profiles(TWO_BY_TWO, None).expect("前提: 読み込みは通ること");
        let view = view_of(
            &editable(),
            Reading::Bytes(TWO_BY_TWO.as_bytes().to_vec()),
            None,
        );
        assert_eq!((view.state, view.problem.clone()), ("ok", None));
        /// （名前・本文の値・裏方の値）。裏方の外側の `None` は「裏方の面が無い」。
        type Windows<'a> = (&'a str, Option<u64>, Option<Option<u64>>);
        let windows: Vec<Windows<'_>> = view
            .profiles
            .iter()
            .map(|profile| {
                (
                    profile.name.as_str(),
                    profile.main.context_tokens,
                    profile.sub.as_ref().map(|sub| sub.context_tokens),
                )
            })
            .collect();
        assert_eq!(
            windows,
            vec![
                ("alpha", Some(MAIN_WINDOW), Some(None)),
                ("beta", None, Some(Some(SUB_WINDOW))),
                ("gamma", None, None),
            ]
        );
    }

    /// **書いた `context_tokens` は、その面の鍵としてファイルに入る**（保存をバイト列で見る）。
    ///
    /// **期待値は直書き**。本文と裏方に違う値を書き、**どちらも既定（8192）・落とし所（16384）と違う**。
    /// 鍵の並びは `serde_json` の `Map`（並べ替え済み）が決める。
    #[test]
    fn context_tokens_are_written_on_their_own_face_byte_for_byte() {
        let written = apply_edit(
            None,
            "only",
            &edit(
                WrittenFace {
                    context_tokens: Some(MAIN_WINDOW),
                    ..launched(
                        "http://127.0.0.1:44001",
                        "only-main",
                        "~/models/only.gguf",
                        &["--ctx-size", "4096"],
                        "llama",
                    )
                },
                Some(WrittenFace {
                    context_tokens: Some(SUB_WINDOW),
                    ..face("http://127.0.0.1:44002", "only-sub")
                }),
            ),
            None,
        )
        .expect("書けること");
        assert_eq!(
            written,
            concat!(
                "{\n",
                "  \"only\": {\n",
                "    \"main\": {\n",
                "      \"args\": [\n",
                "        \"--ctx-size\",\n",
                "        \"4096\"\n",
                "      ],\n",
                "      \"command\": \"llama\",\n",
                "      \"context_tokens\": 28672,\n",
                "      \"endpoint\": \"http://127.0.0.1:44001\",\n",
                "      \"gguf\": \"~/models/only.gguf\",\n",
                "      \"model\": \"only-main\"\n",
                "    },\n",
                "    \"sub\": {\n",
                "      \"context_tokens\": 10240,\n",
                "      \"endpoint\": \"http://127.0.0.1:44002\",\n",
                "      \"model\": \"only-sub\"\n",
                "    }\n",
                "  }\n",
                "}\n",
            )
        );

        // **書いたファイルは読み込みの検証を通り、その値が面ごとの明示になる**（ADR-0054 §決定1）。
        let parsed = parse_profiles(&written, None).expect("検証に通ること");
        let settings = parsed.select("only").expect("在ること").settings();
        assert_eq!(
            (settings.context_tokens(), settings.sub_context_tokens()),
            (
                u32::try_from(MAIN_WINDOW).ok(),
                u32::try_from(SUB_WINDOW).ok()
            )
        );
    }

    /// **空にすると、鍵ごと消える**（`null` で残さない。読み込みは `"context_tokens": null` を止める）。
    ///
    /// **値を書いてあった `alpha` の本文を、値無しで直す**——同じファイルの `beta` の裏方の値は残る。
    #[test]
    fn emptying_context_tokens_removes_the_key_and_leaves_the_other_profile_alone() {
        let emptied = edit(
            launched(
                "http://127.0.0.1:41001",
                "alpha-main",
                "~/models/alpha-main.gguf",
                &["--ctx-size", "4096"],
                "C:/tools/llama-a.exe",
            ),
            None,
        );
        let written = apply_edit(Some(TWO_BY_TWO), "alpha", &emptied, None).expect("直せること");
        let after: Value = serde_json::from_str(&written).expect("JSON");
        let before: Value = serde_json::from_str(TWO_BY_TWO).expect("JSON");
        assert_eq!(
            before["alpha"]["main"].get("context_tokens"),
            Some(&Value::from(MAIN_WINDOW)),
            "前提: 直す前は書いてあること"
        );
        assert_eq!(after["alpha"]["main"].get("context_tokens"), None);
        assert_eq!(after["beta"], before["beta"], "beta が変わっている");

        let parsed = parse_profiles(&written, None).expect("検証に通ること");
        assert_eq!(
            parsed
                .select("alpha")
                .expect("在ること")
                .settings()
                .context_tokens(),
            None
        );
    }

    /// **使えない値は書かずに理由を返す**（読み込みと同じ規則 / ADR-0054 §決定1）。
    ///
    /// **理由・面・鍵は定数**で、鍵は `context_tokens`（画面がその欄に結びつける）。
    /// 応答の余白（1024）ちょうど・0・`u32` に入らない値の3つ。**止まる側を `sub` に置く腕も**
    /// （面を取り違える実装を赤にする）。
    #[test]
    fn an_unusable_context_tokens_is_rejected_with_its_face_and_key() {
        let at = |main: Option<u64>, sub: Option<u64>| {
            edit(
                WrittenFace {
                    context_tokens: main,
                    ..face("http://127.0.0.1:1", "m")
                },
                Some(WrittenFace {
                    context_tokens: sub,
                    ..face("http://127.0.0.1:2", "s")
                }),
            )
        };
        let overflow = u64::from(u32::MAX) + 1;
        for (edit, face_label) in [
            (at(Some(1024), None), "main"),
            (at(Some(0), Some(SUB_WINDOW)), "main"),
            (at(Some(overflow), None), "main"),
            (at(Some(MAIN_WINDOW), Some(1024)), "sub"),
        ] {
            let err = apply_edit(Some(TWO_BY_TWO), "beta", &edit, None).expect_err("止まること");
            let EditError::Rejected(err) = err else {
                panic!("検証で止まること: {err:?}");
            };
            let shape = err.shape().expect("形の不正であること");
            assert_eq!(
                (shape.reason().code(), shape.face(), shape.key()),
                (
                    "context_tokens_unusable",
                    Some(face_label),
                    Some("context_tokens")
                ),
                "{edit:?}"
            );
        }

        // **対照**: 余白より1大きければ通る（下の境界の内側）。
        assert!(apply_edit(Some(TWO_BY_TWO), "beta", &at(Some(1025), None), None).is_ok());
        // **対照**: `u32::MAX` ちょうども通る（上の境界の内側 / A-13a-2 のレビュー是正）。
        // 上の `overflow`（`u32::MAX + 1`）と対にして、境界を1つ内側へずらす実装を赤にする。
        let written = apply_edit(
            Some(TWO_BY_TWO),
            "beta",
            &at(Some(u64::from(u32::MAX)), Some(u64::from(u32::MAX))),
            None,
        )
        .expect("u32::MAX は通ること");
        let settings = parse_profiles(&written, None)
            .expect("検証に通ること")
            .select("beta")
            .expect("在ること")
            .settings();
        assert_eq!(
            (settings.context_tokens(), settings.sub_context_tokens()),
            (Some(u32::MAX), Some(u32::MAX))
        );
    }

    /// **検証を通っていない入力でも panic しない**（`written_profiles` の前提が崩れた形）。
    /// 整数でない `context_tokens` は、その鍵を名指しして写せないと言う。
    #[test]
    fn a_context_tokens_that_is_not_an_integer_is_named_without_panicking() {
        for raw in ["\"32768\"", "-1", "1.5", "null"] {
            let source = format!(
                r#"{{ "p": {{ "main": {{ "endpoint": "http://127.0.0.1:1", "model": "m", "context_tokens": {raw} }} }} }}"#
            );
            assert!(parse_profiles(&source, None).is_err(), "前提: {source}");
            assert_eq!(
                written_profiles(&source),
                Err(Unrepresentable {
                    face: Some("main"),
                    key: Some("context_tokens"),
                }),
                "{source}"
            );
        }
    }

    // ---------------- 印 ----------------

    /// **印は同じ内容で同じ、1バイト違えば違う**（改行の違いも含む）。
    #[test]
    fn the_revision_follows_every_byte() {
        let a = revision_of(b"{\"a\":1}\n");
        assert!(!a.is_empty(), "印が空");
        assert_eq!(a, revision_of(b"{\"a\":1}\n"), "同じ内容で違う印");
        for other in [
            &b"{\"a\":2}\n"[..],
            &b"{\"a\":1}\r\n"[..],
            &b"{\"a\":1}"[..],
            &b""[..],
        ] {
            assert_ne!(a, revision_of(other), "{other:?}");
        }
    }

    // ---------------- 名前 ----------------

    /// **名前は `--upstream` が受け付ける綴りだけ。**
    #[test]
    fn a_name_is_accepted_only_if_the_flag_would_select_it() {
        for name in ["qwen-official", "a b", "日本語", "-x"] {
            assert!(accepts_name(name), "{name}");
        }
        for name in ["", "   ", " padded", "padded ", "--fresh", "--x"] {
            assert!(!accepts_name(name), "{name:?}");
        }
        // **制御文字も断る**（作業単位 F-4f / レビュー1周目の是正3。判定は `parse_args` に任せている）。
        for name in ["x\nFORGED", "x\rFORGED", "x\u{1b}[31m", "a\tb"] {
            assert!(!accepts_name(name), "{name:?}");
        }
    }

    // ---------------- 足す・直す ----------------

    /// **鍵も承認も送らない（残す・今の規則）依頼**（A-13b-1 の前の依頼と同じ形）。
    fn edit(main: WrittenFace, sub: Option<WrittenFace>) -> ProfileEdit {
        let keep = |face: WrittenFace| FaceEdit {
            face,
            key: KeyChange::Keep,
            approval: None,
        };
        ProfileEdit {
            main: keep(main),
            sub: sub.map(keep),
        }
    }

    /// **1つを直しても、他のプロファイルの値は1つも変わらない**（帰属）。
    ///
    /// **3件のうち真ん中（`beta`）を直す**（端に置かない）。
    #[test]
    fn editing_one_profile_leaves_every_other_value_alone() {
        let next = edit(
            face("http://127.0.0.1:42003", "beta-main-2"),
            Some(launched(
                "http://127.0.0.1:42004",
                "beta-sub-2",
                "~/models/beta-sub-2.gguf",
                &["--ctx-size", "2048"],
                "llama",
            )),
        );
        let written = apply_edit(Some(TWO_BY_TWO), "beta", &next, None).expect("保存できること");

        let before: Value = serde_json::from_str(TWO_BY_TWO).expect("JSON");
        let after: Value = serde_json::from_str(&written).expect("JSON");
        assert_eq!(after["alpha"], before["alpha"], "alpha が変わっている");
        assert_eq!(after["gamma"], before["gamma"], "gamma が変わっている");
        assert_ne!(after["beta"], before["beta"], "前提: beta は変わること");

        let profiles = written_profiles(&written).expect("写せること");
        let beta = profiles
            .iter()
            .find(|p| p.name == "beta")
            .expect("在ること");
        assert_eq!(
            (&beta.main, beta.sub.as_ref()),
            (&next.main.face, next.sub.as_ref().map(|sub| &sub.face))
        );
        assert_eq!(
            profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["alpha", "beta", "gamma"]
        );
    }

    /// **無い名前は足す。`sub` を省けば `sub` の無いプロファイルになる**（直す＝丸ごと置き換える）。
    #[test]
    fn a_new_name_is_added_and_an_omitted_sub_is_removed() {
        let added = apply_edit(
            Some(TWO_BY_TWO),
            "delta",
            &edit(face("http://127.0.0.1:43001", "delta-main"), None),
            None,
        )
        .expect("足せること");
        let profiles = written_profiles(&added).expect("写せること");
        assert_eq!(profiles.len(), 4);
        assert!(
            profiles
                .iter()
                .any(|p| p.name == "delta" && p.sub.is_none())
        );

        let trimmed = apply_edit(
            Some(TWO_BY_TWO),
            "alpha",
            &edit(face("http://127.0.0.1:43002", "alpha-only"), None),
            None,
        )
        .expect("直せること");
        let after: Value = serde_json::from_str(&trimmed).expect("JSON");
        assert_eq!(after["alpha"].get("sub"), None, "sub が残っている");
        // **書かない鍵は `null` で残さない**（読み込みは `"gguf": null` を止める）。
        assert_eq!(after["alpha"]["main"].get("gguf"), None);
        assert_eq!(after["alpha"]["main"].get("args"), None);
        assert_eq!(after["alpha"]["main"].get("command"), None);
    }

    /// 承認の是正のテストで使う、宛先の外れた3つの面（鍵ありで承認なし・鍵ありで承認あり・鍵なしで承認あり）。
    const APPROVALS: &str = r#"{
        "far": {
            "main": { "endpoint": "http://192.0.2.41:47911", "model": "far-main",
                      "api_key": "pc-far-main-key-FM01", "allow_key_over_plain_http": true },
            "sub":  { "endpoint": "http://192.0.2.42:47912", "model": "far-sub",
                      "allow_key_over_plain_http": true }
        },
        "near": {
            "main": { "endpoint": "http://192.0.2.43:47913", "model": "near-main",
                      "api_key": "pc-near-main-key-NM02" }
        }
    }"#;

    /// **保存の後に鍵の無い面には、承認を書かない**（作業単位 A-13b-3 のレビュー是正 / 司令塔の決定）。
    ///
    /// 鍵の無い面に承認が残ると、後で同じ宛先に鍵を足したとき、確認なしで承認済みとして送られる。
    /// **断らずに落とす**（422 にしない）。
    ///
    /// | 面 | 鍵 | 承認の依頼 | 書かれる承認 |
    /// |---|---|---|---|
    /// | `far` の本文 | **消す**（`null`） | `true` | **無し** |
    /// | `far` の裏方 | 無いまま（手で書いた承認だけがある） | 送らない・宛先は同じ | **無し**（今の規則でも、鍵の無い面には残さない） |
    /// | `near` の本文（対照） | 残す | `true` | `true` |
    #[test]
    fn an_approval_is_dropped_on_a_face_left_without_a_key() {
        let approving = |face: WrittenFace, key: KeyChange, approval: Option<bool>| FaceEdit {
            face,
            key,
            approval,
        };
        let far = ProfileEdit {
            main: approving(
                face("http://192.0.2.41:47911", "far-main"),
                KeyChange::Clear,
                Some(true),
            ),
            sub: Some(approving(
                face("http://192.0.2.42:47912", "far-sub"),
                KeyChange::Keep,
                None,
            )),
        };
        let written = apply_edit(Some(APPROVALS), "far", &far, None).expect("保存できること");
        let after: Value = serde_json::from_str(&written).expect("JSON");
        assert_eq!(
            (
                after["far"]["main"].get("allow_key_over_plain_http"),
                after["far"]["sub"].get("allow_key_over_plain_http"),
                after["far"]["main"].get("api_key"),
            ),
            (None, None, None),
            "鍵の無い面に承認が残った: {after}"
        );

        // 対照: 鍵の残る面への `true` は書く。
        let near = ProfileEdit {
            main: approving(
                face("http://192.0.2.43:47913", "near-main"),
                KeyChange::Keep,
                Some(true),
            ),
            sub: None,
        };
        let written = apply_edit(Some(APPROVALS), "near", &near, None).expect("保存できること");
        let after: Value = serde_json::from_str(&written).expect("JSON");
        assert_eq!(
            after["near"]["main"]["allow_key_over_plain_http"],
            Value::Bool(true)
        );
    }

    /// **保存の後の読み直しの確認は、送った承認を照合する**（作業単位 A-13b-3 のレビュー是正 / 変異 R4 を塞ぐ）。
    ///
    /// 承認を `true` で送ったのに読み直した一覧が `false` なら、反映されていない。
    /// **鍵の無い面では、`true` を送っても `false` が正しい**（上の [`an_approval_is_dropped_on_a_face_left_without_a_key`]）。
    #[test]
    fn the_reread_check_compares_the_approval_that_was_sent() {
        let key = ApiKey::parse("pc-reread-key-RR03").expect("使える鍵");
        let sent = FaceEdit {
            face: face("http://192.0.2.44:47914", "reread"),
            key: KeyChange::Keep,
            approval: Some(true),
        };
        let listed = |approved: bool, key: Option<ApiKey>| WrittenFace {
            allow_key_over_plain_http: approved,
            ..face("http://192.0.2.44:47914", "reread").with_api_key(key)
        };
        assert!(
            !sent.is_reflected_in(&listed(false, Some(key.clone()))),
            "承認が書かれていないのに、反映されたと読んだ"
        );
        assert!(sent.is_reflected_in(&listed(true, Some(key))));
        assert!(
            sent.is_reflected_in(&listed(false, None)),
            "鍵の無い面で承認を落としたのを、反映されていないと読んだ"
        );
        // 送らなければ見ない。
        let unsent = FaceEdit {
            approval: None,
            ..sent
        };
        assert!(unsent.is_reflected_in(&listed(false, None)));
    }

    /// **`context_tokens` を書いていない面は、A-13a-2 の前と1バイトも違わない形で書く**
    /// （作業単位 A-13a-2 の対照）。
    ///
    /// **期待値は直書き**（A-13a-2 の前のコードで出したバイト列 / 検査対象の関数から作らない）。
    /// 起こす面（`context_tokens` 以外の5つの鍵を全部書いた）と、外の上流の面（2つだけ）の両方を並べる。
    #[test]
    fn a_face_without_context_tokens_is_written_byte_for_byte_as_before() {
        let written = apply_edit(
            None,
            "only",
            &edit(
                launched(
                    "http://127.0.0.1:44001",
                    "only-main",
                    "~/models/only.gguf",
                    &["--ctx-size", "4096"],
                    "llama",
                ),
                Some(face("http://127.0.0.1:44002", "only-sub")),
            ),
            None,
        )
        .expect("書けること");
        assert_eq!(
            written,
            concat!(
                "{\n",
                "  \"only\": {\n",
                "    \"main\": {\n",
                "      \"args\": [\n",
                "        \"--ctx-size\",\n",
                "        \"4096\"\n",
                "      ],\n",
                "      \"command\": \"llama\",\n",
                "      \"endpoint\": \"http://127.0.0.1:44001\",\n",
                "      \"gguf\": \"~/models/only.gguf\",\n",
                "      \"model\": \"only-main\"\n",
                "    },\n",
                "    \"sub\": {\n",
                "      \"endpoint\": \"http://127.0.0.1:44002\",\n",
                "      \"model\": \"only-sub\"\n",
                "    }\n",
                "  }\n",
                "}\n",
            )
        );
    }

    /// **ファイルが無ければ、そのプロファイル1つだけのファイルになる。**
    #[test]
    fn a_missing_file_becomes_a_file_with_one_profile() {
        let written = apply_edit(
            None,
            "only",
            &edit(face("http://127.0.0.1:44001", "only-main"), None),
            None,
        )
        .expect("書けること");
        let profiles = written_profiles(&written).expect("写せること");
        assert_eq!(
            profiles,
            vec![WrittenProfile {
                name: "only".to_owned(),
                main: face("http://127.0.0.1:44001", "only-main"),
                sub: None,
            }]
        );
    }

    /// **直した後のファイルが検証に通らなければ、書かずに理由を返す**（ループバックでない宛先に gguf）。
    #[test]
    fn an_edit_that_fails_the_loader_is_rejected_with_its_reason() {
        let bad = edit(
            WrittenFace {
                gguf: Some("/x.gguf".to_owned()),
                ..face("http://192.168.0.9:1", "m")
            },
            None,
        );
        let err = apply_edit(Some(TWO_BY_TWO), "beta", &bad, None).expect_err("止まること");
        let EditError::Rejected(err) = err else {
            panic!("検証で止まること: {err:?}");
        };
        let shape = err.shape().expect("形の不正であること");
        assert_eq!(
            (shape.reason().code(), shape.face(), shape.key()),
            ("endpoint_not_loopback", Some("main"), Some("endpoint"))
        );

        // **args の --host も同じ関数で止まる**（`sub` 側で見る）。
        let host = edit(
            face("http://127.0.0.1:1", "m"),
            Some(launched(
                "http://127.0.0.1:2",
                "s",
                "/y.gguf",
                &["--host", "0.0.0.0"],
                "llama",
            )),
        );
        let err = apply_edit(Some(TWO_BY_TWO), "beta", &host, None).expect_err("止まること");
        let EditError::Rejected(err) = err else {
            panic!("検証で止まること: {err:?}");
        };
        let shape = err.shape().expect("形の不正であること");
        assert_eq!(
            (shape.reason().code(), shape.face(), shape.key()),
            ("reserved_serve_flag", Some("sub"), Some("args"))
        );
    }

    /// **いまのファイルが検証に通らなければ、画面からは直さない。**
    #[test]
    fn a_current_file_that_fails_the_loader_is_not_edited() {
        for broken in [
            "{ not json",
            r#"{ "a": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "ggml": "x" } } }"#,
        ] {
            let err = apply_edit(
                Some(broken),
                "b",
                &edit(face("http://127.0.0.1:2", "m"), None),
                None,
            )
            .expect_err("止まること");
            assert!(
                matches!(err, EditError::CurrentInvalid(_)),
                "{broken}: {err:?}"
            );
        }
    }

    /// **`--upstream` で選べない名前は、書かない。**
    #[test]
    fn a_name_the_flag_cannot_select_is_rejected() {
        let err = apply_edit(
            Some(TWO_BY_TWO),
            "--fresh",
            &edit(face("http://127.0.0.1:2", "m"), None),
            None,
        )
        .expect_err("止まること");
        assert!(matches!(err, EditError::NameRejected), "{err:?}");
    }

    // ---------------- 置き換えで書く ----------------

    /// ディレクトリの中身（名前の一覧）。
    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("読めること")
            .map(|entry| {
                entry
                    .expect("読めること")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// **置き換えで書いた後、一時ファイルは残らない。**
    #[test]
    fn a_replacement_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().expect("一時ディレクトリ");
        let path = dir.path().join("upstreams.json");
        std::fs::write(&path, b"old").expect("置けること");
        replace_file_with(&RealSteps, &path, b"new contents").expect("書けること");
        assert_eq!(std::fs::read(&path).expect("読めること"), b"new contents");
        assert_eq!(entries(dir.path()), vec!["upstreams.json".to_owned()]);

        // **無いファイルにも書ける。**
        let fresh = dir.path().join("fresh.json");
        replace_file_with(&RealSteps, &fresh, b"first").expect("書けること");
        assert_eq!(std::fs::read(&fresh).expect("読めること"), b"first");
        assert_eq!(
            entries(dir.path()),
            vec!["fresh.json".to_owned(), "upstreams.json".to_owned()]
        );
    }

    /// **途中で落ちる手順**（書く途中・置き換え）。
    struct Failing {
        at_write: bool,
    }

    impl ReplaceSteps for Failing {
        fn write_new(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
            if self.at_write {
                // **半分まで書いてから落ちる**（途中までの中身が残る形を作る）。
                std::fs::write(path, &bytes[..bytes.len() / 2])?;
                return Err(std::io::Error::other("planted write failure"));
            }
            RealSteps.write_new(path, bytes)
        }

        fn replace(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            let _ = (from, to);
            Err(std::io::Error::other("planted replace failure"))
        }
    }

    /// **書く途中でも置き換えでも、落ちたら元のファイルが1バイトも変わらず、一時ファイルも残らない。**
    #[test]
    fn a_failed_replacement_leaves_the_original_and_no_temporary_file() {
        for at_write in [true, false] {
            let dir = tempfile::tempdir().expect("一時ディレクトリ");
            let path = dir.path().join("upstreams.json");
            std::fs::write(&path, b"original bytes").expect("置けること");

            let err = replace_file_with(&Failing { at_write }, &path, b"replacement bytes")
                .expect_err("落ちること");
            assert_eq!(
                err.to_string(),
                "上流プロファイルを書き換えられませんでした"
            );

            assert_eq!(
                std::fs::read(&path).expect("読めること"),
                b"original bytes",
                "元のファイルが変わっている（at_write={at_write}）"
            );
            assert_eq!(
                entries(dir.path()),
                vec!["upstreams.json".to_owned()],
                "一時ファイルが残っている（at_write={at_write}）"
            );
        }
    }

    // ---------------- 一覧の形 ----------------

    fn editable() -> ProfilesLocation {
        ProfilesLocation::Editable(PathBuf::from("/planted/upstreams.json"))
    }

    /// **状態ごとの形**（無い・読めない・JSON でない・形が不正・写せない・読めた）。
    #[test]
    fn the_listing_names_each_state_with_a_constant_reason() {
        let none = view_of(&ProfilesLocation::None, Reading::NoLocation, None);
        assert_eq!(
            (none.state, none.editable, none.path.clone()),
            ("none", false, None)
        );

        let missing = view_of(&editable(), Reading::Missing, None);
        assert_eq!(
            (
                missing.state,
                missing.revision.clone(),
                missing.problem.clone()
            ),
            ("missing", None, None)
        );
        assert!(missing.editable);

        let unreadable = view_of(&editable(), Reading::Unreadable, None);
        assert_eq!(unreadable.state, "unreadable");
        assert_eq!(
            unreadable.problem.as_ref().map(|p| p.reason),
            Some("unreadable")
        );

        let not_utf8 = view_of(&editable(), Reading::Bytes(vec![0xff, 0xfe]), None);
        assert_eq!(not_utf8.state, "unreadable");

        let not_json = view_of(&editable(), Reading::Bytes(b"{ nope".to_vec()), None);
        assert_eq!(not_json.state, "invalid");
        assert_eq!(
            not_json.problem.as_ref().map(|p| p.reason),
            Some("not_json")
        );
        assert!(not_json.revision.is_some(), "読めた内容には印を付ける");
        assert!(not_json.profiles.is_empty());

        let bad = br#"{ "a": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m" } },
                        "b": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "ggml": "x" } } }"#;
        let invalid = view_of(&editable(), Reading::Bytes(bad.to_vec()), None);
        assert_eq!(invalid.state, "invalid");
        assert_eq!(
            invalid.problem,
            Some(ProblemView {
                reason: "unknown_key",
                message: super::super::upstreams::ShapeReason::UnknownKey.message(),
                profile: Some("b".to_owned()),
                face: Some("main"),
                key: None,
            })
        );

        let inert = br#"{ "p": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "args": 1 } } }"#;
        let unrepresentable = view_of(&editable(), Reading::Bytes(inert.to_vec()), None);
        assert_eq!(unrepresentable.state, "invalid");
        assert_eq!(
            unrepresentable
                .problem
                .as_ref()
                .map(|p| (p.reason, p.face, p.key)),
            Some(("unrepresentable", Some("main"), Some("args")))
        );

        let ok = view_of(
            &editable(),
            Reading::Bytes(TWO_BY_TWO.as_bytes().to_vec()),
            None,
        );
        assert_eq!(ok.state, "ok");
        assert_eq!(ok.problem, None);
        assert_eq!(ok.revision, Some(revision_of(TWO_BY_TWO.as_bytes())));
        assert_eq!(
            ok.profiles,
            written_profiles(TWO_BY_TWO).expect("写せること")
        );
        assert_eq!(
            ok.path.as_deref(),
            Some(
                Path::new("/planted/upstreams.json")
                    .display()
                    .to_string()
                    .as_str()
            )
        );
    }

    /// **読むだけの置き場所は `editable: false`。置き場所は言う。**
    #[test]
    fn a_read_only_location_says_so_and_still_names_the_file() {
        let view = view_of(
            &ProfilesLocation::ReadOnly(PathBuf::from("/planted/override.json")),
            Reading::Bytes(TWO_BY_TWO.as_bytes().to_vec()),
            None,
        );
        assert!(!view.editable);
        assert_eq!(view.state, "ok");
        assert_eq!(
            view.path,
            Some(Path::new("/planted/override.json").display().to_string())
        );
    }

    // ---------------- レビュー1周目の是正（作業単位 F-4b-3） ----------------

    /// 保存の依頼（名前以外）。
    fn request(revision: Option<String>, main: WrittenFace) -> SaveRequest {
        SaveRequest {
            revision,
            main: WrittenFaceRequest {
                endpoint: main.endpoint,
                model: main.model,
                gguf: main.gguf,
                args: main.args,
                command: main.command,
                context_tokens: main.context_tokens,
                api_key: KeyChange::Keep,
                allow_key_over_plain_http: None,
            },
            sub: None,
        }
    }

    /// **`ProfilesState::ALL` に数え漏れが無い**（網羅の `match`。ワイルドカードを足さないこと。
    /// `ALL` からは回さない —— `contains` が必ず真になる）。
    #[test]
    fn every_profiles_state_is_listed_in_all() {
        let listed = |state: ProfilesState| match state {
            ProfilesState::None
            | ProfilesState::Missing
            | ProfilesState::Unreadable
            | ProfilesState::Invalid
            | ProfilesState::Ok => ProfilesState::ALL.contains(&state),
        };
        for state in [
            ProfilesState::None,
            ProfilesState::Missing,
            ProfilesState::Unreadable,
            ProfilesState::Invalid,
            ProfilesState::Ok,
        ] {
            assert!(listed(state), "{state:?} が ALL に無い");
        }
        assert_eq!(ProfilesState::ALL.len(), 5);
    }

    /// **型にする前と同じ綴り**（直書き。画面の `api.ts` が読む綴り）。
    #[test]
    fn the_profiles_state_keeps_its_spellings() {
        assert_eq!(
            ProfilesState::ALL.map(ProfilesState::label),
            ["none", "missing", "unreadable", "invalid", "ok"]
        );
    }

    /// **一覧の理由の綴りは、形の理由のほかに3つだけ**（直書き）。
    ///
    /// **A-13a-1 の `not_yet_editable` は、作業単位 A-13a-2 で画面が `context_tokens` を
    /// 扱えるようになったので外した。** 仕様書の `enum`（`ProblemView.reason`）はこの関数から作るので、
    /// ここが戻れば仕様書にも戻る。
    #[test]
    fn the_listing_reasons_are_three_constants_and_the_shape_reasons() {
        let reasons = problem_reasons();
        assert_eq!(
            reasons[..3],
            ["unreadable", "not_json", "unrepresentable"],
            "{reasons:?}"
        );
        assert_eq!(reasons.len(), 3 + ShapeReason::ALL.len(), "{reasons:?}");
        assert!(!reasons.contains(&"not_yet_editable"), "{reasons:?}");
    }

    /// 保存を断った理由の `ApiError`（`Invalid` なら `None`）。
    fn api_error(result: &Result<ProfilesView, SaveError>) -> Option<ApiError> {
        match result {
            Err(SaveError::Api(error)) => Some(*error),
            _ => None,
        }
    }

    /// **検証は通るが画面に写せないファイルには、別の名前でも書かない**（是正1 / 探針 P1）。
    ///
    /// **一覧は `invalid` と言う。保存も 422 で、ファイルは1バイトも変わらない**——
    /// 是正前は置き換えまで進み、書き換えた後に 500 `verify_failed` を返していた。
    #[tokio::test]
    async fn a_file_the_screen_cannot_show_is_not_rewritten_under_another_name() {
        let dir = tempfile::tempdir().expect("一時ディレクトリ");
        let path = dir.path().join("upstreams.json");
        let source = r#"{ "p": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m" },
                                 "sub": { "endpoint": "http://127.0.0.1:2", "model": "s", "args": "x" } } }"#;
        std::fs::write(&path, source).expect("置けること");
        parse_profiles(source, None).expect("前提: 読み込みは通ること");
        let editor = ProfileEditor::new(ProfilesLocation::Editable(path.clone()));

        let listing = editor.view();
        assert_eq!(listing.state, "invalid");
        assert!(listing.revision.is_some(), "前提: 印は返ること");

        let result = editor
            .save_with(
                &RealSteps,
                "q",
                request(listing.revision, face("http://127.0.0.1:3", "q-main")),
            )
            .await;
        assert_eq!(
            api_error(&result),
            Some(ApiError::upstream_profiles_unreadable()),
            "{result:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("読めること"),
            source,
            "書けない形のファイルを書き換えている"
        );
        assert_eq!(entries(dir.path()), vec!["upstreams.json".to_owned()]);
    }

    /// **UTF-8 でないファイルにも印を返し、保存は 422 で断る**（是正2 / 探針 P2）。
    ///
    /// 是正前は一覧の `revision` が `null` で、`null` で送ると 409 `changed` が返り続けた
    /// ——**読み直しても抜けられない。**
    #[tokio::test]
    async fn a_file_that_is_not_utf8_is_refused_as_unreadable_not_as_changed() {
        let dir = tempfile::tempdir().expect("一時ディレクトリ");
        let path = dir.path().join("upstreams.json");
        let bytes = b"{ \"p\": \xff\xfe }".to_vec();
        std::fs::write(&path, &bytes).expect("置けること");
        let editor = ProfileEditor::new(ProfilesLocation::Editable(path.clone()));

        let listing = editor.view();
        assert_eq!(listing.state, "unreadable");
        assert_eq!(
            listing.revision,
            Some(revision_of(&bytes)),
            "印が返っていない"
        );

        let result = editor
            .save_with(
                &RealSteps,
                "q",
                request(listing.revision, face("http://127.0.0.1:3", "q-main")),
            )
            .await;
        assert_eq!(
            api_error(&result),
            Some(ApiError::upstream_profiles_unreadable()),
            "{result:?}"
        );
        assert_eq!(std::fs::read(&path).expect("読めること"), bytes);
    }

    /// **差し替えた直後に別の書き手が書いたら、500 `verify_failed` と言う**（提案 / 探針 P3）。
    ///
    /// 継ぎ目（`ReplaceSteps`）で「改名の直後に1バイト足す」書き手を作る。
    struct WriterAfterReplace;

    impl ReplaceSteps for WriterAfterReplace {
        fn write_new(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
            RealSteps.write_new(path, bytes)
        }

        fn replace(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            RealSteps.replace(from, to)?;
            let mut text = std::fs::read(to)?;
            text.push(b' ');
            std::fs::write(to, text)
        }
    }

    #[tokio::test]
    async fn a_writer_after_the_replacement_is_reported_as_verify_failed() {
        let dir = tempfile::tempdir().expect("一時ディレクトリ");
        let path = dir.path().join("upstreams.json");
        std::fs::write(
            &path,
            r#"{ "p": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m" } } }"#,
        )
        .expect("置けること");
        let editor = ProfileEditor::new(ProfilesLocation::Editable(path.clone()));
        let listing = editor.view();

        let result = editor
            .save_with(
                &WriterAfterReplace,
                "q",
                request(listing.revision, face("http://127.0.0.1:3", "q-main")),
            )
            .await;
        assert_eq!(
            api_error(&result),
            Some(ApiError::upstream_profiles_verify_failed()),
            "{result:?}"
        );

        // **対照**: 本物の手順なら同じ依頼が通る（落ちた理由が割り込んだ書き手だと確かめる）。
        let listing = editor.view();
        let result = editor
            .save_with(
                &RealSteps,
                "q",
                request(listing.revision, face("http://127.0.0.1:3", "q-main")),
            )
            .await;
        assert!(result.is_ok(), "{result:?}");
    }

    /// **置き換えは改名である**（是正3 / ADR-0042 §決定8「置き換えで書く」/ 探針 P4）。
    ///
    /// 元のファイルを **READ|DELETE 共有**で掴んだまま置き換える。
    /// **改名なら通り、パスは新しい中身になり、掴んでいた側は古い中身を読む。**
    /// 宛先へ直接書く形・複製してから消す形は、掴んだファイルへの書き込みが共有違反（os error 32）で落ちる。
    #[cfg(windows)]
    #[test]
    fn the_replacement_is_a_rename_that_a_held_reader_does_not_see() {
        use std::io::Read as _;
        use std::os::windows::fs::OpenOptionsExt as _;

        /// `FILE_SHARE_READ` / `FILE_SHARE_DELETE`（Win32）。
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_DELETE: u32 = 0x0000_0004;

        let dir = tempfile::tempdir().expect("一時ディレクトリ");
        let path = dir.path().join("upstreams.json");
        std::fs::write(&path, b"old bytes").expect("置けること");
        let mut held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
            .open(&path)
            .expect("掴めること");

        replace_file_with(&RealSteps, &path, b"new bytes").expect("改名なら通ること");

        assert_eq!(std::fs::read(&path).expect("読めること"), b"new bytes");
        let mut old = Vec::new();
        held.read_to_end(&mut old).expect("読めること");
        assert_eq!(
            old, b"old bytes",
            "掴んでいた側が新しい中身を読んだ（その場で書いている）"
        );
        drop(held);
        assert_eq!(entries(dir.path()), vec!["upstreams.json".to_owned()]);
    }
}
