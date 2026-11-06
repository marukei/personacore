// @vitest-environment jsdom
/**
 * **キャラクターが1件も無いときの、会話の画面**（作業単位 F-5）。
 *
 * # なぜ、いま要るのか
 *
 * **これは利用者が最初に見る画面である**（既定の経路は `#/chat`）。
 * **空の DB から起動すると、必ずこの状態から始まる。**
 *
 * それなのに、**この状態を描くテストが1本も無かった**
 * （`characters: []` を返す腕がどこにも無い）。
 * 結果として、画面は **`POST /admin/characters` で作れます**と案内していた ——
 * **1クリック先に「新規作成」ボタンがあるのに、`curl` の話をしていた。**
 * **「実在する操作を案内しない」の6例目**（`docs/06-roadmap.md` の F 節）。
 *
 * # ⚠ 名指しした操作は、**別の画面**にある
 *
 * `StatsPanel` の `assertNoticeNamesOnlyRealOperations` は
 * 「**この画面**に実在すること」を問うが、ここは違う ——
 * **会話の画面にキャラクターを作る操作は無い。あるのはキャラクターの画面。**
 *
 * **だから2つを同時に要求する。**
 *
 * 1. **そこへ行くリンクが、この画面に実在する**（`#/characters`）。
 * 2. **名指しした操作が、その行き先の画面に実在する**
 *    —— **`CharacterList` を実際に描いて、そのボタンを引く。**
 *
 * **2 が無いと、案内だけが古びる**（ボタンの綴りを変えた日に、
 * 会話の画面の案内が黙って嘘になる）。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CharacterList from "../src/lib/CharacterList.svelte";
import ChatView from "../src/lib/ChatView.svelte";
import type { CharacterView } from "../src/lib/api";
import { noProposalsFor } from "./support/no-proposals";
import { externalUpstreamFor } from "./support/upstream-external";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

/**
 * **案内が名指しする操作の綴り。**
 *
 * # ⚠ ここに直書きする理由
 *
 * `CharacterList.svelte` から取ってくると、**ボタンの綴りを変える変異が
 * 期待値も一緒に動かして緑**になる（CLAUDE.md
 * 「写しを1つにしても、その1つは自動的には守られない」）。
 * **両側をこの1つの綴りで突き合わせる。**
 */
const CREATE_LABEL = "新規作成";

/** 行き先の画面（`router.ts` の `hrefFor({ route: "characters" })`）。 */
const CHARACTERS_HREF = "#/characters";

/**
 * **リンクが名乗る、行き先の画面の名前。**
 *
 * **⚠ 綴りを直書きする理由。** `labelFor` から作ると、
 * **別の画面の名前に差し替える変異が期待値も一緒に動かして緑**になる
 * （実測: `labelFor({route:"characters"})` → `labelFor({route:"chat"})` が 631 全緑）。
 */
const CHARACTERS_LINK_TEXT = "キャラクターの画面";

/** **別の画面の名前**（`labelFor({ route: "chat" })`）。名乗ってはいけない側。 */
const CHAT_SCREEN_LABEL = "会話";

/**
 * **案内に出てはいけない綴り。**
 *
 * どれも**利用者が画面から辿れない道**で、
 * 出ている限りこの画面は「リポジトリを開いている人」に話しかけている。
 */
const DEVELOPER_WORDS = ["POST", "curl", "/admin/", "http"];

/** キャラクターが1件でも居る状態の対照に使う。 */
const AOI: CharacterView = {
  id: 1,
  handle: "aoi",
  display_name: "藍生",
  base_persona: "静かな声で話す。",
  output_format_instruction: null,
  stat_scale: 5,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:00:00Z",
};

let target: HTMLElement;

/** `/admin/characters` がこれを返す。 */
let listed: CharacterView[];

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown) => {
    const url = String(input);
    // **約束・決まりの提案**（作業単位 A-12c-2）。**この測定は提案を持たない**（空の一覧 / `no-proposals.ts`）。
    const proposals = noProposalsFor(url);
    if (proposals !== null) {
      return proposals;
    }
    const respond = (body: unknown): Promise<Response> =>
      Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(JSON.stringify(body)),
      } as unknown as Response);

    // **会話の一覧**（作業単位 D-12c-2）。**この測定は過去の会話を持たない**（空の一覧）。
    // 足さないと、画面を開いたときの一覧の要求が下の台本を1本食う。
    // **本文の面の状態**（作業単位 D-16）。**外の上流を返す**（送信を止めない）。
    // 足さないと、画面を開いたときの状態の要求が下の台本を1本食う（`/sessions` と同じ）。
    const upstreamReply = externalUpstreamFor(url);
    if (upstreamReply !== null) {
      return upstreamReply;
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
      return respond({ characters: listed, unreadable: [] });
    }
    if (url.endsWith("/stats")) {
      return respond({ character: "?", stat_scale: 5, stats: [] });
    }
    return respond({});
  });
}

/** 空白（改行と字下げ）を畳んだ文面。**折り返しの位置に寄りかからない。** */
function flatText(element: HTMLElement): string {
  return (element.textContent ?? "").replace(/\s+/g, " ").trim();
}

/** 空状態の案内。 */
function emptyNotice(): HTMLElement {
  const found = target.querySelector("[data-no-characters]");
  expect(found, "キャラクターが1件も無いときの案内が出ること").not.toBeNull();
  return found as HTMLElement;
}

beforeEach(() => {
  listed = [];
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("キャラクターが1件も無いとき、実在する導線を案内する（作業単位 F-5）", () => {
  it("案内が出て、括弧の中が空にならない", async () => {
    const view = mount(ChatView, { target });
    await vi.waitFor(() => {
      expect(target.querySelector("[data-no-characters]")).not.toBeNull();
    });

    const text = flatText(emptyNotice());
    // **起きていること自体は言う。**
    expect(text).toContain("キャラクターがまだ1件もありません");
    // **括弧の中が空にならない。**
    //
    // **⚠ これは F-5 で報告された見え方そのものではない**（レビューの訂正 / 2026-08-29）。
    // 着手前の `textContent` は `（POST /admin/characters で作れます）` で、
    // **中身は入っていた** —— この検査は着手前も緑だった。
    // **F-5 を実際に捕まえているのは、下の `DEVELOPER_WORDS` の側。**
    //
    // **ここが止めるのは「括弧だけ残して中身を落とす」変異**で、それは実在しうる。
    expect(text, text).not.toMatch(/（\s*）/);

    await unmount(view, { outro: false });
  });

  it("`curl` の道ではなく、画面の操作を案内する", async () => {
    const view = mount(ChatView, { target });
    await vi.waitFor(() => {
      expect(target.querySelector("[data-no-characters]")).not.toBeNull();
    });

    const text = flatText(emptyNotice());
    for (const word of DEVELOPER_WORDS) {
      expect(text, `案内に ${word} が出ていないこと: ${text}`).not.toContain(word);
    }
    // **名指しするのは画面の操作**（`「」` で1つだけ）。
    const named = [...text.matchAll(/「([^」]+)」/g)].map(
      (found) => found[1] as string,
    );
    expect(named, `案内が名指しした操作: ${named.join(" / ")}`).toEqual([
      CREATE_LABEL,
    ]);

    await unmount(view, { outro: false });
  });

  it("そこへ行くリンクが、この画面に実在する", async () => {
    const view = mount(ChatView, { target });
    await vi.waitFor(() => {
      expect(target.querySelector("[data-no-characters]")).not.toBeNull();
    });

    const links = [...emptyNotice().querySelectorAll("a")];
    expect(
      links.map((link) => link.getAttribute("href")),
      "案内の中にキャラクターの画面へのリンクがあること",
    ).toEqual([CHARACTERS_HREF]);

    // **⚠ 「空でないこと」では足りない**（レビューの実測 / 2026-08-29）。
    // **文言だけを別の画面の名前にする変異が 631 全緑**を通った ——
    // `href` は `#/characters` のまま、文が
    // 「**会話の画面**の「新規作成」から作れます」になる。
    // **「載っているか」ではなく「どれのものか」**を見ること。
    expect(
      (links[0] as HTMLAnchorElement).textContent?.trim(),
      "リンクが、行き先の画面の名前を名乗ること",
    ).toBe(CHARACTERS_LINK_TEXT);
    // **対照: 他の画面の名前を名乗っていない。**
    expect((links[0] as HTMLAnchorElement).textContent).not.toContain(
      CHAT_SCREEN_LABEL,
    );

    await unmount(view, { outro: false });
  });

  /**
   * **名指しした操作が、行き先の画面に実在する。**
   *
   * **`CharacterList` を実際に描いて引く。** 綴りを写して満足すると、
   * **ボタンの綴りを変えた日に、会話の画面の案内だけが黙って嘘になる。**
   */
  it("名指しした操作が、キャラクターの画面に実在する", async () => {
    listed = [AOI];
    const view = mount(CharacterList, { target });
    await vi.waitFor(() => {
      expect(target.querySelector("tbody tr")).not.toBeNull();
    });

    const labels = [...target.querySelectorAll("button")].map(
      (button) => button.textContent?.trim() ?? "",
    );
    expect(
      labels,
      `キャラクターの画面のボタン: ${labels.join(" / ")}`,
    ).toContain(CREATE_LABEL);

    await unmount(view, { outro: false });
  });

  /**
   * **対照: 1件でも居れば、その案内は出ない。**
   *
   * 出しっぱなしにする実装は、上の3本を全部満たしてしまう。
   */
  it("対照: キャラクターが1件でも居れば、案内は出ない", async () => {
    listed = [AOI];
    const view = mount(ChatView, { target });
    await vi.waitFor(() => {
      expect(target.querySelector("select")).not.toBeNull();
    });

    expect(target.querySelector("[data-no-characters]")).toBeNull();
    expect(flatText(target)).not.toContain("キャラクターがまだ1件もありません");

    await unmount(view, { outro: false });
  });
});
