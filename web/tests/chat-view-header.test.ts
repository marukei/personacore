// @vitest-environment jsdom
/**
 * **会話の見出しの帯が、表示名の長いキャラクターで崩れない**（作業単位 B-67 / Issue origin#96）。
 *
 * 選択肢は `{display_name}（{handle}）` なので、`<select>` の幅は一番長い選択肢で決まる。
 * 帯（`header`）は折り返すので、幅が足りなくなると**ラベルの中で**折り返し、
 * 「相手:」が上の行に残って選択だけが下の段に出ていた。
 *
 * # ここで見るもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 「相手:」と選択が**同じ1つの `label`** に入り、文言が別の要素に分かれていない | **ここ**（構造） |
 * | 選択肢の文字列は変えない（省略記号を自前で足さない） | **ここ**（構造） |
 * | その `label` が横に並べて折り返さず、選択が残り幅まで縮む宣言を持つ | **ここ**（`<style>` の綴り） |
 *
 * # ⚠ 押さえられないもの
 *
 * **jsdom はレイアウトを計算せず、部品の `<style>` も当てない**（`src/app.css` の冒頭）。
 * **実際に同じ行に並ぶか・高さが揃うか・375px で横にはみ出さないか**は、
 * 司令塔がブラウザで見る。ここが言えるのは、それを支える**構造と宣言の綴り**だけ。
 *
 * **どの規則が勝つか（詳細度・層）も jsdom では測れない。** そこで `partner` を含む選択子を
 * `.partner` と `.partner select` の2つに固定し、**別の選択子からの上書き**（`label.partner { … }` 等）だけは止める。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ChatView from "../src/lib/ChatView.svelte";
import type { CharacterView } from "../src/lib/api";
import { noProposalsFor } from "./support/no-proposals";
import { externalUpstreamFor } from "./support/upstream-external";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

/** **全角 20 字**（Issue origin#96 の例の長さ）。 */
const LONG_NAME = "とても長い表示名を持つキャラクターです！";

function character(id: number, handle: string, display_name: string): CharacterView {
  return {
    id,
    handle,
    display_name,
    base_persona: "静かな声で話す。",
    output_format_instruction: null,
    stat_scale: 5,
    stat_drift_mode: null,
    axis_discovery_slots: 0,
    created_at: "2026-10-21T00:00:00Z",
    archived_at: null,
  };
}

let target: HTMLElement;

function stubFetch(listed: CharacterView[]): void {
  vi.stubGlobal("fetch", (input: unknown) => {
    const url = String(input);
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

beforeEach(() => {
  target = document.createElement("div");
  document.body.append(target);
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("見出しの帯: 「相手:」と選択は1つの塊", () => {
  it("「相手:」の文言と選択が同じ label の直下にあり、文言は別の要素に分かれていない", async () => {
    // **長い名前を真ん中に置く**（端の選択肢だけを見る実装を捕まえる）。
    stubFetch([
      character(1, "aoi", "葵"),
      character(2, "long", LONG_NAME),
      character(3, "nar", "語り手"),
    ]);
    const view = mount(ChatView, { target });
    await vi.waitFor(() => {
      expect(target.querySelectorAll("header select option").length).toBe(3);
    });

    const header = target.querySelector("header") as HTMLElement;
    const select = header.querySelector("select") as HTMLSelectElement;
    const label = select.parentElement as HTMLElement;

    // 選択を包むのは、帯の直下の `label.partner`（`<style>` が狙う綴り）。
    expect(label.tagName).toBe("LABEL");
    expect(label.parentElement).toBe(header);
    expect(label.classList.contains("partner")).toBe(true);

    // **label の子の要素は選択だけ**。文言は label 直下の文字（別の `span` 等で包まない）。
    expect([...label.children]).toEqual([select]);
    const ownText = [...label.childNodes]
      .filter((node) => node.nodeType === Node.TEXT_NODE)
      .map((node) => node.textContent ?? "")
      .join("")
      .trim();
    expect(ownText).toBe("相手:");

    // **選択肢の文字列は変えない**（省略記号を自前で足さない）。
    expect(
      [...select.querySelectorAll("option")].map((option) => option.textContent?.trim()),
    ).toEqual(["葵（aoi）", `${LONG_NAME}（long）`, "語り手（nar）"]);

    await unmount(view, { outro: false });
  });
});

/**
 * **`new URL(…, import.meta.url)` を使わない** —— この1本は jsdom で走るので、
 * vite が書き換えた URL が `file:` にならず読めない（`tests/character-form-examples.test.ts` と同じ）。
 */
const CHAT_VIEW = join(dirname(fileURLToPath(import.meta.url)), "..", "src", "lib", "ChatView.svelte");

/** `ChatView.svelte` の `<style>` の中身。 */
function componentStyle(): string {
  const source = readFileSync(CHAT_VIEW, "utf8");
  const found = /<style>([\s\S]*?)<\/style>/.exec(source);
  if (found === null || found[1] === undefined) {
    throw new Error("ChatView.svelte に <style> が見つからない");
  }
  // コメントを落とす（注記の中の綴りに当たらないように）。
  return found[1].replace(/\/\*[\s\S]*?\*\//g, "");
}

/** 選択子がちょうど `selector` の規則の宣言を、`名前: 値` の並びで返す。 */
function declarationsOf(style: string, selector: string): string[] {
  const rules = [...style.matchAll(/([^{}]+)\{([^{}]*)\}/g)];
  const matched = rules.filter((rule) => (rule[1] ?? "").trim() === selector);
  if (matched.length !== 1) {
    throw new Error(`${selector} の規則がちょうど1つではない（${matched.length} 個）`);
  }
  return (matched[0]?.[2] ?? "")
    .split(";")
    .map((declaration) => declaration.trim().replace(/\s+/g, " "))
    .filter((declaration) => declaration !== "");
}

describe("見出しの帯: 「相手:」と選択を同じ行に置く宣言", () => {
  it("label は横に並べ、中で折り返さず、帯の残り幅まで縮める", () => {
    // **全宣言を固定する**（含むかだけを見ると、`flex-wrap: wrap` を足して症状に戻っても緑のまま）。
    expect(declarationsOf(componentStyle(), ".partner")).toEqual([
      "display: flex",
      "align-items: center",
      "gap: 0.5rem",
      "white-space: nowrap",
      "min-width: 0",
    ]);
  });

  it("選択は中身の幅に戻し（土台の `width: 100%` を外す）、残り幅まで縮める", () => {
    expect(declarationsOf(componentStyle(), ".partner select")).toEqual([
      "width: auto",
      "min-width: 0",
    ]);
  });

  it("`partner` を含む選択子は `.partner` と `.partner select` の2つだけ（別の選択子からの上書きを止める）", () => {
    // 規則の選択子を `,` で割って1つずつ数える（`@media` の中の規則も拾う）。
    const selectors = [...componentStyle().matchAll(/([^{}]+)\{[^{}]*\}/g)]
      .flatMap((rule) => (rule[1] ?? "").split(","))
      .map((selector) => selector.trim().replace(/\s+/g, " "))
      .filter((selector) => selector.includes("partner"));
    expect(selectors).toEqual([".partner", ".partner select"]);
  });

  it("対照: 帯そのものは今までどおり折り返せる（狭い幅で帯ごと段を変える）", () => {
    expect(declarationsOf(componentStyle(), "header")).toEqual(["flex-wrap: wrap"]);
  });
});
