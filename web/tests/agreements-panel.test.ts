// @vitest-environment jsdom
/**
 * **約束・決まりの画面**（作業単位 A-12a / `src/lib/AgreementsPanel.svelte`）。
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 送る本文・上限の案内・失敗の読み分け・写しの突き合わせ | `agreements.test.ts` |
 * | **画面が何を出すか・どの識別子のどの取り決めへ送るか・上限で止まるか** | **ここ** |
 *
 * # ⚠ 3件にして真ん中を終わらせる
 *
 * 選ぶ操作のテストは、選ばれる対象が端にあると「常に端」の変異を通す（`D-2` のレビュー）。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import AgreementsPanel from "../src/lib/AgreementsPanel.svelte";
import type {
  AdoptAgreementRequest,
  AgreementTransitionView,
  AgreementTransitionsView,
  AgreementView,
  AgreementsView,
  CreateAgreementRequest,
  EndAgreementRequest,
} from "../src/lib/api";
import { AGREEMENT_LIMIT_REACHED_CODE } from "../src/lib/agreements";
import { readerInZone } from "./support/zone";

readerInZone("Pacific/Marquesas");

const MIO = "mio";
const NAR = "nar";

/**
 * `mio` の取り決め。**有効3件（守る側も種類も互いに違う）と、終わったもの1件。**
 */
const MIO_AGREEMENTS: AgreementView[] = [
  {
    id: 11,
    content: "名前を呼び捨てにしない",
    kind: "rule",
    keeper: "user",
    state: "active",
    source_session_id: null,
    created_at: "2026-10-20T00:00:00Z",
  },
  {
    id: 12,
    content: "週末に海へ連れて行く",
    kind: "promise",
    keeper: "character",
    state: "active",
    source_session_id: null,
    created_at: "2026-10-21T00:00:00Z",
  },
  {
    id: 13,
    content: "取り消した約束",
    kind: "promise",
    keeper: "both",
    state: "withdrawn",
    source_session_id: null,
    created_at: "2026-10-21T06:00:00Z",
  },
  {
    id: 14,
    content: "毎朝あいさつを交わす",
    kind: "rule",
    keeper: "both",
    state: "active",
    source_session_id: null,
    created_at: "2026-10-22T00:00:00Z",
  },
];

/** `nar` の取り決め。**`mio` と1文字も重ならない。** */
const NAR_AGREEMENTS: AgreementView[] = [
  {
    id: 21,
    content: "本を返す",
    kind: "promise",
    keeper: "user",
    state: "active",
    source_session_id: null,
    created_at: "2026-10-22T00:00:00Z",
  },
];

const TRANSITIONS: AgreementTransitionsView = {
  character: MIO,
  transitions: [
    {
      id: 104,
      agreement_id: 13,
      content: "取り消した約束",
      kind: "promise",
      keeper: "both",
      before: "active",
      after: "withdrawn",
      created_at: "2026-10-22T09:00:00Z",
    },
    {
      id: 103,
      agreement_id: 14,
      content: "毎朝あいさつを交わす",
      kind: "rule",
      keeper: "both",
      before: null,
      after: "active",
      created_at: "2026-10-22T00:00:00Z",
    },
  ],
  total_transitions: 9,
  // **サーバの既定（200）と違う値** —— 応答の上限を読んでいることの証拠。
  transitions_limit: 2,
};

/**
 * **提案を持つキャラクター**（作業単位 A-12c-2）。**提案中3件（守る側も中身も互いに違う）・
 * 有効1件・却下1件・果たした1件。** 提案の種類はサーバの決めどおり、いつも `rule`。
 */
const YUI = "yui";

const YUI_AGREEMENTS: AgreementView[] = [
  {
    id: 30,
    content: "敬語を使わない",
    kind: "rule",
    keeper: "user",
    state: "active",
    source_session_id: null,
    created_at: "2026-10-20T00:00:00Z",
  },
  {
    id: 31,
    content: "朝は先に起こす",
    kind: "rule",
    keeper: "character",
    state: "proposed",
    source_session_id: 5,
    created_at: "2026-10-21T00:00:00Z",
  },
  {
    id: 32,
    content: "寝る前におやすみを言う",
    kind: "rule",
    keeper: "both",
    state: "proposed",
    source_session_id: 5,
    created_at: "2026-10-22T00:00:00Z",
  },
  {
    id: 34,
    content: "捨てた提案",
    kind: "rule",
    keeper: "user",
    state: "rejected",
    source_session_id: 5,
    created_at: "2026-10-22T03:00:00Z",
  },
  {
    id: 33,
    content: "週に一度は手紙を書く",
    kind: "rule",
    keeper: "user",
    state: "proposed",
    source_session_id: 6,
    created_at: "2026-10-22T06:00:00Z",
  },
  {
    id: 35,
    content: "果たした約束",
    kind: "promise",
    keeper: "character",
    state: "fulfilled",
    source_session_id: null,
    created_at: "2026-10-22T09:00:00Z",
  },
];

/** `yui` の記録（新しい順）。**捨てた提案の「無かった → 提案中 → 却下」。** */
const YUI_TRANSITIONS: AgreementTransitionView[] = [
  {
    id: 201,
    agreement_id: 34,
    content: "捨てた提案",
    kind: "rule",
    keeper: "user",
    before: "proposed",
    after: "rejected",
    created_at: "2026-10-22T04:00:00Z",
  },
  {
    id: 200,
    agreement_id: 34,
    content: "捨てた提案",
    kind: "rule",
    keeper: "user",
    before: null,
    after: "proposed",
    created_at: "2026-10-22T03:00:00Z",
  },
];

let yuiTransitions: AgreementTransitionView[];
/** 次の一覧の読みを、この門が開くまで止める（`null` なら止めない。止めるのは1回だけ）。 */
let listHold: Promise<void> | null;
/** 採る・捨てるの失敗（`null` なら通す）。 */
let decisionFailure: { status: number; code: string; message: string } | null;

let target: HTMLElement;
let agreementsOf: Record<string, AgreementView[]>;
/** 上限（**サーバの既定 16 と違う値**にできる）。 */
let limit: number;
let posts: { url: string; body: unknown }[];
let createFailure: { status: number; code: string; message: string } | null;
/** 直近のターンの載り方（`mio` の一覧が返す）。 */
let lastTurn: AgreementsView["last_turn"];

function respond(body: unknown, status = 200): Promise<Response> {
  return Promise.resolve({
    ok: true,
    status,
    text: () => Promise.resolve(JSON.stringify(body)),
  } as unknown as Response);
}

function refuse(status: number, code: string, message: string): Promise<Response> {
  return Promise.resolve({
    ok: false,
    status,
    text: () =>
      Promise.resolve(
        JSON.stringify({ error: { message, type: "invalid_request_error", code } }),
      ),
  } as unknown as Response);
}

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    for (const handle of [MIO, NAR, YUI]) {
      const base = `/admin/characters/${handle}/agreements`;
      const listed = agreementsOf[handle] ?? [];
      if (url === base && method === "GET") {
        const view: AgreementsView = {
          character: handle,
          agreements: listed,
          active_limit: limit,
          last_turn: handle === MIO ? lastTurn : null,
        };
        // **一度だけ、返すのを止める**（読み直しが返るまでの間を作る / 作業単位 A-12c-2 のレビュー）。
        const hold = listHold;
        listHold = null;
        return hold === null ? respond(view) : hold.then(() => respond(view));
      }
      if (url === `${base}/transitions` && method === "GET") {
        if (handle === YUI) {
          return respond({
            character: handle,
            transitions: [...yuiTransitions],
            total_transitions: yuiTransitions.length,
            transitions_limit: 200,
          });
        }
        return respond(
          handle === MIO
            ? TRANSITIONS
            : { character: handle, transitions: [], total_transitions: 0, transitions_limit: 200 },
        );
      }
      // **提案を採る・捨てる**（作業単位 A-12c-2）。**記録も足す**（却下が記録の側に出ることを見るため）。
      const decision = new RegExp(`^${base}/(\\d+)/(adoption|rejection)$`).exec(url);
      if (decision !== null && method === "POST") {
        const id = Number(decision[1]);
        const adopting = decision[2] === "adoption";
        const body: unknown =
          init?.body === undefined ? undefined : (JSON.parse(String(init.body)) as unknown);
        posts.push({ url, body });
        if (decisionFailure !== null) {
          return refuse(decisionFailure.status, decisionFailure.code, decisionFailure.message);
        }
        const found = listed.find((agreement) => agreement.id === id);
        if (found === undefined) {
          return refuse(404, "agreement_not_found", "指定された取り決めが存在しません");
        }
        const decided: AgreementView = adopting
          ? { ...found, ...(body as AdoptAgreementRequest), state: "active" }
          : { ...found, state: "rejected" };
        agreementsOf[handle] = listed.map((agreement) =>
          agreement.id === id ? decided : agreement,
        );
        yuiTransitions = [
          {
            id: 300 + yuiTransitions.length,
            agreement_id: id,
            content: decided.content,
            kind: decided.kind,
            keeper: decided.keeper,
            before: "proposed",
            after: decided.state,
            created_at: "2026-10-23T12:00:00Z",
          },
          ...yuiTransitions,
        ];
        return adopting
          ? respond(decided)
          : respond({ character: handle, agreement_id: id, state: "rejected" });
      }
      if (url === base && method === "POST") {
        const body = JSON.parse(String(init?.body)) as CreateAgreementRequest;
        posts.push({ url, body });
        if (createFailure !== null) {
          return refuse(createFailure.status, createFailure.code, createFailure.message);
        }
        const written: AgreementView = {
          id: 99,
          ...body,
          state: "active",
          source_session_id: null,
          created_at: "2026-10-22T12:00:00Z",
        };
        agreementsOf[handle] = [...listed, written];
        return respond(written, 201);
      }
      const ending = new RegExp(`^${base}/(\\d+)/transitions$`).exec(url);
      if (ending !== null && method === "POST") {
        const body = JSON.parse(String(init?.body)) as EndAgreementRequest;
        posts.push({ url, body });
        const id = Number(ending[1]);
        agreementsOf[handle] = listed.map((agreement) =>
          agreement.id === id ? { ...agreement, state: body.state } : agreement,
        );
        return respond({ character: handle, agreement_id: id, state: body.state });
      }
    }
    return respond({ character: "?", agreements: [], active_limit: 0 });
  });
}

beforeEach(() => {
  target = document.createElement("div");
  document.body.append(target);
  agreementsOf = {
    [MIO]: [...MIO_AGREEMENTS],
    [NAR]: [...NAR_AGREEMENTS],
    [YUI]: [...YUI_AGREEMENTS],
  };
  limit = 16;
  posts = [];
  createFailure = null;
  decisionFailure = null;
  lastTurn = null;
  yuiTransitions = [...YUI_TRANSITIONS];
  listHold = null;
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

async function show(handle: string = MIO): Promise<{ dispose: () => Promise<void> }> {
  const view = mount(AgreementsPanel, { target, props: { handle } });
  await vi.waitFor(() => {
    expect(target.querySelector("[data-transition-total]")).not.toBeNull();
    expect(target.querySelector("[data-agreement-limit]")).not.toBeNull();
  });
  flushSync();
  return {
    dispose: async () => {
      await unmount(view, { outro: false });
    },
  };
}

/** 節の行を、セルの文字列の配列で返す（**`data-*` は権威ではない**ので、見える文字で比べる）。 */
function rowsOf(section: string): string[][] {
  return [...target.querySelectorAll(`[data-section="${section}"] tr`)].map((row) =>
    [...row.querySelectorAll("td")].map((cell) => cell.textContent?.trim() ?? ""),
  );
}

function rowById(section: string, id: number): HTMLElement {
  const found = target.querySelector(`[data-section="${section}"] tr[data-agreement-id="${id}"]`);
  expect(found, `${section} に ${id} の行があること`).not.toBeNull();
  return found as HTMLElement;
}

function buttonIn(scope: HTMLElement, label: string): HTMLButtonElement {
  const found = [...scope.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `「${label}」があること`).toBeDefined();
  return found as HTMLButtonElement;
}

function fill(name: string, value: string): void {
  const field = target.querySelector(`[name="${name}"]`) as HTMLInputElement | HTMLSelectElement;
  expect(field, `${name} の欄があること`).not.toBeNull();
  field.value = value;
  field.dispatchEvent(
    new Event(field instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }),
  );
  flushSync();
}

function submit(): void {
  const form = target.querySelector("form");
  expect(form).not.toBeNull();
  (form as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
}

describe("出すもの", () => {
  it("有効なものと終わったものを分けて、守る側・種類・中身を出す", async () => {
    const view = await show();
    expect(rowsOf("active-agreements").map((cells) => cells.slice(0, 3))).toEqual([
      ["利用者が守る", "決まり", "名前を呼び捨てにしない"],
      ["キャラクターが守る", "約束", "週末に海へ連れて行く"],
      ["両方が守る", "決まり", "毎朝あいさつを交わす"],
    ]);
    expect(rowsOf("ended-agreements").map((cells) => cells.slice(0, 4))).toEqual([
      ["両方が守る", "約束", "取り消した約束", "取り消した"],
    ]);
    // **時刻は読み手の時間帯で**（`Pacific/Marquesas` = UTC−09:30 / 直書き）。
    expect(rowsOf("active-agreements")[0]?.[3]).toBe("2026-10-19 14:30:00 (UTC-09:30)");
    await view.dispose();
  });

  it("記録は新しい順で、前の「無かった」は「書いた」と出す。総数と応答の上限を言う", async () => {
    const view = await show();
    expect(rowsOf("agreement-transitions").map((cells) => cells.slice(1))).toEqual([
      ["両方が守る", "約束", "取り消した約束", "有効", "取り消した"],
      ["両方が守る", "決まり", "毎朝あいさつを交わす", "（書いた）", "有効"],
    ]);
    expect(target.querySelector("[data-transition-total]")?.textContent).toContain(
      "全 9 件の記録のうち、新しい 2 件だけ",
    );
    expect(target.querySelector("[data-transition-total]")?.textContent).toContain(
      "上限は 2 件",
    );
    await view.dispose();
  });

  /**
   * **直近のターンで載らなかった取り決めがあれば、件数と時刻を言う**（作業単位 A-12a の決め直し）。
   * 落ちていない・まだターンが無いなら何も出さない。
   */
  it("直近のターンで落ちた件数を出す（落ちていなければ出さない）", async () => {
    let view = await show();
    expect(target.querySelector("[data-agreement-dropped]")).toBeNull();
    await view.dispose();

    lastTurn = { included: 2, dropped: 1, at: "2026-10-22T10:00:00Z" };
    view = await show();
    const notice = target.querySelector("[data-agreement-dropped]");
    expect(notice?.getAttribute("data-agreement-dropped")).toBe("1");
    expect(notice?.textContent?.trim()).toBe(
      "直近のターン（2026-10-22 00:30:00 (UTC-09:30)）では、有効な約束・決まりのうち古い 1 件が" +
        "本文に載りませんでした（予算に入りきらなかったため。載ったのは新しい 2 件）。" +
        "古いものを終わらせると、残りが載りやすくなります。",
    );
    await view.dispose();

    lastTurn = { included: 3, dropped: 0, at: "2026-10-22T10:00:00Z" };
    view = await show();
    expect(target.querySelector("[data-agreement-dropped]")).toBeNull();
    await view.dispose();
  });

  it("そのキャラクターの取り決めだけを出す（識別子を経路から取る）", async () => {
    const view = await show(NAR);
    expect(rowsOf("active-agreements").map((cells) => cells[2])).toEqual(["本を返す"]);
    expect(target.textContent).not.toContain("週末に海へ連れて行く");
    await view.dispose();
  });
});

describe("終わらせる", () => {
  it("真ん中を「果たした」で終わらせると、確かめてから、その取り決めへ送って読み直す", async () => {
    const view = await show();
    const middle = rowById("active-agreements", 12);
    buttonIn(middle, "果たした").click();
    flushSync();
    // **押しただけでは送らない**（戻せないので確かめる）。
    expect(posts).toEqual([]);
    expect(middle.querySelector("[data-agreement-confirm]")?.textContent).toContain(
      "「果たした」として終わらせます。戻せません。",
    );
    buttonIn(middle, "終わらせる").click();

    await vi.waitFor(() => {
      expect(posts).toHaveLength(1);
    });
    expect(posts[0]).toEqual({
      url: "/admin/characters/mio/agreements/12/transitions",
      body: { state: "fulfilled" },
    });
    await vi.waitFor(() => {
      expect(rowsOf("active-agreements").map((cells) => cells[2])).toEqual([
        "名前を呼び捨てにしない",
        "毎朝あいさつを交わす",
      ]);
    });
    expect(rowsOf("ended-agreements").map((cells) => [cells[2], cells[3]])).toEqual([
      ["週末に海へ連れて行く", "果たした"],
      ["取り消した約束", "取り消した"],
    ]);
    await view.dispose();
  });

  it("3つの終わらせ方は、それぞれの綴りで送る。「やめる」なら送らない", async () => {
    const view = await show();
    const middle = rowById("active-agreements", 12);
    buttonIn(middle, "破った").click();
    flushSync();
    buttonIn(middle, "やめる").click();
    flushSync();
    expect(posts).toEqual([]);
    buttonIn(rowById("active-agreements", 12), "取り消した").click();
    flushSync();
    buttonIn(rowById("active-agreements", 12), "終わらせる").click();
    await vi.waitFor(() => {
      expect(posts).toHaveLength(1);
    });
    expect(posts[0]?.body).toEqual({ state: "withdrawn" });
    await view.dispose();
  });
});

describe("書く", () => {
  it("中身・種類・守る側を選んで書くと、そのキャラクターへ送る", async () => {
    const view = await show();
    fill("content", "夏祭りに一緒に行く");
    fill("kind", "promise");
    fill("keeper", "character");
    submit();
    await vi.waitFor(() => {
      expect(posts).toHaveLength(1);
    });
    expect(posts[0]).toEqual({
      url: "/admin/characters/mio/agreements",
      body: { content: "夏祭りに一緒に行く", kind: "promise", keeper: "character" },
    });
    await vi.waitFor(() => {
      expect(rowsOf("active-agreements").map((cells) => cells[2])).toContain(
        "夏祭りに一緒に行く",
      );
    });
    await view.dispose();
  });

  it("守る側を選んでいなければ送らない（既定を作らない）", async () => {
    const view = await show();
    fill("content", "夏祭りに一緒に行く");
    fill("kind", "promise");
    submit();
    await Promise.resolve();
    expect(posts).toEqual([]);
    expect(target.querySelector("[data-build-error]")?.textContent).toBe(
      "誰が守るのかを選んでください。",
    );
    await view.dispose();
  });

  /**
   * **上限に達したら書く欄を止め、古いものを終わらせるよう促す。**
   *
   * 上限は**応答の `active_limit`**（ここでは 3 —— サーバの既定 16 と違う値）。
   */
  it("上限に達したら書く欄を止め、終わらせるよう促す。1件終わらせると開く", async () => {
    limit = 3;
    const view = await show();
    const fieldset = target.querySelector("fieldset") as HTMLFieldSetElement;
    expect(fieldset.disabled).toBe(true);
    expect(fieldset.dataset["agreementForm"]).toBe("full");
    expect(target.querySelector("[data-agreement-full]")?.textContent).toContain(
      "古いものを「果たした」「破った」「取り消した」のどれかで終わらせてください",
    );

    buttonIn(rowById("active-agreements", 12), "破った").click();
    flushSync();
    buttonIn(rowById("active-agreements", 12), "終わらせる").click();
    await vi.waitFor(() => {
      expect((target.querySelector("fieldset") as HTMLFieldSetElement).disabled).toBe(false);
    });
    expect(target.querySelector("[data-agreement-full]")).toBeNull();
    await view.dispose();
  });

  it("サーバが上限で断ったら、理由と一手を出し、何も足さない", async () => {
    createFailure = {
      status: 409,
      code: AGREEMENT_LIMIT_REACHED_CODE,
      message: "有効な取り決めが上限に達しています。古いものを終わらせてください",
    };
    const view = await show();
    fill("content", "夏祭りに一緒に行く");
    fill("kind", "promise");
    fill("keeper", "both");
    submit();
    await vi.waitFor(() => {
      expect(target.querySelector("[data-agreement-reason]")).not.toBeNull();
    });
    expect(target.querySelector("[data-agreement-reason]")?.getAttribute("data-agreement-reason")).toBe(
      "limit",
    );
    expect(target.querySelector("[data-agreement-guidance]")?.textContent).toContain(
      "古いものを終わらせてから",
    );
    expect(rowsOf("active-agreements")).toHaveLength(3);
    await view.dispose();
  });
});

/** その囲みの中の欄に値を入れる（**画面の別の欄と取り違えない**ように、囲みで絞る）。 */
function fillIn(scope: HTMLElement, name: string, value: string): void {
  const field = scope.querySelector(`[name="${name}"]`) as HTMLInputElement | HTMLSelectElement;
  expect(field, `${name} の欄があること`).not.toBeNull();
  field.value = value;
  field.dispatchEvent(
    new Event(field instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }),
  );
  flushSync();
}

/**
 * **提案の欄**（作業単位 A-12c-2 / ADR-0051 §決定2・5）。
 *
 * **3件にして真ん中（32）を選ぶ**（端を選ぶ変異を通さない）。
 */
describe("提案", () => {
  it("提案中だけを提案の欄に出し、種類は「未判定」。終わった欄に提案中・却下を出さない", async () => {
    const view = await show(YUI);
    expect(rowsOf("proposed-agreements").map((cells) => cells.slice(0, 3))).toEqual([
      ["キャラクターが守る", "未判定", "朝は先に起こす"],
      ["両方が守る", "未判定", "寝る前におやすみを言う"],
      ["利用者が守る", "未判定", "週に一度は手紙を書く"],
    ]);
    expect(rowsOf("active-agreements").map((cells) => cells.slice(0, 3))).toEqual([
      ["利用者が守る", "決まり", "敬語を使わない"],
    ]);
    expect(rowsOf("ended-agreements").map((cells) => cells.slice(0, 4))).toEqual([
      ["キャラクターが守る", "約束", "果たした約束", "果たした"],
    ]);
    // **時刻は読み手の時間帯で**（`Pacific/Marquesas` = UTC−09:30 / 直書き）。
    expect(rowsOf("proposed-agreements")[1]?.[3]).toBe("2026-10-21 14:30:00 (UTC-09:30)");
    // **却下は記録の側**（「無かった → 提案中」は「書いた」と言わない）。
    // **いま却下の取り決めの行は、種類も「未判定」**（保存の「決まり」は判定ではない）。
    expect(rowsOf("agreement-transitions").map((cells) => cells.slice(1))).toEqual([
      ["利用者が守る", "未判定", "捨てた提案", "提案中", "却下"],
      ["利用者が守る", "未判定", "捨てた提案", "（提案された）", "提案中"],
    ]);
    await view.dispose();
  });

  it("提案が無ければ、無いと言う", async () => {
    const view = await show(MIO);
    expect(rowsOf("proposed-agreements")).toEqual([]);
    expect(target.querySelector("[data-no-proposals]")?.textContent?.trim()).toBe(
      "いま提案はありません。",
    );
    await view.dispose();
  });

  it("真ん中を「採る」と、そのまま（`{}`）送り、有効の表へ移る", async () => {
    const view = await show(YUI);
    buttonIn(rowById("proposed-agreements", 32), "採る").click();
    await vi.waitFor(() => {
      expect(posts).toHaveLength(1);
    });
    expect(posts[0]).toEqual({ url: "/admin/characters/yui/agreements/32/adoption", body: {} });
    await vi.waitFor(() => {
      expect(rowsOf("active-agreements").map((cells) => cells.slice(0, 3))).toEqual([
        ["利用者が守る", "決まり", "敬語を使わない"],
        ["両方が守る", "決まり", "寝る前におやすみを言う"],
      ]);
    });
    expect(rowsOf("proposed-agreements").map((cells) => cells[2])).toEqual([
      "朝は先に起こす",
      "週に一度は手紙を書く",
    ]);
    expect(target.querySelector('p.notice[role="status"]')?.textContent?.trim()).toBe(
      "提案を採りました。次のターンから本文に載ります。",
    );
    // **採った提案の記録は、最終的な種類で出す**（もう提案中ではない —— 「未判定」にしない）。
    expect(rowsOf("agreement-transitions")[0]?.slice(1)).toEqual([
      "両方が守る",
      "決まり",
      "寝る前におやすみを言う",
      "提案中",
      "有効",
    ]);
    await view.dispose();
  });

  it("真ん中を「直して採る」と、直した中身・種類・守る側で送り、その中身で有効になる", async () => {
    const view = await show(YUI);
    buttonIn(rowById("proposed-agreements", 32), "直して採る").click();
    flushSync();
    const editor = target.querySelector<HTMLElement>('[data-proposal-editor="32"]');
    expect(editor, "直す欄が出ること").not.toBeNull();
    const scope = editor as HTMLElement;
    // **中身と守る側は提案から写し、種類は選んでいない状態で始まる。**
    expect((scope.querySelector('[name="proposal-content"]') as HTMLInputElement).value).toBe(
      "寝る前におやすみを言う",
    );
    expect((scope.querySelector('[name="proposal-keeper"]') as HTMLSelectElement).value).toBe(
      "both",
    );
    expect((scope.querySelector('[name="proposal-kind"]') as HTMLSelectElement).value).toBe("");
    // **種類を選ぶまで送れない**（未判定のまま「約束」「決まり」を勝手に決めない）。
    expect(buttonIn(scope, "この内容で採る").disabled).toBe(true);

    fillIn(scope, "proposal-content", "寝る前に電話でおやすみを言う");
    fillIn(scope, "proposal-kind", "promise");
    fillIn(scope, "proposal-keeper", "character");
    buttonIn(scope, "この内容で採る").click();
    await vi.waitFor(() => {
      expect(posts).toHaveLength(1);
    });
    expect(posts[0]).toEqual({
      url: "/admin/characters/yui/agreements/32/adoption",
      body: { content: "寝る前に電話でおやすみを言う", kind: "promise", keeper: "character" },
    });
    await vi.waitFor(() => {
      expect(rowsOf("active-agreements").map((cells) => cells.slice(0, 3))).toEqual([
        ["利用者が守る", "決まり", "敬語を使わない"],
        ["キャラクターが守る", "約束", "寝る前に電話でおやすみを言う"],
      ]);
    });
    expect(target.querySelector("[data-proposal-editor]")).toBeNull();
    await view.dispose();
  });

  it("「直して採る」を「やめる」と、送らずに閉じる", async () => {
    const view = await show(YUI);
    buttonIn(rowById("proposed-agreements", 32), "直して採る").click();
    flushSync();
    buttonIn(target.querySelector('[data-proposal-editor="32"]') as HTMLElement, "やめる").click();
    flushSync();
    expect(target.querySelector("[data-proposal-editor]")).toBeNull();
    expect(posts).toEqual([]);
    await view.dispose();
  });

  it("真ん中を「捨てる」と、提案の欄から消え、終わった欄には出ず、記録に「提案中 → 却下」が残る", async () => {
    const view = await show(YUI);
    buttonIn(rowById("proposed-agreements", 32), "捨てる").click();
    await vi.waitFor(() => {
      expect(posts).toHaveLength(1);
    });
    expect(posts[0]).toEqual({
      url: "/admin/characters/yui/agreements/32/rejection",
      body: undefined,
    });
    await vi.waitFor(() => {
      expect(rowsOf("proposed-agreements").map((cells) => cells[2])).toEqual([
        "朝は先に起こす",
        "週に一度は手紙を書く",
      ]);
    });
    expect(rowsOf("ended-agreements").map((cells) => cells[2])).toEqual(["果たした約束"]);
    expect(rowsOf("active-agreements").map((cells) => cells[2])).toEqual(["敬語を使わない"]);
    expect(rowsOf("agreement-transitions")[0]?.slice(1)).toEqual([
      "両方が守る",
      "未判定",
      "寝る前におやすみを言う",
      "提案中",
      "却下",
    ]);
    await view.dispose();
  });

  /** **黙って失敗しない** —— サーバが上限で断ったら、古いものを終わらせるよう伝え、提案は残す。 */
  it("採ろうとして上限で断られたら、「古いものを終わらせてください」と伝え、提案は残る", async () => {
    decisionFailure = {
      status: 409,
      code: AGREEMENT_LIMIT_REACHED_CODE,
      message: "有効な取り決めが上限に達しています。古いものを終わらせてください",
    };
    const view = await show(YUI);
    buttonIn(rowById("proposed-agreements", 32), "採る").click();
    await vi.waitFor(() => {
      expect(target.querySelector("[data-proposal-reason]")).not.toBeNull();
    });
    expect(target.querySelector("[data-proposal-reason]")?.getAttribute("data-proposal-reason")).toBe(
      "limit",
    );
    expect(target.querySelector('[role="alert"][data-proposal-reason]')?.textContent?.trim()).toBe(
      "有効な取り決めが上限に達しています。古いものを終わらせてください",
    );
    expect(target.querySelector("[data-proposal-guidance]")?.textContent?.trim()).toBe(
      "有効な約束・決まりが上限に達しているので、採れませんでした。" +
        "古いものを「果たした」「破った」「取り消した」のどれかで終わらせてください。" +
        "終わらせてから、もう一度採れます（提案はそのまま残っています）。",
    );
    expect(rowsOf("proposed-agreements").map((cells) => cells[2])).toEqual([
      "朝は先に起こす",
      "寝る前におやすみを言う",
      "週に一度は手紙を書く",
    ]);
    expect(rowsOf("active-agreements")).toHaveLength(1);
    await view.dispose();
  });

  /**
   * **上限に達していると分かっているなら、採る2つを止めて促す**（送らせてから 409 で断らない）。
   * **捨てるは押せる**（上限に関係しない）。上限は応答の `active_limit`（ここでは 1）。
   */
  it("上限に達していたら「採る」「直して採る」を止めて促す。「捨てる」は押せる", async () => {
    limit = 1;
    const view = await show(YUI);
    const middle = rowById("proposed-agreements", 32);
    expect(buttonIn(middle, "採る").disabled).toBe(true);
    expect(buttonIn(middle, "直して採る").disabled).toBe(true);
    expect(buttonIn(middle, "捨てる").disabled).toBe(false);
    const full = target.querySelector("[data-proposal-full]");
    expect(full?.textContent?.trim()).toBe(
      "有効な約束・決まりが上限の 1 件に達しているので、いまは採れません。古いものを終わらせてください。",
    );
    // **割り込まない**（状態の案内であって、いま起きた失敗ではない）。
    expect(full?.getAttribute("role")).toBe("status");
    await view.dispose();
  });

  /** **採った後・捨てた後、読み直しが返るまで押させない**（古い一覧の上で、もう採った提案を押させない）。 */
  it.each(["採る", "捨てる"] as const)(
    "「%s」の後、一覧の読み直しが返るまで提案のボタンを押せない",
    async (action) => {
      const view = await show(YUI);
      let open = (): void => undefined;
      listHold = new Promise<void>((resolve) => {
        open = resolve;
      });
      buttonIn(rowById("proposed-agreements", 32), action).click();
      await vi.waitFor(() => {
        expect(posts).toHaveLength(1);
      });
      await new Promise((resolve) => setTimeout(resolve, 0));
      flushSync();
      // **まだ古い一覧（32 が居る）で、押せない。**
      expect(
        [...rowById("proposed-agreements", 32).querySelectorAll("button")].map(
          (button) => button.disabled,
        ),
      ).toEqual([true, true, true]);
      open();
      await vi.waitFor(() => {
        expect(
          target.querySelector('[data-section="proposed-agreements"] tr[data-agreement-id="32"]'),
        ).toBeNull();
      });
      expect(buttonIn(rowById("proposed-agreements", 31), "捨てる").disabled).toBe(false);
      await view.dispose();
    },
  );

  /**
   * **直す欄を開いた後に上限へ達したら、「この内容で採る」も止める**（行の「採る」と同じ）。
   * 上限 2・有効 1 の状態で、真ん中（32）を直しかけたまま、先頭（31）を採る。
   */
  it("直す欄を開いた後に上限へ達したら、「この内容で採る」も止める", async () => {
    limit = 2;
    const view = await show(YUI);
    buttonIn(rowById("proposed-agreements", 32), "直して採る").click();
    flushSync();
    const scope = target.querySelector<HTMLElement>('[data-proposal-editor="32"]') as HTMLElement;
    fillIn(scope, "proposal-kind", "promise");
    expect(buttonIn(scope, "この内容で採る").disabled).toBe(false);

    buttonIn(rowById("proposed-agreements", 31), "採る").click();
    await vi.waitFor(() => {
      expect(rowsOf("active-agreements")).toHaveLength(2);
    });
    const after = target.querySelector<HTMLElement>('[data-proposal-editor="32"]');
    expect(after, "直す欄は開いたまま（打った字を消さない）").not.toBeNull();
    expect(buttonIn(after as HTMLElement, "この内容で採る").disabled).toBe(true);
    expect((after?.querySelector('[name="proposal-kind"]') as HTMLSelectElement).value).toBe(
      "promise",
    );
    await view.dispose();
  });
});
