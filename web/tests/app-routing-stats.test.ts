// @vitest-environment jsdom
/**
 * **`#/stats/{識別子}` の繋ぎ目**（作業単位 D-4 / `src/App.svelte`）。
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
 * `StatsPanel.svelte` は初期化のときに1回だけ読む（`$effect` を持たない）。
 * したがって**作り直されなければ、識別子が変わっても読み直しが起きない** ——
 * **見出しだけが新しいキャラクターを名乗り、表には前のキャラクターの値が残る。**
 * **その状態で「置き換える」を押すと、B の画面から A の性格値へ `PUT` が飛ぶ**
 * ——**しかも `PUT` は丸ごと置き換え**なので、A の軸が B の軸で上書きされる。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import App from "../src/App.svelte";
import type { DriftView, StatsView } from "../src/lib/api";
import { noProposalsFor } from "./support/no-proposals";
import { readerInZone } from "./support/zone";

/**
 * **読み手の時間帯を名乗る**（作業単位 F-7d）。
 *
 * このファイルは**時刻の綴りを1つも見ていない**が、
 * **`App.svelte` 越しに `StatsPanel` を開く**ので、
 * 画面は `browserZone()` で機械に訊く。
 * **名乗らないと、その読みはその場で落ちる**（`tests/support/ambient-zone.ts`）。
 *
 * **どの時間帯でもよい** —— ここが見ているのは経路であって時刻ではない。
 */
readerInZone("America/New_York");

/** 1人目。 */
const MIO = "mio";
/** 2人目。**1人目と1文字も重ならない綴り。** */
const NAR = "nar";

const MIO_STATS: StatsView = {
  character: MIO,
  stat_scale: 7,
  stats: [{ axis: "勇気", value: 0 }],
};

const MIO_DRIFT: DriftView = {
  character: MIO,
  stat_scale: 7,
  axes: [
    {
      axis: "勇気",
      value: 0,
      charge: 1.75,
      charge_updated_at: "2026-08-26T01:02:03.400Z",
      ups: 3,
      downs: 1,
    },
  ],
  total_votes: 137,
  recent_limit: 3,
  recent_votes: [
    {
      axis: "勇気",
      delta: "up",
      session_id: 41,
      created_at: "2026-08-27T09:00:00Z",
    },
  ],
  run_mode: "on",
  run_mode_source: "default",
  default_run_mode: "on",
};

const NAR_STATS: StatsView = {
  character: NAR,
  stat_scale: 9,
  stats: [{ axis: "気だるさ", value: 8 }],
};

const NAR_DRIFT: DriftView = {
  character: NAR,
  stat_scale: 9,
  axes: [
    {
      axis: "気だるさ",
      value: 8,
      charge: 0.25,
      charge_updated_at: "2023-03-03T03:03:03.030Z",
      ups: 6,
      downs: 2,
    },
  ],
  total_votes: 4,
  recent_limit: 50,
  recent_votes: [
    {
      axis: "気だるさ",
      delta: "down",
      session_id: 77,
      created_at: "2026-08-27T05:00:00Z",
    },
  ],
  // **`mio` とは別の走らせ方**（識別子ごとに違うものを返す / 同値化を避ける）。
  run_mode: "shadow",
  run_mode_source: "default",
  default_run_mode: "shadow",
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
    if (url === `/admin/characters/${MIO}/stats`) {
      return respond(MIO_STATS);
    }
    if (url === `/admin/characters/${MIO}/drift`) {
      return respond(MIO_DRIFT);
    }
    if (url === `/admin/characters/${NAR}/stats`) {
      return respond(NAR_STATS);
    }
    if (url === `/admin/characters/${NAR}/drift`) {
      return respond(NAR_DRIFT);
    }
    // 別の識別子（決め打ちの変異はここへ来る）。**空を返す。**
    return respond({
      character: "?",
      stat_scale: 1,
      stats: [],
      axes: [],
      total_votes: 0,
      recent_limit: 50,
      recent_votes: [],
      run_mode: "off",
    });
  });
}

function axesShown(): string[] {
  return [...target.querySelectorAll("tr[data-drift-axis]")].map(
    (row) => (row as HTMLElement).dataset["driftAxis"] ?? "",
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

it("`#/stats/{識別子}` は、その識別子の性格値と変容を読む", async () => {
  window.location.hash = `#/stats/${MIO}`;
  const view = mount(App, { target });

  await vi.waitFor(() => {
    expect(axesShown()).toEqual(["勇気"]);
  });
  expect(urls).toContain(`/admin/characters/${MIO}/stats`);
  expect(urls).toContain(`/admin/characters/${MIO}/drift`);
  // **対照: 別の識別子は1度も引いていない。**
  expect(urls.join(" ")).not.toContain(NAR);
  expect(urls.join(" ")).not.toContain("aoi");
  // **記憶の画面ではない**（同じ形の経路が2つある）。
  expect(urls.join(" ")).not.toContain("/memories");
  expect(heading()).toContain(MIO);
  expect(heading()).not.toContain(NAR);
  // **総数が出ている**（この画面の存在理由 —— 変容が見えること）。
  expect(target.querySelector("[data-vote-total]")?.textContent).toContain(
    "137",
  );

  await unmount(view, { outro: false });
});

/**
 * **識別子が変わったら、中身も変わる。**
 *
 * これが無いと、**見出しだけが新しいキャラクターを名乗り、
 * 表には前のキャラクターの値が残る**（`StatsPanel` は作り直されなければ読み直さない）。
 */
it("別のキャラクターへ移ると、そのキャラクターの値に入れ替わる", async () => {
  window.location.hash = `#/stats/${MIO}`;
  const view = mount(App, { target });
  await vi.waitFor(() => {
    expect(axesShown()).toEqual(["勇気"]);
  });

  window.location.hash = `#/stats/${NAR}`;

  await vi.waitFor(() => {
    expect(heading()).toContain(NAR);
  });
  await vi.waitFor(() => {
    expect(urls).toContain(`/admin/characters/${NAR}/stats`);
  });
  // **前のキャラクターの軸が残っていない。**
  await vi.waitFor(() => {
    expect(axesShown()).toEqual(["気だるさ"]);
  });
  expect(
    (target.querySelector("[data-vote-total]") as HTMLElement | null)?.dataset[
      "voteTotal"
    ],
  ).toBe("4");
  expect(heading()).not.toContain(MIO);

  await unmount(view, { outro: false });
});

/**
 * **記憶の画面と行き先が入れ替わっていない。**
 *
 * `#/memories/{識別子}` と `#/stats/{識別子}` は綴りが1語しか違わず、
 * **どちらでも同じ画面を出す配線**は、片方だけを見ていると通り抜ける。
 */
it("`#/memories/{識別子}` は、性格値の画面にならない", async () => {
  window.location.hash = `#/memories/${MIO}`;
  const view = mount(App, { target });

  await vi.waitFor(() => {
    // **問い合わせは外して見る**（記憶の一覧は `?include=superseded` を付ける /
    // 作業単位 D-3c）。**ここが見ているのは行き先であって、問い合わせではない。**
    expect(
      urls.map((url) => new URL(url, "http://127.0.0.1").pathname),
    ).toContain(`/admin/characters/${MIO}/memories`);
  });
  expect(target.querySelector("[data-vote-total]")).toBeNull();
  expect(urls.join(" ")).not.toContain("/drift");

  await unmount(view, { outro: false });
});
