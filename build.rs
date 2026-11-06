//! `migrations/` の変更を cargo に追跡させる。
//!
//! `sqlx::migrate!` は各 SQL を `include_str!` 相当で展開するため、既存ファイルの
//! **内容変更は追跡されるが、ファイルの追加は追跡されない**。追加した直後は
//! 再コンパイルが起きず、**古いスキーマのままテストが緑になる**。
//!
//! ここで `migrations` ディレクトリを再実行トリガに登録すると、cargo が配下を再帰的に
//! 走査するため、**ファイルの追加でもビルドスクリプトが再実行され、lib ごと再コンパイル
//! される**。`cargo clean -p personacore` や `.rs` の touch は不要になる。
//!
//! 効果は実測で確認済み: `0003_probe.sql` を足して `.rs` を一切触らずに `cargo test`
//! したとき、build.rs なしでは新しいマイグレーションが**無視されて緑のまま**だったのに対し、
//! build.rs ありでは適用されてテーブル数のテストが落ちた。

//! # 画面（`web/dist`）の埋め込み（ADR-0041 §決定2 / 作業単位 F-4a）
//!
//! `src/server/web.rs` の `include_dir!` が `web/dist` を**コンパイル時に**読む。
//! ここでやることは2つ:
//!
//!   * **`web/dist` が無ければ空で作る。** **`web/dist` は `.gitignore` に入る
//!     ビルド成果物**なので、clone した直後には無い。
//!     無いまま `include_dir!` に渡すとコンパイルが落ち、
//!     **素の `cargo build` が門でなくなる**（ADR-0041 §決定2 の却下した案）。
//!     **`cargo test` は FE をビルドしない**ので、この形でないと成り立たない。
//!   * **`web/dist` を再実行トリガに登録する。** **要るのは「追加」と「削除」のため。**
//!
//! # ⚠ 何が登録で拾われ、何が拾われないのか（**4通りとも実測** / 2026-09-18）
//!
//! > **初版は「`include_dir!` はファイルの追加も内容の変更も cargo に伝えない」と書いていた。**
//! > **半分だけ正しかった**（内容の変更は伝わる）。
//! > **⚠ 2周目に書き直した「削除は伝わらない」も偽だった** ——
//! > **1周目に測ったのは上の2行だけで、削除は測っていなかった。**
//! > **測ったら、削除は伝わる**（dep-info に載った綴りが消えて `stat` に失敗し、dirty になる）。
//!
//! **`rerun-if-changed=web/dist` を落とした状態で `cargo build` を回した結果**
//! （`Compiling personacore` が出たか）:
//!
//! | 変えたもの | 組み直されるか | なぜ |
//! |---|---|---|
//! | **既存ファイルの中身** | **される** | `include_dir!` が `include_bytes!` を展開し、rustc が dep-info に載せる |
//! | **ファイルの追加** | **されない** | 新しいファイルは、まだどの `include_bytes!` にも現れていない |
//! | **ファイルの削除** | **される** | dep-info に載った綴りが消える＝`stat` に失敗して dirty |
//! | **削除して、また戻す** | **されない** | 削除で一度組み直した後、dep-info はもうその綴りを持たない |
//!
//! **⚠ 4行目がいちばん意地が悪い。** **`web/dist` を作り直す操作そのもの**
//! （`vite build` は `emptyOutDir` で消してから書く）**が、この形になる。**
//!
//! **`vite build` は名前にハッシュを入れる**（`assets/index-<ハッシュ>.js`）ので、
//! **画面を直すと、消えた綴りと新しい綴りが出る。**
//! **消えた側は伝わるが、新しい側は伝わらない**ので、
//! **登録を落とすと「組み直したのに、新しい画面が入っていない」**という形になる。
//! **「Finished in 0.4s は直っていない」と同じ罠が、画面の側に出る。**
//!
//! **⚠ 「削除で組み直されるから要らない」と読まないこと。**
//! **組み直しが起きることと、新しい中身が入ることは別である。**

//! # `--version` の綴り（作業単位 F-4c / Issue origin#23）
//!
//! **`git describe --always --tags` の結果を `PERSONACORE_GIT_DESCRIBE` に入れる**
//! （`server::VERSION_LINE` が `env!` で読む）。**配布物がどのコミットか言えるように。**
//!
//! | 場合 | 綴り |
//! |---|---|
//! | リポジトリの中で組んだ | `git describe` の出力（タグが無ければ短いハッシュ） |
//! | `git` が無い／失敗した | `unknown` |
//! | **このパッケージの根がリポジトリの根でない**（他人のリポジトリの中へ複製した等） | `unknown` |
//!
//! **3行目は、嘘を言わないため。** 複製先が別のリポジトリの中だと、
//! `git describe` は**そのリポジトリの**コミットを答える。
//!
//! # ⚠ `--version` の綴りが古びる経路（**どこまで許すか**）
//!
//! **build.rs は `rerun-if-changed` に挙げたものが変わったときしか走らない**（上の2行を含む）。
//! **何も足さなければ、コミットが進んでも綴りは組んだ日のまま**になる。
//! **だから次の4つを足す**（**実在するものだけ**——実在しない綴りを挙げると、
//! cargo は毎回 build.rs を走らせ、毎回 lib ごと組み直す）:
//!
//! * **`HEAD`**（ワークツリーごとの。`git rev-parse --git-path HEAD`）——**枝の切り替え・detached の checkout**
//! * **`logs/HEAD`**（`HEAD` の reflog）——**`HEAD` が動くたびに1行足される**（コミット・amend・reset・checkout）
//! * **`HEAD` が指す枝の ref**（loose）——**コミット・amend・reset**（reflog を切った機のため）
//! * **`packed-refs`**——**`git gc` で ref が詰められたとき**（loose が消える＝それ自体も変化として拾われる）
//!
//! > **⚠ 2つ目が要る理由**（2026-09-23 / 使い捨ての clone で実測）——
//! > **`git pack-refs` で loose の ref が消えた後、build.rs は「在るものだけ」を挙げ直すので
//! > loose を監視から外す。その後のコミットは loose を作り直すが、監視の外である。**
//! > **`HEAD` そのものは `ref: refs/heads/…` のまま変わらない。** 拾えるのは reflog だけ。
//! > **`logs/HEAD` を外す変異では、`pack-refs` → 空のコミットの後も古い綴りのまま**
//! > （組み直しが起きない）。**入れた形では新しい綴りになる。**
//!
//! **実測**（使い捨ての clone / 2026-09-23。手順と結果は作業者の報告）:
//! 空のコミット・detached の checkout で組み直されて綴りが変わる／何も変えなければ組み直さない／
//! **監視の行を丸ごと消す変異では、空のコミットの後も古い綴りのまま**（組み直しが起きない）。
//! **⚠ この4つの監視に、`cargo test` の中の見張り番は無い**——コミットを作る操作を
//! テストから走らせない（`tests/upstream_cli.rs` の `--version` の腕は「いまの `HEAD` を指すか」を
//! 見るので、**古びていればその日に赤になる**が、古びる操作そのものはテストの中で起きない）。
//!
//! **それでも古びる（許す）もの:**
//!
//! | 起きたこと | 綴り | 許す理由 |
//! |---|---|---|
//! | **コミットせずにソースを変えた** | 組んだ時点の `HEAD` を言う（`-dirty` を付けない） | **`--dirty` を付けても、作業ツリーの変化は build.rs を走らせない**ので、付けた日から嘘になりうる。**付けない方が「どのコミットの上か」だけを正しく言う** |
//! | **後からタグを打った** | 打つ前の綴り（短いハッシュ）のまま | **同じコミットを指している**（`git rev-parse` で引ける）。次に上の4つのどれかが動けば直る |
//! | **組んだときに `git` が無く、後から入れた** | `unknown` のまま | **`git` の有無は監視できない。** 配布物を組む機（`scripts/package.ps1`）には在る |
//!
//! **⚠ `migrations` と `web/dist` の2行（上）は消さない・意味を変えない。**
//! **足した行は「在るものだけを足す」**ので、2行の効き方は1バイトも変わらない
//! （どれか1つが変われば build.rs が走る、という和の関係のまま）。

use std::path::{Path, PathBuf};
use std::process::Command;

/// `git` が使えないときの綴り。
const UNKNOWN_VERSION: &str = "unknown";

/// **`git` を1回走らせ、成功したら標準出力を返す。** 失敗は全部 `None`（ビルドを止めない）。
fn git(manifest_dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// **このパッケージの根が、リポジトリ（かワークツリー）の根か。**
fn is_repository_root(manifest_dir: &Path) -> bool {
    let Some(top) = git(manifest_dir, &["rev-parse", "--show-toplevel"]) else {
        return false;
    };
    match (
        std::fs::canonicalize(top.trim()),
        std::fs::canonicalize(manifest_dir),
    ) {
        (Ok(top), Ok(here)) => top == here,
        _ => false,
    }
}

/// **`git describe --always --tags`。** 使えなければ [`UNKNOWN_VERSION`]。
///
/// **綴りは印字できる ASCII だけを通す**（`env!` で埋めて `--version` がそのまま印字する）。
fn describe(manifest_dir: &Path) -> String {
    git(manifest_dir, &["describe", "--always", "--tags"])
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty() && raw.chars().all(|ch| ch.is_ascii_graphic()))
        .unwrap_or_else(|| UNKNOWN_VERSION.to_owned())
}

/// **`--version` の綴りを動かしうる git のファイル**（実在するものだけ）。
fn git_files_to_watch(manifest_dir: &Path) -> Vec<PathBuf> {
    let mut names = vec![
        "HEAD".to_owned(),
        "logs/HEAD".to_owned(),
        "packed-refs".to_owned(),
    ];
    // **detached なら枝は無い**（`HEAD` そのものにハッシュが書いてある）。
    if let Some(branch) = git(manifest_dir, &["symbolic-ref", "-q", "HEAD"]) {
        names.push(branch.trim().to_owned());
    }
    names
        .iter()
        .filter_map(|name| git(manifest_dir, &["rev-parse", "--git-path", name]))
        .map(|raw| {
            let path = PathBuf::from(raw.trim());
            // **相対で返ることがある**（通常のリポジトリでは `.git/HEAD`）。基準は `current_dir`。
            if path.is_absolute() {
                path
            } else {
                manifest_dir.join(path)
            }
        })
        .filter(|path| path.is_file())
        .collect()
}

fn main() {
    println!("cargo:rerun-if-changed=migrations");

    // **`--version` の綴り**（上の「`--version` の綴り」）。
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let version = if is_repository_root(manifest_dir) {
        for watched in git_files_to_watch(manifest_dir) {
            println!("cargo:rerun-if-changed={}", watched.display());
        }
        describe(manifest_dir)
    } else {
        UNKNOWN_VERSION.to_owned()
    };
    println!("cargo:rustc-env=PERSONACORE_GIT_DESCRIBE={version}");

    // **画面の埋め込み元**（`server::config::DEFAULT_WEB_DIR` と同じ綴り）。
    // **ここは build.rs なので、lib の定数を参照できない**——
    // 綴りの一致は `server::web` の `the_embedded_ui_comes_from_the_default_web_dir`
    // が `include_dir!` の側で見張っている。
    let web_dist = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("web")
        .join("dist");
    if !web_dist.is_dir() {
        // **失敗しても止めない。** 止めると、書けない置き方をした瞬間に
        // ビルドそのものが落ちる。落ちるべきなのは `include_dir!` の側で、
        // そこには「画面が無い」という正しい理由が出る。
        let _ = std::fs::create_dir_all(&web_dist);
    }
    println!("cargo:rerun-if-changed=web/dist");
}
