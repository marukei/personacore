//! **上流が数えた部品の費用の表**（[ADR-0052](../../docs/adr/0052-count-with-the-upstream-tokenizer.md)
//! §決定2 / 作業単位 B-60c）。
//!
//! # `estimate_tokens` の中へ入れない理由
//!
//! `estimate_tokens` は**バイト数の上界**（ADR-0013 第1層 / ADR-0052 §決定1 の (i)）で、
//! その名前の下で上界でない値を返すと、上界を前提にした見張り番がすべて別の主張になる
//! （ADR-0052 §見張り番の扱い）。だから**数えた値は別の型・別の名前**にする。
//!
//! # 上界ではない
//!
//! ここに入る数は、上流のトークナイザが**部品1つを単独で**数えた値である。
//! トークン数は連結で足し算にならない（ADR-0052 §背景の反例）ので、
//! **部品の和は組み上がりの上界にならない。** 安全は送る直前の封が持つ（`server::counted_plan` / B-60d-1）。
//! この表が担うのは**計画（どの記憶と履歴を載せるか）だけ**である。
//!
//! # 表に無い部品はバイト数
//!
//! 数えていない部品は [`estimate_tokens`]（上界）で数える。**混ざってよい**（§決定2）。
//!
//! 何も数えていない表（[`CountedParts::default`]）について、見張り番
//! `super::tests::an_upstream_that_cannot_count_leaves_the_prompt_exactly_as_today` が見ているのは2つ:
//!
//! * **変更前（B-60c の前の `main`）が組んだ計画と本文に一致する**（腕0。期待値は `main` で書き出した
//!   リテラルで、検査対象から作っていない）。**見ている予算は書き出した数点だけ**で、全部の予算ではない。
//! * **表に無い部品の費用が、バイト数を直に渡したときと同じになる**（腕1。予算の掃引）。
//!   こちらは両辺が同じ値付け（`super::price`）を通るので、**値付けそのものが変わっても緑のまま**
//!   （それを止めるのは腕0と、既存のバイト数の見張り番）。
//!
//! # I/O を持たない
//!
//! 数えるのは `server`（`server::counted_plan` / B-60d-1）で、ここは**数えた結果を値として受け取るだけ**（CLAUDE.md §3）。

use std::collections::HashMap;

use super::estimate_tokens;

/// 部品の文字列 → 上流が数えたトークン数。
///
/// **鍵は部品の文字列そのもの**（無害化と引用を通した後の行 / ADR-0052 §決定6）。
/// どの文字列が部品かは [`super::parts_to_count`] が出す。**正規化しない**
/// （空白1つ違えば別の部品 —— 別の文字列は別のトークン列になりうる）。
///
/// **DB に持たない**（§決定6）。トークナイザに依存する数を行に残すと、
/// モデルを替えたときに古い数が「事実」として残る。
#[derive(Debug, Clone, Default)]
pub struct CountedParts {
    tokens: HashMap<String, u32>,
}

impl CountedParts {
    /// 部品1つの数えた値を覚える。同じ部品をもう一度入れたら新しい値で置き換える。
    pub fn insert(&mut self, part: String, tokens: u32) {
        self.tokens.insert(part, tokens);
    }

    /// 部品1つの費用。**表に無ければバイト数**（[`estimate_tokens`]）。
    pub(super) fn tokens(&self, part: &str) -> u32 {
        self.tokens
            .get(part)
            .copied()
            .unwrap_or_else(|| estimate_tokens(part))
    }
}

impl FromIterator<(String, u32)> for CountedParts {
    fn from_iter<I: IntoIterator<Item = (String, u32)>>(iter: I) -> Self {
        Self {
            tokens: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **表に無い部品はバイト数**（上界のまま）。値は関数の外の値（`len()`）で押さえる。
    #[test]
    fn a_part_that_was_not_counted_costs_its_byte_length() {
        let table = CountedParts::from_iter([("> - 猫が好き".to_owned(), 3)]);
        for part in ["", "こんにちは", "> - 犬が好き", "\u{20BB7}"] {
            assert_eq!(
                table.tokens(part),
                u32::try_from(part.len()).expect("短い文字列であること"),
                "{part:?}"
            );
        }
        assert_eq!(CountedParts::default().tokens("こんにちは"), 15);
    }

    /// **表にある部品は、上流が数えた値そのもの**（バイト数より大きくても小さくても、丸めない）。
    #[test]
    fn a_counted_part_costs_what_the_upstream_said() {
        let mut table = CountedParts::default();
        table.insert("> - 猫が好き".to_owned(), 3);
        table.insert("ab".to_owned(), 7);
        assert_eq!(table.tokens("> - 猫が好き"), 3);
        assert_eq!(table.tokens("ab"), 7, "バイト数（2）より大きい値も丸めない");

        // 入れ直したら新しい値。
        table.insert("ab".to_owned(), 1);
        assert_eq!(table.tokens("ab"), 1);
    }

    /// **鍵は文字列そのもの**（正規化しない）。空白1つ違えば表に無い部品で、バイト数に落ちる。
    #[test]
    fn a_part_is_looked_up_by_its_exact_text() {
        let table = CountedParts::from_iter([("> - 猫が好き".to_owned(), 3)]);
        // `> - ` 4 バイト ＋ 4文字 × 3 バイト ＋ 空白 1 = 17。
        assert_eq!(table.tokens("> - 猫が好き "), 17);
        assert_eq!(table.tokens(">  - 猫が好き"), 17);
        assert_eq!(table.tokens("> - 猫が好き"), 3);
    }
}
