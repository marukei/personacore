//! **返答の中で同じ単位がそのまま続く暴走を見つけて畳む規則**
//! （[ADR-0062](../../docs/adr/0062-runaway-repetition-is-folded-for-the-model-and-cut-in-the-relay.md) §決定1 / 作業単位 A-21a）。
//!
//! **I/O を持たない純粋関数**。本文の読み手（`server::turn` の分解・`prompt::history_turn`・
//! 裏方のサブLLMへ渡す `TurnJob::character_message`）は全部ここを通る（ADR-0062 §決定2）。
//! どの読み手も、**漏れた思考を剥がした後**（`super::thinking::strip_leaked_thinking`）に畳む。
//! 履歴では `SafeText::sanitize` の**前**に畳む —— 畳むと、残した単位1回分と区間の後ろの本文が
//! 新しく隣り合うので、そこに組み上がった綴りも無害化に通すため。
//!
//! # 畳まないもの
//!
//! **`message.raw_content`（保存）は畳まない**（CLAUDE.md 絶対制約）。
//! **OpenAI 互換 API の使い手に返す本文も畳まない**（ADR-0062 §決定2 の決定。中継は畳む代わりに生成を打ち切る / §決定4）。
//! **画面は、この決定では畳まない**（ADR-0062 §決定6。こちらは別の作業単位へ後回しにしたもの）。
//!
//! # 会話キャッシュ
//!
//! 同じ入力には同じ結果を返す（ADR-0062 §決定3）。履歴は生の出力から毎ターン畳み直すので、
//! 同じ返答は毎ターン同じ綴りで載る。

use std::borrow::Cow;

use super::text::{LINE_BREAKS, TRIMMABLE};

/// 暴走とみなす単位の**最少の回数**（そのまま続いた回数。区間の長さの下限は `3p`）。
///
/// # 来歴（ADR-0033 / ADR-0062 §背景）
///
/// **測った対象:** 第三者の推論サーバ（ADR-0062 §背景の「その推論サーバ」）の
/// Qwen3.8-Flash-Next の**日本語の返答 928 本**（2026-11-13）。内訳は、測定の置き場所に残っていた
/// 返答のうち空白を除いて 20 文字以上の 868 本（短いものを除いたのはこちらだけ）と、
/// 利用者の DB の返答 60 本（除外なし）。
/// 判定の道具は `<measurements>\empty-2026-11-13\rule_hits.py` の `rule`。
/// **ほかのモデル・ほかの言語は測っていない。**
///
/// **同じ文を2回言い直すのは、強調として書かれうる。** 3回で区切ると、測った範囲では
/// 言い直しを拾わずにループだけを拾えた。
///
/// **境目のすぐ下にあった3本**（区間の最長が 40〜63 文字。どれも畳まない）:
///
/// 1. 4 文字の単位を 13 回繰り返して自然に止まった返答（52 文字）
/// 2. 同じ文の並びを3回続けた返答（60 文字）
/// 3. 2つの句を交互に言い直した返答（41 文字）
///
/// **つまみにしない**（ADR-0036 §決定3。根拠となる計測がある）。外れたら ADR-0062 の追記で直す。
pub const RUNAWAY_MIN_COPIES: usize = 3;

/// 暴走とみなす区間の**最短の長さ**（空白を除いた文字数）。
///
/// # 来歴（ADR-0033 / ADR-0062 §背景）
///
/// 測った対象と境目の下の3本は [`RUNAWAY_MIN_COPIES`] の doc と同じ（同じ 928 本・同じ道具）。
/// 64 文字以上は 41 本で、最短の 67 文字から 163 文字までの 12 本は全部読んで、どれも
/// 同じ文か句を3回以上そのまま繰り返すループだった。40〜63 文字は上の3本だけ
/// （52・60・41 文字）で、どれもこの値に届かない。**短いループ（52・60 文字）を拾わないことは、
/// この値の代償として ADR-0062 §結果が受け入れている。**
pub const RUNAWAY_MIN_CHARS: usize = 64;

/// 単位の**最長の周期**（空白を除いた文字数）。これより長い単位の繰り返しは見ない。
///
/// # 来歴（ADR-0033 / ADR-0062 §背景）
///
/// 測った対象・道具・境目の下の3本は [`RUNAWAY_MIN_COPIES`] の doc と同じ。
/// 測定は周期 2〜64 で数えた —— **65 文字以上の単位は、測った範囲の外**である。
pub const RUNAWAY_MAX_PERIOD: usize = 64;

/// 単位の**最短の周期**。測定（`rule_hits.py`）と同じく 2 から数える。
///
/// 周期 1（同じ文字の連続）は単独では見ない。ただし同じ文字が続けば周期 2 の条件も満たすので、
/// 同じ文字が 64 文字続けば周期 2 として畳まれる（測定の道具も同じに数える）。
const RUNAWAY_MIN_PERIOD: usize = 2;

/// 畳んだ区間1つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunawayRun {
    /// 区間の最初の文字の、**渡した文字列での**バイト位置。
    pub start: usize,
    /// 区間の最後の文字の**直後の**バイト位置（区間の後ろの空白は含まない）。
    pub end: usize,
    /// 単位の長さ（空白を除いた文字数）。
    pub period: usize,
    /// 区間の長さ（空白を除いた文字数。単位1回分を含む）。
    pub length: usize,
}

/// 畳んだ結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedRunaway<'a> {
    /// 畳んだ後の本文。**畳むものが無ければ元の文字列を借りたまま**（1バイトも変えない）。
    pub text: Cow<'a, str>,
    /// 畳んだ区間。**位置の順。** 無ければ空。
    pub runs: Vec<RunawayRun>,
}

impl FoldedRunaway<'_> {
    /// 最長の区間の長さ（空白を除いた文字数）。**区間が無ければ 0。**
    pub fn longest(&self) -> usize {
        self.runs.iter().map(|run| run.length).max().unwrap_or(0)
    }
}

/// 返答の中の暴走を畳む（ADR-0062 §決定1）。**panic しない。**
///
/// # 規則
///
/// * **空白を除いた文字列**で数える。空白は `TRIMMABLE` と `LINE_BREAKS`（漏れた思考の剥がしと同じ集合 /
///   `super::thinking`）。測定の道具（Python の `str.split()`）が除くのは `str.isspace()` が真の文字
///   （Unicode の White_Space に加えて、White_Space ではない U+001C〜U+001F も含む）なので、
///   U+001C〜U+001F・U+1680・U+2000〜U+200A・U+202F・U+205F の扱いだけが測定と違う。
///   測定の置き場所の 868 本には差の文字が 0 個だった（A-21a のレビュアーが数えた）。DB の 60 本は確かめていない。
/// * 周期 p（`RUNAWAY_MIN_PERIOD`〜[`RUNAWAY_MAX_PERIOD`]）ごとに、`t[i] == t[i+p]` が続く長さ cur を数え、
///   続きが切れた（または末尾に達した）ところで、**cur ≥ (回数 − 1)·p かつ cur + p ≥ 64** なら
///   `t[i_0 .. i_0 + cur + p]` を区間の候補にする（長さ cur + p。下限は `max(3p, 64)`）。
/// * 候補は**長いほう → 周期の短いほう → 前にあるほう**の順に採る（「あい」の繰り返しは周期 4・6・… でも
///   同じ長さの区間になる —— 単位は最短の「あい」）。
/// * **既に採った区間と重なる候補は、重ならない残り（ふつうは前側・後ろ側のどちらか、または両側）を判定し直す**
///   （ADR-0062 §決定1 の追記 / 作業単位 A-21a）。周期 p の区間の部分区間はやはり周期 p なので、残りが
///   長さ ≥ `max(3p, 64)` を満たせばその部分を区間としてその場で採り、満たさなければ捨てる（並べ直しはしない）。
///   偶然の一致で区間は境目の前後へ数文字伸びるので、隣り合う別々のループが1〜2文字重なることがある ——
///   丸ごと捨てると、短いほうのループが畳まれずに残る。
///
/// # 畳み方
///
/// 区間ごとに、**単位1回分（区間の最初の p 文字）を残し、その直後から区間の最後の文字までを落とす。**
/// 元の文字列で言えば、落とすのは「単位1回分の最後の文字の直後」から「区間の最後の文字の直後」までのバイト範囲で、
///
/// * 単位1回分の**中の**空白は元の位置のまま残る。
/// * 落とす範囲の中の空白（改行を含む）は一緒に落ちる。
/// * 区間の後ろ（区間の最後の文字の直後から）は空白も本文も全部残る。
///
/// 区間の始まりは、ループの前の本文が偶然単位の末尾と一致した分（1文字とは限らない）だけ前へ伸びる。
/// そのとき残る単位1回分は、ループの単位を回転させた綴りになる（例: 前の文の `。` と単位の末尾の `。`）。
///
/// # 計算量
///
/// 候補を集めるのは周期ごとに1回の走査で O(n · 64)（n は空白を除いた文字数）。同じ周期の候補どうしは、
/// 間の食い違い1文字を挟んで p − 1 文字まで重なりうるが、始まりは「続いた長さ ＋ 1」文字ずつ離れ、続いた長さは
/// `max(2p, 64 − p)` ≥ 43 なので、数は周期ごとに高々 n/44 + 1（全体で 63 倍）。採った区間は互いに重ならず
/// 64 文字以上なので高々 n/64 個。採る部分は候補ごとにそれと突き合わせるので最悪 O(n²) だが、定数は
/// およそ 63/44 · 1/64 ≈ 1/45 で、返答の長さ（数千文字）では小さい。
pub fn fold_runaway_repetition(text: &str) -> FoldedRunaway<'_> {
    // 空白を除いた文字と、その元の文字列でのバイト位置。
    let glyphs: Vec<(usize, char)> = text
        .char_indices()
        .filter(|(_, character)| !is_blank(*character))
        .collect();

    let mut candidates = runaway_candidates(&glyphs);
    // **長いほう → 周期の短いほう → 前にあるほう**の順に採る。
    candidates.sort_by(|left, right| {
        right
            .length
            .cmp(&left.length)
            .then(left.period.cmp(&right.period))
            .then(left.start.cmp(&right.start))
    });
    let mut chosen: Vec<Candidate> = Vec::new();
    for candidate in candidates {
        // **重なる区間を除いた残りを判定し直して採る**（ADR-0062 §決定1 の追記 / A-21a）。
        for remainder in candidate.remainders(&chosen) {
            if remainder.is_runaway() {
                chosen.push(remainder);
            }
        }
    }
    chosen.sort_by_key(|candidate| candidate.start);

    if chosen.is_empty() {
        return FoldedRunaway {
            text: Cow::Borrowed(text),
            runs: Vec::new(),
        };
    }

    let mut runs = Vec::with_capacity(chosen.len());
    let mut folded = String::with_capacity(text.len());
    let mut cursor = 0_usize;
    for candidate in &chosen {
        // 区間の最初の文字・単位1回分の最後の文字・区間の最後の文字（どれも空白を除いた添字）。
        let (Some(first), Some(unit_last), Some(last)) = (
            glyphs.get(candidate.start),
            glyphs.get(candidate.start + candidate.period - 1),
            glyphs.get(candidate.start + candidate.length - 1),
        ) else {
            // 候補は `glyphs` の中からしか作らないので来ない。来たら畳まずに次へ（panic しない）。
            continue;
        };
        let keep_until = glyph_end(*unit_last);
        let run_end = glyph_end(*last);
        folded.push_str(text.get(cursor..keep_until).unwrap_or_default());
        cursor = run_end;
        runs.push(RunawayRun {
            start: first.0,
            end: run_end,
            period: candidate.period,
            length: candidate.length,
        });
    }
    folded.push_str(text.get(cursor..).unwrap_or_default());

    FoldedRunaway {
        text: Cow::Owned(folded),
        runs,
    }
}

/// 区間の候補（添字は空白を除いた文字列のもの）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Candidate {
    start: usize,
    period: usize,
    length: usize,
}

impl Candidate {
    fn end(&self) -> usize {
        self.start + self.length
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.start < other.end() && other.start < self.end()
    }

    /// 暴走の条件（長さ ≥ `max(回数·p, 64)`）を満たすか。`t[i] == t[i+p]` の続いた長さは `length - p`。
    fn is_runaway(&self) -> bool {
        let matched = self.length.saturating_sub(self.period);
        self.length >= self.period
            && matched >= (RUNAWAY_MIN_COPIES - 1) * self.period
            && self.length >= RUNAWAY_MIN_CHARS
    }

    /// `taken`（既に採った区間）と重ならない残り。**周期はそのまま**（周期 p の区間の部分区間は周期 p）。
    ///
    /// どれとも重ならなければ自分自身が1つだけ返る。採った区間は互いに重ならない。もとの候補はどれも自分以上の
    /// 長さなので、ふつう残りは前側・後ろ側の高々2つだが、**削った残りを採った区間は自分より短くなりうる**
    /// （自分の中に収まり、3つ以上に割れうる）。ここはどの場合も、採った区間を順に引いて扱う。
    fn remainders(&self, taken: &[Candidate]) -> Vec<Candidate> {
        let mut pieces = vec![*self];
        for cut in taken.iter().filter(|cut| self.overlaps(cut)) {
            let mut next = Vec::with_capacity(pieces.len() + 1);
            for piece in pieces {
                if !piece.overlaps(cut) {
                    next.push(piece);
                    continue;
                }
                if piece.start < cut.start {
                    next.push(Candidate {
                        start: piece.start,
                        period: piece.period,
                        length: cut.start - piece.start,
                    });
                }
                if cut.end() < piece.end() {
                    next.push(Candidate {
                        start: cut.end(),
                        period: piece.period,
                        length: piece.end() - cut.end(),
                    });
                }
            }
            pieces = next;
        }
        pieces
    }
}

/// 周期ごとに1回ずつ走査して、条件を満たす区間の候補を全部集める。
fn runaway_candidates(glyphs: &[(usize, char)]) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for period in RUNAWAY_MIN_PERIOD..=RUNAWAY_MAX_PERIOD {
        // `t[i] == t[i+p]` が続いている長さと、その始まり。
        let mut matched = 0_usize;
        for (index, (left, right)) in glyphs.iter().zip(glyphs.iter().skip(period)).enumerate() {
            if left.1 == right.1 {
                matched += 1;
            } else {
                // `matched <= index` なので引き算は桁あふれしない（`saturating_sub` は念のため）。
                push_if_runaway(
                    &mut candidates,
                    index.saturating_sub(matched),
                    period,
                    matched,
                );
                matched = 0;
            }
        }
        // 末尾まで続いた分。比べた組の数は `n - p`。
        let compared = glyphs.len().saturating_sub(period);
        push_if_runaway(
            &mut candidates,
            compared.saturating_sub(matched),
            period,
            matched,
        );
    }
    candidates
}

/// `t[start .. start + matched + period]` が暴走の条件を満たせば候補に足す。
fn push_if_runaway(candidates: &mut Vec<Candidate>, start: usize, period: usize, matched: usize) {
    let candidate = Candidate {
        start,
        period,
        length: matched + period,
    };
    if candidate.is_runaway() {
        candidates.push(candidate);
    }
}

/// 文字の直後のバイト位置。
fn glyph_end((at, character): (usize, char)) -> usize {
    at + character.len_utf8()
}

/// 数えるときに除く空白か（`TRIMMABLE` と `LINE_BREAKS`。漏れた思考の剥がしの空白と同じ集合）。
fn is_blank(character: char) -> bool {
    TRIMMABLE.contains(&character) || LINE_BREAKS.contains(&character)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 畳んだ本文と、区間の（周期, 長さ）の列。
    fn fold(text: &str) -> (String, Vec<(usize, usize)>) {
        let folded = fold_runaway_repetition(text);
        (
            folded.text.to_string(),
            folded
                .runs
                .iter()
                .map(|run| (run.period, run.length))
                .collect(),
        )
    }

    /// 畳まずに、元の文字列を借りたまま返すこと。
    fn assert_untouched(text: &str) {
        let folded = fold_runaway_repetition(text);
        assert!(
            matches!(folded.text, Cow::Borrowed(kept) if kept == text),
            "畳むものが無ければ元を借りたまま返すこと: {text:?} -> {folded:?}"
        );
        assert!(folded.runs.is_empty(), "{text:?} -> {:?}", folded.runs);
        assert_eq!(folded.longest(), 0, "{text:?}");
    }

    /// 21 文字の単位（どの2文字の組も周期 21 未満で繰り返さない並び）。**手で数えた。**
    const UNIT_21: &str = "今日はいい天気ですね、散歩に行きましょう。";
    /// 22 文字の単位（[`UNIT_21`] に1文字足したもの）。**手で数えた。**
    const UNIT_22: &str = "今日はいい天気ですね、散歩に行きましょうか。";
    /// 36 文字の文。**2回だけの言い直し**に使う。**手で数えた。**
    const SENTENCE_36: &str =
        "わたしはあなたのことが本当に大好きで、ずっとそばにいたいと思っているの。";
    /// 区間の後ろに続く本文。**どの単位とも続かない文字で始まり、どの単位の末尾とも違う文字で終わる**
    /// （`。` で終えると、次の区間の単位の末尾の `。` と周期が合い、区間が1文字前へ伸びる）。
    const TAIL: &str = "最後に彼女は笑った！";
    /// 区間の前に置く本文（同じ理由で、どの単位の末尾とも違う文字で終わる）。
    const HEAD: &str = "前置き：";

    /// `count` 個の互いに違う漢字（U+4E00 から）。**どの周期でも繰り返さない単位**を作る。
    fn distinct(count: u32) -> String {
        (0..count)
            .filter_map(|offset| char::from_u32(0x4E00 + offset))
            .collect()
    }

    #[test]
    fn the_fixture_units_have_the_lengths_written_on_them() {
        assert_eq!(UNIT_21.chars().count(), 21);
        assert_eq!(UNIT_22.chars().count(), 22);
        assert_eq!(SENTENCE_36.chars().count(), 36);
        assert_eq!(distinct(64).chars().count(), 64);
        assert_eq!(distinct(65).chars().count(), 65);
    }

    #[test]
    fn ordinary_text_is_returned_borrowed_and_untouched() {
        for text in [
            "",
            "あ",
            "「おかえりなさい」 彼女は微笑んだ。",
            "\u{3000}「待って、待って」と彼女は言った。",
            // **同じ文字が続くだけで 64 文字に届かない**（周期 2 で 63 文字）。
            &"ー".repeat(63),
        ] {
            assert_untouched(text);
        }
    }

    /// **`TRIMMABLE` と `LINE_BREAKS` の全部の文字を、数えるときに除く**（全文字ループ）。
    ///
    /// 「あい」＋その文字を 22 回。除けば 44 文字で畳まない。**数えてしまうと 66 文字の周期 3 の区間**になり、
    /// 畳まれる（対照の `う` がその形）。
    #[test]
    fn every_blank_is_left_out_of_the_count() {
        for blank in TRIMMABLE.iter().chain(LINE_BREAKS.iter()) {
            let text = format!("あい{blank}").repeat(22);
            assert_untouched(&text);
        }
        // 対照: 空白でない文字は数える。
        let counted = "あいう".repeat(22);
        assert_eq!(fold(&counted), ("あいう".to_owned(), vec![(3, 66)]));
    }

    // ---- 64 文字の条件（周期 2） ----

    #[test]
    fn a_two_character_unit_reaching_64_characters_is_folded() {
        let text = format!("{HEAD}{}", "あい".repeat(32));
        assert_eq!(fold(&text), (format!("{HEAD}あい"), vec![(2, 64)]));
    }

    #[test]
    fn a_two_character_unit_stopping_at_63_characters_is_left_alone() {
        let text = format!("{HEAD}{}あ", "あい".repeat(31));
        assert_eq!("あい".repeat(31).chars().count() + 1, 63);
        assert_untouched(&text);
    }

    // ---- 3回の条件と 64 文字の条件の交わり ----

    #[test]
    fn three_copies_of_a_21_character_unit_fall_short_of_64_characters() {
        let text = format!("{HEAD}{}", UNIT_21.repeat(3));
        assert_untouched(&text);
    }

    #[test]
    fn three_copies_of_a_22_character_unit_reach_66_characters_and_are_folded() {
        let text = format!("{HEAD}{}", UNIT_22.repeat(3));
        assert_eq!(fold(&text), (format!("{HEAD}{UNIT_22}"), vec![(22, 66)]));
    }

    #[test]
    fn a_sentence_said_twice_is_left_alone_but_three_times_is_folded() {
        // **2回だけ**（72 文字 —— 長さは足りるが回数が足りない）。
        let twice = format!("{HEAD}{}", SENTENCE_36.repeat(2));
        assert_untouched(&twice);

        // 対照: **3回**（108 文字）は畳む。
        let thrice = format!("{HEAD}{}", SENTENCE_36.repeat(3));
        assert_eq!(
            fold(&thrice),
            (format!("{HEAD}{SENTENCE_36}"), vec![(36, 108)])
        );
    }

    // ---- 周期の上限 ----

    #[test]
    fn three_copies_of_a_64_character_unit_are_folded() {
        let unit = distinct(64);
        let text = format!("{HEAD}{}", unit.repeat(3));
        assert_eq!(fold(&text), (format!("{HEAD}{unit}"), vec![(64, 192)]));
    }

    #[test]
    fn a_65_character_unit_is_beyond_the_periods_looked_at() {
        let text = format!("{HEAD}{}", distinct(65).repeat(3));
        assert_untouched(&text);
    }

    // ---- 畳み方 ----

    #[test]
    fn the_text_after_the_run_is_kept() {
        let text = format!("{HEAD}{}\n{TAIL}", "「好き」……".repeat(20));
        assert_eq!(
            fold(&text),
            (format!("{HEAD}「好き」……\n{TAIL}"), vec![(6, 120)])
        );
    }

    #[test]
    fn a_run_cut_in_the_middle_of_a_unit_is_folded_to_one_whole_unit() {
        // `max_tokens` で切れた形（最後の単位が途中まで）。
        let text = format!("{HEAD}{}「好", "「好き」……".repeat(20));
        assert_eq!(fold(&text), (format!("{HEAD}「好き」……"), vec![(6, 122)]));
    }

    #[test]
    fn a_loop_across_line_breaks_keeps_the_unit_and_the_break_after_the_run() {
        // **単位の中の空白は残す側の元の位置のまま**、落とす区間の中の改行は一緒に落ちる。
        let text = format!("{HEAD}\n{}{TAIL}", "「好き」 ……\n".repeat(20));
        assert_eq!(
            fold(&text),
            (format!("{HEAD}\n「好き」 ……\n{TAIL}"), vec![(6, 120)])
        );
    }

    /// **CRLF の腕**（CLAUDE.md「改行と置換の道具」）。`\r` と `\n` は2つの空白として位置の対応に入る。
    #[test]
    fn a_loop_across_crlf_line_breaks_keeps_the_crlf_after_the_run() {
        let text = format!("{HEAD}\r\n{}{TAIL}", "「好き」 ……\r\n".repeat(20));
        let (folded, runs) = fold(&text);
        assert_eq!(folded, "前置き：\r\n「好き」 ……\r\n最後に彼女は笑った！");
        assert_eq!(runs, vec![(6, 120)]);
    }

    #[test]
    fn the_run_positions_are_byte_offsets_in_the_given_text() {
        let text = format!("{HEAD}{} {TAIL}", "あい".repeat(32));
        let folded = fold_runaway_repetition(&text);
        let start = HEAD.len();
        let end = start + "あい".repeat(32).len();
        assert_eq!(
            folded.runs,
            vec![RunawayRun {
                start,
                end,
                period: 2,
                length: 64
            }]
        );
        assert_eq!(text.get(end..), Some(" 最後に彼女は笑った！"));
    }

    #[test]
    fn two_separate_runs_are_both_folded_in_order() {
        let text = format!(
            "{HEAD}{}{TAIL}{}{TAIL}",
            "あい".repeat(40),
            UNIT_22.repeat(4)
        );
        let folded = fold_runaway_repetition(&text);
        assert_eq!(folded.text, format!("{HEAD}あい{TAIL}{UNIT_22}{TAIL}"));
        let runs: Vec<(usize, usize)> = folded
            .runs
            .iter()
            .map(|run| (run.period, run.length))
            .collect();
        assert_eq!(runs, vec![(2, 80), (22, 88)]);
        assert_eq!(folded.longest(), 88, "最長の区間は2つ目");
        assert!(folded.runs[0].end <= folded.runs[1].start, "位置の順");
    }

    // ---- 重なり ----

    #[test]
    fn a_run_seen_under_several_periods_is_counted_once_with_the_shortest_unit() {
        // 「あい」の繰り返しは周期 4・6・… でも同じ長さの区間になる。**1つに数え、単位は最短。**
        let text = format!("{HEAD}{}", "あい".repeat(40));
        assert_eq!(fold(&text), (format!("{HEAD}あい"), vec![(2, 80)]));
    }

    /// **重なったら長い方を先に採り、短い方は重ならない残りで判定し直す**（ADR-0062 §決定1 の追記 / A-21a）。
    ///
    /// 周期 3 の 90 文字の区間（`あいう`×30）の最後の2文字（`いう`）から、周期 2 の区間が始まる
    /// （偶然の一致で2文字前へ伸びる）。ここは**残りが条件に届かない腕**: 周期 2 の側は 64 文字で、
    /// 重なりを除くと 62 文字なので捨てる。
    #[test]
    fn of_two_overlapping_runs_the_longer_one_wins_and_a_short_remainder_is_dropped() {
        let text = format!("{}{}", "あいう".repeat(30), "いう".repeat(31));
        assert_eq!(
            fold(&text),
            (format!("あいう{}", "いう".repeat(31)), vec![(3, 90)])
        );
    }

    /// **残りが条件を満たす腕**: 周期 2 の側を 82 文字（重なり2文字 ＋ `いう`×40）にすると、
    /// 重なりを除いた 80 文字が周期 2 の区間として畳まれる。**丸ごと捨てる実装では、80 文字のループが残る。**
    #[test]
    fn the_remainder_of_an_overlapping_run_is_folded_when_it_still_qualifies() {
        let text = format!("{}{}", "あいう".repeat(30), "いう".repeat(40));
        assert_eq!(
            fold(&text),
            ("あいういう".to_owned(), vec![(3, 90), (2, 80)])
        );
    }

    /// **残りが前側に出る腕**: 短いループ（`いう`×40、周期 2・80 文字）の後ろに長いループ（`あいう`×30）が来る。
    /// 周期 3 の区間は偶然の一致で2文字前へ伸びて `[78, 170)`（92 文字）になり、周期 2 の区間の最後の2文字と重なる。
    /// 周期 2 の側は重ならない前側の 78 文字で畳む。残る単位1回分は、後ろの区間が `いうあ`（回転した綴り）。
    #[test]
    fn the_front_remainder_of_an_overlapping_run_is_folded_when_it_still_qualifies() {
        let text = format!("{}{}", "いう".repeat(40), "あいう".repeat(30));
        assert_eq!(
            fold(&text),
            ("いういうあ".to_owned(), vec![(2, 78), (3, 92)])
        );
    }

    /// **採った区間が内側に収まると、残りは両側に割れる**（`Candidate::remainders` を直に呼ぶ / A-21a の再レビュー）。
    ///
    /// いまの `fold_runaway_repetition` の入力では、1つの採った区間が候補の内側に収まる形を作りにくい
    /// （削った残りを採った後にだけ起きる）。分岐を直に固定する: `[0, 200)` から `[80, 120)` を引くと
    /// `[0, 80)` と `[120, 200)`。**周期は候補のまま**（引いた側の周期 3 を写さない）。
    #[test]
    fn a_cut_inside_a_candidate_leaves_both_sides() {
        let piece = Candidate {
            start: 0,
            period: 2,
            length: 200,
        };
        let cut = Candidate {
            start: 80,
            period: 3,
            length: 40,
        };
        assert_eq!(
            piece.remainders(&[cut]),
            vec![
                Candidate {
                    start: 0,
                    period: 2,
                    length: 80
                },
                Candidate {
                    start: 120,
                    period: 2,
                    length: 80
                },
            ]
        );
        // 対照: 重ならない区間を引いても、自分自身が1つだけ返る。
        let apart = Candidate {
            start: 200,
            period: 3,
            length: 40,
        };
        assert_eq!(piece.remainders(&[apart]), vec![piece]);
    }

    /// **隙間なく隣り合う2つの区間は、どちらも畳む**（レビューの指摘 / A-21a）。
    ///
    /// 8 文字の単位 ×9、先頭の1文字を欠いた単位 ×1、単位 ×9。周期 8 の候補は空白を除いた位置で
    /// `[4, 76)`（72 文字）と `[76, 155)`（79 文字）になり、**境目を共有するが重ならない。**
    /// 後ろの区間は欠けた単位から始まるので、残る単位1回分は回転した綴り（`っと一緒だよ、ず`）になる。
    #[test]
    fn two_runs_that_touch_without_overlapping_are_both_folded() {
        const UNIT: &str = "ずっと一緒だよ、";
        let clipped = "っと一緒だよ、";
        let text = format!("{HEAD}{}{clipped}{}{TAIL}", UNIT.repeat(9), UNIT.repeat(9));
        let folded = fold_runaway_repetition(&text);
        assert_eq!(folded.text, format!("{HEAD}{UNIT}っと一緒だよ、ず{TAIL}"));
        let runs: Vec<(usize, usize)> = folded
            .runs
            .iter()
            .map(|run| (run.period, run.length))
            .collect();
        assert_eq!(runs, vec![(8, 72), (8, 79)]);
        // 前提: 2つの区間は境目のバイト位置を共有する（隙間も重なりも無い）。
        assert_eq!(folded.runs[0].end, folded.runs[1].start);
    }

    // ---- 決定性 ----

    #[test]
    fn the_same_input_gives_the_same_result() {
        let text = format!(
            "{HEAD}\r\n{}{TAIL}{}",
            "「好き」 ……\r\n".repeat(20),
            UNIT_22.repeat(5)
        );
        let first = fold_runaway_repetition(&text);
        let second = fold_runaway_repetition(&text);
        assert_eq!(first, second);
        assert_eq!(
            first.runs.len(),
            2,
            "前提: 区間が2つ畳まれること: {first:?}"
        );
    }
}
