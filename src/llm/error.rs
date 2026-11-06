//! `llm` のエラー型。
//!
//! **`Display` は全バリアントが定数文字列**（CLAUDE.md 絶対制約）。エンドポイント URL・
//! 応答本文・ステータス値のいずれも文言に埋め込まない。詳細は `#[source]` と
//! `tracing` のログ側に残す。`server` 層はここからステータスをマップするので、
//! 数値が要る場合は `LlmError::status` を使う（文言からは取れない）。

/// LM Studio との通信・応答の解釈で起こりうる失敗。
///
/// **上流の応答は「外部からの信頼できない入力」**として扱う。どの経路も panic せず、
/// この型のいずれかに落ちる。
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    /// 上流へ接続できなかった。
    ///
    /// **これは異常ではない。** LM Studio は GPU を占有し続けないよう普段は停止しており
    /// （docs/04-environment.md）、「落ちている」は想定された分岐。
    /// 他の失敗と同じ扱いにすると、起動を促す案内が出せなくなるので分けてある。
    #[error("推論サーバへ接続できませんでした")]
    Connect(#[source] reqwest::Error),

    /// 制限時間内に応答が完了しなかった。
    ///
    /// 接続はできているので `Connect` とは別物。モデルの JIT ロード待ちで
    /// 数十秒かかることがあるため、上限は設定で変えられる（`LmStudioConfig`）。
    #[error("推論サーバの応答が時間内に完了しませんでした")]
    Timeout(#[source] reqwest::Error),

    /// 接続後の送受信に失敗した（本文の途中で切れた等）。クライアント構築の失敗も含む。
    #[error("推論サーバとの通信に失敗しました")]
    Transport(#[source] reqwest::Error),

    /// 上流が 2xx 以外を返した。
    ///
    /// **`Display` にステータスを埋め込まない。** 文言は定数のままにして、
    /// 数値は `status()` で取り出す。応答本文は `tracing::warn!` にだけ出す。
    #[error("推論サーバがエラーを返しました")]
    Status {
        /// 上流の HTTP ステータス。
        status: u16,
    },

    /// 応答本文を JSON として解釈できなかった。
    #[error("推論サーバの応答を解釈できませんでした")]
    Decode(#[source] serde_json::Error),

    /// SSE のフレーミングが壊れていた（UTF-8 不正・行の構文違反）。
    ///
    /// `Decode`（JSON の破損）とは層が違う。原因の詳細は `tracing::warn!` に出す。
    #[error("推論サーバのストリームが壊れています")]
    MalformedEvent,

    /// SSE のイベントに中身が無かった（`data:` が空白のみ）。
    ///
    /// **`Decode` と分ける。** 「壊れた JSON が来た」と「そもそも中身が無い」は
    /// プロンプト側の問題か上流の問題かの切り分けが変わる。
    #[error("推論サーバが空のチャンクを返しました")]
    EmptyPayload,

    /// 応答に本文が含まれていなかった（`choices` が空、または `content` が無い）。
    ///
    /// 空文字の本文（`content: ""`）は**正当な応答として受け取る**（これには落ちない）。
    /// 「モデルが何も言わなかった」こと自体が調査材料になるため。
    #[error("推論サーバの応答に本文がありません")]
    MissingContent,

    /// 上流の一覧に、設定中のモデルの**使えるコンテキスト長**が無かった。
    ///
    /// 「一覧に居ない」「ロードされていない」「値が欠けている・型が違う・0・巨大すぎる」を
    /// まとめてここに落とす。**どれも起動を止める理由にはならない**
    /// （ADR-0013 第2層は「取れたら得をする」経路で、取れなければ既定値へ落ちる）。
    /// 区別が要る調査は `tracing::warn!` 側に残る。
    #[error("推論サーバからコンテキスト長を取得できませんでした")]
    MissingContextLength,

    /// `[DONE]` を受け取る前に上流のストリームが終わった。
    ///
    /// 下流を宙吊りにしないため、**この失敗を1件流してからストリームを終端する**。
    /// ここまでに受け取ったチャンクは有効なので、呼び出し側は蓄積分を保存してよい。
    #[error("推論サーバのストリームが完了前に終了しました")]
    Truncated,

    /// **鍵を平文の `http://` でローカルネットワークの外へ送る承認が無いので、送らなかった**
    /// （ADR-0056 §決定5 / 作業単位 A-13b-2）。
    ///
    /// **接続する前に止めている**（IP アドレスの宛先は組んだときの判定で、ホスト名の宛先は接続のたびの
    /// 名前解決で）。**上流は鍵を見ていない**ので、[`Self::refused_the_key`] とは分ける。
    /// どのアドレスが外だったかは `tracing::warn!` に出す（ここには載せない）。
    #[error(
        "鍵を暗号化せずにローカルネットワークの外へ送る承認が無いので、推論サーバへ送りませんでした"
    )]
    KeyNotApproved,
}

impl LlmError {
    /// 上流の HTTP ステータス。`Status` 以外では `None`。
    ///
    /// `Display` に数値を出さない代わりの取り出し口。`server` 層が
    /// 下流へ返すステータスを決めるのに使う。
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status } => Some(*status),
            _ => None,
        }
    }

    /// **上流が鍵を断ったか**（401 / 403 / ADR-0056 §決定4）。
    ///
    /// 鍵が無い・違う・期限切れのどれでも真になる。**判定はここ1箇所**
    /// （応答の `upstream_unauthorized` と、上流の画面の `unauthorized` が同じ規則を読む）。
    #[must_use]
    pub const fn refused_the_key(&self) -> bool {
        matches!(self, Self::Status { status: 401 | 403 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Decode` の文言。**定数との完全一致で守る**（緩い検査では守れない。下記参照）。
    const DECODE_MESSAGE: &str = "推論サーバの応答を解釈できませんでした";

    /// `#[source]` を持たないバリアント。**追加したらここにも足すこと。**
    ///
    /// `#[source]` を持つのは4つ（`Connect` / `Timeout` / `Transport` / `Decode`）で、
    /// うち前3つは `reqwest::Error` を外から構築できないため**ここでは作れない。**
    /// その3つの文言は結合テスト（`tests/llm_lmstudio.rs` の
    /// `assert_constant_message`）が定数との完全一致で検査している。
    /// **こちらへ足そうとして手が止まったら、あちらを見ること。**
    ///
    /// 残る `Decode` は `serde_json::Error` なのでここで作れる。
    /// `decode_display_does_not_reflect_the_upstream_body` が担当する。
    fn sourceless_variants() -> Vec<LlmError> {
        vec![
            LlmError::Status { status: 503 },
            LlmError::MalformedEvent,
            LlmError::EmptyPayload,
            LlmError::MissingContent,
            LlmError::MissingContextLength,
            LlmError::Truncated,
            LlmError::KeyNotApproved,
        ]
    }

    /// `Display` は定数文字列。**ステータス値も応答本文も混ぜない。**
    #[test]
    fn display_is_a_constant_message() {
        for error in sourceless_variants() {
            let shown = error.to_string();
            assert!(!shown.is_empty(), "説明があること");
            assert!(
                !shown.contains("503"),
                "ステータスを埋め込まないこと: {shown}"
            );
            assert!(!shown.contains("http"), "URL を埋め込まないこと: {shown}");
        }
    }

    /// 文言が区別できる（同じ説明の使い回しをしない）。
    #[test]
    fn each_variant_has_its_own_message() {
        let messages: Vec<String> = sourceless_variants()
            .iter()
            .map(ToString::to_string)
            .collect();
        for (index, message) in messages.iter().enumerate() {
            for other in &messages[index + 1..] {
                assert_ne!(message, other, "説明が重複しないこと");
            }
        }
    }

    /// 上流の応答本文を反射する `serde_json::Error`。
    ///
    /// `serde_json` の型不一致エラーは**受け取った値をそのまま文言に載せる**ので、
    /// 上流（= 信頼できない入力）の中身がここから漏れうる。
    fn decode_failure() -> serde_json::Error {
        serde_json::from_str::<Vec<u8>>(r#""MODEL-secret-token-abc""#)
            .expect_err("配列でない値は Vec<u8> として解釈できないこと")
    }

    /// **`Decode` の `Display` に上流の応答本文を混ぜない。**
    ///
    /// `#[error("...: {0}")]` を1つ足すだけで、`serde_json::Error` の文言
    /// （`invalid type: string "..."` の形で**応答本文をそのまま反射する**）が
    /// エラーレスポンスに載る。CLAUDE.md の「エラーレスポンスに応答本文を含めない」に
    /// 直接抵触するので、**定数との完全一致**で止める。
    ///
    /// 「`choices` を含まない」のような緩い検査では素通りする
    /// （漏れる内容は上流が何を返したか次第で、特定の語では捕まえられない）。
    #[test]
    fn decode_display_does_not_reflect_the_upstream_body() {
        let source = decode_failure();
        // 前提の確認。ここが偽だと以下の検査は何も守っていない。
        assert!(
            source.to_string().contains("MODEL-secret-token-abc"),
            "source 自体は応答本文を反射すること（検査の前提）: {source}"
        );

        let error = LlmError::Decode(source);
        assert_eq!(
            error.to_string(),
            DECODE_MESSAGE,
            "Display は定数文字列であること（source を混ぜない）: {error:?}"
        );
    }

    /// `Decode` の文言は他のバリアントと重複しない。
    #[test]
    fn decode_has_its_own_message() {
        for other in sourceless_variants() {
            assert_ne!(other.to_string(), DECODE_MESSAGE, "説明が重複しないこと");
        }
    }

    /// ステータスは文言ではなく `status()` から取る。
    #[test]
    fn status_is_available_without_the_message() {
        assert_eq!(LlmError::Status { status: 404 }.status(), Some(404));
        assert_eq!(LlmError::Truncated.status(), None);
        assert_eq!(LlmError::EmptyPayload.status(), None);
    }
}
