// @vitest-environment jsdom
/**
 * **会話の画面の、約束・決まりの提案の問いかけ**（作業単位 A-12c-2 / ADR-0051 §決定2 の決め直し）。
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 送る本文・失敗の読み分け | `agreements.test.ts` |
 * | 約束・決まりの画面の「提案」の欄 | `agreements-panel.test.ts` |
 * | **会話の画面に問いかけが出るか・どの提案へ送るか・いつ取りに行くか・送信を塞がないか** | **ここ** |
 *
 * **部品（`ProposalPrompt.svelte`）を単独で mount しない** —— 取りに行く時機は
 * `ChatView.svelte` の配線（ターンの終わりを数える）が決めるので、配線ごと測る。
 *
 * # ⚠ 3件にして真ん中を選ぶ
 *
 * 「次へ」で真ん中（32）へ動かしてから操作する（端を選ぶ変異を通さない / `D-2` のレビュー）。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ChatView from "../src/lib/ChatView.svelte";
import type { AdoptAgreementRequest, AgreementView, AgreementsView } from "../src/lib/api";
import { AGREEMENT_LIMIT_REACHED_CODE } from "../src/lib/agreements";
import { externalUpstreamFor } from "./support/upstream-external";
import { readerInZone } from "./support/zone";

readerInZone("America/New_York");

const AOI = "aoi";
const RIN = "rin";

/** 1件の取り決め。 */
function agreement(
  id: number,
  content: string,
  keeper: AgreementView["keeper"],
  state: AgreementView["state"],
): AgreementView {
  return {
    id,
    content,
    // **提案の種類はいつも `rule`**（サーバは判定しない）。有効なものは約束にしておく（取り違えが見える）。
    kind: state === "proposed" ? "rule" : "promise",
    keeper,
    state,
    source_session_id: state === "proposed" ? 41 : null,
    created_at: "2026-10-23T00:00:00Z",
  };
}

/** `aoi` の取り決め。**提案中3件（守る側も中身も互いに違う）と、有効1件・却下1件。** */
function aoiAgreements(): AgreementView[] {
  return [
    agreement(30, "敬語を使わない", "user", "active"),
    agreement(31, "朝は先に起こす", "character", "proposed"),
    agreement(34, "捨てた提案", "user", "rejected"),
    agreement(32, "寝る前におやすみを言う", "both", "proposed"),
    agreement(33, "週に一度は手紙を書く", "user", "proposed"),
  ];
}

/** `rin` の取り決め。**`aoi` と1文字も重ならない。** */
function rinAgreements(): AgreementView[] {
  return [agreement(51, "雨の日は傘を貸す", "character", "proposed")];
}

let target: HTMLElement;
/** いま載っている画面（畳んだら `null`）。 */
let mounted: ReturnType<typeof mount> | null = null;
let agreementsOf: Map<string, AgreementView[]>;
let limit: number;
/** 一覧の口を叩いた回数（キャラクターごと）。 */
let listReads: Map<string, number>;
/** 採る・捨てるに送ったもの。 */
let decisions: { url: string; body: unknown }[];
let decisionFailure: { status: number; code: string; message: string } | null;
/** 送信ごとの応答。**`[DONE]` まで流すか**を回ごとに決める。 */
let replies: { text: string; done: boolean }[];
/** 一覧の口を読んだときに呼ぶ（**裏方が提案を書いた**状態を、読みの合間に作る）。 */
let onListRead: ((handle: string, count: number) => void) | null;
/** 一覧の口の壊れ方（`null` なら正しく返す）。 */
let listFailure: "malformed" | "refused" | null;
/** `aoi` の一覧の **n 回目**の応答を、門が開くまで止める（返す順をテストが決める）。 */
let holds: Map<number, Promise<void>>;
/** 一覧の口に渡された中断の合図（読んだ順）。 */
let listSignals: (AbortSignal | null)[];

/** テストが開ける門。 */
function gate(): { opened: Promise<void>; open: () => void } {
  let open = (): void => undefined;
  const opened = new Promise<void>((resolve) => {
    open = resolve;
  });
  return { opened, open };
}

function json(body: unknown, status = 200): Promise<Response> {
  return Promise.resolve({
    ok: status >= 200 && status < 300,
    status,
    text: () => Promise.resolve(JSON.stringify(body)),
  } as unknown as Response);
}

function refuse(status: number, code: string, message: string): Promise<Response> {
  return json({ error: { message, type: "invalid_request_error", code } }, status);
}

/** 1回ぶんの SSE。**`done` が偽なら `[DONE]` を流さない**（その回は保存されない）。 */
function stream(model: string, text: string, done: boolean): Promise<Response> {
  const encoder = new TextEncoder();
  const events = [`data: ${JSON.stringify({ model, choices: [{ delta: { content: text } }] })}\n\n`];
  if (done) {
    events.push("data: [DONE]\n\n");
  }
  let index = 0;
  return Promise.resolve({
    ok: true,
    status: 200,
    body: {
      getReader: () => ({
        read: () => {
          const next = events[index];
          index += 1;
          return Promise.resolve(
            next === undefined
              ? { done: true, value: undefined }
              : { done: false, value: encoder.encode(next) },
          );
        },
        cancel: () => Promise.resolve(undefined),
      }),
    },
  } as unknown as Response);
}

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";

    const upstream = externalUpstreamFor(url);
    if (upstream !== null) {
      return upstream;
    }
    if (url === "/admin/characters") {
      return json({
        characters: [AOI, RIN].map((handle, index) => ({
          id: index + 1,
          handle,
          display_name: handle === AOI ? "藍生" : "凛",
          base_persona: "静かな声で話す。",
          output_format_instruction: null,
          stat_scale: 5,
          created_at: "2026-08-27T00:00:00Z",
        })),
        unreadable: [],
      });
    }
    if (url.endsWith("/sessions")) {
      return json({ character: "", sessions: [], total_sessions: 0, sessions_limit: 100 });
    }
    for (const handle of [AOI, RIN]) {
      const base = `/admin/characters/${handle}/agreements`;
      if (url === base && method === "GET") {
        const count = (listReads.get(handle) ?? 0) + 1;
        listReads.set(handle, count);
        listSignals.push(init?.signal ?? null);
        onListRead?.(handle, count);
        if (listFailure === "malformed") {
          return json({ character: handle });
        }
        if (listFailure === "refused") {
          return refuse(500, "internal_error", "内部エラーが発生しました");
        }
        // **中身は読まれた時点のもの**（返すのを止めても、止めている間の変化は載せない ——
        // 本物のサーバも、読んだ時点の一覧を返す）。
        const view: AgreementsView = {
          character: handle,
          agreements: [...(agreementsOf.get(handle) ?? [])],
          active_limit: limit,
          last_turn: null,
        };
        const hold = handle === AOI ? holds.get(count) : undefined;
        return hold === undefined ? json(view) : hold.then(() => json(view));
      }
      const decision = new RegExp(`^${base}/(\\d+)/(adoption|rejection)$`).exec(url);
      if (decision !== null && method === "POST") {
        const id = Number(decision[1]);
        const adopting = decision[2] === "adoption";
        const body: unknown =
          init?.body === undefined ? undefined : (JSON.parse(String(init.body)) as unknown);
        decisions.push({ url, body });
        if (decisionFailure !== null) {
          return refuse(decisionFailure.status, decisionFailure.code, decisionFailure.message);
        }
        const listed = agreementsOf.get(handle) ?? [];
        const found = listed.find((entry) => entry.id === id);
        if (found === undefined) {
          return refuse(404, "agreement_not_found", "指定された取り決めが存在しません");
        }
        const decided: AgreementView = adopting
          ? { ...found, ...(body as AdoptAgreementRequest), state: "active" }
          : { ...found, state: "rejected" };
        agreementsOf.set(
          handle,
          listed.map((entry) => (entry.id === id ? decided : entry)),
        );
        return adopting
          ? json(decided)
          : json({ character: handle, agreement_id: id, state: "rejected" });
      }
    }
    if (url === "/v1/chat/completions") {
      const reply = replies.shift() ?? { text: "うん", done: true };
      return stream(`${AOI}@41`, reply.text, reply.done);
    }
    return refuse(404, "not_found", `知らない URL: ${url}`);
  });
}

beforeEach(() => {
  target = document.createElement("div");
  document.body.append(target);
  agreementsOf = new Map([
    [AOI, aoiAgreements()],
    [RIN, rinAgreements()],
  ]);
  limit = 16;
  listReads = new Map();
  decisions = [];
  decisionFailure = null;
  replies = [];
  onListRead = null;
  listFailure = null;
  holds = new Map();
  listSignals = [];
  stubFetch();
});

afterEach(async () => {
  // **赤で途中終了した測定の画面も畳む** —— 畳まないと読み直しの予約が次の測定へ漏れ、
  // 次の測定の読みの回数を水増しする（変異 M7 で実際に起きた）。
  if (mounted !== null) {
    await unmount(mounted, { outro: false });
    mounted = null;
  }
  vi.unstubAllGlobals();
  target.remove();
});

/**
 * 画面を開き、問いかけ（か、問いかけが無いこと）が決まるまで待つ。
 *
 * **読み直しの間隔は既定（30 秒）のまま**にする —— 時機を見る測定だけが短くする。
 */
async function open(
  props: { proposalRecheckMs?: number; proposalRechecks?: number } = {},
): Promise<{ dispose: () => Promise<void> }> {
  const view = mount(ChatView, { target, props });
  mounted = view;
  await vi.waitFor(() => {
    expect(target.querySelector("select")).not.toBeNull();
    expect(listReads.get(AOI) ?? 0).toBeGreaterThan(0);
  });
  await settle();
  return {
    dispose: async () => {
      mounted = null;
      await unmount(view, { outro: false });
    },
  };
}

/** 飛んでいる読みが着くまで待つ（`fetch` の代役は即座に返すので、数回の小休止で足りる）。 */
async function settle(): Promise<void> {
  for (let round = 0; round < 5; round += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  flushSync();
}

function prompt(): HTMLElement | null {
  return target.querySelector<HTMLElement>("[data-proposal-prompt]");
}

function thePrompt(): HTMLElement {
  const found = prompt();
  expect(found, "問いかけが出ていること").not.toBeNull();
  return found as HTMLElement;
}

/** 問いかけが見せている提案の、**見える文字**（中身・注記）。`data-*` は権威ではない。 */
function shown(): { content: string; note: string; position: string } {
  const scope = thePrompt();
  return {
    content: scope.querySelector(".proposal-content")?.textContent?.trim() ?? "",
    note: scope.querySelector(".proposal-note")?.textContent?.trim() ?? "",
    position: scope.querySelector(".proposal-position")?.textContent?.trim() ?? "",
  };
}

function buttonIn(scope: HTMLElement, label: string): HTMLButtonElement {
  const found = [...scope.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `「${label}」があること`).toBeDefined();
  return found as HTMLButtonElement;
}

function fillIn(scope: HTMLElement, name: string, value: string): void {
  const field = scope.querySelector(`[name="${name}"]`) as HTMLInputElement | HTMLSelectElement;
  expect(field, `${name} の欄があること`).not.toBeNull();
  field.value = value;
  field.dispatchEvent(
    new Event(field instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }),
  );
  flushSync();
}

/** 発言を送り、その回の結末が出るまで待つ。 */
async function say(text: string): Promise<void> {
  const before = target.querySelectorAll("ol.turns > li.assistant").length;
  const textarea = target.querySelector("textarea") as HTMLTextAreaElement;
  textarea.value = text;
  textarea.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
  (target.querySelector("form") as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  await vi.waitFor(() => {
    const turns = target.querySelectorAll("ol.turns > li.assistant");
    expect(turns.length).toBe(before + 1);
    expect(turns[turns.length - 1]?.textContent).not.toContain("秒");
  });
}

describe("問いかけを出す", () => {
  it("開いたら提案を読み、1件目を問いかける（中身・守る側・種類は未判定）", async () => {
    const view = await open();
    expect(listReads.get(AOI)).toBe(1);
    const scope = thePrompt();
    expect(scope.querySelector(".proposal-question")?.textContent?.trim()).toBe(
      "今こんな約束した？",
    );
    expect(shown()).toEqual({
      content: "朝は先に起こす",
      note:
        "キャラクターが守る。" +
        "種類は未判定です（そのまま採ると「決まり」になります。違うなら「直して採る」で選べます）。",
      position: "（提案 1 / 3 件）",
    });
    expect(
      [...scope.querySelectorAll(".proposal-actions button")].map((button) =>
        button.textContent?.trim(),
      ),
    ).toEqual(["採る", "直して採る", "違う", "次へ"]);
    // **有効なもの・却下したものは問いかけない。**
    expect(scope.textContent).not.toContain("敬語を使わない");
    expect(scope.textContent).not.toContain("捨てた提案");
    await view.dispose();
  });

  it("提案が無ければ、何も出さない", async () => {
    agreementsOf.set(AOI, [agreement(30, "敬語を使わない", "user", "active")]);
    const view = await open();
    expect(prompt()).toBeNull();
    await view.dispose();
  });

  it("「次へ」で順に見せ、最後の次は最初に戻る", async () => {
    const view = await open();
    buttonIn(thePrompt(), "次へ").click();
    flushSync();
    expect(shown().content).toBe("寝る前におやすみを言う");
    expect(shown().position).toBe("（提案 2 / 3 件）");
    expect(shown().note.startsWith("両方が守る。")).toBe(true);
    buttonIn(thePrompt(), "次へ").click();
    buttonIn(thePrompt(), "次へ").click();
    flushSync();
    expect(shown().content).toBe("朝は先に起こす");
    await view.dispose();
  });

  /** **相手を変えたら、その相手の提案を読み直して出す**（帰属 —— 前の相手の提案を残さない）。 */
  it("相手を変えると、その相手の提案を問いかける", async () => {
    const view = await open();
    const select = target.querySelector("select") as HTMLSelectElement;
    select.value = RIN;
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.waitFor(() => {
      expect(listReads.get(RIN)).toBe(1);
    });
    await settle();
    expect(shown()).toEqual({
      content: "雨の日は傘を貸す",
      note:
        "キャラクターが守る。" +
        "種類は未判定です（そのまま採ると「決まり」になります。違うなら「直して採る」で選べます）。",
      position: "（提案 1 / 1 件）",
    });
    expect(thePrompt().textContent).not.toContain("朝は先に起こす");
    // **1件なら「次へ」は出さない。**
    expect(
      [...thePrompt().querySelectorAll(".proposal-actions button")].map((button) =>
        button.textContent?.trim(),
      ),
    ).toEqual(["採る", "直して採る", "違う"]);
    await view.dispose();
  });
});

describe("採る・直して採る・違う", () => {
  it("真ん中を「採る」と、その提案を `{}` で採り、残りを問いかける", async () => {
    const view = await open();
    buttonIn(thePrompt(), "次へ").click();
    flushSync();
    buttonIn(thePrompt(), "採る").click();
    await vi.waitFor(() => {
      expect(decisions).toHaveLength(1);
    });
    expect(decisions[0]).toEqual({ url: "/admin/characters/aoi/agreements/32/adoption", body: {} });
    await vi.waitFor(() => {
      expect(shown().position).toBe("（提案 1 / 2 件）");
    });
    expect(shown().content).toBe("朝は先に起こす");
    expect(thePrompt().querySelector('p.notice[role="status"]')?.textContent?.trim()).toBe(
      "提案を採りました。次のターンから本文に載ります。",
    );
    await view.dispose();
  });

  it("真ん中を「違う」と、その提案を捨てる", async () => {
    const view = await open();
    buttonIn(thePrompt(), "次へ").click();
    flushSync();
    buttonIn(thePrompt(), "違う").click();
    await vi.waitFor(() => {
      expect(decisions).toHaveLength(1);
    });
    expect(decisions[0]).toEqual({
      url: "/admin/characters/aoi/agreements/32/rejection",
      body: undefined,
    });
    await vi.waitFor(() => {
      expect(shown().position).toBe("（提案 1 / 2 件）");
    });
    expect(thePrompt().querySelector('p.notice[role="status"]')?.textContent?.trim()).toBe(
      "提案を捨てました（記録には残ります）。",
    );
    await view.dispose();
  });

  it("最後の1件を「違う」と、報せだけを残して問いかけは消える", async () => {
    agreementsOf.set(AOI, [agreement(31, "朝は先に起こす", "character", "proposed")]);
    const view = await open();
    buttonIn(thePrompt(), "違う").click();
    await vi.waitFor(() => {
      expect(thePrompt().querySelector(".proposal-content")).toBeNull();
    });
    expect(thePrompt().textContent?.trim()).toBe("提案を捨てました（記録には残ります）。");
    await view.dispose();
  });

  it("真ん中を「直して採る」と、直した中身・種類・守る側で採る", async () => {
    const view = await open();
    buttonIn(thePrompt(), "次へ").click();
    flushSync();
    buttonIn(thePrompt(), "直して採る").click();
    flushSync();
    const editor = thePrompt().querySelector<HTMLElement>('[data-proposal-editor="32"]');
    expect(editor, "直す欄が出ること").not.toBeNull();
    const scope = editor as HTMLElement;
    expect((scope.querySelector('[name="proposal-content"]') as HTMLInputElement).value).toBe(
      "寝る前におやすみを言う",
    );
    fillIn(scope, "proposal-content", "寝る前に電話でおやすみを言う");
    fillIn(scope, "proposal-kind", "promise");
    fillIn(scope, "proposal-keeper", "user");
    buttonIn(scope, "この内容で採る").click();
    await vi.waitFor(() => {
      expect(decisions).toHaveLength(1);
    });
    expect(decisions[0]).toEqual({
      url: "/admin/characters/aoi/agreements/32/adoption",
      body: { content: "寝る前に電話でおやすみを言う", kind: "promise", keeper: "user" },
    });
    await vi.waitFor(() => {
      expect(thePrompt().querySelector("[data-proposal-editor]")).toBeNull();
    });
    expect(agreementsOf.get(AOI)?.find((entry) => entry.id === 32)).toMatchObject({
      content: "寝る前に電話でおやすみを言う",
      kind: "promise",
      keeper: "user",
      state: "active",
    });
    await view.dispose();
  });

  /** **黙って失敗しない** —— 上限で断られたら「古いものを終わらせてください」と言い、終わらせる画面へ案内する。 */
  it("上限で断られたら、古いものを終わらせるよう伝え、約束・決まりの画面へ案内する", async () => {
    decisionFailure = {
      status: 409,
      code: AGREEMENT_LIMIT_REACHED_CODE,
      message: "有効な取り決めが上限に達しています。古いものを終わらせてください",
    };
    const view = await open();
    buttonIn(thePrompt(), "次へ").click();
    flushSync();
    buttonIn(thePrompt(), "採る").click();
    await vi.waitFor(() => {
      expect(thePrompt().querySelector("[data-proposal-reason]")).not.toBeNull();
    });
    const scope = thePrompt();
    expect(scope.querySelector('[role="alert"][data-proposal-reason="limit"]')?.textContent?.trim()).toBe(
      "有効な取り決めが上限に達しています。古いものを終わらせてください",
    );
    const guidance = scope.querySelector("[data-proposal-guidance]");
    expect(guidance?.textContent?.trim()).toBe(
      "有効な約束・決まりが上限に達しているので、採れませんでした。" +
        "古いものを「果たした」「破った」「取り消した」のどれかで終わらせてください。" +
        "終わらせてから、もう一度採れます（提案はそのまま残っています）。" +
        "aoi の約束・決まりの画面で終わらせられます。",
    );
    expect(guidance?.querySelector("a")?.getAttribute("href")).toBe("#/agreements/aoi");
    // **提案は残る**（失敗した提案を見せたまま）。
    expect(shown().content).toBe("寝る前におやすみを言う");
    expect(shown().position).toBe("（提案 2 / 3 件）");
    await view.dispose();
  });

  it("上限に達していると分かっていれば、採る2つを止めて案内する。「違う」は押せる", async () => {
    limit = 1;
    const view = await open();
    const scope = thePrompt();
    expect(buttonIn(scope, "採る").disabled).toBe(true);
    expect(buttonIn(scope, "直して採る").disabled).toBe(true);
    expect(buttonIn(scope, "違う").disabled).toBe(false);
    const full = scope.querySelector("[data-proposal-full]");
    expect(full?.textContent?.trim()).toBe(
      "有効な約束・決まりが上限の 1 件に達しているので、いまは採れません。古いものを終わらせてください。" +
        "aoi の約束・決まりの画面で終わらせられます。",
    );
    expect(full?.querySelector("a")?.getAttribute("href")).toBe("#/agreements/aoi");
    // **割り込まない**（状態の案内。0件から提案が現れるたびに読み上げを止めない）。
    expect(full?.getAttribute("role")).toBe("status");
    await view.dispose();
  });

  /** **直す欄を開いた後に上限へ達したら、「この内容で採る」も止める**（ターンの後の読み直しで分かった形）。 */
  it("直す欄を開いた後に上限へ達したら、「この内容で採る」も止める", async () => {
    const view = await open();
    buttonIn(thePrompt(), "直して採る").click();
    flushSync();
    const editor = (): HTMLElement =>
      thePrompt().querySelector<HTMLElement>('[data-proposal-editor="31"]') as HTMLElement;
    fillIn(editor(), "proposal-kind", "promise");
    expect(buttonIn(editor(), "この内容で採る").disabled).toBe(false);
    // **別の画面で有効なものが増え、上限に達した**（有効 1 件 = 上限 1）。
    limit = 1;
    await say("ただいま");
    await vi.waitFor(() => {
      expect(thePrompt().querySelector("[data-proposal-full]")).not.toBeNull();
    });
    expect(buttonIn(editor(), "この内容で採る").disabled).toBe(true);
    expect((editor().querySelector('[name="proposal-kind"]') as HTMLSelectElement).value).toBe(
      "promise",
    );
    await view.dispose();
  });

  /**
   * **採る・捨てるの失敗を、次のターンまで居座らせない**（作業単位 A-12c-2 のレビュー）。
   * 別の経路で先に採られて 409 → 読み直しで 0 件 → 次のターンの後に、失敗の段落が残っていない。
   */
  it("採れなかった失敗は、次のターンの後に消える", async () => {
    agreementsOf.set(AOI, [agreement(31, "朝は先に起こす", "character", "proposed")]);
    decisionFailure = {
      status: 409,
      code: "agreement_not_proposed",
      message: "指定された取り決めは提案中ではありません",
    };
    const view = await open();
    // **別の経路（別の画面）で先に採られた。**
    agreementsOf.set(AOI, [agreement(31, "朝は先に起こす", "character", "active")]);
    buttonIn(thePrompt(), "採る").click();
    await vi.waitFor(() => {
      expect(thePrompt().querySelector("[data-proposal-reason]")?.getAttribute("data-proposal-reason")).toBe(
        "decided",
      );
    });
    // **読み直しで 0 件**（問いかけの中身は消え、失敗だけが残っている）。
    await vi.waitFor(() => {
      expect(thePrompt().querySelector(".proposal-content")).toBeNull();
    });
    await say("ただいま");
    await vi.waitFor(() => {
      expect(prompt()).toBeNull();
    });
    await view.dispose();
  });
});

describe("読めなかったとき", () => {
  /** **「読めなかった」を「提案が無い」に見せない**（サーバが言ったことを出す）。 */
  it("断られたら、その旨を言う", async () => {
    listFailure = "refused";
    const view = await open();
    expect(thePrompt().querySelector("[data-proposal-load-error]")?.textContent?.trim()).toBe(
      "約束・決まりの提案を読めませんでした（次のターンの後に読み直します）: 内部エラーが発生しました",
    );
    await view.dispose();
  });

  /**
   * **形の崩れた応答で、会話の画面ごと止めない**（`ProposalPrompt.svelte` の `refresh`）。
   * 崩れた一覧を `$derived` に通すと `filter` が投げ、未処理の例外になる。
   */
  it("形の崩れた応答でも投げず、読めなかったと言い、送信はできる", async () => {
    listFailure = "malformed";
    const view = await open();
    expect(thePrompt().querySelector("[data-proposal-load-error]")?.textContent?.trim()).toBe(
      "約束・決まりの提案を読めませんでした（次のターンの後に読み直します）: 応答の形が想定と違いました",
    );
    await say("ただいま");
    expect(target.querySelector("ol.turns")?.textContent).toContain("うん");
    await view.dispose();
  });
});

describe("本文の邪魔をしない", () => {
  it("問いかけが出ている間も、入力も送信もできる", async () => {
    const view = await open();
    expect(prompt()).not.toBeNull();
    expect((target.querySelector("textarea") as HTMLTextAreaElement).disabled).toBe(false);
    await say("ただいま");
    expect(target.querySelector("ol.turns")?.textContent).toContain("うん");
    // **直している途中でも送れる。**
    buttonIn(thePrompt(), "直して採る").click();
    flushSync();
    await say("おなかすいた");
    expect(target.querySelectorAll("ol.turns > li.assistant")).toHaveLength(2);
    await view.dispose();
  });
});

/**
 * **いつ取りに行くか**（Issue の範囲 / `ProposalPrompt.svelte` の doc）。
 *
 * * 開いたとき1回。**ターンが無い間は読み直さない**（開いているだけで口を叩き続けない）。
 * * **保存されたターンが終わるたびに**、すぐ1回と、間隔を空けて決まった回数だけ。
 * * 保存されなかった回（`[DONE]` が届かない）では読みに行かない（裏方が走らない）。
 * * 画面を離れたら止める。
 *
 * **間隔は短くして回す**（既定は 30 秒）。数は「その時刻までに何回読んだか」で見る。
 */
describe("取りに行く時機", () => {
  const INTERVAL = 25;
  const RECHECKS = 2;

  /** 読み直しが全部終わるより十分長く待つ。 */
  function quiet(): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, INTERVAL * (RECHECKS + 4)));
  }

  it("ターンが無い間は、開いたときの1回だけ", async () => {
    const view = await open({ proposalRecheckMs: INTERVAL, proposalRechecks: RECHECKS });
    await quiet();
    expect(listReads.get(AOI)).toBe(1);
    await view.dispose();
  });

  it("ターンが終わるとすぐ読み、その後は決まった回数だけ読み直して止まる", async () => {
    const view = await open({ proposalRecheckMs: INTERVAL, proposalRechecks: RECHECKS });
    await say("ただいま");
    await vi.waitFor(() => {
      expect(listReads.get(AOI)).toBe(1 + 1 + RECHECKS);
    });
    await quiet();
    expect(listReads.get(AOI)).toBe(1 + 1 + RECHECKS);
    // **2回目のターンでも同じだけ**（1回きりの仕掛けではない）。
    await say("おなかすいた");
    await vi.waitFor(() => {
      expect(listReads.get(AOI)).toBe(2 * (1 + RECHECKS) + 1);
    });
    await quiet();
    expect(listReads.get(AOI)).toBe(2 * (1 + RECHECKS) + 1);
    await view.dispose();
  });

  /** **読み直しが、裏方が後から書いた提案を実際に画面へ出す**（数えるだけでなく、出たものを見る）。 */
  it("ターンの後に裏方が書いた提案は、読み直しで問いかけに出る", async () => {
    agreementsOf.set(AOI, []);
    const view = await open({ proposalRecheckMs: INTERVAL, proposalRechecks: RECHECKS });
    expect(prompt()).toBeNull();
    // **ターンの直後の1回では、まだ無い。** 2回目の読み（最初の読み直し）の前に、裏方が書く。
    onListRead = (handle, count) => {
      if (handle === AOI && count === 2) {
        agreementsOf.set(AOI, [agreement(36, "毎晩おやすみを言う", "both", "proposed")]);
      }
    };
    await say("ただいま");
    await vi.waitFor(() => {
      expect(prompt()?.querySelector(".proposal-content")?.textContent?.trim()).toBe(
        "毎晩おやすみを言う",
      );
    });
    await view.dispose();
  });

  it("保存されなかった回（`[DONE]` が届かない）では読みに行かない", async () => {
    replies = [{ text: "途中で", done: false }];
    const view = await open({ proposalRecheckMs: INTERVAL, proposalRechecks: RECHECKS });
    await say("ただいま");
    await quiet();
    expect(listReads.get(AOI)).toBe(1);
    await view.dispose();
  });

  it("画面を離れたら、読み直しを止める", async () => {
    // **間隔を長めにする** —— ターンの直後の1回を見てから離れるまでに、読み直しが走らないように。
    const slow = 200;
    const view = await open({ proposalRecheckMs: slow, proposalRechecks: RECHECKS });
    await say("ただいま");
    await vi.waitFor(
      () => {
        expect(listReads.get(AOI)).toBe(2);
      },
      { interval: 5 },
    );
    await view.dispose();
    await new Promise((resolve) => setTimeout(resolve, slow * (RECHECKS + 2)));
    expect(listReads.get(AOI)).toBe(2);
  });

  /**
   * **読み直しの予約中に次のターンが来ても、予約を重ねない**（前のターンの予約を消して数え直す）。
   * 重ねると `1 + 2 * (1 + RECHECKS)` 回になる。間隔は、2ターンを送り終えるより長くする。
   */
  it("予約中に次のターンが来たら、前の予約を消して数え直す", async () => {
    const slow = 400;
    const view = await open({ proposalRecheckMs: slow, proposalRechecks: RECHECKS });
    await say("ただいま");
    await say("おなかすいた");
    // **最初の読み直しより前に、2回目のターンが終わっている**（前提が崩れたら、この測定は意味を持たない）。
    await vi.waitFor(
      () => {
        expect(listReads.get(AOI)).toBe(1 + 2);
      },
      { interval: 5 },
    );
    await vi.waitFor(
      () => {
        expect(listReads.get(AOI)).toBe(1 + 2 + RECHECKS);
      },
      { timeout: slow * (RECHECKS + 3) },
    );
    await new Promise((resolve) => setTimeout(resolve, slow * 2));
    expect(listReads.get(AOI)).toBe(1 + 2 + RECHECKS);
    await view.dispose();
  });

  /**
   * **古い読みが新しい読みを上書きしない**（`refresh` の番号）。
   *
   * ターン直後の読み A（2回目）を止めている間に「採る」を押す。採った後の読み B（3回目）が先に返り、
   * A が後から返る。**A は採る前の一覧なので、通すと採った提案がまた問いかけに出る。**
   */
  it("採った後の読みより遅れて返った古い読みで、採った提案を出し直さない", async () => {
    const readA = gate();
    holds.set(2, readA.opened);
    const view = await open();
    await say("ただいま");
    await vi.waitFor(() => {
      expect(listReads.get(AOI)).toBe(2);
    });
    expect(shown().content).toBe("朝は先に起こす");
    buttonIn(thePrompt(), "採る").click();
    await vi.waitFor(() => {
      expect(listReads.get(AOI)).toBe(3);
    });
    await vi.waitFor(() => {
      expect(shown().content).toBe("寝る前におやすみを言う");
    });
    // **A を後から返す。**
    readA.open();
    await settle();
    expect(shown().content).toBe("寝る前におやすみを言う");
    expect(shown().position).toBe("（提案 1 / 2 件）");
    await view.dispose();
  });

  /** **採った後、読み直しが返るまでボタンを押させない**（古い一覧の上で、採った提案をもう一度押させない）。 */
  it.each(["採る", "違う"] as const)("「%s」の後、読み直しが返るまで押せない", async (action) => {
    const readAfter = gate();
    holds.set(2, readAfter.opened);
    const view = await open();
    buttonIn(thePrompt(), action).click();
    await vi.waitFor(() => {
      expect(listReads.get(AOI)).toBe(2);
    });
    await settle();
    // **まだ古い一覧（採った提案が先頭に居る）で、押せない。**
    expect(shown().content).toBe("朝は先に起こす");
    expect(
      [...thePrompt().querySelectorAll<HTMLButtonElement>(".proposal-actions button")].map(
        (button) => button.disabled,
      ),
    ).toEqual([true, true, true, true]);
    readAfter.open();
    await vi.waitFor(() => {
      expect(shown().content).toBe("寝る前におやすみを言う");
    });
    expect(buttonIn(thePrompt(), "採る").disabled).toBe(false);
    await view.dispose();
  });

  /** **画面を離れたら、飛んでいる読みを落とす**（`chat-view-teardown.test.ts` と同じ形 —— 合図を見る）。 */
  it("画面を離れたら、飛んでいる一覧の読みを落とす", async () => {
    holds.set(1, new Promise<void>(() => undefined));
    const view = await open();
    const signal = listSignals[0];
    expect(signal, "一覧の読みに中断の合図を渡していること").toBeInstanceOf(AbortSignal);
    expect(signal?.aborted).toBe(false);
    await view.dispose();
    expect(signal?.aborted).toBe(true);
  });
});
