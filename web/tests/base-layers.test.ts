/**
 * **土台の層と、共通の部品の見た目**（作業単位 D-12b-1 / ADR-0040 の追記）。
 *
 * # ここで見るものと、見ないもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **層の順番の宣言が、主題・土台・部品・実用の順であること** | **ここ** |
 * | **`app.css` の規則で層の外に在るのは、主題と暗い側の差し替えだけであること** | **ここ** |
 * | **土台の層は要素の既定、部品の層はクラス**（区別の仕方） | **ここ** |
 * | **ボタンと入力欄の見た目が、要素の既定に置いてあること**（枠・地・字・角丸の値まで） | **ここ** |
 * | **主ボタンとカードが、部品の層のクラスであること**（値まで） | **ここ** |
 * | **チェックボックスを包む `label` が、箱と文言を横に並べること**（規則と、マークアップがその形であること） | **ここ** |
 * | **フォーカスの輪を消す宣言が、`app.css` にも部品の `<style>` にも無いこと** | **ここ** |
 * | **ボタンと入力欄の押せる高さ** | **ここ** |
 * | **字と地を両方持つ規則の明度差が、明るい側でも暗い側でも開いていること** | **ここ**（粗い下限 / 下の ⚠） |
 * | **色を混ぜる関数を使わないこと**（ビルドが生の 16 進の代替を書き足すから） | **ここ** |
 * | **帯（`App.svelte`）の見た目と、いまいる画面の印** | `tests/app-nav.test.ts` |
 * | **色の綴りが1箇所 / 意味の帳簿 / 字の割り当て** | `tests/design-tokens.test.ts` |
 * | **配られた CSS で、本当にユーティリティが勝つか** | **どこにも無い**（下の ⚠） |
 *
 * # なぜ層に入れたのか
 *
 * **`@layer` の外の通常宣言は、どの層の通常宣言よりも強い**（CSS Cascading L5）。
 * **ユーティリティは `utilities` の層に在る**ので、土台を層の外に置いたままだと、
 * **`a { color: var(--color-read) }` を持つ帯のリンクに `text-ink-2` を書いても効かない。**
 * **D-12b-1 の帯がまさにその形**（リンクに `text-ink-2` を当てる）なので、ここで層に入れた。
 * 理由の権威は ADR-0040 の追記。
 *
 * # ⚠ 守れていないこと
 *
 * * **ここは Tailwind を1度も走らせない。** 見ているのはソースの上の綴りで、
 *   **配られた CSS の中で `@layer base{…}` がユーティリティより前に並ぶか**は言えない。
 *   **D-12b-1 では `pnpm build` の出力を読んで確かめた**（見張り番としては置いていない）。
 * * **部品の `<style>` は層の外に在る**（Svelte が層を付けない）。
 *   **部品の側の規則は、土台にもユーティリティにも勝つ。** ここはそれを見ていない。
 * * **明度差の下限は WCAG のコントラスト比ではない。** oklch の L の差で、
 *   **「明るい側では読めるのに、暗い側で字と地が寄った」形を止める粗い網**である。
 *   **読めるかどうかは人が見るしかない**（`tests/colour-scheme.test.ts` と同じ立場）。
 * * **明度差を見るのは、字と地を同じ規則で両方持つものだけ。**
 *   **帯の字と地はユーティリティでマークアップに書いてある**ので、ここには来ない。
 *
 * **ブラウザは起動しない**（jsdom すら要らない / 既定の `node` で走る）。
 */

import { describe, expect, it } from "vitest";

import {
  cssRules,
  declarationOf,
  locatedDeclarations,
  oklchLightness,
  sourceFiles,
  stylesheetOf,
  tokenReference,
  type CssRule,
  type ScannedFile,
} from "./support/design-tokens";

/** 走査が読んだもの（**1回だけ読む**）。 */
const SOURCES = sourceFiles();

/** トークンを定義している唯一のファイル。 */
const TOKENS = "app.css";

/** 暗い側の囲み（`tests/colour-scheme.test.ts` と同じ綴り）。 */
const DARK = "@media (prefers-color-scheme: dark)";

/** 土台の層（要素の既定）。 */
const BASE = "base";

/** 部品の層（区別が要るものだけのクラス）。 */
const COMPONENTS = "components";

/**
 * **文字の入力欄の選択子**（`input` の文字の欄・`textarea`・`select`）。
 *
 * **⚠ 人が書いた綴り。`app.css` から作らない**（CLAUDE.md / `16b`）。
 */
const TEXT_FIELDS = 'input:not([type="checkbox"], [type="radio"]), textarea, select';

/** **チェックボックス・ラジオを包む `label` の選択子**（人が書いた綴り）。 */
const CHECKABLE_LABEL =
  'label:has(> input[type="checkbox"]), label:has(> input[type="radio"])';

/**
 * **`<input …>` の1つ**（チェックボックスかラジオのもの）。
 *
 * **タグの終わりは「`=>` の一部でない `>`」** —— 属性の中の矢印関数で切らないため。
 * **⚠ 属性の中に素の `>`（比較）を書くと、そこで切れる。** そのときは型の綴りまで届かず、
 * 数え漏れる（下の「数えている」で 0 件になれば気づく）。
 */
const CHECKABLE_INPUT = /<input\b(?:=>|[^>])*>/g;

/**
 * そのタグがチェックボックスかラジオか。
 *
 * **綴りは3通り読む** —— `type="checkbox"` / `type='checkbox'` / `type={"checkbox"}`（`{'…'}` も）。
 * **⚠ `type={kind}` のように変数で渡す形は読めない**（値を解かない）。そのときは数え漏れる。
 */
function isCheckable(tag: string): boolean {
  return /\btype\s*=\s*(?:"|'|\{\s*["'])(?:checkbox|radio)["']/.test(tag);
}

/**
 * そのソースで、**`label` の直接の子になっていない**チェックボックス・ラジオの行番号（1 始まり）。
 *
 * **直前のタグが `<label …>` の開始で、その間に別のタグが無い**ときだけ「直接の子」と読む。
 * **間に注記（`<!-- -->`）を挟んでも咎める**（広く採る側に倒してある）。
 */
function checkablesOutsideLabel(source: string): number[] {
  const offences: number[] = [];
  for (const match of source.matchAll(CHECKABLE_INPUT)) {
    if (!isCheckable(match[0])) {
      continue;
    }
    const index = match.index ?? 0;
    const before = source.slice(0, index);
    const opener = before.lastIndexOf("<label");
    const between = opener < 0 ? null : before.slice(opener);
    // **⚠ `[\s\S]*?` で書かないこと** —— 最短一致でも後ろの `[^<]*$` が合わなければ伸び、
    // `<label>` の `>` を越えて `<span>` の `>` まで食う（植えたソースの対照が実際に止めた）。
    if (between === null || !/^<label\b(?:=>|[^>])*>[^<]*$/.test(between)) {
      offences.push(before.split("\n").length);
    }
  }
  return offences;
}

/** 輪に関わる宣言の綴り（`outline-offset` は輪を消さないので入れない）。 */
const RING_PROPERTIES = new Set(["outline", "outline-width", "outline-style", "outline-color"]);

/** 長さ 0（`0` / `0px` / `0.0rem` …）。 */
const ZERO_LENGTH = /^0(?:\.0+)?[a-z%]*$/;

/**
 * **アルファが 0 の色**（`oklch(… / 0)` / `rgb(… / 0%)` / `rgba(…, 0)` …）。
 * **値の全体に当てる**（語に割ると関数の中で切れるので）。
 * **`,` の形は引数が4つのときだけ** —— `rgb(10, 20, 0)` の3つ目の 0 は青の量で、透明ではない。
 */
const ZERO_ALPHA =
  /\/\s*0(?:\.0+)?%?\s*\)|\([^,()]*,[^,()]*,[^,()]*,\s*0(?:\.0+)?%?\s*\)/;

/**
 * **`all` で輪ごと戻す値**（作業単位 D-12b-1 の再レビュー）。
 *
 * **`all: unset` / `all: initial` は `outline` を初期値（`none`）へ戻す。**
 * **部品の `<style>` は層の外に在る**ので、土台の `:focus-visible` に勝つ ——
 * **輪の綴りを1文字も書かずに輪が消える。** `inherit` も親の `outline`（ふつうは無い）を引くので入れる。
 * **`revert` / `revert-layer` は入れない**（前者は閲覧器の既定の輪へ、後者は下の層の `:focus-visible` へ戻る）。
 */
const RING_RESETTING_ALL = /^(?:unset|initial|inherit)$/i;

/**
 * そのファイルのスタイルシートで、**フォーカスの輪を消す宣言**（`選択子 { 綴り: 値 }`）。
 *
 * **値の語を1つずつ見る**（`!important` は外してから）——
 * **`none` / `hidden`・長さ 0・`transparent`** のどれかが在るか、
 * **値の全体にアルファ 0 の色が在れば**、輪を消すと読む。**`all: unset` / `initial` / `inherit` も咎める。**
 *
 * **⚠ 守れていないこと**:
 * * **`var(…)` で太さを 0 にする・色を透明にする形は見えない**（値を解かない）。
 * * **輪の色を地と同じ色にする形**（`outline-color: var(--color-paper)`）**は見えない** ——
 *   **見えなくなるのは地の色に依るので、宣言だけでは決まらない。**
 */
function ringRemovals(file: ScannedFile): string[] {
  return locatedDeclarations(file)
    .filter((where) => {
      const value = where.value.replace(/!\s*important/i, "").trim();
      if (where.property === "all") {
        return RING_RESETTING_ALL.test(value);
      }
      if (!RING_PROPERTIES.has(where.property)) {
        return false;
      }
      return (
        ZERO_ALPHA.test(value) ||
        outsideParentheses(value)
          .split(/\s+/)
          .some((word) => /^(?:none|hidden|transparent)$/i.test(word) || ZERO_LENGTH.test(word))
      );
    })
    .map((where) => `${where.path}: ${where.selector} { ${where.property}: ${where.value} }`);
}

/**
 * 値から、**括弧の中身を落とした綴り**（`oklch(0 0 0)` → `oklch()`）。
 *
 * **語に割って長さ 0 を探すとき、関数の引数の `0` を太さと読まないため**（`rgb(0 0 0)` は不透明の黒）。
 * 入れ子は内側から畳む。
 */
function outsideParentheses(value: string): string {
  let current = value;
  for (;;) {
    const next = current.replace(/\([^()]*\)/g, "()");
    if (next === current) {
      return current;
    }
    current = next;
  }
}

/** そのソースで、チェックボックス・ラジオの `<input>` を数える（**文字の欄は数えない**）。 */
function countCheckables(source: string): number {
  return [...source.matchAll(CHECKABLE_INPUT)].filter((match) => isCheckable(match[0])).length;
}

function sourceAt(path: string): ScannedFile {
  const found = SOURCES.find((entry) => entry.path === path);
  expect(found, `${path} を走査が読んでいること`).toBeDefined();
  return found as ScannedFile;
}

/** トークンを定義しているファイルの、規則すべて。 */
function tokenRules(): CssRule[] {
  return cssRules(stylesheetOf(sourceAt(TOKENS)));
}

/** その層の、その選択子の、その宣言の値（無ければ `null`。**囲みの外だけ**）。 */
function declarationInLayer(
  rules: readonly CssRule[],
  layer: string,
  selector: string,
  property: string,
): string | null {
  return declarationOf(
    rules.filter((rule) => rule.layer === layer),
    selector,
    property,
  );
}

/** `2.5rem` / `40px` を px へ（**根の字の大きさは 16px と置く**）。読めなければ `null`。 */
function lengthInPx(value: string | null): number | null {
  if (value === null) {
    return null;
  }
  const found = /^([\d.]+)(rem|px)$/.exec(value.trim());
  if (found === null) {
    return null;
  }
  const amount = Number.parseFloat(found[1] as string);
  return found[2] === "rem" ? amount * 16 : amount;
}

/** **押せる高さの下限**（絵の「普通のボタン」の 40px。帯のタブの 44px は `app-nav` が見る）。 */
const MIN_CONTROL_PX = 40;

/**
 * **字と地の明度差の下限**（oklch の L / モジュール doc の ⚠）。
 *
 * **D-12b-1 の時点の最小は、主ボタンの明るい側の 0.485**（紙 0.985 / read 0.5）。
 * **主ボタンの字を墨に替えると、明るい側 0.26・暗い側 0.2 で止まる。**
 */
const MIN_LIGHTNESS_GAP = 0.4;

describe("土台は層に入っている", () => {
  /**
   * **層の順番の宣言**（`app.css` の冒頭）。
   *
   * **⚠ ここが崩れると、層に入れた意味が逆になる** ——
   * **`base` を `utilities` より後ろに宣言すると、土台がユーティリティに勝つ。**
   * **綴りは直書き**（並びの意味を、ここが持つ）。
   */
  it("層の順番は、主題・土台・部品・実用", () => {
    expect(sourceAt(TOKENS).source).toContain(
      "@layer theme, base, components, utilities;",
    );
  });

  /**
   * **層の外に在るのは、主題と暗い側の差し替えだけ。**
   *
   * **主題**（`@theme`）は Tailwind が自分で `theme` の層へ入れる。
   * **暗い側の差し替え**はカスタムプロパティしか宣言しないので、
   * **ユーティリティと同じ綴りを取り合わない**（ADR-0040 の追記）。
   *
   * **⚠ この一覧に足すときは、その規則がユーティリティと取り合わないことを先に確かめること。**
   */
  it("層の外に在るのは、主題と暗い側の差し替えだけ", () => {
    const unlayered = tokenRules()
      .filter((rule) => rule.layer === null)
      .map((rule) =>
        rule.within === null ? rule.selector : `${rule.within} ${rule.selector}`,
      );
    expect(unlayered).toEqual(["@theme", `${DARK} :root`]);
  });

  /**
   * **層は2つだけ使う**（`base` と `components`）。
   *
   * **`utilities` へ入れると、ユーティリティと同じ層で特異性の勝負になる** ——
   * **層に入れた理由**（ユーティリティが必ず勝つ）**が消える。**
   */
  it("使っている層は、土台と部品の2つだけ", () => {
    const layers = new Set(
      tokenRules()
        .map((rule) => rule.layer)
        .filter((layer) => layer !== null),
    );
    expect([...layers].sort()).toEqual([BASE, COMPONENTS]);
  });

  /**
   * **土台は要素の既定、部品はクラス。**
   *
   * **要素の既定に置くと、マークアップを書き換えなくても全画面に効く**
   * （ボタン・入力欄の見た目はこれで揃えた）。
   * **区別が要るもの**（主ボタン・カード・報せ）**だけをクラスにする。**
   */
  it("土台の層の選択子はクラスを持たず、部品の層の選択子はクラスを持つ", () => {
    const hasClass = (selector: string): boolean => /\.[A-Za-z_]/.test(selector);
    const rules = tokenRules();
    const baseWithClass = rules
      .filter((rule) => rule.layer === BASE && hasClass(rule.selector))
      .map((rule) => rule.selector);
    const componentsWithoutClass = rules
      .filter((rule) => rule.layer === COMPONENTS && !hasClass(rule.selector))
      .map((rule) => rule.selector);
    expect(baseWithClass).toEqual([]);
    expect(componentsWithoutClass).toEqual([]);
    // **空の層で緑にしない。**
    expect(rules.filter((rule) => rule.layer === BASE).length).toBeGreaterThanOrEqual(10);
    expect(
      rules.filter((rule) => rule.layer === COMPONENTS).length,
    ).toBeGreaterThanOrEqual(5);
  });
});

describe("共通の部品の見た目", () => {
  /**
   * **ボタンと文字の入力欄は、要素の既定で見た目を持つ** —— **値まで見る**（D-12b-1 のレビュー）。
   *
   * **⚠ ここが言えること**: **`button` / 文字の入力欄という綴りの規則が土台の層に在り、
   * 枠・地・字・角丸がこの値であること。** だから既存の画面の `<button>` / `<input>` は、
   * 印を付けなくても新しい見た目になる。
   *
   * **⚠ かつては「在る」しか見ていなかった** —— 枠を `border: none` / `0` にしても緑のままだった
   * （レビュアーの予想 / 変異で確かめた）。**値は人が書いた帳簿で、`app.css` から作らない**（`16b`）。
   */
  it.each([
    ["button", "border", "1px solid var(--color-rule-2)"],
    ["button", "background", "var(--color-paper)"],
    ["button", "color", "var(--color-ink)"],
    ["button", "border-radius", "0.375rem"],
    [TEXT_FIELDS, "border", "1px solid var(--color-rule-2)"],
    [TEXT_FIELDS, "background", "var(--color-paper)"],
    [TEXT_FIELDS, "color", "var(--color-ink)"],
    [TEXT_FIELDS, "border-radius", "0.375rem"],
  ])("`%s` は土台の層で `%s: %s`", (selector, property, value) => {
    expect(declarationInLayer(tokenRules(), BASE, selector, property)).toBe(value);
  });

  /**
   * **フォーカスの輪は、要素を選ばずに土台で引く。**
   *
   * **⚠ 輪を消す宣言は、土台にも部品の `<style>` にも置かない**（下の [`ringRemovals`]）。
   * **キーボードで操る人は、いまどこにいるかを輪でしか知らない。**
   */
  it("フォーカスの輪は、土台の層で read の色を引く", () => {
    const ring = declarationInLayer(tokenRules(), BASE, ":focus-visible", "outline");
    expect(ring).toBe("2px solid var(--color-read)");
  });

  /**
   * **輪を消す宣言が、走査の射程のどこにも無い**（`app.css` ＋ 部品の `<style>` ＋ 入口の文書）。
   *
   * **⚠ かつては `app.css` の `outline` / `outline-style` が `none` / `0` と完全に一致する形しか見ていなかった**
   * （D-12b-1 のレビュー）。`outline-width: 0` / `outline: 0 solid …` / `outline: none !important` /
   * `outline: 0px` は素通りし、部品の `<style>` は読んでもいなかった。
   * **再レビューで `all: unset` / `initial` とアルファ 0 の色も足した。** 射程外は [`ringRemovals`] の doc。
   */
  it("輪を消す宣言が、どこにも無い", () => {
    const offences = SOURCES.flatMap((entry) => ringRemovals(entry));
    expect(offences, offences.join(" / ")).toEqual([]);
  });

  /** **チェックボックスは、大きさと色を土台で持つ**（色は read）。 */
  it("チェックボックスは、土台の層で read の色を持つ", () => {
    const accent = declarationInLayer(
      tokenRules(),
      BASE,
      'input[type="checkbox"], input[type="radio"]',
      "accent-color",
    );
    expect(accent).toBe("var(--color-read)");
  });

  /**
   * **チェックボックスを包む `label` は、箱と文言を横に並べる**（D-12b-1 のレビュー / 司令塔がブラウザで実測）。
   *
   * **土台の `form label` は「見出し / 入力 / 文字数」の格子**（`10rem 1fr auto`）なので、
   * **チェックボックスを包む `label` もそこへ入り、箱が 160px の列に置かれて、文言が 168px 右から始まっていた**
   * （上流の画面の「裏方（抽出・変容）を別の上流にする」。着手前からの崩れ）。
   * **`:has(> …)` の綴りは `form label` より強い**（`:has` は中の選択子の強さを持つ）ので、
   * 同じ土台の層の中で、記述の順に依らず勝つ。
   */
  it.each([
    ["display", "flex"],
    ["align-items", "center"],
    // **押せる高さ 44px**（文言ごと押せる / 絵の Edit）。**名乗っている値なので、値で押さえる**（再レビュー）。
    ["min-height", "2.75rem"],
  ])("チェックボックスを包む `label` は、土台の層で `%s: %s`", (property, value) => {
    expect(declarationInLayer(tokenRules(), BASE, CHECKABLE_LABEL, property)).toBe(value);
  });

  /**
   * **上の綴りが当たる形でマークアップが書いてある** —— **チェックボックスとラジオは、`label` の直接の子。**
   *
   * **⚠ `:has(> …)` は直接の子にしか当たらない。** 箱を `<span>` で包んだ日に、規則は黙って外れ、
   * 箱と文言はまた格子で離れる。**この1本がそれを止める**（いまの在り処は `lib/UpstreamProfiles.svelte` の1つだけ）。
   */
  it("チェックボックスとラジオは、どの部品でも `label` の直接の子", () => {
    const svelte = SOURCES.filter((entry) => entry.path.endsWith(".svelte"));
    const offences = svelte.flatMap((entry) =>
      checkablesOutsideLabel(entry.source).map((line) => `${entry.path}:${line}`),
    );
    expect(offences, offences.join(" / ")).toEqual([]);
    // **数えている**（0 件なら上は何も見ていない）。
    // **⚠ チェックボックス・ラジオだけを数える**（再レビュー）—— かつては `<input>` を全部数えていたので、
    // **文字の欄だけで必ず 1 以上になり、`isCheckable` が何にも当たらなくなっても緑だった**（自己成就）。
    const found = svelte.reduce((sum, entry) => sum + countCheckables(entry.source), 0);
    expect(found).toBeGreaterThanOrEqual(1);
  });

  /**
   * **主ボタンとカードは、部品の層のクラス** —— **値まで見る**（D-12b-1 のレビュー）。
   *
   * **⚠ 部品の層に置く理由**: 土台（`button`）より必ず勝ち、
   * **ユーティリティには必ず負ける**（画面ごとに余白などを上書きできる）。
   */
  it.each([
    [".primary", "background", "var(--color-read)"],
    [".primary", "border-color", "var(--color-read)"],
    [".primary", "color", "var(--color-paper)"],
    [".card", "border", "1px solid var(--color-rule)"],
    [".card", "background", "var(--color-paper)"],
    [".card", "border-radius", "0.625rem"],
    [".card", "padding", "1.25rem"],
  ])("`%s` は部品の層で `%s: %s`", (selector, property, value) => {
    expect(declarationInLayer(tokenRules(), COMPONENTS, selector, property)).toBe(value);
  });

  /** **押せる高さ**（モジュール doc の表）。 */
  it.each([["button"], [TEXT_FIELDS]])("`%s` の押せる高さ", (selector) => {
    const height = lengthInPx(
      declarationInLayer(tokenRules(), BASE, selector, "min-height"),
    );
    expect(height, "min-height が rem か px で書いてあること").not.toBeNull();
    expect(height as number).toBeGreaterThanOrEqual(MIN_CONTROL_PX);
  });
});

/** その綴りの、その配色での値（明るい側は `@theme`、暗い側は暗い側の `:root`）。 */
function tokenValue(
  rules: readonly CssRule[],
  scheme: "light" | "dark",
  name: string,
): string | null {
  if (scheme === "light") {
    return declarationOf(rules, "@theme", name);
  }
  let value: string | null = null;
  for (const rule of rules) {
    if (rule.within !== DARK || rule.selector !== ":root") {
      continue;
    }
    for (const entry of rule.declarations) {
      if (entry.property === name) {
        value = entry.value;
      }
    }
  }
  return value;
}

/** 字と地を同じ規則で両方持つもの（**どちらもトークン1つの `var()`**）。 */
function inkOnPaperRules(
  rules: readonly CssRule[],
): { selector: string; ink: string; paper: string }[] {
  const found: { selector: string; ink: string; paper: string }[] = [];
  for (const rule of rules) {
    if (rule.layer !== BASE && rule.layer !== COMPONENTS) {
      continue;
    }
    const value = (property: string): string | null =>
      rule.declarations.find((entry) => entry.property === property)?.value ?? null;
    const ink = tokenReference(value("color") ?? "");
    const paper = tokenReference(value("background") ?? value("background-color") ?? "");
    if (ink !== null && paper !== null) {
      found.push({ selector: rule.selector, ink, paper });
    }
  }
  return found;
}

describe("暗い側でも、字と地が寄らない", () => {
  /**
   * **字と地を両方持つ規則の明度差**（モジュール doc の ⚠ —— 粗い下限）。
   *
   * **実ファイルから集める**が、**下限はこの検査が持つ**ので、
   * 字か地の綴りを入れ替える変異はここで止まる。
   * **空の一覧で緑にしないよう、主ボタンとボタンが居ることを先に言う。**
   */
  it("字と地の明度差が、明るい側でも暗い側でも開いている", () => {
    const rules = tokenRules();
    const pairs = inkOnPaperRules(rules);
    const selectors = pairs.map((pair) => pair.selector);
    expect(selectors).toContain("button");
    expect(selectors).toContain(".primary");

    const offences: string[] = [];
    for (const pair of pairs) {
      for (const scheme of ["light", "dark"] as const) {
        const ink = oklchLightness(tokenValue(rules, scheme, pair.ink) ?? "");
        const paper = oklchLightness(tokenValue(rules, scheme, pair.paper) ?? "");
        if (ink === null || paper === null) {
          offences.push(`${pair.selector} (${scheme}): 値が oklch で読めない`);
          continue;
        }
        const gap = Math.abs(ink - paper);
        if (gap < MIN_LIGHTNESS_GAP) {
          offences.push(`${pair.selector} (${scheme}): ${gap.toFixed(3)}`);
        }
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });
});

/**
 * **色を混ぜる関数・色を相対で作る綴り**（`color-mix(` / `oklch(from …)` の類）。
 *
 * **宣言の値だけを見る**（コメントは落とす —— 理由を書いた注記で赤くしない）。
 */
const COLOUR_DERIVATION = /color-mix\(|\(\s*from\s/;

/** そのファイルのスタイルシートで、色を混ぜている宣言（`選択子 { 綴り }`）。 */
function colourDerivations(file: ScannedFile): string[] {
  return locatedDeclarations(file)
    .filter((where) => COLOUR_DERIVATION.test(where.value))
    .map((where) => `${where.path}: ${where.selector} { ${where.property} }`);
}

describe("配られる CSS に、生の色を作らせない", () => {
  /**
   * **色を混ぜる関数を使わない**（作業単位 D-12b-1 で実測）。
   *
   * **主ボタンの押したときの地を `color-mix(in oklch, var(--color-read) …, var(--color-ink))` で書いたら、
   * `pnpm build` が `.primary:hover:not(:disabled){background:#285c8d}` を書き足した**
   * （その関数を読めない閲覧器のための代替。**値は明るい側のトークンで計算済み**）。
   * **`web/src` には生の 16 進が1つも無いのに、配られる CSS には在る** ——
   * **しかも暗い側でも明るい側の色が出る。** `design-tokens.test.ts` の走査はソースしか見ないので、
   * **ここで書き方そのものを止める。**
   *
   * **⚠ 守れていないこと**: **ビルドが代替を書き足す綴りは、これだけとは限らない。**
   * **見ているのは、D-12b-1 で実際に代替が生えた形と、同じ仕組み（相対の色）の綴りまで。**
   */
  it("色を混ぜる関数を使わない", () => {
    const offences = SOURCES.flatMap((entry) => colourDerivations(entry));
    expect(offences, offences.join(" / ")).toEqual([]);
  });
});

/**
 * **対照 —— 植えたソースを、同じ引き方に通す**（CLAUDE.md / `15b`）。
 *
 * **CRLF の腕を対にする**（作業ツリーの `.css` は CRLF / `E-1`）。
 */
describe("対照: 植えたソースで、層と明度差の読み方が効いていることを見る", () => {
  const PLANTED = [
    "@theme {",
    "  --color-paper: oklch(0.985 0.004 85);",
    "  --color-ink: oklch(0.24 0.012 265);",
    "  --color-read: oklch(0.5 0.11 250);",
    "}",
    "@layer base {",
    "  button {",
    "    min-height: 2.5rem;",
    "    color: var(--color-ink);",
    "    background: var(--color-paper);",
    "  }",
    "}",
    "@layer components {",
    "  .primary {",
    "    color: var(--color-ink);",
    "    background: var(--color-read);",
    "  }",
    "}",
    "@media (prefers-color-scheme: dark) {",
    "  :root {",
    "    --color-paper: oklch(0.2 0.004 85);",
    "    --color-ink: oklch(0.92 0.012 265);",
    "    --color-read: oklch(0.72 0.11 250);",
    "  }",
    "}",
  ];

  function check(newline: string): void {
    const rules = cssRules(PLANTED.join(newline));
    expect(declarationInLayer(rules, BASE, "button", "min-height")).toBe("2.5rem");
    // **層が違えば取れない。**
    expect(declarationInLayer(rules, COMPONENTS, "button", "min-height")).toBeNull();
    expect(lengthInPx("2.5rem")).toBe(40);
    expect(lengthInPx("44px")).toBe(44);
    expect(lengthInPx("calc(1rem + 2px)")).toBeNull();

    expect(tokenValue(rules, "light", "--color-read")).toBe("oklch(0.5 0.11 250)");
    expect(tokenValue(rules, "dark", "--color-read")).toBe("oklch(0.72 0.11 250)");

    expect(inkOnPaperRules(rules)).toEqual([
      { selector: "button", ink: "--color-ink", paper: "--color-paper" },
      { selector: ".primary", ink: "--color-ink", paper: "--color-read" },
    ]);
    // **墨の字を read の地に置いた主ボタンは、下限を割る**（両方の配色で）。
    const gap = (scheme: "light" | "dark"): number =>
      Math.abs(
        (oklchLightness(tokenValue(rules, scheme, "--color-ink") ?? "") as number) -
          (oklchLightness(tokenValue(rules, scheme, "--color-read") ?? "") as number),
      );
    expect(gap("light")).toBeLessThan(MIN_LIGHTNESS_GAP);
    expect(gap("dark")).toBeLessThan(MIN_LIGHTNESS_GAP);
    // **墨の字を紙の地に置いたボタンは、下限を越える。**
    expect(
      Math.abs(
        (oklchLightness(tokenValue(rules, "light", "--color-ink") ?? "") as number) -
          (oklchLightness(tokenValue(rules, "light", "--color-paper") ?? "") as number),
      ),
    ).toBeGreaterThanOrEqual(MIN_LIGHTNESS_GAP);
  }

  it("LF で組んだソース", () => {
    check("\n");
  });

  it("CRLF で組んだソース", () => {
    check("\r\n");
  });

  it("色を混ぜる宣言を見つけ、注記の中の綴りは咎めない", () => {
    const planted = [
      "<style>",
      "  /* 注記: color-mix( と書いても咎めない */",
      "  .a { background: color-mix(in oklch, var(--color-read) 85%, var(--color-ink)); }",
      "  .b { color: oklch(from var(--color-read) l c h); }",
      "  .c { color: var(--color-read); }",
      "</style>",
    ];
    for (const newline of ["\n", "\r\n"]) {
      expect(
        colourDerivations({ path: "lib/X.svelte", source: planted.join(newline) }),
      ).toEqual(["lib/X.svelte: .a { background }", "lib/X.svelte: .b { color }"]);
    }
  });

  it("輪を消す宣言を、書き方を問わず見つける", () => {
    const planted = [
      "<style>",
      "  /* 注記: outline: none と書いても咎めない */",
      "  .ring { outline: 2px solid var(--color-read); outline-offset: 0; }",
      "  .a { outline: none; }",
      "  .b { outline-width: 0; }",
      "  .c { outline: 0 solid var(--color-read); }",
      "  .d { outline: none !important; }",
      "  .e { outline: 0px; }",
      "  .f { outline-style: hidden; }",
      "  .g { outline-color: transparent; }",
      "  .h { all: unset; }",
      "  .i { all: initial; }",
      "  .j { outline-color: oklch(0.5 0.11 250 / 0); }",
      "  .k { outline: 2px solid rgb(0 0 0 / 0%); }",
      "  .l { outline-color: rgba(0, 0, 0, 0); }",
      // **咎めない側**: 半透明・不透明の古い綴り・閲覧器の既定へ戻す `all`・地と同じ色（射程外 / doc）。
      "  .m { outline-color: oklch(0.5 0.11 250 / 0.5); }",
      "  .n { outline-color: rgb(10, 20, 0); }",
      "  .o { all: revert; }",
      "  .p { outline-color: var(--color-paper); }",
      // **関数の引数の 0 は太さではない**（不透明の黒）。
      "  .q { outline: 2px solid oklch(0 0 0); }",
      "</style>",
    ];
    for (const newline of ["\n", "\r\n"]) {
      expect(ringRemovals({ path: "lib/X.svelte", source: planted.join(newline) })).toEqual([
        "lib/X.svelte: .a { outline: none }",
        "lib/X.svelte: .b { outline-width: 0 }",
        "lib/X.svelte: .c { outline: 0 solid var(--color-read) }",
        "lib/X.svelte: .d { outline: none !important }",
        "lib/X.svelte: .e { outline: 0px }",
        "lib/X.svelte: .f { outline-style: hidden }",
        "lib/X.svelte: .g { outline-color: transparent }",
        "lib/X.svelte: .h { all: unset }",
        "lib/X.svelte: .i { all: initial }",
        "lib/X.svelte: .j { outline-color: oklch(0.5 0.11 250 / 0) }",
        "lib/X.svelte: .k { outline: 2px solid rgb(0 0 0 / 0%) }",
        "lib/X.svelte: .l { outline-color: rgba(0, 0, 0, 0) }",
      ]);
    }
  });

  it("チェックボックス・ラジオの綴りを3通り読み、文字の欄は数えない", () => {
    const planted = [
      '<input type="checkbox" />',
      "<input type='radio' />",
      '<input type={"checkbox"} />',
      "<input type={ 'radio' } />",
      '<input type="text" />',
      "<input name=\"a\" />",
      '<input type="checkboxes" />',
    ].join("\n");
    expect(countCheckables(planted)).toBe(4);
    // **文字の欄だけのソースは 0**（「数えている」が自己成就しないこと）。
    expect(countCheckables('<label>\n  <input type="text" />\n</label>')).toBe(0);
  });

  it("`label` の直接の子でないチェックボックス・ラジオを見つける", () => {
    const planted = [
      "<label>",
      '  <input type="checkbox" onchange={(event) => { x = 1; }} />',
      "  直接の子",
      "</label>",
      "<label>",
      '  <span><input type="radio" /></span>',
      "</label>",
      '<input type="checkbox" />',
      '<label class="x" onclick={() => y()}>',
      '  <input name="a" type="radio" />',
      "</label>",
      "<label>",
      '  <input type="text" />',
      "</label>",
      '<div><input type="text" /></div>',
      // **単引用符と波括弧の綴りも、包めば咎める**（再レビュー）。
      "<label>",
      "  <span><input type='checkbox' /></span>",
      "</label>",
      '<div><input type={"radio"} /></div>',
    ];
    for (const newline of ["\n", "\r\n"]) {
      // **2行目（直接の子）・10行目（矢印関数を持つ label の直接の子）・13/15行目（文字の欄）は咎めない。**
      expect(checkablesOutsideLabel(planted.join(newline))).toEqual([6, 8, 17, 19]);
    }
  });

  it("`oklch()` の明度を読む", () => {
    expect(oklchLightness("oklch(0.985 0.004 85)")).toBe(0.985);
    expect(oklchLightness(" oklch(0.2 0.004 85) ")).toBe(0.2);
    expect(oklchLightness("var(--color-ink)")).toBeNull();
    expect(oklchLightness("")).toBeNull();
  });
});
