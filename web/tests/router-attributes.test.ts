// @vitest-environment jsdom
/**
 * **属性の画面への経路**（作業単位 D-12a / `src/lib/router.ts`）。
 *
 * # なぜ経路が識別子を運ぶのか
 *
 * 属性はキャラクターごと（`GET /admin/characters/{handle}/attributes`）なので、
 * **`#/attributes` だけでは行き先が決まらない。**
 * 名前と識別子を別々に持たせると、**画面が A を名乗ったまま中身が B**
 * という食い違いが作れる（`router.ts` の `Screen` の doc）。
 *
 * # ⚠ **同じ形の経路が3つになった**
 *
 * `#/memories/{識別子}` / `#/stats/{識別子}` / `#/attributes/{識別子}` は
 * **どれも `#/{名前}/{識別子}`** である。
 * **どの綴りでも同じ画面になる**変異は、1つだけを見ていると通り抜けるので、
 * **3つを1本の中で突き合わせる。**
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
  it("`#/attributes/{識別子}` は、その識別子の属性の画面", () => {
    const screen = screenOf(`#/attributes/${MIO}`);
    expect(screen).toEqual({ route: "attributes", handle: MIO });
    // **対照: 別の識別子にならない**（決め打ちを落とす）。
    expect(screen).not.toEqual({ route: "attributes", handle: AOI });
    expect(screen).not.toEqual({ route: "attributes", handle: NAR });
  });

  it("記憶とも性格値とも混ざらない（同じ形の経路が3つある）", () => {
    expect(screenOf(`#/attributes/${MIO}`)).toEqual({
      route: "attributes",
      handle: MIO,
    });
    expect(screenOf(`#/stats/${MIO}`)).toEqual({ route: "stats", handle: MIO });
    expect(screenOf(`#/memories/${MIO}`)).toEqual({
      route: "memories",
      handle: MIO,
    });
    // **同じ識別子でも、画面は別物。**
    expect(screenOf(`#/attributes/${MIO}`)).not.toEqual(
      screenOf(`#/stats/${MIO}`),
    );
    expect(screenOf(`#/attributes/${MIO}`)).not.toEqual(
      screenOf(`#/memories/${MIO}`),
    );
  });

  it("識別子の無い `#/attributes` は知らない綴り（空の識別子を作らない）", () => {
    // 空の識別子を作ると `/admin/characters//attributes` を引きに行く画面ができる。
    expect(screenOf("#/attributes")).toBeNull();
    expect(screenOf("#/attributes/")).toBeNull();
  });

  /**
   * **段が多すぎる綴りも知らない綴り。**
   *
   * **⚠ `revisions` は API のパスであって画面ではない** ——
   * `#/attributes/{識別子}/revisions` を受けると、
   * **画面が2つある**ように見えて、どちらが権威か分からなくなる。
   * 変更の記録は**同じ画面の中の節**である。
   */
  it("段が多すぎる綴りも知らない綴り", () => {
    expect(screenOf(`#/attributes/${MIO}/revisions`)).toBeNull();
  });

  it("壊れたパーセント符号化でも例外を投げず、知らない綴りにする", () => {
    expect(screenOf("#/attributes/%E0%A4%A")).toBeNull();
  });

  it("既存の画面は今までどおり", () => {
    expect(screenOf("#/chat")).toEqual({ route: "chat" });
    expect(screenOf("#/characters/")).toEqual({ route: "characters" });
    expect(screenOf("#/upstream")).toEqual({ route: "upstream" });
    expect(screenOf("#/attributess/mio")).toBeNull();
  });
});

describe("画面へのリンク", () => {
  it("識別子を符号化して往復する", () => {
    // **`/` を含む識別子**（サーバは許さないが、呼び出し側の性質に寄りかからない）。
    for (const handle of ["a/b", "あ お い", "100%", MIO]) {
      const href = hrefFor({ route: "attributes", handle });
      expect(href.startsWith("#/attributes/")).toBe(true);
      expect(screenOf(href)).toEqual({ route: "attributes", handle });
    }
  });

  it("識別子が違えばリンク先も違う", () => {
    expect(hrefFor({ route: "attributes", handle: MIO })).not.toBe(
      hrefFor({ route: "attributes", handle: NAR }),
    );
  });

  it("記憶と性格値のリンクと同じ綴りにならない", () => {
    const href = hrefFor({ route: "attributes", handle: MIO });
    expect(href).not.toBe(hrefFor({ route: "memories", handle: MIO }));
    expect(href).not.toBe(hrefFor({ route: "stats", handle: MIO }));
  });

  it("見出しは、その識別子を名乗る", () => {
    const label = labelFor({ route: "attributes", handle: MIO });
    expect(label).toContain(MIO);
    expect(label).not.toContain(NAR);
    // **他の2つの見出しと別物**（同じ文言だと、どちらの画面か読み手に分からない）。
    expect(label).not.toBe(labelFor({ route: "memories", handle: MIO }));
    expect(label).not.toBe(labelFor({ route: "stats", handle: MIO }));
  });
});

describe("ナビゲーションには並ばない", () => {
  /**
   * 並べるには識別子を1つ決め打つことになり、
   * **どのキャラクターの属性かをナビが勝手に決める**ことになる
   * （記憶・性格値と同じ理由 / `router.ts` の `NAV`）。
   */
  it("`navScreens` に属性の画面が無い", () => {
    // **並びごと固定したままにしてある** —— `attributes` が紛れ込めば赤くなる。
    expect(navScreens().map((screen) => screen.route)).toEqual([
      "chat",
      "characters",
      "upstream",
    ]);
  });
});

describe("いまの画面", () => {
  it("`#/attributes/{識別子}` を見ているときは、その画面", () => {
    window.location.hash = `#/attributes/${NAR}`;
    expect(currentScreen()).toEqual({ route: "attributes", handle: NAR });
  });
});
