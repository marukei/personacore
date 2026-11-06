//! **軸の候補の試用の配線**（[ADR-0046](../docs/adr/0046-drift-proposes-new-axes.md) §決定3・4 /
//! 作業単位 A-9c / Issue origin#43）の検証。**本番の入口（`App::bind`）を通す。**
//!
//! 何を採点するか・どう判定するか・固定の軸を変えないことは `server::axis_trial` の単体テストが持つ。
//! ここが見るのは**本番の入口から届くもの**だけ:
//!
//! | 腕 | 走らせ方 | `T` | `K` | 線 | 試用の答え | 往復 | いまの状態 |
//! |---|---|---|---|---|---|---|---|
//! | `T` と `K` を設定 | `shadow` | **2** | **1** | 既定 | `+1` | 2 | **不合格（張り付き）** |
//! | `T` だけ既定 | `shadow` | 既定 | 1 | 既定 | `+1` | 2 | 試用（票 2） |
//! | `K` を設定 | `shadow` | 10 | **7** | 既定 | `+1 -1 0` の繰り返し | 10 | **合格**（非ゼロ 7 ≧ 7・上げ 57%） |
//! | `K` だけ既定 | `shadow` | 10 | 既定 | 既定 | 同上 | 10 | **不合格（標本不足）**（非ゼロ 7 ＜ 既定） |
//! | 線を設定 | `shadow` | 4 | 1 | **70** | `+1 +1 +1 -1` | 4 | **不合格（張り付き）**（上げ 75% ＞ 70%） |
//! | 線だけ既定 | `shadow` | 4 | 1 | 既定 | `+1 +1 +1 -1` | 4 | **合格**（75% ≦ 90%） |
//! | 走らせ方が off | 未設定（既定 off） | 2 | 1 | 既定 | `+1` | 2 | 試用（**票 0・依頼 0**） |
//!
//! **既定の腕と並べないと、「設定を読まずに決め打ちで判定する」実装も設定した腕を満たす**
//! （CLAUDE.md「既定と違う値で回る腕を1本置く」「対比は安い側だけでは足りない」）。
//!
//! 上流は `wiremock`。実機の LM Studio には繋がない。

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use personacore::domain::{
    AxisCandidateRecord, AxisCandidateState, AxisDiscoverySlots, AxisTrialFailure, AxisTrialTally,
    CharacterId, CharacterUpdate, DisplayName, Handle, MemoryContent, NewMemory, Persona, StatAxis,
    Subject,
};
use personacore::server::{
    App, AppConfig, BackgroundProbe, BindAddr, ENV_AXIS_TRIAL_LINE_PERCENT,
    ENV_AXIS_TRIAL_MIN_NONZERO, ENV_AXIS_TRIAL_TURNS, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL,
    ENV_STAT_DRIFT, ENV_SUB_MODEL,
};
use personacore::store::Store;
use serde_json::{Value, json};
use tokio::sync::oneshot;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate, matchers};

use common::fixture_scale;

const HANDLE: &str = "aoi";
const DISPLAY_NAME: &str = "葵";
const MAIN_MODEL: &str = "ministral-3-14b-reasoning-2512";
const SUB_MODEL: &str = "ministral-3-3b-reasoning-2512";

/// 試用中の候補の名前。
const CANDIDATE: &str = "照れ屋";

/// **試用の依頼だけに現れる見出し**（振り分けに使う）。
///
/// **`prompt::axis_trial` の定数を呼ばず、テスト側に期待値として持つ**
/// （呼ぶと綴りを変える変異が両側で同時に動いて緑になる）。
const TRIAL_HEADING: &str = "# 採点する項目";

/// 抽出の依頼だけに現れる見出し。
const EXTRACTION_HEADING: &str = "# 会話";

/// 本文の依頼だけに現れる見出し。
const PERSONA_HEADING: &str = "# ペルソナ";

/// 本文の上流が返す応答。
const REPLY: &str = "「おかえりなさい」 彼女は本を閉じた。";

fn upstream_body(model: &str, content: &str) -> String {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "created": 1,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop",
        }],
    })
    .to_string()
}

/// **試用の依頼に、台本を順に（尽きたら繰り返して）答える。** `delay` があれば遅らせて答える。
struct TrialScript {
    answers: Vec<String>,
    next: AtomicUsize,
    delay: Duration,
}

impl Respond for TrialScript {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let at = self.next.fetch_add(1, Ordering::SeqCst) % self.answers.len();
        ResponseTemplate::new(200)
            .set_body_raw(
                upstream_body(SUB_MODEL, &self.answers[at]),
                "application/json",
            )
            .set_delay(self.delay)
    }
}

struct Harness {
    upstream: MockServer,
    store: Store,
    addr: std::net::SocketAddr,
    character: CharacterId,
    background: BackgroundProbe,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<Result<(), personacore::server::ServerError>>>,
    _dir: tempfile::TempDir,
}

impl Harness {
    /// **本番の入口（`App::bind`）で起動し、試用中の候補を1件置く。** `env` に無い設定は既定。
    async fn start(env: &[(&str, &str)], answers: &[&str], delay: Duration) -> Self {
        let upstream = MockServer::start().await;
        // **試用の依頼を先に据える**（いちばん狭い見出しで振り分ける）。
        Mock::given(matchers::method("POST"))
            .and(matchers::path("/v1/chat/completions"))
            .and(matchers::body_string_contains(TRIAL_HEADING))
            .respond_with(TrialScript {
                answers: answers
                    .iter()
                    .map(|answer| format!("- {CANDIDATE} {answer}"))
                    .collect(),
                next: AtomicUsize::new(0),
                delay,
            })
            .mount(&upstream)
            .await;
        for (heading, model, content) in [
            // 抽出は何も書かない（記憶が増えても候補づくりは枠が埋まっていて走らない）。
            (EXTRACTION_HEADING, SUB_MODEL, ""),
            (PERSONA_HEADING, MAIN_MODEL, REPLY),
        ] {
            Mock::given(matchers::method("POST"))
                .and(matchers::path("/v1/chat/completions"))
                .and(matchers::body_string_contains(heading))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(upstream_body(model, content), "application/json"),
                )
                .mount(&upstream)
                .await;
        }

        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let mut pairs: Vec<(String, String)> = vec![
            (ENV_LLM_ENDPOINT.to_owned(), upstream.uri()),
            (ENV_MAIN_MODEL.to_owned(), MAIN_MODEL.to_owned()),
            (ENV_SUB_MODEL.to_owned(), SUB_MODEL.to_owned()),
        ];
        pairs.extend(
            env.iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
        );
        let config = AppConfig::from_lookup(|key| {
            pairs
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(dir.path().join("personacore.sqlite"));

        // **`bind_with_clients` ではない。** 本番と同じ入口であることが要点。
        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let store = app.store().clone();
        let background = app.background_probe();

        let handle = Handle::parse(HANDLE).expect("正当な識別子");
        let character = store
            .create_character(
                &handle,
                &DisplayName::parse(DISPLAY_NAME).expect("正当な表示名"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナ"),
                None,
                fixture_scale(),
            )
            .await
            .expect("キャラクターを作成できること");
        // **枠は 1**、そこに試用中の候補が1件（空きは無い → 候補づくりは走らない）。
        // 固定の軸は持たない（変容は `no_axes` で飛ぶ → 依頼は本文・抽出・試用だけ）。
        store
            .update_character(
                &handle,
                &CharacterUpdate {
                    axis_discovery_slots: Some(AxisDiscoverySlots::parse(1).expect("正当な枠の数")),
                    ..CharacterUpdate::default()
                },
            )
            .await
            .expect("枠の数を書けること");
        let source = store
            .add_memory(
                character,
                &NewMemory {
                    subject: Subject::Character,
                    content: MemoryContent::parse("人前で褒められると顔が赤くなる")
                        .expect("正当な本文"),
                    category: None,
                },
            )
            .await
            .expect("記憶を足せること");
        store
            .record_axis_candidate(
                character,
                &AxisCandidateRecord::Propose {
                    name: StatAxis::parse(CANDIDATE).expect("正当な軸名"),
                    source_memory: source,
                },
            )
            .await
            .expect("候補を作れること");

        let (shutdown, rx) = oneshot::channel();
        let serving = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });

        Self {
            upstream,
            store,
            addr,
            character,
            background,
            shutdown: Some(shutdown),
            handle: Some(serving),
            _dir: dir,
        }
    }

    /// 1往復投げて**読み切る**。
    async fn chat(&self) {
        let response = reqwest::Client::new()
            .post(format!("http://{}/v1/chat/completions", self.addr))
            .json(&json!({
                "model": HANDLE,
                "messages": [{"role": "user", "content": "ただいま。"}],
            }))
            .send()
            .await
            .expect("サーバへ接続できること");
        assert!(response.status().is_success(), "{:?}", response.status());
        let body = response.text().await.expect("本文を読めること");
        assert!(
            body.contains("おかえりなさい"),
            "前提: 本文が返ること: {body}"
        );
    }

    /// そのターンの裏方が終わるまで待つ。
    ///
    /// **「A が B になる」を待つ形**: 先に抽出の依頼が `extractions` 件目まで届いたこと（＝裏方が走った）を
    /// 押さえ、その後で枠が空くのを待つ。**どちらも上限つき。**
    async fn settle(&self, extractions: usize) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if count(&self.requests().await, EXTRACTION_HEADING) >= extractions {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "抽出の依頼が 30 秒たっても届かない"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        while !self.background.is_idle() {
            assert!(
                Instant::now() < deadline,
                "裏方の枠が 30 秒たっても空かない"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// `turns` 往復を、1往復ずつ裏方が終わるのを待ちながら回す。
    async fn chat_turns(&self, turns: usize) {
        for turn in 1..=turns {
            self.chat().await;
            self.settle(turn).await;
        }
    }

    async fn requests(&self) -> Vec<Value> {
        self.upstream
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|request| request.url.path() == "/v1/chat/completions")
            .map(|request| serde_json::from_slice(&request.body).expect("JSON として読めること"))
            .collect()
    }

    /// 候補の (いまの状態, 票の内訳)。**候補は1件だけ**のはず。
    async fn candidate(&self) -> (AxisCandidateState, AxisTrialTally) {
        let listed = self
            .store
            .axis_candidates(self.character, 10)
            .await
            .expect("候補を読めること");
        assert_eq!(listed.len(), 1, "前提: 候補は1件");
        (listed[0].state(), listed[0].tally())
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).expect("停止シグナルを送れること");
        }
        self.handle
            .take()
            .expect("起動時のタスクがあること")
            .await
            .expect("サーバタスクが panic しないこと")
            .expect("正常に終了すること");
    }
}

fn count(requests: &[Value], heading: &str) -> usize {
    requests
        .iter()
        .filter(|request| request["messages"].to_string().contains(heading))
        .count()
}

fn tally(up: u32, down: u32, zero: u32) -> AxisTrialTally {
    AxisTrialTally { up, down, zero }
}

/// 1つの腕を回して (いまの状態, 票の内訳, 試用の依頼の数) を返す。
async fn run_arm(
    env: &[(&str, &str)],
    answers: &[&str],
    turns: usize,
) -> (AxisCandidateState, AxisTrialTally, usize) {
    let harness = Harness::start(env, answers, Duration::ZERO).await;
    harness.chat_turns(turns).await;
    let (state, votes) = harness.candidate().await;
    let requests = harness.requests().await;
    let trial_requests = count(&requests, TRIAL_HEADING);
    if trial_requests > 0 {
        let trial = requests
            .iter()
            .find(|request| request["messages"].to_string().contains(TRIAL_HEADING))
            .expect("試用の依頼があること");
        assert_eq!(trial["model"], SUB_MODEL, "試用の依頼はサブモデルへ");
    }
    harness.stop().await;
    (state, votes, trial_requests)
}

/// **`PERSONACORE_AXIS_TRIAL_*` の3つが本番の入口から裏方まで届く。off では試用が走らない。**
///
/// 腕の表はモジュールの doc。**期待値は既定ではない値で測り、既定の腕と並べる。**
#[tokio::test]
async fn the_axis_trial_rule_reaches_the_background_through_the_real_bind() {
    // **前提: 既定は 60 / 10 / 90**（既定の腕の期待値がこれに寄りかかる）。
    let default = AppConfig::from_lookup(|_| None)
        .expect("既定で組み立てられること")
        .axis_trial();
    assert!(
        default.turns() > 4,
        "前提: 既定の T は、この台本の往復より多い"
    );
    assert!(
        default.min_nonzero() > 7 && default.min_nonzero() <= 10,
        "前提: 既定の K は 7 より大きく、T = 10 で起動できる"
    );
    assert!(default.line_percent() >= 75, "前提: 既定の線は 75% 以上");

    let shadow = (ENV_STAT_DRIFT, "shadow");

    // --- T と K を設定: 2 票で判定し、上げ 100% は張り付き ---
    let (state, votes, requests) = run_arm(
        &[
            shadow,
            (ENV_AXIS_TRIAL_TURNS, "2"),
            (ENV_AXIS_TRIAL_MIN_NONZERO, "1"),
        ],
        &["+1"],
        2,
    )
    .await;
    assert_eq!(requests, 2, "1往復に1回ずつ採点すること");
    assert_eq!(votes, tally(2, 0, 0));
    assert_eq!(state, AxisCandidateState::Failed(AxisTrialFailure::Pinned));

    // --- T だけ既定: 2 票では判定しない ---
    let (state, votes, _) = run_arm(&[shadow, (ENV_AXIS_TRIAL_MIN_NONZERO, "1")], &["+1"], 2).await;
    assert_eq!(votes, tally(2, 0, 0));
    assert_eq!(
        state,
        AxisCandidateState::Trial,
        "既定の T では 2 票で判定しない"
    );

    // --- K を設定: 10 票のうち非ゼロ 7（上げ 4・下げ 3）は K = 7 に届き、合格 ---
    // **既定の K は T を超えられない**（起動が止まる）ので、この2本の腕は T = 10 で回す。
    let mixed = ["+1", "-1", "0"];
    let (state, votes, _) = run_arm(
        &[
            shadow,
            (ENV_AXIS_TRIAL_TURNS, "10"),
            (ENV_AXIS_TRIAL_MIN_NONZERO, "7"),
        ],
        &mixed,
        10,
    )
    .await;
    assert_eq!(votes, tally(4, 3, 3));
    assert_eq!(state, AxisCandidateState::Passed);

    // --- K だけ既定: 同じ票でも、非ゼロ 7 は既定の K に届かない ---
    let (state, votes, _) = run_arm(&[shadow, (ENV_AXIS_TRIAL_TURNS, "10")], &mixed, 10).await;
    assert_eq!(votes, tally(4, 3, 3));
    assert_eq!(
        state,
        AxisCandidateState::Failed(AxisTrialFailure::TooFewSamples),
        "既定の K では標本不足"
    );

    // --- 線を設定: 上げ 75% は線 70% を超える ---
    let script = ["+1", "+1", "+1", "-1"];
    let (state, votes, _) = run_arm(
        &[
            shadow,
            (ENV_AXIS_TRIAL_TURNS, "4"),
            (ENV_AXIS_TRIAL_MIN_NONZERO, "1"),
            (ENV_AXIS_TRIAL_LINE_PERCENT, "70"),
        ],
        &script,
        4,
    )
    .await;
    assert_eq!(votes, tally(3, 1, 0));
    assert_eq!(state, AxisCandidateState::Failed(AxisTrialFailure::Pinned));

    // --- 線だけ既定: 上げ 75% は既定の線を超えない ---
    let (state, _, _) = run_arm(
        &[
            shadow,
            (ENV_AXIS_TRIAL_TURNS, "4"),
            (ENV_AXIS_TRIAL_MIN_NONZERO, "1"),
        ],
        &script,
        4,
    )
    .await;
    assert_eq!(state, AxisCandidateState::Passed, "既定の線では合格");

    // --- 走らせ方が off（既定）: 試用が走らない ---
    let (state, votes, requests) = run_arm(
        &[
            (ENV_AXIS_TRIAL_TURNS, "2"),
            (ENV_AXIS_TRIAL_MIN_NONZERO, "1"),
        ],
        &["+1"],
        2,
    )
    .await;
    assert_eq!(requests, 0, "off では試用の依頼を出さない");
    assert_eq!(votes, tally(0, 0, 0));
    assert_eq!(state, AxisCandidateState::Trial);
}

/// **本文の応答は試用を待たない**（CLAUDE.md 絶対制約 —— 裏方は本文を返し切った後に走る）。
///
/// 試用の上流は **5 秒遅れて**答える。本文が返り切った時点で試用の票がまだ無いこと
/// （同期で走らせていれば、返った時点で票が入っている）、**その後で試用の依頼が届き、票が入る**ことを見る
/// （試用が走っていないから速い、で緑にならない）。
#[tokio::test]
async fn the_response_never_waits_for_the_trial() {
    let harness = Harness::start(
        &[(ENV_STAT_DRIFT, "shadow")],
        &["0"],
        Duration::from_secs(5),
    )
    .await;

    let started = Instant::now();
    harness.chat().await;
    let answered = started.elapsed();
    let (_, votes) = harness.candidate().await;
    assert_eq!(
        votes,
        tally(0, 0, 0),
        "本文が返った時点では試用の票が無いこと（{answered:?}）"
    );
    assert!(
        answered < Duration::from_secs(5),
        "本文が試用の遅れを待たないこと: {answered:?}"
    );

    // **その後で試用の依頼が届き、票が入る**（上限つき）。
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let requests = harness.requests().await;
        let trial = requests
            .iter()
            .find(|request| request["messages"].to_string().contains(TRIAL_HEADING));
        if let Some(trial) = trial {
            assert!(
                trial["messages"].to_string().contains("おかえりなさい"),
                "試用の依頼には返し切った本文が載ること: {trial}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "試用の依頼が 30 秒たっても届かない"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    harness.settle(1).await;
    assert_eq!(
        harness.candidate().await.1,
        tally(0, 0, 1),
        "開けた後に票が入ること"
    );

    harness.stop().await;
}

/// **読めないつまみは起動を止める**（黙って既定へ落とさない）。本番の入口の設定の読みで見る。
#[test]
fn an_unreadable_axis_trial_rule_stops_the_startup() {
    for (name, raw) in [
        (ENV_AXIS_TRIAL_TURNS, "0"),
        (ENV_AXIS_TRIAL_MIN_NONZERO, "abc"),
        (ENV_AXIS_TRIAL_LINE_PERCENT, "100"),
    ] {
        let result = AppConfig::from_lookup(|key| (key == name).then(|| raw.to_owned()));
        assert!(result.is_err(), "{name}={raw:?} で起動を止めること");
    }
}
