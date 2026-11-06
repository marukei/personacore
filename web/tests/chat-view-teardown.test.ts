// @vitest-environment jsdom
/**
 * **`$effect` の後始末** —— **画面を離れたら、走っている要求を落とす。**
 *
 * `ChatView.svelte` の後始末の `$effect` は2つを落とす:
 *
 * | 中断ハンドル | 何を落とす |
 * |---|---|
 * | `controller` | **送信中の1回**（「中止」ボタンと同じもの） |
 * | `lifetime` | **画面が生きている間の取得**（キャラクター一覧） |
 *
 * **司令塔がブラウザで実測した振る舞い**を、ここで再現可能な形にする
 * （ADR-0031 §5 / 訂正の枠。**ブラウザは起動しない**——jsdom だけ）。
 *
 * # なぜ落とすことが要るのか
 *
 * 落とさないと `sendMessage` は `ChatView` が破棄された後も読み続ける。
 * **中止ボタンごと消えるので止められず、その回は保存されるのに画面のどこにも出ない。**
 * 戻ってすぐ送信すると**2本が同時に走る**（`ChatView.svelte` の doc）。
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
 * jsdom の環境で `Response` / `ReadableStream` がどちらの実装になるかは
 * 器の都合で変わりうるので、`chat.ts` と `api.ts` が実際に読む口
 * （`ok` / `status` / `text()` / `body.getReader()`）だけを持つ物を渡す。
 */
function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    calls.push({ url, signal: init?.signal ?? null });

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
            created_at: "2026-08-26T00:00:00Z",
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

    // **返し切らない応答。** 送信は「走っている」まま止まる。
    return Promise.resolve({
      ok: true,
      status: 200,
      body: {
        getReader: () => ({
          read: () => new Promise<never>(() => undefined),
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

describe("画面を離れたときの後始末", () => {
  it("送信中に破棄されると、その回の要求が中断される", async () => {
    const view = mount(ChatView, { target });

    // キャラクターが1件届くまで待つ（届かないと送信の口が開かない）。
    const select = await vi.waitFor(() => {
      const found = target.querySelector("select");
      expect(found).not.toBeNull();
      return found as HTMLSelectElement;
    });
    expect(select.value).toBe(HANDLE);

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

    const sending = await vi.waitFor(() => {
      const found = callTo("/v1/chat/completions");
      expect(found).toBeDefined();
      return found as Call;
    });
    const listing = callTo("/admin/characters");
    expect(listing).toBeDefined();

    const sendingSignal = sending.signal;
    const listingSignal = (listing as Call).signal;
    expect(sendingSignal).not.toBeNull();
    expect(listingSignal).not.toBeNull();
    // **別のハンドルであること。** 同じ物を渡していると、
    // 「中止ボタンが一覧の取得まで落とす」形になっていても分からない。
    expect(sendingSignal).not.toBe(listingSignal);

    // ここまでは、まだ何も落ちていない。
    expect((sendingSignal as AbortSignal).aborted).toBe(false);
    expect((listingSignal as AbortSignal).aborted).toBe(false);

    // **画面を離れる。**
    await unmount(view, { outro: false });
    flushSync();

    expect((sendingSignal as AbortSignal).aborted).toBe(true);
    expect((listingSignal as AbortSignal).aborted).toBe(true);
  });

  it("送信していないまま破棄されても、一覧の取得だけは落ちる", async () => {
    const view = mount(ChatView, { target });

    const listing = await vi.waitFor(() => {
      const found = callTo("/admin/characters");
      expect(found).toBeDefined();
      return found as Call;
    });
    const signal = listing.signal;
    expect(signal).not.toBeNull();
    expect((signal as AbortSignal).aborted).toBe(false);

    await unmount(view, { outro: false });
    flushSync();

    // **送信中の `controller` は `null` のまま。** そこで落ちずに
    // `lifetime` まで進むことを見ている（`controller?.abort()` の `?.`）。
    expect((signal as AbortSignal).aborted).toBe(true);
    expect(callTo("/v1/chat/completions")).toBeUndefined();
  });
});
