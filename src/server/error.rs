//! FE へ返すエラー応答。
//!
//! # 定数しか載せられない形にする
//!
//! `ApiError` が持てるメッセージは **`&'static str` だけ**。`String` を持たせないので、
//! `format!("{err:?}")` のような詳細を**型として載せられない**（絶対制約:
//! エラーレスポンスに内部パス・接続文字列・スタックトレースを含めない）。
//! 詳細は呼び出し側が `tracing` に出す。
//!
//! # 形は OpenAI 互換
//!
//! ```json
//! {"error": {"message": "…", "type": "server_error"}}
//! ```
//!
//! 既存のクライアントがそのまま FE になる（ADR-0001）ため、
//! エラーも相手が知っている形にする。
//!
//! # 理由の伝え方
//!
//! 管理 API（`/admin/`）は入力の不備を利用者に伝える必要があるが、
//! **理由を `String` で組み立てない。** `domain` のエラー型
//! （`HandleError` / `AxisError`）が理由を区別できる enum なので、
//! **バリアントごとに定数を選ぶ**ことで理由を伝える。
//! 「定数しか載せられない」性質を保ったまま、袋小路にならない粒度を出す。

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Map, Value};
use utoipa::ToSchema;

use crate::domain::{
    AgreementContentError, AttributeKeyError, AttributeSetError, AttributeValueError,
    AxisDiscoverySlotsError, AxisError, CategoryError, ContentError, DisplayNameError, HandleError,
    OutputFormatError, PersonaError, ScaleError, SelectorError, StatDriftModeError, ValueError,
};

/// エラー応答の本文。
///
/// **`pub(super)` なのは仕様書のため**（各 handler の `#[utoipa::path]` が名指す /
/// 作業単位 P-2a）。**組み立てる口はこのモジュールの外に無い**（欄は非公開）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct ErrorBody {
    error: ErrorDetail,
}

/// OpenAI 互換のエラー本体。
///
/// **`deny_unknown_fields` は仕様書のため**（作業単位 P-2a のレビュー是正）。この型は直列化しか
/// しないので `serde` の振る舞いは変わらないが、`utoipa` はこれを読んで
/// `additionalProperties: false` を書く。**この形の本文は、実際にこの3つの鍵しか持たない**
/// ——鍵を足した本文（保存の 422 / `ApiError::into_response_with`）は別の型として仕様書に載る
/// （`profile_editor::SaveRejection`）。許したままだと、2つの形を `oneOf` で区別できない。
#[derive(Debug, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct ErrorDetail {
    /// **定数文字列のみ。** 入力もエラーの詳細も反射しない。
    message: &'static str,
    /// `server_error` / `invalid_request_error` など。
    #[serde(rename = "type")]
    kind: &'static str,
    /// 細分類（`model_not_found` など）。無いときはキーごと落とす。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    code: Option<&'static str>,
}

/// FE へ返すエラー。**構築時に定数を選ぶだけ**で、動的な文字列は入らない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiError {
    status: StatusCode,
    message: &'static str,
    kind: &'static str,
    code: Option<&'static str>,
}

impl ApiError {
    /// 内部で処理を続けられなかった（DB 障害など）。**原因はログにだけ出す。**
    #[must_use]
    pub const fn internal() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "内部エラーが発生しました",
            kind: "server_error",
            code: None,
        }
    }

    /// 存在しないパス。**リクエストの内容を反射しない。**
    #[must_use]
    pub const fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "指定されたエンドポイントは存在しません",
            kind: "invalid_request_error",
            code: None,
        }
    }

    /// パスは合っているがメソッドが違う。**リクエストの内容を反射しない。**
    ///
    /// axum の既定は**空ボディの 405** なので、明示的に足さないとここだけ
    /// JSON にならない（FE がエラーを構造化して読めなくなる）。
    #[must_use]
    pub const fn method_not_allowed() -> Self {
        Self {
            status: StatusCode::METHOD_NOT_ALLOWED,
            message: "このエンドポイントは指定されたメソッドを受け付けません",
            kind: "invalid_request_error",
            code: None,
        }
    }

    /// リクエスト本文を読めない（JSON として壊れている・型が違う・未知のフィールド）。
    ///
    /// **axum の既定の `JsonRejection` をそのまま返さないこと。**
    /// 既定の本文は serde のメッセージ（`invalid type: integer \`1\`` のように
    /// **入力値を含む**）で、そのまま返すと管理 API が入力反射の窓口になる。
    #[must_use]
    pub const fn invalid_body() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "リクエスト本文の形式が不正です",
            kind: "invalid_request_error",
            code: Some("invalid_body"),
        }
    }

    /// リクエスト本文が上限を超えた。
    ///
    /// **実運用の判定を握っているのは `body::MAX_BODY_BYTES`。**
    /// axum 既定の上限（2MB）も残してあるが、`body` のミドルウェアが
    /// 先に本文を読むので、判定は必ずそちらが先に行う
    /// （両者が同値であることは `body` の
    /// `the_limit_matches_the_axum_default` が別途固定している）。
    ///
    /// **`invalid_body` に吸収しないこと。** 上限超過は本文を1バイトも解釈せずに
    /// 打ち切られた結果であり、「JSON が壊れている」と返すと原因から遠い
    /// （送った側は正しい JSON を書いているので、綴りを疑って堂々巡りになる）。
    ///
    /// **実運用でこれを返すのは `body` モジュールのミドルウェア。**
    /// 抽出器（`from_json_rejection`）まで到達した場合も同じ応答になるが、
    /// そちらは「読み捨てをせずに打ち切った」経路なので**応答が届かないことがある**
    /// （`body` モジュールの doc / docs/06-roadmap.md「0005 の範囲」）。
    #[must_use]
    pub const fn body_too_large() -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            message: "リクエスト本文が大きすぎます",
            kind: "invalid_request_error",
            code: Some("body_too_large"),
        }
    }

    /// **本文を読めなかった `JsonRejection` を応答へ写す唯一の入口。**
    ///
    /// # なぜ入口をここ1つにするか
    ///
    /// この関数の中身は整形ではなく**セキュリティ方針そのもの**で、内容は2つある:
    ///
    /// 1. `JsonRejection` の `Display`（= serde のメッセージ。**入力値を含む**）を
    ///    応答に載せない。詳細はログにだけ出す。
    /// 2. **413 だけは `invalid_body` に潰さない**（`body_too_large` の doc）。
    ///
    /// `/admin/` と `/v1/` で方針が違うのは `deny_unknown_fields` の側であって、
    /// **本文拒否の方針は同一**。入口を分けると片方だけが劣化し、しかも
    /// **応答本文を見るまで気づけない**（型は何も守らない）。
    ///
    /// 判定は `rejection` の**バリアントではなくステータス**で行う
    /// （`JsonRejection` は非網羅的で、上限超過を表す形が増えても追随する）。
    ///
    /// `what` はログに出すラベル（どちらの入口で起きたかを残す）。**応答には載らない。**
    pub(crate) fn from_json_rejection(rejection: JsonRejection, what: &'static str) -> Self {
        tracing::debug!(error = %rejection, endpoint = what, "リクエスト本文を読めませんでした");
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            Self::body_too_large()
        } else {
            Self::invalid_body()
        }
    }

    /// **問い合わせパラメータを読めない**（知らない欄・知らない値 / 作業単位 D-3b）。
    ///
    /// # 黙って無視しない
    ///
    /// 綴りを間違えた問い合わせ（`?includes=…`）を捨てると、
    /// **送った側は「求めたものが1件も無かった」と読む。**
    /// 求めたのに返らない節と、求めていないので返らない節が同じ形になり、
    /// **画面が「これで全部」と嘘をつく。**
    ///
    /// **入力を反射しない。** `Display` に載るのは定数の文言だけで、
    /// 何をどう間違えたかは**ログにだけ**出す（本文の拒否と同じ方針）。
    #[must_use]
    pub const fn invalid_query() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "問い合わせパラメータの形式が不正です",
            kind: "invalid_request_error",
            code: Some("invalid_query"),
        }
    }

    /// パスのパラメータを読めない（不正なパーセントエンコーディングなど）。
    #[must_use]
    pub const fn invalid_path() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "パスの形式が不正です",
            kind: "invalid_request_error",
            code: Some("invalid_path"),
        }
    }

    /// キャラクター**識別子**が受け付けられない。**理由ごとに別の定数を選ぶ。**
    ///
    /// **文言は「名前」ではなく「識別子」で書く**（作業単位 0004b-3 / `HandleError` の doc）。
    /// 作成要求には `handle` と `display_name` の両方が載るので、
    /// 「キャラクター名」と書くとどちらの欄が悪いのか応答から読めない。
    #[must_use]
    pub const fn invalid_character_handle(reason: HandleError) -> Self {
        let (message, code) = match reason {
            HandleError::Empty => ("キャラクター識別子が空です", "character_handle_empty"),
            HandleError::AtSign => (
                "キャラクター識別子に @ は使えません",
                "character_handle_at_sign",
            ),
            HandleError::Control => (
                "キャラクター識別子に改行や制御文字は使えません",
                "character_handle_control",
            ),
            HandleError::TooLong => (
                "キャラクター識別子が長すぎます",
                "character_handle_too_long",
            ),
            // **使える文字を応答に列挙しない**（`HandleError::Charset` の doc）。
            // 列挙すると `Handle::ALLOWED_CHARACTERS` の写しが増える。
            HandleError::Charset => (
                "キャラクター識別子に使えない文字が含まれています",
                "character_handle_charset",
            ),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// 表示名が受け付けられない。**理由ごとに別の定数を選ぶ。**
    ///
    /// **`invalid_character_handle` と同じ応答に潰さないこと。** 作成要求には
    /// `handle`（識別子）と `display_name`（呼び名）の両方が載るので、
    /// 同じ文言だとどちらの欄を直せばよいのか応答から読めない。
    #[must_use]
    pub const fn invalid_display_name(reason: DisplayNameError) -> Self {
        let (message, code) = match reason {
            DisplayNameError::Empty => ("表示名が空です", "display_name_empty"),
            DisplayNameError::Control => {
                ("表示名に改行や制御文字は使えません", "display_name_control")
            }
            DisplayNameError::TooLong => ("表示名が長すぎます", "display_name_too_long"),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// ペルソナ本文が受け付けられない。**理由ごとに別の定数を選ぶ。**
    ///
    /// 上限値そのものは載せない（`Display` を定数に保つ絶対制約）。
    /// 「長すぎる」と「空」を同じ文言に潰すと、送り直しの判断ができない。
    #[must_use]
    pub const fn invalid_persona(reason: PersonaError) -> Self {
        let (message, code) = match reason {
            PersonaError::Empty => ("ペルソナ本文が空です", "persona_empty"),
            PersonaError::TooLong => ("ペルソナ本文が長すぎます", "persona_too_long"),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// 出力形式指示が受け付けられない。**理由ごとに別の定数を選ぶ。**
    ///
    /// 上限値そのものは載せない（`Display` を定数に保つ絶対制約）。
    #[must_use]
    pub const fn invalid_output_format(reason: OutputFormatError) -> Self {
        let (message, code) = match reason {
            OutputFormatError::Empty => ("出力形式指示が空です", "output_format_empty"),
            OutputFormatError::TooLong => ("出力形式指示が長すぎます", "output_format_too_long"),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// 性格ステータスの軸名が受け付けられない。**理由ごとに別の定数を選ぶ。**
    #[must_use]
    pub const fn invalid_stat_axis(reason: AxisError) -> Self {
        let (message, code) = match reason {
            AxisError::Empty => ("性格ステータスの軸名が空です", "stat_axis_empty"),
            AxisError::Control => (
                "性格ステータスの軸名に改行や制御文字は使えません",
                "stat_axis_control",
            ),
            AxisError::TooLong => ("性格ステータスの軸名が長すぎます", "stat_axis_too_long"),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// 性格ステータスの尺度が受け付けられない。**理由ごとに別の定数を選ぶ。**
    ///
    /// 上下限そのものは載せない（`Display` を定数に保つ絶対制約）。
    /// 「小さすぎる」と「大きすぎる」を潰すと、**送り直しの向き**が応答から読めない。
    #[must_use]
    pub const fn invalid_stat_scale(reason: ScaleError) -> Self {
        let (message, code) = match reason {
            ScaleError::TooSmall => ("性格ステータスの尺度が小さすぎます", "stat_scale_too_small"),
            ScaleError::TooLarge => ("性格ステータスの尺度が大きすぎます", "stat_scale_too_large"),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// **見いだす枠の数が大きすぎる**（ADR-0046 §決定1 / 作業単位 A-9a）。
    ///
    /// **400。** 上限そのものは載せない（`Display` を定数に保つ絶対制約 / `invalid_stat_scale` と同じ）。
    /// 負の数や整数でない値はここに来ない（`u32` に読めず `invalid_body`）。
    #[must_use]
    pub const fn invalid_axis_discovery_slots(reason: AxisDiscoverySlotsError) -> Self {
        let (message, code) = match reason {
            AxisDiscoverySlotsError::TooLarge => (
                "軸を見いだす枠の数が大きすぎます",
                "axis_discovery_slots_too_large",
            ),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// **変容の走らせ方の綴りが不正**（ADR-0045 §決定3 / 作業単位 A-8）。
    ///
    /// **400。** 受けるのは `off` / `shadow` / `on` だけで、環境変数の別名
    /// （`0` / `true` / `yes` 等）もここに落ちる。**受け付ける綴りは文面に書かない**
    /// ——書くと `StatDriftMode::label` の写しになり、片方だけ古びる。
    /// 入力は反射しない（定数のみ）。
    #[must_use]
    pub const fn invalid_stat_drift_mode() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "変容の走らせ方の綴りが不正です",
            kind: "invalid_request_error",
            code: Some("stat_drift_mode_invalid"),
        }
    }

    /// 性格ステータスの値がそのキャラクターの尺度を超えている（作業単位 0011a-2）。
    ///
    /// **400。** 送られた値そのものが要求として成立していない（値を直せば通る）。
    /// **`stat_scale_below_stored_value`（409）と潰さないこと**——あちらは
    /// 「値は正しいが、いまの状態では尺度を下げられない」であって、直す対象が逆。
    ///
    /// **尺度も入力も載せない**（`Display` を定数に保つ絶対制約）。
    /// 尺度は `GET /admin/characters/{handle}/stats` が `stat_scale` として返す。
    #[must_use]
    pub const fn invalid_stat_value(reason: ValueError) -> Self {
        let (message, code) = match reason {
            ValueError::AboveScale => (
                "性格ステータスの値が尺度を超えています",
                "stat_value_above_scale",
            ),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// **保存されている値より小さい尺度へ変更しようとした**（ADR-0015 / 作業単位 0011a-2）。
    ///
    /// 409 なのは、要求そのものは正しいが**現在の状態と衝突している**ため
    /// （先例は `character_has_sessions`）。400 にすると「送り方が悪い」と読めてしまい、
    /// 実際にやるべきこと（**先に `PUT .../stats` で値を下げる**）に辿り着けない。
    ///
    /// **`stat_scale_too_small`（400）と潰さないこと。** あちらは
    /// 「尺度そのものが下限（`StatScale::MIN`）未満」で、送り直せば直る。
    /// こちらは尺度自体は正当で、**先に別の API を呼ばない限り永久に通らない。**
    #[must_use]
    pub const fn stat_scale_below_stored_value() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "保存されている性格ステータスの値より小さい尺度には変更できません",
            kind: "invalid_request_error",
            code: Some("stat_scale_below_stored_value"),
        }
    }

    /// **値を組み立ててから書くまでの間に、尺度が変わった**（作業単位 0011a-2）。
    ///
    /// `PUT .../stats` は引いた尺度で値を検証するので、`store` の突き合わせが
    /// 食い違うのは `PATCH` と競合したときだけ。**500 に潰さない**
    /// ——サーバの障害ではなく状態との衝突で、**送り直せば通る。**
    #[must_use]
    pub const fn stat_scale_changed() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "性格ステータスの尺度が変更されました。送り直してください",
            kind: "invalid_request_error",
            code: Some("stat_scale_changed"),
        }
    }

    /// 同じ軸を複数回指定した（正規化後に同じになる場合を含む）。
    #[must_use]
    pub const fn duplicate_stat_axis() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "同じ軸を重複して指定することはできません",
            kind: "invalid_request_error",
            code: Some("duplicate_stat_axis"),
        }
    }

    /// キャラクター属性のキーが受け付けられない（ADR-0009 / 作業単位 12a）。
    /// **理由ごとに別の定数を選ぶ。**
    ///
    /// **`invalid_stat_axis` と潰さないこと。** どちらも「短い1行のラベル」だが、
    /// 上限が違う（軸名 20 文字 / キー 12 文字）ので、
    /// 同じ応答にすると「軸としては通る長さがキーでは通らない」理由が伝わらない。
    #[must_use]
    pub const fn invalid_attribute_key(reason: AttributeKeyError) -> Self {
        let (message, code) = match reason {
            AttributeKeyError::Empty => ("属性のキーが空です", "attribute_key_empty"),
            AttributeKeyError::Control => (
                "属性のキーに改行や制御文字は使えません",
                "attribute_key_control",
            ),
            AttributeKeyError::TooLong => ("属性のキーが長すぎます", "attribute_key_too_long"),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// キャラクター属性の値が受け付けられない。**理由ごとに別の定数を選ぶ。**
    ///
    /// **キー側と潰さないこと。** 上限が違う（12 / 20）ので、潰すと
    /// 受け取った側が**どちらの欄を縮めればよいか分からない。**
    #[must_use]
    pub const fn invalid_attribute_value(reason: AttributeValueError) -> Self {
        let (message, code) = match reason {
            AttributeValueError::Empty => ("属性の値が空です", "attribute_value_empty"),
            AttributeValueError::Control => (
                "属性の値に改行や制御文字は使えません",
                "attribute_value_control",
            ),
            AttributeValueError::TooLong => ("属性の値が長すぎます", "attribute_value_too_long"),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// 属性の主語（`subject`）の綴りが `user` / `character` のどちらでもない。
    ///
    /// **`invalid_body` に潰さないこと。** `assistant`（`message.role` の綴り）は
    /// 現実に起きる取り違えで、「本文の形式が不正」とだけ返すと直す場所が分からない。
    ///
    /// **受理される綴りをメッセージに書くのは意図的。** 2 値しか無く、
    /// 入力を反射しているわけでもないので、定数文字列の性質は保たれる。
    #[must_use]
    pub const fn invalid_attribute_subject() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "属性の主語は user か character のどちらかです",
            kind: "invalid_request_error",
            code: Some("attribute_subject_unknown"),
        }
    }

    /// 属性の件数が `AttributeSet::MAX_LEN` を超えている（ADR-0009）。
    ///
    /// **400。** 送られた要求そのものが成立していない（減らせば通る）。
    /// **件数も上限も本文に載せない**（`Display` を定数に保つ絶対制約）。
    #[must_use]
    pub const fn too_many_attributes(reason: AttributeSetError) -> Self {
        let (message, code) = match reason {
            AttributeSetError::TooMany => ("属性の件数が多すぎます", "too_many_attributes"),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// 取り決めの中身が受け付けられない（ADR-0051 / 作業単位 A-12a）。**理由ごとに別の定数を選ぶ。**
    ///
    /// **属性の値・記憶の本文と潰さないこと**（上限が違い、直す欄が違う）。上限の数を文言に書かない。
    #[must_use]
    pub const fn invalid_agreement_content(reason: AgreementContentError) -> Self {
        let (message, code) = match reason {
            AgreementContentError::Empty => ("取り決めの中身が空です", "agreement_content_empty"),
            AgreementContentError::Control => (
                "取り決めの中身に改行や制御文字は使えません",
                "agreement_content_control",
            ),
            AgreementContentError::TooLong => {
                ("取り決めの中身が長すぎます", "agreement_content_too_long")
            }
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// 取り決めの種類（`kind`）の綴りが `promise` / `rule` のどちらでもない（作業単位 A-12a）。
    ///
    /// **`invalid_body` に潰さない**（属性の主語と同じ理由 —— 直す欄が分かる理由で返す）。
    /// 受理される綴りをメッセージに書くのは意図的（入力を反射しているわけではない）。
    #[must_use]
    pub const fn invalid_agreement_kind() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "取り決めの種類は promise か rule のどちらかです",
            kind: "invalid_request_error",
            code: Some("agreement_kind_unknown"),
        }
    }

    /// 取り決めの守る側（`keeper`）の綴りが `user` / `character` / `both` のどれでもない（作業単位 A-12a）。
    ///
    /// **`invalid_attribute_subject` と潰さない** —— 守る側には `both` があり、直す要求も違う。
    #[must_use]
    pub const fn invalid_agreement_keeper() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "取り決めの守る側は user / character / both のどれかです",
            kind: "invalid_request_error",
            code: Some("agreement_keeper_unknown"),
        }
    }

    /// 終わらせ方（`state`）の綴りが `fulfilled` / `broken` / `withdrawn` のどれでもない（作業単位 A-12a）。
    ///
    /// **`active` もここに落ちる**（有効へ「終わらせる」ことはできない）。
    #[must_use]
    pub const fn invalid_agreement_ending() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "取り決めの終わらせ方は fulfilled / broken / withdrawn のどれかです",
            kind: "invalid_request_error",
            code: Some("agreement_ending_unknown"),
        }
    }

    /// **有効な取り決めが上限に達している**（ADR-0051 §決定4 / 作業単位 A-12a）。**何も書かれない。**
    ///
    /// **409。** 要求そのものは正しく、**いまの状態と衝突している**だけ（先例は
    /// `stat_scale_below_stored_value` —— 先に別の操作をすれば通る）。**直すのは古い取り決めを
    /// 終わらせること**で、文言はその次の一手を言う。**400（`too_many_attributes`）と潰さない**
    /// —— あちらは送った集合そのものが多すぎる（減らして送れば通る）。
    ///
    /// **件数も上限も本文に載せない**（`Display` を定数に保つ絶対制約）。上限は一覧の応答が
    /// `active_limit` として返す。
    ///
    /// **文言は操作に依らない**（作業単位 A-12c-2 で「…終わらせてから書いてください」から直した）——
    /// 書く口だけでなく、提案を採る口（`adopt_agreement`）も同じ 409 を返す。「書いて」と言うと、
    /// 採ろうとした利用者に、していない操作を指す。
    #[must_use]
    pub const fn agreement_limit_reached() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "有効な取り決めが上限に達しています。古いものを終わらせてください",
            kind: "invalid_request_error",
            code: Some("agreement_limit_reached"),
        }
    }

    /// 指した取り決めが**このキャラクターのものとして存在しない**（作業単位 A-12a）。
    ///
    /// 他のキャラクターの取り決めと存在しない id を分けない（`store` の `UnknownAgreement` の doc）。
    /// **`character_not_found` / `memory_not_found` と別のコード**（直す相手が違う）。
    #[must_use]
    pub const fn agreement_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "指定された取り決めが存在しません",
            kind: "invalid_request_error",
            code: Some("agreement_not_found"),
        }
    }

    /// **有効でない取り決め（終わったもの・提案中・却下）を終わらせようとした**（作業単位 A-12a。
    /// 提案中・却下は A-12c-1 で増えた）。**何も書かれない。**
    ///
    /// **409**（行はこのキャラクターのもので、状態だけが衝突している /
    /// `memory_already_superseded` と同じ立場）。**404 に潰さない。**
    /// 文言は A-12c-1 で「もう終わっています」から直した（提案中にも当たるようになったため）。
    #[must_use]
    pub const fn agreement_not_active() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "指定された取り決めは有効ではありません",
            kind: "invalid_request_error",
            code: Some("agreement_not_active"),
        }
    }

    /// **提案中でない取り決めを、採る・捨てるしようとした**（作業単位 A-12c-1）。**何も書かれない。**
    ///
    /// **409**（`agreement_not_active` と同じ立場）。もう採った・捨てた提案を、もう一度押した形。
    #[must_use]
    pub const fn agreement_not_proposed() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "指定された取り決めは提案中ではありません",
            kind: "invalid_request_error",
            code: Some("agreement_not_proposed"),
        }
    }

    /// 同じ主語の同じキーを複数回指定した（正規化後に同じになる場合を含む）。
    ///
    /// **`duplicate_stat_axis` と潰さないこと。** 直す対象のテーブルが違う。
    #[must_use]
    pub const fn duplicate_attribute() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "同じ属性を重複して指定することはできません",
            kind: "invalid_request_error",
            code: Some("duplicate_attribute"),
        }
    }

    /// 指定されたキャラクターが存在しない。
    #[must_use]
    pub const fn character_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "指定されたキャラクターが存在しません",
            kind: "invalid_request_error",
            code: Some("character_not_found"),
        }
    }

    /// 同じ**識別子**のキャラクターが既に存在する（**正当な重複**）。
    ///
    /// **文言は「名前」ではなく「識別子」で書く**（作業単位 0004b-3 /
    /// `invalid_character_handle` と同じ理由）。この 409 が返る経路は
    /// `classify_create` の `is_unique_violation()` だけで、いま `character` に残る
    /// 一意制約は **`handle` の列 UNIQUE のみ**（`display_name` は一意でなくてよい /
    /// ADR-0011）。「同じ名前の…」と返すと、受け取った側は呼び名が衝突したと読んで
    /// `display_name` を変えて再送し、また 409 になる。
    ///
    /// **`code` は変えない**（`character_already_exists`）。区別の担い手は `code` で、
    /// ここで直しているのは人間が読む側だけ。
    #[must_use]
    pub const fn character_already_exists() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "同じ識別子のキャラクターが既に存在します",
            kind: "invalid_request_error",
            code: Some("character_already_exists"),
        }
    }

    // **`conflicting_unnormalized_name()` はここにあった**（作業単位 0004b-2 で削除）。
    //
    // 「ゴースト行（`" aoi "`）が正規化キーを塞いでいる」を `character_already_exists`
    // と分けるための 409 だったが、`StoreError::ConflictingUnnormalizedName` ごと
    // 消えたので**返す経路が無い**（`migrations/0006_drop_character_name_key_idx.sql`）。
    // ゴースト行があっても `POST /admin/characters` は成功する。
    /// **しまっていない、会話のあるキャラクターは削除できない**（ADR-0016 §2 / ADR-0049 §決定1）。
    ///
    /// **文言は次の一手（先にしまう）を言う。** しまえば `DELETE` は通る（中身ごと消える）。
    /// **`code` は変えない**（`character_has_sessions`。区別の担い手は `code`）。
    ///
    /// **`character_not_found` と同じ応答に潰さないこと。** 「そんなキャラクターは居ない」と
    /// 「居るが消せない」は、利用者から見て別の話（前者は名前の打ち間違いなので綴りを直す、
    /// 後者は仕様上の拒否なので綴りを直しても永久に通らない）。
    ///
    /// 409 なのは、要求そのものは正しいが**現在の状態と衝突している**ため。
    /// 400 にすると「送り方が悪い」と読めてしまう。
    #[must_use]
    pub const fn character_has_sessions() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "会話のあるキャラクターは、先にしまわないと削除できません",
            kind: "invalid_request_error",
            code: Some("character_has_sessions"),
        }
    }

    /// 記憶の主語（`subject`）の綴りが `user` / `character` のどちらでもない
    /// （ADR-0022 / 作業単位 14b）。
    ///
    /// **`invalid_attribute_subject` と同じ応答に潰さないこと。** 綴りの判定こそ同じ
    /// （`admin::subject_of` の1箇所）だが、**直す相手の要求が違う。**
    /// 「属性の主語は…」と返された運用者は `PUT .../attributes` の本文を見に行く。
    ///
    /// **受理される綴りをメッセージに書くのは意図的**（`invalid_attribute_subject` と同じ。
    /// 2値しか無く、入力を反射しているわけでもないので定数の性質は保たれる）。
    #[must_use]
    pub const fn invalid_memory_subject() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "記憶の主語は user か character のどちらかです",
            kind: "invalid_request_error",
            code: Some("memory_subject_unknown"),
        }
    }

    /// 訂正版の本文が受け付けられない（ADR-0022 / 作業単位 14b）。
    ///
    /// **`invalid_body` に潰さないこと。** JSON の形は正しく、値だけが規則に合わない。
    /// **上限の数値をメッセージに書かない**（`Display` を定数に保つ絶対制約。
    /// 数字の出どころは `MemoryContent::MAX_CHARS` ひとつ）。
    #[must_use]
    pub const fn invalid_memory_content(reason: ContentError) -> Self {
        let (message, code) = match reason {
            ContentError::Empty => ("記憶の本文が空です", "memory_content_empty"),
            ContentError::TooLong => ("記憶の本文が長すぎます", "memory_content_too_long"),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// 訂正版の分類が受け付けられない（ADR-0022 / 作業単位 14b）。
    ///
    /// **本文の理由と混ぜないこと。** どちらの欄を直すのかが応答から分かる必要がある。
    #[must_use]
    pub const fn invalid_memory_category(reason: CategoryError) -> Self {
        let (message, code) = match reason {
            CategoryError::Empty => ("記憶の分類が空です", "memory_category_empty"),
            CategoryError::Control => (
                "記憶の分類に改行や制御文字は使えません",
                "memory_category_control",
            ),
            CategoryError::TooLong => ("記憶の分類が長すぎます", "memory_category_too_long"),
        };
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            kind: "invalid_request_error",
            code: Some(code),
        }
    }

    /// 訂正しようとした記憶が**どこにも無い**（ADR-0022 / 作業単位 14b）。
    ///
    /// **`character_not_found` と別のコード。** 直す相手が違う——
    /// あちらはパスの識別子、こちらはパスの id。
    ///
    /// **`memory_belongs_to_another_character` とも別のコード**（下記）。
    #[must_use]
    pub const fn memory_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "指定された記憶が存在しません",
            kind: "invalid_request_error",
            code: Some("memory_not_found"),
        }
    }

    /// 指した記憶が**別のキャラクターのもの**（ADR-0022 / 作業単位 14b）。
    ///
    /// # なぜ「存在しない」と同じ応答に潰さないのか
    ///
    /// 潰すと、**正しい id を持っている運用者が id を疑って探し直す。**
    /// 直すべきはパスの識別子の方で、**そこを直せば通る。**
    ///
    /// **存在を隠すことで守れるものが無い。** `/admin/` に認証は無く（ADR-0008）、
    /// **有効な記憶なら**、同じ相手が `GET /admin/characters/{相手}/memories` を叩けば読める。
    /// **隠せるのは「読める相手」からだけで、実際には誰からも隠せていない。**
    ///
    /// # 上書き済みの行については、この応答の方が多くを明かす（**過大主張しない**）
    ///
    /// **修復の一覧は `superseded_by IS NULL` の行しか返さない**ので、
    /// 上書き済みの行は**どの `GET` にも現れない。** それでもこの応答と
    /// `memory_already_superseded` を突き合わせれば、総当たりで次が分かる。
    ///
    /// | 引ける | 引けない |
    /// |---|---|
    /// | その id の行が**存在する**こと | **本文**（1バイトも出ない） |
    /// | それが**このキャラクターのものではない**こと（＝持ち主の絞り込み） | 分類・主語・日時 |
    /// | 持ち主の識別子で叩き直せば**鎖の状態**（訂正済みかどうか） | 後継の id |
    ///
    /// **実害は無い**——認証が無い以上、守るべき境界そのものが存在しない
    /// （DB を直接読める相手と同じ立場）。**「塞いだ」と書かないこと**が要点で、
    /// 認証を足す日にはこの前提ごと変わる。
    ///
    /// # それでも**鎖の状態は明かさない**
    ///
    /// 別キャラクターの・すでに訂正済みの記憶を指しても、返るのは**この応答**
    /// （`memory_already_superseded` ではない / ADR-0022「拒否の優先順」）。
    /// あちらを返すと「その記憶は存在し、訂正済みである」という
    /// **他キャラクターの状態の開示**になる。判定の順序は `store` が持つ。
    ///
    /// **404 なのは、指したパス（このキャラクターの記憶）が存在しないから。**
    /// 409 は「このキャラクターの記憶ではあるが、状態が衝突している」に取ってある。
    #[must_use]
    pub const fn memory_belongs_to_another_character() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "指定された記憶はこのキャラクターのものではありません",
            kind: "invalid_request_error",
            code: Some("memory_belongs_to_another_character"),
        }
    }

    /// すでに訂正済みの記憶を、もう一度訂正しようとした（ADR-0022 / 作業単位 14b）。
    ///
    /// **409。** 要求そのものは正しく（宛先も本文も合っている）、
    /// **現在の状態と衝突している**だけ（先例は `character_has_sessions`）。
    /// 400 にすると「送り方が悪い」と読めてしまい、実際にやるべきこと
    /// （**一覧を取り直して、いま有効な行を指す**）に辿り着けない。
    ///
    /// 鎖を線形に保つための拒否なので、**`memory_not_found` に潰さないこと**
    /// ——行はある。指すべきなのが後継の方である。
    #[must_use]
    pub const fn memory_already_superseded() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "指定された記憶はすでに訂正されています。いま有効な記憶を指定してください",
            kind: "invalid_request_error",
            code: Some("memory_already_superseded"),
        }
    }

    /// `model` フィールドを解釈できない。**理由ごとに別の定数を選ぶ。**
    ///
    /// **404 `model_not_found` に潰さないこと**（ADR-0004 / docs/06-roadmap.md）。
    /// `aoi@new `（接尾辞に空白）が「そんなキャラクターは無い」に化けると、
    /// 接尾辞を trim しない設計を知らない利用者は原因に到達できない。
    #[must_use]
    pub const fn invalid_model(reason: SelectorError) -> Self {
        match reason {
            // 名前部の理由はそのまま伝える（既存の定数を使い回す。潰さない）。
            SelectorError::Handle(inner) => Self::invalid_character_handle(inner),
            SelectorError::EmptySuffix => Self {
                status: StatusCode::BAD_REQUEST,
                message: "model の @ の後ろが空です",
                kind: "invalid_request_error",
                code: Some("model_empty_suffix"),
            },
            SelectorError::InvalidSuffix => Self {
                status: StatusCode::BAD_REQUEST,
                message: "model の @ の後ろを解釈できません",
                kind: "invalid_request_error",
                code: Some("model_invalid_suffix"),
            },
        }
    }

    /// `model` が指すキャラクターが存在しない（ADR-0004 の 404 / `model_not_found`）。
    ///
    /// **`character_not_found`（`/admin/`）と同じ応答に潰さない。** あちらはパスで
    /// 指したキャラクターの話で、こちらは `model` フィールドの話。
    #[must_use]
    pub const fn model_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "指定された model のキャラクターが存在しません",
            kind: "invalid_request_error",
            code: Some("model_not_found"),
        }
    }

    /// `model` が指すキャラクターは**しまわれている**（ADR-0049 §決定1 / 作業単位 A-10）。
    ///
    /// **409。** キャラクターは居て、要求そのものも正しく、**いまの状態と衝突している**だけ
    /// （先例は `character_has_sessions`）。戻せば通る。
    ///
    /// **`model_not_found` に潰さないこと。** 「そんなキャラクターは居ない」と返すと、
    /// 利用者は識別子の綴りを疑い、作り直そうとして 409 `character_already_exists` に当たる
    /// （しまっただけのあいだ、識別子は使用中のまま / ADR-0049 §決定1）。
    ///
    /// 互換の上では「今まで起きなかった条件に新しい失敗を返す」なので `1.x`（ADR-0047 §② P-2b の表）。
    /// **今まで成功していた要求は1つも失敗にならない**（しまう操作そのものが新しい）。
    #[must_use]
    pub const fn model_archived() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "指定された model のキャラクターはしまわれています。戻すと話せます",
            kind: "invalid_request_error",
            code: Some("model_archived"),
        }
    }

    /// `aoi@<id>` のセッションが、そのキャラクターのものとして存在しない。
    ///
    /// **`model_not_found` に潰さない。** キャラクターは実在するのに
    /// 「そんなキャラクターは無い」と返すと、原因が session id 側だと分からない。
    ///
    /// **会話履歴を読む口**（`GET /admin/characters/{handle}/sessions/{id}/messages` /
    /// 作業単位 D-12c-1）も同じ応答を返す。**他のキャラクターのセッションと、
    /// 存在しないセッションを分けない**——判定は `Store::find_session` の1文で、
    /// あちらが両方を `None` にしている（`aoi@<id>` と同じ立場）。
    #[must_use]
    pub const fn session_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "指定されたセッションが存在しません",
            kind: "invalid_request_error",
            code: Some("session_not_found"),
        }
    }

    /// 会話のページの指定（`before` / `limit`）が**範囲の外**（作業単位 D-12c-1）。
    ///
    /// **`invalid_query` に潰さない。** あちらは「読めない」（数でない・知らない欄）で、
    /// 直すのは綴りである。こちらは読めたうえで値が外れている（0・負・上限超え）ので、
    /// 直すのは値である。
    ///
    /// **上限の数を文言に写さない**（写すと `admin.rs` の定数との写しが1つ増え、黙って古びる）。
    #[must_use]
    pub const fn message_page_out_of_range() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "before と limit は 1 以上で、limit は上限以内で指定してください",
            kind: "invalid_request_error",
            code: Some("message_page_out_of_range"),
        }
    }

    /// `messages` に `user` の発言が1件も無い。
    ///
    /// 採用するのは最後の `user` メッセージだけ（docs/02-architecture.md
    /// 「自前DBを正とする」）なので、1件も無ければ生成すべき発言が決まらない。
    #[must_use]
    pub const fn no_user_message() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "messages に user のメッセージがありません",
            kind: "invalid_request_error",
            code: Some("no_user_message"),
        }
    }

    /// 採用する発言（**最後の `user`**）の `content` が文字列でなかった。
    ///
    /// マルチモーダル形式（`[{"type": "text", ...}]`）や `null` がこれに当たる。
    ///
    /// **`invalid_body` に潰さないこと。** 「本文が壊れている」と
    /// 「この形式の content は未対応」は原因も直し方も違う。前者だと返すと、
    /// 正しい JSON を送っている側は綴りを疑って堂々巡りになる。
    ///
    /// **`no_user_message` にも潰さない。** `user` の発言は在るのだから、
    /// 「1件も無い」と返すのは事実に反する。
    #[must_use]
    pub const fn unsupported_message_content() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "この形式の content にはまだ対応していません",
            kind: "invalid_request_error",
            code: Some("unsupported_message_content"),
        }
    }

    /// `max_tokens: 0` を受け取った。**黙って上流へ流さない。**
    ///
    /// 0 は「1トークンも生成するな」であり、要求として成立していない
    /// （OpenAI 側も最小 1）。切り下げ（`cap_max_tokens`）は上限の話なので
    /// 0 を弾けない。入口で拒否する。
    #[must_use]
    pub const fn invalid_max_tokens() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "max_tokens には 1 以上を指定してください",
            kind: "invalid_request_error",
            code: Some("invalid_max_tokens"),
        }
    }

    /// 削れない枠だけでトークン予算を超えた（`PromptError::BudgetExhausted`）。
    ///
    /// **`internal()` に潰さない。** これは障害ではなく
    /// 「ペルソナ・形式指示・今回の発言が予算に対して大きすぎる」という
    /// **運用者向けの情報**で、`{err:?}` を出さずとも原因を指し示せる。
    #[must_use]
    pub const fn prompt_budget_exhausted() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "キャラクター設定と発言がコンテキスト予算に収まりません",
            kind: "invalid_request_error",
            code: Some("prompt_budget_exhausted"),
        }
    }

    /// 推論サーバへ接続できなかった。
    ///
    /// **他の上流エラーと分ける。** LM Studio は普段停止している運用なので
    /// （docs/04-environment.md）、「起動しているか確認する」という案内が要る。
    #[must_use]
    pub const fn upstream_unavailable() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "推論サーバへ接続できませんでした。起動しているか確認してください",
            kind: "server_error",
            code: Some("upstream_unavailable"),
        }
    }

    /// 推論サーバの応答が時間内に完了しなかった。
    #[must_use]
    pub const fn upstream_timeout() -> Self {
        Self {
            status: StatusCode::GATEWAY_TIMEOUT,
            message: "推論サーバの応答が時間内に完了しませんでした",
            kind: "server_error",
            code: Some("upstream_timeout"),
        }
    }

    /// 推論サーバとのやり取りが上記以外の理由で失敗した。
    ///
    /// **上流のステータス値も応答本文もここには載せない**（ログにだけ出す）。
    #[must_use]
    pub const fn upstream_failed() -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: "推論サーバとの通信に失敗しました",
            kind: "server_error",
            code: Some("upstream_failed"),
        }
    }

    /// **推論サーバが鍵を受け付けなかった**（上流の 401 / 403 / ADR-0056 §決定4 / 作業単位 A-13b-1）。
    ///
    /// **`upstream_failed` に潰さない**——鍵が無い・違う・期限切れのどれでも、直すのは
    /// プロファイルの `api_key` であって、上流の起動や通信ではない（origin#77 では
    /// 「通信に失敗しました」としか出ず、401 はログにしか無かった）。
    ///
    /// **状態は 502**（`upstream_failed` と同じ）。401 / 403 を返すと、クライアントは
    /// 「personacore への鍵が違う」と読む。断ったのは personacore の先の上流である。
    /// 判定は `LlmError::refused_the_key`（1箇所）。
    #[must_use]
    pub const fn upstream_unauthorized() -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: "推論サーバが鍵を受け付けませんでした",
            kind: "server_error",
            code: Some("upstream_unauthorized"),
        }
    }

    /// **鍵を平文の `http://` でローカルネットワークの外へ送る承認が無いので、送らなかった**
    /// （ADR-0056 §決定5 / 作業単位 A-13b-2）。
    ///
    /// **`upstream_unauthorized` と分ける**——上流は鍵を見ていない（接続する前に止めている）。
    /// 直すのはプロファイルの面の承認（`allow_key_over_plain_http`）か宛先で、鍵ではない。
    ///
    /// **状態は 502**（`upstream_unauthorized` と同じ考え方 / ADR-0056 追記3）。401 / 403 を返すと、
    /// クライアントは「personacore への鍵が違う」と読む。409 / 422 は依頼そのものの問題に読める
    /// （依頼は正しく、直す場所は personacore の先へ送る設定）。**503 は「待てば通る」に読める**ので採らない。
    /// 判定は `LlmError::KeyNotApproved`（1箇所）。
    #[must_use]
    pub const fn upstream_key_not_approved() -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: "鍵を暗号化せずにローカルネットワークの外へ送る承認が無いので、推論サーバへ送りませんでした",
            kind: "server_error",
            code: Some("upstream_key_not_approved"),
        }
    }

    // ------------------------------------------------------------------
    // 走ったままの差し替え（`POST /admin/upstream/profile` / ADR-0042 §決定4 / 作業単位 F-4b-2）
    //
    // **どれも詳細（パス・綴り）を載せない。** 何が駄目だったかはログにだけ出す
    // （`server::switch` が同じ場所で `warn` / `info` を出している）。
    // ------------------------------------------------------------------

    /// **別の差し替えが進行中**（ADR-0042 §決定4「同時に1つだけ。列に並べない」）。
    ///
    /// 409 なのは、要求そのものは正しいが**いまの状態と衝突している**ため
    /// （`character_has_sessions` と同じ判断）。**待てば通る**ので、そう言う。
    #[must_use]
    pub const fn upstream_switch_in_progress() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "上流の差し替えが進行中です。終わってから選び直してください",
            kind: "invalid_request_error",
            code: Some("upstream_switch_in_progress"),
        }
    }

    /// **この起動は上流を差し替えられない**（プロファイルの置き場所か照会先を持たない）。
    ///
    /// **`upstream_switch_in_progress` と潰さない** —— あちらは待てば通り、こちらは永久に通らない。
    #[must_use]
    pub const fn upstream_switch_unavailable() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "この起動では上流を差し替えられません",
            kind: "invalid_request_error",
            code: Some("upstream_switch_unavailable"),
        }
    }

    /// **その名前の上流プロファイルが無い。**
    #[must_use]
    pub const fn upstream_profile_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "指定された上流プロファイルがありません",
            kind: "invalid_request_error",
            code: Some("upstream_profile_not_found"),
        }
    }

    /// **上流プロファイルのファイルを読めない**（無い・JSON でない・形が不正）。
    ///
    /// **置き場所は載せない**（絶対制約）。起動ログがその場所を言っている。
    #[must_use]
    pub const fn upstream_profiles_unreadable() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "上流プロファイルを読めませんでした。ファイルの形を確かめてください",
            kind: "invalid_request_error",
            code: Some("upstream_profiles_unreadable"),
        }
    }

    /// **差し替え先の宛先かモデルの綴りが不正**（起動時の設定と同じ検証）。
    #[must_use]
    pub const fn upstream_profile_unusable() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "上流プロファイルの宛先かモデルの綴りが不正です",
            kind: "invalid_request_error",
            code: Some("upstream_profile_unusable"),
        }
    }

    /// **差し替え先の gguf が無い**（ADR-0042 §決定2「起こす前に確かめる」）。**何も止めていない。**
    #[must_use]
    pub const fn upstream_gguf_missing() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "上流プロファイルが名指しする gguf がありません",
            kind: "invalid_request_error",
            code: Some("upstream_gguf_missing"),
        }
    }

    /// **差し替え先の起こす面が、personacore 自身の待ち受けと同じポート**（作業単位 B-52）。
    #[must_use]
    pub const fn upstream_port_collision() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "上流プロファイルの宛先が personacore 自身の待ち受けと同じです",
            kind: "invalid_request_error",
            code: Some("upstream_port_collision"),
        }
    }

    // ------------------------------------------------------------------
    // 画面から上流プロファイルを足す・直す
    // （`PUT /admin/upstream/profiles/{name}` / ADR-0042 §決定8 / 作業単位 F-4b-3）
    //
    // **どれもパスも綴りも載せない。** 置き場所は `GET /admin/upstream/profiles` の `path` が言う。
    // ------------------------------------------------------------------

    /// **その名前は `--upstream <名前>` で選べない**（空・前後の空白・`--` で始まる）。
    ///
    /// **画面で作れたのに、起動時には選べない名前を作らない。**
    #[must_use]
    pub const fn upstream_profile_name_invalid() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "その名前は上流プロファイルの名前に使えません（空・前後の空白・-- で始まる名前は使えません）",
            kind: "invalid_request_error",
            code: Some("upstream_profile_name_invalid"),
        }
    }

    /// **この置き場所は画面からは直せない**（`PERSONACORE_UPSTREAMS` が別の場所を指している／置き場所が無い）。
    ///
    /// 409 なのは `upstream_switch_unavailable` と同じ判断（要求は正しいが、この起動の状態と衝突する）。
    #[must_use]
    pub const fn upstream_profiles_read_only() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "この起動では上流プロファイルを画面から直せません（読むだけです）",
            kind: "invalid_request_error",
            code: Some("upstream_profiles_read_only"),
        }
    }

    /// **画面が読んだ後に、ファイルが書き換えられていた**（ADR-0042 §決定8）。**上書きしていない。**
    ///
    /// **`upstream_profiles_read_only` と潰さない**——こちらは読み直せば通る。
    #[must_use]
    pub const fn upstream_profiles_changed() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "上流プロファイルは読んだ後に書き換えられています。読み直してから直してください",
            kind: "invalid_request_error",
            code: Some("upstream_profiles_changed"),
        }
    }

    /// **直した後のファイルが読み込みの検証に通らない。何も書いていない。**
    ///
    /// **どこが駄目かは、応答の `reason` / `face` / `key` が言う**（[`Self::into_response_with`]。どれも定数）。
    #[must_use]
    pub const fn upstream_profile_invalid() -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: "上流プロファイルの形が不正なので保存しませんでした",
            kind: "invalid_request_error",
            code: Some("upstream_profile_invalid"),
        }
    }

    /// **書き換えられなかった**（一時ファイルを書けない・差し替えられない・読み直しが合わない）。
    ///
    /// **元のファイルは残っている**（置き換えで書くので）。原因はログにだけ出す。
    #[must_use]
    pub const fn upstream_profiles_write_failed() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "上流プロファイルを書き換えられませんでした（元のファイルは残っています）",
            kind: "server_error",
            code: Some("upstream_profiles_write_failed"),
        }
    }

    /// **書いた後に読み直すと、書いたもの・依頼と違った**（**ファイルは書き換わっている**）。
    ///
    /// **`upstream_profiles_write_failed` と潰さない**——あちらは元のファイルが残っており、
    /// こちらは書き換わった後である（別の書き手が割り込んだか、書き出しと読み込みの形が食い違った）。
    #[must_use]
    pub const fn upstream_profiles_verify_failed() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "上流プロファイルを書いた後に読み直すと、依頼と違いました。一覧を読み直して確かめてください",
            kind: "server_error",
            code: Some("upstream_profiles_verify_failed"),
        }
    }

    // ------------------------------------------------------------------
    // 待ち受けのポートの保存（`PUT /admin/server/port` / ADR-0057 §決定5・6 / 作業単位 F-12a）
    //
    // **どれもパスも番号も載せない。** 置き場所と原因はログにだけ出す（`listening_port`）。
    // ------------------------------------------------------------------

    /// **保存できる範囲（1024〜65535）の外**（`0` と負の値を含む）。
    ///
    /// 400 なのは、数の範囲の誤りに 400 を返す先例 [`Self::invalid_max_tokens`] に倣う（ADR-0057 §決定6）。
    /// **`invalid_body` に潰さない**——本文は読めていて、直すのは番号である。
    #[must_use]
    pub const fn invalid_listening_port() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "待ち受けのポートには 1024〜65535 の番号を指定してください",
            kind: "invalid_request_error",
            code: Some("invalid_listening_port"),
        }
    }

    /// **この起動では待ち受けのポートを保存できない**（`PERSONACORE_UPSTREAMS` が別の場所を指している／置き場所が無い）。
    ///
    /// 409 なのは [`Self::upstream_profiles_read_only`] に倣う（要求は正しいが、この起動の状態と衝突する）。
    #[must_use]
    pub const fn listening_port_read_only() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: "この起動では待ち受けのポートを保存できません",
            kind: "invalid_request_error",
            code: Some("listening_port_read_only"),
        }
    }

    /// **保存のファイルを書き換えられなかった**（書けない・消せない）。**前の中身は残っている。**
    #[must_use]
    pub const fn listening_port_write_failed() -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "待ち受けのポートの保存を書き換えられませんでした（前の中身は残っています）",
            kind: "server_error",
            code: Some("listening_port_write_failed"),
        }
    }

    /// **`Host` / `Origin` がこのサーバのものではない要求**（作業単位 B-58 / `host_guard` モジュール）。
    ///
    /// **届いた `Host` / `Origin` を反射しない**（ログにだけ出す / `host_guard::refuse`）。
    /// **`not_found` 等に潰さない**——正しい口を叩いているのに断られた、と読めることが直す手掛かりになる
    /// （リバースプロキシや `hosts` の別名から繋いだ使い手が、何を直せばよいか分かる）。
    #[must_use]
    pub const fn request_origin_not_allowed() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: "この要求の Host または Origin は受け付けられません。\
                      http://127.0.0.1:<ポート> か http://localhost:<ポート> から繋いでください",
            kind: "invalid_request_error",
            code: Some("request_origin_not_allowed"),
        }
    }

    /// **定数の欄を足した応答**（作業単位 F-4b-3）。
    ///
    /// **足せるのは `&'static str` だけ**——この型の「定数しか載せられない」性質を保つ。
    /// `null`（`None`）の欄も鍵ごと残す（読み手の型を1つにする）。
    /// **`error` の中の既存の鍵（`message` / `type` / `code`）は上書きしない。**
    pub(crate) fn into_response_with(
        self,
        details: &[(&'static str, Option<&'static str>)],
    ) -> Response {
        let mut detail = Map::new();
        detail.insert("message".to_owned(), Value::from(self.message));
        detail.insert("type".to_owned(), Value::from(self.kind));
        if let Some(code) = self.code {
            detail.insert("code".to_owned(), Value::from(code));
        }
        for (key, value) in details {
            if !detail.contains_key(*key) {
                detail.insert((*key).to_owned(), value.map_or(Value::Null, Value::from));
            }
        }
        let mut body = Map::new();
        body.insert("error".to_owned(), Value::Object(detail));
        (self.status, Json(Value::Object(body))).into_response()
    }

    /// HTTP ステータス。
    #[must_use]
    pub const fn status(self) -> StatusCode {
        self.status
    }

    /// 細分類の綴り（仕様書が `code` の `enum` を定数から作るため / 作業単位 P-2a）。
    #[must_use]
    pub(crate) const fn code(self) -> Option<&'static str> {
        self.code
    }

    /// **SSE の途中で失敗を伝えるためのデータ**（作業単位 11a-2b / `server::relay`）。
    ///
    /// 200 とヘッダを送った後は 4xx/5xx を返せないので、同じ error オブジェクトを
    /// SSE の `data:` として流す。**載る値は `into_response` の本文と完全に同じ定数**で、
    /// ここでも入力もエラーの詳細も反射しない
    /// （`the_event_data_matches_the_response_body` が両者の一致を固定している）。
    ///
    /// **ステータスは載せない。** 既に 200 を送っている以上、ここに 502 と書いても
    /// それは応答のステータスではなく、読み手を混乱させるだけ。区別は `code` が担う。
    ///
    /// `Value` を直に組み立てるのは、直列化の失敗という**到達不能な分岐**
    /// （`unwrap` も、誰も通らない `unwrap_or_else` も）を作らないため。
    pub(crate) fn as_event_data(self) -> String {
        let mut detail = Map::new();
        detail.insert("message".to_owned(), Value::from(self.message));
        detail.insert("type".to_owned(), Value::from(self.kind));
        // 無いときはキーごと落とす（`ErrorDetail` の `skip_serializing_if` と同じ）。
        if let Some(code) = self.code {
            detail.insert("code".to_owned(), Value::from(code));
        }

        let mut body = Map::new();
        body.insert("error".to_owned(), Value::Object(detail));

        Value::Object(body).to_string()
    }
}

impl From<HandleError> for ApiError {
    fn from(reason: HandleError) -> Self {
        Self::invalid_character_handle(reason)
    }
}

impl From<DisplayNameError> for ApiError {
    fn from(reason: DisplayNameError) -> Self {
        Self::invalid_display_name(reason)
    }
}

impl From<AxisError> for ApiError {
    fn from(reason: AxisError) -> Self {
        Self::invalid_stat_axis(reason)
    }
}

impl From<PersonaError> for ApiError {
    fn from(reason: PersonaError) -> Self {
        Self::invalid_persona(reason)
    }
}

impl From<OutputFormatError> for ApiError {
    fn from(reason: OutputFormatError) -> Self {
        Self::invalid_output_format(reason)
    }
}

impl From<ScaleError> for ApiError {
    fn from(reason: ScaleError) -> Self {
        Self::invalid_stat_scale(reason)
    }
}

impl From<AxisDiscoverySlotsError> for ApiError {
    fn from(reason: AxisDiscoverySlotsError) -> Self {
        Self::invalid_axis_discovery_slots(reason)
    }
}

impl From<StatDriftModeError> for ApiError {
    fn from(reason: StatDriftModeError) -> Self {
        match reason {
            StatDriftModeError::Unknown => Self::invalid_stat_drift_mode(),
        }
    }
}

impl From<ValueError> for ApiError {
    fn from(reason: ValueError) -> Self {
        Self::invalid_stat_value(reason)
    }
}

impl From<AttributeKeyError> for ApiError {
    fn from(reason: AttributeKeyError) -> Self {
        Self::invalid_attribute_key(reason)
    }
}

impl From<AttributeValueError> for ApiError {
    fn from(reason: AttributeValueError) -> Self {
        Self::invalid_attribute_value(reason)
    }
}

impl From<AttributeSetError> for ApiError {
    fn from(reason: AttributeSetError) -> Self {
        Self::too_many_attributes(reason)
    }
}

impl From<AgreementContentError> for ApiError {
    fn from(reason: AgreementContentError) -> Self {
        Self::invalid_agreement_content(reason)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: ErrorDetail {
                    message: self.message,
                    kind: self.kind,
                    code: self.code,
                },
            }),
        )
            .into_response()
    }
}

// ----------------------------------------------------------------------
// 失敗の一覧（API の形の指紋のため / ADR-0047 §② P-2b の版の上げ方・作業単位 O-7）
// ----------------------------------------------------------------------

/// **失敗の応答の形**（状態コード・`type`・`code`）。**文言（`message`）は入れない**
/// —— 文言を直しただけで API の版が上がらないように。
#[cfg(any(test, feature = "testing"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ErrorShape {
    /// HTTP の状態コード。
    pub status: u16,
    /// `error.type`。
    pub kind: &'static str,
    /// `error.code`（無いときは `None`）。
    pub code: Option<&'static str>,
}

#[cfg(any(test, feature = "testing"))]
impl From<ApiError> for ErrorShape {
    fn from(error: ApiError) -> Self {
        Self {
            status: error.status.as_u16(),
            kind: error.kind,
            code: error.code,
        }
    }
}

/// **domain の理由の enum の全部の値**（**網羅は型が強制する**）。
///
/// 同じ並びから、ワイルドカードの無い `match` と配列の両方を作る。
/// **変種を足すと `match` が網羅していないと言ってコンパイルが止まり、
/// ここに足せば配列にも入る**（並びが1つなので、片方だけ足す経路が無い）。
#[cfg(any(test, feature = "testing"))]
macro_rules! every {
    ($ty:ident: $($variant:ident),+ $(,)?) => {{
        const fn covers(value: $ty) {
            match value {
                $($ty::$variant => {}),+
            }
        }
        let all = [$($ty::$variant),+];
        for value in all {
            covers(value);
        }
        all
    }};
}

/// **`ApiError` の構築子が返しうる、全部の失敗の形**（構築子の名前つき）。
///
/// API の形の指紋（`tests/api_shape_fingerprint.rs`）に畳み込む。仕様書の失敗の応答は
/// `default` → `ErrorBody` だけで、`code` は `enum` の無い文字列なので、**ここに出さないと
/// 新しい `code` も状態コードの変更も指紋に出ない**（O-7 のレビュー）。
///
/// # 網羅の強制
///
/// * **理由の enum の値**は [`every!`] が型で強制する。
/// * **構築子の一覧**は型では強制できない（関数を足しても何も止まらない）。
///   **見張り番が2本で強制する**（このファイルの `catalog_tests`）——
///   `ApiError` の値を書く場所（構造体リテラルと欄への代入）を囲む `fn` の名前がどれもここに在ること
///   （`impl` の種類・返り値の綴り・マクロを問わない）、
///   と、このファイルの本体に書かれた `snake_case` の文字列リテラルの集合 ==
///   ここの `code` と `type` の集合（＋理由つきの例外 —— 本文の鍵とフィーチャ名）。
///   **`code` の綴りを足して一覧に足し忘れると、後者が赤になる。**
#[cfg(any(test, feature = "testing"))]
#[must_use]
pub fn error_catalog() -> Vec<(&'static str, ErrorShape)> {
    let mut out: Vec<(&'static str, ErrorShape)> = Vec::new();
    macro_rules! entry {
        ($name:ident) => {
            out.push((stringify!($name), ApiError::$name().into()));
        };
        ($name:ident, $reasons:expr) => {
            for reason in $reasons {
                out.push((stringify!($name), ApiError::$name(reason).into()));
            }
        };
    }

    entry!(internal);
    entry!(not_found);
    entry!(method_not_allowed);
    entry!(invalid_body);
    entry!(body_too_large);
    entry!(invalid_query);
    entry!(invalid_path);
    entry!(
        invalid_character_handle,
        every!(HandleError: Empty, AtSign, Control, TooLong, Charset)
    );
    entry!(
        invalid_display_name,
        every!(DisplayNameError: Empty, Control, TooLong)
    );
    entry!(invalid_persona, every!(PersonaError: Empty, TooLong));
    entry!(
        invalid_output_format,
        every!(OutputFormatError: Empty, TooLong)
    );
    entry!(
        invalid_stat_axis,
        every!(AxisError: Empty, Control, TooLong)
    );
    entry!(invalid_stat_scale, every!(ScaleError: TooSmall, TooLarge));
    entry!(
        invalid_axis_discovery_slots,
        every!(AxisDiscoverySlotsError: TooLarge)
    );
    entry!(invalid_stat_drift_mode);
    entry!(invalid_stat_value, every!(ValueError: AboveScale));
    entry!(stat_scale_below_stored_value);
    entry!(stat_scale_changed);
    entry!(duplicate_stat_axis);
    entry!(
        invalid_attribute_key,
        every!(AttributeKeyError: Empty, Control, TooLong)
    );
    entry!(
        invalid_attribute_value,
        every!(AttributeValueError: Empty, Control, TooLong)
    );
    entry!(invalid_attribute_subject);
    entry!(too_many_attributes, every!(AttributeSetError: TooMany));
    entry!(
        invalid_agreement_content,
        every!(AgreementContentError: Empty, Control, TooLong)
    );
    entry!(invalid_agreement_kind);
    entry!(invalid_agreement_keeper);
    entry!(invalid_agreement_ending);
    entry!(agreement_limit_reached);
    entry!(agreement_not_found);
    entry!(agreement_not_active);
    entry!(agreement_not_proposed);
    entry!(duplicate_attribute);
    entry!(character_not_found);
    entry!(character_already_exists);
    entry!(character_has_sessions);
    entry!(invalid_memory_subject);
    entry!(invalid_memory_content, every!(ContentError: Empty, TooLong));
    entry!(
        invalid_memory_category,
        every!(CategoryError: Empty, Control, TooLong)
    );
    entry!(memory_not_found);
    entry!(memory_belongs_to_another_character);
    entry!(memory_already_superseded);
    // **`SelectorError::Handle` は値を持つ**ので `every!` に載らない。ここだけ手で網羅する
    // （ワイルドカードの無い `match` が、変種を足したら止まる）。
    entry!(invalid_model, {
        const fn covers(value: SelectorError) {
            match value {
                SelectorError::Handle(_)
                | SelectorError::EmptySuffix
                | SelectorError::InvalidSuffix => {}
            }
        }
        let mut all: Vec<SelectorError> =
            every!(HandleError: Empty, AtSign, Control, TooLong, Charset)
                .into_iter()
                .map(SelectorError::Handle)
                .collect();
        all.extend([SelectorError::EmptySuffix, SelectorError::InvalidSuffix]);
        for value in &all {
            covers(*value);
        }
        all
    });
    entry!(model_not_found);
    entry!(model_archived);
    entry!(session_not_found);
    entry!(message_page_out_of_range);
    entry!(no_user_message);
    entry!(unsupported_message_content);
    entry!(invalid_max_tokens);
    entry!(prompt_budget_exhausted);
    entry!(upstream_unavailable);
    entry!(upstream_timeout);
    entry!(upstream_failed);
    entry!(upstream_unauthorized);
    entry!(upstream_key_not_approved);
    entry!(upstream_switch_in_progress);
    entry!(upstream_switch_unavailable);
    entry!(upstream_profile_not_found);
    entry!(upstream_profiles_unreadable);
    entry!(upstream_profile_unusable);
    entry!(upstream_gguf_missing);
    entry!(upstream_port_collision);
    entry!(upstream_profile_name_invalid);
    entry!(upstream_profiles_read_only);
    entry!(upstream_profiles_changed);
    entry!(upstream_profile_invalid);
    entry!(upstream_profiles_write_failed);
    entry!(upstream_profiles_verify_failed);
    entry!(invalid_listening_port);
    entry!(listening_port_read_only);
    entry!(listening_port_write_failed);
    entry!(request_origin_not_allowed);
    out
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use serde_json::json;

    use super::*;

    /// 応答をステータスと JSON 本文に分解する。
    async fn parts(error: ApiError) -> (StatusCode, Value) {
        let response = error.into_response();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("本文を読めること");
        let value = serde_json::from_slice(&bytes).expect("JSON であること");
        (status, value)
    }

    /// 500 の本文は定数のみ。
    #[tokio::test]
    async fn internal_error_body_is_constant() {
        let (status, body) = parts(ApiError::internal()).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            body,
            json!({"error": {"message": "内部エラーが発生しました", "type": "server_error"}}),
            "詳細を足さないこと（`{{err:?}}` を載せない）"
        );
    }

    /// 404 の本文も定数のみ。
    #[tokio::test]
    async fn not_found_body_is_constant() {
        let (status, body) = parts(ApiError::not_found()).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(
            body,
            json!({
                "error": {
                    "message": "指定されたエンドポイントは存在しません",
                    "type": "invalid_request_error",
                }
            })
        );
    }

    /// 405 の本文も定数のみ（**空ボディにしない**）。
    #[tokio::test]
    async fn method_not_allowed_body_is_constant() {
        let (status, body) = parts(ApiError::method_not_allowed()).await;

        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            body,
            json!({
                "error": {
                    "message": "このエンドポイントは指定されたメソッドを受け付けません",
                    "type": "invalid_request_error",
                }
            })
        );
    }

    /// **`Host` / `Origin` を断った 403 の本文も定数のみ**（作業単位 B-58）。
    #[tokio::test]
    async fn request_origin_not_allowed_body_is_constant() {
        let (status, body) = parts(ApiError::request_origin_not_allowed()).await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body,
            json!({
                "error": {
                    "message": "この要求の Host または Origin は受け付けられません。\
                                http://127.0.0.1:<ポート> か http://localhost:<ポート> から繋いでください",
                    "type": "invalid_request_error",
                    "code": "request_origin_not_allowed",
                }
            })
        );
    }

    /// `Content-Type` は JSON（FE がエラーを構造化して読める）。
    #[tokio::test]
    async fn responses_are_json() {
        for error in [
            ApiError::internal(),
            ApiError::not_found(),
            ApiError::method_not_allowed(),
        ] {
            let response = error.into_response();
            let content_type = response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .expect("Content-Type があること");
            assert!(
                content_type.starts_with("application/json"),
                "実際の値 = {content_type}"
            );
        }
    }

    /// 管理 API のエラーは**すべて互いに区別できる**（同じ応答に潰さない）。
    ///
    /// **`conflicting_unnormalized_name` は 0004b-2 で消えた**ので、ここには並ばない
    /// （正規化キーの索引ごと落としたため、返す経路が存在しない）。
    #[test]
    fn admin_errors_are_all_distinguishable() {
        let errors = [
            ApiError::internal(),
            ApiError::not_found(),
            ApiError::method_not_allowed(),
            ApiError::invalid_body(),
            ApiError::body_too_large(),
            ApiError::invalid_path(),
            ApiError::invalid_character_handle(HandleError::Empty),
            ApiError::invalid_character_handle(HandleError::AtSign),
            ApiError::invalid_character_handle(HandleError::Control),
            ApiError::invalid_character_handle(HandleError::TooLong),
            ApiError::invalid_display_name(DisplayNameError::Empty),
            ApiError::invalid_display_name(DisplayNameError::Control),
            ApiError::invalid_display_name(DisplayNameError::TooLong),
            ApiError::invalid_stat_axis(AxisError::Empty),
            ApiError::invalid_stat_axis(AxisError::Control),
            ApiError::invalid_stat_axis(AxisError::TooLong),
            ApiError::invalid_persona(PersonaError::Empty),
            ApiError::invalid_persona(PersonaError::TooLong),
            ApiError::invalid_output_format(OutputFormatError::Empty),
            ApiError::invalid_output_format(OutputFormatError::TooLong),
            ApiError::duplicate_stat_axis(),
            // **属性のエラー**（ADR-0009 / 作業単位 12a）。
            // 軸名（`invalid_stat_axis`）と潰していないことも、この総当たりが見ている。
            ApiError::invalid_attribute_key(AttributeKeyError::Empty),
            ApiError::invalid_attribute_key(AttributeKeyError::Control),
            ApiError::invalid_attribute_key(AttributeKeyError::TooLong),
            ApiError::invalid_attribute_value(AttributeValueError::Empty),
            ApiError::invalid_attribute_value(AttributeValueError::Control),
            ApiError::invalid_attribute_value(AttributeValueError::TooLong),
            ApiError::invalid_attribute_subject(),
            ApiError::too_many_attributes(AttributeSetError::TooMany),
            ApiError::duplicate_attribute(),
            ApiError::character_not_found(),
            ApiError::character_already_exists(),
            ApiError::character_has_sessions(),
            // **変容の走らせ方**（ADR-0045 / 作業単位 A-8）。
            ApiError::invalid_stat_drift_mode(),
            // **見いだす枠の数**（ADR-0046 / 作業単位 A-9a）。尺度（`invalid_stat_scale`）と潰さない。
            ApiError::invalid_axis_discovery_slots(AxisDiscoverySlotsError::TooLarge),
            // **取り決め**（ADR-0051 / 作業単位 A-12a）。属性・記憶の理由と潰さない。
            ApiError::invalid_agreement_content(AgreementContentError::Empty),
            ApiError::invalid_agreement_content(AgreementContentError::Control),
            ApiError::invalid_agreement_content(AgreementContentError::TooLong),
            ApiError::invalid_agreement_kind(),
            ApiError::invalid_agreement_keeper(),
            ApiError::invalid_agreement_ending(),
            ApiError::agreement_limit_reached(),
            ApiError::agreement_not_found(),
            ApiError::agreement_not_active(),
            // **提案の採否**（作業単位 A-12c-1）。終わらせる口の衝突（`agreement_not_active`）と潰さない。
            ApiError::agreement_not_proposed(),
        ];

        for (i, left) in errors.iter().enumerate() {
            for right in &errors[i + 1..] {
                assert_ne!(left, right, "同じ応答に潰さないこと");
                assert_ne!(
                    (left.message, left.code),
                    (right.message, right.code),
                    "文言か code のどちらかで区別できること"
                );
            }
        }

        // ステータスの割り当て。
        assert_eq!(
            ApiError::character_already_exists().status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ApiError::character_not_found().status(),
            StatusCode::NOT_FOUND
        );
        // **削除の2つの拒否は、状態の違いとして 409 / 404 に分かれる。**
        assert_eq!(
            ApiError::character_has_sessions().status(),
            StatusCode::CONFLICT
        );
        assert_ne!(
            ApiError::character_has_sessions(),
            ApiError::character_not_found(),
            "打ち間違いと仕様上の拒否を同じ応答に潰さないこと"
        );
        // **上限超過は 400 ではない。** 「JSON が壊れている」と読める応答に潰さない。
        assert_eq!(
            ApiError::body_too_large().status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        for error in [
            ApiError::invalid_body(),
            ApiError::invalid_path(),
            ApiError::invalid_character_handle(HandleError::Empty),
            ApiError::invalid_display_name(DisplayNameError::Empty),
            ApiError::invalid_display_name(DisplayNameError::TooLong),
            ApiError::invalid_stat_axis(AxisError::Empty),
            ApiError::invalid_persona(PersonaError::Empty),
            ApiError::invalid_persona(PersonaError::TooLong),
            ApiError::invalid_output_format(OutputFormatError::Empty),
            ApiError::invalid_output_format(OutputFormatError::TooLong),
            ApiError::duplicate_stat_axis(),
            ApiError::invalid_attribute_key(AttributeKeyError::TooLong),
            ApiError::invalid_attribute_value(AttributeValueError::TooLong),
            ApiError::invalid_attribute_subject(),
            ApiError::too_many_attributes(AttributeSetError::TooMany),
            ApiError::duplicate_attribute(),
            ApiError::invalid_stat_drift_mode(),
            ApiError::invalid_axis_discovery_slots(AxisDiscoverySlotsError::TooLarge),
            ApiError::invalid_agreement_content(AgreementContentError::TooLong),
            ApiError::invalid_agreement_kind(),
            ApiError::invalid_agreement_keeper(),
            ApiError::invalid_agreement_ending(),
        ] {
            assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        }
        // **取り決めの状態との衝突は 409、指した先が無いのは 404**（作業単位 A-12a）。
        assert_eq!(
            ApiError::agreement_limit_reached().status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ApiError::agreement_not_active().status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ApiError::agreement_not_proposed().status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ApiError::agreement_not_found().status(),
            StatusCode::NOT_FOUND
        );
    }

    /// **差し替えの拒否は、互いにも、他の管理 API の拒否とも区別できる**（作業単位 F-4b-2）。
    ///
    /// **とくに2つの 409 を潰さない** —— 「進行中」は待てば通り、「差し替えられない」は永久に通らない。
    #[test]
    fn upstream_switch_errors_are_all_distinguishable() {
        let switch = [
            ApiError::upstream_switch_in_progress(),
            ApiError::upstream_switch_unavailable(),
            ApiError::upstream_profile_not_found(),
            ApiError::upstream_profiles_unreadable(),
            ApiError::upstream_profile_unusable(),
            ApiError::upstream_gguf_missing(),
            ApiError::upstream_port_collision(),
            ApiError::upstream_profile_name_invalid(),
            ApiError::upstream_profiles_read_only(),
            ApiError::upstream_profiles_changed(),
            ApiError::upstream_profile_invalid(),
            ApiError::upstream_profiles_write_failed(),
            ApiError::upstream_profiles_verify_failed(),
        ];
        let others = [
            ApiError::not_found(),
            ApiError::invalid_body(),
            ApiError::character_not_found(),
            ApiError::character_already_exists(),
            ApiError::character_has_sessions(),
            ApiError::upstream_unavailable(),
            ApiError::upstream_failed(),
        ];
        for (i, left) in switch.iter().enumerate() {
            for right in switch[i + 1..].iter().chain(others.iter()) {
                assert_ne!(
                    (left.message, left.code),
                    (right.message, right.code),
                    "文言か code のどちらかで区別できること"
                );
            }
        }

        assert_eq!(
            ApiError::upstream_switch_in_progress().status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ApiError::upstream_switch_unavailable().status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ApiError::upstream_profile_not_found().status(),
            StatusCode::NOT_FOUND
        );
        // **画面からの保存**（作業単位 F-4b-3）。
        assert_eq!(
            ApiError::upstream_profile_name_invalid().status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ApiError::upstream_profiles_read_only().status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ApiError::upstream_profiles_changed().status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ApiError::upstream_profiles_write_failed().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            ApiError::upstream_profiles_verify_failed().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        for error in [
            ApiError::upstream_profiles_unreadable(),
            ApiError::upstream_profile_unusable(),
            ApiError::upstream_gguf_missing(),
            ApiError::upstream_port_collision(),
            ApiError::upstream_profile_invalid(),
        ] {
            assert_eq!(
                error.status(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "{error:?}"
            );
        }
    }

    /// **待ち受けのポートの保存の拒否は、互いにも、先例の拒否とも区別でき、状態は先例に倣う**
    /// （ADR-0057 §決定6 / 作業単位 F-12a）。
    ///
    /// **先例（`invalid_max_tokens` / `upstream_profiles_read_only`）と同じ応答に潰さない**——
    /// 状態は同じでも `code` で分かれること。
    #[test]
    fn listening_port_errors_are_distinguishable_and_follow_their_precedents() {
        let port = [
            ApiError::invalid_listening_port(),
            ApiError::listening_port_read_only(),
            ApiError::listening_port_write_failed(),
        ];
        let others = [
            ApiError::invalid_body(),
            ApiError::invalid_max_tokens(),
            ApiError::upstream_profiles_read_only(),
            ApiError::upstream_profiles_write_failed(),
            ApiError::internal(),
        ];
        for (i, left) in port.iter().enumerate() {
            for right in port[i + 1..].iter().chain(others.iter()) {
                assert_ne!(left.code, right.code, "{left:?} / {right:?}");
                assert_ne!(left.message, right.message, "{left:?} / {right:?}");
            }
        }
        assert_eq!(
            ApiError::invalid_listening_port().status(),
            ApiError::invalid_max_tokens().status()
        );
        assert_eq!(
            ApiError::invalid_listening_port().status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ApiError::listening_port_read_only().status(),
            ApiError::upstream_profiles_read_only().status()
        );
        assert_eq!(
            ApiError::listening_port_read_only().status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ApiError::listening_port_write_failed().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    /// **属性のエラーは、対応する性格ステータスのエラーと別物**（作業単位 12a）。
    ///
    /// 上の総当たりは「同じ応答に潰していない」ことしか見ない。
    /// ここは**取り違えの向き**を名指しで固定する——`AttributeKeyError` を
    /// `invalid_stat_axis` に流す実装は、`admin_errors_are_all_distinguishable` では
    /// 検出できない（どちらも定義済みの別値のままなので）。
    #[test]
    fn attribute_errors_never_read_as_stat_errors() {
        assert_ne!(
            ApiError::invalid_attribute_key(AttributeKeyError::TooLong),
            ApiError::invalid_stat_axis(AxisError::TooLong),
            "属性のキーと性格ステータスの軸名を同じ応答にしないこと"
        );
        assert_ne!(
            ApiError::duplicate_attribute(),
            ApiError::duplicate_stat_axis(),
            "重複の応答も別であること"
        );
        // **`?` の変換先も名指しで固定する**（`From` の実装を取り違えると
        // 「キーの誤りが軸名の誤りとして返る」）。
        assert_eq!(
            ApiError::from(AttributeKeyError::Control),
            ApiError::invalid_attribute_key(AttributeKeyError::Control)
        );
        assert_eq!(
            ApiError::from(AttributeValueError::Empty),
            ApiError::invalid_attribute_value(AttributeValueError::Empty)
        );
        assert_eq!(
            ApiError::from(AttributeSetError::TooMany),
            ApiError::too_many_attributes(AttributeSetError::TooMany)
        );
        // **キーと値も入れ替わらない。**
        assert_ne!(
            ApiError::from(AttributeKeyError::TooLong),
            ApiError::invalid_attribute_value(AttributeValueError::TooLong)
        );
    }

    /// `?` で変換できる（`domain` のエラーを握り潰さずに応答へ写す）。
    #[test]
    fn domain_errors_convert_without_losing_the_reason() {
        assert_eq!(
            ApiError::from(HandleError::AtSign),
            ApiError::invalid_character_handle(HandleError::AtSign)
        );
        assert_eq!(
            ApiError::from(AxisError::Control),
            ApiError::invalid_stat_axis(AxisError::Control)
        );
        assert_eq!(
            ApiError::from(PersonaError::TooLong),
            ApiError::invalid_persona(PersonaError::TooLong)
        );
        assert_ne!(
            ApiError::from(HandleError::Empty),
            ApiError::from(AxisError::Empty),
            "識別子と軸名のエラーを取り違えないこと"
        );
        assert_eq!(
            ApiError::from(OutputFormatError::TooLong),
            ApiError::invalid_output_format(OutputFormatError::TooLong)
        );
        assert_ne!(
            ApiError::from(HandleError::Empty),
            ApiError::from(PersonaError::Empty),
            "識別子とペルソナのエラーを取り違えないこと"
        );
        assert_ne!(
            ApiError::from(PersonaError::TooLong),
            ApiError::from(OutputFormatError::TooLong),
            "ペルソナと形式指示のエラーを取り違えないこと"
        );
        assert_eq!(
            ApiError::from(DisplayNameError::Control),
            ApiError::invalid_display_name(DisplayNameError::Control)
        );
        // **識別子と表示名のエラーを取り違えない**（作成要求には両方が載る）。
        for (name_reason, display_reason) in [
            (HandleError::Empty, DisplayNameError::Empty),
            (HandleError::Control, DisplayNameError::Control),
            (HandleError::TooLong, DisplayNameError::TooLong),
        ] {
            assert_ne!(
                ApiError::from(name_reason),
                ApiError::from(display_reason),
                "どちらの欄が悪いのか応答から読めること: {name_reason:?}"
            );
        }
    }

    /// `/v1/chat/completions` のエラー一式。
    fn chat_errors() -> Vec<ApiError> {
        vec![
            ApiError::invalid_model(SelectorError::EmptySuffix),
            ApiError::invalid_model(SelectorError::InvalidSuffix),
            ApiError::model_not_found(),
            ApiError::model_archived(),
            ApiError::session_not_found(),
            ApiError::no_user_message(),
            ApiError::unsupported_message_content(),
            ApiError::invalid_max_tokens(),
            ApiError::prompt_budget_exhausted(),
            ApiError::upstream_unavailable(),
            ApiError::upstream_timeout(),
            ApiError::upstream_failed(),
            ApiError::upstream_unauthorized(),
            ApiError::upstream_key_not_approved(),
        ]
    }

    /// チャット経路のエラーも**すべて互いに区別できる**（同じ応答に潰さない）。
    ///
    /// 特に次の3組を潰すと、原因に到達できない形の壊れ方をする:
    ///
    /// * `invalid_model(*)` と `model_not_found` — `aoi@new ` が
    ///   「そんなキャラクターは無い」に化ける（ADR-0004）。
    /// * `model_not_found` と `session_not_found` — 原因が名前側か id 側か分からない。
    /// * `upstream_unavailable` と `upstream_failed` — LM Studio の起動を促せない。
    #[test]
    fn chat_errors_are_all_distinguishable() {
        let errors = chat_errors();

        for (i, left) in errors.iter().enumerate() {
            for right in &errors[i + 1..] {
                assert_ne!(left, right, "同じ応答に潰さないこと");
                assert_ne!(
                    (left.message, left.code),
                    (right.message, right.code),
                    "文言か code のどちらかで区別できること"
                );
            }
        }

        // **`model` の解釈の失敗を 404 に潰さない。**
        for reason in [
            SelectorError::EmptySuffix,
            SelectorError::InvalidSuffix,
            SelectorError::Handle(HandleError::Empty),
            SelectorError::Handle(HandleError::TooLong),
            SelectorError::Handle(HandleError::Control),
            SelectorError::Handle(HandleError::AtSign),
        ] {
            let error = ApiError::invalid_model(reason);
            assert_eq!(
                error.status(),
                StatusCode::BAD_REQUEST,
                "reason = {reason:?}"
            );
            assert_ne!(error, ApiError::model_not_found(), "reason = {reason:?}");
        }
        // 名前部の理由は握り潰さず、そのまま伝える。
        assert_eq!(
            ApiError::invalid_model(SelectorError::Handle(HandleError::TooLong)),
            ApiError::invalid_character_handle(HandleError::TooLong)
        );

        // ステータスの割り当て。
        assert_eq!(ApiError::model_not_found().status(), StatusCode::NOT_FOUND);
        // **しまったキャラクターは 409**（居るが、いまは話せない / ADR-0049 §決定1）。
        assert_eq!(ApiError::model_archived().status(), StatusCode::CONFLICT);
        assert_eq!(
            ApiError::session_not_found().status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            ApiError::no_user_message().status(),
            StatusCode::BAD_REQUEST
        );
        // **`invalid_body` にも `no_user_message` にも潰さない。**
        assert_eq!(
            ApiError::unsupported_message_content().status(),
            StatusCode::BAD_REQUEST
        );
        assert_ne!(
            ApiError::unsupported_message_content(),
            ApiError::invalid_body(),
            "「本文が壊れている」と「この形式は未対応」を同じ応答にしないこと"
        );
        assert_ne!(
            ApiError::unsupported_message_content(),
            ApiError::no_user_message(),
            "user の発言は在るので「1件も無い」と返さないこと"
        );
        assert_eq!(
            ApiError::invalid_max_tokens().status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ApiError::prompt_budget_exhausted().status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            ApiError::upstream_unavailable().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            ApiError::upstream_timeout().status(),
            StatusCode::GATEWAY_TIMEOUT
        );
        assert_eq!(
            ApiError::upstream_failed().status(),
            StatusCode::BAD_GATEWAY
        );
        // **鍵を断られたのは personacore の先の上流**（ADR-0056 §決定4）。401 にすると、
        // クライアントは「personacore への鍵が違う」と読む。
        assert_eq!(
            ApiError::upstream_unauthorized().status(),
            StatusCode::BAD_GATEWAY
        );
        // **承認が無くて送らなかったのも、personacore の先へ送る設定の話**（ADR-0056 §決定5）。
        assert_eq!(
            ApiError::upstream_key_not_approved().status(),
            StatusCode::BAD_GATEWAY
        );
    }

    /// **管理 API のエラーとチャットのエラーも取り違えない。**
    ///
    /// `character_not_found`（パスで指したキャラクター）と
    /// `model_not_found`（`model` フィールド）を同じ応答にすると、
    /// どちらの入り口の話なのかが応答から読めなくなる。
    #[test]
    fn chat_errors_never_collide_with_admin_errors() {
        let admin = [
            ApiError::internal(),
            ApiError::not_found(),
            ApiError::method_not_allowed(),
            ApiError::invalid_body(),
            ApiError::body_too_large(),
            ApiError::invalid_path(),
            ApiError::character_not_found(),
            ApiError::character_already_exists(),
            ApiError::character_has_sessions(),
            ApiError::duplicate_stat_axis(),
            ApiError::duplicate_attribute(),
            ApiError::invalid_attribute_subject(),
            ApiError::too_many_attributes(AttributeSetError::TooMany),
        ];

        for left in &chat_errors() {
            for right in &admin {
                assert_ne!(left, right, "同じ応答に潰さないこと");
                assert_ne!(
                    (left.message, left.code),
                    (right.message, right.code),
                    "文言か code のどちらかで区別できること"
                );
            }
        }
        assert_ne!(
            ApiError::model_not_found(),
            ApiError::character_not_found(),
            "どちらの入り口の 404 かが分かること"
        );
    }

    /// 上流エラーの本文は**定数のみ**（`{err:?}` もステータス値も載らない）。
    #[tokio::test]
    async fn upstream_error_bodies_are_constant() {
        let (status, body) = parts(ApiError::upstream_failed()).await;

        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(
            body,
            json!({
                "error": {
                    "message": "推論サーバとの通信に失敗しました",
                    "type": "server_error",
                    "code": "upstream_failed",
                }
            })
        );
    }

    /// **足した欄は定数のまま載り、`null` も鍵ごと残り、既存の鍵は上書きされない**（作業単位 F-4b-3）。
    #[tokio::test]
    async fn extra_details_ride_along_without_overwriting_the_error() {
        let response = ApiError::upstream_profile_invalid().into_response_with(&[
            ("reason", Some("endpoint_not_loopback")),
            ("face", Some("sub")),
            ("key", None),
            ("code", Some("planted-overwrite")),
        ]);
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("本文を読めること");
        let body: Value = serde_json::from_slice(&bytes).expect("JSON であること");

        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            body,
            json!({
                "error": {
                    "message": "上流プロファイルの形が不正なので保存しませんでした",
                    "type": "invalid_request_error",
                    "code": "upstream_profile_invalid",
                    "reason": "endpoint_not_loopback",
                    "face": "sub",
                    "key": null,
                }
            })
        );
    }

    /// **SSE へ流す error オブジェクトは、通常の応答本文と完全に同じ。**
    ///
    /// 200 を送った後は 4xx/5xx で伝えられないので、SSE では
    /// `as_event_data` だけが理由の運び手になる（`server::relay` の doc）。
    /// ここが `into_response` と食い違うと、**同じ失敗が経路によって別の形で届く**
    /// ——クライアント側は片方しか解釈できない。
    #[tokio::test]
    async fn the_event_data_matches_the_response_body() {
        for error in [
            ApiError::upstream_unavailable(),
            ApiError::upstream_timeout(),
            ApiError::upstream_failed(),
            ApiError::upstream_unauthorized(),
            ApiError::upstream_key_not_approved(),
            // `code` を持たない側（キーごと落ちること）。
            ApiError::internal(),
            ApiError::not_found(),
        ] {
            let (_, body) = parts(error).await;
            let event: Value =
                serde_json::from_str(&error.as_event_data()).expect("JSON であること");

            assert_eq!(event, body, "応答本文と同じ定数を流すこと");
        }

        // `code` の無いものはキーごと落ちる（`null` を書かない）。
        let internal: Value =
            serde_json::from_str(&ApiError::internal().as_event_data()).expect("JSON であること");
        assert!(
            internal["error"].get("code").is_none(),
            "code が無いときはキーごと落とすこと: {internal}"
        );
    }

    /// 種別ごとにステータスと `type` が分かれている。
    #[test]
    fn kinds_are_distinguishable() {
        let errors = [
            ApiError::internal(),
            ApiError::not_found(),
            ApiError::method_not_allowed(),
        ];
        for (i, left) in errors.iter().enumerate() {
            for right in &errors[i + 1..] {
                assert_ne!(left, right);
            }
        }
        assert_eq!(
            ApiError::internal().status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(ApiError::not_found().status(), StatusCode::NOT_FOUND);
        assert_eq!(
            ApiError::method_not_allowed().status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
}

/// **[`error_catalog`] の網羅を見張る**（構築子の一覧は型では強制できない / 作業単位 O-7）。
///
/// | 主張 | 破れると |
/// |---|---|
/// | **`ApiError` の値を書く場所**（構造体リテラル —— `ApiError { … }` と、`Self` が `ApiError` の `impl` の中の `Self { … }` —— と、欄への代入 `.status =` / `.message =` / `.kind =` / `.code =`）を囲む `fn` の名前が、どれも一覧に在る | 構築子を足して一覧に足し忘れると、その失敗が指紋に出ない |
/// | 本体（`#[cfg(test)]` より前）の `snake_case` の文字列リテラルの集合 == 一覧の `code` と `type` の集合 ＋ 理由つきの例外（`NOT_CODES`） | 既存の構築子に `code` の綴りを足して（理由の分岐を増やして）一覧に足し忘れると、同上 |
///
/// # 1本目の走査が `impl` の種類も返り値の綴りも問わない理由（O-7 の再レビュー）
///
/// 最初の形は「最初の `impl ApiError {` の中の `fn … -> Self` の名前」を見ていたので、
/// `-> ApiError` と書く・2つ目の `impl ApiError` に置く・`impl From<…> for ApiError` に直接リテラルを書く・
/// `-> Option<Self>` を返す・マクロで生やす、のどれでも一覧の外に出られた。**既にある綴りだけで組めば、
/// 2本目の走査も緑のまま通る**（例: `character_not_found` の code を 409 で返す構築子）。
/// **値を書く場所から辿る**ので、どの書き方でも引っかかる。`fn` の外（`const` 等）で書いたら赤
/// （`<fn の外>`）。マクロで生やした `fn $名前` も、`$名前` が一覧に無いので赤。
///
/// **走査するのはこのファイルだけでよい** —— `ApiError` の欄は非公開なので、このモジュールの外では値を書けない。
///
/// **⚠ 共通の組み立て関数（引数で code を受ける `const fn make(…) -> Self`）は作れない** —— 作ると
/// その名前が一覧に載らず赤になる（載せる手段も無い）。網羅を見張るための制約である。
///
/// # 字句の扱い
///
/// **走査は文字列・文字リテラル・コメントを分けてから読む**（CLAUDE.md §4「ソースを数える道具は…」）。
/// 通常の文字列・生文字列（`r"…"` / `r#"…"#`）・入れ子のブロックコメント・行コメント・
/// 文字リテラル（`'"'` / `'\''` / `'}'`）とライフタイム（`'static`）を分ける。
/// 植えたソースを走査本体に通す対照を対にする。
#[cfg(test)]
mod catalog_tests {
    use std::collections::BTreeSet;

    use super::error_catalog;

    /// 値を書く場所が `fn` の外にあったときの印。
    const OUTSIDE_FN: &str = "<fn の外>";

    /// `ApiError` の欄の名前（代入を探す）。
    const FIELDS: [&str; 4] = ["status", "message", "kind", "code"];

    /// リテラルの走査で、`code` でも `type` でもないと分かっている綴り（理由つき）。
    const NOT_CODES: [(&str, &str); 5] = [
        ("error", "本文の鍵"),
        ("message", "本文の鍵"),
        ("type", "本文の鍵"),
        ("code", "本文の鍵"),
        (
            "testing",
            "`#[cfg(any(test, feature = \"testing\"))]` のフィーチャ名",
        ),
    ];

    /// ソースの断片。
    #[derive(Debug, PartialEq, Eq)]
    enum Token {
        /// 文字列リテラルの中身（エスケープは解かない）。
        Str(String),
        /// コード（コメントと文字列を除いたもの）。1文字ずつ。
        Code(char),
    }

    fn is_ident_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }

    /// `chars[i]` から生文字列（`r"…"` / `r#"…"#` / `br"…"`）が始まるなら、`(中身, 次の位置)`。
    fn raw_string(chars: &[char], i: usize) -> Option<(String, usize)> {
        if i > 0 && is_ident_char(chars[i - 1]) && chars[i - 1] != 'b' {
            return None;
        }
        if i > 1 && chars[i - 1] == 'b' && is_ident_char(chars[i - 2]) {
            return None;
        }
        if chars.get(i) != Some(&'r') {
            return None;
        }
        let mut j = i + 1;
        let mut hashes = 0;
        while chars.get(j) == Some(&'#') {
            hashes += 1;
            j += 1;
        }
        if chars.get(j) != Some(&'"') {
            return None;
        }
        j += 1;
        let start = j;
        while j < chars.len() {
            if chars[j] == '"' && (1..=hashes).all(|k| chars.get(j + k) == Some(&'#')) {
                return Some((chars[start..j].iter().collect(), j + 1 + hashes));
            }
            j += 1;
        }
        Some((chars[start..].iter().collect(), chars.len()))
    }

    /// **コメント・文字列・文字リテラルを分ける。** ライフタイム（`'static`）は文字リテラルとして読まない。
    /// コメントは空白1つにする（前後の綴りを繋げない）。ブロックコメントの入れ子を数える。
    fn tokens(source: &str) -> Vec<Token> {
        let chars: Vec<char> = source.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            if c == '/' && next == Some('/') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            } else if c == '/' && next == Some('*') {
                i += 2;
                let mut depth = 1usize;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                out.push(Token::Code(' '));
            } else if let Some((text, end)) = raw_string(&chars, i) {
                i = end;
                out.push(Token::Str(text));
            } else if c == '"' {
                let mut text = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        text.push(chars[i]);
                        i += 1;
                    }
                    text.push(chars[i]);
                    i += 1;
                }
                i += 1;
                out.push(Token::Str(text));
            } else if c == '\'' {
                // 文字リテラル（`'x'` / `'\n'` / `'\''` / `'\u{…}'`）なら飛ばす。それ以外はライフタイム。
                // **エスケープの次の1文字は必ず中身**（`'\''` の2つ目の `'` で閉じない）。
                if next == Some('\\') {
                    i += 3;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if chars.get(i + 2) == Some(&'\'') {
                    i += 3;
                } else {
                    out.push(Token::Code(c));
                    i += 1;
                }
            } else {
                out.push(Token::Code(c));
                i += 1;
            }
        }
        out
    }

    /// テストより前の本体（**最初の `#[cfg(test)]` の行より前**）。
    fn body(source: &str) -> &str {
        source
            .find("\n#[cfg(test)]")
            .map_or(source, |end| &source[..end])
    }

    /// 本体の `snake_case` の文字列リテラル（`[a-z0-9_]` だけからなる、空でないもの）。
    fn snake_literals(source: &str) -> BTreeSet<String> {
        tokens(body(source))
            .into_iter()
            .filter_map(|token| match token {
                Token::Str(text)
                    if !text.is_empty()
                        && text
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') =>
                {
                    Some(text)
                }
                _ => None,
            })
            .collect()
    }

    /// 本体のコード（文字列は `"` の1文字、コメントは空白）を、語と記号の列にする。
    /// **`$` も語の文字に入れる**（マクロの `fn $名前` を1語で読む）。
    fn words(source: &str) -> Vec<String> {
        let code: String = tokens(body(source))
            .into_iter()
            .map(|token| match token {
                Token::Code(c) => c,
                Token::Str(_) => '"',
            })
            .collect();
        let mut out = Vec::new();
        let mut current = String::new();
        for c in code.chars() {
            if is_ident_char(c) || c == '$' {
                current.push(c);
                continue;
            }
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            if !c.is_whitespace() {
                out.push(c.to_string());
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
        out
    }

    /// 開いている `{` が何か。
    enum Frame {
        Impl(String),
        Fn(String),
        Other,
    }

    /// `impl … for 型` / `impl 型` の見出しの語から、型の名前（最後の区切りの後ろ、総称を除く）。
    fn impl_type(header: &[String]) -> String {
        let after = header
            .iter()
            .rposition(|word| word == "for")
            .map_or(header, |at| &header[at + 1..]);
        let mut depth = 0i32;
        let mut name = String::new();
        for word in after {
            match word.as_str() {
                "<" => depth += 1,
                ">" => depth -= 1,
                _ if depth == 0 && word.chars().all(|c| is_ident_char(c) || c == '$') => {
                    name.clone_from(word);
                }
                _ => {}
            }
        }
        name
    }

    /// **`ApiError` の値を書く場所を囲む `fn` の名前**（現れた順。`fn` の外なら [`OUTSIDE_FN`]）。
    fn builders(source: &str) -> Vec<String> {
        let words = words(source);
        let mut stack: Vec<Frame> = Vec::new();
        let mut pending_fn: Option<String> = None;
        let mut pending_impl: Option<Vec<String>> = None;
        let mut found = Vec::new();
        let enclosing_fn = |stack: &[Frame]| -> String {
            stack
                .iter()
                .rev()
                .find_map(|frame| match frame {
                    Frame::Fn(name) => Some(name.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| OUTSIDE_FN.to_owned())
        };
        let self_is_api_error = |stack: &[Frame]| -> bool {
            stack
                .iter()
                .rev()
                .find_map(|frame| match frame {
                    Frame::Impl(name) => Some(name == "ApiError"),
                    _ => None,
                })
                .unwrap_or(false)
        };
        for (k, word) in words.iter().enumerate() {
            let before = |back: usize| k.checked_sub(back).map(|at| words[at].as_str());
            match word.as_str() {
                // **署名の中の `impl Trait` は `impl` の見出しではない。**
                "impl" if pending_fn.is_none() => pending_impl = Some(Vec::new()),
                "fn" => {
                    pending_fn = words.get(k + 1).cloned();
                    pending_impl = None;
                }
                "{" => {
                    if let Some(header) = pending_impl.take() {
                        stack.push(Frame::Impl(impl_type(&header)));
                    } else if let Some(name) = pending_fn.take() {
                        stack.push(Frame::Fn(name));
                    } else {
                        let literal = match before(1) {
                            Some("Self") => self_is_api_error(&stack),
                            Some("ApiError") => !matches!(
                                before(2),
                                Some("struct" | "enum" | "union" | "impl" | "for")
                            ),
                            _ => false,
                        };
                        if literal {
                            found.push(enclosing_fn(&stack));
                        }
                        stack.push(Frame::Other);
                    }
                }
                "}" => {
                    stack.pop();
                }
                ";" => pending_fn = None,
                "=" if FIELDS.contains(&before(1).unwrap_or(""))
                    && before(2) == Some(".")
                    && words.get(k + 1).map(String::as_str) != Some("=") =>
                {
                    found.push(enclosing_fn(&stack));
                }
                _ => {
                    if let Some(header) = pending_impl.as_mut() {
                        header.push(word.clone());
                    }
                }
            }
        }
        found
    }

    const SOURCE: &str = include_str!("error.rs");

    /// **`ApiError` の値を書く場所を囲む `fn` が、どれも一覧に在る。**
    #[test]
    fn every_place_that_builds_an_api_error_is_in_the_catalog() {
        let found = builders(SOURCE);
        assert!(
            found.len() >= 70,
            "前提: 走査が値を書く場所を読めていること（{}）",
            found.len()
        );
        let listed: BTreeSet<String> = error_catalog()
            .into_iter()
            .map(|(name, _)| name.to_owned())
            .collect();
        let missing: BTreeSet<&String> = found
            .iter()
            .filter(|name| !listed.contains(*name))
            .collect();
        assert!(
            missing.is_empty(),
            "ApiError の値を書く関数が、失敗の一覧（error_catalog）に無い: {missing:?}"
        );
    }

    /// **本体の `snake_case` のリテラル == 一覧の `code` と `type` ＋ 理由つきの例外。**
    #[test]
    fn every_code_spelled_in_the_source_is_in_the_catalog() {
        let catalog = error_catalog();
        let mut listed: BTreeSet<String> = catalog
            .iter()
            .flat_map(|(_, shape)| shape.code.into_iter().chain([shape.kind]))
            .map(str::to_owned)
            .collect();
        listed.extend(NOT_CODES.iter().map(|(key, _)| (*key).to_owned()));
        let spelled = snake_literals(SOURCE);
        assert!(
            spelled.len() >= 80,
            "前提: 走査がリテラルを読めていること（{}）",
            spelled.len()
        );
        assert_eq!(spelled, listed);
    }

    /// **対照: 値を書く場所の走査が、植えたソースのどの書き方も拾う**（再レビューが挙げた抜け道を全部植える）。
    ///
    /// 拾う: `impl ApiError` の `Self { … }`・2つ目の `impl ApiError` の `-> ApiError` と `ApiError { …, .. }`・
    /// `impl From<…> for ApiError` の直書き・`-> Option<Self>`・マクロの `fn $名前`・欄への代入・`fn` の外の `const`。
    /// 拾わない: 型の定義・別の型の `impl` の `Self { … }`・委譲（`Self::alpha()`）・比較（`==` / `!=`）・閉包の `{`・
    /// コメントと文字列の中・`#[cfg(test)]` の後ろ。CRLF。
    #[test]
    fn the_builder_scan_reads_a_planted_source() {
        let planted = concat!(
            "pub struct ApiError { status: StatusCode, code: Option<&'static str> }\r\n",
            "pub struct ErrorShape { status: u16 }\r\n",
            "impl ApiError {\r\n",
            "    // fn commented() -> Self { Self { status: S } }\r\n",
            "    /* ApiError { status: S } /* 入れ子 */ Self { status: S } */\r\n",
            "    pub const fn alpha() -> Self { Self { status: S, code: Some(\"a\") } }\r\n",
            "    fn delegate() -> Self { Self::alpha() }\r\n",
            "    fn maybe(x: bool) -> Option<Self> { if x { Some(Self { status: S, code: None }) } else { None } }\r\n",
            "    fn reads(self) -> bool { let f = |v: u8| { v }; self.code == None && self.status != S }\r\n",
            "    fn text() -> &'static str { \"Self { status: S }\" }\r\n",
            "}\r\n",
            "impl ApiError {\r\n",
            "    pub const fn conflict() -> ApiError { ApiError { status: S, ..ApiError::alpha() } }\r\n",
            "}\r\n",
            "impl From<X> for ApiError { fn from(x: X) -> Self { Self { status: S, code: None } } }\r\n",
            "impl From<ApiError> for ErrorShape { fn from(e: ApiError) -> Self { Self { status: 1 } } }\r\n",
            "macro_rules! make { ($n:ident) => { impl ApiError { pub const fn $n() -> Self { Self { status: S } } } } }\r\n",
            "fn tweak(mut e: ApiError) -> ApiError { e.code = Some(\"x\"); e }\r\n",
            "const BAD: ApiError = ApiError { status: S, code: None };\r\n",
            "#[cfg(test)]\r\n",
            "mod tests { fn after() -> ApiError { ApiError { status: S } } }\r\n",
        );
        assert_eq!(
            builders(planted),
            vec![
                "alpha", "maybe", "conflict", "from", "$n", "tweak", OUTSIDE_FN
            ]
        );
    }

    /// **対照: リテラルの走査が、植えたソースの字句を分ける**（コメント（入れ子も）・生文字列・
    /// 文字リテラル（`'"'` / `'\''` / `'}'`）・ライフタイム・テストの後ろ。CRLF も）。
    ///
    /// **生文字列の中の `"` と、`'\''` の2つ目の `'` で判定が反転すると、後ろの綴りを見逃す**ので、
    /// その直後に拾うべき綴りを置く。
    #[test]
    fn the_literal_scan_reads_a_planted_source() {
        let planted = concat!(
            "pub struct ApiError { code: Option<&'static str> }\r\n",
            "// \"in_a_comment\"\r\n",
            "/* \"in_a_block\" /* \"in_a_nested_block\" */ \"still_in_the_block\" */\r\n",
            "const A: &str = \"alpha_code\";\r\n",
            // 中の `"` を奇数個にする（偶数個だと、生文字列を読めない走査でも偶然に揃って緑になる）。
            "const R: &str = r#\"say \"hi\"#; const AFTER_RAW: &str = \"after_raw\";\r\n",
            "const RAW_CODE: &str = r\"raw_code\";\r\n",
            // `'\''` の直後に空白を置かない（置くと、2つ目の `'` で閉じる走査でも偶然に揃って緑になる）。
            "const Q: [char; 2] = ['\\'','\"']; const AFTER_QUOTE: &str = \"after_quote_char\";\r\n",
            "const DQ: char = '\"'; const AFTER_DQ: &str = \"after_dq_char\";\r\n",
            "const B: char = '}'; const AFTER_BRACE: &str = \"after_brace_char\";\r\n",
            "const S: &str = \"Mixed_Case\";\r\n",
            "#[cfg(test)]\r\n",
            "mod tests { const T: &str = \"after_tests\"; }\r\n",
        );
        let literals: Vec<String> = snake_literals(planted).into_iter().collect();
        assert_eq!(
            literals,
            vec![
                "after_brace_char",
                "after_dq_char",
                "after_quote_char",
                "after_raw",
                "alpha_code",
                "raw_code",
            ]
        );
    }

    /// **一覧の値は構築子と同じ**（名前と形の帰属を、外の値で押さえる）。
    #[test]
    fn the_catalog_carries_what_the_constructors_return() {
        let catalog = error_catalog();
        let find = |name: &str| -> Vec<super::ErrorShape> {
            catalog
                .iter()
                .filter(|(listed, _)| *listed == name)
                .map(|(_, shape)| *shape)
                .collect()
        };
        assert_eq!(
            find("upstream_unauthorized"),
            vec![super::ErrorShape {
                status: 502,
                kind: "server_error",
                code: Some("upstream_unauthorized"),
            }]
        );
        assert_eq!(
            find("agreement_not_proposed"),
            vec![super::ErrorShape {
                status: 409,
                kind: "invalid_request_error",
                code: Some("agreement_not_proposed"),
            }]
        );
        // 理由ごとの構築子は、理由の数だけ載る（`HandleError` は5つ）。
        assert_eq!(find("invalid_character_handle").len(), 5);
        // `invalid_model` は名前部の5つ ＋ 接尾辞の2つ。
        assert_eq!(find("invalid_model").len(), 7);
    }
}
