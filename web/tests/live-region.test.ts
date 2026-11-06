// @vitest-environment jsdom
/**
 * **読み上げる側に、何が届くか**（作業単位 D-11b）。
 *
 * # ここで見るものと、見ないもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **流れてきた本文が、告げる要素の「中に」在ること** | **ここ**（画面を実際に動かす） |
 * | **その要素が、本文より先に在ること** | **ここ** |
 * | **その要素が `polite` を名乗っていること**（`off` でも `assertive` でもない） | **ここ** |
 * | **毎秒変わる経過秒の段落が `aria-live="off"` を名乗っていること** | **ここ** |
 * | **エラーの段落が、その文言を持ったまま `role="alert"` であること** | **ここ**（画面 ＋ 走査） |
 * | **通った報せと「読み込み中」が `role="status"` であること** | **ここ**（走査） |
 * | **画面本体が `main`、画面を選ぶ帯が名乗った `nav` であること** | **ここ** |
 * | **実際に読み上げられるか** | **どこにも無い。人が聞くしかない** |
 *
 * # ⚠ 告げる要素は、`ol.turns` に置いた
 *
 * **受信中に生える `<p>` そのものに付ける形は採らなかった。**
 * **告げる領域は、中身が変わる「前から」在る必要がある** ——
 * 領域と中身が同時に挿入されると、**読み上げない閲覧器がある。**
 * `ol.turns` は画面が出た時点から在り、**本文はその中で育つ**ので、
 * **完結したときに `<p>` が別の要素へ差し替わっても、囲みは動かない。**
 *
 * **⚠ 代償**: **利用者自身の発言も同じ囲みの中に在る**ので、一緒に告げられる。
 * 分けるには囲みをもう1つ要る＝マークアップを書く回（`D-12`）。
 *
 * # ⚠ 守れていないこと
 *
 * * **「読み上げられた」ことは、ここでは言えない。** jsdom は読み上げない。
 *   見ているのは**属性と、その要素が持つ文字**までである。
 * * **`role="status"` を付けた要素の多くは、条件描画で「現れる」**
 *   （`読み込み中…` の段落は、読み終わると消える）。
 *   **現れた瞬間の中身を読まない閲覧器がある。**
 *   安定した容れ物を置くには**マークアップを書く回（`D-12`）**が要る。
 * * **表のセルの中の短い報せ**（一覧の行ごとの「読み込み中…」）**にも同じ印を付けている。**
 *   行の数だけ告げる領域が生えるので、**多いと騒がしい。**
 * * **`<span class="error">` は見ていない。** ここが見るのは**段落**だけである
 *   （行の中の短い断りは、表の読み上げの一部として読まれる）。
 * * **経過秒を `off` にしても、「告げられない」ことは言えない。**
 *   言えるのは**その段落がその属性を名乗っていること**まで（上と同じ理由）。
 * * **完結したときに、同じ返答がもう一度告げられうる。**
 *   受信中の `<p class="body">{turn.raw}</p>` が `[DONE]` で `{#each segments}` の形へ
 *   **差し替わる**ので、**囲みから見ると本文が丸ごと入れ替わったことになる。**
 *   **属性では直らない＝構造を変える回（`D-12`）**（`web/README.md` の「守れていないこと」）。
 *   **⚠ ここの「完結した回の本文も、同じ要素の中に在る」は、差し替えの後に
 *   本文が在ることしか見ていない。** 二度告げられるかは、jsdom では測れない。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../src/App.svelte";
import ChatView from "../src/lib/ChatView.svelte";
import { noProposalsFor } from "./support/no-proposals";
import { externalUpstreamFor } from "./support/upstream-external";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

/** 画面が最初に選ぶキャラクター。**綴りと表示名を別の値にしてある。** */
const HANDLE = "aoi";
const DISPLAY_NAME = "藍生";

/** 応答が名乗るセッション。 */
const SESSION = `${HANDLE}@41`;

/** 送る本文。**届く本文とは別の文字列。** */
const DRAFT = "ただいま";

/** **最初に届く増分。** 送った本文とも、2つ目の増分とも別の文字列。 */
const FIRST_CHUNK = "おかえりなさい";

/** **2つ目の増分。** 「育っていること」を見るのに要る。 */
const SECOND_CHUNK = "ごはんにする";

/** 断りの文言。**画面の他のどの文字列とも違う。** */
const REJECT_MESSAGE = "そのキャラクターは休んでいます";

/** **経過秒の単位**（画面の文言そのもの）。毎秒書き換わるのはこの欄だけ。 */
const SECONDS = "秒";

/** 断りの状態番号。 */
const REJECT_STATUS = 418;

/** 一覧が取れなかったときの言い訳。 */
const LIST_FAILURE_STATUS = 503;

/** 台本の1回ぶん（`null` は「返らない読み取り」＝送信中のまま止まる）。 */
type Script = (string | null)[];

let scripts: Script[];
/** `null` でなければ、送信は 200 の前に断られる。 */
let rejection: { status: number; body: string } | null;
/** `true` なら `/admin/characters` が失敗する。 */
let charactersFail: boolean;
let target: HTMLElement;

function event(payload: unknown): string {
  return `data: ${JSON.stringify(payload)}\n\n`;
}

function delta(text: string): string {
  return event({ model: SESSION, choices: [{ delta: { content: text } }] });
}

const DONE_EVENT = "data: [DONE]\n\n";

function errorBody(message: string): string {
  return JSON.stringify({
    error: { message, type: "invalid_request_error", code: "unknown" },
  });
}

/**
 * `fetch` の代わり。**本物の `Response` を組み立てない**
 * （`chat-view-notices.test.ts` と同じ理由——jsdom 側の実装差を測定に混ぜない）。
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
      if (charactersFail) {
        return Promise.resolve({
          ok: false,
          status: LIST_FAILURE_STATUS,
          text: () => Promise.resolve(errorBody(REJECT_MESSAGE)),
        } as unknown as Response);
      }
      const body = JSON.stringify({
        characters: [
          {
            id: 3,
            handle: HANDLE,
            display_name: DISPLAY_NAME,
            base_persona: "静かな声で話す。",
            output_format_instruction: null,
            stat_scale: 5,
            created_at: "2026-09-08T00:00:00Z",
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

    if (rejection !== null) {
      const said = rejection;
      return Promise.resolve({
        ok: false,
        status: said.status,
        text: () => Promise.resolve(said.body),
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

/** キャラクターが届くまで待つ（届かないと送信の口が開かない）。 */
async function waitForCharacters(): Promise<void> {
  await vi.waitFor(() => {
    expect(target.querySelector("select")).not.toBeNull();
  });
}

/** 本文を入れて送信する（**結末は待たない**）。 */
function submit(text: string): void {
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

/**
 * **`aria-live` を名乗っている要素、全部**（**`off` も含む**）。
 *
 * **`data-*` の印は使わない** —— テストが置いた印は、読み手が何を受け取るかの
 * 権威ではない（CLAUDE.md / `D-4`）。**属性そのものを引く。**
 */
function liveRegions(): HTMLElement[] {
  return [...target.querySelectorAll<HTMLElement>("[aria-live]")];
}

/**
 * **実際に告げる要素**（`off` を名乗っているものは、告げない）。
 *
 * **`off` は「名乗っているが告げない」**ので、数に入れると
 * 「告げる要素はちょうど1つ」が意味を変える。
 */
function announcingRegions(): HTMLElement[] {
  return liveRegions().filter(
    (element) => element.getAttribute("aria-live") !== "off",
  );
}

/**
 * 告げる要素が**1つだけ**在ること。その要素を返す。
 *
 * **⚠ 値まで見る。** `polite` を `off` にすれば**在るまま何も告げなくなり**、
 * `assertive` にすれば**増分が届くたびに読み上げが中断される**
 * （`ChatView.svelte` の囲みの ⚠）。
 * **どちらも「属性が在ること」しか見ない検査では通り抜ける**
 * ——**D-11b のレビューが実測: 2通りとも 846 全緑だった。**
 */
function theLiveRegion(): HTMLElement {
  const found = announcingRegions();
  expect(found.length, "告げる要素はちょうど1つ").toBe(1);
  const region = found[0] as HTMLElement;
  expect(region.getAttribute("aria-live")).toBe("polite");
  return region;
}

/** 割り込んで告げる要素（`role="alert"`）。 */
function alerts(): HTMLElement[] {
  return [...target.querySelectorAll<HTMLElement>('[role="alert"]')];
}

beforeEach(() => {
  scripts = [];
  rejection = null;
  charactersFail = false;
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("流れてくる本文は、告げる要素の中で育つ", () => {
  it("届く前から告げる要素が在り、届いた本文がその中に入る", async () => {
    // **`[DONE]` を受けないまま止まる回**（増分が2つ、そのあと返らない読み取り）。
    scripts = [[delta(FIRST_CHUNK), null]];

    const app = mount(ChatView, { target });
    try {
      await waitForCharacters();

      // **本文より先に在ること。** 領域と中身が同時に挿入されると、
      // **読み上げない閲覧器がある**（モジュール doc の ⚠）。
      const before = theLiveRegion();
      expect(before.textContent ?? "").not.toContain(FIRST_CHUNK);

      submit(DRAFT);

      await vi.waitFor(() => {
        expect(theLiveRegion().textContent ?? "").toContain(FIRST_CHUNK);
      });

      // **同じ要素のまま育っている**（差し替わっていない）。
      expect(theLiveRegion()).toBe(before);
      // **利用者の発言も同じ囲みの中に在る**（代償 / モジュール doc の ⚠）。
      expect(theLiveRegion().textContent ?? "").toContain(DRAFT);
    } finally {
      unmount(app);
    }
  });

  it("2つ目の増分も、同じ要素の中に足される", async () => {
    scripts = [[delta(FIRST_CHUNK), delta(SECOND_CHUNK), null]];

    const app = mount(ChatView, { target });
    try {
      await waitForCharacters();
      submit(DRAFT);

      await vi.waitFor(() => {
        const said = theLiveRegion().textContent ?? "";
        expect(said).toContain(FIRST_CHUNK);
        expect(said).toContain(SECOND_CHUNK);
      });
    } finally {
      unmount(app);
    }
  });

  it("完結した回の本文も、同じ要素の中に在る", async () => {
    scripts = [[delta(FIRST_CHUNK), DONE_EVENT]];

    const app = mount(ChatView, { target });
    try {
      await waitForCharacters();
      const before = theLiveRegion();
      submit(DRAFT);

      // **完結すると `<p>` は別の要素へ差し替わる。囲みは動かない。**
      // **「受信中… N 秒」が消えるまで**待つ（消えていなければ結末が入っていない）。
      await vi.waitFor(() => {
        expect(theLiveRegion().textContent ?? "").toContain(FIRST_CHUNK);
        expect(
          [...target.querySelectorAll("p.note")]
            .map((note) => note.textContent ?? "")
            .join(""),
        ).not.toContain("秒");
      });
      expect(theLiveRegion()).toBe(before);
    } finally {
      unmount(app);
    }
  });
});

describe("毎秒変わる欄は、告げる側に積まない", () => {
  it("経過秒を持つ段落は、告げる要素の中で `aria-live=\"off\"` を名乗る", async () => {
    // **`[DONE]` を受けないまま止まる回** —— 「受信中… N 秒」が出たまま残る。
    scripts = [[delta(FIRST_CHUNK), null]];

    const app = mount(ChatView, { target });
    try {
      await waitForCharacters();
      const region = theLiveRegion();
      submit(DRAFT);

      // **経過秒の文言を実際に持つ段落を引く。**
      // **属性の在処だけを見ると、どの要素に付いているかを一切見ていない**
      // （CLAUDE.md ——「その行が出た」と「その行がこう言っている」は別の主張）。
      let ticking: HTMLElement[] = [];
      await vi.waitFor(() => {
        ticking = [...region.querySelectorAll<HTMLElement>("p")].filter(
          (paragraph) => (paragraph.textContent ?? "").includes(SECONDS),
        );
        expect(ticking.length, "経過秒の段落が在ること").toBe(1);
      });

      for (const paragraph of ticking) {
        expect(
          paragraph.getAttribute("aria-live"),
          `経過秒の段落: ${paragraph.textContent?.trim()}`,
        ).toBe("off");
      }

      // **告げる要素は、いまも1つのまま**（`off` は数に入らない）。
      expect(theLiveRegion()).toBe(region);
    } finally {
      unmount(app);
    }
  });
});

describe("断りは、その文言を持ったまま割り込んで告げる", () => {
  it("一覧が取れなかったときの断り", async () => {
    charactersFail = true;

    const app = mount(ChatView, { target });
    try {
      await vi.waitFor(() => {
        expect(alerts().length).toBeGreaterThan(0);
      });
      // **文言を持っているのは、その印が付いた要素そのもの。**
      const said = alerts().map((element) => element.textContent ?? "");
      expect(said.some((text) => text.includes(REJECT_MESSAGE))).toBe(true);
      // **中身の無い要素に印だけ付いている形を、赤にする。**
      for (const text of said) {
        expect(text.trim()).not.toBe("");
      }
    } finally {
      unmount(app);
    }
  });

  it("送信が断られたときの断り", async () => {
    rejection = { status: REJECT_STATUS, body: errorBody(REJECT_MESSAGE) };

    const app = mount(ChatView, { target });
    try {
      await waitForCharacters();
      // **送る前は、割り込んで告げるものが1つも無い。**
      expect(alerts()).toEqual([]);

      submit(DRAFT);

      await vi.waitFor(() => {
        expect(alerts().length).toBeGreaterThan(0);
      });
      const said = alerts().map((element) => element.textContent ?? "");
      expect(said.some((text) => text.includes(REJECT_MESSAGE))).toBe(true);
      for (const text of said) {
        expect(text.trim()).not.toBe("");
      }
    } finally {
      unmount(app);
    }
  });
});

describe("画面の骨格が、名前を持っている", () => {
  it("画面本体が `main`、画面を選ぶ帯が名乗った `nav`", () => {
    const app = mount(App, { target });
    try {
      const nav = target.querySelector("nav");
      expect(nav, "画面を選ぶ帯が在ること").not.toBeNull();
      // **名乗りが空でないこと**（`aria-label=""` は名乗っていない）。
      expect((nav as HTMLElement).getAttribute("aria-label")?.trim()).toBeTruthy();

      const main = target.querySelector("main");
      expect(main, "画面本体が在ること").not.toBeNull();
      // **帯は本体の外**（本体の中に入れると、画面ごとの内容と混ざる）。
      expect((main as HTMLElement).contains(nav as HTMLElement)).toBe(false);
      // **いまの画面の中身は本体の中に在る**（空の `main` を赤にする）。
      expect((main as HTMLElement).textContent?.trim()).not.toBe("");
    } finally {
      unmount(app);
    }
  });
});
