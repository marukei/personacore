/**
 * **告げる印が、どの部品にも付いているか**（作業単位 D-11b）。
 *
 * # ここで見るものと、見ないもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **エラーの段落が `role="alert"` を名乗ること**（全部品） | **ここ** |
 * | **通った報せの段落が `role="status"` を名乗ること**（全部品） | **ここ** |
 * | **「読み込み中」を出す綴りが `role="status"` を名乗ること**（全部品） | **ここ** |
 * | **その印が付いた要素が、実際に文言を持っていること** | `live-region.test.ts`（画面を動かす） |
 * | **流れてきた本文が、告げる要素の中に在ること** | `live-region.test.ts` |
 *
 * # ⚠ ここは、印が「付いていること」しか言えない
 *
 * **走査はソースの上の綴りを見る。** 印の付いた要素が
 * **中身の無い枠**であっても、ここは緑になる ——
 * **その主張の権威は `live-region.test.ts`**（実際に画面を動かして、
 * その要素が文言を持っていることを見る）。**2本で1つの主張である。**
 *
 * # ⚠ 守れていないこと
 *
 * * **`<span class="error">` は見ていない**（見るのは**段落**だけ）。
 * * **コメントの中も見る。** 字句解析はしない ——
 *   **日本語のコメントで走査が壊れた実績がある**（CLAUDE.md / `B-21d`）ので、
 *   **広く採る側に倒してある。** 「読み込み中」をコメントに書いたら、そこも咎める。
 * * **⚠ 「開始タグは1行に収まっている」という前提は、もう置いていない**
 *   （[`openTags`] の ⚠。**その前提は、この家のマークアップで既に破れていた**）。
 * * **「読み込み中」の宿主だけは、いまも「同じ行の中」で探す**（[`loadingWithoutStatus`]）——
 *   **「その行が出た」と「その行がこう言っている」は別の主張である**（CLAUDE.md）。
 *   **開始タグを割ったうえで文言を別の行へ置くと、宿主が見つからず「印が無い」として挙がる**
 *   （**緑にならない側へ倒れる**ので、見落としにはならない）。
 *
 * **ブラウザは起動しない**（jsdom すら要らない / 既定の `node` で走る）。
 */

import { describe, expect, it } from "vitest";

import { sourceFiles, type ScannedFile } from "./support/design-tokens";

/**
 * **走査 —— 画面の側の綴りを、全部数える。**
 *
 * 上の4本は `ChatView` と `App` しか動かさない。
 * **同じ印を他の部品でも付け忘れていないこと**は、ソースの上で数える。
 *
 * **⚠ 走査本体はソースを引数で受け取る**（CLAUDE.md / `15b`）ので、
 * **実ファイルと、テストが植えたソースを、同じ判定に通せる。**
 */

/** 開始タグ1件。 */
interface OpenTag {
  /** `<` から `>` まで、そのまま。 */
  readonly text: string;
  /** **1 始まりの行番号。** */
  readonly line: number;
}

/**
 * そのソースに在る開始タグ（出てきた順、重複もそのまま）。
 *
 * **⚠ これは HTML の構文解析ではない。** 残っている前提は
 * **「属性の値に `<` も `>` も出てこない」**ことだけである。
 * **崩れたらここを作り直すこと。**
 *
 * **⚠ かつてここには「開始タグは1行に収まっていて（この家のマークアップは
 * そうなっている）」と書いてあった。事実ではなかった** ——
 * **D-11b のレビューが実測: `MemoryList.svelte` の断りを3行に折り返して `role` を落とす変異が
 * 846 全緑を通った。** 床の `errors >= 15` にも掛からないので、**6本まで黙って消せた。**
 *
 * **⚠ 数は、レビューが挙げた6箇所より多い**（是正のときに数え直した）——
 * **`web/src` の `.svelte` に、複数行へ割った開始タグは 55 箇所ある**
 * （`tr` 8 / `button` 7 / `form` `input` `textarea` 各6 / `span` 5 / `td` 5 / `a` 3 / …）。
 * **行ごとに探す走査は 469 個しか見ておらず、ソース全体なら 524 個である。**
 * **レビューが挙げた「6箇所」は、そのうち `<p>` と `<span>`**
 * ——**この走査が判定する要素**——**だけを数えた数だった。**
 * **いま `p.error` の数は両方の走査で 21 で同じ**なので、
 * **是正だけを入れても実ファイルの結果は動かない**（変異を当てて初めて差が出る）。
 * **だから、行ごとではなくソース全体を走査する。**
 *
 * **⚠ 行番号は、頭からの改行を数えて出す**ので、LF でも CRLF でも同じ答えになる
 * （CLAUDE.md / `E-1`。**対照に CRLF の腕を対にしてある**）。
 */
function openTags(source: string): OpenTag[] {
  const found: OpenTag[] = [];
  for (const match of source.matchAll(/<[a-zA-Z][^<>]*>/g)) {
    const at = match.index ?? 0;
    found.push({ text: match[0], line: source.slice(0, at).split("\n").length });
  }
  return found;
}

/** そのタグが、その `class` をちょうど名乗っているか。 */
function hasClass(tag: string, name: string): boolean {
  const found = /\bclass="([^"]*)"/.exec(tag);
  return found === null
    ? false
    : (found[1] as string).split(/\s+/).includes(name);
}

/** そのタグが、その役割を名乗っているか。 */
function hasRole(tag: string, role: string): boolean {
  return tag.includes(`role="${role}"`);
}

/** その `class` を名乗る段落のうち、その役割を名乗っていないもの。 */
function paragraphsMissingRole(
  source: string,
  className: string,
  role: string,
): OpenTag[] {
  return openTags(source).filter(
    (tag) =>
      /^<p\b/.test(tag.text) &&
      hasClass(tag.text, className) &&
      !hasRole(tag.text, role),
  );
}

/** 「読み込み中」を名乗る綴り（画面の文言そのもの）。 */
const LOADING = "読み込み中";

/**
 * その行に「読み込み中」が在るのに、**その直前の開始タグが役割を名乗っていない**もの。
 *
 * **⚠ 見ているのは「同じ行の中」である**（CLAUDE.md ——
 * 「その行が出た」と「その行がこう言っている」は別の主張）。
 */
function loadingWithoutStatus(source: string): OpenTag[] {
  const found: OpenTag[] = [];
  source.split(/\r?\n/).forEach((line, at) => {
    const said = line.indexOf(LOADING);
    if (said < 0) {
      return;
    }
    const tags = [...line.matchAll(/<[a-zA-Z][^<>]*>/g)].filter(
      (match) => (match.index ?? 0) < said,
    );
    const host = tags[tags.length - 1];
    if (host === undefined || !hasRole(host[0], "status")) {
      found.push({ text: line.trim(), line: at + 1 });
    }
  });
  return found;
}

/** 画面の部品だけ（`.ts` は文言を持たない）。 */
function components(): ScannedFile[] {
  return sourceFiles().filter((entry) => entry.path.endsWith(".svelte"));
}

describe("走査: 告げる印が、どの部品にも付いている", () => {
  it("部品を、本当に読んでいる", () => {
    const paths = components().map((entry) => entry.path);
    expect(paths).toContain("App.svelte");
    expect(paths).toContain("lib/ChatView.svelte");
    expect(paths).toContain("lib/StatsPanel.svelte");
    expect(paths.length).toBeGreaterThanOrEqual(8);
  });

  it("エラーの段落は、全部 `role=\"alert\"` を名乗る", () => {
    const offences: string[] = [];
    for (const entry of components()) {
      for (const tag of paragraphsMissingRole(entry.source, "error", "alert")) {
        offences.push(`${entry.path}:${tag.line}: ${tag.text}`);
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });

  it("通った報せの段落は、全部 `role=\"status\"` を名乗る", () => {
    const offences: string[] = [];
    for (const entry of components()) {
      for (const tag of paragraphsMissingRole(
        entry.source,
        "notice",
        "status",
      )) {
        offences.push(`${entry.path}:${tag.line}: ${tag.text}`);
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });

  it("「読み込み中」を出す綴りは、全部 `role=\"status\"` を名乗る", () => {
    const offences: string[] = [];
    for (const entry of components()) {
      for (const tag of loadingWithoutStatus(entry.source)) {
        offences.push(`${entry.path}:${tag.line}: ${tag.text}`);
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });

  /** **数が痩せていないこと。** 0 件なら、上の3本は何も見ていない。 */
  it("印を付けた箇所が、実際に数えられている", () => {
    let errors = 0;
    let notices = 0;
    let loading = 0;
    for (const entry of components()) {
      for (const tag of openTags(entry.source)) {
        if (/^<p\b/.test(tag.text) && hasClass(tag.text, "error")) {
          errors += 1;
        }
        if (/^<p\b/.test(tag.text) && hasClass(tag.text, "notice")) {
          notices += 1;
        }
      }
      loading += entry.source
        .split(/\r?\n/)
        .filter((line) => line.includes(LOADING)).length;
    }
    expect(errors).toBeGreaterThanOrEqual(15);
    expect(notices).toBeGreaterThanOrEqual(4);
    expect(loading).toBeGreaterThanOrEqual(6);
  });
});

/**
 * **対照 —— 植えたソースを、走査本体に通す。**
 *
 * 実ファイルだけを読む対照は自己成就する（CLAUDE.md / `15b`）——
 * **変異は「その綴りを持つ行」を実ファイルへ書き込めば緑にできる。**
 * **植えたソースなら、その道が無い。**
 *
 * **⚠ CRLF の腕を対にしてある**（CLAUDE.md / `E-1`）——
 * **`openTags` は行を自分で割って番号を数えている。**
 */
describe("対照: 植えたソースで、走査の目が開いていることを見る", () => {
  const PLANTED = [
    "<p>ふつうの段落</p>",
    '<p class="error">印の無い断り</p>',
    '<p class="error" role="alert">印のある断り</p>',
    '<p class="notice">印の無い報せ</p>',
    '<p class="notice" role="status">印のある報せ</p>',
    '<span class="error">段落ではない断り</span>',
    "<p>読み込み中…</p>",
    '<p role="status">読み込み中…</p>',
    '<p class="errorish">似た綴りは別の語</p>',
  ];

  function check(newline: string): void {
    const source = PLANTED.join(newline);

    // **開始タグを、行つきで拾えている。**
    const tags = openTags(source);
    // **開始タグだけ**（`</p>` は数えない）——植えた行がちょうど9つ。
    expect(tags.length).toBe(9);
    expect(tags[0]).toEqual({ text: "<p>", line: 1 });
    expect(tags[5]).toEqual({ text: '<span class="error">', line: 6 });

    // **印の無い断りだけが挙がる**（段落でないものと、似た綴りは挙がらない）。
    expect(paragraphsMissingRole(source, "error", "alert")).toEqual([
      { text: '<p class="error">', line: 2 },
    ]);
    expect(paragraphsMissingRole(source, "notice", "status")).toEqual([
      { text: '<p class="notice">', line: 4 },
    ]);
    // **印の無い「読み込み中」だけが挙がる。**
    expect(loadingWithoutStatus(source)).toEqual([
      { text: "<p>読み込み中…</p>", line: 7 },
    ]);

    // **複数行に割った開始タグ**（この家に6箇所実在する形 / [`openTags`] の ⚠）。
    //
    // **⚠ 行ごとに探す走査は、これを1つも見ない** ——
    // **`role` を落とすために断りの段落を3行に折り返す変異が、846 全緑を通った**
    // （D-11b のレビューが実測。床の `errors >= 15` にも掛からず、**6本まで黙って消せた**）。
    const FOLDED_ERROR = `<p${newline}  class="error"${newline}>`;
    const FOLDED_NOTICE = `<p class="notice"${newline}  role="status"${newline}>`;
    const folded = [
      "<p",
      '  class="error"',
      ">印の無い断り</p>",
      '<p class="notice"',
      '  role="status"',
      ">印のある報せ</p>",
    ].join(newline);

    // **タグの綴りも、行番号も、そのまま押さえる**
    // （`contains` で「行き過ぎ」しか見ない形にしない / CLAUDE.md `E-1`）。
    expect(openTags(folded)).toEqual([
      { text: FOLDED_ERROR, line: 1 },
      { text: FOLDED_NOTICE, line: 4 },
    ]);
    // **折り返しても、印の無い断りは挙がる。**
    expect(paragraphsMissingRole(folded, "error", "alert")).toEqual([
      { text: FOLDED_ERROR, line: 1 },
    ]);
    // **折り返した先に印が在れば、挙がらない。**
    expect(paragraphsMissingRole(folded, "notice", "status")).toEqual([]);
  }

  it("LF で組んだソース", () => {
    check("\n");
  });

  it("CRLF で組んだソース", () => {
    check("\r\n");
  });

  /** **印が全部付いているソースからは、何も出ない。** */
  it("全部に印が付いていれば、何も挙がらない", () => {
    const source = [
      '<p class="error" role="alert">断り</p>',
      '<p class="notice" role="status">報せ</p>',
      '<span class="muted" role="status">読み込み中…</span>',
    ].join("\n");
    expect(paragraphsMissingRole(source, "error", "alert")).toEqual([]);
    expect(paragraphsMissingRole(source, "notice", "status")).toEqual([]);
    expect(loadingWithoutStatus(source)).toEqual([]);
  });

  /**
   * **⚠ 印が「同じ行の別のタグ」に付いていても、通さない。**
   *
   * CLAUDE.md —— **「その行が出た」と「その行がこう言っている」は別の主張である。**
   */
  it("同じ行の別のタグに付いた印は、数えない", () => {
    const source = '<span role="status"></span><p class="error">断り</p>';
    expect(paragraphsMissingRole(source, "error", "alert")).toEqual([
      { text: '<p class="error">', line: 1 },
    ]);
    const loading = '<span role="status"></span><span class="muted">読み込み中…</span>';
    expect(loadingWithoutStatus(loading).length).toBe(1);
  });

  /** **`class` は語で見る**（`error` と `errors` は別の語）。 */
  it("`class` の綴りは、語として見る", () => {
    expect(hasClass('<p class="error">', "error")).toBe(true);
    expect(hasClass('<p class="body error">', "error")).toBe(true);
    expect(hasClass('<p class="errorish">', "error")).toBe(false);
    expect(hasClass("<p>", "error")).toBe(false);
  });
});
