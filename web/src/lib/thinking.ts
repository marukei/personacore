/**
 * **上流が本文に漏らした思考（`<think>…</think>`）を剥がす**（表示のため / 作業単位 B-61 / ADR-0053）。
 *
 * # ⚠ **これは `src/domain/thinking.rs` の `strip_leaked_thinking` を手で写したものである**
 *
 * **生成していない。** 写しの性質（Rust 側を変えると黙って古びる）は ADR-0031 §6 が持っている。
 * 境目の空行を決める **`TRIMMABLE` / `LINE_BREAKS` は `emphasis.ts` の写しを使う**
 * （`src/domain/text.rs` の2つの表。ここで3つ目の写しを作らない）。
 *
 * # なぜ画面でも剥がすのか
 *
 * 画面に届くのは**上流の生出力そのもの**（SSE の増分も、読み直した `raw_content` も）。
 * サーバは保存する `dialogue` / `narration` と次のターンの履歴から剥がすが、
 * **生出力は剥がさずに保存する**（CLAUDE.md 絶対制約）ので、**画面の側でも剥がす。**
 *
 * # 見張り番は Rust 側と共有の表
 *
 * **`tests/fixtures/thinking_corpus.tsv`**（リポジトリのルート側）を、
 * `tests/thinking_corpus.rs`（Rust）と `web/tests/thinking-corpus.test.ts`（こちら）の
 * **両方が読む。** 表は**手書き**で、実装から作り直す仕掛けを持たない。
 * **表はタグの綴りを全角空白・NBSP・改行で突いている** ——
 * **正規表現の空白類（`\s`）でタグを読む実装は、そこで落ちる。**
 */

import { LINE_BREAKS, TRIMMABLE } from "./emphasis";

/** 開きタグの頭。**大文字小文字は区別する。** */
const OPEN_HEAD = "<think";

/** 閉じタグの頭。 */
const CLOSE_HEAD = "</think";

/**
 * タグの頭と `>` のあいだに許す文字（**半角空白と TAB だけ**。0個以上）。
 *
 * **`\s` で代用しないこと。** 全角空白・NBSP・改行まで受けてしまい、Rust と食い違う。
 */
const TAG_PADDING: readonly string[] = [" ", "\u0009"];

/** タグの終わり。 */
const TAG_END = ">";

/** タグの始まりの文字。 */
const TAG_START = "<";

/** 剥がした結果。 */
export interface StrippedThinking {
  /** 剥がした後の本文。**剥がすものが無ければ元の文字列そのまま。** */
  text: string;
  /** 剥がした思考のブロックの数。余った閉じタグは数えない。 */
  blocks: number;
  /** 閉じないまま末尾に達したか。 */
  unclosed: boolean;
}

/** 見つけたタグ。`start` / `end` は UTF-16 の位置（**タグは ASCII なのでサロゲートを割らない**）。 */
interface Tag {
  close: boolean;
  start: number;
  end: number;
}

/** `raw` の `start` から `head` ＋ 空白・TAB ＋ `>` が始まるなら、その長さ。 */
function tagLength(raw: string, start: number, head: string): number | null {
  if (!raw.startsWith(head, start)) {
    return null;
  }
  let at = start + head.length;
  while (at < raw.length && TAG_PADDING.includes(raw.charAt(at))) {
    at += 1;
  }
  return raw.charAt(at) === TAG_END ? at + 1 - start : null;
}

/** `from` 以降で最初のタグを探す。 */
function nextTag(raw: string, from: number): Tag | null {
  let search = from;
  for (;;) {
    const start = raw.indexOf(TAG_START, search);
    if (start < 0) {
      return null;
    }
    const close = tagLength(raw, start, CLOSE_HEAD);
    if (close !== null) {
      return { close: true, start, end: start + close };
    }
    const open = tagLength(raw, start, OPEN_HEAD);
    if (open !== null) {
      return { close: false, start, end: start + open };
    }
    search = start + 1;
  }
}

/** 空行を作る空白か（`TRIMMABLE` のうち行区切りでないもの）。 */
function isPadding(character: string): boolean {
  return TRIMMABLE.includes(character) && !LINE_BREAKS.includes(character);
}

/** 空白か行区切りか。 */
function isWhitespace(character: string): boolean {
  return isPadding(character) || LINE_BREAKS.includes(character);
}

/** 文字列の全部が `test` を満たすか（**コードポイントで回す**）。 */
function every(text: string, test: (character: string) => boolean): boolean {
  return [...text].every(test);
}

/**
 * 最初の行区切りの位置。**行区切りは全部 BMP の1単位**なので、UTF-16 の位置で比べてよい。
 */
function firstLineBreak(text: string): number {
  for (let at = 0; at < text.length; at += 1) {
    if (LINE_BREAKS.includes(text.charAt(at))) {
      return at;
    }
  }
  return -1;
}

/** 最後の行区切りの位置。 */
function lastLineBreak(text: string): number {
  for (let at = text.length - 1; at >= 0; at -= 1) {
    if (LINE_BREAKS.includes(text.charAt(at))) {
      return at;
    }
  }
  return -1;
}

/** 先頭の空行を落とす。**改行で終わらない最後の行（字下げ）は残す。** */
function dropLeadingBlankLines(text: string): string {
  let rest = text;
  for (;;) {
    const at = firstLineBreak(rest);
    if (at < 0 || !every(rest.slice(0, at), isPadding)) {
      return rest;
    }
    rest = rest.slice(at + 1);
  }
}

/** 末尾の空行を落とす（最後の行区切りの後ろが空白だけなら、その行区切りごと）。 */
function dropTrailingBlankLines(text: string): string {
  let rest = text;
  for (;;) {
    const at = lastLineBreak(rest);
    if (at < 0 || !every(rest.slice(at + 1), isPadding)) {
      return rest;
    }
    rest = rest.slice(0, at);
  }
}

/**
 * 上流が本文に漏らした思考を剥がす（ADR-0053 §決定2・3）。
 *
 * 規則の文章は `src/domain/thinking.rs` の `strip_leaked_thinking` の doc にある（ここに写さない）。
 */
export function stripThinking(raw: string): StrippedThinking {
  // ---- 1. 捨てる範囲を決める ----
  const removals: [number, number][] = [];
  let blocks = 0;
  let unclosed = false;
  let openAt: number | null = null;
  let seenATag = false;
  let from = 0;

  for (let tag = nextTag(raw, from); tag !== null; tag = nextTag(raw, from)) {
    if (tag.close) {
      if (openAt !== null) {
        removals.push([openAt, tag.end]);
        blocks += 1;
        openAt = null;
      } else if (!seenATag) {
        // **最初のタグが閉じタグ** —— 先頭から思考が始まっていた。
        removals.push([0, tag.end]);
        blocks += 1;
      } else {
        // 余った閉じタグ。**タグだけ。**
        removals.push([tag.start, tag.end]);
      }
    } else if (openAt === null) {
      openAt = tag.start;
    }
    seenATag = true;
    from = tag.end;
  }

  if (openAt !== null) {
    // **閉じないまま末尾** —— 開きタグから末尾まで。
    removals.push([openAt, raw.length]);
    blocks += 1;
    unclosed = true;
  }

  if (removals.length === 0) {
    return { text: raw, blocks, unclosed };
  }

  // ---- 2. 残す断片 ----
  const pieces: string[] = [];
  let cursor = 0;
  for (const [start, end] of removals) {
    pieces.push(raw.slice(cursor, start));
    cursor = end;
  }
  pieces.push(raw.slice(cursor));

  // ---- 3. 境目の空行 ----
  // (1) 剥がした範囲の直後。
  for (let index = 1; index < pieces.length; index += 1) {
    pieces[index] = dropLeadingBlankLines(pieces[index] as string);
  }
  // (2) 先頭の断片が空白だけなら、その空行（字下げは残る）。
  const first = pieces[0] as string;
  if (every(first, isWhitespace)) {
    pieces[0] = dropLeadingBlankLines(first);
  }
  // (3) 剥がした範囲の後ろに何も残らないなら、その直前。
  let nothingAfter = true;
  for (let index = pieces.length - 1; index >= 0; index -= 1) {
    if (index + 1 < pieces.length && nothingAfter) {
      pieces[index] = dropTrailingBlankLines(pieces[index] as string);
    }
    if ((pieces[index] as string) !== "") {
      nothingAfter = false;
    }
  }

  return { text: pieces.join(""), blocks, unclosed };
}
