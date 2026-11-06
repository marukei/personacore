//! **呼ばれた引数を1行ずつ書き残して、すぐ終わる偽物**（作業単位 F-4c / Issue origin#23）。
//!
//! # なぜ要るのか（CLAUDE.md §4「本物を起こせない経路は、偽物で起こすこと」）
//!
//! **ブラウザを開く経路は、テストから本物では走らせない**（この機の EDR / テストのたびに
//! ブラウザが開く状態は受け入れられない）。**だからといって開く経路を1本も走らせないと、
//! その経路に見張り番が1本も無いのと同じ**になる（`F-9` の回帰の形）。
//!
//! **この偽物が2役を持つ:**
//!
//! | 役 | 置き方 | 何を見るか |
//! |---|---|---|
//! | **ブラウザを開くコマンド** | `PERSONACORE_TEST_BROWSER` に**この実行ファイルのパス**を渡す（`src/server/browser.rs` の doc） | **正しい URL で、1回だけ呼ばれたこと** |
//! | **`llama`** | `llama`（＋接尾辞）の名前で複製し、`PATH` の先頭に置く | **一度も呼ばれないこと**（呼ばれても、待ち受けずにすぐ終わる） |
//!
//! **偽の `llama`（`fake_llama.rs`）を2役目に使わない理由**: あちらは既定で**待ち受ける**ので、
//! **「呼ばれないはず」を破る変異のたびに、生きた偽物が残る**（`tests/upstream_cli.rs` の doc）。
//! **こちらは何を渡されても、書いてすぐ終わる。**
//!
//! # 書き方
//!
//! **環境変数 `FAKE_RECORDER_LOG` が指すファイルへ、引数の列を JSON の配列1行で足す。**
//! **1回の呼び出しが1行**なので、行数がそのまま呼ばれた回数になる。
//! **空の引数（Windows の `start ""` の題名）も落とさない**——JSON の配列なら `""` として残る。
//!
//! # ⚠ この実行ファイルは配布物に入らない
//!
//! `Cargo.toml` の `required-features = ["testing"]` で、**素の `cargo build` からは作られない。**

use std::io::Write;

/// 書き残す先を指す環境変数。
const LOG_ENV: &str = "FAKE_RECORDER_LOG";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(log) = std::env::var_os(LOG_ENV) else {
        eprintln!("fake recorder: {LOG_ENV} が無いので、書き残せません");
        std::process::exit(3);
    };
    let line = match serde_json::to_string(&args) {
        Ok(line) => line,
        Err(err) => {
            eprintln!("fake recorder: 引数を JSON にできません（{err}）");
            std::process::exit(4);
        }
    };
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .and_then(|mut file| writeln!(file, "{line}"));
    if let Err(err) = written {
        eprintln!("fake recorder: 書き残せません（{err}）");
        std::process::exit(5);
    }
}
