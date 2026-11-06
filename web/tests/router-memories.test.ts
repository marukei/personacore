// @vitest-environment jsdom
/**
 * **記憶の画面への経路**（作業単位 D-3 / `src/lib/router.ts`）。
 *
 * # なぜ経路が「名前だけ」では足りないのか
 *
 * **記憶はキャラクターごと**（`GET /admin/characters/{handle}/memories`）なので、
 * `#/memories` だけでは**どのキャラクターの記憶かが決まらない。**
 * 経路が識別子を運ぶ必要があり、**運び方を間違えると
 * 「別のキャラクターの記憶を、正しい見出しの下に出す」**という壊れ方になる
 * （CLAUDE.md の「抜けるのはいつも帰属」）。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { afterEach, describe, expect, it } from "vitest";

import {
  DEFAULT_SCREEN,
  currentScreen,
  hrefFor,
  labelFor,
  navScreens,
  screenOf,
  startRouting,
  type Screen,
} from "../src/lib/router";

/** **3つの識別子は全部違う値**（取り違えが観測できるように）。 */
const AOI = "aoi";
const MIO = "mio";
const NAR = "nar";

afterEach(() => {
  window.location.hash = "";
});

describe("ハッシュを画面に読む", () => {
  it("`#/memories/{識別子}` は、その識別子の記憶の画面", () => {
    const screen = screenOf(`#/memories/${MIO}`);
    expect(screen).toEqual({ route: "memories", handle: MIO });
    // **対照: 別の識別子にならない**（決め打ちを落とす）。
    expect(screen).not.toEqual({ route: "memories", handle: AOI });
    expect(screen).not.toEqual({ route: "memories", handle: NAR });
  });

  it("識別子の無い `#/memories` は知らない綴り（空の識別子を作らない）", () => {
    expect(screenOf("#/memories")).toBeNull();
    expect(screenOf("#/memories/")).toBeNull();
  });

  it("段が多すぎる綴りも知らない綴り", () => {
    expect(screenOf(`#/memories/${MIO}/corrections`)).toBeNull();
  });

  it("既存の2画面は今までどおり", () => {
    expect(screenOf("#/chat")).toEqual({ route: "chat" });
    expect(screenOf("#chat")).toEqual({ route: "chat" });
    expect(screenOf("#/characters/")).toEqual({ route: "characters" });
    expect(screenOf("#/chatt")).toBeNull();
    expect(screenOf("")).toBeNull();
  });

  it("壊れたパーセント符号化でも例外を投げず、知らない綴りにする", () => {
    expect(screenOf("#/memories/%E0%A4%A")).toBeNull();
  });
});

describe("画面へのリンク", () => {
  it("識別子を符号化して往復する", () => {
    // **`/` を含む識別子**（サーバは許さないが、ここは呼び出し側の性質に
    // 寄りかからない —— `api.ts` の `encodeURIComponent` と同じ立場）。
    for (const handle of ["a/b", "あ お い", "100%", MIO]) {
      const href = hrefFor({ route: "memories", handle });
      expect(href.startsWith("#/memories/")).toBe(true);
      expect(screenOf(href)).toEqual({ route: "memories", handle });
    }
  });

  it("識別子が違えばリンク先も違う", () => {
    expect(hrefFor({ route: "memories", handle: MIO })).not.toBe(
      hrefFor({ route: "memories", handle: AOI }),
    );
  });

  it("見出しは、その識別子を名乗る", () => {
    const label = labelFor({ route: "memories", handle: MIO });
    expect(label).toContain(MIO);
    expect(label).not.toContain(AOI);
    // 対照: 既存の2画面の見出しは変わっていない。
    expect(labelFor({ route: "chat" })).toBe("会話");
    expect(labelFor({ route: "characters" })).toBe("キャラクター");
  });
});

describe("ナビゲーション", () => {
  /**
   * **記憶はナビに並べない。**
   *
   * 並べるには識別子を1つ決め打つことになり、
   * **「どのキャラクターの記憶か」をナビが勝手に決める**ことになる。
   */
  it("記憶の画面はナビに並ばない", () => {
    const routes = navScreens().map((screen: Screen) => screen.route);
    expect(routes).toContain("chat");
    expect(routes).toContain("characters");
    expect(routes).not.toContain("memories");
  });
});

describe("いまの画面", () => {
  it("記憶のハッシュを読む", () => {
    window.location.hash = `#/memories/${NAR}`;
    expect(currentScreen()).toEqual({ route: "memories", handle: NAR });
  });

  it("知らない綴りは既定へ", () => {
    window.location.hash = "#/memories";
    expect(currentScreen()).toEqual(DEFAULT_SCREEN);
  });

  it("購読の開始時に、記憶の画面がそのまま届く", () => {
    window.location.hash = `#/memories/${NAR}`;
    const seen: Screen[] = [];
    const stop = startRouting((screen) => seen.push(screen));
    expect(seen).toEqual([{ route: "memories", handle: NAR }]);
    stop();
  });

  it("知らない綴りでは URL の側を既定へ書き換える", () => {
    window.location.hash = "#/memories";
    const seen: Screen[] = [];
    const stop = startRouting((screen) => seen.push(screen));
    // **その回は呼ばない**（書き換えが `hashchange` を起こす）。
    expect(seen).toEqual([]);
    expect(window.location.hash).toBe(hrefFor(DEFAULT_SCREEN));
    stop();
  });
});
