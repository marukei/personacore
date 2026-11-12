//! **上流が本文に漏らした思考（`<think>…</think>`）を剥がす規則**
//! （[ADR-0053](../../docs/adr/0053-strip-leaked-thinking-before-every-reader.md) / 作業単位 B-61）。
//!
//! **I/O を持たない純粋関数**。本文の読み手（`server::turn` の分解・`prompt::history_turn`・
//! 裏方のサブLLMへ渡す `TurnJob::character_message`）は全部ここを通る。
//! 置き場を `domain` にしたのは、`prompt` と `parse` が本番コードで互いに依存しないため
//! （ADR-0053 §決定1）。
//!
//! **画面（`web/src/lib/thinking.ts`）は同じ規則を手で写している。** 両側が
//! `tests/fixtures/thinking_corpus.tsv`（手書きの表）を読み、食い違えばどちらかが赤くなる
//! （`tests/thinking_corpus.rs` / `web/tests/thinking-corpus.test.ts`）。
//!
//! # 剥がさないもの
//!
//! **`message.raw_content`（保存）は剥がさない**（CLAUDE.md 絶対制約）。
//! OpenAI 互換 API の使い手に返す本文（非ストリーミングの応答本文と SSE）も、
//! この作業単位では剥がさない（ADR-0053 §決定6）。

use std::borrow::Cow;

use super::text::{LINE_BREAKS, TRIMMABLE};

/// 開きタグの頭。**大文字小文字は区別する**（観測は小文字だけ / ADR-0053 §決定2）。
const OPEN_HEAD: &str = "<think";

/// 閉じタグの頭。
const CLOSE_HEAD: &str = "</think";

/// タグの頭と `>` のあいだに許す文字（**半角空白と TAB だけ**。0個以上）。
///
/// 空白入りを受けるのは、**personacore 自身の無害化
/// （`prompt::safe::neutralize_control_tokens`）がその綴りを作る**から（ADR-0053 §背景）。
/// 全角空白や改行は受けない。
const TAG_PADDING: [char; 2] = [' ', '\t'];

/// タグの終わり。
const TAG_END: char = '>';

/// タグの始まりの文字（`OPEN_HEAD` / `CLOSE_HEAD` の先頭）。**ASCII なので、
/// UTF-8 の多バイト列の途中には現れない**——ここで割ったバイト位置は必ず文字の境目。
const TAG_START: char = '<';

/// 剥がした結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrippedThinking<'a> {
    /// 剥がした後の本文。**剥がすものが無ければ元の文字列を借りたまま**（1バイトも変えない）。
    pub text: Cow<'a, str>,
    /// 剥がした**思考のブロックの数**。余った閉じタグ（タグだけを捨てる）は数えない。
    pub blocks: usize,
    /// **閉じないまま末尾に達したか**（開きタグから末尾までを思考とみなして捨てた）。
    pub unclosed: bool,
}

/// 上流が本文に漏らした思考を剥がす（ADR-0053 §決定2・3）。**panic しない。**
///
/// # 規則
///
/// 左から走査し、「思考の中か」を状態として持つ。
///
/// * タグは `<think` / `</think` ＋ 0個以上の半角空白・TAB ＋ `>`。大文字小文字は区別する。
/// * **最初に見つかったタグが閉じタグなら、先頭から思考が始まっていた**とみなす。
/// * 思考の外で開きタグ → 入る。思考の中で閉じタグ → 出る。思考の中の開きタグは思考の一部。
/// * 思考の外の閉じタグ（余り）は**タグだけ**を捨てる。
/// * **閉じないまま末尾に達したら、開きタグから末尾までを捨てる。**
/// * 剥がした境目に残る空行は落とす。**行頭の字下げ（全角空白 U+3000）は残す。**
///
/// # 境目の空行
///
/// 「空行」は、`TRIMMABLE` のうち行区切りでない文字（半角空白・TAB・全角空白・NBSP）
/// だけでできた行。行の区切りは `LINE_BREAKS`（7点。CRLF は CR と LF の2つとして数える）。
///
/// 1. 剥がした範囲の**直後**の空行を落とす。
/// 2. 先頭の断片（最初に剥がした範囲より前）が**空白だけ**なら、その空行を落とす
///    （行頭の字下げにあたる、改行で終わらない最後の空白は残る）。
/// 3. 剥がした範囲の**後ろに何も残らない**なら、その**直前**の空行を落とす。
///
/// **本文の途中で剥がしても段落の区切りは1つ残る**（直前の空行は、後ろに本文があれば落とさない）。
pub fn strip_leaked_thinking(raw: &str) -> StrippedThinking<'_> {
    // ---- 1. 捨てるバイト範囲を決める（タグの位置はすべて ASCII の境目） ----
    let mut removals: Vec<(usize, usize)> = Vec::new();
    let mut blocks = 0_usize;
    let mut unclosed = false;
    // 思考の中なら、その開きタグの位置。
    let mut open_at: Option<usize> = None;
    let mut seen_a_tag = false;
    let mut from = 0_usize;

    while let Some(tag) = next_tag(raw, from) {
        match (tag.kind, open_at) {
            // **最初のタグが閉じタグ** —— 先頭から思考が始まっていた。
            (TagKind::Close, None) if !seen_a_tag => {
                removals.push((0, tag.end));
                blocks += 1;
            }
            // 余った閉じタグ。**タグだけ**を捨て、ブロックには数えない。
            (TagKind::Close, None) => removals.push((tag.start, tag.end)),
            (TagKind::Close, Some(start)) => {
                removals.push((start, tag.end));
                blocks += 1;
                open_at = None;
            }
            (TagKind::Open, None) => open_at = Some(tag.start),
            // 思考の中の開きタグは思考の一部。
            (TagKind::Open, Some(_)) => {}
        }
        seen_a_tag = true;
        from = tag.end;
    }

    // **閉じないまま末尾** —— 開きタグから末尾までを思考とみなす（§決定3）。
    if let Some(start) = open_at {
        removals.push((start, raw.len()));
        blocks += 1;
        unclosed = true;
    }

    if removals.is_empty() {
        return StrippedThinking {
            text: Cow::Borrowed(raw),
            blocks,
            unclosed,
        };
    }

    // ---- 2. 残す断片（剥がした範囲の補集合）を集める ----
    // `pieces[0]` は最初の範囲より前、`pieces[i]`（i >= 1）は i 番目の範囲の後ろ。
    let mut pieces: Vec<&str> = Vec::with_capacity(removals.len() + 1);
    let mut cursor = 0_usize;
    for (start, end) in &removals {
        pieces.push(raw.get(cursor..*start).unwrap_or_default());
        cursor = *end;
    }
    pieces.push(raw.get(cursor..).unwrap_or_default());

    // ---- 3. 境目の空行を落とす ----
    // (1) 剥がした範囲の直後。
    for piece in pieces.iter_mut().skip(1) {
        *piece = drop_leading_blank_lines(piece);
    }
    // (2) 先頭の断片が空白だけなら、その空行（字下げは残る）。
    if let Some(first) = pieces.first_mut()
        && first.chars().all(is_whitespace)
    {
        *first = drop_leading_blank_lines(first);
    }
    // (3) 剥がした範囲の後ろに何も残らないなら、その直前。
    let mut nothing_after = true;
    for index in (0..pieces.len()).rev() {
        let is_last = index + 1 == pieces.len();
        if !is_last
            && nothing_after
            && let Some(piece) = pieces.get_mut(index)
        {
            *piece = drop_trailing_blank_lines(piece);
        }
        if pieces.get(index).is_some_and(|piece| !piece.is_empty()) {
            nothing_after = false;
        }
    }

    StrippedThinking {
        text: Cow::Owned(pieces.concat()),
        blocks,
        unclosed,
    }
}

/// **思考を剥がした後に本文が残るか**（[ADR-0059](../../docs/adr/0059-thinking-and-response-room-in-the-upstream-profile.md) §決定3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyBody {
    /// 剥がした後に、空白でない文字が1つ以上残る。**上流が文字を返した回**
    /// （空の `「」` だけ・宙に浮いた強調の印だけも、こちら）。
    Present,
    /// 剥がした後が**空か空白だけ**。`unclosed` は、閉じない思考を剥がした回か。
    Blank {
        /// 閉じないまま末尾に達した思考を剥がしたか（[`StrippedThinking::unclosed`]）。
        unclosed: bool,
    },
}

/// **本文が空の回か**を決める（ADR-0059 §決定3 —— 空の本文を1回引き直す判定の材料）。**panic しない。**
///
/// [`strip_leaked_thinking`] で剥がした後の文字列が、空か空白だけなら [`ReplyBody::Blank`]。
/// 空白は `TRIMMABLE` と `LINE_BREAKS`（剥がしの空行と同じ集合 / `domain::text` の doc。広げない）。
///
/// **分解の `ParseOutcome::Empty` とは別の判定**である —— あちらは空の `「」` だけ・宙に浮いた強調の印だけも
/// 空とみなすが、それは上流が文字を返した回なので、引き直しの対象にしない。
pub fn reply_body(raw: &str) -> ReplyBody {
    let stripped = strip_leaked_thinking(raw);
    if stripped.text.chars().all(is_whitespace) {
        ReplyBody::Blank {
            unclosed: stripped.unclosed,
        }
    } else {
        ReplyBody::Present
    }
}

/// タグの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagKind {
    Open,
    Close,
}

/// 見つけたタグ。`start..end` は `raw` のバイト範囲（**両端とも文字の境目**）。
#[derive(Debug, Clone, Copy)]
struct Tag {
    kind: TagKind,
    start: usize,
    end: usize,
}

/// `from` 以降で最初のタグを探す。
fn next_tag(raw: &str, from: usize) -> Option<Tag> {
    let mut search = from;
    loop {
        let offset = raw.get(search..)?.find(TAG_START)?;
        let start = search + offset;
        let rest = raw.get(start..)?;
        if let Some(length) = tag_length(rest, CLOSE_HEAD) {
            return Some(Tag {
                kind: TagKind::Close,
                start,
                end: start + length,
            });
        }
        if let Some(length) = tag_length(rest, OPEN_HEAD) {
            return Some(Tag {
                kind: TagKind::Open,
                start,
                end: start + length,
            });
        }
        // `<` は1バイト。次の文字の境目から探し直す。
        search = start + TAG_START.len_utf8();
    }
}

/// `rest` の先頭が `head` ＋ 空白・TAB ＋ `>` なら、そのバイト長。
fn tag_length(rest: &str, head: &str) -> Option<usize> {
    let after_head = rest.strip_prefix(head)?;
    let after_padding = after_head.trim_start_matches(TAG_PADDING);
    let after_end = after_padding.strip_prefix(TAG_END)?;
    Some(rest.len() - after_end.len())
}

/// 空行を作る空白か（`TRIMMABLE` のうち行区切りでないもの）。
fn is_padding(character: char) -> bool {
    TRIMMABLE.contains(&character) && !LINE_BREAKS.contains(&character)
}

/// 空白か行区切りか。
fn is_whitespace(character: char) -> bool {
    is_padding(character) || LINE_BREAKS.contains(&character)
}

/// 先頭の空行を落とす。**改行で終わらない最後の行（字下げ）は残す。**
fn drop_leading_blank_lines(text: &str) -> &str {
    let mut rest = text;
    while let Some((at, line_break)) = rest
        .char_indices()
        .find(|(_, character)| LINE_BREAKS.contains(character))
    {
        let line = rest.get(..at).unwrap_or_default();
        if !line.chars().all(is_padding) {
            break;
        }
        rest = rest.get(at + line_break.len_utf8()..).unwrap_or_default();
    }
    rest
}

/// 末尾の空行を落とす（最後の行区切りの後ろが空白だけなら、その行区切りごと）。
fn drop_trailing_blank_lines(text: &str) -> &str {
    let mut rest = text;
    while let Some((at, line_break)) = rest
        .char_indices()
        .rev()
        .find(|(_, character)| LINE_BREAKS.contains(character))
    {
        let tail = rest.get(at + line_break.len_utf8()..).unwrap_or_default();
        if !tail.chars().all(is_padding) {
            break;
        }
        rest = rest.get(..at).unwrap_or_default();
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本文（字下げつき）。**思考のどの断片とも別の文字列。**
    const BODY: &str = "\u{3000}彼女は笑った。「おかえりなさい」";

    fn strip(raw: &str) -> (String, usize, bool) {
        let stripped = strip_leaked_thinking(raw);
        (
            stripped.text.into_owned(),
            stripped.blocks,
            stripped.unclosed,
        )
    }

    #[test]
    fn a_reply_without_tags_is_returned_borrowed_and_untouched() {
        for raw in [
            BODY,
            "",
            "「<think」と書いてみた",
            "<thinking>は別の綴り</thinking>",
            "a < b > c",
        ] {
            let stripped = strip_leaked_thinking(raw);
            assert!(
                matches!(stripped.text, Cow::Borrowed(text) if text == raw),
                "剥がすものが無ければ元を借りたまま返すこと: {raw:?} -> {stripped:?}"
            );
            assert_eq!(stripped.blocks, 0, "{raw:?}");
            assert!(!stripped.unclosed, "{raw:?}");
        }
    }

    #[test]
    fn a_leading_closed_thought_is_removed_and_the_indent_survives() {
        // **C-20 の多数派の形**（先頭に閉じた思考、後ろに本文）。
        let raw = format!("<think>\nThe user is asking.\n</think>\n\n{BODY}");
        assert_eq!(strip(&raw), (BODY.to_owned(), 1, false));
    }

    #[test]
    fn spaced_and_tabbed_spellings_are_tags() {
        // **personacore の無害化が作る綴り**（`<think >`）と、TAB・複数の空白。
        for (open, close) in [
            ("<think >", "</think >"),
            ("<think>", "</think >"),
            ("<think >", "</think>"),
            ("<think\t>", "</think\t>"),
            ("<think  \t >", "</think \t>"),
        ] {
            let raw = format!("{open}考え中{close}\n\n{BODY}");
            assert_eq!(strip(&raw), (BODY.to_owned(), 1, false), "{raw:?}");
        }
    }

    #[test]
    fn other_spellings_are_not_tags() {
        // **大文字・全角空白・改行・頭の前の空白・別の語**は受けない（観測が無い / §決定2）。
        for raw in [
            "<THINK>考え</THINK>本文",
            "<Think>考え</Think>本文",
            "<think\u{3000}>考え</think\u{3000}>本文",
            "<think\n>考え</think\n>本文",
            "< think>考え< /think>本文",
            "<thinking>考え</thinking>本文",
            "<think/>本文",
        ] {
            let stripped = strip_leaked_thinking(raw);
            assert_eq!(stripped.text, raw, "タグとして読まないこと: {raw:?}");
            assert_eq!(stripped.blocks, 0, "{raw:?}");
        }
    }

    #[test]
    fn a_thought_with_no_body_leaves_nothing() {
        // **思考だけで本文が無い**（B+L 5）。空になれば既存の空の経路に入る。
        let raw = "<think >\nShe lives with her dog.\n</think >";
        assert_eq!(strip(raw), (String::new(), 1, false));
    }

    #[test]
    fn a_leading_close_tag_alone_means_the_thought_began_at_the_start() {
        // **テンプレートが開きを生成の頭に置いた形**（B+L 1）。
        assert_eq!(
            strip(&format!("</think>\n\n{BODY}")),
            (BODY.to_owned(), 1, false)
        );
        // 閉じタグの前に思考の文がある形も、先頭から捨てる。
        assert_eq!(
            strip(&format!("The user asks.\n</think >\n\n{BODY}")),
            (BODY.to_owned(), 1, false)
        );
    }

    #[test]
    fn two_thoughts_and_a_stray_close_tag_leave_only_the_body() {
        // **B の turn 32 の形**（先頭に思考2つと、余った閉じタグ）。
        let raw =
            format!("<think >First.</think >\n\n<think >Second.</think >\n</think>\n\n{BODY}");
        assert_eq!(strip(&raw), (BODY.to_owned(), 2, false));
    }

    #[test]
    fn an_unclosed_thought_runs_to_the_end() {
        // **閉じないまま末尾**（§決定3）。開きタグの前は残る。
        assert_eq!(
            strip("<think>\nThe user is asking"),
            (String::new(), 1, true)
        );
        assert_eq!(
            strip("「やあ」<think>独白が続く\n\n「本文らしいもの」"),
            ("「やあ」".to_owned(), 1, true)
        );
        // 閉じたブロックの後で閉じない形も数える。
        assert_eq!(
            strip(&format!("<think>a</think>\n\n{BODY}<think>b")),
            (BODY.to_owned(), 2, true)
        );
    }

    #[test]
    fn an_open_tag_inside_a_thought_is_part_of_the_thought() {
        assert_eq!(
            strip(&format!("<think>a<think>b</think>\n{BODY}")),
            (BODY.to_owned(), 1, false)
        );
    }

    #[test]
    fn a_stray_close_tag_after_a_thought_loses_only_the_tag() {
        // 余った閉じタグは**タグだけ**。ブロックには数えない。
        assert_eq!(
            strip("<think>a</think>「やあ」</think>と笑った。"),
            ("「やあ」と笑った。".to_owned(), 1, false)
        );
    }

    #[test]
    fn a_thought_in_the_middle_keeps_one_paragraph_break() {
        assert_eq!(
            strip("「一つ目」\n\n<think>x</think>\n\n「二つ目」"),
            ("「一つ目」\n\n「二つ目」".to_owned(), 1, false)
        );
        assert_eq!(
            strip("「やあ」<think>x</think>と笑った。"),
            ("「やあ」と笑った。".to_owned(), 1, false)
        );
    }

    #[test]
    fn a_thought_at_the_end_takes_the_blank_lines_before_it() {
        assert_eq!(
            strip("「やあ」\n\n<think>x</think>\n"),
            ("「やあ」".to_owned(), 1, false)
        );
    }

    #[test]
    fn blank_lines_before_a_leading_thought_go_and_the_indent_stays() {
        assert_eq!(
            strip(&format!("\n \u{3000}\n<think>x</think>\n{BODY}")),
            (BODY.to_owned(), 1, false)
        );
        // **改行で終わらない空白は字下げ**として残る。
        assert_eq!(
            strip("\u{3000}<think>x</think>本文"),
            ("\u{3000}本文".to_owned(), 1, false)
        );
    }

    #[test]
    fn crlf_blank_lines_after_a_thought_are_dropped() {
        assert_eq!(
            strip(&format!("<think>x</think>\r\n\r\n{BODY}")),
            (BODY.to_owned(), 1, false)
        );
    }

    #[test]
    fn a_line_that_is_not_blank_after_a_thought_is_kept_whole() {
        // **U+2009 は `TRIMMABLE` に無い**ので、その行は空行ではない。
        assert_eq!(
            strip("<think>x</think>\u{2009}\n本文"),
            ("\u{2009}\n本文".to_owned(), 1, false)
        );
    }

    #[test]
    fn it_never_panics_on_any_prefix_of_tricky_inputs() {
        // **バイト添字で多バイト文字を割らないこと**（パーサは panic しない / 絶対制約）。
        let samples = [
            "「あ」<think >思考😀</think >\n\n\u{3000}本文<",
            "</think\t\t>😀<think",
            "<<think<</think<>>あ</",
            "<think 😀>あ</think \u{3000}>",
        ];
        for sample in samples {
            for (at, _) in sample.char_indices().chain([(sample.len(), ' ')]) {
                let prefix = &sample[..at];
                let stripped = strip_leaked_thinking(prefix);
                assert!(stripped.text.len() <= prefix.len(), "{prefix:?}");
            }
        }
    }

    // ---- 本文が空の回の判定（ADR-0059 §決定3 / 作業単位 A-18b）----

    /// **剥がした後が空か空白だけなら空**。閉じない思考だったかも返す。
    ///
    /// 空白は `TRIMMABLE` と `LINE_BREAKS`（剥がしの空行と同じ集合）。
    #[test]
    fn a_reply_that_is_blank_after_stripping_is_blank() {
        for (raw, unclosed) in [
            // 思考を別欄で返す上流で、本文が空のまま終わった回。
            ("", false),
            // 空白だけ（半角・全角・TAB・改行・NBSP・U+2028）。
            (" \t\n\r\n\u{3000}\u{00A0}\u{2028}", false),
            // 本文の中で閉じた思考だけ。
            ("<think>\nThe user is home.\n</think>\n\n", false),
            ("<think >a</think >\n<think >b</think >", false),
            // 閉じない思考（剥がして空）。
            ("<think>\nThe user is asking", true),
            ("\n<think>考え中", true),
        ] {
            assert_eq!(reply_body(raw), ReplyBody::Blank { unclosed }, "{raw:?}");
        }
    }

    /// **上流が文字を返した回は空ではない**（`ParseOutcome::Empty` に落ちる形でも）。
    ///
    /// 空の `「」` だけ・宙に浮いた強調の印だけは、分解では `Empty` になるが、引き直しの対象にしない
    /// （ADR-0059 §決定3）。**閉じない思考の前に本文がある回**も空ではない。
    #[test]
    fn a_reply_with_any_visible_character_is_not_blank() {
        for raw in [
            "「」",
            "**",
            "`",
            "本文",
            "<think>x</think>「おかえり」",
            "「おかえり」<think>まだ考え",
            // U+2009 は空白の集合に無い（`TRIMMABLE` を広げない / `domain::text` の doc）。
            "\u{2009}",
        ] {
            assert_eq!(reply_body(raw), ReplyBody::Present, "{raw:?}");
        }
    }
}
