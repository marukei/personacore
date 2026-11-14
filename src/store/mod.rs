//! SQLite 永続化層。
//!
//! 責務は **接続の確立・マイグレーションの適用と、テーブルごとの読み書き**。
//! ロジックはここに持たせず、`domain` / `prompt` / `parse` 側の純粋関数に寄せる。
//!
//! 公開 API は検証済みのドメイン型（`Handle` / `StatAxis` / `CharacterId` 等）を
//! 受け取る。`&str` や裸の `i64` を受ける入口を作らないことで、未検証の値が DB に届く
//! 経路を無くす。
//!
//! ただし **`Store::pool` というテスト専用の抜け穴が存在する。** ここを通る値は
//! `domain` の検証を一切受けず、スキーマの CHECK / UNIQUE だけが最後の砦になる。
//! `testing` フィーチャ（と crate 内テスト）でしかコンパイルされないため
//! 本番ビルドからは呼べないが、**テストで作った行はアプリ層の不変条件を満たさない**
//! 前提で読むこと（`tests/` はまさにその状態を再現するために使っている）。
//!
//! `store` にまだ無いクエリが必要になったら、`pool` を掴んで直接書くのではなく
//! **このモジュールに関数を足す**。前者は `testing` フィーチャ無しではコンパイルが通らない。

mod agreement;
mod attribute;
mod axis_candidate;
mod character;
mod drift;
mod memory;
mod memory_repair;
mod migration_ledger;
mod session;

use std::path::Path;

use sqlx::SqlitePool;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

pub use agreement::{AgreementProposalsRecorded, AgreementTransition};
pub use attribute::AttributeRevision;
pub use axis_candidate::{AxisCandidate, AxisCandidateTransition};
pub use drift::{DriftVoteRecord, DriftVoteTally, DriftWrite, StoredDriftCharge, StoredDriftVote};
pub use session::{AppendedTurn, MessagePage, OpeningText, RecentHistory, Session, SessionSummary};

// 行の型は `domain` に置いてある（純粋モジュールが sqlx を引き込まないため。
// 理由は `crate::domain` の doc）。**呼び出し側の名前を変えないよう再公開する。**
pub use crate::domain::{
    AttributeKey, AttributeSet, AttributeValue, Character, CharacterAttribute, CharacterColumn,
    CharacterEntry, CharacterUpdate, FieldUpdate, Memory, MemoryColumn, MemoryEntry, Message,
    UnreadableCharacter, UnreadableMemory,
};

/// `migrations/` 配下の SQL をコンパイル時に埋め込む。
static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// 永続化層のエラー。
///
/// `Display` には**内部パスや接続文字列を含めない**（エラーレスポンスに載りうるため）。
/// 詳細は `source` 側に残るので、ログには出せる。
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// データベースファイルを開けなかった。
    #[error("データベースへの接続に失敗しました")]
    Connect(#[source] sqlx::Error),

    /// マイグレーションの適用に失敗した。
    #[error("マイグレーションの適用に失敗しました")]
    Migrate(#[source] sqlx::migrate::MigrateError),

    /// **新しい版の personacore で開かれた保存先を、古い exe で開こうとした**（B-55）。
    ///
    /// 適用済みなのに、この exe が知らない版が帳簿にある（sqlx の `VersionMissing`）。
    /// **止まること自体が正しい**——知らない列を持つ DB を古い exe が書き換えない。
    /// DB には何も書かずに止まる（版の検証は適用の前）。先に走る付け替えの段
    /// （`migration_ledger.rs`）は既知の版の CRLF 行しか書かず、新しい版の exe が
    /// 開いた帳簿にはその行が残らない。
    /// 版の番号は `source` 側（ログ・`Caused by`）にだけ出る。
    ///
    /// **⚠ この文言が出るのは、この版より後の exe だけ。** 配布済みの v0.1.4 以前の
    /// exe は、英語の sqlx の文のまま止まる。
    #[error(
        "この保存先は新しい版の personacore で開かれています。新しい版の exe を使うか、写しておいた保存先に戻してください"
    )]
    DatabaseFromNewerVersion(#[source] sqlx::migrate::MigrateError),

    /// クエリの実行・行の取り出しに失敗した。
    ///
    /// 制約違反もここに入る。呼び出し側で区別したい場合は
    /// `sqlx::Error::as_database_error()` の `kind()` を見る。
    #[error("データベース操作に失敗しました")]
    Query(#[source] sqlx::Error),

    // **`ConflictingUnnormalizedName` はここにあった**（ADR-0004 / 作業単位 0004b-2 で削除）。
    //
    // 「ゴースト行（`" aoi "`）が正規化キーの UNIQUE 索引を塞いでいる」を
    // 正当な重複と区別するためのバリアントだったが、**その索引を落とした**
    // （`migrations/0006_drop_character_name_key_idx.sql`）ので、
    // `character.handle` に残る一意制約は**列の `UNIQUE`（完全一致）だけ**になった。
    // UNIQUE 違反は「まったく同じ綴りの行が既にある」以外の意味を持たず、
    // **このバリアントに落ちる入力が存在しない**（＝死んだ検査）。
    //
    // **ゴースト行があっても正当な識別子は作れる**ようになったので、
    // 区別すべき袋小路そのものが無い。
    /// 指定された名前のキャラクターが存在しない（更新・削除の対象が無い）。
    ///
    /// **`UnknownSession` に相乗りさせない。** 呼び出し側（`server`）は
    /// どちらも 404 に落とすが、原因も調査先も別である。
    ///
    /// 引き当ては `find_character_by_handle` と**同じ完全一致**（0004b-2）なので、
    /// このエラーは「その綴りの行が1件も無い」ことを意味する。
    /// **格納値が規則外の行（ゴースト行 / `"Aoi"`）は、そもそも指せない**
    /// ——`Handle` を作れないため、この関数まで到達しない。
    #[error("キャラクターが見つかりません")]
    UnknownCharacter,

    /// **しまっていない、会話が1件以上あるキャラクターを削除しようとした**
    /// （ADR-0016 §2 / ADR-0049 §決定1）。
    ///
    /// `session` は `ON DELETE CASCADE` で `message` / `summary` を連れて消えるため、
    /// 会話のあるキャラクターの削除は**取り返しのつかない喪失**になる。
    /// **しまったキャラクターだけは消す**（先に「しまう」を通らせることで、一覧からの
    /// 1回の誤操作で中身を失わないようにしてある / 作業単位 A-10）。
    ///
    /// **`UnknownCharacter` と混ぜない。** 「そんなキャラクターは居ない」と
    /// 「居るが消せない」は、利用者から見て別の話（前者は名前の打ち間違い、
    /// 後者は仕様上の拒否）。
    #[error("しまっていない、会話のあるキャラクターは削除できません")]
    CharacterHasSessions,

    /// **保存されている尺度と違う尺度で作られた性格ステータスの値を渡された**
    /// （ADR-0015 / 作業単位 0011a-2）。
    ///
    /// `StatValue` は「ある尺度を超えないこと」しか覚えていない。
    /// **その尺度がそのキャラクターのものかは型では守れない**ので、
    /// `replace_stat_allocs` が保存済みの尺度と突き合わせる。
    ///
    /// **`InvalidStoredValue` に相乗りさせない。** あちらは「DB に入っている値がおかしい」で、
    /// こちらは「渡された値がこのキャラクター向けではない」。原因も直し方も別。
    ///
    /// **`Display` に尺度も値も載せない**（他の変異と同じ）。
    #[error("性格ステータスの値が、保存されている尺度のものではありません")]
    StatScaleMismatch,

    /// **保存されている値より小さい尺度へ変更しようとした**（ADR-0015 / 作業単位 0011a-2）。
    ///
    /// 許すと、その `PATCH` 一回で `stat_allocs` が丸ごと失敗するようになり、
    /// **以後のチャットが必ず失敗する。** 復旧は「先に `PUT .../stats` で値を下げ、
    /// 次に尺度を下げる」の順で足りる（ADR-0015）。
    ///
    /// **`UnknownCharacter` と混ぜない。** 「そんなキャラクターは居ない」と
    /// 「居るが今は下げられない」は、`CharacterHasSessions` と同じ形の違い
    /// （前者は綴りを直せば通る / 後者は状態を変えない限り永久に通らない）。
    #[error("保存されている性格ステータスの値より小さい尺度には変更できません")]
    StatScaleBelowStoredValue,

    /// 指定されたセッションが存在しない。
    ///
    /// `append_message` は `session.updated_at` の更新が0件だった時点で打ち切る
    /// （ADR-0004: メッセージ追加と `updated_at` 更新は分離できない1操作）。
    /// **外部キー違反として返さないのは、外部キーが無効でも成立させるため。**
    /// `PRAGMA foreign_keys` は接続ごとの設定で、有効化を忘れると黙って素通りする。
    #[error("セッションが見つかりません")]
    UnknownSession,

    /// **裏方の書き込みを生んだ会話が、もうそのキャラクターのものとして存在しない**
    /// （ADR-0049 §決定3 / 作業単位 A-10 のレビュー前の差し戻し）。**何も書かれない。**
    ///
    /// 裏方（抽出・変容・軸の候補）の書き込みは、**それを生んだ会話が今も在り、
    /// 同じキャラクターのものであること**を、書き込みと同じ文（または書きロックを取った
    /// 同じトランザクション）で確かめる（`*_in_session` の関数）。
    ///
    /// # なぜ要るのか —— `character.id` は使い回される
    ///
    /// `character` は AUTOINCREMENT ではないので、**最大の id のキャラクターを完全に消して
    /// すぐ作ると、同じ id になる。** 外部キーは「その id の行が在るか」しか見ないので、
    /// 消す前から走っていた裏方の書き込みが**新しいキャラクターに届く**。
    /// `session.id` は AUTOINCREMENT（`migrations/0003`）なので使い回されず、
    /// 消したキャラクターの会話はカスケードで消えている —— **だから会話の側で縛る。**
    ///
    /// **`UnknownSession` に相乗りさせない。** あちらは本文の経路（`append_message` 等）の
    /// 「宛先の会話が無い」で、呼び出し側は 404 などに落とす。こちらは裏方だけが受け取り、
    /// 「キャラクターが消えた」として捨てる（`server::extract::character_vanished`）。
    #[error("書き込みを生んだ会話がもうありません")]
    TurnSessionGone,

    /// **往復として渡された組が「利用者 → 応答」の形になっていない。**
    ///
    /// `append_turn` は「返答の無い利用者ターン」を作らせないための口だが、
    /// `role` を見ないと**「利用者の発言が2つ並ぶ履歴」を不可分に書き込める。**
    /// これは防ぎたかった状態そのもので、`prompt` は `raw_content` を読むだけなので
    /// 後段の誰も気づかない。呼び出し側の引数順の取り違えもここで止まる。
    ///
    /// **`UnknownSession` に相乗りさせない。** 宛先が無いのと形が違うのは原因も対処も別で、
    /// 混ぜると呼び出し側が区別できなくなる。
    ///
    /// 実際に渡された `role` は `tracing::warn!` に出す。`Display` に載せない。
    #[error("会話の往復の形が不正です")]
    InvalidTurn,

    /// **訂正しようとした記憶が存在しない**（ADR-0022 / 作業単位 14a）。
    ///
    /// 呼び出し側が id を間違えている。**`UnknownCharacter` にも
    /// `MemoryBelongsToAnotherCharacter` にも相乗りさせない**——
    /// 「その id の行がどこにも無い」と「行はあるが別のキャラクターのもの」は、
    /// 打ち間違いなのか宛先違いなのかという**調査先の違い**そのもの。
    /// 混ぜると、一覧で見た id を貼り間違えた運用者が原因に辿り着けない。
    ///
    /// **`Display` に id を載せない**（他の拒否と同じ。手掛かりはログ）。
    #[error("記憶が見つかりません")]
    UnknownMemory,

    /// **別のキャラクターの記憶を訂正しようとした**（ADR-0022 / 作業単位 14a）。
    ///
    /// # スキーマが既に防いでいるのに、なぜ `store` の入口でも弾くのか
    ///
    /// `memory` の複合外部キー
    /// （`FOREIGN KEY (superseded_by, character_id) REFERENCES memory(id, character_id)`）は
    /// **別キャラクターの行を指す UPDATE を構造的に潰す**
    /// （`tests/store_schema.rs::memory_superseded_by_cannot_point_to_another_character`）。
    /// **それでも入口で弾くのは、理由が届かないから。**
    /// 外部キーに任せると `Query`（"データベース操作に失敗しました"）になり、
    /// **「引き当てが無い」と区別が付かない。** しかも FK は接続ごとの
    /// `PRAGMA foreign_keys` に依存する（`UnknownSession` が外部キー違反を名乗らないのと同じ理由）。
    ///
    /// **さらに、入口で弾かないと訂正版が先に書き込まれる。** INSERT は成功し、
    /// UPDATE だけが FK で落ちてロールバックされる——結果は同じでも、
    /// **拒否の理由を決める場所が「アプリの判断」から「DB の偶然」に移る。**
    #[error("その記憶は別のキャラクターのものです")]
    MemoryBelongsToAnotherCharacter,

    /// **すでに上書きされた記憶を、もう一度上書きしようとした**（ADR-0022 / 作業単位 14a）。
    ///
    /// # 鎖を線形に保つため
    ///
    /// 1つの古い行が2つの後継を持つと、「以前はXだったが今はY」の遡行（フェーズ2）が
    /// **どちらを辿ればよいか決められない。** 上書き済みの行を指した呼び出しは、
    /// ほぼ確実に**一覧を取り直していない**（一覧は有効な行しか返さない）。
    ///
    /// **`UnknownMemory` に相乗りさせない。** 「行が無い」なら id が間違っているが、
    /// こちらは**行はある・ただし指すべきは後継の方**で、対処が違う。
    /// 呼び出し側は「いま有効な方を指定してください」と案内できる。
    ///
    /// > **循環は構造的に作れない。** 後継は**必ず新しく作る行**なので、
    /// > 後継が古い行を先に指していることはありえない（`supersede_memory` の doc）。
    #[error("その記憶はすでに上書きされています。いま有効な記憶を指定してください")]
    MemoryAlreadySuperseded,

    /// **空いた枠が無いのに軸の候補を作ろうとした**（ADR-0046 §決定1 / 作業単位 A-9a）。
    ///
    /// 枠を埋めるのは試用・合格・採用の候補（`AxisCandidateState::occupies_slot`）。
    /// **枠の数が 0 のキャラクターでは常にこれになる**——既存の DB の振る舞いを変えない形。
    /// 判定は書き込みと同じトランザクションの中で行う（`BEGIN IMMEDIATE`）。
    #[error("軸を見いだす枠が空いていません")]
    AxisDiscoverySlotsFull,

    /// **そのキャラクターの軸の候補ではない**（ADR-0046 / 作業単位 A-9a）。
    ///
    /// 「その id の候補が無い」と「他のキャラクターの候補」を分けない——
    /// どちらもこのキャラクターからは見えない候補で、一覧も他のキャラクターの候補を出さない。
    /// **`UnknownMemory` に相乗りさせない**（指している表が違う）。
    #[error("軸の候補が見つかりません")]
    UnknownAxisCandidate,

    /// **軸の候補を、規則にない状態へ進めようとした**（ADR-0046 §決定3〜5）。
    ///
    /// 規則は `AxisCandidateState::can_become`（試用 → 合格／不合格、合格 → 採用／却下）。
    /// **何も書かれない。**
    #[error("軸の候補をその状態へは進められません")]
    AxisCandidateTransitionRefused,

    /// **試用中でない軸の候補に、試用の票を足そうとした**（ADR-0046 §決定3）。**何も書かれない。**
    ///
    /// **`AxisCandidateTransitionRefused` と潰さない**——票と遷移は別の書き込みで、
    /// 呼び出し側（A-9c）が「判定の後に遅れて来た票」を区別して数えられるように。
    #[error("軸の候補は試用中ではありません")]
    AxisCandidateNotOnTrial,

    /// **試用の票に、候補の持ち主ではないキャラクターの会話を渡した**
    /// （ADR-0046 §決定3 / 作業単位 A-9c / Issue origin#43 の申し送り2）。**何も書かれない。**
    ///
    /// 試用の票の表の外部キーは会話が**在る**ことしか見ないので、書き手が同じトランザクションで
    /// `session.character_id` を照合する。**その id の会話が無い場合もここに落ちる**
    /// （どちらもこのキャラクターからは見えない会話 / `UnknownAxisCandidate` と同じ畳み方）。
    ///
    /// **`UnknownSession` に相乗りさせない** —— あちらは「宛先の会話が無い」で、
    /// こちらは「会話と候補の持ち主が食い違っている」。呼び出し側が渡した組の取り違えである。
    ///
    /// **裏方の書き込みの会話の縛り（`*_in_session` / 作業単位 A-10）でも使う** ——
    /// 渡した会話が在るのに別のキャラクターのものだったとき。**`TurnSessionGone`（会話が無い＝
    /// キャラクターが消えた）と分けるのが要点**で、こちらは本物の故障として `warn` に出す
    /// （`server::extract::character_vanished` はこちらを「消えた」と読まない）。
    #[error("その会話は書き込み先のキャラクターのものではありません")]
    SessionOfAnotherCharacter,

    /// **有効な取り決めが上限に達しているのに、もう1件書こうとした**
    /// （ADR-0051 §決定4 / 作業単位 A-12a）。**何も書かれない。**
    ///
    /// 上限は `domain::MAX_ACTIVE_AGREEMENTS`。**黙って古いものを落とさない** ——
    /// 呼び出し側（画面）が、古い取り決めを終わらせるよう促す。
    /// 判定は書き込みと同じトランザクションの中（`BEGIN IMMEDIATE`）。
    #[error("有効な取り決めが上限に達しています")]
    AgreementLimitReached,

    /// **そのキャラクターの取り決めではない**（作業単位 A-12a）。
    ///
    /// その id の行が無いのと、他のキャラクターの行なのを分けない（どちらもこのキャラクターからは
    /// 見えない / `UnknownAxisCandidate` と同じ畳み方）。**`UnknownMemory` に相乗りさせない。**
    #[error("取り決めが見つかりません")]
    UnknownAgreement,

    /// **有効でない取り決め（終わったもの・提案中・却下）を終わらせようとした**（作業単位 A-12a。
    /// 提案中・却下は A-12c-1 で増えた）。**何も書かれない。**
    ///
    /// **`UnknownAgreement` に潰さない** —— 行はこのキャラクターのもので、状態だけが衝突している。
    #[error("その取り決めは有効ではありません")]
    AgreementNotActive,

    /// **提案中でない取り決め（有効・終わったもの・却下）を、採る・捨てるしようとした**
    /// （作業単位 A-12c-1）。**何も書かれない。** `AgreementNotActive` と同じ立場の 409。
    #[error("その取り決めは提案中ではありません")]
    AgreementNotProposed,

    /// 格納されていた値がドメインの不変条件を満たしていない。
    ///
    /// **「DB を直接編集した場合にのみ現れる」ではない**（作業単位 0009 / 0009a）。
    /// **検証を後から厳しくすると、`store` 経由で書けた行が不正になる。**
    /// 実例と影響範囲は `Store::list_characters` の doc を参照
    /// （**一覧はこのエラーを返さない。** 行ごとに指し示す形にしてある / ADR-0016 §3）。
    ///
    /// 詳細を載せないのは、格納値そのものがエラーレスポンスに漏れるのを避けるため。
    #[error("保存されている値の形式が不正です")]
    InvalidStoredValue,
}

impl StoreError {
    /// UNIQUE / PRIMARY KEY 制約に違反したか。
    ///
    /// 呼び出し側（`server`）が 409 / 400 に振り分けるために使う。
    /// **判定をここに置くのは、sqlx の型を `store` の外へ出さないため。**
    /// 呼び出し側で `as_database_error()` を辿らせると、`server` が
    /// 「永続化の実体が sqlx であること」を知ることになる。
    ///
    /// **真になるのは `Query` の中身が UNIQUE / PRIMARY KEY 違反のときだけ。**
    /// 0004b-2 で正規化キーの索引を落としたので、`character.handle` の UNIQUE 違反は
    /// 「まったく同じ綴りの行が既にある」以外の意味を持たない
    /// （それ以前は「ゴースト行が塞いでいる」場合があり、専用のバリアントに分けていた）。
    #[must_use]
    pub fn is_unique_violation(&self) -> bool {
        match self {
            Self::Query(err) => err
                .as_database_error()
                .is_some_and(|db| db.kind() == sqlx::error::ErrorKind::UniqueViolation),
            _ => false,
        }
    }
}

/// SQLite への接続プールを保持するハンドル。
#[derive(Clone, Debug)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// 指定パスの SQLite に接続し、マイグレーションを適用する。
    ///
    /// ファイルが無ければ作成する。既に最新まで適用済みなら何もしない（冪等）。
    ///
    /// **マイグレーションに失敗した場合はプールを閉じてから `Err` を返す。**
    /// 呼び出し側は `Store` を受け取れないので、ここで閉じないと誰も閉じられない。
    /// 失われるユーザデータは無いが、`-wal` / `-shm` とファイルロックが
    /// プロセス終了まで残る。
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            // SQLite は外部キー制約を既定で無効にする。プールが張る**接続ごとに**
            // 有効化しないと、制約がエラーにならず黙って素通りする。
            .foreign_keys(true)
            // sqlx 0.9 は journal_mode を明示しない限り設定しない（既定の `delete` のまま）。
            // `delete` では読みトランザクション中の書き込みが数秒待たされた末に
            // `database is locked` で落ちる。メモリ抽出は本文レスポンスを返し切った後に
            // 非同期で走らせる（= 読みと書きが常時同時に走る）ため、WAL が前提。
            //
            // WAL への切り替えは DB 作成後だと排他ロックを要するため、**接続確立時に**指定する。
            .journal_mode(SqliteJournalMode::Wal);

        let pool = SqlitePoolOptions::new()
            .connect_with(options)
            .await
            .map_err(StoreError::Connect)?;

        let store = Self { pool };

        // **`?` で素通しにしない。** 失敗するとプールを掴んだまま `Err` が返り、
        // 呼び出し側には閉じる手段が無い（`Store` を受け取れないため）。
        if let Err(err) = store.migrate().await {
            store.close().await;
            return Err(err);
        }

        Ok(store)
    }

    /// 未適用のマイグレーションを適用する。
    ///
    /// **走らせる前に、帳簿の改行違いを1段だけ直す**（ADR-0044 決定3 /
    /// [`migration_ledger::repair_line_ending_checksums`]）。
    /// **`connect` ではなくここに置く**のは、`MIGRATOR.run` の直前であることが
    /// 主張の本体だから —— この関数を直に呼ぶ経路が将来できても、同じ順序で通る。
    pub async fn migrate(&self) -> Result<(), StoreError> {
        migration_ledger::repair_line_ending_checksums(&self.pool).await?;

        MIGRATOR.run(&self.pool).await.map_err(|err| match err {
            // **この exe が知らない版が適用済み** = 新しい版で開かれた保存先。
            sqlx::migrate::MigrateError::VersionMissing(_) => {
                StoreError::DatabaseFromNewerVersion(err)
            }
            other => StoreError::Migrate(other),
        })?;
        // 適用件数 0 でも毎回出るため info では起動ログのノイズになる。
        tracing::debug!("マイグレーションの適用状態を確認しました");
        Ok(())
    }

    /// 接続プールへの参照。**テスト専用の抜け穴。**
    ///
    /// これを本番コードから使えると、`store` に無いクエリが必要になったとき
    /// 「プールを掴んで直接 SQL を書く」のが最短経路になり、
    /// `Handle::parse` を経ていない値が DB に届く。モジュール doc の宣言
    /// （検証済みのドメイン型しか受け取らない）が成立しなくなるため、
    /// `testing` フィーチャでゲートして本番ビルドから消してある。
    #[cfg(any(test, feature = "testing"))]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// プールを閉じ、進行中の接続を片付ける。
    pub async fn close(&self) {
        self.pool.close().await;
    }
}
