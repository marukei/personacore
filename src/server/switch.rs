//! **走ったまま上流を差し替える**（[ADR-0042](../../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)
//! §決定4 / 作業単位 F-4b-2 / Issue origin#2）。
//!
//! ```text
//! POST /admin/upstream/profile { "name": "<プロファイル名>" }
//!   0. 同時に1つだけ（進行中なら 409。列に並べない）
//!   1. プロファイルを読み直す（利用者が書き換えた直後でも効く）
//!   2. 候補を全部組み、起こすものを全部確かめる        ← ここまでは何にも触らない
//!   3. 残さない子を止める → 終了を待つ                  ← launch::Supervisor::stop_except
//!   4. 変わる面のクライアントを差し替え、古い事実を捨てる ← context::install_face
//!   5. 新しく起こす（載るのは背景で待つ）               ← launch::Supervisor::start_more
//!   6. 外の上流の面は、その場で1回測り直す
//!   7. 選んだ名前を覚える（既定の置き場所のときだけ / ADR-0042 §決定9。書けなくても 200）
//!      いま選ばれている名前（`GET /admin/upstream` の `selected_profile` / F-10b）もここで変える
//! ```
//!
//! # 変わらない面は触らない
//!
//! **面ごとに新旧を比べる**（[`plan_switch`]）。比べるのは
//! **宛先（正規化した綴り）・依頼するモデルの綴り・その面を担う子の記述**の3つで、
//! **3つとも同じなら、クライアントも観測も子も1バイトも触らない。**
//! （のちに明示のコンテキスト長・鍵・承認・サンプラの値も比べる対象に足した —— [`Target`] の doc。）
//!
//! # 同じ名前で、起こすのを待っている面を起こす（ADR-0055 §決定3 / 作業単位 F-10a）
//!
//! **覚えた名前で起きた起動は、gguf の面を `not_started` のまま起こさない**（`App::bind_with`）。
//! **画面の「起こす」はこの入口に同じ名前を送る。** 待っている面は走っている子を持たないので、
//! 上の比べ方のまま「新しく起こす」になる——**経路は差し替えと1つ**（同時に1つだけ・409・
//! 接続が切れても最後まで走る・起こした面は載ってから測り直す、も同じ）。
//!
//! **待っている面が無いときに同じ名前を頼まれたら、今までどおり走っている子は触らない**（状態も変えない）。
//! **起こした後に `failed` になった子**（自分で終了した／別のモデルを名乗った。番人が書く）**も束に残る**ので、
//! 同じ名前では起こし直さない —— 起こし直すには、別のプロファイルへ切り替えてから戻る。
//! **起こすこと自体に失敗した面**（`command` が無い等。`Supervisor::start_more` が `failed` を書く）は
//! 子が束に入らないので、**同じ名前を頼むと、もう一度起こしに行く**（見張り番は `tests/upstream_cli.rs` の
//! `a_remembered_profile_whose_command_is_missing_waits_and_fails_only_when_asked`）。
//!
//! # 同時に1つだけ（**判定の置き場所は `Supervisor` の錠**）
//!
//! **子を止める・起こすのは `Supervisor` だけ**なので、**その錠を `try_lock` で取れなければ 409**。
//! 錠は 3〜6 の間だけ持つ ——**載るのを待つのは背景**（番人）なので、
//! **20 GB の gguf が載るまでの1〜2分、次の差し替えを断り続けることはしない。**
//! **載っている最中に差し替えたら、その子は止める**（自分の子である）。
//! **⚠ 背景の待ちまで「進行中」に数えると、載らないまま黙っている子（ADR-0035 で殺さない）から
//! 二度と差し替えられなくなる。**
//!
//! # 飛んでいる最中のターンは守らない（ADR-0042 §決定4）
//!
//! **子を止めた瞬間、そのターンは上流エラーで終わる**（`raw_content` はそこまでが保存される）。
//! **待って合流する経路は作らない。**
//!
//! # 依頼の接続が切れても、差し替えは最後まで走る（作業単位 B-56 / Issue origin#39）
//!
//! **0〜7 は依頼の接続から切り離したタスクで走り、ハンドラは結果を待つだけ**（[`run_detached`]）。
//! **ハンドラの中で直接待つと、画面を移った（POST が中断された）瞬間に途中で落ちる** ——
//! 止める段で落ちると、面は死んだ子の「準備完了」のまま残る（是正前に実測）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use serde::Deserialize;
use utoipa::ToSchema;

use super::AppState;
use super::config::{BindAddr, BodyFace, FaceSettings, ProfileContext};
use super::context::{Candidate, FaceClient};
use super::error::{ApiError, ErrorBody};
use super::launch::{Armed, FaceState, LaunchPlan, Supervisor};
use super::upstream::UpstreamReport;
use super::upstreams::{ProfileFace, ProfileLaunch, UpstreamProfile, load_profiles};
use crate::llm::{ApiKey, Sampling};
use crate::measured::ModelFace;

/// ログに出す入口の名前（`ApiError::from_json_rejection`）。
const ENDPOINT: &str = "admin/upstream/profile";

/// 面の並び（**添字は [`slot`] と揃える**）。
const FACES: [ModelFace; 2] = [ModelFace::Main, ModelFace::Sub];

const fn slot(face: ModelFace) -> usize {
    match face {
        ModelFace::Main => 0,
        ModelFace::Sub => 1,
    }
}

/// **差し替えの持ち主**（`AppState` が `Arc` で持つ）。
///
/// **子の束（`Supervisor`）をここへ移す**——`serve` の終わりに止める側と、
/// 差し替える側が**同じ錠**を通る。
pub(in crate::server) struct Switcher {
    supervisor: tokio::sync::Mutex<Supervisor>,
    /// **上流プロファイルの置き場所**（`main` が決めた場所 / ADR-0042 §決定1）。
    /// **`None` は「この起動は差し替えられない」**（決められない環境・テストの入口）。
    profiles_path: Option<PathBuf>,
    /// **受け付けた切り替えの名前を覚える先**（ADR-0042 §決定9 / 作業単位 F-4f）。
    /// **`None` は「覚えない」**（`PERSONACORE_UPSTREAMS` が指す先を読んでいる起動）。
    remember_to: Option<PathBuf>,
    /// **personacore 自身が実際に待ち受けているポート**（起こす面との取り合いを見る）。
    bind_port: u16,
    /// **終了処理が錠を取った印**（作業単位 B-56 のレビュー提案1）。
    ///
    /// **立てるのも読むのも [`Self::supervisor`] の錠の中だけ**——錠が順序を決めるので、
    /// 立った後に錠を取った差し替えは必ずこれを見る。**一度立ったら下ろさない。**
    closed: AtomicBool,
    /// **いま選ばれているプロファイルの名前**（`GET /admin/upstream` の `selected_profile` /
    /// 作業単位 F-10b）。**`None` は「選んでいない」**（環境変数の既定で起きた）。
    ///
    /// **書くのは [`Self::switch`] が差し替えを終えた後だけ**（[`Self::supervisor`] の錠の中）。
    /// 断った切り替えは何にも触らないので、ここも変えない。
    ///
    /// **[`Self::supervisor`] とは別の錠にする** —— 読むのは `GET /admin/upstream` で、
    /// 差し替えの最中（子の終了を待つ間）に読みを待たせない。`await` を跨いで持たない。
    selected: Mutex<Option<String>>,
}

/// **送るサンプラの行の `occasion`**（切り替えの後 / `context::report_samplers`）。
const SWITCH_OCCASION: &str = "switch";

/// **終了処理に入ったので差し替えを断った行**（テストがこの綴りで引く）。
const SWITCH_CLOSED_LINE: &str = "終了処理に入ったので、この上流の差し替えは断りました";

/// **終了処理が、進行中の差し替えを待ってから子を止めると言う行**（錠を取る前に出す）。
///
/// **`tests/upstream_launch.rs` の `a_shutdown_waits_for_a_running_switch_and_stops_what_it_started`
/// がこの綴りで「終了処理が始まった瞬間」を引く**——綴りを変えるなら、そちらも。
const SHUTDOWN_LINE: &str =
    "上流の差し替えを閉じます（進行中の差し替えがあれば、終わってから子を止めます）";

impl Switcher {
    pub(in crate::server) fn new(
        supervisor: Supervisor,
        profiles_path: Option<PathBuf>,
        remember_to: Option<PathBuf>,
        bind_port: u16,
        selected: Option<String>,
    ) -> Self {
        Self {
            supervisor: tokio::sync::Mutex::new(supervisor),
            profiles_path,
            remember_to,
            bind_port,
            closed: AtomicBool::new(false),
            selected: Mutex::new(selected),
        }
    }

    /// **いま選ばれているプロファイルの名前**（[`Self::selected`] / 作業単位 F-10b）。
    ///
    /// **錠が毒されていても読む**（`PoisonError::into_inner`）——ここで panic すると、
    /// 上流の様子を読む口ごと落ちる。中身は `Option<String>` の置き換えだけなので壊れない。
    pub(in crate::server) fn selected_profile(&self) -> Option<String> {
        self.selected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// **控えた子を全部止め、終了を待つ**（`serve` の終わり / ADR-0042 §決定2）。
    ///
    /// **進行中の差し替えがあれば、それが終わるのを待ってから止める**（同じ錠）。
    /// 差し替えが錠を持つのは起こすところまで（載るのは待たない）なので、長くは待たない。
    /// **`try_lock` にしないこと** —— 進行中の差し替えが起こした子を、誰も止めなくなる
    /// （`kill_on_drop` は終了を待たない / `launch` の doc）。
    ///
    /// # 錠の中で「閉じた」印を立てる（作業単位 B-56 のレビュー提案1）
    ///
    /// **差し替えは依頼の接続から切り離したタスクで走る**（[`run_detached`]）ので、
    /// **`tokio::spawn` から [`Self::switch`] の `try_lock` に届くまでの間に、
    /// この関数が先に錠を取って返りうる。** 印が無いと、その差し替えは止まった束へ
    /// 新しい子を起こす（[`Supervisor::stop`] は照会先を落とさないので、断る枝に当たらない）
    /// ——**止める人がもう居ない子**になる。**印を見た差し替えは何にも触らずに断る。**
    pub(in crate::server) async fn shutdown(&self) {
        tracing::info!("{SHUTDOWN_LINE}");
        let mut supervisor = self.supervisor.lock().await;
        self.closed.store(true, Ordering::SeqCst);
        supervisor.stop().await;
    }

    /// **名前で選んだプロファイルへ差し替える。**
    async fn switch(&self, name: &str) -> Result<(), ApiError> {
        // **同時に1つだけ。列に並べない**（ADR-0042 §決定4）。
        let Ok(mut supervisor) = self.supervisor.try_lock() else {
            tracing::warn!(
                upstream_profile = ?name,
                "上流の差し替えが進行中なので、この差し替えは断りました"
            );
            return Err(ApiError::upstream_switch_in_progress());
        };

        // **終了処理が先に錠を取った後なら、何にも触らずに断る**（[`Self::shutdown`] の doc）。
        // **錠を取った直後に見る**——ここより後ろは子を起こしうる。
        if self.closed.load(Ordering::SeqCst) {
            tracing::info!(upstream_profile = ?name, "{SWITCH_CLOSED_LINE}");
            return Err(ApiError::upstream_switch_unavailable());
        }

        let (Some(path), Some(armed)) =
            (self.profiles_path.as_deref(), supervisor.armed().cloned())
        else {
            tracing::info!(
                upstream_profile = ?name,
                "この起動は上流を差し替えられません（プロファイルの置き場所か照会先がありません）"
            );
            return Err(ApiError::upstream_switch_unavailable());
        };

        // **毎回読み直す**——画面で「gguf のパスを書き換えて選び直す」が効くように。
        let profiles = load_profiles(path).map_err(|err| {
            tracing::warn!(
                upstream_profiles_path = %path.display(),
                error = %err,
                cause = ?std::error::Error::source(&err),
                "上流プロファイルを読めませんでした（差し替えていません）"
            );
            ApiError::upstream_profiles_unreadable()
        })?;
        let profile = profiles.select(name).map_err(|err| {
            tracing::info!(
                upstream_profile = ?name,
                cause = ?std::error::Error::source(&err),
                "指定された上流プロファイルがありません（差し替えていません）"
            );
            ApiError::upstream_profile_not_found()
        })?;

        let prepared = prepare(profile, &supervisor, &armed, self.bind_port)?;
        tracing::info!(
            upstream_profile = ?name,
            main = ?prepared.plan.touch[slot(ModelFace::Main)],
            sub = ?prepared.plan.touch[slot(ModelFace::Sub)],
            stopping = supervisor
                .running()
                .len()
                .saturating_sub(prepared.plan.keep.len()),
            starting = prepared.plan.start.len(),
            "上流を差し替えます"
        );
        execute(&mut supervisor, &armed, prepared).await;
        tracing::info!(upstream_profile = ?name, "上流を差し替えました");

        // **差し替えたので、いま選ばれている名前にする**（作業単位 F-10b）。
        // **錠を持ったまま書く**——続けて来た切り替えと書く順が入れ替わらない（覚える名前と同じ理由）。
        *self.selected.lock().unwrap_or_else(PoisonError::into_inner) = Some(name.to_owned());

        // **受け付けたので覚える**（ADR-0042 §決定9）。**載り終わるのは待たない。**
        // **錠を持ったまま書く**——続けて来た切り替えと書く順が入れ替わらない（最後に受け付けた名前が残る）。
        // **書けなくても 200**（警告だけ）。
        if let Some(memory) = self.remember_to.as_deref() {
            super::remembered::remember_or_warn(memory, name);
        }
        Ok(())
    }
}

// ----------------------------------------------------------------------
// 何を変えるか（**純粋関数**）
// ----------------------------------------------------------------------

/// **1つの面の、比べる3つ**（正規化した宛先と、依頼するモデルの綴りと、明示のコンテキスト長）。
///
/// # 明示のコンテキスト長も比べる（ADR-0054 §決定3 / 作業単位 A-13a-1）
///
/// **宛先と綴りが同じで値だけが違うプロファイルへ切り替えても、値は切り替え先に追従する。**
/// 比べなければ、その面は「触らない」になり、**前の値のまま組み続ける**。
/// **比べるのは「明示する値」**（本文は面の値、無ければ起動時の環境変数 /
/// `launch::LoadSinks::next_explicit_tokens`）——環境変数と同じ値を面に書いただけの切り替えは触らない。
///
/// # 鍵も比べる（ADR-0056 §決定2 / 作業単位 A-13b-1）
///
/// **宛先と綴りが同じで鍵だけが違うプロファイルへ切り替えても、鍵は切り替え先に追従する。**
/// 比べなければ「触らない」になり、**前の鍵のまま送り続ける**（鍵を作り直した上流は 401 を返す）。
/// `Debug` は伏せる（`llm::ApiKey`）。
///
/// # 承認も比べる（ADR-0056 §決定5 / 作業単位 A-13b-2）
///
/// **宛先・綴り・鍵が同じで承認だけが違うプロファイルへ切り替えても、承認は切り替え先に追従する。**
/// 比べなければ「触らない」になり、**前の承認のまま送り続ける**（承認を足したのに止まり続ける・外したのに送り続ける）。
///
/// # サンプラの値も比べる（ADR-0058 §決定2 / 作業単位 A-16）
///
/// **宛先・綴りが同じで `sampling` だけが違うプロファイルへ切り替えても、値は切り替え先に追従する。**
/// 比べなければ「触らない」になり、**前の値のまま送り続ける**（同じ量子化で値だけを変えて比べる使い方で、
/// 比べたつもりの2つの腕が同じ値で走る）。見張り番は `tests/server_profile_sampling.rs` の腕5。
///
/// # 本文の面の思考の量と返答の上限も比べる（ADR-0059 §決定4 の腕4 / 作業単位 A-18a）
///
/// **宛先・綴りが同じで `reasoning_effort` / `response_tokens` だけが違うプロファイルへ切り替えても、
/// 値（と予算の余白）は切り替え先に追従する。** 比べなければ「触らない」になり、本文の予算の器
/// （`context::SharedBudget`）を差し替えないので、**前の思考の量・前の上限のまま送り続ける。**
/// **本文の面だけが値を持つ**（裏方の面は常に既定 —— [`BodyFace::default`]）。
/// 見張り番は `tests/server_profile_thinking.rs` の腕4と、`tests::a_face_whose_body_settings_alone_change_is_touched`。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    endpoint: String,
    model: String,
    context_tokens: Option<u32>,
    api_key: Option<ApiKey>,
    allow_key_over_plain_http: bool,
    sampling: Sampling,
    body: BodyFace,
}

impl Target {
    fn of_client(client: &FaceClient, context_tokens: Option<u32>, body: BodyFace) -> Self {
        Self {
            endpoint: client.endpoint(),
            model: client.model(),
            context_tokens,
            api_key: client.api_key(),
            allow_key_over_plain_http: client.key_over_plain_http_allowed(),
            sampling: client.sampling(),
            body,
        }
    }

    fn of_candidate(candidate: &Candidate, context_tokens: Option<u32>, body: BodyFace) -> Self {
        Self {
            endpoint: candidate.endpoint().to_owned(),
            model: candidate.model().to_owned(),
            context_tokens,
            api_key: candidate.api_key().cloned(),
            allow_key_over_plain_http: candidate.key_over_plain_http_allowed(),
            sampling: candidate.sampling(),
            body,
        }
    }
}

/// **その面が持つ本文の面の値**（本文の面だけが値を持ち、裏方の面は常に既定 / [`Target`] の doc）。
fn body_of(face: ModelFace, body: BodyFace) -> BodyFace {
    match face {
        ModelFace::Main => body,
        ModelFace::Sub => BodyFace::default(),
    }
}

/// **その面をどう扱うか。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Touch {
    /// **触らない**（宛先・綴り・担う子が全部同じ）。
    Untouched,
    /// **外の上流になる**——クライアントを差し替えて、その場で測り直す。状態は `external`。
    External,
    /// **残す子が担ったまま、綴りだけが変わる**——クライアントを差し替えて測り直す。
    /// **状態はその子のもの**（触らない）。
    KeptChild,
    /// **新しく起こす子が担う**——クライアントを差し替え、載ったら測り直す（番人）。
    Launched,
}

/// **何を止め、何を起こし、どの面を触るか。**
#[derive(Debug, Clone, PartialEq, Eq)]
struct SwitchPlan {
    /// **残す子**（いま走っていて、新しいプロファイルにも同じ記述で在るもの）。
    keep: Vec<ProfileLaunch>,
    /// **新しく起こす子。**
    start: Vec<ProfileLaunch>,
    /// 面ごとの扱い（添字は [`slot`]）。
    touch: [Touch; 2],
}

/// その面を担う子。
fn carrier(launches: &[ProfileLaunch], face: ModelFace) -> Option<&ProfileLaunch> {
    launches
        .iter()
        .find(|launch| launch.faces().contains(&face))
}

/// **新旧を比べて、何を変えるかを決める。純粋関数**（ADR-0042 §決定4）。
///
/// # 子は「記述が同じなら残す」
///
/// **子の記述**（[`ProfileLaunch`]）は gguf・宛先・引数・`command`・**担う面**を全部含む。
/// **1つでも違えば止めて起こし直す**——引数だけを変えたプロファイルに差し替えたのに、
/// 前の引数の子が答え続ける形を作らない。
///
/// # 面の扱い
///
/// | 条件（上から順に） | 扱い |
/// |---|---|
/// | 担う子を**新しく起こす** | [`Touch::Launched`] |
/// | 担っていた子を**止める**、**起こすのを待っていた**（`waiting`）、または宛先か綴りが変わる | 担う子が残るなら [`Touch::KeptChild`]、無ければ [`Touch::External`] |
/// | どれも同じ | [`Touch::Untouched`] |
///
/// # 起こすのを待っていた面（`waiting` / ADR-0055 / 作業単位 F-10a）
///
/// **覚えた名前で起きた起動は、gguf の面を起こさずに `not_started` で待つ**（`App::bind_with`）。
/// その面は**走っている子を持たない**ので、同じプロファイルを頼まれると
/// 「担う子を新しく起こす」に当たり、**そのまま起こされる**（ADR-0055 §決定3）。
///
/// **起こさないプロファイルを頼まれたときに、比べるだけでは足りない** ——
/// 宛先と綴りが同じだと「触らない」になり、**起こされない面が `not_started` を言い続ける。**
/// **だから「止める」と同じ扱いにして、外の上流へ倒す**（その場で測る）。
fn plan_switch(
    running: &[ProfileLaunch],
    wanted: &[ProfileLaunch],
    current: &[Target; 2],
    next: &[Target; 2],
    waiting: [bool; 2],
) -> SwitchPlan {
    let keep: Vec<ProfileLaunch> = running
        .iter()
        .filter(|launch| wanted.contains(launch))
        .cloned()
        .collect();
    let start: Vec<ProfileLaunch> = wanted
        .iter()
        .filter(|launch| !keep.contains(launch))
        .cloned()
        .collect();

    let touch = FACES.map(|face| {
        let new_child = carrier(wanted, face);
        let old_child = carrier(running, face);
        let started = new_child.is_some_and(|child| !keep.contains(child));
        let stopped = old_child.is_some_and(|child| !keep.contains(child));
        if started {
            Touch::Launched
        } else if stopped || waiting[slot(face)] || current[slot(face)] != next[slot(face)] {
            if new_child.is_some() {
                Touch::KeptChild
            } else {
                Touch::External
            }
        } else {
            Touch::Untouched
        }
    });

    SwitchPlan { keep, start, touch }
}

// ----------------------------------------------------------------------
// 何にも触らずに確かめる
// ----------------------------------------------------------------------

/// **組み終えた候補と計画。** ここまでは何にも触っていない。
struct Prepared {
    plan: SwitchPlan,
    /// 触る面の新しいクライアント（添字は [`slot`]。触らない面は `None`）。
    candidates: [Option<Candidate>; 2],
    /// **切り替え先の面に書いたコンテキスト長**（ADR-0054 / 作業単位 A-13a-1）。
    profile_context: ProfileContext,
}

/// **切り替え先の面から写す、宛先とモデル以外の値**（鍵・承認・サンプラの値 / `config::retarget_llm`）。
fn face_settings(face: &ProfileFace) -> FaceSettings<'_> {
    FaceSettings {
        api_key: face.api_key(),
        allow_key_over_plain_http: face.allow_key_over_plain_http(),
        sampling: face.sampling(),
    }
}

/// **候補を全部組み、起こすものを全部確かめる。** 失敗したら何にも触らずに断る。
///
/// | 確かめること | 断るとき |
/// |---|---|
/// | 宛先とモデルの綴り（起動時と同じ検証） | 422 `upstream_profile_unusable` |
/// | **新しく起こす** gguf の実在（ADR-0042 §決定2） | 422 `upstream_gguf_missing` |
/// | **新しく起こす**面と personacore 自身の待ち受けの取り合い（B-52） | 422 `upstream_port_collision` |
///
/// **残す子の gguf は見ない**——走っている子が掴んでいる。
fn prepare(
    profile: &UpstreamProfile,
    supervisor: &Supervisor,
    armed: &Armed,
    bind_port: u16,
) -> Result<Prepared, ApiError> {
    let clients = &armed.clients;
    let unusable = |face: ModelFace| {
        move |err: super::context::ClientBuildError| {
            tracing::warn!(
                upstream_face = %face.label(),
                error = %err,
                cause = ?std::error::Error::source(&err),
                "差し替え先の上流の綴りが不正です（差し替えていません）"
            );
            ApiError::upstream_profile_unusable()
        }
    };

    let main_face = profile.main();
    // **鍵は切り替え先の面のもの**（ADR-0056 §決定2 / `config::retarget_llm`）。**承認も**（§決定5）。
    // **サンプラの値も**（ADR-0058 §決定2 —— 切り替え先に値が無ければ既定へ戻る。持ち越さない）。
    let main = clients
        .face(ModelFace::Main)
        .retargeted(
            main_face.endpoint(),
            main_face.model(),
            face_settings(main_face),
        )
        .map_err(unusable(ModelFace::Main))?;
    // **`sub` を書いていなければ、抽出も本文と同じ上流**（ADR-0039 §決定1）。
    // **比べる綴りは、本文の候補の宛先と、本文と同じモデル**（`UpstreamProfile::settings`）。
    // **鍵も本文の候補のもの**（同じ宛先・本文のクライアントの複製 —— 下の `with_model`）。
    // **サンプラの値も本文の候補のもの**（ADR-0058 §決定1 —— `with_model` の複製に写る）。
    let sub_written = match profile.sub() {
        Some(sub) => Some(
            clients
                .face(ModelFace::Sub)
                .retargeted(sub.endpoint(), sub.model(), face_settings(sub))
                .map_err(unusable(ModelFace::Sub))?,
        ),
        None => None,
    };
    // **面に書いたコンテキスト長**（ADR-0054 §決定1）。**起動時と同じ検証を通す**
    // （`AppConfig::with_profile_context` と同じ関数）——読み込みで検証済みなので、ここで落ちるのは
    // 規則が2つに割れたときだけ。
    //
    // **本文の面の返答の上限が起動時の `PERSONACORE_CONTEXT_TOKENS` に収まるかも、ここで見る**
    // （ADR-0059 §決定2 —— 面に窓が無いプロファイルでは、読み込みは環境変数を見ていない）。
    let profile_context = ProfileContext::of(&profile.settings(), armed.sinks.environment())
        .map_err(|err| {
            tracing::warn!(
                error = %err,
                "差し替え先の上流プロファイルの context_tokens / response_tokens が使えません（差し替えていません）"
            );
            ApiError::upstream_profile_unusable()
        })?;
    let next_context = |face: ModelFace| armed.sinks.next_explicit_tokens(face, profile_context);

    let next_sub = sub_written.as_ref().map_or_else(
        || Target {
            endpoint: main.endpoint().to_owned(),
            model: main_face.model().to_owned(),
            context_tokens: next_context(ModelFace::Sub),
            api_key: main.api_key().cloned(),
            allow_key_over_plain_http: main.key_over_plain_http_allowed(),
            sampling: main.sampling(),
            // **`sub` 面が無くても、本文の面の値は裏方へ写さない**（ADR-0059 §決定1）。
            body: body_of(ModelFace::Sub, profile_context.body),
        },
        |candidate| {
            Target::of_candidate(
                candidate,
                next_context(ModelFace::Sub),
                body_of(ModelFace::Sub, profile_context.body),
            )
        },
    );

    // **本文の面の値は、いまの本文の予算の器から**（ADR-0059 / [`Target`] の doc）。
    let current = FACES.map(|face| {
        Target::of_client(
            clients.face(face),
            armed.sinks.explicit_tokens(face),
            body_of(face, armed.sinks.body()),
        )
    });
    let next = [
        Target::of_candidate(
            &main,
            next_context(ModelFace::Main),
            body_of(ModelFace::Main, profile_context.body),
        ),
        next_sub,
    ];
    // **起こすのを待っている面**（ADR-0055 / 作業単位 F-10a / [`plan_switch`] の doc）。
    let states = supervisor.states().get();
    let waiting = FACES.map(|face| *states.face(face) == FaceState::NotStarted);
    let plan = plan_switch(
        &supervisor.running(),
        profile.launches(),
        &current,
        &next,
        waiting,
    );

    // **新しく起こすものだけを、起動時と同じ関数で確かめる。**
    let starting = LaunchPlan::from_launches(plan.start.clone());
    if let Some(missing) = starting.missing_gguf() {
        tracing::warn!(error = %missing, "差し替え先の gguf がありません（差し替えていません）");
        return Err(ApiError::upstream_gguf_missing());
    }
    if let Some(collision) = starting.port_conflict(BindAddr::loopback(bind_port)) {
        tracing::warn!(
            error = %collision,
            "差し替え先の宛先が personacore 自身の待ち受けと同じです（差し替えていません）"
        );
        return Err(ApiError::upstream_port_collision());
    }

    // **抽出の面を本文から複製するときは、差し替え後の本文と接続プールを共有する**
    // （起動時の `App::bind` と同じ形）。本文を触らないなら、いまの本文のクライアントから。
    let sub = match sub_written {
        Some(sub) => sub,
        None => {
            let shared = if plan.touch[slot(ModelFace::Main)] == Touch::Untouched {
                clients
                    .face(ModelFace::Main)
                    .as_candidate()
                    .with_model(main_face.model())
            } else {
                main.with_model(main_face.model())
            };
            shared.map_err(unusable(ModelFace::Sub))?
        }
    };

    let mut candidates = [Some(main), Some(sub)];
    for face in FACES {
        if plan.touch[slot(face)] == Touch::Untouched {
            candidates[slot(face)] = None;
        }
    }
    Ok(Prepared {
        plan,
        candidates,
        profile_context,
    })
}

// ----------------------------------------------------------------------
// 差し替える
// ----------------------------------------------------------------------

/// **止める → 終了を待つ → 差し替える → 起こす → 測り直す**（ADR-0042 §決定4）。
///
/// **この順を崩さない。** とくに「止めて、終了を待ってから起こす」——
/// 待たずに起こすと、同じポート・同じ GPU を前の子と取り合う（ADR-0042 §決定2）。
async fn execute(supervisor: &mut Supervisor, armed: &Armed, prepared: Prepared) {
    let Prepared {
        plan,
        mut candidates,
        profile_context,
    } = prepared;

    // 1. **残さない子を止め、終了を待つ。**
    let freed = supervisor.stop_except(&plan.keep).await;
    if !freed.is_empty() {
        tracing::info!(stopped_faces = ?freed, "差し替えのために、起こした上流を止めました");
    }

    // 2. **変わる面のクライアントを差し替え、古い事実を捨てる**（起こす前に —— 番人は新しい口へ聞く）。
    //    **明示のコンテキスト長も切り替え先のものにする**（ADR-0054 §決定3）。
    for face in FACES {
        let Some(candidate) = candidates[slot(face)].take() else {
            continue;
        };
        armed
            .sinks
            .install(&armed.clients, face, candidate, profile_context);
        if plan.touch[slot(face)] == Touch::External {
            supervisor.states().set(face, FaceState::External);
        }
    }

    // 3. **起こす。** 載るのは背景で待つ（番人は起こした場で付く）。
    supervisor.start_more(&plan.start);

    // 4. **外の上流は、その場で1回測り直す**（起こした面は載ったときに番人が測る）。
    for face in FACES {
        if matches!(plan.touch[slot(face)], Touch::External | Touch::KeptChild) {
            let (probe, failure) = super::context::probe_face(&armed.clients, face).await;
            if let Some(err) = failure {
                tracing::info!(
                    upstream_face = %face.label(),
                    error = %err,
                    "差し替えた上流へ照会できませんでした（次の引き直しで測り直します）"
                );
            }
            armed.sinks.adopt(face, &probe);
        }
    }

    // **送るサンプラの値を面ごとに言う**（ADR-0058 §決定3）。触らなかった面も言う —— 切り替えの後に
    // 何で送るかを、ログの1か所で読めるように。
    // **本文の面の思考の量と返答の上限も同じ行で言う**（ADR-0059 §決定1・2）。
    //
    // **言うのは外の上流を測り直した後**（A-18a のレビュー是正）—— 差し替えた直後の器は、窓がまだ分からないので
    // 返答の上限が既定の 1024 になっている（`context::install_face`）。測り直した後なら、外の上流では次の本文が
    // 実際に使う上限を言える。起こす上流はまだ載っていないので、窓が分からないまま（`window_unknown`）言う。
    super::context::report_samplers(
        &armed.clients,
        armed.sinks.body(),
        armed.sinks.main_budget(),
        SWITCH_OCCASION,
    );
}

// ----------------------------------------------------------------------
// 依頼の接続から切り離す（作業単位 B-56 / Issue origin#39）
// ----------------------------------------------------------------------

/// **走らせた差し替えが落ちた（panic した）ときの行。** テストがこの綴りで引く。
const DETACHED_PANIC_LINE: &str =
    "上流の差し替えが途中で落ちました（面の状態と照会先が食い違っているかもしれません）";

/// **差し替えを依頼の接続の寿命から切り離して最後まで走らせ、結果だけを待つ**
/// （作業単位 B-56 / Issue origin#39）。
///
/// # なぜ切り離すのか（**是正前に実測した**）
///
/// **axum は接続が切れるとハンドラの future を落とす**（`relay` の doc /
/// `tests/server_chat_cancel.rs`）。ハンドラの中で差し替えを直接待っていたので、
/// **止める段の途中（`kill` と終了待ちの間）で接続を切ると、future ごと落ちた。**
/// 前の子は `kill_on_drop` で死ぬのに、[`Supervisor::stop_except`] は状態を書かず、
/// クライアントの差し替えも次の子を起こすことも走らない
/// ——**面は死んだ子の「準備完了」を言い続け、照会先も死んだ子の宛先のままだった**
/// （`tests/upstream_launch.rs` の `a_switch_cut_off_while_stopping_the_old_child_still_finishes`
/// が是正前に 5 回中 5 回赤）。画面の側では、`#/upstream` から移ると POST が中断される。
///
/// # 「止める段で `stopped` を書く」形は採らなかった
///
/// * **落ちうる場所は止める段だけではない** —— 差し替えた後の測り直し（[`execute`] の 4。
///   外の上流への照会を `await` する）でも落ちる（**読んだ限り。測っていない**）。
///   そこで落ちると、選んだ名前を覚えるところ（[`Switcher::switch`] の終わり）まで届かず、
///   **「受け付けたのに覚えていない」**まま残る。止める段だけを手当てしても、そこは残る。
/// * **利用者が選んだ切り替えが、画面を移っただけで取り消される**（`stopped` のまま何も起きない）。
///
/// # 失敗を握りつぶさない
///
/// * **差し替えの断り（`Err`）は、起きた場所で既にログに出ている**（[`Switcher::switch`] の各枝）。
///   依頼した側が居なくなっていても、その行は残る。
/// * **落ちた（panic）ことは、依頼した側が居なくても言う** —— 結果を見届けるのは
///   別のタスク（`reporter`）で、ハンドラはその結果を待つだけ。ハンドラが落ちても
///   `reporter` は残り、`JoinError` を `error` で出す。**ハンドラの中で `JoinError` を
///   読む形にすると、依頼した側が居ないときに誰も読まない。**
///
/// # 錠は変わらない
///
/// **同時に1つだけ**（409）の判定は、走らせた先の [`Switcher::switch`] が錠で行う。
/// **ハンドラが落ちた後も、走っている差し替えは錠を持ち続ける**ので、
/// その間の次の依頼は今までどおり 409 になる。
///
/// # 終了処理との隙（作業単位 B-56 のレビュー提案1）
///
/// **終了時の [`Switcher::shutdown`] は同じ錠を `lock().await` で待ち、錠の中で「閉じた」印を立てる。**
///
/// | 差し替えが錠を取ったのは | どうなるか |
/// |---|---|
/// | 終了処理より**前** | 終了処理は差し替えが終わるのを待ち、**その差し替えが起こした子も止めてから終わる** |
/// | 終了処理より**後**（`tokio::spawn` から `try_lock` までの隙） | **印を見て、何にも触らずに断る**（`upstream_switch_unavailable`）——終了の後に子を起こさない |
///
/// **1行目の見張り番は `tests/upstream_launch.rs` の
/// `a_shutdown_waits_for_a_running_switch_and_stops_what_it_started`、
/// 2行目は `tests::a_switch_that_reaches_the_lock_after_the_shutdown_is_refused`**
/// （隙そのものは外から決定的に作れないので、印の枝を単体で見る）。
async fn run_detached<F>(work: F) -> Result<(), ApiError>
where
    F: std::future::Future<Output = Result<(), ApiError>> + Send + 'static,
{
    let work = tokio::spawn(work);
    let reporter = tokio::spawn(async move {
        work.await.unwrap_or_else(|err| {
            tracing::error!(error = %err, "{DETACHED_PANIC_LINE}");
            Err(ApiError::internal())
        })
    });
    reporter.await.unwrap_or_else(|err| {
        // **見届け役は panic しない**（上の `unwrap_or_else` だけ）。ここに来るのは
        // ランタイムが止まって abort されたときで、依頼した側もそこで終わる。
        tracing::error!(error = %err, "上流の差し替えの見届け役が途中で止まりました");
        Err(ApiError::internal())
    })
}

// ----------------------------------------------------------------------
// `POST /admin/upstream/profile`
// ----------------------------------------------------------------------

/// 差し替えの依頼。**`deny_unknown_fields`**（`/admin/` の全入口で共通の方針）。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SwitchRequest {
    /// `upstreams.json` に利用者が書いたプロファイルの名前。
    name: String,
}

/// `POST /admin/upstream/profile` — **走ったまま上流を差し替える**（ADR-0042 §決定4）。
///
/// **返すのは `GET /admin/upstream` と同じ形**（差し替えた直後の状態）。
/// **起こした面は `loading` で返る**——載るのは背景で待つ（ADR-0042 §決定3）。
///
/// **依頼の接続が途中で切れても、受け付けた差し替えは最後まで走る**（[`run_detached`] / 作業単位 B-56）。
///
/// | 応答 | いつ |
/// |---|---|
/// | 200 | 差し替えた（**起こせなかった面は `failed` として載る**） |
/// | 404 `upstream_profile_not_found` | その名前が無い |
/// | 409 `upstream_switch_in_progress` | **別の差し替えが進行中**（列に並べない） |
/// | 409 `upstream_switch_unavailable` | この起動はプロファイルの置き場所を持たない |
/// | 422 | ファイルが読めない／綴りが不正／gguf が無い／待ち受けと取り合う（**何にも触っていない**） |
#[utoipa::path(
    post,
    path = "/admin/upstream/profile",
    summary = "Switch the upstream to a saved profile while running",
    tag = "upstream",
    request_body = SwitchRequest,
    responses(
        (status = 200, body = UpstreamReport),
        (status = "default", body = ErrorBody),
    ),
)]
pub(super) async fn switch_profile(
    State(state): State<AppState>,
    body: Result<Json<SwitchRequest>, JsonRejection>,
) -> Result<Json<UpstreamReport>, ApiError> {
    let Json(request) =
        body.map_err(|rejection| ApiError::from_json_rejection(rejection, ENDPOINT))?;
    // **接続が切れても、差し替えは最後まで走る**（[`run_detached`] / 作業単位 B-56）。
    let switcher = std::sync::Arc::clone(state.switcher());
    run_detached(async move { switcher.switch(&request.name).await }).await?;
    Ok(Json(UpstreamReport::of(
        &state.upstream(),
        &state.launch_states(),
        state.profile_sample(),
        state.switcher().selected_profile().as_deref(),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::upstreams::parse_profiles;

    fn target(endpoint: &str, model: &str) -> Target {
        Target {
            endpoint: endpoint.to_owned(),
            model: model.to_owned(),
            context_tokens: None,
            api_key: None,
            allow_key_over_plain_http: false,
            sampling: Sampling::default(),
            body: BodyFace::default(),
        }
    }

    /// **宛先・綴りが同じで、本文の面の思考の量か返答の上限だけが変わった面は触る**
    /// （ADR-0059 §決定4 の腕4 / 作業単位 A-18a）。
    ///
    /// **本文の面だけ**を変えて見る —— 裏方の面は同じまま触らない。**対照**: 同じ値なら触らない。
    /// 値は既定（`none` / 1024）と違える（2048・1536）。思考の量と上限を別々に変える腕を置く
    /// （片方だけを比べる実装を、もう片方の腕が赤にする）。
    #[test]
    fn a_face_whose_body_settings_alone_change_is_touched() {
        use crate::llm::ReasoningEffort;
        let body = |effort: Option<ReasoningEffort>, room: Option<u32>| Target {
            body: BodyFace {
                reasoning_effort: effort,
                response_tokens: room,
            },
            ..target("http://127.0.0.1:1/v1/chat/completions", "main")
        };
        let sub = target("http://127.0.0.1:2/v1/chat/completions", "sub");
        let low = Some(ReasoningEffort::Low);
        for (before, after, touched) in [
            (body(None, None), body(low, None), true),
            (body(low, None), body(None, None), true),
            (body(None, None), body(None, Some(2048)), true),
            (body(None, Some(2048)), body(None, Some(1536)), true),
            (body(low, Some(2048)), body(None, None), true),
            (body(low, Some(2048)), body(low, Some(2048)), false),
            (body(None, None), body(None, None), false),
        ] {
            let plan = plan_switch(
                &[],
                &[],
                &[before.clone(), sub.clone()],
                &[after.clone(), sub.clone()],
                [false, false],
            );
            assert_eq!(
                plan.touch,
                [
                    if touched {
                        Touch::External
                    } else {
                        Touch::Untouched
                    },
                    Touch::Untouched,
                ],
                "{:?} → {:?}",
                before.body,
                after.body
            );
        }
    }

    /// **宛先・綴りが同じで、サンプラの値だけが変わった面は触る**（ADR-0058 §決定2 / 作業単位 A-16）。
    ///
    /// **3通り**（なし→あり・あり→別の値・あり→なし）を、**本文の面だけ**変えて見る ——裏方の面は同じまま
    /// 触らない。**対照**: 同じ値なら触らない。値は既定の 1.1 と違える。
    #[test]
    fn a_face_whose_sampling_alone_changes_is_touched() {
        use crate::llm::RepeatPenalty;
        let tuned = |value: Option<f64>| Target {
            sampling: value.map_or_else(Sampling::default, |value| {
                Sampling::with_repeat_penalty(RepeatPenalty::parse(value).expect("範囲の中"))
            }),
            ..target("http://127.0.0.1:1/v1/chat/completions", "main")
        };
        let sub = target("http://127.0.0.1:2/v1/chat/completions", "sub");
        for (before, after, touched) in [
            (None, Some(1.05), true),
            (Some(1.05), Some(1.2), true),
            (Some(1.05), None, true),
            (Some(1.05), Some(1.05), false),
            (None, None, false),
        ] {
            let plan = plan_switch(
                &[],
                &[],
                &[tuned(before), sub.clone()],
                &[tuned(after), sub.clone()],
                [false, false],
            );
            assert_eq!(
                plan.touch,
                [
                    if touched {
                        Touch::External
                    } else {
                        Touch::Untouched
                    },
                    Touch::Untouched,
                ],
                "{before:?} → {after:?}"
            );
        }
    }

    /// **宛先・綴り・鍵が同じで、承認だけが変わった面は触る**（ADR-0056 §決定5 / 作業単位 A-13b-2）。
    ///
    /// **両向き**（なし→あり・あり→なし）を、**抽出の面だけ**変えて見る。**対照**: 同じ承認なら触らない。
    #[test]
    fn a_face_whose_approval_alone_changes_is_touched() {
        let approved = |allow: bool| Target {
            api_key: Some(ApiKey::parse("key-a-1111").expect("正当")),
            allow_key_over_plain_http: allow,
            ..target("http://192.0.2.1:1/v1/chat/completions", "sub")
        };
        let main = target("http://127.0.0.1:2/v1/chat/completions", "main");
        for (before, after, touched) in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
            (false, false, false),
        ] {
            let plan = plan_switch(
                &[],
                &[],
                &[main.clone(), approved(before)],
                &[main.clone(), approved(after)],
                [false, false],
            );
            assert_eq!(
                plan.touch,
                [
                    Touch::Untouched,
                    if touched {
                        Touch::External
                    } else {
                        Touch::Untouched
                    }
                ],
                "{before} → {after}"
            );
        }
    }

    /// **宛先と綴りが同じで、鍵だけが変わった面は触る**（ADR-0056 §決定2 / 作業単位 A-13b-1）。
    ///
    /// **3通り**（なし→あり・あり→別の鍵・あり→なし）を、**抽出の面だけ**変えて見る
    /// ——本文の面は同じまま触らない。**対照**: 同じ鍵なら触らない。
    #[test]
    fn a_face_whose_key_alone_changes_is_touched() {
        let key = |raw: &str| Some(ApiKey::parse(raw).expect("正当"));
        let keyed = |raw: Option<&str>| Target {
            api_key: raw.and_then(key),
            ..target("http://127.0.0.1:1/v1/chat/completions", "sub")
        };
        let main = target("http://127.0.0.1:2/v1/chat/completions", "main");
        for (before, after, touched) in [
            (None, Some("key-a-1111"), true),
            (Some("key-a-1111"), Some("key-b-2222"), true),
            (Some("key-a-1111"), None, true),
            (Some("key-a-1111"), Some("key-a-1111"), false),
        ] {
            let plan = plan_switch(
                &[],
                &[],
                &[main.clone(), keyed(before)],
                &[main.clone(), keyed(after)],
                [false, false],
            );
            assert_eq!(
                plan.touch,
                [
                    Touch::Untouched,
                    if touched {
                        Touch::External
                    } else {
                        Touch::Untouched
                    }
                ],
                "{before:?} → {after:?}"
            );
        }
    }

    /// **宛先と綴りが同じで、明示のコンテキスト長だけが変わった面は触る**（ADR-0054 §決定3 / 作業単位 A-13a-1）。
    ///
    /// **3通り**（照会→明示・明示→明示・明示→照会）を、**抽出の面だけ**変えて見る
    /// ——本文の面は同じまま触らない（変わった面だけを触る）。**子を担う面は子を残して綴りだけ**
    /// （値だけを変えても `llama` を起こし直さない）。
    #[test]
    fn a_face_whose_written_context_changes_is_touched_without_restarting_its_child() {
        let with = |context: Option<u32>| {
            let mut pair = targets(A);
            pair[slot(ModelFace::Sub)].context_tokens = context;
            pair
        };
        for (before, after) in [
            (None, Some(32_768)),
            (Some(32_768), Some(20_480)),
            (Some(20_480), None),
        ] {
            // 外の上流（子なし）。
            let plan = plan_switch(&[], &[], &with(before), &with(after), NONE_WAITING);
            assert_eq!(
                plan.touch,
                [Touch::Untouched, Touch::External],
                "{before:?} → {after:?}"
            );

            // 子を担う面（子は残して、クライアントだけ差し替える）。
            let running = launches_of(TWO_CHILDREN, "a");
            let plan = plan_switch(
                &running,
                &running,
                &with(before),
                &with(after),
                NONE_WAITING,
            );
            assert_eq!(plan.keep, running, "値だけで子を起こし直している");
            assert!(plan.start.is_empty());
            assert_eq!(
                plan.touch,
                [Touch::Untouched, Touch::KeptChild],
                "{before:?} → {after:?}"
            );
        }

        // **対照**: 値も同じなら触らない。
        let plan = plan_switch(
            &[],
            &[],
            &with(Some(32_768)),
            &with(Some(32_768)),
            NONE_WAITING,
        );
        assert_eq!(plan.touch, [Touch::Untouched, Touch::Untouched]);
    }

    /// **植えた綴りから、その名前のプロファイルの子の記述を取り出す。**
    fn launches_of(source: &str, name: &str) -> Vec<ProfileLaunch> {
        parse_profiles(source, None)
            .expect("読めること")
            .select(name)
            .expect("在ること")
            .launches()
            .to_vec()
    }

    /// **2つの面が別の子を担うプロファイル**と、**本文の gguf だけを変えたもの**。
    ///
    /// **値は全部違える**（宛先・綴り・gguf）——同値だと、面の取り違えが観測できない。
    const TWO_CHILDREN: &str = r#"{
        "a": { "main": { "endpoint": "http://127.0.0.1:45201", "model": "ma",
                         "gguf": "/planted/main-a.gguf" },
               "sub":  { "endpoint": "http://127.0.0.1:45202", "model": "sa",
                         "gguf": "/planted/sub-a.gguf" } },
        "b": { "main": { "endpoint": "http://127.0.0.1:45201", "model": "mb",
                         "gguf": "/planted/main-b.gguf" },
               "sub":  { "endpoint": "http://127.0.0.1:45202", "model": "sa",
                         "gguf": "/planted/sub-a.gguf" } },
        "external": { "main": { "endpoint": "http://127.0.0.1:45203", "model": "me" },
                      "sub":  { "endpoint": "http://127.0.0.1:45202", "model": "sa",
                                "gguf": "/planted/sub-a.gguf" } },
        "args": { "main": { "endpoint": "http://127.0.0.1:45201", "model": "ma",
                            "gguf": "/planted/main-a.gguf", "args": ["--ctx-size", "8192"] },
                  "sub":  { "endpoint": "http://127.0.0.1:45202", "model": "sa",
                            "gguf": "/planted/sub-a.gguf" } }
    }"#;

    const A: [(&str, &str); 2] = [
        ("http://127.0.0.1:45201/v1/chat/completions", "ma"),
        ("http://127.0.0.1:45202/v1/chat/completions", "sa"),
    ];

    fn targets(pairs: [(&str, &str); 2]) -> [Target; 2] {
        pairs.map(|(endpoint, model)| target(endpoint, model))
    }

    /// **起こすのを待っている面は無い**（F-10a より前の形）。
    const NONE_WAITING: [bool; 2] = [false, false];

    /// **起こすのを待っている面へ同じプロファイルを頼むと、その面を起こす**（ADR-0055 §決定3 / 作業単位 F-10a）。
    ///
    /// **走っている子は無い**（覚えた名前で起きた起動は何も起こしていない）ので、
    /// **宛先も綴りも同じなのに起こす**——比べるだけの実装でも、子の記述が `running` に無いので起こす。
    /// **ここで見るのは「待っている印があっても、起こす扱いが崩れない」こと。**
    #[test]
    fn the_same_profile_starts_the_faces_that_are_waiting() {
        let wanted = launches_of(TWO_CHILDREN, "a");
        for waiting in [[true, true], NONE_WAITING] {
            let plan = plan_switch(&[], &wanted, &targets(A), &targets(A), waiting);
            assert!(plan.keep.is_empty());
            assert_eq!(
                plan.start, wanted,
                "待っていた面を起こしていない（{waiting:?}）"
            );
            assert_eq!(plan.touch, [Touch::Launched, Touch::Launched]);
        }
    }

    /// **起こすのを待っている面へ、それを起こさないプロファイルを頼むと、外の上流にする**（作業単位 F-10a）。
    ///
    /// **宛先と綴りが同じ**でも触る——触らないと、その面は `not_started` を言い続ける
    /// （押しても起きないボタンになる）。**待っていない面は今までどおり触らない**（対照・同じ呼び出し）。
    #[test]
    fn a_waiting_face_that_the_next_profile_does_not_launch_becomes_external() {
        let plan = plan_switch(&[], &[], &targets(A), &targets(A), [false, true]);
        assert_eq!(plan.touch, [Touch::Untouched, Touch::External]);
        assert!(plan.keep.is_empty() && plan.start.is_empty());

        let plan = plan_switch(&[], &[], &targets(A), &targets(A), [true, false]);
        assert_eq!(plan.touch, [Touch::External, Touch::Untouched]);
    }

    /// **同じプロファイルへ差し替えたら、何も触らない。**
    #[test]
    fn switching_to_the_same_profile_touches_nothing() {
        let running = launches_of(TWO_CHILDREN, "a");
        let plan = plan_switch(&running, &running, &targets(A), &targets(A), NONE_WAITING);

        assert_eq!(plan.keep, running, "走っている子は全部残すこと");
        assert!(plan.start.is_empty(), "何も起こさないこと: {plan:?}");
        assert_eq!(plan.touch, [Touch::Untouched, Touch::Untouched]);
    }

    /// **本文の gguf だけを変えたら、本文の子だけを起こし直す。抽出の子は残す。**
    ///
    /// **片面だけの差し替えで、もう片面を触らない**（ADR-0042 §見張り番の行）。
    #[test]
    fn changing_one_faces_gguf_restarts_only_that_face() {
        let running = launches_of(TWO_CHILDREN, "a");
        let wanted = launches_of(TWO_CHILDREN, "b");
        let next = targets([
            ("http://127.0.0.1:45201/v1/chat/completions", "mb"),
            ("http://127.0.0.1:45202/v1/chat/completions", "sa"),
        ]);

        let plan = plan_switch(&running, &wanted, &targets(A), &next, NONE_WAITING);

        let sub_child = carrier(&running, ModelFace::Sub).expect("抽出の子が在ること");
        assert_eq!(plan.keep, vec![sub_child.clone()], "抽出の子だけを残すこと");
        assert_eq!(
            plan.start,
            vec![
                carrier(&wanted, ModelFace::Main)
                    .expect("本文の子が在ること")
                    .clone()
            ],
            "本文の子だけを起こすこと"
        );
        assert_eq!(plan.touch, [Touch::Launched, Touch::Untouched]);
    }

    /// **引数だけを変えても、その子は起こし直す**（前の引数の子に答えさせ続けない）。
    #[test]
    fn changing_only_the_args_restarts_the_child() {
        let running = launches_of(TWO_CHILDREN, "a");
        let wanted = launches_of(TWO_CHILDREN, "args");

        let plan = plan_switch(&running, &wanted, &targets(A), &targets(A), NONE_WAITING);

        assert_eq!(plan.touch, [Touch::Launched, Touch::Untouched]);
        assert_eq!(plan.start.len(), 1);
    }

    /// **起こしていた面を外の上流へ差し替えたら、その子を止めて外の上流にする。**
    #[test]
    fn a_launched_face_that_becomes_external_is_stopped() {
        let running = launches_of(TWO_CHILDREN, "a");
        let wanted = launches_of(TWO_CHILDREN, "external");
        let next = targets([
            ("http://127.0.0.1:45203/v1/chat/completions", "me"),
            ("http://127.0.0.1:45202/v1/chat/completions", "sa"),
        ]);

        let plan = plan_switch(&running, &wanted, &targets(A), &next, NONE_WAITING);

        assert!(
            !plan
                .keep
                .contains(carrier(&running, ModelFace::Main).expect("在ること")),
            "本文の子を残している"
        );
        assert!(plan.start.is_empty(), "外の上流を起こしている: {plan:?}");
        assert_eq!(plan.touch, [Touch::External, Touch::Untouched]);
    }

    /// **外の上流から、起こす面へ**（`--upstream` 無しで起動した後に gguf を選ぶ形）。
    #[test]
    fn an_external_face_that_names_a_gguf_is_started() {
        let wanted = launches_of(TWO_CHILDREN, "a");
        let current = targets([
            ("http://127.0.0.1:1234/v1/chat/completions", "lm-main"),
            ("http://127.0.0.1:1234/v1/chat/completions", "lm-sub"),
        ]);

        let plan = plan_switch(&[], &wanted, &current, &targets(A), NONE_WAITING);

        assert!(plan.keep.is_empty());
        assert_eq!(plan.start, wanted, "gguf を書いた面を全部起こすこと");
        assert_eq!(plan.touch, [Touch::Launched, Touch::Launched]);
    }

    /// **外の上流どうしでは、宛先か綴りが違う面だけを触る。**
    ///
    /// **3件のうち真ん中だけが変わる形**にはできない（面は2つ）ので、
    /// **抽出の面だけを変えて、本文の面を端に置かない順で見る**。
    #[test]
    fn between_external_upstreams_only_the_changed_face_is_touched() {
        let current = targets([
            ("http://127.0.0.1:1234/v1/chat/completions", "lm-main"),
            ("http://127.0.0.1:1234/v1/chat/completions", "lm-sub"),
        ]);
        let moved_sub = targets([
            ("http://127.0.0.1:1234/v1/chat/completions", "lm-main"),
            ("http://127.0.0.1:5678/v1/chat/completions", "lm-sub"),
        ]);
        let renamed_sub = targets([
            ("http://127.0.0.1:1234/v1/chat/completions", "lm-main"),
            ("http://127.0.0.1:1234/v1/chat/completions", "lm-sub-2"),
        ]);

        for next in [moved_sub, renamed_sub] {
            let plan = plan_switch(&[], &[], &current, &next, NONE_WAITING);
            assert_eq!(
                plan.touch,
                [Touch::Untouched, Touch::External],
                "宛先か綴りの片方だけが変わっても触ること: {next:?}"
            );
            assert!(plan.keep.is_empty() && plan.start.is_empty());
        }
    }

    /// **残す子が担う面で、依頼する綴りだけが変わる**——子は残し、クライアントだけ差し替える。
    #[test]
    fn a_kept_child_with_a_new_model_spelling_is_only_retargeted() {
        let running = launches_of(TWO_CHILDREN, "a");
        let next = targets([
            ("http://127.0.0.1:45201/v1/chat/completions", "ma"),
            ("http://127.0.0.1:45202/v1/chat/completions", "sa-renamed"),
        ]);

        let plan = plan_switch(&running, &running, &targets(A), &next, NONE_WAITING);

        assert_eq!(plan.keep, running, "子を起こし直している");
        assert!(plan.start.is_empty());
        assert_eq!(plan.touch, [Touch::Untouched, Touch::KeptChild]);
    }

    /// **1つの子が2面を担っていた形から、面ごとの子へ分けたら、両面とも起こし直す。**
    ///
    /// **子の記述は担う面まで含む**ので、`sub` を書き足したプロファイルは別の子になる。
    #[test]
    fn splitting_a_shared_child_restarts_both_faces() {
        let source = r#"{
            "shared": { "main": { "endpoint": "http://127.0.0.1:45211", "model": "m",
                                  "gguf": "/planted/one.gguf" } },
            "split":  { "main": { "endpoint": "http://127.0.0.1:45211", "model": "m",
                                  "gguf": "/planted/one.gguf" },
                        "sub":  { "endpoint": "http://127.0.0.1:45212", "model": "s",
                                  "gguf": "/planted/two.gguf" } }
        }"#;
        let running = launches_of(source, "shared");
        let wanted = launches_of(source, "split");
        let current = targets([
            ("http://127.0.0.1:45211/v1/chat/completions", "m"),
            ("http://127.0.0.1:45211/v1/chat/completions", "m"),
        ]);
        let next = targets([
            ("http://127.0.0.1:45211/v1/chat/completions", "m"),
            ("http://127.0.0.1:45212/v1/chat/completions", "s"),
        ]);

        let plan = plan_switch(&running, &wanted, &current, &next, NONE_WAITING);

        assert!(plan.keep.is_empty(), "2面を担う子を残している: {plan:?}");
        assert_eq!(plan.start, wanted);
        assert_eq!(plan.touch, [Touch::Launched, Touch::Launched]);
    }

    // ------------------------------------------------------------------
    // 依頼の接続から切り離す（作業単位 B-56 / Issue origin#39）
    // ------------------------------------------------------------------

    use tokio::sync::oneshot;

    /// 合図を待つ上限。**来なければ落ちる**（待つ側に上限を置く / CLAUDE.md `G-4`）。
    const SIGNAL_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

    /// **走らせたものが、合図を待ってから `outcome` を返す**（前提の「走り始めた」も知らせる）。
    struct Gated {
        started: oneshot::Receiver<()>,
        go: oneshot::Sender<()>,
        done: oneshot::Receiver<()>,
    }

    fn gated(
        outcome: Result<(), ApiError>,
    ) -> (
        Gated,
        impl std::future::Future<Output = Result<(), ApiError>> + Send + 'static,
    ) {
        let (started_tx, started) = oneshot::channel();
        let (go, go_rx) = oneshot::channel::<()>();
        let (done_tx, done) = oneshot::channel();
        let work = async move {
            let _ = started_tx.send(());
            let _ = go_rx.await;
            let _ = done_tx.send(());
            outcome
        };
        (Gated { started, go, done }, work)
    }

    /// **依頼した側が居れば、走らせたものの結果がそのまま返る**（対照）。
    ///
    /// **成功と、起きた場所で決まった断り（404）の両方**——どちらかに決め打つ実装を赤にする。
    #[tokio::test]
    async fn a_detached_work_hands_its_outcome_back_to_the_caller() {
        for outcome in [Ok(()), Err(ApiError::upstream_profile_not_found())] {
            assert_eq!(run_detached(async move { outcome }).await, outcome);
        }
    }

    /// **依頼した側（ハンドラ）が落ちても、走らせたものは最後まで走る**（作業単位 B-56 の本体）。
    ///
    /// **落とすのは「走り始めた後」**——走り始める前に落とすと、
    /// 走らせ方に関係なく「最後まで走った」と「一度も走らなかった」の区別が要らなくなる。
    #[tokio::test]
    async fn a_detached_work_runs_to_the_end_after_the_caller_is_dropped() {
        let (gate, work) = gated(Ok(()));
        let caller = tokio::spawn(run_detached(work));

        tokio::time::timeout(SIGNAL_LIMIT, gate.started)
            .await
            .expect("前提: 走らせたものが時間内に走り始めること")
            .expect("前提: 走り始めた合図が来ること");
        caller.abort();
        assert!(
            caller
                .await
                .expect_err("前提: 依頼した側が落ちること")
                .is_cancelled(),
            "前提: 依頼した側は abort で落ちていること"
        );

        gate.go
            .send(())
            .expect("依頼した側と一緒に、走らせたものまで落ちている");
        tokio::time::timeout(SIGNAL_LIMIT, gate.done)
            .await
            .expect("走らせたものが時間内に終わること")
            .expect("走らせたものが最後まで走ること");
    }

    /// **走らせたものが落ちたら（panic）、依頼した側には 500 を返し、`error` で言う。**
    #[test]
    fn a_detached_work_that_panics_is_answered_with_an_internal_error() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("ランタイムを作れること");
        let mut answered = None;
        let output = crate::testlog::captured_under_default_filter(|| {
            answered = Some(runtime.block_on(run_detached(async {
                panic!("植えた panic（作業単位 B-56 のテスト）")
            })));
        });

        assert_eq!(answered, Some(Err(ApiError::internal())));
        let line = crate::testlog::line_containing(&output, DETACHED_PANIC_LINE);
        assert!(line.contains("ERROR"), "error で言っていない: {line}");
    }

    /// **終了処理が錠を取った後に届いた差し替えは、何にも触らずに断る**（作業単位 B-56 のレビュー提案1）。
    ///
    /// # なぜ単体で見るのか
    ///
    /// **隙（`tokio::spawn` から `try_lock` まで）は、外から決定的に作れない** ——
    /// 終了処理は待ち受けを閉じてから始まるので、その後の依頼は HTTP では届かず、
    /// 既に切り離されたタスクが錠に届く順は実行時の都合で決まる。
    /// **ここでは [`Switcher::shutdown`] を先に済ませてから [`Switcher::switch`] を呼んで、その順を作る。**
    ///
    /// # 何で見分けるか（**応答の値では見分けられない**）
    ///
    /// **作れるのは照会先を持たない束（[`Supervisor::idle`]）だけ**で、その束は
    /// 印が無くても `upstream_switch_unavailable` で断る（照会先が無い枝）。
    /// **だから「どの枝で断ったか」をログの行で見る。** 対照（閉じる前）は照会先が無い枝の行を出し、
    /// 印の行を出さない——**印を見ない実装では、閉じた後もその行のままになる。**
    #[test]
    fn a_switch_that_reaches_the_lock_after_the_shutdown_is_refused() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("ランタイムを作れること");
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let switcher = Switcher::new(
            Supervisor::idle(
                dir.path().to_path_buf(),
                super::super::launch::SharedLaunchStates::external(),
            ),
            Some(dir.path().join("upstreams.json")),
            None,
            0,
            // **起動時に選んだ名前**（作業単位 F-10b）。断った差し替えでは変わらないことを末尾で見る。
            Some("kept".to_owned()),
        );

        // **対照: 閉じる前は、照会先が無い枝で断る**（印の枝ではない）。
        let before = crate::testlog::captured_under_default_filter(|| {
            let answered = runtime.block_on(switcher.switch("planted"));
            assert_eq!(answered, Err(ApiError::upstream_switch_unavailable()));
        });
        crate::testlog::line_containing(&before, "この起動は上流を差し替えられません");
        assert!(
            !before.contains(SWITCH_CLOSED_LINE),
            "閉じる前なのに、閉じた印で断っている: {before}"
        );

        // **本題: 終了処理が錠を取って返った後に、錠に届いた差し替え。**
        let after = crate::testlog::captured_under_default_filter(|| {
            let answered = runtime.block_on(async {
                switcher.shutdown().await;
                switcher.switch("planted").await
            });
            assert_eq!(answered, Err(ApiError::upstream_switch_unavailable()));
        });
        crate::testlog::line_containing(&after, SHUTDOWN_LINE);
        crate::testlog::line_containing(&after, SWITCH_CLOSED_LINE);
        assert!(
            !after.contains("この起動は上流を差し替えられません"),
            "閉じた印より先に、別の枝で断っている（印を見ていない）: {after}"
        );
        // **断った差し替えは、いま選ばれている名前を変えない**（作業単位 F-10b）。
        // **頼んだ名前（`planted`）と起動時の名前（`kept`）を違えてある**——書き換える実装はここで赤になる。
        assert_eq!(switcher.selected_profile(), Some("kept".to_owned()));
    }

    /// **依頼した側がもう居なくても、落ちたことは `error` で言う**（握りつぶさない）。
    ///
    /// **ハンドラの中で `JoinError` を読む実装は、ここで黙る**——読む側が居ない。
    #[test]
    fn a_detached_work_that_panics_after_the_caller_is_gone_is_still_reported() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("ランタイムを作れること");
        let output = crate::testlog::captured_under_default_filter(|| {
            runtime.block_on(async {
                let (started_tx, started) = oneshot::channel();
                let (go, go_rx) = oneshot::channel::<()>();
                let caller = tokio::spawn(run_detached(async move {
                    let _ = started_tx.send(());
                    let _ = go_rx.await;
                    panic!("植えた panic（作業単位 B-56 のテスト）")
                }));
                tokio::time::timeout(SIGNAL_LIMIT, started)
                    .await
                    .expect("前提: 時間内に走り始めること")
                    .expect("前提: 走り始めた合図が来ること");
                caller.abort();
                assert!(
                    caller.await.expect_err("前提: 落ちること").is_cancelled(),
                    "前提: 依頼した側は abort で落ちていること"
                );
                go.send(()).expect("前提: 走らせたものがまだ待っていること");
                // **current_thread なので、譲れば走らせたものと見届け役が順に走る**
                // （どちらも I/O を待たない）。**回数は上限**で、足りなければ赤に倒れる。
                for _ in 0..1000 {
                    tokio::task::yield_now().await;
                }
            });
        });

        let line = crate::testlog::line_containing(&output, DETACHED_PANIC_LINE);
        assert!(line.contains("ERROR"), "error で言っていない: {line}");
    }
}
