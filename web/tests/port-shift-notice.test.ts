// @vitest-environment jsdom
/**
 * **ずらしの帯が、本物の `App.svelte` の帯の中に出るか**（作業単位 F-11c / ADR-0057 §決定4「画面」）。
 *
 * **配線だけの場所は、部品の被覆とは別に見る**（CLAUDE.md「見張り番の書き方」）——帯の部品
 * （`PortShiftNotice.svelte`）と文（`portShift.ts` / `tests/port-shift.test.ts`）を厚く見ても、それを
 * `header` に置く `App.svelte` の1行は誰も通らない。だからここは `App` を丸ごと `mount` する。
 *
 * | 主張 | 破れると |
 * |---|---|
 * | ずらしたら、帯が `header` の中・`main` の外に出て、文と「設定」へのリンク（`#/settings`）を持つ | どの画面からも見えない・直す場所へ行けない |
 * | どの画面でも出る（会話・上流） | 画面ごとに出たり出なかったりする |
 * | ずらしていなければ出ない | 何も起きていない起動に帯が出る |
 * | `GET /admin/server` が失敗しても帯を出さず、ナビと画面は壊れない | 帯の失敗で画面全体が止まる |
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

const SHIFTED: ServerView = {
  listening_port: 9102,
  listening_port_source: "saved",
  listening_port_requested: 9100,
  listening_port_shifted: true,
  listening_port_skipped: [9101],
  saved_port: 9100,
  saved_port_unreadable: false,
  next_port: 9100,
  next_port_source: "saved",
  editable: true,
};

const NOT_SHIFTED: ServerView = {
  ...SHIFTED,
  listening_port: 9100,
  listening_port_shifted: false,
  listening_port_skipped: [],
};

let target: HTMLElement;
/** `GET /admin/server` に何を返すか（`"fail"` は繋がらない）。 */
let server: ServerView | "fail";

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown) => {
    const url = String(input);
    const proposals = noProposalsFor(url);
    if (proposals !== null) {
      return proposals;
    }
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
      return server === "fail" ? Promise.reject(new TypeError("Failed to fetch")) : respond(server);
    }
    if (url === "/admin/characters") {
      return respond({ characters: [], unreadable: [] });
    }
    return respond({});
  });
}

/** 帯の段落（`header` の中で、ナビの外にある段落）。 */
function banner(): HTMLElement | null {
  return target.querySelector<HTMLElement>("header > div > p");
}

function bannerText(): string {
  return (banner()?.textContent ?? "").replace(/\s+/gu, " ").trim();
}

beforeEach(() => {
  window.location.hash = "";
  target = document.createElement("div");
  document.body.append(target);
  server = SHIFTED;
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
  window.location.hash = "";
});

it("ずらしたら、帯が header の中・main の外に出て、文と「設定」へのリンクを持つ", async () => {
  window.location.hash = "#/chat";
  const view = mount(App, { target });
  try {
    await vi.waitFor(() => {
      expect(banner()).not.toBeNull();
    });
    expect(bannerText()).toBe(
      "ポート 9100 は他のプログラムが使っているなどの理由で使えなかったので、9102 で起動しました。" +
        "ブックマークや外部のクライアントに 9100 を書いているなら、9102 に直してください。" +
        "番号は「設定」の画面で変えられます（次の起動から効きます）。",
    );
    const link = banner()?.querySelector("a");
    expect({ href: link?.getAttribute("href"), text: link?.textContent?.trim() }).toEqual({
      href: SETTINGS_HREF,
      text: SETTINGS_LABEL,
    });
    // **`main` の外**（どの画面の中身でもない）。ナビの中でもない。
    expect(banner()?.closest("main")).toBeNull();
    expect(banner()?.closest("nav")).toBeNull();
    expect(banner()?.closest("header")).not.toBeNull();
  } finally {
    await unmount(view, { outro: false });
  }
});

it("ほかの画面（上流）でも出る", async () => {
  window.location.hash = "#/upstream";
  const view = mount(App, { target });
  try {
    await vi.waitFor(() => {
      expect(bannerText()).toContain("9102 で起動しました");
    });
  } finally {
    await unmount(view, { outro: false });
  }
});

it("ずらしていなければ出ない", async () => {
  server = NOT_SHIFTED;
  window.location.hash = "#/chat";
  const view = mount(App, { target });
  try {
    // **会話の画面が読み終わるのを待ってから見る**（帯の読みも同じ周で終わっている）。
    await vi.waitFor(() => {
      expect(target.querySelector("main")).not.toBeNull();
    });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(banner()).toBeNull();
    expect(target.querySelector("header")?.textContent).not.toContain("で起動しました");
  } finally {
    await unmount(view, { outro: false });
  }
});

it("`GET /admin/server` が失敗しても帯を出さず、ナビと設定の画面へは行ける", async () => {
  server = "fail";
  window.location.hash = "#/chat";
  const view = mount(App, { target });
  try {
    await vi.waitFor(() => {
      expect(target.querySelectorAll("nav a").length).toBeGreaterThan(0);
    });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(banner()).toBeNull();
    // **他の画面は壊れていない**——設定の画面へ移れて、その画面は自分の失敗を言う。
    window.location.hash = SETTINGS_HREF;
    await vi.waitFor(() => {
      expect(target.querySelector("main h1")?.textContent?.trim()).toBe(SETTINGS_LABEL);
    });
  } finally {
    await unmount(view, { outro: false });
  }
});

/**
 * **対照: 帯の段落を拾う綴りが、帯が出たときだけ当たる**（ずらしていない腕の `null` が、拾い方の空振り
 * でないこと）。上の1本目が同じ拾い方で当たっているので、ここでは別の画面（設定）でも当たることを見る。
 */
it("対照: 設定の画面でも、同じ拾い方で帯が当たる", async () => {
  window.location.hash = SETTINGS_HREF;
  const view = mount(App, { target });
  try {
    await vi.waitFor(() => {
      expect(bannerText()).toContain("ポート 9100 は他のプログラムが使っているなどの理由で使えなかったので");
    });
  } finally {
    await unmount(view, { outro: false });
  }
});
