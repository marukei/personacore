// @vitest-environment jsdom
/**
 * **案内が指す操作は、実在するか**（作業単位 D-4 ＋ B-15）。
 *
 * # B-15 —— **ヒントが、無い操作を案内していた**
 *
 * `CharacterForm.svelte` は編集のときに、こう書いていた:
 *
 * > 保存済みの値より小さい尺度には下げられません（**先に性格値を下げてください**）。
 *
 * **その「性格値を下げる」操作は、どこにも存在しなかった** ——
 * `PUT .../stats` に導線が無く、D-4 は未着手だった
 * （`docs/archive/06-roadmap-done.md` の B-15 / **D-7 が `characterDelete.ts` で潰したのと同じ穴**）。
 *
 * **D-4 が入った以上、その案内は本当に実行できるはずである。**
 * **「はず」を書かずに、実際に辿る**のがこのファイルの2本目。
 *
 * # ⚠ **判定を、文言の目視から外す**
 *
 * ロードマップは **「画面に実在する導線の一覧が、どこにも無い」**と書いている。
 * ここでは**案内が「…」で名指しした操作が、実際に描かれた操作の中にあること**
 * を機械的に見る。**無い操作を案内した瞬間に赤くなる。**
 *
 * # ⚠ jsdom はハッシュのリンクを辿らない（実測）
 *
 * `<a href="#/...">` を `click()` しても `location` も `hashchange` も動かない
 * （このワークツリーで測った）。**だから、描かれた `href` をそのまま
 * `location.hash` へ入れて辿る** —— 利用者が押すのと同じ値を使うので、
 * **リンクの行き先が壊れれば、ここも壊れる。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../src/App.svelte";
import CharacterList from "../src/lib/CharacterList.svelte";
import StatsPanel from "../src/lib/StatsPanel.svelte";
import type { CharacterView, DriftView, StatsView } from "../src/lib/api";
import {
  DUPLICATE_AXIS_CODE,
  NOT_FOUND_CODE,
  SCALE_CHANGED_CODE,
  VALUE_ABOVE_SCALE_CODE,
  describeReplaceFailure,
} from "../src/lib/statsEdit";
import { ApiRequestError } from "../src/lib/api";
import { noProposalsFor } from "./support/no-proposals";
import { readerInZone } from "./support/zone";

/**
 * **読み手の時間帯を名乗る**（作業単位 F-7d）。
 *
 * このファイルは**時刻の綴りを1つも見ていない**が、
 * **`StatsPanel` を（`App.svelte` 越しにも）開く**ので、
 * 画面は `browserZone()` で機械に訊く。
 * **名乗らないと、その読みはその場で落ちる**（`tests/support/ambient-zone.ts`）。
 *
 * **どの時間帯でもよい** —— ここが見ているのは尺度の経路であって時刻ではない。
 */
readerInZone("America/New_York");

const AOI: CharacterView = {
  id: 1,
  handle: "aoi",
  display_name: "藍生",
  base_persona: "静かな声で話す。",
  output_format_instruction: null,
  stat_scale: 5,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:00:00Z",
};

/** **真ん中の行**（端を押さない / D-2 のレビューの実測）。 */
const MIO: CharacterView = {
  id: 2,
  handle: "mio",
  display_name: "澪",
  base_persona: "早口で話す。",
  output_format_instruction: "台詞は「」で囲む。",
  stat_scale: 7,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:10:00Z",
};

const NAR: CharacterView = {
  id: 3,
  handle: "nar",
  display_name: "奈落",
  base_persona: "ゆっくり話す。",
  output_format_instruction: null,
  stat_scale: 3,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:20:00Z",
};

/**
 * `mio` のいまの値。**尺度 7 に対して 6 まで使っている** ——
 * つまり**この値を下げない限り、尺度は 6 未満にできない**
 * （`stat_scale_below_stored_value` / ADR-0015）。
 */
let mioStats: StatsView;

/**
 * ⚠ **割り当てを失った軸を1本入れてある**（作業単位 B-16）。
 *
 * その軸の案内（`unallocatedNotice`）も**同じ判定に通す** ——
 * `「」` で名指しした操作が、実際に描かれた操作の中にあること。
 * **入れないと案内が出ないので、その案内は一度も判定されない。**
 */
const MIO_DRIFT: DriftView = {
  character: MIO.handle,
  stat_scale: 7,
  axes: [
    {
      axis: "慎重さ",
      value: 6,
      charge: 1.75,
      charge_updated_at: "2026-08-26T01:02:03.400Z",
      ups: 3,
      downs: 1,
    },
    { axis: "勇気", value: 0, charge: 0, charge_updated_at: null, ups: 0, downs: 0 },
    {
      axis: "きえた軸",
      value: null,
      charge: 2.5,
      charge_updated_at: "2025-12-31T23:59:59.999Z",
      ups: 0,
      downs: 2,
    },
  ],
  total_votes: 137,
  recent_limit: 3,
  recent_votes: [
    {
      axis: "慎重さ",
      delta: "up",
      session_id: 41,
      created_at: "2026-08-27T09:00:00Z",
    },
  ],
  // **停止中にしない。** 停止中の案内は操作を名指ししないので、
  // この画面の「名指しした操作が実在するか」の判定に何も足さない
  // ——ここで見たいのは**捨て軸の案内**の側である。
  run_mode: "on",
  run_mode_source: "default",
  default_run_mode: "on",
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
    const body = typeof raw === "string" ? (JSON.parse(raw) as unknown) : null;
    calls.push({ method, url, body });
    // **約束・決まりの提案**（作業単位 A-12c-2 / 会話の画面が読む）。**この測定は提案を持たない**（`no-proposals.ts`）。
    const proposals = noProposalsFor(url, init);
    if (proposals !== null) {
      return proposals;
    }

    const respond = (payload: unknown, status = 200): Promise<Response> =>
      Promise.resolve({
        ok: status >= 200 && status < 300,
        status,
        text: () => Promise.resolve(JSON.stringify(payload)),
      } as unknown as Response);

    if (url === "/admin/characters" && method === "GET") {
      return respond({ characters: [AOI, MIO, NAR], unreadable: [] });
    }
    if (url === `/admin/characters/${MIO.handle}/stats`) {
      if (method === "PUT") {
        // **丸ごと置き換え。** 送られた本文が、そのまま次の「いまの値」になる。
        const sent = body as { stats: { axis: string; value: number }[] };
        mioStats = { ...mioStats, stats: sent.stats };
        return respond(mioStats);
      }
      return respond(mioStats);
    }
    if (url === `/admin/characters/${MIO.handle}/drift`) {
      return respond(MIO_DRIFT);
    }
    if (method === "PATCH") {
      return respond({ id: MIO.id, character: MIO });
    }
    // 他のキャラクターの性格値（一覧が行ごとに引く）。
    return respond({ character: "?", stat_scale: 5, stats: [] });
  });
}

/** 案内が「…」で名指ししている操作。 */
function quotedOperations(text: string): string[] {
  return [...text.matchAll(/「([^」]+)」/g)].map((found) => found[1] as string);
}

/** いま実際に描かれている操作の見出し。 */
function controlLabels(root: ParentNode): Set<string> {
  return new Set(
    [...root.querySelectorAll("button, a")].map(
      (element) => element.textContent?.trim() ?? "",
    ),
  );
}

beforeEach(() => {
  calls = [];
  mioStats = {
    character: MIO.handle,
    stat_scale: 7,
    stats: [
      { axis: "慎重さ", value: 6 },
      { axis: "勇気", value: 0 },
    ],
  };
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

describe("B-15: 尺度のヒントが指す操作", () => {
  /**
   * **ヒントは、同じ画面に実在する操作だけを指す。**
   *
   * 編集のフォームは一覧の中に開く（`CharacterList` の `{#key editingKey}`）ので、
   * **ヒントが名指しした操作は、その場に見えていなければならない。**
   */
  it("ヒントが名指しした操作は、その画面に実在する", async () => {
    const view = mount(CharacterList, { target });
    await vi.waitFor(() => {
      expect(target.querySelectorAll("tbody tr").length).toBeGreaterThanOrEqual(
        3,
      );
    });
    flushSync();

    // **真ん中の行**の編集を開く。
    const row = [...target.querySelectorAll("tbody tr")].find(
      (tr) => tr.querySelector("code")?.textContent?.trim() === MIO.handle,
    ) as HTMLElement;
    const edit = [...row.querySelectorAll("button")].find(
      (button) => button.textContent?.trim() === "編集",
    );
    expect(edit, "編集の入口があること").toBeDefined();
    (edit as HTMLButtonElement).click();
    flushSync();

    const hint = target.querySelector('[data-hint="stat_scale"]');
    expect(hint, "尺度のヒントがあること").not.toBeNull();
    const text = (hint as HTMLElement).textContent ?? "";

    const named = quotedOperations(text);
    // **名指ししていること自体を見る**（「」を外す変異を空振りで通さない）。
    expect(named.length).toBeGreaterThan(0);

    const labels = controlLabels(target);
    for (const operation of named) {
      expect(
        labels.has(operation),
        `ヒントの「${operation}」が実在すること`,
      ).toBe(true);
    }
    // **対照: 判定が何でも通るわけではない。**
    expect(labels.has("性格値をまとめて下げる")).toBe(false);

    await unmount(view, { outro: false });
  });

  /**
   * **無い操作の名前が残っていないこと。**
   *
   * B-15 の元の文言は「先に性格値を下げてください」だけで、
   * **どこで下げるのかを一言も言っていなかった。**
   * **場所を名指ししない案内は、実在するかどうかを誰も判定できない。**
   */
  it("ヒントは、下げる場所を名指しする", async () => {
    const view = mount(CharacterList, { target });
    await vi.waitFor(() => {
      expect(target.querySelectorAll("tbody tr").length).toBeGreaterThanOrEqual(
        3,
      );
    });
    const row = [...target.querySelectorAll("tbody tr")].find(
      (tr) => tr.querySelector("code")?.textContent?.trim() === MIO.handle,
    ) as HTMLElement;
    const edit = [...row.querySelectorAll("button")].find(
      (button) => button.textContent?.trim() === "編集",
    ) as HTMLButtonElement;
    edit.click();
    flushSync();

    const text =
      target.querySelector('[data-hint="stat_scale"]')?.textContent ?? "";
    expect(text).toContain("下げ");
    expect(quotedOperations(text)).toContain("性格値と変容");

    await unmount(view, { outro: false });
  });
});

describe("置き換えの案内が指す操作", () => {
  /**
   * **案内が名指しする操作は、この画面か、この画面から1手で行ける一覧にある。**
   *
   * 尺度を上げる操作はこの画面には無い（キャラクターの編集）ので、
   * **union は「性格値の画面 ＋ キャラクターの一覧」**である。
   * それより遠いものを指したら、**利用者は探し方を教わっていない。**
   */
  it("4つの案内が名指しした操作が、全部実在する", async () => {
    const panelTarget = document.createElement("div");
    document.body.append(panelTarget);
    const panel = mount(StatsPanel, {
      target: panelTarget,
      props: { handle: MIO.handle },
    });
    await vi.waitFor(() => {
      expect(panelTarget.querySelector("[data-vote-total]")).not.toBeNull();
    });

    const list = mount(CharacterList, { target });
    await vi.waitFor(() => {
      expect(target.querySelectorAll("tbody tr").length).toBeGreaterThanOrEqual(
        3,
      );
    });
    flushSync();

    const labels = new Set([
      ...controlLabels(panelTarget),
      ...controlLabels(target),
    ]);

    const failures = [
      VALUE_ABOVE_SCALE_CODE,
      SCALE_CHANGED_CODE,
      DUPLICATE_AXIS_CODE,
      NOT_FOUND_CODE,
    ].map((code) =>
      describeReplaceFailure(
        new ApiRequestError("何か", 400, "invalid_request_error", code),
      ),
    );

    for (const failure of failures) {
      expect(failure.guidance, `${failure.reason} に案内があること`).not.toBeNull();
      const named = quotedOperations(failure.guidance as string);
      expect(
        named.length,
        `${failure.reason} が操作を名指しすること`,
      ).toBeGreaterThan(0);
      for (const operation of named) {
        expect(
          labels.has(operation),
          `${failure.reason} の「${operation}」が実在すること`,
        ).toBe(true);
      }
    }
    // **対照: 何でも通るわけではない。**
    expect(labels.has("会話を削除")).toBe(false);

    await unmount(panel, { outro: false });
    await unmount(list, { outro: false });
    panelTarget.remove();
  });

  /**
   * **割り当てを失った軸の案内も、同じ判定に通す**（作業単位 B-16）。
   *
   * # なぜ別の本が要るのか
   *
   * 上の4つは `describeReplaceFailure`（**置き換えが拒否されたとき**）の案内で、
   * こちらは **`GET .../drift` が捨て軸を返したとき**に出る別の案内である。
   * **出る条件が違うので、上の判定には一度も掛からない。**
   *
   * # 直す手段は1つしかない
   *
   * 票にも累積にも**消す口がサーバに無い**（追記のみ / ADR-0026 §決定3）ので、
   * 直せるのは「**同じ綴りで置き直す**」だけ ——
   * この画面の **「軸を追加」** と **「この内容で置き換える」** で辿れる。
   */
  it("捨て軸の案内が名指しした操作も、全部実在する", async () => {
    const panelTarget = document.createElement("div");
    document.body.append(panelTarget);
    const panel = mount(StatsPanel, {
      target: panelTarget,
      props: { handle: MIO.handle },
    });
    await vi.waitFor(() => {
      expect(panelTarget.querySelector("[data-vote-total]")).not.toBeNull();
    });
    flushSync();

    const notice = panelTarget.querySelector("[data-unallocated-notice]");
    expect(notice, "捨て軸の案内が出ていること").not.toBeNull();
    const text = (notice as HTMLElement).textContent ?? "";

    const named = quotedOperations(text);
    expect(named.length, "操作を名指しすること").toBeGreaterThan(0);

    // **この画面だけで完結する**（一覧へ行かなくても直せる）。
    const labels = controlLabels(panelTarget);
    for (const operation of named) {
      expect(
        labels.has(operation),
        `捨て軸の案内の「${operation}」が実在すること`,
      ).toBe(true);
    }
    // **対照: 何でも通るわけではない。**
    expect(labels.has("票を削除")).toBe(false);

    await unmount(panel, { outro: false });
    panelTarget.remove();
  });
});

/**
 * **B-15 の案内を、実際に最後まで辿る。**
 *
 * ```text
 * キャラクターの一覧 → 性格値と変容 → 値を下げる → 一覧へ戻る → 編集 → 尺度を下げる
 * ```
 *
 * **一手でも欠けたら赤になる。** ここが緑であることが、
 * 「その案内は実行できる」という主張の根拠そのものである。
 */
it("一覧から性格値を下げ、一覧へ戻って尺度を下げるまで辿れる", async () => {
  window.location.hash = "#/characters";
  const view = mount(App, { target });

  // 1. 一覧が出る。
  await vi.waitFor(() => {
    expect(target.querySelectorAll("tbody tr").length).toBeGreaterThanOrEqual(3);
  });
  flushSync();

  // 2. **真ん中の行**の「性格値と変容」へ。
  const row = [...target.querySelectorAll("tbody tr")].find(
    (tr) => tr.querySelector("code")?.textContent?.trim() === MIO.handle,
  ) as HTMLElement;
  const link = [...row.querySelectorAll("a")].find(
    (anchor) => anchor.textContent?.trim() === "性格値と変容",
  );
  expect(link, "行に性格値への導線があること").toBeDefined();
  // jsdom はリンクを辿らないので、**描かれた行き先をそのまま使う**（冒頭の ⚠）。
  window.location.hash = (link as HTMLAnchorElement).getAttribute(
    "href",
  ) as string;

  await vi.waitFor(() => {
    expect(target.querySelector('tr[data-axis="慎重さ"]')).not.toBeNull();
  });
  flushSync();
  // **その行のキャラクターの画面である。**
  expect(target.querySelector("h1")?.textContent).toContain(MIO.handle);

  // 3. 値を 6 → 2 へ下げて保存する。
  const field = target
    .querySelector('tr[data-axis="慎重さ"]')
    ?.querySelector('input[name="value"]') as HTMLInputElement;
  expect(field.value).toBe("6");
  field.value = "2";
  field.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
  // **送る前の読みの回数を控える。** 一覧も行ごとに `stats` を引くので、
  // 「2回以上読んだ」では送る前から成立してしまう。
  const readsBefore = calls.filter(
    (call) =>
      call.method === "GET" &&
      call.url === `/admin/characters/${MIO.handle}/stats`,
  ).length;
  const form = target.querySelector("form") as HTMLFormElement;
  form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));

  await vi.waitFor(() => {
    expect(calls.some((call) => call.method === "PUT")).toBe(true);
  });
  // **送り先も見る** —— 別のキャラクターへ丸ごと置き換えを飛ばしていない。
  expect(calls.find((call) => call.method === "PUT")?.url).toBe(
    `/admin/characters/${MIO.handle}/stats`,
  );
  // **下がったことが、読み直した後の画面に出る**（送っただけで終わらない）。
  //
  // **⚠ 読み直しを待たずに欄を見ると、打った値がそのまま残っているだけ**で緑になる
  // （実測。`vi.waitFor` は最初に成立した時点で返る）。**読み直しの完了を先に待つ。**
  await vi.waitFor(() => {
    expect(
      calls.filter(
        (call) =>
          call.method === "GET" &&
          call.url === `/admin/characters/${MIO.handle}/stats`,
      ).length,
    ).toBeGreaterThan(readsBefore);
  });
  flushSync();
  const after = target
    .querySelector('tr[data-axis="慎重さ"]')
    ?.querySelector('input[name="value"]') as HTMLInputElement;
  expect(after.value).toBe("2");

  // 4. 一覧へ戻る導線（この画面に実在する）。
  const back = [...target.querySelectorAll("a")].find(
    (anchor) => anchor.textContent?.trim() === "キャラクターの一覧へ",
  );
  expect(back, "一覧へ戻る導線があること").toBeDefined();
  window.location.hash = (back as HTMLAnchorElement).getAttribute(
    "href",
  ) as string;

  // **一覧が出るまで待つ。** 行数では待てない ——
  // **性格値の画面にも `tbody tr` があるので、切り替わる前に条件が満たされる**
  // （実測。待ちの条件は、その画面にしか無いもので書くこと）。
  await vi.waitFor(() => {
    expect(target.querySelector("h1")?.textContent?.trim()).toBe(
      "キャラクター",
    );
  });
  await vi.waitFor(() => {
    expect(
      [...target.querySelectorAll("tbody tr")].some(
        (tr) => tr.querySelector("code")?.textContent?.trim() === MIO.handle,
      ),
    ).toBe(true);
  });
  flushSync();

  // 5. その行の編集を開いて、尺度を下げる。
  const backRow = [...target.querySelectorAll("tbody tr")].find(
    (tr) => tr.querySelector("code")?.textContent?.trim() === MIO.handle,
  ) as HTMLElement;
  const edit = [...backRow.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === "編集",
  ) as HTMLButtonElement;
  edit.click();
  flushSync();

  const scale = target.querySelector(
    '[name="stat_scale"]',
  ) as HTMLInputElement;
  expect(scale.value).toBe(String(MIO.stat_scale));
  scale.value = "3";
  scale.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
  const editForm = target.querySelector("form") as HTMLFormElement;
  editForm.dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );

  await vi.waitFor(() => {
    expect(calls.some((call) => call.method === "PATCH")).toBe(true);
  });
  const patch = calls.find((call) => call.method === "PATCH");
  expect(patch?.url).toBe(`/admin/characters/${MIO.handle}`);
  expect(patch?.body).toEqual({ stat_scale: 3 });

  // **順番も見る** —— 値を下げるのが先（そうでなければサーバが 409 で拒む）。
  const putAt = calls.findIndex((call) => call.method === "PUT");
  const patchAt = calls.findIndex((call) => call.method === "PATCH");
  expect(putAt).toBeGreaterThanOrEqual(0);
  expect(putAt).toBeLessThan(patchAt);

  await unmount(view, { outro: false });
});
