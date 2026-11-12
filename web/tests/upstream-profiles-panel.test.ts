// @vitest-environment jsdom
/**
 * **書いたプロファイルを、画面で選ぶ・足す・直す**（作業単位 F-4b-3 / Issue origin#24 / `src/lib/UpstreamPanel.svelte`）。
 *
 * # 閉じる条件（Issue origin#24）をどこで見るか
 *
 * | 条件 | 節 |
 * |---|---|
 * | 一覧（名前と、面ごとの中身） | 「一覧」 |
 * | **書いた値と観測した値を混ぜない** | 「一覧」の2本目 |
 * | 選んで起こす（`POST /admin/upstream/profile`）・状態が出て変わっていく | 「切り替える」 |
 * | 足す・直す・保存後の表示が保存した内容と一致 | 「足す・直す」 |
 * | 422 を欄に結びつける・409 で黙って上書きしない | 「保存を断られたとき」 |
 * | 読むだけ・無い・読めない・検証に通らない | 「一覧の状態」 |
 * | 消す操作が無い | 「消す操作」 |
 *
 * # ⚠ 見るのは `<td>` のセルであって、`data-*` ではない
 *
 * **行のセルは配列で `toEqual` する。** 属性は有無だけでなく値まで見る。
 *
 * # ⚠ 帰属
 *
 * **3件 × 2面の値を全部違える。選ぶ対象は真ん中（`beta`）に置く**（端を選ぶと、
 * 「最初の1件」「最後の1件」に向ける変異が観測できない）。
 *
 * # ⚠ 「A が B になる」を待つ検査
 *
 * **A（載せている）が画面に出たことを先に確かめ、そのうえで B（準備完了）を待つ。**
 * B を返す読み直しは、**A を確かめた後にしか返らない**（応答を手で解く）。
 * A を一度も出さない実装はここで赤になる。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import UpstreamPanel from "../src/lib/UpstreamPanel.svelte";
import type {
  LaunchView,
  ProfilesView,
  UpstreamReport,
  UpstreamView,
  WrittenFace,
  WrittenFaceRequest,
  WrittenProfile,
} from "../src/lib/api";
import { launchStateRows } from "./support/upstream-cards";
import { enterAsReader, readerInZone } from "./support/zone";

/**
 * **一覧の面を、保存の依頼の面の形にする**（作業単位 A-13b-1）。
 *
 * 応答の `api_key_set` / `api_key_hint` は依頼に無く、画面は触らない鍵と承認を送らない（＝残す・今の規則 / 作業単位 A-13b-3）。
 *
 * **`sampling` / `response_tokens` は書いた値（か `null`）のまま必ず送る**（作業単位 D-21）。**裏方の面には `response_tokens` を載せない。**
 */
function requested(face: WrittenFace, side: "main" | "sub" = "main"): WrittenFaceRequest {
  const {
    api_key_set: _set,
    api_key_hint: _hint,
    allow_key_over_plain_http: _approved,
    response_tokens,
    ...rest
  } = face;
  return side === "main" ? { ...rest, response_tokens } : rest;
}

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

// ----------------------------------------------------------------------
// フィクスチャ
// ----------------------------------------------------------------------

const PATH = "C:\\Users\\someone\\AppData\\Local\\personacore\\upstreams.json";

/**
 * **3件。すべての値が互いに違う。** 選ぶのは真ん中の `beta`。
 *
 * **`context_tokens` は `alpha` の本文と `beta` の裏方にだけ書く**（作業単位 A-13a-2）。
 * どちらも既定（8192）・落とし所（16384）・観測の節の窓（32768 / 8192）と違う。
 */
const ALPHA: WrittenProfile = {
  name: "alpha",
  main: {
    endpoint: "http://127.0.0.1:41001",
    model: "alpha-main",
    gguf: "~/models/alpha-main.gguf",
    args: ["--ctx-size", "4096"],
    command: "C:/tools/llama-a.exe",
    context_tokens: 28672,
    api_key_set: false,
    api_key_hint: null,
    allow_key_over_plain_http: false,
    sampling: null,
    response_tokens: null,
  },
  sub: {
    endpoint: "http://127.0.0.1:41002",
    model: "alpha-sub",
    gguf: "~/models/alpha-sub.gguf",
    args: ["-ngl", "10"],
    command: "C:/tools/llama-b.exe",
    context_tokens: null,
    api_key_set: false,
    api_key_hint: null,
    allow_key_over_plain_http: false,
    sampling: null,
    response_tokens: null,
  },
};

/**
 * **`sampling` / `response_tokens` は真ん中の `beta` にだけ書く**（作業単位 D-21 / ADR-0060 §決定6）。
 * 既定（1.1・1024・`"none"`）と違え、本文と裏方でも違える。
 */
const BETA: WrittenProfile = {
  name: "beta",
  main: {
    endpoint: "http://127.0.0.1:41003",
    model: "beta-main",
    gguf: "D:\\models\\beta-main.gguf",
    args: ["--threads", "6"],
    command: "D:/bin/llama-beta.exe",
    context_tokens: null,
    api_key_set: false,
    api_key_hint: null,
    allow_key_over_plain_http: false,
    sampling: { repeat_penalty: 1.05, reasoning_effort: "low" },
    response_tokens: 2000,
  },
  sub: {
    endpoint: "http://127.0.0.1:41004",
    model: "beta-sub",
    gguf: "D:\\models\\beta-sub.gguf",
    args: ["--batch-size", "256"],
    command: "D:/bin/llama-beta-sub.exe",
    context_tokens: 10240,
    api_key_set: false,
    api_key_hint: null,
    allow_key_over_plain_http: false,
    sampling: { repeat_penalty: 1.25 },
    response_tokens: null,
  },
};

const GAMMA: WrittenProfile = {
  name: "gamma",
  main: {
    endpoint: "http://127.0.0.1:41005",
    model: "gamma-main",
    gguf: null,
    args: null,
    command: null,
    context_tokens: null,
    api_key_set: false,
    api_key_hint: null,
    allow_key_over_plain_http: false,
    sampling: null,
    response_tokens: null,
  },
  sub: null,
};

function listing(overrides: Partial<ProfilesView> = {}): ProfilesView {
  return {
    path: PATH,
    editable: true,
    state: "ok",
    revision: "rev-1",
    problem: null,
    profiles: [ALPHA, BETA, GAMMA],
    ...overrides,
  };
}

const EXTERNAL: LaunchView = {
  state: "external",
  since: null,
  answered_name: null,
  reason: null,
  log: null,
};

/** 観測の1面。**書いたプロファイルのどの値とも違う綴り**にしてある。 */
function view(face: "main" | "sub", launch: LaunchView): UpstreamView {
  return {
    face,
    endpoint: face === "main" ? "http://127.0.0.1:1234" : "http://127.0.0.1:5678",
    model: face === "main" ? "observed-main-model" : "observed-sub-model",
    answered_name: face === "main" ? "observed-main.gguf" : "observed-sub.gguf",
    queried_context_tokens: face === "main" ? 32768 : 8192,
    answered_by: "openai-exact-id",
    unauthorized: false,
    checked_at: "2026-11-03T09:58:00+09:00",
    check_result: "answered",
    measured: { checked: 1, matched: 1, drifted_other: 0, drifted_absent: 0, unanswered: 0 },
    launch,
  };
}

function report(main: LaunchView, sub: LaunchView): UpstreamReport {
  return { main: view("main", main), sub: view("sub", sub), profile_sample: null, selected_profile: null };
}

const IDLE = report(EXTERNAL, EXTERNAL);

const LOADING_SINCE = "2026-09-25T10:00:00+09:00";

/** `beta` を選んだ直後（本文も裏方も gguf を書いている → 両面を起こす）。 */
const BETA_LOADING = report(
  { ...EXTERNAL, state: "loading", since: LOADING_SINCE },
  { ...EXTERNAL, state: "loading", since: "2026-09-25T10:00:01+09:00" },
);

/** 本文は載った。裏方は落ちた。**2面の結末を違える**（帰属を見る）。 */
const BETA_SETTLED = report(
  { ...EXTERNAL, state: "ready", answered_name: "beta-main.gguf" },
  {
    ...EXTERNAL,
    state: "failed",
    reason: "上流が自分で終了しました",
    log: "sub-20260925-100001.err.log",
  },
);

// ----------------------------------------------------------------------
// 偽のサーバ
// ----------------------------------------------------------------------

interface Sent {
  method: string;
  url: string;
  body: unknown;
}

type Reply = { status: number; body: unknown } | Promise<{ status: number; body: unknown }>;

let target: HTMLElement;
let sent: Sent[];
/** `GET /admin/upstream` の応答（呼ばれるたびに先頭を取る。空なら `upstreamDefault`）。 */
let upstreamReplies: Reply[];
/** 列が空のときの `GET /admin/upstream` の応答。 */
let upstreamDefault: UpstreamReport;
/** `GET /admin/upstream/profiles` の応答（同上。空なら `listing()`）。 */
let listingReplies: Reply[];
/** `POST /admin/upstream/profile`。 */
let onSwitch: (body: unknown) => Reply;
/** `PUT /admin/upstream/profiles/{name}`。 */
let onSave: (name: string, body: unknown) => Reply;

function ok(body: unknown): Reply {
  return { status: 200, body };
}

function apiError(status: number, code: string, message: string, extra: Record<string, string | null> = {}): Reply {
  return { status, body: { error: { message, type: "invalid_request_error", code, ...extra } } };
}

/** **手で解く応答**（「A を確かめてから B を返す」ため）。 */
function deferred(): {
  reply: Promise<{ status: number; body: unknown }>;
  resolve: (body: unknown) => void;
  fail: (status: number, body: unknown) => void;
} {
  let settle: (status: number, body: unknown) => void = () => undefined;
  const reply = new Promise<{ status: number; body: unknown }>((done) => {
    settle = (status, body) => done({ status, body });
  });
  return { reply, resolve: (body) => settle(200, body), fail: (status, body) => settle(status, body) };
}

function stubFetch(): void {
  vi.stubGlobal("fetch", async (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const body = typeof init?.body === "string" ? (JSON.parse(init.body) as unknown) : undefined;
    sent.push({ method, url, body });

    let reply: Reply;
    if (method === "GET" && url === "/admin/upstream") {
      reply = upstreamReplies.shift() ?? ok(upstreamDefault);
    } else if (method === "GET" && url === "/admin/upstream/profiles") {
      reply = listingReplies.shift() ?? ok(listing());
    } else if (method === "POST" && url === "/admin/upstream/profile") {
      reply = onSwitch(body);
    } else if (method === "PUT" && url.startsWith("/admin/upstream/profiles/")) {
      reply = onSave(decodeURIComponent(url.slice("/admin/upstream/profiles/".length)), body);
    } else {
      reply = { status: 404, body: "not found" };
    }
    const settled = await reply;
    return {
      ok: settled.status >= 200 && settled.status < 300,
      status: settled.status,
      text: () =>
        Promise.resolve(typeof settled.body === "string" ? settled.body : JSON.stringify(settled.body)),
    } as unknown as Response;
  });
}

// ----------------------------------------------------------------------
// 画面を読む道具
// ----------------------------------------------------------------------

function flat(element: Element | null | undefined): string {
  return (element?.textContent ?? "").replace(/\s+/gu, " ").trim();
}

/** 見出し（`h2`）の文面で節を引く。**無ければ落とす。** */
function sectionNamed(name: string): HTMLElement {
  const found = [...target.querySelectorAll("section")].filter(
    (section) => flat(section.querySelector(":scope > h2")) === name,
  );
  expect(found, `節「${name}」が1つだけあること`).toHaveLength(1);
  return found[0] as HTMLElement;
}

const NOW = "いまの上流";
const WRITTEN = "upstreams.json に書いたプロファイル";

/**
 * 起こした上流の状態を、着手前の表と同じ形（`[見出しの行, 状態の行]`）で読む。
 *
 * **⚠ 作業単位 D-12b-2 で読む先を変えた。** 状態は表（見出し「この実行ファイルが上流をどうしているか」）から、
 * **面ごとのカードの「起こした上流の状態」の行**へ移った。読み方は `support/upstream-cards.ts`。
 * **アサートの側（どの面にどの文面が出るか）は変えていない。**
 */
function launchCells(): string[][] {
  return launchStateRows(sectionNamed(NOW));
}

/** プロファイル1件の塊（`h3` の名前で引く）。 */
function profileBlock(name: string): HTMLElement {
  const found = [...sectionNamed(WRITTEN).querySelectorAll("section")].filter(
    (section) => flat(section.querySelector(":scope > h3")) === name,
  );
  expect(found, `プロファイル「${name}」の塊が1つだけあること`).toHaveLength(1);
  return found[0] as HTMLElement;
}

/** 一覧に並ぶプロファイルの名前（**並び順ごと**）。 */
function listedNames(): string[] {
  return [...sectionNamed(WRITTEN).querySelectorAll("section.profile > h3")].map((h) => flat(h));
}

function buttonNamed(scope: Element, name: string): HTMLButtonElement | undefined {
  return [...scope.querySelectorAll("button")].find((button) => flat(button) === name);
}

function mustButton(scope: Element, name: string): HTMLButtonElement {
  const button = buttonNamed(scope, name);
  expect(button, `ボタン「${name}」があること`).toBeDefined();
  return button as HTMLButtonElement;
}

/** 開いている下書きの欄（**`<label for>` で引く**——見出しと欄の結びつきごと見る）。 */
function fieldOf(legend: string, label: string): HTMLInputElement | HTMLTextAreaElement {
  const form = target.querySelector("form");
  expect(form, "下書きが開いていること").not.toBeNull();
  const fieldset = [...(form as HTMLFormElement).querySelectorAll("fieldset")].find(
    (found) => flat(found.querySelector("legend")) === legend,
  );
  expect(fieldset, `面「${legend}」の欄の組があること`).toBeDefined();
  const labelled = [...(fieldset as HTMLFieldSetElement).querySelectorAll("label")].find(
    (found) => flat(found) === label,
  );
  expect(labelled, `欄「${label}」があること`).toBeDefined();
  const control = document.getElementById((labelled as HTMLLabelElement).htmlFor);
  expect(control, `欄「${label}」が見出しと結びついていること`).not.toBeNull();
  return control as HTMLInputElement | HTMLTextAreaElement;
}

/** 名前の欄。 */
function nameField(): HTMLInputElement {
  const labelled = [...target.querySelectorAll("form label")].find((found) => flat(found) === "名前");
  expect(labelled, "名前の欄があること").toBeDefined();
  return document.getElementById((labelled as HTMLLabelElement).htmlFor) as HTMLInputElement;
}

const MAIN_LEGEND = "本文生成";
const SUB_LEGEND = "裏方（抽出・変容）";
/** **起こす面の endpoint の見出し**（作業単位 D-19。繋ぐ面は [`ENDPOINT_CONNECT`]）。 */
const ENDPOINT_LAUNCH = "待ち受ける場所（endpoint）";
const ENDPOINT_CONNECT = "宛先（endpoint）";
/**
 * **起こす面の6つの欄**（作業単位 D-19 で endpoint の見出しが「待ち受ける場所」になった）。
 * フィクスチャの `alpha` / `beta` は両面とも gguf を書いてあるので、直すと両面とも起こす面で開く。
 */
const KEY_LABELS = [
  ENDPOINT_LAUNCH,
  "依頼するモデルの綴り（model）",
  "gguf のパス（gguf）",
  "起動の引数（args）",
  "実行ファイル（command）",
  "コンテキスト長（context_tokens）",
] as const;

const WINDOW_LABEL = "コンテキスト長（context_tokens）";

/**
 * **プロファイル1件の、面ごとの要点の行**（作業単位 D-19）。`[見出し, 要点]` を行ごとに読む。
 * **`<dt>` / `<dd>` の文面そのもの**（`data-*` ではない）。
 */
function summaryOf(name: string): string[][] {
  return [...profileBlock(name).querySelectorAll("dl > div")].map((row) => [
    flat(row.querySelector("dt")),
    // **要点の1行だけ**（作業単位 D-21 で同じ `<dd>` に足した送る値の行は [`sentOf`] が読む）。
    flat(row.querySelector("dd .summary")),
  ]);
}

/**
 * **プロファイル1件の、面ごとの送る値**（作業単位 D-21 / ADR-0060 §決定3）。行ごとに `[見出し, [[文面, 既定の印 | null], …]]`。
 * **既定の印は、書いた値の文面とは別の要素**（`.sent-default`）—— 書いた値と区別できる形で出ていることを見る。
 */
function sentOf(name: string): [string, [string, string | null][]][] {
  return [...profileBlock(name).querySelectorAll("dl > div")].map((row) => [
    flat(row.querySelector("dt")),
    [...row.querySelectorAll("dd .sent-value")].map((value) => {
      const fallback = value.querySelector(".sent-default");
      return [flat(value.querySelector(".sent-text")), fallback === null ? null : flat(fallback)];
    }),
  ]);
}

/** 開いている下書きの、その面の欄の組。 */
function fieldsetOf(legend: string): HTMLFieldSetElement {
  const found = [...(target.querySelector("form") as HTMLFormElement).querySelectorAll("fieldset")].find(
    (fieldset) => flat(fieldset.querySelector("legend")) === legend,
  );
  expect(found, `面「${legend}」の欄の組があること`).toBeDefined();
  return found as HTMLFieldSetElement;
}

/** その面の欄の見出し（**並び順ごと**。種類を選ぶボタンは数えない）。 */
function labelsOf(legend: string): string[] {
  return [...fieldsetOf(legend).querySelectorAll("label")].map((label) => flat(label));
}

/** その面の、種類を選ぶボタン（`[起こす, 繋ぐ]`）。**名前の付いた組で引く。** */
function kindButtons(legend: string): [HTMLButtonElement, HTMLButtonElement] {
  const group = fieldsetOf(legend).querySelector(`[role=group][aria-label="${legend}の上流の種類"]`);
  expect(group, `面「${legend}」の種類を選ぶ組があること`).not.toBeNull();
  return [
    mustButton(group as Element, "この機で llama を起こす"),
    mustButton(group as Element, "動いているサーバに繋ぐ"),
  ];
}

function pressedOf(legend: string): (string | null)[] {
  return kindButtons(legend).map((button) => button.getAttribute("aria-pressed"));
}

/** **その欄と同じ囲みの中の注記**（`p.note`）。注記が欄の隣にあることを見る。 */
function notesNear(control: Element): string[] {
  return [...(control.parentElement as HTMLElement).querySelectorAll(":scope > p.note")].map((note) => flat(note));
}

/** 1つの面の欄の値を、**鍵の並びで**。 */
function faceValues(legend: string): string[] {
  return KEY_LABELS.map((label) => fieldOf(legend, label).value);
}

function type(control: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  control.value = value;
  control.dispatchEvent(new Event("input", { bubbles: true }));
}

/** その欄の説明として結びついた文面（`aria-describedby` が指す要素）。 */
function describedText(control: Element): string {
  const ids = (control.getAttribute("aria-describedby") ?? "").split(/\s+/u).filter((id) => id !== "");
  return ids.map((id) => flat(document.getElementById(id))).join(" ");
}

const PROFILE_HEAD = ["項目", "本文生成", "裏方（抽出・変容）"];

async function open(): Promise<Record<string, unknown>> {
  const opened = mount(UpstreamPanel, { target, props: { pollIntervalMs: 5 } });
  await vi.waitFor(() => {
    expect(target.querySelector("h2")).not.toBeNull();
    expect(flat(target)).not.toContain("読み込み中");
  });
  return opened as Record<string, unknown>;
}

/**
 * **表を読んだ時刻を固定する**（Issue origin#40 / 状態の文面の経過の「いま」）。
 * `Date` だけを差し替える（`setTimeout` 等は本物のまま）。読み手はニューヨーク（冒頭の `readerInZone`）。
 */
const READ_AT = new Date("2026-09-25T01:00:12Z");

beforeEach(() => {
  vi.setSystemTime(READ_AT);
  sent = [];
  upstreamReplies = [];
  upstreamDefault = IDLE;
  listingReplies = [];
  onSwitch = () => ok(IDLE);
  onSave = () => ok(listing());
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
  target.remove();
});

// ----------------------------------------------------------------------

describe("一覧", () => {
  it("観測と書いたプロファイルを1回ずつ読む", async () => {
    const view = await open();
    expect(sent.map((s) => `${s.method} ${s.url}`).sort()).toEqual([
      "GET /admin/upstream",
      "GET /admin/upstream/profiles",
    ]);
    await unmount(view, { outro: false });
  });

  /**
   * **1件ずつ、面ごとの要点の行を丸ごと突き合わせる**（作業単位 D-19 で表から要点の1行へ変えた）。
   * プロファイルを取り違える変異・面を取り違える変異・種類を取り違える変異のどれも赤になる。
   * **書いていない欄を並べる表は、もう無い。**
   */
  it("名前の順に並び、1件ずつ面ごとに種類の印と要点が1行で出る", async () => {
    const view = await open();
    expect(listedNames()).toEqual(["alpha", "beta", "gamma"]);

    expect(summaryOf("alpha")).toEqual([
      ["本文", "起こす alpha-main.gguf → :41001 · 窓 28,672"],
      ["裏方", "起こす alpha-sub.gguf → :41002"],
    ]);
    expect(summaryOf("beta")).toEqual([
      ["本文", "起こす beta-main.gguf → :41003"],
      ["裏方", "起こす beta-sub.gguf → :41004 · 窓 10,240"],
    ]);
    expect(summaryOf("gamma")).toEqual([
      ["本文", "繋ぐ http://127.0.0.1:41005"],
      ["裏方", "本文と同じ"],
    ]);
    // **書いていない欄を「（書いていません）」で並べない**（表をやめた）。
    const written = sectionNamed(WRITTEN);
    expect(written.querySelectorAll("table")).toHaveLength(0);
    expect(flat(written)).not.toContain("書いていません");
    await unmount(view, { outro: false });
  });

  /**
   * **書いた値と観測した値を混ぜない**（Issue origin#24 の閉じる条件）。
   *
   * **2つは別の節**（入れ子にしない）で、**書いた gguf のパスは書いた節にだけ**、
   * **上流が名乗った名前・窓は観測の節にだけ**出る。
   */
  it("書いた値と、上流に聞いた値が、別の節に分かれて出る", async () => {
    const view = await open();
    const now = sectionNamed(NOW);
    const written = sectionNamed(WRITTEN);
    expect(now.contains(written)).toBe(false);
    expect(written.contains(now)).toBe(false);

    // **一覧は gguf のファイル名だけを出す**（作業単位 D-19。書いたままのパスは「直す」の欄に出る）。
    const writtenGgufs = ["alpha-main.gguf", "alpha-sub.gguf", "beta-main.gguf", "beta-sub.gguf"];
    for (const gguf of writtenGgufs) {
      expect(flat(written), gguf).toContain(gguf);
      expect(flat(now), gguf).not.toContain(gguf);
    }
    for (const observed of ["observed-main.gguf", "observed-sub.gguf", "32768 トークン", "observed-main-model"]) {
      expect(flat(now), observed).toContain(observed);
      expect(flat(written), observed).not.toContain(observed);
    }
    // **観測の窓（本文 32768 / 裏方 8192）は、一覧の「窓 N」の書き方（3桁区切り）でも生の数でも、書いた節に出ない**
    // （D-19 のレビュー —— 一覧の窓を観測の値から作る実装を、単位の付かない形でも止める）。
    for (const observed of ["32768", "32,768", "8192", "8,192"]) {
      expect(flat(written), observed).not.toContain(observed);
    }
    // 対照: 書いた窓（alpha の本文 28672）は、一覧の書き方で書いた節に出ている。
    expect(flat(written)).toContain("窓 28,672");
    // **節は、書いたままだと自分で言う。**
    expect(flat(written)).toContain("ファイルに書いたままの値です");
    await unmount(view, { outro: false });
  });

  it("書く先のファイルを言う", async () => {
    const view = await open();
    expect(flat(sectionNamed(WRITTEN))).toContain(`書く先: ${PATH}`);
    await unmount(view, { outro: false });
  });
});

/**
 * **いま選んでいるプロファイル**（作業単位 D-19）。
 *
 * 印の元は `GET /admin/upstream` の `selected_profile`（`UpstreamPanel.svelte` が持つ観測から props で渡す）。
 * **選ぶ対象は真ん中（`beta`）。** 端を選ぶと、「最初の1件」「最後の1件」に向ける変異が観測できない。
 */
describe("いま選んでいるプロファイル", () => {
  const SWITCH = "このプロファイルに切り替える";
  const RESWITCH = "切り替え直す";
  const NAMES = ["alpha", "beta", "gamma"];

  /** ボタンの見た目の種類。主ボタン（`.primary`）なら `"primary"`、普通のボタンなら `"plain"`、無ければ `null`。 */
  function look(button: HTMLButtonElement | undefined): string | null {
    return button === undefined ? null : button.classList.contains("primary") ? "primary" : "plain";
  }

  /** 3件それぞれの `[aria-current, 「選んでいます」があるか, 「このプロファイルに切り替える」, 「切り替え直す」]`。 */
  function marks(): unknown[][] {
    return NAMES.map((name) => {
      const block = profileBlock(name);
      return [
        block.getAttribute("aria-current"),
        flat(block).includes("選んでいます"),
        look(buttonNamed(block, SWITCH)),
        look(buttonNamed(block, RESWITCH)),
      ];
    });
  }

  /**
   * **選んでいる枠には「切り替える」の代わりに「切り替え直す」（普通のボタン）**（D-19 の是正）。
   * 保存した内容を反映する・`failed` / `stopped` からやり直す手段を、選んでいる枠から消さない。
   */
  it("選んでいるプロファイルには印と「切り替え直す」を出し、ほかは主ボタンの「切り替える」を出す", async () => {
    upstreamDefault = { ...IDLE, selected_profile: "beta" };
    const view = await open();
    await vi.waitFor(() => {
      expect(marks()[1]?.[0]).toBe("true");
    });
    expect(marks()).toEqual([
      [null, false, "primary", null],
      ["true", true, null, "plain"],
      [null, false, "primary", null],
    ]);
    // **直すは、選んでいるプロファイルにも出る**（直すことと選ぶことは別）。
    expect(NAMES.map((name) => buttonNamed(profileBlock(name), "直す") !== undefined)).toEqual([true, true, true]);
    await unmount(view, { outro: false });
  });

  /** **「切り替え直す」は、その枠の名前で同名の差し替えを頼む**（「切り替える」と同じ経路）。 */
  it("「切り替え直す」を押すと、選んでいるプロファイルの名前だけで切り替えを頼む", async () => {
    upstreamDefault = { ...IDLE, selected_profile: "beta" };
    onSwitch = () => ok({ ...BETA_LOADING, selected_profile: "beta" });
    const view = await open();
    await vi.waitFor(() => {
      expect(marks()[1]?.[0]).toBe("true");
    });
    // 切り替えた後の読み直しも、切り替えの応答と同じものを返す（載せている間は読み直し続けるため）。
    upstreamDefault = { ...BETA_LOADING, selected_profile: "beta" };
    mustButton(profileBlock("beta"), RESWITCH).click();
    await vi.waitFor(() => {
      expect(sent.some((s) => s.method === "POST")).toBe(true);
    });
    expect(sent.filter((s) => s.method === "POST")).toEqual([
      { method: "POST", url: "/admin/upstream/profile", body: { name: "beta" } },
    ]);
    // **応答を観測の節が採る**（同じ差し替えなので、起こした状態が出る）。
    await vi.waitFor(() => {
      expect(launchCells()[1]?.[1]).toBe("載せています（21:00:00 から、12 秒経過）");
    });
    await unmount(view, { outro: false });
  });

  /** **状態によらず出す**（`failed` / `stopped` / `not_started` のどれでも、やり直せる）。 */
  it.each(["failed", "stopped", "not_started", "ready"] as const)(
    "本文の面が %s でも「切り替え直す」を出す",
    async (state) => {
      upstreamDefault = { ...report({ ...EXTERNAL, state }, EXTERNAL), selected_profile: "beta" };
      const view = await open();
      await vi.waitFor(() => {
        expect(marks()[1]?.[0]).toBe("true");
      });
      expect(look(buttonNamed(profileBlock("beta"), RESWITCH))).toBe("plain");
      await unmount(view, { outro: false });
    },
  );

  /** その枠の「直す」から開き、何も変えずに保存する（下書きが閉じるまで待つ）。 */
  async function saveFrom(name: string): Promise<void> {
    mustButton(profileBlock(name), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
  }

  /** ボタンの名前（`aria-label` があればそれ、無ければ見える文面）。 */
  function accessibleName(button: Element): string {
    return button.getAttribute("aria-label") ?? flat(button);
  }

  /**
   * **節の中の知らせ（`p.notice`）を、`[ボタンを除いた文面, 中のボタンの名前]` で読む**（作業単位 B-69）。
   * 文面とボタンを分けて読む —— ボタンの名前が文面に混ざると、「ボタンが在る」と「そう言っている」を取り違える。
   * **ボタンは名前（`aria-label`）で読む** —— 同じ節に「切り替え直す」が2つ並ぶので、知らせの側はプロファイル名を名乗る（レビュー是正5）。
   */
  function notices(): [string, string[]][] {
    return [...sectionNamed(WRITTEN).querySelectorAll("p.notice")].map((p) => {
      const copy = p.cloneNode(true) as HTMLElement;
      for (const button of copy.querySelectorAll("button")) {
        button.remove();
      }
      return [flat(copy), [...p.querySelectorAll("button")].map((button) => accessibleName(button))];
    });
  }

  /** 保存の知らせの中の「「beta」を切り替え直す」の名前。 */
  const NOTICE_RESWITCH = "「beta」を切り替え直す";

  /** 保存の知らせの中の「切り替え直す」（**名前で引く**。無ければ落とす）。 */
  function noticeReswitch(): HTMLButtonElement {
    const found = [...sectionNamed(WRITTEN).querySelectorAll("p.notice button")].filter(
      (button) => accessibleName(button) === NOTICE_RESWITCH,
    );
    expect(found, `知らせの中に「${NOTICE_RESWITCH}」が1つだけあること`).toHaveLength(1);
    return found[0] as HTMLButtonElement;
  }

  /**
   * **選んでいるプロファイルを保存したら、知らせの中に「切り替え直す」を置く**（作業単位 B-69 / Issue origin#98 ——
   * 「切り替え直すまで変わりません」を読んでも、次に何を押すかに辿り着きにくかった）。
   * **ほかの枠を保存したときは今までの文面で、ボタンは置かない**（別のプロファイルへ切り替えるのは大きい判断なので、一覧の行に任せる）。
   * 選んでいるのは真ん中（`beta`）。対照は両端（`alpha` と `gamma`）の両方。
   */
  it("保存の知らせは、選んでいるプロファイルなら切り替え直すように言ってボタンを置き、ほかなら今の文面でボタンを置かない", async () => {
    upstreamDefault = { ...IDLE, selected_profile: "beta" };
    onSave = () => ok(listing({ revision: "rev-2" }));
    const view = await open();
    await vi.waitFor(() => {
      expect(marks()[1]?.[0]).toBe("true");
    });
    await saveFrom("beta");
    expect(notices()).toEqual([
      ["「beta」を保存しました。いま動いている上流に反映するには、切り替え直してください。", [NOTICE_RESWITCH]],
    ]);
    // **見える文面は一覧の行と同じ「切り替え直す」**（名前だけがプロファイル名を名乗る）。
    expect(flat(noticeReswitch())).toBe(RESWITCH);
    await saveFrom("gamma");
    expect(notices()).toEqual([
      ["「gamma」を保存しました。いま動いている上流は、このプロファイルに切り替えるまで変わりません。", []],
    ]);
    await saveFrom("alpha");
    expect(notices()).toEqual([
      ["「alpha」を保存しました。いま動いている上流は、このプロファイルに切り替えるまで変わりません。", []],
    ]);
    await unmount(view, { outro: false });
  });

  /**
   * **知らせの「切り替え直す」は、一覧の行の「切り替え直す」と同じ動き**（作業単位 B-69 —— 2つ目の切り替えの経路を作らない）。
   * 同じ要求（選んでいる名前だけ）を1回送り、待つ間はどの切り替えのボタンも押せず、
   * 応答は観測の節が採り、**知らせは切り替えの結果に置き換わる**（「切り替え直してください」を残さない）。
   */
  it("知らせの「切り替え直す」を押すと、一覧の行と同じ要求で切り替え、知らせは切り替えの結果に置き換わる", async () => {
    upstreamDefault = { ...IDLE, selected_profile: "beta" };
    onSave = () => ok(listing({ revision: "rev-2" }));
    const pending = deferred();
    onSwitch = () => pending.reply;
    const view = await open();
    await vi.waitFor(() => {
      expect(marks()[1]?.[0]).toBe("true");
    });
    await saveFrom("beta");
    expect(sent.filter((s) => s.method === "POST")).toEqual([]);

    noticeReswitch().click();
    // **待つ間は、知らせのボタンも一覧の行のボタンも押せない**（一覧の行のボタンと同じ無効化）。
    await vi.waitFor(() => {
      expect(noticeReswitch().disabled).toBe(true);
    });
    expect([
      mustButton(profileBlock("alpha"), SWITCH).disabled,
      mustButton(profileBlock("beta"), RESWITCH).disabled,
      mustButton(profileBlock("gamma"), SWITCH).disabled,
    ]).toEqual([true, true, true]);
    expect(flat(sectionNamed(WRITTEN))).toContain("「beta」に切り替えています…");

    upstreamDefault = { ...BETA_LOADING, selected_profile: "beta" };
    pending.resolve({ ...BETA_LOADING, selected_profile: "beta" });
    await vi.waitFor(() => {
      expect(notices()).toEqual([["「beta」に切り替えました。起こした上流の状態は、上の表に出ます。", []]]);
    });
    expect(sent.filter((s) => s.method === "POST")).toEqual([
      { method: "POST", url: "/admin/upstream/profile", body: { name: "beta" } },
    ]);
    // **応答を観測の節が採る**（一覧の行の「切り替え直す」と同じ）。
    await vi.waitFor(() => {
      expect(launchCells()[1]?.[1]).toBe("載せています（21:00:00 から、12 秒経過）");
    });
    await unmount(view, { outro: false });
  });

  /**
   * **知らせの「切り替え直す」が断られたら、一覧の行のボタンと同じ場所・同じ文面で言う**（作業単位 B-69）。
   * 知らせは残し、もう一度押せる（反映はまだ済んでいない）。
   */
  it("知らせの「切り替え直す」が断られたら、一覧の行と同じく理由を出し、知らせは押せるまま残す", async () => {
    const message = "上流の差し替えが進行中です。終わってから選び直してください";
    upstreamDefault = { ...IDLE, selected_profile: "beta" };
    onSave = () => ok(listing({ revision: "rev-2" }));
    onSwitch = () => apiError(409, "upstream_switch_in_progress", message);
    const view = await open();
    await vi.waitFor(() => {
      expect(marks()[1]?.[0]).toBe("true");
    });
    await saveFrom("beta");
    noticeReswitch().click();
    await vi.waitFor(() => {
      expect(target.querySelector("[role=alert]")).not.toBeNull();
    });
    expect([...sectionNamed(WRITTEN).querySelectorAll("[role=alert]")].map((a) => flat(a))).toEqual([
      `切り替えられませんでした: ${message}`,
    ]);
    expect(notices()).toEqual([
      ["「beta」を保存しました。いま動いている上流に反映するには、切り替え直してください。", [NOTICE_RESWITCH]],
    ]);
    await vi.waitFor(() => {
      expect(noticeReswitch().disabled).toBe(false);
    });
    await unmount(view, { outro: false });
  });

  /**
   * **知らせのボタンは、保存したプロファイルがいまも選ばれているときだけ出す**（B-69 のレビュー是正1）。
   * 別のタブ等で `alpha` に切り替わる（読み直しで `selected` が `alpha` になる）とボタンを消す ——
   * 残すと、押したときに `alpha` を `beta` へ切り替えてしまう。**文面は「保存しました。」だけにする**
   * （「切り替え直してください」は、もう選んでいないプロファイルについては偽）。
   *
   * 読み直しは載せている面があるときだけ回るので、最初は `beta` を載せている状態から始める。
   */
  it("保存したプロファイルが選ばれなくなったら、知らせのボタンを消し、保存したことだけを言う", async () => {
    upstreamDefault = { ...BETA_LOADING, selected_profile: "beta" };
    onSave = () => ok(listing({ revision: "rev-2" }));
    const view = await open();
    await vi.waitFor(() => {
      expect(marks()[1]?.[0]).toBe("true");
    });
    await saveFrom("beta");
    expect(notices()).toEqual([
      ["「beta」を保存しました。いま動いている上流に反映するには、切り替え直してください。", [NOTICE_RESWITCH]],
    ]);
    // **外で alpha に切り替わった**（読み直しの応答が運ぶ）。
    upstreamDefault = { ...BETA_LOADING, selected_profile: "alpha" };
    await vi.waitFor(() => {
      expect(marks()[0]?.[0]).toBe("true");
    });
    expect(notices()).toEqual([["「beta」を保存しました。", []]]);
    expect(sent.filter((s) => s.method === "POST")).toEqual([]);
    await unmount(view, { outro: false });
  });

  /**
   * **一覧の行から別のプロファイルへ切り替えて通ったときも、保存の知らせを消す**（B-69 のレビュー是正2）。
   * 知らせが言う「いま動いている上流」が変わったので、切り替えの結果の知らせだけを残す。
   */
  it("一覧の行から別のプロファイルへ切り替えて通ったら、保存の知らせは切り替えの結果に置き換わる", async () => {
    upstreamDefault = { ...IDLE, selected_profile: "beta" };
    onSave = () => ok(listing({ revision: "rev-2" }));
    onSwitch = () => ok({ ...IDLE, selected_profile: "alpha" });
    const view = await open();
    await vi.waitFor(() => {
      expect(marks()[1]?.[0]).toBe("true");
    });
    await saveFrom("beta");
    expect(notices()).toEqual([
      ["「beta」を保存しました。いま動いている上流に反映するには、切り替え直してください。", [NOTICE_RESWITCH]],
    ]);
    mustButton(profileBlock("alpha"), SWITCH).click();
    await vi.waitFor(() => {
      expect(marks()[0]?.[0]).toBe("true");
    });
    expect(notices()).toEqual([["「alpha」に切り替えました。起こした上流の状態は、上の表に出ます。", []]]);
    expect(sent.filter((s) => s.method === "POST")).toEqual([
      { method: "POST", url: "/admin/upstream/profile", body: { name: "alpha" } },
    ]);
    await unmount(view, { outro: false });
  });

  /**
   * **切り替えを待つ間に保存した新しい知らせを、先に送った切り替えの応答が消さない**（B-69 のレビュー是正3）。
   * 消してよいのは、切り替えを送った時点で出ていた知らせだけ —— 待つ間の保存は、その切り替えに入っていないかもしれない。
   */
  it("切り替えを待つ間に保存した知らせは、先に送った切り替えの応答で消えない", async () => {
    upstreamDefault = { ...IDLE, selected_profile: "beta" };
    onSave = () => ok(listing({ revision: "rev-2" }));
    const pending = deferred();
    onSwitch = () => pending.reply;
    const view = await open();
    await vi.waitFor(() => {
      expect(marks()[1]?.[0]).toBe("true");
    });
    await saveFrom("beta");
    noticeReswitch().click();
    await vi.waitFor(() => {
      expect(noticeReswitch().disabled).toBe(true);
    });
    // **待つ間に、もう一度直して保存する**（新しい知らせ）。
    await saveFrom("beta");
    pending.resolve({ ...IDLE, selected_profile: "beta" });
    await vi.waitFor(() => {
      expect(flat(sectionNamed(WRITTEN))).toContain("「beta」に切り替えました。");
    });
    expect(notices()).toEqual([
      ["「beta」に切り替えました。起こした上流の状態は、上の表に出ます。", []],
      ["「beta」を保存しました。いま動いている上流に反映するには、切り替え直してください。", [NOTICE_RESWITCH]],
    ]);
    expect(noticeReswitch().disabled).toBe(false);
    await unmount(view, { outro: false });
  });

  /** **切り替えの応答（`adopt`）が運んだ名前へ、印が移る。** */
  it("切り替えると、印が切り替えた先へ移る", async () => {
    upstreamDefault = { ...IDLE, selected_profile: "alpha" };
    onSwitch = () => ok({ ...IDLE, selected_profile: "beta" });
    const view = await open();
    await vi.waitFor(() => {
      expect(marks()[0]?.[0]).toBe("true");
    });
    mustButton(profileBlock("beta"), SWITCH).click();
    await vi.waitFor(() => {
      expect(marks()[1]?.[0]).toBe("true");
    });
    expect(marks()).toEqual([
      [null, false, "primary", null],
      ["true", true, null, "plain"],
      [null, false, "primary", null],
    ]);
    await unmount(view, { outro: false });
  });

  /**
   * **どれを選んでいるか分からないときは、印を付けず、全部に切り替えるを出す**
   * （観測が読めない・選んでいない・一覧に無い名前）。**押せなくなる側に倒さない。**
   */
  it.each([
    ["観測が読めない", null, [{ status: 500, body: "boom" }]],
    ["プロファイルを選んでいない", null, []],
    ["一覧に無い名前を選んでいる", "delta", []],
  ] as const)("%s ときは、印を付けず全部に切り替えるを出す", async (_case, selected, replies) => {
    upstreamReplies = [...replies];
    upstreamDefault = { ...IDLE, selected_profile: selected };
    const view = await open();
    expect(marks()).toEqual([
      [null, false, "primary", null],
      [null, false, "primary", null],
      [null, false, "primary", null],
    ]);
    await unmount(view, { outro: false });
  });
});

describe("切り替える", () => {
  it("真ん中のプロファイルを選ぶと、その名前だけを送る", async () => {
    onSwitch = () => ok(BETA_LOADING);
    const view = await open();

    mustButton(profileBlock("beta"), "このプロファイルに切り替える").click();
    await vi.waitFor(() => {
      expect(sent.some((s) => s.method === "POST")).toBe(true);
    });
    expect(sent.filter((s) => s.method === "POST")).toEqual([
      { method: "POST", url: "/admin/upstream/profile", body: { name: "beta" } },
    ]);
    await unmount(view, { outro: false });
  });

  /**
   * **切り替えた後も、書いた節に観測の値が入らない**（Issue origin#24 の閉じる条件 / レビュー1周目 R09）。
   *
   * 切り替えの応答は観測（上流が名乗った名前・窓）を運んでくる。**書いた節が出す一言に
   * それを混ぜると、書いた値と観測した値が同じ節に並ぶ。**
   */
  it("切り替えた後も、書いた節に観測の値が入らない", async () => {
    onSwitch = () => ok(BETA_LOADING);
    const view = await open();

    mustButton(profileBlock("beta"), "このプロファイルに切り替える").click();
    await vi.waitFor(() => {
      expect(flat(sectionNamed(WRITTEN))).toContain("「beta」に切り替えました。");
    });
    const written = flat(sectionNamed(WRITTEN));
    for (const observed of [
      "observed-main.gguf",
      "observed-sub.gguf",
      "observed-main-model",
      "observed-sub-model",
      "32768",
      "8192",
      LOADING_SINCE,
    ]) {
      expect(written, observed).not.toContain(observed);
    }
    // 対照: 観測の値は、観測の節には在る。
    expect(flat(sectionNamed(NOW))).toContain("observed-main.gguf");
    await unmount(view, { outro: false });
  });

  /**
   * **既定の間隔で読み直す**（レビュー1周目 R01 / R02 —— 既定を通るテストが無かった）。
   *
   * 他の測定は間隔を 5 ミリ秒に縮めて回すので、**既定値そのもの（2000）と、
   * 間隔を `setTimeout` に渡していることは、ここでしか見ていない。**
   * 偽の時計は `setTimeout` / `clearTimeout` だけを差し替える（Svelte の予約は触らない）。
   */
  it("既定の間隔（2 秒）ごとに読み直し、それより早くは読み直さない", async () => {
    upstreamDefault = BETA_LOADING;
    const realSetTimeout = globalThis.setTimeout;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      // **対照: 偽の時計が、本物の `setTimeout` を置き換えている。**
      expect(globalThis.setTimeout).not.toBe(realSetTimeout);
      let fired = false;
      setTimeout(() => {
        fired = true;
      }, 1);
      await Promise.resolve();
      expect(fired, "進めるまで予約は走らない").toBe(false);
      await vi.advanceTimersByTimeAsync(1);
      expect(fired, "進めると予約が走る").toBe(true);

      // **既定の間隔で開く**（`pollIntervalMs` を渡さない）。
      const view = mount(UpstreamPanel, { target });
      const reads = (): number => sent.filter((s) => s.url === "/admin/upstream").length;
      await vi.advanceTimersByTimeAsync(0);
      expect(reads(), "開いた直後の1回").toBe(1);
      await vi.advanceTimersByTimeAsync(1999);
      expect(reads(), "2 秒経つ前は読み直さない").toBe(1);
      await vi.advanceTimersByTimeAsync(1);
      expect(reads(), "2 秒で1回").toBe(2);
      await vi.advanceTimersByTimeAsync(1999);
      expect(reads()).toBe(2);
      await vi.advanceTimersByTimeAsync(1);
      expect(reads(), "次の 2 秒でもう1回").toBe(3);
      await unmount(view, { outro: false });
    } finally {
      vi.useRealTimers();
    }
    expect(globalThis.setTimeout).toBe(realSetTimeout);
  });

  /**
   * **読み直しが失敗したら、読み直しをやめる**（失敗の表示と「再試行」に移る）。
   * 載せている最中でも、失敗を出したまま裏で読み直し続けない。
   */
  /**
   * **失敗した後の「再試行」を待つ間、失敗する前の表を出さない**（レビュー2周目 R10）。
   *
   * 失敗の枝で観測を捨てないと、「再試行」が失敗の表示を消した瞬間に、
   * **失敗する前の古い表（「載せています」）が、いまの状態のように出る。**
   * 1周目の報告はこの変異を「等価」と書いたが、この入力で見分けられる（レビュアーのプローブ）。
   */
  it("失敗の後の再試行を待つ間は、読み込み中だけを出し、古い表を出さない", async () => {
    const retried = deferred();
    upstreamReplies = [ok(BETA_LOADING), { status: 500, body: "boom" }, retried.reply];
    const view = await open();
    await vi.waitFor(() => {
      expect(flat(sectionNamed(NOW))).toContain("上流の様子を取得できませんでした");
    });

    mustButton(sectionNamed(NOW), "再試行").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.url === "/admin/upstream")).toHaveLength(3);
    });
    await new Promise((done) => setTimeout(done, 20));
    const now = sectionNamed(NOW);
    expect(now.querySelectorAll("table")).toHaveLength(0);
    expect(flat(now)).toContain("読み込み中");
    expect(flat(now)).not.toContain("載せています");

    // 対照: 応答が着けば、表が出る。
    retried.resolve(BETA_SETTLED);
    await vi.waitFor(() => {
      expect(launchCells()[1]?.[1]).toBe("準備完了（上流が名乗った名前: beta-main.gguf）");
    });
    await unmount(view, { outro: false });
  });

  it("読み直しが失敗したら、失敗を出して読み直しをやめる", async () => {
    upstreamReplies = [ok(BETA_LOADING), { status: 500, body: "boom" }];
    upstreamDefault = BETA_LOADING;
    const view = await open();
    await vi.waitFor(() => {
      expect(flat(sectionNamed(NOW))).toContain("上流の様子を取得できませんでした");
    });
    const reads = sent.filter((s) => s.url === "/admin/upstream").length;
    expect(reads).toBe(2);
    await new Promise((done) => setTimeout(done, 60));
    expect(sent.filter((s) => s.url === "/admin/upstream")).toHaveLength(2);
    expect(flat(sectionNamed(NOW))).toContain("上流の様子を取得できませんでした");
    expect(mustButton(sectionNamed(NOW), "再試行")).toBeDefined();
    await unmount(view, { outro: false });
  });

  /**
   * **状態が出て、変わっていく**（載せている → 準備完了 / 失敗）。
   *
   * **A（載せている）を確かめてから、B を返す読み直しを解く。**
   * 読み直しの応答は手で解くので、**A を一度も出さない実装はここで赤になる。**
   */
  it("載せている状態が出て、読み直すと準備完了と失敗に変わる", async () => {
    onSwitch = () => ok(BETA_LOADING);
    const poll = deferred();
    const view = await open();
    // **開いた時点では、起こしていない。**
    expect(launchCells()).toEqual([
      PROFILE_HEAD,
      [
        "起こした上流の状態",
        "起こしていません（外の上流に依頼しています）",
        "起こしていません（外の上流に依頼しています）",
      ],
    ]);
    upstreamReplies = [poll.reply];

    mustButton(profileBlock("beta"), "このプロファイルに切り替える").click();

    // **A: 載せている**（切り替えの応答そのもの）。
    await vi.waitFor(() => {
      expect(launchCells()[1]).toEqual([
        "起こした上流の状態",
        "載せています（21:00:00 から、12 秒経過）",
        "載せています（21:00:01 から、11 秒経過）",
      ]);
    });
    // **読み直しが、A を出している間に飛んでいる**（B はまだ返していない）。
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.url === "/admin/upstream")).toHaveLength(2);
    });
    expect(launchCells()[1]?.[1]).toBe("載せています（21:00:00 から、12 秒経過）");

    // **B: 読み直しの応答を解く。**
    poll.resolve(BETA_SETTLED);
    await vi.waitFor(() => {
      expect(launchCells()[1]).toEqual([
        "起こした上流の状態",
        "準備完了（上流が名乗った名前: beta-main.gguf）",
        "失敗しました: 上流が自分で終了しました（ログ: upstream-logs の sub-20260925-100001.err.log）",
      ]);
    });

    // **載せている面が無くなったら、読み直しをやめる。**
    const polls = sent.filter((s) => s.url === "/admin/upstream").length;
    await new Promise((done) => setTimeout(done, 60));
    expect(sent.filter((s) => s.url === "/admin/upstream")).toHaveLength(polls);

    await unmount(view, { outro: false });
  });

  /**
   * **読み手の時間帯が、状態の行の時刻に届く**（D-18 のレビュー / `tests/timestamp-wiring.test.ts` の形）。
   * 日をまたぐ瞬間を東京とニューヨークで読む。終わったらファイルの読み手（ニューヨーク）へ戻す。
   */
  for (const [zone, expected] of [
    ["Asia/Tokyo", "載せています（05:00:00 から、12 秒経過）"],
    ["America/New_York", "載せています（16:00:00 から、12 秒経過）"],
  ] as const) {
    it(`状態の行の時刻は、${zone} の読み手の時間帯で出る`, async () => {
      enterAsReader(zone);
      try {
        vi.setSystemTime(new Date("2026-09-26T20:00:12Z"));
        upstreamDefault = report({ ...EXTERNAL, state: "loading", since: "2026-09-26T20:00:00Z" }, EXTERNAL);
        const view = await open();
        await vi.waitFor(() => {
          expect(launchCells()[1]?.[1]).toBe(expected);
        });
        await unmount(view, { outro: false });
      } finally {
        enterAsReader("America/New_York");
      }
    });
  }

  /**
   * **切り替えの応答を採った時刻も「いま」になる**（D-18 のレビュー / `adopt`）。
   * 読み直しの応答は解かずに置くので、出ている文面は切り替えの応答そのもの。
   */
  it("切り替えの直後の文面は、切り替えた時刻からの経過を言う", async () => {
    onSwitch = () => ok(BETA_LOADING);
    const poll = deferred();
    const view = await open();
    upstreamReplies = [poll.reply];
    vi.setSystemTime(new Date(READ_AT.getTime() + 30_000));
    mustButton(profileBlock("beta"), "このプロファイルに切り替える").click();
    await vi.waitFor(() => {
      expect(launchCells()[1]).toEqual([
        "起こした上流の状態",
        "載せています（21:00:00 から、42 秒経過）",
        "載せています（21:00:01 から、41 秒経過）",
      ]);
    });
    await unmount(view, { outro: false });
  });

  /**
   * **経過は、読み直すたびに進む**（Issue origin#40）—— 読んだ時刻を「いま」として渡している。
   * 時刻の側は動かない（同じ `since`）。
   */
  it("載せている間、読み直すたびに経過が進む", async () => {
    upstreamDefault = BETA_LOADING;
    const view = await open();
    await vi.waitFor(() => {
      expect(launchCells()[1]?.[1]).toBe("載せています（21:00:00 から、12 秒経過）");
    });
    vi.setSystemTime(new Date(READ_AT.getTime() + 60_000));
    await vi.waitFor(() => {
      expect(launchCells()[1]).toEqual([
        "起こした上流の状態",
        "載せています（21:00:00 から、1 分 12 秒経過）",
        "載せています（21:00:01 から、1 分 11 秒経過）",
      ]);
    });
    await unmount(view, { outro: false });
  });

  /** **画面を離れたら、読み直しをやめる**（もう無い画面を書き換えない）。 */
  it("画面を閉じたら、載せている最中でも読み直さない", async () => {
    // **何度読み直しても載せている**（自然には止まらない形にする）。
    upstreamDefault = BETA_LOADING;
    const view = await open();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.url === "/admin/upstream").length).toBeGreaterThanOrEqual(3);
    });
    await unmount(view, { outro: false });
    const polls = sent.filter((s) => s.url === "/admin/upstream").length;
    await new Promise((done) => setTimeout(done, 60));
    expect(sent.filter((s) => s.url === "/admin/upstream")).toHaveLength(polls);
  });

  /**
   * **古い読み直しが、新しい切り替えの結果を上書きしない。**
   *
   * 読み直しが飛んでいる最中に切り替えると、応答の着く順は保証されない。
   * **後から着いた古い応答で「準備完了」に戻ると、いま載せているものが見えなくなる。**
   */
  it("切り替えの後に着いた古い読み直しで、表が巻き戻らない", async () => {
    const stale = deferred();
    upstreamReplies = [ok(BETA_LOADING), stale.reply];
    const fresh = report({ ...EXTERNAL, state: "loading", since: "2026-09-25T11:11:11+09:00" }, EXTERNAL);
    onSwitch = () => ok(fresh);
    const view = await open();
    // 開いた時点で載せている → 読み直しが飛ぶ（それを解かずに置いておく）。
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.url === "/admin/upstream")).toHaveLength(2);
    });

    // 切り替えた後の読み直しは、切り替えの応答と同じものを返す。
    upstreamDefault = fresh;
    mustButton(profileBlock("beta"), "このプロファイルに切り替える").click();
    await vi.waitFor(() => {
      expect(launchCells()[1]?.[1]).toBe("載せています（22:11:11 から）");
    });
    stale.resolve(BETA_SETTLED);
    await new Promise((done) => setTimeout(done, 30));
    expect(launchCells()[1]?.[1]).toBe("載せています（22:11:11 から）");
    await unmount(view, { outro: false });
  });

  /**
   * **載せる面の無い相手へ切り替えたときも、古い読み直しで巻き戻らない。**
   *
   * 上の1本では、切り替えの応答が「載せている」なので**次の読み直しが番号を進め**、
   * 切り替えの応答を採るときに番号を進めなくても古い応答は捨てられる（変異 M08 が緑だった）。
   * **載せる面が無ければ次の読み直しは無い**ので、採るときに進めた番号だけが守る。
   */
  it("載せる面の無い相手へ切り替えた後に着いた古い読み直しで、表が巻き戻らない", async () => {
    const stale = deferred();
    upstreamReplies = [ok(BETA_LOADING), stale.reply];
    onSwitch = () => ok(IDLE);
    const view = await open();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.url === "/admin/upstream")).toHaveLength(2);
    });

    mustButton(profileBlock("beta"), "このプロファイルに切り替える").click();
    await vi.waitFor(() => {
      expect(launchCells()[1]?.[1]).toBe("起こしていません（外の上流に依頼しています）");
    });
    stale.resolve(BETA_SETTLED);
    await new Promise((done) => setTimeout(done, 30));
    expect(launchCells()[1]).toEqual([
      "起こした上流の状態",
      "起こしていません（外の上流に依頼しています）",
      "起こしていません（外の上流に依頼しています）",
    ]);
    // **切り替えの後は読み直していない**（載せる面が無い）。
    expect(sent.filter((s) => s.url === "/admin/upstream")).toHaveLength(2);
    await unmount(view, { outro: false });
  });

  /**
   * **切り替えの後に着いた古い読み直しの「失敗」で、表が消えない**（レビュー1周目 R03）。
   * 失敗の枝も番号を照合しないと、切り替えの結果の上に古い失敗が被さる。
   */
  it("切り替えの後に着いた古い読み直しの失敗で、表が消えない", async () => {
    const stale = deferred();
    upstreamReplies = [ok(BETA_LOADING), stale.reply];
    onSwitch = () => ok(IDLE);
    const view = await open();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.url === "/admin/upstream")).toHaveLength(2);
    });

    mustButton(profileBlock("beta"), "このプロファイルに切り替える").click();
    await vi.waitFor(() => {
      expect(launchCells()[1]?.[1]).toBe("起こしていません（外の上流に依頼しています）");
    });
    stale.fail(500, "boom");
    await new Promise((done) => setTimeout(done, 30));
    expect(flat(sectionNamed(NOW))).not.toContain("上流の様子を取得できませんでした");
    expect(launchCells()[1]).toEqual([
      "起こした上流の状態",
      "起こしていません（外の上流に依頼しています）",
      "起こしていません（外の上流に依頼しています）",
    ]);
    await unmount(view, { outro: false });
  });

  /**
   * **観測が失敗の表示のまま切り替えても、切り替えの結果が出る**（レビュー1周目 R04）。
   * 失敗の表示を消さないと、表の代わりに古い失敗が出続ける。
   */
  it("観測が読めていない状態から切り替えると、失敗の表示が消えて表が出る", async () => {
    upstreamReplies = [{ status: 500, body: "boom" }];
    // **載せる面の無い相手へ切り替える**（次の読み直しが無い）。
    // 1周目の是正では「載せている」相手へ切り替えていて、**続く読み直しが失敗の表示を消すので、
    // 採るときに消さない変異（R04）が緑のままだった。**
    onSwitch = () => ok(IDLE);
    const view = await open();
    await vi.waitFor(() => {
      expect(flat(sectionNamed(NOW))).toContain("上流の様子を取得できませんでした");
    });

    mustButton(profileBlock("beta"), "このプロファイルに切り替える").click();
    await vi.waitFor(() => {
      expect(flat(sectionNamed(WRITTEN))).toContain("「beta」に切り替えました。");
    });
    expect(flat(sectionNamed(NOW))).not.toContain("上流の様子を取得できませんでした");
    expect(launchCells()[1]).toEqual([
      "起こした上流の状態",
      "起こしていません（外の上流に依頼しています）",
      "起こしていません（外の上流に依頼しています）",
    ]);
    // **切り替えの後は読み直していない**（表を出したのは切り替えの応答そのもの）。
    expect(sent.filter((s) => s.url === "/admin/upstream")).toHaveLength(1);
    await unmount(view, { outro: false });
  });

  it("進行中の 409 は、理由を出し、表を変えない", async () => {
    const message = "上流の差し替えが進行中です。終わってから選び直してください";
    onSwitch = () => apiError(409, "upstream_switch_in_progress", message);
    const view = await open();

    mustButton(profileBlock("beta"), "このプロファイルに切り替える").click();
    await vi.waitFor(() => {
      expect(target.querySelector("[role=alert]")).not.toBeNull();
    });
    expect([...sectionNamed(WRITTEN).querySelectorAll("[role=alert]")].map((a) => flat(a))).toEqual([
      `切り替えられませんでした: ${message}`,
    ]);
    expect(launchCells()[1]?.[1]).toBe("起こしていません（外の上流に依頼しています）");
    await unmount(view, { outro: false });
  });

  it("切り替えの応答を待つ間は、どのプロファイルも押せない", async () => {
    const pending = deferred();
    onSwitch = () => pending.reply;
    const view = await open();

    mustButton(profileBlock("beta"), "このプロファイルに切り替える").click();
    await vi.waitFor(() => {
      expect(mustButton(profileBlock("alpha"), "このプロファイルに切り替える").disabled).toBe(true);
    });
    expect(
      ["alpha", "beta", "gamma"].map((name) => mustButton(profileBlock(name), "このプロファイルに切り替える").disabled),
    ).toEqual([true, true, true]);

    pending.resolve(BETA_LOADING);
    await vi.waitFor(() => {
      expect(mustButton(profileBlock("gamma"), "このプロファイルに切り替える").disabled).toBe(false);
    });
    await unmount(view, { outro: false });
  });
});

describe("足す・直す", () => {
  /** サーバの振る舞い: **送られた内容で1件を置き換え、読み直した一覧を返す。** */
  function savingServer(): void {
    onSave = (name, body) => {
      const request = body as { main: WrittenFace; sub: WrittenFace | null };
      const saved: WrittenProfile = { name, main: request.main, sub: request.sub };
      const profiles = [...listing().profiles.filter((p) => p.name !== name), saved].sort((a, b) =>
        a.name.localeCompare(b.name),
      );
      return ok(listing({ revision: "rev-2", profiles }));
    };
  }

  it("直す: 真ん中のプロファイルの書いたままの値が、面ごとの欄に入る", async () => {
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });

    expect(flat(target.querySelector("form h3"))).toBe("「beta」を直す");
    expect(faceValues(MAIN_LEGEND)).toEqual([
      "http://127.0.0.1:41003",
      "beta-main",
      "D:\\models\\beta-main.gguf",
      "--threads\n6",
      "D:/bin/llama-beta.exe",
      "",
    ]);
    expect(faceValues(SUB_LEGEND)).toEqual([
      "http://127.0.0.1:41004",
      "beta-sub",
      "D:\\models\\beta-sub.gguf",
      "--batch-size\n256",
      "D:/bin/llama-beta-sub.exe",
      "10240",
    ]);
    const hasSub = target.querySelector<HTMLInputElement>("form input[type=checkbox]");
    expect(hasSub?.checked).toBe(true);
    await unmount(view, { outro: false });
  });

  /**
   * **保存後の一覧が、保存した内容と一致する**（Issue origin#24 の閉じる条件）。
   *
   * 送った本文を丸ごと固定し、**サーバが読み直して返した一覧**を画面が出すことを、セルで見る。
   */
  it("直す: 保存すると、その名前へ直した内容を送り、読み直した一覧を出す", async () => {
    savingServer();
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });

    type(fieldOf(MAIN_LEGEND, "依頼するモデルの綴り（model）"), "beta-main-2");
    type(fieldOf(SUB_LEGEND, "起動の引数（args）"), "--batch-size\n512\n--alias\ntwo words");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();

    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    expect(sent.filter((s) => s.method === "PUT")).toEqual([
      {
        method: "PUT",
        url: "/admin/upstream/profiles/beta",
        body: {
          revision: "rev-1",
          main: requested({ ...BETA.main, model: "beta-main-2" }),
          sub: requested(
            {
              ...(BETA.sub as WrittenFace),
              args: ["--batch-size", "512", "--alias", "two words"],
            },
            "sub",
          ),
        },
      },
    ]);
    // **一覧は、サーバが読み直して返した一覧**（要点の1行。モデルと引数は要点に出ないので、
    // 何を送ったかは上の本文で見ている）。
    expect(summaryOf("beta")).toEqual([
      ["本文", "起こす beta-main.gguf → :41003"],
      ["裏方", "起こす beta-sub.gguf → :41004 · 窓 10,240"],
    ]);
    // **他の2件は触っていない。**
    expect(summaryOf("alpha")).toEqual([
      ["本文", "起こす alpha-main.gguf → :41001 · 窓 28,672"],
      ["裏方", "起こす alpha-sub.gguf → :41002"],
    ]);
    // **保存しても、いま動いている上流は変わらないと言う。**
    expect(flat(sectionNamed(WRITTEN))).toContain(
      "「beta」を保存しました。いま動いている上流は、このプロファイルに切り替えるまで変わりません。",
    );
    // **次の保存は、読み直した印を添える。**
    mustButton(profileBlock("gamma"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.method === "PUT")).toHaveLength(2);
    });
    expect((sent.filter((s) => s.method === "PUT")[1]?.body as { revision: string }).revision).toBe("rev-2");
    await unmount(view, { outro: false });
  });

  it("直す: 裏方を別にしないと決めたら、裏方を消して送る", async () => {
    savingServer();
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    (target.querySelector("form input[type=checkbox]") as HTMLInputElement).click();
    await vi.waitFor(() => {
      expect(
        [...(target.querySelector("form") as HTMLFormElement).querySelectorAll("legend")].map((l) => flat(l)),
      ).toEqual([MAIN_LEGEND]);
    });
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    expect(sent.filter((s) => s.method === "PUT").map((s) => s.body)).toEqual([
      { revision: "rev-1", main: requested(BETA.main), sub: null },
    ]);
    expect(summaryOf("beta")).toEqual([
      ["本文", "起こす beta-main.gguf → :41003"],
      ["裏方", "本文と同じ"],
    ]);
    await unmount(view, { outro: false });
  });

  /**
   * **コンテキスト長を書き・空にする**（作業単位 A-13a-2）。
   *
   * `beta` は本文に書いておらず、裏方に 10240 を書いてある。**本文に書き、裏方を空にする**
   * ——面を取り違える実装は、送る本文でも表のセルでも値が逆の列に出る。
   */
  it("直す: 本文のコンテキスト長を書き、裏方のコンテキスト長を空にして送る", async () => {
    savingServer();
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    type(fieldOf(MAIN_LEGEND, WINDOW_LABEL), "20480");
    type(fieldOf(SUB_LEGEND, WINDOW_LABEL), "");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    expect(sent.filter((s) => s.method === "PUT").map((s) => s.body)).toEqual([
      {
        revision: "rev-1",
        main: requested({ ...BETA.main, context_tokens: 20480 }),
        sub: requested({ ...(BETA.sub as WrittenFace), context_tokens: null }, "sub"),
      },
    ]);
    expect(summaryOf("beta")).toEqual([
      ["本文", "起こす beta-main.gguf → :41003 · 窓 20,480"],
      ["裏方", "起こす beta-sub.gguf → :41004"],
    ]);
    // **他のプロファイルの値は動かない。**
    expect(summaryOf("alpha")).toEqual([
      ["本文", "起こす alpha-main.gguf → :41001 · 窓 28,672"],
      ["裏方", "起こす alpha-sub.gguf → :41002"],
    ]);
    // **起こす面のコンテキスト長は「詳細」に畳んである**（作業単位 D-19 / ADR-0054 —— 起こす面にも書ける）。
    // 下書きを開き直して、欄の在り処を見る。
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    expect(fieldOf(MAIN_LEGEND, WINDOW_LABEL).closest("details")).not.toBeNull();
    expect(fieldOf(MAIN_LEGEND, WINDOW_LABEL).value).toBe("20480");
    await unmount(view, { outro: false });
  });

  /**
   * **数として読めない値は送らず、その欄で言う。直せば送る**（止める → 直す → 送る の3相）。
   *
   * 欄は `type="text"`（`type="number"` だと、読めない入力の値が空文字になり「書かない」として黙って送られる）。
   */
  it("直す: 数として読めないコンテキスト長は送らず、その面の欄にそう言い、直せば送る", async () => {
    savingServer();
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    expect(fieldOf(SUB_LEGEND, WINDOW_LABEL).getAttribute("type")).toBe("text");
    type(fieldOf(SUB_LEGEND, WINDOW_LABEL), "32k");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();

    const culprit = (): Element => fieldOf(SUB_LEGEND, WINDOW_LABEL);
    await vi.waitFor(() => {
      expect(culprit().getAttribute("aria-invalid")).toBe("true");
    });
    expect(culprit().getAttribute("aria-describedby")).toBe("profile-sub-context_tokens-error");
    expect(describedText(culprit())).toBe(
      "保存しませんでした: コンテキスト長は、半角の数字だけで書いてください（空にすると書きません）。",
    );
    expect(sent.filter((s) => s.method === "PUT")).toEqual([]);
    // **本文の同じ欄は無効にならない。**
    expect(fieldOf(MAIN_LEGEND, WINDOW_LABEL).getAttribute("aria-invalid")).toBeNull();

    type(fieldOf(SUB_LEGEND, WINDOW_LABEL), "32768");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.method === "PUT")).toHaveLength(1);
    });
    expect(
      (sent.filter((s) => s.method === "PUT")[0]?.body as { sub: WrittenFace }).sub.context_tokens,
    ).toBe(32768);
    await unmount(view, { outro: false });
  });

  it("足す: 名前と2つの面を書いて保存すると、その名前で送り、一覧に並ぶ", async () => {
    savingServer();
    const view = await open();
    mustButton(sectionNamed(WRITTEN), "プロファイルを足す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    expect(flat(target.querySelector("form h3"))).toBe("プロファイルを足す");
    // **足すときは空から始まる**（直前に見た一覧の値を持ち込まない）。**本文は起こすで開く**ので、起こす面の6つの欄。
    expect(faceValues(MAIN_LEGEND)).toEqual(["", "", "", "", "", ""]);

    // **足すときの本文は起こすで開く**（承認された絵のとおり / D-19 の是正）。
    expect(pressedOf(MAIN_LEGEND)).toEqual(["true", "false"]);
    type(nameField(), "delta");
    type(fieldOf(MAIN_LEGEND, ENDPOINT_LAUNCH), "http://127.0.0.1:41007");
    type(fieldOf(MAIN_LEGEND, "依頼するモデルの綴り（model）"), "delta-main");
    type(fieldOf(MAIN_LEGEND, "gguf のパス（gguf）"), "~/models/delta-main.gguf");
    type(fieldOf(MAIN_LEGEND, "起動の引数（args）"), "--ctx-size\n8192");
    (target.querySelector("form input[type=checkbox]") as HTMLInputElement).click();
    await vi.waitFor(() => {
      expect(
        [...(target.querySelector("form") as HTMLFormElement).querySelectorAll("legend")].map((l) => flat(l)),
      ).toEqual([MAIN_LEGEND, SUB_LEGEND]);
    });
    // **裏方もチェックを入れると起こすで開く。** ここでは繋ぐに切り替えて書く（宛先・モデル・コンテキスト長）。
    expect(pressedOf(SUB_LEGEND)).toEqual(["true", "false"]);
    kindButtons(SUB_LEGEND)[1].click();
    await vi.waitFor(() => {
      expect(pressedOf(SUB_LEGEND)).toEqual(["false", "true"]);
    });
    type(fieldOf(SUB_LEGEND, ENDPOINT_CONNECT), "http://127.0.0.1:41008");
    type(fieldOf(SUB_LEGEND, "依頼するモデルの綴り（model）"), "delta-sub");
    type(fieldOf(SUB_LEGEND, WINDOW_LABEL), "12288");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();

    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    expect(sent.filter((s) => s.method === "PUT")).toEqual([
      {
        method: "PUT",
        url: "/admin/upstream/profiles/delta",
        body: {
          revision: "rev-1",
          main: {
            endpoint: "http://127.0.0.1:41007",
            model: "delta-main",
            gguf: "~/models/delta-main.gguf",
            args: ["--ctx-size", "8192"],
            command: null,
            context_tokens: null,
            // **足すときも2つの欄を必ず送る**（空は `null` / 作業単位 D-21）。
            sampling: null,
            response_tokens: null,
          },
          sub: {
            endpoint: "http://127.0.0.1:41008",
            model: "delta-sub",
            gguf: null,
            args: null,
            command: null,
            context_tokens: 12288,
            // **裏方には `response_tokens` を載せない。**
            sampling: null,
          },
        },
      },
    ]);
    expect(listedNames()).toEqual(["alpha", "beta", "delta", "gamma"]);
    expect(summaryOf("delta")).toEqual([
      ["本文", "起こす delta-main.gguf → :41007"],
      ["裏方", "繋ぐ http://127.0.0.1:41008 · 窓 12,288"],
    ]);
    await unmount(view, { outro: false });
  });

  /** **在る名前で足すと、サーバはそのプロファイルを置き換える。** 画面で止める。 */
  it("足す: 在る名前は送らず、名前の欄にそう言う", async () => {
    const view = await open();
    mustButton(sectionNamed(WRITTEN), "プロファイルを足す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    type(nameField(), "beta");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(nameField().getAttribute("aria-invalid")).toBe("true");
    });
    expect(describedText(nameField())).toBe(
      "その名前のプロファイルはもうあります。直すときは、一覧のその名前の「直す」から開いてください。",
    );
    expect(sent.filter((s) => s.method === "PUT")).toEqual([]);
    await unmount(view, { outro: false });
  });

  /**
   * **下書きは、開いたときの印で送る**（レビュー1周目 要修正3 / プローブ `zz-review-probe2`）。
   *
   * 読み直しが飛んでいる間に「直す」を開くと、下書きは古い一覧の値を持つ。
   * その後に新しい一覧（新しい印）が着いても、**下書きの値は古いまま**なので、
   * 新しい印を添えて送ると**外の書き換えを黙って上書きする。**
   * 開いたときの印を送れば、サーバが 409 で止める。
   */
  it.each([
    ["直す", "rev-1"],
    ["足す", "rev-1"],
  ])("%s: 読み直しの最中に開いた下書きは、開いたときの印で送る", async (mode, expected) => {
    savingServer();
    const reload = deferred();
    const view = await open();
    listingReplies = [reload.reply];
    mustButton(sectionNamed(WRITTEN), "読み直す").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.url === "/admin/upstream/profiles")).toHaveLength(2);
    });

    // **古い一覧（rev-1）から開く。**
    if (mode === "直す") {
      mustButton(profileBlock("beta"), "直す").click();
    } else {
      mustButton(sectionNamed(WRITTEN), "プロファイルを足す").click();
    }
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    if (mode === "足す") {
      type(nameField(), "delta");
      type(fieldOf(MAIN_LEGEND, ENDPOINT_LAUNCH), "http://127.0.0.1:41009");
      type(fieldOf(MAIN_LEGEND, "依頼するモデルの綴り（model）"), "delta-main");
      // **起こす面は gguf が要る**（空なら送らずに止まる / D-19 のレビュー）。
      type(fieldOf(MAIN_LEGEND, "gguf のパス（gguf）"), "~/models/delta-main.gguf");
    }
    // **外で書き換えられた一覧（rev-2）が着く**（一覧の要点に出る待ち受けのポートを変えてある）。
    reload.resolve(
      listing({
        revision: "rev-2",
        profiles: [ALPHA, { ...BETA, main: { ...BETA.main, endpoint: "http://127.0.0.1:41013" } }, GAMMA],
      }),
    );
    await vi.waitFor(() => {
      expect(summaryOf("beta")[0]).toEqual(["本文", "起こす beta-main.gguf → :41013"]);
    });

    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.method === "PUT")).toHaveLength(1);
    });
    expect((sent.filter((s) => s.method === "PUT")[0]?.body as { revision: string | null }).revision).toBe(
      expected,
    );
    await unmount(view, { outro: false });
  });

  it("やめると、何も送らずに閉じる", async () => {
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    mustButton(target.querySelector("form") as HTMLFormElement, "やめる").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    expect(sent.filter((s) => s.method !== "GET")).toEqual([]);
    await unmount(view, { outro: false });
  });
});

/**
 * **編集欄は押した場所のすぐ下に出し、開いたら見出しへフォーカスを移して巻き取り、閉じたら押したボタンへ戻す**
 * （作業単位 B-70 / Issue origin#99）。
 *
 * 着手前は、欄が一覧の一番下に出て、巻き取りもしなかった —— プロファイルが増えると、押しても何も起きなかったように見えた。
 *
 * **直すのは真ん中（`beta`）。** 「最初の行の後」「最後の行の後（＝一覧の一番下）」に出す実装は、どちらもここで赤になる。
 * 位置は jsdom では測れないので、DOM の順序（直前の兄弟）と、`scrollIntoView` が欄に呼ばれたことを見る。
 */
describe("編集欄の場所とフォーカス", () => {
  let scrolled: { element: Element; options: unknown }[];

  beforeEach(() => {
    scrolled = [];
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      writable: true,
      value: function (this: Element, options?: unknown) {
        scrolled.push({ element: this, options });
      },
    });
  });

  afterEach(() => {
    // jsdom の既定（`scrollIntoView` が無い）に戻す。ほかの節は、無いときに落ちないことを見ている。
    delete (HTMLElement.prototype as { scrollIntoView?: unknown }).scrollIntoView;
  });

  function editor(): HTMLFormElement {
    const form = target.querySelector("form");
    expect(form, "編集欄が開いていること").not.toBeNull();
    return form as HTMLFormElement;
  }

  /** 戻す処理は `tick()` の後に走るので、何周か待つ（「戻さない」ことを見るとき）。 */
  async function settle(): Promise<void> {
    for (let turn = 0; turn < 5; turn += 1) {
      await new Promise((done) => setTimeout(done, 0));
    }
  }

  it("直す: 欄は押した行のすぐ下に出て、見出しへフォーカスが移り、欄が巻き取られる", async () => {
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    const form = editor();
    // **押した行の直後の兄弟**で、次の行（`gamma`）より前。
    expect(form.previousElementSibling).toBe(profileBlock("beta"));
    expect(form.compareDocumentPosition(profileBlock("gamma")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // **編集欄は1つだけ。**
    expect(target.querySelectorAll("form")).toHaveLength(1);

    const title = form.querySelector("h3") as HTMLElement;
    expect(flat(title)).toBe("「beta」を直す");
    await vi.waitFor(() => expect(document.activeElement).toBe(title));
    expect(scrolled).toEqual([{ element: form, options: { block: "nearest" } }]);
    await unmount(view, { outro: false });
  });

  it("足す: 欄は「プロファイルを足す」の並びのすぐ下、一覧より前に出て、見出しへフォーカスが移る", async () => {
    const view = await open();
    const add = mustButton(sectionNamed(WRITTEN), "プロファイルを足す");
    add.click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    const form = editor();
    expect(form.previousElementSibling?.contains(add)).toBe(true);
    expect(form.compareDocumentPosition(profileBlock("alpha")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(target.querySelectorAll("form")).toHaveLength(1);

    const title = form.querySelector("h3") as HTMLElement;
    expect(flat(title)).toBe("プロファイルを足す");
    await vi.waitFor(() => expect(document.activeElement).toBe(title));
    expect(scrolled).toEqual([{ element: form, options: { block: "nearest" } }]);
    await unmount(view, { outro: false });
  });

  it("直す: 開いている間は、ほかの行の「直す」と「プロファイルを足す」を押せない", async () => {
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    expect(["alpha", "beta", "gamma"].map((name) => mustButton(profileBlock(name), "直す").disabled)).toEqual([
      true,
      true,
      true,
    ]);
    expect(mustButton(sectionNamed(WRITTEN), "プロファイルを足す").disabled).toBe(true);
    await unmount(view, { outro: false });
  });

  it("直す: やめると、押した行の「直す」へフォーカスが戻る", async () => {
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => expect(editor().contains(document.activeElement)).toBe(true));
    mustButton(editor(), "やめる").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    await vi.waitFor(() => expect(document.activeElement).toBe(mustButton(profileBlock("beta"), "直す")));
    await unmount(view, { outro: false });
  });

  it("直す: 保存すると、読み直した一覧の同じプロファイルの「直す」へフォーカスが戻り、知らせは一覧の前に出る", async () => {
    // **読み直した一覧は、並びを変えて返す**（同じ行の「直す」へ戻ることを、位置ではなく名前で見る）。
    onSave = () => ok(listing({ revision: "rev-2", profiles: [GAMMA, BETA, ALPHA] }));
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => expect(editor().contains(document.activeElement)).toBe(true));
    mustButton(editor(), "保存する").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    await vi.waitFor(() => expect(document.activeElement).toBe(mustButton(profileBlock("beta"), "直す")));
    expect(listedNames()).toEqual(["gamma", "beta", "alpha"]);
    // **保存の知らせは今までどおり一覧の前**（編集欄の位置に引きずられない）。
    const notice = [...sectionNamed(WRITTEN).querySelectorAll("p.notice")].find((p) =>
      flat(p).startsWith("「beta」を保存しました。"),
    );
    expect(notice, "保存の知らせが出ていること").toBeDefined();
    expect(
      (notice as Element).compareDocumentPosition(profileBlock("gamma")) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    await unmount(view, { outro: false });
  });

  it("足す: やめると「プロファイルを足す」へ、保存しても「プロファイルを足す」へフォーカスが戻る", async () => {
    onSave = (name, body) => {
      const request = body as { main: WrittenFace; sub: WrittenFace | null };
      return ok(listing({ revision: "rev-2", profiles: [ALPHA, BETA, { name, ...request }, GAMMA] }));
    };
    const view = await open();
    const add = mustButton(sectionNamed(WRITTEN), "プロファイルを足す");
    add.click();
    await vi.waitFor(() => expect(editor().contains(document.activeElement)).toBe(true));
    mustButton(editor(), "やめる").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    await vi.waitFor(() => expect(document.activeElement).toBe(add));

    add.click();
    await vi.waitFor(() => expect(editor().contains(document.activeElement)).toBe(true));
    type(nameField(), "delta");
    type(fieldOf(MAIN_LEGEND, ENDPOINT_LAUNCH), "http://127.0.0.1:41007");
    type(fieldOf(MAIN_LEGEND, "依頼するモデルの綴り（model）"), "delta-main");
    type(fieldOf(MAIN_LEGEND, "gguf のパス（gguf）"), "~/models/delta-main.gguf");
    mustButton(editor(), "保存する").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    expect(listedNames()).toEqual(["alpha", "beta", "delta", "gamma"]);
    await vi.waitFor(() => expect(document.activeElement).toBe(mustButton(sectionNamed(WRITTEN), "プロファイルを足す")));
    await unmount(view, { outro: false });
  });

  /**
   * **409 のあと、欄の中の「読み直す」で閉じても、押した行の「直す」へ戻る**（読み直した一覧に `beta` が残る形）。
   * 読み直しは並びを変えて返す（位置ではなく名前で戻ることを、読み直しが着いた後に見る）。
   */
  it("直す: 409 のあと欄の中の「読み直す」で閉じると、読み直した一覧の同じプロファイルの「直す」へフォーカスが戻る", async () => {
    onSave = () =>
      apiError(
        409,
        "upstream_profiles_changed",
        "上流プロファイルは読んだ後に書き換えられています。読み直してから直してください",
      );
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => expect(editor().contains(document.activeElement)).toBe(true));
    mustButton(editor(), "保存する").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form [role=alert]")).not.toBeNull();
    });
    listingReplies = [ok(listing({ revision: "rev-9", profiles: [GAMMA, BETA, ALPHA] }))];
    mustButton(target.querySelector("form [role=alert]") as Element, "読み直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
      expect(listedNames()).toEqual(["gamma", "beta", "alpha"]);
    });
    await settle();
    expect(document.activeElement).toBe(mustButton(profileBlock("beta"), "直す"));
    await unmount(view, { outro: false });
  });

  /**
   * **対照: 利用者がほかの場所を触っているときに閉じても、フォーカスを奪わない**
   * （A-13b-3 の再々レビュー是正と同じ考え方 —— 戻すのは、フォーカスが欄の中にあって、欄ごと消えて落ちたときだけ）。
   */
  it("直す: 欄の外へフォーカスを移してから閉じると、フォーカスはそのまま", async () => {
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => expect(editor().contains(document.activeElement)).toBe(true));
    const elsewhere = mustButton(sectionNamed(WRITTEN), "読み直す");
    elsewhere.focus();
    expect(document.activeElement).toBe(elsewhere);
    mustButton(editor(), "やめる").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    await settle();
    expect(document.activeElement).toBe(elsewhere);
    await unmount(view, { outro: false });
  });
});

/**
 * **面ごとに「起こす」「繋ぐ」を選ぶ**（作業単位 D-19 / Issue origin#87）。
 *
 * 着手前は、面が起こすのか繋ぐのかを gguf の欄が埋まっているかで暗に決めていて、画面はそれを言わなかった。
 * **選んだ種類の欄だけを出し、保存で送るのも選んだ種類の欄だけ。もう一方の欄の値は下書きに残す。**
 */
describe("面の種類", () => {
  /**
   * **鍵は両方の種類の欄**（作業単位 A-13b-3 のレビュー是正で、起こす面にも出した）。
   *
   * **作業単位 D-21 で、本文の面に返答の上限（すぐ出す）、両面に繰り返しの罰（詳細の最後）が入った**（ADR-0060 §決定3）。
   * 思考の2択とサンプリングの組は `label` ではない（組の見出しとボタン）ので、ここには数えない。
   */
  const LAUNCH_LABELS_SUB = [
    "gguf のパス（gguf）",
    "依頼するモデルの綴り（model）",
    ENDPOINT_LAUNCH,
    "鍵（api_key）",
    "起動の引数（args）",
    "実行ファイル（command）",
    WINDOW_LABEL,
    "繰り返しの罰（repeat_penalty）",
  ];
  const LAUNCH_LABELS = [
    "gguf のパス（gguf）",
    "依頼するモデルの綴り（model）",
    ENDPOINT_LAUNCH,
    "鍵（api_key）",
    "返答の上限（response_tokens）",
    "起動の引数（args）",
    "実行ファイル（command）",
    WINDOW_LABEL,
    "繰り返しの罰（repeat_penalty）",
  ];
  /** **鍵は両方の種類の欄**（作業単位 A-13b-3 で足した）。 */
  const CONNECT_LABELS = [
    ENDPOINT_CONNECT,
    "依頼するモデルの綴り（model）",
    "鍵（api_key）",
    WINDOW_LABEL,
    "返答の上限（response_tokens）",
    "繰り返しの罰（repeat_penalty）",
  ];

  async function openEditor(name: string): Promise<Record<string, unknown>> {
    const view = await open();
    mustButton(profileBlock(name), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    return view;
  }

  /** **詳細に畳んだ欄**（起こす面の起動の引数・実行ファイル・コンテキスト長）。 */
  function foldedLabels(legend: string): string[] {
    return [...fieldsetOf(legend).querySelectorAll("details label")].map((label) => flat(label));
  }

  /**
   * **開いたときの種類は、その面に gguf があれば起こす、無ければ繋ぐ。**
   * 種類を選ぶボタンが `aria-pressed` で状態を名乗り、その種類の欄だけが出る。
   */
  it("gguf を書いた面は起こす、書いていない面は繋ぐで開き、その種類の欄だけを出す", async () => {
    const view = await openEditor("beta");
    expect([pressedOf(MAIN_LEGEND), pressedOf(SUB_LEGEND)]).toEqual([
      ["true", "false"],
      ["true", "false"],
    ]);
    expect(labelsOf(MAIN_LEGEND)).toEqual(LAUNCH_LABELS);
    // **起動の引数・実行ファイル・コンテキスト長は「詳細」に畳む**（コンテキスト長も落とさない / ADR-0054）。
    // **繰り返しの罰も詳細に畳む**（作業単位 D-21 / ADR-0060 §決定3）。
    expect(foldedLabels(MAIN_LEGEND)).toEqual([
      "起動の引数（args）",
      "実行ファイル（command）",
      WINDOW_LABEL,
      "繰り返しの罰（repeat_penalty）",
    ]);
    expect(flat(fieldsetOf(MAIN_LEGEND).querySelector("details > summary"))).toBe(
      "詳細（起動の引数・実行ファイル・コンテキスト長・繰り返しの罰）",
    );
    mustButton(target.querySelector("form") as HTMLFormElement, "やめる").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });

    mustButton(profileBlock("gamma"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    expect(pressedOf(MAIN_LEGEND)).toEqual(["false", "true"]);
    expect(labelsOf(MAIN_LEGEND)).toEqual(CONNECT_LABELS);
    // **繋ぐ面の詳細は、繰り返しの罰だけ**（作業単位 D-21。D-19 の間は繋ぐ面に詳細が無かった）。
    expect(foldedLabels(MAIN_LEGEND)).toEqual(["繰り返しの罰（repeat_penalty）"]);
    await unmount(view, { outro: false });
  });

  /**
   * **足すときは、本文を起こすで開く。裏方はチェックを入れたときに起こすで開く**（承認された絵のとおり / D-19 の是正）。
   * **対照: 直すときの規則は変えない** —— 裏方を書いていない `gamma` でチェックを入れると、裏方は繋ぐで開く
   * （書いていない面には gguf が無い）。
   */
  it("足すときは本文も裏方も起こすで開き、直すときは gguf の有無の規則のまま", async () => {
    const view = await open();
    mustButton(sectionNamed(WRITTEN), "プロファイルを足す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    expect(pressedOf(MAIN_LEGEND)).toEqual(["true", "false"]);
    expect(labelsOf(MAIN_LEGEND)).toEqual(LAUNCH_LABELS);
    (target.querySelector("form input[type=checkbox]") as HTMLInputElement).click();
    await vi.waitFor(() => {
      expect(target.querySelectorAll("form fieldset")).toHaveLength(2);
    });
    expect(pressedOf(SUB_LEGEND)).toEqual(["true", "false"]);
    expect(labelsOf(SUB_LEGEND)).toEqual(LAUNCH_LABELS_SUB);
    mustButton(target.querySelector("form") as HTMLFormElement, "やめる").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });

    mustButton(profileBlock("gamma"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    (target.querySelector("form input[type=checkbox]") as HTMLInputElement).click();
    await vi.waitFor(() => {
      expect(target.querySelectorAll("form fieldset")).toHaveLength(2);
    });
    expect([pressedOf(MAIN_LEGEND), pressedOf(SUB_LEGEND)]).toEqual([
      ["false", "true"],
      ["false", "true"],
    ]);
    await unmount(view, { outro: false });
  });

  /**
   * **繋ぐにすると、gguf・起動の引数・実行ファイルは送らない**（ファイルから消える）。
   * **下書きには残る** —— 起こすに戻すと元の値が欄に戻る。**もう片方の面（裏方）は動かない。**
   * **止める → 戻す → 送る** の順で、「戻しても元に戻らない」実装と「繋ぐでも送る」実装の両方を落とす。
   */
  it("繋ぐにした面は gguf・引数・実行ファイルを送らず、戻せば下書きの値が元に戻る", async () => {
    onSave = (name, body) => {
      const request = body as { main: WrittenFace; sub: WrittenFace | null };
      const saved: WrittenProfile = {
        name,
        main: { ...request.main, api_key_set: false, api_key_hint: null },
        sub: request.sub === null ? null : { ...request.sub, api_key_set: false, api_key_hint: null },
      };
      return ok(listing({ revision: "rev-2", profiles: [ALPHA, saved, GAMMA] }));
    };
    const view = await openEditor("beta");
    type(fieldOf(MAIN_LEGEND, WINDOW_LABEL), "24576");

    kindButtons(MAIN_LEGEND)[1].click();
    await vi.waitFor(() => {
      expect(pressedOf(MAIN_LEGEND)).toEqual(["false", "true"]);
    });
    expect(labelsOf(MAIN_LEGEND)).toEqual(CONNECT_LABELS);
    // **宛先・モデル・コンテキスト長は、種類をまたいで同じ値**（同じ鍵）。
    expect([ENDPOINT_CONNECT, "依頼するモデルの綴り（model）", WINDOW_LABEL].map((label) => fieldOf(MAIN_LEGEND, label).value)).toEqual([
      "http://127.0.0.1:41003",
      "beta-main",
      "24576",
    ]);
    // **裏方は起こすのまま。**
    expect(pressedOf(SUB_LEGEND)).toEqual(["true", "false"]);

    // **戻すと、下書きの値が元に戻る。**
    kindButtons(MAIN_LEGEND)[0].click();
    await vi.waitFor(() => {
      expect(pressedOf(MAIN_LEGEND)).toEqual(["true", "false"]);
    });
    expect(faceValues(MAIN_LEGEND)).toEqual([
      "http://127.0.0.1:41003",
      "beta-main",
      "D:\\models\\beta-main.gguf",
      "--threads\n6",
      "D:/bin/llama-beta.exe",
      "24576",
    ]);

    // **繋ぐにして保存すると、繋ぐ面の欄だけを送る。**
    kindButtons(MAIN_LEGEND)[1].click();
    await vi.waitFor(() => {
      expect(pressedOf(MAIN_LEGEND)).toEqual(["false", "true"]);
    });
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    expect(sent.filter((s) => s.method === "PUT").map((s) => s.body)).toEqual([
      {
        revision: "rev-1",
        main: {
          endpoint: "http://127.0.0.1:41003",
          model: "beta-main",
          gguf: null,
          args: null,
          command: null,
          context_tokens: 24576,
          // **思考・返答の上限・繰り返しの罰は種類によらず送る**（作業単位 D-21 —— 起こす面・繋ぐ面とも同じ欄）。
          sampling: { repeat_penalty: 1.05, reasoning_effort: "low" },
          response_tokens: 2000,
        },
        sub: requested(BETA.sub as WrittenFace, "sub"),
      },
    ]);
    expect(summaryOf("beta")).toEqual([
      ["本文", "繋ぐ http://127.0.0.1:41003 · 窓 24,576"],
      ["裏方", "起こす beta-sub.gguf → :41004 · 窓 10,240"],
    ]);
    await unmount(view, { outro: false });
  });

  /**
   * **種類を切り替えたときの挙動を、種類を選ぶ組の隣で一言で言う**（面ごと）。
   * > 作業単位 A-13b-3 で「起こすにすると、鍵はファイルから消えます」を足し、レビュー是正で外した（鍵は両方の種類の欄）。
   */
  it("保存で書くのは選んだ種類の欄だけだと、種類を選ぶ組の隣で言う", async () => {
    const view = await openEditor("beta");
    const KIND_NOTE =
      "保存で書くのは、選んだ種類の欄だけです。繋ぐにすると、gguf・起動の引数・実行ファイルはファイルから消えます（種類を戻せば、下書きの値は元に戻ります）。";
    for (const legend of [MAIN_LEGEND, SUB_LEGEND]) {
      const group = kindButtons(legend)[0].parentElement as HTMLElement;
      expect(flat(group.nextElementSibling), legend).toBe(KIND_NOTE);
    }
    await unmount(view, { outro: false });
  });

  /**
   * **起こす面で gguf を空のまま保存しない**（D-19 のレビュー / 必須）。
   *
   * 空のまま送ると、サーバは繋ぐ面として書き、一覧に「繋ぐ http://…」と出て、切り替えると誰も待ち受けていない宛先へ繋ぐ。
   * **止める → 書く → 送る** の3相（「常に送らない」実装を第3相で落とす）。
   */
  it("足すときに起こす面の gguf を空のまま保存すると、送らずに gguf の欄にそう言い、書けば送る", async () => {
    onSave = (name, body) => {
      const request = body as { main: WrittenFace; sub: WrittenFace | null };
      const saved: WrittenProfile = {
        name,
        main: { ...request.main, api_key_set: false, api_key_hint: null },
        sub: null,
      };
      return ok(listing({ revision: "rev-2", profiles: [ALPHA, BETA, saved, GAMMA] }));
    };
    const view = await open();
    mustButton(sectionNamed(WRITTEN), "プロファイルを足す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    type(nameField(), "delta");
    type(fieldOf(MAIN_LEGEND, ENDPOINT_LAUNCH), "http://127.0.0.1:18081");
    type(fieldOf(MAIN_LEGEND, "依頼するモデルの綴り（model）"), "delta-main");
    type(fieldOf(MAIN_LEGEND, "起動の引数（args）"), "--ctx-size\n8192");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();

    const culprit = (): Element => fieldOf(MAIN_LEGEND, "gguf のパス（gguf）");
    await vi.waitFor(() => {
      expect(culprit().getAttribute("aria-invalid")).toBe("true");
    });
    expect(culprit().getAttribute("aria-describedby")).toBe("profile-main-gguf-error");
    expect(describedText(culprit())).toBe(
      "保存しませんでした: 起こす面には gguf のパスを書いてください。繋ぐなら「動いているサーバに繋ぐ」を選んでください。",
    );
    expect(sent.filter((s) => s.method === "PUT")).toEqual([]);
    expect(target.querySelector("form")).not.toBeNull();

    type(fieldOf(MAIN_LEGEND, "gguf のパス（gguf）"), "~/models/delta-main.gguf");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    expect(summaryOf("delta")).toEqual([
      ["本文", "起こす delta-main.gguf → :18081"],
      ["裏方", "本文と同じ"],
    ]);
    await unmount(view, { outro: false });
  });

  /**
   * **理由が欄に出ている状態で、その欄を画面から消しても、理由は消えない**（D-19 のレビュー）。
   * 種類を切り替えて欄が無くなったら、下書き全体へ移る。**戻せば欄へ戻る。** 裏方のチェックを外したときも同じ。
   */
  it("欄に結びついた理由は、種類や裏方のチェックでその欄が消えたら下書き全体へ移り、戻せば欄へ戻る", async () => {
    const view = await openEditor("beta");
    const JOINED =
      "保存しませんでした: 「--batch-size 512」は、空白ごと1つの引数として渡るので、上流が起動しません。--batch-size と 512 は別の行に書いてください。";
    type(fieldOf(SUB_LEGEND, "起動の引数（args）"), "--batch-size 512");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(fieldOf(SUB_LEGEND, "起動の引数（args）").getAttribute("aria-invalid")).toBe("true");
    });
    const alerts = (): string[] =>
      [...(target.querySelector("form") as HTMLFormElement).querySelectorAll("[role=alert]")].map((a) => flat(a));
    expect(alerts()).toEqual([]);

    // **種類を繋ぐにする → 引数の欄が消える → 理由は全体へ。**
    kindButtons(SUB_LEGEND)[1].click();
    await vi.waitFor(() => {
      expect(alerts()).toEqual([JOINED]);
    });
    expect([...target.querySelectorAll("form [aria-invalid]")]).toEqual([]);
    // **戻すと欄へ戻る。**
    kindButtons(SUB_LEGEND)[0].click();
    await vi.waitFor(() => {
      expect(fieldOf(SUB_LEGEND, "起動の引数（args）").getAttribute("aria-invalid")).toBe("true");
    });
    expect(alerts()).toEqual([]);

    // **裏方のチェックを外す → 裏方の欄が全部消える → 理由は全体へ。**
    (target.querySelector("form input[type=checkbox]") as HTMLInputElement).click();
    await vi.waitFor(() => {
      expect(alerts()).toEqual([JOINED]);
    });
    await unmount(view, { outro: false });
  });

  /**
   * **理由の付いた欄が、種類を切り替えて「詳細」に畳まれる側へ移ったら、詳細を開いて見せる**（D-19 の再レビュー / 必須）。
   *
   * コンテキスト長はどちらの種類にもあるので、理由は欄に結びついたまま残る。ところが起こす面では、この欄は詳細に畳まれている。
   * **理由が届いたときに限らず**、出している理由が畳んだ欄に結びついていれば詳細を開く。
   * **A（繋ぐ面で欄に理由・詳細は無い）を確かめてから、B（起こす面で詳細が開き、欄に理由）を待つ。**
   */
  it("繋ぐ面のコンテキスト長に理由が出ている状態で起こすへ切り替えると、詳細が開いて欄に理由が出る", async () => {
    const view = await openEditor("gamma");
    const NOT_DIGITS =
      "保存しませんでした: コンテキスト長は、半角の数字だけで書いてください（空にすると書きません）。";
    type(fieldOf(MAIN_LEGEND, WINDOW_LABEL), "32k");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(fieldOf(MAIN_LEGEND, WINDOW_LABEL).getAttribute("aria-invalid")).toBe("true");
    });
    expect(fieldOf(MAIN_LEGEND, WINDOW_LABEL).closest("details")).toBeNull();
    expect(sent.filter((s) => s.method === "PUT")).toEqual([]);

    kindButtons(MAIN_LEGEND)[0].click();
    await vi.waitFor(() => {
      expect(pressedOf(MAIN_LEGEND)).toEqual(["true", "false"]);
    });
    const field = fieldOf(MAIN_LEGEND, WINDOW_LABEL);
    expect((field.closest("details") as HTMLDetailsElement).open).toBe(true);
    expect(field.getAttribute("aria-invalid")).toBe("true");
    expect(describedText(field)).toBe(NOT_DIGITS);
    await unmount(view, { outro: false });
  });

  /**
   * **送る前の止めの順番を固定する**: 名前 → 起こす面の gguf → 旗と値 → コンテキスト長（D-19 の再レビュー）。
   * 全部が当てはまる下書きから1つずつ直していき、毎回その時点で先頭の理由だけが出ること、最後に送ることを見る。
   */
  it("送る前の止めは、名前 → gguf → 旗と値 → コンテキスト長の順に1つずつ出て、全部直せば送る", async () => {
    onSave = () => ok(listing({ revision: "rev-2" }));
    const view = await open();
    mustButton(sectionNamed(WRITTEN), "プロファイルを足す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    const form = (): HTMLFormElement => target.querySelector("form") as HTMLFormElement;
    const GGUF = "gguf のパス（gguf）";
    const ARGS = "起動の引数（args）";
    /** いま無効になっている欄（名前・gguf・引数・コンテキスト長の順に、`aria-invalid` があるか）。 */
    const invalid = (): boolean[] =>
      [nameField(), fieldOf(MAIN_LEGEND, GGUF), fieldOf(MAIN_LEGEND, ARGS), fieldOf(MAIN_LEGEND, WINDOW_LABEL)].map(
        (control) => control.getAttribute("aria-invalid") === "true",
      );
    const saveAndWait = async (expected: boolean[]): Promise<void> => {
      mustButton(form(), "保存する").click();
      await vi.waitFor(() => {
        expect(invalid()).toEqual(expected);
      });
    };

    type(nameField(), "beta");
    type(fieldOf(MAIN_LEGEND, ENDPOINT_LAUNCH), "http://127.0.0.1:18081");
    type(fieldOf(MAIN_LEGEND, ARGS), "--ctx-size 16384");
    type(fieldOf(MAIN_LEGEND, WINDOW_LABEL), "32k");
    await saveAndWait([true, false, false, false]);
    type(nameField(), "delta");
    await saveAndWait([false, true, false, false]);
    type(fieldOf(MAIN_LEGEND, GGUF), "~/models/delta.gguf");
    await saveAndWait([false, false, true, false]);
    type(fieldOf(MAIN_LEGEND, ARGS), "--ctx-size\n16384");
    await saveAndWait([false, false, false, true]);
    expect(sent.filter((s) => s.method === "PUT")).toEqual([]);
    type(fieldOf(MAIN_LEGEND, WINDOW_LABEL), "32768");
    mustButton(form(), "保存する").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.method === "PUT")).toHaveLength(1);
    });
    await unmount(view, { outro: false });
  });

  /** **保存を待つ間は、種類も切り替えられない**（送った種類と画面の種類を食い違わせない）。 */
  it("保存の応答を待つ間は、種類を選ぶボタンを押せない", async () => {
    const pending = deferred();
    onSave = () => pending.reply;
    const view = await openEditor("beta");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(kindButtons(MAIN_LEGEND).map((button) => button.disabled)).toEqual([true, true]);
    });
    pending.resolve(listing());
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    await unmount(view, { outro: false });
  });
});

describe("保存を断られたとき", () => {
  async function openBetaEditor(): Promise<Record<string, unknown>> {
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    return view;
  }

  const LOOPBACK =
    "gguf を書いた面の endpoint はループバック（127.0.0.1 / localhost / ::1）だけです";

  /**
   * **理由は、止まった面の止まった鍵の欄に結びつく。**
   * 結びつく先は `aria-describedby` の値で引き、`aria-invalid` は値まで見る。
   * **他の欄は無効にならない**（取り違えると、直す場所を間違える）。
   */
  it("422 は、止まった面と鍵の欄に、理由を結びつける", async () => {
    onSave = () =>
      apiError(422, "upstream_profile_invalid", "上流プロファイルの形が不正なので保存しませんでした", {
        reason: "endpoint_not_loopback",
        reason_message: LOOPBACK,
        face: "sub",
        key: "endpoint",
      });
    const view = await openBetaEditor();
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();

    const culprit = (): Element => fieldOf(SUB_LEGEND, ENDPOINT_LAUNCH);
    await vi.waitFor(() => {
      expect(culprit().getAttribute("aria-invalid")).toBe("true");
    });
    // **endpoint の欄は、理由の後に案内も結びつく**（作業単位 B-68 —— 案内は理由の有無に依らず欄の説明）。
    const ENDPOINT_NOTES =
      "起こした llama がここで待ち受けます。この機の中（127.0.0.1 / localhost / ::1）だけです。 ホストとポートまで書きます（例 http://127.0.0.1:8080）。/v1 は personacore が足すので書きません。";
    expect(describedText(culprit())).toBe(`保存しませんでした: ${LOOPBACK} ${ENDPOINT_NOTES}`);
    // **他の11の欄は無効にならない**（本文の同じ鍵も含む）。
    const others = [
      ...KEY_LABELS.map((label) => fieldOf(MAIN_LEGEND, label)),
      ...KEY_LABELS.filter((label) => label !== ENDPOINT_LAUNCH).map((label) => fieldOf(SUB_LEGEND, label)),
    ];
    expect(others.map((field) => field.getAttribute("aria-invalid"))).toEqual(
      Array.from({ length: 11 }, () => null),
    );
    // **理由はどこにも結びつかない**（本文の endpoint の欄は案内だけ。`KEY_LABELS` の先頭が endpoint）。
    expect(others.map((field) => describedText(field))).toEqual([
      ENDPOINT_NOTES,
      ...Array.from({ length: 10 }, () => ""),
    ]);
    // **下書きは閉じない。一覧は変わらない。**
    expect(target.querySelector("form")).not.toBeNull();
    expect(summaryOf("beta")[1]).toEqual(["裏方", "起こす beta-sub.gguf → :41004 · 窓 10,240"]);
    await unmount(view, { outro: false });
  });

  /**
   * **引数の欄（`<textarea>`）にも、理由が結びつく**（レビュー1周目 R07 / R08）。
   *
   * `args` に `--host` / `--port` を書くと `reserved_serve_flag`（key は `args`）で止まる
   * ——**画面から起こせる理由**なので、1行の欄とは別の要素のこの欄でも値まで見る。
   */
  it("422 の `args` は、その面の引数の欄に理由を結びつける", async () => {
    const reserved = "args に --host / --port は書けません（endpoint から作ります）";
    onSave = () =>
      apiError(422, "upstream_profile_invalid", "上流プロファイルの形が不正なので保存しませんでした", {
        reason: "reserved_serve_flag",
        reason_message: reserved,
        face: "sub",
        key: "args",
      });
    const view = await openBetaEditor();
    type(fieldOf(SUB_LEGEND, "起動の引数（args）"), "--port\n9999");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();

    const culprit = (): Element => fieldOf(SUB_LEGEND, "起動の引数（args）");
    await vi.waitFor(() => {
      expect(culprit().getAttribute("aria-invalid")).toBe("true");
    });
    expect(culprit().tagName).toBe("TEXTAREA");
    expect(culprit().getAttribute("aria-describedby")).toBe("profile-sub-args-error");
    expect(describedText(culprit())).toBe(`保存しませんでした: ${reserved}`);
    // **本文の引数の欄は無効にならない**（同じ鍵の、もう片方の面）。
    const mainArgs = fieldOf(MAIN_LEGEND, "起動の引数（args）");
    expect([mainArgs.getAttribute("aria-invalid"), mainArgs.getAttribute("aria-describedby")]).toEqual([
      null,
      null,
    ]);
    await unmount(view, { outro: false });
  });

  /**
   * **コンテキスト長の 422 は、その面のコンテキスト長の欄に結びつく**（作業単位 A-13a-2）。
   *
   * 範囲（応答の余白より大きいか）は画面では見ず、サーバが `context_tokens_unusable` /
   * 鍵 `context_tokens` で返す。**文面はサーバの定数**（`ShapeReason::ContextTokensUnusable`）。
   */
  it("422 の `context_tokens` は、その面のコンテキスト長の欄に理由を結びつける", async () => {
    const unusable = "context_tokens が使えません（応答の余白より大きい整数のトークン数で書いてください）";
    onSave = () =>
      apiError(422, "upstream_profile_invalid", "上流プロファイルの形が不正なので保存しませんでした", {
        reason: "context_tokens_unusable",
        reason_message: unusable,
        face: "main",
        key: "context_tokens",
      });
    const view = await openBetaEditor();
    // **`beta` の本文に書いた返答の上限（2000）は空にする**（作業単位 D-21 で `beta` に足した値。残すと、窓 1024 は
    // 「上限が窓以上」として画面が送る前に止め、ここで見たいサーバの 422 に届かない）。
    type(fieldOf(MAIN_LEGEND, "返答の上限（response_tokens）"), "");
    type(fieldOf(MAIN_LEGEND, WINDOW_LABEL), "1024");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();

    const culprit = (): Element => fieldOf(MAIN_LEGEND, WINDOW_LABEL);
    await vi.waitFor(() => {
      expect(culprit().getAttribute("aria-invalid")).toBe("true");
    });
    expect(describedText(culprit())).toBe(`保存しませんでした: ${unusable}`);
    // **範囲外でも整数なら送っている**（決めるのはサーバ）。
    expect(
      (sent.filter((s) => s.method === "PUT")[0]?.body as { main: WrittenFace }).main.context_tokens,
    ).toBe(1024);
    expect(fieldOf(SUB_LEGEND, WINDOW_LABEL).getAttribute("aria-invalid")).toBeNull();
    await unmount(view, { outro: false });
  });

  /**
   * **詳細に畳んだ欄を名指した理由は、詳細を開いて見せる**（作業単位 D-19）。
   *
   * 起こす面の起動の引数・実行ファイル・コンテキスト長は「詳細」に畳んである。**閉じたままだと、
   * 欄に結びつけた理由が目に見えない。** **A（閉じている）を確かめてから B（開く）を待つ。**
   * **名指されなかった面の詳細は開かない**（面の取り違えを見る）。
   */
  it("422 が詳細に畳んだ欄を名指したら、その面の詳細を開いて見せる", async () => {
    onSave = () =>
      apiError(422, "upstream_profile_invalid", "上流プロファイルの形が不正なので保存しませんでした", {
        reason: "reserved_serve_flag",
        reason_message: "args に --host / --port は書けません（endpoint から作ります）",
        face: "sub",
        key: "args",
      });
    const view = await openBetaEditor();
    const details = (legend: string): HTMLDetailsElement =>
      fieldOf(legend, "起動の引数（args）").closest("details") as HTMLDetailsElement;
    expect([details(MAIN_LEGEND).open, details(SUB_LEGEND).open]).toEqual([false, false]);
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(fieldOf(SUB_LEGEND, "起動の引数（args）").getAttribute("aria-invalid")).toBe("true");
    });
    expect([details(MAIN_LEGEND).open, details(SUB_LEGEND).open]).toEqual([false, true]);
    await unmount(view, { outro: false });
  });

  /**
   * **その面の種類に無い欄を名指した理由は、下書き全体に出す**（作業単位 D-19 / `keepOnShownField`）。
   * 繋ぐ面（`gamma` の本文）には gguf の欄が無い。**出す先の無い欄に結びつけて、理由を消さない。**
   */
  it("繋ぐ面に無い欄を名指した 422 は、下書き全体に理由を出す", async () => {
    onSave = () =>
      apiError(422, "upstream_profile_invalid", "上流プロファイルの形が不正なので保存しませんでした", {
        reason: "gguf_unusable",
        reason_message: "gguf が使えません",
        face: "main",
        key: "gguf",
      });
    const view = await open();
    mustButton(profileBlock("gamma"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    expect(pressedOf(MAIN_LEGEND)).toEqual(["false", "true"]);
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form [role=alert]")).not.toBeNull();
    });
    expect([...(target.querySelector("form") as HTMLFormElement).querySelectorAll("[role=alert]")].map((a) => flat(a))).toEqual([
      "保存しませんでした: gguf が使えません",
    ]);
    expect([...target.querySelectorAll("form [aria-invalid]")]).toEqual([]);
    // 対照: 起こすに切り替えると gguf の欄が出る（その欄には、送った後の理由は残っていない）。
    kindButtons(MAIN_LEGEND)[0].click();
    await vi.waitFor(() => {
      expect(labelsOf(MAIN_LEGEND)).toContain("gguf のパス（gguf）");
    });
    expect(fieldOf(MAIN_LEGEND, "gguf のパス（gguf）").getAttribute("aria-invalid")).toBeNull();
    await unmount(view, { outro: false });
  });

  it("面か鍵が分からない 422 は、下書き全体に理由を出す", async () => {
    onSave = () =>
      apiError(422, "upstream_profile_invalid", "上流プロファイルの形が不正なので保存しませんでした", {
        reason: "same_port_other_gguf",
        reason_message: "同じポートに別の gguf は載せられません",
        face: null,
        key: null,
      });
    const view = await openBetaEditor();
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form [role=alert]")).not.toBeNull();
    });
    expect([...(target.querySelector("form") as HTMLFormElement).querySelectorAll("[role=alert]")].map((a) => flat(a))).toEqual([
      "保存しませんでした: 同じポートに別の gguf は載せられません",
    ]);
    expect([...target.querySelectorAll("form [aria-invalid]")]).toEqual([]);
    await unmount(view, { outro: false });
  });

  /**
   * **409 `upstream_profiles_changed`: 黙って上書きしない。**
   * 読み直す操作は、**一覧を読み直して下書きを閉じる**（外で書き換えられた内容を先に見せる）。
   * **2度目の `PUT` は、利用者が開き直すまで飛ばない。**
   */
  it("409 は、外で書き換えられたことを言い、読み直すと新しい一覧を出す", async () => {
    onSave = () =>
      apiError(
        409,
        "upstream_profiles_changed",
        "上流プロファイルは読んだ後に書き換えられています。読み直してから直してください",
      );
    const view = await openBetaEditor();
    type(fieldOf(MAIN_LEGEND, "依頼するモデルの綴り（model）"), "mine");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();

    await vi.waitFor(() => {
      expect(target.querySelector("form [role=alert]")).not.toBeNull();
    });
    const alert = target.querySelector("form [role=alert]");
    expect(flat(alert)).toContain(
      "upstreams.json は、この画面が読んだ後に外で書き換えられています。上書きはしていません。読み直してから、もう一度直してください。",
    );
    // **外で書き換えられた一覧**（`beta` の本文が別人の手で、繋ぐ面に変わっている）。
    listingReplies = [
      ok(
        listing({
          revision: "rev-9",
          profiles: [
            ALPHA,
            { ...BETA, main: { ...BETA.main, gguf: null, args: null, command: null, endpoint: "http://127.0.0.1:46001" } },
            GAMMA,
          ],
        }),
      ),
    ];
    mustButton(alert as Element, "読み直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    expect(summaryOf("beta")).toEqual([
      ["本文", "繋ぐ http://127.0.0.1:46001"],
      ["裏方", "起こす beta-sub.gguf → :41004 · 窓 10,240"],
    ]);
    expect(sent.filter((s) => s.method === "PUT")).toHaveLength(1);
    expect(sent.filter((s) => s.url === "/admin/upstream/profiles" && s.method === "GET")).toHaveLength(2);
    await unmount(view, { outro: false });
  });

  it("名前が使えない 400 は、名前の欄に結びつける", async () => {
    const message = "その名前は上流プロファイルの名前に使えません（空・前後の空白・-- で始まる名前は使えません）";
    onSave = () => apiError(400, "upstream_profile_name_invalid", message);
    const view = await open();
    mustButton(sectionNamed(WRITTEN), "プロファイルを足す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    type(nameField(), "--x");
    // **起こす面は gguf が要る**（空なら送る前に止まり、名前の 400 まで届かない / D-19 のレビュー）。
    type(fieldOf(MAIN_LEGEND, "gguf のパス（gguf）"), "~/x.gguf");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(nameField().getAttribute("aria-invalid")).toBe("true");
    });
    expect(describedText(nameField())).toBe(message);
    expect(sent.filter((s) => s.method === "PUT").map((s) => s.url)).toEqual(["/admin/upstream/profiles/--x"]);
    await unmount(view, { outro: false });
  });
});

/**
 * **宛先に何を書くかを言い、末尾が `/v1` の宛先は送らずに止める**（作業単位 B-68 / Issue origin#97）。
 *
 * 利用者が実機で踏んだ形: 中継の宛先を `…/v1` まで書くと、personacore が `/v1/chat/completions` を足して
 * `…/v1/v1/chat/completions` に送り、中継が 401 を返して鍵の間違いに見えた。
 *
 * * **案内は両方の面の endpoint の欄**（起こす面「待ち受ける場所」・繋ぐ面「宛先」）。欄と `aria-describedby` で結ぶ。
 * * **止める → 直す → 送る** の3相（「常に送らない」実装を第3相で落とす）。止めるのは悪い面の欄だけ
 *   （本文は正しく、裏方だけが悪い —— 端の面を選ばない）。
 * * **止めない側の対照**（`/v1beta`）は1回で送る。
 */
/**
 * **宛先に書いた秘密は、一覧の要点に出ず、「直す」の欄には書いたまま出る**（作業単位 B-71）。
 *
 * 一覧の応答（`GET /admin/upstream/profiles`）は書いたままの値を返し続ける（編集欄は書いた値を見せないと直せない）。
 * **伏せるのは画面の要点だけ。** 3件にして真ん中に秘密を入れる（端に置くと「最初の1件だけ伏せる」実装が通る）。
 */
describe("宛先に書いた秘密", () => {
  const LEAKY_MAIN = "http://user:pw-secret@127.0.0.1:47941/v1?token=q-secret#f-secret";
  const LEAKY_SUB = "http://sub-user:pw2-secret@127.0.0.1:47942?token=q2-secret#f2-secret";
  const SECRETS = ["pw-secret", "q-secret", "f-secret", "sub-user", "pw2-secret", "q2-secret", "f2-secret"];
  const LEAKY: WrittenProfile = {
    name: "beta",
    main: { ...(GAMMA.main as WrittenFace), endpoint: LEAKY_MAIN, model: "leaky-main" },
    sub: { ...(GAMMA.main as WrittenFace), endpoint: LEAKY_SUB, model: "leaky-sub" },
  };

  it("一覧の要点は秘密を落としてホストとポートを言い、直すと欄に書いたままの宛先が出る", async () => {
    // **読み直されても同じ一覧を返す**（読み直しで既定の一覧に戻ると、「直す」で別の値が開く）。
    listingReplies = Array.from({ length: 100 }, () => ok(listing({ profiles: [ALPHA, LEAKY, GAMMA] })));
    const view = await open();

    // 肯定側: 要点はホスト・ポート・パスを言う。
    expect(summaryOf("beta")).toEqual([
      ["本文", "繋ぐ http://127.0.0.1:47941/v1"],
      ["裏方", "繋ぐ http://127.0.0.1:47942/"],
    ]);
    // 本題: 書いた節のどこにも秘密が無い。
    const written = flat(sectionNamed(WRITTEN));
    for (const secret of SECRETS) {
      expect(written, secret).not.toContain(secret);
    }

    // 対照: 「直す」の欄には書いたままの宛先が出る（伏せた値で開くと、保存で秘密が消える）。
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    expect([fieldOf(MAIN_LEGEND, ENDPOINT_CONNECT).value, fieldOf(SUB_LEGEND, ENDPOINT_CONNECT).value]).toEqual([
      LEAKY_MAIN,
      LEAKY_SUB,
    ]);
    await unmount(view, { outro: false });
  });
});

describe("宛先の案内と、末尾の /v1", () => {
  const LISTEN_NOTE = "起こした llama がここで待ち受けます。この機の中（127.0.0.1 / localhost / ::1）だけです。";
  const LAUNCH_GUIDE = "ホストとポートまで書きます（例 http://127.0.0.1:8080）。/v1 は personacore が足すので書きません。";
  const CONNECT_GUIDE =
    "ホストとポートまで書きます（例 http://127.0.0.1:8080）。/v1/chat/completions などは personacore が足すので、中継の説明書きが …/v1 までを示していても、/v1 は書きません。パスの前置き（https://example.com/api など）は書けます。";
  const V1 =
    "保存しませんでした: 末尾の /v1 は書かないでください（personacore が /v1 を足すので、/v1/v1/… に送ることになります）。ホストとポートまで書いてください（例 http://127.0.0.1:8080）。";

  async function openBetaEditor(): Promise<Record<string, unknown>> {
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    return view;
  }

  /** 裏方の面を繋ぐにする（本文は起こすのまま —— 両方の種類の欄を1つの下書きで見る）。 */
  async function subToConnect(): Promise<void> {
    kindButtons(SUB_LEGEND)[1].click();
    await vi.waitFor(() => {
      expect(pressedOf(SUB_LEGEND)).toEqual(["false", "true"]);
    });
  }

  it("両方の面の endpoint の欄が、ホストとポートまで書くことを言い、欄と結びつく", async () => {
    const view = await openBetaEditor();
    await subToConnect();
    const launch = fieldOf(MAIN_LEGEND, ENDPOINT_LAUNCH);
    const connect = fieldOf(SUB_LEGEND, ENDPOINT_CONNECT);
    expect([notesNear(launch), notesNear(connect)]).toEqual([[LISTEN_NOTE, LAUNCH_GUIDE], [CONNECT_GUIDE]]);
    // **案内は欄の説明として読み上げられる**（`aria-describedby` が指す要素の文面）。無効にはしない。
    expect([describedText(launch), describedText(connect)]).toEqual([
      `${LISTEN_NOTE} ${LAUNCH_GUIDE}`,
      CONNECT_GUIDE,
    ]);
    expect([launch.getAttribute("aria-invalid"), connect.getAttribute("aria-invalid")]).toEqual([null, null]);
    // **案内は endpoint の欄だけ**（モデルの欄には結びつけない）。
    expect(fieldOf(SUB_LEGEND, "依頼するモデルの綴り（model）").getAttribute("aria-describedby")).toBeNull();
    await unmount(view, { outro: false });
  });

  it("末尾が /v1 の宛先は送らずに、その面の endpoint の欄にそう言い、直せば送る", async () => {
    const view = await openBetaEditor();
    await subToConnect();
    type(fieldOf(SUB_LEGEND, ENDPOINT_CONNECT), "http://127.0.0.1:46001/V1/");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();

    const culprit = (): Element => fieldOf(SUB_LEGEND, ENDPOINT_CONNECT);
    await vi.waitFor(() => {
      expect(culprit().getAttribute("aria-invalid")).toBe("true");
    });
    expect(culprit().getAttribute("aria-describedby")?.split(" ")[0]).toBe("profile-sub-endpoint-error");
    expect(describedText(culprit())).toBe(`${V1} ${CONNECT_GUIDE}`);
    expect(sent.filter((s) => s.method === "PUT")).toEqual([]);
    // **本文の同じ欄は無効にならない**（悪いのは裏方だけ）。
    const mainEndpoint = fieldOf(MAIN_LEGEND, ENDPOINT_LAUNCH);
    expect([mainEndpoint.getAttribute("aria-invalid"), describedText(mainEndpoint)]).toEqual([
      null,
      `${LISTEN_NOTE} ${LAUNCH_GUIDE}`,
    ]);
    // **黙って直さない**（欄の値は打ったまま）。
    expect((culprit() as HTMLInputElement).value).toBe("http://127.0.0.1:46001/V1/");

    type(fieldOf(SUB_LEGEND, ENDPOINT_CONNECT), "http://127.0.0.1:46001");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.method === "PUT")).toHaveLength(1);
    });
    expect(
      (sent.filter((s) => s.method === "PUT")[0]?.body as { sub: WrittenFace }).sub.endpoint,
    ).toBe("http://127.0.0.1:46001");
    await unmount(view, { outro: false });
  });

  it("対照: /v1beta で終わる宛先は止めずに送る", async () => {
    const view = await openBetaEditor();
    await subToConnect();
    type(fieldOf(SUB_LEGEND, ENDPOINT_CONNECT), "https://example.com/v1beta");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.method === "PUT")).toHaveLength(1);
    });
    expect(
      (sent.filter((s) => s.method === "PUT")[0]?.body as { sub: WrittenFace }).sub.endpoint,
    ).toBe("https://example.com/v1beta");
    await unmount(view, { outro: false });
  });
});

/**
 * **旗と値を1行に書いた引数は、保存の時点で言う**（作業単位 D-13 / Issue origin#29）。
 *
 * 利用者が実機で踏んだ形: 引数の欄に `--ctx-size 16384` を1行で書くと、
 * 空白を含んだ1つの引数として `llama` に渡り、起動直後に終了する。
 * **保存の時点では何も言わず、切り替えた後に「起動直後に終了しました」とだけ出ていた。**
 *
 * **送らずに止める**（在る名前を打ったときと同じ流れ）。どちらの面か・どの欄かまで結びつける。
 */
describe("旗と値を1行に書いた引数", () => {
  const JOINED =
    "保存しませんでした: 「--ctx-size 16384」は、空白ごと1つの引数として渡るので、上流が起動しません。--ctx-size と 16384 は別の行に書いてください。";

  async function openBetaEditor(): Promise<Record<string, unknown>> {
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    return view;
  }

  /** **止める → 直す → 送る** の3相（「常に送らない」実装を第3相で落とす）。 */
  it("送らずに、その面の引数の欄にそう言い、行を分ければ送る", async () => {
    const view = await openBetaEditor();
    type(fieldOf(MAIN_LEGEND, "起動の引数（args）"), "--threads\n6\n--ctx-size 16384\n--jinja");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();

    const culprit = (): Element => fieldOf(MAIN_LEGEND, "起動の引数（args）");
    await vi.waitFor(() => {
      expect(culprit().getAttribute("aria-invalid")).toBe("true");
    });
    expect(culprit().getAttribute("aria-describedby")).toBe("profile-main-args-error");
    expect(describedText(culprit())).toBe(JOINED);
    // **送っていない。下書きは開いたまま。**
    expect(sent.filter((s) => s.method === "PUT")).toEqual([]);
    expect(target.querySelector("form")).not.toBeNull();
    // **他の11の欄は無効にならない**（裏方の同じ鍵も含む）。
    const others = [
      ...KEY_LABELS.filter((label) => label !== "起動の引数（args）").map((label) => fieldOf(MAIN_LEGEND, label)),
      ...KEY_LABELS.map((label) => fieldOf(SUB_LEGEND, label)),
    ];
    expect(others.map((field) => field.getAttribute("aria-invalid"))).toEqual(
      Array.from({ length: 11 }, () => null),
    );

    // **行を分けて保存し直すと、送る。**
    type(fieldOf(MAIN_LEGEND, "起動の引数（args）"), "--threads\n6\n--ctx-size\n16384\n--jinja");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.method === "PUT")).toHaveLength(1);
    });
    expect((sent.filter((s) => s.method === "PUT")[0]?.body as { main: WrittenFace }).main.args).toEqual([
      "--threads",
      "6",
      "--ctx-size",
      "16384",
      "--jinja",
    ]);
    await unmount(view, { outro: false });
  });

  it("裏方の面に書いたら、裏方の引数の欄にそう言う", async () => {
    const view = await openBetaEditor();
    type(fieldOf(SUB_LEGEND, "起動の引数（args）"), "--ctx-size 16384");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();

    const culprit = (): Element => fieldOf(SUB_LEGEND, "起動の引数（args）");
    await vi.waitFor(() => {
      expect(culprit().getAttribute("aria-invalid")).toBe("true");
    });
    expect(culprit().getAttribute("aria-describedby")).toBe("profile-sub-args-error");
    expect(describedText(culprit())).toBe(JOINED);
    const mainArgs = fieldOf(MAIN_LEGEND, "起動の引数（args）");
    expect([mainArgs.getAttribute("aria-invalid"), mainArgs.getAttribute("aria-describedby")]).toEqual([
      null,
      null,
    ]);
    expect(sent.filter((s) => s.method === "PUT")).toEqual([]);
    await unmount(view, { outro: false });
  });

  /** **空白を含む値（パス）は正当**なので止めない。 */
  it("空白を含むパスの行は止めずに、そのまま送る", async () => {
    const view = await openBetaEditor();
    type(fieldOf(MAIN_LEGEND, "起動の引数（args）"), "--chat-template-file\nC:\\Program Files\\llama\\t.jinja");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.method === "PUT")).toHaveLength(1);
    });
    expect((sent.filter((s) => s.method === "PUT")[0]?.body as { main: WrittenFace }).main.args).toEqual([
      "--chat-template-file",
      "C:\\Program Files\\llama\\t.jinja",
    ]);
    await unmount(view, { outro: false });
  });

  /**
   * **欄の説明が、入力する前に例を言う。**
   * 注記の文面と、引数の欄の見本（`placeholder`）を値まで見る（面ごとに）。
   */
  /**
   * **欄の説明が、入力する前に例を言う。**
   * **注記は、それに関係する欄の隣にだけ置く**（作業単位 D-19）—— 欄と同じ囲みの中の `p.note` を、面ごとに値まで見る。
   */
  it("入力する前に、旗と値を別の行に書く例を、引数の欄の隣で言う", async () => {
    const view = await openBetaEditor();
    const form = target.querySelector("form") as HTMLFormElement;
    const ARGS_NOTE =
      "起動の引数は1行に1つ書きます。オプションとその値も別の行です（--ctx-size と 16384、-ngl と 99 は別の行。1行に続けて書くと、空白ごと1つの引数として渡り、上流が起動しません）。オプションの行の先頭に空白を入れないでください。空白を含むパスは、そのまま1行に書けます。空にすると書きません。";
    expect([
      notesNear(fieldOf(MAIN_LEGEND, "起動の引数（args）")),
      notesNear(fieldOf(SUB_LEGEND, "起動の引数（args）")),
    ]).toEqual([[ARGS_NOTE], [ARGS_NOTE]]);
    expect([
      fieldOf(MAIN_LEGEND, "起動の引数（args）").getAttribute("placeholder"),
      fieldOf(SUB_LEGEND, "起動の引数（args）").getAttribute("placeholder"),
    ]).toEqual(["--ctx-size\n16384\n--jinja", "--ctx-size\n16384\n--jinja"]);
    // **見本は引数の欄だけ**（他の欄に同じ見本を付けない）。
    expect(fieldOf(MAIN_LEGEND, ENDPOINT_LAUNCH).getAttribute("placeholder")).toBeNull();
    expect(fieldOf(MAIN_LEGEND, WINDOW_LABEL).getAttribute("placeholder")).toBeNull();
    // **コンテキスト長の欄がいつ要るか・空にすると何が起きるか**（作業単位 A-13a-2）。
    // **本文の面だけが、裏方を別にしないときの使われ方を言う。**
    const WINDOW_NOTE =
      "コンテキスト長は、上流がコンテキスト長を答えないとき（鍵で守られた中継など）だけ、上流で選んだ窓のトークン数を書きます。空にすると書かず、上流に聞きます。";
    expect([notesNear(fieldOf(MAIN_LEGEND, WINDOW_LABEL)), notesNear(fieldOf(SUB_LEGEND, WINDOW_LABEL))]).toEqual([
      [`${WINDOW_NOTE}裏方を別の上流にしなければ、本文の値が抽出にも使われます。`],
      [WINDOW_NOTE],
    ]);
    // **実行ファイルを書かないときに何を起こすか**と、**待ち受ける場所が何か**も、その欄の隣で言う。
    expect(notesNear(fieldOf(MAIN_LEGEND, "実行ファイル（command）"))).toEqual(["空にすると書かず、llama を起こします。"]);
    // **宛先に何を書くか**（作業単位 B-68 / Issue origin#97）も同じ欄の隣で言う。
    expect(notesNear(fieldOf(MAIN_LEGEND, ENDPOINT_LAUNCH))).toEqual([
      "起こした llama がここで待ち受けます。この機の中（127.0.0.1 / localhost / ::1）だけです。",
      "ホストとポートまで書きます（例 http://127.0.0.1:8080）。/v1 は personacore が足すので書きません。",
    ]);
    // **欄の注記を、欄から離れた下書きの末尾に並べない**（着手前の形）。
    const loose = [...form.querySelectorAll(":scope > p.note")].map((note) => flat(note));
    expect(loose.filter((note) => note.includes("起動の引数") || note.includes("コンテキスト長"))).toEqual([]);
    await unmount(view, { outro: false });
  });
});

/**
 * **思考・返答の上限・繰り返しの罰**（作業単位 D-21 / ADR-0060 §決定3 / §決定6 の腕7）。
 *
 * 値は真ん中の `beta` にだけ書いてある（本文 1.05・`low`・2000 / 裏方 1.25）。**文面は直書き**（定数から作らない）。
 */
describe("思考・返答の上限・繰り返しの罰（作業単位 D-21）", () => {
  const THINKING_GROUP = "思考（reasoning_effort）";
  const ROOM_LABEL = "返答の上限（response_tokens）";
  const PENALTY_LABEL = "繰り返しの罰（repeat_penalty）";
  const SAMPLING_HEADING = "サンプリング（sampling）";

  async function openEditor(name: string): Promise<Record<string, unknown>> {
    const view = await open();
    mustButton(profileBlock(name), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    return view;
  }

  /** **その面のサンプリングの組**（見出しで名乗る `role=group`）。無ければ `null`。 */
  function samplingGroup(legend: string): HTMLElement | null {
    const groups = [...fieldsetOf(legend).querySelectorAll<HTMLElement>("[role=group][aria-labelledby]")].filter(
      (group) => flat(document.getElementById(group.getAttribute("aria-labelledby") ?? "")) === SAMPLING_HEADING,
    );
    expect(groups.length, `面「${legend}」のサンプリングの組は1つまで`).toBeLessThanOrEqual(1);
    return groups[0] ?? null;
  }

  /** 本文の面の思考の2択（`[切る, 入れる]`）。 */
  function thinkingButtons(): [HTMLButtonElement, HTMLButtonElement] {
    const group = fieldsetOf(MAIN_LEGEND).querySelector(`[role=group][aria-label="${THINKING_GROUP}"]`);
    expect(group, "本文の面に思考の2択があること").not.toBeNull();
    return [mustButton(group as Element, "切る（既定）"), mustButton(group as Element, "入れる（low）")];
  }

  function lastPut(): { main: Record<string, unknown>; sub: Record<string, unknown> | null } {
    const puts = sent.filter((s) => s.method === "PUT");
    return puts.at(-1)?.body as { main: Record<string, unknown>; sub: Record<string, unknown> | null };
  }

  /**
   * **一覧: 書いた値を見せ、書いていなければ「書いていない」と言って既定の印を添える**（ADR-0060 §決定3）。
   * **本文の面は3つ、裏方の面は繰り返しの罰だけ。** 既定の印は書いた値の文面と別の要素。
   */
  it("一覧: 面ごとに書いた値を出し、書いていなければ既定の印を別に添える", async () => {
    const view = await open();
    expect(sentOf("beta")).toEqual([
      [
        "本文",
        [
          ["思考 入れる（low）", null],
          ["返答の上限 2,000", null],
          ["繰り返しの罰 1.05", null],
        ],
      ],
      ["裏方", [["繰り返しの罰 1.25", null]]],
    ]);
    expect(sentOf("alpha")).toEqual([
      [
        "本文",
        [
          ["思考 書いていない", "既定 切る"],
          ["返答の上限 書いていない", "既定 1,024"],
          ["繰り返しの罰 書いていない", "既定 1.1"],
        ],
      ],
      ["裏方", [["繰り返しの罰 書いていない", "既定 1.1"]]],
    ]);
    // **裏方を書いていなければ、何も添えない**（本文と同じ）。
    expect(sentOf("gamma")[1]).toEqual(["裏方", []]);
    await unmount(view, { outro: false });
  });

  /**
   * **下書き: 本文の面は思考（すぐ出す）・返答の上限（すぐ出す）・繰り返しの罰（詳細）。裏方の面は繰り返しの罰（詳細）だけ**
   * （ADR-0060 §決定3 の表）。書いた値がその面の欄に入る。
   */
  it("直す: 書いた値がその面の欄に入り、思考と返答の上限は本文の面だけですぐ出し、繰り返しの罰は詳細に畳む", async () => {
    const view = await openEditor("beta");
    expect(thinkingButtons().map((button) => button.getAttribute("aria-pressed"))).toEqual(["false", "true"]);
    expect(thinkingButtons()[0].closest("details")).toBeNull();
    expect(fieldOf(MAIN_LEGEND, ROOM_LABEL).value).toBe("2000");
    expect(fieldOf(MAIN_LEGEND, ROOM_LABEL).closest("details")).toBeNull();
    expect(fieldOf(MAIN_LEGEND, PENALTY_LABEL).value).toBe("1.05");
    expect(fieldOf(MAIN_LEGEND, PENALTY_LABEL).closest("details")).not.toBeNull();
    expect(samplingGroup(MAIN_LEGEND)?.closest("details")).toBeNull();

    // **裏方の面は、思考も返答の上限も出さない。**
    expect(labelsOf(SUB_LEGEND)).not.toContain(ROOM_LABEL);
    expect(fieldsetOf(SUB_LEGEND).querySelector(`[aria-label="${THINKING_GROUP}"]`)).toBeNull();
    expect(fieldOf(SUB_LEGEND, PENALTY_LABEL).value).toBe("1.25");
    expect(fieldOf(SUB_LEGEND, PENALTY_LABEL).closest("details")).not.toBeNull();
    expect(samplingGroup(SUB_LEGEND)?.closest("details")).not.toBeNull();
    // **裏方の組も「保存しただけでは効かず、切り替え直すと効く」と言う**（D-21 のレビュー F2）。思考の注記は本文の面だけ。
    const subNotes = [...(samplingGroup(SUB_LEGEND) as HTMLElement).querySelectorAll("p.note")].map((note) => flat(note));
    expect(subNotes.join(" ")).toContain("同じ名前へ切り替え直すと効きます");
    expect(subNotes.join(" ")).not.toContain("思考させるか");
    await unmount(view, { outro: false });
  });

  /**
   * **保存では2つの欄を必ず送る**（空の欄は `null`）。**思考を「切る」にすると `reasoning_effort` だけが落ち、繰り返しの罰が残る。**
   * **裏方の依頼には `response_tokens` を載せない**（ADR-0060 §決定3）。
   */
  it("保存: 思考を切ると繰り返しの罰だけが残り、空の上限は null で送り、裏方には上限を載せない", async () => {
    const view = await openEditor("beta");
    thinkingButtons()[0].click();
    await vi.waitFor(() => {
      expect(thinkingButtons()[0].getAttribute("aria-pressed")).toBe("true");
    });
    type(fieldOf(MAIN_LEGEND, ROOM_LABEL), "");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.method === "PUT")).toHaveLength(1);
    });
    const body = lastPut();
    expect([body.main["sampling"], body.main["response_tokens"]]).toEqual([{ repeat_penalty: 1.05 }, null]);
    expect(Object.keys(body.main)).toEqual(expect.arrayContaining(["sampling", "response_tokens"]));
    expect(body.sub?.["sampling"]).toEqual({ repeat_penalty: 1.25 });
    expect(Object.keys(body.sub ?? {})).not.toContain("response_tokens");
    expect(Object.keys(body.sub ?? {})).not.toContain("reasoning_effort");
    await unmount(view, { outro: false });
  });

  it("保存: 両方の欄を空にした面は、`sampling` を省かず `null` で送る", async () => {
    const view = await openEditor("beta");
    thinkingButtons()[0].click();
    type(fieldOf(MAIN_LEGEND, PENALTY_LABEL), "");
    type(fieldOf(SUB_LEGEND, PENALTY_LABEL), "");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.method === "PUT")).toHaveLength(1);
    });
    const body = lastPut();
    expect([body.main["sampling"], body.sub?.["sampling"]]).toEqual([null, null]);
    expect(Object.keys(body.sub ?? {})).toContain("sampling");
    await unmount(view, { outro: false });
  });

  /** **事前の検査は、その欄で言う。直せば送る**（範囲 0.5〜2.0・上限は同じ面に書いた窓より小さい）。 */
  it("送る前の検査: 繰り返しの罰の範囲と、窓以上の上限を、その欄で言い、直せば送る", async () => {
    const view = await openEditor("beta");
    type(fieldOf(SUB_LEGEND, PENALTY_LABEL), "2.5");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(fieldOf(SUB_LEGEND, PENALTY_LABEL).getAttribute("aria-invalid")).toBe("true");
    });
    expect(describedText(fieldOf(SUB_LEGEND, PENALTY_LABEL))).toContain("0.5 以上 2.0 以下");
    expect((fieldOf(SUB_LEGEND, PENALTY_LABEL).closest("details") as HTMLDetailsElement).open).toBe(true);
    expect(fieldOf(MAIN_LEGEND, PENALTY_LABEL).getAttribute("aria-invalid")).toBeNull();

    type(fieldOf(SUB_LEGEND, PENALTY_LABEL), "2");
    type(fieldOf(MAIN_LEGEND, WINDOW_LABEL), "8192");
    type(fieldOf(MAIN_LEGEND, ROOM_LABEL), "8192");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(fieldOf(MAIN_LEGEND, ROOM_LABEL).getAttribute("aria-invalid")).toBe("true");
    });
    expect(describedText(fieldOf(MAIN_LEGEND, ROOM_LABEL))).toContain("コンテキスト長より小さく");
    expect(sent.filter((s) => s.method === "PUT")).toHaveLength(0);

    type(fieldOf(MAIN_LEGEND, ROOM_LABEL), "8191");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(sent.filter((s) => s.method === "PUT")).toHaveLength(1);
    });
    const body = lastPut();
    expect([body.main["response_tokens"], body.sub?.["sampling"]]).toEqual([8191, { repeat_penalty: 2 }]);
    await unmount(view, { outro: false });
  });

  /**
   * **422 の `sampling` は、その面のサンプリングの組の見出しに出る**（`reason` だけでは思考と繰り返しの罰を区別できない）。
   * **裏方の面の失敗は本文の面の組に出ない**（詳細に畳んだ裏方の組が開いて見える）。
   */
  it("422: `sampling` はその面のサンプリングの組に出て、裏方の失敗は本文の組に出ない", async () => {
    const message = "sampling が使えません（オブジェクトで書き、repeat_penalty は 0.5 以上 2 以下の数、reasoning_effort は none / low で書いてください）";
    onSave = () =>
      apiError(422, "upstream_profile_invalid", "上流プロファイルの形が不正なので保存しませんでした", {
        reason: "sampling_unusable",
        reason_message: message,
        face: "sub",
        key: "sampling",
      });
    const view = await openEditor("beta");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(flat(samplingGroup(SUB_LEGEND))).toContain(`保存しませんでした: ${message}`);
    });
    expect(flat(samplingGroup(MAIN_LEGEND))).not.toContain("保存しませんでした");
    expect((samplingGroup(SUB_LEGEND)?.closest("details") as HTMLDetailsElement).open).toBe(true);
    // **組の見出しと理由は結びついている。**
    expect(describedText(samplingGroup(SUB_LEGEND) as HTMLElement)).toContain(message);
    await unmount(view, { outro: false });
  });

  /** **上限が窓以上の 422 は `key: "context_tokens"`。起こす面の窓の欄は詳細に畳まれているので、詳細が開く**（作業単位 D-19 の振る舞い）。 */
  it("422: `context_tokens`（上限が窓以上）は窓の欄に出て、起こす面の詳細が開く", async () => {
    onSave = () =>
      apiError(422, "upstream_profile_invalid", "上流プロファイルの形が不正なので保存しませんでした", {
        reason: "context_tokens_unusable",
        reason_message: "context_tokens が使えません（応答の余白より大きい整数のトークン数で書いてください）",
        face: "main",
        key: "context_tokens",
      });
    const view = await openEditor("beta");
    const details = fieldOf(MAIN_LEGEND, WINDOW_LABEL).closest("details") as HTMLDetailsElement;
    expect(details.open).toBe(false);
    type(fieldOf(MAIN_LEGEND, ROOM_LABEL), "4000");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(fieldOf(MAIN_LEGEND, WINDOW_LABEL).getAttribute("aria-invalid")).toBe("true");
    });
    expect(details.open).toBe(true);
    expect(fieldOf(MAIN_LEGEND, ROOM_LABEL).getAttribute("aria-invalid")).toBeNull();
    await unmount(view, { outro: false });
  });

  /**
   * **思考を入れて上限が空か既定以下なら、欄の隣で言う。数は出さない**（ADR-0060 §決定3）。
   * **範囲は柵だと言い、保存しただけでは効かず同じ名前へ切り替え直すと効くと言う。**
   */
  it("案内: 思考を入れて上限が空なら注意を出し、範囲は柵だと言い、切り替え直すと効くと言う", async () => {
    const view = await openEditor("gamma");
    const caution = "思考は返答の上限の中で書かれます";
    expect(flat(samplingGroup(MAIN_LEGEND))).not.toContain(caution);
    thinkingButtons()[1].click();
    await vi.waitFor(() => {
      expect(flat(samplingGroup(MAIN_LEGEND))).toContain(caution);
    });
    const warned = [...(samplingGroup(MAIN_LEGEND) as HTMLElement).querySelectorAll("p")].find((p) =>
      flat(p).includes(caution),
    );
    expect(flat(warned)).not.toMatch(/[0-9]/u);
    type(fieldOf(MAIN_LEGEND, ROOM_LABEL), "1500");
    await vi.waitFor(() => {
      expect(flat(samplingGroup(MAIN_LEGEND))).not.toContain(caution);
    });
    // **値を自動で入れない**（入れたのは利用者の 1500 だけ）。
    expect(fieldOf(MAIN_LEGEND, ROOM_LABEL).value).toBe("1500");

    expect(notesNear(fieldOf(MAIN_LEGEND, PENALTY_LABEL)).join(" ")).toContain("柵");
    expect(flat(samplingGroup(MAIN_LEGEND))).toContain("同じ名前へ切り替え直すと効きます");
    // **繋ぐ面にも詳細ができる**（繰り返しの罰）。
    expect(flat(fieldsetOf(MAIN_LEGEND).querySelector("details > summary"))).toBe("詳細（繰り返しの罰）");
    await unmount(view, { outro: false });
  });
});

describe("一覧の状態", () => {
  it("読むだけの置き場所では、足す・直すを出さず、読んでいるファイルと直せないことを言う", async () => {
    listingReplies = [ok(listing({ editable: false, path: "F:/repo/scripts/upstreams.json" }))];
    const view = await open();
    const written = sectionNamed(WRITTEN);
    expect(flat(written)).toContain("読んでいるファイル: F:/repo/scripts/upstreams.json");
    expect(flat(written)).toContain("この起動では、ここからは直せません（読むだけです）。");
    expect(buttonNamed(written, "プロファイルを足す")).toBeUndefined();
    expect(["alpha", "beta", "gamma"].map((name) => buttonNamed(profileBlock(name), "直す"))).toEqual([
      undefined,
      undefined,
      undefined,
    ]);
    // **選ぶことはできる**（差し替えは読むだけの置き場所からも読む）。
    expect(
      ["alpha", "beta", "gamma"].map((name) => buttonNamed(profileBlock(name), "このプロファイルに切り替える")?.disabled),
    ).toEqual([false, false, false]);
    await unmount(view, { outro: false });
  });

  it("検証に通らないファイルは、理由と直せないことを言い、足す・直すを出さない", async () => {
    listingReplies = [
      ok(
        listing({
          state: "invalid",
          profiles: [],
          problem: {
            reason: "unknown_key",
            message:
              "知らない鍵があります（使えるのは endpoint / model / gguf / args / command / context_tokens だけです）",
            profile: "beta",
            face: "main",
            key: null,
          },
        }),
      ),
    ];
    const view = await open();
    const written = sectionNamed(WRITTEN);
    expect(flat(written)).toContain(
      "ファイルが読み込みの検証に通りません: 知らない鍵があります（使えるのは endpoint / model / gguf / args / command / context_tokens だけです）（プロファイル「beta」の main）",
    );
    expect(flat(written)).toContain("画面からは直せません。ファイルを手で直してから、読み直してください。");
    expect(buttonNamed(written, "プロファイルを足す")).toBeUndefined();
    expect(written.querySelectorAll("section.profile")).toHaveLength(0);
    // **手で直した後の読み直しはできる。**
    expect(buttonNamed(written, "読み直す")).toBeDefined();
    await unmount(view, { outro: false });
  });

  it("ファイルが無ければ、足すことができ、印を持たずに送る", async () => {
    listingReplies = [ok(listing({ state: "missing", revision: null, profiles: [] }))];
    onSave = () => ok(listing({ revision: "rev-new", profiles: [GAMMA] }));
    const view = await open();
    const written = sectionNamed(WRITTEN);
    expect(flat(written)).toContain(
      "まだファイルがありません。プロファイルを足して保存すると、この場所に作ります。",
    );
    mustButton(written, "プロファイルを足す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    type(nameField(), "gamma");
    // **`gamma` の本文は繋ぐ面**（gguf を書かない）。足すと起こすで開くので、繋ぐを選ぶ。
    kindButtons(MAIN_LEGEND)[1].click();
    await vi.waitFor(() => {
      expect(pressedOf(MAIN_LEGEND)).toEqual(["false", "true"]);
    });
    type(fieldOf(MAIN_LEGEND, ENDPOINT_CONNECT), "http://127.0.0.1:41005");
    type(fieldOf(MAIN_LEGEND, "依頼するモデルの綴り（model）"), "gamma-main");
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(listedNames()).toEqual(["gamma"]);
    });
    expect(sent.filter((s) => s.method === "PUT").map((s) => s.body)).toEqual([
      { revision: null, main: requested(GAMMA.main), sub: null },
    ]);
    await unmount(view, { outro: false });
  });

  it("読めないファイルは、理由と直せないことを言う", async () => {
    listingReplies = [
      ok(
        listing({
          state: "unreadable",
          revision: null,
          profiles: [],
          problem: {
            reason: "unreadable",
            message: "上流プロファイルを読めませんでした（置き場所の権限か文字コードを確かめてください）",
            profile: null,
            face: null,
            key: null,
          },
        }),
      ),
    ];
    const view = await open();
    const written = sectionNamed(WRITTEN);
    expect(flat(written)).toContain(
      "ファイルを読めませんでした: 上流プロファイルを読めませんでした（置き場所の権限か文字コードを確かめてください）",
    );
    expect(buttonNamed(written, "プロファイルを足す")).toBeUndefined();
    await unmount(view, { outro: false });
  });

  /**
   * **形の違う応答で画面ごと落ちない**（UI のフォールバックが `{}` を返す等）。
   * 落ちると観測の節まで消える。「一覧を読めなかった」と言うほうに倒す。
   */
  it("形の違う一覧の応答は、読めなかったと言い、観測の節は残る", async () => {
    listingReplies = [ok({})];
    const errors: unknown[] = [];
    const onError = (event: ErrorEvent): void => {
      errors.push(event.error);
    };
    window.addEventListener("error", onError);
    const view = await open();
    await vi.waitFor(() => {
      expect(flat(sectionNamed(WRITTEN))).toContain(
        "プロファイルの一覧を取得できませんでした: 一覧の応答を読めませんでした",
      );
    });
    expect(launchCells()[1]?.[0]).toBe("起こした上流の状態");
    window.removeEventListener("error", onError);
    expect(errors).toEqual([]);
    await unmount(view, { outro: false });
  });

  it("一覧を読めなければ、そう言って読み直せる", async () => {
    listingReplies = [{ status: 500, body: "boom" }];
    const view = await open();
    const written = sectionNamed(WRITTEN);
    await vi.waitFor(() => {
      expect(flat(written)).toContain("プロファイルの一覧を取得できませんでした");
    });
    mustButton(written, "読み直す").click();
    await vi.waitFor(() => {
      expect(listedNames()).toEqual(["alpha", "beta", "gamma"]);
    });
    await unmount(view, { outro: false });
  });
});

describe("消す操作", () => {
  /** **消す操作は置かない**（ADR-0042 §決定8）。 */
  it("消す・削除を名乗るボタンが無く、`DELETE` を送らない", async () => {
    savingServer2();
    const view = await open();
    mustButton(profileBlock("beta"), "直す").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).not.toBeNull();
    });
    const labels = [...target.querySelectorAll("button")].map((button) => flat(button));
    expect(labels.filter((label) => /消|削除/u.test(label))).toEqual([]);
    mustButton(target.querySelector("form") as HTMLFormElement, "保存する").click();
    await vi.waitFor(() => {
      expect(target.querySelector("form")).toBeNull();
    });
    expect(sent.filter((s) => s.method === "DELETE")).toEqual([]);
    await unmount(view, { outro: false });
  });

  function savingServer2(): void {
    onSave = () => ok(listing({ revision: "rev-3" }));
  }
});
