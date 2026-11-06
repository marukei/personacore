/**
 * **`thinking.ts` が `src/domain/thinking.rs` の写しとして成立しているか**（作業単位 B-61 / ADR-0053）。
 *
 * # 権威は Rust 側にある
 *
 * 読んでいる `tests/fixtures/thinking_corpus.tsv` は**手書きの表**で、
 * **`tests/thinking_corpus.rs`（Rust）も同じファイルを読む**（作りは `emphasis-corpus.test.ts` と同じ）。
 *
 * | 何が起きた | 赤くなるのは | 直すのは |
 * |---|---|---|
 * | `src/domain/thinking.rs` の規則を変えた | **Rust 側** | 表 → `thinking.ts` |
 * | `thinking.ts` が Rust に追随していない | **こちら** | `thinking.ts` |
 *
 * **⚠ 表を書き換えて緑にしないこと。** 表は人の決定であって、実装の写しではない。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { stripThinking } from "../src/lib/thinking";

/** **`tests/thinking_corpus.rs` の `fixture_path` と同じ場所。** */
const FIXTURE = fileURLToPath(
  new URL("../../tests/fixtures/thinking_corpus.tsv", import.meta.url),
);

/** 見出しに書いてある件数（`# 件数: 51`）。**読み落としの検出に使う。** */
const COUNT_PATTERN = /^# 件数: (\d+)$/;

/**
 * 行数の下限。**`tests/thinking_corpus.rs` の `MIN_ROWS` と同じ値をここにも置く。**
 *
 * **写しに見えるが、そうではない。** 見出しの件数から作ると、
 * **行を削って件数も直す**変え方が両方の側で緑になる。
 */
const MIN_ROWS = 45;

/** 逃げ方に使う文字（U+005C）。 */
const BACKSLASH = String.fromCharCode(0x5c);

/**
 * 逃げ方の接頭辞。**16進1〜6桁と `}` が続く**（`tests/thinking_corpus.rs` の `UNICODE_ESCAPE` と同じ綴り。
 * 括弧なしにしていない理由は表の見出しにある）。
 */
const UNICODE_ESCAPE = BACKSLASH + "u{";

/** 逃げ方の終わり。 */
const ESCAPE_END = "}";

/** バックスラッシュ自身の逃げ方。 */
const DOUBLED_BACKSLASH = BACKSLASH + BACKSLASH;

/** 16進1〜6桁か。 */
const HEX_DIGITS = /^[0-9A-Fa-f]{1,6}$/;

/** 表の1行。 */
interface Row {
  /** 剥がした思考のブロックの数。 */
  blocks: number;
  /** 閉じないまま末尾に達したか。 */
  unclosed: boolean;
  /** 剥がした後の本文。 */
  stripped: string;
  /** 上流の生出力にあたる入力。 */
  source: string;
}

/**
 * 欄を読む。**受ける逃げ方は2つだけ**で、他の綴りでは落とす
 * （`emphasis-corpus.test.ts` の `decode` と同じ規則）。
 */
function decode(field: string): string {
  let out = "";
  let rest = field;

  for (;;) {
    const at = rest.indexOf(BACKSLASH);
    if (at < 0) {
      return out + rest;
    }
    out += rest.slice(0, at);
    const tail = rest.slice(at);

    if (tail.startsWith(UNICODE_ESCAPE)) {
      const from = UNICODE_ESCAPE.length;
      const end = tail.indexOf(ESCAPE_END, from);
      if (end < 0) {
        throw new Error(`逃げ方が閉じていません: ${JSON.stringify(field)}`);
      }
      const digits = tail.slice(from, end);
      if (!HEX_DIGITS.test(digits)) {
        throw new Error(`逃げ方の中が16進1〜6桁ではありません: ${JSON.stringify(field)}`);
      }
      out += String.fromCodePoint(Number.parseInt(digits, 16));
      rest = tail.slice(end + ESCAPE_END.length);
      continue;
    }
    if (tail.startsWith(DOUBLED_BACKSLASH)) {
      out += BACKSLASH;
      rest = tail.slice(DOUBLED_BACKSLASH.length);
      continue;
    }
    throw new Error(`受けていない逃げ方です: ${JSON.stringify(field)}`);
  }
}

/** 「閉じなかったか」の欄を読む。**`0` と `1` 以外は落とす。** */
function decodeFlag(field: string): boolean {
  if (field === "0") {
    return false;
  }
  if (field === "1") {
    return true;
  }
  throw new Error(`閉じなかったかの欄が 0 / 1 ではありません: ${JSON.stringify(field)}`);
}

/** 行の区切り。**行末の CR は落とす**（`core.autocrlf=true` のリポジトリ）。 */
const LF = String.fromCharCode(0x0a);
const CR = String.fromCharCode(0x0d);
const TAB = String.fromCharCode(0x09);

/** 表を読む。 */
function readTable(): { rows: Row[]; declaredCount: number; text: string } {
  const text = readFileSync(FIXTURE, "utf8");
  const rows: Row[] = [];
  let declaredCount: number | null = null;

  for (const raw of text.split(LF)) {
    const line = raw.endsWith(CR) ? raw.slice(0, -1) : raw;
    if (line.startsWith("#")) {
      const found = COUNT_PATTERN.exec(line);
      if (found?.[1] !== undefined) {
        declaredCount = Number.parseInt(found[1], 10);
      }
      continue;
    }
    if (line === "") {
      continue;
    }
    const fields = line.split(TAB);
    if (fields.length !== 4) {
      throw new Error(`表の欄が4つではありません: ${JSON.stringify(line)}`);
    }
    const [blocks, unclosed, stripped, source] = fields as [string, string, string, string];
    rows.push({
      blocks: Number.parseInt(blocks, 10),
      unclosed: decodeFlag(unclosed),
      stripped: decode(stripped),
      source: decode(source),
    });
  }

  if (declaredCount === null) {
    throw new Error("表の見出しに件数がありません");
  }
  return { rows, declaredCount, text };
}

/**
 * **生で書くと見えない書式文字**のうち、表に紛れ込みうる主なものの範囲（両端を含む）。
 * 一般カテゴリ Cf の全部ではなく、`U+E0001`〜`U+E007F` は未割り当ての位置も含む。
 *
 * **`tests/thinking_corpus.rs` の `FORMAT_RANGES` と同じ範囲。** `\p{Cf}` を使わないのは、
 * Rust 側がカテゴリを引けず範囲で持つので、**数える集合を両側で同じにするため**。
 * `\s` は U+FEFF を空白と認めるが U+200B を認めず、Rust の `is_whitespace` はどちらも認めない
 * —— この一覧が無いと両側で数える文字がずれる（レビューの指摘）。
 */
const FORMAT_RANGES: readonly (readonly [number, number])[] = [
  [0x00ad, 0x00ad], // SOFT HYPHEN
  [0x061c, 0x061c], // ARABIC LETTER MARK
  [0x180e, 0x180e], // MONGOLIAN VOWEL SEPARATOR
  [0x200b, 0x200f], // ZERO WIDTH SPACE 〜 RIGHT-TO-LEFT MARK
  [0x202a, 0x202e], // 双方向の埋め込み・上書き
  [0x2060, 0x2064], // WORD JOINER 〜 INVISIBLE PLUS
  [0x2066, 0x206f], // 双方向の分離 〜 NOMINAL DIGIT SHAPES
  [0xfeff, 0xfeff], // ZERO WIDTH NO-BREAK SPACE（BOM）
  [0xfff9, 0xfffb], // INTERLINEAR ANNOTATION
  [0xe0001, 0xe007f], // タグ文字
];

/** 生で入っていてはいけない文字か（TAB・LF・CR・半角空白だけが生で入ってよい）。 */
function isRawInvisible(character: string): boolean {
  if ([" ", TAB, LF, CR].includes(character)) {
    return false;
  }
  const code = character.codePointAt(0) ?? 0;
  return (
    /\s/u.test(character) ||
    /\p{Cc}/u.test(character) ||
    FORMAT_RANGES.some(([low, high]) => low <= code && code <= high)
  );
}

/** **走査の本体。** 生の不可視文字の位置（符号位置の番号）と符号位置を返す。 */
function rawInvisibles(source: string): [number, number][] {
  return [...source].flatMap((character, at): [number, number][] =>
    isRawInvisible(character) ? [[at, character.codePointAt(0) ?? 0]] : [],
  );
}

const { rows, declaredCount, text } = readTable();

describe("thinking.ts は src/domain/thinking.rs の写しである", () => {
  it("表を見出しの件数どおり読めている", () => {
    // **読み落とすと、下の突き合わせが「少ない行数で全部緑」になる。**
    expect(rows.length).toBe(declaredCount);
    expect(rows.length).toBeGreaterThanOrEqual(MIN_ROWS);
  });

  it("表に生の不可視文字が無い", () => {
    // **`tests/thinking_corpus.rs` と同じ数え方**（TAB・LF・CR・半角空白だけが生で入ってよい）。
    expect(rawInvisibles(text)).toEqual([]);
  });

  it("不可視文字の走査は、植えた文字を数える", () => {
    // **植えた対照を走査の本体に通す**（実ファイルだけだと、走査が何も数えなくても緑になる）。
    // `tests/thinking_corpus.rs` の `the_invisible_scan_catches_planted_characters` と同じ11文字。
    for (const code of [
      0x200b, 0x200c, 0x200d, 0x2060, 0xfeff, 0x00ad, 0x3000, 0x00a0, 0x0085, 0x2028, 0x000b,
    ]) {
      const planted = String.fromCodePoint(code);
      const line = `1${TAB}0${TAB}本文${planted}${TAB}<think>x</think>本文${CR}${LF}`;
      expect(rawInvisibles(line), `U+${code.toString(16)}`).toEqual([[6, code]]);
    }
    // **数えてはいけない側の対照**: 区切りと半角空白と、ふつうの文字・逃げ方の綴りだけの行。
    const clean =
      `# 件数: 1${LF}1${TAB}0${TAB}${BACKSLASH}u{200B}「やあ」 本文${TAB}` +
      `<think>x</think>${BACKSLASH}u{FEFF}本文${CR}${LF}`;
    expect(rawInvisibles(clean)).toEqual([]);
  });

  it("ブロックの数が 0 / 1 / 2 / 3、閉じなかったかが両方そろっている", () => {
    // **「常に 0」「常に閉じた」を返す実装は、その値の行だけでは捕まらない。**
    const seen = [...new Set(rows.map((row) => row.blocks))];
    for (const want of [0, 1, 2, 3]) {
      expect(seen, `ブロックの数が ${want} の行`).toContain(want);
    }
    expect(rows.some((row) => row.unclosed)).toBe(true);
    expect(rows.some((row) => !row.unclosed)).toBe(true);
    expect(rows.some((row) => row.stripped === "")).toBe(true);
  });

  it("全ての行で Rust と同じ本文・同じブロックの数・同じ閉じなかったかになる", () => {
    const mismatches: string[] = [];

    for (const row of rows) {
      const mine = stripThinking(row.source);
      if (
        mine.text !== row.stripped ||
        mine.blocks !== row.blocks ||
        mine.unclosed !== row.unclosed
      ) {
        mismatches.push(
          `${JSON.stringify(row.source)}: ` +
            `表=${JSON.stringify([row.stripped, row.blocks, row.unclosed])} ` +
            `ts=${JSON.stringify([mine.text, mine.blocks, mine.unclosed])}`,
        );
      }
    }

    expect(
      mismatches.length,
      `${mismatches.length} / ${rows.length} 行が表と食い違いました:` +
        LF +
        mismatches.slice(0, 10).join(LF),
    ).toBe(0);
  });
});
