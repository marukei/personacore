// @vitest-environment jsdom
/**
 * **会話の画面で、鍵の失敗をそれぞれそうと言う**（作業単位 A-13b-3 / Issue origin#86 / `src/lib/ChatView.svelte`）。
 *
 * # 見るもの
 *
 * | 結末 | 画面 |
 * |---|---|
 * | `upstream_unauthorized`（推論サーバが鍵を断った） | サーバの文面 ＋ **鍵の欄を確かめる案内**（上流モデルの画面への導線） |
 * | `upstream_key_not_approved`（承認が無いので送らなかった） | サーバの文面 ＋ **上流は鍵を見ていない・承認か宛先を直す案内** |
 * | それ以外（`upstream_failed`） | サーバの文面だけ（**案内を足さない**） |
 *
 * **200 の前に断られた回**（HTTP 502）と、**流れの途中のエラー**（SSE の error）の両方で見る —— どちらの経路でも
 * `code` は届く。**2つの案内は別の文で、取り違えない**（同じ画面に2回並べて、行ごとに読む）。
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

const HANDLE = "aoi";

/** サーバの文面（`error.rs` の定数の写しではない —— 届いた文面をそのまま出すことを見るため、別の綴りにする）。 */
const UNAUTHORIZED_MESSAGE = "推論サーバが鍵を受け付けませんでした（届いた文面）";
const NOT_APPROVED_MESSAGE = "承認が無いので送りませんでした（届いた文面）";
const FAILED_MESSAGE = "推論サーバとの通信に失敗しました（届いた文面）";

const UNAUTHORIZED_GUIDANCE =
  "推論サーバは鍵を受け付けませんでした。鍵が無い・違う・期限切れのほか、上流の側で鍵を有効にしていない、宛先が違う（末尾の /v1 が二重など）ことも考えられます。上流モデルの画面の、いま選んでいるプロファイルの「直す」で鍵の欄と宛先を確かめ、保存したら「切り替え直す」を押してください。";
const NOT_APPROVED_GUIDANCE =
  "personacore は鍵を送っていません。推論サーバは鍵を見ていないので、鍵を替えても直りません。宛先が http:// でローカルネットワークの外にあり、暗号化せずに送る承認が無いためです。上流モデルの画面の、いま選んでいるプロファイルの「直す」で承認するか、宛先を https:// にしてください。";

function errorBody(message: string, code: string): string {
  return JSON.stringify({ error: { message, type: "server_error", code } });
}

/** 送信ごとの結末（先頭から1つずつ取る）。`rejected` は 200 の前に断る、`stream` は流れの途中のエラー。 */
type Outcome = { kind: "rejected"; status: number; body: string } | { kind: "stream"; body: string };
let outcomes: Outcome[];
let target: HTMLElement;
let view: ReturnType<typeof mount> | null;

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
      return Promise.resolve({
        ok: true,
        status: 200,
        text: () =>
          Promise.resolve(JSON.stringify({ character: "", sessions: [], total_sessions: 0, sessions_limit: 100 })),
      } as unknown as Response);
    }
    if (url === "/admin/characters") {
      const body = JSON.stringify({
        characters: [
          {
            id: 3,
            handle: HANDLE,
            display_name: "藍生",
            base_persona: "静かな声で話す。",
            output_format_instruction: null,
            stat_scale: 5,
            created_at: "2026-08-27T00:00:00Z",
          },
        ],
        unreadable: [],
      });
      return Promise.resolve({ ok: true, status: 200, text: () => Promise.resolve(body) } as unknown as Response);
    }
    const next = outcomes.shift();
    if (next === undefined) {
      return Promise.reject(new TypeError(`想定していない送信: ${url}`));
    }
    if (next.kind === "rejected") {
      return Promise.resolve({
        ok: false,
        status: next.status,
        text: () => Promise.resolve(next.body),
      } as unknown as Response);
    }
    const events = [`data: ${next.body}\n\n`];
    let index = 0;
    return Promise.resolve({
      ok: true,
      status: 200,
      body: {
        getReader: () => ({
          read: () => {
            const chunk = events[index];
            index += 1;
            return Promise.resolve(
              chunk === undefined ? { done: true, value: undefined } : { done: false, value: encoder.encode(chunk) },
            );
          },
          cancel: () => Promise.resolve(undefined),
        }),
      },
    } as unknown as Response);
  });
}

beforeEach(() => {
  outcomes = [];
  view = null;
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(async () => {
  if (view !== null) {
    await unmount(view, { outro: false });
  }
  vi.unstubAllGlobals();
  target.remove();
});

function flat(element: Element | null | undefined): string {
  return (element?.textContent ?? "").replace(/\s+/gu, " ").trim();
}

function assistantTurns(): HTMLElement[] {
  return [...target.querySelectorAll<HTMLElement>("ol.turns > li.assistant")];
}

async function open(): Promise<void> {
  view = mount(ChatView, { target });
  await vi.waitFor(() => {
    const select = target.querySelector("select");
    expect(select).not.toBeNull();
    expect((select as HTMLSelectElement).value).toBe(HANDLE);
  });
}

/** 本文を入れて送信し、**その回の結末が画面に出るまで**待つ。 */
async function submit(text: string): Promise<HTMLElement> {
  const before = assistantTurns().length;
  const input = target.querySelector("textarea") as HTMLTextAreaElement;
  input.value = text;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
  (target.querySelector("form") as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  return vi.waitFor(() => {
    const turns = assistantTurns();
    expect(turns.length).toBe(before + 1);
    const last = turns[turns.length - 1] as HTMLElement;
    expect(last.querySelector("p.error")).not.toBeNull();
    return last;
  });
}

/** その行の `[エラー文, 鍵の案内（無ければ null）, 案内の導線の href（無ければ null）]`。 */
function readTurn(turn: HTMLElement): [string, string | null, string | null] {
  const guidance = turn.querySelector("p.key-guidance");
  return [
    flat(turn.querySelector("p.error")),
    guidance === null ? null : flat(guidance),
    guidance?.querySelector("a")?.getAttribute("href") ?? null,
  ];
}

describe("鍵の失敗の案内", () => {
  /**
   * **3つの結末を同じ画面に並べ、行ごとに読む**（案内を取り違える変異・全部に足す変異・どれにも足さない変異を1本で）。
   * 1回目は 200 の前に断られ、2回目は流れの途中のエラー、3回目は鍵と関係の無い失敗。
   */
  it("断られた・送らなかった・それ以外を、行ごとに別に言う", async () => {
    outcomes = [
      { kind: "rejected", status: 502, body: errorBody(UNAUTHORIZED_MESSAGE, "upstream_unauthorized") },
      { kind: "stream", body: errorBody(NOT_APPROVED_MESSAGE, "upstream_key_not_approved") },
      { kind: "rejected", status: 502, body: errorBody(FAILED_MESSAGE, "upstream_failed") },
    ];
    await open();
    const first = await submit("一つ目");
    const second = await submit("二つ目");
    const third = await submit("三つ目");
    expect([readTurn(first), readTurn(second), readTurn(third)]).toEqual([
      [UNAUTHORIZED_MESSAGE, UNAUTHORIZED_GUIDANCE, "#/upstream"],
      [NOT_APPROVED_MESSAGE, NOT_APPROVED_GUIDANCE, "#/upstream"],
      [FAILED_MESSAGE, null, null],
    ]);
  });

  /** **逆の経路でも同じ**（承認が無いのを 200 の前に、断られたのを流れの途中で）。 */
  it("経路を入れ替えても、`code` で読み分ける", async () => {
    outcomes = [
      { kind: "rejected", status: 502, body: errorBody(NOT_APPROVED_MESSAGE, "upstream_key_not_approved") },
      { kind: "stream", body: errorBody(UNAUTHORIZED_MESSAGE, "upstream_unauthorized") },
    ];
    await open();
    const first = await submit("一つ目");
    const second = await submit("二つ目");
    expect([readTurn(first)[1], readTurn(second)[1]]).toEqual([NOT_APPROVED_GUIDANCE, UNAUTHORIZED_GUIDANCE]);
  });
});
