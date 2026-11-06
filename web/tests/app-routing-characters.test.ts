// @vitest-environment jsdom
/**
 * **`#/characters` の繋ぎ目**（作業単位 D-6 / `src/App.svelte`）。
 *
 * # なぜこの1本が要るのか
 *
 * **「配線だけの場所」は、層に関係なく観測対象から落ちる**（CLAUDE.md）。
 * **`App.svelte` を mount するのは `app-routing*.test.ts` だけ**で、
 * D-3 のレビューの時点では**記憶の画面しか通っていなかった** ——
 * つまり **`#/characters` → `CharacterList` の1行は、誰も通っていない。**
 *
 * **D-6 の入口はその先にある**（一覧の「読み出せなかった行」の中）。
 * 部品の被覆率は組み上げの被覆を1ミリも意味しないので、
 * **経路の頭から、修復のフォームが開くところまでを1本で通す。**
 *
 * # `character-list-repair.test.ts` との分担
 *
 * あちらは `CharacterList` を直に mount して、**押した行と送り先の帰属**を見る。
 * **ここが見るのは「ハッシュから、その画面に辿り着けること」だけ。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import App from "../src/App.svelte";
import type { CharacterView, UnreadableCharacterView } from "../src/lib/api";
import { noProposalsFor } from "./support/no-proposals";

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

/** 直せる行・1件目。 */
const KOU: UnreadableCharacterView = {
  id: 11,
  handle: "kou",
  invalid_columns: ["display_name"],
};

/** **真ん中**（直せる行のうち、先頭でも最終でもない）。 */
const SEN: UnreadableCharacterView = {
  id: 12,
  handle: "sen",
  invalid_columns: ["base_persona", "stat_scale"],
};

/** 直せる行・3件目。**SEN を端に置かないために居る。** */
const TSUKI: UnreadableCharacterView = {
  id: 14,
  handle: "tsuki",
  invalid_columns: ["output_format_instruction"],
};

let urls: string[];
let target: HTMLElement;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown) => {
    const url = String(input);
    urls.push(url);
    // **約束・決まりの提案**（作業単位 A-12c-2 / 会話の画面が読む）。**この測定は提案を持たない**（`no-proposals.ts`）。
    const proposals = noProposalsFor(url);
    if (proposals !== null) {
      return proposals;
    }

    const respond = (body: unknown): Promise<Response> =>
      Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(JSON.stringify(body)),
      } as unknown as Response);

    if (url === "/admin/characters") {
      return respond({
        characters: [AOI],
        unreadable: [KOU, SEN, TSUKI],
      });
    }
    return respond({ character: AOI.handle, stat_scale: 5, stats: [] });
  });
}

beforeEach(() => {
  urls = [];
  window.location.hash = "";
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
  window.location.hash = "";
});

it("`#/characters` は、キャラクターの一覧を読む", async () => {
  window.location.hash = "#/characters";
  const view = mount(App, { target });

  await vi.waitFor(() => {
    expect(urls).toContain("/admin/characters");
  });
  await vi.waitFor(() => {
    expect(target.querySelectorAll("tr[data-unreadable-id]").length).toBe(3);
  });
  // **記憶の画面ではない**（経路を取り違えていない）。
  expect(urls.join(" ")).not.toContain("/memories");

  await unmount(view, { outro: false });
});

/**
 * **壊れた行の修復まで、経路の頭から辿れる。**
 *
 * ここが赤くなるのは「`App` が `CharacterList` を出していない」か
 * 「一覧が壊れた行を出していない」か「修復の入口が無い」の**どれか**。
 * **帰属（どの行を押したらどこへ送るか）は
 * `character-list-repair.test.ts` が見る**ので、ここでは重ねない。
 */
it("壊れた行の修復が、`#/characters` から辿れる", async () => {
  window.location.hash = "#/characters";
  const view = mount(App, { target });

  await vi.waitFor(() => {
    expect(target.querySelectorAll("tr[data-unreadable-id]").length).toBe(3);
  });

  const row = target.querySelector(`tr[data-unreadable-id="${SEN.id}"]`);
  expect(row).not.toBeNull();
  const button = [...(row as HTMLElement).querySelectorAll("button")].find(
    (found) => found.textContent?.trim() === "修復",
  );
  expect(button, "真ん中の壊れた行に「修復」があること").toBeDefined();

  (button as HTMLButtonElement).click();
  flushSync();

  const heading = target.querySelector("[data-repair-row]");
  expect(heading).not.toBeNull();
  expect((heading as HTMLElement).textContent).toContain(SEN.handle as string);
  expect((heading as HTMLElement).textContent).not.toContain(
    KOU.handle as string,
  );
  expect((heading as HTMLElement).textContent).not.toContain(
    TSUKI.handle as string,
  );

  await unmount(view, { outro: false });
});
