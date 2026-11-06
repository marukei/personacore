// @vitest-environment jsdom
/**
 * **修復フォーム単体**（作業単位 D-6 / `src/lib/CharacterRepairForm.svelte`）。
 *
 * # `character-repair.test.ts` との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 送る本文の組み立て・失敗の読み分け（純粋関数） | `character-repair.test.ts` |
 * | **どの行のフォームが、どの識別子へ、何を送るか** | **ここ** |
 * | 一覧のどの行を押したらこのフォームが開くか | `character-list-repair.test.ts` |
 *
 * # ⚠ **失敗の結末を `fetch` から通す**
 *
 * 「文言は見られているが、経路は通っていない」形にしない ——
 * **拒否は実際の応答本文（`{"error":{...,"code":...}}`）として返す**ので、
 * `api.ts` の `parseErrorBody` → `ApiRequestError.code` →
 * `describeRepairFailure` → 画面、までが1本に繋がっていることを見る。
 *
 * # ⚠ **識別子を決め打ちにする変異を殺すため、腕が2本ある**
 *
 * **フィクスチャの `handle` が1つしか無いと、決め打ちが全緑を通る**（`D-3` で実測）。
 * **別の行で回る腕**を必ず1本置く。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CharacterRepairForm from "../src/lib/CharacterRepairForm.svelte";
import { LIMITS } from "../src/lib/characterEdit";
import { REPAIR_COLUMNS } from "../src/lib/characterRepair";
import type { RepairableRow } from "../src/lib/characterRepair";

/** 1件目。**壊れた列が複数。** */
const SEN: RepairableRow = {
  id: 12,
  handle: "sen",
  invalid_columns: ["base_persona", "stat_scale"],
};

/**
 * 2件目。**1件目と id も識別子も壊れた列も違う。**
 *
 * **決め打ちの変異はここで落ちる**（上の ⚠）。
 */
const KOU: RepairableRow = {
  id: 11,
  handle: "kou",
  invalid_columns: ["display_name"],
};

/**
 * 3件目。**`output_format_instruction` が壊れている行。**
 *
 * # ⚠ **この行が無いと、4列のうち1列だけ無防備になる**
 *
 * **D-6 のレビューで実測**: `broken("output_format_instruction")` を
 * `broken("output_format")` に変えても **262 全緑**だった ——
 * **SEN と KOU のどちらでもその列が壊れていないので、
 * 「印が付かないこと」しか観測していなかった。**
 * **正の主張（印が付く）を一度も受けていない列が1つあった。**
 *
 * **4列とも、付く腕と付かない腕の両方を通すこと。**
 */
const TSUKI: RepairableRow = {
  id: 14,
  handle: "tsuki",
  invalid_columns: ["output_format_instruction"],
};

/**
 * 4件目。**変容の走らせ方（`stat_drift_mode`）が壊れている行**（作業単位 D-15）。
 *
 * **列を足したら、その列に印が付く行を1つ置く**（`TSUKI` の doc の実測と同じ理由 ——
 * 無いと、その列は「印が付かないこと」しか観測されない）。
 */
const NAGI: RepairableRow = {
  id: 17,
  handle: "nagi",
  invalid_columns: ["stat_drift_mode"],
};

interface Call {
  method: string;
  url: string;
  body: unknown;
}

let calls: Call[];
let target: HTMLElement;
/** 次の応答（テストごとに差し替える）。 */
let respondWith: (call: Call) => { status: number; body: unknown };

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const raw = init?.body;
    const call: Call = {
      method: init?.method ?? "GET",
      url: String(input),
      body: typeof raw === "string" ? (JSON.parse(raw) as unknown) : null,
    };
    calls.push(call);
    const { status, body } = respondWith(call);
    return Promise.resolve({
      ok: status >= 200 && status < 300,
      status,
      text: () => Promise.resolve(JSON.stringify(body)),
    } as unknown as Response);
  });
}

/** **読み直せた姿**を返す既定の応答。 */
function ok(call: Call): { status: number; body: unknown } {
  return {
    status: 200,
    body: {
      id: 12,
      character: {
        id: 12,
        handle: String(call.url.split("/").pop()),
        display_name: "直した",
        base_persona: "直した本文",
        output_format_instruction: null,
        stat_scale: 5,
        created_at: "2026-08-27T00:00:00Z",
      },
    },
  };
}

/** サーバの拒否（**本文の形は `error.rs` の `ErrorBody`**）。 */
function rejection(
  status: number,
  message: string,
  code: string | null,
): { status: number; body: unknown } {
  return {
    status,
    body: {
      error:
        code === null
          ? { message, type: "invalid_request_error" }
          : { message, type: "invalid_request_error", code },
    },
  };
}

function open(row: RepairableRow): ReturnType<typeof mount> {
  return mount(CharacterRepairForm, { target, props: { row } });
}

function form(): HTMLFormElement {
  const found = target.querySelector("form");
  expect(found, "フォームが開いていること").not.toBeNull();
  return found as HTMLFormElement;
}

function field(name: string): HTMLInputElement | HTMLTextAreaElement {
  const found = form().querySelector(`[name="${name}"]`);
  expect(found, `${name} の欄があること`).not.toBeNull();
  return found as HTMLInputElement | HTMLTextAreaElement;
}

function type(name: string, value: string): void {
  const input = field(name);
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

/** 文字の4つの欄を全部埋める（走らせ方は `choose` で選ぶ）。 */
function fillAll(): void {
  type("display_name", "宗");
  type("base_persona", "低い声で話す。");
  type("output_format_instruction", "台詞は「」で囲む。");
  type("stat_scale", "7");
}

/** 走らせ方を表示名で選ぶ（`bind:value` を起こすため `change` を投げる）。 */
function choose(labelPrefix: string): void {
  const select = form().querySelector('select[name="stat_drift_mode"]');
  expect(select, "走らせ方の選択欄があること").not.toBeNull();
  const element = select as HTMLSelectElement;
  const option = [...element.options].find((candidate) =>
    candidate.textContent?.trim().startsWith(labelPrefix),
  );
  expect(option, `「${labelPrefix}」の選択肢があること`).toBeDefined();
  element.value = (option as HTMLOptionElement).value;
  element.dispatchEvent(new Event("change", { bubbles: true }));
  flushSync();
}

function submit(): void {
  form().dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
}

async function patchCall(): Promise<Call> {
  return vi.waitFor(() => {
    const found = calls.find((call) => call.method === "PATCH");
    expect(found).toBeDefined();
    return found as Call;
  });
}

/** その属性を持つ要素の文字列（**画面全体の `toContain` を使わない**）。 */
function textOf(selector: string): string {
  const found = target.querySelector(selector);
  expect(found, `${selector} があること`).not.toBeNull();
  return (found as HTMLElement).textContent?.trim() ?? "";
}

/** その要素の属性の**値**（セレクタとして使うだけにしない / 下記）。 */
function attrOf(selector: string, attribute: string): string | null {
  const found = target.querySelector(selector);
  expect(found, `${selector} があること`).not.toBeNull();
  return (found as HTMLElement).getAttribute(attribute);
}

beforeEach(() => {
  calls = [];
  respondWith = ok;
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("送り先は、開いた行の識別子", () => {
  it("1件目: その行へ `PATCH` する", async () => {
    const view = open(SEN);
    fillAll();
    submit();

    const patch = await patchCall();
    expect(patch.url).toBe(`/admin/characters/${SEN.handle}`);
    // **もう1つの行へ流れていない。**
    expect(patch.url).not.toContain(KOU.handle);

    await unmount(view, { outro: false });
  });

  /** **別の識別子で回る腕**（決め打ちの変異はここで落ちる）。 */
  it("2件目: 別の行なら、別の識別子へ `PATCH` する", async () => {
    const view = open(KOU);
    fillAll();
    submit();

    const patch = await patchCall();
    expect(patch.url).toBe(`/admin/characters/${KOU.handle}`);
    expect(patch.url).not.toContain(SEN.handle);

    await unmount(view, { outro: false });
  });
});

describe("全部の欄を送る", () => {
  /**
   * **5つのキーが全部載る**（5つ目の走らせ方は作業単位 D-15 で足した）。
   * **走らせ方は真ん中の「記録だけ」を選ぶ**（端を選ぶと決め打ちが通る）。
   *
   * **`SEN` は走らせ方の列が壊れていない行** —— **選び直したので送る**
   * （触らなければ省く側は下の「走らせ方の列が壊れていない行は…」）。
   */
  it("5つのキーが全部載る（差分ではない）", async () => {
    const view = open(SEN);
    fillAll();
    choose("記録だけ");
    submit();

    const patch = await patchCall();
    expect(patch.body).toEqual({
      display_name: "宗",
      base_persona: "低い声で話す。",
      output_format_instruction: "台詞は「」で囲む。",
      stat_scale: 7,
      stat_drift_mode: "shadow",
    });
    expect(Object.keys(patch.body as object).length).toBe(5);

    await unmount(view, { outro: false });
  });

  /**
   * **空のまま送っても、キーは落ちない。**
   *
   * **落とすと「変えない」になり、その列は壊れたまま残る。**
   * 空であることの是非はサーバが言う（`DisplayName::parse("")` は 400）。
   */
  it("空欄でもキーは落ちない（形式指示は `null`）", async () => {
    const view = open(SEN);
    type("stat_scale", "5");
    submit();

    const patch = await patchCall();
    expect(patch.body).toEqual({
      display_name: "",
      base_persona: "",
      output_format_instruction: null,
      stat_scale: 5,
      // **`SEN` は走らせ方の列が壊れていない**ので、触らなければキーを省く
      // （作業単位 D-15 のレビューの是正 / 下の「走らせ方の列」の節）。
    });

    await unmount(view, { outro: false });
  });

  /**
   * **走らせ方の列が壊れていない行は、触らなければ走らせ方を送らない**
   * （作業単位 D-15 のレビューの是正）。
   *
   * 壊れていない列の値は正しい。**送ると、その正しい値を上書きする**
   * （初期値が「既定に従う」だったころは、未設定で上書きしていた）。
   * **サーバは省いた列を残す**（`admin.rs` の `UpdateCharacterRequest`）。
   */
  it("走らせ方の列が壊れていない行は、触らなければ走らせ方のキーが無い", async () => {
    const view = open(SEN);
    const select = form().querySelector(
      'select[name="stat_drift_mode"]',
    ) as HTMLSelectElement;
    expect(select.selectedOptions[0]?.textContent?.trim()).toBe(
      "変えない（いまの設定のまま）",
    );
    fillAll();
    submit();

    const patch = await patchCall();
    expect("stat_drift_mode" in (patch.body as object)).toBe(false);
    expect(Object.keys(patch.body as object).sort()).toEqual([
      "base_persona",
      "display_name",
      "output_format_instruction",
      "stat_scale",
    ]);

    await unmount(view, { outro: false });
  });

  /**
   * **走らせ方の列が壊れている行は、触らなくても送る**（`null` ＝ 既定に従う）。
   * **壊れたまま残さない**（修復の目的そのもの）。
   */
  it("走らせ方の列が壊れている行は、触らなくても走らせ方を送る", async () => {
    const view = open(NAGI);
    const select = form().querySelector(
      'select[name="stat_drift_mode"]',
    ) as HTMLSelectElement;
    // **「変えない」は出さない**（壊れた値を残す選択肢になる）。
    expect([...select.options].map((option) => option.textContent?.trim())).toEqual([
      "既定に従う",
      "止める",
      "記録だけ（値は動かさない）",
      "動かす",
    ]);
    fillAll();
    submit();

    const patch = await patchCall();
    expect((patch.body as Record<string, unknown>).stat_drift_mode).toBeNull();
    expect(Object.keys(patch.body as object).length).toBe(5);

    await unmount(view, { outro: false });
  });

  it("識別子のキーは載らない（不変 / ADR-0016 §1）", async () => {
    const view = open(SEN);
    fillAll();
    submit();

    const patch = await patchCall();
    expect(Object.keys(patch.body as object)).not.toContain("handle");
    // **識別子を打ち替える欄も無い。**
    expect(form().querySelector('[name="handle"]')).toBeNull();

    await unmount(view, { outro: false });
  });

  it("尺度が空なら、そもそも送らない", async () => {
    const view = open(SEN);
    type("display_name", "宗");
    submit();
    flushSync();

    expect(calls.filter((call) => call.method === "PATCH")).toEqual([]);
    expect(textOf("[data-repair-blocked]")).toContain("尺度");

    await unmount(view, { outro: false });
  });
});

describe("読めなかった列を出す", () => {
  it("その行の壊れた列を、そのまま名乗る", async () => {
    const view = open(SEN);
    const columns = textOf("[data-invalid-columns]");
    // **生の列名を落とさない**（DB を直接見に行く手掛かり）。
    expect(columns).toContain("base_persona");
    expect(columns).toContain("stat_scale");
    // **別の行の列は出ていない**（帰属）。
    expect(columns).not.toContain("display_name");

    await unmount(view, { outro: false });
  });

  /**
   * **属性の値も、その行のものであること**（レビューの軽い指摘1）。
   *
   * **実測**: 属性値だけを別の行のものに決め打つ変異が **262 全緑**だった ——
   * **テストは `textContent` しか読んでおらず、属性はセレクタとしてしか
   * 使われていなかった。** 実害は無かったが、
   * **「属性が列名を運んでいる」と読める形で死んだ値が残っていた。**
   * **運ばせるなら、押さえる。**
   */
  it("属性も、その行の列を運ぶ", async () => {
    const view = open(SEN);
    expect(attrOf("[data-invalid-columns]", "data-invalid-columns")).toBe(
      SEN.invalid_columns.join(" "),
    );
    await unmount(view, { outro: false });

    const other = open(KOU);
    expect(attrOf("[data-invalid-columns]", "data-invalid-columns")).toBe(
      KOU.invalid_columns.join(" "),
    );
    // **同じ値ではない**（決め打ちなら両方同じになる）。
    expect(KOU.invalid_columns.join(" ")).not.toBe(
      SEN.invalid_columns.join(" "),
    );
    await unmount(other, { outro: false });
  });

  /** **別の行では、別の列が出る**（決め打ち・取り違えの両方をここで殺す）。 */
  it("別の行なら、別の列が出る", async () => {
    const view = open(KOU);
    const columns = textOf("[data-invalid-columns]");
    expect(columns).toContain("display_name");
    expect(columns).not.toContain("base_persona");
    expect(columns).not.toContain("stat_scale");

    await unmount(view, { outro: false });
  });

  /**
   * **どの欄が壊れていたのかが、欄の側で分かる。**
   *
   * **付いた欄と付いていない欄を両方見る** ——
   * 「全部の欄に付ける」変異も「1つも付けない」変異も、ここで落ちる。
   */
  it("壊れていた欄にだけ印が付く", async () => {
    const view = open(SEN);
    expect(
      target.querySelector('[data-broken-field="base_persona"]'),
    ).not.toBeNull();
    expect(
      target.querySelector('[data-broken-field="stat_scale"]'),
    ).not.toBeNull();
    expect(
      target.querySelector('[data-broken-field="display_name"]'),
    ).toBeNull();
    expect(
      target.querySelector('[data-broken-field="output_format_instruction"]'),
    ).toBeNull();

    await unmount(view, { outro: false });
  });

  it("別の行では、印の付く欄が入れ替わる", async () => {
    const view = open(KOU);
    expect(
      target.querySelector('[data-broken-field="display_name"]'),
    ).not.toBeNull();
    expect(target.querySelector('[data-broken-field="base_persona"]')).toBeNull();
    expect(target.querySelector('[data-broken-field="stat_scale"]')).toBeNull();
    expect(
      target.querySelector('[data-broken-field="output_format_instruction"]'),
    ).toBeNull();

    await unmount(view, { outro: false });
  });

  /**
   * **4列目の正の主張**（`TSUKI` の doc / レビューの是正2）。
   *
   * これが無いと、**`output_format_instruction` だけが
   * 「印が付かないこと」しか見られていない**状態になり、
   * その1行の綴りを崩す変異が全緑を通る（実測済み）。
   */
  it("出力形式指示が壊れている行でも、その欄に印が付く", async () => {
    const view = open(TSUKI);
    expect(
      target.querySelector('[data-broken-field="output_format_instruction"]'),
    ).not.toBeNull();
    // **他の3列には付いていない。**
    expect(
      target.querySelector('[data-broken-field="display_name"]'),
    ).toBeNull();
    expect(target.querySelector('[data-broken-field="base_persona"]')).toBeNull();
    expect(target.querySelector('[data-broken-field="stat_scale"]')).toBeNull();

    await unmount(view, { outro: false });
  });

  /**
   * **4列とも、付く腕と付かない腕の両方を通っていること。**
   *
   * 上の3本を合わせると、**`REPAIR_COLUMNS` の各列について
   * `true` の腕が1つ以上、`false` の腕が1つ以上**ある。
   * **ここはその表を明示して、列が増えたときに気付ける形にしてある。**
   */
  it("列すべてに、印が付く行が実在する", () => {
    const covered = new Set([
      ...SEN.invalid_columns,
      ...KOU.invalid_columns,
      ...TSUKI.invalid_columns,
      ...NAGI.invalid_columns,
    ]);
    expect(covered).toEqual(new Set(REPAIR_COLUMNS));
  });

  /**
   * **走らせ方の列の正の主張**（作業単位 D-15 / `NAGI` の doc）。
   * **付かない腕**は、他の行（`SEN` / `KOU` / `TSUKI`）で見る。
   */
  it("走らせ方が壊れている行では、その欄にだけ印が付く", async () => {
    const view = open(NAGI);
    expect(
      target.querySelector('[data-broken-field="stat_drift_mode"]'),
    ).not.toBeNull();
    for (const other of [
      "display_name",
      "base_persona",
      "output_format_instruction",
      "stat_scale",
    ]) {
      expect(target.querySelector(`[data-broken-field="${other}"]`), other).toBeNull();
    }
    await unmount(view, { outro: false });

    for (const row of [SEN, KOU, TSUKI]) {
      const another = open(row);
      expect(
        target.querySelector('[data-broken-field="stat_drift_mode"]'),
        `${row.handle} には付かない`,
      ).toBeNull();
      await unmount(another, { outro: false });
    }
  });

  it("見出しは、その行の id と識別子を名乗る", async () => {
    const view = open(SEN);
    const heading = textOf("[data-repair-row]");
    expect(heading).toContain(String(SEN.id));
    expect(heading).toContain(SEN.handle);
    expect(heading).not.toContain(KOU.handle);
    expect(heading).not.toContain(String(KOU.id));

    await unmount(view, { outro: false });
  });
});

describe("文字数計", () => {
  /**
   * **⚠ `toContain` では見られない。** `LIMITS.output_format_instruction`（150）は
   * `LIMITS.display_name`（50）を**部分文字列として含む**ので、
   * 「他の欄の上限が出ていない」を含有で書くと必ず落ちる／必ず通る。
   * **丸ごと一致で見る** —— **3つとも別の値**なので、取り違えは1本で全部赤になる。
   */
  it("カウンタは、その欄の上限を出す", async () => {
    const view = open(SEN);
    expect(textOf('[data-counter="display_name"]')).toBe(
      `0 / ${LIMITS.display_name}`,
    );
    expect(textOf('[data-counter="base_persona"]')).toBe(
      `0 / ${LIMITS.base_persona}`,
    );
    expect(textOf('[data-counter="output_format_instruction"]')).toBe(
      `0 / ${LIMITS.output_format_instruction}`,
    );

    await unmount(view, { outro: false });
  });

  it("上限を超えたら、送らずに理由を出す", async () => {
    const view = open(SEN);
    type("stat_scale", "5");
    type("base_persona", "あ".repeat(LIMITS.base_persona + 1));
    submit();
    flushSync();

    expect(calls.filter((call) => call.method === "PATCH")).toEqual([]);
    expect(textOf("[data-repair-blocked]")).toContain("ペルソナ本文");

    await unmount(view, { outro: false });
  });
});

/**
 * **拒否は `fetch` から通す。**
 *
 * **区別するのは `classify_update` が分けている2つだけ**
 * （`character_not_found` / `stat_scale_below_stored_value`）。
 * それ以外は「分からない」に落として、**次の一手を捏造しない。**
 */
describe("拒否されたときに、画面が言うこと", () => {
  it("尺度が保存済みの値より小さい（409）", async () => {
    respondWith = () =>
      rejection(
        409,
        "保存されている性格ステータスの値より小さい尺度には変更できません",
        "stat_scale_below_stored_value",
      );

    const view = open(SEN);
    fillAll();
    submit();

    await vi.waitFor(() => {
      expect(
        target.querySelector('[data-repair-reason="scale-below-stored"]'),
      ).not.toBeNull();
    });
    // **サーバの文言をそのまま出す。**
    expect(textOf('[data-repair-reason="scale-below-stored"]')).toContain(
      "小さい尺度には変更できません",
    );
    // **次の一手が出る。**
    expect(textOf("[data-repair-guidance]")).toContain("大きい尺度");

    await unmount(view, { outro: false });
  });

  it("その識別子が居ない（404）", async () => {
    respondWith = () =>
      rejection(404, "指定されたキャラクターが存在しません", "character_not_found");

    const view = open(SEN);
    fillAll();
    submit();

    await vi.waitFor(() => {
      expect(
        target.querySelector('[data-repair-reason="not-found"]'),
      ).not.toBeNull();
    });
    expect(textOf("[data-repair-guidance]")).toContain("再読み込み");
    // **尺度側の一手と混ざっていない。**
    expect(textOf("[data-repair-guidance]")).not.toContain("大きい尺度");

    await unmount(view, { outro: false });
  });

  /**
   * **500 に次の一手は無い。**
   *
   * `classify_update` の `_ => internal(...)` はサーバの障害なので、
   * **推測で埋めない**（`characterDelete.ts` と同じ立場）。
   */
  it("分類できない失敗には、次の一手を出さない", async () => {
    respondWith = () =>
      rejection(500, "キャラクターを更新できませんでした", null);

    const view = open(SEN);
    fillAll();
    submit();

    await vi.waitFor(() => {
      expect(
        target.querySelector('[data-repair-reason="unknown"]'),
      ).not.toBeNull();
    });
    expect(textOf('[data-repair-reason="unknown"]')).toContain(
      "更新できませんでした",
    );
    expect(target.querySelector("[data-repair-guidance]")).toBeNull();

    await unmount(view, { outro: false });
  });

  /** **400 は素通し**（サーバが壊れている欄を名乗る）。 */
  it("`parse` の拒否（400）は、サーバの文言をそのまま出す", async () => {
    respondWith = () =>
      rejection(400, "ペルソナ本文が長すぎます", "persona_too_long");

    const view = open(SEN);
    fillAll();
    submit();

    await vi.waitFor(() => {
      expect(
        target.querySelector('[data-repair-reason="unknown"]'),
      ).not.toBeNull();
    });
    expect(textOf('[data-repair-reason="unknown"]')).toContain(
      "ペルソナ本文が長すぎます",
    );

    await unmount(view, { outro: false });
  });
});

describe("通ったとき", () => {
  it("読み直せたら、そう言う", async () => {
    const view = open(SEN);
    fillAll();
    submit();

    await vi.waitFor(() => {
      expect(target.querySelector("[data-repair-notice]")).not.toBeNull();
    });
    expect(textOf("[data-repair-notice]")).toContain("読み出せるようになりました");

    await unmount(view, { outro: false });
  });

  /**
   * **`character: null` は失敗ではない**（`admin.rs` の `UpdatedCharacterView`）。
   * 更新は通っていて、読み直せなかっただけ。**失敗として出さない。**
   */
  it("読み直せなくても、失敗として出さない", async () => {
    respondWith = () => ({ status: 200, body: { id: SEN.id, character: null } });

    const view = open(SEN);
    fillAll();
    submit();

    await vi.waitFor(() => {
      expect(target.querySelector("[data-repair-notice]")).not.toBeNull();
    });
    expect(textOf("[data-repair-notice]")).toContain("読み直せていません");
    expect(target.querySelector("[data-repair-reason]")).toBeNull();

    await unmount(view, { outro: false });
  });

  it("親に知らせる（一覧を取り直すため）", async () => {
    const seen: unknown[] = [];
    const view = mount(CharacterRepairForm, {
      target,
      props: { row: SEN, onRepaired: (value: unknown) => seen.push(value) },
    });
    fillAll();
    submit();

    await vi.waitFor(() => {
      expect(seen.length).toBe(1);
    });

    await unmount(view, { outro: false });
  });
});

/**
 * **画面が、利用者に ADR の番号を見せない**（作業単位 F-8）。
 *
 * **文は消さない。** 「識別子は変えられない」は
 * **利用者が知る必要のある事実**で、**落とすのは出典だけ。**
 *
 * # ⚠ ソースの走査だけでは足りない
 *
 * `tests/screen-wording.test.ts` は**「出典が無いこと」しか言わない**ので、
 * **文ごと消しても緑になる。** ここは、**実際に描かれた DOM の上で文を見ている。**
 *
 * # ⚠ 腕が2本ある
 *
 * **その行の識別子を名乗っている**ことまで見る ——
 * 決め打ちの変異が、この検査でも通らないように。
 */
describe("画面が利用者の言葉で話す（作業単位 F-8）", () => {
  it("識別子が変えられないことを、その行の識別子つきで言う（出典は出さない）", async () => {
    const view = mount(CharacterRepairForm, { target, props: { row: SEN } });

    const text = (target.textContent ?? "").replace(/\s+/g, " ");
    expect(text).toContain("識別子");
    expect(text).toContain("変えられません");
    expect(text).toContain(SEN.handle);
    // **対照: 別の行の識別子は出ていない。**
    expect(text).not.toContain(KOU.handle);
    // **出典は画面に出さない。**
    expect(text).not.toContain("ADR");
    // **括弧の中が空にならない。**
    // **⚠ F-5 で報告された見え方は捕まえていない**（`character-form.test.ts` の同じ注記）。
    // ここが止めるのは「括弧だけ残して中身を落とす」変異。
    expect(text).not.toMatch(/（\s*）/);

    await unmount(view, { outro: false });
  });

  it("別の行でも、同じことをその行の識別子で言う", async () => {
    const view = mount(CharacterRepairForm, { target, props: { row: KOU } });

    const text = (target.textContent ?? "").replace(/\s+/g, " ");
    expect(text).toContain("変えられません");
    expect(text).toContain(KOU.handle);
    expect(text).not.toContain(SEN.handle);
    expect(text).not.toContain("ADR");

    await unmount(view, { outro: false });
  });
});
