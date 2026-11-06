/**
 * **`emphasis.ts` が `src/parse/extraction.rs` の写しとして成立しているか。**
 *
 * # 権威は Rust 側にある
 *
 * 読んでいる `tests/fixtures/emphasis_corpus.tsv` は**手書きの表**で、
 * **`tests/emphasis_corpus.rs`（Rust）も同じファイルを読む。**
 * つまり赤くなる場所が2つに割れている:
 *
 * | 何が起きた | 赤くなるのは | 直すのは |
 * |---|---|---|
 * | `src/parse/extraction.rs` の規則を変えた | **Rust 側** | 表 → `emphasis.ts` |
 * | `emphasis.ts` が Rust に追随していない | **こちら** | `emphasis.ts` |
 *
 * **⚠ 表を書き換えて緑にしないこと。** 表は人の決定であって、実装の写しではない
 * （`dialogue_corpus.tsv` とはそこが違う。あちらは `parse_response` の出力そのもの）。
 *
 * # ここで見ているもの
 *
 * 1. **表の全行で `stripEmphasis` が Rust と同じ答えを出す**（本文と、対の数）。
 * 2. **`split()` が実際にそれを通している**（`emphasis.ts` を呼ばない実装を緑にしない）。
 * 3. **崩れた回は生出力のまま**（Rust の `fallback` と同じ）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { split } from "../src/lib/dialogue";
import { stripEmphasis } from "../src/lib/emphasis";

/** **`tests/emphasis_corpus.rs` の `fixture_path` と同じ場所。** */
const FIXTURE = fileURLToPath(
  new URL("../../tests/fixtures/emphasis_corpus.tsv", import.meta.url),
);

/** 見出しに書いてある件数（`# 件数: 42`）。**読み落としの検出に使う。** */
const COUNT_PATTERN = /^# 件数: (\d+)$/;

/**
 * 行数の下限。**`tests/emphasis_corpus.rs` の `MIN_ROWS` と同じ値をここにも置く。**
 *
 * **写しに見えるが、そうではない。** 見出しの件数から作ると、
 * **行を削って件数も直す**変え方が両方の側で緑になる。
 */
const MIN_ROWS = 40;

/** 逃げ方に使う文字（U+005C）。 */
const BACKSLASH = String.fromCharCode(0x5c);

/** 逃げ方の接頭辞。**16進4桁が続く。** */
const UNICODE_ESCAPE = BACKSLASH + "u";

/** バックスラッシュ自身の逃げ方。 */
const DOUBLED_BACKSLASH = BACKSLASH + BACKSLASH;

/** 逃げ方の16進の桁数。 */
const ESCAPE_DIGITS = 4;

/** 16進4桁か。 */
const HEX_DIGITS = /^[0-9A-Fa-f]{4}$/;

/** 表の1行。 */
interface Row {
  /** 生出力にあたる入力。 */
  source: string;
  /** 落とした**対**の数。 */
  pairs: number;
  /** 落とした後の本文。 */
  stripped: string;
}

/**
 * 欄を読む。**受ける逃げ方は2つだけ**で、他の綴りでは落とす
 * （黙って読み替えると、表に書いたつもりの文字とは違うものを測ることになる）。
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
      const digits = tail.slice(from, from + ESCAPE_DIGITS);
      if (!HEX_DIGITS.test(digits)) {
        throw new Error(`逃げ方の後ろが16進4桁ではありません: ${JSON.stringify(field)}`);
      }
      out += String.fromCodePoint(Number.parseInt(digits, 16));
      rest = tail.slice(from + ESCAPE_DIGITS);
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

/** 行の区切り。**行末の CR は落とす**（`core.autocrlf=true` のリポジトリ）。 */
const LF = String.fromCharCode(0x0a);
const CR = String.fromCharCode(0x0d);
const TAB = String.fromCharCode(0x09);

/** 表を読む。 */
function readTable(): { rows: Row[]; declaredCount: number } {
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
    if (fields.length !== 3) {
      throw new Error(`表の欄が3つではありません: ${JSON.stringify(line)}`);
    }
    const [source, pairs, stripped] = fields as [string, string, string];
    rows.push({
      source: decode(source),
      pairs: Number.parseInt(pairs, 10),
      stripped: decode(stripped),
    });
  }

  if (declaredCount === null) {
    throw new Error("表の見出しに件数がありません");
  }
  return { rows, declaredCount };
}

const { rows, declaredCount } = readTable();

describe("emphasis.ts は src/parse/extraction.rs の写しである", () => {
  it("表を見出しの件数どおり読めている", () => {
    // **読み落とすと、下の突き合わせが「少ない行数で全部緑」になる。**
    expect(rows.length).toBe(declaredCount);
    expect(rows.length).toBeGreaterThanOrEqual(MIN_ROWS);
  });

  it("対の数が 0 / 1 / 2 / 3 の行がそろっている", () => {
    // **「常に 0」を返す実装は、0 の行だけでは捕まらない。**
    const seen = [...new Set(rows.map((row) => row.pairs))];
    for (const want of [0, 1, 2, 3]) {
      expect(seen, `対の数が ${want} の行`).toContain(want);
    }
  });

  it("全ての行で Rust と同じ本文・同じ対の数になる", () => {
    const mismatches: string[] = [];

    for (const row of rows) {
      const mine = stripEmphasis(row.source);
      if (mine.text !== row.stripped || mine.pairs !== row.pairs) {
        mismatches.push(
          `${JSON.stringify(row.source)}: ` +
            `表=${JSON.stringify([row.stripped, row.pairs])} ` +
            `ts=${JSON.stringify([mine.text, mine.pairs])}`,
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

  it("画面に出る区間からも印が落ちている（split が通していること）", () => {
    // **`stripEmphasis` を直に呼ぶ上の1本は、配線を見ていない。**
    // ここが赤いなら `dialogue.ts` が `emphasis.ts` を通していない。
    const broken: string[] = [];

    for (const row of rows) {
      const result = split(row.source);
      if (result.kind === "fallback") {
        broken.push(`${JSON.stringify(row.source)}: 表の入力が崩れました`);
        continue;
      }
      const rebuilt = result.segments.map((segment) => segment.text).join("");
      if (rebuilt !== row.stripped) {
        broken.push(
          `${JSON.stringify(row.source)} -> ${JSON.stringify(rebuilt)} ` +
            `（表は ${JSON.stringify(row.stripped)}）`,
        );
      }
    }

    expect(
      broken.length,
      `${broken.length} 件が食い違いました:` + LF + broken.slice(0, 10).join(LF),
    ).toBe(0);
  });

  it("崩れた回は生出力のまま（印も落とさない）", () => {
    // **Rust の `fallback` と同じ**（`src/parse/mod.rs` の `fallback`）。
    // 崩れた出力を整形すると、何を見て崩れと判断したかが分からなくなる。
    const raw = "彼女は**口をつぐんだ**。「ヒミツ";
    const result = split(raw);

    expect(result.kind).toBe("fallback");
    if (result.kind === "fallback") {
      expect(result.text).toBe(raw);
      expect(result.reason).toBe("unclosed");
    }

    // 対照: 閉じていれば印は落ちる（この入力が「元から印を持たない」のではない）。
    const closed = split(`${raw}」`);
    expect(closed.kind).toBe("split");
    if (closed.kind === "split") {
      const rebuilt = closed.segments.map((segment) => segment.text).join("");
      expect(rebuilt).toBe("彼女は口をつぐんだ。「ヒミツ」");
    }
  });
});
