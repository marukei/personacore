//! **初期化は「消す」ではなく「退避する」**（[ADR-0041](../../docs/adr/0041-one-executable-per-os.md) §決定4 / 作業単位 F-4a）。
//!
//! ```text
//! personacore --fresh
//!   ↓ DB を開く前に
//!   personacore.sqlite      → personacore.sqlite.bak-20260918T101112
//!   personacore.sqlite-wal  → personacore.sqlite.bak-20260918T101112-wal
//!   personacore.sqlite-shm  → personacore.sqlite.bak-20260918T101112-shm
//! ```
//!
//! # ⚠ **削除の経路を作らない**
//!
//! `memory` の行を DELETE しないのと同じ向き（CLAUDE.md の絶対制約）。
//! **`--fresh` が呼ぶのは [`std::fs::rename`] だけ**で、このモジュールに
//! [`std::fs::remove_file`] は1つも無い。**戻したくなったら手で戻せる。**
//!
//! > **⚠ 「`remove_file` が無い」は、消えない理由にならない。**
//! > **[`std::fs::rename`] は宛先が在れば黙って上書きする** ——
//! > **初版はこの論法で上書き破壊を見落とした**（同じ秒に2回 `--fresh` すると
//! > 1回目の退避が消えた / レビュー指摘1 / 2026-09-18 に実測）。
//! > **だから宛先の空きを見てから名前を決める。権威は [`names`] の doc。**
//! >
//! > **⚠ そして途中で失敗したら、動かした分を戻す**（[`stash_with`]）。
//! > **戻さないと、本体だけ退避されて `-wal` が残る** ——
//! > **ADR-0041 §決定4 が名指しした「片方だけ残す」形そのもの。**
//!
//! # ⚠ **`-wal` / `-shm` を置き去りにしない**
//!
//! SQLite は WAL モードで3つのファイルを持つ。**DB 本体だけ改名すると、
//! 次に開いた新しい DB へ古い `-wal` が当たる**（ADR-0041 §決定4）。
//! だから[`backup_moves`] は3つを**同じ接尾辞で**まとめて動かす。
//!
//! **退避先の綴りは、SQLite の作法をそのまま残す**——
//! `…bak-<時刻>` / `…bak-<時刻>-wal` / `…bak-<時刻>-shm` の並びなので、
//! **退避した組をそのまま `sqlite3` で開ける。**
//! （`personacore.sqlite-wal.bak-<時刻>` の形にすると、戻すまで開けない。）
//!
//! # 引数の解析
//!
//! **`--fresh` / `--upstream <名前>` / `--no-browser` / `--version` / `--no-ui` の5つだけ。**
//! **依存（clap 等）は足さない**（ADR-0041 §決定4 / ADR-0042 §決定1 / 作業単位 F-4c）。
//! **4つを超えたら決め直す、と Issue origin#23 で決めていた。** 5つ目の `--no-ui` は
//! [ADR-0047](../../docs/adr/0047-the-api-is-the-product.md) §① で決め直した結果
//! （**まだ手書きの解析で足りる**——値を取るのは `--upstream` だけのまま。作業単位 P-1）。
//! **次に足すときも、そこで決め直す。**
//! **知らない引数は起動を止める**——綴り違いを黙って無視すると、
//! **利用者は初期化したつもりで古い DB のまま遊ぶ。**
//!
//! **`--upstream` は値を1つ取る。** **次の引数を無条件に食べない**——
//! `--upstream --fresh` を通すと **`--fresh` が黙って消える**（同じ穴の別の口）。

use std::path::{Path, PathBuf};

/// `--fresh` の綴り。
pub const FRESH_FLAG: &str = "--fresh";

/// `--upstream` の綴り（ADR-0042 §決定1）。**値を1つ取る。**
///
/// **`--upstream=<名前>` の形は受け付けない**——受け付ける形を2つ持つと、
/// 片方だけを直したときにもう片方が黙って古びる（`--fresh=1` と同じ扱い）。
pub const UPSTREAM_FLAG: &str = "--upstream";

/// `--no-browser` の綴り（作業単位 F-4c）。**待ち受けが開いても、ブラウザを開かない。**
pub const NO_BROWSER_FLAG: &str = "--no-browser";

/// `--no-ui` の綴り（[ADR-0047](../../docs/adr/0047-the-api-is-the-product.md) §① / 作業単位 P-1）。
///
/// **画面を配らない**（`/` と画面の資産に 404）。**ブラウザも開かない**（開く頁が無い）。
/// **API（`/v1` / `/admin`）の振る舞いは1バイトも変えない。**
///
/// **この旗は `--no-browser` を立てない**——「開かない」は `App::run_with_upstreams` が
/// 画面の有無から決める（旗の解釈を2箇所に割らない）。
/// **環境変数の形（`PERSONACORE_NO_UI` 等）は持たない**（ADR-0047 §①4）。
pub const NO_UI_FLAG: &str = "--no-ui";

/// `--version` の綴り（作業単位 F-4c）。**版を言って、何もせずに終わる。**
///
/// **`-V` / `-v` は受け付けない**——受け付ける形を2つ持たない（`--upstream=` と同じ扱い）。
pub const VERSION_FLAG: &str = "--version";

/// **`--version` が言う1行**（作業単位 F-4c / Issue origin#23）。
///
/// `personacore <Cargo.toml の版> (<git describe --always --tags>)`。
/// **括弧の中は `build.rs` がビルド時に埋める**（`PERSONACORE_GIT_DESCRIBE`）。
/// **`git` が無い・リポジトリの外で組んだときは `unknown`。**
///
/// **⚠ 古びうる場合がある**——いつ埋め直されるかは `build.rs` の doc
/// （「`--version` の綴りが古びる経路」）が権威。
pub const VERSION_LINE: &str = concat!(
    "personacore ",
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("PERSONACORE_GIT_DESCRIBE"),
    ")"
);

/// SQLite が DB 本体の隣に置くファイルの接尾辞。
///
/// **`-journal` は入れていない。** この家は WAL モードで開く（`store`）。
/// 入れても害は無いが、**「在れば動かす」の対象を、実際に作られるものに限る。**
const SIDECAR_SUFFIXES: [&str; 2] = ["-wal", "-shm"];

/// 退避先の名前に挟む印。
const BACKUP_MARK: &str = ".bak-";

/// 退避先の時刻の綴り（`20260918T101112`）。
///
/// **`:` を使わない**——Windows のファイル名に使えない。
const BACKUP_TIME_FORMAT: &str = "%Y%m%dT%H%M%S";

/// 起動時の引数。
///
/// **`Copy` ではなくなった**（ADR-0042 §決定1 で名前を1つ持つようになった）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    fresh: bool,
    /// **選ばれた上流プロファイルの名前。** `None` は「旗が無い」。
    ///
    /// **空文字では表さない。** 空を「未設定」と読む欄が他にある
    /// （`PERSONACORE_SUB_LLM_ENDPOINT`）ので、**同じ綴りに2つの意味を持たせない。**
    upstream: Option<String>,
    /// `--no-browser` が渡されたか（作業単位 F-4c）。
    no_browser: bool,
    /// `--version` が渡されたか（作業単位 F-4c）。
    version: bool,
    /// `--no-ui` が渡されたか（ADR-0047 §① / 作業単位 P-1）。
    no_ui: bool,
}

impl Args {
    /// `--fresh` が渡されたか。
    #[must_use]
    pub const fn fresh(&self) -> bool {
        self.fresh
    }

    /// `--upstream` で選ばれたプロファイルの名前。**`None` は「旗が無い」。**
    #[must_use]
    pub fn upstream(&self) -> Option<&str> {
        self.upstream.as_deref()
    }

    /// `--no-browser` が渡されたか。**偽なら、待ち受けが開いたらブラウザを開く。**
    #[must_use]
    pub const fn no_browser(&self) -> bool {
        self.no_browser
    }

    /// `--version` が渡されたか。**真なら、版を言って他は何もしない**（`main`）。
    #[must_use]
    pub const fn version(&self) -> bool {
        self.version
    }

    /// `--no-ui` が渡されたか。**真なら、画面を配らず、ブラウザも開かない。**
    #[must_use]
    pub const fn no_ui(&self) -> bool {
        self.no_ui
    }
}

/// **知らない引数**。[`ArgsError`] の `#[source]` 側に入る。
///
/// **綴りをここに持つ**のは、`Display` を定数文字列に保ったまま
/// 「何が知らない綴りだったか」を運用者へ届けるため（CLAUDE.md のエラーの作法）。
#[derive(Debug, thiserror::Error)]
#[error("知らない引数です: {0}")]
pub struct UnknownArgument(String);

/// 引数を解釈できない理由。
///
/// `Display` は**定数文字列**。詳細は `#[source]` 側に残る。
#[derive(Debug, thiserror::Error)]
pub enum ArgsError {
    /// `--fresh` 以外の綴りが渡された。**黙って無視しない。**
    #[error("引数を解釈できませんでした")]
    Unknown(#[source] UnknownArgument),
}

/// 退避に失敗した理由。
///
/// `Display` は**定数文字列**。パスは `#[source]` にも載せない
/// （[`std::io::Error`] は宛先を含まない）——**在処はログに出す。**
#[derive(Debug, thiserror::Error)]
pub enum FreshError {
    /// 改名できなかった（権限・他プロセスが掴んでいる等）。
    #[error("初期化のための退避に失敗しました")]
    Rename(#[source] std::io::Error),

    /// **空いている退避先の名前が見つからない**（レビュー指摘1 / 作業単位 F-4a）。
    ///
    /// **同じ秒に 999 回 `--fresh` した**ときだけ起きる。
    /// **上書きせずに止まる**のが正しい——[`names`] の doc。
    #[error("退避先の名前が空いていません")]
    NoFreeName,

    /// **途中で失敗し、動かした分を戻すのにも失敗した**（レビュー指摘7 / 作業単位 F-4a）。
    ///
    /// **ここへ来たときだけ、SQLite の3点セットがばらけたまま残る。**
    /// **両方の理由を `#[source]` 側に持つ**ので、
    /// 運用者は「何が失敗して、戻すのがなぜ失敗したか」を両方読める。
    #[error("初期化のための退避を取り消せませんでした")]
    Rollback(#[source] RollbackFailure),
}

/// **退避に失敗し、取り消しにも失敗した**理由の組（[`FreshError::Rollback`] の `#[source]`）。
///
/// **2つの [`std::io::Error`] を1つの型に持つ。**
/// `thiserror` の `#[source]` は1つしか持てないので、
/// **どちらも読めるように `Display` の側で並べる**
/// （**この型は `#[source]` の中なので、定数文字列でなくてよい**
/// ——外向きの `Display` は [`FreshError`] のほう）。
#[derive(Debug, thiserror::Error)]
#[error("退避: {cause} / 取り消し: {rollback}")]
pub struct RollbackFailure {
    /// 退避そのものの失敗。
    cause: std::io::Error,
    /// 戻すときの失敗。
    rollback: std::io::Error,
}

/// 引数列（**実行ファイル名を除いたもの**）を解釈する。**純粋関数。**
///
/// **呼び出し側が `skip(1)` する**——ここで飛ばすと、
/// 「1つ目は必ず実行ファイル名」という前提が引数の無い呼び方で崩れる。
pub fn parse_args<I, S>(args: I) -> Result<Args, ArgsError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut parsed = Args::default();
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        let arg = arg.as_ref();
        if arg == FRESH_FLAG {
            parsed.fresh = true;
        } else if arg == NO_BROWSER_FLAG {
            parsed.no_browser = true;
        } else if arg == VERSION_FLAG {
            parsed.version = true;
        } else if arg == NO_UI_FLAG {
            parsed.no_ui = true;
        } else if arg == UPSTREAM_FLAG {
            // **次の1つを名前として取る。**
            //
            // **ただし旗を食べない。** `--upstream --fresh` を通すと
            // **`--fresh` が黙って消えて、古い DB のまま遊び始める**
            // ——`--fresh` の綴り違いを通すのと同じ実害である。
            // **空白だけの名前も通さない**（プロファイル名として引けない）。
            let name = rest.next();
            let name = name.as_ref().map(AsRef::as_ref).unwrap_or("");
            if name.trim().is_empty() || name.starts_with("--") {
                return Err(ArgsError::Unknown(UnknownArgument(format!(
                    "{UPSTREAM_FLAG} の後ろにプロファイル名がありません"
                ))));
            }
            // **制御文字（改行・ESC 等）を含む名前は通さない**（作業単位 F-4f / レビュー1周目の是正3）。
            //
            // **名前はログに載る**——改行を通すと、1つの行が割れて別の行を偽造できる。
            // **ここが根**: 画面の保存と覚える側の関所（`profile_editor::accepts_name`）は
            // この関数に判定を任せているので、**`--upstream`・画面・覚える、の3つの入口がここで閉じる。**
            // **断る理由に名前そのものを載せない**（載せると、この行が割れる）。
            if name.chars().any(char::is_control) {
                return Err(ArgsError::Unknown(UnknownArgument(format!(
                    "{UPSTREAM_FLAG} のプロファイル名に制御文字（改行など）が含まれています"
                ))));
            }
            // **後ろが勝つ。** 最後に打った旗が効く（読み手の期待どおり）。
            parsed.upstream = Some(name.trim().to_owned());
        } else {
            return Err(ArgsError::Unknown(UnknownArgument(arg.to_owned())));
        }
    }
    Ok(parsed)
}

/// 退避先の接尾辞（`bak-20260918T101112`）。**純粋関数。**
#[must_use]
pub fn backup_suffix(now: chrono::DateTime<chrono::Local>) -> String {
    now.format(BACKUP_TIME_FORMAT).to_string()
}

/// **巻き戻しの先が塞がっていたときの理由**（レビュー3周目 / 作業単位 F-4a）。
///
/// 定数にしてあるのは、見張り番（`a_rollback_never_overwrites_what_took_the_name`）が
/// **綴りを写さずに参照できる**ようにするため。
const ROLLBACK_DESTINATION_TAKEN: &str =
    "戻す先が塞がっています（別のプロセスがその名前を使っています）";

/// 同じ接尾辞が既に使われているときに足す連番の上限。
///
/// **これに当たるのは「同じ秒に 999 回 `--fresh` した」ときだけ**で、
/// そのときは**上書きせずに止まる**（[`FreshError::NoFreeName`]）。
const MAX_BACKUP_ATTEMPTS: usize = 999;

/// `personacore.sqlite` に `-wal` のような接尾辞を足した名前。**純粋関数。**
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// **退避先の名前を決める場所。**
///
/// # ⚠ なぜモジュールで囲うのか（**レビュー指摘1 / 2026-09-18 に実測**）
///
/// **[`std::fs::rename`] は、宛先が在れば黙って上書きする。**
/// 接尾辞は**秒まで**なので、**同じ秒に `--fresh` を2回走らせると
/// 1回目の退避が消える**——**ADR-0041 §決定4「削除の経路を作らない」に反する、
/// 唯一の利用者データ破壊経路**だった（実測: 退避先が1つしか残らず、
/// 中身は2回目の空の DB）。
///
/// **塞ぎ方は「宛先が空いていることを見てから名前を決める」**だが、
/// **それを関数に分けただけでは、隣に「見ないで作る関数」を書ける。**
/// だから [`Moves`] の中身をこのモジュールの private にして、
/// **作る道を [`choose`] 1本にした**——**外からは、空きを確かめずに組んだ組を
/// `rename` へ渡すことが書けない。**
///
/// **⚠ 「型で表現できなくした」とまでは言わない。**
/// **このモジュールの中でなら、[`Moves`] を直に組み立てられる**
/// （いまはそう書いていない、というだけ）。**モジュールの外では書けない。**
mod names {
    use std::path::{Path, PathBuf};

    use super::{BACKUP_MARK, MAX_BACKUP_ATTEMPTS, SIDECAR_SUFFIXES, sidecar};

    /// **退避の組**（`(元の名前, 退避先の名前)` の並び。DB 本体が先）。
    ///
    /// **作る道は [`choose`] だけ。** 中身は private。
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Moves(Vec<(PathBuf, PathBuf)>);

    impl Moves {
        /// 組を読む。**`rename` する側はこれしか持たない。**
        pub fn pairs(&self) -> &[(PathBuf, PathBuf)] {
            &self.0
        }
    }

    /// `<接尾辞>` から `<接尾辞>-2` / `-3` … を作る。
    fn numbered(stamp: &str, attempt: usize) -> String {
        if attempt == 0 {
            stamp.to_owned()
        } else {
            format!("{stamp}-{}", attempt + 1)
        }
    }

    /// 接尾辞1つぶんの組（DB 本体・`-wal`・`-shm`）。
    fn with_stamp(database: &Path, stamp: &str) -> Vec<(PathBuf, PathBuf)> {
        let mut backup = database.as_os_str().to_owned();
        backup.push(BACKUP_MARK);
        backup.push(stamp);
        let backup = PathBuf::from(backup);

        let mut moves = vec![(database.to_path_buf(), backup.clone())];
        for suffix in SIDECAR_SUFFIXES {
            moves.push((sidecar(database, suffix), sidecar(&backup, suffix)));
        }
        moves
    }

    /// **宛先が1つも塞がっていない接尾辞を選ぶ。****純粋関数**
    /// ——「その名前が在るか」を述語で受け取るので、植えた世界で測れる。
    ///
    /// **3つの宛先は、常に同じ接尾辞を共有する**（1つでも塞がっていれば、3つとも次の番号へ）
    /// ——ばらけると、退避した組が SQLite の3点セットでなくなる。
    ///
    /// **空きが見つからなければ `None`**（呼び出し側が [`super::FreshError::NoFreeName`] にする）。
    /// **上書きは絶対にしない。**
    pub fn choose(database: &Path, stamp: &str, taken: &dyn Fn(&Path) -> bool) -> Option<Moves> {
        (0..MAX_BACKUP_ATTEMPTS)
            .map(|attempt| with_stamp(database, &numbered(stamp, attempt)))
            .find(|moves| !moves.iter().any(|(_, to)| taken(to)))
            .map(Moves)
    }
}

/// **`(元の名前, 退避先の名前)` の組を、宛先の空きを見てから決める。**
/// **純粋関数**——「その名前が在るか」は述語で受け取る。
///
/// 詳細と、なぜ組を作る道が1本しか無いのかは [`names`] の doc。
pub fn backup_moves(
    database: &Path,
    stamp: &str,
    taken: &dyn Fn(&Path) -> bool,
) -> Option<names::Moves> {
    names::choose(database, stamp, taken)
}

/// **DB を開く前に、在るものだけを退避する**（ADR-0041 §決定4）。
///
/// 戻り値は**実際に動かした組**。1つも無ければ空（**初回に `--fresh` を付けても失敗しない**）。
///
/// **ここはファイル操作だけ。** 名前の組み立ては [`names::choose`]（純粋）が持つ。
///
/// # ⚠ **上書きしない**（レビュー指摘1）
///
/// **宛先が既に在れば、連番を足した別の接尾辞を選ぶ**——
/// **同じ秒に2回走らせても、1回目の退避は残る。**
pub fn stash_database(
    database: &Path,
    now: chrono::DateTime<chrono::Local>,
) -> Result<Vec<(PathBuf, PathBuf)>, FreshError> {
    let moves =
        backup_moves(database, &backup_suffix(now), &present).ok_or(FreshError::NoFreeName)?;
    // **閉包で包む。** `std::fs::rename` をそのまま渡すと、
    // 総称（`P: AsRef<Path>`）が高階の束縛に載らずコンパイルが通らない。
    stash_with(&moves, |from: &Path, to: &Path| std::fs::rename(from, to))
}

/// **その名前が塞がっているか。**
///
/// **`Path::exists` ではなく [`std::fs::symlink_metadata`]**
/// （同じファイルの `super::web::count_disk_files` と同じ作法 / レビュー指摘6）——
/// **`exists` はリンクを辿るので、壊れたリンクを「無い」と読む。**
/// **壊れたリンクの上へ `rename` すれば、そのリンクは消える。**
/// **ここでは「何かがその名前を占めている」だけを見る。**
fn present(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// **決めた組を、渡された改名で実行する。**
///
/// 戻り値は**実際に動かした組**。1つも無ければ空（**初回に `--fresh` を付けても失敗しない**）。
///
/// # ⚠ **途中で失敗したら、動かした分を戻す**（レビュー指摘7 / 作業単位 F-4a）
///
/// **戻さないと、本体だけ退避されて `-wal` が残る。**
/// **ADR-0041 §決定4 が名指しした形そのもの**——
/// **次に開いた新しい DB へ、古い WAL が当たる。**
///
/// **戻すのにも失敗したら [`FreshError::Rollback`]**（両方の理由を持つ）。
/// **そこまで来たときだけ、3点セットはばらけたまま残る。**
///
/// # なぜ改名を引数で受け取るのか
///
/// **「2つ目で失敗する改名」を植えないと、戻す枝を一度も通せない**
/// （CLAUDE.md `15b`「対照は植てたものを本体に通す」）。
/// **本番は [`std::fs::rename`] を渡すだけ。**
pub fn stash_with<R>(moves: &names::Moves, rename: R) -> Result<Vec<(PathBuf, PathBuf)>, FreshError>
where
    R: Fn(&Path, &Path) -> std::io::Result<()>,
{
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in moves.pairs() {
        if !present(from) {
            continue;
        }
        if let Err(cause) = rename(from, to) {
            // **動かした分を、逆順で戻す。**
            //
            // **⚠ 戻す向きでも `rename` は上書きする**（レビュー3周目 / 2026-09-18 に実測）。
            // **退避に成功してから失敗するまでの間に、別のプロセスが元の名前で
            // 新しい DB を作っている**ことがある（`--fresh` は DB を開く前に走るが、
            // **同じ置き場所を指した別の personacore を止める手段は無い**）。
            // **そこへ戻すと、割り込んだファイルが消え、退避先も戻し切って消える**
            // ——**痕跡がゼロになる。**
            //
            // **ADR-0041 §実装8 の「`remove_file` が無いことは消えない理由にならない」は、
            // 戻す向きにも当たる。** **塞がっていたら戻さずに止める**
            // ——**割り込んだ側が生き残り、退避先も残る**（手で戻せる）。
            for (done_from, done_to) in moved.iter().rev() {
                if present(done_from) {
                    tracing::error!(
                        from = %done_to.display(),
                        to = %done_from.display(),
                        "退避を取り消せませんでした（戻す先が塞がっています。上書きしません）"
                    );
                    return Err(FreshError::Rollback(RollbackFailure {
                        cause,
                        rollback: std::io::Error::other(ROLLBACK_DESTINATION_TAKEN),
                    }));
                }
                if let Err(rollback) = rename(done_to, done_from) {
                    tracing::error!(
                        from = %done_to.display(),
                        to = %done_from.display(),
                        "退避を取り消せませんでした（3点セットがばらけています）"
                    );
                    return Err(FreshError::Rollback(RollbackFailure { cause, rollback }));
                }
            }
            return Err(FreshError::Rename(cause));
        }
        tracing::info!(
            from = %from.display(),
            to = %to.display(),
            "初期化のため退避しました（削除していません。手で戻せます）"
        );
        moved.push((from.clone(), to.clone()));
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 決め打ちの時刻（**この機の「いま」に寄りかからない**）。
    fn fixed_now() -> chrono::DateTime<chrono::Local> {
        use chrono::TimeZone;
        chrono::Local
            .with_ymd_and_hms(2026, 9, 18, 10, 11, 12)
            .single()
            .expect("一意に決まる時刻であること")
    }

    /// **`--fresh` だけを受け付ける。**
    #[test]
    fn only_the_fresh_flag_is_accepted() {
        assert!(
            !parse_args(Vec::<String>::new())
                .expect("引数なしは正当であること")
                .fresh(),
            "引数が無ければ退避しないこと"
        );
        assert!(
            parse_args([FRESH_FLAG])
                .expect("--fresh は正当であること")
                .fresh(),
            "--fresh を受け取ること"
        );
        // **2回書いても同じ**（`--fresh --fresh`）。
        assert!(
            parse_args([FRESH_FLAG, FRESH_FLAG])
                .expect("重ねても正当であること")
                .fresh()
        );
    }

    /// **知らない引数は起動を止める**（綴り違いを黙って無視しない / ADR-0041 §決定4）。
    ///
    /// # なぜ「似た綴り」を並べるか
    ///
    /// **黙って無視する実装の実害は、`--fresh` の綴り間違いである。**
    /// `-fresh` / `--Fresh` / `--fresh=1` は**どれも「初期化したつもり」で打たれる形**で、
    /// 通すと**古い DB のまま遊び始める**（しかも起動は成功する）。
    #[test]
    fn an_unknown_argument_stops_the_startup() {
        for raw in [
            "-fresh",
            "--Fresh",
            "--fresh=1",
            "--fres",
            "--freshh",
            "",
            "-h",
            "--help",
            "extra",
        ] {
            let err = parse_args([raw]).expect_err("止まること");
            let ArgsError::Unknown(unknown) = &err;
            assert!(
                unknown.to_string().contains(raw),
                "どの綴りが知らない引数だったかを言うこと: {raw:?} / {unknown}"
            );
            // **`Display` は定数文字列**（綴りは `#[source]` 側だけに出る）。
            //
            // **`contains(raw)` では書けない**——`raw` が空文字列のとき、
            // どんな文面でも `contains("")` は真になる（**この行で実際に踏んだ**）。
            // **定数そのものと突き合わせる。**
            assert_eq!(
                err.to_string(),
                "引数を解釈できませんでした",
                "表の文は定数文字列であること（詳細は source 側 / {raw:?}）"
            );
        }

        // **正しい綴りと並べても、知らない方で止まること。**
        assert!(parse_args([FRESH_FLAG, "--nope"]).is_err());
        assert!(parse_args(["--nope", FRESH_FLAG]).is_err());
    }

    /// **`--upstream <名前>` は値を1つ取る**（ADR-0042 §決定1 / 作業単位 F-4b-1）。
    ///
    /// # ⚠ `--fresh` と両立する
    ///
    /// **順番も見る。** 片方だけを読む実装は、もう片方を黙って落とす。
    #[test]
    fn the_upstream_flag_takes_one_name_and_lives_beside_fresh() {
        assert_eq!(
            parse_args(Vec::<String>::new())
                .expect("引数なしは正当であること")
                .upstream(),
            None,
            "旗が無ければ、プロファイルは1度も読まない"
        );

        let only = parse_args([UPSTREAM_FLAG, "qwen-official"]).expect("正当であること");
        assert_eq!(only.upstream(), Some("qwen-official"));
        assert!(!only.fresh(), "--upstream は --fresh を立てない");

        // **両立する。どちらの順でも同じ。**
        for args in [
            vec![FRESH_FLAG, UPSTREAM_FLAG, "split"],
            vec![UPSTREAM_FLAG, "split", FRESH_FLAG],
        ] {
            let parsed = parse_args(args.clone()).unwrap_or_else(|err| {
                panic!("{args:?} は正当であること: {err}");
            });
            assert!(parsed.fresh(), "{args:?} で --fresh が落ちている");
            assert_eq!(parsed.upstream(), Some("split"), "{args:?}");
        }

        // **2回書いたら、後ろが勝つ**（最後に打ったものが効く）。
        assert_eq!(
            parse_args([UPSTREAM_FLAG, "a", UPSTREAM_FLAG, "b"])
                .expect("正当であること")
                .upstream(),
            Some("b")
        );
    }

    /// **名前を書かずに終わったら止まる**（黙って既定へ落ちない）。
    ///
    /// # ⚠ 次の引数を名前として食べない
    ///
    /// `--upstream --fresh` は**「`--fresh` という名前のプロファイル」ではない。**
    /// 食べると、**`--fresh` が黙って消えて古い DB のまま遊び始める。**
    #[test]
    fn the_upstream_flag_without_a_name_stops_the_startup() {
        let err = parse_args([UPSTREAM_FLAG]).expect_err("止まること");
        assert_eq!(
            err.to_string(),
            "引数を解釈できませんでした",
            "表の文は定数文字列であること"
        );
        // **`ArgsError` はいま1つしかバリアントを持たない**ので、
        // `let else` は到達しない枝になる（clippy が鳴る）。**分解だけする。**
        let ArgsError::Unknown(unknown) = &err;
        assert!(
            unknown.to_string().contains(UPSTREAM_FLAG),
            "どの旗が値を欠いたかを言うこと: {unknown}"
        );

        // **次の旗を名前として食べない。**
        assert!(
            parse_args([UPSTREAM_FLAG, FRESH_FLAG]).is_err(),
            "旗を名前として食べている（--fresh が黙って消える）"
        );
        // **空の名前も止める**（`--upstream \"\"`）。
        assert!(
            parse_args([UPSTREAM_FLAG, ""]).is_err(),
            "空の名前を通さない"
        );
        assert!(
            parse_args([UPSTREAM_FLAG, "   "]).is_err(),
            "空白だけの名前を通さない"
        );
    }

    /// **制御文字（改行・CR・ESC・TAB）を含む名前は止める。理由に名前を載せない**
    /// （作業単位 F-4f / レビュー1周目の是正3）。
    ///
    /// **名前はログに載る**ので、改行を通すと行を偽造できる。**対照**: 制御文字の無い名前
    /// （空白・日本語を含む）は通る——「何でも止める」実装をここで弾く。
    #[test]
    fn an_upstream_name_with_a_control_character_is_refused() {
        for name in ["x\nFORGED", "x\rFORGED", "x\u{1b}[31m", "a\tb", "x\u{7f}"] {
            let err = parse_args([UPSTREAM_FLAG, name]).expect_err("止まること");
            let ArgsError::Unknown(unknown) = &err;
            assert!(
                !unknown.to_string().contains("FORGED") && !unknown.to_string().contains('\n'),
                "断る理由に名前を載せている: {unknown:?}"
            );
        }
        for name in ["a b", "日本語", "qwen-3.6"] {
            assert_eq!(
                parse_args([UPSTREAM_FLAG, name])
                    .expect("通ること")
                    .upstream(),
                Some(name)
            );
        }
    }

    /// **`--upstream=<名前>` は受け付けない**（`--fresh=1` と同じ扱い）。
    ///
    /// **受け付ける形を2つ持たない。** 片方だけを直すと、もう片方が黙って古びる。
    #[test]
    fn the_equals_form_of_the_upstream_flag_is_unknown() {
        assert!(parse_args(["--upstream=split"]).is_err());
    }

    /// **`--no-browser` と `--version` と `--no-ui` は、それぞれ自分の真偽値だけを立てる**
    /// （作業単位 F-4c / Issue origin#23・作業単位 P-1 / Issue origin#44）。
    ///
    /// # なぜ5つを1本で見るのか
    ///
    /// **旗を1つ足すたびに「別の旗を立ててしまう」取り違えが書ける。**
    /// 旗ごとに「自分だけが立ち、他の4つは立たない」ことを見ないと、
    /// **`--no-browser` が `--fresh` を立てる**（＝ブラウザを止めたつもりで DB を退避する）
    /// 形が緑を通る。
    #[test]
    fn each_flag_raises_only_its_own_switch() {
        let none = parse_args(Vec::<String>::new()).expect("引数なしは正当であること");
        assert!(!none.no_browser(), "引数が無ければブラウザを開く側");
        assert!(!none.version(), "引数が無ければ版を言うだけで終わらない");
        assert!(!none.no_ui(), "引数が無ければ画面を配る側");

        let no_browser = parse_args([NO_BROWSER_FLAG]).expect("正当であること");
        assert!(no_browser.no_browser());
        assert!(
            !no_browser.version(),
            "--no-browser が --version を立てている"
        );
        assert!(!no_browser.fresh(), "--no-browser が --fresh を立てている");
        assert!(!no_browser.no_ui(), "--no-browser が --no-ui を立てている");
        assert_eq!(no_browser.upstream(), None);

        let version = parse_args([VERSION_FLAG]).expect("正当であること");
        assert!(version.version());
        assert!(
            !version.no_browser(),
            "--version が --no-browser を立てている"
        );
        assert!(!version.fresh(), "--version が --fresh を立てている");
        assert!(!version.no_ui(), "--version が --no-ui を立てている");
        assert_eq!(version.upstream(), None);

        // **`--no-ui` は画面の旗だけを立てる**（作業単位 P-1）。
        //
        // **ブラウザの旗は立てない**——「開かない」は `App::run_with_upstreams` が
        // 画面の有無から決める（`--no-browser` を立てる形にすると、旗の解釈が2箇所に割れる）。
        let no_ui = parse_args([NO_UI_FLAG]).expect("正当であること");
        assert!(no_ui.no_ui());
        assert!(!no_ui.no_browser(), "--no-ui が --no-browser を立てている");
        assert!(!no_ui.version(), "--no-ui が --version を立てている");
        assert!(!no_ui.fresh(), "--no-ui が --fresh を立てている");
        assert_eq!(no_ui.upstream(), None);

        // **逆向き**——既存の2つも、新しい3つを立てない。
        let fresh = parse_args([FRESH_FLAG]).expect("正当であること");
        assert!(!fresh.no_browser() && !fresh.version() && !fresh.no_ui());
        let upstream = parse_args([UPSTREAM_FLAG, "split"]).expect("正当であること");
        assert!(!upstream.no_browser() && !upstream.version() && !upstream.no_ui());

        // **5つとも同時に渡せる。順番に寄りかからない。**
        for args in [
            vec![
                FRESH_FLAG,
                NO_BROWSER_FLAG,
                NO_UI_FLAG,
                UPSTREAM_FLAG,
                "split",
                VERSION_FLAG,
            ],
            vec![
                VERSION_FLAG,
                UPSTREAM_FLAG,
                "split",
                NO_UI_FLAG,
                NO_BROWSER_FLAG,
                FRESH_FLAG,
            ],
        ] {
            let parsed = parse_args(args.clone()).unwrap_or_else(|err| {
                panic!("{args:?} は正当であること: {err}");
            });
            assert!(parsed.fresh(), "{args:?}");
            assert!(parsed.no_browser(), "{args:?}");
            assert!(parsed.version(), "{args:?}");
            assert!(parsed.no_ui(), "{args:?}");
            assert_eq!(parsed.upstream(), Some("split"), "{args:?}");
        }
    }

    /// **綴り違いは止める**（`--fresh` と同じ理由 / ADR-0041 §決定4）。
    ///
    /// **`--no-browser` の綴り違いを黙って通すと、止めたつもりでブラウザが開く。**
    /// **`--no-ui` も同じ**——通すと、画面を配らないつもりで配る（作業単位 P-1）。
    /// **`-V` / `-v` も受け付けない**——受け付ける形を2つ持たない（`--upstream=` と同じ）。
    #[test]
    fn misspelt_new_flags_stop_the_startup() {
        for raw in [
            "--no_browser",
            "--nobrowser",
            "--no-browser=1",
            "--No-Browser",
            "--version=1",
            "--Version",
            "-V",
            "-v",
            "--no_ui",
            "--noui",
            "--no-ui=1",
            "--No-UI",
            "--no-gui",
            "--headless",
        ] {
            let err = parse_args([raw]).expect_err("止まること");
            let ArgsError::Unknown(unknown) = &err;
            assert!(
                unknown.to_string().contains(raw),
                "どの綴りが知らない引数だったかを言うこと: {raw:?} / {unknown}"
            );
        }
        // **`--upstream` の値として食べない**（旗は名前にならない）。
        assert!(parse_args([UPSTREAM_FLAG, NO_BROWSER_FLAG]).is_err());
        assert!(parse_args([UPSTREAM_FLAG, VERSION_FLAG]).is_err());
        assert!(parse_args([UPSTREAM_FLAG, NO_UI_FLAG]).is_err());
    }

    /// **退避先の名前は、DB 本体と `-wal` / `-shm` で同じ接尾辞を使う。**
    ///
    /// # ⚠ 期待値を [`backup_moves`] から作らない
    ///
    /// 綴りをここに**直に書く**。組み立て側から作ると、
    /// 接尾辞の形を変える変異と一緒に期待値が動く（CLAUDE.md `16b`）。
    #[test]
    fn the_backup_names_keep_the_sqlite_convention() {
        let moves = backup_moves(
            Path::new("/tmp/pc/personacore.sqlite"),
            "20260918T101112",
            &|_| false,
        )
        .expect("空いていれば決まること");

        assert_eq!(
            moves.pairs(),
            [
                (
                    PathBuf::from("/tmp/pc/personacore.sqlite"),
                    PathBuf::from("/tmp/pc/personacore.sqlite.bak-20260918T101112"),
                ),
                (
                    PathBuf::from("/tmp/pc/personacore.sqlite-wal"),
                    PathBuf::from("/tmp/pc/personacore.sqlite.bak-20260918T101112-wal"),
                ),
                (
                    PathBuf::from("/tmp/pc/personacore.sqlite-shm"),
                    PathBuf::from("/tmp/pc/personacore.sqlite.bak-20260918T101112-shm"),
                ),
            ]
        );
    }

    /// **宛先が1つでも塞がっていれば、3つとも次の番号へ**（レビュー指摘1）。
    ///
    /// # なぜ「1つでも」なのか
    ///
    /// **3つは同じ接尾辞を共有していないと、退避した組が SQLite の3点セットにならない。**
    /// **塞がっているのが `-wal` だけでも、DB 本体まで次の番号へ動かす。**
    ///
    /// **⚠ 述語で測る。** ファイルを作らないので、
    /// **「どの名前を見に行ったか」までここで固定できる。**
    #[test]
    fn a_taken_destination_pushes_the_whole_set_to_the_next_number() {
        let db = Path::new("/tmp/pc/personacore.sqlite");
        let stamp = "20260918T101112";
        let chosen = |taken: &dyn Fn(&Path) -> bool| {
            backup_moves(db, stamp, taken)
                .expect("空きがあること")
                .pairs()
                .iter()
                .map(|(_, to)| to.display().to_string())
                .collect::<Vec<_>>()
        };

        // **1回目**（何も塞がっていない）。
        assert_eq!(
            chosen(&|_| false),
            vec![
                "/tmp/pc/personacore.sqlite.bak-20260918T101112".to_owned(),
                "/tmp/pc/personacore.sqlite.bak-20260918T101112-wal".to_owned(),
                "/tmp/pc/personacore.sqlite.bak-20260918T101112-shm".to_owned(),
            ]
        );

        // **DB 本体の宛先だけが塞がっている** → 3つとも `-2` へ。
        let first_set = ["/tmp/pc/personacore.sqlite.bak-20260918T101112"];
        assert_eq!(
            chosen(&|path| first_set.contains(&path.display().to_string().as_str())),
            vec![
                "/tmp/pc/personacore.sqlite.bak-20260918T101112-2".to_owned(),
                "/tmp/pc/personacore.sqlite.bak-20260918T101112-2-wal".to_owned(),
                "/tmp/pc/personacore.sqlite.bak-20260918T101112-2-shm".to_owned(),
            ]
        );

        // **`-wal` の宛先だけが塞がっている**（本体は空いている）→ それでも3つとも `-2` へ。
        let wal_only = ["/tmp/pc/personacore.sqlite.bak-20260918T101112-wal"];
        assert_eq!(
            chosen(&|path| wal_only.contains(&path.display().to_string().as_str())),
            vec![
                "/tmp/pc/personacore.sqlite.bak-20260918T101112-2".to_owned(),
                "/tmp/pc/personacore.sqlite.bak-20260918T101112-2-wal".to_owned(),
                "/tmp/pc/personacore.sqlite.bak-20260918T101112-2-shm".to_owned(),
            ],
            "本体が空いていても、隣が塞がっていれば組ごと動かすこと"
        );

        // **全部塞がっている** → 空きが無いので `None`（**上書きしない**）。
        assert_eq!(
            backup_moves(db, stamp, &|_| true),
            None,
            "空きが無いなら、上書きせずに諦めること"
        );
    }

    /// **接尾辞は、その時刻をそのまま綴る**（`:` を使わない）。
    ///
    /// # ⚠ 2つの時刻で回す（レビュー指摘3）
    ///
    /// **1つの時刻でしか回していなかったので、引数を捨てて固定文字列を返す変異が
    /// 2096 全緑を通っていた**（レビュアーが実測）。
    /// **違う時刻が違う綴りになること**まで見ないと、「その時刻を綴った」は言えない。
    #[test]
    fn the_backup_stamp_spells_the_time_it_is_given() {
        use chrono::TimeZone;

        let other = chrono::Local
            .with_ymd_and_hms(2027, 1, 2, 3, 4, 5)
            .single()
            .expect("一意に決まる時刻であること");
        assert_ne!(fixed_now(), other, "前提: 2つは別の時刻であること");

        assert_eq!(backup_suffix(fixed_now()), "20260918T101112");
        assert_eq!(backup_suffix(other), "20270102T030405");
        assert_ne!(
            backup_suffix(fixed_now()),
            backup_suffix(other),
            "別の時刻は別の綴りになること（引数を捨てる実装はここで赤くなる）"
        );

        for stamp in [backup_suffix(fixed_now()), backup_suffix(other)] {
            assert!(
                !stamp.contains(':'),
                "Windows のファイル名に `:` は使えない: {stamp}"
            );
        }
    }

    /// **同じ秒に2回退避しても、1回目が消えない**（レビュー指摘1 / ADR-0041 §決定4）。
    ///
    /// # なぜこれが重大なのか
    ///
    /// **[`std::fs::rename`] は宛先を黙って上書きする。** 接尾辞は**秒まで**なので、
    /// **同じ秒に `--fresh` を2回走らせると、1回目に退避した利用者の DB が消える**
    /// ——**この設計で唯一の「削除の経路」**だった（ADR-0041 §決定4 が禁じたもの）。
    ///
    /// **中身まで見る**——名前が2つ在っても、両方が2回目の DB なら意味が無い。
    #[test]
    fn two_stashes_in_the_same_second_both_survive() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db = dir.path().join("personacore.sqlite");

        std::fs::write(&db, "FIRST-DB").expect("書けること");
        let first = stash_database(&db, fixed_now()).expect("1回目を退避できること");

        // **同じ時刻**（実運用では「同じ秒に2回叩いた」に当たる）。
        std::fs::write(&db, "SECOND-DB").expect("書けること");
        let second = stash_database(&db, fixed_now()).expect("2回目を退避できること");

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_ne!(
            first[0].1, second[0].1,
            "同じ秒でも、退避先は別の名前になること"
        );
        assert_eq!(
            std::fs::read_to_string(&first[0].1).expect("1回目の退避先を読めること"),
            "FIRST-DB",
            "1回目の退避が上書きされている（利用者のデータが消えた）"
        );
        assert_eq!(
            std::fs::read_to_string(&second[0].1).expect("2回目の退避先を読めること"),
            "SECOND-DB"
        );
    }

    /// **3ファイル揃っているときは、3つとも動く**（ADR-0041 §決定4）。
    ///
    /// # 何を見るか
    ///
    /// * **元の名前が無くなっていること**（残っていたら、退避していない）。
    /// * **退避先に、元の中身がそのまま在ること**（**削除する変異はここで赤くなる**）。
    /// * **`-wal` / `-shm` も一緒に動くこと**（片方だけ残すと、
    ///   次に開いた新しい DB へ古い WAL が当たる）。
    #[test]
    fn all_three_files_move_together() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db = dir.path().join("personacore.sqlite");
        // **3つとも違う中身**（同じにすると、取り違えが観測できない）。
        let bodies = [
            (db.clone(), "DB-BODY"),
            (dir.path().join("personacore.sqlite-wal"), "WAL-BODY"),
            (dir.path().join("personacore.sqlite-shm"), "SHM-BODY"),
        ];
        for (path, body) in &bodies {
            std::fs::write(path, body).expect("書けること");
        }

        let moved = stash_database(&db, fixed_now()).expect("退避できること");

        assert_eq!(moved.len(), 3, "3つとも動かすこと: {moved:?}");
        for (path, body) in &bodies {
            assert!(!path.exists(), "元の名前が残っている: {}", path.display());
            let to = moved
                .iter()
                .find(|(from, _)| from == path)
                .map(|(_, to)| to)
                .unwrap_or_else(|| panic!("{} の退避先があること: {moved:?}", path.display()));
            assert_eq!(
                std::fs::read_to_string(to).expect("退避先を読めること"),
                *body,
                "退避先に元の中身が残っていること（消してはいけない）: {}",
                to.display()
            );
        }
    }

    /// **DB 本体だけのときは、それだけ動く**（在らないものを探しに行かない）。
    #[test]
    fn only_the_database_moves_when_there_is_no_wal() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db = dir.path().join("personacore.sqlite");
        std::fs::write(&db, "DB-BODY").expect("書けること");

        let moved = stash_database(&db, fixed_now()).expect("退避できること");

        assert_eq!(moved.len(), 1, "動かすのは1つだけ: {moved:?}");
        assert!(!db.exists(), "元の名前が残っている");
        assert_eq!(
            std::fs::read_to_string(&moved[0].1).expect("退避先を読めること"),
            "DB-BODY"
        );
        // **隣のファイルを勝手に作らないこと。**
        assert!(!dir.path().join("personacore.sqlite-wal").exists());
    }

    /// **何も無ければ何もしない**（初回に `--fresh` を付けても失敗しない）。
    #[test]
    fn nothing_to_stash_is_not_a_failure() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db = dir.path().join("personacore.sqlite");

        let moved = stash_database(&db, fixed_now()).expect("失敗しないこと");

        assert!(moved.is_empty(), "動かしたものが無いこと: {moved:?}");
        // **退避先も作らない。**
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .expect("読めること")
            .map(|entry| entry.expect("項目を読めること").file_name())
            .collect();
        assert!(entries.is_empty(), "何も作らないこと: {entries:?}");
    }

    // ------------------------------------------------------------------
    // 途中で失敗したときの戻し（レビュー指摘7 / **植えた改名で3状態を測る**）
    // ------------------------------------------------------------------

    /// **3つとも実在する状態**を作り、その DB のパスと組を返す。
    ///
    /// 中身はどれも違う文字列（同じにすると、取り違えが観測できない）。
    fn three_files(dir: &Path) -> (PathBuf, names::Moves) {
        let db = dir.join("personacore.sqlite");
        for (name, body) in SEEDED {
            std::fs::write(dir.join(name), body).expect("書けること");
        }
        let moves = backup_moves(&db, "20260918T101112", &|_| false).expect("空いていること");
        (db, moves)
    }

    /// 3つのファイルと、その中身。
    const SEEDED: [(&str, &str); 3] = [
        ("personacore.sqlite", "DB-BODY"),
        ("personacore.sqlite-wal", "WAL-BODY"),
        ("personacore.sqlite-shm", "SHM-BODY"),
    ];

    /// **`n` 回目の改名だけ失敗する改名**（それ以外は本物）。
    ///
    /// **本物を通すのが肝**——植えた改名が何も動かさないと、
    /// 「戻っている」を実ファイルで確かめられない。
    fn rename_failing_at(nth: usize) -> impl Fn(&Path, &Path) -> std::io::Result<()> {
        let calls = std::cell::Cell::new(0usize);
        move |from: &Path, to: &Path| {
            calls.set(calls.get() + 1);
            if calls.get() == nth {
                Err(std::io::Error::other("植えた失敗"))
            } else {
                std::fs::rename(from, to)
            }
        }
    }

    /// **失敗しない改名なら、3つとも動く**（植えた改名の対照）。
    ///
    /// これが無いと、下の2本は「常に失敗する改名」でも成立する。
    #[test]
    fn the_injected_rename_moves_everything_when_it_succeeds() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let (_, moves) = three_files(dir.path());

        let moved = stash_with(&moves, rename_failing_at(0)).expect("成功すること");

        assert_eq!(moved.len(), 3);
        for (name, _) in SEEDED {
            assert!(!dir.path().join(name).exists(), "{name} が残っている");
        }
        assert_eq!(
            std::fs::read_to_string(&moved[0].1).expect("退避先を読めること"),
            SEEDED[0].1
        );
    }

    /// **2つ目（`-wal`）で失敗したら、1つ目（本体）が元の名前に戻る**
    /// （レビュー指摘7 / ADR-0041 §決定4「片方だけ残さない」）。
    ///
    /// # なぜこれが要るのか
    ///
    /// **戻さないと、退避されたのは本体だけで `-wal` はその場に残る。**
    /// **次に開いた新しい DB へ、古い WAL が当たる**——
    /// **ADR-0041 §決定4 が名指しした形そのもの**で、**見張り番は無かった。**
    #[test]
    fn a_failure_halfway_puts_the_first_file_back() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let (_, moves) = three_files(dir.path());

        let err = stash_with(&moves, rename_failing_at(2)).expect_err("失敗すること");

        assert!(
            matches!(err, FreshError::Rename(_)),
            "退避そのものの失敗として返すこと: {err:?}"
        );

        // **3つとも元の名前・元の中身で残っていること。**
        for (name, body) in SEEDED {
            assert_eq!(
                std::fs::read_to_string(dir.path().join(name))
                    .unwrap_or_else(|err| panic!("{name} が元の名前に無い: {err}")),
                body,
                "{name} の中身が変わっている"
            );
        }

        // **退避先は1つも残っていないこと**（本体だけ動いたままにしない）。
        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .expect("読めること")
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.contains(".bak-"))
            .collect();
        assert!(leftovers.is_empty(), "退避先が残っている: {leftovers:?}");
    }

    /// **戻すのにも失敗したら、両方の理由を持って止まる**（レビュー指摘7）。
    ///
    /// **ここへ来たときだけ 3点セットはばらけたまま残る。**
    /// **黙って `Ok` を返さないこと**が守りたい点。
    #[test]
    fn a_failing_rollback_reports_both_reasons() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let (_, moves) = three_files(dir.path());

        // **2回目（`-wal` の退避）で失敗 → 3回目（戻し）でも失敗**させる。
        let calls = std::cell::Cell::new(0usize);
        let err = stash_with(&moves, |from: &Path, to: &Path| {
            calls.set(calls.get() + 1);
            if calls.get() >= 2 {
                Err(std::io::Error::other(match calls.get() {
                    2 => "植えた失敗（退避）",
                    _ => "植えた失敗（戻し）",
                }))
            } else {
                std::fs::rename(from, to)
            }
        })
        .expect_err("失敗すること");

        let FreshError::Rollback(both) = &err else {
            panic!("取り消しの失敗として返すこと: {err:?}");
        };
        let detail = both.to_string();
        assert!(
            detail.contains("植えた失敗（退避）") && detail.contains("植えた失敗（戻し）"),
            "両方の理由を持つこと: {detail}"
        );
        // **外向きの `Display` は定数文字列**（詳細は `#[source]` 側だけ）。
        assert_eq!(err.to_string(), "初期化のための退避を取り消せませんでした");

        // **戻せなかったので、本体は退避先に在る**（事実をそのまま固定する）。
        assert!(
            !dir.path().join(SEEDED[0].0).exists(),
            "戻せていないのに元の名前に在る"
        );
    }

    /// **巻き戻しは、その名前を取った者を上書きしない**（レビュー3周目 / 作業単位 F-4a）。
    ///
    /// # 何が起きうるのか
    ///
    /// **`--fresh` は DB を開く前に走るが、同じ置き場所を指した別の personacore を
    /// 止める手段は無い。** だからこの窓が開く ——
    ///
    /// 1. 本体の退避に**成功する**（元の名前が空く）。
    /// 2. **別のプロセスが、空いた元の名前で新しい DB を作る。**
    /// 3. 次（`-wal`）の改名が**失敗する**。
    /// 4. 巻き戻しが `rename(退避先, 元の名前)` を呼ぶ ——
    ///    **`rename` は上書きするので、2 で作られた DB が消える。**
    ///    **しかも退避先も戻し切って消えるので、痕跡がゼロになる。**
    ///
    /// **ADR-0041 §実装8 の「`remove_file` が無いことは消えない理由にならない」は、
    /// 戻す向きにも当たる。** **初版はそれを退避の向きにしか当てていなかった。**
    ///
    /// # 何を固定するか
    ///
    /// **戻さずに止める**ので、次の3つが同時に成り立つ:
    ///
    /// * **割り込んだファイルの中身が無傷**（消さない）。
    /// * **退避先が残っている**（手で戻せる）。
    /// * **[`FreshError::Rollback`] が返る**（黙って `Ok` にしない）。
    #[test]
    fn a_rollback_never_overwrites_what_took_the_name() {
        /// 窓の中で、別のプロセスが元の名前に書いたことにする中身。
        const INTERLOPER: &str = "ANOTHER-PROCESS-DB";

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let (db, moves) = three_files(dir.path());
        assert_ne!(
            INTERLOPER, SEEDED[0].1,
            "前提: 割り込む中身は種と別であること"
        );

        // **1つ目は本物で動かし、その直後に元の名前を塞ぐ。2つ目は失敗させる。**
        let calls = std::cell::Cell::new(0usize);
        let err = stash_with(&moves, |from: &Path, to: &Path| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                let done = std::fs::rename(from, to);
                // **窓**: 退避で空いた名前を、別のプロセスが取る。
                std::fs::write(&db, INTERLOPER).expect("割り込みを書けること");
                done
            } else {
                Err(std::io::Error::other("植えた失敗"))
            }
        })
        .expect_err("失敗すること");

        // ③ **取り消しの失敗として返る**（`Rename` ではない）。
        let FreshError::Rollback(both) = &err else {
            panic!("取り消しの失敗として返すこと: {err:?}");
        };
        assert!(
            both.to_string().contains(ROLLBACK_DESTINATION_TAKEN),
            "戻す先が塞がっていたことを言うこと: {both}"
        );

        // ① **割り込んだファイルが無傷**（上書きして消していない）。
        assert_eq!(
            std::fs::read_to_string(&db).expect("割り込んだファイルを読めること"),
            INTERLOPER,
            "巻き戻しが、その名前を取った側を上書きした（痕跡がゼロになる）"
        );

        // ② **退避先が残っている**（手で戻せる）。
        let backup = &moves.pairs()[0].1;
        assert_eq!(
            std::fs::read_to_string(backup).expect("退避先を読めること"),
            SEEDED[0].1,
            "退避先まで消えている（元の DB が取り戻せない）"
        );
    }

    /// **壊れたリンクも「塞がっている」と読む**（レビュー指摘6）。
    ///
    /// **`Path::exists` はリンクを辿るので、壊れたリンクを「無い」と読む。**
    /// **その名前へ `rename` すれば、そのリンクは消える**——
    /// **`--fresh` が何かを消す、唯一の残った道**だった。
    ///
    /// **⚠ Windows でリンクを作るには権限が要る**ので、
    /// **作れない機ではこの検査は「作れなかった」と言って何も見ない。**
    /// **決定的な権威は [`a_taken_destination_pushes_the_whole_set_to_the_next_number`]**
    /// （植えた述語で回す）。
    #[test]
    fn a_broken_link_counts_as_taken() {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let broken = dir.path().join("broken-link");

        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(dir.path().join("nowhere"), &broken).is_ok();
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(dir.path().join("nowhere"), &broken).is_ok();

        if !made {
            // **リンクを作れない機**（Windows は権限が要る）。ここでは何も言わない。
            return;
        }

        assert!(!broken.exists(), "前提: 壊れたリンクであること（辿れない）");
        assert!(
            present(&broken),
            "壊れたリンクも「塞がっている」と読むこと（上書きすると、そのリンクが消える）"
        );
    }
}
