//! 記憶のドメイン型。

use std::fmt;

use chrono::{DateTime, Utc};

use super::text::{contains_control_or_line_break, is_effectively_blank, trim_blank};

/// `memory` テーブルの1行。
///
/// **読み書きは `store`、置き場所はここ**（理由は `super` の doc）。
///
/// **`superseded_by` は持たない。** 読み出しは有効な記憶（`superseded_by IS NULL`）だけを
/// 返すため常に NULL であり、フィールドにすると「読めるなら書けるはず」という
/// 誤解の入口になる。フェーズ2で遡行の読み出しを足すときは、そのための型を別に用意する。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Memory {
    /// 主キー。
    pub id: MemoryId,
    /// どのキャラクターの記憶か。
    pub character_id: super::CharacterId,
    /// **誰についての記憶か。** 落とすと一人称が混線する。
    pub subject: Subject,
    /// 記憶本文。
    pub content: MemoryContent,
    /// 分類。未設定なら `None`。
    pub category: Option<MemoryCategory>,
    /// 作成日時（UTC）。
    pub created_at: DateTime<Utc>,
    /// 手動ロック。**読み出しではフィルタしない**（表示・保護の判断は上位層の責務）。
    pub locked: bool,
}

/// `memory` テーブルの主キー。
///
/// `CharacterId` / `SessionId` / `MessageId` と**別の型**にしてある。どれも中身は `i64` だが、
/// 取り違えても外部キーは黙って通る（`memory.id = 3` と `session.id = 3` は
/// ほぼ常に両方存在する）。
///
/// `From<i64>` も `pub fn new` も生やさない。得る手段は `Store::add_memory` の
/// 戻り値と `Memory::id` だけ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemoryId(i64);

impl MemoryId {
    /// DB から復元する内部経路。**crate 外からは呼べない。**
    pub(crate) const fn from_row(raw: i64) -> Self {
        Self(raw)
    }

    /// 生の主キー値。SQL へのバインド値はこれを使う。
    pub const fn get(self) -> i64 {
        self.0
    }

    /// **テスト専用の抜け穴。** 「存在しない id」を作る用途にのみ使う。
    #[cfg(any(test, feature = "testing"))]
    pub const fn from_raw_for_tests(raw: i64) -> Self {
        Self(raw)
    }
}

impl fmt::Display for MemoryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 記憶の主体。**誰についての記憶か。**
///
/// **`String` で持たず、`Option` にもしない**（CLAUDE.md 絶対制約 /
/// docs/03-data-model.md の不変条件1）。「猫が好き」がユーザの発言かキャラの発言かを
/// 区別しないと一人称が混線し、**キャラがユーザの記憶を自分のものとして喋り出す。**
/// 省略できない形にするため、`NewMemory` の必須フィールドにしてある。
///
/// スキーマの `CHECK (subject IN ('user', 'character'))` と対応する2値。
/// **DB との相互変換は `store` に閉じ込める**（`src/store/memory.rs`）。
/// ここに `as_str` を生やすと、綴りの決定権が `domain` と `store` に分散する。
///
/// `Role` とは**別の型**。値の綴りは片方が `assistant`、こちらが `character` で異なり、
/// 混同すると CHECK 違反として実行時にしか表面化しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subject {
    /// 利用者についての記憶。
    User,
    /// キャラクターについての記憶。
    Character,
}

impl Subject {
    /// **全部の値**（作業単位 P-2a —— 仕様書が応答の綴りの `enum` をここから作る）。
    ///
    /// **綴りはここに無い**（HTTP の綴りは `server::admin` の `subject_to_wire`、DB の綴りは `store`）。
    /// 数え漏れは `every_subject_variant_is_listed_in_all` が見る。
    pub const ALL: [Self; 2] = [Self::User, Self::Character];

    /// **記憶抽出のやり取りで使う語**（ADR-0021 / 作業単位 13a）。
    ///
    /// 抽出プロンプトが「この2語だけを主語に使え」と指示し、
    /// 抽出パーサが**この2語だけ**を主語として受け取る。
    /// **綴りが片側だけ動くと全件が破棄される**ので、両者が引く定義をここ1箇所に置く
    /// （`prompt::extraction` と `parse::extraction` は互いに依存しない）。
    ///
    /// 語彙を `利用者` / `あなた` にしたのは、
    /// 本文プロンプトの小見出し（`## 利用者について` / `## あなた自身について`）と
    /// 同じ語だから。**モデルが見た語をそのまま返せる**（ADR-0021）。
    ///
    /// # **DB の綴り（`user` / `character`）とは別物**
    ///
    /// こちらは**プロンプトに載る日本語**で、スキーマの
    /// `CHECK (subject IN ('user', 'character'))` とは何の関係も無い。
    /// DB との相互変換は従来どおり `store` に閉じ込めてある
    /// （この型の doc）。**ここを DB の綴りに使わないこと。**
    pub const fn extraction_word(self) -> &'static str {
        match self {
            Self::User => "利用者",
            Self::Character => "あなた",
        }
    }
}

/// 検証・正規化を通った記憶の本文。
///
/// **`parse` が唯一のコンストラクタ**（`Handle` / `StatAxis` と同じ発想）。
///
/// # 空文字を拒否する理由
///
/// `message.raw_content` は空文字を**許す**が、こちらは**拒否する**。非対称は意図的。
///
/// * `raw_content` は「LLM が空を返した」という事象そのものが調査材料であり、
///   拒否するとその事実を記録できなくなる。
/// * 記憶は違う。抽出サブLLM が生んだ**二次生成物**で、空の記憶は
///   ADR-0003 の全件注入で「- （空行）」としてプロンプトに載り、
///   トークン予算だけを食って何も伝えない。しかも**発生源の調査材料は
///   `message.raw_content` 側に残っている**ので、拒否しても失うものが無い。
///
/// 前後の空白は除去する。中間の空白・改行は保持する（記憶本文は散文であり、
/// 「2つの街に住んだ」のような複数行の記憶を落としたくない）。
/// **制御文字も拒否しない**（`StatAxis` との非対称。軸名やキャラ名は照合キー・
/// 短いラベルだが、本文は自由文）。プロンプトへの整形は `prompt` 層の責務。
///
/// # 長すぎる本文を拒否する理由
///
/// 空文字を拒否するのと**まったく同じ論法**が使える。記憶は抽出サブLLM が生んだ
/// 二次生成物であり、弾いても発生源の調査材料は `message.raw_content` 側に残る。
///
/// 拒否しないと壊れ方が重い。ADR-0003 は**全件注入**なので、抽出が一度でも暴走して
/// 会話ログ丸ごとを1件の記憶として吐くと、**そのキャラの以後の全プロンプトに
/// 永久に載り続ける。** しかもフェーズ1の `store` には削除も上書きも
/// `superseded_by` の書き込み API も無く、**アプリ経由では二度と取り除けない**
/// （手段は DB の直接編集だけ）。入口で止めるのが唯一の防御になる。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemoryContent(String);

impl MemoryContent {
    /// 本文の上限。**文字数**（Unicode スカラー値の個数）で数える。
    ///
    /// # なぜ 500 文字か
    ///
    /// 記憶は「猫が好き」のような短い事実の集まりで、ADR-0003 の見積もりも
    /// **1件あたり平均30トークン**を前提にしている。500文字は日本語でも
    /// その10倍以上の余裕があり、**正当な記憶を弾かない**。
    /// 一方で会話ログ丸ごと（数千〜数十万文字）は確実に止まる。
    /// 上限の役目は「1件の暴走を止める」ことであって、
    /// 総量の管理（トークン予算配分）は `prompt` 層の責務。
    ///
    /// # なぜバイト数で数えないか
    ///
    /// UTF-8 では日本語1文字が3バイトなので、**バイト上限にすると日本語の記憶だけが
    /// 英語の 1/3 しか書けなくなる。** 主用途が日本語である以上この非対称は許容できない。
    /// 書記素クラスタ（結合文字を1つと数える）まで踏み込まないのは、外部クレートが要る割に
    /// 「暴走を止める」という目的に対して精度が過剰なため。絵文字の一部が
    /// 複数文字として数えられる程度のずれは、この上限では問題にならない。
    ///
    /// # **この値は後から下げられない**
    ///
    /// 同じ定数が**書き込みの検証**（`Store::add_memory` の入口）と**読み出しの検証**
    /// （`StoreError::InvalidStoredValue`）の両方に使われる。500 → 300 に下げると、
    /// **保存済みの正当な 400 文字の記憶が即座に想起全体を止め**、`store` には削除も
    /// 上書きも無いので **DB の直接編集以外に復旧手段が無い。**
    /// 下げるなら既存データを詰めるマイグレーションが要る。上げる方向は安全。
    pub const MAX_CHARS: usize = 500;

    /// 唯一のコンストラクタ。正規化してから検証する。
    pub fn parse(raw: &str) -> Result<Self, ContentError> {
        let normalized = trim_blank(raw);

        if normalized.is_empty() {
            return Err(ContentError::Empty);
        }
        // **「広い意味で空白だけ」も空として拒否する**（作業単位 0009）。
        // 行区切りを丸ごと拒否はできない（複数行の記憶は正当）が、
        // 全部が空白相当なら `prompt::SafeText::sanitize` の後に何も残らず、
        // 全件注入のプロンプトに `> -` だけの行が載る。
        if is_effectively_blank(normalized) {
            return Err(ContentError::Empty);
        }
        // **長さの判定は正規化の後。** 前に置くと、前後に空白を足しただけで
        // 正当な記憶が弾かれる。
        if normalized.chars().count() > Self::MAX_CHARS {
            return Err(ContentError::TooLong);
        }

        Ok(Self(normalized.to_owned()))
    }

    /// 正規化後の文字列。SQL へのバインド値はこれを使う。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MemoryContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 記憶の本文が受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ContentError {
    /// 正規化すると空になる（空文字・空白のみ）。
    #[error("記憶の本文が空です")]
    Empty,

    /// 正規化後の文字数が `MemoryContent::MAX_CHARS` を超える。
    ///
    /// **上限値をメッセージに埋め込まない。** `Display` は定数文字列に保つ
    /// （CLAUDE.md 絶対制約）。弾いた本文そのものは `message.raw_content` に残る。
    #[error("記憶の本文が長すぎます")]
    TooLong,
}

/// 検証・正規化を通った記憶の分類。
///
/// 本文と違い**短いラベル**（「好み」「経歴」など）で、プロンプトには見出しとして載る。
/// そのため `StatAxis` と同じ扱いにし、空文字と制御文字を拒否する。
/// 分類そのものは任意（`memory.category` は NULL 可）。
///
/// # 長すぎる分類を拒否する理由
///
/// **`MemoryContent` の上限とまったく同じ論法が当てはまる。** 分類も抽出サブLLM が生んだ
/// 二次生成物であり、全件注入では**見出しとしてプロンプトに載る**。フェーズ1には削除も
/// 上書きも無いので、一度入ると**アプリ経由では二度と取り除けない。**
///
/// 「分類は短いから暴走しない」とは考えない。**本文と分類を取り違えて分類欄に
/// 会話ログを吐くのは、小型モデルでは十分に起こりうる**（CLAUDE.md「小型モデルは
/// 形式を崩す前提」）。`MemoryContent::parse` は本文側しか見ないため、
/// ここで止めなければ素通りする。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemoryCategory(String);

impl MemoryCategory {
    /// 分類の上限。**文字数**（Unicode スカラー値の個数）で数える。
    ///
    /// # なぜ 50 文字か
    ///
    /// 分類は「好み」「経歴」「家族構成」のような**見出し**であり、
    /// 実際の値はどれも数文字に収まる。50 文字はその10倍以上の余裕があるので
    /// 正当な分類を弾かない一方、会話ログ丸ごとは確実に止まる。
    /// 本文（500 文字）より小さいのは、分類が散文ではなくラベルだから。
    ///
    /// バイト数で数えないのは `MemoryContent::MAX_CHARS` と同じ理由
    /// （UTF-8 では日本語1文字が3バイトで、日本語だけが 1/3 しか書けなくなる）。
    ///
    /// # **この値は後から下げられない**
    ///
    /// 同じ定数が書き込みの検証と読み出しの検証（`StoreError::InvalidStoredValue`）の
    /// 両方に使われる。下げると**保存済みの正当な分類が想起全体を止め**、
    /// `store` に削除も上書きも無いので DB の直接編集以外に復旧手段が無い。
    /// 下げるならデータを詰めるマイグレーションが要る。上げる方向は安全。
    pub const MAX_CHARS: usize = 50;

    /// 唯一のコンストラクタ。正規化してから検証する。
    pub fn parse(raw: &str) -> Result<Self, CategoryError> {
        let normalized = trim_blank(raw);

        if normalized.is_empty() {
            return Err(CategoryError::Empty);
        }
        // **見えない文字だけの分類も「空」として拒否する**（作業単位 0009a）。
        // 0009 は行区切りしか見ておらず、`MemoryCategory::parse("\u{2000}")` が通って
        // **見出しの中身が消えた `> - [ ] 猫が好き`** がプロンプトに載っていた。
        if is_effectively_blank(normalized) {
            return Err(CategoryError::Empty);
        }
        if contains_control_or_line_break(normalized) {
            return Err(CategoryError::Control);
        }
        // **長さの判定は正規化の後。** 前に置くと、前後に空白を足しただけで
        // 正当な分類が弾かれる。
        if normalized.chars().count() > Self::MAX_CHARS {
            return Err(CategoryError::TooLong);
        }

        Ok(Self(normalized.to_owned()))
    }

    /// 正規化後の文字列。SQL へのバインド値はこれを使う。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 記憶の分類が受け付けられない理由。`Display` は定数文字列（CLAUDE.md 絶対制約）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CategoryError {
    /// 正規化すると空になる（空文字・空白のみ）。
    #[error("記憶の分類が空です")]
    Empty,

    /// 制御文字を含む。
    #[error("記憶の分類に改行や制御文字は使えません")]
    Control,

    /// 正規化後の文字数が `MemoryCategory::MAX_CHARS` を超える。
    ///
    /// **上限値をメッセージに埋め込まない。** `Display` は定数文字列に保つ
    /// （CLAUDE.md 絶対制約）。弾いた値そのものは `message.raw_content` に残る。
    #[error("記憶の分類が長すぎます")]
    TooLong,
}

/// これから書き込む記憶。
///
/// **`subject` は `Option` ではない。** 構造体リテラルでしか作れないので、
/// うっかり省略するとコンパイルが通らない（不変条件1を型で表現する）。
///
/// `superseded_by` と `locked` は**ここに無い**。前者はフェーズ2で古い行を指すための列で、
/// 追加時に埋めるものではない。後者は手動運用のフラグで、既定値は DB 側が持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMemory {
    /// 誰についての記憶か。**省略できない。**
    pub subject: Subject,
    /// 記憶本文。
    pub content: MemoryContent,
    /// 分類。未設定なら `None`。
    pub category: Option<MemoryCategory>,
}

impl NewMemory {
    /// **会話1ターンから積める記憶の上限**（ADR-0021 / 作業単位 13a）。
    ///
    /// 上限が無いと、饒舌なモデルが1ターンで `memory` を溢れさせる。
    /// `memory` には削除も上書きも無いので、**入ったものは永久に残り、
    /// 想起の質を静かに下げ続ける。**
    ///
    /// **超えた分は捨てて数える。切り詰めではない**——捨てたことが
    /// `parse::extraction` の破棄集計に残る（数に残らない黙った切り捨てを作らない）。
    ///
    /// # なぜ `domain` にあるか
    ///
    /// **2つの純粋モジュールが同じ数を必要とする。** `prompt::extraction` は
    /// 「1回に書けるのは5件まで」とモデルに伝え、`parse::extraction` はその件数で止める。
    /// 写しを2つ持つと、**プロンプトが宣言する上限と実際に効く上限が黙ってずれる**
    /// （モデルは6件書いてよいと読み、1件が毎回捨てられる）。
    /// `prompt` と `parse` は互いに依存しない設計（CLAUDE.md 3節）なので、
    /// 共有できる場所は `domain` しかない。
    ///
    /// # この値は実測ではなく見積もり
    ///
    /// ADR-0021 が「実機で回してから見直す」と明記している。
    /// **上げる方向は安全**（既存データに影響しない）。
    pub const MAX_PER_TURN: u32 = 5;
}

/// 修復のための一覧の1件。**読めた行と、読めなかった行の両方**を表す
/// （ADR-0022 §2 / 作業単位 14a）。
///
/// # **これは想起ではない**
///
/// 想起の入口は `Store::recall_memories` **ひとつだけ**（ADR-0003 / ADR-0023）。
/// こちらは**管理のための一覧**で、**プロンプトには一切載らない。**
/// 読み出す関数の名前も置き場所も分けてある（`Store::list_memories_for_repair` /
/// `src/store/memory_repair.rs`）。
///
/// **`prompt` からこの型を触らないこと。** 触りたくなったら、それは想起の入口が
/// 2本になろうとしている合図である。
///
/// # なぜ `Vec<Memory>` ではないのか
///
/// `recall_memories` は**1行でも壊れていると丸ごと `InvalidStoredValue` で落ちる**
/// （`memory_from_row` の doc）。**それは想起としては正しい**——記憶を静かに減らすと
/// キャラクターの振る舞いだけが不可視に変わるので、うるさく止める方を選んでいる。
///
/// **しかし修復には使えない。** 直したい行がある状態では読めないので、
/// 「どの行が壊れているか」を返す経路が別に要る。`CharacterEntry` が
/// キャラクターで解いたのとまったく同じ問題（ADR-0016 §3）。
///
/// # **型では守れない。守っているのはテスト**
///
/// 列挙にすると1件ごとに `match` で書き分けることにはなるが、
/// **「壊れた行を応答に出す」ことを型が強制するわけではない。**
/// `filter_map` で `Unreadable` を落とす実装は**そのままコンパイルが通る**
/// （`CharacterEntry` で実測済み / 0012b）。
///
/// **`ValidTag`（ADR-0006）と同じ強さだと読まないこと。** あちらは辞書引き以外の
/// 構築手段が無く「表現できない」が、こちらは**表現できてしまう。**
/// 黙って落とす実装を赤にしているのは `tests/store_memory.rs` の次の見張り番:
///
/// * `a_broken_memory_row_does_not_take_the_healthy_ones_with_it`
/// * `every_kind_of_broken_memory_column_is_pointed_at`
/// * `the_repair_listing_stays_readable_where_recall_stops`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryEntry {
    /// 全列が `parse` を通った行。
    Readable(Memory),
    /// **1列以上が `parse` を通らなかった行。** 修復の対象（ADR-0022 §2）。
    Unreadable(UnreadableMemory),
}

/// 読めなかった記憶の**手掛かり**。修復に必要な最小限だけを持つ。
///
/// # **格納値そのものは持たない**
///
/// 上限超過の本文は数千文字ありうるうえ、何が入っているか分からない
/// （`StoreError::InvalidStoredValue` の `Display` に格納値を載せないのと同じ判断。
/// `UnreadableCharacter` も同じ）。修復に要るのは「どの行か」と「どの列か」であって、
/// 壊れた値そのものではない。中身を確かめたいときは `id` で DB を直接見る。
///
/// # 修復の宛先はここから作れる
///
/// `Store::supersede_memory` は `(CharacterId, MemoryId)` で行を指す。
/// **両方をこの型が持っている**ので、一覧が指した行は必ず訂正できる
/// （見張り番は `tests/store_memory.rs::a_broken_memory_can_still_be_superseded`）。
/// `UnreadableCharacter::handle` が `None` になりうる——つまり**指せない行がある**——のとは
/// 対照的で、記憶の主キーは `parse` を通さないので**指せない行は無い。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableMemory {
    /// 主キー。**この行を指す唯一の確実な手掛かり。**
    pub id: MemoryId,
    /// どのキャラクターの記憶か。`Memory` と揃えてある。
    pub character_id: super::CharacterId,
    /// 読めなかった列。**壊れ方が複数なら全部並べる**（1つ直しても読めないことが分かる）。
    ///
    /// 空にはならない（空なら読めた行として返している）。
    pub invalid_columns: Vec<MemoryColumn>,
}

/// **上書きされた記憶1件**（ADR-0022 / 作業単位 D-3b）。
///
/// # なぜ [`MemoryEntry`] を包んでいるのか
///
/// 行の**組み立てと検証を写さない**ため（`DriftVoteRecord` が `StoredDriftVote` を
/// 包んでいるのと同じ形）。違いは「どの行に置き換わったか」を持つことだけで、
/// **有効な行にはその値が無い**——だから `MemoryEntry` の側に列を足さない。
/// 足すと、有効な行にも常に `None` が付いて回り、
/// **「まだ上書きされていない」と「上書き先を読めなかった」が同じ形になる。**
///
/// # **これも想起ではない**
///
/// 想起の入口は `Store::recall_memories` **ひとつだけ**（ADR-0003 / ADR-0023）で、
/// **`superseded_by IS NULL` の行しか読まない。**
/// この型が表すのは**その条件から外れた行**——つまり
/// **想起に載ることが二度と無い行**である。**`prompt` から触らないこと。**
///
/// # 壊れた行も表せる
///
/// `entry` が [`MemoryEntry::Unreadable`] になりうる。
/// 上書き済みの行は**訂正して想起から外した後も残る**ので、壊れたまま残っている行も
/// ある。読み飛ばすと**壊れた環だけが鎖から落ちる**（総数には残るので、
/// 読み手には数が合わないとしか見えない）。
/// **型は強制しない**（`filter_map` で捨てる実装はコンパイルが通る / `MemoryEntry` の doc）。
/// 守っているのは
/// `tests/store_memory.rs::a_broken_superseded_row_is_pointed_at_without_taking_the_healthy_ones`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersededMemory {
    /// 上書きされた行そのもの。**本文も分類も作成日時も、書かれたときのまま。**
    ///
    /// 「以前はXだったが今はY」の X の側。**消えていない**（時系列保持 / 絶対制約）。
    pub entry: MemoryEntry,
    /// **どの行に置き換わったか。**
    ///
    /// `Option` ではない——**この型は `superseded_by` が NULL でない行だけ**を表す。
    /// 落とすと鎖が辿れなくなり、**残っているのは「消えた記憶の山」**でしかなくなる。
    pub superseded_by: MemoryId,
}

/// `memory` の列のうち、読み出しで `parse` を通すもの。
///
/// 文字列表現は**DB の列名と同じ**にしてある（手掛かりから DB を直接見に行けるように）。
/// 綴りが実在の列名からずれると、応答を見た運用者が
/// `sqlite3` で `SELECT <その名前> FROM memory` を打って `no such column` で止まる
/// ——**指し示す先が存在しない**という、最も避けたい壊れ方になる。
/// 見張り番は `tests/store_schema.rs::every_pointed_at_column_is_a_real_memory_column`
/// （`PRAGMA table_info(memory)` と突き合わせるので、列名を変えるマイグレーションにも効く）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MemoryColumn {
    /// 誰についての記憶か。スキーマの CHECK があるので普通は壊れないが、
    /// `PRAGMA ignore_check_constraints` で外せる（`tests/common` の抜け穴と同じ）。
    Subject,
    /// 記憶本文。**上限を後から厳しくすると、`store` 経由で書けた行が不正になる。**
    Content,
    /// 分類。**NULL は「未設定」であって不正ではない。**
    Category,
    /// 手動ロック。
    ///
    /// # なぜ**意味の未確定な列**を指し示せるようにしてあるのか
    ///
    /// ADR-0022 §4 は「`locked` の意味はまだ確定していないので、この作業単位では
    /// 読みも書きもしない」と決めている。**この列挙子はその決定に反しない。**
    /// ここで決めていることは「`locked` に 0 / 1 以外が入っていたら**読めない行**である」
    /// だけで、`locked` が何を意味するかには一切触れていない
    /// （**絞り込みにも使わないし、書き込みもしない**）。
    ///
    /// **外せない理由の方が強い。** `memory_from_row` は
    /// `locked` が 0 / 1 以外の行を `InvalidStoredValue` で弾く
    /// ——つまり**その1行でチャットも抽出も止まる**（ADR-0022 の背景そのもの）。
    /// この列挙子が無いと、一覧はその行を**「読めた行」として並べる**ことになり、
    /// **故障を起こしている当の行だけが修復の入口から見えない。**
    Locked,
}

impl MemoryColumn {
    /// **全 variant。** 列名の突き合わせ（上の見張り番）が「全部」を回るために要る。
    ///
    /// 数え漏らしは `every_memory_column_variant_is_listed_in_all` が止める
    /// （variant を足すとあのテストの `match` がコンパイルできなくなる）。
    pub const ALL: [Self; 4] = [Self::Subject, Self::Content, Self::Category, Self::Locked];

    /// DB の列名。ログと API 応答に出す。
    ///
    /// **`Display` は生やさない**（`CharacterColumn` と同じ理由。同じ文字列を返す経路が
    /// 2本あると、片方だけが綴りを変えられても気付けない）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Subject => "subject",
            Self::Content => "content",
            Self::Category => "category",
            Self::Locked => "locked",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LINE_BREAKS;
    use crate::domain::text::INVISIBLE_SAMPLES;

    /// 正規化すると空になる文字列。`StatAxis` と同じ集合。
    const BLANKS: [&str; 12] = [
        "",
        " ",
        "   ",
        "\t",
        "\n",
        "\u{000B}",
        "\u{000C}",
        "\r",
        "\u{3000}",
        "\u{00A0}",
        " \u{3000}\t\r\n",
        "\u{000B}\u{000C}\u{00A0}\u{3000} \t",
    ];

    fn content(raw: &str) -> MemoryContent {
        MemoryContent::parse(raw).expect("正当な本文として解釈できること")
    }

    fn category(raw: &str) -> MemoryCategory {
        MemoryCategory::parse(raw).expect("正当な分類として解釈できること")
    }

    /// 生の値を往復させる（`store` のバインド値として使うため）。
    #[test]
    fn memory_id_round_trips_the_raw_key() {
        for raw in [1_i64, 42, i64::MAX] {
            assert_eq!(MemoryId::from_row(raw).get(), raw);
        }
        assert_eq!(MemoryId::from_row(42).to_string(), "42");
    }

    /// 値が違えば別の id。同じなら等しい。
    #[test]
    fn memory_id_compares_by_value() {
        assert_eq!(MemoryId::from_row(3), MemoryId::from_row(3));
        assert_ne!(MemoryId::from_row(3), MemoryId::from_row(4));
        assert!(MemoryId::from_row(3) < MemoryId::from_row(4));
    }

    /// テスト専用の抜け穴も同じ表現を作る。
    #[test]
    fn test_only_constructor_matches_the_internal_one() {
        assert_eq!(
            MemoryId::from_raw_for_tests(9_999),
            MemoryId::from_row(9_999)
        );
    }

    /// 主体は2値。取り違えを検出できるよう比較可能にしてある。
    #[test]
    fn subject_has_two_distinct_values() {
        assert_eq!(Subject::User, Subject::User);
        assert_ne!(Subject::User, Subject::Character);
    }

    /// 本文の前後の空白は除去される。
    #[test]
    fn content_trims_surrounding_blanks() {
        for (raw, expected) in [
            (" 猫が好き ", "猫が好き"),
            ("\u{3000}猫が好き\u{3000}", "猫が好き"),
            ("\tlikes cats\n", "likes cats"),
            ("\u{00A0}likes cats\u{00A0}", "likes cats"),
        ] {
            assert_eq!(content(raw).as_str(), expected, "raw = {raw:?}");
        }
    }

    /// 本文の中間の空白・改行は保持する（散文をそのまま残す）。
    #[test]
    fn content_keeps_inner_blanks_and_newlines() {
        for raw in ["猫が 好き", "1行目\n2行目", "a\tb"] {
            assert_eq!(content(raw).as_str(), raw, "raw = {raw:?}");
        }
    }

    /// **空の本文は作れない。**
    ///
    /// `raw_content` と違い、空の記憶は全件注入のプロンプトに空行として載るだけで
    /// 何も伝えない。発生源の調査材料は `message.raw_content` に残る。
    #[test]
    fn content_rejects_blank() {
        for raw in BLANKS {
            assert_eq!(
                MemoryContent::parse(raw),
                Err(ContentError::Empty),
                "raw = {raw:?}"
            );
        }
    }

    /// **「広い意味で空白」だけの本文は作れない**（作業単位 0009）。
    ///
    /// 空だけを拒否していると `"\u{2028}"` の記憶が `parse` を通り、
    /// 全件注入のプロンプトに **`> -` だけの行**として載る。
    /// 枠と予算を食って何も伝えないうえ、フェーズ1には削除も上書きも無いので
    /// **そのキャラの以後の全プロンプトに載り続ける。**
    ///
    /// **中間の行区切りは正当**（「2つの街に住んだ」のような複数行の記憶）。
    #[test]
    fn content_rejects_values_that_sanitize_to_nothing() {
        for separator in LINE_BREAKS {
            assert_eq!(
                MemoryContent::parse(&separator.to_string()),
                Err(ContentError::Empty),
                "separator = {separator:?}"
            );
            assert_eq!(
                MemoryContent::parse(&format!("\u{3000}{separator} {separator}\t")),
                Err(ContentError::Empty),
                "空白と混ぜても空: separator = {separator:?}"
            );
            assert!(
                MemoryContent::parse(&format!("東京に住んでいた{separator}今は大阪")).is_ok(),
                "中間の行区切りは正当な複数行の記憶であること: separator = {separator:?}"
            );
        }
        // 制御文字だけの本文も同じ（`SafeText` が空白に潰して行ごと落とす）。
        assert_eq!(
            MemoryContent::parse("\u{0000}\u{0007}"),
            Err(ContentError::Empty)
        );
    }

    /// **見えない文字だけの本文は作れない**（作業単位 0009a）。
    ///
    /// 0009 の条件（無害化すると空になる）では U+2000 / U+200B が素通りし、
    /// **`> - `（本文の見えない行）**が全件注入のプロンプトに永久に載っていた。
    #[test]
    fn content_rejects_invisible_characters() {
        for character in INVISIBLE_SAMPLES {
            assert_eq!(
                MemoryContent::parse(&character.to_string()),
                Err(ContentError::Empty),
                "見えない文字だけの本文が通った: U+{:04X}",
                character as u32
            );
            assert_eq!(
                MemoryContent::parse(&format!("\u{3000}{character}\n{character} ")),
                Err(ContentError::Empty),
                "空白・改行と混ぜても空: U+{:04X}",
                character as u32
            );
            assert!(
                MemoryContent::parse(&format!("東京{character}大阪")).is_ok(),
                "見える文字があるのに拒否した: U+{:04X}",
                character as u32
            );
        }
    }

    /// **上限ちょうどは受理し、1文字超えたら拒否する。**
    ///
    /// 境界を固定しないと「上限がある」ことしか守れず、値の意味がずれても気付けない。
    #[test]
    fn content_accepts_the_limit_and_rejects_one_more() {
        let at_limit = "あ".repeat(MemoryContent::MAX_CHARS);
        assert_eq!(
            MemoryContent::parse(&at_limit).map(|c| c.as_str().chars().count()),
            Ok(MemoryContent::MAX_CHARS),
            "上限ちょうどは受理すること"
        );

        let over_limit = "あ".repeat(MemoryContent::MAX_CHARS + 1);
        assert_eq!(
            MemoryContent::parse(&over_limit),
            Err(ContentError::TooLong),
            "上限を1文字超えたら拒否すること"
        );
    }

    /// **長さは文字数で数える。バイト数ではない。**
    ///
    /// バイト数で数えると日本語の記憶だけが英語の 1/3 しか書けなくなる。
    /// 日本語が主用途なので、ここは必ず `chars().count()`。
    #[test]
    fn content_length_is_counted_in_characters_not_bytes() {
        let japanese = "猫".repeat(MemoryContent::MAX_CHARS);
        assert!(
            japanese.len() > MemoryContent::MAX_CHARS,
            "前提: 日本語はバイト数の方が大きいこと ({} バイト)",
            japanese.len()
        );
        assert!(
            MemoryContent::parse(&japanese).is_ok(),
            "バイト数で弾かないこと"
        );

        // 絵文字（サロゲートペア相当の1スカラー値）も1文字として数える。
        let emoji = "🐈".repeat(MemoryContent::MAX_CHARS);
        assert!(MemoryContent::parse(&emoji).is_ok(), "1スカラー値=1文字");
        assert_eq!(
            MemoryContent::parse(&"🐈".repeat(MemoryContent::MAX_CHARS + 1)),
            Err(ContentError::TooLong)
        );
    }

    /// 長さの判定は**正規化（前後の空白除去）の後**に行う。
    ///
    /// 前に判定すると、空白を足すだけで正当な記憶が弾かれる。
    #[test]
    fn content_length_is_checked_after_trimming() {
        let padded = format!("\u{3000} {} \n", "あ".repeat(MemoryContent::MAX_CHARS));
        assert_eq!(
            MemoryContent::parse(&padded).map(|c| c.as_str().chars().count()),
            Ok(MemoryContent::MAX_CHARS),
            "前後の空白は長さに数えないこと"
        );
    }

    /// **暴走した抽出結果（会話ログ丸ごと）は受け付けない。**
    ///
    /// ADR-0003 は全件注入なので、一度入ると以後の全プロンプトに載り続ける。
    /// フェーズ1には削除も上書きもないため、アプリ経由では二度と取り除けない。
    #[test]
    fn content_rejects_a_runaway_extraction() {
        let conversation_log = "ユーザ: こんにちは\n葵: こんにちは。\n".repeat(10_000);
        assert!(
            conversation_log.chars().count() > 100_000,
            "前提: 十分に長いこと"
        );
        assert_eq!(
            MemoryContent::parse(&conversation_log),
            Err(ContentError::TooLong)
        );
    }

    /// 上限は「数百文字」の範囲に収める（想定から外れたら気付けるようにする）。
    ///
    /// 大きすぎれば暴走を止められず、小さすぎれば正当な記憶を弾く。
    #[test]
    fn content_limit_stays_in_the_intended_range() {
        assert!(
            (200..=1_000).contains(&MemoryContent::MAX_CHARS),
            "上限は数百文字程度であること: {}",
            MemoryContent::MAX_CHARS
        );
    }

    /// `Display` は正規化後の値をそのまま返す。
    #[test]
    fn content_display_matches_as_str() {
        let value = content(" 猫が 好き ");
        assert_eq!(value.to_string(), "猫が 好き");
        assert_eq!(value.to_string(), value.as_str());
    }

    /// 分類も前後の空白を除去する。
    #[test]
    fn category_trims_surrounding_blanks() {
        assert_eq!(category(" 好み ").as_str(), "好み");
        assert_eq!(category("\u{3000}好み\u{3000}").as_str(), "好み");
        assert_eq!(category(" 好み "), category("好み"));
        assert_ne!(category("好み"), category("経歴"));
    }

    /// 空の分類は作れない（プロンプトに見出しのない分類が載るのを防ぐ）。
    #[test]
    fn category_rejects_blank() {
        for raw in BLANKS {
            assert_eq!(
                MemoryCategory::parse(raw),
                Err(CategoryError::Empty),
                "raw = {raw:?}"
            );
        }
    }

    /// **分類も上限ちょうどは受理し、1文字超えたら拒否する。**
    ///
    /// 本文と同じ論法（全件注入 / 削除も上書きも無い）が分類にも当てはまる。
    /// 抽出サブLLM が本文と分類を取り違えて分類欄に会話ログを吐くのは、
    /// 小型モデルでは十分に起こりうる。
    #[test]
    fn category_accepts_the_limit_and_rejects_one_more() {
        let at_limit = "あ".repeat(MemoryCategory::MAX_CHARS);
        assert_eq!(
            MemoryCategory::parse(&at_limit).map(|c| c.as_str().chars().count()),
            Ok(MemoryCategory::MAX_CHARS),
            "上限ちょうどは受理すること"
        );

        let over_limit = "あ".repeat(MemoryCategory::MAX_CHARS + 1);
        assert_eq!(
            MemoryCategory::parse(&over_limit),
            Err(CategoryError::TooLong),
            "上限を1文字超えたら拒否すること"
        );
    }

    /// 分類の長さも**文字数**で数える（バイト数ではない）。
    #[test]
    fn category_length_is_counted_in_characters_not_bytes() {
        let japanese = "好".repeat(MemoryCategory::MAX_CHARS);
        assert!(
            japanese.len() > MemoryCategory::MAX_CHARS,
            "前提: 日本語はバイト数の方が大きいこと ({} バイト)",
            japanese.len()
        );
        assert!(
            MemoryCategory::parse(&japanese).is_ok(),
            "バイト数で弾かないこと"
        );
    }

    /// 分類の長さの判定も**正規化の後**に行う。
    #[test]
    fn category_length_is_checked_after_trimming() {
        let padded = format!("\u{3000} {} ", "あ".repeat(MemoryCategory::MAX_CHARS));
        assert_eq!(
            MemoryCategory::parse(&padded).map(|c| c.as_str().chars().count()),
            Ok(MemoryCategory::MAX_CHARS),
            "前後の空白は長さに数えないこと"
        );
    }

    /// **分類欄に会話ログを吐かれても入口で止まる。**
    ///
    /// 分類は見出しとしてプロンプトに載るため、暴走が1件入ると
    /// 本文と同じく**そのキャラの以後の全プロンプトに永久に載り続ける。**
    #[test]
    fn category_rejects_a_runaway_extraction() {
        // 制御文字（改行）も行区切りも含まない形にして、**長さだけで**弾かれることを確かめる。
        let conversation_log = "ユーザ: こんにちは 葵: こんにちは。 ".repeat(10_000);
        assert!(
            !contains_control_or_line_break(&conversation_log),
            "前提: 制御文字や行区切りでは弾かれないこと"
        );
        assert_eq!(
            MemoryCategory::parse(&conversation_log),
            Err(CategoryError::TooLong)
        );
    }

    /// 分類の上限は本文より小さく、短いラベルとして妥当な範囲に収める。
    ///
    /// **下げる方向の変更はマイグレーションを伴う**（`MAX_CHARS` の doc 参照）ので、
    /// 想定から外れたらここで気付けるようにしておく。
    #[test]
    fn category_limit_stays_in_the_intended_range() {
        assert!(
            (10..=100).contains(&MemoryCategory::MAX_CHARS),
            "上限は短いラベルに見合う長さであること: {}",
            MemoryCategory::MAX_CHARS
        );
        assert!(
            (1..MemoryContent::MAX_CHARS).contains(&MemoryCategory::MAX_CHARS),
            "分類の上限は本文より小さいこと: {} / {}",
            MemoryCategory::MAX_CHARS,
            MemoryContent::MAX_CHARS
        );
    }

    /// **エラーの `Display` に格納値も長さも載せない**（CLAUDE.md 絶対制約）。
    #[test]
    fn category_error_display_is_a_constant_string() {
        let over_limit = "秘密".repeat(MemoryCategory::MAX_CHARS);
        let err = MemoryCategory::parse(&over_limit).expect_err("上限超過であること");
        let message = err.to_string();
        assert!(!message.contains("秘密"), "分類を含めないこと: {message}");
        assert!(
            !message.contains(&MemoryCategory::MAX_CHARS.to_string()),
            "実行時の値を埋め込まない定数文字列であること: {message}"
        );
    }

    /// **分類も行区切りを拒否する**（作業単位 0009）。
    ///
    /// U+2028 / U+2029 は `char::is_control` が偽（Zl / Zp）なので、
    /// 制御文字の検査だけでは素通りする。
    ///
    /// # 守っているのは「**1行のラベルは1行である**」という衛生条件
    ///
    /// **注入防御ではない。** `prompt::writer::quote_line` は `Seg::Text` を
    /// `to_inline()` で1行に畳むので、`"好\u{2028}み"` は `> - [好 み] 猫が好き`
    /// になるだけで偽の役割行にはならない。**この理由を書いておかないと**、後から
    /// 「`to_inline` があるのだから要らない」として外される（`0006` の `Mutex` と同じ形）。
    ///
    /// 「見出しの中身が消えて `> - [ ] 猫が好き` になる」方は
    /// `category_rejects_invisible_characters` の担当。
    ///
    /// **`LINE_BREAKS` の全文字を回す**（固定リストにすると、集合に文字を足した人が
    /// 気付けない）。
    #[test]
    fn category_rejects_every_line_separator() {
        for separator in LINE_BREAKS {
            assert_eq!(
                MemoryCategory::parse(&format!("好{separator}み")),
                Err(CategoryError::Control),
                "separator = {separator:?}"
            );
            assert!(
                MemoryCategory::parse(&separator.to_string()).is_err(),
                "行区切りだけの分類も作れないこと: separator = {separator:?}"
            );
        }
    }

    /// **何も見えない分類は `Control` ではなく `Empty`**（作業単位 0009b）。
    ///
    /// 0009a で `is_effectively_blank` を `contains_control_or_line_break` の**前**に
    /// 置いた結果、入力とエラーの対応が変わっている。**意図した対応**だが、
    /// `category_rejects_every_line_separator` は単独の場合を `is_err()` としか見ておらず、
    /// **順序を戻す変異が緑を通る。** 詳細は
    /// `domain::character::tests::handle_reports_empty_rather_than_control_when_nothing_is_visible`。
    #[test]
    fn category_reports_empty_rather_than_control_when_nothing_is_visible() {
        for raw in ["\u{0007}", "\u{0000}", "\u{007F}", "\u{0007}\u{0000}"] {
            assert_eq!(
                MemoryCategory::parse(raw),
                Err(CategoryError::Empty),
                "何も見えない値は「空」として報告すること: raw = {raw:?}"
            );
        }
        for separator in LINE_BREAKS {
            assert_eq!(
                MemoryCategory::parse(&separator.to_string()),
                Err(CategoryError::Empty),
                "行区切りだけの値も「空」として報告すること: separator = {separator:?}"
            );
        }
        for raw in ["\u{0007}好み", "好み\u{0007}", "好\u{2028}み"] {
            assert_eq!(
                MemoryCategory::parse(raw),
                Err(CategoryError::Control),
                "見える文字があるなら制御文字として報告すること: raw = {raw:?}"
            );
        }
    }

    /// **見えない文字だけの分類は作れない**（作業単位 0009a）。
    ///
    /// 0009 の条件（無害化すると空になる）では U+2000 / U+200B が素通りし、
    /// **見出しの中身が見えない `> - [ ] 猫が好き`** がプロンプトに載っていた。
    #[test]
    fn category_rejects_invisible_characters() {
        for character in INVISIBLE_SAMPLES {
            assert_eq!(
                MemoryCategory::parse(&character.to_string()),
                Err(CategoryError::Empty),
                "見えない文字だけの分類が通った: U+{:04X}",
                character as u32
            );
            assert_eq!(
                MemoryCategory::parse(&format!(" {character}\u{3000}{character}\t")),
                Err(CategoryError::Empty),
                "trim 対象の空白と混ぜても空: U+{:04X}",
                character as u32
            );
            assert!(
                MemoryCategory::parse(&format!("好{character}み")).is_ok(),
                "見える文字があるのに拒否した: U+{:04X}",
                character as u32
            );
        }
    }

    /// 分類は制御文字を拒否する（本文との非対称は意図したもの）。
    #[test]
    fn category_rejects_control_characters() {
        for raw in ["好\u{0007}み", "好み\u{0000}", "a\nb", "a\tb"] {
            assert_eq!(
                MemoryCategory::parse(raw),
                Err(CategoryError::Control),
                "raw = {raw:?}"
            );
            assert!(
                MemoryContent::parse(raw).is_ok(),
                "本文は同じ値を受け入れること: raw = {raw:?}"
            );
        }
    }

    /// `Display` は正規化後の値をそのまま返す。
    #[test]
    fn category_display_matches_as_str() {
        let value = category(" 好み ");
        assert_eq!(value.to_string(), "好み");
        assert_eq!(value.to_string(), value.as_str());
    }

    /// エラーは理由を区別できる。
    #[test]
    fn error_variants_are_distinguishable() {
        assert_ne!(CategoryError::Empty, CategoryError::Control);
        assert_ne!(CategoryError::Empty, CategoryError::TooLong);
        assert_ne!(CategoryError::Control, CategoryError::TooLong);
        assert_ne!(
            CategoryError::Empty.to_string(),
            CategoryError::Control.to_string()
        );
        assert_ne!(
            CategoryError::Control.to_string(),
            CategoryError::TooLong.to_string()
        );
        assert_ne!(ContentError::Empty, ContentError::TooLong);
        assert_ne!(
            ContentError::Empty.to_string(),
            ContentError::TooLong.to_string()
        );
        assert!(!ContentError::Empty.to_string().is_empty());
    }

    /// **エラーの `Display` に格納値も長さも載せない**（CLAUDE.md 絶対制約）。
    ///
    /// `TooLong` は「LLM が吐いた文字列を弾いた」ときに出るので、
    /// うっかり本文の一部を載せるとそのままレスポンスに漏れる。
    #[test]
    fn content_error_display_is_a_constant_string() {
        let over_limit = "秘密".repeat(MemoryContent::MAX_CHARS);
        let err = MemoryContent::parse(&over_limit).expect_err("上限超過であること");
        let message = err.to_string();
        assert!(!message.contains("秘密"), "本文を含めないこと: {message}");
        assert!(
            !message.contains(&MemoryContent::MAX_CHARS.to_string()),
            "実行時の値を埋め込まない定数文字列であること: {message}"
        );
    }

    /// `NewMemory` は主体・本文・分類の組。分類だけが省略できる。
    #[test]
    fn new_memory_requires_a_subject() {
        let memory = NewMemory {
            subject: Subject::Character,
            content: content("紅茶が好き"),
            category: None,
        };
        assert_eq!(memory.subject, Subject::Character);
        assert_eq!(memory.content.as_str(), "紅茶が好き");
        assert_eq!(memory.category, None);
    }

    /// **抽出の主語は2語で、互いに違い、DB の綴りとも違う。**
    ///
    /// 同じ語になれば `parse::extraction` は主体を区別できず、
    /// DB の綴り（`user` / `character`）になればプロンプトに識別子が載る。
    /// `match` で書いてあるので、`Subject` に値が増えたらコンパイルできなくなる。
    #[test]
    fn the_two_extraction_words_are_distinct_and_not_the_database_spelling() {
        assert_ne!(
            Subject::User.extraction_word(),
            Subject::Character.extraction_word()
        );
        for subject in [Subject::User, Subject::Character] {
            let word = subject.extraction_word();
            assert_ne!(word, "user", "DB の綴りを使わないこと");
            assert_ne!(word, "character", "DB の綴りを使わないこと");
            assert!(!word.is_empty());
        }
        // 取り違えを固定する（片方だけ見ても区別が付かない）。
        assert_eq!(Subject::User.extraction_word(), "利用者");
        assert_eq!(Subject::Character.extraction_word(), "あなた");
    }

    /// **1ターンの上限は 1 件以上。** 0 だと抽出が常に全件破棄になる。
    #[test]
    fn the_per_turn_limit_admits_at_least_one_memory() {
        const { assert!(NewMemory::MAX_PER_TURN >= 1) };
    }

    /// **`Subject::ALL` に数え漏れが無い**（作業単位 P-2a —— 仕様書の `enum` の元）。
    ///
    /// 仕掛けは下の `every_memory_column_variant_is_listed_in_all` と同じ（網羅の `match`）。
    #[test]
    fn every_subject_variant_is_listed_in_all() {
        // **ワイルドカードの腕を足さないこと**（足した瞬間に見張り番が死ぬ）。
        let listed = |subject: Subject| match subject {
            Subject::User | Subject::Character => Subject::ALL.contains(&subject),
        };
        // **`ALL` から回さない**（`ALL` から回すと `contains` は必ず真になる）。
        for subject in [Subject::User, Subject::Character] {
            assert!(listed(subject), "{subject:?} が ALL に無い");
        }
        assert_eq!(
            Subject::ALL.len(),
            2,
            "主語を足したなら ALL の要素数も増えていること"
        );
    }

    // ------------------------------------------------------------------
    // 修復のための一覧の型（ADR-0022 §2 / 作業単位 14a）
    // ------------------------------------------------------------------

    /// **`ALL` に数え漏れが無い。**
    ///
    /// 綴りが実在の列名と一致することは
    /// `tests/store_schema.rs::every_pointed_at_column_is_a_real_memory_column` が
    /// `PRAGMA table_info` と突き合わせて見るが、あちらが回るのは `ALL` の中身だけ。
    /// **`ALL` から漏れた variant は、あの突き合わせを素通りする。**
    ///
    /// variant を足すと下の `match` がコンパイルできなくなり、
    /// 足した腕は `ALL` に入っていることを主張するので、`ALL` を直すまで赤のまま。
    #[test]
    fn every_memory_column_variant_is_listed_in_all() {
        // **網羅の `match`。** ワイルドカードの腕を足さないこと（足した瞬間に見張り番が死ぬ）。
        let listed = |column: MemoryColumn| match column {
            MemoryColumn::Subject
            | MemoryColumn::Content
            | MemoryColumn::Category
            | MemoryColumn::Locked => MemoryColumn::ALL.contains(&column),
        };

        for column in MemoryColumn::ALL {
            assert!(listed(column), "{column:?} が ALL に無い");
        }
        assert_eq!(
            MemoryColumn::ALL.len(),
            4,
            "列を足したなら ALL の要素数も増えていること"
        );
    }

    /// 列名は**互いに違う**（1つの綴りが2つの列を指さない）。
    ///
    /// 実在の列名かどうかは統合テスト側（`PRAGMA table_info`）の担当。
    #[test]
    fn memory_column_names_are_distinct() {
        for (index, left) in MemoryColumn::ALL.iter().enumerate() {
            assert!(!left.as_str().is_empty(), "{left:?} の列名が空");
            for right in &MemoryColumn::ALL[index + 1..] {
                assert_ne!(
                    left.as_str(),
                    right.as_str(),
                    "{left:?} と {right:?} が同じ列名を指している"
                );
            }
        }
    }

    /// **`locked` を指し示せる。**
    ///
    /// ADR-0022 §4 は `locked` の**意味**を確定させていないが、
    /// `memory_from_row` は 0 / 1 以外の `locked` を `InvalidStoredValue` で弾く。
    /// **指し示せないと、チャットを止めている当の行が一覧から見えない**
    /// （理由は `MemoryColumn::Locked` の doc）。
    #[test]
    fn the_locked_column_can_be_pointed_at() {
        assert!(
            MemoryColumn::ALL.contains(&MemoryColumn::Locked),
            "locked を指し示せること: {:?}",
            MemoryColumn::ALL
        );
        assert_eq!(MemoryColumn::Locked.as_str(), "locked");
    }
}
