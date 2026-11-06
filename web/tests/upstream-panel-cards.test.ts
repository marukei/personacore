// @vitest-environment jsdom
/**
 * **「いまの上流」を、面ごとのカードと状態の印で出し、異常を帯で知らせ、照らし合わせを畳む**
 * （作業単位 D-12b-2 / Issue origin#93 / `src/lib/UpstreamPanel.svelte`）。
 *
 * # ここで見るもの（**配線** —— 観測 → カード・帯・照らし合わせ）
 *
 * | 見るもの | どこ |
 * |---|---|
 * | どの値から、どの印・帯・開き方を決めるか（組み合わせの表） | `upstream-status.test.ts`（純粋関数） |
 * | **カードが面ごとに、その面の値を出すこと**（帰属は見出しで読む） | **ここ** |
 * | **状態の印が、その面の状態の文面と色で出ること** | **ここ** |
 * | **異常の帯が出る状態・出ない状態**（画面に描かれたもの） | **ここ** |
 * | **照らし合わせが常に閉じて出て、見出しが一致しない行の数を言い、読み直しで数が更新されること** | **ここ** |
 * | **照らし合わせの開閉は、利用者の操作だけで変わること**（読み直しで開きも閉じもしない） | **ここ** |
 *
 * **⚠ 司令塔の判断で仕様を変えた（D-12b-2 の2周目）。** 「答えがない」は帯にしない（照会していない面も同じ形で
 * 届くので、コンテキスト長を書いた繋ぐ面で帯が出続ける）。照らし合わせは一致しない行があっても自動で開かない
 * （llama-server ではずれが常にある / G-5）。1周目の「読み直しで開く」「畳んだら開き直さない」は、上の2行へ置き換えた。
 *
 * **⚠ 作業単位 B-66 で「答えがない」をもう一度割った**（Issue origin#94）。外の上流で照会したが答えない面は帯
 * （「繋がりません」）、まだ照会していない面は中立（「まだ確かめていません」）。カードに「最後の確認」の行と、
 * 節に「いま確かめる」ボタンが加わった（このファイルの末尾）。
 * | 起こすボタン・読み直し・書いた値との分離 | `upstream-panel-start` / `upstream-profiles-panel` / `upstream-panel` |
 *
 * # ⚠ 2つの面で同じ値にしない
 *
 * 宛先・名乗った名前・長さ・突き合わせの数を、面ごとに全部違えてある（取り違えが観測できるように）。
 *
 * # ⚠ 期待値は直書きする（実装の定数から作らない）
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import UpstreamPanel from "../src/lib/UpstreamPanel.svelte";
import type { LaunchView, MeasuredView, ProfilesView, UpstreamReport, UpstreamView } from "../src/lib/api";
import { cardHeading, cardNamed, cardRows, faceCards } from "./support/upstream-cards";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

const UPSTREAM_PATH = "/admin/upstream";
const PROFILES_PATH = "/admin/upstream/profiles";

const EMPTY_LISTING: ProfilesView = {
  path: "C:\\Users\\someone\\AppData\\Local\\personacore\\upstreams.json",
  editable: true,
  state: "ok",
  revision: "rev",
  problem: null,
  profiles: [],
};

const MAIN_HEADING = "本文生成";
const SUB_HEADING = "裏方（抽出・変容）";

function launch(overrides: Partial<LaunchView>): LaunchView {
  return { state: "external", since: null, answered_name: null, reason: null, log: null, ...overrides };
}

const MAIN_ENDPOINT = "http://127.0.0.1:41001/v1/chat/completions";
const SUB_ENDPOINT = "http://127.0.0.1:41002/v1/chat/completions";

/** **一致しない行が無い**突き合わせ（面ごとに数を違える）。 */
const MAIN_AGREES: MeasuredView = { checked: 5, matched: 5, drifted_other: 0, drifted_absent: 0, unanswered: 0 };
const SUB_AGREES: MeasuredView = { checked: 3, matched: 1, drifted_other: 0, drifted_absent: 0, unanswered: 2 };

/** 1面。**既定は「外の上流が答えている」。** 2つの面で1欄も同じ値にしない。 */
function face(name: "main" | "sub", overrides: Partial<UpstreamView> = {}): UpstreamView {
  return {
    face: name,
    endpoint: name === "main" ? MAIN_ENDPOINT : SUB_ENDPOINT,
    model: name === "main" ? "main-model-asked" : "sub-model-asked",
    answered_name: name === "main" ? "main-answering.gguf" : "sub-answering.gguf",
    queried_context_tokens: name === "main" ? 16384 : 8192,
    answered_by: name === "main" ? "native-api" : "openai-exact-id",
    unauthorized: false,
    // **確かめた時刻**（作業単位 B-66。画面は読み手の時間帯 —— 冒頭の `readerInZone("America/New_York")` ——
    // の `HH:MM:SS` で言う。2026-11-03 の New York は UTC-5 なので 10:03:30。**サーバの綴り（小数部 9 桁・`+00:00`）で書く**）。
    checked_at: "2026-11-03T15:03:30.123456789+00:00",
    check_result: "answered",
    measured: name === "main" ? MAIN_AGREES : SUB_AGREES,
    launch: launch({}),
    ...overrides,
  };
}

function report(main: Partial<UpstreamView> = {}, sub: Partial<UpstreamView> = {}): UpstreamReport {
  return { main: face("main", main), sub: face("sub", sub), profile_sample: null, selected_profile: null };
}

/** **照会したが答えない**（作業単位 B-66 で `check_result` を足した）。 */
const SILENT: Partial<UpstreamView> = {
  answered_by: null,
  answered_name: null,
  queried_context_tokens: null,
  check_result: "no_answer",
};
/** **まだ照会していない**（作業単位 B-66）。 */
const NOT_CHECKED: Partial<UpstreamView> = { ...SILENT, checked_at: null, check_result: "not_checked" };
const REFUSED: Partial<UpstreamView> = { ...SILENT, unauthorized: true, check_result: "refused" };
/** **承認が無いので鍵を送らなかった**（作業単位 A-13b-3）。 */
const WITHHELD: Partial<UpstreamView> = {
  ...SILENT,
  key_over_plain_http: "not_approved",
  check_result: "key_not_sent",
};
/** **答えない外の上流の帯の文面**（作業単位 B-66 / **直書き**）。 */
const NO_ANSWER_BAND =
  "推論サーバへ繋がりません（10:03:30 に確かめました）。推論サーバを起動したら「いま確かめる」で確かめ直せます。";
/** **届いたが一覧が読めない**（レビュー是正）。 */
const NO_MODEL_LIST: Partial<UpstreamView> = { ...SILENT, check_result: "no_model_list" };

/** 鍵を断られたときの帯の文面（**直書き**）。 */
const REFUSED_BAND =
  "推論サーバが鍵を受け付けませんでした。鍵が無い・違う・期限切れのほか、上流の側で鍵を有効にしていない、宛先が違う（末尾の /v1 が二重など）ことも考えられます。下の「upstreams.json に書いたプロファイル」の「直す」でこの面の鍵の欄と宛先を確かめ、保存したら「切り替え直す」を押してください。";
/** 承認が無くて鍵を送らなかったときの帯の文面（**直書き**）。 */
const WITHHELD_BAND =
  "鍵を暗号化せずにローカルネットワークの外へ送る承認が無いので、鍵を送っていません（推論サーバは鍵を見ていません）。下の「upstreams.json に書いたプロファイル」の「直す」で承認するか、宛先を https:// にしてください。";
const READY = launch({ state: "ready", answered_name: "main-answering.gguf" });
const LOADING = launch({ state: "loading" });
const NOT_STARTED = launch({ state: "not_started" });
const FAILED = launch({ state: "failed", reason: "子が自分で終了しました", log: "main-1.err.log" });
const STOPPED = launch({ state: "stopped" });

let target: HTMLElement;
let upstream: UpstreamReport;
let views: ReturnType<typeof mount>[];
/** **`POST /admin/upstream/check` の応答**（作業単位 B-66）。テストが差し替える。 */
let checkAnswer: () => Promise<Response>;
/** `POST /admin/upstream/check` を受けた回数。 */
let checks: number;

const CHECK_PATH = "/admin/upstream/check";

function json(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    text: () => Promise.resolve(JSON.stringify(body)),
  } as unknown as Response;
}

beforeEach(() => {
  views = [];
  checks = 0;
  upstream = report();
  checkAnswer = () => Promise.resolve(json(upstream));
  target = document.createElement("div");
  document.body.append(target);
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    if (url === PROFILES_PATH) {
      return Promise.resolve(json(EMPTY_LISTING));
    }
    if (url === UPSTREAM_PATH) {
      return Promise.resolve(json(upstream));
    }
    if (url === CHECK_PATH && init?.method === "POST") {
      checks += 1;
      return checkAnswer();
    }
    return Promise.reject(new TypeError(`想定していない口: ${url}`));
  });
});

afterEach(async () => {
  for (const mounted of views) {
    await unmount(mounted, { outro: false });
  }
  vi.unstubAllGlobals();
  target.remove();
});

function flat(element: Element | null): string {
  return (element?.textContent ?? "").replace(/\s+/gu, " ").trim();
}

function nowSection(): HTMLElement {
  const found = [...target.querySelectorAll("section")].filter(
    (section) => flat(section.querySelector(":scope > h2")) === "いまの上流",
  );
  expect(found, "観測の節が1つだけあること").toHaveLength(1);
  return found[0] as HTMLElement;
}

async function open(): Promise<void> {
  views.push(mount(UpstreamPanel, { target, props: { pollIntervalMs: 5 } }));
  await vi.waitFor(() => {
    expect(faceCards(nowSection())).toHaveLength(2);
  });
}

/** **カードの状態の印**（見出しの隣の要素）。 */
function badgeOf(card: Element): HTMLElement {
  const badge = card.querySelector("h3")?.nextElementSibling;
  expect(badge, `カード「${cardHeading(card)}」に状態の印があること`).toBeTruthy();
  return badge as HTMLElement;
}

/** 2枚のカードの印の文面（本文 → 裏方。**見出しで引く**）。 */
function badges(): string[] {
  return [MAIN_HEADING, SUB_HEADING].map((heading) => flat(badgeOf(cardNamed(nowSection(), heading))));
}

/** カードの大きな名前（印の行の次の段落）。 */
function nameOf(card: Element): string {
  return flat(card.querySelector(":scope > p"));
}

/** **異常の帯**の、子ごとの文面（`[面, 文面, 宛先?]`）。 */
function bands(): string[][] {
  const holder = nowSection().querySelector('[aria-label="上流の異常"]');
  expect(holder, "帯の容れ物は、異常が無くても在ること").not.toBeNull();
  return [...(holder as Element).children].map((band) => [...band.children].map((part) => flat(part)));
}

/** 照らし合わせ（`<details>`）。1つだけ。 */
function comparison(): HTMLDetailsElement {
  const found = nowSection().querySelectorAll("details");
  expect(found, "照らし合わせが1つだけあること").toHaveLength(1);
  return found[0] as HTMLDetailsElement;
}

describe("面ごとのカード", () => {
  /**
   * **1枚ずつ、中身を丸ごと突き合わせる。** 本文は起こした面（準備完了）、裏方は外の上流（黙っている）にして、
   * 起こし方・宛先・長さ・名乗った名前の全部が面ごとに違う形で並ぶ。
   */
  it("本文 → 裏方の順に、その面の印・名前・起こし方・宛先・状態・長さを出す", async () => {
    upstream = report({ launch: READY }, SILENT);
    await open();

    expect(faceCards(nowSection()).map((card) => cardHeading(card))).toEqual([MAIN_HEADING, SUB_HEADING]);

    const main = cardNamed(nowSection(), MAIN_HEADING);
    expect(flat(badgeOf(main))).toBe("動いています");
    expect(nameOf(main)).toBe("main-answering.gguf");
    expect(cardRows(main)).toEqual([
      ["起こし方", "この機で llama を起こす"],
      ["宛先", MAIN_ENDPOINT],
      ["起こした上流の状態", "準備完了（上流が名乗った名前: main-answering.gguf）"],
      ["最後の確認", "10:03:30 に確かめました（答えました）"],
      ["コンテキスト長", "16384 トークン（上流が申告した値）"],
    ]);

    // **照会したが答えない外の上流**（作業単位 B-66 —— 着手前は中立の「名乗りがありません」だった）。
    const sub = cardNamed(nowSection(), SUB_HEADING);
    expect(flat(badgeOf(sub))).toBe("繋がりません");
    expect(nameOf(sub)).toBe("（上流は名乗っていません）");
    expect(cardRows(sub)).toEqual([
      ["起こし方", "動いているサーバに繋ぐ"],
      ["宛先", SUB_ENDPOINT],
      ["起こした上流の状態", "起こしていません（外の上流に依頼しています）"],
      ["最後の確認", "10:03:30 に確かめました（繋がりません）"],
      ["コンテキスト長", "読めませんでした"],
    ]);
  });

  /**
   * **まだ照会していない面は、時刻を言わず、「読めませんでした」とも言わない**（作業単位 B-66 / Issue origin#94 のコメント1）。
   * 本文だけをまだ照会していない形にして、裏方（答えた）の行が動かないことも見る。
   * **印・名前・「最後の確認」・長さを「まだ確かめていません」の1つの言い方に揃えた**（レビュー是正）。
   */
  it("まだ照会していない面は、印・名前・最後の確認・長さで「まだ確かめていません」と言う", async () => {
    upstream = report(NOT_CHECKED, {});
    await open();
    const main = cardNamed(nowSection(), MAIN_HEADING);
    expect(flat(badgeOf(main))).toBe("まだ確かめていません");
    expect(nameOf(main)).toBe("まだ確かめていません");
    expect(cardRows(main).filter((row) => row[0] === "最後の確認" || row[0] === "コンテキスト長")).toEqual([
      ["最後の確認", "まだ確かめていません"],
      ["コンテキスト長", "まだ確かめていません"],
    ]);
    expect(cardRows(cardNamed(nowSection(), SUB_HEADING)).find((row) => row[0] === "最後の確認")).toEqual([
      "最後の確認",
      "10:03:30 に確かめました（答えました）",
    ]);
  });

  /**
   * **届いたが一覧が読めない外の上流は、中立の印で「一覧に答えません」と言い、帯にしない**（レビュー是正 /
   * ADR-0038 §決定1 —— 一覧は上流に「任意」の能力）。裏方の側に置き、本文の側（答えた）は動かない。
   */
  it("一覧に答えない外の上流は、中立の印で言い、帯にしない", async () => {
    upstream = report({}, NO_MODEL_LIST);
    await open();
    expect(badges()).toEqual(["答えています", "一覧に答えません"]);
    const sub = cardNamed(nowSection(), SUB_HEADING);
    expect(badgeOf(sub).className).not.toContain("warn");
    expect(cardRows(sub).find((row) => row[0] === "最後の確認")).toEqual([
      "最後の確認",
      "10:03:30 に確かめました（一覧に答えません）",
    ]);
    expect(bands()).toEqual([]);
  });

  /**
   * **鍵を暗号化せずに送っている面のカードに、送り方の印の行が出る**（作業単位 A-13b-3）。
   * 本文はローカルネットワーク・裏方は承認済みにして、**その面のカードにだけ、その面の文面が出る**（帰属）。
   * **欄の無い面には行ごと出さない**（2つ目の画面で対照）。
   */
  it("鍵の送り方の印は、その面のカードにだけ、その面の文面で出る", async () => {
    upstream = report({ key_over_plain_http: "local_network" }, { key_over_plain_http: "approved" });
    await open();
    expect(
      [MAIN_HEADING, SUB_HEADING].map((heading) =>
        cardRows(cardNamed(nowSection(), heading)).find((row) => row[0] === "鍵の送り方"),
      ),
    ).toEqual([
      ["鍵の送り方", "鍵を暗号化せずに送っています（宛先はローカルネットワーク）"],
      ["鍵の送り方", "鍵を暗号化せずに送っています（承認済み。宛先はローカルネットワークの外）"],
    ]);
    // **暗号化せずに送っているだけでは、帯を出さない。**
    expect(bands()).toEqual([]);
    await unmount(views.pop() as ReturnType<typeof mount>, { outro: false });

    upstream = report({ key_over_plain_http: "local_network" }, {});
    await open();
    expect(cardRows(cardNamed(nowSection(), SUB_HEADING)).map((row) => row[0])).not.toContain("鍵の送り方");
  });

  /** **名乗った名前は等幅、名乗らなかった理由は等幅にしない**（綴りに見えないように）。 */
  it("名乗った名前だけを等幅で出す", async () => {
    upstream = report({}, SILENT);
    await open();
    const main = cardNamed(nowSection(), MAIN_HEADING).querySelector(":scope > p");
    const sub = cardNamed(nowSection(), SUB_HEADING).querySelector(":scope > p");
    expect(main?.classList.contains("font-mono")).toBe(true);
    expect(sub?.classList.contains("font-mono")).toBe(false);
  });
});

describe("状態の印", () => {
  /**
   * **本文の面だけを各状態にし、裏方は外の上流が答えている形に固定する**（面を取り違えると、裏方の側が動く）。
   * 期待値は直書き。
   */
  it.each([
    ["準備完了", { launch: READY }, "動いています"],
    ["外の上流が答えている", { answered_by: "openai-sole-entry" }, "答えています"],
    ["起こしていない", { ...SILENT, launch: NOT_STARTED }, "起こしていません"],
    ["載せている", { ...SILENT, launch: LOADING }, "載せています"],
    ["失敗した", { ...SILENT, launch: FAILED }, "失敗しました"],
    ["止めた", { ...SILENT, launch: STOPPED }, "止めました"],
    ["外の上流が答えない", SILENT, "繋がりません"],
    ["外の上流にまだ照会していない", NOT_CHECKED, "まだ確かめていません"],
    ["準備完了で名乗らない", { ...SILENT, launch: READY }, "名乗りがありません"],
    ["鍵を断られた", REFUSED, "鍵を断られました"],
    ["知らない綴り", { launch: launch({ state: "sleeping" }) }, "sleeping"],
  ] as const)("本文が%s → 本文の印だけが変わる", async (_name, main, badge) => {
    upstream = report(main, {});
    await open();
    expect(badges()).toEqual([badge, "答えています"]);
  });

  /** **裏方の側でも同じ**（本文の面だけを見る変異を止める）。 */
  it("裏方だけが載せていれば、裏方の印だけが「載せています」", async () => {
    upstream = report({ launch: READY }, { ...SILENT, launch: LOADING });
    await open();
    expect(badges()).toEqual(["動いています", "載せています"]);
  });

  /**
   * **印の色は、壊れている側（暖色）・読む側（寒色）・色なしの3つに分かれる。**
   * **名乗りが無い面は色なし**（異常と言わない —— 照会していない面と区別できない）。
   * 見るのは当てたユーティリティの綴り（単体テストは Tailwind を走らせない / ADR-0040 の追記）。
   */
  it("異常は暖色、動いている・載せているは寒色、起こしていない・名乗りが無いは色を持たない", async () => {
    upstream = report({ ...SILENT, launch: FAILED }, { launch: READY });
    await open();
    const failed = badgeOf(cardNamed(nowSection(), MAIN_HEADING)).className;
    const running = badgeOf(cardNamed(nowSection(), SUB_HEADING)).className;
    expect(failed).toContain("text-warn");
    expect(failed).not.toContain("read");
    expect(running).toContain("text-read");
    expect(running).not.toContain("warn");
    await unmount(views.pop() as ReturnType<typeof mount>, { outro: false });

    upstream = report({ ...SILENT, launch: LOADING }, { ...SILENT, launch: NOT_STARTED });
    await open();
    const loading = badgeOf(cardNamed(nowSection(), MAIN_HEADING)).className;
    const waiting = badgeOf(cardNamed(nowSection(), SUB_HEADING)).className;
    expect(loading).toContain("text-read");
    expect(loading).not.toContain("warn");
    expect(waiting).not.toContain("read");
    expect(waiting).not.toContain("warn");
    await unmount(views.pop() as ReturnType<typeof mount>, { outro: false });

    // **まだ照会していない外の上流・名乗りが無い準備完了の面は色なし**（作業単位 B-66 で本文の側を `NOT_CHECKED` へ替えた）。
    // 対照に、鍵を断られた面・答えない外の上流は暖色。
    upstream = report(NOT_CHECKED, { ...SILENT, launch: READY });
    await open();
    for (const [heading, text] of [
      [MAIN_HEADING, "まだ確かめていません"],
      [SUB_HEADING, "名乗りがありません"],
    ] as const) {
      const quiet = badgeOf(cardNamed(nowSection(), heading));
      expect(flat(quiet), heading).toBe(text);
      expect(quiet.className, heading).not.toContain("read");
      expect(quiet.className, heading).not.toContain("warn");
    }
    await unmount(views.pop() as ReturnType<typeof mount>, { outro: false });

    upstream = report(REFUSED, SILENT);
    await open();
    expect(badgeOf(cardNamed(nowSection(), MAIN_HEADING)).className).toContain("text-warn");
    expect(badgeOf(cardNamed(nowSection(), SUB_HEADING)).className).toContain("text-warn");
  });
});

describe("異常の帯", () => {
  it("失敗した本文と、鍵を断られた裏方に、面を名乗った帯が1本ずつ出る", async () => {
    upstream = report({ ...SILENT, launch: FAILED }, REFUSED);
    await open();
    expect(bands()).toEqual([
      [MAIN_HEADING, "載せられませんでした。理由は、この面のカードの「起こした上流の状態」にあります。"],
      [SUB_HEADING, REFUSED_BAND, `宛先: ${SUB_ENDPOINT}`],
    ]);
  });

  /**
   * **承認が無くて鍵を送らなかった面にも帯が出る**（作業単位 A-13b-3 / `key_over_plain_http` の `not_approved`）。
   * **断られた面とは別の文面**で、面と宛先を名乗る。印も「鍵を送っていません」になる。
   */
  it("鍵を送らなかった本文と、鍵を断られた裏方に、別の文面の帯が1本ずつ出る", async () => {
    upstream = report(WITHHELD, REFUSED);
    await open();
    expect(badges()).toEqual(["鍵を送っていません", "鍵を断られました"]);
    expect(bands()).toEqual([
      [MAIN_HEADING, WITHHELD_BAND, `宛先: ${MAIN_ENDPOINT}`],
      [SUB_HEADING, REFUSED_BAND, `宛先: ${SUB_ENDPOINT}`],
    ]);
  });

  /**
   * **裏方が本文と同じ宛先で、同じ理由なら、帯は1本**（作業単位 A-13b-3 のレビュー是正 / 司令塔が画面で見た2本並び）。
   * 面の名前は両方言う。
   */
  it("両方の面が同じ宛先で鍵を送っていなければ、帯は1本で、両方の面を名乗る", async () => {
    upstream = report({ ...WITHHELD, endpoint: MAIN_ENDPOINT }, { ...WITHHELD, endpoint: MAIN_ENDPOINT });
    await open();
    expect(bands()).toEqual([[`${MAIN_HEADING}・${SUB_HEADING}`, WITHHELD_BAND, `宛先: ${MAIN_ENDPOINT}`]]);
  });

  /**
   * **帯が名指す直す場所が、同じ画面に在る**（作業単位 A-13b-3）。帯は「下の「upstreams.json に書いたプロファイル」の
   * 「直す」」と言うので、その見出しの節が同じ画面に在ること（見出しを変えると、帯だけが黙って嘘になる）。
   */
  it("帯が名指す節が、同じ画面に在る", async () => {
    upstream = report(WITHHELD, REFUSED);
    await open();
    const named = [...new Set(bands().flatMap((band) => [...(band[1] ?? "").matchAll(/「([^」]+)」の「直す」/gu)].map((m) => m[1])))];
    expect(named).toEqual(["upstreams.json に書いたプロファイル"]);
    const headings = [...target.querySelectorAll("h2")].map((h2) => flat(h2));
    expect(headings).toContain(named[0]);
  });

  /**
   * **止めた本文には帯、まだ照会していない裏方には帯なし**（作業単位 B-66 で裏方を `SILENT` から `NOT_CHECKED` へ替えた）。
   */
  it("止めた本文には帯が出て、まだ照会していない裏方には出ない", async () => {
    upstream = report({ ...SILENT, launch: STOPPED }, NOT_CHECKED);
    await open();
    expect(bands()).toEqual([[MAIN_HEADING, "この実行ファイルが終わるときに止めました。"]]);
  });

  /**
   * **照会したが答えない外の上流には、帯が出る**（作業単位 B-66 / Issue origin#94 の決定4）。
   * 確かめた時刻（読み手の時間帯 / レビュー是正で経過から替えた）と、宛先を言う。**裏方の側だけ**にして、面の帰属も見る。
   */
  it("答えない外の上流には、確かめた時刻と宛先を言う帯が出る", async () => {
    upstream = report({}, SILENT);
    await open();
    expect(bands()).toEqual([[SUB_HEADING, NO_ANSWER_BAND, `宛先: ${SUB_ENDPOINT}`]]);
  });

  /**
   * **異常でなければ1本も出さない**（常に出す変異を止める）。容れ物は在る（`bands` が確かめる）。
   * **まだ照会していない外の上流・名乗りが無い準備完了の面も帯なし。**
   */
  it.each([
    ["両面とも答えている", {}, {}],
    ["準備完了と外の上流", { launch: READY }, {}],
    ["載せている・起こしていない", { ...SILENT, launch: LOADING }, { ...SILENT, launch: NOT_STARTED }],
    ["両面とも外の上流でまだ照会していない", NOT_CHECKED, NOT_CHECKED],
    ["準備完了で名乗りが無い", { ...SILENT, launch: READY }, { ...SILENT, launch: READY }],
  ] as const)("%s → 帯なし", async (_name, main, sub) => {
    upstream = report(main, sub);
    await open();
    expect(bands()).toEqual([]);
  });

  /** **帯は読むだけ** —— 押せるもの・入力欄を足さない（観測の節には「起こす」と「再試行」しか置かない）。 */
  it("帯の中に押せるものが無い", async () => {
    upstream = report({ ...SILENT, launch: FAILED }, REFUSED);
    await open();
    const holder = nowSection().querySelector('[aria-label="上流の異常"]') as Element;
    expect(holder.querySelectorAll("button, input, select, textarea, form, a")).toHaveLength(0);
  });

  /** **載せている → 失敗**に読み直しで移ったら、帯が現れる（「A が B になる」—— A で帯が無いことを先に確かめる）。 */
  it("読み直しで失敗に移ったら、その面の帯が現れる", async () => {
    upstream = report({ ...SILENT, launch: LOADING }, {});
    await open();
    expect(bands()).toEqual([]);
    upstream = report({ ...SILENT, launch: FAILED }, {});
    await vi.waitFor(() => expect(bands().map((band) => band[0])).toEqual([MAIN_HEADING]));
  });
});

/** 照らし合わせの見出しの文面。 */
function summaryText(): string {
  return flat(comparison().querySelector("summary"));
}

/**
 * **観測の口への読みを、これから n 回待つ。** 待ち終えたとき、それより前の読みの応答は描画まで済んでいる
 * （読み直しは、前の応答を描いた後に次の1回が予約される）。
 */
async function readsMore(n: number): Promise<void> {
  let reads = 0;
  const original = globalThis.fetch;
  vi.stubGlobal("fetch", (input: unknown) => {
    if (String(input) === UPSTREAM_PATH) {
      reads += 1;
    }
    return original(input as RequestInfo);
  });
  await vi.waitFor(() => expect(reads).toBeGreaterThanOrEqual(n));
  vi.stubGlobal("fetch", original);
}

/** **一致しない行が本文 3 行**の突き合わせ（裏方の数とは違える）。 */
const MAIN_DRIFTS: MeasuredView = { ...MAIN_AGREES, matched: 2, drifted_absent: 3 };

describe("照らし合わせ", () => {
  it("一致しない行が無ければ、閉じたまま、そう言う見出しで出す（突き合わせの表はその中）", async () => {
    await open();
    const details = comparison();
    expect(details.open).toBe(false);
    expect(summaryText()).toBe("照らし合わせの詳細 —— 一致しない行はありません");
    // **表と注記は、照らし合わせの中に在る**（畳む対象）。
    const table = details.querySelector("table");
    expect(flat(table?.querySelector("caption") ?? null)).toBe("上流に聞いた値");
    expect(flat(details)).toContain("見た行は、その下の4つの和");
  });

  /**
   * **一致しない行があっても閉じたまま。数は見出しが面ごとに言う**
   * （片方の面からだけ数を取る変異も、面を取り違える変異も、数が入れ替わって赤になる）。
   */
  it.each([
    ["本文に別のモデルが載っている", { measured: { ...MAIN_AGREES, matched: 4, drifted_other: 1 } }, {}, "本文 1 行・裏方 0 行"],
    ["裏方の綴りが一覧に無い", {}, { measured: { ...SUB_AGREES, matched: 0, drifted_absent: 1 } }, "本文 0 行・裏方 1 行"],
    ["両方ずれている", { measured: MAIN_DRIFTS }, { measured: { ...SUB_AGREES, matched: 0, drifted_absent: 1 } }, "本文 3 行・裏方 1 行"],
  ] as const)("%s → 閉じたまま、見出しが数を言う", async (_name, main, sub, counts) => {
    upstream = report(main, sub);
    await open();
    expect(comparison().open).toBe(false);
    expect(summaryText()).toBe(`照らし合わせの詳細 —— 一致しない行があります（${counts}）`);
  });

  /**
   * **読み直しで数が更新される**（「A が B になる」—— A の数を先に確かめる）。
   * 裏方を載せている形にして、読み直しを続けさせる。**ずれが現れても、消えても、開かない。**
   */
  it("読み直しで見出しの数が更新され、開きはしない", async () => {
    upstream = report({}, { ...SILENT, launch: LOADING });
    await open();
    expect(summaryText()).toBe("照らし合わせの詳細 —— 一致しない行はありません");

    upstream = report({ measured: MAIN_DRIFTS }, { ...SILENT, launch: LOADING });
    await vi.waitFor(() =>
      expect(summaryText()).toBe("照らし合わせの詳細 —— 一致しない行があります（本文 3 行・裏方 0 行）"),
    );
    expect(comparison().open).toBe(false);

    upstream = report(
      { measured: { ...MAIN_AGREES, matched: 4, drifted_other: 1 } },
      { ...SILENT, launch: LOADING, measured: { ...SUB_AGREES, matched: 0, drifted_absent: 1 } },
    );
    await vi.waitFor(() =>
      expect(summaryText()).toBe("照らし合わせの詳細 —— 一致しない行があります（本文 1 行・裏方 1 行）"),
    );
    expect(comparison().open).toBe(false);
  });

  /**
   * **「上流が答えませんでした」の注記は、聞いたのに答えなかった面のことだと言い、まだ聞いていない面の文面を名指す**
   * （作業単位 B-66 / Issue origin#94）。
   *
   * > **⚠ 作業単位 B-66 で主張が変わった。** 着手前の名前は「答えなかった面の注記は、照会していない場合もあると言う」で、
   * > 「コンテキスト長を設定した面には聞きません」を含むことを見ていた（D-12b-2 の2周目）。**いまは書いた面にも照会する**ので、
   * > その一文は偽になった。照会していない面は別の文面（「まだ確かめていません」/ レビュー是正で揃えた）で出る。
   */
  it("答えなかった面の注記は、聞いたのに答えなかった面のことで、まだ確かめていない面は別の文面だと言う", async () => {
    await open();
    const note = [...comparison().querySelectorAll("p")].map((p) => flat(p)).find((text) =>
      text.startsWith("「上流が答えませんでした」と出ている面は"),
    );
    expect(note, "答えなかった面の注記があること").toBeDefined();
    expect(note).toContain("まだ確かめていない面には「まだ確かめていません」と出ます");
    expect(note).not.toContain("コンテキスト長を設定した面には聞きません");
  });

  /**
   * **開閉は利用者の操作だけで変わる。** 利用者が開いたら、読み直しが続いても開いたまま。閉じたら閉じたまま。
   * 読み直しの間に「ずれがある ↔ ない」を行き来させる（開き方を数から決め直す実装を、どちらの向きでも止める）。
   */
  it("利用者が開けば開いたまま、閉じれば閉じたまま（読み直しでは変わらない）", async () => {
    upstream = report({ measured: MAIN_DRIFTS }, { ...SILENT, launch: LOADING });
    await open();
    expect(comparison().open).toBe(false);

    comparison().open = true;
    upstream = report({}, { ...SILENT, launch: LOADING });
    await readsMore(3);
    expect(summaryText()).toBe("照らし合わせの詳細 —— 一致しない行はありません");
    expect(comparison().open).toBe(true);

    upstream = report({ measured: MAIN_DRIFTS }, { ...SILENT, launch: LOADING });
    await readsMore(3);
    expect(summaryText()).toContain("一致しない行があります");
    expect(comparison().open).toBe(true);

    comparison().open = false;
    upstream = report({}, { ...SILENT, launch: LOADING });
    await readsMore(3);
    expect(summaryText()).toBe("照らし合わせの詳細 —— 一致しない行はありません");
    expect(comparison().open).toBe(false);

    upstream = report({ measured: MAIN_DRIFTS }, { ...SILENT, launch: LOADING });
    await readsMore(3);
    expect(summaryText()).toContain("一致しない行があります");
    expect(comparison().open).toBe(false);
  });
});

/** **「いま確かめる」ボタン**（観測の節の中に1つだけ）。 */
function checkButton(): HTMLButtonElement {
  const found = [...nowSection().querySelectorAll("button")].filter((button) => flat(button) === "いま確かめる");
  expect(found, "「いま確かめる」が観測の節に1つだけあること").toHaveLength(1);
  return found[0] as HTMLButtonElement;
}

describe("いま確かめる（作業単位 B-66 / Issue origin#94 の決定3・4）", () => {
  /**
   * **押すと `POST /admin/upstream/check` を1本送り、応答が着くまでボタンを無効にし、着いた観測で差し替える**
   * （「A が B になる」—— 押す前に A（裏方が繋がらない・帯がある）を確かめる）。
   *
   * 応答は手で着かせる（着く前の無効を見るため）。**応答の裏方だけを「答えた」に替える**ので、
   * 本文の側を書き換える・観測を差し替えない変異はどれも赤になる。
   */
  it("押すと確かめ、着くまで押せず、着いた観測でカードと帯を差し替える", async () => {
    upstream = report({}, SILENT);
    await open();
    expect(badges()).toEqual(["答えています", "繋がりません"]);
    expect(bands().map((band) => band[0])).toEqual([SUB_HEADING]);
    expect(checkButton().disabled).toBe(false);

    let arrive: (response: Response) => void = () => undefined;
    checkAnswer = () =>
      new Promise((resolve) => {
        arrive = resolve;
      });
    checkButton().click();
    await vi.waitFor(() => expect(checkButton().disabled).toBe(true));
    expect(checks).toBe(1);
    // **無効の間は押しても2本目を送らない。**
    checkButton().click();
    expect(checks).toBe(1);

    const answered = report({}, {});
    arrive(json(answered));
    await vi.waitFor(() => expect(badges()).toEqual(["答えています", "答えています"]));
    expect(bands()).toEqual([]);
    expect(checkButton().disabled).toBe(false);
  });

  /**
   * **失敗したら、既存の作法でエラーを出す**（`role="alert"`・サーバの文面をそのまま）。**観測は消さない**
   * （カードはそのまま）。ボタンはまた押せる。
   */
  it("失敗したらサーバの文面でエラーを出し、観測はそのまま、また押せる", async () => {
    upstream = report({}, SILENT);
    await open();
    checkAnswer = () =>
      Promise.resolve(
        json({ error: { message: "推論サーバとの通信で問題が起きました", type: "server_error" } }, 500),
      );
    checkButton().click();
    await vi.waitFor(() => {
      const alert = [...nowSection().querySelectorAll('[role="alert"]')].map((element) => flat(element));
      expect(alert).toContain("確かめられませんでした: 推論サーバとの通信で問題が起きました");
    });
    expect(faceCards(nowSection())).toHaveLength(2);
    expect(badges()).toEqual(["答えています", "繋がりません"]);
    expect(checkButton().disabled).toBe(false);
  });
});
