// @vitest-environment jsdom
/**
 * **過去の会話の一覧の部品**（作業単位 D-12c-2 / `src/lib/SessionPicker.svelte`）。
 *
 * **ここは部品だけを見る。** 画面がこの部品を何に繋いでいるか（選んだら何を読み、何を送るか）は
 * `chat-view-history.test.ts` が見る（CLAUDE.md「配線は部品の被覆とは別に見る」）。
 *
 * **時刻の配線**（読み手の時間帯を解決して渡すこと）は `timestamp-wiring.test.ts`。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import SessionPicker from "../src/lib/SessionPicker.svelte";
import type { SessionListView, SessionSummaryView } from "../src/lib/api";
import { readerInZone } from "./support/zone";

readerInZone("Pacific/Marquesas");

const HANDLE = "aoi";

function summary(
  id: number,
  count: number,
  first: string | null,
  truncated = false,
): SessionSummaryView {
  return {
    id,
    label: null,
    created_at: "2026-09-20T00:00:00Z",
    updated_at: `2026-09-2${String(id % 10)}T00:00:00Z`,
    message_count: count,
    first_user_message: first,
    first_user_message_truncated: truncated,
  };
}

/** **3件。真ん中を選ぶ**（端の決め打ちを通さない）。件数も手がかりも全部違えてある。 */
const THREE: SessionListView = {
  character: HANDLE,
  sessions: [
    summary(41, 14, "いちばん新しい話"),
    summary(12, 4, "あ".repeat(80), true),
    summary(7, 0, null),
  ],
  total_sessions: 3,
  sessions_limit: 100,
};

let target: HTMLElement;
let chosen: SessionSummaryView[];
/** 「新しい会話を始める」が押された回数（作業単位 D-17）。 */
let begun: number;

function render(
  list: SessionListView,
  options: { current?: number | null; busy?: boolean } = {},
): ReturnType<typeof mount> {
  const view = mount(SessionPicker, {
    target,
    props: {
      handle: HANDLE,
      list,
      current: options.current ?? null,
      busy: options.busy ?? false,
      onChoose: (session: SessionSummaryView) => {
        chosen.push(session);
      },
      onBeginNew: () => {
        begun += 1;
      },
    },
  });
  flushSync();
  return view;
}

function rows(): HTMLElement[] {
  return [...target.querySelectorAll<HTMLElement>("li[data-session-id]")];
}

function openButton(row: HTMLElement): HTMLButtonElement {
  const found = row.querySelector("button");
  expect(found).not.toBeNull();
  return found as HTMLButtonElement;
}

beforeEach(() => {
  target = document.createElement("div");
  document.body.append(target);
  chosen = [];
  begun = 0;
});

afterEach(() => {
  target.remove();
});

describe("並びと手がかり", () => {
  it("サーバの並びのまま、件数と最初の発言を出す", async () => {
    const view = render(THREE);

    expect(rows().map((row) => row.dataset["sessionId"])).toEqual(["41", "12", "7"]);
    expect(rows().map((row) => row.querySelector(".count")?.textContent?.trim())).toEqual([
      "14 件",
      "4 件",
      "0 件",
    ]);
    expect(rows().map((row) => row.querySelector(".opening")?.textContent)).toEqual([
      "いちばん新しい話",
      // **切ったことを隠さない。**
      `${"あ".repeat(80)}…`,
      "（まだ発言がありません）",
    ]);

    await unmount(view, { outro: false });
  });

  /** **先頭だけが「素の識別子で送ると続く会話」**（並びがサーバの続け方と同じだから）。 */
  it("先頭の会話だけに、素の識別子で送ると続くと書く", async () => {
    const view = render(THREE);

    const marked = rows().map((row) => row.textContent?.includes("で送ると続く会話") ?? false);
    expect(marked).toEqual([true, false, false]);
    expect(rows()[0]?.querySelector("code")?.textContent).toBe(HANDLE);

    await unmount(view, { outro: false });
  });

  it("全部を出していないときは、そう言う", async () => {
    const view = render({ ...THREE, total_sessions: 103 });
    expect(target.textContent?.replace(/\s+/gu, "")).toContain(
      "全103件のうち、新しい3件を出しています。",
    );
    await unmount(view, { outro: false });
  });

  it("全部を出しているときは、何も言わない", async () => {
    const view = render(THREE);
    expect(target.textContent).not.toContain("のうち、新しい");
    await unmount(view, { outro: false });
  });

  it("会話が1つも無ければ、そう言う", async () => {
    const view = render({ ...THREE, sessions: [], total_sessions: 0 });
    expect(rows()).toHaveLength(0);
    expect(target.textContent).toContain("まだ会話がありません。");
    await unmount(view, { outro: false });
  });
});

describe("選ぶ", () => {
  it("真ん中の会話の釦を押すと、その会話だけを伝える", async () => {
    const view = render(THREE);

    openButton(rows()[1] as HTMLElement).click();

    expect(chosen.map((session) => session.id)).toEqual([12]);

    await unmount(view, { outro: false });
  });

  it("いま出している会話の釦は塞がれている（他は押せる）", async () => {
    const view = render(THREE, { current: 12 });
    expect(rows().map((row) => openButton(row).disabled)).toEqual([false, true, false]);
    await unmount(view, { outro: false });
  });

  it("送信中は、どれも選べない", async () => {
    const view = render(THREE, { busy: true });
    expect(rows().map((row) => openButton(row).disabled)).toEqual([true, true, true]);
    await unmount(view, { outro: false });
  });
});

/** 一覧の「新しい会話を始める」（作業単位 D-17）。 */
function beginNewButton(): HTMLButtonElement {
  const found = [...target.querySelectorAll("button")].filter(
    (candidate) => candidate.textContent?.trim() === "新しい会話を始める",
  );
  expect(found, "「新しい会話を始める」がちょうど1つあること").toHaveLength(1);
  return found[0] as HTMLButtonElement;
}

/**
 * **会話を選ぶ場所で、新しく始めることもできる**（作業単位 D-17 / Issue origin#38）。
 *
 * **何を送るか（`@new`）は親が決める**（冒頭の「読むのは親」と同じ立場）。ここは「押された」を伝えるだけ。
 * 画面がそれを `@new` の送信に繋ぐことは `chat-view-new-session.test.ts` が見る。
 */
describe("新しい会話を始める", () => {
  it("押すと「始める」だけを伝え、どの会話も選ばない", async () => {
    const view = render(THREE, { current: 12 });

    beginNewButton().click();

    expect(begun).toBe(1);
    expect(chosen).toEqual([]);

    await unmount(view, { outro: false });
  });

  it("会話が1つも無くても置く", async () => {
    const view = render({ ...THREE, sessions: [], total_sessions: 0 });
    beginNewButton().click();
    expect(begun).toBe(1);
    await unmount(view, { outro: false });
  });

  it("送信中は押せない（送り先が途中で変わる）", async () => {
    const view = render(THREE, { busy: true });
    expect(beginNewButton().disabled).toBe(true);
    await unmount(view, { outro: false });
  });

  /** **対照**: 送信中でなければ、いま出している会話があっても押せる。 */
  it("送信中でなければ押せる", async () => {
    const view = render(THREE, { current: 41 });
    expect(beginNewButton().disabled).toBe(false);
    await unmount(view, { outro: false });
  });
});
