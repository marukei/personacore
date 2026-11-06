// @vitest-environment jsdom
/**
 * **属性と変更の記録の画面**（作業単位 D-12a / `src/lib/AttributesPanel.svelte`）。
 *
 * # この画面の存在理由
 *
 * **属性は「削れない枠」に毎ターン載る**（ADR-0009 / 13d の実測 ——
 * 名指しした事実はドリフトゼロ）。**同じ理由で、古くなっても毎ターン載る。**
 * そして **`GET .../attributes` が答えるのは「いまどうなっているか」だけ**なので、
 * **静かに変わったことに気付く手段が、履歴の表以外に無い**（ADR-0028 §決定5）。
 *
 * **したがって、この画面は変更の記録を必ず出す。**
 * 出さなければ、画面があっても D-12 の目的は半分しか果たされない。
 *
 * # `attributes-edit.test.ts` との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 送る本文・要約の数・案内の文言・写しの突き合わせ | `attributes-edit.test.ts` |
 * | **画面が何を出すか・どの識別子へ送るか・失敗の見え方** | **ここ** |
 *
 * # ⚠ 時間帯の綴りを直書きされたら落ちること
 *
 * この測定は **`Pacific/Marquesas`（UTC−09:30 / 夏時間なし）**の読み手として走り、
 * **`app-routing-attributes.test.ts` は `Asia/Katmandu`（UTC+05:45）**を名乗る。
 * **消費側を2本にして、別々の時間帯を名乗らせてある** ——
 * **どちらの綴りを直書きしても、必ずもう一方が赤くなる**
 * （`MemoryList` と同じ冗長度 / `stats-panel.test.ts` 冒頭の ⚠ の実測）。
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import AttributesPanel from "../src/lib/AttributesPanel.svelte";
import type {
  AttributeRevisionsView,
  AttributesView,
  ReplaceAttributesRequest,
} from "../src/lib/api";
import {
  ATTRIBUTE_LIMITS,
  DUPLICATE_ATTRIBUTE_CODE,
  KEY_CONTROL_CODE,
  MAX_ATTRIBUTES,
  NOT_FOUND_CODE,
  VALUE_CONTROL_CODE,
} from "../src/lib/attributesEdit";
import { readerInZone } from "./support/zone";

readerInZone("Pacific/Marquesas");

/** 見ているキャラクター。 */
const MIO = "mio";
/** **別のキャラクター。** 決め打ちの変異はこちらのデータを出す。 */
const NAR = "nar";

/**
 * `mio` のいまの値。
 *
 * **⚠ 3件で、直すのは真ん中**（`D-2` のレビューの実測 ——
 * 端を触ると「常に先頭」「常に最終」の変異が片側だけ通り抜ける）。
 *
 * **⚠ 主語をまたいで同じキーがある**（`年齢`）——
 * **`(subject, key)` で突き合わせていることの唯一の証拠。**
 *
 * **⚠ 値・キー・主語は互いに違う綴り**（取り違えが観測できるように）。
 */
const MIO_ATTRIBUTES: AttributesView = {
  character: MIO,
  attributes: [
    { subject: "character", key: "一人称", value: "僕" },
    { subject: "character", key: "年齢", value: "19" },
    { subject: "user", key: "年齢", value: "32" },
  ],
};

/** `nar` のいまの値。**`mio` と1文字も重ならない。** */
const NAR_ATTRIBUTES: AttributesView = {
  character: NAR,
  attributes: [{ subject: "user", key: "住所", value: "京都" }],
};

/**
 * `mio` の変更の記録。**上限で切られている状態**（`total_revisions > revisions.length`）。
 *
 * **⚠ 上限を 4 にしてある**（サーバの既定は 200）——
 * **応答の `revisions_limit` を読んでいることの証拠。**
 * 写しを持っている実装は、ここで 200 を出す。
 *
 * **⚠ 4行で「変更」「追加」「削除」の3通り**を覆っている。
 *
 * # ⚠ **利用者の行が2件ある**（`年齢` と `好物`）
 *
 * **1件しか無いと、履歴の主語の帰属を見る腕が「その1行」だけに寄りかかる**
 * （D-12a のレビューの指摘 / 2026-09-09）。
 * 並びは **`キャラクター / 利用者 / 利用者 / キャラクター`** なので、
 * **主語を先頭行のもので埋める変異も、最終行のもので埋める変異も落ちる**
 * （`D-2` のレビューの「端を押すと片側だけ通り抜ける」と同じ形）。
 *
 * **⚠ ただし、2件にして新しく捕まる変異は無い**（実測 / `web/README.md` の表）——
 * **1件だった頃も、赤くなるのは同じ1本**（下の `日時・主語・キー・前・後を、その順で出す`）。
 * **増えたのは「その1本の中で証人になる行」が1行から2行になったことだけ**である。
 * **「錨を足した」を「守った」と読まないこと**（CLAUDE.md / `G-5`）。
 *
 * **⚠ キーは全部違う綴りで、`null` でない値も互いに違う**
 * （どの2つを取り違えても観測できる）。
 */
const MIO_REVISIONS: AttributeRevisionsView = {
  character: MIO,
  revisions: [
    {
      id: 31,
      subject: "character",
      key: "一人称",
      before: "私",
      after: "僕",
      created_at: "2026-08-27T09:00:00Z",
    },
    {
      id: 30,
      subject: "user",
      key: "年齢",
      before: null,
      after: "32",
      created_at: "2026-08-26T08:00:00Z",
    },
    {
      id: 29,
      subject: "user",
      key: "好物",
      before: "珈琲",
      after: "紅茶",
      created_at: "2025-01-02T03:04:05Z",
    },
    {
      id: 28,
      subject: "character",
      key: "種族",
      before: "猫獣人",
      after: null,
      created_at: "2024-05-04T03:02:01.100Z",
    },
  ],
  total_revisions: 9,
  revisions_limit: 4,
};

/** 保存した後に増える1件（**保存前の応答には無い**）。 */
const SAVED_REVISION: AttributeRevisionsView["revisions"][number] = {
  id: 32,
  subject: "character",
  key: "年齢",
  before: "19",
  after: "20",
  created_at: "2026-08-28T10:00:00Z",
};

/** `nar` の変更の記録。**`mio` とは総数も上限も違う。** */
const NAR_REVISIONS: AttributeRevisionsView = {
  character: NAR,
  revisions: [],
  total_revisions: 0,
  revisions_limit: 50,
};

/**
 * **画面に出る時刻の綴り**（`Pacific/Marquesas` = UTC−09:30）。
 *
 * **⚠ ここに直書きする理由。** `timeLabel` から作ると、
 * **綴りを変える変異が期待値も一緒に動かして緑**になる
 * （CLAUDE.md「写しを1つにしても、その1つは自動的には守られない」）。
 *
 * **⚠ 4件とも日付が前日へ戻る**（時差が負で、しかも 30 分ある）ので、
 * **日・時・分の3桁が全部動く。**
 */
const FIRST_REVISION_LABEL = "2026-08-26 23:30:00 (UTC-09:30)";
/** 同上。 */
const SECOND_REVISION_LABEL = "2026-08-25 22:30:00 (UTC-09:30)";
/** 同上。**2件目の利用者の行**（主語の帰属を1行に寄りかからせないために足した）。 */
const THIRD_REVISION_LABEL = "2025-01-01 17:34:05 (UTC-09:30)";
/** 同上。**ミリ秒は畳まれる**（生の綴りは `title` に残る）。 */
const FOURTH_REVISION_LABEL = "2024-05-03 17:32:01 (UTC-09:30)";
/** 保存した後に増える1件の綴り。**日付は動かない**（時差を足しても同じ日）。 */
const SAVED_REVISION_LABEL = "2026-08-28 00:30:00 (UTC-09:30)";

let target: HTMLElement;
/** 送られた `PUT` の本文（**宛先つき**）。 */
let puts: { url: string; body: ReplaceAttributesRequest }[];
/** 引いた URL（順のまま）。 */
let urls: string[];
/** いまの値の読みを失敗させる。 */
let attributesFailure: { status: number; code: string | null } | null;
/** 変更の記録の読みを失敗させる。 */
let revisionsFailure: { status: number; code: string | null } | null;
/** 置き換えを失敗させる。 */
let putFailure: { status: number; code: string | null; message: string } | null;
/** 保存が通ったか（**通った後は記録が1件増える**）。 */
let saved: boolean;

function respond(body: unknown): Promise<Response> {
  return Promise.resolve({
    ok: true,
    status: 200,
    text: () => Promise.resolve(JSON.stringify(body)),
  } as unknown as Response);
}

function refuse(
  status: number,
  code: string | null,
  message: string,
): Promise<Response> {
  return Promise.resolve({
    ok: false,
    status,
    text: () =>
      Promise.resolve(
        JSON.stringify({
          error: {
            message,
            type: "invalid_request_error",
            ...(code === null ? {} : { code }),
          },
        }),
      ),
  } as unknown as Response);
}

function revisionsOf(handle: string): AttributeRevisionsView {
  if (handle !== MIO) {
    return NAR_REVISIONS;
  }
  return saved
    ? {
        ...MIO_REVISIONS,
        revisions: [SAVED_REVISION, ...MIO_REVISIONS.revisions],
        total_revisions: MIO_REVISIONS.total_revisions + 1,
      }
    : MIO_REVISIONS;
}

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    urls.push(`${method} ${url}`);

    for (const handle of [MIO, NAR]) {
      const base = `/admin/characters/${handle}/attributes`;
      if (url === `${base}/revisions`) {
        if (revisionsFailure !== null) {
          return refuse(
            revisionsFailure.status,
            revisionsFailure.code,
            "記録を読めませんでした",
          );
        }
        return respond(revisionsOf(handle));
      }
      if (url === base && method === "GET") {
        if (attributesFailure !== null) {
          return refuse(
            attributesFailure.status,
            attributesFailure.code,
            "いまの値を読めませんでした",
          );
        }
        return respond(handle === MIO ? MIO_ATTRIBUTES : NAR_ATTRIBUTES);
      }
      if (url === base && method === "PUT") {
        const body = JSON.parse(String(init?.body)) as ReplaceAttributesRequest;
        puts.push({ url, body });
        if (putFailure !== null) {
          return refuse(putFailure.status, putFailure.code, putFailure.message);
        }
        saved = true;
        return respond({ character: handle, attributes: body.attributes });
      }
    }
    // **知らない宛先**（決め打ちの変異はここへ来る）。**空を返す。**
    return respond({
      character: "?",
      attributes: [],
      revisions: [],
      total_revisions: 0,
      revisions_limit: 0,
    });
  });
}

beforeEach(() => {
  target = document.createElement("div");
  document.body.append(target);
  puts = [];
  urls = [];
  attributesFailure = null;
  revisionsFailure = null;
  putFailure = null;
  saved = false;
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

/** 画面を開く。**変更の記録の件数の案内が出るまで待つ。** */
async function show(
  handle: string = MIO,
): Promise<{ dispose: () => Promise<void> }> {
  const view = mount(AttributesPanel, { target, props: { handle } });
  await vi.waitFor(() => {
    expect(target.querySelector("[data-revision-total]")).not.toBeNull();
  });
  flushSync();
  return {
    dispose: async () => {
      await unmount(view, { outro: false });
    },
  };
}

/** 画面を開く（**待たない**）。失敗の枝で使う。 */
function open(handle: string = MIO): { dispose: () => Promise<void> } {
  const view = mount(AttributesPanel, { target, props: { handle } });
  return {
    dispose: async () => {
      await unmount(view, { outro: false });
    },
  };
}

/** 空白の並び（改行と字下げ）。 */
const WHITESPACE = /\s+/g;

/** 空白を畳んだ文面。**折り返しの位置に寄りかからない。** */
function flatText(element: Element): string {
  return (element.textContent ?? "").replace(WHITESPACE, " ").trim();
}

/**
 * **その行のセルの中身**（＝**読み手が実際に見る値**）。
 *
 * **`data-*` はテストが置いた印であって、読み手が何を見るかの権威ではない**
 * （CLAUDE.md / `D-4` の実測 —— セルを入れ替える変異が 372 全緑を通った）。
 * **だから列を配列で突き合わせる。**
 */
function cellsOf(row: Element): string[] {
  return [...row.querySelectorAll("td")].map((cell) => flatText(cell));
}

/** いまの値の行。**`(subject, key)` で引く**（並びに寄りかからない）。 */
function attributeRow(subject: string, key: string): HTMLElement {
  const found = target.querySelector(
    `tr[data-attribute-subject="${subject}"][data-attribute-key="${key}"]`,
  );
  expect(found, `${subject} / ${key} の行があること`).not.toBeNull();
  return found as HTMLElement;
}

/** いまの値の行、全部（表の順のまま）。 */
function attributeRows(): HTMLElement[] {
  return [
    ...target.querySelectorAll('tbody[data-section="attributes"] tr'),
  ] as HTMLElement[];
}

/** 変更の記録の行、全部（表の順のまま）。 */
function revisionRows(): HTMLElement[] {
  return [
    ...target.querySelectorAll('tbody[data-section="attribute-revisions"] tr'),
  ] as HTMLElement[];
}

/** その行の値の欄。 */
function valueFieldOf(subject: string, key: string): HTMLInputElement {
  const found = attributeRow(subject, key).querySelector('input[name="value"]');
  expect(found, `${subject} / ${key} の値の欄があること`).not.toBeNull();
  return found as HTMLInputElement;
}

/** その行の値の欄に打つ。 */
function typeValue(subject: string, key: string, value: string): void {
  const field = valueFieldOf(subject, key);
  field.value = value;
  field.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

/** その行のボタンを押す。 */
function clickInRow(row: HTMLElement, label: string): void {
  const found = [...row.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `行に「${label}」があること`).toBeDefined();
  (found as HTMLButtonElement).click();
  flushSync();
}

/** 画面のボタン。 */
function buttonLabelled(label: string): HTMLButtonElement {
  const found = [...target.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `「${label}」ボタンがあること`).toBeDefined();
  return found as HTMLButtonElement;
}

function submit(): void {
  const form = target.querySelector("form");
  expect(form).not.toBeNull();
  (form as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  flushSync();
}

/** 要約の段落。 */
function summary(): HTMLElement {
  const found = target.querySelector("[data-replace-summary]");
  expect(found).not.toBeNull();
  return found as HTMLElement;
}

/** 件数の案内。 */
function revisionTotal(): HTMLElement {
  const found = target.querySelector("[data-revision-total]");
  expect(found).not.toBeNull();
  return found as HTMLElement;
}

/** 断りの段落（**告げる印を持つものだけ**）。 */
function alerts(): HTMLElement[] {
  return [...target.querySelectorAll('p[role="alert"]')] as HTMLElement[];
}

/**
 * **どの状態にも当てる判定** ——
 * **段落が `「」` で名指しした操作は、全部この画面に描かれている。**
 *
 * **無い操作を案内しない**（`B-15` / `D-6` / `D-7` / `D-8` /
 * `stats-panel.test.ts` の `assertNoticeNamesOnlyRealOperations` と同じ立場）。
 */
function assertNoticeNamesOnlyRealOperations(): void {
  const operations = [
    ...[...target.querySelectorAll("button")].map((button) =>
      (button.textContent ?? "").trim(),
    ),
    ...[...target.querySelectorAll("a")].map((anchor) =>
      (anchor.textContent ?? "").trim(),
    ),
  ];
  const named: string[] = [];
  for (const paragraph of target.querySelectorAll("p")) {
    for (const found of flatText(paragraph).matchAll(/「([^」]+)」/gu)) {
      named.push(found[1] as string);
    }
  }
  for (const operation of named) {
    expect(
      operations,
      `案内が名指しした「${operation}」が、この画面に実在すること`,
    ).toContain(operation);
  }
}

describe("画面の骨格", () => {
  it("見出しは、そのキャラクターの属性であると名乗る", async () => {
    const view = await show();
    const heading = target.querySelector("h1");
    expect(heading?.textContent).toContain(MIO);
    expect(heading?.textContent).not.toContain(NAR);
    // **一覧へ戻る導線と、読み直しがある**（案内が名指しできる操作）。
    expect(buttonLabelled("再読み込み")).toBeDefined();
    const back = [...target.querySelectorAll("a")].find(
      (anchor) => anchor.textContent?.trim() === "キャラクターの一覧へ",
    );
    expect(back).toBeDefined();
    await view.dispose();
  });

  it("2つの節を、そのキャラクターの識別子で引く", async () => {
    const view = await show();
    expect(urls).toContain(`GET /admin/characters/${MIO}/attributes`);
    expect(urls).toContain(`GET /admin/characters/${MIO}/attributes/revisions`);
    // **対照: 別の識別子は1度も引いていない。**
    expect(urls.join(" ")).not.toContain(NAR);
    // **記憶でも性格値でもない。**
    expect(urls.join(" ")).not.toContain("/memories");
    expect(urls.join(" ")).not.toContain("/stats");
    await view.dispose();
  });

  it("別のキャラクターを渡せば、そのキャラクターを引く", async () => {
    const view = await show(NAR);
    expect(urls).toContain(`GET /admin/characters/${NAR}/attributes`);
    expect(urls.join(" ")).not.toContain(MIO);
    await view.dispose();
  });
});

describe("いまの値", () => {
  it("主語とキーを、行ごとに出す", async () => {
    const view = await show();

    // **人が見るセルを、列ごと突き合わせる**（`cellsOf` の doc）。
    expect(cellsOf(attributeRow("character", "一人称"))).toEqual([
      "キャラクター",
      "一人称",
      `1 / ${ATTRIBUTE_LIMITS.value}`,
      "外す",
    ]);
    // **主語だけが違う2行**（`(subject, key)` の帰属）。
    expect(cellsOf(attributeRow("character", "年齢"))).toEqual([
      "キャラクター",
      "年齢",
      `2 / ${ATTRIBUTE_LIMITS.value}`,
      "外す",
    ]);
    expect(cellsOf(attributeRow("user", "年齢"))).toEqual([
      "利用者",
      "年齢",
      `2 / ${ATTRIBUTE_LIMITS.value}`,
      "外す",
    ]);

    // **値は欄の中にある**（読み手が直せるように）。
    expect(valueFieldOf("character", "年齢").value).toBe("19");
    expect(valueFieldOf("user", "年齢").value).toBe("32");
    expect(valueFieldOf("character", "一人称").value).toBe("僕");

    await view.dispose();
  });

  it("行の並びは、応答のまま", async () => {
    const view = await show();
    expect(
      attributeRows().map(
        (row) =>
          `${row.dataset["attributeSubject"]}/${row.dataset["attributeKey"]}`,
      ),
    ).toEqual(["character/一人称", "character/年齢", "user/年齢"]);
    await view.dispose();
  });

  /**
   * **保存済みの主語とキーは書き換えられない。**
   *
   * 改名も主語の付け替えも「消して足す」でしか表現できない
   * （`PUT` は丸ごと置き換えなので、綴りが変われば別の属性になる）。
   * **その2手を1つの欄に見せない** ——気付かずに属性を消すことになる。
   */
  it("保存済みの行に、主語とキーの入力欄が無い", async () => {
    const view = await show();
    const row = attributeRow("character", "年齢");
    expect(row.querySelector('input[name="key"]')).toBeNull();
    expect(row.querySelector("select")).toBeNull();
    await view.dispose();
  });

  it("足した行には、主語とキーの欄がある（主語は選ばれていない）", async () => {
    const view = await show();
    buttonLabelled("属性を追加").click();
    flushSync();

    const added = attributeRows().at(-1) as HTMLElement;
    const select = added.querySelector("select") as HTMLSelectElement;
    expect(select).not.toBeNull();
    // **推測で埋めない**（押し間違いで持ち主の入れ替わった属性を作らない）。
    expect(select.value).toBe("");
    expect(added.querySelector('input[name="key"]')).not.toBeNull();

    await view.dispose();
  });

  /** **欄ごとに、自分の上限を出す**（他の欄の上限を出さない / `D-2` の実測）。 */
  it("文字数の欄は、その欄の上限を名乗る", async () => {
    const view = await show();
    buttonLabelled("属性を追加").click();
    flushSync();

    const added = attributeRows().at(-1) as HTMLElement;
    const keyCounter = added.querySelector('[data-counter="key"]');
    const valueCounter = added.querySelector('[data-counter="value"]');
    expect(flatText(keyCounter as Element)).toBe(`0 / ${ATTRIBUTE_LIMITS.key}`);
    expect(flatText(valueCounter as Element)).toBe(
      `0 / ${ATTRIBUTE_LIMITS.value}`,
    );
    // **2つの上限は別の数**（同値なら取り違えが観測できない）。
    expect(ATTRIBUTE_LIMITS.key).not.toBe(ATTRIBUTE_LIMITS.value);

    await view.dispose();
  });

  /**
   * **3つの上限を、越える前に画面に出す。**
   *
   * 件数の上限（8）は**越えてからでは「9件目を打ち終えてから断られる」**ので、
   * **足す前に読めるところに出す。**
   */
  it("3つの上限を、越える前に名乗る", async () => {
    const view = await show();
    const notes = [...target.querySelectorAll("p")]
      .map((paragraph) => flatText(paragraph))
      .join(" ");
    expect(notes).toContain(String(ATTRIBUTE_LIMITS.key));
    expect(notes).toContain(String(ATTRIBUTE_LIMITS.value));
    expect(notes).toContain(String(MAX_ATTRIBUTES));
    // **3つは互いに別の数**（同値なら取り違えが観測できない）。
    expect(
      new Set([ATTRIBUTE_LIMITS.key, ATTRIBUTE_LIMITS.value, MAX_ATTRIBUTES]).size,
    ).toBe(3);
    await view.dispose();
  });

  it("上限を越えた欄に印が付く", async () => {
    const view = await show();
    const row = attributeRow("character", "年齢");
    expect(row.querySelector('[data-counter="value"]')?.classList.contains("over")).toBe(
      false,
    );

    typeValue("character", "年齢", "あ".repeat(ATTRIBUTE_LIMITS.value + 1));
    expect(
      attributeRow("character", "年齢")
        .querySelector('[data-counter="value"]')
        ?.classList.contains("over"),
    ).toBe(true);

    await view.dispose();
  });
});

describe("置き換えは、全行を送る", () => {
  /**
   * **真ん中の行だけ直しても、3行とも本文に在る。**
   *
   * 絞った瞬間、**絞られた属性は消える**（`PUT` は丸ごと置き換え）。
   */
  it("真ん中の行を直して送ると、3行とも本文に在る", async () => {
    const view = await show();
    typeValue("character", "年齢", "20");
    submit();

    await vi.waitFor(() => {
      expect(puts).toHaveLength(1);
    });
    expect(puts[0]?.url).toBe(`/admin/characters/${MIO}/attributes`);
    expect(puts[0]?.body.attributes).toEqual([
      { subject: "character", key: "一人称", value: "僕" },
      { subject: "character", key: "年齢", value: "20" },
      { subject: "user", key: "年齢", value: "32" },
    ]);

    await view.dispose();
  });

  it("外した行だけが載らない", async () => {
    const view = await show();
    clickInRow(attributeRow("character", "年齢"), "外す");
    submit();

    await vi.waitFor(() => {
      expect(puts).toHaveLength(1);
    });
    expect(
      puts[0]?.body.attributes.map((row) => `${row.subject}/${row.key}`),
    ).toEqual(["character/一人称", "user/年齢"]);

    await view.dispose();
  });

  it("押しただけでは、1バイトも送らない", async () => {
    const view = await show();
    clickInRow(attributeRow("character", "年齢"), "外す");
    expect(puts).toHaveLength(0);
    // **「戻す」で元に戻る。**
    clickInRow(attributeRow("character", "年齢"), "戻す");
    expect(puts).toHaveLength(0);
    await view.dispose();
  });

  it("通ったら、その旨を告げる", async () => {
    const view = await show();
    typeValue("character", "年齢", "20");
    submit();

    await vi.waitFor(() => {
      const notice = target.querySelector('p.notice[role="status"]');
      expect(notice).not.toBeNull();
      expect(flatText(notice as Element)).toContain("置き換えました");
    });

    await view.dispose();
  });
});

describe("保存すると何が記録されるか、押す前に言う", () => {
  it("値を直すと、変更が1件だと言う", async () => {
    const view = await show();
    typeValue("character", "年齢", "20");
    expect(flatText(summary())).toBe(
      "保存すると、この属性は 3 件になります。" +
        "変更の記録には 追加 0 件 / 変更 1 件 / 削除 0 件 が残ります。",
    );
    await view.dispose();
  });

  /**
   * **追加・変更・削除を同時に含む状態。**
   *
   * **⚠ 直す先は `user / 年齢` の `19`** ——
   * **`character / 年齢` の保存値と同じ綴り**なので、
   * **キーだけで突き合わせる実装では「変わっていない」と読まれて数が減る。**
   */
  it("追加・変更・削除を、それぞれ数えて言う", async () => {
    const view = await show();
    typeValue("user", "年齢", "19");
    clickInRow(attributeRow("character", "一人称"), "外す");
    buttonLabelled("属性を追加").click();
    flushSync();
    const added = attributeRows().at(-1) as HTMLElement;
    const select = added.querySelector("select") as HTMLSelectElement;
    select.value = "user";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    const keyField = added.querySelector('input[name="key"]') as HTMLInputElement;
    keyField.value = "住所";
    keyField.dispatchEvent(new Event("input", { bubbles: true }));
    const valueField = added.querySelector(
      'input[name="value"]',
    ) as HTMLInputElement;
    valueField.value = "京都";
    valueField.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();

    expect(flatText(summary())).toBe(
      "保存すると、この属性は 3 件になります。" +
        "変更の記録には 追加 1 件 / 変更 1 件 / 削除 1 件 が残ります。" +
        "消えるのは『キャラクターの一人称』です（置き換えなので、送らない行は消えます）。",
    );
    assertNoticeNamesOnlyRealOperations();

    await view.dispose();
  });

  it("何も直していなければ、記録は増えないと言う", async () => {
    const view = await show();
    expect(flatText(summary())).toContain("いまの内容と同じ");
    await view.dispose();
  });

  it("全部外すと、1件も残らないと言う", async () => {
    const view = await show();
    for (const row of attributeRows()) {
      clickInRow(row, "外す");
    }
    expect(flatText(summary())).toContain("1件も残りません");
    expect(flatText(summary())).toContain("削除 3 件");
    await view.dispose();
  });
});

describe("送れない下書きは、送らずに理由を出す", () => {
  /**
   * **⚠ 止めているのは主語の門である。**
   *
   * 足したばかりの行は**主語もキーも値も空**で、
   * `buildReplaceAttributes` は**主語 → キー → 値**の順に見るので、
   * **キーの門までは届かない**（この腕の名前は「キーが空」ではない）。
   * **キーの門を見る腕は下にある**（主語を選んでから、キーだけを空にする）。
   */
  it("主語の選ばれていない行があれば、断りを告げて送らない", async () => {
    const view = await show();
    buttonLabelled("属性を追加").click();
    flushSync();
    submit();

    expect(puts).toHaveLength(0);
    const shown = alerts().map((paragraph) => flatText(paragraph)).join(" ");
    expect(shown).toContain("主語");
    // **キーの門の文言ではない**（2つの門を、名前で見分ける）。
    expect(shown).not.toContain("キー");
    assertNoticeNamesOnlyRealOperations();

    await view.dispose();
  });

  /** **主語を選んで、キーだけを空にした行**（上の腕が届かない門）。 */
  it("キーの空いている行があれば、断りを告げて送らない", async () => {
    const view = await show();
    buttonLabelled("属性を追加").click();
    flushSync();
    const added = attributeRows().at(-1) as HTMLElement;
    const select = added.querySelector("select") as HTMLSelectElement;
    select.value = "user";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();
    submit();

    expect(puts).toHaveLength(0);
    const shown = alerts().map((paragraph) => flatText(paragraph)).join(" ");
    expect(shown).toContain("キー");
    // **主語の門は通っている**（選んだので）。
    expect(shown).not.toContain("主語");
    assertNoticeNamesOnlyRealOperations();

    await view.dispose();
  });
});

describe("置き換えが断られたときの読み分け", () => {
  /** `code` ごとに1本。**まとめると、どの `code` が落ちたか名前から分からない。** */
  const CASES: { code: string; reason: string; status: number }[] = [
    { code: DUPLICATE_ATTRIBUTE_CODE, reason: "duplicate", status: 400 },
    { code: KEY_CONTROL_CODE, reason: "control", status: 400 },
    { code: VALUE_CONTROL_CODE, reason: "control", status: 400 },
    { code: NOT_FOUND_CODE, reason: "not-found", status: 404 },
  ];

  it.each(CASES)(
    "`$code` は $reason として読める",
    async ({ code, reason, status }) => {
      putFailure = { status, code, message: `サーバの文言（${code}）` };
      const view = await show();
      typeValue("character", "年齢", "20");
      submit();

      await vi.waitFor(() => {
        expect(target.querySelector("[data-replace-reason]")).not.toBeNull();
      });
      const shown = target.querySelector("[data-replace-reason]") as HTMLElement;
      expect(shown.dataset["replaceReason"]).toBe(reason);
      // **告げる印は、その文言を持つ要素そのものに付いている。**
      expect(shown.getAttribute("role")).toBe("alert");
      expect(flatText(shown)).toContain(`サーバの文言（${code}）`);
      // **⚠ 失敗した回に、打った下書きが残っている**（読み直していない）。
      //
      // 断られた回にサーバの値から下書きを作り直すと、
      // **案内が指す行が画面から消える** ——`duplicate_attribute` の案内は
      // 「片方を「外す」か、キーを直してください」と言うのに、
      // **その2つ目の行はもう画面に無い**（実在しない導線を指す形 / `B-15`）。
      // **保存の失敗は「打ち直させる」ためのものなので、打ったものを消してはならない。**
      expect(valueFieldOf("character", "年齢").value).toBe("20");
      // **次の一手が出ている**（分類できた失敗なので）。
      expect(target.querySelector("[data-replace-guidance]")).not.toBeNull();
      assertNoticeNamesOnlyRealOperations();

      await view.dispose();
    },
  );

  it("知らない `code` は、文言だけを出して手を書かない", async () => {
    putFailure = {
      status: 400,
      code: "attribute_key_too_long",
      message: "属性のキーが長すぎます",
    };
    const view = await show();
    typeValue("character", "年齢", "20");
    submit();

    await vi.waitFor(() => {
      expect(target.querySelector("[data-replace-reason]")).not.toBeNull();
    });
    const shown = target.querySelector("[data-replace-reason]") as HTMLElement;
    expect(shown.dataset["replaceReason"]).toBe("unknown");
    expect(flatText(shown)).toContain("属性のキーが長すぎます");
    expect(target.querySelector("[data-replace-guidance]")).toBeNull();

    await view.dispose();
  });
});

describe("変更の記録", () => {
  /**
   * **行のセルを、配列で突き合わせる。**
   *
   * **`before` と `after` の入れ替え・主語の入れ替え・列の順**を、
   * これ1本で全部押さえる（`cellsOf` の doc の実測）。
   */
  it("日時・主語・キー・前・後を、その順で出す", async () => {
    const view = await show();
    const rows = revisionRows();
    expect(rows).toHaveLength(4);

    expect(cellsOf(rows[0] as HTMLElement)).toEqual([
      FIRST_REVISION_LABEL,
      "キャラクター",
      "一人称",
      "私",
      "僕",
    ]);
    // **追加**（`before` が `null`）。**主語は利用者。**
    expect(cellsOf(rows[1] as HTMLElement)).toEqual([
      SECOND_REVISION_LABEL,
      "利用者",
      "年齢",
      "（無かった）",
      "32",
    ]);
    // **2件目の利用者の行**（前後とも `null` でない / 主語の帰属をもう1本で押さえる）。
    expect(cellsOf(rows[2] as HTMLElement)).toEqual([
      THIRD_REVISION_LABEL,
      "利用者",
      "好物",
      "珈琲",
      "紅茶",
    ]);
    // **削除**（`after` が `null`）。
    expect(cellsOf(rows[3] as HTMLElement)).toEqual([
      FOURTH_REVISION_LABEL,
      "キャラクター",
      "種族",
      "猫獣人",
      "（消えた）",
    ]);

    await view.dispose();
  });

  it("新しい順のまま出す（並べ替えない）", async () => {
    const view = await show();
    expect(revisionRows().map((row) => row.dataset["revisionId"])).toEqual([
      "31",
      "30",
      "29",
      "28",
    ]);
    await view.dispose();
  });

  /** **生の綴りを `title` に残す**（ログや `sqlite3` の値と突き合わせる道）。 */
  it("日時の欄に、生の綴りが残っている", async () => {
    const view = await show();
    const titles = revisionRows().map((row) =>
      (row.querySelectorAll("td")[0] as HTMLTableCellElement).getAttribute(
        "title",
      ),
    );
    expect(titles).toEqual([
      "2026-08-27T09:00:00Z",
      "2026-08-26T08:00:00Z",
      "2025-01-02T03:04:05Z",
      "2024-05-04T03:02:01.100Z",
    ]);
    await view.dispose();
  });

  /**
   * **総数と、出した件数を両方言う**（`AttributeRevisionsView` の doc ——
   * 上限を名乗るだけでは「たまたまその数」と「切られた」を区別できない）。
   */
  it("上限で切られているとき、総数と出した件数の両方を出す", async () => {
    const view = await show();
    const text = flatText(revisionTotal());
    // **数は、それを名乗る語と一緒に探す**
    //（理由と実測は `attributes-edit.test.ts` の同じ節の doc）。
    expect(text).toContain("全 9 件");
    expect(text).toContain("新しい 4 件");
    expect(text).toContain("残り 5 件");
    // **上限は応答のもの**（サーバの既定 200 を写していない）。
    expect(text).not.toContain("200");
    await view.dispose();
  });

  it("1件も無ければ、そう言う", async () => {
    const view = await show(NAR);
    expect(flatText(revisionTotal())).toContain("まだ変更はありません");
    expect(revisionRows()).toHaveLength(0);
    await view.dispose();
  });
});

/**
 * **保存が通ったら、記録を読み直す。**
 *
 * 読み直さないと、**いま自分が起こした変更だけが表に出ない**という形になる
 * ——「静かに変わったことに気付く」ための表なのに、
 * **自分の変更が見えない**のではその目的を果たさない。
 *
 * # ⚠ **`vi.waitFor` は最初に成立した時点で返る**（CLAUDE.md / `D-4` の教訓）
 *
 * 「表に行がある」を待つと、**保存前から4行あるので即座に成立する。**
 * **待つのは「保存前の応答には無かった行が現れること」**である。
 */
describe("保存したら、記録を読み直す", () => {
  it("保存前には無かった行が現れる", async () => {
    const view = await show();
    // **保存前**: その id は表に無い。
    expect(revisionRows().map((row) => row.dataset["revisionId"])).not.toContain(
      String(SAVED_REVISION.id),
    );

    typeValue("character", "年齢", "20");
    submit();

    await vi.waitFor(() => {
      expect(
        revisionRows().map((row) => row.dataset["revisionId"]),
      ).toContain(String(SAVED_REVISION.id));
    });
    // **総数も増えている**（読み直したのは表だけではない）。**役に結ぶ。**
    expect(flatText(revisionTotal())).toContain(
      `全 ${MIO_REVISIONS.total_revisions + 1} 件`,
    );
    // **新しい行が先頭**（並びは応答のまま）。
    expect(cellsOf(revisionRows()[0] as HTMLElement)).toEqual([
      SAVED_REVISION_LABEL,
      "キャラクター",
      "年齢",
      "19",
      "20",
    ]);

    await view.dispose();
  });
});

describe("2つの読みは別々（片方が落ちても、もう片方は出す）", () => {
  it("いまの値が読めなくても、記録は出る（置き換えのフォームは出ない）", async () => {
    attributesFailure = { status: 500, code: null };
    const view = await show();

    const shown = alerts().map((paragraph) => flatText(paragraph)).join(" ");
    expect(shown).toContain("いまの値");
    // **記録は出ている。**
    expect(revisionRows()).toHaveLength(4);
    // **置き換えのフォームは無い**（下書きが作れないので）。
    expect(target.querySelector("form")).toBeNull();
    expect(target.querySelector("[data-replace-summary]")).toBeNull();
    // **無い操作を案内しない。**
    assertNoticeNamesOnlyRealOperations();

    await view.dispose();
  });

  it("記録が読めなくても、置き換えはできる", async () => {
    revisionsFailure = { status: 500, code: null };
    const view = open();

    await vi.waitFor(() => {
      expect(target.querySelector("[data-replace-summary]")).not.toBeNull();
    });
    const shown = alerts().map((paragraph) => flatText(paragraph)).join(" ");
    expect(shown).toContain("変更の記録");

    typeValue("character", "年齢", "20");
    submit();
    await vi.waitFor(() => {
      expect(puts).toHaveLength(1);
    });

    await view.dispose();
  });
});

describe("読み直し", () => {
  /**
   * **打ちかけを残さない**（`AttributesPanel.svelte` の `draft` の doc が
   * 「`load()` のたびに作り直す」と言っている ——**その主張の見張り番**）。
   *
   * 残すと、**別の窓で変わった値の上に古い写しを書き戻す。**
   * `PUT` は丸ごと置き換えなので、それは**その属性を古い値へ戻す**ことであり、
   * **属性は削れない枠に毎ターン載る**ので、以後の全ターンがその古い事実で喋る。
   *
   * # ⚠ **待つ前に、打った値が在ることを見る**（`D-4` の教訓）
   *
   * `vi.waitFor` は**最初に成立した時点で返る**ので、
   * 「サーバの値になること」だけを待つと、
   * **打てていなかった回**（欄が最初からサーバの値）でも緑になる。
   */
  it("「再読み込み」で、打ちかけを捨てて取り直す", async () => {
    const view = await show();
    // **サーバの値**（戻り先）。**画面から読む** ——写しを持たない。
    const storedAge = valueFieldOf("character", "年齢").value;
    expect(storedAge).toBe("19");

    typeValue("character", "年齢", "99");
    // **待つ前**: 打った値が在り、しかも戻り先と違う綴りである。
    expect(valueFieldOf("character", "年齢").value).toBe("99");
    expect(valueFieldOf("character", "年齢").value).not.toBe(storedAge);

    buttonLabelled("再読み込み").click();

    // **サーバの値に戻っている**（下書きを作り直した）。
    await vi.waitFor(() => {
      expect(valueFieldOf("character", "年齢").value).toBe(storedAge);
    });

    await view.dispose();
  });

  it("「再読み込み」で、2つとも取り直す", async () => {
    const view = await show();
    const before = urls.length;
    buttonLabelled("再読み込み").click();

    await vi.waitFor(() => {
      expect(urls.length).toBeGreaterThan(before);
    });
    await vi.waitFor(() => {
      expect(
        urls.slice(before).filter((url) => url.includes("/attributes")).length,
      ).toBe(2);
    });

    await view.dispose();
  });
});
