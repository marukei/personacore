/**
 * **訂正版を組み立てる純粋関数**（作業単位 D-3 / `src/lib/memoryRepair.ts`）。
 *
 * # `memory-list.test.ts` との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **何を送るか**（キーの有無・拒否の理由） | **ここ** |
 * | 一覧のどの行を選んだら、どの記憶へ送られるか | `memory-list.test.ts` |
 *
 * **I/O を持たない**ので jsdom も要らない（既定の `node` で走る）。
 */

import { describe, expect, it } from "vitest";

import type { MemoryView } from "../src/lib/api";
import {
  buildCorrection,
  draftForMemory,
  draftForUnreadable,
  isUnchanged,
  subjectLabel,
  type CorrectionDraft,
} from "../src/lib/memoryRepair";

/**
 * 3件の記憶。**主語も本文も分類も、3件で全部違う。**
 *
 * **`subject` を全件 `user` にしない**（CLAUDE.md の最頻の穴）——
 * 揃えると「主語を決め打つ」変異が観測できない。
 */
const USER_MEMORY: MemoryView = {
  id: 11,
  subject: "user",
  content: "黒猫を飼っている",
  category: "好み",
  created_at: "2026-08-27T00:00:00Z",
};

/** **真ん中の行。** 端に置くと「常に先頭／常に末尾」の片方が通り抜ける。 */
const CHARACTER_MEMORY: MemoryView = {
  id: 22,
  subject: "character",
  content: "喫茶店でアルバイトをしている",
  category: "経歴",
  created_at: "2026-08-27T00:10:00Z",
};

/** **分類が未設定の行**（`null` と空文字の扱いを分けるために要る）。 */
const UNCATEGORIZED_MEMORY: MemoryView = {
  id: 33,
  subject: "user",
  content: "朝は紅茶を飲む",
  category: null,
  created_at: "2026-08-27T00:20:00Z",
};

it("フィクスチャの値は互いに違う（同値だと取り違えが観測できない）", () => {
  expect(USER_MEMORY.subject).not.toBe(CHARACTER_MEMORY.subject);
  expect(USER_MEMORY.content).not.toBe(CHARACTER_MEMORY.content);
  expect(USER_MEMORY.category).not.toBe(CHARACTER_MEMORY.category);
  expect(USER_MEMORY.id).not.toBe(CHARACTER_MEMORY.id);
  expect(CHARACTER_MEMORY.category).not.toBe(UNCATEGORIZED_MEMORY.category);
});

describe("主語の見出し", () => {
  it("主語ごとに違う綴りになる", () => {
    expect(subjectLabel("user")).toBe("利用者");
    expect(subjectLabel("character")).toBe("キャラクター");
    expect(subjectLabel("user")).not.toBe(subjectLabel("character"));
  });
});

describe("下書きの初期値", () => {
  it("読めた行は、その行の主語・本文・分類から始まる", () => {
    const draft = draftForMemory(CHARACTER_MEMORY);
    expect(draft.subject).toBe(CHARACTER_MEMORY.subject);
    expect(draft.content).toBe(CHARACTER_MEMORY.content);
    expect(draft.category).toBe(CHARACTER_MEMORY.category);
    // **対照: 他の行の値ではない。**
    expect(draft.subject).not.toBe(USER_MEMORY.subject);
    expect(draft.content).not.toBe(USER_MEMORY.content);
    expect(draft.content).not.toBe(UNCATEGORIZED_MEMORY.content);
  });

  it("分類が未設定の行は、空欄から始まる（`null` を画面へ出さない）", () => {
    expect(draftForMemory(UNCATEGORIZED_MEMORY).category).toBe("");
  });

  /**
   * **壊れた行の主語は推測しない。**
   *
   * 既定を入れると、**押し間違いで主語が入れ替わった記憶**が生まれる
   * ——一人称の混線そのもの（絶対制約）。選ばせる。
   */
  it("読めなかった行は、主語が決まっていない", () => {
    const draft = draftForUnreadable();
    expect(draft.subject).toBeNull();
    expect(draft.content).toBe("");
    expect(draft.category).toBe("");
  });
});

describe("送る本文", () => {
  it("主語は下書きのもの（元の行のものではない）", () => {
    const draft: CorrectionDraft = {
      ...draftForMemory(CHARACTER_MEMORY),
      subject: "user",
    };
    const built = buildCorrection(draft);
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect(built.body.subject).toBe("user");
    expect(built.body.subject).not.toBe(CHARACTER_MEMORY.subject);
    expect(built.body.content).toBe(CHARACTER_MEMORY.content);
  });

  it("分類が空ならキーごと落ちる（`deny_unknown_fields` の側ではなく、未設定の表し方）", () => {
    const built = buildCorrection({
      subject: "user",
      content: "黒猫を飼っている",
      category: "   ",
    });
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect("category" in built.body).toBe(false);
    expect(Object.keys(built.body).sort()).toEqual(["content", "subject"]);
  });

  it("分類があれば載る", () => {
    const built = buildCorrection(draftForMemory(CHARACTER_MEMORY));
    expect(built.ok).toBe(true);
    if (!built.ok) {
      return;
    }
    expect(built.body.category).toBe(CHARACTER_MEMORY.category);
  });

  it("主語を選んでいなければ送らない（理由は主語を名乗る）", () => {
    const built = buildCorrection({
      subject: null,
      content: "黒猫を飼っている",
      category: "",
    });
    expect(built.ok).toBe(false);
    if (built.ok) {
      return;
    }
    expect(built.message).toContain("主語");
  });

  it("本文が空白だけなら送らない（後継の無い取り消しを作らない / ADR-0022 §1）", () => {
    const built = buildCorrection({
      subject: "user",
      content: "  \n ",
      category: "",
    });
    expect(built.ok).toBe(false);
    if (built.ok) {
      return;
    }
    expect(built.message).toContain("本文");
  });
});

describe("元と同じ訂正は送らない", () => {
  it("主語も本文も分類も同じなら、送るものが無い", () => {
    expect(isUnchanged(CHARACTER_MEMORY, draftForMemory(CHARACTER_MEMORY))).toBe(
      true,
    );
    expect(
      isUnchanged(UNCATEGORIZED_MEMORY, draftForMemory(UNCATEGORIZED_MEMORY)),
    ).toBe(true);
  });

  /**
   * **⚠ ここを「本文が同じなら送らない」にしないこと。**
   *
   * 実測で最も多い誤りは**主語の取り違え**（ADR-0021 §10 / 約 6%）で、
   * その訂正は**本文が1文字も変わらない。** 本文だけで判定すると、
   * **いちばん直したい誤りだけが直せなくなる。**
   */
  it("主語だけを直す訂正は送れる", () => {
    const draft: CorrectionDraft = {
      ...draftForMemory(CHARACTER_MEMORY),
      subject: "user",
    };
    expect(draft.content).toBe(CHARACTER_MEMORY.content);
    expect(isUnchanged(CHARACTER_MEMORY, draft)).toBe(false);
  });

  it("本文だけ・分類だけを直す訂正も送れる", () => {
    expect(
      isUnchanged(CHARACTER_MEMORY, {
        ...draftForMemory(CHARACTER_MEMORY),
        content: "喫茶店の店主である",
      }),
    ).toBe(false);
    expect(
      isUnchanged(CHARACTER_MEMORY, {
        ...draftForMemory(CHARACTER_MEMORY),
        category: "",
      }),
    ).toBe(false);
    expect(
      isUnchanged(UNCATEGORIZED_MEMORY, {
        ...draftForMemory(UNCATEGORIZED_MEMORY),
        category: "好み",
      }),
    ).toBe(false);
  });
});
