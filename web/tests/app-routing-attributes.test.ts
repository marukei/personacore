// @vitest-environment jsdom
/**
 * **`#/attributes/{識別子}` の繋ぎ目**（作業単位 D-12a / `src/App.svelte`）。
 *
 * # なぜこの1本が要るのか
 *
 * **「配線だけの場所」は、層に関係なく観測対象から落ちる**（CLAUDE.md）。
 * **`App.svelte` を mount するのは `app-routing*.test.ts` だけ**で、
 * **D-3 のレビューでは、その配線の2行がどちらも無防備だと実測されている**
 * （`handle="aoi"` に決め打つ変異も `{#key "fixed"}` にする変異も 107 全緑）。
 *
 * # `{#key}` を外すと何が起きるか（**推論ではなく、ここで観測する**）
 *
 * `AttributesPanel.svelte` は初期化のときに1回だけ読む（`$effect` を持たない）。
 * したがって**作り直されなければ、識別子が変わっても読み直しが起きない** ——
 * **見出しだけが新しいキャラクターを名乗り、表には前のキャラクターの属性が残る。**
 * **その状態で「置き換える」を押すと、B の画面から A の属性へ `PUT` が飛ぶ**
 * ——**しかも `PUT` は丸ごと置き換え**なので、A の属性が B の属性で上書きされる。
 * **そして属性は削れない枠に毎ターン載る**ので、A の以後の全ターンが B の事実で喋る。
 *
 * # ⚠ 時間帯は `attributes-panel.test.ts` と別のものを名乗る
 *
 * あちらは `Pacific/Marquesas`、ここは **`Asia/Katmandu`（UTC+05:45）**。
 * **消費側が2本あって別々の時間帯を名乗るので、
 * どちらの綴りを画面に直書きしても、必ずもう一方が赤くなる**
 * （`stats-panel.test.ts` 冒頭の ⚠ の実測 —— `StatsPanel` は消費側が1本しか無く、
 * 直書きが素通りした）。
 * **だからここは、時刻の綴りを実際に1つ見る。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import App from "../src/App.svelte";
import type { AttributeRevisionsView, AttributesView } from "../src/lib/api";
import { noProposalsFor } from "./support/no-proposals";
import { readerInZone } from "./support/zone";

readerInZone("Asia/Katmandu");

/** 1人目。 */
const MIO = "mio";
/** 2人目。**1人目と1文字も重ならない綴り。** */
const NAR = "nar";

const MIO_ATTRIBUTES: AttributesView = {
  character: MIO,
  attributes: [{ subject: "character", key: "一人称", value: "僕" }],
};

const MIO_REVISIONS: AttributeRevisionsView = {
  character: MIO,
  revisions: [
    {
      id: 31,
      subject: "character",
      key: "一人称",
      before: "私",
      after: "僕",
      created_at: "2026-08-27T09:00:00Z",
    },
  ],
  total_revisions: 5,
  revisions_limit: 3,
};

const NAR_ATTRIBUTES: AttributesView = {
  character: NAR,
  attributes: [{ subject: "user", key: "住所", value: "京都" }],
};

const NAR_REVISIONS: AttributeRevisionsView = {
  character: NAR,
  revisions: [
    {
      id: 12,
      subject: "user",
      key: "住所",
      before: null,
      after: "京都",
      created_at: "2023-03-03T03:03:03.030Z",
    },
  ],
  // **`mio` とは別の総数と上限**（同値化を避ける）。
  total_revisions: 1,
  revisions_limit: 50,
};

/**
 * **画面に出る時刻の綴り**（`Asia/Katmandu` = UTC+05:45）。
 *
 * **⚠ 直書きである**（`timeLabel` から作ると、綴りを変える変異が期待値も動かす）。
 * **45 分の時差**なので、分の桁が動く。
 */
const MIO_REVISION_LABEL = "2026-08-27 14:45:00 (UTC+05:45)";

let urls: string[];
let target: HTMLElement;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    urls.push(`${init?.method ?? "GET"} ${url}`);
    // **約束・決まりの提案**（作業単位 A-12c-2 / 会話の画面が読む）。**この測定は提案を持たない**（`no-proposals.ts`）。
    const proposals = noProposalsFor(url, init);
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
    if (url === `/admin/characters/${MIO}/attributes`) {
      return respond(MIO_ATTRIBUTES);
    }
    if (url === `/admin/characters/${MIO}/attributes/revisions`) {
      return respond(MIO_REVISIONS);
    }
    if (url === `/admin/characters/${NAR}/attributes`) {
      return respond(NAR_ATTRIBUTES);
    }
    if (url === `/admin/characters/${NAR}/attributes/revisions`) {
      return respond(NAR_REVISIONS);
    }
    // 別の識別子（決め打ちの変異はここへ来る）。**空を返す。**
    return respond({
      character: "?",
      attributes: [],
      revisions: [],
      total_revisions: 0,
      revisions_limit: 0,
      characters: [],
      unreadable: [],
      stats: [],
      stat_scale: 1,
      axes: [],
      total_votes: 0,
      recent_limit: 50,
      recent_votes: [],
      run_mode: "off",
      memories: [],
    });
  });
}

/** 表に出ているキーの綴り（**行から引く**）。 */
function keysShown(): string[] {
  return [...target.querySelectorAll("tr[data-attribute-key]")].map(
    (row) => (row as HTMLElement).dataset["attributeKey"] ?? "",
  );
}

function heading(): string {
  return target.querySelector("h1")?.textContent?.trim() ?? "";
}

/** 変更の記録の行の id。 */
function revisionIds(): string[] {
  return [...target.querySelectorAll("tr[data-revision-id]")].map(
    (row) => (row as HTMLElement).dataset["revisionId"] ?? "",
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

it("`#/attributes/{識別子}` は、その識別子の属性と変更の記録を読む", async () => {
  window.location.hash = `#/attributes/${MIO}`;
  const view = mount(App, { target });

  await vi.waitFor(() => {
    expect(keysShown()).toEqual(["一人称"]);
  });
  expect(urls).toContain(`GET /admin/characters/${MIO}/attributes`);
  expect(urls).toContain(`GET /admin/characters/${MIO}/attributes/revisions`);
  // **対照: 別の識別子は1度も引いていない。**
  expect(urls.join(" ")).not.toContain(NAR);
  expect(urls.join(" ")).not.toContain("aoi");
  // **記憶でも性格値でもない**（同じ形の経路が3つある）。
  expect(urls.join(" ")).not.toContain("/memories");
  expect(urls.join(" ")).not.toContain("/stats");
  expect(heading()).toContain(MIO);
  expect(heading()).not.toContain(NAR);
  // **変更の記録が出ている**（この画面の存在理由の半分）。
  expect(revisionIds()).toEqual(["31"]);
  // **時刻は読み手の時間帯で出る**（画面が綴りを直書きしていない）。
  expect(
    (target.querySelector("tr[data-revision-id] td") as HTMLElement | null)
      ?.textContent,
  ).toBe(MIO_REVISION_LABEL);

  await unmount(view, { outro: false });
});

/**
 * **識別子が変わったら、中身も変わる。**
 *
 * これが無いと、**見出しだけが新しいキャラクターを名乗り、
 * 表には前のキャラクターの属性が残る**（`AttributesPanel` は作り直されなければ読み直さない）。
 */
it("別のキャラクターへ移ると、そのキャラクターの属性に入れ替わる", async () => {
  window.location.hash = `#/attributes/${MIO}`;
  const view = mount(App, { target });
  await vi.waitFor(() => {
    expect(keysShown()).toEqual(["一人称"]);
  });

  window.location.hash = `#/attributes/${NAR}`;

  await vi.waitFor(() => {
    expect(heading()).toContain(NAR);
  });
  await vi.waitFor(() => {
    expect(urls).toContain(`GET /admin/characters/${NAR}/attributes`);
  });
  // **前のキャラクターの属性が残っていない。**
  await vi.waitFor(() => {
    expect(keysShown()).toEqual(["住所"]);
  });
  await vi.waitFor(() => {
    expect(revisionIds()).toEqual(["12"]);
  });
  expect(heading()).not.toContain(MIO);

  await unmount(view, { outro: false });
});

/**
 * **性格値の画面と行き先が入れ替わっていない。**
 *
 * `#/stats/{識別子}` と `#/attributes/{識別子}` は**どちらも `#/{名前}/{識別子}`** で、
 * **どちらでも同じ画面を出す配線**は、片方だけを見ていると通り抜ける。
 */
it("`#/stats/{識別子}` は、属性の画面にならない", async () => {
  window.location.hash = `#/stats/${MIO}`;
  const view = mount(App, { target });

  await vi.waitFor(() => {
    expect(urls.join(" ")).toContain(`/admin/characters/${MIO}/stats`);
  });
  expect(urls.join(" ")).not.toContain("/attributes");
  expect(target.querySelector("tr[data-revision-id]")).toBeNull();

  await unmount(view, { outro: false });
});

/** **記憶の画面とも入れ替わっていない。** */
it("`#/memories/{識別子}` は、属性の画面にならない", async () => {
  window.location.hash = `#/memories/${MIO}`;
  const view = mount(App, { target });

  await vi.waitFor(() => {
    expect(
      urls.map((url) => new URL(url.split(" ")[1] as string, "http://127.0.0.1").pathname),
    ).toContain(`/admin/characters/${MIO}/memories`);
  });
  expect(urls.join(" ")).not.toContain("/attributes");
  expect(target.querySelector("tr[data-revision-id]")).toBeNull();

  await unmount(view, { outro: false });
});
