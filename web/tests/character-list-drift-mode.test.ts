// @vitest-environment jsdom
/**
 * **ページの配線 —— 一覧から開いた編集の「変容の走らせ方」**
 * （作業単位 D-15 / `src/lib/CharacterList.svelte`）。
 *
 * # なぜ部品のテスト（`character-form-drift-mode.test.ts`）とは別に要るのか
 *
 * 部品は**既定の取得口を受け取るだけ**で、自分では `GET` しない。
 * **本物の口（`getDrift` の `default_run_mode`）を渡しているのは一覧だけ**なので、
 * 渡し忘れ・別の欄（`run_mode`）を渡す・別の行の識別子で引く、のどれも
 * 部品のテストは緑のまま通す（CLAUDE.md「配線だけの場所は部品の被覆とは別に見る」）。
 *
 * # フィクスチャ
 *
 * **起動時の既定（`default_run_mode`）は3行とも違う**（aoi: `on` / mio: `shadow` / nar: `off`）
 * ので、別の行の既定を言う取り違えは表示が変わる。
 * **開く行（`mio`）では、効いている値（`run_mode` = `on`、キャラクターの値も `on`）と
 * 既定（`shadow`）が違う**ので、既定の代わりに効いている値を渡す取り違えも表示が変わる。
 * （aoi と nar は、効いている値と既定が同じ —— **3つの値が全部の行で互いに違うわけではない**。）
 * **開くのは真ん中の行（`mio`）**（CLAUDE.md「選ぶ対象を端に置かない」）。
 *
 * **ブラウザは起動しない**（jsdom だけ）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import CharacterList from "../src/lib/CharacterList.svelte";
import type { CharacterView, DriftView } from "../src/lib/api";

function character(handle: string, id: number, mode: string | null): CharacterView {
  return {
    id,
    handle,
    display_name: `${handle}の呼び名`,
    base_persona: `${handle}のペルソナ。`,
    output_format_instruction: null,
    stat_scale: 5,
    stat_drift_mode: mode,
    axis_discovery_slots: 0,
    archived_at: null,
    created_at: "2026-08-27T00:00:00Z",
  };
}

const AOI = character("aoi", 1, null);
/** **開く行。** キャラクターの値は `on`、既定は `shadow`（別の値）。 */
const MIO = character("mio", 2, "on");
const NAR = character("nar", 3, "off");

/** 識別子ごとの（効いている値, 起動時の既定）。**3行とも既定が違う。** */
const DRIFT_OF: Record<string, [runMode: string, defaultRunMode: string]> = {
  aoi: ["on", "on"],
  mio: ["on", "shadow"],
  nar: ["off", "off"],
};

interface Call {
  method: string;
  url: string;
  body: unknown;
}

let calls: Call[];
let target: HTMLElement;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const raw = init?.body;
    calls.push({
      method,
      url,
      body: typeof raw === "string" ? (JSON.parse(raw) as unknown) : null,
    });
    const respond = (body: unknown): Promise<Response> =>
      Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(JSON.stringify(body)),
      } as unknown as Response);

    if (url === "/admin/characters" && method === "GET") {
      return respond({ characters: [AOI, MIO, NAR], unreadable: [] });
    }
    const drift = /^\/admin\/characters\/([^/]+)\/drift$/.exec(url);
    if (drift !== null) {
      const handle = drift[1] as string;
      const [runMode, defaultRunMode] = DRIFT_OF[handle] ?? ["?", "?"];
      const view: DriftView = {
        character: handle,
        stat_scale: 5,
        axes: [],
        total_votes: 0,
        recent_limit: 50,
        recent_votes: [],
        run_mode: runMode,
        run_mode_source: "character",
        default_run_mode: defaultRunMode,
      };
      return respond(view);
    }
    if (url.endsWith("/stats")) {
      return respond({ character: "?", stat_scale: 5, stats: [] });
    }
    // 更新（`PATCH`）。**読み直せた姿を返す。**
    return respond({ id: MIO.id, character: { ...MIO, stat_drift_mode: "shadow" } });
  });
}

/** その行の「編集」。**行から引く**（全体から引くと1件目に当たり続ける）。 */
function editButtonOf(row: CharacterView): HTMLButtonElement {
  const tr = [...target.querySelectorAll("tbody tr")].find(
    (candidate) => candidate.querySelector("code")?.textContent?.trim() === row.handle,
  );
  expect(tr, `${row.handle} の行があること`).toBeDefined();
  const found = [...(tr as HTMLElement).querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === "編集",
  );
  expect(found).toBeDefined();
  return found as HTMLButtonElement;
}

function select(): HTMLSelectElement {
  const found = target.querySelector('select[name="stat_drift_mode"]');
  expect(found, "走らせ方の選択欄があること").not.toBeNull();
  return found as HTMLSelectElement;
}

beforeEach(() => {
  calls = [];
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

/**
 * **一覧から開いた編集が、その行の起動時の既定を「既定に従う」の括弧に言い、
 * 選んだ値をその行へ送る。**
 */
it("一覧から開いた編集は、その行の既定を言い、選んだ値をその行へ送る", async () => {
  const view = mount(CharacterList, { target });
  await vi.waitFor(() => {
    expect(target.querySelectorAll("tbody tr").length).toBe(3);
  });

  editButtonOf(MIO).click();
  flushSync();

  // **その行（`mio`）の既定（`shadow`）。** 効いている値（`on`）でも、他の行の既定でもない。
  await vi.waitFor(() => {
    expect(select().options[0]?.textContent?.trim()).toBe(
      "既定に従う（いまは: 記録のみ（票は残りますが、値は動きません））",
    );
  });
  expect(
    calls.filter((call) => call.url.endsWith("/drift")).map((call) => call.url),
  ).toEqual(["/admin/characters/mio/drift"]);

  // **真ん中の選択（「記録だけ」）を選んで保存する。**
  select().value = "shadow";
  select().dispatchEvent(new Event("change", { bubbles: true }));
  flushSync();
  (target.querySelector("form") as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  flushSync();

  await vi.waitFor(() => {
    expect(calls.filter((call) => call.method === "PATCH")).toHaveLength(1);
  });
  expect(calls.filter((call) => call.method === "PATCH")).toEqual([
    { method: "PATCH", url: "/admin/characters/mio", body: { stat_drift_mode: "shadow" } },
  ]);

  await unmount(view, { outro: false });
});
