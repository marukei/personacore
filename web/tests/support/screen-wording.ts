/**
 * **画面に描かれる文言の走査**（作業単位 F-8）。
 *
 * # 何を見るのか
 *
 * **利用者に描かれる文言に、ADR の出典（`ADR-0016` のような番号）が出ていないこと。**
 * 文そのものは残す —— 「識別子は後から変えられない」「記憶は消えない」
 * 「変容は本文に出ない」は**利用者が知る必要のある事実**である。
 * **落とすのは出典だけ。**
 *
 * # ⚠ コメントは描かれない。だから走査の対象から外す
 *
 * `<!-- -->` / `//` / `/* *\/` の中は画面に出ない。
 * **むしろ、落とした出典はコード側のコメントへ残す**ので、
 * ここまで禁じると「理由を近くに書く」ができなくなる。
 *
 * # ⚠ **`<script>` は「描かれない」ではない**（レビューの実測 / 2026-08-29）
 *
 * **`<script>` の中の文字列リテラルは、画面に描かれる。**
 * `StatsPanel.svelte` の `notice = "性格値を置き換えました。"` は
 * `<p class="notice">{notice}</p>` で出るし、
 * `ChatView.svelte` の `"このセッションの続き"` も同じ。
 *
 * **かつてここは `<script>` を丸ごと落とすだけで、`stringLiterals` は `.ts` にしか
 * 掛けていなかった** —— **その文字列に出典を足す変異が 631 全緑を通った。**
 * **しかも対照テストの名前が「`<script>` の中の出典は、見つからない」で、
 * 成り立っていない前提を正しい振る舞いとして固定していた**
 * （**「守っていない場所に『守った』と書く」形そのもの**）。
 *
 * **→ [`scriptSources`] を足し、`<script>` の中身を [`stringLiterals`] に通す。**
 * **[`renderedMarkup`] と [`scriptSources`] は同じ切れ目の裏表**で、
 * 片方が落としたものをもう片方が拾う。
 *
 * # ⚠ 走査本体は、ソースを**引数で**受け取る
 *
 * 実ファイルだけを読む走査は、**対照テストが自己成就する**
 * （CLAUDE.md / `15b` の実測 —— 走査本体の綴りを壊す変異が、
 * 変異そのものを走査対象のファイルへ書き込むので、
 * 「1件見つかった」という対照が緑になる）。
 *
 * **ここでは走査本体を引数の形にしてあるので、
 * 実ファイルと、テストが植えたソースを、同じ判定に通せる。**
 *
 * # `.ts` も見る理由
 *
 * **画面に出る文言の一部は `.ts` の文字列リテラルから来る**
 * （`chat.ts` の `outcomeNotice` / `statsEdit.ts` の `runModeNotice` …）。
 * `.svelte` だけを見ると、**文言を関数へ移した日に守りが消える。**
 */

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * ADR の出典の綴り。
 *
 * **節番号（`§1`）までは見ない。** 番号が出ている時点で違反なので、
 * 拾うのは `ADR-0016` の形だけでよい。
 */
const ADR_CITATION = /ADR-\d+/g;

/** そのテキストに含まれる ADR の出典（**重複もそのまま**）。 */
export function adrCitations(text: string): string[] {
  return [...text.matchAll(ADR_CITATION)].map((found) => found[0]);
}

/** `<script …>` … `</script>` の1件（中身を取り出すのに使う）。 */
const SCRIPT_BLOCK = /<script\b[^>]*>([\s\S]*?)<\/script>/gi;

/**
 * `.svelte` から、**利用者に描かれる部分だけ**を残す。
 *
 * `<script>` / `<style>` / `<!-- -->` を落とす。
 * **順番は script → style → コメント**（この順で植えたソースを対照に通してある）。
 *
 * **⚠ ここが落とした `<script>` の中身は、[`scriptSources`] が拾う。**
 * **落としっぱなしにすると、そこに置いた画面の文言が両方の網の外へ出る。**
 */
export function renderedMarkup(source: string): string {
  const withoutCode = source
    .replace(/<script\b[\s\S]*?<\/script>/gi, "")
    .replace(/<style\b[\s\S]*?<\/style>/gi, "");
  return stripHtmlComments(withoutCode);
}

/**
 * `<!-- -->` を落とす。**ただし属性値の中から始まる `<!--` は、コメントではない。**
 *
 * # なぜ手で走らせるのか
 *
 * `.replace(/<!--[\s\S]*?-->/g, "")` は、
 * `<p title="a <!-- b">出典 ADR-0016<!-- x --></p>` のような並びで
 * **属性値の中の `<!--` から、後ろの `-->` までを丸ごと飲む** ——
 * **その間にある描かれる文言が、走査から静かに消える。**
 *
 * **作為的な形ではある**（司令塔の判断）**が、直す向きは片側だけである** ——
 * ここで残す側に倒すと、**判定は厳しくなることしかない。**
 * 誤って残しすぎたら、その文言に出典があるときだけ赤くなる（＝黙って緩まない）。
 */
function stripHtmlComments(source: string): string {
  let out = "";
  let at = 0;
  for (;;) {
    const open = source.indexOf("<!--", at);
    if (open < 0) {
      out += source.slice(at);
      return out;
    }
    if (insideAttribute(source, open)) {
      // **属性値の中。** コメントではないので、そのまま残して先へ進む。
      out += source.slice(at, open + 4);
      at = open + 4;
      continue;
    }
    const close = source.indexOf("-->", open + 4);
    if (close < 0) {
      // **閉じていない。** 落とす範囲が決まらないので、残す側に倒す。
      out += source.slice(at);
      return out;
    }
    out += source.slice(at, open);
    at = close + 3;
  }
}

/** その位置が、開いたままの属性値の中か（直前の `<` からの引用符の数で見る）。 */
function insideAttribute(source: string, at: number): boolean {
  const tagStart = source.lastIndexOf("<", at - 1);
  if (tagStart < 0) {
    return false;
  }
  let doubles = 0;
  let singles = 0;
  for (let i = tagStart; i < at; i += 1) {
    if (source[i] === '"') {
      doubles += 1;
    } else if (source[i] === "'") {
      singles += 1;
    }
  }
  return doubles % 2 === 1 || singles % 2 === 1;
}

/**
 * `.svelte` の **`<script>` の中身**を、出てきた順に返す（[`renderedMarkup`] の裏側）。
 *
 * **ここに置いた文字列リテラルは、画面に描かれる**（モジュール doc の ⚠）。
 * 取り出したものは [`stringLiterals`] に通すこと。
 */
export function scriptSources(source: string): string[] {
  return [...source.matchAll(SCRIPT_BLOCK)].map((found) => found[1] ?? "");
}

/**
 * `.ts` の**文字列リテラルの中身**を、出てきた順に返す。
 *
 * コメント（`//` と `/* *\/`）と正規表現リテラルは読み飛ばす。
 * **テンプレートリテラルは `${}` の中も含めてそのまま1件**にする
 * —— 広く採る側に倒しておく（見落とすより誤検出のほうが安全）。
 */
export function stringLiterals(source: string): string[] {
  const found: string[] = [];
  let at = 0;
  /** 直前の意味のある文字（**正規表現リテラルの判別に使う**）。 */
  let previous = "";

  while (at < source.length) {
    const here = source[at] as string;
    const next = source[at + 1] ?? "";

    if (here === "/" && next === "/") {
      const end = source.indexOf("\n", at);
      at = end < 0 ? source.length : end;
      continue;
    }
    if (here === "/" && next === "*") {
      const end = source.indexOf("*/", at + 2);
      at = end < 0 ? source.length : end + 2;
      continue;
    }
    if (here === '"' || here === "'" || here === "`") {
      const quoted = readQuoted(source, at, here);
      found.push(quoted.text);
      at = quoted.after;
      previous = here;
      continue;
    }
    if (here === "/" && startsRegex(previous)) {
      at = skipRegex(source, at);
      previous = "/";
      continue;
    }
    if (here.trim() !== "") {
      previous = here;
    }
    at += 1;
  }

  return found;
}

/** 引用符で始まる並びを読む。**返すのは中身と、閉じた次の位置。** */
function readQuoted(
  source: string,
  start: number,
  quote: string,
): { text: string; after: number } {
  let at = start + 1;
  let text = "";
  while (at < source.length) {
    const here = source[at] as string;
    if (here === "\\") {
      text += source.slice(at, at + 2);
      at += 2;
      continue;
    }
    if (here === quote) {
      return { text, after: at + 1 };
    }
    if (here === "\n" && quote !== "`") {
      // **閉じていない。** 行をまたぐ文字列はテンプレートだけ。
      return { text, after: at + 1 };
    }
    text += here;
    at += 1;
  }
  return { text, after: source.length };
}

/**
 * その位置の `/` が正規表現リテラルの始まりか。
 *
 * **直前の意味のある文字だけで決める**（JavaScript の一般的な判別法）。
 * 外すと、`/^\//` のような並びの中の `//` を行コメントと読んで**行の残りを捨てる。**
 */
function startsRegex(previous: string): boolean {
  return previous === "" || "(,=:[!&|?{};+-*%<>~^".includes(previous);
}

/** 正規表現リテラルを読み飛ばして、**次の位置**を返す。 */
function skipRegex(source: string, start: number): number {
  let at = start + 1;
  let inClass = false;
  while (at < source.length) {
    const here = source[at] as string;
    if (here === "\\") {
      at += 2;
      continue;
    }
    if (here === "[") {
      inClass = true;
    } else if (here === "]") {
      inClass = false;
    } else if (here === "/" && !inClass) {
      return at + 1;
    } else if (here === "\n") {
      // **正規表現ではなかった。** 1文字だけ進めて読み直させる。
      return start + 1;
    }
    at += 1;
  }
  return source.length;
}

/** 走査した1件。 */
export interface ScreenSource {
  /** `src/` からの相対パス（区切りは `/`）。 */
  readonly path: string;
  /** ファイルの中身、**そのまま**。 */
  readonly source: string;
}

/** 走査の根（`web/src`）。 */
const SRC = fileURLToPath(new URL("../../src", import.meta.url));

/**
 * `web/src` の下の `.svelte` と `.ts` を全部読む。
 *
 * **一覧を作る側と判定する側を分けてある** ——
 * 判定（[`adrCitations`] / [`renderedMarkup`] / [`stringLiterals`]）は
 * 引数でソースを受け取るので、**植えたソースを同じ判定に通せる。**
 */
export function screenSources(): ScreenSource[] {
  const found: ScreenSource[] = [];
  const walk = (directory: string, prefix: string): void => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const full = join(directory, entry.name);
      const relative = prefix === "" ? entry.name : `${prefix}/${entry.name}`;
      if (entry.isDirectory()) {
        walk(full, relative);
        continue;
      }
      if (entry.name.endsWith(".svelte") || entry.name.endsWith(".ts")) {
        found.push({ path: relative, source: readFileSync(full, "utf8") });
      }
    }
  };
  walk(SRC, "");
  return found;
}
