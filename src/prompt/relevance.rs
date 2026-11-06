//! 記憶の**関連度**を測る（ADR-0023 の3層目「関連」/ docs/adr/0023-three-layer-recall.md）。
//!
//! **I/O を持たない純粋関数**（CLAUDE.md 3節）。DB も上流も触らず、
//! 「今回の発言」と「記憶の本文の一覧」という文字列だけから比較可能な値を作る。
//!
//! # 何を測るか — **文字 unigram ＋ bigram** の、**希少度で重み付けした** Dice 係数
//!
//! ```text
//! score(query, memory) = 2 * Σ w(g)*min(q_g, m_g) / ( Σ w(g)*q_g + Σ w(g)*m_g )
//! ```
//!
//! `g` は gram（unigram または bigram）、`q_g` / `m_g` はその出現回数、
//! `w(g)` は**候補集合の中での希少度**（下記）。
//!
//! * **文字 unigram と bigram を1つの多重集合に混ぜる。**
//! * **文字**＝ Unicode スカラー値。**バイトではない。**
//!   バイトで数えると `𠮷` と `𩸽` が先頭バイトを共有して 0 にならない。**そのうえ、
//!   バイト添字のスライスは文字境界を割ると panic する**
//!   （絶対制約「パーサは panic しない」と同じ理由でここも panic させない）。
//! * **形態素解析はしない。** 依存クレートを増やさないため（ADR-0023 の「依存: 無し」）。
//! * **正規化しない。** 大小文字も Unicode 正規化（NFC / NFKC）も行わない。
//!   `が`（U+304C）と `か` ＋ U+3099 は**別の文字列として扱う。**
//!   正規化を入れるなら、それは採点ではなく入口（`SafeText`）の仕事。
//!
//! ## なぜ unigram を混ぜるのか（作業単位 15a-2 の変更A）
//!
//! **bigram だけでは、日本語の重要語の一致を丸ごと落とす。** 15a の実測:
//!
//! ```text
//! 発言「僕は猫を飼ってるんだけど、葵さんは何か飼ってる？」
//!   黒猫（コーヒー）を飼育している    →  54‰   共通は `を飼` だけ
//!   新しい仕事について迷っている      →  55‰   共通は `って` だけ
//! ```
//!
//! **無関係な記憶が勝っていた。** 原因は bigram の構造的な弱点である ——
//! **活用と複合で隣の文字が変わる**ので `猫を` と `黒猫`、`飼って` と `飼育` が
//! 一致しない。**括弧（`猫（`）も隣の文字を変える。**
//! unigram を足すと `猫` と `飼` が直接当たるようになり、**157‰ 対 135‰ で逆転する。**
//!
//! **別々に Dice を取って合成する形は採らない。** 片側が1文字だと bigram 側の Dice が
//! `0/0` になり、**「未定義をどう混ぜるか」という決めごとが増える。**
//! 1つの多重集合に入れれば、分母が 0 になるのは**両側とも gram を持たないとき**だけで、
//! 境界が1つで済む。整数のままでいられる（合成すると分数の和になり分母が積で膨らむ）のも同じ理由。
//!
//! ## なぜ希少度で重み付けするのか（変更B）
//!
//! **失敗のもう半分は機能語である。** `って` / `てい` / `いる` は
//! 大半の記憶に出るので、**無関係な記憶にも点を配ってしまう。**
//!
//! ```text
//! w(g) = RARITY_SCALE / df(g)        df(g) = その gram を持つ候補記憶の件数
//! ```
//!
//! **「その gram を k 件が共有しているなら、1件あたりの取り分は 1/k」** という素直な形。
//! 古典的な IDF は `log(N/df)` だが、**`N` は全 gram に共通の定数なので比の中で約分され、
//! 線形の `N/df` は `1/df` と完全に同じ順位を与える。**`log` を使うと浮動小数が要るので、
//! **整数のままでいられる `1/df` を採る**（15a で `f64` を避けた判断をそのまま活かす）。
//!
//! `RARITY_SCALE` は整数除算の刻みを細かくするためだけの定数で、**順位には効かない**
//! （全 gram に同じ倍率が掛かるだけ）。
//!
//! ### 切り捨ての上界 — **`df` に比例して悪化する**
//!
//! > **⚠ ここには「切り捨ての影響は最大でも 1/`RARITY_SCALE` 相対」と書いてあったが、
//! > それは誤りだった。`df = 1` のときにしか成り立たない。**
//! > **この誤った上界は、次に誰かが定数を下げるときの根拠になる。**
//!
//! `w = floor(S/df)` は真値 `S/df` より最大 1 だけ小さい。相対誤差の上界は
//!
//! ```text
//! 1 / (S/df) = df / RARITY_SCALE
//! ```
//!
//! つまり **`df` に比例して悪化する。** 効き目の側から言うと、
//! **隣り合う `df` が必ず区別できる十分条件は**
//!
//! ```text
//! S/df - S/(df+1) = S / (df*(df+1)) >= 1     ⟺     df*(df+1) <= RARITY_SCALE
//! ```
//!
//! すなわち **`df ≲ √RARITY_SCALE`。現行値なら約 1000**
//! （十分条件では `df = 999` まで。実測では `df = 1032` で丸めが始まる ——
//! 十分条件であって必要条件ではないので、実測の方が少し先になる）。
//! **想起の作業点は記憶90件前後（ADR-0023）なので、10倍以上の余裕がある。**
//!
//! 見張り番は `rarity_stays_distinguishable_across_the_working_range_of_document_frequencies`
//! （`df` を 1 から 150 まで走査し、**`RARITY_SCALE` から直接**この不等式を検査する）。
//!
//! **効き目は「順序を変える」ではなく「差を広げる」だった**（実測。下表）。
//! 4組のいずれも変更A だけで正しい順序になっており、**B は順序を変えていない。**
//! 変えたのは**高い方と低い方の比**である。
//!
//! | 発言 → 期待して高い方 / 低い方 | 変更前 | 変更A | 変更A＋B |
//! |---|---|---|---|
//! | ① 猫を飼ってる？ → 黒猫（コーヒー）/ 新しい仕事 | 54 / 55 **逆転** | 157 / 135（1.16倍） | 85 / 59（**1.44倍**） |
//! | ② このお店で長いの？ → 10年以上勤務 / 猫が盗む | 160 / 0 | 230 / 43（5.35倍） | 209 / 25（**8.36倍**） |
//! | ③ コーヒーっていたずらする？ → 毛玉 / 仕事の心構え | 45 / 0 | 200 / 78（2.56倍） | 115 / 27（**4.26倍**） |
//! | ④ 新しい仕事、迷ってる → 新しい仕事 / 黒猫 | 428 / 0 | 517 / 133（3.89倍） | 389 / 38（**10.2倍**） |
//!
//! > **変更前が「② 160 / 0」のように 0 を出していたことに注意。**
//! > **0 が並ぶと順序が付かない**ので、変更A は「逆転を直した」だけでなく
//! > **同点を減らしてもいる。**
//!
//! ## 重複する gram は**多重集合**として数える
//!
//! 同じ gram が複数回現れたら、その回数ぶん数える。共通数は**両側の出現回数の小さい方**。
//!
//! ```text
//! query  = "ああああ"  → あ ×4 ＋ (あ,あ) ×3 ＝ 7
//! memory = "あああ"    → あ ×3 ＋ (あ,あ) ×2 ＝ 5
//! 共通 = min(4,3) + min(3,2) = 5  →  2*5 / (7+5) = 10/12
//! ```
//!
//! **集合として数えない理由は長さの正規化。** 集合にすると繰り返しが分母を縮めるので、
//! 「猫が好き猫が好き猫が好き」のような**同じ語を繰り返すだけの記憶が、
//! 一致度で `猫が好き` と並んでしまう。**
//! Dice 係数は本来「長さで正規化された重なり」なので、多重集合の方が定義に忠実。
//!
//! # 境界4種（**unigram を混ぜたので 15a から決め直した**）
//!
//! | 状況 | 値 | 見張り番 |
//! |---|---|---|
//! | どちらかが**空文字** | [`Relevance::MIN`] | `an_empty_side_scores_the_minimum` |
//! | 共通の gram が**無い**（＝**文字が1つも共通しない**） | [`Relevance::MIN`] | `strings_without_a_shared_gram_score_the_minimum` |
//! | 両方が**同一の非空文字列**（**1文字でも**） | [`Relevance::MAX`] | `identical_strings_score_the_maximum` |
//! | **候補が1件**（df が全 gram で 1 以下） | **重みが一様**＝変更A だけの値 | `a_single_candidate_makes_every_weight_uniform` |
//!
//! > **15a の「1文字なら最小」は無くなった。** unigram を持つので、
//! > **同じ1文字どうしは最大**になる。**空文字だけが「gram を1つも持たない」側に残る。**
//!
//! > **「同一なら最大」の例外は空文字同士だけ。** gram が1つも無いので分母が 0 になり、
//! > 比として定義できない。0/0 を 1 と読む流儀もあるが、**想起でそれを採ると
//! > 「発言が空なら全件が満点」＝全件注入に戻る**ので採らない。
//! > 記憶の本文は `MemoryContent::parse` が空を弾くので、実際に効くのは発言側。
//!
//! > **候補が1件のときの重みは「決めごと」である。** 1件しか無ければ
//! > すべてを希少としても平凡としても順位は付かない。**どちらでもよいものは固定しておく。**
//! > 「一様」を選んだので、**候補1件の値は希少度を入れる前と完全に一致する** ——
//! > つまり**変更B を切って測りたければ、候補を1件にすればよい。**
//!
//! # 対称性 — **一部だけ成り立つ。15a から変わった**
//!
//! * **重み関数を固定すれば対称。** 上の式は `q` と `m` の入れ替えで不変。
//! * **重みは候補集合から決まる。** したがって
//!   **候補集合が違う2つの呼び出しの間では対称ではない**（`the_score_is_not_symmetric_across_different_candidate_sets`）。
//! * 候補が1件どうしなら重みが一様になるので、
//!   `score(a, [b]) == score(b, [a])` は**成り立つ**（`the_score_is_symmetric_when_the_weights_are_the_same`）。
//!
//! # 差し替え可能にしてある形 — **値の型を固定し、作り方を1関数に閉じ込める**
//!
//! **型が保証しているのは1つだけ** —— [`Relevance`] のフィールドは非公開で、
//! **外部にコンストラクタが無い。** したがって
//! **呼び出し側は `Relevance` を捏造できず、[`score_relevance`] から受け取るしかない。**
//!
//! **それ以外は規約でしかない。** 型は次のどちらも防いでいない ——
//!
//! * **[`Relevance::per_mille`] は `pub`。** `if r.per_mille() >= 150 { 採用 }` のような
//!   **閾値を呼び出し側に書ける。** `per_mille` は**ログ用**であり
//!   採否や順序の判定に使わない、というのは**規約であって型の保証ではない。**
//!   埋め込みのコサイン類似度へ差し替えたとき、**この閾値は黙って別の意味になる。**
//! * **`Debug` が分子・分母をそのまま印字する**
//!   （`Relevance { numerator: 2000000, denominator: 6000000 }`）。
//!   テストの失敗メッセージには有用だが、**「中身を読めない」わけではない。**
//!
//! > **`ValidTag`（ADR-0006）と同じ強さに読まないこと。** あちらは
//! > **辞書引き以外の構築手段を生やさないことで、不正な値を表現できなくしている。**
//! > ここで表現できなくしてあるのは**捏造だけ**で、
//! > **「比べることしかできない」は型ではなく規約が支えている。**
//! > （CLAUDE.md「何も不可能にしないものを、型で表現したことにしない」。
//! > `0012b` の `enum { Readable, Unreadable }` と同じ形の誤りを繰り返さないための注記）
//!
//! 埋め込みへ移すときに **[`score_relevance`] のシグネチャ差し替えだけで済むのは、
//! 選別側が [`Ord`] しか使っていない限り**であって、**型がそれを強制してはいない。**
//! 選別側を書くときは `per_mille` を呼ばないこと。
//!
//! **1件ずつ採点する入口は用意しない。** 希少度が候補集合に依存する以上、
//! 「1件だけ採点する」は**候補集合が1件である**という別の意味を持ってしまう。
//! **入口を1つにして、候補集合を必ず渡させる。**

use std::cmp::Ordering;
use std::collections::HashMap;
use std::num::NonZeroU64;

/// 数える単位。**文字（Unicode スカラー値）1つ、または隣り合う2つ。**
///
/// バイト列ではなく `char` を持つので、**バイト添字のスライスが登場しない。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Gram {
    /// 文字1つ。**活用や複合で隣が変わっても当たる**ので、変更A の主役。
    Unigram(char),
    /// 隣り合う文字2つ。語順・並びの情報を持つ。
    ///
    /// > **⚠ 「向きを反転する変異が等価」なのは無条件ではない。**
    /// > `Bigram(left, current)` を `Bigram(current, left)` にする変異は
    /// > スイート全体を緑のまま通すが、それは次の3つが**同時に成り立つ限り**である
    /// > （CLAUDE.md「等価という申告こそ疑う / 第5の分類」）——
    /// >
    /// > 1. **構築箇所が [`gram_counts`] の1つだけ**であること。
    /// >    2箇所目ができた瞬間、片方だけ反転すれば当たらなくなる。
    /// > 2. **フィールド順に依存する振る舞いが無い**こと。
    /// >    `Ord`（`(a,b) < (b,a)`）や serde（列の順）を導出した瞬間に等価でなくなる。
    /// > 3. **発言側と記憶側が同じ関数を通る**こと。
    /// >    別経路で作るようになれば、向きの食い違いがそのまま不一致になる。
    /// >
    /// > **どれかを崩すときは、向きを見る見張り番を足すこと。**
    Bigram(char, char),
}

/// gram ごとの出現回数（多重集合）。
type GramCounts = HashMap<Gram, u64>;

/// 希少度の刻み。**順位には効かない**（全 gram に同じ倍率が掛かり、比の中で約分される）。
///
/// 整数除算 `RARITY_SCALE / df` の切り捨てを小さくするためだけの定数。
///
/// **下げるときは、モジュールの doc「切り捨ての上界」を読むこと。**
/// 満たすべき条件は **`df·(df+1) ≤ RARITY_SCALE`**（作業点の `df` に対して）で、
/// **これを割ると希少度が「全部 1 に潰れる」より前に静かに効かなくなる。**
/// 見張り番は `rarity_stays_distinguishable_across_the_working_range_of_document_frequencies`。
const RARITY_SCALE: u64 = 1_000_000;

/// 記憶1件の**関連度。** 大小を比べるためだけの値。
///
/// **フィールドは非公開で、外部にコンストラクタが無い**ので捏造できない。
/// ただし **[`Relevance::per_mille`] と `Debug` からは中身が出る。**
/// 「比べることしかできない」は**規約であって型の保証ではない**
/// （モジュールの doc「差し替え可能にしてある形」を読むこと）。
/// `per_mille` は**ログ用**で、順序の判定に使わない（丸めた値が同じでも比は異なりうる）。
///
/// # なぜ `f64` を返さないか
///
/// * **等価比較が壊れる。** `0.1 + 0.2 != 0.3` の世界で「同点」を判定させたくない。
///   同点は現実に起きる（共通 gram が同数・同長の記憶は珍しくない）。
/// * **`Ord` が付かない。** `f64` は `PartialOrd` しか持たないので、
///   並べ替えるたびに「NaN のときどうするか」を呼び出し側が書くことになる。
///   Dice 係数に NaN は無い（分母 0 は [`Relevance::MIN`] に畳んである）のに、
///   **型がそれを表現できない。**
/// * **テストが丸め誤差の話になる。** 比のまま持てば期待値は整数で書ける。
///
/// 内部は約分していない分数（`numerator / denominator`）で、比較は
/// **たすき掛け（u128）**で厳密に行う。したがって `2/4` と `4/8` は**等しい。**
#[derive(Debug, Clone, Copy)]
pub struct Relevance {
    /// Dice 係数の分子（`2 * 重み付きの共通 gram 数`）。**常に `denominator` 以下。**
    numerator: u64,
    /// Dice 係数の分母（両側の重み付き gram 数の和）。**0 になりえない**ので `NonZeroU64`。
    ///
    /// 型で 0 を排除してあるので、[`Relevance::per_mille`] の除算に
    /// 「分母が 0 なら」という**到達しない分岐を書かずに済む。**
    denominator: NonZeroU64,
}

impl Relevance {
    /// 最小値（＝0）。共通の gram が無い場合と、両側とも gram を作れない場合。
    pub const MIN: Self = Self {
        numerator: 0,
        denominator: NonZeroU64::MIN,
    };

    /// 最大値（＝1）。gram の多重集合が完全に一致した場合。
    pub const MAX: Self = Self {
        numerator: 1,
        denominator: NonZeroU64::MIN,
    };

    /// ログ用の千分率（0〜1000）。**切り捨て。**
    ///
    /// **順序の判定にこれを使わないこと。** 丸めた結果が同じでも元の比は異なりうる
    /// （見張り番 `two_scores_that_round_to_the_same_per_mille_still_order_correctly`）。
    /// 比較は [`Ord`] を使う。
    pub fn per_mille(self) -> u32 {
        let scaled =
            u128::from(self.numerator).saturating_mul(1000) / u128::from(self.denominator.get());
        // 不変条件（`numerator <= denominator`）より最大 1000 なので、この分岐には到達しない。
        // 到達しても panic しないことだけを保証する。
        u32::try_from(scaled).unwrap_or(u32::MAX)
    }
}

impl Ord for Relevance {
    /// たすき掛けで厳密に比較する。**丸めない。**
    ///
    /// 分子・分母とも `u64` なので積は最大 128 ビット。`u128` で受ければ**桁溢れしない。**
    fn cmp(&self, other: &Self) -> Ordering {
        let left = u128::from(self.numerator) * u128::from(other.denominator.get());
        let right = u128::from(other.numerator) * u128::from(self.denominator.get());
        left.cmp(&right)
    }
}

impl PartialOrd for Relevance {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Relevance {
    /// **表現ではなく比で等しさを決める。** `2/4` と `4/8` は等しい。
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Relevance {}

/// 今回の発言に対する、**候補記憶それぞれの**関連度を測る。
///
/// 返る `Vec` は**候補と同じ件数・同じ順序。** 候補が空なら空の `Vec`。
///
/// **1件だけ採点する入口は無い。** 希少度（`w(g) = RARITY_SCALE / df(g)`）が
/// **候補集合全体から決まる**ので、1件を単独で採点することに意味が無いためである。
/// 候補を1件だけ渡せば重みは一様になり、**希少度を入れる前と同じ値**が返る（境界4）。
///
/// **決定的**（同じ入力・同じ順序なら常に同じ値）で、**panic しない。**
/// 空文字・1文字・絵文字・結合文字・アストラル面の文字を渡しても壊れない。
///
/// 計算量は「発言の文字数 ＋ 候補の総文字数」に対して線形。記憶1件は 500 文字以下
/// （`MemoryContent::MAX_CHARS`）、発言はトークン予算で頭打ちなので、
/// 全件に対して毎ターン回しても問題にならない。
pub fn score_relevance<'a>(
    query: &str,
    memories: impl IntoIterator<Item = &'a str>,
) -> Vec<Relevance> {
    // 候補ごとの gram 表を先に作る。**df もここから数える**ので、
    // 候補の文字列を2度走査しない（イテレータを1度しか回さなくて済む）。
    let candidates: Vec<GramCounts> = memories.into_iter().map(gram_counts).collect();
    let frequencies = document_frequencies(&candidates);
    let query_grams = gram_counts(query);

    candidates
        .iter()
        .map(|memory_grams| dice(&query_grams, memory_grams, &frequencies))
        .collect()
}

/// 文字 unigram と bigram の出現回数。**バイトではなく Unicode スカラー値。**
///
/// `chars()` を1つ前の文字と組にして進むだけなので、
/// **バイト添字のスライスを作らない**（文字境界で panic する余地が無い）。
/// 空文字なら空の表を返す。**1文字なら unigram を1つ持つ**（bigram は 0）。
fn gram_counts(text: &str) -> GramCounts {
    let mut counts = GramCounts::new();
    let mut previous: Option<char> = None;

    for current in text.chars() {
        bump(&mut counts, Gram::Unigram(current));
        if let Some(left) = previous {
            bump(&mut counts, Gram::Bigram(left, current));
        }
        previous = Some(current);
    }

    counts
}

/// 出現回数を1つ増やす。**天文学的な長さでも panic させない**（現実には飽和しない）。
fn bump(counts: &mut GramCounts, gram: Gram) {
    let count = counts.entry(gram).or_insert(0u64);
    *count = count.saturating_add(1);
}

/// gram ごとの**文書頻度**（その gram を持つ候補記憶の**件数**）。
///
/// **出現回数ではなく件数。** 1件の記憶が同じ gram を10回持っていても 1 と数える
/// （「その語を持つ記憶が何件あるか」が希少度の意味）。
fn document_frequencies(candidates: &[GramCounts]) -> HashMap<Gram, u64> {
    let mut frequencies: HashMap<Gram, u64> = HashMap::new();
    for memory in candidates {
        for gram in memory.keys() {
            let count = frequencies.entry(*gram).or_insert(0u64);
            *count = count.saturating_add(1);
        }
    }
    frequencies
}

/// gram の希少度。**多くの候補が持つ gram ほど軽い。**
///
/// * **どの候補も持たない gram**（発言にしか無い語）は `df = 0` だが、
///   **いちばん希少**なので `df = 1` と同じ最大の重みにする。
/// * **1 を下回らせない。** `df` が [`RARITY_SCALE`] を超えると整数除算で 0 に潰れ、
///   **その gram が分母からも消えて分母 0 ＝ 最小値**という壊れ方をする。
///
/// > **⚠ 外側の `.max(1)` は現実には到達しない保険である。** 発火するには
/// > `df > RARITY_SCALE`、すなわち**候補記憶が 100 万件を超える**必要がある。
/// > 想起の作業点は90件前後（ADR-0023）なので、**6桁ほど離れている。**
/// > **テストが無いのは穴ではなく、到達不能だからである。**
/// > （逆に言うと、この `.max(1)` を外す変異は**どんなテストでも赤にできない** ——
/// > CLAUDE.md の「死んだ検査」。定数を大きく下げるなら、ここは生き返る）
/// >
/// > **内側の `.max(1)` は別物で、到達する。** `df = 0`（発言にしか無い gram）を
/// > 1 と読み替える本体であり、外すと `RARITY_SCALE / 0` で**除算 panic する。**
fn weight(frequencies: &HashMap<Gram, u64>, gram: Gram) -> u64 {
    let document_frequency = frequencies.get(&gram).copied().unwrap_or(0).max(1);
    (RARITY_SCALE / document_frequency).max(1)
}

/// 重み付きの gram 総数（多重集合の大きさ）。
fn weighted_total(counts: &GramCounts, frequencies: &HashMap<Gram, u64>) -> u64 {
    counts.iter().fold(0u64, |total, (gram, count)| {
        total.saturating_add(weight(frequencies, *gram).saturating_mul(*count))
    })
}

/// 重み付き Dice 係数。
fn dice(query: &GramCounts, memory: &GramCounts, frequencies: &HashMap<Gram, u64>) -> Relevance {
    // 分母は**両側**の重み付き gram 数の和。片側だけにすると
    // 「短い発言に長い記憶が当たりやすい」方向へ偏る。
    let total =
        weighted_total(query, frequencies).saturating_add(weighted_total(memory, frequencies));

    // 分母が 0 ＝ 両側とも gram を持たない（＝両方が空文字）。
    // **比として定義できないので最小値に畳む**（モジュールの doc の境界表）。
    let Some(denominator) = NonZeroU64::new(total) else {
        return Relevance::MIN;
    };

    // 共通数は**多重集合**として数える（両側の出現回数の小さい方）。
    let common = query.iter().fold(0u64, |common, (gram, in_query)| {
        let shared = memory
            .get(gram)
            .map_or(0, |in_memory| *in_query.min(in_memory));
        common.saturating_add(weight(frequencies, *gram).saturating_mul(shared))
    });

    Relevance {
        // Dice 係数の分子。**係数 2 を落とすと同一の文字列でも 1 にならない。**
        numerator: common.saturating_mul(2),
        denominator,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // 実測から採った発言と記憶（作業単位 15a / 15a-2 の指示に載っていたもの）。
    // **どれも別々の文字列**であることを `the_fixtures_are_all_different_strings` で確かめる。
    // ------------------------------------------------------------------

    /// 発言1。飼っている動物の話。
    const CAT_UTTERANCE: &str = "僕は猫を飼ってるんだけど、葵さんは何か飼ってる？";
    /// 発言2。勤続年数の話。
    const TENURE_UTTERANCE: &str = "葵さんはこのお店で長いの？";
    /// 発言3。飼い猫のいたずらの話。
    const MISCHIEF_UTTERANCE: &str = "コーヒーって猫、いたずらするの？";
    /// 発言4。転職の迷いの話。
    const JOB_UTTERANCE: &str = "新しい仕事、やっぱり迷ってるんだ";

    /// 飼い猫の記憶（実測で抽出された形そのまま。**括弧が入っている**）。
    const CAT_MEMORY: &str = "黒猫（コーヒー）を飼育している";
    /// 転職の記憶。
    const JOB_MEMORY: &str = "新しい仕事について迷っている";
    /// 勤続の記憶。
    const TENURE_MEMORY: &str = "お店で10年以上勤務している";
    /// 猫の悪癖の記憶。
    const SNACK_MEMORY: &str = "猫が客からお菓子を盗む";
    /// 飼い猫のいたずらの記憶。
    const MISCHIEF_MEMORY: &str = "飼っている猫が毛玉を作り、客から猫の毛と指摘されることがある";
    /// 転職への心構えの記憶。
    const JOB_FEELING_MEMORY: &str =
        "新しい仕事は最初戸惑うが、慣れてくると楽しくなる可能性があると考えている";

    /// **候補集合。** 希少度はこの集合だけから決まる（[`score_relevance`]）。
    ///
    /// 指示は「8件」と書いていたが、4組の高い方・低い方を並べると
    /// `CAT_MEMORY` と `JOB_MEMORY` が2回ずつ出るため、**相異なるのは6件**である。
    /// **重複させると df が二重に数えられて希少度が歪む**ので、集合としては6件を使う。
    const ALL_MEMORIES: [&str; 6] = [
        CAT_MEMORY,
        JOB_MEMORY,
        TENURE_MEMORY,
        SNACK_MEMORY,
        MISCHIEF_MEMORY,
        JOB_FEELING_MEMORY,
    ];

    /// 実測の4組。**「この発言なら、こちらが上に来てほしい」** の一覧。
    const MEASURED_PAIRS: [(&str, &str, &str); 4] = [
        (CAT_UTTERANCE, CAT_MEMORY, JOB_MEMORY),
        (TENURE_UTTERANCE, TENURE_MEMORY, SNACK_MEMORY),
        (MISCHIEF_UTTERANCE, MISCHIEF_MEMORY, JOB_FEELING_MEMORY),
        (JOB_UTTERANCE, JOB_MEMORY, CAT_MEMORY),
    ];

    /// **候補1件**で採点する。境界4により**重みは一様**なので、
    /// 希少度を含まない素の Dice（＝変更A だけ）の値になる。
    fn alone(query: &str, memory: &str) -> Relevance {
        let scores = score_relevance(query, [memory]);
        assert_eq!(scores.len(), 1, "候補1件なら1件返ること");
        scores[0]
    }

    /// [`ALL_MEMORIES`] を候補集合として、その中の1件の点を取る。
    fn among_all(query: &str, memory: &str) -> Relevance {
        let index = ALL_MEMORIES
            .iter()
            .position(|candidate| *candidate == memory)
            .expect("候補集合に含まれる記憶であること");
        let scores = score_relevance(query, ALL_MEMORIES);
        assert_eq!(scores.len(), ALL_MEMORIES.len());
        scores[index]
    }

    /// 変わった入力の見本。**panic しないことを確かめるためのもの。**
    ///
    /// 絵文字（ZWJ で繋がった家族）・結合文字・アストラル面・双方向制御・制御文字・
    /// 空白だけ・1文字・空文字を混ぜてある。
    fn odd_samples() -> Vec<String> {
        vec![
            String::new(),
            "a".to_owned(),
            "𠮷".to_owned(),
            "𠮷野家で𩸽を食べた".to_owned(),
            "👨‍👩‍👧‍👦".to_owned(),
            "🐱🐈🐱🐈🐱".to_owned(),
            "が".to_owned(),
            "か\u{3099}\u{3099}\u{3099}".to_owned(),
            "\u{202e}gnirts desrever\u{202c}".to_owned(),
            "\u{0000}\u{0007}\u{feff}".to_owned(),
            "  \t\n  ".to_owned(),
            "🐱".repeat(500),
            CAT_UTTERANCE.to_owned(),
            CAT_MEMORY.to_owned(),
        ]
    }

    /// すべて異なる文字を `count` 個並べた文字列（CJK 統合漢字の先頭から連番）。
    ///
    /// **文字が重複しないので gram も重複せず**、「N 文字なら gram は `N + (N-1)`
    /// ＝ unigram N 個 ＋ bigram N-1 個」と数えられる。分母を狙って作るために使う。
    fn distinct_chars(count: usize) -> String {
        (0..count)
            .map(|i| u32::try_from(i).expect("テストの長さは u32 に収まること"))
            .filter_map(|i| char::from_u32(0x4e00 + i))
            .collect()
    }

    /// フィクスチャが**同値になっていない**ことの入口検査。
    ///
    /// 「A のほうが B より高い」という検査は、A と B が別物であって初めて意味を持つ
    /// （CLAUDE.md の「フィクスチャが2つの値を同値にすると、その取り違えは検出できない」）。
    #[test]
    fn the_fixtures_are_all_different_strings() {
        for (left_index, left) in ALL_MEMORIES.iter().enumerate() {
            for right in ALL_MEMORIES.iter().skip(left_index + 1) {
                assert_ne!(left, right, "候補集合に同じ記憶が2回入っていないこと");
            }
        }
        for (query, high, low) in MEASURED_PAIRS {
            assert_ne!(high, low, "高い方と低い方が同じ記憶では順序を測れない");
            // 発言と記憶が同一だと「同一なら最大」に化けて順序の検査が無意味になる。
            assert_ne!(query, high);
            assert_ne!(query, low);
        }
        // 4つの発言もすべて別物。
        assert_ne!(CAT_UTTERANCE, TENURE_UTTERANCE);
        assert_ne!(MISCHIEF_UTTERANCE, JOB_UTTERANCE);
        // 最小と最大が同値だと、以下のすべての検査が空振りする。
        assert_ne!(Relevance::MIN, Relevance::MAX);
        assert!(Relevance::MIN < Relevance::MAX);
        // 見本が短すぎて gram を持たない、という取り違えも潰しておく。
        assert!(distinct_chars(4).chars().count() == 4);
    }

    // ------------------------------------------------------------------
    // 境界4種（**unigram を混ぜたので決め直した**）
    // ------------------------------------------------------------------

    /// **境界1: 空文字。** 空文字は gram を1つも持たないので最小値。
    ///
    /// 片側だけが空なら共通 gram が 0、両方空なら分母が 0 で、どちらも最小値。
    #[test]
    fn an_empty_side_scores_the_minimum() {
        assert_eq!(alone("", CAT_MEMORY), Relevance::MIN);
        assert_eq!(alone(CAT_MEMORY, ""), Relevance::MIN);
        // **同一の文字列でも最大にならない**ことを明示的に固定する。
        assert_eq!(alone("", ""), Relevance::MIN);
        assert_ne!(alone("", ""), Relevance::MAX);
        // 候補集合が空なら結果も空（panic せず、空の Vec）。
        assert!(score_relevance(CAT_UTTERANCE, []).is_empty());
    }

    /// **境界2: 共通の gram が無ければ最小値。**
    ///
    /// **unigram を混ぜたので「文字が1つも共通しない」ことが条件になった。**
    /// bigram だけで数えていた頃は `"ab"` と `"ba"` も最小だったが、**いまは違う**
    /// （`a` と `b` を共有する）。その差は
    /// `a_shared_character_now_scores_even_when_no_bigram_matches` が見張る。
    #[test]
    fn strings_without_a_shared_gram_score_the_minimum() {
        assert_eq!(alone("abcd", "wxyz"), Relevance::MIN);
        assert_eq!(alone("猫", "犬"), Relevance::MIN);
        assert_eq!(alone("あいうえ", "かきくけ"), Relevance::MIN);
    }

    /// **境界3: 同一の文字列は最大値。****1文字でも、絵文字でも、候補が何件でも。**
    ///
    /// **ここが変更前と逆になった。** bigram だけの頃は1文字同士は
    /// bigram を作れないので最小値だったが、**unigram があるので1文字でも最大**になる。
    #[test]
    fn identical_strings_score_the_maximum() {
        for sample in [
            CAT_MEMORY,
            TENURE_MEMORY,
            CAT_UTTERANCE,
            "ああああ",
            "🐱🐈🐱🐈🐱",
            "𠮷野家",
            "aa",
            // 1文字（旧・境界2）。
            "猫",
            "𠮷",
            "🐱",
        ] {
            assert_eq!(
                alone(sample, sample),
                Relevance::MAX,
                "同一の文字列は最大値になること: {sample}"
            );
            assert_eq!(alone(sample, sample).per_mille(), 1000);
        }
        // **重み付きでも最大は最大。** 候補集合の中でも同一なら 1000‰。
        assert_eq!(among_all(CAT_MEMORY, CAT_MEMORY), Relevance::MAX);
    }

    /// **境界4: 候補が1件のときは、重みが一様。**
    ///
    /// 候補1件では**すべての gram の df が 1（または 0）**になるので、
    /// 希少度は全 gram で同じ値になり、比の中で約分されて消える。
    /// つまり**候補1件の採点は、希少度を入れる前（変更A だけ）と完全に同じ。**
    ///
    /// これは実装の都合ではなく**決めた規則**である。1件しか無いとき
    /// 「すべてが希少」でも「すべてが平凡」でも順位は付かないので、どちらでもよい。
    /// **どちらでもよいものは固定しておく**（曖昧なまま置かない）。
    #[test]
    fn a_single_candidate_makes_every_weight_uniform() {
        // 候補1件のときは、同じ発言・同じ記憶でも候補集合の中とは値が変わる。
        // （変わらなければ希少度が効いていない ＝ 変更B が入っていない）
        assert_ne!(
            alone(CAT_UTTERANCE, CAT_MEMORY),
            among_all(CAT_UTTERANCE, CAT_MEMORY),
            "候補集合が変われば値も変わること"
        );
        // **一様であることの直接の観測。**
        // `あい` と `あ甲` の共通は unigram `あ` の1つだけ、gram は両側とも3つ。
        // 重みが一様なら、比は**希少度の無い素の Dice `2*1/(3+3) = 2/6` に厳密に一致する。**
        // （比で等しさを決める型なので、`RARITY_SCALE` 倍されていても一致する）
        let unweighted = Relevance {
            numerator: 2,
            denominator: NonZeroU64::new(6).expect("6 は 0 ではない"),
        };
        assert_eq!(alone("あい", "あ甲"), unweighted);
        assert_eq!(alone("あい", "あ甲").per_mille(), 333);

        // **候補が2件以上になれば一様ではなくなる。**
        // 候補3件がどれも `あ` を持つと `あ` だけが軽くなり、同じ組でも値が変わる。
        let skewed = score_relevance("あい", ["あ甲", "あ乙", "あ丙"])[0];
        assert_ne!(skewed, unweighted, "df が上がれば値が変わること");
        assert!(skewed > Relevance::MIN, "0 に潰れた比較になっていないこと");

        // **同じ記憶を重複させても df は上がる**（件数で数えているため）。
        let duplicated = score_relevance(CAT_UTTERANCE, [CAT_MEMORY, CAT_MEMORY, CAT_MEMORY]);
        assert_eq!(duplicated.len(), 3);
        for score in &duplicated {
            assert_eq!(*score, duplicated[0], "同じ記憶どうしは同点");
            assert_ne!(*score, alone(CAT_UTTERANCE, CAT_MEMORY));
        }
    }

    // ------------------------------------------------------------------
    // 変更A — unigram と bigram を1つの多重集合に混ぜる
    // ------------------------------------------------------------------

    /// **文字だけが共通で bigram が1つも一致しない組にも点が付く。**
    ///
    /// これが変更A の目的そのもの。日本語は活用と複合で隣の文字が変わるため
    /// （`猫を` / `猫が`、`飼って` / `飼育`）、bigram だけだと重要語の一致を丸ごと落とす。
    #[test]
    fn a_shared_character_now_scores_even_when_no_bigram_matches() {
        // bigram は (a,b) と (b,a) で一致しないが、unigram の a・b が一致する。
        let reordered = alone("ab", "ba");
        assert!(reordered > Relevance::MIN, "unigram のぶんだけ点が付くこと");
        assert!(
            reordered < Relevance::MAX,
            "bigram が違うので最大にはならない"
        );

        // 実測の例: 発言の `猫を` と記憶の `黒猫` は bigram では一致しないが、
        // unigram の `猫` で一致する。
        let cat = alone("猫を飼っている", "黒猫");
        assert!(cat > Relevance::MIN);
    }

    /// 重複する gram を**多重集合**として数える。集合で数えると値が変わる。
    ///
    /// * `"ああああ"` → `あ ×4` ＋ `(あ,あ) ×3` ＝ 7 / `"あああ"` → `あ ×3` ＋ `(あ,あ) ×2` ＝ 5
    ///   共通 `min(4,3) + min(3,2) = 5` → `2*5/(7+5) = 10/12 = 833‰`
    ///   **集合なら共通 2・分母 4 で 1000‰（＝最大）になる。**
    /// * 逆向き（記憶の方が多い）も同じ規則。`min` がどちらを選ぶ場合も通す。
    #[test]
    fn repeated_grams_are_counted_as_a_multiset() {
        let more_in_query = alone("ああああ", "あああ");
        assert_eq!(more_in_query.per_mille(), 833);
        assert!(more_in_query < Relevance::MAX);

        // "ああ" → あ×2 + (あ,あ)×1 = 3 / "ああああああ" → あ×6 + (あ,あ)×5 = 11
        // 共通 min(2,6)+min(1,5) = 3 → 2*3/(3+11) = 6/14 = 428‰（集合なら 1000‰）。
        let more_in_memory = alone("ああ", "ああああああ");
        assert_eq!(more_in_memory.per_mille(), 428);
        assert!(more_in_memory < more_in_query);
    }

    /// 同じ重なりでも**相手が長いほど低い**（Dice は長さで正規化される）。
    ///
    /// 分母を片側だけにする実装だと、`query` 側の分母は変わらないのでこの差が消える。
    #[test]
    fn a_longer_memory_with_the_same_overlap_scores_lower() {
        let query = "猫が好き";
        let exact = alone(query, "猫が好き");
        let padded = alone(query, "猫が好きだと前に言っていた気がする");
        let repeated = alone(query, "猫が好き猫が好き猫が好き");

        assert!(exact > padded);
        assert!(exact > repeated);
        // どれも最小ではない（全部 0 に潰れた実装でこの検査が空振りしないこと）。
        assert!(padded > Relevance::MIN);
        assert!(repeated > Relevance::MIN);
    }

    // ------------------------------------------------------------------
    // 変更B — 候補集合の中での希少度
    // ------------------------------------------------------------------

    /// **多くの候補が持つ gram は、軽く数える。**
    ///
    /// 決定的なフィクスチャ。`あ` と `い` は、候補が2件のうちは**完全に対等**
    /// （どちらも df 1）なので `あ甲` と `い乙` は**同点**になる。
    /// そこへ **`あ` を持つ候補を3件足す**と `あ` の df だけが 4 に上がり、
    /// **`あ` で当たった記憶が下がる。**
    ///
    /// > **この検査は「その分岐が効く状態」に入っている** ——
    /// > 前半で同点、後半で順序が付く。**同じ発言・同じ記憶で、重みだけが違う。**
    #[test]
    fn a_gram_shared_by_many_candidates_counts_for_less() {
        let by_a = "あ甲";
        let by_i = "い乙";
        let query = "あい";

        // 候補2件: `あ` も `い` も df 1 なので対等 → 同点。
        let balanced = score_relevance(query, [by_a, by_i]);
        assert_eq!(balanced.len(), 2);
        assert_eq!(balanced[0], balanced[1], "df が同じなら同点になること");
        assert!(balanced[0] > Relevance::MIN, "両方 0 では何も測れていない");

        // `あ` を持つ候補を3件足す → `あ` の df が 4、`い` の df は 1 のまま。
        let skewed = score_relevance(query, [by_a, by_i, "あ丙", "あ丁", "あ戊"]);
        assert_eq!(skewed.len(), 5);
        assert!(
            skewed[1] > skewed[0],
            "希少な `い` で当たった方が上に来ること: {:?} vs {:?}",
            skewed[1],
            skewed[0]
        );
        // 足した3件は `by_a` と同じ当たり方なので、`by_a` と同点のまま。
        for shifted in &skewed[2..] {
            assert_eq!(*shifted, skewed[0]);
        }
    }

    /// **`df = 1` と `df = 2` を区別すること** ——「その gram を持つ記憶が1件だけ」という、
    /// **いちばん情報量の多い区別**の見張り番。
    ///
    /// > **変異検査で実測。** `df` を leave-one-out（`df - 1`）にする変異は、
    /// > `weight` の `.max(1)` に吸われて **`df = 1` と `df = 2` の重みを
    /// > どちらも `RARITY_SCALE` に潰す。** それでも
    /// > `a_gram_shared_by_many_candidates_counts_for_less`（df 1 対 4）も
    /// > `rarity_stays_distinguishable_*`（df 60 対 61）も**緑を通り抜けた** ——
    /// > **`df` が 1 と 2 になるフィクスチャがどこにも無かった。**
    ///
    /// **失敗の形**: 記憶90件のうち、鍵となる語が正解の1件にしか出ず、
    /// 近い語が2件に出るとき、**両者が同点になり、上位 M 件の取り合いが
    /// 並び順（`created_at`）で決まる。**
    ///
    /// 候補 `["あ甲", "い乙", "い丙"]` に対し `あ` は df 1、`い` は df 2。
    /// 現行値では 4/11 対 1/5 ＝ **1.82 倍**の差が付く。
    #[test]
    fn a_gram_in_one_candidate_outranks_a_gram_in_two() {
        let scores = score_relevance("あい", ["あ甲", "い乙", "い丙"]);
        assert_eq!(scores.len(), 3);

        // `い乙` と `い丙` は当たり方が同じなので同点（フィクスチャの対称性の確認）。
        assert_eq!(scores[1], scores[2], "df が同じ2件は同点になること");
        // 潰れた比較になっていないこと（両方 0 なら「上」は何も意味しない）。
        assert!(scores[1] > Relevance::MIN);
        assert!(scores[0] < Relevance::MAX);

        assert!(
            scores[0] > scores[1],
            "df 1 の `あ` で当たった方が、df 2 の `い` で当たった方より上に来ること: {:?} vs {:?}",
            scores[0],
            scores[1]
        );
    }

    /// **df が 1 違うだけでも区別が付くこと** ——`RARITY_SCALE` の刻みの見張り番。
    ///
    /// 重みは整数除算 `RARITY_SCALE / df` なので、**`RARITY_SCALE` が小さいと
    /// 近い df どうしの重みが同じ値に丸まり、希少度が静かに効かなくなる。**
    /// この壊れ方は「全部 1 に潰れる」ほど露骨ではないので、**件数が多いときだけ現れる。**
    ///
    /// > **変異検査で実測（M11）。** `1_000_000` を `1_000` に下げる変異は、
    /// > 少件数のフィクスチャしか無かった段階では**緑を通り抜けた。**
    /// > `1_000 / 60` と `1_000 / 61` が**どちらも 16 に丸まる**ため、
    /// > この検査が赤にする。
    ///
    /// 件数（121）は空想ではない —— ADR-0023 の実測で**記憶は 90 件前後で
    /// プロンプト取り分の半分に達する。** そこが本来の作業点である。
    ///
    /// > **⚠ この検査は `df` 60 対 61 の1点しか見ていない。**
    /// > `RARITY_SCALE` を `1_200` にする変異は**ここを緑で通り抜ける**
    /// > （60/61 では差が付くため）が、その値では **`df = 39` で丸めが始まり、
    /// > 作業点の手前で希少度が死ぬ。** 全域を見張るのは
    /// > `rarity_stays_distinguishable_across_the_working_range_of_document_frequencies`。
    /// > **こちらは `score_relevance` を端から端まで通す経路の見張り番として残してある**
    /// > （`document_frequencies` の数え方の誤りは、`weight` を直接叩く検査では見えない）。
    #[test]
    fn rarity_stays_distinguishable_when_document_frequencies_differ_by_one() {
        /// 埋め草を作る。`prefix` ＋ **他とぶつからない1文字。**
        ///
        /// `甲`（U+7532）や `乙`（U+4E59）と衝突しない領域から採る。
        fn filler(prefix: char, index: u32) -> String {
            let unique = char::from_u32(0x5000 + index).expect("この範囲は有効なスカラー値");
            format!("{prefix}{unique}")
        }

        let query = "あい";
        let mut candidates = vec!["あ甲".to_owned(), "い乙".to_owned()];
        // `あ` の df を 60、`い` の df を 61 にする（自分自身の1件を含めて数える）。
        for index in 0..59 {
            candidates.push(filler('あ', index));
        }
        for index in 0..60 {
            candidates.push(filler('い', 100 + index));
        }
        assert_eq!(candidates.len(), 121);

        let scores = score_relevance(query, candidates.iter().map(String::as_str));
        assert_eq!(scores.len(), 121);
        // `あ` の方が df が小さい（60 < 61）＝ わずかに希少 ＝ わずかに上。
        assert!(
            scores[0] > scores[1],
            "df が 1 違えば順序が付くこと（刻みが粗いと同点に丸まる）: {:?} vs {:?}",
            scores[0],
            scores[1]
        );
        // 潰れた比較になっていないこと。
        assert!(scores[1] > Relevance::MIN);
        assert!(scores[0] < Relevance::MAX);
    }

    /// **見張る `df` の上限。** 想起の作業点（記憶90件前後 / ADR-0023）を確実に含める。
    ///
    /// 90 ちょうどではなく 150 まで見るのは、**作業点が上へずれても効く余裕**を持たせるため。
    const MAX_WORKING_DF: u64 = 150;

    /// **`df` 1〜150 の全域で、隣り合う `df` の重みが相異なること。**
    ///
    /// 上の `rarity_stays_distinguishable_when_document_frequencies_differ_by_one` は
    /// **`df` 60 対 61 という1点しか見ていない。** 変異検査で実測 ——
    ///
    /// | `RARITY_SCALE` | 60/61 の検査 | 丸めが始まる `df` |
    /// |---|---|---|
    /// | `10_000` | **緑を通過** | 110 |
    /// | `1_200` | **緑を通過** | **39** |
    /// | 現行 `1_000_000` | — | 1032 |
    ///
    /// **`1_200` は M11 が捕まえた `1_000` のわずか 1.2 倍**だが、60/61 では差が付くので
    /// 通ってしまう。そしてその値では **`df = 39` で丸めが始まる** ——
    /// **作業点（90件前後）より手前で希少度が静かに死ぬ。**
    ///
    /// > **関数の外にある値で押さえる**（CLAUDE.md）。1つめの主張は
    /// > **`weight` を1度も呼ばず、`RARITY_SCALE` と `df` だけ**から
    /// > 十分条件 `df·(df+1) ≤ RARITY_SCALE` を検査する。
    /// > **`weight` が緩めば期待値も一緒に動く、という共倒れを避けるため。**
    /// > 2つめの主張で、実際に `weight` がその全域で狭義単調減少することを見る。
    #[test]
    fn rarity_stays_distinguishable_across_the_working_range_of_document_frequencies() {
        let gram = Gram::Unigram('あ');
        let mut previous: Option<u64> = None;

        for document_frequency in 1..=MAX_WORKING_DF {
            // 主張1: 十分条件を**定数そのもの**に対して検査する（`weight` を1度も呼ばない）。
            // `S/df - S/(df+1) = S/(df*(df+1)) >= 1` なら、切り捨てても必ず 1 以上離れる。
            assert!(
                document_frequency * (document_frequency + 1) <= RARITY_SCALE,
                "df·(df+1) <= RARITY_SCALE を満たすこと（df = {document_frequency} / \
                 RARITY_SCALE = {RARITY_SCALE}）: 満たさない値では作業点の手前で重みが丸まる"
            );

            // 主張2: `weight` が実際にその全域で狭義単調減少すること。
            let mut frequencies: HashMap<Gram, u64> = HashMap::new();
            frequencies.insert(gram, document_frequency);
            let current = weight(&frequencies, gram);
            if let Some(heavier) = previous {
                assert!(
                    heavier > current,
                    "df = {} と df = {document_frequency} の重みが同じ値に丸まっている（{heavier}）",
                    document_frequency - 1
                );
            }
            previous = Some(current);
        }

        // 下限の 1 に張り付いていない ＝ 全部が同じ値に潰れた状態を「単調」と読んでいない。
        assert!(previous.is_some_and(|lightest| lightest > 1));
    }

    /// **同じ発言・同じ記憶でも、候補集合が変われば点が変わる。**
    ///
    /// 希少度が入っていない実装（変更A だけ）では、この検査は落ちる。
    #[test]
    fn the_same_pair_scores_differently_under_a_different_candidate_set() {
        let narrow = score_relevance(CAT_UTTERANCE, [CAT_MEMORY, TENURE_MEMORY]);
        let wide = score_relevance(CAT_UTTERANCE, ALL_MEMORIES);

        assert_ne!(
            narrow[0], wide[0],
            "候補集合が変われば同じ記憶の点も変わること"
        );
        // どちらも潰れていない（0 同士・1000 同士の比較になっていない）。
        for score in [narrow[0], wide[0]] {
            assert!(score > Relevance::MIN);
            assert!(score < Relevance::MAX);
        }
    }

    /// 返る点は**候補と同じ順序・同じ件数。** 並べ替えても、点も同じように並べ替わる。
    ///
    /// 添字を取り違える実装・内部で並べ替えてしまう実装をここで捕まえる。
    ///
    /// > **⚠ 「逆順にしたら逆順になる」だけでは足りない。** 変異検査で実測（M9）——
    /// > **出力を丸ごと逆順にする実装は、その検査だけでは緑を通り抜ける**
    /// > （両方の呼び出しが同じだけずれるので打ち消し合う）。
    /// > **候補の中に発言そのものを混ぜて、`MAX` がどの添字に来るかを直接見る**
    /// > 検査を先に置いてある。**位置は先頭**（逆順にすると末尾へ動く長さ4の並び）。
    #[test]
    fn the_scores_come_back_in_the_input_order() {
        // 発言そのものを**先頭**に置く。同一なので、その1件だけが `MAX` になる。
        let marked = score_relevance(
            CAT_UTTERANCE,
            [CAT_UTTERANCE, JOB_MEMORY, TENURE_MEMORY, SNACK_MEMORY],
        );
        assert_eq!(marked.len(), 4);
        assert_eq!(marked[0], Relevance::MAX, "発言と同一の候補は先頭のまま");
        for other in &marked[1..] {
            assert!(*other < Relevance::MAX, "同一でない候補は最大にならない");
        }

        let forward = score_relevance(CAT_UTTERANCE, ALL_MEMORIES);
        let mut reversed_input = ALL_MEMORIES;
        reversed_input.reverse();
        let backward = score_relevance(CAT_UTTERANCE, reversed_input);

        assert_eq!(forward.len(), ALL_MEMORIES.len());
        assert_eq!(backward.len(), ALL_MEMORIES.len());
        for (index, score) in forward.iter().enumerate() {
            assert_eq!(
                *score,
                backward[ALL_MEMORIES.len() - 1 - index],
                "候補を逆順にしたら点も逆順になること"
            );
        }
        // **点がすべて同じでは、並び順の取り違えを検出できない。**
        assert_ne!(forward[0], forward[1]);
    }

    // ------------------------------------------------------------------
    // 対称性 — **重みを固定すれば対称。候補集合をまたぐと対称ではない**
    // ------------------------------------------------------------------

    /// **重み関数が同じなら対称。** 候補1件どうしは重みが一様なので対称。
    ///
    /// **最小・最大に潰れた組だけで確かめると、何も測っていないことになる。**
    /// 中間の値を持つ組が最低1つあることを同じテストで確かめる。
    #[test]
    fn the_score_is_symmetric_when_the_weights_are_the_same() {
        let samples: Vec<String> = odd_samples()
            .into_iter()
            .chain([
                JOB_MEMORY.to_owned(),
                TENURE_UTTERANCE.to_owned(),
                TENURE_MEMORY.to_owned(),
                "ああああ".to_owned(),
                "あああ".to_owned(),
            ])
            .collect();

        let mut saw_intermediate = false;
        for a in &samples {
            for b in &samples {
                let forward = alone(a, b);
                let backward = alone(b, a);
                assert_eq!(forward, backward, "対称であること: {a:?} / {b:?}");
                assert_eq!(forward.per_mille(), backward.per_mille());
                if forward > Relevance::MIN && forward < Relevance::MAX {
                    saw_intermediate = true;
                }
            }
        }
        assert!(
            saw_intermediate,
            "最小でも最大でもない組が1つも無いなら、対称性を測れていない"
        );
    }

    /// **候補集合をまたぐと対称ではない。** 希少度を入れた結果として失われた性質。
    ///
    /// `あい` を発言・`あう` を記憶として測るときと、役割を入れ替えたときとでは、
    /// **候補集合が違うので `あ` の df が違い、点が変わる。**
    #[test]
    fn the_score_is_not_symmetric_across_different_candidate_sets() {
        // 候補側に `あ` を持つ記憶が多い → `あ` は軽い。
        let forward = score_relevance("あい", ["あう", "あ丙", "あ丁"])[0];
        // 候補側に `い` を持つ記憶が多い → `あ` は重いまま。
        let backward = score_relevance("あう", ["あい", "い丙", "い丁"])[0];

        assert_ne!(
            forward, backward,
            "候補集合が違えば、同じ2つの文字列でも点は一致しない"
        );
        // どちらも潰れていない（MIN 同士・MAX 同士を比べていない）。
        for score in [forward, backward] {
            assert!(score > Relevance::MIN);
            assert!(score < Relevance::MAX);
        }
    }

    // ------------------------------------------------------------------
    // 順序 — 実測で出た4組
    // ------------------------------------------------------------------

    /// **実測の4組が、期待した順序になること。**
    ///
    /// **絶対値では固定しない**（採点の中身を変えたら全部書き直しになる）。
    /// 順序と、「どちらも 0 に潰れていない」ことだけを見る。
    ///
    /// 希少度を効かせるため、**候補集合は6件すべて**（[`ALL_MEMORIES`]）。
    #[test]
    fn every_measured_utterance_prefers_its_own_memory() {
        for (query, expected_high, expected_low) in MEASURED_PAIRS {
            let high = among_all(query, expected_high);
            let low = among_all(query, expected_low);

            assert!(
                high > low,
                "{query} → {expected_high} が {expected_low} より上に来ること: {high:?} vs {low:?}"
            );
            // **両方 0 なら順序を測れていない**（低い方も機能語で必ず重なる）。
            assert!(low > Relevance::MIN, "低い方も 0 ではないこと: {query}");
            assert!(high < Relevance::MAX, "高い方が同一文字列ではないこと");
        }
    }

    /// **旧 `the_measured_cat_memory_loses_to_an_unrelated_one` の置き換え。**
    ///
    /// 作業単位 15a では、発言「僕は猫を飼ってるんだけど、葵さんは何か飼ってる？」に対し
    /// **実測で抽出された `CAT_MEMORY` が `JOB_MEMORY` に負けていた。**
    /// その事実を写した characterization test を置いていたが、**15a-2 で直った**ので、
    /// 「いまこうである」ではなく**「こうあるべき」を主張する検査に置き換えた。**
    ///
    /// | | 変更前（bigram のみ） | 変更A（unigram を混ぜる） | 変更A＋B（希少度） |
    /// |---|---|---|---|
    /// | 黒猫（コーヒー）を飼育している | 54‰ | **157‰** | **85‰** |
    /// | 新しい仕事について迷っている | 55‰ | 135‰ | 59‰ |
    /// | 順序 | **逆転（NG）** | OK | OK |
    ///
    /// **逆転を直したのは変更A である。** 変更B は順序を変えていないが、
    /// 差の比を 1.16 倍から 1.44 倍へ広げている（モジュールの doc を読むこと）。
    #[test]
    fn the_measured_cat_memory_now_beats_the_unrelated_one() {
        let intended_high = among_all(CAT_UTTERANCE, CAT_MEMORY);
        let intended_low = among_all(CAT_UTTERANCE, JOB_MEMORY);

        assert!(
            intended_high > intended_low,
            "猫の記憶が上に来ること: {intended_high:?} vs {intended_low:?}"
        );
        // **変更A だけでも逆転は直る**ことを、同じ検査の中で見る
        // （候補1件 ＝ 重みが一様 ＝ 変更A だけ、が境界4）。
        let without_rarity_high = alone(CAT_UTTERANCE, CAT_MEMORY);
        let without_rarity_low = alone(CAT_UTTERANCE, JOB_MEMORY);
        assert!(without_rarity_high > without_rarity_low);

        // **変更前の 54‰ / 55‰ には戻っていない。**
        assert!(without_rarity_high.per_mille() > 100);
        assert!(intended_high.per_mille() > 60);
    }

    // ------------------------------------------------------------------
    // 順序の厳密さ
    // ------------------------------------------------------------------

    /// **丸めた値が同じでも、比が違えば順序が付く。**
    ///
    /// `per_mille` で比較する実装（丸めてから比べる）だとこの検査が落ちる。
    #[test]
    fn two_scores_that_round_to_the_same_per_mille_still_order_correctly() {
        let shorter = distinct_chars(1002); // gram 1002 + 1001 = 2003 個
        let longer = distinct_chars(1003); // gram 1003 + 1002 = 2005 個
        let query: String = shorter.chars().take(2).collect(); // gram 3 個・すべて共通

        let high = alone(&query, &shorter); // 6/2006
        let low = alone(&query, &longer); // 6/2008

        assert_eq!(
            high.per_mille(),
            low.per_mille(),
            "丸めると同じ値になること"
        );
        assert_eq!(high.per_mille(), 2);
        assert!(high > low, "丸める前の比では差が付くこと");
        assert_ne!(high, low);
    }

    /// **等しさは表現ではなく比で決まる。** `2/4` と `6/12` は等しい。
    #[test]
    fn equal_ratios_compare_equal_even_with_different_representations() {
        // query 1文字（gram 1）× memory 2文字（gram 3）→ 共通 1 → 2/4
        let half_small = alone(&distinct_chars(1), &distinct_chars(2));
        // query 2文字（gram 3）× memory 5文字（gram 9）→ 共通 3 → 6/12
        let half_large = alone(&distinct_chars(2), &distinct_chars(5));

        // **表現は違う**（同値のフィクスチャで検査を空振りさせない）。
        assert_ne!(half_small.numerator, half_large.numerator);
        assert_ne!(half_small.denominator, half_large.denominator);
        // それでも比としては等しい。
        assert_eq!(half_small, half_large);
        assert_eq!(half_small.per_mille(), 500);
        assert_eq!(half_large.per_mille(), 500);
    }

    /// `per_mille` は**切り捨て**で、最小は 0、最大は 1000。
    #[test]
    fn per_mille_is_the_floor_of_the_ratio() {
        assert_eq!(Relevance::MIN.per_mille(), 0);
        assert_eq!(Relevance::MAX.per_mille(), 1000);

        let query: String = distinct_chars(1002).chars().take(2).collect();
        // 6/2006 = 2.99…‰ → 切り捨てて 2。
        assert_eq!(alone(&query, &distinct_chars(1002)).per_mille(), 2);
    }

    // ------------------------------------------------------------------
    // panic しないこと / 文字の扱い
    // ------------------------------------------------------------------

    /// **gram は Unicode スカラー値であってバイトではない。**
    ///
    /// `𠮷`（F0 A0 AE B7）と `𩸽`（F0 A9 B8 BD）は**先頭バイトを共有する**ので、
    /// バイトで数える実装なら 0 にならない。**文字で数えれば共通は無い。**
    /// `🐱`（F0 9F 90 B1）と `🐈`（F0 9F 90 88）は**3バイトも共有する**ので、
    /// バイト unigram でも bigram でも当たってしまう。
    #[test]
    fn grams_are_scalar_values_not_bytes() {
        assert_eq!(alone("𠮷", "𩸽"), Relevance::MIN);
        assert_eq!(alone("🐱", "🐈"), Relevance::MIN);
        assert_eq!(alone("🐱🐱", "🐈🐈"), Relevance::MIN);
        // 同じ文字なら 1文字でも最大（境界3）。
        assert_eq!(alone("𠮷", "𠮷"), Relevance::MAX);
        assert_eq!(alone("👨‍👩‍👧‍👦", "👨‍👩‍👧‍👦"), Relevance::MAX);
    }

    /// **Unicode 正規化はしない。** 合成済みと結合文字列は別物として扱う。
    #[test]
    fn no_unicode_normalization_is_applied() {
        // `が`（U+304C）と `か` ＋ U+3099 は文字を1つも共有しない。
        assert_eq!(alone("が", "か\u{3099}"), Relevance::MIN);
        // 大小文字も畳まない。
        assert_eq!(alone("AB", "ab"), Relevance::MIN);
    }

    /// **変わった入力でも panic せず、範囲に収まる。**
    ///
    /// バイト添字のスライスを作る実装は文字境界で panic するので、ここで落ちる。
    /// **見本を丸ごと候補集合にする**ので、希少度の経路も同じ入力を通る。
    #[test]
    fn unusual_unicode_never_panics_and_stays_within_the_range() {
        let samples = odd_samples();
        let candidates: Vec<&str> = samples.iter().map(String::as_str).collect();

        for a in &samples {
            // 候補集合ぜんぶを一度に採点する（重みが効く経路）。
            let scores = score_relevance(a, candidates.iter().copied());
            assert_eq!(scores.len(), candidates.len());
            for score in &scores {
                assert!(*score >= Relevance::MIN, "最小を下回らないこと: {a:?}");
                assert!(*score <= Relevance::MAX, "最大を上回らないこと: {a:?}");
                assert!(score.per_mille() <= 1000);
            }
            // 1件ずつでも同じ範囲に収まる。
            for b in &samples {
                let score = alone(a, b);
                assert!(score >= Relevance::MIN);
                assert!(score <= Relevance::MAX);
            }
        }
    }

    /// 上限いっぱいの記憶（`MemoryContent::MAX_CHARS` = 500 文字）でも壊れない。
    #[test]
    fn a_memory_at_its_length_limit_is_handled() {
        let long_memory = "猫".repeat(crate::domain::MemoryContent::MAX_CHARS);
        let score = alone(CAT_UTTERANCE, &long_memory);

        assert!(score > Relevance::MIN, "`猫` を共有するので 0 ではない");
        assert!(score < Relevance::MAX);
        // 同じ長さの発言と突き合わせても最大を超えない。
        assert_eq!(alone(&long_memory, &long_memory), Relevance::MAX);
    }

    // ------------------------------------------------------------------
    // 「選別側は千分率を呼ばない」の走査（作業単位 15b）
    //
    // 規約の本体はモジュール doc「差し替え可能にしてある形」にある。
    // ここはそれを**実行可能**にする網で、`store::memory` の
    // `no_statement_updates_or_deletes_a_memory` と同じ作法。
    // ------------------------------------------------------------------

    /// 走査対象の根。**`src` 全体**（`prompt` に閉じない）。
    ///
    /// 選別は将来どこにでも書かれうる（`server` がスコアで並べ替える等）ので、
    /// **モジュールを絞ると1ファイル足すだけで網の外に出られる。**
    const SCAN_ROOT: &str = "src";

    /// **走査から外す唯一のファイル**（＝この節を定義しているファイル）。
    ///
    /// 定義と doc とテストがここに集まっているので、除外しないと必ず自分に当たる。
    /// **除外を増やさないこと。** 増やした分だけ網が緩む。
    const SCORING_MODULE: [&str; 2] = ["prompt", "relevance.rs"];

    /// 走査が探す綴り（＝ログ用の入口の名前）。
    ///
    /// **走査本体と対照テストで同じ定数を見ること。** 15b のレビューで、
    /// 両方が `let needle = "per_mille";` と**別々に綴っていた**ため、
    /// **走査本体の綴りだけを壊す変異が全緑で通り抜けた**
    /// ——対照テストは自分の写しを見ていたので「探せている」と言い続けた。
    /// `SCAN_ROOT` / `SCORING_MODULE` と同じく、写しを作らない。
    const LOGGING_ACCESSOR: &str = "per_mille";

    /// `SCAN_ROOT` 配下の `.rs` を「パス, 生ソース」で返す。
    fn scanned_sources() -> Vec<(std::path::PathBuf, String)> {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        let mut stack = vec![manifest.join(SCAN_ROOT)];

        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir).expect("走査対象を読めること");
            for entry in entries {
                let path = entry.expect("ディレクトリ項目を読めること").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !path.extension().is_some_and(|extension| extension == "rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("ソースを読めること");
                files.push((path, source));
            }
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        files
    }

    /// このファイル自身か。
    fn is_the_scoring_module(path: &std::path::Path) -> bool {
        let expected: std::path::PathBuf = SCORING_MODULE.iter().collect();
        path.ends_with(expected)
    }

    /// 走査の範囲。**`bool` の引数にしない**（呼び出し側で真偽の意味が読めない）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Scope {
        /// このファイル自身を除く。**規約の見張り番はこちら。**
        WithoutTheScoringModule,
        /// 何も除かない。**対照はこちら**（このファイルは必ず綴っている）。
        Everything,
    }

    /// [`LOGGING_ACCESSOR`] を綴っているファイルを探す。戻り値は `(見たファイル数, 見つけたパス)`。
    ///
    /// **走査の本体はここ1つだけ。** 見張り番も対照も**この関数を通す**こと。
    ///
    /// # ソースを引数で受ける理由 —— **対照が実ファイルを読むと自己成就する**
    ///
    /// 15b の是正で実測: 走査本体の `contains` に別の綴りを直接書く変異を当てると、
    /// **その綴りは走査本体のあるファイル（＝このファイル）に書かれる**ので、
    /// **`src` を読む対照は「1件見つけた」と言って緑のまま通った。**
    /// 対照は**植えたソース**（[`planted_sources`]）を渡し、
    /// **見つかるはずの1件と見つからないはずの1件**で本体を検算する。
    fn files_spelling_the_accessor(
        sources: &[(std::path::PathBuf, String)],
        scope: Scope,
    ) -> (usize, Vec<String>) {
        assert!(
            sources.len() >= 5,
            "走査対象が少なすぎるなら、この走査は何も守っていない: {}",
            sources.len()
        );
        assert!(
            sources.iter().any(|(path, _)| is_the_scoring_module(path)),
            "除外するはずのファイルが走査対象に含まれていない（走査が壊れている）"
        );

        let mut scanned = 0;
        let mut found = Vec::new();
        for (path, source) in sources {
            if scope == Scope::WithoutTheScoringModule && is_the_scoring_module(path) {
                continue;
            }
            scanned += 1;
            if source.contains(LOGGING_ACCESSOR) {
                found.push(path.display().to_string());
            }
        }

        (scanned, found)
    }

    /// 対照のために**植えた**ソース一式。**実ファイルは読まない。**
    ///
    /// 綴りは引数で受け取り、`LOGGING_ACCESSOR` を直接は書かない
    /// ——**写しを作ると、綴りを壊す変異が対照ごと道連れになって緑になる。**
    ///
    /// | 植えたパス | 中身 | 走査の期待 |
    /// |---|---|---|
    /// | `src/prompt/relevance.rs` | 定義 | **除外される**（`Everything` なら見つかる） |
    /// | `src/server/chat.rs` | **呼び出しの形** | **見つかる** |
    /// | `src/prompt/recall.rs` | `Ord` での比較 | 見つからない |
    /// | 他2件 | 無関係 | 見つからない |
    ///
    /// > **⚠ ここのリテラルに `memory` という語を入れないこと。**
    /// > `store::memory` の走査（`no_statement_updates_or_deletes_a_memory`）は
    /// > **`src/**` の文字列リテラルからテーブル名の言及を拾う**ので、
    /// > 植えたパスに `memory.rs` と書いた時点で**そちらが赤くなる**（是正中に実際に踏んだ）。
    fn planted_sources(spelling: &str) -> Vec<(std::path::PathBuf, String)> {
        vec![
            (
                ["src", "prompt", "relevance.rs"].iter().collect(),
                format!("pub fn {spelling}(self) -> u32 {{ 0 }}"),
            ),
            (
                ["src", "server", "chat.rs"].iter().collect(),
                format!("if score.{spelling}() >= 150 {{ /* 採用 */ }}"),
            ),
            (
                ["src", "prompt", "recall.rs"].iter().collect(),
                "if *score > Relevance::MIN { /* 採用 */ }".to_owned(),
            ),
            (
                ["src", "domain", "character.rs"].iter().collect(),
                "pub struct Persona(String);".to_owned(),
            ),
            (
                ["src", "store", "session.rs"].iter().collect(),
                "pub(crate) fn append_turn() {}".to_owned(),
            ),
        ]
    }

    /// **選別側は [`Relevance::per_mille`] を呼ばない**（15a のレビューで確定した規約）。
    ///
    /// `per_mille` は `pub` なので、`if score.per_mille() >= 150 { 採用 }` と
    /// **閾値を書ける。型は止めていない。**
    /// 埋め込みのコサイン類似度へ差し替えた瞬間、**その閾値は黙って別の意味になる**
    /// （同じ 150 が「まあ関連あり」から「ほぼ無関係」へ動きうる）。
    /// 選別は [`Ord`] だけで行う——これが**規約であって型の保証ではない**以上、
    /// 実行可能な網を1枚張っておく。
    ///
    /// # この走査が守れる範囲
    ///
    /// **`src/**` のうちこのファイル以外に、その綴りが1文字でも現れたら落ちる。**
    /// 呼び出しでも、関数として取り出す（`let f = Relevance::per_mille;`）のでも、
    /// `use` でも、コメントでも同じ。
    /// **Rust にはメソッド名を実行時の文字列から呼ぶ手段が無い**ので、
    /// **呼び出し側のソースに綴りを書かずに実行時に解決する**経路は存在しない。
    ///
    /// # 守れないもの（把握したうえで残している）
    ///
    /// **「塞いだ」ではない。** 次はすべて素通りする——
    ///
    /// * **このファイルの中に選別を書くこと。** 走査の外側そのもの。
    /// * **別名で再公開すること。** ここに
    ///   `pub fn score(self) -> u32 { self.per_mille() }` を足せば、
    ///   外側は綴らずに数値を得られる。**本体は「数値を出す入口を増やさない」ことで、
    ///   走査が見ているのはその一部でしかない。**
    /// * **このファイルの中で `macro_rules!` を定義してエクスポートすること。**
    ///   **マクロ展開はコンパイル時なので、「実行時の文字列から呼べない」の論拠の外側にある。**
    ///   `macro_rules! score_of { ($r:expr) => { $r.per_mille() } }` を
    ///   `#[macro_export]` すれば、**呼び出し側は綴らずに千分率を得られ、走査は素通りする。**
    ///   （前項の「別名で再公開」と同根で、**どちらもこのファイルに入口を足す**形。
    ///   走査は外側しか見ていない。）
    /// * **`Debug` から読むこと。** `format!("{score:?}")` は分子と分母をそのまま印字する
    ///   （モジュール doc に明記してある既知の穴）。
    /// * **`Ord` で書く閾値。** `score > 何かの基準値` は掛からないし、掛けるべきでもない
    ///   （選別を `Ord` で行うのが規約）。**ただし「基準スコアを作って比べる」は
    ///   閾値と同じ危険を持ちうる。**
    /// * **`tests/` 配下。** 統合テストは選別を書く場所ではない。
    ///
    /// # [`Relevance::MIN`] との比較を許している理由 —— **効き目までは保たれない**
    ///
    /// `score > Relevance::MIN`（`super::recall` の層3の絞り込み）だけは許している。
    /// ただし**保たれるのはラベルの意味であって、選別としての効き目ではない**
    /// （15b のレビューで訂正。ADR-0023「層3の絞り込みは、埋め込みへ移すと効き目を失う」）——
    ///
    /// * **ラベルの意味は保たれる。** どの尺度でも `MIN` は「いちばん関連が薄い」を指す。
    /// * **効き目は保たれない。** いまの語の重なりでは
    ///   **候補のかなりの割合が実際に `MIN` を取る**（文字が1つも重ならない記憶は珍しくない）。
    ///   **コサイン類似度では、最小値をぴったり取る候補はまず存在しない。**
    ///   差し替えた瞬間、**この絞り込みは無条件 true と同じになる。**
    ///   `super::recall` のコード直上のコメントが警告している
    ///   **「層3が枠を埋めるためだけに無関係な記憶を拾う」状態が、静かに実現する。**
    ///
    /// **埋め込みへ移すときは、絞り込みの条件も一緒に決め直すこと。**
    #[test]
    fn the_selection_side_never_spells_the_logging_accessor() {
        let (scanned, found) =
            files_spelling_the_accessor(&scanned_sources(), Scope::WithoutTheScoringModule);

        assert!(
            scanned >= 4,
            "このファイル以外を1つも見ていない（走査が壊れている）: {scanned} 件"
        );
        assert!(
            found.is_empty(),
            "選別側から千分率を呼んでいます（`Ord` で比べること / モジュール doc）:\n{}",
            found.join("\n")
        );
    }

    /// **走査本体が現に綴りを見つけられる**ことの対照。
    ///
    /// 「1件も見つからないから緑」と「そもそも探せていないから緑」を区別する。
    ///
    /// # 15b のレビューで通り抜けた形と、是正で分かったこと
    ///
    /// レビュー時点では、見張り番と対照が
    /// **`let needle = "per_mille";` と別々に綴りを持っていた**ため、
    /// **走査本体の綴りだけを壊す変異が全緑で通り抜けた**
    /// ——対照は自分の写しを見て「探せている」と言い続けた。
    ///
    /// **是正で分かったのは、綴りを1本にするだけでは足りないこと**（実測 / M2）。
    /// 本体の `contains` に別の綴りを直接書く変異を当てると、
    /// **その綴りは本体のあるファイル（＝このファイル）に書かれる**ので、
    /// **実ファイルを読む対照は「1件見つけた」と言って緑のまま通る。**
    /// **対照は実ファイルではなく [`planted_sources`] を読むこと。**
    ///
    /// # ここで塞いでいる4つ
    ///
    /// 1. **綴りが本物のメソッド名を指しているか。** 定数だけを書き換える変異は
    ///    「1件も見つからない」形になって見張り番を緑のまま通す。
    ///    **定数の宣言自身がこのファイルにあるので、「見つかった」だけでは何も言えない。**
    /// 2. **植えた呼び出しを、見張り番と同じ本体が見つけるか**（綴りを壊す変異）。
    /// 3. **`Ord` で書いた行を見つけないか**（無条件 true への変異）。
    /// 4. **除外がちょうど1ファイル分だけ効くか**（`is_the_scoring_module` の変異）。
    #[test]
    fn the_scan_would_notice_the_spelling() {
        // 1. 綴りが本物のメソッド名を指していること。
        let real_sources = scanned_sources();
        let scoring_module = real_sources
            .iter()
            .find(|(path, _)| is_the_scoring_module(path))
            .expect("このファイル自身を読めること");
        assert!(
            scoring_module
                .1
                .contains(&format!("pub fn {LOGGING_ACCESSOR}(")),
            "その綴りのログ用の入口が実在しない（走査の綴りが古い）: {LOGGING_ACCESSOR}"
        );

        // 2 と 3. 植えたソースを、見張り番と同じ本体に通す。
        let planted = planted_sources(LOGGING_ACCESSOR);
        let (scanned, found) =
            files_spelling_the_accessor(&planted, Scope::WithoutTheScoringModule);
        assert_eq!(scanned, planted.len() - 1, "除外が1件分だけ効くこと");
        assert_eq!(
            found.len(),
            1,
            "植えた呼び出しをちょうど1件見つけること（探し方が壊れている）: {found:?}"
        );
        assert!(
            found[0].ends_with("chat.rs"),
            "見つけたのが呼び出しを植えたファイルではない: {found:?}"
        );

        // 4. 除外を外すと、定義を植えた1件も増える。
        let (scanned_all, found_all) = files_spelling_the_accessor(&planted, Scope::Everything);
        assert_eq!(scanned_all, planted.len(), "除外なしでは全件を見ること");
        assert_eq!(
            found_all.len(),
            2,
            "除外を外したのに見つかる件数が増えていない: {found_all:?}"
        );
    }
}
