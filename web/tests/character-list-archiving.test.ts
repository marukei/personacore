// @vitest-environment jsdom
/**
 * **一覧からしまう・しまった一覧から戻す・完全に消す**（作業単位 A-10 / `src/lib/CharacterList.svelte`）。
 *
 * # 何を見るのか
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 一覧の出し分け・識別子の照合（純粋関数） | `character-archive.test.ts` |
 * | **どの行を押したら、どの識別子へ何が飛ぶか** | **ここ** |
 * | **しまったものが上の一覧に出ず、下の一覧に出ること** | **ここ** |
 * | **完全に消す前に識別子を打ち込ませること** | **ここ** |
 *
 * # ⚠ **押す行を端に置かない**
 *
 * 「選ぶ」操作なので、3件のうち**真ん中（MIO）**を押す（`character-list-deleting.test.ts` と同じ理由）。
 * しまった一覧の側も3件にして真ん中を押す。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CharacterList from "../src/lib/CharacterList.svelte";
import type { CharacterView } from "../src/lib/api";

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
    created_at: "2026-10-21T00:00:00Z",
    archived_at: null,
  };
}

const AOI = character(1, "aoi", "藍生");
const MIO = character(2, "mio", "澪");
const NAR = character(3, "nar", "奈落");
const ALL = [AOI, MIO, NAR];

/** しまった時刻（サーバが書く綴り）。 */
const ARCHIVED_AT = "2026-10-21T03:04:05.678Z";

interface Call {
  method: string;
  url: string;
  body: unknown;
}

let calls: Call[];
let target: HTMLElement;
/** サーバの側の状態。**一覧はここを引いた結果を返す。** */
let archived: Set<string>;
let deleted: Set<string>;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const body: unknown =
      typeof init?.body === "string" ? JSON.parse(init.body) : undefined;
    calls.push({ method, url, body });

    const respond = (payload: unknown, status = 200): Promise<Response> =>
      Promise.resolve({
        ok: status >= 200 && status < 300,
        status,
        text: () => Promise.resolve(JSON.stringify(payload)),
      } as unknown as Response);
    const view = (c: CharacterView): CharacterView => ({
      ...c,
      archived_at: archived.has(c.handle) ? ARCHIVED_AT : null,
    });

    if (url === "/admin/characters" && method === "GET") {
      return respond({
        characters: ALL.filter((c) => !deleted.has(c.handle)).map(view),
        unreadable: [],
      });
    }
    if (url.endsWith("/stats")) {
      return respond({ character: "?", stat_scale: 5, stats: [] });
    }
    const handle = url.slice("/admin/characters/".length);
    const found = ALL.find((c) => c.handle === handle);
    if (method === "PATCH" && found !== undefined) {
      const archive = (body as { archived?: boolean }).archived;
      if (archive === true) {
        archived.add(handle);
      } else if (archive === false) {
        archived.delete(handle);
      }
      return respond({ id: found.id, character: view(found) });
    }
    if (method === "DELETE" && found !== undefined) {
      deleted.add(handle);
      return Promise.resolve({
        ok: true,
        status: 204,
        text: () => Promise.resolve(""),
      } as unknown as Response);
    }
    return respond({ error: { message: "?", type: "invalid_request_error" } }, 500);
  });
}

/** 上の一覧（しまっていないキャラクター）の識別子。 */
function activeShown(): string[] {
  return [...target.querySelectorAll("[data-active-character]")].map(
    (row) => (row as HTMLElement).dataset["activeCharacter"] ?? "",
  );
}

/** しまった一覧の識別子。 */
function archivedShown(): string[] {
  return [...target.querySelectorAll("[data-archived-character]")].map(
    (row) => (row as HTMLElement).dataset["archivedCharacter"] ?? "",
  );
}

/** 行の中のボタン（**行から引く**。全体から引くと1件目に当たり続ける）。 */
function buttonIn(selector: string, label: string): HTMLButtonElement {
  const row = target.querySelector(selector);
  expect(row, `${selector} の行があること`).not.toBeNull();
  const found = [...(row as HTMLElement).querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `${selector} の行に「${label}」があること`).toBeDefined();
  return found as HTMLButtonElement;
}

function purgePanel(): HTMLElement {
  const found = target.querySelector("[data-confirm-purge]");
  expect(found, "完全に消す確認が開いていること").not.toBeNull();
  return found as HTMLElement;
}

function purgeButton(): HTMLButtonElement {
  const found = [...purgePanel().querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === "完全に消す",
  );
  expect(found, "確認に「完全に消す」があること").toBeDefined();
  return found as HTMLButtonElement;
}

function typeHandle(text: string): void {
  const input = purgePanel().querySelector("input");
  expect(input, "識別子を打つ欄があること").not.toBeNull();
  const field = input as HTMLInputElement;
  field.value = text;
  field.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

function writes(): Call[] {
  return calls.filter((call) => call.method !== "GET");
}

async function waitForActive(count: number): Promise<void> {
  await vi.waitFor(() => {
    expect(activeShown().length).toBe(count);
  });
}

beforeEach(() => {
  calls = [];
  archived = new Set();
  deleted = new Set();
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("しまう", () => {
  it("押した行の識別子へ `archived: true` を送り、その行は下の一覧へ移る", async () => {
    const view = mount(CharacterList, { target });
    await waitForActive(3);
    expect(archivedShown()).toEqual([]);

    buttonIn('[data-active-character="mio"]', "しまう").click();

    await vi.waitFor(() => {
      expect(archivedShown()).toEqual(["mio"]);
    });
    expect(writes()).toEqual([
      { method: "PATCH", url: "/admin/characters/mio", body: { archived: true } },
    ]);
    expect(activeShown()).toEqual(["aoi", "nar"]);

    await unmount(view, { outro: false });
  });

  it("しまったキャラクターは、上の一覧に出ない", async () => {
    archived = new Set(["mio"]);
    const view = mount(CharacterList, { target });
    await waitForActive(2);

    expect(activeShown()).toEqual(["aoi", "nar"]);
    expect(archivedShown()).toEqual(["mio"]);

    await unmount(view, { outro: false });
  });
});

describe("しまった一覧から戻す", () => {
  it("押した行の識別子へ `archived: false` を送り、上の一覧へ戻る", async () => {
    archived = new Set(["aoi", "mio", "nar"]);
    const view = mount(CharacterList, { target });
    await vi.waitFor(() => {
      expect(archivedShown()).toEqual(["aoi", "mio", "nar"]);
    });
    expect(activeShown()).toEqual([]);

    buttonIn('[data-archived-character="mio"]', "戻す").click();

    await waitForActive(1);
    expect(activeShown()).toEqual(["mio"]);
    expect(archivedShown()).toEqual(["aoi", "nar"]);
    expect(writes()).toEqual([
      { method: "PATCH", url: "/admin/characters/mio", body: { archived: false } },
    ]);

    await unmount(view, { outro: false });
  });
});

describe("しまった一覧から中を見る（消す前に確かめられるように）", () => {
  /** その行のリンクの `(文言, 行き先)`。**行から引く。** */
  function linksIn(selector: string): [string, string][] {
    const row = target.querySelector(selector);
    expect(row, `${selector} の行があること`).not.toBeNull();
    return [...(row as HTMLElement).querySelectorAll("a")].map((link) => [
      link.textContent?.trim() ?? "",
      link.getAttribute("href") ?? "",
    ]);
  }

  it("真ん中の行に、その行のキャラクターの記憶・性格値・属性・約束へのリンクがある", async () => {
    archived = new Set(["aoi", "mio", "nar"]);
    const view = mount(CharacterList, { target });
    await vi.waitFor(() => {
      expect(archivedShown()).toEqual(["aoi", "mio", "nar"]);
    });

    expect(linksIn('[data-archived-character="mio"]')).toEqual([
      ["記憶", "#/memories/mio"],
      ["性格値と変容", "#/stats/mio"],
      ["属性", "#/attributes/mio"],
      // **約束・決まり**（作業単位 A-12a）。しまった後も中を見てから消すかを決められるように。
      ["約束・決まり", "#/agreements/mio"],
    ]);

    await unmount(view, { outro: false });
  });

  it("上の一覧の行と同じ導線である（文言と行き先の形が揃っている）", async () => {
    archived = new Set(["mio"]);
    const view = mount(CharacterList, { target });
    await vi.waitFor(() => {
      expect(archivedShown()).toEqual(["mio"]);
    });

    const above = linksIn('[data-active-character="nar"]').map(([label, href]) => [
      label,
      href.replace(/\/nar$/, "/<handle>"),
    ]);
    const below = linksIn('[data-archived-character="mio"]').map(([label, href]) => [
      label,
      href.replace(/\/mio$/, "/<handle>"),
    ]);
    expect(below).toEqual(above);

    await unmount(view, { outro: false });
  });
});

describe("完全に消す（識別子を打ち込ませる）", () => {
  async function openPurgeOfMio(): Promise<ReturnType<typeof mount>> {
    archived = new Set(["aoi", "mio", "nar"]);
    const view = mount(CharacterList, { target });
    await vi.waitFor(() => {
      expect(archivedShown()).toEqual(["aoi", "mio", "nar"]);
    });
    buttonIn('[data-archived-character="mio"]', "完全に消す").click();
    flushSync();
    return view;
  }

  it("押しただけでは何も送らず、押した行のキャラクターを名乗って、取り消せないことを言う", async () => {
    const view = await openPurgeOfMio();

    const panel = purgePanel();
    expect(panel.dataset["confirmPurge"]).toBe("mio");
    const text = panel.textContent ?? "";
    expect(text).toContain(MIO.display_name);
    expect(text).not.toContain(AOI.display_name);
    expect(text).not.toContain(NAR.display_name);
    expect(text).toContain("取り消せません");
    expect(text).toContain("会話");
    expect(text).toContain("記憶");
    expect(writes()).toEqual([]);

    await unmount(view, { outro: false });
  });

  it.each([
    ["空のまま", ""],
    ["隣の識別子", "aoi"],
    ["後ろに空白", "mio "],
    ["大文字", "MIO"],
  ])("打ち間違えると（%s）、消せない", async (_, typed) => {
    const view = await openPurgeOfMio();

    typeHandle(typed);
    expect(purgeButton().disabled).toBe(true);
    purgeButton().click();
    flushSync();

    expect(writes()).toEqual([]);
    expect(archivedShown()).toEqual(["aoi", "mio", "nar"]);

    await unmount(view, { outro: false });
  });

  it("正しく打つと、その識別子へ `DELETE` が飛び、一覧から消える", async () => {
    const view = await openPurgeOfMio();

    typeHandle("mio");
    expect(purgeButton().disabled).toBe(false);
    purgeButton().click();

    await vi.waitFor(() => {
      expect(archivedShown()).toEqual(["aoi", "nar"]);
    });
    expect(writes()).toEqual([
      { method: "DELETE", url: "/admin/characters/mio", body: undefined },
    ]);
    expect(target.querySelector("[data-confirm-purge]")).toBeNull();

    await unmount(view, { outro: false });
  });

  it("「やめる」で閉じ、何も送らない", async () => {
    const view = await openPurgeOfMio();

    typeHandle("mio");
    const cancel = [...purgePanel().querySelectorAll("button")].find(
      (button) => button.textContent?.trim() === "やめる",
    ) as HTMLButtonElement;
    cancel.click();
    flushSync();

    expect(target.querySelector("[data-confirm-purge]")).toBeNull();
    expect(writes()).toEqual([]);

    await unmount(view, { outro: false });
  });
});
