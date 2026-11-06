// @vitest-environment jsdom
/**
 * **一覧から約束・決まりの画面へ辿る導線**（作業単位 A-12a / `src/lib/CharacterList.svelte`）。
 *
 * `character-list-attributes.test.ts` と同じ形で、**その行のキャラクターへ行くか**（帰属）を見る。
 * **端の行を押さない**（3件にして真ん中を見る）。しまった一覧の行は
 * `character-list-archiving.test.ts` が見ている。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import CharacterList from "../src/lib/CharacterList.svelte";
import type { CharacterView } from "../src/lib/api";
import { hrefFor } from "../src/lib/router";

function character(id: number, handle: string, display_name: string): CharacterView {
  return {
    id,
    handle,
    display_name,
    base_persona: "静かな声で話す。",
    output_format_instruction: null,
    stat_scale: 5,
    stat_drift_mode: null,
    axis_discovery_slots: 0,
    archived_at: null,
    created_at: "2026-08-27T00:00:00Z",
  };
}

const AOI = character(1, "aoi", "藍生");
/** **真ん中の行**。 */
const MIO = character(2, "mio", "澪");
const NAR = character(3, "nar", "奈落");

const AGREEMENTS_LINK = "約束・決まり";

let target: HTMLElement;

beforeEach(() => {
  target = document.createElement("div");
  document.body.append(target);
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    // **一覧以外（行ごとの性格値の読み）は空の性格値で答える**（`character-list-attributes.test.ts` と同じ）。
    const body =
      url === "/admin/characters" && (init?.method ?? "GET") === "GET"
        ? { characters: [AOI, MIO, NAR], unreadable: [] }
        : { character: "?", stat_scale: 5, stats: [] };
    return Promise.resolve({
      ok: true,
      status: 200,
      text: () => Promise.resolve(JSON.stringify(body)),
    } as unknown as Response);
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

function linkOf(character: CharacterView, label: string): HTMLAnchorElement {
  const row = [...target.querySelectorAll("tbody tr")].find(
    (tr) => tr.querySelector("code")?.textContent?.trim() === character.handle,
  );
  expect(row, `${character.handle} の行があること`).toBeDefined();
  const found = [...(row as HTMLElement).querySelectorAll("a")].find(
    (anchor) => anchor.textContent?.trim() === label,
  );
  expect(found, `${character.handle} の行に「${label}」があること`).toBeDefined();
  return found as HTMLAnchorElement;
}

it("行の「約束・決まり」は、その行のキャラクターを指す", async () => {
  const view = mount(CharacterList, { target });
  await vi.waitFor(() => {
    expect(target.querySelectorAll("tbody tr").length).toBeGreaterThanOrEqual(3);
  });
  flushSync();

  const link = linkOf(MIO, AGREEMENTS_LINK);
  expect(link.getAttribute("href")).toBe(hrefFor({ route: "agreements", handle: MIO.handle }));
  expect(link.getAttribute("href")).not.toContain(AOI.handle);
  expect(link.getAttribute("href")).not.toContain(NAR.handle);
  expect(linkOf(AOI, AGREEMENTS_LINK).getAttribute("href")).toContain(AOI.handle);
  expect(linkOf(NAR, AGREEMENTS_LINK).getAttribute("href")).toContain(NAR.handle);

  // **同じ行の4つは互いに別の行き先**（綴りが1語しか違わない経路の取り違えを落とす）。
  expect(
    new Set(
      ["記憶", "性格値と変容", "属性", AGREEMENTS_LINK].map((label) =>
        linkOf(MIO, label).getAttribute("href"),
      ),
    ).size,
  ).toBe(4);

  await unmount(view, { outro: false });
});
