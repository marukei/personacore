// @vitest-environment jsdom
/**
 * **`#/settings` が、「設定」の画面まで届いているか**（作業単位 F-12b / `src/App.svelte`）。
 *
 * **配線だけの場所は、部品の被覆とは別に見る**（CLAUDE.md「見張り番の書き方」）。
 * `router.ts` と `SettingsPanel.svelte` を厚く見ても、それらを繋ぐ `App.svelte` の1行は誰も通らない。
 *
 * ナビの並び順は `tests/app-routing-upstream.test.ts` が固定している（ここでは「辿れる」ことだけを見る）。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import App from "../src/App.svelte";
import type { ServerView } from "../src/lib/api";
import { noProposalsFor } from "./support/no-proposals";
import { externalUpstreamFor } from "./support/upstream-external";
import { readerInZone } from "./support/zone";

// **上流の画面が `browserZone()` を訊く**（`tests/app-routing-upstream.test.ts` と同じ理由）。
readerInZone("America/New_York");

/** 「設定」の画面のリンク先と見出し。**`router.ts` の綴りを直書きで突き合わせる。** */
const SETTINGS_HREF = "#/settings";
const SETTINGS_LABEL = "設定";

const SERVER: ServerView = {
  listening_port: 8123,
  listening_port_source: "default",
  listening_port_requested: 8123,
  listening_port_shifted: false,
  listening_port_skipped: [],
  saved_port: null,
  saved_port_unreadable: false,
  next_port: 8789,
  next_port_source: "default",
  editable: true,
};

let target: HTMLElement;
let urls: string[];

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown) => {
    const url = String(input);
    urls.push(url);
    const proposals = noProposalsFor(url);
    if (proposals !== null) {
      return proposals;
    }
    // **会話の画面が上流の観測を読む**（本文の面の状態）。外の上流として答える。
    const upstream = externalUpstreamFor(url);
    if (upstream !== null) {
      return upstream;
    }
    const respond = (body: unknown): Promise<Response> =>
      Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(JSON.stringify(body)),
      } as unknown as Response);
    if (url === "/admin/server") {
      return respond(SERVER);
    }
    if (url === "/admin/characters") {
      return respond({ characters: [], unreadable: [] });
    }
    return respond({});
  });
}

/** 「設定」の画面の行（`<dt>` と `<dd>` の組）。 */
function rows(): string[][] {
  return [...target.querySelectorAll("main dl > div")].map((row) =>
    [...row.querySelectorAll("dt, dd")].map((cell) =>
      (cell.textContent ?? "").replace(/\s+/gu, " ").trim(),
    ),
  );
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

it("`#/settings` は、待ち受けのポートを読んで出す", async () => {
  window.location.hash = SETTINGS_HREF;
  const view = mount(App, { target });
  try {
    await vi.waitFor(() => {
      expect(rows().length).toBe(3);
    });
    // **2回**——設定の画面と、全部の画面の上のずらしの帯（作業単位 F-11c / `PortShiftNotice.svelte`）が、
    // それぞれ開いたときに1回ずつ読む。ほかの口は読まない。
    expect(urls).toEqual(["/admin/server", "/admin/server"]);
    expect(target.querySelector("main h1")?.textContent?.trim()).toBe(SETTINGS_LABEL);
    expect(rows()[0]).toEqual(["いま待ち受けている番号", "8123（既定の番号）"]);
  } finally {
    await unmount(view, { outro: false });
  }
});

it("ナビから辿れる（リンク先と文言）", async () => {
  window.location.hash = "#/chat";
  const view = mount(App, { target });
  try {
    await vi.waitFor(() => {
      expect(target.querySelectorAll("nav a").length).toBeGreaterThan(0);
    });
    const links = [...target.querySelectorAll("nav a")].map((link) => ({
      href: link.getAttribute("href") ?? "",
      text: (link.textContent ?? "").trim(),
    }));
    expect(links).toContainEqual({ href: SETTINGS_HREF, text: SETTINGS_LABEL });
  } finally {
    await unmount(view, { outro: false });
  }
});

/**
 * **別の画面から移っても、設定の画面になる**（`{:else if}` の綴りを間違えると何も描かれない）。
 */
it("会話の画面から移ると、設定の画面に入れ替わる", async () => {
  window.location.hash = "#/chat";
  const view = mount(App, { target });
  try {
    await vi.waitFor(() => {
      expect(urls).toContain("/admin/characters");
    });
    expect(rows()).toEqual([]);

    window.location.hash = SETTINGS_HREF;
    await vi.waitFor(() => {
      expect(rows().length).toBe(3);
    });
    expect(urls).toContain("/admin/server");
    // **会話の画面は残っていない。**
    expect(target.querySelector("textarea")).toBeNull();
  } finally {
    await unmount(view, { outro: false });
  }
});
