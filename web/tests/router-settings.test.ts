// @vitest-environment jsdom
/**
 * **「設定」の画面への経路**（作業単位 F-12b / ADR-0057 §決定5 / `src/lib/router.ts`）。
 *
 * # なぜ識別子を運ばないのか
 *
 * **待ち受けのポートは personacore 自身の設定**で、どのキャラクターを開いていても同じ。
 * だから上流の画面と同じ側 ——**ナビに並ぶ。**
 *
 * # ⚠ 「識別子を運ぶ画面」の側へ落ちていないこと
 *
 * `#/settings/aoi` を受けると、画面がキャラクターを名乗ったまま中身は全体の設定、
 * という食い違いが作れる。**段が増えた綴りは知らない綴りにする。**
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

/** 識別子（**運ばれてはいけない**ことの対照に使う）。 */
const AOI = "aoi";

afterEach(() => {
  window.location.hash = "";
});

describe("ハッシュを画面に読む", () => {
  it("`#/settings` は設定の画面", () => {
    expect(screenOf("#/settings")).toEqual({ route: "settings" });
    expect(screenOf("#/settings/")).toEqual({ route: "settings" });
    expect(screenOf("settings")).toEqual({ route: "settings" });
  });

  it("識別子を運ばない（段が増えた綴りは知らない綴り）", () => {
    expect(screenOf(`#/settings/${AOI}`)).toBeNull();
  });

  it("既存の画面と混ざらない", () => {
    expect(screenOf("#/settings")).not.toEqual(screenOf("#/upstream"));
    expect(screenOf("#/settings")).not.toEqual(screenOf("#/chat"));
    // **前方一致で拾わない。**
    expect(screenOf("#/setting")).toBeNull();
    expect(screenOf("#/settingss")).toBeNull();
  });
});

describe("リンクと見出し", () => {
  it("リンク先は `#/settings`", () => {
    expect(hrefFor({ route: "settings" })).toBe("#/settings");
  });

  it("見出しは「設定」で、他のどの画面とも違う綴り", () => {
    expect(labelFor({ route: "settings" })).toBe("設定");
    for (const other of navScreens().filter((screen) => screen.route !== "settings")) {
      expect(labelFor({ route: "settings" })).not.toBe(labelFor(other));
    }
  });
});

describe("ナビゲーション", () => {
  /**
   * **ナビの最後に並ぶ**（ADR-0057 §決定5「ナビに足す」）。
   * 並びごと固定する —— 識別子を運ぶ画面が紛れ込んでも赤くなる。
   */
  it("`navScreens` の最後に設定の画面がある", () => {
    const routes = navScreens().map((screen: Screen) => screen.route);
    expect(routes).toEqual(["chat", "characters", "upstream", "settings"]);
  });
});

describe("いまの画面", () => {
  it("`#/settings` を見ているときは、その画面", () => {
    window.location.hash = "#/settings";
    expect(currentScreen()).toEqual({ route: "settings" });
    expect(currentScreen()).not.toEqual(DEFAULT_SCREEN);
  });

  it("購読の開始時に、設定の画面がそのまま届く", () => {
    window.location.hash = "#/settings";
    const seen: Screen[] = [];
    const stop = startRouting((screen) => seen.push(screen));
    expect(seen).toEqual([{ route: "settings" }]);
    stop();
  });
});
