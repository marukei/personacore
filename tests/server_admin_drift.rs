//! `GET /admin/characters/{handle}/drift` の検証
//! （[ADR-0024](../docs/adr/0024-stat-drift.md) §9 / 作業単位 D-4a）。
//!
//! **LM Studio には繋がない。** 管理 API は上流を必要としない。
//! **サーバは本番の入口（`App::bind`）から起動する**——テスト専用の入口を作ると、
//! 本番の配線が誰にも通られないまま残る（CLAUDE.md「テスト専用の入口を作ると…」）。
//!
//! # なぜこの入口が要るのか
//!
//! **変容は本文にもユーザーにも見せない**（ADR-0024 §9）ので、
//! **票と累積はログを読まないと一切見えない。**
//! `GET .../stats` が返すのは「いまの値」だけで、
//! **変容したのか人が `PUT .../stats` で設定したのかを区別できない。**
//!
//! | 見るもの | 何が壊れたら赤になるか |
//! |---|---|
//! | 票が**このキャラクターのものだけ**返る | どれかの文から `character_id` の絞り込みが消える |
//! | 票の `session_id` が**その行のもの** | 持ち主と往復の取り違え |
//! | 軸ごとの**上げ・下げ**が入れ替わらない | 向きの取り違え |
//! | **新しい順**（同着は `id` の降順） | 並びの反転・第2キーの脱落 |
//! | **上限まで**返し、**総数は全部**を数える | 上限の変更／総数を返した件数にすり替える |
//! | **累積の行が無い軸**も 0 として返る | 内部結合にする／落とす |
//! | **値 0 の軸**も返る | `prompt` 層の都合をここへ持ち込む |
//! | 経路は `GET` **1本だけ** | 書き込みの口・1件を指すパスが生える |
//! | **1行でも読めなければ 500**（総数と蓄積は止まらない） | 読み飛ばしに変える |
//!
//! # 上限そのものの権威はここ
//!
//! 上限 `N` は `src/server/admin.rs` の `RECENT_DRIFT_VOTES` にあるが、
//! **期待値をその定数から作らない**（`N` を倍にする変異が緑を通る / CLAUDE.md `16b`）。
//! `N` より多い票を入れて、**外の値（直書きの 50 と 63）**で押さえる。

use std::net::SocketAddr;

use chrono::{DateTime, Utc};
use personacore::domain::{
    CharacterId, DriftDelta, DriftOutcome, DriftProposal, DriftSuggestion, DriftVote, Handle,
    SessionId, StatAlloc, StatAxis, StatDriftMode,
};
use personacore::server::{App, AppConfig, BindAddr, ENV_STAT_DRIFT, ServerError};
use personacore::store::{Store, StoreError};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

mod common;

use common::{
    fixture_scale, insert_drift_charge_at, insert_drift_vote_ignoring_checks,
    insert_stat_alloc_directly,
};

/// 変容を見るキャラクターの識別子。
const HANDLE: &str = "aoi";

/// その呼び名。**識別子とは別の綴り**（ADR-0011）。
const DISPLAY_NAME: &str = "葵";

/// **票が立たない**キャラクターの識別子。
///
/// 1体しか作らないと `character.id` と `1` が常に同値になり、
/// **持ち主を決め打ちにする実装が原理的に観測できない**（CLAUDE.md）。
const BYSTANDER_HANDLE: &str = "kaede";

/// 傍観者の呼び名。
const BYSTANDER_DISPLAY_NAME: &str = "楓";

/// 値も累積も票も持つ軸。**傍観者と綴りを共有する唯一の軸**
/// ——共有していないと、集計から持ち主の絞り込みが消えても
/// 「知らない軸が増える」だけになり、**軸の一覧には現れないので観測できない。**
const AXIS_PASSION: &str = "パッション";

/// **値が 0 の軸。** 累積も票も持つ（落とすと3方向から赤になる）。
const AXIS_LOGIC: &str = "ロジック";

/// **累積の行が無い軸**（一度も適用まで届いていない）。票はある。
const AXIS_CHEER: &str = "陽気さ";

/// **票が1件も無い軸。** 累積はある。
const AXIS_CARE: &str = "気配り";

/// 傍観者だけが持つ軸。
const BYSTANDER_AXIS: &str = "落ち着き";

/// **もう軸として保存されていない綴り。** 票だけが残っている。
const RETIRED_AXIS: &str = "むかしの軸";

/// **割り当てを失い、累積だけが残っている軸**（作業単位 B-16）。
///
/// `PUT .../stats` は全置換なので、軸を改名すると古い綴りの
/// `stat_drift_charge` / `stat_drift_vote` の行が置き去りになる。
///
/// **名前は `RETIRED_AXIS` より前に並ぶ**（`き` U+304D < `む` U+3080）。
/// 並びの見張り番が、届いた順ではなく名前で並べていることを見るために要る。
const LOST_CHARGED_AXIS: &str = "きえた軸";

/// **割り当てを失い、累積も票も残っている軸**（作業単位 B-16）。
///
/// **名前は `RETIRED_AXIS` より後**（`わ` U+308F > `む` U+3080）。
/// 累積の読みは `LOST_CHARGED_AXIS` と一緒に来るので、
/// **届いた順（累積 → 票）で並べる実装は、この軸が2番目に来て赤になる。**
const LOST_BOTH_AXIS: &str = "わすれた軸";

/// **傍観者だけが持つ、割り当ての無い軸**（累積も票もある / 作業単位 B-16）。
///
/// 名前は葵の3つの捨て軸の**真ん中**に落ちる（`し` U+3057）ので、
/// どれかの読みから持ち主の絞り込みが消えると
/// **葵の一覧の途中に現れて**、完全一致の突き合わせが赤になる。
const BYSTANDER_LOST_AXIS: &str = "しずけさ";

/// エフェメラルポートで起動した実サーバ。
struct TestServer {
    addr: SocketAddr,
    /// アプリと**同じプールを指す**ハンドル。票と累積の仕込みに使う。
    store: Store,
    /// DB ファイルの絶対パス。エラーレスポンスに漏れていないことの確認に使う。
    db_path: String,
    client: reqwest::Client,
    shutdown: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), ServerError>>>,
    _dir: TempDir,
}

impl TestServer {
    /// **本番の入口（`App::bind`）から起動する。** 走らせ方は**既定（`off`）。**
    async fn start() -> Self {
        Self::start_with_mode(None).await
    }

    /// **走らせ方を指定して**本番の入口から起動する（作業単位 D-5a）。
    ///
    /// `None` なら未設定（＝既定の `off`）。
    /// **綴りは環境変数へ書く形そのまま**を渡す——`StatDriftMode` を先に組んで渡すと、
    /// **設定の読み取り（`parse_stat_drift`）を飛ばした配線**しか見られない。
    ///
    /// **プロセスの環境変数は触らない**（`AppConfig::from_lookup` は参照関数を取る）。
    /// 触ると、並行して走る他のテストに漏れる。
    async fn start_with_mode(stat_drift: Option<&str>) -> Self {
        let dir = tempfile::tempdir().expect("一時ディレクトリを作成できること");
        let db_path = dir.path().join("personacore.sqlite");

        let config = AppConfig::from_lookup(|key| {
            if key == ENV_STAT_DRIFT {
                return stat_drift.map(str::to_owned);
            }
            None
        })
        .expect("設定を組み立てられること")
        .with_bind(BindAddr::loopback(0))
        .with_database_path(&db_path);

        let app = App::bind(&config).await.expect("待ち受けを開始できること");
        let addr = app.local_addr();
        let store = app.store().clone();

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
            db_path: db_path.display().to_string(),
            client: reqwest::Client::new(),
            shutdown: Some(shutdown),
            handle: Some(handle),
            _dir: dir,
        }
    }

    async fn send(&self, method: Method, path: &str, body: Option<&Value>) -> (StatusCode, Value) {
        let mut request = self
            .client
            .request(method, format!("http://{}{path}", self.addr));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.expect("サーバへ接続できること");
        let status = response.status();
        let text = response.text().await.expect("本文を読めること");
        let value = serde_json::from_str(&text)
            .unwrap_or_else(|err| panic!("応答が JSON であること: {text}: {err}"));
        (status, value)
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send(Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        self.send(Method::POST, path, Some(body)).await
    }

    async fn put(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        self.send(Method::PUT, path, Some(body)).await
    }

    /// キャラクターを1件作る。**識別子と表示名は違う文字列。尺度は既定と違う値。**
    async fn create_character(&self, handle: &str, display_name: &str) -> CharacterId {
        assert_ne!(handle, display_name, "識別子と表示名を同値にしないこと");
        let (status, body) = self
            .post(
                "/admin/characters",
                &json!({
                    "handle": handle,
                    "display_name": display_name,
                    "base_persona": "ペルソナ本文",
                    // **既定（10）にしない。** 既定だと、尺度を捨てて
                    // `StatScale::DEFAULT` を書く実装と区別が付かない。
                    "stat_scale": fixture_scale().get(),
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "作成できること: {body}");
        self.character_id(handle).await
    }

    /// キャラクターの主キー（票を仕込むために要る）。
    async fn character_id(&self, raw: &str) -> CharacterId {
        let handle = Handle::parse(raw).expect("テストで使う識別子は正当であること");
        self.store
            .find_character_by_handle(&handle)
            .await
            .expect("引けること")
            .expect("作成済みであること")
            .id
    }

    /// 軸と値を置く（**本番の入口** `PUT .../stats`）。
    async fn put_stats(&self, handle: &str, axes: &[(&str, u32)]) {
        let stats: Vec<Value> = axes
            .iter()
            .map(|(axis, value)| json!({"axis": axis, "value": value}))
            .collect();
        let (status, body) = self
            .put(&stats_path(handle), &json!({"stats": stats}))
            .await;
        assert_eq!(status, StatusCode::OK, "軸を置けること: {body}");
    }

    /// **票を1件、時刻を指定して仕込む。** 行の id を返す。
    async fn plant_vote(
        &self,
        character: CharacterId,
        session: SessionId,
        axis: &str,
        delta: DriftDelta,
        created_at: &str,
    ) -> i64 {
        insert_drift_vote_ignoring_checks(
            &self.store,
            character.get(),
            session.get(),
            axis,
            delta.label(),
            created_at,
        )
        .await
    }

    /// 累積を**本番の書き込み口**で置く（許可証が要る）。
    async fn save_charges(&self, character: CharacterId, charges: &[(&str, f64)]) {
        let charges: Vec<(StatAxis, f64)> = charges
            .iter()
            .map(|(axis, charge)| {
                (
                    StatAxis::parse(axis).expect("正当な軸名であること"),
                    *charge,
                )
            })
            .collect();
        self.store
            .save_drift_charges(
                character,
                &charges,
                StatDriftMode::On
                    .applying()
                    .expect("on なら許可証を得られること"),
            )
            .await
            .expect("累積を書けること");
    }

    async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            shutdown.send(()).expect("停止シグナルを送れること");
        }
        if let Some(handle) = self.handle.take() {
            handle
                .await
                .expect("サーバタスクが panic していないこと")
                .expect("サーバが正常終了すること");
        }
    }
}

/// 変容の一覧のパス。
fn drift_path(handle: &str) -> String {
    format!("/admin/characters/{handle}/drift")
}

/// 性格ステータスのパス。
fn stats_path(handle: &str) -> String {
    format!("/admin/characters/{handle}/stats")
}

/// エラー応答の本文を**完全一致**で組み立てる（内部情報が混ざったら赤になる）。
fn error_body(message: &str, code: &str) -> Value {
    json!({"error": {"message": message, "type": "invalid_request_error", "code": code}})
}

/// **内部情報が漏れていないこと。** どのエラー応答にも通す。
fn assert_no_internals(server: &TestServer, body: &Value) {
    let text = body.to_string();
    for leak in [
        server.db_path.as_str(),
        "sqlite",
        "sqlx",
        "personacore.sqlite",
        "src\\",
        "src/",
        "stat_drift",
    ] {
        assert!(
            !text.contains(leak),
            "内部情報 {leak:?} が漏れていないこと: {text}"
        );
    }
}

/// 応答の `axes` を取り出す。
fn axes(body: &Value) -> Vec<Value> {
    body.get("axes")
        .unwrap_or_else(|| panic!("axes を載せること: {body}"))
        .as_array()
        .expect("axes は配列であること")
        .clone()
}

/// 軸1件から `charge_updated_at` を**抜き取る**（作業単位 D-5a）。
///
/// **抜くときに必ず見る**ので、この欄を落とす変異はここで panic して赤になる
/// （票の `created_at` を抜く [`take_created_at`] と同じ作法）。
///
/// **累積の行が無い軸は `null`**（`0` でも現在時刻でもない / `DriftAxisView` の doc）。
fn take_charge_time(axis: &mut Value) -> Option<DateTime<Utc>> {
    let shown = axis.to_string();
    let object = axis
        .as_object_mut()
        .unwrap_or_else(|| panic!("軸はオブジェクトであること: {shown}"));
    let raw = object
        .remove("charge_updated_at")
        .unwrap_or_else(|| panic!("charge_updated_at を載せること: {shown}"));
    if raw.is_null() {
        return None;
    }
    let raw = raw
        .as_str()
        .unwrap_or_else(|| panic!("charge_updated_at は文字列か null であること: {shown}"));
    Some(
        DateTime::parse_from_rfc3339(raw)
            .unwrap_or_else(|err| panic!("RFC3339 として読めること: {err} / {raw}"))
            .with_timezone(&Utc),
    )
}

/// `axes` から時刻を抜いた形。**他の欄の突き合わせはこちらで行う。**
///
/// 時刻そのものは、それを主題にしたテスト
/// （[`each_axis_shows_when_its_own_charge_last_moved`]）が
/// **軸名と対にして**押さえる——ここで一緒に見ると、
/// **本番の書き込み口が押す時刻は「いま」なので、期待値が書けない。**
fn axes_without_times(body: &Value) -> Value {
    Value::Array(
        axes(body)
            .into_iter()
            .map(|mut axis| {
                take_charge_time(&mut axis);
                axis
            })
            .collect(),
    )
}

/// `axes` を `(軸名, 累積が最後に動いた時刻)` の組で返す（作業単位 D-5a）。
fn charge_times(body: &Value) -> Vec<(String, Option<DateTime<Utc>>)> {
    axes(body)
        .into_iter()
        .map(|mut axis| {
            let at = take_charge_time(&mut axis);
            let name = axis
                .get("axis")
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("axis を載せること: {axis}"))
                .to_owned();
            (name, at)
        })
        .collect()
}

/// **総数と軸の行の辻褄が合っている**（作業単位 B-16 の条件2）。
///
/// # なぜこの検査が要るのか
///
/// 是正前は「全 N 件」と言いながら、**割り当てを失った軸の票がどの行にも現れなかった。**
/// 読み手には「総数と、軸ごとの件数の合計が合わない」としか見えず、
/// **どこへ消えたのかを応答から知る手段が無い**（`unreadable` の形も無い）。
///
/// **どの軸の行も持たない票が1件でもあれば、ここで赤になる。**
/// 逆向き（同じ票が2つの軸に数えられる）も同じ式で落ちる。
///
/// # 呼ぶ側で件数を数え直さないこと
///
/// 期待値は**応答の中の別々の欄どうし**で突き合わせる。
/// 仕込んだ数からも実装の定数からも作らない。
///
/// # **残っていた条件は上流で潰した**（作業単位 B-19 / 2026-08-28）
///
/// B-16 の時点では条件が1つ残っていた——
/// **「`stat_alloc` の軸が、正規化した後も一意であること」。**
/// `stat_alloc` の主キーは**生のテキスト**なので、DB を直接編集して
/// `パッション` と ` パッション` を置くと同居でき、`Store::stat_allocs` が
/// **両方を同じ軸名に正規化して受理していた。**
/// **同じ軸の行が2つ並び、どちらも同じ集計を引き当てて総和が総数を超える**
/// （実測: `ups 2 / downs 1` の行が2つ、総数 3 に対して総和 6）。
///
/// **B-19 で `Store::stat_allocs` がその状態を `InvalidStoredValue` にした**ので、
/// **重複したまま 200 が返ることは無い**——
/// **この等式に残る条件は「この応答が 200 で返っていること」だけ**である。
/// 見張り番は
/// `two_rows_that_normalize_to_the_same_axis_stop_both_endpoints_until_the_stats_are_replaced`
/// （`GET .../stats` と `GET .../drift` を**同じ状態に**通して、両方が止まることを見る）。
///
/// **⚠ それでも、赤を見たらまず軸の重複を疑うこと。**
/// **この関数を呼ぶのは4つのテスト関数（呼び出しは5箇所。B-19 で数え直した）で、
/// どれも 200 が返ることを前提にしている。**
/// 重複が上流をすり抜ける実装に戻せば、**原因は変容の側ではなく `stat_alloc`** である。
fn assert_every_vote_belongs_to_an_axis(body: &Value) {
    let counted: u64 = axes(body)
        .iter()
        .map(|axis| {
            let ups = axis
                .get("ups")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| panic!("ups を載せること: {axis}"));
            let downs = axis
                .get("downs")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| panic!("downs を載せること: {axis}"));
            ups + downs
        })
        .sum();
    let total = body
        .get("total_votes")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("total_votes を載せること: {body}"));

    assert_eq!(
        counted, total,
        "総数に数えた票が、どの軸の行にも現れていません（辻褄が合っていない）: {body}"
    );
}

/// 応答の `recent_votes` を取り出す。
fn recent_votes(body: &mut Value) -> Vec<Value> {
    let shown = body.to_string();
    body.get_mut("recent_votes")
        .unwrap_or_else(|| panic!("recent_votes を載せること: {shown}"))
        .as_array_mut()
        .expect("recent_votes は配列であること")
        .clone()
}

/// 票1件から `created_at` を**抜き取って**日時にする。
///
/// **抜くときに必ず見る**ので、`created_at` を落とす変異はここで panic して赤になる。
fn take_created_at(vote: &mut Value) -> DateTime<Utc> {
    let shown = vote.to_string();
    let object = vote
        .as_object_mut()
        .unwrap_or_else(|| panic!("票はオブジェクトであること: {shown}"));
    let raw = object
        .remove("created_at")
        .unwrap_or_else(|| panic!("created_at を載せること: {shown}"));
    let raw = raw.as_str().expect("created_at は文字列であること");
    DateTime::parse_from_rfc3339(raw)
        .unwrap_or_else(|err| panic!("RFC3339 として読めること: {err} / {raw}"))
        .with_timezone(&Utc)
}

/// 仕込んだ時刻の綴りを日時にする（期待値の側）。
fn instant(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .expect("テストで使う時刻は RFC3339 であること")
        .with_timezone(&Utc)
}

/// `recent_votes` を `(残りのオブジェクト, created_at)` に割る。
fn split_votes(body: &mut Value) -> (Vec<Value>, Vec<DateTime<Utc>>) {
    let mut votes = recent_votes(body);
    let times = votes.iter_mut().map(take_created_at).collect();
    (votes, times)
}

/// 票1件の期待値（`created_at` を抜いた後の形）。
fn expected_vote(axis: &str, delta: DriftDelta, session: SessionId) -> Value {
    json!({"axis": axis, "delta": delta.label(), "session_id": session.get()})
}

/// 仕込む票の並び（**古い順**）。時刻・軸・向き・往復を全部ばらしてある。
fn ballots(
    first: SessionId,
    second: SessionId,
) -> Vec<(SessionId, &'static str, DriftDelta, &'static str)> {
    vec![
        (
            first,
            AXIS_PASSION,
            DriftDelta::Up,
            "2026-08-27T10:00:00.000Z",
        ),
        (
            second,
            AXIS_LOGIC,
            DriftDelta::Down,
            "2026-08-27T10:00:01.000Z",
        ),
        (
            first,
            AXIS_CHEER,
            DriftDelta::Up,
            "2026-08-27T10:00:02.000Z",
        ),
        (
            second,
            AXIS_PASSION,
            DriftDelta::Down,
            "2026-08-27T10:00:03.000Z",
        ),
        (
            first,
            AXIS_LOGIC,
            DriftDelta::Up,
            "2026-08-27T10:00:04.000Z",
        ),
        (
            second,
            AXIS_CHEER,
            DriftDelta::Up,
            "2026-08-27T10:00:05.000Z",
        ),
        (
            first,
            AXIS_PASSION,
            DriftDelta::Up,
            "2026-08-27T10:00:06.000Z",
        ),
        (
            second,
            AXIS_LOGIC,
            DriftDelta::Down,
            "2026-08-27T10:00:07.000Z",
        ),
        (
            first,
            AXIS_PASSION,
            DriftDelta::Up,
            "2026-08-27T10:00:08.000Z",
        ),
    ]
}

/// 2体のキャラクター・5つの往復・仕込んだ票と累積。
struct Fixture {
    server: TestServer,
    character: CharacterId,
    /// 票を立てる往復。
    first: SessionId,
    /// **同じキャラクターの別の往復。** `session_id` の決め打ちを観測する。
    second: SessionId,
}

/// **票が立つ側と立たない側**を作り、票と累積を仕込む。
///
/// # 同値を作らないこと（CLAUDE.md の最頻の穴）
///
/// * 傍観者を**先に**作るので、票が立つ側の `character_id` は 1 ではない。
/// * 往復は**傍観者の側に3つ作ってから**作るので、`session_id` は `character_id` と別の値。
/// * 軸の値・累積・上げ下げの件数は**全部違う数**（どの2つを取り違えても赤になる）。
async fn fresh() -> Fixture {
    let server = TestServer::start().await;

    let bystander = server
        .create_character(BYSTANDER_HANDLE, BYSTANDER_DISPLAY_NAME)
        .await;
    let character = server.create_character(HANDLE, DISPLAY_NAME).await;
    assert_ne!(character, bystander, "前提: 2体の id が別であること");
    assert_ne!(
        character,
        CharacterId::from_raw_for_tests(1),
        "前提: 票が立つ側の id が 1 でないこと（決め打ちと同値にしない）"
    );

    server
        .put_stats(
            HANDLE,
            &[
                (AXIS_PASSION, 3),
                (AXIS_LOGIC, 0),
                (AXIS_CHEER, 5),
                (AXIS_CARE, 2),
            ],
        )
        .await;
    server
        .put_stats(BYSTANDER_HANDLE, &[(AXIS_PASSION, 6), (BYSTANDER_AXIS, 4)])
        .await;

    // **傍観者の往復を3つ先に作る**（＝ `session_id` が `character_id` とずれる）。
    let mut bystander_sessions = Vec::new();
    for _ in 0..3 {
        bystander_sessions.push(
            server
                .store
                .create_session(bystander)
                .await
                .expect("セッションを作成できること"),
        );
    }
    let first = server
        .store
        .create_session(character)
        .await
        .expect("セッションを作成できること");
    let second = server
        .store
        .create_session(character)
        .await
        .expect("セッションを作成できること");
    assert_ne!(first, second, "前提: 2つの往復が別であること");
    assert_ne!(
        first.get(),
        character.get(),
        "前提: 往復の id と持ち主の id が別の値であること"
    );
    assert_ne!(
        second.get(),
        character.get(),
        "前提: 往復の id と持ち主の id が別の値であること"
    );

    for (session, axis, delta, at) in ballots(first, second) {
        server.plant_vote(character, session, axis, delta, at).await;
    }

    // **傍観者の票は、こちらより新しい時刻に置く。**
    // 絞り込みが消えたら**一覧の先頭**に現れるので、取り違えが目に見える。
    for index in 0..5 {
        server
            .plant_vote(
                bystander,
                bystander_sessions[index % bystander_sessions.len()],
                AXIS_PASSION,
                DriftDelta::Up,
                &format!("2026-08-27T11:00:0{index}.000Z"),
            )
            .await;
    }

    // **累積は3軸だけ。** `AXIS_CHEER` は行そのものが無い。
    server
        .save_charges(
            character,
            &[(AXIS_PASSION, 0.25), (AXIS_LOGIC, -0.75), (AXIS_CARE, 1.5)],
        )
        .await;
    // 傍観者の累積は**同じ軸名で違う値**（取り違えたら赤になる）。
    server
        .save_charges(bystander, &[(AXIS_PASSION, 9.5), (BYSTANDER_AXIS, -2.25)])
        .await;

    Fixture {
        server,
        character,
        first,
        second,
    }
}

/// **軸ごとの姿が、いまの値・累積・上げ下げの件数を、それぞれの軸のものとして返す。**
///
/// # ここで初めて観測できること
///
/// * **値 0 の軸**（`AXIS_LOGIC`）が落ちない——落とすのは `prompt` 層の責務であり、
///   この入口は `GET .../stats` と同じ立場を取る。
/// * **累積の行が無い軸**（`AXIS_CHEER`）が 0 として並ぶ——内部結合にすると消える。
/// * **票が1件も無い軸**（`AXIS_CARE`）が 0/0 として並ぶ。
///
/// 4つの軸で値・累積・件数が**全部違う**ので、どの2つを取り違えても赤になる。
#[tokio::test]
async fn every_axis_carries_its_own_value_charge_and_tally() {
    let fixture = fresh().await;

    let (status, body) = fixture.server.get(&drift_path(HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "一覧を引けること: {body}");

    assert_eq!(
        body.get("character"),
        Some(&json!(HANDLE)),
        "どのキャラクターのものかを名乗ること: {body}"
    );
    assert_eq!(
        body.get("stat_scale"),
        Some(&json!(fixture_scale().get())),
        "尺度はこのキャラクターのものであること: {body}"
    );
    assert_eq!(
        axes_without_times(&body),
        json!([
            {"axis": AXIS_PASSION, "value": 3, "charge": 0.25, "ups": 3, "downs": 1},
            {"axis": AXIS_LOGIC, "value": 0, "charge": -0.75, "ups": 1, "downs": 2},
            {"axis": AXIS_CHEER, "value": 5, "charge": 0.0, "ups": 2, "downs": 0},
            {"axis": AXIS_CARE, "value": 2, "charge": 1.5, "ups": 0, "downs": 0},
        ]),
        "軸ごとの姿が、保存順に、それぞれの値で返ること: {body}"
    );

    fixture.server.stop().await;
}

/// **軸の並びと値は `GET .../stats` と同じ**（読んでいるものが同じ）。
///
/// 片方だけが並べ替わると、画面が2つの一覧を並べたときに行がずれる。
#[tokio::test]
async fn the_axes_match_the_stats_endpoint() {
    let fixture = fresh().await;

    let (_, drift) = fixture.server.get(&drift_path(HANDLE)).await;
    let (_, stats) = fixture.server.get(&stats_path(HANDLE)).await;

    let from_drift: Vec<Value> = drift
        .get("axes")
        .expect("axes を載せること")
        .as_array()
        .expect("axes は配列であること")
        .iter()
        .map(|axis| json!({"axis": axis.get("axis"), "value": axis.get("value")}))
        .collect();
    let from_stats: Vec<Value> = stats
        .get("stats")
        .expect("stats を載せること")
        .as_array()
        .expect("stats は配列であること")
        .iter()
        .map(|stat| json!({"axis": stat.get("axis"), "value": stat.get("value")}))
        .collect();

    assert_eq!(
        from_drift, from_stats,
        "軸の並びと値が2つの入口で一致すること: {drift} / {stats}"
    );
    assert_eq!(
        drift.get("stat_scale"),
        stats.get("stat_scale"),
        "尺度も一致すること: {drift} / {stats}"
    );

    fixture.server.stop().await;
}

/// **票そのものが、新しい順に、それぞれの軸・向き・往復・時刻のまま返る。**
///
/// # 帰属を4方向から見る
///
/// * **軸と向きの組** —— 同じ軸に上げと下げが両方あるので、組が入れ替われば赤。
/// * **往復** —— 同じキャラクターの2つの往復が交互に並ぶ。
/// * **時刻** —— 行ごとに違う値で、抜き取って突き合わせる。
/// * **持ち主** —— 傍観者の5件は**より新しい**ので、混ざれば先頭に出る。
#[tokio::test]
async fn every_vote_keeps_its_axis_direction_turn_and_time() {
    let fixture = fresh().await;

    let (status, mut body) = fixture.server.get(&drift_path(HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "一覧を引けること: {body}");
    let (votes, times) = split_votes(&mut body);

    let planted = ballots(fixture.first, fixture.second);
    let expected: Vec<Value> = planted
        .iter()
        .rev()
        .map(|(session, axis, delta, _)| expected_vote(axis, *delta, *session))
        .collect();
    let expected_times: Vec<DateTime<Utc>> = planted
        .iter()
        .rev()
        .map(|(_, _, _, at)| instant(at))
        .collect();

    assert_eq!(votes, expected, "票が新しい順に、そのまま返ること");
    assert_eq!(times, expected_times, "時刻がその行のものであること");

    fixture.server.stop().await;
}

/// **傍観者の票も累積も混ざらない**（`character_id` の絞り込み）。
///
/// 3つの読み（一覧・総数・集計）が**それぞれ**絞り込みを持っていることを、
/// 傍観者の側から見て確かめる。
#[tokio::test]
async fn another_characters_drift_never_leaks_in() {
    let fixture = fresh().await;

    let (status, body) = fixture.server.get(&drift_path(BYSTANDER_HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "傍観者の一覧も引けること: {body}");

    assert_eq!(
        body.get("total_votes"),
        Some(&json!(5)),
        "傍観者の総数は傍観者の票だけであること: {body}"
    );
    assert_eq!(
        axes_without_times(&body),
        json!([
            {"axis": AXIS_PASSION, "value": 6, "charge": 9.5, "ups": 5, "downs": 0},
            {"axis": BYSTANDER_AXIS, "value": 4, "charge": -2.25, "ups": 0, "downs": 0},
        ]),
        "同じ綴りの軸でも、値・累積・件数が混ざらないこと: {body}"
    );

    let votes = body
        .get("recent_votes")
        .expect("recent_votes を載せること")
        .as_array()
        .expect("配列であること");
    assert_eq!(votes.len(), 5, "傍観者の票は5件であること: {body}");
    for vote in votes {
        assert_eq!(
            vote.get("axis"),
            Some(&json!(AXIS_PASSION)),
            "傍観者の票だけであること: {body}"
        );
    }

    fixture.server.stop().await;
}

/// **同着は `id` の降順で決まる**（並びが問い合わせ計画に依存しない）。
///
/// # なぜ索引を外すのか
///
/// 索引 `(character_id, created_at)` を後ろから辿る計画では、
/// **同着の中が偶然 `id` の降順になる。** つまり第2キーが無くても結果は同じで、
/// 索引がある状態では**第2キーを守れない**（`tests/store_drift_vote.rs` が実測）。
/// 索引を外すと計画が「表を走査してから並べ替える」に変わり、
/// **同着の中は `id` の昇順**になる——順が逆になるので、第2キーが無いと必ず落ちる。
///
/// 落としているのは**このテスト専用の一時 DB** の索引で、マイグレーションは触らない。
///
/// # 同着は本番で普通に起きる
///
/// **1ターンで複数の軸が同時に書かれる**（全軸採点）ので、
/// `created_at` だけでは順序が決まらない。
#[tokio::test]
async fn ties_are_ordered_by_id_even_when_the_index_is_not_used() {
    let server = TestServer::start().await;
    let character = server.create_character(HANDLE, DISPLAY_NAME).await;
    server
        .put_stats(HANDLE, &[(AXIS_PASSION, 3), (AXIS_LOGIC, 1)])
        .await;
    let session = server
        .store
        .create_session(character)
        .await
        .expect("セッションを作成できること");
    let other = server
        .store
        .create_session(character)
        .await
        .expect("セッションを作成できること");

    const SAME_INSTANT: &str = "2026-08-27T12:00:00.000Z";
    let tied = [
        (session, AXIS_PASSION, DriftDelta::Up),
        (other, AXIS_PASSION, DriftDelta::Down),
        (session, AXIS_LOGIC, DriftDelta::Up),
        (other, AXIS_LOGIC, DriftDelta::Down),
    ];
    let mut ids = Vec::new();
    for (session, axis, delta) in tied {
        ids.push(
            server
                .plant_vote(character, session, axis, delta, SAME_INSTANT)
                .await,
        );
    }
    assert!(
        ids.windows(2).all(|pair| pair[0] < pair[1]),
        "前提: 後の行ほど id が大きいこと: {ids:?}"
    );

    // **計画を変える。** 索引が使えないと、同着の中は `id` の昇順で出てくる。
    sqlx::query("DROP INDEX stat_drift_vote_recent_idx")
        .execute(server.store.pool())
        .await
        .expect("索引を落とせること");

    let (_, mut body) = server.get(&drift_path(HANDLE)).await;
    let (votes, _) = split_votes(&mut body);
    let expected: Vec<Value> = tied
        .iter()
        .rev()
        .map(|(session, axis, delta)| expected_vote(axis, *delta, *session))
        .collect();
    assert_eq!(
        votes, expected,
        "索引が無くても、同着は入った順の逆（= id の降順）で返ること"
    );

    server.stop().await;
}

/// **上限を超えたら直近だけを返す。総数は全部を数える。**
///
/// # 期待値を定数から作らない（CLAUDE.md `16b`）
///
/// `RECENT_DRIFT_VOTES` から期待値を組むと、**その定数を倍にする変異が緑を通る。**
/// ここは**外の値**（直書きの 63 と 50）で押さえる。
///
/// # 総数を「返した件数」にすり替える変異は、この状態でしか観測できない
///
/// 票が上限より少ないうちは総数と件数が同値なので、**どちらでも緑になる。**
///
/// # ⚠ **この1本が3つの主張を持っている**（分割・簡略化するときの注意）
///
/// 1. **総数**が「返した件数」ではないこと（`total_votes == 63`）。
/// 2. **上限**が効いていること（`recent_votes.len() == 50` と `recent_limit == 50`）。
/// 3. **新しい順**であること（先頭が最後に入れた票）。
///
/// **3つとも、票が上限を超えている状態でしか観測できない。**
/// 上限以下では総数と件数が同値になり、どちらへ倒す実装も緑になる
/// ——**この状態を作るテストはここ1本だけ**である。
/// **分けるなら、分けた先それぞれで 50 件超を積むこと。**
/// 積まずに分けると、**3つ同時に無防備になる。**
#[tokio::test]
async fn only_the_most_recent_votes_come_back_but_the_total_counts_them_all() {
    let server = TestServer::start().await;
    let character = server.create_character(HANDLE, DISPLAY_NAME).await;
    server.put_stats(HANDLE, &[(AXIS_PASSION, 3)]).await;
    let session = server
        .store
        .create_session(character)
        .await
        .expect("セッションを作成できること");

    // **上限より多く入れる。** 時刻は1秒ずつ進めるので同着は起きない。
    for index in 0..63 {
        server
            .plant_vote(
                character,
                session,
                AXIS_PASSION,
                DriftDelta::Up,
                &format!("2026-08-27T13:{:02}:{:02}.000Z", index / 60, index % 60),
            )
            .await;
    }

    let (status, body) = server.get(&drift_path(HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "一覧を引けること: {body}");

    let votes = body
        .get("recent_votes")
        .expect("recent_votes を載せること")
        .as_array()
        .expect("配列であること");
    assert_eq!(votes.len(), 50, "直近 50 件だけを返すこと: {}", votes.len());
    assert_eq!(
        body.get("recent_limit"),
        Some(&json!(50)),
        "上限を名乗ること（画面が『これで全部』と言えないため）: {body}"
    );
    assert_eq!(
        body.get("total_votes"),
        Some(&json!(63)),
        "総数は返した件数ではなく、保存されている全部であること: {body}"
    );
    assert_eq!(
        axes_without_times(&body),
        json!([
            {"axis": AXIS_PASSION, "value": 3, "charge": 0.0, "ups": 63, "downs": 0},
        ]),
        "集計も上限で切られないこと: {body}"
    );

    // **いちばん新しい票が先頭。**
    let newest = votes.first().expect("1件はあること");
    assert_eq!(
        newest
            .get("created_at")
            .and_then(Value::as_str)
            .map(instant),
        Some(instant("2026-08-27T13:01:02.000Z")),
        "新しい順であること: {newest}"
    );

    server.stop().await;
}

/// **票が1件も無いキャラクターでも、軸は 0 として並ぶ。**
///
/// 空の一覧を 404 やエラーにしない——「まだ何も起きていない」は正常な状態である。
#[tokio::test]
async fn a_character_without_any_drift_still_lists_its_axes() {
    let server = TestServer::start().await;
    server.create_character(HANDLE, DISPLAY_NAME).await;
    server
        .put_stats(HANDLE, &[(AXIS_PASSION, 3), (AXIS_LOGIC, 0)])
        .await;

    let (status, body) = server.get(&drift_path(HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "一覧を引けること: {body}");
    assert_eq!(
        body,
        json!({
            "character": HANDLE,
            "stat_scale": fixture_scale().get(),
            "axes": [
                // **累積の行が1つも無いので、時刻は全部 `null`**（作業単位 D-5a）。
                {"axis": AXIS_PASSION, "value": 3, "charge": 0.0,
                 "charge_updated_at": null, "ups": 0, "downs": 0},
                {"axis": AXIS_LOGIC, "value": 0, "charge": 0.0,
                 "charge_updated_at": null, "ups": 0, "downs": 0},
            ],
            "total_votes": 0,
            "recent_limit": 50,
            "recent_votes": [],
            // **この応答こそが「壊れている」と紛らわしい形である**（作業単位 D-5a）——
            // **既定は off なので、変容は判定すらされない。**
            // **走らせ方を名乗らないと、0 件の理由が応答から読めない。**
            "run_mode": "off",
            // **出どころと起動時の既定**（ADR-0045 §決定4 / 作業単位 A-8）。
            // キャラクターは未設定なので、既定がそのまま効いている。
            "run_mode_source": "default",
            "default_run_mode": "off",
        }),
        "応答の形が丸ごと固定されること"
    );

    server.stop().await;
}

/// **本番の書き込み口（票の記録・累積の保存）で入れたものが、そのまま見える。**
///
/// 他のテストの仕込みは素の SQL なので、**書き手と読み手が繋がっていること**は
/// ここでしか観測できない（列の綴りが片方だけ変わったら赤になる）。
#[tokio::test]
async fn what_the_production_writer_records_is_what_this_endpoint_shows() {
    let server = TestServer::start().await;
    let character = server.create_character(HANDLE, DISPLAY_NAME).await;
    server
        .put_stats(HANDLE, &[(AXIS_PASSION, 3), (AXIS_LOGIC, 1)])
        .await;
    let session = server
        .store
        .create_session(character)
        .await
        .expect("セッションを作成できること");

    let current = server
        .store
        .stat_allocs(character)
        .await
        .expect("軸を読めること");
    for (axis, delta) in [
        (AXIS_PASSION, DriftDelta::Up),
        (AXIS_LOGIC, DriftDelta::Down),
        (AXIS_PASSION, DriftDelta::Up),
    ] {
        server
            .store
            .record_drift_vote(character, session, &vote(&current, axis, delta))
            .await
            .expect("票を記録できること");
    }
    server.save_charges(character, &[(AXIS_LOGIC, -0.5)]).await;

    let (status, mut body) = server.get(&drift_path(HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "一覧を引けること: {body}");
    assert_eq!(
        axes_without_times(&body),
        json!([
            {"axis": AXIS_PASSION, "value": 3, "charge": 0.0, "ups": 2, "downs": 0},
            {"axis": AXIS_LOGIC, "value": 1, "charge": -0.5, "ups": 0, "downs": 1},
        ]),
        "記録した票と累積が、それぞれの軸に載ること: {body}"
    );
    assert_eq!(
        body.get("total_votes"),
        Some(&json!(3)),
        "総数が記録した数であること: {body}"
    );

    let (votes, times) = split_votes(&mut body);
    assert_eq!(
        votes,
        vec![
            expected_vote(AXIS_PASSION, DriftDelta::Up, session),
            expected_vote(AXIS_LOGIC, DriftDelta::Down, session),
            expected_vote(AXIS_PASSION, DriftDelta::Up, session),
        ],
        "記録した順の逆で返ること"
    );
    assert!(
        times.windows(2).all(|pair| pair[0] >= pair[1]),
        "時刻が新しい順であること: {times:?}"
    );

    server.stop().await;
}

/// **軸ごとに、その軸の累積が最後に動いた時刻が載る**（作業単位 D-5a）。
///
/// # ここが是正の半分である
///
/// `stat_drift_charge.updated_at` は 16e-b から**毎回書き込まれていた**のに、
/// **`store` のどの `SELECT` も読んでいなかった**
/// ——`store::drift` の `UPSERT_CHARGE` の doc が
/// 「**表から取れる時間の手掛かりはこの1列だけ**」と書いていた列が、
/// **`sqlite3` からしか取れない**状態だった。
///
/// # 4つの軸で、時刻の在り方を全部通す
///
/// | 軸 | 状態 |
/// |---|---|
/// | `パッション` / `ロジック` / `気配り` | 累積の行がある（**時刻は3つとも別の値**） |
/// | `陽気さ` | **累積の行が無い** → `null`（`0` でも現在時刻でもない） |
///
/// **押さえるのは真ん中**（`ロジック`）——2件だと「常に先頭」「常に末尾」の
/// 片方が通り抜ける（`D-2` の実測）。
///
/// # 軸名の順と時刻の順を、わざと食い違わせる
///
/// 保存順は `パッション` → `ロジック` → `陽気さ` → `気配り` で、
/// 時刻は 2026 → 2024 → （無し）→ 2025 にしてある。
/// **どちらか一方の順に頼って引き当てる実装は、ここで必ず赤になる。**
///
/// # 傍観者にも同じ軸名で別の時刻を置く
///
/// 累積の読みから持ち主の絞り込みが消えたら、**他人の時刻が載る。**
#[tokio::test]
async fn each_axis_shows_when_its_own_charge_last_moved() {
    /// 累積を植える時刻。**軸ごとに全部違う。**
    const PASSION_AT: &str = "2026-08-26T01:02:03.400Z";
    const LOGIC_AT: &str = "2024-05-04T03:02:01.100Z";
    const CARE_AT: &str = "2025-12-31T23:59:59.999Z";
    /// **傍観者の時刻。** 混ざったら赤になる。
    const BYSTANDER_AT: &str = "2019-01-01T00:00:00.000Z";

    let server = TestServer::start().await;
    let bystander = server
        .create_character(BYSTANDER_HANDLE, BYSTANDER_DISPLAY_NAME)
        .await;
    let character = server.create_character(HANDLE, DISPLAY_NAME).await;
    server
        .put_stats(
            HANDLE,
            &[
                (AXIS_PASSION, 3),
                (AXIS_LOGIC, 0),
                (AXIS_CHEER, 5),
                (AXIS_CARE, 2),
            ],
        )
        .await;
    server
        .put_stats(BYSTANDER_HANDLE, &[(AXIS_PASSION, 6)])
        .await;

    for (axis_name, charge, at) in [
        (AXIS_PASSION, 0.25_f64, PASSION_AT),
        (AXIS_LOGIC, -0.75, LOGIC_AT),
        (AXIS_CARE, 1.5, CARE_AT),
    ] {
        insert_drift_charge_at(&server.store, character.get(), axis_name, charge, at).await;
    }
    insert_drift_charge_at(
        &server.store,
        bystander.get(),
        AXIS_PASSION,
        9.5,
        BYSTANDER_AT,
    )
    .await;

    let (status, body) = server.get(&drift_path(HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "一覧を引けること: {body}");

    assert_eq!(
        charge_times(&body),
        vec![
            (AXIS_PASSION.to_owned(), Some(instant(PASSION_AT))),
            (AXIS_LOGIC.to_owned(), Some(instant(LOGIC_AT))),
            // **累積の行が無い軸は `null`。**
            (AXIS_CHEER.to_owned(), None),
            (AXIS_CARE.to_owned(), Some(instant(CARE_AT))),
        ],
        "軸ごとに、その軸の時刻が載ること: {body}"
    );

    // **累積の値も一緒に見る**（時刻と値の取り違えを落とす）。
    assert_eq!(
        axes_without_times(&body),
        json!([
            {"axis": AXIS_PASSION, "value": 3, "charge": 0.25, "ups": 0, "downs": 0},
            {"axis": AXIS_LOGIC, "value": 0, "charge": -0.75, "ups": 0, "downs": 0},
            {"axis": AXIS_CHEER, "value": 5, "charge": 0.0, "ups": 0, "downs": 0},
            {"axis": AXIS_CARE, "value": 2, "charge": 1.5, "ups": 0, "downs": 0},
        ]),
        "累積の値の側も、それぞれの軸のものであること: {body}"
    );

    // **傍観者の時刻は傍観者の側にだけ出る。**
    let (status, other) = server.get(&drift_path(BYSTANDER_HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "傍観者も引けること: {other}");
    assert_eq!(
        charge_times(&other),
        vec![(AXIS_PASSION.to_owned(), Some(instant(BYSTANDER_AT)))],
        "傍観者の時刻が混ざっていないこと: {other}"
    );

    server.stop().await;
}

/// **累積の時刻が「0 に戻った」と「一度も採点されていない」を分ける**（作業単位 D-5a）。
///
/// # なぜ別のテストなのか
///
/// **`charge` の値だけでは区別が付かない**——`store::drift` の doc が
/// 「一度も採点されていないと 0 に戻ったは表の上でも同じ意味」と書いていたのは
/// **累積の列だけを見ていたから**である。**時刻の列がその2つを分ける。**
///
/// **同じ応答の中に両方を置く**——別々のテストに分けると、
/// **どちらも `null` にする実装**も**どちらも時刻を付ける実装**も、
/// 片方ずつ緑になる（`a_zero_valued_axis_is_never_confused_with…` と同じ形）。
#[tokio::test]
async fn a_charge_that_fell_back_to_zero_is_not_a_charge_that_never_happened() {
    /// **0 に戻った軸**の時刻。
    const SETTLED_AT: &str = "2026-01-02T03:04:05.600Z";

    let server = TestServer::start().await;
    server
        .create_character(BYSTANDER_HANDLE, BYSTANDER_DISPLAY_NAME)
        .await;
    let character = server.create_character(HANDLE, DISPLAY_NAME).await;
    server
        .put_stats(HANDLE, &[(AXIS_PASSION, 3), (AXIS_LOGIC, 0)])
        .await;

    // **`パッション` は 0 まで薄れた**（行はある）。`ロジック` は一度も書かれていない。
    insert_drift_charge_at(
        &server.store,
        character.get(),
        AXIS_PASSION,
        0.0,
        SETTLED_AT,
    )
    .await;

    let (status, body) = server.get(&drift_path(HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "一覧を引けること: {body}");

    // **累積の値は、2つとも 0.0**（＝この欄だけでは区別できない）。
    assert_eq!(
        axes_without_times(&body),
        json!([
            {"axis": AXIS_PASSION, "value": 3, "charge": 0.0, "ups": 0, "downs": 0},
            {"axis": AXIS_LOGIC, "value": 0, "charge": 0.0, "ups": 0, "downs": 0},
        ]),
        "前提: 累積の値では2つが同じに見えること: {body}"
    );
    // **時刻の側が分ける。**
    assert_eq!(
        charge_times(&body),
        vec![
            (AXIS_PASSION.to_owned(), Some(instant(SETTLED_AT))),
            (AXIS_LOGIC.to_owned(), None),
        ],
        "0 に戻った軸と、一度も採点されていない軸が、時刻で分かれること: {body}"
    );

    server.stop().await;
}

/// **応答が、いまサーバが走らせている変容の走らせ方を名乗る**（作業単位 D-5a）。
///
/// # なぜこれが要るのか
///
/// **`PERSONACORE_STAT_DRIFT` の既定は `off`** で、`off` は
/// **上流を呼ばず、判定もせず、票も累積も1行も作らない。**
/// **つまり既定の配置では `total_votes: 0` / 全軸 `charge: 0.0` /
/// `recent_votes: []` が返る**——**変容が壊れている状態と1バイトも違わない。**
/// 走らせ方は起動ログにしか出ておらず、**ログは流れる。**
///
/// # 3値を1本で回す（**片方の腕だけでは足りない**）
///
/// * **既定（`off`）だけ**だと、走らせ方を捨てて既定を書く実装が緑になる
///   （CLAUDE.md「起動時に決めるものを増やしたら、本番の入口を通るテストを
///   必ず1本足すこと。そのテストの期待値を既定値にしないこと」/ `B-7` の実測）。
/// * **`shadow` だけ**だと、`on` との入れ替えが観測できない。
///
/// **綴りは直書き**（`StatDriftMode::label` から作らない）——
/// 期待値を同じ関数から作ると、**綴りを入れ替える変異が期待値も一緒に動かして緑**になる。
///
/// # 通るのは本番の入口（`App::bind`）
///
/// テスト専用の入口を通すと、**本番の配線が誰にも通られないまま残る。**
/// **環境変数の綴りから読ませている**ので、`parse_stat_drift` から
/// `AppState::default_stat_drift` までが1本に繋がる（A-8 で改名。キャラクターは未設定なので既定がそのまま効く）。
#[tokio::test]
async fn the_response_names_the_run_mode_the_server_was_started_with() {
    for (configured, expected) in [
        (None, "off"),
        (Some("shadow"), "shadow"),
        (Some("on"), "on"),
    ] {
        let server = TestServer::start_with_mode(configured).await;
        server.create_character(HANDLE, DISPLAY_NAME).await;
        server.put_stats(HANDLE, &[(AXIS_PASSION, 3)]).await;

        let (status, body) = server.get(&drift_path(HANDLE)).await;
        assert_eq!(status, StatusCode::OK, "一覧を引けること: {body}");
        assert_eq!(
            body.get("run_mode"),
            Some(&json!(expected)),
            "起動した走らせ方（{configured:?}）を名乗ること: {body}"
        );

        server.stop().await;
    }
}

/// **軸として保存されていない綴りの票も、総数にも一覧にも軸の行にも残る。**
///
/// `PUT .../stats` は軸を消せるが、**票は消えない**（追記のみ / ADR-0026 §決定3）。
///
/// > **⚠ この見張り番は 2026-08-28（作業単位 B-16）に主張が変わった。**
/// > それまでは **「保存されていない軸は軸の一覧に現れないこと」** を固定していた
/// > ——つまり **`total_votes` は数えるのに、その票はどの行にも現れない**状態を
/// > 正しいものとして押さえていた。**司令塔の決定で、割り当ての無い軸も返す形にした**
/// > （`value` は `null`）。**総数を落とさない主張は、そのまま残っている。**
#[tokio::test]
async fn a_vote_for_an_axis_that_is_no_longer_allocated_gets_a_row_with_no_value() {
    let fixture = fresh().await;

    fixture
        .server
        .plant_vote(
            fixture.character,
            fixture.first,
            RETIRED_AXIS,
            DriftDelta::Down,
            "2026-08-27T10:00:09.000Z",
        )
        .await;

    let (_, mut body) = fixture.server.get(&drift_path(HANDLE)).await;
    assert_eq!(
        body.get("total_votes"),
        Some(&json!(10)),
        "総数に入ること: {body}"
    );
    assert_eq!(
        axes_without_times(&body),
        json!([
            {"axis": AXIS_PASSION, "value": 3, "charge": 0.25, "ups": 3, "downs": 1},
            {"axis": AXIS_LOGIC, "value": 0, "charge": -0.75, "ups": 1, "downs": 2},
            {"axis": AXIS_CHEER, "value": 5, "charge": 0.0, "ups": 2, "downs": 0},
            {"axis": AXIS_CARE, "value": 2, "charge": 1.5, "ups": 0, "downs": 0},
            // **割り当てが無いので `value` は `null`。** 0 と混ぜない。
            {"axis": RETIRED_AXIS, "value": null, "charge": 0.0, "ups": 0, "downs": 1},
        ]),
        "割り当てを失った軸も、値を名乗らないまま一覧の末尾に並ぶこと: {body}"
    );
    // **総数に数えた 10 件が、軸の行の側でも 10 件になる。**
    assert_every_vote_belongs_to_an_axis(&body);

    let (votes, _) = split_votes(&mut body);
    assert_eq!(
        votes.first(),
        Some(&expected_vote(
            RETIRED_AXIS,
            DriftDelta::Down,
            fixture.first
        )),
        "票の一覧には残ること"
    );

    fixture.server.stop().await;
}

/// 未知のキャラクターは **404**（`GET .../stats` と同じ作法）。
#[tokio::test]
async fn an_unknown_character_is_not_found() {
    let fixture = fresh().await;

    let (status, body) = fixture.server.get(&drift_path("nobody")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "404 であること: {body}");
    assert_eq!(
        body,
        error_body(
            "指定されたキャラクターが存在しません",
            "character_not_found"
        ),
        "理由がそのまま返ること"
    );
    assert_no_internals(&fixture.server, &body);

    fixture.server.stop().await;
}

/// **生えているのは `GET` 1本だけ。書き込みの口も、1件を指すパスも無い。**
///
/// # なぜ塞ぐのか（`no_admin_endpoint_deletes_or_rewrites_a_memory` と同じ立場）
///
/// 1件を指すパス（`.../drift/{id}`）を生やせば、**`PUT`（置き換え）と
/// `DELETE`（削除）が自然な形になる。**
/// 票は追記のみ（ADR-0026 §決定3）で、**外から書き換える口は作らない。**
/// 変容の記録は本文処理が書くものであり、書き換えられたらそれは記録ではない。
#[tokio::test]
async fn only_get_is_exposed_for_the_drift_path() {
    let server = TestServer::start().await;
    server.create_character(HANDLE, DISPLAY_NAME).await;

    let not_found = json!({
        "error": {
            "message": "指定されたエンドポイントは存在しません",
            "type": "invalid_request_error",
        }
    });
    let method_not_allowed = json!({
        "error": {
            "message": "このエンドポイントは指定されたメソッドを受け付けません",
            "type": "invalid_request_error",
        }
    });

    // **1件を指すパスは、どのメソッドでも 404。**
    for path in [
        "/admin/characters/aoi/drift/1",
        "/admin/characters/aoi/drifts",
        "/admin/drift",
    ] {
        for method in [Method::GET, Method::POST, Method::PUT, Method::DELETE] {
            let (status, body) = server.send(method.clone(), path, Some(&json!({}))).await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "{method} {path} が生えていないこと: {body}"
            );
            assert_eq!(body, not_found, "{method} {path}");
        }
    }

    // **一覧のパスが受けるのは `GET` だけ**（405）。
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        let (status, body) = server
            .send(method.clone(), &drift_path(HANDLE), Some(&json!({})))
            .await;
        assert_eq!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "{method} が生えていないこと: {body}"
        );
        assert_eq!(body, method_not_allowed, "{method}");
    }

    server.stop().await;
}

/// **許可リストを通して**票を組み立てる（`tests/store_drift_vote.rs` と同じ門）。
fn vote(current: &[StatAlloc], axis: &str, delta: DriftDelta) -> DriftVote {
    let proposal = DriftProposal::Suggested(DriftSuggestion {
        axis: StatAxis::parse(axis).expect("正当な軸名であること"),
        delta,
    });
    let outcome = personacore::domain::apply_drift(current, proposal);
    assert!(
        matches!(
            outcome,
            DriftOutcome::Applied(_) | DriftOutcome::Saturated { .. }
        ),
        "門を通る提案であること: {outcome:?}"
    );
    outcome.vote().expect("票になること")
}

/// **壊れた時刻の行1つで、この一覧は丸ごと止まる。総数も蓄積も止まらない。**
///
/// # なぜこの見張り番が要るのか（レビューの実測 / 2026-08-27）
///
/// `src/store/drift.rs` と `get_drift` の doc は「**1行でも読めなければ丸ごと止まる**」と
/// 書いているのに、**読み飛ばしに変える変異が 1602 全緑**だった。
/// **蓄積側の同じ決定には権威が2本ある**
/// （`tests/store_drift_vote.rs::a_broken_direction_stops_the_read_but_not_the_recording` /
/// `a_broken_axis_stops_the_read_the_same_way`）が、
/// **この作業単位で足した3つの読みだけが観測されていなかった。**
///
/// # 読み飛ばしに変えると何が起きるか
///
/// **壊れた行の票は一覧から静かに消えるのに、`total_votes` には残る**
/// （総数は軸も向きも読まない）。読み手には「63 件のうち 50 件」と出たまま**1件足りず**、
/// **どの行が壊れているのかは分からない**——`unreadable` の形にしない判断をしているので、
/// 手掛かりが他に無い。
///
/// # 「止まる読み」と「止まらない読み」を**同じテストで**見る
///
/// 片方だけだと、**どちらへ倒す変異も片側を通り抜ける。**
/// 時刻だけが壊れている行は、**蓄積の読みからは見えない**
/// （あちらは `created_at` を読まない / `DriftVoteRecord` の doc）。
#[tokio::test]
async fn a_broken_time_stops_this_listing_but_not_the_total_or_the_accumulation() {
    let server = TestServer::start().await;
    let character = server.create_character(HANDLE, DISPLAY_NAME).await;
    server.put_stats(HANDLE, &[(AXIS_PASSION, 3)]).await;
    let session = server
        .store
        .create_session(character)
        .await
        .expect("セッションを作成できること");

    for at in ["2026-08-27T15:00:00.000Z", "2026-08-27T15:00:01.000Z"] {
        server
            .plant_vote(character, session, AXIS_PASSION, DriftDelta::Up, at)
            .await;
    }
    // **日時として読めない綴り。** 列に CHECK が無いので、素の SQL で作れる。
    server
        .plant_vote(
            character,
            session,
            AXIS_PASSION,
            DriftDelta::Down,
            "こわれた時刻",
        )
        .await;

    let (status, body) = server.get(&drift_path(HANDLE)).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "1行読めなければ丸ごと止まること: {body}"
    );
    assert_eq!(
        body,
        json!({"error": {"message": "内部エラーが発生しました", "type": "server_error"}}),
        "理由を反射しないこと"
    );
    // **500 の本文こそ内部情報が載りうる**（`sqlx` のデコード失敗が絡む）。
    // 404 だけを通していると、その側が検査対象の外に残る。
    assert_no_internals(&server, &body);

    // **止まらない読み1: 総数。** 軸も向きも時刻も読まないので、数は返り続ける。
    assert_eq!(
        server
            .store
            .count_drift_votes(character)
            .await
            .expect("総数は数えられること"),
        3,
        "壊れた行も票として数に入ること"
    );

    // **止まらない読み2: 蓄積。** `created_at` を読まないので、変容は走り続ける。
    assert_eq!(
        server
            .store
            .recent_drift_votes(character, 32)
            .await
            .expect("蓄積の読みは止まらないこと")
            .len(),
        3,
        "蓄積は3件とも読めること"
    );

    server.stop().await;
}

/// **壊れた軸名の行では、この一覧と蓄積の両方が止まる。総数だけは止まらない。**
///
/// `a_broken_time_stops_this_listing_but_not_the_total_or_the_accumulation` と対にして、
/// **`src/store/drift.rs` の doc の表（壊れている列ごとに止まる範囲が違う）を丸ごと固定する。**
/// 片方だけだと、doc の表の半分が「書いてあるだけ」になる。
#[tokio::test]
async fn a_broken_axis_stops_this_listing_and_the_accumulation_but_never_the_total() {
    let server = TestServer::start().await;
    let character = server.create_character(HANDLE, DISPLAY_NAME).await;
    server.put_stats(HANDLE, &[(AXIS_PASSION, 3)]).await;
    let session = server
        .store
        .create_session(character)
        .await
        .expect("セッションを作成できること");

    server
        .plant_vote(
            character,
            session,
            AXIS_PASSION,
            DriftDelta::Up,
            "2026-08-27T16:00:00.000Z",
        )
        .await;
    // **空白だけの軸名。** `StatAxis::parse` も CHECK も通らない値
    // （`tests/store_drift_vote.rs::a_broken_axis_stops_the_read_the_same_way` と同じ形）。
    server
        .plant_vote(
            character,
            session,
            "   ",
            DriftDelta::Up,
            "2026-08-27T16:00:01.000Z",
        )
        .await;

    let (status, body) = server.get(&drift_path(HANDLE)).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "1行読めなければ丸ごと止まること: {body}"
    );
    assert_no_internals(&server, &body);

    assert_eq!(
        server
            .store
            .count_drift_votes(character)
            .await
            .expect("総数は数えられること"),
        2,
        "総数は軸名を読まないので止まらないこと"
    );
    assert!(
        matches!(
            server.store.recent_drift_votes(character, 32).await,
            Err(StoreError::InvalidStoredValue)
        ),
        "軸名が壊れていれば蓄積の読みも止まること（同じ検証を通すため）"
    );

    server.stop().await;
}

/// **上限の外にある壊れた行でも、この一覧は止まる**（集計は上限を持たないため）。
///
/// # なぜ別のテストが要るのか
///
/// 上の2本は**履歴の読みが先に落ちる**ので、
/// **集計の読みだけを読み飛ばしに変える変異が通り抜ける**（履歴が落ち続けて 500 のまま）。
/// **壊れた行を直近の窓の外へ置く**と、履歴はその行を一度も見ない——
/// **止まるのは集計だけ**になり、そこを緩める変異が初めて観測できる。
///
/// 窓の外に押し出すために、**上限より多い新しい票**を積む（直書きの 50 件）。
/// 上限を大きくしても、壊れた行が窓に入って**履歴の側が落ちるだけ**なので、
/// このテストが偽の赤になることは無い。
#[tokio::test]
async fn a_broken_row_outside_the_recent_window_still_stops_the_listing() {
    let server = TestServer::start().await;
    let character = server.create_character(HANDLE, DISPLAY_NAME).await;
    server.put_stats(HANDLE, &[(AXIS_PASSION, 3)]).await;
    let session = server
        .store
        .create_session(character)
        .await
        .expect("セッションを作成できること");

    // **いちばん古い位置に壊れた行を置く。**
    server
        .plant_vote(
            character,
            session,
            "   ",
            DriftDelta::Down,
            "2026-08-27T17:00:00.000Z",
        )
        .await;
    for index in 0..50 {
        server
            .plant_vote(
                character,
                session,
                AXIS_PASSION,
                DriftDelta::Up,
                &format!("2026-08-27T18:00:{index:02}.000Z"),
            )
            .await;
    }

    // **前提: 履歴の読みはこの行を見ていない。**
    assert_eq!(
        server
            .store
            .drift_vote_history(character, 50)
            .await
            .expect("履歴の読みは止まらないこと")
            .len(),
        50,
        "壊れた行は直近の窓の外にあること"
    );
    // **前提: 総数はそれでも全部を数える。**
    assert_eq!(
        server
            .store
            .count_drift_votes(character)
            .await
            .expect("総数は数えられること"),
        51,
        "窓の外の票も数に入ること"
    );

    let (status, body) = server.get(&drift_path(HANDLE)).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "集計は上限を持たないので、窓の外の壊れた行で止まること: {body}"
    );
    assert_no_internals(&server, &body);

    server.stop().await;
}

// =========================================================================
// 割り当てを失った軸（作業単位 B-16）
//
// **`PUT .../stats` は全置換なので、軸を改名すると綴りが変わる。**
// `stat_drift_charge` / `stat_drift_vote` には `stat_alloc` への外部キーが無いので
// 行は消えず、**総数には数えられ続けるのに、どの軸の行にもならない**状態になる
// （D-4 が `sqlite3` で実測）。
//
// このリポジトリは一貫して「**存在する行は落とさずに指し示す**」立場を取る
// （`list_characters` / `list_memories` の `unreadable`、値 0 の軸、
// 上書きされた記憶）。**ここも落とさない。ただし `value` を `null` にして、
// 「割り当てが無い」と応答から判別できる形で返す。**
// =========================================================================

/// 割り当てを失った軸のフィクスチャで仕込む票（**古い順**）。
///
/// 軸は**混ぜてある**——1つの軸の票が固まっていると、
/// 「入った順に軸をまとめる」実装と区別が付かない。
const ORPHAN_BALLOTS: [(&str, DriftDelta); 16] = [
    (AXIS_PASSION, DriftDelta::Up),
    (LOST_BOTH_AXIS, DriftDelta::Up),
    (AXIS_LOGIC, DriftDelta::Up),
    (RETIRED_AXIS, DriftDelta::Down),
    (AXIS_PASSION, DriftDelta::Down),
    (LOST_BOTH_AXIS, DriftDelta::Down),
    (AXIS_PASSION, DriftDelta::Up),
    (LOST_BOTH_AXIS, DriftDelta::Up),
    (RETIRED_AXIS, DriftDelta::Down),
    (AXIS_PASSION, DriftDelta::Up),
    (LOST_BOTH_AXIS, DriftDelta::Up),
    (AXIS_PASSION, DriftDelta::Down),
    (LOST_BOTH_AXIS, DriftDelta::Up),
    (RETIRED_AXIS, DriftDelta::Down),
    (AXIS_PASSION, DriftDelta::Up),
    (LOST_BOTH_AXIS, DriftDelta::Up),
];

/// 葵の応答に並ぶべき軸の行（**この形が正**）。
///
/// # 同値を作らない
///
/// 値（0 / 3）・累積（0.5 / -0.25 / 1.5 / 0.0 / -2.75）・上げ（1 / 4 / 0 / 0 / 5）・
/// 下げ（0 / 2 / 0 / 3 / 1）は**取り違えたら必ずどこかが動く**ように散らしてある。
///
/// # 並びの主張は3つ
///
/// 1. **割り当てのある軸が先**。`stat_alloc` の保存順（`ロジック` → `パッション`）。
/// 2. **割り当ての無い軸が後**。**軸名の昇順**（`きえた` → `むかしの` → `わすれた`）。
/// 3. 1 と 2 は**混ざらない**。全体を名前で並べると、捨て軸3つが**先頭へ来る**
///    （`き`/`む`/`わ` はどれも `パ` U+30D1・`ロ` U+30ED より小さい）。
fn expected_orphan_axes() -> Value {
    json!([
        {"axis": AXIS_LOGIC, "value": 0, "charge": 0.5, "ups": 1, "downs": 0},
        {"axis": AXIS_PASSION, "value": 3, "charge": -0.25, "ups": 4, "downs": 2},
        {"axis": LOST_CHARGED_AXIS, "value": null, "charge": 1.5, "ups": 0, "downs": 0},
        {"axis": RETIRED_AXIS, "value": null, "charge": 0.0, "ups": 0, "downs": 3},
        {"axis": LOST_BOTH_AXIS, "value": null, "charge": -2.75, "ups": 5, "downs": 1},
    ])
}

/// 割り当てを失った軸を持つキャラクターと、**同じ形の傍観者**。
struct OrphanFixture {
    server: TestServer,
    /// 票を仕込んだ往復（`session_id` の帰属に使う）。
    session: SessionId,
}

/// **3種類の「割り当ての無い軸」を作る。** 3つは別の状態である。
///
/// | 軸 | 累積 | 票 |
/// |---|---|---|
/// | `LOST_CHARGED_AXIS` | **ある** | 無い |
/// | `RETIRED_AXIS` | 無い | **ある** |
/// | `LOST_BOTH_AXIS` | **ある** | **ある** |
///
/// # 届いた順と、返すべき順を**わざとずらす**
///
/// * 割り当ては**名前の降順**で置く（`ロジック` → `パッション`）。
///   全体を名前で並べる実装は、この2つが入れ替わって赤になる。
/// * 累積は `きえた` と `わすれた`、票は `むかしの` と `わすれた`。
///   **累積 → 票の順に繋いだだけ**の実装は `きえた` → `わすれた` → `むかしの` になる。
///
/// # 傍観者も同じ形を持つ
///
/// `BYSTANDER_LOST_AXIS` は**傍観者にだけ**累積と票がある。
/// どれかの読みから `character_id` の絞り込みが消えると、
/// **葵の一覧の3番目と4番目のあいだ**に現れる（`し` U+3057）。
async fn orphans() -> OrphanFixture {
    let server = TestServer::start().await;

    // **傍観者を先に作る**（票が立つ側の id を 1 にしない）。
    let bystander = server
        .create_character(BYSTANDER_HANDLE, BYSTANDER_DISPLAY_NAME)
        .await;
    let character = server.create_character(HANDLE, DISPLAY_NAME).await;
    assert_ne!(character, bystander, "前提: 2体の id が別であること");

    // **保存順を名前の昇順とはずらす。**
    server
        .put_stats(HANDLE, &[(AXIS_LOGIC, 0), (AXIS_PASSION, 3)])
        .await;
    server
        .put_stats(BYSTANDER_HANDLE, &[(BYSTANDER_AXIS, 4)])
        .await;

    // **傍観者の往復を3つ先に作る**（＝ `session_id` が `character_id` とずれる）。
    let mut bystander_sessions = Vec::new();
    for _ in 0..3 {
        bystander_sessions.push(
            server
                .store
                .create_session(bystander)
                .await
                .expect("セッションを作成できること"),
        );
    }
    let bystander_session = bystander_sessions[0];
    let session = server
        .store
        .create_session(character)
        .await
        .expect("セッションを作成できること");
    assert_ne!(
        session.get(),
        character.get(),
        "前提: 往復の id と持ち主の id が別の値であること"
    );

    for (index, (axis, delta)) in ORPHAN_BALLOTS.iter().enumerate() {
        server
            .plant_vote(
                character,
                session,
                axis,
                *delta,
                &format!("2026-08-28T09:00:{index:02}.000Z"),
            )
            .await;
    }

    // **累積は4軸。** 割り当てのある2軸と、割り当ての無い2軸。
    server
        .save_charges(
            character,
            &[
                (AXIS_LOGIC, 0.5),
                (AXIS_PASSION, -0.25),
                (LOST_CHARGED_AXIS, 1.5),
                (LOST_BOTH_AXIS, -2.75),
            ],
        )
        .await;

    // **傍観者の捨て軸。** 葵の側へ混ざったら一覧の途中に現れる。
    server
        .save_charges(bystander, &[(BYSTANDER_LOST_AXIS, 3.25)])
        .await;
    server
        .plant_vote(
            bystander,
            bystander_session,
            BYSTANDER_LOST_AXIS,
            DriftDelta::Up,
            "2026-08-28T10:00:00.000Z",
        )
        .await;

    OrphanFixture { server, session }
}

/// **累積だけの軸も、票だけの軸も、両方ある軸も、落とさずに返る。**
///
/// # 3つは別の状態である（片方だけ通す実装を落とす）
///
/// * `LOST_CHARGED_AXIS` —— 累積の側だけを見ている実装なら出る。
/// * `RETIRED_AXIS` —— 票の側だけを見ている実装なら出る。
/// * `LOST_BOTH_AXIS` —— どちらか片方でも出るので、**この軸だけでは何も守れない。**
///
/// # ここで初めて観測できること
///
/// **是正前は、この3軸がどれも応答に現れなかった**（`stat_alloc` だけが軸の一覧を決めていた）。
/// それでいて `total_votes` は 16 を名乗る。**9 票がどの行にも現れない状態**だった。
#[tokio::test]
async fn an_axis_with_only_a_charge_and_an_axis_with_only_votes_are_both_listed() {
    let fixture = orphans().await;

    let (status, body) = fixture.server.get(&drift_path(HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "一覧を引けること: {body}");

    assert_eq!(
        axes_without_times(&body),
        expected_orphan_axes(),
        "軸の一覧が、割り当ての有無を含めてそのまま返ること: {body}"
    );

    fixture.server.stop().await;
}

/// **「値が 0 の軸」と「割り当てが無い軸」は、応答の上で別物である**（B-16 の条件1）。
///
/// # 0 で埋めてはいけない理由
///
/// `0` は**人が書いた宣言**（「この軸は語らない」/ ADR-0024 §4）で、
/// **変容の対象外**という意味を持つ。割り当てを失った軸を `0` として混ぜると、
/// 画面には**同じ行**として出る——「まだ動かない軸」と「もう無い軸」の区別が消え、
/// **`PUT .../stats` で直すべきかどうかを読み手が判断できなくなる。**
///
/// # `null` を選んだ理由
///
/// **型が変わるので、数として読む経路は必ず壊れる。**
/// 欄を省く（`value` を載せない）形にすると、読み手が `?? 0` で埋めた瞬間に
/// **静かに 0 と混ざる**——それは「落とさずに指し示す」ことにならない。
///
/// # 同じ応答の中で並べて見る
///
/// 別々のテストに分けると、**両方を `0` にする実装**でも片方ずつは緑になる。
#[tokio::test]
async fn a_zero_valued_axis_is_never_confused_with_an_axis_that_has_no_allocation() {
    let fixture = orphans().await;

    let (_, body) = fixture.server.get(&drift_path(HANDLE)).await;
    let rows = axes(&body);

    let zero = rows
        .iter()
        .find(|row| row.get("axis") == Some(&json!(AXIS_LOGIC)))
        .unwrap_or_else(|| panic!("値 0 の軸が居ること: {body}"));
    let lost = rows
        .iter()
        .find(|row| row.get("axis") == Some(&json!(RETIRED_AXIS)))
        .unwrap_or_else(|| panic!("割り当ての無い軸が居ること: {body}"));

    assert_eq!(
        zero.get("value"),
        Some(&json!(0)),
        "値 0 の軸は 0 を名乗ること: {zero}"
    );
    assert_eq!(
        lost.get("value"),
        Some(&Value::Null),
        "割り当ての無い軸は値を名乗らないこと: {lost}"
    );
    assert_ne!(
        zero.get("value"),
        lost.get("value"),
        "2つが同じ形で返っています（区別が消えました）: {body}"
    );

    fixture.server.stop().await;
}

/// **総数に数えた票は、必ずどれかの軸の行に現れる**（B-16 の条件2）。
///
/// # この状態でしか観測できない
///
/// 割り当てを失った軸の票が**9件**ある（`RETIRED_AXIS` の 3 と `LOST_BOTH_AXIS` の 6）。
/// 是正前は総数 16 に対して軸の行の合計が 7 で、**差の 9 がどこにも現れなかった。**
/// 割り当てのある軸だけのフィクスチャでは、**この差が 0 なので何も守れない。**
///
/// # 期待値を仕込んだ数からも定数からも作らない
///
/// 16 は**直書き**（応答が総数を正しく数えていることの外側の押さえ）で、
/// 辻褄そのものは**応答の別々の欄どうし**で突き合わせる。
#[tokio::test]
async fn every_vote_in_the_total_shows_up_in_some_axis_row() {
    let fixture = orphans().await;

    let (_, body) = fixture.server.get(&drift_path(HANDLE)).await;

    assert_eq!(
        body.get("total_votes"),
        Some(&json!(16)),
        "総数は保存されている全部であること: {body}"
    );
    assert_every_vote_belongs_to_an_axis(&body);

    // **割り当ての無い軸が持っている票の数**（落とすと辻褄が 9 ずれる）。
    let rows = axes(&body);
    let unallocated: u64 = rows
        .iter()
        .filter(|row| row.get("value") == Some(&Value::Null))
        .map(|row| {
            row.get("ups").and_then(Value::as_u64).unwrap_or_default()
                + row.get("downs").and_then(Value::as_u64).unwrap_or_default()
        })
        .sum();
    assert_eq!(unallocated, 9, "割り当ての無い軸が抱えている票の数: {body}");

    fixture.server.stop().await;
}

/// **並びは問い合わせ計画に依存しない**（B-16 の条件3）。
///
/// # 決めた並び
///
/// **割り当てのある軸が先**（`stat_alloc` の保存順）、
/// **割り当ての無い軸が後**（**軸名の昇順**）。
///
/// # 同着は起きない
///
/// 並べる鍵は**軸名**で、軸名は1体のキャラクターの中で一意である
/// （`stat_alloc` は主キー、`stat_drift_charge` も主キー、集計は軸ごとに畳まれる）。
/// **第2の鍵が要らないのはこのため**で、
/// 「同着の決め手が無い」ではなく「**同着が作れない**」。
///
/// # なぜ索引を落とすのか（`ties_are_ordered_by_id_even_when_the_index_is_not_used` と同じ理由）
///
/// **索引を辿る計画では、並べていなくても答えが偶然そろう**（`D-3b` / `D-4a` の実測）。
/// 累積は主キー `(character_id, axis)` の索引を辿るので**軸名の昇順で返り**、
/// 集計も `GROUP BY axis` の一時 b-tree で軸名の昇順になる。
/// **つまり「並べていない実装」でも、片方だけ見れば正しく見える。**
///
/// 落とせる索引は票の側（`stat_drift_vote_recent_idx`）だけ
/// ——**主キーの索引は SQLite が落とさせない**ので、
/// **計画に依存しないことの主な押さえは「2つの読みを繋いだ順とは違う順で返る」こと**である
/// （`きえた` → `わすれた` → `むかしの` にならない）。
#[tokio::test]
async fn the_axes_without_an_allocation_come_last_in_name_order() {
    let fixture = orphans().await;

    let names = |body: &Value| -> Vec<Value> {
        axes(body)
            .iter()
            .filter_map(|row| row.get("axis").cloned())
            .collect()
    };
    let expected = vec![
        json!(AXIS_LOGIC),
        json!(AXIS_PASSION),
        json!(LOST_CHARGED_AXIS),
        json!(RETIRED_AXIS),
        json!(LOST_BOTH_AXIS),
    ];

    let (_, before) = fixture.server.get(&drift_path(HANDLE)).await;
    assert_eq!(names(&before), expected, "索引がある状態での並び: {before}");

    // **計画を変える。**
    sqlx::query("DROP INDEX stat_drift_vote_recent_idx")
        .execute(fixture.server.store.pool())
        .await
        .expect("索引を落とせること");

    let (_, after) = fixture.server.get(&drift_path(HANDLE)).await;
    assert_eq!(
        names(&after),
        expected,
        "索引が無くても同じ並びであること: {after}"
    );
    assert_eq!(
        after.get("axes"),
        before.get("axes"),
        "索引の有無で軸の行が変わらないこと"
    );

    fixture.server.stop().await;
}

/// **割り当てのある軸は、いまも `GET .../stats` と1行ずつ揃う。**
///
/// # いまの振る舞いが変わっていないことの押さえ
///
/// 画面は2つの入口を並べて出す前提なので、**先頭から揃っていないと行がずれる**。
/// 割り当ての無い軸を**先頭や途中に挿す実装**は、ここで赤になる
/// （`the_axes_match_the_stats_endpoint` は捨て軸の無いフィクスチャなので観測できない）。
///
/// **`GET .../stats` の側は1行も増えない**——あちらは `stat_alloc` だけを返す。
#[tokio::test]
async fn the_allocated_axes_still_line_up_with_the_stats_endpoint() {
    let fixture = orphans().await;

    let (_, drift) = fixture.server.get(&drift_path(HANDLE)).await;
    let (_, stats) = fixture.server.get(&stats_path(HANDLE)).await;

    let rows = axes(&drift);
    let allocated: Vec<Value> = rows
        .iter()
        .take_while(|row| row.get("value") != Some(&Value::Null))
        .map(|row| json!({"axis": row.get("axis"), "value": row.get("value")}))
        .collect();
    let from_stats: Vec<Value> = stats
        .get("stats")
        .expect("stats を載せること")
        .as_array()
        .expect("stats は配列であること")
        .iter()
        .map(|stat| json!({"axis": stat.get("axis"), "value": stat.get("value")}))
        .collect();

    assert_eq!(
        allocated, from_stats,
        "割り当てのある軸が、先頭から `GET .../stats` と揃うこと: {drift} / {stats}"
    );
    assert_eq!(
        from_stats.len(),
        2,
        "前提: `GET .../stats` は捨て軸を返さないこと: {stats}"
    );
    assert_eq!(
        rows.len(),
        5,
        "前提: こちらは捨て軸のぶんだけ行が多いこと: {drift}"
    );

    fixture.server.stop().await;
}

/// **傍観者の捨て軸は、葵の一覧に現れない**（`character_id` の絞り込み）。
///
/// # 是正で増えた経路を、持ち主の側から見る
///
/// 軸の一覧が `stat_alloc` だけで決まっていたころは、
/// **累積や集計の絞り込みが消えても「知らない軸」は増えなかった**
/// （値の取り違えとしてしか現れなかった）。
/// **いまは行そのものが増える**ので、絞り込みが消えたら一覧の形が変わる。
///
/// 傍観者の側でも同じ形（割り当て1軸＋捨て軸1軸）を確かめる。
#[tokio::test]
async fn another_characters_orphan_axes_never_appear() {
    let fixture = orphans().await;

    let (_, aoi) = fixture.server.get(&drift_path(HANDLE)).await;
    let aoi_names: Vec<Value> = axes(&aoi)
        .iter()
        .filter_map(|row| row.get("axis").cloned())
        .collect();
    assert!(
        !aoi_names.contains(&json!(BYSTANDER_LOST_AXIS)),
        "傍観者の捨て軸が混ざっています: {aoi_names:?}"
    );
    assert!(
        !aoi_names.contains(&json!(BYSTANDER_AXIS)),
        "傍観者の軸が混ざっています: {aoi_names:?}"
    );

    let (status, other) = fixture.server.get(&drift_path(BYSTANDER_HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "傍観者の一覧も引けること: {other}");
    assert_eq!(
        axes_without_times(&other),
        json!([
            {"axis": BYSTANDER_AXIS, "value": 4, "charge": 0.0, "ups": 0, "downs": 0},
            {"axis": BYSTANDER_LOST_AXIS, "value": null, "charge": 3.25, "ups": 1, "downs": 0},
        ]),
        "傍観者の捨て軸は傍観者の側にだけ出ること: {other}"
    );
    assert_eq!(
        other.get("total_votes"),
        Some(&json!(1)),
        "傍観者の総数は傍観者の票だけであること: {other}"
    );
    assert_every_vote_belongs_to_an_axis(&other);

    fixture.server.stop().await;
}

/// **捨て軸の票も、直近の一覧の中では往復と時刻をそのまま保つ。**
///
/// 軸の行が増えたことで、**票の一覧の側が壊れていない**ことを別に見ておく
/// （`recent_votes` は `stat_alloc` を参照しない）。
#[tokio::test]
async fn the_recent_list_still_carries_the_votes_of_a_lost_axis() {
    let fixture = orphans().await;

    let (_, mut body) = fixture.server.get(&drift_path(HANDLE)).await;
    let (votes, times) = split_votes(&mut body);

    assert_eq!(votes.len(), 16, "16 件とも返ること（上限 50 の内側）");
    assert_eq!(
        votes.first(),
        Some(&expected_vote(
            LOST_BOTH_AXIS,
            DriftDelta::Up,
            fixture.session
        )),
        "いちばん新しい票が捨て軸のものであること: {votes:?}"
    );
    assert_eq!(
        times.first(),
        Some(&instant("2026-08-28T09:00:15.000Z")),
        "時刻がその行のものであること: {times:?}"
    );
    assert!(
        times.windows(2).all(|pair| pair[0] > pair[1]),
        "新しい順であること: {times:?}"
    );

    fixture.server.stop().await;
}

/// **正規化すると同じ軸になる行が2つあれば、2つの入口が揃って止まる**（作業単位 B-19）。
///
/// # 何が起きていたか（B-16 のレビューの実測。推測ではない）
///
/// `stat_alloc` に `パッション` と ` パッション` を直接置くと、
/// `Store::stat_allocs` が**両方を `StatAxis::parse` で正規化して受理する。**
/// その結果 `GET .../drift` は**同じ集計を2回引き当て**、
/// `ups 2 / downs 1` の行が2つ並んで**総和 6 が総数 3 を超えた。**
/// `GET .../stats` の側でも同じ軸が2行並び、**どちらの値が本当かを応答から決められない。**
///
/// **`stat_alloc` の主キーは `(character_id, axis)`** なので DB は一意のつもりでいる。
/// **一意でなくしていたのは読み出し側の正規化**である。
///
/// # なぜ2つの入口を**1本のテスト**で見るのか
///
/// 片方ずつ別のテストに分けると、**片方の入口にだけ検査を置く実装が半分ずつ緑になる。**
/// 2つの入口の意味が食い違うことこそが避けたい状態なので、同じ状態を両方に通す。
///
/// # **API の入口が残っていること**（**この作業単位の条件3**）
///
/// `PUT .../stats` は**既存の軸を読まない**（全消し → 挿入）ので、重複したままでも届く。
/// **応答は読み直したものを返す**（`replace_stats`）ので、
/// **直った直後に `Store::stat_allocs` をもう一度通っている**——
/// ここが 200 で返ることが「API の入口が閉じていない」ことの観測である。
/// **ADR-0016 §4 の「直したい行ほど読めない」に落ちていない。**
///
/// # **⚠ ただし画面からは辿れない**（B-19 のレビューの実測 / 2026-08-28）
///
/// **ここが見ているのは HTTP の入口だけで、利用者の導線ではない。**
/// `web/src/lib/StatsPanel.svelte` は `statsState.kind === "failed"` の枝で
/// **エラーの `<p>` しか描かない** ——「軸を追加」も「この内容で置き換える」も
/// `{:else}` 側のフォームの中にしか無く、**`replaceStats` の呼び出し元はその1箇所だけ。**
/// **その振る舞いは FE 側で「期待される形」として固定済み**
/// （`web/tests/stats-panel.test.ts` の
/// 「読めなかったとき」→「いまの値が読めなくても、変容の側は出る（逆も同じ）」）。
/// **この状態を画面から直すことはできない**（`curl` か `sqlite3` が要る）。
/// **画面の導線は積むだけで、この作業単位では実装していない。**
///
/// # 基準を先に取る
///
/// 重複を置く**前**に両方の入口が 200 で答えることを見ておく。
/// 見ておかないと、フィクスチャが別の理由で 500 になっていても気付けない。
#[tokio::test]
async fn two_rows_that_normalize_to_the_same_axis_stop_both_endpoints_until_the_stats_are_replaced()
{
    let server = TestServer::start().await;
    // **傍観者を先に作る**（票が立つ側の id を 1 にしない）。
    let bystander = server
        .create_character(BYSTANDER_HANDLE, BYSTANDER_DISPLAY_NAME)
        .await;
    let character = server.create_character(HANDLE, DISPLAY_NAME).await;
    assert_ne!(character, bystander, "前提: 2体の id が別であること");

    let axes = json!([
        {"axis": AXIS_PASSION, "value": 3},
        {"axis": AXIS_LOGIC, "value": 0},
    ]);
    server
        .put_stats(HANDLE, &[(AXIS_PASSION, 3), (AXIS_LOGIC, 0)])
        .await;
    // **軸の綴りを傍観者と共有する**（過剰検出しないことの対照）。
    // **⚠ 「持ち主で絞らずに数える実装」は書けない** ——
    // `SELECT_STAT_ALLOCS` が既に `WHERE character_id = ?` で絞っているので、
    // その変異は SQL の WHERE を落とすことになり、無関係な数十本が先に赤くなる。
    server
        .put_stats(BYSTANDER_HANDLE, &[(AXIS_PASSION, 4)])
        .await;

    let session = server
        .store
        .create_session(character)
        .await
        .expect("セッションを作成できること");
    for (index, delta) in [DriftDelta::Up, DriftDelta::Up, DriftDelta::Down]
        .into_iter()
        .enumerate()
    {
        server
            .plant_vote(
                character,
                session,
                AXIS_PASSION,
                delta,
                &format!("2026-08-28T11:00:{index:02}.000Z"),
            )
            .await;
    }

    // --- 基準: 重複を置く前は、両方の入口が答えられる ---
    let (status, stats) = server.get(&stats_path(HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "前提: 値を引けること: {stats}");
    assert_eq!(
        stats.get("stats"),
        Some(&axes),
        "前提: 2軸が1つずつ: {stats}"
    );
    let (status, drift) = server.get(&drift_path(HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "前提: 変容を引けること: {drift}");
    assert_eq!(
        drift.get("total_votes"),
        Some(&json!(3)),
        "前提: 票は3件: {drift}"
    );
    assert_every_vote_belongs_to_an_axis(&drift);

    // --- 重複する行を1つ置く（`StatAxis::parse` を通すと同じ軸になる綴り） ---
    let padded = format!(" {AXIS_PASSION}");
    assert_eq!(
        StatAxis::parse(&padded),
        StatAxis::parse(AXIS_PASSION),
        "前提: 正規化すると同じ軸になること"
    );
    // **値をずらす**（同じ値だと、落として1行にする実装と区別が付かない）。
    insert_stat_alloc_directly(&server.store, character.get(), &padded, 5).await;

    for path in [stats_path(HANDLE), drift_path(HANDLE)] {
        let (status, body) = server.get(&path).await;
        assert_eq!(
            status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "重複した軸のまま答えないこと: {path}: {body}"
        );
        assert_eq!(
            body,
            json!({"error": {"message": "内部エラーが発生しました", "type": "server_error"}}),
            "理由を反射しないこと: {path}"
        );
        assert_no_internals(&server, &body);
    }

    // **傍観者は止まらない。** 同じ綴りの軸を持っていても巻き込まれない。
    let (status, other) = server.get(&drift_path(BYSTANDER_HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "傍観者は引けること: {other}");
    assert_eq!(
        axes_without_times(&other),
        json!([
            {"axis": AXIS_PASSION, "value": 4, "charge": 0.0, "ups": 0, "downs": 0},
        ]),
        "傍観者の軸はそのまま返ること: {other}"
    );

    // --- 直す: `PUT .../stats` は既存の軸を読まないので届く ---
    let (status, replaced) = server
        .put(
            &stats_path(HANDLE),
            &json!({"stats": [
                {"axis": AXIS_PASSION, "value": 3},
                {"axis": AXIS_LOGIC, "value": 0},
            ]}),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "重複したままでも置き換えは届くこと（応答は読み直したもの）: {replaced}"
    );
    assert_eq!(
        replaced.get("stats"),
        Some(&axes),
        "置き換えた後の応答が、軸を1つずつだけ載せること: {replaced}"
    );

    let (status, stats) = server.get(&stats_path(HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "直した後は引けること: {stats}");
    assert_eq!(stats.get("stats"), Some(&axes), "値が戻ること: {stats}");

    let (status, drift) = server.get(&drift_path(HANDLE)).await;
    assert_eq!(status, StatusCode::OK, "変容も引けること: {drift}");
    assert_eq!(
        axes_without_times(&drift),
        json!([
            {"axis": AXIS_PASSION, "value": 3, "charge": 0.0, "ups": 2, "downs": 1},
            {"axis": AXIS_LOGIC, "value": 0, "charge": 0.0, "ups": 0, "downs": 0},
        ]),
        "軸ごとの集計が1つずつだけ現れること（是正前はこの行が2つ並んだ）: {drift}"
    );
    assert_eq!(
        drift.get("total_votes"),
        Some(&json!(3)),
        "票は消えていないこと: {drift}"
    );
    assert_every_vote_belongs_to_an_axis(&drift);

    server.stop().await;
}

/// **応答が、このキャラクターに効いている走らせ方・その出どころ・起動時の既定を名乗る**
/// （ADR-0045 §決定4 / 作業単位 A-8）。**同じサーバで `PATCH` し、次の読みから変わる。**
///
/// # 腕の選び方
///
/// * **起動時の既定を `shadow` にする**（`off` にすると、既定に決め打ちする実装と
///   「未設定の読み」が区別できない）。
/// * **キャラクターの値は既定の両側**（`on` と `off`）へ振る。
/// * **`null` で戻したら、既定へ戻る**（出どころも `default` へ戻る）。
///
/// **綴りは直書き。**
#[tokio::test]
async fn the_response_names_the_effective_run_mode_its_source_and_the_default() {
    let server = TestServer::start_with_mode(Some("shadow")).await;
    server.create_character(HANDLE, DISPLAY_NAME).await;

    let read = || async {
        let (status, body) = server.get(&drift_path(HANDLE)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        [
            body["run_mode"].clone(),
            body["run_mode_source"].clone(),
            body["default_run_mode"].clone(),
        ]
    };
    let patch = |mode: Value| {
        let server = &server;
        async move {
            let (status, body) = server
                .send(
                    Method::PATCH,
                    &format!("/admin/characters/{HANDLE}"),
                    Some(&json!({"stat_drift_mode": mode})),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
    };

    assert_eq!(
        read().await,
        [json!("shadow"), json!("default"), json!("shadow")],
        "未設定は起動時の既定に従う"
    );

    patch(json!("on")).await;
    assert_eq!(
        read().await,
        [json!("on"), json!("character"), json!("shadow")],
        "キャラクターの値が既定に勝つ（既定より強い側）"
    );

    patch(json!("off")).await;
    assert_eq!(
        read().await,
        [json!("off"), json!("character"), json!("shadow")],
        "キャラクターの値が既定に勝つ（既定より弱い側）"
    );

    patch(Value::Null).await;
    assert_eq!(
        read().await,
        [json!("shadow"), json!("default"), json!("shadow")],
        "null で未設定へ戻すと既定に従う"
    );

    server.stop().await;
}
