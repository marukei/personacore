//! 上流の**実コンテキスト長**からトークン予算を決める（ADR-0013 第2層）。
//!
//! # 優先順位
//!
//! **明示設定 → 上流への照会 → 保守的な既定値 ＋ `warn`。**
//!
//! | 段 | 出どころ | 失敗したら |
//! |---|---|---|
//! | 1 | `PERSONACORE_CONTEXT_TOKENS` | **起動時に落とす**（`server::config`） |
//! | 2 | LM Studio の `/api/v0/models` の `loaded_context_length` | 次の段へ |
//! | 3 | `TokenBudget::default()`（8192） | — |
//!
//! # ここは「取れたら得をする」経路
//!
//! **照会の失敗で起動を止めない。** 上流は普段停止している運用（docs/04-environment.md）で、
//! しかも `/api/v0/models` は **LM Studio 独自**なので、他の上流に差し替えれば必ず失敗する。
//! 止めると「LM Studio が起動していないとサーバが起動しない」という逆立ちした依存になる。
//!
//! **代わりに `warn` を必ず残す。** 既定値へ落ちた状態は
//! 「実コンテキストの半分しか使っていない」（実測 16384 に対して 8192）という
//! **エラーも出ず応答も返る**壊れ方なので、ログだけが手掛かりになる。
//!
//! # 比率で膨らませない
//!
//! 取得した値は**そのまま**予算のコンテキスト長にする。見積もり単位（UTF-8 バイト数）と
//! 実トークンを 1:1 で対応させる、という第1層の上界性を壊さないため
//! （最悪ケースが 1.00 倍なので、膨らませた瞬間に上界が破れる。ADR-0013）。

use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use tokio::task::JoinHandle;

use super::config::BodyFace;
use super::extract::SubContext;
use super::upstream::{
    CheckOutcome, UpstreamCheck, UpstreamFacts, UpstreamObservation, UpstreamProbe,
    name_that_answered,
};
use crate::llm::{
    ApiKey, ChatRequest, ChatStream, Completion, LlmClient, LlmError, LlmFuture, LmStudioClient,
    ModelId, ReasoningEffort, Sampling, TokenCount,
};
use crate::measured::ModelFace;
use crate::prompt::TokenBudget;

/// 予算を決める。**失敗しない**（どんな経路でも値が決まる）。
///
/// 明示設定があるときは**照会そのものを行わない。** 結果を捨てる往復を
/// 起動のたびに1回増やす意味が無い（ADR-0013 の優先順位でも明示設定が上）。
///
/// # **本文の上流が持っているモデルの一覧も一緒に返す**（作業単位 G-3a の是正）
///
/// **同じ照会の本文から読める**ので、**往復は1回も増えない**
/// （[`LmStudioClient::models`] ＋ [`crate::llm::ModelsReport::into_parts`]。
/// `resolve_sub_context` が E-2 でやったのと同じ形）。
/// 使うのは [`crate::measured::report`] の**本文の面**。
///
/// ## ⚠ ここは「聞いていない」ではなく「聞いたが捨てていた」だった
///
/// **是正前、この関数は [`LmStudioClient::loaded_context_length`] を呼んでいた**
/// ——その中身は `self.models().await?.context_length` で、
/// **[`crate::llm::ModelsReport`] の `catalogue` を捨てているだけ**である。
/// **G-3a のレビューがカウンタを植えて実測**（2026-09-07）:
/// **上流が2つの起動でも `main /api/v0/models hits = 1`。**
///
/// **`server::mod` と `measured` の doc は、そこを「本文の側には一度も聞いていない」と
/// 書いていた。偽だった。** その結果、**本文の面の 4 件が
/// [`crate::measured::ModelCheck::Unanswered`] に落ちていた**
/// ——落ちる先に **`MEASURED_CONTROL_TOKEN_SPELLINGS`**（ADR-0033 §決定3 が
/// 「最優先」と名指しした行）が含まれる。
///
/// ## `None` を返す条件は2つだけ（**どちらも事実の申告**）
///
/// | `None` になる場合 | 意味 |
/// |---|---|
/// | **明示設定がある**（`PERSONACORE_CONTEXT_TOKENS`） | **本当に一度も聞いていない**（早期 return） |
/// | **往復そのものが失敗した** | 聞いたが答えが無い（上流の停止・別実装） |
///
/// **応答が読めた回は、設定中のモデルが未ロードでも一覧は `Some`**
/// （[`crate::llm::ModelsReport`] の2つは失敗の仕方が違う）。
/// **これは上の表の2通りには入っていない3つ目の状態**——
/// **往復は成功していて、コンテキスト長だけが読めない。**
///
/// **見張り番は `the_body_side_query_brings_back_the_catalogue_it_read`（この4通り）と、
/// 本番の入口を通る `tests/server_measured_model.rs` の
/// `two_upstreams_are_each_judged_against_their_own_catalogue`。**
///
/// > **⚠ 未ロードの腕は、是正2周目で足した**（2026-09-07）。
/// > **それまでこの doc は「見張り番は（この3通り）」と名指ししていたが、
/// > その3通りに未ロードの状態は入っていなかった**（照会が通る腕は `loaded`）。
/// > **未ロードの回に一覧まで捨てる変異を赤にしていたのは、
/// > `two_upstreams_are_each_judged_against_their_own_catalogue` ただ1本**
/// > （レビュアーが実測 / `m19`）。**名指しと、実際に赤にしている者が違っていた。**
///
/// **`body` は本文の面に書いた値**（返答の上限が予算の余白になる / ADR-0059 §決定2 —— [`choose_budget`]）。
pub(crate) async fn resolve_budget(
    explicit: Option<TokenBudget>,
    upstream: &LmStudioClient,
    body: BodyFace,
) -> (Chosen<TokenBudget>, UpstreamProbe) {
    if explicit.is_some() {
        // **ここだけが「一度も聞いていない」**（照会を行わないので一覧も読み手も無い）。
        // **鍵を平文の `http://` で送る扱いは載せる**（作業単位 A-13b-2）—— IP アドレスの宛先は
        // 組んだときに判定が決まっている（ホスト名は、まだ解決していないので `None`）。
        let probe = UpstreamProbe {
            key_over_plain_http: upstream.key_over_plain_http(),
            ..UpstreamProbe::unanswered()
        };
        return (choose_budget(explicit, None, body), probe);
    }

    // **同じ1回の往復から3つ読む**（`resolve_sub_context` と同じ形）。
    // **`loaded_context_length()` へ戻さないこと**——あれは同じ往復の一覧と読み手を捨てる。
    let (probe, failure) = probe_upstream(upstream).await;
    if let Some(err) = failure {
        // 詳細は `#[source]` 側に残る。ここで止めないことが要点。
        tracing::info!(error = %err, "上流へコンテキスト長を照会できませんでした");
    }

    (choose_budget(None, probe.context_tokens, body), probe)
}

/// **上流への照会1回ぶん。** 一覧も読み手も捨てない（作業単位 G-3b）。
///
/// # ⚠ [`LmStudioClient::loaded_context_length`] へ戻さないこと
///
/// あれは `self.models().await?.context_length` で、
/// **同じ往復の一覧（[`crate::llm::ModelsReport::catalogue`]）と
/// 読み手（[`crate::llm::AnsweredBy`]）を捨てているだけ**である。
/// **G-3a はこの罠を起動側で1回踏んで直した**（`resolve_budget` の doc）。
/// **引き直しの側は G-3b まで同じ形のまま残っていた。**
///
/// **往復は1回。** 数える見張り番は `the_requery_reads_everything_from_one_round_trip`
/// （**間隔ごとの `/api/v0/models` の本数**を数える）。
///
/// # 返り値の2つ目は「文言を決めるための材料」
///
/// **深刻度と文面は呼び出し側が決める**（本文の予算とサブモデルで調査先が違う）。
/// **`probe.answered_by` が `None` なら往復そのものの失敗**、
/// `Some` なのに `context_tokens` が `None` なら「答えたが長さが読めない」。
///
/// # ⚠ **ここが継ぎ目**——上流が名乗った綴りは、ここでファイル名だけになる
///
/// **`llama-server` の `id` は gguf の絶対パス**で、
/// **[`UpstreamProbe::answered_name`] から先（観測・応答本文・画面）へは、
/// そのディレクトリを1文字も通さない**（絶対制約 / `server::upstream` の doc）。
///
/// **依頼する綴りはクライアントから取る**（`server::App::bind` と同じ理由）
/// ——設定を読み直すと、「設定はこう書いてあるが、クライアントは別のモデルを
/// 向いている」形が観測できない。
///
/// # ⚠ 呼び出し側が3つに増えた（作業単位 F-4b-1）
///
/// 起動時（[`resolve_budget`] / [`resolve_sub_context`]）と引き直し（[`refresh_once`] /
/// [`refresh_sub_once`]）に加えて、**自分で起こした上流が載るのを待つ側**
/// （`super::launch::FaceWatcher`）がここを通る。**判定を作る場所を増やさない**
/// ——「答えた名前」を決める規則（`upstream::name_that_answered`）は、
/// **この関数だけ**が通る継ぎ目である。
///
/// > **作業単位 B-51 で、番人の「待つ間」だけは [`probe_face_while_loading`] へ移った**
/// > （載るまでの 503 を警告にしないため）。**読み方の本体は [`probe_with`] に1つだけ置き、
/// > この関数もそれを通る**ので、上の「継ぎ目は1つ」は崩れていない
/// > （入口が2つ・読み方が1つ）。載った後に残りの面を聞き直すのは今もここ（[`probe_face`] 経由）。
pub(in crate::server) async fn probe_upstream(
    client: &LmStudioClient,
) -> (UpstreamProbe, Option<LlmError>) {
    probe_with(client, UpstreamPhase::Settled).await
}

/// **聞く相手が、いまどういう局面にいるか**（作業単位 B-51）。
///
/// **効くのは「一覧がどちらの綴りでも読めなかった」1行の水準だけ**
/// （`LmStudioClient::models` / `LmStudioClient::models_while_loading`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamPhase {
    /// 起こしていない面・載った後・引き直し。**読めなければ警告。**
    Settled,
    /// **自分で起こした上流が載るのを待つ間。** 読めないのは期待している状態（`debug`）。
    Loading,
    /// **コンテキスト長を書いた面への、観測のためだけの照会**（作業単位 B-66 のレビュー是正）。
    /// 読めなくても警告にしない（`debug` / `LmStudioClient::models_for_observation`）——予算に使わず、
    /// 結果は観測の `check_result` が言う。
    Observing,
}

/// **引き直し・観測の照会の局面**（作業単位 B-66 のレビュー是正）。明示の面は観測のためだけに聞く。
const fn phase_for(explicit: bool) -> UpstreamPhase {
    if explicit {
        UpstreamPhase::Observing
    } else {
        UpstreamPhase::Settled
    }
}

/// **往復1回を、観測1回ぶんに読む**（[`probe_upstream`] と [`probe_face_while_loading`] が共有する）。
///
/// **「答えた名前」を決める規則はここにしか無い**（[`probe_upstream`] の doc の「継ぎ目」）
/// ——作業単位 B-51 で往復の入口が2つになったが、**読み方は1つのまま。**
///
/// **⚠ 返り値の型（`llm::ModelsReport`）をこのファイルで綴らないこと** ——
/// `llm::lmstudio` の公開名なので、`upstream_leaks` の台帳に (このファイル, その名前) が1組増える。
/// **局面を引数で受けて、往復もここで打つ**のはそのため。
async fn probe_with(
    client: &LmStudioClient,
    phase: UpstreamPhase,
) -> (UpstreamProbe, Option<LlmError>) {
    let outcome = match phase {
        UpstreamPhase::Settled => client.models().await,
        UpstreamPhase::Loading => client.models_while_loading().await,
        UpstreamPhase::Observing => client.models_for_observation().await,
    };
    // **鍵を平文の `http://` で送る扱いは、照会の後に読む**（ADR-0056 §決定5 / 作業単位 A-13b-2）——
    // ホスト名の宛先は、この照会の名前解決で判定が決まる。成功でも失敗でも同じ値を載せる。
    let key_over_plain_http = client.key_over_plain_http();
    match outcome {
        Ok(report) => {
            let (length, catalogue, answered_by) = report.into_parts();
            let (context_tokens, failure) = match length {
                Ok(tokens) => (Some(tokens), None),
                Err(err) => (None, Some(err)),
            };
            (
                UpstreamProbe {
                    // **答えた名前は、この往復の一覧から決める**（作業単位 B-45）。
                    // **一覧を持ち回るのはこの関数の中だけ**なので、
                    // **切り落とすならここしかない。**
                    answered_name: name_that_answered(client.config().model().as_str(), &catalogue),
                    // **一覧は取れている**（設定中のモデルが未ロードでも、一覧そのものは読めた）。
                    catalogue: Some(catalogue),
                    context_tokens,
                    answered_by: Some(answered_by),
                    // **一覧は読めた**（上流は鍵を受け付けている）。
                    unauthorized: false,
                    key_over_plain_http,
                    // **答えた**（作業単位 B-66）。コンテキスト長が読めなかった回（`failure` が `Some`）も、
                    // 上流は一覧を出しているので「答えた」——結果の種類は往復そのものの成否で決める。
                    check: Some(UpstreamCheck::now(CheckOutcome::of(None))),
                },
                failure,
            )
        }
        // **往復そのものが失敗した。** 一覧も読み手も「聞けなかった」。
        // **鍵を断られた（401 / 403）ことだけは持ち上げる**（ADR-0056 §決定4）——
        // 上流の画面が「届かない」と「断られた」を見分けられるように。
        // **送らなかった（承認が無い）ことも持ち上げる**（`key_over_plain_http` が `not_approved`）。
        // **照会したことは残す**（作業単位 B-66）——「照会していない」と「照会したが答えない」を割る。
        Err(err) => (
            UpstreamProbe {
                unauthorized: err.refused_the_key(),
                key_over_plain_http,
                check: Some(UpstreamCheck::now(CheckOutcome::of(Some(&err)))),
                ..UpstreamProbe::unanswered()
            },
            Some(err),
        ),
    }
}

/// **サブモデルの実コンテキスト長を決める**（作業単位 B-7 / ADR-0036 決定2 の第1階層）。
/// **失敗しない**（引けなければ落とし所）。
///
/// # 本文用とは別の照会である
///
/// **照会先のモデルが違う。** `upstream` には**サブモデルを指すクライアント**
/// （`LmStudioClient::with_model` の複製）を渡すこと。本文用を渡すと、
/// **抽出の上限が本文モデルの窓から決まる**——2つのモデルが違う長さで
/// ロードされている環境では、上限が過大（窓を超えた依頼を送る / ADR-0021 §8。LM Studio では
/// 前方から黙って切り捨てられる / ADR-0013。llama は 400 `exceed_context_size_error` で断るか、
/// プロンプト ＋ 応答の上限が窓を超えて応答が黙って短くなる / ADR-0052 M5）にも
/// 過小（入るはずの会話を捨てる）にもなる。**見張り番は
/// `tests/server_memory_extraction.rs` の
/// `the_extraction_limit_follows_the_sub_models_context_through_the_real_bind`**
/// ——本文用とサブ用に**別の値**を申告する上流を立てて、2つの腕で見る。
///
/// # ⚠ **`PERSONACORE_CONTEXT_TOKENS` はここを止めない**
///
/// あの設定が優先するのは**本文の予算**であって、抽出の上限ではない
/// （ADR-0020 §4 の理由——「優先されている以上、引いても変わらない」——が
/// こちらには当てはまらない）。**抽出の上限に明示設定は無い**
/// （ADR-0036 決定3。**引ける値はつまみにしない**）ので、引かないと落とし所のままになる。
///
/// # **引き直しの照会先も一緒に返す**（変異検査の結果 / 2026-08-26）
///
/// 起動時に引く相手と、5分ごとに引き直す相手は**同じでなければならない。**
/// 別々に配線できる形にしていたとき、**引き直しだけを本文モデルへ向ける変異が
/// 1554 全緑で通り抜けた**——5分の間隔を待てるテストが無いので、
/// **本番の引き直しがどのモデルを引くかは、スイートのどこからも観測できない。**
///
/// **返り値を1つにして、呼び出し側に選ばせない。**
/// これで、取り違えるには**起動時の照会も一緒に**取り違えることになり、
/// そちらは `the_extraction_limit_follows_the_sub_models_context_through_the_real_bind`
/// が赤にする。
///
/// **⚠ 「型で表現できなくした」とは書けない。** 返り値を捨てて
/// `Some(Arc::clone(&llm))` を書き直す形は**今も書ける。**
/// 消えたのは**1トークンの取り違え**であって、書き換えそのものではない。
///
/// > **⚠ 「観測できない」だった部分は 2026-08-27 に解消した**（レビュー指摘2）。
/// > **書き直した形も、起動時の1行が名乗る**——
/// > [`spawn_refresh_every`] が `main_model` / `sub_model` を同じ行に出し、
/// > `tests/server_context_refresh.rs::the_two_requery_targets_name_their_own_models_on_the_real_bind`
/// > が**同じ行の中で**見る。**5分を待つ必要は無い**
/// > （引く相手は起動時に決まっている事実であって、待って確かめるものではない）。
/// > **⚠ ただし「観測できる」であって「書けない」ではない。**
///
/// # **上流が持っているモデルの一覧も一緒に返す**（作業単位 E-2）
///
/// **同じ照会の本文から読める**ので、往復は増えない。
/// 使うのは `crate::measured::report`——**実測由来の定数が名乗るモデルと、
/// いま上流が応答するモデルがずれていたら報告する。**
/// **`None` は「聞けなかった」**（往復そのものが失敗した）。
///
/// # 上流プロファイルの面に値を書いたら、照会しない（ADR-0054 §決定1 / 作業単位 A-13a-1）
///
/// **`explicit` が `Some` なら [`resolve_budget`] と同じく早期 return する**——
/// 一度も聞かない（一覧も読み手も無い）。**引き直しの照会先は、それでも返す**
/// ——走ったまま値の無いプロファイルへ切り替えたら、そこから引き直す（ADR-0054 §決定3）。
/// **引くかどうかは器の「明示か」が毎回決める**（[`SharedSubContext`] の doc）。
pub(crate) async fn resolve_sub_context(
    upstream: &FaceClient,
    explicit: Option<SubContext>,
) -> (SubContext, Option<FaceClient>, UpstreamProbe) {
    if explicit.is_some() {
        // **鍵を平文の `http://` で送る扱いは載せる**（[`resolve_budget`] の明示の枝と同じ）。
        let probe = UpstreamProbe {
            key_over_plain_http: upstream.current().key_over_plain_http(),
            ..UpstreamProbe::unanswered()
        };
        return (
            choose_sub_context(explicit, None),
            Some(upstream.clone()),
            probe,
        );
    }

    // **同じ1回の往復から3つ読む**（作業単位 E-2 / G-3b / `probe_upstream`）。
    // 起動時の照会をここで増やすと、「照会はモデルごとに1回」を見張っている
    // `tests/server_context_refresh.rs` の主張が変わる。**増やさない。**
    let (probe, failure) = probe_upstream(&upstream.current()).await;
    if let Some(err) = failure {
        // 詳細は `#[source]` 側に残る。**ここで起動を止めない**（`resolve_budget` と同じ）。
        //
        // **往復そのものの失敗と、「答えたが長さが読めない」を同じ文言にまとめてある**
        // ——どちらも調査先は「上流がどのモデルをどう載せているか」で同じ。
        // **空の一覧（`Some(vec![])`）と `None` を区別しない**のも同じ理由で、
        // どちらも `measured::check` が「聞けなかった」に倒す。
        tracing::info!(error = %err, "上流へサブモデルのコンテキスト長を照会できませんでした");
    }

    // **引き直しは必ず行う**（明示設定が無いので止める理由が無い / 上記）。
    // **引いた相手をそのまま渡す**ので、起動時と引き直しで別のモデルにならない。
    (
        choose_sub_context(None, probe.context_tokens),
        Some(upstream.clone()),
        probe,
    )
}

/// 明示・引いた値・落とし所の優先順位。**純粋関数**（通信を持たないので単体で固定できる）。
///
/// **明示（上流プロファイルの面に書いた値 / ADR-0054 §決定1）が最優先**（[`choose_budget`] と同じ形）。
/// 抽出の依頼と応答が同時に収まらない申告値は [`SubContext::new`] が弾き、
/// 落とし所へ落ちる（`choose_budget` の「予算にならない値」と同じ形）。
pub(crate) fn choose_sub_context(explicit: Option<SubContext>, queried: Option<u32>) -> SubContext {
    if let Some(context) = explicit {
        tracing::info!(
            sub_context_length = context.tokens(),
            limit_tokens = context.prompt_limit(),
            "抽出の上限は明示設定のコンテキスト長を使います"
        );
        return context;
    }

    match queried.and_then(SubContext::new) {
        Some(context) => {
            tracing::info!(
                sub_context_length = context.tokens(),
                limit_tokens = context.prompt_limit(),
                "抽出はサブモデルの実コンテキスト長を使います"
            );
            context
        }
        None => {
            let context = SubContext::FALLBACK;
            // **落とし所へ落ちたことは必ず残す**（`choose_budget` と同じ理由）。
            // **文言は「取れなかった」と「取れたが使えなかった」で割る**
            // ——前者は上流の疎通、後者は上流がロードしている設定を調べることになる。
            match queried {
                Some(tokens) => tracing::warn!(
                    queried = tokens,
                    sub_context_length = context.tokens(),
                    limit_tokens = context.prompt_limit(),
                    "上流が申告したサブモデルのコンテキスト長は抽出に使えません。落とし所を使います"
                ),
                None => tracing::warn!(
                    sub_context_length = context.tokens(),
                    limit_tokens = context.prompt_limit(),
                    "サブモデルのコンテキスト長を取得できなかったため落とし所を使います"
                ),
            }
            context
        }
    }
}

/// 優先順位そのもの。**純粋関数**（通信を持たないので単体で固定できる）。
///
/// `queried` は上流が申告したコンテキスト長。予算として成立しない値
/// （生成用の余白以下）はここで弾かれ、既定値へ落ちる。
///
/// # 応答の余白は本文の面の返答の上限（ADR-0059 §決定2 / 作業単位 A-18a）
///
/// **`body` の返答の上限（書いていなければ 1024）を余白にする**（[`budget_for_window`]）。上流から取れるのは
/// コンテキスト全体の長さで、生成用に何を残すかは上流の都合ではない —— **決めるのは本文の面に書いた値**である。
/// **照会した窓が上限以下なら、既定の 1024 に戻して `warn` を1本出す**（止めない）。
/// **窓が分からない（照会に失敗した・照会した窓が予算にならない）ときは、落とし所の窓 8192 で既定の 1024 にする**
/// （面の上限は使わない / [`unknown_window_budget`]）。
/// 明示の予算（`explicit`）は、組んだ時点で面の上限が入っている（`config::ProfileContext::of` が検証済み）。
///
/// **返すのは値と「明示か」・落とし所の窓の印**（[`Chosen`]）—— 器へそのまま渡す。
pub(crate) fn choose_budget(
    explicit: Option<TokenBudget>,
    queried: Option<u32>,
    body: BodyFace,
) -> Chosen<TokenBudget> {
    choose_budget_on(explicit, queried, body, UnknownWindow::Unanswered)
}

/// **起こす上流で起動したときの本文の予算**（A-18a の再レビュー是正）。
///
/// **その面はまだ載っていないので聞いていない**（`App::bind` の `launching_main` の枝 / ADR-0042 §決定3）。
/// 値は照会に失敗したときと同じ（明示があれば明示、無ければ落とし所の窓 8192 で既定の 1024）だが、
/// **「窓が分からない」の行は `info`**（[`UnknownWindow::NotYetQueried`]）—— 切り替えの [`install_face`] と同じ扱い。
/// 載ったら `adopt_face_probe` が窓を採り直す。本当に照会が失敗した起動（外の上流が答えない）は
/// [`choose_budget`] を通り、今どおり `warn`。
///
/// **「取得できなかったため既定値を使います」の行は、ここでも `warn` のまま**（A-18a より前からの振る舞い。触っていない）。
pub(crate) fn budget_before_launch(
    explicit: Option<TokenBudget>,
    body: BodyFace,
) -> Chosen<TokenBudget> {
    choose_budget_on(explicit, None, body, UnknownWindow::NotYetQueried)
}

/// [`choose_budget`] と [`budget_before_launch`] の本体。`occasion` は「窓が分からない」の行の段階だけを決める。
fn choose_budget_on(
    explicit: Option<TokenBudget>,
    queried: Option<u32>,
    body: BodyFace,
    occasion: UnknownWindow,
) -> Chosen<TokenBudget> {
    if let Some(budget) = explicit {
        tracing::info!(
            context_tokens = budget.context_tokens(),
            "コンテキスト長は明示設定を使います"
        );
        return Chosen::explicit(budget);
    }

    let wanted = body.response_room();
    let usable = queried.and_then(|tokens| budget_for_window(tokens, wanted));

    match usable {
        Some(fitted) => {
            let budget = fitted.report(wanted);
            tracing::info!(
                context_tokens = budget.context_tokens(),
                "上流の実コンテキスト長を使います"
            );
            Chosen::measured(budget)
        }
        None => {
            let chosen = unknown_window_budget(body, occasion);
            let budget = chosen.value();
            // **既定値へ落ちたことは必ず残す。** 黙って落ちると、実際の半分しか
            // 使っていない状態がエラーも出さずに続く（ADR-0013）。
            //
            // **文言は「取れなかった」と「取れたが使えなかった」で割る。**
            // 前者は上流の疎通、後者は上流がロードしている設定を調べることになり、
            // 調査先がまるで違う。まとめると必ず片方で空振りする。
            match queried {
                Some(tokens) => tracing::warn!(
                    queried = tokens,
                    context_tokens = budget.context_tokens(),
                    "上流が申告したコンテキスト長は予算になりませんでした。既定値を使います"
                ),
                None => tracing::warn!(
                    context_tokens = budget.context_tokens(),
                    "上流のコンテキスト長を取得できなかったため既定値を使います"
                ),
            }
            chosen
        }
    }
}

/// **窓に、本文の面の返答の上限で予算を組んだ結果**（[`budget_for_window`] / ADR-0059 §決定2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fitted {
    /// 面の上限（書いていなければ既定の 1024）のまま組めた。
    Wanted(TokenBudget),
    /// **窓が面の上限以下だったので、既定の 1024 に戻して組んだ。**
    FellBack(TokenBudget),
}

impl Fitted {
    /// 組んだ予算（ログは出さない）。
    const fn budget(self) -> TokenBudget {
        match self {
            Self::Wanted(budget) | Self::FellBack(budget) => budget,
        }
    }

    /// **既定に戻したなら `warn` を1本出し**（[`RESPONSE_FALLBACK_LINE`]）、予算を返す。
    ///
    /// **黙って戻さない** —— 思考を入れた面では、短い上限で本文が切れうる（ADR-0059 §決定2）。
    /// 戻した原因（窓の大きさ）を、書いた上限と並べて言う。
    fn report(self, wanted: u32) -> TokenBudget {
        match self {
            Self::Wanted(budget) => budget,
            Self::FellBack(budget) => {
                tracing::warn!(
                    context_tokens = budget.context_tokens(),
                    response_tokens_written = wanted,
                    response_tokens = budget.response_tokens(),
                    "{RESPONSE_FALLBACK_LINE}"
                );
                budget
            }
        }
    }
}

/// **返答の上限を既定に戻したと言う行**（[`Fitted::report`]）。テストがこの綴りで引く。
pub(in crate::server) const RESPONSE_FALLBACK_LINE: &str =
    "窓が本文の返答の上限以下なので、返答の上限を既定に戻します";

/// **窓と、使いたい応答の余白から予算を組む。純粋関数**（ADR-0059 §決定2 / 作業単位 A-18a）。
///
/// | 窓 | 返り値 |
/// |---|---|
/// | 既定の 1024 以下 | `None`（予算にならない —— **`wanted` が 1024 未満でも**。呼び出し側が今までどおり扱う） |
/// | 1024 より大きく、`wanted` より大きい | [`Fitted::Wanted`]（面の値のまま） |
/// | 1024 より大きく、`wanted` 以下 | [`Fitted::FellBack`]（既定の 1024 に戻す） |
///
/// **`Some` になるのは、窓が 1024 より大きいときだけ**（`wanted` に依らない）。
///
/// # 下限 1024 を面の上限に追従させない（A-18a のレビュー是正）
///
/// **照会した窓が 1024 以下なら、面の上限が 1024 未満でも採らない。** この下限は「上流が読み込みの途中に答える
/// 小さな窓」を予算にしない備えでもある（単体の `a_queried_value_that_cannot_become_a_budget_is_reported` の
/// 「1024 をロード中」の想定）。面に小さな上限を書いただけでその備えが外れないように、窓の下限は既定の余白で据え置く。
///
/// **照会した窓だけがここへ来る**（起動の [`choose_budget`]・照会し直しの [`adopt_refreshed`]）—— 明示の窓は読み込みで
/// 「上限より大きい」を確かめてあり（`config::ProfileContext::of`）、**窓が分からない（落とし所の窓 8192 を使う）
/// ときは面の上限を使わない**ので、ここを通さず既定の予算にする（[`choose_budget`] / [`install_face`]）。
/// **照会し直すたびに同じ判定をやり直す**ので、窓が戻れば面の値に戻る（[`adopt_refreshed`] が `wanted` を毎回
/// この関数へ渡す）。
pub(crate) fn budget_for_window(window: u32, wanted: u32) -> Option<Fitted> {
    if window <= TokenBudget::DEFAULT_RESPONSE_TOKENS {
        return None;
    }
    if let Ok(budget) = TokenBudget::new(window, wanted) {
        return Some(Fitted::Wanted(budget));
    }
    TokenBudget::new(window, TokenBudget::DEFAULT_RESPONSE_TOKENS)
        .ok()
        .map(Fitted::FellBack)
}

/// **窓が分からない（落とし所の窓 8192 を使う）ときの本文の予算**（A-18a のレビュー是正）。
///
/// **面の返答の上限は使わず、既定の予算（8192・1024）にする。** 照会に失敗して落とし所の窓で組む面で大きな上限を
/// 使うと、プロンプトの予算が縮んで削れない枠が収まらず `BudgetExhausted` になりうる（ADR-0059 §決定2 の
/// 「高い側は遅くなるだけ」は窓が分かっている前提）。**面に上限を書いていれば1行言う**
/// （[`RESPONSE_UNKNOWN_WINDOW_LINE`]。窓の不足の行 [`RESPONSE_FALLBACK_LINE`] とは別の文面）。
/// 窓が分かったら（照会が成功したら）面の値に戻る（[`SharedBudget::adopt_window`]）。
///
/// | `occasion` | 段階 | 理由 |
/// |---|---|---|
/// | [`UnknownWindow::Unanswered`] | `warn` | 照会したが窓が取れなかった（起動 / [`choose_budget`]） |
/// | [`UnknownWindow::NotYetQueried`] | `info` | まだ聞いていない —— 切り替えた直後（[`install_face`]。外の上流はその場で照会し直す）と、起こす上流での起動（[`budget_before_launch`]。載ったら採り直す） |
fn unknown_window_budget(body: BodyFace, occasion: UnknownWindow) -> Chosen<TokenBudget> {
    let budget = TokenBudget::default();
    if body.response_room() != budget.response_tokens() {
        match occasion {
            UnknownWindow::Unanswered => tracing::warn!(
                context_tokens = budget.context_tokens(),
                response_tokens_written = body.response_room(),
                response_tokens = budget.response_tokens(),
                "{RESPONSE_UNKNOWN_WINDOW_LINE}"
            ),
            UnknownWindow::NotYetQueried => tracing::info!(
                context_tokens = budget.context_tokens(),
                response_tokens_written = body.response_room(),
                response_tokens = budget.response_tokens(),
                "{RESPONSE_UNKNOWN_WINDOW_LINE}"
            ),
        }
    }
    Chosen::fallback_window(budget)
}

/// **窓が分からない理由**（[`unknown_window_budget`] の言う段階を決める）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnknownWindow {
    /// 照会したが、予算になる窓が取れなかった。
    Unanswered,
    /// まだ照会していない（切り替えた直後・起こす上流での起動）。
    NotYetQueried,
}

/// **窓が分からないので返答の上限を既定にしたと言う行**（[`unknown_window_budget`]）。テストがこの綴りで引く。
pub(in crate::server) const RESPONSE_UNKNOWN_WINDOW_LINE: &str =
    "上流のコンテキスト長が分からないので、窓が分かるまで返答の上限を既定にします";

/// **本文の予算の明示を決める**（ADR-0054 §決定2・3 / 作業単位 A-13a-1）。
///
/// **上流プロファイルの面に書いた値が `PERSONACORE_CONTEXT_TOKENS` に勝つ。**
/// 面に値が無ければ環境変数（あれば）。**ぶつかったら（両方あって値が違えば）`warn` を1本出す**
/// ——`main.rs` の `warn_on_conflicts` と同じ作法（同じ文面・`setting` / `from_environment` /
/// `from_profile` の欄）。**黙って上書きしない**——現れ方は「設定したはずの窓が効いていない」で、
/// 原因から遠い。
///
/// **起動（`App::bind`）と、走ったままの切り替え（[`install_face`] の本文の面）の両方がここを通る**
/// ——切り替え後にも同じ順位を当てはめる（ADR-0054 §決定3）。
///
/// **`main.rs` の `warn_on_conflicts` には入れていない**——そちらは起動時にしか走らず、
/// 切り替えで同じことを言う場所が要る。**言う場所を2つにしない。**
///
/// # 環境変数の窓にも、本文の面の返答の上限を余白として載せる（ADR-0059 §決定2 / 作業単位 A-18a）
///
/// `environment` は環境変数の窓と既定の余白（1024）で組んだ予算。**面に窓が無く環境変数の窓を使うときだけ、
/// 余白を `body` の上限に組み直す。** 面に窓があれば `profile` をそのまま使い（組んだ時点で面の上限が入っている）、
/// **環境変数の窓では組み直さない** —— 使わない予算を組み直すと、環境変数の窓が上限以下のとき「既定に戻します」と
/// 事実と違う `warn` が出る（A-18a の再レビュー是正）。
/// 面に窓が無く、環境変数の窓では組み直せない（上限が窓以上）組は `config::ProfileContext::of` が読み込み・切り替えの
/// 検証で止めているので、ここでは来ない —— **来たら環境変数の予算のまま使い、`warn` を1本出す**
/// （`unwrap` で「検証済みのはず」に寄りかからない）。
pub(in crate::server) fn explicit_main_budget(
    environment: Option<TokenBudget>,
    profile: Option<TokenBudget>,
    body: BodyFace,
) -> Option<TokenBudget> {
    if let (Some(from_environment), Some(from_profile)) = (environment, profile)
        && from_environment.context_tokens() != from_profile.context_tokens()
    {
        tracing::warn!(
            setting = super::config::ENV_CONTEXT_TOKENS,
            from_environment = from_environment.context_tokens(),
            from_profile = from_profile.context_tokens(),
            "{PROFILE_OVER_ENVIRONMENT}"
        );
    }
    if profile.is_some() {
        return profile;
    }
    environment.map(|from_environment| {
        TokenBudget::new(from_environment.context_tokens(), body.response_room()).unwrap_or_else(
            |_| {
                tracing::warn!(
                    context_tokens = from_environment.context_tokens(),
                    response_tokens_written = body.response_room(),
                    "{RESPONSE_FALLBACK_LINE}"
                );
                from_environment
            },
        )
    })
}

/// **プロファイルが環境変数に勝ったと言う行の本文**（`main.rs` の `warn_on_conflicts` と同じ文面）。
const PROFILE_OVER_ENVIRONMENT: &str = "環境変数よりも選んだ上流プロファイルを優先します";

// ----------------------------------------------------------------------
// 定期的な引き直し（ADR-0020）
//
// **稼働中に上流が小さいコンテキストでモデルを再ロードしても、こちらは気付けない。**
// `usage.prompt_tokens` からは原理的に取れない（ADR-0013 の訂正）。
// **LM Studio では**エラーにもならない。切り捨てられるのはプロンプトの先頭——
// **ペルソナ・性格ステータス・記憶**なので、キャラクターが静かに別人になる（ADR-0013）。
// llama では、小さい窓を超えた本文は 400 `exceed_context_size_error` で断られ、利用者には
// 502 が返る（ADR-0052 M5。断りの本文に窓の大きさは載るが、いまは読んでいない —— 送り直しは
// ADR-0052 §決定8）。ただし、プロンプトが窓に入り、プロンプト ＋ 応答の上限が窓を超えるときは
// 断られず、応答が黙って短くなる（`finish_reason=length`）。
//
// **どちらの上流でも、事象から予算を直す手段が無いので、定期的に引き直す。**
// ----------------------------------------------------------------------

/// 引き直しの間隔。**5分の定数。設定にはしない**（ADR-0020 §2）。
///
/// 引くのは `/api/v0/models` のメタデータ1回で推論を起こさず、ローカルの往復なので
/// 費用は無視できる。再ロードは利用者が LM Studio を触った瞬間に起こるため、
/// 「ずれたまま気づかない窓」は分の単位で足りる。
///
/// **設定可能にしない。** 起動時に決めるものを増やすと、本番の入口を通るテストを
/// 足さない限り配線が観測対象から外れる（ADR-0019 と同じ判断）。
pub(crate) const REQUERY_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// **稼働中に差し替えられるトークン予算。**
///
/// ハンドラは毎回ここを読む（起動時の値を写し取らない）。書けるのはこのモジュールだけで、
/// 唯一の書き手は定期的な引き直し（[`refresh_once`]）。
///
/// # `RwLock` を選んだ理由
///
/// **新しいクレートを足さない**（`arc_swap` 等）。読みは短く、書きは 5 分に高々1回なので、
/// 素の `RwLock` で競合は起きない。`await` を跨いで保持しないこと
/// （[`get`](Self::get) / [`store`](Self::store) はどちらも即座に手放す）。
///
/// # 値と一緒に「明示か」を持つ（ADR-0054 §決定3 / 作業単位 A-13a-1）
///
/// **明示の値（`PERSONACORE_CONTEXT_TOKENS` か、上流プロファイルの面に書いた値）は、
/// 照会の答えで上書きしない**（ADR-0020 §4）。F-4b-2 までは「明示か」を起動時に決めて
/// 引き直しのタスクを丸ごと止めていたが、**走ったまま切り替えると明示の有無が変わる**ので、
/// **「いまの明示」を器が持ち、書き手が毎回読む。**
///
/// **値と印を同じ錠の中に置く**——別々の器にすると、切り替えが「明示の値と印」を書く途中に
/// 引き直しの採用が挟まり、**明示の値を照会の答えで上書きする**窓ができる。
/// **照会の答えを採る口（[`Self::adopt_window`]）は、印を同じ錠の中で見て断る。**
///
/// # 本文の面の値も同じ錠の中に置く（ADR-0059 §決定1・2 / 作業単位 A-18a）
///
/// **本文の面に書いた思考の量と返答の上限（[`BodyFace`]）も、この器が持つ。**
///
/// * **返答の上限は、照会し直すたびに要る** —— 照会した窓が上限以下なら予算は既定の 1024 に戻る（`warn`）が、
///   窓が戻れば面の値に戻す（ADR-0059 §決定2）。いまの予算の余白（戻した後の 1024）からは面の値が読めないので、
///   書いた値を器に残す。
/// * **思考の量は、チャットの入口が予算と同じ1回の読みで取る**（[`Self::turn`]）——切り替えの途中で
///   「新しい予算と古い思考の量」を組まない。
/// * **裏方はこの器を読まない**（抽出の上限は [`SharedSubContext`]）ので、`sub` 面の無い構成でも本文の値が
///   裏方へ写らない（ADR-0059 §却下した選択肢）。
#[derive(Debug, Clone)]
pub struct SharedBudget(Arc<RwLock<MainHeld>>);

/// **本文の器の中身**（予算と「明示か」と、本文の面の値 / [`SharedBudget`] の doc）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MainHeld {
    budget: Held<TokenBudget>,
    body: BodyFace,
}

/// **器の中身**（値と、それが明示かどうか）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Held<T> {
    value: T,
    explicit: bool,
    /// **窓が分からず、落とし所の窓で組んだ値か**（ADR-0059 §決定2 / A-18a のレビュー是正）。
    ///
    /// **本文の器だけが読む**（送る値のログが、返答の上限を既定に戻した理由を「窓の不足」と「窓が分からない」で
    /// 言い分ける / [`report_samplers`]）。抽出の器では常に `false`。照会で予算になる窓が分かったら外れる
    /// （[`SharedBudget::adopt_window`]）。
    fallback_window: bool,
}

/// **起動時・切り替え時に決めた値と、それが明示かどうか**（作業単位 A-13a-1）。
///
/// **器（[`SharedBudget`] / [`SharedSubContext`]）を作る・差し替えるときにだけ使う。**
/// `bool` を別の引数で並べない——並べると、値と印を別々の場所から取り違えて渡せる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::server) struct Chosen<T>(Held<T>);

impl<T> Chosen<T> {
    /// **明示の値**（照会の答えで上書きしない）。
    pub(in crate::server) const fn explicit(value: T) -> Self {
        Self(Held {
            value,
            explicit: true,
            fallback_window: false,
        })
    }

    /// **照会した値か、落とし所**（照会の答えで差し替わる）。
    pub(in crate::server) const fn measured(value: T) -> Self {
        Self(Held {
            value,
            explicit: false,
            fallback_window: false,
        })
    }

    /// **窓が分からず、落とし所の窓で組んだ本文の予算**（照会の答えで差し替わる / [`Held::fallback_window`]）。
    ///
    /// **作るのは [`choose_budget`] と [`install_face`] の窓が分からない枝だけ。**
    pub(in crate::server) const fn fallback_window(value: T) -> Self {
        Self(Held {
            value,
            explicit: false,
            fallback_window: true,
        })
    }
}

impl<T: Copy> Chosen<T> {
    /// 値（起動ログに出す）。
    pub(in crate::server) const fn value(self) -> T {
        self.0.value
    }

    /// **窓が分からず、落とし所の窓で組んだ値か**（[`Held::fallback_window`]）。
    pub(in crate::server) const fn on_fallback_window(self) -> bool {
        self.0.fallback_window
    }
}

impl SharedBudget {
    /// 起動時に決まった値で作る。**このモジュールの外からは作れない。**
    ///
    /// 外に出すと `server::mod` から**2つ目の器**を作れてしまい、
    /// 「引き直しが書く先」と「ハンドラが読む先」を別々にする配線が書けるようになる。
    /// その配線は**引き直しの WARN を正しく出したまま、予算だけが古いまま**という
    /// 壊れ方をする（ADR-0020 が塞ぐと言っている失敗そのもの）。
    /// 唯一の入口は [`PendingRefresh::new`] で、器は [`PendingRefresh::spawn`] が
    /// **引き直しのタスクと対で**返す。
    ///
    /// **private 化が効くのはモジュールの外に対してだけ。** この中では現に呼べるので、
    /// 同じ取り違えが `context` の中では書ける。そちらを止めているのは型ではなく
    /// 見張り番（[`PendingRefresh`] の doc の表）。
    ///
    /// **照会した値（明示ではない）として作る**（明示の器は [`Self::chosen`]）。
    #[cfg(test)]
    fn new(budget: TokenBudget) -> Self {
        Self::chosen(Chosen::measured(budget))
    }

    /// **決めた値と「明示か」で作る**（作業単位 A-13a-1）。外から作れないのは [`Self::new`] と同じ。
    ///
    /// **本文の面の値は「何も書いていない」で始める**（起動で書いた値は [`PendingRefresh::with_body`] が入れる）。
    fn chosen(chosen: Chosen<TokenBudget>) -> Self {
        Self(Arc::new(RwLock::new(MainHeld {
            budget: chosen.0,
            body: BodyFace::default(),
        })))
    }

    fn read(&self) -> MainHeld {
        *self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn held(&self) -> Held<TokenBudget> {
        self.read().budget
    }

    /// **本文の面に書いた値**（思考の量と返答の上限 / ADR-0059 / 作業単位 A-18a）。
    ///
    /// **予算の余白とは限らない** —— 照会した窓が上限以下なら、予算の余白は既定の 1024（[`budget_for_window`]）。
    pub(in crate::server) fn body(&self) -> BodyFace {
        self.read().body
    }

    /// **1ターンぶんの予算と思考の量を、1回の読みで取る**（`super::chat` の入口 / ADR-0059 §決定1）。
    ///
    /// 別々に読むと、間に切り替えが挟まった回だけ「切り替え先の予算と切り替え前の思考の量」で組む。
    pub(in crate::server) fn turn(&self) -> (TokenBudget, Option<ReasoningEffort>) {
        let held = self.read();
        (held.budget.value, held.body.reasoning_effort)
    }

    /// **起動で決めた本文の面の値を入れる**（[`PendingRefresh::with_body`] だけが呼ぶ。予算は触らない）。
    fn set_body(&self, body: BodyFace) {
        self.0.write().unwrap_or_else(PoisonError::into_inner).body = body;
    }

    /// 現在の予算。**`TokenBudget` は `Copy` なので錠はその場で手放す。**
    ///
    /// 毒された錠でも中身は読む（`unwrap` / `expect` を置かないための
    /// `PoisonError::into_inner`）。ここで panic すると、
    /// **無関係な panic 1件でチャットが全滅する。**
    pub(crate) fn get(&self) -> TokenBudget {
        self.held().value
    }

    /// **いまの値が明示か**（作業単位 A-13a-1）。明示なら照会しない・採らない。
    fn is_explicit(&self) -> bool {
        self.held().explicit
    }

    /// **明示ならその値（トークン数）、そうでなければ `None`**（`super::switch` が新旧を比べる）。
    pub(in crate::server) fn explicit_tokens(&self) -> Option<u32> {
        let held = self.held();
        held.explicit.then(|| held.value.context_tokens())
    }

    /// **照会の答え（窓）を採る。明示なら書かない**（同じ錠の中で見る / この型の doc）。予算を書いたら `true`。
    ///
    /// **いまの予算と本文の面の上限も同じ錠の中で読む**（[`adopt_refreshed`] へ渡す）—— 照会の最中に切り替えが
    /// 挟まっても、切り替え先の面の上限で組む。
    ///
    /// **予算になる窓（1024 より大きい / [`budget_for_window`]）が分かったら、落とし所の窓の印を外す** ——
    /// 予算が同じまま（採らない）でも外す（面の上限が窓以上で、落とし所と同じ既定の予算に戻る回）。
    fn adopt_window(&self, queried: Option<u32>) -> bool {
        let mut held = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if held.budget.explicit {
            return false;
        }
        let wanted = held.body.response_room();
        let next = adopt_refreshed(held.budget.value, queried, wanted);
        if queried.is_some_and(|tokens| budget_for_window(tokens, wanted).is_some()) {
            held.budget.fallback_window = false;
        }
        match next {
            Some(budget) => {
                held.budget.value = budget;
                true
            }
            None => false,
        }
    }

    /// **いまの予算と「明示か」・落とし所の窓の印**（切り替えの後に送る値を言う / [`report_samplers`]）。
    pub(in crate::server) fn current(&self) -> Chosen<TokenBudget> {
        Chosen(self.held())
    }

    /// **値と「明示か」と本文の面の値を丸ごと差し替える**（切り替え / [`install_face`]）。
    ///
    /// **本文の面の値も同じ錠の中で書く**（ADR-0059 §決定1・2）—— 切り替え先の予算と切り替え前の思考の量が
    /// 1ターンに混ざらない（[`Self::turn`]）。
    fn install(&self, chosen: Chosen<TokenBudget>, body: BodyFace) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = MainHeld {
            budget: chosen.0,
            body,
        };
    }

    /// **テスト専用。** 引き直しが値を差し替えたのと同じ効果を外から起こす。
    ///
    /// 5分を実時間で待たずに「走っているサーバのハンドラが**現在値**を読むか」を
    /// 確かめるための口（`tests/server_chat.rs` の
    /// `the_running_budget_is_shared_with_the_handlers`）。
    /// **本番から予算を書き換える手段は生やさない**ので `Store::pool` と同じくゲートする。
    ///
    /// **「明示か」は触らない**（値だけを差し替える）。
    #[cfg(any(test, feature = "testing"))]
    pub fn replace_for_tests(&self, budget: TokenBudget) {
        self.0
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .budget
            .value = budget;
    }

    /// **テスト専用。** いまの予算のコンテキスト長（`tests/server_profile_context.rs`）。
    ///
    /// **[`Self::get`] は `pub(crate)`** なので、統合テストはここから読む。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn tokens_for_tests(&self) -> u32 {
        self.get().context_tokens()
    }

    /// **テスト専用。** いまの予算の応答の余白（`tests/server_profile_thinking.rs` / ADR-0059 §決定2）。
    ///
    /// 本文の要求の `max_tokens` の天井と同じ値のはずだが、**予算そのものに効いたか**はこちらで見る
    /// （`max_tokens` だけを面の値にして予算を 1024 のままにする実装を赤にする）。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn response_tokens_for_tests(&self) -> u32 {
        self.get().response_tokens()
    }
}

/// **稼働中に差し替えられるサブモデルのコンテキスト長**（作業単位 B-7）。
///
/// [`SharedBudget`] と**同じ形**（読みは短く、書きは間隔ごとに高々1回）。
/// 抽出器は往復ごとにここを読む（起動時の値を写し取らない）。
///
/// # なぜ [`SharedBudget`] と別の型なのか
///
/// 中身が別物（本文の予算 / サブモデルの窓）で、**取り違えると
/// 「引き直しは正しく WARN を出しているのに、片方だけが古いまま」**になる。
/// 1つの型に畳むと、`AppState` と `MemoryExtractor` へ**同じ器を渡す**配線が
/// 型として書けてしまう。
///
/// **書けるのはこのモジュールだけ**——照会の答えを採るのは [`refresh_sub_once`] と
/// [`adopt_face_probe`]、丸ごと差し替えるのは [`install_face`]（切り替え）。
///
/// **値と一緒に「明示か」を持つ**（[`SharedBudget`] の doc と同じ理由 / ADR-0054 §決定1・3）。
/// **抽出の上限の明示は、上流プロファイルの面に書いた値だけ**（環境変数は無い）。
///
/// **`pub` なのは統合テストが値を読むため**（`App::sub_context_handle` と対）。
/// **型そのものが `testing` の下でしか再公開されていない**ので、本番のビルドでは crate の外から見えない
/// （[`SharedUpstream`] と同じ形）。
#[derive(Debug, Clone)]
pub struct SharedSubContext(Arc<RwLock<Held<SubContext>>>);

impl SharedSubContext {
    /// 起動時に決まった値で作る。**このモジュールの外からは作れない。**
    ///
    /// 外に出すと `server::mod` から**2つ目の器**を作れてしまい、
    /// 「引き直しが書く先」と「抽出器が読む先」を別々にする配線が書ける
    /// （[`SharedBudget::chosen`] と同じ理屈）。唯一の入口は [`PendingRefresh::new`] で、
    /// 器は [`PendingRefresh::sub_context`] が渡す。
    ///
    /// **照会した値（明示ではない）として作る**（明示の器は [`Self::chosen`]）。
    #[cfg(test)]
    fn new(context: SubContext) -> Self {
        Self::chosen(Chosen::measured(context))
    }

    /// **決めた値と「明示か」で作る**（作業単位 A-13a-1）。
    fn chosen(chosen: Chosen<SubContext>) -> Self {
        Self(Arc::new(RwLock::new(chosen.0)))
    }

    fn held(&self) -> Held<SubContext> {
        *self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// 現在のコンテキスト長。**`SubContext` は `Copy` なので錠はその場で手放す。**
    ///
    /// 毒された錠でも中身は読む（`unwrap` / `expect` を置かないための
    /// `PoisonError::into_inner`）。ここで panic すると、
    /// **無関係な panic 1件で裏方が全滅する。**
    pub(in crate::server) fn get(&self) -> SubContext {
        self.held().value
    }

    /// **いまの値が明示か**（[`SharedBudget::is_explicit`] と同じ）。
    fn is_explicit(&self) -> bool {
        self.held().explicit
    }

    /// **明示ならその値（トークン数）、そうでなければ `None`**（`super::switch` が新旧を比べる）。
    pub(in crate::server) fn explicit_tokens(&self) -> Option<u32> {
        let held = self.held();
        held.explicit.then(|| held.value.tokens())
    }

    /// **照会の答えを採る。明示なら書かない**（[`SharedBudget::adopt_window`] と同じ）。書いたら `true`。
    fn adopt(&self, context: SubContext) -> bool {
        let mut held = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if held.explicit {
            return false;
        }
        held.value = context;
        true
    }

    /// **値と「明示か」を丸ごと差し替える**（切り替え / [`install_face`]）。
    fn install(&self, chosen: Chosen<SubContext>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = chosen.0;
    }

    /// **テスト専用。** いまの抽出の窓（トークン数 / `tests/server_profile_context.rs`）。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn tokens_for_tests(&self) -> u32 {
        self.get().tokens()
    }

    /// **落とし所を入れた器。テスト専用。**
    ///
    /// 引き直しを持たない配線（`server::extract` の単体テスト・`super::drift` の
    /// 単体テスト）が抽出器を組み立てるための口。
    /// **本番からは見えない**ので、`App::bind_with` がここを呼ぶと
    /// 素の `cargo build` が落ちる（CLAUDE.md 2節）。
    /// **`#[cfg(test)]` だけ**——`testing` フィーチャの下に置くと、
    /// 使うのが単体テストしかないので**素の `cargo build`（`testing` 付き）で
    /// 未使用の警告**になる（統合テストからは使わない）。
    #[cfg(test)]
    pub(in crate::server) fn fallback_for_tests() -> Self {
        Self::for_tests(SubContext::FALLBACK)
    }

    /// **任意の値を入れた器。テスト専用。**
    ///
    /// 落とし所と**別の値**を入れられることが要点——落とし所しか作れないと、
    /// 「器を読まずに落とし所を使う実装」と区別が付かない。
    #[cfg(test)]
    pub(in crate::server) fn for_tests(context: SubContext) -> Self {
        Self::new(context)
    }

    /// **テスト専用。** 引き直しが差し替えたのと同じ効果を外から起こす。
    ///
    /// 5分を実時間で待たずに「抽出器が**現在値**を読むか」を確かめるための口
    /// （[`SharedBudget::replace_for_tests`] と同じ形）。
    #[cfg(test)]
    pub(in crate::server) fn replace_for_tests(&self, context: SubContext) {
        self.0.write().unwrap_or_else(PoisonError::into_inner).value = context;
    }
}

/// **稼働中に書き替えられる上流の観測**（作業単位 G-3b）。
///
/// [`SharedBudget`] / [`SharedSubContext`] と**同じ形**
/// （読みは短く、書きは間隔ごとに面あたり高々1回）。
/// ハンドラは毎回ここを読む（起動時の値を写し取らない）。
///
/// # なぜ3つ目の器なのか
///
/// 中身が別物である。**予算と窓は「いま使っている値」**で、
/// **こちらは「上流について観測できたこと」**——
/// **引き直しが失敗した回、前者は現在値を保ち、後者は「答えなし」になる。**
/// **その食い違いこそが読みたいもの**なので、1つの型に畳めない。
///
/// **書けるのはこのモジュールだけで、唯一の書き手は
/// [`refresh_once`] と [`refresh_sub_once`]。**
#[derive(Debug, Clone)]
pub struct SharedUpstream(Arc<RwLock<HeldFacts>>);

/// **観測と、面ごとの版**（作業単位 F-4b-2）。
///
/// # なぜ版を持つのか（**古い事実を残さない** / ADR-0042 §決定4）
///
/// **引き直し（5分ごと）は、照会を錠の外で済ませてから書く。**
/// **その往復の最中に面が差し替わると、前の上流の答えが新しい宛先の名前で書かれる**
/// ——画面は「新しい上流が前のモデルの窓を名乗った」と言う。
/// **差し替えは版を1つ進め、引き直しは照会の前に読んだ版と違えば書かない**
/// （[`SharedUpstream::store_face_since`]）。
#[derive(Debug)]
struct HeldFacts {
    facts: UpstreamFacts,
    /// `[本文, 抽出]` の版。**差し替え（[`SharedUpstream::replace_face`]）だけが進める。**
    epochs: [u64; 2],
}

const fn epoch_slot(face: ModelFace) -> usize {
    match face {
        ModelFace::Main => 0,
        ModelFace::Sub => 1,
    }
}

impl SharedUpstream {
    /// 起動時に決まった観測で作る。**このモジュールの外からは作れない。**
    ///
    /// 外に出すと `server::mod` から**2つ目の器**を作れてしまい、
    /// 「引き直しが書く先」と「ハンドラが読む先」を別々にする配線が書ける
    /// （[`SharedBudget::chosen`] と同じ理屈）。唯一の入口は [`PendingRefresh::new`] で、
    /// 器は [`PendingRefresh::upstream`] が渡す。
    fn new(facts: UpstreamFacts) -> Self {
        Self(Arc::new(RwLock::new(HeldFacts {
            facts,
            epochs: [0; 2],
        })))
    }

    /// 現在の観測。**`UpstreamFacts` は `Clone` なので錠はその場で手放す。**
    ///
    /// 毒された錠でも中身は読む（`unwrap` / `expect` を置かないための
    /// `PoisonError::into_inner`）。ここで panic すると、
    /// **無関係な panic 1件で管理 API が全滅する。**
    ///
    /// **本番の読み手は `AppState::upstream`**（`GET /admin/upstream` / 作業単位 G-10）。
    /// **`allow(dead_code)` はその読み手が来たので外してある。**
    ///
    /// **`pub` なのは統合テストがここから読むため**（`App::upstream_handle` と対）。
    /// **型そのものが `testing` の下でしか再公開されていない**ので、
    /// 本番のビルドでは crate の外から見えない。**読み出しだけなので書き換えの口にならない。**
    pub fn get(&self) -> UpstreamFacts {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .facts
            .clone()
    }

    /// **その面だけを差し替える。** 反対の面は触らない。
    ///
    /// **`await` を跨いで錠を保持しない**（照会は錠の外で済ませてから呼ぶこと）。
    fn store_face(&self, face: ModelFace, probe: &UpstreamProbe) {
        self.0
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .facts
            .refresh(face, probe);
    }

    /// **その面の版**（照会の前に読む / [`HeldFacts`] の doc）。
    fn epoch(&self, face: ModelFace) -> u64 {
        self.0.read().unwrap_or_else(PoisonError::into_inner).epochs[epoch_slot(face)]
    }

    /// **その面の世代**（[`FaceGeneration`] / 作業単位 B-60b）。版と答えた名前を**同じ錠の中で**読む。
    ///
    /// **⚠ 写し（`FaceClient::current`）より先に読むこと**（[`refresh_once`] が版を先に読むのと同じ理由）。
    /// 差し替え（[`install_face`]）は「クライアント → 版」の順に書くので、
    /// 「世代 → 写し」の順に読めば、**新しい写しの答えを古い世代の名で覚える**ことはあっても
    /// （その記憶は次のターンには古くて使われない）、**古い写しの答えを新しい世代の名で覚える**ことは無い。
    /// **逆に読むと、前の上流が「数えられない」と言っただけで、新しい上流をその世代の間ずっと数えなくなる。**
    /// **この順に見張り番は無い**（2つの読みの間に `await` が無く、テストが差し替えを挟めない ——
    /// [`install_face`] の doc の「守れないもの」と同じ形）。
    pub(in crate::server) fn generation(&self, face: ModelFace) -> FaceGeneration {
        let held = self.0.read().unwrap_or_else(PoisonError::into_inner);
        FaceGeneration {
            epoch: held.epochs[epoch_slot(face)],
            answered_name: held.facts.face(face).answered_name().map(str::to_owned),
        }
    }

    /// **版が `epoch` のままなら、観測を書き、続けて `adopt` を走らせる。** 書いたら `true`。
    ///
    /// **照会の最中に面が差し替わっていたら、その答えは前の上流のもの**なので捨てる。
    ///
    /// # `adopt` は同じ錠の中で走る（レビュー1周目の是正 / 作業単位 F-4b-2）
    ///
    /// **引き直しは、観測を書いた後に予算（抽出なら上限）を採る。**
    /// **2つを別々に書くと、その間に差し替え（[`install_face`]）が挟まりうる**——
    /// 差し替えは予算を落とし所へ戻すが、**その直後に引き直しが前の上流の窓で書き戻す。**
    /// **版の検査・観測・採用を1つの write 錠の中に置く**ので、差し替えは
    /// 「全部の前」か「全部の後」にしか入れない（前なら版が違って捨てる。後なら落とし所で上書きする）。
    ///
    /// **錠の順は「観測 → 予算」の一方向だけ**（[`install_face`] は2つを入れ子にせず順に取る）。
    /// **`adopt` の中で `await` はできない**（同期の錠なので、書けば型が止める）。
    ///
    /// **⚠ 見張り番は半分だけ**（`tests::a_switch_after_the_observation_leaves_the_fallback_budget`
    /// と抽出の対）。**採用を錠の外へ出して間に `await` を挟む形は赤になるが、
    /// 外へ出すだけで間に待ち点が無い形は緑を通る**（測った / 作業単位 F-4b-2 の報告）。
    /// 後者の窓は数命令ぶんだが、**捕まえる手段はある —— 置いていないだけ**（レビュアーが実測: 予算の錠を読み手として
    /// 掴んだまま観測の `try_read` が `WouldBlock` であり続けることを見る形で、正しいコードで 5/5 緑・この変異で赤。
    /// 負荷をかけた反復は未測なので入れていない。案は Issue origin#2 のコメント）。**いま守っているのはこの doc である。**
    fn store_face_since(
        &self,
        face: ModelFace,
        epoch: u64,
        probe: &UpstreamProbe,
        adopt: impl FnOnce(),
    ) -> bool {
        let mut held = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if held.epochs[epoch_slot(face)] != epoch {
            return false;
        }
        held.facts.refresh(face, probe);
        adopt();
        true
    }

    /// **その面の観測を丸ごと置き換え、版を進める**（作業単位 F-4b-2 / ADR-0042 §決定4）。
    ///
    /// **宛先と綴りが変わる**ので [`UpstreamFacts::refresh`]（宛先と綴りを写す）は使えない。
    /// **版を進める**ので、差し替えより前に始まった照会の答えは書かれない。
    fn replace_face(&self, face: ModelFace, observation: UpstreamObservation) {
        let mut held = self.0.write().unwrap_or_else(PoisonError::into_inner);
        held.facts.replace(face, observation);
        held.epochs[epoch_slot(face)] = held.epochs[epoch_slot(face)].wrapping_add(1);
    }

    /// **テスト専用。** 引き直しがその面を差し替えたのと同じ効果を外から起こす
    /// （[`SharedBudget::replace_for_tests`] と同じ形 / 作業単位 G-10）。
    ///
    /// 5分を実時間で待たずに「走っているサーバのハンドラが**現在値**を読むか」を
    /// 確かめるための口（`tests/server_admin_upstream.rs` の
    /// `the_response_follows_the_refreshed_observation`）。
    ///
    /// **[`UpstreamProbe`] をそのまま受けない。** あれは `pub(crate)` なので、
    /// 公開の関数の引数に置くと本番の面に上流の観測を組み立てる型が出る。
    /// **欄を並べて受け、ここで組む**——組み立てるのは
    /// [`refresh_once`] / [`refresh_sub_once`] と同じ [`UpstreamProbe`] である。
    ///
    /// **本番から観測を書き換える手段は生やさない**ので `Store::pool` と同じくゲートする。
    #[cfg(any(test, feature = "testing"))]
    pub fn refresh_for_tests(
        &self,
        face: ModelFace,
        context_tokens: Option<u32>,
        answered_by: Option<crate::llm::AnsweredBy>,
        catalogue: Option<Vec<String>>,
        answered_name: Option<String>,
    ) {
        // **照会した回と同じく、確かめた時刻と結果を載せる**（作業単位 B-66）——
        // 答えた読み手があれば「答えた」、無ければ「答えない」。
        let outcome = if answered_by.is_some() {
            super::upstream::CheckOutcome::Answered
        } else {
            super::upstream::CheckOutcome::NoAnswer
        };
        self.store_face(
            face,
            &UpstreamProbe {
                catalogue,
                context_tokens,
                answered_by,
                answered_name,
                unauthorized: false,
                key_over_plain_http: None,
                check: Some(super::upstream::UpstreamCheck::now(outcome)),
            },
        );
    }
}

/// **面の世代**（ADR-0052 §推奨4 / 作業単位 B-60b）。
///
/// 「数えられない上流だ」と覚えた記憶（`server::seal`）を**いつ捨てるか**の鍵。
/// ADR-0052 §推奨4 が挙げる3つの合図を、次の2つの値で持つ:
///
/// | 合図 | 何が動くか |
/// |---|---|
/// | 起動 | 記憶そのものがプロセスの中にしか無い（DB に持たない / §推奨6） |
/// | **プロファイルの差し替え** | **版**（[`SharedUpstream::replace_face`] だけが進める） |
/// | **引き直しで答えたモデルが変わる** | **答えた名前**（引き直しが観測へ書く / ADR-0020） |
///
/// **答えた名前は「変わった」ことだけを見る。** 引き直しが失敗した回は `None` になるので、
/// その前後で世代が2回変わり、**もう一度聞きに行く**（往復が1つ増えるだけ。覚えたまま外すより安い）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::server) struct FaceGeneration {
    epoch: u64,
    answered_name: Option<String>,
}

impl FaceGeneration {
    /// **同じ上流の同じモデルのままと見てよいか**（`server::seal` の掛け金の外れ方 / 作業単位 B-60d-2）。
    ///
    /// | 変化 | 結果 |
    /// |---|---|
    /// | 版が違う（プロファイルの差し替え） | `false` |
    /// | 答えた名前が**別の `Some`** に変わった | `false` |
    /// | 答えた名前の片方が `None`（引き直しの失敗） | **`true`**（名前が分からないだけで、変わったとは言えない） |
    ///
    /// **`==` より緩い。** `==` で外すと、引き直しが1回失敗するたびに（`(e, "m")` → `(e, None)` → `(e, "m")`）
    /// 掛け金が外れ、食い違いが分かっている数え方で数えた計画をまた送る。
    pub(in crate::server) fn same_model_as(&self, other: &Self) -> bool {
        self.epoch == other.epoch
            && match (&self.answered_name, &other.answered_name) {
                (Some(mine), Some(theirs)) => mine == theirs,
                (None, _) | (_, None) => true,
            }
    }

    /// **テスト専用。** 値を直に作る（`server::seal` の単体テストが世代を区別するため）。
    #[cfg(test)]
    pub(in crate::server) fn for_tests(epoch: u64, answered_name: Option<&str>) -> Self {
        Self {
            epoch,
            answered_name: answered_name.map(str::to_owned),
        }
    }
}

/// 走っている引き直しの背景タスク。**落とすと止まる。**
///
/// `App::serve` がこれを持ち、停止時に落とす。**タスクを leak させない**
/// （ADR-0020 §3 の3つめ）。
pub(crate) struct RefreshTask(Option<JoinHandle<()>>);

impl RefreshTask {
    /// 引き直さない場合（**照会先が2つとも無いとき**）。
    const fn idle() -> Self {
        Self(None)
    }
}

impl Drop for RefreshTask {
    /// **サーバが止まるときに残さない。**
    fn drop(&mut self) {
        if let Some(handle) = self.0.as_ref() {
            handle.abort();
        }
    }
}

// > **⚠ ここにあった `refresh_source`（明示設定なら照会先を `None` にする）は、作業単位 A-13a-1 で外した。**
// > **走ったまま値の無いプロファイルへ切り替えたら、そこから引き直す**（ADR-0054 §決定3）ので、
// > **照会先を起動時に捨てると戻れない。** 明示の間は引かない（ADR-0020 §4）は、
// > **器の「明示か」を毎回読む形**になった（[`refresh_once`] / [`SharedBudget`] の doc）。

/// **まだ走り出していない引き直しと、その書き先の器。**
///
/// `App` は起動（`bind`）から応答開始（`serve`）までこれを持ち、`serve` で
/// [`spawn`](Self::spawn) する。**器はここで一度だけ作られ、後から差し替えられない。**
///
/// # 「書く先」と「読む先」の取り違えを、どこまで止められているか
///
/// 取り違えた配線は **「上流のコンテキスト長が変わりました」の WARN を正しく出しながら、
/// ハンドラは古い予算で組み続ける**——ADR-0020 が塞ぐはずの失敗が
/// 「仕掛けは動いている」というログ付きで起きる。
///
/// **止め方は3段あり、止まっていない形が1つ残っている。**
/// 下は**すべて実際に書いて測った結果**（11c-4 / 2026-08-20。
/// スイートは変異なしで 875 passed / 0 failed）。
///
/// | 書いてみた形 | 結果 |
/// |---|---|
/// | `serve` で `SharedBudget::new` を直に呼ぶ | **コンパイル不可**（`E0624` / private） |
/// | `serve` で [`PendingRefresh::new`] を**もう一度**呼び、その器を `AppState` へ | 通るが**赤**（`the_running_budget_is_shared_with_the_handlers` / 502 のはずが 422） |
/// | [`spawn_every`](Self::spawn_every) の中で捨て器を渡す | 通るが**赤**（`the_spawned_task_writes_into_the_returned_budget` / 4096 にならず 16384 のまま） |
/// | [`spawn`](Self::spawn) が `spawn_every` を経由せず、自分で `spawn_refresh_every` を呼ぶ | **通って緑**（875 passed / 0 failed） |
///
/// **最後の1行が残っている穴。** `spawn` は [`REQUERY_INTERVAL`]（5分）を渡すので、
/// **テストが実時間で通れない。** だから `spawn` の本体は `spawn_every` への委譲1行に
/// 留めてあり、**器の対応付けをここに書き足さないこと。**
/// 書き足した瞬間、その行を赤にできる見張り番はスイートに存在しなくなる。
///
/// **「型で表現できなくした」とは書けない。** 型が止めているのは1行目だけで、
/// 2行目と3行目は見張り番が、4行目は**何も**止めていない。
///
/// > **作業単位 A-13a-1 で、本番の器の作り方は [`SharedBudget::chosen`]（値と「明示か」を受ける）に替わり、
/// > `SharedBudget::new` は単体テスト専用（`#[cfg(test)]`）になった。** 表の1行目に当たる形は、
/// > いまは `SharedBudget::chosen` を直に呼ぶ形で、**同じく private**（`E0624`。A-13a-1 では測り直していない）。
pub(crate) struct PendingRefresh {
    /// **本文の予算**の照会先。**`None` なら引き直さない**（照会先を持たない注入の入口）。
    ///
    /// **明示の間は持っていても引かない**（ADR-0020 §4）——**引くかどうかは器の
    /// 「明示か」を毎回読んで決める**（作業単位 A-13a-1 / [`refresh_once`]）。
    ///
    /// **差し替え口ごと持つ**（作業単位 F-4b-2）——走ったまま上流を差し替えたら、
    /// **引き直しも新しい上流へ聞く。** 起動時のクライアントを握ると、
    /// 差し替えの後も5分ごとに前の上流へ聞き続ける。
    source: Option<FaceClient>,
    /// 書き先の器。**ハンドラが読むのもこれ**（`spawn` が同じものを返す）。
    budget: SharedBudget,
    /// **サブモデル**の照会先（作業単位 B-7）。**`None` なら引き直さない。**
    ///
    /// **`source` と別のクライアント**（モデルだけを差し替えた複製）。
    /// 同じものを入れると、抽出の上限が本文モデルの窓から決まる。
    sub_source: Option<FaceClient>,
    /// **サブモデルのコンテキスト長の器。** 抽出器が読むのもこれ
    /// （[`sub_context`](Self::sub_context) が同じものを渡す）。
    sub_context: SharedSubContext,
    /// **上流の観測の器**（作業単位 G-3b）。ハンドラが読むのもこれ
    /// （[`upstream`](Self::upstream) が同じものを渡す）。
    upstream: SharedUpstream,
    /// **照会の回の門**（作業単位 B-66 / [`UpstreamChecker`]）。**背景タスクと「いま確かめる」が同じ1つを通る。**
    gate: Arc<CheckGate>,
    /// **背景タスクの最初の1回で照会する面**（作業単位 B-66 / [`CheckAtStart`]）。
    check_at_start: CheckAtStart,
}

/// **起動の直後に照会する面**（作業単位 B-66 / Issue origin#94 の決定1）。
///
/// **`App::bind` の中で照会しなかった面のうち、照会してよい面。** いまは
/// **コンテキスト長を書いた面（明示）で、自分で起こす面ではないもの**がここに入る
/// ——書いていない面は `bind` の中で予算を決めるために照会済み、起こす面は載るのを待つ番人が聞く
/// （`server::App::bind_launching`）。
///
/// **`bind` の中で待たない。** 背景タスクの最初の1回として `serve` の直後に走らせるので、
/// **上流が黙っていても起動は今と同じ速さで返る**（見張り番は
/// `tests/server_upstream_check.rs::a_silent_upstream_does_not_hold_up_the_bind_of_a_written_face`）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CheckAtStart {
    /// 本文の面。
    pub(crate) main: bool,
    /// 抽出の面。
    pub(crate) sub: bool,
}

impl CheckAtStart {
    /// 起動の直後には何も照会しない（注入の入口 —— 照会先を持たない）。
    ///
    /// **本番の入口は使わない**（`App::bind_launching` が面ごとに決める）ので、テストの下にだけ置く。
    #[cfg(any(test, feature = "testing"))]
    pub(crate) const NONE: Self = Self {
        main: false,
        sub: false,
    };

    const fn any(self) -> bool {
        self.main || self.sub
    }
}

impl PendingRefresh {
    /// 起動時に決まった値と照会先から作る。**2つの器を作れる唯一の入口。**
    ///
    /// **本文用とサブ用の照会先は、どちらも `Option<Arc<LmStudioClient>>`。**
    /// 型では取り違えを止められないので、見張り番で止めてある——
    /// [`the_two_sources_are_queried_for_their_own_models`] が
    /// **申告値の違う2つのモデル**を1本で通す。
    ///
    /// **値は「明示か」と一緒に受ける**（[`Chosen`] / 作業単位 A-13a-1）——器がそれを持ち、
    /// 引き直しと切り替えが毎回読む。
    ///
    /// **`check_at_start` は背景タスクの最初の1回で照会する面**（作業単位 B-66 / [`CheckAtStart`]）。
    pub(crate) fn new(
        budget: Chosen<TokenBudget>,
        source: Option<FaceClient>,
        sub_context: Chosen<SubContext>,
        sub_source: Option<FaceClient>,
        upstream: UpstreamFacts,
        check_at_start: CheckAtStart,
    ) -> Self {
        Self {
            source,
            budget: SharedBudget::chosen(budget),
            sub_source,
            sub_context: SharedSubContext::chosen(sub_context),
            upstream: SharedUpstream::new(upstream),
            gate: Arc::new(CheckGate::default()),
            check_at_start,
        }
    }

    /// **起動時に選んだプロファイルの、本文の面の値を器へ入れる**（ADR-0059 §決定1・2 / 作業単位 A-18a）。
    ///
    /// **予算は触らない**（起動の予算は呼び出し側が同じ `body` で組んで [`Self::new`] へ渡している ——
    /// [`choose_budget`]）。ここで入れた値を、引き直し（[`adopt_refreshed`]）とチャットの入口（[`SharedBudget::turn`]）が読む。
    #[must_use]
    pub(crate) fn with_body(self, body: BodyFace) -> Self {
        self.budget.set_body(body);
        self
    }

    /// **照会の口**（作業単位 B-66 / `POST /admin/upstream/check`）。
    ///
    /// **背景タスクが使うのと同じ器・同じ照会先・同じ門**（`Arc` の複製）。`serve` が `spawn` の前に
    /// 控えて `AppState` へ載せる（[`Self::upstream`] と同じく、`spawn` は `self` を食べるので後からは取れない）。
    pub(crate) fn checker(&self) -> UpstreamChecker {
        UpstreamChecker {
            source: self.source.clone(),
            budget: self.budget.clone(),
            sub_source: self.sub_source.clone(),
            sub_context: self.sub_context.clone(),
            upstream: self.upstream.clone(),
            gate: Arc::clone(&self.gate),
        }
    }

    /// 共有の予算のハンドル。**テスト専用**（`App::budget_handle` と対）。
    ///
    /// 返るのは**同じ器**（`Arc` の複製）で、新しい器ではない。
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn shared(&self) -> SharedBudget {
        self.budget.clone()
    }

    /// **サブモデルのコンテキスト長の器**（作業単位 B-7）。
    ///
    /// 返るのは**同じ器**（`Arc` の複製）で、新しい器ではない。
    /// `App::bind_with` がこれを `MemoryExtractor` へ渡す。
    ///
    /// # なぜ `spawn` の返り値ではないのか
    ///
    /// **抽出器は `bind` の時点で組み立てる**（`Store` を握る必要がある）ので、
    /// `serve` まで待てない。**器はここで一度だけ作られている**ので、
    /// 走り出す前に渡しても、走り出した後に書かれる先と同じものになる。
    /// 別の器を渡す道は [`SharedSubContext::chosen`] が private であることで
    /// **モジュールの外からは塞がっている。**
    pub(crate) fn sub_context(&self) -> SharedSubContext {
        self.sub_context.clone()
    }

    /// **上流の観測の器**（作業単位 G-3b）。
    ///
    /// 返るのは**同じ器**（`Arc` の複製）で、新しい器ではない。
    /// `App::bind_with` がこれを `AppState` へ渡す。
    ///
    /// # なぜ `spawn` の返り値ではないのか
    ///
    /// [`sub_context`](Self::sub_context) と同じ——
    /// **器はここで一度だけ作られている**ので、走り出す前に渡しても、
    /// 走り出した後に書かれる先と同じものになる。
    /// 別の器を渡す道は [`SharedUpstream::new`] が private であることで
    /// **モジュールの外からは塞がっている。**
    pub(crate) fn upstream(&self) -> SharedUpstream {
        self.upstream.clone()
    }

    /// 定期的な引き直しを始める。**間隔は [`REQUERY_INTERVAL`] 固定。**
    ///
    /// **これが crate から見える唯一の入口。** 間隔を差し替える口を外に出さないので、
    /// 「設定にしない」（ADR-0020 §2）が配線としても保たれる。
    ///
    /// **返り値の `SharedBudget` は、走り出したタスクが書く先そのもの。**
    /// 呼び出し側はこれを `AppState` に入れるだけで、別の器を選べない。
    ///
    /// **本体は [`spawn_every`](Self::spawn_every) への委譲1行。** ここに残るのは
    /// 「5分の定数を渡す」1点だけで、そこは `tests/server_context_refresh.rs` が
    /// `interval_seconds` で見張っている。**器の対応付けをここに書かない**のは、
    /// 5分の定数を渡すこの関数をテストが通れないため（下記）。
    pub(crate) fn spawn(self) -> (RefreshTask, SharedBudget) {
        self.spawn_every(REQUERY_INTERVAL)
    }

    /// 間隔を指定して引き直しを始める。**このモジュールの中だけ**（テストが短い間隔で回す）。
    ///
    /// # 器の対応付けは、テストが通れるここに置く
    ///
    /// タスクへ渡す器と返す器が**同じ**であることが、この関数の存在理由。
    /// `SharedBudget::chosen` はこのモジュールの中からは呼べるので、
    /// **private 化ではモジュール内の取り違えを塞げない**
    /// （`spawn_refresh_every(self.source, SharedBudget::chosen(Chosen::measured(self.budget.get())), ..)` と
    /// 書けてしまう。作業単位 A-13a-1 より前は `SharedBudget::new` が同じ立場だった ——
    /// いまはそちらは `#[cfg(test)]` で、本番の器は `chosen` で作る）。そこで**5分を待てないテストでも通れる場所**へ対応付けを落とし、
    /// 単体テスト `the_spawned_task_writes_into_the_returned_budget` が
    /// 短い間隔で見張る。
    fn spawn_every(self, interval: Duration) -> (RefreshTask, SharedBudget) {
        let task = spawn_refresh_every(self.checker(), self.check_at_start, interval);
        (task, self.budget)
    }
}

/// **照会の回の門**（作業単位 B-66 / Issue origin#94 の決定3）。
///
/// **1度に走る照会の回は1つだけ。** 背景タスク（起動の直後・5分ごと）と「いま確かめる」
/// （`POST /admin/upstream/check`）が同じ門を通る。
///
/// | 走っている回 | 新しく来た「両方の面の回」 |
/// |---|---|
/// | 無い | **自分で走らせる** |
/// | 両方の面の回（5分ごと・別の「いま確かめる」） | **終わるのを待ち、走らせずに返る**（その回の結果が観測に入っている） |
/// | 起動の直後の回（明示の面だけ） | **終わるのを待ってから、自分で走らせる**（照会していない面が残るので） |
///
/// # 409 で断らない理由
///
/// **押した人が欲しいのは「いまの様子」で、走っている回の結果がちょうどそれである。**
/// 断ると、**5分ごとの回（利用者には見えない）と重なっただけで「確かめられませんでした」が出る**
/// ——押すたびに当たり外れがある失敗は、画面の作法（失敗はエラーで出す）では言い分けられない。
/// **並べもしない**（2本目の往復を上流へ重ねない —— 黙った上流へ同じ待ちを2つ積むことになる）。
///
/// # 「乗ってよいか」は、待っている間に両方の面の回が**終わった**かで決める（レビュー是正）
///
/// **1周目は「いま走っている回が両方の面の回か」の印**（待つ前に読む `bool`）で決めていた。それだと、
/// **持ち主の `POST` が切断されて回の future ごと落ちたとき、乗っていた側（5分ごとの回・2本目の `POST`）が
/// 照会されないまま返った**（印は「走っていた」と言うが、回は終わっていない）。
/// **いまは「終わった両方の面の回の数」**（[`Self::finished_whole_rounds`]）を、待つ前と錠を取った後で比べる。
/// 落とされた回は数を進めない（数えるのは回の本体が最後まで走った後）ので、乗っていた側は自分で照会する。
///
/// **⚠ 待つ側の `whole &&` には見張り番が無い**（外す変異は全緑 / レビュー是正の変異検査）。片面の回は起動の直後の
/// 1回だけで、**その面は両面の回が全部含む**ので、片面の回が待った後に両面の回へ乗っても観測に差が出ない —— **いまは等価**。
/// （待ち受けは `bind` の時点で開いているので、`POST` が先に錠を取って片面の回が待つ側になる順も、理屈の上では起こりうる。）
/// 片面の回の面が両面の回に含まれない形を足すなら、そのときに見張り番を足すこと。
/// **数える側の `if whole` には見張り番がある**（`tests/server_upstream_check.rs` の
/// `a_check_during_the_startup_check_of_one_face_still_checks_both`）。
///
/// **回の本体を `tokio::spawn` で要求から切り離す形は採らなかった** —— 背景タスクを止めても切り離した回が
/// 照会の上限まで残る（ADR-0020 §3「サーバが止まるときに残らない」）うえ、乗った側が受け取るのは
/// 「切断した人が頼んだ回」の結果で、自分で照会するのと待ち時間は変わらない。
#[derive(Debug, Default)]
pub(crate) struct CheckGate {
    /// 回そのものの錠。**照会の往復を跨いで持つ**ので `tokio` の錠。
    round: tokio::sync::Mutex<()>,
    /// **最後まで走り終えた、両方の面の回の数**（待った後に走らせずに返ってよいか）。
    ///
    /// **起動の直後の片面の回は数えない**（照会していない面が残るので、乗った側は自分で走らせる）。
    finished_whole_rounds: std::sync::atomic::AtomicU64,
}

/// **上流へ照会する口**（作業単位 B-66 / Issue origin#94）。
///
/// **背景タスク（起動の直後・5分ごと）と `POST /admin/upstream/check` が同じこれを使う**
/// （[`PendingRefresh::checker`] が同じ器と門の複製を渡す）。**照会の中身は面ごとに
/// [`refresh_once`] / [`refresh_sub_once`]** —— 書いていない面は予算の引き直しまで、
/// 書いた面は観測だけ。**「いま確かめる」を5分ごとと別の中身にしない**（決定3）。
#[derive(Debug, Clone)]
pub(crate) struct UpstreamChecker {
    source: Option<FaceClient>,
    budget: SharedBudget,
    sub_source: Option<FaceClient>,
    sub_context: SharedSubContext,
    upstream: SharedUpstream,
    gate: Arc<CheckGate>,
}

impl UpstreamChecker {
    /// **両方の面をいま照会する**（5分ごとの回・`POST /admin/upstream/check`）。
    ///
    /// **走っている両方の面の回があれば、それを待って返る**（[`CheckGate`] の表）。
    pub(crate) async fn check_now(&self) {
        self.round(CheckAtStart {
            main: true,
            sub: true,
        })
        .await;
    }

    /// **その面だけを照会する**（起動の直後の回 / [`CheckAtStart`]）。
    async fn check_at_start(&self, faces: CheckAtStart) {
        self.round(faces).await;
    }

    /// **1つの回。** 門を通り、照会先のある面を同時に照会する。
    ///
    /// **2つの面は同時に聞く**（`App::bind` の起動時の照会と同じ —— 黙った上流での待ちを2倍にしない）。
    /// **往復は面あたり1回のまま**（[`refresh_once`] / [`refresh_sub_once`]）。
    async fn round(&self, faces: CheckAtStart) {
        use std::sync::atomic::Ordering;
        let whole = faces.main && faces.sub;
        // **錠を待つ前に読む**（待っている間に終わった回だけを「乗れた回」と数える）。
        let finished_before = self.gate.finished_whole_rounds.load(Ordering::SeqCst);
        let _held = match self.gate.round.try_lock() {
            Ok(held) => held,
            Err(_) => {
                let held = self.gate.round.lock().await;
                if whole
                    && self.gate.finished_whole_rounds.load(Ordering::SeqCst) != finished_before
                {
                    // **待っている間に両方の面の回が最後まで走った。** その結果が、もう観測に入っている。
                    return;
                }
                held
            }
        };

        let main = async {
            if let (true, Some(client)) = (faces.main, self.source.as_ref()) {
                refresh_once(client, &self.budget, &self.upstream).await;
            }
        };
        let sub = async {
            if let (true, Some(client)) = (faces.sub, self.sub_source.as_ref()) {
                refresh_sub_once(client, &self.sub_context, &self.upstream).await;
            }
        };
        tokio::join!(main, sub);
        // **最後まで走った両方の面の回だけを数える**（途中で落とされた回は、ここへ来ない）。
        if whole {
            self.gate
                .finished_whole_rounds
                .fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// 間隔を指定して引き直しを始める。**このモジュールの中だけ**（テストが短い間隔で回す）。
///
/// # 応答の経路に待ちを足さない（ADR-0020 §3）
///
/// 引き直しは**背景タスク**で走る。リクエストの処理は共有の予算を読むだけなので、
/// 引く回のリクエストだけが遅くなる、という**ログを見ないと誰にも分からない**症状にならない。
///
/// # 間隔ごとに高々1回
///
/// `tokio::time::interval` ではなく `sleep` を使う。`interval` の既定
/// （`MissedTickBehavior::Burst`）は遅れを取り戻そうと**まとめて発火する**ので、
/// 上流が詰まった直後に往復が増える。`sleep` なら「前の引き直しが終わってから間隔」に
/// なり、**間隔あたりの往復が1回を超えない**ことが構造として決まる。
///
/// # **照会先は2つある**（作業単位 B-7）
///
/// 本文の予算（`source` → `budget`）と、抽出の上限のもと
/// （`sub_source` → `sub_context`）。**モデルが違うので照会も別。**
/// 1つの間隔の中で**同時に**引く（[`UpstreamChecker`] の1回 / 作業単位 B-66 で順から同時に替えた ——
/// 「いま確かめる」と同じ口を通すので、黙った上流での待ちを2倍にしない）。
/// **間隔あたりの往復はモデルごとに1回**のまま（合計 2 回 / 5 分。どちらも推論を起こさないメタデータの往復）。
///
/// **片方が無くてもタスクは走る。** `PERSONACORE_CONTEXT_TOKENS` は
/// 本文の予算だけを止める設定で、**抽出の上限には明示設定が無い**
/// （`resolve_sub_context` の doc）。2つとも無いときだけ走らせない。
/// **照会先が引くモデルの綴り**（ログの欄 / レビュー指摘2 / 2026-08-27）。
///
/// 引かない側は `-`。**空文字にしない**——欄が消えたのか「引かない」のかが読めなくなる。
///
/// **モデルの綴りは上流へ出た HTTP 本文にしか現れない**ので、
/// **この行だけが「どちらの照会先がどのモデルを引くつもりか」を起動時に見せる。**
/// **持ち回るのは所有した綴り。** 照会先そのものはタスクへ移すので、借りては置けない。
/// 起動時に1回ぶんの複製で、以後は動かない。
fn model_field(source: Option<&FaceClient>) -> String {
    source.map_or_else(|| NO_MODEL.to_owned(), FaceClient::model)
}

/// 引き直さない側の欄に入れる綴り。**モデル識別子として正当ではない値**を選ぶこと
/// （`ModelId::parse` は空白のみを拒む）。
const NO_MODEL: &str = "-";

fn spawn_refresh_every(
    checker: UpstreamChecker,
    check_at_start: CheckAtStart,
    interval: Duration,
) -> RefreshTask {
    // **「引くか」は器の「明示か」を毎回読んで決める**（作業単位 A-13a-1 / ADR-0054 §決定3）。
    // **ここで読むのは「いま（起動時点で）引くか」**——下の2行と「開始します」の欄が言うのはそれで、
    // **走ったまま切り替えれば変わる**（照会先は持ったままなので、明示でなくなれば引き直す）。
    let requeries_main = checker.source.is_some() && !checker.budget.is_explicit();
    let requeries_sub = checker.sub_source.is_some() && !checker.sub_context.is_explicit();

    if checker.source.is_some() && !requeries_main {
        // **本文の予算の側だけの話**（レビュー指摘5 / 2026-08-27）。
        // 直後にタスクが「開始します」を出すので、**主語を書かないと日本語の2文が
        // 正反対に読める。** `DISABLED_MARKER` は部分一致なので、綴りを伸ばす方向は安全。
        // **照会そのものは続ける**（観測だけ / 作業単位 B-66）ので、そう言い足す。
        tracing::info!(
            "本文の予算のコンテキスト長は明示設定なので定期的な引き直しを行いません（上流が答えるかの照会は続けます）"
        );
    }
    if checker.sub_source.is_some() && !requeries_sub {
        // **抽出の上限の側**（上流プロファイルの面に書いた値 / ADR-0054 §決定1）。
        tracing::info!(
            "抽出の上限のコンテキスト長は明示設定なので定期的な引き直しを行いません（上流が答えるかの照会は続けます）"
        );
    }

    // **照会先が2つとも無いとき（注入の入口）だけ走らせない。**
    // **明示の面は照会先を持ったまま走らせる**——観測のために照会し（作業単位 B-66）、
    // 走ったまま値の無いプロファイルへ切り替えたら次の間隔から引き直す（ADR-0054 §決定3）。
    if checker.source.is_none() && checker.sub_source.is_none() {
        return RefreshTask::idle();
    }

    // **どのモデルを引くつもりかを、起動時に1行で名乗る**（レビュー指摘2 / 2026-08-27）。
    //
    // **これが無いと、引き直しの照会先の取り違えを観測する手段が存在しない。**
    // 実測（2026-08-27）: 引き直しだけを本文モデルへ向ける形は、
    // **`_` で受け捨てれば clippy も通り 1554 全緑**だった——
    // **間隔（5分）を待てるテストが無い**ので、5分後に静かに壊れる。
    //
    // **5分を待たずに済むのは、これが「起動時に決まっている事実」だから。**
    // 待って確かめる必要のあるものではない。
    // 見張り番は `tests/server_context_refresh.rs` の
    // `the_two_requery_targets_name_their_own_models_on_the_real_bind`
    // （**同じ行の中で**2つの欄を見る）。
    //
    // **起動時点で明示の面は `-`**（作業単位 A-13a-1 より前は、明示の面は照会先そのものを
    // 持たなかったので `-` だった。**欄の意味「いま引くつもりのモデル」を変えない**）。
    let main_model = model_field(checker.source.as_ref().filter(|_| requeries_main));
    let sub_model = model_field(checker.sub_source.as_ref().filter(|_| requeries_sub));

    RefreshTask(Some(tokio::spawn(async move {
        // **タスクの中で出す。** ここが出ていれば「実際に走り出した」ことまで分かる
        // （spawn の前で出すと、spawn を消しても行だけが残る）。
        //
        // **どちらを引くのかも同じ行に出す。** 明示設定があると本文だけが止まるので、
        // 「開始します」だけでは何を引いているのかが読めない。
        tracing::info!(
            interval_seconds = interval.as_secs(),
            requeries_main_budget = requeries_main,
            requeries_sub_context = requeries_sub,
            main_model = %main_model,
            sub_model = %sub_model,
            "コンテキスト長の定期的な引き直しを開始します"
        );

        // **起動の直後の1回**（作業単位 B-66 / [`CheckAtStart`]）。`bind` の中で照会しなかった面だけ。
        // **`bind` の中で待たないために、ここ（背景タスクの最初）に置く。**
        if check_at_start.any() {
            checker.check_at_start(check_at_start).await;
        }

        loop {
            tokio::time::sleep(interval).await;
            // **「いま確かめる」と同じ口・同じ門**（作業単位 B-66）。走っている回があれば、それを待つ。
            checker.check_now().await;
        }
    })))
}

/// 1回分の引き直し。**失敗しても現在値を保つ**（ADR-0020 §5）。
///
/// 照会そのものの失敗は `info`。**上流が停止していれば本文生成が 503 で落ちる**ので、
/// ここが唯一の手掛かりになる状況ではない（起動時の `resolve_budget` と同じ深刻度）。
///
/// # 明示の間は観測だけ書く（ADR-0020 §4 の追記 / 作業単位 B-66 / Issue origin#94）
///
/// **予算が明示（環境変数か、上流プロファイルの面に書いた値）でも照会する。書くのは観測だけ**
/// ——予算は変えず、「変わりました」の `warn` も出さない（[`adopt_refreshed`] を呼ばない）。
/// **明示かどうかは毎回、器から読む**（走ったまま切り替えると変わる / ADR-0054 §決定3）。
///
/// > **作業単位 A-13a-1 から B-66 までは、明示の間は入口で戻っていた**（照会もせず観測も書かない）。
/// > **そのため上流が後から起きても止まっても、観測は起動時の「答えなし」のまま変わらず、
/// > 画面からは上流が生きているか・鍵が通るかが分からなかった**（Issue origin#94 のコメント1）。
/// > ADR-0020 §4 の根拠は「引いても予算は変わらないのに『変わった』と warn する記録が読み手を惑わす」で、
/// > **予算に触れず warn も出さない照会はこれと衝突しない**（利用者と決めた / 2026-11-03）。
///
/// **照会の最中に明示へ切り替わったら、採らない**——採用の口（[`SharedBudget::adopt_window`]）が
/// 同じ錠の中で「明示か」を見て断る。**採るかどうかの検査は観測を書く錠の中で行う**
/// （入口で読んだ「明示か」を持ち越さない）。
async fn refresh_once(source: &FaceClient, budget: &SharedBudget, upstream: &SharedUpstream) {
    // **版を先に、クライアントを後に読む**（作業単位 F-4b-2 / [`HeldFacts`] の doc）。
    // 差し替えは「クライアントを入れ替えてから版を進める」ので、
    // **この順なら、前の上流へ聞いた答えを新しい版で書く形が作れない。**
    let epoch = upstream.epoch(ModelFace::Main);
    let client = source.current();
    let current = budget.get();

    // **1回の往復から3つ読む**（作業単位 G-3b / [`probe_upstream`]）。
    // **`loaded_context_length()` へ戻さないこと**——あれは同じ往復の一覧と読み手を捨てる。
    // **往復は増えない**（見張り番は `the_requery_reads_everything_from_one_round_trip`）。
    //
    // **明示の面は観測のためだけに聞くので、一覧が読めなくても警告にしない**（レビュー是正 / [`UpstreamPhase::Observing`]）。
    let (probe, failure) = probe_with(&client, phase_for(budget.is_explicit())).await;
    if let Some(err) = failure {
        if budget.is_explicit() {
            // **明示の面は、予算の話をしない**（引き直していないので「保ちます」とも言わない）。
            tracing::info!(
                error = %err,
                context_tokens = current.context_tokens(),
                "本文の上流へ照会できませんでした（予算は明示設定のまま変えません）"
            );
        } else {
            // **既定値へ落とさない。** 上流が一瞬詰まっただけで予算が黙って縮む。
            tracing::info!(
                error = %err,
                context_tokens = current.context_tokens(),
                "コンテキスト長を引き直せませんでした。現在の値を保ちます"
            );
        }
    }

    // **観測は、採否と関係なく必ず書く。**
    // **予算は現在値を保ち、観測は「答えなし」になる**——
    // その食い違いだけが「上流がいま答えない」を外から見せる（`server::upstream` の doc）。
    //
    // **⚠ ただし、照会の最中に面が差し替わっていたら何も書かない**（作業単位 F-4b-2）。
    // その答えは前の上流のもので、**予算も前のモデルの窓である。**
    //
    // **予算の採用は、版の検査・観測と同じ錠の中**（[`SharedUpstream::store_face_since`] の doc）。
    // **外へ出すと、観測と予算の間に差し替えが挟まる窓ができる。**
    let written = upstream.store_face_since(ModelFace::Main, epoch, &probe, || {
        // **明示の面は観測だけ**（作業単位 B-66）——採らないので、変わったとも言わない。
        // **照会の最中に明示へ切り替わった回もここで止まる。**
        // **⚠ この `return` を外すと、明示の面で5分ごとに「変わりました」が鳴る**（予算は `adopt` が断るので
        // 変わらないまま）—— ADR-0020 §4 が塞いだ形そのもの。見張り番は
        // `tests::checking_an_explicit_face_never_says_the_length_changed`。
        if budget.is_explicit() {
            return;
        }
        // **余白は本文の面の上限**（ADR-0059 §決定2 —— 窓が戻れば、既定に戻した余白も面の値に戻る）。
        // **新しい値は次のプロンプト組立から効く。** 組立中のものには遡らない
        // （`chat` はリクエストの入口で1回だけ読む）。
        // **明示なら書かない**（同じ錠の中で見る / [`SharedBudget::adopt_window`]）。
        budget.adopt_window(probe.context_tokens);
    });
    if !written {
        tracing::info!("引き直しの最中に本文の上流が差し替わったので、この答えは捨てます");
    }
}

/// **サブモデルの側の1回分**（作業単位 B-7）。**失敗しても現在値を保つ**（ADR-0020 §5）。
///
/// 照会そのものの失敗は `info`。**落とし所へ落とさない**——上流が一瞬詰まっただけで
/// 抽出の上限が黙って広がる（＝窓を超えた依頼を送る側へ倒れる / ADR-0021 §8。LM Studio では
/// 前方から黙って切り捨てられる / ADR-0013。llama は 400 `exceed_context_size_error` で断るか、
/// プロンプト ＋ 応答の上限が窓を超えて応答が黙って短くなる / ADR-0052 M5）。
///
/// **明示の間は観測だけ書く**（上流プロファイルの面に書いた値 / ADR-0054 §決定1。
/// [`refresh_once`] と同じ形で、毎回器から読む / 作業単位 B-66）。
async fn refresh_sub_once(
    source: &FaceClient,
    sub_context: &SharedSubContext,
    upstream: &SharedUpstream,
) {
    // **版を先に、クライアントを後に読む**（[`refresh_once`] と同じ理由）。
    let epoch = upstream.epoch(ModelFace::Sub);
    let client = source.current();
    let current = sub_context.get();

    // **本文の側と同じ形**（[`probe_upstream`]）。往復は面あたり1回のまま。
    // **明示の面は観測のためだけ**（[`refresh_once`] と同じ / [`UpstreamPhase::Observing`]）。
    let (probe, failure) = probe_with(&client, phase_for(sub_context.is_explicit())).await;
    if let Some(err) = failure {
        if sub_context.is_explicit() {
            tracing::info!(
                error = %err,
                sub_context_length = current.tokens(),
                "抽出の上流へ照会できませんでした（上限は明示設定のまま変えません）"
            );
        } else {
            tracing::info!(
                error = %err,
                sub_context_length = current.tokens(),
                "サブモデルのコンテキスト長を引き直せませんでした。現在の値を保ちます"
            );
        }
    }

    // **抽出の面へ書く。** `ModelFace::Main` にすると本文の観測を潰す（帰属）。
    // **照会の最中に差し替わっていたら書かない**（[`refresh_once`] と同じ）。
    // **上限の採用も同じ錠の中**（[`refresh_once`] と同じ理由）。
    let written = upstream.store_face_since(ModelFace::Sub, epoch, &probe, || {
        // **明示の面は観測だけ**（[`refresh_once`] と同じ / 作業単位 B-66）。
        if sub_context.is_explicit() {
            return;
        }
        if let Some(next) = adopt_refreshed_sub(current, probe.context_tokens) {
            // **新しい上限は次の抽出から効く。** 走っている抽出には遡らない
            // （`MemoryExtractor::spawn` が往復の入口で1回だけ読む）。
            sub_context.adopt(next);
        }
    });
    if !written {
        tracing::info!("引き直しの最中に抽出の上流が差し替わったので、この答えは捨てます");
    }
}

/// **1つの面の上流クライアントの差し替え口**（ADR-0042 §決定4 / 作業単位 F-4b-2）。
///
/// # なぜ要るのか
///
/// **F-4b-1 までは、クライアントは `App::bind` で組まれて不変だった。**
/// 走ったまま上流を差し替える（`super::switch`）には、**その面の上流を使う者全員が、
/// 次の往復から新しいクライアントを使う**必要がある。使う者は4つ居る:
///
/// | 使う者 | 何を持っていたか | いま |
/// |---|---|---|
/// | 本文（`AppState::llm`） | `Arc<dyn LlmClient>` | **この口**（[`LlmClient`] を実装している） |
/// | 抽出（`MemoryExtractor`） | `Arc<dyn LlmClient>` | **この口** |
/// | 5分ごとの引き直し（[`PendingRefresh`]） | `Arc<LmStudioClient>` | **この口** |
/// | 載るのを待つ番人（`launch::FaceWatcher`） | `Arc<LmStudioClient>` | **この口**（[`FaceClients`] 経由） |
///
/// **1つの口を全員で共有する**ので、「本文は新しい上流へ、引き直しは古い上流へ」という
/// 食い違いを配線として書けない（**差し替え口を2つ作れば書ける** —— 作るのは `App::bind` の1箇所だけ）。
///
/// # 飛んでいる最中の往復は守らない（ADR-0042 §決定4）
///
/// **往復の入口で1回だけ読む**（[`Self::current`]）。差し替えの前に始まった往復は
/// 前のクライアントのまま走り切るか、**子を止めた瞬間に上流エラーで終わる。**
/// **待って合流する経路は作らない。**
///
/// # ⚠ `llm::LlmClient` の継ぎ目に触っている（ADR-0039 §決定5）
///
/// **具象型（`LmStudioClient`）の綴りは、このファイルと `server::mod`（構築点）にしか現れない**
/// ——どちらも `llm::upstream_leaks` の台帳に既に載っている場所である。
/// **トレイトの側は1バイトも変えていない**（実装を1つ足しただけ）。
#[derive(Clone, Debug)]
pub(in crate::server) struct FaceClient(Arc<RwLock<Arc<LmStudioClient>>>);

impl FaceClient {
    /// 起動時のクライアントで作る。
    ///
    /// **このモジュールの外からは作れない**——作る口は [`FaceClients::new`] だけ。
    /// 外に出すと `server::mod` で**同じ面の口を2つ**作れてしまい、
    /// 「本文は差し替わったのに、引き直しは前の口を握っている」配線が書ける
    /// （[`SharedBudget::chosen`] を private にしたのと同じ理屈）。
    /// **⚠ `FaceClients::new` を2度呼ぶ形は今も書ける**（`PendingRefresh::new` を2度呼ぶ形と同じ）。
    fn new(client: Arc<LmStudioClient>) -> Self {
        Self(Arc::new(RwLock::new(client)))
    }

    /// **いまのクライアント。** 往復の入口で1回だけ読むこと。
    ///
    /// 毒された錠でも中身は読む（`PoisonError::into_inner`）。
    /// **ここで panic すると、本文も抽出も全滅する。**
    pub(in crate::server) fn current(&self) -> Arc<LmStudioClient> {
        Arc::clone(&self.0.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// 実際に叩く宛先（**正規化済み** —— `…/v1/chat/completions` まで）。
    pub(in crate::server) fn endpoint(&self) -> String {
        self.current().config().endpoint().to_owned()
    }

    /// 依頼するモデルの綴り。
    pub(in crate::server) fn model(&self) -> String {
        self.current().config().model().as_str().to_owned()
    }

    /// **いまの鍵**（ADR-0056 §決定2）。無ければ `None`。
    pub(in crate::server) fn api_key(&self) -> Option<ApiKey> {
        self.current().config().api_key().cloned()
    }

    /// **いまの承認**（鍵を平文の `http://` で外へ送る / ADR-0056 §決定5 / 作業単位 A-13b-2）。
    pub(in crate::server) fn key_over_plain_http_allowed(&self) -> bool {
        self.current().config().key_over_plain_http_allowed()
    }

    /// **いま送るサンプラの、面に書いた値**（ADR-0058 / 作業単位 A-16）。何も書いていなければ既定。
    pub(in crate::server) fn sampling(&self) -> Sampling {
        self.current().config().sampling()
    }

    /// **いまのクライアントを、差し替え先の候補として持つ**（その面を変えないとき）。
    pub(in crate::server) fn as_candidate(&self) -> Candidate {
        Candidate(self.current())
    }

    /// **宛先とモデルを差し替えた、新しいクライアントの候補**（作業単位 F-4b-2）。
    ///
    /// **待ち時間の設定はいまのクライアントから引き継ぐ**（`PERSONACORE_LLM_IDLE_TIMEOUT` は
    /// 起動時に読んだきりで、プロファイルには載らない / `config::retarget_llm`）。
    /// **接続プールは新しく作る** —— 同じ宛先に別の子が立ち直ったとき、
    /// 前の子へ張った接続を使い回さない。
    ///
    /// **鍵は切り替え先の面のもの**（`api_key`。いまのクライアントから引き継がない /
    /// ADR-0056 §決定2 / `config::retarget_llm`）。**承認も切り替え先の面のもの**（ADR-0056 §決定5）。
    ///
    /// **サンプラの値も切り替え先の面のもの**（`face.sampling` / ADR-0058 §決定2）。
    pub(in crate::server) fn retargeted(
        &self,
        base_url: &str,
        model: &str,
        face: super::config::FaceSettings<'_>,
    ) -> Result<Candidate, ClientBuildError> {
        let config = super::config::retarget_llm(self.current().config(), base_url, model, face)
            .map_err(ClientBuildError::Config)?;
        let client = LmStudioClient::new(config).map_err(ClientBuildError::Client)?;
        Ok(Candidate(Arc::new(client)))
    }

    /// **同じ差し替え口か**（中身ではなく、口そのもの）。テスト専用。
    #[cfg(test)]
    fn is_same_handle(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// **差し替える。** 次の往復から効く。
    fn replace(&self, candidate: Candidate) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = candidate.0;
    }
}

/// **差し替え口そのものが上流である**（本文と抽出はこれを `Arc<dyn LlmClient>` として持つ）。
///
/// **往復の入口でいまのクライアントを1回だけ読み、あとはそれに任せる**
/// （`LmStudioClient` の固有メソッドを呼ぶ）。
impl LlmClient for FaceClient {
    fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        let client = self.current();
        Box::pin(async move { client.complete(request).await })
    }

    fn stream<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        let client = self.current();
        Box::pin(async move { client.stream(request).await })
    }

    /// **転送を落とさないこと**（ADR-0052 §作業単位 B-60a）。落とすとトレイトの既定
    /// （「数えられない」）に落ち、**黙って今のバイト数の計画のまま**になる ——
    /// どこも赤くならない壊れ方なので、見張り番は
    /// `tests::the_face_client_forwards_counting_to_the_current_client`。
    ///
    /// **⚠ この口そのものは写しを固定しない**（ADR-0052 §推奨3「封と送信は同じ
    /// 上流の写しを通す」）。呼ばれるたびに `current()` を読むので、封と送信をこの口で別々に呼ぶと、
    /// 間の差し替えで別のトークナイザに数えさせうる。
    /// **本文の呼び出し側は、先に [`LlmClient::pinned`]（下）で写しを取り、
    /// 封と送信をその写しで行う**（作業単位 B-60b / `AppState::llm_for_turn`）。
    /// **この転送が本文の経路で使われるのは、写しを取り損ねたときだけ**である。
    fn count_request<'a>(
        &'a self,
        request: &'a ChatRequest,
        streaming: bool,
    ) -> LlmFuture<'a, TokenCount> {
        let client = self.current();
        Box::pin(async move { client.count_request(request, streaming).await })
    }

    fn count_text<'a>(&'a self, text: &'a str) -> LlmFuture<'a, TokenCount> {
        let client = self.current();
        Box::pin(async move { client.count_text(text).await })
    }

    /// **いまのクライアントを、そのターンの写しとして渡す**（ADR-0052 §推奨3 / 作業単位 B-60b）。
    ///
    /// **差し替え口はこれを `Some` で返さなければならない**（[`LlmClient::pinned`] の doc）。
    /// `None` を返すと、本文の呼び出し側はこの口そのものを使い続け、
    /// **封と送信が別々に `current()` を読む**——どの往復も成功するので、どこも赤くならない。
    /// 見張り番は `tests::a_pinned_copy_keeps_talking_to_the_upstream_it_was_taken_from`。
    ///
    /// **⚠ 写しは飛んでいる最中の往復と同じ扱い**（この型の doc「飛んでいる最中の往復は守らない」）。
    /// 写しを取った後に差し替わっても、そのターンは前のクライアントのまま走り切る
    /// （**数えた上流と送った上流が一致する**のが要点で、新しい上流へ追いつくことではない）。
    fn pinned(&self) -> Option<Arc<dyn LlmClient>> {
        let client: Arc<dyn LlmClient> = self.current();
        Some(client)
    }
}

/// **差し替え先の候補**（作業単位 F-4b-2）。**組めたことが、そのまま検証になる。**
///
/// **差し替えは、全部の候補を組み終えてから何かに触る**（`super::switch`）——
/// 宛先の綴りが不正なら、子を1つも止めずに断れる。
#[derive(Debug)]
pub(in crate::server) struct Candidate(Arc<LmStudioClient>);

impl Candidate {
    /// 実際に叩く宛先（**正規化済み**。[`FaceClient::endpoint`] と同じ形で比べられる）。
    pub(in crate::server) fn endpoint(&self) -> &str {
        self.0.config().endpoint()
    }

    /// 依頼するモデルの綴り。
    pub(in crate::server) fn model(&self) -> &str {
        self.0.config().model().as_str()
    }

    /// **この候補が付ける鍵**（ADR-0056 §決定2）。無ければ `None`。
    pub(in crate::server) fn api_key(&self) -> Option<&ApiKey> {
        self.0.config().api_key()
    }

    /// **この候補の承認**（鍵を平文の `http://` で外へ送る / ADR-0056 §決定5）。
    pub(in crate::server) fn key_over_plain_http_allowed(&self) -> bool {
        self.0.config().key_over_plain_http_allowed()
    }

    /// **この候補が送るサンプラの、面に書いた値**（ADR-0058 / 作業単位 A-16）。
    pub(in crate::server) fn sampling(&self) -> Sampling {
        self.0.config().sampling()
    }

    /// **モデルだけを差し替えた複製**（接続プールを共有する / `LmStudioClient::with_model`）。
    ///
    /// **`sub` を書いていないプロファイルの抽出の面**が使う（ADR-0039 §決定1。
    /// 起動時の `App::bind` と同じ形）。
    pub(in crate::server) fn with_model(&self, model: &str) -> Result<Self, ClientBuildError> {
        let model = ModelId::parse(model).map_err(ClientBuildError::Model)?;
        Ok(Self(Arc::new(self.0.with_model(model))))
    }
}

/// **差し替え先のクライアントを組めなかった**（作業単位 F-4b-2）。
///
/// `Display` は**定数文字列**。詳細（綴りそのもの）は `#[source]` 側に残る。
#[derive(Debug, thiserror::Error)]
pub(in crate::server) enum ClientBuildError {
    /// 宛先かモデルの綴りが不正（起動時の設定と同じ検証 / `config::retarget_llm`）。
    #[error("上流の宛先かモデルの綴りが不正です")]
    Config(#[source] super::config::ConfigError),
    /// モデルの綴りが不正（抽出の面を本文から複製するとき）。
    #[error("上流のモデルの綴りが不正です")]
    Model(#[source] crate::llm::ModelIdError),
    /// クライアントそのものを組めない（TLS / リゾルバの初期化）。
    #[error("上流のクライアントを構築できませんでした")]
    Client(#[source] LlmError),
}

/// **面ごとの差し替え口**（ADR-0042 §決定3・4 / 作業単位 F-4b-1・F-4b-2）。
///
/// **本文用と抽出用は別の口**（中身は `with_model` の複製、または別の宛先）。
/// **取り違えると、抽出の上限が本文モデルの窓から決まる**（[`resolve_sub_context`] の doc）。
///
/// # ⚠ なぜ `launch` ではなくここに置くのか
///
/// **`/api/v0/models` は `LlmClient` に載っていない**（`llm` モジュールの doc）ので、
/// 照会先は**具象型**（`LmStudioClient`）で持つしかない。
/// **その綴りが `llm` の外へ出た場所は台帳で数えてある**
/// （`llm::upstream_leaks` / ADR-0036 決定4）——
/// **`launch` へ持たせると、継ぎ目の面が1ファイル増える。**
/// **増やす決定は別に要る**ので、ここ（既に数えられている場所）へ置いた。
#[derive(Clone)]
pub(in crate::server) struct FaceClients {
    main: FaceClient,
    sub: FaceClient,
}

impl FaceClients {
    /// **面ごとの差し替え口を作る。作るのは `App::bind` の1箇所だけ**（[`FaceClient`] の doc）。
    ///
    /// **本文用と抽出用のクライアントを、それぞれの口に入れる。**
    pub(in crate::server) fn new(main: Arc<LmStudioClient>, sub: Arc<LmStudioClient>) -> Self {
        Self {
            main: FaceClient::new(main),
            sub: FaceClient::new(sub),
        }
    }

    /// 面で引く。**帰属を1箇所で決める**（`UpstreamFacts::face` と同じ形）。
    pub(in crate::server) const fn face(&self, face: ModelFace) -> &FaceClient {
        match face {
            ModelFace::Main => &self.main,
            ModelFace::Sub => &self.sub,
        }
    }
}

/// **面ごとに、実際に送るサンプラの5つと、面の値で置き換えた欄を `info` で1行ずつ言う**
/// （ADR-0058 §決定3 / 作業単位 A-16）。
///
/// **呼ぶのは起動（`App::bind`）と走ったままの切り替え（`super::switch`）の後。** どちらも、いまの差し替え口の
/// クライアントから読む —— 送る本文と同じ設定（`LmStudioConfig::sampling`）を言う。
///
/// **小数は `Display` で出す**（`f32` の最短表記＝線の綴り / `llm::SentSamplers` の doc）。
/// **言えるのは「送った値」まで**で、上流で効いた値ではない（効いた値の権威は上流の側の観測）。
///
/// # 思考の量と、本文の返答の上限も言う（ADR-0059 §決定1・2 / 作業単位 A-18a）
///
/// **両方の面の行に `reasoning_effort`**（本文は本文の面に書いた値、無ければ `none`。**裏方は常に `none`**）。
/// **本文の面の行にだけ `response_tokens`**（いまの予算の余白 —— 本文の要求の `max_tokens` の天井）と、
/// `response_tokens_written`（面に書いた値。無ければ `None`）・`response_tokens_fallback`（既定へ戻したか）・
/// `response_tokens_fallback_cause`（戻した理由。窓が上限以下 `window_too_small` か、窓が分からない `window_unknown` か。
/// 戻していなければ `none` / [`response_fallback_of`]）。
/// 本文の面に書いた `reasoning_effort` は `overridden` にも並ぶ。
/// `body` と `budget` は本文の予算の器のいまの中身（起動では組んだ予算、切り替えでは差し替えて外の上流をその場で
/// 照会し直した後の器 —— 起こす上流はまだ載っていないので、窓が分からないまま言う）。
/// **これも「送る値」まで** —— 思考が上流で効いたかは言わない。
pub(in crate::server) fn report_samplers(
    clients: &FaceClients,
    body: BodyFace,
    budget: Chosen<TokenBudget>,
    occasion: &'static str,
) {
    let (fallback, cause) = response_fallback_of(body, budget);
    let budget = budget.value();
    for face in [ModelFace::Main, ModelFace::Sub] {
        let sampling = clients.face(face).sampling();
        let sent = sampling.sent_samplers();
        let mut overridden = sampling.overridden();
        match face {
            ModelFace::Main => {
                let effort = body.reasoning_effort.unwrap_or(ReasoningEffort::Off);
                if body.reasoning_effort.is_some() {
                    overridden.push(crate::server::upstreams::REASONING_EFFORT_KEY);
                }
                tracing::info!(
                    upstream_face = %face.label(),
                    occasion,
                    top_k = sent.top_k(),
                    top_p = %sent.top_p(),
                    min_p = %sent.min_p(),
                    repeat_penalty = %sent.repeat_penalty(),
                    presence_penalty = %sent.presence_penalty(),
                    reasoning_effort = %effort.as_str(),
                    response_tokens = budget.response_tokens(),
                    response_tokens_written = ?body.response_tokens,
                    response_tokens_fallback = fallback,
                    response_tokens_fallback_cause = %cause,
                    overridden = ?overridden,
                    "{SAMPLERS_LINE}"
                );
            }
            ModelFace::Sub => tracing::info!(
                upstream_face = %face.label(),
                occasion,
                top_k = sent.top_k(),
                top_p = %sent.top_p(),
                min_p = %sent.min_p(),
                repeat_penalty = %sent.repeat_penalty(),
                presence_penalty = %sent.presence_penalty(),
                // **裏方は常に既定**（ADR-0059 §決定1 —— 裏方の要求は思考の量を上書きしない）。
                reasoning_effort = %ReasoningEffort::Off.as_str(),
                overridden = ?overridden,
                "{SAMPLERS_LINE}"
            ),
        }
    }
}

/// **返答の上限を既定に戻したか、戻したならその理由**（[`report_samplers`] の `response_tokens_fallback` と
/// `response_tokens_fallback_cause` / A-18a のレビュー是正）。**純粋関数。**
///
/// | 予算の余白 | 落とし所の窓か | 返り値 |
/// |---|---|---|
/// | 面の上限（書いていなければ 1024）と同じ | どちらでも | `(false, "none")` |
/// | 違う | はい | `(true, "window_unknown")`（窓が分からないので既定にした） |
/// | 違う | いいえ | `(true, "window_too_small")`（窓が上限以下なので既定に戻した） |
fn response_fallback_of(body: BodyFace, budget: Chosen<TokenBudget>) -> (bool, &'static str) {
    if budget.value().response_tokens() == body.response_room() {
        (false, "none")
    } else if budget.on_fallback_window() {
        (true, "window_unknown")
    } else {
        (true, "window_too_small")
    }
}

/// **送るサンプラの行**（[`report_samplers`]）。テストがこの綴りで引く。
pub(in crate::server) const SAMPLERS_LINE: &str = "上流へ送るサンプラの値（面ごと）";

/// **その面の照会先へ1回聞く**（ADR-0042 §決定3 / 作業単位 F-4b-1）。
///
/// **帰属を1箇所で決める**——呼び出し側に `if` を書かせない
/// （`UpstreamFacts::face` と同じ形）。**取り違えると、
/// 「本文が載った」と言いながら抽出の窓を採る**形になる。
pub(in crate::server) async fn probe_face(
    clients: &FaceClients,
    face: ModelFace,
) -> (UpstreamProbe, Option<LlmError>) {
    probe_upstream(&clients.face(face).current()).await
}

/// **自分で起こした上流が載るのを待つ間に、その面の照会先へ1回聞く**
/// （作業単位 B-51 / Issue origin#20 / ADR-0042 §決定3）。
///
/// [`probe_face`] と**往復も読み方も同じ**で、違うのは
/// **「一覧がどちらの綴りでも読めなかった」1行が `warn` ではなく `debug` になること**だけ
/// （`LmStudioClient::models_while_loading`）。
///
/// **載っている最中の 503 は期待している状態**で、番人は2秒ごとに聞くので、
/// 警告にすると**本物の警告が埋もれる**（ADR-0035）。
/// **経過は番人が `info` で言い、緩い天井を超えたら `warn` を1本出す**（そちらは触っていない）。
///
/// # ⚠ 呼ぶのは番人の「待つ間」だけ
///
/// **載った後に残りの面を聞き直すのは [`probe_face`]**（載った上流の 503 は期待していない）。
/// **起こしていない面・5分ごとの引き直し・起動時の照会も今のまま警告**である。
/// 見張り番は `tests::an_unreadable_list_is_progress_for_a_launching_face_but_a_warning_for_an_external_one`
/// （**水準の対照だけ**）。**「載った後の聞き直しが `probe_face` であること」には見張り番が無い** ——
/// 聞き直しをこちらへ替える変異は全緑（レビュアーの `r5`）。壊れるのは「同じ子が答えた直後に
/// 2つ目の面へ 503 を返す」ときの水準だけ。
pub(in crate::server) async fn probe_face_while_loading(
    clients: &FaceClients,
    face: ModelFace,
) -> (UpstreamProbe, Option<LlmError>) {
    probe_with(&clients.face(face).current(), UpstreamPhase::Loading).await
}

/// **その面を新しいクライアントへ差し替え、「まだ何も聞いていない」に戻す**
/// （作業単位 F-4b-2 / ADR-0042 §決定4「古い事実を残さない」）。
///
/// # 順序（**クライアント → 観測と版**）
///
/// **先にクライアントを入れ替え、後で版を進める**（[`SharedUpstream::replace_face`]）。
/// 引き直しは「版を読む → クライアントを読む」の順なので（[`refresh_once`]）、
/// **この順なら、前の上流へ聞いた答えが新しい版で書かれる窓が無い。**
///
/// # 何を戻すか
///
/// | 器 | 戻す先 | なぜ |
/// |---|---|---|
/// | 観測 | **新しい宛先と綴りの「一度も聞いていない」** | 残すと画面が前のモデルの窓を言い続ける |
/// | 本文の予算 | **切り替え先の明示**（面の値、無ければ起動時の環境変数）。**明示が無ければ既定値** | 前のモデルの窓のまま組むと、小さい窓の上流で窓を超える（LM Studio ではプロンプトの先頭が黙って切られる / ADR-0013。llama は 400 `exceed_context_size_error` で断る / ADR-0052 M5。ただし、プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）） |
/// | 抽出の上限 | **切り替え先の明示**（面の値）。**明示が無ければ落とし所** | 同上 |
///
/// **起動時と同じ出発点**である（gguf を書いた面は、起動時も落とし所で始めて載ったら採る /
/// `server::App::bind_launching`）。**測り直しは呼び出し側**（外の上流はその場で1回、
/// 起こした面は載ったとき）。**明示の面は測り直しても採らない**（[`adopt_face_probe`]）。
///
/// # ⚠ 守れないもの —— 2つの順序に見張り番が無い（レビュー1周目 / 作業単位 F-4b-2）
///
/// | 順序 | 場所 | 入れ替えると |
/// |---|---|---|
/// | **クライアントを替えてから版を進める** | この関数 | 引き直しが新しい版と前のクライアントを読み、**前の上流の答えを新しい宛先の名前で書く** |
/// | **版を読んでからクライアントを読む** | [`refresh_once`] / [`refresh_sub_once`] | 同上 |
///
/// **どちらも入れ替える変異は `--lib` 全緑**（レビュアーが実測 / `r03` / `r04`）。
/// **2つの読み書きの間に `await` が無い**ので、テストがその間に差し替えを挟む手段が原理的に無い
/// （挟めるのは「照会の往復の最中」だけで、そこは
/// `a_requery_that_raced_a_switch_writes_nothing` が見ている）。
/// **窓は数命令ぶんで、しかも5分に1回の引き直しと差し替えが重なったときだけ開く。**
/// **守っているのはこの doc と、上の2箇所のコメントだけである。**
///
/// **「明示か」の側にはこの窓が無い**（作業単位 A-13a-1）——値と印は同じ錠の中で
/// 丸ごと書き（[`SharedBudget::install`]）、引き直しの採用は同じ錠の中で印を見て断る
/// （[`SharedBudget::adopt_window`]）。
///
/// # 明示の有無と値は、切り替え先に追従する（ADR-0054 §決定3 / 作業単位 A-13a-1）
///
/// **F-4b-2 までは「明示か」を起動時に決め、本文の予算は明示なら触らなかった。**
/// **いまは切り替え先のプロファイルの値（`profile`）と起動時の環境変数（`environment`）から
/// 決め直し、値と「明示か」を器へ丸ごと書く**（[`SharedBudget::install`]）。
/// **順位は起動時と同じ関数**（[`explicit_main_budget`]）——ぶつかったら `warn` を1本。
#[allow(clippy::too_many_arguments)]
pub(in crate::server) fn install_face(
    clients: &FaceClients,
    face: ModelFace,
    candidate: Candidate,
    budget: &SharedBudget,
    sub_context: &SharedSubContext,
    upstream: &SharedUpstream,
    environment: Option<TokenBudget>,
    profile: super::config::ProfileContext,
) {
    let endpoint = candidate.endpoint().to_owned();
    let model = candidate.model().to_owned();

    clients.face(face).replace(candidate);
    upstream.replace_face(
        face,
        UpstreamObservation::unqueried(face, &endpoint, &model),
    );

    match face {
        ModelFace::Main => {
            let previous = budget.get();
            // **本文の面の思考の量と返答の上限も、切り替え先の面のもの**（ADR-0059 §決定1・2）——予算と同じ錠の中で書く
            // （[`SharedBudget::install`]）。切り替え前の値を持ち越さない（書いていない面なら既定に戻る）。
            match explicit_main_budget(environment, profile.main, profile.body) {
                Some(explicit) => {
                    budget.install(Chosen::explicit(explicit), profile.body);
                    tracing::info!(
                        previous_context_tokens = previous.context_tokens(),
                        context_tokens = explicit.context_tokens(),
                        "本文の上流を差し替えました。予算は明示設定のコンテキスト長を使います"
                    );
                }
                None => {
                    // **窓はまだ分からないので、落とし所の窓で既定の余白にする**（面の返答の上限は使わない /
                    // [`unknown_window_budget`] / A-18a のレビュー是正）。器の面の値は切り替え先のものを入れる ——
                    // 照会し直して窓が分かったら面の上限に戻る（外の上流は切り替えがその場で、起こす上流は載ったときに
                    // [`adopt_face_probe`] が採る）。
                    let fallback =
                        unknown_window_budget(profile.body, UnknownWindow::NotYetQueried);
                    budget.install(fallback, profile.body);
                    tracing::info!(
                        previous_context_tokens = previous.context_tokens(),
                        context_tokens = fallback.value().context_tokens(),
                        "本文の上流を差し替えたので、予算を既定値から測り直します"
                    );
                }
            }
        }
        ModelFace::Sub => {
            let previous = sub_context.get();
            match profile.sub {
                Some(explicit) => {
                    sub_context.install(Chosen::explicit(explicit));
                    tracing::info!(
                        previous_sub_context_length = previous.tokens(),
                        sub_context_length = explicit.tokens(),
                        limit_tokens = explicit.prompt_limit(),
                        "抽出の上流を差し替えました。上限は明示設定のコンテキスト長を使います"
                    );
                }
                None => {
                    sub_context.install(Chosen::measured(SubContext::FALLBACK));
                    tracing::info!(
                        previous_sub_context_length = previous.tokens(),
                        sub_context_length = SubContext::FALLBACK.tokens(),
                        "抽出の上流を差し替えたので、上限を落とし所から測り直します"
                    );
                }
            }
        }
    }
}

/// **自分で起こした上流が載った直後の1回ぶんを採る**（ADR-0042 §決定3 / 作業単位 F-4b-1）。
///
/// # なぜ往復を増やさないのか
///
/// **[`probe_upstream`] の答えをそのまま受け取る。** 載ったことを見届けた往復と、
/// 測り直す往復を別にすると、**その隙間に上流が載せ替わった回だけ辻褄が合わなくなる。**
///
/// # 面ごとに、採る先が違う
///
/// | 面 | 観測 | 採用 |
/// |---|---|---|
/// | `Main` | 必ず書く | **本文の予算**（明示なら動かさない / ADR-0020 §4） |
/// | `Sub` | 必ず書く | **抽出の上限**（明示なら動かさない / ADR-0054 §決定1） |
///
/// **「明示か」は器が持つ**（作業単位 A-13a-1 / [`SharedBudget`] の doc）——
/// 走ったまま切り替えると変わるので、起動時に決めた `bool` を持ち回らない。
///
/// **観測は、採否と関係なく必ず書く**（[`refresh_once`] と同じ）——
/// **予算は現在値を保ち、観測だけが動く**状態こそが読みたいもの。
///
/// **⚠ ここで `measured::report` を呼び直さないこと。**
/// 呼ぶと `llama-server` を上流にしたときの 9 行の `Absent` が、
/// **載るたびに再生される**（作業単位 G-5 / `UpstreamObservation::refreshed` の doc）。
/// **判定は [`UpstreamObservation::refreshed`] が `judge_face` で作り直す。**
pub(in crate::server) fn adopt_face_probe(
    face: ModelFace,
    probe: &UpstreamProbe,
    budget: &SharedBudget,
    sub_context: &SharedSubContext,
    upstream: &SharedUpstream,
) {
    upstream.store_face(face, probe);

    match face {
        ModelFace::Main => {
            if budget.is_explicit() {
                // **明示設定が優先される以上、引いても予算は変わらない**（ADR-0020 §4）。
                // **観測だけ書いて帰る。**
                return;
            }
            // **余白は本文の面の上限**（ADR-0059 §決定2 / [`adopt_refreshed`]）。
            budget.adopt_window(probe.context_tokens);
        }
        ModelFace::Sub => {
            if sub_context.is_explicit() {
                // **上流プロファイルの面に書いた値が優先される**（ADR-0054 §決定1）。観測だけ書いて帰る。
                return;
            }
            if let Some(next) = adopt_refreshed_sub(sub_context.get(), probe.context_tokens) {
                sub_context.adopt(next);
            }
        }
    }
}

/// 引き直したサブモデルの申告値を今の値と突き合わせる。**変わったときだけ `Some`。**
///
/// [`adopt_refreshed`] と**同じ表**（失敗 / 同じ / 使えない / 変わった）。
/// **写しではなく別の関数**なのは、扱う型も採否の規則も違うため
/// （あちらは `TokenBudget::new`、こちらは [`SubContext::new`]）。
fn adopt_refreshed_sub(current: SubContext, queried: Option<u32>) -> Option<SubContext> {
    let tokens = queried?;

    if tokens == current.tokens() {
        return None;
    }

    match SubContext::new(tokens) {
        Some(next) => {
            tracing::warn!(
                previous_sub_context_length = current.tokens(),
                sub_context_length = next.tokens(),
                limit_tokens = next.prompt_limit(),
                "サブモデルのコンテキスト長が変わりました。次の抽出から新しい上限を使います"
            );
            Some(next)
        }
        None => {
            // **採用しないことを告げる文言に割る**（`adopt_refreshed` と同じ理由）。
            tracing::warn!(
                queried = tokens,
                sub_context_length = current.tokens(),
                "上流が申告したサブモデルのコンテキスト長は抽出に使えません。現在の値を保ちます"
            );
            None
        }
    }
}

/// 引き直した結果を今の値と突き合わせる。**変わったときだけ `Some`**（純粋な判断）。
///
/// | `queried` | 返り値 | ログ |
/// |---|---|---|
/// | `None`（照会に失敗） | `None`（現在値を保つ） | 呼び出し側が `info` |
/// | 組んだ予算が現在値と同じ（窓が同じで予算にならない回も） | `None` | **出さない**（5分ごとに鳴らせない） |
/// | 予算にならない | `None`（現在値を保つ） | `warn`（保ったことを告げる） |
/// | 窓が変わった | `Some(新しい予算)` | `warn`（前後の値を並べる） |
/// | 窓は同じで余白だけが変わる（落とし所の窓と同じ窓が分かった回 / A-18a のレビュー是正） | `Some(新しい予算)` | `info`（前後の余白を並べる） |
///
/// **黙って予算だけ変えない。** 変えると「昨日と同じ発言が今日は `BudgetExhausted`」の
/// 理由が追えない。
///
/// # 応答の余白は「いまの余白」ではなく、本文の面に書いた上限（ADR-0059 §決定2 / 作業単位 A-18a）
///
/// **`wanted` は本文の面の返答の上限**（[`SharedBudget::body`]。書いていなければ 1024）。**いまの予算の余白を
/// 引き継がない** —— 小さい窓で既定の 1024 に戻した後、窓が戻ったら面の値に戻すため（[`budget_for_window`] が
/// 毎回判定し直す。戻すときは `warn` を1本）。
///
/// **受容している穴（A-18a の再レビュー）:** 落とし所 (8192, 1024) の後に照会した窓がちょうど 8192 で、面の上限が
/// 8192 以上だと、組んだ予算も (8192, 1024) で同じなので早期 return し、**窓の不足の行（[`RESPONSE_FALLBACK_LINE`]）は
/// 出ない**（値は正しい。器の落とし所の印は [`SharedBudget::adopt_window`] が外す）。
fn adopt_refreshed(current: TokenBudget, queried: Option<u32>, wanted: u32) -> Option<TokenBudget> {
    let tokens = queried?;

    // **同じかどうかは、窓の数ではなく組んだ予算で比べる**（A-18a のレビュー是正）—— 窓が分からず落とし所の窓
    // 8192・既定の余白で組んだ後に、照会した窓が同じ 8192 だった回も、面の上限で組み直す。
    let fitted = budget_for_window(tokens, wanted);
    if tokens == current.context_tokens() && fitted.is_none_or(|fitted| fitted.budget() == current)
    {
        return None;
    }

    // **生成用の余白は本文の面の上限。** 上流から取れるのはコンテキスト全体の
    // 長さで、生成用に何を残すかは上流の都合ではない（`choose_budget` と同じ理屈）。
    match fitted {
        Some(fitted) => {
            let next = fitted.report(wanted);
            if tokens == current.context_tokens() {
                // **窓は同じで、余白だけが変わる**（落とし所の窓と同じ窓を照会できた回）。「変わりました」とは言わない。
                tracing::info!(
                    context_tokens = next.context_tokens(),
                    previous_response_tokens = current.response_tokens(),
                    response_tokens = next.response_tokens(),
                    "上流のコンテキスト長が分かったので、本文の返答の上限で予算を組み直します"
                );
            } else {
                tracing::warn!(
                    previous_context_tokens = current.context_tokens(),
                    context_tokens = next.context_tokens(),
                    previous_response_tokens = current.response_tokens(),
                    response_tokens = next.response_tokens(),
                    "上流のコンテキスト長が変わりました。次のプロンプト組立から新しい予算を使います"
                );
            }
            Some(next)
        }
        None => {
            // **採用しないことを告げる文言に割る。**「変わりました」とだけ言うと、
            // 予算が追随したのかしなかったのかが読み手に伝わらない。
            // **理由は既定の余白で組んだときの断り**（`None` のとき、既定の余白でも必ず組めない / [`budget_for_window`]）。
            let error = TokenBudget::new(tokens, TokenBudget::DEFAULT_RESPONSE_TOKENS).err();
            tracing::warn!(
                error = ?error,
                queried = tokens,
                context_tokens = current.context_tokens(),
                "上流が申告したコンテキスト長は予算になりません。現在の値を保ちます"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::llm::AnsweredBy;
    use crate::measured::check_face;
    use crate::server::upstream::UpstreamObservation;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    use super::*;
    use crate::llm::{DEFAULT_MAIN_MODEL, LmStudioConfig, ModelId};
    use crate::testlog::{
        VERBOSE_FILTER, captured_under_default_filter, captured_with_filter, has_field, has_key,
        line_containing,
    };

    /// 実機で観測した値（2026-08-14 / ADR-0013）。**既定値 8192 の 2 倍。**
    const LOADED: u32 = 16_384;

    /// 実機のモデルが取りうる最大値。**これを信じると、LM Studio では黙って切り捨てられる**
    /// （ADR-0013）。llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。ただし、
    /// プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
    const MAXIMUM: u32 = 262_144;

    /// 既定値へ落ちたときのログの手掛かり。**全文一致にしないこと**
    /// （文言の推敲で、見張りたい「既定フィルタで残るか」と無関係に割れる）。
    const FALLBACK_MARKER: &str = "既定値を使います";

    /// 「照会そのものが失敗した」側の手掛かり。
    const UNAVAILABLE_MARKER: &str = "取得できなかった";

    /// 「照会は成功したが、申告値が予算にならなかった」側の手掛かり。
    ///
    /// **この2つを混ぜないこと。** 上流が 1024 のモデルをロードしている状態は
    /// 「上流が停止している」とは調べる先がまるで違うのに、同じ文言だと区別できない。
    const UNUSABLE_MARKER: &str = "予算になりません";

    /// 明示設定を採用した側の手掛かり。
    const EXPLICIT_MARKER: &str = "明示設定を使います";

    /// 上流の申告値を採用した側の手掛かり。
    ///
    /// **この2つも混ぜないこと。** 採用元を取り違えると、運用者は
    /// `PERSONACORE_CONTEXT_TOKENS` と上流のロード設定という
    /// **まるで違う場所**を調べることになる（`FALLBACK_MARKER` を
    /// 「取得できなかった」と「予算になりません」で割ったのと同じ理屈）。
    const UPSTREAM_MARKER: &str = "上流の実コンテキスト長を使います";

    fn model() -> ModelId {
        ModelId::parse(DEFAULT_MAIN_MODEL).expect("既定モデルは正当であること")
    }

    fn budget_of(context_tokens: u32) -> TokenBudget {
        TokenBudget::new(context_tokens, TokenBudget::DEFAULT_RESPONSE_TOKENS)
            .expect("テストで使う予算は正当であること")
    }

    // ------------------------------------------------------------------
    // **本文の面に何も書いていない呼び方**（作業単位 A-18a より前の呼び方）。
    //
    // A-18a で予算を組む関数は本文の面の値（`BodyFace`）を受けるようになった。**このモジュールの既存の
    // テストは「何も書いていない面」の振る舞いを見ている**ので、同じ名前の薄い口で既定の面を渡す
    // （グロブで取り込んだ `super::*` の同名の関数より、このモジュールの定義が勝つ）。
    // **書いた面の腕は、`super::` を直に呼ぶ**（`a18a_` で始まるテスト）。
    // ------------------------------------------------------------------

    fn choose_budget(explicit: Option<TokenBudget>, queried: Option<u32>) -> TokenBudget {
        super::choose_budget(explicit, queried, BodyFace::default()).value()
    }

    async fn resolve_budget(
        explicit: Option<TokenBudget>,
        upstream: &LmStudioClient,
    ) -> (TokenBudget, UpstreamProbe) {
        let (chosen, probe) = super::resolve_budget(explicit, upstream, BodyFace::default()).await;
        (chosen.value(), probe)
    }

    fn adopt_refreshed(current: TokenBudget, queried: Option<u32>) -> Option<TokenBudget> {
        super::adopt_refreshed(current, queried, TokenBudget::DEFAULT_RESPONSE_TOKENS)
    }

    fn explicit_main_budget(
        environment: Option<TokenBudget>,
        profile: Option<TokenBudget>,
    ) -> Option<TokenBudget> {
        super::explicit_main_budget(environment, profile, BodyFace::default())
    }

    /// 実機と同じ形の応答本文。
    fn models_body(loaded: &str) -> String {
        format!(
            r#"{{"object":"list","data":[
                {{"id":"{DEFAULT_MAIN_MODEL}","object":"model","type":"llm","state":"loaded",
                  "max_context_length":{MAXIMUM},"loaded_context_length":{loaded}}}]}}"#
        )
    }

    /// `GET /api/v0/models` に固定応答を返すモックサーバ。**実機には繋がない。**
    async fn upstream_serving(response: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v0/models"))
            .respond_with(response)
            .mount(&server)
            .await;
        server
    }

    /// 実機と同じ本文を返すモックサーバ。
    async fn upstream_reporting(loaded: &str) -> MockServer {
        upstream_serving(
            ResponseTemplate::new(200).set_body_raw(models_body(loaded), "application/json"),
        )
        .await
    }

    fn client_for(base_url: &str) -> LmStudioClient {
        let config = LmStudioConfig::new(base_url, model()).expect("ベース URL が正当であること");
        LmStudioClient::new(config).expect("HTTP クライアントを構築できること")
    }

    /// 誰も待ち受けていないアドレス（**上流が停止している状態**の再現）。
    async fn closed_address() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("一時ポートを確保できること");
        let addr = listener.local_addr().expect("アドレスを取得できること");
        drop(listener);
        format!("http://{addr}")
    }

    /// **上流が申告した値を実際に使う。**
    ///
    /// ここが既定値に固定されていると、実コンテキストの半分しか使わない状態が
    /// エラーも出さずに続く（ADR-0013 の出発点そのもの）。
    #[tokio::test]
    async fn the_upstream_value_is_used_when_nothing_is_configured() {
        let server = upstream_reporting(&LOADED.to_string()).await;

        let (budget, UpstreamProbe { catalogue, .. }) =
            resolve_budget(None, &client_for(&server.uri())).await;

        assert_eq!(
            catalogue.as_deref(),
            Some([DEFAULT_MAIN_MODEL.to_owned()].as_slice()),
            "同じ往復の本文から一覧も読むこと（往復は増やさない）"
        );
        assert_eq!(
            budget.context_tokens(),
            LOADED,
            "照会結果をそのまま使うこと（既定値でも最大値でもない）"
        );
        assert_ne!(
            budget.context_tokens(),
            TokenBudget::DEFAULT_CONTEXT_TOKENS,
            "照会結果を無視して既定値へ落ちていないこと"
        );
        assert_ne!(
            budget.context_tokens(),
            MAXIMUM,
            "max_context_length を信じないこと（実際には無い分まで詰め込む）"
        );
        assert_eq!(
            budget.response_tokens(),
            TokenBudget::DEFAULT_RESPONSE_TOKENS,
            "生成用の余白は上流の都合で動かさないこと"
        );
    }

    /// **本文の側の照会も、読んだ一覧を持ち帰る**（作業単位 G-3a の是正）。
    ///
    /// # なぜ要るのか
    ///
    /// **是正前、この層は同じ往復で読めた一覧を捨てていた。**
    /// 捨てた結果、`measured` の**本文の面 4 件が丸ごと「聞けなかった」**に落ちる
    /// ——落ちる先に `MEASURED_CONTROL_TOKEN_SPELLINGS` が含まれる
    /// （[ADR-0033](../../docs/adr/0033-measured-constants-name-what-they-were-measured-on.md)
    /// §決定3 が「最優先」と名指しした行）。
    ///
    /// # 4通りを1本に置く（**片側だけだと決め打ちが通る**）
    ///
    /// | 腕 | 一覧 | なぜ |
    /// |---|---|---|
    /// | 照会が通る（**ロード済み**） | **`Some`（上流が並べた綴り）** | 捨てる変異が赤になる |
    /// | 明示設定 | **`None`** | 「常に `Some` を返す」変異が赤になる |
    /// | 往復が失敗 | **`None`** | 同上（かつ**空の `Some` に潰す**変異も赤） |
    /// | **応答は読めたが設定中のモデルが未ロード** | **`Some`** | **コンテキスト長が読めない回に一覧まで捨てる**変異が赤になる |
    ///
    /// **⚠ 4つ目の腕は是正2周目で足した**（2026-09-07）。
    /// **それまで [`resolve_budget`] の doc は「一覧は `Some`」という主張の見張り番として
    /// このテストを名指ししていたが、その状態を作る腕が1つも無かった**
    /// ——**赤にしていたのは統合側の
    /// `two_upstreams_are_each_judged_against_their_own_catalogue` ただ1本**
    /// （レビュアーが実測 / `m19`）。
    ///
    /// **一覧の中身まで見る。** 件数だけだと、
    /// **設定中のモデルの綴りを1件でっち上げる**実装と区別が付かない。
    /// **上流には2件並べ、そのうち1件だけが設定中のモデル**にしてある。
    #[tokio::test]
    async fn the_body_side_query_brings_back_the_catalogue_it_read() {
        // **上流は2件持っている**（1件だと「設定中の綴りを返すだけ」と区別が付かない）。
        const STRANGER: &str = "someone-else-entirely";
        assert_ne!(
            STRANGER, DEFAULT_MAIN_MODEL,
            "前提: 一覧の2件は別の綴りであること"
        );
        let body = format!(
            r#"{{"object":"list","data":[
                {{"id":"{DEFAULT_MAIN_MODEL}","object":"model","type":"llm","state":"loaded",
                  "max_context_length":{MAXIMUM},"loaded_context_length":{LOADED}}},
                {{"id":"{STRANGER}","object":"model","type":"llm","state":"not-loaded",
                  "max_context_length":{MAXIMUM}}}]}}"#
        );
        let server =
            upstream_serving(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
                .await;

        // ---- 腕1: 照会が通る。**一覧をそのまま持ち帰る。** ----
        let (budget, UpstreamProbe { catalogue, .. }) =
            resolve_budget(None, &client_for(&server.uri())).await;
        assert_eq!(
            budget.context_tokens(),
            LOADED,
            "前提: 同じ往復でコンテキスト長も読めていること"
        );
        assert_eq!(
            catalogue,
            Some(vec![DEFAULT_MAIN_MODEL.to_owned(), STRANGER.to_owned()]),
            "上流が並べた綴りを、並んだ順にそのまま返すこと"
        );

        // ---- 腕2: 明示設定。**照会そのものを行わないので `None`。** ----
        let explicit = budget_of(4_096);
        let (_, UpstreamProbe { catalogue, .. }) =
            resolve_budget(Some(explicit), &client_for(&server.uri())).await;
        assert_eq!(
            catalogue, None,
            "明示設定があるときだけ「一度も聞いていない」が真であること"
        );

        // ---- 腕3: 往復そのものが失敗。**空の `Some` に潰さない。** ----
        let base_url = closed_address().await;
        let (_, UpstreamProbe { catalogue, .. }) =
            resolve_budget(None, &client_for(&base_url)).await;
        assert_eq!(
            catalogue, None,
            "往復が失敗したなら「聞けなかった」であること（空の一覧と区別する）"
        );

        // ---- 腕4: **応答は読めたが、設定中のモデルが未ロード。** ----
        // **往復は成功していて、コンテキスト長だけが読めない状態**——
        // 上の3通りのどれでもない（腕1は `loaded`）。
        // **一覧とコンテキスト長は失敗の仕方が違う**ので、一覧は `Some` のまま返る。
        let cold = format!(
            r#"{{"object":"list","data":[
                {{"id":"{DEFAULT_MAIN_MODEL}","object":"model","type":"llm","state":"not-loaded",
                  "max_context_length":{MAXIMUM}}},
                {{"id":"{STRANGER}","object":"model","type":"llm","state":"not-loaded",
                  "max_context_length":{MAXIMUM}}}]}}"#
        );
        let server =
            upstream_serving(ResponseTemplate::new(200).set_body_raw(cold, "application/json"))
                .await;
        let (budget, UpstreamProbe { catalogue, .. }) =
            resolve_budget(None, &client_for(&server.uri())).await;
        assert_eq!(
            budget,
            TokenBudget::default(),
            "前提: コンテキスト長のほうは読めていないこと（読めていたら腕1と同じ状態になる）"
        );
        assert_eq!(
            catalogue,
            Some(vec![DEFAULT_MAIN_MODEL.to_owned(), STRANGER.to_owned()]),
            "コンテキスト長が読めなくても、読めた一覧までは捨てないこと"
        );
    }

    /// **比率で膨らませない。** 取得した値をそのままコンテキスト長にする。
    ///
    /// 校正比率（日本語 3.67 倍）で掛けると見積もりの上界が破れ、
    /// **LM Studio ではシステムプロンプトが黙って切り捨てられる**（ADR-0013「却下した選択肢」）。
    /// llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。ただし、
    /// プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
    #[tokio::test]
    async fn the_upstream_value_is_not_scaled() {
        for loaded in [2_048_u32, 16_384, 262_144] {
            let server = upstream_reporting(&loaded.to_string()).await;
            let (budget, _) = resolve_budget(None, &client_for(&server.uri())).await;
            assert_eq!(
                budget.context_tokens(),
                loaded,
                "loaded = {loaded}: 比率で換算しないこと"
            );
        }
    }

    /// **面の値が環境変数に勝ち、ぶつかったら同じ行で言う**（ADR-0054 §決定2 / 作業単位 A-13a-1）。
    ///
    /// | 環境変数 | 面の値 | 結果 | `warn` |
    /// |---|---|---|---|
    /// | 無し | 無し | 無し | 無し |
    /// | あり | 無し | 環境変数 | 無し |
    /// | 無し | あり | 面の値 | 無し |
    /// | あり | **同じ値** | 面の値 | **無し**（勝ち負けが無い） |
    /// | あり | 違う値 | **面の値** | **1本**（何が・どちらの値か・を同じ行で） |
    #[test]
    fn a_profile_value_beats_the_environment_and_a_conflict_is_one_warning() {
        const ENVIRONMENT: u32 = 12_288;
        const PROFILE: u32 = 28_672;
        let tokens = |budget: Option<TokenBudget>| budget.map(TokenBudget::context_tokens);

        let quiet = captured_under_default_filter(|| {
            assert_eq!(tokens(explicit_main_budget(None, None)), None);
            assert_eq!(
                tokens(explicit_main_budget(Some(budget_of(ENVIRONMENT)), None)),
                Some(ENVIRONMENT)
            );
            assert_eq!(
                tokens(explicit_main_budget(None, Some(budget_of(PROFILE)))),
                Some(PROFILE)
            );
            assert_eq!(
                tokens(explicit_main_budget(
                    Some(budget_of(PROFILE)),
                    Some(budget_of(PROFILE))
                )),
                Some(PROFILE)
            );
        });
        assert!(
            !quiet.contains(PROFILE_OVER_ENVIRONMENT),
            "ぶつかっていないのに言っている: {quiet:?}"
        );

        let conflict = captured_under_default_filter(|| {
            assert_eq!(
                tokens(explicit_main_budget(
                    Some(budget_of(ENVIRONMENT)),
                    Some(budget_of(PROFILE))
                )),
                Some(PROFILE),
                "面の値が勝つこと"
            );
        });
        let line = line_containing(&conflict, PROFILE_OVER_ENVIRONMENT);
        assert!(line.contains("WARN"), "warn で言うこと: {line:?}");
        for field in [
            // **`main.rs` の `warn_on_conflicts` と同じ書き方**（`&str` を素で渡すので引用符つき）。
            format!("setting={:?}", super::super::config::ENV_CONTEXT_TOKENS),
            format!("from_environment={ENVIRONMENT}"),
            format!("from_profile={PROFILE}"),
        ] {
            assert!(
                has_field(line, &field),
                "{field} を同じ行で言うこと: {line:?}"
            );
        }
        assert_eq!(
            conflict.matches(PROFILE_OVER_ENVIRONMENT).count(),
            1,
            "1本だけ言うこと: {conflict:?}"
        );
    }

    /// **明示設定は照会より強い。** しかも照会そのものを行わない。
    #[tokio::test]
    async fn an_explicit_setting_wins_over_the_upstream() {
        let server = upstream_reporting(&LOADED.to_string()).await;
        let explicit = budget_of(4_096);

        let (budget, UpstreamProbe { catalogue, .. }) =
            resolve_budget(Some(explicit), &client_for(&server.uri())).await;

        assert_eq!(
            budget, explicit,
            "上流が {LOADED} と申告しても明示設定を優先すること"
        );
        assert_eq!(
            catalogue, None,
            "照会していないのだから一覧も持たないこと（ここだけが「一度も聞いていない」）"
        );
        assert!(
            server
                .received_requests()
                .await
                .is_some_and(|r| r.is_empty()),
            "明示設定があるなら照会しないこと（起動のたびの往復を増やさない）"
        );
    }

    /// **停止している上流でも値が決まる。** 起動は止まらない。
    #[tokio::test]
    async fn an_unreachable_upstream_falls_back_to_the_default() {
        let base_url = closed_address().await;

        let (budget, UpstreamProbe { catalogue, .. }) =
            resolve_budget(None, &client_for(&base_url)).await;

        assert_eq!(budget, TokenBudget::default());
        assert_eq!(
            catalogue, None,
            "往復が失敗したなら一覧も「聞けなかった」であること"
        );
    }

    /// **上流の応答が壊れていても既定値に落ちる。** panic もしない。
    ///
    /// 欠落・型違い・`0`・`state` 違い・モデル不在・HTTP エラーを一通り通す。
    #[tokio::test]
    async fn a_broken_upstream_answer_falls_back_to_the_default() {
        let broken: Vec<ResponseTemplate> = vec![
            // JSON として壊れている。
            ResponseTemplate::new(200).set_body_raw("{\"data\":[", "application/json"),
            ResponseTemplate::new(200).set_body_raw("", "application/json"),
            // `data` が無い。
            ResponseTemplate::new(200).set_body_raw("{}", "application/json"),
            // 設定中のモデルが一覧に無い。
            ResponseTemplate::new(200).set_body_raw(
                r#"{"data":[{"id":"別のモデル","state":"loaded","loaded_context_length":16384}]}"#,
                "application/json",
            ),
            // ロードされていない。
            ResponseTemplate::new(200).set_body_raw(
                format!(
                    r#"{{"data":[{{"id":"{DEFAULT_MAIN_MODEL}","state":"not-loaded",
                        "max_context_length":262144}}]}}"#
                ),
                "application/json",
            ),
            // 値が欠けている / 型が違う / 0 / 巨大すぎる。
            ResponseTemplate::new(200).set_body_raw(models_body("null"), "application/json"),
            ResponseTemplate::new(200).set_body_raw(models_body("\"16384\""), "application/json"),
            ResponseTemplate::new(200).set_body_raw(models_body("0"), "application/json"),
            ResponseTemplate::new(200).set_body_raw(models_body("4294967296"), "application/json"),
            // 予算として成立しない（生成用の余白以下）。
            ResponseTemplate::new(200).set_body_raw(models_body("1024"), "application/json"),
            // HTTP エラー（独自 API を持たない上流・過負荷）。
            ResponseTemplate::new(404),
            ResponseTemplate::new(500).set_body_raw("<html>internal</html>", "text/html"),
        ];

        for (index, response) in broken.into_iter().enumerate() {
            let server = upstream_serving(response).await;

            let (budget, _) = resolve_budget(None, &client_for(&server.uri())).await;

            assert_eq!(
                budget,
                TokenBudget::default(),
                "壊れた応答 #{index} は既定値へ落ちること"
            );
        }
    }

    /// **既定値へ落ちたことは既定のログ設定で見える。**
    ///
    /// ここが `debug` に落とされると、**実コンテキストの半分しか使っていない**状態が
    /// 運用中に一切露見しなくなる（エラーも出ず応答も返るため）。
    #[test]
    fn the_fallback_is_visible_under_the_default_log_filter() {
        let output = captured_under_default_filter(|| {
            let budget = choose_budget(None, None);
            assert_eq!(budget, TokenBudget::default());
        });

        // **深刻度と文言は同じ行で見る**（作業単位 B-2 / `crate::testlog` の doc）。
        //
        // **⚠ いまこの捕捉に囮は無い**（B-2-2 / レビュー指摘 R-3）。
        // `choose_budget` は**どの枝でも1件しか出さない**ので、
        // **2026-08-26 に実測しても1行だけ**だった——出力全体で見ても結果は変わらない。
        // **行に絞ってあるのは、この経路にイベントが1本増えた瞬間、
        // 別々に `contains` で見る形が「その `WARN` がこの行のものか」を
        // 見ないまま緑になるから。**
        let fallback = line_containing(&output, FALLBACK_MARKER);
        assert!(
            fallback.contains("WARN"),
            "深刻度は warn であること（`RUST_LOG=error` 以外では必ず残る）: {fallback:?}"
        );
        assert!(
            fallback.contains(UNAVAILABLE_MARKER),
            "照会そのものが失敗したことが分かる文言であること: {fallback:?}"
        );
    }

    /// **「取れたが予算にならなかった」も既定のログ設定で見える。**
    ///
    /// 上流が 1024 のモデルをロードしている状態では**照会は成功する**。
    /// ここが黙ると、既定値 8192 で**実際の8倍のプロンプトを組む**のに、
    /// **予算が誤っていることを示すログが1行も出ない。**（LM Studio では黙って切り捨てられる /
    /// ADR-0013。llama は 400 `exceed_context_size_error` で断り、`server::chat` がその往復の
    /// `error!` を出すが、原因が予算だとは言わない / ADR-0052 M5。プロンプト ＋ 応答の上限が
    /// 窓を超えると、応答が黙って短くなる（`finish_reason=length`）。）
    /// `choose_budget(None, None)` だけを通していると、
    /// 警告を `if queried.is_none()` で括る変更が素通りする（11c-1 のレビュー指摘）。
    ///
    /// # 否定側（`!contains(UNAVAILABLE_MARKER)`）の前に肯定を置く
    ///
    /// 「取得できなかった」の行が**そもそも出せなくなっていても**、
    /// 否定だけなら緑を通る。先に `trace` で出せることを確かめておく。
    #[test]
    fn a_queried_value_that_cannot_become_a_budget_is_reported() {
        // **前提: 「取得できなかった」側の行は現に出せる。**
        let unavailable = captured_with_filter(VERBOSE_FILTER, || {
            let _ = choose_budget(None, None);
        });
        assert!(
            unavailable.contains(UNAVAILABLE_MARKER),
            "前提: 照会できなかった側の文言は現に出せること: {unavailable:?}"
        );

        // 上流が「1024 をロード中」と正しく申告した状態（既定の余白 1024 以下）。
        let output = captured_under_default_filter(|| {
            let budget = choose_budget(None, Some(512));
            assert_eq!(budget, TokenBudget::default());
        });

        // **深刻度と文言は同じ行で見る**（作業単位 B-2）。
        // **⚠ 囮の有無は上のテストと同じ**（いまは1行だけ / B-2-2 のレビュー指摘 R-3）。
        let fallback = line_containing(&output, FALLBACK_MARKER);
        assert!(
            fallback.contains("WARN"),
            "深刻度は warn であること: {fallback:?}"
        );
        // **調査先が違うので文言を割る。** 値は取れているのに
        // 「取得できなかった」と言われると、上流の疎通を疑って空振りする。
        assert!(
            fallback.contains(UNUSABLE_MARKER),
            "申告値が予算にならなかったことが分かる文言であること: {fallback:?}"
        );
        // **否定は出力全体で見る**（どの行にも無いことを主張したい）。
        assert!(
            !output.contains(UNAVAILABLE_MARKER),
            "値は取れているので「取得できなかった」と報告しないこと: {output:?}"
        );
    }

    /// 取れたときは既定値の警告を出さない（鳴りっぱなしにしない）。
    ///
    /// # 「出ないこと」の前に「出ること」を確かめる
    ///
    /// 否定だけを主張すると、**既定値の警告そのものが出せなくなっていても緑**になる。
    /// 実際に2つの壊れ方がある（`server::usage` の同種のテストと同じ）:
    ///
    /// * `warn!` を消す／`if` で括り直す変更 —— 否定側は「何も出なかった」ので緑。
    /// * `tracing` の callsite interest が `never` に焼き付く —— 同上。
    #[test]
    fn a_successful_lookup_does_not_warn() {
        // **前提: 既定値の警告は現に出せる。** ここが出せないなら、
        // 以下の「出ないこと」は何も見ていない。
        let fallback = captured_with_filter(VERBOSE_FILTER, || {
            let _ = choose_budget(None, None);
        });
        assert!(
            fallback.contains(FALLBACK_MARKER),
            "前提: 既定値へ落ちた側の文言は現に出せること: {fallback:?}"
        );

        let from_upstream = captured_under_default_filter(|| {
            assert_eq!(choose_budget(None, Some(LOADED)).context_tokens(), LOADED);
        });
        assert!(
            !from_upstream.contains(FALLBACK_MARKER),
            "取れているのに既定値の警告を出さないこと: {from_upstream:?}"
        );
        // **黙っているのではなく、採用した値を残していること。**
        // 出力が丸ごと空でも上の否定は通るので、ここで区別する。
        //
        // **欄は採用元を告げる行の中で見る**（作業単位 B-21d）。
        // 塊のまま `has_field` に渡すと `split_whitespace` が改行をまたぐので、
        // 主張は「**その欄を持つ行が出力のどこかに1本ある**」に化ける
        // （`crate::testlog` の `has_field` の doc）。
        let adopted = line_containing(&from_upstream, UPSTREAM_MARKER);
        assert!(
            has_field(adopted, &format!("context_tokens={LOADED}")),
            "採用したコンテキスト長を残すこと: {adopted:?}"
        );

        let explicit = captured_under_default_filter(|| {
            assert_eq!(
                choose_budget(Some(budget_of(4_096)), None).context_tokens(),
                4_096
            );
        });
        assert!(
            !explicit.contains(FALLBACK_MARKER),
            "明示設定を「取得できなかった」と報告しないこと: {explicit:?}"
        );
        // **欄はトークンとして突き合わせる**（作業単位 B-12）——`contains` だと
        // `context_tokens=40960`（桁が伸びた別の値）にも当たる。
        // **そして欄は採用元を告げる行の中で見る**（作業単位 B-21d / 上と同じ理由）。
        let chosen = line_containing(&explicit, EXPLICIT_MARKER);
        assert!(
            has_field(chosen, "context_tokens=4096"),
            "明示設定でも採用した値を残すこと: {chosen:?}"
        );
    }

    /// **どちらの経路で値が決まったのかがログから読める。**
    ///
    /// 採用元は「明示設定」と「上流の申告値」で調べる先がまるで違う。文言を取り違えると、
    /// 上流が想定外の長さでロードされているのに `PERSONACORE_CONTEXT_TOKENS` を
    /// 探して空振りする（逆も同じ）。`FALLBACK_MARKER` を2つに割ったのと同じ理屈。
    ///
    /// # 両方向の否定を置く
    ///
    /// 片側だけ（例: 明示設定側の `!contains(UPSTREAM_MARKER)` だけ）だと、
    /// **2つの文言をそっくり入れ替える変異が両方向とも素通りする**
    /// （0006 のレビュー指摘。実測で緑を通った）。
    ///
    /// 否定の前提——「相手側の文言はそもそも出せる」——は、
    /// **同じテストのもう一方の肯定側**が担保している。
    #[test]
    fn the_source_of_the_budget_is_named_in_the_log() {
        const EXPLICIT_TOKENS: u32 = 4_096;
        assert_ne!(
            EXPLICIT_TOKENS, LOADED,
            "前提: 明示設定と上流の申告値が別の値であること（同値だと取り違えを区別できない）"
        );

        let explicit = captured_under_default_filter(|| {
            assert_eq!(
                choose_budget(Some(budget_of(EXPLICIT_TOKENS)), Some(LOADED)).context_tokens(),
                EXPLICIT_TOKENS
            );
        });
        let from_upstream = captured_under_default_filter(|| {
            assert_eq!(choose_budget(None, Some(LOADED)).context_tokens(), LOADED);
        });

        // 肯定側。**この2つが、下の否定側の前提を兼ねている。**
        assert!(
            explicit.contains(EXPLICIT_MARKER),
            "明示設定を使ったと言うこと: {explicit:?}"
        );
        assert!(
            from_upstream.contains(UPSTREAM_MARKER),
            "上流の実コンテキスト長を使ったと言うこと: {from_upstream:?}"
        );

        // 否定側（両方向）。文言の入れ替えはここで赤になる。
        assert!(
            !explicit.contains(UPSTREAM_MARKER),
            "明示設定なのに上流から取れたと報告しないこと: {explicit:?}"
        );
        assert!(
            !from_upstream.contains(EXPLICIT_MARKER),
            "上流から取れたのに明示設定と報告しないこと: {from_upstream:?}"
        );
    }

    /// 優先順位を純粋関数の側でも固定する（通信を挟まずに読める形で残す）。
    #[test]
    fn the_priority_is_explicit_then_queried_then_default() {
        let explicit = budget_of(4_096);

        assert_eq!(
            choose_budget(Some(explicit), Some(LOADED)),
            explicit,
            "明示設定が最優先"
        );
        assert_eq!(
            choose_budget(None, Some(LOADED)).context_tokens(),
            LOADED,
            "次が照会結果"
        );
        assert_eq!(
            choose_budget(None, None),
            TokenBudget::default(),
            "最後が既定値"
        );
    }

    /// 予算として成立しない申告値は既定値へ落とす（`0` と余白以下）。
    #[test]
    fn a_queried_value_that_cannot_become_a_budget_falls_back() {
        for tokens in [0_u32, 1, 512, TokenBudget::DEFAULT_RESPONSE_TOKENS] {
            assert_eq!(
                choose_budget(None, Some(tokens)),
                TokenBudget::default(),
                "tokens = {tokens} は予算にならないこと"
            );
        }
        // 境界: 余白より 1 大きければ成立する（既定値へ落とさない）。
        assert_eq!(
            choose_budget(None, Some(TokenBudget::DEFAULT_RESPONSE_TOKENS + 1)).prompt_allowance(),
            1
        );
    }

    // ------------------------------------------------------------------
    // 定期的な引き直し（ADR-0020 / 作業単位 11c-4）
    //
    // **稼働中に上流が小さいコンテキストでモデルを再ロードしても、
    // どの観測点にも現れない**（ADR-0013 の訂正）。ここが唯一の網。
    // ------------------------------------------------------------------

    /// 稼働中に上流が**小さいコンテキストで再ロードした**ときの新しい申告値。
    ///
    /// **既定値・起動時の申告値・明示設定のどれとも別の数**にすること
    /// （同値だと「引き直した値」なのか「元の値」なのかを区別できない）。
    const RELOADED: u32 = 4_096;

    /// 明示設定として与える値。**上流の申告値とも既定値とも別。**
    const CONFIGURED: u32 = 12_288;

    /// 引き直しで得たが**予算にならない**申告値（既定の余白 1024 以下）。
    const UNUSABLE: u32 = 512;

    /// 引き直しが**引き継ぐ**生成用の余白。**既定値と別の数**にすること。
    ///
    /// 既定値のままだと「現在値から引き継いだ」と「既定値を置いた」が同値になり、
    /// **引き継ぎを捨てる変異が緑を通る**（このプロジェクト最頻の穴）。
    ///
    /// **A-18a から、引き直しが使う余白の出どころは「いまの予算の余白」ではなく「本文の面の上限」**
    /// （ADR-0059 §決定2 —— 小さい窓で既定へ戻した後、窓が戻れば面の値に戻すため）。本番では2つは同じ値で
    /// 入る（器を作るときに同じ値で組む）ので、この値を使うテストは器に**同じ値を面の上限として**入れる
    /// （[`carried_body`]）。
    const CARRIED_RESPONSE_TOKENS: u32 = 2_048;

    /// **[`CARRIED_RESPONSE_TOKENS`] を本文の面の上限として書いた形**（作業単位 A-18a）。
    fn carried_body() -> BodyFace {
        BodyFace {
            reasoning_effort: None,
            response_tokens: Some(CARRIED_RESPONSE_TOKENS),
        }
    }

    /// 余白が**既定値と別**の予算。引き継ぎを見る側のフィクスチャはこれを使う。
    ///
    /// 前提（2つが別の値であること）を**ヘルパの入口で**確かめる。
    fn carried_budget(context_tokens: u32) -> TokenBudget {
        assert_ne!(
            CARRIED_RESPONSE_TOKENS,
            TokenBudget::DEFAULT_RESPONSE_TOKENS,
            "前提: 引き継ぐ余白が既定値と別であること（同値だと引き継ぎを確認できない）"
        );
        TokenBudget::new(context_tokens, CARRIED_RESPONSE_TOKENS)
            .expect("テストで使う予算は正当であること")
    }

    /// 引き直しのテストで使う間隔。**5分を実時間で待たないための短縮。**
    const SHORT_INTERVAL: Duration = Duration::from_millis(20);

    /// **テストの間には決して経過しない**間隔（「間隔を待つ」側の検証用）。
    const LONG_INTERVAL: Duration = Duration::from_secs(60);

    /// 待ちの上限。超えたら**落とす**（緑のまま何も測らない形にしない）。
    const WAIT_LIMIT: Duration = Duration::from_secs(20);

    /// 待ちの刻み。
    const POLL_STEP: Duration = Duration::from_millis(5);

    /// 本文側で「変わった」と告げる行の手掛かり。**全文一致にしない。**
    ///
    /// **⚠ `サブモデルの` まで含めて、サブ側と重ならない綴りにしてある**
    /// （作業単位 B-21d のレビュー指摘 / 2026-08-29）。
    /// B-21d より前は `"コンテキスト長が変わりました"` だったが、
    /// それは**サブ側の文言（`サブモデルのコンテキスト長が変わりました…`）の部分文字列**で、
    /// **本番では main と sub が1ティックで並んで出る。**
    /// 見張り番は [`each_layer_marker_picks_its_own_line_when_both_layers_speak`]。
    ///
    /// [`each_layer_marker_picks_its_own_line_when_both_layers_speak`]:
    ///     each_layer_marker_picks_its_own_line_when_both_layers_speak
    const CHANGED_MARKER: &str = "上流のコンテキスト長が変わりました";

    /// 本文側で「現在値を保った」と告げる行の手掛かり。
    ///
    /// **`CHANGED_MARKER` と文字列として重ならないこと。**
    /// 重なると「変わったから採用した」と「変わったが採用しなかった」を区別できない。
    ///
    /// **⚠ そして**[`SUB_KEPT_MARKER`]**とも重ならないこと**（B-21d のレビュー指摘）。
    /// **`"現在の値を保ちます"` だけでは、本番の4つの文言すべてに当たる**
    /// ——本文の引き直し失敗・サブの引き直し失敗・本文の採用見送り・サブの採用見送り。
    /// **どちらの層の話かを、手掛かりの側で決める。**
    ///
    /// [`SUB_KEPT_MARKER`]: SUB_KEPT_MARKER
    const KEPT_MARKER: &str = "予算になりません。現在の値を保ちます";

    /// **フィクスチャの値がすべて違う。**
    ///
    /// このプロジェクトで最頻の穴は「2つの値が同値で、取り違えを検出できない」。
    /// 上の定数を書き換えて同値にした瞬間、ここが赤になる。
    #[test]
    fn the_context_fixtures_are_all_different() {
        let values = [
            ("既定値", TokenBudget::DEFAULT_CONTEXT_TOKENS),
            ("起動時の申告値", LOADED),
            ("再ロード後の申告値", RELOADED),
            ("明示設定", CONFIGURED),
            ("予算にならない値", UNUSABLE),
        ];

        for (index, (left_label, left)) in values.iter().enumerate() {
            for (right_label, right) in values.iter().skip(index + 1) {
                assert_ne!(
                    left, right,
                    "{left_label} と {right_label} は別の値であること"
                );
            }
        }
    }

    /// **間隔は 5 分**（ADR-0020 §2）。設定にはしない。
    #[test]
    fn the_requery_interval_is_five_minutes() {
        assert_eq!(REQUERY_INTERVAL, Duration::from_secs(5 * 60));
    }

    /// **答えを途中で変える上流。** 与えた系列を順に返し、尽きたら最後の値を返し続ける。
    ///
    /// 固定応答のモックでは「引き直しが効いている」ことを区別できない
    /// （起動時に取った値のままでも同じ結果になる）。
    #[derive(Clone)]
    struct ChangingAnswers {
        answers: Arc<Vec<u32>>,
        served: Arc<AtomicUsize>,
    }

    impl ChangingAnswers {
        fn new(answers: &[u32]) -> Self {
            assert!(!answers.is_empty(), "答えを1つ以上与えること");
            Self {
                answers: Arc::new(answers.to_vec()),
                served: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl Respond for ChangingAnswers {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            let index = self
                .served
                .fetch_add(1, Ordering::SeqCst)
                .min(self.answers.len() - 1);
            ResponseTemplate::new(200).set_body_raw(
                models_body(&self.answers[index].to_string()),
                "application/json",
            )
        }
    }

    /// 答えを順に変える上流を立てる。
    async fn upstream_answering(answers: &[u32]) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v0/models"))
            .respond_with(ChangingAnswers::new(answers))
            .mount(&server)
            .await;
        server
    }

    /// 引き直しの照会先。**モックサーバへ向ける**（実機には繋がない）。
    fn source_for(server: &MockServer) -> FaceClient {
        face_of(client_for(&server.uri()))
    }

    /// **クライアント1つを、差し替え口に入れる**（作業単位 F-4b-2 / 本番と同じ持ち方）。
    fn face_of(client: LmStudioClient) -> FaceClient {
        FaceClient::new(Arc::new(client))
    }

    /// 観測の器に入れる本文側の宛先。**抽出側とも既定とも別の綴り。**
    const MAIN_ENDPOINT_FOR_TESTS: &str = "http://127.0.0.1:19001";

    /// 観測の器に入れる抽出側の宛先。**本文側とも既定とも別の綴り。**
    const SUB_ENDPOINT_FOR_TESTS: &str = "http://127.0.0.1:19002";

    /// **引き直しが書き込む先の初期状態**（作業単位 G-3b）。
    ///
    /// **「一度も聞いていない」から始める。** 引き直しが書いたのか、
    /// 最初からそうだったのかを区別できる状態にしておくこと
    /// （`the_requery_writes_what_it_read_into_the_upstream_facts` が使う）。
    fn upstream_facts() -> UpstreamFacts {
        assert_ne!(
            MAIN_ENDPOINT_FOR_TESTS, SUB_ENDPOINT_FOR_TESTS,
            "前提: 2つの宛先が別であること（同値だと取り違えを検出できない）"
        );
        assert_ne!(
            DEFAULT_MAIN_MODEL, SUB_MODEL_ID,
            "前提: 2つのモデルの綴りが別であること"
        );
        UpstreamFacts::new(
            UpstreamObservation::unqueried(
                ModelFace::Main,
                MAIN_ENDPOINT_FOR_TESTS,
                DEFAULT_MAIN_MODEL,
            ),
            UpstreamObservation::unqueried(ModelFace::Sub, SUB_ENDPOINT_FOR_TESTS, SUB_MODEL_ID),
        )
    }

    /// **本文の予算だけを引き直す配線**（サブ側の照会先を持たない）。
    ///
    /// 本文側の見張り番はサブ側の往復を数えたくない
    /// ——同じモックサーバを2つの照会が叩くと、`queries_to` の数が2倍になり
    /// 「間隔ごとに高々1回」が読めなくなる。**サブ側は専用の見張り番が見る。**
    fn spawn_main_refresh_every(
        source: Option<FaceClient>,
        budget: SharedBudget,
        interval: Duration,
    ) -> RefreshTask {
        spawn_main_refresh_into(
            source,
            budget,
            SharedUpstream::new(upstream_facts()),
            interval,
        )
    }

    /// **本文の側だけ・観測の器を受け取る形**（作業単位 B-66 —— 明示の面の観測を読むため）。
    fn spawn_main_refresh_into(
        source: Option<FaceClient>,
        budget: SharedBudget,
        upstream: SharedUpstream,
        interval: Duration,
    ) -> RefreshTask {
        spawn_refresh_every(
            checker_of(
                source,
                budget,
                None,
                SharedSubContext::new(SubContext::FALLBACK),
                upstream,
            ),
            CheckAtStart::NONE,
            interval,
        )
    }

    /// **サブモデルの側だけを引き直す配線**（本文の照会先を持たない）。
    fn spawn_sub_refresh_every(
        sub_source: Option<FaceClient>,
        sub_context: SharedSubContext,
        interval: Duration,
    ) -> RefreshTask {
        spawn_sub_refresh_into(
            sub_source,
            sub_context,
            SharedUpstream::new(upstream_facts()),
            interval,
        )
    }

    /// **抽出の側だけ・観測の器を受け取る形**（作業単位 B-66）。
    fn spawn_sub_refresh_into(
        sub_source: Option<FaceClient>,
        sub_context: SharedSubContext,
        upstream: SharedUpstream,
        interval: Duration,
    ) -> RefreshTask {
        spawn_refresh_every(
            checker_of(
                None,
                SharedBudget::new(budget_of(LOADED)),
                sub_source,
                sub_context,
                upstream,
            ),
            CheckAtStart::NONE,
            interval,
        )
    }

    /// **照会の口を直に組む**（門は新しく1つ。本番は [`PendingRefresh::checker`] が組む）。
    fn checker_of(
        source: Option<FaceClient>,
        budget: SharedBudget,
        sub_source: Option<FaceClient>,
        sub_context: SharedSubContext,
        upstream: SharedUpstream,
    ) -> UpstreamChecker {
        UpstreamChecker {
            source,
            budget,
            sub_source,
            sub_context,
            upstream,
            gate: Arc::new(CheckGate::default()),
        }
    }

    /// `/api/v0/models` を叩いた回数。
    async fn queries_to(server: &MockServer) -> usize {
        server.received_requests().await.map_or(0, |requests| {
            requests
                .iter()
                .filter(|request| request.url.path() == "/api/v0/models")
                .count()
        })
    }

    /// 照会が `at_least` 回に届くまで待つ。**届かなければ落とす。**
    async fn wait_for_queries(server: &MockServer, at_least: usize) {
        let deadline = Instant::now() + WAIT_LIMIT;
        loop {
            let seen = queries_to(server).await;
            if seen >= at_least {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "照会が {at_least} 回に届かなかった（実測 {seen} 回）"
            );
            tokio::time::sleep(POLL_STEP).await;
        }
    }

    /// 共有の予算が `tokens` になるまで待つ。**ならなければ落とす。**
    async fn wait_for_budget(shared: &SharedBudget, tokens: u32) {
        let deadline = Instant::now() + WAIT_LIMIT;
        while shared.get().context_tokens() != tokens {
            assert!(
                Instant::now() < deadline,
                "共有の予算が {tokens} にならなかった（実測 {}）",
                shared.get().context_tokens()
            );
            tokio::time::sleep(POLL_STEP).await;
        }
    }

    /// **上流が答えを変えたら、共有の予算が差し替わる。**
    ///
    /// これが死ぬと、上流が 4096 で再ロードされてもこちらは 16384 のつもりで組み続け、
    /// **窓を超える**（ADR-0020）。LM Studio では**プロンプトの先頭（ペルソナ・性格・記憶）が
    /// 黙って切り捨てられる**（ADR-0013）。llama は 400 `exceed_context_size_error` で断る
    /// （ADR-0052 M5）。ただし、プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる
    /// （`finish_reason=length`）。
    ///
    /// 上流の1回目の答えは**起動時と同じ値**にしてある。
    /// 「1回目で必ず書き換える」実装と「変わったときだけ書き換える」実装を、
    /// この順序が区別する。
    ///
    /// # 余白は**既定値と別の数**から始める
    ///
    /// 既定値のまま（`budget_of`）だと、`refresh_once` が現在値を読まずに
    /// `TokenBudget::default()` を突き合わせる変異が緑を通る。
    /// **引き継ぎが消える**だけでなく、上流が安定していても現在値と食い違い続けるので
    /// **5分ごとに「変わりました」が鳴りっぱなしになる。**
    #[tokio::test]
    async fn a_changed_upstream_value_replaces_the_shared_budget() {
        let server = upstream_answering(&[LOADED, RELOADED]).await;
        let shared = SharedBudget::new(carried_budget(LOADED));
        // **本番と同じく、余白と同じ値を面の上限として器に入れる**（[`CARRIED_RESPONSE_TOKENS`] の doc / A-18a）。
        shared.set_body(carried_body());

        let task =
            spawn_main_refresh_every(Some(source_for(&server)), shared.clone(), SHORT_INTERVAL);

        wait_for_budget(&shared, RELOADED).await;
        assert_eq!(
            shared.get().response_tokens(),
            CARRIED_RESPONSE_TOKENS,
            "生成用の余白は引き直しで動かさないこと（既定値へ潰さないこと）"
        );

        drop(task);
    }

    /// **`PendingRefresh` が返す器は、走り出したタスクが書く先そのもの。**
    ///
    /// ここが無いと、[`PendingRefresh::spawn_every`] の中で
    /// `SharedBudget::new(self.budget.get())` を渡して**捨て器へ書かせ、
    /// ハンドラには元の器を返す**配線が書ける（`SharedBudget::new` は
    /// このモジュールの中からは呼べるので、private 化では塞げない）。
    /// その壊れ方は **WARN を正しく出しながらハンドラは古い予算で組み続ける**
    /// ——ADR-0020 が塞ぐと言っている失敗そのもの。
    ///
    /// **本番の入口（`spawn`）は 5 分の定数を渡すので、テストは待てない。**
    /// だから対応付けを `spawn_every` へ落とし、ここが短い間隔で通す。
    /// `spawn` に残るのは [`REQUERY_INTERVAL`] を渡す1点だけで、
    /// そこは `tests/server_context_refresh.rs` が `interval_seconds` で見張っている。
    #[tokio::test]
    async fn the_spawned_task_writes_into_the_returned_budget() {
        let server = upstream_answering(&[LOADED, RELOADED]).await;
        // **サブ側は照会先を持たせない。** 同じモックを2つの照会が叩くと、
        // `ChangingAnswers` の系列をどちらが消費したのかが決まらない。
        let pending = PendingRefresh::new(
            Chosen::measured(budget_of(LOADED)),
            Some(source_for(&server)),
            Chosen::measured(SubContext::FALLBACK),
            None,
            upstream_facts(),
            CheckAtStart::NONE,
        );

        let (task, shared) = pending.spawn_every(SHORT_INTERVAL);

        // **返ってきた器**が差し替わること（別の器へ書かれていたら永遠に届かない）。
        wait_for_budget(&shared, RELOADED).await;

        drop(task);
    }

    // ------------------------------------------------------------------
    // 上流の観測（作業単位 G-3b）。
    //
    // **引き直しは予算だけでなく「上流について観測できたこと」も書き替える。**
    // ここは `refresh_once` / `refresh_sub_once` を**直に**呼ぶ——
    // 往復の本数を数えるのに、背景タスクの間隔に依存させないため。
    // ------------------------------------------------------------------

    /// **引き直しは、読んだものを観測へ書き込む**（作業単位 G-3b）。
    ///
    /// # ⚠ ここが `loaded_context_length()` へ戻る変異を赤にする
    ///
    /// あちらは**同じ往復の一覧と読み手を捨てる**ので、
    /// `answered_by` は `None` に、突き合わせは全行「聞けなかった」に落ちる。
    /// **予算だけを見ていると、その変異は緑を通る**（長さは同じ往復で読めるため）。
    ///
    /// **始まりの状態は「一度も聞いていない」**なので、
    /// 「引き直しが書いた」と「最初からそうだった」が区別できる。
    #[tokio::test]
    async fn the_requery_writes_what_it_read_into_the_upstream_facts() {
        let server = upstream_reporting(&RELOADED.to_string()).await;
        let budget = SharedBudget::new(budget_of(LOADED));
        let upstream = SharedUpstream::new(upstream_facts());

        // 前提: 始まりは「答えなし」。
        let before = upstream.get();
        assert_eq!(before.main().answered_by(), None);
        assert_eq!(before.main().queried_context_tokens(), None);
        assert_eq!(
            before.main().measured().unanswered,
            before.main().measured().checked(),
            "前提: 始まりは全行「聞けなかった」であること"
        );

        refresh_once(&face_of(client_for(&server.uri())), &budget, &upstream).await;

        let after = upstream.get();
        assert_eq!(
            after.main().queried_context_tokens(),
            Some(RELOADED),
            "上流が申告した窓を書き込むこと"
        );
        assert_eq!(
            after.main().answered_by(),
            Some(AnsweredBy::NativeApi),
            "どの読み手が答えたかも書き込むこと（一覧と一緒に捨てない）"
        );
        assert_eq!(
            after.main().measured().unanswered,
            0,
            "一覧を読んだのだから「聞けなかった」は残らないこと"
        );
        // **本文の面の行を見ていること**（作業単位 G-3b のレビュー指摘）。
        //
        // **`> 0` では行数を一切見ていない。** 引き直しが渡す面を取り違えると、
        // **本文の面（4 行）と抽出の面（5 行）が入れ替わる**——
        // `MeasuredRole::Sub` の定数が1つあるので、行数だけが違う。
        // **`unanswered == 0` も `drifted_*` も入れ替えたまま成り立つ**ので、
        // **数を突き合わせる以外にこの取り違えを見る手段が無い。**
        let main_rows = check_face(ModelFace::Main, DEFAULT_MAIN_MODEL, None).len();
        assert!(
            main_rows > 0,
            "前提: 見る行があること（0 行なら上の主張は空振り）"
        );
        assert_eq!(
            after.main().measured().checked(),
            main_rows,
            "本文の面の行を見ていること（抽出の面の行数と取り違えない）"
        );
        assert_eq!(
            after.main().endpoint(),
            before.main().endpoint(),
            "宛先は起動時に決まった事実であって、引き直しでは動かないこと"
        );
        assert_eq!(after.sub(), before.sub(), "反対の面は触らないこと");
    }

    // ------------------------------------------------------------------
    // 走ったままの差し替え（ADR-0042 §決定4 / 作業単位 F-4b-2）
    // ------------------------------------------------------------------

    /// **差し替え口は、次の往復から新しいクライアントへ送る**（本文と抽出が持つ `dyn LlmClient`）。
    ///
    /// **2つの上流に同じ依頼を1本ずつ**——差し替える前は前の上流だけ、後は新しい上流だけ。
    #[tokio::test]
    async fn the_face_client_sends_the_next_request_to_the_replacement() {
        let completion = |model: &str| {
            ResponseTemplate::new(200).set_body_raw(
                format!(
                    r#"{{"id":"x","object":"chat.completion","created":1,"model":"{model}",
                        "choices":[{{"index":0,"message":{{"role":"assistant","content":"「はい」"}},
                                     "finish_reason":"stop"}}]}}"#
                ),
                "application/json",
            )
        };
        let before = MockServer::start().await;
        let after = MockServer::start().await;
        for (server, model) in [(&before, "planted-before"), (&after, "planted-after")] {
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(completion(model))
                .mount(server)
                .await;
        }
        let face = face_of(client_for(&before.uri()));
        let request = crate::llm::ChatRequest::new(vec![crate::llm::ChatMessage::user("ただいま")]);
        let dynamic: &dyn LlmClient = &face;

        dynamic
            .complete(&request)
            .await
            .expect("前の上流が答えること");
        face.replace(Candidate(Arc::new(client_for(&after.uri()))));
        dynamic
            .complete(&request)
            .await
            .expect("新しい上流が答えること");

        let count = |requests: Option<Vec<Request>>| requests.map_or(0, |list| list.len());
        assert_eq!(
            count(before.received_requests().await),
            1,
            "差し替えた後も前の上流へ送っている"
        );
        assert_eq!(
            count(after.received_requests().await),
            1,
            "差し替えた後の往復が新しい上流へ行っていない"
        );
    }

    /// **差し替え口は、数える口もいまのクライアントへ転送する**（ADR-0052 / 作業単位 B-60a）。
    ///
    /// 転送を落とすとトレイトの既定（「数えられない」）に落ち、**黙って今のバイト数のまま**になる。
    /// **2つの上流は違う数を返す**ので、差し替えの前後でどちらが数えたかまで見える
    /// （起動時のクライアントを握り続ける形も赤になる）。
    #[tokio::test]
    async fn the_face_client_forwards_counting_to_the_current_client() {
        let before = MockServer::start().await;
        let after = MockServer::start().await;
        for (server, tokens) in [(&before, "[1,2]"), (&after, "[1,2,3,4,5]")] {
            Mock::given(method("POST"))
                .and(path("/apply-template"))
                .respond_with(ResponseTemplate::new(200).set_body_raw(
                    r#"{"prompt":"<planted>ただいま</planted>"}"#,
                    "application/json",
                ))
                .mount(server)
                .await;
            Mock::given(method("POST"))
                .and(path("/tokenize"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(format!(r#"{{"tokens":{tokens}}}"#), "application/json"),
                )
                .mount(server)
                .await;
        }
        let face = face_of(client_for(&before.uri()));
        let request = crate::llm::ChatRequest::new(vec![crate::llm::ChatMessage::user("ただいま")]);
        let dynamic: &dyn LlmClient = &face;

        assert_eq!(
            dynamic.count_text("ただいま").await.ok(),
            Some(TokenCount::Counted(2)),
            "文字列ごとの口が転送されていない"
        );
        assert_eq!(
            dynamic.count_request(&request, true).await.ok(),
            Some(TokenCount::Counted(2)),
            "本文ごとの口が転送されていない"
        );

        face.replace(Candidate(Arc::new(client_for(&after.uri()))));
        assert_eq!(
            dynamic.count_text("ただいま").await.ok(),
            Some(TokenCount::Counted(5)),
            "差し替えた後も前の上流に数えさせている（文字列ごと）"
        );
        assert_eq!(
            dynamic.count_request(&request, false).await.ok(),
            Some(TokenCount::Counted(5)),
            "差し替えた後も前の上流に数えさせている（本文ごと）"
        );
    }

    /// **差し替え口は写しを返し、その写しは取った時点の上流と話し続ける**
    /// （ADR-0052 §推奨3 / 作業単位 B-60b）。
    ///
    /// | 変異 | 見え方 |
    /// |---|---|
    /// | `pinned` が `None` を返す（**上書きを落とす形は、必須メソッドなので型が止める**） | `expect` で落ちる |
    /// | 写しの代わりに差し替え口そのものの複製を返す | **差し替えの後、写しが後ろの上流に数えさせる** |
    ///
    /// **2つの上流は違う数を返す**ので、どちらが数えたかまで見える。
    /// 対照として、**差し替え口そのものは差し替えの後ろの上流へ行く**ことも見る
    /// （両方が前の上流のままなら、差し替えそのものが効いていないだけ）。
    #[tokio::test]
    async fn a_pinned_copy_keeps_talking_to_the_upstream_it_was_taken_from() {
        let before = MockServer::start().await;
        let after = MockServer::start().await;
        for (server, tokens) in [(&before, "[1,2]"), (&after, "[1,2,3,4,5]")] {
            Mock::given(method("POST"))
                .and(path("/tokenize"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(format!(r#"{{"tokens":{tokens}}}"#), "application/json"),
                )
                .mount(server)
                .await;
        }
        let face = face_of(client_for(&before.uri()));
        let dynamic: &dyn LlmClient = &face;

        let pinned = dynamic
            .pinned()
            .expect("差し替え口は写しを返すこと（None だと写しが固定されない）");
        face.replace(Candidate(Arc::new(client_for(&after.uri()))));

        assert_eq!(
            pinned.count_text("ただいま").await.ok(),
            Some(TokenCount::Counted(2)),
            "写しが、取った後の差し替えに付いていっている"
        );
        assert_eq!(
            dynamic.count_text("ただいま").await.ok(),
            Some(TokenCount::Counted(5)),
            "前提: 差し替え口そのものは差し替えの後ろの上流へ行くこと"
        );
    }

    /// **面の世代は、差し替えと「答えた名前の変化」で進み、それ以外では進まない**
    /// （[`FaceGeneration`] / 作業単位 B-60b）。
    ///
    /// | 出来事 | 本文の面の世代 |
    /// |---|---|
    /// | 同じ名前のままの引き直し | **変わらない**（覚えた「数えられない」を捨てない） |
    /// | 抽出の面の差し替え・引き直し | **変わらない**（面ごとに別の世代） |
    /// | 答えた名前が変わる引き直し | **変わる** |
    /// | 本文の面の差し替え | **変わる** |
    #[test]
    fn a_face_generation_moves_on_a_switch_or_a_new_answered_name_only() {
        let upstream = SharedUpstream::new(upstream_facts());
        let named = |name: Option<&str>| UpstreamProbe {
            answered_name: name.map(str::to_owned),
            ..UpstreamProbe::unanswered()
        };

        let start = upstream.generation(ModelFace::Main);
        upstream.store_face(ModelFace::Main, &named(None));
        assert_eq!(
            upstream.generation(ModelFace::Main),
            start,
            "同じ名前のままの引き直しで世代を進めないこと"
        );

        upstream.store_face(ModelFace::Sub, &named(Some("sub-model.gguf")));
        upstream.replace_face(
            ModelFace::Sub,
            UpstreamObservation::unqueried(ModelFace::Sub, SUB_ENDPOINT_FOR_TESTS, SUB_MODEL_ID),
        );
        assert_eq!(
            upstream.generation(ModelFace::Main),
            start,
            "抽出の面の出来事で、本文の面の世代を進めないこと"
        );

        upstream.store_face(ModelFace::Main, &named(Some("main-model.gguf")));
        let renamed = upstream.generation(ModelFace::Main);
        assert_ne!(renamed, start, "答えた名前が変わったら世代を進めること");
        assert_eq!(
            renamed,
            FaceGeneration::for_tests(0, Some("main-model.gguf")),
            "版はまだ進んでいないこと（名前だけが動いた）"
        );

        upstream.replace_face(
            ModelFace::Main,
            UpstreamObservation::unqueried(
                ModelFace::Main,
                "http://127.0.0.1:19010/v1/chat/completions",
                "planted-switched-model",
            ),
        );
        assert_eq!(
            upstream.generation(ModelFace::Main),
            FaceGeneration::for_tests(1, None),
            "差し替えたら版が進むこと"
        );
    }

    /// **引き直しの最中に面が差し替わったら、その答えは書かない**（[`HeldFacts`] の doc）。
    ///
    /// # 競走しない作り方
    ///
    /// **上流の答えを遅らせ、照会が届いたのを見てから差し替える**
    /// ——その時点で引き直しは「版を読んだ後・書く前」に居る。
    ///
    /// **書けば、前の上流の窓（`RELOADED`）が新しい宛先の名前で観測に載り、予算まで動く。**
    #[tokio::test]
    async fn a_requery_that_raced_a_switch_writes_nothing() {
        let server = upstream_serving(
            ResponseTemplate::new(200)
                .set_body_raw(models_body(&RELOADED.to_string()), "application/json")
                .set_delay(Duration::from_millis(500)),
        )
        .await;
        let budget = SharedBudget::new(budget_of(LOADED));
        let upstream = SharedUpstream::new(upstream_facts());
        let source = face_of(client_for(&server.uri()));

        let refresh = {
            let (source, budget, upstream) = (source.clone(), budget.clone(), upstream.clone());
            tokio::spawn(async move { refresh_once(&source, &budget, &upstream).await })
        };
        wait_for_queries(&server, 1).await;

        // **差し替え**（新しい宛先の「一度も聞いていない」に置き換え、版を進める）。
        let switched = UpstreamObservation::unqueried(
            ModelFace::Main,
            "http://127.0.0.1:19009/v1/chat/completions",
            "planted-switched-model",
        );
        upstream.replace_face(ModelFace::Main, switched.clone());

        refresh.await.expect("引き直しが落ちていないこと");

        assert_eq!(
            upstream.get().main(),
            &switched,
            "差し替えより前に始まった照会の答えを書いている"
        );
        assert_eq!(
            budget.get().context_tokens(),
            LOADED,
            "差し替えより前に始まった照会の窓で予算を動かしている"
        );
    }

    /// **抽出の面も同じ**（引き直しは面ごとに別の関数を通る / [`refresh_sub_once`]）。
    ///
    /// **本文の腕だけでは、抽出の側の版の検査を外す変異が緑を通る**（変異検査で確かめた形）。
    #[tokio::test]
    async fn a_sub_requery_that_raced_a_switch_writes_nothing() {
        let server = upstream_serving(
            ResponseTemplate::new(200)
                .set_body_raw(
                    two_model_body(MAIN_BESIDE_SUB, SUB_RELOADED),
                    "application/json",
                )
                .set_delay(Duration::from_millis(500)),
        )
        .await;
        let sub_context =
            SharedSubContext::new(SubContext::new(SUB_LOADED).expect("使える窓であること"));
        let upstream = SharedUpstream::new(upstream_facts());
        let source = face_of(sub_client_for(&server.uri()));

        let refresh = {
            let (source, sub_context, upstream) =
                (source.clone(), sub_context.clone(), upstream.clone());
            tokio::spawn(async move { refresh_sub_once(&source, &sub_context, &upstream).await })
        };
        wait_for_queries(&server, 1).await;

        let switched = UpstreamObservation::unqueried(
            ModelFace::Sub,
            "http://127.0.0.1:19011/v1/chat/completions",
            "planted-switched-sub-model",
        );
        upstream.replace_face(ModelFace::Sub, switched.clone());

        refresh.await.expect("引き直しが落ちていないこと");

        assert_eq!(
            upstream.get().sub(),
            &switched,
            "差し替えより前に始まった照会の答えを書いている"
        );
        assert_eq!(
            sub_context.get().tokens(),
            SUB_LOADED,
            "差し替えより前に始まった照会の窓で抽出の上限を動かしている"
        );
    }

    /// **対照: 差し替えが無ければ、同じ遅い照会は書く**（上の腕が「何も書かない実装」で通らないように）。
    #[tokio::test]
    async fn a_slow_requery_without_a_switch_still_writes() {
        let server = upstream_serving(
            ResponseTemplate::new(200)
                .set_body_raw(models_body(&RELOADED.to_string()), "application/json")
                .set_delay(Duration::from_millis(500)),
        )
        .await;
        let budget = SharedBudget::new(budget_of(LOADED));
        let upstream = SharedUpstream::new(upstream_facts());

        refresh_once(&face_of(client_for(&server.uri())), &budget, &upstream).await;

        assert_eq!(
            upstream.get().main().queried_context_tokens(),
            Some(RELOADED)
        );
        assert_eq!(budget.get().context_tokens(), RELOADED);
    }

    /// **差し替えた面は「まだ何も聞いていない」に戻り、反対の面は触らない**
    /// （[`install_face`] / ADR-0042 §決定4「古い事実を残さない」）。
    ///
    /// | 器 | 本文を差し替えたとき |
    /// |---|---|
    /// | 本文の観測 | **新しい宛先と綴りの「一度も聞いていない」** |
    /// | 本文の予算 | **切り替え先の明示**（面の値 ＞ 起動時の環境変数）、**無ければ既定値** |
    /// | 抽出の観測と上限 | **そのまま** |
    ///
    /// **本文の予算は3つの腕で見る**（作業単位 A-13a-1 / ADR-0054 §決定2・3）——
    /// 明示なし・環境変数だけ・両方（面の値が勝つ）。**値は3つとも違え、前の予算（`LOADED`）とも違える**
    /// ——「触らない」実装も「環境変数を見ない」実装も「面の値を見ない」実装も、どれかの腕で赤になる。
    /// **「明示か」も一緒に見る**（値が合っていても印が無ければ、次の引き直しで照会の答えに潰される）。
    #[tokio::test]
    async fn installing_a_face_forgets_what_the_previous_upstream_said() {
        /// 起動時の環境変数として渡す値。
        const ENVIRONMENT: u32 = 20_480;
        /// 切り替え先の面に書いた値。
        const PROFILE: u32 = 28_672;
        let previous = upstream_reporting(&RELOADED.to_string()).await;
        let upstream = SharedUpstream::new(upstream_facts());
        // **前の上流の答えを、両面に書いておく**（消えたことが見える状態から始める）。
        refresh_once(
            &face_of(client_for(&previous.uri())),
            &SharedBudget::new(budget_of(LOADED)),
            &upstream,
        )
        .await;
        upstream.store_face(ModelFace::Sub, &upstream_probe_answering(SUB_LOADED));
        let sub_before = upstream.get().sub().clone();
        assert_eq!(
            upstream.get().main().queried_context_tokens(),
            Some(RELOADED),
            "前提: 前の上流の窓が載っていること"
        );

        let default_tokens = TokenBudget::default().context_tokens();
        for (environment, profile, want_tokens, want_explicit) in [
            (None, None, default_tokens, None),
            (Some(ENVIRONMENT), None, ENVIRONMENT, Some(ENVIRONMENT)),
            (Some(ENVIRONMENT), Some(PROFILE), PROFILE, Some(PROFILE)),
        ] {
            let explicit = (environment, profile);
            let budget = SharedBudget::new(budget_of(LOADED));
            let sub_context =
                SharedSubContext::new(SubContext::new(SUB_LOADED).expect("使える窓であること"));
            let clients = FaceClients::new(
                Arc::new(client_for(&previous.uri())),
                Arc::new(sub_client_for(&previous.uri())),
            );
            let next = face_of(client_for("http://127.0.0.1:19010")).as_candidate();
            let next = next.with_model("planted-next-model").expect("正当な綴り");

            install_face(
                &clients,
                ModelFace::Main,
                next,
                &budget,
                &sub_context,
                &upstream,
                environment.map(budget_of),
                super::super::config::ProfileContext {
                    main: profile.map(budget_of),
                    sub: None,
                    body: BodyFace::default(),
                },
            );

            let main = upstream.get().main().clone();
            assert_eq!(
                main,
                UpstreamObservation::unqueried(
                    ModelFace::Main,
                    "http://127.0.0.1:19010/v1/chat/completions",
                    "planted-next-model",
                ),
                "前の上流の事実が残っている（明示 = {explicit:?}）"
            );
            assert_eq!(
                clients.face(ModelFace::Main).model(),
                "planted-next-model",
                "差し替え口の中身が入れ替わっていない"
            );
            assert_eq!(
                (budget.get().context_tokens(), budget.explicit_tokens()),
                (want_tokens, want_explicit),
                "予算の戻し方が違う（環境変数・面の値 = {explicit:?}）"
            );
            // **反対の面は触らない。**
            assert_eq!(upstream.get().sub(), &sub_before);
            assert_eq!(sub_context.get().tokens(), SUB_LOADED);
            assert_eq!(
                clients.face(ModelFace::Sub).model(),
                SUB_MODEL_ID,
                "抽出の差し替え口まで触っている"
            );
        }

        // **抽出の面を差し替えたら、抽出の上限を落とし所へ戻す。本文の予算と観測は触らない。**
        let main_before = upstream.get().main().clone();
        let budget = SharedBudget::new(budget_of(LOADED));
        let sub_context =
            SharedSubContext::new(SubContext::new(SUB_LOADED).expect("使える窓であること"));
        assert_ne!(
            SUB_LOADED,
            SubContext::FALLBACK.tokens(),
            "前提: 落とし所と別の窓から始めること"
        );
        let clients = FaceClients::new(
            Arc::new(client_for(&previous.uri())),
            Arc::new(sub_client_for(&previous.uri())),
        );
        let next = face_of(client_for("http://127.0.0.1:19012"))
            .as_candidate()
            .with_model("planted-next-sub-model")
            .expect("正当な綴り");

        install_face(
            &clients,
            ModelFace::Sub,
            next,
            &budget,
            &sub_context,
            &upstream,
            None,
            super::super::config::ProfileContext::default(),
        );

        assert_eq!(
            upstream.get().sub(),
            &UpstreamObservation::unqueried(
                ModelFace::Sub,
                "http://127.0.0.1:19012/v1/chat/completions",
                "planted-next-sub-model",
            ),
            "抽出の面に前の上流の事実が残っている"
        );
        assert_eq!(
            sub_context.get(),
            SubContext::FALLBACK,
            "抽出の上限を落とし所へ戻していない"
        );
        // **差し替え口の帰属**（レビュー1周目の是正）——候補は抽出の口に入り、本文の口には入らない。
        // **観測の帰属（上の2つ）とは別の主張**：`install_face` が口を取り違えても、
        // 観測は正しい面へ書かれるので、上の assert は緑のまま通る。
        assert_eq!(
            clients.face(ModelFace::Sub).model(),
            "planted-next-sub-model",
            "抽出の差し替え口の中身が入れ替わっていない"
        );
        assert_ne!(
            clients.face(ModelFace::Main).model(),
            "planted-next-sub-model",
            "抽出の候補が本文の差し替え口へ入っている"
        );
        assert_eq!(
            upstream.get().main(),
            &main_before,
            "本文の観測まで触っている"
        );
        assert_eq!(
            budget.get().context_tokens(),
            LOADED,
            "本文の予算まで触っている"
        );
    }

    /// **観測を書いた後に差し替えが入っても、予算は前の上流の窓に戻らない**
    /// （[`SharedUpstream::store_face_since`] の doc / レビュー1周目の是正）。
    ///
    /// **引き直しが観測を書いたのを見てから差し替える。** 版の検査・観測・予算の採用が
    /// 1つの錠の中なら、観測が見えた時点で予算も書き終わっているので、
    /// **差し替えの落とし所がそのまま残る。**
    ///
    /// # ⚠ 何を捕まえ、何を捕まえないか（**測った**）
    ///
    /// **捕まえるのは「観測と予算の間に待ち点がある」形だけ**（予算の採用を錠の外へ出し、
    /// 間で `await` する——レビュアーの `w11`、この回の `m21`）。
    /// **錠の外へ出すだけで間に `await` が無い形（`m20`）は緑を通る**——
    /// 観測と予算の間は数命令で、この形のテストでは挟めない（捕まえる別の形はある。`store_face_since` の doc）。
    /// **それを止めているのは `store_face_since` の doc だけである。**
    #[tokio::test]
    async fn a_switch_after_the_observation_leaves_the_fallback_budget() {
        let server = upstream_reporting(&RELOADED.to_string()).await;
        let budget = SharedBudget::new(budget_of(LOADED));
        let sub_context =
            SharedSubContext::new(SubContext::new(SUB_LOADED).expect("使える窓であること"));
        let upstream = SharedUpstream::new(upstream_facts());
        let clients = FaceClients::new(
            Arc::new(client_for(&server.uri())),
            Arc::new(sub_client_for(&server.uri())),
        );
        let source = clients.face(ModelFace::Main).clone();
        let refresh = {
            let (source, budget, upstream) = (source.clone(), budget.clone(), upstream.clone());
            tokio::spawn(async move { refresh_once(&source, &budget, &upstream).await })
        };
        wait_for_observed_main(&upstream, RELOADED).await;

        let next = face_of(client_for("http://127.0.0.1:19013"))
            .as_candidate()
            .with_model("planted-next-model")
            .expect("正当な綴り");
        install_face(
            &clients,
            ModelFace::Main,
            next,
            &budget,
            &sub_context,
            &upstream,
            None,
            super::super::config::ProfileContext::default(),
        );
        refresh.await.expect("引き直しが落ちていないこと");

        assert_eq!(
            budget.get().context_tokens(),
            TokenBudget::default().context_tokens(),
            "差し替えの後に、前の上流の窓（{RELOADED}）で予算が書かれた"
        );
    }

    /// **抽出の面も同じ**（上限の採用が観測と同じ錠の中にあること）。
    #[tokio::test]
    async fn a_switch_after_the_sub_observation_leaves_the_fallback_limit() {
        let server = upstream_reporting_two_models(MAIN_BESIDE_SUB, SUB_RELOADED).await;
        let budget = SharedBudget::new(budget_of(LOADED));
        let sub_context =
            SharedSubContext::new(SubContext::new(SUB_LOADED).expect("使える窓であること"));
        let upstream = SharedUpstream::new(upstream_facts());
        let clients = FaceClients::new(
            Arc::new(client_for(&server.uri())),
            Arc::new(sub_client_for(&server.uri())),
        );
        let source = clients.face(ModelFace::Sub).clone();
        let refresh = {
            let (source, sub_context, upstream) =
                (source.clone(), sub_context.clone(), upstream.clone());
            tokio::spawn(async move { refresh_sub_once(&source, &sub_context, &upstream).await })
        };
        let mut observed = false;
        for _ in 0..400 {
            if upstream.get().sub().queried_context_tokens() == Some(SUB_RELOADED) {
                observed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(observed, "前提: 引き直しが抽出の観測を書いたこと");

        let next = face_of(client_for("http://127.0.0.1:19014"))
            .as_candidate()
            .with_model("planted-next-sub-model")
            .expect("正当な綴り");
        install_face(
            &clients,
            ModelFace::Sub,
            next,
            &budget,
            &sub_context,
            &upstream,
            None,
            super::super::config::ProfileContext::default(),
        );
        refresh.await.expect("引き直しが落ちていないこと");

        assert_eq!(
            sub_context.get(),
            SubContext::FALLBACK,
            "差し替えの後に、前の上流の窓（{SUB_RELOADED}）で抽出の上限が書かれた"
        );
    }

    /// 本文の観測が `tokens` を名乗るまで待つ（**上限つき**）。
    async fn wait_for_observed_main(upstream: &SharedUpstream, tokens: u32) {
        for _ in 0..400 {
            if upstream.get().main().queried_context_tokens() == Some(tokens) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("前提: 引き直しが本文の観測を書いたこと（{tokens}）");
    }

    /// 照会が通った回の観測（窓だけを持つ）。
    fn upstream_probe_answering(tokens: u32) -> UpstreamProbe {
        UpstreamProbe {
            catalogue: Some(vec![SUB_MODEL_ID.to_owned()]),
            context_tokens: Some(tokens),
            answered_by: Some(AnsweredBy::NativeApi),
            answered_name: Some(SUB_MODEL_ID.to_owned()),
            unauthorized: false,
            key_over_plain_http: None,
            check: Some(UpstreamCheck::now(CheckOutcome::Answered)),
        }
    }

    /// **抽出の側の引き直しも、読んだものを観測へ書き込む**（作業単位 G-3b）。
    ///
    /// # ⚠ この1本は、変異検査が空きを見つけて足したもの
    ///
    /// **本文の側だけを見ていたとき、`refresh_sub_once` を
    /// `loaded_context_length()` へ戻す変異が 1989 全緑を通った**（変異 `m7`）。
    /// **本文の側の同じ変異は 4 赤**だったので、「守られている」と読み違えやすい形。
    /// **写しを作ったら、面の1つずつに当てること**（CLAUDE.md / `D-6`）。
    #[tokio::test]
    async fn the_sub_requery_writes_what_it_read_into_the_upstream_facts() {
        let server = upstream_reporting_two_models(MAIN_BESIDE_SUB, SUB_LOADED).await;
        let sub_context = SharedSubContext::new(SubContext::FALLBACK);
        let upstream = SharedUpstream::new(upstream_facts());

        let before = upstream.get();
        assert_eq!(
            before.sub().answered_by(),
            None,
            "前提: 始まりは「答えなし」"
        );

        refresh_sub_once(
            &face_of(sub_client_for(&server.uri())),
            &sub_context,
            &upstream,
        )
        .await;

        let after = upstream.get();
        assert_eq!(
            after.sub().queried_context_tokens(),
            Some(SUB_LOADED),
            "抽出の上流が申告した窓を書き込むこと"
        );
        assert_eq!(
            after.sub().answered_by(),
            Some(AnsweredBy::NativeApi),
            "どの読み手が答えたかも書き込むこと（一覧と一緒に捨てない）"
        );
        assert_eq!(
            after.sub().measured().unanswered,
            0,
            "一覧を読んだのだから「聞けなかった」は残らないこと"
        );
        // **抽出の面の行を見ていること**（作業単位 G-3b のレビュー指摘）。
        // 理由は本文側の同じ箇所に書いてある。**面ごとに1本ずつ置くこと**
        // （CLAUDE.md / `D-6`——写しを作ったら列・分岐の1つずつに当てる）。
        let sub_rows = check_face(ModelFace::Sub, SUB_MODEL_ID, None).len();
        assert!(
            sub_rows > 0,
            "前提: 見る行があること（0 行なら上の主張は空振り）"
        );
        assert_eq!(
            after.sub().measured().checked(),
            sub_rows,
            "抽出の面の行を見ていること（本文の面の行数と取り違えない）"
        );
        assert_eq!(after.main(), before.main(), "反対の面は触らないこと");
    }

    /// **抽出の側も、1回の往復から全部読む**（作業単位 G-3b）。
    #[tokio::test]
    async fn the_sub_requery_reads_everything_from_one_round_trip() {
        let server = upstream_reporting_two_models(MAIN_BESIDE_SUB, SUB_LOADED).await;
        let sub_context = SharedSubContext::new(SubContext::FALLBACK);
        let upstream = SharedUpstream::new(upstream_facts());

        assert_eq!(queries_to(&server).await, 0, "前提: まだ引いていないこと");

        refresh_sub_once(
            &face_of(sub_client_for(&server.uri())),
            &sub_context,
            &upstream,
        )
        .await;

        assert_eq!(
            queries_to(&server).await,
            1,
            "1回の引き直しで、モデル一覧の照会は1回だけであること"
        );
        assert_eq!(sub_context.get().tokens(), SUB_LOADED, "長さ");
        assert_eq!(
            upstream.get().sub().answered_by(),
            Some(AnsweredBy::NativeApi),
            "読み手"
        );
        assert_eq!(upstream.get().sub().measured().unanswered, 0, "一覧");
    }

    /// **観測に必要なものは、1回の往復から全部読む**（作業単位 G-3b）。
    ///
    /// **往復を1本足す変異**（長さと一覧を別々に引く形／
    /// `loaded_context_length()` の後で `models()` を引く形）を赤にする。
    ///
    /// **これが「往復は増えていない」の実測そのもの**
    /// ——モックサーバに届いた `/api/v0/models` の本数を数える。
    #[tokio::test]
    async fn the_requery_reads_everything_from_one_round_trip() {
        let server = upstream_reporting(&RELOADED.to_string()).await;
        let budget = SharedBudget::new(budget_of(LOADED));
        let upstream = SharedUpstream::new(upstream_facts());

        assert_eq!(queries_to(&server).await, 0, "前提: まだ引いていないこと");

        refresh_once(&face_of(client_for(&server.uri())), &budget, &upstream).await;

        assert_eq!(
            queries_to(&server).await,
            1,
            "1回の引き直しで、モデル一覧の照会は1回だけであること"
        );
        // **その1回で3つとも取れていること**（取れていなければ「1回」に意味が無い）。
        let facts = upstream.get();
        assert_eq!(budget.get().context_tokens(), RELOADED, "長さ");
        assert_eq!(
            facts.main().answered_by(),
            Some(AnsweredBy::NativeApi),
            "読み手"
        );
        assert_eq!(facts.main().measured().unanswered, 0, "一覧");
    }

    /// **答えなかった回は「答えなし」として残る。予算は現在値を保つ**（作業単位 G-3b）。
    ///
    /// **この食い違いだけが「上流がいま答えない」を外から見せる。**
    /// 観測まで現在値を保つと、上流が停止しても何も変わって見えない。
    #[tokio::test]
    async fn a_failing_requery_records_that_the_upstream_did_not_answer() {
        let alive = upstream_reporting(&RELOADED.to_string()).await;
        let budget = SharedBudget::new(budget_of(LOADED));
        let upstream = SharedUpstream::new(upstream_facts());

        // まず一度は答えさせる（「最初から答えなしだった」と区別するため）。
        refresh_once(&face_of(client_for(&alive.uri())), &budget, &upstream).await;
        assert!(
            upstream.get().main().answered_by().is_some(),
            "前提: 一度は答えていること"
        );
        assert_eq!(budget.get().context_tokens(), RELOADED, "前提: 採用済み");

        // 次は届かない相手へ引く。
        let closed = closed_address().await;
        refresh_once(&face_of(client_for(&closed)), &budget, &upstream).await;

        assert_eq!(
            budget.get().context_tokens(),
            RELOADED,
            "予算は現在値を保つこと（一瞬詰まっただけで縮めない）"
        );
        let facts = upstream.get();
        assert_eq!(
            facts.main().answered_by(),
            None,
            "観測のほうは「答えなし」になること"
        );
        assert_eq!(
            facts.main().queried_context_tokens(),
            None,
            "申告値も「読めなかった」になること（採用値を写さない）"
        );
        assert_eq!(
            facts.main().measured().unanswered,
            facts.main().measured().checked(),
            "一覧が無いのだから、突き合わせは全行「聞けなかった」であること"
        );
    }

    /// **本文の観測と抽出の観測を取り違えない**（帰属 / 作業単位 G-3b）。
    ///
    /// **同じ上流が2つのモデルを別々の窓で申告する**ので、
    /// **面を入れ替えると値が入れ替わる。**
    /// **`assert_ne!` では防げない形**（値は全部違っていて、組み合わせだけが間違っている /
    /// CLAUDE.md「抜けるのはいつも帰属」）。
    #[tokio::test]
    async fn the_two_faces_of_the_observation_are_not_swapped() {
        let server = upstream_reporting_two_models(MAIN_BESIDE_SUB, SUB_LOADED).await;
        let budget = SharedBudget::new(budget_of(LOADED));
        let sub_context = SharedSubContext::new(SubContext::FALLBACK);
        let upstream = SharedUpstream::new(upstream_facts());

        refresh_once(&face_of(client_for(&server.uri())), &budget, &upstream).await;
        refresh_sub_once(
            &face_of(sub_client_for(&server.uri())),
            &sub_context,
            &upstream,
        )
        .await;

        let facts = upstream.get();
        assert_eq!(
            facts.main().queried_context_tokens(),
            Some(MAIN_BESIDE_SUB),
            "本文の面には本文モデルの窓が入ること"
        );
        assert_eq!(
            facts.sub().queried_context_tokens(),
            Some(SUB_LOADED),
            "抽出の面には抽出モデルの窓が入ること"
        );
        assert_eq!(facts.main().model(), DEFAULT_MAIN_MODEL);
        assert_eq!(facts.sub().model(), SUB_MODEL_ID);
        assert_ne!(
            facts.main().measured(),
            facts.sub().measured(),
            "面ごとに見る行数が違うこと（同値だと内訳の取り違えを検出できない）"
        );
    }

    /// **`PendingRefresh` が渡す観測の器は、走り出したタスクが書く先そのもの。**
    ///
    /// ハンドラは `bind` の時点でこの器を受け取る（`serve` まで待てない）ので、
    /// **走り出す前に渡した器と、走り出したタスクが書く器が同じ**でなければ、
    /// **引き直しは正しく働きながら、管理 API だけが起動時の値を返し続ける。**
    /// （`the_spawned_task_writes_into_the_handed_out_sub_context` の観測版）
    #[tokio::test]
    async fn the_spawned_task_writes_into_the_handed_out_upstream_facts() {
        let server = upstream_answering(&[RELOADED]).await;
        let pending = PendingRefresh::new(
            Chosen::measured(budget_of(LOADED)),
            Some(source_for(&server)),
            Chosen::measured(SubContext::FALLBACK),
            None,
            upstream_facts(),
            CheckAtStart::NONE,
        );

        // **走り出す前に受け取る**（`App::bind_with` と同じ順序）。
        let shared = pending.upstream();
        assert_eq!(
            shared.get().main().answered_by(),
            None,
            "前提: 走り出す前は「答えなし」であること"
        );

        let (task, _budget) = pending.spawn_every(SHORT_INTERVAL);

        for _ in 0..(WAIT_LIMIT.as_millis() / POLL_STEP.as_millis()) {
            if shared.get().main().answered_by().is_some() {
                drop(task);
                return;
            }
            tokio::time::sleep(POLL_STEP).await;
        }
        panic!("渡した器へ引き直しの結果が届かない（別の器へ書いている）");
    }

    /// **間隔が経つまでは1回も照会しない**（応答の経路に往復を足さないための土台）。
    ///
    /// 前提（「この仕掛けで照会は現に飛ぶ」）は
    /// [`a_changed_upstream_value_replaces_the_shared_budget`] が担保している。
    #[tokio::test]
    async fn nothing_is_queried_before_the_interval_elapses() {
        let server = upstream_answering(&[RELOADED]).await;
        let shared = SharedBudget::new(budget_of(LOADED));

        let task =
            spawn_main_refresh_every(Some(source_for(&server)), shared.clone(), LONG_INTERVAL);

        // 短い間に何度も観測しても、60秒は経たないので照会は起きない。
        for _ in 0..20 {
            tokio::time::sleep(POLL_STEP).await;
            assert_eq!(
                queries_to(&server).await,
                0,
                "間隔（{LONG_INTERVAL:?}）を待たずに照会しないこと"
            );
        }
        assert_eq!(
            shared.get().context_tokens(),
            LOADED,
            "照会していないのに予算が動かないこと"
        );

        drop(task);
    }

    /// **明示の間も間隔ごとに照会し、観測だけを書く。予算は採らない。明示が外れたら、同じタスクが引き直し始める**
    /// （ADR-0020 §4 とその追記 / ADR-0054 §決定3 / 作業単位 A-13a-1・B-66）。
    ///
    /// > **⚠ この見張り番の主張は作業単位 B-66 で変わった**（司令塔・利用者の決定 / Issue origin#94）。
    /// > **着手前の名前は `an_explicit_setting_is_never_requeried_until_it_is_lifted`** で、
    /// > 「明示の間は1回も照会しない」を見ていた。**いまは「照会する（観測は動く）が、予算は動かない」を見る。**
    /// > ADR-0020 §4 が塞いだのは「予算は変わらないのに『変わった』と warn する記録」で、
    /// > warn が出ないことは `checking_an_explicit_face_never_says_the_length_changed` が見る。
    ///
    /// # 照会先は持ったまま、器の「明示か」を毎回読む
    ///
    /// **F-4b-2 までは、明示なら照会先そのものを渡さなかった**（`refresh_source`）。
    /// それだと走ったまま値の無いプロファイルへ切り替えたときに戻れない。
    /// **終段は「切り替えが明示を外した」と同じこと（[`SharedBudget::install`] で照会した値に戻す）を起こし、
    /// 同じタスクが次の間隔から引き直して上流の値（`RELOADED`）になることを見る**
    /// ——起動時に「明示か」を写し取る実装は、`CONFIGURED` のまま止まって赤になる。
    #[tokio::test]
    async fn an_explicit_setting_is_checked_but_never_adopted_until_it_is_lifted() {
        let server = upstream_answering(&[RELOADED]).await;
        let shared = SharedBudget::chosen(Chosen::explicit(budget_of(CONFIGURED)));
        let upstream = SharedUpstream::new(upstream_facts());
        let task = spawn_main_refresh_into(
            Some(source_for(&server)),
            shared.clone(),
            upstream.clone(),
            SHORT_INTERVAL,
        );

        // **間隔ごとに照会する**（2回以上 —— 1回で止まる形も赤にする）。
        wait_for_queries(&server, 2).await;
        let observed = upstream.get();
        assert_eq!(
            (
                observed.main().queried_context_tokens(),
                observed.main().check().map(|check| check.outcome()),
            ),
            (Some(RELOADED), Some(CheckOutcome::Answered)),
            "明示の面の観測に、照会の答えが載ること"
        );
        assert_eq!(
            (shared.get().context_tokens(), shared.explicit_tokens()),
            (CONFIGURED, Some(CONFIGURED)),
            "明示設定の値を、照会の答え（{RELOADED}）で動かした"
        );

        // 終段: 明示が外れたら（切り替え）、同じタスクが引き直し始める。
        shared.install(Chosen::measured(budget_of(CONFIGURED)), BodyFace::default());
        wait_for_budget(&shared, RELOADED).await;

        drop(task);
    }

    /// **抽出の側も同じ**（上流プロファイルの面に書いた値 / ADR-0054 §決定1・3 / 作業単位 B-66）。
    ///
    /// **明示の間も照会し、観測だけを書く。外れたら、同じタスクが引き直し始める。**
    ///
    /// > **⚠ 作業単位 B-66 で主張が変わった。** 着手前の名前は
    /// > `an_explicit_sub_context_is_never_requeried_until_it_is_lifted`（「1回も照会しない」）。
    #[tokio::test]
    async fn an_explicit_sub_context_is_checked_but_never_adopted_until_it_is_lifted() {
        let server = upstream_reporting_two_models(MAIN_BESIDE_SUB, SUB_RELOADED).await;
        let written = SubContext::new(SUB_LOADED).expect("使える窓であること");
        let shared = SharedSubContext::chosen(Chosen::explicit(written));
        let upstream = SharedUpstream::new(upstream_facts());
        let task = spawn_sub_refresh_into(
            Some(sub_source_for(&server)),
            shared.clone(),
            upstream.clone(),
            SHORT_INTERVAL,
        );

        wait_for_queries(&server, 2).await;
        let observed = upstream.get();
        assert_eq!(
            (
                observed.sub().queried_context_tokens(),
                observed.sub().check().map(|check| check.outcome()),
            ),
            (Some(SUB_RELOADED), Some(CheckOutcome::Answered)),
            "抽出の明示の面の観測に、照会の答えが載ること"
        );
        assert_eq!(
            (shared.get().tokens(), shared.explicit_tokens()),
            (SUB_LOADED, Some(SUB_LOADED)),
            "明示の値を、照会の答え（{SUB_RELOADED}）で動かした"
        );

        shared.install(Chosen::measured(written));
        let deadline = Instant::now() + WAIT_LIMIT;
        while shared.get().tokens() != SUB_RELOADED {
            assert!(
                Instant::now() < deadline,
                "明示が外れても引き直さない（いま {}）",
                shared.get().tokens()
            );
            tokio::time::sleep(POLL_STEP).await;
        }

        drop(task);
    }

    /// **照会の最中に明示へ切り替わったら、その答えを採らない**（[`SharedBudget::adopt_window`] / 作業単位 A-13a-1）。
    ///
    /// **引き直しの入口の検査は「照会しない」ためのもの**で、照会が始まった後に明示へ変わる窓は
    /// 採用の口が同じ錠の中で塞ぐ。**遅い上流**（500 ms）への照会の最中に明示を入れ、
    /// 照会が終わった後も明示の値のままであることを見る。
    ///
    /// **版は進めていない**（クライアントを差し替えない明示の付け外しだけ）——
    /// 版の検査（`store_face_since`）では止まらない形を、ここで作っている。
    #[tokio::test]
    async fn a_requery_that_raced_an_explicit_value_does_not_overwrite_it() {
        let server = upstream_serving(
            ResponseTemplate::new(200)
                .set_body_raw(models_body(&RELOADED.to_string()), "application/json")
                .set_delay(Duration::from_millis(500)),
        )
        .await;
        let budget = SharedBudget::new(budget_of(LOADED));
        let upstream = SharedUpstream::new(upstream_facts());
        let refresh = {
            let (source, budget, upstream) = (
                face_of(client_for(&server.uri())),
                budget.clone(),
                upstream.clone(),
            );
            tokio::spawn(async move { refresh_once(&source, &budget, &upstream).await })
        };
        wait_for_queries(&server, 1).await;
        budget.install(Chosen::explicit(budget_of(CONFIGURED)), BodyFace::default());
        refresh.await.expect("引き直しが落ちていないこと");

        assert_eq!(
            upstream.get().main().queried_context_tokens(),
            Some(RELOADED),
            "前提: 照会は終わって観測は書かれていること"
        );
        assert_eq!(
            (budget.get().context_tokens(), budget.explicit_tokens()),
            (CONFIGURED, Some(CONFIGURED)),
            "照会の最中に入った明示の値を、照会の答え（{RELOADED}）で上書きした"
        );
    }

    /// **明示の面への照会は、「コンテキスト長が変わりました」を言わない**（ADR-0020 §4 / 作業単位 B-66）。
    ///
    /// 上流は書いた値（[`CONFIGURED`]）と**違う**長さ（[`RELOADED`]）を申告する。**同じ値だと
    /// 「変わっていないから黙った」と区別が付かない。** 観測には申告値が載ること（照会したこと）を
    /// 同じ回で見る。
    ///
    /// **対照**: 同じ上流・同じ1回の捕捉で、明示ではない器は「変わりました」を言う
    /// （文言を消しても否定側が緑になる形にしない）。
    #[test]
    fn checking_an_explicit_face_never_says_the_length_changed() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime を作れること");
        let capture = |explicit: bool| {
            captured_under_default_filter(|| {
                runtime.block_on(async {
                    let server = upstream_reporting(&RELOADED.to_string()).await;
                    let chosen = if explicit {
                        Chosen::explicit(budget_of(CONFIGURED))
                    } else {
                        Chosen::measured(budget_of(CONFIGURED))
                    };
                    let budget = SharedBudget::chosen(chosen);
                    let upstream = SharedUpstream::new(upstream_facts());
                    refresh_once(&source_for(&server), &budget, &upstream).await;
                    assert_eq!(
                        upstream.get().main().queried_context_tokens(),
                        Some(RELOADED),
                        "前提: 照会して観測を書いたこと（明示 = {explicit}）"
                    );
                    let expected = if explicit { CONFIGURED } else { RELOADED };
                    assert_eq!(
                        budget.get().context_tokens(),
                        expected,
                        "明示なら書いた値のまま、そうでなければ申告値へ（明示 = {explicit}）"
                    );
                });
            })
        };

        let measured = capture(false);
        assert!(
            measured.contains(CHANGED_MARKER),
            "対照: 明示ではない器は「変わりました」を言うこと: {measured:?}"
        );

        let explicit = capture(true);
        assert!(
            !explicit.contains(CHANGED_MARKER),
            "明示の面への照会で「変わりました」と言った: {explicit:?}"
        );
        assert!(
            !explicit.contains("WARN"),
            "明示の面への照会で警告を出した: {explicit:?}"
        );
    }

    /// **照会は「いつ・どう確かめたか」を観測に残す**（作業単位 B-66 / [`probe_upstream`]）。
    ///
    /// | 上流 | 結果 |
    /// |---|---|
    /// | 一覧に答える | `Answered` |
    /// | 鍵を断る（401） | `Refused` |
    /// | 届くが一覧が読めない（独自 API は 500・OpenAI 互換は 404） | `NoModelList`（レビュー是正） |
    /// | 落ちている | `NoAnswer` |
    /// | 鍵を平文で外へ送る承認が無い | `KeyNotSent`（**接続しない**） |
    ///
    /// **時刻は照会を読み終えた時刻**（照会の前後に挟まれること）。
    #[tokio::test]
    async fn a_check_says_when_and_how_the_upstream_answered() {
        let answering = upstream_reporting(&LOADED.to_string()).await;
        let refusing = upstream_serving(ResponseTemplate::new(401)).await;
        // **一覧の口だけが壊れている上流**（`/v1/models` は偽物の既定の 404）。
        let listless = upstream_serving(ResponseTemplate::new(500)).await;
        let closed = closed_address().await;
        // **ローカルネットワークの外の IP アドレス**（RFC 5737 の例示用）・鍵あり・承認なし
        // ——組んだときに「送らない」と決まり、接続しない（`llm::lmstudio` の `authorized`）。
        let withheld = {
            let config = LmStudioConfig::new("http://203.0.113.10:1234", model())
                .expect("ベース URL が正当であること")
                .with_api_key(Some(
                    crate::llm::ApiKey::parse("pc-b66-key-for-tests")
                        .expect("鍵の綴りが正当であること"),
                ));
            LmStudioClient::new(config).expect("HTTP クライアントを構築できること")
        };

        for (client, expected, what) in [
            (
                client_for(&answering.uri()),
                CheckOutcome::Answered,
                "答える上流",
            ),
            (
                client_for(&refusing.uri()),
                CheckOutcome::Refused,
                "鍵を断る上流",
            ),
            (
                client_for(&listless.uri()),
                CheckOutcome::NoModelList,
                "一覧が読めない上流",
            ),
            (
                client_for(&closed),
                CheckOutcome::NoAnswer,
                "落ちている上流",
            ),
            (withheld, CheckOutcome::KeyNotSent, "承認の無い外の宛先"),
        ] {
            let before = chrono::Utc::now();
            let (probe, _) = probe_upstream(&client).await;
            let after = chrono::Utc::now();
            let check = probe
                .check
                .unwrap_or_else(|| panic!("{what}: 照会した回は「確かめた」を残すこと"));
            assert_eq!(check.outcome(), expected, "{what}");
            assert!(
                before <= check.at() && check.at() <= after,
                "{what}: 時刻は照会の前後に挟まれること（{before} / {} / {after}）",
                check.at()
            );
        }
    }

    /// **起動の直後の1回は、頼んだ面だけを、間隔を待たずに照会する**（作業単位 B-66 / [`CheckAtStart`]）。
    ///
    /// 間隔は 60 秒（テストの間には経たない）。**本文の面だけを頼み、抽出の面は頼まない**
    /// ——頼んだ面は1回聞かれ、頼まなかった面は1度も聞かれない（取り違え・決め打ちの両方を赤にする）。
    /// **頼んだ面の予算は動かない**（明示の面）。
    #[tokio::test]
    async fn only_the_faces_asked_for_are_checked_as_soon_as_the_task_starts() {
        let main = upstream_answering(&[RELOADED]).await;
        let sub = upstream_reporting_two_models(MAIN_BESIDE_SUB, SUB_RELOADED).await;
        let pending = PendingRefresh::new(
            Chosen::explicit(budget_of(CONFIGURED)),
            Some(source_for(&main)),
            Chosen::explicit(SubContext::new(SUB_LOADED).expect("使える窓であること")),
            Some(sub_source_for(&sub)),
            upstream_facts(),
            CheckAtStart {
                main: true,
                sub: false,
            },
        );
        let observed = pending.upstream();
        let (task, shared) = pending.spawn_every(LONG_INTERVAL);

        wait_for_queries(&main, 1).await;
        // 照会が観測へ書かれるのを待つ（往復を読んだ後に書く）。
        let deadline = Instant::now() + WAIT_LIMIT;
        while observed.get().main().check().is_none() {
            assert!(Instant::now() < deadline, "頼んだ面の観測が書かれなかった");
            tokio::time::sleep(POLL_STEP).await;
        }
        tokio::time::sleep(SHORT_INTERVAL * 10).await;
        assert_eq!(
            (queries_to(&main).await, queries_to(&sub).await),
            (1, 0),
            "頼んだ本文の面だけを1回、頼まなかった抽出の面は聞かないこと"
        );
        assert_eq!(
            observed.get().sub().check(),
            None,
            "頼まなかった面は「照会していない」のままであること"
        );
        assert_eq!(
            shared.get().context_tokens(),
            CONFIGURED,
            "明示の予算を動かした"
        );

        drop(task);
    }

    /// **引き直しに失敗しても現在値を保つ。既定値へ落とさない**（ADR-0020 §5）。
    ///
    /// 落とすと、上流が一瞬詰まっただけで予算が黙って縮む。
    /// 現在値（`LOADED`）は既定値と別の数なので、落ちたかどうかを区別できる。
    #[tokio::test]
    async fn a_failing_requery_keeps_the_current_budget() {
        let server = upstream_serving(ResponseTemplate::new(500)).await;
        let shared = SharedBudget::new(budget_of(LOADED));

        let task =
            spawn_main_refresh_every(Some(source_for(&server)), shared.clone(), SHORT_INTERVAL);

        wait_for_queries(&server, 3).await;

        assert_eq!(
            shared.get().context_tokens(),
            LOADED,
            "失敗した引き直しで現在値を捨てないこと"
        );
        assert_ne!(
            shared.get(),
            TokenBudget::default(),
            "既定値へ落とさないこと（起動時とは違い、すでに手掛かりを持っている）"
        );

        drop(task);
    }

    /// **予算にならない申告値でも現在値を保つ。** ここも既定値へ落とさない。
    #[tokio::test]
    async fn a_requeried_value_that_cannot_become_a_budget_keeps_the_current_budget() {
        let server = upstream_answering(&[UNUSABLE]).await;
        let shared = SharedBudget::new(budget_of(LOADED));

        let task =
            spawn_main_refresh_every(Some(source_for(&server)), shared.clone(), SHORT_INTERVAL);

        wait_for_queries(&server, 3).await;

        assert_eq!(
            shared.get().context_tokens(),
            LOADED,
            "予算にならない値で現在値を捨てないこと"
        );
        assert_ne!(
            shared.get(),
            TokenBudget::default(),
            "既定値へ落とさないこと"
        );

        drop(task);
    }

    /// **タスクを落としたら引き直しも止まる**（ADR-0020 §3 の3つめ）。
    ///
    /// サーバが止まった後も背景タスクが残ると、上流を叩き続ける。
    #[tokio::test]
    async fn dropping_the_task_stops_the_requery() {
        let server = upstream_answering(&[LOADED]).await;
        let shared = SharedBudget::new(budget_of(LOADED));

        let task =
            spawn_main_refresh_every(Some(source_for(&server)), shared.clone(), SHORT_INTERVAL);
        wait_for_queries(&server, 2).await;
        drop(task);

        // 飛んでいる途中の1回が記録されるのを待ってから基準を取る。
        tokio::time::sleep(SHORT_INTERVAL * 5).await;
        let after_drop = queries_to(&server).await;

        // 30 間隔ぶん待っても増えない。
        tokio::time::sleep(SHORT_INTERVAL * 30).await;
        assert_eq!(
            queries_to(&server).await,
            after_drop,
            "落とした後は照会が増えないこと"
        );
    }

    /// **変わったら `warn`。新しい予算を返す**（ADR-0020 §5）。
    ///
    /// 生成用の余白は**本文の面の上限を引き継ぐ**（既定値に潰さない）。A-18a までは「いまの予算の余白」を
    /// 引き継いでいた —— 本番では同じ値なので、ここでは同じ値を面の上限として渡す（[`CARRIED_RESPONSE_TOKENS`] の doc）。
    #[test]
    fn a_changed_context_length_is_adopted_and_warned() {
        // 前提（余白が既定値と別であること）は `carried_budget` の入口が確かめる。
        let current = carried_budget(LOADED);

        let output = captured_under_default_filter(|| {
            let adopted =
                super::adopt_refreshed(current, Some(RELOADED), carried_body().response_room());
            assert_eq!(
                adopted,
                Some(carried_budget(RELOADED)),
                "申告値をそのままコンテキスト長にし、余白は引き継ぐこと"
            );
        });

        // **深刻度も2つの値も、変化を告げる行の中で見る**（作業単位 B-2）。
        // 前後の値が別の行に散っていたら、運用者は突き合わせられない。
        //
        // **⚠ いまこの捕捉に囮は無い**（B-2-2 / レビュー指摘 R-3）。
        // `adopt_refreshed` は**どの枝でも1件しか出さない**ので、
        // **2026-08-26 に実測しても1行だけ**だった。
        // **効くのは、前後の値を別のイベントへ割る変異が入ったとき**
        // ——出力全体で見る形はその2行から答えを拾って緑を通る。
        let changed = line_containing(&output, CHANGED_MARKER);
        assert!(
            changed.contains("WARN"),
            "深刻度は warn であること: {changed:?}"
        );
        // **⚠ 欄はトークンとして突き合わせること**（作業単位 B-12b）。
        //
        // **この行だけは `contains` が原理的に足りない。**
        // 同じ行に **`previous_context_tokens=` と `context_tokens=` が並ぶ**ので、
        // **後者を探す `contains` は前者の末尾に当たる。**
        // 実測（2026-08-27）: **新しい側の欄名に接頭辞を足す変異**
        // （`context_tokens` → `new_context_tokens`）は、
        // `contains` では **1122 件全緑**を通った——`new_context_tokens=4096` が
        // `context_tokens=4096` を**含む**ため。**`has_field` なら別のトークンなので赤。**
        //
        // **同じ形は「サブモデル側の双子」が先に正しく書いていた**
        // （`a_changed_sub_context_is_adopted_and_warned` は `has_field`）。
        // **作法はあった。こちらへ写されていなかっただけ。**
        assert!(
            has_field(changed, &format!("previous_context_tokens={LOADED}")),
            "変わる前の値を残すこと: {changed:?}"
        );
        assert!(
            has_field(changed, &format!("context_tokens={RELOADED}")),
            "変わった後の値を残すこと（`previous_` の末尾で満たされないこと）: {changed:?}"
        );
    }

    /// **変わっていなければ黙る。**
    ///
    /// 5分ごとに `warn` が出続けると、本当に変わった1行が埋もれる。
    ///
    /// # 否定の前に肯定を置く
    ///
    /// `warn!` ごと消えていても否定側は「何も出なかった」ので緑になる。
    /// 先に `trace` フィルタで**現に出せる**ことを確かめる。
    #[test]
    fn an_unchanged_context_length_is_not_warned() {
        let noisy = captured_with_filter(VERBOSE_FILTER, || {
            let _ = adopt_refreshed(budget_of(LOADED), Some(RELOADED));
        });
        assert!(
            noisy.contains(CHANGED_MARKER),
            "前提: 変わった側の文言は現に出せること: {noisy:?}"
        );

        let output = captured_under_default_filter(|| {
            assert_eq!(
                adopt_refreshed(budget_of(LOADED), Some(LOADED)),
                None,
                "同じ値なら差し替えないこと"
            );
        });

        assert!(
            !output.contains(CHANGED_MARKER),
            "変わっていないのに変わったと言わないこと: {output:?}"
        );
        assert!(
            !output.contains("WARN"),
            "変わっていないときに警告を出さないこと: {output:?}"
        );
    }

    /// **照会できなかったら現在値を保つ**（`None` を返す）。
    ///
    /// ここが `Some(TokenBudget::default())` に化けると、
    /// 上流が一瞬詰まっただけで予算が 16384 から 8192 へ黙って縮む。
    #[test]
    fn a_failed_requery_adopts_nothing() {
        let output = captured_under_default_filter(|| {
            assert_eq!(adopt_refreshed(budget_of(LOADED), None), None);
        });

        assert!(
            !output.contains(CHANGED_MARKER),
            "照会できていないのに変わったと言わないこと: {output:?}"
        );
    }

    /// **予算にならない申告値は採用しないが、黙りもしない。**
    ///
    /// 上流が 512 で再ロードされた状態は、こちらの予算が実際の何倍にもなっている
    /// ということなので、運用者に見えなければならない。
    /// **文言は「変わりました」と割る**（採用したのかしなかったのかが読み手に伝わらない）。
    #[test]
    fn a_requeried_value_that_cannot_become_a_budget_is_reported() {
        let output = captured_under_default_filter(|| {
            assert_eq!(
                adopt_refreshed(budget_of(LOADED), Some(UNUSABLE)),
                None,
                "予算にならない値は採用しないこと"
            );
        });

        // **深刻度も2つの値も、保ったことを告げる行の中で見る**（作業単位 B-2）。
        // **⚠ 囮の有無は上のテストと同じ**（いまは1行だけ / B-2-2 のレビュー指摘 R-3）。
        let kept = line_containing(&output, KEPT_MARKER);
        assert!(kept.contains("WARN"), "深刻度は warn であること: {kept:?}");
        // **欄はトークンとして突き合わせる**（作業単位 B-12b）。
        // **この行には `error = %err` が載る**——値に空白を含むが、
        // [`has_field`] は空白で割ってから**トークン単位で**照合するので、
        // 他の欄の判定は文言に左右されない。
        assert!(
            has_field(kept, &format!("queried={UNUSABLE}")),
            "申告値をそのまま残すこと: {kept:?}"
        );
        assert!(
            has_field(kept, &format!("context_tokens={LOADED}")),
            "保った現在値を残すこと: {kept:?}"
        );
        // **否定は出力全体で見る。**
        assert!(
            !output.contains(CHANGED_MARKER),
            "採用していないのに「変わりました」と言わないこと: {output:?}"
        );
    }

    /// **明示の器は照会の答えを採らない。照会した値の器は採る**（ADR-0020 §4 の器の側 / 作業単位 A-13a-1）。
    ///
    /// > **F-4b-2 までここにあった `an_explicit_setting_leaves_no_refresh_source` は外した**
    /// > （照会先を捨てる `refresh_source` ごと）。**明示は起動時に照会先を捨てる形ではなく、
    /// > 器が持つ印になった**——走ったまま切り替えると変わるので。
    ///
    /// **両方の面を、明示と照会の両側で見る**（どちらかに決め打つ実装を、どこかの腕で赤にする）。
    #[test]
    fn an_explicit_value_refuses_what_a_query_answers() {
        let explicit = SharedBudget::chosen(Chosen::explicit(budget_of(CONFIGURED)));
        assert!(!explicit.adopt_window(Some(RELOADED)), "明示なのに採った");
        assert_eq!(
            (explicit.get().context_tokens(), explicit.explicit_tokens()),
            (CONFIGURED, Some(CONFIGURED))
        );
        let measured = SharedBudget::chosen(Chosen::measured(budget_of(CONFIGURED)));
        assert!(
            measured.adopt_window(Some(RELOADED)),
            "明示でないのに採らない"
        );
        assert_eq!(
            (measured.get().context_tokens(), measured.explicit_tokens()),
            (RELOADED, None)
        );

        let written = SubContext::new(SUB_LOADED).expect("使える窓であること");
        let other = SubContext::new(SUB_RELOADED).expect("使える窓であること");
        let explicit = SharedSubContext::chosen(Chosen::explicit(written));
        assert!(!explicit.adopt(other), "抽出: 明示なのに採った");
        assert_eq!(
            (explicit.get(), explicit.explicit_tokens()),
            (written, Some(SUB_LOADED))
        );
        let measured = SharedSubContext::chosen(Chosen::measured(written));
        assert!(measured.adopt(other), "抽出: 明示でないのに採らない");
        assert_eq!((measured.get(), measured.explicit_tokens()), (other, None));
    }

    // ------------------------------------------------------------------
    // サブモデルのコンテキスト長（作業単位 B-7 / ADR-0036 決定2 の第1階層）
    //
    // **抽出の上限はここからしか出ない。** 定数のままだと、
    // `PERSONACORE_SUB_MODEL` を変えた瞬間に上限が現実と合わなくなる——
    // 短い側にずれれば**窓を超えた依頼を送り**（ADR-0021 §8。LM Studio では前方から黙って
    // 切り捨てられる / ADR-0013。llama は 400 で断るか、応答が黙って短くなる / ADR-0052 M5）、
    // 長い側にずれれば**入るはずの会話を捨てる。**
    // ------------------------------------------------------------------

    /// **本文用とは別の綴りのモデル**（サブモデルとして照会する側）。
    ///
    /// **`crate::llm::DEFAULT_SUB_MODEL` を使わないこと。**
    /// あれは**今は本文の既定と同じ綴り**（ADR-0021 §10 の実測でそうなった）なので、
    /// 照会先を取り違える実装と区別が付かなくなる
    /// （`llm::lmstudio` の `the_sub_model_reaches_the_upstream_through_the_real_bind` と同じ理由）。
    const SUB_MODEL_ID: &str = "ministral-3-3b-instruct-2512";

    /// サブモデルの申告値。**落とし所（16384）とも本文用の申告値とも別。**
    const SUB_LOADED: u32 = 6_144;

    /// **2つのモデルを申告させるときの、本文側の値。**
    ///
    /// **`LOADED`（16384）を使わないこと。** あれは**落とし所と同じ数**（実測値が同じ）なので、
    /// サブ側の器がその値になったとき「本文の行を読んだ」のか
    /// 「落とし所へ落ちた」のかが**区別できない**
    /// ——照会先の取り違えは、この作業単位が塞ぐ失敗そのもの。
    const MAIN_BESIDE_SUB: u32 = 14_336;

    /// 稼働中に再ロードされた後のサブモデルの申告値。**上のどれとも別。**
    const SUB_RELOADED: u32 = 10_240;

    /// 「落とし所を使う」側の手掛かり。**全文一致にしない。**
    const SUB_FALLBACK_MARKER: &str = "落とし所を使います";

    /// 「上流の申告値を使う」側の手掛かり。**上と混ぜないこと**
    /// （調べる先が「上流が停止している」と「上流のロード設定」でまるで違う）。
    const SUB_UPSTREAM_MARKER: &str = "抽出はサブモデルの実コンテキスト長を使います";

    /// サブ側で「変わった」と告げる行の手掛かり。
    const SUB_CHANGED_MARKER: &str = "サブモデルのコンテキスト長が変わりました";

    /// サブ側で「現在値を保った」と告げる行の手掛かり。
    /// **`SUB_CHANGED_MARKER` と文字列として重ならないこと。**
    ///
    /// **⚠ そして本文側の `KEPT_MARKER` とも重ならないこと**（作業単位 B-21d のレビュー指摘）。
    /// 理由はあちらの doc。**ここに書き写さない。**
    const SUB_KEPT_MARKER: &str = "抽出に使えません。現在の値を保ちます";

    /// **フィクスチャの値がすべて違う**（サブ側 / このプロジェクト最頻の穴）。
    ///
    /// **落とし所も並べる。** 同値だと「引いた値を使った」と
    /// 「引けずに落とし所へ落ちた」が区別できない
    /// ——それはこの作業単位が塞ごうとしている失敗そのもの。
    #[test]
    fn the_sub_context_fixtures_are_all_different() {
        let values = [
            ("落とし所", SubContext::FALLBACK.tokens()),
            ("サブの申告値", SUB_LOADED),
            ("再ロード後のサブの申告値", SUB_RELOADED),
            ("本文用の申告値", MAIN_BESIDE_SUB),
            ("抽出に使えない値", UNUSABLE),
        ];

        for (index, (left_label, left)) in values.iter().enumerate() {
            for (right_label, right) in values.iter().skip(index + 1) {
                assert_ne!(
                    left, right,
                    "{left_label} と {right_label} は別の値であること"
                );
            }
        }
    }

    /// サブモデルを指すクライアント。**本文用とは別の綴りを設定する。**
    fn sub_client_for(base_url: &str) -> LmStudioClient {
        let model = ModelId::parse(SUB_MODEL_ID).expect("サブモデルの綴りが正当であること");
        assert_ne!(
            model.as_str(),
            DEFAULT_MAIN_MODEL,
            "前提: 照会するモデルが本文用と別であること（同値だと取り違えを検出できない）"
        );
        let config = LmStudioConfig::new(base_url, model).expect("ベース URL が正当であること");
        LmStudioClient::new(config).expect("HTTP クライアントを構築できること")
    }

    fn sub_source_for(server: &MockServer) -> FaceClient {
        face_of(sub_client_for(&server.uri()))
    }

    /// **2つのモデルを、別のコンテキスト長で申告する上流。**
    ///
    /// **同じ値にしないこと。** 同値にすると、照会先のモデルを取り違える実装が
    /// そのまま緑を通る（CLAUDE.md の最頻の穴）。
    fn two_model_body(main_loaded: u32, sub_loaded: u32) -> String {
        assert_ne!(
            main_loaded, sub_loaded,
            "前提: 2つのモデルの申告値が別であること"
        );
        format!(
            r#"{{"object":"list","data":[
                {{"id":"{DEFAULT_MAIN_MODEL}","object":"model","type":"llm","state":"loaded",
                  "max_context_length":{MAXIMUM},"loaded_context_length":{main_loaded}}},
                {{"id":"{SUB_MODEL_ID}","object":"model","type":"llm","state":"loaded",
                  "max_context_length":{MAXIMUM},"loaded_context_length":{sub_loaded}}}]}}"#
        )
    }

    /// 2つのモデルを申告する上流を立てる。
    async fn upstream_reporting_two_models(main_loaded: u32, sub_loaded: u32) -> MockServer {
        upstream_serving(
            ResponseTemplate::new(200)
                .set_body_raw(two_model_body(main_loaded, sub_loaded), "application/json"),
        )
        .await
    }

    /// サブモデルの器が `tokens` になるまで待つ。**ならなければ落とす。**
    async fn wait_for_sub_context(shared: &SharedSubContext, tokens: u32) {
        let deadline = Instant::now() + WAIT_LIMIT;
        while shared.get().tokens() != tokens {
            assert!(
                Instant::now() < deadline,
                "サブモデルのコンテキスト長が {tokens} にならなかった（実測 {}）",
                shared.get().tokens()
            );
            tokio::time::sleep(POLL_STEP).await;
        }
    }

    /// **上流が申告したサブモデルの窓を実際に使う。**
    ///
    /// ここが落とし所に固定されていると、`PERSONACORE_SUB_MODEL` を変えた環境で
    /// **上限だけが 16384 のつもりのまま**になる。
    ///
    /// # 引き直しの照会先は「引いた相手そのもの」
    ///
    /// **同じ `Arc` が返ること**まで見る（`Arc::ptr_eq`）。
    /// 別のクライアントを組み立てて返す形は、**5分の間隔を待てるテストが無いので
    /// 他のどこからも観測できない**（`resolve_sub_context` の doc）。
    #[tokio::test]
    async fn the_sub_upstream_value_is_used_when_it_can_be_queried() {
        let server = upstream_reporting_two_models(MAIN_BESIDE_SUB, SUB_LOADED).await;
        let client = sub_source_for(&server);

        let (context, source, UpstreamProbe { catalogue, .. }) =
            resolve_sub_context(&client, None).await;

        // **同じ1回の照会から、上流が持っているモデルの綴りも読める**（作業単位 E-2）。
        // **往復は増えていない**——ここが空なら、読んでいない実装になっている。
        let catalogue = catalogue.expect("一覧が取れていること（照会は成功している）");
        assert_eq!(
            catalogue,
            vec![DEFAULT_MAIN_MODEL.to_owned(), SUB_MODEL_ID.to_owned()],
            "上流が並べた順のまま、両方の綴りを返すこと"
        );

        assert_eq!(
            context.tokens(),
            SUB_LOADED,
            "サブモデルの行を読むこと（本文用の行でも落とし所でもない）"
        );
        assert_ne!(
            context.tokens(),
            MAIN_BESIDE_SUB,
            "本文用のモデルの申告値を読んでいないこと"
        );
        assert_ne!(
            context,
            SubContext::FALLBACK,
            "引けているのに落とし所へ落ちていないこと"
        );

        let source = source.expect("引き直しの照会先を返すこと（抽出の上限に明示設定は無い）");
        assert!(
            source.is_same_handle(&client),
            "引き直しの照会先は、起動時に引いた相手そのもの（同じ差し替え口）であること"
        );
    }

    /// **停止している上流でも値が決まる。** 起動は止まらない。
    ///
    /// **それでも引き直しの照会先は返す**——上流が後から起動したら、
    /// 5分以内に本当の窓へ追いつく。
    #[tokio::test]
    async fn an_unreachable_sub_upstream_falls_back() {
        let base_url = closed_address().await;
        let client = face_of(sub_client_for(&base_url));

        let (context, source, UpstreamProbe { catalogue, .. }) =
            resolve_sub_context(&client, None).await;

        assert!(
            source.is_some(),
            "引けなかった回でも引き直しは続けること（後から上流が起きる）"
        );
        assert_eq!(context, SubContext::FALLBACK);
        // **「聞けなかった」を空の一覧と混ぜない**（作業単位 E-2）。
        // 空の一覧を返すと、`measured::check` は同じ「聞けなかった」に倒すが、
        // **ここで `Some(vec![])` を作ると「上流はこう答えた」という嘘になる。**
        assert_eq!(
            catalogue, None,
            "上流に届いていないのだから、一覧は「聞けなかった」であること"
        );
    }

    /// **抽出の依頼と応答が同時に収まらない申告値は採らない。**
    ///
    /// 採ると、依頼だけで窓を使い切って**応答の分が残らない**——
    /// LM Studio では前方から切り捨てられるので、消えるのは指示の側（ADR-0021 §8 / ADR-0013）。
    /// llama は 400 `exceed_context_size_error` で断る（ADR-0052 M5）。ただし、
    /// プロンプト ＋ 応答の上限が窓を超えると、応答が黙って短くなる（`finish_reason=length`）。
    #[test]
    fn a_sub_value_that_cannot_hold_a_request_and_a_reply_falls_back() {
        for tokens in [0_u32, 1, UNUSABLE] {
            assert_eq!(
                choose_sub_context(None, Some(tokens)),
                SubContext::FALLBACK,
                "tokens = {tokens} は抽出に使えないこと"
            );
        }
        assert_eq!(
            choose_sub_context(None, Some(SUB_LOADED)).tokens(),
            SUB_LOADED,
            "使える値はそのまま採ること"
        );
    }

    /// **明示（上流プロファイルの面に書いた値）は、照会した値にも落とし所にも勝つ**
    /// （ADR-0054 §決定1 / 作業単位 A-13a-1 / [`choose_budget`] と同じ順位）。
    ///
    /// **明示・照会・落とし所の3つを全部違える**（どれかに決め打つ実装を赤にする）。
    /// **どこから来たかを、値と同じ行で名乗る**（上流の値とも落とし所とも違う文面）。
    #[test]
    fn an_explicit_sub_context_wins_and_names_itself() {
        let written = SubContext::new(MAIN_BESIDE_SUB).expect("使える窓であること");
        assert_ne!(MAIN_BESIDE_SUB, SUB_LOADED, "前提");
        assert_ne!(written, SubContext::FALLBACK, "前提");

        let output = captured_under_default_filter(|| {
            assert_eq!(choose_sub_context(Some(written), Some(SUB_LOADED)), written);
            assert_eq!(choose_sub_context(Some(written), None), written);
        });
        let line = line_containing(&output, SUB_EXPLICIT_MARKER);
        assert!(
            has_field(line, &format!("sub_context_length={MAIN_BESIDE_SUB}")),
            "明示の値を同じ行で言うこと: {line:?}"
        );
        assert!(
            !output.contains(SUB_UPSTREAM_MARKER) && !output.contains(SUB_FALLBACK_MARKER),
            "明示なのに、上流の値か落とし所を使ったと言っている: {output:?}"
        );
    }

    /// **抽出の上限が明示設定を使ったと言う行の手掛かり**（`choose_sub_context`）。
    const SUB_EXPLICIT_MARKER: &str = "抽出の上限は明示設定のコンテキスト長を使います";

    /// **値を書いた抽出の面は、起動時に照会しない。照会先は返す**（ADR-0054 §決定1・3）。
    ///
    /// **照会先を返さないと、走ったまま値の無いプロファイルへ切り替えたときに引き直せない。**
    #[tokio::test]
    async fn a_written_sub_context_is_not_queried_at_startup_but_keeps_its_source() {
        let server = upstream_reporting_two_models(MAIN_BESIDE_SUB, SUB_LOADED).await;
        let client = sub_source_for(&server);
        let written = SubContext::new(SUB_RELOADED).expect("使える窓であること");

        let (context, source, probe) = resolve_sub_context(&client, Some(written)).await;

        assert_eq!(context, written, "書いた値を使うこと");
        assert_eq!(queries_to(&server).await, 0, "書いた面へ照会している");
        assert_eq!(probe.catalogue, None, "聞いていないのに一覧を持っている");
        assert!(
            source.is_some_and(|source| source.is_same_handle(&client)),
            "引き直しの照会先（同じ差し替え口）を返すこと"
        );
    }

    /// **どちらの経路で決まったのかがログから読める**（`choose_budget` と同じ作法）。
    ///
    /// 採用元は「上流の申告値」と「落とし所」で調べる先がまるで違う。
    /// **両方向の否定を置く**——片側だけだと、2つの文言をそっくり入れ替える変異が
    /// 両方向とも素通りする。
    #[test]
    fn the_source_of_the_sub_context_is_named_in_the_log() {
        let from_upstream = captured_under_default_filter(|| {
            assert_eq!(
                choose_sub_context(None, Some(SUB_LOADED)).tokens(),
                SUB_LOADED
            );
        });
        let fallen_back = captured_under_default_filter(|| {
            assert_eq!(choose_sub_context(None, None), SubContext::FALLBACK);
        });

        // 肯定側。**この2つが、下の否定側の前提を兼ねている。**
        assert!(
            from_upstream.contains(SUB_UPSTREAM_MARKER),
            "上流の申告値を使ったと言うこと: {from_upstream:?}"
        );
        let fallback = line_containing(&fallen_back, SUB_FALLBACK_MARKER);
        assert!(
            fallback.contains("WARN"),
            "落とし所へ落ちたことは既定のログ設定で見えること: {fallback:?}"
        );

        // **上限も同じ行に出す。** 窓の長さだけでは、半分にした後の数が読めない。
        let adopted = line_containing(&from_upstream, SUB_UPSTREAM_MARKER);
        assert!(
            has_field(adopted, &format!("limit_tokens={}", SUB_LOADED / 2)),
            "採用した上限を、採用を告げる行に残すこと: {adopted:?}"
        );
        assert!(
            has_field(adopted, &format!("sub_context_length={SUB_LOADED}")),
            "採用した窓も同じ行に残すこと: {adopted:?}"
        );

        // 否定側（両方向）。
        assert!(
            !from_upstream.contains(SUB_FALLBACK_MARKER),
            "引けているのに落とし所と報告しないこと: {from_upstream:?}"
        );
        assert!(
            !fallen_back.contains(SUB_UPSTREAM_MARKER),
            "引けていないのに上流の申告値と報告しないこと: {fallen_back:?}"
        );
    }

    /// **2つの照会先は、それぞれ自分のモデルを引く**（作業単位 B-7 の要）。
    ///
    /// **同じ1つの上流に、別の窓で2つのモデルを申告させる。**
    /// 照会先を取り違える実装（サブ側に本文用のクライアントを渡す / その逆）は、
    /// **2つの器が両方とも相手の値になる**のでここで赤になる。
    ///
    /// **`assert_ne!` では防げない形**（値は全部違っていて、組み合わせだけが間違っている /
    /// CLAUDE.md「抜けるのはいつも帰属」）なので、**値ではなく行き先**を見る。
    #[tokio::test]
    async fn the_two_sources_are_queried_for_their_own_models() {
        let server = upstream_reporting_two_models(MAIN_BESIDE_SUB, SUB_LOADED).await;

        // **どちらの器も、引いた後の値とは別の値から始める。**
        // 同じ値から始めると「引いて差し替えた」と「最初からそうだった」が区別できない。
        let pending = PendingRefresh::new(
            Chosen::measured(budget_of(RELOADED)),
            Some(source_for(&server)),
            Chosen::measured(
                SubContext::new(SUB_RELOADED).expect("フィクスチャは抽出に使える値であること"),
            ),
            Some(sub_source_for(&server)),
            upstream_facts(),
            CheckAtStart::NONE,
        );
        let sub_shared = pending.sub_context();

        let (task, budget) = pending.spawn_every(SHORT_INTERVAL);

        wait_for_budget(&budget, MAIN_BESIDE_SUB).await;
        wait_for_sub_context(&sub_shared, SUB_LOADED).await;

        drop(task);
    }

    /// **`PendingRefresh` が渡す器は、走り出したタスクが書く先そのもの。**
    ///
    /// 抽出器は `bind` の時点でこの器を受け取る（`serve` まで待てない）ので、
    /// **走り出す前に渡した器と、走り出したタスクが書く器が同じ**でなければ、
    /// **引き直しは正しく WARN を出しながら抽出の上限だけが動かない。**
    /// （`the_spawned_task_writes_into_the_returned_budget` のサブ版）
    #[tokio::test]
    async fn the_spawned_task_writes_into_the_handed_out_sub_context() {
        let server = upstream_reporting_two_models(MAIN_BESIDE_SUB, SUB_LOADED).await;
        let pending = PendingRefresh::new(
            Chosen::measured(budget_of(LOADED)),
            None,
            Chosen::measured(
                SubContext::new(SUB_RELOADED).expect("フィクスチャは抽出に使える値であること"),
            ),
            Some(sub_source_for(&server)),
            upstream_facts(),
            CheckAtStart::NONE,
        );

        // **走り出す前に受け取る**（`App::bind_with` と同じ順序）。
        let sub_shared = pending.sub_context();
        let (task, _budget) = pending.spawn_every(SHORT_INTERVAL);

        wait_for_sub_context(&sub_shared, SUB_LOADED).await;

        drop(task);
    }

    /// **本文の予算に明示設定があっても、サブ側は引き直す**（ADR-0036 決定3）。
    ///
    /// `PERSONACORE_CONTEXT_TOKENS` が優先するのは本文の予算であって、
    /// 抽出の上限ではない。**止めると落とし所のまま動かなくなる**——
    /// しかも「明示設定を置いた運用では仕掛けが動かない」という
    /// ADR-0020 §4 の但し書きが、**設定していない値にまで及ぶ。**
    #[tokio::test]
    async fn an_explicit_main_setting_does_not_stop_the_sub_requery() {
        let server = upstream_reporting_two_models(MAIN_BESIDE_SUB, SUB_LOADED).await;
        let sub_shared = SharedSubContext::for_tests(
            SubContext::new(SUB_RELOADED).expect("フィクスチャは抽出に使える値であること"),
        );

        // **本文側は明示の器**（照会先は持っているが引かない / 作業単位 A-13a-1）。
        let budget = SharedBudget::chosen(Chosen::explicit(budget_of(CONFIGURED)));
        let task = spawn_refresh_every(
            checker_of(
                Some(source_for(&server)),
                budget.clone(),
                Some(sub_source_for(&server)),
                sub_shared.clone(),
                SharedUpstream::new(upstream_facts()),
            ),
            CheckAtStart::NONE,
            SHORT_INTERVAL,
        );

        wait_for_sub_context(&sub_shared, SUB_LOADED).await;
        assert_eq!(
            budget.get().context_tokens(),
            CONFIGURED,
            "本文の明示の値を、引き直しで動かしている"
        );

        drop(task);
    }

    /// **2つとも照会先が無ければ、タスクそのものを立てない。**
    ///
    /// 立てると、何もしないループが 5 分ごとに回るだけになる。
    #[tokio::test]
    async fn nothing_is_spawned_when_neither_side_has_a_source() {
        let server = upstream_reporting_two_models(MAIN_BESIDE_SUB, SUB_LOADED).await;
        let sub_shared = SharedSubContext::fallback_for_tests();

        // **起動の直後の照会も頼んである**（作業単位 B-66）——照会先が無ければ、それでも立てない。
        let task = spawn_refresh_every(
            checker_of(
                None,
                SharedBudget::new(budget_of(CONFIGURED)),
                None,
                sub_shared.clone(),
                SharedUpstream::new(upstream_facts()),
            ),
            CheckAtStart {
                main: true,
                sub: true,
            },
            SHORT_INTERVAL,
        );

        tokio::time::sleep(SHORT_INTERVAL * 20).await;

        assert_eq!(
            queries_to(&server).await,
            0,
            "照会先が無いなら1回も照会しないこと"
        );
        assert_eq!(
            sub_shared.get(),
            SubContext::FALLBACK,
            "照会していないのに値が動かないこと"
        );

        drop(task);
    }

    /// **引き直しに失敗しても現在値を保つ。落とし所へ落とさない**（ADR-0020 §5）。
    ///
    /// 落とすと、上流が一瞬詰まっただけで**上限が黙って広がる**
    /// ——広がった先で組んだ依頼は窓を超える（ADR-0021 §8。LM Studio では前方から黙って
    /// 切り捨てられる / ADR-0013。llama は 400 `exceed_context_size_error` で断るか、
    /// プロンプト ＋ 応答の上限が窓を超えて応答が黙って短くなる / ADR-0052 M5）。
    #[tokio::test]
    async fn a_failing_sub_requery_keeps_the_current_sub_context() {
        let server = upstream_serving(ResponseTemplate::new(500)).await;
        let current = SubContext::new(SUB_LOADED).expect("フィクスチャは抽出に使える値であること");
        let sub_shared = SharedSubContext::for_tests(current);

        let task = spawn_sub_refresh_every(
            Some(sub_source_for(&server)),
            sub_shared.clone(),
            SHORT_INTERVAL,
        );

        wait_for_queries(&server, 3).await;

        assert_eq!(
            sub_shared.get(),
            current,
            "失敗した引き直しで現在値を捨てないこと"
        );
        assert_ne!(
            sub_shared.get(),
            SubContext::FALLBACK,
            "落とし所へ落とさないこと（起動時とは違い、すでに手掛かりを持っている）"
        );

        drop(task);
    }

    /// **抽出に使えない申告値でも現在値を保つ。** ここも落とし所へ落とさない。
    #[tokio::test]
    async fn a_requeried_sub_value_that_cannot_be_used_keeps_the_current_sub_context() {
        let server = upstream_reporting_two_models(MAIN_BESIDE_SUB, UNUSABLE).await;
        let current = SubContext::new(SUB_LOADED).expect("フィクスチャは抽出に使える値であること");
        let sub_shared = SharedSubContext::for_tests(current);

        let task = spawn_sub_refresh_every(
            Some(sub_source_for(&server)),
            sub_shared.clone(),
            SHORT_INTERVAL,
        );

        wait_for_queries(&server, 3).await;

        assert_eq!(
            sub_shared.get(),
            current,
            "抽出に使えない値で現在値を捨てないこと"
        );
        assert_ne!(
            sub_shared.get(),
            SubContext::FALLBACK,
            "落とし所へ落とさないこと"
        );

        drop(task);
    }

    /// **変わったら `warn`。新しい値を返す**（ADR-0020 §5 のサブ版）。
    ///
    /// **黙って上限だけ変えない。** 変えると「昨日は抽出できた往復が今日は
    /// `conversation_too_long` で捨てられる」理由が追えない。
    #[test]
    fn a_changed_sub_context_is_adopted_and_warned() {
        let current = SubContext::new(SUB_LOADED).expect("フィクスチャは抽出に使える値であること");

        let output = captured_under_default_filter(|| {
            let adopted = adopt_refreshed_sub(current, Some(SUB_RELOADED));
            assert_eq!(
                adopted,
                SubContext::new(SUB_RELOADED),
                "申告値をそのままコンテキスト長にすること"
            );
        });

        // **深刻度も前後の値も、変化を告げる行の中で見る**（作業単位 B-2）。
        let changed = line_containing(&output, SUB_CHANGED_MARKER);
        assert!(
            changed.contains("WARN"),
            "深刻度は warn であること: {changed:?}"
        );
        assert!(
            has_field(
                changed,
                &format!("previous_sub_context_length={SUB_LOADED}")
            ),
            "変わる前の値を残すこと: {changed:?}"
        );
        assert!(
            has_field(changed, &format!("sub_context_length={SUB_RELOADED}")),
            "変わった後の値を残すこと: {changed:?}"
        );
        assert!(
            has_field(changed, &format!("limit_tokens={}", SUB_RELOADED / 2)),
            "新しい上限も同じ行に残すこと: {changed:?}"
        );
    }

    /// **変わっていなければ黙る。** 5分ごとに鳴ると、本当に変わった1行が埋もれる。
    ///
    /// # 否定の前に肯定を置く
    ///
    /// `warn!` ごと消えていても否定側は「何も出なかった」ので緑になる。
    #[test]
    fn an_unchanged_sub_context_is_not_warned() {
        let current = SubContext::new(SUB_LOADED).expect("フィクスチャは抽出に使える値であること");

        let noisy = captured_with_filter(VERBOSE_FILTER, || {
            let _ = adopt_refreshed_sub(current, Some(SUB_RELOADED));
        });
        assert!(
            noisy.contains(SUB_CHANGED_MARKER),
            "前提: 変わった側の文言は現に出せること: {noisy:?}"
        );

        let output = captured_under_default_filter(|| {
            assert_eq!(
                adopt_refreshed_sub(current, Some(SUB_LOADED)),
                None,
                "同じ値なら差し替えないこと"
            );
        });

        assert!(
            !output.contains(SUB_CHANGED_MARKER),
            "変わっていないのに変わったと言わないこと: {output:?}"
        );
        assert!(
            !output.contains("WARN"),
            "変わっていないときに警告を出さないこと: {output:?}"
        );
    }

    /// **照会できなかったら現在値を保つ**（`None` を返す）。
    #[test]
    fn a_failed_sub_requery_adopts_nothing() {
        let current = SubContext::new(SUB_LOADED).expect("フィクスチャは抽出に使える値であること");

        let output = captured_under_default_filter(|| {
            assert_eq!(adopt_refreshed_sub(current, None), None);
        });

        assert!(
            !output.contains(SUB_CHANGED_MARKER),
            "照会できていないのに変わったと言わないこと: {output:?}"
        );
    }

    /// **抽出に使えない申告値は採らないが、黙りもしない。**
    ///
    /// 上流が 512 で再ロードされた状態は、こちらの上限が実際の何倍にもなっている
    /// ということなので、運用者に見えなければならない。
    /// **文言は「変わりました」と割る**（採ったのか採らなかったのかが読み手に伝わらない）。
    #[test]
    fn a_requeried_sub_value_that_cannot_be_used_is_reported() {
        let current = SubContext::new(SUB_LOADED).expect("フィクスチャは抽出に使える値であること");

        let output = captured_under_default_filter(|| {
            assert_eq!(
                adopt_refreshed_sub(current, Some(UNUSABLE)),
                None,
                "抽出に使えない値は採らないこと"
            );
        });

        let kept = line_containing(&output, SUB_KEPT_MARKER);
        assert!(kept.contains("WARN"), "深刻度は warn であること: {kept:?}");
        assert!(
            has_field(kept, &format!("queried={UNUSABLE}")),
            "申告値をそのまま残すこと: {kept:?}"
        );
        assert!(
            has_field(kept, &format!("sub_context_length={SUB_LOADED}")),
            "保った現在値を残すこと: {kept:?}"
        );
        assert!(
            !output.contains(SUB_CHANGED_MARKER),
            "採っていないのに「変わりました」と言わないこと: {output:?}"
        );
    }

    /// **本文とサブが同じ窓で並んで喋っても、各層の手掛かりが自分の行を選ぶ**
    /// （作業単位 B-21d のレビュー指摘 / 2026-08-29）。
    ///
    /// # なぜ要るのか
    ///
    /// **本番では1ティックで main と sub が並んで出る**（`spawn_refresh_every` /
    /// `spawn_sub_refresh_every`）。ところが B-21d より前の手掛かりは**層をまたいでいた**:
    ///
    /// | 手掛かり | 当たっていた本番の文言 |
    /// |---|---|
    /// | `"コンテキスト長が変わりました"` | 本文の「変わった」**と**サブの「変わった」 |
    /// | `"現在の値を保ちます"` | **4つ全部**（本文の引き直し失敗 / サブの引き直し失敗 / 本文の見送り / サブの見送り） |
    ///
    /// **他のテストが単一行の窓しか作っていなかったので、実害は出ていなかった。**
    /// **doc は「`CHANGED_MARKER` と重ならないこと」しか言っておらず、
    /// 本当の要件（層をまたがないこと）を誰も見ていなかった。**
    ///
    /// # 4本を1つの窓に並べる
    ///
    /// **層 × 「変わった / 保った」の4通りを、同じ捕捉窓の中で出す。**
    /// 単一行の窓では、手掛かりが層をまたいでいても永久に観測できない
    /// （CLAUDE.md「フィクスチャがその分岐が効く状態に一度も入らない」）。
    ///
    /// # 帰属は**層ごとに違う欄**で見る
    ///
    /// 本文の行は `context_tokens=`、サブの行は `sub_context_length=` を持つ。
    /// **「その手掛かりで選んだ行が、その層の欄を持っていること」**を見るので、
    /// 手掛かりを層をまたぐ綴りへ戻す変異は、**並びのどちら側でも**赤になる。
    #[test]
    fn each_layer_marker_picks_its_own_line_when_both_layers_speak() {
        let sub_current =
            SubContext::new(SUB_LOADED).expect("フィクスチャは抽出に使える値であること");

        // **並びを入れ替えて2回測る。** 片方の順番でしか赤にならない形にしない
        // （CLAUDE.md「囮の順番に寄りかかった赤」）。
        for main_first in [true, false] {
            let output = captured_under_default_filter(|| {
                let speak_main = || {
                    let _ = adopt_refreshed(budget_of(LOADED), Some(RELOADED));
                    let _ = adopt_refreshed(budget_of(LOADED), Some(UNUSABLE));
                };
                let speak_sub = || {
                    let _ = adopt_refreshed_sub(sub_current, Some(SUB_RELOADED));
                    let _ = adopt_refreshed_sub(sub_current, Some(UNUSABLE));
                };
                if main_first {
                    speak_main();
                    speak_sub();
                } else {
                    speak_sub();
                    speak_main();
                }
            });

            // **前提: 4本とも現に出ていること。** ここが偽なら下は何も見ていない。
            assert_eq!(
                output.lines().count(),
                4,
                "前提: 両層の「変わった」と「保った」が1本ずつ出ること: {output:?}"
            );

            // 本文側の2本は、本文の欄を持つ行であること。
            for marker in [CHANGED_MARKER, KEPT_MARKER] {
                let line = line_containing(&output, marker);
                assert!(
                    has_key(line, "context_tokens"),
                    "本文の手掛かり {marker:?} が本文の行を選べていない: {line:?}"
                );
                assert!(
                    !has_key(line, "sub_context_length"),
                    "本文の手掛かり {marker:?} がサブの行を掴んでいる: {line:?}"
                );
            }

            // サブ側の2本は、サブの欄を持つ行であること。
            for marker in [SUB_CHANGED_MARKER, SUB_KEPT_MARKER] {
                let line = line_containing(&output, marker);
                assert!(
                    has_key(line, "sub_context_length"),
                    "サブの手掛かり {marker:?} がサブの行を選べていない: {line:?}"
                );
                assert!(
                    !has_key(line, "context_tokens"),
                    "サブの手掛かり {marker:?} が本文の行を掴んでいる: {line:?}"
                );
            }

            // **4つの手掛かりは、4本の別々の行を選ぶ**（2つが同じ行に落ちない）。
            let picked: Vec<&str> = [
                CHANGED_MARKER,
                KEPT_MARKER,
                SUB_CHANGED_MARKER,
                SUB_KEPT_MARKER,
            ]
            .into_iter()
            .map(|marker| line_containing(&output, marker))
            .collect();
            let mut unique = picked.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(
                unique.len(),
                picked.len(),
                "4つの手掛かりが同じ行を選んでいる: {picked:?}"
            );
        }
    }

    // ------------------ 載せている面と外の上流で、深刻度が割れる（作業単位 B-51）

    /// どちらの読み手も読めなかったことを告げる行の手掛かり（`llm::lmstudio`）。
    const NEITHER_READER_MARKER: &str = "どちらの綴りでも読めませんでした";

    /// 載せている面が依頼する綴り。**外の上流の綴りとも既定とも別**（帰属を行の中で見るため）。
    const LAUNCHING_MODEL: &str = "planted-b51-launching-model";

    /// 外の上流の面が依頼する綴り。
    const EXTERNAL_MODEL: &str = "planted-b51-external-model";

    /// 綴りを指定してクライアントを作る（宛先は同じでよい。**帰属は綴りで見る**）。
    fn client_asking_for(base_url: &str, model: &str) -> LmStudioClient {
        let model = ModelId::parse(model).expect("テストの綴りは正当であること");
        let config = LmStudioConfig::new(base_url, model).expect("ベース URL が正当であること");
        LmStudioClient::new(config).expect("HTTP クライアントを構築できること")
    }

    /// 行の深刻度（`<時刻> <水準> <層>: …` の2語目）。
    fn level_of(line: &str) -> Option<&str> {
        line.split_whitespace().nth(1)
    }

    /// **一覧がどちらの綴りでも読めないとき、深刻度は面ごとに割れる**（作業単位 B-51 / ADR-0042 §決定3）。
    ///
    /// | 面 | 経路 | 期待 |
    /// |---|---|---|
    /// | **載せている面** | [`probe_face_while_loading`]（番人が待つ間に呼ぶ） | **`DEBUG`**（期待している状態。経過は番人が `info` で言う） |
    /// | **外の上流の面** | [`probe_face`]（起こしていない面・載った後の面） | **`WARN`**（期待していない状態。今のまま） |
    ///
    /// **同じ上流・同じ 503・同じ1回の捕捉**で、**レベルと文面と面（依頼した綴り）を同じ行で**見る
    /// （CLAUDE.md §4「ログを見る見張り番は、同じ行の中で見る」）。
    ///
    /// **`DEBUG` で捕まえる**——既定のフィルタで捕まえると、載せている面の行は
    /// **消しても（`Hush` に潰しても）緑になる。** 「出ている、ただし警告ではない」を見るには
    /// 行が見えている必要がある。
    ///
    /// **`INFO` ではなく `DEBUG` である理由**: 番人は2秒ごとに聞く（`launch::POLL_INTERVAL`）。
    /// 経過の `info` は30秒ごと（`launch::PROGRESS_INTERVAL`）に番人が1本出しているので、
    /// **ここを `info` にすると、2分の載せ替えで60行が既定のフィルタに並ぶ**（Issue origin#20 の形が水準だけ変わって残る）。
    #[test]
    fn an_unreadable_list_is_progress_for_a_launching_face_but_a_warning_for_an_external_one() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime を作れること");

        let output = captured_with_filter(crate::testlog::DEBUG_FILTER, || {
            runtime.block_on(async {
                let server = MockServer::start().await;
                // **本物の `llama-server` が読み込み中に返す形**（どの綴りにも 503）。
                for listing in ["/api/v0/models", "/v1/models"] {
                    Mock::given(method("GET"))
                        .and(path(listing))
                        .respond_with(ResponseTemplate::new(503).set_body_string(
                            r#"{"error":{"code":503,"message":"Loading model","type":"unavailable_error"}}"#,
                        ))
                        .mount(&server)
                        .await;
                }
                let clients = FaceClients::new(
                    Arc::new(client_asking_for(&server.uri(), LAUNCHING_MODEL)),
                    Arc::new(client_asking_for(&server.uri(), EXTERNAL_MODEL)),
                );

                let (launching, _) = probe_face_while_loading(&clients, ModelFace::Main).await;
                assert_eq!(launching.answered_by, None, "前提: 載せている面は答えていない");
                let (external, _) = probe_face(&clients, ModelFace::Sub).await;
                assert_eq!(external.answered_by, None, "前提: 外の上流も答えていない");
            });
        });

        // **面ごとに1本ずつ、同じ行の中で選ぶ**（文面と綴りの両方を持つ行）。
        let line_of = |model: &str| -> &str {
            let field = format!("model={model}");
            let lines: Vec<&str> = output
                .lines()
                .filter(|line| line.contains(NEITHER_READER_MARKER) && has_field(line, &field))
                .collect();
            assert_eq!(
                lines.len(),
                1,
                "{model} の面について、どちらの綴りでも読めなかった行がちょうど1本あること: {output:?}"
            );
            lines[0]
        };

        let launching = line_of(LAUNCHING_MODEL);
        assert_eq!(
            level_of(launching),
            Some("DEBUG"),
            "載せている面の「読めなかった」は経過（debug）であること: {launching:?}"
        );

        // **対照**: 同じ 503 でも、外の上流の面は今のまま警告。
        let external = line_of(EXTERNAL_MODEL);
        assert_eq!(
            level_of(external),
            Some("WARN"),
            "外の上流の「読めなかった」は警告のままであること: {external:?}"
        );
    }

    /// 明示の面が依頼する綴り（レビュー是正 / 作業単位 B-66）。
    const WRITTEN_FACE_MODEL: &str = "planted-b66-written-model";

    /// 明示ではない面が依頼する綴り。
    const QUERIED_FACE_MODEL: &str = "planted-b66-queried-model";

    /// 抽出の明示の面が依頼する綴り。
    const WRITTEN_SUB_FACE_MODEL: &str = "planted-b66-written-sub-model";

    /// 抽出の明示ではない面が依頼する綴り。
    const QUERIED_SUB_FACE_MODEL: &str = "planted-b66-queried-sub-model";

    /// **明示の面への照会は、「どちらの綴りでも読めませんでした」を警告にしない**（レビュー是正 / 作業単位 B-66）。
    ///
    /// 作業単位 B-66 の1周目まで、明示の面には照会しなかった —— この `warn` はその面では1度も出ていなかった。
    /// **観測のためだけの照会（予算に使わない）で、今まで出ていなかった警告を5分ごとに新しく鳴らさない。**
    /// 一覧の口を持たない上流（ADR-0038 §決定1 で一覧は「任意」）にコンテキスト長を書いて繋ぐ構成がこれに当たる。
    ///
    /// **同じ上流・同じ 503・同じ1回の捕捉**で、明示の面は `DEBUG`、明示ではない面は今のまま `WARN`
    /// （`DEBUG` で捕まえる —— 既定のフィルタだと、明示の面の行を消す変異でも緑になる）。
    #[test]
    fn an_unreadable_list_is_not_a_warning_for_a_written_face_but_stays_one_otherwise() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime を作れること");

        let output = captured_with_filter(crate::testlog::DEBUG_FILTER, || {
            runtime.block_on(async {
                let server = MockServer::start().await;
                for listing in ["/api/v0/models", "/v1/models"] {
                    Mock::given(method("GET"))
                        .and(path(listing))
                        .respond_with(ResponseTemplate::new(503))
                        .mount(&server)
                        .await;
                }
                let upstream = SharedUpstream::new(upstream_facts());
                let written = SharedBudget::chosen(Chosen::explicit(budget_of(CONFIGURED)));
                refresh_once(
                    &face_of(client_asking_for(&server.uri(), WRITTEN_FACE_MODEL)),
                    &written,
                    &upstream,
                )
                .await;
                let queried = SharedBudget::chosen(Chosen::measured(budget_of(CONFIGURED)));
                refresh_once(
                    &face_of(client_asking_for(&server.uri(), QUERIED_FACE_MODEL)),
                    &queried,
                    &upstream,
                )
                .await;
                // **抽出の面も同じ**（`refresh_sub_once` —— 本文の側だけを直す形を赤にする）。
                let written_sub = SharedSubContext::chosen(Chosen::explicit(SubContext::FALLBACK));
                refresh_sub_once(
                    &face_of(client_asking_for(&server.uri(), WRITTEN_SUB_FACE_MODEL)),
                    &written_sub,
                    &upstream,
                )
                .await;
                let queried_sub = SharedSubContext::chosen(Chosen::measured(SubContext::FALLBACK));
                refresh_sub_once(
                    &face_of(client_asking_for(&server.uri(), QUERIED_SUB_FACE_MODEL)),
                    &queried_sub,
                    &upstream,
                )
                .await;
            });
        });

        let line_of = |model: &str| -> &str {
            let field = format!("model={model}");
            let lines: Vec<&str> = output
                .lines()
                .filter(|line| line.contains(NEITHER_READER_MARKER) && has_field(line, &field))
                .collect();
            assert_eq!(
                lines.len(),
                1,
                "{model} の面について、どちらの綴りでも読めなかった行がちょうど1本あること: {output:?}"
            );
            lines[0]
        };
        let written = line_of(WRITTEN_FACE_MODEL);
        assert_eq!(
            level_of(written),
            Some("DEBUG"),
            "明示の面への照会で、一覧が読めないことを警告にした: {written:?}"
        );
        let queried = line_of(QUERIED_FACE_MODEL);
        assert_eq!(
            level_of(queried),
            Some("WARN"),
            "対照: 明示ではない面は今のまま警告であること: {queried:?}"
        );
        let written_sub = line_of(WRITTEN_SUB_FACE_MODEL);
        assert_eq!(
            level_of(written_sub),
            Some("DEBUG"),
            "抽出の明示の面への照会で、一覧が読めないことを警告にした: {written_sub:?}"
        );
        let queried_sub = line_of(QUERIED_SUB_FACE_MODEL);
        assert_eq!(
            level_of(queried_sub),
            Some("WARN"),
            "対照: 抽出の明示ではない面は今のまま警告であること: {queried_sub:?}"
        );
    }

    // ------------------------------------------------------------------
    // 本文の面の返答の上限（ADR-0059 §決定2 / 作業単位 A-18a）
    // ------------------------------------------------------------------

    /// **本文の面に返答の上限だけを書いた形**（思考の量は書かない）。
    fn a18a_room(room: u32) -> BodyFace {
        BodyFace {
            reasoning_effort: None,
            response_tokens: Some(room),
        }
    }

    /// **窓と面の上限から予算を組む表**（[`budget_for_window`] の doc の表 / ADR-0059 §決定2）。
    ///
    /// **期待値は手で書く**（`TokenBudget::DEFAULT_RESPONSE_TOKENS` から作らない —— 1024 を書く）。
    #[test]
    fn a18a_the_window_carries_the_faces_room_or_falls_back_to_the_default() {
        let budget = |context: u32, response: u32| {
            TokenBudget::new(context, response).expect("期待値の予算は組めること")
        };
        for (window, wanted, expected) in [
            (16_384, 2_048, Some(Fitted::Wanted(budget(16_384, 2_048)))),
            (2_049, 2_048, Some(Fitted::Wanted(budget(2_049, 2_048)))),
            (2_048, 2_048, Some(Fitted::FellBack(budget(2_048, 1_024)))),
            (1_025, 2_048, Some(Fitted::FellBack(budget(1_025, 1_024)))),
            (1_024, 2_048, None),
            (16_384, 1_024, Some(Fitted::Wanted(budget(16_384, 1_024)))),
            (1_024, 1_024, None),
            // **照会した窓の下限 1024 は、面の上限が 1024 未満でも残す**（読み込み中の偽の小さな窓への備え /
            // レビュー是正）。
            (900, 512, None),
            (1_024, 512, None),
            (1_025, 512, Some(Fitted::Wanted(budget(1_025, 512)))),
            (1_025, 1_024, Some(Fitted::Wanted(budget(1_025, 1_024)))),
            (512, 512, None),
        ] {
            assert_eq!(
                budget_for_window(window, wanted),
                expected,
                "{window} / {wanted}"
            );
        }
    }

    /// **照会した窓に面の上限を載せ、窓が上限以下なら既定の 1024 に戻して `warn` を1本出す**
    /// （ADR-0059 §決定2 の「照会した窓が `response_tokens` 以下だったとき」/ 腕6の単体の側）。
    ///
    /// | 腕 | 窓 | 余白 | `warn` |
    /// |---|---|---|---|
    /// | 窓が十分 | 16384 | **2048**（面の値） | 出さない |
    /// | 窓が上限以下 | 2048 | **1024**（既定へ戻す） | **出す**（同じ行に窓・書いた上限・戻した余白） |
    /// | 照会できない | 既定の 8192 | **1024**（窓が分からないので面の値を使わない / レビュー是正） | **窓が分からないと言う行**を出す（窓の不足の行は出さない） |
    /// | 照会した窓が予算にならない（900） | 既定の 8192 | **1024**（同上） | 同上 |
    /// | 明示の予算 | 組んだまま | 組んだまま | 出さない |
    ///
    /// **落とし所の窓かどうか**（[`Chosen::on_fallback_window`]）も見る —— 送る値のログが、既定に戻した理由を
    /// 言い分けるのに使う。
    #[test]
    fn a18a_choose_budget_puts_the_faces_room_on_the_queried_window_and_warns_on_falling_back() {
        let roomy = captured_under_default_filter(|| {
            let chosen = super::choose_budget(None, Some(16_384), a18a_room(2_048));
            let budget = chosen.value();
            assert_eq!(
                (
                    budget.context_tokens(),
                    budget.response_tokens(),
                    chosen.on_fallback_window()
                ),
                (16_384, 2_048, false)
            );
        });
        assert!(
            !roomy.contains(RESPONSE_FALLBACK_LINE),
            "窓が十分なのに戻したと言った: {roomy}"
        );
        assert!(!roomy.contains(RESPONSE_UNKNOWN_WINDOW_LINE), "{roomy}");

        let narrow = captured_under_default_filter(|| {
            let chosen = super::choose_budget(None, Some(2_048), a18a_room(2_048));
            let budget = chosen.value();
            assert_eq!(
                (
                    budget.context_tokens(),
                    budget.response_tokens(),
                    chosen.on_fallback_window()
                ),
                (2_048, 1_024, false),
                "窓が上限以下なら既定の 1024 に戻すこと（窓は分かっている）"
            );
        });
        assert!(!narrow.contains(RESPONSE_UNKNOWN_WINDOW_LINE), "{narrow}");
        let line = line_containing(&narrow, RESPONSE_FALLBACK_LINE);
        assert!(
            line.contains("WARN"),
            "既定のフィルタで見える警告であること: {line}"
        );
        for field in [
            "context_tokens=2048",
            "response_tokens_written=2048",
            "response_tokens=1024",
        ] {
            assert!(has_field(line, field), "{field} を同じ行で言うこと: {line}");
        }

        // **窓が分からない**（照会に失敗した / 照会した窓が予算にならない）—— 落とし所の窓 8192 で組み、
        // **面の上限は使わない**（使うとプロンプトの予算が縮んで削れない枠が収まらなくなりうる / レビュー是正）。
        for queried in [None, Some(900)] {
            let unanswered = captured_under_default_filter(|| {
                let chosen = super::choose_budget(None, queried, a18a_room(2_048));
                let budget = chosen.value();
                assert_eq!(
                    (
                        budget.context_tokens(),
                        budget.response_tokens(),
                        chosen.on_fallback_window()
                    ),
                    (8_192, 1_024, true),
                    "{queried:?}: 窓が分からないなら既定の 1024 にすること"
                );
            });
            assert!(
                !unanswered.contains(RESPONSE_FALLBACK_LINE),
                "{queried:?}: 窓の不足と言った: {unanswered}"
            );
            let line = line_containing(&unanswered, RESPONSE_UNKNOWN_WINDOW_LINE);
            assert!(line.contains("WARN"), "{queried:?}: {line}");
            for field in ["response_tokens_written=2048", "response_tokens=1024"] {
                assert!(
                    has_field(line, field),
                    "{queried:?}: {field} を同じ行で言うこと: {line}"
                );
            }
        }
        // **対照**: 面に何も書いていなければ、窓が分からなくても「戻した」とは言わない（もともと 1024）。
        let plain = captured_under_default_filter(|| {
            let chosen = super::choose_budget(None, None, BodyFace::default());
            assert_eq!(chosen.value(), TokenBudget::default());
        });
        assert!(!plain.contains(RESPONSE_UNKNOWN_WINDOW_LINE), "{plain}");

        let explicit = TokenBudget::new(30_000, 3_000).expect("組めること");
        let chosen = super::choose_budget(Some(explicit), Some(2_048), a18a_room(2_048));
        assert_eq!(
            (chosen.value(), chosen.on_fallback_window()),
            (explicit, false),
            "明示の予算は組んだまま使うこと"
        );
    }

    /// **起こす上流で起動したときは、まだ聞いていないだけなので「窓が分からない」は `info`**
    /// （A-18a の再レビュー是正 / [`super::budget_before_launch`]）。照会したが窓が取れなかった起動
    /// （[`super::choose_budget`]）は今どおり `warn` —— 同じ入力で段階だけが割れることを1本で見る。
    #[test]
    fn a18a_a_launch_has_not_asked_yet_so_the_unknown_window_is_info() {
        let launching = captured_under_default_filter(|| {
            let chosen = super::budget_before_launch(None, a18a_room(2_048));
            assert_eq!(
                (chosen.value(), chosen.on_fallback_window()),
                (TokenBudget::default(), true),
                "起こす上流の起動も、落とし所の窓で既定の 1024"
            );
        });
        let line = line_containing(&launching, RESPONSE_UNKNOWN_WINDOW_LINE);
        assert!(line.contains("INFO"), "まだ聞いていないだけ: {line}");
        assert!(!line.contains("WARN"), "{line}");
        assert!(has_field(line, "response_tokens_written=2048"), "{line}");

        let unanswered = captured_under_default_filter(|| {
            super::choose_budget(None, None, a18a_room(2_048));
        });
        let line = line_containing(&unanswered, RESPONSE_UNKNOWN_WINDOW_LINE);
        assert!(
            line.contains("WARN"),
            "対照: 照会したが取れなかった: {line}"
        );

        let explicit = TokenBudget::new(30_000, 3_000).expect("組めること");
        let chosen = super::budget_before_launch(Some(explicit), a18a_room(3_000));
        assert_eq!(
            (chosen.value(), chosen.on_fallback_window()),
            (explicit, false),
            "明示の予算は組んだまま使うこと"
        );
    }

    /// **落とし所の窓で既定の 1024 にした後、照会が成功したら面の値に戻る** —— 照会した窓が落とし所と同じ 8192 でも
    /// （窓の数だけを比べて「同じだから採らない」と言う実装を赤にする / レビュー是正）。
    ///
    /// 器の側も見る: **落とし所の窓の印**（送る値のログが理由を言い分ける）は、予算になる窓を照会できたら外れる ——
    /// 予算が同じまま（面の上限が 8192 以上で、窓 8192 では既定に戻る）でも外れる。
    #[test]
    fn a18a_a_known_window_after_the_fallback_window_restores_the_faces_room() {
        let restored = super::adopt_refreshed(TokenBudget::default(), Some(8_192), 2_048)
            .expect("窓が分かったので面の上限で組み直すこと");
        assert_eq!(
            (restored.context_tokens(), restored.response_tokens()),
            (8_192, 2_048)
        );
        assert_eq!(
            super::adopt_refreshed(TokenBudget::default(), Some(8_192), 1_024),
            None,
            "対照: 同じ予算なら採らない（5分ごとに鳴らさない）"
        );

        let shared = SharedBudget::chosen(Chosen::fallback_window(TokenBudget::default()));
        shared.set_body(a18a_room(2_048));
        assert!(shared.current().on_fallback_window(), "前提");
        assert!(!shared.adopt_window(None), "照会に失敗したら採らない");
        assert!(
            shared.current().on_fallback_window(),
            "照会に失敗したら印は残る"
        );
        assert!(
            !shared.adopt_window(Some(900)),
            "予算にならない窓は採らない"
        );
        assert!(
            shared.current().on_fallback_window(),
            "予算にならない窓では印は残る"
        );
        assert!(shared.adopt_window(Some(8_192)), "窓が分かったら採る");
        assert_eq!(
            (
                shared.get().response_tokens(),
                shared.current().on_fallback_window()
            ),
            (2_048, false)
        );

        let wide = SharedBudget::chosen(Chosen::fallback_window(TokenBudget::default()));
        wide.set_body(a18a_room(9_000));
        assert!(
            !wide.adopt_window(Some(8_192)),
            "前提: 窓 8192 では上限 9000 は既定に戻る＝予算は同じ"
        );
        assert!(
            !wide.current().on_fallback_window(),
            "予算が同じでも、窓が分かったら印は外れる（理由は窓の不足になる）"
        );
    }

    /// **引き直しの余白は「いまの余白」ではなく面の上限** —— 小さい窓で既定へ戻した後、窓が戻れば面の値に戻る
    /// （ADR-0059 §決定2 の「5分ごとの照会し直しで窓が変われば、同じ判定をやり直す」）。
    ///
    /// **いまの予算の余白（1024）を引き継ぐ実装は、戻る腕で赤になる。** 対照: 面に何も書いていなければ 1024 のまま。
    #[test]
    fn a18a_requery_restores_the_faces_room_when_the_window_grows_back() {
        let fell_back = TokenBudget::new(2_048, 1_024).expect("組めること");
        let restored = super::adopt_refreshed(fell_back, Some(16_384), 2_048)
            .expect("窓が変わったので新しい予算を採ること");
        assert_eq!(
            (restored.context_tokens(), restored.response_tokens()),
            (16_384, 2_048),
            "窓が戻ったら面の上限に戻すこと"
        );

        let shrunk = super::adopt_refreshed(restored, Some(2_048), 2_048)
            .expect("窓が変わったので新しい予算を採ること");
        assert_eq!(
            (shrunk.context_tokens(), shrunk.response_tokens()),
            (2_048, 1_024),
            "窓が上限以下へ縮んだら既定へ戻すこと"
        );

        let plain = super::adopt_refreshed(fell_back, Some(16_384), 1_024)
            .expect("窓が変わったので新しい予算を採ること");
        assert_eq!(
            plain.response_tokens(),
            1_024,
            "対照: 書いていなければ 1024"
        );
    }

    /// **環境変数の窓を使うときも、余白は本文の面の上限**（ADR-0059 §決定2 / [`super::explicit_main_budget`]）。
    ///
    /// 面に窓があれば面の予算をそのまま使う（組んだ時点で上限が入っている）。
    #[test]
    fn a18a_the_environment_window_carries_the_faces_room() {
        let environment = budget_of(12_288);
        let from_environment =
            super::explicit_main_budget(Some(environment), None, a18a_room(3_000))
                .expect("環境変数の窓があること");
        assert_eq!(
            (
                from_environment.context_tokens(),
                from_environment.response_tokens()
            ),
            (12_288, 3_000)
        );
        let profile = TokenBudget::new(30_000, 3_000).expect("組めること");
        assert_eq!(
            super::explicit_main_budget(Some(environment), Some(profile), a18a_room(3_000)),
            Some(profile),
            "面の窓が勝つこと"
        );
        assert_eq!(
            super::explicit_main_budget(Some(environment), None, BodyFace::default()),
            Some(environment),
            "対照: 書いていなければ環境変数の予算のまま"
        );

        // **面の窓があるときは、環境変数の窓で組み直さない**（A-18a の再レビュー是正）—— 環境変数の窓（2048）が面の上限
        // （3000）以下でも、使うのは面の予算なので、「既定に戻します」とは言わない（言うと事実と違う）。
        let small_environment = budget_of(2_048);
        let profile = TokenBudget::new(8_192, 3_000).expect("組めること");
        let output = captured_under_default_filter(|| {
            assert_eq!(
                super::explicit_main_budget(
                    Some(small_environment),
                    Some(profile),
                    a18a_room(3_000)
                ),
                Some(profile),
                "面の窓が勝つこと"
            );
        });
        assert!(
            !output.contains(RESPONSE_FALLBACK_LINE),
            "面の予算を使うのに、既定に戻すと言った: {output}"
        );
        // **対照: 面に窓が無ければ、同じ環境変数の窓では組めないので戻すと言う**（防御の枝。読み込みは止めている）。
        let output = captured_under_default_filter(|| {
            assert_eq!(
                super::explicit_main_budget(Some(small_environment), None, a18a_room(3_000)),
                Some(small_environment),
                "組めなければ環境変数の予算のまま"
            );
        });
        let line = line_containing(&output, RESPONSE_FALLBACK_LINE);
        assert!(line.contains("WARN"), "{line}");
    }

    /// **切り替えで、本文の面の値も予算と一緒に差し替わり、持ち越さない**（ADR-0059 §決定1・2 / [`install_face`]）。
    ///
    /// 1. 面の上限と思考の量を書いたプロファイルへ（窓なし）→ 落とし所の窓 8192 で、**余白は既定の 1024**
    ///    （窓がまだ分からないので面の上限を使わない / レビュー是正）。思考は `low`、器の面の値は書いたもの。
    ///    **窓を照会できたら面の上限 2048 に戻る**（[`SharedBudget::adopt_window`]）。
    /// 2. 何も書いていないプロファイルへ → 1024・`None` に戻る。
    #[test]
    fn a18a_switching_installs_the_body_face_with_the_budget() {
        use crate::llm::ReasoningEffort;

        let base_url = "http://127.0.0.1:9";
        let client = Arc::new(client_for(base_url));
        let clients = FaceClients::new(Arc::clone(&client), Arc::clone(&client));
        let pending = PendingRefresh::new(
            Chosen::measured(budget_of(LOADED)),
            None,
            Chosen::measured(SubContext::FALLBACK),
            None,
            upstream_facts(),
            CheckAtStart::NONE,
        );
        let budget = pending.shared();
        let sub_context = pending.sub_context();
        let upstream = pending.upstream();
        let thinking = BodyFace {
            reasoning_effort: Some(ReasoningEffort::Low),
            response_tokens: Some(2_048),
        };
        for (body, expected_after_query, expected_effort) in [
            (thinking, 2_048, Some(ReasoningEffort::Low)),
            (BodyFace::default(), 1_024, None),
        ] {
            install_face(
                &clients,
                ModelFace::Main,
                clients.face(ModelFace::Main).as_candidate(),
                &budget,
                &sub_context,
                &upstream,
                None,
                super::super::config::ProfileContext {
                    main: None,
                    sub: None,
                    body,
                },
            );
            let (now, effort) = budget.turn();
            assert_eq!(
                (now.context_tokens(), now.response_tokens(), effort),
                (8_192, 1_024, expected_effort),
                "{body:?}: 窓が分からない間は既定の 1024"
            );
            assert_eq!(budget.body(), body, "{body:?}");
            assert!(budget.current().on_fallback_window(), "{body:?}");

            budget.adopt_window(Some(16_384));
            assert_eq!(
                (
                    budget.get().context_tokens(),
                    budget.get().response_tokens()
                ),
                (16_384, expected_after_query),
                "{body:?}: 窓が分かったら面の上限"
            );
        }
    }
}
