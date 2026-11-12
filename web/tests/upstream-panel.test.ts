// @vitest-environment jsdom
/**
 * **上流を見る画面**（作業単位 D-9 / `src/lib/UpstreamPanel.svelte`）。
 *
 * # なぜこの画面が要るのか
 *
 * **着手前、画面に「モデル」に見えるものは1つしか無かった** ——
 * 会話の画面の `送り先: aoi@1`。**それはセッションの識別子**であって、
 * **本文を書いているモデルの綴りではない。**
 * **`GET /v1/models` が返すのもキャラクター**なので、
 * **画面のどこにも上流のモデルは出ていなかった。**
 *
 * # ⚠ 見るのは `<td>` のセルであって、`data-*` ではない
 *
 * **`data-*` はテストが置いた印であって、読み手が何を見るかの権威ではない**
 * （CLAUDE.md / `D-4` の実測 —— `data-ups` / `data-downs` を見ていたせいで、
 * **セルを入れ替える変異が 372 全緑を通った**）。
 * **行のセルは配列で突き合わせる。**
 * **行全体の `textContent` に `toContain` を当てるのは、列の順番を一切見ていない。**
 *
 * # ⚠ フィクスチャは、2つの面で1つも同じ値にしない
 *
 * 同値だと、**面を取り違える変異が観測できない**（CLAUDE.md の最頻の穴）。
 *
 * # ⚠ ここは「上流が申告した窓」であって「採用した予算」ではない
 *
 * **2つが食い違う状態は正常にありうる**（引き直しが失敗した回は、
 * 予算は現在値を保ち、申告値は「答えなし」になる）。
 * **画面がそう読ませていないこと**を、注記の文面で見る。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import UpstreamPanel from "../src/lib/UpstreamPanel.svelte";
import type { ProfilesView, UpstreamReport } from "../src/lib/api";
import { labelFor } from "../src/lib/router";
import { readerInZone } from "./support/zone";

// **画面が `browserZone()` を訊く**（上流の状態の文面の時刻 / Issue origin#40）。
readerInZone("America/New_York");

/** 読む口。**`api.ts` の `getUpstream` が組むパス。** */
const UPSTREAM_PATH = "/admin/upstream";

/**
 * **書いたプロファイルの一覧の口**（作業単位 F-4b-3）。この画面は観測と一緒にこれも読む。
 *
 * **ここでは中身を見ない**（見るのは `tests/upstream-profiles-panel.test.ts`）。
 * 空の一覧を返し、観測の節だけを見る。
 */
const PROFILES_PATH = "/admin/upstream/profiles";

const EMPTY_LISTING: ProfilesView = {
  path: "C:\\Users\\someone\\AppData\\Local\\personacore\\upstreams.json",
  editable: true,
  state: "ok",
  revision: "rev",
  problem: null,
  profiles: [],
};

/**
 * **頼んだ名前と答えた名前が食い違っているときの一言**（作業単位 B-45）。
 *
 * **⚠ `upstream.ts` の定数から作らない** ——
 * **文面を緩める変異が、期待値も一緒に動かして緑になる**（CLAUDE.md）。
 */
const MISMATCH_NOTE = "頼んだ名前と違うものが答えています";

/**
 * **この画面の名前。** ナビのリンクも、会話の画面の案内も、この綴りを名乗る。
 *
 * **⚠ `labelFor` から作らない** —— 見出しと `labelFor` を両方差し替える変異が、
 * 期待値も一緒に動かして緑になる（CLAUDE.md）。
 */
const SCREEN_NAME = "上流モデル";

/**
 * サーバが返す観測。**2つの面で1欄も同じ値にしていない。**
 *
 * **裏方の側は `null` を持つ** —— 上流が黙っている状態を、
 * 値がある側と同じ画面で見比べるため。
 */
const REPORT: UpstreamReport = {
  main: {
    face: "main",
    endpoint: "http://127.0.0.1:1234/v1/chat/completions",
    model: "main-model-for-tests",
    // **頼んだ名前と違う綴りを名乗った上流**（作業単位 B-45 / `C-18` の実測）。
    // **同値にすると、食い違いの一言が出る枝に一度も入らない。**
    answered_name: "answering-model-for-tests.gguf",
    queried_context_tokens: 16384,
    answered_by: "native-api",
    unauthorized: false,
    checked_at: "2026-11-03T09:58:00+09:00",
    check_result: "answered",
    measured: {
      checked: 10,
      matched: 7,
      drifted_other: 1,
      drifted_absent: 2,
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
    // **名乗らなかった面**（`null`）。値がある側と見比べるため。
    answered_name: null,
    queried_context_tokens: null,
    answered_by: null,
    unauthorized: false,
    // **照会したが答えなかった面**（作業単位 B-66。「まだ聞いていない」面ではない）。
    checked_at: "2026-11-03T09:57:00+09:00",
    check_result: "no_answer",
    measured: {
      checked: 9,
      matched: 3,
      drifted_other: 4,
      drifted_absent: 0,
      unanswered: 2,
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

/** 画面に出るはずの表、**まるごと**（見出しの行を含む）。 */
const EXPECTED_TABLE: string[][] = [
  ["項目", "本文生成", "裏方（抽出・変容）"],
  ["応答が名乗った面", "main", "sub"],
  [
    "宛先",
    "http://127.0.0.1:1234/v1/chat/completions",
    "http://127.0.0.1:5678/v1/chat/completions",
  ],
  ["依頼するモデルの綴り", "main-model-for-tests", "sub-model-for-tests"],
  [
    "上流が答えたモデルの綴り",
    "answering-model-for-tests.gguf（頼んだ名前と違うものが答えています）",
    "（上流は名乗っていません）",
  ],
  ["上流が申告したコンテキスト長", "16384 トークン", "読めませんでした"],
  ["上流の答え方", "上流の独自 API が答えました", "上流が答えませんでした"],
  ["突き合わせ: 見た行", "10 行", "9 行"],
  ["突き合わせ: 一致", "7 行", "3 行"],
  ["突き合わせ: 別のモデルが載っている", "1 行", "4 行"],
  ["突き合わせ: 依頼する綴りが一覧に無い", "2 行", "0 行"],
  ["突き合わせ: 聞けなかった", "0 行", "2 行"],
];

let target: HTMLElement;
let urls: string[];
/** 次の応答（`null` なら失敗させる）。 */
let responds: UpstreamReport | null;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown) => {
    const url = String(input);
    urls.push(url);
    // **書いたプロファイルの一覧は、常に空で返す**（この測定は観測の節を見る）。
    const body = url === PROFILES_PATH ? EMPTY_LISTING : responds;
    if (body === null) {
      return Promise.reject(new TypeError("failed to fetch"));
    }
    return Promise.resolve({
      ok: true,
      status: 200,
      text: () => Promise.resolve(JSON.stringify(body)),
    } as unknown as Response);
  });
}

/** 観測の口へ送った読み（**書いたプロファイルの口は除く**）。 */
function upstreamReads(): string[] {
  return urls.filter((url) => url !== PROFILES_PATH);
}

/**
 * **観測の節**（見出し「いまの上流」の `<section>`）。
 *
 * 作業単位 F-4b-3 で、同じ画面に**書いたプロファイルの節**が並んだ。
 * **観測について言う検査は、この節の中だけを見る**（あちらの表やボタンを数えない）。
 */
function nowSection(): HTMLElement {
  const found = [...target.querySelectorAll("section")].filter(
    (section) => flat(section.querySelector(":scope > h2")) === "いまの上流",
  );
  expect(found, "観測の節が1つだけあること").toHaveLength(1);
  return found[0] as HTMLElement;
}

/**
 * **書いたプロファイルの節の外**にある、押せるもの・入力欄・選択肢（レビュー1周目 要修正1）。
 *
 * 選ぶ・足す・直すは書いたプロファイルの節（見出し「upstreams.json に書いたプロファイル」）に
 * だけ置く。**観測の節だけを見ると、`<h1>` と2つの節の間を誰も見ていない**
 * （見出しの直後に選択肢やボタンを足す変異 R05 / R06 が緑だった。`main` ではそれぞれ 2 赤）。
 * **だから「書いた節の外」を丸ごと見る。**
 */
const CONTROLS = "button, input, select, textarea, form, a";

function controlsOutsideWritten(): Element[] {
  const written = [...target.querySelectorAll("section")].filter(
    (section) => flat(section.querySelector(":scope > h2")) === "upstreams.json に書いたプロファイル",
  );
  expect(written, "書いたプロファイルの節が1つだけあること").toHaveLength(1);
  const inside = written[0] as HTMLElement;
  return [...target.querySelectorAll(CONTROLS)].filter((element) => !inside.contains(element));
}

/** 空白を畳んだ文面。**折り返しの位置に寄りかからない。** */
function flat(element: Element | null): string {
  return (element?.textContent ?? "").replace(/\s+/gu, " ").trim();
}

/**
 * 観測の表を、**行ごとのセルの配列**で読む（`data-*` は見ない）。
 *
 * **表は見出し（`<caption>`）で引く**（作業単位 F-4b-3 で、同じ画面に表が増えた）。
 */
function tableCells(): string[][] {
  const table = [...nowSection().querySelectorAll("table")].find(
    (found) => flat(found.querySelector("caption")) === "上流に聞いた値",
  );
  expect(table, "観測の表があること").toBeDefined();
  return [...(table as HTMLTableElement).querySelectorAll("tr")].map((row) =>
    [...row.querySelectorAll("th, td")].map((cell) => flat(cell)),
  );
}

async function open(): Promise<{ view: Record<string, unknown> }> {
  const view = mount(UpstreamPanel, { target });
  await vi.waitFor(() => {
    expect(nowSection().querySelector("table, p.error")).not.toBeNull();
  });
  return { view: view as Record<string, unknown> };
}

beforeEach(() => {
  urls = [];
  responds = REPORT;
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("上流の観測を出す", () => {
  it("`GET /admin/upstream` を1回だけ読む", async () => {
    const { view } = await open();
    await vi.waitFor(() => {
      expect(urls).toContain(PROFILES_PATH);
    });
    expect(upstreamReads()).toEqual([UPSTREAM_PATH]);
    // **ほかに読むのは、書いたプロファイルの一覧だけ**（作業単位 F-4b-3）。
    expect(urls.filter((url) => url !== UPSTREAM_PATH)).toEqual([PROFILES_PATH]);
    await unmount(view, { outro: false });
  });

  /**
   * **表を丸ごと突き合わせる。**
   *
   * **これが、面の取り違えと欄の取り違えの両方を捕まえる唯一の検査。**
   * 行ごとに `[見出し, 本文の面, 裏方の面]` を固定しているので、
   *
   * * `main` と `sub` を入れ替える変異 → **全行がずれる**
   * * `endpoint` と `model` を入れ替える変異 → **その2行がずれる**
   *
   * のどちらも赤になる。
   */
  it("表のセルが、行ごとに正しい列へ入る", async () => {
    const { view } = await open();
    expect(tableCells()).toEqual(EXPECTED_TABLE);
    await unmount(view, { outro: false });
  });

  /**
   * **列の見出しは、面の帰属そのもの。**
   *
   * 見出しだけを入れ替える変異は、上の表の突き合わせでも赤になるが、
   * **「どちらの列が本文か」を1本で名指ししておく。**
   */
  it("列の見出しが、本文と裏方をこの順で名乗る", async () => {
    const { view } = await open();
    const head = tableCells()[0];
    expect(head).toEqual(["項目", "本文生成", "裏方（抽出・変容）"]);
    await unmount(view, { outro: false });
  });

  /**
   * **申告した窓を、採用した予算と読ませない。**
   *
   * **2つが食い違う状態は正常にありうる**ので、
   * **画面が「いま使っている値」と名乗ってはいけない。**
   */
  it("申告値であって、いま使っている予算ではないと書いてある", async () => {
    const { view } = await open();
    const text = flat(target);
    expect(text).toContain("上流が申告した値");
    expect(text).toContain("いま使っている");
    await unmount(view, { outro: false });
  });

  /**
   * **食い違っているのは「どちらの面か」まで読めること**（作業単位 B-45）。
   *
   * # ⚠ 一言は、その値を持つセルの中に出す
   *
   * **本文だけが別物に答えられている状態は普通に起きる**ので、
   * **表の外に1つ置くと、どちらの面の話なのかが画面から消える**
   * （このリポジトリが繰り返し落としているのは「載っているか」ではなく**「どれのものか」**）。
   *
   * **画面全体で1回しか出ないこと**まで見る——表の外に足す実装はここで赤になる。
   */
  it("食い違っている面のセルにだけ、そのことが書いてある", async () => {
    const { view } = await open();

    const row = tableCells().find(
      (cells) => cells[0] === "上流が答えたモデルの綴り",
    );
    expect(row, "答えた名前の行があること").toBeDefined();
    expect(row?.[1]).toContain(MISMATCH_NOTE);
    expect(row?.[2]).not.toContain(MISMATCH_NOTE);

    const whole = flat(target);
    expect(whole.split(MISMATCH_NOTE).length - 1, "画面に1回だけ出ること").toBe(
      1,
    );

    await unmount(view, { outro: false });
  });

  /**
   * **同じなら、一言は出ない。**
   *
   * **常に出す実装も、常に出さない実装も赤にする**ため、
   * **違う腕（上の表）と同じ腕（ここ）の両方を回す。**
   */
  it("答えた名前が頼んだ名前と同じなら、その一言は出ない", async () => {
    responds = {
      main: { ...REPORT.main, answered_name: REPORT.main.model },
      sub: REPORT.sub,
      profile_sample: null,
      selected_profile: null,
    };

    const { view } = await open();

    const row = tableCells().find(
      (cells) => cells[0] === "上流が答えたモデルの綴り",
    );
    expect(row).toEqual([
      "上流が答えたモデルの綴り",
      "main-model-for-tests",
      "（上流は名乗っていません）",
    ]);
    expect(flat(target)).not.toContain(MISMATCH_NOTE);

    await unmount(view, { outro: false });
  });

  /**
   * **`answered_name` の `null` は、2つの事実を潰している**（レビュー是正 / 2026-09-17）。
   *
   * | `answered_name` | `answered_by` | 起きていること |
   * |---|---|---|
   * | `null` | `null` | **往復そのものが失敗した／一覧が空。** 上流は黙っている |
   * | `null` | 値がある | **一覧は読めたが、どれが答えたかを決められない** |
   *
   * **後者で「上流は名乗っていません」と出すのは偽**——上流は現に一覧を出している。
   *
   * **2つを同じ画面に並べて描く**ので、`answered_by` を見ない実装
   * （どちらも「名乗っていません」になる）も、2つを取り違える実装も赤になる。
   */
  it("名指しできない面と、黙っている面が、別の文面で並ぶ", async () => {
    responds = {
      // **一覧は読めた**（`answered_by` に値がある）が、名指しはできなかった面。
      main: {
        ...REPORT.main,
        answered_name: null,
        answered_by: "openai-catalogue-only",
      },
      // **黙っている面**（`answered_by` も `null`）。
      sub: REPORT.sub,
      profile_sample: null,
      selected_profile: null,
    };

    const { view } = await open();

    const row = tableCells().find(
      (cells) => cells[0] === "上流が答えたモデルの綴り",
    );
    expect(row).toEqual([
      "上流が答えたモデルの綴り",
      "上流は一覧を出しましたが、どれが答えたかは決められません",
      "（上流は名乗っていません）",
    ]);
    // **どちらの面も名乗っていない**ので、食い違いの一言は出ない。
    expect(flat(target)).not.toContain(MISMATCH_NOTE);

    await unmount(view, { outro: false });
  });

  /** **`checked` は内訳の和である**と画面が言う（「全部一致」と読ませない）。 */
  it("見た行が内訳の和であることが書いてある", async () => {
    const { view } = await open();
    expect(flat(target)).toContain("見た行は、その下の4つの和");
    await unmount(view, { outro: false });
  });
});

/**
 * **この起動が見本を書いた先**（作業単位 F-4c / Issue origin#23）。
 *
 * **Windows の形の綴り**——区切りの `\\` が、画面で落ちたり化けたりしないことまで見る。
 */
const SAMPLE_PATH =
  "C:\\Users\\someone\\AppData\\Local\\personacore\\upstreams.json";

/**
 * **見本を置いたときの一言**、まるごと。
 *
 * **⚠ 画面の側から作らない**（文面を緩める変異が、期待値も一緒に動かして緑になる）。
 */
const SAMPLE_NOTICE =
  `上流プロファイルの見本を ${SAMPLE_PATH} に置きました。` +
  // **画面で選べるようになった**（作業単位 F-4b-3）。**画面に在る操作の名前で案内する。**
  "下の一覧の「直す」で gguf のパスを自分の機のものに書き換えてから、" +
  "「このプロファイルに切り替える」で選べます。" +
  "起動するときに --upstream <名前> を付けても選べます。" +
  // **`start.ps1` の綴りも言う。ただし、このファイルを読まないことまで言う**
  // （`-Upstream` は `PERSONACORE_UPSTREAMS` を `scripts/` の下へ向ける / レビュー1周目の提案3）。
  "scripts/start.ps1 の -Upstream <名前> は、このファイルではなく scripts/upstreams.json" +
  "（無ければ scripts/upstreams.sample.json）を読みます。";

describe("上流プロファイルの見本（作業単位 F-4c）", () => {
  /**
   * **この起動が見本を書いたときだけ、その場所と次の一手を言う。**
   *
   * **一言まるごとを突き合わせる**——`toContain(パス)` だけだと、
   * 「次に何をするか」を落とす変異も、パスを別の文に載せる変異も通る。
   */
  it("見本を書いた起動では、置いた場所と次の一手を言う", async () => {
    responds = { ...REPORT, profile_sample: SAMPLE_PATH };
    const { view } = await open();

    const notices = [...target.querySelectorAll("p.sample-notice")].map((p) =>
      flat(p),
    );
    expect(notices).toEqual([SAMPLE_NOTICE]);
    // **観測の節の中では1回だけ**（表の外に2つ目を足す形を止める）。
    //
    // **⚠ 作業単位 F-4b-3 で「画面全体」から絞った。** 見本を書く先は既定の置き場所で、
    // **書いたプロファイルの節も「書く先: <同じパス>」と言う**（同じファイルなので正しい）。
    // その1回はあちらの節の見張り番（`tests/upstream-profiles-panel.test.ts`）が見る。
    expect(flat(nowSection()).split(SAMPLE_PATH).length - 1).toBe(1);
    expect(flat(target).split(SAMPLE_PATH).length - 1).toBe(2);

    await unmount(view, { outro: false });
  });

  /**
   * **書いていない起動では、何も言わない**（既に在った・別の置き場所を指した）。
   *
   * **上と対にする**——片側だけだと「常に言う」「常に言わない」の片方が通る。
   */
  it("見本を書いていない起動では、見本の話をしない", async () => {
    const { view } = await open();

    expect(target.querySelectorAll("p.sample-notice").length).toBe(0);
    expect(flat(target)).not.toContain("見本");

    await unmount(view, { outro: false });
  });

  /**
   * **一言は案内だけで、押せるものを足さない**（押す先は一覧の側にある）。
   *
   * **⚠ 作業単位 F-4b-3 で意味を変えた。** 着手前は「画面全体に押せるものが無い」を見ていたが、
   * 画面に選ぶ・直す操作が入ったので、**一言の中と、書いたプロファイルの節の外**に絞った。
   */
  it("見本の一言は、押せるものを足さない", async () => {
    responds = { ...REPORT, profile_sample: SAMPLE_PATH };
    const { view } = await open();

    const notice = target.querySelector("p.sample-notice");
    expect(notice).not.toBeNull();
    expect(
      (notice as Element).querySelectorAll("button, input, select, textarea, form, a").length,
    ).toBe(0);
    // **書いた節の外に在るのは「いま確かめる」だけ**（作業単位 B-66 で足した。一言は何も足さない）。
    expect(
      controlsOutsideWritten().map((element) => `${element.tagName}:${flat(element)}`),
    ).toEqual(["BUTTON:いま確かめる"]);

    await unmount(view, { outro: false });
  });

  /**
   * **一言が名乗る操作は、同じ画面に在る**（画面に無い操作を名乗らない）。
   *
   * 見本を書くのは既定の置き場所（画面から直せる置き場所）だけなので、
   * 一覧が書ける置き場所を返すこの形が、見本を書いた起動の形である。
   */
  it("見本の一言が名乗る操作は、同じ画面にボタンとして在る", async () => {
    responds = { ...REPORT, profile_sample: SAMPLE_PATH };
    const listed: ProfilesView = {
      ...EMPTY_LISTING,
      profiles: [
        {
          name: "sample",
          main: {
            endpoint: "http://127.0.0.1:8080",
            model: "m",
            gguf: "~/x.gguf",
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
        },
      ],
    };
    vi.stubGlobal("fetch", (input: unknown) => {
      const url = String(input);
      urls.push(url);
      return Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(JSON.stringify(url === PROFILES_PATH ? listed : responds)),
      } as unknown as Response);
    });
    const { view } = await open();
    await vi.waitFor(() => {
      expect(target.querySelector("section.profile")).not.toBeNull();
    });

    const named = [...flat(target.querySelector("p.sample-notice")).matchAll(/「([^」]+)」/gu)].map(
      (found) => found[1],
    );
    expect(named).toEqual(["直す", "このプロファイルに切り替える"]);
    const buttons = [...target.querySelectorAll("section.profile button")].map((button) => flat(button));
    for (const name of named) {
      expect(buttons, name).toContain(name);
    }

    await unmount(view, { outro: false });
  });
});

/**
 * **書いたプロファイルの節の外は、読むだけ**（作業単位 F-4b-3 で意味を変えた）。
 *
 * # ⚠ 着手前は「画面全体が読むだけ」を見ていた
 *
 * 2026-09-07 の「モデルは見えるだけでよい。選ばせもしない」は、
 * **切り替えが安くなったので ADR-0042 §決定5 で置き換わった**（選べるのは自分で書いたプロファイルだけ）。
 * **選ぶ・足す・直すは書いたプロファイルの節にだけ置く。** それ以外の場所
 * （見出し・冒頭の注記・観測の節）に押せるものを置かないことは残す。
 *
 * **⚠ レビュー1周目で範囲を直した。** 1周目は「観測の節の中」だけを見ていて、
 * **`<h1>` と2つの節の間に選択肢やボタンを足す変異（R05 / R06）が緑だった。**
 */
describe("書いたプロファイルの節の外は読むだけ", () => {
  /**
   * > **⚠ 作業単位 B-66 で主張が変わった**（Issue origin#94 の決定4）。着手前の名前は
   * > 「読めたときに、書いた節の外に押せるもの・入力欄・選択肢が1つも無い」で、`[]` を見ていた。
   * > **いまは「いま確かめる」が1つだけ在る**——上流へ照会を頼むだけで、観測を書き換える操作ではない
   * > （照会の中身は5分ごとと同じ）。それ以外（入力欄・選択肢・別のボタン）が無いことは変えていない。
   */
  it("読めたときに、書いた節の外で押せるのは「いま確かめる」だけ", async () => {
    const { view } = await open();
    await vi.waitFor(() => {
      expect(urls).toContain(PROFILES_PATH);
    });
    expect(
      controlsOutsideWritten().map((element) => `${element.tagName}:${flat(element)}`),
    ).toEqual(["BUTTON:いま確かめる"]);
    await unmount(view, { outro: false });
  });

  /** **読めなかったときに、書いた節の外で押せるのは「再試行」だけ。** */
  it("読めなかったときに、書いた節の外で押せるのは「再試行」だけ", async () => {
    responds = null;
    const { view } = await open();
    expect(
      controlsOutsideWritten().map((element) => `${element.tagName}:${flat(element)}`),
    ).toEqual(["BUTTON:再試行"]);
    await unmount(view, { outro: false });
  });

  /**
   * **本文の面を起こしていないときに、書いた節の外で押せるのは「起こす」だけ**（作業単位 F-10b /
   * ADR-0055 §決定3 —— 状態を言う節で、起こすかを聞く）。**観測を書き換える操作ではない**
   * （起こすのは、選ばれているプロファイルと同じ名前の差し替え）。
   * 起こしていない以外の状態では、上の1本目のとおり何も無い（対照）。
   */
  // **作業単位 B-66 で「いま確かめる」が並んだ**（名前の「だけ」は「起こす」と「いま確かめる」の2つ）。
  it("本文の面を起こしていないときに、書いた節の外で押せるのは「起こす」だけ", async () => {
    const waiting = { state: "not_started", since: null, answered_name: null, reason: null, log: null };
    responds = {
      ...REPORT,
      main: { ...REPORT.main, launch: waiting },
      sub: { ...REPORT.sub, launch: waiting },
      selected_profile: "beta",
    };
    const { view } = await open();
    await vi.waitFor(() => {
      expect(urls).toContain(PROFILES_PATH);
    });
    expect(
      controlsOutsideWritten().map((element) => `${element.tagName}:${flat(element)}`),
    ).toEqual(["BUTTON:いま確かめる", "BUTTON:「beta」を起こす"]);
    await unmount(view, { outro: false });
  });
});

describe("読めなかったとき", () => {
  it("失敗を出して、再試行できる", async () => {
    responds = null;
    const { view } = await open();

    const error = nowSection().querySelector("p.error");
    expect(error, "失敗が画面に出ること").not.toBeNull();
    expect(flat(error)).toContain("上流の様子を取得できませんでした");
    // **表は出さない**（読めていないのに、読めた形で見せない）。
    expect(nowSection().querySelector("table")).toBeNull();

    const retry = [...nowSection().querySelectorAll("button")].find(
      (button) => flat(button) === "再試行",
    );
    expect(retry, "再試行のボタンがあること").toBeDefined();

    responds = REPORT;
    (retry as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(nowSection().querySelector("table")).not.toBeNull();
    });
    expect(upstreamReads()).toEqual([UPSTREAM_PATH, UPSTREAM_PATH]);
    expect(tableCells()).toEqual(EXPECTED_TABLE);
    // **失敗の表示は残らない。**
    expect(nowSection().querySelector("p.error")).toBeNull();

    await unmount(view, { outro: false });
  });
});

/**
 * **注記が、事実を言っているか**（作業単位 D-9 のレビュー是正 / 2026-09-08）。
 *
 * # ⚠ 着手前の文面は事実ではなかった
 *
 * 「**突き合わせは、起動時に測った相手といまの上流を照らした結果です**」と書いていたが、
 * **「起動時に測った相手」は存在しない。**
 * 突き合わせの相手は **`src/measured.rs` が持つ定数が名乗る「測ったときの相手」**で、
 * [ADR-0033](../../docs/adr/0033-measured-constants-name-what-they-were-measured-on.md)
 * のとおり**過去にこの機で測ってソースへ焼き込んだ綴り**である。
 * **起動時に測っているものは1つも無い。**
 *
 * **照合そのものも起動時だけではない** ——
 * `server::upstream` の書き手は `refresh_once` / `refresh_sub_once` で、
 * **5分ごとの引き直し**（ADR-0020）**でも更新される。**
 *
 * **壊れ方**: 利用者が `見た行 10 / ずれ 3` を見て
 * **「起動し直せば測り直されて 10/0 になる」と読み、再起動して数が変わらないことに困る。**
 * **実際に要るのは定数の測り直し**（ADR-0033）。
 *
 * # ⚠ この画面は「事実を読むため」に在る
 *
 * **その画面が事実でないことを言うのは、他の画面より重い。**
 * それなのに、**この文面を見る検査は1本も無かった**
 * （`screen-wording.test.ts` が見ているのは **ADR の出典が漏れていないこと**であって、
 * 「画面に無い操作を名乗らない」でも「注記が事実か」でもない）。
 *
 * **実測**（`mA` —— 着手時の1文へ戻す変異 / 複製ツリー）:
 * **3 赤・exit 1**（この節の3本）。
 * **この節を外すと 733 全緑・exit 0**（`mA_noguard`）
 * ——**赤にしているのはこの3本だけである。**
 */
describe("注記が、事実を言っている", () => {
  it("突き合わせの相手が、ソースに焼き込んだ定数だと書いてある", async () => {
    const { view } = await open();
    const text = flat(target);
    expect(text).toContain("ソースに焼き込んである");
    expect(text).toContain("測ったときの相手");
    expect(text).toContain("いま上流が載せているモデルの一覧");
    // **「起動時に測った」と読ませない**（測っているものは1つも無い）。
    expect(text).not.toContain("起動時に測った");
    await unmount(view, { outro: false });
  });

  it("測り直すのは人であって、personacore ではないと書いてある", async () => {
    const { view } = await open();
    const text = flat(target);
    expect(text).toContain("personacore が測り直すことはありません");
    await unmount(view, { outro: false });
  });

  /**
   * **失敗シナリオそのものを名指しで塞ぐ。**
   *
   * 「再起動すれば数が直る」と読ませないことと、
   * **代わりに何をすればよいか**（定数の測り直し）を、両方言う。
   */
  /**
   * **頼んだ名前と答えた名前が別物であることを、画面が言う**（作業単位 B-45）。
   *
   * **2つの綴りを並べて出すだけでは、読み手は
   * 「別名で同じものを指しているのだろう」と読む。**
   * **`llama serve` は `model` の欄を無視して、いま載っているモデルで答える**
   * （`C-18` が実測 / 2026-09-17）。
   */
  it("頼んだ名前と答えた名前が、別物だと書いてある", async () => {
    const { view } = await open();
    const text = flat(target);
    expect(text).toContain("personacore が上流に指定した名前");
    expect(text).toContain("いま載っているモデルが答えます");
    expect(text).toContain("上流が自分で名乗った名前");
    await unmount(view, { outro: false });
  });

  /**
   * **切り落としを、画面が自分で言う**（レビュー是正 / 2026-09-17）。
   *
   * **`llama-server` が名乗る `id` は gguf の絶対パス**で、
   * **サーバは継ぎ目でファイル名だけにしている**（絶対制約: 内部パスを外へ出さない）。
   * **言わないと、読み手は「上流はこの短い綴りで名乗った」と読む**
   * ——すぐ上の注記が「上流が自分で名乗った名前」と言っているので、
   * **切ったことを黙っていると、そこだけが黙って嘘になる。**
   */
  it("置き場所ごと名乗られたら、ファイル名だけを出すと書いてある", async () => {
    const { view } = await open();
    const text = flat(target);
    expect(text).toContain("置き場所");
    expect(text).toContain("ファイル名だけ");
    await unmount(view, { outro: false });
  });

  /**
   * **名乗らなかった2つの理由を、画面が分けていると言う**（レビュー是正 / 2026-09-17）。
   *
   * **上流が黙っている**（`answered_by` も `null`）のと、
   * **一覧は読めたが名指しできない**のは、**調べる先がまるで違う。**
   */
  it("名指しできないことと、黙っていることが、別だと書いてある", async () => {
    const { view } = await open();
    const text = flat(target);
    expect(text).toContain("どれが答えたかを決められません");
    expect(text).toContain("上流が黙っているとき");
    expect(text).toContain("別の文面が出ます");
    await unmount(view, { outro: false });
  });

  it("ずれは再起動では消えない、と書いてある", async () => {
    const { view } = await open();
    const text = flat(target);
    expect(text).toContain("再起動では消えません");
    expect(text).toContain("定数を測り直します");
    await unmount(view, { outro: false });
  });
});

/**
 * **見出しは、ここへ案内した文言と同じ名前を名乗る**
 * （作業単位 D-9 のレビュー是正 / 2026-09-08）。
 *
 * # なぜ要るのか
 *
 * **実測（`m20`）: `<h1>上流モデル</h1>` → `<h1>接続先の一覧</h1>` が 714 全緑だった。**
 *
 * 会話の画面の注記は `labelFor({route:"upstream"})` で
 * **「上流モデルの画面で見られます」**と名乗り、**その文言は守られている**
 * （`chat-view-outgoing.test.ts`）。**ところが着いた先の見出しを誰も見ていなかった** ——
 * 片方だけ変えると **「案内だけが黙って嘘になる」**。
 * **`chat-view-outgoing.test.ts` が塞いだつもりの穴の、ちょうど反対側である。**
 *
 * **⚠ 綴りは直書きする。** `labelFor` から作ると、
 * **両方を差し替える変異が期待値も一緒に動かして緑**になる
 * （CLAUDE.md / `chat-view-empty.test.ts` の実測）。
 * **そのうえで `labelFor` と一致することも見る**（写しを2つにしないため）。
 *
 * **是正後に `m20` を当て直した**（複製ツリー / 作業者の実測）: **2 赤・exit 1**
 * （この1本と、`chat-view-outgoing.test.ts` の
 * 「名指しした先の画面が、案内した名前で自分を名乗る」）。
 * **その2本を外すと 734 全緑・exit 0**（`m20_noguard`）
 * ——**赤にしているのはその2本である。**
 */
describe("見出し", () => {
  it("画面の名前を名乗る（ナビ・リンクと同じ綴り）", async () => {
    const { view } = await open();
    const heading = target.querySelector("h1");
    expect(heading, "見出しがあること").not.toBeNull();
    expect(flat(heading)).toBe(SCREEN_NAME);
    expect(flat(heading)).toBe(labelFor({ route: "upstream" }));
    await unmount(view, { outro: false });
  });
});
