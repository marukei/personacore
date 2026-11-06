//! **API の仕様書（OpenAPI）をコードから生む**（[ADR-0047](../../docs/adr/0047-the-api-is-the-product.md) §② / 作業単位 P-2a）。
//!
//! # 何が載っているか
//!
//! **ルート表（`server::router`）の全部の口**（`/v1/*` と `/admin/*`）。
//! 口の一覧は [`ApiDoc`] の `paths(...)` に並べ、**ルート表との食い違いは
//! `the_route_table_and_the_document_list_the_same_operations` が赤にする**
//! （`mod.rs` の `.route(` を走査して、パスとメソッドの組を突き合わせる）。
//!
//! # `utoipa` の既定のままでは、仕様書がずれるところ（**第1段で実測**）
//!
//! | 既定 | 実際 | 補い方 | 見張り番 |
//! |---|---|---|---|
//! | 応答の `Option<T>` を「必須でない」と書く | **必ず `null` として載る** | `#[schema(required = true)]` | `a_response_never_omits_a_field_unless_it_is_named_here` |
//! | `skip_serializing_if` の `Option` を「`null` を許す」と書く | **欠落する。`null` にはならない** | `#[schema(nullable = false)]` | 同上 |
//! | `item_schema = T` は `T` を `components` に集めない（6.0.1） | 宙に浮いた `$ref` | `components(schemas(...))` | `every_reference_points_inside_the_document` |
//! | `IntoParams` の欄の型も `components` に集めない（6.0.1 / `MemoryListQuery::include`） | 宙に浮いた `$ref` | `#[param(inline)]` | 同上 |
//! | `#[serde(flatten)]` は `allOf: [{$ref}, {properties}]` になる | —— | 構造の走査は `allOf` の中も見る | `a_response_never_omits_…` の前提の assert |
//! | `&'static str` の欄は、値の集合が閉じていても素の `string` | 綴りは domain の定義が決める | `#[schema(schema_with = …)]` で `enum` を**実行時に**作る | `closed_values_come_from_their_definitions` |
//! | 同じ名前の型が2つあると、`components` の1つに**黙って畳む** | 別の型 | `#[schema(as = …)]` | `no_two_types_share_a_schema_name` |
//!
//! # 閉じた綴りの `enum` は、domain の定義から実行時に作る
//!
//! **仕様書のための enum に綴りを写さない。** 値の一覧は `ALL` / `label()` / 応答の綴りを決める
//! 関数（`admin::subject_to_wire` 等）を通して [`closed_string`] 等へ渡す。
//! 綴りを変えれば仕様書も変わり、**写しが古びる経路が無い。**
//!
//! # 説明文（`description`）は載せない
//!
//! **doc コメントは保守者向け**（開発機のパス・テスト名・変異の話が入る）なので、
//! 組み上がった仕様書から `description` を全部落とす（[`api_document`] の doc）。
//! 外向けの説明は英語の `summary`（P-2c）で、全部の口が持つ（`every_operation_has_an_english_summary`）。
//! 見張り番は `the_document_carries_no_description` と `the_document_names_no_path_of_this_machine`。
//!
//! **開いている欄**（`enum` を持たない文字列）は `tests::OPEN_STRINGS` に名指す。
//! 閉じた欄を足して `schema_with` を付け忘れると、そこが赤になる。
//!
//! # エラー
//!
//! **口ごとに `default` を `ErrorBody` にする**（司令塔の決定 / 2026-09-30）。
//! **状態コードを並べるのは handler が明示的に返す成功の応答だけ**——エラーの状態コードを
//! 手で並べると、handler の変更で黙って古びる（検証する手段が無い）。
//! **レイヤが返すエラー（本文の上限の 413・`Host` / `Origin` の検査の 403 / 作業単位 B-58）も
//! `default` が受ける**——どの口にも起こりうるので、口ごとに並べない
//! （403 が `default` の形であることは `tests/openapi_responses.rs` の
//! `errors_answer_with_the_documented_error_body` が見る）。
//!
//! # 本番の経路から呼ばれる
//!
//! [`api_document`] は `testing` フィーチャに寄りかからない（素の `cargo build` が門のまま）。
//! **配る口は `GET /openapi.json`**（[`get_document`] / P-2b）。**要求ごとに組み直す**
//! （起動時に作り置く形は採らない——起動を止める経路も、持ち置く状態も要らない）。
//!
//! # API の版（ADR-0047 §② P-2b）
//!
//! **[`API_VERSION`] が綴りの正（1箇所）。** 仕様書の `info.version`・仕様書の全部の応答の
//! `headers`（[`declare_version_header`]）と、**ルータに届いた要求への全部の応答**に付ける
//! ヘッダ `X-Personacore-Api-Version`（[`stamp_api_version`]）がここから読む。
//!
//! **⚠ HTTP として壊れた要求への 400（hyper が返す）には付かない**——ルータに届く前に
//! 答えが決まるので、レイヤを通らない。
//! **ADR-0047 の「いまの API の版」の行も走査で突き合わせる**（`the_adr_names_the_same_api_version`）。
//! **アプリの版（`Cargo.toml` の `version`）とは別に動く**——上げ方は ADR-0047 §② の P-2b の段。

use std::borrow::Cow;

use axum::Json;
use axum::http::{HeaderName, HeaderValue};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use utoipa::openapi::LicenseBuilder;
use utoipa::openapi::schema::{
    AnyOfBuilder, Array, ArrayBuilder, Object, ObjectBuilder, OneOfBuilder, Schema, SchemaType,
    Type,
};
use utoipa::openapi::{Ref, RefOr};
use utoipa::{OpenApi, PartialSchema, ToSchema};

use super::chat::{ASSISTANT_ROLE, finish_reason_schema};
use super::error::{ApiError, ErrorBody};
use super::relay::{CHUNK_OBJECT, DONE_DATA};
use super::{admin, chat, models, profile_editor, switch, upstream};

/// **実験中の口に付ける拡張の名前**（ADR-0047 §② P-2b「安定度を口ごとに名乗る」）。
///
/// **`deprecated` を使わない。** `deprecated` は「使うのをやめよ」で、
/// 実験中（「まだ形が変わりうる」）とは意味が逆向きになる。
/// **印を持つのは `axis-candidates` だけ**（司令塔の決定 / 2026-09-30）。
#[cfg(test)]
const STABILITY_EXTENSION: &str = "x-personacore-stability";

/// **API の版**（ADR-0047 §② P-2b）。**アプリの版とは別に持つ。綴りの正はここ1箇所。**
///
/// 仕様書の `info.version`（[`api_document`]）と、ルータに届いた要求への全部の応答のヘッダ
/// `X-Personacore-Api-Version`（[`stamp_api_version`]）がここを読む。
/// **上げ方は ADR-0047 §② の P-2b の段**（仕様書の差分を見て司令塔が決める）。
/// ADR の「いまの API の版」の行は `the_adr_names_the_same_api_version` が突き合わせる。
///
/// **`1.1`**（作業単位 A-10 / ADR-0049）: `PATCH` の任意の欄 `archived`、応答の欄 `archived_at`、
/// chat の新しい失敗 409 `model_archived`、`DELETE` の受け付ける範囲の拡大（しまったものは会話があっても消す）。
/// どれも ADR-0047 §② の表で小さい側。
///
/// **`1.2`**（作業単位 A-12a / ADR-0051）: 取り決め（約束・決まり）の口4本
/// （一覧・手で書く・手で終わらせる・履歴）と、その新しい失敗（409 `agreement_limit_reached` /
/// `agreement_not_active`、404 `agreement_not_found`、400 の理由）。口の追加なので小さい側。
/// 既存の口の形は1つも変えていない。
///
/// **`1.3`**（作業単位 A-12c-1 / ADR-0051 §決定2 —— δ3）: 約束・決まりの提案の採否の口2本
/// （採る `.../agreements/{agreement_id}/adoption`・捨てる `.../rejection`）、取り決めの状態に
/// `proposed` / `rejected`（提案は裏方が毎往復書き、一覧の口に提案中として載る）、新しい失敗
/// （409 `agreement_not_proposed`）。口と欄と値の追加なので小さい側。既存の口の形は変えていない
/// （`agreement_not_active` の文言だけを、提案中にも当たるように直した）。
///
/// **`1.4`**（作業単位 O-7 / Issue origin#91 がまとめて上げた）: v0.2.0（`1.3`）の後に形を変えた単位
/// （A-13a-1/2・F-10a/b・A-13b-1/2・B-66・B-71・B-61）が版を上げていなかった。**v0.3.0 / v0.3.1 は形が
/// 変わっていたのに `1.3` を名乗って出た。** 上流の口: `GET /admin/upstream`（と同じ形を返す口）の
/// `selected_profile`（F-10b）、面ごとの `unauthorized`（A-13b-1）・`key_over_plain_http`（A-13b-2）・
/// `checked_at` / `check_result`（B-66）、`launch.state` の値 `not_started`（F-10a。同じ名前の差し替えで起こす）、
/// 新しい口 `POST /admin/upstream/check`（B-66）。プロファイルの口: 面の `context_tokens`（A-13a-1/2）・
/// `api_key_set` / `api_key_hint`（A-13b-1）・`allow_key_over_plain_http`（A-13b-2）と、依頼の任意の欄
/// `context_tokens` / `api_key` / `allow_key_over_plain_http`、問題の `key` / `reason` の値。chat の新しい失敗
/// 502 `upstream_unauthorized`（A-13b-1。今までは `upstream_failed`）・502 `upstream_key_not_approved`（A-13b-2）。
/// 値の作り方の変更: 面の `endpoint` からクエリとフラグメントを落とす（読めない宛先は `<読めない宛先>`。B-71）、
/// `GET …/messages` の `dialogue` / `narration` から上流が本文に漏らした思考を外す（新しく書く行だけ。
/// `raw_content` はそのまま。B-61）。**どれも ADR-0047 §② の表で小さい側**（司令塔の決定 / O-7 のレビュー）。
/// 境目の2つの理由:
/// * **F-10a**（覚えた名前で起きたときは上流を起こさない。その間の chat は 503 `upstream_unavailable`）——
///   v0.2.0 のガイドは「起動時に覚えた選択で上流を起こす」と書いていたが、(a) 要求の形は変わらない、
///   (b) 503 `upstream_unavailable` は上流が起きていないときにその要求が返しうる応答としてもともとあった、
///   (c) 同じ名前の `POST /admin/upstream/profile` か `--upstream <名前>` で元の振る舞いに戻せる（ADR-0055 §決定4）。
/// * **A-13b-1** —— 既にあった条件（上流の 401 / 403）の `code` を `upstream_failed` から `upstream_unauthorized`
///   へ細分した（状態コード 502 と `type` は同じ）。ADR-0047 §② の表の「既にある条件の `code` を細分する」。
///
/// **この版から、上げ忘れは `tests/api_shape_fingerprint.rs` が赤にする**（版 → 形の指紋の表。
/// 失敗の形は `error::error_catalog` から畳み込む）。
pub const API_VERSION: &str = "1.4";

/// **ルータに届いた要求への全部の応答に付ける、API の版のヘッダの名前**
/// （HTTP では大文字小文字を区別しない。仕様書の `headers` の鍵もこの綴り）。
pub const API_VERSION_HEADER: &str = "x-personacore-api-version";

/// [`API_VERSION_HEADER`] を型にしたもの。**`from_static` は const 文脈で評価される**ので、
/// 綴りが不正ならビルドが止まる（実行中に panic する経路は無い）。
const API_VERSION_HEADER_NAME: HeaderName = HeaderName::from_static(API_VERSION_HEADER);

/// [`API_VERSION`] を型にしたもの（同上）。
const API_VERSION_HEADER_VALUE: HeaderValue = HeaderValue::from_static(API_VERSION);

/// **応答に API の版のヘッダを付ける**（ルート表のいちばん外のレイヤ / `server::router`）。
///
/// **レイヤで付ける**ので、handler の成功・エラーに加えて、フォールバック（API の 404・画面）・
/// 405・本文の上限（内側のレイヤが返す 413）・SSE・`/openapi.json` にも付く。
/// **HTTP として壊れた要求への hyper の 400 には付かない**（ルータに届く前に答えが決まる）。
/// **本文には触れない**（`/v1/chat/completions` の OpenAI 互換の本文は変わらない）。
/// 見張り番は `tests/server_api_version.rs::every_kind_of_response_carries_the_api_version`。
pub(super) async fn stamp_api_version(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(API_VERSION_HEADER_NAME, API_VERSION_HEADER_VALUE);
    response
}

/// **仕様書を配る口**（ADR-0047 §② P-2b）。**`--no-ui` でも配る**（画面ではなく API の一部）。
///
/// **要求ごとに組み直す**（モジュールの doc）。失敗したら 500 の `ErrorBody`
/// （[`document_response`]）。
#[utoipa::path(
    get,
    path = "/openapi.json",
    summary = "Get this OpenAPI document",
    tag = "meta",
    responses(
        (status = 200, body = OpenApiDocument),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn get_document() -> Response {
    document_response(api_document())
}

/// 組み上がった仕様書を応答にする。**失敗の詳細はログにだけ出す**（CLAUDE.md 絶対制約）。
pub(super) fn document_response(result: Result<Value, DocumentError>) -> Response {
    match result {
        Ok(document) => Json(document).into_response(),
        Err(err) => {
            tracing::error!(error = %err, detail = ?err, "API の仕様書を組み立てられませんでした");
            ApiError::internal().into_response()
        }
    }
}

/// **配る仕様書の形**（`GET /openapi.json` の 200）。**仕様書のためだけに在る。**
///
/// 中身は OpenAPI の文書そのもので、ここでは最上位の必須の欄だけを名乗る
/// （中身の型を写すと、写しが古びる）。**配った本文 == [`api_document`]** は
/// `tests/server_api_version.rs` が突き合わせる。
pub(super) struct OpenApiDocument;

impl PartialSchema for OpenApiDocument {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .schema_type(Type::Object)
            .required("openapi")
            .required("info")
            .required("paths")
            .into()
    }
}

impl ToSchema for OpenApiDocument {
    fn name() -> Cow<'static, str> {
        Cow::Borrowed("OpenApiDocument")
    }
}

/// 仕様書の元（`utoipa` の derive）。**口の一覧は `paths(...)` に並べる。**
///
/// **足し忘れは `the_route_table_and_the_document_list_the_same_operations` が赤にする。**
#[derive(OpenApi)]
#[openapi(
    version = "3.2.0",
    info(title = "personacore"),
    paths(
        models::list_models,
        chat::chat_completions,
        upstream::get_upstream,
        upstream::check_upstream,
        switch::switch_profile,
        profile_editor::list_profiles,
        profile_editor::save_profile,
        admin::list_characters,
        admin::create_character,
        admin::update_character,
        admin::delete_character,
        admin::replace_stats,
        admin::get_stats,
        admin::get_drift,
        admin::replace_attributes,
        admin::get_attributes,
        admin::get_attribute_revisions,
        admin::list_agreements,
        admin::create_agreement,
        admin::end_agreement,
        admin::adopt_agreement,
        admin::reject_agreement,
        admin::list_agreement_transitions,
        admin::get_axis_candidates,
        admin::list_memories,
        admin::correct_memory,
        admin::list_sessions,
        admin::list_session_messages,
        get_document,
    ),
    // **`item_schema` が指す先は `utoipa` が集めない**ので名指す
    // （`every_reference_points_inside_the_document` の doc）。
    components(schemas(ChatCompletionEvent))
)]
struct ApiDoc;

/// **仕様書を JSON にできなかった**（[`api_document`]）。
///
/// `Display` は定数（CLAUDE.md 絶対制約）。詳細は `#[source]`。
/// **起こらないはずの失敗**（`utoipa` の型は文字列の鍵しか持たない）だが、本番の経路で
/// `unwrap` しないために型で返す。配る口（[`get_document`]）はこれを 500 にする。
#[derive(Debug, thiserror::Error)]
#[error("API の仕様書を組み立てられませんでした")]
pub struct DocumentError(#[source] serde_json::Error);

/// **いまの仕様書**（OpenAPI 3.2 の JSON）。**呼ぶたびに組み直す**（持ち置きはしない）。
///
/// # 説明文（`description`）を全部落としてから返す（司令塔の決定の変更 / P-2a のレビュー 2026-09-30）
///
/// `utoipa` は **doc コメントをそのまま `description` にする。** doc コメントは保守者向けで、
/// **開発機のパス・テスト名・変異の話**が入っている（落とす前に数えた回で 209 件中 23 件）。
/// `/openapi.json` で配るので、**実行中のサーバがそれを返す。**
/// **`summary` は残す**——英語の要約（P-2c）はそちらに書いてある（外向けの説明）。
///
/// # なぜ `utoipa` の型ではなく JSON で返すのか
///
/// **`utoipa` の型は、自分が書いた JSON を読み戻せない**（6.0.0 で実測:
/// `data did not match any variant of untagged enum RefOr`）。型のまま落とすには
/// 説明文を持つ型を1つずつ辿る写しが要る。**組み上がった JSON から落とす**方が、
/// `utoipa` が型を足しても取りこぼさない（[`strip_descriptions`]）。
///
/// # Errors
///
/// 仕様書を JSON にできなかったとき（[`DocumentError`]）。
pub fn api_document() -> Result<Value, DocumentError> {
    document_with(env!("CARGO_PKG_LICENSE"), env!("CARGO_PKG_DESCRIPTION"))
}

/// [`api_document`] の本体。**ライセンスと説明を引数で受ける**（空の腕と載る腕を、
/// いまの `Cargo.toml` の値に関わらずテストできるように）。
///
/// # `info` に載せるもの（司令塔の決定 / P-2b）
///
/// * `title` と、`version` = [`API_VERSION`]（**`utoipa` の既定はアプリの版**）
/// * `license` —— `Cargo.toml` の `license`（SPDX の式）。**空なら載せない。**
///   OpenAPI 3.1 以降の `identifier` は SPDX の式を取るので、`name` と同じ綴りを入れる。
/// * `description` —— `Cargo.toml` の `description`。**空なら載せない。**
///   **説明文を全部落とした後に足す**（先に足すと [`strip_descriptions`] に消される）。
///
/// **全部の応答の `headers` に版のヘッダを足す**のもここ（[`declare_version_header`]）。
///
/// **それ以外（`contact` 等）は載せない**——`utoipa` は `Cargo.toml` の `authors` から
/// `contact` を作りうる（連絡先を配ることになる）。見張り番は `the_info_carries_only_the_named_keys`。
fn document_with(license: &str, description: &str) -> Result<Value, DocumentError> {
    let mut api = ApiDoc::openapi();
    api.info.version = API_VERSION.to_owned();
    api.info.contact = None;
    api.info.terms_of_service = None;
    api.info.description = None;
    let license = license.trim();
    api.info.license = (!license.is_empty()).then(|| {
        LicenseBuilder::new()
            .name(license)
            .identifier(Some(license))
            .build()
    });

    let mut document = serde_json::to_value(api).map_err(DocumentError)?;
    strip_descriptions(&mut document, false);
    declare_version_header(&mut document);

    let description = description.trim();
    if !description.is_empty()
        && let Some(info) = document.get_mut("info").and_then(Value::as_object_mut)
    {
        info.insert(
            "description".to_owned(),
            Value::String(description.to_owned()),
        );
    }
    Ok(document)
}

/// **仕様書の全部の応答（`default` を含む）に、版のヘッダを名乗らせる**（P-2b のレビュー）。
///
/// ヘッダは**ルータに届いた要求への全部の応答**に付く（[`stamp_api_version`]）。
/// 口ごとの注釈に書くと足し忘れが出るので、**組み上がった JSON の後処理で全部に足す**
/// （名前も値の例も定数から。写しを作らない）。
/// 見張り番は `every_documented_response_names_the_version_header`。
fn declare_version_header(document: &mut Value) {
    let Some(paths) = document.get_mut("paths").and_then(Value::as_object_mut) else {
        return;
    };
    for item in paths.values_mut() {
        let Some(item) = item.as_object_mut() else {
            continue;
        };
        for operation in item.values_mut() {
            let Some(responses) = operation
                .get_mut("responses")
                .and_then(Value::as_object_mut)
            else {
                continue;
            };
            for response in responses.values_mut() {
                let Some(response) = response.as_object_mut() else {
                    continue;
                };
                let headers = response
                    .entry("headers")
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if let Some(headers) = headers.as_object_mut() {
                    headers.insert(
                        API_VERSION_HEADER.to_owned(),
                        serde_json::json!({
                            "required": true,
                            "schema": {"type": "string", "examples": [API_VERSION]},
                        }),
                    );
                }
            }
        }
    }
}

/// **その下の鍵が「名前」である鍵**（欄の名前・型の名前・パス・状態コード・media type ほか）。
///
/// ここに挙げた鍵の直下では、`description` という鍵を**名前として**扱い、落とさない。
const NAME_MAPS: [&str; 10] = [
    "properties",
    "schemas",
    "paths",
    "responses",
    "content",
    "examples",
    "headers",
    "links",
    "callbacks",
    "parameters",
];

/// `value` の中の `description` の**キーワード**を全部落とす（**名前の `description` は残す**）。
///
/// `keys_are_names` は「この階層の鍵は名前か」（[`NAME_MAPS`] の直下）。
fn strip_descriptions(value: &mut Value, keys_are_names: bool) {
    strip_keywords(value, &["description"], keys_are_names);
}

/// `value` の中の `keywords` の**キーワード**を全部落とす（**名前の位置の同じ綴りは残す**）。
///
/// 名前かどうかの判定は [`NAME_MAPS`]（[`strip_descriptions`] と同じ1つの規則）。
fn strip_keywords(value: &mut Value, keywords: &[&str], keys_are_names: bool) {
    match value {
        Value::Object(map) => {
            if !keys_are_names {
                for keyword in keywords {
                    map.remove(*keyword);
                }
            }
            for (key, inner) in map.iter_mut() {
                strip_keywords(
                    inner,
                    keywords,
                    !keys_are_names && NAME_MAPS.contains(&key.as_str()),
                );
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_keywords(item, keywords, false);
            }
        }
        _ => {}
    }
}

/// **テスト専用**: 組み上がった仕様書から `keywords` のキーワードを落とす（名前の位置は残す）。
///
/// API の形の指紋（`tests/api_shape_fingerprint.rs` / 作業単位 O-7）が `example` / `examples` を
/// 落とすのに使う。**名前かどうかの規則を写さないために、本番の [`strip_descriptions`] と
/// 同じ関数を通す**（本番の入口を通る見張り番は `the_document_carries_no_description`）。
#[cfg(any(test, feature = "testing"))]
pub fn strip_keywords_for_tests(document: &mut Value, keywords: &[&str]) {
    strip_keywords(document, keywords, false);
}

// ----------------------------------------------------------------------
// 閉じた綴りの集合（**値は呼び出し側が domain の定義から渡す**）
// ----------------------------------------------------------------------

/// 重複を落とす（**最初に現れた順を保つ**）。
///
/// `AxisCandidateState::ALL` のように、1つの綴りに複数の値が畳まれる定義があるため。
fn distinct(values: impl IntoIterator<Item = &'static str>) -> Vec<&'static str> {
    let mut seen = Vec::new();
    for value in values {
        if !seen.contains(&value) {
            seen.push(value);
        }
    }
    seen
}

/// **閉じた綴りの集合を `enum` として名乗る文字列。**
///
/// 値は**呼び出し側が domain の定義から実行時に作って渡す**（綴りを写さない / モジュールの doc）。
pub(super) fn closed_string(values: impl IntoIterator<Item = &'static str>) -> Object {
    ObjectBuilder::new()
        .schema_type(Type::String)
        .enum_values(Some(distinct(values)))
        .build()
}

/// [`closed_string`] に **`null` を足したもの**（`Option<&'static str>` の欄）。
///
/// **`enum` にも `null` を入れる**——`type` だけに `null` を足すと、`enum` が `null` を弾く。
pub(super) fn nullable_closed_string(values: impl IntoIterator<Item = &'static str>) -> Object {
    let mut listed: Vec<Value> = distinct(values).into_iter().map(Value::from).collect();
    listed.push(Value::Null);
    ObjectBuilder::new()
        .schema_type(SchemaType::from_iter([Type::String, Type::Null]))
        .enum_values(Some(listed))
        .build()
}

/// [`closed_string`] の配列（`Vec<&'static str>` の欄）。
pub(super) fn closed_string_array(values: impl IntoIterator<Item = &'static str>) -> Array {
    ArrayBuilder::new().items(closed_string(values)).build()
}

// ----------------------------------------------------------------------
// SSE の1件（**写し**。中継の実出力と突き合わせる見張り番は `tests/openapi_responses.rs`）
// ----------------------------------------------------------------------

/// **SSE の1件**（`stream: true` の応答 / `relay` モジュール）。**仕様書のためだけに在る。**
///
/// # ⚠ これは写しである
///
/// **中継は `serde_json::Value` を手で組む**（`relay::Relay::relay`）ので、
/// 線に乗る形を決めている Rust の型が無い。**中継を型にする作り直しはしない**（司令塔の決定）。
/// 代わりに、**中継（モックの上流）が実際に出した各 `data` をこの形で検証する見張り番**を置いた
/// （`tests/openapi_responses.rs` の `every_relayed_event_matches_the_documented_event`）。
///
/// | `data:` の中身 | いつ |
/// |---|---|
/// | [`ChatCompletionChunk`] の JSON | 本文の増分 |
/// | `ErrorBody` の JSON | 途中で失敗した（その後に `[DONE]` は来ない） |
/// | `[DONE]`（JSON ではない） | 上流が尽きた |
pub(super) struct ChatCompletionEvent;

impl PartialSchema for ChatCompletionEvent {
    fn schema() -> RefOr<Schema> {
        let json_data = ObjectBuilder::new()
            .schema_type(Type::String)
            .content_media_type("application/json")
            .content_schema(Some(
                OneOfBuilder::new()
                    .item(RefOr::Ref(Ref::from_schema_name(
                        ChatCompletionChunk::name(),
                    )))
                    .item(RefOr::Ref(Ref::from_schema_name(ErrorBody::name()))),
            ));
        ObjectBuilder::new()
            .schema_type(Type::Object)
            // **`anyOf`**——`[DONE]` は上の腕（文字列）にも当たる。`oneOf` だと
            // 「ちょうど1つ」を満たさず、正しい終端が不正になる（テストの doc）。
            .property(
                "data",
                AnyOfBuilder::new()
                    .item(json_data)
                    .item(closed_string([DONE_DATA])),
            )
            .required("data")
            .into()
    }
}

impl ToSchema for ChatCompletionEvent {
    fn name() -> Cow<'static, str> {
        Cow::Borrowed("ChatCompletionEvent")
    }

    fn schemas(schemas: &mut Vec<(String, RefOr<Schema>)>) {
        for (name, schema) in [
            (ChatCompletionChunk::name(), ChatCompletionChunk::schema()),
            (ErrorBody::name(), ErrorBody::schema()),
        ] {
            schemas.push((name.into_owned(), schema));
        }
        ChatCompletionChunk::schemas(schemas);
        ErrorBody::schemas(schemas);
    }
}

fn chunk_object_schema() -> Object {
    closed_string([CHUNK_OBJECT])
}

fn assistant_role_schema() -> Object {
    closed_string([ASSISTANT_ROLE])
}

/// **SSE の本文の増分1件**（OpenAI 互換の `chat.completion.chunk`）。**写し**（[`ChatCompletionEvent`]）。
#[derive(ToSchema)]
#[expect(
    dead_code,
    reason = "仕様書のためだけの型。中継は serde_json::Value で組む"
)]
struct ChatCompletionChunk {
    id: String,
    #[schema(schema_with = chunk_object_schema)]
    object: String,
    created: i64,
    /// 着地したセッション（`aoi@<session-id>`）。
    model: String,
    choices: Vec<ChunkChoice>,
}

/// 候補1件。**常に1件。**
#[derive(ToSchema)]
#[expect(dead_code, reason = "仕様書のためだけの型")]
struct ChunkChoice {
    index: u32,
    delta: ChunkDelta,
    /// 上流が示さなかった場合と、こちらが綴りを決められない場合は `null`。
    #[schema(schema_with = finish_reason_schema, required = true)]
    finish_reason: Option<String>,
}

/// 増分。**先頭の1件だけが `role` を名乗る**（2件目以降はキーごと無い）。**`content` は毎回載る。**
#[derive(ToSchema)]
#[expect(dead_code, reason = "仕様書のためだけの型")]
struct ChunkDelta {
    #[schema(schema_with = assistant_role_schema, required = false)]
    role: Option<String>,
    content: String,
}

/// 英語の API ガイド（`docs/api/README.md`）の綴りの突き合わせ（P-2c）。
#[cfg(test)]
mod guide_tests;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::{Value, json};

    use super::{API_VERSION, STABILITY_EXTENSION, api_document};

    /// 仕様書を JSON にしたもの。**読み手（外のソフト）が見るのはこの形**なので、
    /// `utoipa` の型ではなく JSON で突き合わせる。
    fn document() -> Value {
        api_document().expect("仕様書は JSON にできること")
    }

    fn schema<'a>(document: &'a Value, name: &str) -> &'a Value {
        let found = &document["components"]["schemas"][name];
        assert!(found.is_object(), "components.schemas.{name} が無い");
        found
    }

    fn required(schema: &Value) -> Vec<&str> {
        schema["required"]
            .as_array()
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    /// `type` が `null` を含むか（OpenAPI 3.1 以降は `"type": ["string", "null"]` の形）。
    fn admits_null(property: &Value) -> bool {
        match &property["type"] {
            Value::Array(types) => types.iter().any(|kind| kind == "null"),
            Value::String(kind) => kind == "null",
            _ => property["oneOf"]
                .as_array()
                .is_some_and(|arms| arms.iter().any(|arm| arm["type"] == "null")),
        }
    }

    /// その型の欄を持つ部分（**自分自身と、`allOf` の中の `$ref` でない部分**）。
    ///
    /// **`#[serde(flatten)]` は `allOf: [{$ref}, {properties}]` になる**（実測 /
    /// `SupersededMemoryView`）ので、自分自身の `properties` だけを見ると、
    /// flatten した型が足した欄（`superseded_by`）を見落とす。
    fn parts(schema: &Value) -> Vec<&Value> {
        std::iter::once(schema)
            .chain(
                schema["allOf"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|part| part["$ref"].is_null()),
            )
            .collect()
    }

    /// 仕様書に載っている `(パス, メソッド)` の組。
    pub(super) fn documented_operations(document: &Value) -> BTreeSet<(String, String)> {
        const METHODS: [&str; 8] = [
            "get", "put", "post", "delete", "options", "head", "patch", "trace",
        ];
        let mut found = BTreeSet::new();
        for (path, item) in document["paths"].as_object().into_iter().flatten() {
            for method in METHODS {
                if item[method].is_object() {
                    found.insert((path.clone(), method.to_owned()));
                }
            }
        }
        found
    }

    #[test]
    fn the_document_is_openapi_3_2() {
        assert_eq!(document()["openapi"], json!("3.2.0"));
    }

    // ------------------------------------------------------------------
    // API の版・ライセンス・説明（ADR-0047 §② P-2b）
    // ------------------------------------------------------------------

    /// **仕様書の `info.version` は API の版**（アプリの版ではない）。
    ///
    /// `utoipa` の既定は `CARGO_PKG_VERSION` を入れる——**アプリの版と API の版を取り違える形。**
    #[test]
    fn the_document_says_the_api_version_not_the_app_version() {
        let document = document();
        assert_eq!(document["info"]["version"], json!(API_VERSION));
        assert_ne!(
            API_VERSION,
            env!("CARGO_PKG_VERSION"),
            "前提: 2つの版が同じ綴りだと、取り違えがこの検査で見えない"
        );
        assert_eq!(document["info"]["title"], json!("personacore"));
    }

    /// **`info` の鍵は、名指したものだけ**（`utoipa` が `Cargo.toml` から勝手に足す欄を配らない）。
    #[test]
    fn the_info_carries_only_the_named_keys() {
        let document = document();
        let mut keys: Vec<&str> = document["info"]
            .as_object()
            .expect("info はオブジェクト")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        let mut expected = vec!["title", "version"];
        if !env!("CARGO_PKG_LICENSE").trim().is_empty() {
            expected.push("license");
        }
        if !env!("CARGO_PKG_DESCRIPTION").trim().is_empty() {
            expected.push("description");
        }
        expected.sort_unstable();
        assert_eq!(keys, expected, "{}", document["info"]);
    }

    /// **ライセンスと説明は `Cargo.toml` から**（写しを作らない）——**いまの値の腕。**
    #[test]
    fn the_license_and_description_are_the_manifests() {
        let info = &document()["info"];
        let license = env!("CARGO_PKG_LICENSE").trim();
        if license.is_empty() {
            assert!(info.get("license").is_none(), "{info}");
        } else {
            assert_eq!(
                info["license"],
                json!({"name": license, "identifier": license})
            );
        }
        let description = env!("CARGO_PKG_DESCRIPTION").trim();
        if description.is_empty() {
            assert!(info.get("description").is_none(), "{info}");
        } else {
            assert_eq!(info["description"], json!(description));
        }
    }

    /// **両方の腕を、既定ではない値で**（いまの `Cargo.toml` は license が空 —— 載る腕を
    /// 実物だけで見ると、O-2 が入るまで一度も通らない）。
    ///
    /// **説明文は落とした後に足す**——落とす前に足すと `strip_descriptions` に消される。
    #[test]
    fn a_planted_license_and_description_reach_the_info_and_empty_ones_do_not() {
        let filled = super::document_with(
            "MIT OR Apache-2.0 OR LicenseRef-ACCRL-0.3",
            "planted description P2B",
        )
        .expect("組み立てられること");
        assert_eq!(
            filled["info"]["license"],
            json!({
                "name": "MIT OR Apache-2.0 OR LicenseRef-ACCRL-0.3",
                "identifier": "MIT OR Apache-2.0 OR LicenseRef-ACCRL-0.3",
            })
        );
        assert_eq!(
            filled["info"]["description"],
            json!("planted description P2B")
        );
        assert_eq!(filled["info"]["version"], json!(API_VERSION));

        for blank in ["", "  "] {
            let empty = super::document_with(blank, blank).expect("組み立てられること");
            assert!(empty["info"].get("license").is_none(), "{}", empty["info"]);
            assert!(
                empty["info"].get("description").is_none(),
                "{}",
                empty["info"]
            );
            assert_eq!(empty["info"]["version"], json!(API_VERSION));
        }
    }

    /// 仕様書の全部の応答（`default` を含む）の数と、そのうち **版のヘッダを正しく名乗らないもの**
    /// （`メソッド パス 状態`）。
    ///
    /// 「正しく」= `headers` に [`super::API_VERSION_HEADER`] があり、`required: true`・
    /// `schema.type == "string"`・`schema.examples == [API_VERSION]`。
    fn responses_without_version_header(document: &Value) -> (usize, Vec<String>) {
        let expected = json!({
            "required": true,
            "schema": {"type": "string", "examples": [API_VERSION]},
        });
        let mut total = 0;
        let mut missing = Vec::new();
        for (path, item) in document["paths"].as_object().into_iter().flatten() {
            for (method, operation) in item.as_object().into_iter().flatten() {
                for (status, response) in operation["responses"].as_object().into_iter().flatten() {
                    total += 1;
                    if response["headers"][super::API_VERSION_HEADER] != expected {
                        missing.push(format!("{method} {path} {status}"));
                    }
                }
            }
        }
        (total, missing)
    }

    /// **仕様書の全部の応答が、版のヘッダを名乗る**（P-2b のレビュー）。
    ///
    /// ヘッダはルータに届いた要求への全部の応答に付く（[`super::stamp_api_version`]）ので、
    /// 仕様書もそう言う。**`default`（エラー）も含む。**
    #[test]
    fn every_documented_response_names_the_version_header() {
        let document = document();
        let (total, missing) = responses_without_version_header(&document);
        let operations = documented_operations(&document).len();
        assert!(
            total >= 2 * operations && operations >= 22,
            "前提: 全部の口の、成功と default の両方を回すこと（応答 {total} / 口 {operations}）"
        );
        assert_eq!(missing, Vec::<String>::new());
    }

    /// **対照: 走査が名乗らない応答を見つけ、足す側がそれを埋める**（植えた文書を両方に通す）。
    #[test]
    fn the_version_header_scan_and_the_declaration_read_a_planted_document() {
        let mut planted = json!({
            "paths": {
                "/a": {
                    "parameters": [],
                    "get": {"responses": {
                        "200": {"headers": {super::API_VERSION_HEADER: {
                            "required": true,
                            "schema": {"type": "string", "examples": [API_VERSION]},
                        }}},
                        "default": {},
                    }},
                },
                "/b": {"post": {"responses": {
                    "201": {"headers": {super::API_VERSION_HEADER: {
                        "required": true,
                        "schema": {"type": "string", "examples": ["0.0-planted"]},
                    }}},
                }}},
            }
        });
        assert_eq!(
            responses_without_version_header(&planted),
            (
                3,
                vec!["get /a default".to_owned(), "post /b 201".to_owned()]
            )
        );
        super::declare_version_header(&mut planted);
        assert_eq!(responses_without_version_header(&planted), (3, Vec::new()));
        assert_eq!(
            planted["paths"]["/a"]["parameters"],
            json!([]),
            "応答でないものには触れない"
        );
    }

    /// **ADR-0047 が名乗る API の版の綴り**（`いまの API の版: \`…\`` の行）。
    fn versions_named_in(adr: &str) -> Vec<&str> {
        const MARKER: &str = "いまの API の版: `";
        adr.match_indices(MARKER)
            .filter_map(|(at, _)| {
                let rest = &adr[at + MARKER.len()..];
                rest.find('`').map(|end| &rest[..end])
            })
            .collect()
    }

    /// **文書（ADR-0047）が名乗る版 == 定数**（版の綴りの正は1箇所 / 司令塔の決定）。
    ///
    /// **期待値の外側の権威**（定数から作らない値）はここ——版を上げて ADR を直し忘れたら赤。
    #[test]
    fn the_adr_names_the_same_api_version() {
        let adr = include_str!("../../docs/adr/0047-the-api-is-the-product.md");
        assert_eq!(versions_named_in(adr), vec![API_VERSION]);
    }

    /// **対照: 走査が綴りを読む**（植えた文面を走査本体に通す）。
    #[test]
    fn the_adr_version_scan_reads_a_planted_text() {
        let planted = "前置き\r\n**いまの API の版: `9.7`**\r\nいまの API の版: `3.1`\n";
        assert_eq!(versions_named_in(planted), vec!["9.7", "3.1"]);
        assert_eq!(versions_named_in("版の話はしない"), Vec::<&str>::new());
    }

    /// **版の綴りを文字列リテラルで書いているのは定数の定義1箇所だけ**
    /// （ヘッダを付ける `mod.rs` と、仕様書を組む `openapi.rs`）。
    ///
    /// 探す綴りは定数から実行時に作る（ここにリテラルを置くと、自分が2つ目になる）。
    #[test]
    fn the_api_version_is_spelled_once() {
        let literal = format!("\"{API_VERSION}\"");
        let count = [include_str!("mod.rs"), include_str!("openapi.rs")]
            .iter()
            .map(|source| source.matches(&literal).count())
            .sum::<usize>();
        assert_eq!(count, 1, "{literal} の綴りが定数の外にもある");
    }

    /// **生成に失敗したら 500 の `ErrorBody`**（`Display` は定数、詳細はログだけ）。
    #[tokio::test]
    async fn a_document_that_cannot_be_built_answers_500_with_the_error_body() {
        use axum::body::to_bytes;
        use axum::http::StatusCode;

        // **入力の断片を文面に含むエラー**で失敗させる（型の不一致は値を文面に出す）。
        // EOF のエラーは入力を含まないので、`{err}` を本文に載せる変異でも末尾の assert が
        // 空振りしていた（P-2b のレビュー）。
        let failure =
            serde_json::from_str::<u8>("\"C:\\\\secret\\\\path\"").expect_err("型が合わないこと");
        assert!(
            failure.to_string().contains("secret"),
            "前提: エラーの文面が入力の断片を含むこと: {failure}"
        );
        let response = super::document_response(Err(super::DocumentError(failure)));
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("本文を読めること");
        let body: Value = serde_json::from_slice(&body).expect("JSON であること");
        let expected = {
            let response = axum::response::IntoResponse::into_response(
                super::super::error::ApiError::internal(),
            );
            let bytes = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("本文を読めること");
            serde_json::from_slice::<Value>(&bytes).expect("JSON であること")
        };
        assert_eq!(body, expected);
        assert!(!body.to_string().contains("secret"), "{body}");
    }

    /// **対照: 組み立てられたら、その JSON がそのまま 200 で返る。**
    #[tokio::test]
    async fn a_built_document_answers_200_as_json() {
        use axum::body::to_bytes;
        use axum::http::StatusCode;

        let planted = json!({"openapi": "3.2.0", "planted": true});
        let response = super::document_response(Ok(planted.clone()));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            "application/json"
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("本文を読めること");
        assert_eq!(
            serde_json::from_slice::<Value>(&body).expect("JSON であること"),
            planted
        );
    }

    // ------------------------------------------------------------------
    // 説明文を載せない（司令塔の決定の変更 / P-2a のレビュー）
    // ------------------------------------------------------------------

    /// `value` の中の `description` の**キーワード**の場所（名前の `description` は数えない）。
    ///
    /// **名前かどうかの判定は、落とす側と同じ [`super::NAME_MAPS`]**
    /// （落とす側が名前だと思って残した鍵を、ここも名前として数えない —— 食い違うと片方だけ緑になる）。
    fn description_keywords(value: &Value, keys_are_names: bool, at: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if !keys_are_names && map.contains_key("description") {
                    out.push(format!("{at}/description"));
                }
                for (key, inner) in map {
                    description_keywords(
                        inner,
                        !keys_are_names && super::NAME_MAPS.contains(&key.as_str()),
                        &format!("{at}/{key}"),
                        out,
                    );
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    description_keywords(item, false, &format!("{at}/{index}"), out);
                }
            }
            _ => {}
        }
    }

    /// **仕様書のどこにも `description` が無い。**
    #[test]
    fn the_document_carries_no_description() {
        let mut found = Vec::new();
        description_keywords(&document(), false, "", &mut found);
        assert!(found.is_empty(), "説明文が残っている: {found:#?}");
    }

    /// **対照: 落とす側と数える側が、キーワードと名前を見分ける**（植えた仕様書）。
    ///
    /// 落とす前は数える側が全部見つけ、落とした後は1つも見つけない。
    /// **`description` という名前の欄（`properties` の下）と型（`schemas` の下）は残る。**
    #[test]
    fn the_description_strip_reads_a_planted_document() {
        let mut planted = json!({
            "info": {"title": "t", "description": "情報"},
            "paths": {"/a": {"get": {
                "description": "口",
                "parameters": [{"name": "p", "in": "query", "description": "引数"}],
                "responses": {"200": {"description": "成功", "content": {
                    "application/json": {"schema": {"$ref": "#/components/schemas/description"}}
                }}},
            }}},
            "components": {"schemas": {"description": {
                "type": "object",
                "description": "型",
                "properties": {"description": {"type": "string", "description": "欄"}},
            }}},
        });
        let mut before = Vec::new();
        description_keywords(&planted, false, "", &mut before);
        before.sort();
        assert_eq!(
            before,
            vec![
                "/components/schemas/description/description",
                "/components/schemas/description/properties/description/description",
                "/info/description",
                "/paths//a/get/description",
                "/paths//a/get/parameters/0/description",
                "/paths//a/get/responses/200/description",
            ]
        );

        super::strip_descriptions(&mut planted, false);
        let mut after = Vec::new();
        description_keywords(&planted, false, "", &mut after);
        assert_eq!(after, Vec::<String>::new());
        assert!(
            planted["components"]["schemas"]["description"]["properties"]["description"]
                .is_object(),
            "description という名前の欄は残る: {planted}"
        );
        assert_eq!(planted["info"]["title"], json!("t"), "他の鍵は残る");
    }

    /// 仕様書の中の文字列（鍵も値も）で、**開発機のパスに見える綴り**を含むもの。
    fn machine_paths(value: &Value, out: &mut Vec<String>) {
        let looks_like_a_path = |text: &str| {
            let bytes = text.as_bytes();
            let drive = bytes
                .windows(3)
                .any(|w| w[0].is_ascii_alphabetic() && w[1] == b':' && w[2] == b'\\');
            drive
                || [
                    "\\\\?\\",
                    "/Users/",
                    "/home/",
                    "personacore-measurements",
                    "AppData",
                ]
                .iter()
                .any(|spelled| text.contains(spelled))
        };
        match value {
            Value::String(text) if looks_like_a_path(text) => out.push(text.clone()),
            Value::Object(map) => {
                for (key, inner) in map {
                    if looks_like_a_path(key) {
                        out.push(key.clone());
                    }
                    machine_paths(inner, out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    machine_paths(item, out);
                }
            }
            _ => {}
        }
    }

    /// **開発機のパスの綴り（`F:\` / `C:\Users\` 等）が仕様書に無い。**
    ///
    /// **落とす前は実際に載っていた**（`UpstreamReport` の説明に測定のディレクトリ）。
    /// 説明文を落とす仕掛けを外すと、ここが赤になる。
    #[test]
    fn the_document_names_no_path_of_this_machine() {
        let mut found = Vec::new();
        machine_paths(&document(), &mut found);
        assert!(found.is_empty(), "開発機のパスが載っている: {found:#?}");
    }

    /// **対照: パスの走査が、ドライブ・ホーム・測定のディレクトリを見つける**（植えた仕様書）。
    #[test]
    fn the_machine_path_scan_reads_a_planted_document() {
        let planted = json!({
            "a": "測定は F:\\dev\\measurements\\g10 にある",
            "b": ["C:\\Users\\someone\\x", "/home/someone/x", "ふつうの文"],
            "c": {"/Users/someone": "鍵にもある"},
            "d": "http://127.0.0.1:1234/v1",
        });
        let mut found = Vec::new();
        machine_paths(&planted, &mut found);
        found.sort();
        assert_eq!(
            found,
            vec![
                "/Users/someone",
                "/home/someone/x",
                "C:\\Users\\someone\\x",
                "測定は F:\\dev\\measurements\\g10 にある",
            ]
        );
    }

    /// **対照: ドライブ文字の無い測定のディレクトリ名だけの綴りも見つける。**
    ///
    /// 上の対照の測定のディレクトリは `F:\` つきで、ドライブの腕でも当たってしまう。
    /// **この1件は、測定のディレクトリの判定だけを確かめる**（ドライブ・ホームの腕では当たらない形）。
    #[test]
    fn the_machine_path_scan_finds_the_measurements_directory_without_a_drive() {
        let planted = json!({
            "a": "生の観測は personacore-measurements の下にある",
            "b": "ふつうの文",
        });
        let mut found = Vec::new();
        machine_paths(&planted, &mut found);
        assert_eq!(
            found,
            vec!["生の観測は personacore-measurements の下にある"]
        );
    }

    // ------------------------------------------------------------------
    // 保存の 422（P-2a のレビュー是正）
    // ------------------------------------------------------------------

    /// **保存の 422 は2つの形のどちらか**（`profile_editor::SaveRejection` の doc）。
    ///
    /// **`ErrorDetail` は余計な欄を許さない**——許すと、鍵を足した形も `ErrorBody` の腕に当たり、
    /// `oneOf` が「ちょうど1つ」を満たさない。
    #[test]
    fn the_profile_save_rejection_is_one_of_two_bodies() {
        let document = document();
        let rejected = &document["paths"]["/admin/upstream/profiles/{name}"]["put"]["responses"]["422"]
            ["content"]["application/json"]["schema"];
        assert_eq!(
            rejected["$ref"],
            json!("#/components/schemas/ProfileSaveRejection"),
            "{rejected}"
        );
        let arms: Vec<&Value> = schema(&document, "ProfileSaveRejection")["oneOf"]
            .as_array()
            .expect("oneOf であること")
            .iter()
            .map(|arm| &arm["$ref"])
            .collect();
        assert_eq!(
            arms,
            vec![
                &json!("#/components/schemas/ProfileInvalidBody"),
                &json!("#/components/schemas/ErrorBody"),
            ]
        );
        assert_eq!(
            schema(&document, "ErrorDetail")["additionalProperties"],
            json!(false)
        );
        let mut detail = required(schema(&document, "ProfileInvalidDetail"));
        detail.sort_unstable();
        assert_eq!(
            detail,
            vec![
                "code",
                "face",
                "key",
                "message",
                "reason",
                "reason_message",
                "type"
            ]
        );
    }

    // ------------------------------------------------------------------
    // ルート表との突き合わせ（**依存を増やさない走査** / 司令塔の決定）
    // ------------------------------------------------------------------

    /// `mod.rs` の `fn router(` の中の `.route("…", get(…).post(…))` を読み、
    /// `(パス, メソッド)` の組を返す。**コメント行は読まない。**
    ///
    /// **`method_router` の綴り（`get(` 等）は、直前が識別子の文字でないときだけ数える**
    /// （`admin::get_stats` や `budget(` を拾わない）。
    fn routes_in(source: &str) -> BTreeSet<(String, String)> {
        const METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];
        let source = source.replace("\r\n", "\n");
        let start = source
            .find("\nfn router(")
            .expect("`fn router(` が見つかること");
        let length = source[start..]
            .find("\n}\n")
            .expect("`fn router(` の終わりが見つかること");
        let body: Vec<&str> = source[start..start + length]
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect();
        let body = body.join("\n");

        let mut found = BTreeSet::new();
        for chunk in body.split(".route(").skip(1) {
            // **最後の `.route(` の後ろには `.fallback(` 以降が続く**ので切る。
            let chunk = chunk.split(".fallback(").next().unwrap_or(chunk);
            let path = chunk
                .split('"')
                .nth(1)
                .expect("`.route(` の最初の引数は文字列リテラル");
            let bytes = chunk.as_bytes();
            for method in METHODS {
                let pattern = format!("{method}(");
                let hit = chunk.match_indices(&pattern).any(|(at, _)| {
                    at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_')
                });
                if hit {
                    found.insert((path.to_owned(), method.to_owned()));
                }
            }
        }
        found
    }

    /// **ルート表の口と、仕様書の口が同じ。** 口を足して仕様書に載せ忘れたら赤。
    ///
    /// **逆向き（仕様書にだけある口）も赤。** 片方向だけ見ると、ルート表から消した口が
    /// 仕様書に残り続ける。
    #[test]
    fn the_route_table_and_the_document_list_the_same_operations() {
        let routes = routes_in(include_str!("mod.rs"));
        assert_eq!(
            routes.len(),
            29,
            "前提: 走査がルート表を読めていること（2026-09-30 は 22。作業単位 A-12a で取り決めの4つを足して 26、\
             A-12c-1 で提案の採否の2つを足して 28、B-66 で上流へいま照会する口を足して 29 / 数えたのは走査の外）: {routes:?}"
        );
        assert_eq!(routes, documented_operations(&document()));
    }

    /// **対照: 走査そのものが壊れていない**（植えたソースを走査本体に通す）。
    ///
    /// 実ファイルだけを見ると、走査の綴りを壊す変異が「両方とも空」で緑になりうる。
    #[test]
    fn the_route_scan_reads_a_planted_router() {
        let planted = concat!(
            "fn other() {\n    x.route(\"/not/here\", get(a))\n}\n",
            "\nfn router(state: AppState) -> Router {\n",
            "    Router::new()\n",
            "        // .route(\"/commented\", get(a))\n",
            "        .route(\"/a\", get(m::get_stats).post(m::budget))\r\n",
            "        .route(\n            \"/b/{id}\",\n            patch(m::x).delete(m::y),\n        )\n",
            "        .route(\"/c\", put(m::z))\n",
            "        .fallback(get_everything)\n",
            "}\n",
        );
        let expected: BTreeSet<(String, String)> = [
            ("/a", "get"),
            ("/a", "post"),
            ("/b/{id}", "patch"),
            ("/b/{id}", "delete"),
            ("/c", "put"),
        ]
        .into_iter()
        .map(|(path, method)| (path.to_owned(), method.to_owned()))
        .collect();
        assert_eq!(routes_in(planted), expected);
    }

    // ------------------------------------------------------------------
    // 型の名前が重ならない（`#[schema(as = …)]` / 司令塔の決定）
    // ------------------------------------------------------------------

    /// **各口が集める型を、`components` に畳む前の生の形で**集める。
    ///
    /// **`utoipa` は同じ名前を `components` の1つに黙って畳む**（実測: 名前が重なった2つの
    /// `MessageView` は、仕様書では1つになり、片方の欄だけが残った）。
    /// 畳んだ後の仕様書からは重なりが見えないので、**畳む前を見る。**
    ///
    /// 一覧は `(型, 名前)` を1回だけ書く——名前は `__path_` を外した綴りで、
    /// **仕様書の `operationId` の集合と突き合わせる**（口を足してここに書き忘れたら赤）。
    macro_rules! collected {
        ($($module:ident :: $path:ident),* $(,)?) => {{
            let mut schemas = Vec::new();
            let mut names = BTreeSet::new();
            $(
                // **`server` から辿る**（`openapi` 自身の口 `get_document` も同じ綴りで並べるため）。
                <super::super::$module::$path as utoipa::__dev::SchemaReferences>::schemas(&mut schemas);
                names.insert(stringify!($path).trim_start_matches("__path_").to_owned());
            )*
            (schemas, names)
        }};
    }

    /// 同じ名前で、**中身の違う**型が2つ以上あるもの。
    fn shared_names(schemas: &[(String, Value)]) -> Vec<String> {
        let mut by_name: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
        for (name, schema) in schemas {
            by_name.entry(name).or_default().push(schema);
        }
        by_name
            .into_iter()
            .filter(|(_, found)| found.iter().any(|schema| *schema != found[0]))
            .map(|(name, _)| name.to_owned())
            .collect()
    }

    #[test]
    fn no_two_types_share_a_schema_name() {
        use utoipa::ToSchema as _;

        let (mut schemas, names) = collected!(
            models::__path_list_models,
            chat::__path_chat_completions,
            upstream::__path_get_upstream,
            upstream::__path_check_upstream,
            switch::__path_switch_profile,
            profile_editor::__path_list_profiles,
            profile_editor::__path_save_profile,
            admin::__path_list_characters,
            admin::__path_create_character,
            admin::__path_update_character,
            admin::__path_delete_character,
            admin::__path_replace_stats,
            admin::__path_get_stats,
            admin::__path_get_drift,
            admin::__path_replace_attributes,
            admin::__path_get_attributes,
            admin::__path_get_attribute_revisions,
            admin::__path_list_agreements,
            admin::__path_create_agreement,
            admin::__path_end_agreement,
            admin::__path_adopt_agreement,
            admin::__path_reject_agreement,
            admin::__path_list_agreement_transitions,
            admin::__path_get_axis_candidates,
            admin::__path_list_memories,
            admin::__path_correct_memory,
            admin::__path_list_sessions,
            admin::__path_list_session_messages,
            openapi::__path_get_document,
        );
        super::ChatCompletionEvent::schemas(&mut schemas);

        let document = document();
        let operation_ids: BTreeSet<String> = document["paths"]
            .as_object()
            .into_iter()
            .flatten()
            .flat_map(|(_, item)| item.as_object().into_iter().flatten())
            .filter_map(|(_, operation)| operation["operationId"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(names, operation_ids, "一覧が仕様書の口と揃っていること");

        let schemas: Vec<(String, Value)> = schemas
            .into_iter()
            .map(|(name, schema)| {
                let value = serde_json::to_value(schema).expect("JSON にできること");
                (name, value)
            })
            .collect();
        assert!(
            schemas.len()
                > document["components"]["schemas"]
                    .as_object()
                    .map_or(0, |s| s.len()),
            "前提: 畳む前の方が多いこと（同じ型を複数の口が使う）"
        );
        assert_eq!(shared_names(&schemas), Vec::<String>::new());
    }

    /// **対照: 重なりを見つける関数が、重なりを見つける**（同じ中身の重なりは見逃す）。
    #[test]
    fn a_planted_shared_name_is_found() {
        let one = json!({"type": "object", "properties": {"a": {"type": "string"}}});
        let other = json!({"type": "object", "properties": {"b": {"type": "string"}}});
        assert_eq!(
            shared_names(&[
                ("Same".to_owned(), one.clone()),
                ("Same".to_owned(), one.clone()),
                ("Twice".to_owned(), one),
                ("Twice".to_owned(), other),
            ]),
            vec!["Twice".to_owned()]
        );
    }

    // ------------------------------------------------------------------
    // 閉じた綴り（**`enum` の値の集合 == domain の定義の集合**）
    // ------------------------------------------------------------------

    /// 仕様書のその欄が名乗る `enum`（配列なら `items` の `enum`）。無ければ `None`。
    fn enum_of(document: &Value, schema_name: &str, property: &str) -> Option<BTreeSet<String>> {
        let found = &document["components"]["schemas"][schema_name]["properties"][property];
        let values = found["enum"]
            .as_array()
            .or_else(|| found["items"]["enum"].as_array())?;
        Some(values.iter().map(Value::to_string).collect())
    }

    /// **期待値の集合**を JSON の綴りで（`"user"` / `null`）。
    fn set_of(values: impl IntoIterator<Item = Option<&'static str>>) -> BTreeSet<String> {
        values
            .into_iter()
            .map(|value| value.map_or(Value::Null, Value::from).to_string())
            .collect()
    }

    fn some(values: impl IntoIterator<Item = &'static str>) -> Vec<Option<&'static str>> {
        values.into_iter().map(Some).collect()
    }

    fn with_null(values: impl IntoIterator<Item = &'static str>) -> Vec<Option<&'static str>> {
        let mut listed = some(values);
        listed.push(None);
        listed
    }

    /// **閉じた綴りの欄と、その値の集合の出どころ**（domain の定義と応答の綴りを決める関数）。
    ///
    /// **この表の期待値は、仕様書を作る側と同じ定義から作っている**——守っているのは
    /// 「その欄に、その定義の `enum` が付いているか」（**付け忘れ・付け違い**）。
    /// **綴りそのものの値**は、下の `closed_values_are_the_spellings_on_the_wire` が
    /// 関数の外の値（直書き）で押さえる。
    fn closed_fields() -> Vec<(&'static str, &'static str, BTreeSet<String>)> {
        use super::super::admin::{delta_to_wire, role_to_wire, subject_to_wire};
        use super::super::chat::{ASSISTANT_ROLE, COMPLETION_OBJECT, finish_reason_to_wire};
        use super::super::error::ApiError;
        use super::super::launch::FaceState;
        use super::super::models::{LIST_OBJECT, MODEL_OBJECT, OWNED_BY};
        use super::super::profile_editor::{ProfilesState, problem_reasons};
        use super::super::relay::CHUNK_OBJECT;
        use super::super::upstreams::{FACE_KEYS, FACE_NAMES, ShapeReason};
        use crate::domain::{
            AgreementEnding, AgreementKeeper, AgreementKind, AgreementState, AxisCandidateState,
            AxisTrialFailure, CharacterColumn, DriftDelta, DriftModeSource, MemoryColumn, Role,
            StatDriftMode, Subject,
        };
        use crate::llm::{AnsweredBy, FinishReason};
        use crate::measured::ModelFace;

        let subjects = || set_of(some(Subject::ALL.map(subject_to_wire)));
        let modes = || set_of(some(StatDriftMode::ALL.map(StatDriftMode::label)));
        let nullable_modes = || set_of(with_null(StatDriftMode::ALL.map(StatDriftMode::label)));
        let candidate_states =
            || set_of(some(AxisCandidateState::ALL.map(AxisCandidateState::label)));
        let failures = || {
            set_of(with_null(
                AxisTrialFailure::ALL.map(AxisTrialFailure::label),
            ))
        };
        let finish = || {
            set_of(
                std::iter::once(None)
                    .chain(FinishReason::ALL.map(Some))
                    .map(finish_reason_to_wire),
            )
        };

        let kinds = || set_of(some(AgreementKind::ALL.map(AgreementKind::label)));
        let keepers = || set_of(some(AgreementKeeper::ALL.map(AgreementKeeper::label)));
        let states = || set_of(some(AgreementState::ALL.map(AgreementState::label)));

        vec![
            // **取り決め**（ADR-0051 / 作業単位 A-12a）。綴りの正は domain の `label`。
            ("AgreementView", "kind", kinds()),
            ("AgreementView", "keeper", keepers()),
            ("AgreementView", "state", states()),
            ("AgreementEndedView", "state", states()),
            ("AgreementTransitionView", "kind", kinds()),
            ("AgreementTransitionView", "keeper", keepers()),
            (
                "AgreementTransitionView",
                "before",
                set_of(with_null(AgreementState::ALL.map(AgreementState::label))),
            ),
            ("AgreementTransitionView", "after", states()),
            ("CreateAgreementRequest", "kind", kinds()),
            ("CreateAgreementRequest", "keeper", keepers()),
            // **提案の採否**（作業単位 A-12c-1）。
            ("AdoptAgreementRequest", "kind", kinds()),
            ("AdoptAgreementRequest", "keeper", keepers()),
            ("AgreementRejectedView", "state", states()),
            (
                "EndAgreementRequest",
                "state",
                set_of(some(
                    AgreementEnding::ALL.map(|ending| ending.state().label()),
                )),
            ),
            ("MemoryView", "subject", subjects()),
            ("AttributeView", "subject", subjects()),
            ("AttributeRevisionView", "subject", subjects()),
            ("AttributeRequest", "subject", subjects()),
            ("CorrectMemoryRequest", "subject", subjects()),
            (
                "UnreadableMemoryView",
                "invalid_columns",
                set_of(some(MemoryColumn::ALL.map(MemoryColumn::as_str))),
            ),
            (
                "UnreadableCharacterView",
                "invalid_columns",
                set_of(some(CharacterColumn::ALL.map(CharacterColumn::as_str))),
            ),
            ("AxisCandidateView", "state", candidate_states()),
            ("AxisCandidateTransitionView", "state", candidate_states()),
            ("AxisCandidateView", "reason", failures()),
            ("AxisCandidateTransitionView", "reason", failures()),
            ("CharacterView", "stat_drift_mode", nullable_modes()),
            (
                "CreateCharacterRequest",
                "stat_drift_mode",
                nullable_modes(),
            ),
            (
                "UpdateCharacterRequest",
                "stat_drift_mode",
                nullable_modes(),
            ),
            ("DriftView", "run_mode", modes()),
            ("DriftView", "default_run_mode", modes()),
            (
                "DriftView",
                "run_mode_source",
                set_of(some(DriftModeSource::ALL.map(DriftModeSource::label))),
            ),
            (
                "DriftVoteView",
                "delta",
                set_of(some(DriftDelta::ALL.map(delta_to_wire))),
            ),
            (
                "MessageView",
                "role",
                set_of(some(Role::ALL.map(role_to_wire))),
            ),
            (
                "ChatCompletionView",
                "object",
                set_of([Some(COMPLETION_OBJECT)]),
            ),
            ("ChoiceView", "finish_reason", finish()),
            ("ChatMessageView", "role", set_of([Some(ASSISTANT_ROLE)])),
            (
                "ChatCompletionChunk",
                "object",
                set_of([Some(CHUNK_OBJECT)]),
            ),
            ("ChunkChoice", "finish_reason", finish()),
            ("ChunkDelta", "role", set_of([Some(ASSISTANT_ROLE)])),
            ("ModelList", "object", set_of([Some(LIST_OBJECT)])),
            ("ModelEntry", "object", set_of([Some(MODEL_OBJECT)])),
            ("ModelEntry", "owned_by", set_of([Some(OWNED_BY)])),
            ("ProblemView", "reason", set_of(some(problem_reasons()))),
            (
                "ProfileInvalidDetail",
                "code",
                set_of([ApiError::upstream_profile_invalid().code()]),
            ),
            (
                "ProfileInvalidDetail",
                "reason",
                set_of(some(ShapeReason::ALL.map(ShapeReason::code))),
            ),
            (
                "ProfileInvalidDetail",
                "face",
                set_of(with_null(FACE_NAMES)),
            ),
            ("ProfileInvalidDetail", "key", set_of(with_null(FACE_KEYS))),
            ("ProblemView", "face", set_of(with_null(FACE_NAMES))),
            ("ProblemView", "key", set_of(with_null(FACE_KEYS))),
            (
                "ProfilesView",
                "state",
                set_of(some(ProfilesState::ALL.map(ProfilesState::label))),
            ),
            (
                "UpstreamView",
                "face",
                set_of(some(ModelFace::ALL.map(ModelFace::label))),
            ),
            (
                "UpstreamView",
                "answered_by",
                set_of(with_null(AnsweredBy::ALL.map(AnsweredBy::label))),
            ),
            ("LaunchView", "state", set_of(some(FaceState::all_labels()))),
            // **最後の照会の結果**（作業単位 B-66）。「照会していない」と結果の4つ。
            (
                "UpstreamView",
                "check_result",
                set_of(some(
                    std::iter::once(crate::server::upstream::NOT_CHECKED).chain(
                        crate::server::upstream::CheckOutcome::ALL
                            .map(crate::server::upstream::CheckOutcome::label),
                    ),
                )),
            ),
        ]
    }

    /// 表の欄のうち、仕様書の `enum` が期待の集合と違うもの（`enum` が無いものも含む）。
    fn mismatched_closed_fields(
        document: &Value,
        fields: &[(&'static str, &'static str, BTreeSet<String>)],
    ) -> Vec<String> {
        fields
            .iter()
            .filter(|(schema_name, property, expected)| {
                enum_of(document, schema_name, property).as_ref() != Some(expected)
            })
            .map(|(schema_name, property, expected)| {
                format!(
                    "{schema_name}.{property}: 仕様書 {:?} / 定義 {expected:?}",
                    enum_of(document, schema_name, property)
                )
            })
            .collect()
    }

    /// **閉じた綴りの欄は、domain の定義と同じ集合の `enum` を名乗る。**
    #[test]
    fn closed_values_come_from_their_definitions() {
        let mismatched = mismatched_closed_fields(&document(), &closed_fields());
        assert!(mismatched.is_empty(), "{mismatched:#?}");
    }

    /// **対照: 突き合わせが、欠けた値・余計な値・`enum` の無い欄を見つける**（植えた仕様書）。
    #[test]
    fn a_planted_mismatch_in_a_closed_field_is_found() {
        let planted = json!({"components": {"schemas": {
            "Dropped": {"properties": {"x": {"type": "string", "enum": ["a"]}}},
            "Extra": {"properties": {"x": {"type": "string", "enum": ["a", "b", "c"]}}},
            "Plain": {"properties": {"x": {"type": "string"}}},
            "Right": {"properties": {"x": {"type": "array", "items": {"enum": ["a", "b"]}}}},
        }}});
        let ab = || set_of([Some("a"), Some("b")]);
        let found = mismatched_closed_fields(
            &planted,
            &[
                ("Dropped", "x", ab()),
                ("Extra", "x", ab()),
                ("Plain", "x", ab()),
                ("Right", "x", ab()),
            ],
        );
        let names: Vec<&str> = found
            .iter()
            .map(|line| line.split(':').next().unwrap_or(""))
            .collect();
        assert_eq!(names, vec!["Dropped.x", "Extra.x", "Plain.x"]);
    }

    /// **綴りそのものを、関数の外の値（直書き）で押さえる**（CLAUDE.md「期待値を検査対象の
    /// 定数から作らない」）。上の表は定義と仕様書の配線しか見ないので、綴りが両方で
    /// 一緒に変わっても緑のまま——**線の上の綴りが変わったことは、ここが言う。**
    #[test]
    fn closed_values_are_the_spellings_on_the_wire() {
        let document = document();
        for (schema_name, property, spelled) in [
            (
                "MemoryView",
                "subject",
                vec![json!("user"), json!("character")],
            ),
            (
                "MessageView",
                "role",
                vec![json!("user"), json!("assistant")],
            ),
            ("DriftVoteView", "delta", vec![json!("up"), json!("down")]),
            (
                "AgreementView",
                "keeper",
                vec![json!("user"), json!("character"), json!("both")],
            ),
            (
                "EndAgreementRequest",
                "state",
                vec![json!("fulfilled"), json!("broken"), json!("withdrawn")],
            ),
            (
                "CharacterView",
                "stat_drift_mode",
                vec![json!("off"), json!("shadow"), json!("on"), Value::Null],
            ),
            (
                "LaunchView",
                "state",
                vec![
                    json!("external"),
                    json!("not_started"),
                    json!("loading"),
                    json!("ready"),
                    json!("failed"),
                    json!("stopped"),
                ],
            ),
            (
                "ChoiceView",
                "finish_reason",
                vec![json!("stop"), json!("length"), Value::Null],
            ),
            // **面の鍵**（作業単位 A-13a-2 で `context_tokens`、A-13b-1 で `api_key`、
            // A-13b-2 で `allow_key_over_plain_http` が入った。
            // 画面は保存の 422 の `key` をこの綴りで欄に結びつける）。
            (
                "ProfileInvalidDetail",
                "key",
                vec![
                    json!("endpoint"),
                    json!("model"),
                    json!("gguf"),
                    json!("args"),
                    json!("command"),
                    json!("context_tokens"),
                    json!("api_key"),
                    json!("allow_key_over_plain_http"),
                    Value::Null,
                ],
            ),
            // **鍵を平文の `http://` で送る扱い**（作業単位 A-13b-2）。**省かれうるが `null` にはならない。**
            (
                "UpstreamView",
                "key_over_plain_http",
                vec![
                    json!("local_network"),
                    json!("approved"),
                    json!("not_approved"),
                ],
            ),
        ] {
            let expected: BTreeSet<String> = spelled.iter().map(Value::to_string).collect();
            assert_eq!(
                enum_of(&document, schema_name, property),
                Some(expected),
                "{schema_name}.{property}"
            );
        }
    }

    /// **要求の主語の `enum` は、サーバが実際に受け付ける綴り**（`admin::subject_of`）。
    ///
    /// 仕様書の値は応答の綴り（`subject_to_wire`）から作っているので、
    /// **読む側の判定と食い違ったら、仕様書が受け付けない値を載せる**ことになる。
    #[test]
    fn every_documented_request_subject_is_accepted() {
        let document = document();
        for schema_name in ["AttributeRequest", "CorrectMemoryRequest"] {
            let values = enum_of(&document, schema_name, "subject").expect("enum があること");
            for value in values {
                let spelled: String = serde_json::from_str(&value).expect("文字列であること");
                assert!(
                    super::super::admin::subject_of(&spelled).is_some(),
                    "{schema_name}.subject の {spelled} を受け付けること"
                );
            }
        }
    }

    /// **`enum` を持たない文字列の欄は、全部ここに名指す**（開いている欄）。
    ///
    /// 閉じた綴りの欄を足して `schema_with` を付け忘れると、その欄は素の `string` になり
    /// ここで赤になる——**「閉じているか、開いているか」を必ず1回決めさせる。**
    const OPEN_STRINGS: &[(&str, &str)] = &[
        ("AdoptAgreementRequest", "content"),
        ("AgreementEndedView", "character"),
        ("AgreementRejectedView", "character"),
        ("AgreementTransitionView", "content"),
        ("AgreementTransitionView", "created_at"),
        ("AgreementTransitionsView", "character"),
        ("AgreementTurnView", "at"),
        ("AgreementView", "content"),
        ("AgreementView", "created_at"),
        ("AgreementsView", "character"),
        ("AttributeRequest", "key"),
        ("AttributeRequest", "value"),
        ("AttributeRevisionView", "after"),
        ("AttributeRevisionView", "before"),
        ("AttributeRevisionView", "created_at"),
        ("AttributeRevisionView", "key"),
        ("AttributeRevisionsView", "character"),
        ("AttributeView", "key"),
        ("AttributeView", "value"),
        ("AttributesView", "character"),
        ("AxisCandidateTransitionView", "created_at"),
        ("AxisCandidateView", "created_at"),
        ("AxisCandidateView", "name"),
        ("AxisCandidatesView", "character"),
        ("CharacterView", "archived_at"),
        ("CharacterView", "base_persona"),
        ("CharacterView", "created_at"),
        ("CharacterView", "display_name"),
        ("CharacterView", "handle"),
        ("CharacterView", "output_format_instruction"),
        ("ChatCompletionChunk", "id"),
        ("ChatCompletionChunk", "model"),
        ("ChatCompletionRequest", "model"),
        ("ChatCompletionView", "id"),
        ("ChatCompletionView", "model"),
        ("ChatMessageView", "content"),
        ("ChunkDelta", "content"),
        ("CorrectMemoryRequest", "category"),
        ("CorrectMemoryRequest", "content"),
        ("CorrectionView", "character"),
        ("CreateAgreementRequest", "content"),
        ("CreateCharacterRequest", "base_persona"),
        ("CreateCharacterRequest", "display_name"),
        ("CreateCharacterRequest", "handle"),
        ("CreateCharacterRequest", "output_format_instruction"),
        ("DriftAxisView", "axis"),
        ("DriftAxisView", "charge_updated_at"),
        ("DriftView", "character"),
        ("DriftVoteView", "axis"),
        ("DriftVoteView", "created_at"),
        // エラーの3つは**閉じているが、定義が1箇所に無い**（`ApiError` の各コンストラクタに散る）。
        ("ErrorDetail", "code"),
        ("ErrorDetail", "message"),
        ("ErrorDetail", "type"),
        ("IncomingMessage", "role"),
        ("LaunchView", "answered_name"),
        ("LaunchView", "log"),
        ("LaunchView", "reason"),
        ("LaunchView", "since"),
        ("MemoryListView", "character"),
        ("MemoryView", "category"),
        ("MemoryView", "content"),
        ("MemoryView", "created_at"),
        ("MessagePageView", "character"),
        ("MessageView", "created_at"),
        ("MessageView", "dialogue"),
        ("MessageView", "narration"),
        ("MessageView", "raw_content"),
        ("ModelEntry", "id"),
        // 文面（**閉じているが、`reason` の綴りごとの定数の日本語**。値を並べる意味が無い）。
        ("ProblemView", "message"),
        ("ProblemView", "profile"),
        // 保存の 422 の本文（写し / `profile_editor::ProfileInvalidDetail`）。文面と種類は `ErrorDetail` と同じ扱い。
        ("ProfileInvalidDetail", "message"),
        ("ProfileInvalidDetail", "reason_message"),
        ("ProfileInvalidDetail", "type"),
        ("ProfilesView", "path"),
        ("ProfilesView", "revision"),
        ("SaveRequest", "revision"),
        ("SessionListView", "character"),
        ("SessionSummaryView", "created_at"),
        ("SessionSummaryView", "first_user_message"),
        ("SessionSummaryView", "label"),
        ("SessionSummaryView", "updated_at"),
        ("StatAllocRequest", "axis"),
        ("StatAllocView", "axis"),
        ("StatsView", "character"),
        ("SwitchRequest", "name"),
        ("UnreadableCharacterView", "handle"),
        ("UpdateCharacterRequest", "base_persona"),
        ("UpdateCharacterRequest", "display_name"),
        ("UpdateCharacterRequest", "output_format_instruction"),
        ("UpstreamReport", "profile_sample"),
        // 利用者が `upstreams.json` に書いたプロファイルの名前（作業単位 F-10b）。
        ("UpstreamReport", "selected_profile"),
        ("UpstreamView", "answered_name"),
        // **照会した時刻**（RFC 3339 / 作業単位 B-66）。`LaunchView.since` と同じ立場。
        ("UpstreamView", "checked_at"),
        ("UpstreamView", "endpoint"),
        ("UpstreamView", "model"),
        // 鍵の末尾4文字（作業単位 A-13b-1 / ADR-0056 §決定3）。
        ("WrittenFace", "api_key_hint"),
        ("WrittenFace", "args"),
        ("WrittenFace", "command"),
        ("WrittenFace", "endpoint"),
        ("WrittenFace", "gguf"),
        ("WrittenFace", "model"),
        // 鍵そのもの（依頼にだけ載る / 作業単位 A-13b-1）。
        ("WrittenFaceRequest", "api_key"),
        ("WrittenFaceRequest", "args"),
        ("WrittenFaceRequest", "command"),
        ("WrittenFaceRequest", "endpoint"),
        ("WrittenFaceRequest", "gguf"),
        ("WrittenFaceRequest", "model"),
        ("WrittenProfile", "name"),
    ];

    /// `type` が文字列を含み、`enum` を持たない欄（配列の `items` も見る）。
    fn open_string_fields(document: &Value) -> BTreeSet<(String, String)> {
        let is_open_string = |property: &Value| {
            let names_string = match &property["type"] {
                Value::String(kind) => kind == "string",
                Value::Array(kinds) => kinds.iter().any(|kind| kind == "string"),
                _ => false,
            };
            names_string && property["enum"].is_null()
        };
        let mut found = BTreeSet::new();
        for (schema_name, schema) in document["components"]["schemas"]
            .as_object()
            .into_iter()
            .flatten()
        {
            for part in parts(schema) {
                for (property, value) in part["properties"].as_object().into_iter().flatten() {
                    if is_open_string(value) || is_open_string(&value["items"]) {
                        found.insert((schema_name.clone(), property.clone()));
                    }
                }
            }
        }
        found
    }

    #[test]
    fn every_string_field_is_either_closed_or_named_open() {
        let found = open_string_fields(&document());
        let named: BTreeSet<(String, String)> = OPEN_STRINGS
            .iter()
            .map(|(schema_name, property)| ((*schema_name).to_owned(), (*property).to_owned()))
            .collect();
        let unnamed: Vec<_> = found.difference(&named).collect();
        let stale: Vec<_> = named.difference(&found).collect();
        assert!(
            unnamed.is_empty() && stale.is_empty(),
            "enum の無い文字列の欄で、一覧に無いもの: {unnamed:#?}\n一覧にあるが、もう無い（または閉じた）もの: {stale:#?}"
        );
    }

    /// **対照: 開いた欄の走査が、`enum` の有無と配列の中を見分ける**（植えた仕様書）。
    #[test]
    fn the_open_string_scan_reads_a_planted_document() {
        let planted = json!({"components": {"schemas": {"P": {"properties": {
            "open": {"type": "string"},
            "nullable_open": {"type": ["string", "null"]},
            "closed": {"type": "string", "enum": ["a"]},
            "array_open": {"type": "array", "items": {"type": "string"}},
            "array_closed": {"type": "array", "items": {"type": "string", "enum": ["a"]}},
            "number": {"type": "integer"},
        }}, "Flattened": {"allOf": [
            {"$ref": "#/components/schemas/P"},
            {"type": "object", "properties": {"inner_open": {"type": "string"}}},
        ]}}}});
        let found: Vec<String> = open_string_fields(&planted)
            .into_iter()
            .map(|(schema_name, property)| format!("{schema_name}.{property}"))
            .collect();
        assert_eq!(
            found,
            vec![
                "Flattened.inner_open",
                "P.array_open",
                "P.nullable_open",
                "P.open"
            ]
        );
    }

    // ------------------------------------------------------------------
    // 形（第1段から）
    // ------------------------------------------------------------------

    /// **`/admin/` の要求は知らない欄を拒む**（`deny_unknown_fields`）——
    /// **`/v1/` の要求は拒まない**（OpenAI 互換。`top_p` 等を普通に受ける / `chat` の doc）。
    ///
    /// **両方を1本で見る。** 片方だけだと、全部に `false` を付ける実装も
    /// 全部に付けない実装も、どちらかの腕で緑になる。
    #[test]
    fn admin_bodies_refuse_unknown_fields_but_the_openai_body_does_not() {
        let document = document();
        for name in [
            "CreateCharacterRequest",
            "UpdateCharacterRequest",
            "ReplaceStatsRequest",
            "StatAllocRequest",
            "ReplaceAttributesRequest",
            "AttributeRequest",
            "CorrectMemoryRequest",
            "SaveRequest",
            "WrittenFaceRequest",
            "SwitchRequest",
        ] {
            assert_eq!(
                schema(&document, name)["additionalProperties"],
                json!(false),
                "{name}"
            );
        }
        assert_ne!(
            schema(&document, "ChatCompletionRequest")["additionalProperties"],
            json!(false),
            "OpenAI 互換の要求に additionalProperties: false を付けないこと"
        );
    }

    /// **`FieldUpdate` の3状態**（省く / `null` / 値）は「必須でない ＋ `null` を許す」で表れる。
    ///
    /// **応答の `Option` とは逆向き**——応答の `output_format_instruction` は
    /// **常に載って `null` になりうる**（`required` に入る ＋ `null` を許す）。
    /// **同じ綴りの欄を要求と応答で並べて見る**（取り違えると片方だけ緑になる）。
    #[test]
    fn a_three_state_field_is_optional_and_nullable_while_the_response_always_carries_it() {
        let document = document();
        let request = schema(&document, "UpdateCharacterRequest");
        for field in [
            "output_format_instruction",
            "stat_drift_mode",
            "display_name",
        ] {
            assert!(
                !required(request).contains(&field),
                "要求の {field} は省けること: {request}"
            );
            assert!(
                admits_null(&request["properties"][field]),
                "要求の {field} は null を受けること: {}",
                request["properties"][field]
            );
        }

        let response = schema(&document, "CharacterView");
        assert!(
            required(response).contains(&"output_format_instruction"),
            "応答の output_format_instruction は常に載ること: {response}"
        );
        assert!(admits_null(
            &response["properties"]["output_format_instruction"]
        ));
        assert!(
            !admits_null(&response["properties"]["handle"]),
            "対照: null にならない欄は null を許さないこと"
        );
    }

    /// **`WrittenFace` は要求と応答で別の型**（司令塔の決定）——同じ欄が、
    /// 応答では常に載り（`null` になりうる）、要求では省ける。
    #[test]
    fn a_written_face_is_carried_in_responses_but_may_be_omitted_in_requests() {
        let document = document();
        let response = schema(&document, "WrittenFace");
        let request = schema(&document, "WrittenFaceRequest");
        for field in ["gguf", "args", "command", "context_tokens"] {
            assert!(
                required(response).contains(&field),
                "応答の {field}: {response}"
            );
            assert!(
                !required(request).contains(&field),
                "要求の {field}: {request}"
            );
        }
        // **鍵**（作業単位 A-13b-1 / ADR-0056 §決定3）——応答は値を持たず、印と末尾4文字を必ず載せる。
        // 要求は `api_key` を省ける（省略は「残す」）し、`null` も送れる（「消す」）。
        for field in ["api_key_set", "api_key_hint"] {
            assert!(
                required(response).contains(&field),
                "応答の {field}: {response}"
            );
        }
        assert!(
            response["properties"].get("api_key").is_none(),
            "応答に鍵の値の欄がある: {response}"
        );
        assert!(
            !required(request).contains(&"api_key"),
            "要求の api_key は省けること: {request}"
        );
        assert!(
            admits_null(&request["properties"]["api_key"]),
            "要求の api_key は null を許すこと（消す）: {request}"
        );
        // **承認**（作業単位 A-13b-3 / ADR-0056 §決定5）——応答は書いた値を必ず載せ（書いていなければ `false`）、
        // 要求は省ける（省略と `null` は今の規則、`true` は承認、`false` は外す）。
        assert!(
            required(response).contains(&"allow_key_over_plain_http"),
            "応答の allow_key_over_plain_http: {response}"
        );
        assert!(
            !admits_null(&response["properties"]["allow_key_over_plain_http"]),
            "応答の allow_key_over_plain_http は null にならないこと: {response}"
        );
        assert!(
            !required(request).contains(&"allow_key_over_plain_http"),
            "要求の allow_key_over_plain_http は省けること: {request}"
        );
        assert_eq!(
            document["components"]["schemas"]["SaveRequest"]["properties"]["main"]["$ref"],
            json!("#/components/schemas/WrittenFaceRequest")
        );
    }

    /// **割り当ての無い軸の `value` は `null`**（`DriftAxisView::value` の doc）。
    #[test]
    fn a_value_that_may_be_null_says_so() {
        let document = document();
        let axis = schema(&document, "DriftAxisView");
        assert!(required(axis).contains(&"value"));
        assert!(admits_null(&axis["properties"]["value"]));
        assert!(!admits_null(&axis["properties"]["charge"]));
    }

    /// **エラーの形**（`{"error": {"message", "type", "code"?}}` / `error` モジュールの doc）。
    /// **どの口も `default` に `ErrorBody` を持つ**（司令塔の決定）。
    #[test]
    fn every_operation_falls_back_to_the_error_body() {
        let document = document();
        let body = schema(&document, "ErrorBody");
        assert_eq!(required(body), vec!["error"]);
        let detail = schema(&document, "ErrorDetail");
        let mut names = required(detail);
        names.sort_unstable();
        assert_eq!(
            names,
            vec!["message", "type"],
            "code は省かれうる: {detail}"
        );

        let operations = documented_operations(&document);
        assert!(operations.len() >= 22, "前提: 全部の口を回すこと");
        for (path, method) in operations {
            let fallback = &document["paths"][&path][&method]["responses"]["default"];
            assert_eq!(
                fallback["content"]["application/json"]["schema"]["$ref"],
                json!("#/components/schemas/ErrorBody"),
                "{method} {path}: {fallback}"
            );
        }
    }

    /// **`stream: true` なら SSE**——同じ口の 200 が JSON と SSE の2通りを名乗る。
    #[test]
    fn the_chat_endpoint_answers_json_or_an_event_stream() {
        let document = document();
        let ok = &document["paths"]["/v1/chat/completions"]["post"]["responses"]["200"]["content"];
        assert_eq!(
            ok["application/json"]["schema"]["$ref"],
            json!("#/components/schemas/ChatCompletionView"),
            "{ok}"
        );
        assert_eq!(
            ok["text/event-stream"]["itemSchema"]["$ref"],
            json!("#/components/schemas/ChatCompletionEvent"),
            "SSE の1件ずつの形（OpenAPI 3.2 の itemSchema）: {ok}"
        );

        // **`data` は `anyOf`**——`[DONE]` は「文字列」の腕にも当てはまる
        // （`contentSchema` は注釈で、検証はしない / JSON Schema 2020-12）。
        // `oneOf` にすると **`[DONE]` が2つの腕に当たり、正しい終端が不正と判定される。**
        let data = &schema(&document, "ChatCompletionEvent")["properties"]["data"];
        let arms = data["anyOf"].as_array().expect("data は anyOf であること");
        assert_eq!(arms.len(), 2, "{data}");
        assert!(data["oneOf"].is_null(), "{data}");
        assert!(
            arms.iter().any(|arm| arm["enum"] == json!(["[DONE]"])),
            "終端の [DONE] を名乗ること: {data}"
        );
    }

    /// **実験中の口だけが印を持つ**（`axis-candidates` だけ / 司令塔の決定）。
    #[test]
    fn only_the_experimental_route_is_marked() {
        let document = document();
        let marked: Vec<_> = documented_operations(&document)
            .into_iter()
            .filter(|(path, method)| {
                !document["paths"][path][method][STABILITY_EXTENSION].is_null()
            })
            .collect();
        assert_eq!(
            marked,
            vec![(
                "/admin/characters/{handle}/axis-candidates".to_owned(),
                "get".to_owned()
            )]
        );
        assert_eq!(
            document["paths"]["/admin/characters/{handle}/axis-candidates"]["get"]
                [STABILITY_EXTENSION],
            json!("experimental")
        );
    }

    /// **応答では、欄は省かれない**（`Option` は `null` として必ず載る）——
    /// **省かれうるのは `skip_serializing_if` の欄だけ**で、それは下の一覧に名指す。
    ///
    /// # ⚠ `utoipa` の既定は逆
    ///
    /// **`utoipa` は `Option<T>` の欄を向きに関わらず「必須でない」と書く**（2026-09-30 に実測）。
    /// 応答の型では `#[schema(required = true)]` を付けないと、
    /// **仕様書が実際より緩くなる**——「省かれうる」と読んだ使い手は `null` と欠落を別に扱う
    /// 分岐を書き、**実際の応答を仕様書で検証しても、この向きのずれは赤にならない**
    /// （緩い仕様書は、より厳しい応答を必ず通す）。**だから構造で見る。**
    ///
    /// 対象は**応答から辿れる全部の型**（`$ref` を再帰で辿る）。
    #[test]
    fn a_response_never_omits_a_field_unless_it_is_named_here() {
        /// `skip_serializing_if` の欄（**応答で省かれうる**）。**ここに無い欄は必須でなければならない。**
        const MAY_BE_ABSENT: &[(&str, &str)] = &[
            ("ErrorDetail", "code"),
            // 上書きされた記憶の節は**訊いたときだけ**載る（`MemoryListView::superseded` の doc）。
            ("MemoryListView", "superseded"),
            // SSE の増分（写し / `ChatCompletionEvent` の doc）。先頭の1件だけが `role` を名乗る。
            ("ChunkDelta", "role"),
            // 鍵を平文の `http://` で送る扱いは**警告があるときだけ**載る（`UpstreamView` の doc / 作業単位 A-13b-2）。
            ("UpstreamView", "key_over_plain_http"),
        ];

        let document = document();
        let mut pending: Vec<String> = Vec::new();
        for item in document["paths"]
            .as_object()
            .into_iter()
            .flat_map(|p| p.values())
        {
            for operation in item.as_object().into_iter().flat_map(|o| o.values()) {
                collect_refs(&operation["responses"], &mut pending);
            }
        }
        let mut seen = BTreeSet::new();
        let mut checked = BTreeSet::new();
        let mut loose = Vec::new();
        while let Some(name) = pending.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let found = schema(&document, &name);
            collect_refs(found, &mut pending);
            for part in parts(found) {
                let required = required(part);
                for property in part["properties"]
                    .as_object()
                    .into_iter()
                    .flat_map(|p| p.keys())
                {
                    checked.insert(format!("{name}.{property}"));
                    if !required.contains(&property.as_str())
                        && !MAY_BE_ABSENT.contains(&(name.as_str(), property.as_str()))
                    {
                        loose.push(format!("{name}.{property}"));
                    }
                }
            }
        }
        assert!(
            seen.contains("CharacterView")
                && seen.contains("ChunkDelta")
                && seen.contains("LaunchView"),
            "前提: 応答の型を辿れていること: {seen:?}"
        );
        assert!(
            checked.contains("SupersededMemoryView.superseded_by"),
            "前提: flatten した型の欄（allOf の中）も見ていること"
        );
        assert!(
            loose.is_empty(),
            "応答で省かれうると書かれている欄（`#[schema(required = true)]` が要る）: {loose:?}"
        );

        // **省かれうる欄は `null` にならない**（`skip_serializing_if = "Option::is_none"` は
        // `None` を**欠落**にする）。`utoipa` は `Option` を見て `null` を許すと書くので、
        // `#[schema(nullable = false)]` が要る。
        for (name, property) in MAY_BE_ABSENT {
            let found = &schema(&document, name)["properties"][*property];
            assert!(found.is_object(), "{name}.{property} が無い");
            assert!(
                !admits_null(found),
                "{name}.{property} は欠落するのであって null にはならない: {found}"
            );
        }
    }

    // ------------------------------------------------------------------
    // 英語の `summary`（ADR-0047 §② P-2c / P-2a のレビューでの司令塔の決定）
    // ------------------------------------------------------------------

    /// `summary` の上限の長さ（文字数）。**1行の要約**であって説明文ではない。
    const SUMMARY_MAX_CHARS: usize = 80;

    /// 仕様書の全部の口の数と、そのうち **英語の1行の `summary` を持たないもの**（`メソッド パス: 理由`）。
    ///
    /// 「英語の1行」= 文字列・前後の空白を落として空でない・改行が無い・**ASCII の印字可能文字と空白だけ**・
    /// [`SUMMARY_MAX_CHARS`] 以下・**`` ` `` で始まらない**。
    ///
    /// # ⚠ `summary` を書き忘れても、欄は空にならない
    ///
    /// **`utoipa` は `summary` を省くと、doc コメントの最初の段落を `summary` にする**
    /// （P-2c の前に配っていた仕様書で実測: `` `GET /admin/characters` — キャラクターの一覧。 `` や、
    /// `../../docs/adr/…` への相対リンク・作業単位の名前が入っていた）。
    /// **だから「在るか」ではなく「英語の1行か」で見る。** いまの handler の doc コメントは
    /// 口の綴りを `` ` `` で書き始め、日本語で続くので、書き忘れはこの条件のどれかに当たる
    /// （P-2c の変異 `m01` では、`get_drift` の書き忘れが「複数行」で赤になった）。
    /// **⚠ 英語で、`` ` `` で始まらない1行の doc コメントを持つ口だと、書き忘れは見えない。**
    fn operations_without_an_english_summary(document: &Value) -> (usize, Vec<String>) {
        let mut total = 0;
        let mut missing = Vec::new();
        for (path, method) in documented_operations(document) {
            total += 1;
            let summary = &document["paths"][&path][&method]["summary"];
            let reason = match summary.as_str() {
                None => Some("無い"),
                // **空白を落とす前に見る**（`trim` は行末の CR を消す）。
                Some(text) if text.contains(['\n', '\r']) => Some("複数行"),
                Some(text) if text.trim().is_empty() => Some("空"),
                Some(text) if !text.chars().all(|c| c == ' ' || c.is_ascii_graphic()) => {
                    Some("ASCII でない")
                }
                Some(text) if text.chars().count() > SUMMARY_MAX_CHARS => Some("長すぎる"),
                Some(text) if text.starts_with('`') => Some("doc コメントの綴り"),
                Some(_) => None,
            };
            if let Some(reason) = reason {
                missing.push(format!("{method} {path}: {reason}"));
            }
        }
        (total, missing)
    }

    /// **仕様書の全部の口が、英語の1行の `summary` を持つ**（P-2c）。
    ///
    /// 説明文（`description`）は全部落とす（[`super::api_document`] の doc）ので、
    /// **外の使い手が読める説明はこれだけ。** 落とす処理が `summary` を巻き込まないことも、
    /// ここが組み上がった（落とした後の）仕様書を見ることで同時に見ている。
    #[test]
    fn every_operation_has_an_english_summary() {
        let document = document();
        let (total, missing) = operations_without_an_english_summary(&document);
        assert!(total >= 22, "前提: 全部の口を回すこと（{total}）");
        assert_eq!(missing, Vec::<String>::new());
    }

    /// **対照: 走査が、無い・空・複数行・日本語・長すぎる・doc コメントの綴りを見分ける**
    /// （植えた仕様書を走査本体に通す）。
    #[test]
    fn the_summary_scan_reads_a_planted_document() {
        let long = "x".repeat(SUMMARY_MAX_CHARS + 1);
        let at_limit = "y".repeat(SUMMARY_MAX_CHARS);
        let planted = json!({"paths": {
            "/good": {"get": {"summary": "List characters"}},
            "/limit": {"get": {"summary": at_limit}},
            "/none": {"get": {}},
            "/empty": {"post": {"summary": "  "}},
            "/lines": {"put": {"summary": "One\nTwo"}},
            "/crlf": {"put": {"summary": "One\r"}},
            "/japanese": {"patch": {"summary": "キャラクターの一覧"}},
            "/long": {"delete": {"summary": long}},
            "/doc": {"get": {"summary": "`GET /doc` - list"}},
        }});
        let (total, mut missing) = operations_without_an_english_summary(&planted);
        missing.sort();
        assert_eq!(total, 9);
        assert_eq!(
            missing,
            vec![
                "delete /long: 長すぎる",
                "get /doc: doc コメントの綴り",
                "get /none: 無い",
                "patch /japanese: ASCII でない",
                "post /empty: 空",
                "put /crlf: 複数行",
                "put /lines: 複数行",
            ]
        );
    }

    /// **説明文を落とす処理は `summary` を残す**（植えた仕様書 / P-2a の決定「英語の要約は `summary` として」）。
    ///
    /// 口・引数・応答・型の `description` は消え、**口の `summary` と、`summary` という名前の欄は残る。**
    #[test]
    fn the_description_strip_keeps_summaries() {
        let mut planted = json!({
            "paths": {"/a": {"get": {
                "summary": "Read a",
                "description": "説明",
                "responses": {"200": {"description": "成功"}},
            }}},
            "components": {"schemas": {"S": {
                "description": "型",
                "properties": {"summary": {"type": "string", "description": "欄"}},
            }}},
        });
        super::strip_descriptions(&mut planted, false);
        assert_eq!(
            planted,
            json!({
                "paths": {"/a": {"get": {
                    "summary": "Read a",
                    "responses": {"200": {}},
                }}},
                "components": {"schemas": {"S": {
                    "properties": {"summary": {"type": "string"}},
                }}},
            })
        );
    }

    /// **仕様書の中の `$ref` は全部、`components.schemas` の中を指す。**
    ///
    /// # ⚠ `utoipa` 6.0.1 の `item_schema` は、指す先を集めない（2026-09-30 に実測）
    ///
    /// `responses(... ("text/event-stream", item_schema = T))` は `$ref` だけを書き、
    /// **`T` の定義を `components` に入れない**（`body = T` は入れる）。
    /// 宙に浮いた `$ref` は**仕様書として不正**で、読み手のツールが落ちる。
    /// **`ApiDoc` の `components(schemas(...))` に名指して補っている。**
    #[test]
    fn every_reference_points_inside_the_document() {
        let document = document();
        let mut refs = Vec::new();
        collect_refs(&document, &mut refs);
        assert!(!refs.is_empty(), "前提: $ref が1つはあること");
        let dangling: Vec<_> = refs
            .iter()
            .filter(|name| !document["components"]["schemas"][name.as_str()].is_object())
            .collect();
        assert!(dangling.is_empty(), "宙に浮いた $ref: {dangling:?}");
    }

    /// `value` の中の `$ref`（`#/components/schemas/<名前>`）を全部拾う。
    fn collect_refs(value: &Value, into: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if let Some(Value::String(target)) = map.get("$ref")
                    && let Some(name) = target.strip_prefix("#/components/schemas/")
                {
                    into.push(name.to_owned());
                }
                for inner in map.values() {
                    collect_refs(inner, into);
                }
            }
            Value::Array(items) => {
                for inner in items {
                    collect_refs(inner, into);
                }
            }
            _ => {}
        }
    }
}
