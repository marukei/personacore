// @vitest-environment jsdom
/**
 * **キャラクターの作成・編集の画面**（作業単位 D-2 / `src/lib/CharacterForm.svelte`）。
 *
 * # ここが見るもの
 *
 * | 何 | なぜ |
 * |---|---|
 * | **どのキャラクターを編集しているか**が、画面と送り先で一致する | このリポジトリが繰り返し落としているのは「載っているか」ではなく**「どれのものか」** |
 * | **作成の後、次の保存が `PATCH` になる**（`POST` を2回しない） | 送り先が切り替わったことを、画面の外から確かめる唯一の手段 |
 * | **上限を超えたら送らない**（未超過→超過→直す、の3相） | 「常に押せない」実装が第1相と第3相を満たして緑になるのを塞ぐ |
 * | **サーバが弾いた理由が画面に出る** | 本文は定数文字列なので、消すと利用者に何も残らない |
 *
 * **`DELETE` は入れていない**（ADR-0016 §2 の「会話が無いときだけ」の案内が別設計になる）。
 * **`stats` / `attributes` も入れていない**（`PUT` の従属資源）。**どちらも別の作業単位。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CharacterForm from "../src/lib/CharacterForm.svelte";
import type { CharacterView } from "../src/lib/api";
import { LIMITS } from "../src/lib/characterEdit";

/**
 * 編集の対象。**`handle` と `display_name` を別の値にしてある**
 * （同じにすると、片方をもう片方に取り違える変異が見えない）。
 */
const MIO: CharacterView = {
  id: 2,
  handle: "mio",
  display_name: "澪",
  base_persona: "早口で話す。",
  output_format_instruction: "台詞は「」で囲む。",
  stat_scale: 5,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:00:00Z",
};

/** **もう1件。** 送り先がこちらへ流れていないことの対照に使う。 */
const AOI_HANDLE = "aoi";

/** 1件ぶんの要求の記録。 */
interface Call {
  method: string;
  url: string;
  body: unknown;
  /** **ヘッダも記録する。** 記録しない欄は観測できない（＝守れない）。 */
  headers: Record<string, string>;
}

let calls: Call[];
let target: HTMLElement;
/** 次の応答（先頭から取り出す）。空なら 200 と `nextView` を返す。 */
let responses: { status: number; body: string }[];
/** 既定の応答本文に使うキャラクター。 */
let nextView: CharacterView;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const raw = init?.body;
    calls.push({
      method: init?.method ?? "GET",
      url: String(input),
      body: typeof raw === "string" ? (JSON.parse(raw) as unknown) : null,
      headers: { ...((init?.headers ?? {}) as Record<string, string>) },
    });

    const queued = responses.shift();
    const status = queued?.status ?? 200;
    const body = queued?.body ?? JSON.stringify(nextView);
    return Promise.resolve({
      ok: status >= 200 && status < 300,
      status,
      text: () => Promise.resolve(body),
    } as unknown as Response);
  });
}

/** 欄。**`name` は送る本文のキーと同じ綴り。** */
function field(name: string): HTMLInputElement | HTMLTextAreaElement {
  const found = target.querySelector(`[name="${name}"]`);
  expect(found, `${name} の欄があること`).not.toBeNull();
  return found as HTMLInputElement | HTMLTextAreaElement;
}

/** 欄に値を入れる（`bind:value` を起こすため `input` を投げる）。 */
function type(name: string, value: string): void {
  const input = field(name);
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

/** 見出しがその綴りのボタン。 */
function buttonLabelled(label: string): HTMLButtonElement {
  const found = [...target.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `「${label}」ボタンがあること`).toBeDefined();
  return found as HTMLButtonElement;
}

/**
 * その欄のカウンタ。**`data-counter` で引く。**
 *
 * **画面の文字列全体では見ない** —— どの欄のカウンタなのかが分からず、
 * **上限だけが別の欄の値になっている状態を見逃す。**
 */
function counter(field: string): HTMLElement {
  const found = target.querySelector(`[data-counter="${field}"]`);
  expect(found, `${field} のカウンタがあること`).not.toBeNull();
  return found as HTMLElement;
}

/** その綴りのボタンがあるか（無いことも見たい）。 */
function hasButton(label: string): boolean {
  return [...target.querySelectorAll("button")].some(
    (button) => button.textContent?.trim() === label,
  );
}

/** 送信する。 */
function submit(): void {
  const form = target.querySelector("form");
  expect(form).not.toBeNull();
  (form as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  flushSync();
}

beforeEach(() => {
  calls = [];
  responses = [];
  nextView = MIO;
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("編集", () => {
  it("どのキャラクターを編集しているかが、画面と送り先で一致する", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    // **画面がそのキャラクターを名乗る。**
    expect(target.textContent).toContain(MIO.handle);
    expect(target.textContent).toContain(MIO.display_name);
    // **対照: もう1件の識別子は出ていない。**
    expect(target.textContent).not.toContain(AOI_HANDLE);
    // 既存の値が入っている（空のフォームから始めない）。
    expect(field("base_persona").value).toBe(MIO.base_persona);

    type("display_name", "澪衣");
    submit();
    await vi.waitFor(() => {
      expect(calls.length).toBe(1);
    });

    const call = calls[0];
    expect(call?.method).toBe("PATCH");
    // **送り先が、画面が名乗ったキャラクター。**
    expect(call?.url).toBe(`/admin/characters/${MIO.handle}`);
    expect(call?.url).not.toContain(AOI_HANDLE);
    // **変えた欄だけ。**
    expect(call?.body).toEqual({ display_name: "澪衣" });

    await unmount(view, { outro: false });
  });

  it("識別子は変えられない（ADR-0016 §1）", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    // **入力欄そのものが無い。** あると「変えたのに反映されない」に見える。
    expect(target.querySelector('[name="handle"]')).toBeNull();

    type("display_name", "澪衣");
    submit();
    await vi.waitFor(() => {
      expect(calls.length).toBe(1);
    });
    // **本文にも載らない**（載せると `deny_unknown_fields` で1列も書かれない）。
    expect(calls[0]?.body).not.toHaveProperty("handle");

    await unmount(view, { outro: false });
  });

  it("変えていなければ保存を押せない（未変更→変更→保存後の3相）", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    // 第1相: **1つも変えていない。**
    expect(buttonLabelled("保存").disabled).toBe(true);

    // 第2相: **変えたら押せる。**
    // ここが無いと「常に押せない」実装が第1相と第3相を満たして緑になる。
    type("display_name", "澪衣");
    expect(buttonLabelled("保存").disabled).toBe(false);

    // 第3相: **保存が通れば、また押せなくなる**（同じ本文を二度送らない）。
    nextView = { ...MIO, display_name: "澪衣" };
    responses.push({
      status: 200,
      body: JSON.stringify({ id: MIO.id, character: nextView }),
    });
    submit();
    await vi.waitFor(() => {
      expect(buttonLabelled("保存").disabled).toBe(true);
    });
    expect(calls.length).toBe(1);

    await unmount(view, { outro: false });
  });

  it("形式指示を空にすると、消す指示が送られる", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    type("output_format_instruction", "");
    submit();
    await vi.waitFor(() => {
      expect(calls.length).toBe(1);
    });
    expect(calls[0]?.body).toEqual({ output_format_instruction: null });

    await unmount(view, { outro: false });
  });

  it("保存が通ったら、更新後の姿を親へ渡す", async () => {
    const saved: CharacterView[] = [];
    const updated = { ...MIO, display_name: "澪衣" };
    const view = mount(CharacterForm, {
      target,
      props: {
        character: MIO,
        onSaved: (character: CharacterView) => {
          saved.push(character);
        },
      },
    });

    type("display_name", "澪衣");
    responses.push({
      status: 200,
      body: JSON.stringify({ id: MIO.id, character: updated }),
    });
    submit();

    await vi.waitFor(() => {
      expect(saved.length).toBe(1);
    });
    expect(saved[0]).toEqual(updated);

    await unmount(view, { outro: false });
  });

  /**
   * **`character` が `null` でも失敗ではない**（`admin.rs` の `UpdatedCharacterView`）。
   * 更新は成功していて、読み直せなかっただけ。**失敗として見せない。**
   */
  it("読み直せなかった応答を失敗として見せない", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    type("display_name", "澪衣");
    responses.push({
      status: 200,
      body: JSON.stringify({ id: MIO.id, character: null }),
    });
    submit();

    await vi.waitFor(() => {
      expect(target.textContent).toContain("更新しました");
    });
    expect(target.textContent).not.toContain("できませんでした");

    await unmount(view, { outro: false });
  });
});

describe("作成", () => {
  it("`POST` で作り、その後の保存は作ったキャラクターの `PATCH` になる", async () => {
    const created: CharacterView = {
      id: 9,
      handle: "rin",
      display_name: "凛",
      base_persona: "早口で話す。",
      output_format_instruction: null,
      stat_scale: 10,
      stat_drift_mode: null,
      axis_discovery_slots: 0,
      archived_at: null,
      created_at: "2026-08-27T01:00:00Z",
    };
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("handle", created.handle);
    type("display_name", created.display_name);
    type("base_persona", created.base_persona);

    responses.push({ status: 201, body: JSON.stringify(created) });
    submit();
    await vi.waitFor(() => {
      expect(calls.length).toBe(1);
    });

    expect(calls[0]?.method).toBe("POST");
    expect(calls[0]?.url).toBe("/admin/characters");
    expect(calls[0]?.body).toEqual({
      handle: created.handle,
      display_name: created.display_name,
      base_persona: created.base_persona,
    });

    // **ここから先は編集。** 同じ画面がもう一度 `POST` すると、
    // 2件目を作ろうとして「既にあります」で 409 になる。
    await vi.waitFor(() => {
      expect(hasButton("保存")).toBe(true);
    });
    expect(hasButton("作成")).toBe(false);

    type("display_name", "凛々");
    responses.push({
      status: 200,
      body: JSON.stringify({
        id: created.id,
        character: { ...created, display_name: "凛々" },
      }),
    });
    submit();
    await vi.waitFor(() => {
      expect(calls.length).toBe(2);
    });

    expect(calls[1]?.method).toBe("PATCH");
    expect(calls[1]?.url).toBe(`/admin/characters/${created.handle}`);
    expect(calls[1]?.body).toEqual({ display_name: "凛々" });

    await unmount(view, { outro: false });
  });

  it("省いた欄はキーごと送らない（サーバの既定に任せる）", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("handle", "rin");
    type("base_persona", "早口で話す。");
    submit();
    await vi.waitFor(() => {
      expect(calls.length).toBe(1);
    });

    expect(calls[0]?.body).toEqual({
      handle: "rin",
      base_persona: "早口で話す。",
    });

    await unmount(view, { outro: false });
  });
});

describe("上限", () => {
  it("超えたら送らない（未超過→超過→直す の3相）", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    // 第1相: **上限ちょうどは送れる。**
    type("base_persona", "あ".repeat(LIMITS.base_persona));
    expect(buttonLabelled("保存").disabled).toBe(false);

    // 第2相: **1文字超えたら押せない。**
    type("base_persona", "あ".repeat(LIMITS.base_persona + 1));
    expect(buttonLabelled("保存").disabled).toBe(true);
    // **何が起きたかが画面に出る**（押せないだけだと理由が分からない）。
    expect(target.textContent).toContain("ペルソナ本文");
    expect(target.textContent).toContain(String(LIMITS.base_persona));

    // **送っていない。**
    submit();
    expect(calls.length).toBe(0);

    // 第3相: **減らせば送れる。**
    type("base_persona", "あ".repeat(LIMITS.base_persona));
    expect(buttonLabelled("保存").disabled).toBe(false);
    submit();
    await vi.waitFor(() => {
      expect(calls.length).toBe(1);
    });

    await unmount(view, { outro: false });
  });

  it("いま何文字かを、サーバと同じ数え方で出す", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    // `🐈` は符号位置で 1（`.length` は 2）。
    type("base_persona", "🐈");
    expect(counter("base_persona").textContent?.trim()).toBe(
      `1 / ${LIMITS.base_persona}`,
    );

    await unmount(view, { outro: false });
  });

  /**
   * **カウンタは、その欄の上限を出す。**
   *
   * `counterOf` の `LIMITS[field]` を `LIMITS.base_persona` に決め打つ変異は、
   * **止まる／止まらないを1文字も変えない**（判定は `built` の側）ので、
   * **打っている最中ずっと「`0 / 600`」という嘘を見せるだけ**になる。
   *
   * **4欄とも自分の上限を出していることを見る**
   * （どれか1つでも他人の上限なら赤）。
   */
  it("カウンタは、その欄の上限を出す", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("handle", "ab");
    type("display_name", "あ");
    type("base_persona", "あい");
    type("output_format_instruction", "あいう");

    expect(counter("handle").textContent?.trim()).toBe(`2 / ${LIMITS.handle}`);
    expect(counter("display_name").textContent?.trim()).toBe(
      `1 / ${LIMITS.display_name}`,
    );
    expect(counter("base_persona").textContent?.trim()).toBe(
      `2 / ${LIMITS.base_persona}`,
    );
    expect(counter("output_format_instruction").textContent?.trim()).toBe(
      `3 / ${LIMITS.output_format_instruction}`,
    );

    await unmount(view, { outro: false });
  });

  /**
   * **超過の印は、超えた欄にだけ付く。**
   *
   * `over(field)` を `isOverLimit("base_persona", …)` に決め打つ変異は、
   * **ボタンもメッセージも正しいまま**（どちらも `built` が決める）、
   * **印だけが消える／別の欄に付く。**
   *
   * **識別子で見る** —— 51 文字は `LIMITS.handle` を超えるが
   * `LIMITS.base_persona` は超えないので、**決め打ちなら印が付かない。**
   */
  it("印は、超えた欄にだけ付く", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("handle", "a".repeat(LIMITS.handle + 1));
    type("base_persona", "あ");

    expect(counter("handle").classList.contains("over")).toBe(true);
    // **対照: 超えていない欄には付かない**（「常に付く」実装と区別する）。
    expect(counter("base_persona").classList.contains("over")).toBe(false);
    expect(counter("display_name").classList.contains("over")).toBe(false);

    await unmount(view, { outro: false });
  });
});

describe("送り方", () => {
  /**
   * **JSON を送るときだけ `content-type` を名乗る。**
   *
   * `sendJson` の doc が「本文の有無で出し分ける」と書いているので、
   * **その doc が指すものを実際に見る**（ヘッダは記録しない限り観測外になる）。
   * **`GET` 側は `character-list-editing.test.ts` が見る。**
   */
  it("本文を送る回は `content-type` を名乗る", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    type("display_name", "澪衣");
    submit();
    await vi.waitFor(() => {
      expect(calls.length).toBe(1);
    });

    expect(calls[0]?.headers["content-type"]).toBe("application/json");
    expect(calls[0]?.headers["accept"]).toBe("application/json");

    await unmount(view, { outro: false });
  });
});

describe("サーバが弾いた回", () => {
  it("理由を画面に出す", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("handle", "Rin");
    type("base_persona", "早口で話す。");
    responses.push({
      status: 400,
      body: JSON.stringify({
        error: {
          message: "キャラクター識別子に使えない文字が含まれています",
          type: "invalid_request_error",
          code: "character_handle_charset",
        },
      }),
    });
    submit();

    await vi.waitFor(() => {
      expect(target.textContent).toContain(
        "キャラクター識別子に使えない文字が含まれています",
      );
    });
    // **作成のままでいる。** 通っていないのに編集へ移ると、
    // 次の保存が `PATCH` になって「存在しません」で 404 になる。
    expect(hasButton("作成")).toBe(true);

    await unmount(view, { outro: false });
  });

  it("繋がらなかった回も、黙って成功にしない", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("failed")));
    type("display_name", "澪衣");
    submit();

    await vi.waitFor(() => {
      expect(target.textContent).toContain("接続できませんでした");
    });
    expect(target.textContent).not.toContain("更新しました");

    await unmount(view, { outro: false });
  });
});

/**
 * **画面が、利用者に ADR の番号を見せない**（作業単位 F-8）。
 *
 * **文は消さない。** 「識別子は作成のときだけ決められる」は
 * **利用者が知る必要のある事実**で、**落とすのは出典だけ。**
 *
 * # ⚠ ソースの走査だけでは足りない
 *
 * `tests/screen-wording.test.ts` は**「出典が無いこと」しか言わない**ので、
 * **文ごと消しても緑になる。** ここは、**実際に描かれた DOM の上で文を見ている。**
 */
describe("画面が利用者の言葉で話す（作業単位 F-8）", () => {
  it("作成のときに、識別子が後から変えられないことを言う（出典は出さない）", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    const text = (target.textContent ?? "").replace(/\s+/g, " ");
    expect(text).toContain("作成のときだけ決められます");
    expect(text).toContain("後から変えられません");
    // **出典は画面に出さない。**
    expect(text).not.toContain("ADR");
    // **括弧の中が空にならない。**
    //
    // **⚠ これは F-5 で報告された見え方を捕まえていない**（レビューの訂正 / 2026-08-29）。
    // 着手前の DOM は `（POST /admin/characters で作れます）` で、**中身は入っていた**
    // ——「空に見える」は司令塔がアクセシビリティツリーを読んだときの見え方だった。
    // **F-5 を実際に捕まえているのは `chat-view-empty.test.ts` の `DEVELOPER_WORDS` の側。**
    //
    // **ここが止めるのは「括弧だけ残して中身を落とす」変異**で、それは実在しうる。
    expect(text).not.toMatch(/（\s*）/);

    await unmount(view, { outro: false });
  });

  it("対照: 編集のときは、その注記そのものが出ない", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    const text = (target.textContent ?? "").replace(/\s+/g, " ");
    expect(text).not.toContain("作成のときだけ決められます");
    expect(text).not.toContain("ADR");

    await unmount(view, { outro: false });
  });
});
