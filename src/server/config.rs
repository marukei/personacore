//! 起動設定。
//!
//! # どこから読むか — **環境変数**
//!
//! 依存クレートを増やさずに済み（`clap` も設定ファイル用のパーサも入っていない）、
//! 「ローカルで1プロセス動かす」用途に対して過不足がないため。
//! `PERSONACORE_PORT=9000 cargo run` のように起動ごとの上書きもできる。
//!
//! 環境そのものは読み取りを1箇所（`AppConfig::from_env`）に閉じ込め、組み立ては
//! **参照関数を受け取る `from_lookup`** に寄せてある。テストがプロセスの環境変数を
//! 書き換えずに済み（`std::env::set_var` は並行するテストを壊す）、設定項目が増えても
//! 検証の形が変わらない。
//!
//! # ⚠ **`from_lookup` は「純粋」ではなくなった**（ADR-0041 §決定3 / 作業単位 F-4a）
//!
//! **DB の既定が OS の作法の場所になった**ので、`from_lookup` は
//! **参照関数の外側にある1つのもの**——OS が言うデータディレクトリ——を引く。
//!
//! **純粋な本体は [`AppConfig::from_lookup_with_data_dir`]** で、
//! **場所を引数で受け取る。**`from_lookup` はそこへ [`os_data_dir`] を渡すだけ。
//! **この機の既定と違う場所で回す腕**（CLAUDE.md「実行時に決まる値を入れたら、
//! 機械の既定と違う状態で回る腕を必ず1本」）は
//! `the_database_falls_back_under_the_data_dir_it_is_given` が持つ。
//!
//! **参照関数の側で `LOCALAPPDATA` を引く形は採らなかった。**
//! それだと `from_lookup(|_| None)` が [`ConfigError::DataDir`] になり、
//! **既に在る 30 箇所の呼び出し（どれも直後に `with_database_path` する）が全部止まる。**
//! **止める意味のない所を止めることになるので、純粋さは本体の側に残した。**
//!
//! **⚠ 代わりに払ったもの: `from_lookup` を使うテスト（約 30 箇所）は、
//! この機に OS のデータディレクトリが在ることを前提にするようになった。**
//! **Windows なら `%LOCALAPPDATA%`、Mac / Linux なら `$HOME`（か `$XDG_DATA_HOME`）。**
//! **どれも無い環境では、`PERSONACORE_DB_PATH` を与えていない `from_lookup` が
//! [`ConfigError::DataDir`] を返して落ちる**——**テストの中身とは無関係に。**
//! **`from_lookup_with_data_dir` を使えば、その前提から外れられる。**
//!
//! # 待ち受けアドレスは**設定できない**
//!
//! 設定できるのは**ポート番号だけ**。`BindAddr` の唯一のコンストラクタが
//! `BindAddr::loopback` なので、**ループバック以外のアドレスは型として表現できない**
//! （ADR-0006 の `ValidTag` と同じ発想で、「気をつける」ではなく「書けなくする」）。
//!
//! # コンテキスト長は**明示設定だけ**を持つ
//!
//! `PERSONACORE_CONTEXT_TOKENS` は ADR-0013 第2層の優先順位
//! 「**明示設定 → 上流への照会 → 保守的な既定値**」の1段目にあたる。
//! ここが持つのは1段目だけで、**未設定は `None` のまま**にする
//! （既定値で埋めると 2 段目の照会が二度と行われない）。
//! 照会と既定値への退避は `super::context` が担う。

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::extract::SubContext;
use super::port_search::PortSearch;
use super::upstreams::ProfileSettings;
use crate::domain::{
    AxisDiscoveryMemories, AxisTrialRule, AxisTrialRuleError, DEFAULT_DRIFT_DECAY,
    DEFAULT_DRIFT_SLOPE, DEFAULT_DRIFT_STEP_COST, DriftForm, DriftNote, DriftTuning,
    DriftTuningError, StatDriftMode,
};
use crate::llm::{
    ApiKey, DEFAULT_ENDPOINT, DEFAULT_MAIN_MODEL, DEFAULT_SUB_MODEL, EndpointError, LmStudioConfig,
    ModelId, ModelIdError,
};
use crate::prompt::TokenBudget;

/// 待ち受けポートの環境変数名。
pub const ENV_PORT: &str = "PERSONACORE_PORT";

/// SQLite ファイルのパスの環境変数名。
pub const ENV_DATABASE_PATH: &str = "PERSONACORE_DB_PATH";

/// **UI（Svelte のビルド成果物）をディスクから配るときの配り先**の環境変数名
/// （[ADR-0031](../../docs/adr/0031-front-end-stack.md) §3 / 作業単位 D-0 /
/// [ADR-0041](../../docs/adr/0041-one-executable-per-os.md) §決定2 / 作業単位 F-4a）。
///
/// | 入力 | 結果 |
/// |---|---|
/// | 未設定 / 空 / 空白のみ | **実行ファイルに埋め込んだ画面を配る**（ADR-0041 §決定2） |
/// | それ以外 | そのパスを `ServeDir` で配る（**前後の空白は落とす**） |
///
/// # ⚠ **未設定は「既定のパス」ではない**（F-4a で変わった）
///
/// **D-0〜F-4 の間は、未設定が [`DEFAULT_WEB_DIR`]（`web/dist`）という
/// カレントディレクトリからの相対パスを意味していた。**
/// **いまの未設定は「埋め込み」である。**
/// [`DEFAULT_WEB_DIR`] は**埋め込み元の綴り**として残っているだけで、
/// 実行時にディスクを指す既定ではない。
///
/// **この旗の用途は開発**（`vite build --watch` の成果物を、
/// 実行ファイルを組み直さずに見る）。
///
/// # ⚠ **空を「未設定」と読む**——[`ENV_DATABASE_PATH`] とは逆
///
/// このファイルは**2つの作法を持っている。**
///
/// | 綴り | 空文字列 |
/// |---|---|
/// | [`ENV_DATABASE_PATH`] | [`ConfigError::DatabasePath`]（**起動を止める**） |
/// | [`ENV_STAT_DRIFT`] / [`ENV_DRIFT_FORM`] / [`ENV_DRIFT_NOTE`] / 集約のつまみ | **既定として読む** |
///
/// **ここは後者に揃えてある**（`the_web_dir_reads_an_empty_value_as_unset`）。
/// DB のパスを止めるのは「**書き込み先を取り違えると壊れるものがある**」からで、
/// 配り先は**読むだけ**である。空で止めると、
/// `PERSONACORE_WEB_DIR=` を置いた compose ファイルが**起動そのものを失敗させる**
/// ——UI が出ないだけの話に対して重すぎる。
///
/// # 存在は検証しない（**起動を止めない**）
///
/// `web/dist` は**ビルド成果物**で `.gitignore` に入る。存在を必須にすると
/// **API だけ使いたい運用が `pnpm build` を強制される。**
/// 代わりに**起動ログに配り先と実在の有無を必ず出す**（ADR-0031 §結果）。
pub const ENV_WEB_DIR: &str = "PERSONACORE_WEB_DIR";

/// **上流プロファイルの置き場所**の環境変数名（ADR-0042 §決定1 / 作業単位 F-4b-1）。
///
/// **未設定なら DB と同じディレクトリの `upstreams.json`**
/// （`server::upstreams::upstreams_path_under`）。
/// **開発中はリポジトリの `scripts/upstreams.json` を指す**（`scripts/start.ps1` が立てる）。
///
/// # ⚠ [`AppConfig`] はこの綴りを読まない
///
/// **読むのは `main` だけ**で、しかも **`--upstream <名前>` を渡した起動に限る。**
/// **渡していない起動では、この綴りが立っていても1バイトも効かない** ——
/// **旗を渡すこと自体が利用者の判断である**（`F-9` から引き継いだ決定）。
///
/// **それでもここに綴りを置くのは、`PERSONACORE_*` の綴りの帳簿がここ1つだから**
/// （`tests/start_script.rs::the_start_script_only_names_environment_variables_the_server_reads`
/// が、スクリプトの名指しをこのファイルの `pub const ENV_` と突き合わせる）。
pub const ENV_UPSTREAMS: &str = "PERSONACORE_UPSTREAMS";

/// 推論サーバのベース URL の環境変数名。
pub const ENV_LLM_ENDPOINT: &str = "PERSONACORE_LLM_ENDPOINT";

/// 本文生成モデルの識別子の環境変数名。
pub const ENV_MAIN_MODEL: &str = "PERSONACORE_MAIN_MODEL";

/// **裏方（記憶抽出）モデル**の識別子の環境変数名（ADR-0021 §10）。
///
/// 既定は `DEFAULT_SUB_MODEL` で、**今は本文の既定と同じ綴り**（2026-08-21 の実測で
/// 「抽出は 3B 級で足りる」を撤回した）。**それでも本文とは別の設定項目のまま**——
/// 小型モデルへ振り分けて試す経路を閉じないため、ここは独立して読む。
pub const ENV_SUB_MODEL: &str = "PERSONACORE_SUB_MODEL";

/// **裏方（記憶抽出）が話す上流のベース URL** の環境変数名
/// （[ADR-0039](../../docs/adr/0039-two-upstreams-for-two-jobs.md) §決定1 / 作業単位 G-3a）。
///
/// | 入力 | 結果 |
/// |---|---|
/// | 未設定 / 空 / 空白のみ | **本文と同じ上流**（[`AppConfig::sub_llm`] が `None`） |
/// | `http://127.0.0.1:1235` | **その上流**（本文とは別のクライアントを組む） |
/// | 綴りが URL として読めない | [`ConfigError::Endpoint`]（**起動を止める**） |
///
/// # なぜ2つ目を持てるようにしたのか（ADR-0039 §背景）
///
/// **`C-12`（2026-09-07）で、抽出だけを別モデルにすると
/// 「持ち主の取り違え」が 64% → 8% になった**（Fisher `p = 4.4e-9`）。
/// **`llama serve` は 1 プロセス 1 モデルで `model` の欄を見ない**ので、
/// **その配置は「モデルを2つ」では表現できず、「上流を2つ」でしか表現できない。**
///
/// **[`ENV_SUB_MODEL`] とは別のつまみ。** 「どのモデルに頼むか」と
/// 「どのプロセスに頼むか」は直交していて、**上流を共有したままモデルだけ替える形**
/// （LM Studio の JIT ロード）は今も一級の配置である。
///
/// # ⚠ **空を「未設定」と読む**——[`ENV_DATABASE_PATH`] とは逆
///
/// [`ENV_WEB_DIR`] と同じ側に揃えてある（このファイルは2つの作法を持っている /
/// 見張り番は `an_empty_sub_llm_endpoint_reads_as_unset`）。
/// DB のパスを止めるのは「**空だと壊れる**」からで、
/// **ここは「空＝本文と同じ」に落ちても壊れない。**
///
/// > **⚠ `PERSONACORE_DRIFT_FORM` の教訓**（[`DRIFT_FORM_SINGLE`] の doc）——
/// > **既定を動かすときは、空文字列の行き先も一緒に動かすこと。**
/// > いまは既定が「未設定＝本文と同じ」なので、空文字列もそこへ落ちるのが正しい。
/// > **既定を「別の上流」に変える日が来たら、空文字列もそちらへ移すこと。**
///
/// # ⚠ **待ち時間のつまみは増やさない**（ADR-0039 §決定2）
///
/// [`ENV_LLM_IDLE_TIMEOUT`] は**1つのまま、両方の上流に同じ値を使う。**
/// **抽出側は本文より遅い上流を指しうる**（MoE を CPU に逃がす配置なら、なおさら）が、
/// それでも増やさない——理由は [`ENV_LLM_IDLE_TIMEOUT`] の doc
/// （**つまみ1つにつき `App::bind` を通る見張り番が1本要る**）。
/// **痛みが出てから測って決める。**
/// 見張り番は `the_second_upstream_shares_the_one_idle_timeout_knob`。
///
/// # **`App::bind` を通る見張り番**（つまみ1つにつき1本 / CLAUDE.md）
///
/// **`tests/server_sub_llm_endpoint.rs`**——**本文用と抽出用に別のモックサーバ**を立て、
/// **どちらの往復がどちらへ行くか**を見る。**同値にしないこと**
/// （同値だと、この設定を丸ごと無視する実装と区別が付かない）。
pub const ENV_SUB_LLM_ENDPOINT: &str = "PERSONACORE_SUB_LLM_ENDPOINT";

/// 上流のコンテキスト長（トークン数）の環境変数名。
///
/// **指定すれば上流への照会より優先される**（ADR-0013 第2層）。
/// 未指定なら照会し、それも駄目なら既定値へ落ちる。
pub const ENV_CONTEXT_TOKENS: &str = "PERSONACORE_CONTEXT_TOKENS";

/// **上流の応答を打ち切る天井（秒）**の環境変数名
/// （[ADR-0035](../../docs/adr/0035-the-watchdog-reports-the-human-decides.md) §決定3 /
/// 作業単位 B-6）。**既定は `llm::lmstudio` の 180 秒。**
///
/// | 入力 | 結果 |
/// |---|---|
/// | 未設定 / 空 / 空白のみ | **既定**（`PERSONACORE_DRIFT_*` と揃える） |
/// | `45` / ` 45 ` | その秒数（前後の空白は落とす） |
/// | `0` / `-1` / `1.5` / `abc` | [`ConfigError::LlmIdleTimeout`]（**起動を止める**） |
///
/// # なぜ**これだけ**がつまみなのか（ADR-0036 §決定3）
///
/// 設定可能にしてよいのは次の3つが揃った値だけである:
///
/// 1. **根拠となる計測が無い** —— `llm/lmstudio.rs` の doc が
///    「180秒は暫定値。リポジトリ内に根拠となるロード時間の計測値は無い」と明言している。
/// 2. **外すと破壊的** —— 天井に当たった回は**1行も保存されない**
///    （[ADR-0019](../../docs/adr/0019-relay-idle-watchdog.md) §4）。
/// 3. **こちらから導出も観測もできない** —— **外から「遅い」と「死んだ」は
///    原理的に区別できない**（ADR-0035 §背景）。
///
/// # ⚠ **中継の見張り（30秒）はつまみにしない**（ADR-0035 §決定5）
///
/// あちらは B-6 で「打ち切る」から「言う」へ移ったので、
/// **外しても起きるのは余計なログが出るか出ないかだけ。** 上の 2. を満たさない。
///
/// # ⚠ **秒を2つ受け取らない**
///
/// 全体上限（`request_timeout`）・照会の上限・接続の上限は
/// **この1つから導く**（`LmStudioConfig::with_idle_budget`）。
/// **つまみ1つにつき `App::bind` を通る見張り番が1本要る**ので、
/// 2つ目を足すと費用も2倍になる。
/// **この1つの見張り番は `tests/server_llm_idle_timeout.rs`。**
pub const ENV_LLM_IDLE_TIMEOUT: &str = "PERSONACORE_LLM_IDLE_TIMEOUT";

/// **性格ステータスの変容**（[ADR-0024](../../docs/adr/0024-stat-drift.md)）の
/// 切り替えスイッチの環境変数名。**既定 off。**
///
/// # ⚠ **キャラクターごとの値が勝つ**（[ADR-0045](../../docs/adr/0045-drift-mode-per-character.md) / 作業単位 A-8）
///
/// **この値は「起動時の既定」である。** キャラクターが `stat_drift_mode` を持っていれば
/// そちらが効き、**未設定（NULL）のキャラクターだけがこの値に従う**
/// （決めるのは `domain::ResolvedDriftMode::resolve`）。
/// **測定を既存のキャラクターで回すなら、先にその値を未設定へ戻すこと**（ADR-0045 §結果の ⚠）。
///
/// **別名（`1` / `true` / `yes` …）を受けるのはこの環境変数だけ**で、
/// キャラクターに書く綴りは `off` / `shadow` / `on` の3つに閉じている（`StatDriftMode::parse`）。
///
/// # **3値**（[ADR-0026](../../docs/adr/0026-drift-by-accumulation.md) §決定1）
///
/// | 綴り | 意味 |
/// |---|---|
/// | 未設定 / `off` / `0` / `false` / `no` / 空 | **既定。** 判定そのものを走らせない |
/// | `shadow` | **記録専用。** 判定して票を残すが、**値は動かさない** |
/// | `on` / `1` / `true` / `yes` | 判定し、票を残し、**値も動かす** |
///
/// 読んだ結果は `crate::domain::StatDriftMode`。
/// **`shadow` では決定性が保たれる**ので、同じ会話を2回投げて比べられる
/// （下の ⚠ が消えるのは、このモードだけ）。
///
/// # なぜ既定を off にするのか（ADR-0024 §8）
///
/// **これは実験の切り替えスイッチである。** 既定を on にすると、
/// **抽出の質を測っている最中に性格が動いてしまい、どちらの効果かを切り分けられない。**
/// docs/05-development.md「属性を実験の切り替えスイッチとして使う」と同じ立場。
///
/// **決定性が崩れる原因の2つ目でもある。** `temperature = 0` でも本文が再現しない原因は
/// 「記憶が増えること」だと切り分け済み（ADR-0021 / 2026-08-21）。
/// on にすると**性格の値もターンごとに動く**ので、
/// **同じ会話を2回投げて比べる測定は off でしか成立しない。**
pub const ENV_STAT_DRIFT: &str = "PERSONACORE_STAT_DRIFT";

/// **変容の依頼の形**（[ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md) §12）の
/// 切り替えスイッチの環境変数名。**既定は `all`**（作業単位 19 / ADR-0027 §決定8）。
///
/// | 綴り | 意味 |
/// |---|---|
/// | 未設定 / 空 / `all` | **既定。** 全項目を**1つずつ採点**させる（`0` を書ける） |
/// | `single` | 動いた項目を**1つだけ**尋ねる（ADR-0024 §3） |
///
/// 読んだ結果は `crate::domain::DriftForm`。
///
/// # なぜ切り替えるのか（ADR-0027 §12）
///
/// **同じ台本で両方を測るため。** 3本の腕（モデルを替える / 軸を替える）が
/// **どちらも「なし」ゼロ・上げ偏り8割・噛み合い3割台**を残した。
/// **残ったのは依頼の形そのもの**で、それを変える実験がこのスイッチだった。
///
/// # ⚠ **既定は 2026-08-25 に `all` へ移した**（作業単位 19 / ADR-0027 §決定8）
///
/// **測って決まった**——`single` は**上げ偏り 94% で分化しない**（16i）。
/// `single` は**測定と比べるために残してある形**であって、明示したときだけ効く。
///
/// # [`ENV_STAT_DRIFT`] とは別のスイッチ
///
/// 「走らせるか」と「どう尋ねるか」は直交している。
/// **`off` のときこの値は使われない**が、**1つの語彙に混ぜない**——
/// 混ぜると `off` / `shadow` / `on` × `single` / `all` の6通りを
/// 1つの綴りで表すことになり、**測定のログに何が効いたのかが書けなくなる。**
pub const ENV_DRIFT_FORM: &str = "PERSONACORE_DRIFT_FORM";

/// **変容の依頼に足す「書き方の注記」**
/// （[ADR-0029](../../docs/adr/0029-language-as-a-matched-bundle.md) §訂正 / 作業単位 16m）の
/// 切り替えスイッチの環境変数名。**既定は `none`。**
///
/// | 綴り | 意味 |
/// |---|---|
/// | 未設定 / `none` / 空 | **既定（腕 X）。** 何も足さない。**依頼は1バイトも変わらない** |
/// | `copy` | **腕 Y。** 軸名は上に書かれている字をそのまま写す、と足す |
/// | `language` | **腕 Z。** 日本語（常用漢字）で書く、と足す |
///
/// 読んだ結果は `crate::domain::DriftNote`。
///
/// # ⚠ **これは機能の設定ではなく、測定の器具である**
///
/// ADR-0029 §決定5 は**保留**——「言語の指示は効かない」という推論を支える測定が
/// 1つも無かった（§訂正）。**732 ターンの実測は推論と逆を指した**
/// ——本文と抽出は `阳` が 0 件、**変容の採点だけが 18 件**で、
/// **崩れているのは日本語の文脈がいちばん薄いプロンプト**である。
///
/// **測って決まったら、この設定は「片方に固定して消す」か
/// 「正式な決定として残す」かのどちらか**になる。
///
/// # [`ENV_DRIFT_FORM`] とは別のスイッチ
///
/// 「どう尋ねるか」と「どう書けと言うか」は直交している。
/// **1つの語彙に混ぜない**——混ぜると `single` / `all` × `none` / `copy` / `language` の
/// 6通りを1つの綴りで表すことになり、**測定のログに何が効いたのかが書けなくなる**
/// （[`ENV_DRIFT_FORM`] が [`ENV_STAT_DRIFT`] と分かれているのと同じ理由）。
///
/// # 自由文を受けない
///
/// 受けるのは**3つの綴りだけ**。ADR-0029 §決定4 と却下欄のとおり、
/// **依頼の文面は測って決めた結果物**であり、
/// **測っていない文面に黙って置き換わる経路を作らない。**
pub const ENV_DRIFT_NOTE: &str = "PERSONACORE_DRIFT_NOTE";

/// **変容の減衰の強さ `λ`**（[ADR-0027](../../docs/adr/0027-drift-from-what-was-not-chosen.md)
/// §決定3）の環境変数名。**既定は [`crate::domain::DEFAULT_DRIFT_DECAY`]。**
///
/// **⚠ ここに値を写さない。** **2026-09-07 まで「既定は `1`」と書いてあり、実際は `0.5` だった**
/// （傾きと `K` も同じく違っていた）。**写した数は、変えた側からは見えない。**
///
/// 「`0` と採点された軸／選ばれなかった軸」が失う量の係数で、
/// **下向きの唯一の供給源**（§10 の表）。`0` にすると下向きが完全に消える。
///
/// # なぜ設定で振れる形にするのか（§決定8 の ⚠）
///
/// ADR は当面の値を「**決めた**ではなく**出発点**」と書いている。
/// **判定の噛み合いが 32〜36% の入力で釣り合わせた値**でしかないので、
/// 入力の質が変われば釣り合いも動く。
pub const ENV_DRIFT_DECAY: &str = "PERSONACORE_DRIFT_DECAY";

/// **変容の値付けの傾き**（ADR-0027 §9）の環境変数名。**既定は [`crate::domain::DEFAULT_DRIFT_SLOPE`]。**
///
/// `0` なら値付けは効かず、費用はどの値でも [`ENV_DRIFT_STEP_COST`] のまま。
/// 上限は `crate::domain::MAX_DRIFT_SLOPE`（超えると費用が負になる）。
pub const ENV_DRIFT_SLOPE: &str = "PERSONACORE_DRIFT_SLOPE";

/// **変容の1段の費用 `K`**（ADR-0027 §決定7）の環境変数名。**既定は [`crate::domain::DEFAULT_DRIFT_STEP_COST`]。**
///
/// **「なし」を出すのはモデルではなく、この閾値である**（§決定7）。
/// 大きくするほど静かになり、**同時に下向きが消える**（§7 の ⚠「同じつまみの両端」）。
pub const ENV_DRIFT_STEP_COST: &str = "PERSONACORE_DRIFT_STEP_COST";

/// **軸の候補を尋ねる間隔 `N`**（[ADR-0046](../../docs/adr/0046-drift-proposes-new-axes.md)
/// §決定2 / 作業単位 A-9b）の環境変数名。**既定は [`AxisDiscoveryMemories::DEFAULT`]。**
///
/// **前回の試みから、そのキャラクター自身の記憶（`subject = character`）がこの件数だけ
/// 増えたら、**空いた枠があるときに**裏方が候補を1つ尋ねる**（`server::axis_discovery`）。
///
/// | 入力 | 結果 |
/// |---|---|
/// | 未設定 / 空 / 空白のみ | **既定**（`PERSONACORE_DRIFT_*` と揃える） |
/// | `3` / ` 3 ` | その件数（前後の空白は落とす） |
/// | `0` / `-1` / `1.5` / `abc` | [`ConfigError::AxisDiscoveryMemories`]（**起動を止める**） |
///
/// # ⚠ **既定は測っていない値**（ADR-0046「まだ決めていないこと」）
///
/// **ここに数を写さない**（[`ENV_DRIFT_DECAY`] の doc の教訓）。権威は
/// `domain::AxisDiscoveryMemories::DEFAULT` で、そこに「測っていない」と書いてある。
///
/// # これは機能の設定ではなく、**測定の器具**である
///
/// ADR-0046 §測る問い（「候補の生成器は、合格する軸を、割に合う率で出すか」）を回すために
/// 振れる形にしてある（[`ENV_DRIFT_NOTE`] と同じ立場）。
/// **外したときに起きるのは「尋ねる回数が増える／減る」だけで、壊れはしない**
/// （CLAUDE.md の判定の問い「遅くなるだけか、壊れるか」の前者）。
/// **だから CLAUDE.md の「つまみにしてよい3条件」は満たしていない** —— 測って既定が
/// 決まったら、片方に固定して消すか、正式な決定として残すかのどちらかにする。
///
/// **`0` を弾く**のは、0 が「増えていなくても尋ねる」＝毎ターン尋ねることになり、
/// ADR-0028 §決定2（「なし」を言わないモデルに毎ターン尋ねない）を設定1つで裏返せるから。
///
/// **本番の入口を通る見張り番は `tests/server_axis_discovery.rs` の
/// `the_axis_discovery_interval_reaches_the_background_through_the_real_bind`**
/// （期待値は既定ではない値。既定の腕と並べてある）。
pub const ENV_AXIS_DISCOVERY_MEMORIES: &str = "PERSONACORE_AXIS_DISCOVERY_MEMORIES";

/// **軸の候補の試用のターン数 `T`**（[ADR-0046](../../docs/adr/0046-drift-proposes-new-axes.md)
/// §決定4 / 作業単位 A-9c）の環境変数名。**既定は [`AxisTrialRule::DEFAULT_TURNS`]。**
///
/// **試用中の候補の票がこの件数そろったら判定する**（`server::axis_trial`）。
///
/// | 入力 | 結果 |
/// |---|---|
/// | 未設定 / 空 / 空白のみ | **既定**（[`ENV_AXIS_DISCOVERY_MEMORIES`] と揃える） |
/// | `30` / ` 30 ` | その件数（前後の空白は落とす） |
/// | `0` / `-1` / `1.5` / `abc` | [`ConfigError::AxisTrial`]（**起動を止める**） |
///
/// # ⚠ **既定は測っていない値**（ADR-0046 §決定4 の注記 /「まだ決めていないこと」）
///
/// **ここに数を写さない**（[`ENV_DRIFT_DECAY`] の doc の教訓）。権威は `domain::AxisTrialRule` で、
/// そこに「ADR-0030 の手順の値で、候補の分布では測っていない」と書いてある。
///
/// # 3つとも**機能の設定ではなく、測定の器具**である
///
/// [`ENV_AXIS_TRIAL_TURNS`] / [`ENV_AXIS_TRIAL_MIN_NONZERO`] / [`ENV_AXIS_TRIAL_LINE_PERCENT`] は、
/// ADR-0046 §測る問い（合格率と不合格の内訳）を回すために振れる形にしてある
/// （[`ENV_AXIS_DISCOVERY_MEMORIES`] と同じ立場）。**外したときに起きるのは「合格・不合格の振り分けが
/// 変わる」だけで、壊れはしない**（合格しても採用は利用者だけが行う / §決定5）。
/// **だから CLAUDE.md の「つまみにしてよい3条件」は満たしていない** —— 測って既定が
/// 決まったら、片方に固定して消すか、正式な決定として残すかのどちらかにする。
///
/// **3つは組で検証する**（`AxisTrialRule::new`）—— `K` が `T` を超える組は、読めても起動を止める。
///
/// **本番の入口を通る見張り番は `tests/server_axis_trial.rs` の
/// `the_axis_trial_rule_reaches_the_background_through_the_real_bind`**
/// （3つとも既定ではない値の腕と、既定の腕を並べてある）。
pub const ENV_AXIS_TRIAL_TURNS: &str = "PERSONACORE_AXIS_TRIAL_TURNS";

/// **軸の候補の試用の標本不足の線 `K`**（ADR-0046 §決定4 / 作業単位 A-9c）の環境変数名。
/// **既定は [`AxisTrialRule::DEFAULT_MIN_NONZERO`]（根拠の無い値）。**
///
/// 判定のとき、**非ゼロの票（上げ＋下げ）がこの件数に届かなければ不合格（標本不足）**。
/// 入力の読み方は [`ENV_AXIS_TRIAL_TURNS`] と同じ（`0` は通る。**`T` を超えると起動を止める**）。
pub const ENV_AXIS_TRIAL_MIN_NONZERO: &str = "PERSONACORE_AXIS_TRIAL_MIN_NONZERO";

/// **軸の候補の試用の判定の線（百分率）**（ADR-0046 §決定4 / 作業単位 A-9c）の環境変数名。
/// **既定は [`AxisTrialRule::DEFAULT_LINE_PERCENT`]。**
///
/// **上げの割合（非ゼロのうち）がこれを超えたら張り付き、`0` の率がこれを超えたら沈黙。**
/// **ちょうど線は合格の側**（`domain::axis_trial` の doc）。受け付けるのは `1`〜`99`
/// （`0` と `100` は判定を捨てるのと同じなので起動を止める）。入力の読み方は [`ENV_AXIS_TRIAL_TURNS`] と同じ。
pub const ENV_AXIS_TRIAL_LINE_PERCENT: &str = "PERSONACORE_AXIS_TRIAL_LINE_PERCENT";

/// 既定の待ち受けポート。
pub const DEFAULT_PORT: u16 = 8080;

/// **待ち受けのポートを他のプログラムが使っていたときに、利用者へ言う文**
/// （作業単位 B-76 / marukei/personacore#13）。
///
/// **ポートの番号と、[`ENV_PORT`] で変えられることを言う。** OS の文（`os error 10048` 等）だけでは、
/// 何が何と重なったのかも、どう直すのかも分からない。
///
/// **エラー型の `Display` ではない**——`main` が `anyhow` の文脈として足す
/// （`ServerError` の `Display` は定数文字列のまま / `ServerError::is_port_in_use`）。
/// 配布物の README がこの文を [`DEFAULT_PORT`] で写している（`tests/distribution.rs` が突き合わせる）。
pub fn port_in_use_message(port: u16) -> String {
    format!(
        "ポート {port} は他のプログラムが使っています。\
         環境変数 {ENV_PORT} で別のポートを指定して起動し直してください"
    )
}

/// **番号を明示していない起動が、ずらす範囲を全部試しても待ち受けを開けなかったときに、利用者へ言う文**
/// （[ADR-0057](../../docs/adr/0057-the-listening-port-shifts-when-taken-and-is-set-from-the-screen.md)
/// §決定3「B-76 の文を、試した範囲を言う形に広げる」/ 作業単位 F-11b）。
///
/// **[`port_in_use_message`]（番号を明示したとき）とは別の文**——明示のときの文は今までどおり。
///
/// # 直し方は2つ言う。先に [`ENV_PORT`]、次に画面の「設定」
///
/// * **この起動では画面を開けない**（待ち受けていない）。だから先に言うのは、いま打てる手の [`ENV_PORT`]。
/// * **毎回 [`ENV_PORT`] を打たなくて済むように**、起動できた後に画面の「設定」で番号を保存すれば、
///   次からはそこから試すことを添える（ADR-0057 の背景 —— 配布物の利用者に環境変数を打たせ続けない）。
///
/// # 何を言うかは、範囲の中で飛ばした番号（`skipped`）で決める（F-11b のレビュー是正）
///
/// | 範囲の中 | 文の頭 |
/// |---|---|
/// | 飛ばした番号が無い | 「ポート {範囲} は、どれも他のプログラムが使っているか、使えない番号でした」 |
/// | 飛ばした番号が在り、試した番号も在る | 「…は、上流プロファイルの llama の面が使う番号（{飛ばした番号}）を除いて、どれも…」 |
/// | **全部を飛ばした**（1つも試していない） | 「…は、どれも上流プロファイルの llama の面が使う番号なので、試せる番号がありませんでした」 |
///
/// **飛ばしていないのに「面の番号は試していない」と言わない**——`upstreams.json` が読めず、飛ばす番号が空の
/// 起動がある。**1つも試していないのに「他のプログラムが使っている」と言わない**——保存した番号が 65535 で、
/// それが面の番号なら、列は1つで、それも飛ばす。
///
/// 範囲が1つの番号しか無い（元の番号が 65535）ときは「〜」を書かず、試した番号が1つなら「どれも」を書かない。
/// **写しが `tests/listening_port_cli.rs` の `a_whole_range_taken_names_the_range_and_how_to_get_out` に在る。**
pub fn ports_in_use_message(first: u16, last: u16, skipped: &[u16]) -> String {
    let range = if first == last {
        first.to_string()
    } else {
        format!("{first}〜{last}")
    };
    let total = usize::from(last.saturating_sub(first)) + 1;
    let tried = total.saturating_sub(skipped.len());
    let head = if tried == 0 {
        let every = if total > 1 { "どれも" } else { "" };
        format!(
            "ポート {range} は、{every}上流プロファイルの llama の面が使う番号なので、\
             試せる番号がありませんでした"
        )
    } else {
        let every = if tried > 1 { "どれも" } else { "" };
        if skipped.is_empty() {
            format!("ポート {range} は、{every}他のプログラムが使っているか、使えない番号でした")
        } else {
            let skipped = skipped
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("・");
            format!(
                "ポート {range} は、上流プロファイルの llama の面が使う番号（{skipped}）を除いて、\
                 {every}他のプログラムが使っているか、使えない番号でした"
            )
        }
    };
    format!(
        "{head}。環境変数 {ENV_PORT} で空いているポートを指定して起動し直してください\
         （起動できたら、画面の「設定」でその番号を保存すると、次からは指定しなくてもその番号から試します）"
    )
}

/// **画面で保存できる待ち受けのポートの下端**（[ADR-0057](../../docs/adr/0057-the-listening-port-shifts-when-taken-and-is-set-from-the-screen.md) §決定5）。
///
/// **上端は `u16::MAX`（65535）。** `0`（OS に選ばせる）と特権ポートは保存できない ——
/// `0` は開発とテストのための値で、起動のたびに番号が変わる。
pub const SAVED_PORT_MIN: u16 = 1024;

/// **画面で保存した待ち受けのポート**（1024〜65535 / ADR-0057 §決定5）。
///
/// **範囲の外の値は作れない**（唯一の構築子が [`Self::new`]）。保存のファイルの読み
/// （`listening_port::parse_saved_port`）も、保存の口（`PUT /admin/server/port`）も、ここを通る。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SavedPort(u16);

impl SavedPort {
    /// **範囲（[`SAVED_PORT_MIN`]〜65535）の中なら作る。** 外なら `None`。
    #[must_use]
    pub const fn new(port: u16) -> Option<Self> {
        if port >= SAVED_PORT_MIN {
            Some(Self(port))
        } else {
            None
        }
    }

    /// **整数から作る**（保存の口の本文は範囲の外の値も負の値も受けてから断る）。**純粋関数。**
    #[must_use]
    pub fn from_integer(value: i64) -> Option<Self> {
        u16::try_from(value).ok().and_then(Self::new)
    }

    /// 番号。
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// **待ち受けのポートが何から決まったか**（ADR-0057 §決定1・5）。
///
/// 効く順は [`ENV_PORT`] → 保存した番号 → [`DEFAULT_PORT`]（[`resolve_port`]）。
/// **「明示か」（塞がっていたらずらすか / ADR-0057 §決定1・作業単位 F-11b の `port_search`）を見分けるのも、
/// `GET /admin/server` が言うのもこれ。** ずらすのは [`Self::Saved`] と [`Self::Default`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortSource {
    /// [`ENV_PORT`]（明示）。
    Environment,
    /// 画面で保存した番号（`listening-port.txt`）。
    Saved,
    /// [`DEFAULT_PORT`]。
    Default,
}

impl PortSource {
    /// **全部の値**（仕様書の `enum` を作る / 網羅は `label` の `match` が強制する）。
    pub const ALL: [Self; 3] = [Self::Environment, Self::Saved, Self::Default];

    /// **線の上の綴り**（`GET /admin/server` の欄と、起動ログの `bind_from`）。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::Saved => "saved",
            Self::Default => "default",
        }
    }
}

/// **待ち受けのポートを効く順で決める。純粋関数**（ADR-0057 §決定5）。
///
/// | 順 | どこから | 出どころ |
/// |---|---|---|
/// | 1 | [`ENV_PORT`]（`environment` に渡された綴り。前後の空白は落とす） | [`PortSource::Environment`] |
/// | 2 | 保存した番号（`saved`） | [`PortSource::Saved`] |
/// | 3 | [`DEFAULT_PORT`] | [`PortSource::Default`] |
///
/// **起動時の設定（[`AppConfig`]）と、`GET /admin/server` の「次の起動で使う番号」が同じ関数を通る**
/// （判定の写しを作らない）。
///
/// # Errors
///
/// [`ENV_PORT`] がポート番号として読めないとき（[`ConfigError::Port`]）。**保存した番号へは落とさない**
/// ——明示した値の誤りは、今までどおり起動を止める。
pub fn resolve_port(
    environment: Option<&str>,
    saved: Option<SavedPort>,
) -> Result<(u16, PortSource), ConfigError> {
    if let Some(raw) = environment {
        let port = raw.trim().parse::<u16>().map_err(|_| ConfigError::Port)?;
        return Ok((port, PortSource::Environment));
    }
    Ok(match saved {
        Some(saved) => (saved.get(), PortSource::Saved),
        None => (DEFAULT_PORT, PortSource::Default),
    })
}

/// [`ENV_STAT_DRIFT`] を on と読む綴り。**閉じた語彙**（小文字に畳んでから照合する）。
const STAT_DRIFT_ON: [&str; 4] = ["1", "true", "on", "yes"];

/// [`ENV_STAT_DRIFT`] を off と読む綴り。**空文字列も含む。**
///
/// **他の2つと語を共有しない**ことは
/// `the_three_stat_drift_vocabularies_never_overlap` が見張る
/// （重なると、同じ綴りが読む順で意味を変える）。
const STAT_DRIFT_OFF: [&str; 5] = ["0", "false", "off", "no", ""];

/// [`ENV_STAT_DRIFT`] を**記録専用**と読む綴り
/// （[ADR-0026](../../docs/adr/0026-drift-by-accumulation.md) §決定1）。
///
/// # なぜ綴りが1つしかないのか
///
/// on / off は「真偽値の綴り」という**既に人の頭にある語彙**なので別名が要るが、
/// 記録専用モードは**この基盤が作った概念**である。別名を用意すると、
/// **測定のログに「どの綴りで起動したか」が現れない**（同じモードを指す語が増えるだけ）。
///
/// **`StatDriftMode::Shadow` の集計キーと同じ綴りにしてある**ことは
/// `every_mode_label_is_a_spelling_the_parser_accepts` が見張る
/// ——ログに出た語をそのまま `PERSONACORE_STAT_DRIFT` へ書き戻せる。
const STAT_DRIFT_SHADOW: [&str; 1] = ["shadow"];

/// [`ENV_DRIFT_FORM`] を**1軸だけ尋ねる形**と読む綴り。
///
/// **別名を用意しない。** `single` / `all` は**この基盤が作った概念**なので、
/// 真偽値のような既にある語彙を借りる理由が無い（[`STAT_DRIFT_SHADOW`] と同じ判断）。
///
/// # ⚠ **空文字列はここに置かない**（作業単位 19 で移した）
///
/// **空文字列は「未設定」と同じ扱い**にする——3つのスイッチで揃えてある
/// （[`STAT_DRIFT_OFF`] / [`DRIFT_NOTE_NONE`] のどちらも、空文字列は**既定側**の語彙に居る）。
/// **既定が `all` へ移った**（ADR-0027 §決定8）ので、空文字列も [`DRIFT_FORM_ALL`] 側へ移した。
///
/// > **⚠ 既定を動かすときは、空文字列も一緒に動かすこと。**
/// > 既定が `single` だった間、空文字列がここに居るのは正しかった。
/// > **既定だけを移すと `PERSONACORE_DRIFT_FORM=` だけが非既定を指す**
/// > ——スクリプトや compose ファイルで**値を空のまま置いた人が、
/// > 測って降ろした形**（16i の上げ偏り 94%・分化しない）**で走る。**
/// > **起動は成功するので、誰も気づかない。**
/// > 見張り番は `every_drift_switch_reads_an_empty_value_as_unset`。
///
/// **`DriftForm::label` と同じ綴り**であることは
/// `every_drift_form_label_is_a_spelling_the_parser_accepts` が見張る。
const DRIFT_FORM_SINGLE: [&str; 1] = ["single"];

/// [`ENV_DRIFT_FORM`] を**全軸採点**と読む綴り。**空文字列も含む**（＝既定 / 作業単位 19）。
///
/// **他方と語を共有しない**ことは `the_two_drift_form_vocabularies_never_overlap` が見張る
/// （重なると、同じ綴りが読む順で意味を変える）。
const DRIFT_FORM_ALL: [&str; 2] = ["all", ""];

/// [`ENV_DRIFT_NOTE`] を**注記なし**（腕 X）と読む綴り。**空文字列も含む。**
///
/// **別名を用意しない**（[`DRIFT_FORM_SINGLE`] と同じ判断）。
/// **`off` / `0` / `false` を足さないこと**——これは真偽値ではなく**3本の腕の1本**で、
/// 「注記を off にした」ではなく「**腕 X で測った**」と読ませたい。
const DRIFT_NOTE_NONE: [&str; 2] = ["none", ""];

/// [`ENV_DRIFT_NOTE`] を**複写の指示**（腕 Y）と読む綴り。
const DRIFT_NOTE_COPY: [&str; 1] = ["copy"];

/// [`ENV_DRIFT_NOTE`] を**言語の指示**（腕 Z）と読む綴り。
///
/// **3つの語彙が1語も重ならない**ことは
/// `the_three_drift_note_vocabularies_never_overlap` が見張る
/// （重なると、同じ綴りが照合する順で腕を変える）。
const DRIFT_NOTE_LANGUAGE: [&str; 1] = ["language"];

/// 既定の DB のファイル名（ADR-0041 §決定3 / 作業単位 F-4a）。
///
/// **置き場所は持たない。** 場所は OS の作法（[`os_data_dir`]）の下の
/// [`APP_DATA_DIR_NAME`] で、そこにこの名前で置く。
///
/// > **⚠ かつて `DEFAULT_DATABASE_PATH = "personacore.sqlite"` という
/// > 「カレントディレクトリからの相対パス」だった。**
/// > **ADR-0041 §決定3 がそれを捨てた**——実行ファイルをどこから叩いたかで
/// > DB が変わる形は、**置き場所に依存するものが実行ファイルの外に在る**ということで、
/// > ADR-0041 の背景に書いた事故そのものである。
pub const DATABASE_FILE_NAME: &str = "personacore.sqlite";

/// OS のデータディレクトリの下に作る、このアプリのディレクトリ名（ADR-0041 §決定3）。
pub const APP_DATA_DIR_NAME: &str = "personacore";

/// **画面を実行ファイルへ埋め込む元**（ADR-0041 §決定2 / 作業単位 F-4a）。
///
/// **`vite build` の出力先と同じ綴り**（`web/vite.config.ts` の `build.outDir`）。
/// **写しなので、片方だけ変えると UI が黙って出なくなる**——
/// 変えるときは両方（と `web/README.md`）を一緒に動かすこと。
///
/// # 見張り番は2本で、両端をそれぞれ押さえる
///
/// | 見張り番 | 突き合わせる相手 |
/// |---|---|
/// | `the_default_web_dir_is_where_vite_actually_builds` | **`web/vite.config.ts` の `build.outDir`**（実ファイルを読む） |
/// | `the_embedded_ui_comes_from_the_default_web_dir`（`server::web`） | **`include_dir!` に書いた綴り**（`src/server/web.rs` を読む） |
///
/// **2本で1本の鎖**——`vite` の出力先 ← この定数 → 埋め込み元。
/// **どちらかが切れると、埋め込まれる画面が「ビルドした画面」でなくなる。**
///
/// > **⚠ この定数は「既定の配り先」ではなくなった**（ADR-0041 §決定2）。
/// > **未設定なら埋め込みを配る。** ディスクから配るのは
/// > [`ENV_WEB_DIR`] が設定されているときだけ。
pub const DEFAULT_WEB_DIR: &str = "web/dist";

/// 待ち受けアドレス。**ループバック固定。**
///
/// `From<SocketAddr>` も `parse` も生やさない。唯一のコンストラクタが `loopback` なので、
/// 「外部公開しない」という絶対制約が**設定ミスでは破れない**（破るにはこの型を書き換える
/// 必要がある）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindAddr(SocketAddr);

impl BindAddr {
    /// 唯一のコンストラクタ。IP は常に `127.0.0.1`。
    ///
    /// ポート `0` は「OS に空きを選ばせる」の意味（実際のポートは
    /// `App::local_addr` で確認する）。
    #[must_use]
    pub const fn loopback(port: u16) -> Self {
        Self(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
    }

    /// `TcpListener::bind` に渡す値。
    #[must_use]
    pub const fn socket_addr(self) -> SocketAddr {
        self.0
    }

    /// 設定されたポート（`0` なら未解決）。
    #[must_use]
    pub const fn port(self) -> u16 {
        self.0.port()
    }
}

impl std::fmt::Display for BindAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 設定が受け付けられない理由。
///
/// `Display` は**定数文字列**（起動時のエラーだが、方針を項目ごとに割らない）。
/// 詳細は `#[source]` 側に残る。
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// ポート番号として読めない（数値でない・範囲外・空）。
    #[error("待ち受けポートの指定が不正です")]
    Port,

    /// DB ファイルのパスが空。
    #[error("データベースのパスが空です")]
    DatabasePath,

    /// **DB の既定の置き場所を決められない**（ADR-0041 §決定3 / 作業単位 F-4a）。
    ///
    /// OS が言うデータディレクトリ（Windows は `%LOCALAPPDATA%`、Mac は `$HOME`、
    /// Linux は `$XDG_DATA_HOME` か `$HOME`）が**1つも読めない**環境で、
    /// かつ [`ENV_DATABASE_PATH`] も設定されていないときに出る。
    ///
    /// # ⚠ **黙ってカレントディレクトリへ落とさない**
    ///
    /// **ADR-0041 の背景そのもの**——開発フォルダを複製したら動かなくなった事故は、
    /// 「置き場所に依存するものが実行ファイルの外に在る」形だった。
    /// **カレントディレクトリへ落とすと、叩いた場所ごとに別の DB が生える。**
    /// 止めて、[`ENV_DATABASE_PATH`] を設定してもらう。
    #[error("データベースの置き場所を決められません。PERSONACORE_DB_PATH を設定してください")]
    DataDir,

    /// 推論サーバのベース URL が不正。
    #[error("推論サーバのエンドポイントの指定が不正です")]
    Endpoint(#[source] EndpointError),

    /// モデル識別子が不正。
    #[error("モデル識別子の指定が不正です")]
    Model(#[source] ModelIdError),

    /// コンテキスト長として読めない、または予算として成立しない。
    ///
    /// **明示設定の不正は起動時に落とす。** 上流への照会（`super::context`）が
    /// 失敗したときと違い、これは利用者の意思表示なので黙って既定値へ落とさない。
    #[error("コンテキスト長の指定が不正です")]
    ContextTokens,

    /// **打ち切りの天井が秒として読めない、または使えない値**（ADR-0035 §決定3 / B-6）。
    ///
    /// [`Self::ContextTokens`] と同じく「明示設定の不正は起動時に落とす」側。
    /// **黙って既定へ落とさない**——`PERSONACORE_LLM_IDLE_TIMEOUT=600s` を
    /// 180 秒として起動すると、**利用者は 10 分待つつもりで 3 分で切られる。**
    /// しかも切られた回は1行も保存されない（ADR-0019 §4）。
    ///
    /// **`0` もここへ落ちる**（理由は [`ENV_LLM_IDLE_TIMEOUT`] の doc と
    /// `an_unusable_llm_idle_timeout_stops_the_startup`）。
    #[error("上流の応答を打ち切る上限の指定が不正です")]
    LlmIdleTimeout,

    /// **性格ステータスの変容スイッチが読めない**（ADR-0024 §8）。
    ///
    /// **黙って off へ落とさない。** これは実験の切り替えスイッチなので、
    /// `PERSONACORE_STAT_DRIFT=onn` を off として起動すると、
    /// **測定者は on のつもりで off の結果を読み、そのまま結論を書く。**
    /// `ContextTokens` と同じく「明示設定の不正は起動時に落とす」側。
    #[error("性格ステータスの変容の指定が不正です")]
    StatDrift,

    /// **変容の依頼の形が読めない**（ADR-0027 §12）。
    ///
    /// [`Self::StatDrift`] とまったく同じ理由で**黙って既定へ落とさない。**
    /// `PERSONACORE_DRIFT_FORM=al` を `single` として起動すると、
    /// **測定者は全軸採点のつもりで1軸の結果を読み、そのまま結論を書く。**
    /// **この作業単位の存在理由そのものが「同じ台本で2つの形を比べる」**なので、
    /// ここを黙って落とすと実験が成立しない。
    #[error("変容の依頼の形の指定が不正です")]
    DriftForm,

    /// **変容の依頼に足す注記が読めない**（ADR-0029 §訂正 / 作業単位 16m）。
    ///
    /// [`Self::DriftForm`] とまったく同じ理由で**黙って既定へ落とさない。**
    /// `PERSONACORE_DRIFT_NOTE=lang` を腕 X（注記なし）として起動すると、
    /// **測定者は腕 Z のつもりで腕 X の結果を読み、「言語の指示は効かなかった」と書く。**
    /// **それは ADR-0029 §決定5 の保留を、測らずに畳むことと同じ**である。
    #[error("変容の依頼に足す注記の指定が不正です")]
    DriftNote,

    /// **変容の集約のつまみが読めない**（ADR-0027 §決定8）。
    ///
    /// [`Self::StatDrift`] / [`Self::DriftForm`] と同じ理由で**黙って既定へ落とさない。**
    /// つまみは**実験のために振るもの**なので、`PERSONACORE_DRIFT_STEP_COST=1.5.0` を
    /// 既定の `1` として起動すると、**測定者は 1.5 のつもりで 1 の結果を読む。**
    ///
    /// # 3つのつまみで理由を分けない
    ///
    /// 詳細は `#[source]` 側（`DriftTuningError`）にあり、
    /// **あちらは3つを区別している。** ここで割ると、
    /// **同じ「読めない」が2つの層に写る**（CLAUDE.md「同じ発見を2箇所に書かない」）。
    #[error("性格の変容の集約のつまみの指定が不正です")]
    DriftTuning(#[source] DriftTuningError),

    /// **軸の候補を尋ねる間隔が読めない、または 0**（ADR-0046 §決定2 / 作業単位 A-9b）。
    ///
    /// [`Self::DriftTuning`] と同じく**黙って既定へ落とさない** —— これは測定の器具で、
    /// `PERSONACORE_AXIS_DISCOVERY_MEMORIES=5件` を既定として起動すると、
    /// **測定者は 5 のつもりで既定の結果を読む。**
    #[error("軸の候補を尋ねる間隔の指定が不正です")]
    AxisDiscoveryMemories,

    /// **軸の候補の試用の判定のつまみが読めない、または組として範囲外**
    /// （ADR-0046 §決定4 / 作業単位 A-9c / `PERSONACORE_AXIS_TRIAL_*`）。
    ///
    /// [`Self::AxisDiscoveryMemories`] と同じく**黙って既定へ落とさない** —— 測定の器具で、
    /// `PERSONACORE_AXIS_TRIAL_TURNS=30回` を既定として起動すると、**測定者は 30 のつもりで 60 の結果を読む。**
    ///
    /// **3つで理由を分けない**（[`Self::DriftTuning`] と同じ）—— 詳細は `#[source]` 側
    /// （`AxisTrialRuleError`）にあり、あちらは3つを区別している。
    #[error("軸の候補の試用の判定の指定が不正です")]
    AxisTrial(#[source] AxisTrialRuleError),
}

/// **上流プロファイルの面に書いたコンテキスト長**（ADR-0054 §決定1 / [`AppConfig::with_profile_context`]）。
///
/// **どちらも検証済みの型で持つ**（本文は予算、抽出は抽出の上限）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::server) struct ProfileContext {
    /// **本文の予算の明示。** [`ENV_CONTEXT_TOKENS`] に勝つ。
    pub(in crate::server) main: Option<TokenBudget>,
    /// **抽出の上限の明示。**
    pub(in crate::server) sub: Option<SubContext>,
}

impl ProfileContext {
    /// **選んだプロファイルの値を、検証済みの型に直す**（起動時の [`AppConfig::with_profile_context`] と、
    /// 走ったままの切り替え（`super::switch`）が同じ関数を通る）。
    ///
    /// **値は読み込み（`upstreams::parse_profiles`）で検証済み**だが、ここでもう一度
    /// [`context_budget_of`] を通す——**`unwrap` で「検証済みのはず」に寄りかからない。**
    /// 抽出の側は抽出の上限としても成り立つかを見る（`extract::SubContext::new`。
    /// 予算の規則を通る値は必ず通る —— `the_profile_context_rule_admits_every_budget_as_an_extraction_limit`）。
    pub(in crate::server) fn of(settings: &ProfileSettings) -> Result<Self, ConfigError> {
        let main = settings
            .context_tokens()
            .map(context_budget_of)
            .transpose()?;
        let sub = match settings.sub_context_tokens() {
            Some(tokens) => {
                context_budget_of(tokens)?;
                Some(SubContext::new(tokens).ok_or(ConfigError::ContextTokens)?)
            }
            None => None,
        };
        Ok(Self { main, sub })
    }
}

/// 起動設定一式。
#[derive(Debug, Clone)]
pub struct AppConfig {
    bind: BindAddr,
    /// **待ち受けのポートが何から決まったか**（ADR-0057 §決定5 / [`resolve_port`]）。
    ///
    /// **[`Self::with_bind`] では変わらない**（テストがエフェメラルポートへ差し替える入口で、
    /// 出どころの話はしない）。
    port_source: PortSource,
    /// **待ち受けを探すときに飛ばす番号**（`upstreams.json` の全部のプロファイルの gguf の面が使う番号 /
    /// ADR-0057 §決定3 / [`Self::with_face_ports`]）。
    ///
    /// **既定は空**（`main` が読んで渡す。テストの入口は渡さない限り何も飛ばさない）。
    /// **明示（`environment`）と `0` の起動では見ない**（`port_search::PortSearch::plan`）。
    face_ports: BTreeSet<u16>,
    database_path: PathBuf,
    /// **UI をディスクから配るなら、その配り先**（ADR-0041 §決定2 / [`ENV_WEB_DIR`]）。
    ///
    /// **`None` は「実行ファイルに埋め込んだ画面を配る」**（＝既定）。
    /// **`Option` になったのは F-4a から**——それまでは未設定が
    /// [`DEFAULT_WEB_DIR`] という**既定のパス**を意味していた。
    ///
    /// **実在するとは限らない。** ビルド成果物なので、`pnpm build` を忘れれば無い。
    /// **その事実は起動ログが出す**（[`ENV_WEB_DIR`] の doc）。
    web_dir: Option<PathBuf>,
    llm: LmStudioConfig,
    /// **裏方（記憶抽出）に使うモデル**（ADR-0021 §10）。
    ///
    /// **`LmStudioConfig` の中に入れない。** あちらは「1つの上流クライアントの設定」で、
    /// モデルは1つしか持たない（`LmStudioClient::with_model` で複製する形）。
    /// ここに並べて持つと、どちらが本文でどちらが裏方かが型で分かる。
    sub_model: ModelId,
    /// **裏方（記憶抽出）が話す上流**（ADR-0039 §決定1 / [`ENV_SUB_LLM_ENDPOINT`]）。
    ///
    /// **明示設定されたときだけ `Some`。** `web_dir` と違って既定値で埋めない——
    /// **`None` は「本文と同じ上流」という決定**であり、そこには
    /// **既定値では表現できない事実**（同じ `reqwest::Client` の接続プールを共有する）が
    /// 乗っている。`Some` に潰すと、未設定の起動でも2つ目のクライアントを組むことになる。
    ///
    /// **⚠ 「既定値で埋めても観測できない」は偽だった**（G-3a の是正2周目 / 2026-09-07）。
    /// **本文と同じ宛先を入れた `Some` でも、[`crate::server::App::bind`] は
    /// `LmStudioClient::new` を通る**ので、
    /// **宛先とモデルが同じまま、接続プールだけが分かれる。**
    /// **そしてそれは観測できる。**
    ///
    /// **実測（`m20`: この欄の `_ => None` を「本文と同じ宛先の `Some`」へ差し替える）は 3 赤・exit 101。**
    /// うち1本は**振る舞いの側**——`tests/server_sub_llm_endpoint.rs` の
    /// `the_unset_extraction_client_shares_the_body_clients_connection_pool`。
    /// 残り2本はこのファイルの
    /// `the_sub_llm_endpoint_builds_a_second_upstream_only_when_it_is_set` と
    /// `an_empty_sub_llm_endpoint_reads_as_unset`。
    /// **どうやって観測するかは [`AppConfig::sub_llm`] の doc。**
    ///
    /// **中の `model` は [`Self::sub_model`] と同じ綴り。** 2つ目の上流を組んでも
    /// 「どのモデルに頼むか」の決定権は動かない。
    sub_llm: Option<LmStudioConfig>,
    /// **明示設定されたときだけ `Some`。** 未設定は `None` のままにする
    /// （既定値で埋めると上流への照会が行われなくなる）。
    ///
    /// **環境変数（[`ENV_CONTEXT_TOKENS`]）の値だけを持つ。** 上流プロファイルの面に書いた値は
    /// [`Self::with_profile_context`] で別の欄（`profile_context`）に入る——
    /// **ここへ重ねると、走ったまま値の無いプロファイルへ切り替えたときに戻る先が消える**
    /// （ADR-0054 §決定3）。
    context_budget: Option<TokenBudget>,
    /// **上流プロファイルの面に書いたコンテキスト長**（ADR-0054 §決定1 / 作業単位 A-13a-1）。
    ///
    /// **既定は「何も書いていない」**（`--upstream` を渡さない起動・環境変数だけの起動）。
    profile_context: ProfileContext,
    /// **性格ステータスの変容の走らせ方**（ADR-0024 §8 / ADR-0026 §決定1）。
    /// **既定は [`StatDriftMode::Off`]。**
    ///
    /// `context_budget` と違って `Option` にしない。ここは
    /// 「まだ決まっていない」という状態を持たない——**未設定は off という決定**である
    /// （上流へ問い合わせる余地が無い）。
    ///
    /// **`bool` をやめたのは 16d。** 記録専用（`shadow`）が増えて3値になった。
    stat_drift: StatDriftMode,
    /// **変容の依頼の形**（ADR-0027 §12 / [`ENV_DRIFT_FORM`]）。
    /// **既定は [`DriftForm::All`]**（作業単位 19 / ADR-0027 §決定8）。
    ///
    /// `stat_drift` と同じく `Option` にしない——**未設定は `all` という決定**である。
    drift_form: DriftForm,
    /// **変容の依頼に足す注記**（ADR-0029 §訂正 / [`ENV_DRIFT_NOTE`]）。
    /// **既定は [`DriftNote::None`]。**
    ///
    /// `drift_form` と同じく `Option` にしない——**未設定は「注記なし」という決定**である。
    drift_note: DriftNote,
    /// **変容の集約のつまみ**（ADR-0027 §決定8 / §9）。**既定は ADR の「当面の値」。**
    ///
    /// `drift_form` と同じく `Option` にしない——未設定は既定値という決定である。
    ///
    /// **3つの `f64` を別々に持たない。** 別々にすると
    /// 「λ だけ検証されている」状態が作れるうえ、**渡すときに順番を取り違えても型が通る。**
    drift_tuning: DriftTuning,
    /// **軸の候補を尋ねる間隔 `N`**（ADR-0046 §決定2 / [`ENV_AXIS_DISCOVERY_MEMORIES`]）。
    ///
    /// `drift_tuning` と同じく `Option` にしない——未設定は既定値という決定である。
    axis_discovery_memories: AxisDiscoveryMemories,
    /// **軸の候補の試用の判定のつまみ**（ADR-0046 §決定4 / [`ENV_AXIS_TRIAL_TURNS`] ほか2つ）。
    ///
    /// **3つを別々に持たない**（`drift_tuning` と同じ理由 —— 組で検証し、順番を取り違えさせない）。
    axis_trial: AxisTrialRule,
}

/// **OS が言うデータディレクトリ**（ADR-0041 §決定3 / 作業単位 F-4a）。
///
/// | OS | 読む環境変数 | 例 |
/// |---|---|---|
/// | Windows | `LOCALAPPDATA` | `C:\Users\…\AppData\Local` |
/// | Mac | `HOME` ＋ `Library/Application Support` | `/Users/…/Library/Application Support` |
/// | それ以外の Unix | `XDG_DATA_HOME`、無ければ `HOME` ＋ `.local/share` | `/home/…/.local/share` |
///
/// **依存クレート（`directories` / `dirs`）を足していない。**
/// 引くのは環境変数だけで、規則は上の表がすべてである。
/// **1つの真偽値に引数解析の依存を足さない**のと同じ判断（ADR-0041 §決定4）。
///
/// # ⚠ **コンパイルされるのは、いま居る OS の腕1本だけ**
///
/// だから **Windows の機からは Windows の腕しか測れない。**
/// 見張り番 `the_data_dir_comes_from_the_os_convention` は、
/// **その1本を植えた参照関数で測る**（実際の環境は読まない）。
/// **他の2本は、この機では一度も走っていない。**
fn data_dir_from<F>(lookup: &F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    /// 空・空白だけの値は「未設定」と読む（貼り付け由来の事故を通さない）。
    fn present<F>(lookup: &F, key: &str) -> Option<PathBuf>
    where
        F: Fn(&str) -> Option<String>,
    {
        let raw = lookup(key)?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        }
    }

    #[cfg(windows)]
    {
        present(lookup, "LOCALAPPDATA")
    }
    #[cfg(target_os = "macos")]
    {
        present(lookup, "HOME").map(|home| home.join("Library").join("Application Support"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        present(lookup, "XDG_DATA_HOME")
            .or_else(|| present(lookup, "HOME").map(|home| home.join(".local").join("share")))
    }
}

/// **プロセスの環境から** OS のデータディレクトリを引く。**ここだけが環境に触る。**
#[must_use]
pub fn os_data_dir() -> Option<PathBuf> {
    data_dir_from(&|key: &str| std::env::var(key).ok())
}

/// データディレクトリの下の、既定の DB のパス。**純粋関数。**
///
/// **`<データディレクトリ>/personacore/personacore.sqlite`**（ADR-0041 §決定3）。
#[must_use]
pub fn database_path_under(data_dir: &Path) -> PathBuf {
    data_dir.join(APP_DATA_DIR_NAME).join(DATABASE_FILE_NAME)
}

/// **DB の置き場所を決める。純粋関数**（ADR-0041 §決定3）。
///
/// [`ENV_DATABASE_PATH`] があればそれ（前後の空白を落とす。**空は [`ConfigError::DatabasePath`]**）、
/// 無ければ `data_dir` の下の既定（[`database_path_under`]）。**どちらも無ければ [`ConfigError::DataDir`]**
/// ——**カレントディレクトリへは落とさない**（決められなければ止める）。
///
/// **[`AppConfig`] の組み立てと、起動の最初にロックを置く場所（ADR-0057 §決定2 / `main`）の両方がここを通る**
/// ——写しを作らない。**`--upstream` のプロファイルはこの値を変えない**（`overlay_profile` が触るのは
/// 宛先とモデルの4つだけ）ので、ロックはプロファイルを読む前に取れる。
pub fn database_path_from<F>(lookup: &F, data_dir: Option<&Path>) -> Result<PathBuf, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(ENV_DATABASE_PATH) {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(ConfigError::DatabasePath);
            }
            Ok(PathBuf::from(trimmed))
        }
        None => Ok(database_path_under(data_dir.ok_or(ConfigError::DataDir)?)),
    }
}

/// **プロセスの環境から** DB の置き場所を決める（[`database_path_from`] に環境と [`os_data_dir`] を渡すだけ）。
///
/// `main` が起動の最初（設定を組む前）に、ロックを置く場所を決めるために呼ぶ（ADR-0057 §決定2）。
pub fn database_path_from_env() -> Result<PathBuf, ConfigError> {
    database_path_from(
        &|key: &str| std::env::var(key).ok(),
        os_data_dir().as_deref(),
    )
}

/// **宛先とモデルだけを差し替えた上流の設定**（走ったままの差し替え / 作業単位 F-4b-2）。
///
/// **検証は起動時と同じ**（[`ModelId::parse`] と `LmStudioConfig::new`）——
/// プロファイルの綴りが起動時には通らないのに差し替えでは通る、という形を作らない。
///
/// # 待ち時間は `like` から引き継ぐ
///
/// **`PERSONACORE_LLM_IDLE_TIMEOUT` は起動時に読んだきりで、プロファイルには載らない**
/// （ADR-0039 §決定2「待ち時間のつまみは増やさない」）。
/// **4つとも写す**——天井から導き直すと、明示されていない起動（既定の4つ）で
/// 既定とは違う値が生える（`with_idle_budget` は既定を再現しない向きがある）。
///
/// **ここに置くのは、`LmStudioConfig` と `EndpointError` を綴る場所を増やさないため**
/// （`llm::upstream_leaks` の台帳にこのファイルは既に載っている）。
///
/// # 鍵は `like` から引き継がない（ADR-0056 §決定2 / 作業単位 A-13b-1）
///
/// **鍵は切り替え先の面のもの**（`api_key`）を付ける。引き継ぐと、鍵の無いプロファイルへ
/// 切り替えても前の鍵を別の宛先へ送り続ける（鍵は宛先に付く / ADR-0056 §決定1）。
/// 写さないと、切り替えた瞬間に 401 になる。
///
/// # 承認も切り替え先の面のもの（ADR-0056 §決定5 / 作業単位 A-13b-2）
///
/// **`allow_key_over_plain_http` は鍵と同じく `like` から引き継がない**（承認は宛先に付く）。
/// **名前解決の口は `like` から写す**（待ち時間と同じ「どう繋ぐか」の設定。本番では OS の名前解決）。
pub(in crate::server) fn retarget_llm(
    like: &LmStudioConfig,
    base_url: &str,
    model: &str,
    api_key: Option<&ApiKey>,
    allow_key_over_plain_http: bool,
) -> Result<LmStudioConfig, ConfigError> {
    let model = ModelId::parse(model).map_err(ConfigError::Model)?;
    Ok(LmStudioConfig::new(base_url, model)
        .map_err(ConfigError::Endpoint)?
        .with_connect_timeout(like.connect_timeout())
        .with_request_timeout(like.request_timeout())
        .with_idle_timeout(like.idle_timeout())
        .with_context_timeout(like.context_timeout())
        .with_host_resolver_of(like)
        .with_api_key(api_key.cloned())
        .with_key_over_plain_http_allowed(allow_key_over_plain_http))
}

impl AppConfig {
    /// プロセスの環境変数から組み立てる。**環境に触るのはここだけ。**
    ///
    /// **`saved_port` は画面で保存した待ち受けのポート**（ADR-0057 §決定5。読むのは `main` で、
    /// 既定の置き場所のときだけ）。**引数で受ける**ので、渡し忘れる呼び出しは書けない。
    pub fn from_env(saved_port: Option<SavedPort>) -> Result<Self, ConfigError> {
        Self::from_lookup_with_saved_port(|key| std::env::var(key).ok(), saved_port)
    }

    /// 参照関数から組み立てる。未設定（`None`）の項目は既定値。
    ///
    /// 環境変数を読む以外の経路（テスト・将来の設定ファイル）はここに合流させる。
    ///
    /// **⚠ 純粋ではない**——[`ENV_DATABASE_PATH`] が未設定のときに
    /// [`os_data_dir`] を引く（モジュール doc の「`from_lookup` は純粋ではなくなった」）。
    /// **純粋な本体は [`Self::from_lookup_with_data_dir`]。**
    pub fn from_lookup<F>(lookup: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        Self::from_lookup_with_data_dir(lookup, os_data_dir())
    }

    /// **組み立ての本体。純粋関数。**
    ///
    /// `data_dir` は **OS の作法の場所**（ADR-0041 §決定3）。
    /// **`None` は「決められない環境」**で、[`ENV_DATABASE_PATH`] も無ければ
    /// [`ConfigError::DataDir`] で止まる。
    pub fn from_lookup_with_data_dir<F>(
        lookup: F,
        data_dir: Option<PathBuf>,
    ) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        Self::from_lookup_with_data_dir_and_saved_port(lookup, data_dir, None)
    }

    /// **画面で保存した待ち受けのポートも渡して組み立てる**（ADR-0057 §決定5 / 作業単位 F-12a）。
    ///
    /// `main` は環境変数の既定の経路（[`Self::from_env`]）とプロファイル経由の経路（`config_for`）の
    /// **両方でここ（の `os_data_dir` 版）を通る**——入口を増やさない（ADR-0042 §決定1）。
    /// **保存した番号を [`ENV_PORT`] の値として混ぜない**（混ぜると出どころが `environment` になる）。
    pub fn from_lookup_with_saved_port<F>(
        lookup: F,
        saved_port: Option<SavedPort>,
    ) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        Self::from_lookup_with_data_dir_and_saved_port(lookup, os_data_dir(), saved_port)
    }

    /// **組み立ての本体。純粋関数。**
    ///
    /// `data_dir` は [`Self::from_lookup_with_data_dir`]、`saved_port` は
    /// [`Self::from_lookup_with_saved_port`] の doc。**待ち受けのポートは [`resolve_port`] が決める。**
    pub fn from_lookup_with_data_dir_and_saved_port<F>(
        lookup: F,
        data_dir: Option<PathBuf>,
        saved_port: Option<SavedPort>,
    ) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        // **アドレスではなくポートだけを読む。** `0.0.0.0:8080` のような値は
        // 数値として読めないので、ここで弾かれる（[`resolve_port`]）。
        let (port, port_source) = resolve_port(lookup(ENV_PORT).as_deref(), saved_port)?;

        // **DB の置き場所は [`database_path_from`] の1か所で決める**（`main` がロックを置く場所も同じ関数 /
        // ADR-0057 §決定2）。
        let database_path = database_path_from(&lookup, data_dir.as_deref())?;

        // **UI をディスクから配るか**（ADR-0041 §決定2 / `PERSONACORE_WEB_DIR`）。
        // **未設定は `None`＝埋め込みを配る**（既定のパスへは落とさない）。
        // **空は未設定と読む**——理由と、`database_path` と作法が逆であることは
        // [`ENV_WEB_DIR`] の doc。
        let web_dir = match lookup(ENV_WEB_DIR) {
            Some(raw) if !raw.trim().is_empty() => Some(PathBuf::from(raw.trim())),
            _ => None,
        };

        let endpoint = lookup(ENV_LLM_ENDPOINT).unwrap_or_else(|| DEFAULT_ENDPOINT.to_owned());
        let model = lookup(ENV_MAIN_MODEL).unwrap_or_else(|| DEFAULT_MAIN_MODEL.to_owned());
        let model = ModelId::parse(&model).map_err(ConfigError::Model)?;
        // 綴り間違いは「起動はしたが本文生成だけ失敗する」形で出ると原因に遠いので、
        // 起動時に検証しておく。
        let llm = LmStudioConfig::new(&endpoint, model).map_err(ConfigError::Endpoint)?;

        // **打ち切りの天井**（ADR-0035 §決定3 / [`ENV_LLM_IDLE_TIMEOUT`]）。
        // **指定されたときだけ差し替える。** 未設定なら `LmStudioConfig::new` が
        // 掴んだ既定4つをそのまま持ち続ける——**ここで既定を書き戻さない**のは、
        // 書き戻すと「導出が既定を再現しているか」を誰も見なくなるため
        // （`llm::lmstudio` の `the_default_ceiling_reproduces_the_shipped_defaults`）。
        //
        // **天井そのものは変数に残す**（ADR-0039 §決定2）。2つ目の上流にも
        // **同じ1つ**を渡すため——ここで `llm` に畳んでしまうと、
        // 取り出す口が `LmStudioConfig::idle_timeout` しか無くなり、
        // 「本文の設定から抽出の設定を導く」という**向きが逆の依存**が生える。
        let idle_timeout = llm_idle_timeout(&lookup)?;
        let llm = match idle_timeout {
            Some(timeout) => llm.with_idle_budget(timeout),
            None => llm,
        };

        // **裏方のモデルも起動時に検証する**（ADR-0021 §10）。綴り間違いを通すと、
        // 現れ方は「本文は返るのに記憶がいつまでも増えない」になり、原因から遠い。
        let sub_model = lookup(ENV_SUB_MODEL).unwrap_or_else(|| DEFAULT_SUB_MODEL.to_owned());
        let sub_model = ModelId::parse(&sub_model).map_err(ConfigError::Model)?;

        // **裏方が話す2つ目の上流**（ADR-0039 §決定1 / [`ENV_SUB_LLM_ENDPOINT`]）。
        //
        // **空は未設定として読む**（`ENV_WEB_DIR` と同じ側。理由はあの const の doc）。
        // **不正な明示設定は起動を止める**（`ConfigError::Endpoint` / 本文側と同じ作法）——
        // 黙って本文と同じ上流へ落とすと、**利用者は2つ目を指したつもりで
        // 1つ目に全部行っている状態**を、ログのどこからも区別できない。
        //
        // **待ち時間のつまみは増やさない**（ADR-0039 §決定2）ので、
        // **本文と同じ天井をそのまま渡す。**
        let sub_llm = match lookup(ENV_SUB_LLM_ENDPOINT) {
            Some(raw) if !raw.trim().is_empty() => {
                let sub = LmStudioConfig::new(raw.trim(), sub_model.clone())
                    .map_err(ConfigError::Endpoint)?;
                Some(match idle_timeout {
                    Some(timeout) => sub.with_idle_budget(timeout),
                    None => sub,
                })
            }
            _ => None,
        };

        let context_budget = match lookup(ENV_CONTEXT_TOKENS) {
            Some(raw) => Some(parse_context_budget(&raw)?),
            None => None,
        };

        // **綴りが読めなければ起動を止める**（ADR-0024 §8 / `ConfigError::StatDrift`）。
        let stat_drift = match lookup(ENV_STAT_DRIFT) {
            Some(raw) => parse_stat_drift(&raw)?,
            None => StatDriftMode::Off,
        };

        // **こちらも綴りが読めなければ起動を止める**（ADR-0027 §12 / `ConfigError::DriftForm`）。
        let drift_form = match lookup(ENV_DRIFT_FORM) {
            Some(raw) => parse_drift_form(&raw)?,
            // **未設定は `all`**（作業単位 19 / ADR-0027 §決定8 / 16j）。
            None => DriftForm::All,
        };

        // **注記も同じ作法**（ADR-0029 §訂正 / `ConfigError::DriftNote`）。
        // **黙って腕 X へ落とすと、測らずに保留を畳むことになる。**
        let drift_note = match lookup(ENV_DRIFT_NOTE) {
            Some(raw) => parse_drift_note(&raw)?,
            None => DriftNote::None,
        };

        // **3つのつまみを1つの型にまとめてから検証する**（ADR-0027 §決定8）。
        // 読めない綴りも範囲の外も、**黙って既定へ落とさずに起動を止める。**
        let drift_tuning = DriftTuning::new(
            drift_knob(
                &lookup,
                ENV_DRIFT_DECAY,
                DEFAULT_DRIFT_DECAY,
                DriftTuningError::Decay,
            )?,
            drift_knob(
                &lookup,
                ENV_DRIFT_SLOPE,
                DEFAULT_DRIFT_SLOPE,
                DriftTuningError::Slope,
            )?,
            drift_knob(
                &lookup,
                ENV_DRIFT_STEP_COST,
                DEFAULT_DRIFT_STEP_COST,
                DriftTuningError::StepCost,
            )?,
        )
        .map_err(ConfigError::DriftTuning)?;

        // **軸の候補を尋ねる間隔**（ADR-0046 §決定2 / `ConfigError::AxisDiscoveryMemories`）。
        // 読めない綴りも 0 も、**黙って既定へ落とさずに起動を止める。**
        let axis_discovery_memories = axis_discovery_memories(&lookup)?;

        // **軸の候補の試用の判定のつまみ**（ADR-0046 §決定4 / `ConfigError::AxisTrial`）。
        // 読めない綴りも範囲外の組も、**黙って既定へ落とさずに起動を止める。**
        let axis_trial = axis_trial_rule(&lookup)?;

        Ok(Self {
            bind: BindAddr::loopback(port),
            port_source,
            face_ports: BTreeSet::new(),
            database_path,
            web_dir,
            llm,
            sub_model,
            sub_llm,
            context_budget,
            profile_context: ProfileContext::default(),
            stat_drift,
            drift_form,
            drift_note,
            drift_tuning,
            axis_discovery_memories,
            axis_trial,
        })
    }

    /// 待ち受けアドレスを差し替える（テストのエフェメラルポート指定など）。
    ///
    /// **出どころ（[`Self::port_source`]）は変えない**——出どころが `default` のままなら、その番号が塞がって
    /// いれば次の空きへずれる（ADR-0057 §決定1）。**塞がっていたら止まってほしいテストは、`PERSONACORE_PORT`
    /// を参照関数から渡す**（出どころ `environment`）。
    #[must_use]
    pub fn with_bind(mut self, bind: BindAddr) -> Self {
        self.bind = bind;
        self
    }

    /// **待ち受けを探すときに飛ばす番号を載せる**（ADR-0057 §決定3 / 作業単位 F-11b）。
    ///
    /// `main` は `upstreams.json` の全部のプロファイルの gguf の面が使う番号（`UpstreamProfiles::launch_ports`）
    /// を、番号を明示していない起動でだけ渡す。**明示（`environment`）と `0` では、渡しても見ない。**
    /// 渡し直すと置き換える（足し合わせない）。
    #[must_use]
    pub fn with_face_ports(mut self, ports: impl IntoIterator<Item = u16>) -> Self {
        self.face_ports = ports.into_iter().collect();
        self
    }

    /// **この起動が待ち受けを探す計画**（候補の列 / `port_search` モジュール）。**純粋関数。**
    pub(in crate::server) fn port_search(&self) -> PortSearch {
        PortSearch::plan(self.bind.port(), self.port_source, &self.face_ports)
    }

    /// **番号を明示したときだけ、その待ち受けのアドレス**（ADR-0057 §決定3）。
    ///
    /// `main` の覚えた名前の事前検査（`LaunchPlan::check_before_starting`）はこれを渡す——**明示していなければ
    /// 番号の取り合いでプロファイルを捨てない**（待ち受けを探すときに面の番号を飛ばすので、取り合わない）。
    #[must_use]
    pub fn explicit_bind(&self) -> Option<BindAddr> {
        (self.port_source == PortSource::Environment).then_some(self.bind)
    }

    /// DB ファイルのパスを差し替える。
    #[must_use]
    pub fn with_database_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.database_path = path.into();
        self
    }

    /// **選んだ上流プロファイルの面に書いたコンテキスト長を載せる**（ADR-0054 §決定1・2 / 作業単位 A-13a-1）。
    ///
    /// `main` は `--upstream`（と覚えた名前）で選んだプロファイルの [`ProfileSettings`] をここへ渡す
    /// （**環境変数4つの側は [`super::upstreams::overlay_profile`]、こちらは別の入口**——
    /// 理由は [`ProfileSettings`] の doc）。
    ///
    /// | 面 | 書いた値の意味 |
    /// |---|---|
    /// | 本文 | **本文の予算の明示。[`ENV_CONTEXT_TOKENS`] に勝つ**（ぶつかったら `App::bind` が `warn` を1本） |
    /// | 抽出 | **抽出の上限の明示**（`sub` 面が無ければ本文の値 / [`ProfileSettings::sub_context_tokens`]） |
    ///
    /// **検証は読み込みと同じ規則でもう一度通す**（`ProfileContext::of` の doc）。
    ///
    /// # Errors
    ///
    /// 値が予算にならないとき（読み込みを通った値では起きない）。
    pub fn with_profile_context(mut self, settings: &ProfileSettings) -> Result<Self, ConfigError> {
        self.profile_context = ProfileContext::of(settings)?;
        Ok(self)
    }

    /// **選んだ上流プロファイルの面に書いた鍵を載せる**（ADR-0056 §決定1・2 / 作業単位 A-13b-1）。
    ///
    /// `main` は `--upstream`（と覚えた名前）で選んだプロファイルの [`ProfileSettings`] をここへ渡す
    /// （**環境変数には鍵の口を作らない** —— ADR-0056 §決定1。だから [`super::upstreams::overlay_profile`]
    /// を通らない）。
    ///
    /// | 面 | 載せる鍵 |
    /// |---|---|
    /// | 本文 | `main` 面の鍵 |
    /// | 抽出（`sub` 面を書いた＝2つ目の上流がある） | **`sub` 面の鍵**（無ければ付けない。本文の鍵を貸さない） |
    /// | 抽出（`sub` 面を書いていない） | 何もしない —— 抽出は本文のクライアントの複製を使うので、本文の鍵が同じ宛先に付く |
    ///
    /// **面の承認（`allow_key_over_plain_http` / ADR-0056 §決定5 / 作業単位 A-13b-2）も同じ規則で載せる**
    /// （承認は鍵を送る宛先に付く。本文の承認を裏方の別の宛先へ貸さない）。
    #[must_use]
    pub fn with_profile_keys(mut self, settings: &ProfileSettings) -> Self {
        self.llm = self
            .llm
            .with_api_key(settings.api_key().cloned())
            .with_key_over_plain_http_allowed(settings.allow_key_over_plain_http());
        self.sub_llm = self.sub_llm.map(|sub| {
            sub.with_api_key(settings.sub_api_key().cloned())
                .with_key_over_plain_http_allowed(settings.sub_allow_key_over_plain_http())
        });
        self
    }

    /// **上流の名前解決を差し替える**（テスト専用 / 作業単位 A-13b-2）。本文と抽出の両方の上流に効く。
    ///
    /// **実際の DNS に依らずに**、鍵を平文の `http://` で送る宛先の判定（ADR-0056 §決定5）を
    /// 本番の入口（`App::bind`）と切り替え（`retarget_llm` が写す）で測るため。
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn with_host_resolver_for_tests(
        mut self,
        resolver: std::sync::Arc<dyn crate::llm::HostResolver>,
    ) -> Self {
        self.llm = self
            .llm
            .with_host_resolver(std::sync::Arc::clone(&resolver));
        self.sub_llm = self.sub_llm.map(|sub| sub.with_host_resolver(resolver));
        self
    }

    /// **上流プロファイルの面に書いたコンテキスト長**（[`Self::with_profile_context`]）。
    pub(in crate::server) const fn profile_context(&self) -> ProfileContext {
        self.profile_context
    }

    /// 待ち受けアドレス。**常にループバック。**
    #[must_use]
    pub const fn bind(&self) -> BindAddr {
        self.bind
    }

    /// **待ち受けのポートが何から決まったか**（ADR-0057 §決定5）。
    #[must_use]
    pub const fn port_source(&self) -> PortSource {
        self.port_source
    }

    /// SQLite ファイルのパス。
    #[must_use]
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// **UI をディスクから配るなら、その配り先**（ADR-0041 §決定2 / [`ENV_WEB_DIR`]）。
    ///
    /// **`None` は「実行ファイルに埋め込んだ画面を配る」**（＝既定）。
    ///
    /// **実在は保証しない**（ビルド成果物なので、`pnpm build` を忘れれば無い）。
    /// どちらから配っていて、そこに何かあるかは起動ログの
    /// `ui_source` / `ui_origin` / `ui_files` に出る。
    #[must_use]
    pub fn web_dir(&self) -> Option<&Path> {
        self.web_dir.as_deref()
    }

    /// 推論サーバの設定。
    #[must_use]
    pub const fn llm(&self) -> &LmStudioConfig {
        &self.llm
    }

    /// **裏方（記憶抽出）に使うモデル**（ADR-0021 §10）。
    ///
    /// **本文生成には使わない。** 本文は `llm().model()` の側。
    /// **既定どうしが同じ綴りなので、取り違えても既定のままでは何も起きない**——
    /// 現れるのは `PERSONACORE_SUB_MODEL` で小型モデルへ振り分けたときで、
    /// **そのとき小型モデルが本文を書く**（＝キャラクターの質が黙って落ちる）。
    #[must_use]
    pub const fn sub_model(&self) -> &ModelId {
        &self.sub_model
    }

    /// **裏方（記憶抽出）が話す2つ目の上流**（ADR-0039 §決定1 / [`ENV_SUB_LLM_ENDPOINT`]）。
    ///
    /// **明示設定されたときだけ `Some`。** `None` は「**本文と同じ上流**」の意味で、
    /// そのとき `server` は `LmStudioClient::with_model` で
    /// **モデルの綴りだけを差し替えた複製**を作る（接続プールは共有される）。
    ///
    /// **`Some` のときだけ2つ目のクライアントを組むこと**（ADR-0039 §決定1）。
    /// 未設定でも組むと、**同じ宛先・同じモデルのまま接続プールだけが分かれる。**
    ///
    /// # ⚠ 「観測できない」は偽だった（**是正2周目で見張り番にした**）
    ///
    /// **1周目のこの節は「この規約を破る変異はスイートを素通りする」と書いていた。**
    /// 根拠は「起動時の2本の照会は**同時に走る**ので、
    /// プールを共有していても接続は2本開く」——
    /// **前半は正しい。そこから『TCP の本数でも割れない』と断定したのが誤りだった**
    /// （レビュアーが実測して割った / 2026-09-07）。
    ///
    /// # 観測するには2つ要る
    ///
    /// * **接続を数える器具。** `wiremock` は「どんな要求が届いたか」しか見せない。
    ///   **宛先もモデルも同じなので、要求そのものは1バイトも変わらない**——
    ///   **手書きの TCP の待ち受けで `accept` を数えるしかない。**
    /// * **重なる往復。** 本文の依頼を2本、同時に。
    ///   **共有なら起動時の2本がどちらも本文側に使えるので接続は増えず（2本）、
    ///   分けると本文側の手持ちは1本しか無いので3本目が開く。**
    ///   **重ならなければ、分けても在庫の1本を順に使い回すだけで増えない。**
    ///
    /// # 実測（G-3a の是正2周目 / 2026-09-07）
    ///
    /// 見張り番は `tests/server_sub_llm_endpoint.rs` の
    /// `the_unset_extraction_client_shares_the_body_clients_connection_pool`。
    ///
    /// | 変異 | 結果 |
    /// |---|---|
    /// | `m14`（`server::App::bind` の未設定の枝を `LmStudioClient::new(llm.with_model(…).config().clone())` へ） | **1 赤 / exit 101**（`left: 3 / right: 2`。**10 回とも赤**） |
    /// | `m14_noguard`（`m14` ＋ その最後の `assert_eq!` を外す） | **1969 全緑 / exit 0** |
    ///
    /// **後者があるので「赤にしているのは隣のアサートではない」と言える**
    /// （CLAUDE.md「是正を入れたら、その是正を外して緑に戻ることまで測る」）。
    ///
    /// # ⚠ 見ているのは本数だけ
    ///
    /// **その見張り番が数えるのは `accept` した接続の総数**であって、
    /// **「どちらのクライアントがどの接続を使ったか」までは見ていない。**
    /// **`m14` を赤にできるのは、分けた側で総数が1本増えるから**である。
    #[must_use]
    pub const fn sub_llm(&self) -> Option<&LmStudioConfig> {
        self.sub_llm.as_ref()
    }

    /// **裏方が実際に話す先の URL。** 未設定なら本文と同じものを返す。
    ///
    /// **起動ログが「どちらの上流の話か」を名乗るための口**（ADR-0039 §決定4）。
    /// **`sub_llm()` の `Option` をそのまま出さない**のは、
    /// **ログに「未設定」と出しても運用者には何も分からない**からである
    /// ——**知りたいのは「抽出はどこへ行くのか」**であって、設定の有無ではない。
    ///
    /// **見張り番は `the_startup_line_names_the_extraction_upstream_apart_from_the_body_one`**
    /// （**設定した腕と未設定の腕を1本に置く**——片方だけだと、
    /// 「常に本文の綴りを出す」実装も「常に抽出の綴りを出す」実装も片側で緑になる）。
    #[must_use]
    pub fn sub_llm_endpoint(&self) -> &str {
        match &self.sub_llm {
            Some(sub) => sub.endpoint(),
            None => self.llm.endpoint(),
        }
    }

    /// **明示設定されたトークン予算。** 未設定なら `None`。
    ///
    /// `None` は「既定値を使う」ではなく「**まだ決まっていない**」の意味で、
    /// 上流への照会（ADR-0013 第2層の 2 段目）へ進む合図になる。
    #[must_use]
    pub const fn context_budget(&self) -> Option<TokenBudget> {
        self.context_budget
    }

    /// **性格ステータスの変容の走らせ方**（ADR-0024 §8 / ADR-0026 §決定1）。
    /// **既定は [`StatDriftMode::Off`]。**
    ///
    /// off 以外にすると、抽出の直後に**サブモデルへの2回目の呼び出し**が走る
    /// （`server::drift`）。**本文の応答には一切影響しない**（裏方のタスクの中）。
    ///
    /// **値を動かすのは [`StatDriftMode::On`] だけ**で、
    /// [`StatDriftMode::Shadow`] は判定して票を残すだけである。
    #[must_use]
    pub const fn stat_drift(&self) -> StatDriftMode {
        self.stat_drift
    }

    /// **変容の依頼の形**（ADR-0027 §12 / [`ENV_DRIFT_FORM`]）。**既定は `all`。**
    ///
    /// `stat_drift` が off ならこの値は使われない（判定そのものが走らない）。
    /// **それでも読み取りは独立している**——2つのスイッチを1つに畳まない理由は
    /// [`ENV_DRIFT_FORM`] の doc。
    #[must_use]
    pub const fn drift_form(&self) -> DriftForm {
        self.drift_form
    }

    /// **変容の依頼に足す注記**（ADR-0029 §訂正 / [`ENV_DRIFT_NOTE`]）。**既定は `none`。**
    ///
    /// `stat_drift` が off ならこの値は使われない（判定そのものが走らない）。
    /// **それでも読み取りは独立している**——理由は [`ENV_DRIFT_NOTE`] の doc。
    #[must_use]
    pub const fn drift_note(&self) -> DriftNote {
        self.drift_note
    }

    /// **変容の集約のつまみ**（ADR-0027 §決定8 / §9）。**既定は ADR の「当面の値」。**
    ///
    /// `stat_drift` が off ならこの値は使われない（判定そのものが走らない）。
    #[must_use]
    pub const fn drift_tuning(&self) -> DriftTuning {
        self.drift_tuning
    }

    /// **軸の候補を尋ねる間隔 `N`**（ADR-0046 §決定2 / [`ENV_AXIS_DISCOVERY_MEMORIES`]）。
    ///
    /// 枠の数が 0 のキャラクター・変容の走らせ方が off のキャラクターでは使われない
    /// （候補づくりそのものが走らない / `server::axis_discovery`）。
    #[must_use]
    pub const fn axis_discovery_memories(&self) -> AxisDiscoveryMemories {
        self.axis_discovery_memories
    }

    /// **軸の候補の試用の判定のつまみ `T` / `K` / 線**（ADR-0046 §決定4 / [`ENV_AXIS_TRIAL_TURNS`] ほか）。
    ///
    /// 枠の数が 0 のキャラクター・変容の走らせ方が off のキャラクターでは使われない
    /// （試用そのものが走らない / `server::axis_trial`）。
    #[must_use]
    pub const fn axis_trial(&self) -> AxisTrialRule {
        self.axis_trial
    }
}

/// **試用の判定のつまみを読む**（[`ENV_AXIS_TRIAL_TURNS`] / [`ENV_AXIS_TRIAL_MIN_NONZERO`] /
/// [`ENV_AXIS_TRIAL_LINE_PERCENT`]）。
///
/// 1つずつ: 未設定・空・空白のみはそのつまみの既定、**読めない綴りは起動を止める**。
/// **範囲と組の検証は `AxisTrialRule::new` に任せる**（止める場所を1つにする）。
fn axis_trial_rule<F>(lookup: &F) -> Result<AxisTrialRule, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let turns = axis_trial_knob(
        lookup,
        ENV_AXIS_TRIAL_TURNS,
        AxisTrialRule::DEFAULT_TURNS,
        AxisTrialRuleError::Turns,
    )?;
    let min_nonzero = axis_trial_knob(
        lookup,
        ENV_AXIS_TRIAL_MIN_NONZERO,
        AxisTrialRule::DEFAULT_MIN_NONZERO,
        AxisTrialRuleError::MinNonzero,
    )?;
    let line_percent = axis_trial_knob(
        lookup,
        ENV_AXIS_TRIAL_LINE_PERCENT,
        AxisTrialRule::DEFAULT_LINE_PERCENT,
        AxisTrialRuleError::Line,
    )?;
    AxisTrialRule::new(turns, min_nonzero, line_percent).map_err(|err| {
        // **組の不整合は、両方の値を欄で名指しする**（A-9c のレビュー提案4）。
        // 片方だけ設定した回にも起きるので、設定していない方の値（既定）まで出さないと、
        // 触っていないつまみを疑わせる。**値は `Display` に載せない**（定数文字列 / 絶対制約）。
        if err == AxisTrialRuleError::MinNonzeroAboveTurns {
            tracing::error!(
                axis_trial_turns = turns,
                axis_trial_min_nonzero = min_nonzero,
                turns_variable = ENV_AXIS_TRIAL_TURNS,
                min_nonzero_variable = ENV_AXIS_TRIAL_MIN_NONZERO,
                "軸の候補の試用の標本不足の線がターン数を超えています"
            );
        }
        ConfigError::AxisTrial(err)
    })
}

/// 試用のつまみを1つ読む。**読めない綴りは `unreadable` の理由で止める**（範囲は見ない）。
fn axis_trial_knob<F>(
    lookup: &F,
    name: &str,
    default: u32,
    unreadable: AxisTrialRuleError,
) -> Result<u32, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = lookup(name) else {
        return Ok(default);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(default);
    }
    trimmed
        .parse::<u32>()
        .map_err(|_| ConfigError::AxisTrial(unreadable))
}

/// **軸の候補を尋ねる間隔を読む**（[`ENV_AXIS_DISCOVERY_MEMORIES`]）。
///
/// 未設定・空・空白のみは既定。**読めない綴りと 0 は起動を止める**（黙って既定へ落とさない）。
/// 0 を拒むのは `AxisDiscoveryMemories::parse`（**止める場所を1つにする**）。
fn axis_discovery_memories<F>(lookup: &F) -> Result<AxisDiscoveryMemories, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = lookup(ENV_AXIS_DISCOVERY_MEMORIES) else {
        return Ok(AxisDiscoveryMemories::DEFAULT);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(AxisDiscoveryMemories::DEFAULT);
    }
    trimmed
        .parse::<u32>()
        .ok()
        .and_then(|count| AxisDiscoveryMemories::parse(count).ok())
        .ok_or(ConfigError::AxisDiscoveryMemories)
}

/// **集約のつまみを1つ読む**（ADR-0027 §決定8）。
///
/// # 綴りの検証をどこまでやるか（16e-b の判断）
///
/// **形式の検証は `f64` の `parse` に任せ、値の検証は `DriftTuning::new` に任せる。**
/// この関数が足すのは**前後の空白を落とすことと、空文字列を既定にすること**だけである。
///
/// | 入力 | 結果 |
/// |---|---|
/// | 未設定 / 空 / 空白のみ | **既定値**（`PERSONACORE_DRIFT_FORM=` と揃える） |
/// | `1.5` / `+0.75` / `1e-1` | そのまま受ける |
/// | `1.5.0` / `1,5` / `abc` | [`ConfigError::DriftTuning`]（**起動を止める**） |
/// | `inf` / `nan` | **`parse` は通る。** 範囲の検証（`DriftTuning::new`）が止める |
///
/// **閉じた語彙にしない。** 他のスイッチ（`single` / `all` / `shadow`）は
/// **意味のある名前が有限個**なので語彙で縛れるが、こちらは連続量である。
/// 語彙で縛ると**測りたい値が書けなくなる。**
///
/// **`inf` / `nan` を綴りの段階で弾かない**のは、
/// **止める場所を1つにするため**——範囲の検証は `DriftTuning::new` にしか無く、
/// ここにも書くと**写しが2つになる**（CLAUDE.md「同じ発見を2箇所に書かない」）。
/// どちらの経路でも起動は止まる。
/// # どのつまみが読めなかったかを落とさない
///
/// `unreadable` を呼び出し側から受け取るのは、**`#[source]` 側に
/// 「どのつまみか」を残すため**である。1つに畳むと、
/// 3つ同時に指定した起動で**どれを直せばいいかが応答にもログにも出ない。**
fn drift_knob<F>(
    lookup: &F,
    key: &str,
    default: f64,
    unreadable: DriftTuningError,
) -> Result<f64, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = lookup(key) else {
        return Ok(default);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(default);
    }
    trimmed
        .parse::<f64>()
        .map_err(|_| ConfigError::DriftTuning(unreadable))
}

/// **打ち切りの天井を秒で読む**（ADR-0035 §決定3 / [`ENV_LLM_IDLE_TIMEOUT`]）。
///
/// | 入力 | 戻り値 |
/// |---|---|
/// | 未設定 / 空 / 空白のみ | `Ok(None)`（＝既定のまま。**呼び出し側は何も差し替えない**） |
/// | `45` | `Ok(Some(45秒))` |
/// | それ以外 | [`ConfigError::LlmIdleTimeout`] |
///
/// # なぜ `Option` を返すのか
///
/// 「既定値」をここで作ると、**既定が `llm` 層とこの関数の2箇所に写る**
/// （CLAUDE.md「同じ発見を2箇所に書かない」）。
/// **既定は `LmStudioConfig::new` にしか無い**まま、
/// **明示されたときだけ差し替える**形にしてある。
///
/// # `0` を弾く —— **線は「使えるかどうか」には引いていない**
///
/// **書式としては妥当。** それでも弾くのは、**導出した接続の上限まで 0 に潰れて、
/// 上流への接続を1度も試みなくなる**からである
/// （`with_idle_budget` は `connect_timeout` を `min(既定, 天井/2)` で導く）。
///
/// **⚠ 「1ターンも成立しない」は理由にならない。それは `1` でも同じ**
/// （レビュー指摘で訂正 / B-6）。**線が `0` にあるのは
/// 「即座に失敗する」と「待って失敗する」という質的な違い**であって、
/// **使える値かどうかではない。** `1` は遅い機では役に立たないが、
/// **待ってから失敗する**ので ADR-0036 §決定1 では「遅い」側に落ちる。
///
/// **上端は縛らない。** 大きすぎる天井が起こすのは「待たされる」だけで、
/// **それは利用者が決めること**（ADR-0035 §決定4）。
fn llm_idle_timeout<F>(lookup: &F) -> Result<Option<Duration>, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(raw) = lookup(ENV_LLM_IDLE_TIMEOUT) else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let secs: u64 = trimmed.parse().map_err(|_| ConfigError::LlmIdleTimeout)?;
    if secs == 0 {
        return Err(ConfigError::LlmIdleTimeout);
    }
    Ok(Some(Duration::from_secs(secs)))
}

/// 依頼の形の綴りを読む。**純粋関数。**
///
/// 前後の空白を落として小文字に畳んでから、**閉じた2つの語彙**と照合する。
/// どれにも無ければ [`ConfigError::DriftForm`]（黙ってどちらかへ落とさない）。
fn parse_drift_form(raw: &str) -> Result<DriftForm, ConfigError> {
    let normalized = raw.trim().to_ascii_lowercase();
    if DRIFT_FORM_ALL.contains(&normalized.as_str()) {
        return Ok(DriftForm::All);
    }
    if DRIFT_FORM_SINGLE.contains(&normalized.as_str()) {
        return Ok(DriftForm::Single);
    }
    Err(ConfigError::DriftForm)
}

/// 注記の綴りを読む。**純粋関数。**
///
/// 前後の空白を落として小文字に畳んでから、**閉じた3つの語彙**と照合する。
/// どれにも無ければ [`ConfigError::DriftNote`]（黙って腕 X へ落とさない）。
///
/// **自由文を受けない**——受ける綴りは `none` / `copy` / `language` の3つだけで、
/// **文面は `prompt::drift` が持つ**（ADR-0029 §決定4）。
fn parse_drift_note(raw: &str) -> Result<DriftNote, ConfigError> {
    let normalized = raw.trim().to_ascii_lowercase();
    if DRIFT_NOTE_COPY.contains(&normalized.as_str()) {
        return Ok(DriftNote::Copy);
    }
    if DRIFT_NOTE_LANGUAGE.contains(&normalized.as_str()) {
        return Ok(DriftNote::Language);
    }
    if DRIFT_NOTE_NONE.contains(&normalized.as_str()) {
        return Ok(DriftNote::None);
    }
    Err(ConfigError::DriftNote)
}

/// 変容スイッチの綴りを読む。**純粋関数。**
///
/// 前後の空白を落として小文字に畳んでから、**閉じた3つの語彙**と照合する。
/// どれにも無ければ [`ConfigError::StatDrift`]（黙って off へ落とさない）。
///
/// **語彙が重ならないこと**は `the_three_stat_drift_vocabularies_never_overlap` が見張る
/// ——重なると、読む順がそのまま意味になる。
fn parse_stat_drift(raw: &str) -> Result<StatDriftMode, ConfigError> {
    let normalized = raw.trim().to_ascii_lowercase();
    if STAT_DRIFT_ON.contains(&normalized.as_str()) {
        return Ok(StatDriftMode::On);
    }
    if STAT_DRIFT_SHADOW.contains(&normalized.as_str()) {
        return Ok(StatDriftMode::Shadow);
    }
    if STAT_DRIFT_OFF.contains(&normalized.as_str()) {
        return Ok(StatDriftMode::Off);
    }
    Err(ConfigError::StatDrift)
}

/// 明示設定のコンテキスト長を予算に変える。**純粋関数。**
///
/// 生成用の余白は既定のまま据え置き、コンテキスト全体だけを差し替える
/// （余白まで設定で動かすと「応答が途中で切れる」設定が作れてしまう）。
fn parse_context_budget(raw: &str) -> Result<TokenBudget, ConfigError> {
    let tokens: u32 = raw.trim().parse().map_err(|_| ConfigError::ContextTokens)?;
    context_budget_of(tokens)
}

/// **明示のコンテキスト長（数）を予算に変える。純粋関数。**
///
/// **`PERSONACORE_CONTEXT_TOKENS` と、上流プロファイルの面の `context_tokens`
/// （ADR-0054 §決定1）が同じ規則を通る**——規則の写しを `upstreams` に作らない。
///
/// 0 も、余白（1024）以下も、ここで `BudgetError` になって落ちる。
pub(in crate::server) fn context_budget_of(tokens: u32) -> Result<TokenBudget, ConfigError> {
    TokenBudget::new(tokens, TokenBudget::DEFAULT_RESPONSE_TOKENS)
        .map_err(|_| ConfigError::ContextTokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与えた組だけを返す参照関数を作る。
    fn lookup_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        }
    }

    /// **テストが植えるデータディレクトリ。**
    ///
    /// **この機の既定（`%LOCALAPPDATA%` 等）とは必ず違う綴りにすること**
    /// ——同じだと「引いてきた」と「決め打った」が区別できない
    /// （CLAUDE.md「実行時に決まる値を入れたら、機械の既定と違う状態で回る腕を必ず1本」）。
    const PLANTED_DATA_DIR: &str = "Z:\\planted-data-dir-for-f4a";

    fn config_of(pairs: &[(&str, &str)]) -> Result<AppConfig, ConfigError> {
        AppConfig::from_lookup_with_data_dir(
            lookup_of(pairs),
            Some(PathBuf::from(PLANTED_DATA_DIR)),
        )
    }

    /// 未設定なら既定値。
    ///
    /// **DB は「植えたデータディレクトリの下」**（ADR-0041 §決定3）。
    /// **期待値は [`database_path_under`] から作らない**——作ると、
    /// 組み立ての規則を変える変異と一緒に期待値が動く（CLAUDE.md `16b`）。
    #[test]
    fn falls_back_to_defaults() {
        let config = config_of(&[]).expect("既定値だけで組み立てられること");

        assert_eq!(config.bind(), BindAddr::loopback(DEFAULT_PORT));
        assert_eq!(
            config.database_path(),
            Path::new("Z:\\planted-data-dir-for-f4a")
                .join("personacore")
                .join("personacore.sqlite")
        );
        assert_eq!(
            config.llm().endpoint(),
            "http://127.0.0.1:1234/v1/chat/completions"
        );
        assert_eq!(config.llm().model().as_str(), DEFAULT_MAIN_MODEL);
    }

    /// 設定されていれば上書きされる。
    ///
    /// **本文モデルと抽出モデルには別の値を与えること。** 同じ綴りにすると、
    /// 片方をもう片方から読む取り違えが検出できない
    /// （CLAUDE.md「フィクスチャが2つの値を同値にすると…」）。
    #[test]
    fn reads_every_item_from_the_lookup() {
        let config = config_of(&[
            (ENV_PORT, "9001"),
            (ENV_DATABASE_PATH, "F:\\data\\pc.sqlite"),
            (ENV_LLM_ENDPOINT, "http://127.0.0.1:5555/"),
            (ENV_MAIN_MODEL, "ministral-3-3b-instruct-2512"),
            (ENV_SUB_MODEL, "ministral-3-3b-reasoning-2512"),
        ])
        .expect("すべて正当な値であること");

        assert_eq!(config.bind().port(), 9001);
        assert_eq!(config.database_path(), Path::new("F:\\data\\pc.sqlite"));
        assert_eq!(
            config.llm().endpoint(),
            "http://127.0.0.1:5555/v1/chat/completions"
        );
        assert_eq!(
            config.llm().model().as_str(),
            "ministral-3-3b-instruct-2512"
        );
        assert_eq!(
            config.sub_model().as_str(),
            "ministral-3-3b-reasoning-2512",
            "抽出モデルは本文モデルとは別の設定項目であること"
        );
        assert_ne!(
            config.sub_model().as_str(),
            config.llm().model().as_str(),
            "前提: 2つの設定に別の値を与えていること"
        );
    }

    // ------------------------------------------------------------------
    // 待ち受けのポートの効く順と出どころ（ADR-0057 §決定5 / 作業単位 F-12a）
    // ------------------------------------------------------------------

    /// 環境変数に書く番号。**保存の番号とも既定とも違う。**
    const ENV_PORT_VALUE: u16 = 9101;

    /// 保存した番号。**環境変数の番号とも既定とも違う。**
    const SAVED_PORT_VALUE: u16 = 9202;

    fn saved() -> SavedPort {
        SavedPort::new(SAVED_PORT_VALUE).expect("前提: 範囲の中の番号であること")
    }

    /// **フィクスチャの3つの番号は全部違う**（同じだと取り違えが観測できない）。
    #[test]
    fn the_three_port_fixtures_differ() {
        assert_ne!(ENV_PORT_VALUE, SAVED_PORT_VALUE);
        assert_ne!(ENV_PORT_VALUE, DEFAULT_PORT);
        assert_ne!(SAVED_PORT_VALUE, DEFAULT_PORT);
    }

    /// **効く順は 環境変数 → 保存 → 既定**（純粋関数の表）。
    #[test]
    fn the_port_comes_from_the_environment_then_the_saved_value_then_the_default() {
        let env = ENV_PORT_VALUE.to_string();
        assert_eq!(
            resolve_port(Some(&env), Some(saved())).ok(),
            Some((ENV_PORT_VALUE, PortSource::Environment)),
            "環境変数が保存に勝つこと"
        );
        assert_eq!(
            resolve_port(Some(&env), None).ok(),
            Some((ENV_PORT_VALUE, PortSource::Environment))
        );
        assert_eq!(
            resolve_port(None, Some(saved())).ok(),
            Some((SAVED_PORT_VALUE, PortSource::Saved)),
            "保存が既定に勝つこと"
        );
        assert_eq!(
            resolve_port(None, None).ok(),
            Some((DEFAULT_PORT, PortSource::Default))
        );
        // **前後の空白は落とす**（今までの `PERSONACORE_PORT` の読み方と同じ）。
        assert_eq!(
            resolve_port(Some(" 9101\t"), Some(saved())).ok(),
            Some((ENV_PORT_VALUE, PortSource::Environment))
        );
        // **`0`（OS に選ばせる）は今までどおり環境変数の値として通る。**
        assert_eq!(
            resolve_port(Some("0"), Some(saved())).ok(),
            Some((0, PortSource::Environment))
        );
    }

    /// **読めない環境変数は、保存へ落ちずに止まる**（明示の誤りは今までどおり起動を止める）。
    #[test]
    fn an_unreadable_environment_port_stops_instead_of_falling_to_the_saved_value() {
        for raw in ["", "abc", "65536", "-1", "0.0.0.0:8080"] {
            assert!(
                matches!(
                    resolve_port(Some(raw), Some(saved())),
                    Err(ConfigError::Port)
                ),
                "{raw:?} を通している"
            );
        }
    }

    /// **保存した番号は 1024〜65535 だけ**（ADR-0057 §決定5）。上端・下端の内外を見る。
    #[test]
    fn a_saved_port_is_only_from_1024_to_65535() {
        assert_eq!(SavedPort::new(0), None);
        assert_eq!(SavedPort::new(1023), None);
        assert_eq!(SavedPort::new(1024).map(SavedPort::get), Some(1024));
        assert_eq!(SavedPort::new(65535).map(SavedPort::get), Some(65535));
        for (value, expected) in [
            (-1_i64, None),
            (0, None),
            (1023, None),
            (1024, Some(1024_u16)),
            (65535, Some(65535)),
            (65536, None),
            (i64::MAX, None),
            (i64::MIN, None),
        ] {
            assert_eq!(
                SavedPort::from_integer(value).map(SavedPort::get),
                expected,
                "{value}"
            );
        }
    }

    /// **出どころの綴り**（関数の外の値で押さえる / 線の上の綴り）。
    #[test]
    fn the_port_source_labels_are_the_spellings_on_the_wire() {
        assert_eq!(
            PortSource::ALL.map(PortSource::label),
            ["environment", "saved", "default"]
        );
    }

    fn config_with_saved(
        pairs: &[(&str, &str)],
        saved: Option<SavedPort>,
    ) -> Result<AppConfig, ConfigError> {
        AppConfig::from_lookup_with_data_dir_and_saved_port(
            lookup_of(pairs),
            Some(PathBuf::from(PLANTED_DATA_DIR)),
            saved,
        )
    }

    /// **保存した番号は `from_lookup` 系の同じ入口から入り、出どころと一緒に `AppConfig` に載る。**
    ///
    /// **`PERSONACORE_PORT` の値として混ぜない**——混ぜると出どころが `environment` になる。
    #[test]
    fn the_saved_port_enters_through_the_same_door_and_keeps_its_source() {
        let env = ENV_PORT_VALUE.to_string();

        let both = config_with_saved(&[(ENV_PORT, &env)], Some(saved())).expect("組めること");
        assert_eq!(both.bind(), BindAddr::loopback(ENV_PORT_VALUE));
        assert_eq!(both.port_source(), PortSource::Environment);

        let only_saved = config_with_saved(&[], Some(saved())).expect("組めること");
        assert_eq!(only_saved.bind(), BindAddr::loopback(SAVED_PORT_VALUE));
        assert_eq!(only_saved.port_source(), PortSource::Saved);

        let neither = config_with_saved(&[], None).expect("組めること");
        assert_eq!(neither.bind(), BindAddr::loopback(DEFAULT_PORT));
        assert_eq!(neither.port_source(), PortSource::Default);

        // **保存を渡さない入口は今までどおり**（出どころも言う）。
        let plain = config_of(&[(ENV_PORT, &env)]).expect("組めること");
        assert_eq!(plain.port_source(), PortSource::Environment);
        assert_eq!(
            config_of(&[]).expect("組めること").port_source(),
            PortSource::Default
        );

        // **読めない環境変数は、保存があっても止まる。**
        assert!(matches!(
            config_with_saved(&[(ENV_PORT, "abc")], Some(saved())),
            Err(ConfigError::Port)
        ));
    }

    /// **探す計画は設定の番号・出どころ・飛ばす番号から作る。** 明示した番号は列が1つ、保存した番号は
    /// 11 個で面の番号を飛ばす。`with_bind` は出どころを変えないので、`default` のまま列が 11 個になる。
    /// 事前検査に渡すアドレス（`explicit_bind`）は明示のときだけ（作業単位 F-11b）。
    #[test]
    fn the_port_search_follows_the_source_and_the_face_ports() {
        use super::super::port_search::PortSearch;

        let env = ENV_PORT_VALUE.to_string();
        let explicit = config_of(&[(ENV_PORT, &env)])
            .expect("組めること")
            .with_face_ports([ENV_PORT_VALUE]);
        assert_eq!(
            explicit.port_search(),
            PortSearch::plan(ENV_PORT_VALUE, PortSource::Environment, &BTreeSet::new())
        );
        assert_eq!(
            explicit.explicit_bind(),
            Some(BindAddr::loopback(ENV_PORT_VALUE))
        );

        let faces = BTreeSet::from([SAVED_PORT_VALUE + 1]);
        let from_saved = config_with_saved(&[], Some(saved()))
            .expect("組めること")
            .with_face_ports(faces.clone());
        assert_eq!(
            from_saved.port_search(),
            PortSearch::plan(SAVED_PORT_VALUE, PortSource::Saved, &faces)
        );
        assert_ne!(
            from_saved.port_search(),
            PortSearch::plan(SAVED_PORT_VALUE, PortSource::Saved, &BTreeSet::new()),
            "前提: 飛ばす番号が列を変えること"
        );
        assert_eq!(from_saved.explicit_bind(), None);

        // **`with_bind` は出どころを変えない**（`default` のまま、塞がっていればずれる）。
        let rebound = config_of(&[])
            .expect("組めること")
            .with_bind(BindAddr::loopback(9400));
        assert_eq!(
            rebound.port_search(),
            PortSearch::plan(9400, PortSource::Default, &BTreeSet::new())
        );
        assert_eq!(rebound.explicit_bind(), None);

        // **渡し直すと置き換える**（足し合わせない）。
        let replaced = config_of(&[])
            .expect("組めること")
            .with_face_ports([8081])
            .with_face_ports([8083]);
        assert_eq!(
            replaced.port_search(),
            PortSearch::plan(DEFAULT_PORT, PortSource::Default, &BTreeSet::from([8083]))
        );
    }

    /// **範囲が全部だめだった文は、範囲・`PERSONACORE_PORT`・画面の「設定」を言う。** 1つの番号の文
    /// （[`port_in_use_message`]）とは違う文で、範囲が1つの番号なら「〜」を書かない。
    #[test]
    fn the_range_message_names_the_range_and_both_ways_out() {
        let message = ports_in_use_message(18_090, 18_100, &[]);
        assert!(
            message.starts_with(
                "ポート 18090〜18100 は、どれも他のプログラムが使っているか、使えない番号でした。"
            ),
            "{message}"
        );
        // **飛ばしていないのに、面の番号を試していないと言わない**（読めない `upstreams.json` の起動）。
        assert!(!message.contains("llama"), "{message}");
        assert!(message.contains(ENV_PORT), "{message}");
        assert!(message.contains("画面の「設定」"), "{message}");
        assert_ne!(message, port_in_use_message(18_090));

        let single = ports_in_use_message(65535, 65535, &[]);
        assert!(
            single.starts_with("ポート 65535 は、他のプログラムが"),
            "{single}"
        );
        assert!(!single.contains('〜'), "{single}");
    }

    /// **飛ばした番号が在るときだけ、それを除いたと言う。1つも試していなければ、試せる番号が無かったと言い、
    /// 「他のプログラムが使っている」と言わない**（F-11b のレビュー是正）。
    #[test]
    fn the_range_message_says_what_was_skipped_and_never_blames_an_untried_port() {
        let some = ports_in_use_message(18_090, 18_100, &[18_091, 18_095]);
        assert!(
            some.starts_with(
                "ポート 18090〜18100 は、上流プロファイルの llama の面が使う番号（18091・18095）を除いて、\
                 どれも他のプログラムが使っているか、使えない番号でした。"
            ),
            "{some}"
        );

        // **保存した番号が 65535 で、それが面の番号**——列は1つで、それも飛ばした。
        let top = ports_in_use_message(65535, 65535, &[65535]);
        assert!(
            top.starts_with(
                "ポート 65535 は、上流プロファイルの llama の面が使う番号なので、試せる番号がありませんでした。"
            ),
            "{top}"
        );
        assert!(!top.contains("他のプログラム"), "{top}");
        assert!(top.contains(ENV_PORT), "{top}");

        let all: Vec<u16> = (18_090..=18_100).collect();
        let every = ports_in_use_message(18_090, 18_100, &all);
        assert!(
            every.starts_with(
                "ポート 18090〜18100 は、どれも上流プロファイルの llama の面が使う番号なので、"
            ),
            "{every}"
        );
        assert!(!every.contains("他のプログラム"), "{every}");

        // **試したのが1つだけなら「どれも」と言わない。**
        let one_tried = ports_in_use_message(65534, 65535, &[65535]);
        assert!(
            one_tried.starts_with(
                "ポート 65534〜65535 は、上流プロファイルの llama の面が使う番号（65535）を除いて、他の"
            ),
            "{one_tried}"
        );
    }

    /// **抽出モデルの既定は本文モデルの既定と同じ綴り**（ADR-0021 §10 / 2026-08-21 の実測）。
    ///
    /// # 何が撤回されたか
    ///
    /// 旧 `the_sub_model_defaults_to_the_small_one` は「抽出の既定と本文の既定は
    /// **別のモデルである**」を主張していた（14B を裏方に使うと本文のメモリを奪う、が理由）。
    /// **実測で撤回された。** 3B は 2.2 倍速いが、**主語を約 10/41 で間違え、形式も 41% 崩れる**
    /// （14B は 0/65・0%）。**`memory` に更新も削除も無い**ので、遅さはその回だけの費用だが、
    /// 間違った記憶は永久に残る。同じ綴りなら上流が2つ目のモデルを載せずに済む利得もある。
    ///
    /// # なぜ「2つの定数が一致している」ことまで主張するのか
    ///
    /// 既定どうしが同じ綴りになった以上、`sub_model()` が `DEFAULT_SUB_MODEL` と
    /// `DEFAULT_MAIN_MODEL` のどちらから来ても**この経路では区別が付かない**。
    /// せめて**片方だけが動く変更**（本文の既定モデルの差し替え、抽出だけ小型へ戻す等）を
    /// 赤にできるよう、一致していること自体を固定する。
    /// **設定で別々に届くこと**は `reads_every_item_from_the_lookup` が、
    /// **配線が別々であること**は `the_sub_model_reaches_the_upstream_through_the_real_bind`
    /// （`tests/server_memory_extraction.rs`）が、どちらも**別の綴りを与えて**見ている。
    #[test]
    fn the_sub_model_defaults_to_the_same_spelling_as_the_main_model() {
        let config = config_of(&[]).expect("既定値だけで組み立てられること");

        assert_eq!(config.sub_model().as_str(), DEFAULT_SUB_MODEL);
        assert_eq!(
            DEFAULT_SUB_MODEL, DEFAULT_MAIN_MODEL,
            "抽出の既定は本文の既定と同じ綴りであること（ADR-0021 §10）"
        );
        assert_eq!(
            config.sub_model().as_str(),
            config.llm().model().as_str(),
            "既定のままなら2つは同じモデルであること"
        );
    }

    /// **2つ目の上流は、明示設定されたときだけ組む**（ADR-0039 §決定1）。
    ///
    /// # 3つの値を全部違えてある
    ///
    /// 本文のエンドポイント・抽出のエンドポイント・既定の3つが**互いに別の綴り。**
    /// 同値にすると「設定を読まずに本文のものを使う」実装と区別が付かない
    /// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
    ///
    /// # モデルの綴りも一緒に見る
    ///
    /// 2つ目の上流に**本文のモデル**が載る取り違えは、
    /// **既定どうしが同じ綴り**（ADR-0021 §10）なので既定では観測できない。
    /// **ここは2つのモデルにも別の綴りを与えてある。**
    #[test]
    fn the_sub_llm_endpoint_builds_a_second_upstream_only_when_it_is_set() {
        const MAIN_ENDPOINT: &str = "http://127.0.0.1:5555";
        const SUB_ENDPOINT: &str = "http://127.0.0.1:6666";
        const MAIN_MODEL: &str = "ministral-3-3b-instruct-2512";
        const SUB_MODEL: &str = "ministral-3-3b-reasoning-2512";

        assert_ne!(MAIN_ENDPOINT, SUB_ENDPOINT, "前提: 2つは別の綴りであること");
        assert_ne!(
            MAIN_MODEL, SUB_MODEL,
            "前提: 2つのモデルも別の綴りであること"
        );
        for endpoint in [MAIN_ENDPOINT, SUB_ENDPOINT] {
            assert_ne!(
                endpoint, DEFAULT_ENDPOINT,
                "前提: 既定とも別の綴りであること（既定だと設定を読まない実装と区別できない）"
            );
        }

        // ---- 設定した腕。**2つ目のクライアントの設定が組まれる。** ----
        let config = config_of(&[
            (ENV_LLM_ENDPOINT, MAIN_ENDPOINT),
            (ENV_SUB_LLM_ENDPOINT, SUB_ENDPOINT),
            (ENV_MAIN_MODEL, MAIN_MODEL),
            (ENV_SUB_MODEL, SUB_MODEL),
        ])
        .expect("すべて正当な値であること");

        let sub = config.sub_llm().expect("明示設定なら2つ目の上流を持つこと");
        assert_eq!(
            sub.endpoint(),
            "http://127.0.0.1:6666/v1/chat/completions",
            "抽出は指定された上流を向くこと"
        );
        assert_eq!(
            config.llm().endpoint(),
            "http://127.0.0.1:5555/v1/chat/completions",
            "本文は本文の上流を向いたままであること"
        );
        assert_ne!(
            sub.endpoint(),
            config.llm().endpoint(),
            "2つの上流が同じ綴りになっていない（取り違えが観測できなくなる）"
        );
        assert_eq!(
            sub.model().as_str(),
            SUB_MODEL,
            "2つ目の上流に載るのは抽出モデルであること"
        );
        assert_eq!(
            config.sub_llm_endpoint(),
            sub.endpoint(),
            "実際に話す先は2つ目の上流であること"
        );

        // ---- 未設定の腕。**2つ目は組まない**（＝本文と同じ上流）。 ----
        let single = config_of(&[
            (ENV_LLM_ENDPOINT, MAIN_ENDPOINT),
            (ENV_MAIN_MODEL, MAIN_MODEL),
            (ENV_SUB_MODEL, SUB_MODEL),
        ])
        .expect("すべて正当な値であること");

        assert!(
            single.sub_llm().is_none(),
            "未設定なら2つ目の上流を組まないこと（既定の振る舞いを1バイトも変えない）"
        );
        assert_eq!(
            single.sub_llm_endpoint(),
            single.llm().endpoint(),
            "未設定なら抽出は本文と同じ上流へ行くこと"
        );
    }

    /// **空文字列は「未設定」**（[`ENV_SUB_LLM_ENDPOINT`] / `ENV_WEB_DIR` と同じ側）。
    ///
    /// **`PERSONACORE_SUB_LLM_ENDPOINT=` を置いた compose ファイルで
    /// 起動そのものを失敗させない。** 空を「その値」として読むと
    /// `LmStudioConfig::new("")` が `EndpointError::Empty` を返して起動が止まる。
    #[test]
    fn an_empty_sub_llm_endpoint_reads_as_unset() {
        for raw in ["", " ", "\t", "\r\n"] {
            let config = config_of(&[(ENV_SUB_LLM_ENDPOINT, raw)]).unwrap_or_else(|err| {
                panic!("空（{raw:?}）で起動を止めないこと: {err}");
            });
            assert!(
                config.sub_llm().is_none(),
                "空（{raw:?}）は未設定として読むこと"
            );
            assert_eq!(
                config.sub_llm_endpoint(),
                config.llm().endpoint(),
                "空（{raw:?}）なら抽出は本文と同じ上流へ行くこと"
            );
        }

        // **対照: 空白で囲んだ正当な値は、空白を落として受ける。**
        // これが無いと「何を渡しても未設定として読む」実装が上を満たして緑になる。
        let trimmed = config_of(&[(ENV_SUB_LLM_ENDPOINT, "  http://127.0.0.1:6666  ")])
            .expect("空白を除けば正当であること");
        assert_eq!(
            trimmed
                .sub_llm()
                .expect("空白だけではないので2つ目の上流を持つこと")
                .endpoint(),
            "http://127.0.0.1:6666/v1/chat/completions"
        );
    }

    /// **不正な2つ目のエンドポイントは起動を止める**（黙って本文と同じ上流へ落とさない）。
    ///
    /// 落とすと、**利用者は2つ目を指したつもりで1つ目に全部行っている状態**を
    /// ログのどこからも区別できない。`ENV_LLM_ENDPOINT` と同じ作法
    /// （`ConfigError::Endpoint`）。
    ///
    /// **`Display` に値を反射しないこと**も一緒に見る（絶対制約 / エラーに設定値を載せない）。
    #[test]
    fn a_malformed_sub_llm_endpoint_stops_the_startup() {
        for raw in ["ftp://127.0.0.1:1234", "127.0.0.1:1234", "ほげ"] {
            let err = config_of(&[(ENV_SUB_LLM_ENDPOINT, raw)])
                .expect_err("不正な2つ目のエンドポイントで起動を止めること");
            assert!(
                matches!(err, ConfigError::Endpoint(_)),
                "エンドポイントの不正として落ちること（{raw:?}）: {err:?}"
            );
            let message = err.to_string();
            assert!(
                !message.contains(raw),
                "指定された値が Display に漏れている（{raw:?}）: {message}"
            );
        }
    }

    /// **待ち時間のつまみは1つのまま、両方の上流に効く**（ADR-0039 §決定2）。
    ///
    /// **既定（180秒）で回さない。** 既定だと「つまみを読まずに `LmStudioConfig::new` の
    /// 既定を使う」実装と文字どおり同じ式になり、区別が付かない
    /// （CLAUDE.md / B-7「実行時に決まる値を入れたら、既定と違う状態で回る腕を1本置く」）。
    ///
    /// **導出した3つも一緒に見る。** 天井だけを写して残りを既定のままにする形
    /// （＝`with_idle_budget` ではなく `with_idle_timeout` を呼ぶ）を赤にする。
    #[test]
    fn the_second_upstream_shares_the_one_idle_timeout_knob() {
        let shipped = config_of(&[])
            .expect("既定値だけで組み立てられること")
            .llm()
            .idle_timeout();
        let ceiling = Duration::from_secs(45);
        assert_ne!(
            shipped, ceiling,
            "前提: 与える天井が既定と違うこと（既定だとつまみを読まない実装と区別できない）"
        );

        let config = config_of(&[
            (ENV_SUB_LLM_ENDPOINT, "http://127.0.0.1:6666"),
            (ENV_LLM_IDLE_TIMEOUT, "45"),
        ])
        .expect("すべて正当な値であること");

        let main = config.llm();
        let sub = config.sub_llm().expect("2つ目の上流を持つこと");
        assert_eq!(main.idle_timeout(), ceiling, "前提: 本文側に天井が載ること");
        assert_eq!(
            sub.idle_timeout(),
            ceiling,
            "2つ目の上流にも同じ天井が効くこと（つまみは1つ）"
        );
        // **導出した3つも同じ**（天井1つから導く / `with_idle_budget`）。
        assert_eq!(sub.request_timeout(), main.request_timeout());
        assert_eq!(sub.context_timeout(), main.context_timeout());
        assert_eq!(sub.connect_timeout(), main.connect_timeout());
    }

    /// **抽出モデルの綴りも起動時に検証する。**
    ///
    /// 裏方の失敗は本文の応答を壊さない（ADR-0021）ので、綴り間違いは
    /// **「記憶がいつまでも増えない」という形でしか現れない。** 起動時に落とす。
    #[test]
    fn rejects_a_malformed_sub_model() {
        assert!(matches!(
            config_of(&[(ENV_SUB_MODEL, "  ")]),
            Err(ConfigError::Model(_))
        ));
        assert!(matches!(
            config_of(&[(ENV_SUB_MODEL, "mo\ndel")]),
            Err(ConfigError::Model(_))
        ));
    }

    /// **ポート以外の待ち受け指定は受け付けない。**
    ///
    /// アドレスを書こうとした値（`0.0.0.0:8080` など）は数値として読めないので落ちる。
    /// つまり設定から外部公開にする手段が無い。
    #[test]
    fn rejects_anything_that_is_not_a_port_number() {
        for raw in [
            "0.0.0.0:8080",
            "0.0.0.0",
            "127.0.0.1:8080",
            "[::]:8080",
            "",
            "  ",
            "abc",
            "-1",
            "65536",
            "8080.0",
            "8080 8081",
        ] {
            assert!(
                matches!(config_of(&[(ENV_PORT, raw)]), Err(ConfigError::Port)),
                "raw = {raw:?} は拒否されること"
            );
        }
    }

    /// 前後の空白は落とす（貼り付け由来の事故を通さない）。
    #[test]
    fn trims_the_port_and_the_database_path() {
        let config = config_of(&[(ENV_PORT, " 9002 "), (ENV_DATABASE_PATH, "  db.sqlite\t")])
            .expect("空白を除けば正当であること");
        assert_eq!(config.bind().port(), 9002);
        assert_eq!(config.database_path(), Path::new("db.sqlite"));
    }

    /// **UI の配り先は設定から来る**（ADR-0031 §3 / [`ENV_WEB_DIR`]）。
    ///
    /// **DB のパスと同時に、別の値で**与える。
    /// 同値だと**取り違えが観測できない**（CLAUDE.md「フィクスチャが2つの値を…」）。
    #[test]
    fn reads_the_web_dir_from_the_lookup() {
        let config = config_of(&[
            (ENV_DATABASE_PATH, "F:\\data\\pc.sqlite"),
            (ENV_WEB_DIR, "F:\\ui\\built"),
        ])
        .expect("正当な値であること");

        assert_eq!(config.web_dir(), Some(Path::new("F:\\ui\\built")));
        assert_ne!(
            config.web_dir(),
            Some(config.database_path()),
            "前提: DB のパスとは別の値を与えていること"
        );
        assert_ne!(
            config.web_dir(),
            Some(Path::new(DEFAULT_WEB_DIR)),
            "前提: 埋め込み元の綴りとも別の値を与えていること"
        );
    }

    /// **未設定なら配り先は無い**＝**埋め込みを配る**（ADR-0041 §決定2 / 作業単位 F-4a）。
    ///
    /// # なぜ2つの腕を1本に置くか
    ///
    /// **「常に `None`」も「常に `Some`」も、片腕だけでは通り抜ける**
    /// （CLAUDE.md「対比は安い側だけでは足りない」）。
    ///
    /// **⚠ 設定した側の期待値を [`DEFAULT_WEB_DIR`] から作らないこと。**
    /// **かつてここは「未設定 → `DEFAULT_WEB_DIR`」を見ていた**ので、
    /// 埋め込みへ落ちる形に変えたあとも定数から作ると、**何も見ていないことになる。**
    #[test]
    fn the_web_dir_is_only_set_when_it_is_configured() {
        let unset = config_of(&[]).expect("既定値だけで組み立てられること");
        assert_eq!(
            unset.web_dir(),
            None,
            "未設定は「埋め込みを配る」であること（既定のパスへ落とさない）"
        );

        let configured =
            config_of(&[(ENV_WEB_DIR, "F:\\ui\\from-disk")]).expect("正当な値であること");
        assert_eq!(configured.web_dir(), Some(Path::new("F:\\ui\\from-disk")));
    }

    /// **DB の既定は「渡されたデータディレクトリの下」**（ADR-0041 §決定3 / 作業単位 F-4a）。
    ///
    /// # なぜ2つの場所で回すか
    ///
    /// **1つだけだと「どこか固定の場所を返す」実装と区別が付かない**
    /// （`tests/server_web_ui.rs::each_server_serves_the_directory_it_was_configured_with` と同じ形）。
    /// **どちらもこの機の既定（`%LOCALAPPDATA%` 等）ではない**——
    /// 既定だと「引いてきた」と「決め打った」が区別できない。
    #[test]
    fn the_database_falls_back_under_the_data_dir_it_is_given() {
        const FIRST: &str = "Z:\\first-data-dir";
        const SECOND: &str = "Z:\\second-data-dir";
        assert_ne!(FIRST, SECOND, "前提: 2つは別の場所であること");

        let machine = os_data_dir();
        for planted in [FIRST, SECOND] {
            assert_ne!(
                Some(PathBuf::from(planted)),
                machine,
                "前提: この機の既定とは別の場所で回すこと"
            );

            let config =
                AppConfig::from_lookup_with_data_dir(|_| None, Some(PathBuf::from(planted)))
                    .expect("既定値だけで組み立てられること");

            assert_eq!(
                config.database_path(),
                Path::new(planted)
                    .join("personacore")
                    .join("personacore.sqlite"),
                "渡された場所の下に置くこと"
            );
        }

        // **明示設定は、渡された場所より強い。**
        let explicit = AppConfig::from_lookup_with_data_dir(
            lookup_of(&[(ENV_DATABASE_PATH, "F:\\explicit\\pc.sqlite")]),
            Some(PathBuf::from(FIRST)),
        )
        .expect("正当な値であること");
        assert_eq!(
            explicit.database_path(),
            Path::new("F:\\explicit\\pc.sqlite")
        );
    }

    /// **場所を決められない環境は、起動を止める**（ADR-0041 §決定3 / 作業単位 F-4a）。
    ///
    /// **黙ってカレントディレクトリへ落とさない**——落とすと、
    /// **叩いた場所ごとに別の DB が生える**（ADR-0041 の背景そのもの）。
    ///
    /// **対比を1本に置く**——`PERSONACORE_DB_PATH` があれば、場所が無くても起動できる。
    #[test]
    fn an_undeterminable_data_dir_stops_the_startup() {
        assert!(
            matches!(
                AppConfig::from_lookup_with_data_dir(|_| None, None),
                Err(ConfigError::DataDir)
            ),
            "場所が決められないなら止めること"
        );

        let rescued = AppConfig::from_lookup_with_data_dir(
            lookup_of(&[(ENV_DATABASE_PATH, "F:\\explicit\\pc.sqlite")]),
            None,
        )
        .expect("明示設定があれば起動できること");
        assert_eq!(
            rescued.database_path(),
            Path::new("F:\\explicit\\pc.sqlite")
        );
    }

    /// **OS の作法の場所は、環境変数から引く**（ADR-0041 §決定3 / 作業単位 F-4a）。
    ///
    /// # ⚠ この機では、Windows の腕しか走らない
    ///
    /// [`data_dir_from`] は **`#[cfg]` で OS ごとに1本だけコンパイルされる。**
    /// **Mac と Linux の腕は、この機では一度も走っていない**
    /// （CLAUDE.md「守っていないものを先に書く」）。
    ///
    /// **プロセスの環境は読まない**——植えた参照関数で測る。
    #[test]
    fn the_data_dir_comes_from_the_os_convention() {
        // **空・空白だけは「未設定」と読む**（どの OS でも同じ）。
        assert_eq!(data_dir_from(&|_: &str| Some(String::new())), None);
        assert_eq!(data_dir_from(&|_: &str| Some("  \t".to_owned())), None);
        assert_eq!(data_dir_from(&|_: &str| None), None);

        #[cfg(windows)]
        {
            assert_eq!(
                data_dir_from(
                    &|key: &str| (key == "LOCALAPPDATA").then(|| "Z:\\planted\\Local".to_owned())
                ),
                Some(PathBuf::from("Z:\\planted\\Local")),
                "Windows は LOCALAPPDATA をそのまま使うこと"
            );
            assert_eq!(
                data_dir_from(&|key: &str| (key == "HOME").then(|| "Z:\\planted\\home".to_owned())),
                None,
                "Windows で HOME を読まないこと（LOCALAPPDATA が権威）"
            );
        }
        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                data_dir_from(&|key: &str| (key == "HOME").then(|| "/planted/home".to_owned())),
                Some(PathBuf::from("/planted/home/Library/Application Support")),
            );
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            assert_eq!(
                data_dir_from(
                    &|key: &str| (key == "XDG_DATA_HOME").then(|| "/planted/xdg".to_owned())
                ),
                Some(PathBuf::from("/planted/xdg")),
                "XDG_DATA_HOME が優先されること"
            );
            assert_eq!(
                data_dir_from(&|key: &str| (key == "HOME").then(|| "/planted/home".to_owned())),
                Some(PathBuf::from("/planted/home/.local/share")),
                "XDG_DATA_HOME が無ければ HOME の下"
            );
        }
    }

    /// FE のソースが置いてあるディレクトリ（リポジトリのルートから見た綴り）。
    ///
    /// **`DEFAULT_WEB_DIR` から作らないこと**——作ると期待値が定数と一緒に動く。
    const WEB_ROOT: &str = "web";

    /// `vite.config.ts` の中で出力先を書いている鍵。
    const VITE_OUT_DIR_KEY: &str = "outDir:";

    /// **`vite.config.ts` の `build.outDir` を拾う**（見つからなければ `None`）。
    ///
    /// **引数でソースを受け取る形にしてある。** 実ファイルと、テストが植えたソースを
    /// **同じ本体に通す**ため（CLAUDE.md `15b`「対照は植えたソースを走査本体に通す」）。
    /// 実ファイルだけを見る対照は**自己成就する**——走査の綴りを壊す変異が、
    /// 壊した綴りを走査対象に書き込むだけで緑になる。
    ///
    /// # 綴りの扱い
    ///
    /// * **大文字小文字を区別する。** 隣にいる `emptyOutDir` は**大文字の `O`** なので
    ///   一致しない（見張り番は `the_vite_scan_reads_the_value_it_is_given`）。
    /// * **最初の一致を採る。** だから `vite.config.ts` 側の注記に
    ///   鍵を綴らせない（そちらを拾ってしまう。**実際に一度踏んだ**）。
    ///
    /// **⚠ 「鍵の直前が識別子の続きでないこと」を見る枝は置かない。**
    /// `emptyOutDir` が唯一の想定敵だったが、**上記のとおり大文字で始まるので
    /// そもそも一致しない**——その枝は**到達しない検査**（変異させても緑のまま）になる。
    fn vite_out_dir(source: &str) -> Option<&str> {
        let at = source.find(VITE_OUT_DIR_KEY)?;
        quoted_value(&source[at + VITE_OUT_DIR_KEY.len()..])
    }

    /// 先頭の空白を飛ばして、引用符で囲まれた値を取り出す。
    fn quoted_value(after_key: &str) -> Option<&str> {
        let rest = after_key.trim_start();
        let quote = rest.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let inside = &rest[quote.len_utf8()..];
        let end = inside.find(quote)?;
        Some(&inside[..end])
    }

    /// `web/vite.config.ts` の中身。
    fn vite_config_source() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(WEB_ROOT)
            .join("vite.config.ts");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("{} を読めること: {err}", path.display()))
    }

    /// **埋め込み元は、`vite build` が実際に書き出す場所と同じ**
    /// （ADR-0031 §3 / ADR-0041 §決定2 で「配り先」から「埋め込み元」へ意味が移った）。
    ///
    /// # なぜ関数の外の値で押さえるか
    ///
    /// `DEFAULT_WEB_DIR` は **`vite.config.ts` の `build.outDir` の写し**である。
    /// **写しどうしが食い違うと、画面が黙って出なくなる**
    /// （起動ログの `ui_files=0` だけが手掛かりになる）。
    /// **期待値をこの定数から作るテストは、綴りを何も見ていない。**
    /// **ここだけが、リポジトリの外側（FE の設定ファイル）から期待値を作る。**
    ///
    /// **⚠ 鎖のもう一方の端は `server::web` の
    /// `the_embedded_ui_comes_from_the_default_web_dir`**——
    /// `include_dir!` に書いた綴りがこの定数と同じであることを見る。
    /// **どちらか一方だけでは、「ビルドした画面が埋め込まれる」は成立しない。**
    #[test]
    fn the_default_web_dir_is_where_vite_actually_builds() {
        let source = vite_config_source();
        let out_dir = vite_out_dir(&source).expect("vite.config.ts から build.outDir を拾えること");

        assert_eq!(
            Path::new(DEFAULT_WEB_DIR),
            Path::new(WEB_ROOT).join(out_dir),
            "DEFAULT_WEB_DIR と vite.config.ts の build.outDir が食い違っている\
             （片方だけ変えると UI が黙って出なくなる）"
        );
    }

    /// **走査本体の対照**——植えたソースを、実ファイルと同じ本体に通す。
    ///
    /// これが無いと、走査の綴りを壊す変異が**空振りしたまま緑**になりうる。
    /// 「1件も無いから見つからない」と「そもそも探せていない」を区別する。
    #[test]
    fn the_vite_scan_reads_the_value_it_is_given() {
        assert_eq!(
            vite_out_dir("  build: {\n    outDir: \"planted-dist\",\n  },\n"),
            Some("planted-dist"),
            "植えた値をそのまま拾うこと"
        );
        assert_eq!(
            vite_out_dir("outDir: 'single-quoted'"),
            Some("single-quoted"),
            "引用符はどちらでも読むこと"
        );
        // **隣にいる `emptyOutDir` を拾わないこと。**
        // **⚠ 効いているのは「大文字小文字を区別する」ことだけ**（`OutDir` ≠ `outDir`）。
        // **識別子の境界は見ていない**——見る枝を置いても到達しないので置いていない
        // （`vite_out_dir` の doc）。**大文字小文字を無視する変異は、ここで赤くなる。**
        assert_eq!(
            vite_out_dir("emptyOutDir: \"decoy\",\n    outDir: \"real\",\n"),
            Some("real"),
            "`emptyOutDir` を先に拾わないこと"
        );
        assert_eq!(
            vite_out_dir("emptyOutDir: \"decoy\",\n"),
            None,
            "`emptyOutDir` しか無いなら「見つからない」と言うこと"
        );
        assert_eq!(
            vite_out_dir("build: { emptyOutDir: true }"),
            None,
            "出力先が書いていないソースを、黙って何かに解釈しないこと"
        );
        assert_eq!(vite_out_dir(""), None, "空のソースは「見つからない」");

        // **最初の一致を採る。** 注記の中に鍵を綴ると、そちらを拾う——
        // **実際に一度踏んだ**（`vite.config.ts` に説明を書き足したら赤くなった）。
        // だから `vite.config.ts` 側の注記には鍵の綴りを書かないと決めてある。
        assert_eq!(
            vite_out_dir("// 例: outDir: \"注記\"\n    outDir: \"real\",\n"),
            Some("注記"),
            "先に現れた方を拾うこと（注記に鍵を綴らせない理由）"
        );

        // **実ファイルも同じ本体を通ること**（対照だけが通る形にしない）。
        assert!(
            vite_out_dir(&vite_config_source()).is_some(),
            "実ファイルからも拾えること"
        );
    }

    /// **空は「未設定」と読む**（[`ENV_WEB_DIR`] の doc の表）。
    ///
    /// **`ENV_DATABASE_PATH` とは逆の作法**なので、1本の中で並べて固定する
    /// ——同じファイルの2つの作法が入れ替わったら、ここが赤くなる。
    /// 前後の空白を落とすこと（貼り付け由来の事故）も同時に見る。
    #[test]
    fn the_web_dir_reads_an_empty_value_as_unset() {
        for raw in ["", " ", "\t", "\r\n"] {
            let config = config_of(&[(ENV_WEB_DIR, raw)]).expect("起動を止めないこと");
            assert_eq!(
                config.web_dir(),
                None,
                "raw = {raw:?} は未設定（＝埋め込みを配る）として読むこと"
            );

            // **同じ綴りを DB のパスへ与えると、あちらは拒否する。**
            assert!(
                matches!(
                    config_of(&[(ENV_DATABASE_PATH, raw)]),
                    Err(ConfigError::DatabasePath)
                ),
                "raw = {raw:?} は DB のパスとしては拒否されること（作法が逆であること）"
            );
        }

        let config =
            config_of(&[(ENV_WEB_DIR, "  ui/dist\t")]).expect("空白を除けば正当であること");
        assert_eq!(config.web_dir(), Some(Path::new("ui/dist")));
    }

    /// 空の DB パスは拒否する（カレントディレクトリを DB として開こうとしない）。
    #[test]
    fn rejects_a_blank_database_path() {
        for raw in ["", " ", "\t", "\r\n"] {
            assert!(
                matches!(
                    config_of(&[(ENV_DATABASE_PATH, raw)]),
                    Err(ConfigError::DatabasePath)
                ),
                "raw = {raw:?} は拒否されること"
            );
        }
    }

    /// **コンテキスト長は明示設定できる**（ADR-0013 第2層の最優先）。
    ///
    /// 生成用の余白は既定のまま据え置き、コンテキスト全体だけを差し替える。
    #[test]
    fn reads_the_context_length_from_the_lookup() {
        let config = config_of(&[(ENV_CONTEXT_TOKENS, "16384")]).expect("正当な値であること");
        let budget = config.context_budget().expect("明示設定が残ること");

        assert_eq!(budget.context_tokens(), 16_384);
        assert_eq!(
            budget.response_tokens(),
            TokenBudget::DEFAULT_RESPONSE_TOKENS,
            "生成用の余白は据え置くこと"
        );
        assert_eq!(budget.prompt_allowance(), 16_384 - 1_024);

        // 前後の空白は落とす（貼り付け由来の事故を通さない）。
        assert_eq!(
            config_of(&[(ENV_CONTEXT_TOKENS, " 16384\t")])
                .expect("空白を除けば正当であること")
                .context_budget()
                .expect("明示設定が残ること")
                .context_tokens(),
            16_384
        );
    }

    /// **未設定は「既定値」ではなく「未設定」。**
    ///
    /// ここで既定値を埋めてしまうと、`App::bind` から見て
    /// 「明示設定された 8192」と「何も指定されていない」の区別が付かなくなり、
    /// **上流への照会が二度と行われなくなる**（ADR-0013 第2層が丸ごと死ぬ）。
    #[test]
    fn an_unset_context_length_stays_unset() {
        assert_eq!(
            config_of(&[]).expect("既定値で組めること").context_budget(),
            None
        );
    }

    /// **予算として成立しない指定は起動時に落とす。**
    ///
    /// 0・非数値・巨大値のいずれも、黙って既定値へ落とさない
    /// （明示設定は「取れたら得をする」経路ではなく、利用者の意思表示）。
    #[test]
    fn rejects_a_context_length_that_cannot_become_a_budget() {
        for raw in [
            "",
            "  ",
            "0",
            "-1",
            "abc",
            "8192.0",
            "8k",
            "16384 16385",
            // u32 に収まらない。
            "4294967296",
            "99999999999999999999",
            // 生成用の余白（1024）以下では、プロンプトの取り分が残らない。
            "1",
            "1023",
            "1024",
        ] {
            assert!(
                matches!(
                    config_of(&[(ENV_CONTEXT_TOKENS, raw)]),
                    Err(ConfigError::ContextTokens)
                ),
                "raw = {raw:?} は拒否されること"
            );
        }

        // 境界: 余白より 1 大きければ成立する。
        assert_eq!(
            config_of(&[(ENV_CONTEXT_TOKENS, "1025")])
                .expect("余白を 1 上回れば正当であること")
                .context_budget()
                .expect("明示設定が残ること")
                .prompt_allowance(),
            1
        );
    }

    /// **予算の規則を通る値は、どれも抽出の上限としても成り立つ**（ADR-0054 §決定1 / 作業単位 A-13a-1）。
    ///
    /// **`sub` 面が無ければ本文の値を抽出にも使う**ので、本文の値は2つの規則を通る
    /// （[`context_budget_of`] と `SubContext::new`）。**読み込みが見るのは前者だけ**なので、
    /// 前者が後者より緩くなった日に、読み込みは通るのに `ProfileContext::of` が止まる値が生まれる。
    /// **余白（1024）の上から十分な範囲を総当たりで見る**（規則はどちらも単調なので、
    /// 下端の側が破れるならこの範囲に現れる）。**対照**: 余白ちょうどは前者で止まる。
    #[test]
    fn the_profile_context_rule_admits_every_budget_as_an_extraction_limit() {
        assert!(context_budget_of(TokenBudget::DEFAULT_RESPONSE_TOKENS).is_err());
        let mut checked = 0_u32;
        for tokens in TokenBudget::DEFAULT_RESPONSE_TOKENS..=200_000 {
            if context_budget_of(tokens).is_ok() {
                checked += 1;
                assert!(
                    SubContext::new(tokens).is_some(),
                    "{tokens} は予算になるのに抽出の上限にならない"
                );
            }
        }
        assert!(
            checked > 100_000,
            "前提: 総当たりが空振りしていない（{checked}）"
        );
    }

    /// **面に書いた値は、環境変数の欄（`context_budget`）を書き換えずに別の欄へ載る**
    /// （ADR-0054 §決定3 —— 切り替えで戻る先を消さない / 作業単位 A-13a-1）。
    ///
    /// **3つの値を全部違える**（環境変数・本文の面・抽出の面）。`sub` 面を書いたプロファイルと
    /// 書いていないプロファイルの両方で見る（後者は本文の値が抽出にも入る）。
    #[test]
    fn the_profile_context_rides_beside_the_environment_value() {
        let profiles = super::super::upstreams::parse_profiles(
            r#"{ "split": { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "context_tokens": 28672 },
                            "sub":  { "endpoint": "http://127.0.0.1:2", "model": "s", "context_tokens": 10240 } },
                 "one":   { "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "context_tokens": 24576 } } }"#,
            None,
        )
        .expect("読めること");

        for (name, main, sub) in [("split", 28_672, 10_240), ("one", 24_576, 24_576)] {
            let settings = profiles.select(name).expect("在ること").settings();
            let config = config_of(&[(ENV_CONTEXT_TOKENS, "12288")])
                .expect("組めること")
                .with_profile_context(&settings)
                .expect("載せられること");
            assert_eq!(
                config.context_budget().map(TokenBudget::context_tokens),
                Some(12_288),
                "{name}: 環境変数の欄を書き換えている"
            );
            let context = config.profile_context();
            assert_eq!(
                (
                    context.main.map(TokenBudget::context_tokens),
                    context.sub.map(SubContext::tokens)
                ),
                (Some(main), Some(sub)),
                "{name}"
            );
        }

        // **対照**: 載せなければ何も書いていない。
        assert_eq!(
            config_of(&[]).expect("組めること").profile_context(),
            ProfileContext::default()
        );
    }

    /// 不正なエンドポイント・モデル識別子は**起動時に**落とす。
    #[test]
    fn rejects_a_malformed_upstream() {
        assert!(matches!(
            config_of(&[(ENV_LLM_ENDPOINT, "127.0.0.1:1234")]),
            Err(ConfigError::Endpoint(_))
        ));
        assert!(matches!(
            config_of(&[(ENV_LLM_ENDPOINT, "")]),
            Err(ConfigError::Endpoint(_))
        ));
        assert!(matches!(
            config_of(&[(ENV_MAIN_MODEL, "  ")]),
            Err(ConfigError::Model(_))
        ));
        assert!(matches!(
            config_of(&[(ENV_MAIN_MODEL, "mo\ndel")]),
            Err(ConfigError::Model(_))
        ));
    }

    /// **どんなポートでも IP はループバック。**
    ///
    /// ここが壊れると絶対制約（外部公開しない）が黙って破れる。
    #[test]
    fn bind_addr_is_always_loopback() {
        for port in [0_u16, 1, 80, 8080, 1234, 49_152, u16::MAX] {
            let addr = BindAddr::loopback(port).socket_addr();
            assert!(addr.ip().is_loopback(), "port = {port}: {addr}");
            assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
            assert_eq!(addr.port(), port);
        }
    }

    /// 表示はそのまま `127.0.0.1:ポート`（起動ログに出す値）。
    #[test]
    fn bind_addr_displays_the_socket_address() {
        assert_eq!(BindAddr::loopback(8080).to_string(), "127.0.0.1:8080");
    }

    /// 差し替えは指定した項目だけを変える。
    #[test]
    fn builders_replace_only_the_given_item() {
        let config = config_of(&[])
            .expect("既定値で組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path("other.sqlite");

        assert_eq!(config.bind().port(), 0);
        assert_eq!(config.database_path(), Path::new("other.sqlite"));
        assert_eq!(config.llm().model().as_str(), DEFAULT_MAIN_MODEL);
    }

    /// 理由ごとにエラーを区別できる（起動失敗時の案内が割れる）。
    ///
    /// # ⚠ **配列リテラルで並べないこと**（作業単位 B-6 / レビュー指摘 R-3）
    ///
    /// 以前はここが**ただの配列リテラル**だったので、
    /// **バリアントを足しても書き忘れればコンパイルが通った。**
    /// 実際に**2つ落ちていた** —— `DriftTuning`（B-6 以前から）と、
    /// B-6 で足した `LlmIdleTimeout`。
    /// **落ちた文言が既存のどれかと衝突しても、誰も気づかない。**
    ///
    /// **いまは `match` を通す**ので、バリアントが増えた日に
    /// **網羅性の検査でコンパイルが止まる**（先例は `parse::extraction` の
    /// `all_lists_every_reason_exactly_once`）。
    /// **[`VARIANTS`] の数え上げが、`all` への足し忘れの側を塞ぐ。**
    #[test]
    fn config_errors_are_distinguishable() {
        /// バリアントの総数。**`all` に足し忘れたらここで落ちる。**
        const VARIANTS: usize = 13;

        /// **`match` なので、バリアントが増えた日にコンパイルできなくなる。**
        ///
        /// 返すのは失敗したときの手掛かりで、**文言そのものではない**
        /// （文言を写すと、文言を変える変異と一緒に期待値が動く）。
        fn tag(err: &ConfigError) -> &'static str {
            match err {
                ConfigError::Port => "port",
                ConfigError::DatabasePath => "database_path",
                ConfigError::DataDir => "data_dir",
                ConfigError::Endpoint(_) => "endpoint",
                ConfigError::Model(_) => "model",
                ConfigError::ContextTokens => "context_tokens",
                ConfigError::LlmIdleTimeout => "llm_idle_timeout",
                ConfigError::StatDrift => "stat_drift",
                ConfigError::DriftForm => "drift_form",
                ConfigError::DriftNote => "drift_note",
                ConfigError::DriftTuning(_) => "drift_tuning",
                ConfigError::AxisDiscoveryMemories => "axis_discovery_memories",
                ConfigError::AxisTrial(_) => "axis_trial",
            }
        }

        let all = [
            ConfigError::Port,
            ConfigError::DatabasePath,
            ConfigError::DataDir,
            ConfigError::Endpoint(EndpointError::Empty),
            ConfigError::Model(ModelIdError::Empty),
            ConfigError::ContextTokens,
            ConfigError::LlmIdleTimeout,
            ConfigError::StatDrift,
            ConfigError::DriftForm,
            ConfigError::DriftNote,
            ConfigError::DriftTuning(DriftTuningError::Decay),
            ConfigError::AxisDiscoveryMemories,
            ConfigError::AxisTrial(AxisTrialRuleError::Turns),
        ];

        assert_eq!(
            all.len(),
            VARIANTS,
            "バリアントを増やしたら、この一覧にも足すこと"
        );
        // **一覧がすべてのバリアントをちょうど1回ずつ並べていること。**
        // 同じバリアントを2つ入れて数だけ合わせる形をここで塞ぐ。
        let mut tags: Vec<&str> = all.iter().map(tag).collect();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(
            tags.len(),
            VARIANTS,
            "同じバリアントが2度並んでいる（別のバリアントが漏れている）: {tags:?}"
        );

        let messages: Vec<String> = all.iter().map(ToString::to_string).collect();
        for (err, message) in all.iter().zip(&messages) {
            assert!(!message.is_empty(), "{} の文言が空", tag(err));
        }
        for (i, (left_err, left)) in all.iter().zip(&messages).enumerate() {
            for (right_err, right) in all[i + 1..].iter().zip(&messages[i + 1..]) {
                assert_ne!(
                    left,
                    right,
                    "{} と {} の文言が同じ（どちらを直せばよいか分からない）",
                    tag(left_err),
                    tag(right_err)
                );
            }
        }
    }

    /// **性格ステータスの変容は既定で off**（ADR-0024 §8）。
    ///
    /// 既定を on にすると、抽出の質を測っている最中に性格が動いてしまい、
    /// **どちらの効果かを切り分けられなくなる。**
    /// 同じ会話を2回投げて比べる測定も成立しなくなる（§8 の ⚠）。
    #[test]
    fn the_stat_drift_switch_is_off_unless_it_is_asked_for() {
        assert_eq!(
            config_of(&[])
                .expect("既定値だけで組み立てられること")
                .stat_drift(),
            StatDriftMode::Off,
            "未設定なら off であること"
        );
    }

    /// **on と読む綴り。** どれも同じ意味になる。
    #[test]
    fn the_stat_drift_switch_turns_on_for_every_affirmative_spelling() {
        for raw in ["1", "true", "TRUE", "True", "on", "ON", "yes", " on "] {
            assert_eq!(
                config_of(&[(ENV_STAT_DRIFT, raw)])
                    .unwrap_or_else(|err| panic!("{raw:?} を読めること: {err}"))
                    .stat_drift(),
                StatDriftMode::On,
                "{raw:?} は on と読むこと"
            );
        }
    }

    /// **off と読む綴り。** 明示的に切れること。
    #[test]
    fn the_stat_drift_switch_turns_off_for_every_negative_spelling() {
        for raw in ["0", "false", "FALSE", "off", "no", "", "   "] {
            assert_eq!(
                config_of(&[(ENV_STAT_DRIFT, raw)])
                    .unwrap_or_else(|err| panic!("{raw:?} を読めること: {err}"))
                    .stat_drift(),
                StatDriftMode::Off,
                "{raw:?} は off と読むこと"
            );
        }
    }

    /// **記録専用と読む綴り**（ADR-0026 §決定1）。
    ///
    /// # ここが `On` に落ちると、測定が黙って壊れる
    ///
    /// 記録専用モードの存在意義は「**値が動かないので決定性が保たれる**」ことで、
    /// on と取り違えると**同じ会話を2回投げて比べる測定が成立しなくなる**
    /// のに、**票は同じように溜まる**ので集計だけ見ても気付けない。
    ///
    /// 前後の空白と大小の畳み込みも、他の2つと同じに効くこと。
    #[test]
    fn the_stat_drift_switch_reads_the_shadow_spelling() {
        for raw in ["shadow", "SHADOW", "Shadow", " shadow ", "\tshadow\n"] {
            assert_eq!(
                config_of(&[(ENV_STAT_DRIFT, raw)])
                    .unwrap_or_else(|err| panic!("{raw:?} を読めること: {err}"))
                    .stat_drift(),
                StatDriftMode::Shadow,
                "{raw:?} は記録専用と読むこと"
            );
        }
    }

    /// **読めない綴りは起動時に落とす。黙って off へ落とさない**（ADR-0024 §8）。
    ///
    /// このスイッチは**実験の切り替え**なので、`PERSONACORE_STAT_DRIFT=onn` を
    /// 黙って off にすると、**測定者は on のつもりで off の結果を読む。**
    /// 明示設定の不正を起動時に落とすのは `ENV_CONTEXT_TOKENS` と同じ扱い。
    ///
    /// **`RUST_LOG=infoo` とは事情が違う。** あちらは書式として妥当な指定
    /// （`infoo` という名前のターゲット）なので落とせないが、
    /// ここは閉じた語彙なので「その語彙に無い」と言い切れる。
    ///
    /// **記録専用の綴りの近傍も並べてある**（`shadows` / `shade`）。
    /// 前方一致で読む実装にすると、`shadows` が黙って記録専用になる。
    #[test]
    fn an_unreadable_stat_drift_spelling_stops_the_start() {
        for raw in [
            "onn", "1.0", "有効", "2", "-1", "y", "shadows", "shade", "sha dow",
        ] {
            // **`expect_err` に書式は使えない**（素の `&str` なので補間されない）。
            // 落ちたときに読みたいのは `raw` なので、`unwrap_or_else` で組み立てる。
            let err = config_of(&[(ENV_STAT_DRIFT, raw)])
                .map_or_else(|err| err, |_| panic!("読めない綴りは拒否すること: {raw:?}"));
            assert!(
                matches!(err, ConfigError::StatDrift),
                "{raw:?} は変容スイッチの理由で落ちること: {err:?}"
            );
        }
    }

    /// **3つの語彙が1語も重ならない。**
    ///
    /// 重なると、同じ綴りが**照合する順**で意味を変える
    /// （`parse_stat_drift` は on → shadow → off の順に見る）。
    /// **実験の切り替えスイッチが、読む順に依存して黙って別の意味になる形**は作らない。
    ///
    /// **記録専用が加わって、重なりの害が重くなった。** 2値の頃は
    /// 「on のつもりで off」だけだったが、いまは**「on のつもりで shadow」**があり、
    /// これは**票が溜まるので動いているように見えるのに値が動かない。**
    #[test]
    fn the_three_stat_drift_vocabularies_never_overlap() {
        let vocabularies: [(&str, &[&str]); 3] = [
            ("on", &STAT_DRIFT_ON),
            ("shadow", &STAT_DRIFT_SHADOW),
            ("off", &STAT_DRIFT_OFF),
        ];
        for (index, (left_name, left)) in vocabularies.iter().enumerate() {
            for (right_name, right) in &vocabularies[index + 1..] {
                for word in *left {
                    assert!(
                        !right.contains(word),
                        "{word:?} が {left_name} と {right_name} の両方の語彙にあります"
                    );
                }
            }
        }
    }

    /// **どのモードの集計キーも、そのまま環境変数に書き戻せる。**
    ///
    /// ログには `StatDriftMode::label` の綴りが出る（`server::drift` /
    /// 起動時の「待ち受けを開始しました」）。測定を読んだ人が
    /// **その語をそのまま `PERSONACORE_STAT_DRIFT` に書いて再現できる**ことを見張る。
    ///
    /// # 往復だけでは足りない（CLAUDE.md）
    ///
    /// 「読んで書いて同じ」は**2つを取り違えて対応させても成立する**ので、
    /// **語彙の外にある値**——`label()` が返す綴りそのもの——で押さえる。
    #[test]
    fn every_mode_label_is_a_spelling_the_parser_accepts() {
        for mode in StatDriftMode::ALL {
            assert_eq!(
                config_of(&[(ENV_STAT_DRIFT, mode.label())])
                    .unwrap_or_else(|err| panic!("{:?} を読めること: {err}", mode.label()))
                    .stat_drift(),
                mode,
                "{:?} が同じモードに戻ること",
                mode.label()
            );
        }
        // **綴りそのものを押さえる**（往復だけでは入れ替えを検出できない）。
        assert_eq!(StatDriftMode::Off.label(), "off");
        assert_eq!(StatDriftMode::Shadow.label(), "shadow");
        assert_eq!(StatDriftMode::On.label(), "on");
    }

    /// **スイッチは他の設定項目を巻き込まない。**
    ///
    /// off 以外にした回でも、モデルもポートも読み方が変わらないこと。
    /// **期待値を既定値にしない**——既定のままだと「設定を読まずに既定を使う」
    /// 実装と区別が付かない（CLAUDE.md）。
    ///
    /// **記録専用で測る。** `on` は「真偽値として読む実装」でも通ってしまうが、
    /// この綴りは**3値になって初めて読めるようになった値**である。
    #[test]
    fn the_stat_drift_switch_sits_beside_the_other_settings() {
        let config = config_of(&[
            (ENV_STAT_DRIFT, "shadow"),
            (ENV_PORT, "9002"),
            (ENV_SUB_MODEL, "ministral-3-3b-instruct-2512"),
        ])
        .expect("すべて正当な値であること");

        assert_eq!(config.stat_drift(), StatDriftMode::Shadow);
        assert_eq!(config.bind().port(), 9002);
        assert_eq!(
            config.sub_model().as_str(),
            "ministral-3-3b-instruct-2512",
            "スイッチを読んでも他の項目は変わらないこと"
        );
    }

    // ---------------------------------------------------------------------
    // 依頼の形（ADR-0027 §12 / PERSONACORE_DRIFT_FORM）
    // ---------------------------------------------------------------------

    /// **依頼の形は既定で `all`**（ADR-0027 §決定8 / 16i / 16j）。
    ///
    /// **`single` は上げ偏りが 94% で分化しなかった**（16i）。
    /// 全軸採点は §12 が「未検証の仮説」として持ち込んだ形だが、
    /// **16d-2 / 16i / 16j / 16l / 16n で実測して置き換わった。**
    /// **`single` は明示したときだけ効く。**
    #[test]
    fn the_drift_form_defaults_to_the_all_axis_request() {
        assert_eq!(
            config_of(&[])
                .expect("既定値だけで組み立てられること")
                .drift_form(),
            DriftForm::All,
            "未設定なら all であること"
        );
    }

    /// **`all` と読む綴り。** 前後の空白と大小も畳む。
    ///
    /// **`single` の側と別のテストにする**——同じテストに入れると、
    /// **どちらも `Single` を返す実装**（`all` を読み落とす）が
    /// 「半分は通っている」形で見えにくくなる。
    #[test]
    fn the_drift_form_reads_the_all_axis_spelling() {
        for raw in ["all", "ALL", "All", " all ", "\tall\n"] {
            assert_eq!(
                config_of(&[(ENV_DRIFT_FORM, raw)])
                    .unwrap_or_else(|err| panic!("{raw:?} を読めること: {err}"))
                    .drift_form(),
                DriftForm::All,
                "{raw:?} は全軸採点と読むこと"
            );
        }
    }

    /// **`single` と読む綴り。**
    ///
    /// # ⚠ **空文字列はここから外した**（作業単位 19）
    ///
    /// 既定が `single` だった間は「空 ＝ 既定 ＝ `single`」で3つとも一致していたが、
    /// **既定が `all` へ移った**ので、空文字列は [`DRIFT_FORM_ALL`] 側へ動かした。
    /// **空文字列を含むのは「既定側の語彙」であって「`single` の語彙」ではない。**
    /// 空の扱いは `every_drift_switch_reads_an_empty_value_as_unset` が3つのスイッチまとめて見る。
    #[test]
    fn the_drift_form_reads_the_single_axis_spelling() {
        for raw in ["single", "SINGLE", " single ", "\tsingle\n"] {
            assert_eq!(
                config_of(&[(ENV_DRIFT_FORM, raw)])
                    .unwrap_or_else(|err| panic!("{raw:?} を読めること: {err}"))
                    .drift_form(),
                DriftForm::Single,
                "{raw:?} は1軸を尋ねる形と読むこと"
            );
        }
    }

    /// **3つのスイッチとも、空文字列は「未設定」と同じに読む**（作業単位 19）。
    ///
    /// # なぜ3つを1本に置くのか
    ///
    /// **これは「スイッチごとの規則」ではなく「この基盤の作法」**である。
    /// スイッチごとに別のテストへ散らすと、**新しいスイッチを足した人が
    /// どれか1本を写して終わり**にでき、しかも**写さなかったほうが既定でない値を指す**
    /// ——`PERSONACORE_DRIFT_FORM` で実際に起きた（作業単位 19 で既定を `all` へ移したとき、
    /// **空文字列だけが `single` に取り残された**）。
    ///
    /// # ⚠ **「未設定」と「空」を両方測る。そして「常に既定」と区別する**
    ///
    /// * **未設定**が既定であること —— 既定そのものの主張。
    /// * **空**が**未設定と同じ値**であること —— ここが 19 で割れた。
    /// * **非既定の綴りを書いたらその値になる**こと —— これが無いと、
    ///   **何を渡しても既定を返す実装**が「空も未設定も既定」を満たして緑を通る
    ///   （CLAUDE.md「期待値が既定値のテストは、その行が何を渡していても緑」）。
    ///
    /// **空白だけの綴りも入れる**——`trim` を落とす実装は `" "` で落ちる。
    ///
    /// 数のつまみ（`PERSONACORE_DRIFT_*`）は語彙ではないので、
    /// 同じ作法を `an_empty_drift_knob_falls_back_to_the_default` が別に見張る。
    #[test]
    fn every_drift_switch_reads_an_empty_value_as_unset() {
        /// 空だと見なす綴り。**`trim` を落とすと空白の側で落ちる。**
        const BLANK: [&str; 4] = ["", " ", "   ", "\t\n"];

        let unset = config_of(&[]).expect("既定値だけで組み立てられること");

        // **期待値は既定値を返す関数からではなく、ADR に書いてある値を直書き**する。
        assert_eq!(unset.stat_drift(), StatDriftMode::Off, "既定は off");
        assert_eq!(unset.drift_form(), DriftForm::All, "既定は all");
        assert_eq!(unset.drift_note(), DriftNote::None, "既定は注記なし");

        for blank in BLANK {
            let empty = config_of(&[
                (ENV_STAT_DRIFT, blank),
                (ENV_DRIFT_FORM, blank),
                (ENV_DRIFT_NOTE, blank),
            ])
            .unwrap_or_else(|err| panic!("{blank:?} で起動できること: {err}"));

            assert_eq!(
                empty.stat_drift(),
                unset.stat_drift(),
                "{blank:?} の走らせ方が未設定と違う"
            );
            assert_eq!(
                empty.drift_form(),
                unset.drift_form(),
                "{blank:?} の依頼の形が未設定と違う"
            );
            assert_eq!(
                empty.drift_note(),
                unset.drift_note(),
                "{blank:?} の注記が未設定と違う"
            );
        }

        // **対照: 非既定の綴りは、ちゃんとその値になる。**
        // これが無いと「何を渡しても既定」の実装が上を全部満たす。
        let spelled = config_of(&[
            (ENV_STAT_DRIFT, "shadow"),
            (ENV_DRIFT_FORM, "single"),
            (ENV_DRIFT_NOTE, "language"),
        ])
        .expect("すべて正当な値であること");
        assert_eq!(spelled.stat_drift(), StatDriftMode::Shadow);
        assert_eq!(spelled.drift_form(), DriftForm::Single);
        assert_eq!(spelled.drift_note(), DriftNote::Language);
        // **前提: 3つとも既定と別の値であること**（同値だと対照が成り立たない）。
        assert_ne!(spelled.stat_drift(), unset.stat_drift());
        assert_ne!(spelled.drift_form(), unset.drift_form());
        assert_ne!(spelled.drift_note(), unset.drift_note());
    }

    /// **読めない綴りは起動時に落とす。黙って `single` へ落とさない**（ADR-0027 §12）。
    ///
    /// **この作業単位の存在理由が「同じ台本で2つの形を比べる」**なので、
    /// `PERSONACORE_DRIFT_FORM=al` を黙って `single` にすると、
    /// **測定者は全軸採点のつもりで1軸の結果を読み、そのまま結論を書く。**
    ///
    /// **綴りの近傍を並べてある**（`al` / `alll` / `allaxes`）。
    /// 前方一致や部分一致で読む実装にすると、どれかが黙って `all` になる。
    #[test]
    fn an_unreadable_drift_form_spelling_stops_the_start() {
        for raw in [
            "al", "alll", "allaxes", "全部", "1", "true", "on", "sing", "singles", "n",
        ] {
            let err = config_of(&[(ENV_DRIFT_FORM, raw)])
                .map_or_else(|err| err, |_| panic!("読めない綴りは拒否すること: {raw:?}"));
            assert!(
                matches!(err, ConfigError::DriftForm),
                "{raw:?} は依頼の形の理由で落ちること: {err:?}"
            );
        }
    }

    /// **2つの語彙が1語も重ならない。**
    ///
    /// 重なると、同じ綴りが**照合する順**で意味を変える
    /// （`parse_drift_form` は all → single の順に見る）。
    #[test]
    fn the_two_drift_form_vocabularies_never_overlap() {
        for word in DRIFT_FORM_ALL {
            assert!(
                !DRIFT_FORM_SINGLE.contains(&word),
                "{word:?} が両方の語彙にあります"
            );
        }
    }

    /// **どちらの形の集計キーも、そのまま環境変数に書き戻せる。**
    ///
    /// ログには `DriftForm::label` の綴りが出る（起動時の「待ち受けを開始しました」）。
    /// 測定を読んだ人が**その語をそのまま `PERSONACORE_DRIFT_FORM` に書いて再現できる**こと。
    ///
    /// 往復だけでは入れ替えを検出できない（CLAUDE.md）ので、**綴りそのものも押さえる。**
    #[test]
    fn every_drift_form_label_is_a_spelling_the_parser_accepts() {
        for form in DriftForm::ALL {
            assert_eq!(
                config_of(&[(ENV_DRIFT_FORM, form.label())])
                    .unwrap_or_else(|err| panic!("{:?} を読めること: {err}", form.label()))
                    .drift_form(),
                form,
                "{:?} が同じ形に戻ること",
                form.label()
            );
        }
        assert_eq!(DriftForm::Single.label(), "single");
        assert_eq!(DriftForm::All.label(), "all");
    }

    /// **2つのスイッチは互いを巻き込まない**（ADR-0027 §12 / `ENV_DRIFT_FORM` の doc）。
    ///
    /// **どちらの期待値も既定値にしない**——既定のままだと
    /// 「片方を読んでもう片方に流用する」実装が観測できない（CLAUDE.md）。
    ///
    /// # ⚠ **明示する形は `single` である**（作業単位 19）
    ///
    /// **既定が `all` へ移った**ので、`all` を書いて `all` を期待すると
    /// **「読んでいない実装」と区別が付かない。**
    /// 「明示して効くこと」を見る側は**非既定の綴り**を書き、
    /// 「指定しなければ既定のまま」を見る側だけが `all` を期待する。
    #[test]
    fn the_drift_form_and_the_drift_mode_are_read_independently() {
        let config = config_of(&[(ENV_STAT_DRIFT, "shadow"), (ENV_DRIFT_FORM, "single")])
            .expect("すべて正当な値であること");
        assert_eq!(config.stat_drift(), StatDriftMode::Shadow);
        assert_eq!(config.drift_form(), DriftForm::Single);

        // **片方だけ指定しても、もう片方は既定のまま。**
        let only_form = config_of(&[(ENV_DRIFT_FORM, "single")]).expect("正当な値であること");
        assert_eq!(only_form.drift_form(), DriftForm::Single);
        assert_eq!(
            only_form.stat_drift(),
            StatDriftMode::Off,
            "形を指定しただけで変容が走り始めないこと"
        );

        let only_mode = config_of(&[(ENV_STAT_DRIFT, "on")]).expect("正当な値であること");
        assert_eq!(only_mode.stat_drift(), StatDriftMode::On);
        assert_eq!(
            only_mode.drift_form(),
            DriftForm::All,
            "走らせただけで形が変わらないこと"
        );

        // **読めない形は、変容スイッチが正当でも起動を止める。**
        let err = config_of(&[(ENV_STAT_DRIFT, "on"), (ENV_DRIFT_FORM, "al")])
            .map_or_else(|err| err, |_| panic!("読めない綴りは拒否すること"));
        assert!(matches!(err, ConfigError::DriftForm), "{err:?}");
    }

    // ---------------------------------------------------------------------
    // 書き方の注記（ADR-0029 §訂正 / PERSONACORE_DRIFT_NOTE / 作業単位 16m）
    // ---------------------------------------------------------------------

    /// **注記は既定で「足さない」**（腕 X / ADR-0029 §訂正）。
    ///
    /// 既定を `copy` や `language` にすると、**m20 / m21 の 732 ターンと比べられなくなる。**
    /// 3本の腕を比べる測定なので、**既定は「いままでと同じ依頼」でなければならない。**
    #[test]
    fn the_drift_note_defaults_to_no_note() {
        assert_eq!(
            config_of(&[])
                .expect("既定値だけで組み立てられること")
                .drift_note(),
            DriftNote::None,
            "未設定なら注記を足さないこと"
        );
    }

    /// **`copy` と読む綴り**（腕 Y）。前後の空白と大小も畳む。
    ///
    /// **腕ごとに別のテストにする**——1本にまとめると、
    /// **全部を `None` に畳む実装**（読み捨て）が「一部は通っている」形で見えにくくなる
    /// （`the_drift_form_reads_the_all_axis_spelling` と同じ判断）。
    #[test]
    fn the_drift_note_reads_the_copy_spelling() {
        for raw in ["copy", "COPY", "Copy", " copy ", "\tcopy\n"] {
            assert_eq!(
                config_of(&[(ENV_DRIFT_NOTE, raw)])
                    .unwrap_or_else(|err| panic!("{raw:?} を読めること: {err}"))
                    .drift_note(),
                DriftNote::Copy,
                "{raw:?} は複写の指示と読むこと"
            );
        }
    }

    /// **`language` と読む綴り**（腕 Z）。
    #[test]
    fn the_drift_note_reads_the_language_spelling() {
        for raw in ["language", "LANGUAGE", " language ", "\tLanguage\n"] {
            assert_eq!(
                config_of(&[(ENV_DRIFT_NOTE, raw)])
                    .unwrap_or_else(|err| panic!("{raw:?} を読めること: {err}"))
                    .drift_note(),
                DriftNote::Language,
                "{raw:?} は言語の指示と読むこと"
            );
        }
    }

    /// **`none` と読む綴り**（腕 X）。空文字列も含む（`PERSONACORE_DRIFT_NOTE=` で止めない）。
    #[test]
    fn the_drift_note_reads_the_no_note_spelling() {
        for raw in ["none", "NONE", " none ", "", "   "] {
            assert_eq!(
                config_of(&[(ENV_DRIFT_NOTE, raw)])
                    .unwrap_or_else(|err| panic!("{raw:?} を読めること: {err}"))
                    .drift_note(),
                DriftNote::None,
                "{raw:?} は注記なしと読むこと"
            );
        }
    }

    /// **読めない綴りは起動時に落とす。黙って腕 X へ落とさない**（ADR-0029 §訂正）。
    ///
    /// `PERSONACORE_DRIFT_NOTE=lang` を注記なしとして起動すると、
    /// **測定者は腕 Z のつもりで腕 X の結果を読み、「言語の指示は効かなかった」と書く。**
    /// **それは §決定5 の保留を、測らずに畳むことと同じ**である。
    ///
    /// **綴りの近傍を並べてある**（`lang` / `languages` / `cop` / `copies`）。
    /// 前方一致や部分一致で読む実装にすると、どれかが黙って腕になる。
    /// **真偽値の語も拒む**——これは on / off ではなく**3本の腕の1本**である。
    #[test]
    fn an_unreadable_drift_note_spelling_stops_the_start() {
        for raw in [
            "lang",
            "languages",
            "cop",
            "copies",
            "copy-axis",
            "ja",
            "japanese",
            "日本語",
            "off",
            "on",
            "0",
            "1",
            "true",
            "false",
            "no",
            "n",
        ] {
            let err = config_of(&[(ENV_DRIFT_NOTE, raw)])
                .map_or_else(|err| err, |_| panic!("読めない綴りは拒否すること: {raw:?}"));
            assert!(
                matches!(err, ConfigError::DriftNote),
                "{raw:?} は注記の理由で落ちること: {err:?}"
            );
        }
    }

    /// **3つの語彙が1語も重ならない。**
    ///
    /// 重なると、同じ綴りが**照合する順**で腕を変える
    /// （`parse_drift_note` は copy → language → none の順に見る）。
    #[test]
    fn the_three_drift_note_vocabularies_never_overlap() {
        for word in DRIFT_NOTE_COPY {
            assert!(
                !DRIFT_NOTE_LANGUAGE.contains(&word) && !DRIFT_NOTE_NONE.contains(&word),
                "{word:?} が複数の語彙にあります"
            );
        }
        for word in DRIFT_NOTE_LANGUAGE {
            assert!(
                !DRIFT_NOTE_NONE.contains(&word),
                "{word:?} が複数の語彙にあります"
            );
        }
    }

    /// **どの腕の集計キーも、そのまま環境変数に書き戻せる。**
    ///
    /// ログには `DriftNote::label` の綴りが出る（起動時の「待ち受けを開始しました」）。
    /// **3本の腕は依頼が1行違うだけ**なので、
    /// **測定を読んだ人がその語をそのまま書いて再現できないと、腕を取り違える。**
    ///
    /// 往復だけでは入れ替えを検出できない（CLAUDE.md）ので、**綴りそのものも押さえる。**
    #[test]
    fn every_drift_note_label_is_a_spelling_the_parser_accepts() {
        for note in DriftNote::ALL {
            assert_eq!(
                config_of(&[(ENV_DRIFT_NOTE, note.label())])
                    .unwrap_or_else(|err| panic!("{:?} を読めること: {err}", note.label()))
                    .drift_note(),
                note,
                "{:?} が同じ腕に戻ること",
                note.label()
            );
        }
        assert_eq!(DriftNote::None.label(), "none");
        assert_eq!(DriftNote::Copy.label(), "copy");
        assert_eq!(DriftNote::Language.label(), "language");
    }

    /// **注記・依頼の形・走らせ方は、互いを巻き込まない**（ADR-0029 §訂正）。
    ///
    /// **どの期待値も既定値にしない**——既定のままだと
    /// 「片方を読んでもう片方に流用する」実装が観測できない（CLAUDE.md）。
    ///
    /// # ⚠ **明示する形は `single` である**（作業単位 19）
    ///
    /// 既定が `all` へ移ったので、**明示して効くことを見る側は `single` を書く。**
    /// `all` を期待するのは「指定しなければ既定のまま」を見る箇所だけ。
    #[test]
    fn the_drift_note_and_the_other_drift_switches_are_read_independently() {
        let config = config_of(&[
            (ENV_STAT_DRIFT, "shadow"),
            (ENV_DRIFT_FORM, "single"),
            (ENV_DRIFT_NOTE, "language"),
        ])
        .expect("すべて正当な値であること");
        assert_eq!(config.stat_drift(), StatDriftMode::Shadow);
        assert_eq!(config.drift_form(), DriftForm::Single);
        assert_eq!(config.drift_note(), DriftNote::Language);

        // **注記だけを指定しても、他の2つは既定のまま。**
        let only_note = config_of(&[(ENV_DRIFT_NOTE, "copy")]).expect("正当な値であること");
        assert_eq!(only_note.drift_note(), DriftNote::Copy);
        assert_eq!(
            only_note.stat_drift(),
            StatDriftMode::Off,
            "注記を指定しただけで変容が走り始めないこと"
        );
        assert_eq!(
            only_note.drift_form(),
            DriftForm::All,
            "注記を指定しただけで依頼の形が変わらないこと"
        );

        // **形だけを指定しても、注記は既定のまま。**
        let only_form = config_of(&[(ENV_DRIFT_FORM, "single")]).expect("正当な値であること");
        assert_eq!(
            only_form.drift_note(),
            DriftNote::None,
            "形を指定しただけで注記が付かないこと"
        );

        // **読めない注記は、他の2つが正当でも起動を止める。**
        let err = config_of(&[
            (ENV_STAT_DRIFT, "on"),
            (ENV_DRIFT_FORM, "single"),
            (ENV_DRIFT_NOTE, "lang"),
        ])
        .map_or_else(|err| err, |_| panic!("読めない綴りは拒否すること"));
        assert!(matches!(err, ConfigError::DriftNote), "{err:?}");
    }

    // ---------------------------------------------------------------------
    // 集約のつまみ（ADR-0027 §決定8 / §9）
    // ---------------------------------------------------------------------

    /// **未設定なら ADR-0027 §決定8 の値**（λ=**0.5** / **傾き 1.5** / K=**0.3**）。
    ///
    /// 出どころは ADR-0027 の「16j」節——**実データ2本の提案列に 16 通り当てて、
    /// 5つの条件（飽和 0 / 両方向 / 段数 4〜12 / 端に貼り付かない / 分化）で選んだ点。**
    ///
    /// # 期待値を `DriftTuning::default()` から作らない
    ///
    /// 作ると、**既定を変える変異が期待値を道連れにして緑のまま通る**
    /// （CLAUDE.md「関数の外にある値で押さえる」）。
    /// ここは**ADR に書いてある数を直書き**する。
    #[test]
    fn the_drift_tuning_defaults_to_the_values_the_adr_settled_on() {
        let tuning = config_of(&[])
            .expect("既定値だけで組み立てられること")
            .drift_tuning();
        assert!((tuning.decay() - 0.5).abs() < 1e-12, "λ の既定は 0.5");
        assert!((tuning.slope() - 1.5).abs() < 1e-12, "傾きの既定は 1.5");
        assert!((tuning.step_cost() - 0.3).abs() < 1e-12, "K の既定は 0.3");
    }

    /// **3つのつまみが、それぞれ自分の環境変数から読まれる。**
    ///
    /// # 3つとも**別の値**にする
    ///
    /// 同じ値で測ると、**綴りを取り違える実装**（λ の欄に K を入れる等）が緑を通る
    /// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
    /// **既定とも全部違う値**にしてある。
    ///
    /// # ⚠ **傾きは `1.25`。既定の `1.5` を書かないこと**（作業単位 20a で直した）
    ///
    /// **doc は「既定とも全部違う値」と書いているのに、傾きだけ既定と同じ `1.5` だった**
    /// （19 のレビューが指摘。**着手前からの食い違い**）。
    /// 既定と同値だと、**`ENV_DRIFT_SLOPE` を読まずに既定へ落とす実装**が緑を通る。
    /// **`DriftTuning::new` の範囲は `0..=`[`MAX_DRIFT_SLOPE`]（＝ `1.8`）**なので、
    /// 差し替えるときはその中で選ぶこと。
    #[test]
    fn every_drift_knob_is_read_from_its_own_variable() {
        let tuning = config_of(&[
            (ENV_DRIFT_DECAY, "0.25"),
            (ENV_DRIFT_SLOPE, "1.25"),
            (ENV_DRIFT_STEP_COST, "0.75"),
        ])
        .expect("つまみを読めること")
        .drift_tuning();

        assert!((tuning.decay() - 0.25).abs() < 1e-12, "λ が届いていない");
        assert!((tuning.slope() - 1.25).abs() < 1e-12, "傾きが届いていない");
        assert!(
            (tuning.step_cost() - 0.75).abs() < 1e-12,
            "K が届いていない"
        );
        assert_ne!(
            tuning,
            DriftTuning::default(),
            "前提: 既定と違う組であること"
        );
        // **前提: 1つずつ既定と違うこと**（組として違うだけでは、
        // 既定へ落とす欄が1つ紛れていても気付けない / doc の ⚠）。
        assert_ne!(tuning.decay(), DEFAULT_DRIFT_DECAY, "λ が既定と同値");
        assert_ne!(tuning.slope(), DEFAULT_DRIFT_SLOPE, "傾きが既定と同値");
        assert_ne!(
            tuning.step_cost(),
            DEFAULT_DRIFT_STEP_COST,
            "K が既定と同値"
        );
    }

    /// **小数の綴りを受ける。** 指数表記も符号付きも整数も。
    ///
    /// 閉じた語彙にできない項目なので、**受ける形を名指しで固定する。**
    #[test]
    fn the_drift_knobs_accept_the_spellings_a_measurement_would_write() {
        for (raw, want) in [
            ("1", 1.0_f64),
            ("1.5", 1.5),
            ("+0.75", 0.75),
            ("0.5e1", 5.0),
            (" 1.25 ", 1.25),
            ("0", 0.0),
        ] {
            let tuning = config_of(&[(ENV_DRIFT_DECAY, raw)])
                .unwrap_or_else(|err| panic!("{raw:?} を読めること: {err}"))
                .drift_tuning();
            assert!(
                (tuning.decay() - want).abs() < 1e-12,
                "{raw:?} が {want} として読まれていない: {}",
                tuning.decay()
            );
        }
    }

    /// **空文字列と空白だけは既定へ落とす**（`PERSONACORE_DRIFT_FORM=` と揃える）。
    ///
    /// **読めない綴りとは別扱い**——空は「指定していない」であって「書き間違い」ではない。
    #[test]
    fn an_empty_drift_knob_falls_back_to_the_default() {
        for raw in ["", " ", "\t\n"] {
            let tuning = config_of(&[
                (ENV_DRIFT_DECAY, raw),
                (ENV_DRIFT_SLOPE, raw),
                (ENV_DRIFT_STEP_COST, raw),
            ])
            .unwrap_or_else(|err| panic!("{raw:?} で起動できること: {err}"))
            .drift_tuning();
            assert_eq!(tuning, DriftTuning::default(), "{raw:?} が既定にならない");
        }
    }

    /// **読めない綴りは起動を止める。黙って既定へ落とさない**（ADR-0024 §8 と同じ立場）。
    ///
    /// # どのつまみが読めなかったかまで残す
    ///
    /// 3つ同時に指定した起動で「どれを直せばいいか」が出ないと、
    /// **測定者は3つとも書き直すことになる。**
    /// 理由は `#[source]` 側（`DriftTuningError`）が持つ。
    #[test]
    fn an_unreadable_drift_knob_stops_the_startup_and_says_which_one() {
        for (key, want) in [
            (ENV_DRIFT_DECAY, DriftTuningError::Decay),
            (ENV_DRIFT_SLOPE, DriftTuningError::Slope),
            (ENV_DRIFT_STEP_COST, DriftTuningError::StepCost),
        ] {
            for raw in ["1.5.0", "1,5", "abc", "１", "0x1"] {
                let err = config_of(&[(key, raw)])
                    .err()
                    .unwrap_or_else(|| panic!("{key}={raw:?} は起動を止めること"));
                let ConfigError::DriftTuning(reason) = err else {
                    panic!("{key}={raw:?} が別の理由になった: {err}");
                };
                assert_eq!(reason, want, "{key}={raw:?} のつまみが取り違えられている");
            }
        }
    }

    /// **範囲の外も起動を止める**（`inf` / `nan` / 負 / 傾きの上限超え）。
    ///
    /// # 綴りとしては読める値である
    ///
    /// `f64` の `parse` は `inf` も `nan` も受けるので、
    /// **止めているのは `DriftTuning::new` の側**である（`drift_knob` の doc）。
    /// **止める場所を2つに写さない**ための配置で、ここはその配置が現に効くことを見る。
    #[test]
    fn a_drift_knob_outside_its_range_stops_the_startup() {
        for (key, raw) in [
            (ENV_DRIFT_DECAY, "-1"),
            (ENV_DRIFT_DECAY, "inf"),
            (ENV_DRIFT_DECAY, "nan"),
            (ENV_DRIFT_SLOPE, "-0.5"),
            // **上限を超える傾きは費用を負にする。**
            // **`2` は `v = 0` の上げの費用を `0` にする**（ADR-0027「訂正」）。
            (ENV_DRIFT_SLOPE, "2"),
            (ENV_DRIFT_SLOPE, "1.9"),
            (ENV_DRIFT_SLOPE, "2.5"),
            (ENV_DRIFT_SLOPE, "inf"),
            (ENV_DRIFT_STEP_COST, "0"),
            // **亜正規数**——`f64` としては読めるが、
            // **いちばん安い段がアンダーフローして `0` になる**（レビュー指摘C / 16e-b）。
            (ENV_DRIFT_STEP_COST, "5e-324"),
            (ENV_DRIFT_STEP_COST, "1e-323"),
            (ENV_DRIFT_STEP_COST, "-1"),
            (ENV_DRIFT_STEP_COST, "inf"),
        ] {
            let err = config_of(&[(key, raw)])
                .err()
                .unwrap_or_else(|| panic!("{key}={raw:?} は起動を止めること"));
            assert!(
                matches!(err, ConfigError::DriftTuning(_)),
                "{key}={raw:?} が別の理由になった: {err}"
            );
        }

        // **端はちょうど受ける**（門が1つずれていないこと）。
        assert!(
            config_of(&[(ENV_DRIFT_SLOPE, "1.8")]).is_ok(),
            "傾きの上限ちょうど（1.8）が拒否された"
        );
        assert!(
            config_of(&[(ENV_DRIFT_DECAY, "0")]).is_ok(),
            "λ = 0（減衰なし）が拒否された"
        );
    }

    /// **つまみの理由の綴りが、他の設定エラーと重ならない。**
    ///
    /// `Display` は**定数文字列**（CLAUDE.md 絶対制約）。応答に載るので、
    /// **どの項目の話かが読めないと直しようがない。**
    #[test]
    fn the_drift_tuning_error_says_which_setting_without_leaking_the_value() {
        let err = config_of(&[(ENV_DRIFT_STEP_COST, "F:\\秘密\\path")])
            .expect_err("読めない綴りは起動を止めること");
        let message = err.to_string();
        assert!(
            !message.contains("秘密") && !message.contains('\\'),
            "指定された値が Display に漏れている: {message}"
        );
        assert_ne!(
            message,
            ConfigError::DriftForm.to_string(),
            "依頼の形のエラーと綴りが同じ"
        );
        assert_ne!(
            message,
            ConfigError::StatDrift.to_string(),
            "スイッチのエラーと綴りが同じ"
        );
    }

    // ---------------------------------------------------------------------
    // 打ち切りの天井（ADR-0035 §決定3 / ADR-0036 §決定3 / 作業単位 B-6）
    // ---------------------------------------------------------------------

    /// **天井を秒で受け取り、`llm` の4つの上限へ届く。**
    ///
    /// # ⚠ 期待値を既定にしない（B-7 の穴）
    ///
    /// 既定（180秒）で回すと、**「読んだ値」と「定数」が文字どおり同じ式**になり、
    /// **つまみを丸ごと捨てる変異が緑を通る。** `assert_ne!` にも掛からない
    /// （値がどちらも 180 秒だから）。**既定と違う値で回すこと。**
    ///
    /// 導出そのもの（残り3つの関係）は `llm::lmstudio` の
    /// `one_knob_derives_every_other_deadline` が見る。ここが見ているのは
    /// **秒として読んで、その `LmStudioConfig` を持って帰る**ところまで。
    #[test]
    fn reads_the_llm_idle_timeout_as_seconds() {
        /// 既定とは別の天井。**桁も揃えない。**
        const RAW: &str = "45";
        /// 上と同じ値。**実装の `parse` を通さずに書き下す。**
        const EXPECTED: Duration = Duration::from_secs(45);

        let unset = config_of(&[]).expect("既定値だけで組み立てられること");
        assert_ne!(
            unset.llm().idle_timeout(),
            EXPECTED,
            "前提: 期待値が既定と違うこと（同値だと、読まずに既定を掴む実装と区別できない）"
        );

        let config = config_of(&[(ENV_LLM_IDLE_TIMEOUT, RAW)]).expect("秒として読めること");
        assert_eq!(
            config.llm().idle_timeout(),
            EXPECTED,
            "指定した天井が `llm` の設定まで届くこと"
        );
        assert!(
            config.llm().request_timeout() >= EXPECTED,
            "全体上限が天井を下回ると、天井の方が到達不能になる: {:?}",
            config.llm().request_timeout()
        );
        assert!(
            config.llm().context_timeout() < EXPECTED,
            "照会の上限は天井より短いこと: {:?}",
            config.llm().context_timeout()
        );
        assert_ne!(
            config.llm().request_timeout(),
            unset.llm().request_timeout(),
            "全体上限が天井についてきていない（導出ではなく既定を掴んでいる）"
        );

        // 前後の空白は落とす（他のつまみと同じ作法）。
        assert_eq!(
            config_of(&[(ENV_LLM_IDLE_TIMEOUT, " 45\t")])
                .expect("空白を落として読めること")
                .llm()
                .idle_timeout(),
            EXPECTED
        );
    }

    /// **未設定・空・空白だけは既定の天井**（`drift_knob` と同じ作法）。
    ///
    /// **対照を1本置く**——これが無いと「何を渡しても既定」の実装が
    /// 空の側を全部満たして緑を通る
    /// （`every_drift_switch_reads_an_empty_value_as_unset` と同じ形）。
    #[test]
    fn an_empty_llm_idle_timeout_falls_back_to_the_default() {
        /// 空だと見なす綴り。**`trim` を落とすと空白の側で落ちる。**
        const BLANK: [&str; 4] = ["", " ", "   ", "\t\n"];

        let unset = config_of(&[]).expect("既定値だけで組み立てられること");
        // **期待値は `llm` の定数からではなく、ADR-0019 に書いてある値を直書き。**
        assert_eq!(
            unset.llm().idle_timeout(),
            Duration::from_secs(180),
            "既定の天井は 180 秒"
        );

        for blank in BLANK {
            let empty = config_of(&[(ENV_LLM_IDLE_TIMEOUT, blank)])
                .unwrap_or_else(|err| panic!("{blank:?} で起動できること: {err}"));
            assert_eq!(
                empty.llm().idle_timeout(),
                unset.llm().idle_timeout(),
                "{blank:?} の天井が未設定と違う"
            );
            assert_eq!(
                empty.llm().request_timeout(),
                unset.llm().request_timeout(),
                "{blank:?} の全体上限が未設定と違う"
            );
        }

        // **対照: 書けばその値になる。**
        assert_eq!(
            config_of(&[(ENV_LLM_IDLE_TIMEOUT, "7")])
                .expect("秒として読めること")
                .llm()
                .idle_timeout(),
            Duration::from_secs(7)
        );
    }

    /// **読めない綴りと `0` は起動を止める**（黙って既定へ落とさない）。
    ///
    /// # `0` を弾く判断（作業単位 B-6）
    ///
    /// `0` は**書式としては妥当**なので、`RUST_LOG=infoo` の論点に当たる。
    /// **違いは1つ** —— あちらは**正しい値が導出できない**が、
    /// こちらは**正しくない値が一意に特定できる。**
    ///
    /// 弾く理由は、**導出した接続の上限まで 0 に潰れて、
    /// 上流への接続を1度も試みなくなる**こと。
    /// [ADR-0036](../../../docs/adr/0036-the-range-of-machines-is-a-constraint.md) §決定1 の
    /// 判定の問いでは**壊れる**側に落ちる。
    ///
    /// **⚠ 「使えない値だから」ではない**（レビュー指摘で訂正）。
    /// **`1` も遅い機では役に立たない**が、**待ってから失敗する**ので通す。
    /// **線は「即座に失敗する」と「待って失敗する」の境目にある。**
    ///
    /// **負数も同じ**（`u64` として読めない）。**小数も受けない**——
    /// この値は「秒」であって連続量の実験のつまみではない
    /// （`PERSONACORE_DRIFT_*` とは性質が違う）。
    #[test]
    fn an_unusable_llm_idle_timeout_stops_the_startup() {
        for raw in ["0", "-1", "1.5", "abc", "180s", "1 80", "١٨٠"] {
            let err = config_of(&[(ENV_LLM_IDLE_TIMEOUT, raw)])
                .expect_err(&format!("{raw:?} で起動を止めること"));
            assert!(
                matches!(err, ConfigError::LlmIdleTimeout),
                "{raw:?} を別の理由で落としている: {err}"
            );
        }
    }

    /// **エラーの `Display` に、指定された値を反射しない**（CLAUDE.md 絶対制約）。
    ///
    /// **他のバリアントと文言が衝突しないことは、ここでは見ない。**
    /// `config_errors_are_distinguishable` が**全 10 バリアントの総当たり**で見る
    /// ——ここに2つだけ書き写すと、**写さなかった相手と衝突しても緑**になる
    /// （レビュー指摘 R-3 / CLAUDE.md「同じ発見を2箇所に書かない」）。
    #[test]
    fn the_llm_idle_timeout_error_says_which_setting_without_leaking_the_value() {
        let err = config_of(&[(ENV_LLM_IDLE_TIMEOUT, "F:\\秘密\\path")])
            .expect_err("読めない綴りは起動を止めること");
        // 前提: 見ているのが天井のエラーであること（別の理由で落ちていない）。
        assert!(
            matches!(err, ConfigError::LlmIdleTimeout),
            "前提: 天井の指定として弾かれていること: {err}"
        );
        let message = err.to_string();
        assert!(
            !message.contains("秘密") && !message.contains('\\'),
            "指定された値が Display に漏れている: {message}"
        );
    }

    // ---------------------------------------------------------------------
    // 軸の候補を尋ねる間隔（ADR-0046 §決定2 / 作業単位 A-9b）
    // ---------------------------------------------------------------------

    /// **未設定・空・空白のみは既定（10）。数は前後の空白を落として読む。**
    ///
    /// 既定の期待値は**定数から作らない**（外の値 10 で押さえる）。
    #[test]
    fn the_axis_discovery_interval_defaults_and_reads_a_count() {
        for pairs in [
            &[][..],
            &[(ENV_AXIS_DISCOVERY_MEMORIES, "")][..],
            &[(ENV_AXIS_DISCOVERY_MEMORIES, "   ")][..],
        ] {
            assert_eq!(
                config_of(pairs)
                    .expect("既定で起動できること")
                    .axis_discovery_memories()
                    .get(),
                10,
                "{pairs:?}"
            );
        }
        for (raw, count) in [("3", 3), (" 3 ", 3), ("1", 1), ("250", 250)] {
            assert_eq!(
                config_of(&[(ENV_AXIS_DISCOVERY_MEMORIES, raw)])
                    .expect("読めること")
                    .axis_discovery_memories()
                    .get(),
                count,
                "{raw:?}"
            );
        }
    }

    /// **読めない綴りと 0 は起動を止める**（黙って既定へ落とさない）。値は `Display` に反射しない。
    #[test]
    fn an_unusable_axis_discovery_interval_stops_the_startup() {
        for raw in ["0", "-1", "1.5", "abc", "5件", "1 0", "４"] {
            let err = config_of(&[(ENV_AXIS_DISCOVERY_MEMORIES, raw)])
                .expect_err(&format!("{raw:?} で起動を止めること"));
            assert!(
                matches!(err, ConfigError::AxisDiscoveryMemories),
                "{raw:?} を別の理由で落としている: {err}"
            );
            assert!(!err.to_string().contains(raw), "値を反射しない: {err}");
        }
    }

    // ---------------------------------------------------------------------
    // 軸の候補の試用の判定のつまみ（ADR-0046 §決定4 / 作業単位 A-9c）
    // ---------------------------------------------------------------------

    /// **未設定・空・空白のみは既定（60 / 10 / 90）。数は前後の空白を落として読む。**
    ///
    /// 既定の期待値は**定数から作らない**（外の値で押さえる）。3つを**別々の値**で読み、
    /// 取り違え（`K` の欄に `T` を入れる等）を観測できる形にする。
    #[test]
    fn the_axis_trial_rule_defaults_and_reads_each_knob() {
        for pairs in [
            &[][..],
            &[
                (ENV_AXIS_TRIAL_TURNS, ""),
                (ENV_AXIS_TRIAL_MIN_NONZERO, "  "),
                (ENV_AXIS_TRIAL_LINE_PERCENT, ""),
            ][..],
        ] {
            let rule = config_of(pairs).expect("既定で起動できること").axis_trial();
            assert_eq!(
                (rule.turns(), rule.min_nonzero(), rule.line_percent()),
                (60, 10, 90),
                "{pairs:?}"
            );
        }
        let rule = config_of(&[
            (ENV_AXIS_TRIAL_TURNS, " 30 "),
            (ENV_AXIS_TRIAL_MIN_NONZERO, "7"),
            (ENV_AXIS_TRIAL_LINE_PERCENT, "85"),
        ])
        .expect("読めること")
        .axis_trial();
        assert_eq!(
            (rule.turns(), rule.min_nonzero(), rule.line_percent()),
            (30, 7, 85)
        );
        // **1つだけ設定すれば、残りは既定**（組の検証は既定との組で行う）。
        let rule = config_of(&[(ENV_AXIS_TRIAL_LINE_PERCENT, "80")])
            .expect("読めること")
            .axis_trial();
        assert_eq!(
            (rule.turns(), rule.min_nonzero(), rule.line_percent()),
            (60, 10, 80)
        );
    }

    /// **読めない綴り・範囲外・組として成り立たない値は起動を止める。理由はつまみごと。**
    ///
    /// 値は `Display` に反射しない。
    #[test]
    fn an_unusable_axis_trial_rule_stops_the_startup() {
        for (name, raw, expected) in [
            (ENV_AXIS_TRIAL_TURNS, "abc", AxisTrialRuleError::Turns),
            (ENV_AXIS_TRIAL_TURNS, "-1", AxisTrialRuleError::Turns),
            (ENV_AXIS_TRIAL_TURNS, "30回", AxisTrialRuleError::Turns),
            (ENV_AXIS_TRIAL_TURNS, "0", AxisTrialRuleError::Turns),
            // **既定の K（10）を下回る T は、T が読めても組として止まる**（理由は組の不整合）。
            (
                ENV_AXIS_TRIAL_TURNS,
                "9",
                AxisTrialRuleError::MinNonzeroAboveTurns,
            ),
            (
                ENV_AXIS_TRIAL_MIN_NONZERO,
                "1.5",
                AxisTrialRuleError::MinNonzero,
            ),
            (
                ENV_AXIS_TRIAL_MIN_NONZERO,
                "61",
                AxisTrialRuleError::MinNonzeroAboveTurns,
            ),
            (
                ENV_AXIS_TRIAL_LINE_PERCENT,
                "９０",
                AxisTrialRuleError::Line,
            ),
            (ENV_AXIS_TRIAL_LINE_PERCENT, "0", AxisTrialRuleError::Line),
            (ENV_AXIS_TRIAL_LINE_PERCENT, "100", AxisTrialRuleError::Line),
        ] {
            let err =
                config_of(&[(name, raw)]).expect_err(&format!("{name}={raw:?} で起動を止めること"));
            assert!(
                matches!(err, ConfigError::AxisTrial(reason) if reason == expected),
                "{name}={raw:?} を別の理由で落としている: {err:?}"
            );
            assert!(!err.to_string().contains(raw), "値を反射しない: {err}");
        }
    }

    /// **組の不整合（K > T）は、触っていない方を疑わせない**（A-9c のレビュー提案4）。
    ///
    /// `T` = 9 だけを設定すると、既定の `K`（10）との組で止まる。**理由は「組の不整合」**で、
    /// 「標本不足の線の指定が不正」（`K` が読めない）とは別。**文言は両方のつまみを名指しし、数は載せない。**
    /// **値は同じログの行に両方とも出る**（設定していない方の既定まで）。
    #[test]
    fn a_k_above_t_names_both_knobs_and_logs_both_values() {
        let mut result = None;
        let output = crate::testlog::captured_under_default_filter(|| {
            result = Some(config_of(&[(ENV_AXIS_TRIAL_TURNS, "9")]));
        });
        let err = result.expect("走ったこと").expect_err("組として止まること");
        let ConfigError::AxisTrial(reason) = err else {
            panic!("別の理由で落としている: {err:?}");
        };
        assert_eq!(reason, AxisTrialRuleError::MinNonzeroAboveTurns);
        let message = reason.to_string();
        assert!(
            message.contains("標本不足の線") && message.contains("ターン数"),
            "両方のつまみを名指しすること: {message}"
        );
        assert!(
            !message.chars().any(|ch| ch.is_ascii_digit()),
            "数を載せないこと: {message}"
        );
        assert_ne!(
            message,
            AxisTrialRuleError::MinNonzero.to_string(),
            "K が読めない回と同じ文言にしない"
        );
        let line = crate::testlog::line_containing(
            &output,
            "軸の候補の試用の標本不足の線がターン数を超えています",
        );
        for field in ["axis_trial_turns=9", "axis_trial_min_nonzero=10"] {
            assert!(crate::testlog::has_field(line, field), "{field}: {line}");
        }
    }
}
