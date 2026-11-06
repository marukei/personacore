// @vitest-environment jsdom
/**
 * **画面に描かれる台詞と地の文に、Markdown の強調の印が残っていない**（作業単位 B-46）。
 *
 * # なぜ、ここでも見るのか
 *
 * `tests/emphasis-corpus.test.ts` は **`split()` の返り値**を見ている。
 * **返り値と、画面に出る文字は別の主張である** ——
 * `ChatView.svelte` が `split()` を呼ばずに `turn.raw` をそのまま描く形にすれば、
 * あちらは全部緑のまま、**画面にはアスタリスクが出る。**
 *
 * **読むのは要素の `textContent`** で、`data-*` の印は置かない
 * （CLAUDE.md「`data-*` はテストが置いた印であって、読み手が何を見るかの権威ではない」）。
 *
 * # 受信中は落とさない（**そう決めた**）
 *
 * **`[DONE]` の前は生出力をそのまま描く。** 対になるかどうかは
 * **後ろの文字が届くまで決まらない**ので、途中で落とすと
 * **一度出た文字が消える**（`**朝` → `朝`）という見え方になる。
 * **ここでは、その状態も1本見ている**（決めたことを記録に残すため）。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ChatView from "../src/lib/ChatView.svelte";
import { noProposalsFor } from "./support/no-proposals";
import { externalUpstreamFor } from "./support/upstream-external";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

/** 画面が選ぶキャラクター。**`handle` と `display_name` を別の値にしてある。** */
const HANDLE = "aoi";
const DISPLAY_NAME = "藍生";

/** 着地するセッション。 */
const SESSION = `${HANDLE}@41`;

/**
 * **強調の印を含む応答**（`C-18` が実機で観測した形）。
 *
 * **台詞の中と外の両方**に置いてある。片方だけだと、
 * 「台詞には落とすが地の文には落とさない」実装が緑を通る。
 */
const REPLY = "「**朝**がいいよ」と**笑った**。";

/** 落とした後に画面へ出るはずの台詞と地の文。**手で書いた期待値。** */
const DIALOGUE = "「朝がいいよ」";
const NARRATION = "と笑った。";

/** 強調の印。**画面に1つも出てはいけない。** */
const MARK = "**";

/** 送る本文。**応答のどの断片とも別の文字列。** */
const DRAFT = "いつ行こうか";

/** 1回の送信で流すもの。 */
type Script = (string | null)[];

let scripts: Script[];
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

/**
 * `fetch` の代わり。**本物の `Response` を組み立てない**
 * （`chat-view-session.test.ts` と同じ理由）。
 */
function stubFetch(): void {
  const encoder = new TextEncoder();
  vi.stubGlobal("fetch", (input: unknown) => {
    const url = String(input);

    // **会話の一覧**（作業単位 D-12c-2）。**この測定は過去の会話を持たない**（空の一覧）。
    // 足さないと、画面を開いたときの一覧の要求が下の台本を1本食う。
    // **本文の面の状態**（作業単位 D-16）。**外の上流を返す**（送信を止めない）。
    // 足さないと、画面を開いたときの状態の要求が下の台本を1本食う（`/sessions` と同じ）。
    const upstreamReply = externalUpstreamFor(url);
    if (upstreamReply !== null) {
      return upstreamReply;
    }
    // **約束・決まりの提案**（作業単位 A-12c-2）。足さないと、開いたときの読みが台本を1本食う。
    const proposals = noProposalsFor(url);
    if (proposals !== null) {
      return proposals;
    }

    if (url.endsWith("/sessions")) {
      return Promise.resolve({
        ok: true,
        status: 200,
        text: () =>
          Promise.resolve(
            JSON.stringify({
              character: "",
              sessions: [],
              total_sessions: 0,
              sessions_limit: 100,
            }),
          ),
      } as unknown as Response);
    }

    if (url === "/admin/characters") {
      const body = JSON.stringify({
        characters: [
          {
            id: 1,
            handle: HANDLE,
            display_name: DISPLAY_NAME,
            base_persona: "静かな声で話す。",
            output_format_instruction: null,
            stat_scale: 5,
            created_at: "2026-09-17T00:00:00Z",
          },
        ],
        unreadable: [],
      });
      return Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(body),
      } as unknown as Response);
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
              // **返らない読み取り。** その回は送信中のまま止まる。
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

/** 相手の側の行だけ。 */
function assistantTurns(): HTMLElement[] {
  return [...target.querySelectorAll<HTMLElement>("ol.turns > li.assistant")];
}

/** その行の本文の段落。 */
function bodyOf(turn: HTMLElement): HTMLElement {
  const body = turn.querySelector<HTMLElement>("p.body");
  expect(body).not.toBeNull();
  return body as HTMLElement;
}

beforeEach(() => {
  scripts = [];
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("画面に出る本文と、強調の印", () => {
  it("完結した回の台詞と地の文から、印が落ちている", async () => {
    // 前提: 応答に現に印があること（無ければ、この主張は何も言っていない）。
    expect(REPLY).toContain(MARK);

    scripts = [[delta(REPLY), DONE_EVENT]];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    typeAndSubmit(DRAFT);

    const body = await vi.waitFor(() => {
      const turns = assistantTurns();
      expect(turns.length).toBe(1);
      const found = bodyOf(turns[0] as HTMLElement);
      // **区間に割れるまで待つ**（受信中の `p.body` は生出力そのもの）。
      expect(found.querySelectorAll("span.dialogue").length).toBe(1);
      return found;
    });

    // **要素の `textContent` で見る**（`data-*` の印は置かない）。
    const dialogues = [...body.querySelectorAll("span.dialogue")].map(
      (span) => span.textContent,
    );
    const narrations = [...body.querySelectorAll("span.narration")].map(
      (span) => span.textContent,
    );
    expect(dialogues).toEqual([DIALOGUE]);
    expect(narrations).toEqual([NARRATION]);

    // **段落まるごと見ても印は無い**（区間の外に描かれる形も塞ぐ）。
    expect(body.textContent).not.toContain(MARK);
    expect(body.textContent).toBe(DIALOGUE + NARRATION);

    await unmount(view, { outro: false });
  });

  it("受信中は生出力のまま（印はまだ落とさない）", async () => {
    // **決めたことの記録。** 対になるかは後ろの文字が届くまで決まらないので、
    // 途中で落とすと **一度出た文字が消える**。
    scripts = [[delta(REPLY), null]];
    const view = mount(ChatView, { target });
    await waitForCharacters();

    typeAndSubmit(DRAFT);

    const body = await vi.waitFor(() => {
      const turns = assistantTurns();
      expect(turns.length).toBe(1);
      const found = bodyOf(turns[0] as HTMLElement);
      expect(found.textContent).not.toBe("");
      return found;
    });

    // **まだ割れていない**（区間の要素が1つも無い）。
    expect(body.querySelectorAll("span.dialogue").length).toBe(0);
    expect(body.textContent).toBe(REPLY);

    await unmount(view, { outro: false });
  });
});
