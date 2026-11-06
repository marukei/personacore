//! **新しい版で開いた DB を古い exe で起こすと、日本語で言って止まる**（B-55 / origin#36）。
//!
//! `src/main.rs` の終了経路（`anyhow` が `Caused by` として並べる鎖）を、
//! 実行ファイルそのもので確かめる。**LM Studio には繋がない。**
//!
//! **⚠ 今回の直しが効くのは、この版より後の exe だけ。** 配布済みの v0.1.4 以前の exe は
//! 英語の sqlx の文のまま止まる（振る舞いは変わらない）。
//!
//! **行の中で見る**: 「新しい版」の文言と、それが載る行が同じであること
//! （別の行に出た文言で満たされない）。

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

const MAX_WAITS: usize = 300;
const WAIT_STEP: Duration = Duration::from_millis(100);

/// 起こした exe の番人。`Drop` で kill して wait する（`tests/fresh_cli.rs` の `Running` と同じ）。
struct Running(std::process::Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn unknown_version_database(path: &Path) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime を作れること")
        .block_on(async {
            let store = personacore::store::Store::connect(path)
                .await
                .expect("開けること");
            sqlx::query(
                "INSERT INTO _sqlx_migrations \
                 (version, description, success, checksum, execution_time) \
                 VALUES (987654, 'from a newer version', 1, ?, 0)",
            )
            .bind(vec![7_u8; 48])
            .execute(store.pool())
            .await
            .expect("未知の版を足せること");
            store.close().await;
        });
}

#[test]
fn the_exit_path_says_in_japanese_that_the_database_is_from_a_newer_version() {
    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let db_dir = dir.path().join("db");
    std::fs::create_dir_all(&db_dir).expect("置き場所を作れること");
    let database = db_dir.join("personacore.sqlite");
    unknown_version_database(&database);

    let mut command = Command::new(env!("CARGO_BIN_EXE_personacore"));
    for (key, _) in std::env::vars() {
        if key.starts_with("PERSONACORE_") {
            command.env_remove(key);
        }
    }
    let child = command
        .env("LOCALAPPDATA", dir.path().join("localappdata"))
        .env("XDG_DATA_HOME", dir.path().join("xdg"))
        .env("HOME", dir.path().join("home"))
        .env("USERPROFILE", dir.path().join("home"))
        .env("PERSONACORE_UPSTREAMS", dir.path().join("none.json"))
        .env("PERSONACORE_DB_PATH", &database)
        .env("PERSONACORE_PORT", "0")
        .env("PERSONACORE_LLM_ENDPOINT", "http://127.0.0.1:9")
        .args(["--no-ui", "--no-browser"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("実行ファイルを起こせること");

    // **番人を通す**：`try_wait` が panic しても、上限に当たっても、`Drop` が止める。
    let mut child = Running(child);
    let mut status = None;
    for _ in 0..MAX_WAITS {
        if let Some(s) = child.0.try_wait().expect("状態を見られること") {
            status = Some(s);
            break;
        }
        std::thread::sleep(WAIT_STEP);
    }
    let Some(status) = status else {
        panic!("{MAX_WAITS} 回待っても終了しなかった");
    };
    let mut stderr = String::new();
    if let Some(mut pipe) = child.0.stderr.take() {
        use std::io::Read;
        let _ = pipe.read_to_string(&mut stderr);
    }

    assert!(!status.success(), "止まること（終了コード {status}）");
    let line = stderr
        .lines()
        .find(|line| line.contains("新しい版の personacore"))
        .unwrap_or_else(|| panic!("日本語の説明の行が無い:\n{stderr}"));
    assert!(
        line.contains("写しておいた保存先に戻してください"),
        "戻し方まで同じ行にあること: {line}"
    );
}
