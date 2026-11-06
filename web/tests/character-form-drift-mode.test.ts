// @vitest-environment jsdom
/**
 * **作成・編集の画面の「変容の走らせ方」**（作業単位 D-15 / `src/lib/CharacterForm.svelte`）。
 *
 * # ここが見るもの（部品の被覆）
 *
 * | 何 | なぜ |
 * |---|---|
 * | **選んだものが、送る本文にそのまま載る**（真ん中の「記録だけ」を選ぶ） | 端を選ぶと「常に先頭／末尾」に決め打つ実装が通る |
 * | **触らなかったら本文に `stat_drift_mode` のキーが無い** | `null`（未設定に戻す）と「変えない」を同じ表現にしない |
 * | **「既定に従う」の括弧は、既定の取得口が返した値から** | キャラクターの値（`on`）と違う既定（`off`）で回す |
 * | **既定の取得口は、編集している行の識別子で呼ばれる** | 帰属（どれのものか）を取り違えない |
 * | **新規作成では既定を引かず、括弧を出さない** | キャラクターがまだ無い |
 *
 * **ページの配線（一覧が取得口を渡す所）は別に見る** ——
 * `tests/character-list-drift-mode.test.ts`（CLAUDE.md「配線だけの場所は部品の被覆とは別に見る」）。
 *
 * **ブラウザは起動しない**（jsdom だけ）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CharacterForm from "../src/lib/CharacterForm.svelte";
import type { CharacterView } from "../src/lib/api";

/**
 * 編集の対象。**キャラクターの値は `on`**（既定に従っていない）。
 *
 * **取得口が返す既定（`off`）とわざと違う値にしてある** ——
 * 同じだと、「既定に従う」の括弧にキャラクターの値を書く取り違えが見えない。
 */
const MIO: CharacterView = {
  id: 2,
  handle: "mio",
  display_name: "澪",
  base_persona: "早口で話す。",
  output_format_instruction: null,
  stat_scale: 5,
  stat_drift_mode: "on",
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:00:00Z",
};

/** 新規作成で作られる行（作成の応答）。 */
const RIN: CharacterView = {
  id: 9,
  handle: "rin",
  display_name: "凛",
  base_persona: "低い声で話す。",
  output_format_instruction: null,
  stat_scale: 10,
  stat_drift_mode: "off",
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T01:00:00Z",
};

interface Call {
  method: string;
  url: string;
  body: unknown;
}

let calls: Call[];
let target: HTMLElement;
/** 既定の取得口が呼ばれた識別子。 */
let asked: string[];

function stubFetch(respondWith: () => unknown): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const raw = init?.body;
    calls.push({
      method: init?.method ?? "GET",
      url: String(input),
      body: typeof raw === "string" ? (JSON.parse(raw) as unknown) : null,
    });
    return Promise.resolve({
      ok: true,
      status: 200,
      text: () => Promise.resolve(JSON.stringify(respondWith())),
    } as unknown as Response);
  });
}

/** 既定の取得口。**識別子を記録して、決まった既定を返す。** */
function loaderReturning(mode: string): (handle: string) => Promise<string> {
  return (handle: string) => {
    asked.push(handle);
    return Promise.resolve(mode);
  };
}

/** 走らせ方の選択欄。 */
function select(): HTMLSelectElement {
  const found = target.querySelector('select[name="stat_drift_mode"]');
  expect(found, "走らせ方の選択欄があること").not.toBeNull();
  return found as HTMLSelectElement;
}

/** 選択肢の表示（**並び順のまま**）。 */
function optionLabels(): string[] {
  return [...select().options].map((option) => option.textContent?.trim() ?? "");
}

/** 表示名で選ぶ（`bind:value` を起こすため `change` を投げる）。 */
function choose(labelPrefix: string): void {
  const option = [...select().options].find((candidate) =>
    candidate.textContent?.trim().startsWith(labelPrefix),
  );
  expect(option, `「${labelPrefix}」の選択肢があること`).toBeDefined();
  select().value = (option as HTMLOptionElement).value;
  select().dispatchEvent(new Event("change", { bubbles: true }));
  flushSync();
}

function type(name: string, value: string): void {
  const input = target.querySelector(`[name="${name}"]`) as HTMLInputElement;
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

function submit(): void {
  (target.querySelector("form") as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  flushSync();
}

/** 書き込みの要求（`GET` 以外）。 */
function writes(): Call[] {
  return calls.filter((call) => call.method !== "GET");
}

beforeEach(() => {
  calls = [];
  asked = [];
  target = document.createElement("div");
  document.body.append(target);
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("編集", () => {
  /**
   * **「既定に従う」の括弧は、取得口が返した既定を言う**（キャラクターの値ではない）。
   */
  it("「既定に従う」の括弧は、編集している行の既定を言う", async () => {
    stubFetch(() => ({ id: MIO.id, character: MIO }));
    const view = mount(CharacterForm, {
      target,
      props: { character: MIO, loadDefaultRunMode: loaderReturning("off") },
    });
    await vi.waitFor(() => {
      expect(optionLabels()[0]).toContain("いまは");
    });

    expect(optionLabels()).toEqual([
      "既定に従う（いまは: 停止中（判定も記録もしていません））",
      "止める",
      "記録だけ（値は動かさない）",
      "動かす",
    ]);
    // **帰属: 編集している行の識別子で1回だけ引く。**
    expect(asked).toEqual(["mio"]);
    // **下書きはキャラクターの値から始まる**（`on` ＝「動かす」）。
    expect(select().selectedOptions[0]?.textContent?.trim()).toBe("動かす");

    await unmount(view, { outro: false });
  });

  /** **真ん中を選ぶ。** 送る本文は、その選択だけ。 */
  it("選んだ走らせ方だけを送る（真ん中の「記録だけ」）", async () => {
    stubFetch(() => ({ id: MIO.id, character: { ...MIO, stat_drift_mode: "shadow" } }));
    const view = mount(CharacterForm, {
      target,
      props: { character: MIO, loadDefaultRunMode: loaderReturning("off") },
    });

    choose("記録だけ");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });

    expect(writes()).toEqual([
      {
        method: "PATCH",
        url: "/admin/characters/mio",
        body: { stat_drift_mode: "shadow" },
      },
    ]);
    // **保存の後も、同じ行の既定を引き直さない**（鍵は識別子だけ）。
    await vi.waitFor(() => {
      expect(target.textContent).toContain("更新しました");
    });
    expect(asked).toEqual(["mio"]);

    await unmount(view, { outro: false });
  });

  /** **「既定に従う」は `null`**（未設定に戻す）。 */
  it("「既定に従う」を選ぶと `null` を送る", async () => {
    stubFetch(() => ({ id: MIO.id, character: { ...MIO, stat_drift_mode: null } }));
    const view = mount(CharacterForm, {
      target,
      props: { character: MIO, loadDefaultRunMode: loaderReturning("off") },
    });

    choose("既定に従う");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });
    expect(writes()[0]?.body).toEqual({ stat_drift_mode: null });

    await unmount(view, { outro: false });
  });

  /**
   * **触らずに他の欄だけ直したら、走らせ方のキーは本文に無い**（`null` とは別）。
   */
  it("選択を触らずに表示名だけ直すと、走らせ方のキーは無い", async () => {
    stubFetch(() => ({ id: MIO.id, character: { ...MIO, display_name: "碧" } }));
    const view = mount(CharacterForm, {
      target,
      props: { character: MIO, loadDefaultRunMode: loaderReturning("off") },
    });

    type("display_name", "碧");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });

    const body = writes()[0]?.body as Record<string, unknown>;
    expect(body).toEqual({ display_name: "碧" });
    expect("stat_drift_mode" in body).toBe(false);

    await unmount(view, { outro: false });
  });

  /**
   * **サーバの知らない綴りの行を開くと、その綴りが選択に出て、
   * 触らずに他の欄だけ直しても走らせ方のキーは無い**（作業単位 D-15 のレビュー）。
   *
   * **この分岐が無いと、選択欄が4つのどれにも当たらず、
   * 触っていないのに別の値を送る形になりうる**（`CharacterForm.svelte` の注記）。
   */
  it("知らない綴りはそのまま選択に出て、触らなければ送らない", async () => {
    const future: CharacterView = { ...MIO, stat_drift_mode: "holiday" };
    stubFetch(() => ({ id: MIO.id, character: { ...future, display_name: "碧" } }));
    const view = mount(CharacterForm, {
      target,
      props: { character: future, loadDefaultRunMode: loaderReturning("off") },
    });
    flushSync();

    // **5つ目に、知らない綴りがそのまま並び、選ばれている。**
    expect(optionLabels().slice(1)).toEqual([
      "止める",
      "記録だけ（値は動かさない）",
      "動かす",
      "holiday",
    ]);
    expect(select().value).toBe("holiday");
    expect(select().selectedOptions[0]?.textContent?.trim()).toBe("holiday");

    type("display_name", "碧");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });
    const body = writes()[0]?.body as Record<string, unknown>;
    expect(body).toEqual({ display_name: "碧" });
    expect("stat_drift_mode" in body).toBe(false);

    await unmount(view, { outro: false });
  });

  /** **既定が読めなかったら、括弧を出さない**（推測で「停止」と書かない）。 */
  it("既定が読めなければ、括弧を出さない", async () => {
    stubFetch(() => ({}));
    const failing = (handle: string): Promise<string> => {
      asked.push(handle);
      return Promise.reject(new Error("読めない"));
    };
    const view = mount(CharacterForm, {
      target,
      props: { character: MIO, loadDefaultRunMode: failing },
    });
    await vi.waitFor(() => {
      expect(asked).toEqual(["mio"]);
    });
    flushSync();
    expect(optionLabels()[0]).toBe("既定に従う");

    await unmount(view, { outro: false });
  });
});

describe("新規作成", () => {
  /**
   * **キャラクターがまだ無いので、既定は引かない。括弧も出さない。**
   * **作成した後は、その行の既定を引く**（続けて編集できる / `CharacterForm` の `saved`）。
   */
  it("作成するまで既定を引かず、作成した行の既定を引く", async () => {
    stubFetch(() => RIN);
    const view = mount(CharacterForm, {
      target,
      props: { character: null, loadDefaultRunMode: loaderReturning("shadow") },
    });
    flushSync();

    expect(asked).toEqual([]);
    expect(optionLabels()[0]).toBe("既定に従う");

    type("handle", "rin");
    type("base_persona", "低い声で話す。");
    choose("止める");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });
    expect(writes()[0]).toEqual({
      method: "POST",
      url: "/admin/characters",
      body: { handle: "rin", base_persona: "低い声で話す。", stat_drift_mode: "off" },
    });

    // **作成した行（`rin`）の既定を引く。**
    await vi.waitFor(() => {
      expect(asked).toEqual(["rin"]);
    });
    await vi.waitFor(() => {
      expect(optionLabels()[0]).toBe(
        "既定に従う（いまは: 記録のみ（票は残りますが、値は動きません））",
      );
    });

    await unmount(view, { outro: false });
  });

  /** **「既定に従う」のまま作ると、キーごと省く。** */
  it("既定に従うまま作ると、走らせ方のキーは無い", async () => {
    stubFetch(() => RIN);
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("handle", "rin");
    type("base_persona", "低い声で話す。");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });
    const body = writes()[0]?.body as Record<string, unknown>;
    expect("stat_drift_mode" in body).toBe(false);

    await unmount(view, { outro: false });
  });
});
