//! **上流について観測できたことを、実行中も読める形で持つ**（作業単位 G-3b /
//! [ADR-0039](../docs/adr/0039-two-upstreams-for-two-jobs.md)）。
//!
//! # なぜ要るのか
//!
//! **着手前、personacore は「上流が何であるか」を実行中ほとんど何も知らなかった。**
//! `App::bind` が一覧を取り、`measured::report` に渡した後で**捨てていた。**
//! 観測は**起動ログにしか残らない**——つまり
//!
//! * **5分ごとの引き直し**（ADR-0020）で上流が別のモデルを載せ直しても、
//!   いま何と突き合わせているのかを読む手段が無い。
//! * 上流が停止して答えなくなっても、**予算は現在値を保つ**ので
//!   （`context::refresh_once`）**外からは何も変わって見えない。**
//!
//! `GET /admin/upstream`（作業単位 G-10）と画面（D-9）がここを読む。
//!
//! # ⚠ 上流が名乗った綴りは、**ファイル名だけ**を持つ（**絶対制約に触れる1点**）
//!
//! **`llama-server` の `/v1/models` の `id` は gguf の絶対パス**である
//! （読んだ綴りは `llm::response::EffectiveContext::model_id` / **いまログにも出ている**。
//! ここから見えるのは [`UpstreamProbe::catalogue`] に並ぶ同じ綴りのほう）。
//!
//! ## 決定（作業単位 B-45 / 2026-09-17）——**頼んだ名前と、答えた名前を分ける**
//!
//! **`C-18` が実測した**（2026-09-17）——**`llama serve` は `model` の欄を無視して、
//! いま載っているモデルで答える。** 画面に出ていたのは**頼んだ既定名**だけだったので、
//! **実際に答えていたのが別のモデルでも、そうとは読めなかった**
//! ——手掛かりは `answered_by: openai-sole-entry` という**分類**だけで、
//! **名乗るものを1つも持っていなかった**（それが G-10 までの決定である / 下記）。
//!
//! | | 欄 | 中身 |
//! |---|---|---|
//! | **頼んだ名前** | [`UpstreamObservation::model`] | personacore が依頼する綴り。**意味は1バイトも変えていない** |
//! | **答えた名前** | [`UpstreamObservation::answered_name`] | 上流が名乗った id の**ファイル名だけ**。`None` は「名乗っていない／名指しできない」 |
//!
//! **ディレクトリ部分は保持もしない。** 切り落とすのは**継ぎ目**
//! （`server::context::probe_upstream` が [`UpstreamProbe`] を作る所 / [`name_that_answered`]）で、
//! **[`UpstreamFacts`] にも [`UpstreamObservation`] にも応答本文にも、ディレクトリは1文字も入らない。**
//!
//! **⚠ 「切る」のはパスに見える綴りだけ**（[`looks_like_a_path`] / レビュー是正 / 2026-09-17）。
//! **無条件に切ると、`mistralai/devstral-small-2-2512` のような `vendor/model` 形が
//! `devstral-small-2-2512` に短くなり、上流が一度も名乗っていない綴りを画面が出す**
//! ——しかも注記は「上流が自分で名乗った名前」と言うので、**画面が嘘をつく。**
//! **代わりに残った穴**——**拡張子の無い相対 POSIX パスは切らない。**
//! [`looks_like_a_path`] の doc が権威。
//!
//! **⚠ 絶対制約は「内部パス・接続文字列を出さない」であって、
//! 「上流のモデル名を出さない」ではない。** 残すファイル名は
//! **宛先（現に出している）と同じ種類の事実**であり、
//! **出さないと「どのモデルが答えたか」を読む手段が画面に1つも無い。**
//!
//! ## ⚠ 見張り番3本の**意味が変わった**（司令塔が承認 / 2026-09-17）
//!
//! **着手前の3本は「上流が名乗った綴りを1文字も持たない」を見ていた。**
//! いまは**「ディレクトリ部分を持たない」**を見る。**3本とも、
//! ファイル名が現に保持されている／応答に出ていることまで見る**ので、
//! **切り落としを外す変異**でも**答えた名前を丸ごと落とす変異**でも赤になる（空振りしない）:
//!
//! * `tests/server_upstream_facts.rs::the_held_facts_never_carry_the_upstreams_own_model_id`
//! * `tests::the_catalogue_the_upstream_listed_is_not_kept`
//! * `tests/server_admin_upstream.rs::the_response_never_carries_the_upstreams_own_model_id`
//!
//! **⚠ この3本は層が違う**（保持した器の `Debug` / この器の単体 / 応答本文）ので、
//! **1つの変異で3本同時に赤くなることはない**（レビュー是正 / 2026-09-17）。
//! **「3本が見ている」を「3本とも赤になる」と読まないこと**
//! ——G-10 の `m3`（保持したうえで手書き `Debug` から隠す）は**応答本文の1本だけ**が赤だった。
//!
//! **⚠ `/` と `\` の両方で測ること。** 片方しか見ない切り落としは、
//! もう片方の形の絶対パスを丸ごと通す。
//! **そして `Debug` も JSON も `\` を `\\` へ逃がす**ので、
//! **生の綴りだけを探す検査は Windows 形で空振りする**——**逃がした形も一緒に探すこと。**
//!
//! ## 残っている理由（G-3b / G-10 が挙げた3つのうち、2つは今も生きている）
//!
//! 1. **絶対制約**——「エラーレスポンスに内部パス・接続文字列を含めない」。
//!    **ディレクトリを落とすのはこの1点のため**で、**ここだけは変えていない。**
//! 2. ~~要るのは「どちらの読み手が答えたか」までである~~
//!    **→ 足りなかった**（`C-18`）。**どの読み手が答えたかは、何が答えたかを言わない。**
//! 3. **写しを作らない**（CLAUDE.md「同じ発見を2箇所に書かない」）。
//!    **ログの1行（`llm::lmstudio` の `upstream_model_id`）は絶対パスのまま**で、
//!    **こちらはファイル名だけ**——**同じ事実の別の切り口**であって、写しではない。
//!    **調査でディレクトリまで要るときは、今までどおりログを読む。**
//!
//! **⚠ 「型で表現できなくした」とは書けない**（CLAUDE.md / `0012b` の教訓）。
//! 止めているのは型ではなく、[`name_that_answered`] の1行と上の見張り番3本である。
//! **[`UpstreamProbe::catalogue`] をそのまま [`UpstreamObservation`] に残す形は今も書ける**
//! ——`llama-server` の一覧は**gguf の絶対パス1件**なので、同じ綴りがそのまま外へ出る。
//!
//! ## ⚠ `Debug` を見る検査は、応答本文の検査の代わりにならない（**G-10 の実測**）
//!
//! **観測の手段が `#[derive(Debug)]` 1本しか無い**ので、
//! **`Debug` を手書きにして新しい欄を印字しない形は素通りする**
//! ——上流の綴りを [`UpstreamObservation`] に保持したまま **1995 全緑・exit 0**
//! （**実測**／作業単位 G-3b のレビュー是正 / 2026-09-08）。
//!
//! **G-10 が応答本文そのものを読む見張り番を別に置いて、その申し送りを閉じた。**
//! **実測**（変異 `m3`。**上流が名乗った綴りを保持し、
//! `Debug` を手書きにしてその欄を印字しない形**）:
//!
//! | 見張り番 | `m3` での色 |
//! |---|---|
//! | `the_held_facts_never_carry_the_upstreams_own_model_id`（`Debug`） | **緑** |
//! | `tests::the_catalogue_the_upstream_listed_is_not_kept`（`Debug`） | **緑** |
//! | **`the_response_never_carries_the_upstreams_own_model_id`（応答本文）** | **赤** |
//!
//! **2004 passed / 1 failed / exit 101** で、**赤いのはその1本だけ。**
//! **その1本を外すと 2004 全緑**（`m3_noguard` / exit 0）なので、
//! **この変異についてはそれが唯一の権威である。**
//!
//! **⚠ この表は「持たない」と決めていた頃の形で測ったもの**（2026-09-08）。
//! **いま同じ変異を当てるなら「ディレクトリごと保持する形」になる**が、
//! **`Debug` と応答本文が別の主張であることは変わらない。**
//!
//! ## 見張り番と、実際に当てた変異（**作業単位 B-45 / 2026-09-17**）
//!
//! **複製ツリー（`pcf1-b45`）で直列に。着手後の全緑は 2053 passed / exit 0。**
//! **下はすべて exit 101。** 右端は**その赤を外して緑に戻ることまで測った**もの。
//!
//! | 変異 | 赤 | その見張り番を外すと |
//! |---|---|---|
//! | `file_name` が切り落とさない（`id` を返す） | 4 | **exit 0** |
//! | `file_name` が `/` しか見ない（`\` を見ない） | 4 | **exit 0** |
//! | 答えた名前を**頼んだ名前で埋める**（一致の枝を `if true`） | 5 | **exit 0** |
//! | 答えが無い面が `Some("")` を名乗る（[`UpstreamProbe::unanswered`]） | 4 | **exit 0** |
//! | **完全一致の枝を消す**（依頼した綴りも切り落とす） | 2 | **exit 0** |
//! | **継ぎ目の経路を外す**（`probe_upstream` が `None` を入れる） | **6** | **exit 0** |
//! | [`UpstreamView`] で**頼んだ名前と答えた名前を入れ替える** | 4 | **exit 0** |
//!
//! **画面の側**（`web` / 着手後は 976 passed / exit 0。下は exit 1）:
//!
//! | 変異 | 赤 | 外すと |
//! |---|---|---|
//! | 食い違いの一言を**常に**出す | 2 | **exit 0** |
//! | 食い違いの一言を**一度も**出さない | 5 | **exit 0** |
//! | 名乗らなかった面を**頼んだ名前で埋める** | 6 | **exit 0** |
//!
//! > **⚠ 「継ぎ目の経路を外す」で赤くなった 6 本は、全部 `App::bind` を通る腕である。**
//! > **単体テスト（この器の入口）は 1 本も赤くならない**
//! > ——`bind_with_clients` も単体の器も、`probe_upstream` を通らない。
//! > **本番の入口を通る腕を外すと、この作業単位は丸ごと無防備になる。**
//!
//! ## 見張り番と、実際に当てた変異（**レビュー是正 / 2026-09-17**）
//!
//! **複製ツリー（`pcf1-b45fixbc32`）で直列に。是正後の全緑は 2054 passed / exit 0。**
//! **測ったもの一式は `<measurements>\b45-2026-09-17\fix-agent-bc3225e4\`**
//! （**適用したコマンドそのもの**＝`batch1.py` / `batchw.py` を、ログの隣に置いてある）。
//!
//! | 変異 | 赤 | exit | 外すと |
//! |---|---|---|---|
//! | **一覧が1件なら無条件に切る**（[`looks_like_a_path`] の門を外す） | 1 | 101 | **exit 0 / 2054 全緑** |
//! | **一度も切らない**（[`looks_like_a_path`] が常に `false`） | **4** | 101 | —— |
//! | 手掛かり `\` を落とす | 1 | 101 | —— |
//! | 手掛かり `/` で始まるを落とす | 1 | 101 | —— |
//! | 手掛かり `.gguf` で終わるを落とす | 1 | 101 | —— |
//! | 手掛かり 2文字目の `:` を落とす | 1 | 101 | —— |
//! | **継ぎ目が一覧の各要素をファイル名に切る**（`context::probe_upstream`） | 1 | 101 | **exit 0** |
//!
//! > **⚠ 「一度も切らない」の 4 赤には、絶対制約の側の3本が入っている**
//! > （`the_held_facts_never_carry_the_upstreams_own_model_id` /
//! > `tests::the_catalogue_the_upstream_listed_is_not_kept` /
//! > `tests/server_admin_upstream.rs::the_response_never_carries_the_upstreams_own_model_id`）。
//! > **切る側と切らない側の両向きが、別々の見張り番に押さえられている。**
//!
//! > **⚠ 継ぎ目の変異は等価ではない**（是正前は 2053 全緑だった）——
//! > `measured::check` は**完全一致**なので、`vendor/model` 形が切られた一覧から消え、
//! > 判定が `Drift::Other` から `Drift::Absent` へ倒れる。**調査先がまるで違う。**
//! > 見張り番は `tests/server_upstream_facts.rs::
//! > the_seam_hands_the_measured_check_the_catalogue_as_the_upstream_listed_it`。
//!
//! **画面の側**（`web` / 是正後は 980 passed / exit 0。下は exit 1）:
//!
//! | 変異 | 赤 | 外すと |
//! |---|---|---|
//! | `answeredNameLabel` が `answered_by` を見ない（常に「名乗っていません」） | 2 | **exit 0** |
//! | 「名乗っていません」と「名指しできません」を**入れ替える** | **8** | —— |
//! | 行の組み立てが `answered_by` を渡さない（`null` を渡す） | **1** | **exit 0** |
//! | 画面の注記2つ（切り落とし／名指しできない）を落とす | 2 | —— |
//!
//! > **⚠ 同値のフィクスチャでは、2つの欄の入れ替えが観測できない**
//! > （**この変異検査で分かった** / 2026-09-17）。
//! > **`the_two_upstreams_come_back_named_apart` と
//! > `the_two_upstreams_are_observed_apart_through_the_real_bind` は、
//! > 上流が依頼した綴りをそのまま一覧に持つ形**（LM Studio の `/api/v0/models`）**なので、
//! > `model` と `answered_name` が同値になる。** **入れ替えても両方とも緑。**
//! > **捕まえているのは、綴りが違う腕**
//! > （`the_response_never_carries_the_upstreams_own_model_id` と、
//! > 器の側の `a_face_that_answered_reports_who_answered_and_what_it_declared`）。
//!
//! # ⚠ 宛先の綴りは持つ。ただし認証情報・クエリ・フラグメントは落とす
//!
//! **`endpoint` は接続文字列そのもの**（`http://127.0.0.1:1234/v1/chat/completions`）で、
//! **G-10 がこれを HTTP へ出す。**
//!
//! **決定: 宛先は持ち、認証情報（userinfo）を `***@` に伏せ、クエリとフラグメントを落とす**
//! （[`endpoint_for_view`]。G-3b では userinfo だけを落とす `redact_userinfo` だった。
//! **B-71 / Issue origin#100 で、`?token=…` がそのまま出ていたのを塞いで改名した**。
//! 落とす中身はログと同じ `llm::endpoint_for_log`）。
//! **以下の実測の記録に出る `redact_userinfo` は、改名前の名前である。**
//!
//! * **持つ理由**——「いまどこと話しているか」はこの観測の存在理由そのもの。
//!   **2つの上流を別々に見る**（ADR-0039 §決定1）のに、宛先を落としたら何も読めない。
//! * **落とす理由**——**絶対制約**「エラーレスポンスに内部パス・**接続文字列**を含めない」。
//!   `http://user:pass@127.0.0.1:1234` は**現に設定できる**
//!   （`llm::lmstudio` の `normalize_base` はスキーム・ホスト・空白/制御文字しか見ない）。
//!   **モデルの綴りと同じ判断で、出さない一番確実な方法は持たないこと。**
//! * **上流への接続は1バイトも変えない。** 落とすのは**外へ出す側（観測とログ / B-71）だけ**で、
//!   実際に叩く URL（`llm::LmStudioConfig::endpoint`）は設定どおり。
//!   **`normalize_base` で弾く形は採らなかった**——
//!   弾くと**基本認証を要求する上流の前に立てなくなる。**
//!   ~~**起動ログには設定どおりの綴りが出る**（ログには出してよい / 絶対制約）。~~
//!   **→ B-71 で撤回。** ログにも同じものを落として出す（`llm::endpoint_for_log`）——
//!   ログはファイルへ残り、人に渡される。
//!
//! **⚠ これは「型で表現できなくした」ではない。** [`UpstreamObservation::new`] が
//! [`endpoint_for_view`] を通すだけなので、**その1行を外せば綴りはそのまま入る。**
//! 止めているのは**見張り番2本**——
//! `tests/server_upstream_facts.rs::the_held_facts_never_show_the_endpoints_credentials`
//! （**本番の入口を通す**）と `tests::credentials_are_not_kept_in_the_endpoint`（この器の入口）。
//!
//! **実測**——`redact_userinfo` の呼びを外すと**この2本が赤**（1993 passed / 2 failed）。
//! **2本とも外すと 1993 全緑**なので、赤にしているのはこの2本である。
//! **⚠ 純粋関数の表**（`tests::only_the_userinfo_query_and_fragment_are_redacted`。B-71 で改名）
//! **はその変異で緑のまま。** 器がその関数を通しているかを、あちらは見ていない。
//!
//! **⚠ G-10 が足した3本目は、`redact_userinfo` を外す変異に限れば単独の権威ではない**
//! （作業単位 G-10 / 2026-09-08）。
//! `tests/server_admin_upstream.rs::the_response_never_shows_the_endpoints_credentials`
//! は**応答本文**を読むが、**その変異（`m4`）では上の2本も赤になる**
//! ——**3本目を外しても exit 101 / 2 赤のまま**（`m4_noguard`）。
//!
//! **⚠ しかし「応答本文だけが捕まえる漏れ方」は在る**（G-10 のレビューが実測 / 2026-09-08）。
//! **[`UpstreamObservation`] に「落とす前の綴り」の欄を足し、その欄の `Debug` を
//! 手書きで隠して、[`UpstreamView`] の宛先をそこから取る**と、
//! **応答本文だけが赤になる**（**2004 passed / 1 failed / exit 101**。
//! `Debug` を見る `server_upstream_facts` の1本も、`endpoint()` を見る単体の1本も緑）。
//! **つまりこの3本目は写しではなく、その族に対する唯一の関所である。外さないこと。**
//!
//! > **⚠ 作業者が「道が見つからなかった」と書いたのは、探し方の問題だった。**
//! > **同じ手法（保持したうえで `Debug` から隠す）を、すぐ上の `model` の欄で自分が成功させていた**
//! > （この節の `m3`）。**宛先の側では「いま [`UpstreamView`] から何が触れるか」しか探していない。**
//! > **変異の空間には [`UpstreamObservation`] を変える形が入っている。**
//!
//! # 一覧（`catalogue`）もここには残らない
//!
//! 一覧には上流が名乗る綴りがそのまま並ぶ（＝**同じ絶対パスが入る**）。
//! [`UpstreamProbe`] は起動と引き直しの**その場**でだけ持ち、
//! [`crate::measured`] の判定に通したら捨てる。**残るのは数だけ**（[`FaceTally`]）。
//!
//! # 採用した値はここに写さない
//!
//! [`UpstreamObservation::queried_context_tokens`] は**上流が申告した値**であって、
//! **いま使っている値ではない。** 使っている値は
//! `context::SharedBudget`（本文）と `context::SharedSubContext`（抽出）が持つ
//! ——**写すと、片方を直したときにもう片方が黙って古びる。**
//!
//! **2つが食い違う状態は正常にありうる**（引き直しが失敗した回は、
//! 予算は現在値を保ち、申告値は「答えなし」になる）。**その食い違いこそが読みたいもの。**

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa::openapi::schema::Object;

use super::error::ErrorBody;
use super::launch::{FaceState, LaunchStates};
use super::{AppState, openapi};
use crate::llm::{AnsweredBy, KeyOverPlainHttp, endpoint_for_log, endpoint_has_userinfo};
use crate::measured::{FaceTally, ModelFace, judge_face};

/// **上流への照会1回ぶんの生の観測。** 起動時と引き直しが同じ形で作る。
///
/// **持ち回るのはこの関数の中だけ。** `catalogue` は
/// [`crate::measured`] の判定に通したらそこで捨てる（このモジュールの doc）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UpstreamProbe {
    /// 上流が持っているモデルの綴り。**`None` は「聞けなかった」。**
    pub(crate) catalogue: Option<Vec<String>>,
    /// 上流が申告したコンテキスト長。**`None` は「読めなかった」。**
    pub(crate) context_tokens: Option<u32>,
    /// どの読み手が答えたか。**`None` は「往復そのものが失敗した」。**
    pub(crate) answered_by: Option<AnsweredBy>,
    /// **上流が名乗った名前**（[`name_that_answered`] / 作業単位 B-45）。
    ///
    /// **ディレクトリは既に落ちている。** 作るのは
    /// `server::context::probe_upstream` で、**そこが継ぎ目である。**
    /// **`None` は「名乗っていない／名指しできない」。**
    pub(crate) answered_name: Option<String>,
    /// **照会が鍵を断られたか**（401 / 403 / ADR-0056 §決定4 / 作業単位 A-13b-1）。
    ///
    /// **`answered_by: None` だけでは「届かない」と「断られた」が区別できない**（origin#77 では
    /// どちらも `answered_name: null` だった）。判定は `LlmError::refused_the_key`。
    pub(crate) unauthorized: bool,
    /// **鍵を平文の `http://` で別の機へ送る扱い**（ADR-0056 §決定5 / 作業単位 A-13b-2）。
    ///
    /// **照会の後にクライアントから読む**（`llm::LmStudioClient::key_over_plain_http`）——
    /// ホスト名の宛先は、照会の名前解決で判定が決まる。`None` は「判定しない宛先」か「まだ解決していない」。
    pub(crate) key_over_plain_http: Option<KeyOverPlainHttp>,
    /// **いつ・どう確かめたか**（作業単位 B-66 / Issue origin#94）。**`None` は「照会していない」。**
    ///
    /// **作るのは照会の往復を読んだ1箇所だけ**（`server::context::probe_with`）。
    /// 照会しなかった面（自分で起こす面の起動時・注入の入口・差し替えの直後）は
    /// [`Self::unanswered`] の `None` のまま —— **「照会していない」と「照会したが答えない」を、ここで割る。**
    pub(crate) check: Option<UpstreamCheck>,
}

impl UpstreamProbe {
    /// **聞いていない／聞けなかった。**
    ///
    /// 2つを区別しない（どちらも「答えが無い」）。
    /// **区別が要るのは `measured` の側だけ**で、そちらは `None` の一覧を
    /// [`crate::measured::ModelCheck::Unanswered`] に倒す。
    ///
    /// > **画面が2つを区別するための欄は [`Self::check`]**（作業単位 B-66）。ここでは `None`
    /// > （照会していない）にしておき、照会したが答えなかった回は `probe_with` が `Some` を入れる。
    pub(crate) const fn unanswered() -> Self {
        Self {
            catalogue: None,
            context_tokens: None,
            answered_by: None,
            answered_name: None,
            unauthorized: false,
            key_over_plain_http: None,
            check: None,
        }
    }
}

/// **最後の照会の結果の種類**（作業単位 B-66 / Issue origin#94）。
///
/// **画面が「答えない」「断られた」「送っていない」「答えた」を言い分けるための欄**
/// （`GET /admin/upstream` の `check_result`）。**「照会していない」はこの型に入れない**
/// （[`UpstreamProbe::check`] の `None` が言う。応答の綴りは [`NOT_CHECKED`]）。
///
/// | 値 | 起きたこと |
/// |---|---|
/// | `Answered` | 往復が成功した（一覧が読めた。コンテキスト長が読めたかは問わない） |
/// | `NoAnswer` | **繋がらない・黙っている**（`Connect` / `Timeout` / `Transport` —— 応答が1つも来なかった） |
/// | `NoModelList` | **届いたが、一覧が読めない**（401 / 403 以外のステータス・読めない本文など） |
/// | `Refused` | 鍵を断られた（401 / 403 / `LlmError::refused_the_key`） |
/// | `KeyNotSent` | 鍵を平文で外へ送る承認が無いので、送らなかった（`LlmError::KeyNotApproved`） |
///
/// # `NoAnswer` と `NoModelList` を割る理由（レビュー是正）
///
/// **モデル一覧は上流に求める「任意」の能力**（ADR-0038 §決定1）。一覧の口を持たない（あるいは一覧だけ 500 を
/// 返す）上流にコンテキスト長を書いて繋ぐと、**会話は通るのに照会は毎回失敗する。** 画面は `NoAnswer` を
/// 「繋がりません」の異常の帯にするので、まとめると帯が出続ける（D-12b-2 の2周目が避けた形の再来）。
/// **`NoModelList` は中立**（「一覧に答えません」）にする。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckOutcome {
    /// 上流が答えた。
    Answered,
    /// 繋がらない・黙っている（応答が1つも来なかった）。
    NoAnswer,
    /// 届いたが、モデル一覧が読めない（一覧の口が無い・一覧だけ失敗する など）。
    NoModelList,
    /// 鍵を断られた（401 / 403）。
    Refused,
    /// 承認が無いので鍵を送らなかった（上流は鍵を見ていない）。
    KeyNotSent,
}

/// **「照会していない」の綴り**（`check_result` の先頭の値。値は全部で6つ / 作業単位 B-66）。
///
/// [`CheckOutcome`] の値ではない —— 照会していない面は [`UpstreamProbe::check`] が `None`。
pub const NOT_CHECKED: &str = "not_checked";

impl CheckOutcome {
    /// 全部の値（仕様書の閉じた綴りを作る / `server::openapi`）。
    pub const ALL: [Self; 5] = [
        Self::Answered,
        Self::NoAnswer,
        Self::NoModelList,
        Self::Refused,
        Self::KeyNotSent,
    ];

    /// 応答の綴り。**綴りの正はここ。**
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::NoAnswer => "no_answer",
            Self::NoModelList => "no_model_list",
            Self::Refused => "refused",
            Self::KeyNotSent => "key_not_sent",
        }
    }

    /// **往復の失敗から種類を決める。純粋関数**（失敗が無ければ `Answered`）。
    ///
    /// **判定はここ1箇所**（`server::context::probe_with` が呼ぶ）。断られたかの規則は
    /// [`crate::llm::LlmError::refused_the_key`] に預ける（`unauthorized` と同じ規則を読む）。
    #[must_use]
    pub(crate) const fn of(failure: Option<&crate::llm::LlmError>) -> Self {
        match failure {
            None => Self::Answered,
            Some(crate::llm::LlmError::KeyNotApproved) => Self::KeyNotSent,
            Some(err) if err.refused_the_key() => Self::Refused,
            // **応答が1つも来なかった**（`llm::lmstudio` の `models` は、この3つで2つ目の読み手を投げない）。
            Some(
                crate::llm::LlmError::Connect(_)
                | crate::llm::LlmError::Timeout(_)
                | crate::llm::LlmError::Transport(_),
            ) => Self::NoAnswer,
            // **届いたが一覧が読めない**（404 / 500 / 読めない本文 など）。
            Some(_) => Self::NoModelList,
        }
    }
}

/// **いつ・どう確かめたか**（作業単位 B-66）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpstreamCheck {
    at: chrono::DateTime<chrono::Utc>,
    outcome: CheckOutcome,
}

impl UpstreamCheck {
    /// **いま確かめた**（往復を読み終えた時刻）。
    #[must_use]
    pub(crate) fn now(outcome: CheckOutcome) -> Self {
        Self {
            at: chrono::Utc::now(),
            outcome,
        }
    }

    /// 確かめた時刻。
    #[must_use]
    pub const fn at(&self) -> chrono::DateTime<chrono::Utc> {
        self.at
    }

    /// 結果の種類。
    #[must_use]
    pub const fn outcome(&self) -> CheckOutcome {
        self.outcome
    }
}

/// **観測に持つ宛先。認証情報（userinfo）を `***@` に伏せ、クエリとフラグメントを落とす。純粋関数**
/// （作業単位 G-3b のレビュー是正で userinfo を、B-71 / Issue origin#100 でクエリとフラグメントを /
/// このモジュールの doc）。
///
/// `http://user:pass@127.0.0.1:1234/v1/chat/completions?token=…#…`
/// → `http://***@127.0.0.1:1234/v1/chat/completions`
///
/// # 落とすのはログと同じ関数
///
/// **落とすのは [`crate::llm::endpoint_for_log`]**（ログに出す宛先と同じもの / 2つ目を作らない）。
/// ここが足すのは `***@` の印だけ。読めない綴りは `<読めない宛先>`（ログと同じ）。
///
/// # 消したことを隠さない
///
/// **`***@` を残す。** 丸ごと消すと
/// **「この宛先には認証情報が設定されている」という、それ自体が診断の手掛かりになる事実**
/// まで消える。**クエリ・フラグメントには印を残さない**（画面の宛先は「どこへ繋いでいるか」で、
/// クエリの有無は切り分けの手掛かりにならない。ログと同じ形）。
///
/// # どこを見るか
///
/// **送信と同じパーサ（WHATWG）で読む**（`llm::endpoint_has_userinfo`）。
/// **経路の中の `@`**（`http://host/p@th`）**は userinfo ではない。**
fn endpoint_for_view(endpoint: &str) -> String {
    let shown = endpoint_for_log(endpoint);
    if endpoint_has_userinfo(endpoint) {
        // `endpoint_for_log` が読めた綴りなので、`://` は必ずある（読めなければ印を足さない）。
        shown.replacen("://", "://***@", 1)
    } else {
        shown
    }
}

/// **上流が名乗った id から、ファイル名だけを取り出す。純粋関数**（作業単位 B-45）。
///
/// 区切りは**最後の `/` または `\`**。区切りが無ければ id そのもの。
///
/// **⚠ 両方見ること。** `llama-server` が名乗るのは**その機のパス**なので、
/// **Windows で走らせれば `\` になる。**
/// **片方しか見ない切り落としは、もう片方の形の絶対パスを丸ごと通す。**
/// **その綴りは、置き場所つきの名前に見えるか。純粋関数**（作業単位 B-45 のレビュー是正）。
///
/// # なぜ要るのか
///
/// **一覧が1件のときに無条件で切ると、`mistralai/devstral-small-2-2512` のような
/// `vendor/model` 形の綴りが `devstral-small-2-2512` に短くなる。**
/// **上流はその綴りで名乗っているので、画面は
/// 「頼んだ名前と違うものが答えています」と、起きていないことを言う。**
///
/// # 手掛かりは4つ（**どれか1つでもパスと見なす**）
///
/// | 手掛かり | 例 |
/// |---|---|
/// | `\` を含む | `models\deep` |
/// | `/` で始まる | `/models/deep` |
/// | `.gguf` で終わる | `models/deep.gguf` |
/// | 2文字目が `:`（ドライブ文字） | `C:/models/deep` |
///
/// # ⚠ 守れないもの
///
/// **拡張子の無い相対 POSIX パス**（`../../secret/foo`）**は切らない。**
/// 手掛かりが1つも当たらないので `vendor/model` と区別が付かない。
/// **上流が `/v1/models` にその形で名乗った回は、ディレクトリ部分が画面に出る。**
/// **これは推測を足さないことと引き換えに残した穴で、塞いだとは書けない。**
fn looks_like_a_path(id: &str) -> bool {
    id.contains('\\')
        || id.starts_with('/')
        || id.ends_with(".gguf")
        // **ドライブ文字**（`C:\` / `C:/`）。`as_bytes` で見るのは、
        // 添字 1 が文字の境界とは限らないため（`あ:` のような綴りでも落ちない）。
        || id.as_bytes().get(1) == Some(&b':')
}

fn file_name(id: &str) -> &str {
    match id.rfind(['/', '\\']) {
        // **区切りは 1 バイトの ASCII** なので、この添字は必ず文字の境界。
        Some(at) => &id[at + 1..],
        None => id,
    }
}

/// **上流が名乗った名前。** `None` は「名乗っていない／名指しできない」（作業単位 B-45）。
///
/// # なぜ要るのか
///
/// **`llama serve` は `model` の欄を無視して、いま載っているモデルで答える**
/// （`C-18` が実測 / 2026-09-17）。**頼んだ名前しか出していないと、
/// 別物が答えていても画面からは読めない。**
///
/// # 決め方（**推測を1つも足さない**）
///
/// | 一覧 | 名乗る名前 | なぜ |
/// |---|---|---|
/// | **依頼した綴りが在る** | **依頼した綴りそのまま** | 上流が自分でその綴りを名乗った。**切り落とさない**——`vendor/model` のような綴りが短くなると、**「頼んだ名前と違うものが答えた」と画面が嘘をつく** |
/// | **1件だけ**で、**パスに見える**（[`looks_like_a_path`]） | **その1件のファイル名** | 答えられるモデルが1つしかないなら、答えるのはそれ（[`AnsweredBy::CompatSoleEntry`] と同じ推論）。**置き場所は外へ出さない**（絶対制約） |
/// | **1件だけ**で、**パスに見えない** | **その綴りそのまま** | `mistralai/devstral-small-2-2512` を短くすると、**上流が一度も名乗っていない綴りを画面が出し、「頼んだ名前と違うものが答えています」と嘘をつく**（レビュー是正 / 2026-09-17） |
/// | **2件以上あって、一致も無い** | **`None`** | **どれが答えたか決められない。** 名乗らないほうが、違う名前を出すより良い |
/// | **空** | **`None`** | 同上 |
///
/// **⚠ 呼ぶのは往復が成功した回だけ**（`server::context::probe_upstream`）。
/// 往復そのものが失敗した面は [`UpstreamProbe::unanswered`] が `None` にする。
///
/// # ディレクトリはここで落ちる（**継ぎ目**）
///
/// **ここから先**（[`UpstreamProbe`] → [`UpstreamObservation`] → 応答本文）**に、
/// パスに見える綴りのディレクトリは1文字も入らない**
/// （**絶対制約: 内部パスを外へ出さない** / このモジュールの doc）。
///
/// **⚠ 「1文字も入らない」と無条件には書けない**（レビュー是正 / 2026-09-17）。
/// **[`looks_like_a_path`] が挙げる手掛かりに1つも当たらない綴りは切らない**ので、
/// **拡張子の無い相対 POSIX パス**（`../../secret/foo`）**はそのまま外へ出る。**
/// **`vendor/model` と区別する手段が無いからで、塞いだとは書けない。**
///
/// **区切りで終わる id**（`/models/`）**は空になるので `None`** にする
/// ——`Some("")` は「名乗ったが空だった」という在りえない状態を画面に出す。
pub(super) fn name_that_answered(model: &str, catalogue: &[String]) -> Option<String> {
    // **上流が依頼した綴りを自分で名乗った。** 推測が1つも無いので、そのまま出す。
    if catalogue.iter().any(|id| id == model) {
        return Some(model.to_owned());
    }
    // **一覧そのものが1件のときだけ。** 2件以上なら、どれが答えるか分からない
    // （`llm::response::parse_effective_context_length` の2段目と同じ条件）。
    let [only] = catalogue else {
        return None;
    };
    // **パスに見えないなら、切らずにそのまま名乗る**（レビュー是正 / [`looks_like_a_path`]）。
    // **`vendor/model` 形を短くすると、画面が起きていないことを言う。**
    if !looks_like_a_path(only) {
        return Some(only.clone());
    }
    let name = file_name(only);
    (!name.is_empty()).then(|| name.to_owned())
}

/// **上流1面ぶんの観測**（本文 / 抽出）。
///
/// **面ごとに別の値になりうる**——`PERSONACORE_SUB_LLM_ENDPOINT` を設定すれば
/// 宛先まで別になる（ADR-0039 §決定1）。**取り違えると、
/// 「抽出が死んでいるのに本文の数を見て正常と読む」形になる。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamObservation {
    endpoint: String,
    model: String,
    answered_name: Option<String>,
    queried_context_tokens: Option<u32>,
    answered_by: Option<AnsweredBy>,
    measured: FaceTally,
    /// **最後の照会が鍵を断られたか**（[`UpstreamProbe::unauthorized`]）。
    unauthorized: bool,
    /// **鍵を平文の `http://` で別の機へ送る扱い**（[`UpstreamProbe::key_over_plain_http`]）。
    key_over_plain_http: Option<KeyOverPlainHttp>,
    /// **最後にいつ・どう確かめたか**（[`UpstreamProbe::check`] / 作業単位 B-66）。`None` は「照会していない」。
    check: Option<UpstreamCheck>,
}

impl UpstreamObservation {
    /// 起動時の観測。**`measured` の内訳は呼び出し側が渡す。**
    ///
    /// # なぜ内訳を受け取るのか（**判定を作る場所を1つにする**）
    ///
    /// 起動時は [`crate::measured::report`] が**一度だけ**判定して報告し、
    /// **その戻り値をそのまま**ここへ入れる。ここで判定し直すと、
    /// **同じ事実の権威が2つ**になる（CLAUDE.md）。
    ///
    /// **⚠ 本文の面は、一覧を抽出側から借りることがある**
    /// （宛先が同じで本文の照会が通らなかった回 / `server::App::bind` の表）。
    /// 借りた後の一覧で判定するのは `report` なので、
    /// **ここで自分の `probe.catalogue` から作り直すと答えが変わる。**
    ///
    /// **⚠ 借りるのは一覧だけ。[`Self::answered_name`] は借りない**
    /// （レビュー是正 / 2026-09-17）。**その回の `probe` は
    /// [`UpstreamProbe::unanswered`] なので `None` のまま**であり、
    /// **抽出側が名乗った綴りを本文の面が名乗ることは無い。**
    /// **⚠ そのとき突き合わせの内訳だけが「答えた」ように見えうる**
    /// （借りた一覧で判定するので `Unanswered` にならない）。
    /// **同じ回に [`Self::answered_by`] も `null` になる**ので、
    /// **画面は隣の行で「上流が答えませんでした」と言う**
    /// ——**内訳だけを見て「この面の上流が答えた」と読む形にはならない。**
    #[must_use]
    pub(crate) fn new(
        endpoint: &str,
        model: &str,
        probe: &UpstreamProbe,
        measured: FaceTally,
    ) -> Self {
        Self {
            // **認証情報・クエリ・フラグメントはここで落とす**（絶対制約 / このモジュールの doc）。
            // **構築点はこの1つ**——`unqueried` も `refreshed` もここを通るか、
            // 落とした後の綴りを写すかのどちらかである。
            endpoint: endpoint_for_view(endpoint),
            model: model.to_owned(),
            // **上流が名乗った名前。** ディレクトリは継ぎ目で落ちている
            // （[`name_that_answered`] / このモジュールの doc）。
            answered_name: probe.answered_name.clone(),
            queried_context_tokens: probe.context_tokens,
            answered_by: probe.answered_by,
            measured,
            unauthorized: probe.unauthorized,
            key_over_plain_http: probe.key_over_plain_http,
            check: probe.check,
        }
    }

    /// **一度も聞いていない面**（`App::bind_with_clients` の注入経路と、
    /// 走ったままの差し替え / 作業単位 F-4b-2）。
    ///
    /// 判定は全部「聞けなかった」になる。**それが事実である。**
    ///
    /// **⚠ F-4b-1 までは `cfg_attr` で黙らせていた**（呼ぶのがテスト専用の入口だけだった）。
    /// **差し替え（`context::install_face`）が本番から呼ぶようになったので外した。**
    #[must_use]
    pub(crate) fn unqueried(face: ModelFace, endpoint: &str, model: &str) -> Self {
        let probe = UpstreamProbe::unanswered();
        let measured = judge_face(face, model, None);
        Self::new(endpoint, model, &probe, measured)
    }

    /// **引き直しで動く3つだけを差し替えた新しい観測。**
    ///
    /// 宛先とモデルの綴りは**起動時に決まっていて動かない**ので写す。
    ///
    /// **判定は [`judge_face`] で作り直す**——引き直しはその面の上流にしか
    /// 聞かないので、両面ぶんを見る `report` は通れない。
    /// **答えは同じ**（どちらも `measured::row_for` で判定し `measured::tally` で数える。
    /// 見張り番は `measured` の `judging_one_face_alone_agrees_with_judging_both`）。
    ///
    /// **⚠ ここで `report` を呼び直さないこと。** 呼ぶと**5分ごとに警告が鳴る**
    /// ——`llama-server` を上流にすると 9 行が全部 `Absent` になる形（作業単位 G-5）を、
    /// そのまま定期的に再生することになる。
    #[must_use]
    pub(crate) fn refreshed(&self, face: ModelFace, probe: &UpstreamProbe) -> Self {
        Self {
            endpoint: self.endpoint.clone(),
            model: self.model.clone(),
            // **引き直しのたびに動く。** 上流が別のモデルを載せ直したら、
            // ここが変わることだけがその手掛かりになる（ADR-0020 の引き直し）。
            answered_name: probe.answered_name.clone(),
            queried_context_tokens: probe.context_tokens,
            answered_by: probe.answered_by,
            measured: judge_face(face, &self.model, probe.catalogue.as_deref()),
            // **引き直しのたびに動く**（鍵を直して上流が受け付けたら、次の照会で消える）。
            unauthorized: probe.unauthorized,
            // **引き直しのたびに動く**（解決先が変われば、次の照会の名前解決で変わる）。
            key_over_plain_http: probe.key_over_plain_http,
            // **引き直しのたびに動く**（作業単位 B-66）。照会した回は必ず `Some`。
            check: probe.check,
        }
    }

    /// 設定された宛先の綴り。**未設定でも空にしない**（`AppConfig::sub_llm_endpoint`）。
    ///
    /// **⚠ 実際に叩く URL と1バイト一致するとは限らない。**
    /// **認証情報（userinfo）は `***@` に伏せ、クエリとフラグメントは落としてある**
    /// （[`endpoint_for_view`] / このモジュールの doc）。**接続に使う綴りは `llm::LmStudioConfig::endpoint`。**
    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// personacore が依頼するモデルの綴り。**上流が名乗る綴りではない。**
    ///
    /// **上流が名乗ったほうは [`Self::answered_name`]**（作業単位 B-45）。
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// **上流が名乗った名前**（作業単位 B-45 / [`name_that_answered`]）。
    ///
    /// **`None` は「名乗っていない／名指しできない」。**
    /// **ディレクトリは含まない**——`llama-server` の `id` は gguf の絶対パスなので、
    /// **継ぎ目でファイル名だけにしてある**（このモジュールの doc）。
    ///
    /// **⚠ [`Self::model`] と食い違うのは異常ではない。**
    /// **`llama serve` は `model` の欄を無視する**ので、
    /// **食い違いこそが「別物が答えている」の唯一の見え方である**（`C-18`）。
    #[must_use]
    pub fn answered_name(&self) -> Option<&str> {
        self.answered_name.as_deref()
    }

    /// **上流が申告したコンテキスト長。** `None` は「読めなかった」。
    ///
    /// **採用した値ではない**（このモジュールの doc）。
    #[must_use]
    pub const fn queried_context_tokens(&self) -> Option<u32> {
        self.queried_context_tokens
    }

    /// **どの読み手が答えたか。** `None` は「答えなし」。
    #[must_use]
    pub const fn answered_by(&self) -> Option<AnsweredBy> {
        self.answered_by
    }

    /// 実測由来の定数との突き合わせの内訳。
    #[must_use]
    pub const fn measured(&self) -> FaceTally {
        self.measured
    }

    /// **最後の照会が鍵を断られたか**（401 / 403 / ADR-0056 §決定4）。
    #[must_use]
    pub const fn unauthorized(&self) -> bool {
        self.unauthorized
    }

    /// **鍵を平文の `http://` で別の機へ送る扱い**（ADR-0056 §決定5 / 作業単位 A-13b-2）。`None` は警告なし。
    #[must_use]
    pub const fn key_over_plain_http(&self) -> Option<KeyOverPlainHttp> {
        self.key_over_plain_http
    }

    /// **最後にいつ・どう確かめたか**（作業単位 B-66）。**`None` は「照会していない」。**
    ///
    /// **[`Self::answered_by`] の `None` は「照会したが答えない」と「照会していない」の両方で立つ**
    /// （Issue origin#94）。**2つを割るのはこの欄。**
    #[must_use]
    pub const fn check(&self) -> Option<UpstreamCheck> {
        self.check
    }
}

/// **両面ぶんの観測。** `AppState` が持ち、引き直しが面ごとに書き替える。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamFacts {
    main: UpstreamObservation,
    sub: UpstreamObservation,
}

impl UpstreamFacts {
    /// 起動時の2面から作る。
    #[must_use]
    pub(crate) const fn new(main: UpstreamObservation, sub: UpstreamObservation) -> Self {
        Self { main, sub }
    }

    /// 本文生成の上流。
    #[must_use]
    pub const fn main(&self) -> &UpstreamObservation {
        &self.main
    }

    /// 裏方（抽出・変容）の上流。
    #[must_use]
    pub const fn sub(&self) -> &UpstreamObservation {
        &self.sub
    }

    /// 面で引く。**帰属を1箇所で決める**（呼び出し側に `if` を書かせない）。
    #[must_use]
    pub const fn face(&self, face: ModelFace) -> &UpstreamObservation {
        match face {
            ModelFace::Main => &self.main,
            ModelFace::Sub => &self.sub,
        }
    }

    /// **その面だけを引き直した結果で置き換える。**
    ///
    /// 反対の面は1バイトも触らない——引き直しは**面ごとに別の往復**なので、
    /// 片方の失敗でもう片方の観測を潰してはならない。
    pub(in crate::server) fn refresh(&mut self, face: ModelFace, probe: &UpstreamProbe) {
        match face {
            ModelFace::Main => self.main = self.main.refreshed(face, probe),
            ModelFace::Sub => self.sub = self.sub.refreshed(face, probe),
        }
    }

    /// **その面の観測を丸ごと置き換える**（走ったままの差し替え / 作業単位 F-4b-2）。
    ///
    /// **[`Self::refresh`] と違い、宛先と依頼する綴りも入れ替わる**
    /// ——差し替えた面の古い事実を1つも残さない（ADR-0042 §決定4）。
    /// **反対の面は1バイトも触らない。**
    pub(in crate::server) fn replace(&mut self, face: ModelFace, observation: UpstreamObservation) {
        match face {
            ModelFace::Main => self.main = observation,
            ModelFace::Sub => self.sub = observation,
        }
    }
}

// ------------------------------------------------------------------
// `GET /admin/upstream`（作業単位 G-10 / ADR-0039 §決定4）
// ------------------------------------------------------------------

/// **両面ぶんの応答**（`GET /admin/upstream`）。
///
/// # 読むだけ
///
/// **`POST` / `PUT` / `PATCH` は生やさない。** 2026-09-07 に利用者が決めている
/// ——「モデルは見えるだけでよい。personacore はプロセスを起こさない。選ばせもしない」
/// （[ADR-0039](../docs/adr/0039-two-upstreams-for-two-jobs.md) §却下した選択肢）。
/// **1つの面を指すパス（`/admin/upstream/main`）も生やさない**
/// ——生やせば `PUT` が自然な形になり、いま塞いだものがそのまま開く。
/// 見張り番は `tests/server_admin_upstream.rs::only_get_is_exposed_for_the_upstream_path`。
///
/// # ⚠ `/v1/models` へ混ぜない
///
/// **あちらはキャラクターの一覧である**（ADR-0004）。上流のモデルを並べた瞬間に、
/// **`model` に書ける綴りとそうでない綴りが同じ一覧に混ざる。**
/// 見張り番は `tests/server_admin_upstream.rs::
/// the_upstream_models_never_appear_in_the_openai_model_list`
/// （**上流の綴りがこちらの入口には出ていること**まで見るので、空振りしない）。
///
/// # ⚠ 応答本文への走査は、`Debug` の見張り番の代わりにならない
///
/// **`Debug` に出ないことは、応答本文に出ないことを1ミリも意味しない**
/// （このモジュールの doc の申し送り）。**逆も真である。**
/// **本文を見る見張り番は `tests/server_admin_upstream.rs` の
/// `the_response_never_carries_the_upstreams_own_model_id` と
/// `the_response_never_shows_the_endpoints_credentials` の2本。**
///
/// # 見張り番と、実際に当てた変異（**作業単位 G-10 / 2026-09-08**）
///
/// **着手前は 2005 全緑・exit 0**（複製ツリー）。**下は全部 exit 101。**
///
/// | 変異 | 赤 | その見張り番を外すと |
/// |---|---|---|
/// | 応答の面を入れ替える（[`UpstreamReport::of`]） | 5 | **exit 0** |
/// | `AppState::upstream()` が面を入れ替える | 3 | **exit 0** |
/// | 上流の綴りを保持 ＋ 手書き `Debug` で隠す | **1** | **exit 0** |
/// | `redact_userinfo` を外す | 3 | **exit 101 / 2 赤**（唯一の権威ではない） |
/// | ハンドラが起動時の写しを読む | **1** | **exit 0** |
/// | 内訳の `matched` ↔ `unanswered` | 3 | —— |
/// | [`UpstreamView::face`] を `Main` に決め打つ | 2 | —— |
/// | 内訳の `matched` ↔ `drifted_absent` | **1** | **exit 0** |
///
/// **測ったもの一式は `<measurements>\g10-2026-09-08\`**
/// （**適用したコマンドそのもの**＝`mut.py` を、ログの隣に置いてある）。
#[derive(Debug, Serialize, ToSchema)]
pub(super) struct UpstreamReport {
    /// 本文生成の上流。
    main: UpstreamView,
    /// 裏方（抽出・変容）の上流。
    sub: UpstreamView,
    /// **この起動が上流プロファイルの見本を書いた先**（作業単位 F-4c / Issue origin#23）。
    ///
    /// **`null` は「この起動は見本を書いていない」**（既に在った／
    /// `PERSONACORE_UPSTREAMS` が別の置き場所を指した／書けなかった）。
    /// **書いていない起動で「書いた」と言わない。**
    ///
    /// 画面（`UpstreamPanel.svelte`）がこれを読み、
    /// 「見本を置いた。gguf のパスを書き換えて選び直す」と言う。
    ///
    /// # ⚠ パスを載せる（**絶対制約の例外。司令塔の決定 / 2026-09-23**）
    ///
    /// CLAUDE.md §1 は「**エラーレスポンス**に内部パスを含めない」。
    /// **これはエラーレスポンスではなく、`127.0.0.1` に居る利用者本人への状態表示である**
    /// ——**利用者が次に開くべきファイルの場所**で、言わなければこの欄の意味が無い。
    /// **エラー型の `Display` やエラーレスポンスには、今までどおりパスを載せない**
    /// （[`crate::server::SampleWriteError`] の `Display` は定数文字列）。
    ///
    /// **⚠ ここに載るのは「この実行ファイルが自分で書いたファイルの場所」だけ。**
    /// 上流が名乗った綴り（gguf の絶対パス）を載せない決定（このモジュールの doc）は変えていない。
    #[schema(required = true)]
    profile_sample: Option<String>,
    /// **いま選ばれている上流プロファイルの名前**（作業単位 F-10b / ADR-0055 §決定3）。
    ///
    /// **起動時に選んだもの**（`--upstream <名前>` か覚えた名前）か、**最後に受け付けた
    /// `POST /admin/upstream/profile` の名前**。断った切り替え（404 / 409 / 422）では変わらない。
    /// **`null` は「プロファイルを選んでいない」**（環境変数の既定で起きて、まだ切り替えていない）。
    ///
    /// **画面の「起こす」はこの名前を `POST /admin/upstream/profile` へ送る**
    /// ——面が `not_started` のとき、同じ名前の依頼がその面を起こす（ADR-0055 §決定3）。
    /// **画面はこれ以外の手段で名前を知らない**（`GET /admin/upstream/profiles` は一覧だけで、
    /// どれが選ばれているかを言わない）。
    ///
    /// **利用者が `upstreams.json` に書いた綴りそのまま**（127.0.0.1 の本人への状態表示）。
    #[schema(required = true)]
    selected_profile: Option<String>,
}

impl UpstreamReport {
    /// **保持している観測から組む。写しを作らない。**
    ///
    /// `profile_sample` は**この起動が見本を書いた先**（`AppState::profile_sample`）、
    /// `selected_profile` は**いま選ばれているプロファイルの名前**（`switch::Switcher::selected_profile`）。
    ///
    /// **差し替え（`POST /admin/upstream/profile` / `super::switch`）も同じ形で返す**
    /// ——差し替えた直後の状態を、読み直さずに見せる。
    pub(super) fn of(
        facts: &UpstreamFacts,
        states: &LaunchStates,
        profile_sample: Option<&std::path::Path>,
        selected_profile: Option<&str>,
    ) -> Self {
        Self {
            main: UpstreamView::of(facts, states, ModelFace::Main),
            sub: UpstreamView::of(facts, states, ModelFace::Sub),
            profile_sample: profile_sample.map(|path| path.display().to_string()),
            selected_profile: selected_profile.map(str::to_owned),
        }
    }
}

/// **上流1面ぶんの姿。**
///
/// # 面を必ず名乗る（ADR-0039 §決定4）
///
/// **鍵（`main` / `sub`）だけでは、1件を取り出して眺めたときに帰属が消える。**
/// [`Self::face`] は**観測を引くのに使った [`ModelFace`] そのもの**から作るので、
/// **鍵と中身が食い違えば、そのまま応答に現れる。**
#[derive(Debug, Serialize, ToSchema)]
struct UpstreamView {
    /// どちらの面か（`main` / `sub`）。**綴りの正は [`ModelFace::label`]。**
    #[schema(schema_with = face_schema)]
    face: &'static str,
    /// 設定された宛先。**認証情報（userinfo）は `***@` に伏せ、クエリとフラグメントは落としてある**
    /// （[`endpoint_for_view`] / このモジュールの doc）。
    endpoint: String,
    /// **頼んだ名前。** personacore が依頼するモデルの綴り。
    ///
    /// **上流が名乗った名前は隣の [`Self::answered_name`]**（作業単位 B-45）。
    /// **この欄の意味は1バイトも変えていない。**
    model: String,
    /// **答えた名前。** 上流が名乗った id の**ファイル名だけ**。
    ///
    /// **`null` は「上流は名乗っていない／名指しできない」。**
    ///
    /// **⚠ [`Self::model`] と食い違うのは異常ではない** ——
    /// **`llama serve` は `model` の欄を無視して、載っているモデルで答える**
    /// （`C-18` が実測 / 2026-09-17）。**食い違いこそが読みたいもの。**
    ///
    /// **ディレクトリは載らない**（継ぎ目で落ちている / このモジュールの doc）。
    #[schema(required = true)]
    answered_name: Option<String>,
    /// **上流が申告したコンテキスト長。** `null` は「読めなかった」。
    ///
    /// **いま使っている値ではない**（このモジュールの doc）。
    /// 本文が使っている値は `AppState::budget`、抽出のそれは `MemoryExtractor` が持つ
    /// ——**写しを作らないので、ここには載せない。**
    /// **2つが食い違う状態は正常にありうる**（引き直しが失敗した回）。
    #[schema(required = true)]
    queried_context_tokens: Option<u32>,
    /// **どの読み手が答えたか。** `null` は「答えなし」＝**上流が黙っている。**
    ///
    /// 上流が停止しても**予算は現在値を保つ**ので、
    /// **ここが `null` になることだけが、外から見える手掛かりになる。**
    #[schema(schema_with = answered_by_schema, required = true)]
    answered_by: Option<&'static str>,
    /// **最後のモデルの照会が、鍵を断られた（401 / 403）か**（ADR-0056 §決定4 / 作業単位 A-13b-1）。
    ///
    /// **`answered_by: null` と組で読む** —— `null` ＋ `false` は「届かない・答えない」、
    /// `null` ＋ `true` は「届いたが、鍵を受け付けなかった」。
    ///
    /// > **作業単位 B-66 まで、`context_tokens` を書いた面には照会しなかったので、ここは `false` のままだった**
    /// > （本文の依頼の `upstream_unauthorized` が初めての知らせになっていた）。**いまは書いた面も
    /// > 起動の直後・5分ごと・`POST /admin/upstream/check` で照会する**ので、最初の照会の後に立つ。
    /// > **照会したかどうかは [`Self::check_result`] が言う。**
    unauthorized: bool,
    /// **最後に照会した時刻**（RFC 3339 / 作業単位 B-66）。**`null` は「まだ照会していない」。**
    ///
    /// **経過は載せない**（`LaunchView::since` と同じ —— 読み手が応答の時刻との差を取る）。
    #[schema(required = true)]
    checked_at: Option<String>,
    /// **最後の照会の結果**（作業単位 B-66 / Issue origin#94）。
    ///
    /// | 値 | 意味 |
    /// |---|---|
    /// | `not_checked` | まだ照会していない（`checked_at` も `null`） |
    /// | `answered` | 上流が答えた |
    /// | `no_answer` | 繋がらない・黙っている（応答が1つも来なかった） |
    /// | `no_model_list` | 届いたが、モデル一覧が読めない（一覧の口が無い・一覧だけ失敗する など。レビュー是正で割った） |
    /// | `refused` | 鍵を断られた（`unauthorized` も `true`） |
    /// | `key_not_sent` | 鍵を平文で外へ送る承認が無いので、送らなかった |
    ///
    /// **`answered_by: null` だけでは「照会していない」と「照会したが答えない」が区別できなかった**
    /// （Issue origin#94）。**画面はこの欄で言い分ける。**
    #[schema(schema_with = check_result_schema)]
    check_result: &'static str,
    /// **鍵を平文の `http://` で別の機へ送る扱い**（ADR-0056 §決定5 / 作業単位 A-13b-2）。
    /// **警告が無い面では欄ごと省く**（`null` にしない）。
    ///
    /// | 値 | 意味 |
    /// |---|---|
    /// | `local_network` | 宛先が全部ループバックかローカルネットワーク。**鍵を暗号化せずに送っている** |
    /// | `approved` | 宛先にローカルネットワークの外が混ざるが、面に承認がある。**鍵を暗号化せずに外へ送っている** |
    /// | `not_approved` | 宛先にローカルネットワークの外が混ざり、承認が無い。**送っていない**（本文の依頼は `upstream_key_not_approved`） |
    /// | （欄が無い） | 鍵の無い面・`https://`・ループバック・全部ループバックに解決した、または宛先のホスト名をまだ解決していない |
    ///
    /// **値は最後のモデルの照会の後に読んだもの**（`unauthorized` と同じく、照会しない面ではホスト名の判定が載らない。
    /// IP アドレスの宛先は組んだときに決まるので載る）。
    ///
    /// # 欄ごと省く理由
    ///
    /// **画面の写し（`web/src/lib/api.ts`）と、その型を使う画面のテストの全部に、新しい必須の欄を足さずに済む**
    /// （画面の文言は作業単位 A-13b-3）。`null` と「欄が無い」を同じ意味で読む使い手には差が出ない。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(schema_with = key_over_plain_http_schema)]
    key_over_plain_http: Option<&'static str>,
    /// 実測由来の定数との突き合わせの内訳（ADR-0033）。
    measured: MeasuredView,
    /// **この実行ファイルが、この面の上流をどう扱っているか**
    /// （ADR-0042 §決定3 / 作業単位 F-4b-1）。
    ///
    /// # ⚠ [`Self::answered_by`] と混同しないこと
    ///
    /// **あちらは「上流が何と答えたか」、こちらは「この実行ファイルが何をしたか」。**
    /// **どちらも `null` / `external` になりうるが、意味が違う** ——
    /// **`loading` ＋ `answered_by: null` は「いま載せている最中」**で、
    /// **`external` ＋ `answered_by: null` は「外の上流が黙っている」。**
    /// **畳むと、その違いを画面から復元できない。**
    launch: LaunchView,
}

// 仕様書の閉じた綴り（作業単位 P-2a）。**値は応答を組むのと同じ定義から作る**（綴りを写さない）。
fn face_schema() -> Object {
    openapi::closed_string(ModelFace::ALL.map(ModelFace::label))
}

fn answered_by_schema() -> Object {
    openapi::nullable_closed_string(AnsweredBy::ALL.map(AnsweredBy::label))
}

/// **`check_result` の閉じた綴り**（作業単位 B-66）。**「照会していない」を先頭に、結果の5つ（[`CheckOutcome::ALL`]）を続ける。**
fn check_result_schema() -> Object {
    openapi::closed_string(
        std::iter::once(NOT_CHECKED).chain(CheckOutcome::ALL.map(CheckOutcome::label)),
    )
}

fn launch_state_schema() -> Object {
    openapi::closed_string(FaceState::all_labels())
}

/// **省かれうるが `null` にはならない**（`skip_serializing_if` / `openapi` の
/// `a_response_never_omits_a_field_unless_it_is_named_here`）。
fn key_over_plain_http_schema() -> Object {
    openapi::closed_string(KeyOverPlainHttp::ALL.map(KeyOverPlainHttp::label))
}

impl UpstreamView {
    /// **面で引く。帰属を決めるのはこの1箇所**（[`UpstreamFacts::face`] に預ける）。
    fn of(facts: &UpstreamFacts, states: &LaunchStates, face: ModelFace) -> Self {
        let observation = facts.face(face);
        Self {
            face: face.label(),
            endpoint: observation.endpoint().to_owned(),
            model: observation.model().to_owned(),
            answered_name: observation.answered_name().map(str::to_owned),
            queried_context_tokens: observation.queried_context_tokens(),
            answered_by: observation.answered_by().map(AnsweredBy::label),
            unauthorized: observation.unauthorized(),
            checked_at: observation.check().map(|check| check.at().to_rfc3339()),
            check_result: observation
                .check()
                .map_or(NOT_CHECKED, |check| check.outcome().label()),
            key_over_plain_http: observation
                .key_over_plain_http()
                .map(KeyOverPlainHttp::label),
            measured: MeasuredView::of(observation.measured()),
            // **状態も同じ [`ModelFace`] から引く。** 観測と別の面を引くと、
            // **「本文は載った」と言いながら抽出の観測を並べる**形になる。
            launch: LaunchView::of(states.face(face)),
        }
    }
}

/// **この実行ファイルが、その面の上流をどう扱っているか**
/// （ADR-0042 §決定3 / 作業単位 F-4b-1 / [`crate::server::FaceState`]）。
///
/// # ⚠ 置き場所は載せない（**絶対制約**）
///
/// **[`Self::log`] は `upstream-logs/` の下のファイル名だけ。**
/// **絶対パスを応答へ載せない**（`endpoint_for_view` と同じ判断 / このモジュールの doc）。
/// **ディレクトリまで要るときは、今までどおりログを読む。**
#[derive(Debug, Serialize, ToSchema)]
struct LaunchView {
    /// `external` / `not_started` / `loading` / `ready` / `failed` / `stopped`。
    ///
    /// **`not_started` は「gguf を名指ししているが、起こすのを待っている」**（ADR-0055 / 作業単位 F-10a）
    /// ——覚えた名前で起きた起動の面。同じ名前の `POST /admin/upstream/profile` で起こす。
    ///
    /// **綴りの正は [`crate::server::FaceState::label`]。**
    /// **閉じた型にしていない**（`face` と同じ立場 —— 次の状態が増えた日に、
    /// 画面からこの欄が消えるのがいちばん質の悪い壊れ方）。
    #[schema(schema_with = launch_state_schema)]
    state: &'static str,
    /// **載せ始めた時刻**（`loading` のときだけ）。`null` はそれ以外の状態。
    ///
    /// **経過そのものは載せない。** 読み手が見るのは応答の時刻との差で、
    /// **秒数を載せると、その回の応答の中でしか意味を持たない数が1つ増える。**
    #[schema(required = true)]
    since: Option<String>,
    /// **上流が名乗った名前**（`ready` のときだけ）。**ファイル名だけ。**
    ///
    /// **[`UpstreamView::answered_name`] の写しではない** ——
    /// **あちらは引き直しのたびに動き、こちらは「載ったと認めた時点の名前」で固定される。**
    /// **2つが食い違う状態は正常にありうる**（載ったあとに上流が載せ替えられた回）。
    #[schema(required = true)]
    answered_name: Option<String>,
    /// **失敗の理由**（`failed` のときだけ）。
    #[schema(required = true)]
    reason: Option<String>,
    /// **出力の落とし先のファイル名**（`failed` のときだけ）。**置き場所は含まない。**
    #[schema(required = true)]
    log: Option<String>,
}

impl LaunchView {
    /// **状態から組む。** 欄は状態ごとに `null` になる（畳まない）。
    fn of(state: &FaceState) -> Self {
        let mut view = Self {
            state: state.label(),
            since: None,
            answered_name: None,
            reason: None,
            log: None,
        };
        match state {
            FaceState::External | FaceState::NotStarted | FaceState::Stopped => {}
            FaceState::Loading { since } => view.since = Some(since.to_rfc3339()),
            FaceState::Ready { answered_name } => {
                view.answered_name = Some(answered_name.clone());
            }
            FaceState::Failed { reason, log } => {
                view.reason = Some(reason.clone());
                view.log = log.clone();
            }
        }
        view
    }
}

/// **突き合わせの内訳**（[`FaceTally`]）。
///
/// # ⚠ ずれの2種類を1つに潰さない
///
/// `drifted_other` は「測り直す先が決まっている」、
/// `drifted_absent` は「誰が答えるかまだ分からない」。**調査先がまるで違う**
/// （[`crate::measured::report`] の doc）。**潰すと、その違いを画面から復元できない。**
#[derive(Debug, Serialize, ToSchema)]
struct MeasuredView {
    /// 見た行の総数。**内訳の和**（`checked` だけを見て「全部一致」と読まないこと）。
    checked: usize,
    /// 測ったときの相手と、いま応答する相手が同じ。
    matched: usize,
    /// 一覧には在るが、測ったときの相手ではない。
    drifted_other: usize,
    /// 依頼する綴りが一覧に無い。
    drifted_absent: usize,
    /// 聞けなかった（上流の停止・別実装・明示設定）。
    unanswered: usize,
}

impl MeasuredView {
    const fn of(tally: FaceTally) -> Self {
        Self {
            checked: tally.checked(),
            matched: tally.matched,
            drifted_other: tally.drifted_other,
            drifted_absent: tally.drifted_absent,
            unanswered: tally.unanswered,
        }
    }
}

/// `GET /admin/upstream` — **上流について観測できたこと**（作業単位 G-10）。
///
/// **失敗しない。** DB も上流も触らず、`AppState` が持っている観測を写すだけである
/// ——**上流が停止していても 200 を返す**（そのとき `answered_by` が `null` になる）。
/// **上流の停止を 5xx にしないこと**——この入口は「上流が生きているか」を
/// **読むため**に在るので、上流の状態で自分が落ちたら読む手段そのものが消える。
///
/// **毎回 `AppState::upstream` を読む**（起動時の写しを持たない）。
/// 引き直しが 5 分ごとに書き替えるので、写し取ると
/// **止まった上流を永久に「生きている」と言う。**
/// 見張り番は `tests/server_admin_upstream.rs::
/// the_response_follows_the_refreshed_observation`。
#[utoipa::path(
    get,
    path = "/admin/upstream",
    summary = "Report what is known about the upstream inference servers",
    tag = "upstream",
    responses(
        (status = 200, body = UpstreamReport),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn get_upstream(State(state): State<AppState>) -> Json<UpstreamReport> {
    Json(report_now(&state))
}

/// **いまの観測と状態を、同じ1回の読みで揃えて組む**（`GET` と `POST .../check` が共有する）。
fn report_now(state: &AppState) -> UpstreamReport {
    // **観測と状態を、同じ1回の読みで揃える**（ADR-0042 §決定3）。
    // **どちらも毎回読む**——写し取ると、止まった上流を永久に「生きている」と言う。
    UpstreamReport::of(
        &state.upstream(),
        &state.launch_states(),
        state.profile_sample(),
        state.switcher().selected_profile().as_deref(),
    )
}

/// `POST /admin/upstream/check` — **両方の面の上流に、いま照会する**（作業単位 B-66 / Issue origin#94）。
///
/// **返るのは `GET /admin/upstream` と同じ形**（照会した後の観測）。
///
/// # 照会の中身は5分ごとと同じ
///
/// **面ごとに `context::refresh_once` / `refresh_sub_once`**（`context::UpstreamChecker::check_now`）——
/// コンテキスト長を書いていない面は予算の引き直しまで、書いた面は観測だけ（予算も「変わった」の warn も動かない）。
///
/// # 走っている照会があれば、2本目を並べずに待つ（409 にしない）
///
/// 理由は `context::CheckGate` の doc。**5分ごとの照会と重なっても、押した人は待つだけで結果を得る。**
///
/// **失敗しない**（`GET` と同じ）——上流が黙っていても 200 で、観測が「答えない」になる。
/// **上流の状態で自分が落ちたら、読む手段そのものが消える**（[`get_upstream`] の doc）。
///
/// **`Host` / `Origin` の検査はこの口にも効く**（`host_guard` が全部の経路の手前にある）。見張り番は
/// `tests/server_upstream_check.rs::a_foreign_host_cannot_make_the_server_check`。
#[utoipa::path(
    post,
    path = "/admin/upstream/check",
    summary = "Ask both upstream inference servers now whether they answer",
    tag = "upstream",
    responses(
        (status = 200, body = UpstreamReport),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn check_upstream(State(state): State<AppState>) -> Json<UpstreamReport> {
    state.checker().check_now().await;
    Json(report_now(&state))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本文の宛先。**抽出用とも既定とも別。**
    const MAIN_ENDPOINT: &str = "http://127.0.0.1:19001";
    /// 抽出の宛先。**本文用とも既定とも別。**
    const SUB_ENDPOINT: &str = "http://127.0.0.1:19002";
    /// 本文のモデル。**抽出用とも別。**
    const MAIN_MODEL: &str = "main-model-for-tests";
    /// 抽出のモデル。**本文用とも別。**
    const SUB_MODEL: &str = "sub-model-for-tests";

    /// **上流が名乗った、パスに見えない `vendor/model` 形の綴り。**
    ///
    /// **`/` は入っているが、ディレクトリではない。**
    /// 切り落とすと `devstral-small-2-2512` になり、
    /// **画面が「頼んだ名前と違うものが答えています」と嘘をつく。**
    const VENDOR_STYLE_ID: &str = "mistralai/devstral-small-2-2512";

    /// **本文の面に載った gguf の名前。** 抽出の面のどの綴りとも別。
    const MAIN_ANSWERED_GGUF: &str = "main-loaded-model.gguf";

    /// **抽出の面の失敗の理由。** 本文の面のどの綴りとも別。
    const SUB_FAILURE_REASON: &str = "sub-face-failed-for-tests";

    /// **抽出の面のログの名前。** `upstream-logs/` の下のファイル名だけ（置き場所を含まない）。
    const SUB_FAILURE_LOG: &str = "sub-20260918-101112.err.log";

    /// **認証情報・クエリ・フラグメントつきの宛先。** `normalize_base` は弾かないので**現に設定できる。**
    ///
    /// **3つに別々の秘密を入れる**（作業単位 B-71）——1つを落とし忘れた実装を、残りが隠さない。
    const ENDPOINT_WITH_CREDENTIALS: &str =
        "http://alice:s3cr3t@127.0.0.1:19003/v1/chat/completions?token=q-s3cr3t#f-s3cr3t";

    /// 上の宛先に入れた秘密（利用者名・合言葉・クエリ・フラグメント）。
    const ENDPOINT_SECRETS: [&str; 4] = ["alice", "s3cr3t", "q-s3cr3t", "f-s3cr3t"];

    /// 上から認証情報を `***` に伏せ、クエリとフラグメントを落とした形。**手で書く**——
    /// [`endpoint_for_view`] から作ると、緩めたときに期待値も一緒に動く（CLAUDE.md）。
    const ENDPOINT_REDACTED: &str = "http://***@127.0.0.1:19003/v1/chat/completions";

    /// フィクスチャの4つが本当に違うこと。**同値なら以下は何も守っていない。**
    #[test]
    fn the_fixture_never_makes_two_faces_equal() {
        assert_ne!(MAIN_ENDPOINT, SUB_ENDPOINT);
        assert_ne!(MAIN_MODEL, SUB_MODEL);
        // **依頼する綴りと、上流が名乗る綴りも別**
        // ——同値なら、完全一致の枝に落ちて「切らない」側を一度も通らない。
        assert_ne!(MAIN_MODEL, VENDOR_STYLE_ID);
        assert_ne!(SUB_MODEL, VENDOR_STYLE_ID);
    }

    /// **利用者名も合言葉も、観測には入らない**（絶対制約 / このモジュールの doc）。
    ///
    /// # 空振りではないこと
    ///
    /// **ホストと経路が残ることまで見る。** 丸ごと落とす実装は
    /// 「認証情報を消した」ではなく「**宛先を消した**」であり、
    /// **G-10 の応答から上流の同定そのものが消える。**
    ///
    /// # ⚠ これが本番の入口を通る腕ではない
    ///
    /// **設定から `App::bind` を通る腕は
    /// `tests/server_upstream_facts.rs::the_held_facts_never_show_the_endpoints_credentials`。**
    /// ここが見ているのは**器の入口**（`UpstreamObservation::new`）である。
    #[test]
    fn credentials_are_not_kept_in_the_endpoint() {
        let observation =
            UpstreamObservation::unqueried(ModelFace::Main, ENDPOINT_WITH_CREDENTIALS, MAIN_MODEL);

        assert_eq!(
            observation.endpoint(),
            ENDPOINT_REDACTED,
            "認証情報・クエリ・フラグメントだけを落とすこと"
        );
        let rendered = format!("{observation:?}");
        for secret in ENDPOINT_SECRETS {
            assert!(
                !rendered.contains(secret),
                "設定の認証情報を保持しないこと（{secret}）: {rendered}"
            );
        }
        assert!(
            rendered.contains("127.0.0.1:19003"),
            "宛先の同定は残ること（丸ごと落とさない）: {rendered}"
        );
    }

    /// **伏せるのは権限部の userinfo（`***@` を残す）、落とすのはクエリとフラグメント**（純粋関数の表 /
    /// 作業単位 B-71 でクエリとフラグメントの腕を足した）。
    ///
    /// **⚠ 上の見張り番の代わりにはならない。** ここが見ているのは
    /// [`endpoint_for_view`] の振る舞いであって、**器がそれを通しているか**ではない
    /// （CLAUDE.md / `E-3`——道具に腕が n 本あるなら、対照も n 本要る）。
    #[test]
    fn only_the_userinfo_query_and_fragment_are_redacted() {
        for (input, expected, what) in [
            (
                "http://127.0.0.1:1234/v1/chat/completions?token=q-s3cr3t",
                "http://127.0.0.1:1234/v1/chat/completions",
                "クエリを落とす（ユーザー情報が無くても）",
            ),
            (
                "http://127.0.0.1:1234/v1/chat/completions#f-s3cr3t",
                "http://127.0.0.1:1234/v1/chat/completions",
                "フラグメントを落とす",
            ),
            (
                "http://alice:s3cr3t@127.0.0.1:1234/v1?token=q-s3cr3t/v1/chat/completions",
                "http://***@127.0.0.1:1234/v1",
                "宛先に書いたクエリは、足した経路ごと落ちる（送信が叩く経路は /v1）",
            ),
            (
                "http://127.0.0.1:1234/v1/chat/completions",
                "http://127.0.0.1:1234/v1/chat/completions",
                "認証情報が無ければ1バイトも触らない",
            ),
            (
                "http://alice@127.0.0.1:1234/v1",
                "http://***@127.0.0.1:1234/v1",
                "利用者名だけでも落とす",
            ),
            (
                "https://alice:s3cr3t@example.test:8443/v1",
                "https://***@example.test:8443/v1",
                "https でも同じ",
            ),
            (
                "http://127.0.0.1:1234/mail@host",
                "http://127.0.0.1:1234/mail@host",
                "経路の中の @ は触らない",
            ),
            (
                "http://[::1]:1234/v1",
                "http://[::1]:1234/v1",
                "IPv6 の綴りを壊さない",
            ),
            (
                "http://alice:s3cr3t@[::1]:1234/v1",
                "http://***@[::1]:1234/v1",
                "IPv6 でも落とす",
            ),
            (
                "127.0.0.1:1234",
                "<読めない宛先>",
                "URL として読めない綴りは出さない（伏せ損ねた綴りをそのまま出さない）",
            ),
        ] {
            assert_eq!(endpoint_for_view(input), expected, "{what}");
        }
    }

    /// **照会が通った回の観測。**
    ///
    /// **`answered_name` は [`name_that_answered`] に作らせる**
    /// ——本番（`server::context::probe_upstream`）と同じ形にしないと、
    /// **この器が何を受け取るのかが、ここだけ本番と違うことになる。**
    fn probe(
        model: &str,
        tokens: u32,
        answered_by: AnsweredBy,
        catalogue: &[&str],
    ) -> UpstreamProbe {
        let catalogue: Vec<String> = catalogue.iter().map(|id| (*id).to_owned()).collect();
        UpstreamProbe {
            answered_name: name_that_answered(model, &catalogue),
            catalogue: Some(catalogue),
            context_tokens: Some(tokens),
            answered_by: Some(answered_by),
            unauthorized: false,
            key_over_plain_http: None,
            check: Some(UpstreamCheck::now(CheckOutcome::Answered)),
        }
    }

    fn facts() -> UpstreamFacts {
        UpstreamFacts::new(
            UpstreamObservation::unqueried(ModelFace::Main, MAIN_ENDPOINT, MAIN_MODEL),
            UpstreamObservation::unqueried(ModelFace::Sub, SUB_ENDPOINT, SUB_MODEL),
        )
    }

    /// **どちらの面も起こしていない**（`--upstream` を渡していない起動）。
    fn no_launch() -> LaunchStates {
        LaunchStates::of_for_tests(FaceState::External, FaceState::External)
    }

    /// **面ごとに別の状態**（帰属を見る腕。同値では取り違えが観測できない）。
    fn split_launch() -> LaunchStates {
        LaunchStates::of_for_tests(
            FaceState::Ready {
                answered_name: MAIN_ANSWERED_GGUF.to_owned(),
            },
            FaceState::Failed {
                reason: SUB_FAILURE_REASON.to_owned(),
                log: Some(SUB_FAILURE_LOG.to_owned()),
            },
        )
    }

    /// **面で引いた観測が、その面のもの**（帰属）。
    #[test]
    fn each_face_returns_its_own_observation() {
        let facts = facts();
        assert_eq!(facts.face(ModelFace::Main), facts.main());
        assert_eq!(facts.face(ModelFace::Sub), facts.sub());
        assert_eq!(facts.main().endpoint(), MAIN_ENDPOINT);
        assert_eq!(facts.sub().endpoint(), SUB_ENDPOINT);
        assert_eq!(facts.main().model(), MAIN_MODEL);
        assert_eq!(facts.sub().model(), SUB_MODEL);
        assert_ne!(facts.main(), facts.sub());
    }

    /// **片方を引き直しても、もう片方は1バイトも動かない。**
    #[test]
    fn refreshing_one_face_leaves_the_other_alone() {
        let mut facts = facts();
        let before_sub = facts.sub().clone();

        facts.refresh(
            ModelFace::Main,
            &probe(MAIN_MODEL, 4_096, AnsweredBy::NativeApi, &[MAIN_MODEL]),
        );

        assert_eq!(facts.main().queried_context_tokens(), Some(4_096));
        assert_eq!(facts.main().answered_by(), Some(AnsweredBy::NativeApi));
        assert_eq!(facts.sub(), &before_sub, "反対の面は触らないこと");
    }

    /// **「鍵を断られた」は引き直しのたびに動く。両向きとも**（ADR-0056 §決定4 / 作業単位 A-13b-1）。
    ///
    /// 鍵を直して上流が受け付けたら、次の照会で `false` に戻ること——前の値を持ち越すと、
    /// **直したのに画面が「断られた」と言い続ける。** 反対の面は動かない。
    #[test]
    fn a_refresh_moves_the_refused_key_flag_both_ways() {
        let mut facts = facts();
        let refused = UpstreamProbe {
            unauthorized: true,
            ..UpstreamProbe::unanswered()
        };

        facts.refresh(ModelFace::Main, &refused);
        assert_eq!(
            (facts.main().unauthorized(), facts.sub().unauthorized()),
            (true, false),
            "断られた面にだけ立つこと"
        );

        facts.refresh(
            ModelFace::Main,
            &probe(MAIN_MODEL, 4_096, AnsweredBy::NativeApi, &[MAIN_MODEL]),
        );
        assert!(
            !facts.main().unauthorized(),
            "受け付けた照会の後も「断られた」を持ち越している"
        );
    }

    /// **鍵を平文の `http://` で送る扱いは、引き直しのたびに動き、その面の応答にだけ出る**
    /// （ADR-0056 §決定5 / 作業単位 A-13b-2）。**警告が無い面では欄ごと省く**（`null` にしない）。
    ///
    /// 本文と裏方に**別の値**を入れて帰属を見る。解決先がローカルへ戻ったら、次の照会で欄が消えること
    /// （前の値を持ち越すと、直したのに画面が「承認が無い」と言い続ける）。
    #[test]
    fn the_plain_http_verdict_follows_each_refresh_and_stays_on_its_face() {
        let mut facts = facts();
        let with = |verdict: Option<KeyOverPlainHttp>| UpstreamProbe {
            key_over_plain_http: verdict,
            ..UpstreamProbe::unanswered()
        };
        let face_of = |facts: &UpstreamFacts, face: &str| {
            serde_json::to_value(UpstreamReport::of(facts, &no_launch(), None, None))
                .expect("応答を JSON にできること")[face]
                .get("key_over_plain_http")
                .cloned()
        };

        assert_eq!(
            face_of(&facts, "main"),
            None,
            "前提: 判定が無ければ欄ごと無い"
        );
        facts.refresh(ModelFace::Main, &with(Some(KeyOverPlainHttp::NotApproved)));
        facts.refresh(ModelFace::Sub, &with(Some(KeyOverPlainHttp::LocalNetwork)));
        assert_eq!(
            (face_of(&facts, "main"), face_of(&facts, "sub")),
            (
                Some(serde_json::json!("not_approved")),
                Some(serde_json::json!("local_network"))
            ),
            "面を取り違えないこと"
        );

        facts.refresh(ModelFace::Main, &with(Some(KeyOverPlainHttp::Approved)));
        assert_eq!(face_of(&facts, "main"), Some(serde_json::json!("approved")));

        facts.refresh(ModelFace::Main, &with(None));
        assert_eq!(
            (face_of(&facts, "main"), face_of(&facts, "sub")),
            (None, Some(serde_json::json!("local_network"))),
            "判定が消えたのに前の値を持ち越している／反対の面が動いた"
        );
    }

    /// **往復の失敗から、結果の種類を1箇所で決める**（作業単位 B-66 / [`CheckOutcome::of`]）。
    ///
    /// **401 と 403 は断られた、承認が無いのは送らなかった、届いたが一覧が読めない失敗は
    /// `NoModelList`、失敗が無ければ答えた**。**403 と 404・500 を置く**（「状態の失敗は全部断られた」と
    /// 「401 だけ」の両方の決め打ちを赤にする）。
    ///
    /// **繋がらない（`Connect` / `Timeout` / `Transport`）→ `NoAnswer` の腕はここに置けない**
    /// （`reqwest::Error` を外から組めない）。その腕は `server::context` の
    /// `a_check_says_when_and_how_the_upstream_answered`（落ちた宛先）が本物の往復で見る。
    ///
    /// > **レビュー是正（作業単位 B-66）**: 1周目は 404 / 500 / 読めない答えも `NoAnswer`（画面の「繋がりません」の帯）に
    /// > まとめていた。**モデル一覧は上流に「任意」の能力**（ADR-0038 §決定1）なので、一覧の口を持たない上流に
    /// > コンテキスト長を書いて繋ぐと、会話は通るのに帯が出続ける。
    #[test]
    fn the_outcome_of_a_check_is_read_from_the_failure() {
        use crate::llm::LlmError;
        for (failure, expected, what) in [
            (None, CheckOutcome::Answered, "失敗が無い"),
            (
                Some(LlmError::Status { status: 401 }),
                CheckOutcome::Refused,
                "401",
            ),
            (
                Some(LlmError::Status { status: 403 }),
                CheckOutcome::Refused,
                "403",
            ),
            (
                Some(LlmError::Status { status: 404 }),
                CheckOutcome::NoModelList,
                "404（一覧の口が無い）",
            ),
            (
                Some(LlmError::Status { status: 500 }),
                CheckOutcome::NoModelList,
                "500",
            ),
            (
                Some(LlmError::KeyNotApproved),
                CheckOutcome::KeyNotSent,
                "承認が無いので送らなかった",
            ),
            (
                Some(LlmError::MissingContextLength),
                CheckOutcome::NoModelList,
                "答えが読めない",
            ),
        ] {
            assert_eq!(CheckOutcome::of(failure.as_ref()), expected, "{what}");
        }
    }

    /// **応答は「いつ・どう確かめたか」を面ごとに言う**（作業単位 B-66 / Issue origin#94）。
    ///
    /// | 面 | 観測 | `checked_at` | `check_result` |
    /// |---|---|---|---|
    /// | 本文 | 答えた照会 | 照会の時刻 | `answered` |
    /// | 裏方 | まだ照会していない → 断られた照会 | `null` → 照会の時刻 | `not_checked` → `refused` |
    ///
    /// **面ごとに別の値**にして帰属を見る。**時刻は器が持った値を RFC 3339 で写すこと**まで見る
    /// （応答を組むときの時刻で埋める形は、読み直すたびに「いま確かめた」と嘘をつく）。
    #[test]
    fn the_report_says_when_and_how_each_face_was_checked() {
        let mut facts = facts();
        let view = |facts: &UpstreamFacts| {
            serde_json::to_value(UpstreamReport::of(facts, &no_launch(), None, None))
                .expect("応答を JSON にできること")
        };

        let before = view(&facts);
        for face in ["main", "sub"] {
            assert_eq!(
                (&before[face]["checked_at"], &before[face]["check_result"]),
                (&serde_json::Value::Null, &serde_json::json!("not_checked")),
                "照会していない面は「まだ確かめていない」と言うこと（{face}）"
            );
        }

        let answered = probe(MAIN_MODEL, 4_096, AnsweredBy::NativeApi, &[MAIN_MODEL]);
        let answered_at = answered
            .check
            .map(|check| check.at())
            .expect("前提: 照会した回の観測");
        facts.refresh(ModelFace::Main, &answered);
        let refused_at = chrono::DateTime::parse_from_rfc3339("2026-11-03T01:02:03Z")
            .expect("時刻を読めること")
            .with_timezone(&chrono::Utc);
        facts.refresh(
            ModelFace::Sub,
            &UpstreamProbe {
                unauthorized: true,
                check: Some(UpstreamCheck {
                    at: refused_at,
                    outcome: CheckOutcome::Refused,
                }),
                ..UpstreamProbe::unanswered()
            },
        );

        let after = view(&facts);
        assert_eq!(
            (
                &after["main"]["check_result"],
                &after["sub"]["check_result"]
            ),
            (
                &serde_json::json!("answered"),
                &serde_json::json!("refused")
            ),
            "面ごとに、その面の結果を言うこと"
        );
        assert_eq!(
            (&after["main"]["checked_at"], &after["sub"]["checked_at"]),
            (
                &serde_json::json!(answered_at.to_rfc3339()),
                &serde_json::json!(refused_at.to_rfc3339())
            ),
            "照会した時刻を、その面に写すこと"
        );
    }

    /// **引き直しても、宛先とモデルの綴りは動かない**（起動時に決まっている事実）。
    #[test]
    fn a_refresh_never_moves_the_endpoint_or_the_model() {
        let mut facts = facts();
        facts.refresh(
            ModelFace::Sub,
            &probe(SUB_MODEL, 2_048, AnsweredBy::CompatSoleEntry, &[SUB_MODEL]),
        );

        assert_eq!(facts.sub().endpoint(), SUB_ENDPOINT);
        assert_eq!(facts.sub().model(), SUB_MODEL);
    }

    /// **答えなかった回は「答えなし」になり、判定は全部「聞けなかった」。**
    ///
    /// **上流が停止した状態そのもの。** 予算は現在値を保つ（`context::refresh_once`）ので、
    /// **ここが `None` になることだけが、外から見える手掛かりになる。**
    #[test]
    fn an_upstream_that_stopped_answering_is_recorded_as_unanswered() {
        let mut facts = facts();
        facts.refresh(
            ModelFace::Main,
            &probe(MAIN_MODEL, 4_096, AnsweredBy::NativeApi, &[MAIN_MODEL]),
        );
        assert!(
            facts.main().answered_by().is_some(),
            "前提: 一度は答えていること"
        );

        facts.refresh(ModelFace::Main, &UpstreamProbe::unanswered());

        assert_eq!(facts.main().answered_by(), None);
        assert_eq!(facts.main().queried_context_tokens(), None);
        let measured = facts.main().measured();
        assert!(measured.checked() > 0, "見る行があること");
        assert_eq!(
            measured.unanswered,
            measured.checked(),
            "聞けなかった面は全行を「聞けなかった」に数えること"
        );
    }

    /// **4つの数え上げが、それぞれ自分の欄へ出る**（作業単位 G-10）。
    ///
    /// # なぜ純粋関数の側で見るのか
    ///
    /// **統合テストのフィクスチャでは 4 欄のうち 2 つが 0 になる**
    /// ——`matched` は依頼する綴りが `measured_on`（既定のメインモデル）と
    /// 一致したときにしか立たないので、**「既定で回さない」と両立しない。**
    /// **0 どうしの入れ替えは、どんなアサートでも観測できない**（CLAUDE.md の最頻の穴）。
    ///
    /// **ここは 4 つとも別の値**にして、欄の取り違えを1本で赤にする。
    ///
    /// **実測**（作業単位 G-10 / 2026-09-08）——
    /// **`matched` と `drifted_absent` を入れ替える変異は、この1本しか赤にしない**
    /// （2004 passed / 1 failed / exit 101。**この1本を外すと 2004 全緑**）。
    /// **統合テストの腕はどちらの欄も 0 なので、原理的に区別できない。**
    #[test]
    fn the_breakdown_keeps_every_counter_apart() {
        let view = MeasuredView::of(FaceTally {
            matched: 1,
            drifted_other: 2,
            drifted_absent: 3,
            unanswered: 4,
        });

        assert_eq!(
            serde_json::to_value(&view).expect("内訳を JSON にできること"),
            serde_json::json!({
                "checked": 10,
                "matched": 1,
                "drifted_other": 2,
                "drifted_absent": 3,
                "unanswered": 4,
            }),
            "数え上げが自分の欄へ出ること"
        );
    }

    /// **応答が面ごとに別の観測を載せる**（帰属 / ADR-0039 §決定4）。
    ///
    /// **鍵（`main` / `sub`）と、中の `face` の両方**を見る
    /// ——1件を取り出して眺めたときに帰属が消えないこと。
    ///
    /// **本番の入口を通る腕は `tests/server_admin_upstream.rs::
    /// the_two_upstreams_come_back_named_apart`。** ここが見ているのは組み立てだけである。
    #[test]
    fn the_report_puts_each_face_under_its_own_key() {
        let facts = facts();
        let report = UpstreamReport::of(&facts, &no_launch(), None, None);

        assert_eq!(
            serde_json::to_value(&report).expect("応答を JSON にできること"),
            serde_json::json!({
                "main": {
                    "face": "main",
                    "endpoint": MAIN_ENDPOINT,
                    "model": MAIN_MODEL,
                    // **一度も聞いていない面は、名乗るものを持っていない。**
                    "answered_name": serde_json::Value::Null,
                    "queried_context_tokens": serde_json::Value::Null,
                    "answered_by": serde_json::Value::Null,
                    // **鍵を断られていない**（一度も聞いていない / 作業単位 A-13b-1）。
                    "unauthorized": false,
                    // **一度も聞いていない**と言う（作業単位 B-66）。
                    "checked_at": serde_json::Value::Null,
                    "check_result": "not_checked",
                    "measured": {
                        // **面ごとに行数が違う**（`MeasuredRole::Sub` の定数が1つある）。
                        // 同数だと、内訳を面ごと入れ替えても気づけない。
                        "checked": 4,
                        "matched": 0,
                        "drifted_other": 0,
                        "drifted_absent": 0,
                        "unanswered": 4,
                    },
                    // **起こしていない面は `external`**（ADR-0042 §決定3）。
                    "launch": {
                        "state": "external",
                        "since": serde_json::Value::Null,
                        "answered_name": serde_json::Value::Null,
                        "reason": serde_json::Value::Null,
                        "log": serde_json::Value::Null,
                    },
                },
                "sub": {
                    "face": "sub",
                    "endpoint": SUB_ENDPOINT,
                    "model": SUB_MODEL,
                    "answered_name": serde_json::Value::Null,
                    "queried_context_tokens": serde_json::Value::Null,
                    "answered_by": serde_json::Value::Null,
                    "unauthorized": false,
                    "checked_at": serde_json::Value::Null,
                    "check_result": "not_checked",
                    "measured": {
                        "checked": 5,
                        "matched": 0,
                        "drifted_other": 0,
                        "drifted_absent": 0,
                        "unanswered": 5,
                    },
                    "launch": {
                        "state": "external",
                        "since": serde_json::Value::Null,
                        "answered_name": serde_json::Value::Null,
                        "reason": serde_json::Value::Null,
                        "log": serde_json::Value::Null,
                    },
                },
                // **見本を書いていない起動は、書いたと言わない**（作業単位 F-4c）。
                "profile_sample": serde_json::Value::Null,
                // **プロファイルを選んでいない起動は、選んだと言わない**（作業単位 F-10b）。
                "selected_profile": serde_json::Value::Null,
            }),
            "面ごとの観測が、その面の鍵の下に出ること"
        );
    }

    /// **起こした上流の状態は、面ごとに、その面のものが出る**（ADR-0042 §決定3）。
    ///
    /// # なぜ帰属を別に見るのか
    ///
    /// **`external` はどちらの面でも起きる**ので、
    /// **どちらも起こしていない腕では、面を入れ替えても両方 `external` のまま**である
    /// （上の `the_report_puts_each_face_under_its_own_key` はその状態で回っている）。
    /// **2つの面に別々の状態を入れて初めて、取り違えが観測できる**
    /// （CLAUDE.md「抜けるのはいつも帰属」）。
    ///
    /// # ⚠ 置き場所は載らない（**絶対制約**）
    ///
    /// **`log` は `upstream-logs/` の下のファイル名だけ。**
    /// **区切り（`/` も `\\`）を1つも含まないこと**まで見る
    /// ——`the_response_never_carries_the_upstreams_own_model_id` と同じ向き。
    #[test]
    fn the_launch_state_of_each_face_comes_back_under_its_own_key() {
        let facts = facts();
        let report = serde_json::to_value(UpstreamReport::of(&facts, &split_launch(), None, None))
            .expect("応答を JSON にできること");

        assert_eq!(
            report["main"]["launch"]["state"],
            serde_json::json!("ready")
        );
        assert_eq!(
            report["main"]["launch"]["answered_name"],
            serde_json::json!(MAIN_ANSWERED_GGUF)
        );
        // **準備完了の面は、失敗の欄を持たない。**
        assert_eq!(report["main"]["launch"]["reason"], serde_json::Value::Null);
        assert_eq!(report["main"]["launch"]["log"], serde_json::Value::Null);

        assert_eq!(
            report["sub"]["launch"]["state"],
            serde_json::json!("failed")
        );
        assert_eq!(
            report["sub"]["launch"]["reason"],
            serde_json::json!(SUB_FAILURE_REASON)
        );
        assert_eq!(
            report["sub"]["launch"]["log"],
            serde_json::json!(SUB_FAILURE_LOG)
        );
        // **失敗した面は、名乗るものを持たない。**
        assert_eq!(
            report["sub"]["launch"]["answered_name"],
            serde_json::Value::Null
        );

        // **応答のどこにも置き場所が出ていないこと**（絶対制約）。
        let rendered = report.to_string();
        assert!(
            rendered.contains(SUB_FAILURE_LOG),
            "ログの名前そのものは出ること（空振りではない）: {rendered}"
        );
        for separator in ['/', '\\'] {
            assert!(
                !SUB_FAILURE_LOG.contains(separator),
                "フィクスチャが区切りを含んでいる（この検査が何も見ていない）"
            );
        }
    }

    /// **見本を書いた起動だけが、その場所を言う**（作業単位 F-4c / Issue origin#23）。
    ///
    /// # 両向きを1本に置く
    ///
    /// | 起動 | `profile_sample` |
    /// |---|---|
    /// | この起動が見本を書いた | **書いた先のパス**（そのままの綴り） |
    /// | 書いていない（既に在った・別の置き場所を指した） | **`null`**——「書いた」と言わない |
    ///
    /// **片側だけだと「常に null」「常に何か言う」の片方が通る。**
    ///
    /// # ⚠ パスが応答に載る（**絶対制約の例外**）
    ///
    /// **エラーレスポンスではなく、127.0.0.1 の利用者本人への状態表示**である
    /// （司令塔の決定 / [`UpstreamReport::profile_sample`] の doc）。
    #[test]
    fn the_report_names_the_sample_only_when_this_run_wrote_it() {
        let facts = facts();

        let silent = serde_json::to_value(UpstreamReport::of(&facts, &no_launch(), None, None))
            .expect("応答を JSON にできること");
        assert_eq!(
            silent["profile_sample"],
            serde_json::Value::Null,
            "書いていない起動が、書いたと言っている: {silent}"
        );

        // **わざと区切りを両方含む綴り**（Windows の形）で、**加工されずに出る**ことを見る。
        let written =
            std::path::Path::new(r"C:\Users\someone\AppData\Local\personacore\upstreams.json");
        let spoken = serde_json::to_value(UpstreamReport::of(
            &facts,
            &no_launch(),
            Some(written),
            None,
        ))
        .expect("応答を JSON にできること");
        assert_eq!(
            spoken["profile_sample"],
            serde_json::json!(r"C:\Users\someone\AppData\Local\personacore\upstreams.json"),
            "書いた先をそのまま言うこと: {spoken}"
        );
        // **面の側は1バイトも変わらない**（欄を足しただけ）。
        assert_eq!(spoken["main"], silent["main"]);
        assert_eq!(spoken["sub"], silent["sub"]);
    }

    /// **いま選ばれているプロファイルの名前を、選んだ起動だけが言う**（作業単位 F-10b / ADR-0055 §決定3）。
    ///
    /// | 起動 | `selected_profile` |
    /// |---|---|
    /// | プロファイルを選んでいない（環境変数の既定） | **`null`** |
    /// | 選んでいる | **その名前**（そのままの綴り） |
    ///
    /// **画面の「起こす」はこの名前を `POST /admin/upstream/profile` へ送る**ので、
    /// **見本の場所（`profile_sample`）と取り違えると、無い名前を頼むことになる** ——
    /// 2つを同時に別の値で入れて、帰属まで見る。
    #[test]
    fn the_report_names_the_selected_profile_only_when_one_is_selected() {
        let facts = facts();
        let written = std::path::Path::new("upstreams.json");

        let unselected = serde_json::to_value(UpstreamReport::of(
            &facts,
            &no_launch(),
            Some(written),
            None,
        ))
        .expect("応答を JSON にできること");
        assert_eq!(
            unselected["selected_profile"],
            serde_json::Value::Null,
            "選んでいない起動が、選んだと言っている: {unselected}"
        );

        let selected = serde_json::to_value(UpstreamReport::of(
            &facts,
            &no_launch(),
            Some(written),
            Some("qwen-official"),
        ))
        .expect("応答を JSON にできること");
        assert_eq!(
            selected["selected_profile"],
            serde_json::json!("qwen-official"),
            "{selected}"
        );
        assert_eq!(
            selected["profile_sample"],
            serde_json::json!("upstreams.json"),
            "見本の場所と取り違えている: {selected}"
        );
        assert_eq!(selected["main"], unselected["main"]);
        assert_eq!(selected["sub"], unselected["sub"]);
    }

    /// **読み手の綴りと、申告された窓が、そのまま応答へ出る。**
    ///
    /// 上の1本は**答えなし**の状態しか見ていない
    /// ——`None` どうしなら、`answered_by` と `queried_context_tokens` を
    /// 取り違えても両方 `null` のままである。
    #[test]
    fn a_face_that_answered_reports_who_answered_and_what_it_declared() {
        let mut facts = facts();
        facts.refresh(
            ModelFace::Sub,
            &probe(SUB_MODEL, 2_048, AnsweredBy::CompatExactId, &[SUB_MODEL]),
        );

        let report = serde_json::to_value(UpstreamReport::of(&facts, &no_launch(), None, None))
            .expect("応答を JSON にできること");

        assert_eq!(
            report["sub"]["answered_by"],
            serde_json::json!("openai-exact-id")
        );
        assert_eq!(
            report["sub"]["queried_context_tokens"],
            serde_json::json!(2_048)
        );
        // **依頼した綴りがそのまま一覧に在った**ので、答えた名前も同じ綴り。
        assert_eq!(report["sub"]["answered_name"], serde_json::json!(SUB_MODEL));
        // 反対の面は「答えなし」のまま。
        assert_eq!(report["main"]["answered_by"], serde_json::Value::Null);
        assert_eq!(report["main"]["answered_name"], serde_json::Value::Null);
        assert_eq!(
            report["main"]["queried_context_tokens"],
            serde_json::Value::Null
        );
    }

    /// **一覧はここに残らない。残るのは、答えた名前のファイル名だけ**
    /// （このモジュールの doc / 作業単位 B-45）。
    ///
    /// # ⚠ この1本は、B-45 で意味が変わった（司令塔が承認 / 2026-09-17）
    ///
    /// **着手前は「上流が名乗った綴りを1文字も持たない」だった。**
    /// **いまは「ディレクトリ部分を持たない」を見る。**
    ///
    /// # 空振りではないこと
    ///
    /// **ファイル名が現に保持されていること**まで見る
    /// ——落とす実装でも、一覧ごと抱え込む実装でも赤になる。
    ///
    /// **⚠ `Debug` は `\` を `\\` へ逃がす**ので、**逃がした形も一緒に探す。**
    ///
    /// 本番の入口を通る腕は
    /// `tests/server_upstream_facts.rs::the_held_facts_never_carry_the_upstreams_own_model_id`。
    #[test]
    fn the_catalogue_the_upstream_listed_is_not_kept() {
        /// 上流が名乗る綴りのうち、**外へ出てよい部分**。
        const UPSTREAM_FILE: &str = "Ministral-3-14B-Instruct-2512-Q4_K_M.gguf";

        for (upstream_id, directory, what) in [
            (
                "/models/Ministral-3-14B-Instruct-2512-Q4_K_M.gguf",
                "/models/",
                "POSIX 形の絶対パス",
            ),
            (
                "C:\\models\\llm\\Ministral-3-14B-Instruct-2512-Q4_K_M.gguf",
                "C:\\models\\llm\\",
                "Windows 形の絶対パス",
            ),
        ] {
            let mut facts = facts();
            facts.refresh(
                ModelFace::Main,
                &probe(
                    MAIN_MODEL,
                    4_096,
                    AnsweredBy::CompatSoleEntry,
                    &[upstream_id],
                ),
            );

            assert_eq!(
                facts.main().answered_by(),
                Some(AnsweredBy::CompatSoleEntry),
                "{what}: 前提: その一覧を実際に通していること（通っていなければ何も守っていない）"
            );
            assert_eq!(
                facts.main().answered_name(),
                Some(UPSTREAM_FILE),
                "{what}: ファイル名だけは名乗ること"
            );

            let rendered = format!("{facts:?}");
            assert!(
                !rendered.contains(upstream_id),
                "{what}: 上流が名乗った綴りを丸ごと保持しないこと: {rendered}"
            );
            // **`Debug` は `\` を `\\` へ逃がす。** 生の綴りだけを探すと空振りする。
            for needle in [directory.to_owned(), directory.replace('\\', "\\\\")] {
                assert!(
                    !rendered.contains(&needle),
                    "{what}: ディレクトリ {needle:?} を保持しないこと: {rendered}"
                );
            }
            assert!(
                rendered.contains(UPSTREAM_FILE),
                "{what}: ファイル名は現に保持されていること（空振り防止）: {rendered}"
            );
        }
    }

    /// **一度も聞いていない面は、名乗るものを持っていない**（作業単位 B-45）。
    ///
    /// **空文字にしないこと。** 画面は `null` を「上流は名乗っていません」と出すので、
    /// `Some("")` は「名乗ったが空だった」という在りえない状態を画面に出す。
    #[test]
    fn a_face_that_was_never_queried_names_nothing() {
        let facts = facts();
        assert_eq!(facts.main().answered_name(), None);
        assert_eq!(facts.sub().answered_name(), None);
    }

    /// **答えた名前の決め方**（純粋関数の表 / [`name_that_answered`]）。
    ///
    /// **⚠ 器がこの関数を通しているかは、ここでは見ていない**
    /// （`only_the_userinfo_query_and_fragment_are_redacted` と同じ立場 /
    /// CLAUDE.md `E-3`——道具に腕が n 本あるなら、対照も n 本要る）。
    /// **通していることを見るのは、上の1本と本番の入口を通る統合テストのほう。**
    ///
    /// # ⚠ 名前は、いまの主張より狭い（レビュー是正 / 2026-09-17）
    ///
    /// **「ファイル名だけ」になるのは、パスに見える綴りのときだけ**
    /// （[`looks_like_a_path`]）。**`vendor/model` 形はそのまま名乗る。**
    /// **名前は据え置いた**——是正の指示がこの綴りで名指ししているので、
    /// 追跡できる形を残した。**綴りを参照しているのはこの1行だけ**
    /// （**リポジトリ全体を `grep` して数えた** / 2026-09-17）なので、
    /// **改名の判断は司令塔に預ける。**
    #[test]
    fn the_name_that_answered_is_only_ever_a_file_name() {
        for (model, catalogue, expected, what) in [
            (
                MAIN_MODEL,
                &["/models/llm/deep.gguf"] as &[&str],
                Some("deep.gguf"),
                "POSIX 形は、最後の / の後ろだけ",
            ),
            (
                MAIN_MODEL,
                &["C:\\models\\llm\\deep.gguf"],
                Some("deep.gguf"),
                "Windows 形も同じ（片方しか見ないと、この形を丸ごと通す）",
            ),
            (
                MAIN_MODEL,
                &["C:\\models/llm\\deep.gguf"],
                Some("deep.gguf"),
                "混ざっていても、最後の区切りで切る",
            ),
            (
                MAIN_MODEL,
                &["plain-model-id"],
                Some("plain-model-id"),
                "区切りが無ければ、その綴りそのもの",
            ),
            (
                MAIN_MODEL,
                &["/models/other.gguf", MAIN_MODEL],
                Some(MAIN_MODEL),
                "依頼した綴りが在れば、それを名乗る（一覧が2件でも決まる）",
            ),
            (
                "vendor/model-2512",
                &["vendor/model-2512", "/models/other.gguf"],
                Some("vendor/model-2512"),
                "依頼した綴りに / が入っていても切らない（切ると、違うものが答えたと嘘をつく）",
            ),
            (
                MAIN_MODEL,
                &["/models/a.gguf", "/models/b.gguf"],
                None,
                "2件あって一致も無ければ、どれが答えたか決められない",
            ),
            (
                MAIN_MODEL,
                &[VENDOR_STYLE_ID],
                Some(VENDOR_STYLE_ID),
                "一覧が1件でも、パスに見えない綴りは切らない（切ると画面が嘘をつく）",
            ),
            (
                MAIN_MODEL,
                &["models\\deep"],
                Some("deep"),
                "\\ を含めばパス（この手掛かりだけが効く形）",
            ),
            (
                MAIN_MODEL,
                &["/models/deep"],
                Some("deep"),
                "/ で始まればパス（この手掛かりだけが効く形）",
            ),
            (
                MAIN_MODEL,
                &["models/deep.gguf"],
                Some("deep.gguf"),
                ".gguf で終わればパス（この手掛かりだけが効く形）",
            ),
            (
                MAIN_MODEL,
                &["C:/models/deep"],
                Some("deep"),
                "2文字目が : ならパス（C:/ の形。この手掛かりだけが効く形）",
            ),
            (MAIN_MODEL, &[], None, "空の一覧では名乗らない"),
            (
                MAIN_MODEL,
                &["/models/"],
                None,
                "区切りで終わる綴りは空になるので、名乗らない（Some(\"\") にしない）",
            ),
        ] {
            let owned: Vec<String> = catalogue.iter().map(|id| (*id).to_owned()).collect();
            assert_eq!(
                name_that_answered(model, &owned).as_deref(),
                expected,
                "{what}"
            );
        }
    }
}
