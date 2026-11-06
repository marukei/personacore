//! `session` / `message` の読み書き。
//!
//! **メッセージを `message.rs` に分けていないのは意図的。** ADR-0004 により
//! 「メッセージ追加」と「`session.updated_at` の更新」は同一トランザクションの1操作で
//! なければならず、**呼び出し側が片方だけ実行できない形**にする必要がある。
//! モジュールを分けると `updated_at` を触らないメッセージ追加 API が自然に生えるため、
//! 同じ場所に置いて分離できなくしてある。
//!
//! この層は薄く保つ。`&str` や裸の `i64` を受ける公開 API を作らないのは
//! `character.rs` と同じ（`CharacterId` / `SessionId` / `MessageId` は別型）。

use chrono::{DateTime, Utc};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::{Store, StoreError};
use crate::domain::{CharacterId, Message, MessageId, NewMessage, Role, SessionId};

/// `session` テーブルの1行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// 主キー。
    pub id: SessionId,
    /// どのキャラクターの会話か。
    pub character_id: CharacterId,
    /// 人間向けの識別名。未設定なら `None`。
    pub label: Option<String>,
    /// 作成日時（UTC）。
    pub created_at: DateTime<Utc>,
    /// **最終更新日時（UTC）。`aoi` の「最新セッション」はこの列で決まる**（ADR-0004）。
    pub updated_at: DateTime<Utc>,
}

/// `append_turn` が書き込んだ往復の id。
///
/// **`(MessageId, MessageId)` のタプルにしない。** 2つとも同じ型なので、
/// 受け取り側で順序を取り違えても型検査は通り、`memory` の紐付け先が
/// 静かに入れ替わる（`MessageId` を `CharacterId` / `SessionId` と別型にしてあるのと同じ論法）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppendedTurn {
    /// 利用者の発言（先に入る方）。
    pub user: MessageId,
    /// 応答（後に入る方）。
    pub reply: MessageId,
}

/// 会話の一覧の1行（[`Store::list_sessions`] / 作業単位 D-12c-1）。
///
/// **書き込みの口は無い。** これは読むための姿で、`session` の行を変える経路は
/// 今までどおり `append_message` / `append_turn`（`updated_at` の押し上げ）だけである。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// セッションそのもの。
    pub session: Session,
    /// そのセッションのメッセージの件数（利用者と応答の両方）。
    pub message_count: u32,
    /// **最初の利用者の発言の先頭**（選ぶための手がかり）。利用者の発言が無ければ `None`。
    pub first_user_message: Option<OpeningText>,
}

/// 上限で切った本文の先頭（[`SessionSummary::first_user_message`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpeningText {
    /// 先頭から、頼んだ文字数まで。
    pub text: String,
    /// **切ったかどうか。** `text` の長さからは「ちょうどその長さだった」と区別できない。
    pub truncated: bool,
}

/// 1ページぶんのメッセージ（[`Store::message_page`] / 作業単位 D-12c-1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagePage {
    /// **古い順**（画面は上から古い順に積む）。
    pub messages: Vec<Message>,
    /// **このページより古いメッセージが残っているか。**
    ///
    /// 1件多く頼んで決める——総数との差で決めると、読んでいる間に増えた分で狂う。
    pub has_older: bool,
}

/// 日時の書式。**`migrations/0001_initial.sql` の DEFAULT と同一にすること。**
///
/// `updated_at` の大小比較は TEXT の辞書順で行われるため、桁数の違う書式が混ざると
/// 「最新セッション」の解決が静かに狂う。SQL には**プレースホルダで渡す**ので、
/// 書式の定義はこの1箇所だけ。
///
/// 一致は `tests/store_session.rs::updated_at_uses_the_same_timestamp_format_as_the_schema_default`
/// が固定している（同じ行の `created_at`（DEFAULT）と `updated_at`（この定数）を比べる）。
const TIMESTAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%fZ";

/// **読みから始まって書きに昇格するトランザクションの開始文**（ADR-0004）。
///
/// SQLite の既定（`BEGIN DEFERRED`）は最初の読みで読み取りスナップショットを固定し、
/// その後の書きで `SQLITE_BUSY_SNAPSHOT`(517) を返す。これは
/// **`busy_timeout` の対象外で、待たずに即座に失敗する。**
/// 最初から書きロックを取っておけば、競合しても `busy_timeout` の範囲で待てる。
///
/// 逆に「最初の文が書き」のトランザクション（`append_message` / `delete_character`）には
/// 不要。**DELETE は0行しか消さない場合でも書きロックを取る**ので、
/// 「0件のときだけ読みから始まる」ということは起こらない（実測は `delete_character` の doc）。
///
/// # 将来の最適化を入れるときの注意
///
/// この開始文は「既存セッションを読むだけ」の経路でも書きロックを取るため、
/// 最も多く通る `aoi` の同時実行が直列化される。現状はトランザクション内が
/// SELECT 1本だけ（そのキャラクターのセッションを整列する / `SELECT_LATEST_SESSION`）で、
/// 保持時間がサブミリ秒なので問題にならない。
///
/// 負荷が上がって「まずトランザクション外で読み、`None` のときだけ
/// `BEGIN IMMEDIATE` で入り直す」二段構えにする場合、
/// **入り直した後のトランザクション内での再読み（double-check）を絶対に省略しないこと。**
/// 「外で `None` と分かっているのだから」と省くと、2本が同時に `None` を見て
/// 両方 INSERT し、**空セッションが2つできて会話が分断される。**
/// 逐次実行のテストでは検出できず、
/// `latest_or_create_survives_concurrent_callers` だけが唯一の防波堤になる。
const BEGIN_IMMEDIATE: &str = "BEGIN IMMEDIATE";

/// セッションの作成。`created_at` はスキーマの DEFAULT に任せる。
///
/// `updated_at` は「現在時刻」と「そのキャラクターの現在の最大値 + 1ms」の**大きい方**。
/// こうしないと、同一ミリ秒内に2つセッションを作ったとき `updated_at` が同値になり、
/// 「最新セッション」が不定になる（`0001` の日時はミリ秒精度）。
///
/// `?1` = 日時書式 / `?2` = キャラクター id。
/// 既存行が無ければ副問合せは NULL になるので `coalesce` で空文字に落とす
/// （`max(x, NULL)` は NULL になり NOT NULL 違反になるため）。
///
/// **`coalesce` が守るのは NOT NULL 違反だけで、単調増加は守らない。**
/// 格納値が日時として解釈できない場合（`'not-a-date'`）や、1ms 加算が
/// 表現範囲を超える場合（`'9999-12-31T23:59:59.999Z'`）も `strftime` は NULL を返すが、
/// このとき `max(現在時刻, '')` = 現在時刻になるため **`updated_at` は実時刻へ巻き戻り、
/// そのセッションは最新でなくなる。** `store` 経由の書き込みではこの状態を作れず、
/// DB を直接編集した場合にしか到達しない。
const INSERT_SESSION: &str = "INSERT INTO session (character_id, updated_at) \
     SELECT ?2, max( \
         strftime(?1, 'now'), \
         coalesce((SELECT strftime(?1, max(updated_at), '+0.001 seconds') \
                   FROM session WHERE character_id = ?2), ''))";

/// メッセージ追加に伴う `session.updated_at` の更新。**単独で公開しない。**
///
/// 押し上げ方は `INSERT_SESSION` と同じ。副問合せは自分自身の行も含めるので、
/// 結果は**そのキャラクターの全セッションより真に大きい**値になる。
/// これにより、追加した側が確実に「最新セッション」になる。
/// （`INSERT_SESSION` に書いた通り、格納値が日時として解釈できない場合だけは
/// 実時刻へ巻き戻る。DB を直接編集した場合にしか起きない。）
///
/// `?1` = 日時書式 / `?2` = セッション id。
const TOUCH_SESSION: &str = "UPDATE session SET updated_at = max( \
     strftime(?1, 'now'), \
     coalesce((SELECT strftime(?1, max(updated_at), '+0.001 seconds') FROM session \
               WHERE character_id = (SELECT character_id FROM session WHERE id = ?2)), '')) \
     WHERE id = ?2";

/// **素の `aoi` が続ける順**（`SELECT_LATEST_SESSION` の `ORDER BY`）。**綴りはここ1箇所。**
///
/// 会話の一覧（`SELECT_SESSION_SUMMARIES` / 作業単位 D-12c-1）も**この並びをそのまま使う。**
/// 画面は一覧の先頭を「素の `aoi` が続ける会話」として出すので、
/// **並びが食い違うと、画面が見せる会話と本文が書き込まれる会話が分かれる**
/// （ADR-0004 が最も避けたい「気づきにくい記憶の混線」の、見える側の版）。
/// 写しを2つ持たないために、文字列の定数ではなくマクロにしてある
/// （`concat!` はリテラルしか繋げない）。
///
/// 見張り番は `tests/server_admin_sessions.rs::
/// the_session_list_follows_the_order_the_bare_handle_continues`
/// （一覧の先頭を `Store::latest_session` と突き合わせる）。
macro_rules! continuation_order {
    () => {
        "ORDER BY EXISTS (SELECT 1 FROM message WHERE message.session_id = session.id) DESC, \
         updated_at DESC, id DESC"
    };
}

/// 最新セッションの取得。**`created_at` ではなく `updated_at` 基準**（ADR-0004）。
///
/// `created_at` 基準だと `aoi@42` で過去セッションに復旧した直後に `aoi` へ戻ったとき
/// 42 ではなく最新セッションへ飛び、気づきにくい記憶の混線になる。
///
/// `id DESC` は保険。書き込み経路は `updated_at` の同値を作らないが、DB を直接編集すれば
/// 同値はありうるので、その場合でも結果が実行ごとに揺れないようにしておく。
///
/// # 「最新」は**中身のあるセッション**を先に見る（作業単位 B-2b）
///
/// `server::chat` は**上流を開くより前に**セッションを解決する（＝作る）ので、
/// 上流が開けなかった回は **`session` の行だけが残る**（`message` は0件）。
/// **正常系にも同じ状態がある** —— 中継が打ち切った回・人が中止した回は
/// 1行も保存しない（ADR-0019 §4）。**空セッションは異常系だけの現象ではない。**
///
/// `INSERT` でも `updated_at` は押し上がるので、`updated_at` だけで選ぶと
/// **その空セッションが「最新」になり、素の `aoi` が会話ではなく空の続きに着地する。**
/// 履歴が空になるだけで応答は普通に返るため、**利用者は分断に気付けない。**
/// ADR-0004 が「最も避けたい」と書いた記憶の混線の隣。
///
/// ## 除外ではなく降格
///
/// `WHERE EXISTS (…)` で**弾かない**。弾くと、空セッションしか無い状態で
/// `latest_or_create_session` が**呼ばれるたびに新しい行を作り**、
/// 上流が落ちている間じゅう空セッションが増え続ける。
/// **空セッションは履歴が空なので、新規に作るのと振る舞いを区別できない。**
/// 増やさない方を採る —— 中身のあるものが1つも無ければ、
/// **いちばん新しい空セッションをそのまま使う**（`ORDER BY` の第1キーにするだけ）。
///
/// **`aoi@<id>` は影響を受けない**（`SELECT_SESSION_BY_ID` は別の文）。明示した指定は殺さない。
/// ただし `@new` が失敗して残った空セッションの id は**利用者に返っていない**
/// （502 は 200 の前に返るので `model` が無い）ので、**読み出し経路からは完全に外れる。**
///
/// ## 費用（**`EXPLAIN QUERY PLAN` で確認 / SQLite 3.53.4。時間は測っていない**）
///
/// 行の絞り込みは変更前と同じく `session_character_recent_idx (character_id=?)` が効く。
/// 変わったのは整列で、**`USE TEMP B-TREE FOR LAST TERM OF ORDER BY` が
/// `USE TEMP B-TREE FOR ORDER BY` になった**（第1キーが式なので索引の順序を使えない）。
/// 相関副問合せは `message_session_idx` の被覆索引で引かれる。
///
/// **⚠ そのキャラクターのセッション件数に上限は無い。**
/// `session` の行は `@new` のたび・失敗したターンのたびに増え、**掃除の経路が無い。**
/// 「実際には少ないから問題ない」とは書けない —— **測っていない。**
/// この文は `aoi` の経路でトランザクション内から呼ばれるので、
/// **重くしたらここが最初に効く。**
const SELECT_LATEST_SESSION: &str = concat!(
    "SELECT id, character_id, label, created_at, updated_at \
     FROM session WHERE character_id = ? ",
    continuation_order!(),
    " LIMIT 1"
);

/// id によるセッションの取得（`aoi@<id>` の意味論）。
///
/// **`character_id` も必ず条件に入れる。** id だけで引くと、他キャラのセッション id を
/// `model` に書くだけで別キャラの会話に書き込めてしまう（ADR-0004 の「記憶の混線」）。
///
/// **メッセージの有無は見ない。** `SELECT_LATEST_SESSION` が空セッションの順位を下げても、
/// 明示した指定はここを通るので届く（B-2b で殺してはいけないと決めた経路）。
const SELECT_SESSION_BY_ID: &str = "SELECT id, character_id, label, created_at, updated_at \
     FROM session WHERE id = ? AND character_id = ?";

/// メッセージの追加。`raw_content` は NOT NULL なので省略できない。
const INSERT_MESSAGE: &str = "INSERT INTO message \
     (session_id, role, dialogue, narration, raw_content) VALUES (?, ?, ?, ?, ?)";

/// 直近 N 件のメッセージ。**新しい順に取り出して呼び出し側で反転する。**
///
/// 並び替えに `created_at` を使わないのは、ミリ秒精度では同値になりうるため。
/// `id`（rowid）は挿入順に単調増加するので、時系列の権威はこちら。
const SELECT_RECENT_MESSAGES: &str = "SELECT id, role, dialogue, narration, raw_content, created_at \
     FROM message WHERE session_id = ? ORDER BY id DESC LIMIT ?";

/// **会話の一覧**（作業単位 D-12c-1）。**並びは素の `aoi` が続ける順そのもの**
/// （`continuation_order` を共有する。写しを持たない）。
///
/// `?1` = キャラクター id / `?2` = 最初の発言を何文字で切るか /
/// `?3` = 利用者の役割の綴り（`role_to_db(Role::User)`。**綴りを SQL に写さない**）/
/// `?4` = 何件返すか。
///
/// # 最初の利用者の発言
///
/// **`substr` / `length` は TEXT に対して文字（符号位置）で数える**ので、
/// 日本語の途中でバイトを割らない。切ったかどうかは `length` で同じ行から決める
/// （切った後の文字列の長さからは「ちょうどその長さだった」と区別できない）。
/// **役割で選ぶ**——応答が先に入っている会話でも、選ぶ手がかりは利用者の言葉である。
///
/// **時間は測っていない。** 上限 `?4` が抑えるのは**返す行数だけ**で、整列は
/// そのキャラクターの全セッションを見る（`SELECT_LATEST_SESSION` の「費用」と同じ形。
/// あちらと違い、トランザクションの中からは呼ばれない）。
const SELECT_SESSION_SUMMARIES: &str = concat!(
    "SELECT id, character_id, label, created_at, updated_at, \
         (SELECT count(*) FROM message WHERE message.session_id = session.id) AS message_count, \
         (SELECT substr(raw_content, 1, ?2) FROM message \
          WHERE message.session_id = session.id AND role = ?3 \
          ORDER BY message.id LIMIT 1) AS first_user_message, \
         (SELECT length(raw_content) > ?2 FROM message \
          WHERE message.session_id = session.id AND role = ?3 \
          ORDER BY message.id LIMIT 1) AS first_user_message_truncated \
     FROM session WHERE character_id = ?1 ",
    continuation_order!(),
    " LIMIT ?4"
);

/// そのキャラクターのセッションの総数（**上限で切らない** / 空のセッションも数える）。
const COUNT_SESSIONS: &str = "SELECT count(*) FROM session WHERE character_id = ?";

/// **1ページぶんのメッセージ**（作業単位 D-12c-1）。**新しい順に取り出して呼び出し側で反転する。**
///
/// `?1` = セッション id / `?2` = カーソル（**この id より古いものだけ**。無ければ `i64::MAX`）/
/// `?3` = 取り出す件数（**呼び出し側は 1 件多く頼む**。余った1件は「もっと古いものがある」印）。
///
/// **`<` であって `<=` ではない。** カーソルはいま画面の先頭にある行なので、
/// それを含めると境目の1件が2回出る。並びの権威は `id`（`SELECT_RECENT_MESSAGES` と同じ理由）。
const SELECT_MESSAGE_PAGE: &str = "SELECT id, role, dialogue, narration, raw_content, created_at \
     FROM message WHERE session_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3";

/// そのセッションのメッセージの総数（上限で切らない）。
const COUNT_MESSAGES: &str = "SELECT count(*) FROM message WHERE session_id = ?";

/// **その応答までの、利用者の発言の数**（＝ その往復の通し番号 / 作業単位 A-12c-1）。
///
/// `?1` = セッション id / `?2` = 利用者の役割の綴り / `?3` = 応答の id（**これ以下**）。
/// 並びの権威は `id`（`SELECT_RECENT_MESSAGES` と同じ理由）。
const COUNT_USER_MESSAGES_THROUGH: &str =
    "SELECT count(*) FROM message WHERE session_id = ?1 AND role = ?2 AND id <= ?3";

impl Store {
    /// セッションを作成し、作成した行の id を返す（`aoi@new` の意味論）。
    ///
    /// ラベルは付けない（フェーズ1では自動生成しないため NULL）。
    pub async fn create_session(&self, character_id: CharacterId) -> Result<SessionId, StoreError> {
        let done = sqlx::query(INSERT_SESSION)
            .bind(TIMESTAMP_FORMAT)
            .bind(character_id.get())
            .execute(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let id = done.last_insert_rowid();
        tracing::debug!(
            session_id = id,
            character_id = character_id.get(),
            "セッションを作成しました"
        );
        Ok(SessionId::from_row(id))
    }

    /// そのキャラクターの最新セッションを返す。1つも無ければ `None`。
    ///
    /// 「最新」は **メッセージを持つセッションのうち `updated_at` が最も新しいもの**
    /// （ADR-0004 ＋ 作業単位 B-2b）。中身のあるものが1つも無ければ、
    /// **いちばん新しい空セッション**を返す（`None` になるのは1行も無いときだけ）。
    /// 理由と代償は `SELECT_LATEST_SESSION` の doc。
    pub async fn latest_session(
        &self,
        character_id: CharacterId,
    ) -> Result<Option<Session>, StoreError> {
        fetch_latest_session(&self.pool, character_id).await
    }

    /// そのキャラクターの最新セッションを返し、1つも無ければ作る（`aoi` の意味論）。
    ///
    /// 取得と作成は**同一トランザクション**。分けると、同時に2本のリクエストが来たときに
    /// 空のセッションが2つできて会話が分断される。
    ///
    /// **「1つも無ければ」は文字どおり1行も無いとき**（作業単位 B-2b）。
    /// 空セッションしか無い状態では**作らずにそれを使う** ——
    /// 空セッションと新規セッションは履歴が空で区別できないので、増やす理由が無い。
    /// 作るのを空セッションの有無で決める実装にすると、上流が落ちている間じゅう
    /// 中身の無いセッションが増え続ける（`SELECT_LATEST_SESSION` の doc）。
    ///
    /// **トランザクションは `BEGIN IMMEDIATE` で開始する**（ADR-0004）。
    /// この関数は「読んでから書く」ため、素の `BEGIN`（deferred）だと書きへの昇格が
    /// `SQLITE_BUSY_SNAPSHOT`(517) になり、**`busy_timeout` が一切効かず即座に失敗する。**
    /// 最初から書きロックを取れば書き↔書きの待ちになり、`busy_timeout` の範囲で待てる。
    /// `aoi` は最も多く通る経路なので、これを怠ると FE が2本同時に投げただけで片方が落ちる。
    pub async fn latest_or_create_session(
        &self,
        character_id: CharacterId,
    ) -> Result<Session, StoreError> {
        let mut tx = self
            .pool
            .begin_with(BEGIN_IMMEDIATE)
            .await
            .map_err(StoreError::Query)?;

        if let Some(session) = fetch_latest_session(&mut *tx, character_id).await? {
            tx.commit().await.map_err(StoreError::Query)?;
            return Ok(session);
        }

        let done = sqlx::query(INSERT_SESSION)
            .bind(TIMESTAMP_FORMAT)
            .bind(character_id.get())
            .execute(&mut *tx)
            .await
            .map_err(StoreError::Query)?;
        let id = done.last_insert_rowid();

        // 直前に挿入した行を読み直す（`created_at` / `updated_at` は DB 側で決まるため）。
        // 万一見つからなければ `RowNotFound` になり、捏造した値を返すことはない。
        let row = sqlx::query(SELECT_SESSION_BY_ID)
            .bind(id)
            .bind(character_id.get())
            .fetch_one(&mut *tx)
            .await
            .map_err(StoreError::Query)?;
        let session = session_from_row(&row)?;

        tx.commit().await.map_err(StoreError::Query)?;
        tracing::debug!(
            session_id = id,
            character_id = character_id.get(),
            "最新セッションが無いため作成しました"
        );
        Ok(session)
    }

    /// id でセッションを取得する（`aoi@<id>` の意味論）。
    ///
    /// **そのキャラクターのセッションでなければ `None`。** 他キャラのセッション id を
    /// 渡されても引けないことが、ADR-0004 の「別キャラの記憶が混ざる」を塞ぐ要。
    ///
    /// # 未検証の値の入口
    ///
    /// **`raw_session_id` は `aoi@42` の `42`、つまりクライアントから来る信頼できない入力。**
    /// `store` の公開 API で生の `i64` を受けるのはここだけで、それは意図的である。
    ///
    /// この関数が所有確認を行うため、**`SessionId` が存在する = そのキャラクターの
    /// セッションだと確認済み**、という不変条件が成立する（`Handle` と同じ論法）。
    /// ここが `SessionId` を受ける形だと、呼び出し側は未検証の `i64` から `SessionId` を
    /// 組み立てるほかなく、その一行は「DB から復元した値」と見分けが付かない。
    ///
    /// 値の範囲は検証しない。負値や存在しない id は WHERE に一致せず `None` になるだけで、
    /// SQL へはプレースホルダで渡すので解釈もされない。
    pub async fn find_session(
        &self,
        character_id: CharacterId,
        raw_session_id: i64,
    ) -> Result<Option<Session>, StoreError> {
        let row = sqlx::query(SELECT_SESSION_BY_ID)
            .bind(raw_session_id)
            .bind(character_id.get())
            .fetch_optional(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        row.as_ref().map(session_from_row).transpose()
    }

    /// **その会話がまだ在るか**（作業単位 A-10 のレビュー / `server::turn` が使う）。
    ///
    /// 往復の保存が `UnknownSession` で失敗した後に、**理由を確かめるためだけに**引く
    /// （生成している間にキャラクターが完全に消されると、会話もカスケードで消える）。
    /// 書くかどうかを決める読みではない（保存はもう失敗している）。
    pub async fn session_exists(&self, session_id: SessionId) -> Result<bool, StoreError> {
        let owner: Option<i64> = sqlx::query_scalar(SELECT_SESSION_OWNER)
            .bind(session_id.get())
            .fetch_optional(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        Ok(owner.is_some())
    }

    /// メッセージを追加し、**同時に `session.updated_at` を進める**。
    ///
    /// 2つの書き込みは**同一トランザクションの1操作**（ADR-0004）。片方だけを行う公開 API は
    /// 存在しない。更新漏れは「セッションの取り違え」として、しかも**気づきにくい形で**
    /// 表面化するため、呼び出し側の規律に委ねない。
    ///
    /// `updated_at` はそのキャラクターの全セッションより真に大きい値になるので、
    /// **同一ミリ秒に複数回追加しても順序が壊れない。**
    ///
    /// セッションが存在しなければ `StoreError::UnknownSession`。
    /// このとき**メッセージは書き込まれない**（更新を先に行い、0件ならロールバックする）。
    pub async fn append_message(
        &self,
        session_id: SessionId,
        message: &NewMessage,
    ) -> Result<MessageId, StoreError> {
        let mut tx = self.pool.begin().await.map_err(StoreError::Query)?;

        // **先に `updated_at` を進める。** 対象が無ければここで打ち切るので、
        // 「メッセージだけが入って `updated_at` が付いてこない」状態は作れない
        // （外部キーが無効化されていても成立する）。
        touch_session(&mut *tx, session_id).await?;

        let id = insert_message(&mut *tx, session_id, message).await?;

        tx.commit().await.map_err(StoreError::Query)?;

        tracing::debug!(
            message_id = id.get(),
            session_id = session_id.get(),
            "メッセージを追加しました"
        );
        Ok(id)
    }

    /// 会話の往復（利用者の発言と応答）を**1トランザクションで**追加する。
    ///
    /// `append_message` を2回呼ぶのとの違いは**不可分であること**。分けて書くと、
    /// 応答側の書き込みだけが失敗したときに**「返答の無い利用者ターン」が履歴に残る。**
    /// 次のターンのプロンプトには利用者の発言が2つ並ぶが、`prompt` は `raw_content` を
    /// 読むだけなのでそこでは何も検知されず、**モデルの応答が静かに崩れるだけ**になる。
    /// 呼び出し側の規律に委ねず、片方だけ入る状態を作れない口をここに置く。
    ///
    /// 書き込み順は**利用者 → 応答**。`role` は引数の `NewMessage` が持つ値をそのまま使う
    /// （位置から決め打つと、呼び出し側の取り違えをこの層が握り潰す）。
    ///
    /// # 往復の形の検証
    ///
    /// **`user.role` が `User`、`reply.role` が `Assistant` でなければ
    /// `StoreError::InvalidTurn` で弾く。** `role` を見ないと
    /// 「利用者の発言が2つ並ぶ履歴」や「利用者と応答が入れ替わった履歴」を
    /// **不可分に**書き込めてしまい、この関数が防ぐと宣言している状態そのものになる。
    /// `Role` は2値なので往復の形は一意に定まり、引数順の取り違えもここで止まる。
    ///
    /// **検証は DB に触る前に行う。** 弾いたときは1行も書かれず、`updated_at` も動かない。
    ///
    /// ただし**行数と `updated_at` でこの位置を固定できるのは、検証より前の書き込みが
    /// トランザクションの外へ出た場合だけ**である。検証を `touch_session` の後
    /// （トランザクションの中）へ下げる変異は早期 return でロールバックされるので、
    /// **行数も `updated_at` も外からは同一に見え、それでは観測できない**（変異検査で実測）。
    /// 位置を観測するのは**宛先も形も不正なときの返り値**で、検証が先なら `InvalidTurn`、
    /// 後なら `touch_session` が先に 0 件を返して `UnknownSession` になる
    /// （`append_turn_checks_the_shape_before_touching_the_database`）。
    ///
    /// なお、この検証を入れた結果 **`append_turn` の経路だけを見ると
    /// 「`role` を位置から決め打つ」実装と区別が付かなくなっている**（変異検査で実測。
    /// 正しい向きに決め打つ変異は全テストが緑のまま通る）。
    /// **これはテストの穴ではなく等価変異である。** `Role` は2値なので、ガードを通過した
    /// 時点で入力域は `(User, Assistant)` の**単集合に潰れており**、決め打ちと
    /// `NewMessage::role` からの bind は**到達可能な全入力で同一のバイト列**を書く。
    /// 受理集合が同一である以上、どんなテストを足してもこの変異は赤にできない。
    /// **逆向きに決め打つ変異は赤**（形の検証を通った組と食い違うため）。
    ///
    /// **行が死んでいるわけでもない。** bind の実体は `insert_message` にあり、これを共有する
    /// `append_message` は片方向のメッセージを書く口なので、綴りを固定する変異は
    /// あちら側で赤になる（`role_is_stored_with_the_spelling_the_schema_expects` 他）。
    /// それが `NewMessage::role` から bind し続ける理由。
    ///
    /// `session.updated_at` の押し上げは**1回**で足りる。同一トランザクション内なので、
    /// そのキャラクターの全セッションより真に大きい値へ1回進めば、
    /// 往復を書いた側が確実に最新セッションになる。
    ///
    /// セッションが存在しなければ `StoreError::UnknownSession`。
    /// このとき**どちらのメッセージも書き込まれない**。
    pub async fn append_turn(
        &self,
        session_id: SessionId,
        user: &NewMessage,
        reply: &NewMessage,
    ) -> Result<AppendedTurn, StoreError> {
        // **DB に触る前に検証する。** ここを `touch_session` より後へ下げると、形が不正な呼び出しが
        // 毎回 SQLite の書きロックを取ってから弾かれ、裏方の抽出書き込みと重なる設計
        // （CLAUDE.md）で `busy_timeout` を無駄に食う。加えて宛先も不正なとき、
        // 返る種別が `InvalidTurn`（呼び出し側のバグ）から `UnknownSession`（404 相当）へ
        // 静かに化ける。**「1行も書かれない」だけではこの位置は固定できない**
        // （早期 return はロールバックされるため。doc コメントを参照）。
        if user.role != Role::User || reply.role != Role::Assistant {
            // 何を渡されたかは調査に要るのでログにだけ出す（`Display` には載せない）。
            tracing::warn!(
                user_role = ?user.role,
                reply_role = ?reply.role,
                session_id = session_id.get(),
                "往復の形になっていない組の追加を拒否しました"
            );
            return Err(StoreError::InvalidTurn);
        }

        let mut tx = self.pool.begin().await.map_err(StoreError::Query)?;

        // `append_message` と同じく、対象が無ければ書き込む前に打ち切る。
        touch_session(&mut *tx, session_id).await?;

        let user_id = insert_message(&mut *tx, session_id, user).await?;
        let reply_id = insert_message(&mut *tx, session_id, reply).await?;

        tx.commit().await.map_err(StoreError::Query)?;

        tracing::debug!(
            user_message_id = user_id.get(),
            reply_message_id = reply_id.get(),
            session_id = session_id.get(),
            "会話の往復を追加しました"
        );
        Ok(AppendedTurn {
            user: user_id,
            reply: reply_id,
        })
    }

    /// そのセッションの直近 `limit` 件のメッセージを**時系列順**（古い順）で返す。
    ///
    /// プロンプト組立が使う並びなので、返す順序を逆にしないこと。
    /// `limit` が 0 なら空。件数より大きければ全件。
    pub async fn recent_messages(
        &self,
        session_id: SessionId,
        limit: u32,
    ) -> Result<Vec<Message>, StoreError> {
        let rows = sqlx::query(SELECT_RECENT_MESSAGES)
            .bind(session_id.get())
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let mut messages = Vec::with_capacity(rows.len());
        // SQL は新しい順に取っているので、時系列順に戻す。
        for row in rows.iter().rev() {
            messages.push(message_from_row(row, session_id)?);
        }
        Ok(messages)
    }

    /// そのキャラクターの会話を**素の `aoi` が続ける順**に `limit` 件まで返す（作業単位 D-12c-1）。
    ///
    /// **先頭は [`Store::latest_session`] が返すものと同じ**（並びの綴りを共有している /
    /// `continuation_order`）。**空のセッションは後ろへ回る**（除外はしない。数にも入る）。
    ///
    /// `opening_chars` は最初の利用者の発言を何文字で切るか。
    ///
    /// **読むだけ。トランザクションを取らない**——一覧と総数（[`Store::count_sessions`]）が
    /// 1件ずれうるのは承知の上（`get_drift` と同じ判断。囲うと本文処理の書き込みと取り合う）。
    pub async fn list_sessions(
        &self,
        character_id: CharacterId,
        limit: u32,
        opening_chars: u32,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        let rows = sqlx::query(SELECT_SESSION_SUMMARIES)
            .bind(character_id.get())
            .bind(i64::from(opening_chars))
            .bind(role_to_db(Role::User))
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        rows.iter().map(session_summary_from_row).collect()
    }

    /// そのキャラクターのセッションの総数（**上限で切らない** / 空のセッションも数える）。
    pub async fn count_sessions(&self, character_id: CharacterId) -> Result<u32, StoreError> {
        let counted: i64 = sqlx::query_scalar(COUNT_SESSIONS)
            .bind(character_id.get())
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        // **`try_get::<u32>` にしない**（`count_attribute_revisions` と同じ理由）。
        u32::try_from(counted).map_err(|_| StoreError::InvalidStoredValue)
    }

    /// そのセッションのメッセージを**1ページぶん、古い順で**返す（作業単位 D-12c-1）。
    ///
    /// `before` は**カーソル**で、その id より古いものだけを返す（無ければ最新から）。
    /// **カーソルの行がこのセッションのものかは確かめない**——確かめなくても、
    /// 返るのは `session_id` のメッセージだけである（`SessionId` は所有確認を通ってしか作れない）。
    /// **型の名前を「存在する」と読まないこと**（`MemoryId` と同じ注意 / `server::admin`）。
    ///
    /// `limit` が 0 なら空（`has_older` は古いものが1件でもあれば `true`）。
    pub async fn message_page(
        &self,
        session_id: SessionId,
        before: Option<MessageId>,
        limit: u32,
    ) -> Result<MessagePage, StoreError> {
        let rows = sqlx::query(SELECT_MESSAGE_PAGE)
            .bind(session_id.get())
            .bind(before.map_or(i64::MAX, MessageId::get))
            // **1件多く頼む。** 余った1件が「もっと古いものがある」印になる。
            .bind(i64::from(limit) + 1)
            .fetch_all(&self.pool)
            .await
            .map_err(StoreError::Query)?;

        let wanted = usize::try_from(limit).map_err(|_| StoreError::InvalidStoredValue)?;
        let has_older = rows.len() > wanted;
        let mut messages = Vec::with_capacity(rows.len().min(wanted));
        // SQL は新しい順。**余った1件（いちばん古い）を落としてから**、時系列順に戻す。
        for row in rows.iter().take(wanted).rev() {
            messages.push(message_from_row(row, session_id)?);
        }
        Ok(MessagePage {
            messages,
            has_older,
        })
    }

    /// そのセッションのメッセージの総数（**上限で切らない**）。
    pub async fn count_messages(&self, session_id: SessionId) -> Result<u32, StoreError> {
        let counted: i64 = sqlx::query_scalar(COUNT_MESSAGES)
            .bind(session_id.get())
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        u32::try_from(counted).map_err(|_| StoreError::InvalidStoredValue)
    }

    /// **その応答の往復が、会話の何往復目か**（最初の往復が 1 / 作業単位 A-12c-1）。
    ///
    /// 数えるのは**その応答までの利用者の発言**。約束・決まりの提案の依頼の「往復 n」に使う
    /// （`server::agreement_proposal`）。番号は依頼の中で往復を指し合うためのもので、
    /// 1つずれても依頼が壊れるだけで保存は壊れない。**読むだけ**（トランザクションを取らない）。
    pub async fn turn_number(
        &self,
        session_id: SessionId,
        reply: MessageId,
    ) -> Result<u32, StoreError> {
        let counted: i64 = sqlx::query_scalar(COUNT_USER_MESSAGES_THROUGH)
            .bind(session_id.get())
            .bind(role_to_db(Role::User))
            .bind(reply.get())
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::Query)?;
        u32::try_from(counted).map_err(|_| StoreError::InvalidStoredValue)
    }
}

/// 一覧の1行を組み立てる。
fn session_summary_from_row(row: &SqliteRow) -> Result<SessionSummary, StoreError> {
    let session = session_from_row(row)?;
    let message_count: i64 = row.try_get("message_count").map_err(StoreError::Query)?;
    let message_count = u32::try_from(message_count).map_err(|_| StoreError::InvalidStoredValue)?;
    let text: Option<String> = row
        .try_get("first_user_message")
        .map_err(StoreError::Query)?;
    // `length(...) > ?` は 0 / 1。利用者の発言が無ければ NULL。
    let truncated: Option<i64> = row
        .try_get("first_user_message_truncated")
        .map_err(StoreError::Query)?;

    Ok(SessionSummary {
        session,
        message_count,
        first_user_message: text.map(|text| OpeningText {
            text,
            truncated: truncated.is_some_and(|flag| flag != 0),
        }),
    })
}

/// `session.updated_at` を押し上げる。**トランザクションの中からしか呼ばない。**
///
/// 対象のセッションが無ければ `StoreError::UnknownSession` を返す。呼び出し側は
/// commit せずに抜けるので、そこまでの書き込みはロールバックされる。
/// **この 0 件検査を外すと、存在しないセッションにメッセージだけが入る。**
async fn touch_session<'e, E>(executor: E, session_id: SessionId) -> Result<(), StoreError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let touched = sqlx::query(TOUCH_SESSION)
        .bind(TIMESTAMP_FORMAT)
        .bind(session_id.get())
        .execute(executor)
        .await
        .map_err(StoreError::Query)?;

    if touched.rows_affected() != 1 {
        tracing::warn!(
            session_id = session_id.get(),
            "存在しないセッションへのメッセージ追加を拒否しました"
        );
        return Err(StoreError::UnknownSession);
    }
    Ok(())
}

/// `message` を1行書く。**トランザクションの中からしか呼ばない**（`updated_at` の
/// 押し上げと対で使う。単独で公開すると `append_message` の doc の宣言が崩れる）。
async fn insert_message<'e, E>(
    executor: E,
    session_id: SessionId,
    message: &NewMessage,
) -> Result<MessageId, StoreError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let done = sqlx::query(INSERT_MESSAGE)
        .bind(session_id.get())
        .bind(role_to_db(message.role))
        .bind(message.dialogue.as_deref())
        .bind(message.narration.as_deref())
        // **`raw_content` は常に書き込む。** 分解できたかどうかに関わらず保持する。
        .bind(message.raw_content.as_str())
        .execute(executor)
        .await
        .map_err(StoreError::Query)?;

    Ok(MessageId::from_row(done.last_insert_rowid()))
}

/// 最新セッションを1件引く。プールでもトランザクションでも使えるようにしてある。
async fn fetch_latest_session<'e, E>(
    executor: E,
    character_id: CharacterId,
) -> Result<Option<Session>, StoreError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let row = sqlx::query(SELECT_LATEST_SESSION)
        .bind(character_id.get())
        .fetch_optional(executor)
        .await
        .map_err(StoreError::Query)?;

    row.as_ref().map(session_from_row).transpose()
}

/// 1行を `Session` に組み立てる。
fn session_from_row(row: &SqliteRow) -> Result<Session, StoreError> {
    Ok(Session {
        id: SessionId::from_row(row.try_get::<i64, _>("id").map_err(StoreError::Query)?),
        character_id: CharacterId::from_row(
            row.try_get::<i64, _>("character_id")
                .map_err(StoreError::Query)?,
        ),
        label: row.try_get("label").map_err(StoreError::Query)?,
        created_at: row.try_get("created_at").map_err(StoreError::Query)?,
        updated_at: row.try_get("updated_at").map_err(StoreError::Query)?,
    })
}

/// 1行を `Message` に組み立てる。`session_id` は WHERE で固定済みなので引数を使う。
fn message_from_row(row: &SqliteRow, session_id: SessionId) -> Result<Message, StoreError> {
    let stored: String = row.try_get("role").map_err(StoreError::Query)?;
    // スキーマの CHECK があるため通常は起こらないが、**panic せずエラーに落とす**。
    let Some(role) = role_from_db(&stored) else {
        // 格納値は Display に載せられないので、調査の手掛かりはここにしかない。
        tracing::warn!(stored_role = ?stored, "想定外の role が格納されています");
        return Err(StoreError::InvalidStoredValue);
    };

    Ok(Message {
        id: MessageId::from_row(row.try_get::<i64, _>("id").map_err(StoreError::Query)?),
        session_id,
        role,
        dialogue: row.try_get("dialogue").map_err(StoreError::Query)?,
        narration: row.try_get("narration").map_err(StoreError::Query)?,
        raw_content: row.try_get("raw_content").map_err(StoreError::Query)?,
        created_at: row.try_get("created_at").map_err(StoreError::Query)?,
    })
}

/// `Role` の DB 表現。**`migrations/0001_initial.sql` の
/// `CHECK (role IN ('user', 'assistant'))` と同じ綴りにすること。**
///
/// 変換を `domain` ではなくここに置いているのは、綴りの決定権を DB スキーマの隣に
/// 集めるため。`domain::Role` は DB の都合を知らない。
const fn role_to_db(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

/// DB 表現から `Role` へ。想定外の綴りは `None`（呼び出し側でエラーに落とす）。
///
/// **trim や小文字化はしない。** 受理を広げると、CHECK を通らない値を
/// アプリ層だけが受け入れる非対称ができる。
fn role_from_db(stored: &str) -> Option<Role> {
    match stored {
        "user" => Some(Role::User),
        "assistant" => Some(Role::Assistant),
        _ => None,
    }
}

/// その会話の持ち主（作業単位 A-10 / [`turn_session_refusal`]）。
const SELECT_SESSION_OWNER: &str = "SELECT character_id FROM session WHERE id = ?";

/// **裏方の書き込みが会話の縛りで書けなかった理由**を分ける（作業単位 A-10 のレビュー）。
///
/// | 会話 | 返すもの |
/// |---|---|
/// | 無い | `Some(StoreError::TurnSessionGone)` —— キャラクターが完全に消された（会話もカスケードで消えた） |
/// | 在るが別のキャラクターのもの | `Some(StoreError::SessionOfAnotherCharacter)` —— **呼び出し側の取り違え（本物の故障）** |
/// | そのキャラクターのもの | `None` —— 縛りでは止まっていない |
///
/// **2つを分けるのは、後者を「消えた」として `info` に化かさないため**
/// （`server::extract::character_vanished` は `TurnSessionGone` だけを読む）。
///
/// **書くかどうかを決める読みではない。** 縛りは書き込みの文（または書きロックの中）で
/// 済んでいて、ここは**書けなかった後に理由を引くだけ**なので、間に窓があっても害にならない。
/// 書きロックの中から呼ぶときは、そのトランザクションを渡す。
pub(super) async fn turn_session_refusal<'e, E>(
    executor: E,
    character_id: CharacterId,
    session_id: SessionId,
) -> Result<Option<StoreError>, StoreError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let owner: Option<i64> = sqlx::query_scalar(SELECT_SESSION_OWNER)
        .bind(session_id.get())
        .fetch_optional(executor)
        .await
        .map_err(StoreError::Query)?;
    Ok(match owner {
        None => Some(StoreError::TurnSessionGone),
        Some(owner) if owner != character_id.get() => {
            tracing::warn!(
                character_id = character_id.get(),
                session_id = session_id.get(),
                "別のキャラクターの会話を名乗った裏方の書き込みを拒みました"
            );
            Some(StoreError::SessionOfAnotherCharacter)
        }
        Some(_) => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 綴りは DB スキーマの CHECK と一致する。
    ///
    /// **往復（`role_from_db(role_to_db(x)) == x`）だけでは足りない。**
    /// 2値を取り違えて対応させても往復は成立してしまうため、綴りそのものを固定する。
    #[test]
    fn role_uses_the_spelling_the_schema_expects() {
        assert_eq!(role_to_db(Role::User), "user");
        assert_eq!(role_to_db(Role::Assistant), "assistant");
    }

    /// 往復する。
    #[test]
    fn role_round_trips() {
        for role in [Role::User, Role::Assistant] {
            assert_eq!(role_from_db(role_to_db(role)), Some(role));
        }
    }

    /// 想定外の綴りは受け付けない（panic もしない）。
    #[test]
    fn unknown_role_is_rejected() {
        for stored in ["", "system", "User", "assistant ", " user", "tool"] {
            assert_eq!(role_from_db(stored), None, "stored = {stored:?}");
        }
    }
}
