// @vitest-environment jsdom
/**
 * **プロファイルの編集の鍵の欄と、保存の前の確認**（作業単位 A-13b-3 / Issue origin#86 / `src/lib/UpstreamProfiles.svelte`）。
 *
 * # 見るもの（**配線** —— 下書き → 画面 → 送る本文）
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 鍵と承認の3通りが依頼の何になるか・いつ何を聞くか（組み合わせ） | `upstream-profiles-key-draft.test.ts`（純粋関数） |
 * | **鍵の欄が繋ぐ面に在り、値を出さず、設定済みかと末尾4文字を言う** | **ここ** |
 * | **新しい鍵・消す・触らないが、`PUT` の本文の `api_key` になる** | **ここ** |
 * | **宛先だけを変えた保存の前に聞き、答えで本文が変わる。やめれば送らない** | **ここ** |
 * | **外の `http://` の鍵の保存の前に、危険を示して承認を取る** | **ここ** |
 * | **ローカルネットワークの警告・承認の状態が、その面の鍵の欄の隣に出る** | **ここ** |
 * | **裏方のチェックを外すと、裏方の鍵が消えることをチェックの隣で言う** | **ここ** |
 *
 * # ⚠ 帰属
 *
 * 一覧は3件で、**直すのは真ん中（`relay`）**。面ごとに宛先・末尾4文字・承認の有無を全部違える。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import UpstreamProfiles from "../src/lib/UpstreamProfiles.svelte";
import type { ProfilesView, WrittenFace, WrittenProfile } from "../src/lib/api";

function connect(endpoint: string, model: string, key: { hint: string | null; approved: boolean } | null): WrittenFace {
  return {
    endpoint,
    model,
    gguf: null,
    args: null,
    command: null,
    context_tokens: null,
    api_key_set: key !== null,
    api_key_hint: key?.hint ?? null,
    allow_key_over_plain_http: key?.approved ?? false,
    sampling: null,
    response_tokens: null,
  };
}

/** 先頭。**本文は外の `http://`（鍵あり・承認あり）。** */
const FAR: WrittenProfile = {
  name: "far",
  main: connect("http://203.0.113.5:47833", "far-main", { hint: "F4r1", approved: true }),
  sub: null,
};

/** **真ん中（直すのはこれ）。** 本文はループバック、裏方はローカルネットワーク。どちらも鍵あり・末尾は別。 */
const KEY_PROTECTED_RELAY: WrittenProfile = {
  name: "relay",
  main: connect("http://127.0.0.1:46001", "relay-main", { hint: "k3Qz", approved: false }),
  sub: connect("http://192.168.0.7:47832", "relay-sub", { hint: "S9x2", approved: false }),
};

/** **起こす面に鍵を書いた**（`args` の `--api-key` と組 —— llama-server の鍵）。 */
const TAIL: WrittenProfile = {
  name: "tail",
  main: {
    ...connect("http://127.0.0.1:47835", "tail-main", { hint: "T4il", approved: false }),
    gguf: "~/m/tail.gguf",
    args: ["--api-key", "pc-tail-key-T4il"],
  },
  sub: null,
};

/** **ホスト名の宛先に鍵を書いた・承認なし**（`http://relay.example:…` のように、宛先を名前で書いた中継）。裏方も別のホスト名で、鍵あり・承認なし。 */
const NAMED: WrittenProfile = {
  name: "named",
  main: connect("http://relay.example:46001", "named-main", { hint: "Nm3d", approved: false }),
  sub: connect("http://sub.local:47832", "named-sub", { hint: "Sb4n", approved: false }),
};

function listing(): ProfilesView {
  return {
    path: "C:\\Users\\someone\\AppData\\Local\\personacore\\upstreams.json",
    editable: true,
    state: "ok",
    revision: "rev-1",
    problem: null,
    profiles: [FAR, KEY_PROTECTED_RELAY, TAIL, NAMED],
  };
}

/** `scrollIntoView` を呼ばれた要素（jsdom には実装が無いので、測定が据える）。 */
let scrolled: Element[];

const MAIN = "本文生成";
const SUB = "裏方（抽出・変容）";
const KEY_LABEL = "鍵（api_key）";
const ENDPOINT = "宛先（endpoint）";

/** 囮の新しい鍵（どの末尾4文字とも違う）。 */
const NEW_KEY = "pc-typed-new-key-TY07";

let target: HTMLElement;
let saved: { name: string; body: unknown }[];
let saveReply: () => { status: number; body: unknown };
let view: ReturnType<typeof mount> | null;

beforeEach(() => {
  scrolled = [];
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    writable: true,
    value: function (this: Element) {
      scrolled.push(this);
    },
  });
  saved = [];
  saveReply = () => ({ status: 200, body: listing() });
  view = null;
  target = document.createElement("div");
  document.body.append(target);
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    let reply: { status: number; body: unknown };
    if (method === "GET" && url === "/admin/upstream/profiles") {
      reply = { status: 200, body: listing() };
    } else if (method === "PUT" && url.startsWith("/admin/upstream/profiles/")) {
      saved.push({
        name: decodeURIComponent(url.slice("/admin/upstream/profiles/".length)),
        body: JSON.parse(String(init?.body)) as unknown,
      });
      reply = saveReply();
    } else {
      reply = { status: 404, body: "not found" };
    }
    return Promise.resolve({
      ok: reply.status >= 200 && reply.status < 300,
      status: reply.status,
      text: () => Promise.resolve(typeof reply.body === "string" ? reply.body : JSON.stringify(reply.body)),
    } as unknown as Response);
  });
});

afterEach(async () => {
  if (view !== null) {
    await unmount(view, { outro: false });
  }
  vi.unstubAllGlobals();
  delete (HTMLElement.prototype as { scrollIntoView?: unknown }).scrollIntoView;
  target.remove();
});

function flat(element: Element | null | undefined): string {
  return (element?.textContent ?? "").replace(/\s+/gu, " ").trim();
}

function buttonIn(scope: Element, name: string): HTMLButtonElement | undefined {
  return [...scope.querySelectorAll("button")].find((button) => flat(button) === name);
}

function mustButton(scope: Element, name: string): HTMLButtonElement {
  const button = buttonIn(scope, name);
  expect(button, `ボタン「${name}」があること`).toBeDefined();
  return button as HTMLButtonElement;
}

/** プロファイル1件の塊（`h3` の名前で引く）。 */
function profileBlock(name: string): HTMLElement {
  const found = [...target.querySelectorAll("section.profile")].filter(
    (section) => flat(section.querySelector(":scope > h3")) === name,
  );
  expect(found, `プロファイル「${name}」の塊が1つだけあること`).toHaveLength(1);
  return found[0] as HTMLElement;
}

async function openEditor(name: string): Promise<void> {
  view = mount(UpstreamProfiles, { target, props: { onswitched: () => undefined, selected: null } });
  await vi.waitFor(() => {
    expect(target.querySelectorAll("section.profile")).toHaveLength(4);
  });
  mustButton(profileBlock(name), "直す").click();
  await vi.waitFor(() => {
    expect(target.querySelector("form")).not.toBeNull();
  });
}

function form(): HTMLFormElement {
  const found = target.querySelector("form");
  expect(found, "下書きが開いていること").not.toBeNull();
  return found as HTMLFormElement;
}

function fieldsetOf(legend: string): HTMLFieldSetElement {
  const found = [...form().querySelectorAll("fieldset")].find((fieldset) => flat(fieldset.querySelector("legend")) === legend);
  expect(found, `面「${legend}」の欄の組があること`).toBeDefined();
  return found as HTMLFieldSetElement;
}

/** その面の、その見出しの欄（**`<label for>` で引く**）。 */
function fieldOf(legend: string, label: string): HTMLInputElement {
  const labelled = [...fieldsetOf(legend).querySelectorAll("label")].find((found) => flat(found) === label);
  expect(labelled, `欄「${label}」があること`).toBeDefined();
  const control = document.getElementById((labelled as HTMLLabelElement).htmlFor);
  expect(control, `欄「${label}」が見出しと結びついていること`).not.toBeNull();
  return control as HTMLInputElement;
}

/** **鍵の欄の囲み**（見出しと入力欄の親）。注記・ボタンはこの中に在る。 */
function keyBox(legend: string): HTMLElement {
  return fieldOf(legend, KEY_LABEL).parentElement as HTMLElement;
}

function type(control: HTMLInputElement, value: string): void {
  control.value = value;
  control.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

/** 保存を押す（`submit` を投げる）。 */
function submit(): void {
  form().dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
}

/** 保存の前の確認（`role="alertdialog"`）。無ければ `null`。 */
function question(): HTMLElement | null {
  return form().querySelector('[role="alertdialog"]');
}

async function waitQuestion(): Promise<HTMLElement> {
  return vi.waitFor(() => {
    const found = question();
    expect(found, "確認が出ていること").not.toBeNull();
    return found as HTMLElement;
  });
}

/** 確認の見出しと段落（`aria-labelledby` / `aria-describedby` が指す要素の文面。段落は1つずつ）。 */
function questionText(dialog: HTMLElement): { title: string; body: string[] } {
  const body = document.getElementById(dialog.getAttribute("aria-describedby") ?? "");
  expect(body, "確認の段落が aria-describedby で結びついていること").not.toBeNull();
  return {
    title: flat(document.getElementById(dialog.getAttribute("aria-labelledby") ?? "")),
    body: [...(body as HTMLElement).querySelectorAll("p")].map((p) => flat(p)),
  };
}

async function waitSaved(count: number): Promise<{ name: string; body: Record<string, Record<string, unknown> | null> }> {
  await vi.waitFor(() => expect(saved).toHaveLength(count));
  return saved[count - 1] as { name: string; body: Record<string, Record<string, unknown> | null> };
}

describe("鍵の欄", () => {
  /**
   * **繋ぐ面に鍵の欄がある。値は出さず、設定済みかと末尾4文字を、その面の欄の隣で言う。**
   * 入力欄は開いたときに空で、伏せ字の欄（`password`）。
   */
  it("繋ぐ面の鍵の欄は空で開き、その面の末尾4文字で設定済みと言う", async () => {
    await openEditor("relay");
    const main = fieldOf(MAIN, KEY_LABEL);
    const sub = fieldOf(SUB, KEY_LABEL);
    expect([main.type, sub.type]).toEqual(["password", "password"]);
    // **閲覧器の保存・補完に拾わせない**（レビュー是正 —— 削っても緑だった）。
    expect([main.getAttribute("autocomplete"), sub.getAttribute("autocomplete")]).toEqual(["off", "off"]);
    expect([main.value, sub.value]).toEqual(["", ""]);
    expect(flat(keyBox(MAIN))).toContain("設定済みです（…k3Qz）。");
    expect(flat(keyBox(SUB))).toContain("設定済みです（…S9x2）。");
    expect(flat(keyBox(MAIN))).not.toContain("S9x2");
  });

  it("新しい鍵を書いた面だけ、その鍵を送る", async () => {
    await openEditor("relay");
    type(fieldOf(SUB, KEY_LABEL), NEW_KEY);
    expect(flat(keyBox(SUB))).toContain("保存すると、新しい鍵に置き換えます。");
    submit();
    const request = await waitSaved(1);
    expect(request.name).toBe("relay");
    expect(request.body["sub"]?.["api_key"]).toBe(NEW_KEY);
    expect(Object.keys(request.body["main"] ?? {})).not.toContain("api_key");
    // **書いた鍵は、送った後に画面のどこにも出ない**（欄の値も含めて —— 保存で下書きは閉じる）。
    await vi.waitFor(() => expect(target.querySelector("form")).toBeNull());
    expect(target.innerHTML).not.toContain(NEW_KEY);
  });

  it("「鍵を消す」で `null` を送る。「消すのをやめる」で戻せる", async () => {
    await openEditor("relay");
    mustButton(keyBox(MAIN), "鍵を消す").click();
    flushSync();
    expect(flat(keyBox(MAIN))).toContain("保存すると、鍵を消します。");
    expect(fieldOf(MAIN, KEY_LABEL).disabled).toBe(true);
    mustButton(keyBox(MAIN), "消すのをやめる").click();
    flushSync();
    expect(flat(keyBox(MAIN))).toContain("設定済みです（…k3Qz）。");
    expect(fieldOf(MAIN, KEY_LABEL).disabled).toBe(false);
    mustButton(keyBox(MAIN), "鍵を消す").click();
    flushSync();
    submit();
    const request = await waitSaved(1);
    expect(request.body["main"]?.["api_key"]).toBeNull();
    expect(Object.keys(request.body["sub"] ?? {})).not.toContain("api_key");
  });

  it("触らなければ、どちらの面も `api_key` を送らず、何も聞かない", async () => {
    await openEditor("relay");
    submit();
    const request = await waitSaved(1);
    expect(question()).toBeNull();
    for (const face of ["main", "sub"]) {
      expect(Object.keys(request.body[face] ?? {}), face).not.toContain("api_key");
    }
  });

  /** **鍵を名指した 422 は、その面の鍵の欄に結びつく**（作業単位 A-13b-3 —— 着手前は全体に出していた）。 */
  it("鍵を名指した 422 は、その面の鍵の欄に出る", async () => {
    saveReply = () => ({
      status: 422,
      body: {
        error: {
          message: "上流プロファイルを保存できません",
          type: "invalid_request_error",
          code: "upstream_profile_invalid",
          reason: "api_key_unusable",
          reason_message: "鍵に使えない字があります（空白・改行・制御文字・ASCII の外の字は使えません）",
          face: "sub",
          key: "api_key",
        },
      },
    });
    await openEditor("relay");
    type(fieldOf(SUB, KEY_LABEL), "has space");
    submit();
    await waitSaved(1);
    const control = fieldOf(SUB, KEY_LABEL);
    await vi.waitFor(() => expect(control.getAttribute("aria-invalid")).toBe("true"));
    expect(flat(document.getElementById(control.getAttribute("aria-describedby") ?? ""))).toBe(
      "保存しませんでした: 鍵に使えない字があります（空白・改行・制御文字・ASCII の外の字は使えません）",
    );
    expect(fieldOf(MAIN, KEY_LABEL).getAttribute("aria-invalid")).toBeNull();
  });

  /**
   * **起こす面にも鍵の欄がある。モデル名だけを直して保存しても、鍵は送らない（＝残る）**
   * （作業単位 A-13b-3 のレビュー是正。着手時は欄が無く、`null` で送って鍵を消していた）。
   */
  it("起こす面にも鍵の欄が出て、モデル名だけを直して保存しても鍵を送らない", async () => {
    await openEditor("tail");
    expect(fieldOf(MAIN, KEY_LABEL).type).toBe("password");
    expect(flat(keyBox(MAIN))).toContain("設定済みです（…T4il）。");
    expect(flat(fieldsetOf(MAIN))).not.toContain("保存すると鍵は消えます");
    type(fieldOf(MAIN, "依頼するモデルの綴り（model）"), "tail-main-2");
    submit();
    const request = await waitSaved(1);
    expect(request.body["main"]?.["model"]).toBe("tail-main-2");
    expect(Object.keys(request.body["main"] ?? {})).not.toContain("api_key");
  });

  /**
   * **書いた新しい鍵を、下書きに残さない**（作業単位 A-13b-3 のレビュー是正）。書いて「やめる」→ 開き直すと空。
   *
   * ⚠ 開き直しは下書きを作り直すので、この1本は是正の前から緑（`forgetTypedKeys` を外しても緑）。
   * 是正は「閉じた下書きの中に鍵を持ち続けない」ことで、それを外から観測する口は無い。
   */
  it("新しい鍵を書いて「やめる」と、開き直したとき鍵の欄は空", async () => {
    await openEditor("relay");
    type(fieldOf(MAIN, KEY_LABEL), NEW_KEY);
    mustButton(form(), "やめる").click();
    await vi.waitFor(() => expect(target.querySelector("form")).toBeNull());
    expect(target.innerHTML).not.toContain(NEW_KEY);
    mustButton(profileBlock("relay"), "直す").click();
    await vi.waitFor(() => expect(target.querySelector("form")).not.toBeNull());
    expect(fieldOf(MAIN, KEY_LABEL).value).toBe("");
  });
});

describe("宛先だけを変えたときの鍵の確認（利用者の決定1）", () => {
  it("保存の前に、その面の鍵を新しい宛先へ送るかを聞き、答えるまで送らない", async () => {
    await openEditor("relay");
    type(fieldOf(MAIN, ENDPOINT), "http://127.0.0.1:47999");
    submit();
    const dialog = await waitQuestion();
    expect(questionText(dialog)).toEqual({
      title: "本文生成の鍵を、新しい宛先へも送りますか",
      body: [
        "この面には鍵が書いてあります（…k3Qz）。宛先を http://127.0.0.1:46001 から http://127.0.0.1:47999 に変えました。",
        "鍵は宛先に付くものなので、新しい宛先へ送ってよいかを確かめます。鍵を消すと、この面は鍵を送らなくなります。",
      ],
    });
    expect(saved).toHaveLength(0);
    mustButton(dialog, "送る").click();
    const request = await waitSaved(1);
    expect(request.body["main"]?.["endpoint"]).toBe("http://127.0.0.1:47999");
    expect(Object.keys(request.body["main"] ?? {})).not.toContain("api_key");
  });

  it("「鍵を消す」と答えると、その面の鍵を `null` で送る", async () => {
    await openEditor("relay");
    type(fieldOf(SUB, ENDPOINT), "http://192.168.0.8:47832");
    submit();
    const dialog = await waitQuestion();
    expect(questionText(dialog).title).toBe("裏方（抽出・変容）の鍵を、新しい宛先へも送りますか");
    mustButton(dialog, "鍵を消す").click();
    const request = await waitSaved(1);
    expect(request.body["sub"]?.["api_key"]).toBeNull();
    expect(Object.keys(request.body["main"] ?? {})).not.toContain("api_key");
  });

  it("「保存をやめる」と答えると送らず、下書きは開いたまま", async () => {
    await openEditor("relay");
    type(fieldOf(MAIN, ENDPOINT), "http://127.0.0.1:47999");
    submit();
    const dialog = await waitQuestion();
    mustButton(dialog, "保存をやめる").click();
    flushSync();
    expect(question()).toBeNull();
    expect(saved).toHaveLength(0);
    expect(fieldOf(MAIN, ENDPOINT).value).toBe("http://127.0.0.1:47999");
  });

  /** **起こす面にも効く**（利用者の決定1 / レビュー是正）。宛先はループバックなので、承認は聞かない。 */
  it("起こす面で待ち受ける場所だけを変えても、鍵を送るかを聞き、承認は聞かない", async () => {
    await openEditor("tail");
    type(fieldOf(MAIN, "待ち受ける場所（endpoint）"), "http://127.0.0.1:47999");
    submit();
    const dialog = await waitQuestion();
    expect(questionText(dialog).title).toBe("本文生成の鍵を、新しい宛先へも送りますか");
    mustButton(dialog, "送る").click();
    const request = await waitSaved(1);
    expect(Object.keys(request.body["main"] ?? {})).not.toContain("api_key");
  });

  /**
   * **確認が出たら、フォーカスを確認の見出しへ移し、確認を巻き取って見せる。出ている間は「保存する」を押せない。
   * 閉じたらフォーカスを「保存する」へ戻す**（作業単位 A-13b-3 のレビュー是正 / 司令塔が画面で見た —— 確認が画面の下へ
   * はみ出し、フォーカスが「保存する」に残っていた）。位置は jsdom では測れないので、`scrollIntoView` が確認に呼ばれたことを見る。
   */
  it("確認が出るとフォーカスが確認へ移って巻き取られ、「保存する」は押せず、閉じるとフォーカスが戻る", async () => {
    await openEditor("relay");
    const save = mustButton(form(), "保存する");
    type(fieldOf(MAIN, ENDPOINT), "http://127.0.0.1:47999");
    save.focus();
    submit();
    const dialog = await waitQuestion();
    const title = document.getElementById(dialog.getAttribute("aria-labelledby") ?? "");
    await vi.waitFor(() => expect(document.activeElement).toBe(title));
    expect(dialog.contains(document.activeElement)).toBe(true);
    expect(scrolled).toContain(dialog);
    expect(save.disabled).toBe(true);
    // 押せない間に submit が来ても、送らない（確認は1つのまま）。
    submit();
    flushSync();
    expect(saved).toHaveLength(0);
    expect(form().querySelectorAll('[role="alertdialog"]')).toHaveLength(1);

    mustButton(dialog, "保存をやめる").click();
    await vi.waitFor(() => expect(document.activeElement).toBe(save));
    expect(save.disabled).toBe(false);
  });

  /** **答えて次の確認が出たら、フォーカスは次の確認へ移る**（鍵の確認 → 承認）。 */
  it("鍵の確認に答えて承認の確認が出ると、フォーカスは承認の確認へ移る", async () => {
    await openEditor("far");
    type(fieldOf(MAIN, ENDPOINT), "http://203.0.113.7:47833");
    submit();
    mustButton(await waitQuestion(), "送る").click();
    await vi.waitFor(() => {
      const next = question() as HTMLElement;
      expect(questionText(next).title).toBe("本文生成の鍵を、暗号化せずに送ることを承認しますか");
      expect(document.activeElement).toBe(document.getElementById(next.getAttribute("aria-labelledby") ?? ""));
      expect(scrolled).toContain(next);
    });
  });
});

describe("ホスト名の宛先（作業単位 A-13b-3 のレビュー是正）", () => {
  const NAME_NOTE =
    "名前で書いた宛先は、送るたびに名前解決して判定します。ローカルネットワークの外に解決したら、承認が無ければ送りません。";

  /**
   * **保存の前に止めない。** 承認の無いホスト名の面は、ほかの欄を直すたびに聞かれていた（「承認せずに保存する」は
   * 何も残さない）。いまは欄の隣で言い、「承認する」を置くだけ。
   */
  it("ほかの欄を直して保存しても聞かず、欄の隣で名前解決で判定すると言う", async () => {
    await openEditor("named");
    expect(flat(keyBox(MAIN))).toContain(NAME_NOTE);
    expect(buttonIn(keyBox(MAIN), "承認する")).toBeDefined();
    type(fieldOf(MAIN, "依頼するモデルの綴り（model）"), "named-main-2");
    submit();
    const request = await waitSaved(1);
    expect(question()).toBeNull();
    expect(Object.keys(request.body["main"] ?? {})).not.toContain("allow_key_over_plain_http");
  });

  /**
   * **「承認する」は、保存の前の確認と同じ危険を示して聞く。** 「承認する」と答えれば保存で `true` を送る。
   * 聞いている間は「保存する」を押せず、フォーカスは確認の見出しにある。
   */
  it("「承認する」で危険を示して聞き、承認すれば保存で `true` を送る", async () => {
    await openEditor("named");
    mustButton(keyBox(MAIN), "承認する").click();
    const dialog = await vi.waitFor(() => {
      const found = keyBox(MAIN).querySelector('[role="alertdialog"]');
      expect(found).not.toBeNull();
      return found as HTMLElement;
    });
    const text = questionText(dialog);
    expect(text.title).toBe("本文生成の鍵を、暗号化せずに送ることを承認しますか");
    expect(text.body).toContain(
      "承認すると、鍵は暗号化されずに送られます。経路の途中で記録されれば、その記録は残り、鍵が悪用されることがあります。",
    );
    await vi.waitFor(() =>
      expect(document.activeElement).toBe(document.getElementById(dialog.getAttribute("aria-labelledby") ?? "")),
    );
    expect(mustButton(form(), "保存する").disabled).toBe(true);
    expect(saved).toHaveLength(0);
    mustButton(dialog, "承認する").click();
    flushSync();
    expect(keyBox(MAIN).querySelector('[role="alertdialog"]')).toBeNull();
    expect(flat(keyBox(MAIN))).toContain("保存すると、この宛先へ鍵を暗号化せずに送ることを承認します。");
    submit();
    const request = await waitSaved(1);
    expect(request.body["main"]?.["allow_key_over_plain_http"]).toBe(true);
  });

  it("「承認する」の確認で「やめる」と答えれば、何も変えない", async () => {
    await openEditor("named");
    mustButton(keyBox(MAIN), "承認する").click();
    const dialog = await vi.waitFor(() => {
      const found = keyBox(MAIN).querySelector('[role="alertdialog"]');
      expect(found).not.toBeNull();
      return found as HTMLElement;
    });
    mustButton(dialog, "やめる").click();
    flushSync();
    expect(flat(keyBox(MAIN))).toContain(NAME_NOTE);
    submit();
    const request = await waitSaved(1);
    expect(Object.keys(request.body["main"] ?? {})).not.toContain("allow_key_over_plain_http");
  });

  /** その面の「承認する」の確認を出して、出たことを確かめる。 */
  async function openNameApproval(legend: string): Promise<void> {
    mustButton(keyBox(legend), "承認する").click();
    await vi.waitFor(() => expect(keyBox(legend).querySelector('[role="alertdialog"]')).not.toBeNull());
    expect(mustButton(form(), "保存する").disabled, "前提: 確認の間は押せない").toBe(true);
  }

  /**
   * **確認が消えたら「保存する」は押せる状態に戻る**（作業単位 A-13b-3 の再レビュー是正）。
   *
   * 是正前は、確認を出したまま確認の条件を外す操作（鍵を消す・宛先を変える・裏方のチェックを外す）をすると、
   * 確認は消えるのに「保存する」が理由の出ないまま押せなくなった（Enter の保存も無視された）。
   * 3つの操作それぞれで、**押せる・送れる**ことを見る。
   */
  it.each([
    ["鍵を消す", MAIN, () => mustButton(keyBox(MAIN), "鍵を消す").click(), false],
    // 宛先を変えたので、保存では鍵の確認（送るか）が出る —— 「送る」と答えて送れること（submit が無視されないこと）を見る。
    ["宛先を https:// に変える", MAIN, () => type(fieldOf(MAIN, ENDPOINT), "https://relay.example:46001"), true],
    [
      "裏方のチェックを外す",
      SUB,
      () => (form().querySelector("input[type=checkbox]") as HTMLInputElement).click(),
      false,
    ],
  ] as const)("確認を出したまま「%s」と、確認が消えて「保存する」が押せる", async (_name, legend, act, carries) => {
    await openEditor("named");
    await openNameApproval(legend);
    act();
    flushSync();
    expect(form().querySelectorAll('[role="alertdialog"]')).toHaveLength(0);
    await vi.waitFor(() => expect(mustButton(form(), "保存する").disabled).toBe(false));
    submit();
    if (carries) {
      mustButton(await waitQuestion(), "送る").click();
    }
    await waitSaved(1);
  });

  /**
   * **条件を外れたら、聞いていた面を忘れる** —— 条件が戻っても、確認がひとりでに出直さない
   * （再レビュー是正。忘れないと、消すのをやめた・チェックを入れ直した瞬間に、押していない確認が現れる）。
   */
  it.each([
    [
      "鍵を消して、消すのをやめる",
      MAIN,
      () => mustButton(keyBox(MAIN), "鍵を消す").click(),
      () => mustButton(keyBox(MAIN), "消すのをやめる").click(),
    ],
    [
      "裏方のチェックを外して、入れ直す",
      SUB,
      () => (form().querySelector("input[type=checkbox]") as HTMLInputElement).click(),
      () => (form().querySelector("input[type=checkbox]") as HTMLInputElement).click(),
    ],
  ] as const)("確認を出したまま「%s」と、確認は出直さない", async (_name, legend, leave, back) => {
    await openEditor("named");
    await openNameApproval(legend);
    leave();
    flushSync();
    back();
    flushSync();
    await vi.waitFor(() => expect(buttonIn(keyBox(legend), "承認する")).toBeDefined());
    expect(form().querySelectorAll('[role="alertdialog"]')).toHaveLength(0);
    expect(mustButton(form(), "保存する").disabled).toBe(false);
  });

  /**
   * **ほかの欄を打っている最中に確認が外れても、フォーカスを奪わない**（作業単位 A-13b-3 の再々レビュー是正）。
   *
   * 是正前は、確認が消えるとフォーカスが「保存する」へ飛び、宛先を `https://` へ打ち直す途中の文字が欄に入らなかった。
   * 「保存する」へ戻すのは、フォーカスが確認の中にあったときだけ。
   */
  it("宛先の欄を打っていて確認が外れても、フォーカスは宛先の欄のまま", async () => {
    await openEditor("named");
    await openNameApproval(MAIN);
    const endpoint = fieldOf(MAIN, ENDPOINT);
    endpoint.focus();
    expect(document.activeElement).toBe(endpoint);
    type(endpoint, "https://relay.example:46001");
    expect(form().querySelectorAll('[role="alertdialog"]')).toHaveLength(0);
    // 戻す処理は `tick()` の後に走るので、何周か待ってから見る。
    for (let turn = 0; turn < 5; turn += 1) {
      await new Promise((settle) => setTimeout(settle, 0));
    }
    expect(document.activeElement).toBe(endpoint);
  });

  /** **対照: 確認の中の「やめる」で閉じたら、フォーカスは「保存する」へ戻る。** */
  it("確認の「やめる」で閉じたら、フォーカスは「保存する」へ戻る", async () => {
    await openEditor("named");
    await openNameApproval(MAIN);
    const dialog = keyBox(MAIN).querySelector('[role="alertdialog"]') as HTMLElement;
    await vi.waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));
    mustButton(dialog, "やめる").click();
    await vi.waitFor(() => expect(document.activeElement).toBe(mustButton(form(), "保存する")));
  });

  /**
   * **承認は、押したときの宛先に付く**（作業単位 A-13b-3 の再レビュー是正 —— 開いたときの宛先に結びつける変異を止める）。
   * 宛先を別のホスト名に変えてから「承認する」→ 保存（鍵の確認に「送る」）で `true` を送る。
   */
  it("宛先を別のホスト名に変えてから承認すれば、保存で `true` を送る", async () => {
    await openEditor("named");
    type(fieldOf(MAIN, ENDPOINT), "http://moved.local:46001");
    await openNameApproval(MAIN);
    mustButton(keyBox(MAIN).querySelector('[role="alertdialog"]') as HTMLElement, "承認する").click();
    flushSync();
    submit();
    mustButton(await waitQuestion(), "送る").click();
    const request = await waitSaved(1);
    expect(request.body["main"]).toMatchObject({
      endpoint: "http://moved.local:46001",
      allow_key_over_plain_http: true,
    });
  });

  /** **承認した後に宛先を変えれば、`true` を送らない**（答えは宛先に付く）。 */
  it("承認した後に宛先を変えれば、承認を送らない", async () => {
    await openEditor("named");
    await openNameApproval(MAIN);
    mustButton(keyBox(MAIN).querySelector('[role="alertdialog"]') as HTMLElement, "承認する").click();
    flushSync();
    type(fieldOf(MAIN, ENDPOINT), "http://moved.local:46001");
    expect(flat(keyBox(MAIN))).toContain(NAME_NOTE);
    submit();
    mustButton(await waitQuestion(), "送る").click();
    const request = await waitSaved(1);
    expect(Object.keys(request.body["main"] ?? {})).not.toContain("allow_key_over_plain_http");
  });
});

describe("平文送信の承認（ADR-0056 §決定5）", () => {
  /**
   * **確認は危険を言う**（鍵が暗号化されずに送られる・経路の途中で記録されれば残り、悪用されうる）。
   * 「承認して保存する」で `true` を送る。
   */
  it("外の `http://` へ鍵を書いた面の保存の前に、危険を示して承認を取る", async () => {
    await openEditor("relay");
    type(fieldOf(SUB, ENDPOINT), "http://203.0.113.9:47832");
    type(fieldOf(SUB, KEY_LABEL), NEW_KEY);
    expect(flat(keyBox(SUB))).toContain("保存の前に、鍵を暗号化せずに送ることの承認を求めます。");
    submit();
    const dialog = await waitQuestion();
    const text = questionText(dialog);
    expect(text.title).toBe("裏方（抽出・変容）の鍵を、暗号化せずに送ることを承認しますか");
    expect(text.body).toContain("宛先 http://203.0.113.9:47832 は http:// で、ローカルネットワークの外にあります。");
    expect(text.body).toContain(
      "承認すると、鍵は暗号化されずに送られます。経路の途中で記録されれば、その記録は残り、鍵が悪用されることがあります。",
    );
    expect(saved).toHaveLength(0);
    mustButton(dialog, "承認して保存する").click();
    const request = await waitSaved(1);
    expect(request.body["sub"]).toMatchObject({ api_key: NEW_KEY, allow_key_over_plain_http: true });
    expect(Object.keys(request.body["main"] ?? {})).not.toContain("allow_key_over_plain_http");
  });

  it("「承認せずに保存する」で `false` を送る", async () => {
    await openEditor("relay");
    type(fieldOf(SUB, ENDPOINT), "http://203.0.113.9:47832");
    type(fieldOf(SUB, KEY_LABEL), NEW_KEY);
    submit();
    mustButton(await waitQuestion(), "承認せずに保存する").click();
    const request = await waitSaved(1);
    expect(request.body["sub"]?.["allow_key_over_plain_http"]).toBe(false);
  });

  /** **鍵の確認の後に、承認を聞き直す**（承認は宛先に付く —— 宛先を変えた面の承認は持ち越さない）。 */
  it("承認のある面の宛先を変えると、鍵の確認の後に承認を聞き直す", async () => {
    await openEditor("far");
    expect(flat(keyBox(MAIN))).toContain("この宛先へ、鍵を暗号化せずに送ることを承認しています。");
    type(fieldOf(MAIN, ENDPOINT), "http://203.0.113.7:47833");
    submit();
    mustButton(await waitQuestion(), "送る").click();
    await vi.waitFor(() => {
      expect(questionText(question() as HTMLElement).title).toBe("本文生成の鍵を、暗号化せずに送ることを承認しますか");
    });
    expect(saved).toHaveLength(0);
    mustButton(question() as HTMLElement, "承認して保存する").click();
    const request = await waitSaved(1);
    expect(request.body["main"]).toMatchObject({ endpoint: "http://203.0.113.7:47833", allow_key_over_plain_http: true });
    expect(Object.keys(request.body["main"] ?? {})).not.toContain("api_key");
  });

  it("承認している面は「承認を外す」で `false` を送る", async () => {
    await openEditor("far");
    mustButton(keyBox(MAIN), "承認を外す").click();
    flushSync();
    expect(flat(keyBox(MAIN))).toContain("承認せずに保存します（ローカルネットワークの外へは鍵を送りません）。");
    submit();
    const request = await waitSaved(1);
    expect(question()).toBeNull();
    expect(request.body["main"]?.["allow_key_over_plain_http"]).toBe(false);
  });

  /** **ローカルネットワークの `http://` には警告を出す**（承認は求めない）。**その面の鍵の欄の隣にだけ。** */
  it("ローカルネットワークの宛先の面にだけ、暗号化されずに送られることを警告する", async () => {
    await openEditor("relay");
    const warning = "この宛先は http:// のローカルネットワークなので、鍵は暗号化されずに送られます（承認は要りません）。同じネットワークの機器は、その鍵を読めます。";
    expect(flat(keyBox(SUB))).toContain(warning);
    expect(flat(keyBox(MAIN))).not.toContain(warning);
  });
});

describe("裏方のチェックの一言（利用者の決定2）", () => {
  it("チェックを外すと、裏方に鍵があれば、チェックの隣で消えると言う", async () => {
    await openEditor("relay");
    const checkbox = form().querySelector("input[type=checkbox]") as HTMLInputElement;
    const row = (checkbox.parentElement as HTMLElement).parentElement as HTMLElement;
    expect(flat(row)).not.toContain("保存すると裏方の鍵も消えます");
    checkbox.click();
    flushSync();
    expect(flat(row)).toContain("保存すると裏方の鍵も消えます");
    checkbox.click();
    flushSync();
    expect(flat(row)).not.toContain("保存すると裏方の鍵も消えます");
  });
});
