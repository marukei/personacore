// @vitest-environment jsdom
/**
 * **しまったキャラクターは、会話の相手に出ない**（作業単位 A-10 / `src/lib/ChatView.svelte`）。
 *
 * サーバの `GET /admin/characters` は、しまったキャラクターも返す（時刻つき）。
 * 会話の画面はそれを相手の選択肢から外す —— 選べるとしても、送れば 409 で断られる。
 *
 * **最初に開く相手も、しまっていない方から選ぶ**（先頭がしまったキャラクターでも、
 * そのキャラクターの会話を読みに行かない）。
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

function character(
  id: number,
  handle: string,
  archived_at: string | null,
): CharacterView {
  return {
    id,
    handle,
    display_name: `${handle}さん`,
    base_persona: "静かな声で話す。",
    output_format_instruction: null,
    stat_scale: 5,
    stat_drift_mode: null,
    axis_discovery_slots: 0,
    created_at: "2026-10-21T00:00:00Z",
    archived_at,
  };
}

let target: HTMLElement;
let listed: CharacterView[];
let urls: string[];

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown) => {
    const url = String(input);
    urls.push(url);
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

    const upstreamReply = externalUpstreamFor(url);
    if (upstreamReply !== null) {
      return upstreamReply;
    }
    if (url.endsWith("/sessions")) {
      return respond({
        character: "",
        sessions: [],
        total_sessions: 0,
        sessions_limit: 100,
      });
    }
    if (url === "/admin/characters") {
      return respond({ characters: listed, unreadable: [] });
    }
    return respond({});
  });
}

function optionValues(): string[] {
  return [...target.querySelectorAll("select option")].map(
    (option) => (option as HTMLOptionElement).value,
  );
}

beforeEach(() => {
  urls = [];
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("しまったキャラクターは会話の相手に出ない", () => {
  it("真ん中のしまったキャラクターだけが、選択肢から外れる", async () => {
    listed = [
      character(1, "aoi", null),
      character(2, "mio", "2026-10-21T01:00:00Z"),
      character(3, "nar", null),
    ];
    const view = mount(ChatView, { target });
    await vi.waitFor(() => {
      expect(optionValues().length).toBeGreaterThan(0);
    });

    expect(optionValues()).toEqual(["aoi", "nar"]);

    await unmount(view, { outro: false });
  });

  it("先頭がしまったキャラクターなら、次の相手で開き、しまった方の会話を読みに行かない", async () => {
    listed = [
      character(1, "mio", "2026-10-21T01:00:00Z"),
      character(2, "aoi", null),
      character(3, "nar", null),
    ];
    const view = mount(ChatView, { target });
    await vi.waitFor(() => {
      expect(urls).toContain("/admin/characters/aoi/sessions");
    });

    expect(optionValues()).toEqual(["aoi", "nar"]);
    expect((target.querySelector("select") as HTMLSelectElement).value).toBe(
      "aoi",
    );
    expect(urls.filter((url) => url.includes("/mio"))).toEqual([]);

    await unmount(view, { outro: false });
  });
});

/** 空白（改行と字下げ）を畳んだ文面。 */
function flatText(element: Element): string {
  return (element.textContent ?? "").replace(/\s+/g, " ").trim();
}

describe("しまったキャラクターしか居ないときの文言（キャラクターの画面と同じ形で分ける）", () => {
  /** **案内が名指す操作の綴り**（`CharacterList.svelte` から取らない。取ると綴りの変異が両側で動く）。 */
  const RESTORE_LABEL = "戻す";

  it("「まだ1件もありません」と言わず、戻す導線を案内する", async () => {
    listed = [
      character(1, "aoi", "2026-10-21T01:00:00Z"),
      character(2, "mio", "2026-10-21T01:00:00Z"),
      character(3, "nar", "2026-10-21T01:00:00Z"),
    ];
    const view = mount(ChatView, { target });
    await vi.waitFor(() => {
      expect(target.querySelector("[data-only-archived]")).not.toBeNull();
    });

    const notice = target.querySelector("[data-only-archived]") as HTMLElement;
    const text = flatText(notice);
    expect(text).toContain("しまっていないキャラクターはありません");
    expect(target.querySelector("[data-no-characters]")).toBeNull();
    expect(flatText(target)).not.toContain("キャラクターがまだ1件もありません");
    // **名指すのは、行き先の画面に実在する操作だけ**（「」で1つ）。
    const named = [...text.matchAll(/「([^」]+)」/g)].map((found) => found[1]);
    expect(named).toEqual([RESTORE_LABEL]);
    expect(
      [...notice.querySelectorAll("a")].map((link) => link.getAttribute("href")),
    ).toEqual(["#/characters"]);

    await unmount(view, { outro: false });
  });

  it("名指した操作が、キャラクターの画面のしまった一覧に実在する", async () => {
    listed = [character(1, "aoi", "2026-10-21T01:00:00Z")];
    const view = mount(CharacterList, { target });
    await vi.waitFor(() => {
      expect(target.querySelector("[data-archived-character]")).not.toBeNull();
    });

    const labels = [
      ...(target.querySelector("[data-archived-character]") as HTMLElement).querySelectorAll(
        "button",
      ),
    ].map((button) => button.textContent?.trim() ?? "");
    expect(labels).toContain(RESTORE_LABEL);

    await unmount(view, { outro: false });
  });

  it("対照: 1件も居なければ、今までどおり「まだ1件もありません」", async () => {
    listed = [];
    const view = mount(ChatView, { target });
    await vi.waitFor(() => {
      expect(target.querySelector("[data-no-characters]")).not.toBeNull();
    });
    expect(target.querySelector("[data-only-archived]")).toBeNull();

    await unmount(view, { outro: false });
  });

  it("対照: 話せる相手が1人でも居れば、どちらの文言も出ない", async () => {
    listed = [
      character(1, "aoi", "2026-10-21T01:00:00Z"),
      character(2, "mio", null),
    ];
    const view = mount(ChatView, { target });
    await vi.waitFor(() => {
      expect(optionValues()).toEqual(["mio"]);
    });
    expect(target.querySelector("[data-only-archived]")).toBeNull();
    expect(target.querySelector("[data-no-characters]")).toBeNull();

    await unmount(view, { outro: false });
  });
});
