// @vitest-environment jsdom
/**
 * **一覧から性格値と変容の画面へ辿る導線**（作業単位 D-4 /
 * `src/lib/CharacterList.svelte`）。
 *
 * # なぜ別ファイルなのか
 *
 * `character-list-memories.test.ts` が記憶への行き先を見るのと同じ形で、
 * **ここは性格値への行き先だけ**を見る ——**その行のキャラクターへ行くか**（帰属）。
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
const STATS_LINK = "性格値と変容";

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

/** その行の「性格値と変容」リンク。**行から引く**（全体から引くと1件目に当たり続ける）。 */
function statsLinkOf(character: CharacterView): HTMLAnchorElement {
  const row = [...target.querySelectorAll("tbody tr")].find(
    (tr) => tr.querySelector("code")?.textContent?.trim() === character.handle,
  );
  expect(row, `${character.handle} の行があること`).toBeDefined();
  const found = [...(row as HTMLElement).querySelectorAll("a")].find(
    (anchor) => anchor.textContent?.trim() === STATS_LINK,
  );
  expect(
    found,
    `${character.handle} の行に「${STATS_LINK}」があること`,
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

it("行の「性格値と変容」は、その行のキャラクターを指す", async () => {
  const view = mount(CharacterList, { target });
  await vi.waitFor(() => {
    expect(target.querySelectorAll("tbody tr").length).toBeGreaterThanOrEqual(3);
  });
  flushSync();

  const link = statsLinkOf(MIO);
  expect(link.getAttribute("href")).toBe(
    hrefFor({ route: "stats", handle: MIO.handle }),
  );
  expect(link.getAttribute("href")).toContain(MIO.handle);
  // **対照: 前後の行のどちらへも向いていない**（「常に先頭」「常に最終」を両方落とす）。
  expect(link.getAttribute("href")).not.toContain(AOI.handle);
  expect(link.getAttribute("href")).not.toContain(NAR.handle);
  // 端の行も、それぞれ自分を指す。
  expect(statsLinkOf(AOI).getAttribute("href")).toContain(AOI.handle);
  expect(statsLinkOf(NAR).getAttribute("href")).toContain(NAR.handle);

  await unmount(view, { outro: false });
});

it("記憶のリンクと行き先が入れ替わっていない", async () => {
  const view = mount(CharacterList, { target });
  await vi.waitFor(() => {
    expect(target.querySelectorAll("tbody tr").length).toBeGreaterThanOrEqual(3);
  });
  flushSync();

  const row = [...target.querySelectorAll("tbody tr")].find(
    (tr) => tr.querySelector("code")?.textContent?.trim() === MIO.handle,
  ) as HTMLElement;
  const memories = [...row.querySelectorAll("a")].find(
    (anchor) => anchor.textContent?.trim() === "記憶",
  );
  expect(memories).toBeDefined();
  expect((memories as HTMLAnchorElement).getAttribute("href")).toBe(
    hrefFor({ route: "memories", handle: MIO.handle }),
  );
  expect(statsLinkOf(MIO).getAttribute("href")).not.toBe(
    (memories as HTMLAnchorElement).getAttribute("href"),
  );

  await unmount(view, { outro: false });
});
