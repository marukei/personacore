// @vitest-environment jsdom
/**
 * **新しいキャラクターの画面が、入力する前に書き方を言う**（作業単位 D-14 / Issue origin#30 /
 * `src/lib/CharacterForm.svelte`）。
 *
 * # ここが見るもの
 *
 * | 何 | どう見るか |
 * |---|---|
 * | **識別子に使える文字を、入力する前に言う** | 描かれた説明から文字の集合を読み、`src/domain/character.rs` の `Handle::ALLOWED_CHARACTERS` と突き合わせる |
 * | **その列挙は画面に一度だけ** | 画面の文面全体から読んだ文字に重なりが無い |
 * | **尺度の既定の値** | 描かれた説明の数を、`src/domain/stat.rs` の `StatScale::DEFAULT` と突き合わせる |
 * | **各欄の記入例** | `placeholder` を値まで見る |
 *
 * # ⚠ 使える文字は、画面に写してある（写しの3箇所目）
 *
 * 利用者は「日本語を打つと何を直せばよいか分からない」ところで止まった（Issue origin#30）。
 * サーバの理由は使える文字を列挙しない（`HandleError::Charset` の doc）ので、
 * **入力する前に一度だけ言う。** 写しが古びないように、写し元を読む見張り番をここに置く
 * （`tests/character-edit.test.ts` が `LIMITS` を写し元と突き合わせるのと同じ作法）。
 *
 * # ⚠ 対照は、植えたソースを読む関数と比べる関数に通す
 *
 * 写し元を読む関数（`allowedCharactersOf` / `statScaleDefaultOf`）と、
 * 説明から文字を読む関数（`charactersNamedIn`）は、**引数でソースを受け取る。**
 * 実ファイルと植えたソースを同じ判定に通すので、読めていないのに緑、にならない。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import CharacterForm from "../src/lib/CharacterForm.svelte";
import type { CharacterView } from "../src/lib/api";

const MIO: CharacterView = {
  id: 2,
  handle: "mio",
  display_name: "澪",
  base_persona: "早口で話す。",
  output_format_instruction: "台詞は「」で囲む。",
  stat_scale: 5,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:00:00Z",
};

let target: HTMLElement;

beforeEach(() => {
  target = document.createElement("div");
  document.body.append(target);
});

afterEach(() => {
  target.remove();
});

/**
 * 写し元を読む。
 *
 * **`new URL(…, import.meta.url)` を使わない** —— この1本は jsdom で走るので、
 * vite が書き換えた URL が `file:` にならず読めない（`tests/character-edit.test.ts` は `node` で走る）。
 */
const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const CHARACTER_RS = join(REPO_ROOT, "src", "domain", "character.rs");
const STAT_RS = join(REPO_ROOT, "src", "domain", "stat.rs");

function readRepo(path: string): string {
  return readFileSync(path, "utf8");
}

function flat(element: Element | null | undefined): string {
  return (element?.textContent ?? "").replace(/\s+/gu, " ").trim();
}

function hint(name: string): Element | null {
  return target.querySelector(`[data-hint="${name}"]`);
}

// ----------------------------------------------------------------------
// 写し元を読む・説明を読む（**引数でソースを受け取る**）
// ----------------------------------------------------------------------

/** `Handle::ALLOWED_CHARACTERS` の綴り。**1つだけ見つからなければ落とす。** */
function allowedCharactersOf(rustSource: string): string[] {
  const found = [
    ...rustSource.matchAll(/pub const ALLOWED_CHARACTERS: &'static str = "([^"]*)";/gu),
  ];
  if (found.length !== 1 || found[0]?.[1] === undefined) {
    throw new Error(`ALLOWED_CHARACTERS が1つだけ見つかりません（${found.length} 件）`);
  }
  return [...found[0][1]];
}

/**
 * **説明が名指す文字を、書かれた順に全部**（重なりもそのまま）。
 *
 * 読む綴りは2つだけ —— 範囲 `a〜z`（両端が ASCII の1文字）と、1文字の引用 `「-」`。
 */
function charactersNamedIn(text: string): string[] {
  const named: string[] = [];
  for (const range of text.matchAll(/([!-~])[〜～]([!-~])/gu)) {
    const from = (range[1] as string).charCodeAt(0);
    const to = (range[2] as string).charCodeAt(0);
    for (let code = from; code <= to; code += 1) {
      named.push(String.fromCharCode(code));
    }
  }
  for (const single of text.matchAll(/「([!-~])」/gu)) {
    named.push(single[1] as string);
  }
  return named;
}

/**
 * 並びを問わず、同じ文字の集合か。**重なりがあれば同じとみなさない**
 * （数が同じで、写し元の文字が全部在るなら、重なりの入る余地は無い。写し元に重なりが無い限り）。
 */
function sameCharacters(named: readonly string[], allowed: readonly string[]): boolean {
  return (
    named.length === allowed.length && allowed.every((character) => named.includes(character))
  );
}

/** `impl StatScale` の中の `DEFAULT`。**見つからなければ落とす。** */
function statScaleDefaultOf(rustSource: string): number {
  const start = rustSource.indexOf("impl StatScale {");
  if (start < 0) {
    throw new Error("impl StatScale が見つかりません");
  }
  const rest = rustSource.slice(start + 1);
  const end = rest.indexOf("\nimpl ");
  const block = end < 0 ? rest : rest.slice(0, end);
  const found = /pub const DEFAULT: Self = Self\((\d+)\);/u.exec(block);
  if (found?.[1] === undefined) {
    throw new Error("impl StatScale に DEFAULT がありません");
  }
  return Number.parseInt(found[1], 10);
}

// ----------------------------------------------------------------------

const HANDLE_HINT =
  "英小文字（a〜z）・数字（0〜9）・「-」・「_」だけが使えます。会話の送り先や URL に使われます。日本語の名前は、下の表示名に書いてください。作成のときだけ決められます（後から変えられません）。";

describe("識別子に使える文字", () => {
  it("作成のとき、識別子の欄の説明が使える文字を言う", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });
    expect(flat(hint("handle"))).toBe(HANDLE_HINT);
    await unmount(view, { outro: false });
  });

  /**
   * **描かれた説明から読んだ文字 = `Handle::ALLOWED_CHARACTERS`。**
   * どちらか片方だけを変えれば赤になる。
   */
  it("説明が言う文字は、`src/domain/character.rs` の `ALLOWED_CHARACTERS` と一致する", async () => {
    const allowed = allowedCharactersOf(readRepo(CHARACTER_RS));
    const view = mount(CharacterForm, { target, props: { character: null } });
    const named = charactersNamedIn(flat(hint("handle")));
    expect([...named].sort()).toEqual([...allowed].sort());
    expect(sameCharacters(named, allowed)).toBe(true);
    await unmount(view, { outro: false });
  });

  /** **列挙は一度だけ**（画面の文面全体で、同じ文字を2度名指さない）。 */
  it("使える文字の列挙は、画面に一度だけ出る", async () => {
    const allowed = allowedCharactersOf(readRepo(CHARACTER_RS));
    const view = mount(CharacterForm, { target, props: { character: null } });
    const named = charactersNamedIn(flat(target));
    expect(named.length).toBe(allowed.length);
    expect(new Set(named).size).toBe(named.length);
    await unmount(view, { outro: false });
  });

  it("対照: 編集のときは、識別子の説明そのものが出ない（識別子は変えられない）", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });
    expect(hint("handle")).toBeNull();
    expect(charactersNamedIn(flat(target))).toEqual([]);
    await unmount(view, { outro: false });
  });

  describe("対照: 植えたソースを、同じ読み方と比べ方に通す", () => {
    const real = "abcdefghijklmnopqrstuvwxyz0123456789-_";

    it("写し元の文字が1つ増えれば、食い違う", () => {
      const planted = `impl Handle {\n    pub const ALLOWED_CHARACTERS: &'static str = "${real}.";\n}`;
      const allowed = allowedCharactersOf(planted);
      expect(allowed).toContain(".");
      expect(sameCharacters(charactersNamedIn(HANDLE_HINT), allowed)).toBe(false);
    });

    it("写し元の文字が1つ減れば、食い違う", () => {
      const planted = `pub const ALLOWED_CHARACTERS: &'static str = "${real.replace("_", "")}";`;
      expect(sameCharacters(charactersNamedIn(HANDLE_HINT), allowedCharactersOf(planted))).toBe(false);
    });

    it("説明から1文字落ちれば、食い違う", () => {
      const planted = HANDLE_HINT.replace("・「_」", "");
      expect(planted).not.toBe(HANDLE_HINT);
      expect(sameCharacters(charactersNamedIn(planted), [...real])).toBe(false);
    });

    it("説明の範囲が1つずれれば、食い違う", () => {
      const planted = HANDLE_HINT.replace("0〜9", "1〜9");
      expect(planted).not.toBe(HANDLE_HINT);
      expect(sameCharacters(charactersNamedIn(planted), [...real])).toBe(false);
    });

    it("説明が同じ文字を2度名指せば、食い違う", () => {
      expect(sameCharacters(charactersNamedIn(`${HANDLE_HINT}「-」`), [...real])).toBe(false);
    });

    it("対照の対照: 植えた写し元が本物と同じなら、一致する", () => {
      const planted = `pub const ALLOWED_CHARACTERS: &'static str = "${real}";`;
      expect(sameCharacters(charactersNamedIn(HANDLE_HINT), allowedCharactersOf(planted))).toBe(true);
    });

    it("写し元から読めなければ落ちる（探せていないことを緑にしない）", () => {
      expect(() => allowedCharactersOf("pub const ALLOWED: &'static str = \"abc\";")).toThrow();
      expect(() =>
        allowedCharactersOf(
          `pub const ALLOWED_CHARACTERS: &'static str = "a";\npub const ALLOWED_CHARACTERS: &'static str = "b";`,
        ),
      ).toThrow();
    });
  });
});

// ----------------------------------------------------------------------

describe("記入例", () => {
  function placeholderOf(name: string): string | null {
    const found = target.querySelector(`[name="${name}"]`);
    expect(found, `${name} の欄があること`).not.toBeNull();
    return (found as Element).getAttribute("placeholder");
  }

  const NAMES = ["handle", "display_name", "base_persona", "output_format_instruction"] as const;

  it("作成のとき、各欄に記入例がある（値まで）", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });
    expect(NAMES.map((name) => placeholderOf(name))).toEqual([
      "aoi",
      "葵",
      "葵は大学2年生。落ち着いた口調で話し、相手の話を最後まで聞く。",
      "台詞は「」で囲み、動作や情景は「」の外に書く。",
    ]);
    await unmount(view, { outro: false });
  });

  it("欄の説明が、何を書くものかを言う", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });
    expect([
      flat(hint("display_name")),
      flat(hint("base_persona")),
      flat(hint("output_format_instruction")),
    ]).toEqual([
      "キャラクターの呼び名です。日本語で書けます。省くと識別子と同じ値になります。",
      "キャラクターの設定と話し方を書きます。会話のたびにモデルへ渡ります。",
      "返答の書き方の指示です。空にすると未設定に戻ります（この節はプロンプトから落ちます）。",
    ]);
    await unmount(view, { outro: false });
  });

  /** **編集のときは、書いてある値が見本の代わり。** 省いたときの話もしない。 */
  it("編集のときは、表示名の説明から「省くと」を言わない", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });
    expect(flat(hint("display_name"))).toBe("キャラクターの呼び名です。日本語で書けます。");
    await unmount(view, { outro: false });
  });
});

// ----------------------------------------------------------------------

describe("性格ステータスの尺度", () => {
  const MEANING = "性格ステータスの各軸に振れる値の上限です（0 からこの値まで）。";

  it("作成のとき、何の上限かと、省いたときの値を言う", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });
    expect(flat(hint("stat_scale_meaning"))).toBe(`${MEANING}省くと 10 になります。`);
    await unmount(view, { outro: false });
  });

  /** **描かれた数 = `StatScale::DEFAULT`。** 片方だけを変えれば赤になる。 */
  it("省いたときの値は、`src/domain/stat.rs` の `StatScale::DEFAULT` と一致する", async () => {
    const expected = statScaleDefaultOf(readRepo(STAT_RS));
    const view = mount(CharacterForm, { target, props: { character: null } });
    const found = /省くと (\d+) になります/u.exec(flat(hint("stat_scale_meaning")));
    expect(found?.[1]).toBeDefined();
    expect(Number.parseInt(found?.[1] as string, 10)).toBe(expected);
    await unmount(view, { outro: false });
  });

  it("編集のときは、何の上限かだけを言う（省けないので既定を言わない）", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });
    expect(flat(hint("stat_scale_meaning"))).toBe(MEANING);
    await unmount(view, { outro: false });
  });

  describe("対照: 植えたソースを、同じ読み方に通す", () => {
    it("`impl StatScale` の中の値を読む（他の型の `DEFAULT` を読まない）", () => {
      const planted =
        "impl Other {\n    pub const DEFAULT: Self = Self(3);\n}\nimpl StatScale {\n    pub const MIN: u32 = 1;\n    pub const DEFAULT: Self = Self(12);\n}\nimpl Next {\n    pub const DEFAULT: Self = Self(7);\n}";
      expect(statScaleDefaultOf(planted)).toBe(12);
    });

    it("写し元から読めなければ落ちる", () => {
      expect(() => statScaleDefaultOf("impl StatScale {\n    pub const MIN: u32 = 1;\n}\nimpl Next {\n    pub const DEFAULT: Self = Self(7);\n}")).toThrow();
      expect(() => statScaleDefaultOf("impl Other {}")).toThrow();
    });
  });
});
