// @vitest-environment jsdom
/**
 * **一覧から修復を開く導線**（作業単位 D-6 / `src/lib/CharacterList.svelte`）。
 *
 * # `character-repair-form.test.ts` との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | フォーム単体の欄・送る本文・拒否の表示 | `character-repair-form.test.ts` |
 * | **一覧のどの行を押したら、どの行のフォームが開くか** | **ここ** |
 * | 送る本文の組み立て（純粋関数） | `character-repair.test.ts` |
 *
 * # ⚠ **選ぶ操作は、選ばれる対象を端に置かない**
 *
 * D-2 のレビューで実測: **「常に先頭行」は3赤だが「常に最終行」は本命を素通り**した
 * （本命が最終行を押していたため）。
 * **直せる行を3つ用意して、真ん中を押す。** これで1本で両向きが赤になる。
 *
 * # ⚠ **識別子が1つしか無いと、決め打ちが全緑を通る**（`D-3` で実測）
 *
 * **別の行を押す腕**を必ず1本置く。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CharacterList from "../src/lib/CharacterList.svelte";
import type { CharacterView, UnreadableCharacterView } from "../src/lib/api";

/** 読み出せた行（壊れた行と同居していることを見るために1件だけ置く）。 */
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

/** 直せる行・1件目。**壊れた列は1つ。** */
const KOU: UnreadableCharacterView = {
  id: 11,
  handle: "kou",
  invalid_columns: ["display_name"],
};

/**
 * **識別子そのものが読めない行。**
 *
 * **修復の入口を出さない**（`PATCH` でも `DELETE` でも指せない /
 * `domain::UnreadableCharacter` の `handle` の doc）。
 * **直せる行の間に挟んである** —— 「直せる行だけを並べた一覧」ではないことを、
 * 押す位置の側でも作っておく。
 */
const NAMELESS: UnreadableCharacterView = {
  id: 13,
  handle: null,
  invalid_columns: ["handle", "base_persona"],
};

/**
 * 直せる行・2件目。**これが「真ん中」**（直せる行のうち、先頭でも最終でもない）。
 *
 * **壊れた列が複数**なので、1件の行との取り違えも観測できる。
 */
const SEN: UnreadableCharacterView = {
  id: 12,
  handle: "sen",
  invalid_columns: ["base_persona", "stat_scale"],
};

/** 直せる行・3件目。**SEN を端に置かないために居る**（上記）。 */
const TSUKI: UnreadableCharacterView = {
  id: 14,
  handle: "tsuki",
  invalid_columns: ["output_format_instruction"],
};

/**
 * 直せる行・4件目。**`TSUKI` を端に置かないために居る。**
 *
 * # ⚠ **2本目の腕も、端でない行を押す**（D-6 のレビューの軽い指摘2）
 *
 * 直せる行が3件だと、**本命（真ん中）以外に押せるのは端しか残らない。**
 * **端を押す腕は、「常に端」の変異を片側しか殺せない**（D-2 の教訓）。
 * **4件にすると、真ん中が2つできる**（`SEN` と `TSUKI`）ので、
 * **両方の腕が端でない行を押せる。**
 */
const RIN: UnreadableCharacterView = {
  id: 15,
  handle: "rin",
  invalid_columns: ["display_name", "output_format_instruction"],
};

/** 一覧が返す並び。**直せない行が間に挟まっている。** */
const UNREADABLE = [KOU, NAMELESS, SEN, TSUKI, RIN];

interface Call {
  method: string;
  url: string;
  body: unknown;
}

let calls: Call[];
let target: HTMLElement;
/** その回の一覧が返す壊れた行（0件の腕で差し替える）。 */
let unreadable: UnreadableCharacterView[];

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

    const respond = (body: unknown, status = 200): Promise<Response> =>
      Promise.resolve({
        ok: status >= 200 && status < 300,
        status,
        text: () => Promise.resolve(JSON.stringify(body)),
      } as unknown as Response);

    if (url === "/admin/characters" && method === "GET") {
      return respond({ characters: [AOI], unreadable });
    }
    if (url.endsWith("/stats")) {
      return respond({ character: AOI.handle, stat_scale: 5, stats: [] });
    }
    // 修復（`PATCH`）。**読み直せた姿を返す。**
    return respond({ id: 99, character: AOI });
  });
}

/** 壊れた行の `<tr>`。**id で引く**（並び順に寄りかからない）。 */
function brokenRow(row: UnreadableCharacterView): HTMLElement {
  const found = target.querySelector(`tr[data-unreadable-id="${row.id}"]`);
  expect(found, `id ${row.id} の行があること`).not.toBeNull();
  return found as HTMLElement;
}

/** その行の「修復」ボタン。**無ければ `null`。** */
function repairButtonOf(
  row: UnreadableCharacterView,
): HTMLButtonElement | null {
  const found = [...brokenRow(row).querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === "修復",
  );
  return found ?? null;
}

/** 押す（無ければテストごと落とす）。 */
function pressRepair(row: UnreadableCharacterView): void {
  const button = repairButtonOf(row);
  expect(button, `id ${row.id} の行に「修復」があること`).not.toBeNull();
  (button as HTMLButtonElement).click();
  flushSync();
}

/**
 * 開いている**修復**フォーム。
 *
 * # ⚠ **`querySelector("form")` で引かないこと**（レビューの軽い指摘3）
 *
 * この画面には**編集フォームも開きうる**（`CharacterForm`）。
 * 素で引くと**先頭の `<form>`**＝マークアップ上いま前にある編集フォームを掴む。
 * **いまは当たらない**（このファイルは編集を開かない）が、
 * **「編集を開いた状態で修復を開く」腕を将来足した日に、黙って別のフォームを掴む。**
 *
 * **修復フォームの目印（`[data-repair-row]`）の祖先から引く。**
 */
function form(): HTMLFormElement {
  const heading = target.querySelector("[data-repair-row]");
  expect(heading, "修復フォームが開いていること").not.toBeNull();
  const found = (heading as HTMLElement).closest("form");
  expect(found, "修復フォームが `<form>` の中にあること").not.toBeNull();
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

/** その属性を持つ要素の文字列（**画面全体の `toContain` を使わない**）。 */
function textOf(selector: string, within: HTMLElement = target): string {
  const found = within.querySelector(selector);
  expect(found, `${selector} があること`).not.toBeNull();
  return (found as HTMLElement).textContent?.trim() ?? "";
}

/** 壊れた行が並ぶまで待つ。 */
async function waitForBrokenRows(count: number): Promise<void> {
  await vi.waitFor(() => {
    expect(target.querySelectorAll("tr[data-unreadable-id]").length).toBe(count);
  });
}

beforeEach(() => {
  calls = [];
  unreadable = UNREADABLE;
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("押した行の修復が開く", () => {
  /** **真ん中を押す**（「常に先頭」も「常に最終」もここで落ちる）。 */
  it("真ん中の行を押すと、その行のフォームが開く", async () => {
    const view = mount(CharacterList, { target });
    await waitForBrokenRows(UNREADABLE.length);

    pressRepair(SEN);

    const heading = textOf("[data-repair-row]");
    expect(heading).toContain(String(SEN.id));
    expect(heading).toContain(SEN.handle as string);
    // **他のどの行でもない**（先頭・末尾を含む）。
    expect(heading).not.toContain(KOU.handle as string);
    expect(heading).not.toContain(TSUKI.handle as string);
    expect(heading).not.toContain(RIN.handle as string);
    expect(heading).not.toContain(String(NAMELESS.id));

    await unmount(view, { outro: false });
  });

  it("押した行の識別子へ `PATCH` される", async () => {
    const view = mount(CharacterList, { target });
    await waitForBrokenRows(UNREADABLE.length);

    pressRepair(SEN);
    type("stat_scale", "5");
    form().dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    const patch = await vi.waitFor(() => {
      const found = calls.find((call) => call.method === "PATCH");
      expect(found).toBeDefined();
      return found as Call;
    });
    expect(patch.url).toBe(`/admin/characters/${SEN.handle}`);
    expect(patch.url).not.toContain(KOU.handle as string);
    expect(patch.url).not.toContain(TSUKI.handle as string);
    // **全部の欄が載る**（差分モードに戻っていない）。
    expect(Object.keys(patch.body as object).sort()).toEqual([
      "base_persona",
      "display_name",
      "output_format_instruction",
      // **変容の走らせ方は載らない**（作業単位 D-15）—— この行は走らせ方の列が
      // 壊れていないので、触らなければ省く（壊れている行では載る /
      // `character-repair-form.test.ts`）。
      "stat_scale",
    ]);

    await unmount(view, { outro: false });
  });

  /** **別の識別子で回る腕**（決め打ちの変異はここで落ちる）。 */
  it("別の行を押すと、別の識別子へ `PATCH` される", async () => {
    const view = mount(CharacterList, { target });
    await waitForBrokenRows(UNREADABLE.length);

    pressRepair(TSUKI);
    type("stat_scale", "5");
    form().dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    const patch = await vi.waitFor(() => {
      const found = calls.find((call) => call.method === "PATCH");
      expect(found).toBeDefined();
      return found as Call;
    });
    expect(patch.url).toBe(`/admin/characters/${TSUKI.handle}`);
    expect(patch.url).not.toContain(SEN.handle as string);
    // **端のどちらでもない**（`RIN` の doc —— 2本目の腕も端を押さない）。
    expect(patch.url).not.toContain(KOU.handle as string);
    expect(patch.url).not.toContain(RIN.handle as string);

    await unmount(view, { outro: false });
  });

  /**
   * **開き直すと作り直される**（`CharacterForm` の `draft` と同じ壊れ方）。
   *
   * 持ち越すと、**前の行に書きかけたペルソナが別の行へ書き込まれる**
   * ——しかも見出しは新しい行を名乗る。**帰属の壊れ方。**
   */
  it("別の行を開くと、前の行の下書きが残らない", async () => {
    const view = mount(CharacterList, { target });
    await waitForBrokenRows(UNREADABLE.length);

    pressRepair(SEN);
    type("base_persona", "打ちかけの本文");

    pressRepair(TSUKI);

    expect(textOf("[data-repair-row]")).toContain(TSUKI.handle as string);
    expect(field("base_persona").value).toBe("");
    expect(field("base_persona").value).not.toBe("打ちかけの本文");

    await unmount(view, { outro: false });
  });

  it("修復が通ったら一覧を読み直す", async () => {
    const view = mount(CharacterList, { target });
    await waitForBrokenRows(UNREADABLE.length);
    const before = calls.filter(
      (call) => call.url === "/admin/characters" && call.method === "GET",
    ).length;

    pressRepair(SEN);
    type("stat_scale", "5");
    form().dispatchEvent(
      new Event("submit", { bubbles: true, cancelable: true }),
    );

    await vi.waitFor(() => {
      const after = calls.filter(
        (call) => call.url === "/admin/characters" && call.method === "GET",
      ).length;
      expect(after).toBe(before + 1);
    });

    await unmount(view, { outro: false });
  });
});

/**
 * **識別子が読めない行には、入口を出さない。**
 *
 * **⚠ 無い操作を案内しない**（D-7 と同じ立場）。その行は
 * `PATCH` でも `DELETE` でも指せない —— **直す手段は `sqlite3` だけ**
 * （`domain::UnreadableCharacter` の `handle` の doc）。
 */
describe("識別子そのものが壊れている行", () => {
  it("修復の入口が出ない", async () => {
    const view = mount(CharacterList, { target });
    await waitForBrokenRows(UNREADABLE.length);

    expect(repairButtonOf(NAMELESS)).toBeNull();
    // **対照: 直せる行には出ている**（「1つも出ない」と区別する）。
    expect(repairButtonOf(KOU)).not.toBeNull();
    expect(repairButtonOf(SEN)).not.toBeNull();
    expect(repairButtonOf(TSUKI)).not.toBeNull();

    await unmount(view, { outro: false });
  });

  /**
   * **事実だけを言う。**
   *
   * 「DB を直せ」は**この画面から辿れる導線ではない**ので、
   * **操作として案内しない**——いまどういう状態なのかを述べるにとどめる。
   */
  it("状態を説明する（API からは指せない、と言う）", async () => {
    const view = mount(CharacterList, { target });
    await waitForBrokenRows(UNREADABLE.length);

    const said = textOf("[data-unrepairable]", brokenRow(NAMELESS));
    expect(said).toContain("API");
    expect(said).toContain("指せません");
    expect(said).toContain("DB");
    // **押せるものとして書いていない。**
    expect(brokenRow(NAMELESS).querySelectorAll("button").length).toBe(0);
    expect(brokenRow(NAMELESS).querySelectorAll("a").length).toBe(0);

    await unmount(view, { outro: false });
  });

  it("直せる行には、その説明が出ない", async () => {
    const view = mount(CharacterList, { target });
    await waitForBrokenRows(UNREADABLE.length);

    expect(brokenRow(SEN).querySelector("[data-unrepairable]")).toBeNull();
    expect(brokenRow(KOU).querySelector("[data-unrepairable]")).toBeNull();

    await unmount(view, { outro: false });
  });
});

describe("読めなかった列は、行ごとに出る", () => {
  /** **1件の行と複数の行を、両方見る**（`join` を落とす変異も帰属の変異も殺す）。 */
  it("その行の列だけが、その行に出る", async () => {
    const view = mount(CharacterList, { target });
    await waitForBrokenRows(UNREADABLE.length);

    const one = textOf("[data-invalid-columns]", brokenRow(KOU));
    expect(one).toContain("display_name");
    expect(one).not.toContain("base_persona");
    expect(one).not.toContain("stat_scale");

    const many = textOf("[data-invalid-columns]", brokenRow(SEN));
    expect(many).toContain("base_persona");
    expect(many).toContain("stat_scale");
    expect(many).not.toContain("display_name");

    const other = textOf("[data-invalid-columns]", brokenRow(TSUKI));
    expect(other).toContain("output_format_instruction");
    expect(other).not.toContain("stat_scale");

    await unmount(view, { outro: false });
  });

  /**
   * **属性の値も、その行のものであること**（レビューの軽い指摘1）。
   *
   * **実測**: 属性値だけを別の行のものに決め打つ変異が **262 全緑**だった ——
   * テストが `textContent` しか読んでいなかったため。
   * **運ばせるなら、押さえる。**
   */
  it("属性も、その行の列を運ぶ", async () => {
    const view = mount(CharacterList, { target });
    await waitForBrokenRows(UNREADABLE.length);

    for (const row of UNREADABLE) {
      const cell = brokenRow(row).querySelector("[data-invalid-columns]");
      expect(cell, `id ${row.id} に列の欄があること`).not.toBeNull();
      expect((cell as HTMLElement).getAttribute("data-invalid-columns")).toBe(
        row.invalid_columns.join(" "),
      );
    }
    // **全部が同じ値ではない**（決め打ちなら揃ってしまう）。
    expect(
      new Set(UNREADABLE.map((row) => row.invalid_columns.join(" "))).size,
    ).toBeGreaterThan(1);

    await unmount(view, { outro: false });
  });

  /** **直せない行でも列は出る**（入口が無いことと、壊れ方が見えないことは別）。 */
  it("直せない行でも、読めなかった列は出る", async () => {
    const view = mount(CharacterList, { target });
    await waitForBrokenRows(UNREADABLE.length);

    const columns = textOf("[data-invalid-columns]", brokenRow(NAMELESS));
    expect(columns).toContain("handle");
    expect(columns).toContain("base_persona");

    await unmount(view, { outro: false });
  });
});

describe("壊れた行が0件のとき", () => {
  /**
   * **節ごと消さない**（D-3 と同じ立場 ——「壊れた行が無い」ことと
   * 「この画面が壊れた行を見せない古い版である」ことを、見る側が区別できるように）。
   */
  it("節は出るが、修復の入口は1つも無い", async () => {
    unreadable = [];
    const view = mount(CharacterList, { target });
    await vi.waitFor(() => {
      expect(target.querySelectorAll("table").length).toBeGreaterThanOrEqual(1);
    });

    expect(target.querySelectorAll("tr[data-unreadable-id]").length).toBe(0);
    const repairButtons = [...target.querySelectorAll("button")].filter(
      (button) => button.textContent?.trim() === "修復",
    );
    expect(repairButtons).toEqual([]);
    // **「読み出せなかった行」の節そのものは残っている。**
    expect(
      [...target.querySelectorAll("h2")].some((heading) =>
        heading.textContent?.includes("読み出せなかった行"),
      ),
    ).toBe(true);

    await unmount(view, { outro: false });
  });
});
