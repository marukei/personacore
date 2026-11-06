// @vitest-environment jsdom
/**
 * **記憶の一覧と修復の画面**（作業単位 D-3 / `src/lib/MemoryList.svelte` / ADR-0022）。
 *
 * # ここが見るもの
 *
 * | 見るもの | なぜ |
 * |---|---|
 * | **主語がどちらか** | 落とすと一人称が混線する（絶対制約） |
 * | **読めなかった行が出るか** | 落とすと**修復の対象が画面から消える**（ADR-0022 §2） |
 * | **どの行を選んだら、どの記憶へ送られるか** | **帰属**の壊れ方（CLAUDE.md） |
 * | **訂正が通った行が「訂正済み」側へ移るか** | 想起から外れたことが、画面で分かる |
 * | **消す・書き換える要求を出さないこと** | 絶対制約（`memory` の行は消さない） |
 *
 * **何を送るか（キーの有無・拒否の理由）は `memory-repair.test.ts`。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import MemoryList from "../src/lib/MemoryList.svelte";
import type { MemoryView, UnreadableMemoryView } from "../src/lib/api";
import { readerInZone } from "./support/zone";

/**
 * **この測定は、ニューヨークの読み手として走る**（作業単位 F-7b）。
 *
 * **機械の既定に寄りかからない** —— 寄りかかると、
 * **この機（`Asia/Tokyo`）でしか成立しない期待値**になる。
 * **配線の権威はここではない**（`tests/timestamp-wiring.test.ts` が
 * 同じ画面を2つの時間帯に通す）。ここは期待値を決定的にするだけ。
 */
readerInZone("America/New_York");

/** 対象のキャラクター。**本文にも分類にも出てこない綴り。** */
const HANDLE = "mio";

/** 別のキャラクター（**送り先が漏れていないことの対照**）。 */
const OTHER_HANDLE = "aoi";

/** 1件目。 */
const FIRST: MemoryView = {
  id: 11,
  subject: "user",
  content: "黒猫を飼っている",
  category: "好み",
  created_at: "2026-08-27T00:00:00Z",
};

/**
 * 2件目。**主語も本文も分類も1件目と違う。**
 *
 * # ⚠ **これは「真ん中」でなければならない**
 *
 * 選ぶ操作のテストは、**選ばれる対象が端にあると「常に端」の変異を通す**
 * （D-2 のレビューで実測。「常に先頭」は赤くなるが「常に最終」は素通りした）。
 * **3件にして真ん中を押せば、1本で両向きが赤になる。**
 */
const MIDDLE: MemoryView = {
  id: 22,
  subject: "character",
  content: "喫茶店でアルバイトをしている",
  category: "経歴",
  created_at: "2026-08-27T00:10:00Z",
};

/** 3件目。**MIDDLE を端に置かないために居る**（上記）。分類は未設定。 */
const LAST: MemoryView = {
  id: 33,
  subject: "user",
  content: "朝は紅茶を飲む",
  category: null,
  created_at: "2026-08-27T00:20:00Z",
};

/**
 * **画面に出る作成時刻の綴り**（作業単位 F-7 → **F-7b で読み手の時間帯になった**）。
 *
 * **⚠ ここに直書きする理由。** `timeLabel` から作ると、
 * **綴りを変える変異が期待値も一緒に動かして緑**になる
 * （CLAUDE.md「写しを1つにしても、その1つは自動的には守られない」）。
 *
 * **⚠ 3件とも別の分**にしてある —— 行どうしの取り違えが見えるように。
 *
 * **⚠ 手で引いた値である。** この測定はニューヨークの読み手として走り
 * （上の [`readerInZone`]）、8 月は EDT = UTC-4。
 * **`00:10Z` から 4 時間引くと前日の `20:10`** ——**日付が変わる。**
 * 時間帯を無視する変異は、時のずれではなく**日付の違い**として出る。
 */
const MIDDLE_CREATED_LABEL = "2026-08-26 20:10:00 (UTC-04:00)";
/** 同上（3件目）。 */
const LAST_CREATED_LABEL = "2026-08-26 20:20:00 (UTC-04:00)";

/** 読み出せなかった行。**id は他のどれとも違う。** */
const BROKEN: UnreadableMemoryView = {
  id: 99,
  invalid_columns: ["subject", "locked"],
};

/** 訂正版として作られる行（サーバが返す `correction_id`）。 */
const CORRECTION_ID = 77;

interface Call {
  method: string;
  url: string;
  body: unknown;
  headers: Record<string, string>;
}

let calls: Call[];
let target: HTMLElement;
/** **サーバ側の状態**（訂正が通ると、有効な行の集合が変わる）。 */
let active: MemoryView[];
/** 訂正の応答を失敗にするか。 */
let correctionFails: boolean;
/**
 * **一覧の取得を失敗させる**（`0` = しない / `1` = 1回目から / `2` = 2回目から…）。
 *
 * **「いつから」を持つ理由**: 一覧の取得は
 * **最初の読み込み**と**訂正が通った後の読み直し**の2回起きる。
 * **どちらの失敗も、別の壊れ方をする**（前者は画面が空になる／
 * 後者は訂正が通っているのに一覧だけが古い）。
 */
let listFailsFrom: number;
/**
 * **`ApiRequestError` ではない例外**を起こす。
 *
 * `ok` を読んだ瞬間に投げるので、`api.ts` の `try` の外
 * （`response.ok` の行）で生の `Error` が上がる。
 * **`messageOf` の「予期しない失敗です」の枝は、これでしか通らない。**
 */
let listThrowsRaw: boolean;
/**
 * **サーバが古い一覧を返し続ける。**
 *
 * 訂正が通っても `superseded_by` を入れ忘れた（あるいは別の窓の古い応答が届いた）状態。
 * **この状態を作らないと、`activeMemories` の絞り込みが一度も効かない。**
 */
let serverKeepsStaleList: boolean;
/** 一覧の取得が何回来たか（`listFailsFrom` の判定に使う）。 */
let listCalls: number;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const raw = init?.body;
    calls.push({
      method,
      url,
      body: typeof raw === "string" ? (JSON.parse(raw) as unknown) : null,
      headers: { ...((init?.headers ?? {}) as Record<string, string>) },
    });

    const respond = (body: unknown, status = 200): Promise<Response> =>
      Promise.resolve({
        ok: status >= 200 && status < 300,
        status,
        text: () => Promise.resolve(JSON.stringify(body)),
      } as unknown as Response);

    if (method === "GET") {
      listCalls += 1;
      if (listThrowsRaw) {
        return Promise.resolve({
          get ok(): boolean {
            // **内部の詳細を持った例外**（画面へ出てはいけない値）。
            throw new Error("内部の詳細 C:\\personacore\\secret.db");
          },
          status: 200,
          text: () => Promise.resolve("{}"),
        } as unknown as Response);
      }
      if (listFailsFrom !== 0 && listCalls >= listFailsFrom) {
        return respond(
          {
            error: {
              message: "記憶の一覧を取得できませんでした",
              type: "api_error",
            },
          },
          500,
        );
      }
      // **サーバと同じ振る舞い** —— **訊かれたときだけ上書き済みの節を足す**
      // （作業単位 D-3c）。**ここでは常に空**で、中身が並ぶ腕は
      // `memory-superseded.test.ts` が持つ。
      const asked = new URL(url, "http://127.0.0.1").searchParams.get("include");
      return respond({
        character: HANDLE,
        memories: active,
        unreadable: [BROKEN],
        ...(asked === "superseded"
          ? { superseded: { memories: [], unreadable: [], total: 0, limit: 100 } }
          : {}),
      });
    }

    if (correctionFails) {
      return respond(
        {
          error: {
            message: "その記憶はすでに訂正されています",
            type: "invalid_request_error",
            code: "memory_already_superseded",
          },
        },
        409,
      );
    }

    // **成功したサーバの姿を真似る** —— 訂正された行は
    // `superseded_by` が入るので、次の一覧には出てこない（訂正版だけが出る）。
    const supersededId = Number(
      /\/memories\/(\d+)\/corrections$/.exec(url)?.[1] ?? "0",
    );
    const sent = typeof raw === "string" ? (JSON.parse(raw) as {
      subject: MemoryView["subject"];
      content: string;
      category?: string;
    }) : null;
    if (!serverKeepsStaleList) {
      active = [
        ...active.filter((memory) => memory.id !== supersededId),
        {
          id: CORRECTION_ID,
          subject: sent?.subject ?? "user",
          content: sent?.content ?? "",
          category: sent?.category ?? null,
          created_at: "2026-08-27T01:00:00Z",
        },
      ];
    }
    return respond(
      {
        character: HANDLE,
        superseded_id: supersededId,
        correction_id: CORRECTION_ID,
      },
      201,
    );
  });
}

/** その id の行。**行から引く**（全体から引くと1件目に当たり続ける）。 */
function rowOf(id: number): HTMLElement {
  const found = target.querySelector(`tr[data-memory-id="${id}"]`);
  expect(found, `id ${id} の行があること`).not.toBeNull();
  return found as HTMLElement;
}

/** その節に並んでいる id。 */
function idsIn(section: string): number[] {
  return [
    ...target.querySelectorAll(`tbody[data-section="${section}"] tr`),
  ].map((row) => Number((row as HTMLElement).dataset["memoryId"]));
}

/**
 * その行のセル。**列の順番ごと見る。**
 *
 * # ⚠ **`data-*` はテストが置いた印であって、読み手が見るものではない**
 *
 * `D-4` のレビューで実測（セルを入れ替える変異が 372 全緑）。
 * **D-3c のレビューで、この画面の3表も同じ穴だと分かった**（2026-08-28）——
 * **`主語` と `本文` を入れ替える変異が 406 全緑。**
 * `data-subject` は正しいままなので、**テストからは永久に見えない。**
 * **`memory.subject` を落とさないという絶対制約の、画面側の写しにあたる場所。**
 */
function cellsOf(section: string, id: number): string[] {
  const row = target.querySelector(
    `tbody[data-section="${section}"] tr[data-memory-id="${id}"]`,
  );
  expect(row, `${section} に id ${id} の行があること`).not.toBeNull();
  return [...(row as HTMLElement).querySelectorAll("td")].map((cell) =>
    (cell.textContent ?? "").replace(/\s+/gu, " ").trim(),
  );
}

/** その表の見出しのセル。 */
function headersOf(section: string): string[] {
  const table = target
    .querySelector(`tbody[data-section="${section}"]`)
    ?.closest("table");
  expect(table, `${section} の表があること`).not.toBeNull();
  return [...(table as HTMLTableElement).querySelectorAll("th")].map((cell) =>
    (cell.textContent ?? "").replace(/\s+/gu, " ").trim(),
  );
}

/** その行の「訂正版を足す」ボタン。 */
function correctButtonOf(id: number): HTMLButtonElement {
  const found = [...rowOf(id).querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === "訂正版を足す",
  );
  expect(found, `id ${id} の行に「訂正版を足す」があること`).toBeDefined();
  return found as HTMLButtonElement;
}

/** 開いている訂正の下書き。 */
function form(): HTMLFormElement {
  const found = target.querySelector("form");
  expect(found, "訂正のフォームが開いていること").not.toBeNull();
  return found as HTMLFormElement;
}

function field(name: string): HTMLInputElement | HTMLTextAreaElement {
  const found = form().querySelector(`[name="${name}"]`);
  expect(found, `${name} の欄があること`).not.toBeNull();
  return found as HTMLInputElement | HTMLTextAreaElement;
}

/** 画面が出している通知（訂正が通った後の文面）。**無ければ空文字。** */
function notice(): string {
  return target.querySelector("p.notice")?.textContent?.trim() ?? "";
}

/** 画面が出している失敗の文言（フォームの外側）。 */
function pageErrors(): string {
  return [...target.querySelectorAll("section > p.error")]
    .map((paragraph) => paragraph.textContent?.trim() ?? "")
    .join("\n");
}

/** フォームが出している止めた理由。**無ければ `null`。** */
function formError(): string | null {
  const found = form().querySelector("p.error");
  return found === null ? null : (found.textContent?.trim() ?? "");
}

/** 送信のボタン。 */
function submitButton(): HTMLButtonElement {
  const found = form().querySelector('button[type="submit"]');
  expect(found, "送信のボタンがあること").not.toBeNull();
  return found as HTMLButtonElement;
}

function subjectSelect(): HTMLSelectElement {
  const found = form().querySelector('[name="subject"]');
  expect(found, "主語の欄があること").not.toBeNull();
  return found as HTMLSelectElement;
}

function typeInto(name: string, value: string): void {
  const input = field(name);
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

function chooseSubject(value: string): void {
  const select = subjectSelect();
  select.value = value;
  select.dispatchEvent(new Event("change", { bubbles: true }));
  flushSync();
}

function submit(): void {
  form().dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
}

/** 一覧が出るまで待つ（**真ん中を押すために3件要る**）。 */
async function waitForRows(): Promise<void> {
  await vi.waitFor(() => {
    expect(idsIn("active").length).toBeGreaterThanOrEqual(3);
  });
}

/**
 * その識別子で開く。
 *
 * **識別子を引数に取る**のが要点 —— **1つのフィクスチャでしか開かないと、
 * `handle` を決め打つ実装と区別が付かない**（`CLAUDE.md` の `B-7` と同型:
 * 「既定で回っている限り、引いてこようが定数だろうが区別が付かない」）。
 */
async function openWith(handle: string): Promise<ReturnType<typeof mount>> {
  const view = mount(MemoryList, { target, props: { handle } });
  await waitForRows();
  return view;
}

async function open(): Promise<ReturnType<typeof mount>> {
  return openWith(HANDLE);
}

beforeEach(() => {
  calls = [];
  active = [FIRST, MIDDLE, LAST];
  correctionFails = false;
  listFailsFrom = 0;
  listThrowsRaw = false;
  serverKeepsStaleList = false;
  listCalls = 0;
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

it("フィクスチャの値は互いに違う（同値だと取り違えが観測できない）", () => {
  const ids = [FIRST.id, MIDDLE.id, LAST.id, BROKEN.id, CORRECTION_ID];
  expect(new Set(ids).size).toBe(ids.length);
  expect(FIRST.subject).not.toBe(MIDDLE.subject);
  expect(FIRST.content).not.toBe(MIDDLE.content);
  expect(HANDLE).not.toBe(OTHER_HANDLE);
  // **識別子が本文や分類に紛れていない**（送り先の照合が偶然当たらないように）。
  for (const memory of [FIRST, MIDDLE, LAST]) {
    expect(memory.content).not.toContain(HANDLE);
    expect(String(memory.category)).not.toContain(HANDLE);
  }
});

describe("一覧", () => {
  it("そのキャラクターの記憶を引く", async () => {
    const view = await open();
    const listing = calls.find((call) => call.method === "GET");
    // **`?include=superseded` を足す**（作業単位 D-3c）。
    // **その節の中身を見るのは `memory-superseded.test.ts`。**
    expect(listing?.url).toBe(
      `/admin/characters/${HANDLE}/memories?include=superseded`,
    );
    expect(listing?.url).not.toContain(OTHER_HANDLE);
    await unmount(view, { outro: false });
  });

  /**
   * **主語は行ごとに出る。**
   *
   * **どちらの記憶かが一目で分かること**が、この画面でいちばん重要
   * ——訂正版を書くときに主語を取り違えると、**一人称の混線がそのまま固定される。**
   * しかも**持ち主を取り違えた記憶は、正しい持ち主へ移す手段が無い**（CLAUDE.md）。
   */
  it("主語が、その行のものとして出る", async () => {
    const view = await open();

    expect(rowOf(FIRST.id).textContent).toContain("利用者");
    expect(rowOf(FIRST.id).textContent).not.toContain("キャラクター");
    expect(rowOf(MIDDLE.id).textContent).toContain("キャラクター");
    // **「キャラクター」は「利用者」を含まない綴りなので、両向きに見られる。**
    expect(rowOf(MIDDLE.id).textContent).not.toContain("利用者");
    expect(rowOf(LAST.id).textContent).toContain("利用者");

    // **生の綴りも持つ**（写した型が外れたときに気付けるように）。
    expect(rowOf(MIDDLE.id).dataset["subject"]).toBe(MIDDLE.subject);
    expect(rowOf(FIRST.id).dataset["subject"]).toBe(FIRST.subject);

    await unmount(view, { outro: false });
  });

  it("本文と分類が、その行のものとして出る", async () => {
    const view = await open();

    expect(rowOf(MIDDLE.id).textContent).toContain(MIDDLE.content);
    expect(rowOf(MIDDLE.id).textContent).toContain(MIDDLE.category as string);
    expect(rowOf(MIDDLE.id).textContent).not.toContain(FIRST.content);
    expect(rowOf(MIDDLE.id).textContent).not.toContain(LAST.content);
    expect(rowOf(FIRST.id).textContent).toContain(FIRST.content);

    await unmount(view, { outro: false });
  });

  /**
   * **読めなかった行を落とさない**（ADR-0022 §2）。
   *
   * 落とすと、**チャットと抽出を止めている当の行が、修復の入口から消える。**
   */
  /**
   * **想起される記憶の表 —— 列の順番ごと見る。**
   *
   * **`D-4` の穴がこの表にも残っていた**（D-3c のレビューで実測 / 2026-08-28）——
   * **`主語` と `本文` のセルを入れ替える変異が 406 全緑。**
   * `data-subject` は正しいままなので、**`toContain` をいくら並べても見えない。**
   *
   * **⚠ ここが入れ替わると、「利用者の記憶」と「キャラクターの記憶」の列が入れ替わる。**
   * **`memory.subject` を落とさない**という絶対制約の、画面側の写しにあたる。
   */
  it("想起される行のセルが、列の順番どおりに出る", async () => {
    const view = await open();

    expect(headersOf("active")).toEqual([
      "id",
      "主語",
      "本文",
      "分類",
      "作成",
      "",
    ]);
    // **真ん中の行**（端に置くと「常に先頭／常に末尾」の片方が通る）。
    expect(cellsOf("active", MIDDLE.id)).toEqual([
      String(MIDDLE.id),
      "キャラクター",
      MIDDLE.content,
      MIDDLE.category as string,
      MIDDLE_CREATED_LABEL,
      "訂正版を足す",
    ]);
    // **主語が違う行**（決め打ちと区別する）と、**分類が未設定の行。**
    expect(cellsOf("active", LAST.id)).toEqual([
      String(LAST.id),
      "利用者",
      LAST.content,
      "未設定",
      LAST_CREATED_LABEL,
      "訂正版を足す",
    ]);

    await unmount(view, { outro: false });
  });

  /** **読めなかった行の表** —— `id` と壊れた列が入れ替わっても見えるように。 */
  it("読み出せなかった行のセルが、列の順番どおりに出る", async () => {
    const view = await open();

    expect(headersOf("unreadable")).toEqual(["id", "読めなかった列", ""]);
    expect(cellsOf("unreadable", BROKEN.id)).toEqual([
      String(BROKEN.id),
      BROKEN.invalid_columns.join(", "),
      "訂正版を足す",
    ]);

    await unmount(view, { outro: false });
  });

  it("読み出せなかった行が、壊れた列つきで出る", async () => {
    const view = await open();

    expect(idsIn("unreadable")).toEqual([BROKEN.id]);
    for (const column of BROKEN.invalid_columns) {
      expect(rowOf(BROKEN.id).textContent).toContain(column);
    }
    // **有効な行の側に混ざらない。**
    expect(idsIn("active")).not.toContain(BROKEN.id);

    await unmount(view, { outro: false });
  });

  /**
   * **想起される行と、訂正済みの行が、見て区別できる。**
   *
   * 想起は `superseded_by IS NULL` の行しか読まない（絶対制約）。
   */
  it("最初は、全部が「想起される」側にある", async () => {
    const view = await open();

    expect(idsIn("active")).toEqual([FIRST.id, MIDDLE.id, LAST.id]);
    expect(idsIn("superseded")).toEqual([]);
    expect(rowOf(MIDDLE.id).dataset["state"]).toBe("active");

    await unmount(view, { outro: false });
  });
});

describe("訂正の導線", () => {
  it("下書きは、選んだ行の値から始まる", async () => {
    const view = await open();

    correctButtonOf(MIDDLE.id).click();
    flushSync();

    expect(subjectSelect().value).toBe(MIDDLE.subject);
    expect(field("content").value).toBe(MIDDLE.content);
    expect(field("category").value).toBe(MIDDLE.category);
    // **対照: 前後の行の値ではない。**
    expect(field("content").value).not.toBe(FIRST.content);
    expect(field("content").value).not.toBe(LAST.content);
    expect(subjectSelect().value).not.toBe(FIRST.subject);
    // **どの記憶を直しているかが、フォームに出ている。**
    expect(form().textContent).toContain(String(MIDDLE.id));

    await unmount(view, { outro: false });
  });

  it("別の行を選ぶと、前の行の下書きが残らない", async () => {
    const view = await open();

    correctButtonOf(MIDDLE.id).click();
    flushSync();
    typeInto("content", "打ちかけの本文");

    correctButtonOf(LAST.id).click();
    flushSync();

    expect(field("content").value).toBe(LAST.content);
    expect(field("content").value).not.toBe("打ちかけの本文");
    expect(form().textContent).toContain(String(LAST.id));

    await unmount(view, { outro: false });
  });

  it("選んだ行の id へ送られる（前後の行へは送られない）", async () => {
    const view = await open();

    correctButtonOf(MIDDLE.id).click();
    flushSync();
    typeInto("content", "喫茶店の店主である");
    submit();

    const post = await vi.waitFor(() => {
      const found = calls.find((call) => call.method === "POST");
      expect(found).toBeDefined();
      return found as Call;
    });
    expect(post.url).toBe(
      `/admin/characters/${HANDLE}/memories/${MIDDLE.id}/corrections`,
    );
    expect(post.url).not.toContain(`/memories/${FIRST.id}/`);
    expect(post.url).not.toContain(`/memories/${LAST.id}/`);
    expect(post.url).not.toContain(`/memories/${BROKEN.id}/`);
    expect(post.body).toEqual({
      subject: MIDDLE.subject,
      content: "喫茶店の店主である",
      category: MIDDLE.category,
    });
    expect(post.headers["content-type"]).toBe("application/json");

    await unmount(view, { outro: false });
  });

  /**
   * **主語だけを直す訂正が送れること**（実測でいちばん多い誤り / ADR-0021 §10）。
   *
   * 本文が1文字も変わらないので、「本文が同じなら送らない」実装では通らない。
   */
  it("主語だけを直して送れる", async () => {
    const view = await open();

    correctButtonOf(MIDDLE.id).click();
    flushSync();
    chooseSubject("user");
    submit();

    const post = await vi.waitFor(() => {
      const found = calls.find((call) => call.method === "POST");
      expect(found).toBeDefined();
      return found as Call;
    });
    expect(post.body).toEqual({
      subject: "user",
      content: MIDDLE.content,
      category: MIDDLE.category,
    });

    await unmount(view, { outro: false });
  });

  /**
   * **主語を「選んでください」へ戻せる。戻したら送れない。**
   *
   * **読めた行から始めても、主語は `null` に戻りうる**（`value=""`）。
   * **D-3 のレビューが実測した穴** —— `selectSubject` の `?? null` を
   * `?? "user"` にする変異が 107 全緑だった。
   * その実装では、**戻したつもりの利用者が「利用者」の記憶を書き込む。**
   * **持ち主を取り違えた記憶は、正しい持ち主へ移す手段が無い**（CLAUDE.md 1節）。
   */
  it("主語を選び直して空へ戻すと、送れなくなる", async () => {
    const view = await open();

    correctButtonOf(MIDDLE.id).click();
    flushSync();
    // 元は「キャラクター」。**別の主語を一度選んでから**空へ戻す
    // （最初から空のときとは別の経路）。
    chooseSubject("user");
    expect(subjectSelect().value).toBe("user");
    expect(submitButton().disabled).toBe(false);

    chooseSubject("");
    expect(subjectSelect().value).toBe("");
    expect(formError()).toContain("主語");
    expect(submitButton().disabled).toBe(true);

    submit();
    await Promise.resolve();
    flushSync();
    expect(calls.filter((call) => call.method === "POST")).toEqual([]);

    await unmount(view, { outro: false });
  });

  it("元と同じ内容では送れない", async () => {
    const view = await open();

    correctButtonOf(MIDDLE.id).click();
    flushSync();
    submit();
    await Promise.resolve();
    flushSync();

    expect(calls.filter((call) => call.method === "POST")).toEqual([]);
    // **止めた理由を出している**（押しても何も起きない画面にしない）。
    expect(formError()).toContain("元と同じ");
    expect(submitButton().disabled).toBe(true);

    await unmount(view, { outro: false });
  });

  /**
   * **成功したら、その行が「訂正済み」側へ移る。**
   *
   * サーバは訂正された行を一覧に返さなくなる（`superseded_by` が入る）ので、
   * **読み直さないと、想起から外れた行が「想起される」側に残り続ける。**
   */
  it("成功したら、その行が訂正済み側へ移り、訂正版が想起される側に出る", async () => {
    const view = await open();

    correctButtonOf(MIDDLE.id).click();
    flushSync();
    typeInto("content", "喫茶店の店主である");
    submit();

    await vi.waitFor(() => {
      expect(idsIn("superseded")).toEqual([MIDDLE.id]);
    });
    expect(idsIn("active")).not.toContain(MIDDLE.id);
    // **訂正版は想起される側に出る**（＝一覧を読み直している）。
    expect(idsIn("active")).toContain(CORRECTION_ID);
    expect(rowOf(CORRECTION_ID).textContent).toContain("喫茶店の店主である");
    // **両隣は動かない。**
    expect(idsIn("active")).toContain(FIRST.id);
    expect(idsIn("active")).toContain(LAST.id);
    // **移った行は消えていない**（時系列保持 / 訂正版の id を指す）。
    expect(rowOf(MIDDLE.id).dataset["state"]).toBe("superseded");
    expect(rowOf(MIDDLE.id).textContent).toContain(String(CORRECTION_ID));
    expect(rowOf(MIDDLE.id).textContent).toContain(MIDDLE.content);

    /*
     * **2つの id を取り違えないこと。**
     *
     * `admin.rs` の `CorrectionView` は `superseded_id` を
     * **「どの行が無効になったかを後から辿れる唯一の手掛かり」**と書いている。
     * **逆に出すと、運用者は生きている行を「外れた」と記録する。**
     * **どちらの id も文面に出るので、`toContain` を2本並べても入れ替えは捕まらない**
     * ——**どちらの id が、どちらの役として出ているか**を見る。
     */
    expect(notice()).toContain(`id ${MIDDLE.id} は想起から外れました`);
    expect(notice()).toContain(`訂正版は id ${CORRECTION_ID}`);
    expect(notice()).not.toContain(`id ${CORRECTION_ID} は想起から外れました`);

    await unmount(view, { outro: false });
  });

  /**
   * **この画面で訂正した表 —— 列の順番ごと見る。**
   *
   * ここも `主語` と `本文` の入れ替えが 406 全緑だった（D-3c のレビュー）。
   * **読めなかった行を直した回は別の枝**（主語も本文も出せない）なので、両方見る。
   */
  it("訂正した行のセルが、列の順番どおりに出る", async () => {
    const view = await open();

    correctButtonOf(MIDDLE.id).click();
    flushSync();
    typeInto("content", "喫茶店の店主である");
    submit();
    await vi.waitFor(() => {
      expect(idsIn("superseded")).toEqual([MIDDLE.id]);
    });

    expect(headersOf("superseded")).toEqual([
      "id",
      "主語",
      "本文",
      "いまの状態",
    ]);
    // **本文は「直す前の姿」**（訂正版の本文ではない）。
    expect(cellsOf("superseded", MIDDLE.id)).toEqual([
      String(MIDDLE.id),
      "キャラクター",
      MIDDLE.content,
      `想起から外れました（訂正版 id ${CORRECTION_ID}）。行は残っています。`,
    ]);

    await unmount(view, { outro: false });
  });

  /** **読めなかった行を直した回**（主語も本文も出せない枝）。 */
  it("読めなかった行を訂正した後のセルが、列の順番どおりに出る", async () => {
    const view = await open();

    correctButtonOf(BROKEN.id).click();
    flushSync();
    chooseSubject("character");
    typeInto("content", "黒猫の名前はコーヒーである");
    submit();
    await vi.waitFor(() => {
      expect(idsIn("superseded")).toEqual([BROKEN.id]);
    });

    expect(cellsOf("superseded", BROKEN.id)).toEqual([
      String(BROKEN.id),
      "読めませんでした",
      BROKEN.invalid_columns.join(", "),
      `想起から外れました（訂正版 id ${CORRECTION_ID}）。行は残っています。`,
    ]);

    await unmount(view, { outro: false });
  });

  /**
   * **サーバが古い一覧を返しても、想起から外れた行を「想起される」側に戻さない。**
   *
   * `superseded_by` を入れ忘れた応答や、**別の窓の古い応答**が届く形。
   * この状態を作らないと `activeMemories` の絞り込みが一度も効かない
   * （D-3 のレビューで、絞り込みを外す変異が 107 全緑だった）。
   */
  it("一覧が古いままでも、訂正した行は想起される側に戻らない", async () => {
    serverKeepsStaleList = true;
    const view = await open();

    correctButtonOf(MIDDLE.id).click();
    flushSync();
    typeInto("content", "喫茶店の店主である");
    submit();

    await vi.waitFor(() => {
      expect(idsIn("superseded")).toEqual([MIDDLE.id]);
    });
    // **サーバはまだ返している**（＝絞り込みが効いていることの前提）。
    const listing = calls.filter((call) => call.method === "GET");
    expect(listing.length).toBeGreaterThanOrEqual(2);
    expect(active.map((memory) => memory.id)).toContain(MIDDLE.id);
    // **それでも想起される側には出ない。**
    expect(idsIn("active")).not.toContain(MIDDLE.id);
    // **両隣は残る**（丸ごと消す実装と区別する）。
    expect(idsIn("active")).toEqual([FIRST.id, LAST.id]);

    await unmount(view, { outro: false });
  });

  /**
   * **訂正は通ったが、読み直しに失敗した回。**
   *
   * **訂正そのものは成立している**ので、
   * **「どの行が外れて、どれが訂正版か」を画面から失わないこと**が要る
   * （`superseded_id` は、後から辿れる唯一の手掛かり）。
   */
  it("訂正の後で読み直しに失敗しても、通ったことと2つの id は残る", async () => {
    const view = await open();

    correctButtonOf(MIDDLE.id).click();
    flushSync();
    typeInto("content", "喫茶店の店主である");
    // 訂正の `POST` は通し、その後の読み直しだけを落とす。
    listFailsFrom = 2;
    submit();

    await vi.waitFor(() => {
      expect(target.textContent).toContain("記憶の一覧を取得できませんでした");
    });
    expect(calls.filter((call) => call.method === "POST")).toHaveLength(1);
    expect(notice()).toContain(`id ${MIDDLE.id} は想起から外れました`);
    expect(notice()).toContain(`訂正版は id ${CORRECTION_ID}`);

    await unmount(view, { outro: false });
  });

  it("失敗したら、その行は移らず、サーバの理由が出る", async () => {
    correctionFails = true;
    const view = await open();

    correctButtonOf(MIDDLE.id).click();
    flushSync();
    typeInto("content", "喫茶店の店主である");
    submit();

    await vi.waitFor(() => {
      expect(target.textContent).toContain("その記憶はすでに訂正されています");
    });
    expect(idsIn("superseded")).toEqual([]);
    expect(idsIn("active")).toContain(MIDDLE.id);
    expect(rowOf(MIDDLE.id).dataset["state"]).toBe("active");

    await unmount(view, { outro: false });
  });
});

describe("読めなかった行の訂正", () => {
  /**
   * **壊れた行にも訂正版を書かせる**（ADR-0022 §1）。
   * ただし**主語は推測しない** —— 元の主語は API から返ってこない。
   */
  it("主語を選ぶまで送れない", async () => {
    const view = await open();

    correctButtonOf(BROKEN.id).click();
    flushSync();
    expect(subjectSelect().value).toBe("");
    typeInto("content", "黒猫の名前はコーヒーである");
    submit();
    await Promise.resolve();
    flushSync();

    expect(calls.filter((call) => call.method === "POST")).toEqual([]);
    // **見出しの綴りではなく、止めた理由の段落を見る**
    // （見出しは常に出ているので、それでは何も観測できない）。
    expect(formError()).toContain("主語");
    expect(submitButton().disabled).toBe(true);

    await unmount(view, { outro: false });
  });

  it("主語を選べば、その行の id へ送られる", async () => {
    const view = await open();

    correctButtonOf(BROKEN.id).click();
    flushSync();
    chooseSubject("character");
    typeInto("content", "黒猫の名前はコーヒーである");
    submit();

    const post = await vi.waitFor(() => {
      const found = calls.find((call) => call.method === "POST");
      expect(found).toBeDefined();
      return found as Call;
    });
    expect(post.url).toBe(
      `/admin/characters/${HANDLE}/memories/${BROKEN.id}/corrections`,
    );
    expect(post.url).not.toContain(`/memories/${MIDDLE.id}/`);
    expect(post.body).toEqual({
      subject: "character",
      content: "黒猫の名前はコーヒーである",
    });

    await unmount(view, { outro: false });
  });

  /**
   * **壊れた行を訂正したら、「読み出せなかった」側から消えて「訂正済み」へ移る。**
   *
   * **D-3 のレビューが実測した穴** —— 訂正の記録を
   * 「読めた行のときだけ」にする変異が 107 全緑だった。
   * **`stubFetch` は訂正の後も `unreadable: [BROKEN]` を返し続ける**ので、
   * **記録して絞り込んでいなければ、壊れた行はそのまま残る。**
   *
   * **実害**: 直したはずの行が「まだ壊れている」と見え続け、
   * **二重に訂正しようとして `MemoryAlreadySuperseded` に当たる。**
   */
  it("訂正が通ったら、読み出せなかった側から消えて訂正済みへ移る", async () => {
    const view = await open();
    expect(idsIn("unreadable")).toEqual([BROKEN.id]);

    correctButtonOf(BROKEN.id).click();
    flushSync();
    chooseSubject("character");
    typeInto("content", "黒猫の名前はコーヒーである");
    submit();

    await vi.waitFor(() => {
      expect(idsIn("superseded")).toEqual([BROKEN.id]);
    });
    // **サーバはまだ壊れた行として返している**（絞り込みの前提）。
    expect(idsIn("unreadable")).not.toContain(BROKEN.id);
    // **訂正済みの行は、壊れていた列と訂正版の id を持つ。**
    expect(rowOf(BROKEN.id).dataset["state"]).toBe("superseded");
    expect(rowOf(BROKEN.id).textContent).toContain(String(CORRECTION_ID));
    for (const column of BROKEN.invalid_columns) {
      expect(rowOf(BROKEN.id).textContent).toContain(column);
    }
    // **有効な記憶は巻き添えにならない。**
    expect(idsIn("active")).toContain(MIDDLE.id);

    await unmount(view, { outro: false });
  });
});

describe("送り先は、この画面のキャラクター", () => {
  /**
   * **識別子が prop から来ていることを観測する。**
   *
   * **フィクスチャが1つしか無いと、決め打ちと区別が付かない**
   * （D-3 のレビューで、`listMemories("mio")` / `correctMemory("mio", …)` に
   * 決め打つ変異が 107 全緑だった）。**別の識別子で開く腕を1本置く。**
   *
   * **実害は帰属**: B の画面で書いた訂正版が **A の記憶へ `POST`** される。
   */
  it("別のキャラクターで開くと、その識別子へ読み書きする", async () => {
    const view = await openWith(OTHER_HANDLE);

    const listing = calls.find((call) => call.method === "GET");
    expect(listing?.url).toBe(
      `/admin/characters/${OTHER_HANDLE}/memories?include=superseded`,
    );
    expect(listing?.url).not.toContain(HANDLE);

    correctButtonOf(MIDDLE.id).click();
    flushSync();
    typeInto("content", "喫茶店の店主である");
    submit();

    const post = await vi.waitFor(() => {
      const found = calls.find((call) => call.method === "POST");
      expect(found).toBeDefined();
      return found as Call;
    });
    expect(post.url).toBe(
      `/admin/characters/${OTHER_HANDLE}/memories/${MIDDLE.id}/corrections`,
    );
    expect(post.url).not.toContain(HANDLE);
    // 見出しも、開いたキャラクターを名乗る。
    expect(target.querySelector("h1")?.textContent).toContain(OTHER_HANDLE);
    expect(target.querySelector("h1")?.textContent).not.toContain(HANDLE);

    await unmount(view, { outro: false });
  });
});

describe("一覧が取れなかったとき", () => {
  /**
   * **最初の読み込みに失敗した回。**
   *
   * **サーバの理由をそのまま見せる**（`listError` を黙って `null` にすると、
   * **「記憶が1件も無いキャラクター」に見える** ——
   * 修復の入口としては最悪の嘘になる）。
   */
  it("理由つきで失敗を出す（空の一覧に見せない）", async () => {
    listFailsFrom = 1;
    const view = mount(MemoryList, { target, props: { handle: HANDLE } });

    await vi.waitFor(() => {
      expect(pageErrors()).toContain("記憶の一覧を取得できませんでした");
    });
    // **「ありません」と並べない**（無いのか、取れなかったのかの区別）。
    expect(target.textContent).not.toContain("いま想起される記憶");
    expect(target.querySelector('tbody[data-section="active"]')).toBeNull();

    await unmount(view, { outro: false });
  });

  /**
   * **`ApiRequestError` ではない例外**（`api.ts` の外で起きた失敗）。
   *
   * **絶対制約**: エラーに内部の詳細を載せない。
   * **`String(failure)` に差し替える実装は、例外の文言をそのまま画面へ出す**
   * ——`api.ts` が組み立てていない値なので、内部パスを含みうる。
   */
  it("想定外の例外でも、内部の詳細を画面へ出さない", async () => {
    listThrowsRaw = true;
    const view = mount(MemoryList, { target, props: { handle: HANDLE } });

    await vi.waitFor(() => {
      expect(pageErrors()).toContain("予期しない失敗です");
    });
    const text = target.textContent ?? "";
    expect(text).not.toContain("secret.db");
    expect(text).not.toContain("C:\\");
    expect(text).not.toContain("Error");

    await unmount(view, { outro: false });
  });
});

describe("消す道は無い", () => {
  /**
   * **絶対制約**: `memory` の行を DELETE / UPDATE で潰さない。
   *
   * サーバ側にその経路は無い（`src/server/mod.rs` のルータの doc /
   * `tests/server_admin.rs::no_admin_endpoint_deletes_or_rewrites_a_memory`）。
   * **画面もそれを叩こうとしないこと** —— 叩けば 404 / 405 になるだけだが、
   * **「消せるはず」と読める導線が画面にある**ことが、そもそも設計と食い違う。
   */
  it("記憶へ出す要求は、読み出しと訂正の追加だけ", async () => {
    const view = await open();

    correctButtonOf(MIDDLE.id).click();
    flushSync();
    typeInto("content", "喫茶店の店主である");
    submit();
    await vi.waitFor(() => {
      expect(idsIn("superseded")).toEqual([MIDDLE.id]);
    });

    expect(calls.length).toBeGreaterThan(1);
    for (const call of calls) {
      expect(["GET", "POST"]).toContain(call.method);
      if (call.method === "POST") {
        expect(call.url.endsWith("/corrections")).toBe(true);
      }
    }
    // **画面の文言も「消す」「書き換える」と読めないこと。**
    const text = target.textContent ?? "";
    expect(text).not.toContain("削除");
    expect(text).toContain("訂正版");

    await unmount(view, { outro: false });
  });
});

/**
 * **画面が、利用者に ADR の番号を見せない**（作業単位 F-8）。
 *
 * **文は消さない。** 「記憶は消えない／誤りは訂正版を足して置き換える」は
 * **利用者が知る必要のある事実**（絶対制約の、画面側の写しにあたる）で、
 * **落とすのは出典だけ。**
 *
 * # ⚠ ソースの走査だけでは足りない
 *
 * `tests/screen-wording.test.ts` は**「出典が無いこと」しか言わない**ので、
 * **文ごと消しても緑になる。** ここは、**実際に描かれた DOM の上で文を見ている。**
 */
/**
 * **時刻が、生の ISO 表記で出ていない**（作業単位 F-7）。
 *
 * **瞬間は動かさない**（地域化しない / `src/lib/timestamp.ts` の doc）。
 * **落としたミリ秒は `title` に残す** —— ログや `sqlite3` の値と
 * 目で突き合わせる道を閉じないため。
 */
describe("時刻を、生の ISO 表記で出さない（作業単位 F-7）", () => {
  it("作成の欄が、読める綴りになっている", async () => {
    const view = await open();

    // **セルの配列が権威**（`data-*` はテストが置いた印にすぎない）。
    expect(cellsOf("active", MIDDLE.id)[4]).toBe(MIDDLE_CREATED_LABEL);
    // **生の綴りは、画面のどこにも出ていない。**
    expect(target.textContent).not.toContain(MIDDLE.created_at);
    expect(target.textContent).not.toContain("T00:10:00Z");

    await unmount(view, { outro: false });
  });

  /**
   * **落とした精度を、`title` に残す。**
   *
   * `data-*` と違い、**`title` は読み手が実際に触れる**（重ねると出る）。
   * これが無いと、**同じ秒に立った行どうしを画面から区別できない**
   * ——1ターンで複数の記憶が入るので、本番で普通に起きる。
   */
  it("生の綴りを `title` に残す", async () => {
    const view = await open();

    const cell = rowOf(MIDDLE.id).querySelectorAll("td")[4];
    expect(cell?.getAttribute("title")).toBe(MIDDLE.created_at);
    // **対照: 別の行には、その行の綴りが入っている。**
    const other = rowOf(LAST.id).querySelectorAll("td")[4];
    expect(other?.getAttribute("title")).toBe(LAST.created_at);

    await unmount(view, { outro: false });
  });
});

describe("画面が利用者の言葉で話す（作業単位 F-8）", () => {
  it("記憶が消えないことを言う（出典は出さない）", async () => {
    const view = await open();

    const text = (target.textContent ?? "").replace(/\s+/g, " ");
    expect(text).toContain("記憶は消えません");
    expect(text).toContain("訂正版を足して");
    // **出典は画面に出さない。**
    expect(text).not.toContain("ADR");
    // **括弧の中が空にならない。**
    // **⚠ F-5 で報告された見え方は捕まえていない**（`character-form.test.ts` の同じ注記）。
    // ここが止めるのは「括弧だけ残して中身を落とす」変異。
    expect(text).not.toMatch(/（\s*）/);

    await unmount(view, { outro: false });
  });
});
