// @vitest-environment jsdom
/**
 * **会話の画面が、本文の面の状態を読み、送信の口に結ぶ**（作業単位 D-16 / Issue origin#37 / ADR-0042 §決定3 の追記）。
 *
 * # ここで見るもの（**配線** —— 画面 → 読む口 → 送信の釦）
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 状態ごとの一言・止めるか（純粋関数） | `body-face.test.ts` |
 * | **画面が `GET /admin/upstream` を読み、一言を出し、送信を止める／戻す** | **ここ** |
 *
 * **部品が正しくても、画面がそれを送信の釦に結ばなければ壊れる**（CLAUDE.md「配線だけの場所は
 * 部品の被覆とは別に見る」）。**止めたことは「釦が塞がる」だけでなく「送信が飛ばない」で見る** ——
 * 釦は塞いでも Ctrl+Enter の口（`send()`）が開いていれば、載る前に送れてしまう。
 *
 * # 「A が B になる」を待つ形
 *
 * **載せている → 準備完了で送信が戻る**は、「釦が押せるようになる」を待つ。
 * **載せているままでは成り立たない**（止めている間は釦が塞がっている）ことを、同じ1本の前半で確かめてから待つ。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。実の上流も起こさない（`fetch` の代役）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ChatView from "../src/lib/ChatView.svelte";
import type { LaunchView, UpstreamReport, UpstreamView } from "../src/lib/api";
import { hrefFor } from "../src/lib/router";
import { noProposalsFor } from "./support/no-proposals";
import { enterAsReader, readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

const AOI = "aoi";

/** 止めている間の読み直しの間隔（テストは短く回す）。 */
const POLL_MS = 5;
/** 止めていない間の読み直しの間隔。**止めている間と違う値にする**（取り違えを測れるように）。 */
const IDLE_MS = 40;

let target: HTMLElement;
/** 叩かれた URL（順に）。 */
let urls: string[];
/** `/v1/chat/completions` に送られた `model`。 */
let sent: string[];
/** `/admin/upstream` が返すもの（いまの値。テストの途中で差し替える）。`null` は 500。 */
let upstream: UpstreamReport | null;
/**
 * `/v1/chat/completions` がどう終わるか（D-16 のレビュー）。
 *
 * | 値 | 画面の `StreamOutcome` |
 * |---|---|
 * | `done` | `done` |
 * | `stream-error` | `upstream-error`（200 の後、SSE の中のエラー） |
 * | `unavailable` | `rejected`（200 の前の 503。上流が落ちているときの形） |
 */
let completion: "done" | "stream-error" | "unavailable";
/** 送信が届いた瞬間に呼ぶ（差し替えが同時に起きた状態を作る）。 */
let onSend: () => void;
/**
 * **まだ離れていない画面。** `afterEach` が必ず離れる。
 *
 * **途中で赤になったテストは `leave` まで届かない。** 離れないまま次のテストへ進むと、
 * **その画面の読み直しが次のテストの `urls` に積まれ**、数を見るテスト（後片付け・間隔）が
 * 無関係に赤くなる（変異検査で実際に起きた / 作業単位 D-16）。
 */
let live: Set<ReturnType<typeof mount>>;

function launch(overrides: Partial<LaunchView>): LaunchView {
  return { state: "external", since: null, answered_name: null, reason: null, log: null, ...overrides };
}

function view(face: string, launched: LaunchView): UpstreamView {
  return {
    face,
    endpoint: face === "main" ? "http://127.0.0.1:43001" : "http://127.0.0.1:43002",
    model: face === "main" ? "main-model" : "sub-model",
    answered_name: null,
    queried_context_tokens: null,
    answered_by: null,
    unauthorized: false,
    checked_at: null,
    check_result: "not_checked",
    measured: { checked: 0, matched: 0, drifted_other: 0, drifted_absent: 0, unanswered: 0 },
    launch: launched,
  };
}

/**
 * **選んだプロファイルの名前は、どの状態でも在る**（作業単位 F-10b）——起こすボタンを出さないことを、
 * 名前が無いからではなく状態で決めていると見るため。**見本の場所とは別の綴り**にする（取り違えを測る）。
 */
const SELECTED = "beta";

function report(main: LaunchView, sub: LaunchView, selected: string | null = SELECTED): UpstreamReport {
  return {
    main: view("main", main),
    sub: view("sub", sub),
    profile_sample: "C:\\Users\\someone\\AppData\\Local\\personacore\\upstreams.json",
    selected_profile: selected,
  };
}

const READY = launch({ state: "ready", answered_name: "main-12b.gguf" });
const READY_SUB = launch({ state: "ready", answered_name: "sub-3b.gguf" });
const LOADING = launch({ state: "loading", since: "2026-09-26T10:00:00+09:00" });
/** 起こすのを待っている面（作業単位 F-10b / ADR-0055）。 */
const NOT_STARTED = launch({ state: "not_started" });
/** その一言と止める理由（**直書き** —— 実装の定数から作らない）。 */
const NOT_STARTED_LINE =
  "本文のモデル: まだ起こしていません（前回選んだプロファイルは、起動しただけでは起こしません）";
const NOT_STARTED_HOLD = "本文のモデルをまだ起こしていないので、送れません。";

/**
 * **`POST /admin/upstream/profile` に送られた名前**（順に / 作業単位 F-10b）。
 */
let switched: string[];
/**
 * **`POST /admin/upstream/profile` の応答**（作業単位 F-10b）。
 *
 * | 値 | 応答 |
 * |---|---|
 * | `UpstreamReport` | 200（差し替えの応答の形） |
 * | `"busy"` | 409 `upstream_switch_in_progress`（サーバの文面） |
 * | `Promise` | それが解けるまで待たせる（押した後の「起こしています…」を見る） |
 */
let switchResponse: UpstreamReport | "busy" | Promise<UpstreamReport>;
/**
 * **保留している `GET /admin/upstream` を解く関数**（飛んだ順 / F-10b のレビュー是正）。
 * `null` なら保留せず、`upstream` をすぐ返す。
 */
let heldReads: ((body: UpstreamReport) => void)[] | null;

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

/** 200 の後に、SSE の中でエラーを1件流して終わる応答。 */
function streamError(): Response {
  const encoder = new TextEncoder();
  const events = [
    `data: ${JSON.stringify({ error: { message: "上流との通信が途中で切れました", type: "server_error" } })}\n\n`,
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
        json({ character: AOI, sessions: [], total_sessions: 0, sessions_limit: 100 }),
      );
    }
    if (url === "/admin/upstream") {
      // **読みを保留する**（F-10b のレビュー是正）。解くのはテストが選んだ時点・選んだ中身。
      if (heldReads !== null) {
        const held = heldReads;
        return new Promise<Response>((resolve) => {
          held.push((body) => resolve(json(body)));
        });
      }
      return Promise.resolve(
        upstream === null
          ? json({ error: { message: "内部エラーが発生しました", type: "server_error" } }, 500)
          : json(upstream),
      );
    }
    // **起こすボタン**（作業単位 F-10b）。**送られた名前を控え、応答は `switchResponse` で決める。**
    if (url === "/admin/upstream/profile" && init?.method === "POST") {
      const body = JSON.parse(String(init.body)) as { name: string };
      switched.push(body.name);
      const response = switchResponse;
      if (response === "busy") {
        return Promise.resolve(
          json(
            {
              error: {
                message: "上流の差し替えが進行中です。終わってから選び直してください",
                type: "invalid_request_error",
                code: "upstream_switch_in_progress",
              },
            },
            409,
          ),
        );
      }
      return Promise.resolve(response).then((next) => {
        // **サーバは応答と同じ状態を、次の読みでも返す**（起こした面は `loading`）。
        upstream = next;
        return json(next);
      });
    }
    if (url === "/v1/chat/completions") {
      const body = JSON.parse(String(init?.body)) as { model: string };
      sent.push(body.model);
      onSend();
      if (completion === "unavailable") {
        return Promise.resolve(
          json({ error: { message: "上流に接続できませんでした", type: "server_error" } }, 503),
        );
      }
      if (completion === "stream-error") {
        return Promise.resolve(streamError());
      }
      return Promise.resolve(stream(`${AOI}@41`, "「うん」"));
    }
    return Promise.resolve(json({ error: { message: "?", type: "invalid_request_error" } }, 404));
  });
}

function mountView(): ReturnType<typeof mount> {
  const view = mount(ChatView, {
    target,
    props: { pollIntervalMs: POLL_MS, idlePollIntervalMs: IDLE_MS },
  });
  live.add(view);
  return view;
}

/** 画面を離れる（`live` から外す）。 */
async function leave(view: ReturnType<typeof mount>): Promise<void> {
  live.delete(view);
  await unmount(view, { outro: false });
}

function sendButton(): HTMLButtonElement {
  const found = [...target.querySelectorAll("button")].find(
    (candidate) => candidate.textContent?.trim() === "送信",
  );
  expect(found, "送信の釦があること").toBeDefined();
  return found as HTMLButtonElement;
}

/** 本文の面の一言（**常に出る**）。 */
function faceLine(): string {
  const found = target.querySelector("[data-body-face]");
  expect(found, "本文の面の一言があること").not.toBeNull();
  return (found as HTMLElement).textContent?.replace(/\s+/gu, " ").trim() ?? "";
}

/** 送信を止めている理由（止めていなければ `null`）。 */
function holdLine(): HTMLElement | null {
  return target.querySelector<HTMLElement>("form [data-send-hold]");
}

function subLine(): string | null {
  return target.querySelector("[data-sub-face]")?.textContent?.replace(/\s+/gu, " ").trim() ?? null;
}

/**
 * **画面にある「…を起こす」釦の文面**（作業単位 F-10b）。
 *
 * 起こすボタンが出ているかを、**`data-*` ではなく利用者が見る釦の文面で**数える。
 * 送信の欄（`form`）の中と外の両方を見る（どこに置いても数える）。
 */
function startButtons(): string[] {
  return [...target.querySelectorAll("button")]
    .map((button) => button.textContent?.replace(/\s+/gu, " ").trim() ?? "")
    .filter((text) => text.endsWith("を起こす"));
}

function startButton(): HTMLButtonElement {
  const found = [...target.querySelectorAll("button")].find(
    (candidate) => candidate.textContent?.trim() === `「${SELECTED}」を起こす`,
  );
  expect(found, "起こすボタンがあること").toBeDefined();
  return found as HTMLButtonElement;
}

function upstreamReads(): number {
  return urls.filter((url) => url === "/admin/upstream").length;
}

async function waitForCharacters(): Promise<void> {
  await vi.waitFor(() => {
    const select = target.querySelector("select");
    expect(select).not.toBeNull();
    expect((select as HTMLSelectElement).value).toBe(AOI);
  });
}

/** 下書きを入れる（送信は押さない）。 */
function type(text: string): HTMLTextAreaElement {
  const textarea = target.querySelector("textarea") as HTMLTextAreaElement;
  textarea.value = text;
  textarea.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
  return textarea;
}

/** **釦ではない口**から送る（Ctrl+Enter / フォームの送信）。止めている間は飛ばないこと。 */
function submitByKeyboardAndForm(textarea: HTMLTextAreaElement): void {
  textarea.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true }),
  );
  (target.querySelector("form") as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  flushSync();
}

/**
 * **送信が飛ぶなら飛び終わるまでの待ち**（`setTimeout` を1回 / D-16 のレビュー）。
 * 「飛ばない」を見る腕と「飛ぶ」を見る対照が、**同じこの待ち**を使う。
 */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  flushSync();
}

/** 状態を読み終えるまで待つ（読み込み中の一言が消える）。 */
async function waitForFace(expected: string): Promise<void> {
  await vi.waitFor(() => expect(faceLine()).toBe(expected));
}

/**
 * **表を読んだ時刻を固定する**（Issue origin#40 / 状態の文面の経過の「いま」）。
 * `Date` だけを差し替える（`setTimeout` 等は本物のまま）。読み手はニューヨーク（冒頭の `readerInZone`）。
 */
const READ_AT = new Date("2026-09-26T01:00:12Z");

beforeEach(() => {
  vi.setSystemTime(READ_AT);
  target = document.createElement("div");
  document.body.append(target);
  urls = [];
  sent = [];
  upstream = report(READY, READY_SUB);
  completion = "done";
  onSend = () => undefined;
  switched = [];
  switchResponse = report(LOADING, LOADING);
  heldReads = null;
  live = new Set();
  stubFetch();
});

afterEach(async () => {
  for (const view of [...live]) {
    await leave(view);
  }
  vi.unstubAllGlobals();
  vi.useRealTimers();
  target.remove();
});

describe("状態ごとの、送信の可否と一言（画面を通して）", () => {
  it("載せている → 送信を止め、理由を1行。Ctrl+Enter でもフォームでも飛ばない", async () => {
    upstream = report(LOADING, READY_SUB);
    const view = mountView();
    await waitForCharacters();
    await waitForFace("本文のモデル: 載せています（21:00:00 から、12 秒経過）");

    const textarea = type("こんにちは");
    // **入力はできる**（止めるのは送信だけ）。
    expect(textarea.disabled).toBe(false);
    expect(sendButton().disabled).toBe(true);
    expect(holdLine()?.textContent?.trim()).toBe(
      "本文のモデルを載せている最中なので、まだ送れません。載ったら自動で送れるようになります。",
    );
    submitByKeyboardAndForm(textarea);
    await settle();
    expect(sent).toEqual([]);
    expect(target.querySelectorAll("ol.turns > li").length).toBe(0);

    // **対照（同じ1本・同じ待ち）**: 載ったら、同じ口から同じ待ちで飛ぶ。
    // **この待ちで飛ぶことが言えないと、上の「飛ばない」は待ちが短すぎただけかもしれない。**
    upstream = report(READY, READY_SUB);
    await vi.waitFor(() => expect(sendButton().disabled).toBe(false));
    submitByKeyboardAndForm(textarea);
    await settle();
    expect(sent).toEqual([AOI]);

    await leave(view);
  });

  it("準備完了 → 止めない（送れば飛ぶ）", async () => {
    const view = mountView();
    await waitForCharacters();
    await waitForFace("本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）");

    type("こんにちは");
    expect(sendButton().disabled).toBe(false);
    expect(holdLine()).toBeNull();
    sendButton().click();
    await vi.waitFor(() => expect(sent).toEqual([AOI]));

    await leave(view);
  });

  it("外の上流 → 止めない（その旨を言う）", async () => {
    upstream = report(launch({ state: "external" }), launch({ state: "external" }));
    const view = mountView();
    await waitForCharacters();
    await waitForFace("本文のモデル: 起こしていません（外の上流に依頼しています）");

    const textarea = type("こんにちは");
    expect(sendButton().disabled).toBe(false);
    expect(holdLine()).toBeNull();
    submitByKeyboardAndForm(textarea);
    await vi.waitFor(() => expect(sent).toEqual([AOI]));

    await leave(view);
  });

  it("裏方だけ載せている → 止めない（裏方の一言だけ出す）", async () => {
    upstream = report(READY, launch({ state: "loading", since: "2026-09-26T10:05:00+09:00" }));
    const view = mountView();
    await waitForCharacters();
    await waitForFace("本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）");

    type("こんにちは");
    expect(sendButton().disabled).toBe(false);
    expect(holdLine()).toBeNull();
    expect(subLine()).toBe("裏方のモデルを載せています。載るまでの間、記憶の抽出と変容は行われません。");
    sendButton().click();
    await vi.waitFor(() => expect(sent).toEqual([AOI]));

    await leave(view);
  });

  it("失敗 → 止める。理由とログを言い、上流モデルの画面への導線を置く", async () => {
    upstream = report(
      launch({ state: "failed", reason: "子が自分で終了しました", log: "main-20260926-100000.err.log" }),
      READY_SUB,
    );
    const view = mountView();
    await waitForCharacters();
    await waitForFace(
      "本文のモデル: 失敗しました: 子が自分で終了しました（ログ: upstream-logs の main-20260926-100000.err.log）",
    );

    type("こんにちは");
    expect(sendButton().disabled).toBe(true);
    const hold = holdLine();
    expect(hold?.textContent?.replace(/\s+/gu, "")).toBe(
      "本文のモデルを載せられなかったので、送れません。上流モデルの画面で確かめられます。",
    );
    expect([...(hold as HTMLElement).querySelectorAll("a")].map((a) => a.getAttribute("href"))).toEqual([
      hrefFor({ route: "upstream" }),
    ]);

    await leave(view);
  });

  it("止めた → 止める（導線を置く）", async () => {
    upstream = report(launch({ state: "stopped" }), READY_SUB);
    const view = mountView();
    await waitForCharacters();
    await waitForFace("本文のモデル: 止めました");

    type("こんにちは");
    expect(sendButton().disabled).toBe(true);
    expect(holdLine()?.textContent?.replace(/\s+/gu, "")).toBe(
      "本文のモデルが止まっているので、送れません。上流モデルの画面で確かめられます。",
    );

    await leave(view);
  });

  it("状態を読めない → 止めない（読めなかったと言う）", async () => {
    upstream = null;
    const view = mountView();
    await waitForCharacters();
    await waitForFace(
      "本文のモデル: 状態を読めませんでした（内部エラーが発生しました）。送信は止めていません。",
    );

    type("こんにちは");
    expect(sendButton().disabled).toBe(false);
    expect(holdLine()).toBeNull();

    await leave(view);
  });
});

/**
 * **読み手の時間帯が、帯の時刻に届く**（D-18 のレビュー / `tests/timestamp-wiring.test.ts` の形）。
 *
 * 日をまたぐ瞬間（`…T20:00:00Z`）を東京とニューヨークで読む —— 東京は翌日 05:00、
 * ニューヨークは同日 16:00。どちらを決め打っても片方で赤になる。
 * 終わったらファイルの読み手（ニューヨーク）へ戻す。
 */
describe("帯の時刻は、読み手の時間帯で出る", () => {
  const CROSSING = launch({ state: "loading", since: "2026-09-26T20:00:00Z" });

  for (const [zone, expected] of [
    ["Asia/Tokyo", "本文のモデル: 載せています（05:00:00 から、12 秒経過）"],
    ["America/New_York", "本文のモデル: 載せています（16:00:00 から、12 秒経過）"],
  ] as const) {
    it(`${zone} の読み手`, async () => {
      enterAsReader(zone);
      try {
        vi.setSystemTime(new Date("2026-09-26T20:00:12Z"));
        upstream = report(CROSSING, READY_SUB);
        mountView();
        await waitForCharacters();
        await waitForFace(expected);
      } finally {
        enterAsReader("America/New_York");
      }
    });
  }
});

describe("状態が変わったら、送信の可否が追いつく", () => {
  /** **「A が B になる」**: 前半で A（止めている）を確かめてから、B（戻る）を待つ。 */
  /** **経過は読み直すたびに進む**（Issue origin#40）—— 読んだ時刻を「いま」として渡している。 */
  it("載せている間、読み直すたびに帯の経過が進む", async () => {
    upstream = report(LOADING, READY_SUB);
    mountView();
    await waitForCharacters();
    await waitForFace("本文のモデル: 載せています（21:00:00 から、12 秒経過）");
    vi.setSystemTime(new Date(READ_AT.getTime() + 60_000));
    await waitForFace("本文のモデル: 載せています（21:00:00 から、1 分 12 秒経過）");
  });

  it("載せている → 準備完了で、送信が自動で戻る", async () => {
    upstream = report(LOADING, READY_SUB);
    const view = mountView();
    await waitForCharacters();
    await waitForFace("本文のモデル: 載せています（21:00:00 から、12 秒経過）");
    type("こんにちは");
    expect(sendButton().disabled).toBe(true);
    // **止めている間も読み直し続けている**（載るのを待つ）。
    const before = upstreamReads();
    await vi.waitFor(() => expect(upstreamReads()).toBeGreaterThan(before + 1));
    expect(sendButton().disabled).toBe(true);

    upstream = report(READY, READY_SUB);
    await vi.waitFor(() => expect(sendButton().disabled).toBe(false));
    expect(faceLine()).toBe("本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）");
    expect(holdLine()).toBeNull();
    sendButton().click();
    await vi.waitFor(() => expect(sent).toEqual([AOI]));

    await leave(view);
  });

  /**
   * **差し替え中**: 開いている間に（別のタブ・外のクライアントから）差し替えられると、
   * 本文の面は準備完了 → 載せている（切り替えの応答の形）→ 準備完了と動く。**止めていない間も読み直す。**
   */
  it("準備完了 → 差し替えで載せている → 準備完了（止めて、戻る）", async () => {
    const view = mountView();
    await waitForCharacters();
    await waitForFace("本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）");
    type("こんにちは");
    expect(sendButton().disabled).toBe(false);

    upstream = report(launch({ state: "loading", since: "2026-09-26T11:30:00+09:00" }), READY_SUB);
    await vi.waitFor(() => expect(sendButton().disabled).toBe(true));
    expect(faceLine()).toBe("本文のモデル: 載せています（22:30:00 から）");

    upstream = report(launch({ state: "ready", answered_name: "other-24b.gguf" }), READY_SUB);
    await vi.waitFor(() => expect(sendButton().disabled).toBe(false));
    expect(faceLine()).toBe("本文のモデル: 準備完了（上流が名乗った名前: other-24b.gguf）");

    await leave(view);
  });

  /**
   * **止めている間のほうが速く読み直す**（載るのを待っている利用者を待たせない）。
   * 止めていない間まで速く読むと、開いているだけで上流の口を叩き続ける。
   */
  it("止めている間は短い間隔、止めていない間は長い間隔で読み直す", async () => {
    upstream = report(LOADING, READY_SUB);
    const held = mountView();
    await waitForCharacters();
    await waitForFace("本文のモデル: 載せています（21:00:00 から、12 秒経過）");
    const heldStart = upstreamReads();
    await new Promise((resolve) => setTimeout(resolve, 200));
    const heldCount = upstreamReads() - heldStart;
    await leave(held);

    urls = [];
    upstream = report(READY, READY_SUB);
    const free = mountView();
    await waitForCharacters();
    await waitForFace("本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）");
    const freeStart = upstreamReads();
    await new Promise((resolve) => setTimeout(resolve, 200));
    const freeCount = upstreamReads() - freeStart;
    await leave(free);

    // 200ms で、5ms 間隔なら十数回、40ms 間隔なら数回（機の揺れを見込んで比で見る）。
    expect(freeCount).toBeGreaterThan(0);
    expect(heldCount).toBeGreaterThan(freeCount * 2);
  });
});

describe("送信が完結しなかった直後に、状態を読み直す（D-16 のレビュー）", () => {
  /**
   * **差し替えの窓で送った**: 送った瞬間にサーバは載せ直しに入り（`onSend` で `loading` へ）、送信は失敗で終わる。
   * **止めていない間の予約は長い**（ここでは 60 秒）ので、**すぐ読み直さなければ、1秒以内には止まらない。**
   */
  it.each(["stream-error", "unavailable"] as const)(
    "%s で終わった回の直後に読み、載せているなら送信を止める",
    async (how) => {
      completion = how;
      const view = mount(ChatView, {
        target,
        props: { pollIntervalMs: POLL_MS, idlePollIntervalMs: 60_000 },
      });
      live.add(view);
      await waitForCharacters();
      await waitForFace("本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）");
      const readsBefore = upstreamReads();
      onSend = () => {
        upstream = report(LOADING, READY_SUB);
      };

      type("こんにちは");
      // **A**: 送る前は止めていない。
      expect(sendButton().disabled).toBe(false);
      sendButton().click();
      await vi.waitFor(() => expect(sent).toEqual([AOI]));

      // **B**: 失敗の直後に読み、止める側へ追いつく。
      await vi.waitFor(() => expect(holdLine()).not.toBeNull());
      expect(upstreamReads()).toBeGreaterThan(readsBefore);
      expect(faceLine()).toBe("本文のモデル: 載せています（21:00:00 から、12 秒経過）");

      await leave(view);
    },
  );

  /** **対照**: 完結した回では読み直さない（止めていない間の予約のまま）。 */
  it("完結した回の直後には読み直さない", async () => {
    const view = mount(ChatView, {
      target,
      props: { pollIntervalMs: POLL_MS, idlePollIntervalMs: 60_000 },
    });
    live.add(view);
    await waitForCharacters();
    await waitForFace("本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）");
    const readsBefore = upstreamReads();

    type("こんにちは");
    sendButton().click();
    await vi.waitFor(() => expect(sent).toEqual([AOI]));
    await vi.waitFor(() =>
      expect(target.querySelector("ol.turns > li.assistant")?.textContent).not.toContain("秒"),
    );
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(upstreamReads()).toBe(readsBefore);

    await leave(view);
  });
});

describe("読めなかった後も、読み直し続ける", () => {
  /**
   * **読めない回が続いても読み直しをやめず、読めたら新しい状態に追いつく。**
   *
   * **同じ失敗が2回続くと、画面の状態は1バイトも変わらない**（同じ理由の文字列）。
   * 状態の変化だけを合図に予約し直すと、**そこで読み直しが止まる**
   * ——サーバが戻っても、画面は「読めませんでした」のまま（変異 `m11` が緑で残った穴）。
   * **読めた後に古い失敗を消す**ことも、ここで見る（消さないと止める判定に届かない / `m16`）。
   */
  it("読めない回が3回続いた後に載せているが読めたら、送信を止める", async () => {
    upstream = null;
    const view = mountView();
    await waitForCharacters();
    await waitForFace(
      "本文のモデル: 状態を読めませんでした（内部エラーが発生しました）。送信は止めていません。",
    );
    // **同じ失敗が続いても読み直している。**
    await vi.waitFor(() => expect(upstreamReads()).toBeGreaterThanOrEqual(3));
    type("こんにちは");
    expect(sendButton().disabled).toBe(false);

    upstream = report(LOADING, READY_SUB);
    await vi.waitFor(() => expect(sendButton().disabled).toBe(true));
    expect(faceLine()).toBe("本文のモデル: 載せています（21:00:00 から、12 秒経過）");

    await leave(view);
  });
});

/**
 * **起こしていない本文の面を、会話の画面から起こす**（作業単位 F-10b / ADR-0055 §決定3）。
 *
 * **配線を見る** —— 部品（`bodyFace.ts` の `startProfile`）が正しくても、画面が釦に結ばなければ
 * 起こせない。**押した釦が送った名前**（`POST /admin/upstream/profile` の本文）まで見る。
 */
describe("起こしていない本文の面（作業単位 F-10b）", () => {
  it("送信を止めて理由を言い、起こすボタンを出す。Ctrl+Enter でもフォームでも飛ばない", async () => {
    upstream = report(NOT_STARTED, NOT_STARTED);
    const view = mountView();
    await waitForCharacters();
    await waitForFace(NOT_STARTED_LINE);

    const textarea = type("こんにちは");
    expect(textarea.disabled).toBe(false);
    expect(sendButton().disabled).toBe(true);
    expect(holdLine()?.textContent?.replace(/\s+/gu, "")).toBe(NOT_STARTED_HOLD);
    expect(startButtons()).toEqual([`「${SELECTED}」を起こす`]);
    submitByKeyboardAndForm(textarea);
    await settle();
    expect(sent).toEqual([]);
    // **起こすボタンは送信の欄の中にあっても、フォームの送信で押されたことにならない。**
    expect(switched).toEqual([]);

    await leave(view);
  });

  /**
   * **押すと選んだ名前で頼み、応答（`loading`）にそのまま乗り、載ったら送れる**（「A が B になる」）。
   *
   * 前半で A（起こしていない・送れない）を確かめてから押し、B（載せている → 準備完了）を待つ。
   */
  it("押すと、選んだ名前で起こす依頼を送り、載せている → 準備完了で送信が戻る", async () => {
    upstream = report(NOT_STARTED, NOT_STARTED);
    let release: (next: UpstreamReport) => void = () => undefined;
    switchResponse = new Promise<UpstreamReport>((resolve) => {
      release = resolve;
    });
    const view = mountView();
    await waitForCharacters();
    await waitForFace(NOT_STARTED_LINE);
    type("こんにちは");
    expect(sendButton().disabled).toBe(true);

    startButton().click();
    await vi.waitFor(() => expect(switched).toEqual([SELECTED]));
    // **応答を待つ間は、もう一度押せない**（二度頼むと、2回目は 409 で断られるだけ）。
    await vi.waitFor(() => expect(startButton().disabled).toBe(true));
    expect(target.textContent).toContain(`「${SELECTED}」を起こしています…`);

    release(report(LOADING, LOADING));
    await waitForFace("本文のモデル: 載せています（21:00:00 から、12 秒経過）");
    expect(holdLine()?.textContent?.trim()).toBe(
      "本文のモデルを載せている最中なので、まだ送れません。載ったら自動で送れるようになります。",
    );
    expect(startButtons()).toEqual([]);
    expect(sendButton().disabled).toBe(true);

    upstream = report(READY, READY_SUB);
    await vi.waitFor(() => expect(sendButton().disabled).toBe(false));
    sendButton().click();
    await vi.waitFor(() => expect(sent).toEqual([AOI]));
    expect(switched).toEqual([SELECTED]);

    await leave(view);
  });

  /**
   * **応答そのものを状態として採る**（読み直しを待たない）。
   *
   * **読み直しの間隔を長くして**（60 秒）、押した後に状態が進むのが応答のおかげであることを見る。
   * 採らない実装は、次の読み直しまで「起こしていない」と釦を出し続け、二度押しを誘う
   * （上の1本は 5ms ごとに読み直すので、採らなくても緑になる —— 変異 W5b で実測）。
   */
  it("押した応答をそのまま採り、読み直しを待たずに載せている最中へ移る", async () => {
    upstream = report(NOT_STARTED, NOT_STARTED);
    const view = mount(ChatView, {
      target,
      props: { pollIntervalMs: 60_000, idlePollIntervalMs: 60_000 },
    });
    live.add(view);
    await waitForCharacters();
    await waitForFace(NOT_STARTED_LINE);
    const readsBefore = upstreamReads();

    startButton().click();
    await waitForFace("本文のモデル: 載せています（21:00:00 から、12 秒経過）");
    expect(startButtons()).toEqual([]);
    // **読み直しで追いついたのではない。**
    expect(upstreamReads()).toBe(readsBefore);

    await leave(view);
  });

  /**
   * **押す前に飛んだ読みが、応答より後に着いても、古い「起こしていない」へ巻き戻さない**
   * （F-10b のレビュー是正 / `startUpstream` の `upstreamTicket += 1`）。
   *
   * 1回目の読みは解いてボタンを出し、**2回目の読み（押す前に飛んだもの）を保留したまま押す。**
   * 応答（載せている）を採った後に、保留した読みを `not_started` で解く。番号を進めない実装は、
   * ここで一言が「まだ起こしていません」に戻り、押せるボタンが出直す。
   * **押した後に飛んだ読みは保留したまま解かない**（それは新しい読みで、書いてよい）。
   *
   * **⚠ 読み直しの間隔を 500ms にする。** 押した後の次の読みが、古い読みを解くより先に飛ぶと、
   * その読みが番号を進めて古い読みを捨ててしまい、**番号を進めない実装も緑になる**（5ms で書いた初版は
   * 変異 W11 で緑だった）。本番の間隔（2 秒）では、その前に古い読みが着きうる。
   */
  it("押す前に飛んだ読みが応答の後に着いても、起こしていないへ巻き戻さない", async () => {
    upstream = report(NOT_STARTED, NOT_STARTED);
    heldReads = [];
    const held = heldReads;
    const view = mount(ChatView, {
      target,
      props: { pollIntervalMs: 500, idlePollIntervalMs: 60_000 },
    });
    live.add(view);
    await waitForCharacters();
    await vi.waitFor(() => expect(held.length).toBeGreaterThanOrEqual(1));
    // **それまでに飛んだ読みを全部解いて、ボタンを出す**（読み込み中は長い間隔の予約も飛ぶので、
    // 着く前に2本目が飛んでいることがある。いちばん新しい読みだけが書く）。
    for (const resolve of held.splice(0)) {
      resolve(report(NOT_STARTED, NOT_STARTED));
    }
    await waitForFace(NOT_STARTED_LINE);
    // **次の読みが飛んで、保留されるのを待つ**（止めている間は 500ms ごとに読む）。
    await vi.waitFor(() => expect(held.length).toBe(1), { timeout: 2000 });
    const stale = held[0] as (body: UpstreamReport) => void;

    startButton().click();
    await waitForFace("本文のモデル: 載せています（21:00:00 から、12 秒経過）");
    expect(switched).toEqual([SELECTED]);
    // **前提: 押した後の読みはまだ飛んでいない**（飛んでいれば、それが古い読みを捨ててしまう）。
    expect(held.length).toBe(1);

    // **押す前に飛んだ読みが、いま着く。**
    stale(report(NOT_STARTED, NOT_STARTED));
    await settle();
    await settle();
    expect(held.length).toBe(1);
    expect(faceLine()).toBe("本文のモデル: 載せています（21:00:00 から、12 秒経過）");
    expect(startButtons()).toEqual([]);

    await leave(view);
  });

  it("断られたら（409）、サーバの理由を言い、送信は止めたまま、もう一度押せる", async () => {
    upstream = report(NOT_STARTED, NOT_STARTED);
    switchResponse = "busy";
    const view = mountView();
    await waitForCharacters();
    await waitForFace(NOT_STARTED_LINE);
    type("こんにちは");

    startButton().click();
    await vi.waitFor(() =>
      expect(target.querySelector("[role=alert]")?.textContent?.trim()).toBe(
        "起こせませんでした: 上流の差し替えが進行中です。終わってから選び直してください",
      ),
    );
    expect(switched).toEqual([SELECTED]);
    expect(sendButton().disabled).toBe(true);
    expect(startButton().disabled).toBe(false);

    // **もう一度押せば、もう一度頼む**（断られた理由は消える）。
    switchResponse = report(LOADING, LOADING);
    startButton().click();
    await waitForFace("本文のモデル: 載せています（21:00:00 から、12 秒経過）");
    expect(switched).toEqual([SELECTED, SELECTED]);
    expect(target.textContent).not.toContain("起こせませんでした");

    await leave(view);
  });

  /** **選んだ名前が無ければ、ボタンを出さない**（無い名前を頼ませない）。送信は止め、導線を出す。 */
  it("選んだ名前が無ければ、ボタンを出さずに上流モデルの画面への導線を出す", async () => {
    upstream = report(NOT_STARTED, launch({ state: "external" }), null);
    const view = mountView();
    await waitForCharacters();
    await waitForFace(NOT_STARTED_LINE);

    type("こんにちは");
    expect(sendButton().disabled).toBe(true);
    expect(startButtons()).toEqual([]);
    const hold = holdLine();
    expect(hold?.textContent?.replace(/\s+/gu, "")).toBe(
      `${NOT_STARTED_HOLD}上流モデルの画面で確かめられます。`,
    );
    expect([...(hold as HTMLElement).querySelectorAll("a")].map((a) => a.getAttribute("href"))).toEqual([
      hrefFor({ route: "upstream" }),
    ]);

    await leave(view);
  });

  /** **対照: 裏方だけが起こしていないなら、止めないし、ボタンも出さない**（裏方の面の状態では止めない）。 */
  it("裏方だけ起こしていない → 送信を止めず、起こすボタンも出さない", async () => {
    upstream = report(READY, NOT_STARTED);
    const view = mountView();
    await waitForCharacters();
    await waitForFace("本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）");

    type("こんにちは");
    expect(sendButton().disabled).toBe(false);
    expect(holdLine()).toBeNull();
    expect(startButtons()).toEqual([]);
    sendButton().click();
    await vi.waitFor(() => expect(sent).toEqual([AOI]));

    await leave(view);
  });

  /** **対照: 起こしていない以外の状態では、選んだ名前があってもボタンを出さない。** */
  it.each([
    ["準備完了", READY, "本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）"],
    ["外の上流", launch({ state: "external" }), "本文のモデル: 起こしていません（外の上流に依頼しています）"],
    ["載せている", LOADING, "本文のモデル: 載せています（21:00:00 から、12 秒経過）"],
    ["止めた", launch({ state: "stopped" }), "本文のモデル: 止めました"],
  ] as const)("%s → 起こすボタンを出さない", async (_name, main, line) => {
    upstream = report(main, READY_SUB);
    const view = mountView();
    await waitForCharacters();
    await waitForFace(line);
    expect(startButtons()).toEqual([]);
    await leave(view);
  });

  /** **本番の入口**（props なし）でも、起こしていないと言い、ボタンを出す。 */
  it("props なしでも、起こしていない面を言い、ボタンを出す", async () => {
    upstream = report(NOT_STARTED, NOT_STARTED);
    const view = mount(ChatView, { target });
    live.add(view);
    await waitForCharacters();
    await waitForFace(NOT_STARTED_LINE);
    expect(startButtons()).toEqual([`「${SELECTED}」を起こす`]);
    await leave(view);
  });
});

describe("後片付け", () => {
  it("画面を離れたら、読み直しを止める", async () => {
    upstream = report(LOADING, READY_SUB);
    const view = mountView();
    await waitForCharacters();
    await waitForFace("本文のモデル: 載せています（21:00:00 から、12 秒経過）");
    await vi.waitFor(() => expect(upstreamReads()).toBeGreaterThan(2));

    await leave(view);
    const after = upstreamReads();
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(upstreamReads()).toBe(after);
  });
});

describe("既定の間隔（props を渡さない本番の入口）", () => {
  /** **テスト専用の入口だけを通さない**（CLAUDE.md「本番の入口を通るテストを1本置く」）。 */
  it("props なしでも、開いたら1回読み、状態を言う", async () => {
    upstream = report(LOADING, READY_SUB);
    const view = mount(ChatView, { target });
    live.add(view);
    await waitForCharacters();
    await waitForFace("本文のモデル: 載せています（21:00:00 から、12 秒経過）");
    expect(upstreamReads()).toBe(1);
    type("こんにちは");
    expect(sendButton().disabled).toBe(true);

    await leave(view);
  });
});
