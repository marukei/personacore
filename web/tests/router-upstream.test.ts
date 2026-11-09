// @vitest-environment jsdom
/**
 * **上流を見る画面への経路**（作業単位 D-9 / `src/lib/router.ts`）。
 *
 * # なぜ識別子を運ばないのか
 *
 * **上流はキャラクターに紐づかない。** 面（本文 / 裏方）は2つとも
 * personacore の設定で決まっていて、**どのキャラクターを開いていても同じ**である。
 * だから `#/chat` / `#/characters` と同じ側 ——**ナビに並ぶ。**
 *
 * # ⚠ 「識別子を運ぶ画面」の側へ落ちていないこと
 *
 * `#/upstream/aoi` のような綴りを受けてしまうと、
 * **画面がキャラクターを名乗ったまま、中身は全体の設定**という食い違いが作れる。
 * **段が増えた綴りは知らない綴りにする。**
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
  it("`#/upstream` は上流の画面", () => {
    expect(screenOf("#/upstream")).toEqual({ route: "upstream" });
    expect(screenOf("#/upstream/")).toEqual({ route: "upstream" });
    expect(screenOf("upstream")).toEqual({ route: "upstream" });
  });

  it("識別子を運ばない（段が増えた綴りは知らない綴り）", () => {
    expect(screenOf(`#/upstream/${AOI}`)).toBeNull();
  });

  it("既存の画面と混ざらない", () => {
    expect(screenOf("#/upstream")).not.toEqual(screenOf("#/chat"));
    expect(screenOf("#/upstream")).not.toEqual(screenOf("#/characters"));
    // **前方一致で拾わない。**
    expect(screenOf("#/upstreams")).toBeNull();
  });
});

describe("リンクと見出し", () => {
  it("リンク先は `#/upstream`", () => {
    expect(hrefFor({ route: "upstream" })).toBe("#/upstream");
  });

  it("見出しは、他のどの画面とも違う綴り", () => {
    expect(labelFor({ route: "upstream" })).toBe("上流モデル");
    expect(labelFor({ route: "upstream" })).not.toBe(
      labelFor({ route: "chat" }),
    );
    expect(labelFor({ route: "upstream" })).not.toBe(
      labelFor({ route: "characters" }),
    );
  });
});

describe("ナビゲーション", () => {
  /**
   * **ナビに並ぶ。** 識別子が要らない画面なので、
   * **どのキャラクターのものかをナビが勝手に決める**問題が起きない
   * （記憶と性格値がナビに並ばない理由の、ちょうど裏側）。
   */
  it("`navScreens` に上流の画面がある", () => {
    const routes = navScreens().map((screen: Screen) => screen.route);
    // **作業単位 F-12b で「設定」が最後に並んだ**（`tests/router-settings.test.ts`）。
    expect(routes).toEqual(["chat", "characters", "upstream", "settings"]);
  });
});

describe("いまの画面", () => {
  it("`#/upstream` を見ているときは、その画面", () => {
    window.location.hash = "#/upstream";
    expect(currentScreen()).toEqual({ route: "upstream" });
    expect(currentScreen()).not.toEqual(DEFAULT_SCREEN);
  });

  it("購読の開始時に、上流の画面がそのまま届く", () => {
    window.location.hash = "#/upstream";
    const seen: Screen[] = [];
    const stop = startRouting((screen) => seen.push(screen));
    expect(seen).toEqual([{ route: "upstream" }]);
    stop();
  });
});
