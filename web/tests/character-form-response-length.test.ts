// @vitest-environment jsdom
/**
 * **作成・編集の画面の「返答の長さの目安」**（作業単位 A-22a / `src/lib/CharacterForm.svelte`）。
 *
 * # ここが見るもの（部品の被覆）
 *
 * | 何 | なぜ |
 * |---|---|
 * | **下限と上限の欄が、保存済みの値から始まる**（下限 ≠ 上限） | 2つの欄の取り違え |
 * | **変えた組が `PATCH` の本文に載る** | 欄から本文までの配線 |
 * | **両方空にすると `null`、触らなければキー無し** | 「変えない」と「未設定に戻す」を同じ表現にしない |
 * | **片方だけなら送らず、案内を出す** | サーバは片方だけの本文を `invalid_body` にする |
 * | **作成でも組が載る** | `POST` にだけ口が無い、を作らない |
 * | **「しないこと」（縮んだ会話は戻さない・上限としては働かない）を欄の近くに書く** | ADR-0065 §決定4 |
 *
 * **ブラウザは起動しない**（jsdom だけ）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CharacterForm from "../src/lib/CharacterForm.svelte";
import type { CharacterView } from "../src/lib/api";
import { RESPONSE_LENGTH_HALF_MESSAGE } from "../src/lib/characterEdit";

/** 編集の対象。**目安は 200〜400**（下限と上限を違う値にしてある）。 */
const MIO: CharacterView = {
  id: 2,
  handle: "mio",
  display_name: "澪",
  base_persona: "早口で話す。",
  output_format_instruction: null,
  stat_scale: 5,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  response_length: { min: 200, max: 400 },
  archived_at: null,
  created_at: "2026-08-27T00:00:00Z",
};

interface Call {
  method: string;
  url: string;
  body: unknown;
}

let calls: Call[];
let target: HTMLElement;

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

function field(name: string): HTMLInputElement {
  const found = target.querySelector(`[name="${name}"]`);
  expect(found, `${name} の欄があること`).not.toBeNull();
  return found as HTMLInputElement;
}

function type(name: string, value: string): void {
  const input = field(name);
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

function writes(): Call[] {
  return calls.filter((call) => call.method !== "GET");
}

beforeEach(() => {
  calls = [];
  target = document.createElement("div");
  document.body.append(target);
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("編集", () => {
  it("下限と上限の欄は、保存済みの値から始まる", async () => {
    stubFetch(() => ({ id: MIO.id, character: MIO }));
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    expect([field("response_length_min").value, field("response_length_max").value]).toEqual([
      "200",
      "400",
    ]);

    await unmount(view, { outro: false });
  });

  it("上限だけ変えると、組で送る", async () => {
    stubFetch(() => ({
      id: MIO.id,
      character: { ...MIO, response_length: { min: 200, max: 800 } },
    }));
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    type("response_length_max", "800");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });
    expect(writes()).toEqual([
      {
        method: "PATCH",
        url: "/admin/characters/mio",
        body: { response_length: { min: 200, max: 800 } },
      },
    ]);

    await unmount(view, { outro: false });
  });

  it("両方空にすると `null`（未設定に戻す）を送る", async () => {
    stubFetch(() => ({ id: MIO.id, character: { ...MIO, response_length: null } }));
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    type("response_length_min", "");
    type("response_length_max", "");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });
    expect(writes()[0]?.body).toEqual({ response_length: null });

    await unmount(view, { outro: false });
  });

  it("目安を触らずに表示名だけ直すと、目安のキーは無い", async () => {
    stubFetch(() => ({ id: MIO.id, character: { ...MIO, display_name: "碧" } }));
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    type("display_name", "碧");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });
    expect(writes()[0]?.body).toEqual({ display_name: "碧" });

    await unmount(view, { outro: false });
  });

  it("片方だけなら送らず、両方入れるよう案内する", async () => {
    stubFetch(() => ({ id: MIO.id, character: MIO }));
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    type("response_length_min", "");
    const alert = target.querySelector('[role="alert"]');
    expect(alert?.textContent).toBe(RESPONSE_LENGTH_HALF_MESSAGE);
    const button = target.querySelector('button[type="submit"]') as HTMLButtonElement;
    expect(button.disabled, "保存を押させない").toBe(true);
    submit();
    expect(writes()).toEqual([]);

    await unmount(view, { outro: false });
  });

  /**
   * **ADR-0065 §決定4 の「しないこと」を欄の近くに書く**（この欄だけでは縮んだ会話は戻さない・上限としては働かない）。
   * **戻す向きに働く手（上流モデルの画面の、本文生成の「終わりのトークン」）も同じ場所で言う**（作業単位 A-22b のレビュー）。
   * 出典（ADR の番号）は画面に書かない（`tests/screen-wording.test.ts`）。
   */
  it("欄の近くに、この欄がしないことと、縮んだ会話を戻す手を書く", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    const hint = target.querySelector('[data-hint="response_length"]');
    expect(hint, "目安の案内があること").not.toBeNull();
    const text = (hint as HTMLElement).textContent ?? "";
    expect(text).toContain("両方空にすると未設定になります。");
    expect(text).toContain("この上限で返答が切られることはなく");
    expect(text).toContain("この欄だけでは、短くなってしまった会話は元に戻りません。");
    expect(text).toContain("上流モデルの画面で、本文生成の詳細に「終わりのトークン」の番号と値を書いておくと");
    expect(text).toContain("返答がこの欄の下限より短くなった会話で、返答を長くする向きに働きます。");
    // **取り消した案内を残さない**（目安を最初から入れた会話も縮んだので「新しい会話を始めて」は当てにならない）。
    expect(text).not.toContain("新しい会話を始めてください");
    expect(text).not.toMatch(/ADR/);

    await unmount(view, { outro: false });
  });
});

describe("作成", () => {
  it("両方入れると、作成の本文に組で載る", async () => {
    const created: CharacterView = {
      ...MIO,
      id: 9,
      handle: "rin",
      display_name: "rin",
      response_length: { min: 800, max: 1200 },
    };
    stubFetch(() => created);
    const view = mount(CharacterForm, { target, props: { character: null } });

    expect([field("response_length_min").value, field("response_length_max").value]).toEqual([
      "",
      "",
    ]);
    type("handle", "rin");
    type("base_persona", "早口で話す。");
    type("response_length_min", "800");
    type("response_length_max", "1200");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });
    expect(writes()).toEqual([
      {
        method: "POST",
        url: "/admin/characters",
        body: {
          handle: "rin",
          base_persona: "早口で話す。",
          response_length: { min: 800, max: 1200 },
        },
      },
    ]);

    await unmount(view, { outro: false });
  });
});
