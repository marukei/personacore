//! **偽の `llama`**（[ADR-0042](../../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)
//! §見張り番 / 作業単位 F-4b-1）。
//!
//! # なぜ要るのか（CLAUDE.md §4）
//!
//! > **本物を起こせない経路は、偽物（何もしない同名の実行ファイルを `PATH` の先頭に置く）で
//! > 起こすこと。** 「走らせられないから走査だけ」は、その経路に見張り番が1本も無いのと同じ。
//!
//! **`F-9` はこれを守らず、実機で回帰を出した**（起こす経路が死に、`llama` が
//! GPU を 11.7 GB 掴んだまま残った）。**本物の `llama serve` は GPU を掴むので走らせない。**
//! **こちらは GPU も VRAM も1バイトも使わない。**
//!
//! # 受ける形は本物と同じ
//!
//! ```text
//! llama serve -m <gguf> --host <h> --port <p> [args…]
//! ```
//!
//! **`--pc-` で始まる引数だけが、この偽物の腕を選ぶ**（本物には無い綴り）。
//!
//! | 腕 | 何をするか |
//! |---|---|
//! | （既定） | 待ち受けて、`/v1/models` に **`-m` の綴りそのもの**を並べる |
//! | `--pc-answer <id>` | **別の名前**を名乗る（「載ったのは別のモデル」の腕） |
//! | `--pc-exit <code>` | **待ち受けずに即終了**（「子が自分で終了した」の腕） |
//! | `--pc-touch <path>` | 起きた印にそのファイルを作る。**中身は受け取った引数列そのもの**（**一度も呼ばれないこと**と、**`--host` / `--port` に何が渡ったか**を見る腕） |
//! | `--pc-ctx <N>` | `/v1/models` の `meta.n_ctx` に `N` を名乗る（**載ったら測り直す**ことを見る腕） |
//! | `--pc-exit-after <秒>` | 待ち受けたまま、その秒数で**自分で終了する**（実行ファイル経由の腕が `kill` しか送れないので、**生きた偽物に上限を付ける**） |
//! | `--pc-loading <秒>` | 起きてからその秒数のあいだ、**どの綴りにも 503 を返す**（「載せている最中」の腕 / 作業単位 B-51）。返すたびに標準出力へ `LOADING_LINE` を1行書く |
//!
//! **⚠ `--host` にワイルドカード（`0.0.0.0` / `::` 等）が1つでもあれば、待ち受けずに終わる**
//! （終了コード 97 / 作業単位 F-4b-2）。**変異検査で personacore の検証を外しても、
//! この機にワイルドカードの待ち受けは作られない。**
//!
//! # ⚠ `/api/v0/models` は 404 を返す
//!
//! **本物の `llama-server` がそうだから**（作業単位 G-2 が実測）。
//! personacore は1つ目で 404 を受けて `/v1/models` へ読み替える
//! （`llm::LmStudioClient::models`）——**その経路ごと通す。**
//!
//! # ⚠ この実行ファイルは配布物に入らない
//!
//! `Cargo.toml` の `required-features = ["testing"]` で、
//! **素の `cargo build` からは作られない。**

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

/// 名乗る綴りを差し替える腕。
const ANSWER_FLAG: &str = "--pc-answer";
/// 即終了する腕。
const EXIT_FLAG: &str = "--pc-exit";
/// 起きた印を作る腕。
const TOUCH_FLAG: &str = "--pc-touch";
/// **窓（`meta.n_ctx`）を名乗る腕。**
///
/// **本物の `llama-server` は名乗る。** 名乗らないと、personacore は
/// **実効コンテキスト長を採れない**ので、予算が落とし所のまま動かない
/// ——それでは「載ったら採る」経路を一度も通らない。
const CTX_FLAG: &str = "--pc-ctx";
/// **しばらく答えてから、自分で終わる腕。**
///
/// **実行ファイルを起こすテストは、強制終了しかできない**ので、
/// **そのままだと偽物が孤児として残る**（`tests/upstream_cli.rs` の doc）。
/// **自分から終わる腕を用意しておけば、起こす経路を通しても残さずに済む。**
const EXIT_AFTER_FLAG: &str = "--pc-exit-after";
/// **載せている最中を演じる腕**（作業単位 B-51）。
///
/// **本物の `llama-server` は、待ち受けを開いてからモデルを読む。**
/// 読み終わるまでは**どの綴りにも** `503`（`Loading model`）を返す
/// ——`/api/v0/models` も `/v1/models` も。**両方の読み手が失敗する形**を作るのはこの腕だけ。
const LOADING_FLAG: &str = "--pc-loading";

/// **503 を返したたびに標準出力へ書く1行**（`upstream-logs/*.out.log` に落ちる）。
///
/// **見る側（`tests/upstream_launch.rs`）はこの綴りで「番人が本当に 503 を受けた」を確かめる**
/// ——書かれていなければ、「WARN が出ない」は 503 を一度も受けていないだけでも成立する。
const LOADING_LINE: &str = "fake llama: 載せている最中なので 503 を返しました";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let value_of = |flag: &str| -> Option<String> {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };

    // **起きた印は、いちばん先に付ける。** 即終了の腕でも残る。
    if let Some(path) = value_of(TOUCH_FLAG) {
        let _ = std::fs::write(&path, args.join(" "));
    }

    // **即終了の腕は、待ち受けずに帰る**（`HasExited` の相当）。
    if let Some(code) = value_of(EXIT_FLAG) {
        eprintln!("fake llama: 即終了します（{code}）");
        std::process::exit(code.parse().unwrap_or(1));
    }

    // **ワイルドカードでは絶対に待ち受けない**（作業単位 F-4b-2 / Issue origin#2）。
    //
    // **この機にワイルドカードの待ち受けを作らない**——変異検査で personacore の検証を外すと、
    // **この偽物は `--host 0.0.0.0` を受け取りうる。** 検証が効いているかは
    // 「偽物が一度も呼ばれない（印が無い）」で見るので、**呼ばれたら待ち受けずに終わればよい。**
    // **`--host` は全部見る**（`=` で繋いだ形も）——下の `value_of` は最初の1つしか読まない。
    if let Some(wildcard) = wildcard_host(&args) {
        eprintln!("fake llama: ワイルドカード（{wildcard}）では待ち受けません");
        std::process::exit(WILDCARD_EXIT);
    }

    let host = value_of("--host").unwrap_or_else(|| "127.0.0.1".to_owned());
    let port = value_of("--port").unwrap_or_else(|| "0".to_owned());
    // **既定は `-m` の綴りそのもの。** 本物の `llama-server` は gguf の絶対パスを名乗る。
    let model = value_of(ANSWER_FLAG)
        .or_else(|| value_of("-m"))
        .unwrap_or_else(|| "unknown".to_owned());

    // **窓は起動時に1度だけ読む。** 応答のたびに読み直しても同じ答えになる。
    let context_length = value_of(CTX_FLAG);

    // **載せている最中の締め切り。起きた時点から数える**（待ち受けより前に決める）。
    let loading_until = value_of(LOADING_FLAG)
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(|seconds| std::time::Instant::now() + std::time::Duration::from_secs(seconds));

    // **決められた秒数で自分を終わらせる**（`--pc-exit-after`）。
    // **待ち受けの `incoming()` は止まらない**ので、別のスレッドから丸ごと終える。
    if let Some(seconds) = value_of(EXIT_AFTER_FLAG).and_then(|raw| raw.parse::<u64>().ok()) {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(seconds));
            eprintln!("fake llama: {seconds} 秒たったので終わります");
            std::process::exit(0);
        });
    }

    let listener = match TcpListener::bind(format!("{host}:{port}")) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("fake llama: 待ち受けできません（{err}）");
            std::process::exit(2);
        }
    };
    println!("fake llama: {host}:{port} で待ち受けます（{model}）");

    // **1本ずつ受ける。** 起動時の照会は直列なので、これで足りる。
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut reader = BufReader::new(match stream.try_clone() {
            Ok(clone) => clone,
            Err(_) => continue,
        });
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            continue;
        }
        // 残りのヘッダを読み捨てる（本文は受け取らない）。
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) if line.trim().is_empty() => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }

        // **`/v1/models` だけ答える。** `/api/v0/models` は 404
        // ——本物の `llama-server` と同じ（作業単位 G-2）。
        let path = request_line.split_whitespace().nth(1).unwrap_or("");
        let loading = loading_until.is_some_and(|until| std::time::Instant::now() < until);
        let response = if loading {
            // **本物の `llama-server` が読み込み中に返す本文と同じ形。**
            println!("{LOADING_LINE}（{path}）");
            let body =
                r#"{"error":{"code":503,"message":"Loading model","type":"unavailable_error"}}"#;
            format!(
                "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        } else if path.ends_with("/v1/models") {
            // **窓を名乗る腕**（`--pc-ctx`）。名乗らなければ `meta` ごと出さない
            // ——「長さを持っていない上流」も本物に居るので、両方を作れるようにしておく。
            let meta = match &context_length {
                Some(tokens) => format!(",\"meta\":{{\"n_ctx\":{tokens}}}"),
                None => String::new(),
            };
            let body = format!(
                "{{\"object\":\"list\",\"data\":[{{\"id\":{},\"object\":\"model\"{meta}}}]}}",
                json_string(&model)
            );
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        } else {
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned()
        };
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }
}

/// ワイルドカードを渡されたときの終了コード（**本物の `llama` には無い意味**）。
const WILDCARD_EXIT: i32 = 97;

/// **引数列のどこかに、ワイルドカードの `--host` があるか。**
///
/// `--host <値>` と `--host=<値>` の両方を、**全部**見る。
fn wildcard_host(args: &[String]) -> Option<String> {
    let mut values = Vec::new();
    for (at, arg) in args.iter().enumerate() {
        if arg == "--host" {
            if let Some(value) = args.get(at + 1) {
                values.push(value.clone());
            }
        } else if let Some(value) = arg.strip_prefix("--host=") {
            values.push(value.to_owned());
        }
    }
    values.into_iter().find(|value| {
        let bare = value.trim_start_matches('[').trim_end_matches(']');
        bare.is_empty()
            || bare == "*"
            || bare
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_unspecified())
    })
}

/// **JSON の文字列として綴る。** gguf の絶対パスには `\` が入る（Windows）ので、
/// **素で埋めると JSON が壊れる。**
fn json_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for ch in raw.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
