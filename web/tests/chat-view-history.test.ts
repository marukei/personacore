// @vitest-environment jsdom
/**
 * **会話の画面が、過去のやり取りを出す**（作業単位 D-12c-2 / Issue origin#4）。
 *
 * # ここで見るもの（**配線** —— 画面 → 読む口 → 部品）
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 一覧の部品が何を描くか・選ぶと何を伝えるか | `session-picker.test.ts`（部品だけ） |
 * | 読む口の URL の組み立て | `api-paths.test.ts` |
 * | **開いたら続ける会話の過去が出る／遡る／選ぶと送り先が変わる／本文の整え方** | **ここ** |
 *
 * **部品の被覆と配線を分けて見る**（CLAUDE.md「配線だけの場所は部品の被覆とは別に見る」）——
 * 部品が正しくても、画面がそれを別の会話へ繋げば壊れる。
 *
 * # 代役のサーバ
 *
 * `fetch` を差し替え、**ページの切り方まで本物と同じ規則で返す**（`before` より前を、
 * 古い順に、1件多く見て `has_older`）。**1ページは 5 件**（本物の既定 50 を写さない ——
 * 画面は件数を送らないので、何件で切られても同じ振る舞いになるべき）。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ChatView from "../src/lib/ChatView.svelte";
import type { MessageView, SessionSummaryView } from "../src/lib/api";
import { noProposalsFor } from "./support/no-proposals";
import { readerInZone } from "./support/zone";

// 一覧を開く測定は時刻を描く（`SessionPicker.svelte`）。**配線の権威は `timestamp-wiring.test.ts`。**
readerInZone("Pacific/Marquesas");

const AOI = "aoi";
const RIN = "rin";

/** 代役の1ページの件数。 */
const PAGE = 5;

/** **強調の印を含む応答**（台詞の中と外）。 */
const EMPHASIZED = "「**朝**がいいよ」と**笑った**。";
/** **崩れた応答**（閉じない `「`）。崩れた回は丸ごと地の文で、注記が付く。 */
const BROKEN = "「**閉じない**まま終わる";

let target: HTMLElement;
/** 叩かれた URL（順に）。 */
let urls: string[];
/** `/v1/chat/completions` に送られた本文。 */
let sent: { model: string }[];
/** 送信ごとに流す応答の本文。 */
let replies: string[];
/** セッション → メッセージ（古い順）。 */
let conversations: Map<number, MessageView[]>;
/** キャラクター → 一覧（並びはサーバが決めたもの）。 */
let lists: Map<string, SessionSummaryView[]>;
/** その URL の **n 回目**の応答を、門が開くまで止めておく（URL → [n, 門]）。 */
let gates: Map<string, [number, Promise<void>]>;
/**
 * **素の識別子で送ったときの着地先**（`null` なら一覧の先頭）。
 *
 * 別のタブや外のクライアントが先に `aoi@new` で書いた状態を作る —— そのとき本物のサーバは、
 * この画面が開いた時点の先頭ではなく、**その後に書かれた会話**へ着地させる。
 */
let landOverride: number | null;
/** 代役のサーバが保存する行の次の id。 */
let nextId: number;
/** 一覧を失敗させるキャラクター。 */
let failingList: Set<string>;

function message(id: number, role: "user" | "assistant", raw: string): MessageView {
  return {
    id,
    role,
    dialogue: null,
    narration: null,
    raw_content: raw,
    created_at: "2026-09-26T00:00:00Z",
  };
}

function summary(id: number, count: number, first: string | null): SessionSummaryView {
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

/** 14 件の会話（id 101..114）。**偶数番目が応答。** 最後の応答だけ強調の印を持つ。 */
function longConversation(): MessageView[] {
  const messages: MessageView[] = [];
  for (let index = 1; index <= 14; index += 1) {
    const id = 100 + index;
    if (index % 2 === 1) {
      messages.push(message(id, "user", `発言${id}`));
    } else {
      messages.push(message(id, "assistant", index === 14 ? EMPHASIZED : `「応答${id}」`));
    }
  }
  return messages;
}

function json(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    text: () => Promise.resolve(JSON.stringify(body)),
  } as unknown as Response;
}

/** 1回ぶんの SSE（本文1件と `[DONE]`）。 */
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

/** **本物と同じ規則で1ページを切る**（`before` より前・古い順・1件多く見る）。 */
function page(handle: string, sessionId: number, before: number | null): unknown {
  const all = conversations.get(sessionId) ?? [];
  const older = before === null ? all : all.filter((m) => m.id < before);
  const messages = older.slice(Math.max(0, older.length - PAGE));
  return {
    character: handle,
    session_id: sessionId,
    messages,
    has_older: older.length > PAGE,
    total_messages: all.length,
    limit: PAGE,
  };
}

function stubFetch(): void {
  vi.stubGlobal("fetch", async (input: unknown, init?: RequestInit) => {
    const url = String(input);
    urls.push(url);
    const gate = gates.get(url);
    if (gate !== undefined && urls.filter((seen) => seen === url).length === gate[0]) {
      await gate[1];
    }

    // **約束・決まりの提案**（作業単位 A-12c-2）。**この測定は提案を持たない**（空の一覧 / `no-proposals.ts`）。
    const proposals = noProposalsFor(url, init);
    if (proposals !== null) {
      return proposals;
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
          stat_drift_mode: null,
          created_at: "2026-09-01T00:00:00Z",
        })),
        unreadable: [],
      });
    }
    if (url === "/v1/chat/completions") {
      const body = JSON.parse(String(init?.body)) as {
        model: string;
        messages: { content: string }[];
      };
      sent.push(body);
      const [handle, suffix] = body.model.split("@");
      const sessionId =
        suffix !== undefined
          ? Number(suffix)
          : (landOverride ?? lists.get(handle ?? "")?.[0]?.id ?? 41);
      const reply = replies.shift() ?? "「うん」";
      // **本物と同じく、`[DONE]` の前に往復を保存する**（`relay.rs` は保存してから `[DONE]` を流す）。
      const saved = conversations.get(sessionId) ?? [];
      saved.push(message(nextId, "user", body.messages.at(-1)?.content ?? ""));
      saved.push(message(nextId + 1, "assistant", reply));
      nextId += 2;
      conversations.set(sessionId, saved);
      return stream(`${handle ?? ""}@${sessionId}`, reply);
    }
    const path = new URL(url, "http://127.0.0.1");
    const listed = /^\/admin\/characters\/([^/]+)\/sessions$/u.exec(path.pathname);
    if (listed?.[1] !== undefined) {
      const handle = listed[1];
      if (failingList.has(handle)) {
        return json(
          { error: { message: "内部エラーが発生しました", type: "server_error" } },
          500,
        );
      }
      const sessions = lists.get(handle) ?? [];
      return json({
        character: handle,
        sessions,
        total_sessions: sessions.length,
        sessions_limit: 100,
      });
    }
    const paged = /^\/admin\/characters\/([^/]+)\/sessions\/(\d+)\/messages$/u.exec(
      path.pathname,
    );
    if (paged?.[1] !== undefined && paged[2] !== undefined) {
      const before = path.searchParams.get("before");
      return json(page(paged[1], Number(paged[2]), before === null ? null : Number(before)));
    }
    return json({ error: { message: "?", type: "invalid_request_error" } }, 404);
  });
}

/**
 * その URL の **`nth` 回目**の応答を止める。**戻り値を呼ぶと流れる。**
 *
 * 回数で選ぶのは、同じ URL が開いたときにも一度叩かれるから（一覧は2回目を止めたい）。
 */
function hold(url: string, nth = 1): () => void {
  let open: () => void = () => undefined;
  gates.set(url, [
    nth,
    new Promise<void>((resolve) => {
      open = resolve;
    }),
  ]);
  return () => open();
}

function historyIds(): number[] {
  return [...target.querySelectorAll<HTMLElement>("ol.history > li")].map((li) =>
    Number(li.dataset["messageId"]),
  );
}

function historyItems(): HTMLElement[] {
  return [...target.querySelectorAll<HTMLElement>("ol.history > li")];
}

function button(label: string): HTMLButtonElement {
  const found = [...target.querySelectorAll("button")].find(
    (candidate) => candidate.textContent?.trim() === label,
  );
  expect(found, `「${label}」の釦があること`).toBeDefined();
  return found as HTMLButtonElement;
}

function hasButton(label: string): boolean {
  return [...target.querySelectorAll("button")].some(
    (candidate) => candidate.textContent?.trim() === label,
  );
}

function outgoingModel(): string {
  return target.querySelector("p.wire code")?.textContent?.trim() ?? "";
}

function outgoingMeaning(): string {
  return target.querySelector("p.wire .meaning")?.textContent?.trim() ?? "";
}

async function waitForCharacters(): Promise<void> {
  await vi.waitFor(() => {
    const select = target.querySelector("select");
    expect(select).not.toBeNull();
    expect((select as HTMLSelectElement).value).toBe(AOI);
  });
}

async function send(text: string): Promise<void> {
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
    const last = turns[turns.length - 1] as HTMLElement;
    expect(last.textContent).not.toContain("秒");
  });
}

/** 本文の段落の**見え方**（区間の種類・文字・注記）。スコープの印も含めて比べる。 */
function looksOf(item: HTMLElement): unknown {
  const body = item.querySelector("p.body");
  expect(body).not.toBeNull();
  return {
    bodyClass: (body as HTMLElement).className,
    text: (body as HTMLElement).textContent,
    segments: [...(body as HTMLElement).querySelectorAll("span")].map((span) => [
      span.className,
      span.textContent,
    ]),
    notes: [...item.querySelectorAll("p.note")].map((note) => note.textContent?.trim()),
  };
}

beforeEach(() => {
  target = document.createElement("div");
  document.body.append(target);
  urls = [];
  sent = [];
  replies = [];
  gates = new Map();
  landOverride = null;
  nextId = 1001;
  failingList = new Set();
  conversations = new Map([
    [41, longConversation()],
    [12, [message(201, "user", "昔の話"), message(202, "assistant", "「覚えてるよ」")]],
    [7, [message(301, "user", "最初の話"), message(302, "assistant", "「はじめまして」")]],
    [90, [message(901, "user", "凛への話"), message(902, "assistant", "「凛だよ」")]],
  ]);
  lists = new Map([
    // **並びはサーバが決めたもの**（先頭が素の aoi の続ける会話）。
    [AOI, [summary(41, 14, "発言101"), summary(12, 2, "昔の話"), summary(7, 2, "最初の話")]],
    [RIN, [summary(90, 2, "凛への話")]],
  ]);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("開いたら、続ける会話の過去が出る", () => {
  it("一覧の先頭の会話の、最新のページが古い順に出る", async () => {
    const view = mount(ChatView, { target });
    await waitForCharacters();

    await vi.waitFor(() => expect(historyIds()).toEqual([110, 111, 112, 113, 114]));
    // **続ける会話は一覧の先頭**（画面で選び直さない）。
    expect(urls).toContain("/admin/characters/aoi/sessions");
    expect(urls).toContain("/admin/characters/aoi/sessions/41/messages");
    // **他の会話は読んでいない。**
    expect(urls.some((url) => url.includes("/sessions/12/"))).toBe(false);
    // 送り先は素の識別子のまま（最新の続き）。
    expect(outgoingModel()).toBe(AOI);
    expect(outgoingMeaning()).toBe("（最新の会話の続き）");
    // 役割ごとに見出しが付く。
    expect(historyItems().map((li) => li.className.split(" ")[0])).toEqual([
      "assistant",
      "user",
      "assistant",
      "user",
      "assistant",
    ]);
    expect(historyItems()[1]?.querySelector("p.body")?.textContent).toBe("発言111");

    await unmount(view, { outro: false });
  });

  it("会話が1つも無ければ、過去は出さず、メッセージも読まない", async () => {
    lists.set(AOI, []);
    const view = mount(ChatView, { target });
    await waitForCharacters();
    await vi.waitFor(() => expect(urls).toContain("/admin/characters/aoi/sessions"));
    await Promise.resolve();
    flushSync();

    expect(target.querySelector("ol.history")).toBeNull();
    expect(urls.some((url) => url.includes("/messages"))).toBe(false);
    expect(target.querySelector("p.error")).toBeNull();

    await unmount(view, { outro: false });
  });

  it("読めなければ、その旨を告げ、送信の口は開いたまま", async () => {
    failingList.add(AOI);
    const view = mount(ChatView, { target });
    await waitForCharacters();

    const alert = await vi.waitFor(() => {
      const found = [...target.querySelectorAll('p.error[role="alert"]')].find((p) =>
        p.textContent?.includes("過去のやり取りを読み込めませんでした"),
      );
      expect(found).toBeDefined();
      return found as HTMLElement;
    });
    expect(alert.textContent).toContain("内部エラーが発生しました");
    expect((target.querySelector("textarea") as HTMLTextAreaElement).disabled).toBe(false);

    await unmount(view, { outro: false });
  });
});

describe("遡り（重複も欠けも無い）", () => {
  /**
   * **3ページ**（5 / 5 / 4）。**真ん中のページ**は両端に境目を持つ唯一のページで、
   * カーソルを取り違える（末尾の id を渡す・先頭の1つ後を渡す）と、ここで重なるか欠ける。
   */
  it("「もっと前を読む」で、真ん中のページが境目を重ねずに先頭へ足される", async () => {
    const view = mount(ChatView, { target });
    await waitForCharacters();
    await vi.waitFor(() => expect(historyIds()).toEqual([110, 111, 112, 113, 114]));

    button("もっと前を読む").click();
    await vi.waitFor(() =>
      expect(historyIds()).toEqual([105, 106, 107, 108, 109, 110, 111, 112, 113, 114]),
    );
    // **カーソルは、いま画面の先頭にある行。**
    expect(urls).toContain("/admin/characters/aoi/sessions/41/messages?before=110");

    button("もっと前を読む").click();
    await vi.waitFor(() => expect(historyIds()).toHaveLength(14));
    expect(urls).toContain("/admin/characters/aoi/sessions/41/messages?before=105");

    const ids = historyIds();
    expect(ids).toEqual([...ids].sort((a, b) => a - b));
    expect(new Set(ids).size).toBe(14);
    expect(ids[0]).toBe(101);
    // **もう前は無い。** 釦ごと消える。
    expect(hasButton("もっと前を読む")).toBe(false);

    await unmount(view, { outro: false });
  });
});

describe("本文の整え方は、実行中の返事と同じ", () => {
  /**
   * **同じ生出力を、読み直した側（履歴）と、いま届いた側（送信）の両方に通して、見え方を比べる。**
   *
   * **強調の印を含む返事**（`**` が両方で落ちる）と、**崩れた返事**（両方で丸ごと地の文 ＋ 注記）。
   * 片方だけ `split` を通さない実装、保存された `dialogue` 列を読む実装は、ここで食い違う。
   */
  it("実行中の返事と、読み直した返事が同じに見える", async () => {
    conversations.set(41, [
      message(501, "user", "朝と夜どっち"),
      message(502, "assistant", EMPHASIZED),
      message(503, "user", "続けて"),
      message(504, "assistant", BROKEN),
    ]);
    replies = [EMPHASIZED, BROKEN];
    const view = mount(ChatView, { target });
    await waitForCharacters();
    await vi.waitFor(() => expect(historyIds()).toEqual([501, 502, 503, 504]));

    await send("もう一度");
    await send("さらに");

    const live = [...target.querySelectorAll<HTMLElement>("ol.turns > li.assistant")];
    const stored = historyItems().filter((li) => li.classList.contains("assistant"));
    expect(live).toHaveLength(2);
    expect(stored).toHaveLength(2);

    // 前提: 強調の印は、どちらの側にも1つも出ていない（出ていれば整え方を通っていない）。
    expect(looksOf(stored[0] as HTMLElement)).toMatchObject({ text: "「朝がいいよ」と笑った。" });
    expect(stored[0]?.querySelectorAll("span.dialogue")).toHaveLength(1);

    expect(looksOf(stored[0] as HTMLElement)).toEqual(looksOf(live[0] as HTMLElement));
    expect(looksOf(stored[1] as HTMLElement)).toEqual(looksOf(live[1] as HTMLElement));
    // 崩れた回は**生出力のまま**（印も残る）で、注記が付く。
    expect(stored[1]?.querySelector("p.body")?.textContent).toBe(BROKEN);
    expect(
      [...(stored[1] as HTMLElement).querySelectorAll("p.note")].some((note) =>
        note.textContent?.includes("分解できなかった"),
      ),
    ).toBe(true);

    await unmount(view, { outro: false });
  });
});

describe("過去の会話を選ぶ", () => {
  /**
   * **真ん中を選ぶ**（端の決め打ちを通さない / CLAUDE.md）。
   * 選んだ会話を読み、**送り先がその会話になり**、画面がそう言い、**送るとその会話へ行く。**
   */
  it("真ん中の会話を選ぶと、その会話を読み、送るとその会話の続きになる", async () => {
    const view = mount(ChatView, { target });
    await waitForCharacters();
    await vi.waitFor(() => expect(historyIds()).toHaveLength(5));

    button("過去の会話を選ぶ").click();
    const rows = await vi.waitFor(() => {
      const found = [...target.querySelectorAll<HTMLElement>("li[data-session-id]")];
      expect(found).toHaveLength(3);
      return found;
    });
    const middle = rows[1] as HTMLElement;
    expect(middle.dataset["sessionId"]).toBe("12");
    (middle.querySelector("button") as HTMLButtonElement).click();

    await vi.waitFor(() => expect(historyIds()).toEqual([201, 202]));
    expect(urls).toContain("/admin/characters/aoi/sessions/12/messages");
    // **一覧は閉じる。**
    expect(target.querySelector("li[data-session-id]")).toBeNull();
    // **送り先がその会話になり、画面がそう言う。**
    expect(outgoingModel()).toBe("aoi@12");
    expect(outgoingMeaning()).toBe("（このセッションの続き）");
    const chosenNote = target.querySelector("p[data-chosen-note]");
    expect(chosenNote?.textContent?.replace(/\s+/gu, "")).toContain(
      "送ると、この会話（aoi@12）の続きになります。",
    );

    await send("続きを話そう");
    expect(sent.map((body) => body.model)).toEqual(["aoi@12"]);

    await unmount(view, { outro: false });
  });

  it("一覧を開くたびに読み直す", async () => {
    const view = mount(ChatView, { target });
    await waitForCharacters();
    await vi.waitFor(() => expect(historyIds()).toHaveLength(5));

    button("過去の会話を選ぶ").click();
    await vi.waitFor(() =>
      expect(target.querySelectorAll("li[data-session-id]")).toHaveLength(3),
    );
    button("過去の会話の一覧を閉じる").click();
    flushSync();
    expect(target.querySelector("li[data-session-id]")).toBeNull();

    lists.set(AOI, [summary(12, 4, "昔の話"), summary(41, 14, "発言101"), summary(7, 2, "最初の話")]);
    button("過去の会話を選ぶ").click();
    await vi.waitFor(() =>
      expect(
        [...target.querySelectorAll<HTMLElement>("li[data-session-id]")].map(
          (li) => li.dataset["sessionId"],
        ),
      ).toEqual(["12", "41", "7"]),
    );
    expect(urls.filter((url) => url === "/admin/characters/aoi/sessions")).toHaveLength(3);

    await unmount(view, { outro: false });
  });
});

describe("会話を切り替えたとき", () => {
  it("新しい会話を始めると、過去は出ない", async () => {
    const view = mount(ChatView, { target });
    await waitForCharacters();
    await vi.waitFor(() => expect(historyIds()).toHaveLength(5));

    button("新しい会話を始める").click();
    flushSync();
    expect(target.querySelector("ol.history")).toBeNull();
    expect(outgoingModel()).toBe("aoi@new");

    await unmount(view, { outro: false });
  });

  /**
   * **キャラクターを変えたら、そのキャラクターの続ける会話が出る。**
   * **遅れて届いた前のキャラクターの過去で上書きしない**（別人の会話が出る）。
   */
  it("キャラクターを変えると、そのキャラクターの過去が出て、遅れた前の過去では上書きされない", async () => {
    const release = hold("/admin/characters/aoi/sessions/41/messages");
    const view = mount(ChatView, { target });
    await waitForCharacters();
    await vi.waitFor(() =>
      expect(urls).toContain("/admin/characters/aoi/sessions/41/messages"),
    );

    const select = target.querySelector("select") as HTMLSelectElement;
    select.value = RIN;
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.waitFor(() => expect(historyIds()).toEqual([901, 902]));

    release();
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
    expect(historyIds()).toEqual([901, 902]);

    await unmount(view, { outro: false });
  });
});

/**
 * 送って、**その回が終わる（中止の釦が塞がる）まで**待つ。
 *
 * `send` と違い、**この画面の回が残っていることを待たない** —— 別の会話に着地した回は、
 * 読み直した後にこの画面の回から消える（読み直した中に入る）ので。
 */
async function sendAndSettle(text: string): Promise<void> {
  const before = sent.length;
  const textarea = target.querySelector("textarea") as HTMLTextAreaElement;
  textarea.value = text;
  textarea.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
  (target.querySelector("form") as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  await vi.waitFor(() => {
    expect(sent.length).toBe(before + 1);
    expect(button("中止").disabled).toBe(true);
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

/** 画面の全文の中で、その語が何回出るか。 */
function occurrences(needle: string): number {
  return (target.textContent ?? "").split(needle).length - 1;
}

/** 告げる一言（`role="status"` の報せ）の文。 */
function notices(): string[] {
  return [...target.querySelectorAll<HTMLElement>('p.notice[role="status"]')].map(
    (p) => p.textContent?.replace(/\s+/gu, "") ?? "",
  );
}

describe("送った返事が、出している会話と別の会話に着地したとき", () => {
  /**
   * **開いたときは X（41）を出していた。** その後、別のタブが Y（12）に書いたので、
   * 素の `aoi` で送った返事は Y に保存され、`aoi@12` に固定される。
   *
   * **黙っていると、画面は X の過去の下に Y の返事を並べる**（どの会話の続きか読めない）。
   * → 一言告げ、**Y の最新ページを読み直して差し替える。**
   * **送った発言と返事は、読み直した中に入るので、ちょうど1回ずつ。**
   */
  it("その旨を告げ、着地した会話を読み直し、送った往復は1回ずつ出る", async () => {
    landOverride = 12;
    replies = ["「やあ、こっちだよ」"];
    const view = mount(ChatView, { target });
    await waitForCharacters();
    await vi.waitFor(() => expect(historyIds()).toEqual([110, 111, 112, 113, 114]));

    await sendAndSettle("こんにちは");

    await vi.waitFor(() => expect(historyIds()).toEqual([201, 202, 1001, 1002]));
    expect(urls).toContain("/admin/characters/aoi/sessions/12/messages");
    expect(outgoingModel()).toBe("aoi@12");
    expect(notices()).toEqual([
      "返事は、表示していた会話とは別の会話（aoi@12）に保存されました。その会話を読み直しました。",
    ]);
    // **2回出ない。** 読み直した中にあるので、この画面の回としては残さない。
    expect(target.querySelectorAll("ol.turns > li")).toHaveLength(0);
    expect(occurrences("こんにちは")).toBe(1);
    expect(occurrences("やあ、こっちだよ")).toBe(1);

    // **居座らせない。** 次の返事が、いま表示している会話（12）に着地したら報せは消え、読み直さない。
    const reads = urls.filter((url) => url.includes("/sessions/12/messages")).length;
    replies = ["「続けよう」"];
    await sendAndSettle("続けて");
    expect(sent.map((body) => body.model)).toEqual(["aoi", "aoi@12"]);
    expect(notices()).toEqual([]);
    expect(urls.filter((url) => url.includes("/sessions/12/messages"))).toHaveLength(reads);
    expect(historyIds()).toEqual([201, 202, 1001, 1002]);
    expect(target.querySelectorAll("ol.turns > li")).toHaveLength(2);

    await unmount(view, { outro: false });
  });

  /** **対照: 同じ会話に着地したら、何も告げず、読み直さない。** */
  it("同じ会話に着地したら、何も告げず、読み直さない", async () => {
    replies = ["「やあ」"];
    const view = mount(ChatView, { target });
    await waitForCharacters();
    await vi.waitFor(() => expect(historyIds()).toEqual([110, 111, 112, 113, 114]));

    await sendAndSettle("こんにちは");

    expect(outgoingModel()).toBe("aoi@41");
    expect(notices()).toEqual([]);
    expect(historyIds()).toEqual([110, 111, 112, 113, 114]);
    expect(
      urls.filter((url) => url.startsWith("/admin/characters/aoi/sessions/41/messages")),
    ).toHaveLength(1);
    // この画面の回として1回ずつ。
    expect(target.querySelectorAll("ol.turns > li")).toHaveLength(2);
    expect(occurrences("こんにちは")).toBe(1);
    expect(occurrences("やあ")).toBe(1);

    await unmount(view, { outro: false });
  });
});

describe("遅れて届いた読み込みで、切り替えた先を上書きしない", () => {
  async function switchTo(handle: string): Promise<void> {
    const select = target.querySelector("select") as HTMLSelectElement;
    select.value = handle;
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.waitFor(() => expect(historyIds()).toEqual([901, 902]));
  }

  async function settle(): Promise<void> {
    await new Promise((resolve) => setTimeout(resolve, 0));
    flushSync();
  }

  /** **遡りの応答を止めたまま**キャラクターを変える（`loadOlder` の番号の検査）。 */
  it("遡りの応答が遅れて届いても、変えた先の過去のまま", async () => {
    const view = mount(ChatView, { target });
    await waitForCharacters();
    await vi.waitFor(() => expect(historyIds()).toHaveLength(5));

    const release = hold("/admin/characters/aoi/sessions/41/messages?before=110");
    button("もっと前を読む").click();
    await vi.waitFor(() =>
      expect(urls).toContain("/admin/characters/aoi/sessions/41/messages?before=110"),
    );
    await switchTo(RIN);

    release();
    await settle();
    expect(historyIds()).toEqual([901, 902]);

    await unmount(view, { outro: false });
  });

  /** **一覧の2回目の要求を止めたまま**キャラクターを変える（`togglePicker` の番号の検査）。 */
  it("一覧の応答が遅れて届いても、前のキャラクターの一覧を出さない", async () => {
    const view = mount(ChatView, { target });
    await waitForCharacters();
    await vi.waitFor(() => expect(historyIds()).toHaveLength(5));

    const release = hold("/admin/characters/aoi/sessions", 2);
    button("過去の会話を選ぶ").click();
    await vi.waitFor(() =>
      expect(urls.filter((url) => url === "/admin/characters/aoi/sessions")).toHaveLength(2),
    );
    await switchTo(RIN);

    release();
    await settle();
    expect(
      [...target.querySelectorAll<HTMLElement>("li[data-session-id]")].map(
        (li) => li.dataset["sessionId"],
      ),
    ).toEqual([]);
    expect(historyIds()).toEqual([901, 902]);

    await unmount(view, { outro: false });
  });
});
