//! **抽出サブLLM の出力の分解**（ADR-0021 / 作業単位 13a）。
//!
//! **I/O を持たない純粋関数のみ**を置く（CLAUDE.md 3節）。入力は抽出サブLLM の生出力
//! （`&str`）、出力は [`NewMemory`] の列と**捨てた分の内訳**。
//! 依頼側の組立は `prompt::extraction`（**互いに依存しない。共有する定義は `domain` にある**）。
//!
//! # 形式（ADR-0021）
//!
//! **1記憶1行。`- 主語: 本文`。** 分類は任意で `- 利用者: [好み] 犬を二匹飼っている`。
//!
//! ```text
//! - 利用者: 犬を二匹飼っている
//! - あなた: 常連客と世間話をするのが好き
//! ```
//!
//! **JSON にしない理由も ADR-0021 にある**——1行崩れても他の行が生き残ることが選定理由で、
//! この分解器はその性質を実装したものにすぎない。
//!
//! # 本文のパーサ（`super`）と決定的に違う点
//!
//! **フォールバック先が無い。** 本文は崩れたら生出力を丸ごと地の文にできる（ADR-0007）が、
//! 抽出で同じことをすると**壊れた文字列が恒久的に `memory` に残る**（削除も上書きも無い）。
//! したがってここでの回復は「**捨てる**」の一手しかなく、
//! 代わりに**捨てた量を理由ごとに数える**（[`DiscardCounts`]）。
//!
//! **捨てたこと自体は失敗ではない**（ADR-0021）。会話に事実が無ければ 0 件が正しい答えで、
//! **破棄率が高い＝壊れている、と読んではいけない。**
//!
//! # `domain` の `parse` で弾かれた分もここで数える
//!
//! [`MemoryContent::parse`] / [`MemoryCategory::parse`] は純粋関数なので `tracing` を持てず、
//! **呼び出し側が数えないと痕跡がまったく残らない**（ADR-0021）。
//! だから本文の空判定も長さ判定も**この層では書かず**、`domain` に投げて
//! 返ってきたエラーを [`DiscardReason`] に写す。
//! （自前で先に弾くと、`domain` の上限が動いたときにここが黙って食い違う。）
//!
//! # Markdown の強調記号は落とす。行ごと捨てない（ADR-0021 §11 / 13c-1）
//!
//! `- あなた: 黒い猫（名前：**コーヒー**）を飼っている` は
//! **`- 主語: 本文` の形を完全に満たす**ので [`DiscardCounts`] には現れず、
//! `memory` に更新も削除も無い以上、そのままだと**恒久的に残る。**
//! 行ごと捨てると**良い事実まで失う**ので、印だけを落とす（[`strip_emphasis`]）。
//!
//! 落とすのは**対になった2文字の印**と、**行の端で宙に浮いた印**の2つだけ
//! （後者の条件は「前後のどちらにも文字が無く、**かつ少なくとも片側が行の端**」——
//! 行の端を求めないと `2 ** 10` が `2  10` になる。[`is_floating`]）。
//!
//! **落とすのは行の解析より前**——[`BULLETS`] に `*` が含まれるので、
//! 箇条書き記号を先に剥がすと `**利用者**: 犬` の頭が1つだけ削れて主語が壊れる。
//! **落とした行数は [`ExtractedMemories::emphasis_stripped`] に数える**
//! （破棄率とは別の数。混ぜると過去の測定値と比較できなくなる）。
//!
//! # 行頭の引用記号は落とす。行ごと捨てない（作業単位 B-29 / C-15）
//!
//! **依頼は信頼できない入力を `> ` で始まる引用行にして見せる**
//! （`prompt::writer::QUOTE_PREFIX`）。**モデルはその書式を答えの側へ持ち込む。**
//!
//! C-15 の実測（2026-08-29 / `scratchpad/agent-abde25c1a7e958507-c15/`）:
//! **Qwen3.5-9B 無検閲版の 272 観測のうち 15（5.5%）が「全行が `> ` で始まる」形**で、
//! **そのターンは記憶が1件も残らなかった**——67 行が主語なし（[`DiscardReason::MissingSubject`]）
//! として捨てられていた。**内容は崩れていない。行頭の2文字だけの問題である。**
//!
//! **指示で直そうとしない。剥がす**（[`strip_quote`]）。13c-1（Markdown の正規化は
//! 決定的に直り、「使うな」という指示は効かなかった）と 16e-c（変容で同じことが起きた）と
//! 同じ判断で、**行頭の記号は決定的に直せる側**である。
//!
//! | 形 | どうなるか |
//! |---|---|
//! | `> - あなた: 朝が弱い` | **剥がす**（C-15 で観測された 67 行はすべてこの形） |
//! | `> あなた: 朝が弱い` | **剥がす**（引用 → 箇条書きの順なので、記号の有無に依らない） |
//! | `＞ あなた: 朝が弱い` | **剥がさない。** 依頼が書くのは半角（[`QUOTE_MARK`]） |
//! | `>> あなた: 朝が弱い` | **1つだけ剥がす。** 入れ子は開かない（残りは主語の一部） |
//! | `- あなた: 矢印を > で書く` | **本文の `>` には触らない。** 剥がすのは行頭だけ |
//!
//! **落とした行数は [`ExtractedMemories::quotes_stripped`] に数える**
//! （[`ExtractedMemories::emphasis_stripped`] と同じで、破棄率とは別の数）。
//!
//! # 打ち切られた応答の最後の1行は捨てる（ADR-0021 §17 / 作業単位 24）
//!
//! **上流が `max_tokens` で打ち切ると、最後の行は断片になる。**
//! 断片が `- 主語: 本文` の形をしていれば**このパーサは何事もなく通す**ので、
//! 「犬を二匹飼っている」が切れた `犬を二匹飼って` が**恒久的な記憶になる。**
//!
//! ADR-0021 §8 は入力側で同じことを禁じている（「断片から抽出しないこと」）。
//! **同じ理屈が出力側にも要る**ので、[`Completeness::Truncated`] を渡された回は
//! **最後の1行だけ**を捨てて [`DiscardReason::Truncated`] に数える。
//! **前の行は完結している**ので落とさない——落とすと、正しい事実を捨てることになる。
//!
//! **打ち切られたかどうかは引数で受ける。** ここは純粋関数なので
//! `finish_reason` を持つ `llm` の型を見に行かない（判定は `server::extract`）。
//!
//! # panic しないこと
//!
//! **絶対制約。** 走査は `split_once` と `chars()` だけで行い、
//! **バイト添字によるスライスを作らない。** 件数の加算も飽和加算にしてある。
//! `unwrap` / `expect` / 添字アクセスも置かない。

use crate::domain::{
    CategoryError, ContentError, MemoryCategory, MemoryContent, NewMemory, Subject, TRIMMABLE,
};

use super::{is_line_separator, trim_blank};

/// 主語と本文の区切り。**半角と全角の両方を受ける。**
///
/// 小型モデルは日本語入力の流れで全角コロンを書く。**寛容に書く**のは
/// 本文のパーサと同じ規約（CLAUDE.md「小型モデルは形式を崩す前提」）。
const SEPARATORS: [char; 2] = [':', '：'];

/// 行頭の箇条書き記号。**あっても無くてもよい。**
///
/// プロンプトは `- ` を示すが、モデルは `*` や `・` に置き換えることがある。
/// **記号の有無で捨てない**——捨てても得るものが無く、破棄率だけが上がる。
const BULLETS: [char; 9] = ['-', '*', '+', '•', '・', '‐', '–', '—', '−'];

/// **行頭の引用記号**（作業単位 16e-c / B-29）。**半角の `>` だけ。**
///
/// 依頼は信頼できない入力を `> ` で始まる引用行にして見せる
/// （`prompt::writer::QUOTE_PREFIX`）。**モデルはその記号を答えの側へ持ち込む。**
///
/// # なぜ全角の `＞` を入れないか
///
/// **書くのではなく写しているから。** 引用記号は文の流れで打つ約物ではなく、
/// **依頼に見えている構造の記号**で、依頼が書くのは半角である。
/// 変容の腕E（16e-c）で持ち込まれた 20 回も、
/// C-15 で記憶抽出に持ち込まれた 67 行も、**全部半角**だった。
/// [`EMPHASIS_MARKS`] と同じ作法で、**観測された形に絞り、観測されたら広げる。**
///
/// 剥がされなかった記号は**主語の一部**になるので、
/// [`DiscardReason::MissingSubject`] で落ちる。**黙って記憶にはならない。**
pub(super) const QUOTE_MARK: char = '>';

/// 分類を囲む括弧。**開きと閉じの対応を取る**（`[好み】` は分類として読まない）。
const CATEGORY_BRACKETS: [(char, char); 3] = [('[', ']'), ('［', '］'), ('【', '】')];

/// 落とす Markdown の強調記号。`(印の文字, 2文字重ねる形か)`（ADR-0021 §11）。
///
/// **2文字の形だけを対象にする。** `*…*` / `_…_`（1文字の形）を入れると、
/// 算術（`2*3*4`）が CommonMark の強調条件を満たしてしまい、
/// しかも `*` は [`BULLETS`] でもある。**実機で観測されたのは `**` の形だけ**なので、
/// そこに絞る。1文字の形が観測されたら広げる。
const EMPHASIS_MARKS: [(char, bool); 3] = [('*', true), ('_', true), ('`', false)];

/// 抽出の結果。**採った記憶と、捨てた分の内訳。**
///
/// **捨てた分は「件数」だけを持ち、捨てた文字列は保持しない。**
/// そのままログへ出せる形にするため（LLM 出力をログに丸ごと載せない）。
/// 調査材料は `message.raw_content` の側に残っている（絶対制約）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtractedMemories {
    memories: Vec<NewMemory>,
    discards: DiscardCounts,
    emphasis_stripped: usize,
    quotes_stripped: usize,
}

impl ExtractedMemories {
    /// 採った記憶。**書かれた順**。件数は [`NewMemory::MAX_PER_TURN`] 以下。
    pub fn memories(&self) -> &[NewMemory] {
        &self.memories
    }

    /// 採った記憶を取り出す（保存側へ渡す用）。
    pub fn into_memories(self) -> Vec<NewMemory> {
        self.memories
    }

    /// 捨てた分の内訳。**呼び出し側が破棄率を出すための値。**
    pub fn discards(&self) -> &DiscardCounts {
        &self.discards
    }

    /// **Markdown の強調記号を落とした行数**（ADR-0021 §11）。
    ///
    /// # [`discards`](Self::discards) と別の数にしてある
    ///
    /// 破棄率は**既に定義された指標**で、過去の測定値と比べるために使っている。
    /// **別の意味の数を混ぜると、以後の比較が全部無効になる。**
    /// そもそも強調記号を落とした行は**捨てていない**（行ごと捨てると良い事実まで失う）。
    ///
    /// # 採否に関係なく数える
    ///
    /// 測っているのは「**モデルが Markdown を吐く頻度**」なので、
    /// その行が最終的に捨てられたかどうかは関係しない。
    /// この数が無いと、プロンプトに「Markdown を使うな」と書いたときに
    /// **効いたかどうかが分からない**——落とした後の `memory` からは、
    /// 元が `**` 付きだったか復元できない。
    pub fn emphasis_stripped(&self) -> usize {
        self.emphasis_stripped
    }

    /// **行頭の引用記号を落とした行数**（[`QUOTE_MARK`] / 作業単位 B-29）。
    ///
    /// # 破棄の内訳とも、強調の数とも、別の数にしてある
    ///
    /// [`emphasis_stripped`](Self::emphasis_stripped) とまったく同じ理由——
    /// **破棄率は過去の測定値と比べるための指標**で、別の意味の数を混ぜると
    /// 以後の比較が全部無効になる。そもそも引用記号を剥がした行は**捨てていない。**
    ///
    /// 強調記号と分けてあるのは、**測っている現象が別**だから。
    /// あちらは「モデルが Markdown を吐く頻度」、こちらは
    /// **「モデルが依頼の引用書式を答えへ持ち込む頻度」**である
    /// （`parse::drift::DriftScoringReport::quotes_stripped` と同じ数の意味）。
    ///
    /// # 採否に関係なく数える
    ///
    /// **その行が最終的に捨てられたかどうかは関係しない。**
    /// この数が無いと、**是正が効いたのか、モデルが書かなくなったのかが区別できない**
    /// ——剥がした後の `memory` からは、元が `> ` 付きだったか復元できない。
    /// **C-15 は 5.5% の観測でこれが起きていた**（そのターンは記憶が1件も残らなかった）。
    pub fn quotes_stripped(&self) -> usize {
        self.quotes_stripped
    }
}

/// 捨てた理由。**1つの数に潰さない**（ADR-0021）。
///
/// 潰すと「モデルが主語を書けない」と「本文が長すぎる」と「饒舌すぎる」が区別できず、
/// **プロンプトを直すべきかモデルを替えるべきかの判断材料にならない。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiscardReason {
    /// 行が `主語: 本文` の形になっていない（区切りが無い）。
    Malformed,
    /// 区切りはあるが、主語が [`Subject::extraction_word`] の2語のどちらでもない。
    ///
    /// **推測しない**（ADR-0021）。取り違えた記憶は恒久的に残り、消せない。
    MissingSubject,
    /// 本文が空だと `domain` が判定した（[`ContentError::Empty`]）。
    EmptyContent,
    /// 本文が長すぎると `domain` が判定した（[`ContentError::TooLong`]）。
    ContentTooLong,
    /// 分類を `domain` が受け付けなかった（[`CategoryError`] のいずれか）。
    ///
    /// **行ごと捨てる。** 分類は任意なので「分類だけ落として本文を採る」道もあるが、
    /// 分類が弾かれる典型は**本文と分類を取り違えて分類欄に会話ログを吐いた**場合で
    /// （`MemoryCategory` の doc）、そのとき残る「本文」は本文ではない。
    /// 恒久的に残る値を作る側に倒さない。
    InvalidCategory,
    /// 1ターンの上限（[`NewMemory::MAX_PER_TURN`]）を超えた。
    OverLimit,
    /// **応答が打ち切られたので、最後の行を断片として捨てた**（ADR-0021 §17 / 作業単位 24）。
    ///
    /// **形式が崩れているとは限らない。** むしろ危険なのは
    /// `- 利用者: 十年前から通って` のように**そのままパースを通ってしまう断片**で、
    /// 通せば**誤った事実が恒久的に残る**（`memory` に更新も削除も無い）。
    /// だから[`Malformed`](Self::Malformed) に混ぜない——
    /// **混ぜると「上限が現実に合っていない」というシグナルが読めなくなる。**
    Truncated,
}

impl DiscardReason {
    /// すべての理由。**集計の走査に使う。**
    pub const ALL: [Self; 7] = [
        Self::Malformed,
        Self::MissingSubject,
        Self::EmptyContent,
        Self::ContentTooLong,
        Self::InvalidCategory,
        Self::OverLimit,
        Self::Truncated,
    ];

    /// 集計キー。**値ごとに異なる定数**で、入力を反射しない（`ParseOutcome::label` と同じ扱い）。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::MissingSubject => "missing_subject",
            Self::EmptyContent => "content_empty",
            Self::ContentTooLong => "content_too_long",
            Self::InvalidCategory => "category_rejected",
            Self::OverLimit => "over_limit",
            Self::Truncated => "truncated",
        }
    }

    /// **`domain` の `parse` が弾いた分か。**
    ///
    /// ADR-0021 が「`domain` の `parse` で弾かれた分もここで数える」と決めた分。
    /// `domain` は `tracing` を持てないので、**この印だけが由来の手掛かり**になる。
    pub const fn from_domain(self) -> bool {
        matches!(
            self,
            Self::EmptyContent | Self::ContentTooLong | Self::InvalidCategory
        )
    }

    /// 集計配列の添字。**値ごとに異なる**（重なると理由が1つに潰れる）。
    const fn index(self) -> usize {
        match self {
            Self::Malformed => 0,
            Self::MissingSubject => 1,
            Self::EmptyContent => 2,
            Self::ContentTooLong => 3,
            Self::InvalidCategory => 4,
            Self::OverLimit => 5,
            Self::Truncated => 6,
        }
    }
}

/// 理由ごとの破棄件数。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscardCounts {
    counts: [usize; DiscardReason::ALL.len()],
}

impl DiscardCounts {
    /// その理由で捨てた件数。
    pub fn get(&self, reason: DiscardReason) -> usize {
        self.counts.get(reason.index()).copied().unwrap_or_default()
    }

    /// 捨てた総数。**破棄率の分子。**
    pub fn total(&self) -> usize {
        self.counts
            .iter()
            .fold(0usize, |sum, count| sum.saturating_add(*count))
    }

    /// 理由と件数の組を全件返す（**0 件の理由も含む**）。ログ出力の走査用。
    pub fn iter(&self) -> impl Iterator<Item = (DiscardReason, usize)> + '_ {
        DiscardReason::ALL
            .into_iter()
            .map(|reason| (reason, self.get(reason)))
    }

    /// 1件数える。**飽和加算**（どんな入力でも panic しない）。
    fn record(&mut self, reason: DiscardReason) {
        if let Some(count) = self.counts.get_mut(reason.index()) {
            *count = count.saturating_add(1);
        }
    }
}

/// **応答が最後まで書かれたか**（ADR-0021 §17 / 作業単位 24）。
///
/// **`bool` にしない。** 呼び出し側で `true` / `false` を取り違えると、
/// **完結した応答の最後の1行が毎回捨てられる**か、
/// **断片が恒久的な記憶になる**かのどちらかになり、**どちらもログの数だけでは気付けない。**
///
/// 判定の入口は上流の `finish_reason` だが、**その型はここでは見ない**
/// （`parse` は I/O を持つ `llm` に依存しない / CLAUDE.md 3節）。
/// 写しを作らせないため、対応付けは `server::extract` の1箇所だけにある。
///
/// # **抽出だけのものではない**（作業単位 B-1）
///
/// 変容（`server::drift`）も同じ型で受け取る。**言っているのは封筒のこと**
/// ——「上流が最後まで書いたか」だけで、**何を捨てるかは呼ぶ側が決める。**
/// 抽出は**最後の1行だけ**を捨て、変容は**丸ごと**捨てる
/// （ADR-0027 §打ち切り）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Completeness {
    /// 上流が最後まで書いた（`stop` / それ以外 / 申告なし）。**1行も捨てない。**
    Complete,
    /// **トークン上限で打ち切られた**（`length`）。
    ///
    /// 抽出にとっては「**最後の1行は断片**」を意味する。
    /// **変容にとっては「その応答は丸ごと信用できない」**——
    /// あちらは 2 行以上を丸ごと捨てる決まりなので、
    /// **何行あったのか分からない回の1行を信じる根拠が無い**（ADR-0027 §打ち切り）。
    Truncated,
}

/// 抽出サブLLM の生出力を記憶の列に分解する。**純粋関数**（I/O も乱数も時刻も見ない）。
///
/// **主語は行が名乗る**（`- 主語: 本文`）。
///
/// > **⚠ 「依頼が主語を決める」経路は B-11 で落とした**（ADR-0037 §決定4）。
/// > A-1 で `AskStyle::Split` を実測して悪化したので（ADR-0034 は**撤回**）、
/// > `SubjectSource` / `parse_extraction_from` / `DiscardReason::SubjectWritten` も
/// > 一緒に落としてある。**入口は1つ。**
///
/// **どんな入力でも panic せず、`Result` も返さない。** 崩れた行は捨てて数える
/// （ADR-0021 / CLAUDE.md「パーサは絶対に panic しない」）。
/// **0 件は正常な結果**であって失敗ではない。
///
/// `completeness` が [`Completeness::Truncated`] の回は、**最後の1行を断片として捨てる**
/// （モジュールの doc）。
pub fn parse_extraction(raw: &str, completeness: Completeness) -> ExtractedMemories {
    let mut memories: Vec<NewMemory> = Vec::new();
    let mut discards = DiscardCounts::default();
    // **採った件数は `u32` で数える。** 上限が `u32` なので、比較に cast を挟まない。
    let mut accepted: u32 = 0;
    // **強調記号を落とした行数**（ADR-0021 §11）。破棄とは別の数。
    let mut emphasis_stripped: usize = 0;
    // **行頭の引用記号を落とした行数**（作業単位 B-29）。これも破棄とは別の数。
    let mut quotes_stripped: usize = 0;

    // 行区切りは `LINE_BREAKS` に一本化する（`super::is_line_separator`）。
    // CRLF は空行を1つ生むが、空行はどのみち読み飛ばす。
    let unified: String = raw
        .chars()
        .map(|character| {
            if is_line_separator(character) {
                '\n'
            } else {
                character
            }
        })
        .collect();

    // **打ち切られた回だけ、最後の1行を断片として捨てる**（ADR-0021 §17 / 作業単位 24）。
    // 落とす行が無い回（1行も書かれていない）は `None` ——**捨てていないので数えない。**
    let fragment = match completeness {
        Completeness::Truncated => last_line_with_content(&unified),
        Completeness::Complete => None,
    };

    for (index, line) in unified.split('\n').enumerate() {
        let line = trim_blank(line);
        // **空行は破棄として数えない。** モデルは行間を空けるし、前後にも空行を置く。
        // 数えると破棄率が本文と無関係に膨らみ、指標として使えなくなる。
        if line.is_empty() {
            continue;
        }

        // **断片は、中身を一切見ずに捨てる**（ADR-0021 §17）。
        //
        // **他のどの判定よりも先。** 断片が `- 主語: 本文` の形を満たしていても
        // （むしろ**それがいちばん危ない**）採らないし、崩れていても `malformed` に
        // 数えない——理由が混ざると「上限が現実に合っていない」というシグナルが読めなくなる。
        // **強調記号の勘定にも入れない**（打ち切りは `**強い**` を `**強い` に切るので、
        // 数えると「モデルが Markdown を吐く頻度」が上限の設定次第で動く）。
        if Some(index) == fragment {
            discards.record(DiscardReason::Truncated);
            continue;
        }

        // **強調記号は、箇条書き記号を剥がすより前に落とす**（ADR-0021 §11）。
        // 後に置くと、`BULLETS` に含まれる `*` が `**利用者**: 犬` の先頭から
        // 1つだけ剥がれて `*利用者**: 犬` になり、そこから強調を落とすと
        // **`利用者*` という壊れた主語**ができる。
        let normalized = strip_emphasis(line);
        if normalized.is_some() {
            // **採否に関係なく数える**（測っているのは「モデルが Markdown を吐く頻度」）。
            // **飽和加算**——どんな入力でも panic しない。
            emphasis_stripped = emphasis_stripped.saturating_add(1);
        }
        let line = normalized.as_deref().unwrap_or(line);

        // **行頭の引用記号を落とす**（作業単位 B-29 / モジュールの doc）。
        // **強調記号の後、箇条書き記号（`parse_line` の `strip_bullet`）の前。**
        // 順序は `parse::drift::parse_score_line` と同じ——
        // 実測された形は `> - 主語: 本文` で、**引用を先に剥がさないと箇条書きが残る。**
        let unquoted = strip_quote(line);
        if unquoted.is_some() {
            // **採否に関係なく数える**（`emphasis_stripped` と同じ理由）。
            quotes_stripped = quotes_stripped.saturating_add(1);
        }
        let line = unquoted.unwrap_or(line);

        match parse_line(line) {
            Ok(memory) => {
                // **上限を超えた分は捨てて数える。** 黙って切り詰めない（ADR-0021）。
                // 判定を**行の検証より後**に置いてあるので、上限に達した後の崩れた行は
                // その行自身の理由で数えられる（「上限超過」に紛れ込ませない）。
                if accepted >= NewMemory::MAX_PER_TURN {
                    discards.record(DiscardReason::OverLimit);
                } else {
                    accepted = accepted.saturating_add(1);
                    memories.push(memory);
                }
            }
            Err(reason) => discards.record(reason),
        }
    }

    ExtractedMemories {
        memories,
        discards,
        emphasis_stripped,
        quotes_stripped,
    }
}

/// **中身のある最後の行の番号**（ADR-0021 §17 / 作業単位 24）。1行も無ければ `None`。
///
/// # 「最後の行」ではない
///
/// 行区切りは `\r` も `\n` も畳んであるので、**CRLF で書く上流の応答は末尾に空行が付く。**
/// 単純に最後の行を取ると、**断片を1行も落とさないまま `truncated=0` と記録する**
/// ——ログからは正常に見えるので、気付く手段がなくなる。
///
/// 空行の判定は本走査と同じ `trim_blank` で行う。**別の判定を書かない**
/// （食い違うと、本走査が読む行と番号がずれる）。
fn last_line_with_content(unified: &str) -> Option<usize> {
    let mut last = None;
    for (index, line) in unified.split('\n').enumerate() {
        if !trim_blank(line).is_empty() {
            last = Some(index);
        }
    }
    last
}

/// 行を走査した結果の1片。**バイト添字を作らずに印を扱うための中間形。**
enum Piece {
    /// 印ではない文字。
    Text(char),
    /// 強調記号1つ。
    Mark {
        /// [`EMPHASIS_MARKS`] での並び順。**印の種類ごとに対を数えるための鍵。**
        kind: usize,
        /// 印の文字（書き戻すときに使う）。
        mark: char,
        /// 印を成す文字数（`**` なら 2、`` ` `` なら 1）。
        width: usize,
    },
}

/// 行から Markdown の強調記号を落とす。**落とすものが無ければ `None`**（ADR-0021 §11）。
///
/// **`Some` / `None` の区別がそのまま件数になる**
/// （[`ExtractedMemories::emphasis_stripped`]）。行ごと捨てないのは、
/// **捨てると良い事実まで失う**から——`memory` に削除は無い。
///
/// # 落とすもの
///
/// * **対になった印**。同じ種類の印が並んでいたら、**前から順に2つずつ対にする。**
///   奇数個なら最後の1つが余る。
/// * **行の端で宙に浮いた印**——前後のどちらにも文字が無く（行の端か空白）、
///   **かつ少なくとも片側が行の端**である印（`- あなた: **` の `**`）。
///   **囲む中身が無い以上、装飾の残骸でしかない。**
///   行の端を求める理由は [`is_floating`] を参照——
///   求めないと `2 ** 10` が `2  10` になる。
///
/// # 落とさないもの
///
/// * **1文字の形**（`*…*` / `_…_`）。[`EMPHASIS_MARKS`] の doc を参照。
/// * **対になっていない印**（`**閉じない` / `強調しない**`）。
///   落とすと、崩れた出力の本文を**こちらの推測で書き換える**ことになる。
/// * **行の途中で空白に挟まれただけの印**（`2 ** 10` の `**`）。**算術を壊さないため。**
/// * **入れ子は追わない。** `***強い***` は `*強い*` になる（ADR-0021 §11 の表）。
///
/// # 中身の判断はしない
///
/// ここでやるのは**印の位置だけで決まる正規化**で、本文が空かどうかも長さも見ない
/// （モジュールの doc「自前で先に弾かない」）。`**` だけの本文は、
/// 落とした結果を **`domain` が [`ContentError::Empty`] として弾く。**
pub(super) fn strip_emphasis(line: &str) -> Option<String> {
    strip_emphasis_counted(line).map(|stripped| stripped.text)
}

/// 落とした結果と、**落とした対の数**。[`strip_emphasis_counted`] の返り値。
pub(super) struct StrippedLine {
    /// 落とした後の行。
    pub(super) text: String,
    /// 落とした**対**の数。**宙に浮いた印は対ではないので数えない**（下記）。
    pub(super) pairs: usize,
}

/// [`strip_emphasis`] と同じ規則で落としたうえで、**落とした対の数**も返す。
///
/// **規則の説明は [`strip_emphasis`] にある。ここに写さない。**
/// 実装をこちらに置いてあるのは、**規則を2つ持たないため**——
/// 抽出（`parse_extraction`）と本文（`parse::parse_response`）が
/// **同じ1つの走査**を通る。
///
/// # 「対の数」と「落とした印の数」は違う
///
/// **宙に浮いた印**（`- あなた: **` の `**`）は対を成していないので、
/// **落としても `pairs` は増えない。** つまり
/// **`Some` を返したのに `pairs` が 0**、という結果がありうる。
///
/// 抽出側（[`ExtractedMemories::emphasis_stripped`]）が数えているのは
/// **落とした行数**で、こちらとは意味が違う。**同じ数ではない。**
/// 本文側が対を数えるのは、**1行に何対あっても1と数えると
/// 「落としたのに数が動かない」が起きる**ため（作業単位 B-46）。
pub(super) fn strip_emphasis_counted(line: &str) -> Option<StrippedLine> {
    // 印を1文字も含まない行が大半。**その場合は走査も割り当てもしない。**
    if !line
        .chars()
        .any(|character| emphasis_kind(character).is_some())
    {
        return None;
    }

    let pieces = split_into_pieces(line);

    // 種類ごとの総数。**対にできるのは偶数個まで。**
    let mut totals = [0_usize; EMPHASIS_MARKS.len()];
    for piece in &pieces {
        if let Piece::Mark { kind, .. } = piece
            && let Some(total) = totals.get_mut(*kind)
        {
            *total = total.saturating_add(1);
        }
    }

    let mut stripped = String::with_capacity(line.len());
    let mut seen = [0_usize; EMPHASIS_MARKS.len()];
    let mut dropped = 0_usize;
    // **対として落とした印の数。** 種類ごとに偶数個ずつ落とすので、常に偶数。
    let mut paired_marks = 0_usize;

    for (index, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Text(character) => stripped.push(*character),
            Piece::Mark { kind, mark, width } => {
                let total = totals.get(*kind).copied().unwrap_or_default();
                let so_far = seen.get(*kind).copied().unwrap_or_default();
                // 前から2つずつ対にする（奇数個なら最後の1つが対から漏れる）。
                let paired = so_far < total.saturating_sub(total % 2);

                if paired || is_floating(&pieces, index) {
                    dropped = dropped.saturating_add(1);
                    if paired {
                        paired_marks = paired_marks.saturating_add(1);
                    }
                } else {
                    for _ in 0..*width {
                        stripped.push(*mark);
                    }
                }

                if let Some(count) = seen.get_mut(*kind) {
                    *count = count.saturating_add(1);
                }
            }
        }
    }

    // **落としたときだけ `Some`。** 件数はこの区別だけを根拠に数える。
    (dropped > 0).then_some(StrippedLine {
        text: stripped,
        // **印2つで1対。** 宙に浮いた印はここに入っていない。
        pairs: paired_marks / 2,
    })
}

/// 行を「印」と「それ以外の文字」に割る。**添字もバイト範囲も作らない。**
///
/// 2文字の形は**同じ文字が2つ続いたときだけ**印になる（`2*3*4` は印ではない）。
/// 先読みは1文字（`peek`）で足りる——[`EMPHASIS_MARKS`] に3文字以上の形は無い。
fn split_into_pieces(line: &str) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut characters = line.chars().peekable();

    while let Some(character) = characters.next() {
        match emphasis_kind(character) {
            Some((kind, doubled)) if !doubled => pieces.push(Piece::Mark {
                kind,
                mark: character,
                width: 1,
            }),
            Some((kind, _)) if characters.peek() == Some(&character) => {
                // 2文字目を食べる（`***` の3つ目は印にならず本文に残る）。
                let _ = characters.next();
                pieces.push(Piece::Mark {
                    kind,
                    mark: character,
                    width: 2,
                });
            }
            _ => pieces.push(Piece::Text(character)),
        }
    }

    pieces
}

/// 印の種類。[`EMPHASIS_MARKS`] の並び順と「2文字重ねる形か」を返す。
fn emphasis_kind(character: char) -> Option<(usize, bool)> {
    EMPHASIS_MARKS
        .iter()
        .enumerate()
        .find(|(_, (mark, _))| *mark == character)
        .map(|(kind, (_, doubled))| (kind, *doubled))
}

/// **宙に浮いた印か**（ADR-0021 §11）。条件は2つとも**位置だけで決まる。**
///
/// 1. **前後のどちらにも文字が無い**（行の端か空白）。
/// 2. **少なくとも片側が行の端である。**
///
/// `- あなた: **` の `**` がこれ。**囲む中身が無いので装飾の残骸**でしかなく、
/// 残すと `**` だけの記憶が恒久的に残る。
/// 片側にでも文字があれば（`**閉じない` / `強調しない**`）**残す。**
///
/// # なぜ「行の端」を求めるか
///
/// 1 だけだと、**空白で囲まれたべき乗が壊れる**（`2 ** 10` → `2  10`）。
/// 1文字の形（`*…*`）を [`EMPHASIS_MARKS`] から外した理由——**算術との衝突**——と
/// **同じ穴を開け直すことになる。** 行の端に接することを求めれば `2 ** 10` は無傷で残り、
/// 落としたいもの（`- あなた: **` は後ろが行端、`** 利用者: 犬` は前が行端）は
/// そのまま落ちる。
fn is_floating(pieces: &[Piece], index: usize) -> bool {
    let before = index
        .checked_sub(1)
        .and_then(|previous| pieces.get(previous));
    let after = pieces.get(index.saturating_add(1));
    // **行の端に接していること。** 行の途中で空白に挟まれただけの印は残す。
    let touches_the_end = before.is_none() || after.is_none();

    touches_the_end && side_is_empty(before) && side_is_empty(after)
}

/// 印の片側に文字が無いか。**行の端（`None`）と空白を同じに扱う。**
fn side_is_empty(piece: Option<&Piece>) -> bool {
    match piece {
        None => true,
        Some(Piece::Text(character)) => TRIMMABLE.contains(character),
        // 隣も印なら「文字がある」側に倒す（対の判定に任せる）。
        Some(Piece::Mark { .. }) => false,
    }
}

/// 1行を記憶に落とす。**捨てるときは理由を返す。**
///
/// **主語は行が名乗る**（`- 主語: 本文`）。読めなければ**推測しない**——
/// 取り違えた記憶は恒久的に残り、消せない（ADR-0021）。
fn parse_line(line: &str) -> Result<NewMemory, DiscardReason> {
    let body = strip_bullet(line);

    let Some((subject_part, rest)) = split_at_separator(body) else {
        return Err(DiscardReason::Malformed);
    };
    let Some(subject) = subject_from(trim_blank(subject_part)) else {
        return Err(DiscardReason::MissingSubject);
    };

    let (category_part, content_part) = split_category(trim_blank(rest));
    let category = match category_part {
        Some(raw) => Some(parse_category(raw)?),
        None => None,
    };
    // **空・長すぎの判定は `domain` に委ねる**（写しを持たない / モジュールの doc）。
    let content = MemoryContent::parse(content_part).map_err(|error| match error {
        ContentError::Empty => DiscardReason::EmptyContent,
        ContentError::TooLong => DiscardReason::ContentTooLong,
    })?;

    Ok(NewMemory {
        subject,
        content,
        category,
    })
}

/// 分類を検証する。**`domain` が拒否した理由は畳んで1つにする。**
///
/// 分類側は空・制御文字・長すぎの3種に分かれるが、
/// **どれも「分類欄が使い物にならない」という同じ現象**で、プロンプト設計の
/// 打ち手も変わらない。本文側（空／長すぎ）を分けているのは、
/// 前者が「モデルが主語だけ書いた」、後者が「会話ログを吐いた」という別の壊れ方だから。
fn parse_category(raw: &str) -> Result<MemoryCategory, DiscardReason> {
    MemoryCategory::parse(raw).map_err(|error| match error {
        CategoryError::Empty | CategoryError::Control | CategoryError::TooLong => {
            DiscardReason::InvalidCategory
        }
    })
}

/// 行頭の引用記号を**1つだけ**落とす。**無ければ `None`**（＝剥がしていない）。
///
/// [`strip_bullet`] と同じ形（行頭の1文字を見て、続く空白ごと落とす）。
/// **入れ子（`>> 利用者: 犬`）は開かない**——剥がすのは1つだけで、
/// 残った `>` は主語の一部になって [`DiscardReason::MissingSubject`] で落ちる。
///
/// **`Some` / `None` の区別がそのまま件数になる**
/// （[`ExtractedMemories::quotes_stripped`] / [`strip_emphasis`] と同じ作法）。
///
/// # **`parse::drift` と共有している**（作業単位 B-29）
///
/// あちらは 16e-c で先に同じものを持っていた。**同じ現象が両方で出た**ので、
/// [`strip_bullet`] / [`strip_emphasis`] と同じくここに1つだけ置く。
/// **写しを作ると、片方の受理集合だけが黙って動く。**
pub(super) fn strip_quote(line: &str) -> Option<&str> {
    let mut characters = line.chars();
    match characters.next() {
        Some(QUOTE_MARK) => Some(trim_blank(characters.as_str())),
        _ => None,
    }
}

/// 行頭の箇条書き記号を1つだけ落とす。**無ければそのまま。**
pub(super) fn strip_bullet(line: &str) -> &str {
    let mut characters = line.chars();
    match characters.next() {
        Some(first) if BULLETS.contains(&first) => trim_blank(characters.as_str()),
        _ => line,
    }
}

/// **最初の**区切りで主語と残りに割る。
///
/// 本文にコロンが含まれることは正当（`予定は10:30`）なので、後ろの区切りは見ない。
/// **バイト添字を作らない**ように `split_once` だけで書く。
fn split_at_separator(body: &str) -> Option<(&str, &str)> {
    SEPARATORS
        .iter()
        .filter_map(|separator| body.split_once(*separator))
        // 前半が短い方＝先に現れた区切り。
        .min_by_key(|(head, _)| head.len())
}

/// 主語の語を [`Subject`] に写す。**語彙は2語だけ**（ADR-0021）。
///
/// **綴りは `domain` の [`Subject::extraction_word`] から引く。**
/// ここに文字列を直書きすると、抽出プロンプトが指示する語との対応が
/// 2箇所に分かれ、片方だけ動いたときに全件破棄になる。
fn subject_from(word: &str) -> Option<Subject> {
    [Subject::User, Subject::Character]
        .into_iter()
        .find(|subject| subject.extraction_word() == word)
}

/// 先頭の `[分類]` を切り出す。**無ければ `None` と元の文字列。**
///
/// 閉じ括弧が無い行は**分類なし**として扱い、括弧ごと本文に残す
/// （どこまでが分類かを推測すると、本文の頭が黙って削れる）。
fn split_category(rest: &str) -> (Option<&str>, &str) {
    let mut characters = rest.chars();
    let Some(open) = characters.next() else {
        return (None, rest);
    };
    let Some(close) = CATEGORY_BRACKETS
        .iter()
        .find(|(opening, _)| *opening == open)
        .map(|(_, closing)| *closing)
    else {
        return (None, rest);
    };

    match characters.as_str().split_once(close) {
        Some((category, content)) => (Some(category), content),
        None => (None, rest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **打ち切られていない応答**として分解する（既定の形）。
    ///
    /// 打ち切りを見る検査だけが [`parse_truncated`] を使う。
    /// **どちらも同じ [`parse_extraction`] を通る**ので、本番の入口が観測から外れない。
    fn parse_complete(raw: &str) -> ExtractedMemories {
        parse_extraction(raw, Completeness::Complete)
    }

    /// **打ち切られた応答**として分解する（ADR-0021 §17）。
    fn parse_truncated(raw: &str) -> ExtractedMemories {
        parse_extraction(raw, Completeness::Truncated)
    }

    /// 期待値の組み立て。**テストの中でしか使わない**（本番に `expect` を置かない）。
    fn memory(subject: Subject, content: &str, category: Option<&str>) -> NewMemory {
        NewMemory {
            subject,
            content: MemoryContent::parse(content).expect("正当な本文であること"),
            category: category.map(|raw| MemoryCategory::parse(raw).expect("正当な分類であること")),
        }
    }

    /// 上限を1文字超える本文。
    fn over_long_content() -> String {
        "あ".repeat(MemoryContent::MAX_CHARS + 1)
    }

    /// 上限を1文字超える分類。
    fn over_long_category() -> String {
        "い".repeat(MemoryCategory::MAX_CHARS + 1)
    }

    /// 破棄の内訳が、指定した理由だけを持つこと。
    fn assert_only(counts: &DiscardCounts, expected: &[(DiscardReason, usize)]) {
        for reason in DiscardReason::ALL {
            let want = expected
                .iter()
                .find(|(candidate, _)| *candidate == reason)
                .map_or(0, |(_, count)| *count);
            assert_eq!(
                counts.get(reason),
                want,
                "理由 {} の件数が違う: {counts:?}",
                reason.label()
            );
        }
        let total: usize = expected.iter().map(|(_, count)| *count).sum();
        assert_eq!(counts.total(), total, "{counts:?}");
    }

    // ------------------------------------------------------------------
    // 正常系
    // ------------------------------------------------------------------

    /// **分類ありの行も、分類なしの行も、そのまま記憶になる。**
    ///
    /// 主語・分類の有無・本文を**すべて別の値**にしてある
    /// （どれか2つを同値にすると、その取り違えが緑になる）。
    /// 並びも `あなた` から始めて交互にしてあるので、
    /// **先頭・中間・末尾のどれを取り違えても `assert_eq!` が落ちる。**
    #[test]
    fn lines_with_and_without_a_category_both_become_memories() {
        let raw = "\
- あなた: 常連客と世間話をするのが好き
- 利用者: 犬を二匹飼っている
- あなた: [好み] 苦い珈琲が好き
- 利用者: [経歴] 大阪で生まれ育った";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::Character, "常連客と世間話をするのが好き", None),
                memory(Subject::User, "犬を二匹飼っている", None),
                memory(Subject::Character, "苦い珈琲が好き", Some("好み")),
                memory(Subject::User, "大阪で生まれ育った", Some("経歴")),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
    }

    /// **2語がそれぞれの主体に対応する。両方を含む出力で測る。**
    ///
    /// 片方だけの出力では、対応を入れ替える取り違えを検出できない。
    #[test]
    fn the_two_subject_words_land_on_their_own_side() {
        let raw = "- 利用者: 犬を二匹飼っている\n- あなた: 常連客と世間話をするのが好き";
        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::User, "犬を二匹飼っている", None),
                memory(Subject::Character, "常連客と世間話をするのが好き", None),
            ]
            .as_slice()
        );
    }

    /// **箇条書き記号は無くても、種類が違ってもよい。区切りは全角でもよい。**
    ///
    /// 小型モデルは形式を崩す前提（CLAUDE.md）。記号の違いで捨てても得るものが無い。
    #[test]
    fn bullets_are_optional_and_the_separator_may_be_full_width() {
        let raw = "\
利用者: 犬を二匹飼っている
* あなた: 常連客と世間話をするのが好き
・利用者：大阪で生まれ育った
- あなた：[好み] 苦い珈琲が好き";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::User, "犬を二匹飼っている", None),
                memory(Subject::Character, "常連客と世間話をするのが好き", None),
                memory(Subject::User, "大阪で生まれ育った", None),
                memory(Subject::Character, "苦い珈琲が好き", Some("好み")),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
    }

    /// **共有集合 `BULLETS` の受理集合を、`parse::extraction` の側から押さえる。**
    ///
    /// # なぜここに要るか（16e-c のレビューの実測 / 作業単位 20a）
    ///
    /// [`BULLETS`] は **`parse::extraction` と `parse::drift` の共有**である。
    /// **共有集合に `'>'` を足す変異で赤くなったのは `parse::drift` に足した2本だけ**だった
    /// ——つまり「共有だから触らない」という判断を、**あちらのテストが裏から支えていた。**
    /// **`parse::drift` を消したら、共有集合は誰にも守られなくなる。**
    /// この見張り番は**あちらに一切寄りかからない**（同じ判定を抽出側の入口で見る）。
    ///
    /// # 綴りを [`BULLETS`] から引かない
    ///
    /// `for bullet in BULLETS` と書くと、**要素を落とす変異が期待値を道連れにして緑で通る**
    /// （CLAUDE.md「関数の外にある値で押さえる」）。**9つの綴りをここに直書きする。**
    ///
    /// # 両方向を見る（片方向では「増える方向」しか見ていない）
    ///
    /// | 変異 | ここで赤になる理由 |
    /// |---|---|
    /// | **要素を1つ落とす** | その記号の行の主語が `+ 利用者` のままになり、[`DiscardReason::MissingSubject`] へ落ちる |
    ///
    /// > **⚠ `'>'` を足す変異は、ここでは赤にならなくなった**（作業単位 B-29 / 2026-09-02）。
    /// > **`> 利用者: …` は [`strip_quote`] が剥がして採るのが正しい振る舞いになった**ので、
    /// > **「剥がさない側」に置くと、この検査自身が仕様違反を期待値にする。**
    /// > **共有集合への `'>'` の追加を赤にするのは
    /// > [`a_nested_quote_is_not_opened`]**——足すと `>>` が
    /// > 「引用1つ ＋ 箇条書き1つ」として開いてしまう。**あちらを消さないこと。**
    ///
    /// **落とさない側には隣の綴りも入れてある**——`‒`（U+2012）と `―`（U+2015）は
    /// 集合に入っている4本のダッシュ（`‐` `–` `—` `−`）と見た目が近く、
    /// **足しても誰も気付かない形**だから。
    #[test]
    fn the_shared_bullet_set_is_exactly_these_nine_marks() {
        // **剥がす側。** `BULLETS` の9つを直書きする（定数から引かない）。
        for bullet in [
            '-',        // U+002D HYPHEN-MINUS
            '*',        // U+002A ASTERISK
            '+',        // U+002B PLUS SIGN
            '\u{2022}', // •  BULLET
            '\u{30FB}', // ・ KATAKANA MIDDLE DOT
            '\u{2010}', // ‐  HYPHEN
            '\u{2013}', // –  EN DASH
            '\u{2014}', // —  EM DASH
            '\u{2212}', // −  MINUS SIGN
        ] {
            let raw = format!("{bullet} 利用者: 犬を二匹飼っている");
            let parsed = parse_complete(&raw);

            assert_eq!(
                parsed.memories(),
                [memory(Subject::User, "犬を二匹飼っている", None)].as_slice(),
                "行頭の {bullet:?} を箇条書き記号として剥がしていない: {raw:?}"
            );
            assert_only(parsed.discards(), &[]);
        }

        // **剥がさない側。** ここに増えた記号は、**測っていない側の受理集合を黙って広げる。**
        for other in [
            '\u{FF1E}', // ＞ 全角の引用記号（半角は `strip_quote` が別に剥がす / B-29）
            '\u{00BB}', // »  右二重山括弧
            '#',        // U+0023 見出し記号
            '\u{2012}', // ‒  FIGURE DASH（集合のダッシュ4本とは別物）
            '\u{2015}', // ―  HORIZONTAL BAR（同上）
        ] {
            let raw = format!("{other} 利用者: 犬を二匹飼っている");
            let parsed = parse_complete(&raw);

            assert!(
                parsed.memories().is_empty(),
                "共有の箇条書き記号に {other:?} を足した: {raw:?}"
            );
            assert_only(parsed.discards(), &[(DiscardReason::MissingSubject, 1)]);
        }
    }

    /// **主語と区切りの間に空白があってもよい。**
    ///
    /// 小型モデルが区切りの前に空白を打つのは、全角コロンを打つのと同程度に
    /// ありうる崩れ方。**この寛容さは実在するのに、誰も観測していなかった**——
    /// 箇条書き記号の直後の空白しか無いフィクスチャでは `strip_bullet` 側の
    /// trim が肩代わりし、主語側の trim を外す変異が全件緑で通った
    /// （実測 1003 passed / 2026-08-21）。
    ///
    /// **記号の無い行を混ぜてある**——`strip_bullet` を一切通らない経路でも
    /// 空白入りの主語が読めることを、肩代わりなしで固定するため。
    #[test]
    fn whitespace_between_the_subject_and_the_separator_is_tolerated() {
        let raw = "\
- 利用者 : 犬を二匹飼っている
あなた　： [好み] 苦い珈琲が好き
・利用者\u{3000}：大阪で生まれ育った";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::User, "犬を二匹飼っている", None),
                memory(Subject::Character, "苦い珈琲が好き", Some("好み")),
                memory(Subject::User, "大阪で生まれ育った", None),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
    }

    /// **箇条書き記号と本文の間の空白も、記号と一緒に落とす。**
    ///
    /// `parse_line` から見ると、この trim は主語側の trim
    /// （`subject_from(trim_blank(subject_part))`）と**互いをマスクする**——
    /// 落とし損ねた空白は主語側で必ず落ちるので、
    /// **`parse_extraction` の出力ではどんな入力でも差が出ない**（実測 / 2026-08-21）。
    /// だからこの関数を直接見る。**片方を消しても、もう片方が黙って肩代わりする**
    /// 関係を、ここで固定しておく。
    #[test]
    fn strip_bullet_removes_the_space_that_follows_the_mark() {
        assert_eq!(
            strip_bullet("- 利用者: 犬を二匹飼っている"),
            "利用者: 犬を二匹飼っている"
        );
        assert_eq!(
            strip_bullet("・\u{3000}あなた: 朝が弱い"),
            "あなた: 朝が弱い"
        );
        // 記号が無ければ何も落とさない（先頭の空白も含めてそのまま）。
        assert_eq!(
            strip_bullet("利用者: 犬を二匹飼っている"),
            "利用者: 犬を二匹飼っている"
        );
        // 記号は1つだけ落とす（2つ目は本文の一部として残る）。
        assert_eq!(strip_bullet("-- 利用者: 犬"), "- 利用者: 犬");
    }

    /// **本文にコロンが入っていても切れない**（区切りは最初の1つだけ）。
    #[test]
    fn a_body_may_contain_the_separator() {
        let parsed = parse_complete("- 利用者: 起きるのは毎朝6:30ごろ");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "起きるのは毎朝6:30ごろ", None)].as_slice()
        );
    }

    /// **主語を切るのは「最初に現れた区切り」。半角か全角かで順が決まるのではない。**
    ///
    /// **両方向で測る**——半角と全角の**両方**を含む行を、
    /// 先に来るのが全角の場合と半角の場合の2通り作る。
    /// 片方だけだと、候補が1つしか無い行ばかりになり
    /// 「どちらを選ぶか」を決める部分が観測されない
    /// （実測: この2本を足す前は、最初ではなく**最後**の区切りを採る変異が
    /// スイート全体を素通りした / 2026-08-21）。
    #[test]
    fn the_first_separator_wins_whichever_kind_it_is() {
        // 全角が先、半角が後ろ。
        let parsed = parse_complete("- 利用者：予定は10:30から");
        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "予定は10:30から", None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);

        // 半角が先、全角が後ろ。
        let parsed = parse_complete("- あなた: 開店は9：00から");
        assert_eq!(
            parsed.memories(),
            [memory(Subject::Character, "開店は9：00から", None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
    }

    /// **空行は破棄として数えない。** 数えると破棄率が本文と無関係に膨らむ。
    #[test]
    fn blank_lines_are_never_counted_as_discards() {
        let parsed = parse_complete("\n\n- 利用者: 犬を二匹飼っている\n\n\u{3000}\n\n");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "犬を二匹飼っている", None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
    }

    /// **0 件は正常な結果**（ADR-0021）。失敗ではないので破棄も数えない。
    #[test]
    fn an_empty_output_is_zero_memories_and_zero_discards() {
        for raw in ["", "   ", "\n\n\n", "\u{3000}\u{00A0}"] {
            let parsed = parse_complete(raw);
            assert!(parsed.memories().is_empty(), "{raw:?}");
            assert_only(parsed.discards(), &[]);
        }
    }

    // ------------------------------------------------------------------
    // 主語
    // ------------------------------------------------------------------

    /// **主語が語彙に無い行は捨てて数える。推測しない**（ADR-0021）。
    #[test]
    fn a_line_without_a_known_subject_is_dropped_and_counted() {
        let raw = "\
- 誰か: 犬を二匹飼っている
- : 常連客と世間話をするのが好き
- 利用者: 大阪で生まれ育った";

        let parsed = parse_complete(raw);

        // **通ったのは主語のある1件だけ**（捨てた行が `利用者` として紛れ込んでいない）。
        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "大阪で生まれ育った", None)].as_slice()
        );
        assert_only(parsed.discards(), &[(DiscardReason::MissingSubject, 2)]);
    }

    /// **語彙は2語ちょうど。似た語は通さない。**
    ///
    /// 「ユーザー」「キャラクター」を通すと、抽出プロンプトが指示していない語が
    /// 主語として定着し、以後どちらの語で書かれるかがモデル任せになる。
    #[test]
    fn only_the_two_words_are_accepted_as_subjects() {
        for word in [
            "ユーザー",
            "キャラクター",
            "あなた自身",
            "利用者さん",
            "user",
            "character",
            "葵",
        ] {
            let raw = format!("- {word}: 犬を二匹飼っている");
            let parsed = parse_complete(&raw);
            assert!(parsed.memories().is_empty(), "{raw} が通った");
            assert_only(parsed.discards(), &[(DiscardReason::MissingSubject, 1)]);
        }
    }

    /// **区切りが無い行は「形式が違う」として数える。**
    ///
    /// 主語が無い行（`MissingSubject`）と分けてあるのは、
    /// **前者は「主語だけ書けなかった」、後者は「形式ごと崩れた」**という別の壊れ方で、
    /// プロンプトの打ち手が変わるため。
    #[test]
    fn a_line_without_a_separator_is_counted_as_malformed() {
        let raw = "\
以下の事実を抽出しました
- 犬を二匹飼っている
### 記憶
- 利用者: 大阪で生まれ育った";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "大阪で生まれ育った", None)].as_slice()
        );
        assert_only(parsed.discards(), &[(DiscardReason::Malformed, 3)]);
    }

    // ------------------------------------------------------------------
    // `domain` が弾いた分
    // ------------------------------------------------------------------

    /// **本文が空の行は `domain` 由来の理由として数える。**
    #[test]
    fn an_empty_body_is_counted_as_a_domain_rejection() {
        let raw = "- あなた:\n- 利用者: \u{3000}\n- 利用者: 犬を二匹飼っている";
        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "犬を二匹飼っている", None)].as_slice()
        );
        assert_only(parsed.discards(), &[(DiscardReason::EmptyContent, 2)]);
        assert!(DiscardReason::EmptyContent.from_domain());
    }

    /// **本文が上限を超える行は `domain` 由来の理由として数える。**
    ///
    /// 空とは別の数にしてある——空は「主語だけ書けた」、
    /// 長すぎは「会話ログを吐いた」で、壊れ方も打ち手も違う。
    #[test]
    fn a_too_long_body_is_counted_as_a_domain_rejection() {
        let raw = format!(
            "- あなた: {}\n- 利用者: 犬を二匹飼っている",
            over_long_content()
        );
        let parsed = parse_complete(&raw);

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "犬を二匹飼っている", None)].as_slice()
        );
        assert_only(parsed.discards(), &[(DiscardReason::ContentTooLong, 1)]);
        assert!(DiscardReason::ContentTooLong.from_domain());
    }

    /// **上限ちょうどの本文は通る**（境界の向きを固定する）。
    #[test]
    fn a_body_at_the_limit_still_becomes_a_memory() {
        let body = "あ".repeat(MemoryContent::MAX_CHARS);
        let parsed = parse_complete(&format!("- 利用者: {body}"));

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, &body, None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
    }

    /// **分類を `domain` が拒否したら行ごと捨てて数える。**
    #[test]
    fn a_rejected_category_is_counted_as_a_domain_rejection() {
        let raw = format!(
            "- あなた: [{}] 苦い珈琲が好き\n- 利用者: 犬を二匹飼っている",
            over_long_category()
        );
        let parsed = parse_complete(&raw);

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "犬を二匹飼っている", None)].as_slice()
        );
        assert_only(parsed.discards(), &[(DiscardReason::InvalidCategory, 1)]);
        assert!(DiscardReason::InvalidCategory.from_domain());
    }

    /// **閉じ括弧が無ければ分類として読まない。** 本文の頭を黙って削らない。
    #[test]
    fn an_unclosed_bracket_stays_in_the_body() {
        let parsed = parse_complete("- 利用者: [好み 犬を二匹飼っている");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "[好み 犬を二匹飼っている", None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
    }

    /// **開きと閉じが対応しない括弧も分類として読まない。**
    #[test]
    fn mismatched_brackets_are_not_a_category() {
        let parsed = parse_complete("- 利用者: [好み】犬を二匹飼っている");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "[好み】犬を二匹飼っている", None)].as_slice()
        );
    }

    /// **全角・隅付きの括弧も分類として読む。**
    #[test]
    fn full_width_brackets_are_read_as_a_category() {
        let raw = "- 利用者: ［経歴］大阪で生まれ育った\n- あなた: 【好み】苦い珈琲が好き";
        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::User, "大阪で生まれ育った", Some("経歴")),
                memory(Subject::Character, "苦い珈琲が好き", Some("好み")),
            ]
            .as_slice()
        );
    }

    // ------------------------------------------------------------------
    // 件数の上限
    // ------------------------------------------------------------------

    /// **上限で止まり、超えた分は捨てて数える。切り詰めて黙らない**（ADR-0021）。
    ///
    /// 行数（8）は上限（5）と**別の値**にしてある。
    /// 本文はすべて違う文字列なので、**どれが残ったか**まで固定される。
    #[test]
    fn memories_past_the_limit_are_dropped_and_counted() {
        let raw = "\
- 利用者: 犬を二匹飼っている
- あなた: 常連客と世間話をするのが好き
- 利用者: [好み] 苦い珈琲が苦手
- あなた: [経歴] 三年前に店を継いだ
- 利用者: 弟が名古屋に住んでいる
- あなた: 朝が弱い
- 利用者: [予定] 来週から出張する
- あなた: 猫を飼ってみたい";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::User, "犬を二匹飼っている", None),
                memory(Subject::Character, "常連客と世間話をするのが好き", None),
                memory(Subject::User, "苦い珈琲が苦手", Some("好み")),
                memory(Subject::Character, "三年前に店を継いだ", Some("経歴")),
                memory(Subject::User, "弟が名古屋に住んでいる", None),
            ]
            .as_slice()
        );
        // **捨てた3件が数に残っている**——これが「切り詰め」との唯一の違い。
        // `Vec::truncate` に置き換えると、記憶の側は同じでこの数だけが 0 になる。
        assert_only(parsed.discards(), &[(DiscardReason::OverLimit, 3)]);
        assert_eq!(
            u32::try_from(parsed.memories().len()),
            Ok(NewMemory::MAX_PER_TURN)
        );
    }

    /// **上限ちょうどなら1件も捨てない**（境界の向きを固定する）。
    #[test]
    fn exactly_the_limit_drops_nothing() {
        let raw = "\
- 利用者: 犬を二匹飼っている
- あなた: 常連客と世間話をするのが好き
- 利用者: [好み] 苦い珈琲が苦手
- あなた: [経歴] 三年前に店を継いだ
- 利用者: 弟が名古屋に住んでいる";

        let parsed = parse_complete(raw);

        assert_eq!(
            u32::try_from(parsed.memories().len()),
            Ok(NewMemory::MAX_PER_TURN)
        );
        assert_only(parsed.discards(), &[]);
    }

    /// **上限に達した後の崩れた行は、その行自身の理由で数える。**
    ///
    /// 「上限超過」に紛れ込ませると、
    /// **モデルが饒舌なのか形式を崩したのか**が区別できなくなる。
    #[test]
    fn a_broken_line_after_the_limit_keeps_its_own_reason() {
        let raw = "\
- 利用者: 犬を二匹飼っている
- あなた: 常連客と世間話をするのが好き
- 利用者: [好み] 苦い珈琲が苦手
- あなた: [経歴] 三年前に店を継いだ
- 利用者: 弟が名古屋に住んでいる
以上です
- あなた: 朝が弱い";

        let parsed = parse_complete(raw);

        assert_only(
            parsed.discards(),
            &[(DiscardReason::Malformed, 1), (DiscardReason::OverLimit, 1)],
        );
    }

    // ------------------------------------------------------------------
    // 破棄の内訳
    // ------------------------------------------------------------------

    /// **理由は1つの数に潰れていない。** 6種を1回の出力に混ぜて、各1件を数える。
    ///
    /// 添字が重なる（＝理由が潰れる）と、どれかが 2 になり別のどれかが 0 になる。
    #[test]
    fn one_output_carries_every_discard_reason_separately() {
        let raw = format!(
            "\
- 利用者: 犬を二匹飼っている
以下が抽出結果です
- 誰か: 猫が好き
- あなた:
- 利用者: [{}] 短い本文
- あなた: {}
- 利用者: 弟が名古屋に住んでいる
- あなた: 朝が弱い
- 利用者: [経歴] 大阪で生まれ育った
- あなた: [好み] 苦い珈琲が好き
- 利用者: 来週から出張する",
            over_long_category(),
            over_long_content()
        );

        let parsed = parse_complete(&raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::User, "犬を二匹飼っている", None),
                memory(Subject::User, "弟が名古屋に住んでいる", None),
                memory(Subject::Character, "朝が弱い", None),
                memory(Subject::User, "大阪で生まれ育った", Some("経歴")),
                memory(Subject::Character, "苦い珈琲が好き", Some("好み")),
            ]
            .as_slice()
        );
        assert_only(
            parsed.discards(),
            &[
                (DiscardReason::Malformed, 1),
                (DiscardReason::MissingSubject, 1),
                (DiscardReason::EmptyContent, 1),
                (DiscardReason::ContentTooLong, 1),
                (DiscardReason::InvalidCategory, 1),
                (DiscardReason::OverLimit, 1),
            ],
        );
    }

    /// **理由ごとに独立した数を持つ**（集計配列の添字が重なっていない）。
    #[test]
    fn every_reason_is_counted_on_its_own() {
        for reason in DiscardReason::ALL {
            let mut counts = DiscardCounts::default();
            counts.record(reason);

            for other in DiscardReason::ALL {
                let expected = usize::from(other == reason);
                assert_eq!(
                    counts.get(other),
                    expected,
                    "{} を数えたのに {} が動いた",
                    reason.label(),
                    other.label()
                );
            }
            assert_eq!(counts.total(), 1);
        }
    }

    /// **集計キーは値ごとに異なる。** 同じになると内訳がログで潰れる。
    #[test]
    fn labels_are_distinct() {
        let mut labels: Vec<&str> = DiscardReason::ALL
            .iter()
            .map(|reason| reason.label())
            .collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), DiscardReason::ALL.len());
    }

    /// **`domain` 由来の印が、実際に `domain` の `parse` が弾く3種だけに付く。**
    ///
    /// 印が全件に付く／1件も付かないと、
    /// 「純粋関数が黙って弾いた分」を後から数えられなくなる（ADR-0021）。
    #[test]
    fn only_the_domain_rejections_are_marked_as_such() {
        for reason in DiscardReason::ALL {
            let expected = matches!(
                reason,
                DiscardReason::EmptyContent
                    | DiscardReason::ContentTooLong
                    | DiscardReason::InvalidCategory
            );
            assert_eq!(reason.from_domain(), expected, "{}", reason.label());
        }
        assert_eq!(
            DiscardReason::ALL
                .iter()
                .filter(|reason| reason.from_domain())
                .count(),
            3
        );
    }

    /// **`ALL` はすべての値をちょうど1回ずつ並べ、集計キーの綴りも固定する。**
    ///
    /// `match` で書いてあるので、値が増えたら**このテストがコンパイルできなくなる**
    /// （`ALL` に足し忘れたまま集計から漏れる経路を塞ぐ）。
    #[test]
    fn all_lists_every_reason_exactly_once() {
        for reason in DiscardReason::ALL {
            let expected = match reason {
                DiscardReason::Malformed => "malformed",
                DiscardReason::MissingSubject => "missing_subject",
                DiscardReason::EmptyContent => "content_empty",
                DiscardReason::ContentTooLong => "content_too_long",
                DiscardReason::InvalidCategory => "category_rejected",
                DiscardReason::OverLimit => "over_limit",
                DiscardReason::Truncated => "truncated",
            };
            assert_eq!(reason.label(), expected);
            assert_eq!(
                DiscardReason::ALL
                    .iter()
                    .filter(|candidate| **candidate == reason)
                    .count(),
                1
            );
        }
        assert_eq!(DiscardReason::ALL.len(), 7);
    }

    /// **走査は理由を1つも飛ばさない**（0 件の理由も出す）。
    #[test]
    fn iterating_visits_every_reason() {
        let parsed = parse_complete("- 誰か: 犬が好き");
        let visited: Vec<(DiscardReason, usize)> = parsed.discards().iter().collect();

        assert_eq!(visited.len(), DiscardReason::ALL.len());
        for reason in DiscardReason::ALL {
            let found = visited
                .iter()
                .find(|(candidate, _)| *candidate == reason)
                .map(|(_, count)| *count);
            assert_eq!(found, Some(parsed.discards().get(reason)));
        }
    }

    // ------------------------------------------------------------------
    // 崩れた入力
    // ------------------------------------------------------------------

    /// **どんな入力でも panic しない**（絶対制約）。
    ///
    /// 空文字・記号だけ・巨大な入力・行区切りの混在。
    /// マルチバイト文字の境目でバイト添字を取ると、ここが panic になる。
    ///
    /// # **引用記号の巨大な連なりを、この一覧に置いておくこと**（作業単位 B-29）
    ///
    /// [`strip_quote`] は 16e-c で `parse::drift` に生まれ、B-29 でここへ移した。
    /// **移設の時点で、panic しない保証だけが `parse::drift` に残っていた**
    /// （`the_quote_stripper_never_panics_on_hostile_input` の `">".repeat(100_000)`）。
    /// **`20a` のレビューが指摘した「あちらに寄りかかる」形**——
    /// 次元が受理集合から panic 安全性へ移っただけである。
    ///
    /// **実際に踏んだ**: B-29 の変異検査で「入れ子を再帰で開く」変異を当てたとき、
    /// **lib のテストバイナリごと `STATUS_STACK_OVERFLOW` で死んだ。**
    /// **捕まえたのは移設先ではなく移設元だった。**
    /// **`>` だけの巨大な行・`> ` の繰り返し・先頭が `>` の巨大な行**を、
    /// 抽出側の入口（[`parse_extraction`]）でも通す。
    #[test]
    fn broken_output_never_panics() {
        let huge_line = format!("- 利用者: {}", "犬".repeat(200_000));
        let many_lines = "- あなた: 猫\n".repeat(20_000);
        // **行頭の引用記号**（作業単位 B-29）。剥がすのは1つだけだが、
        // **入れ子を開く実装に変えた瞬間にここが落ちる**（実測: 再帰版は stack overflow）。
        let quote_run = ">".repeat(100_000);
        let quote_pairs = "> ".repeat(100_000);
        let quoted_huge_line = format!("> - 利用者: {}", "犬".repeat(200_000));
        let quoted_lines = "> - あなた: 猫\n".repeat(20_000);
        let inputs = [
            String::new(),
            "   ".to_owned(),
            ":".to_owned(),
            "：".to_owned(),
            "-".to_owned(),
            "- ".to_owned(),
            "[]".to_owned(),
            "- :".to_owned(),
            "- 利用者".to_owned(),
            "- 利用者:".to_owned(),
            "- 利用者:[".to_owned(),
            "- 利用者:[]".to_owned(),
            "- 利用者:[]".to_owned(),
            "利用者：：：".to_owned(),
            "「」『』【】［］".to_owned(),
            "\u{0000}\u{001F}\u{007F}".to_owned(),
            "🐕🐕🐕".to_owned(),
            "- 利用者: 🐕".to_owned(),
            "- あ🐕: 犬".to_owned(),
            "\u{FEFF}- 利用者: 犬を二匹飼っている".to_owned(),
            // Markdown の強調記号（ADR-0021 §11）。**印だけ・奇数個・多バイトとの境目。**
            "**".to_owned(),
            "***".to_owned(),
            "`".to_owned(),
            "``".to_owned(),
            "____".to_owned(),
            "- 利用者: **🐕**".to_owned(),
            "- 利用者: `🐕".to_owned(),
            "- 利用者: **🐕".to_owned(),
            "**".repeat(50_000),
            huge_line,
            many_lines,
            // 行頭の引用記号（作業単位 B-29）。**記号だけ・入れ子・巨大な連なり。**
            ">".to_owned(),
            "> ".to_owned(),
            ">>".to_owned(),
            ">>>>".to_owned(),
            "> - ".to_owned(),
            "> :".to_owned(),
            "> **".to_owned(),
            "> 利用者:".to_owned(),
            "> - 利用者: 🐕".to_owned(),
            "＞>＞>".to_owned(),
            quote_run,
            quote_pairs,
            quoted_huge_line,
            quoted_lines,
            // 行区切りの混在（CRLF / NEL / LS / PS / VT / FF）。
            "- 利用者: 犬\r\n- あなた: 猫\u{0085}- 利用者: 鳥\u{2028}- あなた: 魚\u{2029}- 利用者: 蟹\u{000B}- あなた: 蛸\u{000C}"
                .to_owned(),
        ];

        for raw in inputs {
            let parsed = parse_complete(&raw);
            // 上限は入力によらず守られる。
            assert!(
                u32::try_from(parsed.memories().len())
                    .is_ok_and(|len| len <= NewMemory::MAX_PER_TURN),
                "上限を超えた: {}",
                parsed.memories().len()
            );
        }
    }

    /// **行区切りが混ざっていても、行ごとに読める。**
    ///
    /// `\n` だけを見る実装だと、U+2028 で区切られた3行が1行に潰れて全部捨てられる。
    #[test]
    fn every_line_separator_splits_the_output() {
        let raw = "- 利用者: 犬を二匹飼っている\r\n- あなた: 常連客と世間話をするのが好き\u{2028}- 利用者: [経歴] 大阪で生まれ育った\u{0085}- あなた: [好み] 苦い珈琲が好き\u{2029}- 利用者: 弟が名古屋に住んでいる";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::User, "犬を二匹飼っている", None),
                memory(Subject::Character, "常連客と世間話をするのが好き", None),
                memory(Subject::User, "大阪で生まれ育った", Some("経歴")),
                memory(Subject::Character, "苦い珈琲が好き", Some("好み")),
                memory(Subject::User, "弟が名古屋に住んでいる", None),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
    }

    /// **記憶の本文に行区切りが残らない**——1行1件なので、行の中には現れない。
    #[test]
    fn a_memory_body_never_carries_a_line_separator() {
        let parsed = parse_complete("- 利用者: 犬を二匹飼っている\u{2028}- あなた: 猫が好き");

        for memory in parsed.memories() {
            assert!(
                !memory.content.as_str().chars().any(is_line_separator),
                "{:?}",
                memory.content.as_str()
            );
        }
    }

    // ------------------------------------------------------------------
    // Markdown の強調記号（ADR-0021 §11 / 作業単位 13c-1）
    // ------------------------------------------------------------------

    /// **強調記号は落とす。行ごと捨てない**（ADR-0021 §11）。
    ///
    /// 実機の 14B が書いた行そのもの。`**コーヒー**` は `- 主語: 本文` の形を
    /// **完全に満たすので破棄には一切現れず**、`memory` に更新も削除も無い以上、
    /// 落とさなければ**恒久的に残る。**
    #[test]
    fn markdown_emphasis_is_dropped_from_the_body() {
        let parsed = parse_complete("- あなた: 黒い猫（名前：**コーヒー**）を飼っている");

        assert_eq!(
            parsed.memories(),
            [memory(
                Subject::Character,
                "黒い猫（名前：コーヒー）を飼っている",
                None
            )]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 1);
    }

    /// **強調で囲まれた主語も読める**（ADR-0021 §11 の副産物）。
    ///
    /// # 順序の見張り番
    ///
    /// 正規化は**箇条書き記号を剥がすより前**でなければならない。
    /// 後ろに置くと、[`BULLETS`] に含まれる `*` が先頭から1つだけ剥がれて
    /// `*利用者**: 犬` になり、そこから強調を落とすと
    /// **`利用者*` という壊れた主語**ができて `missing_subject` で捨てられる。
    #[test]
    fn an_emphasized_subject_is_still_read() {
        let parsed = parse_complete("**利用者**: 犬を二匹飼っている");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "犬を二匹飼っている", None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 1);
    }

    /// **下線とバッククォートも同じ扱い**（ADR-0021 §11）。
    #[test]
    fn underscores_and_backticks_are_dropped_too() {
        let raw = "- あなた: __強い__コーヒーが好き\n- 利用者: `コーヒー` が好き";
        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::Character, "強いコーヒーが好き", None),
                memory(Subject::User, "コーヒー が好き", None),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 2);
    }

    /// **1文字の形は落とさない**（ADR-0021 §11）。
    ///
    /// 算術（`2*3*4`）が CommonMark の強調条件を満たしてしまい、
    /// しかも `*` は箇条書き記号（[`BULLETS`]）でもある。
    /// **実機で観測されたのは `**` の形だけ。**
    #[test]
    fn single_character_marks_are_left_alone() {
        let raw = "\
- あなた: 2*3*4 を暗算できる
- 利用者: *強調* を使う
- あなた: _下線_ も使う";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::Character, "2*3*4 を暗算できる", None),
                memory(Subject::User, "*強調* を使う", None),
                memory(Subject::Character, "_下線_ も使う", None),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 0);
    }

    /// **対になっていない印は落とさない**（ADR-0021 §11）。
    ///
    /// 落とすと、崩れた出力の本文を**こちらの推測で書き換える**ことになる。
    #[test]
    fn an_unpaired_mark_is_left_alone() {
        let parsed = parse_complete("- あなた: **閉じない強調");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::Character, "**閉じない強調", None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 0);
    }

    /// **同じ行に2組あれば2組とも落とす。数えるのは「行」1つ。**
    ///
    /// 件数が印の数（4つ）や組の数（2組）になっていないことを、ここで固定する。
    #[test]
    fn two_pairs_on_the_same_line_are_both_dropped() {
        let parsed = parse_complete("- あなた: **a** と **b**");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::Character, "a と b", None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 1);
    }

    /// **印だけの本文は、落とした結果を `domain` が空として弾く**（ADR-0021 §11）。
    ///
    /// **この層で空判定を書かない**（モジュールの doc）。落とすところまでがここの仕事で、
    /// 「空だから捨てる」を決めるのは `domain` 側。
    /// 数の上でも `content_empty` に計上され、**強調の件数とは別に**残る。
    #[test]
    fn a_body_of_marks_only_is_rejected_by_the_domain() {
        let parsed = parse_complete("- あなた: **\n- 利用者: 犬を二匹飼っている");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "犬を二匹飼っている", None)].as_slice()
        );
        assert_only(parsed.discards(), &[(DiscardReason::EmptyContent, 1)]);
        assert_eq!(parsed.emphasis_stripped(), 1);
    }

    /// **入れ子の強調は追わない**（ADR-0021 §11。この値で固定してよい）。
    #[test]
    fn nested_marks_are_not_followed() {
        let parsed = parse_complete("- あなた: ***強い***");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::Character, "*強い*", None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 1);
    }

    /// **落とした結果、上限に収まるようになった本文は採る**（判断は `domain` に従う）。
    ///
    /// 印を落とす前は上限 +4 文字。**正規化が `domain` の検証より前**でなければ
    /// `content_too_long` として捨てられる。
    #[test]
    fn a_body_that_fits_after_dropping_the_marks_is_kept() {
        let body = "あ".repeat(MemoryContent::MAX_CHARS);
        let parsed = parse_complete(&format!("- 利用者: **{body}**"));

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, &body, None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 1);
    }

    /// **数えるのは「印を落とした行数」。採否に関係なく数える**（ADR-0021 §11）。
    ///
    /// 測りたいのは「モデルが Markdown を吐く頻度」なので、**捨てた行の分も含める。**
    /// 含めないと、プロンプトに「Markdown を使うな」と書いたときに
    /// **効いたかどうかが分からなくなる**（落とした後の DB からは復元できない）。
    ///
    /// 採った件数（3）・落とした行数（4）・捨てた件数（2）を**すべて別の数**にしてある
    /// ——同値にすると取り違えが緑で通る。
    #[test]
    fn lines_that_carried_emphasis_are_counted_even_when_dropped() {
        let raw = "\
- 誰か: **推測**した
- **利用者**さん: 何かした
- 利用者: **犬**を二匹飼っている
- あなた: **朝**が弱い
- 利用者: 大阪で生まれ育った";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::User, "犬を二匹飼っている", None),
                memory(Subject::Character, "朝が弱い", None),
                memory(Subject::User, "大阪で生まれ育った", None),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[(DiscardReason::MissingSubject, 2)]);
        // **採用 3 件・破棄 2 件のどちらとも違う数。**
        assert_eq!(parsed.emphasis_stripped(), 4);
    }

    /// **強調の件数を破棄の数に混ぜない**（ADR-0021 §11）。
    ///
    /// 破棄率は既に定義された指標で、過去の測定値と比べるために使っている。
    /// **別の意味の数を混ぜると、以後の比較が全部無効になる。**
    #[test]
    fn dropping_emphasis_is_not_a_discard() {
        let parsed = parse_complete("- 利用者: **犬**を二匹飼っている");

        assert_eq!(parsed.memories().len(), 1);
        assert_eq!(parsed.emphasis_stripped(), 1);
        assert_eq!(parsed.discards().total(), 0, "{:?}", parsed.discards());
        for reason in DiscardReason::ALL {
            assert_eq!(
                parsed.discards().get(reason),
                0,
                "{} に混ぜないこと",
                reason.label()
            );
        }
    }

    /// **`strip_emphasis` は「落としたか」を返す。落とさなかった行は `None`。**
    ///
    /// 件数はこの返り値だけを根拠に数える。
    /// `parse_extraction` の出力だけを見ていると、
    /// **「落とした結果が元と同じ」場合と区別できない。**
    #[test]
    fn strip_emphasis_reports_only_the_lines_it_changed() {
        // 対になった2文字の形は落とす。
        assert_eq!(strip_emphasis("**コーヒー**").as_deref(), Some("コーヒー"));
        assert_eq!(
            strip_emphasis("__強い__コーヒー").as_deref(),
            Some("強いコーヒー")
        );
        assert_eq!(
            strip_emphasis("`コーヒー` が好き").as_deref(),
            Some("コーヒー が好き")
        );
        assert_eq!(strip_emphasis("**a** と **b**").as_deref(), Some("a と b"));
        // 入れ子は追わない。
        assert_eq!(strip_emphasis("***強い***").as_deref(), Some("*強い*"));
        // **宙に浮いた印**（前後のどちらにも文字が無い）は、対が無くても落とす。
        assert_eq!(
            strip_emphasis("- あなた: **").as_deref(),
            Some("- あなた: ")
        );

        // 落とさない形。**`None` でなければならない**（件数がここから出る）。
        for line in [
            "- あなた: 2*3*4 を暗算できる",
            "- あなた: *強調* を使う",
            "- あなた: _下線_ を使う",
            "- あなた: **閉じない強調",
            // 前に文字がある印は、対が無ければ残す（宙に浮いていない）。
            "- あなた: 強調しない**",
            "- 利用者: 犬を二匹飼っている",
        ] {
            assert_eq!(strip_emphasis(line), None, "{line}");
        }
    }

    /// **宙に浮いた印は「行の端に接している」ときだけ落とす**（ADR-0021 §11 / 13c-1）。
    ///
    /// 「前後のどちらにも文字が無い」だけを条件にすると、**空白で囲まれたべき乗が壊れる**
    /// （`2 ** 10` → `2  10`）。1文字の形（`*…*`）を対象から外した理由——
    /// **算術との衝突**——と同じ穴を、宙に浮いた印の規則が開け直していた。
    /// 少なくとも片側が行の端であることを求めれば、行の中の `**` は無傷で残る。
    #[test]
    fn a_floating_mark_must_touch_the_end_of_the_line() {
        let raw = "\
- あなた: 2 ** 10 は 1024
- 利用者: 猫が好き ** 犬は苦手";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::Character, "2 ** 10 は 1024", None),
                memory(Subject::User, "猫が好き ** 犬は苦手", None),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 0);

        // **行の端に接していれば、対が無くても落とす**（狭めすぎの見張り番）。
        // 落とさなくなると、本文が `**` だけの記憶が恒久的に残る。
        assert_eq!(
            strip_emphasis("- あなた: **").as_deref(),
            Some("- あなた: ")
        );
        assert_eq!(
            strip_emphasis("** 利用者: 犬").as_deref(),
            Some(" 利用者: 犬")
        );
        // 行の途中で浮いている印は残す。
        assert_eq!(strip_emphasis("- あなた: 2 ** 10 は 1024"), None);
    }

    /// **落とす印と残す印が同じ行に混在する**（ADR-0021 §11 の表）。
    ///
    /// # なぜこの形でしか観測できないか
    ///
    /// [`strip_emphasis`] は**落としたときだけ `Some`** を返すので、
    /// 落とすものが無い行では**書き戻した結果が捨てられる。**
    /// つまり「**印を残す**枝」は、**同じ行に落とす印と残す印が両方あるとき**にしか
    /// 出力に現れない。1行1種類・全落ちか全残しかの入力だけでは、
    /// 残す枝が何を書いても緑になる。
    #[test]
    fn a_line_can_carry_both_a_dropped_pair_and_a_kept_mark() {
        let parsed = parse_complete("- あなた: **A** と **閉じない");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::Character, "A と **閉じない", None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 1);
        // **印は2文字とも、元の文字のまま書き戻す。**
        assert_eq!(
            strip_emphasis("- あなた: **A** と **閉じない").as_deref(),
            Some("- あなた: A と **閉じない")
        );
    }

    /// **対は印の種類ごとに数える**（ADR-0021 §11 の表）。
    ///
    /// `**` は1つ（対にならない）、`` ` `` は2つ（対になる）。
    /// 種類を混ぜて数えると `**` まで落ち、**崩れた本文をこちらの推測で書き換える**
    /// ことになる。
    #[test]
    fn each_kind_of_mark_is_paired_on_its_own() {
        let parsed = parse_complete("- あなた: **a と `b`");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::Character, "**a と b", None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 1);
    }

    /// **残す印は、その印自身の文字で書き戻す**（ADR-0021 §11 の表）。
    ///
    /// `**` の対を落とした行に、対の無い `` ` `` が残る形。
    /// 書き戻す文字を取り違えると、**`` ` `` が `*` にすり替わって**
    /// 元の出力とは別の本文が恒久的に残る。
    #[test]
    fn a_kept_mark_is_written_back_with_its_own_character() {
        let parsed = parse_complete("- あなた: **a** と `未閉じ");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::Character, "a と `未閉じ", None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 1);
    }

    /// **隣が印なら「文字がある」側に倒す**（対の判定に任せる / ADR-0021 §11）。
    ///
    /// 印が隣り合っているだけで宙に浮いたと見なすと、
    /// **残すべき印が消える**（`` `**強い** `` の `` ` `` が落ちる）。
    #[test]
    fn a_mark_next_to_another_mark_is_not_floating() {
        let raw = "\
- あなた: `**強い**
- 利用者: 好き`**";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::Character, "`強い", None),
                memory(Subject::User, "好き`**", None),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
        // 落としたのは1行目だけ。
        assert_eq!(parsed.emphasis_stripped(), 1);
        assert_eq!(strip_emphasis("- 利用者: 好き`**"), None);
    }

    /// **印の隣の「空白」は半角スペースだけではない**（`TRIMMABLE` に従う）。
    ///
    /// 全角空白を数えないと `好き　**` の `**` が宙に浮いたと見なされず、
    /// **本文が `好き　**` の記憶が恒久的に残る**（`memory` に更新も削除も無い）。
    #[test]
    fn a_full_width_space_beside_a_mark_counts_as_empty() {
        let parsed = parse_complete("- 利用者: 好き\u{3000}**");

        assert_eq!(
            parsed.memories(),
            [memory(Subject::User, "好き", None)].as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 1);
        // **落とすところまでがこの層の仕事**——残った全角空白は `domain` が落とす。
        assert_eq!(
            strip_emphasis("- 利用者: 好き\u{3000}**").as_deref(),
            Some("- 利用者: 好き\u{3000}")
        );
    }

    /// **行頭の対無しの印でも、直後に文字があれば残す**（ADR-0021 §11 / 13c-1）。
    ///
    /// [`is_floating`] は前後の**両側**に文字が無いことを求める。`after` 側を落として
    /// `before` だけで決めると、**行頭に接している印は直後に文字があっても落ちる。**
    ///
    /// # なぜこの向きが痛いか
    ///
    /// 閉じ忘れ（`**利用者: 犬…`）は小型モデルが実際に書く形。印が残れば
    /// [`BULLETS`] の `*` が1つだけ剥がれて主語が `*利用者` になり、
    /// **未知の主語として捨てられる。** 印を落とすと主語が `利用者` として通り、
    /// **捨てるはずの行が記憶になる。** `memory` に更新も削除も無いので
    /// **恒久的に残る値を作る側**に倒れる。
    ///
    /// 行末側の対称形（`- あなた: 強調しない**`）は
    /// [`strip_emphasis_reports_only_the_lines_it_changed`] が押さえている。
    #[test]
    fn a_mark_at_the_head_of_the_line_is_kept_when_a_character_follows() {
        let raw = "**利用者: 犬を二匹飼っている";
        let parsed = parse_complete(raw);

        assert!(parsed.memories().is_empty(), "{:?}", parsed.memories());
        assert_only(parsed.discards(), &[(DiscardReason::MissingSubject, 1)]);
        assert_eq!(parsed.emphasis_stripped(), 0);
        // 印は1文字も落ちない（`*利用者` という壊れた主語のまま捨てられる）。
        assert_eq!(strip_emphasis(raw), None);
    }

    /// **「行の端」はトリム後の行に対する判定**（ADR-0021 §11 / 13c-1）。
    ///
    /// 行末に空白が付いていても、その手前の `**` は行の端に接していると見なす。
    /// [`parse_extraction`] のトリムを行頭だけにすると
    /// [`is_floating`] の「少なくとも片側が行の端」が成り立たなくなり、
    /// **本文が `**` の記憶がそのまま保存される**（`memory` に更新も削除も無い）。
    ///
    /// # `strip_emphasis` 単体では観測できない
    ///
    /// トリムするのは [`parse_extraction`] 側なので、**必ずそこを通して見る。**
    /// [`strip_emphasis`] に空白付きの行を直に渡すと、印は行の端に接していないので
    /// **正しい実装でも `None`** になる。
    #[test]
    fn the_end_of_the_line_is_judged_after_trimming() {
        // 半角スペース・全角空白のどちらでも同じ（`TRIMMABLE` に従う）。
        for raw in ["- あなた: ** ", "- あなた: **\u{3000}"] {
            let parsed = parse_complete(raw);

            assert!(
                parsed.memories().is_empty(),
                "{raw:?} {:?}",
                parsed.memories()
            );
            assert_only(parsed.discards(), &[(DiscardReason::EmptyContent, 1)]);
            assert_eq!(parsed.emphasis_stripped(), 1, "{raw:?}");
        }

        // トリムはこの層（`parse_extraction`）の仕事——`strip_emphasis` は行を trim しない。
        assert_eq!(strip_emphasis("- あなた: ** "), None);
    }

    /// **宙に浮いた印の規則は、印の種類を問わず効く**（ADR-0021 §11 / 13c-1）。
    ///
    /// 他の見張り番にある「浮いた印」は**すべて `**`**（2文字幅の `*`）なので、
    /// 規則を2文字の形だけに狭めても、`_` の行だけを潰しても素通りする。
    /// 行末の対無しバッククォート・対無し `__` は小型モデルが実際に書く形で、
    /// 残すと**本文の末尾に印が付いた記憶が恒久的に残る。**
    ///
    /// [`underscores_and_backticks_are_dropped_too`] は**対になった**形しか見ていない。
    #[test]
    fn a_floating_mark_is_dropped_whatever_its_kind() {
        let raw = "\
- 利用者: コーヒーが好き `
- あなた: 朝が弱い __";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::User, "コーヒーが好き", None),
                memory(Subject::Character, "朝が弱い", None),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
        assert_eq!(parsed.emphasis_stripped(), 2);

        // 1文字幅（`` ` ``）と2文字幅（`__`）の両方が落ちる。
        assert_eq!(
            strip_emphasis("- 利用者: コーヒーが好き `").as_deref(),
            Some("- 利用者: コーヒーが好き ")
        );
        assert_eq!(
            strip_emphasis("- あなた: 朝が弱い __").as_deref(),
            Some("- あなた: 朝が弱い ")
        );
    }

    // ------------------------------------------------------------------
    // 行頭の引用記号（作業単位 B-29 / C-15 の実測）
    // ------------------------------------------------------------------

    /// **依頼の引用書式を真似た行を、剥がしてから読む**（作業単位 B-29）。
    ///
    /// # 実測された形をそのまま置いてある（C-15 / 2026-08-29）
    ///
    /// Qwen3.5-9B 無検閲版の **272 観測のうち 15（5.5%）が「全行が `> ` で始まる」形**で、
    /// **そのターンは記憶が1件も残らなかった**（67 行とも `missing_subject`）。
    /// 観測された 67 行は**全部 `> - 主語: 本文`**（引用記号と箇条書き記号が重なった形）。
    ///
    /// **1行目は実測そのもの**（`scratchpad/agent-abde25c1a7e958507-c15/qu_a.out` の
    /// ターン 21）。**残りの3行で、箇条書き記号の有無・分類の有無・記号の後の空白の有無を
    /// 全部変えてある**——`"> - "` の4文字を決め打ちで落とすような
    /// 「実測の形だけを扱う」実装は、**2行目と4行目で赤になる。**
    #[test]
    fn a_line_that_copies_the_quote_prefix_is_read_after_the_mark_is_stripped() {
        let raw = "\
> - あなた: 平日は8時に開店する
> 利用者: 犬を二匹飼っている
> - 利用者: [経歴] 大阪で生まれ育った
>あなた: 朝が弱い";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::Character, "平日は8時に開店する", None),
                memory(Subject::User, "犬を二匹飼っている", None),
                memory(Subject::User, "大阪で生まれ育った", Some("経歴")),
                memory(Subject::Character, "朝が弱い", None),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
    }

    /// **剥がすのは1つだけ。入れ子は開かない**（`parse::drift` と同じ作法）。
    ///
    /// # `>` が [`BULLETS`] に入っていないことを、ここが押さえている
    ///
    /// **B-29 より前は `the_shared_bullet_set_is_exactly_these_nine_marks` の
    /// 「剥がさない側」に `'>'` が居た。** 剥がすようになった以上あそこには置けないので、
    /// **判別できる唯一の形**をこちらへ移してある——
    /// **`>` を共有集合へ足すと、`>>` が「引用1つ ＋ 箇条書き1つ」として開いてしまう。**
    /// **1つだけ剥がす実装では、残った `>` が主語の一部になって捨てられる。**
    #[test]
    fn a_nested_quote_is_not_opened() {
        let parsed = parse_complete(">> 利用者: 犬を二匹飼っている");

        assert!(parsed.memories().is_empty(), "入れ子の引用記号を開いた");
        assert_only(parsed.discards(), &[(DiscardReason::MissingSubject, 1)]);
    }

    /// **全角の `＞` は剥がさない**（[`QUOTE_MARK`] の doc と同じ理由）。
    ///
    /// **書くのではなく写している**——引用記号は文の流れで打つ約物ではなく、
    /// **依頼に見えている構造の記号**で、依頼（`prompt::writer::QUOTE_PREFIX`）が
    /// 書くのは半角である。C-15 で観測された 67 行も**全部半角**だった。
    #[test]
    fn a_full_width_quote_mark_is_not_stripped() {
        let parsed = parse_complete("＞ 利用者: 犬を二匹飼っている");

        assert!(parsed.memories().is_empty(), "全角の引用記号を剥がした");
        assert_only(parsed.discards(), &[(DiscardReason::MissingSubject, 1)]);
    }

    /// **行頭以外の `>` は本文の一部。落とさない**（作業単位 B-29）。
    ///
    /// 剥がすのは**行頭の1文字**だけで、`MemoryContent` の中身には触らない。
    /// **本文を推測で書き換えない**のは `strip_emphasis` の「対になっていない印を残す」
    /// と同じ規約で、`memory` に更新も削除も無い以上、
    /// **落とした文字は二度と戻らない。**
    #[test]
    fn a_quote_mark_inside_the_body_is_kept() {
        let raw = "\
- 利用者: 「2 > 1」と板書する癖がある
- あなた: 矢印を > で書く
利用者: >_< という顔文字をよく使う";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::User, "「2 > 1」と板書する癖がある", None),
                memory(Subject::Character, "矢印を > で書く", None),
                memory(Subject::User, ">_< という顔文字をよく使う", None),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
    }

    /// **強調記号を落とした後に引用記号を剥がす**（`parse::drift::parse_score_line` と同じ順序）。
    ///
    /// 逆順にすると `**> あなた**: 朝が弱い` の行頭が `*` のままで、
    /// [`BULLETS`] が `*` を1つ剥がして**主語が壊れる。**
    #[test]
    fn emphasis_is_dropped_before_the_quote_mark_is_stripped() {
        let raw = "\
> **あなた**: 朝が弱い
**> 利用者**: 犬を二匹飼っている";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::Character, "朝が弱い", None),
                memory(Subject::User, "犬を二匹飼っている", None),
            ]
            .as_slice()
        );
        assert_only(parsed.discards(), &[]);
    }

    /// **剥がした行数は、採否に関係なく数える**（[`ExtractedMemories::quotes_stripped`]）。
    ///
    /// # 1回の出力に4種類を混ぜてある
    ///
    /// | 行 | 剥がすか | 採るか |
    /// |---|---|---|
    /// | `> - 利用者: 犬を二匹飼っている` | **剥がす** | **採る** |
    /// | `> 誰か: 猫が好き` | **剥がす** | 捨てる（[`DiscardReason::MissingSubject`]） |
    /// | `> 以下が抽出結果です` | **剥がす** | 捨てる（[`DiscardReason::Malformed`]） |
    /// | `- あなた: 朝が弱い` | 剥がさない | 採る |
    ///
    /// **捨てた行を数えない実装は 3 ではなく 1 になる。**
    /// そうなると「是正が効いた」と「モデルが書かなくなった」が区別できない——
    /// **捨てられた回にこそ引用記号が集まっている**（C-15 では全行が `> ` の回だけが 0 件だった）。
    ///
    /// **`emphasis_stripped` と `discards` に混ざっていないことも同じテストで見る**
    /// （3つとも別の意味の数で、混ぜると破棄率が過去の測定と比べられなくなる）。
    #[test]
    fn a_stripped_quote_is_counted_even_when_the_line_is_discarded() {
        let raw = "\
> - 利用者: 犬を二匹飼っている
> 誰か: 猫が好き
> 以下が抽出結果です
- あなた: 朝が弱い";

        let parsed = parse_complete(raw);

        assert_eq!(
            parsed.memories(),
            [
                memory(Subject::User, "犬を二匹飼っている", None),
                memory(Subject::Character, "朝が弱い", None),
            ]
            .as_slice()
        );
        assert_eq!(
            parsed.quotes_stripped(),
            3,
            "捨てた行の引用記号を数えていない"
        );
        // **破棄にも強調の数にも混ざっていない。**
        assert_only(
            parsed.discards(),
            &[
                (DiscardReason::MissingSubject, 1),
                (DiscardReason::Malformed, 1),
            ],
        );
        assert_eq!(parsed.emphasis_stripped(), 0);
    }

    /// **剥がしていない回は 0。** 数えているのが「行数」であって「行の数」ではないこと。
    ///
    /// **全角の `＞` と入れ子の2つ目も数に入らない**——
    /// [`a_full_width_quote_mark_is_not_stripped`] は剥がさないので 0、
    /// [`a_nested_quote_is_not_opened`] は**1つだけ**剥がすので 1 である。
    #[test]
    fn quotes_are_only_counted_when_a_mark_is_actually_stripped() {
        assert_eq!(
            parse_complete("- 利用者: 犬を二匹飼っている").quotes_stripped(),
            0
        );
        assert_eq!(
            parse_complete("＞ 利用者: 犬を二匹飼っている").quotes_stripped(),
            0,
            "全角の引用記号を剥がした行として数えた"
        );
        assert_eq!(
            parse_complete(">> 利用者: 犬を二匹飼っている").quotes_stripped(),
            1,
            "入れ子を1行で2回数えた"
        );
        // **本文の中の `>` は行頭ではない。**
        assert_eq!(
            parse_complete("- あなた: 矢印を > で書く").quotes_stripped(),
            0
        );
    }

    // ------------------------------------------------------------------
    // 打ち切られた応答（ADR-0021 §17 / 作業単位 24）
    // ------------------------------------------------------------------

    /// 打ち切られた応答。**3行目が「十年前から通っている」の途中で切れたもの。**
    ///
    /// **断片なのに `- 主語: 本文` の形を完全に満たす**ので、
    /// 印を見なければ [`DiscardCounts`] には一切現れず、
    /// **誤った事実が恒久的に残る**（`memory` に更新も削除も無い）。
    const TRUNCATED_RAW: &str = "\
- 利用者: 犬を二匹飼っている
- あなた: 苦い珈琲が好き
- 利用者: 十年前から通って";

    /// **同じ入力を両方の印に通す**（ADR-0021 §17 / 作業単位 24）。
    ///
    /// # ここが押さえているもの
    ///
    /// * **切れた最後の行が、そのままパースを通ってしまう形。**
    ///   [`TRUNCATED_RAW`] の3行目は**崩れていない。**
    ///   崩れた断片だけで検査すると、[`DiscardReason::Malformed`] が偶然拾うので
    ///   **この穴が残ったまま緑になる。**
    /// * **落とすのは最後の1行だけ。** 3行とも別の内容で、1行目と3行目は主語も同じ
    ///   ——**最初の1行を落とす実装は、残る2件の中身が変わる。**
    /// * **印が無ければ1行も落とさない。** 同じ入力で3件とも残ることまで見るので、
    ///   「常に最後の1行を捨てる」実装も赤になる。
    #[test]
    fn only_the_last_line_is_dropped_and_only_when_the_response_was_truncated() {
        let truncated = parse_truncated(TRUNCATED_RAW);

        assert_eq!(
            truncated.memories(),
            [
                memory(Subject::User, "犬を二匹飼っている", None),
                memory(Subject::Character, "苦い珈琲が好き", None),
            ]
            .as_slice(),
            "落とすのは最後の1行だけ（前の行は完結している）"
        );
        assert_only(truncated.discards(), &[(DiscardReason::Truncated, 1)]);

        let complete = parse_complete(TRUNCATED_RAW);

        assert_eq!(
            complete.memories(),
            [
                memory(Subject::User, "犬を二匹飼っている", None),
                memory(Subject::Character, "苦い珈琲が好き", None),
                memory(Subject::User, "十年前から通って", None),
            ]
            .as_slice(),
            "打ち切りの印が無い回は1行も落とさない"
        );
        assert_only(complete.discards(), &[]);
    }

    /// **崩れた断片は、崩れではなく「切れた」として数える**（ADR-0021 §17）。
    ///
    /// 同じ行が、印の有無で **`malformed` / `truncated`** に分かれる。
    /// 混ぜると **「上限が現実に合っていない」というシグナル**が
    /// 「モデルが形式を崩している」に紛れて読めなくなる
    /// （ADR-0021 §3 が理由ごとに数える理由そのもの）。
    #[test]
    fn a_broken_fragment_is_counted_as_truncated_not_as_malformed() {
        // 区切りに届く前に切れた行。**それ自体が崩れている。**
        let raw = "- 利用者: 犬を二匹飼っている\n- 利用";

        let truncated = parse_truncated(raw);
        assert_eq!(truncated.memories().len(), 1);
        assert_only(truncated.discards(), &[(DiscardReason::Truncated, 1)]);

        let complete = parse_complete(raw);
        assert_eq!(complete.memories().len(), 1);
        assert_only(complete.discards(), &[(DiscardReason::Malformed, 1)]);
    }

    /// **断片は「最後の行」ではなく「最後の中身のある行」**（ADR-0021 §17）。
    ///
    /// # CRLF の応答では必ずこの形になる
    ///
    /// [`parse_extraction`] は `\r` も `\n` も行区切りに畳むので、
    /// **CRLF で書く上流の応答は、最後に必ず空行が付く。**
    /// 末尾の空行を「最後の行」と読む実装は、**断片を1行も落とさない**
    /// ——しかも `truncated=0` と記録されるので、ログからは正常に見える。
    #[test]
    fn the_fragment_is_the_last_line_that_has_content() {
        let crlf = "- 利用者: 犬を二匹飼っている\r\n- 利用者: 十年前から通って\r\n";

        let truncated = parse_truncated(crlf);

        assert_eq!(
            truncated.memories(),
            [memory(Subject::User, "犬を二匹飼っている", None)].as_slice(),
            "末尾の空行ではなく、中身のある最後の行を落とすこと"
        );
        assert_only(truncated.discards(), &[(DiscardReason::Truncated, 1)]);
    }

    /// **落とす行が無い回は数えない**（ADR-0021 §17）。
    ///
    /// 打ち切られたうえに1行も書かれていない回。**捨てていないので数えない**
    /// ——数えると、破棄率の分子に「存在しなかった行」が混ざる。
    #[test]
    fn a_truncated_response_with_no_content_discards_nothing() {
        for raw in ["", "\n\n", "\u{3000}\n \n"] {
            let parsed = parse_truncated(raw);

            assert!(parsed.memories().is_empty(), "{raw:?}");
            assert_only(parsed.discards(), &[]);
        }
    }

    /// **1行しか無い回は、その1行が断片**（ADR-0021 §17）。
    ///
    /// 「前の行は完結している」を「1行目は必ず残す」と実装すると、
    /// **上限に張り付いた回の唯一の行がそのまま記憶になる。**
    #[test]
    fn the_only_line_of_a_truncated_response_is_the_fragment() {
        let parsed = parse_truncated("- 利用者: 十年前から通って");

        assert!(parsed.memories().is_empty(), "{:?}", parsed.memories());
        assert_only(parsed.discards(), &[(DiscardReason::Truncated, 1)]);
    }

    /// **断片は「上限超過」ではなく「切れた」として数える**（ADR-0021 §17）。
    ///
    /// 上限（[`NewMemory::MAX_PER_TURN`]）ちょうどまで採ったうえで断片が続く回。
    /// `over_limit` に紛れ込ませると、**「饒舌すぎる」と「応答が切れた」が区別できない**
    /// ——打ち手が正反対（プロンプトを直す / 上限を上げる）なので、混ぜると判断できない。
    #[test]
    fn the_fragment_is_not_counted_as_over_the_limit() {
        let mut lines: Vec<String> = (0..NewMemory::MAX_PER_TURN)
            .map(|index| format!("- 利用者: 犬を{index}匹飼っている"))
            .collect();
        lines.push("- あなた: 十年前から通って".to_owned());
        let raw = lines.join("\n");

        let parsed = parse_truncated(&raw);

        assert_eq!(
            u32::try_from(parsed.memories().len()).unwrap_or(u32::MAX),
            NewMemory::MAX_PER_TURN
        );
        assert_only(parsed.discards(), &[(DiscardReason::Truncated, 1)]);
    }

    /// **断片の中身は数えない**（[`ExtractedMemories::emphasis_stripped`] / ADR-0021 §17）。
    ///
    /// 打ち切りは `**コーヒー**` を `**コーヒー` に切る。
    /// **断片から数えた印は「モデルが Markdown を吐く頻度」ではなく打ち切りの副産物**で、
    /// 混ぜるとこの指標が上限の設定次第で動く。**断片は分解より前に落とす。**
    #[test]
    fn the_fragment_does_not_contribute_to_the_emphasis_count() {
        let raw = "- 利用者: 犬を二匹飼っている\n- あなた: **強い**珈琲と **甘い";

        let truncated = parse_truncated(raw);
        assert_eq!(truncated.emphasis_stripped(), 0);
        assert_eq!(truncated.memories().len(), 1);

        // **同じ行を完結として通せば数えられる**（＝この行に落とす印がある）。
        let complete = parse_complete(raw);
        assert_eq!(complete.emphasis_stripped(), 1);
        assert_eq!(complete.memories().len(), 2);
    }

    /// **断片の引用記号も数えない**（[`ExtractedMemories::quotes_stripped`] / 作業単位 B-29）。
    ///
    /// 理由は [`the_fragment_does_not_contribute_to_the_emphasis_count`] と同じ——
    /// **断片は中身を一切見ずに捨てる**（ADR-0021 §17）ので、
    /// 剥がす処理そのものを通らない。通してしまうと
    /// 「モデルが引用書式を持ち込む頻度」が**上限の設定次第で動く。**
    #[test]
    fn the_fragment_does_not_contribute_to_the_quote_count() {
        let raw = "> - 利用者: 犬を二匹飼っている\n> - あなた: 十年前から通って";

        let truncated = parse_truncated(raw);
        assert_eq!(truncated.quotes_stripped(), 1);
        assert_eq!(truncated.memories().len(), 1);
        assert_only(truncated.discards(), &[(DiscardReason::Truncated, 1)]);

        // **同じ行を完結として通せば数えられる**（＝この行に剥がす記号がある）。
        let complete = parse_complete(raw);
        assert_eq!(complete.quotes_stripped(), 2);
        assert_eq!(complete.memories().len(), 2);
    }
}
