// @vitest-environment jsdom
/**
 * **上書きされた記憶を画面に出す**（作業単位 D-3c / `src/lib/MemoryList.svelte` / ADR-0022）。
 *
 * # `memory-list.test.ts` との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | いま想起される記憶・読めない行・訂正の導線 | `memory-list.test.ts` |
 * | **DB に残っている上書き済みの行** | **ここ** |
 * | 鎖の組み立て（純粋関数） | `memory-chain.test.ts` |
 *
 * # ここが守るもの
 *
 * | 守るもの | 壊れると |
 * |---|---|
 * | **`?include=superseded` を送る** | 節が来ず、**画面が何も言えない** |
 * | **`total` と `limit` を出す** | **画面が「これで全部」と嘘をつく**（D-4a と同じ立場） |
 * | **鎖が辿れる** | 「以前はXだったが今はY」の **Y へ辿れない** |
 * | **読めない上書き済みの行が出る** | **壊れた環だけが鎖から落ち、`total` にだけ残る** |
 * | **有効な行に混ざらない** | 二度と想起されない行が「想起される」ものとして読まれる |
 * | **再読み込みで消えない** | D-3 の制限そのもの（画面の寿命しか持たない記録） |
 *
 * **セルは配列で突き合わせる**（`data-*` はテストが置いた印であって、
 * 読み手が何を見るかの権威ではない / `D-4` のレビューの実測）。
 *
 * **`?include=` の綴りが写し元と一致するか**は `memory-chain.test.ts`
 * （`import.meta.url` が **jsdom では `file:` にならない**ので、
 * ソースを読む見張り番は `node` で走る側に置いてある）。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import MemoryList from "../src/lib/MemoryList.svelte";
import type {
  MemoryView,
  SupersededListView,
  SupersededMemoryView,
  UnreadableMemoryView,
  UnreadableSupersededMemoryView,
} from "../src/lib/api";
import { readerInZone } from "./support/zone";

/**
 * **この測定は、アデレードの読み手として走る**（作業単位 F-7b）。
 *
 * **機械の既定に寄りかからない** —— 寄りかかると、
 * **この機（`Asia/Tokyo`）でしか成立しない期待値**になる。
 * **時差が 30 分の時間帯**を選んである ——
 * **時の桁しか動かさない実装**は、ここでしか落ちない。
 * **配線の権威は `tests/timestamp-wiring.test.ts`。**
 */
readerInZone("Australia/Adelaide");

/** 対象のキャラクター。 */
const HANDLE = "mio";

/** 別のキャラクター（**識別子を決め打つ実装と区別する**）。 */
const OTHER_HANDLE = "aoi";

/**
 * 鎖 —— **`45 → 41 → 42 → 43`**。**`43` だけがいま想起される。**
 *
 * `44` は鎖の外で、後継 `55` は**どちらの一覧にも居ない**（上限の先）。
 */

/** いま想起される行。**`42` の後継**（鎖の先端）。 */
const ACTIVE_LATEST: MemoryView = {
  id: 43,
  subject: "character",
  content: "喫茶店でアルバイトをしている",
  category: "経歴",
  created_at: "2026-08-27T00:00:00Z",
};

/** いま想起される、鎖と無関係な行。 */
const ACTIVE_OTHER: MemoryView = {
  id: 46,
  subject: "user",
  content: "朝は紅茶を飲む",
  category: null,
  created_at: "2026-08-27T00:10:00Z",
};

/** いま想起される、**読めない**行（上書き済みの読めない行と混ざらないための対照）。 */
const ACTIVE_BROKEN: UnreadableMemoryView = {
  id: 47,
  invalid_columns: ["subject"],
};

/** 上書き済み。**`41` を置き換え、`43` に置き換わった**（鎖の途中）。 */
const SUPERSEDED_MID: SupersededMemoryView = {
  id: 42,
  subject: "character",
  content: "喫茶店を経営している",
  category: "職業",
  created_at: "2026-08-26T00:10:00Z",
  superseded_by: 43,
};

/** 上書き済み。**`45` を置き換え、`42` に置き換わった**（鎖の途中）。 */
const SUPERSEDED_OLD: SupersededMemoryView = {
  id: 41,
  subject: "user",
  content: "店を経営している",
  category: "経歴",
  created_at: "2026-08-26T00:00:00Z",
  superseded_by: 42,
};

/** 上書き済み。**後継はこの一覧に出ていない**（分類も未設定）。 */
const SUPERSEDED_LONE: SupersededMemoryView = {
  id: 44,
  subject: "user",
  content: "黒い犬を飼っている",
  category: null,
  created_at: "2026-08-26T00:20:00Z",
  superseded_by: 55,
};

/**
 * 上書き済み。**後継が「いま想起される、読めない行」**（`47`）。
 *
 * **この行が要る理由**: 後継を探すときに**読めない行を数に入れないと、
 * この行だけが「この一覧には出ていません」に見える。**
 * 実際には想起の側に残っていて、**そのキャラクターの会話を止めているのはその行**である。
 */
const SUPERSEDED_TO_BROKEN: SupersededMemoryView = {
  id: 48,
  subject: "character",
  content: "コーヒーという名前の犬を飼っている",
  category: "同居",
  created_at: "2026-08-26T00:30:00Z",
  superseded_by: 47,
};

/**
 * **画面に出る作成時刻の綴り**（作業単位 F-7 → **F-7b で読み手の時間帯になった**）。
 *
 * **⚠ ここに直書きする理由。** `timeLabel` から作ると、
 * **綴りを変える変異が期待値も一緒に動かして緑**になる
 * （CLAUDE.md「写しを1つにしても、その1つは自動的には守られない」）。
 *
 * **⚠ 4件とも別の分**にしてある —— 行どうしの取り違えが見えるように。
 *
 * **⚠ 手で足した値である。** この測定はアデレードの読み手として走り
 * （上の [`readerInZone`]）、**8 月は南半球の冬＝標準時 UTC+9:30。**
 * `00:10Z` に 9 時間 30 分足すと `09:40`。
 * **30 分の時差を落とす実装は、ここで分の桁が合わなくなる。**
 */
const SUPERSEDED_MID_CREATED_LABEL = "2026-08-26 09:40:00 (UTC+09:30)";
/** 同上。 */
const SUPERSEDED_OLD_CREATED_LABEL = "2026-08-26 09:30:00 (UTC+09:30)";
/** 同上。 */
const SUPERSEDED_LONE_CREATED_LABEL = "2026-08-26 09:50:00 (UTC+09:30)";
/** 同上。 */
const SUPERSEDED_TO_BROKEN_CREATED_LABEL = "2026-08-26 10:00:00 (UTC+09:30)";

/** 上書き済みで、**読み出せなかった**行。**鎖の一番奥。** */
const SUPERSEDED_BROKEN: UnreadableSupersededMemoryView = {
  id: 45,
  invalid_columns: ["content", "locked"],
  superseded_by: 41,
};

/**
 * **上限で切られている状態。**
 *
 * # ⚠ **総数・出した件数・上限を、3つとも違う値にすること**
 *
 * **当初は `limit` を 5 にしていた**（出した件数と同じ）。**現実のサーバはそう返す**が、
 * **その形では「直近 N 件」を `limit` に差し替える変異が 406 全緑を通った**
 * （D-3c のレビューで実測）。
 *
 * **⚠ そして当初の doc は「上限と件数の取り違えは `ALL_SHOWN` の腕が見る」と
 * 書いていたが、あれは嘘だった** —— `ALL_SHOWN` は `hidden === 0` なので
 * **「すべて出しています」の枝に入り、その枝は出した件数を一度も描画しない。**
 *
 * **現実らしさより、3つが別の値であることを採る**（`limit: 7`）。
 * この節が守るのは**どの数をどこに出すか**であって、サーバの切り方ではない。
 */
const TRIMMED: SupersededListView = {
  memories: [
    SUPERSEDED_MID,
    SUPERSEDED_OLD,
    SUPERSEDED_LONE,
    SUPERSEDED_TO_BROKEN,
  ],
  unreadable: [SUPERSEDED_BROKEN],
  total: 113,
  limit: 7,
};

/**
 * **全部出ている状態。**
 *
 * **`limit`（9）と出した件数（4）と総数（4）**——
 * **上限を件数として出す変異**はここで赤くなる。
 */
const ALL_SHOWN: SupersededListView = {
  memories: [
    SUPERSEDED_MID,
    SUPERSEDED_OLD,
    SUPERSEDED_LONE,
    SUPERSEDED_TO_BROKEN,
  ],
  unreadable: [SUPERSEDED_BROKEN],
  total: 5,
  limit: 9,
};

/** **1件も上書きされていない状態。** */
const NONE_SUPERSEDED: SupersededListView = {
  memories: [],
  unreadable: [],
  total: 0,
  limit: 100,
};

interface Call {
  method: string;
  url: string;
}

let calls: Call[];
let target: HTMLElement;
/** いま想起される行。 */
let active: MemoryView[];
/** いま想起される、読めない行。 */
let activeBroken: UnreadableMemoryView[];
/** サーバが返す上書き済みの節。**`null` は「1件も無い」ではなく「返さない」。** */
let supersededSection: SupersededListView | null;
/**
 * **`?include=superseded` を無視して節を返さないサーバ。**
 *
 * 古い版のサーバ、あるいは**問い合わせが届いていない**状態。
 * **画面がこれを「0 件」と読まないこと**を見る。
 */
let serverOmitsSuperseded: boolean;
/**
 * **節の中から欄を落とすサーバ**（欄名が変わった / 版が食い違った）。
 *
 * **型は手で写した主張であって、保証ではない**（`api.ts` 冒頭の ⚠）。
 * **サーバは 400 も返さない**ので、**画面が自分で気づくしかない。**
 */
let dropFromSection: string[];
/**
 * **行から欄を落とすサーバ**（例: `superseded_by`）。
 *
 * 落ちたまま描くと **`id undefined に置き換わりました`** になる。
 */
let dropFromRow: string[];
/** 一覧の取得が何回来たか（**2回目から別のものを返す**腕で使う）。 */
let listCalls: number;

/** 節から欄を落とす（**JSON の上で落とす** —— 型の上では表現できない状態）。 */
function damage(section: SupersededListView): unknown {
  const copy = JSON.parse(JSON.stringify(section)) as Record<string, unknown>;
  for (const field of dropFromSection) {
    delete copy[field];
  }
  if (dropFromRow.length > 0) {
    for (const key of ["memories", "unreadable"]) {
      const rows = copy[key];
      if (!Array.isArray(rows)) {
        continue;
      }
      for (const row of rows as Record<string, unknown>[]) {
        for (const field of dropFromRow) {
          delete row[field];
        }
      }
    }
  }
  return copy;
}

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    calls.push({ method, url });

    const respond = (body: unknown, status = 200): Promise<Response> =>
      Promise.resolve({
        ok: status >= 200 && status < 300,
        status,
        text: () => Promise.resolve(JSON.stringify(body)),
      } as unknown as Response);

    listCalls += 1;
    // **サーバと同じ振る舞い**——**訊かれたときだけ節を足す。**
    // 訊かれていなければキーごと無い（`skip_serializing_if`）。
    const asked = new URL(url, "http://127.0.0.1").searchParams.get("include");
    const withSection =
      asked === "superseded" &&
      !serverOmitsSuperseded &&
      supersededSection !== null;

    return respond({
      character: HANDLE,
      memories: active,
      unreadable: activeBroken,
      ...(withSection
        ? { superseded: damage(supersededSection as SupersededListView) }
        : {}),
    });
  });
}

/** その節に並んでいる id。 */
function idsIn(section: string): number[] {
  return [
    ...target.querySelectorAll(`tbody[data-section="${section}"] tr`),
  ].map((row) => Number((row as HTMLElement).dataset["memoryId"]));
}

/** その行のセル。**列の順番ごと見る**（`data-*` に頼らない）。 */
function cellsOf(section: string, id: number): string[] {
  const row = target.querySelector(
    `tbody[data-section="${section}"] tr[data-memory-id="${id}"]`,
  );
  expect(row, `${section} に id ${id} の行があること`).not.toBeNull();
  return [...(row as HTMLElement).querySelectorAll("td")].map((cell) =>
    squash(cell.textContent ?? ""),
  );
}

/** その表の見出しのセル。 */
function headersOf(section: string): string[] {
  const table = target
    .querySelector(`tbody[data-section="${section}"]`)
    ?.closest("table");
  expect(table, `${section} の表があること`).not.toBeNull();
  return [...(table as HTMLTableElement).querySelectorAll("th")].map((cell) =>
    squash(cell.textContent ?? ""),
  );
}

/** 空白を1つに畳む（テンプレートの改行を無視して中身だけ見る）。 */
function squash(text: string): string {
  return text.replace(/\s+/gu, " ").trim();
}

/** 上書き済みの節が出している件数の文。 */
function supersededSummary(): string {
  // **総数の欄ではなく、件数の段落そのものを掴む。**
  // 総数が応答に無い回は `data-superseded-total` が消える（値が `null` だから）ので、
  // そちらで掴むと**いちばん見たい腕だけが空文字になる。**
  return squash(
    target.querySelector("[data-superseded-summary]")?.textContent ?? "",
  );
}

/** 上書き済みの節ぜんぶの文言（見出しから表まで）。 */
function supersededText(): string {
  return squash(
    target.querySelector('[data-block="stored-superseded"]')?.textContent ?? "",
  );
}

async function openWith(handle: string): Promise<ReturnType<typeof mount>> {
  const view = mount(MemoryList, { target, props: { handle } });
  await vi.waitFor(() => {
    expect(
      target.querySelector('[data-block="stored-superseded"]'),
    ).not.toBeNull();
  });
  flushSync();
  return view;
}

async function open(): Promise<ReturnType<typeof mount>> {
  return openWith(HANDLE);
}

beforeEach(() => {
  calls = [];
  active = [ACTIVE_LATEST, ACTIVE_OTHER];
  activeBroken = [ACTIVE_BROKEN];
  supersededSection = TRIMMED;
  serverOmitsSuperseded = false;
  dropFromSection = [];
  dropFromRow = [];
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
  const ids = [
    ACTIVE_LATEST.id,
    ACTIVE_OTHER.id,
    ACTIVE_BROKEN.id,
    SUPERSEDED_MID.id,
    SUPERSEDED_OLD.id,
    SUPERSEDED_LONE.id,
    SUPERSEDED_TO_BROKEN.id,
    SUPERSEDED_BROKEN.id,
  ];
  expect(new Set(ids).size).toBe(ids.length);
  // **id と後継が同じ行は1つも無い。**
  for (const row of [
    SUPERSEDED_MID,
    SUPERSEDED_OLD,
    SUPERSEDED_LONE,
    SUPERSEDED_TO_BROKEN,
    SUPERSEDED_BROKEN,
  ]) {
    expect(row.superseded_by).not.toBe(row.id);
  }
  // **主語は両方が居る**（決め打ちが観測できるように）。
  expect(SUPERSEDED_MID.subject).not.toBe(SUPERSEDED_OLD.subject);
  // **`55` はどちらの一覧にも居ない。**
  expect(ids).not.toContain(SUPERSEDED_LONE.superseded_by);
  // **総数・上限・出した件数は、両方の腕で互いに違う。**
  // **⚠ `TRIMMED` 側にも置くこと** —— ここが同値だと、
  // 「直近 N 件」を `limit` に差し替える変異が通り抜ける（レビューで実測）。
  const trimmedShown = TRIMMED.memories.length + TRIMMED.unreadable.length;
  expect(new Set([TRIMMED.total, TRIMMED.limit, trimmedShown]).size).toBe(3);
  const allShown = ALL_SHOWN.memories.length + ALL_SHOWN.unreadable.length;
  expect(ALL_SHOWN.limit).not.toBe(allShown);
  // **`ALL_SHOWN` は `total === 件数`**（全部出ている、を作るために必然）。
  expect(ALL_SHOWN.total).toBe(allShown);
  expect(HANDLE).not.toBe(OTHER_HANDLE);
});

describe("問い合わせ", () => {
  it("上書きされた記憶を足して引く（`?include=superseded`）", async () => {
    const view = await open();

    const listing = calls.find((call) => call.method === "GET");
    expect(listing?.url).toBe(
      `/admin/characters/${HANDLE}/memories?include=superseded`,
    );

    await unmount(view, { outro: false });
  });

  /**
   * **識別子は prop から来る。**
   *
   * **フィクスチャが1つしか無いと、決め打ちと区別が付かない**
   * （D-3 のレビューで、`listMemories("mio")` に決め打つ変異が全緑だった）。
   */
  it("別のキャラクターで開くと、その識別子へ問い合わせる", async () => {
    const view = await openWith(OTHER_HANDLE);

    const listing = calls.find((call) => call.method === "GET");
    expect(listing?.url).toBe(
      `/admin/characters/${OTHER_HANDLE}/memories?include=superseded`,
    );
    expect(listing?.url).not.toContain(HANDLE);

    await unmount(view, { outro: false });
  });
});

describe("DB に残っている上書き済みの記憶", () => {
  /**
   * **この画面が訂正していなくても出る。**
   *
   * D-3 の画面は**その画面が `POST` を通した行しか並べられなかった**
   * ——別のキャラクターへ移って戻れば消え、再読み込みでも消えた。
   * **ここでは1度も `POST` していない。**
   */
  it("読み込んだだけで、上書き済みの行が並ぶ", async () => {
    const view = await open();

    expect(calls.every((call) => call.method === "GET")).toBe(true);
    expect(idsIn("stored-superseded")).toEqual([
      SUPERSEDED_MID.id,
      SUPERSEDED_OLD.id,
      SUPERSEDED_LONE.id,
      SUPERSEDED_TO_BROKEN.id,
    ]);
    expect(idsIn("stored-superseded-unreadable")).toEqual([
      SUPERSEDED_BROKEN.id,
    ]);

    await unmount(view, { outro: false });
  });

  /**
   * **再読み込みで消えない。**
   *
   * **画面を作り直して**（`unmount` → `mount`）も、行はサーバから来る。
   * D-3 の「画面と同じ寿命しか持たない」記録との違いはここ。
   */
  it("画面を作り直しても、同じ行が並ぶ", async () => {
    const first = await open();
    expect(idsIn("stored-superseded")).toHaveLength(4);
    await unmount(first, { outro: false });
    expect(idsIn("stored-superseded")).toEqual([]);

    const second = await open();
    expect(idsIn("stored-superseded")).toEqual([
      SUPERSEDED_MID.id,
      SUPERSEDED_OLD.id,
      SUPERSEDED_LONE.id,
      SUPERSEDED_TO_BROKEN.id,
    ]);

    await unmount(second, { outro: false });
  });

  /**
   * **読み直すと、サーバの新しい答えに入れ替わる。**
   *
   * **⚠ `vi.waitFor` は最初に成立した時点で返る**ので、
   * **「もともと出ていた」腕では何も観測できない**（`D-4` の実測）。
   * **1回目は空、2回目から並ぶ**ようにして、押す前に空であることを確かめる。
   */
  it("再読み込みを押すと、サーバの新しい答えが出る", async () => {
    supersededSection = NONE_SUPERSEDED;
    const view = await open();
    // **押す前は空**（あとで待つものが、まだ成立していない）。
    expect(idsIn("stored-superseded")).toEqual([]);
    expect(supersededSummary()).toContain("まだ1件も");

    supersededSection = TRIMMED;
    const reload = [...target.querySelectorAll("button")].find(
      (button) => button.textContent?.trim() === "再読み込み",
    );
    expect(reload, "再読み込みのボタンがあること").toBeDefined();
    (reload as HTMLButtonElement).click();

    await vi.waitFor(() => {
      expect(idsIn("stored-superseded")).toEqual([
        SUPERSEDED_MID.id,
        SUPERSEDED_OLD.id,
        SUPERSEDED_LONE.id,
        SUPERSEDED_TO_BROKEN.id,
      ]);
    });
    expect(listCalls).toBeGreaterThanOrEqual(2);

    await unmount(view, { outro: false });
  });

  /**
   * **セルは、その行のもの。**
   *
   * **列ごと突き合わせる**——`data-*` を見ていると、
   * **`<td>` を入れ替える変異が通る**（`D-4` のレビューで実測）。
   */
  it("行のセルが、列の順番どおりに出る", async () => {
    const view = await open();

    expect(headersOf("stored-superseded")).toEqual([
      "id",
      "主語",
      "本文",
      "分類",
      "作成",
      "置き換わった先",
      "鎖",
    ]);
    expect(cellsOf("stored-superseded", SUPERSEDED_MID.id)).toEqual([
      String(SUPERSEDED_MID.id),
      "キャラクター",
      SUPERSEDED_MID.content,
      SUPERSEDED_MID.category as string,
      SUPERSEDED_MID_CREATED_LABEL,
      `id ${SUPERSEDED_MID.superseded_by} に置き換わりました（いま想起されます）`,
      `id ${SUPERSEDED_OLD.id} を置き換えた版です（鎖の途中）`,
    ]);
    expect(cellsOf("stored-superseded", SUPERSEDED_OLD.id)).toEqual([
      String(SUPERSEDED_OLD.id),
      "利用者",
      SUPERSEDED_OLD.content,
      SUPERSEDED_OLD.category as string,
      SUPERSEDED_OLD_CREATED_LABEL,
      `id ${SUPERSEDED_OLD.superseded_by} に置き換わりました（さらに訂正されました）`,
      `id ${SUPERSEDED_BROKEN.id} を置き換えた版です（鎖の途中）`,
    ]);
    // **分類が未設定の行**と、**後継がこの一覧に居ない行。**
    expect(cellsOf("stored-superseded", SUPERSEDED_LONE.id)).toEqual([
      String(SUPERSEDED_LONE.id),
      "利用者",
      SUPERSEDED_LONE.content,
      "未設定",
      SUPERSEDED_LONE_CREATED_LABEL,
      `id ${SUPERSEDED_LONE.superseded_by} に置き換わりました（この一覧には出ていません）`,
      "この一覧に、この行が置き換えた行はありません",
    ]);

    await unmount(view, { outro: false });
  });

  /**
   * **後継が「いま想起される、読めない行」でも、そう見えること。**
   *
   * **読めない行を「いま想起される行」に数えない実装だと、この行だけが
   * 「この一覧には出ていません」に見える。**
   * 実際には想起の側に残っていて、**会話と抽出を止めているのはその行**である
   * ——「もう無い」と読むと、**直すべき行から目が逸れる。**
   */
  it("後継が読めない行でも、いま想起される側に居るとして出る", async () => {
    const view = await open();

    expect(cellsOf("stored-superseded", SUPERSEDED_TO_BROKEN.id)).toEqual([
      String(SUPERSEDED_TO_BROKEN.id),
      "キャラクター",
      SUPERSEDED_TO_BROKEN.content,
      SUPERSEDED_TO_BROKEN.category as string,
      SUPERSEDED_TO_BROKEN_CREATED_LABEL,
      `id ${SUPERSEDED_TO_BROKEN.superseded_by} に置き換わりました（いま想起されます）`,
      "この一覧に、この行が置き換えた行はありません",
    ]);
    // **前提: その後継は「いま想起される、読めない行」として並んでいる。**
    expect(idsIn("unreadable")).toContain(SUPERSEDED_TO_BROKEN.superseded_by);

    await unmount(view, { outro: false });
  });

  /**
   * **読めない上書き済みの行を落とさない**（ADR-0022 §2）。
   *
   * 落とすと**壊れた環だけが鎖から落ち、`total` にだけ残る**——
   * 読み手には**数が合わない**としか見えない。
   * **`superseded_by` は壊れた行でも載る**ので、**鎖はここで切れない。**
   */
  it("読めない上書き済みの行も、壊れた列と後継つきで出る", async () => {
    const view = await open();

    expect(headersOf("stored-superseded-unreadable")).toEqual([
      "id",
      "読めなかった列",
      "置き換わった先",
      "鎖",
    ]);
    expect(
      cellsOf("stored-superseded-unreadable", SUPERSEDED_BROKEN.id),
    ).toEqual([
      String(SUPERSEDED_BROKEN.id),
      SUPERSEDED_BROKEN.invalid_columns.join(", "),
      `id ${SUPERSEDED_BROKEN.superseded_by} に置き換わりました（さらに訂正されました）`,
      "この一覧に、この行が置き換えた行はありません",
    ]);
    // **いま想起される読めない行とは、別の節。**
    expect(idsIn("unreadable")).toEqual([ACTIVE_BROKEN.id]);
    expect(idsIn("unreadable")).not.toContain(SUPERSEDED_BROKEN.id);
    expect(idsIn("stored-superseded-unreadable")).not.toContain(
      ACTIVE_BROKEN.id,
    );

    await unmount(view, { outro: false });
  });

  /**
   * **帰属** —— 上書き済みの行を、想起される側に混ぜない。
   *
   * 混ぜると、**二度と想起されない行が「いま想起される」ものとして読まれ**、
   * その上に訂正版を書こうとして `memory_already_superseded` に当たる。
   */
  it("上書き済みの行は、いま想起される側に出ない", async () => {
    const view = await open();

    expect(idsIn("active")).toEqual([ACTIVE_LATEST.id, ACTIVE_OTHER.id]);
    for (const row of [SUPERSEDED_MID, SUPERSEDED_OLD, SUPERSEDED_LONE]) {
      expect(idsIn("active")).not.toContain(row.id);
    }
    expect(idsIn("stored-superseded")).not.toContain(ACTIVE_LATEST.id);
    expect(idsIn("stored-superseded")).not.toContain(ACTIVE_OTHER.id);

    await unmount(view, { outro: false });
  });

  /**
   * **上書き済みの行には訂正の導線を出さない。**
   *
   * サーバは `memory_already_superseded` で拒む（**もう訂正もできない**）。
   * 押せる形にすると、**必ず 409 になるボタン**を画面に置くことになる。
   */
  it("上書き済みの行に「訂正版を足す」は無い", async () => {
    const view = await open();

    for (const section of [
      "stored-superseded",
      "stored-superseded-unreadable",
    ]) {
      const buttons = [
        ...target.querySelectorAll(
          `tbody[data-section="${section}"] tr button`,
        ),
      ];
      expect(buttons).toEqual([]);
    }
    // **対照: 想起される側には出ている**（節ごと消していない）。
    expect(
      target.querySelectorAll('tbody[data-section="active"] tr button').length,
    ).toBeGreaterThan(0);

    await unmount(view, { outro: false });
  });

  /**
   * **有効な行が1件も無くても、上書き済みは出る。**
   *
   * 全部を訂正したキャラクターがこの状態になる。
   * **有効な行の下にぶら下げる実装だと、ここで節ごと消える。**
   */
  it("いま想起される記憶が1件も無くても、上書き済みは並ぶ", async () => {
    active = [];
    activeBroken = [];
    const view = await open();

    expect(idsIn("active")).toEqual([]);
    expect(idsIn("stored-superseded")).toEqual([
      SUPERSEDED_MID.id,
      SUPERSEDED_OLD.id,
      SUPERSEDED_LONE.id,
      SUPERSEDED_TO_BROKEN.id,
    ]);
    // **後継が有効な行として居ないので、鎖の先端は「出ていません」になる。**
    expect(cellsOf("stored-superseded", SUPERSEDED_MID.id)).toEqual([
      String(SUPERSEDED_MID.id),
      "キャラクター",
      SUPERSEDED_MID.content,
      SUPERSEDED_MID.category as string,
      SUPERSEDED_MID_CREATED_LABEL,
      `id ${SUPERSEDED_MID.superseded_by} に置き換わりました（この一覧には出ていません）`,
      `id ${SUPERSEDED_OLD.id} を置き換えた版です（鎖の途中）`,
    ]);

    await unmount(view, { outro: false });
  });
});

describe("総数と上限", () => {
  /**
   * **総数は「返した件数」ではない。**
   *
   * **返した件数を総数と呼ぶと、画面が「これで全部」と嘘をつく**
   * （`admin.rs` の `RECENT_SUPERSEDED_MEMORIES` の doc / D-4a と同じ立場）。
   * **読めなかった行も、出した件数と総数の両方に入る。**
   */
  it("上限で切られていたら、総数・出した件数・上限・残りを全部言う", async () => {
    const view = await open();

    const summary = supersededSummary();
    // **出した件数は 5**（読めた4件 ＋ 読めなかった1件）。**総数は 113。**
    expect(summary).toContain(`全 ${TRIMMED.total} 件`);
    expect(summary).toContain("直近 5 件");
    expect(summary).toContain(`上限は ${TRIMMED.limit} 件`);
    expect(summary).toContain(`残り ${TRIMMED.total - 5} 件`);
    // **「これで全部」と読める文言を出さない。**
    expect(summary).not.toContain("すべて出しています");

    await unmount(view, { outro: false });
  });

  /**
   * **全部出ているときは、そう言う。**
   *
   * この腕は**上限（9）と出した件数（4）が違う**ので、
   * **上限を件数として出す変異**がここで赤くなる。
   */
  it("全部出ているときは、上限とともにそう言う", async () => {
    supersededSection = ALL_SHOWN;
    const view = await open();

    const summary = supersededSummary();
    expect(summary).toContain(`全 ${ALL_SHOWN.total} 件`);
    expect(summary).toContain("すべて出しています");
    expect(summary).toContain(`上限は ${ALL_SHOWN.limit} 件`);
    expect(summary).not.toContain("残り");

    await unmount(view, { outro: false });
  });

  it("1件も上書きされていなければ、そう言う（節は消さない）", async () => {
    supersededSection = NONE_SUPERSEDED;
    const view = await open();

    expect(supersededSummary()).toContain("まだ1件も");
    expect(idsIn("stored-superseded")).toEqual([]);
    // **節そのものは残る**（「無い」ことと「この画面が出さない」ことの区別）。
    expect(supersededText()).toContain("上書きされた記憶");

    await unmount(view, { outro: false });
  });

  /**
   * **節が来なかったときに「0 件」と言わない。**
   *
   * **⚠ これは「訊いたが1件も無い」とは別の状態である。**
   * 問い合わせが届いていない（あるいは古い版のサーバ）だけで、
   * **DB には上書き済みの行がいくらでもありうる。**
   * 0 件と言い切ると、**その画面は嘘をつく。**
   */
  it("応答に節が無ければ、「0 件」ではなく「取得していない」と言う", async () => {
    serverOmitsSuperseded = true;
    const view = await open();

    const text = supersededText();
    expect(text).toContain("節がありませんでした");
    expect(text).not.toContain("まだ1件も");
    expect(text).not.toContain("すべて出しています");
    expect(target.querySelector("[data-superseded-total]")).toBeNull();

    await unmount(view, { outro: false });
  });
});

/**
 * **応答の欄が欠けたとき、画面が何と言うか**（D-3c のレビューで実測 / 2026-08-28）。
 *
 * # なぜ要るのか
 *
 * **応答の形は誰も検証していない**（`api.ts` 冒頭の ⚠ ——
 * 型は `admin.rs` から**手で写した主張**であって、保証ではない）。
 * **サーバが欄名を変えても 400 は返らない**ので、**画面が自分で気づくしかない。**
 *
 * **⚠ 当初の実装は、欄を1つ落とすだけで次のようになっていた**（レビュアーの実測）:
 *
 * | 落ちた欄 | 当時の振る舞い |
 * |---|---|
 * | `memories` / `unreadable` | **画面全体が「読み込み中…」で固まる**（`$derived` の中で `TypeError`）。**有効な行も「訂正版を足す」も1つも出ない** |
 * | `total` | 「**全　件…すべて出しています**」——**空白のまま「これで全部」と言い切る** |
 * | `superseded_by` | 「**id undefined に置き換わりました**」——**鎖が黙って嘘をつく** |
 *
 * **`?include=` の綴りに写し検査を置いた理由は「ずれると一覧ごと出なくなる」だった。**
 * **欄名がずれると、400 も出ないまま同じことが起きる**——**修復の入口が閉じる。**
 * しかも**利用者にはサーバが遅いようにしか見えない。**
 */
describe("応答の欄が欠けたとき", () => {
  /**
   * **画面が生きていること。**
   *
   * **上書き済みの節は、この画面の付け足しである。**
   * そこの欄が1つ欠けたくらいで、**修復の入口（有効な行と訂正のボタン）を
   * 道連れにしてはいけない。**
   */
  function expectScreenAlive(): void {
    expect(target.textContent).not.toContain("読み込み中");
    expect(idsIn("active")).toEqual([ACTIVE_LATEST.id, ACTIVE_OTHER.id]);
    expect(idsIn("unreadable")).toEqual([ACTIVE_BROKEN.id]);
    expect(
      [...target.querySelectorAll("button")].some(
        (button) => button.textContent?.trim() === "訂正版を足す",
      ),
      "「訂正版を足す」が1つ以上あること",
    ).toBe(true);
    // **`undefined` を画面に出さない**（欄が無いことを、値のように見せない）。
    expect(supersededText()).not.toContain("undefined");
  }

  it("`memories` が欠けても、画面は生き、残りの行は出る", async () => {
    dropFromSection = ["memories"];
    const view = await open();

    expectScreenAlive();
    expect(idsIn("stored-superseded")).toEqual([]);
    // **読めない上書き済みの行は残る**（片方が欠けても、もう片方を道連れにしない）。
    expect(idsIn("stored-superseded-unreadable")).toEqual([
      SUPERSEDED_BROKEN.id,
    ]);
    // **出した件数は、実際に来た1件**（読めない行も数える）。
    expect(supersededSummary()).toContain("直近 1 件");
    expect(supersededSummary()).toContain(`残り ${TRIMMED.total - 1} 件`);

    await unmount(view, { outro: false });
  });

  it("`unreadable` が欠けても、画面は生き、残りの行は出る", async () => {
    dropFromSection = ["unreadable"];
    const view = await open();

    expectScreenAlive();
    expect(idsIn("stored-superseded")).toEqual([
      SUPERSEDED_MID.id,
      SUPERSEDED_OLD.id,
      SUPERSEDED_LONE.id,
      SUPERSEDED_TO_BROKEN.id,
    ]);
    expect(idsIn("stored-superseded-unreadable")).toEqual([]);
    expect(supersededSummary()).toContain("直近 4 件");
    expect(supersededSummary()).toContain(`残り ${TRIMMED.total - 4} 件`);

    await unmount(view, { outro: false });
  });

  /**
   * **⚠ この画面がいちばん避けたい形。**
   *
   * 総数が無いまま描くと「**全　件の上書きされた記憶を、すべて出しています**」——
   * **数を1つも言わないまま「これで全部」と言い切る。**
   */
  it("`total` が欠けたら、「すべて出しています」と言わない", async () => {
    dropFromSection = ["total"];
    const view = await open();

    expectScreenAlive();
    const summary = supersededSummary();
    expect(summary).toContain("総数が、応答にありませんでした");
    expect(summary).toContain("これで全部とは限りません");
    expect(summary).toContain("直近 5 件");
    expect(summary).not.toContain("すべて出しています");
    expect(summary).not.toContain("まだ1件も");
    // **総数を作らない**（欄が無いのだから、生の値も出ない）。
    expect(target.querySelector("[data-superseded-total]")).toBeNull();
    // **行は出したまま**（言えないのは総数だけ）。
    expect(idsIn("stored-superseded")).toHaveLength(4);

    await unmount(view, { outro: false });
  });

  it("`limit` が欠けたら、`undefined 件` と書かずにそう言う", async () => {
    dropFromSection = ["limit"];
    const view = await open();

    expectScreenAlive();
    const summary = supersededSummary();
    expect(summary).toContain("上限は 応答にありませんでした");
    expect(summary).toContain(`全 ${TRIMMED.total} 件`);
    expect(summary).toContain("直近 5 件");

    await unmount(view, { outro: false });
  });

  /**
   * **後継が欠けたら、辿れないと言う。**
   *
   * `id undefined に置き換わりました` は、**辿れる先があるかのように読める。**
   * **鎖が黙って嘘をつく**のが、この節でいちばん質の悪い壊れ方
   * ——「以前はXだったが今はY」の **Y へ辿る唯一の手掛かり**だから。
   */
  it("行の `superseded_by` が欠けたら、鎖を辿れないと書く", async () => {
    dropFromRow = ["superseded_by"];
    const view = await open();

    expectScreenAlive();
    expect(cellsOf("stored-superseded", SUPERSEDED_MID.id)).toEqual([
      String(SUPERSEDED_MID.id),
      "キャラクター",
      SUPERSEDED_MID.content,
      SUPERSEDED_MID.category as string,
      SUPERSEDED_MID_CREATED_LABEL,
      "置き換わった先が応答にありませんでした（鎖を辿れません）",
      // **鎖の途中の判定も消える**（誰も誰を指していない）。
      "この一覧に、この行が置き換えた行はありません",
    ]);
    expect(
      cellsOf("stored-superseded-unreadable", SUPERSEDED_BROKEN.id),
    ).toEqual([
      String(SUPERSEDED_BROKEN.id),
      SUPERSEDED_BROKEN.invalid_columns.join(", "),
      "置き換わった先が応答にありませんでした（鎖を辿れません）",
      "この一覧に、この行が置き換えた行はありません",
    ]);
    // **他の欄は無事**（後継だけが言えない）。
    expect(idsIn("stored-superseded")).toHaveLength(4);

    await unmount(view, { outro: false });
  });
});

/**
 * **上書き済みの表でも、落とした精度を `title` に残す**（作業単位 F-7 / レビューの実測）。
 *
 * # ⚠ なぜ、この1本が要ったのか
 *
 * **想起される側（`memory-list.test.ts`）は2行ぶん見られているのに、
 * 上書き済み側は誰も見ていなかった** ——
 * `title={row.memory.created_at}` を `row.memory.content` にしても、
 * **`title` を丸ごと落としても、631 全緑**だった（2026-08-29）。
 *
 * **「写した先で列が1つだけ落ちる」形**である。
 * そこのコメント自身が「上の表と同じ扱い」と書いていて、
 * **`timestamp.ts` の「呼び出し側は生の綴りを `title` に残すこと」が、
 * ここでは1度も観測されていなかった。**
 *
 * # ⚠ 4行ぶんを配列で突き合わせる
 *
 * 1行だけ見ると、**「常に先頭」「常に末尾」「常に定数」**が通り抜ける
 * （`D-2` の実測）。**4件とも別の分**にしてあるので、どの決め打ちも赤になる。
 */
describe("上書き済みの表も、生の綴りを `title` に残す（作業単位 F-7）", () => {
  /** その節の各行の、`作成` の欄（5列目）の `title`。**並びは表のまま。** */
  function createdTitles(section: string): (string | null)[] {
    return [
      ...target.querySelectorAll(`tbody[data-section="${section}"] tr`),
    ].map((row) => {
      const cell = row.querySelectorAll("td")[4];
      expect(cell, "作成の欄があること").toBeDefined();
      return (cell as HTMLTableCellElement).getAttribute("title");
    });
  }

  it("行ごとに、その行の生の綴りが `title` に入る", async () => {
    const view = await open();

    // **前提**: 見出しの5列目が `作成`（列がずれていたら、この検査は別の欄を見ている）。
    expect(headersOf("stored-superseded")[4]).toBe("作成");
    // **並びは表のまま**（`TRIMMED` の順）。**4件とも別の分。**
    expect(createdTitles("stored-superseded")).toEqual([
      SUPERSEDED_MID.created_at,
      SUPERSEDED_OLD.created_at,
      SUPERSEDED_LONE.created_at,
      SUPERSEDED_TO_BROKEN.created_at,
    ]);

    await unmount(view, { outro: false });
  });

  /**
   * **`title` は時刻であって、本文ではない。**
   *
   * 別の列を入れる変異は、上の配列でも赤になるが、
   * **「時刻の形をしていること」を別に押さえておく** ——
   * 列を入れ替えたときに、何が起きたのかが読める。
   */
  it("`title` が本文や分類になっていない", async () => {
    const view = await open();

    for (const title of createdTitles("stored-superseded")) {
      expect(title, `時刻の綴りであること: ${title}`).toMatch(
        /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/,
      );
    }
    // **対照: 本文はどの `title` にも入っていない。**
    expect(createdTitles("stored-superseded")).not.toContain(
      SUPERSEDED_MID.content,
    );

    await unmount(view, { outro: false });
  });

  /**
   * **見出しは畳んだ綴り、`title` は生の綴り。**
   *
   * 2つが同じになると、**畳んでいないか、残していないか**のどちらか。
   * **同じ行の中で突き合わせる。**
   */
  it("見出しと `title` が、同じ行の中で別の綴りになっている", async () => {
    const view = await open();

    const shown = cellsOf("stored-superseded", SUPERSEDED_MID.id)[4];
    const raw = createdTitles("stored-superseded")[0];

    expect(shown).toBe(SUPERSEDED_MID_CREATED_LABEL);
    expect(raw).toBe(SUPERSEDED_MID.created_at);
    expect(shown).not.toBe(raw);

    await unmount(view, { outro: false });
  });
});
