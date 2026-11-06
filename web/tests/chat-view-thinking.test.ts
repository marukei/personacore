// @vitest-environment jsdom
/**
 * **画面に、上流が漏らした思考（`<think>…</think>`）が出ない**（作業単位 B-61 / ADR-0053 §決定4）。
 *
 * # なぜ、ここでも見るのか
 *
 * `tests/thinking-corpus.test.ts` は **`stripThinking()` の返り値**を見ている。
 * **返り値と、画面に出る文字は別の主張である** —— `ChatView.svelte` が `stripThinking()` を
 * 呼ばずに `turn.raw` を描けば、あちらは全部緑のまま、画面には英語の独白が出る。
 *
 * # 見ている経路は3つ
 *
 * | 経路 | どこで剥がすか |
 * |---|---|
 * | 完結した回（`[DONE]` の後） | `replyBody` |
 * | 読み直した過去の返事（`raw_content`） | `replyBody`（同じ snippet） |
 * | 受信中の回 | 受信中の `p.body` |
 *
 * **剥がして空になった回は「応答が空でした。」**、**受信中に思考しか届いていない回は
 * 「受信中…」のまま**（「応答を待っています…」の判定は生の本文で行う）。
 *
 * **読むのは要素の `textContent`** で、`data-*` の印は置かない。**ブラウザは起動しない**（jsdom だけ）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ChatView from "../src/lib/ChatView.svelte";
import { noProposalsFor } from "./support/no-proposals";
import { externalUpstreamFor } from "./support/upstream-external";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

/** 画面が選ぶキャラクター。 */
const HANDLE = "aoi";
const DISPLAY_NAME = "藍生";

/** 着地するセッション。 */
const SESSION_ID = 41;
const SESSION = `${HANDLE}@${SESSION_ID}`;

/** 思考の中身。**画面に1文字も出てはいけない。** */
const THOUGHT = "The user just came home.";

/**
 * **思考の後ろに本文が続く応答**（C-20 の多数派の形。閉じタグは空白入り）。
 * 本文は台詞と地の文の両方を持つ（片方だけ剥がす実装を区別する）。
 */
const REPLY = `<think>\n${THOUGHT}\n</think >\n\n「おかえり」と笑った。`;

/** 剥がした後に画面へ出るはずの台詞と地の文。**手で書いた期待値。** */
const DIALOGUE = "「おかえり」";
const NARRATION = "と笑った。";

/** **思考だけの応答**（B+L 5）。 */
const THOUGHT_ONLY = `<think >\n${THOUGHT}\n</think >`;

/** 空の回の注記。 */
const EMPTY_NOTE = "応答が空でした。";

/** 送る本文。**応答のどの断片とも別の文字列。** */
const DRAFT = "ただいま";

/** 1回の送信で流すもの。`null` は「返らない読み取り」（受信中のまま止める）。 */
type Script = (string | null)[];

let scripts: Script[];
/** 開いたときに出す過去の返事（`null` なら過去の会話は無い）。 */
let historyReply: string | null;
let target: HTMLElement;

/** 1件ぶんの SSE イベント。 */
function event(payload: unknown): string {
  return `data: ${JSON.stringify(payload)}\n\n`;
}

/** 本文の増分1件。 */
function delta(text: string): string {
  return event({ model: SESSION, choices: [{ delta: { content: text } }] });
}

/** 終端の番兵。 */
const DONE_EVENT = "data: [DONE]\n\n";

function json(body: unknown): Response {
  return {
    ok: true,
    status: 200,
    text: () => Promise.resolve(JSON.stringify(body)),
  } as unknown as Response;
}

function stubFetch(): void {
  const encoder = new TextEncoder();
  vi.stubGlobal("fetch", (input: unknown) => {
    const url = String(input);

    const upstreamReply = externalUpstreamFor(url);
    if (upstreamReply !== null) {
      return upstreamReply;
    }
    const proposals = noProposalsFor(url);
    if (proposals !== null) {
      return proposals;
    }

    if (url.endsWith("/sessions")) {
      const sessions =
        historyReply === null
          ? []
          : [
              {
                id: SESSION_ID,
                label: null,
                created_at: "2026-10-24T00:00:00Z",
                updated_at: "2026-10-24T00:00:00Z",
                message_count: 2,
                first_user_message: "昔の話",
                first_user_message_truncated: false,
              },
            ];
      return Promise.resolve(
        json({
          character: HANDLE,
          sessions,
          total_sessions: sessions.length,
          sessions_limit: 100,
        }),
      );
    }

    if (url.includes(`/sessions/${SESSION_ID}/messages`)) {
      const messages = [
        {
          id: 1,
          role: "user",
          dialogue: null,
          narration: null,
          raw_content: "昔の話",
          created_at: "2026-10-24T00:00:00Z",
        },
        {
          id: 2,
          role: "assistant",
          dialogue: null,
          narration: null,
          raw_content: historyReply ?? "",
          created_at: "2026-10-24T00:00:00Z",
        },
      ];
      return Promise.resolve(
        json({
          character: HANDLE,
          session_id: SESSION_ID,
          messages,
          has_older: false,
          total_messages: messages.length,
          limit: 50,
        }),
      );
    }

    if (url === "/admin/characters") {
      return Promise.resolve(
        json({
          characters: [
            {
              id: 1,
              handle: HANDLE,
              display_name: DISPLAY_NAME,
              base_persona: "静かな声で話す。",
              output_format_instruction: null,
              stat_scale: 5,
              created_at: "2026-10-24T00:00:00Z",
            },
          ],
          unreadable: [],
        }),
      );
    }

    const script = scripts.shift() ?? [];
    let index = 0;
    return Promise.resolve({
      ok: true,
      status: 200,
      body: {
        getReader: () => ({
          read: () => {
            const next = script[index];
            index += 1;
            if (next === undefined) {
              return Promise.resolve({ done: true, value: undefined });
            }
            if (next === null) {
              return new Promise<never>(() => undefined);
            }
            return Promise.resolve({ done: false, value: encoder.encode(next) });
          },
          cancel: () => Promise.resolve(undefined),
        }),
      },
    } as unknown as Response);
  });
}

/** キャラクターが1件届くまで待つ（届かないと送信の口が開かない）。 */
async function waitForCharacters(): Promise<void> {
  const select = await vi.waitFor(() => {
    const found = target.querySelector("select");
    expect(found).not.toBeNull();
    return found as HTMLSelectElement;
  });
  expect(select.value).toBe(HANDLE);
}

/** 本文を入れて送信するところまで。**結末は待たない。** */
function typeAndSubmit(text: string): void {
  const textarea = target.querySelector("textarea");
  expect(textarea).not.toBeNull();
  const input = textarea as HTMLTextAreaElement;
  input.value = text;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();

  const form = target.querySelector("form");
  expect(form).not.toBeNull();
  (form as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
}

/** 相手の側の行（この画面で送った回）。 */
function assistantTurns(): HTMLElement[] {
  return [...target.querySelectorAll<HTMLElement>("ol.turns > li.assistant")];
}

/** 区間の文字を種類ごとに。 */
function segmentsOf(body: Element, kind: "dialogue" | "narration"): (string | null)[] {
  return [...body.querySelectorAll(`span.${kind}`)].map((span) => span.textContent);
}

beforeEach(() => {
  scripts = [];
  historyReply = null;
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("画面に出る本文と、漏れた思考", () => {
  it("完結した回の本文から、思考が剥がれている", async () => {
    // 前提: 応答に現に思考があること（無ければ、この主張は何も言っていない）。
    expect(REPLY).toContain(THOUGHT);

    scripts = [[delta(REPLY), DONE_EVENT]];
    const view = mount(ChatView, { target });
    await waitForCharacters();
    typeAndSubmit(DRAFT);

    const body = await vi.waitFor(() => {
      const turns = assistantTurns();
      expect(turns.length).toBe(1);
      const found = (turns[0] as HTMLElement).querySelector("p.body");
      expect(found).not.toBeNull();
      expect((found as HTMLElement).querySelectorAll("span.dialogue").length).toBe(1);
      return found as HTMLElement;
    });

    expect(segmentsOf(body, "dialogue")).toEqual([DIALOGUE]);
    expect(segmentsOf(body, "narration")).toEqual([NARRATION]);
    expect(body.textContent).toBe(DIALOGUE + NARRATION);
    expect(target.textContent).not.toContain(THOUGHT);
    expect(target.textContent).not.toContain("think");

    await unmount(view, { outro: false });
  });

  it("思考だけの回は「応答が空でした。」になる", async () => {
    scripts = [[delta(THOUGHT_ONLY), DONE_EVENT]];
    const view = mount(ChatView, { target });
    await waitForCharacters();
    typeAndSubmit(DRAFT);

    const turn = await vi.waitFor(() => {
      const turns = assistantTurns();
      expect(turns.length).toBe(1);
      const found = turns[0] as HTMLElement;
      expect(found.textContent).toContain(EMPTY_NOTE);
      return found;
    });

    expect(turn.querySelector("p.body")).toBeNull();
    expect(target.textContent).not.toContain(THOUGHT);

    await unmount(view, { outro: false });
  });

  it("受信中も思考は剥がれ、思考しか届いていなければ本文は空で「受信中…」のまま", async () => {
    // **閉じタグがまだ来ていない**（開きタグから末尾までが思考）。
    scripts = [[delta(`<think>\n${THOUGHT}`), null]];
    const view = mount(ChatView, { target });
    await waitForCharacters();
    typeAndSubmit(DRAFT);

    const turn = await vi.waitFor(() => {
      const turns = assistantTurns();
      expect(turns.length).toBe(1);
      const found = turns[0] as HTMLElement;
      // **届いたことは画面に出る**（生の本文で判定する / ChatView の注記）。
      expect(found.textContent).toContain("受信中…");
      return found;
    });

    expect(turn.textContent).not.toContain("応答を待っています");
    expect(turn.querySelector("p.body")?.textContent).toBe("");
    expect(target.textContent).not.toContain(THOUGHT);

    await unmount(view, { outro: false });
  });

  it("受信中に閉じタグが届けば、後ろの本文から出る", async () => {
    scripts = [[delta(`<think>\n${THOUGHT}\n</think >`), delta("\n\n「おかえり」"), null]];
    const view = mount(ChatView, { target });
    await waitForCharacters();
    typeAndSubmit(DRAFT);

    const body = await vi.waitFor(() => {
      const turns = assistantTurns();
      expect(turns.length).toBe(1);
      const found = (turns[0] as HTMLElement).querySelector("p.body");
      expect(found?.textContent).toBe("「おかえり」");
      return found as HTMLElement;
    });

    // **まだ割れていない**（受信中は区間に分けない / chat-view-emphasis と同じ）。
    expect(body.querySelectorAll("span.dialogue").length).toBe(0);
    expect(target.textContent).not.toContain(THOUGHT);

    await unmount(view, { outro: false });
  });

  it("読み直した過去の返事からも、思考が剥がれている", async () => {
    historyReply = REPLY;
    const view = mount(ChatView, { target });
    await waitForCharacters();

    const item = await vi.waitFor(() => {
      const found = target.querySelector<HTMLElement>("ol.history > li.assistant");
      expect(found).not.toBeNull();
      return found as HTMLElement;
    });
    const body = item.querySelector("p.body");
    expect(body).not.toBeNull();

    expect(segmentsOf(body as Element, "dialogue")).toEqual([DIALOGUE]);
    expect(segmentsOf(body as Element, "narration")).toEqual([NARRATION]);
    expect(target.textContent).not.toContain(THOUGHT);

    await unmount(view, { outro: false });
  });
});
