//! **`--fresh` の結線を、実行ファイルそのもので確かめる**
//! （[ADR-0041](../docs/adr/0041-one-executable-per-os.md) §決定4 / 作業単位 F-4a /
//! レビュー指摘5）。**LM Studio には繋がない。**
//!
//! # なぜ実行ファイルを起こすのか
//!
//! **`src/main.rs` は bin なので、統合テストから `use` できない。**
//! **`if args.fresh()` を常に偽にする変異が 2096 全緑を通った**（レビュアーが実測）——
//! **`fresh` モジュールは厚く見ているのに、それを呼ぶ1行だけが誰も通らない場所**だった
//! （CLAUDE.md「配線だけの場所は、層に関係なく観測対象から落ちる」）。
//!
//! **`main` の中身を `server` 側へ1段出す形も採れたが、採らなかった。**
//! **出しても「`main` がその関数を呼ぶ1行」が同じ理由で無防備に残る**だけで、
//! **穴が1段ずれるだけ**である。**`env!("CARGO_BIN_EXE_personacore")` は
//! その1行まで通る唯一の道**なので、こちらを採った。
//!
//! # 何を守っていないか（**先に書く**）
//!
//! * **止めるのは `kill`。** Ctrl+C の経路（`shutdown_signal`）は通っていない。
//! * **画面にも API にも触っていない。** 見ているのは退避と引数の解釈だけ。
//! * **上流は閉じたポートを指す。** 繋がらないまま起動が続くこと自体は
//!   `tests/server_web_ui.rs` の側が見ている。

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// **上流に繋ぎに行かせない宛先**（閉じている loopback のポート）。
const CLOSED_UPSTREAM: &str = "http://127.0.0.1:9";

/// 待つ側の上限（回数）。**「働く側」ではなく「待つ側」に置く**（CLAUDE.md `G-4`）。
const MAX_WAITS: usize = 200;

/// 1回あたりの待ち時間。**上限は `MAX_WAITS` と合わせて 20 秒。**
const WAIT_STEP: Duration = Duration::from_millis(100);

/// 退避される3つのファイルと、その中身。**どれも違う文字列**
/// ——同じにすると、取り違えが観測できない。
const SEEDED: [(&str, &str); 3] = [
    ("personacore.sqlite", "DB-BEFORE-FRESH"),
    ("personacore.sqlite-wal", "WAL-BEFORE-FRESH"),
    ("personacore.sqlite-shm", "SHM-BEFORE-FRESH"),
];

/// 3つのファイルを置いた一時ディレクトリを作り、DB のパスを返す。
fn seeded_database(dir: &Path) -> PathBuf {
    let db = dir.join("db");
    std::fs::create_dir_all(&db).expect("置き場所を作れること");
    for (name, body) in SEEDED {
        std::fs::write(db.join(name), body).expect("書けること");
    }
    db.join(SEEDED[0].0)
}

/// **起こした実行ファイルの番人。`Drop` で必ず止める。**
///
/// # ⚠ なぜ要るのか（**レビュー指摘1 / 2026-09-18 に実測**）
///
/// **[`std::process::Child`] は drop しても kill しない。**
/// **`wait_until` が上限で `panic!` すると、その下の `stop` に到達しない**ので、
/// **`cargo test` が終わったあとも `personacore.exe --fresh` が生き残る**
/// （レビュアーが実測し、手で止めた）。
///
/// **`F-9` と同じ族** —— **起こしたものを番人に持たせていなかった。**
/// **`panic!` でも巻き戻しは走る**ので、`Drop` に置けば必ず通る。
struct Running(Child);

impl Running {
    /// **上限つきで終了を待ち、終了コードと stderr を返す。**
    ///
    /// # ⚠ なぜ `.output()` を使わないのか（**レビュー3周目の注記**）
    ///
    /// **[`std::process::Command::output`] は上限なしで待つ。**
    /// **いま止まっているのは「種の DB が SQLite として読めず exit 1 になる」
    /// フィクスチャのおかげ**で、**種を「開ける DB」に替えた日から無上限になる。**
    /// **待つ側に上限を置く**（CLAUDE.md `G-4`）——そして**番人を通す。**
    fn wait_for_exit(&mut self, what: &str) -> (std::process::ExitStatus, String) {
        for _ in 0..MAX_WAITS {
            if let Some(status) = self.0.try_wait().expect("プロセスの状態を見られること")
            {
                // **終了済みなので、stderr を EOF まで読んでも待たされない。**
                let mut stderr = String::new();
                if let Some(mut pipe) = self.0.stderr.take() {
                    use std::io::Read;
                    let _ = pipe.read_to_string(&mut stderr);
                }
                return (status, stderr);
            }
            std::thread::sleep(WAIT_STEP);
        }
        panic!("{what} を {MAX_WAITS} 回待っても終了しなかった");
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // **失敗しても何もしない。** 既に終わっていれば `kill` は `Err` を返す。
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// **親から `PERSONACORE_*` を1つも継がせない**（レビュー指摘8）。
///
/// **測定中の機には `PERSONACORE_STAT_DRIFT` などが立っていることがある。**
/// 継ぐと、**このテストの結果が「誰がどの環境変数を立てていたか」に左右される。**
///
/// **綴りの帳簿を持たない** —— **接頭辞で全部落とす。**
/// `config.rs` の `ENV_*` が増えても、ここへ足す作業が要らない。
fn without_inherited_settings() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_personacore"));
    for (key, _) in std::env::vars() {
        if key.starts_with("PERSONACORE_") {
            command.env_remove(key);
        }
    }
    command
}

/// 実行ファイルを起こす。**ポートは 0（OS に選ばせる）。**
///
/// **ログは捨てる**（このファイルはログを読まない。読むなら行ごとに見ること）。
fn spawn(database: &Path, args: &[&str]) -> Running {
    spawn_with_stderr(database, args, Stdio::null())
}

/// **stderr を捕まえて**起こす（`--Fresh` の腕が、綴りを読むために使う）。
fn spawn_capturing_stderr(database: &Path, args: &[&str]) -> Running {
    spawn_with_stderr(database, args, Stdio::piped())
}

/// 起こす本体。**3本とも [`Running`] を通る**（起こしたものを必ず番人が持つ）。
fn spawn_with_stderr(database: &Path, args: &[&str], stderr: Stdio) -> Running {
    let root = database
        .parent()
        .and_then(Path::parent)
        .expect("DB は <一時ディレクトリ>/db/ の下にあること");
    Running(
        without_inherited_settings()
            // **利用者の本物の置き場所へ届く道を断つ**（作業単位 F-4c）。
            // **引数なしの起動は、置き場所に上流プロファイルの見本を書く**ので、
            // **断たないと `%LOCALAPPDATA%\personacore\upstreams.json` を作る。**
            // **ブラウザは開かない**——`cargo test` が組む実行ファイルは、
            // `PERSONACORE_TEST_BROWSER` が無ければ何も起こさない（`src/server/browser.rs`）。
            .env("LOCALAPPDATA", root.join("localappdata"))
            .env("XDG_DATA_HOME", root.join("xdg"))
            .env("HOME", root.join("home"))
            .env("USERPROFILE", root.join("home"))
            .args(args)
            .env("PERSONACORE_DB_PATH", database)
            .env("PERSONACORE_PORT", "0")
            .env("PERSONACORE_LLM_ENDPOINT", CLOSED_UPSTREAM)
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .expect("実行ファイルを起こせること"),
    )
}

/// 退避先（`.bak-` を含む名前）を集める。
fn backups(db_dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(db_dir)
        .expect("置き場所を読めること")
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.contains(".bak-"))
        .collect();
    found.sort();
    found
}

/// **条件が成り立つまで、上限つきで待つ。**
///
/// **毎周「書き手が生きているか」を見る**（CLAUDE.md `G-4`）——
/// **プロセスが死んだ瞬間、条件は永久に来ない。**
/// 上限に当たったら、そう言って落とす（無限待機にしない）。
///
/// **⚠ ここで `panic!` しても、起こしたものは [`Running`] の `Drop` が止める。**
fn wait_until(running: &mut Running, what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..MAX_WAITS {
        if ready() {
            return;
        }
        match running.0.try_wait().expect("プロセスの状態を見られること") {
            Some(status) => panic!("{what} を待つ前にプロセスが終了した（{status}）"),
            None => std::thread::sleep(WAIT_STEP),
        }
    }
    panic!("{what} を {MAX_WAITS} 回待っても観測できなかった");
}

/// **`--fresh` を付けて起こすと、3つとも退避される**（ADR-0041 §決定4）。
///
/// **`main` の `if args.fresh()` を通る唯一の検査。**
#[test]
fn the_fresh_flag_stashes_the_database_before_the_server_opens_it() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let database = seeded_database(dir.path());
    let db_dir = database.parent().expect("親があること").to_path_buf();

    // **前提**——まだ退避先が無いこと。
    assert!(backups(&db_dir).is_empty(), "前提: 退避先がまだ無いこと");

    let mut running = spawn(&database, &["--fresh"]);
    wait_until(&mut running, "退避先が現れること", || {
        backups(&db_dir).len() == SEEDED.len()
    });
    drop(running);

    let moved = backups(&db_dir);
    assert_eq!(moved.len(), 3, "3つとも退避されること: {moved:?}");

    // **中身まで見る**——名前が3つ在っても、中身が違えば退避ではない。
    for (name, body) in SEEDED {
        let backup = moved
            .iter()
            .find(|candidate| {
                // `personacore.sqlite-wal` → `…bak-<時刻>-wal`
                match name.strip_prefix(SEEDED[0].0) {
                    Some("") => !candidate.ends_with("-wal") && !candidate.ends_with("-shm"),
                    Some(suffix) => candidate.ends_with(suffix),
                    None => false,
                }
            })
            .unwrap_or_else(|| panic!("{name} の退避先が無い: {moved:?}"));
        assert_eq!(
            std::fs::read_to_string(db_dir.join(backup)).expect("退避先を読めること"),
            body,
            "{name} の中身が退避先に残っていない（消してはいけない）"
        );
    }
}

/// **対比: `--fresh` を付けなければ、何も退避しない。**
///
/// **これが無いと「常に退避する」実装がここで緑を通る**
/// （CLAUDE.md「対比は安い側だけでは足りない」）。
///
/// # ⚠ ここだけ**本物の DB** を置く
///
/// **上の腕の種は SQLite として読めない綴り**でよい（開く前に退けられるので）。
/// **こちらは退けないので、開ける DB でないと起動が exit 1 で終わる**
/// ——**実際にそうなった**（最初の版は種をそのまま使って落ちた）。
///
/// **待つ相手は `-wal` が出来ること**——`Store` は WAL で開くので、
/// **起動が DB まで進んだ確かな印**になる（ポートを知らなくてよい）。
#[test]
fn without_the_flag_nothing_is_stashed() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let db_dir = dir.path().join("db");
    std::fs::create_dir_all(&db_dir).expect("置き場所を作れること");
    let database = db_dir.join(SEEDED[0].0);

    // **開ける DB を先に作る**（マイグレーション済みの本物）。
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime を作れること")
        .block_on(async {
            let store = personacore::store::Store::connect(&database)
                .await
                .expect("DB を作れること");
            store.close().await;
        });
    let wal = db_dir.join(SEEDED[1].0);
    let _ = std::fs::remove_file(&wal);

    // **前提**——DB は在り、`-wal` はまだ無いこと。
    assert!(database.is_file(), "前提: 開ける DB が在ること");
    assert!(!wal.exists(), "前提: -wal がまだ無いこと");

    let mut running = spawn(&database, &[]);
    wait_until(&mut running, "DB が開かれること", || wal.exists());
    drop(running);

    assert!(
        backups(&db_dir).is_empty(),
        "旗が無いのに退避している: {:?}",
        backups(&db_dir)
    );
}

/// **知らない引数は、起動せずに終わる**（ADR-0041 §決定4）。
///
/// **`--fresh` の綴り間違いを黙って無視すると、利用者は初期化したつもりで
/// 古い DB のまま遊び始める。**
#[test]
fn a_misspelt_flag_stops_the_executable() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let database = seeded_database(dir.path());
    let db_dir = database.parent().expect("親があること").to_path_buf();

    let mut running = spawn_capturing_stderr(&database, &["--Fresh"]);
    let (status, stderr) = running.wait_for_exit("知らない引数で終了すること");

    assert!(
        !status.success(),
        "知らない引数で起動してしまった（{status:?}）"
    );
    assert!(
        stderr.contains("--Fresh"),
        "どの綴りが知らない引数だったかを言うこと: {stderr}"
    );
    assert!(
        backups(&db_dir).is_empty(),
        "止まったのに退避している: {:?}",
        backups(&db_dir)
    );
    // **DB も開いていないこと**（起動そのものが始まっていない）。
    assert_eq!(
        std::fs::read_to_string(&database).expect("DB を読めること"),
        SEEDED[0].1,
        "止まるはずの起動が DB を触っている"
    );
}
