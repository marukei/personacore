/**
 * **`AbortSignal` の配線** —— **中断を「接続できませんでした」と言わない。**
 *
 * `chat.ts` の `sendMessage` は失敗を2箇所で受ける（`fetch` の `catch` と、
 * 本文を読む途中の `catch`）。**どちらも `signal.aborted` を見て枝を分ける。**
 * 見なければ、**利用者が「中止」を押した回に
 * 「サーバへ接続できませんでした（personacore は動いていますか）」**と出る
 * ——自分で落とした結果を、サーバの不調として画面に出すことになる。
 *
 * **ここは jsdom を使わない。** `sendMessage` は DOM を触らないので、
 * 器を増やすと何が効いているのか分からなくなる。
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import {
  outcomeNotice,
  sendMessage,
  type StreamCallbacks,
  type StreamOutcome,
} from "../src/lib/chat";

/** 送り先。**中身は何でもよいが、他の値と同じ文字列にしない。** */
const MODEL = "aoi@41";

/** 送る本文。**`MODEL` とも、下の応答の本文とも別の文字列。** */
const CONTENT = "ただいま";

/** 応答として流す本文。**送った本文と取り違えたら分かるように別の文字列。** */
const REPLY = "おかえりなさい";

/** 1件ぶんの SSE イベント。 */
function event(payload: unknown): string {
  return `data: ${JSON.stringify(payload)}\n\n`;
}

/** 本文の増分1件。 */
function delta(text: string): string {
  return event({ model: MODEL, choices: [{ delta: { content: text } }] });
}

/**
 * 流すもの。**関数を置くとその場で呼ばれ、返した例外でストリームが壊れる**
 * ——「読んでいる最中に中止された」を作るため。
 */
type Chunk = string | (() => Error);

function streamOf(chunks: Chunk[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  let index = 0;
  return new ReadableStream<Uint8Array>({
    pull(controller) {
      const next = chunks[index];
      index += 1;
      if (next === undefined) {
        controller.close();
        return;
      }
      if (typeof next === "function") {
        controller.error(next());
        return;
      }
      controller.enqueue(encoder.encode(next));
    },
  });
}

/** 受け取った増分と `model` を控える。 */
function recorder(): StreamCallbacks & { deltas: string[]; models: string[] } {
  const deltas: string[] = [];
  const models: string[] = [];
  return {
    deltas,
    models,
    onDelta: (text) => void deltas.push(text),
    onModel: (model) => void models.push(model),
  };
}

/** 中断のときにブラウザが投げるもの。 */
function abortError(): Error {
  return new DOMException("The operation was aborted.", "AbortError");
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("中断と、繋がらなかったこと", () => {
  it("繋がらなかった回は unreachable", async () => {
    // **`signal` は最後まで立たない。** 落としたのはこちらではない。
    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("fetch failed")));
    const controller = new AbortController();

    const outcome = await sendMessage(
      MODEL,
      CONTENT,
      recorder(),
      controller.signal,
    );

    expect(outcome).toEqual({ kind: "unreachable" });
    expect(controller.signal.aborted).toBe(false);
  });

  it("送る前に中止した回は aborted（unreachable と同じ経路を通るのに、別の結末になる）", async () => {
    const controller = new AbortController();
    // **同じ `catch` に入る。** 違うのは `signal.aborted` だけ。
    vi.stubGlobal("fetch", () => {
      controller.abort();
      return Promise.reject(abortError());
    });

    const outcome = await sendMessage(
      MODEL,
      CONTENT,
      recorder(),
      controller.signal,
    );

    expect(outcome).toEqual({ kind: "aborted" });
    expect(controller.signal.aborted).toBe(true);
  });

  it("受信中に中止した回も aborted。届いていた分は画面に残る", async () => {
    const controller = new AbortController();
    const seen = recorder();
    vi.stubGlobal("fetch", () =>
      Promise.resolve(
        new Response(
          streamOf([
            delta(REPLY),
            () => {
              controller.abort();
              return abortError();
            },
          ]),
        ),
      ),
    );

    const outcome = await sendMessage(MODEL, CONTENT, seen, controller.signal);

    expect(outcome).toEqual({ kind: "aborted" });
    // **中止しても、届いた分は捨てない**（画面には残す / ADR-0019 第4節）。
    expect(seen.deltas).toEqual([REPLY]);
    expect(seen.models).toEqual([MODEL]);
  });

  it("中止していないのに `[DONE]` の前に終わった回は truncated", async () => {
    const controller = new AbortController();
    const seen = recorder();
    // **同じ「途中で終わった」でも、こちらは利用者のせいではない。**
    vi.stubGlobal("fetch", () =>
      Promise.resolve(new Response(streamOf([delta(REPLY)]))),
    );

    const outcome = await sendMessage(MODEL, CONTENT, seen, controller.signal);

    expect(outcome).toEqual({ kind: "truncated" });
    expect(controller.signal.aborted).toBe(false);
    expect(seen.deltas).toEqual([REPLY]);
  });

  it("読んでいる最中に壊れた回は、中止していないなら truncated", async () => {
    // **上の truncated は正常終了の枝（`step.done`）で作っている。**
    // **こちらは同じ `catch` の中の truncated** ——
    // 受信中の中止（aborted）と、ここを同じ結末に潰す変異は、
    // **これが無いと全部緑を通る**（D-0b のレビューが実測）。
    //
    // 実害の向きは、このファイルの冒頭が書いていることの**裏返し**:
    // 接続が切れただけの回に「中止しました」と出て、
    // **利用者は押していない中止を押したことにされる。**
    const controller = new AbortController();
    const seen = recorder();
    vi.stubGlobal("fetch", () =>
      Promise.resolve(
        new Response(
          streamOf([delta(REPLY), () => new TypeError("network error")]),
        ),
      ),
    );

    const outcome = await sendMessage(MODEL, CONTENT, seen, controller.signal);

    expect(outcome).toEqual({ kind: "truncated" });
    expect(controller.signal.aborted).toBe(false);
    expect(seen.deltas).toEqual([REPLY]);
  });

  it("4つの結末は、画面に出る文言まで別物になる", () => {
    // **「取り違えない」は結末の種類だけでは足りない。**
    // 同じ文言に落ちた瞬間、利用者から見れば取り違えたのと同じ。
    const outcomes: StreamOutcome[] = [
      { kind: "aborted" },
      { kind: "unreachable" },
      { kind: "truncated" },
      { kind: "unreadable" },
    ];

    const notices = outcomes.map((outcome) => outcomeNotice(outcome));
    expect(notices.every((notice) => notice !== null && notice !== "")).toBe(
      true,
    );
    expect(new Set(notices).size).toBe(outcomes.length);
  });
});
