//! 想起の選別（ADR-0023 の**層2「直近」**と**層3「関連」**）。
//!
//! **I/O を持たない純粋関数**（CLAUDE.md 3節）。DB も上流も触らず、
//! 「今回の発言」「候補の一覧」「記憶に回してよい枠」だけから
//! **どれを載せるか**と**載らなかった件数**を決める。
//!
//! # 層1（身元）はここに来ない
//!
//! ADR-0023 の層1は `character_attribute` で、**削れない枠**に載る
//! （`super::render_system` の「# 属性」節）。この関数は**呼ばれもしない。**
//! 属性が予算で削れないのは「ここで守っている」からではなく、
//! **そもそも選別の対象になっていない**からである。
//! 見張り番は `super::tests::attributes_are_never_trimmed_even_when_memories_and_history_are`。
//!
//! # 順序 —— **採点は分割の前に、候補の全件に対して1回だけ**
//!
//! ```text
//! 1. 有効な記憶を全件そろえ、利用者についての記憶だけを候補にする
//!    （`store` は今までどおり両方の主語を全件返す。絞るのは `super::build_prompt` / A-11a）
//! 2. その全件を候補集合として、1回だけ採点する      ← ADR-0023 の決定
//! 3. 層2（直近＝新しい順）と層3（関連＝スコア順）に分ける
//! 4. 同じ記憶が両方に該当したら1回だけ数える
//! ```
//!
//! **2 と 3 を入れ替えないこと。** 層2で採った分を候補集合から除いてから採点すると、
//! **`df` が変わって除いた記憶が持っていた gram が「希少」に化け、残りものが不当に上がる。**
//! 見張り番は `the_score_is_taken_over_every_candidate_before_the_split`
//! （**除いてから採点する実装では、選ばれる記憶が実際に入れ替わる**ことを固定している）。
//!
//! # 配分 —— **半々。片方が余ったらもう片方が使う**
//!
//! 層2が枠を全部食うと、それは **FIFO** であり ADR-0003 が
//! 「商用の原価制約から来た妥協」として却下したものに戻る。
//! そこで**層2の先取り分を [`RECENT_SHARE_NUMERATOR`] / [`RECENT_SHARE_DENOMINATOR`] に
//! 制限**し、層3を採ったあとで**余りを層2へ返す。**
//!
//! **比率は測ってから動かす**（ADR-0023「配分と数え方」）。いまは半々。
//!
//! # 層2と層3で「打ち切り方」が違う
//!
//! | 層 | 並べ方 | 入らない1件に当たったら |
//! |---|---|---|
//! | 層2（直近） | 新しい順 | **打ち切る**（`break`） |
//! | 層3（関連） | スコアの高い順 | **飛ばして次を試す**（`continue`） |
//!
//! **層2を飛ばし読みにしないこと。** 飛ばすと時系列が歯抜けになり、
//! モデルが辻褄合わせを始める（`super::budget` の `take_newest` と同じ理由）。
//! **層3は時系列で並んでいない**ので、そもそも歯抜けという概念が無い。
//! 大きい1件で打ち切ると、その後ろの小さくて関連の高い記憶が理由なく落ちる。
//!
//! # `Relevance` の扱い —— **[`Ord`] だけを使う**
//!
//! 千分率を取り出す入口（ログ用）は**選別側から呼ばない。**
//! 呼ぶと閾値を書けてしまい、**埋め込みへ差し替えたときにその閾値は黙って別の意味になる**
//! （`super::relevance` のモジュール doc「差し替え可能にしてある形」）。
//! **このファイルにその綴りが現れたら走査が落とす**ので、doc にも綴らない。
//! 見張り番は `super::relevance` の
//! `the_selection_side_never_spells_the_logging_accessor`。
//!
//! [`Relevance::MIN`] との比較は `Ord` なので使ってよい。**「関連が最小」＝
//! 発言と文字が1つも重ならない**ことなので、そこに層3の意味は無い（下記）。

use std::cmp::Reverse;

use super::relevance::{Relevance, score_relevance};

/// 層2（直近）が**先に**使ってよい枠の割合（分子）。
///
/// **比率は測ってから動かす**（ADR-0023）。いまは半々で、
/// **「直近と関連のどちらが効くか」を測る前に片方へ寄せない**という判断そのもの。
const RECENT_SHARE_NUMERATOR: u32 = 1;

/// 層2（直近）が**先に**使ってよい枠の割合（分母）。
///
/// **分子と等しくしないこと。** 等しくすると層2が枠を全部食い、
/// ADR-0003 が却下した **FIFO** に戻る。
const RECENT_SHARE_DENOMINATOR: u32 = 2;

/// **比率そのものはコンパイル時に縛る。**
///
/// 「分子＝分母」（＝ FIFO）と「分子＝0」（＝直近を捨てる）を**書いた時点で
/// ビルドが落ちる**ので、定数をいじる経路はここで閉じている。
///
/// **これで FIFO を「表現できなくした」わけではない**（CLAUDE.md
/// 「型で表現できなくしたと書く前に、表現できてしまう実装を書いてみること」）。
/// [`recent_share`] が `frame` をそのまま返す実装は**このまま通る。**
/// そちらを落とすのは見張り番
/// `the_related_layer_keeps_its_share_when_the_recent_ones_could_fill_the_frame`
/// と `the_recent_layer_never_takes_the_whole_frame`（`recent_share` の出力を見る）。
const _: () = assert!(
    RECENT_SHARE_NUMERATOR > 0 && RECENT_SHARE_NUMERATOR < RECENT_SHARE_DENOMINATOR,
    "層2の先取り分は枠より小さいこと（等しくすると ADR-0003 が却下した FIFO に戻る）"
);

/// 選別にかける候補1件。
///
/// **`&[&str]` と `&[u32]` を並べて渡さない。** 並列のスライスは長さがずれても
/// 型を通り、**採点した本文と課金した費用が1件ずつずれる**という壊れ方をする
/// （ずれても値は出るので、どのアサートにも引っかからない）。
#[derive(Debug, Clone, Copy)]
pub(super) struct Candidate<'a> {
    /// 採点に使う本文。**プロンプトに載るのと同じ文字列**を渡すこと
    /// （`super::MemoryLine` の `body`）。
    pub content: &'a str,
    /// プロンプトの1行にしたときのトークン費用（行区切りを含む）。
    pub cost: u32,
}

/// 想起の内訳（ADR-0023 §3「引かなかったものを数える」）。
///
/// **`parse::extraction::DiscardCounts` にも `emphasis_stripped` にも混ぜない。**
/// どれも別の意味の指標で、混ぜると破棄率が過去の測定値と比べられなくなる
/// （`server::extract` の `record_run` の doc と同じ理由）。
/// **こちらが数えているのは「抽出が何を捨てたか」ではなく「想起が何を引かなかったか」。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecallCounts {
    recent: usize,
    related: usize,
    omitted: usize,
}

impl RecallCounts {
    /// 層2（直近）として載せた件数。
    pub const fn recalled_recent(self) -> usize {
        self.recent
    }

    /// 層3（関連）として載せた件数。**層2と重なった分はこちらに数えない。**
    pub const fn recalled_related(self) -> usize {
        self.related
    }

    /// **有効な記憶のうち、載らなかった件数。**
    ///
    /// 「載っていたのに使われなかった」と「そもそも載っていなかった」を
    /// 事後に切り分けるための数（ADR-0023 §3）。
    pub const fn omitted(self) -> usize {
        self.omitted
    }

    /// 候補の総数。**`recalled_recent + recalled_related + omitted` と一致する。**
    pub const fn candidates(self) -> usize {
        self.recent
            .saturating_add(self.related)
            .saturating_add(self.omitted)
    }
}

/// 選別の結果。
pub(super) struct Recall {
    /// 候補と同じ長さ・同じ並びの採否。**添字は候補の添字（＝作成順）。**
    kept: Vec<bool>,
    counts: RecallCounts,
}

impl Recall {
    /// 候補と同じ並びの採否。**プロンプトには作成順のまま並べる**（層を書かない / ADR-0023）。
    pub(super) fn kept(&self) -> &[bool] {
        &self.kept
    }

    /// 内訳。
    pub(super) fn counts(&self) -> RecallCounts {
        self.counts
    }
}

/// 層2の先取り分。**枠に対する割合**で、切り捨て。
///
/// 掛けてから割るので `u64` で受ける（`u32` のままだと大きな枠で溢れる）。
/// 分子 ≤ 分母なので結果は必ず枠以下に収まり、`u32` へ戻せる。
fn recent_share(frame: u32) -> u32 {
    let scaled = u64::from(frame) * u64::from(RECENT_SHARE_NUMERATOR)
        / u64::from(RECENT_SHARE_DENOMINATOR.max(1));
    u32::try_from(scaled).unwrap_or(frame)
}

/// 今回の発言に対して、**枠に収まるだけの記憶を選ぶ。**
///
/// `candidates` は**作成順（古い順）**であること（`Store::recall_memories` の並び）。
/// `frame` は記憶に回してよいトークン数。
///
/// **決定的**（同じ入力なら常に同じ結果）で、**panic しない。**
pub(super) fn recall(query: &str, candidates: &[Candidate<'_>], frame: u32) -> Recall {
    let mut selection = Selection::new(candidates);

    // **1回だけ採点する。** 候補集合は「そのキャラクターの有効な記憶」の全件で固定
    // （ADR-0023）。ここを層ごとに測り直すと `df` が揺れてスコアが比較不能になる。
    let scores = score_relevance(query, candidates.iter().map(|candidate| candidate.content));

    // 層2（直近）。**先取り分まで。** ここで枠を全部使わせないことが FIFO への逆戻りを防ぐ。
    selection.take_recent(recent_share(frame));
    // 層3（関連）。**層2の余りも使ってよい。**
    selection.take_related(&scores, frame);
    // 層3が余らせた分を層2へ返す。**枠を遊ばせない。**
    selection.take_recent(frame);

    selection.finish()
}

/// 選別の途中経過。**採否・使用量・件数を1つにまとめて持つ**
/// （別々の `&mut` で持ち回ると、片方だけ更新する取り違えが型を通る）。
struct Selection<'a> {
    candidates: &'a [Candidate<'a>],
    kept: Vec<bool>,
    used: u32,
    recent: usize,
    related: usize,
    /// 層2が**次に試す添字 + 1。** 新しい側から古い側へ進む。
    ///
    /// 2度目の呼び出しが**同じところから続き**を採るために覚えておく
    /// （先頭から数え直すと、1度目に打ち切った1件を再び試して同じ場所で止まる）。
    next_recent: usize,
}

impl<'a> Selection<'a> {
    fn new(candidates: &'a [Candidate<'a>]) -> Self {
        Self {
            candidates,
            kept: vec![false; candidates.len()],
            used: 0,
            recent: 0,
            related: 0,
            next_recent: candidates.len(),
        }
    }

    /// 1件を採る。**枠に収まらなければ採らない。**
    fn take(&mut self, index: usize, limit: u32) -> bool {
        let Some(candidate) = self.candidates.get(index) else {
            return false;
        };
        let Some(next) = self.used.checked_add(candidate.cost) else {
            return false;
        };
        if next > limit {
            return false;
        }
        match self.kept.get_mut(index) {
            Some(kept) => *kept = true,
            None => return false,
        }
        self.used = next;
        true
    }

    /// 層2（直近）。**新しい側から連続して**採る。
    ///
    /// **入らない1件に当たったら打ち切る**（歯抜けにしない / モジュール doc）。
    /// **既に層3が採っている記憶は数え直さず、そこで打ち切りもしない**
    /// ——その記憶は既に載っているので、並びに穴は空かない。
    fn take_recent(&mut self, limit: u32) {
        while self.next_recent > 0 {
            let index = self.next_recent - 1;
            if self.kept.get(index).copied().unwrap_or(false) {
                self.next_recent = index;
                continue;
            }
            if !self.take(index, limit) {
                break;
            }
            self.recent = self.recent.saturating_add(1);
            self.next_recent = index;
        }
    }

    /// 層3（関連）。**スコアの高い順。** 入らない1件は飛ばして次を試す。
    ///
    /// **`Relevance` は [`Ord`] としてしか使わない**（モジュール doc）。
    /// 同点は**新しい方**を先に採る（決定的にするための決めごと。
    /// 見張り番は `a_tie_in_the_score_is_broken_by_recency`）。
    fn take_related(&mut self, scores: &[Relevance], limit: u32) {
        let mut order: Vec<usize> = (0..self.candidates.len())
            .filter(|index| !self.kept.get(*index).copied().unwrap_or(false))
            // **文字が1つも重ならない記憶は「関連」ではない。**
            // ここを外すと層3が枠を埋めるためだけに無関係な記憶を拾い、
            // 「関連」という層の意味が消える（余った枠は層2へ返る）。
            .filter(|index| {
                scores
                    .get(*index)
                    .is_some_and(|score| *score > Relevance::MIN)
            })
            .collect();
        order.sort_by_key(|index| (Reverse(scores.get(*index).copied()), Reverse(*index)));

        for index in order {
            if self.take(index, limit) {
                self.related = self.related.saturating_add(1);
            }
        }
    }

    fn finish(self) -> Recall {
        let kept_count = self.kept.iter().filter(|kept| **kept).count();
        Recall {
            counts: RecallCounts {
                recent: self.recent,
                related: self.related,
                omitted: self.kept.len().saturating_sub(kept_count),
            },
            kept: self.kept,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    // ------------------------------------------------------------------
    // フィクスチャ
    //
    // **費用は本文の長さから導かない。** 選別の算術を読めるようにするため、
    // 本文と費用を独立に置く（`Candidate` が2つを1件に束ねているので、
    // 並列スライスのようにずれる形にはならない）。
    // ------------------------------------------------------------------

    /// 発言。**「さかな」と「とり」を両方含む。**
    ///
    /// 「さかな」は直近の記憶が何度も持つ**ありふれた語**、
    /// 「とり」は1件しか持たない**希少な語**になるように候補を組んである。
    const MIXED_QUERY: &str = "さかなととり";
    /// 希少な語を持つ記憶。**候補の全件で採点すれば、こちらが勝つ。**
    const BIRD_MEMORY: &str = "とりが好き";
    /// ありふれた語を持つ記憶。**直近を除いてから採点すると、こちらが勝ってしまう。**
    const FISH_MEMORY: &str = "さかなが好き";
    /// 直近の記憶（新しい側3件）。**どれも「さかな」を持つ。**
    const RECENT_FISH: [&str; 3] = ["さかなを焼いた", "さかなを買った", "さかなを釣った"];

    /// 猫の話の発言。
    const CAT_QUERY: &str = "ねこ";
    /// **`CAT_QUERY` と文字が1つも重ならない**記憶（＝関連度が最小）。
    const UNRELATED: [&str; 7] = [
        "あめがふった",
        "でんしゃにのった",
        "やまへいった",
        "うみでおよいだ",
        "ほんをよんだ",
        "パンをたべた",
        "みずをのんだ",
    ];
    /// `CAT_QUERY` と重なる記憶。
    const CAT_MEMORIES: [&str; 3] = ["ねこをなでた", "ねこがないた", "ねことあそんだ"];

    /// **`CAT_MEMORIES` のどれよりも `CAT_QUERY` との関連度が高い**記憶。
    ///
    /// 余計な文字がほとんど無いので Dice 係数の分母が小さく、`CAT_MEMORIES[0]` を上回る
    /// （前提は `the_related_layer_skips_a_memory_that_does_not_fit` の中で確認している）。
    const TOP_CAT_MEMORY: &str = "ねこだ";

    /// 完全一致の発言（同点と重複の検査で使う）。**上のどれとも違う文字列。**
    const EXACT_QUERY: &str = "ねこがすき";

    fn candidates<'a>(items: &'a [(&'a str, u32)]) -> Vec<Candidate<'a>> {
        items
            .iter()
            .map(|(content, cost)| Candidate {
                content,
                cost: *cost,
            })
            .collect()
    }

    /// 採用した添字（**作成順で昇順**）。
    fn selected(recall: &Recall) -> Vec<usize> {
        recall
            .kept()
            .iter()
            .enumerate()
            .filter_map(|(index, kept)| kept.then_some(index))
            .collect()
    }

    /// 採用分の費用の合計。
    fn spent(recall: &Recall, candidates: &[Candidate<'_>]) -> u32 {
        selected(recall)
            .into_iter()
            .filter_map(|index| candidates.get(index))
            .fold(0_u32, |total, candidate| {
                total.saturating_add(candidate.cost)
            })
    }

    /// **どの結果でも成り立つ不変条件。** すべてのテストから呼ぶ。
    ///
    /// * 採否は候補と同じ長さ。
    /// * **枠を超えない。**
    /// * **`直近 + 関連 + 落ちた = 総数`。**
    /// * **採用した件数 ＝ 直近 ＋ 関連。**
    ///   ここが「同じ記憶を1回だけ数える」の実体。
    fn assert_consistent(recall: &Recall, candidates: &[Candidate<'_>], frame: u32) {
        let counts = recall.counts();
        assert_eq!(recall.kept().len(), candidates.len(), "採否の長さ");
        assert!(
            spent(recall, candidates) <= frame,
            "枠 {frame} を超えた: {} / 選んだのは {:?}",
            spent(recall, candidates),
            selected(recall)
        );
        assert_eq!(
            counts.candidates(),
            candidates.len(),
            "直近 {} ＋ 関連 {} ＋ 落ちた {} が総数と合わない",
            counts.recalled_recent(),
            counts.recalled_related(),
            counts.omitted()
        );
        assert_eq!(
            selected(recall).len(),
            counts
                .recalled_recent()
                .saturating_add(counts.recalled_related()),
            "採用した件数と、層ごとの件数の和が合わない（同じ記憶を2回数えている）"
        );
    }

    /// フィクスチャの文字列が**全部違う**こと（CLAUDE.md「最頻の穴」）。
    #[test]
    fn the_fixture_strings_are_all_different() {
        let mut all: Vec<&str> = vec![
            MIXED_QUERY,
            BIRD_MEMORY,
            FISH_MEMORY,
            CAT_QUERY,
            TOP_CAT_MEMORY,
            EXACT_QUERY,
        ];
        all.extend(RECENT_FISH);
        all.extend(UNRELATED);
        all.extend(CAT_MEMORIES);
        let unique: BTreeSet<&str> = all.iter().copied().collect();
        assert_eq!(unique.len(), all.len(), "同値のフィクスチャがある: {all:?}");
    }

    /// **`UNRELATED` は `CAT_QUERY` と文字を1つも共有しない**（前提の確認）。
    ///
    /// ここが崩れると「層3が0件」を作っているつもりのフィクスチャが
    /// 実は関連ありになり、**その分岐を一度も通らないまま緑になる。**
    #[test]
    fn the_unrelated_fixtures_really_share_nothing_with_the_query() {
        let scores = score_relevance(CAT_QUERY, UNRELATED);
        for (fixture, score) in UNRELATED.iter().zip(&scores) {
            assert_eq!(*score, Relevance::MIN, "{fixture} が発言と重なっている");
        }
        let related = score_relevance(CAT_QUERY, CAT_MEMORIES);
        for (fixture, score) in CAT_MEMORIES.iter().zip(&related) {
            assert!(*score > Relevance::MIN, "{fixture} が発言と重なっていない");
        }
    }

    /// **層2は枠を全部は取れない**（＝ FIFO に戻らない）。
    ///
    /// **候補も予算も通さず、先取り分を出す関数そのものを見る。**
    /// 呼び出し側の期待値から作ると、比率を変える変異が期待値ごと動いて緑になる
    /// （CLAUDE.md「関数の外にある値で押さえる見張り番」）。
    ///
    /// **定数を直接比べる検査はここには置かない。** それは
    /// `const _: () = assert!(..)`（コンパイル時）の仕事で、
    /// ここに写しを置くと clippy が「定数の比較」として弾く。
    #[test]
    fn the_recent_layer_never_takes_the_whole_frame() {
        // 枠が 2 以上あれば、先取り分は必ず枠より小さい。
        for frame in [2_u32, 3, 100, 1_000, u32::MAX] {
            assert!(
                recent_share(frame) < frame,
                "先取り分が枠と同じになった: 枠 {frame} / 先取り {}",
                recent_share(frame)
            );
        }
    }

    /// 候補が空なら何も起きない。
    #[test]
    fn an_empty_candidate_set_recalls_nothing() {
        let items = candidates(&[]);
        let recalled = recall(CAT_QUERY, &items, 1_000);

        assert_consistent(&recalled, &items, 1_000);
        assert_eq!(recalled.counts().omitted(), 0);
        assert_eq!(recalled.counts().candidates(), 0);
    }

    /// 枠が 0 なら1件も載らず、**全件が「落ちた」に数えられる。**
    #[test]
    fn without_a_frame_every_memory_is_omitted() {
        let items = candidates(&[(CAT_MEMORIES[0], 10), (UNRELATED[0], 10)]);
        let recalled = recall(CAT_QUERY, &items, 0);

        assert_consistent(&recalled, &items, 0);
        assert_eq!(selected(&recalled), Vec::<usize>::new());
        assert_eq!(recalled.counts().omitted(), 2);
    }

    /// 直近と関連の両方を持つ、**採点の順序を観測できる**候補集合。
    ///
    /// | 添字 | 中身 | 費用 |
    /// |---|---|---|
    /// | 0 | `BIRD_MEMORY`（希少な語） | 30 |
    /// | 1 | `FISH_MEMORY`（ありふれた語） | 30 |
    /// | 2..4 | `RECENT_FISH`（新しい側） | 10 |
    ///
    /// 枠 60・先取り 30 なので、**層2は 2..4 の3件でちょうど先取り分を使い切り、
    /// 層3に残るのは 30 ＝ 0 か 1 のどちらか1件だけ。**
    fn mixed() -> Vec<(&'static str, u32)> {
        let mut items = vec![(BIRD_MEMORY, 30_u32), (FISH_MEMORY, 30)];
        items.extend(RECENT_FISH.iter().map(|memory| (*memory, 10_u32)));
        items
    }

    /// **採点は分割の前に、候補の全件に対して1回だけ行う**（ADR-0023 の決定）。
    ///
    /// 層2で採った3件（どれも「さかな」を持つ）を**除いてから**採点すると、
    /// `さ` `か` `な` の `df` が 4 から 1 に落ちて**希少に化け**、
    /// `FISH_MEMORY` が `BIRD_MEMORY` を追い抜く。
    /// **選ばれる記憶が実際に入れ替わる**ので、ここで観測できる。
    #[test]
    fn the_score_is_taken_over_every_candidate_before_the_split() {
        let items = mixed();
        let items = candidates(&items);
        let recalled = recall(MIXED_QUERY, &items, 60);
        assert_consistent(&recalled, &items, 60);

        assert_eq!(
            selected(&recalled),
            vec![0, 2, 3, 4],
            "希少な語を持つ記憶（添字 0）が引かれること"
        );

        // **前提の明示。** 候補集合を「直近を除いた2件」にすると順序が逆転する
        // ——この逆転が起きない集合で測っても、上のアサートは何も守らない。
        let whole = score_relevance(MIXED_QUERY, items.iter().map(|item| item.content));
        assert!(
            whole.first() > whole.get(1),
            "全件で採点すれば希少な語の側が高いこと: {whole:?}"
        );
        let without_recent = score_relevance(MIXED_QUERY, [BIRD_MEMORY, FISH_MEMORY]);
        assert!(
            without_recent.first() < without_recent.get(1),
            "直近を除くと順序が入れ替わること（この逆転が見張り番の効き目）: {without_recent:?}"
        );
    }

    /// **直近が枠を全部食わない。** 層3の取り分が必ず残る。
    ///
    /// 同じ枠を層2に先取りさせると 1..4 が連続で入り、
    /// **添字 0（関連の高い記憶）は永久に届かない**＝ FIFO。
    #[test]
    fn the_related_layer_keeps_its_share_when_the_recent_ones_could_fill_the_frame() {
        let items = mixed();
        let items = candidates(&items);
        let recalled = recall(MIXED_QUERY, &items, 60);
        assert_consistent(&recalled, &items, 60);

        let counts = recalled.counts();
        assert_eq!(counts.recalled_recent(), 3);
        assert_eq!(counts.recalled_related(), 1, "関連の取り分が残ること");
        assert_eq!(counts.omitted(), 1);

        // FIFO なら、費用が同じ添字 1 が入って添字 0 が落ちる。
        assert_eq!(recalled.kept().first(), Some(&true), "関連で引いた1件");
        assert_eq!(recalled.kept().get(1), Some(&false));
    }

    /// 10件・費用 20・枠 100 の候補。**載る・関連・落ちる・総数がすべて違う値になる。**
    fn counted() -> Vec<(&'static str, u32)> {
        vec![
            (CAT_MEMORIES[0], 20_u32),
            (UNRELATED[0], 20),
            (CAT_MEMORIES[1], 20),
            (UNRELATED[1], 20),
            (CAT_MEMORIES[2], 20),
            (UNRELATED[2], 20),
            (UNRELATED[3], 20),
            (UNRELATED[4], 20),
            (UNRELATED[5], 20),
            (UNRELATED[6], 20),
        ]
    }

    /// **`omitted` は「有効な記憶のうち、載らなかった件数」。**
    ///
    /// フィクスチャは **直近 2 / 関連 3 / 落ちた 5 / 総数 10** で、
    /// **4つとも違う値**にしてある（取り違えが同値に隠れない）。
    #[test]
    fn omitted_counts_every_active_memory_that_did_not_make_it() {
        let items = counted();
        let items = candidates(&items);
        let recalled = recall(CAT_QUERY, &items, 100);
        assert_consistent(&recalled, &items, 100);

        let counts = recalled.counts();
        assert_eq!(counts.recalled_recent(), 2);
        assert_eq!(counts.recalled_related(), 3);
        assert_eq!(counts.omitted(), 5);
        assert_eq!(counts.candidates(), 10);

        // 4つが互いに違う値であること（フィクスチャの見張り）。
        let values = [
            counts.recalled_recent(),
            counts.recalled_related(),
            counts.omitted(),
            counts.candidates(),
        ];
        let unique: BTreeSet<usize> = values.iter().copied().collect();
        assert_eq!(unique.len(), values.len(), "同値の欄がある: {values:?}");

        // 関連で引いたのは「ねこ」を含む3件、直近で引いたのは新しい2件。
        assert_eq!(selected(&recalled), vec![0, 2, 4, 8, 9]);
    }

    /// **同じ記憶が両方に該当したら1回だけ載る。**
    ///
    /// 添字 2 は**いちばん新しく、かつ発言と完全に一致する**（＝関連度も最大）。
    /// 重複排除を外すと層3がもう一度これを採り、**枠を二重に食って添字 0 が落ちる。**
    #[test]
    fn a_memory_in_both_layers_is_kept_once() {
        let items = candidates(&[(CAT_MEMORIES[0], 20), (UNRELATED[0], 20), (EXACT_QUERY, 20)]);
        let recalled = recall(EXACT_QUERY, &items, 40);
        assert_consistent(&recalled, &items, 40);

        assert_eq!(selected(&recalled), vec![0, 2]);
        assert_eq!(recalled.counts().recalled_recent(), 1);
        assert_eq!(recalled.counts().recalled_related(), 1);
        assert_eq!(recalled.counts().omitted(), 1);
    }

    /// **層3が0件でも、層2は載る。**
    ///
    /// 発言と文字を1つも共有しない記憶しか無い状況。
    /// 層3が使わなかった枠は層2へ戻るので、**枠は遊ばない。**
    #[test]
    fn an_empty_related_layer_still_recalls_the_recent_ones() {
        let items = candidates(&[(UNRELATED[0], 20), (UNRELATED[1], 20), (UNRELATED[2], 20)]);
        let recalled = recall(CAT_QUERY, &items, 100);
        assert_consistent(&recalled, &items, 100);

        assert_eq!(recalled.counts().recalled_related(), 0);
        assert_eq!(recalled.counts().recalled_recent(), 3);
        assert_eq!(recalled.counts().omitted(), 0);
        assert_eq!(selected(&recalled), vec![0, 1, 2]);
    }

    /// **層2が0件でも、層3は載る。**
    ///
    /// いちばん新しい記憶が**先取り分に収まらない**ので層2は打ち切られ、
    /// それでも関連の記憶は引かれる。
    #[test]
    fn an_empty_recent_layer_still_recalls_the_related_ones() {
        let items = candidates(&[(CAT_MEMORIES[0], 50), (UNRELATED[0], 60)]);
        let recalled = recall(CAT_QUERY, &items, 100);
        assert_consistent(&recalled, &items, 100);

        assert_eq!(recalled.counts().recalled_recent(), 0, "層2は空");
        assert_eq!(recalled.counts().recalled_related(), 1);
        assert_eq!(selected(&recalled), vec![0]);
    }

    /// **同点は新しい方を先に採る。**
    ///
    /// 添字 0 と 1 は**同じ本文**なので関連度も等しい。
    /// 決めておかないと並べ替えの実装次第で結果が変わる（決定性が壊れる）。
    #[test]
    fn a_tie_in_the_score_is_broken_by_recency() {
        let items = candidates(&[
            (EXACT_QUERY, 20),
            (EXACT_QUERY, 20),
            (UNRELATED[0], 20),
            (UNRELATED[1], 20),
        ]);
        let scores = score_relevance(EXACT_QUERY, items.iter().map(|item| item.content));
        assert_eq!(scores.first(), scores.get(1), "前提: 同点であること");

        let recalled = recall(EXACT_QUERY, &items, 50);
        assert_consistent(&recalled, &items, 50);
        assert_eq!(
            selected(&recalled),
            vec![1, 3],
            "同点なら新しい添字 1 を採る"
        );
    }

    /// **層2は入らない1件で打ち切る**（歯抜けにしない）。
    ///
    /// 添字 2 が大きすぎて枠に入らないので、**その先の小さな 0・1 は拾わない。**
    /// 飛ばし読みにすると時系列に穴が空く。
    #[test]
    fn the_recent_layer_stops_at_the_first_memory_that_does_not_fit() {
        let items = candidates(&[
            (UNRELATED[0], 10),
            (UNRELATED[1], 10),
            (UNRELATED[2], 100),
            (UNRELATED[3], 20),
        ]);
        let recalled = recall(CAT_QUERY, &items, 100);
        assert_consistent(&recalled, &items, 100);

        assert_eq!(selected(&recalled), vec![3], "打ち切った先を拾わないこと");
        assert_eq!(recalled.counts().recalled_recent(), 1);
        assert_eq!(recalled.counts().omitted(), 3);
    }

    /// **層3は入らない1件を飛ばして次を試す**（打ち切らない）。
    ///
    /// モジュール doc の表で「層2は `break`・層3は `continue`」と書き分けている、
    /// **その分岐に入る唯一の形** ——**層3の並びで、高価な1件の後ろに安価な1件が来る。**
    ///
    /// | 添字 | 中身 | 費用 | 層3での順位 |
    /// |---|---|---|---|
    /// | 0 | `CAT_MEMORIES[0]` | 10 | 2番目 |
    /// | 1 | `TOP_CAT_MEMORY` | 100 | **1番目**（枠 60 に入らない） |
    /// | 2 | `UNRELATED[0]` | 1000 | 層3の対象外（関連が最小） |
    ///
    /// **`continue` を `break` にすると、枠が 60 も空いているのに層3が0件になる**
    /// ——層3の存在理由そのものが消える（実測: `selected` が `[0]` から `[]` になる）。
    ///
    /// 費用が全件同じフィクスチャや、層3の候補が1件だけのフィクスチャでは
    /// **この分岐に一度も入らない**（15b のレビューで実際に通り抜けた）。
    #[test]
    fn the_related_layer_skips_a_memory_that_does_not_fit() {
        const FRAME: u32 = 60;
        let items = candidates(&[
            (CAT_MEMORIES[0], 10),
            (TOP_CAT_MEMORY, 100),
            (UNRELATED[0], 1_000),
        ]);

        // 前提1: 層3の並びが「高価が先・安価が後」になっていること。
        let scores = score_relevance(CAT_QUERY, items.iter().map(|item| item.content));
        assert!(
            scores.get(1) > scores.first(),
            "高価な添字 1 が層3の先頭に来ていない（この並びでないと分岐に入らない）: {scores:?}"
        );
        assert_eq!(scores.get(2), Some(&Relevance::MIN), "{scores:?}");
        // 前提2: 先頭の1件は枠に入らず、次の1件は入る。
        assert!(items[1].cost > FRAME, "先頭が入ってしまうと分岐に入らない");
        assert!(items[0].cost < FRAME);

        let recalled = recall(CAT_QUERY, &items, FRAME);
        assert_consistent(&recalled, &items, FRAME);

        assert_eq!(
            selected(&recalled),
            vec![0],
            "入らない1件で打ち切っている（層3は飛ばして次を試すこと）"
        );
        assert_eq!(recalled.counts().recalled_related(), 1);
        assert_eq!(recalled.counts().recalled_recent(), 0);
        assert_eq!(recalled.counts().omitted(), 2);

        // 打ち切った場合との差を明示する——枠はまだ大きく空いている。
        assert!(
            spent(&recalled, &items) < FRAME,
            "枠が余っていないなら、この検査は打ち切りと区別できない"
        );
    }

    /// **層2は「層3が既に採った記憶」を飛ばす**（そこで打ち切らない）。
    ///
    /// 既に載っている記憶を飛ばしても**並びに穴は空かない**ので、
    /// `break` と同じ扱いにすると**その先の直近の記憶が理由なく落ちる。**
    ///
    /// | 添字 | 中身 | 費用 |
    /// |---|---|---|
    /// | 0 | `UNRELATED[0]`（関連は最小） | 5 |
    /// | 1 | `CAT_MEMORIES[0]`（**層3が採る**） | 15 |
    /// | 2 | `UNRELATED[1]`（層2が先取り分で採る） | 15 |
    ///
    /// 枠 40・先取り 20。層2の1度目は添字 2 だけを採って添字 1 で打ち切り、
    /// 層3が添字 1 を採り、**層2の2度目が添字 1 を飛ばして添字 0 まで届く。**
    /// 飛ばさずに打ち切ると添字 0 が落ちる（`selected` が `[0,1,2]` から `[1,2]` になる）。
    #[test]
    fn the_recent_layer_steps_over_a_memory_the_related_layer_already_took() {
        const FRAME: u32 = 40;
        let items = candidates(&[(UNRELATED[0], 5), (CAT_MEMORIES[0], 15), (UNRELATED[1], 15)]);

        // 前提: 層3が採るのは添字 1 だけ（0 と 2 は関連が最小）。
        let scores = score_relevance(CAT_QUERY, items.iter().map(|item| item.content));
        assert_eq!(scores.first(), Some(&Relevance::MIN), "{scores:?}");
        assert!(scores.get(1) > Some(&Relevance::MIN), "{scores:?}");
        assert_eq!(scores.get(2), Some(&Relevance::MIN), "{scores:?}");

        let recalled = recall(CAT_QUERY, &items, FRAME);
        assert_consistent(&recalled, &items, FRAME);

        assert_eq!(
            selected(&recalled),
            vec![0, 1, 2],
            "層3が採った記憶で層2が打ち切っている（飛ばして続けること）"
        );
        assert_eq!(recalled.counts().recalled_recent(), 2, "添字 2 と 0");
        assert_eq!(recalled.counts().recalled_related(), 1, "添字 1");
        assert_eq!(recalled.counts().omitted(), 0);
    }

    /// **どんな枠でも枠を超えない。** 下端から上端まで舐める。
    ///
    /// 全部入る枠より上まで回して、**「余った枠で何かがおかしくなる」形も見る。**
    #[test]
    fn the_selection_never_exceeds_its_frame() {
        let items = counted();
        let items = candidates(&items);
        let total = items
            .iter()
            .fold(0_u32, |sum, item| sum.saturating_add(item.cost));

        for frame in 0..=total + 40 {
            let recalled = recall(CAT_QUERY, &items, frame);
            assert_consistent(&recalled, &items, frame);
        }

        // 全部入る枠なら1件も落ちない。
        let everything = recall(CAT_QUERY, &items, total);
        assert_eq!(everything.counts().omitted(), 0);
        assert_eq!(selected(&everything).len(), items.len());
    }

    /// 発言が空でも panic せず、**関連は0件**になる（全件が最小値なので）。
    #[test]
    fn an_empty_utterance_recalls_only_the_recent_ones() {
        let items = counted();
        let items = candidates(&items);
        let recalled = recall("", &items, 100);
        assert_consistent(&recalled, &items, 100);

        assert_eq!(recalled.counts().recalled_related(), 0);
        assert_eq!(recalled.counts().recalled_recent(), 5);
        assert_eq!(selected(&recalled), vec![5, 6, 7, 8, 9]);
    }

    /// 費用が飽和するような値でも panic しない。
    #[test]
    fn huge_costs_do_not_panic() {
        let items = candidates(&[(CAT_MEMORIES[0], u32::MAX), (UNRELATED[0], u32::MAX)]);
        let recalled = recall(CAT_QUERY, &items, u32::MAX);
        assert_consistent(&recalled, &items, u32::MAX);
        assert_eq!(selected(&recalled).len(), 1);
    }
}
