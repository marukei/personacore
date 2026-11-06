// @vitest-environment jsdom
/**
 * **「中止」ボタン**（作業単位 B-6 / d /
 * [ADR-0035](../../docs/adr/0035-the-watchdog-reports-the-human-decides.md) §決定4）。
 *
 * # なぜ、いま要るのか
 *
 * B-6 は**中継の打ち切りをやめた。** 上流が黙っても personacore はもう切らず、
 * **待ち続けて `warn` を出すだけ**になる。
 * **つまり「もう待たない」を決めるのは、待っている人になった。**
 *
 * **⚠ 緩める側だけ入れて、人が止める側が壊れていたら、利用者はただ待たされる。**
 * ボタン自体は D-1 で入っているが、**それを見る見張り番が無かった。**
 *
 * # `chat-abort.test.ts` との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 中止したときに `sendMessage` が返す**結末** | `chat-abort.test.ts`（4通り） |
 * | 画面を離れたときに**後始末が落とす2つ** | `chat-view-teardown.test.ts` |
 * | **ボタンを押したら止まる／送信中でなければ押せない** | **ここ** |
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ChatView from "../src/lib/ChatView.svelte";
import { noProposalsFor } from "./support/no-proposals";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

/** 画面が選ぶキャラクター。**`handle` と `display_name` を別の値にしてある。** */
const HANDLE = "aoi";
const DISPLAY_NAME = "藍生";

/** 送る本文。 */
const DRAFT = "ただいま";

/** ボタンの綴り。**送信の側と取り違えないための定数。** */
const ABORT_LABEL = "中止";
const SEND_LABEL = "送信";

/** 1件ぶんの要求の記録。 */
interface Call {
  url: string;
  signal: AbortSignal | null;
}

let calls: Call[];
let target: HTMLElement;

/**
 * `fetch` の代わり。**本物の `Response` を組み立てない。**
 *
 * # ⚠ 本文の読みは、中断で**必ず失敗する**
 *
 * 本物の `fetch` は、`signal` が落ちると本文のリーダも拒否する。
 * **ここを「永久に未解決」にすると、押しても `sendMessage` が返らず、
 * 画面は送信中のまま**——つまり
 * 「押したら止まる」を**測れないのに緑**になる（偽陰性）。
 */
function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const signal = init?.signal ?? null;
    calls.push({ url, signal });

    // **約束・決まりの提案**（作業単位 A-12c-2）。**この測定は提案を持たない**（空の一覧 / `no-proposals.ts`）。
    const proposals = noProposalsFor(url, init);
    if (proposals !== null) {
      return proposals;
    }

    // **会話の一覧**（作業単位 D-12c-2）。**この測定は過去の会話を持たない**（空の一覧）。
    // 足さないと、画面を開いたときの一覧の要求が下の台本を1本食う。
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
            created_at: "2026-08-27T00:00:00Z",
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

    // **返し切らない応答。** 中断されるまで送信は「走っている」まま。
    return Promise.resolve({
      ok: true,
      status: 200,
      body: {
        getReader: () => ({
          read: () =>
            new Promise<never>((_resolve, reject) => {
              signal?.addEventListener("abort", () => {
                reject(new DOMException("aborted", "AbortError"));
              });
            }),
          cancel: () => Promise.resolve(undefined),
        }),
      },
    } as unknown as Response);
  });
}

/** 記録した中で最初にその URL を叩いた回。 */
function callTo(url: string): Call | undefined {
  return calls.find((call) => call.url === url);
}

/** 見出しがその綴りのボタン。**`type` では引かない**（送信も中止も同じ `<div>` に居る）。 */
function buttonLabelled(label: string): HTMLButtonElement {
  const found = [...target.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `「${label}」ボタンがあること`).toBeDefined();
  return found as HTMLButtonElement;
}

/** キャラクターが1件届くまで待つ（届かないと送信の口が開かない）。 */
async function waitForCharacters(): Promise<HTMLSelectElement> {
  const select = await vi.waitFor(() => {
    const found = target.querySelector("select");
    expect(found).not.toBeNull();
    return found as HTMLSelectElement;
  });
  expect(select.value).toBe(HANDLE);
  return select;
}

/** 本文を入れて送信する。**要求が出るまで待つ。** */
async function submit(): Promise<Call> {
  const textarea = target.querySelector("textarea");
  expect(textarea).not.toBeNull();
  const input = textarea as HTMLTextAreaElement;
  input.value = DRAFT;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();

  const form = target.querySelector("form");
  expect(form).not.toBeNull();
  (form as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );

  return await vi.waitFor(() => {
    const found = callTo("/v1/chat/completions");
    expect(found).toBeDefined();
    return found as Call;
  });
}

beforeEach(() => {
  calls = [];
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("「中止」ボタン", () => {
  it("押すと、送信中の要求が中断される", async () => {
    const view = mount(ChatView, { target });
    await waitForCharacters();

    const sending = await submit();
    const signal = sending.signal;
    expect(signal).not.toBeNull();
    // ここまでは、まだ落ちていない。
    expect((signal as AbortSignal).aborted).toBe(false);

    // **押す。**
    buttonLabelled(ABORT_LABEL).click();
    flushSync();

    expect((signal as AbortSignal).aborted).toBe(true);

    // **止まったことが画面に出る。** 落ちただけで画面が送信中のままなら、
    // 利用者にとっては「止まっていない」のと同じ。
    await vi.waitFor(() => {
      expect(target.textContent).toContain("中止しました");
    });

    await unmount(view, { outro: false });
  });

  it("送信していない間は押せない", async () => {
    const view = mount(ChatView, { target });
    await waitForCharacters();

    // 第1相: **まだ送っていない。**
    expect(buttonLabelled(ABORT_LABEL).disabled).toBe(true);

    // 第2相: **送信中は押せる。**
    // ここが無いと「常に押せない」実装が第1相と第3相を満たして緑になる。
    await submit();
    flushSync();
    expect(buttonLabelled(ABORT_LABEL).disabled).toBe(false);
    // 対照: **送信中は「送信」の方が押せなくなる**（2つのボタンの向きは逆）。
    expect(buttonLabelled(SEND_LABEL).disabled).toBe(true);

    // 第3相: **止めたら、また押せなくなる。**
    buttonLabelled(ABORT_LABEL).click();
    await vi.waitFor(() => {
      expect(buttonLabelled(ABORT_LABEL).disabled).toBe(true);
    });

    await unmount(view, { outro: false });
  });

  it("押しても、キャラクター一覧の取得までは落とさない", async () => {
    const view = mount(ChatView, { target });
    await waitForCharacters();

    const sending = await submit();
    const listing = callTo("/admin/characters");
    expect(listing).toBeDefined();

    const sendingSignal = sending.signal;
    const listingSignal = (listing as Call).signal;
    expect(sendingSignal).not.toBeNull();
    expect(listingSignal).not.toBeNull();
    // **別のハンドルであること。** 同じ物を渡していると、
    // 「中止ボタンが一覧の取得まで落とす」形になっていても分からない。
    expect(sendingSignal).not.toBe(listingSignal);

    buttonLabelled(ABORT_LABEL).click();
    flushSync();

    expect((sendingSignal as AbortSignal).aborted).toBe(true);
    // **こちらは生きたまま。** 落とすと、画面はまだ生きているのに
    // 一覧が二度と取れなくなる（`AbortController` は戻せない）。
    expect((listingSignal as AbortSignal).aborted).toBe(false);

    // 対照: **画面を離せば、こちらも落ちる。**
    // これが無いと「そもそも落ちる経路が無い」ことと区別できない。
    await unmount(view, { outro: false });
    flushSync();
    expect((listingSignal as AbortSignal).aborted).toBe(true);
  });
});
