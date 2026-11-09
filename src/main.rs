//! personacore — ローカルLLMキャラクター会話基盤のオーケストレーション層。
//!
//! 起動の順序は
//! **引数（`--version` ならここで終わる）→ ログ初期化 → 上流プロファイル（無ければ見本を書く）
//! → 設定読み込み → 待ち受け → ブラウザを開く（`--no-ui` なら開かない）→ 停止待ち**。
//! 実体は `personacore::server` にあり、ここは結線だけを持つ。
//!
//! 起動失敗は `anyhow` で返す（CLAUDE.md 5節: `main` 付近のみ許容）。
//! `ServerError` / `ConfigError` の `Display` は定数文字列なので、
//! 詳細は `#[source]` 側（anyhow が `Caused by` として並べる）に出る。

use std::process::ExitCode;

use anyhow::Context;
use personacore::llm::endpoint_for_log;
use personacore::server::{
    App, AppConfig, Browser, ENV_UPSTREAMS, LaunchPlan, LaunchTiming, ProfileSettings,
    SampleOutcome, SavedPort, ServerError, Startup, Ui, UpstreamProfiles, VERSION_LINE,
    init_tracing, load_profiles, overlay_profile, parse_args, port_in_use_message, read_remembered,
    read_saved_port, remembered_path_beside, saved_port_path_beside, shutdown_signal,
    upstreams_path_under, wait_before_closing_if_alone, write_sample_if_missing,
};

/// **失敗を言ってから、ダブルクリックで開いた窓なら Enter を待って終わる**（作業単位 B-76）。
///
/// 書き出しは標準ライブラリの `main` が `Result` を返したときと同じ形（`Error: {:?}` ＋ 終了コード 1）。
/// **`eprintln!` を使わない**——標準エラーが閉じていると panic する（`--version` の doc と同じ）。
/// **待つのは失敗の枝だけ。** 正常に終わったとき（Ctrl+C）は待たない。
/// **実行時（tokio）を落とした後で待つ**——起こした子（`kill_on_drop`）を、待っている間に残さない。
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            use std::io::Write as _;
            let _ = writeln!(std::io::stderr().lock(), "Error: {err:?}");
            wait_before_closing_if_alone();
            ExitCode::FAILURE
        }
    }
}

/// **待ち受けを開けなかった理由が「他のプログラムが使っている」なら、番号と直し方を言う**
/// （作業単位 B-76 / marukei/personacore#13）。
///
/// **それ以外の失敗の文は変えない**（今までどおり「サーバの実行に失敗しました」）。
/// 見張り番は `tests/upstream_cli.rs` の `an_occupied_port_names_the_port_and_the_setting_and_never_waits`。
fn explain_server_failure(err: ServerError, port: u16) -> anyhow::Error {
    if err.is_port_in_use() {
        anyhow::Error::new(err).context(port_in_use_message(port))
    } else {
        anyhow::Error::new(err).context("サーバの実行に失敗しました")
    }
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
    // **引数はログより先に読む。** 綴り違いで止まる話に、起動ログは要らない。
    //
    // **`args_os` を使う**——`std::env::args()` は Unicode でない引数で **panic する**
    // （CLAUDE.md「本番コードパスに panic! を置かない」）。
    // **読めない綴りは「知らない引数」として止まる**のが正しい振る舞いで、
    // それは `to_string_lossy` を通した綴りでそのまま成立する。
    let args = parse_args(
        std::env::args_os()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned()),
    )
    .context("起動時の引数を読めませんでした")?;

    // **`--version` は版を言って終わる。他の旗が並んでいても、何もしない**（作業単位 F-4c）。
    //
    // **ログより先・ファイルより先。** `--version --fresh` で DB を退避しない、
    // 見本も書かない（見張り番は `tests/upstream_cli.rs` の
    // `the_version_flag_names_the_commit_and_does_nothing_else`）。
    //
    // **`println!` を使わない**——標準出力が閉じていると panic する
    // （`personacore --version | head -c 0` の形）。書けなければ、そう言って終わる。
    if args.version() {
        use std::io::Write as _;
        writeln!(std::io::stdout().lock(), "{VERSION_LINE}").context("版を書き出せませんでした")?;
        return Ok(());
    }

    init_tracing().context("ログ出力を初期化できませんでした")?;

    // **上流プロファイルの置き場所**（ADR-0042 §決定1）。
    let profiles_path = profiles_path();

    // **無ければ見本を書く。在れば1バイトも触らない**（作業単位 F-4c / Issue origin#23）。
    //
    // **`--upstream` を選ぶより先**——初めて叩いた起動で `--upstream <見本の名前>` を
    // 付けても、書いた見本から選べる（見本の gguf が無い機では、そこで gguf のパスを挙げて止まる）。
    let profile_sample = match &profiles_path {
        ProfilesPath::Default(path) => write_profile_sample(path),
        // **`PERSONACORE_UPSTREAMS` が指す場所には書かない。**
        // **指した先が無いのは綴り違いのことが多い**——黙って作ると、
        // 綴り違いの場所に見本が出来て、利用者は正しい場所を直したつもりになる。
        // **`--upstream` を渡せば、今までどおり置き場所を挙げて止まる。**
        ProfilesPath::Overridden(_) | ProfilesPath::Undecidable => None,
    };

    // **上流プロファイルを1つ選ぶ**（ADR-0042 §決定1・9 / 作業単位 F-4b-1・F-4f）。
    //
    // | 順 | どこから | 使えないとき |
    // |---|---|---|
    // | 1 | `--upstream <名前>`（明示） | **置き場所と在る名前を挙げて止まる**（今までどおり） |
    // | 2 | **覚えた名前**（`selected-upstream.txt`。**既定の置き場所のときだけ読む**） | **警告を1本出して 3 で起きる。覚えた名前は消さない** |
    // | 3 | 環境変数の既定 | 設定が読めなければ止まる（今までどおり） |
    //
    // **旗も覚えた名前も無ければ、`upstreams.json` を読まない**——上流の決め方は
    // F-4f より前と1バイトも変わらない。
    //
    // **プロファイルから来た値は、環境変数4つと同じ入口へ流す**（ADR-0042 §決定1）。
    // **入口を増やさない**——増やすと「どちらが効いているか」が2つになる。
    //
    // **画面で保存した待ち受けのポートも、同じ入口へ流す**（ADR-0057 §決定5 / 作業単位 F-12a）。
    // **読むのはここで1回だけ**——下の3つの経路（旗・覚えた名前・環境変数の既定）が同じ番号を見る。
    let saved_port = saved_port_of(&profiles_path);
    let chosen = match args.upstream() {
        Some(name) => {
            let selected = select_profile(name, &profiles_path)?;
            let config =
                config_for(&selected.settings, saved_port).context("設定を読み込めませんでした")?;
            warn_on_conflicts(&selected.settings, ProfileFrom::Flag);
            Chosen {
                selected: Some(selected),
                from: ProfileFrom::Flag,
                config,
            }
        }
        None => match remembered_choice(&profiles_path, saved_port) {
            Some(chosen) => chosen,
            None => Chosen {
                selected: None,
                from: ProfileFrom::Environment,
                config: AppConfig::from_env(saved_port).context("設定を読み込めませんでした")?,
            },
        },
    };
    let config = chosen.config;

    log_settings(
        &config,
        chosen.selected.as_ref().map(|s| s.name.as_str()),
        chosen.from,
    );

    // **`--fresh` は DB を開く前**（ADR-0041 §決定4）。
    //
    // **消さずに退避する。** 改名した先は `stash_database` が info で出す——
    // **手で戻せる。** ファイルが1つも無ければ何もしない（初回でも失敗しない）。
    if args.fresh() {
        let moved =
            personacore::server::stash_database(config.database_path(), chrono::Local::now())
                .context("初期化のためにデータベースを退避できませんでした")?;
        if moved.is_empty() {
            tracing::info!("退避するデータベースはありませんでした（まっさらから始めます）");
        }
    }

    // Ctrl+C（Windows ではコンソールの終了要求）まで応答し続ける。
    // 戻ってきた時点で `Store::close()` は済んでいる。
    //
    // **`bind` と `serve` を分けて書かない。** 分けると両者の間に失敗しうる処理を
    // 置けてしまい、そこで早期 return した瞬間に待ち受け済みの `App` を落としたまま
    // 抜ける（= `Store` を閉じない）。`App::run_with_upstreams` はその形を書けなくする。
    //
    // **起こした上流を止めるのも、その中**（ADR-0042 §決定2）。
    // **計画が空なら、`App::run` と1バイトも変わらない。**
    //
    // **見本を書いたこと・ブラウザを開くかも、ここで渡す**（作業単位 F-4c）。
    // **開くのは待ち受けが開いた後**（`App::run_with_upstreams` の doc）。
    // **`--upstream` で選んだ名前は、起こせたら覚える**（ADR-0042 §決定9）。
    // **覚える先を決めるのは `Startup` の置き場所**（`with_editable_profiles_file` のときだけ書く）。
    let remember = match (chosen.from, chosen.selected.as_ref()) {
        (ProfileFrom::Flag, Some(selected)) => Some(selected.name.clone()),
        _ => None,
    };
    // **覚えた名前で起きたときは、gguf の面を起こさない**（ADR-0055 §決定1・2 / 作業単位 F-10a）。
    //
    // **覚えた名前は「前回そう選んだ」という過去の意図**で、そのあいだに利用者が別の上流
    // （鍵で守られた中継の先の推論サーバ等）で GPU を使っていても personacore には分からない。**起こすのは、画面が聞いて
    // 同じ名前の `POST /admin/upstream/profile` が来たとき。** `--upstream` の明示は今までどおり起こす。
    // 見張り番は `tests/upstream_cli.rs` の `assert_waits_then_starts_on_the_same_name` を通る腕
    // （`--upstream` で起きる腕と対）。
    let timing = match chosen.from {
        ProfileFrom::Remembered => LaunchTiming::WhenAsked,
        ProfileFrom::Flag | ProfileFrom::Environment => LaunchTiming::AtStartup,
    };
    if timing == LaunchTiming::WhenAsked
        && let Some(selected) = chosen.selected.as_ref()
    {
        log_waiting_faces(selected);
    }
    let parts = StartupParts {
        browser: if args.no_browser() {
            Browser::Stay
        } else {
            Browser::Open
        },
        // **`--no-ui` はここで `Browser` を書き換えない**——「配らないなら開かない」は
        // `App::run_with_upstreams` が決める（旗の解釈を2箇所に割らない / 作業単位 P-1）。
        ui: if args.no_ui() {
            Ui::Withhold
        } else {
            Ui::Serve
        },
        profile_sample,
        profiles_path,
    };
    // **⚠ F-10a まで、ここに「覚えた名前のプロファイルを起こせなかったら環境変数の既定で起き直す」
    // 枝があった**（`ServerError::Launch` / ADR-0042 §決定9 / F-4f のレビュー1周目の是正1）。
    // **覚えた名前の起動は起こさなくなった**（`LaunchTiming::WhenAsked`）ので、`bind` は起こさず、
    // その枝へ来る道が無くなった。**起こせないこと（`command` が無い等）は、同じ名前で起こしたときに
    // その面の `failed` として言う**（`switch` / 見張り番は `tests/upstream_cli.rs` の
    // `a_remembered_profile_whose_command_is_missing_waits_and_fails_only_when_asked`）。
    //
    // **選んだプロファイルの名前は、旗でも覚えた名前でも渡す**（`GET /admin/upstream` の
    // `selected_profile` / 作業単位 F-10b）——画面の「起こす」はこの名前で同じ名前の
    // `POST /admin/upstream/profile` を送る。見張り番は `tests/upstream_cli.rs` の `assert_faces_on`。
    let (plan, selected_name) = match chosen.selected {
        Some(selected) => (selected.plan, Some(selected.name)),
        None => (LaunchPlan::none(), None),
    };
    let mut startup = parts.startup(plan, remember);
    if let Some(name) = selected_name {
        startup = startup.selecting(name);
    }
    App::run_with_upstreams(&config, startup.launch_timing(timing), shutdown_signal())
        .await
        .map_err(|err| explain_server_failure(err, config.bind().port()))?;

    tracing::info!("停止しました");
    Ok(())
}

/// **起動のたびに同じものを渡す部分**（ブラウザ・見本・置き場所）。
///
/// **F-10a まで、覚えた名前で起こせずにやり直すとき（ADR-0042 §決定9）も同じ値から `Startup` を組んでいた**
/// （やり直しの起動だけ差し替えや画面の保存ができない、という形を作らないため）。
/// **覚えた名前の起動は起こさなくなり（ADR-0055）、やり直しは無くなった。** 組むのは1回だけ。
struct StartupParts {
    browser: Browser,
    ui: Ui,
    profile_sample: Option<std::path::PathBuf>,
    profiles_path: ProfilesPath,
}

impl StartupParts {
    fn startup(&self, plan: LaunchPlan, remember: Option<String>) -> Startup {
        // **見本を書いたこと・ブラウザを開くかも、ここで渡す**（作業単位 F-4c）。
        // **開くのは待ち受けが開いた後**（`App::run_with_upstreams` の doc）。
        let mut startup = Startup::new(plan).browser(self.browser).ui(self.ui);
        if let Some(written_to) = &self.profile_sample {
            startup = startup.with_profile_sample(written_to.clone());
        }
        if let Some(name) = remember {
            startup = startup.remembering(name);
        }
        // **走ったまま差し替えるときに読む置き場所**（ADR-0042 §決定4 / 作業単位 F-4b-2）。
        // **`--upstream` の有無に関わらず渡す**——環境変数で起動してから、画面でプロファイルを選べる。
        // **決められない環境では渡さない**（差し替えは 409 で断られる）。
        //
        // **画面から書けるのは既定の置き場所だけ**（ADR-0042 §決定8 / 作業単位 F-4b-3）。
        // **`PERSONACORE_UPSTREAMS` が指す先は読むだけ**——`start.ps1 -Upstream` はそれを
        // リポジトリのファイル（コミットされた見本を含む）へ向ける。
        // 見張り番は `tests/upstream_cli.rs` の
        // `an_overridden_profile_file_is_read_only_and_neither_file_changes` と
        // `the_default_profile_file_is_editable_through_the_executable`。
        match &self.profiles_path {
            ProfilesPath::Overridden(path) => startup.with_profiles_file(path.clone()),
            ProfilesPath::Default(path) => startup.with_editable_profiles_file(path.clone()),
            ProfilesPath::Undecidable => startup,
        }
    }
}

/// **読み込んだ設定を1行で言う。**
///
/// **サブモデルの綴りも出す**（ADR-0021 §10）。`AppConfig` の検証は**書式しか見ない**ので、
/// LM Studio に存在しない綴りを `PERSONACORE_SUB_MODEL` に入れても起動は通り、
/// 現れ方は「本文は返るのに記憶がいつまでも増えない」になる——**いちばん原因から遠い形。**
/// 起動時に読める場所へ出しておけば、そこで綴りを見比べられる。
fn log_settings(config: &AppConfig, profile: Option<&str>, from: ProfileFrom) {
    let upstream = endpoint_for_log(config.llm().endpoint());
    let sub_upstream = endpoint_for_log(config.sub_llm_endpoint());
    tracing::info!(
        bind = %config.bind(),
        // **その番号が何から決まったか**（ADR-0057 §決定5）——`environment` / `saved` / `default`。
        // 見張り番は `tests/listening_port_cli.rs`。
        bind_from = %config.port_source().label(),
        database = ?config.database_path(),
        // **宛先はユーザー情報・クエリ・フラグメントを落として出す**（作業単位 B-71 / Issue origin#100 /
        // `llm::endpoint_for_log`）。見張り番は実行ファイルを起こす
        // `tests/upstream_cli.rs::the_secrets_written_in_an_endpoint_never_reach_the_log_or_the_upstream_report`。
        // **`&str` のまま渡す**（着手前と同じ、引用符つきの綴りで出す）。
        upstream = upstream.as_str(),
        // **抽出の上流も出す**（ADR-0039 §決定4 / `PERSONACORE_SUB_LLM_ENDPOINT`）。
        // 未設定なら本文と同じ綴りになる。
        //
        // **⚠ この行の宛先・モデルの欄の値に見張り番は無い**（`main` は bin なので統合テストから観測できない）。
        // **観測できる権威は `App::bind` が通る `bind_with` の起動ログ**——
        // `the_startup_line_names_the_extraction_upstream_apart_from_the_body_one`。
        sub_upstream = sub_upstream.as_str(),
        model = config.llm().model().as_str(),
        sub_model = config.sub_model().as_str(),
        // **どのプロファイルで起動したか**（ADR-0042 §決定1）。
        // **選んでいなければ `-`**——空文字にしない（欄が消えたのか未指定なのかが読めなくなる）。
        // **Debug で書く**（名前は入力から来る。改行で行を割らせない / レビュー1周目の是正3）。
        upstream_profile = ?profile.unwrap_or("-"),
        // **それが何から決まったか**（ADR-0042 §決定9）——`flag` / `remembered` / `-`（環境変数の既定）。
        // 見張り番は `tests/upstream_cli.rs` の F-4f の節（`assert_started_with`）。
        upstream_profile_from = %from.label(),
        "設定を読み込みました"
    );
}

/// **上流プロファイルの置き場所**（ADR-0042 §決定1）。**どこから決まったか**を持つ。
///
/// **見本を書いてよいのは [`Self::Default`] だけ**（`main` の doc / 作業単位 F-4c）。
enum ProfilesPath {
    /// `PERSONACORE_UPSTREAMS` が指した場所（**開発中の置き場所**）。
    Overridden(std::path::PathBuf),
    /// **DB と同じディレクトリの `upstreams.json`**（ADR-0041 §決定3 と1箇所に揃える）。
    Default(std::path::PathBuf),
    /// **OS のデータディレクトリが読めない。** `--upstream` を渡したときだけ止まる。
    Undecidable,
}

/// **置き場所を決める。** `PERSONACORE_UPSTREAMS`、無ければ OS のデータディレクトリの下。
fn profiles_path() -> ProfilesPath {
    match std::env::var(ENV_UPSTREAMS) {
        Ok(raw) if !raw.trim().is_empty() => {
            ProfilesPath::Overridden(std::path::PathBuf::from(raw.trim()))
        }
        _ => personacore::server::os_data_dir().map_or(ProfilesPath::Undecidable, |data_dir| {
            ProfilesPath::Default(upstreams_path_under(&data_dir))
        }),
    }
}

/// **無ければ見本を書き、書いたらその場所を返す**（作業単位 F-4c / Issue origin#23）。
///
/// **在れば何もしない（`None`）。書けなくても止めない**——見本が無くても、
/// 上流無しで待ち受けられる（キャラクターの作成は上流が無くても出来る）。
///
/// **書いたら、起動ログがその場所を言う**（画面は `GET /admin/upstream` の `profile_sample`）。
fn write_profile_sample(path: &std::path::Path) -> Option<std::path::PathBuf> {
    match write_sample_if_missing(path) {
        Ok(SampleOutcome::Wrote) => {
            tracing::info!(
                upstream_profiles_path = %path.display(),
                "上流プロファイルの見本を置きました。gguf のパスを自分の機のものに書き換えて、\
                 --upstream <名前> を付けて起動し直してください"
            );
            Some(path.to_path_buf())
        }
        Ok(SampleOutcome::AlreadyThere) => None,
        Err(err) => {
            // **パスはログにだけ出す**（エラー型の `Display` は定数文字列）。
            tracing::warn!(
                upstream_profiles_path = %path.display(),
                error = %err,
                cause = ?std::error::Error::source(&err),
                "上流プロファイルの見本を置けませんでした（上流無しで待ち受けます）"
            );
            None
        }
    }
}

/// **選んだプロファイルから取り出したもの。**
///
/// **綴り（4つの欄）と、起こすもの（gguf を書いた面）を別々に持つ。**
/// **同じ `UpstreamProfile` から両方を作る**ので、片方だけが別のプロファイルを
/// 指す配線が書けない。**名前**は起動ログと、覚える名前（ADR-0042 §決定9）に使う。
struct Selected {
    name: String,
    settings: ProfileSettings,
    plan: LaunchPlan,
}

/// **上流が何から決まったか**（ADR-0042 §決定9 の優先順位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileFrom {
    /// `--upstream <名前>`（明示）。
    Flag,
    /// 覚えた名前（`selected-upstream.txt`）。
    Remembered,
    /// 環境変数の既定（プロファイルを選んでいない）。
    Environment,
}

impl ProfileFrom {
    /// 起動ログの `upstream_profile_from` の綴り。
    const fn label(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Remembered => "remembered",
            Self::Environment => "-",
        }
    }
}

/// **選んだ結果と、それで組んだ設定。**
///
/// **設定を一緒に持つ**——覚えた名前は「設定まで組めたら採る」ので、組めなかったときに
/// 環境変数の既定へ落ちる判断が、設定を組む場所と離れない。
struct Chosen {
    selected: Option<Selected>,
    from: ProfileFrom,
    config: AppConfig,
}

/// **プロファイルの4つを環境変数に重ねて、設定を組む**（ADR-0042 §決定1）。
///
/// **面に書いたコンテキスト長（ADR-0054 §決定1）は別の入口で載せる**（`with_profile_context`）
/// ——`PERSONACORE_CONTEXT_TOKENS` に重ねると、走ったまま値の無いプロファイルへ切り替えたときに
/// 戻る先（起動時の環境変数の値）が消える。**ぶつかったときの `warn` は `App::bind` が出す**
/// （切り替えでも同じ行を出すので、言う場所を1つにしてある / `context::explicit_main_budget`）。
///
/// **面に書いた鍵（ADR-0056 §決定1）も別の入口で載せる**（`with_profile_keys`）——環境変数には
/// 鍵の口を作らない。**`warn_on_conflicts` の組にも鍵を混ぜない**（値を平文で並べる行なので）。
///
/// **画面で保存した待ち受けのポート（ADR-0057 §決定5）も、環境変数の既定の経路と同じ入口で渡す**
/// （`AppConfig::from_lookup_with_saved_port`。プロファイルは待ち受けのポートを持たない）。
fn config_for(
    settings: &ProfileSettings,
    saved_port: Option<SavedPort>,
) -> Result<AppConfig, personacore::server::ConfigError> {
    Ok(AppConfig::from_lookup_with_saved_port(
        overlay_profile(settings, |key| std::env::var(key).ok()),
        saved_port,
    )?
    .with_profile_context(settings)?
    .with_profile_keys(settings))
}

/// **画面で保存した待ち受けのポートを読む**（ADR-0057 §決定5 / 作業単位 F-12a）。
///
/// | 条件 | 結果 |
/// |---|---|
/// | 既定の置き場所でない（`PERSONACORE_UPSTREAMS` / 決められない） | **`None`。ファイルを読みもしない**（覚えた名前と同じ） |
/// | ファイルが無い・空 | `None` |
/// | 読めない（範囲の外・数でない・2行以上・ディレクトリ） | **警告を1本出して `None`。ファイルは消さない** |
///
/// **`PERSONACORE_PORT` があっても読む**（効く順は `config::resolve_port` が決める。壊れたファイルは知らせる）。
fn saved_port_of(profiles_path: &ProfilesPath) -> Option<SavedPort> {
    let ProfilesPath::Default(path) = profiles_path else {
        return None;
    };
    let file = saved_port_path_beside(path);
    match read_saved_port(&file) {
        Ok(saved) => saved,
        Err(err) => {
            tracing::warn!(
                saved_in = %file.display(),
                error = %err,
                cause = ?std::error::Error::source(&err),
                "{SAVED_PORT_UNREADABLE}"
            );
            None
        }
    }
}

/// **保存した待ち受けのポートを読めないときの警告の本文**（ADR-0057 §決定5）。
///
/// **写しが `tests/listening_port_cli.rs` の `SAVED_PORT_UNREADABLE_LINE` と
/// `SAVED_PORT_UNREADABLE_SCREEN` に在る**（文面を変えたら、そちらは「言っていない」で赤になる）。
///
/// **直し方の先頭は画面の「設定」**（作業単位 F-12b）——保存し直せば置き換わり、消せばファイルも消える。
/// 手で直す道も残して言う（画面を出さない `--no-ui` の起動がある）。
const SAVED_PORT_UNREADABLE: &str = "保存した待ち受けのポートを読めないので、使わずに起動します（ファイルは消していません。画面の「設定」で番号を保存し直すか保存を消すと直ります。手で直すなら、中身を 1024〜65535 の番号1行にするか、ファイルを消してください）";

/// **覚えた名前で起き、gguf の面を起こさずに待つと言う行の本文**（ADR-0055 / 作業単位 F-10a のレビュー是正）。
///
/// **写しが `tests/upstream_cli.rs` の `WAITING_TO_LAUNCH_LINE` に在る**
/// （文面を変えたら、そちらは「言っていない」で赤になる）。
const WAITING_TO_LAUNCH: &str = "覚えていた上流プロファイルで起動しました。gguf の面はまだ起こしていません（画面の「起こす」ボタンか、同じ名前の POST /admin/upstream/profile で起こします。起動と同時に起こすなら --upstream で選んでください）";

/// **覚えた名前で起きたとき、どの面を・どのプロファイルで起こさずに待っているかを1行で言う**
/// （ADR-0055 §決定1 / 作業単位 F-10a のレビュー是正）。
///
/// **gguf の面が無いプロファイルでは言わない**（待っているものが無い）。
/// **名前は Debug で書く**（入力から来る。改行で行を割らせない / `log_settings` と同じ）。
/// 見張り番は `tests/upstream_cli.rs` の `assert_waits_then_starts_on_the_same_name`（同じ行の中で見る）。
fn log_waiting_faces(selected: &Selected) {
    let waiting: Vec<&str> = personacore::measured::ModelFace::ALL
        .into_iter()
        .filter(|face| selected.plan.launches_face(*face))
        .map(personacore::measured::ModelFace::label)
        .collect();
    if waiting.is_empty() {
        return;
    }
    tracing::info!(
        upstream_profile = ?selected.name,
        upstream_faces = %waiting.join("+"),
        "{WAITING_TO_LAUNCH}"
    );
}

/// **覚えた名前が使えないときの警告の本文**（ADR-0042 §決定9）。
///
/// **写しが `tests/upstream_cli.rs` の `REMEMBERED_UNUSABLE_LINE` に在る**
/// （文面を変えたら、そちらは「言っていない」で赤になる）。
const REMEMBERED_UNUSABLE: &str = "覚えていた上流プロファイルを使えないので、環境変数の既定で起動します（覚えた名前は消していません。upstreams.json を直すか、--upstream で選び直してください）";

/// **覚えた名前で起きられるなら、その選択を返す**（ADR-0042 §決定9）。
///
/// **`None` は「環境変数の既定で起きる」。** 次のどれでも `None`:
///
/// | 条件 | 言うこと |
/// |---|---|
/// | 既定の置き場所でない（`PERSONACORE_UPSTREAMS` / 決められない） | **何も言わない。ファイルを読みもしない** |
/// | 覚えた名前が無い（ファイルが無い・空） | 何も言わない |
/// | 覚えた名前を読めない（ディレクトリ・権限・2行以上） | 警告 |
/// | **覚えた名前が使えない**（名前が無い／検証に通らない／gguf が無い／待ち受けと取り合う／設定を組めない） | **警告。同じ行が名前を言う** |
///
/// **どの場合も、覚えた名前は消さない**（利用者が `upstreams.json` を直せば、次から効く）。
fn remembered_choice(
    profiles_path: &ProfilesPath,
    saved_port: Option<SavedPort>,
) -> Option<Chosen> {
    let ProfilesPath::Default(path) = profiles_path else {
        return None;
    };
    let memory = remembered_path_beside(path);
    let name = match read_remembered(&memory) {
        Ok(Some(name)) => name,
        Ok(None) => return None,
        Err(err) => {
            tracing::warn!(
                remembered_in = %memory.display(),
                error = %err,
                cause = ?std::error::Error::source(&err),
                "覚えた上流プロファイルの名前を読めないので、環境変数の既定で起動します"
            );
            return None;
        }
    };
    match remembered_profile(&name, profiles_path, saved_port) {
        Ok((selected, config)) => {
            warn_on_conflicts(&selected.settings, ProfileFrom::Remembered);
            Some(Chosen {
                selected: Some(selected),
                from: ProfileFrom::Remembered,
                config,
            })
        }
        Err(err) => {
            // **パスはログにだけ出す**（ここはログ。応答には載らない）。
            //
            // **鎖は1本の文字列にしてから Debug で書く**（レビュー2周目の要修正）——鎖の中の
            // `upstreams::UnknownProfile` が利用者の鍵を生のまま並べる（改行入りの鍵で行が割れる）。
            // **`?err` にしない**: `anyhow::Error` の Debug は `Caused by:` を**本物の改行**で並べる。
            tracing::warn!(
                upstream_profile = ?name,
                remembered_in = %memory.display(),
                error = ?format!("{err:#}"),
                "{REMEMBERED_UNUSABLE}"
            );
            None
        }
    }
}

/// **覚えた名前のプロファイルを、起こす前に確かめられることを全部確かめてから採る。**
///
/// **明示の `--upstream` と同じ関数を通す**（[`select_profile`] と、`bind` が止まるのと同じ判定の
/// `LaunchPlan::check_before_starting`）——**判定の写しを作らない。**
///
/// **待ち受けとの取り合いは、保存した番号も見る**（`config.bind()` が保存した番号になる / ADR-0057 §決定5）。
/// 振る舞いは今までどおり（取り合えば覚えた名前を使わない）。番号を明示していないときに取り合わなくするのは
/// F-11（ADR-0057 §決定3）。
fn remembered_profile(
    name: &str,
    profiles_path: &ProfilesPath,
    saved_port: Option<SavedPort>,
) -> anyhow::Result<(Selected, AppConfig)> {
    let selected = select_profile(name, profiles_path)?;
    let config =
        config_for(&selected.settings, saved_port).context("設定を読み込めませんでした")?;
    selected
        .plan
        .check_before_starting(config.bind())
        .context("上流を起こせません")?;
    Ok((selected, config))
}

/// **プロファイルを読んで、1つ選ぶ**（ADR-0042 §決定1）。
///
/// 置き場所は `PERSONACORE_UPSTREAMS`、無ければ **DB と同じディレクトリの
/// `upstreams.json`**（ADR-0041 §決定3 と1箇所に揃える）。
///
/// **読めない／名前が無いときは、置き場所と在る名前を挙げて止まる。**
/// **黙って既定へ落ちない**——旗を渡した意味が消える（`F-9` と同じ決定）。
fn select_profile(name: &str, profiles_path: &ProfilesPath) -> anyhow::Result<Selected> {
    let path = match profiles_path {
        ProfilesPath::Overridden(path) | ProfilesPath::Default(path) => path.clone(),
        ProfilesPath::Undecidable => anyhow::bail!(
            "上流プロファイルの置き場所を決められません\
             （OS のデータディレクトリが読めないので PERSONACORE_UPSTREAMS を指定してください）"
        ),
    };

    let profiles: UpstreamProfiles = load_profiles(&path)
        .with_context(|| format!("上流プロファイルを読めませんでした（{}）", path.display()))?;
    let profile = profiles
        .select(name)
        .with_context(|| format!("上流プロファイルを選べませんでした（{}）", path.display()))?;

    tracing::info!(
        upstream_profile = ?name,
        upstream_profiles_path = %path.display(),
        upstream_launches = profile.launches().len(),
        "上流プロファイルを読みました"
    );

    Ok(Selected {
        name: name.to_owned(),
        settings: profile.settings(),
        plan: LaunchPlan::of(profile),
    })
}

/// **選んだプロファイルと環境変数がぶつかったら、そう言ってからプロファイルを優先する**（`F-9` と同じ）。
///
/// **黙って上書きしない。** 現れ方は「指定したはずの上流が効いていない」または
/// 「いつもと違う上流を向いている」で、**どちらも原因から遠い。**
/// **覚えた名前も同じ**（ADR-0042 §決定9 の優先順位は「覚えた名前 ＞ 環境変数の既定」）。
fn warn_on_conflicts(settings: &ProfileSettings, from: ProfileFrom) {
    // **3つ目の値は「宛先か」**（作業単位 B-71 / Issue origin#100）。宛先は両側とも
    // `endpoint_for_log` を通してから並べる（ユーザー情報・クエリ・フラグメントを落とす）。
    for (key, chosen, is_endpoint) in [
        (
            personacore::server::ENV_LLM_ENDPOINT,
            settings.endpoint().to_owned(),
            true,
        ),
        (
            personacore::server::ENV_MAIN_MODEL,
            settings.model().to_owned(),
            false,
        ),
        (
            personacore::server::ENV_SUB_LLM_ENDPOINT,
            settings.sub_endpoint().to_owned(),
            true,
        ),
        (
            personacore::server::ENV_SUB_MODEL,
            settings.sub_model().to_owned(),
            false,
        ),
    ] {
        let Ok(existing) = std::env::var(key) else {
            continue;
        };
        if existing.trim() == chosen.trim() {
            continue;
        }
        let shown = |value: &str| {
            if is_endpoint {
                endpoint_for_log(value)
            } else {
                value.to_owned()
            }
        };
        tracing::warn!(
            setting = key,
            from_environment = %shown(existing.trim()),
            from_profile = %shown(&chosen),
            upstream_profile_from = %from.label(),
            "環境変数よりも選んだ上流プロファイルを優先します"
        );
    }
}
