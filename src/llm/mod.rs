//! 推論サーバ（LM Studio）との通信。
//!
//! **上位層は具象型ではなく `LlmClient` トレイトに依存する**（CLAUDE.md 5節）。
//! テストではモック実装を注入し、**実際の LM Studio には繋がない**
//! （docs/05-development.md）。
//!
//! # 信頼境界
//!
//! **このモジュールが「LLM 出力」という信頼できない入力の入口。**
//! 受け取った文字列を SQL・シェル・パスへ渡す API は**ここには存在しない**。
//! 値はただのデータとして上位へ返すだけで、扱いは呼び出し側に委ねる
//! （`store` はプレースホルダで束縛し、`parse` は台詞／地の文へ分解する）。
//!
//! 応答がどれだけ壊れていても panic せず、`LlmError` のいずれかに落ちる。
//!
//! # モデルの指定
//!
//! 上流のモデルを決めるのは `LmStudioConfig`（設定）だけで、`ChatRequest` からは
//! 差し込めない。FE から来る `model` フィールドは**キャラクター／セッションの選択子**
//! であって推論モデルの指定ではないため（ADR-0004）、それが上流へ素通りする経路を
//! 型として作らない。
//!
//! # 上流に何を要求するのか（ADR-0038 §決定1 / 作業単位 A-7a）
//!
//! **要求は「必須」と「任意（あれば使う）」の層に分かれていて、
//! その表は `docs/adr/0038-what-personacore-requires-of-an-upstream.md` §決定1 が持つ。**
//! **ここには写さない**（CLAUDE.md「同じ発見を2箇所に書かない。片方が黙って古びる」）。
//! ここに置くのは**どこを読むかの誘導**だけである。
//!
//! **置き場所がここである理由**: **継ぎ目そのものがこのモジュール**（[`LlmClient`]）だから。
//! 上流を差し替える人も、上流固有の何かを足そうとする人も、**必ずこのファイルを通る。**
//! docs/ に置くと「読まれたかどうか」に依存するが、ここは**触る前に必ず開く場所**である。
//!
//! **⚠ 「任意」の層は既に任意になっている。** 聞けなかったときの枝は
//! `server::context`（コンテキスト長）と `crate::measured`（載っているモデル）が既に持つ。
//! **3層は新しい制約ではなく、いまの実装が何を前提にしているかに名前を付けたもの。**
//!
//! # 上流固有の漏れを数える（ADR-0036 §決定4）
//!
//! あちらはこう書いている ——
//! **「継ぎ目がある」を主張し続けるには、漏れを数え続ける必要がある。増えたら決定の変更である。**
//!
//! **数えているのは、このファイルの `#[cfg(test)] mod upstream_leaks`。**
//! **数の権威は見張り番の `EXPECTED_LEAKS` そのもの**で、**この doc には数を写さない**
//! （写した時点で、片方が黙って古びる）。
//!
//! **⚠ この見張り番が言えるのは「この数え方で数えて、記録と一致している」までである。**
//! **「継ぎ目が守られている」とは言えない。** 何を覆っていないかは
//! `upstream_leaks` の doc に**測ったうえで**書いてある。

mod error;
mod lmstudio;
mod request;
mod response;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::stream::BoxStream;

pub use error::LlmError;
pub use lmstudio::{
    DEFAULT_ENDPOINT, DEFAULT_MAIN_MODEL, DEFAULT_SUB_MODEL, EndpointError, LmStudioClient,
    LmStudioConfig, ModelsReport,
};
pub use request::{
    API_KEY_MAX_BYTES, ApiKey, ApiKeyError, ChatRequest, HostLookup, HostResolver,
    KeyOverPlainHttp, ModelId, ModelIdError, REPEAT_PENALTY_MAX, REPEAT_PENALTY_MIN, RepeatPenalty,
    SUB_LLM_TEMPERATURE, Sampling, SentSamplers, endpoint_for_log, endpoint_has_userinfo,
};

// メッセージの値型は `domain` に置いてある（`prompt` が reqwest を引き込まないため。
// 理由は `crate::domain` の doc）。**呼び出し側の名前を変えないよう再公開する。**
// 綴り（`"system"` 等）の決定権はこの層に残してある（`request::role_to_wire`）。
pub use crate::domain::{ChatMessage, ChatRole};
pub use response::{AnsweredBy, ChatChunk, Completion, FinishReason, TokenUsage};

/// 逐次流れるチャンク列。
///
/// **溜め込まずに流すことが前提**（docs/02-architecture.md）。項目の `Err` は
/// 「そこで上流が壊れた／切れた」の意味で、その直後にストリームは必ず終端する。
/// 終端までに受け取ったチャンクは有効なので、呼び出し側は蓄積分を保存してよい。
pub type ChatStream = BoxStream<'static, Result<ChatChunk, LlmError>>;

/// トレイトのメソッドが返す future。
///
/// `async fn` をトレイトに直接書くと `dyn LlmClient` にできない（`server` は
/// `Arc<dyn LlmClient>` で持ち回りたい）ため、明示的に箱に入れる。
pub type LlmFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, LlmError>> + Send + 'a>>;

/// 推論サーバとの会話。
///
/// **`server` は本文生成についてはこのトレイトにのみ依存する。**
/// 実装の差し替え（LM Studio → vLLM）やテストのモック注入は、この境界だけで完結する。
///
/// # 例外 — コンテキスト長の照会（ADR-0013 第2層）
///
/// **`server::context` だけは `LmStudioClient` の固有メソッド
/// `loaded_context_length()` を直接呼ぶ。** `/api/v0/models` は LM Studio 独自で、
/// **トレイトに載せると他の実装が実装できない項目になる**ため。
///
/// 呼ぶのは起動時の1回（`App::bind`）と、**5分ごとの引き直し**（ADR-0020。
/// `serve` が持つ背景タスクが具象型を握り続ける）だけ。
/// **`AppState` が持ち回るのは今も `Arc<dyn LlmClient>` のみ**で、
/// リクエストの経路からは具象型が見えない。**「不整合だから」とトレイトへ移さないこと。**
pub trait LlmClient: Send + Sync {
    /// 応答を最後まで受け取る（裏方のメモリ抽出など、逐次表示の要らない用途）。
    fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion>;

    /// 応答を逐次受け取る（本文生成の中継）。
    ///
    /// 戻り値の future は**本文の到着を待たずに解決する**こと。HTTP のステータス異常は
    /// ストリームの項目ではなくこの future の失敗として返すこと。
    fn stream<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, ChatStream>;

    /// **本文ごとに数える**（封 / ADR-0052 §推奨3）。
    ///
    /// `request` と `streaming` は、**このあと [`Self::complete`]（`false`）か
    /// [`Self::stream`]（`true`）へ渡すものと同じ値を渡すこと。**
    /// 実装は**チャットへ送るのと同じ本文**を数える（本文を別に組まない ——
    /// `reasoning_effort` 等がテンプレートの出力を変えうる）。
    ///
    /// # 返り値の3通り
    ///
    /// | 返り値 | 意味 | 呼び出し側 |
    /// |---|---|---|
    /// | `Ok(Counted(n))` | 上流が数えた | 封に使ってよい |
    /// | `Ok(Uncountable)` | **上流は答えたが、数えられない**（口が無い＝4xx・形が読めない・既定の実装） | その回はバイト数 |
    /// | `Err(_)` | **往復そのものが失敗した**（接続できない・時間切れ・通信の失敗・**5xx**） | その回はバイト数 |
    ///
    /// **5xx は `Err` の側**（作業単位 B-60b のレビュー）。読み込み中の 503 は上流の性質ではなく
    /// そのときの状態なので、「数えられない上流だと分かった」に入れない。
    ///
    /// **`Uncountable` と `Err` を分けてあるのは、「数えられない上流だと分かった」と
    /// 「今回は聞けなかった」が別の事実だから**（ADR-0052 §推奨4 の「覚える」を
    /// どちらで行うかは呼び出し側が決める。この層は決めない）。
    ///
    /// # 既定の実装は「数えられない」
    ///
    /// **数えたふりをしないこと**（0 や何かの数を返すと、呼び出し側は封をしたと信じる）。
    /// 既存のモックや数える口を持たない上流は、何も足さずに今のバイト数の計画のまま動く。
    fn count_request<'a>(
        &'a self,
        request: &'a ChatRequest,
        streaming: bool,
    ) -> LlmFuture<'a, TokenCount> {
        let _ = (request, streaming);
        Box::pin(async { Ok(TokenCount::Uncountable) })
    }

    /// **文字列ごとに数える**（計画の部品 / ADR-0052 §推奨2）。
    ///
    /// **部品の数は計画にだけ使い、安全は担わない**（部品の和は全体の上界にならない
    /// —— ADR-0052 §背景）。返り値の読み方は [`Self::count_request`] と同じ。
    ///
    /// 既定の実装は「数えられない」（[`Self::count_request`] と同じ理由）。
    fn count_text<'a>(&'a self, text: &'a str) -> LlmFuture<'a, TokenCount> {
        let _ = text;
        Box::pin(async { Ok(TokenCount::Uncountable) })
    }

    /// **この往復のあいだ使い続ける写し**（ADR-0052 §推奨3 / 作業単位 B-60b）。
    ///
    /// 封（[`Self::count_request`]）と送信（[`Self::complete`] / [`Self::stream`]）を
    /// **同じ上流の写しで行う**ための口。呼び出し側は**1ターンに1回だけ**呼び、
    /// 以後はその写しだけを使う。
    ///
    /// | 返り値 | 意味 | 呼び出し側 |
    /// |---|---|---|
    /// | `Some(写し)` | **中で上流が差し替わりうる実装**（走ったままの差し替え口） | **以後その写しだけを使う** |
    /// | `None` | 差し替わらない実装（具象の上流・テストのモック） | 自分自身をそのまま使ってよい |
    ///
    /// # なぜ要るのか
    ///
    /// 差し替え口は**呼ばれるたびに**いまの上流を読む。封と送信を別々に呼ぶと、
    /// **その間の差し替えで、前のプロファイルのトークナイザで数えた封のまま
    /// 新しいモデルへ送る**ことになる（B-60d-1 から封の数で送るかを決めているので、
    /// 安全の根拠が崩れる —— 別のトークナイザで「収まる」と示した本文を別のモデルへ送る）。
    ///
    /// # 既定の実装を置かない（作業単位 B-60b のレビュー）
    ///
    /// **自分自身を `Arc` で返す既定の実装は書けない**（`&self` から `Arc<dyn LlmClient>` は作れない）ので、
    /// 置けるとしたら `None` だけになる。**それを既定にすると、差し替え口を包むラッパが
    /// この口の転送を忘れたとき、黙って `None`（＝写しが固定されない）に落ちる**
    /// ——どの往復も成功するので、どこも赤くならない。
    /// **だから必須にして、実装ごとに `None` か `Some` かを書かせる**（忘れると型が止める）。
    ///
    /// **⚠ 差し替え口を実装するなら、必ず `Some` を返すこと。** 見張り番は `server::context` の
    /// `tests::a_pinned_copy_keeps_talking_to_the_upstream_it_was_taken_from`。
    fn pinned(&self) -> Option<Arc<dyn LlmClient>>;
}

/// **上流が数えたトークン数か、数えられなかったか**（ADR-0052 §推奨4 / 作業単位 B-60a）。
///
/// **`prompt::estimate_tokens`（バイト数の上界）とは別の型・別の名前にしてある。**
/// 上流の数を上界の名前で返すと、上界でない値が上界として扱われる
/// （ADR-0052 §見張り番の扱い）。
///
/// **上流の名前を名乗らない**（`/tokenize` などの綴りは `llm` の中に閉じる /
/// ADR-0036 §決定4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenCount {
    /// 上流のトークナイザが数えた数。
    Counted(u32),
    /// **数えられない。** 口が無い・応答の形が読めない・既定の実装。
    Uncountable,
}

/// **上流固有の漏れを数える見張り番**（作業単位 A-7a / ADR-0036 §決定4）。
///
/// # 何を「漏れ」と呼ぶか
///
/// **`llm` の外に出ている、特定の上流に紐づいた綴り。**
/// 数え方は**3枚の網**で、**どれも綴りの一覧を持たない**
/// （持つのは「継ぎ目の側」の一覧で、**知らない綴りは自動的に漏れの側へ落ちる**）。
///
/// | 網 | 何を見るか | 一覧に頼らない理由 |
/// |---|---|---|
/// | 1. 定義側 | `src/llm/` の**継ぎ目ではないファイル**の `pub` 項目名（**入れ子の `mod` の中も含む。メソッドは除く**）を、`src/**` の外側で探す | 綴りは**実装ファイルから毎回引き直す。** `src/llm/ollama.rs` を足せば、その中の名前が自動で対象になる |
/// | 2. 経路側 | `llm::` の直後から取り出している名前のうち、**継ぎ目の面に無いもの** | 一覧は**継ぎ目の側**（`mod.rs` の再公開から導出）。**新しい綴りは一覧を触らずに漏れになる** |
/// | 3. 迂回路 | **上流と直に話す道具**（[`UPSTREAM_TRANSPORT`]）が `llm` の外に現れないこと | `llm` を経由せずに上流へ話す形は、網1・網2 の**両方の外側**にある |
///
/// **網1と網2は独立に同じ答えを出すこと**を要求する（片方を潰す変異が緑にならないよう、
/// **それぞれ単独で** [`EXPECTED_LEAKS`] と突き合わせる）。
///
/// # 守れないもの（**把握したうえで残している。測った**）
///
/// **「塞いだ」ではない。** 次は素通りする ——
///
/// * **再公開で綴りを隠す形。** `src/server/mod.rs` に
///   `pub(crate) use crate::llm::LmStudioClient as Upstream;` と書けば、
///   `src/server/**` の他のファイルは `Upstream` としか綴らない。
///   **`mod.rs` は既に記録済みなので、箇所の数は増えないまま新しい依存が生える。**
/// * **`llm` を経由しない上流固有の値。** `server` が自分で綴りを書いても、
///   網1・網2 は見ていない。**網3（`reqwest`）が引っかかるのは
///   「実際に喋る」ところまで来た場合だけ**で、**綴りを組み立てて `llm` に渡す形は素通りする。**
///
///   **⚠ 類型の話ではない。いま実在する**（レビューの指摘で数え直した / 2026-08-29）:
///
///   | 場所 | 綴り |
///   |---|---|
///   | `src/server/config.rs` | `"http://127.0.0.1:1234/v1/chat/completions"` |
///   | `src/server/context.rs` | `path("/api/v0/models")` |
///   | `src/server/context.rs` | `path("/v1/chat/completions")`（**数え落としていた** / B-60a のレビュー） |
///   | `src/server/context.rs` | `path("/apply-template")`・`path("/tokenize")`（llama.cpp 固有 / B-60a） |
///
///   **⚠ 後ろの2行は LM Studio 固有ではない**（OpenAI 互換の口と llama.cpp の口。
///   `FaceClient` のテストが偽の上流に載せる綴り）。**それでも上流の綴りであり、
///   `llm` の外にあり、どの網も見ていない。** **帳簿（[`EXPECTED_LEAKS`]）の数は変えていない**
///   ——網が見ない形だからで、それがこの節の中身である。
///
///   **どちらも LM Studio 固有で、`llm` の外にあり、どの網も見ていない。**
///   （どちらも `#[cfg(test)]` の中だが、**網は本番とテストを区別しない**ので、
///   それは見逃している理由ではない。）
/// * **既存の継ぎ目ファイルへ定義を移す形**（作業単位 G-3b のレビュー指摘 / 2026-09-08）。
///   [`SEAM_MODULES`] は**ファイルの一覧**なので、上流固有の綴りを
///   `src/llm/lmstudio.rs` から `src/llm/response.rs` へ移すだけで、
///   **一覧を1文字も触らずに帳簿が減る。**
///
///   **⚠ 類型の話ではない。実測した** ——
///   [`AnsweredBy`] の定義を**振る舞いを1バイトも変えずに** `response.rs` から
///   `lmstudio.rs` へ移すと、**この台帳が 2 赤になる**
///   （`the_definition_net_sees_exactly_the_recorded_leaks` と
///   `the_import_net_sees_exactly_the_recorded_leaks`。**他は 1993 全部緑**）
///   ——**いまの置き場所は、その差ぶんだけ数えられていない。**
///   **G-3b がこの向きを実際に使った最初の例である。**
///
///   **⚠ その編集はレビューを通るが、赤くはならない。** 「減った」と気づく手掛かりが
///   スイートの側に無い、というのがこの項目の中身である。
///   **`SEAM_MODULES` の doc が言う「古びる向きが安全側」は、
///   新しいファイルを作る向きにしか成り立たない。**
/// * **`src/**` の中のテストモジュールと本番コードを区別しない**（レビュー指摘3）。
///   **`tests/**` は除外するのに、`src/**` の `#[cfg(test)] mod tests` は数える**、
///   という非対称がある。実際、記録の `src/server/context.rs` の3語のうち
///   **`DEFAULT_MAIN_MODEL` と `LmStudioConfig` はテストモジュールの中にしか無い**
///   （本番側は `LmStudioClient` だけ）。
///
///   **⚠ 単独では「うるさい側」だが、向きが変わる組み合わせが1つある** ——
///   **テスト由来で既に記録済みの (ファイル, 名前) に、本番コードが後から依存しても
///   何も赤くならない。** `context.rs` の本番コードが `LmStudioConfig` を
///   使い始める形がまさにそれ。**区別するには字句解析で `#[cfg(test)]` の範囲を
///   切り出す必要があり、いまは持っていない。**
/// * **`reqwest` 以外の HTTP クライアント。** 網3 は綴りを1つしか知らない
///   （**これは綴りの一覧である**）。**古びていないことは
///   [`the_transport_spelling_is_real`] が `Cargo.toml` と実装ファイルの両方で確かめる**が、
///   **別のクレートを足す形は止まらない。**
/// * **行末コメント・ブロックコメント・文字列リテラルの中の綴り。**
///   落とすのは**行頭が `//` の行だけ**なので、これらは**漏れとして数えてしまう**
///   （＝うるさく落ちる方へ倒してある。`src/**` に実在するブロックコメントは
///   0 件であることを確認済み）。
/// * **`tests/**`。** 走査は `src/**` だけを見る。統合テストが具象型を触るのは
///   モック注入の設計上あたりまえなので、数えない。
///
/// # 3つ目が生えたら
///
/// [`EXPECTED_LEAKS`] を書き換えることになる。**それは A-7 への着手ではなく、
/// ADR-0036 §決定4 の変更である**（ADR-0038 §決定3）。
/// [`the_counts_are_pinned_separately_from_the_record`] が
/// **箇所・名前・(ファイル, 名前) の3つの数を別に押さえてある**ので、
/// **記録に足すだけでは緑にならない。**
///
/// **⚠ かつてここは「箇所の数」しか押さえていなかった**（レビュー指摘2 / 2026-08-29）。
/// **新しいファイルを足す形は赤くなったが、既に記録のあるファイルへ名前を1語足す形は全緑**だった。
/// **ロードマップ A-7 と ADR-0038 は「名前で数えろ」と書いている**ので、
/// **箇所だけの門は、その主張を守っていなかった。**
///
/// [`the_transport_spelling_is_real`]: the_transport_spelling_is_real
/// [`the_counts_are_pinned_separately_from_the_record`]:
///     the_counts_are_pinned_separately_from_the_record
#[cfg(test)]
mod upstream_leaks {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// **継ぎ目の側のファイル**（`src/llm/` のうち、特定の上流に紐づかないもの）。
    ///
    /// **⚠ 一覧なのは「継ぎ目の側」である。** ここに無いファイルは
    /// **上流固有の実装として扱う**ので、`src/llm/ollama.rs` を足しても
    /// **この一覧を触らずに上流固有の側へ入る**（＝3つ目が素通りしない）。
    ///
    /// **逆向きの代償**: 継ぎ目の部品を新しいファイルへ切り出すと、
    /// **ここへ足すまで漏れとして数えられる。** その編集は意図的な行為であり、
    /// レビューを通る。**この向きは安全側に古びる。**
    ///
    /// **⚠ 「古びる向きが安全側になっている」とは書けない。片側でしか成り立たない**
    /// （作業単位 G-3b のレビュー指摘 / 2026-09-08）。
    /// **既存の継ぎ目ファイルへ定義を移す向きは、この一覧を1文字も触らずに帳簿を減らす。**
    /// **実測は、このモジュールの doc の「守れないもの」の節。**
    const SEAM_MODULES: [&str; 4] = ["mod", "error", "request", "response"];

    /// **構築点**（ADR-0036 §決定4 の表が「当然であって漏れではない」と書いている場所）。
    ///
    /// **数えないのではなく、区別して数える。** ここに新しい上流固有の綴りが増えれば
    /// [`EXPECTED_LEAKS`] が赤くなる。
    const CONSTRUCTION_POINT: &str = "src/server/mod.rs";

    /// 上流と**直に話す**道具（網3）。
    const UPSTREAM_TRANSPORT: &str = "reqwest";

    /// **いま記録されている漏れ**（`llm` の外に出ている上流固有の綴り / パスは昇順）。
    ///
    /// **⚠ これが数の権威。** doc にもロードマップにも数を写さない。
    ///
    /// # ⚠ この帳簿が数えているのは何か（**次に数える人へ**）
    ///
    /// **数えているのは「LM Studio への結合」ではなく、
    /// 「`llm::lmstudio` の公開面への結合」である。**
    /// **ADR-0036 §決定4 の引き金は前者を意図している。**
    ///
    /// **ずれは実在する** —— 上の「守れないもの」の表の2件
    /// （`"http://127.0.0.1:1234/v1/chat/completions"` と `path("/api/v0/models")`）は
    /// **LM Studio への結合だが、この帳簿には載らない。**
    /// **「帳簿が 2 箇所だから継ぎ目は薄い」とは読めない。**
    ///
    /// # 数え方
    ///
    /// **名前で数える**（ロードマップ A-7 と ADR-0038 §決定3 がそう書いている）ので、
    /// **型だけでなく定数も載る**（既定のエンドポイントとモデル id）。
    ///
    /// **⚠ 型だけを数えても 3 つある**（`LmStudioClient` / `LmStudioConfig` /
    /// **`EndpointError`**）。ADR-0036 §決定4 と ADR-0038 が名指ししているのは前の2つで、
    /// **`EndpointError` は落ちていた**（司令塔が 2026-08-29 に数え直して確認）。
    ///
    /// **`DEFAULT_MAIN_MODEL` / `DEFAULT_SUB_MODEL` を落とさないこと。**
    /// モデル id は「上流固有」より狭い「モデル固有」だが、
    /// **うるさい側＝安全な側**として意図的に載せてある。
    const EXPECTED_LEAKS: [(&str, &[&str]); 3] = [
        (
            "src/server/config.rs",
            &[
                "DEFAULT_ENDPOINT",
                "DEFAULT_MAIN_MODEL",
                "DEFAULT_SUB_MODEL",
                "EndpointError",
                "LmStudioConfig",
            ],
        ),
        (
            "src/server/context.rs",
            &["DEFAULT_MAIN_MODEL", "LmStudioClient", "LmStudioConfig"],
        ),
        // **構築点**（[`CONSTRUCTION_POINT`]）。**数えないのではなく、区別して数える。**
        ("src/server/mod.rs", &["LmStudioClient"]),
    ];

    // ------------------------------------------------------------------
    // 走査の本体。**すべてソースを引数で受け取る**（実ファイルと植えたソースを
    // 同じ判定に通すため / CLAUDE.md「走査の対照は自己成就しうる」）。
    // ------------------------------------------------------------------

    /// 語を構成しうる文字か。**綴りの端に接していたら「別の綴りの一部」と見なす。**
    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || ch == '_'
    }

    /// `name` を**トークンとして丸ごと**含むか（`WidgetForgeExtra` は `WidgetForge` ではない）。
    fn mentions(source: &str, name: &str) -> bool {
        source.match_indices(name).any(|(at, _)| {
            let before = source[..at].chars().next_back();
            let after = source[at + name.len()..].chars().next();
            !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
        })
    }

    /// **行頭が `//` の行**（行コメント・doc コメント）を落とす。行数は変えない。
    ///
    /// **⚠ 行末コメントとブロックコメントは落とさない。**
    /// 落とすほうへ倒すと「コメントに書いたら数えない」抜け道になるので、
    /// **数えすぎる側**へ倒してある。
    fn strip_comment_lines(source: &str) -> String {
        source
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("//") {
                    ""
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 語（識別子になりうる文字の並び）を順に取り出す。**経路の予約語は落とす。**
    fn identifiers(text: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut current = String::new();
        for ch in text.chars() {
            if is_word_char(ch) {
                current.push(ch);
            } else if !current.is_empty() {
                names.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            names.push(current);
        }
        names.retain(|name| !matches!(name.as_str(), "as" | "self" | "crate" | "super"));
        names
    }

    /// `pub` 項目の名前（`pub use` は項目ではないので落とす）。
    ///
    /// # 字下げした `pub` も拾う（**作業単位 A-7a のレビュー指摘1**）
    ///
    /// **⚠ かつては列 0 だけを見ていた。その形では、この道具の主目的が空振りする** ——
    /// レビュアーが実測（2026-08-29）:
    ///
    /// ```text
    /// src/llm/lmstudio.rs: pub mod bundle { pub struct ReasoningEffort; }
    /// src/llm/mod.rs:      pub use lmstudio::bundle::ReasoningEffort;
    /// src/server/config.rs: use crate::llm::ReasoningEffort;
    /// ```
    ///
    /// → **記録を1文字も触らずに全緑。** 拾えていたのは外側の `bundle` だけで、
    /// **`config.rs` が綴るのは中の型のほう**だった。
    /// **[ADR-0038] §決定3 の「束」は `lmstudio.rs` に一段掘るのが自然な形**なので、
    /// **いちばんありそうな3つ目が、いちばん通り抜けやすかった。**
    ///
    /// # ただし字下げした**関数**は拾わない
    ///
    /// `impl` の中のメソッド（`pub fn new` / `pub const fn model` / `pub async fn stream`）まで
    /// 拾うと、探し語が `new` / `model` / `stream` になって
    /// **`src/**` のほぼ全ファイルが「漏れ」になる。**
    /// **拾うのは型の側の項目だけ**（`struct` / `enum` / `trait` / `type` / `union` /
    /// `mod` / `const` / `static`）。
    ///
    /// **⚠ 覆えないもの**: `impl` の中の**関連定数**（字下げした `pub const`）は拾ってしまう。
    /// いま `src/llm/**` に1つも無いので実害は無いが、足すと綴り次第で誤爆しうる
    /// （そのときは記録が赤くなるので、黙って緩むことはない）。
    fn top_level_pub_items(source: &str) -> Vec<String> {
        let mut items = Vec::new();
        for line in source.lines() {
            let indented = line.starts_with(char::is_whitespace);
            let trimmed = line.trim_start();
            let Some(rest) = trimmed
                .strip_prefix("pub ")
                .or_else(|| trimmed.strip_prefix("pub(crate) "))
                .or_else(|| trimmed.strip_prefix("pub(super) "))
            else {
                continue;
            };

            let mut words = rest.split_whitespace();
            let mut name = None;
            while let Some(word) = words.next() {
                match word {
                    "async" | "unsafe" | "extern" | "default" => continue,
                    "struct" | "enum" | "trait" | "type" | "union" | "mod" | "static" => {
                        name = words.next();
                    }
                    // 字下げした `fn` は `impl` の中のメソッド。拾わない（上の doc）。
                    "fn" => {
                        if !indented {
                            name = words.next();
                        }
                    }
                    // `const fn` は関数、それ以外は定数。
                    "const" => {
                        name = match words.next() {
                            Some("fn") => {
                                if indented {
                                    None
                                } else {
                                    words.next()
                                }
                            }
                            other => other,
                        };
                    }
                    _ => {}
                }
                break;
            }

            let Some(name) = name else { continue };
            let name: String = name.chars().take_while(|ch| is_word_char(*ch)).collect();
            if !name.is_empty() {
                items.push(name);
            }
        }
        items
    }

    /// `pub use …;` を**行をまたいで**1文にして返す。
    ///
    /// **⚠ 文の頭は字下げ無し（列 0）に限る。** 字下げも許すと、
    /// **このファイル自身の中に植えたフィクスチャの文字列が拾われる**
    /// （着手時に実際に踏んだ ——`ForgeError` などが実ツリーの継ぎ目の面に混ざった。
    /// 見張り番は [`the_planted_fixtures_do_not_reach_the_real_seam_surface`]）。
    ///
    /// [`the_planted_fixtures_do_not_reach_the_real_seam_surface`]:
    ///     the_planted_fixtures_do_not_reach_the_real_seam_surface
    fn pub_use_statements(source: &str) -> Vec<String> {
        let mut statements = Vec::new();
        let mut current: Option<String> = None;
        for line in source.lines() {
            let trimmed = line.trim();
            if current.is_none() && !line.starts_with("pub use ") {
                continue;
            }
            let accumulated = current.get_or_insert_with(String::new);
            if !accumulated.is_empty() {
                accumulated.push(' ');
            }
            accumulated.push_str(trimmed);
            if trimmed.contains(';')
                && let Some(statement) = current.take()
            {
                statements.push(statement);
            }
        }
        statements
    }

    /// `pub use 経路::名前;` を「**経路の区間すべて**, 公開される名前」に割る。
    ///
    /// **⚠ かつては末尾の区間だけを返していた**（レビュー指摘1 の後半 / 2026-08-29）。
    /// それだと `pub use lmstudio::bundle::ReasoningEffort;` の出どころが `bundle` になり、
    /// **`bundle` は `src/llm/` のファイル名ではない**ので「上流固有ではない」と判定され、
    /// **中の型が継ぎ目の面に載っていた**（＝網2 が漏れと呼ばなくなる）。
    /// **経路のどこかに上流固有のファイル名があれば上流固有**として扱う。
    fn split_pub_use(statement: &str) -> Option<(Vec<String>, Vec<String>)> {
        let rest = statement
            .strip_prefix("pub use ")?
            .trim()
            .trim_end_matches(';');
        let (prefix, names) = if let Some(at) = rest.find("::{") {
            (&rest[..at], identifiers(&rest[at + 3..]))
        } else {
            let at = rest.rfind("::")?;
            (&rest[..at], identifiers(&rest[at + 2..]))
        };
        let segments: Vec<String> = prefix
            .split("::")
            .map(|segment| segment.trim().to_owned())
            .filter(|segment| !segment.is_empty())
            .collect();
        Some((segments, names))
    }

    /// 網1の対象 —— **継ぎ目ではないファイル**で定義されている `pub` 項目名（昇順）。
    fn upstream_item_names(modules: &[(String, String)]) -> Vec<String> {
        modules
            .iter()
            .filter(|(stem, _)| !SEAM_MODULES.contains(&stem.as_str()))
            .flat_map(|(_, source)| top_level_pub_items(&strip_comment_lines(source)))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// 網2の対象 —— **継ぎ目の面**（`mod.rs` 自身の項目 ＋ 継ぎ目ファイルからの再公開 / 昇順）。
    fn seam_surface(modules: &[(String, String)]) -> Vec<String> {
        let stems: BTreeSet<&str> = modules.iter().map(|(stem, _)| stem.as_str()).collect();
        let Some((_, root)) = modules.iter().find(|(stem, _)| stem == "mod") else {
            return Vec::new();
        };
        let root = strip_comment_lines(root);

        let mut names: BTreeSet<String> = top_level_pub_items(&root).into_iter().collect();
        for statement in pub_use_statements(&root) {
            let Some((path, exported)) = split_pub_use(&statement) else {
                continue;
            };
            // **経路のどこか**に「`llm` の中のファイルで、かつ継ぎ目ではない」区間があれば
            // 上流固有。継ぎ目の面には載せない（`lmstudio::bundle::X` もここで落ちる）。
            let upstream = path.iter().any(|segment| {
                stems.contains(segment.as_str()) && !SEAM_MODULES.contains(&segment.as_str())
            });
            if !upstream {
                names.extend(exported);
            }
        }
        names.into_iter().collect()
    }

    /// `llm::` の直後から取り出している名前（昇順・重複なし）。
    ///
    /// `use crate::llm::{A, B}` / `crate::llm::A::b()` / `crate::llm::mod_name::c()` を拾う。
    /// **直前が語の文字なら別の綴り**（`xllm::` は `llm::` ではない）。
    fn names_taken_from_llm(source: &str) -> Vec<String> {
        let mut names = BTreeSet::new();
        for (at, _) in source.match_indices("llm::") {
            if source[..at].chars().next_back().is_some_and(is_word_char) {
                continue;
            }
            let rest = source[at + "llm::".len()..].trim_start();
            if let Some(group) = rest.strip_prefix('{') {
                let mut depth = 1_usize;
                let mut end = None;
                for (index, ch) in group.char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = Some(index);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                // 閉じていないなら読めない。**読めなかったことを黙って緑にしない**ため、
                // 読めた範囲だけを使う（この形は正しい Rust では起きない）。
                let inner = end.map_or(group, |end| &group[..end]);
                names.extend(identifiers(inner));
            } else {
                names.extend(identifiers(rest).into_iter().take(1));
            }
        }
        names.into_iter().collect()
    }

    /// 網1 —— 上流固有の綴りを**トークンとして**持つファイル（パス昇順）。
    fn leaks_by_definition(
        outside: &[(String, String)],
        items: &[String],
    ) -> Vec<(String, Vec<String>)> {
        let mut found: Vec<(String, Vec<String>)> = outside
            .iter()
            .filter_map(|(path, source)| {
                let stripped = strip_comment_lines(source);
                let names: Vec<String> = items
                    .iter()
                    .filter(|item| mentions(&stripped, item))
                    .cloned()
                    .collect();
                (!names.is_empty()).then(|| (path.clone(), names))
            })
            .collect();
        found.sort();
        found
    }

    /// 網2 —— `llm::` から取り出している名前のうち、**継ぎ目の面に無いもの**（パス昇順）。
    fn leaks_by_import(
        outside: &[(String, String)],
        seam: &[String],
    ) -> Vec<(String, Vec<String>)> {
        let mut found: Vec<(String, Vec<String>)> = outside
            .iter()
            .filter_map(|(path, source)| {
                let stripped = strip_comment_lines(source);
                let names: Vec<String> = names_taken_from_llm(&stripped)
                    .into_iter()
                    .filter(|name| !seam.iter().any(|allowed| allowed == name))
                    .collect();
                (!names.is_empty()).then(|| (path.clone(), names))
            })
            .collect();
        found.sort();
        found
    }

    /// 網3 —— 上流と直に話す道具を名指ししているファイル（パス昇順）。
    fn direct_transport_users(outside: &[(String, String)]) -> Vec<String> {
        let mut found: Vec<String> = outside
            .iter()
            .filter(|(_, source)| mentions(&strip_comment_lines(source), UPSTREAM_TRANSPORT))
            .map(|(path, _)| path.clone())
            .collect();
        found.sort();
        found
    }

    // ------------------------------------------------------------------
    // 実ファイルの読み出し。
    // ------------------------------------------------------------------

    /// リポジトリ根。**絶対パスで組む**（相対パスはプロセスの作業ディレクトリで
    /// 解決され、別のツリーを読みうる / CLAUDE.md の `A-1a`）。
    fn repository_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// リポジトリ根からの相対パス（`/` 区切り）に直す。
    fn relative(root: &Path, path: &Path) -> String {
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// `src/**` の Rust ソースを「相対パス, 中身」で返す。
    fn source_files() -> Vec<(String, String)> {
        let root = repository_root();
        let mut files = Vec::new();
        let mut stack = vec![root.join("src")];
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
                files.push((relative(&root, &path), source));
            }
        }
        files.sort();
        files
    }

    /// `src/llm/**` を「モジュール名（拡張子なし）, 中身」で返す。
    fn llm_modules() -> Vec<(String, String)> {
        let mut modules: Vec<(String, String)> = source_files()
            .into_iter()
            .filter(|(path, _)| path.starts_with("src/llm/"))
            .map(|(path, source)| {
                let stem = path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&path)
                    .trim_end_matches(".rs")
                    .to_owned();
                (stem, source)
            })
            .collect();
        modules.sort();
        modules
    }

    /// `src/**` のうち `src/llm/**` 以外。
    fn sources_outside_llm() -> Vec<(String, String)> {
        source_files()
            .into_iter()
            .filter(|(path, _)| !path.starts_with("src/llm/"))
            .collect()
    }

    /// [`EXPECTED_LEAKS`] を比較できる形に直す。
    fn recorded() -> Vec<(String, Vec<String>)> {
        EXPECTED_LEAKS
            .iter()
            .map(|(path, names)| {
                (
                    (*path).to_owned(),
                    names.iter().map(|name| (*name).to_owned()).collect(),
                )
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // 走査対象と対象語の**下限**。
    //
    // **⚠ これは「保険」ではない。ある一群の変異については、ここだけが権威である**
    // （レビュー指摘4 / 2026-08-29 の実測）——
    // **走査を `src/server/**` に狭める変異**は、**記録の3ファイルが全部その下にある**ので
    // `found == recorded` が保たれ、**網1も網2も緑のまま通る。**
    // **床を外すと lib 全緑になった。** CLAUDE.md の第5分類（条件つき等価）そのもの。
    // **床を消さないこと。**
    // ------------------------------------------------------------------

    /// **走査対象が空でない**（[`sources_outside_llm`] が `src/**` の大半を占めること）。
    ///
    /// **⚠ 絶対数の床にしないこと**（レビュー指摘の提案 / 2026-08-29）。
    /// `>= 50` は実数 52 に対して余裕が 2 しか無く、
    /// **`src` が増えるほど黙って緩む**（80 ファイルになれば「30 落とす狭め方」が通る）。
    /// **総数との比で持つ。**
    ///
    /// **狭め方そのもの**（`source_files` の根を掘り下げる形）は
    /// [`the_scan_reaches_every_part_of_src`] が見る。
    ///
    /// [`sources_outside_llm`]: sources_outside_llm
    /// [`the_scan_reaches_every_part_of_src`]: the_scan_reaches_every_part_of_src
    #[test]
    fn the_scan_has_something_to_look_at() {
        let all = source_files();
        let outside = sources_outside_llm();
        assert!(
            outside.len() * 4 >= all.len() * 3,
            "`src/**` の {} 件のうち {} 件しか見ていないなら、この走査は何も守っていない",
            all.len(),
            outside.len()
        );

        let modules = llm_modules();
        assert!(
            modules.len() >= 5,
            "`src/llm/` を読めていない: {} 件",
            modules.len()
        );

        // 継ぎ目の一覧が古びていない（綴りを変えたら、そのファイルは上流固有側へ落ちる）。
        for seam in SEAM_MODULES {
            assert!(
                modules.iter().any(|(stem, _)| stem == seam),
                "継ぎ目として数えているファイルが実在しない: src/llm/{seam}.rs"
            );
        }

        // 上流固有側のファイルが1つも無ければ、網1 は何も探していない。
        assert!(
            modules
                .iter()
                .any(|(stem, _)| !SEAM_MODULES.contains(&stem.as_str())),
            "上流固有の実装ファイルが1つも無い: {:?}",
            modules.iter().map(|(stem, _)| stem).collect::<Vec<_>>()
        );
    }

    /// **走査が `src` の全部位に届いている。**
    ///
    /// **⚠ これが要る理由は実測で出た**（レビュー指摘4 / 2026-08-29）——
    /// **走査を `src/server/**` に狭めると、記録の3ファイルは全部その下にある**ので
    /// **網1も網2も緑のまま通る。** 件数の床だけが赤くなった。
    /// **「床が唯一の権威」という状態を解くために、ここを2本目の権威として置いてある**
    /// （だから見るのは `source_files` ではなく **網が実際に読む集合**）。
    ///
    /// **期待は一覧で持たない。`src` を読んで、その場で作る** ——
    /// **直下の項目（ディレクトリと `.rs`）が1つでも走査に現れなければ落ちる。**
    /// **`src` が増えるほど厳しくなる**（緩む向きが無い）。
    #[test]
    fn the_scan_reaches_every_part_of_src() {
        // **網が実際に読む集合**を見る（`source_files` ではない）。
        // 狭め方は2通りあり、**両方ともここで見えること**が要る ——
        // 「読み出しの根を掘り下げる」形と「外側の絞りを変える」形。
        let scanned = sources_outside_llm();
        let root = repository_root().join("src");

        let mut parts = 0_usize;
        for entry in std::fs::read_dir(&root).expect("`src` を読めること") {
            let path = entry.expect("ディレクトリ項目を読めること").path();
            let Some(name) = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            else {
                continue;
            };

            // `src/llm/**` は網の対象外（自分自身）。存在は
            // [`the_scan_has_something_to_look_at`] が別に見る。
            if name == "llm" {
                continue;
            }

            let expected = if path.is_dir() {
                format!("src/{name}/")
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                format!("src/{name}")
            } else {
                continue;
            };

            parts += 1;
            assert!(
                scanned.iter().any(|(path, _)| path.starts_with(&expected)),
                "走査が `src` の一部に届いていません: {expected}"
            );
        }

        assert!(parts >= 7, "`src` の直下を読めていない: {parts} 件");
    }

    /// **網1が探す綴りが空でない。** いま 7 件（`src/llm/lmstudio.rs` の実数）。
    #[test]
    fn the_definition_net_has_names_to_look_for() {
        let items = upstream_item_names(&llm_modules());
        assert!(
            items.len() >= 7,
            "上流固有の項目を読み出せていないなら、網1 は何も探していない: {items:?}"
        );
    }

    /// **網2が許す面が空でない。** いま 13 件（`mod.rs` の再公開と自身の項目の実数）。
    #[test]
    fn the_seam_surface_is_not_empty() {
        let seam = seam_surface(&llm_modules());
        assert!(
            seam.len() >= 13,
            "継ぎ目の面が読めていないなら、網2 は全部を漏れと呼ぶ: {seam:?}"
        );
    }

    // ------------------------------------------------------------------
    // 実ツリーに対する主張（**網ごとに単独で**記録と突き合わせる）。
    // ------------------------------------------------------------------

    /// **網1 が見る漏れは、記録と完全に一致する。**
    #[test]
    fn the_definition_net_sees_exactly_the_recorded_leaks() {
        let items = upstream_item_names(&llm_modules());
        let found = leaks_by_definition(&sources_outside_llm(), &items);

        assert_eq!(
            found,
            recorded(),
            "`llm` の外に出ている上流固有の綴りが記録と違います。\
             増えているなら、それは ADR-0036 決定4 の変更です（ADR-0038 決定3）。\
             探した綴り: {items:?}"
        );
    }

    /// **網2 が見る漏れは、記録と完全に一致する。**
    ///
    /// **網1 と同じ答えを、別の見方で出すこと**を要求している
    /// （片方を潰す変異が、もう片方に隠れて緑にならないように）。
    #[test]
    fn the_import_net_sees_exactly_the_recorded_leaks() {
        let seam = seam_surface(&llm_modules());
        let found = leaks_by_import(&sources_outside_llm(), &seam);

        assert_eq!(
            found,
            recorded(),
            "`llm` から取り出している名前が記録と違います。継ぎ目の面: {seam:?}"
        );
    }

    /// **`llm` の外に、上流と直に話すファイルは無い。**
    #[test]
    fn no_file_outside_the_llm_module_talks_to_the_upstream_directly() {
        let found = direct_transport_users(&sources_outside_llm());
        assert!(
            found.is_empty(),
            "`{UPSTREAM_TRANSPORT}` が `llm` の外に出ています（継ぎ目を迂回する形）: {found:?}"
        );
    }

    /// **網3 の綴りが実在する。** `Cargo.toml` と上流固有の実装ファイルの両方で確かめる。
    ///
    /// 綴りが古びると、網3 は**何も見ずに緑**になる（15b の自己成就と同じ形）。
    #[test]
    fn the_transport_spelling_is_real() {
        let manifest = std::fs::read_to_string(repository_root().join("Cargo.toml"))
            .expect("Cargo.toml を読めること");
        assert!(
            mentions(&manifest, UPSTREAM_TRANSPORT),
            "依存に無い綴りで探している: {UPSTREAM_TRANSPORT}"
        );

        let modules = llm_modules();
        assert!(
            modules
                .iter()
                .filter(|(stem, _)| !SEAM_MODULES.contains(&stem.as_str()))
                .any(|(_, source)| mentions(&strip_comment_lines(source), UPSTREAM_TRANSPORT)),
            "上流固有の実装が {UPSTREAM_TRANSPORT} を使っていない（綴りが古びた）"
        );
    }

    /// **数は、記録とは別に押さえる**（2段目の門）。
    ///
    /// 記録（[`EXPECTED_LEAKS`]）を書き換えて走査を緑に戻しても、ここが赤くなる。
    /// **押さえるのは3つ:**
    ///
    /// | 数え方 | いま | どの形を止めるか |
    /// |---|---|---|
    /// | **箇所**（構築点を除く） | 2 | **新しいファイル**が漏れ始める |
    /// | **名前**（重複を畳んだもの） | 6 | **新しい上流固有の綴り**が外へ出る |
    /// | **(ファイル, 名前) の組** | 9 | **既に記録のあるファイル**へ 1語 増える |
    ///
    /// **⚠ 3つ目が要る理由は実測で出た**（レビュー指摘2 / 2026-08-29）——
    /// **箇所だけを押さえていたとき、既記録のファイルへ名前を1語足す形は全緑だった。**
    /// **`reasoning_effort` が `LmStudioConfig` の欄として `server/config.rs` に生えるのは、
    /// いちばんありそうな形の1つ**なので、そこが素通りするのは致命的だった。
    ///
    /// **⚠ 名前の数（6）は ADR-0036 §決定4 と ADR-0038 の「2」とは別の数え方である。**
    /// あちらは**ファイル**を数えている（この表の1行目）。
    #[test]
    fn the_counts_are_pinned_separately_from_the_record() {
        assert!(
            EXPECTED_LEAKS
                .iter()
                .any(|(path, _)| *path == CONSTRUCTION_POINT),
            "構築点が記録から消えています: {CONSTRUCTION_POINT}"
        );

        let places: Vec<&str> = EXPECTED_LEAKS
            .iter()
            .map(|(path, _)| *path)
            .filter(|path| *path != CONSTRUCTION_POINT)
            .collect();

        assert_eq!(
            places.len(),
            2,
            "上流固有の漏れの箇所が増減しました（構築点を除く）: {places:?}。\
             ADR-0036 決定4 は「増えたら決定の変更」と書いています"
        );

        let names: BTreeSet<&str> = EXPECTED_LEAKS
            .iter()
            .flat_map(|(_, names)| names.iter().copied())
            .collect();

        assert_eq!(
            names.len(),
            6,
            "`llm` の外に出ている上流固有の名前が増減しました: {names:?}。\
             ロードマップ A-7 と ADR-0038 は「名前で数えろ」と書いています"
        );

        let pairs: usize = EXPECTED_LEAKS.iter().map(|(_, names)| names.len()).sum();

        assert_eq!(
            pairs, 9,
            "(ファイル, 名前) の組が増減しました。\
             既に記録のあるファイルへ1語足す形も、決定の変更です"
        );
    }

    // ------------------------------------------------------------------
    // 対照 —— **植えたソース**を、見張り番と同じ本体に通す
    // （実ファイルだけを見る対照は自己成就する / 15b の実測）。
    // ------------------------------------------------------------------

    /// 植えた「上流固有の実装ファイル」と「継ぎ目のファイル」。
    ///
    /// **綴りはいまの2つ（`LmStudio…`）と似ていない**——
    /// **一覧に頼っていないことは、似ていない綴りで初めて言える。**
    ///
    /// # ⚠ フィクスチャの綴りは、実物と**似せない**こと（規約）
    ///
    /// **走査はこのファイル自身も読む**（実ツリー側の導出が `src/llm/mod.rs` を通る）。
    /// **実物と同じ綴りを植えると、それが実ツリーの面や対象語に混ざっても
    /// [`the_planted_fixtures_do_not_reach_the_real_seam_surface`] が気づけない**
    /// （交差が「もともと同じ綴り」で埋まってしまう）。
    ///
    /// **いまの実害は 0**（植えた 7 語は実物の 6 語と1つも衝突しない）**が、
    /// それは規約が守られているからであって、仕掛けが守っているからではない。**
    /// **`Forge` / `Planted` / `Widget` のような、実物に無い語幹を使うこと。**
    ///
    /// [`the_planted_fixtures_do_not_reach_the_real_seam_surface`]:
    ///     the_planted_fixtures_do_not_reach_the_real_seam_surface
    fn planted_modules() -> Vec<(String, String)> {
        vec![
            (
                "ollama".to_owned(),
                planted_lines(&[
                    "pub struct WidgetForge {}",
                    "pub const FORGE_HOME: &str = \"/tmp\";",
                    // **束と同じ形**（一段掘った中の型）。レビュー指摘1 の実測がこの形だった。
                    "pub mod harness {",
                    "    pub struct ForgeDial;",
                    "}",
                    // `impl` の中のメソッドは**拾わない**（`new` を探し語にしない）。
                    "impl WidgetForge {",
                    "    pub fn new() -> Self { Self {} }",
                    "}",
                ]),
            ),
            (
                "request".to_owned(),
                planted_lines(&["pub struct PlantedRequest {}"]),
            ),
            ("mod".to_owned(), planted_root()),
        ]
    }

    /// 植えた `src/llm/mod.rs`（継ぎ目の面の導出を通すため）。
    fn planted_root() -> String {
        planted_lines(&[
            "pub trait ForgeClient {}",
            "pub type ForgeStream = ();",
            "pub use error::ForgeError;",
            "pub use ollama::{",
            "    WidgetForge, FORGE_HOME,",
            "};",
            // **一段掘った経路からの再公開。** これが継ぎ目の面に載ってはいけない。
            "pub use ollama::harness::ForgeDial;",
            "pub use crate::domain::PlantedMessage;",
            "pub use request::PlantedRequest;",
        ])
    }

    /// 植えた行をソースに組み立てる。
    ///
    /// **⚠ 1行ずつのリテラルで書くこと。** 続き行に生の `pub …` を書くと、
    /// **このファイル自身の走査（字下げした `pub` も拾う）に引っかかり、
    /// 実ツリーの継ぎ目の面へ混ざる**（[`the_planted_fixtures_do_not_reach_the_real_seam_surface`]）。
    /// この形なら、ソース上のどの行も引用符で始まるので拾われない。
    ///
    /// [`the_planted_fixtures_do_not_reach_the_real_seam_surface`]:
    ///     the_planted_fixtures_do_not_reach_the_real_seam_surface
    fn planted_lines(lines: &[&str]) -> String {
        let mut source = lines.join("\n");
        source.push('\n');
        source
    }

    /// **上流固有の綴りは、実装ファイルから引き直している**（一覧ではない）。
    ///
    /// 1本で4つを見ている ——
    /// **継ぎ目のファイル（`request`）の項目は混ざらない** /
    /// **一段掘った `pub mod` の中の型（`ForgeDial`）も拾う**（レビュー指摘1）/
    /// **入れ物（`harness`）も拾う** / **`impl` の中のメソッド（`new`）は拾わない。**
    #[test]
    fn the_item_names_come_from_the_upstream_implementation_file() {
        let items = upstream_item_names(&planted_modules());
        assert_eq!(
            items,
            vec![
                "FORGE_HOME".to_owned(),
                "ForgeDial".to_owned(),
                "WidgetForge".to_owned(),
                "harness".to_owned(),
            ],
            "継ぎ目以外のファイルの `pub` 項目を、入れ子も含めて拾うこと（メソッドは除く）"
        );
    }

    /// **継ぎ目の面は、`mod.rs` の再公開から導いている**（一覧ではない）。
    ///
    /// 上流固有のファイルからの再公開（`ollama::…`）は**面に載らない。**
    /// **一段掘った経路（`ollama::harness::ForgeDial`）も同じ**（レビュー指摘1 の後半）。
    #[test]
    fn the_seam_surface_comes_from_the_reexports() {
        let seam = seam_surface(&planted_modules());
        assert_eq!(
            seam,
            vec![
                "ForgeClient".to_owned(),
                "ForgeError".to_owned(),
                "ForgeStream".to_owned(),
                "PlantedMessage".to_owned(),
                "PlantedRequest".to_owned(),
            ],
            "上流固有のファイルからの再公開は、経路が何段でも継ぎ目と数えないこと"
        );
    }

    /// **植えた綴りが、実ツリーの継ぎ目の面にも対象語にも混ざっていない。**
    ///
    /// **フィクスチャはこのファイル（`src/llm/mod.rs`）の中にある**ので、
    /// **走査が自分自身を読む**という形になっている
    /// （CLAUDE.md「走査系のテストが増えると、互いのフィクスチャで誤爆する」）。
    ///
    /// **⚠ 机上の話ではない。着手時に実際に起きた** ——
    /// `pub use` の行頭を `trim()` してから見ていたので、
    /// **植えた文字列の中の `pub use error::ForgeError;` が実ツリーの面に入った。**
    /// 混ざると**継ぎ目の面が広がる＝漏れを漏れと呼ばなくなる**方向に効くので、
    /// **緑のまま守備範囲だけが縮む。**
    #[test]
    fn the_planted_fixtures_do_not_reach_the_real_seam_surface() {
        let planted: BTreeSet<String> = seam_surface(&planted_modules())
            .into_iter()
            .chain(upstream_item_names(&planted_modules()))
            .collect();
        assert!(!planted.is_empty(), "対照が空では何も確かめていない");

        let real: BTreeSet<String> = seam_surface(&llm_modules())
            .into_iter()
            .chain(upstream_item_names(&llm_modules()))
            .collect();

        let mixed: Vec<&String> = real.intersection(&planted).collect();
        assert!(
            mixed.is_empty(),
            "植えたフィクスチャの綴りが実ツリーの走査に混ざっています: {mixed:?}"
        );
    }

    /// **網1 は、いまの2つと似ていない綴りの3つ目を見つける。**
    ///
    /// 1行で4つを見ている ——
    /// **見つける** / **行頭コメントは数えない** / **綴りが伸びた別物は数えない** /
    /// **無関係なファイルを見つけない。**
    #[test]
    fn the_definition_net_would_notice_a_third_leak_under_an_unrelated_name() {
        let items = upstream_item_names(&planted_modules());
        let planted = vec![
            (
                "src/server/turn.rs".to_owned(),
                "let forge = WidgetForge::new();\n".to_owned(),
            ),
            (
                "src/server/body.rs".to_owned(),
                "    /// WidgetForge のことは書くが、使ってはいない\n    let value = 1;\n"
                    .to_owned(),
            ),
            (
                "src/server/usage.rs".to_owned(),
                "let other = WidgetForgeExtra::new();\n".to_owned(),
            ),
            (
                "src/prompt/mod.rs".to_owned(),
                "pub fn plain() {}\n".to_owned(),
            ),
        ];

        let found = leaks_by_definition(&planted, &items);
        assert_eq!(
            found,
            vec![(
                "src/server/turn.rs".to_owned(),
                vec!["WidgetForge".to_owned()]
            )],
            "植えた3つ目をちょうど1件見つけること"
        );
    }

    /// **網2 は、いまの2つと似ていない綴りの3つ目を見つける。**
    ///
    /// **モジュール経路での漏れ**（`llm::ollama::…`）も見つけ、
    /// **接頭辞が伸びた別の経路**（`xllm::`）は見つけない。
    #[test]
    fn the_import_net_would_notice_a_third_leak_under_an_unrelated_name() {
        let seam = seam_surface(&planted_modules());
        let planted = vec![
            (
                "src/server/turn.rs".to_owned(),
                "use crate::llm::{PlantedRequest, WidgetForge};\n".to_owned(),
            ),
            (
                "src/server/body.rs".to_owned(),
                "use crate::llm::PlantedRequest;\nlet client: crate::llm::ForgeClient;\n"
                    .to_owned(),
            ),
            (
                "src/server/usage.rs".to_owned(),
                "/// use crate::llm::WidgetForge;\nlet value = 1;\n".to_owned(),
            ),
            (
                "src/server/web.rs".to_owned(),
                "let home = crate::llm::ollama::FORGE_HOME;\n".to_owned(),
            ),
            (
                "src/prompt/mod.rs".to_owned(),
                "let value = xllm::WidgetForge;\n".to_owned(),
            ),
        ];

        let found = leaks_by_import(&planted, &seam);
        assert_eq!(
            found,
            vec![
                (
                    "src/server/turn.rs".to_owned(),
                    vec!["WidgetForge".to_owned()]
                ),
                ("src/server/web.rs".to_owned(), vec!["ollama".to_owned()]),
            ],
            "継ぎ目の面に無い名前だけを、経路から拾うこと"
        );
    }

    /// **網3 は、継ぎ目を迂回して直に話す形を見つける。**
    #[test]
    fn the_transport_net_would_notice_a_direct_call() {
        let planted = vec![
            (
                "src/server/turn.rs".to_owned(),
                format!("let client = {UPSTREAM_TRANSPORT}::Client::new();\n"),
            ),
            (
                "src/server/body.rs".to_owned(),
                format!("//! {UPSTREAM_TRANSPORT} の話をしているだけ\nlet value = 1;\n"),
            ),
            (
                "src/prompt/mod.rs".to_owned(),
                "pub fn plain() {}\n".to_owned(),
            ),
        ];

        assert_eq!(
            direct_transport_users(&planted),
            vec!["src/server/turn.rs".to_owned()],
            "植えた迂回路をちょうど1件見つけること"
        );
    }
}
