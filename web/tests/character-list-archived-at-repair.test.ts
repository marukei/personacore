// @vitest-environment jsdom
/**
 * **しまった時刻が読めない行を、画面から直せる**（作業単位 A-10 / `src/lib/CharacterList.svelte`）。
 *
 * しまった時刻が読めない行は、上の一覧にも、しまった一覧にも出ない（読み出せなかった行に出る）。
 * 直す口は `PATCH {"archived": false}`（時刻を消す＝戻す）だけで、**その入口が画面に無い状態を無くす。**
 *
 * 3行並べて**真ん中だけ**が時刻の壊れた行（端に置くと「常に先頭」「常に最終」が通り抜ける）。
 * 識別子まで壊れた行には出ない（`PATCH` で指せない）。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CharacterList from "../src/lib/CharacterList.svelte";
import type { UnreadableCharacterView } from "../src/lib/api";
import { archivedAtFixOf } from "../src/lib/characterRepair";

const DISPLAY_BROKEN: UnreadableCharacterView = {
  id: 1,
  handle: "akane",
  invalid_columns: ["display_name"],
};
const TIME_BROKEN: UnreadableCharacterView = {
  id: 2,
  handle: "mio",
  invalid_columns: ["archived_at"],
};
const HANDLE_BROKEN: UnreadableCharacterView = {
  id: 3,
  handle: null,
  invalid_columns: ["handle", "archived_at"],
};

interface Call {
  method: string;
  url: string;
  body: unknown;
}

let calls: Call[];
let target: HTMLElement;
let unreadable: UnreadableCharacterView[];

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const body: unknown =
      typeof init?.body === "string" ? JSON.parse(init.body) : undefined;
    calls.push({ method, url, body });
    const respond = (payload: unknown): Promise<Response> =>
      Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(JSON.stringify(payload)),
      } as unknown as Response);

    if (url === "/admin/characters" && method === "GET") {
      return respond({ characters: [], unreadable });
    }
    if (method === "PATCH" && url === "/admin/characters/mio") {
      // **直った**ことにする（次の一覧ではこの行が消える）。
      unreadable = unreadable.filter((row) => row.handle !== "mio");
      return respond({ id: 2, character: null });
    }
    return respond({});
  });
}

function fixButtons(): string[] {
  return [...target.querySelectorAll("[data-clear-archived-at]")].map(
    (button) => (button as HTMLElement).dataset["clearArchivedAt"] ?? "",
  );
}

beforeEach(() => {
  calls = [];
  unreadable = [DISPLAY_BROKEN, TIME_BROKEN, HANDLE_BROKEN];
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("archivedAtFixOf（純粋関数）", () => {
  it("時刻が壊れ、識別子が読める行だけが直せる", () => {
    expect(archivedAtFixOf(DISPLAY_BROKEN)).toBeNull();
    expect(archivedAtFixOf(TIME_BROKEN)).toBe("mio");
    expect(archivedAtFixOf(HANDLE_BROKEN)).toBeNull();
  });
});

describe("読み出せなかった行の「しまった時刻を消す」", () => {
  it("時刻が壊れた行にだけ出る", async () => {
    const view = mount(CharacterList, { target });
    await vi.waitFor(() => {
      expect(target.querySelectorAll("[data-unreadable-id]").length).toBe(3);
    });

    expect(fixButtons()).toEqual(["mio"]);
    const row = target.querySelector('[data-unreadable-id="2"]') as HTMLElement;
    expect(row.querySelector("[data-clear-archived-at]")).not.toBeNull();

    await unmount(view, { outro: false });
  });

  it("押すと、その識別子へ `archived: false` を送り、一覧を読み直す", async () => {
    const view = mount(CharacterList, { target });
    await vi.waitFor(() => {
      expect(fixButtons()).toEqual(["mio"]);
    });

    (target.querySelector("[data-clear-archived-at]") as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(target.querySelectorAll("[data-unreadable-id]").length).toBe(2);
    });
    expect(calls.filter((call) => call.method !== "GET")).toEqual([
      { method: "PATCH", url: "/admin/characters/mio", body: { archived: false } },
    ]);
    expect(fixButtons()).toEqual([]);

    await unmount(view, { outro: false });
  });
});
