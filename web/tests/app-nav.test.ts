// @vitest-environment jsdom
/**
 * **画面を選ぶ帯**（作業単位 D-12b-1 / `src/App.svelte`）。
 *
 * # ここで見るものと、見ないもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **いまいる画面だけが `aria-current="page"` を持ち、移ると印も移ること** | **ここ** |
 * | **いまいる画面の見た目（太字・下の線）が、`aria-current` そのものから引かれていること** | **ここ** |
 * | **タブの押せる高さが 44px 以上であること** | **ここ** |
 * | **製品名が帯に在り、`nav` にも `main` にも入っていないこと** | **ここ** |
 * | **帯が名乗った `nav` で、`main` の外に在ること** | `tests/live-region.test.ts` |
 * | **帯の並び順** | `tests/app-routing-upstream.test.ts` |
 * | **配られた CSS に、その印の規則が本当に生えたか** | **どこにも無い**（下の ⚠） |
 *
 * # なぜ「印は `aria-current` から引く」を見るのか
 *
 * **いまいる画面を読み上げに告げる綴り（`aria-current`）と、目に見える印が
 * 別の条件から出ていると、片方だけが古びる**（CLAUDE.md の「写し」）。
 * **D-12b-1 では、印を `aria-[current=page]:` の変位で書いた** ——
 * **属性が付いた要素にだけ効く**ので、**条件はマークアップに1つしか無い。**
 * **3つのタブの `class` が一字一句同じであること**がその現れで、
 * **いまいる画面だけに別のクラスを足す形に戻すと、ここが赤になる。**
 *
 * # ⚠ 守れていないこと
 *
 * * **ここは Tailwind を走らせない。** 見ているのは `class` の綴りで、
 *   **その綴りから規則が生えたか**（`aria-[current=page]:border-read` が配られた CSS に在るか）は言えない。
 *   **D-12b-1 では `pnpm build` の出力を読んで確かめた。**
 * * **44px は「v4 の間隔の1目盛りが 0.25rem、根の字が 16px」を前提に数えている。**
 *   **`@theme` で `--spacing` を変えた日には、ここが黙って嘘になる。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import App from "../src/App.svelte";
import { noProposalsFor } from "./support/no-proposals";
import { readerInZone } from "./support/zone";

// **上流の画面が `browserZone()` を訊く**（`tests/app-routing-upstream.test.ts` と同じ理由）。
readerInZone("America/New_York");

/** **真ん中のタブ**（選ぶ対象を端に置かない / CLAUDE.md「見張り番の書き方」）。 */
const MIDDLE = { href: "#/characters", text: "キャラクター" };
/** 端のタブ（移った先）。 */
const LAST = { href: "#/upstream", text: "上流モデル" };

/** 製品名（帯の左端）。 */
const PRODUCT = "personacore";

/** **タブの押せる高さの下限**（Issue origin#88）。 */
const MIN_TAB_PX = 44;

let target: HTMLElement;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown) => {
    const url = String(input);
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
      return respond({ characters: [], unreadable: [] });
    }
    if (url === "/admin/upstream/profiles") {
      return respond({
        path: null,
        editable: false,
        state: "none",
        revision: null,
        problem: null,
        profiles: [],
      });
    }
    // **上流の観測は、この測定では読めなくてよい**（帯だけを見る）。
    return Promise.resolve({
      ok: false,
      status: 503,
      text: () => Promise.resolve("{}"),
    } as unknown as Response);
  });
}

function tabs(): HTMLAnchorElement[] {
  return [...target.querySelectorAll<HTMLAnchorElement>("nav a")];
}

/** `aria-current="page"` を持つタブの文言。 */
function current(): string[] {
  return tabs()
    .filter((tab) => tab.getAttribute("aria-current") === "page")
    .map((tab) => (tab.textContent ?? "").trim());
}

beforeEach(() => {
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

it("いまいる画面だけが `aria-current` を持ち、移ると印も移る", async () => {
  window.location.hash = MIDDLE.href;
  const view = mount(App, { target });
  try {
    await vi.waitFor(() => {
      expect(tabs().length).toBe(3);
    });
    expect(current()).toEqual([MIDDLE.text]);

    window.location.hash = LAST.href;
    await vi.waitFor(() => {
      expect(current()).toEqual([LAST.text]);
    });
    // **前の画面のタブに、印が残っていない**（上の `toEqual` が1件であることも言っている）。
    const middle = tabs().find((tab) => tab.getAttribute("href") === MIDDLE.href);
    expect(middle?.hasAttribute("aria-current")).toBe(false);
  } finally {
    await unmount(view, { outro: false });
  }
});

it("いまいる画面の印は、`aria-current` そのものから引く", async () => {
  window.location.hash = MIDDLE.href;
  const view = mount(App, { target });
  try {
    await vi.waitFor(() => {
      expect(current()).toEqual([MIDDLE.text]);
    });
    const classes = tabs().map((tab) => tab.getAttribute("class") ?? "");
    // **3つのタブの `class` が一字一句同じ** —— いまいる画面だけに別の印を足していない。
    expect(new Set(classes).size).toBe(1);
    const tokens = (classes[0] as string).split(/\s+/);
    // **太字と、下の read の線は、`aria-current="page"` が付いたときだけ効く綴り。**
    expect(tokens).toContain("aria-[current=page]:font-bold");
    expect(tokens).toContain("aria-[current=page]:border-read");
    // **⚠ 線の太さは、変位の付かない綴りが持つ**（D-12b-1 のレビュー）。
    // `border-read` は色しか持たないので、**太さの綴りを削ると線が消えるのに、上の2行は緑のまま**だった。
    // **`border-b`（1px）か `border-b-<n>`（n ≥ 1）が、変位なしでちょうど1つ在ること。**
    const widths = tokens
      .map((token) => /^border-b(?:-(\d+))?$/.exec(token))
      .filter((found) => found !== null)
      .map((found) => (found[1] === undefined ? 1 : Number.parseInt(found[1], 10)));
    expect(widths, "下の線の太さの綴りが1つ在ること").toHaveLength(1);
    expect(widths[0] as number).toBeGreaterThanOrEqual(1);
  } finally {
    await unmount(view, { outro: false });
  }
});

it("タブの押せる高さは 44px 以上", async () => {
  window.location.hash = MIDDLE.href;
  const view = mount(App, { target });
  try {
    await vi.waitFor(() => {
      expect(tabs().length).toBe(3);
    });
    for (const tab of tabs()) {
      const tokens = (tab.getAttribute("class") ?? "").split(/\s+/);
      // **変位の付かない `min-h-<数>` だけを数える**（`sm:min-h-…` は狭い幅で効かない）。
      const heights = tokens
        .map((token) => /^min-h-(\d+(?:\.\d+)?)$/.exec(token))
        .filter((found) => found !== null)
        .map((found) => Number.parseFloat(found[1] as string) * 4);
      expect(heights, `${tab.textContent} が min-h-<数> を1つ持つこと`).toHaveLength(1);
      expect(heights[0] as number).toBeGreaterThanOrEqual(MIN_TAB_PX);
    }
  } finally {
    await unmount(view, { outro: false });
  }
});

it("製品名は帯に在り、`nav` にも `main` にも入っていない", async () => {
  window.location.hash = MIDDLE.href;
  const view = mount(App, { target });
  try {
    await vi.waitFor(() => {
      expect(tabs().length).toBe(3);
    });
    const nav = target.querySelector("nav") as HTMLElement;
    const main = target.querySelector("main") as HTMLElement;
    const names = [...target.querySelectorAll("*")].filter(
      (element) =>
        element.children.length === 0 && (element.textContent ?? "").trim() === PRODUCT,
    );
    expect(names).toHaveLength(1);
    const name = names[0] as HTMLElement;
    expect(nav.contains(name)).toBe(false);
    expect(main.contains(name)).toBe(false);
    // **帯と同じ囲みに居る**（製品名と `nav` の共通の親が `main` を含まない）。
    expect(name.parentElement?.contains(nav)).toBe(true);
    expect(name.parentElement?.contains(main)).toBe(false);
  } finally {
    await unmount(view, { outro: false });
  }
});
