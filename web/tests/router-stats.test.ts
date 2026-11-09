// @vitest-environment jsdom
/**
 * **性格値と変容の画面への経路**（作業単位 D-4 / `src/lib/router.ts`）。
 *
 * # なぜ経路が識別子を運ぶのか
 *
 * 性格値も変容もキャラクターごと（`GET /admin/characters/{handle}/stats` /
 * `.../drift`）なので、**`#/stats` だけでは行き先が決まらない。**
 * 名前と識別子を別々に持たせると、**画面が A を名乗ったまま中身が B**
 * という食い違いが作れる（`router.ts` の `Screen` の doc）。
 *
 * # ⚠ **記憶の画面と同じ形なので、取り違えが起きうる**
 *
 * `#/memories/{識別子}` と `#/stats/{識別子}` は綴りが1語しか違わない。
 * **どちらの綴りでも同じ画面になる**変異は、片方だけを見ていると通り抜けるので、
 * **両方を1本の中で突き合わせる。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { afterEach, describe, expect, it } from "vitest";

import {
  currentScreen,
  hrefFor,
  labelFor,
  navScreens,
  screenOf,
} from "../src/lib/router";

/** **3つの識別子は全部違う値**（取り違えが観測できるように）。 */
const AOI = "aoi";
const MIO = "mio";
const NAR = "nar";

afterEach(() => {
  window.location.hash = "";
});

describe("ハッシュを画面に読む", () => {
  it("`#/stats/{識別子}` は、その識別子の性格値の画面", () => {
    const screen = screenOf(`#/stats/${MIO}`);
    expect(screen).toEqual({ route: "stats", handle: MIO });
    // **対照: 別の識別子にならない**（決め打ちを落とす）。
    expect(screen).not.toEqual({ route: "stats", handle: AOI });
    expect(screen).not.toEqual({ route: "stats", handle: NAR });
  });

  it("記憶の画面と混ざらない（同じ形の経路が2つある）", () => {
    expect(screenOf(`#/stats/${MIO}`)).toEqual({ route: "stats", handle: MIO });
    expect(screenOf(`#/memories/${MIO}`)).toEqual({
      route: "memories",
      handle: MIO,
    });
    // **同じ識別子でも、画面は別物。**
    expect(screenOf(`#/stats/${MIO}`)).not.toEqual(
      screenOf(`#/memories/${MIO}`),
    );
  });

  it("識別子の無い `#/stats` は知らない綴り（空の識別子を作らない）", () => {
    // 空の識別子を作ると `/admin/characters//stats` を引きに行く画面ができる。
    expect(screenOf("#/stats")).toBeNull();
    expect(screenOf("#/stats/")).toBeNull();
  });

  it("段が多すぎる綴りも知らない綴り", () => {
    expect(screenOf(`#/stats/${MIO}/drift`)).toBeNull();
  });

  it("壊れたパーセント符号化でも例外を投げず、知らない綴りにする", () => {
    expect(screenOf("#/stats/%E0%A4%A")).toBeNull();
  });

  it("既存の画面は今までどおり", () => {
    expect(screenOf("#/chat")).toEqual({ route: "chat" });
    expect(screenOf("#/characters/")).toEqual({ route: "characters" });
    expect(screenOf("#/statss/mio")).toBeNull();
  });
});

describe("画面へのリンク", () => {
  it("識別子を符号化して往復する", () => {
    // **`/` を含む識別子**（サーバは許さないが、呼び出し側の性質に寄りかからない）。
    for (const handle of ["a/b", "あ お い", "100%", MIO]) {
      const href = hrefFor({ route: "stats", handle });
      expect(href.startsWith("#/stats/")).toBe(true);
      expect(screenOf(href)).toEqual({ route: "stats", handle });
    }
  });

  it("識別子が違えばリンク先も違う", () => {
    expect(hrefFor({ route: "stats", handle: MIO })).not.toBe(
      hrefFor({ route: "stats", handle: NAR }),
    );
  });

  it("記憶のリンクと同じ綴りにならない", () => {
    expect(hrefFor({ route: "stats", handle: MIO })).not.toBe(
      hrefFor({ route: "memories", handle: MIO }),
    );
  });

  it("見出しは、その識別子を名乗る", () => {
    const label = labelFor({ route: "stats", handle: MIO });
    expect(label).toContain(MIO);
    expect(label).not.toContain(NAR);
    // **記憶の見出しと別物**（同じ文言だと、どちらの画面か読み手に分からない）。
    expect(label).not.toBe(labelFor({ route: "memories", handle: MIO }));
  });
});

describe("ナビゲーションには並ばない", () => {
  /**
   * 並べるには識別子を1つ決め打つことになり、
   * **どのキャラクターの性格値かをナビが勝手に決める**ことになる
   * （記憶の画面と同じ理由 / `router.ts` の `NAV`）。
   */
  it("`navScreens` に性格値の画面が無い", () => {
    // **⚠ `upstream` が増えたのは、この検査が緩んだからではない**（作業単位 D-9）。
    // **上流の画面は識別子を1つも決め打たない**ので、
    // ここが避けている「どのキャラクターのものかをナビが勝手に決める」形にならない。
    // **並びごと固定したままにしてある** —— `stats` が紛れ込めば、いまも赤くなる。
    expect(navScreens().map((screen) => screen.route)).toEqual([
      "chat",
      "characters",
      "upstream",
      // **識別子を運ばない「設定」**（作業単位 F-12b）。上の `upstream` と同じ理由で並ぶ。
      "settings",
    ]);
  });
});

describe("いまの画面", () => {
  it("`#/stats/{識別子}` を見ているときは、その画面", () => {
    window.location.hash = `#/stats/${NAR}`;
    expect(currentScreen()).toEqual({ route: "stats", handle: NAR });
  });
});
