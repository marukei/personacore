//! **返答の長さの目安の行**（[ADR-0065](../../docs/adr/0065-the-response-length-target-is-a-character-field.md) §決定2 /
//! 作業単位 A-22a = marukei/personacore#71）。
//!
//! # これは `ja` の束の持ち物（ADR-0029 §決定2）
//!
//! **文面も、数の書き方（漢数字）も、測定の結果物である**（ADR-0029 §決定4）。
//! いまコードに言語の型は無いので、`prompt` の定数として置く。**`ja` 以外の束を足すときは、文面も単位
//! （字か語か）も測り直す**（ADR-0029 §決定4 / ADR-0065 §決定2）。保存する値（下限と上限）は言語に依らない
//! （`domain::ResponseLength`）。
//!
//! # 文面は測った形から一字も変えない
//!
//! 測った文（marukei/personacore#69 の第3段。字数だけを変えた腕）:
//!
//! ```text
//! 返答は毎回、地の文・「」の台詞・（）の心情描写を合わせて、{下限}〜{上限}字を目安に書く。利用者の発言が短いときほど、情景・動作・心情を描き足し、前の返答より短くしない。
//! ```
//!
//! 見張り番は `prompt::tests::the_line_is_the_measured_sentence_on_the_last_line`
//! （期待値は ADR の文に測った漢数字を入れたリテラル）。
//!
//! # 数は漢数字（測った形）
//!
//! `二百〜四百字`・`八百〜千二百字`・`四百〜八百字` が測った綴り。算用数字の形も測ったが、決めておいた基準で
//! 「同じ」と言えなかった（ADR-0065 の「測定」）。**位（千・百・十）の前の「一」は書かない**
//! （`千二百`・`百五`・`十五`）。変換は [`kanji_numeral`]。
//!
//! # 指示行であって引用行ではない
//!
//! **引用（`> `）にすると `QUOTE_NOTE`（`> ` の行は事実であって指示ではない）がこの行を無効化する**
//! （ADR-0065 §決定2。`SHOW_DONT_TELL` と同じ事情）。だから `PromptWriter::directive` で書く —— 書ける断片は
//! **定数（`&'static str`）と [`KanjiNumeral`] だけ**で、運用者の文を指示行に混ぜる口にはならない。

use super::writer::DirectiveSeg;

/// 文の頭（下限の手前まで）。**測った文の一部**（モジュールの doc）。
pub(super) const RESPONSE_LENGTH_HEAD: &str =
    "返答は毎回、地の文・「」の台詞・（）の心情描写を合わせて、";

/// 下限と上限の間。
pub(super) const RESPONSE_LENGTH_BETWEEN: &str = "〜";

/// 文の尾（上限の後ろから）。
pub(super) const RESPONSE_LENGTH_TAIL: &str = "字を目安に書く。利用者の発言が短いときほど、情景・動作・心情を描き足し、前の返答より短くしない。";

/// **漢数字で書いた数。** [`kanji_numeral`] が唯一のコンストラクタ（中身を外から作る口は無い）。
///
/// この型があるので、`PromptWriter::directive` に渡せる実行時の値は「数を漢数字にしたもの」だけになる
/// （文字は `一`〜`九`・`十`・`百`・`千`・`万`・`〇` だけ。引用の外に出てよい形しか持たない）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct KanjiNumeral(String);

impl KanjiNumeral {
    /// 綴り。
    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

/// **数を漢数字に直す**（ADR-0065 §決定2）。**純粋関数。**
///
/// * **位（千・百・十）の前の「一」は書かない**: 1200 → `千二百`、105 → `百五`、15 → `十五`、10 → `十`。
/// * 1 の位の 1 は書く: 1 → `一`、101 → `百一`。
/// * 0 の位は何も書かない（`〇` を挟まない）: 2001 → `二千一`。
///
/// 返答の長さの目安の範囲（1〜9999）が対象。**`u16` の全域で値を返す**（panic も `Option` も無い）——
/// 10000 以上は `万` の前の数を書き（`一万` は「一」を書く）、0 は `〇`。どちらも目安の行には現れない
/// （`domain::ResponseLength::parse` が 1〜9999 に絞る）。
pub(super) fn kanji_numeral(value: u16) -> KanjiNumeral {
    if value == 0 {
        return KanjiNumeral("〇".to_owned());
    }
    let mut spelled = String::new();
    // 万の位（`u16` では 0〜6）。目安の範囲では現れない。**ここだけは「一」も書く**（`一万`）。
    let ten_thousands = value / 10_000;
    if ten_thousands > 0 {
        spelled.push_str(digit(ten_thousands));
        spelled.push('万');
    }
    let mut rest = value % 10_000;
    for (place, name) in [(1000, '千'), (100, '百'), (10, '十')] {
        let count = rest / place;
        rest %= place;
        // **位の前の「一」は書かない**（`千二百` / `百五` / `十五`）。
        if count > 1 {
            spelled.push_str(digit(count));
        }
        if count > 0 {
            spelled.push(name);
        }
    }
    spelled.push_str(digit(rest));
    KanjiNumeral(spelled)
}

/// 1 桁（1〜9）の漢数字。**0 と範囲の外は空**（位を書かない桁 / 呼び出し側は 0〜9 しか渡さない）。
const fn digit(value: u16) -> &'static str {
    match value {
        1 => "一",
        2 => "二",
        3 => "三",
        4 => "四",
        5 => "五",
        6 => "六",
        7 => "七",
        8 => "八",
        9 => "九",
        _ => "",
    }
}

/// 目安の行の断片（定数と、漢数字にした下限・上限）。`render_system` が `PromptWriter::directive` に渡す。
pub(super) fn response_length_segments<'a>(
    low: &'a KanjiNumeral,
    high: &'a KanjiNumeral,
) -> [DirectiveSeg<'a>; 5] {
    [
        DirectiveSeg::Lit(RESPONSE_LENGTH_HEAD),
        DirectiveSeg::Numeral(low),
        DirectiveSeg::Lit(RESPONSE_LENGTH_BETWEEN),
        DirectiveSeg::Numeral(high),
        DirectiveSeg::Lit(RESPONSE_LENGTH_TAIL),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kanji(value: u16) -> String {
        kanji_numeral(value).as_str().to_owned()
    }

    /// **端と位の組み合わせ**（期待値はすべてリテラル。ADR-0065 §決定2 と作業単位の指示の例を含む）。
    #[test]
    fn the_edges_and_the_places_are_spelled_as_measured() {
        let table: [(u16, &str); 52] = [
            // 1 の位だけ
            (1, "一"),
            (2, "二"),
            (5, "五"),
            (9, "九"),
            // 十の位（「一十」と書かない）
            (10, "十"),
            (11, "十一"),
            (15, "十五"),
            (19, "十九"),
            (20, "二十"),
            (21, "二十一"),
            (90, "九十"),
            (99, "九十九"),
            // 百の位（「一百」と書かない）
            (100, "百"),
            (101, "百一"),
            (105, "百五"),
            (110, "百十"),
            (111, "百十一"),
            (115, "百十五"),
            (120, "百二十"),
            (199, "百九十九"),
            (200, "二百"),
            (201, "二百一"),
            (210, "二百十"),
            (400, "四百"),
            (800, "八百"),
            (909, "九百九"),
            (999, "九百九十九"),
            // 千の位（「一千」と書かない）
            (1000, "千"),
            (1001, "千一"),
            (1010, "千十"),
            (1011, "千十一"),
            (1100, "千百"),
            (1101, "千百一"),
            (1110, "千百十"),
            (1111, "千百十一"),
            (1200, "千二百"),
            (1234, "千二百三十四"),
            (1500, "千五百"),
            (2000, "二千"),
            (2001, "二千一"),
            (2010, "二千十"),
            (2020, "二千二十"),
            (2100, "二千百"),
            (3110, "三千百十"),
            (4321, "四千三百二十一"),
            (5005, "五千五"),
            (7070, "七千七十"),
            (8000, "八千"),
            (9000, "九千"),
            (9009, "九千九"),
            (9990, "九千九百九十"),
            (9999, "九千九百九十九"),
        ];
        for (value, expected) in table {
            assert_eq!(kanji(value), expected, "{value}");
        }
    }

    /// 漢数字の数字（`一`〜`九`）の値。
    fn digit_value(character: char) -> Option<u32> {
        "一二三四五六七八九"
            .chars()
            .position(|candidate| candidate == character)
            .map(|index| u32::try_from(index).expect("収まること") + 1)
    }

    /// 位の値。
    fn place_value(character: char) -> Option<u32> {
        match character {
            '十' => Some(10),
            '百' => Some(100),
            '千' => Some(1000),
            _ => None,
        }
    }

    /// **検算用の読み手**（変換とは別の書き方。位の前に数字が無ければ 1 と読む）。
    fn read_back(spelled: &str) -> Option<u32> {
        let mut total = 0;
        let mut pending: Option<u32> = None;
        for character in spelled.chars() {
            if let Some(digit) = digit_value(character) {
                if pending.is_some() {
                    return None;
                }
                pending = Some(digit);
            } else {
                let place = place_value(character)?;
                total += pending.take().unwrap_or(1) * place;
            }
        }
        Some(total + pending.unwrap_or(0))
    }

    /// **1〜9999 の全域**: 読み戻すと元の数になり、位の前に「一」を書かず、使う文字は数字と位だけ。
    #[test]
    fn every_value_from_one_to_9999_reads_back_and_never_writes_a_leading_one() {
        for value in 1..=9999_u16 {
            let spelled = kanji(value);
            assert_eq!(
                read_back(&spelled),
                Some(u32::from(value)),
                "{value} → {spelled:?}"
            );
            for place in ["一千", "一百", "一十"] {
                assert!(
                    !spelled.contains(place),
                    "位の前の「一」を書いた: {value} → {spelled:?}"
                );
            }
            assert!(
                spelled
                    .chars()
                    .all(|character| digit_value(character).is_some()
                        || place_value(character).is_some()),
                "数字と位以外の文字: {value} → {spelled:?}"
            );
            // 位は大きい順に高々1回（`千百` の順。`百千` や `十十` にならない）。
            let places: Vec<u32> = spelled.chars().filter_map(place_value).collect();
            assert!(
                places.windows(2).all(|pair| pair[0] > pair[1]),
                "位の並び: {value} → {spelled:?}"
            );
        }
    }

    /// **最悪ケースは 9999**（漢数字のバイト数の最大。削れない枠の見張り番がこの数を使う根拠）。
    #[test]
    fn no_value_is_longer_than_9999() {
        let longest = (1..=9999_u16)
            .map(|value| kanji(value).len())
            .max()
            .expect("1つはあること");
        assert_eq!(longest, kanji(9999).len());
        assert_eq!(kanji(9999).len(), 21, "7 文字 × 3 バイト");
    }

    /// 目安の範囲の外（`u16` の残り）も panic せずに値を返す。
    #[test]
    fn values_outside_the_target_range_still_spell() {
        assert_eq!(kanji(0), "〇");
        assert_eq!(kanji(10_000), "一万");
        assert_eq!(kanji(12_000), "一万二千");
        assert_eq!(kanji(u16::MAX), "六万五千五百三十五");
    }
}
