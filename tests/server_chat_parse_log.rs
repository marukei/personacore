//! **パース成功率のログ**（CLAUDE.md 絶対制約 / 作業単位 11b-2）の検証。
//!
//! 見ているのは4点だけ:
//!
//!   1. **非ストリーミングと中継の両方**の経路で、分解の結果が記録されること。
//!   2. 記録は `ParseOutcome::label()`（定数）であり、**成功と崩れが区別できる**こと。
//!   3. **生出力をログに載せない**こと。
//!   4. **軸名の漏れ**（ADR-0018）が同じ1行に載り、
//!      **その判定が保存を1件も変えない**こと（漏れたターンも往復2件がそのまま入る）。
//!
//! # なぜ専用のテストバイナリなのか
//!
//! 判定はサーバのタスク側で走るので、スレッドローカルな `with_default` では捕まらず、
//! `set_global_default` が要る。**それはプロセスにつき1回しか呼べない**ので、
//! `tests/server_chat.rs` と `tests/server_chat_stream.rs` は既に1本ずつ使い切っている。
//! cargo は `tests/*.rs` ごとに別プロセスにするため、**この観測窓を持てる**
//! （docs/06-roadmap.md「統合テスト側には同じ問題が残っている」の選択肢1）。
//!
//! **⚠ 行と欄の突き合わせは、もう複製ではない**（作業単位 B-3a）。
//! [`line_containing`] / [`line_with_field`] / [`has_field`] / [`bracketed_field`] は
//! `common` 経由で **`src/testlog/assert.rs` と同じファイル**を読んでいる。
//! ここで写しを作り直さないこと
//! ——押さえの見張り番はあちらにしか無いので、写した瞬間に**この経路だけ無防備**になる。
//!
//! **⚠ 行も欄も「丸ごとのトークン」で見る**（作業単位 B-21b）。
//! このファイルは B-3a の時点で [`has_field`] を取り込んでいたのに、
//! **`fell_back=…` と `axis_leak_…=[]` は素の `contains`、
//! `outcome=…` での行の選択は [`line_containing`]** のままだった
//! ——**作法は同じファイルの中にあって、写されていなかっただけ**（B-12 の「収穫」と同じ形）。
//!
//! | 手掛かり | 使うもの |
//! |---|---|
//! | **欄で行を選ぶ**（`outcome=…`） | [`line_with_field`] |
//! | **文言で行を選ぶ**（[`PARSE_MARKER`] / [`PERSIST_FAILURE_MARKER`]） | [`line_containing`] |
//! | **欄の値を見る**（`fell_back=…` / `session_id=…`） | [`has_field`] |
//! | **欄の値に空白が入りうる**（`axis_leak_…=[…]`） | [`bracketed_field`] |
//!
//! **否定側（「載っていないこと」）だけは素の `contains` のまま。**
//! 探しているのが `キー=値` ではなく**値の断片**（応答本文・利用者の発言・軸名）なので、
//! **キーが伸びて別の欄に当たる、という壊れ方が起こらない**
//! （[`has_field`] の doc の「否定に使ってよいか」の表）。
//!
//! **⚠ バッファと subscriber の設置も、もう複製ではない**（作業単位 B-3）。
//! [`SharedBuffer`]（`Write` / `MakeWriter` を含む）と [`shared_log`] の中身は
//! **同じ `src/testlog/assert.rs`** にある。**「1プロセス1回」は共有を妨げない**
//! ——`#[path]` で取り込んだファイルは**バイナリごとに別々にコンパイルされる**ので、
//! あちらの `OnceLock` も**このバイナリ専用**になる。
//! **畳めないのは「バイナリを4本に分けていること」そのもの。**
//!
//! **バイナリを分けてもバッファの共有は消えない。**（当初はそう書いていたが誤り。）
//! 1バイナリの中で `set_global_default` が使えるのは1回だけなので、
//! **このバイナリのテストは1つのバッファを共有する。** その扱いは下の
//! 「1つのバッファを複数のテストで共有する」を読むこと。
//!
//! # 2つの経路を1つのバッファで見分ける方法
//!
//! **経路ごとに違う崩れ方の応答を返させる。** 非ストリーミングは正常に分解でき、
//! 中継は `「` が閉じない。ラベルが `structured` と `fallback_unclosed` に分かれるので、
//! **片方の経路だけログを消す変異**がそのまま赤になる。
//! 同じ応答を返させると、1本の行で両方の主張が満たされてしまう。
//!
//! # 保存に失敗したときの `error!` も、ここで見張る（11b-2 の追補）
//!
//! `TurnSink::persist` は `Result` を返さない設計なので、
//! **`error!` の1行が「黙ったデータ欠落」に気付く唯一の手段**（`server::turn` の doc）。
//! これも `set_global_default` が要るため、同じバイナリに置いてある。
//!
//! **保存できた回とできなかった回で、応答のラベルを分けてある**
//! （`narration_only` と `fallback_unclosed`）。分けないと
//! **「保存できた回だけ記録する」変異が通り抜ける。**
//! 実測値は `server::turn` の `log_outcome` の位置に置いてある
//! （**同じ数字を2箇所に書かない。片方を直しても、もう片方が黙って古びる**）。
//!
//! # 1つのバッファを複数のテストで共有する
//!
//! subscriber の設置は [`shared_log`] に集約し、プロセスで1回だけ行う。
//! 共有の弊害（別のテストが出した行で主張が満たされる）は、
//! **約束事ではなく仕掛けで潰してある:**
//!
//! 1. [`LogSection::enter`] が [`ONE_TEST_AT_A_TIME`] を通す
//!    ——**同時に走るテストは常に1本。**
//! 2. 入場時のバッファ長を控え、[`LogSection::mine`] は**その後ろだけ**を返す。
//!
//! この2つで「肯定側の主張は、自分のリクエストが出した行でしか満たされない」が成り立つ。
//!
//! > **厳密には例外が1つある。** 仕掛け自身の見張り番
//! > （`a_log_section_shows_only_the_lines_written_after_it_opened`）は、
//! > **入場前に1行書く**——それが区間に入らないことを見るのが目的なので、
//! > **錠の外で書くしかない。** その行は他の区間へ混ざりうる。
//! > 今はどの主張とも一致しない文字列なので実害が無いだけで、
//! > **`mine()` に対する否定側の主張や行数の検査を足すとフレーキーになる。**
//!
//! **どちらか一方でも外すと、ラベルの一意性をコメントで守っていた状態に戻る**
//! ——実測で、`fallback_unclosed` を出すテストをもう1本足しただけで
//! `both_paths_record_…` が中継側の記録を消す変異を捕まえられなくなった。
//!
//! 否定側（「ログに載っていないこと」）だけは [`LogSection::everything`] で
//! **バッファ全体**を見る。区間に狭めると弱くなるだけだから。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use personacore::domain::{DisplayName, Handle, Message, Persona, Role, StatAlloc, StatAxis};
use personacore::llm::{
    ChatChunk, ChatRequest, ChatStream, Completion, FinishReason, LlmClient, LlmError, LlmFuture,
};
use personacore::parse::parse_response;
use personacore::server::{App, AppConfig, BindAddr, ServerError};
use personacore::store::Store;
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

use common::{
    SharedBuffer, bracketed_field, fixture_scale, has_field, has_key, install_global_log,
    line_containing, line_with_field,
};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// 非ストリーミングで返す応答。**正常に分解できる形**（`structured`）。
const GOOD_REPLY: &str = "「ヒミツだよ」 彼女は微笑んだ。";

/// 中継で返す応答。**`「` が閉じない**（`fallback_unclosed`）。
const BROKEN_REPLY: &str = "彼女は口をつぐんだ。「ヒミツ";

/// 両方の応答に含めてある目印。**ログに1文字も出てはいけない。**
const SECRET: &str = "ヒミツ";

/// 分解を記録した行の手掛かり。**全文一致にしない**（文言の推敲で割れる）。
const PARSE_MARKER: &str = "応答を分解しました";

/// 保存に失敗したことを記録した行の手掛かり。**全文一致にしない。**
const PERSIST_FAILURE_MARKER: &str = "会話の往復を保存できませんでした";

/// 保存に**成功する**回（前提づくりの3ターン）の応答。**地の文だけ**（`narration_only`）。
const SAVED_REPLY: &str = "彼女は黙って頷いた。";

/// 保存に**失敗する**回だけの応答。**閉じない `「`**（`fallback_unclosed`）。
///
/// **保存に成功した回と別のラベルにしてある。** 同じラベルだと
/// 「保存できた回の記録」で「保存できなかった回の記録」の主張が満たされてしまい、
/// **`log_outcome` を `append_turn` の後ろへ移して成功時だけ呼ぶ変異**が通り抜ける。
///
/// **`ParseOutcome::Empty` を使わないこと。** 応答が空文字になるので、
/// 否定側の `!contents.contains(UNSAVED_REPLY)` が `contains("")` で**必ず真**になり、
/// 「生出力が漏れていないこと」の主張が**無条件に通る死んだ検査**になる。
///
/// **入れ子の `「外「内」外」` も使えない**（ADR-0014 で崩れから外れ、
/// `fell_back=true` を主張できなくなった）。崩れの実例は `Unclosed` から採る。
///
/// # ラベルは**もうプロセス内で一意ではない**（根拠が入れ替わった）
///
/// 入れ子を使っていた頃の `fallback_nested` はこのバイナリで唯一のラベルだったが、
/// `fallback_unclosed` は [`BROKEN_REPLY`]（`both_paths_…` の中継側）と**衝突する。**
/// 実測で、[`LogSection`] の区間分離を無効化して `both_paths_…` を先に走らせると
/// [`line_containing`] は**他のテストの行を掴む。**
///
/// それでも主張が他所の行で満たされないのは、
/// **[`LogSection`] の区間と `session_id` の2枚で持っている**から
/// （`has_field(outcome, "session_id=…")` が効いている）。
/// **「ラベルが一意だから安全」と読まないこと。** 上の「別のラベルにしてある」は
/// **保存できた回 [`SAVED_REPLY`] との**区別だけを指す。
const UNSAVED_REPLY: &str = "彼女は言い淀んだ。「そのことは";

/// 保存に失敗させる回の発言。**ログに1文字も出てはいけない。**
const UNSAVED_USER: &str = "この発言はどこにも残らない";

/// 上流（LM Studio）の代役。**ネットワークには一切触らない。**
struct MockLlm {
    /// `stream` が返すチャンク列。**1回だけ取り出せる。**
    source: Mutex<Option<ChatStream>>,
    reply: Completion,
    /// **応答を返す直前に永続化層を壊す**（保存だけを失敗させるための細工）。
    ///
    /// セッション解決・プロンプト組立・上流呼び出しがすべて済んだ後に
    /// **保存だけが失敗する**状況を作る唯一の手段
    /// （`tests/server_chat.rs` と同じ細工）。
    sabotage: Mutex<Option<Store>>,
    /// **次の1回だけ差し替える応答。**
    ///
    /// 保存に失敗する回だけ別のラベルになる応答を返させるため
    /// （[`UNSAVED_REPLY`] の doc）。
    next_reply: Mutex<Option<String>>,
}

impl MockLlm {
    /// **中継で流すチャンク列を差し替えたモック。**
    ///
    /// [`MockLlm::new`] のチャンク列は `both_paths_…` 専用に固定されているので、
    /// 別の本文を中継させたいテストはこちらを使う（`complete` は空を返す）。
    fn streaming(deltas: &[&str]) -> Arc<Self> {
        let last = deltas.len().saturating_sub(1);
        let chunks: Vec<Result<ChatChunk, LlmError>> = deltas
            .iter()
            .enumerate()
            .map(|(index, delta)| {
                Ok(ChatChunk {
                    delta: (*delta).to_owned(),
                    finish_reason: (index == last).then_some(FinishReason::Stop),
                    usage: None,
                })
            })
            .collect();

        Arc::new(Self {
            source: Mutex::new(Some(Box::pin(futures_util::stream::iter(chunks)))),
            reply: Completion {
                content: String::new(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            },
            sabotage: Mutex::new(None),
            next_reply: Mutex::new(None),
        })
    }

    /// 固定の本文を返すモック。**中継は使わない**（`stream` は失敗する）。
    fn replying(content: &str) -> Arc<Self> {
        Arc::new(Self {
            source: Mutex::new(None),
            reply: Completion {
                content: content.to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            },
            sabotage: Mutex::new(None),
            next_reply: Mutex::new(None),
        })
    }

    /// **次の `complete` で永続化層を閉じる。**
    fn break_the_store_on_the_next_call(&self, store: Store) {
        *self.sabotage.lock().expect("ロックを取れること") = Some(store);
    }

    /// **次の `complete` だけ別の本文を返す。**
    ///
    /// 保存に失敗する回を**ラベルで名指しできる**ようにするための細工。
    /// 全ターンが同じ応答だと、記録の有無を行数で数えるしか手が無くなる。
    fn reply_once_with(&self, content: &str) {
        *self.next_reply.lock().expect("ロックを取れること") = Some(content.to_owned());
    }

    fn new() -> Arc<Self> {
        // **中継は複数チャンクに割って流す**（蓄積した本文が分解対象であることの確認）。
        let chunks: Vec<Result<ChatChunk, LlmError>> = vec![
            Ok(ChatChunk {
                delta: "彼女は口をつぐんだ。".to_owned(),
                finish_reason: None,
                usage: None,
            }),
            Ok(ChatChunk {
                delta: "「ヒミツ".to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            }),
        ];

        Arc::new(Self {
            source: Mutex::new(Some(Box::pin(futures_util::stream::iter(chunks)))),
            reply: Completion {
                content: GOOD_REPLY.to_owned(),
                finish_reason: Some(FinishReason::Stop),
                usage: None,
            },
            sabotage: Mutex::new(None),
            next_reply: Mutex::new(None),
        })
    }
}

impl LlmClient for MockLlm {
    fn pinned(&self) -> Option<std::sync::Arc<dyn LlmClient>> {
        None
    }

    fn complete<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, Completion> {
        Box::pin(async move {
            // **ロックを握ったまま `await` しない**（取り出してから閉じる）。
            let sabotage = self.sabotage.lock().expect("ロックを取れること").take();
            let next_reply = self.next_reply.lock().expect("ロックを取れること").take();
            if let Some(store) = sabotage {
                store.close().await;
            }
            let mut reply = self.reply.clone();
            if let Some(content) = next_reply {
                reply.content = content;
            }
            Ok(reply)
        })
    }

    fn stream<'a>(&'a self, _request: &'a ChatRequest) -> LlmFuture<'a, ChatStream> {
        Box::pin(async move {
            self.source
                .lock()
                .expect("ロックを取れること")
                .take()
                .ok_or(LlmError::MissingContent)
        })
    }
}

/// エフェメラルポートで起動した実サーバ。**上流はモック。**
struct TestServer {
    addr: SocketAddr,
    /// 永続化層。**細工（プールを閉じる）にだけ使う。**
    store: Store,
    client: reqwest::Client,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    async fn start() -> Self {
        Self::start_with(MockLlm::new()).await
    }

    async fn start_with(llm: Arc<MockLlm>) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db_path = dir.path().join("personacore.sqlite");

        let config = AppConfig::from_lookup(|_| None)
            .expect("設定を組み立てられること")
            .with_bind(BindAddr::loopback(0))
            .with_database_path(&db_path);

        // **モックを注入する。** ここが実際の LM Studio に繋がらないことの根拠。
        let app = App::bind_with_client(&config, llm)
            .await
            .expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let store = app.store().clone();

        store
            .create_character(
                &Handle::parse("aoi").expect("正当な名前であること"),
                &DisplayName::parse("葵").expect("正当な表示名であること"),
                &Persona::parse("落ち着いた口調で話す。").expect("正当なペルソナであること"),
                None,
                fixture_scale(),
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

        Self {
            addr,
            store,
            client: reqwest::Client::new(),
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    /// **キャラクターに性格ステータスを持たせる**（軸名の漏れの判定材料 / ADR-0018）。
    ///
    /// **`/admin/` ではなく永続化層から入れる。** 見たいのはチャット経路の配線で、
    /// 軸をどう登録したかではない。
    ///
    /// # 末尾に値 0 の軸を置かないこと
    ///
    /// `Store::stat_allocs` は `ORDER BY stat_alloc.rowid` なので、
    /// **ここへ渡した順がそのまま `parse::leak` が走査する順**になる。
    /// 末尾が検出対象外（値 0）の軸だと、「一覧の最後の軸を見ない」実装
    /// （`stats[..len - 1]`）と「述語で落とした」結果が同じになる。
    /// **値ではなく位置の前提**なので、軸名の `assert_ne!` では守れない——入口で見る。
    ///
    /// **実測値は写さない**（`parse::leak` の `tests::stats` の doc に置いてある）。
    async fn give_stats(&self, allocs: &[(&str, u32)]) {
        let (last_axis, last_value) = allocs.last().expect("軸を1本以上渡すこと");
        assert_ne!(
            *last_value, 0,
            "末尾に値 0 の軸を置かないこと（末尾を見ない実装が緑になる）: {last_axis}"
        );

        let handle = Handle::parse("aoi").expect("正当な名前であること");
        let character = self
            .store
            .find_character_by_handle(&handle)
            .await
            .expect("キャラクターを取得できること")
            .expect("作成済みであること");

        let allocs: Vec<StatAlloc> = allocs
            .iter()
            .map(|(axis, value)| StatAlloc {
                axis: StatAxis::parse(axis).expect("正当な軸名であること"),
                value: character
                    .stat_scale
                    .value(*value)
                    .expect("尺度に収まる値であること"),
            })
            .collect();

        self.store
            .replace_stat_allocs(character.id, &allocs)
            .await
            .expect("性格ステータスを入れられること");
    }

    /// **そのセッションに保存された会話**を時系列順で読む。
    ///
    /// セッションは応答の `model`（`aoi@<id>`）から取った生の id で引く。
    /// `SessionId` は所有確認を通ってしか作れないので、ここも `find_session` を通す。
    async fn saved_messages(&self, raw_session_id: i64) -> Vec<Message> {
        let handle = Handle::parse("aoi").expect("正当な名前であること");
        let character = self
            .store
            .find_character_by_handle(&handle)
            .await
            .expect("キャラクターを引けること")
            .expect("作成済みであること");
        let session = self
            .store
            .find_session(character.id, raw_session_id)
            .await
            .expect("セッションを引けること")
            .expect("セッションが存在すること");
        self.store
            .recent_messages(session.id, 50)
            .await
            .expect("会話履歴を読めること")
    }

    /// 応答を最後まで読み切る（本文だけ返す。**読み切らないと保存が走らない**）。
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

/// **このバイナリのテストを1本ずつ通す回転扉。**
///
/// [`LogSection`] の「自分の区間」は、**同時に他のテストが走っていない**ことで
/// 初めて「自分の行だけ」になる。ここを外すと区間に他のテストの行が混ざり、
/// **ラベルの一意性をコメントで守っていた状態へ逆戻りする。**
///
/// `std` ではなく `tokio` の錠なのは、`await` を跨いで持つため
/// （毒（poison）も無いので、1本が落ちても残りは走る）。
static ONE_TEST_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// **1本のテストが出したログの区間。**
///
/// 各テストは最初にこれを取り、[`Self::mine`] で**自分のリクエストが出した行だけ**を見る。
/// 差す前・他のテストの行は入らないので、
/// **同じラベルを出すテストが他にあっても主張が満たされない。**
struct LogSection {
    /// 回転扉の通行証。**落とすまで次のテストは走らない**（区間が自分だけである根拠）。
    _pass: tokio::sync::MutexGuard<'static, ()>,
    buffer: &'static SharedBuffer,
    /// 入場した時点のバッファ長。
    mark: usize,
}

impl LogSection {
    /// 回転扉を通り、**この瞬間からの区間**を開く。
    async fn enter() -> Self {
        let pass = ONE_TEST_AT_A_TIME.lock().await;
        let buffer = shared_log();
        let mark = buffer.mark();
        Self {
            _pass: pass,
            buffer,
            mark,
        }
    }

    /// **自分が入ってから出た行だけ。** 肯定側の主張はこちらで見る。
    fn mine(&self) -> String {
        self.buffer.contents_since(self.mark)
    }

    /// バッファ全体。**否定側（「載っていないこと」）はこちらで見る。**
    ///
    /// 区間に狭めると「他のテストの回で漏れていた」を見逃す方向に**弱くなる**ので、
    /// 漏れの検査だけは全体を見る。
    fn everything(&self) -> String {
        self.buffer.contents()
    }
}

/// **区間の仕掛けそのものを見る。**
///
/// 下の2本は「同じラベルを出すテストが他にもある」場面でしか区間分けを観測できない
/// ——実測で、[`SharedBuffer::contents_since`] が区間を無視して全体を返すようにしても
/// 2本とも緑のまま通った。**仕掛けを直接見るテストが無いと、黙って外せてしまう**
/// （CLAUDE.md「無用に見える仕掛けを取り除くときは、近くの見張り番がまだ決定的かを
/// 測り直すこと」）。
///
/// **⚠ 「2本とも緑」は、その後に足したテストで**部分的に**古びた**
/// （2026-08-28 / B-3 で測り直し。**同じ変異で同じバイナリを6回**）。
///
/// | テスト | 6回のうち赤だった回 |
/// |---|---|
/// | このテスト（区間の仕掛けそのもの） | **6 / 6**（元からそのための1本） |
/// | `both_paths_record_the_parse_outcome_without_the_raw_output` | **0 / 6**（当時の記述どおり緑） |
/// | `a_failure_to_persist_is_recorded_as_an_error` | 回による（当時は緑） |
/// | 軸の漏れを見る2本（B-12b で足した） | 回による（当時は存在しなかった） |
///
/// **⚠ バイナリ全体の赤の本数は 2〜4 で、実行順に依存する**
/// （6回の実測: `2 / 3 / 4 / 4 / 2 / 4`）。**再現する数ではないので、本数で判断しないこと。**
///
/// **仕掛けを直接見る必要が消えたわけではない。**
/// 揺れる側は**どれも「他のテストの行を拾って期待値と違う」で落ちているだけ**で、
/// **区間そのものを主張していない。**
/// **ラベルの重なり方が変わらなくても、実行順が変わるだけで黙って緑に戻る。**
/// **決定的に赤くなるのはこの1本だけ。**
///
/// 見ているもの:
///
/// 1. 入場**前**に出た行は [`LogSection::mine`] に入らない
///    （＝他のテストの行では肯定側の主張が満たされない）。
/// 2. 入場**後**に出た行は入る（＝「いつも空」で 1. が成り立っているのではない）。
/// 3. 区間を開いている間は [`ONE_TEST_AT_A_TIME`] が塞がっている
///    （＝他のテストが同時に走って区間へ割り込むことがない）。
/// 4. [`LogSection::everything`] には入場前の行も入る（否定側が全体を見ている根拠）。
#[tokio::test]
async fn a_log_section_shows_only_the_lines_written_after_it_opened() {
    /// 入場**前**に出す目印。
    const BEFORE: &str = "この行は区間の外で出た";
    /// 入場**後**に出す目印。
    const INSIDE: &str = "この行は区間の中で出た";

    // **ここだけ [`shared_log`] を直に呼ぶ。** 入場前のイベントを出すには
    // subscriber が先に立っている必要があるため。
    let buffer = shared_log();
    assert_ne!(BEFORE, INSIDE, "前提: 2つの目印は別の文字列であること");

    // `target` を `personacore` にするのは、既定のフィルタ（`warn,personacore=info,…`）が
    // 他のターゲットの `info` を落とすため。
    tracing::info!(target: "personacore", "{BEFORE}");
    assert!(
        buffer.contents().contains(BEFORE),
        "前提: 入場前の行はバッファには入っていること（そもそも出ていないのではない）"
    );

    let log = LogSection::enter().await;
    tracing::info!(target: "personacore", "{INSIDE}");

    let mine = log.mine();
    assert!(mine.contains(INSIDE), "自分の行は区間に入ること: {mine:?}");
    assert!(
        !mine.contains(BEFORE),
        "入場前に出た行を区間に含めないこと（他のテストの行で主張が満たされる）: {mine:?}"
    );

    let everything = log.everything();
    assert!(
        everything.contains(BEFORE) && everything.contains(INSIDE),
        "全体には入場前の行も入ること（否定側はこちらで見る）: {everything:?}"
    );

    assert!(
        ONE_TEST_AT_A_TIME.try_lock().is_err(),
        "区間を開いている間は他のテストが走れないこと（区間が自分だけである根拠）"
    );
}

/// 応答の `model`（`aoi@<session-id>`）から着地したセッション id を取り出す。
fn session_id_of(body: &str) -> i64 {
    let value: Value = serde_json::from_str(body)
        .unwrap_or_else(|err| panic!("JSON として読めること: {err} / {body}"));
    let model = value["model"]
        .as_str()
        .unwrap_or_else(|| panic!("model は文字列であること: {body}"))
        .to_owned();
    let (character, session) = model
        .split_once('@')
        .unwrap_or_else(|| panic!("`名前@セッションid` の形であること: {model}"));
    assert_eq!(character, "aoi", "model = {model}");
    session
        .parse()
        .unwrap_or_else(|err| panic!("セッション id が整数であること: {err} / {model}"))
}

/// **プロセス全体の subscriber を1回だけ差し、共有のバッファを返す。**
///
/// 判定はサーバのタスク側で走るので、スレッドローカルな `with_default` では捕まらず
/// `set_global_default` が要る。**それはプロセスにつき1回しか成功しない**ので、
/// このバイナリのテストは1つのバッファを共有する（モジュールの doc を参照）。
///
/// **テストから直に呼ばないこと。** 呼ぶのは [`LogSection::enter`] で、
/// そちらが「自分の区間」を切る（共有の弊害はそこで潰してある）。
/// 例外は仕掛け自体の見張り番
/// （[`a_log_section_shows_only_the_lines_written_after_it_opened`]）だけ。
/// 差す前に出たイベントはどこにも残らない。
///
/// **⚠ 器も差し込みも、もう複製ではない**（作業単位 B-3）。
/// [`common::install_global_log`] が `src/testlog/assert.rs` の1本を読み、
/// **`OnceLock` は取り込んだバイナリごとに別の静的変数**になるので
/// 「このバイナリで1回だけ」はそちらが保証する。
/// **ここで写しを作り直さないこと**（押さえの見張り番はあちらにしか無い）。
fn shared_log() -> &'static SharedBuffer {
    install_global_log(personacore::server::DEFAULT_LOG_FILTER)
}

/// **両方の経路が、分解の結果を既定のログフィルタで記録する。**
///
/// **2つの経路を1つのテストにまとめてあるのは、経路ごとにラベルを分けるため。**
/// `structured` と `fallback_unclosed` に分かれるので、片方だけ記録が消えても赤になる。
///
/// バッファは [`shared_log`] 経由でバイナリ全体と共有しているが、
/// **見るのは [`LogSection`] が切った自分の区間だけ**なので、
/// 同じラベルを出すテストが他にあっても主張は満たされない
/// （実測: `fallback_unclosed` を出すテストをもう1本足すと、
/// 区間に閉じる前は中継側の記録を消す変異が緑を通り抜けた）。
#[tokio::test]
async fn both_paths_record_the_parse_outcome_without_the_raw_output() {
    let log = LogSection::enter().await;

    assert!(
        GOOD_REPLY.contains(SECRET) && BROKEN_REPLY.contains(SECRET),
        "前提: どちらの応答にも目印が入っていること"
    );

    let server = TestServer::start().await;

    // 1. 非ストリーミング。**正常に分解できる応答。**
    let body = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "教えて"}],
        }))
        .await;
    assert!(body.contains(GOOD_REPLY), "前提: 応答が返ること: {body}");

    // 2. 中継。**`「` が閉じない応答。**
    let body = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "やっぱり教えて"}],
            "stream": true,
        }))
        .await;
    assert!(
        body.contains("[DONE]"),
        "前提: 中継が完結すること（保存の手前まで進むこと）: {body}"
    );

    // **自分の区間だけ**を見る（他のテストが出した同じラベルの行では満たされない）。
    let logged = log.mine();

    // **非ストリーミング側。** ラベルは `ParseOutcome::label()` の定数。
    //
    // **欄で行を選ぶので [`line_with_field`]**（作業単位 B-21b）。
    // [`line_containing`] は素の部分一致なので、`x_outcome=structured` の行も
    // `outcome=structured_x` の行も掴む——**どちらも欄名・値を伸ばす変異が作る形。**
    let structured = line_with_field(&logged, "outcome=structured");
    assert!(
        structured.contains(PARSE_MARKER),
        "分解の記録であること: {structured:?}"
    );
    assert!(
        has_field(structured, "fell_back=false"),
        "成功率の分子になる値を載せること: {structured:?}"
    );

    // **中継側。** 崩れた回は理由まで残る。
    let broken = line_with_field(&logged, "outcome=fallback_unclosed");
    assert!(
        broken.contains(PARSE_MARKER),
        "分解の記録であること: {broken:?}"
    );
    assert!(
        has_field(broken, "fell_back=true"),
        "崩れた回を成功に数えないこと: {broken:?}"
    );

    // **生出力をログに載せない**（`label()` は定数を返す）。
    // 漏れの検査だけは**バッファ全体**を見る（区間に狭めると弱くなる）。
    let everything = log.everything();
    assert!(
        !everything.contains(SECRET),
        "LLM の出力をログへ垂れ流さないこと: {everything:?}"
    );

    server.stop().await;
}

/// **保存に失敗したら `error!` で記録する。** 応答は返すが、黙って消さない。
///
/// # なぜこの見張り番が要るか
///
/// `TurnSink::persist` は `Result` を返さない（返しても呼び出し側にできることが無い /
/// `server::turn` の doc）。つまり**この `error!` の1行が、
/// 「黙ったデータ欠落」に気付ける唯一の手段。** 実測で、この1行を
/// `let _ = err;` に置換しても **683/683 が緑のまま**通り抜けた。
/// 消えたことに誰も気付けないなら、持ち越しの「破棄率のメトリクス」も土台ごと失われる。
///
/// # 見ているもの
///
/// 1. **`ERROR` で出る**こと（`debug!` に落ちると既定のフィルタから消え、
///    `warn!` に落ちると重大さが伝わらない）。
/// 2. **どのセッションで落ちたか**が載ること（載っていないと調査の起点が無い）。
/// 3. **生出力も利用者の発言もログに載らない**こと（運用ログを会話の写しにしない）。
/// 4. **保存できなかった回でも、分解の結果が記録される**こと
///    （`server::turn` の「保存できてもできなくても記録する」/ CLAUDE.md 絶対制約）。
///    `error!` は「保存できなかった」しか言わないので、これが消えると
///    **分解できたのか崩れたのかが失われ、しかも消えるのはまさに調査したい回。**
///    成功率の分母から抜けた分だけ、実態より良い数字が出る。
///    保存に成功する回（[`SAVED_REPLY`]）と**別のラベル**を出させて名指ししている。
///
/// # セッションを3つ作る理由
///
/// **1つしか無いと `session_id` は必ず `1` になり、「解決したセッションを載せている」と
/// 「`1` を書いている」「最新を載せている」が区別できない**（CLAUDE.md「最頻の穴」）。
/// 3つ作って**真ん中**を明示すれば、載る値は先頭（1）とも最新（3）とも違う。
///
/// 応答が 200 で返ること自体は `tests/server_chat.rs` が見ている。
#[tokio::test]
async fn a_failure_to_persist_is_recorded_as_an_error() {
    let log = LogSection::enter().await;

    // 前提: 保存に成功する回と失敗する回で**ラベルが分かれる**こと。
    // 同じラベルだと、失敗した回の記録が消えても成功した回の行で主張が満たされる。
    let saved_label = parse_response(SAVED_REPLY).outcome.label();
    let unsaved_label = parse_response(UNSAVED_REPLY).outcome.label();
    assert_ne!(
        saved_label, unsaved_label,
        "前提: 保存できた回とできなかった回でラベルが分かれること"
    );

    let llm = MockLlm::replying(SAVED_REPLY);
    let server = TestServer::start_with(llm.clone()).await;

    // 前提: 細工の無い状態なら保存まで通る（この後の失敗が「元から壊れている」ではない）。
    // ついでに**セッションを3つ**作る（先頭・真ん中・最新が全部違う値になる）。
    let mut sessions = Vec::new();
    for turn in 1..=3 {
        let body = server
            .chat(&json!({
                "model": "aoi@new",
                "messages": [{"role": "user", "content": format!("まずは普通に {turn}")}],
            }))
            .await;
        assert!(body.contains(SAVED_REPLY), "前提: 応答が返ること: {body}");
        sessions.push(session_id_of(&body));
    }
    let [first, middle, latest] = sessions.as_slice() else {
        unreachable!("3回叩いた");
    };
    assert!(
        first != middle && middle != latest,
        "前提: 3つとも別のセッションであること: {sessions:?}"
    );

    // **上流の応答を返す直前に永続化層を閉じる**（保存だけが失敗する状況）。
    // 同時に、この回だけ**別のラベルになる応答**を返させる（記録を名指しするため）。
    llm.break_the_store_on_the_next_call(server.store.clone());
    llm.reply_once_with(UNSAVED_REPLY);

    // **真ん中**のセッションを明示する。先頭でも最新でもない値が載るはず。
    let body = server
        .chat(&json!({
            "model": format!("aoi@{middle}"),
            "messages": [{"role": "user", "content": UNSAVED_USER}],
        }))
        .await;
    let session = session_id_of(&body);
    assert_eq!(session, *middle, "前提: 真ん中に着地すること: {body}");
    assert!(
        body.contains(UNSAVED_REPLY),
        "前提: 保存に失敗した回でも応答は返ること（差し替えた本文が返ること）: {body}"
    );

    // **自分の区間だけ**を見る（他のテストの行では満たされない）。
    let logged = log.mine();

    // **こちらは文言で拾う**（`キー=値` ではないので [`line_containing`] のまま / B-21b）。
    let failure = line_containing(&logged, PERSIST_FAILURE_MARKER);
    // **深刻度も「丸ごとのトークン」で見る**（作業単位 B-21b。**申し送りの一覧の外で見つけた1件**）。
    //
    // **⚠ 机上の話ではない。実測してある**（2026-08-28）——
    // **`error!` を `warn!` へ下げ、同じイベントに `ERROR` を値に持つ欄を1本足す変異**は、
    // 素の `contains` では **1711 全緑**を通り抜けた（囮の欄がこの主張を満たす）。
    // [`has_field`] にすると **1 赤**（このテスト）。
    // **深刻度は `キー=値` ではないが、`tracing` の整形では空白で挟まれた1トークン**なので、
    // 「伸びた別の綴りに当たる」という同じ壊れ方をする。
    assert!(
        has_field(failure, "ERROR"),
        "保存できなかったことは ERROR で記録すること（下げると既定のフィルタから消える）: {failure:?}"
    );
    assert!(
        has_field(failure, &format!("session_id={session}")),
        "どのセッションで落ちたかを載せること（調査の起点）: {failure:?}"
    );

    // **保存できなかった回でも分解の結果を記録する**（成功率は保存の成否と別の話）。
    // この回だけ `fallback_unclosed`（= 保存に成功した回と別のラベル）なので、
    // 「成功した回の行」では満たせない。
    let outcome = line_with_field(&logged, &format!("outcome={unsaved_label}"));
    assert!(
        outcome.contains(PARSE_MARKER),
        "保存に失敗した回でも分解を記録すること: {outcome:?}"
    );
    assert!(
        has_field(outcome, "fell_back=true"),
        "崩れた回を成功に数えないこと: {outcome:?}"
    );
    assert!(
        has_field(outcome, &format!("session_id={session}")),
        "分解の記録にも落ちたセッションが載ること: {outcome:?}"
    );

    // **ログを会話の写しにしない。** 生出力も利用者の発言も載らないこと。
    // 漏れの検査だけは**バッファ全体**を見る（区間に狭めると弱くなる）。
    let everything = log.everything();
    assert!(
        !everything.contains(SAVED_REPLY) && !everything.contains(UNSAVED_REPLY),
        "LLM の出力をログへ垂れ流さないこと: {everything:?}"
    );
    assert!(
        !everything.contains(UNSAVED_USER),
        "利用者の発言をログへ垂れ流さないこと: {everything:?}"
    );

    // 永続化層を閉じたので `stop()` の期待（正常終了）はそのまま成立する。
    server.stop().await;
}

// ---- 軸名の漏れ（ADR-0018 / 作業単位 0011c）----

/// **地の文に漏れる軸名。** 台詞用・0 の軸と別の文字列。
const NARRATION_AXIS: &str = "ユーモア";

/// **台詞に漏れる軸名。** 地の文用と別の文字列（同じだと欄の取り違えが検出できない）。
const DIALOGUE_AXIS: &str = "ロジック";

/// **値 0 の軸名。** プロンプトに載らないので、応答に出ても数えない（ADR-0018）。
const ZERO_AXIS: &str = "パッション";

/// **どこにも漏れない軸名。** 「常に全軸を載せる」実装を緑にしないために要る。
const CLEAN_AXIS: &str = "勇気";

/// 軸名が**地の文と台詞の両方**に出る応答。0 の軸名も混ぜてある。
const LEAKY_REPLY: &str =
    "「ロジックだけじゃ足りないよ」 パッションを込めて、ユーモアたっぷりに笑った。";

/// 軸名が1つも出ない応答。**地の文だけ**（`narration_only`）。
const CLEAN_REPLY: &str = "（窓の外へ目を向けた。）";

/// [`LEAKY_REPLY`] のうち**軸名ではない**断片。ログに1文字も出てはいけない。
const LEAKY_REPLY_TEXT: &str = "だけじゃ足りないよ";

/// 軸名の漏れを問い合わせる発言。**ログに1文字も出てはいけない。**
const LEAKY_USER: &str = "その調子で話してみて";

/// **漏れたターンでも、往復2件がそのまま履歴に入っていること**（ADR-0018）。
///
/// # なぜこれが要るのか
///
/// ADR-0018 の中心的な宣言は「**検出は記録だけ。落とさない・書き換えない**」だが、
/// **その宣言だけが保存の側で無防備だった。** `server::turn` の
/// `the_detection_changes_nothing_that_gets_saved` は純粋関数 `turn_messages` しか通らず、
/// このバイナリの2本はログ行しか見ていなかったので、
/// **`TurnSink::persist` に「漏れたターンは保存しない」1行を足す変異**は
/// **839/839 緑**で通り抜けた（実測）。後日「抑え込み」を入れた瞬間に
/// **軸名が漏れたターンだけが履歴から静かに消える。**
///
/// 見るのは `raw_content`（絶対制約）で、**送った生出力と1バイトも違わないこと。**
fn assert_the_turn_was_saved_untouched(saved: &[Message], user_message: &str, raw_reply: &str) {
    assert_ne!(
        user_message, raw_reply,
        "前提: 発言と応答は別の文字列であること（取り違えが検出できなくなる）"
    );
    assert_eq!(
        saved.len(),
        2,
        "漏れたターンも往復2件がそのまま入ること: {saved:?}"
    );
    assert_eq!(saved[0].role, Role::User, "先に利用者の発言が入ること");
    assert_eq!(
        saved[0].raw_content, user_message,
        "利用者の発言を落とさないこと"
    );
    assert_eq!(saved[1].role, Role::Assistant, "後に応答が入ること");
    assert_eq!(
        saved[1].raw_content, raw_reply,
        "生出力を丸ごと残すこと（絶対制約 / 書き換えないこと）"
    );
}

/// **本番の入口（`POST /v1/chat/completions`）を通しても、軸名の漏れが同じ1行に載る。**
///
/// # ここでしか見られないもの
///
/// `server::turn` の単体テストは [`log_outcome`] を直に呼ぶので、
/// **「キャラクターの軸が `TurnSink` まで届いていること」は誰も見ていない。**
/// `stat_allocs` の結果を捨てて `Vec::new()` を渡す変異は、
/// 単体テストだけでは全部緑で通る（CLAUDE.md「本番の入口が観測対象から外れる」）。
///
/// # 見ているもの
///
/// 1. 漏れたターンで、**地の文と台詞が別々の欄**に載る（値 0 の軸は載らない）。
/// 2. 漏れが無いターンでも**欄が出る**（`[]`。欠落と 0 件を区別できること）。
/// 3. **応答本文も利用者の発言もログに出ない**（載るのは軸名だけ）。
/// 4. **漏れたターンがそのまま保存されている**
///    （[`assert_the_turn_was_saved_untouched`]。ADR-0018「記録だけ」の実体）。
///
/// 2つのターンは**別のラベル**（`structured` / `narration_only`）を出すので、
/// 片方の行でもう片方の主張が満たされることはない。
///
/// [`log_outcome`]: personacore
#[tokio::test]
async fn the_chat_path_records_axis_leaks_in_the_same_line() {
    let log = LogSection::enter().await;

    // 前提: フィクスチャの語がすべて別であること（同値だと欄の取り違えを検出できない）。
    let axes = [NARRATION_AXIS, DIALOGUE_AXIS, ZERO_AXIS, CLEAN_AXIS];
    for (index, left) in axes.iter().enumerate() {
        for right in &axes[index + 1..] {
            assert_ne!(left, right, "前提: 軸名は互いに別であること");
        }
    }
    assert_ne!(LEAKY_REPLY, CLEAN_REPLY, "前提: 2つの応答は別であること");
    assert!(
        LEAKY_REPLY.contains(NARRATION_AXIS)
            && LEAKY_REPLY.contains(DIALOGUE_AXIS)
            && LEAKY_REPLY.contains(ZERO_AXIS),
        "前提: 3つの軸名が現に応答へ出ること"
    );
    assert!(
        !CLEAN_REPLY.contains(NARRATION_AXIS) && !CLEAN_REPLY.contains(DIALOGUE_AXIS),
        "前提: 綺麗な応答には軸名が無いこと"
    );

    let llm = MockLlm::replying(LEAKY_REPLY);
    let server = TestServer::start_with(llm.clone()).await;
    // **先頭は漏れない軸**（「先頭の軸しか見ない」実装を緑にしない）。
    // **末尾は漏れる軸**（「最後の軸を見ない」実装を緑にしない / `give_stats` の doc）。
    server
        .give_stats(&[
            (CLEAN_AXIS, 4),
            // **値 0 の軸**。プロンプトに載らないので、応答に出ても数えない。
            // **中間に置く**（末尾に置くと「末尾を見ない」実装と区別が付かない）。
            (ZERO_AXIS, 0),
            (NARRATION_AXIS, 5),
            (DIALOGUE_AXIS, 2),
        ])
        .await;

    // 1. 漏れるターン。
    let body = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": LEAKY_USER}],
        }))
        .await;
    assert!(body.contains(LEAKY_REPLY), "前提: 応答が返ること: {body}");
    let leaky_session = session_id_of(&body);

    // **検出は保存を1件も変えない**（ADR-0018）。**次のターンを送る前に**読む
    // ——このセッションに入っているのは、いま漏れたターンの2件だけ。
    assert_the_turn_was_saved_untouched(
        &server.saved_messages(leaky_session).await,
        LEAKY_USER,
        LEAKY_REPLY,
    );

    // 2. 漏れないターン（**同じキャラクター＝同じ軸**で、欄が空になること）。
    llm.reply_once_with(CLEAN_REPLY);
    let body = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "そろそろ休もう"}],
        }))
        .await;
    assert!(body.contains(CLEAN_REPLY), "前提: 応答が返ること: {body}");

    let logged = log.mine();

    // **漏れたターン。** 地の文と台詞が別の欄に、別の軸名で載る。
    // **欄で行を選ぶ**（作業単位 B-21b / `both_paths_…` と同じ理由）。
    let leaked = line_with_field(&logged, "outcome=structured");
    assert!(
        leaked.contains(PARSE_MARKER),
        "分解の記録であること: {leaked:?}"
    );
    assert!(
        has_field(leaked, &format!("session_id={leaky_session}")),
        "同じ1行にセッションも載ること: {leaked:?}"
    );
    // **欄は「`キー=` に続く閉じ括弧まで」を1つの値として読む**（作業単位 B-12b）。
    // **`contains` では締まらない**——実測で、`log_outcome` の欄名に接頭辞を足す変異
    // （`x_axis_leak_narration=…`）は**この2本を緑のまま通り抜けた**（部分文字列として当たる）。
    // **`has_field` も使えない**（値に空白が入りうる。軸名は中間の空白を保ち、
    // 2軸漏れれば `["A", "B"]` になる）。**期待値はテスト側の綴りで組み立てる。**
    assert_eq!(
        bracketed_field(leaked, "axis_leak_narration"),
        Some(format!("[{NARRATION_AXIS:?}]").as_str()),
        "地の文の漏れを地の文の欄に載せること: {leaked:?}"
    );
    assert_eq!(
        bracketed_field(leaked, "axis_leak_dialogue"),
        Some(format!("[{DIALOGUE_AXIS:?}]").as_str()),
        "台詞の漏れを台詞の欄に載せること: {leaked:?}"
    );

    // **漏れが無いターンでも欄は出る**（`[]`）。
    let clean = line_with_field(&logged, "outcome=narration_only");
    assert!(
        clean.contains(PARSE_MARKER),
        "分解の記録であること: {clean:?}"
    );
    // **0 件のときも [`bracketed_field`] で読む**（作業単位 B-21b）。
    //
    // **どちらの道具でも読めて、検出力は等価。** 理由は [`bracketed_field`] の doc
    // （B-21b の節）に**1本だけ**置いてある。**ここに書き写さないこと。**
    // こちらが [`bracketed_field`] なのは、**同じテストの1件ある側**（B-12b が締めた2本）に
    // 揃えるため。
    //
    // **⚠ この2本の「自分ぶんの検出力」は、隣の assert に隠れて見えない**
    // （B-21b のレビューが実測）。**欄名を伸ばす変異は上の1件ある側が先に落とす**ので、
    // **スイートの合否はこの2本が有っても無くても変わらない**（CLAUDE.md
    // 「第4の分類 —— 権威側にマスクされた変異」）。**単離して測るには、**
    // **空のときだけ綴りが変わる変異**（`[]` → `[]x`）**を当てること。**
    // 着手前の素の `contains` はそれを**見逃していた**（部分一致で真になる）。
    //
    // **`if let` で畳まず `Some(..)` と比べる**ので、**欄が丸ごと消えた変異は
    // `None` になって必ず赤**（あちらの doc「呼び出し側は `Some(期待値)` と比べること」）。
    assert_eq!(
        bracketed_field(clean, "axis_leak_narration"),
        Some("[]"),
        "漏れが無いターンでも地の文の欄を出すこと（欠落と 0 件を区別できるように）: {clean:?}"
    );
    assert_eq!(
        bracketed_field(clean, "axis_leak_dialogue"),
        Some("[]"),
        "漏れが無いターンでも台詞の欄を出すこと（欠落と 0 件を区別できるように）: {clean:?}"
    );

    // **値 0 の軸はどこにも出ない**（プロンプトに載っていない＝モデルは読んでいない）。
    // **応答本文と利用者の発言も出ない。** 否定側は**バッファ全体**を見る。
    let everything = log.everything();
    assert!(
        !everything.contains(ZERO_AXIS),
        "プロンプトに載らなかった軸を数えないこと: {everything:?}"
    );
    assert!(
        !everything.contains(CLEAN_AXIS),
        "漏れていない軸を載せないこと: {everything:?}"
    );
    assert!(
        !everything.contains(LEAKY_REPLY_TEXT) && !everything.contains(CLEAN_REPLY),
        "LLM の出力をログへ垂れ流さないこと: {everything:?}"
    );
    assert!(
        !everything.contains(LEAKY_USER),
        "利用者の発言をログへ垂れ流さないこと: {everything:?}"
    );

    server.stop().await;
}

/// 中継で流すチャンク。**地の文と台詞に別々の軸名**が出る（分割して流す）。
const STREAMED_LEAK: [&str; 2] = [
    "ユーモアたっぷりに笑って、",
    "「ロジックだけじゃ足りないよ」",
];

/// [`STREAMED_LEAK`] のうち**軸名ではない**断片。ログに1文字も出てはいけない。
const STREAMED_LEAK_TEXT: &str = "たっぷりに笑って";

/// 中継のターンの発言。**保存された往復と突き合わせる**ので定数で持つ。
const STREAMED_USER: &str = "中継で話して";

/// **中継（SSE）経路でも、軸名の漏れが同じ1行に載る。**
///
/// # なぜ非ストリーミングと別に要るのか
///
/// **配線が2箇所ある。** `server::chat` は経路ごとに `TurnSink` を組み立てるので、
/// 片方だけ軸を渡し忘れても、もう片方のテストは緑のまま通る。
/// 実測で、中継側だけ `Vec::new()` を渡す変異は
/// **このテストを足すまで 838/838 緑**で通り抜けた（非ストリーミング側の同じ変異は赤）。
#[tokio::test]
async fn the_streaming_path_also_records_axis_leaks() {
    let log = LogSection::enter().await;

    let streamed: String = STREAMED_LEAK.concat();
    assert!(
        streamed.contains(NARRATION_AXIS) && streamed.contains(DIALOGUE_AXIS),
        "前提: 中継する本文に2つの軸名が出ること"
    );

    let server = TestServer::start_with(MockLlm::streaming(&STREAMED_LEAK)).await;
    // **末尾は漏れる軸、0 の軸は中間**（非ストリーミング側と同じ理由 / `give_stats` の doc）。
    server
        .give_stats(&[
            (CLEAN_AXIS, 4),
            (ZERO_AXIS, 0),
            (NARRATION_AXIS, 5),
            (DIALOGUE_AXIS, 2),
        ])
        .await;

    let body = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": STREAMED_USER}],
            "stream": true,
        }))
        .await;
    assert!(
        body.contains("[DONE]"),
        "前提: 中継が完結すること（保存の手前まで進むこと）: {body}"
    );

    // **中継経路でも、検出は保存を1件も変えない**（ADR-0018）。
    // **配線が2本あるので、非ストリーミング側と別に見る必要がある**
    // ——`TurnSink::persist` に「漏れたターンは保存しない」を足す変異は両方を赤にするが、
    // 片方にしか検査が無ければ、その片方を外すだけで穴が戻る。
    assert_the_turn_was_saved_untouched(
        &server.saved_messages(session_id_of_sse(&body)).await,
        STREAMED_USER,
        &streamed,
    );

    let logged = log.mine();
    // **ここは文言で拾う**（作業単位 B-21b）。この区間で分解の記録は1行だけなので、
    // **区別すべきラベルが無い**——手掛かりが `キー=値` でないなら [`line_containing`]。
    let line = line_containing(&logged, PARSE_MARKER);
    // **非ストリーミング側と同じ読み方**（作業単位 B-12b / 上のテストの comment）。
    assert_eq!(
        bracketed_field(line, "axis_leak_narration"),
        Some(format!("[{NARRATION_AXIS:?}]").as_str()),
        "中継経路でも地の文の漏れを載せること: {line:?}"
    );
    assert_eq!(
        bracketed_field(line, "axis_leak_dialogue"),
        Some(format!("[{DIALOGUE_AXIS:?}]").as_str()),
        "中継経路でも台詞の漏れを載せること: {line:?}"
    );
    assert!(
        has_field(line, &format!("session_id={}", session_id_of_sse(&body))),
        "同じ1行にセッションも載ること: {line:?}"
    );

    // **応答本文と 0 の軸はログに出ない。** 否定側は**バッファ全体**を見る。
    let everything = log.everything();
    assert!(
        !everything.contains(STREAMED_LEAK_TEXT),
        "LLM の出力をログへ垂れ流さないこと: {everything:?}"
    );
    assert!(
        !everything.contains(ZERO_AXIS) && !everything.contains(CLEAN_AXIS),
        "載らなかった軸・漏れていない軸を載せないこと: {everything:?}"
    );

    server.stop().await;
}

/// SSE の本文から、着地したセッション id を取り出す。
///
/// 中継のチャンクは `data: {...}` の形で、どれも同じ `model`（`aoi@<session-id>`）を持つ
/// （`server::relay` の `Relay`）。**最初の1つで足りる。**
fn session_id_of_sse(body: &str) -> i64 {
    let payload = body
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .find(|payload| payload.contains("\"model\""))
        .unwrap_or_else(|| panic!("model を持つチャンクがあること: {body}"));
    session_id_of(payload)
}

/// **強調の印を含む応答**（作業単位 B-46 / C-18 が実機で観測した形）。
///
/// **台詞の中と外の両方**に置いてある。片方だけだと、
/// 「台詞には落とすが地の文には落とさない」実装が緑を通る。
const EMPHASISED_REPLY: &str = "「**朝**がいいよ」と**笑った**。";

/// [`EMPHASISED_REPLY`] から落ちる対の数。**手で数えた値**（実装から作らない）。
const EMPHASISED_PAIRS: usize = 2;

/// 強調の印。**この2文字が本文に残ってはいけない。**
const EMPHASIS_MARK: &str = "**";

/// 強調の印を含む発言。**利用者の側は1文字も書き換えない。**
const EMPHASISED_USER: &str = "**朝**と**夜**、どっちがいい";

/// **生出力は印ごと残り、読み手に届く本文からは印が落ちている**（作業単位 B-46）。
///
/// # ここでしか見られないもの
///
/// `parse` の単体テストも `tests/emphasis_corpus.rs` も、**保存の側を通らない。**
/// **`raw_content` に落とした後の本文を入れる**変異——`server::turn` の
/// `raw_content: raw_reply.to_owned()` を分解結果から作る形——は、
/// **保存を読むテストでしか捕まらない**（CLAUDE.md 絶対制約
/// 「`message.raw_content` を必ず保持する」）。
///
/// # 見ているもの
///
/// 1. **`raw_content` が送った生出力と1バイトも違わない**（印が残っている）。
/// 2. **`dialogue` にも `narration` にも印が無い**（台詞の中と外の両方）。
/// 3. **利用者の発言は 1 文字も変わらない**——`turn_messages` は利用者側を
///    パースしない（`server::turn` の doc）ので、印もそのまま残る。
/// 4. **落とした対の数が、分解の記録と同じ1行に載る**（画面には出さない）。
#[tokio::test]
async fn the_raw_output_keeps_the_emphasis_that_the_saved_body_lost() {
    let log = LogSection::enter().await;

    // 前提: 応答にも発言にも印が現にあり、互いに別の文字列であること。
    assert!(
        EMPHASISED_REPLY.contains(EMPHASIS_MARK) && EMPHASISED_USER.contains(EMPHASIS_MARK),
        "前提: どちらにも印があること"
    );
    assert_ne!(
        EMPHASISED_REPLY, EMPHASISED_USER,
        "前提: 発言と応答は別の文字列であること"
    );

    let llm = MockLlm::replying(EMPHASISED_REPLY);
    let server = TestServer::start_with(llm.clone()).await;

    let body = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": EMPHASISED_USER}],
        }))
        .await;
    // **クライアントへは印ごとそのまま返る**（`server::chat` の
    // 「応答は上流から受け取ったまま返す」）。**落とすのは保存と画面の側。**
    assert!(
        body.contains(EMPHASISED_REPLY),
        "上流の応答をそのまま返すこと: {body}"
    );
    let session = session_id_of(&body);

    let saved = server.saved_messages(session).await;
    assert_eq!(saved.len(), 2, "往復2件が入ること: {saved:?}");

    // 1. 利用者の発言。**パースしないので印もそのまま。**
    assert_eq!(saved[0].role, Role::User);
    assert_eq!(
        saved[0].raw_content, EMPHASISED_USER,
        "利用者の発言を書き換えないこと"
    );

    // 2. 応答。**生出力は印ごと残る**（絶対制約）。
    assert_eq!(saved[1].role, Role::Assistant);
    assert_eq!(
        saved[1].raw_content, EMPHASISED_REPLY,
        "生出力を丸ごと残すこと（絶対制約 / 印も残る）"
    );

    // 3. **読み手に届く側からは落ちている。** 台詞と地の文を別々に見る。
    let dialogue = saved[1]
        .dialogue
        .as_deref()
        .unwrap_or_else(|| panic!("台詞が入ること: {saved:?}"));
    let narration = saved[1]
        .narration
        .as_deref()
        .unwrap_or_else(|| panic!("地の文が入ること: {saved:?}"));
    assert!(
        !dialogue.contains(EMPHASIS_MARK),
        "台詞から印が落ちていること: {dialogue:?}"
    );
    assert!(
        !narration.contains(EMPHASIS_MARK),
        "地の文から印が落ちていること: {narration:?}"
    );
    // **落としたのは印だけ**（本文まで削っていないこと）。
    assert_eq!(dialogue, "朝がいいよ", "台詞の本文: {dialogue:?}");
    assert_eq!(narration, "と笑った。", "地の文の本文: {narration:?}");

    // 4. **対の数が、分解の記録と同じ1行に載る**（欄で行を選ぶ / 作業単位 B-21b）。
    let logged = log.mine();
    let line = line_with_field(&logged, &format!("session_id={session}"));
    assert!(
        line.contains(PARSE_MARKER),
        "分解の記録であること: {line:?}"
    );
    assert!(
        has_field(line, &format!("emphasis_pairs_stripped={EMPHASISED_PAIRS}")),
        "同じ1行に対の数が載ること: {line:?}"
    );
    // **抽出側の欄名を名乗らないこと**（レビュー1周目の指摘）。
    // `server::extract` の `emphasis_stripped` は**落とした行数**で、こちらは**対の数**。
    // **どちらの行も `session_id` を持つ**ので、同じ綴りだと集計で1つの列に混ざる。
    // **値を見ない `has_key` で言う**——値が違う回に黙る形にしないため。
    assert!(
        !has_key(line, "emphasis_stripped"),
        "抽出側の欄名を本文の記録に出さないこと: {line:?}"
    );

    server.stop().await;
}

/// **思考2つと余った閉じタグを持つ応答**（C-20 の B の turn 32 の形を縮めたもの / 作業単位 B-61）。
const THINKING_REPLY: &str = "<think >First thought.</think >\n\n<think >Second thought.</think >\n</think>\n\n「おかえり」と彼女は笑った。";

/// [`THINKING_REPLY`] から剥がれる思考のブロックの数。**手で数えた値**（実装から作らない）。
const THINKING_BLOCKS: usize = 2;

/// 思考の中身の断片。**台詞にも地の文にも、ログにも出てはいけない。**
const THOUGHT_FRAGMENT: &str = "thought.";

/// **タグを書いた利用者の発言。** 利用者の側は1文字も剥がさない。
const THINKING_USER: &str = "<think>は何のタグ？</think>";

/// **保存は思考込み、分解は思考なし、数は同じ1行**——同じ回で見る（ADR-0053 §決定4・5 / 作業単位 B-61）。
///
/// `server::turn` の単体テストは `turn_messages` を直に呼ぶので、
/// **`TurnSink::persist` が剥がす前の生出力を保存に渡しているか**はここでしか見えない。
#[tokio::test]
async fn the_raw_output_keeps_the_thought_that_the_saved_body_lost() {
    let log = LogSection::enter().await;

    let llm = MockLlm::replying(THINKING_REPLY);
    let server = TestServer::start_with(llm.clone()).await;

    let body = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": THINKING_USER}],
        }))
        .await;
    let session = session_id_of(&body);
    let saved = server.saved_messages(session).await;
    assert_eq!(saved.len(), 2, "往復2件が入ること: {saved:?}");

    // 1. 利用者の発言は剥がさない。
    assert_eq!(saved[0].raw_content, THINKING_USER);

    // 2. 応答の生出力は思考込み（絶対制約）。
    assert_eq!(
        saved[1].raw_content, THINKING_REPLY,
        "生出力を丸ごと残すこと（思考も残る）"
    );

    // 3. 分解した本文は思考なし。
    assert_eq!(saved[1].dialogue.as_deref(), Some("おかえり"));
    assert_eq!(saved[1].narration.as_deref(), Some("と彼女は笑った。"));

    // 4. 数が分解の記録と同じ1行に載る。思考の中身はログに出ない。
    let logged = log.mine();
    let line = line_with_field(&logged, &format!("session_id={session}"));
    assert!(
        line.contains(PARSE_MARKER),
        "分解の記録であること: {line:?}"
    );
    assert!(
        has_field(line, &format!("thinking_blocks_stripped={THINKING_BLOCKS}")),
        "剥がしたブロックの数が同じ1行に載ること: {line:?}"
    );
    assert!(
        has_field(line, "thinking_unclosed=false"),
        "閉じなかったかが同じ1行に載ること: {line:?}"
    );
    assert!(
        has_field(line, "outcome=structured"),
        "剥がした本文で分解すること: {line:?}"
    );
    assert!(
        !log.everything().contains(THOUGHT_FRAGMENT),
        "思考の中身をログに出さないこと"
    );

    server.stop().await;
}

/// **中継でも、チャンクをまたいだタグを剥がしてから分解する**（作業単位 B-61）。
///
/// 保存は中継が蓄積した生出力全体に対して行う（`server::relay`）ので、
/// タグが `<thi` / `nk>` に割れて届いても規則は1本の文字列を見る。
/// **SSE で使い手へ流す本文は剥がさない**（ADR-0053 §決定6）。
#[tokio::test]
async fn the_streaming_path_strips_a_thought_split_across_chunks() {
    let log = LogSection::enter().await;

    let llm = MockLlm::streaming(&["<thi", "nk>\nOnly a thought.\n</thi", "nk >"]);
    let server = TestServer::start_with(llm.clone()).await;

    let body = server
        .chat(&json!({
            "model": "aoi",
            "messages": [{"role": "user", "content": "ただいま"}],
            "stream": true,
        }))
        .await;
    assert!(body.contains("[DONE]"), "前提: 中継が完結すること: {body}");
    assert!(
        body.contains("Only a thought."),
        "前提: SSE は上流のまま流すこと: {body}"
    );
    let session = session_id_of_sse(&body);
    let saved = server.saved_messages(session).await;
    assert_eq!(saved.len(), 2, "往復2件が入ること: {saved:?}");
    assert_eq!(
        saved[1].raw_content, "<think>\nOnly a thought.\n</think >",
        "蓄積した生出力を丸ごと残すこと"
    );
    assert_eq!(saved[1].dialogue, None);
    assert_eq!(
        saved[1].narration, None,
        "思考だけの回は空として分解すること"
    );

    // **中継は `SSE 中継を開始します` の行も `session_id` を持つ**ので、文言で行を選び、
    // セッションは同じ行の欄で確かめる（`the_streaming_path_also_records_axis_leaks` と同じ）。
    let logged = log.mine();
    let line = line_containing(&logged, PARSE_MARKER);
    assert!(
        has_field(line, &format!("session_id={session}")),
        "{line:?}"
    );
    assert!(has_field(line, "outcome=empty"), "{line:?}");
    assert!(has_field(line, "thinking_blocks_stripped=1"), "{line:?}");
    assert!(has_field(line, "thinking_unclosed=false"), "{line:?}");

    server.stop().await;
}
