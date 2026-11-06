// @vitest-environment jsdom
/**
 * **同じキャラクターと新しい会話を始められることが、会話の画面で分かる**（作業単位 D-17 / Issue origin#38）。
 *
 * # ここで見るもの（**配線** —— 釦 → 送り先 → 送信 → 固定）
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 一覧の部品が「新しい会話を始める」を置き、押されたことを伝える | `session-picker.test.ts`（部品だけ） |
 * | **押した後の一言／一覧から始めると `@new` で送られる／送った後に新しい会話の id に固定される** | **ここ** |
 * | 送り先が目に付く形か（見出しの帯に在り、注記の見た目でない） | **ここ**（下の ⚠） |
 *
 * **部品が正しくても、画面がそれを別の送り先に繋げば壊れる**（CLAUDE.md「配線だけの場所は
 * 部品の被覆とは別に見る」）。**一覧から始める測定は、先に真ん中の会話を選んで固定してから始める** ——
 * 固定が残る実装（`@12` のまま送る）を捕まえるため。
 *
 * # ⚠ 「目に付く」は jsdom では測れない
 *
 * 見えるかどうかは字の大きさ・色で決まるが、jsdom はスタイルを当てない（`src/app.css` の冒頭）。
 * ここが言えるのは**構造**だけ ——**送り先の意味が注記（`.note` / `.muted`）の見た目に入っていない**、
 * **見出しの帯（本文の面の一言と同じ帯）に在り、過去のやり取りより前に在る**。見た目は司令塔が実機で見る。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ChatView from "../src/lib/ChatView.svelte";
import type { MessageView, SessionSummaryView } from "../src/lib/api";
import { noProposalsFor } from "./support/no-proposals";
import { externalUpstreamFor } from "./support/upstream-external";
import { readerInZone } from "./support/zone";

// 一覧を開く測定は時刻を描く（`SessionPicker.svelte`）。
readerInZone("Pacific/Marquesas");

const AOI = "aoi";
/** **新しい会話が着地する id**（一覧のどの id とも違う）。 */
const FRESH_ID = 55;

/** 押した後の一言（Issue origin#38 の例のまま）。 */
const BEGUN_NOTICE =
  "次に送った発言から、新しい会話になります。前の会話は「過去の会話を選ぶ」から読めます。";

let target: HTMLElement;
let urls: string[];
/** `/v1/chat/completions` に送られた `model`。 */
let sent: string[];
let conversations: Map<number, MessageView[]>;
let sessions: SessionSummaryView[];
let nextId: number;

function message(id: number, role: "user" | "assistant", raw: string): MessageView {
  return { id, role, dialogue: null, narration: null, raw_content: raw, created_at: "2026-09-26T00:00:00Z" };
}

function summary(id: number, count: number, first: string): SessionSummaryView {
  return {
    id,
    label: null,
    created_at: "2026-09-20T00:00:00Z",
    updated_at: "2026-09-26T00:00:00Z",
    message_count: count,
    first_user_message: first,
    first_user_message_truncated: false,
  };
}

function json(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    text: () => Promise.resolve(JSON.stringify(body)),
  } as unknown as Response;
}

function stream(model: string, text: string): Response {
  const encoder = new TextEncoder();
  const events = [
    `data: ${JSON.stringify({ model, choices: [{ delta: { content: text } }] })}\n\n`,
    "data: [DONE]\n\n",
  ];
  let index = 0;
  return {
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
  } as unknown as Response;
}

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    urls.push(url);
    // **約束・決まりの提案**（作業単位 A-12c-2）。**この測定は提案を持たない**（空の一覧 / `no-proposals.ts`）。
    const proposals = noProposalsFor(url, init);
    if (proposals !== null) {
      return proposals;
    }
    const upstreamReply = externalUpstreamFor(url);
    if (upstreamReply !== null) {
      return upstreamReply;
    }
    if (url === "/admin/characters") {
      return Promise.resolve(
        json({
          characters: [
            {
              id: 1,
              handle: AOI,
              display_name: "藍生",
              base_persona: "静かな声で話す。",
              output_format_instruction: null,
              stat_scale: 5,
              stat_drift_mode: null,
              created_at: "2026-09-01T00:00:00Z",
            },
          ],
          unreadable: [],
        }),
      );
    }
    if (url === "/admin/characters/aoi/sessions") {
      return Promise.resolve(
        json({ character: AOI, sessions, total_sessions: sessions.length, sessions_limit: 100 }),
      );
    }
    const paged = /^\/admin\/characters\/aoi\/sessions\/(\d+)\/messages$/u.exec(url);
    if (paged?.[1] !== undefined) {
      const id = Number(paged[1]);
      const messages = conversations.get(id) ?? [];
      return Promise.resolve(
        json({
          character: AOI,
          session_id: id,
          messages,
          has_older: false,
          total_messages: messages.length,
          limit: 50,
        }),
      );
    }
    if (url === "/v1/chat/completions") {
      const body = JSON.parse(String(init?.body)) as { model: string; messages: { content: string }[] };
      sent.push(body.model);
      const suffix = body.model.split("@")[1];
      const landed =
        suffix === "new" ? FRESH_ID : suffix !== undefined ? Number(suffix) : (sessions[0]?.id ?? 0);
      const saved = conversations.get(landed) ?? [];
      saved.push(message(nextId, "user", body.messages.at(-1)?.content ?? ""));
      saved.push(message(nextId + 1, "assistant", "「うん」"));
      nextId += 2;
      conversations.set(landed, saved);
      return Promise.resolve(stream(`${AOI}@${landed}`, "「うん」"));
    }
    return Promise.resolve(json({ error: { message: "?", type: "invalid_request_error" } }, 404));
  });
}

function buttons(label: string): HTMLButtonElement[] {
  return [...target.querySelectorAll<HTMLButtonElement>("button")].filter(
    (candidate) => candidate.textContent?.trim() === label,
  );
}

function button(label: string): HTMLButtonElement {
  const found = buttons(label);
  expect(found, `「${label}」の釦がちょうど1つあること`).toHaveLength(1);
  return found[0] as HTMLButtonElement;
}

function outgoingModel(): string {
  return target.querySelector("p.wire code")?.textContent?.trim() ?? "";
}

function outgoingMeaning(): string {
  return target.querySelector("p.wire .meaning")?.textContent?.trim() ?? "";
}

/** 押した後の一言（出ていなければ `null`）。 */
function begunNotice(): string | null {
  return target.querySelector("[data-new-session-notice]")?.textContent?.replace(/\s+/gu, "").trim() ?? null;
}

function historyIds(): number[] {
  return [...target.querySelectorAll<HTMLElement>("ol.history > li")].map((li) =>
    Number(li.dataset["messageId"]),
  );
}

async function waitForHistory(): Promise<void> {
  await vi.waitFor(() => {
    expect((target.querySelector("select") as HTMLSelectElement | null)?.value).toBe(AOI);
    expect(historyIds()).toEqual([411, 412]);
  });
}

async function send(text: string): Promise<void> {
  const before = sent.length;
  const textarea = target.querySelector("textarea") as HTMLTextAreaElement;
  textarea.value = text;
  textarea.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
  button("送信").click();
  await vi.waitFor(() => {
    expect(sent.length).toBe(before + 1);
    const turns = target.querySelectorAll("ol.turns > li.assistant");
    expect(turns[turns.length - 1]?.textContent).not.toContain("秒");
  });
}

/** 一覧を開き、描かれるまで待つ。 */
async function openPicker(): Promise<HTMLElement> {
  button("過去の会話を選ぶ").click();
  return vi.waitFor(() => {
    const picker = target.querySelector<HTMLElement>("section.picker");
    expect(picker).not.toBeNull();
    return picker as HTMLElement;
  });
}

beforeEach(() => {
  target = document.createElement("div");
  document.body.append(target);
  urls = [];
  sent = [];
  nextId = 1001;
  sessions = [summary(41, 2, "いちばん新しい話"), summary(12, 2, "真ん中の話"), summary(7, 2, "最初の話")];
  conversations = new Map([
    [41, [message(411, "user", "いちばん新しい話"), message(412, "assistant", "「新しいね」")]],
    [12, [message(121, "user", "真ん中の話"), message(122, "assistant", "「真ん中だね」")]],
    [7, [message(71, "user", "最初の話"), message(72, "assistant", "「はじめまして」")]],
  ]);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("見出しの「新しい会話を始める」", () => {
  it("押すと何が起きたかを言い、送ると `@new` で飛び、新しい会話の id に固定される", async () => {
    const view = mount(ChatView, { target });
    await waitForHistory();
    // **押す前は言っていない**（「A が B になる」の A）。
    expect(begunNotice()).toBeNull();
    expect(outgoingModel()).toBe(AOI);

    button("新しい会話を始める").click();
    flushSync();
    expect(begunNotice()).toBe(BEGUN_NOTICE);
    expect(outgoingModel()).toBe(`${AOI}@new`);
    expect(outgoingMeaning()).toBe("（新しい会話）");

    await send("はじめまして");
    expect(sent).toEqual([`${AOI}@new`]);
    // **送った後は、新しい会話の id に固定される**（ADR-0004）。**一言は古い話になるので消える。**
    await vi.waitFor(() => expect(outgoingModel()).toBe(`${AOI}@${FRESH_ID}`));
    expect(outgoingMeaning()).toBe("（このセッションの続き）");
    expect(begunNotice()).toBeNull();

    // **2回目は固定した id に送る**（`@new` を繰り返さない）。
    await send("続けて");
    expect(sent).toEqual([`${AOI}@new`, `${AOI}@${FRESH_ID}`]);

    await unmount(view, { outro: false });
  });

  /** **一言を居座らせない**: 会話を選んだら「次に送ると新しい会話」は偽になる。 */
  it("押した後に一覧から会話を選ぶと、一言は消え、送り先はその会話になる", async () => {
    const view = mount(ChatView, { target });
    await waitForHistory();
    button("新しい会話を始める").click();
    flushSync();
    expect(begunNotice()).toBe(BEGUN_NOTICE);

    const picker = await openPicker();
    const middle = picker.querySelector<HTMLElement>('li[data-session-id="12"]');
    (middle?.querySelector("button") as HTMLButtonElement).click();
    flushSync();

    expect(begunNotice()).toBeNull();
    expect(outgoingModel()).toBe(`${AOI}@12`);

    await unmount(view, { outro: false });
  });
});

describe("過去の会話が無いキャラクター（D-17 のレビュー）", () => {
  /** 一言の前半（後半の「前の会話は…」を持たない形）。 */
  const BEGUN_FIRST_HALF = "次に送った発言から、新しい会話になります。";

  /**
   * **会話が1つも無いなら、後半を言わない。** 1回送って会話ができた後に押し直すと、**後半が戻る**（対照）。
   * 古い「無い」を持ち越すと、2回目も後半が欠ける。
   */
  it("過去の会話が無ければ後半を出さず、会話ができた後に押し直すと後半が出る", async () => {
    sessions = [];
    const view = mount(ChatView, { target });
    await vi.waitFor(() => {
      expect((target.querySelector("select") as HTMLSelectElement | null)?.value).toBe(AOI);
      expect(urls).toContain("/admin/characters/aoi/sessions");
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    button("新しい会話を始める").click();
    flushSync();
    expect(begunNotice()).toBe(BEGUN_FIRST_HALF);

    await send("はじめまして");
    await vi.waitFor(() => expect(outgoingModel()).toBe(`${AOI}@${FRESH_ID}`));

    button("新しい会話を始める").click();
    flushSync();
    expect(begunNotice()).toBe(BEGUN_NOTICE);

    await unmount(view, { outro: false });
  });
});

describe("一覧の「新しい会話を始める」", () => {
  /**
   * **先に真ん中の会話を選んで固定してから**、一覧から新しく始める。
   * 固定を解かない配線（`@12` のまま送る）なら、ここで捕まる。
   */
  it("一覧から始めると、一覧は閉じ、一言を言い、`@new` で送られ、新しい会話の id に固定される", async () => {
    const view = mount(ChatView, { target });
    await waitForHistory();

    let picker = await openPicker();
    (picker.querySelector('li[data-session-id="12"] button') as HTMLButtonElement).click();
    await vi.waitFor(() => expect(historyIds()).toEqual([121, 122]));
    expect(outgoingModel()).toBe(`${AOI}@12`);

    picker = await openPicker();
    const inPicker = [...picker.querySelectorAll<HTMLButtonElement>("button")].filter(
      (candidate) => candidate.textContent?.trim() === "新しい会話を始める",
    );
    expect(inPicker).toHaveLength(1);
    (inPicker[0] as HTMLButtonElement).click();
    flushSync();

    expect(target.querySelector("section.picker")).toBeNull();
    expect(historyIds()).toEqual([]);
    expect(begunNotice()).toBe(BEGUN_NOTICE);
    expect(outgoingModel()).toBe(`${AOI}@new`);

    await send("別の話をしよう");
    expect(sent).toEqual([`${AOI}@new`]);
    await vi.waitFor(() => expect(outgoingModel()).toBe(`${AOI}@${FRESH_ID}`));
    expect(begunNotice()).toBeNull();

    await unmount(view, { outro: false });
  });
});

describe("送り先が目に付く形（構造だけ。冒頭の ⚠）", () => {
  it("送り先の意味は注記の見た目に入れず、本文の面の一言と同じ帯に、過去のやり取りより前に置く", async () => {
    const view = mount(ChatView, { target });
    await waitForHistory();
    button("新しい会話を始める").click();
    flushSync();

    const wire = target.querySelector<HTMLElement>("p.wire");
    expect(wire).not.toBeNull();
    const meaning = (wire as HTMLElement).querySelector<HTMLElement>(".meaning");
    expect(meaning?.tagName).toBe("STRONG");
    expect(wire?.closest(".note, .muted")).toBeNull();
    expect(meaning?.closest(".note, .muted")).toBeNull();
    // **新しい会話のときだけの印**（見た目は `.fresh` が持つ）。
    expect(wire?.classList.contains("fresh")).toBe(true);

    // **本文の面の一言と同じ帯。**
    const strip = wire?.closest<HTMLElement>(".strip");
    expect(strip).not.toBeNull();
    expect(strip?.querySelector("[data-body-face]")).not.toBeNull();
    // **やり取りの一覧より前**（下へ流れた先に埋もれない）。
    const turns = target.querySelector("ol.turns") as HTMLElement;
    expect(
      (strip as HTMLElement).compareDocumentPosition(turns) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);

    await unmount(view, { outro: false });
  });

  it("続きの会話のときは、新しい会話の印を付けない（対照）", async () => {
    const view = mount(ChatView, { target });
    await waitForHistory();
    expect(target.querySelector("p.wire")?.classList.contains("fresh")).toBe(false);
    await unmount(view, { outro: false });
  });
});
