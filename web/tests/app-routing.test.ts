// @vitest-environment jsdom
/**
 * **経路が運ぶ識別子が、画面の中身まで届いているか**（作業単位 D-3 / `src/App.svelte`）。
 *
 * # なぜこの1本が要るのか
 *
 * `router.ts` の doc は **「画面が A を名乗ったまま、中身が B のもの」**という
 * 食い違い方を名指ししていて、**それが起きる唯一の場所が `App.svelte` の
 * `{#key screen.handle}` と `handle={screen.handle}` の2行**である。
 *
 * **D-3 のレビューで、その2行がどちらも無防備だと実測された**
 * （`handle="aoi"` に決め打つ変異も、`{#key "fixed"}` にする変異も 107 全緑）。
 * **doc が「守っている」と書いた場所を、誰も見ていなかった。**
 *
 * # `{#key}` を外すと何が起きるか（**推論ではなく、ここで観測する**）
 *
 * `MemoryList.svelte` は初期化のときに1回だけ `void load()` を呼ぶ（`$effect` を持たない）。
 * したがって**作り直されなければ、識別子が変わっても読み直しが起きない** ——
 * **見出しだけが新しいキャラクターを名乗り、表には前のキャラクターの記憶が残る。**
 * このファイルの2本目が、その状態を実際に赤にする。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import App from "../src/App.svelte";
import type { MemoryView } from "../src/lib/api";
import { noProposalsFor } from "./support/no-proposals";
import { readerInZone } from "./support/zone";

/**
 * **読み手の時間帯を名乗る**（作業単位 F-7d）。
 *
 * このファイルは**時刻の綴りを1つも見ていない**が、
 * **`App.svelte` 越しに `MemoryList` を開く**ので、
 * 画面は `browserZone()` で機械に訊く。
 * **名乗らないと、その読みはその場で落ちる**（`tests/support/ambient-zone.ts`）。
 *
 * **どの時間帯でもよい** —— ここが見ているのは経路であって時刻ではない。
 * **名乗る意味は「この測定が機械の既定に寄りかかっていない」ことを固定する**点にある。
 */
readerInZone("America/New_York");

/** 1人目。 */
const MIO = "mio";
/** 2人目。**1人目と1文字も重ならない綴り。** */
const NAR = "nar";

/** `mio` の記憶。**`nar` のものと id も本文も違う。** */
const MIO_MEMORY: MemoryView = {
  id: 22,
  subject: "character",
  content: "喫茶店でアルバイトをしている",
  category: "経歴",
  created_at: "2026-08-27T00:10:00Z",
};

/** `nar` の記憶。 */
const NAR_MEMORY: MemoryView = {
  id: 55,
  subject: "user",
  content: "朝は紅茶を飲む",
  category: null,
  created_at: "2026-08-27T00:20:00Z",
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

    // **識別子ごとに違うものを返す**（同じものを返すと取り違えが観測できない）。
    // **問い合わせは外して見る**（記憶の一覧は `?include=superseded` を付ける /
    // 作業単位 D-3c）。**ここが見ているのは経路であって、問い合わせではない。**
    const path = pathOf(url);
    if (path === `/admin/characters/${MIO}/memories`) {
      return respond({
        character: MIO,
        memories: [MIO_MEMORY],
        unreadable: [],
      });
    }
    if (path === `/admin/characters/${NAR}/memories`) {
      return respond({
        character: NAR,
        memories: [NAR_MEMORY],
        unreadable: [],
      });
    }
    // 別の識別子（決め打ちの変異はここへ来る）。**空を返す。**
    return respond({ character: "?", memories: [], unreadable: [] });
  });
}

/** 問い合わせを外したパス。 */
function pathOf(url: string): string {
  return new URL(url, "http://127.0.0.1").pathname;
}

function idsShown(): number[] {
  return [...target.querySelectorAll('tbody[data-section="active"] tr')].map(
    (row) => Number((row as HTMLElement).dataset["memoryId"]),
  );
}

function heading(): string {
  return target.querySelector("h1")?.textContent?.trim() ?? "";
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

it("`#/memories/{識別子}` は、その識別子の記憶を読む", async () => {
  window.location.hash = `#/memories/${MIO}`;
  const view = mount(App, { target });

  await vi.waitFor(() => {
    expect(idsShown()).toEqual([MIO_MEMORY.id]);
  });
  expect(urls.map(pathOf)).toContain(`/admin/characters/${MIO}/memories`);
  // **対照: 別の識別子は1度も引いていない。**
  expect(urls.join(" ")).not.toContain(NAR);
  expect(urls.join(" ")).not.toContain("aoi");
  expect(heading()).toContain(MIO);
  expect(heading()).not.toContain(NAR);
  expect(target.textContent).toContain(MIO_MEMORY.content);
  expect(target.textContent).not.toContain(NAR_MEMORY.content);

  await unmount(view, { outro: false });
});

/**
 * **識別子が変わったら、中身も変わる。**
 *
 * これが無いと、**見出しだけが新しいキャラクターを名乗り、
 * 表には前のキャラクターの記憶が残る**（`MemoryList` は
 * 作り直されなければ読み直さない）。**その状態で「訂正版を足す」を押すと、
 * B の画面から A の記憶へ `POST` する**ことになる。
 */
it("別のキャラクターへ移ると、そのキャラクターの記憶に入れ替わる", async () => {
  window.location.hash = `#/memories/${MIO}`;
  const view = mount(App, { target });
  await vi.waitFor(() => {
    expect(idsShown()).toEqual([MIO_MEMORY.id]);
  });

  window.location.hash = `#/memories/${NAR}`;

  await vi.waitFor(() => {
    expect(heading()).toContain(NAR);
  });
  // **読み直している。**
  await vi.waitFor(() => {
    expect(urls.map(pathOf)).toContain(`/admin/characters/${NAR}/memories`);
  });
  // **前のキャラクターの記憶が残っていない。**
  await vi.waitFor(() => {
    expect(idsShown()).toEqual([NAR_MEMORY.id]);
  });
  expect(target.textContent).toContain(NAR_MEMORY.content);
  expect(target.textContent).not.toContain(MIO_MEMORY.content);
  expect(heading()).not.toContain(MIO);

  await unmount(view, { outro: false });
});
