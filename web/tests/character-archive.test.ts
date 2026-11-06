/**
 * **しまう・完全に消す**の純粋な部分（作業単位 A-10 / `src/lib/characterArchive.ts`）。
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 一覧の出し分け・識別子の打ち込みの照合（純粋関数） | **ここ** |
 * | どの行を押したら、どの識別子へ要求が飛ぶか | `character-list-archiving.test.ts` |
 */

import { describe, expect, it } from "vitest";

import type { CharacterView } from "../src/lib/api";
import {
  activeCharacters,
  archivedCharacters,
  purgeConfirmed,
} from "../src/lib/characterArchive";

function character(
  id: number,
  handle: string,
  archived_at: string | null,
): CharacterView {
  return {
    id,
    handle,
    display_name: handle,
    base_persona: "ペルソナ。",
    output_format_instruction: null,
    stat_scale: 5,
    stat_drift_mode: null,
    axis_discovery_slots: 0,
    created_at: "2026-10-21T00:00:00Z",
    archived_at,
  };
}

/** しまったもの・しまっていないものを交互に並べる（どちらも端と真ん中に居る）。 */
const LISTED = [
  character(1, "aoi", null),
  character(2, "kaede", "2026-10-21T01:00:00Z"),
  character(3, "mio", null),
  character(4, "nar", "2026-10-21T02:00:00Z"),
  character(5, "rin", null),
];

describe("一覧の出し分け", () => {
  it("しまっていないものだけを、並びを保って返す", () => {
    expect(activeCharacters(LISTED).map((c) => c.handle)).toEqual([
      "aoi",
      "mio",
      "rin",
    ]);
  });

  it("しまったものだけを、並びを保って返す", () => {
    expect(archivedCharacters(LISTED).map((c) => c.handle)).toEqual([
      "kaede",
      "nar",
    ]);
  });

  /**
   * **欄の無い応答は「しまっていない」**（しまう操作を知らない版のサーバ）。
   * `!== null` で比べる実装は、ここで全員を「しまった」側へ落とす。
   */
  it("`archived_at` の欄が無いキャラクターは、しまっていない側に入る", () => {
    const { archived_at: _, ...older } = character(9, "old", null);
    const listed = [older as CharacterView];
    expect(activeCharacters(listed).map((c) => c.handle)).toEqual(["old"]);
    expect(archivedCharacters(listed)).toEqual([]);
  });

  it("2つを合わせると、取りこぼしも重複も無い", () => {
    const both = [...activeCharacters(LISTED), ...archivedCharacters(LISTED)]
      .map((c) => c.id)
      .sort((a, b) => a - b);
    expect(both).toEqual([1, 2, 3, 4, 5]);
  });
});

describe("完全に消す前の確認（識別子を打ち込ませる）", () => {
  it("識別子をそのまま打てば、消してよい", () => {
    expect(purgeConfirmed("mio", "mio")).toBe(true);
  });

  /**
   * **整えてから比べない。** 前後の空白を落とす・大文字小文字を畳むと、
   * 「打ち間違えたのに消える」が作れる（取り消しの効かない操作の確認として意味が無くなる）。
   */
  it.each([
    ["空", ""],
    ["別の識別子", "aoi"],
    ["途中まで", "mi"],
    ["余分な文字", "mio2"],
    ["後ろに空白", "mio "],
    ["前に空白", " mio"],
    ["大文字", "MIO"],
    ["全角", "ｍｉｏ"],
  ])("打ち間違い（%s）なら、消さない", (_, typed) => {
    expect(purgeConfirmed("mio", typed)).toBe(false);
  });
});
