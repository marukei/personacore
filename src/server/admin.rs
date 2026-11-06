//! `/admin/` 配下の管理 API（ADR-0008）。
//!
//! **キャラクターと性格ステータスの書き込み経路はここだけ。**
//! CLI も起動時シードも作らない（2つ目の書き込み経路になり、検証が二重管理になる）。
//! FE ができるまでは `scripts/seed-characters.ps1` がこの API のクライアントになる。
//!
//! # 置かないもの
//!
//! * **`memory` の行を消す・書き換えるエンドポイント。**
//!   不変条件2（`superseded_by` は時系列保持 / docs/03-data-model.md）は変わらない。
//!   **ADR-0022 が足したのは「読む入口」と「行を追加する入口」だけ**で、
//!   `DELETE` も `PUT` も生えていない（1件の記憶を指すパスすら無い。下の表）。
//!   見張り番は `tests/server_admin.rs::no_admin_endpoint_deletes_or_rewrites_a_memory`。
//!
//!   > **ADR-0008 は「`memory` を触る管理 API を作らない」と書いていた。**
//!   > 前提は「触らせなければ壊れない」だったが、**壊れた行を直せないまま
//!   > キャラクターが死ぬ**方が重い（13c-3b で実際に起きた）。**ADR-0022 が正。**
//! * **生の SQL。** 必要なクエリは `store` に足すこと。
//!   `memory` の SQL 走査テスト（`no_statement_updates_or_deletes_a_memory`）は
//!   **`src/**` 全体**を見るので、ここに書いた SQL も走査に掛かる
//!   （かつては `src/store/**` だけを見ており、このファイルは網の外にあった）。
//! * **キャラクターの識別子を変える経路。** `handle` は不変（ADR-0016 §1）。
//!   更新の要求型に識別子のフィールドを持たせないので、
//!   新しい識別子を `store` へ届ける経路は**型として存在しない。**
//! * **しまっていないキャラクターの会話を巻き添えにする削除。** しまっておらず会話が1件でもあれば
//!   `store` が拒否する（ADR-0016 §2 / ADR-0049 §決定1）。`/admin/` 側でその判定を作り直さない
//!   （判定と削除は `store` の1文で不可分に行われている）。**しまったキャラクターは中身ごと消える**
//!   ——完全に消す前に必ず「しまう」（`PATCH` の `archived`）を通るので、1回の誤操作では消えない。
//!
//! # パス設計
//!
//! | メソッド | パス | 用途 |
//! |---|---|---|
//! | `POST` | `/admin/characters` | キャラクターの作成 |
//! | `GET` | `/admin/characters` | キャラクターの一覧（**壊れた行を指し示す**） |
//! | `PATCH` | `/admin/characters/{handle}` | 表示名・ペルソナ・形式指示の**部分更新**（尺度・走らせ方・**見いだす枠の数**・**しまう／戻す**も） |
//! | `DELETE` | `/admin/characters/{handle}` | キャラクターの削除（**会話が無いか、しまってあるときだけ**。しまってあれば中身ごと） |
//! | `PUT` | `/admin/characters/{handle}/stats` | 性格ステータスの**丸ごと置き換え** |
//! | `GET` | `/admin/characters/{handle}/stats` | 性格ステータスの取得 |
//! | `PUT` | `/admin/characters/{handle}/attributes` | キャラクター属性の**丸ごと置き換え** |
//! | `GET` | `/admin/characters/{handle}/attributes` | キャラクター属性の取得 |
//! | `GET` | `/admin/characters/{handle}/attributes/revisions` | **属性の変更の履歴**（ADR-0028 §決定5 / **読むだけ**。総数と上限も返す） |
//! | `GET` | `/admin/characters/{handle}/agreements` | **取り決め（約束・決まり）の一覧**（ADR-0051 / 有効なものも終わったものも。上限も返す） |
//! | `POST` | `/admin/characters/{handle}/agreements` | 取り決めを**手で書く**（有効で始まる。**上限なら 409**） |
//! | `POST` | `/admin/characters/{handle}/agreements/{agreement_id}/transitions` | 取り決めを**手で終わらせる**（果たした・破った・取り消した。**行は消えない**） |
//! | `GET` | `/admin/characters/{handle}/agreements/transitions` | **取り決めの履歴**（**読むだけ**。総数と上限も返す） |
//! | `GET` | `/admin/characters/{handle}/drift` | **変容の履歴**（票と累積 / **読むだけ**） |
//! | `GET` | `/admin/characters/{handle}/axis-candidates` | **軸の候補**（ADR-0046 §決定6 / **読むだけ**。いまの状態・移り変わり・試用の票の内訳。総数と上限も返す） |
//! | `GET` | `/admin/characters/{handle}/memories` | **修復のための**記憶一覧（**壊れた行を指し示す**）。`?include=superseded` で**上書きされた記憶**も |
//! | `POST` | `/admin/characters/{handle}/memories/{memory_id}/corrections` | 記憶の**訂正版で置き換える** |
//! | `GET` | `/admin/characters/{handle}/sessions` | **会話の一覧**（作業単位 D-12c-1 / **読むだけ**。素の `aoi` が続ける順。総数と上限も返す） |
//! | `GET` | `/admin/characters/{handle}/sessions/{session_id}/messages` | **1つの会話のメッセージ**（**読むだけ**。古い順のページを `?before=` で遡る。総数も返す） |
//!
//! **⚠ `/admin/` の下にもう1本ある。ここには無い。**
//! `GET /admin/upstream`（**上流について観測できたこと** / ADR-0039 §決定4 / 作業単位 G-10）は
//! **`server::upstream` が持つ**——キャラクターにも `store` にも掛からない入口なので、
//! 観測を保持している側と同じファイルに置いてある
//! （**応答へ何を出すかの判断が、保持の判断と離れない**ようにするため）。
//!
//! 会話履歴（`GET .../sessions` と `.../sessions/{session_id}/messages`）にも**書き込みの口は無い。**
//! 会話を書くのは本文処理（`/v1/chat/completions`）だけで、`session.updated_at` の押し上げと
//! 不可分に行う（ADR-0004）。**1件のセッション・1件のメッセージを指すパス**
//! （`.../sessions/{session_id}` / `.../messages/{message_id}`）**も生やしていない**——
//! 生やせば `PUT` と `DELETE` が自然な形になり、**`message.raw_content` を消す道が表現できてしまう。**
//! 見張り番は `tests/server_admin_sessions.rs::only_get_is_exposed_for_the_session_paths`。
//!
//! 変容の履歴（`GET .../drift`）に**書き込みの口は無い。** 票は追記のみ
//! （ADR-0026 §決定3）で、書くのは本文処理だけである。
//! **1件を指すパス（`.../drift/{id}`）も生やしていない**——記憶の側とまったく同じ理由で、
//! 生やせば `PUT` と `DELETE` が自然な形になる。
//!
//! 記憶の訂正が `POST`（`PUT` / `DELETE` ではない）なのは、**起きるのが行の追加**だから
//! （ADR-0022: 新しい行を INSERT し、古い行にそれを指させる）。
//! **1件の記憶を指すパス（`.../memories/{memory_id}`）は生やしていない**
//! ——生やせば `PUT`（置き換え）と `DELETE`（削除）が自然な形になり、
//! **ADR-0022 が却下した2つの操作**がパスとして表現できてしまう。
//! **記憶の一覧は想起ではない**（ADR-0003 の「入口はひとつ」に反しない）。
//! 返すものはプロンプトに一切載らず、名前も `recall_memories` と分けてある。
//!
//! **上書きされた記憶を読む入口も、そこに足してある**（`?include=superseded` /
//! 作業単位 D-3b）。**2本目の経路は生やしていない**——
//! ADR-0022 の要は「消さずに時系列で残す」ことなのに、
//! **読み出す入口がどれも `superseded_by IS NULL` しか見ていなかった**ので、
//! 外からは「消さずに残す」が**書き込み専用**に見えていた。
//! **既定の応答は 14b の形のまま**（訊かなければ節ごと現れない）。
//!
//! 性格ステータスを `PUT`（`PATCH` ではない）にしているのは、
//! **`store` の意味論が丸ごと置き換え**（`replace_stat_allocs`）だから。
//! `PATCH` に見せると部分更新だと誤解され、「消したはずの軸が残る」と読まれる。
//! 逆にキャラクターの更新が `PATCH`（`PUT` ではない）なのは、
//! **`PUT` が「丸ごと置き換え」= 識別子を含む全フィールドを受ける形**に読まれるため。
//!
//! キャラクターは **id ではなく識別子（`handle`）で指す。** `model` フィールドに書くのも
//! 同じ値なので（ADR-0004）、利用者が既に手元に持っている綴りで操作できる。
//! 識別子は `[a-z0-9_-]` だけなので（ADR-0017）、パスのエスケープは要らない。
//!
//! # 引き当ては**全経路で完全一致**（0004b-2 以降）
//!
//! `PATCH` / `DELETE` / `GET` / チャットのどれもが、格納値と**完全一致**する行だけを引く。
//! かつて `PATCH` / `DELETE` だけが正規化キーで引いていた（0012a〜0012c の「引き当ての非対称」）
//! が、その抜け道は `migrations/0006_drop_character_name_key_idx.sql` と併せて畳んである。
//!
//! それでも更新の応答は「更新した行の id」と「読み直せた姿（`null` になりうる）」を分ける
//! （`UpdatedCharacterView`）。**`null` は失敗ではない。**
//! **どの場合に `null` になるかはここに書かない**——条件は `updated_character` の doc が持つ。
//! 2箇所に書くと、片方を直したときにもう片方が黙って古びる（この節が実際にそうなった）。

use axum::Json;
use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::openapi::schema::{Array, Object};
use utoipa::{IntoParams, ToSchema};

use super::error::{ApiError, ErrorBody};
use super::{AppState, openapi};
use crate::domain::{
    Agreement, AgreementContent, AgreementEdits, AgreementEnding, AgreementKeeper, AgreementKind,
    AgreementState, MAX_ACTIVE_AGREEMENTS, NewAgreement, SessionId,
};
use crate::domain::{
    AttributeKey, AttributeSet, AttributeValue, AxisCandidateState, AxisDiscoverySlots,
    AxisTrialFailure, Character, CharacterAttribute, CharacterColumn, CharacterEntry,
    CharacterUpdate, DisplayName, DriftDelta, DriftModeSource, FieldUpdate, Handle, Memory,
    MemoryCategory, MemoryColumn, MemoryContent, MemoryEntry, MemoryId, Message, MessageId,
    NewMemory, OutputFormat, Persona, ResolvedDriftMode, Role, StatAlloc, StatAxis, StatDriftMode,
    StatScale, Subject, SupersededMemory, UnreadableCharacter, UnreadableMemory,
};
use crate::store::{
    AttributeRevision, AxisCandidate, DriftVoteRecord, DriftVoteTally, MessagePage, SessionSummary,
    StoreError, StoredDriftCharge,
};

/// 本文を読めなかったときにログへ残す入口のラベル（**応答には載らない**）。
const ENDPOINT: &str = "admin";

/// キャラクター作成の要求。
///
/// **`deny_unknown_fields`。** 投入スクリプトの綴り間違いを黙って通すと、
/// 「設定したはずの値が効かない」形で後から表面化する。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateCharacterRequest {
    /// 識別子。`model` フィールドで指す値（ADR-0004）。
    /// `Handle::parse` を通す（未検証のまま `store` へ渡さない）。
    handle: String,
    /// **プロンプトに載る呼び名**（ADR-0011）。`DisplayName::parse` を通す。
    ///
    /// **省略可。省略時は `handle` の値で埋める。**
    /// 必須にすると、既に動いている投入クライアント（`scripts/seed-characters.ps1` と
    /// `scripts/characters.sample.json`）が `deny_unknown_fields` ではなく
    /// **欠落**で 400 になる。省略時に `handle` を使えば **0004a 以前とまったく同じ
    /// 振る舞い**（名前がそのままプロンプトに載る）になり、既存の要求の意味が変わらない。
    /// `handle` が `DisplayName` としても必ず正当であることは
    /// `domain` の `every_valid_handle_is_also_a_valid_display_name` が固定している。
    #[serde(default)]
    display_name: Option<String>,
    /// ペルソナ本文。`Persona::parse` を通す（同上）。
    ///
    /// **検証をこの型（`#[serde(deserialize_with = ...)]` 等）に持たせない。**
    /// 制約は `domain::Persona` の側にあり、ここは受け取るだけ。
    base_persona: String,
    /// 台詞 / 地の文の形式指示。省略可（未設定なら `prompt` が節ごと落とす）。
    ///
    /// **`OutputFormat::parse` を通す。** 空文字は「未設定」ではなくエラーで、
    /// 未設定はキーごと省く（`null` でもよい）。
    #[serde(default)]
    output_format_instruction: Option<String>,
    /// 性格ステータスの尺度（ADR-0015）。**省略時は `StatScale::DEFAULT`。**
    ///
    /// `StatScale::parse` を通す（未検証のまま `store` へ渡さない）。
    /// 既定を決めるのは**この入口だけ**（`store::create_character` は必ず値を要求する）。
    #[serde(default)]
    stat_scale: Option<u32>,
    /// **変容の走らせ方**（[ADR-0045](../../docs/adr/0045-drift-mode-per-character.md) §決定3）。
    ///
    /// **省略・`null` は未設定**（起動時の既定 `PERSONACORE_STAT_DRIFT` に従う）。
    /// **既定の値で埋めない**——埋めると「既定に従う」と「既定と同じ値を選んだ」が区別できない。
    ///
    /// `StatDriftMode::parse` を通す。**受けるのは `off` / `shadow` / `on` だけ**
    /// （環境変数の別名 `0` / `true` / `yes` 等は 400）。
    #[serde(default)]
    #[schema(schema_with = nullable_drift_mode_schema)]
    stat_drift_mode: Option<String>,
}

/// キャラクター更新の要求（**部分更新** / ADR-0016）。
///
/// # 識別子のフィールドが無いのは仕様
///
/// `character.handle` は不変（ADR-0016 §1）。`model` フィールドで指す値なので、
/// 変えると **FE 側に保存された `aoi@42` が指さなくなる。**
/// `domain::CharacterUpdate` と同じく**渡せない形**にしてあり、
/// 新しい識別子が `store` へ届く経路は型として存在しない。
///
/// # 識別子が要求に入っていたら、**要求まるごと拒否する**
///
/// `deny_unknown_fields`（`/admin/` の全入口で共通の方針）なので
/// `{"handle": "rin", "display_name": "凛"}` は 400 になり、**1列も書かれない。**
/// 「未知のキーは黙って無視する」形にすると、改名したつもりの利用者が
/// 200 を受け取り、**呼び名だけが変わって識別子は元のまま**の状態になる
/// ——ADR-0011 が分けた2つを混同したまま「成功した」と読ませるのが最悪。
///
/// **専用のエラーコードは足していない。** 足すには要求型に `handle` を持たせて
/// 「受け取ってから拒否する」形にするしかなく、それは「型で表現できなくする」を
/// 崩す（綴り違いの `name` / `id` も同じ扱いになるべきで、
/// 特別扱いできるのは今の綴りだけ、という点でも一貫しない）。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateCharacterRequest {
    /// プロンプトに載る呼び名。**キーごと省けば変えない。**
    ///
    /// `null` も「変えない」（`POST` 側で `null` を省略と同じに扱っているのに揃える）。
    /// この列は NOT NULL なので「消す」に意味が無く、3値にする理由が無い。
    #[serde(default)]
    display_name: Option<String>,
    /// ペルソナ本文。**キーごと省けば変えない。** `null` の扱いは表示名と同じ。
    #[serde(default)]
    base_persona: Option<String>,
    /// 台詞 / 地の文の形式指示。**この列だけ3値**（NULL になりうるため）。
    ///
    /// | JSON | 意味 |
    /// |---|---|
    /// | キーごと省略 | 変えない |
    /// | `"..."` | その値に設定する |
    /// | `null` | **未設定に戻す**（消す） |
    ///
    /// `null` = 削除は JSON Merge Patch（RFC 7396）と同じ約束で、
    /// **「変えない」と「消す」が別の表現になる**のが要点。同じ表現にすると、
    /// 表示名だけを変える更新のたびに形式指示が黙って消える。しかも消えても
    /// `prompt` が節ごと落とすだけなので、エラーにも警告にもならない。
    #[serde(default, deserialize_with = "nullable_field_update")]
    #[schema(value_type = Option<String>)]
    output_format_instruction: FieldUpdate<String>,
    /// 性格ステータスの尺度。**キーごと省けば変えない。**
    ///
    /// `null` も「変えない」（表示名・ペルソナと同じ）。この列は NOT NULL なので
    /// 「消す」に意味が無く、3値にする理由が無い。
    #[serde(default)]
    stat_scale: Option<u32>,
    /// **変容の走らせ方**（ADR-0045 §決定3）。**形式指示と同じ3値。**
    ///
    /// | JSON | 意味 |
    /// |---|---|
    /// | キーごと省略 | 変えない |
    /// | `"off"` / `"shadow"` / `"on"` | その値に設定する（**次のターンから効く**） |
    /// | `null` | **未設定に戻す**（起動時の既定に従う） |
    ///
    /// 綴りは `StatDriftMode::parse` を通す（別名は 400）。
    #[serde(default, deserialize_with = "nullable_field_update")]
    #[schema(schema_with = nullable_drift_mode_schema)]
    stat_drift_mode: FieldUpdate<String>,
    /// **見いだす枠の数 `m`**（[ADR-0046](../../docs/adr/0046-drift-proposes-new-axes.md) §決定1）。
    ///
    /// | JSON | 意味 |
    /// |---|---|
    /// | キーごと省略 | 変えない |
    /// | `null` | **変えない**（NOT NULL の列なので「消す」が無い / `stat_scale` と同じ約束） |
    /// | `0`〜`4` | その値に設定する |
    ///
    /// **範囲外は 400**——`5` 以上は `AxisDiscoverySlots::parse` が
    /// `axis_discovery_slots_too_large`、負の数・整数でない値は `u32` に読めず `invalid_body`。
    /// **作成（`POST`）では受けない**（Issue origin#41 は `PATCH` だけを求めた。作るときは既定の 0）。
    #[serde(default)]
    axis_discovery_slots: Option<u32>,
    /// **しまう／戻す**（[ADR-0049](../../docs/adr/0049-deleting-characters-and-conversations.md) §決定1 /
    /// 作業単位 A-10）。
    ///
    /// | JSON | 意味 |
    /// |---|---|
    /// | キーごと省略 | 変えない |
    /// | `null` | **変えない**（`stat_scale` と同じ約束） |
    /// | `true` | しまう（`GET /v1/models` と画面の一覧から外れ、chat は 409 `model_archived`）。**すでにしまってあれば時刻はそのまま** |
    /// | `false` | 戻す |
    ///
    /// **中身（会話・記憶・性格値など）は1行も変わらない。** しまったキャラクターは
    /// `DELETE` で完全に消せる（会話があっても）。**作成（`POST`）では受けない**（作るときは常にしまっていない）。
    #[serde(default)]
    archived: Option<bool>,
}

/// NULL になりうる列の3値を読む（`null` = 消す / 文字列 = 設定する）。
///
/// **形式指示と変容の走らせ方の2列が使う**（どちらも同じ約束 / 作業単位 A-8 で共有した）。
///
/// **キーが無い場合はこの関数が呼ばれず、`#[serde(default)]` の `Keep` になる。**
/// そこが「変えない」と「消す」を分ける仕掛けそのもの
/// （`Option<String>` を直に受けると `null` と欠落が同じ `None` に潰れる）。
fn nullable_field_update<'de, D>(deserializer: D) -> Result<FieldUpdate<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Option::<String>::deserialize(deserializer)? {
        Some(value) => FieldUpdate::Set(value),
        None => FieldUpdate::Clear,
    })
}

/// 性格ステータス置き換えの要求。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplaceStatsRequest {
    /// **この配列が全体。** 含まれない軸は消える。
    stats: Vec<StatAllocRequest>,
}

/// 1軸分の指定。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct StatAllocRequest {
    axis: String,
    /// **0 も正当な値。** 落とすのは `prompt` 層の責務なので、ここでは受理する。
    value: u32,
}

/// キャラクター属性の置き換えの要求（ADR-0009 / 作業単位 12a）。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplaceAttributesRequest {
    /// **この配列が全体。** 含まれない属性は消える。
    attributes: Vec<AttributeRequest>,
}

/// 1件分の属性の指定。
///
/// # `subject` を `String` で受ける理由
///
/// serde の enum（`#[serde(rename_all = "lowercase")]`）にすると、綴り違いは
/// **`JsonRejection` になって `invalid_body`（「リクエスト本文の形式が不正です」）**に潰れる。
/// `assistant`（`message.role` の綴り）は現実に起きる取り違えなので、
/// **どこを直せばよいかが分かる専用の理由**（`attribute_subject_unknown`）で返す。
/// 綴りの判定は `subject_of` が1箇所で行う。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct AttributeRequest {
    #[schema(schema_with = subject_schema)]
    subject: String,
    key: String,
    value: String,
}

/// 取り決めを**手で書く**要求（ADR-0051 §決定2・5 / 作業単位 A-12a）。書いたものは有効で始まる。
///
/// **3つとも必須**（省くと 400 `invalid_body`）。とくに `keeper` を省略できる形にしない
/// ——省略できる形にした時点で、守る側を落とした行が必ず生まれる（`memory.subject` と同じ理由）。
///
/// `kind` / `keeper` を `String` で受けるのは `AttributeRequest::subject` と同じ理由
/// （綴り違いを `invalid_body` に潰さず、直す欄が分かる理由で返す）。綴りの正は domain の `label`。
/// **状態も出どころも受けない**（`deny_unknown_fields` なので送れば 400）。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateAgreementRequest {
    /// 中身（要旨の平叙文）。上限は `AgreementContent::MAX_CHARS`。
    content: String,
    /// 種類（`promise` / `rule`）。
    #[schema(schema_with = agreement_kind_schema)]
    kind: String,
    /// 守る側（`user` / `character` / `both`）。
    #[schema(schema_with = agreement_keeper_schema)]
    keeper: String,
}

/// **提案を採る**要求（ADR-0051 §決定2「直してから採る」/ 作業単位 A-12c-1）。
///
/// **3つとも省ける**（省いた欄は提案のまま。`{}` は「そのまま採る」）。綴りの扱いは
/// [`CreateAgreementRequest`] と同じ（`String` で受け、直す欄が分かる理由で返す）。
/// **状態は受けない**（`deny_unknown_fields` なので送れば 400 —— 採ると必ず有効になる）。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct AdoptAgreementRequest {
    /// 直した中身。上限は `AgreementContent::MAX_CHARS`。
    #[serde(default)]
    content: Option<String>,
    /// 直した種類（`promise` / `rule`）。
    #[serde(default)]
    #[schema(schema_with = agreement_kind_schema)]
    kind: Option<String>,
    /// 直した守る側（`user` / `character` / `both`）。
    #[serde(default)]
    #[schema(schema_with = agreement_keeper_schema)]
    keeper: Option<String>,
}

/// 取り決めを**手で終わらせる**要求（ADR-0051 §決め直し 3 / 作業単位 A-12a）。
///
/// `state` は終わった後の状態（`fulfilled` / `broken` / `withdrawn`）。**`active` は受けない**
/// （400 `agreement_ending_unknown`）。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct EndAgreementRequest {
    #[schema(schema_with = agreement_ending_schema)]
    state: String,
}

/// 記憶の**訂正**の要求（ADR-0022 / 作業単位 14b）。
///
/// # **訂正版の中身が必須**（後継の無い取り消しを作らない / ADR-0022 §1）
///
/// `content` に `#[serde(default)]` を**付けない。** 省くと 400（`invalid_body`）になり、
/// 「この記憶を無かったことにする」だけの要求は**型として組み立てられない。**
/// 壊れた行（本文が空白のみ等）でも訂正版を書かせる立場を取る
/// ——「消したい」は「今は何が正しいか」を言えるはず、という判断。
///
/// **`superseded` を本文に持たない。** 訂正する相手はパスが決める。
/// 本文にも書けると、パスと食い違ったときにどちらが勝つかを決める羽目になる。
///
/// **`locked` を受けない**（ADR-0022 §4。意味が未確定なので書き込まない。
/// 訂正版は DB の既定値 0 で入る）。`deny_unknown_fields` なので、
/// 送られてきたら 400 になる。
///
/// **見張り番は `tests/server_admin_memories.rs::
/// an_unknown_key_in_an_otherwise_valid_correction_is_refused_and_writes_nothing`。**
/// **本文が正当でなければこの枝には届かない**——`content` を欠かせた要求は
/// `deny_unknown_fields` が無くても `missing field content` で同じ `invalid_body` になる
/// （14b の着手時、未知のキーの枝はどのテストも一度も通っていなかった）。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CorrectMemoryRequest {
    /// **誰についての記憶か。** 省略できない（絶対制約）。
    ///
    /// 綴りの判定は `subject_of`（属性と共有する1箇所）。
    #[schema(schema_with = subject_schema)]
    subject: String,
    /// 訂正版の本文。**省略できない**（上記）。
    content: String,
    /// 分類。省略・`null` は「未設定」。
    #[serde(default)]
    category: Option<String>,
}

/// **記憶の一覧の問い合わせパラメータ**（ADR-0022 / 作業単位 D-3b）。
///
/// # なぜ経路を増やさず、選択肢を足したのか
///
/// `.../memories/history` のような**別の経路**を生やすと、
/// 「そのキャラクターの記憶を読む入口」が2つになり、**どちらが正かが分かれる。**
/// 画面は結局2つを並べて出すので、**同じ絞り込みの2つの写し**を抱えることになる。
///
/// # `deny_unknown_fields`（**知らない欄は 400**）
///
/// 本文の要求型と同じ方針。綴りを間違えた問い合わせを黙って捨てると、
/// **送った側は「求めたものが1件も無かった」と読む。**
/// 見張り番は `tests/server_admin_memories.rs::
/// an_unknown_query_parameter_is_a_bad_request_even_for_an_unknown_character`。
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub(super) struct MemoryListQuery {
    /// 既定の一覧に**足して返すもの**。省略なら足さない（**既定の応答は変わらない**）。
    ///
    /// **知らない綴りは 400**（列挙なので `serde` が弾く）。
    ///
    /// **仕様書では値の一覧をその場に書く**（`inline`）——`IntoParams` は、欄の型を
    /// `$ref` で指すのに**その型の定義を `components` に集めない**（`utoipa` 6.0.1 で実測 /
    /// `openapi::tests::every_reference_points_inside_the_document` が赤にした）。
    #[serde(default)]
    #[param(inline)]
    include: Option<MemoryListInclude>,
}

/// 一覧に足せるもの（作業単位 D-3b）。**いまは1つだけ。**
///
/// 列挙にしてあるのは、**知らない綴りをその場で落とす**ため
/// （`bool` の欄を並べる形にすると、綴り違いが `deny_unknown_fields` 頼みになり、
/// 欄が増えるたびに組み合わせが増える）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum MemoryListInclude {
    /// **上書きされた記憶**（`superseded_by` が付いた行）も一緒に返す。
    Superseded,
}

/// **上書きされた記憶を何件返すか**（作業単位 D-3b）。
///
/// # なぜ上限が要るのか
///
/// **行は消えない**（時系列保持 / 絶対制約）。訂正するたびに1件ずつ増え、
/// **`memory` は単調増加する**（ADR-0022 §結果）。上限が無いと、応答は
/// そのキャラクターを直した回数に比例して伸び続ける。
///
/// # なぜ 100 なのか
///
/// **有効な記憶の一覧の側には上限が無い**（`GET .../memories` は全件返す）。
/// 上書き済みだけを厳しく切ると、**同じ画面に並ぶ2つの一覧の性格が食い違う。**
/// 一方で訂正は**人が1件ずつ叩く操作**なので、票（1ターンに軸の数だけ立つ /
/// `RECENT_DRIFT_VOTES`）ほど速くは増えない。
/// 100 は「**これまでの訂正をだいたい全部見る**」ための桁で、
/// それを超えたぶんは `total` との差として読める。
///
/// **⚠ 実測由来の値ではない。**
/// この機で測った時間でもモデルの都合でもないので、
/// [ADR-0036](../../docs/adr/0036-the-range-of-machines-is-a-constraint.md) の
/// 「機に依存する定数」ではなく、**`MEASURED_*` の作法には乗せない。**
/// 外したときに起きるのは「一覧が長い／短い」だけで、**壊れない。**
///
/// # ページ送りは足していない（**足さないと決めた**）
///
/// 足すには**カーソルの意味**（同着をどう跨ぐか・途中で訂正が増えたらどうなるか）を
/// 決める必要があり、この節の用途（**さっき何を直したか**を見る）には要らない。
/// **総数を一緒に返してある**ので、画面は「全部で M 件のうち直近 N 件」と言える
/// ——**言えないまま切ると、画面は「これで全部」と嘘をつく。**
///
/// 全部を読みたくなったら、そのときに `?before=` の形で足せる
/// （**並びが決まっている** / [`SupersededListView::memories`] の doc）。
///
/// # **期待値をこの定数から作らないこと**
///
/// 作ると、**この定数を倍にする変異が緑を通る**（CLAUDE.md `16b` の実測）。
/// 権威は `tests/server_admin_memories.rs::
/// only_the_most_recent_superseded_memories_come_back_but_the_total_counts_them_all` で、
/// **この定数より多い行を仕込んで、外の値（直書きの 100 と 113）**で押さえている。
const RECENT_SUPERSEDED_MEMORIES: u32 = 100;

/// 修復のための記憶一覧の応答（ADR-0022 §2 / 作業単位 14b）。
///
/// # **これは想起ではない**
///
/// 想起の入口は `Store::recall_memories` ひとつだけ（ADR-0003）。
/// この応答は**プロンプトに一切載らない**管理のための一覧で、
/// 名前も置き場所も想起と分けてある。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct MemoryListView {
    /// どのキャラクターのものか（パスを読み違えていないことが応答から分かる）。
    character: String,
    /// 読めた記憶。**有効な行だけ**（上書き済みは `store` が外す）。
    memories: Vec<MemoryView>,
    /// **読み出せなかった行**（`CharacterListView::unreadable` と同じ形）。
    ///
    /// 空でも**キーごと落とさない**（壊れた行が無いことと、この応答が
    /// 壊れた行を見せない古い版であることを、受け取った側が区別できる）。
    unreadable: Vec<UnreadableMemoryView>,
    /// **上書きされた記憶**（`?include=superseded` を付けたときだけ / 作業単位 D-3b）。
    ///
    /// # ここだけ「空ならキーごと落とす」ではなく「訊かなければキーごと無い」
    ///
    /// `unreadable` と扱いが違うのは、**訊かなくても返る節ではない**から。
    /// **訊いたのは受け取った側自身**なので、
    /// 「訊いていないから無い」と「訊いたが1件も無い」は区別できる
    /// （後者は `{"memories": [], "unreadable": [], "total": 0, "limit": …}` になる）。
    ///
    /// **既定の応答を1バイトも変えないため**でもある——
    /// 常に載せると、上書き済みの行を持たない既存の呼び出し側にも新しいキーが増える。
    /// 見張り番は `tests/server_admin_memories.rs::
    /// by_default_the_listing_says_nothing_about_superseded_memories`。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    superseded: Option<SupersededListView>,
}

/// **上書きされた記憶の節**（ADR-0022 / 作業単位 D-3b）。
///
/// # これは想起でも、修復の対象でもない
///
/// 並ぶのは**想起の条件（`superseded_by IS NULL`）から外れた行**——
/// つまり**二度と想起に載らない行**である。
/// **もう訂正もできない**（`supersede_memory` は `MemoryAlreadySuperseded` で拒む）。
/// **読むためだけの記録**で、ADR-0022 の「消さずに時系列で残す」の
/// **残した側**にあたる。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct SupersededListView {
    /// 読めた行。**新しい順**（`created_at` の降順、同着は `id` の降順）。
    ///
    /// 並びを決めているのは `store` の1文（`Store::list_superseded_memories`）で、
    /// **ここで並べ替えない。** 同着は**本番で普通に起きる**
    /// （1ターンで最大5件の記憶が入るので、まとめて訂正すれば時刻が並ぶ）。
    ///
    /// **並ぶのは「訂正した時刻」ではない。** いつ上書きされたかは保存していないので、
    /// **その記憶が書かれた時刻**である。
    memories: Vec<SupersededMemoryView>,
    /// **読み出せなかった行。** 上書き済みでも指し示す。
    ///
    /// 読み飛ばすと**壊れた環だけが鎖から落ち、`total` にだけ残る**
    /// ——読み手には数が合わないとしか見えない。
    unreadable: Vec<UnreadableSupersededMemoryView>,
    /// **保存されている上書き済みの行の総数。** 返した件数**ではない。**
    ///
    /// 返した件数を総数と呼ぶと、**画面が「これで全部」と嘘をつく。**
    /// **読めなかった行も数に入る**（総数は主語も本文も読まない）。
    total: u32,
    /// この節に載る上限（[`RECENT_SUPERSEDED_MEMORIES`]）。
    ///
    /// **`total` との差が「見えていない行」である。**
    /// 件数だけでは「上限で切られたのか、たまたまその数なのか」が読めない。
    limit: u32,
}

/// 上書きされた記憶1件の応答（作業単位 D-3b）。
///
/// # `MemoryView` を**包む**（写さない）
///
/// 有効な行と**同じ形**で、`superseded_by` が1つ増えるだけ。
/// 列を書き写すと、**`locked` を落とす判断（ADR-0022 §4）が片側から消える**
/// ——`MemoryView::from` を通す限り、その決定は1箇所のままである。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct SupersededMemoryView {
    /// 有効な行と同じ欄（`id` / `subject` / `content` / `category` / `created_at`）。
    #[serde(flatten)]
    memory: MemoryView,
    /// **どの行に置き換わったか。**
    ///
    /// **落とさないこと。** 落とすと鎖が辿れず、残るのは
    /// 「消えた記憶の山」でしかなくなる——「以前はXだったが今はY」の
    /// **Y へ辿る唯一の手掛かり**である。
    superseded_by: i64,
}

/// 上書きされた、しかも読み出せなかった記憶1件の応答（作業単位 D-3b）。
///
/// **格納値そのものは載せない**（`UnreadableMemoryView` と同じ）。
/// **`superseded_by` は載せる**——`parse` を通さない列なので、
/// **行が壊れていても後継は分かる。** 鎖はここで切れない。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct UnreadableSupersededMemoryView {
    /// `id` と、読めなかった列。
    #[serde(flatten)]
    broken: UnreadableMemoryView,
    /// **どの行に置き換わったか。**
    superseded_by: i64,
}

/// 記憶1件の応答。
///
/// # **`locked` を載せない**（ADR-0022 §4 / 司令塔の判断 / 作業単位 14b）
///
/// **`locked` が何を意味するかはまだ決まっていない。** 決まっているのは
/// 「0 / 1 以外なら読めない行」だけである。**意味の未確定な値を API に出すと、
/// 外から意味を決められてしまう**（受け取った側が独自の解釈で分岐を書き、
/// 後から意味を決めるときにその解釈と衝突する）。
/// `MemoryEntry::Readable(Memory)` は `locked` を持っているので、**この型で落とす。**
///
/// **壊れた列としては指し示す**（`UnreadableMemoryView::invalid_columns` に
/// `"locked"` が出る）。**値を出すことと、壊れている列を名指すことは別**
/// ——後者が無いと、チャットと抽出を止めている当の行が修復の入口から見えない。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct MemoryView {
    /// 主キー。**訂正のパスに貼る値**（一覧 → 修復が1本の線になる）。
    id: i64,
    /// **誰についての記憶か。必ず載せる**（絶対制約 / `AttributeView` と同じ理由）。
    ///
    /// 落とすと、応答を見た側が「猫が好き」を誰のものか判断できない
    /// ——訂正版を書くときに主語を取り違え、一人称の混線がそのまま固定される。
    #[schema(schema_with = subject_schema)]
    subject: &'static str,
    content: String,
    /// 未設定は `null`（キーごと落とさない。設定済みとの区別が付かなくなる）。
    #[schema(required = true)]
    category: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<Memory> for MemoryView {
    fn from(memory: Memory) -> Self {
        Self {
            id: memory.id.get(),
            subject: subject_to_wire(memory.subject),
            content: memory.content.as_str().to_owned(),
            category: memory.category.map(|category| category.as_str().to_owned()),
            created_at: memory.created_at,
            // **`memory.locked` はここで落ちる**（上記）。
        }
    }
}

/// 読み出せなかった記憶1件の応答。**修復に要る手掛かりだけ。**
///
/// **格納値そのものは載せない**（`store` が返していない。理由は
/// `domain::UnreadableMemory` の doc）。中身を見たいときは `id` で DB を直接見る。
///
/// **`handle` に相当するフィールドは無い。** キャラクターの一覧では
/// 「識別子そのものが壊れていて API からは指せない行」がありえたが、
/// 記憶の主キーは `parse` を通さないので**指せない行が無い**
/// （`UnreadableMemory` の doc）。この `id` は必ず訂正のパスに貼れる。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct UnreadableMemoryView {
    /// 主キー。**この行を指す唯一の確実な手掛かり。**
    id: i64,
    /// 読めなかった列。**DB の列名と同じ綴り。**
    #[schema(schema_with = memory_columns_schema)]
    invalid_columns: Vec<&'static str>,
}

impl From<UnreadableMemory> for UnreadableMemoryView {
    fn from(broken: UnreadableMemory) -> Self {
        Self {
            id: broken.id.get(),
            invalid_columns: broken
                .invalid_columns
                .into_iter()
                .map(MemoryColumn::as_str)
                .collect(),
        }
    }
}

/// 訂正の応答（ADR-0022 / 作業単位 14b）。**id を2つ返す。**
///
/// # なぜ訂正版の姿を返さないのか
///
/// `store` に「記憶を1件引く」経路が無く、返すには一覧をもう一度引くことになる。
/// **一覧は次の `GET` で取れる**ので、ここでは「どの行が何に置き換わったか」だけを返す。
///
/// **`superseded_id` を落とさないこと。** 要求のパスに書いた値と同じだが、
/// **応答だけを記録している運用者**（スクリプトのログ）が、
/// どの行が無効になったかを後から辿れる唯一の手掛かりになる。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct CorrectionView {
    /// どのキャラクターのものか（パスを読み違えていないことが応答から分かる）。
    character: String,
    /// **想起から外れた**行。消えてはいない（時系列保持）。
    superseded_id: i64,
    /// 新しく書かれた行。**次に直すときはこの id を指す。**
    correction_id: i64,
}

/// キャラクター属性の応答。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AttributesView {
    /// どのキャラクターのものか（パスを読み違えていないことが応答から分かる）。
    character: String,
    /// 属性の一覧。**空でもキーごと落とさない**（`characters` / `unreadable` と同じ理由）。
    attributes: Vec<AttributeView>,
}

/// 1件分の属性の応答。
///
/// **`subject` を必ず載せる**（ADR-0009 / CLAUDE.md 絶対制約と同じ理由）。
/// 落とすと、応答を見た側が「年齢: 19」を誰のものか判断できない。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AttributeView {
    #[schema(schema_with = subject_schema)]
    subject: &'static str,
    key: String,
    value: String,
}

/// `revisions` に載る上限（[ADR-0028](../../docs/adr/0028-attributes-that-change.md) §決定5）。
///
/// 履歴は**追記のみで消えない**ので、全件返すと応答が編集回数に比例して伸び続ける。
///
/// # **期待値をこの定数から作らないこと**
///
/// 作ると、**この定数を倍にする変異が緑を通る**（CLAUDE.md `16b` の実測）。
/// 権威は `tests/server_admin_attributes.rs::
/// only_the_newest_revisions_come_back_but_the_total_counts_them_all` で、
/// **この定数より多い履歴を積んで、外の値（直書きの 208 と 200）**で押さえている。
///
/// > **⚠ `tests/store_attribute.rs::only_the_newest_revisions_come_back` は権威ではない。**
/// > あちらは `attribute_revisions` に**上限を直接渡している**ので、この定数を一度も見ない
/// > ——**「見ている」と書いてあったのは偽**だった（A-3a のレビューで訂正 / 2026-09-09。
/// > **この定数を 5 にする変異が 2046 全緑を通った**）。
const ATTRIBUTE_REVISIONS_LIMIT: u32 = 200;

/// 属性の履歴の応答（ADR-0028 §決定5 / 作業単位 A-3a）。
///
/// **`GET .../attributes` の上位互換ではない。** あちらが答えるのは「いまどうなっているか」で、
/// ここが答えるのは「**いつ何から何に変わったか**」である。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AttributeRevisionsView {
    /// どのキャラクターのものか（パスを読み違えていないことが応答から分かる）。
    character: String,
    /// **新しい順**（`created_at DESC, id DESC`）。空でもキーごと落とさない。
    revisions: Vec<AttributeRevisionView>,
    /// **保存されている履歴の総数。** `revisions` の件数**ではない。**
    ///
    /// 返した件数を総数と呼ぶと、**画面が「これで全部」と嘘をつく。**
    /// **読めなかった行も数に入る**（数え上げは主語も値も時刻も読まない）。
    ///
    /// > **⚠ これが無いと [`AttributeRevisionsView::revisions_limit`] は何も守れない。**
    /// > 上限を名乗るだけでは、**返した件数が上限と同じとき**に
    /// > 「たまたまその数」と「切られた」を区別できない
    /// > ——**差を取る相手が要る**（`DriftView::total_votes` と同じ形）。
    total_revisions: u32,
    /// [`ATTRIBUTE_REVISIONS_LIMIT`]。
    ///
    /// **`total_revisions` との差が「見えていない件数」である。**
    /// 件数だけでは「上限で切られたのか、たまたまその数なのか」が読めない
    /// （[`DriftView::recent_limit`] と同じ理由）。
    revisions_limit: u32,
}

/// 属性の変更1件の応答。
///
/// **`subject` を必ず載せる**（ADR-0028 §決定4）。落とすと、
/// 「年齢が 19 から 20 になった」が誰の話なのか読めない。
///
/// **`before` / `after` の `null` は「無かった」「消えた」。**
/// キーごと落とすと**追加と削除の区別が消える**ので、`Option` のまま載せる。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AttributeRevisionView {
    /// 主キー。**`sqlite3` で同じ行を指せる唯一の手掛かり**
    /// （1件を指すパスは生やしていないので、貼る先はそこだけ）。
    id: i64,
    #[schema(schema_with = subject_schema)]
    subject: &'static str,
    key: String,
    /// 変更前の値。`null` は**その属性が無かった**こと。
    #[schema(required = true)]
    before: Option<String>,
    /// 変更後の値。`null` は**消えた**こと。
    #[schema(required = true)]
    after: Option<String>,
    created_at: DateTime<Utc>,
}

/// 取り決めの一覧の応答（ADR-0051 §決定5 / 作業単位 A-12a）。**有効なものも終わったものも**、書いた順。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AgreementsView {
    /// どのキャラクターのものか（パスを読み違えていないことが応答から分かる）。
    character: String,
    /// 書いた順（`id` の昇順）。**空でもキーごと落とさない。**
    agreements: Vec<AgreementView>,
    /// **有効な取り決めの件数の上限**（`domain::MAX_ACTIVE_AGREEMENTS`）。
    ///
    /// 画面はこれを読んで、上限に達したら書く欄を止める（**画面に写しを持たせない**）。
    active_limit: u32,
    /// **直近のターンで、有効な取り決めが何件本文に載り、何件落ちたか**（作業単位 A-12a の決め直し）。
    ///
    /// 取り決めは予算に入りきらないときだけ**古い側から**落ちる（優先の枠）。落ちたことを
    /// 画面に出すための欄。**この起動でまだそのキャラクターのターンが無ければ `null`**
    /// （DB に持たないので、起動し直すと空になる）。
    #[schema(required = true)]
    last_turn: Option<AgreementTurnView>,
}

/// 直近のターンの取り決めの載り方（作業単位 A-12a の決め直し）。
///
/// **「直近のターン」は、本文のプロンプトを組み立てた最後の回**（上流が失敗した回も含む）。
/// その後に書いた・終わらせた取り決めは、ここには反映されていない（`at` で読む）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AgreementTurnView {
    /// 本文に載った件数（新しい側から）。
    included: u32,
    /// 予算に入りきらずに落ちた件数（古い側から）。**0 でなければ、古い側の取り決めは本文に無かった。**
    dropped: u32,
    /// そのターンの時刻。
    at: DateTime<Utc>,
}

/// 取り決め1件の応答。**`keeper` を必ず載せる**（落とすと誰が守るのかが読めない）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AgreementView {
    id: i64,
    content: String,
    #[schema(schema_with = agreement_kind_schema)]
    kind: &'static str,
    #[schema(schema_with = agreement_keeper_schema)]
    keeper: &'static str,
    /// いまの状態。**本文に載るのは `active` だけ。**
    #[schema(schema_with = agreement_state_schema)]
    state: &'static str,
    /// 生まれた会話。**手で書いたものは `null`**（キーごと落とさない）。
    #[schema(required = true)]
    source_session_id: Option<i64>,
    created_at: DateTime<Utc>,
}

/// 取り決めを終わらせた結果（作業単位 A-12a）。
///
/// **取り決めの姿を読み直して返さない。** 終わらせる口は中身を読まない（中身が壊れた行も
/// 終わらせられるように / `Store::end_agreement` の doc）ので、読み直すと、その道がここで閉じる。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AgreementEndedView {
    /// どのキャラクターのものか。
    character: String,
    /// 終わらせた取り決め。
    agreement_id: i64,
    /// 終わった後の状態。
    #[schema(schema_with = agreement_state_schema)]
    state: &'static str,
}

/// 提案を捨てた結果（作業単位 A-12c-1）。**取り決めの姿を読み直して返さない**
/// （捨てる口は中身を読まない —— [`AgreementEndedView`] と同じ理由）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AgreementRejectedView {
    /// どのキャラクターのものか。
    character: String,
    /// 捨てた取り決め。
    agreement_id: i64,
    /// 捨てた後の状態（いつも `rejected`）。
    #[schema(schema_with = agreement_state_schema)]
    state: &'static str,
}

/// `transitions` に載る上限（作業単位 A-12a）。
///
/// 履歴は追記のみで消えないので、全件返すと応答が伸び続ける。**200 の根拠は無い**
/// —— 属性の履歴（`ATTRIBUTE_REVISIONS_LIMIT`）と同じ値にした。外しても遅くなるだけで壊れない
/// （見えない件数は `total_transitions` との差で読める）。
/// **期待値をこの定数から作らないこと**（権威は `tests/server_admin_agreements.rs` の直書きの値）。
const AGREEMENT_TRANSITIONS_LIMIT: u32 = 200;

/// 取り決めの履歴の応答（作業単位 A-12a）。**新しい順。プロンプトには載らない。**
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AgreementTransitionsView {
    character: String,
    /// **新しい順**（`created_at DESC, id DESC`）。空でもキーごと落とさない。
    transitions: Vec<AgreementTransitionView>,
    /// **保存されている履歴の総数**（`transitions` の件数ではない）。
    total_transitions: u32,
    /// [`AGREEMENT_TRANSITIONS_LIMIT`]。**`total_transitions` との差が「見えていない件数」。**
    transitions_limit: u32,
}

/// 状態の変化1件の応答（その取り決めの中身・種類・守る側と結んで返す）。
///
/// **`before` の `null` は「書かれた」**（無かった → `after`）。キーごと落とさない。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AgreementTransitionView {
    id: i64,
    agreement_id: i64,
    content: String,
    #[schema(schema_with = agreement_kind_schema)]
    kind: &'static str,
    #[schema(schema_with = agreement_keeper_schema)]
    keeper: &'static str,
    #[schema(schema_with = nullable_agreement_state_schema, required = true)]
    before: Option<&'static str>,
    #[schema(schema_with = agreement_state_schema)]
    after: &'static str,
    created_at: DateTime<Utc>,
}

/// `candidates` に載る上限（ADR-0046 §決定6 / 作業単位 A-9a）。
///
/// 候補は**追記のみで消えない**（不合格・却下も残る）ので、全件返すと応答が伸び続ける。
/// **50 の根拠は無い**——候補は記憶が `N` 件溜まるごとに高々1件しか増えない（ADR-0046 §決定2）ので、
/// 画面が要るのは最近の数件で、50 はその十分な上。**外したときに遅くなるだけで、壊れない**
/// （見えない件数は `total_candidates` との差で読める）。
///
/// **期待値をこの定数から作らないこと**（作ると、この定数を倍にする変異が緑を通る）。
/// 権威は `tests/server_admin_axis_candidates.rs::
/// only_the_newest_candidates_come_back_but_the_total_counts_them_all`（外の値 52 と 50）。
const AXIS_CANDIDATES_LIMIT: u32 = 50;

/// 軸の候補の一覧の応答（ADR-0046 §決定6 / 作業単位 A-9a）。**プロンプトには載らない。**
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AxisCandidatesView {
    /// どのキャラクターのものか（パスを読み違えていないことが応答から分かる）。
    character: String,
    /// **新しい順**（`created_at DESC, id DESC`）。空でもキーごと落とさない。
    candidates: Vec<AxisCandidateView>,
    /// **保存されている候補の総数。** `candidates` の件数**ではない**（`total_revisions` と同じ理由）。
    total_candidates: u32,
    /// [`AXIS_CANDIDATES_LIMIT`]。`total_candidates` との差が「見えていない件数」である。
    candidates_limit: u32,
}

/// 軸の候補1件の応答。
///
/// **`state` は保存された値ではない。** 移り変わりの列から導いたもの（`transitions` の最後）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AxisCandidateView {
    /// 主キー。**`sqlite3` で同じ行を指せる唯一の手掛かり**（1件を指すパスは生やしていない）。
    id: i64,
    /// 候補の名前（`StatAxis` の検証を通っている）。
    name: String,
    /// 出どころの記憶（ADR-0046 §決定2）。**同じキャラクターの記憶**（複合外部キー）。
    source_memory_id: i64,
    /// 候補が生まれた時刻。
    created_at: DateTime<Utc>,
    /// **いまの状態**（`trial` / `passed` / `failed` / `adopted` / `rejected`）。
    /// 綴りの正は `AxisCandidateState::label`。
    #[schema(schema_with = candidate_state_schema)]
    state: &'static str,
    /// 不合格の理由（`pinned` / `silent` / `too_few_samples`）。**不合格以外は `null`**（キーごと落とさない）。
    #[schema(schema_with = trial_failure_schema, required = true)]
    reason: Option<&'static str>,
    /// 状態の移り変わり（**古い順**。先頭は必ず `trial`）。
    transitions: Vec<AxisCandidateTransitionView>,
    /// 試用の票の内訳（ADR-0046 §決定5「上げ・下げ・`0` の件数を添える」）。
    trial_votes: AxisTrialVotesView,
}

/// 移り変わり1件の応答。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AxisCandidateTransitionView {
    #[schema(schema_with = candidate_state_schema)]
    state: &'static str,
    /// 不合格の理由。不合格以外は `null`。
    #[schema(schema_with = trial_failure_schema, required = true)]
    reason: Option<&'static str>,
    created_at: DateTime<Utc>,
}

/// 試用の票の内訳の応答。**変容の票（`GET .../drift`）とは別の数**（ADR-0046 §決定6）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct AxisTrialVotesView {
    up: u32,
    down: u32,
    zero: u32,
}

/// キャラクター1件の応答。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct CharacterView {
    /// 主キー。`store` がゴースト行の `warn` ログに出す id と突き合わせられる。
    id: i64,
    handle: String,
    /// プロンプトに載る呼び名。**`handle` と同じ値でも省かない**
    /// （同じかどうかは応答を見た側が判断する）。
    display_name: String,
    base_persona: String,
    /// 未設定は `null`（キーごと落とさない。設定済みとの区別が付かなくなる）。
    #[schema(required = true)]
    output_format_instruction: Option<String>,
    /// 性格ステータスの尺度（ADR-0015）。**省略時の既定でも省かない**
    /// （既定なのか明示なのかは応答を見た側が判断する）。
    stat_scale: u32,
    /// **書いた変容の走らせ方**（ADR-0045）。**未設定は `null`**（キーごと落とさない）。
    ///
    /// **効いている値ではない。** 未設定のときに効く値（起動時の既定）は
    /// 変容の応答（`GET .../drift` の [`DriftView::run_mode`]）が言う。
    /// ここに既定を書くと、「既定に従う」と「既定と同じ値を選んだ」が区別できない。
    ///
    /// 綴りの正は `StatDriftMode::label`（写しを作らない）。
    #[schema(schema_with = nullable_drift_mode_schema, required = true)]
    stat_drift_mode: Option<&'static str>,
    /// **見いだす枠の数**（ADR-0046 §決定1）。**既定の 0 でも省かない**（`stat_scale` と同じ理由）。
    axis_discovery_slots: u32,
    created_at: DateTime<Utc>,
    /// **しまった時刻**（ADR-0049 §決定1 / 作業単位 A-10）。**しまっていなければ `null`**（キーごと落とさない）。
    ///
    /// しまったキャラクターも `GET /admin/characters` には出る（しまった一覧の元）。
    /// 出し分けるのは読み手（画面）で、`GET /v1/models` はサーバが外す。
    #[schema(required = true)]
    archived_at: Option<DateTime<Utc>>,
}

impl From<Character> for CharacterView {
    fn from(character: Character) -> Self {
        Self {
            id: character.id.get(),
            handle: character.handle.as_str().to_owned(),
            display_name: character.display_name.as_str().to_owned(),
            base_persona: character.base_persona.as_str().to_owned(),
            output_format_instruction: character
                .output_format_instruction
                .map(|format| format.as_str().to_owned()),
            stat_scale: character.stat_scale.get(),
            stat_drift_mode: character.stat_drift_mode.map(StatDriftMode::label),
            axis_discovery_slots: character.axis_discovery_slots.get(),
            created_at: character.created_at,
            archived_at: character.archived_at,
        }
    }
}

/// キャラクター更新の応答（ADR-0016 / 作業単位 0012c）。
///
/// # なぜ `CharacterView` をそのまま返さないのか
///
/// **更新した行が読み出せるとは限らない**（条件は `updated_character` の doc。
/// **ここには写さない**）。`CharacterView` を必ず返す形にすると、
/// 壊れた列がまだ残っている行——たとえば2列が壊れていて片方だけを直した直後——を
/// 404 か 500 に化けさせることになる。
///
/// **更新そのものは成功している。** 失敗に見せると修復の途中で手が止まる。
///
/// # `null` を受け取った側の次の一手
///
/// どちらのケースかは **`GET /admin/characters` の `unreadable[].invalid_columns`** で割れる。
/// **応答にそれを示すフィールドは足していない**（`store` は更新した行を再 `parse` しないので、
/// ここで理由を名乗るには読み直しをもう一段増やすことになる）。
///
/// | `invalid_columns` | 次の一手 |
/// |---|---|
/// | `"handle"` を含む | **`DELETE`。** 識別子は不変（ADR-0016 §1）なので `PATCH` では永久に直らない |
/// | それ以外 | **もう一度 `PATCH`。** 残っている列を直せば読めるようになる |
///
/// **読み出しが失敗した場合はここに含まれない**（500 で返る）。
/// 「読み直せなかった」と「DB が壊れている」を同じ応答にしないこと。
/// 見張り番は `tests/server_admin.rs` の
/// `patching_a_ghost_row_reports_success_and_actually_writes_the_row` と
/// `a_row_whose_created_at_cannot_be_decoded_fails_the_update_loudly`。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct UpdatedCharacterView {
    /// 更新した行の主キー。**必ず入る**（`store` の UPDATE が引き当てた id）。
    ///
    /// 下の `character` が `null` のとき、この行を指す手掛かりはこれだけになる
    /// （`GET /admin/characters` の `unreadable` に出る id と同じ値）。
    id: i64,
    /// 更新後の姿。**`null` は失敗ではない**（読み出せなかっただけ。上記）。
    ///
    /// 空でも**キーごと落とさない**（`unreadable` と同じ理由）。
    #[schema(required = true)]
    character: Option<CharacterView>,
}

/// キャラクター一覧の応答。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct CharacterListView {
    characters: Vec<CharacterView>,
    /// **読み出せなかった行**（ADR-0016 §3 / 作業単位 0012b）。
    ///
    /// 空でも**キーごと落とさない**（`characters` と同じ理由。壊れた行が無いことと、
    /// この応答が壊れた行を見せない古い版であることを、受け取った側が区別できる）。
    unreadable: Vec<UnreadableCharacterView>,
}

/// 読み出せなかった行1件の応答。**修復に要る手掛かりだけ。**
///
/// **格納値そのものは載せない**（`store` が返していない。理由は
/// `domain::UnreadableCharacter` の doc）。中身を見たいときは `id` で DB を直接見る。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct UnreadableCharacterView {
    /// 主キー。**この行を指す唯一の確実な手掛かり。**
    id: i64,
    /// 修復に使える識別子。**`null` は「識別子そのものが壊れていて API からは指せない」**
    /// （名前は不変なので、その行は削除もできない。DB を直接直すしかない）。
    #[schema(required = true)]
    handle: Option<String>,
    /// 読めなかった列。**DB の列名と同じ綴り。**
    #[schema(schema_with = character_columns_schema)]
    invalid_columns: Vec<&'static str>,
}

impl From<UnreadableCharacter> for UnreadableCharacterView {
    fn from(broken: UnreadableCharacter) -> Self {
        Self {
            id: broken.id.get(),
            handle: broken.handle.map(|handle| handle.as_str().to_owned()),
            invalid_columns: broken
                .invalid_columns
                .into_iter()
                .map(CharacterColumn::as_str)
                .collect(),
        }
    }
}

/// 性格ステータスの応答。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct StatsView {
    /// どのキャラクターのものか（パスを読み違えていないことが応答から分かる）。
    character: String,
    /// そのキャラクターの尺度（ADR-0015）。**値の意味はこれが無いと決まらない。**
    ///
    /// ADR-0015 が「独立項目にする」理由の1つに挙げたのが
    /// **GUI で「この軸は 0〜10 の整数」と出せること**で、
    /// 性格ステータスを返すこのエンドポイントがそれを返せないと、
    /// 呼び出し側は**軸の一覧と尺度を別々に取りに行く**ことになる。
    ///
    /// **`value <= stat_scale` は保証していない**（作業単位 0011a-1 の範囲外）。
    /// 尺度は「そう宣言されている上端」であって、いま実データを縛ってはいない。
    stat_scale: u32,
    stats: Vec<StatAllocView>,
}

/// 1軸分の応答。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct StatAllocView {
    axis: String,
    value: u32,
}

/// **直近の票を何件返すか**（作業単位 D-4a）。
///
/// # なぜ上限が要るのか
///
/// **票は消えない**（追記のみ / ADR-0026 §決定3）。しかも**全軸採点では
/// 1ターンにつき軸の数だけ立つ。** 上限が無いと、応答は会話の長さに比例して
/// 伸び続け、**古いキャラクターほど画面が開かなくなる。**
///
/// # なぜ 50 なのか
///
/// **5軸なら直近 10 ターンぶん**にあたる。
/// 「さっき何が起きたか」を追うのに要るのはこの桁で、
/// **1画面をスクロールして読み切れる量**でもある。
///
/// **⚠ 実測由来の値ではない。**
/// この機で測った時間でもモデルの都合でもないので、
/// [ADR-0036](../../docs/adr/0036-the-range-of-machines-is-a-constraint.md) の
/// 「機に依存する定数」ではなく、**`MEASURED_*` の作法には乗せない。**
/// 外したときに起きるのは「一覧が長い／短い」だけで、**壊れない。**
///
/// # ページ送りは足していない（**足さないと決めた**）
///
/// 足すには**カーソルの意味**（同着をどう跨ぐか・途中で票が増えたらどうなるか）を
/// 決める必要があり、この入口の用途（**直近の変容を見る**）には要らない。
/// **総数を一緒に返してある**ので、画面は「全部で M 件のうち直近 N 件」と言える
/// ——**言えないまま切ると、画面は「これで全部」と嘘をつく。**
///
/// 全部を読みたくなったら、そのときに `?before=` の形で足せる
/// （**並びが決まっている**ので後から足せる / [`DriftView::recent_votes`] の doc）。
///
/// # **期待値をこの定数から作らないこと**
///
/// 作ると、**この定数を倍にする変異が緑を通る**（CLAUDE.md `16b` の実測）。
/// 権威は `tests/server_admin_drift.rs::
/// only_the_most_recent_votes_come_back_but_the_total_counts_them_all` で、
/// **この定数より多い票を入れて、外の値（直書き）で押さえている。**
const RECENT_DRIFT_VOTES: u32 = 50;

/// 変容の履歴の応答（ADR-0024 §9 / 作業単位 D-4a）。
///
/// # **`GET .../stats` の上位互換ではない**
///
/// 軸といまの値は同じものを返すが、この入口が答えるのは
/// 「**なぜその値になったのか**」である。画面はふたつを並べて出す前提で、
/// **軸の並びは `stats_view` と同じ**（保存順 / `Store::stat_allocs`）にしてある。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct DriftView {
    /// どのキャラクターのものか（パスを読み違えていないことが応答から分かる）。
    character: String,
    /// そのキャラクターの尺度（ADR-0015）。**値の意味はこれが無いと決まらない。**
    stat_scale: u32,
    /// 軸ごとの姿。**値 0 の軸も返す**（落とすのは `prompt` 層の責務 / `get_stats` と同じ）。
    ///
    /// # **割り当ての無い軸も返す**（作業単位 B-16）
    ///
    /// 累積か票の残っている軸は、`stat_alloc` に行が無くても並ぶ
    /// （`value` は `null` / [`DriftAxisView::value`] の doc）。
    /// **並びは [`drift_view`] が決める。**
    axes: Vec<DriftAxisView>,
    /// **保存されている票の総数。** `recent_votes` の件数**ではない。**
    ///
    /// 返した件数を総数と呼ぶと、**画面が「これで全部」と嘘をつく。**
    /// **軸として保存されていない綴りの票も数に入る**（票は消えない）。
    ///
    /// # **数えた票は、どれかの軸の行に現れる**（作業単位 B-16）
    ///
    /// `axes` の `ups + downs` を全部足すとこの数になる。
    /// **是正前は合わなかった**——割り当てを失った軸の票は、
    /// 総数に数えられるのに**どの行にも現れなかった。**
    /// 読み手には「数が合わない」としか見えず、
    /// **どこへ消えたのかを応答から知る手段が無かった。**
    ///
    /// 見張り番は `tests/server_admin_drift.rs::assert_every_vote_belongs_to_an_axis`
    /// を呼ぶ**4つのテスト関数**（呼び出しは5箇所。B-19 で数え直した）。
    /// **辻褄は応答の別々の欄どうしで突き合わせる。**
    ///
    /// ## **残っていた条件は上流で潰した**（作業単位 B-19 / 2026-08-28）
    ///
    /// B-16 の時点では条件が1つ残っていた——
    /// **「`stat_alloc` の軸が、正規化した後も一意であること」。**
    /// `stat_alloc` の主キーは**生のテキスト**なので `パッション` と ` パッション` は同居でき、
    /// **`SELECT_STAT_ALLOCS` は両方を返して `parse` が両方とも同じ軸名に正規化していた。**
    /// **同じ軸の行が2つ並び、どちらも同じ集計を引き当てて総和が総数を超える**
    /// （レビューの実測: `ups 2 / downs 1` の行が2つ、`total_votes` は 3、総和は 6）。
    ///
    /// **B-19 で `Store::stat_allocs` がその状態を `InvalidStoredValue` にした**ので、
    /// **重複したまま 200 が返ることは無い**（`GET .../stats` も**同じ1箇所**で止まる。
    /// 2つの入口が食い違わない理由と、却下した案はあちらの doc）。
    ///
    /// **⚠ 「1行でも読めなければ 500」に統合された。** B-16 が
    /// 「この経路には効かない」と書いていたのは、
    /// **読めない行ではなく読めてしまう重複行**だったからで、いまは読めない側に入っている。
    ///
    /// **等式が寄りかかっている不変条件は3つ**——どれも上流で強制されている:
    ///
    /// 1. **軸の行が正規化の上でも一意**（`Store::stat_allocs`）。
    /// 2. **集計は正規化した軸ごとに畳まれる**（`store::drift` の `fold_tallies`）。
    /// 3. **割り当ての無い軸は、重複を潰してから並ぶ**（[`unallocated_axes`]。
    ///    割り当てのある軸と重ならないことも同じ関数が見ている）。
    ///
    /// **未実測**: FE の `{#each drift.axes as axis (axis.axis)}` に鍵の重複を
    /// 通したときの挙動。**この応答からは作れなくなった**（重複は 500）ので、
    /// **測る対象が無い。**
    total_votes: u32,
    /// `recent_votes` に載る上限（[`RECENT_DRIFT_VOTES`]）。
    ///
    /// **`total_votes` との差が「見えていない票」である。**
    /// 件数だけでは「上限で切られたのか、たまたまその数なのか」が読めない。
    recent_limit: u32,
    /// **直近の票そのもの。新しい順**（`created_at` の降順、同着は `id` の降順）。
    ///
    /// 並びを決めているのは `store` の1文（`Store::drift_vote_history`）で、
    /// **ここで並べ替えない。** 同着は**本番で普通に起きる**
    /// （1ターンで複数の軸が同時に書かれる）ので、時刻だけでは順序が決まらない。
    recent_votes: Vec<DriftVoteView>,
    /// **このキャラクターにいま効いている変容の走らせ方**
    /// （`off` / `shadow` / `on` / ADR-0026 §決定1 / 作業単位 D-5a / **A-8 で意味を変えた**）。
    ///
    /// # ⚠ A-8 から「サーバの」ではなく「このキャラクターの」値（ADR-0045 §決定4）
    ///
    /// **キャラクターの値があればそれ、無ければ起動時の既定。** 出どころは隣の
    /// [`Self::run_mode_source`]、起動時の既定そのものは [`Self::default_run_mode`]。
    /// 決めるのは `ResolvedDriftMode::resolve` の1箇所で、**変容を実際に走らせる側
    /// （`server::extract::Extraction::start`）と同じ関数**を通る。
    ///
    /// # **これが無いと「壊れている」と「off である」が区別できない**
    ///
    /// **`PERSONACORE_STAT_DRIFT` の既定は `off`** で、`off` は
    /// **上流を呼ばず、判定もせず、票も累積も1行も作らない**（`StatDriftMode::runs`）。
    /// **つまり既定の配置で画面を開くと `total_votes: 0` / 全軸 `charge: 0.0` /
    /// `recent_votes: []` が返る**——**変容が壊れている状態と、応答が1バイトも違わない。**
    ///
    /// 走らせ方は起動ログと `server::drift` の記録の欄にしか出ていなかった。
    /// **ログは流れるし、そもそも画面を見ている人は読めない。**
    ///
    /// > **⚠ その欄の綴りをここに書かないこと。** `server::drift` の走査
    /// > （`the_drift_fields_never_collide_with_another_module`）が
    /// > `src/**` から欄名を探して落とす（着手時に実際に踏んだ）。
    ///
    /// # 綴りの正は [`StatDriftMode::label`]
    ///
    /// **写しを作らない。** ログの欄・環境変数の綴り・この応答が同じ綴りなので、
    /// **画面に出た語をそのまま `PERSONACORE_STAT_DRIFT` へ書き戻せる。**
    ///
    /// # 値の出どころは、変容を実際に止めている値そのもの
    ///
    /// 起動時の既定は [`AppState::default_stat_drift`] が**抽出器が持っている値**を返す
    /// （`server::mod` の doc）。**別の写しを持たない**ので、
    /// 「応答は `on` と言うのに走らない」形が作れない。
    #[schema(schema_with = drift_mode_schema)]
    run_mode: &'static str,
    /// **[`Self::run_mode`] がどこから来たか**（`character` / `default` / ADR-0045 §決定4）。
    ///
    /// 綴りの正は `DriftModeSource::label`。
    #[schema(schema_with = run_mode_source_schema)]
    run_mode_source: &'static str,
    /// **起動時の既定**（`PERSONACORE_STAT_DRIFT`。無ければ `off` / ADR-0045 §決定4）。
    ///
    /// **キャラクターが値を持っていても載せる**——画面が「既定に従う（いまは停止）」と
    /// 言うための材料で、未設定へ戻したときに何が効くかを先に言える。
    #[schema(schema_with = drift_mode_schema)]
    default_run_mode: &'static str,
}

/// 1軸分の変容の姿。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct DriftAxisView {
    axis: String,
    /// いまの値（`GET .../stats` が返すものと同じ）。
    ///
    /// # **割り当てが無ければ `null`**（作業単位 B-16）
    ///
    /// 累積か票だけが残っている軸（`PUT .../stats` の全置換で綴りが変わった軸）は、
    /// `stat_alloc` に行が無いので**いまの値を持たない。**
    ///
    /// **`0` で埋めないこと。** `0` は**人が書いた宣言**
    /// （「この軸は語らない」/ ADR-0024 §4）であり、**変容の対象外**という意味を持つ。
    /// 埋めると画面には同じ行として出て、
    /// **「まだ動かない軸」と「もう無い軸」が区別できなくなる。**
    ///
    /// **欄ごと落とす形も採らない。** 読み手が `?? 0` で埋めた瞬間に静かに 0 と混ざる。
    /// `null` なら**型が変わるので、数として読む経路は必ず壊れる。**
    ///
    /// 権威は `tests/server_admin_drift.rs::
    /// a_zero_valued_axis_is_never_confused_with_an_axis_that_has_no_allocation`
    /// （**同じ応答の中に両方を置いて**見る。別々のテストに分けると、
    /// 両方を `0` にする実装が片方ずつ緑になる）。
    #[schema(required = true)]
    value: Option<u32>,
    /// **溜まっている累積**（ADR-0027 §決定1〜3）。
    ///
    /// **行が無い軸は 0。** 累積の値だけでは
    /// 「一度も採点されていない」と「0 に戻った」が区別できない。
    ///
    /// > **⚠ 区別は [`DriftAxisView::charge_updated_at`] が付ける**（作業単位 D-5a）。
    /// > **`null` なら行が無い**（一度も書かれていない）、
    /// > **時刻があれば 0 に戻った**である。**この欄だけを見て区別しないこと。**
    charge: f64,
    /// **その軸の累積が最後に書き戻された時刻**（作業単位 D-5a）。
    ///
    /// # **累積の行が無い軸は `null`**
    ///
    /// **`0` でも現在時刻でも埋めないこと**（[`DriftAxisView::value`] と同じ立場）。
    /// 現在時刻で埋めると**「たったいま動いた」と読める**——
    /// この入口の存在理由が「いつ動いたのかを表から読む」ことなので、
    /// **埋めた瞬間にこの欄は嘘しか言わなくなる。**
    ///
    /// # なぜ落とせないのか
    ///
    /// 累積は**履歴を持たない**（履歴は票とログの側）ので、
    /// **表から取れる時間の手掛かりはこの1列だけ**である
    /// （`store::drift` の `UPSERT_CHARGE` の doc が自分でそう書いている）。
    /// **書き込みは毎回この列を進めているのに、読む口がどこにも無かった。**
    ///
    /// # 票の `created_at` とは別の時刻である
    ///
    /// 票は**判定のたび**に立つ（記録専用モードでも立つ）。
    /// 累積が動くのは**値を動かしてよいモードのときだけ**（`DriftApply` が要る）。
    /// **`shadow` で回している間は、票の時刻だけが進んで、この欄は止まったまま**になる。
    ///
    /// # ⚠ **軸ごとにばらけるとは限らない。何を答える欄なのかを取り違えないこと**
    ///
    /// `server::drift::apply_scores` は**動いた軸だけでなく全軸ぶん**を書き戻す
    /// （薄れた端数を持ち越すため）。したがって **`on` で回っている間、
    /// いま割り当てのある軸の時刻は毎ターン一斉に同じ値へ進む。**
    ///
    /// **この欄が答えるのは「その軸がいつ動いたか」ではなく、次の2つである:**
    ///
    /// | 読めること | 何から |
    /// |---|---|
    /// | **このキャラクターの変容が最後に走ったのはいつか** | 割り当てのある軸の時刻 |
    /// | **置き去りの軸が取り残されたのはいつごろか** | 改名で割り当てを失った軸の、止まった時刻 |
    ///
    /// **`applied` と `saturated` は、この欄からも区別できない**——
    /// 飽和した回も全軸ぶんの書き戻しは走るので、**時刻は同じように進む。**
    /// その区別は**いまも `server::drift` が出す結末の欄にしか無い**
    /// （**その欄名をここに綴らないこと。** あちらの走査
    /// `the_drift_fields_never_collide_with_another_module` が拾って落とす
    /// ——この段を書いたときに実際に踏んだ）。
    #[schema(required = true)]
    charge_updated_at: Option<DateTime<Utc>>,
    /// この軸に立った**上げ**の票の数（**上限で切らない**）。
    ups: u32,
    /// この軸に立った**下げ**の票の数。
    ///
    /// **`ups` との差ではなく両方を返す。** 畳むと「1回も動いていない」と
    /// 「押し合った」が同じ値になり、ADR-0024 の「危険 2. 一方向へ流れる」を
    /// 見るための比が作れない。
    downs: u32,
}

/// 票1件の応答。
///
/// **`id` を載せない。** 1件を指すパスを生やさない以上、貼る先が無い
/// （`MemoryView::id` が訂正のパスに貼る値なのと対照的）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct DriftVoteView {
    axis: String,
    /// 向き。**HTTP 上の綴りは `delta_to_wire` が決める。**
    #[schema(schema_with = delta_schema)]
    delta: &'static str,
    /// **どの往復で立った票か。** 落とすと、画面はどの会話が動かしたのかを言えない。
    session_id: i64,
    created_at: DateTime<Utc>,
}

/// **会話の一覧を何件返すか**（作業単位 D-12c-1）。
///
/// # なぜ上限が要るのか
///
/// **セッションは消えない**うえに、`@new` のたび・上流が開けなかった回のたびに1行増える
/// （`store::session` の `SELECT_LATEST_SESSION` の doc —— **掃除の経路が無い**）。
/// 全部返すと、応答は遊んだ回数に比例して伸び続ける。
///
/// # なぜ 100 なのか
///
/// 一覧は**選ぶための**もので、画面は新しい側から読む。100 は「最近の会話をだいたい全部見る」桁で、
/// 超えた分は `total_sessions` との差として読める。**ページ送りは足していない**
/// （足すならカーソルの意味——並びの第1キーが「中身があるか」なので、`updated_at` だけでは
/// 位置が決まらない——を先に決める必要がある）。
///
/// **⚠ 実測由来の値ではない**（外したときに起きるのは「一覧が長い／短い」だけで、壊れない）。
///
/// # **期待値をこの定数から作らないこと**
///
/// 権威は `tests/server_admin_sessions.rs::
/// only_the_newest_sessions_come_back_but_the_total_counts_them_all`
/// （**この定数より多い会話を仕込んで、外の値（直書きの 100 と 103）**で押さえている）。
const SESSION_LIST_LIMIT: u32 = 100;

/// **最初の利用者の発言を何文字で切るか**（一覧の「選ぶための手がかり」/ 作業単位 D-12c-1）。
///
/// 手がかりであって本文ではないので、1行に収まる程度でよい。**切ったことは応答が言う**
/// （[`SessionSummaryView::first_user_message_truncated`]）。
/// 権威は `tests/server_admin_sessions.rs::each_row_describes_its_own_session`
/// （81 文字の発言が 80 文字で返る）と `an_opening_of_exactly_the_limit_is_not_truncated`。
const SESSION_OPENING_CHARS: u32 = 80;

/// **1ページの既定の件数**（`?limit=` を省いたとき / 作業単位 D-12c-1）。
///
/// 画面はこれを使う（**画面に件数を写さない**——写すと、この値と画面の値の2つが黙って食い違う）。
/// 権威は `tests/server_admin_sessions.rs::the_default_page_and_the_largest_page_are_bounded`
/// （206 件を積んで、直書きの 50 で押さえる）。
const MESSAGE_PAGE_DEFAULT: u32 = 50;

/// **1ページの上限**（`?limit=` で頼める最大 / 作業単位 D-12c-1）。
///
/// 上限が無いと、`?limit=` 1つで1つの会話を丸ごと返させられる（会話にも上限は無い）。
/// **超えたら 400**（黙って切り詰めない——頼んだ数と返った数が食い違うと、
/// 呼び出し側は「これで全部」と読む）。権威は上と同じテスト（直書きの 200 と 201）。
const MESSAGE_PAGE_MAX: u32 = 200;

/// 会話のページの問い合わせ（`GET .../sessions/{session_id}/messages` / 作業単位 D-12c-1）。
///
/// # `deny_unknown_fields`（**知らない欄は 400**）
///
/// `MemoryListQuery` と同じ方針。`?after=` のような綴り違いを黙って捨てると、
/// **送った側は「最新のページ」を「頼んだページ」と読む。**
///
/// # 数は符号付きで受ける
///
/// **負の値も「読めた」うえで範囲外として断る**（`message_page_out_of_range`）。
/// 符号無しで受けると `-5` は「形式が不正」に落ち、直す相手（綴りか値か）が応答から読めない。
/// **数として読めないもの・`i64` に収まらないものは `invalid_query`**（`serde` が弾く）。
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub(super) struct MessagePageQuery {
    /// **この id より古いメッセージだけ**（カーソル）。省略なら最新から。
    #[serde(default)]
    before: Option<i64>,
    /// 何件返すか。省略なら [`MESSAGE_PAGE_DEFAULT`]、最大 [`MESSAGE_PAGE_MAX`]。
    #[serde(default)]
    limit: Option<i64>,
}

/// 会話の一覧の応答（`GET /admin/characters/{handle}/sessions` / 作業単位 D-12c-1）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct SessionListView {
    /// どのキャラクターのものか（パスを読み違えていないことが応答から分かる）。
    character: String,
    /// **素の `aoi` が続ける順**（先頭が `Store::latest_session`）。**ここで並べ替えない。**
    ///
    /// **空のセッションも出す**（件数 0。後ろへ回る）。`aoi@<id>` はそこへも届くので、
    /// 見えないことにすると「一覧に無いのに続けられる会話」ができる。
    sessions: Vec<SessionSummaryView>,
    /// **保存されているセッションの総数。** `sessions` の件数**ではない**
    /// （返した件数を総数と呼ぶと、画面が「これで全部」と嘘をつく）。
    total_sessions: u32,
    /// [`SESSION_LIST_LIMIT`]。**`total_sessions` との差が「見えていない件数」。**
    sessions_limit: u32,
}

/// 会話1件の姿（選ぶための手がかり）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct SessionSummaryView {
    /// 主キー。**`aoi@<id>` の `<id>` そのもの**（画面はこれを送り先に使う / ADR-0004）。
    id: i64,
    /// 人間向けの識別名。未設定なら `null`（いまは付ける経路が無い）。
    #[schema(required = true)]
    label: Option<String>,
    created_at: DateTime<Utc>,
    /// **並びの権威**（中身のあるものの中で、新しい順）。
    updated_at: DateTime<Utc>,
    /// メッセージの件数（利用者と応答の両方）。
    message_count: u32,
    /// **最初の利用者の発言の先頭**（[`SESSION_OPENING_CHARS`] 文字まで）。無ければ `null`。
    #[schema(required = true)]
    first_user_message: Option<String>,
    /// **切ったかどうか。** 文字列の長さからは「ちょうどその長さだった」と区別できない。
    first_user_message_truncated: bool,
}

impl From<SessionSummary> for SessionSummaryView {
    fn from(summary: SessionSummary) -> Self {
        let (first_user_message, first_user_message_truncated) = match summary.first_user_message {
            Some(opening) => (Some(opening.text), opening.truncated),
            None => (None, false),
        };
        Self {
            id: summary.session.id.get(),
            label: summary.session.label,
            created_at: summary.session.created_at,
            updated_at: summary.session.updated_at,
            message_count: summary.message_count,
            first_user_message,
            first_user_message_truncated,
        }
    }
}

/// 会話の1ページの応答（`GET .../sessions/{session_id}/messages` / 作業単位 D-12c-1）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct MessagePageView {
    /// どのキャラクターのものか。
    character: String,
    /// どのセッションのものか（パスを読み違えていないことが応答から分かる）。
    session_id: i64,
    /// **古い順。** 次に遡るときは、先頭の `id` を `?before=` に渡す。
    messages: Vec<MessageView>,
    /// **このページより古いメッセージが残っているか。**
    ///
    /// **総数との差から決めない**（読んでいる間に増えた分で狂う）。`store` が1件多く頼んで決める。
    has_older: bool,
    /// **そのセッションのメッセージの総数**（上限で切らない）。
    total_messages: u32,
    /// **このページに適用した上限**（頼んだ値か、既定）。
    limit: u32,
}

/// メッセージ1件の応答。
///
/// **`raw_content` を必ず載せる**（絶対制約 —— 分解に失敗した回の唯一の調査手段）。
/// 台詞と地の文は**保存された列そのもの**で、ここで整え直さない
/// （画面が見せる本文は、生出力を実行中の返事と同じ関数に通して作る / 作業単位 D-12c-2）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct MessageView {
    id: i64,
    /// `user` / `assistant`（**綴りは `role_to_wire`**）。
    #[schema(schema_with = role_schema)]
    role: &'static str,
    #[schema(required = true)]
    dialogue: Option<String>,
    #[schema(required = true)]
    narration: Option<String>,
    raw_content: String,
    created_at: DateTime<Utc>,
}

impl From<Message> for MessageView {
    fn from(message: Message) -> Self {
        Self {
            id: message.id.get(),
            role: role_to_wire(message.role),
            dialogue: message.dialogue,
            narration: message.narration,
            raw_content: message.raw_content,
            created_at: message.created_at,
        }
    }
}

/// `POST /admin/characters` — キャラクターの作成。
#[utoipa::path(
    post,
    path = "/admin/characters",
    summary = "Create a character",
    tag = "characters",
    request_body = CreateCharacterRequest,
    responses(
        (status = 201, body = CharacterView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn create_character(
    State(state): State<AppState>,
    body: Result<Json<CreateCharacterRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<CharacterView>), ApiError> {
    // **本文の拒否は `ApiError::from_json_rejection` に一本化してある。**
    // 入力を反射しないこと・413 を 400 に潰さないことは、そちらの doc を参照。
    let Json(request) =
        body.map_err(|rejection| ApiError::from_json_rejection(rejection, ENDPOINT))?;
    // **`&str` のまま `store` へ渡さない**（ADR-0004）。
    let handle = Handle::parse(&request.handle)?;
    // 表示名も同じ。**省略時は `handle` で埋める**（フィールドの doc に理由がある）。
    // 検証前の `request.handle` ではなく**検証後の `handle`** を使うこと。
    // **2つの `parse` の正規化が一致し続けることに依存しない**ため。
    // 現時点ではどちらも先頭で `text::trim_blank` を呼ぶので結果は同じだが、
    // 片方の正規化だけが変わった瞬間に、`" 葵 "` で作ったキャラクターの表示名だけが
    // 識別子とずれる。省略時の既定値は「識別子と同じ値」であるべきなので、
    // **識別子として確定した値**をそのまま渡す。
    let display_name = display_name_of(&handle, request.display_name.as_deref())?;
    // ペルソナも同じ。空・長すぎるものはここで止める（`PATCH` で直せるようになった
    // 今も、**気付くのは「そのキャラクターと話せなくなってから」**。
    // 重さの権威は `domain::character::Persona` の doc の 0012a の注記）。
    let base_persona = Persona::parse(&request.base_persona)?;
    // 形式指示も同じ。**削れない枠に載るうえ、「指示として従え」と宣言される唯一の節**
    // なので、長さの検証を通っていない値を `store` へ渡さない。
    let output_format = request
        .output_format_instruction
        .as_deref()
        .map(OutputFormat::parse)
        .transpose()?;
    // 尺度も同じ。**省略時の既定を決めるのはここだけ**（`store` は必ず値を要求する）。
    let stat_scale = stat_scale_of(request.stat_scale)?;
    // 走らせ方（ADR-0045 §決定3）。**省略は未設定のまま**（既定で埋めない）。
    // **別名の綴りはここで 400**（DB に届く前に止める）。
    let stat_drift_mode = request
        .stat_drift_mode
        .as_deref()
        .map(StatDriftMode::parse)
        .transpose()?;

    state
        .store()
        .create_character_with_drift_mode(
            &handle,
            &display_name,
            &base_persona,
            output_format.as_ref(),
            stat_scale,
            stat_drift_mode,
        )
        .await
        .map_err(|err| classify_create(&err))?;

    // 作成した行を読み直して返す（`created_at` は DB が書くため）。
    let character = find_character(&state, &handle).await?;
    tracing::info!(character = %handle, "キャラクターを作成しました");

    Ok((StatusCode::CREATED, Json(CharacterView::from(character))))
}

/// `GET /admin/characters` — キャラクターの一覧。
///
/// **1件も無ければ空の一覧**でエラーにしない。
///
/// # **読み出せない行は `unreadable` に出す**（ADR-0016 §3 / 作業単位 0012b）
///
/// **ここが修復の入口。** 更新 API は名前で引くので、
/// 「どの行が壊れているか」をこの応答で知れないと直しに行けない。
/// `characters` にだけ詰めて捨てると、`Store::list_characters` が指し示した意味が
/// ここで消える（黙って読み飛ばすのと同じ）。
#[utoipa::path(
    get,
    path = "/admin/characters",
    summary = "List characters",
    tag = "characters",
    responses(
        (status = 200, body = CharacterListView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn list_characters(
    State(state): State<AppState>,
) -> Result<Json<CharacterListView>, ApiError> {
    let entries = state
        .store()
        .list_characters()
        .await
        .map_err(|err| internal(&err, "キャラクター一覧を取得できませんでした"))?;

    let mut characters = Vec::with_capacity(entries.len());
    let mut unreadable = Vec::new();
    for entry in entries {
        match entry {
            CharacterEntry::Readable(character) => characters.push(CharacterView::from(character)),
            CharacterEntry::Unreadable(broken) => {
                unreadable.push(UnreadableCharacterView::from(broken));
            }
        }
    }

    Ok(Json(CharacterListView {
        characters,
        unreadable,
    }))
}

/// `PATCH /admin/characters/{handle}` — 表示名・ペルソナ・形式指示の**部分更新**。
///
/// **しまう／戻すもここ**（`archived` / ADR-0049 §決定1）。**口を分けない**——
/// しまうのは列を1つ書く更新で、別の口にすると同じ行の更新に入口が2つになる。
///
/// 識別子は変えられない（ADR-0016 §1。要求型に口が無い）。
/// 指定されなかった列は**触らない**（既定値で潰さない）。
///
/// # 応答は「更新した行の id」と「読み直せた姿」を分ける
///
/// **読み直せなくても失敗ではない**（モジュール doc の「引き当ての非対称」）。
/// `UpdatedCharacterView` の doc に2つの正しい `null` のケースがある。
#[utoipa::path(
    patch,
    path = "/admin/characters/{handle}",
    summary = "Update some fields of a character",
    tag = "characters",
    params(("handle" = String, Path)),
    request_body = UpdateCharacterRequest,
    responses(
        (status = 200, body = UpdatedCharacterView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn update_character(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    // **`Json` は最後に置く**（本文を消費する抽出器は1つだけ、かつ末尾）。
    body: Result<Json<UpdateCharacterRequest>, JsonRejection>,
) -> Result<Json<UpdatedCharacterView>, ApiError> {
    let handle = path_handle(path)?;
    // **本文の拒否は `ApiError::from_json_rejection` に一本化してある。**
    let Json(request) =
        body.map_err(|rejection| ApiError::from_json_rejection(rejection, ENDPOINT))?;
    // **検証を先に済ませる。** 1つでも不正なら DB に触れない。
    let update = character_update(request)?;

    let id = state
        .store()
        .update_character(&handle, &update)
        .await
        .map_err(|err| classify_update(&err))?;

    let character = updated_character(&state, &handle).await?;
    tracing::info!(
        character = %handle,
        character_id = id.get(),
        readable = character.is_some(),
        "キャラクターを更新しました"
    );

    Ok(Json(UpdatedCharacterView {
        id: id.get(),
        character: character.map(CharacterView::from),
    }))
}

/// `DELETE /admin/characters/{handle}` — キャラクターの削除。
///
/// **しまっておらず、会話が1件でもあれば拒否する**（ADR-0016 §2 / ADR-0049 §決定1）。
/// **しまってあれば、会話・記憶・性格値なども全部消す**（カスケード。控えは残さない）。
/// 判定は `store` が削除と同じ文で不可分に行うので、**ここで会話の有無もしまったかどうかも
/// 確かめ直さない**（確かめてから消すと、その間に戻された・会話が始まったキャラクターを消しうる）。
///
/// 成功は **204**（返す状態がもう無い）。
#[utoipa::path(
    delete,
    path = "/admin/characters/{handle}",
    summary = "Delete a character",
    tag = "characters",
    params(("handle" = String, Path)),
    responses(
        (status = 204),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn delete_character(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<StatusCode, ApiError> {
    let handle = path_handle(path)?;

    state
        .store()
        .delete_character(&handle)
        .await
        .map_err(|err| classify_delete(&err))?;

    tracing::info!(character = %handle, "キャラクターを削除しました");
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /admin/characters/{handle}/stats` — 性格ステータスの取得。
///
/// **値 0 の軸も返す。** 落とすのは `prompt` 層の責務。
///
/// # 1行でも読めなければ 500（`GET .../drift` と同じ）
///
/// **正規化すると同じ軸になる行が2つある場合も「読めない」に入る**（作業単位 B-19）。
/// **判定は `Store::stat_allocs` の1箇所**で、こちらには写していない
/// ——写すと、**片方の入口だけが落ちても緑のまま**になる（CLAUDE.md `D-6`）。
/// 理由・却下した案・直し方はあちらの doc。
#[utoipa::path(
    get,
    path = "/admin/characters/{handle}/stats",
    summary = "Get the personality stats of a character",
    tag = "stats",
    params(("handle" = String, Path)),
    responses(
        (status = 200, body = StatsView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn get_stats(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<StatsView>, ApiError> {
    let handle = path_handle(path)?;
    let character = find_character(&state, &handle).await?;

    let stats = state
        .store()
        .stat_allocs(character.id)
        .await
        .map_err(|err| internal(&err, "性格ステータスを取得できませんでした"))?;

    Ok(Json(stats_view(&handle, character.stat_scale, stats)))
}

/// `PUT /admin/characters/{handle}/stats` — 性格ステータスの**丸ごと置き換え**。
///
/// 要求に含まれない軸は消える（`store` の `replace_stat_allocs` の意味論そのまま）。
/// 空の配列を渡すと全軸が消える。
#[utoipa::path(
    put,
    path = "/admin/characters/{handle}/stats",
    summary = "Replace all personality stats of a character",
    tag = "stats",
    params(("handle" = String, Path)),
    request_body = ReplaceStatsRequest,
    responses(
        (status = 200, body = StatsView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn replace_stats(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    // **`Json` は最後に置く**（本文を消費する抽出器は1つだけ、かつ末尾）。
    body: Result<Json<ReplaceStatsRequest>, JsonRejection>,
) -> Result<Json<StatsView>, ApiError> {
    let handle = path_handle(path)?;
    // **本文の拒否は `ApiError::from_json_rejection` に一本化してある。**
    // 入力を反射しないこと・413 を 400 に潰さないことは、そちらの doc を参照。
    let Json(request) =
        body.map_err(|rejection| ApiError::from_json_rejection(rejection, ENDPOINT))?;

    // **キャラクターを先に引く。** 値の検証に**そのキャラクターの尺度**が要る
    // （ADR-0015 / 作業単位 0011a-2）。
    //
    // **`StatScale::DEFAULT` を使わないこと。** 既定の写しが増えるうえ、
    // 尺度を明示したキャラクターを**他人の尺度で検証する**ことになる。
    let character = find_character(&state, &handle).await?;

    // **軸名と値の検証を、DB に触れる前に済ませる。** 1つでも不正なら書き込まない
    // （置き換えは全消し→挿入なので、途中で弾くと空になった状態が観測されうる。
    //   実際にはトランザクションが守るが、無駄な書き込みも避ける）。
    let allocs = request
        .stats
        .into_iter()
        .map(|stat| {
            Ok(StatAlloc {
                axis: StatAxis::parse(&stat.axis)?,
                value: character.stat_scale.value(stat.value)?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    state
        .store()
        .replace_stat_allocs(character.id, &allocs)
        .await
        .map_err(|err| classify_replace_stats(&err))?;

    // 読み直して返す（応答が DB の状態と一致していることを保証する）。
    let stored = state
        .store()
        .stat_allocs(character.id)
        .await
        .map_err(|err| internal(&err, "性格ステータスを取得できませんでした"))?;
    tracing::info!(character = %handle, axes = stored.len(), "性格ステータスを設定しました");

    Ok(Json(stats_view(&handle, character.stat_scale, stored)))
}

/// `GET /admin/characters/{handle}/drift` — **変容の履歴**
/// （[ADR-0024](../../docs/adr/0024-stat-drift.md) §9 / 作業単位 D-4a）。
///
/// # なぜ `GET .../stats` では足りないのか
///
/// **変容は本文にもユーザーにも見せない**（ADR-0024 §9）ので、
/// 起きたことが見えるのは**ログ**だけだった。ログは流れる。
/// そして `GET .../stats` が返すのは「いまの値」なので、
/// **変容したのか、人が `PUT .../stats` で設定したのかを区別できない。**
///
/// # 読むだけ。書き込みの口は無い
///
/// 票は追記のみ（ADR-0026 §決定3）で、**書くのは本文処理だけ**である。
/// 外から書き換えられる記録は記録ではない。
/// **1件を指すパス（`.../drift/{id}`）も生やさない**——生やせば
/// `PUT`（置き換え）と `DELETE`（削除）が自然な形になる
/// （`.../memories/{id}` を生やさないのと同じ理由 / モジュール doc の表）。
///
/// # **軸の一覧は `stat_alloc` だけでは決まらない**（作業単位 B-16）
///
/// `PUT .../stats` で軸を改名すると、古い綴りの累積と票が置き去りになる。
/// **落とさずに、`value` を `null` にして並べる**（`drift_view` の doc に理由と並び順）。
/// **読むだけ。置き去りの行を消す経路は足していない。**
///
/// # 読みが5つに分かれている
///
/// 軸・累積・集計・総数・直近の一覧を**別々の文**で引く。
/// **1つのトランザクションで囲っていない**ので、変容が同時に走っていると
/// 総数と一覧が1件ずれることがある。**画面の表示が1件ずれるだけ**で、
/// 囲うと**裏方の書き込みと読みロックを取り合う**（CLAUDE.md「トランザクションを
/// 長く保持すると `busy_timeout` を食い潰す」）——釣り合わない。
///
/// # 1行でも読めなければ 500
///
/// `GET .../stats` と同じ方針（`Store::stat_allocs` / `Store::drift_charges` の doc）。
/// **壊れた行を指し示す形（`unreadable`）にはしていない**——
/// 記憶の一覧（ADR-0022）がそうしているのは**そこが唯一の修復の入口**だからで、
/// 変容の記録には**アプリ経由の修復の入口がそもそも無い**（追記のみ）。
/// 指し示しても、読み手にできることが増えない。
///
/// **見張り番は `tests/server_admin_drift.rs` の3本**（レビューの実測で足した / 2026-08-27。
/// それまで**読み飛ばしに変える変異が 1602 全緑**だった）:
/// `a_broken_time_stops_this_listing_but_not_the_total_or_the_accumulation` /
/// `a_broken_axis_stops_this_listing_and_the_accumulation_but_never_the_total` /
/// `a_broken_row_outside_the_recent_window_still_stops_the_listing`。
/// **3本とも 500 の本文を `assert_no_internals` に通している**——
/// **内部情報が載りうるのはデコード失敗が絡むこちら側**で、404 だけを見ていると外に残る。
///
/// **読み飛ばしに変えないこと。** 変えると、**壊れた行の票は一覧から静かに消えるのに
/// `total_votes` には残る**（総数は軸も向きも読まない）。
/// 読み手には1件足りない一覧が出るだけで、**どの行が壊れているかは分からない。**
#[utoipa::path(
    get,
    path = "/admin/characters/{handle}/drift",
    summary = "Get the stat drift state and its recent votes",
    tag = "drift",
    params(("handle" = String, Path)),
    responses(
        (status = 200, body = DriftView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn get_drift(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<DriftView>, ApiError> {
    let handle = path_handle(path)?;
    let character = find_character(&state, &handle).await?;

    let stats = state
        .store()
        .stat_allocs(character.id)
        .await
        .map_err(|err| internal(&err, "性格ステータスを取得できませんでした"))?;
    let charges = state
        .store()
        .drift_charges(character.id)
        .await
        .map_err(|err| internal(&err, "変容の累積を取得できませんでした"))?;
    let tallies = state
        .store()
        .drift_vote_tallies(character.id)
        .await
        .map_err(|err| internal(&err, "変容の集計を取得できませんでした"))?;
    // **上限で切らない**（`recent_votes` の件数とは別の値 / `RECENT_DRIFT_VOTES` の doc）。
    let total_votes = state
        .store()
        .count_drift_votes(character.id)
        .await
        .map_err(|err| internal(&err, "変容の票を数えられませんでした"))?;
    let recent = state
        .store()
        .drift_vote_history(character.id, RECENT_DRIFT_VOTES)
        .await
        .map_err(|err| internal(&err, "変容の履歴を取得できませんでした"))?;

    Ok(Json(drift_view(
        &handle,
        character.stat_scale,
        stats,
        &charges,
        &tallies,
        total_votes,
        recent,
        // **キャラクターの値**（ADR-0045）。未設定なら既定に従う。
        character.stat_drift_mode,
        // **起動時の既定は DB に無い。設定である**（作業単位 D-5a）。
        // **読むのはここ1箇所**で、値は抽出器が実際に判定に使っているものそのもの。
        state.default_stat_drift(),
    )))
}

/// `GET /admin/characters/{handle}/attributes` — キャラクター属性の取得。
///
/// **1件も無ければ空の配列**でエラーにしない。
///
/// **1行でも読めなければ 500。** `Store::character_attributes` の方針
/// （1行壊れると丸ごと失敗する）をそのまま伝える。壊れた行の直し方は
/// `replace_attributes` の doc。
#[utoipa::path(
    get,
    path = "/admin/characters/{handle}/attributes",
    summary = "Get the attributes of a character",
    tag = "attributes",
    params(("handle" = String, Path)),
    responses(
        (status = 200, body = AttributesView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn get_attributes(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<AttributesView>, ApiError> {
    let handle = path_handle(path)?;
    let character = find_character(&state, &handle).await?;

    let attributes = state
        .store()
        .character_attributes(character.id)
        .await
        .map_err(|err| internal(&err, "キャラクター属性を取得できませんでした"))?;

    Ok(Json(attributes_view(&handle, attributes)))
}

/// `GET /admin/characters/{handle}/attributes/revisions` — **属性の変更の履歴**
/// （ADR-0028 §決定5 / 作業単位 A-3a）。**読むだけ。**
///
/// # なぜ要るのか
///
/// 属性は**削れない枠**に毎ターン載るので、静かに変わると
/// 「なんか前と違う」以上のことが分からない（§決定5）。
/// **変容には票の表がある。属性にもこれが要る。**
///
/// # **書き込みの口を足さない**
///
/// 履歴は追記のみで、書くのは `PUT .../attributes` ひとつ（§決定5「書き込み経路は1つ」）。
/// **1件を指すパス（`.../revisions/{id}`）も生やさない**——生やせば
/// `PUT`（置き換え）と `DELETE`（削除）が自然な形になり、
/// 記憶の側で塞いだのとまったく同じ穴が開く。
/// 見張り番は `tests/server_admin_attributes.rs::only_get_is_exposed_for_the_revisions_path`。
///
/// # 1行でも読めなければ 500
///
/// `Store::attribute_revisions` の方針をそのまま伝える。
/// **止まるのはこの一覧だけ**で、属性そのものの読みも置き換えも通る
/// （見張り番は `a_broken_history_row_is_a_five_hundred_without_internals`）。
#[utoipa::path(
    get,
    path = "/admin/characters/{handle}/attributes/revisions",
    summary = "List the revision history of the attributes",
    tag = "attributes",
    params(("handle" = String, Path)),
    responses(
        (status = 200, body = AttributeRevisionsView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn get_attribute_revisions(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<AttributeRevisionsView>, ApiError> {
    let handle = path_handle(path)?;
    let character = find_character(&state, &handle).await?;

    let revisions = state
        .store()
        .attribute_revisions(character.id, ATTRIBUTE_REVISIONS_LIMIT)
        .await
        .map_err(|err| internal(&err, "キャラクター属性の履歴を取得できませんでした"))?;
    // **上限で切らない**（返した件数とは別の値 / [`AttributeRevisionsView::total_revisions`]）。
    // **2つの読みを1つのトランザクションで囲わない**——`get_drift` と同じ判断で、
    // ずれるのは表示が1件ぶんだけ、囲うと裏方の書き込みと読みロックを取り合う。
    let total_revisions = state
        .store()
        .count_attribute_revisions(character.id)
        .await
        .map_err(|err| internal(&err, "キャラクター属性の履歴を数えられませんでした"))?;

    Ok(Json(attribute_revisions_view(
        &handle,
        revisions,
        total_revisions,
    )))
}

/// `GET /admin/characters/{handle}/agreements` — **取り決め（約束・決まり）の一覧**
/// （ADR-0051 §決定5 / 作業単位 A-12a）。有効なものも終わったものも、書いた順に返す。
///
/// **1行でも読めなければ 500**（`Store::agreements` の方針）。本文が読むのは有効な行だけなので、
/// 終わった行が壊れても本文は止まらない。
#[utoipa::path(
    get,
    path = "/admin/characters/{handle}/agreements",
    summary = "List the promises and rules of a character",
    tag = "agreements",
    params(("handle" = String, Path)),
    responses(
        (status = 200, body = AgreementsView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn list_agreements(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<AgreementsView>, ApiError> {
    let handle = path_handle(path)?;
    let character = find_character(&state, &handle).await?;

    let agreements = state
        .store()
        .agreements(character.id)
        .await
        .map_err(|err| internal(&err, "取り決めを取得できませんでした"))?;

    let fit = state.agreement_fit(&character);
    Ok(Json(agreements_view(&handle, agreements, fit)))
}

/// `POST /admin/characters/{handle}/agreements` — **取り決めを手で書く**（ADR-0051 §決定2 / 作業単位 A-12a）。
///
/// 書いたものは**有効で始まり、次のターンから本文に載る**（§決定4）。
///
/// # 上限に達していたら 409（**黙って古いものを落とさない**）
///
/// 有効な取り決めが `domain::MAX_ACTIVE_AGREEMENTS` 件あれば `agreement_limit_reached`。
/// **何も書かれない。** 古いものを終わらせれば書ける。
///
/// # 検証の順序
///
/// 本文の検証（中身・種類・守る側）を**キャラクターを引く前に**済ませる（`replace_attributes` と同じ
/// ——「不正な本文 × 未知のキャラクター」は 400 を優先する）。
#[utoipa::path(
    post,
    path = "/admin/characters/{handle}/agreements",
    summary = "Write a promise or rule by hand (it starts active)",
    tag = "agreements",
    params(("handle" = String, Path)),
    request_body = CreateAgreementRequest,
    responses(
        (status = 201, body = AgreementView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn create_agreement(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    // **`Json` は最後に置く**（本文を消費する抽出器は1つだけ、かつ末尾）。
    body: Result<Json<CreateAgreementRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<AgreementView>), ApiError> {
    let handle = path_handle(path)?;
    let Json(request) =
        body.map_err(|rejection| ApiError::from_json_rejection(rejection, ENDPOINT))?;
    let agreement = new_agreement(request)?;

    let character = find_character(&state, &handle).await?;

    let written = state
        .store()
        .add_agreement(character.id, &agreement)
        .await
        .map_err(|err| classify_agreement_write(&err))?;
    tracing::info!(
        character = %handle,
        agreement = written.id.get(),
        "取り決めを書きました"
    );

    Ok((StatusCode::CREATED, Json(agreement_view(written))))
}

/// `POST /admin/characters/{handle}/agreements/{agreement_id}/transitions` — **取り決めを手で終わらせる**
/// （ADR-0051 §決め直し 3 —— 終わらせるのは利用者だけ / 作業単位 A-12a）。
///
/// **`POST`＝履歴の行の追加**（投影の状態も同じトランザクションで書き換わる）。**行は消えない。**
/// 終わったものは次のターンから本文に載らない。
///
/// | 拒否 | 応答 |
/// |---|---|
/// | その id がこのキャラクターの取り決めでない | 404 `agreement_not_found` |
/// | もう終わっている | 409 `agreement_not_active` |
/// | 終わらせ方の綴りが違う（`active` を含む） | 400 `agreement_ending_unknown` |
#[utoipa::path(
    post,
    path = "/admin/characters/{handle}/agreements/{agreement_id}/transitions",
    summary = "End a promise or rule (fulfilled, broken or withdrawn)",
    tag = "agreements",
    params(("handle" = String, Path), ("agreement_id" = i64, Path)),
    request_body = EndAgreementRequest,
    responses(
        (status = 200, body = AgreementEndedView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn end_agreement(
    State(state): State<AppState>,
    path: Result<Path<(String, i64)>, PathRejection>,
    // **`Json` は最後に置く**（本文を消費する抽出器は1つだけ、かつ末尾）。
    body: Result<Json<EndAgreementRequest>, JsonRejection>,
) -> Result<Json<AgreementEndedView>, ApiError> {
    let (handle, agreement_id) = path_handle_and_agreement_id(path)?;
    let Json(request) =
        body.map_err(|rejection| ApiError::from_json_rejection(rejection, ENDPOINT))?;
    let ending = agreement_ending(&request.state)?;

    let character = find_character(&state, &handle).await?;

    let ended = state
        .store()
        .end_agreement(character.id, agreement_id, ending)
        .await
        .map_err(|err| classify_agreement_ending(&err))?;
    tracing::info!(
        character = %handle,
        agreement = ended.get(),
        state = ending.state().label(),
        "取り決めを終わらせました"
    );

    Ok(Json(AgreementEndedView {
        character: handle.as_str().to_owned(),
        agreement_id: ended.get(),
        state: ending.state().label(),
    }))
}

/// `POST /admin/characters/{handle}/agreements/{agreement_id}/adoption` — **提案を採る**
/// （提案中 → 有効 / ADR-0051 §決定2「採る・直してから採る」/ 作業単位 A-12c-1）。
///
/// 本文の欄（`content` / `kind` / `keeper`）は**どれも省ける**（省いた欄は提案のまま）。
/// 採ったものは**次のターンから本文に載る**。履歴に「提案中 → 有効」が残る。
///
/// | 拒否 | 応答 |
/// |---|---|
/// | その id がこのキャラクターの取り決めでない | 404 `agreement_not_found` |
/// | 提案中でない（もう採った・捨てた・手で書いた） | 409 `agreement_not_proposed` |
/// | 有効なものが上限に達している（**提案中は数えない**） | 409 `agreement_limit_reached` |
/// | 直した中身・種類・守る側の綴りが不正 | 400（`create_agreement` と同じ理由） |
///
/// **本文の検証をキャラクターを引く前に済ませる**（`create_agreement` と同じ順序）。
#[utoipa::path(
    post,
    path = "/admin/characters/{handle}/agreements/{agreement_id}/adoption",
    summary = "Adopt a proposed promise or rule (optionally edited first)",
    tag = "agreements",
    params(("handle" = String, Path), ("agreement_id" = i64, Path)),
    request_body = AdoptAgreementRequest,
    responses(
        (status = 200, body = AgreementView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn adopt_agreement(
    State(state): State<AppState>,
    path: Result<Path<(String, i64)>, PathRejection>,
    // **`Json` は最後に置く**（本文を消費する抽出器は1つだけ、かつ末尾）。
    body: Result<Json<AdoptAgreementRequest>, JsonRejection>,
) -> Result<Json<AgreementView>, ApiError> {
    let (handle, agreement_id) = path_handle_and_agreement_id(path)?;
    let Json(request) =
        body.map_err(|rejection| ApiError::from_json_rejection(rejection, ENDPOINT))?;
    let edits = agreement_edits(request)?;

    let character = find_character(&state, &handle).await?;

    let adopted = state
        .store()
        .adopt_agreement(character.id, agreement_id, &edits)
        .await
        .map_err(|err| classify_agreement_decision(&err))?;
    tracing::info!(
        character = %handle,
        agreement = adopted.id.get(),
        "取り決めの提案を採りました"
    );
    Ok(Json(agreement_view(adopted)))
}

/// `POST /admin/characters/{handle}/agreements/{agreement_id}/rejection` — **提案を捨てる**
/// （提案中 → 却下 / 作業単位 A-12c-1）。**行は消えない**（一覧と履歴に残り、次からの提案の
/// 依頼の「# 提案済み」に載る —— 同じものを二度出さないため）。本文は読まない。
///
/// | 拒否 | 応答 |
/// |---|---|
/// | その id がこのキャラクターの取り決めでない | 404 `agreement_not_found` |
/// | 提案中でない | 409 `agreement_not_proposed` |
#[utoipa::path(
    post,
    path = "/admin/characters/{handle}/agreements/{agreement_id}/rejection",
    summary = "Reject a proposed promise or rule (the row stays)",
    tag = "agreements",
    params(("handle" = String, Path), ("agreement_id" = i64, Path)),
    responses(
        (status = 200, body = AgreementRejectedView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn reject_agreement(
    State(state): State<AppState>,
    path: Result<Path<(String, i64)>, PathRejection>,
) -> Result<Json<AgreementRejectedView>, ApiError> {
    let (handle, agreement_id) = path_handle_and_agreement_id(path)?;
    let character = find_character(&state, &handle).await?;

    let rejected = state
        .store()
        .reject_agreement(character.id, agreement_id)
        .await
        .map_err(|err| classify_agreement_decision(&err))?;
    tracing::info!(
        character = %handle,
        agreement = rejected.get(),
        "取り決めの提案を捨てました"
    );
    Ok(Json(AgreementRejectedView {
        character: handle.as_str().to_owned(),
        agreement_id: rejected.get(),
        state: AgreementState::Rejected.label(),
    }))
}

/// `GET /admin/characters/{handle}/agreements/transitions` — **取り決めの履歴**（作業単位 A-12a）。**読むだけ。**
///
/// 書かれた・終わったの変化を新しい順に返す。**書く口は上の2本だけ**（履歴は追記のみ）。
/// **1件の履歴を指すパスは生やさない**（生やせば `PUT` と `DELETE` が自然な形になる）。
/// **総数と一覧を1つのトランザクションで囲わない**（`get_attribute_revisions` と同じ判断）。
#[utoipa::path(
    get,
    path = "/admin/characters/{handle}/agreements/transitions",
    summary = "List the history of the promises and rules",
    tag = "agreements",
    params(("handle" = String, Path)),
    responses(
        (status = 200, body = AgreementTransitionsView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn list_agreement_transitions(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<AgreementTransitionsView>, ApiError> {
    let handle = path_handle(path)?;
    let character = find_character(&state, &handle).await?;

    let transitions = state
        .store()
        .agreement_transitions(character.id, AGREEMENT_TRANSITIONS_LIMIT)
        .await
        .map_err(|err| internal(&err, "取り決めの履歴を取得できませんでした"))?;
    let total_transitions = state
        .store()
        .count_agreement_transitions(character.id)
        .await
        .map_err(|err| internal(&err, "取り決めの履歴を数えられませんでした"))?;

    Ok(Json(AgreementTransitionsView {
        character: handle.as_str().to_owned(),
        transitions: transitions
            .into_iter()
            .map(|transition| AgreementTransitionView {
                id: transition.id(),
                agreement_id: transition.agreement_id().get(),
                content: transition.content().as_str().to_owned(),
                kind: transition.kind().label(),
                keeper: transition.keeper().label(),
                before: transition.before().map(AgreementState::label),
                after: transition.after().label(),
                created_at: transition.created_at(),
            })
            .collect(),
        total_transitions,
        transitions_limit: AGREEMENT_TRANSITIONS_LIMIT,
    }))
}

/// `GET /admin/characters/{handle}/axis-candidates` — **軸の候補**
/// （[ADR-0046](../../docs/adr/0046-drift-proposes-new-axes.md) §決定6 / 作業単位 A-9a）。**読むだけ。**
///
/// 候補ごとに**いまの状態（移り変わりから導いたもの）・移り変わり・試用の票の内訳**を返す。
/// **他のキャラクターの候補は出ない**（`store` が持ち主で絞る）。
///
/// # **書き込みの口を足さない**
///
/// 候補を書くのは裏方（ADR-0046 §決定2〜4）と、第2段の採用・却下だけ。
/// **1件を指すパス（`.../axis-candidates/{id}`）も生やさない**——生やせば `PUT` と `DELETE` が
/// 自然な形になり、追記のみの記録が書き換えられる形になる（属性の履歴と同じ理由）。
/// 見張り番は `tests/server_admin_axis_candidates.rs::only_get_is_exposed_for_the_candidates_path`。
///
/// # 1件でも読めなければ 500
///
/// `Store::axis_candidates` の方針をそのまま伝える（黙って飛ばすと、どの候補がどこにいるのかが
/// 静かに1件消える）。**総数と一覧を1つのトランザクションで囲わない**（`get_attribute_revisions` と同じ判断）。
///
/// **実験中の口**（ADR-0047 §② P-2b —— ADR-0046 の第2段の前）。
/// 拡張の綴りは `openapi::STABILITY_EXTENSION` と同じ（`utoipa` は綴りに定数を受けない）。
#[utoipa::path(
    get,
    path = "/admin/characters/{handle}/axis-candidates",
    summary = "List proposed new stat axes (experimental)",
    tag = "axis-candidates",
    params(("handle" = String, Path)),
    responses(
        (status = 200, body = AxisCandidatesView),
        (status = "default", body = ErrorBody),
    ),
    extensions(("x-personacore-stability" = json!("experimental"))),
)]
pub(super) async fn get_axis_candidates(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<AxisCandidatesView>, ApiError> {
    let handle = path_handle(path)?;
    let character = find_character(&state, &handle).await?;

    let candidates = state
        .store()
        .axis_candidates(character.id, AXIS_CANDIDATES_LIMIT)
        .await
        .map_err(|err| internal(&err, "軸の候補を取得できませんでした"))?;
    // **上限で切らない**（返した件数とは別の値 / [`AxisCandidatesView::total_candidates`]）。
    let total_candidates = state
        .store()
        .count_axis_candidates(character.id)
        .await
        .map_err(|err| internal(&err, "軸の候補を数えられませんでした"))?;

    Ok(Json(axis_candidates_view(
        &handle,
        candidates,
        total_candidates,
    )))
}

/// `GET /admin/characters/{handle}/sessions` — **会話の一覧**（作業単位 D-12c-1 / Issue origin#35）。
/// **読むだけ。**
///
/// # 並びは素の `aoi` が続ける順
///
/// **先頭が「素の `aoi` で話しかけたら続く会話」である**（`Store::latest_session` と
/// 同じ `ORDER BY` を共有している / `store::session` の `continuation_order`）。
/// 画面はこの先頭を「続ける会話」として出すので、**並びがずれると、
/// 画面に見えている会話と本文が書き込まれる会話が分かれる。**
///
/// # 一覧と総数を1つのトランザクションで囲わない
///
/// `get_attribute_revisions` と同じ判断——ずれるのは表示が1件ぶんだけで、
/// 囲うと本文処理の書き込みと取り合う。
#[utoipa::path(
    get,
    path = "/admin/characters/{handle}/sessions",
    summary = "List the conversations of a character",
    tag = "sessions",
    params(("handle" = String, Path)),
    responses(
        (status = 200, body = SessionListView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn list_sessions(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<SessionListView>, ApiError> {
    let handle = path_handle(path)?;
    let character = find_character(&state, &handle).await?;

    let sessions = state
        .store()
        .list_sessions(character.id, SESSION_LIST_LIMIT, SESSION_OPENING_CHARS)
        .await
        .map_err(|err| internal(&err, "会話の一覧を取得できませんでした"))?;
    // **上限で切らない**（返した件数とは別の値 / [`SessionListView::total_sessions`]）。
    let total_sessions = state
        .store()
        .count_sessions(character.id)
        .await
        .map_err(|err| internal(&err, "会話を数えられませんでした"))?;

    Ok(Json(SessionListView {
        character: handle.as_str().to_owned(),
        sessions: sessions.into_iter().map(SessionSummaryView::from).collect(),
        total_sessions,
        sessions_limit: SESSION_LIST_LIMIT,
    }))
}

/// `GET /admin/characters/{handle}/sessions/{session_id}/messages` — **1つの会話のメッセージ**
/// （作業単位 D-12c-1 / Issue origin#35）。**読むだけ。**
///
/// **古い順に並べた1ページ**を返し、`?before=<メッセージ id>` で遡る。
///
/// # 他のキャラクターの会話は覗けない
///
/// セッションは `Store::find_session` で引く——**キャラクターの条件も同じ1文に入っている**
/// （`aoi@<id>` と同じ所有確認）。他のキャラクターのものも、存在しないものも
/// **同じ 404 `session_not_found`**（`ApiError::session_not_found` の doc）。
///
/// # 検証の順序
///
/// パス → **問い合わせ** → キャラクター → セッション。**問い合わせの検証は DB に触れる前**
/// （`list_memories` と同じ順序）。見張り番は `tests/server_admin_sessions.rs::
/// malformed_paging_parameters_are_rejected_before_touching_the_database`
/// （**未知のキャラクターに不正な問い合わせ**を送る——正当な宛先だけでは順序は見えない）。
#[utoipa::path(
    get,
    path = "/admin/characters/{handle}/sessions/{session_id}/messages",
    summary = "Read the messages of one conversation",
    tag = "sessions",
    params(("handle" = String, Path), ("session_id" = i64, Path), MessagePageQuery),
    responses(
        (status = 200, body = MessagePageView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn list_session_messages(
    State(state): State<AppState>,
    path: Result<Path<(String, i64)>, PathRejection>,
    query: Result<Query<MessagePageQuery>, QueryRejection>,
) -> Result<Json<MessagePageView>, ApiError> {
    let (handle, raw_session_id) = path_handle_and_session_id(path)?;
    let Query(query) = query.map_err(|rejection| {
        tracing::debug!(error = %rejection, endpoint = ENDPOINT, "問い合わせパラメータを読めませんでした");
        ApiError::invalid_query()
    })?;
    let (before, limit) = message_page_of(&query)?;
    let character = find_character(&state, &handle).await?;

    let session = state
        .store()
        .find_session(character.id, raw_session_id)
        .await
        .map_err(|err| internal(&err, "会話を取得できませんでした"))?
        .ok_or_else(|| {
            tracing::debug!(
                character = %handle,
                session_id = raw_session_id,
                "そのキャラクターのものとして存在しないセッションが指定されました"
            );
            ApiError::session_not_found()
        })?;

    let MessagePage {
        messages,
        has_older,
    } = state
        .store()
        .message_page(session.id, before, limit)
        .await
        .map_err(|err| internal(&err, "会話のメッセージを取得できませんでした"))?;
    // **上限で切らない**（返した件数とは別の値 / [`MessagePageView::total_messages`]）。
    let total_messages = state
        .store()
        .count_messages(session.id)
        .await
        .map_err(|err| internal(&err, "会話のメッセージを数えられませんでした"))?;

    Ok(Json(MessagePageView {
        character: handle.as_str().to_owned(),
        session_id: session.id.get(),
        messages: messages.into_iter().map(MessageView::from).collect(),
        has_older,
        total_messages,
        limit,
    }))
}

/// 問い合わせを**カーソルと件数**にする（作業単位 D-12c-1）。**純粋関数。**
///
/// | 値 | 結果 |
/// |---|---|
/// | `before` 省略 | 最新から |
/// | `before` ≥ 1 | その id より古いものだけ（**存在は確かめない**——カーソルである） |
/// | `before` ≤ 0 | 400 `message_page_out_of_range`（id は 1 から振られる。0 以下を送る呼び出し側は壊れている） |
/// | `limit` 省略 | [`MESSAGE_PAGE_DEFAULT`] |
/// | `limit` が 1〜[`MESSAGE_PAGE_MAX`] | そのまま |
/// | それ以外 | 400 `message_page_out_of_range`（**黙って切り詰めない**） |
fn message_page_of(query: &MessagePageQuery) -> Result<(Option<MessageId>, u32), ApiError> {
    let before = match query.before {
        None => None,
        Some(raw) if raw >= 1 => Some(MessageId::from_row(raw)),
        Some(_) => return Err(ApiError::message_page_out_of_range()),
    };
    let limit = match query.limit {
        None => MESSAGE_PAGE_DEFAULT,
        Some(raw) => u32::try_from(raw)
            .ok()
            .filter(|limit| (1..=MESSAGE_PAGE_MAX).contains(limit))
            .ok_or_else(ApiError::message_page_out_of_range)?,
    };
    Ok((before, limit))
}

/// `PUT /admin/characters/{handle}/attributes` — キャラクター属性の**丸ごと置き換え**。
///
/// 要求に含まれない属性は消える（`store` の `replace_character_attributes` の意味論そのまま）。
/// 空の配列を渡すと全属性が消える。**`PATCH` にしないのは `PUT .../stats` と同じ理由。**
///
/// # **壊れた行の復旧経路はここ**
///
/// `store` の置き換えは既存の行を読まないので、`GET` が 500 になる状態からでも届く。
/// 見張り番は `tests/server_admin_attributes.rs::
/// a_broken_stored_row_blocks_the_get_and_the_put_fixes_it`。
#[utoipa::path(
    put,
    path = "/admin/characters/{handle}/attributes",
    summary = "Replace all attributes of a character",
    tag = "attributes",
    params(("handle" = String, Path)),
    request_body = ReplaceAttributesRequest,
    responses(
        (status = 200, body = AttributesView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn replace_attributes(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    // **`Json` は最後に置く**（本文を消費する抽出器は1つだけ、かつ末尾）。
    body: Result<Json<ReplaceAttributesRequest>, JsonRejection>,
) -> Result<Json<AttributesView>, ApiError> {
    let handle = path_handle(path)?;
    // **本文の拒否は `ApiError::from_json_rejection` に一本化してある。**
    let Json(request) =
        body.map_err(|rejection| ApiError::from_json_rejection(rejection, ENDPOINT))?;

    // **検証を DB に触れる前に済ませる。** 1つでも不正なら1行も書かない。
    // 件数の上限も `AttributeSet::parse` がここで見る（`store` は数え直さない）。
    let attributes = attribute_set(request)?;

    // **キャラクターを引くのは検証の後。** 存在しないキャラクターへ
    // 不正な要求を送ったときに、404 と 400 のどちらが返るかを決めるのはこの順序。
    // **400 を優先する**——宛先が正しくても直すべきものが残っているため。
    // 見張り番は `tests/server_admin_attributes.rs::
    // an_invalid_body_for_an_unknown_character_is_a_bad_request`。
    // **正当な本文を未知のキャラクターへ送るテストではこの順序を観測できない**
    // （どちらの順でも 404 になる）。「不正な本文 × 未知のキャラクター」が要る。
    let character = find_character(&state, &handle).await?;

    state
        .store()
        .replace_character_attributes(character.id, &attributes)
        .await
        .map_err(|err| classify_replace_attributes(&err))?;

    // 読み直して返す（応答が DB の状態と一致していることを保証する）。
    let stored = state
        .store()
        .character_attributes(character.id)
        .await
        .map_err(|err| internal(&err, "キャラクター属性を取得できませんでした"))?;
    tracing::info!(
        character = %handle,
        attributes = stored.len(),
        "キャラクター属性を設定しました"
    );

    Ok(Json(attributes_view(&handle, stored)))
}

/// `GET /admin/characters/{handle}/memories` — **修復のための記憶一覧**（ADR-0022 §2）。
///
/// # **これは想起ではない**
///
/// 想起の入口は `Store::recall_memories` ひとつだけ（ADR-0003）。
/// ここが返すものは**プロンプトに一切載らない。**
///
/// # なぜ `recall_memories` で代用できないのか
///
/// あちらは**1行でも壊れていれば丸ごと 500** になる。それは想起としては正しいが、
/// **修復には使えない**——直したい行がある状態では読めない。
/// 壊れた行は `unreadable` に出す（`GET /admin/characters` と同じ形 / ADR-0016 §3）。
///
/// **`characters` にだけ詰めて捨てないこと。** 捨てると
/// `Store::list_memories_for_repair` が指し示した意味がここで消える
/// （黙って読み飛ばすのと同じ。**型は強制しない** / `MemoryEntry` の doc）。
#[utoipa::path(
    get,
    path = "/admin/characters/{handle}/memories",
    summary = "List the memories of a character for review",
    tag = "memories",
    params(("handle" = String, Path), MemoryListQuery),
    responses(
        (status = 200, body = MemoryListView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn list_memories(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    query: Result<Query<MemoryListQuery>, QueryRejection>,
) -> Result<Json<MemoryListView>, ApiError> {
    let handle = path_handle(path)?;
    // **問い合わせの検証は DB に触れる前に済ませる**（本文の検証と同じ順序）。
    // 知らない欄・知らない値なら、宛先が未知でも 400 を返す。
    let Query(query) = query.map_err(|rejection| {
        tracing::debug!(error = %rejection, endpoint = ENDPOINT, "問い合わせパラメータを読めませんでした");
        ApiError::invalid_query()
    })?;
    let character = find_character(&state, &handle).await?;

    let entries = state
        .store()
        .list_memories_for_repair(character.id)
        .await
        .map_err(|err| internal(&err, "記憶の一覧を取得できませんでした"))?;

    let mut memories = Vec::with_capacity(entries.len());
    let mut unreadable = Vec::new();
    for entry in entries {
        match entry {
            MemoryEntry::Readable(memory) => memories.push(MemoryView::from(memory)),
            MemoryEntry::Unreadable(broken) => unreadable.push(UnreadableMemoryView::from(broken)),
        }
    }

    // **訊かれたときだけ読む。** 訊かれていなければ SQL も投げない
    // ——既定の応答は 14b から1バイトも変わらない。
    let superseded = match query.include {
        None => None,
        Some(MemoryListInclude::Superseded) => {
            let listed = state
                .store()
                .list_superseded_memories(character.id, RECENT_SUPERSEDED_MEMORIES)
                .await
                .map_err(|err| internal(&err, "上書きされた記憶の一覧を取得できませんでした"))?;
            // **上限で切らない**（返した件数とは別の値 / `RECENT_SUPERSEDED_MEMORIES` の doc）。
            let total = state
                .store()
                .count_superseded_memories(character.id)
                .await
                .map_err(|err| internal(&err, "上書きされた記憶を数えられませんでした"))?;
            Some(superseded_list_view(listed, total))
        }
    };

    Ok(Json(MemoryListView {
        character: handle.as_str().to_owned(),
        memories,
        unreadable,
        superseded,
    }))
}

/// 上書きされた記憶の節を組み立てる（作業単位 D-3b）。
///
/// # **読めなかった行を捨てない**
///
/// `filter_map` で `Unreadable` を落とす実装は**そのままコンパイルが通る**
/// （`MemoryEntry` の doc / 0012b で実測）。守っているのは
/// `tests/server_admin_memories.rs::
/// every_superseded_row_keeps_its_own_time_subject_category_and_successor` の
/// `unreadable` の欄。
///
/// # 並べ替えない
///
/// 並びは `store` の1文が決めている。ここで組み直すと、
/// **同じ「新しい順」という言葉が2つの意味を持つ。**
fn superseded_list_view(listed: Vec<SupersededMemory>, total: u32) -> SupersededListView {
    let mut memories = Vec::with_capacity(listed.len());
    let mut unreadable = Vec::new();
    for old in listed {
        // **後継はどちらの枝でも同じ行から取る**（読めたかどうかで帰属を変えない）。
        let superseded_by = old.superseded_by.get();
        match old.entry {
            MemoryEntry::Readable(memory) => memories.push(SupersededMemoryView {
                memory: MemoryView::from(memory),
                superseded_by,
            }),
            MemoryEntry::Unreadable(broken) => unreadable.push(UnreadableSupersededMemoryView {
                broken: UnreadableMemoryView::from(broken),
                superseded_by,
            }),
        }
    }

    SupersededListView {
        memories,
        unreadable,
        total,
        limit: RECENT_SUPERSEDED_MEMORIES,
    }
}

/// `POST /admin/characters/{handle}/memories/{memory_id}/corrections` — **訂正版で置き換える。**
///
/// ```text
/// 新しい行を INSERT   →   古い行の superseded_by に、その id を入れる
/// ```
///
/// 古い行は**消えない**（時系列保持）。想起から外れるだけで、本文も分類も残る。
/// 不可分性（両方入るか、どちらも入らないか）は `store` の1トランザクションが持つ
/// ——**ここで作り直さない。**
///
/// # なぜ `POST`（`PUT` / `DELETE` ではない）なのか
///
/// **起きるのは行の追加**である。`PUT .../memories/{id}` は「その行を置き換える」、
/// `DELETE` は「消す」と読まれ、**どちらも ADR-0022 が却下した操作**の名前になる。
/// 生やしたパスが操作の意味を語るので、**消す道はパスとしても存在しない。**
/// 成功は **201**（新しい行ができた / `POST /admin/characters` と同じ）。
///
/// # 検証は DB に触れる前に済ませる
///
/// 1つでも不正なら1行も書かない。**キャラクターを引くのは検証の後**
/// （`replace_attributes` と同じ順序。不正な本文を未知のキャラクターへ送ったら 400）。
///
/// **正当な本文を未知のキャラクターへ送るテストではこの順序を観測できない**
/// （どちらの順でも 404 になる）。**「不正な本文 × 未知のキャラクター」が要る。**
/// 見張り番は `tests/server_admin_memories.rs::
/// an_invalid_body_for_an_unknown_character_is_a_bad_request`
/// ——`replace_attributes` の doc が同じことを書いていたのに、
/// 14b はこの順序と主張だけを写して見張り番を写さなかった（レビューで指摘された）。
#[utoipa::path(
    post,
    path = "/admin/characters/{handle}/memories/{memory_id}/corrections",
    summary = "Correct one of the memories by appending a new version",
    tag = "memories",
    params(("handle" = String, Path), ("memory_id" = i64, Path)),
    request_body = CorrectMemoryRequest,
    responses(
        (status = 201, body = CorrectionView),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn correct_memory(
    State(state): State<AppState>,
    path: Result<Path<(String, i64)>, PathRejection>,
    // **`Json` は最後に置く**（本文を消費する抽出器は1つだけ、かつ末尾）。
    body: Result<Json<CorrectMemoryRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<CorrectionView>), ApiError> {
    let (handle, superseded) = path_handle_and_memory_id(path)?;
    // **本文の拒否は `ApiError::from_json_rejection` に一本化してある。**
    let Json(request) =
        body.map_err(|rejection| ApiError::from_json_rejection(rejection, ENDPOINT))?;
    let correction = new_memory(request)?;

    let character = find_character(&state, &handle).await?;

    let correction_id = state
        .store()
        .supersede_memory(character.id, superseded, &correction)
        .await
        .map_err(|err| classify_correction(&err))?;

    tracing::info!(
        character = %handle,
        superseded = superseded.get(),
        correction = correction_id.get(),
        "記憶を訂正版で置き換えました"
    );

    Ok((
        StatusCode::CREATED,
        Json(CorrectionView {
            character: handle.as_str().to_owned(),
            superseded_id: superseded.get(),
            correction_id: correction_id.get(),
        }),
    ))
}

/// 訂正の要求を検証済みの記憶にする。**検証は追加と同じ型を通す。**
///
/// ここを素通しにすると、抽出の入口で塞いだ穴（暴走した本文・分類）が
/// 管理 API 側に開き直る。**全件注入なので、1件入ると以後の全プロンプトに載り続ける。**
fn new_memory(request: CorrectMemoryRequest) -> Result<NewMemory, ApiError> {
    Ok(NewMemory {
        subject: memory_subject(&request.subject)?,
        content: MemoryContent::parse(&request.content)
            .map_err(ApiError::invalid_memory_content)?,
        category: request
            .category
            .as_deref()
            .map(MemoryCategory::parse)
            .transpose()
            .map_err(ApiError::invalid_memory_category)?,
    })
}

/// 要求を検証済みの集合にする。**件数の上限もここで見る。**
///
/// **キー → 値 → 主語の順に検証しない**（1件ずつ、主語・キー・値の順に見る）。
/// 先に見つかった不備の理由を返すので、**同じ要求に複数の不備があるときの応答は
/// 「最初の1件の、最初の不備」**になる。修復は何往復もするものなので、
/// ここで全部を集めて返す形（`UnreadableCharacter` の `invalid_columns`）は取っていない
/// ——あちらは**直したい行を指し示す**ための一覧で、こちらは**送り直す**だけで済む。
fn attribute_set(request: ReplaceAttributesRequest) -> Result<AttributeSet, ApiError> {
    let attributes = request
        .attributes
        .into_iter()
        .map(|attribute| {
            Ok(CharacterAttribute {
                subject: attribute_subject(&attribute.subject)?,
                key: AttributeKey::parse(&attribute.key)?,
                value: AttributeValue::parse(&attribute.value)?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(AttributeSet::parse(attributes)?)
}

/// 手で書く要求を検証済みの取り決めにする（作業単位 A-12a）。**1つでも不正なら1行も書かない。**
///
/// 中身 → 種類 → 守る側の順に見て、最初の不備の理由を返す。
fn new_agreement(request: CreateAgreementRequest) -> Result<NewAgreement, ApiError> {
    let content = AgreementContent::parse(&request.content)?;
    let kind = AgreementKind::from_label(&request.kind).ok_or_else(|| {
        // 入力は**ログにだけ**出す（応答には反射しない）。
        tracing::debug!(kind = %request.kind, "未知の取り決めの種類が指定されました");
        ApiError::invalid_agreement_kind()
    })?;
    let keeper = AgreementKeeper::from_label(&request.keeper).ok_or_else(|| {
        tracing::debug!(keeper = %request.keeper, "未知の取り決めの守る側が指定されました");
        ApiError::invalid_agreement_keeper()
    })?;
    Ok(NewAgreement {
        content,
        kind,
        keeper,
    })
}

/// 採る要求を直しにする（作業単位 A-12c-1）。**1つでも不正なら何も変えない。**
///
/// 中身 → 種類 → 守る側の順に見て、最初の不備の理由を返す（`new_agreement` と同じ順・同じ理由）。
fn agreement_edits(request: AdoptAgreementRequest) -> Result<AgreementEdits, ApiError> {
    let content = request
        .content
        .as_deref()
        .map(AgreementContent::parse)
        .transpose()?;
    let kind = request
        .kind
        .as_deref()
        .map(|raw| {
            AgreementKind::from_label(raw).ok_or_else(|| {
                tracing::debug!(kind = %raw, "未知の取り決めの種類が指定されました");
                ApiError::invalid_agreement_kind()
            })
        })
        .transpose()?;
    let keeper = request
        .keeper
        .as_deref()
        .map(|raw| {
            AgreementKeeper::from_label(raw).ok_or_else(|| {
                tracing::debug!(keeper = %raw, "未知の取り決めの守る側が指定されました");
                ApiError::invalid_agreement_keeper()
            })
        })
        .transpose()?;
    Ok(AgreementEdits {
        content,
        kind,
        keeper,
    })
}

/// 終わらせる要求の `state` を終わらせ方にする。**`active` も知らない綴りも 400。**
fn agreement_ending(raw: &str) -> Result<AgreementEnding, ApiError> {
    AgreementEnding::from_label(raw).ok_or_else(|| {
        tracing::debug!(state = %raw, "未知の取り決めの終わらせ方が指定されました");
        ApiError::invalid_agreement_ending()
    })
}

/// 要求の `subject` を `domain::Subject` にする。**HTTP 上の綴りを決めるのはここだけ。**
///
/// # 綴りが3箇所にあるのは意図的
///
/// | 層 | 場所 | 何のための綴りか |
/// |---|---|---|
/// | HTTP | ここ | 要求と応答の語彙 |
/// | DB | `store::memory::subject_to_db` | `CHECK (subject IN (…))` と揃える綴り |
/// | domain | `Subject`（綴りを持たない） | 2値であることだけ |
///
/// **`domain::Subject` に `as_str` を生やして共有しない。**
/// 生やすと HTTP の語彙と DB の語彙が同じ定数に固定され、
/// 片方だけを変える（たとえば API を `character` から `persona` へ）ことが
/// **スキーマの CHECK を巻き込む変更**になる。
/// いまは 3 つとも同じ綴りだが、それは**揃えてあるだけで、同じ定数ではない。**
/// 揃っていることは `subject_uses_the_wire_spelling` と
/// `tests/server_admin_attributes.rs::an_unknown_subject_is_rejected_with_its_own_reason` が見る。
///
/// **trim も小文字化もしない。** 受理を広げると、`"User"` を送った側が
/// 「通ったのだから正しい綴り」と学習し、綴りが2つに増える。
///
/// # **綴りの判定はここだけ。理由の選択は呼び出し側**（作業単位 14b）
///
/// 属性（`PUT .../attributes`）と記憶の訂正（`POST .../corrections`）が同じ綴りを使うが、
/// **返すべき理由は別**——「属性の主語は…」と返された運用者は属性の要求を見に行く。
/// 綴りの表をここに1つだけ置き、`ApiError` は `attribute_subject` / `memory_subject` が選ぶ。
/// **表を写さない**（写すと、片方の受理集合だけが黙って広がる）。
pub(super) fn subject_of(raw: &str) -> Option<Subject> {
    match raw {
        "user" => Some(Subject::User),
        "character" => Some(Subject::Character),
        _ => None,
    }
}

/// 属性の要求の主語。**綴りの判定は `subject_of`。**
fn attribute_subject(raw: &str) -> Result<Subject, ApiError> {
    subject_of(raw).ok_or_else(|| {
        // 入力は**ログにだけ**出す（応答には反射しない）。
        tracing::debug!(subject = %raw, "未知の属性の主語が指定されました");
        ApiError::invalid_attribute_subject()
    })
}

/// 記憶の訂正の要求の主語。**綴りの判定は `subject_of`。**
fn memory_subject(raw: &str) -> Result<Subject, ApiError> {
    subject_of(raw).ok_or_else(|| {
        tracing::debug!(subject = %raw, "未知の記憶の主語が指定されました");
        ApiError::invalid_memory_subject()
    })
}

/// `domain::Subject` を応答の綴りにする。**`subject_of` の逆向き。**
///
/// 定数を共有せずに書き下すのは、`subject_of` の doc の理由による。
/// 往復が成り立つことは単体テストが見張る。
pub(super) const fn subject_to_wire(subject: Subject) -> &'static str {
    match subject {
        Subject::User => "user",
        Subject::Character => "character",
    }
}

/// `Role` を HTTP 上の綴りにする（会話履歴の応答 / 作業単位 D-12c-1）。
///
/// **DB の綴り（`store::session` の `role_to_db`）とたまたま同じだが、共有しない。**
/// あちらは `migrations/0001_initial.sql` の CHECK に縛られ、こちらは画面の写し
/// （`web/src/lib/api.ts` の `MessageRole`）に縛られる。**縛る相手が違う。**
/// 綴りそのものは単体テスト（`role_uses_the_spelling_the_screen_reads`）が固定する。
pub(super) const fn role_to_wire(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

// ----------------------------------------------------------------------
// 仕様書の閉じた綴り（作業単位 P-2a / `openapi` モジュールの doc）
//
// **値は、応答を組むのと同じ定義・同じ関数から作る**（綴りを写さない）。
// 付け忘れ・付け違いは `openapi::tests::closed_values_come_from_their_definitions`、
// 綴りそのものは `closed_values_are_the_spellings_on_the_wire` が見る。
// ----------------------------------------------------------------------

fn subject_schema() -> Object {
    openapi::closed_string(Subject::ALL.map(subject_to_wire))
}

fn role_schema() -> Object {
    openapi::closed_string(Role::ALL.map(role_to_wire))
}

fn delta_schema() -> Object {
    openapi::closed_string(DriftDelta::ALL.map(delta_to_wire))
}

fn drift_mode_schema() -> Object {
    openapi::closed_string(StatDriftMode::ALL.map(StatDriftMode::label))
}

fn nullable_drift_mode_schema() -> Object {
    openapi::nullable_closed_string(StatDriftMode::ALL.map(StatDriftMode::label))
}

fn run_mode_source_schema() -> Object {
    openapi::closed_string(DriftModeSource::ALL.map(DriftModeSource::label))
}

fn candidate_state_schema() -> Object {
    openapi::closed_string(AxisCandidateState::ALL.map(AxisCandidateState::label))
}

fn trial_failure_schema() -> Object {
    openapi::nullable_closed_string(AxisTrialFailure::ALL.map(AxisTrialFailure::label))
}

fn agreement_kind_schema() -> Object {
    openapi::closed_string(AgreementKind::ALL.map(AgreementKind::label))
}

fn agreement_keeper_schema() -> Object {
    openapi::closed_string(AgreementKeeper::ALL.map(AgreementKeeper::label))
}

fn agreement_state_schema() -> Object {
    openapi::closed_string(AgreementState::ALL.map(AgreementState::label))
}

fn nullable_agreement_state_schema() -> Object {
    openapi::nullable_closed_string(AgreementState::ALL.map(AgreementState::label))
}

/// 終わらせ方の綴り（**`AgreementState::label` を通す**。写しを作らない）。
fn agreement_ending_schema() -> Object {
    openapi::closed_string(AgreementEnding::ALL.map(|ending| ending.state().label()))
}

fn memory_columns_schema() -> Array {
    openapi::closed_string_array(MemoryColumn::ALL.map(MemoryColumn::as_str))
}

fn character_columns_schema() -> Array {
    openapi::closed_string_array(CharacterColumn::ALL.map(CharacterColumn::as_str))
}

/// 応答の組み立て。
fn attributes_view(handle: &Handle, attributes: Vec<CharacterAttribute>) -> AttributesView {
    AttributesView {
        character: handle.as_str().to_owned(),
        attributes: attributes
            .into_iter()
            .map(|attribute| AttributeView {
                subject: subject_to_wire(attribute.subject),
                key: attribute.key.as_str().to_owned(),
                value: attribute.value.as_str().to_owned(),
            })
            .collect(),
    }
}

/// 履歴の応答の組み立て。
///
/// **`subject` の綴りは `subject_to_wire`**（属性の応答と同じ表を通す。写さない）。
fn attribute_revisions_view(
    handle: &Handle,
    revisions: Vec<AttributeRevision>,
    total_revisions: u32,
) -> AttributeRevisionsView {
    AttributeRevisionsView {
        character: handle.as_str().to_owned(),
        revisions: revisions
            .into_iter()
            .map(|revision| AttributeRevisionView {
                id: revision.id(),
                subject: subject_to_wire(revision.subject()),
                key: revision.key().as_str().to_owned(),
                before: revision.before().map(|value| value.as_str().to_owned()),
                after: revision.after().map(|value| value.as_str().to_owned()),
                created_at: revision.created_at(),
            })
            .collect(),
        total_revisions,
        revisions_limit: ATTRIBUTE_REVISIONS_LIMIT,
    }
}

/// 軸の候補の応答を組み立てる（ADR-0046 §決定6）。
fn axis_candidates_view(
    handle: &Handle,
    candidates: Vec<AxisCandidate>,
    total_candidates: u32,
) -> AxisCandidatesView {
    AxisCandidatesView {
        character: handle.as_str().to_owned(),
        candidates: candidates
            .into_iter()
            .map(|candidate| {
                let tally = candidate.tally();
                AxisCandidateView {
                    id: candidate.id().get(),
                    name: candidate.name().as_str().to_owned(),
                    source_memory_id: candidate.source_memory().get(),
                    created_at: candidate.created_at(),
                    state: candidate.state().label(),
                    reason: failure_label(candidate.state()),
                    transitions: candidate
                        .transitions()
                        .iter()
                        .map(|transition| AxisCandidateTransitionView {
                            state: transition.state().label(),
                            reason: failure_label(transition.state()),
                            created_at: transition.created_at(),
                        })
                        .collect(),
                    trial_votes: AxisTrialVotesView {
                        up: tally.up,
                        down: tally.down,
                        zero: tally.zero,
                    },
                }
            })
            .collect(),
        total_candidates,
        candidates_limit: AXIS_CANDIDATES_LIMIT,
    }
}

/// 取り決めの一覧の応答を組み立てる（作業単位 A-12a）。
fn agreements_view(
    handle: &Handle,
    agreements: Vec<Agreement>,
    fit: Option<super::AgreementFit>,
) -> AgreementsView {
    // **`usize` → `u32` は panic せず飽和させる**（件数は 16 前後で、収まらない値は起こりようがない）。
    let count = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
    AgreementsView {
        character: handle.as_str().to_owned(),
        agreements: agreements.into_iter().map(agreement_view).collect(),
        active_limit: count(MAX_ACTIVE_AGREEMENTS),
        last_turn: fit.map(|fit| AgreementTurnView {
            included: count(fit.included),
            dropped: count(fit.dropped),
            at: fit.at,
        }),
    }
}

/// 取り決め1件の応答。**綴りの正は domain の `label`**（保存と応答で同じ綴り）。
fn agreement_view(agreement: Agreement) -> AgreementView {
    AgreementView {
        id: agreement.id.get(),
        content: agreement.content.as_str().to_owned(),
        kind: agreement.kind.label(),
        keeper: agreement.keeper.label(),
        state: agreement.state.label(),
        source_session_id: agreement.source_session.map(SessionId::get),
        created_at: agreement.created_at,
    }
}

/// 取り決めを書くときのエラーを分類する。**上限は 409**（`agreement_limit_reached` の doc）。
fn classify_agreement_write(err: &StoreError) -> ApiError {
    match err {
        StoreError::AgreementLimitReached => ApiError::agreement_limit_reached(),
        _ => internal(err, "取り決めを書けませんでした"),
    }
}

/// 提案を採る・捨てるときのエラーを分類する（作業単位 A-12c-1）。**3種を 500 に潰さない。**
fn classify_agreement_decision(err: &StoreError) -> ApiError {
    match err {
        StoreError::UnknownAgreement => ApiError::agreement_not_found(),
        StoreError::AgreementNotProposed => ApiError::agreement_not_proposed(),
        StoreError::AgreementLimitReached => ApiError::agreement_limit_reached(),
        _ => internal(err, "取り決めの提案を採る・捨てることができませんでした"),
    }
}

/// 取り決めを終わらせるときのエラーを分類する。**2種を 500 に潰さない。**
fn classify_agreement_ending(err: &StoreError) -> ApiError {
    match err {
        StoreError::UnknownAgreement => ApiError::agreement_not_found(),
        StoreError::AgreementNotActive => ApiError::agreement_not_active(),
        _ => internal(err, "取り決めを終わらせられませんでした"),
    }
}

/// 不合格の理由の綴り（不合格以外は `None`）。**綴りの正は `AxisTrialFailure::label`。**
fn failure_label(state: AxisCandidateState) -> Option<&'static str> {
    state.reason().map(AxisTrialFailure::label)
}

/// キャラクター属性の置き換え時のエラーを分類する。
///
/// 主キー違反（同じ主語の同じキーが2件）だけを 400 に分け、残りは内部エラー。
/// **`duplicate_stat_axis` に相乗りさせない**（直す対象のテーブルが違う）。
fn classify_replace_attributes(err: &StoreError) -> ApiError {
    if err.is_unique_violation() {
        // 置き換え全体はロールバック済みなので、前の設定は残っている。
        return ApiError::duplicate_attribute();
    }
    internal(err, "キャラクター属性を設定できませんでした")
}

/// 要求の表示名を検証済みの型にする。**省略時は識別子の値で埋める。**
///
/// 埋めた場合の結果は 0004a 以前と同じ（名前がそのままプロンプトに載る）。
/// `handle` は `DisplayName` としても必ず正当なので、この経路で
/// エラーになるのは**明示的に不正な表示名を送った場合だけ**
/// （`domain` の `every_valid_handle_is_also_a_valid_display_name` が固定している）。
fn display_name_of(handle: &Handle, requested: Option<&str>) -> Result<DisplayName, ApiError> {
    Ok(DisplayName::parse(
        requested.unwrap_or_else(|| handle.as_str()),
    )?)
}

/// 要求の尺度を検証済みの型にする。**省略時は `StatScale::DEFAULT`。**
///
/// **省略を「既定」に落とすのはこの1箇所だけ。** `store::create_character` が
/// `StatScale` を必ず要求する形にしてあるので、既定の定義が2箇所に分かれない
/// （分かれると片方だけ変えても誰も気付けない）。
///
/// **`None` を `DEFAULT` に潰すのは作成のときだけ。** 更新（`character_update`）では
/// `None` は「変えない」であり、既定で上書きしない。
fn stat_scale_of(requested: Option<u32>) -> Result<StatScale, ApiError> {
    match requested {
        Some(raw) => Ok(StatScale::parse(raw)?),
        None => Ok(StatScale::DEFAULT),
    }
}

/// 更新要求を検証済みの差分にする。**指定されなかった列は「触らない」に落ちる。**
///
/// 検証は作成時とまったく同じ型（`DisplayName` / `Persona` / `OutputFormat`）を通す。
/// **ここを素通しにすると、`POST` で塞いだ「削れない枠」の穴が `PATCH` に開き直る。**
/// 上限を超えた値が入ると、そのキャラクターの以後のチャットが
/// `BudgetExhausted` になり続ける（CLAUDE.md 1節）。
fn character_update(request: UpdateCharacterRequest) -> Result<CharacterUpdate, ApiError> {
    Ok(CharacterUpdate {
        display_name: request
            .display_name
            .as_deref()
            .map(DisplayName::parse)
            .transpose()?,
        base_persona: request
            .base_persona
            .as_deref()
            .map(Persona::parse)
            .transpose()?,
        // **`Keep` を `Clear` に潰さない。** そこが3値である理由そのもの。
        output_format_instruction: match &request.output_format_instruction {
            FieldUpdate::Keep => FieldUpdate::Keep,
            FieldUpdate::Set(raw) => FieldUpdate::Set(OutputFormat::parse(raw)?),
            FieldUpdate::Clear => FieldUpdate::Clear,
        },
        // **`None` を `DEFAULT` に潰さない。** ここでの `None` は「変えない」であり、
        // 潰すと表示名だけを変える更新のたびに尺度が既定へ戻る。
        stat_scale: request.stat_scale.map(StatScale::parse).transpose()?,
        // **`Keep` を `Clear` に潰さない**（形式指示と同じ / ADR-0045 §決定3）。
        stat_drift_mode: match &request.stat_drift_mode {
            FieldUpdate::Keep => FieldUpdate::Keep,
            FieldUpdate::Set(raw) => FieldUpdate::Set(StatDriftMode::parse(raw)?),
            FieldUpdate::Clear => FieldUpdate::Clear,
        },
        // **`None` を 0 に潰さない**（尺度と同じ。`None` は「変えない」）。
        axis_discovery_slots: request
            .axis_discovery_slots
            .map(AxisDiscoverySlots::parse)
            .transpose()?,
        // **`None` を `false` に潰さない**（`None` は「変えない」。潰すと、表示名だけを変える
        // 更新のたびに、しまったキャラクターが黙って一覧へ戻ってくる）。
        archived: request.archived,
    })
}

/// 更新した行を読み直す。**読めなくても失敗ではない。**
///
/// `find_character_by_handle` は全フィールドを `parse` するので、
/// **更新が成功していても `Err(InvalidStoredValue)` になりうる**
/// （壊れた列がまだ残っている＝片方だけ直した状態。モジュール doc / ADR-0016）。
///
/// **これを 500 にしないことが肝心。** 修復は何往復もするものなので、
/// 途中の状態が「サーバ障害」に見えると手が止まる。
///
/// # `Ok(None)` はもう起きにくい（0004b-2）
///
/// 以前はここに「ゴースト行を直した。識別子は不変なので完全一致では引けないまま」
/// という経路があった。**更新の引き当てを完全一致にしたので、
/// 更新できた行の識別子は必ず読める**（引き当てられた＝`parse` を通る綴りだった）。
/// 残る `Ok(None)` は「commit 後・読み直し前に、その行が消えた」という競合だけで、
/// これは 404 ではなく 200 + `character: null` のままでよい（更新は実際に起きた）。
/// **経路を潰さないこと**——潰すと、その競合で嘘の 404 が返る。
async fn updated_character(
    state: &AppState,
    handle: &Handle,
) -> Result<Option<Character>, ApiError> {
    match state.store().find_character_by_handle(handle).await {
        Ok(found) => Ok(found),
        Err(StoreError::InvalidStoredValue) => {
            tracing::info!(
                character = %handle,
                "更新は成功しましたが、まだ読み出せない列が残っています"
            );
            Ok(None)
        }
        Err(err) => Err(internal(&err, "更新したキャラクターを取得できませんでした")),
    }
}

/// 応答の組み立て。**尺度は引いた行のものを載せる**（既定を書かない）。
fn stats_view(handle: &Handle, stat_scale: StatScale, stats: Vec<StatAlloc>) -> StatsView {
    StatsView {
        character: handle.as_str().to_owned(),
        stat_scale: stat_scale.get(),
        stats: stats
            .into_iter()
            .map(|alloc| StatAllocView {
                axis: alloc.axis.as_str().to_owned(),
                // **尺度は軸ごとには載せない**（`stat_scale` として1つだけ返す）。
                value: alloc.value.get(),
            })
            .collect(),
    }
}

/// `domain::DriftDelta` を応答の綴りにする。**HTTP 上の綴りを決めるのはここだけ。**
///
/// # 定数を共有せずに書き下す理由（`subject_to_wire` と同じ）
///
/// `DriftDelta::label` は**集計キー**（ログの欄と DB の綴り）である。
/// ここで借りると、**API の語彙を変えることがログと DB の綴りを変えること**になり、
/// 過去のログを読む道具と `CHECK (delta IN (…))` を巻き込む。
/// いまは同じ綴りだが、それは**揃えてあるだけで、同じ定数ではない。**
/// 揃っていることは `the_wire_spelling_of_a_direction_is_fixed` が見る。
pub(super) const fn delta_to_wire(delta: DriftDelta) -> &'static str {
    match delta {
        DriftDelta::Up => "up",
        DriftDelta::Down => "down",
    }
}

/// 変容の応答の組み立て。**軸の一覧は「割り当て・累積・票のどれかを持つ軸」の合併。**
///
/// # 0 で埋めるのはここ
///
/// 累積も集計も**行のある軸しか返さない**（`store` の doc）。
/// 埋めずに落とすと、**一度も採点されていない軸が一覧から消える**
/// ——「軸を消したのか、まだ動いていないのか」が読み手に区別できなくなる。
///
/// # **割り当ての無い軸も出す**（作業単位 B-16）
///
/// `PUT .../stats` は**全置換**なので、軸を1つ改名すると綴りが変わる。
/// **累積の表も票の表も `stat_alloc` への外部キーを持たない**
/// （持っているのは `character` への参照だけ）ので、**行は消えない。**
///
/// それでいて `total_votes` は数え続ける——
/// **「全 N 件」と言いながら、その票がどの行にも現れない**状態になっていた（D-4 の実測）。
///
/// > **表の名前をここに綴らないこと。** `store::drift` の走査
/// > （`no_statement_updates_or_deletes_a_vote`）は `src/**` を素のトークンで読むので、
/// > Rust のコメントに票の表の名前を書くと**それだけで赤になる**
/// > （あちらの doc が「代償」として明記している。着手時に実際に踏んだ）。
///
/// このリポジトリは一貫して**存在する行を落とさずに指し示す**立場を取る
/// （`list_characters` / `list_memories` の `unreadable`、値 0 の軸、上書きされた記憶）。
/// **ここも落とさない。ただし `value` は `null`** ——
/// 0 で埋めると値 0 の軸と混ざる（[`DriftAxisView::value`] の doc）。
///
/// **書き込みは1つも足していない。** 置き去りの行を消す経路も作らない
/// （票は追記のみ / ADR-0026 §決定3、累積には削除の導線が無い）。
/// **読み手が直す手段は `PUT .../stats` で同じ綴りを置き直すことだけ**で、
/// そうすれば累積も件数もそのまま戻る。
///
/// # 並び（**決めてある**）
///
/// 1. **割り当てのある軸が先。** `stat_alloc` の保存順のまま
///    ——画面は `GET .../stats` と2つの一覧を並べて出すので、
///    **先頭から揃っていないと行がずれる**（`the_allocated_axes_still_line_up_with_the_stats_endpoint`）。
/// 2. **割り当ての無い軸が後。** **軸名の昇順**（`StatAxis` の `Ord` = UTF-8 のバイト順）。
///
/// **同着は起きない。** 並べる鍵は軸名で、軸名は1体のキャラクターの中で一意である
/// （`stat_alloc` も累積の表も主キーに軸名を含み、集計は軸ごとに畳まれる）。
/// **第2の鍵が要らないのはこのため**で、「決め手が無い」のではなく「同着が作れない」。
///
/// **DB の返す順に頼らない。** 累積は主キーの索引を辿るので軸名の昇順で返り、
/// 集計も `GROUP BY axis` の一時 b-tree で軸名の昇順になる——
/// **つまり「並べていない実装」でも、片方だけ見れば正しく見える。**
/// 権威は `the_axes_without_an_allocation_are_sorted_by_name_however_they_arrive`
/// （**わざと崩した順**を渡せる純粋関数の側）と、索引を落として測る統合テスト。
#[allow(clippy::too_many_arguments)]
fn drift_view(
    handle: &Handle,
    stat_scale: StatScale,
    stats: Vec<StatAlloc>,
    charges: &[StoredDriftCharge],
    tallies: &[DriftVoteTally],
    total_votes: u32,
    recent: Vec<DriftVoteRecord>,
    stat_drift_mode: Option<StatDriftMode>,
    default_run_mode: StatDriftMode,
) -> DriftView {
    // **変容を走らせる側と同じ関数で決める**（ADR-0045 §決定2 / `Extraction::start`）。
    let run_mode = ResolvedDriftMode::resolve(stat_drift_mode, default_run_mode);
    let unallocated = unallocated_axes(&stats, charges, tallies);
    let mut axes = Vec::with_capacity(stats.len() + unallocated.len());
    for alloc in stats {
        // **尺度は軸ごとには載せない**（`stats_view` と同じ）。
        axes.push(axis_view(
            &alloc.axis,
            Some(alloc.value.get()),
            charges,
            tallies,
        ));
    }
    for axis in &unallocated {
        axes.push(axis_view(axis, None, charges, tallies));
    }

    DriftView {
        character: handle.as_str().to_owned(),
        stat_scale: stat_scale.get(),
        axes,
        total_votes,
        recent_limit: RECENT_DRIFT_VOTES,
        recent_votes: recent
            .into_iter()
            .map(|record| DriftVoteView {
                axis: record.vote().axis().as_str().to_owned(),
                delta: delta_to_wire(record.vote().delta()),
                // **どの往復で立った票か。落とすと帰属が消える。**
                session_id: record.session_id().get(),
                created_at: record.created_at(),
            })
            .collect(),
        // **綴りの正は `StatDriftMode::label`**（写しを作らない / `DriftView::run_mode`）。
        run_mode: run_mode.mode().label(),
        run_mode_source: run_mode.source().label(),
        default_run_mode: default_run_mode.label(),
    }
}

/// **累積か票を持つが、割り当ての無い軸**を軸名の昇順で返す（作業単位 B-16）。
///
/// # 2つの読みは「別の状態」を持っている
///
/// * 累積だけがある軸 —— 適用まで届いたが、その後に軸が改名された。
/// * 票だけがある軸 —— まだ閾値へ届いていないうちに軸が改名された。
///
/// **どちらか片方だけを見る実装では、もう片方の軸が消える。**
/// 見張り番は `tests/server_admin_drift.rs::
/// an_axis_with_only_a_charge_and_an_axis_with_only_votes_are_both_listed`。
///
/// # 重複はここで畳む
///
/// 累積と票の両方を持つ軸は2つの読みから来るので、
/// **並べ替えてから隣どうしを潰す**（`dedup` は隣接した重複しか消さないため、
/// `sort` が先である必要がある）。
///
/// # 軸の数は小さい
///
/// 軸の件数はキャラクターごとに「人が書いた数 + 改名した回数」で、
/// **削れない枠に載る以上そもそも増やせない**（ADR-0024 §1）。
/// 線形探索の入れ子で足りる——集合を組むより、**読んで分かる形**を選ぶ。
fn unallocated_axes(
    stats: &[StatAlloc],
    charges: &[StoredDriftCharge],
    tallies: &[DriftVoteTally],
) -> Vec<StatAxis> {
    let mut axes: Vec<StatAxis> = charges
        .iter()
        .map(StoredDriftCharge::axis)
        .chain(tallies.iter().map(DriftVoteTally::axis))
        .filter(|axis| !stats.iter().any(|alloc| alloc.axis == **axis))
        .cloned()
        .collect();
    axes.sort();
    axes.dedup();
    axes
}

/// 1軸ぶんの応答を組み立てる。**累積と件数の引き当ては1箇所。**
///
/// `value` は**呼び出し側が決める**——`Some` なら `stat_alloc` の行があり、
/// `None` なら**割り当てが無い**（[`DriftAxisView::value`] の doc）。
///
/// 行の無い軸は累積 0・件数 0/0 で埋める（`drift_view` の doc）。
///
/// > **名前に `drift` と `axis` を続けて綴らないこと。** `server::drift` の走査
/// > （`the_drift_fields_never_collide_with_another_module`）が、
/// > ログの欄名の綴りを `src/**` から探して落とす（着手時に実際に踏んだ）。
fn axis_view(
    axis: &StatAxis,
    value: Option<u32>,
    charges: &[StoredDriftCharge],
    tallies: &[DriftVoteTally],
) -> DriftAxisView {
    // **累積と時刻は同じ1行から取る。** 別々に引き当てると、
    // **ある軸の累積に別の軸の時刻が付く**形が書ける（帰属の壊れ方 / CLAUDE.md）。
    let stored = charges.iter().find(|stored| stored.axis() == axis);
    let tally = tallies.iter().find(|tally| tally.axis() == axis);

    DriftAxisView {
        axis: axis.as_str().to_owned(),
        value,
        charge: stored.map_or(0.0, StoredDriftCharge::charge),
        // **行が無ければ `null`。** 0 でも現在時刻でも埋めない（この欄の doc）。
        charge_updated_at: stored.map(StoredDriftCharge::updated_at),
        ups: tally.map_or(0, DriftVoteTally::ups),
        downs: tally.map_or(0, DriftVoteTally::downs),
    }
}

/// パスのキャラクター識別子を検証済みの型にする。
fn path_handle(path: Result<Path<String>, PathRejection>) -> Result<Handle, ApiError> {
    let Path(raw) = path.map_err(|rejection| {
        // 既定の本文は入力を含みうるので、詳細はログにだけ出す。
        tracing::debug!(error = %rejection, "パスのキャラクター識別子を読めませんでした");
        ApiError::invalid_path()
    })?;

    Ok(Handle::parse(&raw)?)
}

/// パスの「キャラクター識別子 + 記憶の主キー」を読む（作業単位 14b）。
///
/// # id は**未検証の値のまま** `store` へ渡す
///
/// `MemoryId::from_row` で包むが、**これは「所有確認を通った」という意味を持たない。**
/// 持ち主の確認は `Store::supersede_memory` がトランザクションの中で行い、
/// 別のキャラクターの行なら `MemoryBelongsToAnotherCharacter` で返る
/// ——**入口で確かめてから呼ぶ形にしない**（確かめてから書くまでの間に状態が変わる）。
///
/// `SessionRef::Existing` が生の `i64` を保つのと**同じ立場**だが、
/// あちらは `Store::find_session` が生の値を受け取れるのに対し、
/// こちらの `store` は `MemoryId` を要求する（14a のシグネチャ）。
/// **型の名前を「検証済み」と読まないこと。**
///
/// 数値として読めないパスは 400（`invalid_path`）。**404 に潰さない**
/// ——直すのはパスの形であって、id の値ではない。
///
/// # 識別子の綴りの理由は `invalid_path` に潰さない
///
/// **`path_handle` と同じ関数ではない。** あちらは他所のテストが見張っているが、
/// こちらは 14b で新設したので**別に見張り番が要る**
/// （`Handle::parse` を `invalid_path` に潰す変異が、着手時は全緑を通り抜けた）。
/// 見張り番は `tests/server_admin_memories.rs::
/// a_malformed_character_handle_keeps_its_own_reason_on_both_memory_paths`。
fn path_handle_and_memory_id(
    path: Result<Path<(String, i64)>, PathRejection>,
) -> Result<(Handle, MemoryId), ApiError> {
    let Path((raw_handle, raw_memory_id)) = path.map_err(|rejection| {
        // 既定の本文は入力を含みうるので、詳細はログにだけ出す。
        tracing::debug!(error = %rejection, "パスの記憶の指定を読めませんでした");
        ApiError::invalid_path()
    })?;

    Ok((
        Handle::parse(&raw_handle)?,
        MemoryId::from_row(raw_memory_id),
    ))
}

/// パスの「キャラクター識別子 + 取り決めの主キー」を読む（作業単位 A-12a）。
///
/// **取り決めの id は生の `i64` のまま返す。** 持ち主の確認は `Store::end_agreement` が
/// トランザクションの中で行う（入口で確かめてから呼ぶ形にしない）。
/// 数値として読めないパスは 400（`invalid_path`）。**識別子の綴りの理由は `invalid_path` に潰さない。**
fn path_handle_and_agreement_id(
    path: Result<Path<(String, i64)>, PathRejection>,
) -> Result<(Handle, i64), ApiError> {
    let Path((raw_handle, raw_agreement_id)) = path.map_err(|rejection| {
        // 既定の本文は入力を含みうるので、詳細はログにだけ出す。
        tracing::debug!(error = %rejection, "パスの取り決めの指定を読めませんでした");
        ApiError::invalid_path()
    })?;

    Ok((Handle::parse(&raw_handle)?, raw_agreement_id))
}

/// パスの「キャラクター識別子 + セッションの主キー」を読む（作業単位 D-12c-1）。
///
/// **セッションの id は生の `i64` のまま返す。** `SessionId` は所有確認
/// （`Store::find_session`）を通ってしか作らない（`find_session` の doc の不変条件）。
///
/// 数値として読めないパスは 400（`invalid_path`）。**404 に潰さない**
/// ——直すのはパスの形であって、id の値ではない（`path_handle_and_memory_id` と同じ）。
/// **識別子の綴りの理由は `invalid_path` に潰さない**（見張り番は
/// `tests/server_admin_sessions.rs::a_malformed_path_keeps_its_own_reason`）。
fn path_handle_and_session_id(
    path: Result<Path<(String, i64)>, PathRejection>,
) -> Result<(Handle, i64), ApiError> {
    let Path((raw_handle, raw_session_id)) = path.map_err(|rejection| {
        // 既定の本文は入力を含みうるので、詳細はログにだけ出す。
        tracing::debug!(error = %rejection, "パスのセッションの指定を読めませんでした");
        ApiError::invalid_path()
    })?;

    Ok((Handle::parse(&raw_handle)?, raw_session_id))
}

/// 識別子でキャラクターを引く。見つからなければ 404。
async fn find_character(state: &AppState, handle: &Handle) -> Result<Character, ApiError> {
    state
        .store()
        .find_character_by_handle(handle)
        .await
        .map_err(|err| internal(&err, "キャラクターを取得できませんでした"))?
        .ok_or_else(|| {
            tracing::debug!(character = %handle, "存在しないキャラクターが指定されました");
            ApiError::character_not_found()
        })
}

/// 作成時のエラーを分類する。
///
/// # かつては重複が2種類あった（0004b-2 で1種類になった）
///
/// 「ゴースト行との衝突」（`StoreError::ConflictingUnnormalizedName` → 409
/// `conflicting_unnormalized_name`）を正当な重複と分けていたが、
/// **正規化キーの UNIQUE 索引を落とした**ので、その状態が作れなくなった
/// （`migrations/0006_drop_character_name_key_idx.sql`）。
/// いま UNIQUE 違反が返るのは「まったく同じ綴りの行が既にある」ときだけ。
fn classify_create(err: &StoreError) -> ApiError {
    if err.is_unique_violation() {
        return ApiError::character_already_exists();
    }
    internal(err, "キャラクターを作成できませんでした")
}

/// 更新時のエラーを分類する。
///
/// **対象が無いのは 404。** `store` の引き当ては完全一致（0004b-2）なので、これは
/// 「その綴りの行が1件も無い」ことを意味する。
///
/// **尺度が保存済みの値より小さいのは 409**（ADR-0015 / 作業単位 0011a-2）。
/// 要求そのものは正しく、**現在の状態と衝突している**だけ
/// （先例は `character_has_sessions`）。400 にすると「送り方が悪い」と読めてしまい、
/// 実際にやるべきこと（**先に `PUT .../stats` で値を下げる**）に辿り着けない。
/// **`stat_scale_too_small`（400 / 尺度そのものが下限未満）とも別のコード。**
fn classify_update(err: &StoreError) -> ApiError {
    match err {
        StoreError::UnknownCharacter => ApiError::character_not_found(),
        StoreError::StatScaleBelowStoredValue => ApiError::stat_scale_below_stored_value(),
        _ => internal(err, "キャラクターを更新できませんでした"),
    }
}

/// 性格ステータス置き換え時のエラーを分類する。
///
/// # `StatScaleMismatch` が返るのは**競合したとき**（作業単位 0011a-2）
///
/// 値は直前に引いた `character.stat_scale` で組み立てているので、通常は一致する。
/// 食い違うのは、引いてから書くまでの間に `PATCH` が尺度を変えた場合だけ。
/// **500 に潰さないこと**——サーバの障害ではなく状態との衝突なので、
/// 送り直せば通る（`character_has_sessions` と同じ 409）。
fn classify_replace_stats(err: &StoreError) -> ApiError {
    if err.is_unique_violation() {
        // 同一軸の重複（正規化後に同じになる場合を含む）。置き換え全体は
        // ロールバック済みなので、前の設定は残っている。
        return ApiError::duplicate_stat_axis();
    }
    match err {
        StoreError::StatScaleMismatch => ApiError::stat_scale_changed(),
        _ => internal(err, "性格ステータスを設定できませんでした"),
    }
}

/// 削除時のエラーを分類する。
///
/// **「該当なし」と「会話があるので消せない」を同じ応答に潰さない**（ADR-0016 §2）。
/// 前者は名前の打ち間違いなので綴りを直せば通るが、後者は仕様上の拒否なので
/// 綴りを直しても永久に通らない。**対処が正反対。**
fn classify_delete(err: &StoreError) -> ApiError {
    match err {
        StoreError::UnknownCharacter => ApiError::character_not_found(),
        StoreError::CharacterHasSessions => ApiError::character_has_sessions(),
        _ => internal(err, "キャラクターを削除できませんでした"),
    }
}

/// 訂正時のエラーを分類する（ADR-0022 / 作業単位 14b）。
///
/// # **3種を1つに潰さないこと**
///
/// `_ => internal(...)` に任せると**3種とも 500 になり、`store` が型を分けた意味が消える**
/// （14a のレビューの申し送り）。500 は「サーバの障害」なので、
/// 運用者は**送り直すか、諦めるか**しかできない——実際にはどれも**直せる拒否**である。
///
/// | `StoreError` | 応答 | 直す相手 |
/// |---|---|---|
/// | `UnknownMemory` | 404 `memory_not_found` | パスの id（打ち間違い） |
/// | `MemoryBelongsToAnotherCharacter` | 404 `memory_belongs_to_another_character` | パスの識別子（宛先違い） |
/// | `MemoryAlreadySuperseded` | 409 `memory_already_superseded` | 一覧を取り直して、いま有効な行を指す |
///
/// **前2つが同じ 404 なのは意図的で、`code` が違う。**
/// どちらも「このキャラクターの記憶としては指せない」ので状態番号は同じになるが、
/// **直す相手が違う**ので理由は分ける。
/// 3つ目だけ 409 なのは、**行はこのキャラクターのもので、状態だけが衝突している**から
/// （先例は `character_has_sessions` / `stat_scale_below_stored_value`）。
///
/// **判定の順序は `store` が持つ**（宛先違いが先 / ADR-0022「拒否の優先順」）。
/// ここで並べ替えられる余地は無い——返ってくるのは常に1つ。
fn classify_correction(err: &StoreError) -> ApiError {
    match err {
        StoreError::UnknownMemory => ApiError::memory_not_found(),
        StoreError::MemoryBelongsToAnotherCharacter => {
            ApiError::memory_belongs_to_another_character()
        }
        StoreError::MemoryAlreadySuperseded => ApiError::memory_already_superseded(),
        _ => internal(err, "記憶を訂正できませんでした"),
    }
}

/// 想定外の失敗。**詳細はログにだけ出す。**
///
/// `Debug` は `#[source]` 経由でパスを含みうるので、出すのはログに限る。
fn internal(err: &StoreError, message: &'static str) -> ApiError {
    tracing::error!(error = %err, detail = ?err, "{}", message);
    ApiError::internal()
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::{Value, json};

    use super::*;
    use crate::domain::CharacterId;

    fn axis(raw: &str) -> StatAxis {
        StatAxis::parse(raw).expect("テストで使う軸名は正当であること")
    }

    fn handle(raw: &str) -> Handle {
        Handle::parse(raw).expect("テストで使う名前は正当であること")
    }

    fn display_name(raw: &str) -> DisplayName {
        DisplayName::parse(raw).expect("テストで使う表示名は正当であること")
    }

    /// JSON の要求を検証済みの差分にする（**実際の入口と同じ2段**を通す）。
    fn update_from(raw: serde_json::Value) -> CharacterUpdate {
        let request =
            serde_json::from_value::<UpdateCharacterRequest>(raw).expect("要求として読めること");
        character_update(request).expect("正当な要求であること")
    }

    /// キャラクター1件の形。**未設定の形式指示は `null`**（キーごと落とさない）。
    #[test]
    fn a_character_view_keeps_a_missing_format_instruction_as_null() {
        let created = Utc
            .with_ymd_and_hms(2026, 8, 14, 1, 2, 3)
            .single()
            .expect("実在する日時であること");
        let view = CharacterView {
            id: 7,
            handle: "aoi-v2".to_owned(),
            display_name: "葵 碧".to_owned(),
            base_persona: "ペルソナ本文".to_owned(),
            output_format_instruction: None,
            // **既定（10）と違う値。** 既定にすると、`CharacterView` が
            // `stat_scale` を捨てて既定を書く実装と区別が付かない。
            stat_scale: 7,
            // **未設定も `null` でキーごと残す**（ADR-0045）。
            stat_drift_mode: None,
            // **既定（0）と違う値**（ADR-0046）。既定にすると、捨てて既定を書く実装と区別が付かない。
            axis_discovery_slots: 3,
            created_at: created,
            // **しまっていなくても `null` でキーごと残す**（ADR-0049 / 作業単位 A-10）。
            archived_at: None,
        };

        assert_eq!(
            serde_json::to_value(view).expect("JSON にできること"),
            json!({
                "id": 7,
                "handle": "aoi-v2",
                "display_name": "葵 碧",
                "base_persona": "ペルソナ本文",
                "output_format_instruction": null,
                "stat_scale": 7,
                "stat_drift_mode": null,
                "axis_discovery_slots": 3,
                "created_at": "2026-08-14T01:02:03Z",
                "archived_at": null,
            })
        );
    }

    /// **表示名は省略できる。省略時は識別子の値で埋まる**（ADR-0011 / 0004a）。
    ///
    /// 既存の投入クライアントが `display_name` を送らなくても、
    /// 0004a 以前と同じ結果（名前がそのままプロンプトに載る）になること。
    #[test]
    fn an_omitted_display_name_falls_back_to_the_handle() {
        assert_eq!(
            display_name_of(&handle("aoi"), None).map(|value| value.as_str().to_owned()),
            Ok("aoi".to_owned())
        );
        // 識別子に使える全文字が、そのまま表示名にもなれること
        // （`every_valid_handle_is_also_a_valid_display_name` の server 側の裏取り）。
        assert_eq!(
            display_name_of(&handle("aoi-v2_7"), None).map(|value| value.as_str().to_owned()),
            Ok("aoi-v2_7".to_owned())
        );
        // 明示された値が優先され、こちらも正規化される。
        assert_eq!(
            display_name_of(&handle("aoi"), Some(" 葵 碧 ")).map(|value| value.as_str().to_owned()),
            Ok("葵 碧".to_owned())
        );
        // **`@` は表示名では正当**（識別子の側だけの制約 / ADR-0011）。
        assert_eq!(
            display_name_of(&handle("aoi"), Some("葵@図書館"))
                .map(|value| value.as_str().to_owned()),
            Ok("葵@図書館".to_owned())
        );
    }

    /// 明示された表示名が不正なら、**表示名としての理由**で弾く。
    ///
    /// 識別子のエラーに潰すと、どちらの欄を直せばよいのか分からなくなる。
    #[test]
    fn an_invalid_display_name_is_rejected_with_its_own_reason() {
        assert_eq!(
            display_name_of(&handle("aoi"), Some("\u{3000}")),
            Err(ApiError::invalid_display_name(
                crate::domain::DisplayNameError::Empty
            ))
        );
        assert_eq!(
            display_name_of(&handle("aoi"), Some("葵\u{0007}")),
            Err(ApiError::invalid_display_name(
                crate::domain::DisplayNameError::Control
            ))
        );
        let over_limit = "あ".repeat(crate::domain::DisplayName::MAX_CHARS + 1);
        assert_eq!(
            display_name_of(&handle("aoi"), Some(&over_limit)),
            Err(ApiError::invalid_display_name(
                crate::domain::DisplayNameError::TooLong
            ))
        );
        assert_ne!(
            display_name_of(&handle("aoi"), Some("\u{3000}")),
            Err(ApiError::invalid_character_handle(
                crate::domain::HandleError::Empty
            )),
            "識別子のエラーに潰さないこと"
        );
    }

    /// 空の一覧も配列を持つ（`null` にしない）。**壊れた行の欄も同じ。**
    ///
    /// `unreadable` をキーごと落とすと、「壊れた行が無い」と
    /// 「壊れた行を見せない古い版」が受け取った側から区別できない。
    #[test]
    fn an_empty_character_list_still_has_an_array() {
        let value = serde_json::to_value(CharacterListView {
            characters: Vec::new(),
            unreadable: Vec::new(),
        })
        .expect("JSON にできること");

        assert_eq!(value, json!({"characters": [], "unreadable": []}));
    }

    /// **読み出せない行は、修復に要る手掛かりだけを載せる。**
    ///
    /// 格納値そのものは載せない（`domain::UnreadableCharacter` が持っていない）。
    /// 列名は**DB の列名と同じ綴り**で、そのまま `PATCH` の対象や
    /// `sqlite3` での確認に使える。
    #[test]
    fn an_unreadable_row_carries_the_pointer_and_the_columns() {
        let value = serde_json::to_value(CharacterListView {
            characters: Vec::new(),
            unreadable: vec![
                UnreadableCharacterView::from(UnreadableCharacter {
                    id: crate::domain::CharacterId::from_raw_for_tests(7),
                    handle: Some(handle("aoi")),
                    invalid_columns: vec![
                        CharacterColumn::DisplayName,
                        CharacterColumn::BasePersona,
                    ],
                }),
                // 識別子そのものが壊れている行は、指せる名前が無い。
                UnreadableCharacterView::from(UnreadableCharacter {
                    id: crate::domain::CharacterId::from_raw_for_tests(9),
                    handle: None,
                    invalid_columns: vec![CharacterColumn::Handle],
                }),
            ],
        })
        .expect("JSON にできること");

        assert_eq!(
            value,
            json!({
                "characters": [],
                "unreadable": [
                    {"id": 7, "handle": "aoi", "invalid_columns": ["display_name", "base_persona"]},
                    {"id": 9, "handle": null, "invalid_columns": ["handle"]},
                ],
            })
        );
    }

    /// 性格ステータスの形。**値 0 の軸を落とさない**（落とすのは `prompt` の責務）。
    #[test]
    fn a_stats_view_keeps_zero_valued_axes_in_the_given_order() {
        // **既定（10）と違う値。** 既定にすると、応答が要求を無視して
        // 既定を書いていても気付けない。
        let scale = StatScale::parse(7).expect("正当な尺度であること");
        // **値は尺度と別の数にする**（同値だと、値の欄に尺度を書く実装が通る）。
        let view = stats_view(
            &handle("aoi"),
            scale,
            vec![
                StatAlloc {
                    axis: axis("パッション"),
                    value: scale.value(0).expect("0 は正当な値であること"),
                },
                StatAlloc {
                    axis: axis("ロジック"),
                    value: scale.value(5).expect("尺度に収まること"),
                },
            ],
        );

        assert_eq!(
            serde_json::to_value(view).expect("JSON にできること"),
            json!({
                "character": "aoi",
                "stat_scale": 7,
                "stats": [
                    {"axis": "パッション", "value": 0},
                    {"axis": "ロジック", "value": 5},
                ],
            })
        );
    }

    /// 更新の応答は **id と「読み直せた姿」を分ける**。`character` は `null` になりうる。
    ///
    /// **`null` でもキーごと落とさない**（`unreadable` と同じ理由）。落とすと
    /// 「読めなかった」と「そういうキーを持たない古い版」が区別できない。
    #[test]
    fn an_updated_character_view_keeps_the_id_even_when_the_row_cannot_be_read() {
        let value = serde_json::to_value(UpdatedCharacterView {
            id: 7,
            character: None,
        })
        .expect("JSON にできること");

        assert_eq!(value, json!({"id": 7, "character": null}));
    }

    /// 要求の3値が `FieldUpdate` に**そのまま**移る（`Keep` を `Clear` に潰さない）。
    ///
    /// | JSON | 期待 |
    /// |---|---|
    /// | キーごと省略 | `Keep` |
    /// | `"..."` | `Set` |
    /// | `null` | `Clear` |
    #[test]
    fn an_omitted_format_instruction_is_kept_and_an_explicit_null_clears_it() {
        let keep = update_from(json!({"display_name": "葵"}));
        assert_eq!(
            keep.output_format_instruction,
            FieldUpdate::Keep,
            "キーを省いたら変えないこと"
        );
        assert_eq!(keep.display_name, Some(display_name("葵")));
        assert_eq!(keep.base_persona, None, "触れていない列は None であること");

        assert_eq!(
            update_from(json!({"output_format_instruction": "台詞は「」で囲むこと。"}))
                .output_format_instruction,
            FieldUpdate::Set(
                OutputFormat::parse("台詞は「」で囲むこと。").expect("正当な形式指示")
            )
        );

        let clear = update_from(json!({"output_format_instruction": null}));
        assert_eq!(clear.output_format_instruction, FieldUpdate::Clear);
        assert_ne!(
            clear.output_format_instruction,
            FieldUpdate::Keep,
            "「消す」を「変えない」と同じにしないこと"
        );

        // 1列も指定しない要求は「1列も変えない」。
        assert_eq!(update_from(json!({})), CharacterUpdate::default());
    }

    /// **変容の走らせ方も3状態**（ADR-0045 §決定3 / 作業単位 A-8）。
    ///
    /// | JSON | 期待 |
    /// |---|---|
    /// | キーごと省略 | `Keep` |
    /// | `"shadow"` | `Set(Shadow)` |
    /// | `null` | `Clear` |
    ///
    /// **形式指示の3状態と取り違えない**——同じ要求に両方を載せ、互いに違う状態にする。
    #[test]
    fn the_drift_mode_update_has_three_states() {
        let keep = update_from(json!({"output_format_instruction": null}));
        assert_eq!(
            keep.stat_drift_mode,
            FieldUpdate::Keep,
            "キーを省いたら変えない"
        );
        assert_eq!(keep.output_format_instruction, FieldUpdate::Clear);

        for (spelled, expected) in [
            ("off", StatDriftMode::Off),
            ("shadow", StatDriftMode::Shadow),
            ("on", StatDriftMode::On),
        ] {
            let set = update_from(json!({"stat_drift_mode": spelled}));
            assert_eq!(set.stat_drift_mode, FieldUpdate::Set(expected), "{spelled}");
            assert_eq!(
                set.output_format_instruction,
                FieldUpdate::Keep,
                "走らせ方だけの要求は形式指示に触らない"
            );
        }

        let clear = update_from(json!({"stat_drift_mode": null}));
        assert_eq!(clear.stat_drift_mode, FieldUpdate::Clear);
        assert_eq!(clear.output_format_instruction, FieldUpdate::Keep);
    }

    /// **環境変数の別名は、作成でも更新でも 400**（ADR-0045 §決定3）。
    ///
    /// 更新はこの関数（`character_update`）で見る。作成は handler の中なので、
    /// `tests/server_admin.rs` 側で本番の入口を通して見る。
    #[test]
    fn an_alias_of_the_drift_mode_is_refused() {
        for alias in ["1", "true", "yes", "0", "false", "no", "", "ON", " on"] {
            let request =
                serde_json::from_value::<UpdateCharacterRequest>(json!({"stat_drift_mode": alias}))
                    .expect("要求として読めること");
            assert_eq!(
                character_update(request),
                Err(ApiError::invalid_stat_drift_mode()),
                "{alias:?} を受けないこと"
            );
        }
        // **綴り以外の型は本文の形式違い**（`null` は `Clear` / 数値は 400 `invalid_body`）。
        assert!(
            serde_json::from_value::<UpdateCharacterRequest>(json!({"stat_drift_mode": 1}))
                .is_err()
        );
    }

    /// **書いた値は応答に出る**（ADR-0045 / 作業単位 A-8）。綴りは直書きで突き合わせる。
    #[test]
    fn a_character_view_names_the_drift_mode_it_was_written_with() {
        let created = Utc
            .with_ymd_and_hms(2026, 9, 26, 1, 2, 3)
            .single()
            .expect("実在する日時であること");
        let named: Vec<Value> = [
            None,
            Some(StatDriftMode::Off),
            Some(StatDriftMode::Shadow),
            Some(StatDriftMode::On),
        ]
        .into_iter()
        .map(|mode| {
            let view = CharacterView::from(Character {
                id: CharacterId::from_row(3),
                handle: handle("aoi"),
                display_name: display_name("葵"),
                base_persona: crate::domain::Persona::parse("ペルソナ本文")
                    .expect("正当なペルソナ"),
                output_format_instruction: None,
                stat_scale: StatScale::parse(7).expect("正当な尺度であること"),
                stat_drift_mode: mode,
                axis_discovery_slots: AxisDiscoverySlots::NONE,
                created_at: created,
                archived_at: None,
            });
            serde_json::to_value(view).expect("JSON にできること")["stat_drift_mode"].clone()
        })
        .collect();

        assert_eq!(
            named,
            vec![json!(null), json!("off"), json!("shadow"), json!("on")]
        );
    }

    /// **識別子は要求に載せられない**（ADR-0016 §1）。綴り違いも同じ扱い。
    ///
    /// `deny_unknown_fields` なので**要求まるごと**拒否される。
    /// 「未知のキーを黙って無視する」形にすると、改名したつもりの利用者が
    /// 200 を受け取り、**呼び名だけが変わって識別子は元のまま**の状態になる。
    #[test]
    fn an_update_request_has_no_field_for_the_handle() {
        for key in ["name", "handle", "id", "created_at", "displayname"] {
            let raw = json!({key: "rin", "display_name": "凛"});
            assert!(
                serde_json::from_value::<UpdateCharacterRequest>(raw).is_err(),
                "識別子を指しうるキー {key:?} は要求として成立しないこと"
            );
        }
        // 3つの列だけは通る（弾きすぎていないことの確認）。
        assert!(
            serde_json::from_value::<UpdateCharacterRequest>(json!({
                "display_name": "凛",
                "base_persona": "夜勤の警備員。",
                "output_format_instruction": "台詞は「」で囲むこと。",
            }))
            .is_ok()
        );
    }

    /// 更新も**作成と同じ上限**を通る（削れない枠の絶対制約）。
    ///
    /// 素通しにすると、`POST` で塞いだ穴が `PATCH` に開き直る。
    /// 理由は列ごとに区別できること（どの欄を直せばよいか応答から読めること）。
    #[test]
    fn an_update_goes_through_the_same_limits_as_creation() {
        let over_limit = "零".repeat(crate::domain::Persona::MAX_CHARS + 1);
        for (raw, expected) in [
            (
                json!({"display_name": "葵\u{0007}碧"}),
                ApiError::invalid_display_name(crate::domain::DisplayNameError::Control),
            ),
            (
                json!({"base_persona": over_limit}),
                ApiError::invalid_persona(crate::domain::PersonaError::TooLong),
            ),
            (
                json!({"base_persona": " \u{3000}\t"}),
                ApiError::invalid_persona(crate::domain::PersonaError::Empty),
            ),
            (
                // **空文字は「消す」ではなくエラー。** 消すのは `null`。
                json!({"output_format_instruction": ""}),
                ApiError::invalid_output_format(crate::domain::OutputFormatError::Empty),
            ),
        ] {
            let request = serde_json::from_value::<UpdateCharacterRequest>(raw)
                .expect("要求として読めること");
            assert_eq!(character_update(request), Err(expected));
        }
    }

    /// 更新・削除の拒否は**理由ごとに別の応答**（ADR-0016 §2）。
    ///
    /// 「そんなキャラクターは居ない」と「居るが消せない」を潰すと、
    /// 綴りを疑って直しても永久に通らない相手に送り続けることになる。
    #[test]
    fn an_unknown_character_and_a_conversation_are_different_refusals() {
        assert_eq!(
            classify_delete(&StoreError::UnknownCharacter),
            ApiError::character_not_found()
        );
        assert_eq!(
            classify_delete(&StoreError::CharacterHasSessions),
            ApiError::character_has_sessions()
        );
        assert_ne!(
            classify_delete(&StoreError::CharacterHasSessions),
            classify_delete(&StoreError::UnknownCharacter),
            "2つの拒否を同じ応答に潰さないこと"
        );
        // それ以外は 500（原因はログにだけ出す）。
        assert_eq!(
            classify_delete(&StoreError::InvalidStoredValue),
            ApiError::internal()
        );

        assert_eq!(
            classify_update(&StoreError::UnknownCharacter),
            ApiError::character_not_found()
        );
        assert_eq!(
            classify_update(&StoreError::InvalidStoredValue),
            ApiError::internal(),
            "格納値の不正を 404 に化けさせないこと"
        );
        // **更新に「会話がある」は関係しない**（禁じているのは削除だけ）。
        assert_eq!(
            classify_update(&StoreError::CharacterHasSessions),
            ApiError::internal()
        );
    }

    /// **重複は 409、それ以外は 500**（0004b-2 で分類が1本になった）。
    ///
    /// 以前はここに「ゴースト行との衝突を正当な重複に潰さない」があったが、
    /// **その状態を作る `StoreError` のバリアントごと消えた**
    /// （`migrations/0006_drop_character_name_key_idx.sql`）。
    ///
    /// # **この検査が守っているのは 500 の側だけ**（実測）
    ///
    /// `store` を通さずに UNIQUE 違反の `StoreError` は作れないので、ここは
    /// **409 を返す経路を一度も通っていない。** したがって
    /// **`is_unique_violation()` の分岐を落としてもこの検査は緑のまま**で、
    /// 赤くなるのは統合テスト
    /// `tests/server_admin.rs::a_ghost_row_no_longer_blocks_creating_the_same_handle`
    /// **1本だけ**（0004b-2 のレビューと追補で実測。スイート全体で 1 failed）。
    ///
    /// ここが赤にするのは「**重複でないものを 409 にする**」向きの変異
    /// （判定を反転する `if !err.is_unique_violation()` で実測。逆向きは緑のまま）。
    #[test]
    fn a_duplicate_handle_maps_to_conflict_and_nothing_else_does() {
        // 判定そのものを見る形にする（`is_unique_violation` は `Query` にしか真を返さない）。
        for err in [
            StoreError::InvalidStoredValue,
            StoreError::UnknownCharacter,
            StoreError::CharacterHasSessions,
            StoreError::UnknownSession,
            StoreError::InvalidTurn,
        ] {
            assert!(
                !err.is_unique_violation(),
                "前提: これらは重複ではないこと: {err:?}"
            );
            assert_eq!(
                classify_create(&err),
                ApiError::internal(),
                "重複でないものを 409 にしないこと: {err:?}"
            );
        }
        assert_ne!(
            ApiError::character_already_exists(),
            ApiError::internal(),
            "前提: 2つの応答は区別できること"
        );
    }

    /// パスの名前も `Handle` を通る（未検証の値が `store` へ届かない）。
    #[test]
    fn a_path_handle_goes_through_the_validated_type() {
        assert_eq!(
            path_handle(Ok(Path("aoi-v2".to_owned()))),
            Ok(handle("aoi-v2")),
            "規則を満たす識別子はそのまま通ること"
        );
        assert_eq!(
            path_handle(Ok(Path("aoi@new".to_owned()))),
            Err(ApiError::invalid_character_handle(
                crate::domain::HandleError::AtSign
            )),
            "予約語と衝突する名前は理由付きで弾くこと"
        );
        assert_eq!(
            path_handle(Ok(Path("\u{3000}".to_owned()))),
            Err(ApiError::invalid_character_handle(
                crate::domain::HandleError::Empty
            ))
        );
        // **パスの値も正規化されない**（ADR-0017）。`" aoi "` を `"aoi"` に直すと、
        // 直った名前で `store` を引くことになり、パスと引き当て先がずれる。
        assert_eq!(
            path_handle(Ok(Path(" aoi ".to_owned()))),
            Err(ApiError::invalid_character_handle(
                crate::domain::HandleError::Charset
            )),
            "前後に空白のあるパスは trim せずに弾くこと"
        );
    }

    /// **HTTP 上の `subject` の綴りは `user` / `character` のちょうど2つ。**
    ///
    /// **往復（`subject_to_wire(subject_of(x)) == x`）だけでは足りない。**
    /// 2つを入れ替えても往復は成立するので、**綴りそのもの**を固定する
    /// （`store::memory::subject_uses_the_spelling_the_schema_expects` と同じ形）。
    #[test]
    fn subject_uses_the_wire_spelling() {
        assert_eq!(subject_of("user"), Some(Subject::User));
        assert_eq!(subject_of("character"), Some(Subject::Character));
        assert_eq!(subject_to_wire(Subject::User), "user");
        assert_eq!(subject_to_wire(Subject::Character), "character");

        for subject in [Subject::User, Subject::Character] {
            assert_eq!(subject_of(subject_to_wire(subject)), Some(subject));
        }

        // **2つの入口が同じ表を引いている**（属性 / 記憶の訂正 / 作業単位 14b）。
        for subject in [Subject::User, Subject::Character] {
            let raw = subject_to_wire(subject);
            assert_eq!(attribute_subject(raw), Ok(subject), "raw = {raw:?}");
            assert_eq!(memory_subject(raw), Ok(subject), "raw = {raw:?}");
        }
    }

    /// **未知の綴りは専用の理由で拒否する。** `invalid_body` に潰さない。
    ///
    /// `assistant` は `message.role` の綴り、`Character` は大文字違い。
    /// **trim も小文字化もしない**（受理を広げると綴りが2つに増える）。
    ///
    /// # 属性と記憶で**別の理由**を返す（作業単位 14b）
    ///
    /// 綴りの判定は1箇所（`subject_of`）だが、**直す相手の要求が違う。**
    /// 「属性の主語は…」と返された運用者は `PUT .../attributes` の本文を見に行く
    /// ——訂正の要求を送ったのに。
    #[test]
    fn an_unknown_subject_spelling_is_refused() {
        for raw in [
            "assistant",
            "system",
            "User",
            "Character",
            "",
            " user",
            "user ",
            "USER",
        ] {
            assert_eq!(subject_of(raw), None, "raw = {raw:?}");
            assert_eq!(
                attribute_subject(raw),
                Err(ApiError::invalid_attribute_subject()),
                "raw = {raw:?}"
            );
            assert_eq!(
                memory_subject(raw),
                Err(ApiError::invalid_memory_subject()),
                "raw = {raw:?}"
            );
        }
        assert_ne!(
            attribute_subject("assistant"),
            Err(ApiError::invalid_body()),
            "本文の形式の誤りに潰さないこと"
        );
        assert_ne!(
            memory_subject("assistant"),
            attribute_subject("assistant"),
            "属性と記憶で別の理由を返すこと（直す相手の要求が違う）"
        );
    }

    /// 属性の応答の形。**空でも配列を持ち、`subject` を必ず載せる。**
    #[test]
    fn an_attributes_view_keeps_the_subject_of_every_row() {
        let view = attributes_view(
            &handle("aoi"),
            vec![
                CharacterAttribute {
                    subject: Subject::Character,
                    key: AttributeKey::parse("一人称").expect("正当なキー"),
                    value: AttributeValue::parse("僕").expect("正当な値"),
                },
                // **同じキーを別の主語が持つ**（`subject` を落とす実装がここで崩れる）。
                CharacterAttribute {
                    subject: Subject::User,
                    key: AttributeKey::parse("年齢").expect("正当なキー"),
                    value: AttributeValue::parse("32").expect("正当な値"),
                },
                CharacterAttribute {
                    subject: Subject::Character,
                    key: AttributeKey::parse("年齢").expect("正当なキー"),
                    value: AttributeValue::parse("19").expect("正当な値"),
                },
            ],
        );

        assert_eq!(
            serde_json::to_value(view).expect("JSON にできること"),
            json!({
                "character": "aoi",
                "attributes": [
                    {"subject": "character", "key": "一人称", "value": "僕"},
                    {"subject": "user", "key": "年齢", "value": "32"},
                    {"subject": "character", "key": "年齢", "value": "19"},
                ],
            })
        );
    }

    /// 属性が1件も無くても `attributes` は配列（`null` にしない）。
    #[test]
    fn an_empty_attributes_view_still_has_an_array() {
        assert_eq!(
            serde_json::to_value(attributes_view(&handle("aoi"), Vec::new()))
                .expect("JSON にできること"),
            json!({"character": "aoi", "attributes": []})
        );
    }

    /// 修復の一覧の形（ADR-0022 / 作業単位 14b）。
    ///
    /// 固定するのは3つ:
    ///
    /// * **`subject` を必ず載せる**（絶対制約）。`user` と `character` の両方を並べるので、
    ///   落とす変異も片方に固定する変異もここで崩れる。
    /// * **`locked` を載せない**（ADR-0022 §4）。**`locked = true` の行を入れてある**
    ///   ので、応答に足す変異は「true が漏れる」形で必ず現れる
    ///   （全部 false のフィクスチャでは、既定値と区別が付かない）。
    /// * **壊れた行を落とさない。** `unreadable` に id と列名だけが出る（格納値は出ない）。
    ///
    /// # フィクスチャに同値を作らない
    ///
    /// `id` と `character_id` を違える（取り違えると赤）。分類は有り・無しの両方。
    /// 本文・分類・主語のどれも互いに同じ文字列にしていない。
    #[test]
    fn a_memory_listing_keeps_the_subject_and_drops_the_locked_flag() {
        let created = Utc
            .with_ymd_and_hms(2026, 8, 22, 4, 5, 6)
            .single()
            .expect("実在する日時であること");
        let character_id = crate::domain::CharacterId::from_raw_for_tests(3);
        let memory = |id: i64, subject: Subject, content: &str, category: Option<&str>, locked| {
            MemoryEntry::Readable(Memory {
                id: crate::domain::MemoryId::from_raw_for_tests(id),
                character_id,
                subject,
                content: MemoryContent::parse(content).expect("正当な本文"),
                category: category.map(|raw| MemoryCategory::parse(raw).expect("正当な分類")),
                created_at: created,
                locked,
            })
        };

        let entries = vec![
            memory(11, Subject::User, "猫を飼っている", Some("好み"), false),
            memory(12, Subject::Character, "図書館で働いている", None, false),
            // **保護印の付いた健全な行。** `locked` を応答に足すとここで漏れる。
            memory(13, Subject::User, "コーヒーが苦手", Some("食事"), true),
            MemoryEntry::Unreadable(UnreadableMemory {
                id: crate::domain::MemoryId::from_raw_for_tests(14),
                character_id,
                invalid_columns: vec![MemoryColumn::Content, MemoryColumn::Locked],
            }),
        ];

        let mut memories = Vec::new();
        let mut unreadable = Vec::new();
        for entry in entries {
            match entry {
                MemoryEntry::Readable(readable) => memories.push(MemoryView::from(readable)),
                MemoryEntry::Unreadable(broken) => {
                    unreadable.push(UnreadableMemoryView::from(broken));
                }
            }
        }
        let view = MemoryListView {
            character: "aoi".to_owned(),
            memories,
            unreadable,
            // **訊かれていないので節ごと無い**（作業単位 D-3b）。
            superseded: None,
        };

        assert_eq!(
            serde_json::to_value(view).expect("JSON にできること"),
            json!({
                "character": "aoi",
                "memories": [
                    {
                        "id": 11,
                        "subject": "user",
                        "content": "猫を飼っている",
                        "category": "好み",
                        "created_at": "2026-08-22T04:05:06Z",
                    },
                    {
                        "id": 12,
                        "subject": "character",
                        "content": "図書館で働いている",
                        "category": null,
                        "created_at": "2026-08-22T04:05:06Z",
                    },
                    {
                        "id": 13,
                        "subject": "user",
                        "content": "コーヒーが苦手",
                        "category": "食事",
                        "created_at": "2026-08-22T04:05:06Z",
                    },
                ],
                "unreadable": [{"id": 14, "invalid_columns": ["content", "locked"]}],
            }),
            "subject を載せ、locked を載せず、壊れた行を id と列名で指すこと"
        );
    }

    /// 記憶が1件も無くても `memories` / `unreadable` は配列（`null` にしない）。
    #[test]
    fn an_empty_memory_listing_still_has_both_arrays() {
        let view = MemoryListView {
            character: "aoi".to_owned(),
            memories: Vec::new(),
            unreadable: Vec::new(),
            superseded: None,
        };
        assert_eq!(
            serde_json::to_value(view).expect("JSON にできること"),
            json!({"character": "aoi", "memories": [], "unreadable": []})
        );
    }

    /// **上書き済みの節は、訊かれたときだけ現れる**（作業単位 D-3b）。
    ///
    /// 空の節（`[]` / `0`）と、節そのものの不在を**同じ型で**見る。
    /// 「空なら落とす」形にすると、**訊いたのに1件も無い**ときに
    /// 訊いていないときと同じ応答になり、画面が「これで全部」と嘘をつく。
    ///
    /// **`limit` は件数に関係なく名乗る**（切ったときだけ載せる形にしない）。
    #[test]
    fn the_superseded_section_appears_only_when_it_was_asked_for() {
        let asked = MemoryListView {
            character: "aoi".to_owned(),
            memories: Vec::new(),
            unreadable: Vec::new(),
            superseded: Some(superseded_list_view(Vec::new(), 0)),
        };
        assert_eq!(
            serde_json::to_value(asked).expect("JSON にできること"),
            json!({
                "character": "aoi",
                "memories": [],
                "unreadable": [],
                "superseded": {
                    "memories": [],
                    "unreadable": [],
                    "total": 0,
                    "limit": RECENT_SUPERSEDED_MEMORIES,
                },
            }),
            "空でも節ごと返し、上限を名乗ること"
        );

        let not_asked = MemoryListView {
            character: "aoi".to_owned(),
            memories: Vec::new(),
            unreadable: Vec::new(),
            superseded: None,
        };
        let json = serde_json::to_value(not_asked).expect("JSON にできること");
        assert!(
            json.get("superseded").is_none(),
            "訊いていなければキーごと無いこと: {json}"
        );
    }

    /// **総数は「返した件数」ではない**（作業単位 D-3b）。
    ///
    /// 節の中の2つの数が**別の場所から来ている**ことを、
    /// **件数と総数がずれた状態**で固定する。
    /// 実挙動の権威は `tests/server_admin_memories.rs::
    /// only_the_most_recent_superseded_memories_come_back_but_the_total_counts_them_all`
    /// （上限を超える行を仕込む）。ここは組み立てが総数を素通しにしていることだけを見る。
    #[test]
    fn the_total_is_not_the_number_of_rows_that_came_back() {
        let character_id = CharacterId::from_raw_for_tests(7);
        let listed = vec![SupersededMemory {
            entry: MemoryEntry::Readable(Memory {
                id: MemoryId::from_raw_for_tests(11),
                character_id,
                subject: Subject::User,
                content: MemoryContent::parse("犬を飼っている").expect("正当な本文であること"),
                category: None,
                created_at: "2026-08-27T04:05:06Z"
                    .parse::<DateTime<Utc>>()
                    .expect("固定の日時が読めること"),
                locked: false,
            }),
            superseded_by: MemoryId::from_raw_for_tests(12),
        }];

        let view = superseded_list_view(listed, 42);
        let json = serde_json::to_value(view).expect("JSON にできること");
        assert_eq!(json.get("total"), Some(&json!(42)), "総数を素通しすること");
        assert_eq!(
            json.get("memories").and_then(Value::as_array).map(Vec::len),
            Some(1),
            "返した件数は別の数であること: {json}"
        );
        assert_eq!(
            json.pointer("/memories/0/superseded_by"),
            Some(&json!(12)),
            "後継はその行のものであること（id に取り違えない）: {json}"
        );
        assert_eq!(
            json.pointer("/memories/0/id"),
            Some(&json!(11)),
            "行の id を後継にすり替えないこと: {json}"
        );
    }

    /// 訂正の応答は **id を2つ**返す（ADR-0022 / 作業単位 14b）。
    ///
    /// **`superseded_id` を落とさないこと。** 応答だけを記録している運用者にとって、
    /// どの行が無効になったかを後から辿れる唯一の手掛かりになる。
    /// **2つを同値にしない**——取り違える実装が緑を通る。
    #[test]
    fn a_correction_view_names_both_rows() {
        let view = CorrectionView {
            character: "aoi".to_owned(),
            superseded_id: 4,
            correction_id: 9,
        };
        assert_eq!(
            serde_json::to_value(view).expect("JSON にできること"),
            json!({"character": "aoi", "superseded_id": 4, "correction_id": 9})
        );
    }

    /// **訂正の拒否3種は、3つとも別の応答**（ADR-0022 / 14a の申し送り）。
    ///
    /// `_ => internal(...)` に任せると**3種とも 500** になり、
    /// `store` が型を分けた意味が消える。500 は「サーバの障害」なので、
    /// 運用者は送り直すか諦めるかしかできない——実際にはどれも**直せる拒否**である。
    ///
    /// **`assert_ne!` を総当たりで置く。** 2つを同じ応答に潰す変異は、
    /// 「どれか1組が同値になる」形でしか現れない。
    #[test]
    fn each_refusal_of_a_correction_is_a_different_response() {
        let unknown = classify_correction(&StoreError::UnknownMemory);
        let wrong_owner = classify_correction(&StoreError::MemoryBelongsToAnotherCharacter);
        let superseded = classify_correction(&StoreError::MemoryAlreadySuperseded);

        assert_eq!(unknown, ApiError::memory_not_found());
        assert_eq!(wrong_owner, ApiError::memory_belongs_to_another_character());
        assert_eq!(superseded, ApiError::memory_already_superseded());

        for (left, right) in [
            (unknown, wrong_owner),
            (unknown, superseded),
            (wrong_owner, superseded),
        ] {
            assert_ne!(left, right, "3種を同じ応答に潰さないこと");
        }

        // **500 に落とさない**（どれも直せる拒否）。
        for refusal in [unknown, wrong_owner, superseded] {
            assert_ne!(refusal, ApiError::internal(), "内部エラーに潰さないこと");
            assert_ne!(
                refusal.status(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "状態番号も 500 にしないこと"
            );
        }

        // **状態番号の割り当て。** 前2つは「このキャラクターの記憶としては指せない」ので
        // 404、3つ目は「行はあるが状態が衝突している」ので 409。
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        assert_eq!(wrong_owner.status(), StatusCode::NOT_FOUND);
        assert_eq!(superseded.status(), StatusCode::CONFLICT);

        // **想定外の失敗は 500 のまま**（拒否の3種に紛れ込ませない）。
        assert_eq!(
            classify_correction(&StoreError::UnknownCharacter),
            ApiError::internal()
        );
    }

    /// **キャラクターの拒否と、記憶の拒否を混ぜない。**
    ///
    /// `character_not_found` と `memory_not_found` はどちらも 404 だが、
    /// **直す相手が違う**（パスの識別子 / パスの id）。
    #[test]
    fn a_missing_memory_is_not_a_missing_character() {
        assert_ne!(
            ApiError::memory_not_found(),
            ApiError::character_not_found(),
            "同じ 404 でも理由を分けること"
        );
    }

    /// **件数の上限は `AttributeSet::parse` が見る**（`store` に届く前）。
    ///
    /// 8 件ちょうどは通り、9 件は `too_many_attributes`。
    /// **`invalid_body` にも `duplicate_attribute` にも潰さない。**
    #[test]
    fn the_count_limit_is_checked_before_the_store() {
        let request = |count: usize| ReplaceAttributesRequest {
            attributes: (0..count)
                .map(|index| AttributeRequest {
                    subject: "character".to_owned(),
                    key: format!("キー{index}"),
                    value: format!("値{index}"),
                })
                .collect(),
        };

        assert_eq!(
            attribute_set(request(crate::domain::AttributeSet::MAX_LEN))
                .map(|set| set.len())
                .ok(),
            Some(crate::domain::AttributeSet::MAX_LEN),
            "上限ちょうどは通ること"
        );
        assert_eq!(
            attribute_set(request(crate::domain::AttributeSet::MAX_LEN + 1)),
            Err(ApiError::too_many_attributes(
                crate::domain::AttributeSetError::TooMany
            )),
            "1件超えたら件数の理由で拒否すること"
        );
    }

    /// **1件分の検証は主語・キー・値のそれぞれの理由で返る。**
    ///
    /// キーと値を同じ応答に潰す実装は、ここで崩れる。
    #[test]
    fn each_field_of_an_attribute_is_refused_for_its_own_reason() {
        let one = |subject: &str, key: &str, value: &str| {
            attribute_set(ReplaceAttributesRequest {
                attributes: vec![AttributeRequest {
                    subject: subject.to_owned(),
                    key: key.to_owned(),
                    value: value.to_owned(),
                }],
            })
        };

        assert_eq!(
            one("assistant", "一人称", "僕"),
            Err(ApiError::invalid_attribute_subject())
        );
        assert_eq!(
            one("character", "\u{3000}", "僕"),
            Err(ApiError::invalid_attribute_key(
                crate::domain::AttributeKeyError::Empty
            ))
        );
        assert_eq!(
            one("character", "一人称", "\u{3000}"),
            Err(ApiError::invalid_attribute_value(
                crate::domain::AttributeValueError::Empty
            ))
        );
        // **同じ長さでも、キーとしては長すぎ・値としては通る**（上限が別）。
        let between = "あ".repeat(AttributeKey::MAX_CHARS + 1);
        assert_eq!(
            one("character", &between, "僕"),
            Err(ApiError::invalid_attribute_key(
                crate::domain::AttributeKeyError::TooLong
            ))
        );
        assert!(
            one("character", "一人称", &between).is_ok(),
            "キーの上限を超える長さでも値としては通ること"
        );
        // 正規化される。
        let set = one("character", " 一人称 ", "\u{3000}僕\u{3000}").expect("通ること");
        assert_eq!(set.as_slice()[0].key.as_str(), "一人称");
        assert_eq!(set.as_slice()[0].value.as_str(), "僕");
    }

    /// **HTTP 上の向きの綴りは `up` / `down` のちょうど2つ。**
    ///
    /// **往復だけでは足りない**（2つを入れ替えても往復は成立する）ので、
    /// **綴りそのもの**を直書きで固定する（`subject_uses_the_wire_spelling` と同じ形）。
    #[test]
    fn the_wire_spelling_of_a_direction_is_fixed() {
        assert_eq!(delta_to_wire(DriftDelta::Up), "up");
        assert_eq!(delta_to_wire(DriftDelta::Down), "down");
        assert_ne!(
            delta_to_wire(DriftDelta::Up),
            delta_to_wire(DriftDelta::Down),
            "2つの向きが同じ綴りになっている"
        );

        // **いまは集計キーと揃えてある。** 揃っているだけで、同じ定数ではない
        // （`delta_to_wire` の doc）。ずれたらここで気付けるように見ておく。
        for delta in DriftDelta::ALL {
            assert_eq!(
                delta_to_wire(delta),
                delta.label(),
                "API の語彙と集計キーがずれました（意図した変更ならこの assert を外すこと）"
            );
        }
    }

    /// **累積の行が無い軸も、票が1件も無い軸も、値 0 の軸も落とさない。**
    ///
    /// # 落ちると何が起きるか
    ///
    /// * **値 0 の軸** —— 落とすのは `prompt` 層の責務（`get_stats` と同じ立場）。
    ///   ここで落とすと、**「軸を消したのか、値が 0 なのか」が画面から区別できない。**
    /// * **累積の行が無い軸** —— 一度も採点されていない軸は行そのものが無い。
    ///   内部結合にすると**新しいキャラクターの一覧が丸ごと空になる。**
    ///
    /// # 同値を作らない
    ///
    /// 尺度（7）・値（0 と 5）・累積（-1.25 と 0.0）・総数（3）を**全部違う数**にしてある。
    /// 累積は**2番目の軸にだけ**付けてあるので、1番目に付ける実装は赤になる。
    ///
    /// # 総数は「返した件数」ではない
    ///
    /// 直近の一覧が空でも `total_votes` は 3 のまま。
    /// **すり替える実装はここで 0 になる。**
    #[test]
    fn a_drift_view_fills_in_the_axes_that_have_no_stored_rows() {
        // **既定（10）と違う値。** 既定にすると、尺度を捨てる実装と区別が付かない。
        let scale = StatScale::parse(7).expect("正当な尺度であること");
        let view = drift_view(
            &handle("aoi"),
            scale,
            vec![
                StatAlloc {
                    axis: axis("パッション"),
                    value: scale.value(0).expect("0 は正当な値であること"),
                },
                StatAlloc {
                    axis: axis("ロジック"),
                    value: scale.value(5).expect("尺度に収まること"),
                },
            ],
            // **1番目の軸には累積が無い**（行が無い軸を 0 で埋める枝を通す）。
            &[charge("ロジック", -1.25, LOGIC_AT)],
            &[],
            3,
            Vec::new(),
            // **既定（`off`）にしない。** 既定で回すと、走らせ方を捨てる実装と
            // 区別が付かない（CLAUDE.md / `B-7` の実測）。
            None,
            StatDriftMode::Shadow,
        );

        assert_eq!(
            serde_json::to_value(view).expect("JSON にできること"),
            json!({
                "character": "aoi",
                "stat_scale": 7,
                "axes": [
                    // **累積の行が無い軸は、時刻も `null`**（0 でも「いま」でもない）。
                    {"axis": "パッション", "value": 0, "charge": 0.0,
                     "charge_updated_at": null, "ups": 0, "downs": 0},
                    {"axis": "ロジック", "value": 5, "charge": -1.25,
                     "charge_updated_at": LOGIC_AT, "ups": 0, "downs": 0},
                ],
                "total_votes": 3,
                "recent_limit": 50,
                "recent_votes": [],
                "run_mode": "shadow",
                "run_mode_source": "default",
                "default_run_mode": "shadow",
            })
        );
    }

    /// **保存済みの軸が1つも無くても、累積の残っている軸は指し示す**（作業単位 B-16）。
    ///
    /// > **⚠ この見張り番は 2026-08-28 に主張が反転した。**
    /// > 以前は `a_drift_view_does_not_invent_axes_from_the_stored_charges` という名前で、
    /// > **「累積の側から軸を生やさない」**（＝ `axes` が空になること）を固定していた。
    /// > **司令塔の決定で、存在する行は落とさずに出す形にした。**
    ///
    /// # なぜ「生やさない」ではいけなかったのか
    ///
    /// `PUT .../stats` は**全置換**なので、軸を1つ改名すると古い綴りの
    /// 累積の表と票の表の行が置き去りになる
    /// （どちらも `stat_alloc` への外部キーを持たない / D-4 の実測）。
    /// **落とすと、その票は `total_votes` にだけ残って行のどこにも現れない。**
    ///
    /// # 値は `null`。0 で埋めない
    ///
    /// 埋めると**値 0 の軸**（人が「この軸は語らない」と宣言した軸 / ADR-0024 §4）と
    /// 区別が付かなくなる。2つが同居する形の権威は
    /// `tests/server_admin_drift.rs::
    /// a_zero_valued_axis_is_never_confused_with_an_axis_that_has_no_allocation`。
    #[test]
    fn a_drift_view_points_at_the_axes_that_only_the_charges_remember() {
        let scale = StatScale::parse(7).expect("正当な尺度であること");
        let view = drift_view(
            &handle("aoi"),
            scale,
            Vec::new(),
            &[charge("むかしの軸", 2.5, RETIRED_AT)],
            &[],
            1,
            Vec::new(),
            // **キャラクターの値が既定と違う**（ADR-0045）。
            Some(StatDriftMode::On),
            StatDriftMode::Off,
        );

        assert_eq!(
            serde_json::to_value(view).expect("JSON にできること"),
            json!({
                "character": "aoi",
                "stat_scale": 7,
                "axes": [
                    // **割り当ては無くても、いつ最後に動いたかは残っている。**
                    {"axis": "むかしの軸", "value": null, "charge": 2.5,
                     "charge_updated_at": RETIRED_AT, "ups": 0, "downs": 0},
                ],
                "total_votes": 1,
                "recent_limit": 50,
                "recent_votes": [],
                "run_mode": "on",
                "run_mode_source": "character",
                "default_run_mode": "off",
            })
        );
    }

    /// **割り当ての無い軸は、届いた順ではなく軸名の昇順に並ぶ**（作業単位 B-16）。
    ///
    /// # 純粋関数の側でしか作れない状態
    ///
    /// DB から来る2つの読みは、実際には**どちらも軸名の昇順**で返る
    /// （累積は主キーの索引を辿り、集計は `GROUP BY axis` の一時 b-tree を通る）。
    /// **つまり統合テストの側では「並べていない実装」が偶然そろいうる。**
    /// ここは入力を**わざと崩した順**で渡せるので、並べ替えの有無が直接見える。
    ///
    /// # 3つの主張
    ///
    /// 1. **割り当てのある軸が先**で、`stat_alloc` の保存順のまま
    ///    （`ロジック` → `パッション` は名前の昇順とは**逆**）。
    /// 2. 割り当ての無い軸は**軸名の昇順**（`きえた` → `むかしの` → `わすれた`）。
    /// 3. **累積と集計の両方にある軸は1行だけ**（`わすれた` を二重に出さない）。
    #[test]
    fn the_axes_without_an_allocation_are_sorted_by_name_however_they_arrive() {
        let scale = StatScale::parse(7).expect("正当な尺度であること");
        let view = drift_view(
            &handle("aoi"),
            scale,
            vec![
                StatAlloc {
                    axis: axis("ロジック"),
                    value: scale.value(2).expect("尺度に収まること"),
                },
                StatAlloc {
                    axis: axis("パッション"),
                    value: scale.value(4).expect("尺度に収まること"),
                },
            ],
            // **わざと降順で渡す。時刻も軸ごとに全部違う値**（作業単位 D-5a）。
            //
            // **届いた順（`わすれた` / `きえた` / `ロジック`）と、
            // 出ていく順（`ロジック` / … / `きえた` / … / `わすれた`）が食い違う**ので、
            // **添字で引き当てる実装はここで赤になる。**
            // **押さえるのは真ん中**（`きえた軸`）——2件だと「常に先頭」「常に末尾」の
            // 片方が通り抜ける（`D-2` の実測）。
            &[
                charge("わすれた軸", -2.75, FORGOTTEN_AT),
                charge("きえた軸", 1.5, LOST_AT),
                charge("ロジック", 0.5, LOGIC_AT),
            ],
            &[
                tally(axis("むかしの軸"), 0, 3),
                tally(axis("わすれた軸"), 5, 1),
            ],
            9,
            Vec::new(),
            None,
            StatDriftMode::On,
        );

        let value = serde_json::to_value(view).expect("JSON にできること");
        assert_eq!(
            value.get("axes"),
            Some(&json!([
                {"axis": "ロジック", "value": 2, "charge": 0.5,
                 "charge_updated_at": LOGIC_AT, "ups": 0, "downs": 0},
                // **累積の行が無い**（`0.0` だが時刻は `null`）。
                {"axis": "パッション", "value": 4, "charge": 0.0,
                 "charge_updated_at": null, "ups": 0, "downs": 0},
                {"axis": "きえた軸", "value": null, "charge": 1.5,
                 "charge_updated_at": LOST_AT, "ups": 0, "downs": 0},
                // **票だけがある軸。** 累積は一度も書かれていないので時刻も `null`
                // ——**`charge: 0.0` と並べても「0 に戻った」と読めない。**
                {"axis": "むかしの軸", "value": null, "charge": 0.0,
                 "charge_updated_at": null, "ups": 0, "downs": 3},
                {"axis": "わすれた軸", "value": null, "charge": -2.75,
                 "charge_updated_at": FORGOTTEN_AT, "ups": 5, "downs": 1},
            ])),
            "割り当ての有無で2段に分かれ、後段は軸名の昇順であること: {value}"
        );
    }

    /// **走らせ方の3つの綴りが、そのまま応答に出る**（作業単位 D-5a）。
    ///
    /// # 綴りを直書きする（**関数の外にある値**）
    ///
    /// 期待値を `mode.label()` から作ると、**綴りを入れ替える変異が
    /// 期待値も一緒に入れ替えて緑のまま**になる
    /// （CLAUDE.md「写しを1つにしても、その1つは自動的には守られない」）。
    ///
    /// # 3値を1本で回す
    ///
    /// **既定（`off`）に決め打ちする変異も、`shadow` と `on` を入れ替える変異も、
    /// ここ1本で赤になる。** 片方だけの腕を置くと、
    /// 「その値へ決め打ちする実装」が通り抜ける。
    ///
    /// **これは純粋関数の側の速報である。**
    /// **`AppState` から `drift_view` まで実際に届いていること**の権威は
    /// `tests/server_admin_drift.rs::
    /// the_response_names_the_run_mode_the_server_was_started_with`
    /// （**本番の入口 `App::bind` を通す**）。
    #[test]
    fn the_view_names_the_run_mode_it_was_given() {
        let scale = StatScale::parse(7).expect("正当な尺度であること");
        let named: Vec<String> = [StatDriftMode::Off, StatDriftMode::Shadow, StatDriftMode::On]
            .into_iter()
            .map(|mode| {
                let view = drift_view(
                    &handle("aoi"),
                    scale,
                    Vec::new(),
                    &[],
                    &[],
                    0,
                    Vec::new(),
                    None,
                    mode,
                );
                serde_json::to_value(view).expect("JSON にできること")["run_mode"]
                    .as_str()
                    .expect("走らせ方は文字列であること")
                    .to_owned()
            })
            .collect();

        assert_eq!(named, vec!["off", "shadow", "on"]);
    }

    /// **効いている走らせ方・出どころ・起動時の既定の3つが、組として出る**
    /// （ADR-0045 §決定4 / 作業単位 A-8）。
    ///
    /// # 腕の選び方
    ///
    /// * **既定と違う値で回る腕**（既定 off × キャラクター on / 既定 on × キャラクター off）
    ///   ——既定を `run_mode` に書く実装と、キャラクターの値を既定欄に書く実装の両方を止める。
    /// * **キャラクターの値と既定が一致する腕**——出どころを「値が一致するか」で決める実装を止める。
    /// * **未設定の腕は既定を off 以外にする**——「常に off」と区別する。
    ///
    /// **期待値は綴りの直書き。**
    #[test]
    fn the_view_names_where_the_run_mode_came_from_and_the_default() {
        let scale = StatScale::parse(7).expect("正当な尺度であること");
        let cases = [
            (
                Some(StatDriftMode::On),
                StatDriftMode::Off,
                ["on", "character", "off"],
            ),
            (
                Some(StatDriftMode::Off),
                StatDriftMode::On,
                ["off", "character", "on"],
            ),
            (
                Some(StatDriftMode::Shadow),
                StatDriftMode::Shadow,
                ["shadow", "character", "shadow"],
            ),
            (None, StatDriftMode::On, ["on", "default", "on"]),
        ];
        for (chosen, default, expected) in cases {
            let view = drift_view(
                &handle("aoi"),
                scale,
                Vec::new(),
                &[],
                &[],
                0,
                Vec::new(),
                chosen,
                default,
            );
            let value = serde_json::to_value(view).expect("JSON にできること");
            assert_eq!(
                [
                    value["run_mode"].clone(),
                    value["run_mode_source"].clone(),
                    value["default_run_mode"].clone(),
                ],
                expected.map(|spelled| json!(spelled)),
                "キャラクター = {chosen:?} / 既定 = {default:?}"
            );
        }
    }

    /// テストで植える累積の時刻。**軸ごとに全部違う値。**
    ///
    /// **綴りは RFC 3339。** `chrono` が返す形（`Z` 付き・ミリ秒）に揃えてあるので、
    /// JSON の直書きの期待値とそのまま突き合わせられる。
    const LOGIC_AT: &str = "2026-08-26T01:02:03.400Z";
    const RETIRED_AT: &str = "2024-05-04T03:02:01.100Z";
    const LOST_AT: &str = "2025-12-31T23:59:59.999Z";
    const FORGOTTEN_AT: &str = "2023-03-03T03:03:03.030Z";

    /// テストで使う累積（`StoredDriftCharge` の構築経路は `store` の中にしか無い）。
    fn charge(name: &str, value: f64, at: &str) -> StoredDriftCharge {
        StoredDriftCharge::from_parts_for_tests(
            axis(name),
            value,
            DateTime::parse_from_rfc3339(at)
                .expect("テストで使う時刻は RFC 3339 であること")
                .with_timezone(&Utc),
        )
    }

    /// テストで使う集計（`DriftVoteTally` の構築経路は `store` の中にしか無い）。
    ///
    /// **上げと下げを別の数にすること**——同値にすると入れ替えが観測できない。
    fn tally(axis: StatAxis, ups: u32, downs: u32) -> DriftVoteTally {
        assert_ne!(ups, downs, "上げと下げを同値にしないこと");
        DriftVoteTally::from_counts_for_tests(axis, ups, downs)
    }

    fn page_query(before: Option<i64>, limit: Option<i64>) -> MessagePageQuery {
        MessagePageQuery { before, limit }
    }

    /// 会話のページの指定（作業単位 D-12c-1）。**境目の両側を1つずつ。**
    ///
    /// 期待値は**直書き**（50 / 200）。定数から作ると、定数を動かす変異が緑を通る。
    #[test]
    fn message_page_accepts_only_the_bounded_range() {
        // 省略は最新から・既定の件数。
        assert_eq!(
            message_page_of(&page_query(None, None)).expect("通ること"),
            (None, 50)
        );
        // カーソルは 1 から。
        let (before, limit) = message_page_of(&page_query(Some(1), Some(1))).expect("通ること");
        assert_eq!(before.map(MessageId::get), Some(1));
        assert_eq!(limit, 1);
        let (before, limit) =
            message_page_of(&page_query(Some(i64::MAX), Some(200))).expect("通ること");
        assert_eq!(before.map(MessageId::get), Some(i64::MAX));
        assert_eq!(limit, 200);

        let out_of_range = ApiError::message_page_out_of_range();
        for (before, limit) in [
            (Some(0), None),
            (Some(-1), None),
            (Some(i64::MIN), None),
            (None, Some(0)),
            (None, Some(-1)),
            (None, Some(201)),
            (None, Some(i64::from(u32::MAX) + 1)),
            (None, Some(i64::MAX)),
        ] {
            assert_eq!(
                message_page_of(&page_query(before, limit)),
                Err(out_of_range),
                "before={before:?} limit={limit:?}"
            );
        }
    }

    /// 役割の綴りは画面の写し（`api.ts` の `MessageRole`）が読むもの。**綴りそのものを固定する**
    /// （2値を取り違えても往復は成り立つので、往復では足りない）。
    #[test]
    fn role_uses_the_spelling_the_screen_reads() {
        assert_eq!(role_to_wire(Role::User), "user");
        assert_eq!(role_to_wire(Role::Assistant), "assistant");
    }
}

/// 「キャラクターを直す入口は存在しない」と書いていないことの走査（作業単位 20b）。
///
/// # なぜ走査なのか
///
/// 0012a で `PATCH` / `DELETE` が入った時点で、コードの各所に散っていた
/// 「アプリ経由では直せない」という**重さの見立てが一斉に事実でなくなった。**
/// それでも **`src` に 12 箇所・`tests` に 5 箇所**が残っていた
/// （0012a から 20b まで気付かれずにいた）。
/// **doc はテストで守れない**ので、`store::memory` の
/// `no_statement_updates_or_deletes_a_memory` と同じ作法——
/// **実ファイルを読んで、書いてはいけない主張を落とす。**
///
/// # **どこまでを走査で守り、どこからを人が見るのか**
///
/// **走査が守るのは「キャラクター側の資源について、直す入口が無いと言う主張」だけ。**
/// 判定は次の3つの**連言**で、どれか1つでも欠ければ違反にしない。
///
/// | | 見るもの | なぜ要るか |
/// |---|---|---|
/// | 1 | 「〜が無い／未実装」の綴り（`absence_phrases`） | 主張そのもの |
/// | 2 | その直前 `LOOK_BEHIND` 文字に「更新」か「削除」 | 何の入口の話か |
/// | 3 | **同じコメント塊**に資源の名前（`character_resources`） | **どの資源の話か** |
///
/// 3 が要る理由は実測にある。**2 までだと `tests/store_drift_vote.rs` が誤爆した**
/// ——票の側は**追記のみで、直す入口が本当に無い**（ADR-0026 §決定3）。
///
/// > **⚠ その module doc の一文をここへ引き写さないこと。** 引き写した時点で
/// > **この塊には「キャラクター」が居るので、3 の条件を満たして自分自身が違反に上がる**
/// > （20b で実際に踏んだ。走査は自分のファイルも読む）。
///
/// **除外の一覧を足すのではなく、主語をキャラクター側に寄せて狭めてある**
/// （`the_scan_leaves_the_statements_that_are_still_true` の doc）。
/// 同じ理由で `tests/store_schema.rs` の**セッション削除**の記述も掛からない。
///
/// # **`memory` は走査の対象外。人が見る**（**20b のレビューで判定をやり直した**）
///
/// **`memory` にも直す入口はある**（ADR-0022 / 14a・14b）——
/// `POST /admin/characters/{handle}/memories/{memory_id}/corrections` と
/// [`crate::store::Store::supersede_memory`]。行は DELETE も UPDATE もしないが、
/// **想起は `superseded_by IS NULL` しか読まない**ので、
/// 訂正版を入れて古い行に指させれば**その瞬間に読み出し経路から外れる。**
/// 「行を潰さない」（絶対制約）と「直せない」は**別のこと**である。
///
/// **それでも走査では守らない。** `memory` 側の正しさは**条件つき**で、
/// 綴りからは区別できないため:
///
/// * **中身・主語（`user` / `character`）の誤りは直せる** ——
///   訂正版で上書きすれば想起から外れる。
/// * **持ち主（`character_id`）の取り違えは直しきれない** ——
///   `supersede_memory` は**同じ持ち主の下にしか**訂正版を入れないので、
///   想起から外すことはできても、**正しい持ち主へ移す手段は無い。**
///
/// **この2つを綴りで割る手段は無い。** 一括りに「直せない」と書かないこと、
/// どちらなのかを**人が読んで書き分けること**が、ここでの唯一の守り方である。
#[cfg(test)]
mod stale_api_claim_scan {
    /// 走査対象の根。
    ///
    /// **`tests` を外さないこと。** ロードマップの項目は「まだコードに残っている」で、
    /// **`tests/` はコードである。** 実測で、`src` だけに絞ると
    /// `tests/server_admin.rs` の 3 箇所（同じ主張の HTTP 側の写し）と
    /// `tests/store_schema.rs` の 2 箇所が**そのまま残っていた。**
    const SCAN_ROOTS: [&str; 2] = ["src", "tests"];

    /// 「〜の入口が存在しない」と言っている綴り。
    ///
    /// **連続した形でソースに書かない**（書くとこの走査が自分自身を落とす。
    /// `store::memory` の `table_word` と同じ理由）。
    fn absence_phrases() -> Vec<String> {
        ["が無い", "も無い", "は無い", "が未実装"]
            .iter()
            .map(|tail| ["API", tail].concat())
            .collect()
    }

    /// 直前にあると「何かを直す入口」の話になる語。
    fn subject_words() -> [&'static str; 2] {
        ["更新", "削除"]
    }

    /// **キャラクター側の資源の名前。** 同じコメント塊にこれが無ければ違反にしない。
    ///
    /// ここに `記憶` / `票` / `セッション` を足さないこと。
    /// **それらは「直す入口」の事情が別**（モジュール doc の
    /// 「どこまでを走査で守るのか」）。
    fn character_resources() -> [&'static str; 7] {
        [
            "キャラクター",
            "表示名",
            "呼び名",
            "ペルソナ",
            "形式指示",
            "属性",
            "性格ステータス",
        ]
    }

    /// 主語を探しにいく幅（正規化後の文字数）。
    ///
    /// **行をまたいで探す必要がある。** 実在した記述の1つ
    /// （`domain::character` の `persona_rejects_a_pasted_document`）は
    /// **主語だけを行末に残して折り返し**、次の行が綴りの側から始まっていた。
    /// **1行ずつ見る走査では、この形が落ちない**（実測。20b の変異検査で確認）。
    ///
    /// > **この doc に、その2語を隣り合わせで書かないこと。**
    /// > 走査は自分自身のファイルも読むので、書いた瞬間にここが違反として上がる
    /// > （`absence_phrases` を分けて綴っているのと同じ理由）。
    ///
    /// **押さえてあるのは「はみ出す向き」だけ**（実測: 14〜18 の5値が緑を通る）。
    /// 狭めすぎれば `the_scan_catches_a_planted_claim` が、
    /// 広げすぎれば `the_scan_leaves_the_statements_that_are_still_true` が赤になる。
    /// **その内側のどの値でもよい**という意味であって、16 に根拠があるわけではない。
    const LOOK_BEHIND: usize = 16;

    /// 走査の単位。**連続したコメント行を1つの塊にする**
    /// （`(先頭行, 正規化後の文字, 各文字の行)`）。
    ///
    /// **塊が単位である理由は「どの資源の話か」を読むため。**
    /// 資源の名前は主張と同じ行には書かれない（実測: `tests` の 5 箇所とも別の行にあった）。
    /// コメントでない行は**1行で1塊**にする（文字列リテラルに書かれた場合の取りこぼし防止）。
    fn blocks(source: &str) -> Vec<(usize, Vec<char>, Vec<usize>)> {
        let mut blocks: Vec<(usize, Vec<char>, Vec<usize>)> = Vec::new();
        let mut in_comment_run = false;

        for (index, line) in source.lines().enumerate() {
            let is_comment = line.trim_start().starts_with("//");
            // 装飾（`/`・`*`・`!`・バッククォート・`#`）と空白を落として繋ぐ。
            let stripped: Vec<char> = line
                .chars()
                .filter(|character| {
                    !character.is_whitespace()
                        && !matches!(character, '/' | '*' | '!' | '`' | '#' | '\u{3000}')
                })
                .collect();

            if is_comment
                && in_comment_run
                && let Some(last) = blocks.last_mut()
            {
                last.2
                    .extend(std::iter::repeat_n(index + 1, stripped.len()));
                last.1.extend(stripped);
                continue;
            }
            let lines = vec![index + 1; stripped.len()];
            blocks.push((index + 1, stripped, lines));
            in_comment_run = is_comment;
        }
        blocks
    }

    /// 走査の本体。**引数でソースを受け取る**（実ファイルと、テストが植えたソースの
    /// 両方を同じ判定に通すため。実ファイルだけを読む対照は自己成就する）。
    ///
    /// 返すのは `(行番号, 抜粋)` の並び。
    fn stale_claims(source: &str) -> Vec<(usize, String)> {
        let mut found = Vec::new();

        for (_, normalized, lines) in blocks(source) {
            let text: String = normalized.iter().collect();
            // **塊のどこかに資源の名前があること**（連言の3つ目）。
            if !character_resources()
                .iter()
                .any(|resource| text.contains(resource))
            {
                continue;
            }

            // 正規化後の**文字**位置へ引き直すための索引（バイト位置 → 文字位置）。
            let mut char_of_byte = vec![0_usize; text.len() + 1];
            for (character_index, (byte_index, _)) in text.char_indices().enumerate() {
                char_of_byte[byte_index] = character_index;
            }

            for phrase in absence_phrases() {
                for (byte_index, _) in text.match_indices(&phrase) {
                    let at = char_of_byte[byte_index];
                    let from = at.saturating_sub(LOOK_BEHIND);
                    let window: String = normalized[from..at].iter().collect();
                    if !subject_words().iter().any(|word| window.contains(word)) {
                        continue;
                    }
                    let until = (at + phrase.chars().count() + 8).min(normalized.len());
                    found.push((
                        lines[at],
                        normalized[from..until].iter().collect::<String>(),
                    ));
                }
            }
        }

        found.sort_unstable();
        found
    }

    /// `SCAN_ROOTS` 配下の Rust ソースを「パス, 生ソース」で返す。
    fn rust_sources() -> Vec<(String, String)> {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        let mut stack: Vec<std::path::PathBuf> =
            SCAN_ROOTS.iter().map(|root| manifest.join(root)).collect();

        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("走査対象を読めること") {
                let path = entry.expect("ディレクトリ項目を読めること").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !path.extension().is_some_and(|extension| extension == "rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("ソースを読めること");
                files.push((path.display().to_string(), source));
            }
        }
        files
    }

    /// **入口は実在する。** ここが**コンパイルできなくなったら**、
    /// 下の走査が守っている前提そのものが消えたということなので、
    /// 各所の doc を「直せない」側へ書き戻す判断からやり直すこと。
    ///
    /// **`memory` の側の入口も一緒に押さえる**（モジュール doc の「人が見る」の節）。
    /// ここが消えたら、`server::extract` と `prompt::extraction` の
    /// 「上書きして想起から外せる」という記述が事実でなくなる。
    ///
    /// HTTP の経路を押さえているのは `tests/server_admin.rs` の
    /// **`only_patch_and_delete_are_exposed_for_a_single_character`**
    /// （`PATCH` / `DELETE` が通り、`PUT` / `POST` / `GET` が 405 になることまで見る。
    /// **`PUT` は意図的に生やしていない** / ADR-0016 §1）。
    #[test]
    fn the_character_update_and_delete_entry_points_exist() {
        let _update = crate::store::Store::update_character;
        let _delete = crate::store::Store::delete_character;
        let _handler_update = super::update_character;
        let _handler_delete = super::delete_character;
        // **`memory` は「行を潰さない」まま直せる。** 入口はこれ。
        let _supersede = crate::store::Store::supersede_memory;
        let _handler_correct = super::correct_memory;
    }

    /// **`src` と `tests` のどこにも「キャラクターを直す入口が無い」と書いていない。**
    ///
    /// # 対象を「件数」で確かめない
    ///
    /// **`SCAN_ROOTS` を `src/server` に狭める変異は、件数だけでも赤になった**
    /// （14 < 15）**が、それは偶然**。`src/domain` なら 12 で同じく赤だが、
    /// **ファイルが増えれば静かに通るようになる。**
    /// 17 箇所は `domain` / `prompt` / `server` の3モジュールと `tests` に散っていたので、
    /// **モジュールと根が揃っていること**を名指しで見る
    /// （`store::memory` の `the_scan_covers_every_module_not_just_the_store` と同じ形）。
    #[test]
    fn no_source_claims_the_character_update_api_is_missing() {
        let sources = rust_sources();
        assert!(
            sources.len() >= 15,
            "走査対象が少なすぎるなら、この assert は何も守っていない: {:?}",
            sources.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );

        let separator = std::path::MAIN_SEPARATOR;
        for module in ["domain", "prompt", "server", "store", "llm", "parse"] {
            let expected = format!("src{separator}{module}{separator}");
            assert!(
                sources.iter().any(|(path, _)| path.contains(&expected)),
                "{module} が走査対象に入っていない: {:?}",
                sources.iter().map(|(path, _)| path).collect::<Vec<_>>()
            );
        }
        // **`tests/` を落とさない。** ここが抜けると、同じ主張の HTTP 側の写しが残る。
        let tests_root = format!("{separator}tests{separator}");
        assert!(
            sources
                .iter()
                .filter(|(path, _)| path.contains(&tests_root))
                .count()
                >= 5,
            "統合テストが走査対象に入っていない: {:?}",
            sources.iter().map(|(path, _)| path).collect::<Vec<_>>()
        );

        let mut found = Vec::new();
        for (path, source) in &sources {
            for (line, excerpt) in stale_claims(source) {
                found.push(format!("{path}:{line}: …{excerpt}…"));
            }
        }

        assert!(
            found.is_empty(),
            "0012a 以降は事実でない記述があります（`PATCH` / `DELETE` があります）:\n{}",
            found.join("\n")
        );
    }

    /// **植えたソースなら落ちる**（走査そのものが生きていることの対照）。
    ///
    /// # 実ファイルを読む対照にしない
    ///
    /// 「実ファイルから1件は見つかること」を対照にすると、
    /// **綴りを壊す変異を入れたときに、その壊れた綴りを実ファイルへ書き込む形の変異が
    /// 自己成就で緑になる**（`15b` の実測）。**ここでは綴りをこのテストが直接持ち、
    /// 走査本体へ引数で渡す。** 綴りは `absence_phrases` から導出しないこと。
    #[test]
    fn the_scan_catches_a_planted_claim() {
        let planted = [
            // 1行に収まっている形。
            "/// キャラクターの更新".to_string() + " API " + "が無いので二度と直せない。\n",
            // **行をまたぐ形。** 実在した記述そのもの（`Persona` の貼り付け事故のテスト）。
            "    /// ペルソナが巨大だと以後のすべてのチャットが失敗する。更新も削除も\n"
                .to_string()
                + "    /// "
                + "API "
                + "が無いため、作成できてしまうと直せない。\n",
            // 「・」で並べた形。
            "// 表示名の更新・削除".to_string() + " API " + "は無いため、入口で止める。\n",
            // 「未実装」で言う形。
            "//! 性格ステータスの削除".to_string()
                + " API "
                + "が未実装なので、作った行は残り続ける。\n",
            // **主語と綴りが離れている形。** ここが `LOOK_BEHIND` の下端を押さえる
            // （正規化後で 14 文字ぶん離してある。これより短くすると落ちなくなる）。
            "/// 属性の更新も削除も、いまのところ管理系の".to_string()
                + " API "
                + "が無いままである。\n",
            // **資源の名前が別の行にある形。** `tests` に実在した 5 箇所はすべてこれで、
            // **主張の行には資源の名前が無い。塊で見ていないと落ちない。**
            "/// **形式指示も検証される**（空 / 空白のみ / 長すぎる）。\n".to_string()
                + "///\n"
                + "/// 以後のすべてのチャットが予算超過で失敗する。更新"
                + " API "
                + "も削除"
                + " API "
                + "も無いため、\n"
                + "/// **作成できてしまうと二度と直せない。**\n",
            // **呼び名の側。** ここが無いと `character_resources` の
            // 「呼び名」だけを落とす変異が緑を通る（`tests/store_schema.rs` の
            // バックフィルの記述がこの語で書かれている）。
            "/// 埋め損なうと、プロンプトに載る呼び名が黙って別物になる。\n".to_string()
                + "/// 更新"
                + " API "
                + "が無いので直せない。\n",
        ];

        for source in &planted {
            assert!(
                !stale_claims(source).is_empty(),
                "植えた主張を落とせていません: {source}"
            );
        }
    }

    /// **正当な記述は落とさない。**
    ///
    /// ここが赤になったら、走査が広すぎる。**除外を足すのではなく、
    /// 判定の綴りを狭めること**（除外の一覧は黙って伸びる）。
    #[test]
    fn the_scan_leaves_the_statements_that_are_still_true() {
        let allowed = [
            // 入口が在ることを述べる記述。
            "/// キャラクターの更新".to_string()
                + " API "
                + "（`update_character`）は識別子で引く。\n",
            // **票には本当に更新も削除も無い**（ADR-0026 §決定3）。
            // 実測で、資源の名前を要求する前はここが誤爆していた
            // （`tests/store_drift_vote.rs` の module doc）。
            "//! `store` に票を更新する".to_string()
                + " API "
                + "も削除する"
                + " API "
                + "も無い。\n",
            // **セッションを消す入口も無い。** 0012a で入ったのはキャラクターの削除だけ。
            "/// 生 SQL でのセッション削除。`store` にセッションを削除する".to_string()
                + " API "
                + "は無い。\n",
            // **`memory` の側は綴りが違う**（モジュール doc の「人が見る」の節）。
            [
                "/// `",
                "mem",
                "ory",
                "` の行は潰さないが、訂正版で上書きすれば想起から外れる。\n",
            ]
            .concat(),
            // 歴史として引用した形（0012a の注記）。
            "/// > **0012a より前は「アプリ経由では二度と直せない」と書いてあった。**\n"
                .to_string(),
            // **主語が遠い形。** ここが `LOOK_BEHIND` の上端を押さえる
            // （正規化後で 20 文字ぶん離してある。これより広げると誤爆する）。
            "/// キャラクターのコンテキスト長の更新は5分ごとに引き直す。\n".to_string()
                + "/// 上流の応答に"
                + " API "
                + "が無いときは既定へ落とす。\n",
        ];

        for source in &allowed {
            assert_eq!(
                stale_claims(source),
                Vec::new(),
                "正当な記述を落としています: {source}"
            );
        }
    }
}
