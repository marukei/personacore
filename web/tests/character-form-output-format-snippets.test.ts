// @vitest-environment jsdom
/**
 * **作成・編集の画面の「出力形式指示」に、よく使う指示を差し込むボタン**
 * （作業単位 D-22 / `src/lib/CharacterForm.svelte` / `src/lib/outputFormatSnippets.ts`）。
 *
 * # ここが見るもの
 *
 * | 何 | なぜ |
 * |---|---|
 * | **4つのボタンが、この順・この文面で出る** | 期待値は**ここに直書き**（定数モジュールから作ると、文面の取り違えが自己成就する） |
 * | **空（空白だけを含む）の欄では、その文だけになる** | 保存時の `trim` と食い違わない（空白を残すと、見えない行頭が送られる） |
 * | **末尾が改行でなければ改行1つを挟み、改行なら挟まない** | 値全体を `toBe` で固定 |
 * | **同じ行があるボタンだけが押せない**（前後の空白は無視） | 同じ入力で、押せるものと押せないものを両方見る |
 * | **入れるとちょうど上限なら押せ、1字超えるなら押せない** | 数え方はカウンタと同じ（符号位置）。**絵文字を混ぜて、UTF-16 の数え方との違いが出る入力にしてある** |
 * | **保存中は押せない** | 送っている最中に下書きが変わらない |
 * | **差し込んだ値が `POST` / `PATCH` の本文に載る** | 欄から本文までの配線 |
 * | **他の欄は変わらない** | 差し込み先の取り違え |
 *
 * **ブラウザは起動しない**（jsdom だけ）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CharacterForm from "../src/lib/CharacterForm.svelte";
import type { CharacterView } from "../src/lib/api";

/** **期待する文面と順番。定数モジュールから作らない**（上の表）。 */
const EXPECTED = [
  "キャラクターの台詞は「」で囲む。",
  "地の文はそのまま出す。",
  "明確な心情描写は（）で囲む。",
  "利用者の台詞を代弁しない。",
];

/** 編集の対象。**形式指示は1行入っている**（末尾は改行でない）。 */
const MIO: CharacterView = {
  id: 2,
  handle: "mio",
  display_name: "澪",
  base_persona: "早口で話す。",
  output_format_instruction: "台詞は短く。",
  stat_scale: 5,
  stat_drift_mode: null,
  axis_discovery_slots: 0,
  response_length: null,
  archived_at: null,
  created_at: "2026-08-27T00:00:00Z",
};

interface Call {
  method: string;
  url: string;
  body: unknown;
}

let calls: Call[];
let target: HTMLElement;

function stubFetch(respondWith: () => unknown): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const raw = init?.body;
    calls.push({
      method: init?.method ?? "GET",
      url: String(input),
      body: typeof raw === "string" ? (JSON.parse(raw) as unknown) : null,
    });
    return Promise.resolve({
      ok: true,
      status: 200,
      text: () => Promise.resolve(JSON.stringify(respondWith())),
    } as unknown as Response);
  });
}

/** **応答を返さない** fetch（保存中のまま止めておくため）。 */
function stubPendingFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const raw = init?.body;
    calls.push({
      method: init?.method ?? "GET",
      url: String(input),
      body: typeof raw === "string" ? (JSON.parse(raw) as unknown) : null,
    });
    return new Promise<Response>(() => {});
  });
}

function field(name: string): HTMLInputElement | HTMLTextAreaElement {
  const found = target.querySelector(`[name="${name}"]`);
  expect(found, `${name} の欄があること`).not.toBeNull();
  return found as HTMLInputElement | HTMLTextAreaElement;
}

function type(name: string, value: string): void {
  const input = field(name);
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

function format(): string {
  return field("output_format_instruction").value;
}

function snippetButtons(): HTMLButtonElement[] {
  return [
    ...target.querySelectorAll<HTMLButtonElement>("[data-output-format-snippets] button"),
  ];
}

/** **その文面のボタン**（無ければ落とす）。 */
function snippetButton(text: string): HTMLButtonElement {
  const found = snippetButtons().find((button) => button.textContent?.trim() === text);
  expect(found, `「${text}」のボタンがあること`).toBeDefined();
  return found as HTMLButtonElement;
}

function press(text: string): void {
  snippetButton(text).click();
  flushSync();
}

/** ボタンごとの「押せない」を、表示順の配列で。 */
function disabledStates(): boolean[] {
  return snippetButtons().map((button) => button.disabled);
}

function submit(): void {
  (target.querySelector("form") as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  flushSync();
}

function writes(): Call[] {
  return calls.filter((call) => call.method !== "GET");
}

beforeEach(() => {
  calls = [];
  target = document.createElement("div");
  document.body.append(target);
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("並び", () => {
  it("4つのボタンが、この順・この文面で出る", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    expect(snippetButtons().map((button) => button.textContent?.trim())).toEqual(EXPECTED);
    // **送信ボタンにしない**（押すと保存が走る形を作らない）。
    expect(snippetButtons().map((button) => button.type)).toEqual([
      "button",
      "button",
      "button",
      "button",
    ]);

    await unmount(view, { outro: false });
  });

  it("編集の画面にも同じボタンが出る", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    expect(snippetButtons().map((button) => button.textContent?.trim())).toEqual(EXPECTED);

    await unmount(view, { outro: false });
  });

  it("欄の近くに、ボタンの働きと押せないときを書く", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    const hint = target.querySelector('[data-hint="output_format_snippets"]');
    expect(hint, "ボタンの案内があること").not.toBeNull();
    expect((hint as HTMLElement).textContent?.replace(/\s+/gu, "")).toBe(
      "下のボタンを押すと、その文が欄の末尾に1行で入ります。入れた後は自由に直せます。同じ行がすでにあるときと、入れると150字を超えるときは押せません。",
    );

    await unmount(view, { outro: false });
  });
});

/** **案内の文面**（直書き。比べる前に空白を落とす）。 */
const SNIPPET_HINT =
  "下のボタンを押すと、その文が欄の末尾に1行で入ります。入れた後は自由に直せます。同じ行がすでにあるときと、入れると150字を超えるときは押せません。";

describe("案内との結び付け", () => {
  /**
   * **各ボタンの `aria-describedby` が、案内の段落を指す**（読み上げで、押せない理由まで届く）。
   * 指す先は **`document` から**引く —— id の綴りが正しくても、描かれていなければ届かない。
   */
  it("各ボタンの `aria-describedby` が、案内の文面を指す", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    const described = snippetButtons().map((button) => {
      const id = button.getAttribute("aria-describedby") ?? "";
      const element = id === "" ? null : document.getElementById(id);
      return element?.textContent?.replace(/\s+/gu, "") ?? null;
    });
    expect(described).toEqual([SNIPPET_HINT, SNIPPET_HINT, SNIPPET_HINT, SNIPPET_HINT]);

    await unmount(view, { outro: false });
  });

  /**
   * **画面に2つ出ても id が衝突しない**（いまの一覧は1つずつしか出さないが、
   * 部品としては同時に複数出しうる）。**ボタンは自分の部品の案内を指す。**
   */
  it("2つ同時に出しても、ボタンはそれぞれ自分の案内を指す", async () => {
    const other = document.createElement("div");
    document.body.append(other);
    const first = mount(CharacterForm, { target, props: { character: null } });
    const second = mount(CharacterForm, { target: other, props: { character: MIO } });

    const ids = [target, other].map((root) => {
      const hint = root.querySelector('[data-hint="output_format_snippets"]');
      return hint?.id ?? "";
    });
    expect(ids[0], "id があること").not.toBe("");
    expect(ids[0]).not.toBe(ids[1]);
    for (const [index, root] of [target, other].entries()) {
      const pointed = [
        ...root.querySelectorAll<HTMLButtonElement>("[data-output-format-snippets] button"),
      ].map((button) => button.getAttribute("aria-describedby"));
      expect(pointed).toEqual([ids[index], ids[index], ids[index], ids[index]]);
    }

    await unmount(first, { outro: false });
    await unmount(second, { outro: false });
    other.remove();
  });
});

describe("差し込み", () => {
  it("空の欄で真ん中のボタンを押すと、その文だけになる", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    press("明確な心情描写は（）で囲む。");
    expect(format()).toBe("明確な心情描写は（）で囲む。");

    await unmount(view, { outro: false });
  });

  it("空白だけの欄は空として扱い、その文だけになる", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("output_format_instruction", "  \n ");
    press("地の文はそのまま出す。");
    expect(format()).toBe("地の文はそのまま出す。");

    await unmount(view, { outro: false });
  });

  it("末尾が改行でない中身には、改行1つを挟んで足す", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("output_format_instruction", "台詞は短く。");
    press("地の文はそのまま出す。");
    expect(format()).toBe("台詞は短く。\n地の文はそのまま出す。");

    await unmount(view, { outro: false });
  });

  it("末尾が改行の中身には、改行を挟まずに足す", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("output_format_instruction", "台詞は短く。\n");
    press("地の文はそのまま出す。");
    expect(format()).toBe("台詞は短く。\n地の文はそのまま出す。");

    await unmount(view, { outro: false });
  });

  it("続けて押すと、1行ずつ積まれ、その後も自由に直せる", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    press("利用者の台詞を代弁しない。");
    press("キャラクターの台詞は「」で囲む。");
    expect(format()).toBe("利用者の台詞を代弁しない。\nキャラクターの台詞は「」で囲む。");

    type("output_format_instruction", "利用者の台詞を代弁しない。");
    expect(format()).toBe("利用者の台詞を代弁しない。");

    await unmount(view, { outro: false });
  });

  it("押しても他の欄は変わらない", async () => {
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    press("明確な心情描写は（）で囲む。");
    expect([
      field("display_name").value,
      field("base_persona").value,
      field("stat_scale").value,
      field("response_length_min").value,
      field("response_length_max").value,
    ]).toEqual(["澪", "早口で話す。", "5", "", ""]);
    expect(format()).toBe("台詞は短く。\n明確な心情描写は（）で囲む。");

    await unmount(view, { outro: false });
  });
});

describe("押せないとき", () => {
  it("同じ行があるボタンだけが押せず、他は押せる", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("output_format_instruction", "台詞は短く。\n地の文はそのまま出す。\n最後の行。");
    expect(disabledStates()).toEqual([false, true, false, false]);

    await unmount(view, { outro: false });
  });

  it("前後に空白が付いた同じ行でも押せない", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("output_format_instruction", "台詞は短く。\n  明確な心情描写は（）で囲む。\t\n最後の行。");
    expect(disabledStates()).toEqual([false, false, true, false]);

    await unmount(view, { outro: false });
  });

  /**
   * **CRLF で保存された値**（サーバは改行を正規化しない）。行の比較は `\n` で割って
   * trim するので `\r` は落ち、同じ行として見つかる。**差し込みは `\n` を足す**
   * （既存の `\r\n` は書き換えない）。
   *
   * **⚠ 差し込んだ結果は `PATCH` の本文で見る。** `<textarea>` の `value` は
   * HTML の約束で CRLF を LF に畳んで返すので、欄の値を読んでも下書きの `\r` は見えない
   * （下書き＝送る値は `\r\n` を持ったまま）。
   */
  it("CRLF で保存された値でも同じ行を見つけ、差し込みは改行1つを足す", async () => {
    const crlf: CharacterView = {
      ...MIO,
      output_format_instruction: "台詞は短く。\r\n地の文はそのまま出す。",
    };
    stubFetch(() => ({ id: crlf.id, character: crlf }));
    const view = mount(CharacterForm, { target, props: { character: crlf } });

    expect(disabledStates()).toEqual([false, true, false, false]);
    press("明確な心情描写は（）で囲む。");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });
    const sent = (writes()[0]?.body as { output_format_instruction?: unknown })
      .output_format_instruction;
    expect(sent).toBe("台詞は短く。\r\n地の文はそのまま出す。\n明確な心情描写は（）で囲む。");

    await unmount(view, { outro: false });
  });

  /**
   * **上の1本では、同じ行が最後の行にある**（`\r` が付かない）ので、
   * 比較から `\r` の落とし方を外しても赤にならない（変異 M11 で実測）。
   * **ここでは同じ行を最後でない位置に置く** —— その行は `\r` を末尾に持つ。
   */
  it("CRLF で保存された値の、最後でない行も同じ行として見つける", async () => {
    const crlf: CharacterView = {
      ...MIO,
      output_format_instruction: "利用者の台詞を代弁しない。\r\n台詞は短く。",
    };
    const view = mount(CharacterForm, { target, props: { character: crlf } });

    expect(disabledStates()).toEqual([false, false, false, true]);

    await unmount(view, { outro: false });
  });

  it("文の一部を含むだけの行では押せる", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("output_format_instruction", "地の文はそのまま出す。ただし短く。");
    expect(disabledStates()).toEqual([false, false, false, false]);

    await unmount(view, { outro: false });
  });

  /**
   * **境目は「地の文はそのまま出す。」（11 字）＋ 改行 1 字。**
   * 中身 138 字 → 150 字ちょうど（押せる）、139 字 → 151 字（押せない）。
   *
   * **絵文字を 10 個混ぜてある**（UTF-16 では 1 個 2 単位）。
   * `.length` で数えると、138 字の側も 160 になって押せなくなる。
   */
  it("入れるとちょうど上限なら押せ、1字超えるなら押せない", async () => {
    const view = mount(CharacterForm, { target, props: { character: null } });

    const fits = `${"🐈".repeat(10)}${"あ".repeat(128)}`;
    type("output_format_instruction", fits);
    expect(snippetButton("地の文はそのまま出す。").disabled, "150 字ちょうどなら押せる").toBe(
      false,
    );

    type("output_format_instruction", `${fits}あ`);
    expect(snippetButton("地の文はそのまま出す。").disabled, "151 字になるなら押せない").toBe(
      true,
    );

    type("output_format_instruction", fits);
    press("地の文はそのまま出す。");
    expect(format()).toBe(`${fits}\n地の文はそのまま出す。`);
    // **カウンタと同じ数え方であること**（押せた結果がちょうど上限）。
    expect(
      target.querySelector('[data-counter="output_format_instruction"]')?.textContent?.trim(),
    ).toBe("150 / 150");

    await unmount(view, { outro: false });
  });

  it("保存中は押せない", async () => {
    stubPendingFetch();
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    type("display_name", "碧");
    expect(disabledStates(), "保存の前は押せる").toEqual([false, false, false, false]);
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });
    flushSync();
    expect(disabledStates(), "保存中は押せない").toEqual([true, true, true, true]);

    await unmount(view, { outro: false });
  });
});

describe("送る本文", () => {
  it("編集では、差し込んだ値がそのまま `PATCH` に載る", async () => {
    stubFetch(() => ({
      id: MIO.id,
      character: {
        ...MIO,
        output_format_instruction: "台詞は短く。\n明確な心情描写は（）で囲む。",
      },
    }));
    const view = mount(CharacterForm, { target, props: { character: MIO } });

    press("明確な心情描写は（）で囲む。");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });
    expect(writes()).toEqual([
      {
        method: "PATCH",
        url: "/admin/characters/mio",
        body: { output_format_instruction: "台詞は短く。\n明確な心情描写は（）で囲む。" },
      },
    ]);

    await unmount(view, { outro: false });
  });

  it("作成では、差し込んだ値がそのまま `POST` に載る", async () => {
    const created: CharacterView = {
      ...MIO,
      id: 9,
      handle: "rin",
      display_name: "rin",
      output_format_instruction: "地の文はそのまま出す。\n利用者の台詞を代弁しない。",
    };
    stubFetch(() => created);
    const view = mount(CharacterForm, { target, props: { character: null } });

    type("handle", "rin");
    type("base_persona", "早口で話す。");
    press("地の文はそのまま出す。");
    press("利用者の台詞を代弁しない。");
    submit();
    await vi.waitFor(() => {
      expect(writes()).toHaveLength(1);
    });
    expect(writes()).toEqual([
      {
        method: "POST",
        url: "/admin/characters",
        body: {
          handle: "rin",
          base_persona: "早口で話す。",
          output_format_instruction: "地の文はそのまま出す。\n利用者の台詞を代弁しない。",
        },
      },
    ]);

    await unmount(view, { outro: false });
  });
});
