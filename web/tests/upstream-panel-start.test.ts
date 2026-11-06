// @vitest-environment jsdom
/**
 * **`#/upstream` の画面が、起こしていない本文の面を言い、起こすボタンを出す**
 * （作業単位 F-10b / Issue origin#83 / ADR-0055 §決定3）。
 *
 * # ここで見るもの（**配線** —— 画面 → 起こす依頼 → 状態の行）
 *
 * **状態の行**は、作業単位 D-12b-2 から面ごとのカードの「起こした上流の状態」（着手前は表の1行）。
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 起こすボタンが送る名前（純粋関数） | `upstream-profiles.test.ts` の「起こすボタンが送る名前」 |
 * | 会話の画面の釦と送信の停止 | `chat-view-body-face.test.ts` の「起こしていない本文の面」 |
 * | **この画面が釦を出し、押すと選ばれている名前で頼み、状態の行が `loading` → `ready` と進む** | **ここ** |
 *
 * # ⚠ 選ばれているプロファイルは、一覧の端に置かない
 *
 * **一覧は3件（`alpha` / `beta` / `gamma`）、選ばれているのは真ん中の `beta`**（CLAUDE.md「選ぶ操作のテストは
 * 対象を端に置かない」）。**一覧の先頭・末尾を送る実装を、送った名前で赤にする。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。実の上流も起こさない（`fetch` の代役）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import UpstreamPanel from "../src/lib/UpstreamPanel.svelte";
import type { LaunchView, ProfilesView, UpstreamReport, UpstreamView, WrittenProfile } from "../src/lib/api";
import { launchStateRows } from "./support/upstream-cards";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

const UPSTREAM_PATH = "/admin/upstream";
const PROFILES_PATH = "/admin/upstream/profiles";
const SWITCH_PATH = "/admin/upstream/profile";

/** **選ばれているプロファイル**（一覧の真ん中）。 */
const SELECTED = "beta";

function profile(name: string, port: number): WrittenProfile {
  return {
    name,
    main: {
      endpoint: `http://127.0.0.1:${port}`,
      model: `${name}-main`,
      gguf: `~/models/${name}.gguf`,
      args: null,
      command: null,
      context_tokens: null,
      api_key_set: false,
      api_key_hint: null,
      allow_key_over_plain_http: false,
    },
    sub: null,
  };
}

const LISTING: ProfilesView = {
  path: "C:\\Users\\someone\\AppData\\Local\\personacore\\upstreams.json",
  editable: true,
  state: "ok",
  revision: "rev",
  problem: null,
  profiles: [profile("alpha", 41001), profile(SELECTED, 41002), profile("gamma", 41003)],
};

function launch(overrides: Partial<LaunchView>): LaunchView {
  return { state: "external", since: null, answered_name: null, reason: null, log: null, ...overrides };
}

function view(face: string, launched: LaunchView): UpstreamView {
  return {
    face,
    endpoint: face === "main" ? "http://127.0.0.1:41002/v1/chat/completions" : "http://127.0.0.1:41012/v1/chat/completions",
    model: face === "main" ? "beta-main" : "beta-sub",
    answered_name: null,
    queried_context_tokens: null,
    answered_by: null,
    unauthorized: false,
    checked_at: null,
    check_result: "not_checked",
    measured: { checked: 0, matched: 0, drifted_other: 0, drifted_absent: 0, unanswered: 0 },
    launch: launched,
  };
}

function report(main: LaunchView, sub: LaunchView, selected: string | null = SELECTED): UpstreamReport {
  return {
    main: view("main", main),
    sub: view("sub", sub),
    // **見本の場所とは別の綴り**（名前と取り違えたら、送った名前で赤になる）。
    profile_sample: "C:\\Users\\someone\\AppData\\Local\\personacore\\upstreams.json",
    selected_profile: selected,
  };
}

const NOT_STARTED = launch({ state: "not_started" });
const LOADING = launch({ state: "loading" });
const READY_MAIN = launch({ state: "ready", answered_name: "beta-main.gguf" });
const READY_SUB = launch({ state: "ready", answered_name: "beta-sub.gguf" });

/** 状態の文面（**直書き** —— 実装の定数から作らない）。 */
const NOT_STARTED_LABEL = "まだ起こしていません（前回選んだプロファイルは、起動しただけでは起こしません）";
const START_BUTTON = `「${SELECTED}」を起こす`;

let target: HTMLElement;
let urls: string[];
/** `GET /admin/upstream` が返すもの（いまの値）。 */
let upstream: UpstreamReport;
/** `POST /admin/upstream/profile` に送られた名前（順に）。 */
let switched: string[];
/** `POST /admin/upstream/profile` の応答（`"busy"` は 409）。 */
let switchResponse: UpstreamReport | "busy";

function json(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    text: () => Promise.resolve(JSON.stringify(body)),
  } as unknown as Response;
}

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    urls.push(url);
    if (url === PROFILES_PATH) {
      return Promise.resolve(json(LISTING));
    }
    if (url === UPSTREAM_PATH) {
      return Promise.resolve(json(upstream));
    }
    if (url === SWITCH_PATH && init?.method === "POST") {
      switched.push((JSON.parse(String(init.body)) as { name: string }).name);
      if (switchResponse === "busy") {
        return Promise.resolve(
          json(
            {
              error: {
                message: "上流の差し替えが進行中です。終わってから選び直してください",
                type: "invalid_request_error",
                code: "upstream_switch_in_progress",
              },
            },
            409,
          ),
        );
      }
      // **サーバは応答と同じ状態を、次の読みでも返す。**
      upstream = switchResponse;
      return Promise.resolve(json(switchResponse));
    }
    return Promise.resolve(json({ error: { message: "?", type: "invalid_request_error" } }, 404));
  });
}

function flat(element: Element | null): string {
  return (element?.textContent ?? "").replace(/\s+/gu, " ").trim();
}

/** 観測の節（見出し「いまの上流」）。 */
function nowSection(): HTMLElement {
  const found = [...target.querySelectorAll("section")].filter(
    (section) => flat(section.querySelector(":scope > h2")) === "いまの上流",
  );
  expect(found, "観測の節が1つだけあること").toHaveLength(1);
  return found[0] as HTMLElement;
}

/**
 * **起こした上流の状態**を、着手前の表と同じ形（`[見出しの行, 状態の行]`）で読む。
 *
 * **⚠ 作業単位 D-12b-2 で読む先を変えた。** 状態は表（見出し「この実行ファイルが上流をどうしているか」）から、
 * **面ごとのカードの「起こした上流の状態」の行**へ移った。読み方は `support/upstream-cards.ts`。
 * **アサートの側（どの面にどの文面が出るか）は変えていない。**
 */
function launchCells(): string[][] {
  return launchStateRows(nowSection());
}

/** **画面にある「…を起こす」釦の文面**（どこに置いても数える）。 */
function startButtons(): string[] {
  return [...target.querySelectorAll("button")].map((button) => flat(button)).filter((text) => text.endsWith("を起こす"));
}

function startButton(): HTMLButtonElement {
  const found = [...target.querySelectorAll("button")].find((button) => flat(button) === START_BUTTON);
  expect(found, "起こすボタンがあること").toBeDefined();
  return found as HTMLButtonElement;
}

let views: ReturnType<typeof mount>[];

async function open(): Promise<void> {
  views.push(mount(UpstreamPanel, { target, props: { pollIntervalMs: 5 } }));
  await vi.waitFor(() => {
    expect(nowSection().querySelector("table")).not.toBeNull();
    // **一覧も読み終えてから見る**（一覧の「このプロファイルに切り替える」と数え分けるため）。
    expect(target.querySelectorAll("section.profile")).toHaveLength(3);
  });
}

beforeEach(() => {
  urls = [];
  switched = [];
  views = [];
  upstream = report(NOT_STARTED, NOT_STARTED);
  switchResponse = report(LOADING, LOADING);
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(async () => {
  for (const mounted of views) {
    await unmount(mounted, { outro: false });
  }
  vi.unstubAllGlobals();
  target.remove();
});

describe("起こしていない本文の面を言い、起こすボタンを出す", () => {
  it("表が両方の面を起こしていないと言い、選ばれているプロファイルを起こすボタンが1つだけ出る", async () => {
    await open();
    expect(launchCells()).toEqual([
      ["項目", "本文生成", "裏方（抽出・変容）"],
      ["起こした上流の状態", NOT_STARTED_LABEL, NOT_STARTED_LABEL],
    ]);
    expect(startButtons()).toEqual([START_BUTTON]);
    // **釦は、状態を言う節の中にある**（状態の隣で聞く / ADR-0055 §決定3）。
    expect(nowSection().contains(startButton())).toBe(true);
    // **押す前に確かめることを言う**（ADR-0055 の背景 —— ほかの上流が GPU を使っているかもしれない）。
    expect(flat(nowSection())).toContain("ほかの推論サーバが GPU を使っていないことを確かめてから押してください。");
    // **押すまでは何も頼まない。**
    expect(switched).toEqual([]);
  });

  /**
   * **押すと、選ばれている名前（一覧の真ん中）で頼み、応答の `loading` にそのまま乗り、載ったら `ready`**
   * （「A が B になる」—— 前半で A（起こしていない）を確かめてから押す）。
   */
  it("押すと、一覧の真ん中の選ばれている名前で頼み、表が載せている → 準備完了と進む", async () => {
    await open();
    expect(launchCells()[1]).toEqual(["起こした上流の状態", NOT_STARTED_LABEL, NOT_STARTED_LABEL]);

    startButton().click();
    await vi.waitFor(() => expect(switched).toEqual([SELECTED]));
    await vi.waitFor(() =>
      expect(launchCells()[1]).toEqual(["起こした上流の状態", "載せています", "載せています"]),
    );
    // **起こした後は、もう釦を出さない。**
    expect(startButtons()).toEqual([]);

    // **載せている間は読み直し続け、載ったら準備完了を言う**（既存の差し替えの表示）。
    upstream = report(READY_MAIN, READY_SUB);
    await vi.waitFor(() =>
      expect(launchCells()[1]).toEqual([
        "起こした上流の状態",
        "準備完了（上流が名乗った名前: beta-main.gguf）",
        "準備完了（上流が名乗った名前: beta-sub.gguf）",
      ]),
    );
    expect(switched).toEqual([SELECTED]);
  });

  it("断られたら（409）、サーバの理由を言い、状態は変えず、もう一度押せる", async () => {
    switchResponse = "busy";
    await open();

    startButton().click();
    await vi.waitFor(() =>
      expect(flat(nowSection().querySelector("[role=alert]"))).toBe(
        "起こせませんでした: 上流の差し替えが進行中です。終わってから選び直してください",
      ),
    );
    expect(switched).toEqual([SELECTED]);
    expect(launchCells()[1]).toEqual(["起こした上流の状態", NOT_STARTED_LABEL, NOT_STARTED_LABEL]);
    expect(startButton().disabled).toBe(false);

    switchResponse = report(LOADING, LOADING);
    startButton().click();
    await vi.waitFor(() =>
      expect(launchCells()[1]).toEqual(["起こした上流の状態", "載せています", "載せています"]),
    );
    expect(switched).toEqual([SELECTED, SELECTED]);
    expect(target.textContent).not.toContain("起こせませんでした");
  });
});

/**
 * **対照: 起こしていない以外では、釦を出さない**（`not_started` 以外の表示を変えない）。
 * **裏方だけが起こしていないときも出さない**（本文の面で決める / ADR-0055 §決定3）。
 */
describe("対照: 起こすボタンを出さない状態", () => {
  it.each([
    ["準備完了", READY_MAIN, READY_SUB],
    ["外の上流", launch({ state: "external" }), launch({ state: "external" })],
    ["載せている", LOADING, READY_SUB],
    ["失敗", launch({ state: "failed", reason: "子が自分で終了しました" }), READY_SUB],
    ["止めた", launch({ state: "stopped" }), launch({ state: "stopped" })],
    ["裏方だけ起こしていない", READY_MAIN, NOT_STARTED],
  ] as const)("%s → 釦を出さない", async (_name, main, sub) => {
    upstream = report(main, sub);
    await open();
    expect(startButtons()).toEqual([]);
  });

  it("選ばれている名前が無ければ、起こしていなくても釦を出さない（無い名前を頼ませない）", async () => {
    upstream = report(NOT_STARTED, NOT_STARTED, null);
    await open();
    expect(launchCells()[1]).toEqual(["起こした上流の状態", NOT_STARTED_LABEL, NOT_STARTED_LABEL]);
    expect(startButtons()).toEqual([]);
  });
});

/** **本番の入口**（props なし）でも、釦を出し、押せば選ばれている名前で頼む。 */
describe("既定の間隔（props を渡さない本番の入口）", () => {
  it("props なしでも、釦を出し、押せば選ばれている名前で頼む", async () => {
    views.push(mount(UpstreamPanel, { target }));
    await vi.waitFor(() => expect(startButtons()).toEqual([START_BUTTON]));
    startButton().click();
    await vi.waitFor(() => expect(switched).toEqual([SELECTED]));
    expect(urls.filter((url) => url === SWITCH_PATH)).toHaveLength(1);
  });
});
