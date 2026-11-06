//! `GET /v1/models`。
//!
//! **キャラクターのみを並べる**（ADR-0004）。セッションは並べない
//! （`aoi@new` / `aoi@<session-id>` は受け付けるが一覧には出さない）。
//! クライアントのモデル選択 UI が、そのままキャラクター選択 UI になる。
//!
//! 上流（LM Studio）には**問い合わせない**。ここで並ぶ `id` は推論モデルではなく
//! キャラクターなので、上流のモデル一覧とは無関係。

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa::openapi::schema::Object;

use super::error::{ApiError, ErrorBody};
use super::{AppState, openapi};
use crate::store::CharacterEntry;

/// `object` フィールドの定数（OpenAI 互換）。
pub(super) const LIST_OBJECT: &str = "list";
pub(super) const MODEL_OBJECT: &str = "model";

/// 所有者表記。上流のモデルではなく本サービスのキャラクターであることを示す。
pub(super) const OWNED_BY: &str = "personacore";

/// 一覧の応答。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct ModelList {
    #[schema(schema_with = list_object_schema)]
    object: &'static str,
    data: Vec<ModelEntry>,
}

/// 一覧の1件 = 1キャラクター。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct ModelEntry {
    /// **キャラクター識別子**（`character.handle`）。`model` フィールドに
    /// そのまま書ける値（ADR-0004）。
    ///
    /// **プロンプトに載る呼び名（`display_name`）ではない**（ADR-0011）。
    /// `aoi` と `aoi-v2` が**どちらもプロンプト上は「葵」**として共存しうるので、
    /// ここに呼び名を載せると一覧に同じ綴りが2つ並び、しかも `model` に書き戻せない
    /// （呼び名では引けない）。固定しているのは
    /// `an_entry_carries_the_handle_as_the_id`。
    id: String,
    #[schema(schema_with = model_object_schema)]
    object: &'static str,
    /// 作成日時（Unix 秒）。OpenAI 互換クライアントが整数を期待する。
    created: i64,
    #[schema(schema_with = owned_by_schema)]
    owned_by: &'static str,
}

// 仕様書の閉じた綴り（作業単位 P-2a）。**値は上の定数から作る**（綴りを写さない）。
fn list_object_schema() -> Object {
    openapi::closed_string([LIST_OBJECT])
}

fn model_object_schema() -> Object {
    openapi::closed_string([MODEL_OBJECT])
}

fn owned_by_schema() -> Object {
    openapi::closed_string([OWNED_BY])
}

/// キャラクター一覧を OpenAI 互換の形で返す。
///
/// **1件も無ければ空の一覧**（エラーにしない）。
///
/// # **読めなかった行はここには並べない**（ADR-0016 §3 / 作業単位 0012b）
///
/// この一覧はクライアントのモデル選択 UI にそのまま並ぶ。壊れた行を混ぜると
/// **「選べるのに必ず失敗するモデル」**になる（チャットは `find_character_by_handle` を
/// 通るので `InvalidStoredValue` で落ちる）。しかも `id` に載せられる識別子が
/// 作れない行もある。
///
/// **黙って捨てているのではない。** 壊れた行は `Store::list_characters` が
/// 指し示し、修復の導線は `GET /admin/characters` 側にある。ここは
/// 「今そのまま話せるキャラクター」だけを並べる場所で、運用の窓口ではない。
#[utoipa::path(
    get,
    path = "/v1/models",
    summary = "List the characters as OpenAI-compatible models",
    tag = "openai",
    responses(
        (status = 200, body = ModelList),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn list_models(
    State(state): State<AppState>,
) -> Result<Json<ModelList>, ApiError> {
    let entries = state.store().list_characters().await.map_err(|err| {
        // **詳細はここ（ログ）にだけ出す。** レスポンスは定数のみ。
        // `Debug` は `#[source]` 経由でパスを含みうるので、出すのはログに限る。
        tracing::error!(error = %err, detail = ?err, "キャラクター一覧を取得できませんでした");
        ApiError::internal()
    })?;

    let mut data = Vec::with_capacity(entries.len());
    let mut unreadable = 0_usize;
    for entry in entries {
        match entry {
            // **しまったキャラクターは並べない**（ADR-0049 §決定1 / 作業単位 A-10）。
            // 並べると「選べるのに必ず 409 になるモデル」になる（chat は `model_archived` で断る）。
            // 黙って捨てているのではない —— `GET /admin/characters` が時刻つきで返す。
            CharacterEntry::Readable(character) if character.archived_at.is_some() => {}
            CharacterEntry::Readable(character) => data.push(ModelEntry {
                id: character.handle.as_str().to_owned(),
                object: MODEL_OBJECT,
                created: character.created_at.timestamp(),
                owned_by: OWNED_BY,
            }),
            // 個々の行の手掛かり（id と壊れた列）は `store` 側が既に `warn` に出している。
            CharacterEntry::Unreadable(_) => unreadable += 1,
        }
    }

    if unreadable > 0 {
        tracing::warn!(
            unreadable,
            "読み出せない行をモデル一覧から除きました。GET /admin/characters で確認できます"
        );
    }
    tracing::debug!(count = data.len(), "キャラクター一覧を返します");

    Ok(Json(ModelList {
        object: LIST_OBJECT,
        data,
    }))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::*;

    /// 空の一覧も `data: []` を持つ（`null` にしない）。
    #[test]
    fn an_empty_list_still_has_a_data_array() {
        let value = serde_json::to_value(ModelList {
            object: LIST_OBJECT,
            data: Vec::new(),
        })
        .expect("JSON にできること");

        assert_eq!(value, json!({"object": "list", "data": []}));
    }

    /// 1件の形。**識別子がそのまま `id`。**（ADR-0004。`/v1/models` の綴りは変えない）
    #[test]
    fn an_entry_carries_the_handle_as_the_id() {
        let created = Utc
            .with_ymd_and_hms(2026, 8, 14, 1, 2, 3)
            .single()
            .expect("実在する日時であること");
        let value = serde_json::to_value(ModelList {
            object: LIST_OBJECT,
            data: vec![ModelEntry {
                id: "葵 碧".to_owned(),
                object: MODEL_OBJECT,
                created: created.timestamp(),
                owned_by: OWNED_BY,
            }],
        })
        .expect("JSON にできること");

        assert_eq!(
            value,
            json!({
                "object": "list",
                "data": [{
                    "id": "葵 碧",
                    "object": "model",
                    "created": created.timestamp(),
                    "owned_by": "personacore",
                }],
            })
        );
    }
}
