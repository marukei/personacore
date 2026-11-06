/**
 * **結末に至る道**（作業単位 B-8 / D-0b のレビューが列挙した穴）。
 *
 * # なぜ、いま要るのか
 *
 * `chat-abort.test.ts` は **`aborted` / `unreachable` / `truncated`** の3本しか通っていない。
 * 残る **`rejected` / `upstream-error` / `unreadable`** は、
 * **`outcomeNotice` に値を直接渡す1本（文言が別物であること）でしか見られていなかった。**
 *
 * **文言を見るのと、その結末に至る道を通るのは別の主張である。**
 * `outcomeNotice({ kind: "unreadable" })` をいくら呼んでも、
 * **`readFrame` がどの入力で `unreadable` を返すか**は1文字も測っていない。
 * ここは **`fetch` のモックから結末まで実際に通す。**
 *
 * # `SseDecoder` は外に出ていない
 *
 * `chat.ts` は `SseDecoder` を export していない（画面からは使わない）。
 * **export を増やすのは、本番の入口が観測対象から外れる形**（CLAUDE.md 4節）なので、
 * **`sendMessage` 越しに、読み取りの切れ目まで指定して測る。**
 *
 * **ここは jsdom を使わない。** `sendMessage` は DOM を触らない。
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import {
  outcomeNotice,
  sendMessage,
  type StreamCallbacks,
  type StreamOutcome,
} from "../src/lib/chat";

/** 送り先。**他のどの定数とも別の文字列。** */
const MODEL = "aoi@41";

/** 送る本文。 */
const CONTENT = "ただいま";

/** 応答として流す本文。**送った本文とも別の文字列。** */
const REPLY = "おかえりなさい";

/** 2件目の本文。**`REPLY` と取り違えたら分かるように別の文字列。** */
const REPLY_TAIL = "遅かったね";

/**
 * **エラー応答の3つの欄。全部別の文字列にしてある。**
 *
 * 同じ文字列にすると、`message` と `type` と `code` の取り違えが
 * どれも緑を通る（CLAUDE.md 4節の最頻の穴）。
 */
const ERROR_MESSAGE = "キャラクターが見つかりません";
const ERROR_TYPE = "invalid_request_error";
const ERROR_CODE = "unknown_character";

/**
 * **断られたときの状態番号。2つの検査で別の数にしてある。**
 *
 * 同じ数だと「状態番号を本当に応答から取っているか」が測れない。
 * **どちらも件数（1・2・3）とも別の数。**
 */
const REJECTED_STATUS = 503;
const UNREADABLE_BODY_STATUS = 502;

/** 終端の番兵。**`chat.ts` の定数の写し**（あちらは export していない）。 */
const DONE_EVENT = "data: [DONE]\n\n";

/** 1件ぶんの SSE イベント。 */
function event(payload: unknown): string {
  return `data: ${JSON.stringify(payload)}\n\n`;
}

/** 本文の増分1件（`model` 付き）。 */
function delta(text: string): string {
  return event({ model: MODEL, choices: [{ delta: { content: text } }] });
}

/** 本文の増分1件（**`model` は載せない**）。 */
function deltaWithoutModel(text: string): string {
  return event({ choices: [{ delta: { content: text } }] });
}

/** SSE の中を流れるエラー1件（`error.rs` の `as_event_data`）。 */
function errorEvent(): string {
  return event({
    error: { message: ERROR_MESSAGE, type: ERROR_TYPE, code: ERROR_CODE },
  });
}

/** HTTP のエラー応答の本文（`error.rs` の `ErrorBody`）。 */
function errorBody(): string {
  return JSON.stringify({
    error: { message: ERROR_MESSAGE, type: ERROR_TYPE, code: ERROR_CODE },
  });
}

/**
 * 流すもの。**1要素が「1回の読み取り」**——
 * どこで切れるかを指定できないと、`SseDecoder` の状態を作り分けられない。
 */
function streamOf(chunks: string[]): ReadableStream<Uint8Array> {
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

/** その読み取り列を流したときの結末と、届いた分。 */
async function run(chunks: string[]): Promise<{
  outcome: StreamOutcome;
  deltas: string[];
  models: string[];
}> {
  vi.stubGlobal("fetch", () => Promise.resolve(new Response(streamOf(chunks))));
  const seen = recorder();
  const outcome = await sendMessage(
    MODEL,
    CONTENT,
    seen,
    new AbortController().signal,
  );
  return { outcome, deltas: seen.deltas, models: seen.models };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("200 を送る前に断られた（rejected）", () => {
  it("サーバが言ったことをそのまま持ち帰る（3つの欄を取り違えない）", async () => {
    vi.stubGlobal("fetch", () =>
      Promise.resolve(new Response(errorBody(), { status: REJECTED_STATUS })),
    );
    const seen = recorder();

    const outcome = await sendMessage(
      MODEL,
      CONTENT,
      seen,
      new AbortController().signal,
    );

    expect(outcome.kind).toBe("rejected");
    if (outcome.kind !== "rejected") {
      return;
    }
    expect(outcome.status).toBe(REJECTED_STATUS);
    // **欄ごとに見る。** まとめて1つの値として見ると、
    // どの欄がどこから来たのかを一切見ないことになる。
    expect(outcome.detail?.message).toBe(ERROR_MESSAGE);
    expect(outcome.detail?.kind).toBe(ERROR_TYPE);
    expect(outcome.detail?.code).toBe(ERROR_CODE);

    // **画面に出るのはサーバの文言そのもの。** こちらの言い換えに落ちない。
    expect(outcomeNotice(outcome)).toBe(ERROR_MESSAGE);

    // 本文は1文字も届いていない。
    expect(seen.deltas).toEqual([]);
    expect(seen.models).toEqual([]);
  });

  it("本文が読めなければ detail は null で、文言に**その**状態番号が出る", async () => {
    vi.stubGlobal("fetch", () =>
      Promise.resolve(
        new Response("<html>proxy error</html>", {
          status: UNREADABLE_BODY_STATUS,
        }),
      ),
    );

    const outcome = await sendMessage(
      MODEL,
      CONTENT,
      recorder(),
      new AbortController().signal,
    );

    expect(outcome.kind).toBe("rejected");
    if (outcome.kind !== "rejected") {
      return;
    }
    expect(outcome.status).toBe(UNREADABLE_BODY_STATUS);
    expect(outcome.detail).toBeNull();

    const notice = outcomeNotice(outcome);
    expect(notice).toContain(String(UNREADABLE_BODY_STATUS));
    // **もう一方の状態番号を出さない。** 定数へ決め打つ変異はここで落ちる。
    expect(notice).not.toContain(String(REJECTED_STATUS));
    // **サーバの文言を騙らない**（読めていないのだから、言えることは無い）。
    expect(notice).not.toContain(ERROR_MESSAGE);
  });

  it("断られた回と、読めない応答が来た回は、別の結末になる", async () => {
    // **200 の前に断られた**（rejected）と、**200 の後に形が壊れていた**（unreadable）は
    // どちらも「サーバがおかしい」だが、**調べる場所が違う。**
    vi.stubGlobal("fetch", () =>
      Promise.resolve(new Response(errorBody(), { status: REJECTED_STATUS })),
    );
    const rejected = await sendMessage(
      MODEL,
      CONTENT,
      recorder(),
      new AbortController().signal,
    );
    vi.unstubAllGlobals();

    const broken = await run([event({ model: MODEL, choices: {} })]);

    expect(rejected.kind).toBe("rejected");
    expect(broken.outcome.kind).toBe("unreadable");
    expect(outcomeNotice(rejected)).not.toBe(outcomeNotice(broken.outcome));
  });
});

describe("中継の途中で失敗した（upstream-error）", () => {
  it("エラーが流れたら、そこで止まる（後ろの [DONE] まで読み進めない）", async () => {
    const ran = await run([
      delta(REPLY),
      errorEvent(),
      // **ここから先は読まれてはいけない。** 読み進める実装だと
      // 結末が `done` になり、**失敗した回が「保存された」ことになる。**
      deltaWithoutModel(REPLY_TAIL),
      DONE_EVENT,
    ]);

    expect(ran.outcome.kind).toBe("upstream-error");
    if (ran.outcome.kind !== "upstream-error") {
      return;
    }
    expect(ran.outcome.detail.message).toBe(ERROR_MESSAGE);
    expect(ran.outcome.detail.kind).toBe(ERROR_TYPE);
    expect(ran.outcome.detail.code).toBe(ERROR_CODE);
    expect(outcomeNotice(ran.outcome)).toBe(ERROR_MESSAGE);

    // **届いていた分は捨てない。** そして**後ろの分は届かない。**
    expect(ran.deltas).toEqual([REPLY]);
    expect(ran.models).toEqual([MODEL]);
  });

  it("1文字も来ないうちにエラーが流れた回も upstream-error", async () => {
    const ran = await run([errorEvent(), DONE_EVENT]);

    expect(ran.outcome.kind).toBe("upstream-error");
    expect(ran.deltas).toEqual([]);
    expect(ran.models).toEqual([]);
  });
});

describe("読めない応答が来た（unreadable）", () => {
  it("本文のストリームそのものが無い回", async () => {
    // `body` が `null` になるのは 204 などの「本文を持たない応答」。
    // **200 なのに読む口が無い**——ここも読めない側に落とす。
    vi.stubGlobal("fetch", () => Promise.resolve(new Response(null)));

    const outcome = await sendMessage(
      MODEL,
      CONTENT,
      recorder(),
      new AbortController().signal,
    );

    expect(outcome).toEqual({ kind: "unreadable" });
  });

  /**
   * **`readFrame` が読めないと判断する形を、1つずつ通す。**
   *
   * どれも**完成したイベント**として流す（途中で切れた形は別の状態）。
   * **`[DONE]` を後ろに置いてある**ので、
   * 「読めないのに読み飛ばして先へ進む」実装は `done` になって落ちる。
   */
  const unreadablePayloads: [label: string, data: string][] = [
    ["JSON ですらない", "{"],
    ["JSON だがオブジェクトではない", "41"],
    ["null", "null"],
    ["choices が配列ではない", JSON.stringify({ model: MODEL, choices: {} })],
    [
      "delta がオブジェクトではない",
      JSON.stringify({ model: MODEL, choices: [{ delta: REPLY }] }),
    ],
    [
      "content が文字列ではない",
      JSON.stringify({ model: MODEL, choices: [{ delta: { content: 41 } }] }),
    ],
  ];

  for (const [label, data] of unreadablePayloads) {
    it(`${label} → unreadable`, async () => {
      const ran = await run([`data: ${data}\n\n`, DONE_EVENT]);

      expect(ran.outcome).toEqual({ kind: "unreadable" });
      expect(ran.deltas).toEqual([]);
      // **`model` も渡さない。** 読めないと判断した枠から
      // 一部だけ拾って画面へ流すと、セッションの固定に嘘が混ざる。
      expect(ran.models).toEqual([]);
    });
  }

  it("**対照** —— 本文が空なだけのチャンクは読めている（unreadable にしない）", async () => {
    // **これが無いと「全部 unreadable」の実装が上の6本を満たす。**
    // どれも `model` は届く——**本文が空でも、着地したセッションは分かる。**
    const empties: string[] = [
      // 使用量チャンクの形（`relay.rs` の `is_usage_only`）。
      JSON.stringify({ model: MODEL, choices: [] }),
      // 役割だけの先頭チャンク。
      JSON.stringify({
        model: MODEL,
        choices: [{ delta: { role: "assistant" } }],
      }),
      // `content` が明示的に null。
      JSON.stringify({ model: MODEL, choices: [{ delta: { content: null } }] }),
    ];

    for (const data of empties) {
      const ran = await run([`data: ${data}\n\n`, DONE_EVENT]);
      vi.unstubAllGlobals();

      expect(ran.outcome, data).toEqual({ kind: "done" });
      // **空文字列は流さない**（`sendMessage` の `frame.text !== ""`）。
      expect(ran.deltas, data).toEqual([]);
      expect(ran.models, data).toEqual([MODEL]);
    }
  });

  it("`model` が文字列でなければ、渡さない（本文は読めている）", async () => {
    const ran = await run([
      `data: ${JSON.stringify({
        model: 41,
        choices: [{ delta: { content: REPLY } }],
      })}\n\n`,
      DONE_EVENT,
    ]);

    expect(ran.outcome).toEqual({ kind: "done" });
    expect(ran.deltas).toEqual([REPLY]);
    // **`as string` で押し込む実装はここで落ちる。**
    expect(ran.models).toEqual([]);
  });
});

describe("SSE の組み立て（読み取りの切れ目と、イベントの形）", () => {
  it("1回の読み取りに2件入っていても、順番どおりに読む", async () => {
    const ran = await run([
      delta(REPLY) + deltaWithoutModel(REPLY_TAIL) + DONE_EVENT,
    ]);

    expect(ran.outcome).toEqual({ kind: "done" });
    // **順番が入れ替わる変異はここで落ちる**（本文が逆順に組み上がる）。
    expect(ran.deltas).toEqual([REPLY, REPLY_TAIL]);
    // 2件目に `model` は無い。**載っている回だけ渡す。**
    expect(ran.models).toEqual([MODEL]);
  });

  it("1件が2回の読み取りにまたがっても、1件として読む", async () => {
    // **JSON の途中で切る。** 溜めずに流す実装だと、
    // 前半だけが `data:` として読まれ、**読めない応答**になる。
    const payload = JSON.stringify({
      model: MODEL,
      choices: [{ delta: { content: REPLY } }],
    });
    const cut = payload.indexOf("choices") + 3;
    const ran = await run([
      `data: ${payload.slice(0, cut)}`,
      `${payload.slice(cut)}\n\n${DONE_EVENT}`,
    ]);

    expect(ran.outcome).toEqual({ kind: "done" });
    expect(ran.deltas).toEqual([REPLY]);
    expect(ran.models).toEqual([MODEL]);
  });

  it("区切りの空行だけが次の読み取りに来ても、1件として読む", async () => {
    // **行は完成しているが、イベントはまだ完成していない**という状態。
    // 上の検査（行の途中で切れる）とは別の枝。
    const ran = await run([
      `data: ${JSON.stringify({
        model: MODEL,
        choices: [{ delta: { content: REPLY } }],
      })}\n`,
      `\n${DONE_EVENT}`,
    ]);

    expect(ran.outcome).toEqual({ kind: "done" });
    expect(ran.deltas).toEqual([REPLY]);
  });

  it("複数行の `data:` は1件の本文になる", async () => {
    // いまのサーバは1行しか出さないが、**仕様の側に合わせてある**（`chat.ts` の doc）。
    // **行ごとに読む実装はここで落ちる**——どちらの行も単独では JSON にならない。
    const ran = await run([
      `data: {"model": "${MODEL}",\ndata: "choices": [{"delta": {"content": "${REPLY}"}}]}\n\n`,
      DONE_EVENT,
    ]);

    expect(ran.outcome).toEqual({ kind: "done" });
    expect(ran.deltas).toEqual([REPLY]);
    expect(ran.models).toEqual([MODEL]);
  });

  it("複数行の `data:` は**改行で**繋ぐ（詰めて繋がない）", async () => {
    // **`[DONE]` を2行に割る。** 詰めて繋ぐと `[DONE]` になってしまい、
    // **完結していない回が「保存された」ことになる。**
    // 改行で繋げば `"[DO\nNE]"` で、読めない応答として止まる。
    const ran = await run(["data: [DO\ndata: NE]\n\n"]);

    expect(ran.outcome).toEqual({ kind: "unreadable" });
  });

  it("コメント行は本文に混ざらない。コメントだけの読み取りは何も起こさない", async () => {
    const ran = await run([
      // **コメント＋空行だけの読み取り。** 空行だけで「イベント1件」を作る実装だと、
      // 空の `data:` が読めない応答として立ち上がる。
      ": keep-alive\n\n",
      `: これは本文ではない\ndata: ${JSON.stringify({
        model: MODEL,
        choices: [{ delta: { content: REPLY } }],
      })}\n\n`,
      DONE_EVENT,
    ]);

    expect(ran.outcome).toEqual({ kind: "done" });
    expect(ran.deltas).toEqual([REPLY]);
  });

  it("`data:` の後ろの空白は1つだけ落とす", async () => {
    // 空白2つの `[DONE]` は `[DONE]` **ではない**（先頭に空白が残る）。
    const twoSpaces = await run(["data:  [DONE]\n\n"]);
    expect(twoSpaces.outcome).toEqual({ kind: "unreadable" });
    vi.unstubAllGlobals();

    // **対照** —— 空白0個でも読める（`data:` の直後から本文）。
    // これが無いと「先頭を必ず1文字落とす」実装が上を満たして緑になる。
    const noSpace = await run(["data:[DONE]\n\n"]);
    expect(noSpace.outcome).toEqual({ kind: "done" });
  });

  it("行末の CR は落とす（CRLF の中継でも読める）", async () => {
    const ran = await run([
      `data: ${JSON.stringify({
        model: MODEL,
        choices: [{ delta: { content: REPLY } }],
      })}\r\n\r\n`,
      "data: [DONE]\r\n\r\n",
    ]);

    expect(ran.outcome).toEqual({ kind: "done" });
    expect(ran.deltas).toEqual([REPLY]);
  });

  it("空行が来ないまま終わったイベントは捨てる（直前の完成した分は届く）", async () => {
    // **落とす側と落とさない側が同居していないと、どちらの枝も観測できない**
    // （CLAUDE.md 4節「変えなかったら元を返す枝」）。
    const ran = await run([delta(REPLY), "data: [DONE]\n"]);

    // **`[DONE]` は完成していないので、完結していない。**
    expect(ran.outcome).toEqual({ kind: "truncated" });
    expect(ran.deltas).toEqual([REPLY]);
  });
});
