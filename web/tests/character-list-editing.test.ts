// @vitest-environment jsdom
/**
 * **一覧から作成・編集を開く導線**（作業単位 D-2 / `src/lib/CharacterList.svelte`）。
 *
 * # `character-form.test.ts` との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | フォーム単体の送り先・上限・エラー | `character-form.test.ts` |
 * | **一覧のどの行を押したら、どのキャラクターのフォームが開くか** | **ここ** |
 *
 * # なぜ「開き直す」を見るのか
 *
 * **打ちかけの下書きは、親の再描画では消えない**（`CharacterForm` の `draft` の doc）。
 * その代わり、**別の行を開いたときに作り直さないと、前の行の値が残ったまま
 * 別のキャラクターへ `PATCH` される。**
 * **これは「載っているか」ではなく「どれのものか」の壊れ方**で、
 * 画面は正しい見出しを出したまま中身だけが他人のものになる。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CharacterList from "../src/lib/CharacterList.svelte";
import type { CharacterView } from "../src/lib/api";

/** 1件目。**`handle` と `display_name` は別の値。** */
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

/**
 * 2件目。**1件目と全部の欄が違う。**
 *
 * # ⚠ **これは「真ん中」でなければならない**
 *
 * 選ぶ操作のテストは、**選ばれる対象が端にあると「常に端」の変異を通す**
 * （D-2 のレビューで実測: 「常に先頭行」は3赤だが、
 * **「常に最終行」は本命を素通り**した —— 当時 MIO が最終行だったため）。
 * **3件にして真ん中を押せば、1本で両向きが赤になる。**
 */
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

/** 3件目。**MIO を端に置かないために居る**（上記）。 */
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

interface Call {
  method: string;
  url: string;
  body: unknown;
  /** **ヘッダも記録する。** 記録しない欄は観測できない（＝守れない）。 */
  headers: Record<string, string>;
}

let calls: Call[];
let target: HTMLElement;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const raw = init?.body;
    calls.push({
      method: init?.method ?? "GET",
      url,
      body: typeof raw === "string" ? (JSON.parse(raw) as unknown) : null,
      headers: { ...((init?.headers ?? {}) as Record<string, string>) },
    });

    const respond = (body: unknown, status = 200): Promise<Response> =>
      Promise.resolve({
        ok: status >= 200 && status < 300,
        status,
        text: () => Promise.resolve(JSON.stringify(body)),
      } as unknown as Response);

    if (url === "/admin/characters" && (init?.method ?? "GET") === "GET") {
      return respond({ characters: [AOI, MIO, NAR], unreadable: [] });
    }
    if (url.endsWith("/stats")) {
      return respond({ character: "?", stat_scale: 5, stats: [] });
    }
    // 更新（`PATCH`）。**読み直せた姿を返す。**
    return respond({ id: MIO.id, character: MIO });
  });
}

/** その行の「編集」ボタン。**行から引く**（全体から引くと1件目に当たり続ける）。 */
function editButtonOf(character: CharacterView): HTMLButtonElement {
  const row = [...target.querySelectorAll("tbody tr")].find((tr) =>
    tr.querySelector("code")?.textContent?.trim() === character.handle,
  );
  expect(row, `${character.handle} の行があること`).toBeDefined();
  const found = [...(row as HTMLElement).querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === "編集",
  );
  expect(found, `${character.handle} の行に「編集」があること`).toBeDefined();
  return found as HTMLButtonElement;
}

/** 開いているフォーム。 */
function form(): HTMLFormElement {
  const found = target.querySelector("form");
  expect(found, "フォームが開いていること").not.toBeNull();
  return found as HTMLFormElement;
}

/** フォームの欄。 */
function field(name: string): HTMLInputElement | HTMLTextAreaElement {
  const found = form().querySelector(`[name="${name}"]`);
  expect(found, `${name} の欄があること`).not.toBeNull();
  return found as HTMLInputElement | HTMLTextAreaElement;
}

/** 全体からその綴りのボタンを引く（一覧の「新規作成」用）。 */
function buttonLabelled(label: string): HTMLButtonElement {
  const found = [...target.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `「${label}」ボタンがあること`).toBeDefined();
  return found as HTMLButtonElement;
}

/** 一覧が3件出るまで待つ（**真ん中を押すために3件要る**。MIO の doc）。 */
async function waitForRows(): Promise<void> {
  await vi.waitFor(() => {
    expect(target.querySelectorAll("tbody tr").length).toBeGreaterThanOrEqual(
      3,
    );
  });
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

describe("一覧から編集を開く", () => {
  it("押した行のキャラクターが開く", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();

    editButtonOf(MIO).click();
    flushSync();

    expect(form().textContent).toContain(MIO.handle);
    expect(form().textContent).toContain(MIO.display_name);
    // **対照: 前の行の値も、後ろの行の値も入っていない。**
    // **両側を見る**——片側だけだと「常に先頭」か「常に末尾」の片方を通す。
    expect(form().textContent).not.toContain(AOI.handle);
    expect(form().textContent).not.toContain(NAR.handle);
    expect(field("base_persona").value).toBe(MIO.base_persona);
    expect(field("base_persona").value).not.toBe(AOI.base_persona);
    expect(field("base_persona").value).not.toBe(NAR.base_persona);

    await unmount(view, { outro: false });
  });

  /**
   * **開き直すと作り直される。**
   *
   * これが無いと、1件目を開いた後に2件目を開いても**1件目の下書きが残り**、
   * **2件目へ1件目のペルソナを書き込む**（しかも見出しは2件目を名乗る）。
   */
  it("別の行を開くと、前の行の下書きが残らない", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();

    editButtonOf(AOI).click();
    flushSync();
    expect(field("base_persona").value).toBe(AOI.base_persona);
    // 1件目で打ちかけてから、2件目へ移る。
    const typing = field("base_persona");
    typing.value = "打ちかけの本文";
    typing.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();

    editButtonOf(MIO).click();
    flushSync();

    expect(form().textContent).toContain(MIO.display_name);
    expect(field("base_persona").value).toBe(MIO.base_persona);
    expect(field("base_persona").value).not.toBe("打ちかけの本文");

    await unmount(view, { outro: false });
  });

  it("保存は、開いた行の識別子へ送られる", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();

    editButtonOf(MIO).click();
    flushSync();

    const input = field("display_name");
    input.value = "澪衣";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    form().dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));

    const patch = await vi.waitFor(() => {
      const found = calls.find((call) => call.method === "PATCH");
      expect(found).toBeDefined();
      return found as Call;
    });
    expect(patch.url).toBe(`/admin/characters/${MIO.handle}`);
    // **両隣のどちらへも流れていない。**
    expect(patch.url).not.toContain(AOI.handle);
    expect(patch.url).not.toContain(NAR.handle);
    expect(patch.body).toEqual({ display_name: "澪衣" });

    await unmount(view, { outro: false });
  });

  it("保存が通ったら一覧を読み直す", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();
    const before = calls.filter((call) => call.url === "/admin/characters")
      .length;

    editButtonOf(MIO).click();
    flushSync();
    const input = field("display_name");
    input.value = "澪衣";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    form().dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));

    await vi.waitFor(() => {
      const after = calls.filter(
        (call) => call.url === "/admin/characters" && call.method === "GET",
      ).length;
      expect(after).toBe(before + 1);
    });

    await unmount(view, { outro: false });
  });
});

describe("一覧から作成を開く", () => {
  it("識別子の欄がある新しいフォームが開く（既存の行の値は入らない）", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();

    // 開くまではフォームが無い（常に出ているわけではない）。
    expect(target.querySelector("form")).toBeNull();

    buttonLabelled("新規作成").click();
    flushSync();

    expect(field("handle").value).toBe("");
    expect(field("base_persona").value).toBe("");
    expect(form().textContent).not.toContain(MIO.handle);
    expect(form().textContent).not.toContain(NAR.handle);

    await unmount(view, { outro: false });
  });

  it("閉じられる", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();

    buttonLabelled("新規作成").click();
    flushSync();
    expect(target.querySelector("form")).not.toBeNull();

    buttonLabelled("閉じる").click();
    flushSync();
    expect(target.querySelector("form")).toBeNull();

    await unmount(view, { outro: false });
  });
});

describe("送り方", () => {
  /**
   * **本文の無い回は `content-type` を名乗らない**（`api.ts` の `sendJson` の doc）。
   *
   * 中身が無いのに「JSON を送った」と名乗ることになる。
   * **本文がある側は `character-form.test.ts` が見る**——
   * **両側を見て初めて「出し分け」が観測できる**（片側だけなら
   * 「常に付ける」「常に付けない」のどちらかが通り抜ける）。
   */
  it("取得の要求は `content-type` を名乗らない", async () => {
    const view = mount(CharacterList, { target });
    await waitForRows();

    const listing = calls.find(
      (call) => call.url === "/admin/characters" && call.method === "GET",
    );
    expect(listing).toBeDefined();
    expect((listing as Call).body).toBeNull();
    expect((listing as Call).headers["content-type"]).toBeUndefined();
    // 対照: **`accept` は付く**（ヘッダを1つも渡していないだけ、と区別する）。
    expect((listing as Call).headers["accept"]).toBe("application/json");

    await unmount(view, { outro: false });
  });
});
