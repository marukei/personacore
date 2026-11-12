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
//! | `the_listing_carries_the_written_sampling_and_response_room_only_where_written` | 一覧の面が `sampling` / `response_tokens` を載せない・既定値で埋める・`{}` を `null` にしない・別の名前や面に出る（D-21 / ADR-0060 の腕6） |
//! | `omitting_the_fields_keeps_what_was_written_including_an_empty_object` | 依頼が欄を省くと手で書いた値（と `{}`）が消える（D-21 / ADR-0060 の腕1） |
//! | `null_removes_each_field_and_leaves_the_face_as_if_never_written` | `null` で消えない・消した面が書いていない面とバイト列で違う・片方の `null` がもう片方を消す（D-21 / ADR-0060 の腕2） |
//! | `a_sampling_in_the_request_replaces_the_whole_object` | `sampling` を鍵ごとに合成する・面を取り違える・他の名前が動く（D-21 / ADR-0060 の腕3） |
//! | `an_empty_sampling_object_removes_it_like_null` | `{}` をそのまま書く（D-21 / ADR-0060 の腕4） |
//! | `each_unusable_value_is_refused_with_the_loaders_key_and_reason` | 使えない値が書かれる・400 になる・`key` / `reason` が読み込みと違う（窓との境界は `context_tokens`）（D-21 / ADR-0060 の腕5） |

use std::path::{Path, PathBuf};
use std::time::Duration;

use personacore::server::{
    App, AppConfig, ENV_LLM_ENDPOINT, ENV_PORT, LaunchPlan, ServerError, Startup,
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
        // **番号は参照関数の `PERSONACORE_PORT` で明示する**（作業単位 F-11b）——取られていたら待ち受けを
        // 開けずに返る（下の起こし直しの前提）。明示していない番号（`with_bind`）は、取られていれば次の空きへ
        // ずれて起き、このポートには別の誰かが答える（ADR-0057 §決定1）。
        let port_text = port.to_string();
        let config = AppConfig::from_lookup_with_data_dir(
            move |key: &str| match key {
                ENV_LLM_ENDPOINT => Some(CLOSED_ENDPOINT.to_owned()),
                ENV_PORT => Some(port_text.clone()),
                _ => None,
            },
            Some(dir.to_path_buf()),
        )
        .expect("設定を組めること")
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
///
/// **`sampling` と `response_tokens` は書いていなければ `null` で載る**（作業単位 D-21 / ADR-0060 §決定1）。
/// **鍵を1つも持たない `sampling: {}` も `null`**（書いていない面と送る値が同じなので、見え方も揃える）。
fn as_listed(written: &Value) -> Value {
    let mut listed = json!({
        "gguf": null, "args": null, "command": null, "context_tokens": null,
        "api_key_set": false, "api_key_hint": null, "allow_key_over_plain_http": false,
        "sampling": null, "response_tokens": null,
    });
    for (key, value) in written.as_object().expect("面はオブジェクト") {
        if key == "api_key" {
            if let Some(raw) = value.as_str() {
                listed["api_key_set"] = json!(true);
                listed["api_key_hint"] = json!(&raw[raw.len() - 4..]);
            }
            continue;
        }
        if key == "sampling" && value.as_object().is_some_and(serde_json::Map::is_empty) {
            listed[key] = Value::Null;
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

/// **手で書いたサンプラの値（`sampling`）は、画面から保存しても残る。`endpoint` や `model` を変えた面でも残る。
/// 残すのはその名前・その面のものだけ**（[ADR-0058](../docs/adr/0058-sampling-values-in-the-upstream-profile.md)
/// §決定6 / 作業単位 A-16）。
///
/// 前例 [`a_hand_written_approval_survives_a_save_from_the_screen`] と同じ形。**承認と違い、宛先を変えても落とさない**
/// （知らせずに消えるほうが、知らせずに残るより見つけにくい）。
///
/// | 面 | 保存で変えたもの | 値 |
/// |---|---|---|
/// | `tuned` の本文（1.05） | `model` だけ | **残る** |
/// | `tuned` の裏方（1.25） | `endpoint` と `model` | **残る** |
/// | `fresh`（新しい名前） | —— | 書かれない（他の名前から拾わない） |
///
/// **値を面ごとに違える**（面を取り違えて写す実装を赤にする）。**一覧は値を書いたファイルでも読める**
/// （`sampling` は [`FACE_KEYS`] の鍵 —— 画面の形に写せないと言わない）。
///
/// > **A-16 の間は「依頼が `sampling` を送ると 400」も見ていた**（依頼の型が欄を持たなかった）。
/// > **作業単位 D-21（ADR-0060 §決定6）で依頼が欄を持ったので、その腕は ADR-0060 の腕3・腕5
/// > （[`a_sampling_in_the_request_replaces_the_whole_object`] / [`each_unusable_value_is_refused_with_the_loaders_key_and_reason`]）
/// > に置き換えた。** ここは「省いたら残す」（ADR-0060 の腕1）だけを、意味を変えずに見る（司令塔の決定）。
///
/// [`FACE_KEYS`]: personacore::server::FACE_KEYS
#[tokio::test]
async fn a_hand_written_sampling_survives_a_save_from_the_screen() {
    const SAMPLING: &str = "sampling";
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let source = json!({
        "tuned": {
            "main": { "endpoint": "http://127.0.0.1:47931", "model": "tuned-main",
                      SAMPLING: { "repeat_penalty": 1.05 } },
            "sub": { "endpoint": "http://127.0.0.1:47932", "model": "tuned-sub",
                     SAMPLING: { "repeat_penalty": 1.25 } }
        }
    });
    let path = write_by_hand(dir.path(), &source);
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let listing = list(server.port).await;
    assert_eq!(
        profile_named(&listing, "tuned")["main"]["model"],
        json!("tuned-main"),
        "前提: 値を書いたファイルが一覧に載ること: {listing}"
    );

    // **本文はモデルだけ、裏方は宛先とモデルを直す**（`sampling` は送らない）。新しい名前も足す。
    let main = face("http://127.0.0.1:47931", "tuned-main-2", None, None, None);
    let sub = face("http://127.0.0.1:47942", "tuned-sub-2", None, None, None);
    let (status, first) = save(
        server.port,
        "tuned",
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
            &disk["tuned"]["main"]["model"],
            &disk["tuned"]["sub"]["endpoint"],
            &disk["tuned"]["sub"]["model"],
        ),
        (
            &json!("tuned-main-2"),
            &json!("http://127.0.0.1:47942"),
            &json!("tuned-sub-2")
        ),
        "前提: 依頼どおりに保存されたこと: {disk}"
    );
    assert_eq!(
        (
            &disk["tuned"]["main"][SAMPLING],
            &disk["tuned"]["sub"][SAMPLING],
            disk["fresh"]["main"].get(SAMPLING),
        ),
        (
            &json!({ "repeat_penalty": 1.05 }),
            &json!({ "repeat_penalty": 1.25 }),
            None
        ),
        "同じ名前・同じ面の値が消えた／面を取り違えた／別の名前へ写った: {disk}"
    );
    server.stop().await;
}

/// **手で書いた本文の返答の上限（`response_tokens`）と思考の量は、画面から保存しても残る**
/// （[ADR-0059](../docs/adr/0059-thinking-and-response-room-in-the-upstream-profile.md) §決定5 / 作業単位 A-18a）。
///
/// 前例 [`a_hand_written_sampling_survives_a_save_from_the_screen`] と同じ形（`endpoint` と `model` を変えた保存でも残る）。
///
/// | 回 | 依頼 | 結果 |
/// |---|---|---|
/// | 2 | 窓（`context_tokens`）を上限（3000）以下の 2048 にする | **422 `context_tokens_unusable`・鍵 `context_tokens`**（窓の検証が面の上限に追従する）。ファイルは触らない |
/// | 3 | 宛先とモデルを直す（窓は 8192） | 200。**上限 3000 と `sampling.reasoning_effort` が残る** |
///
/// **一覧は値を書いたファイルでも読める**（`response_tokens` は [`FACE_KEYS`] の鍵）。
///
/// > **A-18a の間は1回目に「`response_tokens` を載せると 400」も見ていた**（依頼の型が欄を持たなかった）。
/// > **作業単位 D-21（ADR-0060 §決定6）でその腕は ADR-0060 の腕3・腕5 に置き換えた**（番号は元のまま残す）。
/// > 「省いたら残す」（ADR-0060 の腕1）は意味を変えずに残す（司令塔の決定）。
///
/// [`FACE_KEYS`]: personacore::server::FACE_KEYS
#[tokio::test]
async fn a_hand_written_response_room_survives_a_save_from_the_screen() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let source = json!({
        "thinking": {
            "main": { "endpoint": "http://127.0.0.1:47951", "model": "thinking-main",
                      "response_tokens": 3000, "sampling": { "reasoning_effort": "low" } }
        }
    });
    let path = write_by_hand(dir.path(), &source);
    let before = std::fs::read(&path).expect("読めること");
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let listing = list(server.port).await;
    assert_eq!(
        profile_named(&listing, "thinking")["main"]["model"],
        json!("thinking-main"),
        "前提: 値を書いたファイルが一覧に載ること: {listing}"
    );

    // **2. 窓を上限以下へ縮める保存は、窓の欄で断る。**
    let narrow = windowed(
        face("http://127.0.0.1:47951", "thinking-main", None, None, None),
        2048,
    );
    let (status, refused) = save(
        server.port,
        "thinking",
        &json!({ "revision": listing["revision"], "main": narrow }),
    )
    .await;
    assert_eq!(status, 422, "上限以下の窓を受け付けた: {refused}");
    assert_eq!(
        (
            &refused["error"]["code"],
            &refused["error"]["reason"],
            &refused["error"]["face"],
            &refused["error"]["key"],
        ),
        (
            &json!("upstream_profile_invalid"),
            &json!("context_tokens_unusable"),
            &json!("main"),
            &json!("context_tokens")
        ),
        "窓の欄に理由を結びつけること: {refused}"
    );
    assert_eq!(
        std::fs::read(&path).expect("読めること"),
        before,
        "断った保存でファイルが変わった"
    );

    // **3. 宛先とモデルを直す**（窓は上限より大きい 8192）。
    let main = windowed(
        face(
            "http://127.0.0.1:47959",
            "thinking-main-2",
            None,
            None,
            None,
        ),
        8192,
    );
    let (status, saved) = save(
        server.port,
        "thinking",
        &json!({ "revision": listing["revision"], "main": main }),
    )
    .await;
    assert_eq!(status, 200, "{saved}");
    let disk: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("読めること")).expect("JSON");
    assert_eq!(
        (
            &disk["thinking"]["main"]["endpoint"],
            &disk["thinking"]["main"]["model"],
            &disk["thinking"]["main"]["context_tokens"],
        ),
        (
            &json!("http://127.0.0.1:47959"),
            &json!("thinking-main-2"),
            &json!(8192)
        ),
        "前提: 依頼どおりに保存されたこと: {disk}"
    );
    assert_eq!(
        (
            &disk["thinking"]["main"]["response_tokens"],
            &disk["thinking"]["main"]["sampling"],
        ),
        (&json!(3000), &json!({ "reasoning_effort": "low" })),
        "手で書いた上限か思考の量が消えた: {disk}"
    );
    server.stop().await;
}

// ----------------------------------------------------------------------
// 依頼の `sampling` / `response_tokens`（作業単位 D-21 / ADR-0060 §決定1・2・§決定6 の腕1〜6）
// ----------------------------------------------------------------------

/// **D-21 の腕のフィクスチャ**（ADR-0060 §決定6）。**3件で、値は真ん中（`mid`）にだけ書く**（腕6 —— 端に置かない）。
///
/// **値は既定（1.1・1024・`"none"`）と違え、本文と裏方でも違える。** `omega` の本文は鍵を1つも持たない `{}`。
/// `mid` の本文の窓は 8192（腕5 の `response_tokens` の境界が使う）。
fn d21_profiles() -> Value {
    json!({
        "alpha": {
            "main": { "endpoint": "http://127.0.0.1:47971", "model": "alpha-main" },
            "sub":  { "endpoint": "http://127.0.0.1:47972", "model": "alpha-sub" }
        },
        "mid": {
            "main": { "endpoint": "http://127.0.0.1:47973", "model": "mid-main", "context_tokens": 8192,
                      "sampling": { "repeat_penalty": 1.05, "reasoning_effort": "low" },
                      "response_tokens": 2000 },
            "sub":  { "endpoint": "http://127.0.0.1:47974", "model": "mid-sub",
                      "sampling": { "repeat_penalty": 1.25 } }
        },
        "omega": {
            "main": { "endpoint": "http://127.0.0.1:47975", "model": "omega-main", "sampling": {} }
        }
    })
}

/// **`mid` の本文の依頼の面**（窓は書いてある 8192 のまま。`sampling` / `response_tokens` は省く＝残す）。
fn mid_main() -> Value {
    windowed(
        face("http://127.0.0.1:47973", "mid-main", None, None, None),
        8192,
    )
}

/// **`mid` の裏方の依頼の面**（`sampling` は省く＝残す）。
fn mid_sub() -> Value {
    face("http://127.0.0.1:47974", "mid-sub", None, None, None)
}

/// **その面の依頼に欄を足す**（`extra` の鍵をそのまま載せる。`null` も載せる）。
fn with_fields(mut face: Value, extra: &Value) -> Value {
    for (key, value) in extra.as_object().expect("足す欄はオブジェクト") {
        face[key] = value.clone();
    }
    face
}

/// **ファイルを JSON として読む。**
fn disk_of(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("読めること")).expect("JSON")
}

/// **その形のファイルを置いたサーバに1回だけ保存させ、保存の後のファイルのバイト列を返す**（腕2・腕4 の比べる側）。
async fn bytes_after_one_save(
    source: &Value,
    name: &str,
    main: &Value,
    sub: Option<&Value>,
) -> Vec<u8> {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), source);
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let listing = list(server.port).await;
    let (status, saved) = save(
        server.port,
        name,
        &json!({ "revision": listing["revision"], "main": main, "sub": sub }),
    )
    .await;
    assert_eq!(status, 200, "{saved}");
    let bytes = std::fs::read(&path).expect("読めること");
    server.stop().await;
    bytes
}

/// **腕6: 一覧の面は `sampling` と `response_tokens` を必ず載せ、書いた値を返す。書いていない面と `{}` の面は `null`**
/// （ADR-0060 §決定1 / §決定6 の腕6）。
///
/// **3件のうち真ん中の1件だけに値を書く**（端に置かない）。**既定値で埋めない**（書いていない面は `null` で、
/// 1.1 や 1024 を返さない）。欄が無い（省かれた）のと `null` は別のことなので、鍵が在ることも見る。
#[tokio::test]
async fn the_listing_carries_the_written_sampling_and_response_room_only_where_written() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &d21_profiles());
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let listing = list(server.port).await;
    assert_eq!(listing["state"], "ok", "{listing}");

    let faces = [
        ("alpha", "main"),
        ("alpha", "sub"),
        ("mid", "main"),
        ("mid", "sub"),
        ("omega", "main"),
    ];
    for (name, face_label) in faces {
        let listed = &profile_named(&listing, name)[face_label];
        for key in ["sampling", "response_tokens"] {
            assert!(
                listed
                    .as_object()
                    .is_some_and(|face| face.contains_key(key)),
                "{name} の {face_label} に {key} の欄が無い（null でも載せる）: {listing}"
            );
        }
    }
    let seen: Vec<(Value, Value)> = faces
        .iter()
        .map(|(name, face_label)| {
            let listed = &profile_named(&listing, name)[face_label];
            (
                listed["sampling"].clone(),
                listed["response_tokens"].clone(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        vec![
            (Value::Null, Value::Null),
            (Value::Null, Value::Null),
            (
                json!({ "repeat_penalty": 1.05, "reasoning_effort": "low" }),
                json!(2000)
            ),
            (json!({ "repeat_penalty": 1.25 }), Value::Null),
            (Value::Null, Value::Null),
        ],
        "書いた値が書いた面にだけ出ない／既定値で埋めた／{{}} を null にしていない: {listing}"
    );
    // **面の全部の欄も、書いたファイルから組んだ形と一致する**（帰属を丸ごと）。
    assert_eq!(
        profile_named(&listing, "mid")["main"],
        as_listed(&d21_profiles()["mid"]["main"]),
        "{listing}"
    );
    server.stop().await;
}

/// **腕1: 依頼が欄を省くと、手で書いた `sampling` / `response_tokens` と `{}` が残る**（ADR-0060 §決定2 の表の1行目）。
///
/// **API 1.8 までの使い手のための規則**（欄を知らない使い手が保存しても、手で書いた値が知らせずに消えない）。
/// 宛先とモデルを直した保存で見る（`endpoint` / `model` が変わっても落とさない —— ADR-0058 §決定6 のまま）。
/// **`omega` の `{}` も残る**（省いたら書いてあるものをそのまま残す。`null` に直さない）。
#[tokio::test]
async fn omitting_the_fields_keeps_what_was_written_including_an_empty_object() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &d21_profiles());
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let listing = list(server.port).await;

    let main = windowed(
        face("http://127.0.0.1:47983", "mid-main-2", None, None, None),
        8192,
    );
    let sub = face("http://127.0.0.1:47984", "mid-sub-2", None, None, None);
    let (status, first) = save(
        server.port,
        "mid",
        &json!({ "revision": listing["revision"], "main": main, "sub": sub }),
    )
    .await;
    assert_eq!(status, 200, "{first}");
    let omega = face("http://127.0.0.1:47985", "omega-main-2", None, None, None);
    let (status, second) = save(
        server.port,
        "omega",
        &json!({ "revision": first["revision"], "main": omega }),
    )
    .await;
    assert_eq!(status, 200, "{second}");

    let disk = disk_of(&path);
    assert_eq!(
        (
            &disk["mid"]["main"]["model"],
            &disk["mid"]["sub"]["endpoint"],
            &disk["omega"]["main"]["model"],
        ),
        (
            &json!("mid-main-2"),
            &json!("http://127.0.0.1:47984"),
            &json!("omega-main-2")
        ),
        "前提: 依頼どおりに保存されたこと: {disk}"
    );
    assert_eq!(
        (
            &disk["mid"]["main"]["sampling"],
            &disk["mid"]["main"]["response_tokens"],
            &disk["mid"]["sub"]["sampling"],
            &disk["omega"]["main"]["sampling"],
        ),
        (
            &json!({ "repeat_penalty": 1.05, "reasoning_effort": "low" }),
            &json!(2000),
            &json!({ "repeat_penalty": 1.25 }),
            &json!({}),
        ),
        "省いた欄の値が消えた／面を取り違えた／{{}} を残していない: {disk}"
    );
    assert_eq!(
        (
            profile_named(&second, "mid")["main"]["response_tokens"].clone(),
            profile_named(&second, "omega")["main"]["sampling"].clone(),
        ),
        (json!(2000), Value::Null),
        "保存の応答が、残した値を返していない: {second}"
    );
    server.stop().await;
}

/// **腕2: `null` で消え、消した面は書いていない面と1バイトも違わない。片方だけ `null` にすると、もう片方が残る**
/// （ADR-0060 §決定2 の表の2行目 / §決定6 の腕2）。
///
/// | 段 | いまのファイル | 依頼 | 見ること |
/// |---|---|---|---|
/// | 1 | `mid` に値あり（A）／値なし（B） | 両面とも `sampling: null`・本文の `response_tokens: null` | **A と B の保存の後のバイト列が一致する** |
/// | 2 | `mid` に値あり | 本文の `sampling: null` だけ（`response_tokens` は省く） | **`response_tokens` 2000 が残る** |
/// | 3 | `mid` に値あり | 本文の `response_tokens: null` だけ（`sampling` は省く） | **`sampling` が残る** |
#[tokio::test]
async fn null_removes_each_field_and_leaves_the_face_as_if_never_written() {
    let cleared_main = with_fields(
        mid_main(),
        &json!({ "sampling": null, "response_tokens": null }),
    );
    let cleared_sub = with_fields(mid_sub(), &json!({ "sampling": null }));
    let mut blank = d21_profiles();
    for (face_label, key) in [
        ("main", "sampling"),
        ("main", "response_tokens"),
        ("sub", "sampling"),
    ] {
        blank["mid"][face_label]
            .as_object_mut()
            .expect("面はオブジェクト")
            .remove(key);
    }
    let written =
        bytes_after_one_save(&d21_profiles(), "mid", &cleared_main, Some(&cleared_sub)).await;
    let never = bytes_after_one_save(&blank, "mid", &cleared_main, Some(&cleared_sub)).await;
    assert_eq!(
        String::from_utf8_lossy(&written),
        String::from_utf8_lossy(&never),
        "null で消した面が、書いていない面と違うバイト列になった"
    );
    let written: Value = serde_json::from_slice(&written).expect("JSON");
    assert_eq!(
        (
            written["mid"]["main"].get("sampling"),
            written["mid"]["main"].get("response_tokens"),
            written["mid"]["sub"].get("sampling"),
        ),
        (None, None, None),
        "null で消えていない: {written}"
    );

    // **片方だけ `null`。もう片方は省く（残す）。**
    for (extra, gone, kept, kept_value) in [
        (
            json!({ "sampling": null }),
            "sampling",
            "response_tokens",
            json!(2000),
        ),
        (
            json!({ "response_tokens": null }),
            "response_tokens",
            "sampling",
            json!({ "repeat_penalty": 1.05, "reasoning_effort": "low" }),
        ),
    ] {
        let bytes = bytes_after_one_save(
            &d21_profiles(),
            "mid",
            &with_fields(mid_main(), &extra),
            Some(&mid_sub()),
        )
        .await;
        let disk: Value = serde_json::from_slice(&bytes).expect("JSON");
        assert_eq!(
            (disk["mid"]["main"].get(gone), disk["mid"]["main"].get(kept)),
            (None, Some(&kept_value)),
            "{extra}: null の欄が消えない／省いた欄が消えた: {disk}"
        );
        assert_eq!(
            disk["mid"]["sub"]["sampling"],
            json!({ "repeat_penalty": 1.25 }),
            "{extra}: 裏方の値が動いた: {disk}"
        );
    }
}

/// **腕3: 値で丸ごと置き換わる**（ADR-0060 §決定2 の表の3行目 / §決定6 の腕3）。
///
/// **いまのファイル `{ repeat_penalty, reasoning_effort }` に依頼 `{ repeat_penalty }` を送ると、`reasoning_effort` が残らない**
/// （鍵ごとに合成しない）。`response_tokens` も置き換わり、裏方の値は裏方の依頼の値になる。**他の名前は動かない。**
/// 保存の応答（読み直した一覧）も、置き換えた値を返す。
#[tokio::test]
async fn a_sampling_in_the_request_replaces_the_whole_object() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &d21_profiles());
    let before = disk_of(&path);
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let listing = list(server.port).await;
    let main = with_fields(
        mid_main(),
        &json!({ "sampling": { "repeat_penalty": 0.9 }, "response_tokens": 3000 }),
    );
    let sub = with_fields(mid_sub(), &json!({ "sampling": { "repeat_penalty": 1.4 } }));
    let (status, saved) = save(
        server.port,
        "mid",
        &json!({ "revision": listing["revision"], "main": main, "sub": sub }),
    )
    .await;
    assert_eq!(status, 200, "{saved}");

    let disk = disk_of(&path);
    assert_eq!(
        (
            &disk["mid"]["main"]["sampling"],
            &disk["mid"]["main"]["response_tokens"],
            &disk["mid"]["sub"]["sampling"],
        ),
        (
            &json!({ "repeat_penalty": 0.9 }),
            &json!(3000),
            &json!({ "repeat_penalty": 1.4 }),
        ),
        "丸ごと置き換わっていない（鍵ごとに合成した／面を取り違えた）: {disk}"
    );
    assert_eq!(
        (&disk["alpha"], &disk["omega"]),
        (&before["alpha"], &before["omega"]),
        "他の名前が動いた: {disk}"
    );
    assert_eq!(
        profile_named(&saved, "mid")["main"],
        as_listed(&main),
        "{saved}"
    );
    assert_eq!(
        profile_named(&saved, "mid")["sub"],
        as_listed(&sub),
        "{saved}"
    );
    server.stop().await;
}

/// **腕4: `sampling` を書いてある面に `{}` を送ると消え、`null` を送ったときとバイト列が一致する**
/// （ADR-0060 §決定2 —— ファイルに `"sampling": {}` を書かない / §決定6 の腕4）。両面で見る。
#[tokio::test]
async fn an_empty_sampling_object_removes_it_like_null() {
    let send = |value: Value| {
        (
            with_fields(mid_main(), &json!({ "sampling": value.clone() })),
            with_fields(mid_sub(), &json!({ "sampling": value })),
        )
    };
    let (empty_main, empty_sub) = send(json!({}));
    let (null_main, null_sub) = send(Value::Null);
    let empty = bytes_after_one_save(&d21_profiles(), "mid", &empty_main, Some(&empty_sub)).await;
    let null = bytes_after_one_save(&d21_profiles(), "mid", &null_main, Some(&null_sub)).await;
    assert_eq!(
        String::from_utf8_lossy(&empty),
        String::from_utf8_lossy(&null),
        "{{}} と null の保存の後のバイト列が違う"
    );
    let disk: Value = serde_json::from_slice(&empty).expect("JSON");
    assert_eq!(
        (
            disk["mid"]["main"].get("sampling"),
            disk["mid"]["sub"].get("sampling"),
            &disk["mid"]["main"]["response_tokens"],
        ),
        (None, None, &json!(2000)),
        "{{}} で消えていない（か、省いた response_tokens が消えた）: {disk}"
    );
}

/// **生の本文で保存を頼む**（JSON として読めない本文の腕）。
async fn save_raw(port: u16, name: &str, body: &str) -> (u16, Value) {
    let response = reqwest::Client::new()
        .put(format!(
            "http://127.0.0.1:{port}/admin/upstream/profiles/{name}"
        ))
        .header("content-type", "application/json")
        .body(body.to_owned())
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .expect("管理 API へ繋がること");
    let status = response.status().as_u16();
    let text = response.text().await.expect("本文を読めること");
    let json = serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("JSON として読めること: {err} / {text}"));
    (status, json)
}

/// **腕5: 使えない値は、保存されずに（ファイルが1バイトも変わらずに）422 になり、`key` と `reason` は読み込みと同じ**
/// （ADR-0060 §決定2 の表の6行 / §決定6 の腕5）。
///
/// **中身は型の付かない JSON で受けて、判定を読み込みに委ねる** —— 負の数・文字列・小数も 400 ではなく 422 で、
/// どの欄の何が悪いかが返る。**`response_tokens` が同じ面の窓（8192）と等しい境界**も、窓の欄（`context_tokens`）で止まる。
/// **対照**: 窓より1小さい 8191 は通る（境界を1つ内側へずらす実装を赤にする）。
/// **400 のまま**なのは、本文が JSON として読めないときと、ほかの欄の形が違うとき（`context_tokens` の文字列）だけ。
#[tokio::test]
async fn each_unusable_value_is_refused_with_the_loaders_key_and_reason() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &d21_profiles());
    let before = std::fs::read(&path).expect("読めること");
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let listing = list(server.port).await;

    // （何を, 本文の面に足す欄, 裏方の面に足す欄, 面, key, reason）
    let rows: [(&str, Value, Value, &str, &str, &str); 15] = [
        (
            "sampling が数",
            json!({ "sampling": 5 }),
            json!({}),
            "main",
            "sampling",
            "sampling_unusable",
        ),
        (
            "sampling が文字列",
            json!({ "sampling": "low" }),
            json!({}),
            "main",
            "sampling",
            "sampling_unusable",
        ),
        (
            "repeat_penalty が範囲の外",
            json!({ "sampling": { "repeat_penalty": 2.5 } }),
            json!({}),
            "main",
            "sampling",
            "sampling_unusable",
        ),
        (
            "repeat_penalty が文字列",
            json!({ "sampling": { "repeat_penalty": "1.0" } }),
            json!({}),
            "main",
            "sampling",
            "sampling_unusable",
        ),
        (
            "reasoning_effort の知らない値",
            json!({ "sampling": { "reasoning_effort": "high" } }),
            json!({}),
            "main",
            "sampling",
            "sampling_unusable",
        ),
        (
            "sampling の知らない鍵",
            json!({ "sampling": { "top_k": 40 } }),
            json!({}),
            "main",
            "sampling",
            "unknown_sampling_key",
        ),
        (
            "裏方の reasoning_effort",
            json!({}),
            json!({ "sampling": { "reasoning_effort": "low" } }),
            "sub",
            "sampling",
            "main_face_only",
        ),
        (
            "裏方の response_tokens",
            json!({}),
            json!({ "response_tokens": 1500 }),
            "sub",
            "response_tokens",
            "main_face_only",
        ),
        (
            "response_tokens が 0",
            json!({ "response_tokens": 0 }),
            json!({}),
            "main",
            "response_tokens",
            "response_tokens_unusable",
        ),
        (
            "response_tokens が小数",
            json!({ "response_tokens": 1.5 }),
            json!({}),
            "main",
            "response_tokens",
            "response_tokens_unusable",
        ),
        (
            "response_tokens が負",
            json!({ "response_tokens": -1 }),
            json!({}),
            "main",
            "response_tokens",
            "response_tokens_unusable",
        ),
        (
            "response_tokens が文字列",
            json!({ "response_tokens": "2000" }),
            json!({}),
            "main",
            "response_tokens",
            "response_tokens_unusable",
        ),
        (
            "response_tokens が 32 ビットの外",
            json!({ "response_tokens": 4_294_967_296_u64 }),
            json!({}),
            "main",
            "response_tokens",
            "response_tokens_unusable",
        ),
        (
            "response_tokens が窓と等しい",
            json!({ "response_tokens": 8192 }),
            json!({}),
            "main",
            "context_tokens",
            "context_tokens_unusable",
        ),
        (
            "response_tokens が窓より大きい",
            json!({ "response_tokens": 9000 }),
            json!({}),
            "main",
            "context_tokens",
            "context_tokens_unusable",
        ),
    ];
    for (what, main_extra, sub_extra, face_label, key, reason) in rows {
        let body = json!({
            "revision": listing["revision"],
            "main": with_fields(mid_main(), &main_extra),
            "sub": with_fields(mid_sub(), &sub_extra),
        });
        let (status, refused) = save(server.port, "mid", &body).await;
        assert_eq!(status, 422, "{what}: 422 でない: {refused}");
        assert_eq!(
            (
                &refused["error"]["code"],
                &refused["error"]["face"],
                &refused["error"]["key"],
                &refused["error"]["reason"],
            ),
            (
                &json!("upstream_profile_invalid"),
                &json!(face_label),
                &json!(key),
                &json!(reason),
            ),
            "{what}: {refused}"
        );
        assert_eq!(
            std::fs::read(&path).expect("読めること"),
            before,
            "{what}: 断った保存でファイルが変わった"
        );
    }

    // **400 のままの2つ**（本文が JSON として読めない・ほかの欄の形が違う）。ファイルは触らない。
    let (status, refused) = save_raw(server.port, "mid", "{ not json").await;
    assert_eq!(
        (status, &refused["error"]["code"]),
        (400, &json!("invalid_body")),
        "{refused}"
    );
    let (status, refused) = save(
        server.port,
        "mid",
        &json!({
            "revision": listing["revision"],
            "main": with_fields(mid_main(), &json!({ "context_tokens": "8192" })),
        }),
    )
    .await;
    assert_eq!(
        (status, &refused["error"]["code"]),
        (400, &json!("invalid_body")),
        "{refused}"
    );
    assert_eq!(std::fs::read(&path).expect("読めること"), before);

    // **対照**: 窓より1小さい上限は通る。
    let (status, saved) = save(
        server.port,
        "mid",
        &json!({
            "revision": listing["revision"],
            "main": with_fields(mid_main(), &json!({ "response_tokens": 8191 })),
            "sub": mid_sub(),
        }),
    )
    .await;
    assert_eq!(status, 200, "窓より小さい上限を断った: {saved}");
    assert_eq!(
        disk_of(&path)["mid"]["main"]["response_tokens"],
        json!(8191)
    );
    server.stop().await;
}

/// **桁の多い繰り返しの罰（17 桁以上の小数）を送っても、保存は書き換えた後に 500 にならない**（D-21 のレビュー F1）。
///
/// この依存の `serde_json` は `float_roundtrip` を有効にしていないので、数の読みは最も近い倍精度から 1 ULP ずれうる。
/// **実測**（2026-11-13 / `<measurements>\d21-2026-11-13\PROGRESS.md`）: `1.8893261662489246058` を読むと
/// `1.8893261662489247` と書かれ、それを読み直すと `1.889326166248925` になる —— 依頼の値と読み直した値が
/// `Value` として違い、保存の後の読み直しの確認（`is_reflected_in`）が `upstream_profiles_verify_failed` で落ちていた。
/// **画面は最短の綴りで送る**（JS の `JSON.stringify`）ので、この形を作るのは API の使い手。
/// 照合は**意味で比べる**（数は 4 ULP 以内を同じとみなす）。
#[tokio::test]
async fn a_long_decimal_repeat_penalty_is_saved_without_a_verify_failure() {
    let dir = tempfile::tempdir().expect("一時ディレクトリ");
    let path = write_by_hand(dir.path(), &d21_profiles());
    let server = serve(dir.path(), |s| s.with_editable_profiles_file(path.clone())).await;
    let listing = list(server.port).await;
    let body = format!(
        r#"{{ "revision": {}, "main": {{ "endpoint": "http://127.0.0.1:47973", "model": "mid-main",
              "context_tokens": 8192, "sampling": {{ "repeat_penalty": 1.8893261662489246058 }} }},
              "sub": {} }}"#,
        listing["revision"],
        mid_sub()
    );
    let (status, saved) = save_raw(server.port, "mid", &body).await;
    assert_eq!(status, 200, "桁の多い繰り返しの罰の保存が通らない: {saved}");
    let written = disk_of(&path)["mid"]["main"]["sampling"]["repeat_penalty"]
        .as_f64()
        .expect("数で書かれていること");
    assert!(
        (written - 1.889_326_166_248_924_6).abs() < 1e-14,
        "書いた値が送った値から離れている: {written}"
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
