/**
 * **画面が、利用者に ADR の番号を見せていない**（作業単位 F-8）。
 *
 * # なぜ要るのか
 *
 * **画面が「リポジトリを開いている人」に話しかけていた** ——
 * 「作成のときだけ決められます（後から変えられません / **ADR-0016 §1**）」のように、
 * **出典が本文と同じ括弧の中に描かれていた。**
 * 見つけたのは利用者で、**司令塔はそのうち3つを画面で見ていながら素通ししている**
 * （2026-08-28 / `docs/06-roadmap.md` の F 節）。
 *
 * **文は消さない。** 「識別子は後から変えられない」「記憶は消えない」
 * 「変容は本文に出ない」は**利用者が知る必要のある事実**である。
 * **落とすのは出典だけ**で、**理由はコード側のコメントに残す。**
 *
 * # ここで見るものと、見ないもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **ソースの上で、描かれる部分に出典が無いこと** | **ここ** |
 * | **その文が、実際に画面へ描かれること** | `character-form` / `character-repair-form` / `memory-list` / `stats-panel` |
 *
 * **片方だけでは足りない。** ソースの走査は「消した」ことしか言わず、
 * **文ごと消しても緑になる。**
 *
 * # ⚠ 対照は、植えたソースを走査本体に通す
 *
 * 実ファイルだけを読む対照は自己成就する（CLAUDE.md / `15b` の実測）。
 * **走査本体は引数でソースを受け取る形**（`tests/support/screen-wording.ts`）なので、
 * **実ファイルと植えたソースを、同じ判定に通している。**
 *
 * **ブラウザは起動しない**（この1本は jsdom すら要らない / 既定の `node` で走る）。
 */

import { describe, expect, it } from "vitest";

import {
  adrCitations,
  renderedMarkup,
  screenSources,
  scriptSources,
  stringLiterals,
} from "./support/screen-wording";

/**
 * **出典が描かれていた4つの画面**（`docs/06-roadmap.md` の F-8 の内訳）。
 *
 * **一覧に載っていること自体を見る** —— 走査が
 * **この4つを1つも読んでいない状態**を、緑で通さないため。
 */
const GUILTY = [
  "lib/CharacterForm.svelte",
  "lib/CharacterRepairForm.svelte",
  "lib/MemoryList.svelte",
  "lib/StatsPanel.svelte",
];

/** 走査が読んだもの（**1回だけ読む**）。 */
const SOURCES = screenSources();

describe("走査が、実ファイルを本当に読んでいる", () => {
  it("`src/` の `.svelte` と `.ts` を、まとめて読んでいる", () => {
    const paths = SOURCES.map((entry) => entry.path);

    for (const path of GUILTY) {
      expect(paths, `${path} を読んでいること`).toContain(path);
    }
    // **配線の入口も読んでいる。**
    //
    // **⚠ 「`App.svelte` はどのテストからも import されていない」は、
    // もう事実ではない**（作業単位 F-7b のレビューが数え直した / 2026-08-29）。
    // **`D-3` の時点ではそう実測され**（配線を潰す変異が 107 全緑だった）、
    // **その後 4 本が mount するようになって塞がった**
    // （`app-routing` / `app-routing-characters` / `app-routing-stats` /
    // `stat-scale-path`）。**走査がここを読む理由は変わらない**ので、
    // 一覧に入っていることだけを見る。
    expect(paths).toContain("App.svelte");
    // **`.ts` も読んでいる。**
    expect(paths).toContain("lib/statsEdit.ts");
    // **一覧が痩せていない。** 空の一覧なら、この下の判定は全部素通りする。
    expect(paths.length).toBeGreaterThanOrEqual(18);
    // **中身が空でない。**
    for (const entry of SOURCES) {
      expect(entry.source.length, `${entry.path} の中身`).toBeGreaterThan(0);
    }
  });

  /**
   * **綴りが実ファイルの上で現に見つかる。**
   *
   * ここで見ているのは**生のソース**（コメントを落とす前）。
   * **落とした出典はコード側のコメントに残っている**ので、
   * この4つには必ず `ADR-` が残る —— **残っていなければ、
   * 走査が読んでいないか、綴りが壊れているかのどちらか。**
   */
  /**
   * **`<script>` の取り出しが、実ファイルの上で空になっていない。**
   *
   * 空を返す実装だと、上の「`<script>` の中の文字列リテラル」は
   * **見るものが1つも無いまま永久に緑**になる（`15b` の「そもそも探せていない」形）。
   *
   * **判定に使う綴りはこの検査の中に直書き**してある ——
   * 実ファイルから作ると、取り出しが壊れても期待値が一緒に動く。
   */
  it("`<script>` の中身を、実ファイルから取り出せている", () => {
    const panel = SOURCES.find(
      (found) => found.path === "lib/StatsPanel.svelte",
    );
    expect(panel).toBeDefined();
    const scripts = scriptSources((panel as { source: string }).source);
    expect(scripts.length, "StatsPanel の `<script>` が1つ").toBe(1);

    const literals = stringLiterals(scripts[0] as string);
    // **画面に描かれる文字列が、現に取り出せている。**
    expect(literals).toContain("性格値を置き換えました。");
    // **`<script>` の外は入っていない**（描かれる本文まで拾っていない）。
    expect(literals).not.toContain("いまの値");
  });

  it("4つの画面には、コメントの中に出典が残っている", () => {
    for (const path of GUILTY) {
      const entry = SOURCES.find((found) => found.path === path);
      expect(entry, `${path}`).toBeDefined();
      expect(
        adrCitations((entry as { source: string }).source).length,
        `${path} のコメントに出典が残っていること`,
      ).toBeGreaterThan(0);
    }
  });
});

describe("描かれる文言に、ADR の出典が1つも無い", () => {
  it("`.svelte` の描画される部分", () => {
    const offences: string[] = [];
    for (const entry of SOURCES) {
      if (!entry.path.endsWith(".svelte")) {
        continue;
      }
      for (const citation of adrCitations(renderedMarkup(entry.source))) {
        offences.push(`${entry.path}: ${citation}`);
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });

  /**
   * **`.svelte` の `<script>` の中の文字列リテラル**（レビューの実測 / 2026-08-29）。
   *
   * **`<script>` は「描かれない」ではない。**
   * `StatsPanel.svelte` の `notice = "性格値を置き換えました。"` は
   * `<p class="notice">{notice}</p>` で**実際に描かれる。**
   *
   * **かつてここは両方の網の外だった** ——
   * `renderedMarkup` が `<script>` を丸ごと落とし、
   * `stringLiterals` は `.ts` にしか掛かっていなかったので、
   * **その文字列に出典を足す変異が 631 全緑を通った。**
   */
  it("`.svelte` の `<script>` の中の文字列リテラル", () => {
    const offences: string[] = [];
    for (const entry of SOURCES) {
      if (!entry.path.endsWith(".svelte")) {
        continue;
      }
      for (const script of scriptSources(entry.source)) {
        for (const literal of stringLiterals(script)) {
          for (const citation of adrCitations(literal)) {
            offences.push(`${entry.path}: ${citation}`);
          }
        }
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });

  it("`.ts` の文字列リテラル", () => {
    const offences: string[] = [];
    for (const entry of SOURCES) {
      if (!entry.path.endsWith(".ts")) {
        continue;
      }
      for (const literal of stringLiterals(entry.source)) {
        for (const citation of adrCitations(literal)) {
          offences.push(`${entry.path}: ${citation}`);
        }
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });
});

/**
 * **対照 —— 植えたソースを、実ファイルと同じ判定に通す。**
 *
 * `15b` の実測: 走査本体の綴りだけを壊す変異が**緑**だった。
 * **変異は別の綴りを走査対象のファイルそのものへ書き込む**ので、
 * **実ファイルを読む対照は、それを1件見つけて緑になる。**
 * **植えたソースなら、その道が無い。**
 */
describe("対照: 植えたソースで、走査の目が開いていることを見る", () => {
  it("描画される場所の出典は、見つかる", () => {
    const planted = [
      "<p>",
      "  作成のときだけ決められます（後から変えられません / ADR-0016 §1）。",
      "</p>",
    ].join("\n");
    expect(adrCitations(renderedMarkup(planted))).toEqual(["ADR-0016"]);
  });

  /**
   * **⚠ この検査の名前と意味を変えた**（レビューの実測 / 2026-08-29）。
   *
   * 以前の名前は「**`<script>` の中の出典は、見つからない**」で、
   * **「script は描かれない」という成り立っていない前提を、正しい振る舞いとして
   * 固定していた**（`notice = "…"` は現に画面へ出る）。
   *
   * **アサートは1つも緩めていない。** `renderedMarkup` が `<script>` を落とすのは
   * いまも正しい振る舞いで、**変わったのは「だから見なくてよい」という結論のほう。**
   * **落ちた中身は、すぐ下の検査が [`scriptSources`] で拾う。**
   */
  it("`renderedMarkup` は `<script>` を落とす（拾うのは `scriptSources` の側）", () => {
    const planted = [
      "<script lang=\"ts\">",
      "  /** 識別子は不変（ADR-0016 §1）。 */",
      "  const handle = 'aoi';",
      "</script>",
      "<p>識別子は変えられません。</p>",
    ].join("\n");
    expect(adrCitations(renderedMarkup(planted))).toEqual([]);
    // **対照の対照**: 落としたのは `<script>` だけで、本文は残っている。
    expect(renderedMarkup(planted)).toContain("識別子は変えられません。");
  });

  /**
   * **`<script>` の中の、描かれる文字列は見つかる。**
   *
   * 上の検査と**同じ植えたソース**を使う ——
   * **落とす側と拾う側が、同じ切れ目の裏表であること**を1本で見る。
   */
  it("`<script>` の中の文字列リテラルの出典は、見つかる", () => {
    const planted = [
      "<script lang=\"ts\">",
      "  /** 識別子は不変（ADR-0016 §1）。 */",
      "  let notice = \"識別子は変えられません（ADR-0022）。\";",
      "</script>",
      "<p>{notice}</p>",
    ].join("\n");

    // **描かれる側の網には、掛からない。**
    expect(adrCitations(renderedMarkup(planted))).toEqual([]);
    // **`<script>` の側の網に掛かる。** コメントの出典（ADR-0016）は拾わない。
    const literals = scriptSources(planted).flatMap((script) =>
      stringLiterals(script),
    );
    expect(adrCitations(literals.join(" "))).toEqual(["ADR-0022"]);
  });

  it("対照: `<script>` が無いソースからは、何も取り出さない", () => {
    expect(scriptSources("<p>識別子は変えられません。</p>")).toEqual([]);
  });

  /**
   * **属性値の中の `<!--` から、コメントを始めない。**
   *
   * 始めると、**その後ろの `-->` までにある描かれる文言が走査から消える。**
   * **作為的な形**だが、**倒す向きは「残す」側**なので、
   * 誤って残しすぎても黙って緩むことはない。
   */
  it("属性値の中の `<!--` で、描かれる文言を飲み込まない", () => {
    const planted = '<p title="a <!-- b">識別子は変えられません（ADR-0016 §1）。</p>';
    expect(adrCitations(renderedMarkup(planted))).toEqual(["ADR-0016"]);
    // **対照: ふつうのコメントは、これまでどおり落ちる。**
    expect(
      adrCitations(renderedMarkup("<!-- ADR-0016 --><p>本文</p>")),
    ).toEqual([]);
  });

  it("`<style>` の中の出典は、見つからない", () => {
    const planted = [
      "<p>記憶は消えません。</p>",
      "<style>",
      "  /* 色は ADR-0022 の説明とは関係が無い */",
      "  .hint { color: #666; }",
      "</style>",
    ].join("\n");
    expect(adrCitations(renderedMarkup(planted))).toEqual([]);
    expect(renderedMarkup(planted)).toContain("記憶は消えません。");
  });

  it("`<!-- -->` の中の出典は、見つからない", () => {
    const planted = [
      "<!-- **消せる道は無い**（ADR-0022 §1）。 -->",
      "<p>記憶は消えません。</p>",
    ].join("\n");
    expect(adrCitations(renderedMarkup(planted))).toEqual([]);
    expect(renderedMarkup(planted)).toContain("記憶は消えません。");
  });

  /**
   * **同じファイルの中で、コメントの外と中が別々に扱われる。**
   *
   * 片方だけを植えると、「全部落としている」実装と
   * 「全部残している」実装のどちらかが通る。
   */
  it("同じソースの中で、コメントの外だけが残る", () => {
    const planted = [
      "<!-- 出典はここに残す（ADR-0024 §9）。 -->",
      "<p>変容は本文に出ません（ADR-0024 §9）。</p>",
    ].join("\n");
    expect(adrCitations(renderedMarkup(planted))).toEqual(["ADR-0024"]);
  });

  it("`.ts` の文字列リテラルの出典は、見つかる", () => {
    const planted = 'export const HINT = "識別子は変えられません（ADR-0016 §1）。";';
    expect(stringLiterals(planted)).toEqual([
      "識別子は変えられません（ADR-0016 §1）。",
    ]);
    expect(adrCitations(stringLiterals(planted).join(" "))).toEqual([
      "ADR-0016",
    ]);
  });

  it("`.ts` のコメントの出典は、見つからない", () => {
    const planted = [
      "/**",
      " * 識別子は不変（ADR-0016 §1）。",
      " */",
      "// 記憶は消えない（ADR-0022）。",
      'export const HINT = "識別子は変えられません。";',
    ].join("\n");
    expect(stringLiterals(planted)).toEqual(["識別子は変えられません。"]);
  });

  /**
   * **正規表現リテラルの中の `//` を、行コメントと読まない。**
   *
   * 読むと**行の残りを捨てる**ので、`router.ts` の
   * `.replace(/^\//, "")` の後ろに置かれた文字列が走査から消える。
   */
  it("正規表現リテラルの中の `//` で、行の残りを捨てない", () => {
    const planted =
      'const path = hash.replace(/^\\//, "").replace(/\\/+$/, "見えるか（ADR-0016）");';
    expect(adrCitations(stringLiterals(planted).join(" "))).toEqual([
      "ADR-0016",
    ]);
  });

  it("テンプレートリテラルの中の出典も、見つかる", () => {
    const planted = "const hint = `識別子（${row.handle}）は変えられません（ADR-0016 §1）。`;";
    expect(adrCitations(stringLiterals(planted).join(" "))).toEqual([
      "ADR-0016",
    ]);
  });

  it("対照: 出典が1つも無いソースからは、何も出ない", () => {
    expect(adrCitations(renderedMarkup("<p>識別子は変えられません。</p>"))).toEqual(
      [],
    );
    expect(
      adrCitations(stringLiterals('const a = "識別子は変えられません。";').join(" ")),
    ).toEqual([]);
  });
});
