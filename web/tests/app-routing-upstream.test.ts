// @vitest-environment jsdom
/**
 * **`#/upstream` が、上流の画面まで届いているか**（作業単位 D-9 / `src/App.svelte`）。
 *
 * # なぜこの1本が要るのか
 *
 * **「配線だけの場所」は、層に関係なく観測対象から落ちる**（CLAUDE.md / `D-3` の実測）。
 * 部品（`router.ts` / `UpstreamPanel.svelte`）をいくら厚く見ても、
 * **それらを繋いでいる `App.svelte` の1行は誰も通らない。**
 *
 * **ナビもここで見る** —— 上流の画面は識別子を要らないので**ナビに並ぶ**側で、
 * **並んでいなければ、利用者はこの画面へ辿り着けない。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import App from "../src/App.svelte";
import type { UpstreamReport } from "../src/lib/api";
import { noProposalsFor } from "./support/no-proposals";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

/** 上流の画面のリンク先と見出し。**`router.ts` の綴りを直書きで突き合わせる。** */
const UPSTREAM_HREF = "#/upstream";
const UPSTREAM_LABEL = "上流モデル";

/** サーバが返す観測。**2つの面で1欄も同じ値にしていない。** */
const REPORT: UpstreamReport = {
  main: {
    face: "main",
    endpoint: "http://127.0.0.1:1234/v1/chat/completions",
    model: "main-model-for-tests",
    // **上流は、頼んだ名前と違うものを名乗っている**（作業単位 B-45 / `C-18`）。
    answered_name: "answering-model-for-tests.gguf",
    queried_context_tokens: 16384,
    answered_by: "native-api",
    unauthorized: false,
    checked_at: "2026-11-03T09:58:00+09:00",
    check_result: "answered",
    measured: {
      checked: 4,
      matched: 4,
      drifted_other: 0,
      drifted_absent: 0,
      unanswered: 0,
    },
    // **この起動は `--upstream` を渡していない**（ADR-0042 §決定3）。
    launch: {
      state: "external",
      since: null,
      answered_name: null,
      reason: null,
      log: null,
    },
  },
  sub: {
    face: "sub",
    endpoint: "http://127.0.0.1:5678/v1/chat/completions",
    model: "sub-model-for-tests",
    answered_name: "sub-model-for-tests",
    queried_context_tokens: 8192,
    answered_by: "openai-exact-id",
    unauthorized: false,
    checked_at: "2026-11-03T09:57:00+09:00",
    check_result: "answered",
    measured: {
      checked: 5,
      matched: 2,
      drifted_other: 1,
      drifted_absent: 1,
      unanswered: 1,
    },
    // **この起動は `--upstream` を渡していない**（ADR-0042 §決定3）。
    launch: {
      state: "external",
      since: null,
      answered_name: null,
      reason: null,
      log: null,
    },
  },
  // **この起動は見本を書いていない**（作業単位 F-4c）。
  profile_sample: null,
  // **この起動はプロファイルを選んでいない**（作業単位 F-10b）。
  selected_profile: null,
};

let target: HTMLElement;
let urls: string[];

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

    if (url === "/admin/upstream") {
      return respond(REPORT);
    }
    // **書いたプロファイルの一覧**（作業単位 F-4b-3）。この画面は観測と一緒に読む。
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
    if (url === "/admin/characters") {
      return respond({ characters: [], unreadable: [] });
    }
    return respond({});
  });
}

/** 表を、**行ごとのセルの配列**で読む。 */
function tableCells(): string[][] {
  return [...target.querySelectorAll("table tr")].map((row) =>
    [...row.querySelectorAll("th, td")].map((cell) =>
      (cell.textContent ?? "").replace(/\s+/gu, " ").trim(),
    ),
  );
}

/** ナビのリンク（`href` と文言の組）。 */
function navLinks(): { href: string; text: string }[] {
  return [...target.querySelectorAll("nav a")].map((link) => ({
    href: link.getAttribute("href") ?? "",
    text: (link.textContent ?? "").trim(),
  }));
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

it("`#/upstream` は、上流の観測を読んで出す", async () => {
  window.location.hash = UPSTREAM_HREF;
  const view = mount(App, { target });

  await vi.waitFor(() => {
    expect(target.querySelector("table")).not.toBeNull();
    expect(urls).toContain("/admin/upstream/profiles");
  });
  // **観測と、書いたプロファイルの一覧を1回ずつ**（作業単位 F-4b-3 で後者が増えた）。
  expect([...urls].sort()).toEqual(["/admin/upstream", "/admin/upstream/profiles"]);

  // **中身が、応答のとおりに出ている**（面ごとの列で）。
  // **行は見出しで引く**（作業単位 F-4b-3 で、観測の前に起こした状態の表が並んだ）。
  const cells = tableCells();
  expect(cells).toContainEqual(["応答が名乗った面", "main", "sub"]);
  expect(cells).toContainEqual([
    "依頼するモデルの綴り",
    "main-model-for-tests",
    "sub-model-for-tests",
  ]);

  await unmount(view, { outro: false });
});

it("ナビから辿れる（並び順ごと固定する）", async () => {
  window.location.hash = "#/chat";
  const view = mount(App, { target });
  await vi.waitFor(() => {
    expect(navLinks().length).toBeGreaterThan(0);
  });

  expect(navLinks()).toEqual([
    { href: "#/chat", text: "会話" },
    { href: "#/characters", text: "キャラクター" },
    { href: UPSTREAM_HREF, text: UPSTREAM_LABEL },
    // **作業単位 F-12b で「設定」が最後に並んだ**（`tests/app-routing-settings.test.ts`）。
    { href: "#/settings", text: "設定" },
  ]);

  await unmount(view, { outro: false });
});

/**
 * **別の画面から移っても、上流の画面になる。**
 *
 * `{:else if}` の綴りを間違えると、**何も描かれない**（受け皿にしていないので、
 * すぐ気付ける形になっている / `App.svelte` の注記）。
 */
it("会話の画面から移ると、上流の画面に入れ替わる", async () => {
  window.location.hash = "#/chat";
  const view = mount(App, { target });
  await vi.waitFor(() => {
    expect(urls).toContain("/admin/characters");
  });
  expect(target.querySelector("table")).toBeNull();

  window.location.hash = UPSTREAM_HREF;

  await vi.waitFor(() => {
    expect(target.querySelector("table")).not.toBeNull();
  });
  expect(urls).toContain("/admin/upstream");
  // **会話の画面は残っていない。**
  expect(target.querySelector("textarea")).toBeNull();

  await unmount(view, { outro: false });
});
