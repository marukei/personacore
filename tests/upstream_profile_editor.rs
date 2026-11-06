//! **画面から上流プロファイルを読む・足す・直す**（[ADR-0042](../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)
//! §決定8 / 作業単位 F-4b-3 / Issue origin#24）を、**本番の入口（`App::run_with_upstreams`）で**確かめる。
//!
//! **本物の `llama` にも LM Studio にも繋がない。** 起動時の計画は空（1つも起こさない）で、
//! 上流の宛先は閉じたポート（[`CLOSED_ENDPOINT`]）。**保存するプロファイルに gguf を書いても、
//! 差し替えを頼まない限り起こされない**（このファイルが頼む差し替えは、gguf の無いものだけ）。
//!
//! **利用者の本物の置き場所（`%LOCALAPPDATA%\personacore\`）には触らない** ——
//! 置き場所はどれも一時ディレクトリで、`Startup` に直に渡す。
//! **`main` が「既定なら書ける／`PERSONACORE_UPSTREAMS` なら読むだけ」と決める1行は
//! `tests/upstream_cli.rs` が実行ファイルで見る**（このファイルからは観測できない）。
//!
//! # ここが見ているもの
//!
//! | 見張り番 | 何が壊れたら赤になるか |
//! |---|---|
//! | `a_saved_profile_reads_back_as_it_was_sent` | 保存 → 読み直しで内容が一致しない |
//! | `an_edit_that_fails_the_loader_leaves_the_file_byte_for_byte` | 検証に通らない入力が書かれる／理由が返らない |
//! | `a_stale_revision_is_refused_and_the_file_is_untouched` | 印の食い違いで上書きする |
//! | `a_read_only_location_refuses_to_write` | 読むだけの置き場所に書く |
//! | `the_listing_returns_the_gguf_as_written` | 一覧の gguf が展開される（観測した値と混ざる） |
//! | `editing_one_profile_leaves_the_other_profile_and_face_alone` | 1つを直すと他の値が変わる（帰属） |
//! | `a_replacement_that_fails_on_disk_keeps_the_original`（Windows） | 本物の置き換えの失敗で元のファイルが壊れる |
//! | `the_switch_selects_a_profile_that_was_just_saved` | 画面が書くファイルと差し替えが読むファイルが別 |
//! | `no_path_deletes_a_profile` | 消す口が生える |
//! | `a_file_with_context_tokens_is_listed_and_kept_when_another_profile_is_saved` | `context_tokens` を書いたファイルが一覧に出ない・別の面に出る・別のプロファイルの保存で消える（A-13a-2） |
//! | `context_tokens_are_written_and_emptied_byte_for_byte` | 書いた値がその面に入らない・空にしても残る・**書いていない面の保存結果が変わる**（A-13a-2） |
//! | `a_key_is_listed_as_set_with_its_last_four_and_never_by_value` | 鍵の値が一覧に載る・印や末尾4文字が別の面に出る（A-13b-1） |
//! | `a_saved_key_is_kept_cleared_or_replaced_as_the_request_says` | 送らなかった鍵が消える・`null` で消えない・置き換わらない・応答に鍵が載る（A-13b-1） |
//! | `keeping_a_key_never_borrows_another_profiles_or_faces_key` | 残す鍵を別の面・別の名前へ写す（A-13b-1） |
//! | `an_unusable_key_is_refused_without_echoing_it` | 使えない鍵・平文で外へ流れる鍵が書かれる・理由が返らない・応答に鍵が載る（A-13b-1） |

use std::path::{Path, PathBuf};
use std::time::Duration;

use personacore::server::{
    App, AppConfig, BindAddr, ENV_LLM_ENDPOINT, LaunchPlan, ServerError, Startup,
};
use reqwest::Method;
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// 待つ側の上限（回数）。**「働く側」ではなく「待つ側」に置く**（CLAUDE.md `G-4`）。
const MAX_WAITS: usize = 300;

/// 1回あたりの待ち時間。**上限は `MAX_WAITS` と合わせて 30 秒。**
const WAIT_STEP: Duration = Duration::from_millis(100);

/// **上流の宛先**（閉じている loopback のポート）。**LM Studio に繋ぎに行かせない。**
const CLOSED_ENDPOINT: &str = "http://127.0.0.1:9";

/// **2つのプロファイル × 2面。値は全部違える**（同値だと帰属の取り違えが観測できない）。
///
/// **`gguf` は `~` を含む綴り**（書いたままで返ることを見る）。
fn two_by_two() -> Value {
    json!({
        "north": {
            "main": { "endpoint": "http://127.0.0.1:47101", "model": "north-main",
                      "gguf": "~/models/north-main.gguf", "args": ["--ctx-size", "4096"],
                      "command": "C:/tools/north-llama.exe" },
            "sub":  { "endpoint": "http://127.0.0.1:47102", "model": "north-sub",
                      "gguf": "~/models/north-sub.gguf", "args": ["-ngl", "20"],
                      "command": "C:/tools/north-sub-llama.exe" }
        },
        "south": {
            "main": { "endpoint": "http://127.0.0.1:47103", "model": "south-main",
                      "gguf": "~/models/south-main.gguf", "args": ["--threads", "6"],
                      "command": "C:/tools/south-llama.exe" },
            "sub":  { "endpoint": "http://127.0.0.1:47104", "model": "south-sub",
                      "gguf": "~/models/south-sub.gguf", "args": ["-c", "2048"],
                      "command": "C:/tools/south-sub-llama.exe" }
        }
    })
}

/// **手で書いた形**（利用者のエディタが書いたもの。整形も並びも保存が書く形とは違う）。
fn write_by_hand(dir: &Path, source: &Value) -> PathBuf {
    let path = dir.join("upstreams.json");
    std::fs::write(&path, source.to_string()).expect("プロファイルを置けること");
    path
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("ポートを取れること");
    let port = listener.local_addr().expect("宛先を読めること").port();
    drop(listener);
    port
}

/// **走っているサーバ。**
struct Serving {
    port: u16,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), ServerError>>>,
}

impl Serving {
    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.await
                .expect("サーバのタスクが落ちていないこと")
                .expect("サーバが正常に止まること");
        }
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// **ポートを取られて待ち受けを開けなかったとき、別のポートで起こし直す回数の上限。**
const MAX_BINDS: usize = 5;

/// **本番の入口でサーバを起こし、待ち受けが答えるまで待つ。** 置き場所の渡し方は呼び出し側が決める。
///
/// **環境変数は1つも読まない**（測定中の機の `PERSONACORE_*` に左右させない）。
///
/// # ポートの取り合い（**機が飽和した回に実測 / 2026-09-24**）
///
/// [`free_port`] は「空いていた」を返すだけで、**起こすまでの間に別のテストが同じポートを取りうる。**
/// 取られると `App::run_with_upstreams` は待ち受けを開けずに返り、**そのポートには別の誰かが答える**
/// ——答えない相手なら、依頼は時間切れまで固まる。**だから待つ間、毎周サーバのタスクが生きているかを見て、
/// 終わっていたら別のポートで起こし直す**（書き手の生存確認 / CLAUDE.md §4）。
async fn serve(dir: &Path, startup: impl Fn(Startup) -> Startup) -> Serving {
    for _ in 0..MAX_BINDS {
        let port = free_port();
        let config = AppConfig::from_lookup_with_data_dir(
            |key: &str| (key == ENV_LLM_ENDPOINT).then(|| CLOSED_ENDPOINT.to_owned()),
            Some(dir.to_path_buf()),
        )
        .expect("設定を組めること")
        .with_bind(BindAddr::loopback(port))
        .with_database_path(dir.join("db").join("personacore.sqlite"));
        let plan = startup(Startup::new(LaunchPlan::none()));

        let (shutdown, wait) = oneshot::channel();
        let task = tokio::spawn(async move {
            App::run_with_upstreams(&config, plan, async move {
                let _ = wait.await;
            })
            .await
        });
        let mut serving = Serving {
            port,
            shutdown: Some(shutdown),
            task: Some(task),
        };
        if serving.answers().await {
            return serving;
        }
        // **タスクは終わっている**（待ち受けを開けなかった）。起こし直す。
        serving.task = None;
    }
    panic!("{MAX_BINDS} 回起こし直しても待ち受けを開けなかった");
}

impl Serving {
    /// **この待ち受けが一覧の口に答えるまで待つ。** サーバのタスクが終わったら `false`。
    ///
    /// **答えの形まで見る**——ポートを取った別の誰かの応答を、このサーバの応答と読まない。
    async fn answers(&self) -> bool {
        let url = format!("http://127.0.0.1:{}/admin/upstream/profiles", self.port);
        for _ in 0..MAX_WAITS {
            if self
                .task
                .as_ref()
                .is_none_or(tokio::task::JoinHandle::is_finished)
            {
                return false;
            }
            let answered = reqwest::Client::new()
                .get(&url)
                .timeout(Duration::from_secs(2))
                .send()
                .await;
            if let Ok(response) = answered
                && let Ok(body) = response.json::<Value>().await
                && body.get("editable").is_some()
            {
                return true;
            }
            tokio::time::sleep(WAIT_STEP).await;
        }
        panic!("待ち受けを {MAX_WAITS} 回待っても答えなかった");
    }
}

/// **1回の依頼の上限。**
///
/// # なぜ 30 秒ではないのか（**実測 / 2026-09-24**）
///
/// **このテストの実行ファイルを 12 本同時に走らせると、最初に起こした1本が、
/// 起動の数秒後にファイル操作の1つで 55〜60 秒止まる**（15 回のうち 13 回。うち 3 回は2本目も）。
/// **止まる操作は決まっていない** —— 計測の印を入れて測ると、保存の中の `create_dir_all` で止まる回と、
/// それを外した版で次の `std::fs::read` で止まる回があった（`<measurements>\f4b3a-2026-09-24\sync_ab\`）。
/// **止まっている間はそのテストのスレッドごと動かない**ので、時計も進まず、明けた瞬間に時間切れになる。
/// **原因は特定していない**（この機の EDR を疑っているが、確かめていない）。
/// **上限は待つ側に置いたまま、止まる長さより長くする。**
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// **1回の依頼**（待ち受けは [`serve`] が答えるのを確かめてから返している）。
async fn send(port: u16, method: Method, path: &str, body: Option<&Value>) -> (u16, Value) {
    let url = format!("http://127.0.0.1:{port}{path}");
    let mut request = reqwest::Client::new()
        .request(method, &url)
        .timeout(REQUEST_TIMEOUT);
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.expect("管理 API へ繋がること");
    let status = response.status().as_u16();
    let text = response.text().await.expect("本文を読めること");
    let json = serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("JSON として読めること: {err} / {text}"));
    (status, json)
}

async fn list(port: u16) -> Value {
    let (status, body) = send(port, Method::GET, "/admin/upstream/profiles", None).await;
    assert_eq!(status, 200, "{body}");
    body
}

async fn save(port: u16, name: &str, body: &Value) -> (u16, Value) {
    send(
        port,
        Method::PUT,
        &format!("/admin/upstream/profiles/{name}"),
        Some(body),
    )
    .await
}

/// ディレクトリの中身（名前の一覧。**一時ファイルが残っていないか**を見る）。
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("読めること")
        .map(|entry| {
            entry
                .expect("読めること")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

/// **応答の一覧から、その名前のプロファイルを取り出す。**
fn profile_named<'a>(listing: &'a Value, name: &str) -> &'a Value {
    listing["profiles"]
        .as_array()
        .expect("profiles は配列")
        .iter()
        .find(|profile| profile["name"] == name)
        .unwrap_or_else(|| panic!("{name} が一覧に無い: {listing}"))
}

/// **保存の依頼の面**（応答と同じ形。**省いた鍵は `null` で返る**）。
///
/// **`context_tokens` は書かない（`null`）**——書く腕は [`windowed`] で足す。
fn face(
    endpoint: &str,
    model: &str,
    gguf: Option<&str>,
    args: Option<&[&str]>,
    command: Option<&str>,
) -> Value {
    json!({
        "endpoint": endpoint,
        "model": model,
        "gguf": gguf,
        "args": args,
        "command": command,
        "context_tokens": null,
    })
}

/// **その面に `context_tokens` を書く**（作業単位 A-13a-2）。
fn windowed(mut face: Value, tokens: u64) -> Value {
    face["context_tokens"] = json!(tokens);
    face
}

/// **ファイルに書いた面（か保存の依頼の面）を、応答の形にする**（応答は書いていない鍵も `null` で必ず載せる）。
///
/// **鍵（`api_key`）は値を載せず、`api_key_set` / `api_key_hint` に直す**（作業単位 A-13b-1 /
/// ADR-0056 §決定3）。**期待値の末尾4文字は、ここで鍵の綴りから切る**（実装の `hint` を使わない）。
///
/// **承認（`allow_key_over_plain_http`）は書いていなければ `false` で載る**（作業単位 A-13b-3）。
fn as_listed(written: &Value) -> Value {
    let mut listed = json!({
        "gguf": null, "args": null, "command": null, "context_tokens": null,
        "api_key_set": false, "api_key_hint": null, "allow_key_over_plain_http": false,
    });
    for (key, value) in written.as_object().expect("面はオブジェクト") {
        if key == "api_key" {
            if let Some(raw) = value.as_str() {
                listed["api_key_set"] = json!(true);
                listed["api_key_hint"] = json!(&raw[raw.len() - 4..]);
            }
            continue;
        }
        listed[key] = value.clone();
    }
    listed
}

/// 本文の面に書く `context_tokens`。**既定（8192）とも落とし所（16384）とも、裏方の値とも違う。**
const MAIN_WINDOW: u64 = 28_672;

/// 裏方の面に書く `context_tokens`。**本文の値とも既定とも落とし所とも違う。**
const SUB_WINDOW: u64 = 10_240;

// ----------------------------------------------------------------------
// 本命
// ----------------------------------------------------------------------

/// **保存したものを読み直すと、依頼と同じ中身が返る**（Issue origin#24 の閉じる条件）。
///
/// **ファイルそのものも読み直す**（応答の組み立てだけが合っている形を通さない）。
/// **一時ファイルは残らない。** 新しい名前を足す腕と、在る名前を直す腕の両方。
#[tokio::test]
async fn a_saved_profile_reads_back_as_it_was_sent() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let profiles_dir = dir.path().join("app");
    std::fs::create_dir_all(&profiles_dir).expect("作れること");
    let path = write_by_hand(&profiles_dir, &two_by_two());
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;

    let before = list(server.port).await;
    assert_eq!(before["state"], "ok", "{before}");
    assert_eq!(before["editable"], true, "{before}");
    assert_eq!(before["path"], json!(path.display().to_string()));

    // **足す**（`sub` 無し）。
    let added_main = face(
        "http://127.0.0.1:47201",
        "east-main",
        Some("~/models/east.gguf"),
        Some(&["--ctx-size", "1024"]),
        None,
    );
    let (status, saved) = save(
        server.port,
        "east",
        &json!({ "revision": before["revision"], "main": added_main }),
    )
    .await;
    assert_eq!(status, 200, "{saved}");

    let after = list(server.port).await;
    assert_eq!(saved, after, "保存の応答が、読み直した一覧と違う");
    let east = profile_named(&after, "east");
    assert_eq!(east["main"], as_listed(&added_main), "{after}");
    assert_eq!(east["sub"], Value::Null, "{after}");
    assert_ne!(after["revision"], before["revision"], "印が変わっていない");

    // **ファイルそのもの**を、読み込みと同じ関数で読む。
    let on_disk = std::fs::read_to_string(&path).expect("読めること");
    let parsed = personacore::server::parse_profiles(&on_disk, None).expect("検証に通ること");
    let east_profile = parsed.select("east").expect("在ること");
    assert_eq!(east_profile.main().model(), "east-main");
    assert!(east_profile.sub().is_none());

    // **直す**（在る名前。`sub` を足す）。
    let south_sub = face("http://127.0.0.1:47202", "south-sub-2", None, None, None);
    let south_main = face(
        "http://127.0.0.1:47203",
        "south-main-2",
        Some("D:/models/south.gguf"),
        Some(&["-ngl", "99"]),
        Some("llama"),
    );
    let (status, saved) = save(
        server.port,
        "south",
        &json!({ "revision": after["revision"], "main": south_main, "sub": south_sub }),
    )
    .await;
    assert_eq!(status, 200, "{saved}");
    let again = list(server.port).await;
    assert_eq!(
        profile_named(&again, "south")["main"],
        as_listed(&south_main)
    );
    assert_eq!(profile_named(&again, "south")["sub"], as_listed(&south_sub));

    assert_eq!(
        entries(&profiles_dir),
        vec!["upstreams.json".to_owned()],
        "一時ファイルが残っている"
    );
    server.stop().await;
}

/// **検証に通らない入力は書かれない。ファイルのバイト列は1つも変わらず、理由が定数で返る。**
///
/// **読み込みと同じ関数（`parse_profiles`）が止める形を3つ並べる**（ループバックでない宛先に gguf /
/// `args` の `--host` / 空の `command`）。**名前が選べない形も並べる。**
#[tokio::test]
async fn an_edit_that_fails_the_loader_leaves_the_file_byte_for_byte() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &two_by_two());
    let original = std::fs::read(&path).expect("読めること");
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let revision = list(server.port).await["revision"].clone();

    let cases = [
        (
            json!({ "revision": revision,
                    "main": face("http://192.168.10.2:1", "m", Some("/x.gguf"), None, None) }),
            ("endpoint_not_loopback", json!("main"), json!("endpoint")),
        ),
        (
            json!({ "revision": revision,
                    "main": face("http://127.0.0.1:1", "m", None, None, None),
                    "sub": face("http://127.0.0.1:2", "s", Some("/y.gguf"), Some(&["--host", "0.0.0.0"]), None) }),
            ("reserved_serve_flag", json!("sub"), json!("args")),
        ),
        (
            json!({ "revision": revision,
                    "main": face("http://127.0.0.1:1", "m", Some("/x.gguf"), None, Some("   ")) }),
            ("command_empty", json!("main"), json!("command")),
        ),
        // **コンテキスト長が応答の余白（1024）以下**（作業単位 A-13a-2）。**止まる側を裏方に置く**
        // （本文には使える値）——面を取り違える実装を赤にする。鍵は欄に結びつく `context_tokens`。
        (
            json!({ "revision": revision,
                    "main": windowed(face("http://127.0.0.1:1", "m", None, None, None), MAIN_WINDOW),
                    "sub": windowed(face("http://127.0.0.1:2", "s", None, None, None), 1024) }),
            (
                "context_tokens_unusable",
                json!("sub"),
                json!("context_tokens"),
            ),
        ),
    ];
    for (body, (reason, face_label, key)) in cases {
        let (status, response) = save(server.port, "south", &body).await;
        assert_eq!(status, 422, "{response}");
        assert_eq!(
            response["error"]["code"], "upstream_profile_invalid",
            "{response}"
        );
        assert_eq!(response["error"]["reason"], reason, "{response}");
        assert_eq!(response["error"]["face"], face_label, "{response}");
        assert_eq!(response["error"]["key"], key, "{response}");
        assert!(
            response["error"]["reason_message"]
                .as_str()
                .is_some_and(|m| !m.is_empty()),
            "{response}"
        );
        // **利用者の綴り（宛先・名前）を応答に反射しない。**
        let text = response.to_string();
        for echoed in ["192.168.10.2", "0.0.0.0", "south"] {
            assert!(!text.contains(echoed), "{echoed} を反射している: {text}");
        }
        assert_eq!(
            std::fs::read(&path).expect("読めること"),
            original,
            "検証に通らないのに書いている（{reason}）"
        );
    }

    // **`--upstream` で選べない名前**（前後の空白・`--` で始まる）。
    for name in ["%20padded", "--fresh"] {
        let (status, response) = save(
            server.port,
            name,
            &json!({ "revision": revision, "main": face("http://127.0.0.1:1", "m", None, None, None) }),
        )
        .await;
        assert_eq!(status, 400, "{name}: {response}");
        assert_eq!(response["error"]["code"], "upstream_profile_name_invalid");
        assert_eq!(
            std::fs::read(&path).expect("読めること"),
            original,
            "{name}"
        );
    }

    // **知らない欄**（`deny_unknown_fields`）。
    let (status, response) = save(
        server.port,
        "south",
        &json!({ "revision": revision,
                 "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "ggml": "/x.gguf" } }),
    )
    .await;
    assert_eq!(status, 400, "{response}");
    assert_eq!(std::fs::read(&path).expect("読めること"), original);

    // **整数でない・負の `context_tokens`** は本文として読めない（400。画面は送る前に止める）。
    for raw in [json!(-1), json!(1.5), json!("32768")] {
        let (status, response) = save(
            server.port,
            "south",
            &json!({ "revision": revision,
                     "main": { "endpoint": "http://127.0.0.1:1", "model": "m", "context_tokens": raw } }),
        )
        .await;
        assert_eq!(status, 400, "{raw}: {response}");
        assert_eq!(std::fs::read(&path).expect("読めること"), original, "{raw}");
    }

    assert_eq!(
        entries(dir.path())
            .iter()
            .filter(|n| n.ends_with(".tmp"))
            .count(),
        0
    );
    server.stop().await;
}

/// **印が違えば 409。ファイルは変わらない**（ADR-0042 §決定8「手で書き換えられていたら上書きしない」）。
///
/// **読んだ後に手で書き換えた**形と、**無いと思って送ったが在る**形（`revision: null`）の2つ。
/// **対照**: 書き換えた後に読み直した印なら通る（印そのものを見ていることの確認）。
#[tokio::test]
async fn a_stale_revision_is_refused_and_the_file_is_untouched() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &two_by_two());
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let stale = list(server.port).await["revision"].clone();

    // **手で書き換える**（値を1つだけ変える）。
    let mut edited = two_by_two();
    edited["north"]["main"]["model"] = json!("north-main-by-hand");
    std::fs::write(&path, edited.to_string()).expect("書けること");
    let by_hand = std::fs::read(&path).expect("読めること");

    let body = |revision: &Value| json!({ "revision": revision, "main": face("http://127.0.0.1:1", "m", None, None, None) });
    for revision in [stale.clone(), Value::Null] {
        let (status, response) = save(server.port, "south", &body(&revision)).await;
        assert_eq!(status, 409, "{revision}: {response}");
        assert_eq!(response["error"]["code"], "upstream_profiles_changed");
        assert_eq!(
            std::fs::read(&path).expect("読めること"),
            by_hand,
            "手で書き換えた内容を上書きしている"
        );
    }

    // **対照**: 読み直した印なら通り、手で書き換えた値は残る。
    let fresh = list(server.port).await;
    assert_ne!(fresh["revision"], stale, "前提: 印が変わっていること");
    let (status, response) = save(server.port, "south", &body(&fresh["revision"])).await;
    assert_eq!(status, 200, "{response}");
    assert_eq!(
        profile_named(&response, "north")["main"]["model"],
        "north-main-by-hand"
    );
    server.stop().await;
}

/// **読むだけの置き場所には書かない**（`with_profiles_file` ＝ `PERSONACORE_UPSTREAMS` の側）。
///
/// **置き場所が無い起動も同じく断る。** どちらも一覧は読める（読むだけの側は中身も返す）。
#[tokio::test]
async fn a_read_only_location_refuses_to_write() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &two_by_two());
    let original = std::fs::read(&path).expect("読めること");

    let server = serve(dir.path(), |s| s.with_profiles_file(path.clone())).await;
    let listing = list(server.port).await;
    assert_eq!(listing["editable"], false, "{listing}");
    assert_eq!(listing["state"], "ok", "{listing}");
    assert_eq!(listing["path"], json!(path.display().to_string()));
    let (status, response) = save(
        server.port,
        "north",
        &json!({ "revision": listing["revision"],
                 "main": face("http://127.0.0.1:1", "m", None, None, None) }),
    )
    .await;
    assert_eq!(status, 409, "{response}");
    assert_eq!(response["error"]["code"], "upstream_profiles_read_only");
    assert_eq!(std::fs::read(&path).expect("読めること"), original);
    assert_eq!(
        entries(dir.path())
            .iter()
            .filter(|n| n.ends_with(".tmp"))
            .count(),
        0
    );
    server.stop().await;

    let nowhere = tempfile::tempdir().expect("一時ディレクトリ");
    let server = serve(nowhere.path(), |s| s).await;
    let listing = list(server.port).await;
    assert_eq!(
        (&listing["state"], &listing["editable"], &listing["path"]),
        (&json!("none"), &json!(false), &Value::Null),
        "{listing}"
    );
    let (status, response) = save(
        server.port,
        "north",
        &json!({ "revision": null, "main": face("http://127.0.0.1:1", "m", None, None, None) }),
    )
    .await;
    assert_eq!(status, 409, "{response}");
    assert_eq!(response["error"]["code"], "upstream_profiles_read_only");
    server.stop().await;
}

/// **一覧の gguf は書いたままの綴り**（`~` なら `~` のまま / Issue origin#24「書いた値と観測した値を混ぜない」）。
///
/// **全部の欄を配列で突き合わせる**——`~` を展開する変異も、面を取り違える変異もここで赤になる。
#[tokio::test]
async fn the_listing_returns_the_gguf_as_written() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &two_by_two());
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;

    let listing = list(server.port).await;
    let written = two_by_two();
    let expected: Vec<Value> = ["north", "south"]
        .into_iter()
        .map(|name| {
            json!({ "name": name,
                    "main": as_listed(&written[name]["main"]),
                    "sub": as_listed(&written[name]["sub"]) })
        })
        .collect();
    assert_eq!(listing["profiles"], json!(expected), "{listing}");
    assert_eq!(
        profile_named(&listing, "north")["main"]["gguf"],
        "~/models/north-main.gguf"
    );
    server.stop().await;
}

/// **帰属: 2つのプロファイル × 2面の値をすべて違え、1つの面を直しても、他は1つも変わらない。**
///
/// **`north` の `sub` だけを直す**——`north` の `main` と、`south` の両面が残ること。
#[tokio::test]
async fn editing_one_profile_leaves_the_other_profile_and_face_alone() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &two_by_two());
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let before = list(server.port).await;
    let written = two_by_two();

    let new_sub = face(
        "http://127.0.0.1:47301",
        "north-sub-2",
        Some("~/models/north-sub-2.gguf"),
        Some(&["-ngl", "33"]),
        Some("C:/tools/north-sub-llama-2.exe"),
    );
    let (status, saved) = save(
        server.port,
        "north",
        &json!({ "revision": before["revision"], "main": written["north"]["main"], "sub": new_sub }),
    )
    .await;
    assert_eq!(status, 200, "{saved}");

    // **応答（一覧）でも、ファイルの値でも。**
    let on_disk: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("読めること")).expect("JSON");
    assert_eq!(on_disk["south"], written["south"], "south が変わっている");
    assert_eq!(
        on_disk["north"]["main"], written["north"]["main"],
        "north の main が変わっている"
    );
    assert_eq!(
        on_disk["north"]["sub"],
        json!({
            "endpoint": "http://127.0.0.1:47301", "model": "north-sub-2",
            "gguf": "~/models/north-sub-2.gguf", "args": ["-ngl", "33"],
            "command": "C:/tools/north-sub-llama-2.exe"
        })
    );
    assert_eq!(
        profile_named(&saved, "south"),
        profile_named(&before, "south")
    );
    assert_eq!(
        profile_named(&saved, "north")["main"],
        profile_named(&before, "north")["main"]
    );
    assert_eq!(profile_named(&saved, "north")["sub"], as_listed(&new_sub));
    server.stop().await;
}

/// **無いファイルは `missing`（印 `null`）。`revision: null` で送れば作る**（親のディレクトリも）。
#[tokio::test]
async fn a_missing_file_is_created_by_the_first_save() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = dir.path().join("not-yet").join("upstreams.json");
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;

    let listing = list(server.port).await;
    assert_eq!(
        (
            &listing["state"],
            &listing["revision"],
            &listing["editable"]
        ),
        (&json!("missing"), &Value::Null, &json!(true)),
        "{listing}"
    );
    let main = face("http://127.0.0.1:47401", "first", None, None, None);
    let (status, saved) = save(
        server.port,
        "first",
        &json!({ "revision": null, "main": main }),
    )
    .await;
    assert_eq!(status, 200, "{saved}");
    assert_eq!(profile_named(&saved, "first")["main"], as_listed(&main));
    assert!(path.is_file(), "ファイルが出来ていない");
    server.stop().await;
}

/// **`context_tokens` を書いたファイルも一覧に出て、別のプロファイルを足しても値は残る**
/// （ADR-0054 §決定5 / 作業単位 A-13a-2。A-13a-1 の間は `not_yet_editable` で一覧ごと扱えなかった）。
///
/// **値を書いたのは `north` の本文と `south` の裏方**（プロファイルも面も違える）。
/// **一覧は全部の欄を配列で突き合わせる**——値を別の面・別のプロファイルに出す変異も、
/// 落とす変異もここで赤になる。
#[tokio::test]
async fn a_file_with_context_tokens_is_listed_and_kept_when_another_profile_is_saved() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let mut written = two_by_two();
    written["north"]["main"]["context_tokens"] = json!(MAIN_WINDOW);
    written["south"]["sub"]["context_tokens"] = json!(SUB_WINDOW);
    let path = write_by_hand(dir.path(), &written);
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;

    let listing = list(server.port).await;
    assert_eq!(
        (&listing["state"], &listing["problem"]),
        (&json!("ok"), &Value::Null),
        "{listing}"
    );
    let expected: Vec<Value> = ["north", "south"]
        .into_iter()
        .map(|name| {
            json!({ "name": name,
                    "main": as_listed(&written[name]["main"]),
                    "sub": as_listed(&written[name]["sub"]) })
        })
        .collect();
    assert_eq!(listing["profiles"], json!(expected), "{listing}");
    assert_eq!(
        [
            &profile_named(&listing, "north")["main"]["context_tokens"],
            &profile_named(&listing, "north")["sub"]["context_tokens"],
            &profile_named(&listing, "south")["main"]["context_tokens"],
            &profile_named(&listing, "south")["sub"]["context_tokens"],
        ],
        [
            &json!(MAIN_WINDOW),
            &Value::Null,
            &Value::Null,
            &json!(SUB_WINDOW)
        ],
        "前提: 値は書いた面にだけ出ること"
    );

    // **別のプロファイルを足しても、書いてあった値はファイルに残る**（組み直しで落とさない）。
    let (status, saved) = save(
        server.port,
        "east",
        &json!({ "revision": listing["revision"],
                 "main": face("http://127.0.0.1:47501", "east-main", None, None, None) }),
    )
    .await;
    assert_eq!(status, 200, "{saved}");
    let on_disk: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("読めること")).expect("JSON");
    assert_eq!(on_disk["north"], written["north"], "north が変わっている");
    assert_eq!(on_disk["south"], written["south"], "south が変わっている");
    server.stop().await;
}

/// **画面の API で `context_tokens` を書き・空にできる。ファイルのバイト列で見る**（作業単位 A-13a-2）。
///
/// 1つのファイルに3回保存する:
///
/// | 回 | 送る | 見ること |
/// |---|---|---|
/// | 1 | どちらの面にも書かない | **A-13a-2 の前と同じバイト列**（対照 / 直書き） |
/// | 2 | 本文に [`MAIN_WINDOW`]・裏方に [`SUB_WINDOW`] | その面の鍵として入ったバイト列。一覧にもその面で出る |
/// | 3 | 本文を空にする（裏方は残す） | 本文の鍵だけが消えたバイト列 |
///
/// **期待値は直書き**（保存が書く形 —— 2 空白の字下げ・鍵は名前の順・末尾に改行）。
#[tokio::test]
async fn context_tokens_are_written_and_emptied_byte_for_byte() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = dir.path().join("upstreams.json");
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;

    let main = face(
        "http://127.0.0.1:47601",
        "only-main",
        Some("~/models/only.gguf"),
        Some(&["-ngl", "99"]),
        None,
    );
    let sub = face("http://127.0.0.1:47602", "only-sub", None, None, None);

    // 1. **書かない**（対照）。
    let (status, first) = save(
        server.port,
        "only",
        &json!({ "revision": null, "main": main, "sub": sub }),
    )
    .await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("読めること"),
        concat!(
            "{\n",
            "  \"only\": {\n",
            "    \"main\": {\n",
            "      \"args\": [\n",
            "        \"-ngl\",\n",
            "        \"99\"\n",
            "      ],\n",
            "      \"endpoint\": \"http://127.0.0.1:47601\",\n",
            "      \"gguf\": \"~/models/only.gguf\",\n",
            "      \"model\": \"only-main\"\n",
            "    },\n",
            "    \"sub\": {\n",
            "      \"endpoint\": \"http://127.0.0.1:47602\",\n",
            "      \"model\": \"only-sub\"\n",
            "    }\n",
            "  }\n",
            "}\n",
        ),
        "書いていない面の保存結果が変わった"
    );

    // 2. **書く**（面ごとに違う値）。
    let (status, second) = save(
        server.port,
        "only",
        &json!({ "revision": first["revision"],
                 "main": windowed(main.clone(), MAIN_WINDOW),
                 "sub": windowed(sub.clone(), SUB_WINDOW) }),
    )
    .await;
    assert_eq!(status, 200, "{second}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("読めること"),
        concat!(
            "{\n",
            "  \"only\": {\n",
            "    \"main\": {\n",
            "      \"args\": [\n",
            "        \"-ngl\",\n",
            "        \"99\"\n",
            "      ],\n",
            "      \"context_tokens\": 28672,\n",
            "      \"endpoint\": \"http://127.0.0.1:47601\",\n",
            "      \"gguf\": \"~/models/only.gguf\",\n",
            "      \"model\": \"only-main\"\n",
            "    },\n",
            "    \"sub\": {\n",
            "      \"context_tokens\": 10240,\n",
            "      \"endpoint\": \"http://127.0.0.1:47602\",\n",
            "      \"model\": \"only-sub\"\n",
            "    }\n",
            "  }\n",
            "}\n",
        )
    );
    let listed = list(server.port).await;
    assert_eq!(second, listed, "保存の応答が、読み直した一覧と違う");
    assert_eq!(
        (
            &profile_named(&listed, "only")["main"],
            &profile_named(&listed, "only")["sub"]
        ),
        (
            &as_listed(&windowed(main.clone(), MAIN_WINDOW)),
            &as_listed(&windowed(sub.clone(), SUB_WINDOW))
        )
    );
    // **書いたファイルは読み込みの検証を通り、値が面ごとの明示になる**（ADR-0054 §決定1）。
    let on_disk = std::fs::read_to_string(&path).expect("読めること");
    let settings = personacore::server::parse_profiles(&on_disk, None)
        .expect("検証に通ること")
        .select("only")
        .expect("在ること")
        .settings();
    assert_eq!(
        (settings.context_tokens(), settings.sub_context_tokens()),
        (Some(28_672), Some(10_240))
    );

    // 3. **本文を空にする**（`null` で送る）。裏方は残す。
    let (status, third) = save(
        server.port,
        "only",
        &json!({ "revision": second["revision"],
                 "main": main,
                 "sub": windowed(sub.clone(), SUB_WINDOW) }),
    )
    .await;
    assert_eq!(status, 200, "{third}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("読めること"),
        concat!(
            "{\n",
            "  \"only\": {\n",
            "    \"main\": {\n",
            "      \"args\": [\n",
            "        \"-ngl\",\n",
            "        \"99\"\n",
            "      ],\n",
            "      \"endpoint\": \"http://127.0.0.1:47601\",\n",
            "      \"gguf\": \"~/models/only.gguf\",\n",
            "      \"model\": \"only-main\"\n",
            "    },\n",
            "    \"sub\": {\n",
            "      \"context_tokens\": 10240,\n",
            "      \"endpoint\": \"http://127.0.0.1:47602\",\n",
            "      \"model\": \"only-sub\"\n",
            "    }\n",
            "  }\n",
            "}\n",
        ),
        "空にした本文の値が残っている（または裏方まで消えた）"
    );
    assert_eq!(
        profile_named(&third, "only")["main"]["context_tokens"],
        Value::Null
    );
    server.stop().await;
}

/// 本文の面に書く鍵（囮。`north` の本文）。
const NORTH_MAIN_KEY: &str = "pc-north-main-key-NM01";
/// 裏方の面に書く鍵（囮。`south` の裏方）。**本文の鍵と別の綴り・別の末尾4文字。**
const SOUTH_SUB_KEY: &str = "pc-south-sub-key-SS02";
/// 置き換える鍵（囮）。**上の2つと別の綴り・別の末尾4文字。**
const REPLACED_KEY: &str = "pc-replaced-key-RP03";

/// **応答の本文に、囮の鍵が1つも載っていないこと**（ADR-0056 §決定3）。
fn assert_no_key_in(body: &Value) {
    let text = body.to_string();
    for key in [NORTH_MAIN_KEY, SOUTH_SUB_KEY, REPLACED_KEY] {
        assert!(!text.contains(key), "応答に鍵が載っている（{key}）: {text}");
    }
}

/// **鍵は一覧に値で載らない。書いた面にだけ「設定済み」と末尾4文字が出る**
/// （ADR-0056 §決定3 / 作業単位 A-13b-1）。
///
/// **鍵を書いたのは `north` の本文と `south` の裏方**（プロファイルも面も違える）。
/// 一覧は全部の欄を配列で突き合わせる —— 印や末尾を別の面に出す変異も、ここで赤になる。
#[tokio::test]
async fn a_key_is_listed_as_set_with_its_last_four_and_never_by_value() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let mut written = two_by_two();
    written["north"]["main"]["api_key"] = json!(NORTH_MAIN_KEY);
    written["south"]["sub"]["api_key"] = json!(SOUTH_SUB_KEY);
    let path = write_by_hand(dir.path(), &written);
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;

    let listing = list(server.port).await;
    assert_eq!(listing["state"], "ok", "{listing}");
    assert_no_key_in(&listing);
    assert_eq!(
        [
            (
                &profile_named(&listing, "north")["main"]["api_key_set"],
                &profile_named(&listing, "north")["main"]["api_key_hint"]
            ),
            (
                &profile_named(&listing, "north")["sub"]["api_key_set"],
                &profile_named(&listing, "north")["sub"]["api_key_hint"]
            ),
            (
                &profile_named(&listing, "south")["main"]["api_key_set"],
                &profile_named(&listing, "south")["main"]["api_key_hint"]
            ),
            (
                &profile_named(&listing, "south")["sub"]["api_key_set"],
                &profile_named(&listing, "south")["sub"]["api_key_hint"]
            ),
        ],
        [
            (&json!(true), &json!("NM01")),
            (&json!(false), &Value::Null),
            (&json!(false), &Value::Null),
            (&json!(true), &json!("SS02")),
        ],
        "印と末尾4文字は書いた面にだけ出ること"
    );
    // **応答に `api_key` の鍵そのものが無い**（値を `null` にして残す形も通さない）。
    for name in ["north", "south"] {
        for face in ["main", "sub"] {
            assert!(
                profile_named(&listing, name)[face].get("api_key").is_none(),
                "{name} の {face} に api_key の欄がある: {listing}"
            );
        }
    }
    server.stop().await;
}

/// **編集の依頼の `api_key` は「書いていない＝残す / `null`＝消す / 文字列＝置き換える」**
/// （ADR-0056 §決定3 / 作業単位 A-13b-1）。**ファイルのバイト列と一覧の両方で見る。**
///
/// | 回 | 送る | 見ること |
/// |---|---|---|
/// | 1 | **ほかの欄だけを直す**（`model`）。`api_key` は送らない | 両方の面の鍵が残る（画面の保存で鍵が消えない） |
/// | 2 | 本文に `null`・裏方は送らない | 本文の鍵だけが消え、裏方は残る |
/// | 3 | 本文に新しい鍵・裏方は送らない | 本文が新しい鍵になり、裏方は残る |
///
/// **3つとも応答に鍵が載らない**（書いた後の一覧も）。**`south` には触らない**（帰属）。
#[tokio::test]
async fn a_saved_key_is_kept_cleared_or_replaced_as_the_request_says() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let mut written = two_by_two();
    written["north"]["main"]["api_key"] = json!(NORTH_MAIN_KEY);
    written["north"]["sub"]["api_key"] = json!(SOUTH_SUB_KEY);
    let path = write_by_hand(dir.path(), &written);
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let on_disk = || -> Value {
        serde_json::from_str(&std::fs::read_to_string(&path).expect("読めること")).expect("JSON")
    };
    // **送る面は、ファイルの面から `api_key` を落とした形**（画面は鍵の値を持たない）。
    let without_key = |face: &Value| {
        let mut face = face.clone();
        face.as_object_mut().expect("面").remove("api_key");
        face
    };
    let listing = list(server.port).await;

    // 1. **ほかの欄だけを直す。鍵は送らない。**
    let mut main = without_key(&written["north"]["main"]);
    main["model"] = json!("north-main-renamed");
    let sub = without_key(&written["north"]["sub"]);
    let (status, first) = save(
        server.port,
        "north",
        &json!({ "revision": listing["revision"], "main": main, "sub": sub }),
    )
    .await;
    assert_eq!(status, 200, "{first}");
    assert_no_key_in(&first);
    let disk = on_disk();
    assert_eq!(
        (
            &disk["north"]["main"]["api_key"],
            &disk["north"]["sub"]["api_key"],
            &disk["north"]["main"]["model"]
        ),
        (
            &json!(NORTH_MAIN_KEY),
            &json!(SOUTH_SUB_KEY),
            &json!("north-main-renamed")
        ),
        "送らなかった鍵が消えた（か、直した欄が入っていない）"
    );
    assert_eq!(disk["south"], written["south"], "south が変わっている");

    // 2. **本文の鍵を消す**（`null`）。裏方は送らない。
    let mut cleared = main.clone();
    cleared["api_key"] = Value::Null;
    let (status, second) = save(
        server.port,
        "north",
        &json!({ "revision": first["revision"], "main": cleared, "sub": sub }),
    )
    .await;
    assert_eq!(status, 200, "{second}");
    assert_no_key_in(&second);
    let disk = on_disk();
    assert!(
        disk["north"]["main"].get("api_key").is_none(),
        "null で送った鍵が残っている（鍵ごと消えること）: {disk}"
    );
    assert_eq!(
        disk["north"]["sub"]["api_key"],
        json!(SOUTH_SUB_KEY),
        "本文の鍵を消したら裏方の鍵まで消えた"
    );
    assert_eq!(
        (
            &profile_named(&second, "north")["main"]["api_key_set"],
            &profile_named(&second, "north")["sub"]["api_key_set"]
        ),
        (&json!(false), &json!(true))
    );

    // 3. **本文に新しい鍵を書く。** 裏方は送らない。
    let mut replaced = main.clone();
    replaced["api_key"] = json!(REPLACED_KEY);
    let (status, third) = save(
        server.port,
        "north",
        &json!({ "revision": second["revision"], "main": replaced, "sub": sub }),
    )
    .await;
    assert_eq!(status, 200, "{third}");
    assert_no_key_in(&third);
    let disk = on_disk();
    assert_eq!(
        (
            &disk["north"]["main"]["api_key"],
            &disk["north"]["sub"]["api_key"]
        ),
        (&json!(REPLACED_KEY), &json!(SOUTH_SUB_KEY))
    );
    assert_eq!(
        (
            &profile_named(&third, "north")["main"]["api_key_hint"],
            &profile_named(&third, "north")["sub"]["api_key_hint"]
        ),
        (&json!("RP03"), &json!("SS02"))
    );
    // **書いた後の一覧にも鍵は載らない。**
    assert_no_key_in(&list(server.port).await);
    assert_eq!(on_disk()["south"], written["south"], "south が変わっている");
    server.stop().await;
}

/// **手で書いた承認（`allow_key_over_plain_http`）は、画面から保存しても、宛先（`endpoint`）が変わらない面でだけ残る。
/// 残すのはその名前・その面のものだけ**（作業単位 A-13b-2 とそのレビュー是正 / ADR-0056 §決定5）。
///
/// 依頼で承認の欄を送らなければ、この規則で決まる（欄を送る形は
/// [`an_approval_in_the_request_is_written_removed_or_left_to_the_rule`]）。残さないと、別の欄を直しただけで
/// 承認が消え、次の送信から `upstream_key_not_approved` になる。
///
/// > A-13b-2 の間は、一覧の面に承認の欄が無く、ここは「載せない」を見ていた。作業単位 A-13b-3 で画面が承認を
/// > 出すようになったので、**一覧は書いた値を面ごとに載せる**（帰属を見るため、両面とも見る）。
///
/// **宛先を変えた面では落とす**（レビュー是正）。承認は「この宛先へ鍵を平文で送ってよい」という判断で、
/// 持ち越すと、利用者が承認していない新しい外の宛先へ鍵が平文で出る（ADR-0056 §決定5 は危険を示したうえで取る）。
///
/// | 面 | 保存で変えたもの | 承認 |
/// |---|---|---|
/// | `lan` の本文 | `model` だけ | **残る** |
/// | `lan` の裏方 | `endpoint` | **落ちる** |
/// | `fresh`（新しい名前） | —— | 書かれない（他の名前から拾わない） |
#[tokio::test]
async fn a_hand_written_approval_survives_a_save_from_the_screen() {
    const APPROVAL: &str = "allow_key_over_plain_http";
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let source = json!({
        "lan": {
            "main": { "endpoint": "http://192.0.2.31:47901", "model": "lan-main",
                      "api_key": NORTH_MAIN_KEY, APPROVAL: true },
            "sub": { "endpoint": "http://192.0.2.32:47902", "model": "lan-sub",
                     "api_key": SOUTH_SUB_KEY, APPROVAL: true }
        }
    });
    let path = write_by_hand(dir.path(), &source);
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let listing = list(server.port).await;
    assert_eq!(
        (
            &profile_named(&listing, "lan")["main"][APPROVAL],
            &profile_named(&listing, "lan")["sub"][APPROVAL],
        ),
        (&json!(true), &json!(true)),
        "一覧の面に、書いた承認が載っていない: {listing}"
    );

    // **本文はモデルだけ、裏方は宛先を直す**（承認も鍵も送らない）。新しい名前も足す。
    let main = face("http://192.0.2.31:47901", "lan-main-2", None, None, None);
    let sub = face("http://203.0.113.32:47902", "lan-sub", None, None, None);
    let (status, first) = save(
        server.port,
        "lan",
        &json!({ "revision": listing["revision"], "main": main, "sub": sub }),
    )
    .await;
    assert_eq!(status, 200, "{first}");
    let (status, second) = save(
        server.port,
        "fresh",
        &json!({ "revision": first["revision"], "main": main }),
    )
    .await;
    assert_eq!(status, 200, "{second}");

    let disk: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("読めること")).expect("JSON");
    assert_eq!(
        (
            &disk["lan"]["main"]["model"],
            &disk["lan"]["sub"]["endpoint"],
        ),
        (&json!("lan-main-2"), &json!("http://203.0.113.32:47902")),
        "前提: 依頼どおりに保存されたこと: {disk}"
    );
    assert_eq!(
        (
            &disk["lan"]["main"][APPROVAL],
            disk["lan"]["sub"].get(APPROVAL),
            disk["fresh"]["main"].get(APPROVAL),
        ),
        (&json!(true), None, None),
        "宛先の同じ面の承認が消えた／宛先を変えた面へ持ち越した／別の名前へ写った: {disk}"
    );
    // **鍵は宛先を変えても残る**（鍵の扱いは A-13b-1 のまま。落とすのは承認だけ）。
    assert_eq!(
        disk["lan"]["sub"]["api_key"],
        json!(SOUTH_SUB_KEY),
        "{disk}"
    );
    server.stop().await;
}

/// **依頼の承認（`allow_key_over_plain_http`）は「送らない＝今の規則 / `true`＝承認する / `false`＝外す」**
/// （作業単位 A-13b-3 / ADR-0056 §決定5）。**一覧はその面に書いた承認を返す。**
///
/// | 回 | 面 | 送る | ファイル |
/// |---|---|---|---|
/// | 1 | `far` の本文（承認なし） | `true`・宛先は同じ | **`true` が書かれる** |
/// | 1 | `far` の裏方（承認あり） | `false`・宛先は同じ | **鍵ごと消える**（`false` を書き残さない） |
/// | 2 | `far` の本文 | `true`・**宛先を変える** | **`true` のまま**（明示の承認は新しい宛先についてのもの） |
/// | 2 | `far` の裏方 | 送らない・宛先は同じ | 無いまま（今の規則は、無いものを生やさない） |
/// | 3 | `near` の本文（承認あり） | **`null`**・宛先は同じ | **`true` のまま**（`null` は送らないのと同じ） |
///
/// **`near` は1・2の回で動かない**（帰属）。値の組を面ごとに違える（本文と裏方で承認の有無を逆にする）。
#[tokio::test]
async fn an_approval_in_the_request_is_written_removed_or_left_to_the_rule() {
    const APPROVAL: &str = "allow_key_over_plain_http";
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let source = json!({
        "far": {
            "main": { "endpoint": "http://203.0.113.41:47911", "model": "far-main",
                      "api_key": NORTH_MAIN_KEY },
            "sub": { "endpoint": "http://203.0.113.42:47912", "model": "far-sub",
                     "api_key": SOUTH_SUB_KEY, APPROVAL: true }
        },
        "near": {
            "main": { "endpoint": "http://203.0.113.43:47913", "model": "near-main",
                      "api_key": REPLACED_KEY, APPROVAL: true }
        }
    });
    let path = write_by_hand(dir.path(), &source);
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let on_disk = || -> Value {
        serde_json::from_str(&std::fs::read_to_string(&path).expect("読めること")).expect("JSON")
    };
    let listing = list(server.port).await;
    assert_eq!(
        (
            &profile_named(&listing, "far")["main"][APPROVAL],
            &profile_named(&listing, "far")["sub"][APPROVAL],
            &profile_named(&listing, "near")["main"][APPROVAL],
        ),
        (&json!(false), &json!(true), &json!(true)),
        "一覧が面ごとの承認を返していない: {listing}"
    );

    // 1. **宛先は同じまま、本文は承認する・裏方は外す。**
    let mut main = face("http://203.0.113.41:47911", "far-main", None, None, None);
    main[APPROVAL] = json!(true);
    let mut sub = face("http://203.0.113.42:47912", "far-sub", None, None, None);
    sub[APPROVAL] = json!(false);
    let (status, first) = save(
        server.port,
        "far",
        &json!({ "revision": listing["revision"], "main": main, "sub": sub }),
    )
    .await;
    assert_eq!(status, 200, "{first}");
    assert_no_key_in(&first);
    let disk = on_disk();
    assert_eq!(
        (
            &disk["far"]["main"][APPROVAL],
            disk["far"]["sub"].get(APPROVAL),
        ),
        (&json!(true), None),
        "true が書かれていない／false を鍵ごと消していない: {disk}"
    );
    assert_eq!(
        (
            &profile_named(&first, "far")["main"][APPROVAL],
            &profile_named(&first, "far")["sub"][APPROVAL],
        ),
        (&json!(true), &json!(false)),
        "保存の応答が、書いた承認を返していない: {first}"
    );
    // **鍵は送っていないので、両方とも残る。**
    assert_eq!(
        (
            &disk["far"]["main"]["api_key"],
            &disk["far"]["sub"]["api_key"]
        ),
        (&json!(NORTH_MAIN_KEY), &json!(SOUTH_SUB_KEY)),
        "{disk}"
    );
    assert_eq!(disk["near"], source["near"], "near が動いた");

    // 2. **本文の宛先を変えて、承認を送る。** 裏方は送らない。
    let mut moved = face("http://203.0.113.44:47914", "far-main", None, None, None);
    moved[APPROVAL] = json!(true);
    let plain_sub = face("http://203.0.113.42:47912", "far-sub", None, None, None);
    let (status, second) = save(
        server.port,
        "far",
        &json!({ "revision": first["revision"], "main": moved, "sub": plain_sub }),
    )
    .await;
    assert_eq!(status, 200, "{second}");
    let disk = on_disk();
    assert_eq!(
        (
            &disk["far"]["main"]["endpoint"],
            &disk["far"]["main"][APPROVAL],
            disk["far"]["sub"].get(APPROVAL),
        ),
        (&json!("http://203.0.113.44:47914"), &json!(true), None),
        "宛先を変えた面の明示の承認が落ちた／送らなかった面に承認が生えた: {disk}"
    );
    assert_eq!(disk["near"], source["near"], "near が動いた");

    // 3. **`null` は送らないのと同じ**（宛先が同じなので残る）。
    let mut nulled = face("http://203.0.113.43:47913", "near-main", None, None, None);
    nulled[APPROVAL] = Value::Null;
    let (status, third) = save(
        server.port,
        "near",
        &json!({ "revision": second["revision"], "main": nulled }),
    )
    .await;
    assert_eq!(status, 200, "{third}");
    assert_eq!(
        on_disk()["near"]["main"][APPROVAL],
        json!(true),
        "null で承認が外れた（null は送らないのと同じ）"
    );
    server.stop().await;
}

/// **残す鍵は、その名前・その面のものだけ**（帰属 / 作業単位 A-13b-1）。
///
/// * **新しい名前**を足すとき、`api_key` を送らなければ鍵は書かれない（他のプロファイルの鍵を拾わない）。
/// * **`sub` を新しく足す**とき、送らなければ裏方に鍵は書かれない（本文の鍵を写さない）。
#[tokio::test]
async fn keeping_a_key_never_borrows_another_profiles_or_faces_key() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let source = json!({
        "keyed": { "main": { "endpoint": "http://127.0.0.1:47701", "model": "keyed-main",
                             "api_key": NORTH_MAIN_KEY } }
    });
    let path = write_by_hand(dir.path(), &source);
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let listing = list(server.port).await;

    // **`keyed` に裏方を足す**（鍵は送らない）。
    let main = face("http://127.0.0.1:47701", "keyed-main", None, None, None);
    let sub = face("http://127.0.0.1:47702", "keyed-sub", None, None, None);
    let (status, first) = save(
        server.port,
        "keyed",
        &json!({ "revision": listing["revision"], "main": main, "sub": sub }),
    )
    .await;
    assert_eq!(status, 200, "{first}");
    // **新しい名前を足す**（鍵は送らない）。
    let (status, second) = save(
        server.port,
        "fresh",
        &json!({ "revision": first["revision"], "main": main }),
    )
    .await;
    assert_eq!(status, 200, "{second}");

    let disk: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("読めること")).expect("JSON");
    assert_eq!(
        (
            &disk["keyed"]["main"]["api_key"],
            disk["keyed"]["sub"].get("api_key"),
            disk["fresh"]["main"].get("api_key"),
        ),
        (&json!(NORTH_MAIN_KEY), None, None),
        "残す鍵を別の面・別の名前へ写している: {disk}"
    );
    server.stop().await;
}

/// **使えない鍵・ユーザー情報のある宛先の鍵は、読み込みと同じ検証が止める。ファイルは変わらず、
/// 理由は定数で返り、応答に鍵は載らない**（ADR-0056 §決定1・3 / 作業単位 A-13b-1・A-13b-2）。
///
/// **送らなかった鍵を残した結果として止まる形も並べる**（宛先だけをユーザー情報つきに直す）——
/// 残す鍵も、新しく書く鍵と同じ規則を通ること。
///
/// > A-13b-1 はここで「平文で外へ流れる宛先」（`api_key_over_plain_http`）も止めていた。A-13b-2 で
/// > 送る側の判定（接続のたびの名前解決）に置き換えたので、その宛先は保存できる（止まるのは送るとき）。
#[tokio::test]
async fn an_unusable_key_is_refused_without_echoing_it() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let mut written = two_by_two();
    written["north"]["main"]["api_key"] = json!(NORTH_MAIN_KEY);
    let path = write_by_hand(dir.path(), &written);
    let original = std::fs::read(&path).expect("読めること");
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let revision = list(server.port).await["revision"].clone();

    let mut spaced = face("http://127.0.0.1:47801", "m", None, None, None);
    spaced["api_key"] = json!(format!("{REPLACED_KEY} spaced"));
    let mut userinfo = face("http://u:p@192.0.2.20:47802", "m", None, None, None);
    userinfo["api_key"] = json!(REPLACED_KEY);
    // **鍵は送らない（残す）が、宛先をユーザー情報つきに直す。**
    let mut moved = written["north"]["main"].clone();
    moved.as_object_mut().expect("面").remove("api_key");
    moved["endpoint"] = json!("http://u:p@192.0.2.21:47803");
    moved.as_object_mut().expect("面").remove("gguf");
    moved.as_object_mut().expect("面").remove("args");
    moved.as_object_mut().expect("面").remove("command");

    for (name, main, reason, key) in [
        ("spaced", &spaced, "api_key_unusable", "api_key"),
        ("userinfo", &userinfo, "api_key_with_userinfo", "endpoint"),
        ("north", &moved, "api_key_with_userinfo", "endpoint"),
    ] {
        let (status, body) = save(
            server.port,
            name,
            &json!({ "revision": revision, "main": main }),
        )
        .await;
        assert_eq!(status, 422, "{name}: {body}");
        assert_eq!(
            (
                &body["error"]["reason"],
                &body["error"]["face"],
                &body["error"]["key"]
            ),
            (&json!(reason), &json!("main"), &json!(key)),
            "{name}: {body}"
        );
        assert_no_key_in(&body);
        assert_eq!(
            std::fs::read(&path).expect("読めること"),
            original,
            "{name}: 止めたのにファイルが変わった"
        );
    }
    server.stop().await;
}

/// **いまのファイルが検証に通らなければ、画面からは直さない**（422。バイト列は変わらない）。
///
/// **一覧はその理由を定数で言い、止まったプロファイルの名前を挙げる。**
#[tokio::test]
async fn a_file_the_loader_rejects_is_listed_as_invalid_and_not_rewritten() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let mut broken = two_by_two();
    broken["south"]["sub"]["ggml"] = json!("typo");
    let path = write_by_hand(dir.path(), &broken);
    let original = std::fs::read(&path).expect("読めること");
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;

    let listing = list(server.port).await;
    assert_eq!(listing["state"], "invalid", "{listing}");
    assert_eq!(
        listing["problem"],
        json!({
            "reason": "unknown_key",
            "message": listing["problem"]["message"],
            "profile": "south",
            "face": "sub",
            "key": null,
        }),
        "{listing}"
    );
    assert!(
        !listing["problem"].to_string().contains("typo"),
        "綴りを反射している"
    );
    assert_eq!(listing["profiles"], json!([]));

    let (status, response) = save(
        server.port,
        "north",
        &json!({ "revision": listing["revision"],
                 "main": face("http://127.0.0.1:1", "m", None, None, None) }),
    )
    .await;
    assert_eq!(status, 422, "{response}");
    assert_eq!(response["error"]["code"], "upstream_profiles_unreadable");
    assert_eq!(std::fs::read(&path).expect("読めること"), original);
    server.stop().await;
}

/// **保存したプロファイルを、そのまま差し替えで選べる**（画面が書くファイル＝差し替えが読むファイル）。
///
/// **gguf の無いプロファイルだけ**（何も起こさない）。**宛先は起動時とも囮とも別。**
#[tokio::test]
async fn the_switch_selects_a_profile_that_was_just_saved() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &two_by_two());
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let revision = list(server.port).await["revision"].clone();

    // **宛先は答える偽物**（空いていたポートを使うと、差し替えの測り直しが
    // 同じポートを取った別の誰かに当たって固まりうる）。答えの中身は見ない。
    let upstream = wiremock::MockServer::start().await;
    let target = upstream.uri();
    let (status, saved) = save(
        server.port,
        "west",
        &json!({ "revision": revision, "main": face(&target, "west-model", None, None, None) }),
    )
    .await;
    assert_eq!(status, 200, "{saved}");

    let (status, report) = send(
        server.port,
        Method::POST,
        "/admin/upstream/profile",
        Some(&json!({ "name": "west" })),
    )
    .await;
    assert_eq!(
        status, 200,
        "保存したプロファイルを差し替えで選べない: {report}"
    );
    assert_eq!(
        report["main"]["endpoint"],
        json!(format!("{target}/v1/chat/completions"))
    );
    assert_eq!(report["main"]["model"], "west-model");
    server.stop().await;
}

/// **消す口は無い**（ADR-0042 §決定8「消す操作はまだ置かない」）。**一覧の側にも書く口は無い。**
#[tokio::test]
async fn no_path_deletes_a_profile() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &two_by_two());
    let original = std::fs::read(&path).expect("読めること");
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;

    for method in [Method::DELETE, Method::POST, Method::PATCH, Method::GET] {
        let (status, body) = send(
            server.port,
            method.clone(),
            "/admin/upstream/profiles/north",
            None,
        )
        .await;
        assert_eq!(status, 405, "{method}: {body}");
    }
    for method in [Method::DELETE, Method::POST, Method::PATCH, Method::PUT] {
        let (status, body) = send(
            server.port,
            method.clone(),
            "/admin/upstream/profiles",
            None,
        )
        .await;
        assert_eq!(status, 405, "{method}: {body}");
    }
    assert_eq!(std::fs::read(&path).expect("読めること"), original);
    server.stop().await;
}

/// **本物の置き換えが失敗しても、元のファイルが残り、一時ファイルも残らない**（Windows）。
///
/// # どう失敗させるか（**継ぎ目を使わない**）
///
/// **宛先を「読むのは許すが、消す・書くのは許さない」共有で開いておく**（`FILE_SHARE_READ` だけ）。
/// **読む（印の確認）は通り、置き換え（`MoveFileExW` の `REPLACE_EXISTING`）だけが共有違反で落ちる。**
/// 継ぎ目（`ReplaceSteps`）を差し替える腕は `src/server/profile_editor.rs` の単体テストにある——
/// **こちらは本番の入口・本物の手順で同じことを見る。**
#[cfg(windows)]
#[tokio::test]
async fn a_replacement_that_fails_on_disk_keeps_the_original() {
    use std::os::windows::fs::OpenOptionsExt as _;

    /// `FILE_SHARE_READ`（Win32）。
    const FILE_SHARE_READ: u32 = 0x0000_0001;

    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &two_by_two());
    let original = std::fs::read(&path).expect("読めること");
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let revision = list(server.port).await["revision"].clone();

    let holder = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&path)
        .expect("掴めること");
    let (status, response) = save(
        server.port,
        "south",
        &json!({ "revision": revision, "main": face("http://127.0.0.1:1", "m", None, None, None) }),
    )
    .await;
    drop(holder);

    assert_eq!(status, 500, "{response}");
    assert_eq!(response["error"]["code"], "upstream_profiles_write_failed");
    assert!(
        !response
            .to_string()
            .contains(&dir.path().display().to_string()),
        "応答に置き場所が載っている: {response}"
    );
    assert_eq!(std::fs::read(&path).expect("読めること"), original);
    assert_eq!(
        entries(dir.path()),
        vec!["db".to_owned(), "upstreams.json".to_owned()],
        "一時ファイルが残っている"
    );

    // **対照**: 手放せば、同じ依頼が通る（落ちた理由が掴んでいたことだと確かめる）。
    let (status, response) = save(
        server.port,
        "south",
        &json!({ "revision": revision, "main": face("http://127.0.0.1:1", "m", None, None, None) }),
    )
    .await;
    assert_eq!(status, 200, "{response}");
    server.stop().await;
}
