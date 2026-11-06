// @vitest-environment jsdom
/**
 * **いまの値が読めない画面から、性格ステータスを直す**（作業単位 D-8 /
 * `src/lib/StatsRepairForm.svelte` ＋ `src/lib/StatsPanel.svelte` の失敗の枝）。
 *
 * # ⚠ **本物の壊れ方では、変容の側も一緒に落ちる**
 *
 * `get_drift` も `Store::stat_allocs` を読む
 * （`tests/server_admin.rs::a_broken_stat_row_stops_stats_and_drift_but_the_listing_still_reports_the_scale`
 * で実測）。**だから、ここの既定の標本は「stats も drift も 500」である。**
 * **片方だけ落ちる標本で測ると、本物の壊れ方を一度も通らない。**
 *
 * # ⚠ **尺度の出どころは一覧しか無い**
 *
 * `GET /admin/characters/{handle}` は存在せず（`PATCH` と `DELETE` だけ / ADR-0016 §1）、
 * **`GET .../drift` は一緒に落ちている。**
 * **一覧は `stat_alloc` を1行も読まない**ので、そこだけが残る（同じテストで実測）。
 *
 * # `stats-repair.test.ts` との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 判定・写し元・下書き | `stats-repair.test.ts` |
 * | **画面が何を出すか・どこへ何を送るか** | **ここ** |
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CharacterList from "../src/lib/CharacterList.svelte";
import StatsPanel from "../src/lib/StatsPanel.svelte";
import type {
  CharacterListView,
  CharacterView,
  DriftView,
  StatsView,
} from "../src/lib/api";
import {
  DUPLICATE_AXIS_CODE,
  NOT_FOUND_CODE,
  SCALE_CHANGED_CODE,
  VALUE_ABOVE_SCALE_CODE,
} from "../src/lib/statsEdit";
import {
  MALFORMED_LISTING_NOTICE,
  UNKNOWN_LOSS_NOTICE,
  UNKNOWN_SCALE_NOTICE,
} from "../src/lib/statsRepair";
import { readerInZone } from "./support/zone";

/**
 * **読み手の時間帯を名乗る**（作業単位 F-7d）。
 *
 * このファイルは**時刻の綴りを1つも見ていない**が、
 * **`StatsPanel` を開く**ので、画面は `browserZone()` で機械に訊く。
 * **名乗らないと、その読みはその場で落ちる**（`tests/support/ambient-zone.ts`）。
 *
 * **どの時間帯でもよい** —— ここが見ているのは修復の枝であって時刻ではない。
 */
readerInZone("America/New_York");

/** 直す相手。**一覧の真ん中に置く**（端を押さない / `D-2` のレビューの実測）。 */
const MIO = "mio";
/** **前の行。** 「一覧の先頭を取る」変異はこちらの尺度を出す。 */
const AOI = "aoi";
/** **後ろの行。** 「一覧の末尾を取る」変異はこちらの尺度を出す。 */
const NAR = "nar";
/** **キャラクターの行そのものが壊れている相手。** */
const KURO = "kuro";

/**
 * ⚠ **尺度は3件とも別の値**（同値化を避ける / CLAUDE.md）。
 * **どれも `StatScale::DEFAULT`（10）ではない。**
 */
const AOI_SCALE = 5;
const MIO_SCALE = 7;
const NAR_SCALE = 9;

function characterOf(handle: string, id: number, scale: number): CharacterView {
  return {
    id,
    handle,
    display_name: `${handle}の呼び名`,
    base_persona: `${handle}のペルソナ。`,
    output_format_instruction: null,
    stat_scale: scale,
    stat_drift_mode: null,
    axis_discovery_slots: 0,
    archived_at: null,
    created_at: "2026-08-28T00:00:00Z",
  };
}

const LISTING: CharacterListView = {
  characters: [
    characterOf(AOI, 1, AOI_SCALE),
    characterOf(MIO, 2, MIO_SCALE),
    characterOf(NAR, 3, NAR_SCALE),
  ],
  unreadable: [
    { id: 4, handle: KURO, invalid_columns: ["base_persona"] },
    { id: 5, handle: null, invalid_columns: ["handle"] },
  ],
};

/**
 * 直った後に読み直せる値。
 *
 * **⚠ 打った内容と同じにしない。** 同じにすると
 * **「読み直した」と「打った値が欄に残っている」が同じ見え方**になる
 * （`stats-panel.test.ts` の `mioStatsNow` と同じ理由）。
 */
const REPAIRED: StatsView = {
  character: MIO,
  stat_scale: MIO_SCALE,
  stats: [
    { axis: "勇気", value: 1 },
    { axis: "慎重さ", value: 4 },
    { axis: "陽気さ", value: 6 },
  ],
};

const REPAIRED_DRIFT: DriftView = {
  character: MIO,
  stat_scale: MIO_SCALE,
  axes: [
    {
      axis: "勇気",
      value: 1,
      charge: 0.5,
      charge_updated_at: "2026-08-26T01:02:03.400Z",
      ups: 2,
      downs: 1,
    },
  ],
  total_votes: 3,
  recent_limit: 50,
  recent_votes: [
    {
      axis: "勇気",
      delta: "up",
      session_id: 41,
      created_at: "2026-08-28T09:00:00Z",
    },
  ],
  run_mode: "on",
  run_mode_source: "default",
  default_run_mode: "on",
};

/** **`ApiError::internal()` の応答**（`src/server/error.rs` / **`code` は無い**）。 */
const INTERNAL_BODY = JSON.stringify({
  error: { message: "内部エラーが発生しました", type: "server_error" },
});

/** **`ApiError::character_not_found()` の応答。** */
const NOT_FOUND_BODY = JSON.stringify({
  error: {
    message: "指定されたキャラクターが存在しません",
    type: "invalid_request_error",
    code: NOT_FOUND_CODE,
  },
});

interface Call {
  method: string;
  url: string;
  body: unknown;
}

let calls: Call[];
let target: HTMLElement;

/** いまの値と変容が読めるか。**既定は「どちらも壊れている」**（本物の壊れ方）。 */
let statsReadable: boolean;
/**
 * **いまの値が 200 で返るが、形が違う。**
 *
 * `api.ts` は**返り値の形を検証しない**ので、`draftOfStats` が `TypeError` を投げ、
 * `describeStatsFailure` が **`unknown`** を返す（`statsRepair.ts` の ⚠）。
 * **サーバが 500 を返す回とは、別の経路である。**
 */
let statsMalformed: boolean;
/** 一覧の応答（`null` なら一覧そのものが 500）。 */
let listing: CharacterListView | null;
/** **一覧が 200 で返るが、形が違う**（版の食い違い）。 */
let listingMalformed: boolean;
/**
 * **一覧の応答を待たせる関門。**
 *
 * `loadStats` は `statsState` を立ててから `await loadScale()` するので、
 * **その間の描画**（`pending`）をここで捕まえる。
 * **`vi.waitFor` では書けない** —— 見たいのは「A が B になる」ではなく
 * 「**その間 B であってはならない**」（`D-4` の罠の裏返し）。
 */
let listingGate: { promise: Promise<void>; open: () => void } | null;
/** `fetch` そのものが失敗する（サーバへ届かない）。 */
let unreachable: boolean;
/** `PUT` の応答（`null` なら成功）。 */
let putResponse: { status: number; body: string } | null;

/** 一覧の応答を止める。**開けるまで `loadScale` は終わらない。** */
function holdListing(): void {
  let open = (): void => {};
  const promise = new Promise<void>((resolve) => {
    open = () => {
      resolve();
    };
  });
  listingGate = { promise, open };
}

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

    if (unreachable) {
      // **`fetch` が投げる** —— `api.ts` はこれを状態番号 `0` に写す。
      return Promise.reject(new TypeError("Failed to fetch"));
    }

    const respond = (body: unknown, status = 200): Promise<Response> =>
      Promise.resolve({
        ok: status >= 200 && status < 300,
        status,
        text: () =>
          Promise.resolve(
            typeof body === "string" ? body : JSON.stringify(body),
          ),
      } as unknown as Response);

    if (url === "/admin/characters") {
      const answer = (): Promise<Response> => {
        if (listingMalformed) {
          // **200 だが形が違う**（`characters` が配列ではない）。
          return respond({ characters: "配列ではない", unreadable: [] });
        }
        return listing === null ? respond(INTERNAL_BODY, 500) : respond(listing);
      };
      const gate = listingGate;
      return gate === null ? answer() : gate.promise.then(answer);
    }
    if (method === "PUT") {
      if (putResponse !== null) {
        return respond(putResponse.body, putResponse.status);
      }
      // **⚠ 宛先を見る。** 見ないと、**別のキャラクターへ飛ばす変異でも
      // 「直った」ことになる**（フィクスチャが2つを同値にする形 / CLAUDE.md）。
      // **実測**: 宛先を決め打つ変異は、ここを見ないと 1 赤・見ると 2 赤。
      if (url !== `/admin/characters/${MIO}/stats`) {
        return respond(NOT_FOUND_BODY, 404);
      }
      statsReadable = true;
      return respond(REPAIRED);
    }
    if (url === `/admin/characters/${MIO}/stats`) {
      if (statsMalformed) {
        // **200。しかし `stats` が無い** —— `draftOfStats` が落ちる。
        return respond({ character: MIO, stat_scale: MIO_SCALE });
      }
      return statsReadable ? respond(REPAIRED) : respond(INTERNAL_BODY, 500);
    }
    if (url === `/admin/characters/${MIO}/drift`) {
      return statsReadable
        ? respond(REPAIRED_DRIFT)
        : respond(INTERNAL_BODY, 500);
    }
    if (url.startsWith(`/admin/characters/${KURO}/`)) {
      return respond(INTERNAL_BODY, 500);
    }
    // **居ないキャラクター。**
    return respond(NOT_FOUND_BODY, 404);
  });
}

/** 画面を出して、いまの値の枝が決まるまで待つ。 */
async function show(handle: string): Promise<{ dispose: () => Promise<void> }> {
  const view = mount(StatsPanel, { target, props: { handle } });
  await vi.waitFor(() => {
    expect(target.querySelector("[data-stats-error]")).not.toBeNull();
  });
  flushSync();
  return {
    dispose: async () => {
      await unmount(view, { outro: false });
    },
  };
}

/** 修復のフォームが出るまで待つ。 */
async function showWithRepairForm(
  handle: string,
): Promise<{ dispose: () => Promise<void> }> {
  const shown = await show(handle);
  await vi.waitFor(() => {
    expect(target.querySelector("[data-stats-repair]")).not.toBeNull();
  });
  flushSync();
  return shown;
}

/** 修復の行。**索引で引く**（軸名は空にも重複にもなりうる）。 */
function repairRow(index: number): HTMLElement {
  const found = target.querySelector(`tr[data-repair-row="${index}"]`);
  expect(found, `${index} 行目があること`).not.toBeNull();
  return found as HTMLElement;
}

function fieldOf(index: number, name: string): HTMLInputElement {
  const found = repairRow(index).querySelector(`input[name="${name}"]`);
  expect(found, `${index} 行目に ${name} の欄があること`).not.toBeNull();
  return found as HTMLInputElement;
}

function type(index: number, name: string, value: string): void {
  const field = fieldOf(index, name);
  field.value = value;
  field.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

/** 画面のボタンを押す（**見出しで引く** —— 読み手が押すのと同じもの）。 */
function click(label: string): void {
  const found = [...target.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `「${label}」があること`).toBeDefined();
  (found as HTMLButtonElement).click();
  flushSync();
}

/** 行の中のボタンを押す。 */
function clickInRow(index: number, label: string): void {
  const found = [...repairRow(index).querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `${index} 行目に「${label}」があること`).toBeDefined();
  (found as HTMLButtonElement).click();
  flushSync();
}

function submitRepair(): void {
  const form = target.querySelector("form[data-stats-repair]");
  expect(form, "修復のフォームがあること").not.toBeNull();
  (form as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  flushSync();
}

/**
 * 空白の並び（改行と字下げ）。
 *
 * **⚠ 綴りを1箇所に置いてある。** `Bash` 経由で書き込むと逆斜線が1段潰れて
 * `/s+/` になり、**構文としては通るので緑のまま**壊れる（CLAUDE.md / `D-4` の実測）。
 */
const WHITESPACE = /\s+/g;

function flatText(element: Element): string {
  return (element.textContent ?? "").replace(WHITESPACE, " ").trim();
}

/** 案内が `「」` で名指ししている操作。 */
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

/**
 * **案内が出ているなら、名指しした操作は全部この画面に描かれている。**
 *
 * `stats-panel.test.ts` の `assertNoticeNamesOnlyRealOperations` と同じ物差しを、
 * **失敗の枝の案内にも当てる**（D-7 / B-15 / B-16 / D-6 で4回出ている形）。
 */
function assertGuidanceNamesOnlyRealOperations(element: Element): void {
  const named = quotedOperations(flatText(element));
  // **名指ししていること自体を見る**（「」を外す変異を空振りで通さない）。
  expect(named.length, "案内が操作を名指しすること").toBeGreaterThan(0);
  const labels = controlLabels(target);
  for (const operation of named) {
    expect(
      labels.has(operation),
      `案内の「${operation}」が、いまの画面に実在すること（実在するのは ${[...labels].join(" / ")}）`,
    ).toBe(true);
  }
}

beforeEach(() => {
  calls = [];
  statsReadable = false;
  statsMalformed = false;
  listing = LISTING;
  listingMalformed = false;
  listingGate = null;
  unreachable = false;
  putResponse = null;
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("いまの値が読めなくても、画面だけで直せる", () => {
  /**
   * **この作業単位の存在理由。**
   *
   * **標本は「stats も drift も 500」**（本物の壊れ方）。
   * **変容が読めることに寄りかかった入口は、この状態で一度も出ない。**
   */
  it("いまの値も変容も読めない回に、修復の入口が出る", async () => {
    const shown = await showWithRepairForm(MIO);

    // **通常のフォームは出ていない**（読めていないので下書きが作れない）。
    expect(target.querySelector("tr[data-axis]")).toBeNull();
    // **両方の失敗が出ている**（本物の壊れ方であること）。
    expect(target.querySelector("[data-stats-error]")).not.toBeNull();
    expect(target.querySelector("[data-drift-error]")).not.toBeNull();
    // **直す操作が実在する。**
    const labels = controlLabels(target);
    expect(labels.has("軸を1本足す")).toBe(true);
    expect(labels.has("この内容で作り直す")).toBe(true);

    await shown.dispose();
  });

  /**
   * **200 でも形が違えば、`unknown` として扱う**（レビューの指摘 / 2026-08-28）。
   *
   * # なぜこの標本が要るのか
   *
   * `loadStats` は **`draftOfStats(value)` を `getStats` と同じ `try` の中**で呼ぶ。
   * `api.ts` は**返り値の形を検証しない**と自ら明記しているので、
   * **`stats` の欠けた 200 は `TypeError` になり、`unknown` に落ちる。**
   *
   * **その回に入口が出ないと、D-8 が直そうとした状態そのもの**になる ——
   * **エラーの `<p>` だけが出て、フォームも説明も無い。**
   * **`repairEntryFor` は `unknown` でもフォームを返すが、
   * 画面がその状態に一度も入らなければ、つないでいる1行は誰も通らない**
   * （`13c-3a` の「分岐が効く状態にフィクスチャが入らない」）。
   */
  it("いまの値が 200 でも形が違えば、想定外として扱い、それでも直せる", async () => {
    statsMalformed = true;
    const shown = await showWithRepairForm(MIO);

    // **`unknown` の定型文**（サーバの 500 の文言ではない ——
    // これが出ている＝この標本が `unknown` の枝に入っている証拠）。
    const error = flatText(
      target.querySelector("[data-stats-error]") as Element,
    );
    expect(error).toContain("予期しない失敗です");
    expect(error).not.toContain("内部エラーが発生しました");
    // **中身を画面へ流していない**（絶対制約）。
    expect(error).not.toContain("TypeError");
    expect(error).not.toContain("undefined");
    // **尺度は一覧から引けている**（想定外でも、そこは動く）。
    expect(
      flatText(target.querySelector('[data-repair-scale="known"]') as Element),
    ).toContain(String(MIO_SCALE));
    // **直せる。**
    expect(controlLabels(target).has("この内容で作り直す")).toBe(true);

    await shown.dispose();
  });

  /**
   * **尺度は一覧から引いた、このキャラクターのもの。**
   *
   * **⚠ 既定と同値にしない**（`B-7` の実測 —— 落とし所で回すと、
   * 引いてこようが決め打ちだろうが区別が付かない）。
   * `mio` は 7 で、**一覧の先頭は 5・末尾は 9。**
   */
  it("尺度は一覧のこのキャラクターのもの（先頭でも末尾でもない）", async () => {
    const shown = await showWithRepairForm(MIO);

    const line = target.querySelector('[data-repair-scale="known"]');
    expect(line, "尺度を言う行があること").not.toBeNull();
    const text = flatText(line as Element);
    expect(text).toContain(String(MIO_SCALE));
    expect(text).not.toContain(String(AOI_SCALE));
    expect(text).not.toContain(String(NAR_SCALE));
    // **値の欄にも同じ上端が出る**（読み手が実際に見る場所）。
    expect(flatText(repairRow(0))).toContain(`/ ${MIO_SCALE}`);
    // **一覧を実際に引いている。**
    expect(calls.some((call) => call.url === "/admin/characters")).toBe(true);

    await shown.dispose();
  });

  /**
   * **打った軸が、そのまま・全部・このキャラクターへ送られる。**
   *
   * **3行にして真ん中も押す**（`D-2` の実測 ——
   * 2件だと「常に先頭」「常に末尾」の片方が通り抜ける）。
   */
  it("打った軸を全部、この識別子へ送る", async () => {
    const shown = await showWithRepairForm(MIO);

    click("軸を1本足す");
    click("軸を1本足す");
    type(0, "axis", "勇気");
    type(0, "value", "1");
    type(1, "axis", "慎重さ");
    type(1, "value", "4");
    type(2, "axis", "陽気さ");
    type(2, "value", "6");

    submitRepair();
    await vi.waitFor(() => {
      expect(calls.some((call) => call.method === "PUT")).toBe(true);
    });

    const put = calls.find((call) => call.method === "PUT");
    // **宛先** —— 別のキャラクターへ丸ごと置き換えを飛ばしていない。
    expect(put?.url).toBe(`/admin/characters/${MIO}/stats`);
    // **本文** —— 順序も件数も、打ったとおり。
    expect(put?.body).toEqual({
      stats: [
        { axis: "勇気", value: 1 },
        { axis: "慎重さ", value: 4 },
        { axis: "陽気さ", value: 6 },
      ],
    });

    await shown.dispose();
  });

  /** **真ん中の行を外すと、その行だけ送らない**（外すことが「消す」の唯一の表現）。 */
  it("外した行は送らない（真ん中を外す）", async () => {
    const shown = await showWithRepairForm(MIO);

    click("軸を1本足す");
    click("軸を1本足す");
    type(0, "axis", "勇気");
    type(0, "value", "1");
    type(1, "axis", "慎重さ");
    type(1, "value", "4");
    type(2, "axis", "陽気さ");
    type(2, "value", "6");
    clickInRow(1, "外す");

    submitRepair();
    await vi.waitFor(() => {
      expect(calls.some((call) => call.method === "PUT")).toBe(true);
    });

    expect(calls.find((call) => call.method === "PUT")?.body).toEqual({
      stats: [
        { axis: "勇気", value: 1 },
        { axis: "陽気さ", value: 6 },
      ],
    });

    await shown.dispose();
  });

  /** **軸名が空の行は送らない**（案内が名指しする「外す」は、この画面に実在する）。 */
  it("軸名が空のままでは送らず、実在する操作を名指しする", async () => {
    const shown = await showWithRepairForm(MIO);

    type(0, "value", "1");
    const blocked = target.querySelector("[data-repair-blocked]");
    expect(blocked, "止めた理由が出ること").not.toBeNull();
    assertGuidanceNamesOnlyRealOperations(blocked as Element);
    expect(calls.some((call) => call.method === "PUT")).toBe(false);

    await shown.dispose();
  });

  /**
   * **保存が通ったら、いまの値を読み直して通常の画面に戻る。**
   *
   * # ⚠ 「A が B になる」を待つ
   *
   * **`vi.waitFor` は最初に成立した時点で返る**（`D-4` の実測）ので、
   * **打った値が欄に残っているだけ**でも成立する条件では待てない。
   * **待つのは「読めた枝にしか無い印」**——`tr[data-axis]` は
   * `draftOfStats` から作られる行で、**修復のフォームには存在しない。**
   * **押す前に、その印が無いことを見てから待つ。**
   */
  it("直ったら、いまの値を読み直して通常の画面に戻る", async () => {
    const shown = await showWithRepairForm(MIO);

    // **押す前は、読めた枝の印が無い。**
    expect(target.querySelector("tr[data-axis]")).toBeNull();

    type(0, "axis", "勇気");
    type(0, "value", "1");
    submitRepair();

    await vi.waitFor(() => {
      expect(target.querySelector('tr[data-axis="慎重さ"]')).not.toBeNull();
    });
    flushSync();
    // **サーバが読み直した姿が出ている**（打った1軸ではなく、応答の3軸）。
    expect(
      [...target.querySelectorAll("tr[data-axis]")].map(
        (row) => (row as HTMLElement).dataset["axis"],
      ),
    ).toEqual(["勇気", "慎重さ", "陽気さ"]);
    // **修復のフォームは消えている。**
    expect(target.querySelector("[data-stats-repair]")).toBeNull();
    expect(target.querySelector("[data-stats-error]")).toBeNull();

    await shown.dispose();
  });

  /**
   * **できないことを、できるふりをしない**（D-3c の作法）。
   *
   * 通常のフォームは「外した X は消えます」と名指しで言うが、
   * **この画面ではそれが原理的に書けない**（いまの軸を1本も読めない）。
   */
  it("何が消えるかは言えない、と画面が言う", async () => {
    const shown = await showWithRepairForm(MIO);

    const notice = target.querySelector("[data-unknown-loss]");
    expect(notice, "断りがあること").not.toBeNull();
    expect(flatText(notice as Element)).toBe(UNKNOWN_LOSS_NOTICE);

    // **何が残るかは言える。**
    type(0, "axis", "勇気");
    type(0, "value", "1");
    expect(
      flatText(target.querySelector("[data-repair-summary]") as Element),
    ).toContain("勇気");

    await shown.dispose();
  });

  /**
   * **拒否されたら、サーバが言ったことと次の一手を出す。**
   *
   * **読み分けは通常の置き換えと同じ `describeReplaceFailure`**（写しを増やさない）。
   */
  it("拒否されたら、理由と次の一手が出る", async () => {
    const shown = await showWithRepairForm(MIO);

    putResponse = {
      status: 400,
      body: JSON.stringify({
        error: {
          message: "その値は性格ステータスの尺度を超えています",
          type: "invalid_request_error",
          code: "stat_value_above_scale",
        },
      }),
    };
    type(0, "axis", "勇気");
    type(0, "value", "99");
    submitRepair();

    await vi.waitFor(() => {
      expect(
        target.querySelector('[data-repair-reason="value-above-scale"]'),
      ).not.toBeNull();
    });
    flushSync();
    expect(
      flatText(
        target.querySelector(
          '[data-repair-reason="value-above-scale"]',
        ) as Element,
      ),
    ).toContain("尺度を超えています");
    expect(target.querySelector("[data-repair-guidance]")).not.toBeNull();
    // **フォームは残っている**（打ち直せる）。
    expect(target.querySelector("[data-stats-repair]")).not.toBeNull();

    await shown.dispose();
  });

  /**
   * **拒否の案内が名指しした操作は、4つとも実在する**（レビューの指摘 / 2026-08-28）。
   *
   * # なぜ「出ていること」だけでは足りないのか
   *
   * `data-repair-blocked-reason` と `data-repair-blocked` には
   * `assertGuidanceNamesOnlyRealOperations` を当てていたのに、
   * **`describeReplaceFailure` が出す `data-repair-guidance` には
   * `not.toBeNull()` しか当てていなかった** ——
   * **B-16 が直した形と同じ**（その状態を描くテストはあった。判定を当てていなかっただけ）。
   *
   * # 実在の範囲は「この画面 ＋ 1手で行ける一覧」
   *
   * 尺度を上げる操作はこの画面に無い（キャラクターの**編集**）ので、
   * **union は `StatsPanel` ＋ `CharacterList`**
   * （`stat-scale-path.test.ts::4つの案内が名指しした操作が、全部実在する` と同じ立場）。
   *
   * # 4つ全部に当てる
   *
   * **代表1つでは足りない**（`D-6` の「列・分岐・枝の1つずつに当てる」）。
   * `duplicate-axis` は **「外す」** を名指しするが、それは**修復のフォームの中**にある ——
   * **通常のフォームが描かれていないこの状態でも実在すること**が、ここで初めて測られる。
   */
  it("拒否の案内が名指しした操作は、4つとも実在する", async () => {
    const shown = await showWithRepairForm(MIO);

    const listTarget = document.createElement("div");
    document.body.append(listTarget);
    const list = mount(CharacterList, { target: listTarget });
    await vi.waitFor(() => {
      expect(
        listTarget.querySelectorAll("tbody tr").length,
      ).toBeGreaterThanOrEqual(3);
    });
    flushSync();
    const labels = new Set([
      ...controlLabels(target),
      ...controlLabels(listTarget),
    ]);

    for (const [code, reason] of [
      [VALUE_ABOVE_SCALE_CODE, "value-above-scale"],
      [SCALE_CHANGED_CODE, "scale-changed"],
      [DUPLICATE_AXIS_CODE, "duplicate-axis"],
      [NOT_FOUND_CODE, "not-found"],
    ] as [string, string][]) {
      putResponse = {
        status: 400,
        body: JSON.stringify({
          error: {
            message: `${code} のときのサーバの文言`,
            type: "invalid_request_error",
            code,
          },
        }),
      };
      type(0, "axis", "勇気");
      type(0, "value", "1");
      submitRepair();
      await vi.waitFor(() => {
        expect(
          target.querySelector(`[data-repair-reason="${reason}"]`),
        ).not.toBeNull();
      });
      flushSync();

      const guidance = target.querySelector("[data-repair-guidance]");
      expect(guidance, `${reason} に次の一手があること`).not.toBeNull();
      const named = quotedOperations(flatText(guidance as Element));
      // **名指ししていること自体を見る**（「」を外す変異を空振りで通さない）。
      expect(
        named.length,
        `${reason} が操作を名指しすること`,
      ).toBeGreaterThan(0);
      for (const operation of named) {
        expect(
          labels.has(operation),
          `${reason} の「${operation}」が実在すること（実在するのは ${[...labels].join(" / ")}）`,
        ).toBe(true);
      }
    }
    // **対照: 判定が何でも通るわけではない。**
    expect(labels.has("票を削除")).toBe(false);
    expect(labels.has("性格ステータスを削除")).toBe(false);

    await unmount(list, { outro: false });
    listTarget.remove();
    await shown.dispose();
  });
});

describe("入口を出さない状態では、状態だけ説明する", () => {
  /**
   * **サーバへ届いていない。**
   *
   * **押せば必ず失敗するボタンを出さない。**
   * 名指しできるのは「再読み込み」だけ（この画面に実在する）。
   */
  it("サーバへ届かない回は、フォームを出さない", async () => {
    unreachable = true;
    const shown = await show(MIO);
    await vi.waitFor(() => {
      expect(
        target.querySelector('[data-repair-blocked-reason="unreachable"]'),
      ).not.toBeNull();
    });
    flushSync();

    expect(target.querySelector("[data-stats-repair]")).toBeNull();
    assertGuidanceNamesOnlyRealOperations(
      target.querySelector(
        '[data-repair-blocked-reason="unreachable"]',
      ) as Element,
    );
    // **一覧を引きに行っていない**（引いても使い道が無い）。
    expect(calls.some((call) => call.url === "/admin/characters")).toBe(false);

    await shown.dispose();
  });

  /** **そのキャラクターが居ない。** 名指しできるのは「キャラクターの一覧へ」。 */
  it("居ないキャラクターには、フォームを出さない", async () => {
    const shown = await show("いない");
    await vi.waitFor(() => {
      expect(
        target.querySelector('[data-repair-blocked-reason="missing"]'),
      ).not.toBeNull();
    });
    flushSync();

    expect(target.querySelector("[data-stats-repair]")).toBeNull();
    assertGuidanceNamesOnlyRealOperations(
      target.querySelector('[data-repair-blocked-reason="missing"]') as Element,
    );

    await shown.dispose();
  });

  /**
   * **キャラクターの行そのものが読めない。**
   *
   * `PUT .../stats` は**先にキャラクターを引く**ので、この行では必ず 500 になる
   * （`admin.rs` の `replace_stats`）。**押せば必ず失敗する操作は出さない**（D-6）。
   */
  it("キャラクターの行が読めない回は、フォームを出さず一覧へ送る", async () => {
    const shown = await show(KURO);
    await vi.waitFor(() => {
      expect(
        target.querySelector(
          '[data-repair-blocked-reason="character-unreadable"]',
        ),
      ).not.toBeNull();
    });
    flushSync();

    expect(target.querySelector("[data-stats-repair]")).toBeNull();
    assertGuidanceNamesOnlyRealOperations(
      target.querySelector(
        '[data-repair-blocked-reason="character-unreadable"]',
      ) as Element,
    );
    // **対照: 判定が何でも通るわけではない。**
    expect(controlLabels(target).has("この行を修復")).toBe(false);

    await shown.dispose();
  });

  /**
   * **一覧そのものが読めなくても、修復は止めない。**
   *
   * 分からないのは**上端だけ**で、値が超えていればサーバが理由付きで返す。
   * **知らないことを、知らないと言う。**
   */
  it("一覧が読めない回は、尺度を言わずにフォームを出す", async () => {
    listing = null;
    const shown = await showWithRepairForm(MIO);

    expect(target.querySelector('[data-repair-scale="known"]')).toBeNull();
    const unknown = target.querySelector('[data-repair-scale="unknown"]');
    expect(unknown, "尺度が分からないことを言うこと").not.toBeNull();
    expect(flatText(unknown as Element)).toBe(UNKNOWN_SCALE_NOTICE);
    // **名指しする操作は実在する。**
    assertGuidanceNamesOnlyRealOperations(unknown as Element);
    // **値の欄に上端を書かない**（既定値で埋めない）。
    expect(flatText(repairRow(0))).not.toContain("/");

    await shown.dispose();
  });

  /**
   * **形の食い違いを、握り潰さない**（レビューの指摘 / 2026-08-28）。
   *
   * # なぜ別の文が要るのか
   *
   * **「サーバから引けなかった」と「そもそも読めない形だった」は原因が違う。**
   * 前者はサーバが落ちているだけ、後者は**版の食い違い**である。
   * **同じ文にすると、道具の壊れが「サーバが落ちていた」に紛れて永久に見えない。**
   *
   * **例外にはしない** —— `scaleFromListing` が全域関数で、
   * **`malformed` という値**を返す（画面が描画の途中で落ちない）。
   */
  it("一覧の形が違えば、尺度が引けなかった回とは別の文を出す", async () => {
    listingMalformed = true;
    const shown = await showWithRepairForm(MIO);

    const notice = target.querySelector('[data-repair-scale="unknown"]');
    expect(notice, "形が読めなかったことを言うこと").not.toBeNull();
    expect(flatText(notice as Element)).toBe(MALFORMED_LISTING_NOTICE);
    // **対照: 「引けなかった」回の文ではない。**
    expect(flatText(notice as Element)).not.toBe(UNKNOWN_SCALE_NOTICE);
    // **名指しする操作は実在する。**
    assertGuidanceNamesOnlyRealOperations(notice as Element);
    // **それでも直せる**（フォームは出ている）。
    expect(controlLabels(target).has("この内容で作り直す")).toBe(true);

    await shown.dispose();
  });

  /**
   * **尺度を引いている間は、何も出さない**（`pending` / レビューの指摘 / 2026-08-28）。
   *
   * # ⚠ `vi.waitFor` では書けない
   *
   * 見たいのは「A が B になる」ではなく「**その間、B であってはならない**」。
   * **`waitFor` は最初に成立した時点で返る**ので、**通り過ぎた一瞬を見逃す**
   * （`D-4` の罠の裏返し）。**一覧の応答を止めて、その間を直接見る。**
   *
   * # 何が壊れるのか
   *
   * `pending` をフォームに寄せると、**`character-unreadable` の回でも
   * 一瞬「押せば必ず失敗する」フォームが描かれる** ——
   * 「押せば必ず失敗するボタンは出さない」が、その瞬間だけ破れる。
   */
  it("尺度を引いている間は、修復の入口も説明も出さない", async () => {
    holdListing();
    // **`character-unreadable` になる相手**（最後は `blocked` に落ち着く）。
    const shown = await show(KURO);

    // --- 一覧が返る前 ---
    expect(
      target.querySelector("[data-stats-repair]"),
      "尺度を引いている間にフォームを描かないこと（押せば必ず失敗する）",
    ).toBeNull();
    expect(
      target.querySelector("[data-repair-blocked-reason]"),
      "理由が決まる前に説明を描かないこと",
    ).toBeNull();
    // **一覧はもう要求されている**（止めているだけで、忘れてはいない）。
    expect(calls.some((call) => call.url === "/admin/characters")).toBe(true);

    // --- 開けると、説明に落ち着く ---
    listingGate?.open();
    await vi.waitFor(() => {
      expect(
        target.querySelector(
          '[data-repair-blocked-reason="character-unreadable"]',
        ),
      ).not.toBeNull();
    });
    flushSync();
    expect(target.querySelector("[data-stats-repair]")).toBeNull();

    await shown.dispose();
  });

  /**
   * **同じことを、フォームに落ち着く相手でも見る。**
   *
   * 上の1本だけだと「`blocked` の回はもともとフォームを出さない」で説明が付き、
   * **`pending` が効いていることの証拠にならない**（`D-6` の「代表1つでは足りない」）。
   * **こちらは最後にフォームが出る相手**なので、
   * **「出る前は出ていない」を見ていることになる。**
   */
  it("フォームが出る相手でも、引いている間は出さない", async () => {
    holdListing();
    const shown = await show(MIO);

    expect(
      target.querySelector("[data-stats-repair]"),
      "尺度を引き終える前にフォームを描かないこと",
    ).toBeNull();

    listingGate?.open();
    await vi.waitFor(() => {
      expect(target.querySelector("[data-stats-repair]")).not.toBeNull();
    });
    flushSync();
    expect(
      flatText(target.querySelector('[data-repair-scale="known"]') as Element),
    ).toContain(String(MIO_SCALE));

    await shown.dispose();
  });
});
