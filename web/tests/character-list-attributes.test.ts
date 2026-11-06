// @vitest-environment jsdom
/**
 * **一覧から属性の画面へ辿る導線**（作業単位 D-12a /
 * `src/lib/CharacterList.svelte`）。
 *
 * # なぜ別ファイルなのか
 *
 * `character-list-memories.test.ts` / `character-list-stats.test.ts` と同じ形で、
 * **ここは属性への行き先だけ**を見る ——**その行のキャラクターへ行くか**（帰属）。
 *
 * # ⚠ **端の行を押さない**
 *
 * 選ぶ操作のテストは、**選ばれる対象が端にあると「常に端」の変異を通す**
 * （D-2 のレビューで実測）。**3件にして真ん中を見る。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import CharacterList from "../src/lib/CharacterList.svelte";
import type { CharacterView } from "../src/lib/api";
import { hrefFor } from "../src/lib/router";

const AOI: CharacterView = {
  id: 1,
  handle: "aoi",
  display_name: "藍生",
  base_persona: "静かな声で話す。",
  output_format_instruction: null,
  stat_scale: 5,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:00:00Z",
};

/** **真ん中の行**（上の ⚠）。 */
const MIO: CharacterView = {
  id: 2,
  handle: "mio",
  display_name: "澪",
  base_persona: "早口で話す。",
  output_format_instruction: "台詞は「」で囲む。",
  stat_scale: 7,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:10:00Z",
};

const NAR: CharacterView = {
  id: 3,
  handle: "nar",
  display_name: "奈落",
  base_persona: "ゆっくり話す。",
  output_format_instruction: null,
  stat_scale: 3,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:20:00Z",
};

/** 一覧の行から辿る先の見出し。**綴りは1箇所で決める。** */
const ATTRIBUTES_LINK = "属性";

let target: HTMLElement;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const respond = (body: unknown): Promise<Response> =>
      Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(JSON.stringify(body)),
      } as unknown as Response);

    if (url === "/admin/characters" && (init?.method ?? "GET") === "GET") {
      return respond({ characters: [AOI, MIO, NAR], unreadable: [] });
    }
    return respond({ character: "?", stat_scale: 5, stats: [] });
  });
}

/** その行の行。 */
function rowOf(character: CharacterView): HTMLElement {
  const found = [...target.querySelectorAll("tbody tr")].find(
    (tr) => tr.querySelector("code")?.textContent?.trim() === character.handle,
  );
  expect(found, `${character.handle} の行があること`).toBeDefined();
  return found as HTMLElement;
}

/** その行の「属性」リンク。**行から引く**（全体から引くと1件目に当たり続ける）。 */
function attributesLinkOf(character: CharacterView): HTMLAnchorElement {
  const found = [...rowOf(character).querySelectorAll("a")].find(
    (anchor) => anchor.textContent?.trim() === ATTRIBUTES_LINK,
  );
  expect(
    found,
    `${character.handle} の行に「${ATTRIBUTES_LINK}」があること`,
  ).toBeDefined();
  return found as HTMLAnchorElement;
}

beforeEach(() => {
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

it("行の「属性」は、その行のキャラクターを指す", async () => {
  const view = mount(CharacterList, { target });
  await vi.waitFor(() => {
    expect(target.querySelectorAll("tbody tr").length).toBeGreaterThanOrEqual(3);
  });
  flushSync();

  const link = attributesLinkOf(MIO);
  expect(link.getAttribute("href")).toBe(
    hrefFor({ route: "attributes", handle: MIO.handle }),
  );
  expect(link.getAttribute("href")).toContain(MIO.handle);
  // **対照: 前後の行のどちらへも向いていない**（「常に先頭」「常に最終」を両方落とす）。
  expect(link.getAttribute("href")).not.toContain(AOI.handle);
  expect(link.getAttribute("href")).not.toContain(NAR.handle);
  // 端の行も、それぞれ自分を指す。
  expect(attributesLinkOf(AOI).getAttribute("href")).toContain(AOI.handle);
  expect(attributesLinkOf(NAR).getAttribute("href")).toContain(NAR.handle);

  await unmount(view, { outro: false });
});

/**
 * **記憶・性格値のリンクと行き先が入れ替わっていない。**
 *
 * 同じ行に3つ並ぶので、**綴りが1語しか違わない経路の取り違え**が起きうる。
 */
it("記憶と性格値のリンクと行き先が入れ替わっていない", async () => {
  const view = mount(CharacterList, { target });
  await vi.waitFor(() => {
    expect(target.querySelectorAll("tbody tr").length).toBeGreaterThanOrEqual(3);
  });
  flushSync();

  const row = rowOf(MIO);
  const hrefOf = (label: string): string | null => {
    const found = [...row.querySelectorAll("a")].find(
      (anchor) => anchor.textContent?.trim() === label,
    );
    expect(found, `「${label}」があること`).toBeDefined();
    return (found as HTMLAnchorElement).getAttribute("href");
  };

  expect(hrefOf(ATTRIBUTES_LINK)).toBe(
    hrefFor({ route: "attributes", handle: MIO.handle }),
  );
  expect(hrefOf("記憶")).toBe(hrefFor({ route: "memories", handle: MIO.handle }));
  expect(hrefOf("性格値と変容")).toBe(
    hrefFor({ route: "stats", handle: MIO.handle }),
  );
  // **3つが互いに別の行き先。**
  expect(
    new Set([
      hrefOf(ATTRIBUTES_LINK),
      hrefOf("記憶"),
      hrefOf("性格値と変容"),
    ]).size,
  ).toBe(3);

  await unmount(view, { outro: false });
});
