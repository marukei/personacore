//! **名指しされた `llama serve` を、実行ファイルが起こして止める**
//! （[ADR-0042](../../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)
//! §決定2 / §決定3 / §決定7 / 作業単位 F-4b-1 / F-4b-2）。
//!
//! ```text
//! App::bind_with
//!   1. 待ち受けを開く（TcpListener::bind）      ← **先に開く**
//!   2. gguf を書いた面のために子を起こす         ← Supervisor::start
//!   3. DB を開く
//! App::serve
//!   4. 背景で「載った」を待つ番人を付ける         ← Supervisor::arm
//!   ・走ったまま差し替える（F-4b-2）             ← Supervisor::stop_except → start_more
//!                                                  （`super::switch` が呼ぶ）
//!   5. 応答が終わったら、控えた子だけを止める     ← Supervisor::stop
//! ```
//!
//! # なぜ「待ち受け → 起こす」の順なのか（ADR-0042 §決定3）
//!
//! **逆にすると、`bind` に失敗した起動が上流を残す**——起こした後で待ち受けに失敗すると、
//! **止める人がもう居ない。** `F-9` の PowerShell は「載るまで待ってから `cargo run`」で、
//! **画面が使えない時間が1〜2分できていた。** こちらは順序を逆にして、
//! **載るのは背景で待ち、状態で言う。**
//!
//! # 起こしたものしか止めない（ADR-0042 §決定2）
//!
//! **止める相手は [`StartedUpstream`] に控えた子だけ。**
//! **名前で探して止める経路をこのモジュールに作らない**——
//! LM Studio が内側で立てる `llama-server` にも、利用者が手で立てた `llama` にも触らない。
//! **`F-9` は実機でこれを踏んだ**（控えが PID を失い、GPU を 11.7 GB 掴んだまま残った）。
//!
//! **控えるのは「起こした直後」**（載るのを待つ前）。ここでは**型がそれを強制している**——
//! [`Supervisor::start`] は子を [`Supervisor`] の中へ入れてから返り、
//! **待つのは番人（[`Supervisor::arm`] が付ける）で、`serve` にならないと付かない。**
//! **`start` が返る前に落ちた経路でも、[`tokio::process::Command::kill_on_drop`] が殺す。**
//!
//! > **⚠ 「型で表現できなくした」とは書けない**（CLAUDE.md / `0012b`）。
//! > **`Supervisor` の中で待つ実装は今も書ける。**
//! > 止めているのは順序と、`tests/upstream_launch.rs` の
//! > `the_named_gguf_is_started_reaches_ready_and_is_stopped_with_the_server`
//! > （**偽の `llama` を本当に起こして、本当に止める**）。
//!
//! # ⚠ [`Supervisor::stop`] と `kill_on_drop` は、互いを隠し合っている（**実測 / 2026-09-19**）
//!
//! **変異検査で2つとも単独では全緑だった**（2157 passed / exit 0）——
//! **「控えた子を1つも止めない」**（`m1`）と**「`kill_on_drop(true)` を外す」**（`m1b`）。
//! **組にすると 3 赤**（`m1x`）。**CLAUDE.md の第5の分類そのものである。**
//!
//! | 変異 | なぜ緑か |
//! |---|---|
//! | 止めない | **`serve` を抜けるときに `App` が落ち、`kill_on_drop` が殺す** |
//! | `kill_on_drop` を外す | **正常な停止では [`Supervisor::stop`] が殺す。`?` で抜ける経路にはまだ子が居ない**（gguf の確認も待ち受けの失敗も、起こす前） |
//!
//! **⚠ だから「どちらか片方でよい」とは読まないこと。**
//! **`kill_on_drop` は終了を待たない** —— [`Supervisor::stop`] の `wait` が無いと、
//! **VRAM の解放を待たずに次へ進む**（ADR-0042 §決定2 が明示的に要求している）。
//! **`stop` が単独で赤にされているのは「名前で殺さないこと」だけ**
//! （`tests/upstream_launch.rs::stopping_the_server_leaves_someone_elses_llama_alone`）。
//!
//! > **⚠ 上の表は終了時の [`Supervisor::stop`] の話である。** 走ったままの差し替え
//! > （[`Supervisor::stop_except`] / 作業単位 F-4b-2）には `kill_on_drop` が効く場面が無い
//! > （`App` は落ちない）ので、**止めない変異は単独で赤になる**。
//! > **終了を待つこと・止めてから起こす順は、それぞれ別の見張り番が見る**
//! > （**どちらも競走しない形**。同じポートを取り直す腕では、待たない実装も
//! > 起こしてから止める実装も、たいてい間に合ってしまうため）:
//! >
//! > | 見張り番 | 見ているもの |
//! > |---|---|
//! > | `tests::stopping_a_child_waits_until_it_has_exited` | **終わるのが遅い子**（作り物 / [`UpstreamProcess`]）で、終わるまで返らないこと |
//! > | `tests/upstream_launch.rs::a_switch_stops_the_old_child_before_it_starts_the_next` | この実行ファイルが出す3行（止めます → 終了しました → 起こします）の順 |
//! > | `tests/upstream_launch.rs::switching_restarts_only_the_face_whose_gguf_changed` | **前の子を止めない**形（同じポートを掴まれたまま、次の子が待ち受けられない） |
//! >
//! > **数は作業単位 F-4b-2 の報告にある**（ここへ写さない）。
//!
//! # 待ち方は ADR-0035（**固定の秒数で殺さない**）
//!
//! **経過は `info` で定期的に報告し、[`LOAD_CEILING`] を超えたら `warn` で
//! 「まだ載っていない」と言うだけで止めない。**
//! **20 GB の gguf は載るまで1〜2分で、画像生成と取り合えばもっと**（ADR-0019 の反例）。
//! **⚠ ただし、子が自分で終了したら即「失敗」**——**死んだ書き手を待たない**
//! （CLAUDE.md `G-4`「上限は待つ側に」）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use tokio::process::{Child, Command};

use super::config::BindAddr;
use super::context::{FaceClients, SharedBudget, SharedSubContext, SharedUpstream};
use super::upstreams::{ProfileLaunch, UpstreamProfile};
use crate::llm::endpoint_for_log;
use crate::measured::ModelFace;

/// 上流の出力を落とすディレクトリの名前。**DB と同じディレクトリの下**（ADR-0042 §決定2）。
///
/// **personacore 自身のログに混ぜない**（`F-9` と同じ）——
/// `llama` の出力は行単位で長く、混ぜると起動の1行が読めなくなる。
pub const UPSTREAM_LOG_DIR_NAME: &str = "upstream-logs";

/// **載るのを待つ緩い天井**（ADR-0035 / ADR-0042 §決定3）。
///
/// **超えても止めない。** `warn` を1本出して待ち続ける。
/// **設定にしない**——「根拠が無い・外すと破壊的・導出も観測もできない」が揃っていない
/// （CLAUDE.md §1 の最後）。**外しても遅くなるだけで、壊れない。**
const LOAD_CEILING: Duration = Duration::from_secs(15 * 60);

/// 載ったかを見に行く間隔。
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// 経過を `info` で報告する間隔。**黙って待たない**（ADR-0035）。
const PROGRESS_INTERVAL: Duration = Duration::from_secs(30);

/// 退避先の時刻の綴り（`20260918-101112`）。**`:` を使わない**（Windows のファイル名）。
const LOG_TIME_FORMAT: &str = "%Y%m%d-%H%M%S";

// ----------------------------------------------------------------------
// 面ごとの状態（ADR-0042 §決定3）
// ----------------------------------------------------------------------

/// **その面の上流を、この実行ファイルがどう扱っているか。**
///
/// **`GET /admin/upstream` がこれを言う**（`super::upstream::LaunchView`）。
/// **観測（`UpstreamFacts`）とは別物**——あちらは「上流が何と答えたか」で、
/// こちらは「この実行ファイルが何をしたか」。**畳むと、
/// 『まだ載せている最中』と『上流が黙っている』が区別できなくなる。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaceState {
    /// **起こしていない。** 外の上流（LM Studio・利用者が手で立てたもの）。
    External,
    /// **起こしていない（起こすのを待っている）。** gguf を名指しした面だが、まだ起こしていない
    /// （[ADR-0055](../../docs/adr/0055-do-not-start-the-remembered-upstream-at-launch.md) §決定1 / 作業単位 F-10a）。
    ///
    /// **覚えた名前で起きた起動**（`LaunchTiming::WhenAsked`）がこの状態で始める。
    /// **同じ名前の `POST /admin/upstream/profile` で起こす**（`super::switch`）。
    /// **[`Self::External`] と畳まない**——こちらは「頼まれたら personacore が起こす」面で、
    /// 画面は起こすボタンを出す（ADR-0055 §決定3）。
    NotStarted,
    /// **載せている最中。** 子は生きていて、まだ gguf を名乗っていない。
    Loading {
        /// 起こした時刻。**経過を読むのはこの欄だけ。**
        since: chrono::DateTime<chrono::Local>,
    },
    /// **準備完了。** 上流が gguf の名前を名乗った。
    Ready {
        /// 上流が名乗った名前（**ファイル名だけ**）。
        answered_name: String,
    },
    /// **失敗。** 子が自分で終了した／別のモデルが載った。
    Failed {
        /// 何が起きたか。**置き場所は載せない**（下の `log` はファイル名だけ）。
        reason: String,
        /// 出力の落とし先の**ファイル名だけ**（`upstream-logs/` の下）。
        log: Option<String>,
    },
    /// **止めた。** この実行ファイルが終わるときに止めた。
    Stopped,
}

impl FaceState {
    /// **全部の綴り**（作業単位 P-2a —— 仕様書が `LaunchView::state` の `enum` をここから作る）。
    ///
    /// **綴りを写さない**——代表の値を1つずつ作って [`Self::label`] に通す
    /// （variant が値を持つので `ALL` の配列は作れない）。
    /// 数え漏れは `every_state_is_listed_in_all_labels` が見る。
    #[must_use]
    pub fn all_labels() -> [&'static str; 6] {
        [
            Self::External,
            Self::NotStarted,
            Self::Loading {
                since: chrono::DateTime::<chrono::Local>::default(),
            },
            Self::Ready {
                answered_name: String::new(),
            },
            Self::Failed {
                reason: String::new(),
                log: None,
            },
            Self::Stopped,
        ]
        .map(|state| state.label())
    }

    /// 綴り。**画面と応答が読む正**（`super::upstream::LaunchView::state`）。
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::External => "external",
            Self::NotStarted => "not_started",
            Self::Loading { .. } => "loading",
            Self::Ready { .. } => "ready",
            Self::Failed { .. } => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// **両面ぶんの状態。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchStates {
    main: FaceState,
    sub: FaceState,
}

impl LaunchStates {
    /// **どちらも外の上流**（`--upstream` を渡していない起動）。
    const fn external() -> Self {
        Self {
            main: FaceState::External,
            sub: FaceState::External,
        }
    }

    /// 面で引く。**帰属を1箇所で決める**（`UpstreamFacts::face` と同じ形）。
    #[must_use]
    pub const fn face(&self, face: ModelFace) -> &FaceState {
        match face {
            ModelFace::Main => &self.main,
            ModelFace::Sub => &self.sub,
        }
    }

    fn set(&mut self, face: ModelFace, state: FaceState) {
        match face {
            ModelFace::Main => self.main = state,
            ModelFace::Sub => self.sub = state,
        }
    }

    /// **面ごとに別の状態を入れた組。テスト専用。**
    ///
    /// **2つを違えられることが要点**——同値しか作れないと、
    /// **応答が面を取り違える変異が観測できない**（CLAUDE.md の帰属の族）。
    #[cfg(test)]
    pub(in crate::server) const fn of_for_tests(main: FaceState, sub: FaceState) -> Self {
        Self { main, sub }
    }
}

/// **稼働中に書き替えられる、面ごとの状態。**
///
/// `super::context::SharedUpstream` と**同じ形**（読みは短く、書きは面あたり数回）。
/// **ハンドラは毎回ここを読む**（起動時の写しを持たない）——
/// 写すと**「載せている」と言ったまま永久に固まる。**
#[derive(Debug, Clone)]
pub struct SharedLaunchStates(Arc<RwLock<LaunchStates>>);

impl SharedLaunchStates {
    /// **どちらも外の上流**の器を作る。
    pub(in crate::server) fn external() -> Self {
        Self(Arc::new(RwLock::new(LaunchStates::external())))
    }

    /// 現在の状態。
    ///
    /// 毒された錠でも中身は読む（`unwrap` / `expect` を置かないための
    /// `PoisonError::into_inner`）。**ここで panic すると管理 API が全滅する。**
    #[must_use]
    pub fn get(&self) -> LaunchStates {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// **その面だけを差し替える。** 反対の面は1バイトも触らない。
    ///
    /// **書き手は `Supervisor`（起こす・止める）と番人と、差し替え（`super::switch`）だけ。**
    pub(in crate::server) fn set(&self, face: ModelFace, state: FaceState) {
        self.0
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .set(face, state);
    }
}

// ----------------------------------------------------------------------
// 起こす計画
// ----------------------------------------------------------------------

/// **起こすもの一式。** 空なら1つも起こさない（＝いままでと同じ起動）。
#[derive(Debug, Clone, Default)]
pub struct LaunchPlan {
    launches: Vec<ProfileLaunch>,
}

impl LaunchPlan {
    /// **1つも起こさない。** `--upstream` を渡していない起動と、注入の入口が使う。
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// プロファイルから作る。**gguf を書いた面だけが入る。**
    #[must_use]
    pub fn of(profile: &UpstreamProfile) -> Self {
        Self {
            launches: profile.launches().to_vec(),
        }
    }

    /// **差し替えで新しく起こすものだけの計画**（作業単位 F-4b-2 / `super::switch`）。
    ///
    /// **検証（gguf の実在・待ち受けとの取り合い）を起動時と同じ関数に通す**ための形。
    pub(in crate::server) const fn from_launches(launches: Vec<ProfileLaunch>) -> Self {
        Self { launches }
    }

    /// 起こすものが1つも無いか。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.launches.is_empty()
    }

    /// **名指しされた gguf のうち、無いものを1つ挙げる。純粋ではない（ファイルを見る）。**
    ///
    /// **1つも起こす前に、全部を見る**（ADR-0042 §決定2）。起動時（[`Supervisor::start`]）と
    /// 差し替え（`super::switch`）が**同じ関数**を通る。
    pub(in crate::server) fn missing_gguf(&self) -> Option<MissingGguf> {
        self.launches.iter().find_map(|launch| {
            let gguf = launch.launch().gguf();
            // **`symlink_metadata`**——壊れたリンクを「在る」と読まない
            // （`fresh::names` が退避先の空きを見るのと同じ作法）。
            std::fs::symlink_metadata(gguf)
                .is_err()
                .then(|| MissingGguf {
                    face: face_label_of(launch),
                    path: gguf.display().to_string(),
                })
        })
    }

    /// **起こす前に分かる失敗を、起動時と同じ関数で確かめる**（ADR-0042 §決定9 / 作業単位 F-4f）。
    ///
    /// **`main` が「覚えた名前のプロファイル」を採るかを決めるために使う**——使えなければ、
    /// 止まらずに環境変数の既定で起きる（**明示の `--upstream` はここを通らず、`bind` が今までどおり止める**）。
    ///
    /// 見るのは [`Self::missing_gguf`] と [`Self::port_conflict`] だけ（**判定の写しを作らない**）。
    /// **起こせるか（`command` が `PATH` に在るか）は、起こすまで分からない**ので見ない。
    /// **覚えた名前の起動は起こさない**（ADR-0055 / 作業単位 F-10a）ので、そちらの失敗は
    /// **同じ名前で起こしたときに、その面の `failed` として出る**（F-10a より前は、`main` が
    /// `ServerError::Launch` を受けて環境変数の既定で起き直していた。その枝は消えた）。
    ///
    /// # 待ち受けとの取り合いは、番号を明示したときだけ見る（ADR-0057 §決定3 / 作業単位 F-11b）
    ///
    /// **`bind` が `None` なら番号の取り合いを見ない**（`main` は `AppConfig::explicit_bind` を渡す）。
    /// 明示していない起動は、待ち受けを探すときに `upstreams.json` の全部の gguf の面の番号を飛ばすので、
    /// 面と取り合わない——そこで覚えた名前を捨てると、使えるプロファイルを理由なく捨てることになる。
    ///
    /// # Errors
    ///
    /// gguf が無い、または（`bind` が `Some` のとき）personacore 自身の待ち受けとポートを取り合うとき。
    pub fn check_before_starting(&self, bind: Option<BindAddr>) -> Result<(), LaunchError> {
        if let Some(missing) = self.missing_gguf() {
            return Err(LaunchError::MissingGguf(missing));
        }
        if let Some(collision) = bind.and_then(|bind| self.port_conflict(bind)) {
            return Err(LaunchError::PortCollision(collision));
        }
        Ok(())
    }

    /// **その面をこの実行ファイルが起こすか。**
    ///
    /// **起こす面には、起動時に上流を聞かない**（ADR-0042 §決定3 / `App::bind`）——
    /// まだ載っていないのだから、聞けば必ず「答えなし」になる。
    #[must_use]
    pub fn launches_face(&self, face: ModelFace) -> bool {
        self.launches
            .iter()
            .any(|launch| launch.faces().contains(&face))
    }

    /// **自分の待ち受けを取りに行く面を、起こす前に見つける。純粋関数**
    /// （作業単位 B-52 / Issue origin#21。ADR-0042 §決定2「起こす前に分かることは起こす前に止める」）。
    ///
    /// # なぜ起こす前に止めるのか（利用者の実機 / 2026-09-19）
    ///
    /// **`-Port` を渡さない起動は 8080 で待ち受ける。**
    /// 見本のプロファイルも 8080 を名乗っていたので、
    /// **personacore が待ち受けた直後に `llama` が同じポートを取りに行き、
    /// 2 秒後に `exit code: 1` で終わった。**
    /// 画面に出せるのは**「失敗（exit code: 1）」だけ**で、
    /// **本当の理由は `llama` の err ログの中にしか無い。**
    /// **exe は両方の値を知っている**のだから、起こす前に両方を挙げて止まれる。
    ///
    /// # 見ているのはポートだけ（ホストの綴りは見ない）
    ///
    /// **gguf を書いた面はループバックしか許していない**（`upstreams` の検証規則）。
    /// **`127.9.9.9` のような別名なら実際には同居できる**が、
    /// **その同居を案内していないし、当てにされると `-Port` を変えた瞬間に壊れる。**
    /// **ここは「起こす前に分かる取り合い」を止める門で、可能性の網羅ではない。**
    ///
    /// # gguf の無い面は入っていない
    ///
    /// [`Self::of`] は**起こす面しか入れない**ので、**外の上流は原理的に縛られない**
    /// ——そこに待ち受けを作ったのは personacore ではない。
    #[must_use]
    pub(in crate::server) fn port_conflict(&self, bind: BindAddr) -> Option<PortCollision> {
        // **`0` は「OS に空きを選ばせる」**（`BindAddr::loopback` の doc）。`0` が来たら取り合いを判定しない。
        //
        // **起動時（`App::bind_with`）は、実際に開いた番号（`local_addr`）を渡す**（ADR-0057 §決定3 /
        // 作業単位 F-11b）——ずらした起動では設定の番号と違い、`PERSONACORE_PORT=0` では OS が選んだ番号になる。
        // **だから起動時に `0` は来ない。** `0` が来うるのは、事前検査（`check_before_starting`）に明示の `0` を
        // 渡したときだけで、そのときは実際の番号がまだ決まっていないので判定しない。
        // （F-11b より前は設定の番号を渡していて、`PERSONACORE_PORT=0` の起動ではこの門が一度も効かなかった
        // ——レビューが指摘 / 2026-09-19。引数の型は `BindAddr` のまま、渡す値を `local_addr` に替えて塞いだ。）
        let port = bind.port();
        if port == 0 {
            return None;
        }
        self.launches
            .iter()
            .find(|launch| launch.launch().serve_port() == port)
            .map(|launch| PortCollision {
                face: face_label_of(launch),
                // **`Display` に載る（起動を止めた理由としてログと標準エラーへ出る）ので、
                // ユーザー情報・クエリ・フラグメントを落とした形で持つ**（作業単位 B-71）。
                endpoint: endpoint_for_log(launch.endpoint()),
                bind: bind.to_string(),
            })
    }
}

// ----------------------------------------------------------------------
// 失敗の理由
// ----------------------------------------------------------------------

/// **gguf が無い**（[`LaunchError::MissingGguf`] の `#[source]`）。
///
/// **起こす前に確かめる**（ADR-0042 §決定2）。無い物を指したまま先へ進ませない。
/// **置き場所を載せるのは `upstreams::ProfileFileUnreadable` と同じ理由**
/// ——この型が HTTP へ出る経路は無く、直す手はその綴りにしか無い。
#[derive(Debug, thiserror::Error)]
#[error("{face} の gguf がありません: {path}")]
pub struct MissingGguf {
    face: &'static str,
    path: String,
}

/// **待ち受けとポートを取り合っている**（[`LaunchError::PortCollision`] の `#[source]` /
/// 作業単位 B-52）。
///
/// **両方の値を挙げる。** どちらを直せばよいか（プロファイルか `-Port` か）は、
/// **片方だけでは決まらない。**
/// **置き場所を載せる他の2つと同じく、この型が HTTP へ出る経路は無い。**
#[derive(Debug, thiserror::Error)]
#[error("{face} の宛先 {endpoint} は、personacore 自身の待ち受け {bind} と同じポートです")]
pub struct PortCollision {
    face: &'static str,
    endpoint: String,
    bind: String,
}

/// **起こせなかった**（[`LaunchError::Spawn`] の `#[source]`）。
#[derive(Debug, thiserror::Error)]
#[error("{command} を起こせませんでした: {source}")]
pub struct SpawnFailure {
    command: String,
    #[source]
    source: std::io::Error,
}

/// 上流を起こせなかった理由。
///
/// `Display` は**定数文字列**。詳細は `#[source]` 側に残る。
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// 名指しされた gguf が実在しない。**1つも起こさずに止まる。**
    #[error("上流のモデルファイルが見つかりません")]
    MissingGguf(#[source] MissingGguf),

    /// **起こす面が、personacore 自身の待ち受けと同じポートを取りに行っている。**
    /// **1つも起こさずに止まる**（作業単位 B-52 / Issue origin#21）。
    ///
    /// **[`Self::Spawn`] と分けてある** ——あちらは「起こせない」で、
    /// こちらは**起こせば動くが、その先で必ず取り合いになる**。直す手が違う。
    #[error("上流の宛先が personacore 自身の待ち受けと同じです")]
    PortCollision(#[source] PortCollision),

    /// 出力の落とし先を作れない。
    #[error("上流のログの置き場所を作れませんでした")]
    LogDir(#[source] std::io::Error),

    /// 実行ファイルを起こせない（PATH に無い・権限が無い）。
    #[error("上流を起こせませんでした")]
    Spawn(#[source] SpawnFailure),
}

// ----------------------------------------------------------------------
// 起こしたもの
// ----------------------------------------------------------------------

/// **止めて、終了を待つ相手**（作業単位 F-4b-2 / Issue origin#2 の持ち越し1）。
///
/// **本番の実装は [`tokio::process::Child`] だけ。** その3つのメソッドをそのまま通す。
///
/// # なぜトレイトにしたのか（**見張り番を競走なしに置くため**）
///
/// **「止めたら終了を待つ」は、子がすぐには終わらないときにしか現れない。**
/// **本物の子（偽の `llama` も）は `kill` のあと数ミリ秒で終わる**ので、
/// **待たない実装でも、次の子が起きる頃にはたいてい終わっている**——
/// 「待ったか」を外から見る腕は、**その数ミリ秒との競走**になる。
/// **終わるのが遅い子を作れるのは作り物だけ**なので、ここに継ぎ目を置いた
/// （見張り番は `tests::stopping_a_child_waits_until_it_has_exited`）。
///
/// **`try_wait` も載せてある**のは、載るのを待つ番人（[`FaceWatcher`]）が同じ継ぎ目を通るため
/// ——**待つ側と見る側が別の口を使う形にしない。**
pub(in crate::server) trait UpstreamProcess: Send {
    /// 止める合図を送る（**送るだけ。終わるのは待たない**）。
    fn start_kill(&mut self) -> std::io::Result<()>;
    /// **いま終わっているか**（待たない）。
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
    /// **終わるまで待つ。**
    fn wait(
        &mut self,
    ) -> impl std::future::Future<Output = std::io::Result<std::process::ExitStatus>> + Send;
}

impl UpstreamProcess for Child {
    fn start_kill(&mut self) -> std::io::Result<()> {
        Self::start_kill(self)
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        Self::try_wait(self)
    }

    fn wait(
        &mut self,
    ) -> impl std::future::Future<Output = std::io::Result<std::process::ExitStatus>> + Send {
        Self::wait(self)
    }
}

/// **控えた子を止め、終了を待つ**（ADR-0042 §決定2）。
///
/// **待たずに返らない。** 待たずに次を起こすと VRAM の取り合いになる（`C-16` の形）
/// ——**走ったままの差し替え（`super::switch`）は、この1点に寄りかかっている。**
///
/// # 見張り番（**F-4b-1 では1本も無かった**）
///
/// **`wait().await` を `try_wait()` に落とす変異は、F-4b-1 の回に 2157 全緑だった**
/// （「次を起こす」経路がまだ無かった）。**いまは
/// `tests::stopping_a_child_waits_until_it_has_exited` が、終わるのが遅い子で見ている。**
async fn stop_process<P: UpstreamProcess>(process: &mut P, pid: Option<u32>) {
    // **既に死んでいれば `Err`。** そこで鳴らすと、本当の理由を上書きして消す。
    let _ = process.start_kill();
    match process.wait().await {
        Ok(status) => tracing::info!(
            upstream_pid = ?pid,
            upstream_exit = %status,
            "起こした上流が終了しました"
        ),
        Err(err) => tracing::warn!(
            error = %err,
            upstream_pid = ?pid,
            "起こした上流の終了を待てませんでした"
        ),
    }
}

/// **起こした子1つぶんの控え。**
///
/// **`Child` を手放さない。** 手放すと止める手段が名前しか残らず、
/// **他人のプロセスを巻き込む経路ができる**（ADR-0042 §決定2）。
struct StartedUpstream {
    /// **控えた子そのもの。** 番人（`try_wait`）と止める側（`kill` ＋ `wait`）が共有する。
    child: Arc<tokio::sync::Mutex<Child>>,
    /// **控えた PID。** ログに出すためだけに持つ（止めるのは [`Self::child`] 経由）。
    pid: Option<u32>,
    /// **起こしたときの記述そのもの**（作業単位 F-4b-2）。
    ///
    /// **差し替えは「同じ記述の子は残す」で決まる**（`super::switch`）ので、
    /// **面・宛先・gguf・引数を、起こしたときの形のまま持つ。**
    spec: ProfileLaunch,
    /// **載ったと認める名前**（gguf のファイル名）。
    ///
    /// **依頼したモデルの綴りでは確かめない**——`llama serve` は `model` の欄を無視する
    /// （`B-45` / `C-18` と同じ根拠）。
    expected_name: String,
    /// 標準出力の落とし先の**ファイル名だけ**（応答に絶対パスを載せない）。
    out_log: String,
    /// 標準エラーの落とし先の**ファイル名だけ**。
    err_log: String,
    /// **この子が載るのを待つ番人**（[`Supervisor::arm`] の後にだけ在る）。
    ///
    /// **子ごとに持つ**（作業単位 F-4b-2）——差し替えで1つだけ止めるとき、
    /// **その子の番人だけを先に止める**（止めないと、止めた子を見て「失敗」と書きに来る）。
    watch: Option<tokio::task::JoinHandle<()>>,
}

impl StartedUpstream {
    fn faces(&self) -> &[ModelFace] {
        self.spec.faces()
    }

    fn endpoint(&self) -> &str {
        self.spec.endpoint()
    }
}

impl Drop for StartedUpstream {
    /// **番人を残さない。** `JoinHandle` は落としても止まらない（切り離されるだけ）。
    /// 子そのものは `kill_on_drop` が殺す。
    fn drop(&mut self) {
        if let Some(watch) = self.watch.as_ref() {
            watch.abort();
        }
    }
}

/// **載るのを待つ番人が要るもの**（照会先と、観測を書く先）。
///
/// **`serve` になってから揃う**（予算の器は引き直しを走らせたときに返る）ので、
/// [`Supervisor::arm`] で後から渡す。
#[derive(Clone)]
pub(in crate::server) struct Armed {
    /// 面ごとの照会先（**差し替えるとここも差し替わる** / `context::FaceClient`）。
    pub(in crate::server) clients: FaceClients,
    /// 載った直後の観測を書く先。
    pub(in crate::server) sinks: LoadSinks,
}

/// **起こした子を束ねて持つ。** 止められるのはここだけ。
///
/// # 作業単位 F-4b-2 で変わったこと
///
/// **起動時の一度きりではなくなった。** 走ったまま差し替える（`super::switch`）ので、
/// **止める・起こすが稼働中に何度も起きる。** そのため
///
/// * **番人は子ごとに持つ**（[`StartedUpstream::watch`]）。
/// * **`--upstream` の無い起動でも、空の束を持つ**（[`Self::idle`]）——
///   環境変数で起動してから、gguf のプロファイルへ差し替えられるように。
pub(in crate::server) struct Supervisor {
    started: Vec<StartedUpstream>,
    states: SharedLaunchStates,
    /// 出力の落とし先（**起こすたびに作る。空の束はディレクトリを作らない**）。
    log_dir: PathBuf,
    /// 番人を起こすのに要るもの。**`serve` までは `None`。**
    armed: Option<Armed>,
}

/// **差し替えで起こせなかったときに、面の状態へ書く理由。**
///
/// **定数にしてある** —— `SpawnFailure` の `Display` は `command` の綴り（絶対パスでありうる）を
/// 含むので、**応答に出る欄へは載せない**（絶対制約）。詳細はログにだけ出す。
const SPAWN_FAILED_REASON: &str =
    "上流を起こせませんでした（プロファイルの command と、personacore のログを確かめてください）";

impl Supervisor {
    /// **何も起こしていない束。** ファイルにも触らない。
    pub(in crate::server) const fn idle(log_dir: PathBuf, states: SharedLaunchStates) -> Self {
        Self {
            started: Vec::new(),
            states,
            log_dir,
            armed: None,
        }
    }

    /// **計画のとおりに起こす。**
    ///
    /// 1. **gguf の実在を、1つも起こす前に全部確かめる**（ADR-0042 §決定2）。
    ///    途中で気づくと、**半分だけ GPU を掴んだ状態で止まる。**
    /// 2. 起こして、**その場で控える。**（出力の落とし先は [`spawn_one`] が用意する）
    ///
    /// **失敗したら、それまでに起こした子は [`Drop`] が殺す**
    /// （`kill_on_drop(true)`）——`?` で抜ける経路にも控えが効く。
    pub(in crate::server) async fn start(
        plan: &LaunchPlan,
        log_dir: &Path,
        states: &SharedLaunchStates,
    ) -> Result<Self, LaunchError> {
        // **1つも起こす前に、全部の gguf を見る。**
        if let Some(missing) = plan.missing_gguf() {
            return Err(LaunchError::MissingGguf(missing));
        }

        let mut supervisor = Self::idle(log_dir.to_path_buf(), states.clone());
        for launch in &plan.launches {
            // **`?` はここ。** 途中で失敗しても、`supervisor` が落ちる時点で
            // それまでの子は `kill_on_drop` が殺す。
            let started = spawn_one(launch, &supervisor.log_dir)?;
            // **控えるのはここ。載るのを待つのは番人（`arm` の後）。**
            supervisor.adopt(started);
        }
        Ok(supervisor)
    }

    /// **起こした子を控え、面を「載せている」にする。** 番人が揃っていれば番人も起こす。
    fn adopt(&mut self, mut started: StartedUpstream) {
        for face in started.faces() {
            self.states.set(
                *face,
                FaceState::Loading {
                    since: chrono::Local::now(),
                },
            );
        }
        if let Some(armed) = self.armed.as_ref() {
            started.watch = Some(spawn_watch(&started, armed, &self.states));
        }
        self.started.push(started);
    }

    /// **載るのを待つ番人を揃える**（ADR-0042 §決定3）。**`serve` から1回だけ呼ぶ。**
    ///
    /// **既に起こした子には、ここで番人を付ける。** 以後に起こす子（差し替え）には、
    /// 起こした場で付く（[`Self::adopt`]）。
    pub(in crate::server) fn arm(&mut self, clients: FaceClients, sinks: LoadSinks) {
        let armed = Armed { clients, sinks };
        for started in &mut self.started {
            if started.watch.is_none() {
                started.watch = Some(spawn_watch(started, &armed, &self.states));
            }
        }
        self.armed = Some(armed);
    }

    /// 番人を起こすのに要るもの。**`None` は「まだ `serve` していない」か
    /// 「照会先を持たない起動」**（モックを注入する入口）。
    pub(in crate::server) const fn armed(&self) -> Option<&Armed> {
        self.armed.as_ref()
    }

    /// 面ごとの状態の器（**書き手と同じ器**）。
    pub(in crate::server) const fn states(&self) -> &SharedLaunchStates {
        &self.states
    }

    /// **いま走らせている子の記述**（起こした順）。
    pub(in crate::server) fn running(&self) -> Vec<ProfileLaunch> {
        self.started
            .iter()
            .map(|started| started.spec.clone())
            .collect()
    }

    /// **控えた子だけを止め、終了を待つ**（ADR-0042 §決定2）。**終了時に呼ぶ。**
    ///
    /// **番人を先に止める**——止めないと、止めた子を見て「失敗」と書きに来る。
    pub(in crate::server) async fn stop(&mut self) {
        for started in std::mem::take(&mut self.started) {
            let faces = started.faces().to_vec();
            stop_started(started).await;
            for face in faces {
                self.states.set(face, FaceState::Stopped);
            }
        }
    }

    /// **`keep` に無い子を止め、終了を待つ**（作業単位 F-4b-2 / ADR-0042 §決定4）。
    ///
    /// **面の状態は書かない** —— 止めた面が次に何になるか（起こし直す／外の上流になる）は
    /// 差し替えの側が決める。**止めた子が担っていた面を返す。**
    pub(in crate::server) async fn stop_except(
        &mut self,
        keep: &[ProfileLaunch],
    ) -> Vec<ModelFace> {
        let (kept, stopping): (Vec<_>, Vec<_>) = std::mem::take(&mut self.started)
            .into_iter()
            .partition(|started| keep.contains(&started.spec));
        self.started = kept;

        let mut freed = Vec::new();
        for started in stopping {
            freed.extend_from_slice(started.faces());
            // **1つずつ、終了を待ってから次へ。** 待ち終わるまで次を起こさない。
            stop_started(started).await;
        }
        freed
    }

    /// **差し替えで新しく起こす**（作業単位 F-4b-2）。
    ///
    /// **起こせなかった子は、その面を「失敗」にして続ける** ——
    /// 前の子は既に止めてあるので、**起動時のように全体を止めて戻る場所が無い。**
    /// **理由は定数**（[`SPAWN_FAILED_REASON`]）で、詳細はログにだけ出す。
    pub(in crate::server) fn start_more(&mut self, launches: &[ProfileLaunch]) {
        for launch in launches {
            match spawn_one(launch, &self.log_dir) {
                Ok(started) => self.adopt(started),
                Err(err) => {
                    tracing::warn!(
                        upstream_face = %super::upstreams::face_labels(launch.faces()),
                        upstream_endpoint = %endpoint_for_log(launch.endpoint()),
                        error = %err,
                        cause = ?std::error::Error::source(&err),
                        "差し替えた上流を起こせませんでした"
                    );
                    for face in launch.faces() {
                        self.states.set(
                            *face,
                            FaceState::Failed {
                                reason: SPAWN_FAILED_REASON.to_owned(),
                                log: None,
                            },
                        );
                    }
                }
            }
        }
    }
}

/// **1つの子を止める**（番人を止めて待ち、子を止めて待つ）。
async fn stop_started(mut started: StartedUpstream) {
    if let Some(watch) = started.watch.take() {
        watch.abort();
        // **番人が本当に止まるまで待つ。** `abort` は次の待ち点で効くので、
        // その手前で観測を書いている最中の番人が、止めた後に書き終えうる。
        let _ = watch.await;
    }
    tracing::info!(
        upstream_pid = ?started.pid,
        upstream_endpoint = %endpoint_for_log(started.endpoint()),
        "起こした上流を止めます"
    );
    let mut child = started.child.lock().await;
    stop_process(&mut *child, started.pid).await;
}

/// **番人を起こす。**
fn spawn_watch(
    started: &StartedUpstream,
    armed: &Armed,
    states: &SharedLaunchStates,
) -> tokio::task::JoinHandle<()> {
    let watcher = FaceWatcher {
        child: Arc::clone(&started.child),
        faces: started.faces().to_vec(),
        // **ログに出す形で持つ**（番人はこの綴りを行に載せるだけ / 作業単位 B-71）。
        endpoint_for_log: endpoint_for_log(started.endpoint()),
        expected_name: started.expected_name.clone(),
        out_log: started.out_log.clone(),
        err_log: started.err_log.clone(),
        clients: armed.clients.clone(),
        sinks: armed.sinks.clone(),
        states: states.clone(),
    };
    tokio::spawn(watcher.run())
}

/// 起こした子を落とす先（引き直しと予算の器）。
///
/// **3つを別々に持ち回らない**——渡すときに順番を取り違えても型が通る形にしない
/// （`config::DriftTuning` と同じ判断）。
#[derive(Clone)]
pub(in crate::server) struct LoadSinks {
    budget: SharedBudget,
    sub_context: SharedSubContext,
    upstream: SharedUpstream,
    /// **起動時の `PERSONACORE_CONTEXT_TOKENS`**（ADR-0054 §決定3 / 作業単位 A-13a-1）。
    ///
    /// **走ったまま値の無いプロファイルへ切り替えたときに戻る先。**
    /// **「いま明示か」はここではなく器（`budget` / `sub_context`）が持つ**——
    /// F-4b-2 までの `main_budget_is_explicit: bool` は起動時に決めた値で、切り替えに追従しなかった。
    environment: Option<crate::prompt::TokenBudget>,
}

impl LoadSinks {
    pub(in crate::server) const fn new(
        budget: SharedBudget,
        sub_context: SharedSubContext,
        upstream: SharedUpstream,
        environment: Option<crate::prompt::TokenBudget>,
    ) -> Self {
        Self {
            budget,
            sub_context,
            upstream,
            environment,
        }
    }

    /// **照会1回ぶんを、その面へ採る**（`context::adopt_face_probe`）。
    ///
    /// **明示の面は観測だけ書いて、値は採らない**（器が「明示か」を持つ）。
    pub(in crate::server) fn adopt(&self, face: ModelFace, probe: &super::upstream::UpstreamProbe) {
        super::context::adopt_face_probe(
            face,
            probe,
            &self.budget,
            &self.sub_context,
            &self.upstream,
        );
    }

    /// **その面を新しいクライアントへ差し替え、「まだ何も聞いていない」に戻す**
    /// （作業単位 F-4b-2 / ADR-0042 §決定4 / `context::install_face`）。
    ///
    /// **古い事実を残さない**——観測は新しい宛先と綴りの「一度も聞いていない」に、
    /// 予算と抽出の上限は**切り替え先の明示**（`profile` の値、本文は無ければ起動時の環境変数 /
    /// ADR-0054 §決定3）に、明示が無ければ落とし所に戻す。
    pub(in crate::server) fn install(
        &self,
        clients: &FaceClients,
        face: ModelFace,
        candidate: super::context::Candidate,
        profile: super::config::ProfileContext,
    ) {
        super::context::install_face(
            clients,
            face,
            candidate,
            &self.budget,
            &self.sub_context,
            &self.upstream,
            self.environment,
            profile,
        );
    }

    /// **いまその面が明示している値**（トークン数 / 明示でなければ `None`）。`super::switch` が新旧を比べる。
    pub(in crate::server) fn explicit_tokens(&self, face: ModelFace) -> Option<u32> {
        match face {
            ModelFace::Main => self.budget.explicit_tokens(),
            ModelFace::Sub => self.sub_context.explicit_tokens(),
        }
    }

    /// **そのプロファイルへ切り替えたら、その面が明示する値**（トークン数）。**`warn` は出さない**
    /// ——比べるだけ（言うのは実際に差し替える [`Self::install`]）。
    ///
    /// **本文は面の値、無ければ起動時の環境変数**（ADR-0054 §決定2・3）。**抽出は面の値だけ。**
    pub(in crate::server) fn next_explicit_tokens(
        &self,
        face: ModelFace,
        profile: super::config::ProfileContext,
    ) -> Option<u32> {
        match face {
            ModelFace::Main => profile
                .main
                .or(self.environment)
                .map(crate::prompt::TokenBudget::context_tokens),
            ModelFace::Sub => profile.sub.map(super::extract::SubContext::tokens),
        }
    }

    /// **いまの本文の面の値**（思考の量と返答の上限 / ADR-0059 / 作業単位 A-18a）。`super::switch` が新旧を比べる。
    ///
    /// **裏方の面にはこの値が無い**（比べる側は既定と比べる —— `switch::Target` の doc）。
    pub(in crate::server) fn body(&self) -> super::config::BodyFace {
        self.budget.body()
    }

    /// **いまの本文の予算と、落とし所の窓の印**（切り替えの後に送る値を言う / `context::report_samplers`）。
    pub(in crate::server) fn main_budget(
        &self,
    ) -> super::context::Chosen<crate::prompt::TokenBudget> {
        self.budget.current()
    }

    /// **起動時の `PERSONACORE_CONTEXT_TOKENS`**（切り替え先の返答の上限が収まるかを確かめる /
    /// `config::ProfileContext::of` / ADR-0059 §決定2）。
    pub(in crate::server) const fn environment(&self) -> Option<crate::prompt::TokenBudget> {
        self.environment
    }
}

// ----------------------------------------------------------------------
// 起こす（1つぶん）
// ----------------------------------------------------------------------

fn face_label_of(launch: &ProfileLaunch) -> &'static str {
    // **どの面のための子かを、必ず名乗る**（2つ起こす形があるので）。
    launch.faces().first().map_or("main", |face| face.label())
}

fn spawn_one(launch: &ProfileLaunch, log_dir: &Path) -> Result<StartedUpstream, LaunchError> {
    let spec = launch.launch();
    let stamp = chrono::Local::now().format(LOG_TIME_FORMAT).to_string();
    let label = super::upstreams::face_labels(launch.faces());
    // **起こす直前に、その面の古い世代を消して名付ける**（ADR-0042 §決定7）。
    let (out_name, err_name, out, err) = prepare_logs(log_dir, &label, &stamp)?;

    // **引数の形は `F-9` と同じ**（`llama serve -m <gguf> --host <h> --port <p> <args…>`）。
    //
    // **`--host` / `--port` は `endpoint` から作って必ず渡す**（`args` に書かせない。
    // 書けば `upstreams` の検証が止める / 作業単位 F-4b-2）。
    // **引用符は足さない**——`Command` は引数を1つずつ渡すので、
    // **空白を含む gguf のパスがそこで割れることは無い**
    // （PowerShell の `Start-Process` はそれができず、`F-9` は手で囲っていた）。
    let mut command = Command::new(spec.command());
    command
        .arg("serve")
        .arg("-m")
        .arg(spec.gguf())
        .arg("--host")
        .arg(spec.serve_host())
        .arg("--port")
        .arg(spec.serve_port().to_string())
        .args(spec.extra_args())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(out))
        .stderr(std::process::Stdio::from(err))
        // **控えそこねた経路でも殺す。** `Supervisor::start` が `?` で抜けるとき、
        // それまでに起こした子はここで死ぬ。
        .kill_on_drop(true);

    tracing::info!(
        upstream_face = %label,
        // **ユーザー情報・クエリ・フラグメントは落として出す**（作業単位 B-71 / `llm::endpoint_for_log`）。
        upstream_endpoint = %endpoint_for_log(launch.endpoint()),
        // **Debug で書く**——利用者のファイルの綴りで、改行を通すと行が割れる（F-4f レビュー2周目）。
        upstream_command = ?spec.command(),
        upstream_log = %out_name,
        "上流を起こします"
    );

    let child = command.spawn().map_err(|source| {
        LaunchError::Spawn(SpawnFailure {
            command: spec.command().to_owned(),
            source,
        })
    })?;
    // **PID はここで控える。** 載るのを待つ前（ADR-0042 §決定2）。
    let pid = child.id();
    tracing::info!(
        upstream_face = %label,
        upstream_pid = ?pid,
        "上流を起こしました（載るのは背景で待ちます）"
    );

    Ok(StartedUpstream {
        child: Arc::new(tokio::sync::Mutex::new(child)),
        pid,
        spec: launch.clone(),
        // **載ったと認める名前は gguf のファイル名**（`B-45` と同じ根拠）。
        expected_name: spec
            .gguf()
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
        out_log: out_name,
        err_log: err_name,
        watch: None,
    })
}

// ----------------------------------------------------------------------
// `upstream-logs/` の世代（ADR-0042 §決定7 / 作業単位 F-4b-2）
// ----------------------------------------------------------------------

/// **面ごとに残す世代の数**（自分が起こすぶんを含む / ADR-0042 §決定7）。
///
/// **実測値ではなく方針**（ADR-0036 の問いには「遅くも壊れもしない。残る数が変わるだけ」）。
/// **つまみにしない。**
const LOG_GENERATIONS: usize = 5;

/// 標準出力の落とし先の接尾辞。
const OUT_LOG_SUFFIX: &str = ".log";

/// 標準エラーの落とし先の接尾辞。**[`OUT_LOG_SUFFIX`] でも終わる**ので、先に見ること。
const ERR_LOG_SUFFIX: &str = ".err.log";

/// 時刻の綴りの長さ（[`LOG_TIME_FORMAT`] ＝ `20260918-101112`）。
const STAMP_LEN: usize = 15;

/// 同じ秒の名前を探す上限（**待つ側に上限を置く** / CLAUDE.md `G-4`）。
const MAX_NAME_ATTEMPTS: usize = 64;

/// **1回の起動ぶんの印**（時刻の綴りと、同じ秒の中の番号）。
///
/// **並びは「時刻 → 番号」**（`derive(Ord)` の欄の順）。時刻は固定幅の数字なので、
/// **綴りの辞書順がそのまま時刻の順になる。** 番号の無い名前は `1`。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LogGeneration {
    stamp: String,
    seq: u32,
}

/// `YYYYMMDD-HHMMSS` の形か。
fn is_stamp(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == STAMP_LEN
        && bytes.iter().enumerate().all(|(at, byte)| {
            if at == 8 {
                *byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
}

/// **その名前は、この面のために exe が名付けたログか。純粋関数**（ADR-0042 §決定7）。
///
/// 読むのは `<面>-<YYYYMMDD-HHMMSS>[-<番号>].log` と `.err.log` だけ。
/// **番号は 2 以上で、先頭に 0 を付けない**（exe が付ける形だけを読む）。
/// **面の綴りは完全一致**——`main` の掃除は `main+sub-…` を数えない（面ごとに数える）。
fn log_generation(label: &str, name: &str) -> Option<LogGeneration> {
    let rest = name.strip_prefix(label)?.strip_prefix('-')?;
    let body = rest
        .strip_suffix(ERR_LOG_SUFFIX)
        .or_else(|| rest.strip_suffix(OUT_LOG_SUFFIX))?;
    let (stamp, tail) = body.split_at_checked(STAMP_LEN)?;
    if !is_stamp(stamp) {
        return None;
    }
    let seq = if tail.is_empty() {
        1
    } else {
        let digits = tail.strip_prefix('-')?;
        let seq: u32 = digits.parse().ok()?;
        if seq < 2 || seq.to_string() != digits {
            return None;
        }
        seq
    };
    Some(LogGeneration {
        stamp: stamp.to_owned(),
        seq,
    })
}

/// **消す名前**——この面の世代のうち、新しいほうから `keep` 個を残した残り。**純粋関数。**
///
/// **世代の単位は1回の起動**（`.log` と `.err.log` は一緒に消える / ADR-0042 §決定7）。
fn stale_logs(label: &str, existing: &[String], keep: usize) -> Vec<String> {
    let generations: std::collections::BTreeSet<LogGeneration> = existing
        .iter()
        .filter_map(|name| log_generation(label, name))
        .collect();
    let excess = generations.len().saturating_sub(keep);
    let doomed: Vec<&LogGeneration> = generations.iter().take(excess).collect();
    let mut stale: Vec<String> = existing
        .iter()
        .filter(|name| log_generation(label, name).is_some_and(|found| doomed.contains(&&found)))
        .cloned()
        .collect();
    stale.sort();
    stale
}

/// **この起動の2つの名前。純粋関数**（ADR-0042 §決定7 の「同じ秒に2回起こす腕」）。
///
/// **同じ秒に同じ面の世代が既に在れば、その秒のいちばん大きい番号の次を付ける**
/// ——`File::create` が1つ前の起動のログ（しばしば失敗の理由）を黙って空にしないように。
/// **番号は「空いている最小」ではなく「最大の次」**——途中が消えていても、
/// 後から起こしたものが必ず新しい世代として並ぶ。
fn log_names(label: &str, stamp: &str, existing: &[String]) -> (String, String) {
    let taken = existing
        .iter()
        .filter_map(|name| log_generation(label, name))
        .filter(|generation| generation.stamp == stamp)
        .map(|generation| generation.seq)
        .max();
    let base = match taken {
        None => format!("{label}-{stamp}"),
        Some(seq) => format!("{label}-{stamp}-{}", seq.saturating_add(1)),
    };
    (
        format!("{base}{OUT_LOG_SUFFIX}"),
        format!("{base}{ERR_LOG_SUFFIX}"),
    )
}

/// **その面の古い世代を消し、この起動の落とし先を開く**（ADR-0042 §決定7）。
///
/// * **消すのは exe が名付けた形だけ**（[`log_generation`]）。
/// * **消せなくても止めない。** `warn` で名前を言って、起こすほうを続ける
///   （ログの掃除は上流を起こす理由より弱い）。
/// * **開くのは `create_new`**——同じ名前が在れば番号を進めて開き直す（上書きしない）。
fn prepare_logs(
    dir: &Path,
    label: &str,
    stamp: &str,
) -> Result<(String, String, std::fs::File, std::fs::File), LaunchError> {
    std::fs::create_dir_all(dir).map_err(LaunchError::LogDir)?;

    let mut existing: Vec<String> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(err) => {
            // **読めなくても起こす。** 掃除できないだけで、落とし先は開ける。
            tracing::warn!(
                upstream_face = %label,
                error = %err,
                "上流のログの置き場所を読めませんでした（古いログは消していません）"
            );
            Vec::new()
        }
    };

    for name in stale_logs(label, &existing, LOG_GENERATIONS - 1) {
        match std::fs::remove_file(dir.join(&name)) {
            Ok(()) => tracing::info!(upstream_log = %name, "古いログを消しました"),
            Err(err) => tracing::warn!(
                upstream_log = %name,
                error = %err,
                "古いログを消せませんでした（上流は起こします）"
            ),
        }
    }

    for _ in 0..MAX_NAME_ATTEMPTS {
        let (out_name, err_name) = log_names(label, stamp, &existing);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(&out_name))
        {
            Ok(out) => {
                let err =
                    std::fs::File::create(dir.join(&err_name)).map_err(LaunchError::LogDir)?;
                return Ok((out_name, err_name, out, err));
            }
            // **一覧を読んだ後に誰かが同じ名前を作った。** 次の番号で開き直す。
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => existing.push(out_name),
            Err(err) => return Err(LaunchError::LogDir(err)),
        }
    }
    Err(LaunchError::LogDir(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "上流のログの名前が空きませんでした",
    )))
}

// ----------------------------------------------------------------------
// 載るのを待つ
// ----------------------------------------------------------------------

struct FaceWatcher {
    child: Arc<tokio::sync::Mutex<Child>>,
    faces: Vec<ModelFace>,
    /// **ログに出す宛先**（`llm::endpoint_for_log` を通した後。ユーザー情報・クエリ・フラグメントは無い /
    /// 作業単位 B-71）。**行に載せる以外に使わない**（送信の宛先はクライアントが持つ）。
    endpoint_for_log: String,
    expected_name: String,
    out_log: String,
    err_log: String,
    clients: FaceClients,
    sinks: LoadSinks,
    states: SharedLaunchStates,
}

impl FaceWatcher {
    /// **1つの子について、載るまで待つ。**
    ///
    /// | 見えたこと | どうするか |
    /// |---|---|
    /// | **子が自分で終了した** | 即「失敗」（終了コードとログの名前を添える）。**死んだ書き手を待たない** |
    /// | **一覧が gguf の名前を名乗った** | 「準備完了」。**その往復の観測をそのまま採る**（往復を増やさない） |
    /// | **一覧が別の名前を名乗った** | 即「失敗」（載ったのは別のモデル） |
    /// | **まだ答えない** | 待つ。[`PROGRESS_INTERVAL`] ごとに `info`、[`LOAD_CEILING`] を超えたら `warn` 1本 |
    ///
    /// # ⚠ 決着したら、この番人は帰る（**`ready` は張り付く**）
    ///
    /// **載ったあと上流が落ちても、`launch.state` は `ready` のままである。**
    /// **それは「この実行ファイルが何をしたか」の記録**であって、いまの疎通ではない。
    /// **いま答えるかどうかは `answered_by`**（5分ごとの引き直しが `null` にする /
    /// `server::upstream` の doc）。**2つを畳まないのは、そのためである。**
    async fn run(self) {
        let started_at = std::time::Instant::now();
        let mut last_progress = started_at;
        let mut warned = false;
        let label = super::upstreams::face_labels(&self.faces);

        loop {
            // **毎周「書き手が生きているか」を見る**（CLAUDE.md `G-4`）。
            // **番兵は、書き手が死んだ瞬間に永久に来ない。**
            let exited = {
                let mut child = self.child.lock().await;
                // **止める側と同じ継ぎ目を通る**（[`UpstreamProcess`] の doc）。
                UpstreamProcess::try_wait(&mut *child).ok().flatten()
            };
            if let Some(status) = exited {
                let reason = format!("上流が起動直後に終了しました（終了コード {status}）");
                tracing::warn!(
                    upstream_face = %label,
                    upstream_endpoint = %self.endpoint_for_log,
                    upstream_exit = %status,
                    upstream_log = %self.err_log,
                    "起こした上流が自分で終了しました"
                );
                self.mark_failed(&reason);
                return;
            }

            // **載ったかは、その面の照会先へ聞く**（`context::probe_upstream`）。
            // **確かめるのは「答えた名前」**——`llama serve` は `model` の欄を無視するので、
            // **頼んだ綴りでは載ったモデルを確かめられない**（`B-45`）。
            let first = self.faces.first().copied().unwrap_or(ModelFace::Main);
            //
            // **載せている最中の照会で聞く**（作業単位 B-51 / Issue origin#20）——
            // 載るまでの 503 は期待している状態なので、「どちらの綴りでも読めない」を
            // `warn` ではなく `debug` で言う。**経過はこの番人が下で `info` で言う。**
            // **`probe_face` に戻すと、2秒ごとに警告が鳴る**（20 秒の載せ替えで 8 行 / 実機）。
            let (probe, _failure) =
                super::context::probe_face_while_loading(&self.clients, first).await;

            match probe.answered_name.as_deref() {
                Some(name) if name == self.expected_name => {
                    tracing::info!(
                        upstream_face = %label,
                        upstream_endpoint = %self.endpoint_for_log,
                        upstream_answered_name = %name,
                        elapsed_seconds = started_at.elapsed().as_secs(),
                        "上流にモデルが載りました"
                    );
                    self.adopt(first, &probe).await;
                    for face in self.faces.iter().skip(1) {
                        // **面ごとに聞き直す**——窓も名乗りもモデルごとに違う
                        // （`context::resolve_sub_context` の doc）。
                        let (probe, _) = super::context::probe_face(&self.clients, *face).await;
                        self.adopt(*face, &probe).await;
                    }
                    for face in &self.faces {
                        self.states.set(
                            *face,
                            FaceState::Ready {
                                answered_name: name.to_owned(),
                            },
                        );
                    }
                    return;
                }
                Some(other) => {
                    let reason = format!(
                        "上流が別のモデルを名乗っています（{other}／期待は {}）",
                        self.expected_name
                    );
                    tracing::warn!(
                        upstream_face = %label,
                        upstream_endpoint = %self.endpoint_for_log,
                        upstream_answered_name = %other,
                        upstream_expected_name = %self.expected_name,
                        upstream_log = %self.out_log,
                        "起こした上流に別のモデルが載っています"
                    );
                    self.mark_failed(&reason);
                    return;
                }
                None => {}
            }

            let elapsed = started_at.elapsed();
            // **超えても止めない**（ADR-0035）。**言うだけ。**
            if !warned && elapsed >= LOAD_CEILING {
                warned = true;
                tracing::warn!(
                    upstream_face = %label,
                    upstream_endpoint = %self.endpoint_for_log,
                    elapsed_seconds = elapsed.as_secs(),
                    ceiling_seconds = LOAD_CEILING.as_secs(),
                    upstream_log = %self.out_log,
                    "上流にモデルがまだ載っていません。待ち続けます（止めていません）"
                );
            }
            if last_progress.elapsed() >= PROGRESS_INTERVAL {
                last_progress = std::time::Instant::now();
                tracing::info!(
                    upstream_face = %label,
                    upstream_endpoint = %self.endpoint_for_log,
                    elapsed_seconds = elapsed.as_secs(),
                    "上流にモデルを載せています"
                );
            }

            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    /// **失敗を、担う面の全部へ書く。**
    fn mark_failed(&self, reason: &str) {
        for face in &self.faces {
            self.states.set(
                *face,
                FaceState::Failed {
                    reason: reason.to_owned(),
                    log: Some(self.err_log.clone()),
                },
            );
        }
    }

    /// **載った直後の1回ぶんを採る**（ADR-0042 §決定3）。
    async fn adopt(&self, face: ModelFace, probe: &super::upstream::UpstreamProbe) {
        super::context::adopt_face_probe(
            face,
            probe,
            &self.sinks.budget,
            &self.sinks.sub_context,
            &self.sinks.upstream,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **`FaceState::all_labels` に数え漏れが無い**（作業単位 P-2a）。
    ///
    /// 網羅の `match`（**ワイルドカードの腕を足さないこと**）。variant を足すとここが
    /// コンパイルできなくなり、足した腕は `all_labels` に載っていることを主張する。
    #[test]
    fn every_state_is_listed_in_all_labels() {
        let listed = |state: &FaceState| match state {
            FaceState::External
            | FaceState::NotStarted
            | FaceState::Loading { .. }
            | FaceState::Ready { .. }
            | FaceState::Failed { .. }
            | FaceState::Stopped => FaceState::all_labels().contains(&state.label()),
        };
        for state in [
            FaceState::External,
            FaceState::NotStarted,
            FaceState::Loading {
                since: chrono::Local::now(),
            },
            FaceState::Ready {
                answered_name: "a.gguf".to_owned(),
            },
            FaceState::Failed {
                reason: "x".to_owned(),
                log: None,
            },
            FaceState::Stopped,
        ] {
            assert!(listed(&state), "{state:?} が all_labels に無い");
        }
    }

    /// **状態の綴りは、6つとも別**（画面と応答が読む正）。
    #[test]
    fn every_state_has_its_own_label() {
        let labels = [
            FaceState::External.label(),
            FaceState::NotStarted.label(),
            FaceState::Loading {
                since: chrono::Local::now(),
            }
            .label(),
            FaceState::Ready {
                answered_name: "a.gguf".to_owned(),
            }
            .label(),
            FaceState::Failed {
                reason: "x".to_owned(),
                log: None,
            }
            .label(),
            FaceState::Stopped.label(),
        ];
        let unique: std::collections::BTreeSet<&str> = labels.iter().copied().collect();
        assert_eq!(unique.len(), labels.len(), "綴りが重なっている: {labels:?}");
    }

    /// **面ごとに別々に書ける。反対の面は触らない**（帰属）。
    #[test]
    fn writing_one_face_leaves_the_other_alone() {
        let states = SharedLaunchStates::external();
        states.set(
            ModelFace::Sub,
            FaceState::Ready {
                answered_name: "sub.gguf".to_owned(),
            },
        );

        let read = states.get();
        assert_eq!(
            read.face(ModelFace::Main),
            &FaceState::External,
            "本文の面が巻き込まれている"
        );
        assert_eq!(
            read.face(ModelFace::Sub),
            &FaceState::Ready {
                answered_name: "sub.gguf".to_owned()
            }
        );
    }

    /// **空の計画は、どの面も起こさない。**
    #[test]
    fn an_empty_plan_launches_no_face() {
        let plan = LaunchPlan::none();
        assert!(plan.is_empty());
        assert!(!plan.launches_face(ModelFace::Main));
        assert!(!plan.launches_face(ModelFace::Sub));
    }

    /// **計画は、gguf を書いた面だけを起こす**（両向きを1本で）。
    #[test]
    fn the_plan_names_only_the_faces_that_carry_a_gguf() {
        let profiles = super::super::upstreams::parse_profiles(
            r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:1234", "model": "m" },
                          "sub":  { "endpoint": "http://127.0.0.1:8080", "model": "n",
                                    "gguf": "/planted/b.gguf" } } }"#,
            None,
        )
        .expect("読めること");
        let plan = LaunchPlan::of(profiles.select("one").expect("在ること"));

        assert!(!plan.is_empty());
        assert!(
            !plan.launches_face(ModelFace::Main),
            "gguf の無い面は起こさない"
        );
        assert!(
            plan.launches_face(ModelFace::Sub),
            "gguf を書いた面は起こす"
        );
    }

    /// **gguf が無ければ、1つも起こさずに止まる**（ADR-0042 §決定2）。
    ///
    /// # 空振りではないこと
    ///
    /// **`command` に在りえない綴りを書いてある**ので、
    /// **gguf の確認を外した実装はここで `Spawn` の側に落ちる**
    /// ——「止まったこと」ではなく「**どの理由で**止まったか」を見る。
    #[tokio::test]
    async fn a_missing_gguf_stops_before_anything_is_started() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let profiles = super::super::upstreams::parse_profiles(
            r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m",
                                    "gguf": "/planted/never-here.gguf",
                                    "command": "personacore-no-such-command" } } }"#,
            None,
        )
        .expect("読めること");
        let plan = LaunchPlan::of(profiles.select("one").expect("在ること"));
        let states = SharedLaunchStates::external();

        // **`Supervisor` は `Debug` を持たない**（中身は子プロセスそのもの）ので、
        // `expect_err` ではなく `let else` で取り出す。
        let Err(err) = Supervisor::start(&plan, &dir.path().join("logs"), &states).await else {
            panic!("gguf が無いのに起こしている");
        };

        assert!(
            matches!(err, LaunchError::MissingGguf(_)),
            "gguf の実在で止まること（起こしに行っていない）: {err:?}"
        );
        assert_eq!(
            err.to_string(),
            "上流のモデルファイルが見つかりません",
            "表の文は定数文字列であること"
        );
        // **状態も動いていない。**
        assert_eq!(states.get().face(ModelFace::Main), &FaceState::External);
        // **ログの置き場所も作っていない**（起こす前に止まる）。
        assert!(
            !dir.path().join("logs").exists(),
            "1つも起こさないのにログの置き場所を作っている"
        );
    }

    // ------------------------------------------------------------------
    // 待ち受けとの取り合い（作業単位 B-52 / Issue origin#21）
    // ------------------------------------------------------------------

    /// **植えた綴りから計画を作る。**
    ///
    /// **ポートは 8080（[`super::super::config::DEFAULT_PORT`]）を使わない** ——
    /// 既定に決め打った実装と、渡されたポートを見る実装が区別できなくなる
    /// （CLAUDE.md「実行時に決まる値を入れたら、既定と違う状態で回る腕を置く」）。
    fn plan_of(source: &str) -> LaunchPlan {
        let profiles = super::super::upstreams::parse_profiles(source, None).expect("読めること");
        LaunchPlan::of(profiles.select("one").expect("在ること"))
    }

    /// **自分の待ち受けと同じポートを取りに行く面は、衝突として見つかる。**
    ///
    /// **宛先のホストの綴りを `localhost` にしてある** ——
    /// **待ち受けの綴り（`127.0.0.1:…`）と字面が重ならない**ので、
    /// 「両方の値を挙げた」が2つの独立した主張になる
    /// （同じ綴りだと、片方を落とす実装でも両方の `contains` が通る）。
    /// **取り合いの文面は、宛先の秘密（ユーザー情報・クエリ・フラグメント）を落として挙げる**
    /// （作業単位 B-71 のレビュー是正）。`Display` は起動を止めた理由としてログと標準エラーへ出る。
    ///
    /// **肯定側**（ホスト・ポート・パスは挙げる）を同じ文面で先に見る。
    #[test]
    fn a_port_conflict_names_the_endpoint_without_its_secrets() {
        let plan = plan_of(
            r#"{ "one": { "main": {
                   "endpoint": "http://u:pw-secret@localhost:45101/v1?token=q-secret#f-secret",
                   "model": "m", "gguf": "/planted/a.gguf" } } }"#,
        );

        let conflict = plan
            .port_conflict(BindAddr::loopback(45_101))
            .expect("同じポートなら衝突として見つかること");
        let rendered = conflict.to_string();
        let debugged = format!("{conflict:?}");
        assert!(
            rendered.contains("http://localhost:45101/v1"),
            "前提: 宛先のホスト・ポート・パスを挙げていること: {rendered}"
        );
        for secret in ["pw-secret", "q-secret", "f-secret", "u:"] {
            assert!(
                !rendered.contains(secret),
                "文面に宛先の秘密が出ている（{secret}）: {rendered}"
            );
            assert!(
                !debugged.contains(secret),
                "Debug に宛先の秘密が出ている（{secret}）: {debugged}"
            );
        }
    }

    /// **事前検査は、待ち受けのアドレスを渡したときだけ取り合いを見る**（ADR-0057 §決定3 / 作業単位 F-11b）。
    ///
    /// 同じ計画・同じ番号で、`Some` なら取り合い、`None` なら通る（gguf は在る）。**gguf の実在はどちらでも見る**
    /// （無い gguf の計画は `None` でも止まる）。
    #[test]
    fn the_check_before_starting_sees_the_port_only_when_one_is_given() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let gguf = dir.path().join("there.gguf");
        std::fs::write(&gguf, b"not a real gguf").expect("置けること");
        let plan = plan_of(
            &serde_json::json!({ "one": { "main": {
                "endpoint": "http://localhost:45101", "model": "m", "gguf": gguf,
            } } })
            .to_string(),
        );

        assert!(
            matches!(
                plan.check_before_starting(Some(BindAddr::loopback(45_101))),
                Err(LaunchError::PortCollision(_))
            ),
            "明示した番号と同じなら取り合いで止まること"
        );
        assert!(
            plan.check_before_starting(None).is_ok(),
            "番号を渡さないのに取り合いで止めた"
        );
        assert!(
            plan.check_before_starting(Some(BindAddr::loopback(45_102)))
                .is_ok(),
            "違う番号なのに止めた"
        );

        let missing = plan_of(
            r#"{ "one": { "main": { "endpoint": "http://localhost:45101", "model": "m",
                                    "gguf": "/planted/never-there.gguf" } } }"#,
        );
        assert!(
            matches!(
                missing.check_before_starting(None),
                Err(LaunchError::MissingGguf(_))
            ),
            "番号を渡さなくても gguf の実在は見ること"
        );
    }

    #[test]
    fn a_launch_on_the_servers_own_port_is_a_conflict() {
        let plan = plan_of(
            r#"{ "one": { "main": { "endpoint": "http://localhost:45101", "model": "m",
                                    "gguf": "/planted/a.gguf" } } }"#,
        );

        let conflict = plan
            .port_conflict(BindAddr::loopback(45_101))
            .expect("同じポートなら衝突として見つかること");
        let rendered = conflict.to_string();
        assert!(
            rendered.contains("http://localhost:45101"),
            "プロファイルの宛先を挙げていない: {rendered}"
        );
        assert!(
            rendered.contains("127.0.0.1:45101"),
            "personacore 自身の待ち受けを挙げていない: {rendered}"
        );
        assert!(
            rendered.contains("main"),
            "どの面のことか言っていない: {rendered}"
        );

        // **外向きは定数文字列**（詳細は `#[source]` 側 / CLAUDE.md のエラーの作法）。
        let err = LaunchError::PortCollision(conflict);
        assert_eq!(
            err.to_string(),
            "上流の宛先が personacore 自身の待ち受けと同じです",
            "表の文は定数文字列であること"
        );
    }

    /// **違うポートなら衝突ではない**（対比の腕。これが無いと「常に衝突」が緑を通る）。
    #[test]
    fn a_launch_on_another_port_is_not_a_conflict() {
        let plan = plan_of(
            r#"{ "one": { "main": { "endpoint": "http://localhost:45101", "model": "m",
                                    "gguf": "/planted/a.gguf" } } }"#,
        );
        assert!(
            plan.port_conflict(BindAddr::loopback(45_102)).is_none(),
            "別のポートを取り合いと読んでいる"
        );
    }

    /// **gguf の無い面は縛らない**（同居していないので取り合いにならない）。
    ///
    /// **外の上流の宛先は personacore 自身の待ち受けと同じでも構わない**
    /// ——そこに待ち受けを作ったのはこちらではない。
    #[test]
    fn a_face_without_a_gguf_may_name_the_servers_own_port() {
        let plan = plan_of(
            r#"{ "one": { "main": { "endpoint": "http://localhost:45101", "model": "m" },
                          "sub":  { "endpoint": "http://localhost:45102", "model": "n",
                                    "gguf": "/planted/b.gguf" } } }"#,
        );
        // **空振りではないこと** ——起こす面が1つも無ければ、何を渡しても `None` になる。
        assert!(!plan.is_empty(), "起こす面が無い（この検査が届いていない）");

        assert!(
            plan.port_conflict(BindAddr::loopback(45_101)).is_none(),
            "gguf の無い面まで縛っている"
        );
    }

    /// **待ち受けが `0`（OS に選ばせる）なら、衝突しないと読む。**
    ///
    /// **実際のポートは `bind` の後にしか分からない**が、
    /// **`0` を名指しする面は無い**（書けば `llama serve --port 0` になる）。
    #[test]
    fn a_bind_port_of_zero_is_never_a_conflict() {
        let plan = plan_of(
            r#"{ "one": { "main": { "endpoint": "http://localhost:0", "model": "m",
                                    "gguf": "/planted/a.gguf" } } }"#,
        );
        assert!(
            plan.port_conflict(BindAddr::loopback(0)).is_none(),
            "OS に選ばせる待ち受けを取り合いと読んでいる"
        );
    }

    // ------------------------------------------------------------------
    // 止めたら終了を待つ（ADR-0042 §決定2 / 作業単位 F-4b-2 / Issue origin#2 の持ち越し1）
    // ------------------------------------------------------------------

    /// **止めても終わらない子。** `start_kill` を受けても、合図が来るまで `wait` が返らない。
    ///
    /// **本物の子では「終わるのが遅い」を作れない**（`TerminateProcess` / `SIGKILL` は
    /// 子の側で引き延ばせない）ので、ここだけは作り物で待つ側を見る。
    struct SlowToExit {
        killed: bool,
        exited: Option<tokio::sync::oneshot::Receiver<()>>,
    }

    impl UpstreamProcess for SlowToExit {
        fn start_kill(&mut self) -> std::io::Result<()> {
            self.killed = true;
            Ok(())
        }

        fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
            // **まだ終わっていない**（待たない実装は、ここを見て先へ進む）。
            Ok(None)
        }

        fn wait(
            &mut self,
        ) -> impl std::future::Future<Output = std::io::Result<std::process::ExitStatus>> + Send
        {
            let exited = self.exited.take();
            async move {
                if let Some(exited) = exited {
                    let _ = exited.await;
                }
                Ok(std::process::ExitStatus::default())
            }
        }
    }

    /// **止めたら、子が終わるまで返らない**（ADR-0042 §決定2 / Issue origin#2 の持ち越し1）。
    ///
    /// # なぜ作り物の子なのか
    ///
    /// **Issue の案（偽物にポートを掴ませ、前の子の終了を待たずに次を起こすと
    /// bind に失敗する形）は、競走なしには赤にならない。** `kill` は子を
    /// **数ミリ秒以内に**終わらせ、**次の子が起きて bind するまでには
    /// それ以上かかる**——待たない実装でも、ほぼ毎回ポートは空いている
    /// （この作業単位の変異検査で測った / 報告）。
    /// **「待ったか」は「子がすぐには終わらない」ときにしか現れない**ので、
    /// **その状態を作れる作り物で、待つ側そのものを見る。**
    ///
    /// **同じポートを取り直す腕（本物の偽物）は `tests/upstream_launch.rs` の
    /// `switching_restarts_only_the_face_whose_gguf_changed` に置いてある**
    /// ——あちらは「待った結果、取り直せる」の対照で、待たない変異の見張り番ではない。
    #[tokio::test]
    async fn stopping_a_child_waits_until_it_has_exited() {
        let (exit, exited) = tokio::sync::oneshot::channel();
        let mut child = SlowToExit {
            killed: false,
            exited: Some(exited),
        };

        {
            let mut stopping = std::pin::pin!(stop_process(&mut child, Some(4242)));
            // **合図を出す前は返らない。** 待たない実装はここで返る。
            assert!(
                futures_util::FutureExt::now_or_never(stopping.as_mut()).is_none(),
                "子が終わる前に「止めた」と言って返っている（次を起こすと VRAM を取り合う）"
            );
            let _ = exit.send(());
            stopping.await;
        }
        assert!(child.killed, "終了を待つ前に kill を送っていない");
    }

    /// **対照: 既に終わっている子なら、止めるのはその場で済む**（待ち続けない）。
    ///
    /// **上の腕だけだと「永久に返らない実装」も通る。**
    #[tokio::test]
    async fn stopping_a_child_that_has_already_exited_returns_at_once() {
        let mut child = SlowToExit {
            killed: false,
            exited: None,
        };
        assert!(
            futures_util::FutureExt::now_or_never(stop_process(&mut child, None)).is_some(),
            "終わっている子を止めるのに待ち続けている"
        );
        assert!(child.killed);
    }

    // ------------------------------------------------------------------
    // `upstream-logs/` の世代（ADR-0042 §決定7 / 作業単位 F-4b-2）
    // ------------------------------------------------------------------

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    /// **1回の起動ぶんの2ファイル**（標準出力と標準エラー）。
    fn generation(label: &str, stamp: &str) -> [String; 2] {
        [
            format!("{label}-{stamp}.log"),
            format!("{label}-{stamp}.err.log"),
        ]
    }

    /// **exe が名付けた形だけを、世代として読む**（§決定7「名付けた形のファイルだけを消す」）。
    ///
    /// **両向きを1本に置く** —— 読める綴りと読めない綴りを並べないと、
    /// 「何でも世代と読む」実装と「何も読まない」実装のどちらかが通る。
    #[test]
    fn only_names_the_executable_gives_are_read_as_generations() {
        let at = |stamp: &str, seq: u32| {
            Some(LogGeneration {
                stamp: stamp.to_owned(),
                seq,
            })
        };
        for (name, expected) in [
            ("main-20260924-101112.log", at("20260924-101112", 1)),
            ("main-20260924-101112.err.log", at("20260924-101112", 1)),
            ("main-20260924-101112-2.log", at("20260924-101112", 2)),
            ("main-20260924-101112-13.err.log", at("20260924-101112", 13)),
            // **他の面のものは、この面の世代ではない**（面ごとに数える）。
            ("sub-20260924-101112.log", None),
            ("main+sub-20260924-101112.log", None),
            // **利用者が置いたもの・形の崩れたもの**は触らない。
            ("main-notes.log", None),
            ("main-20260924-101112.txt", None),
            ("main-2026092-101112.log", None),
            ("main-20260924_101112.log", None),
            ("main-20260924-101112-.log", None),
            ("main-20260924-101112-1.log", None),
            ("main-20260924-101112-02.log", None),
            ("main-20260924-101112-x.log", None),
            ("main-20260924-101112.log.bak", None),
            ("xmain-20260924-101112.log", None),
        ] {
            assert_eq!(log_generation("main", name), expected, "{name}");
        }
    }

    /// **6 世代目を起こす前に、最古の1組だけが消える**（§決定7）。
    ///
    /// **他の面の世代と、名付けていないファイルは残る。**
    /// **最古を端に置かない** —— 並びの途中に古いものを混ぜ、並べ替えを見る。
    #[test]
    fn the_sixth_launch_removes_only_the_oldest_generation_of_its_own_face() {
        let mut existing = Vec::new();
        for stamp in [
            "20260924-101500",
            "20260924-101100",
            "20260924-101400",
            "20260924-101000",
            "20260924-101300",
        ] {
            existing.extend(generation("main", stamp));
        }
        // **他の面は5世代より多くても、この面の掃除では触らない。**
        for stamp in [
            "20260101-000000",
            "20260101-000001",
            "20260101-000002",
            "20260101-000003",
            "20260101-000004",
            "20260101-000005",
        ] {
            existing.extend(generation("sub", stamp));
            existing.extend(generation("main+sub", stamp));
        }
        existing.extend(names(&["notes.txt", "main-keep-me.log", "main.log"]));

        let stale = stale_logs("main", &existing, LOG_GENERATIONS - 1);

        assert_eq!(
            stale,
            names(&["main-20260924-101000.err.log", "main-20260924-101000.log"]),
            "最古の1組だけを消すこと"
        );
    }

    /// **5 世代に満たなければ、1つも消さない**（対照）。
    #[test]
    fn fewer_generations_than_the_limit_are_all_kept() {
        let mut existing = Vec::new();
        for stamp in ["20260924-101000", "20260924-101100", "20260924-101200"] {
            existing.extend(generation("main", stamp));
        }
        assert!(stale_logs("main", &existing, LOG_GENERATIONS - 1).is_empty());
    }

    /// **同じ秒に2回起こしても、前の起動のログを潰さない**（§決定7 の見張り番の行）。
    ///
    /// # 決めたこと
    ///
    /// **同じ秒の2回目は `-2`、3回目は `-3` …を後ろに付ける。**
    /// **1回目より新しい世代として数える**（時刻が同じなら番号で並べる）。
    ///
    /// **何もしないと `File::create` が同じ名前を開き直し、
    /// 1つ前の起動のログ（しばしば失敗の理由）を黙って空にする。**
    #[test]
    fn a_second_launch_in_the_same_second_gets_its_own_name() {
        let stamp = "20260924-101112";
        let mut existing: Vec<String> = generation("main", stamp).into();
        assert_eq!(
            log_names("main", stamp, &existing),
            (
                "main-20260924-101112-2.log".to_owned(),
                "main-20260924-101112-2.err.log".to_owned()
            )
        );
        existing.extend(names(&[
            "main-20260924-101112-2.log",
            "main-20260924-101112-2.err.log",
        ]));
        assert_eq!(
            log_names("main", stamp, &existing),
            (
                "main-20260924-101112-3.log".to_owned(),
                "main-20260924-101112-3.err.log".to_owned()
            )
        );
        // **他の面の同じ秒は、番号を進めない**（面ごとに数える）。
        assert_eq!(
            log_names("sub", stamp, &existing),
            (
                "sub-20260924-101112.log".to_owned(),
                "sub-20260924-101112.err.log".to_owned()
            )
        );
    }

    /// **同じ秒の2つは、番号の小さいほうが古い**（最古を消すときに取り違えない）。
    ///
    /// **時刻の綴りだけで並べる実装は、同じ秒の2つを同じ世代に畳む**
    /// ——`-2` の側まで一緒に消える。
    #[test]
    fn within_one_second_the_lower_number_is_the_older_generation() {
        let mut existing = Vec::new();
        existing.extend(generation("main", "20260924-101112"));
        existing.extend(names(&[
            "main-20260924-101112-2.log",
            "main-20260924-101112-2.err.log",
        ]));
        for stamp in ["20260924-101113", "20260924-101114", "20260924-101115"] {
            existing.extend(generation("main", stamp));
        }

        assert_eq!(
            stale_logs("main", &existing, LOG_GENERATIONS - 1),
            names(&["main-20260924-101112.err.log", "main-20260924-101112.log"]),
            "同じ秒の2つのうち、古いほう（番号なし）だけを消すこと"
        );
    }

    /// **起こす道（`spawn_one`）が、掃除と名付けを通る**（§決定7 / 配線の見張り番）。
    ///
    /// **在りえない `command` で6回起こす** —— 起こすこと自体は失敗するが、
    /// **ログの落とし先は起こす前に用意される**ので、世代だけが積もる。
    /// **本物の子は1つも起きない。**
    ///
    /// **6回は同じ秒に収まることも、跨ぐこともある**——どちらでも
    /// 「最古の1組だけが消え、5 世代が残る」は変わらない（番号が秒の中の順を持つ）。
    #[test]
    fn launching_keeps_the_newest_five_generations_of_the_face() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        let plan = plan_of(
            r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:45103", "model": "m",
                                    "gguf": "/planted/a.gguf",
                                    "command": "personacore-no-such-command" },
                          "sub":  { "endpoint": "http://127.0.0.1:45104", "model": "n" } } }"#,
        );
        let launch = &plan.launches[0];
        // **他の面のログと、利用者のファイル**（どちらも残ること）。
        std::fs::write(dir.path().join("sub-20200101-000000.log"), "x").expect("置けること");
        std::fs::write(dir.path().join("notes.txt"), "x").expect("置けること");

        let mut first = None;
        for _ in 0..6 {
            let Err(err) = spawn_one(launch, dir.path()) else {
                panic!("在りえない command が起きている");
            };
            assert!(matches!(err, LaunchError::Spawn(_)), "{err:?}");
            let listed = listed(dir.path());
            if first.is_none() {
                first = listed
                    .iter()
                    .find(|name| name.starts_with("main-") && name.ends_with(".err.log"))
                    .cloned();
            }
        }

        let listed = listed(dir.path());
        let generations: std::collections::BTreeSet<LogGeneration> = listed
            .iter()
            .filter_map(|name| log_generation("main", name))
            .collect();
        assert_eq!(generations.len(), LOG_GENERATIONS, "{listed:?}");
        assert_eq!(
            listed
                .iter()
                .filter(|name| log_generation("main", name).is_some())
                .count(),
            LOG_GENERATIONS * 2,
            "1世代は2ファイル（片方だけ消す／残す実装）: {listed:?}"
        );
        let first = first.expect("1回目のログがあること");
        assert!(
            !listed.contains(&first),
            "最古の世代（{first}）が残っている: {listed:?}"
        );
        assert!(
            listed.contains(&"sub-20200101-000000.log".to_owned()),
            "他の面のログまで消している: {listed:?}"
        );
        assert!(
            listed.contains(&"notes.txt".to_owned()),
            "名付けていないファイルまで消している: {listed:?}"
        );
    }

    /// **消せない世代があっても、起こすほうを続ける。消せなかったことは言う**（§決定7）。
    ///
    /// **消せない形は「同じ名前のディレクトリ」で作る**（`remove_file` はディレクトリを消せない）
    /// ——**どの OS でも決定的に失敗する。** 実機で起きるのは「エディタが開いている」形。
    #[test]
    fn a_generation_that_cannot_be_removed_does_not_stop_the_launch() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作れること");
        // **最古の世代の片方を、消せない形で置く。**
        std::fs::create_dir(dir.path().join("main-20200101-000000.log")).expect("置けること");
        std::fs::write(dir.path().join("main-20200101-000000.err.log"), "x").expect("置けること");
        for stamp in [
            "20200101-000001",
            "20200101-000002",
            "20200101-000003",
            "20200101-000004",
        ] {
            for name in generation("main", stamp) {
                std::fs::write(dir.path().join(name), "x").expect("置けること");
            }
        }

        let mut prepared = None;
        let output = crate::testlog::captured_under_default_filter(|| {
            prepared = Some(prepare_logs(dir.path(), "main", "20260924-101112"));
        });
        let (out_name, err_name, _, _) = prepared
            .expect("呼ばれていること")
            .expect("消せない世代があっても、落とし先は用意すること");

        let listed = listed(dir.path());
        assert!(listed.contains(&out_name) && listed.contains(&err_name));
        // **消せたほうは消えている**（1つ失敗しても残りを諦めない）。
        assert!(
            !listed.contains(&"main-20200101-000000.err.log".to_owned()),
            "同じ世代の消せるほうまで残している: {listed:?}"
        );
        // **言う。** 名前は同じ行に載ること。
        let line = crate::testlog::line_containing(&output, "古いログを消せませんでした");
        assert!(
            line.contains("main-20200101-000000.log") && line.contains("WARN"),
            "消せなかったログの名前を WARN で言うこと: {line}"
        );
    }

    fn listed(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("読めること")
            .map(|entry| {
                entry
                    .expect("項目を読めること")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// **名指しするのは、そのポートを取りに行く面**（帰属）。
    ///
    /// **2つの面が別のポートに別の gguf を載せる形**で、
    /// **先頭の面を決め打って名乗る実装**を見分ける。
    #[test]
    fn the_conflict_names_the_face_that_wants_the_port() {
        let plan = plan_of(
            r#"{ "one": { "main": { "endpoint": "http://localhost:45102", "model": "m",
                                    "gguf": "/planted/a.gguf" },
                          "sub":  { "endpoint": "http://localhost:45101", "model": "n",
                                    "gguf": "/planted/b.gguf" } } }"#,
        );

        let rendered = plan
            .port_conflict(BindAddr::loopback(45_101))
            .expect("同じポートなら衝突として見つかること")
            .to_string();
        assert!(
            rendered.contains("sub") && rendered.contains("http://localhost:45101"),
            "そのポートを取りに行く面を名乗っていない: {rendered}"
        );
        assert!(
            !rendered.contains("45102"),
            "取り合っていない面まで挙げている: {rendered}"
        );
    }
}
