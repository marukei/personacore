//! **記憶抽出を走らせて保存する**（ADR-0021 §6〜10 / 作業単位 13b）の配線の検証。
//!
//! 見ているのは「サーバの外から見て、抽出が本当に走ったか」だけ。
//! 分解そのもの（崩れた行を捨てて数える）は `parse::extraction` の単体テストが、
//! 走らせ方の分岐（忙しい／長すぎる／落とす）は `server::extract` の単体テストが持つ。
//!
//! | 見るもの | 何が壊れたら赤になるか |
//! |---|---|
//! | 非ストリーミング経路で記憶が増える | その経路の配線が落ちる |
//! | 中継経路で記憶が増える | **中継側だけ**の配線が落ちる |
//! | 応答が抽出を待たない | 抽出を同期で走らせる |
//! | 保存に失敗した回は上流を呼ばない | `persist` の成否を見ずに走らせる |
//! | 停止でタスクが落ちる | leak する |
//! | **本番の入口**から**サブモデル**の綴りが上流へ届く | 本文モデルで抽出する／設定が届かない |
//!
//! # 2つの経路を別のモックで見分ける
//!
//! **本文用と抽出用の上流を別のモックにしてある**（`App::bind_with_clients`）。
//! 同じモックにすると「本文の呼び出し」と「抽出の呼び出し」が1つの数に混ざり、
//! **抽出が走っていないのに本文の呼び出しで主張が満たされる。**
//!
//! **経路ごとに違う記憶を返させる。** 非ストリーミングと中継で本文も抽出結果も
//! 別の文字列にしてあるので、**片方の経路の配線だけを落とす変異**がそのまま赤になる
//! （11c-3b の「配線が2本あるのに統合テストが片方しか通っていない」を避ける）。
//!
//! # 実機の LM Studio には繋がない
//!
//! モック注入か、`wiremock` のローカルサーバのどちらか
//! （docs/05-development.md）。**本番の入口（`App::bind`）を通る回だけ**が後者で、
//! そこは注入の口が無い（サブモデルの綴りは上流へ出た HTTP 本文でしか観測できない）。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use personacore::domain::{
    Character, DisplayName, Handle, Memory, MemoryContent, NewMemory, Persona, StatScale, Subject,
};
use personacore::llm::{
    ChatChunk, ChatRequest, ChatStream, Completion, FinishReason, LlmClient, LlmError, LlmFuture,
};
use personacore::server::{
    App, AppConfig, BindAddr, ENV_LLM_ENDPOINT, ENV_MAIN_MODEL, ENV_SUB_MODEL, ServerError,
};
use personacore::store::{RecallScope, Store};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

/// 非ストリーミング経路が返す本文。**中継の本文とは別の文字列。**
const REPLY: &str = "「おかえりなさい」 彼女は微笑んだ。";

/// 中継経路が流すチャンク。**繋ぐと本文になる**（蓄積したものが抽出に渡る）。
const RELAY_CHUNKS: [&str; 2] = ["「いらっしゃい」", " 彼女は本を閉じた。"];

/// 中継経路の本文（蓄積後）。
const RELAYED_REPLY: &str = "「いらっしゃい」 彼女は本を閉じた。";

/// 非ストリーミング経路の抽出結果。**利用者の記憶とキャラクターの記憶を1件ずつ。**
///
/// # 2件の本文を同じにしないこと
///
/// 同じにすると、**主語を取り違える実装**（両方 `利用者` にする / 入れ替える）が
/// そのまま緑を通る（CLAUDE.md「フィクスチャが2つの値を同値にすると…」）。
///
/// **プロンプトの手本（`EXAMPLE_USER_LINE` / `EXAMPLE_CHARACTER_LINE`）とも
/// 別の文字列にしてある。** 手本をそのまま使うと、「依頼の中の例を拾って保存する」
/// 実装と区別が付かない。
const EXTRACTED: &str = "- 利用者: 山あいの町で育った\n- あなた: 雨の日は店を早く閉める";

/// 非ストリーミング経路で入るはずの記憶（利用者について）。
const USER_FACT: &str = "山あいの町で育った";

/// 非ストリーミング経路で入るはずの記憶（キャラクター自身について）。
const CHARACTER_FACT: &str = "雨の日は店を早く閉める";

/// 中継経路の抽出結果。**非ストリーミング経路とは全部別の文字列。**
const RELAY_EXTRACTED: &str = "- 利用者: 弟がふたりいる\n- あなた: 苦い珈琲を淹れるのが得意";

/// 中継経路で入るはずの記憶（利用者について）。
const RELAY_USER_FACT: &str = "弟がふたりいる";

/// 中継経路で入るはずの記憶（キャラクター自身について）。
const RELAY_CHARACTER_FACT: &str = "苦い珈琲を淹れるのが得意";

/// 非ストリーミング経路で投げる利用者の発言。**応答とも中継の発言とも別の文字列。**
const USER_MESSAGE: &str = "ただいま";

/// 中継経路で投げる利用者の発言。**非ストリーミング経路の発言とも本文とも別の文字列。**
const RELAY_USER_MESSAGE: &str = "いま着いた";

/// **会話する**キャラクターの識別子（`model` に入れる値 / ADR-0004）。
const HANDLE: &str = "aoi";

/// **会話する**キャラクターの呼び名。識別子とは別の綴り（ADR-0011）。
const DISPLAY_NAME: &str = "葵";

/// **会話しない**キャラクターの識別子。会話する方とは別の綴り。
///
/// # なぜ2体目が要るのか
///
/// 1体しか作らないと、そのキャラクターの `id` と `1` が**常に同値**になり、
/// **記憶の持ち主を決め打ちにする実装**（`CharacterId::from_row(1)`）が
/// 原理的に観測できない（CLAUDE.md「フィクスチャが2つの値を同値にすると…」）。
/// 実測で、`Extraction::new` の `character_id` を `1` へ決め打つ変異は、
/// 2体目を足すまで **1029/1029 緑**で通り抜けた。
///
/// **この2体目を先に作る**ので、会話する `aoi` の `id` は 1 にならない。
const BYSTANDER_HANDLE: &str = "kaede";

/// **会話しない**キャラクターの呼び名。他の3つとは別の綴り。
const BYSTANDER_DISPLAY_NAME: &str = "楓";

/// **会話しない**キャラクターが最初から持っている記憶。
///
/// 抽出結果のどれとも別の本文にしてある（同値だと、どちらの行なのか区別が付かない）。
/// **0 件にしない**——「持ち主が誰であれ空を返す」実装と区別が付かなくなる。
const BYSTANDER_FACT: &str = "港の見える坂道に住んでいる";

/// 抽出の依頼で**利用者の発言**が載る小見出し。
///
/// **`prompt::extraction` の定数を呼ばず、テスト側に期待値として持つ。**
/// 呼ぶと綴りを変える変異が両側で同時に動いて緑になる
/// （`server::turn::tests::every_outcome_is_recorded_under_its_own_label` と同じ理由）。
/// 綴りが変われば [`assert_each_turn_sits_under_its_own_heading`] が
/// 「小見出しが載ること」で落ちるので、黙って古びることはない。
const USER_TURN_HEADING: &str = "## 利用者の発言";

/// 抽出の依頼で**キャラクターの応答**が載る小見出し。利用者側とは別の綴り。
const CHARACTER_TURN_HEADING: &str = "## あなたの発言";

/// 記憶が現れるのを待つ上限。**超えたら落とす**（緑のまま何も測らない形にしない）。
const WAIT_LIMIT: Duration = Duration::from_secs(20);

/// 待ちの刻み。
const POLL_STEP: Duration = Duration::from_millis(10);

/// 「起きていないこと」を主張する前に置く猶予。
///
/// **肯定側（起きること）は同じ仕掛けで数十ミリ秒以内に観測できている**ので、
/// これだけ待って起きなければ「起きない」と言ってよい。
///
/// # **肯定側の主張にこれを使わないこと**（13b のレビュー指摘）
///
/// 否定側は遅れても偽陽性にならない（待ちが長い分だけ安全側）が、
/// 肯定側は**負荷が高いと単純に足りなくなる**——「500ms 以内に走り出す」は
/// 実装ではなく機械の速さについての主張になる。
/// 肯定側は [`wait_for_calls`] / [`TestServer::wait_for_memories`] のように
/// **締切まで poll する**形にすること。
const SETTLE: Duration = Duration::from_millis(500);

/// 本文生成の上流の代役。**ネットワークには一切触らない。**
struct MainLlm {
    /// 中継で流すチャンク列。**1回だけ取り出せる。**
    source: Mutex<Option<ChatStream>>,
    /// `complete` が返す本文。
    reply: String,
    /// **応答を返す直前に永続化層を壊す**（保存だけを失敗させる細工）。
    sabotage: Mutex<Option<Store>>,
    /// 呼ばれた回数（`complete` と `stream` の合計）。
    calls: AtomicUsize,
    /// 受け取った本文の依頼（`complete` と `stream` の両方）。**中身まで見る**（A-11a）。
    requests: Mutex<Vec<ChatRequest>>,
}

impl MainLlm {
    fn new() -> Arc<Self> {
        Self::replying(REPLY)
    }

    /// **非ストリーミングで返す本文を差し替えたモック**（作業単位 B-61）。中継のチャンクは [`Self::new`] と同じ。
    fn replying(reply: &str) -> Arc<Self> {
        let chunks: Vec<Result<ChatChunk, LlmError>> = RELAY_CHUNKS
            .iter()
            .enumerate()
            .map(|(index, delta)| {
                Ok(ChatChunk {
                    delta: (*delta).to_owned(),
                    finish_reason: (index + 1 == RELAY_CHUNKS.len()).then_some(FinishReason::Stop),
                    usage: None,
                })
            })
            .collect();

        Arc::new(Self {
            source: Mutex::new(Some(Box::pin(futures_util::stream::iter(chunks)))),
            reply: reply.to_owned(),
            sabotage: Mutex::new(None),
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        })
    }

    /// 最後に受け取った本文の依頼を1つに繋いだもの。
    fn last_prompt(&self) -> String {
        let requests = self.requests.lock().expect("ロックを取れること");
        let last = requests.last().expect("本文の依頼を1件以上受けていること");
        last.messages
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// **次の `complete` で永続化層を閉じる**（保存だけが失敗する状況を作る）。
    fn break_the_store_on_the_next_call(&self, store: Store) {
        *self.sabotage.lock().expect("ロックを取れること") = Some(store);
    }
}

impl LlmClient for MainLlm {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        self.requests
            .lock()
            .expect("ロックを取れること")
            .push(request.clone());
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            // **ロックを握ったまま `await` しない。**
            let sabotage = self.sabotage.lock().expect("ロックを取れること").take();
            if let Some(store) = sabotage {
                store.close().await;
            }
            Ok(Completion {
                content: self.reply.clone(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        self.requests
            .lock()
            .expect("ロックを取れること")
            .push(request.clone());
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.source
                .lock()
                .expect("ロックを取れること")
                .take()
                .ok_or(LlmError::MissingContent)
        })
    }
}

/// **抽出の未来が落とされたことを知らせる印。**
///
/// 停止でタスクが `abort` されると、走っている `complete` の future ごと落ちる。
/// 「落ちた」は外から直接見えないので、future の中に置いたこの番人の `Drop` で観測する。
struct DropSignal(Arc<AtomicUsize>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// 抽出（裏方）の上流の代役。**本文用とは別のモック。**
struct SubLlm {
    /// 受け取った依頼。**中身まで見る**（抽出の依頼であることの確認）。
    requests: Mutex<Vec<ChatRequest>>,
    /// 返す本文。**取り出すたびに先頭から**（経路ごとに別の記憶を返す）。
    replies: Mutex<Vec<String>>,
    /// `Some` なら**開くまで返さない**（応答が抽出を待たないことの確認に使う）。
    gate: Mutex<Option<oneshot::Receiver<()>>>,
    /// 上流の失敗を演じるか。
    fails: bool,
    /// 呼ばれた回数。
    calls: AtomicUsize,
    /// **future が落とされた回数**（停止で `abort` されたことの観測）。
    dropped: Arc<AtomicUsize>,
}

impl SubLlm {
    /// 順に返す本文を持つモック。
    fn replying(replies: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
            replies: Mutex::new(
                replies
                    .iter()
                    .rev()
                    .map(|text| (*text).to_owned())
                    .collect(),
            ),
            gate: Mutex::new(None),
            fails: false,
            calls: AtomicUsize::new(0),
            dropped: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// **上流が必ず失敗するモック。**
    fn failing() -> Arc<Self> {
        Arc::new(Self {
            requests: Mutex::new(Vec::new()),
            replies: Mutex::new(Vec::new()),
            gate: Mutex::new(None),
            fails: true,
            calls: AtomicUsize::new(0),
            dropped: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// **開けるまで応答を返さないモック**（返す本文は開いた後に流れる）。
    fn gated(replies: &[&str]) -> (Arc<Self>, oneshot::Sender<()>) {
        let (open, gate) = oneshot::channel();
        let mock = Arc::new(Self {
            requests: Mutex::new(Vec::new()),
            replies: Mutex::new(
                replies
                    .iter()
                    .rev()
                    .map(|text| (*text).to_owned())
                    .collect(),
            ),
            gate: Mutex::new(Some(gate)),
            fails: false,
            calls: AtomicUsize::new(0),
            dropped: Arc::new(AtomicUsize::new(0)),
        });
        (mock, open)
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn dropped_count(&self) -> usize {
        self.dropped.load(Ordering::SeqCst)
    }

    /// 最後に受け取った依頼の本文を1つに繋いだもの。
    fn last_prompt(&self) -> String {
        let requests = self.requests.lock().expect("ロックを取れること");
        let last = requests.last().expect("依頼を1件以上受けていること");
        last.messages
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl LlmClient for SubLlm {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        // **約束・決まりの提案の依頼（作業単位 A-12c-1）は、抽出の上流の代役として受けない。**
        // 提案は同じ裏方の列のいちばん後ろで毎往復走るので、ここで数えたり台本を消費したりすると、
        // 抽出の回数・最後の依頼・次の往復の抽出の答えが提案にずれる。提案の配線は
        // `tests/server_agreement_proposals.rs` が見る。**空の答えを返し、何も控えない。**
        if is_agreement_proposal(request) {
            return Box::pin(async move {
                Ok(Completion {
                    content: String::new(),
                    finish_reason: Some(FinishReason::Stop),
                    usage: None,
                })
            });
        }
        self.requests
            .lock()
            .expect("ロックを取れること")
            .push(request.clone());
        self.calls.fetch_add(1, Ordering::SeqCst);

        // **落ちたことを観測するための番人。** future ごと落ちれば `Drop` が走る。
        let signal = DropSignal(Arc::clone(&self.dropped));
        let gate = self.gate.lock().expect("ロックを取れること").take();

        Box::pin(async move {
            // **番人を future の中で持つ**（`abort` で落ちるのはここ）。
            let _signal = signal;
            if let Some(gate) = gate {
                let _ = gate.await;
            }
            if self.fails {
                return Err(LlmError::MissingContent);
            }
            let content = self
                .replies
                .lock()
                .expect("ロックを取れること")
                .pop()
                .unwrap_or_default();
            Ok(Completion {
                content,
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            })
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        // **抽出は逐次表示を必要としない**（`LlmClient::complete` の doc）。
        Box::pin(async move { Err(LlmError::MissingContent) })
    }
}

/// **約束・決まりの提案の依頼の綴り**（作業単位 A-12c-1。検出 `jd-v3` と確かめ `c1-v1` の書き方の行）。
///
/// **`prompt` の定数を呼ばず、期待値として持つ**（`tests/server_agreement_proposals.rs` と同じ綴り）。
const AGREEMENT_PROPOSAL_MARKERS: [&str; 2] = [
    "抜｜引用｜取り決め｜守る側｜中身",
    "点検｜番号｜行い｜分類｜守る側｜判定｜中身",
];

/// 依頼が約束・決まりの提案のものか（[`AGREEMENT_PROPOSAL_MARKERS`]）。
fn is_agreement_proposal(request: &ChatRequest) -> bool {
    request.messages.iter().any(|message| {
        AGREEMENT_PROPOSAL_MARKERS
            .iter()
            .any(|marker| message.content.contains(marker))
    })
}

/// 上流へ届いた本文（JSON）が約束・決まりの提案の依頼か（`wiremock` の側）。
fn is_agreement_proposal_body(request: &Value) -> bool {
    let messages = request["messages"].to_string();
    AGREEMENT_PROPOSAL_MARKERS
        .iter()
        .any(|marker| messages.contains(marker))
}

/// 抽出の上流が `count` 回呼ばれるまで待つ。**ならなければ落とす。**
///
/// **肯定側（走り出すこと）を固定の `sleep` で待たない**（[`SETTLE`] の doc）。
/// 締切まで poll する形なら、遅い機械でも余計に待つだけで済む。
async fn wait_for_calls(sub: &SubLlm, count: usize) {
    let deadline = Instant::now() + WAIT_LIMIT;
    while sub.call_count() < count {
        assert!(
            Instant::now() < deadline,
            "抽出の上流が {count} 回呼ばれること: {}",
            sub.call_count()
        );
        tokio::time::sleep(POLL_STEP).await;
    }
}

/// キャラクターを1件作って読み直す。
async fn create_character(store: &Store, handle: &str, display_name: &str) -> Character {
    let handle = Handle::parse(handle).expect("正当な名前であること");
    store
        .create_character(
            &handle,
            &DisplayName::parse(display_name).expect("正当な表示名であること"),
            &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
            None,
            StatScale::parse(7).expect("正当な尺度であること"),
        )
        .await
        .expect("キャラクターを作成できること");
    store
        .find_character_by_handle(&handle)
        .await
        .expect("キャラクターを引けること")
        .expect("作成済みであること")
}

/// エフェメラルポートで起動した実サーバ。**上流は2つともモック。**
struct TestServer {
    addr: SocketAddr,
    store: Store,
    client: reqwest::Client,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    async fn start(main: Arc<MainLlm>, sub: Arc<SubLlm>) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db_path = dir.path().join("personacore.sqlite");

        let config = AppConfig::from_lookup(|_| None)
            .expect("設定を組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(&db_path);

        // **本文と抽出で別のモックを注入する。** 1つにすると呼び出しが混ざる。
        let app = App::bind_with_clients(&config, main, sub)
            .await
            .expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let store = app.store().clone();

        // **会話しない方を先に作る**（＝ `id` が 1 になるのはこちら / `BYSTANDER_HANDLE`）。
        let bystander = create_character(&store, BYSTANDER_HANDLE, BYSTANDER_DISPLAY_NAME).await;
        let character = create_character(&store, HANDLE, DISPLAY_NAME).await;

        assert_ne!(character.id, bystander.id, "前提: 2体の id が別であること");
        assert_ne!(
            character.handle.as_str(),
            bystander.handle.as_str(),
            "前提: 2体の識別子が別であること"
        );
        assert_ne!(
            character.display_name.as_str(),
            bystander.display_name.as_str(),
            "前提: 2体の呼び名が別であること"
        );

        // **傍観者に1件だけ入れておく**（増減で取り違えが読める / `BYSTANDER_FACT`）。
        store
            .add_memory(
                bystander.id,
                &NewMemory {
                    subject: Subject::Character,
                    content: MemoryContent::parse(BYSTANDER_FACT).expect("正当な本文であること"),
                    category: None,
                },
            )
            .await
            .expect("傍観者の記憶を入れられること");

        let (shutdown, rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            app.serve(async move {
                let _ = rx.await;
            })
            .await
        });

        Self {
            addr,
            store,
            client: reqwest::Client::new(),
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    /// 応答を最後まで読み切る（**読み切らないと保存も抽出も走らない**）。
    async fn chat(&self, body: &Value) -> String {
        let response = self
            .client
            .post(format!("http://{}/v1/chat/completions", self.addr))
            .json(body)
            .send()
            .await
            .expect("サーバへ接続できること");
        assert!(response.status().is_success(), "{:?}", response.status());
        response.text().await.expect("本文を読めること")
    }

    async fn character(&self) -> Character {
        self.character_named(HANDLE).await
    }

    /// **会話しないキャラクター。** ここの記憶が増えたら持ち主の取り違え。
    async fn bystander(&self) -> Character {
        self.character_named(BYSTANDER_HANDLE).await
    }

    async fn character_named(&self, handle: &str) -> Character {
        let handle = Handle::parse(handle).expect("正当な名前であること");
        self.store
            .find_character_by_handle(&handle)
            .await
            .expect("キャラクターを引けること")
            .expect("作成済みであること")
    }

    /// 保存された記憶を作成順で読む。
    async fn memories(&self) -> Vec<Memory> {
        let character = self.character().await;
        self.store
            .recall_memories(character.id, RecallScope::Everything)
            .await
            .expect("記憶を読めること")
    }

    /// **会話しないキャラクター**の記憶を作成順で読む。
    async fn bystander_memories(&self) -> Vec<Memory> {
        let bystander = self.bystander().await;
        self.store
            .recall_memories(bystander.id, RecallScope::Everything)
            .await
            .expect("傍観者の記憶を読めること")
    }

    /// 記憶が `count` 件になるまで待つ。**ならなければ落とす。**
    async fn wait_for_memories(&self, count: usize) -> Vec<Memory> {
        let deadline = Instant::now() + WAIT_LIMIT;
        loop {
            let memories = self.memories().await;
            if memories.len() >= count {
                return memories;
            }
            assert!(
                Instant::now() < deadline,
                "記憶が {count} 件になること: {memories:?}"
            );
            tokio::time::sleep(POLL_STEP).await;
        }
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

/// 非ストリーミングの1往復を投げる本文。
fn non_streaming_body(message: &str) -> Value {
    json!({
        "model": HANDLE,
        "messages": [{"role": "user", "content": message}],
    })
}

/// 中継の1往復を投げる本文。
fn streaming_body(message: &str) -> Value {
    json!({
        "model": HANDLE,
        "messages": [{"role": "user", "content": message}],
        "stream": true,
    })
}

/// 依頼の中での位置。**見つからなければ落とす**（`Option` の比較で黙って通さない）。
fn position_of(prompt: &str, needle: &str) -> usize {
    prompt
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} が依頼に載ること: {prompt:?}"))
}

/// **2つの発言が、それぞれ自分の小見出しの下に載っていること。**
///
/// # `contains` では取り違えを捕まえられない
///
/// 入れ替えても**どちらの文字列も依頼の「どこかには」在る**ので、
/// 「載っていること」しか見ない主張は両方とも満たされる。実測で、
/// `TurnSink::persist` が `Extraction::start` に渡す2つの引数を入れ替える変異は
/// **1029/1029 緑**で通り抜けた。だから**位置**（小見出しとの前後関係）で見る。
///
/// # 取り違えると何が起きるか
///
/// サブモデルには**役割が反転した会話**が渡り、抽出される事実の主語が系統的に反転する
/// （ADR-0021 §2 / CLAUDE.md「`memory.subject` を省略しない ——
/// `user` / `character` の区別を落とすと一人称が混線する」）。
///
/// > **行は消えないが、直せる**（20b で判定をやり直した）——
/// > 主語を正した訂正版を入れれば誤った行は想起から外れる（ADR-0022）。
/// > **ただし直すのは人で、しかも1件ずつ。**系統的に反転していれば全件が対象になる。
fn assert_each_turn_sits_under_its_own_heading(prompt: &str, user_message: &str, reply: &str) {
    assert_ne!(
        user_message, reply,
        "前提: 2つの発言は別の文字列であること（同値だと取り違えが観測できない）"
    );

    let user_heading = position_of(prompt, USER_TURN_HEADING);
    let character_heading = position_of(prompt, CHARACTER_TURN_HEADING);
    let user_at = position_of(prompt, user_message);
    let reply_at = position_of(prompt, reply);

    assert!(
        user_heading < user_at && user_at < character_heading,
        "利用者の発言は `{USER_TURN_HEADING}` の下に載ること: {prompt:?}"
    );
    assert!(
        character_heading < reply_at,
        "応答は `{CHARACTER_TURN_HEADING}` の下に載ること: {prompt:?}"
    );
}

/// 記憶を `(主語, 本文)` の組にして並べる。
fn as_pairs(memories: &[Memory]) -> Vec<(Subject, String)> {
    memories
        .iter()
        .map(|memory| (memory.subject, memory.content.as_str().to_owned()))
        .collect()
}

/// **非ストリーミング経路で抽出が走り、主語つきで保存される。**
///
/// 主語は `利用者` と `あなた` の**両方**を含む出力で見る（片方だけだと
/// 「常に `User` を入れる」実装が緑を通る）。
#[tokio::test]
async fn the_non_streaming_route_extracts_and_saves_memories() {
    let sub = SubLlm::replying(&[EXTRACTED]);
    let server = TestServer::start(MainLlm::new(), Arc::clone(&sub)).await;

    let body = server.chat(&non_streaming_body(USER_MESSAGE)).await;
    assert!(body.contains(REPLY), "前提: 本文が返ること: {body}");

    let memories = server.wait_for_memories(2).await;

    assert_eq!(
        as_pairs(&memories),
        vec![
            (Subject::User, USER_FACT.to_owned()),
            (Subject::Character, CHARACTER_FACT.to_owned()),
        ],
        "主語と本文が対応したまま保存されること"
    );

    // **会話しなかった方は1件も増えないこと**（`BYSTANDER_HANDLE` の doc）。
    assert_eq!(
        as_pairs(&server.bystander_memories().await),
        vec![(Subject::Character, BYSTANDER_FACT.to_owned())],
        "会話していないキャラクターに記憶を入れないこと（持ち主を決め打ちにしない）"
    );

    // **抽出の依頼であることを確かめる**（本文生成の依頼を数えていない）。
    // **役割が正しい側の小見出しに載っていること**まで見る——
    // 「載っていること」だけでは2つの取り違えが緑を通る（ヘルパの doc）。
    let prompt = sub.last_prompt();
    assert_each_turn_sits_under_its_own_heading(&prompt, USER_MESSAGE, REPLY);
    assert_eq!(sub.call_count(), 1, "抽出は1往復につき1回だけであること");

    server.stop().await;
}

/// **中継経路でも抽出が走る。**
///
/// 非ストリーミングとは**別の記憶**が入るので、片方の配線だけを落とす変異が
/// このテストか上のテストのどちらかを名指しで赤にする。
#[tokio::test]
async fn the_relay_route_extracts_and_saves_memories() {
    let sub = SubLlm::replying(&[RELAY_EXTRACTED]);
    let server = TestServer::start(MainLlm::new(), Arc::clone(&sub)).await;

    let body = server.chat(&streaming_body(RELAY_USER_MESSAGE)).await;
    assert!(body.contains("[DONE]"), "前提: 中継が完結すること: {body}");

    let memories = server.wait_for_memories(2).await;

    assert_eq!(
        as_pairs(&memories),
        vec![
            (Subject::User, RELAY_USER_FACT.to_owned()),
            (Subject::Character, RELAY_CHARACTER_FACT.to_owned()),
        ],
        "中継経路でも主語つきで保存されること"
    );

    // **会話しなかった方は1件も増えないこと**（`BYSTANDER_HANDLE` の doc）。
    assert_eq!(
        as_pairs(&server.bystander_memories().await),
        vec![(Subject::Character, BYSTANDER_FACT.to_owned())],
        "会話していないキャラクターに記憶を入れないこと（持ち主を決め打ちにしない）"
    );

    // **蓄積した本文**が、**キャラクター側の小見出しの下に**載る
    // （チャンク1件だけではない／利用者の発言と取り違えていない）。
    let prompt = sub.last_prompt();
    assert_each_turn_sits_under_its_own_heading(&prompt, RELAY_USER_MESSAGE, RELAYED_REPLY);

    server.stop().await;
}

/// 思考の中身。**抽出の依頼に1文字も載ってはいけない**（作業単位 B-61）。
const LEAKED_THOUGHT: &str = "The user just came home.";

/// **上流が本文に思考を漏らした応答**（C-20 の形。閉じタグは空白入り）。後ろの本文は [`REPLY`]。
const THINKING_REPLY: &str =
    "<think>\nThe user just came home.\n</think >\n\n「おかえりなさい」 彼女は微笑んだ。";

/// **裏方のサブLLMへは、思考を剥がした応答が渡る**（ADR-0053 §決定4 / 作業単位 B-61）。
///
/// 抽出・変容・試用・約束の提案は、**全部 `TurnJob::character_message` の1つの値を読む**
/// （剥がすのは `Extraction::start`）。ここで見るのはその代表の抽出の依頼で、
/// **剥がした本文がキャラクター側の小見出しの下に載り、思考は載らない**ことを同じ依頼で見る。
#[tokio::test]
async fn the_extraction_request_reads_the_reply_without_the_leaked_thought() {
    assert!(
        THINKING_REPLY.ends_with(REPLY) && THINKING_REPLY.contains(LEAKED_THOUGHT),
        "前提: 思考の後ろの本文は REPLY と同じで、思考を含むこと"
    );

    let sub = SubLlm::replying(&[EXTRACTED]);
    let server = TestServer::start(MainLlm::replying(THINKING_REPLY), Arc::clone(&sub)).await;

    let body = server.chat(&non_streaming_body(USER_MESSAGE)).await;
    // **API の使い手に返す本文はこの単位では剥がさない**（ADR-0053 §決定6）。
    assert!(
        body.contains("The user just came home."),
        "前提: 非ストリーミングの応答本文は上流のまま返ること: {body}"
    );

    wait_for_calls(&sub, 1).await;
    let prompt = sub.last_prompt();
    assert_each_turn_sits_under_its_own_heading(&prompt, USER_MESSAGE, REPLY);
    assert!(
        !prompt.contains(LEAKED_THOUGHT),
        "抽出の依頼に思考を載せないこと: {prompt:?}"
    );
    assert!(
        !prompt.contains("think"),
        "タグの綴りも載せないこと: {prompt:?}"
    );

    server.stop().await;
}

/// 暴走の単位（8 文字）。**`、` で終える** —— [`REPLY`] の末尾の `。` と周期が合わないように
/// （合うと区間が1文字前へ伸び、残る単位が回転する / 作業単位 A-21a）。
const RUNAWAY_UNIT: &str = "ずっと一緒だよ、";

/// **[`REPLY`] の後ろに、思考を挟んで2つに割れた暴走が続く応答**（ADR-0062 / 作業単位 A-21a）。
///
/// 暴走の単位 5 回（40 文字）が思考の前後に1つずつ。**剥がした後なら 80 文字の1つの区間**として畳まれ、
/// **剥がす前に畳むと、どちらの半分も 64 文字に届かず畳まれない** —— 1つのフィクスチャで
/// 「畳むこと」と「剥がした後に畳むこと」を全部の読み手で見る。
fn runaway_reply() -> String {
    let half = RUNAWAY_UNIT.repeat(5);
    format!("{REPLY}{half}<think>\n{LEAKED_THOUGHT}\n</think >{half}")
}

/// [`runaway_reply`] を剥がして畳んだ後。**手で書いた値**（実装から作らない）。
const FOLDED_RUNAWAY_REPLY: &str = "「おかえりなさい」 彼女は微笑んだ。ずっと一緒だよ、";

/// **暴走を含む生の出力は、分解・裏方・次のターンの履歴へは畳まれて届き、保存の `raw_content` は畳まれない**
/// （ADR-0062 §決定2・§決定7 / 作業単位 A-21a）。
///
/// 3つの読み手（`server::turn` の分解・`Extraction::start` の `character_message`・`prompt::history_turn`）と
/// 保存を、**同じ1つの応答**で見る。単体テストは各読み手を直に呼ぶので、**本番の配線
/// （`TurnSink::persist` が畳む前の生出力を保存と抽出に渡しているか、`chat` が保存した行から履歴を組むか）**
/// はここでしか見えない。
#[tokio::test]
async fn a_runaway_reaches_every_reader_folded_and_is_saved_whole() {
    let raw = runaway_reply();
    // 前提: 畳んだ形は生の出力の頭と同じなので、「畳んだ形が載る」だけでは畳んだことにならない。
    // **単位が2回続く綴りが載らないこと**を一緒に見る（生の出力には在る）。
    assert!(
        raw.starts_with(FOLDED_RUNAWAY_REPLY) && raw.contains(&RUNAWAY_UNIT.repeat(2)),
        "前提: 生の出力は単位を2回以上続けて持つこと"
    );

    let main = MainLlm::replying(&raw);
    let sub = SubLlm::replying(&[EXTRACTED]);
    let server = TestServer::start(Arc::clone(&main), Arc::clone(&sub)).await;

    let body = server.chat(&non_streaming_body(USER_MESSAGE)).await;
    // **API の使い手に返す本文は畳まない**（ADR-0062 §決定2）。
    let value: Value = serde_json::from_str(&body).expect("JSON として読めること");
    assert_eq!(
        value["choices"][0]["message"]["content"].as_str(),
        Some(raw.as_str()),
        "前提: 非ストリーミングの応答本文は上流のまま返ること"
    );

    // 1. 裏方（記憶抽出）には、剥がして畳んだ本文がキャラクター側の小見出しの下に載る。
    wait_for_calls(&sub, 1).await;
    let extraction = sub.last_prompt();
    assert_each_turn_sits_under_its_own_heading(&extraction, USER_MESSAGE, FOLDED_RUNAWAY_REPLY);
    assert!(
        !extraction.contains(&RUNAWAY_UNIT.repeat(2)),
        "抽出の依頼に暴走を載せないこと: {extraction:?}"
    );
    assert!(
        !extraction.contains(LEAKED_THOUGHT),
        "抽出の依頼に思考を載せないこと: {extraction:?}"
    );

    // 2. 保存: `raw_content` は生のまま、分解した本文は畳んだ後。
    let model = value["model"].as_str().expect("model は文字列であること");
    let (_, raw_session) = model.split_once('@').expect("`名前@セッションid` の形");
    let raw_session: i64 = raw_session.parse().expect("セッション id は整数");
    let character = server.character().await;
    let session = server
        .store
        .find_session(character.id, raw_session)
        .await
        .expect("セッションを引けること")
        .expect("セッションが存在すること");
    let saved = server
        .store
        .recent_messages(session.id, 10)
        .await
        .expect("履歴を読めること");
    assert_eq!(saved.len(), 2, "往復2件が入ること: {saved:?}");
    assert_eq!(
        saved[1].raw_content, raw,
        "`raw_content` を畳まないこと（絶対制約）"
    );
    assert_eq!(saved[1].dialogue.as_deref(), Some("おかえりなさい"));
    assert_eq!(
        saved[1].narration.as_deref(),
        Some(" 彼女は微笑んだ。ずっと一緒だよ、"),
        "分解した本文は剥がして畳んだ後であること"
    );

    // 3. 次のターンの履歴には、保存した生の出力から畳み直した本文が載る。
    server.chat(&non_streaming_body("もう一度")).await;
    let persona = main.last_prompt();
    assert!(
        persona.contains(FOLDED_RUNAWAY_REPLY),
        "履歴に畳んだ応答が載ること: {persona:?}"
    );
    assert!(
        !persona.contains(&RUNAWAY_UNIT.repeat(2)),
        "履歴に暴走を載せないこと: {persona:?}"
    );

    server.stop().await;
}

/// **保存済みのキャラクター自身の記憶は、本文の依頼に載らず、抽出の依頼には載る**
/// （ADR-0050 §作る前の測定と決め直し 1 / A-11a）。
///
/// # 統合テストで見る理由
///
/// 純粋関数の側（`prompt` の
/// `a_characters_own_memory_is_hidden_from_the_persona_prompt_but_shown_to_the_background_requests`）は
/// **同じ列を両方の組立に通す**ので、**絞りを `store` の読み出し
/// （`Store::recall_memories`）へ移す変異**を観測できない。そこへ移すと本文も抽出も
/// 同じ入口から読むので、**抽出の依頼からもキャラクター自身の記憶が消える**
/// （言い直しの重複を止める控除リストが利用者の側だけになる）。ここでそれを止める。
#[tokio::test]
async fn a_stored_own_memory_stays_out_of_the_persona_prompt_but_reaches_the_extraction_request() {
    const STORED_USER_FACT: &str = "猫を二匹飼っている";
    const STORED_OWN_FACT: &str = "紅茶を淹れるのが得意";

    let main = MainLlm::new();
    let sub = SubLlm::replying(&[EXTRACTED]);
    let server = TestServer::start(Arc::clone(&main), Arc::clone(&sub)).await;

    let character = server.character().await;
    for (subject, content) in [
        (Subject::Character, STORED_OWN_FACT),
        (Subject::User, STORED_USER_FACT),
    ] {
        server
            .store
            .add_memory(
                character.id,
                &NewMemory {
                    subject,
                    content: MemoryContent::parse(content).expect("正当な本文であること"),
                    category: None,
                },
            )
            .await
            .expect("記憶を入れられること");
    }

    let body = server.chat(&non_streaming_body(USER_MESSAGE)).await;
    assert!(body.contains(REPLY), "前提: 本文が返ること: {body}");
    wait_for_calls(&sub, 1).await;

    let persona = main.last_prompt();
    assert!(
        !persona.contains(STORED_OWN_FACT),
        "キャラクター自身の記憶が本文の依頼に載った: {persona}"
    );
    assert!(
        persona.contains(STORED_USER_FACT),
        "利用者の記憶は本文の依頼に載ること: {persona}"
    );

    let extraction = sub.last_prompt();
    for fact in [STORED_USER_FACT, STORED_OWN_FACT] {
        assert!(
            extraction.contains(fact),
            "抽出の依頼の既知の記憶から落ちた: {fact}\n{extraction}"
        );
    }

    server.stop().await;
}

/// **抽出が書いた記憶は、次の区切りまで本文の依頼に載らないが、次の抽出の依頼には載る**
/// （ADR-0064 §決定1・§決定3 —— 絞るのは本文の想起だけ）。
///
/// 1ターン目の抽出が `USER_FACT` を書く（本番の書き手 `Store::add_memory_in_session`）。
/// 2ターン目は往復の数 1 で区切りが無く、境の時刻はセッションの作成時刻 —— 抽出の記憶はそれより新しい。
///
/// * 本文の依頼に `USER_FACT` が載らない（境の時刻を渡さない変異で赤）。
/// * 2ターン目の抽出の依頼（既知の記憶）には載る（裏方まで絞る変異で赤）。
///
/// **時刻の同着で揺れない形にしてある。** 上流はモックなので、セッションを作ってから抽出の INSERT までに
/// 待ちが無く、同じミリ秒に入りうる（境は `<=` なので、同着だと記憶が境の内側に入る）。
/// **記憶の行（本番の書き手が書いたもの）には触らず**、境になるセッションの `created_at` だけを
/// 過去へ書き換え、記憶が境より新しいことを前提として確かめてから主張する。
#[tokio::test]
async fn an_extracted_memory_waits_for_the_boundary_in_the_persona_prompt_but_not_in_the_extraction_request()
 {
    const SECOND_MESSAGE: &str = "今日は雨だったね";
    const SESSION_CREATED_AT: &str = "2000-01-01T00:00:00.000Z";

    let main = MainLlm::new();
    let sub = SubLlm::replying(&[EXTRACTED, ""]);
    let server = TestServer::start(Arc::clone(&main), Arc::clone(&sub)).await;

    server.chat(&non_streaming_body(USER_MESSAGE)).await;
    let memories = server.wait_for_memories(2).await;
    let extracted = memories
        .iter()
        .find(|memory| memory.subject == Subject::User && memory.content.as_str() == USER_FACT)
        .unwrap_or_else(|| panic!("前提: 1ターン目の抽出が利用者の記憶を書いたこと: {memories:?}"));

    // 境（往復の数 1 ＝ 区切りが無い → セッションの作成時刻）を過去へ動かす（テストだけの細工）。
    let character = server.character().await;
    let session = server
        .store
        .latest_session(character.id)
        .await
        .expect("会話を引けること")
        .expect("1ターン目の会話があること");
    let done = sqlx::query("UPDATE session SET created_at = ? WHERE id = ?")
        .bind(SESSION_CREATED_AT)
        .bind(session.id.get())
        .execute(server.store.pool())
        .await
        .expect("時刻を書き換えられること");
    assert_eq!(done.rows_affected(), 1, "1行だけ書き換えること");
    let boundary = server
        .store
        .recall_boundary(session.id)
        .await
        .expect("境の時刻を求められること");
    assert_eq!(
        boundary.as_stored(),
        SESSION_CREATED_AT,
        "前提: 境はセッションの作成時刻"
    );
    let extracted_at: String = sqlx::query_scalar("SELECT created_at FROM memory WHERE id = ?")
        .bind(extracted.id.get())
        .fetch_one(server.store.pool())
        .await
        .expect("記憶の時刻を読めること");
    assert!(
        extracted_at.as_str() > boundary.as_stored(),
        "前提の違反: 抽出の記憶（{extracted_at}）が境（{}）より新しくない",
        boundary.as_stored()
    );

    server.chat(&non_streaming_body(SECOND_MESSAGE)).await;
    let persona = main.last_prompt();
    assert!(
        persona.contains(SECOND_MESSAGE),
        "前提: 2ターン目の本文の依頼を見ていること: {persona}"
    );
    assert!(
        !persona.contains(USER_FACT),
        "抽出で増えた記憶は、次の区切りまで本文の依頼に載らないこと: {persona}"
    );

    wait_for_calls(&sub, 2).await;
    let extraction = sub.last_prompt();
    assert!(
        extraction.contains(SECOND_MESSAGE),
        "前提: 2ターン目の抽出の依頼を見ていること: {extraction}"
    );
    assert!(
        extraction.contains(USER_FACT),
        "抽出の依頼の既知の記憶には、境より新しい記憶も載ること: {extraction}"
    );

    server.stop().await;
}

/// **応答は抽出を待たない**（CLAUDE.md 絶対制約 / ADR-0021 §6）。
///
/// 抽出の上流は**開くまで返らない。** それでも本文の応答は返り切る。
/// 同期で走らせる変異を入れると、ここで**応答が返らずタイムアウトする。**
///
/// **開いた後に記憶が入ることまで見る。** 見ないと「抽出が走っていないから速い」
/// でも緑になり、主張が空になる。
#[tokio::test]
async fn the_response_never_waits_for_the_extraction() {
    let (sub, open) = SubLlm::gated(&[EXTRACTED]);
    let server = TestServer::start(MainLlm::new(), Arc::clone(&sub)).await;

    let body = tokio::time::timeout(
        Duration::from_secs(10),
        server.chat(&non_streaming_body(USER_MESSAGE)),
    )
    .await
    .expect("抽出を待たずに応答が返ること（同期で走らせていない）");
    assert!(body.contains(REPLY), "前提: 本文が返ること: {body}");

    // 前提: 抽出は**まだ**終わっていない（開けていないので記憶は入らない）。
    assert!(
        server.memories().await.is_empty(),
        "前提: 開ける前に記憶が入っていないこと"
    );

    open.send(()).expect("抽出の上流を開けること");
    let memories = server.wait_for_memories(2).await;
    assert_eq!(memories.len(), 2, "開けた後に記憶が入ること");

    server.stop().await;
}

/// **中継経路でも応答は抽出を待たない。**
///
/// 中継は `[DONE]` の直前で保存する（`server::relay`）ので、
/// 抽出を同期で走らせると**`[DONE]` が永久に届かない。**
#[tokio::test]
async fn the_relay_never_waits_for_the_extraction() {
    let (sub, open) = SubLlm::gated(&[RELAY_EXTRACTED]);
    let server = TestServer::start(MainLlm::new(), Arc::clone(&sub)).await;

    let body = tokio::time::timeout(
        Duration::from_secs(10),
        server.chat(&streaming_body(RELAY_USER_MESSAGE)),
    )
    .await
    .expect("抽出を待たずに `[DONE]` まで流れ切ること");
    assert!(body.contains("[DONE]"), "前提: 中継が完結すること: {body}");

    assert!(
        server.memories().await.is_empty(),
        "前提: 開ける前に記憶が入っていないこと"
    );

    open.send(()).expect("抽出の上流を開けること");
    server.wait_for_memories(2).await;

    server.stop().await;
}

/// **保存に失敗した回は抽出を走らせない**（ADR-0021 §6「保存に成功した直後」）。
///
/// 失敗した往復は履歴に載っていない。そこから抽出すると
/// **どこにも残っていない会話から作られた記憶**が恒久的に残る。
///
/// # 先に成功する回を1つ通してある
///
/// 「呼ばれていない」は**仕掛けが動いていないだけ**でも成り立つ。
/// 同じ経路で1回成功させ、その分が数十ミリ秒で数に出ることを見てから
/// 否定側を主張する。**その回の抽出が終わり切るのも待つ**——
/// 走ったままだと、2回目は「忙しいので走らない」（§7）で数が増えず、
/// **保存の成否を見ない変異が緑を通り抜ける。**
#[tokio::test]
async fn a_failed_persist_never_starts_an_extraction() {
    let main = MainLlm::new();
    let sub = SubLlm::replying(&[EXTRACTED, RELAY_EXTRACTED]);
    let server = TestServer::start(Arc::clone(&main), Arc::clone(&sub)).await;

    // 1. 保存が成功する回。**ここで数が 1 になる。**
    server.chat(&non_streaming_body(USER_MESSAGE)).await;
    server.wait_for_memories(2).await;
    // 抽出のタスクが**終わり切る**のを待つ（`is_finished` になるまで）。
    tokio::time::sleep(SETTLE).await;
    assert_eq!(sub.call_count(), 1, "前提: 成功した回では抽出が走ること");

    // 2. 保存だけが失敗する回。**応答は返るが往復は書けない。**
    main.break_the_store_on_the_next_call(server.store.clone());
    let body = server.chat(&non_streaming_body("やっぱりただいま")).await;
    assert!(body.contains(REPLY), "前提: 応答自体は返ること: {body}");

    tokio::time::sleep(SETTLE).await;
    assert_eq!(
        sub.call_count(),
        1,
        "保存できなかった往復から抽出を始めないこと"
    );

    // **細工でプールを閉じた後でも停止は通る**（`Store::close` は冪等）。
    server.stop().await;
}

/// **抽出が失敗しても本文の応答は壊れない**（ADR-0021 §6 の前提）。
///
/// 上流が落ちていても、崩れた出力を返しても、**応答は返り、記憶が増えないだけ。**
#[tokio::test]
async fn a_failing_extraction_leaves_the_reply_untouched() {
    let sub = SubLlm::failing();
    let server = TestServer::start(MainLlm::new(), Arc::clone(&sub)).await;

    let body = server.chat(&non_streaming_body(USER_MESSAGE)).await;
    assert!(body.contains(REPLY), "本文は返ること: {body}");

    // **肯定側は締切まで poll する**（固定の `sleep` にしない / [`SETTLE`] の doc）。
    wait_for_calls(&sub, 1).await;
    assert_eq!(sub.call_count(), 1, "抽出は1往復につき1回だけであること");

    // ここから先は否定側。**上流が呼ばれた後**なので、記憶が入るなら入っている。
    assert!(
        server.memories().await.is_empty(),
        "失敗した抽出から記憶を作らないこと"
    );

    server.stop().await;
}

/// **サーバを止めると、走っている抽出も落ちる**（ADR-0021 §9。leak させない）。
///
/// 抽出の上流は開かないままにしてあるので、落とさなければ
/// **待ち受けが終わった後もタスクが生き残る。**
#[tokio::test]
async fn a_running_extraction_is_dropped_when_the_server_stops() {
    let (sub, _open) = SubLlm::gated(&[EXTRACTED]);
    let server = TestServer::start(MainLlm::new(), Arc::clone(&sub)).await;

    server.chat(&non_streaming_body(USER_MESSAGE)).await;

    // 前提: 抽出が現に走り出していること（走っていないものは落とせない）。
    let deadline = Instant::now() + WAIT_LIMIT;
    while sub.call_count() == 0 {
        assert!(Instant::now() < deadline, "抽出が走り出すこと");
        tokio::time::sleep(POLL_STEP).await;
    }
    assert_eq!(sub.dropped_count(), 0, "前提: まだ落ちていないこと");

    server.stop().await;

    let deadline = Instant::now() + WAIT_LIMIT;
    while sub.dropped_count() == 0 {
        assert!(
            Instant::now() < deadline,
            "停止したら走っている抽出も落ちること（タスクを leak させない）"
        );
        tokio::time::sleep(POLL_STEP).await;
    }
}

/// 上流が返す本文（OpenAI 互換）。**抽出として読める行**を1件だけ含む。
const UPSTREAM_BODY: &str = r#"{
  "id": "chatcmpl-1",
  "object": "chat.completion",
  "created": 1,
  "model": "PLACEHOLDER",
  "choices": [{"index": 0, "message": {"role": "assistant", "content": "- 利用者: 山あいの町で育った"},
               "finish_reason": "stop"}]
}"#;

/// **本番の入口（`App::bind`）を通ったとき、抽出だけがサブモデルへ行く**（ADR-0021 §10）。
///
/// # なぜモック注入では見られないのか
///
/// `LlmClient` はモデル識別子を持たない（`ChatRequest` に項目が無い / ADR-0004）。
/// **綴りが載るのは上流へ出た HTTP 本文だけ**なので、ここだけは
/// `wiremock` のローカルサーバを立てて本番の入口を通す。
///
/// # ここで与える2つは、既定のどちらとも別の綴りにしてある
///
/// 本文モデルと抽出モデルに別の綴りを与え、**そのどちらも既定値と違う**ので、
/// 「設定を読まずに既定を使う」「本文モデルで抽出する」のどちらも赤になる。
///
/// **既定どうしは 2026-08-21 から同じ綴りである**（ADR-0021 §10）。
/// つまり「既定を使う」変異を赤にできるのは、**ここで明示している綴り**だけであり、
/// `DEFAULT_SUB_MODEL` を渡すテストでは区別が付かない。
#[tokio::test]
async fn the_sub_model_reaches_the_upstream_through_the_real_bind() {
    /// 本文生成に指定するモデル。**既定とも抽出用とも別。**
    const MAIN: &str = "ministral-3-14b-reasoning-2512";
    /// 抽出に指定するモデル。**既定とも本文用とも別。**
    const SUB: &str = "ministral-3-3b-reasoning-2512";

    assert_ne!(MAIN, SUB, "前提: 2つは別の綴りであること");
    assert_ne!(
        MAIN,
        personacore::llm::DEFAULT_MAIN_MODEL,
        "前提: 本文モデルは既定と別の綴りであること"
    );
    assert_ne!(
        SUB,
        personacore::llm::DEFAULT_SUB_MODEL,
        "前提: 抽出モデルは既定と別の綴りであること"
    );

    let upstream = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            UPSTREAM_BODY.replace("PLACEHOLDER", MAIN),
            "application/json",
        ))
        .mount(&upstream)
        .await;

    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let db_path = dir.path().join("personacore.sqlite");
    let endpoint = upstream.uri();
    let env = [
        (ENV_LLM_ENDPOINT, endpoint.as_str()),
        (ENV_MAIN_MODEL, MAIN),
        (ENV_SUB_MODEL, SUB),
    ];
    let config = AppConfig::from_lookup(|key| {
        env.iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| (*value).to_owned())
    })
    .expect("設定を組み立てられること")
    .with_bind(BindAddr::loopback(0))
    .with_database_path(&db_path);

    // **`bind_with_client` ではない。** ここが本番と同じ入口であることが要点。
    let app = App::bind(&config).await.expect("待ち受けを開始できること");
    let addr = app.local_addr();
    let store = app.store().clone();
    store
        .create_character(
            &Handle::parse("aoi").expect("正当な名前であること"),
            &DisplayName::parse("葵").expect("正当な表示名であること"),
            &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
            None,
            StatScale::parse(7).expect("正当な尺度であること"),
        )
        .await
        .expect("キャラクターを作成できること");

    let (shutdown, rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        app.serve(async move {
            let _ = rx.await;
        })
        .await
    });

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/v1/chat/completions"))
        .json(&non_streaming_body(USER_MESSAGE))
        .send()
        .await
        .expect("サーバへ接続できること");
    assert!(response.status().is_success(), "{:?}", response.status());
    let _ = response.text().await.expect("本文を読めること");

    // **2往復目（抽出）が上流へ届くまで待つ。**
    let deadline = Instant::now() + WAIT_LIMIT;
    let requests = loop {
        let requests: Vec<Value> = upstream
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|request| request.url.path() == "/v1/chat/completions")
            .map(|request| serde_json::from_slice(&request.body).expect("JSON として読めること"))
            // **約束・決まりの提案の依頼は数えない**（抽出の後ろで走る / 作業単位 A-12c-1）。
            .filter(|request: &Value| !is_agreement_proposal_body(request))
            .collect();
        if requests.len() >= 2 {
            break requests;
        }
        assert!(
            Instant::now() < deadline,
            "抽出の呼び出しが上流へ届くこと: {requests:?}"
        );
        tokio::time::sleep(POLL_STEP).await;
    };

    assert_eq!(
        requests.len(),
        2,
        "本文と抽出で2往復であること: {requests:?}"
    );
    assert_eq!(
        requests[0]["model"].as_str(),
        Some(MAIN),
        "1往復目は本文モデルであること: {:?}",
        requests[0]
    );
    assert_eq!(
        requests[1]["model"].as_str(),
        Some(SUB),
        "2往復目（抽出）はサブモデルであること: {:?}",
        requests[1]
    );

    // 2往復目が**抽出の依頼**であること（本文をもう一度投げているのではない）。
    let extraction = requests[1]["messages"].to_string();
    assert!(
        extraction.contains(personacore::prompt::extraction::EXAMPLE_USER_LINE),
        "抽出の依頼であること: {extraction}"
    );

    shutdown.send(()).expect("停止シグナルを送れること");
    handle
        .await
        .expect("サーバタスクが panic しないこと")
        .expect("正常に終了すること");
}

/// **サブモデルの実コンテキスト長を上流が申告する形の `/api/v0/models`**（ADR-0020 と同じ経路）。
///
/// **本文用と抽出用で必ず別の値を返す。** 同じ値にすると、
/// 照会するモデルを取り違える実装がそのまま緑を通る
/// （CLAUDE.md「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
fn native_models_body(main: &str, main_context: u32, sub: &str, sub_context: u32) -> String {
    assert_ne!(main, sub, "前提: 2つのモデルは別の綴りであること");
    assert_ne!(
        main_context, sub_context,
        "前提: 2つのコンテキスト長は別の値であること（同値だと取り違えを検出できない）"
    );
    format!(
        r#"{{"object":"list","data":[
          {{"id":"{main}","object":"model","type":"llm","state":"loaded",
            "max_context_length":262144,"loaded_context_length":{main_context}}},
          {{"id":"{sub}","object":"model","type":"llm","state":"loaded",
            "max_context_length":262144,"loaded_context_length":{sub_context}}}]}}"#
    )
}

/// **本番の入口を通して1往復を投げ、上流への `/v1/chat/completions` の回数を返す。**
///
/// **1 なら本文だけ**（抽出の依頼は §8 で捨てられた）、**2 なら抽出も上流へ出た。**
///
/// `deadline` は待ち方を呼び出し側が決めるための引数。
/// **肯定側（出ること）は [`WAIT_LIMIT`] まで poll し、
/// 否定側（出ないこと）は [`SETTLE`] だけ猶予を置く**（この2つを取り違えると、
/// 「遅い機械で落ちる見張り番」か「何も測らない見張り番」になる / [`SETTLE`] の doc）。
async fn extraction_roundtrips_through_the_real_bind(
    main_model: &str,
    main_context: u32,
    sub_model: &str,
    sub_context: u32,
    user_message: &str,
    deadline: Duration,
) -> usize {
    let upstream = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            UPSTREAM_BODY.replace("PLACEHOLDER", main_model),
            "application/json",
        ))
        .mount(&upstream)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v0/models"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            native_models_body(main_model, main_context, sub_model, sub_context),
            "application/json",
        ))
        .mount(&upstream)
        .await;

    let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
    let db_path = dir.path().join("personacore.sqlite");
    let endpoint = upstream.uri();
    let env = [
        (ENV_LLM_ENDPOINT, endpoint.as_str()),
        (ENV_MAIN_MODEL, main_model),
        (ENV_SUB_MODEL, sub_model),
    ];
    let config = AppConfig::from_lookup(|key| {
        env.iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| (*value).to_owned())
    })
    .expect("設定を組み立てられること")
    .with_bind(BindAddr::loopback(0))
    .with_database_path(&db_path);

    // **`bind_with_client` ではない。** ここが本番と同じ入口であることが要点。
    let app = App::bind(&config).await.expect("待ち受けを開始できること");
    let addr = app.local_addr();
    let store = app.store().clone();
    store
        .create_character(
            &Handle::parse(HANDLE).expect("正当な名前であること"),
            &DisplayName::parse(DISPLAY_NAME).expect("正当な表示名であること"),
            &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
            None,
            StatScale::parse(7).expect("正当な尺度であること"),
        )
        .await
        .expect("キャラクターを作成できること");

    let (shutdown, rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        app.serve(async move {
            let _ = rx.await;
        })
        .await
    });

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/v1/chat/completions"))
        .json(&non_streaming_body(user_message))
        .send()
        .await
        .expect("サーバへ接続できること");
    assert!(response.status().is_success(), "{:?}", response.status());
    let _ = response.text().await.expect("本文を読めること");

    let limit = Instant::now() + deadline;
    let calls = loop {
        let calls = completions_to(&upstream).await;
        if calls >= 2 || Instant::now() >= limit {
            break calls;
        }
        tokio::time::sleep(POLL_STEP).await;
    };

    shutdown.send(()).expect("停止シグナルを送れること");
    handle
        .await
        .expect("サーバタスクが panic しないこと")
        .expect("正常に終了すること");

    calls
}

/// `/v1/chat/completions` を叩いた回数（**コンテキスト長の照会は数えない**）。
///
/// **約束・決まりの提案の依頼も数えない**（抽出の後ろの同じ列で走る / 作業単位 A-12c-1）。
/// ここが数えたいのは本文と抽出の往復で、提案の依頼の上限は抽出と同じ規則から出る。
async fn completions_to(server: &MockServer) -> usize {
    server.received_requests().await.map_or(0, |requests| {
        requests
            .iter()
            .filter(|request| request.url.path() == "/v1/chat/completions")
            .filter(|request| {
                serde_json::from_slice::<Value>(&request.body)
                    .map_or(true, |body| !is_agreement_proposal_body(&body))
            })
            .count()
    })
}

/// **抽出の上限は「サブモデルの実コンテキストの半分」で、その実コンテキストは
/// 上流に引く**（作業単位 B-7 / ADR-0036 決定2 の第1階層 / ADR-0020 と同じ器）。
///
/// # なぜ本番の入口でなければ見られないのか
///
/// 照会先（`/api/v0/models`）は **LM Studio 独自で `LlmClient` に載っていない**ので、
/// モック注入の入口（`App::bind_with_clients`）は**引き直しの配線を一切通らない**
/// （`App::bind_with_client` の doc）。**ここだけが本番の配線を通る。**
///
/// # 2つの腕は、本文用とサブ用の申告値を**そっくり入れ替えただけ**
///
/// | 腕 | 本文 | 抽出 | 上限 | 依頼（約 4.7k） | 期待 |
/// |---|---|---|---|---|---|
/// | 狭いサブ | 12288 | **6144** | 3072 | 上限を超える | **捨てる**（往復は本文の1回） |
/// | 広いサブ | 6144 | **12288** | 6144 | 収まる | **送る**（2回） |
///
/// **照会するモデルを取り違えると、2つの腕が両方とも逆になる。**
/// **引けた値を捨てて落とし所（16384 / 上限 8192）を使うと、狭い側の腕が送ってしまう。**
/// **「半分」を「全部」に緩めると、狭い側の上限が 6144 になり、やはり送ってしまう**
/// ——依頼の大きさは**わざと `サブ/2` と `サブ` の間**に置いてある。
///
/// # 依頼の大きさは詰め物で決める
///
/// 削れない枠（実測 2339 / `the_fixed_part_of_the_request_leaves_room_for_a_conversation`）は
/// 呼び名の長さで動くので、**枠だけに頼ると腕の期待値が枠の実測に寄りかかる。**
/// 詰め物で 2600 バイト積んで、**どちらの境界からも 1000 トークン以上離す。**
#[tokio::test]
async fn the_extraction_limit_follows_the_sub_models_context_through_the_real_bind() {
    /// 本文生成に指定するモデル。**既定とも抽出用とも別。**
    const MAIN: &str = "ministral-3-14b-reasoning-2512";
    /// 抽出に指定するモデル。**既定とも本文用とも別。**
    const SUB: &str = "ministral-3-3b-reasoning-2512";
    /// 狭い側の申告値。**上限は 3072**（依頼が収まらない）。
    const NARROW: u32 = 6_144;
    /// 広い側の申告値。**上限は 6144**（依頼が収まる）。
    const WIDE: u32 = 12_288;
    /// 詰め物のバイト数（`estimate_tokens` は UTF-8 バイト数）。
    const PADDING_BYTES: usize = 2_600;

    assert_ne!(NARROW, WIDE, "前提: 2つの申告値は別の値であること");
    // 落とし所（引けなかったときのサブのコンテキスト長）とも別であること。
    // **上限からの逆算で書く**（落とし所の綴りをテスト側に写さない）。
    let fallback_context = personacore::server::extraction_prompt_budget_for_tests() * 2;
    for (label, tokens) in [("狭い側", NARROW), ("広い側", WIDE)] {
        assert_ne!(
            tokens, fallback_context,
            "前提: {label}の申告値が落とし所と別であること（同値だと引けた値を捨てる実装が緑になる）"
        );
    }

    let message = format!("{USER_MESSAGE}{}", "a".repeat(PADDING_BYTES));

    // 腕1: **サブが狭い。** 上限 3072 に依頼が収まらないので抽出は捨てられる。
    // **否定側なので猶予（`SETTLE`）だけ置く。**
    let narrow_sub =
        extraction_roundtrips_through_the_real_bind(MAIN, WIDE, SUB, NARROW, &message, SETTLE)
            .await;
    assert_eq!(
        narrow_sub,
        1,
        "サブモデルが {NARROW} なら上限は {} で、この依頼は捨てられること\
         （本文モデルの {WIDE} を見ていないか、落とし所を使っている）",
        NARROW / 2
    );

    // 腕2: **サブが広い。** 上限 6144 に収まるので抽出が上流へ出る。
    // **肯定側なので締切まで poll する。**
    let wide_sub =
        extraction_roundtrips_through_the_real_bind(MAIN, NARROW, SUB, WIDE, &message, WAIT_LIMIT)
            .await;
    assert_eq!(
        wide_sub,
        2,
        "サブモデルが {WIDE} なら上限は {} で、この依頼は上流へ出ること\
         （本文モデルの {NARROW} を見ている）",
        WIDE / 2
    );
}
