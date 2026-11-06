// @vitest-environment jsdom
/**
 * **「送り先」がモデル名に読めていた**（作業単位 D-9 / `src/lib/ChatView.svelte`）。
 *
 * # 何が誤解だったのか
 *
 * 会話の画面は **`送り先: aoi@1（このセッションの続き）`** と出していた。
 * **利用者から見れば、これは「いま使っているモデル」に読める。**
 * **実際はセッションの識別子**で、`GET /v1/models` が返すのも
 * **キャラクターであってモデルではない。**
 *
 * **つまり、着手前の画面に「モデル」に見えるものは1つしか無く、
 * それは1つもモデルではなかった。**
 *
 * # 直し方は2つで1組
 *
 * | やったこと | なぜ片方では足りないか |
 * |---|---|
 * | **セッションの識別子だと画面が言う** | 言うだけでは、**本物のモデルの綴りが画面のどこにも無い**ままである |
 * | **上流の画面への導線を置く** | 導線だけでは、**`送り先` の誤読はそのまま残る** |
 *
 * # ⚠ 導線は「行き先に実在すること」まで見る
 *
 * **案内だけが古びる形**（`chat-view-empty.test.ts` / 作業単位 F-5）を避けるため、
 * **`UpstreamPanel` を実際に描いて、モデルの綴りの行を引く。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ChatView from "../src/lib/ChatView.svelte";
import UpstreamPanel from "../src/lib/UpstreamPanel.svelte";
import type { CharacterView, UpstreamReport } from "../src/lib/api";
import { noProposalsFor } from "./support/no-proposals";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

/** 行き先の画面（`router.ts` の `hrefFor({ route: "upstream" })`）。 */
const UPSTREAM_HREF = "#/upstream";

/**
 * **リンクが名乗る、行き先の画面の名前。**
 *
 * **⚠ 綴りを直書きする理由**（`chat-view-empty.test.ts` と同じ）。
 * `labelFor` から作ると、**別の画面の名前に差し替える変異が
 * 期待値も一緒に動かして緑**になる。
 */
const UPSTREAM_LINK_TEXT = "上流モデルの画面";

/**
 * **着いた先の画面が名乗る名前**（`UpstreamPanel.svelte` の `<h1>`）。
 *
 * **⚠ 着手前、これを見る検査は1本も無かった** ——
 * 実測（`m20`）: `<h1>上流モデル</h1>` → `<h1>接続先の一覧</h1>` が **714 全緑。**
 * **リンクの文言だけが守られ、着いた先の見出しは誰も見ていなかった**ので、
 * **片方だけ変えると「案内だけが黙って嘘になる」**
 * ——このファイルが塞いだつもりの穴の、ちょうど反対側である。
 *
 * **綴りは直書きする**（`labelFor` から作ると、両方を差し替える変異が緑になる）。
 */
const UPSTREAM_SCREEN_NAME = "上流モデル";

/** **別の画面の名前。** 名乗ってはいけない側。 */
const OTHER_SCREEN_LABELS = ["会話", "キャラクター"];

/** 上流の画面が出す、モデルの綴りの行の見出し。 */
const MODEL_ROW_LABEL = "依頼するモデルの綴り";

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

/** 上流の観測。**本文のモデルの綴りは、セッションの識別子と1文字も重ならない。** */
const REPORT: UpstreamReport = {
  main: {
    face: "main",
    endpoint: "http://127.0.0.1:1234/v1/chat/completions",
    model: "main-model-for-tests",
    // **上流は、頼んだ名前と違うものを名乗っている**（作業単位 B-45 / `C-18`）。
    answered_name: "answering-model-for-tests.gguf",
    queried_context_tokens: 16384,
    answered_by: "native-api",
    unauthorized: false,
    checked_at: "2026-11-03T09:58:00+09:00",
    check_result: "answered",
    measured: {
      checked: 4,
      matched: 4,
      drifted_other: 0,
      drifted_absent: 0,
      unanswered: 0,
    },
    // **この起動は `--upstream` を渡していない**（ADR-0042 §決定3）。
    launch: {
      state: "external",
      since: null,
      answered_name: null,
      reason: null,
      log: null,
    },
  },
  sub: {
    face: "sub",
    endpoint: "http://127.0.0.1:5678/v1/chat/completions",
    model: "sub-model-for-tests",
    answered_name: "sub-model-for-tests",
    queried_context_tokens: 8192,
    answered_by: "openai-exact-id",
    unauthorized: false,
    checked_at: "2026-11-03T09:57:00+09:00",
    check_result: "answered",
    measured: {
      checked: 5,
      matched: 5,
      drifted_other: 0,
      drifted_absent: 0,
      unanswered: 0,
    },
    // **この起動は `--upstream` を渡していない**（ADR-0042 §決定3）。
    launch: {
      state: "external",
      since: null,
      answered_name: null,
      reason: null,
      log: null,
    },
  },
  // **この起動は見本を書いていない**（作業単位 F-4c）。
  profile_sample: null,
  // **この起動はプロファイルを選んでいない**（作業単位 F-10b）。
  selected_profile: null,
};

let target: HTMLElement;

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
      return respond({ characters: [AOI], unreadable: [] });
    }
    if (url === "/admin/upstream") {
      return respond(REPORT);
    }
    return respond({});
  });
}

/** 空白を畳んだ文面。 */
function flat(element: Element | null): string {
  return (element?.textContent ?? "").replace(/\s+/gu, " ").trim();
}

/** 送り先の注記。**無ければ落とす。** */
function wireNote(): HTMLElement {
  const found = target.querySelector("[data-wire-note]");
  expect(found, "送り先の注記が出ること").not.toBeNull();
  return found as HTMLElement;
}

async function openChat(): Promise<Record<string, unknown>> {
  const view = mount(ChatView, { target });
  await vi.waitFor(() => {
    expect(target.querySelector("p.wire code")).not.toBeNull();
  });
  return view as Record<string, unknown>;
}

beforeEach(() => {
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("送り先が、セッションの識別子だと分かる", () => {
  /** **送り先そのものは、これまでどおり出している**（隠して解決しない）。 */
  it("送り先の綴りと意味は、これまでどおり出ている", async () => {
    const view = await openChat();
    expect(flat(target.querySelector("p.wire code"))).toBe("aoi");
    expect(flat(target.querySelector("p.wire .meaning"))).toBe(
      "（最新の会話の続き）",
    );
    await unmount(view, { outro: false });
  });

  it("モデルの名前ではないと、画面が言っている", async () => {
    const view = await openChat();
    const text = flat(wireNote());
    expect(text).toContain("この会話を指す名前");
    expect(text).toContain("モデルの名前ではありません");
    await unmount(view, { outro: false });
  });

  it("本文を書いているモデルの綴りが、どこで見られるかを言っている", async () => {
    const view = await openChat();
    const links = [...wireNote().querySelectorAll("a")];
    expect(
      links.map((link) => link.getAttribute("href")),
      "上流の画面へのリンクが1本あること",
    ).toEqual([UPSTREAM_HREF]);
    // **⚠ 「空でないこと」では足りない**（文言だけ別の画面の名前にする変異）。
    expect(flat(links[0] as HTMLAnchorElement)).toBe(UPSTREAM_LINK_TEXT);
    for (const label of OTHER_SCREEN_LABELS) {
      expect(flat(links[0] as HTMLAnchorElement), label).not.toContain(label);
    }
    await unmount(view, { outro: false });
  });

  /**
   * **注記が名指しした先に、モデルの綴りが実在する。**
   *
   * **`UpstreamPanel` を実際に描いて引く。**
   * 写して満足すると、**行の見出しを変えた日に、会話の画面の案内だけが黙って嘘になる。**
   */
  it("名指しした先の画面に、本文のモデルの綴りが実在する", async () => {
    const view = mount(UpstreamPanel, { target });
    await vi.waitFor(() => {
      expect(target.querySelector("table")).not.toBeNull();
    });

    const rows = [...target.querySelectorAll("table tr")].map((row) =>
      [...row.querySelectorAll("th, td")].map((cell) => flat(cell)),
    );
    expect(rows).toContainEqual([
      MODEL_ROW_LABEL,
      REPORT.main.model,
      REPORT.sub.model,
    ]);

    await unmount(view, { outro: false });
  });

  /**
   * **案内した名前で、着いた先が自分を名乗る。**
   *
   * リンクは「**上流モデルの画面**」と言う。**着いた先の見出しが別の名前を名乗れば、
   * 利用者は「違う画面に来た」と読む。**
   * **両側とも直書きの綴りに錨を打ち、その2つの関係だけをここで結ぶ。**
   */
  it("名指しした先の画面が、案内した名前で自分を名乗る", async () => {
    // **フィクスチャの前提**——リンクの文言は、画面の名前を含んでいる。
    expect(UPSTREAM_LINK_TEXT).toBe(`${UPSTREAM_SCREEN_NAME}の画面`);

    const view = mount(UpstreamPanel, { target });
    await vi.waitFor(() => {
      expect(target.querySelector("table")).not.toBeNull();
    });

    expect(flat(target.querySelector("h1"))).toBe(UPSTREAM_SCREEN_NAME);

    await unmount(view, { outro: false });
  });

  /**
   * **対照: 会話の画面は、モデルの綴りを名乗っていない。**
   *
   * ここで綴りを写すと**写しが2つ**になり、片方だけが黙って古びる。
   * **導線を張ったのは、写さないためでもある。**
   */
  it("対照: 会話の画面は、上流のモデルの綴りを名乗らない", async () => {
    const view = await openChat();
    expect(flat(target)).not.toContain(REPORT.main.model);
    expect(flat(target)).not.toContain(REPORT.sub.model);
    await unmount(view, { outro: false });
  });
});
