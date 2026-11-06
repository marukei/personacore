//! **上流プロファイルを読む純粋関数**（[ADR-0042](../docs/adr/0042-the-executable-owns-the-upstream-it-starts.md)
//! §決定1 / 作業単位 F-4b-1）。
//!
//! # なぜここで厚く見るのか
//!
//! **`F-9` の検証規則を PowerShell から丸ごと引き取った。**
//! **写した先が無防備なら、写した瞬間に守りが消える**（CLAUDE.md `B-6`）——
//! あちらは `tests/start_script.rs` が**綴りだけ**を見ていたが、
//! こちらは**実際にその判定を通す。**
//!
//! # 何を守っていないか（**先に書く**）
//!
//! * **プロセスは1つも起こさない。** 起こす側は `tests/upstream_cli.rs`。
//! * **ファイルも読まない**（見本を1本読む腕を除く）。`~` の展開先は引数で渡す。

use std::path::{Path, PathBuf};

use personacore::measured::ModelFace;
use personacore::server::{
    DEFAULT_PORT, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL, ENV_SUB_LLM_ENDPOINT, ENV_SUB_MODEL,
    UpstreamProfiles, overlay_profile, parse_profiles,
};

/// 見本（リポジトリのルートからの位置）。
const SAMPLE: &str = "scripts/upstreams.sample.json";

/// `~` の展開先。**実際のホームではない**——実環境に寄りかからない。
fn home() -> PathBuf {
    PathBuf::from(if cfg!(windows) {
        r"C:\planted-home"
    } else {
        "/planted-home"
    })
}

fn parse(source: &str) -> UpstreamProfiles {
    parse_profiles(source, Some(&home())).expect("植えたプロファイルが読めること")
}

/// **`Display` と `#[source]` の鎖を1つの綴りに畳む。**
///
/// **外向きの `Display` は定数文字列**（CLAUDE.md のエラーの作法）なので、
/// **何が駄目だったかは `#[source]` 側にしか無い。**
fn rendered_chain(err: &dyn std::error::Error) -> String {
    let mut rendered = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        rendered.push_str(" / ");
        rendered.push_str(&cause.to_string());
        source = cause.source();
    }
    rendered
}

fn error_of(source: &str) -> String {
    let err = parse_profiles(source, Some(&home())).expect_err("読めないこと");
    rendered_chain(&err)
}

/// **本文だけの最小のプロファイル。**
const MINIMAL: &str =
    r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m" } } }"#;

// ----------------------------------------------------------------------
// 形（`F-9` から引き取った規則）
// ----------------------------------------------------------------------

/// **知らない面は、名前を挙げて止まる。**
#[test]
fn an_unknown_face_is_named_and_stops_the_parse() {
    let rendered = error_of(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m" },
                     "third": { "endpoint": "http://127.0.0.1:8081", "model": "n" } } }"#,
    );
    assert!(
        rendered.contains("third"),
        "知らない面の綴りを挙げること: {rendered}"
    );
}

/// **知らない鍵も、綴りを挙げて止まる**（`gguf` を `ggml` と書いた回が原因から遠い）。
#[test]
fn an_unknown_key_is_named_and_stops_the_parse() {
    let rendered = error_of(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m", "ggml": "x" } } }"#,
    );
    assert!(
        rendered.contains("ggml"),
        "知らない鍵の綴りを挙げること: {rendered}"
    );
}

/// **`command` は知っている鍵である**（ADR-0042 §決定1 で足した1つ）。
#[test]
fn the_command_key_is_known_and_defaults_to_llama() {
    let profiles = parse(MINIMAL);
    let one = profiles.select("one").expect("在ること");
    assert!(one.main().launch().is_none(), "gguf が無ければ起こさない");

    let named = parse(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m",
                                "gguf": "~/a.gguf" } },
             "two": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m",
                                "gguf": "~/a.gguf", "command": "D:/tools/llama.exe" } } }"#,
    );
    assert_eq!(
        named
            .select("one")
            .expect("在ること")
            .main()
            .launch()
            .expect("起こす面であること")
            .command(),
        "llama",
        "既定の実行ファイル名"
    );
    assert_eq!(
        named
            .select("two")
            .expect("在ること")
            .main()
            .launch()
            .expect("起こす面であること")
            .command(),
        "D:/tools/llama.exe",
        "書いてあれば、そちらを起こす"
    );
}

/// **`endpoint` と `model` は省けない。**
#[test]
fn a_face_without_endpoint_or_model_stops_the_parse() {
    for (source, missing) in [
        (r#"{ "one": { "main": { "model": "m" } } }"#, "endpoint"),
        (
            r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080" } } }"#,
            "model",
        ),
    ] {
        let rendered = error_of(source);
        assert!(
            rendered.contains(missing),
            "欠けている鍵を挙げること（{missing}）: {rendered}"
        );
    }
}

/// **`main` は省けない**（本文の上流が無いプロファイルは成り立たない）。
#[test]
fn a_profile_without_main_stops_the_parse() {
    let rendered =
        error_of(r#"{ "one": { "sub": { "endpoint": "http://127.0.0.1:8080", "model": "m" } } }"#);
    assert!(rendered.contains("main"), "main を挙げること: {rendered}");
}

/// **gguf を書いた面の宛先は、ループバックだけ**（そのホストが `--host` へ渡る）。
///
/// # 空振りではないこと
///
/// **gguf を書いていない面は縛らない**——縛ると「2台目の機に上流を置く」形
/// （ADR-0039 §結果）が表現できなくなる。**両向きを1本で見る。**
#[test]
fn only_a_launched_face_is_pinned_to_the_loopback() {
    let rendered = error_of(
        r#"{ "one": { "main": { "endpoint": "http://192.168.0.10:8080", "model": "m",
                                "gguf": "~/a.gguf" } } }"#,
    );
    assert!(
        rendered.contains("192.168.0.10"),
        "外向きの宛先を挙げて止まること: {rendered}"
    );

    // **起こさない面は通る。**
    let far =
        parse(r#"{ "one": { "main": { "endpoint": "http://192.168.0.10:8080", "model": "m" } } }"#);
    assert_eq!(
        far.select("one").expect("在ること").main().endpoint(),
        "http://192.168.0.10:8080",
        "gguf の無い面は縛らない"
    );
}

/// **`localhost` と `[::1]` もループバック。**
#[test]
fn the_loopback_check_knows_the_three_spellings() {
    for endpoint in [
        "http://127.0.0.1:8080",
        "http://localhost:8080",
        "http://[::1]:8080",
    ] {
        let source = format!(
            r#"{{ "one": {{ "main": {{ "endpoint": "{endpoint}", "model": "m", "gguf": "~/a.gguf" }} }} }}"#
        );
        assert!(
            parse_profiles(&source, Some(&home())).is_ok(),
            "{endpoint} はループバックとして通ること"
        );
    }
}

/// **`~` は渡されたホームへ展開する**（見本は機をまたいで写される）。
#[test]
fn the_tilde_is_expanded_against_the_home_that_was_handed_in() {
    let profiles = parse(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m",
                                "gguf": "~/models/deep.gguf" } } }"#,
    );
    let launch = profiles
        .select("one")
        .expect("在ること")
        .main()
        .launch()
        .expect("起こす面であること");
    assert_eq!(
        launch.gguf(),
        home().join("models").join("deep.gguf"),
        "渡されたホームの下へ展開すること"
    );
}

/// **`--host` と `--port` は `endpoint` から作る**（`args` に書かせない）。
#[test]
fn the_host_and_port_come_from_the_endpoint() {
    let profiles = parse(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8123", "model": "m",
                                "gguf": "~/a.gguf", "args": ["--ctx-size", "16384"] } } }"#,
    );
    let launch = profiles
        .select("one")
        .expect("在ること")
        .main()
        .launch()
        .expect("起こす面であること");
    assert_eq!(launch.serve_host(), "127.0.0.1");
    assert_eq!(launch.serve_port(), 8123);
    assert_eq!(launch.extra_args(), ["--ctx-size", "16384"]);
}

/// **`args` に `--host` / `--port` を書いたら、綴りを挙げて止まる**（作業単位 F-4b-2 / Issue origin#2）。
///
/// # なぜ要るのか
///
/// **`--host` は `endpoint` から作って先頭側に渡す**（`launch::spawn_one`）が、
/// **`args` はその後ろへ足される。** 同じ旗が2回来たとき、**後ろが勝つ実装は珍しくない**
/// ——`"args": ["--host", "0.0.0.0"]` は、ループバックの検証を通ったまま
/// **外から繋がる待ち受けを、この実行ファイルに作らせる。**
///
/// **`=` で繋いだ形も同じ**（`--host=0.0.0.0`）。
/// **ループバックの値でも止める** —— 値を読んで許す形にすると、
/// 「どの綴りなら安全か」の判定がここにもう1つ生える（`endpoint` の側に1つあれば足りる）。
///
/// # 空振りではないこと
///
/// **同じ形の `args` から旗だけを抜いたものは通る**（下の対照）。
#[test]
fn the_host_and_port_cannot_be_given_again_in_the_args() {
    for args in [
        r#"["--host", "0.0.0.0"]"#,
        r#"["--host=0.0.0.0"]"#,
        r#"["--ctx-size", "16384", "--host", "::"]"#,
        r#"["--host", "127.0.0.1"]"#,
        r#"["--port", "9"]"#,
        r#"["--port=9"]"#,
    ] {
        let rendered = error_of(&format!(
            r#"{{ "one": {{ "main": {{ "endpoint": "http://127.0.0.1:8123", "model": "m",
                                     "gguf": "~/a.gguf", "args": {args} }} }} }}"#
        ));
        let named = if args.contains("--host") {
            "--host"
        } else {
            "--port"
        };
        assert!(
            rendered.contains(named),
            "args の {named} を挙げて止まること（{args}）: {rendered}"
        );
        assert!(
            rendered.contains("endpoint"),
            "どこに書けばよいか（endpoint）を言うこと（{args}）: {rendered}"
        );
    }

    // **対照**: 旗の綴りに似ているだけのもの（`--hostname-hint` / `--portable`）は止めない。
    // **値として現れる `--host` は区別しない**（どの旗が値を取るかを、ここは知らない）。
    let profiles = parse(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8123", "model": "m",
                                "gguf": "~/a.gguf",
                                "args": ["--hostname-hint", "x", "--portable"] } } }"#,
    );
    let launch = profiles
        .select("one")
        .expect("在ること")
        .main()
        .launch()
        .expect("起こす面であること");
    assert_eq!(
        launch.extra_args(),
        ["--hostname-hint", "x", "--portable"],
        "旗に似た綴りまで止めている"
    );
}

// ----------------------------------------------------------------------
// 4つの欄へ流す（**環境変数と同じ入口**）
// ----------------------------------------------------------------------

/// **`sub` を省いたら、抽出も本文と同じ上流**（ADR-0039 §決定1）。
///
/// 宛先は**空文字**で渡す（サーバは空を「未設定」と読む）。モデルの綴りは本文と同じ。
#[test]
fn a_profile_without_sub_sends_the_extraction_to_the_body_upstream() {
    let profiles = parse(MINIMAL);
    let settings = profiles.select("one").expect("在ること").settings();

    assert_eq!(settings.endpoint(), "http://127.0.0.1:8080");
    assert_eq!(settings.model(), "m");
    assert_eq!(settings.sub_endpoint(), "", "未設定として渡すこと");
    assert_eq!(settings.sub_model(), "m", "綴りは本文と同じ");
}

/// **`sub` を書いたら、4つとも別**（帰属。同値のフィクスチャでは取り違えが見えない）。
#[test]
fn a_profile_with_sub_keeps_the_two_faces_apart() {
    let profiles = parse(
        r#"{ "split": { "main": { "endpoint": "http://127.0.0.1:1234", "model": "body-model" },
                        "sub":  { "endpoint": "http://127.0.0.1:8080", "model": "sub-model" } } }"#,
    );
    let settings = profiles.select("split").expect("在ること").settings();

    assert_ne!(settings.endpoint(), settings.sub_endpoint());
    assert_ne!(settings.model(), settings.sub_model());
    assert_eq!(settings.endpoint(), "http://127.0.0.1:1234");
    assert_eq!(settings.model(), "body-model");
    assert_eq!(settings.sub_endpoint(), "http://127.0.0.1:8080");
    assert_eq!(settings.sub_model(), "sub-model");
}

/// **プロファイルが勝つ。そして、勝った欄以外は環境変数がそのまま通る。**
#[test]
fn the_profile_wins_over_the_environment_only_for_its_own_four_fields() {
    let profiles = parse(
        r#"{ "split": { "main": { "endpoint": "http://127.0.0.1:1234", "model": "body-model" },
                        "sub":  { "endpoint": "http://127.0.0.1:8080", "model": "sub-model" } } }"#,
    );
    let settings = profiles.select("split").expect("在ること").settings();

    // **環境側は4つとも別の値**（同値なら、どちらが勝ったか観測できない）。
    let base = |key: &str| match key {
        k if k == ENV_LLM_ENDPOINT => Some("http://127.0.0.1:19001".to_owned()),
        k if k == ENV_MAIN_MODEL => Some("env-body".to_owned()),
        k if k == ENV_SUB_LLM_ENDPOINT => Some("http://127.0.0.1:19002".to_owned()),
        k if k == ENV_SUB_MODEL => Some("env-sub".to_owned()),
        "PERSONACORE_PORT" => Some("19003".to_owned()),
        _ => None,
    };
    let lookup = overlay_profile(&settings, base);

    assert_eq!(
        lookup(ENV_LLM_ENDPOINT).as_deref(),
        Some("http://127.0.0.1:1234")
    );
    assert_eq!(lookup(ENV_MAIN_MODEL).as_deref(), Some("body-model"));
    assert_eq!(
        lookup(ENV_SUB_LLM_ENDPOINT).as_deref(),
        Some("http://127.0.0.1:8080")
    );
    assert_eq!(lookup(ENV_SUB_MODEL).as_deref(), Some("sub-model"));
    // **4つ以外は1バイトも触らない。**
    assert_eq!(lookup("PERSONACORE_PORT").as_deref(), Some("19003"));
    assert_eq!(lookup("PERSONACORE_DB_PATH"), None);
}

/// **名前が無ければ、在る名前を並べて止まる**（黙って既定へ落ちない）。
#[test]
fn an_unknown_profile_name_lists_the_names_that_exist() {
    let profiles = parse(
        r#"{ "beta": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m" } },
             "alpha": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m" } } }"#,
    );
    let err = profiles.select("gamma").expect_err("無いこと");
    let rendered = rendered_chain(&err);
    assert!(
        rendered.contains("alpha"),
        "在る名前を挙げること: {rendered}"
    );
    assert!(
        rendered.contains("beta"),
        "在る名前を挙げること: {rendered}"
    );
    assert!(
        rendered.find("alpha") < rendered.find("beta"),
        "名前は並べ替えて挙げること（読み手が探せる）: {rendered}"
    );
}

// ----------------------------------------------------------------------
// コンテキスト長（ADR-0054 §決定1 / 作業単位 A-13a-1）
// ----------------------------------------------------------------------

/// 本文の面に書く値。**既定（8192）とも落とし所（16384）とも、`sub` の値とも違う。**
const MAIN_WRITTEN: u32 = 28_672;

/// 抽出の面に書く値。**本文の値とも既定とも落とし所とも違う。**
const SUB_WRITTEN: u32 = 10_240;

/// **面ごとの `context_tokens` を読み、`sub` 面が無いときだけ本文の値を抽出にも使う**
/// （ADR-0054 §決定1）。
///
/// **4つのプロファイルを並べ、どれも違う組になるようにする**——
/// 「`sub` 面があっても本文の値を写す」実装は `sub_unwritten` で、
/// 「`sub` 面が無くても写さない」実装は `main_only` で、
/// 「本文と抽出を取り違える」実装は `both` で赤になる。
#[test]
fn each_face_reads_its_own_context_tokens_and_a_missing_sub_takes_the_main_value() {
    let profiles = parse(&format!(
        r#"{{
        "both": {{ "main": {{ "endpoint": "http://127.0.0.1:8080", "model": "m", "context_tokens": {MAIN_WRITTEN} }},
                   "sub":  {{ "endpoint": "http://127.0.0.1:8081", "model": "s", "context_tokens": {SUB_WRITTEN} }} }},
        "main_only": {{ "main": {{ "endpoint": "http://127.0.0.1:8080", "model": "m", "context_tokens": {MAIN_WRITTEN} }} }},
        "sub_unwritten": {{ "main": {{ "endpoint": "http://127.0.0.1:8080", "model": "m", "context_tokens": {MAIN_WRITTEN} }},
                            "sub":  {{ "endpoint": "http://127.0.0.1:8081", "model": "s" }} }},
        "none": {{ "main": {{ "endpoint": "http://127.0.0.1:8080", "model": "m" }} }}
    }}"#
    ));

    let read = |name: &str| {
        let profile = profiles.select(name).expect("在ること");
        let settings = profile.settings();
        (
            profile.main().context_tokens(),
            profile.sub().and_then(|sub| sub.context_tokens()),
            settings.context_tokens(),
            settings.sub_context_tokens(),
        )
    };

    assert_eq!(
        read("both"),
        (
            Some(MAIN_WRITTEN),
            Some(SUB_WRITTEN),
            Some(MAIN_WRITTEN),
            Some(SUB_WRITTEN)
        ),
        "面ごとの値（取り違えていないか）"
    );
    assert_eq!(
        read("main_only"),
        (
            Some(MAIN_WRITTEN),
            None,
            Some(MAIN_WRITTEN),
            Some(MAIN_WRITTEN)
        ),
        "sub 面が無ければ、本文の値を抽出にも使う"
    );
    assert_eq!(
        read("sub_unwritten"),
        (Some(MAIN_WRITTEN), None, Some(MAIN_WRITTEN), None),
        "sub 面があって値が無ければ、本文の値を写さない（照会する）"
    );
    assert_eq!(
        read("none"),
        (None, None, None, None),
        "書かなければ何も明示しない"
    );
}

/// **`context_tokens` は `PERSONACORE_CONTEXT_TOKENS` と同じ規則で止める**
/// （ADR-0054 §決定1 —— 0 と、応答の余白 1024 以下は止める）。
///
/// **整数以外も止める**（文字列の `"32768"`・小数・負数・`null`・`u32` に入らない数）。
/// **どの面でも同じ**——`sub` に書いた値も止まることを、`sub` の腕で見る。
///
/// # 空振りではないこと
///
/// **余白の1つ上（1025）は通る**（下の対照）。境界を両側から押さえる。
#[test]
fn an_unusable_context_tokens_stops_the_parse_on_either_face() {
    for raw in ["0", "1024", "-1", "1.5", r#""32768""#, "null", "4294967296"] {
        for face in ["main", "sub"] {
            let (main_extra, sub_extra) = if face == "main" {
                (format!(r#", "context_tokens": {raw}"#), String::new())
            } else {
                (String::new(), format!(r#", "context_tokens": {raw}"#))
            };
            let rendered = error_of(&format!(
                r#"{{ "one": {{ "main": {{ "endpoint": "http://127.0.0.1:8080", "model": "m"{main_extra} }},
                              "sub":  {{ "endpoint": "http://127.0.0.1:8081", "model": "s"{sub_extra} }} }} }}"#
            ));
            assert!(
                rendered.contains("context_tokens"),
                "{face} の {raw} で、鍵の綴りを挙げて止まること: {rendered}"
            );
            assert!(
                rendered.contains(face),
                "{face} の {raw} で、どの面かを挙げること: {rendered}"
            );
        }
    }

    // **対照**: 余白の1つ上は通る。
    let profiles = parse(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m", "context_tokens": 1025 } } }"#,
    );
    assert_eq!(
        profiles
            .select("one")
            .expect("在ること")
            .main()
            .context_tokens(),
        Some(1025)
    );
}

/// **`context_tokens` を書いても、他の鍵の綴り違いは今までどおり止まる**
/// （知らない鍵の一覧に `context_tokens` を足しただけで、検査を緩めていない）。
#[test]
fn an_unknown_key_beside_context_tokens_still_stops_the_parse() {
    let rendered = error_of(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m",
                                "context_tokens": 32768, "context_token": 1 } } }"#,
    );
    assert!(
        rendered.contains("context_token:") || rendered.contains("context_token（"),
        "綴り違いの鍵を挙げること: {rendered}"
    );
    assert!(
        rendered.contains("context_tokens"),
        "使える鍵の一覧に context_tokens が並ぶこと: {rendered}"
    );
}

/// **`context_tokens` は起こす子の記述に入らない**——値だけを変えたプロファイルへ
/// 差し替えても、子を起こし直さない（`server::switch` の「記述が同じなら残す」）。
#[test]
fn the_context_tokens_are_not_part_of_what_is_launched() {
    let profiles = parse(
        r#"{ "a": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m",
                              "gguf": "~/a.gguf", "context_tokens": 32768 } },
             "b": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m",
                              "gguf": "~/a.gguf", "context_tokens": 20480 } } }"#,
    );
    assert_eq!(
        profiles.select("a").expect("在ること").launches(),
        profiles.select("b").expect("在ること").launches(),
        "値だけが違う2つのプロファイルで、起こす子の記述が違っている"
    );
}

// ----------------------------------------------------------------------
// 起こすものの束ね方
// ----------------------------------------------------------------------

/// **`sub` を書いていないプロファイルでは、1つの子が両方の面に答える**
/// （ADR-0039 §決定1 / ADR-0042 §決定3）。
///
/// # なぜ面の帳簿に `sub` を入れるのか（**2026-09-19 に実測して足した**）
///
/// **入れないと、起動時に抽出の面だけが「まだ載っていない上流」へ聞きに行く。**
/// **しかも本文の面は、宛先が同じなら抽出側の一覧を借りる**
/// （`server::App::bind` の表）**ので、借りた先で `Drift::Absent` が並び直し、
/// 「起こす面には聞かない」という決定が丸ごと無効になる。**
/// **実測: 先客を立てた状態で `main.measured.drifted_absent` が 4 になった**
/// （聞いていないはずの面で）。
#[test]
fn a_profile_without_sub_launches_one_child_for_both_faces() {
    let profiles = parse(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m",
                                "gguf": "~/a.gguf" } } }"#,
    );
    let profile = profiles.select("one").expect("在ること");
    assert_eq!(profile.launches().len(), 1, "起こすのは1つ");

    let faces = profile.launches()[0].faces();
    assert!(
        faces.contains(&ModelFace::Main),
        "本文の面が入っていない: {faces:?}"
    );
    assert!(
        faces.contains(&ModelFace::Sub),
        "sub を書いていないのに、抽出の面が入っていない（抽出は本文と同じ上流である）: {faces:?}"
    );
}

/// **対比: `sub` を書いたら、その面は別に扱う**（同じ子には入らない）。
///
/// **これが無いと、「常に両方の面を入れる」実装がここで緑を通る**
/// （CLAUDE.md「対比は安い側だけでは足りない」）。
#[test]
fn a_profile_with_an_external_sub_launches_only_the_body_face() {
    let profiles = parse(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m",
                                "gguf": "~/a.gguf" },
                      "sub":  { "endpoint": "http://127.0.0.1:1234", "model": "n" } } }"#,
    );
    let profile = profiles.select("one").expect("在ること");
    assert_eq!(profile.launches().len(), 1);
    assert_eq!(
        profile.launches()[0].faces(),
        [ModelFace::Main],
        "外の上流を書いた面まで、起こした子の担当にしている"
    );
}

/// **同じ宛先に同じ gguf なら、起こすのは1つだけ。**
#[test]
fn two_faces_on_one_endpoint_with_one_gguf_are_launched_once() {
    let profiles = parse(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m",
                                "gguf": "~/a.gguf" },
                      "sub":  { "endpoint": "http://127.0.0.1:8080", "model": "n",
                                "gguf": "~/a.gguf" } } }"#,
    );
    let profile = profiles.select("one").expect("在ること");
    assert_eq!(
        profile.launches().len(),
        1,
        "1 プロセス 1 モデル。同じものを2回起こさない"
    );
    assert_eq!(
        profile.launches()[0].faces().len(),
        2,
        "1つの子が2つの面を担う"
    );
}

/// **宛先が別なら、別々に起こす**（`split` の形）。
#[test]
fn two_faces_on_two_endpoints_are_launched_twice() {
    let profiles = parse(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m",
                                "gguf": "~/a.gguf" },
                      "sub":  { "endpoint": "http://127.0.0.1:8081", "model": "n",
                                "gguf": "~/b.gguf" } } }"#,
    );
    assert_eq!(
        profiles.select("one").expect("在ること").launches().len(),
        2
    );
}

/// **同じポートに別の gguf は止める**（1 プロセス 1 モデル）。
#[test]
fn two_gguf_on_one_port_stops_the_parse() {
    let rendered = error_of(
        r#"{ "one": { "main": { "endpoint": "http://127.0.0.1:8080", "model": "m",
                                "gguf": "~/a.gguf" },
                      "sub":  { "endpoint": "http://127.0.0.1:8080", "model": "n",
                                "gguf": "~/b.gguf" } } }"#,
    );
    assert!(
        rendered.contains("8080"),
        "同じポートであることを挙げること: {rendered}"
    );
}

// ----------------------------------------------------------------------
// 見本（**実ファイルを、本番と同じ判定に通す**）
// ----------------------------------------------------------------------

/// **見本は、実行ファイルが読む判定をそのまま通ること。**
///
/// **`tests/start_script.rs` の走査とは別の主張**——あちらは JSON の形を
/// 自前で歩いて見るだけで、**本番のパーサを1度も通していない。**
#[test]
fn the_sample_profile_parses_with_the_parser_the_executable_uses() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(SAMPLE);
    let source = std::fs::read_to_string(&path).expect("見本を読めること");
    let profiles = parse_profiles(&source, Some(&home())).expect("見本が本番の判定を通ること");

    assert!(
        profiles.names().count() >= 3,
        "見本の組が痩せている（走査が空振りしている疑い）"
    );
    // **gguf を持つ組が実在すること**——無ければ「起こす」側の規則を一度も通らない。
    assert!(
        profiles
            .names()
            .filter_map(|name| profiles.select(name).ok())
            .any(|profile| !profile.launches().is_empty()),
        "見本に起こす面が1つも無い"
    );
}

/// **見本のどの面も、personacore 自身の既定の待ち受けを取りに行かないこと**
/// （作業単位 B-52 / Issue origin#21）。
///
/// # なぜ（利用者の実機 / 2026-09-19）
///
/// `-Port` を渡さない起動は [`DEFAULT_PORT`] で待ち受ける。
/// **見本がその同じポートを名乗っていると、「使ってみる」の既定の道がそのまま壊れる**
/// ——起こした `llama` が 2 秒で `exit code: 1` になる。
///
/// **いまは実行時にも止まる**（`LaunchPlan::port_conflict`）が、
/// **止まるのは正しく案内するためであって、見本が壊れていてよい理由ではない。**
/// **見本を 8080 へ戻す変更は、この腕だけが赤にする。**
#[test]
fn no_sample_profile_launches_on_the_default_bind_port() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(SAMPLE);
    let source = std::fs::read_to_string(&path).expect("見本を読めること");
    let profiles = parse_profiles(&source, Some(&home())).expect("見本が本番の判定を通ること");

    let mut launched = 0usize;
    for name in profiles.names().map(str::to_owned).collect::<Vec<_>>() {
        let profile = profiles.select(&name).expect("在ること");
        for launch in profile.launches() {
            launched += 1;
            assert_ne!(
                launch.launch().serve_port(),
                DEFAULT_PORT,
                "プロファイル `{name}` が personacore 自身の既定の待ち受け {DEFAULT_PORT} を取りに行っている"
            );
        }
    }

    // **空振りではないこと**（起こす面が1つも無ければ、何を書いても緑になる）。
    assert!(
        launched >= 2,
        "見本に起こす面が {launched} 件しかない（この検査が届く形になっていない）"
    );
}
