/**
 * **約束・決まりの画面への経路**（作業単位 A-12a / `src/lib/router.ts`）。
 *
 * `#/memories` / `#/stats` / `#/attributes` / `#/agreements` は**どれも `#/{名前}/{識別子}`**。
 * どの綴りでも同じ画面になる変異は1つだけを見ていると通り抜けるので、4つを突き合わせる。
 *
 * **jsdom は要らない**（`screenOf` / `hrefFor` / `labelFor` は純粋）。
 */

import { describe, expect, it } from "vitest";

import { hrefFor, labelFor, navScreens, screenOf } from "../src/lib/router";

const AOI = "aoi";
const MIO = "mio";

describe("約束・決まりの経路", () => {
  it("`#/agreements/{識別子}` は、その識別子の約束・決まりの画面", () => {
    expect(screenOf(`#/agreements/${MIO}`)).toEqual({ route: "agreements", handle: MIO });
    expect(screenOf(`#/agreements/${MIO}`)).not.toEqual({ route: "agreements", handle: AOI });
  });

  it("他の3つの画面と混ざらない", () => {
    const screens = ["memories", "stats", "attributes", "agreements"].map((name) =>
      screenOf(`#/${name}/${MIO}`),
    );
    expect(new Set(screens.map((screen) => JSON.stringify(screen))).size).toBe(4);
  });

  it("識別子の無い・段の多い綴りは知らない綴り", () => {
    expect(screenOf("#/agreements")).toBeNull();
    expect(screenOf("#/agreements/")).toBeNull();
    // **記録は画面ではない**（同じ画面の中の節）。
    expect(screenOf(`#/agreements/${MIO}/transitions`)).toBeNull();
  });

  it("リンク先は符号化され、往復で同じ画面に戻る", () => {
    const hostile = "../x y%z";
    const href = hrefFor({ route: "agreements", handle: hostile });
    expect(href).toBe("#/agreements/..%2Fx%20y%25z");
    expect(screenOf(href)).toEqual({ route: "agreements", handle: hostile });
  });

  it("見出しは他の画面と別の文言で、識別子を名乗る", () => {
    expect(labelFor({ route: "agreements", handle: MIO })).toBe("mio の約束・決まり");
    expect(labelFor({ route: "agreements", handle: MIO })).not.toBe(
      labelFor({ route: "attributes", handle: MIO }),
    );
  });

  it("ナビゲーションには並ばない（識別子を決め打たない）", () => {
    expect(navScreens().map((screen) => screen.route)).not.toContain("agreements");
  });
});
