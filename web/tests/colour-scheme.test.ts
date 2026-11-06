/**
 * **明るい側と暗い側と、狭い幅**（作業単位 D-11b）。
 *
 * # ここで見るものと、見ないもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **明るい側の `@theme` の「値」が、D-11a のままであること** | **ここ** |
 * | **暗い側が、色の 12 個を全部上書きしていること** | **ここ** |
 * | **暗い側の色相が、明るい側と同じであること**（暖色は暖色、寒色は寒色） | **ここ** |
 * | **暗い側が、明るい側と違う値であること**（＝何もしていない箱でないこと） | **ここ** |
 * | **狭い幅の規則が、広い幅と同じ選択子を狙っていること** | **ここ** |
 * | **トークンの綴りが1箇所にしかないこと / 字の割り当て / 意味の帳簿** | `design-tokens.test.ts` |
 * | **その色が暗い紙の上で読めるか** | **どこにも無い。人が見るしかない** |
 *
 * # ⚠ 守れていないこと
 *
 * * **彩度と明度は、暗い側でも誰も見ていない。** 見ているのは色相だけである
 *   （明るい側と同じ立場 / `design-tokens.test.ts` の「守れていないこと」）。
 * * **配られた CSS に暗い側の規則が入ったかは、ここでは言えない。**
 *   ここは Tailwind を1度も走らせない。**測るならビルドを回すしかない。**
 * * **狭い幅で「実際にはみ出さないか」は、ここでは言えない。**
 *   ここが見ているのは**ソースの上の綴り**だけで、幅を持った画面は1つも作らない。
 *   **部品の側の `<style>` は、こちらより強い綴りを持つ**ので、
 *   土台の狭い幅の規則で上書きできない箇所が実在する（`web/README.md`）。
 *
 * # ⚠ 色相の帳簿は、明るい側から繋いでいる
 *
 * **暖色 / 寒色の境目は `design-tokens.test.ts` が1つだけ持っている。**
 * **ここでは同じ境目を写さず、「暗い側の色相が明るい側と同じであること」を見る** ——
 * 明るい側が暖色だと向こうが言い、こちらが「同じ」だと言えば、暗い側も暖色である。
 * **写しを2つ置くと、片方だけが黙って古びる**（CLAUDE.md）。
 *
 * **ブラウザは起動しない**（jsdom すら要らない / 既定の `node` で走る）。
 */

import { describe, expect, it } from "vitest";

import {
  cssRules,
  declarationOf,
  oklchHue,
  sourceFiles,
  stylesheetOf,
  type CssRule,
  type ScannedFile,
} from "./support/design-tokens";

/** 走査が読んだもの（**1回だけ読む**）。 */
const SOURCES = sourceFiles();

/** トークンを定義している唯一のファイル。 */
const TOKENS = "app.css";

/** 明るい側の置き場所（Tailwind v4 の主題 / ADR-0040 §決定2）。 */
const THEME = "@theme";

/**
 * **暗い側の囲み。**
 *
 * **`@theme` を2つ置く形は採っていない** —— `@theme` は
 * 「ユーティリティが生える元」でもあるので、条件つきで2つ置くと
 * **生えるクラスの集合が条件に依る。** 理由は `src/app.css` の冒頭にある。
 */
const DARK = "@media (prefers-color-scheme: dark)";

/** **狭い幅の囲み。** 境目の理由は `src/app.css` の冒頭にある。 */
const NARROW = "@media (max-width: 40rem)";

/** 暗い側も、明るい側も、置き場所は `:root`。 */
const ROOT = ":root";

function sourceAt(path: string): ScannedFile {
  const found = SOURCES.find((entry) => entry.path === path);
  expect(found, `${path} を走査が読んでいること`).toBeDefined();
  return found as ScannedFile;
}

/** トークンを定義しているファイルの、規則すべて。 */
function tokenRules(): CssRule[] {
  return cssRules(stylesheetOf(sourceAt(TOKENS)));
}

/**
 * **その囲みの中の、その選択子の、その宣言の値**（無ければ `null`）。
 *
 * **`declarationOf` は囲みの外だけを見る**（`support` の [`CssRule`] の ⚠）ので、
 * 暗い側と狭い幅を読むにはこちらが要る。
 * **同じ綴りが2度出てくれば後の勝ち**（CSS と同じ / `declarationOf` と揃えてある）。
 *
 * **⚠ 対照は下の「植えたソース」で、LF と CRLF の両方を通している**
 * （CLAUDE.md / `15b` ＋ `E-1`）。
 */
function declarationIn(
  rules: readonly CssRule[],
  within: string,
  selector: string,
  property: string,
): string | null {
  let value: string | null = null;
  for (const rule of rules) {
    if (rule.within !== within || rule.selector !== selector) {
      continue;
    }
    for (const declaration of rule.declarations) {
      if (declaration.property === property) {
        value = declaration.value;
      }
    }
  }
  return value;
}

/** その囲みの中に在る規則の、選択子（出てきた順）。 */
function selectorsIn(rules: readonly CssRule[], within: string): string[] {
  return rules
    .filter((rule) => rule.within === within)
    .map((rule) => rule.selector);
}

/** その囲みの中の、その選択子が持つ宣言の綴り（出てきた順）。 */
function propertiesIn(
  rules: readonly CssRule[],
  within: string,
  selector: string,
): string[] {
  return rules
    .filter((rule) => rule.within === within && rule.selector === selector)
    .flatMap((rule) => rule.declarations.map((entry) => entry.property));
}

/**
 * **明るい側の値の帳簿**（作業単位 D-11a のまま）。
 *
 * **⚠ 権威は `git show 2570aa3:web/src/app.css`。**
 * **`app.css` から作らない** —— 作ると、値を動かす変異で期待値も一緒に動いて
 * 緑になる（CLAUDE.md / `16b`）。
 *
 * **⚠ `design-tokens.test.ts` は「綴りが在ること」までしか見ていない。**
 * **値を見るのはここだけである** —— D-11b は暗い側を足す作業単位なので、
 * **明るい側を巻き添えで動かしていないこと**を言う必要がある。
 */
const LIGHT_VALUES: [string, string][] = [
  ["--color-paper", "oklch(0.985 0.004 85)"],
  ["--color-paper-2", "oklch(0.963 0.005 85)"],
  ["--color-paper-3", "oklch(0.938 0.006 85)"],
  ["--color-ink", "oklch(0.24 0.012 265)"],
  ["--color-ink-2", "oklch(0.44 0.01 265)"],
  ["--color-muted", "oklch(0.6 0.008 265)"],
  ["--color-rule", "oklch(0.905 0.006 85)"],
  ["--color-rule-2", "oklch(0.82 0.008 85)"],
  ["--color-read", "oklch(0.5 0.11 250)"],
  ["--color-read-bg", "oklch(0.955 0.028 250)"],
  ["--color-warn", "oklch(0.56 0.11 65)"],
  ["--color-warn-bg", "oklch(0.96 0.035 75)"],
  [
    "--font-mincho",
    '"Yu Mincho", "YuMincho", "Hiragino Mincho ProN", "MS PMincho", serif',
  ],
  [
    "--font-sans",
    '"Yu Gothic UI", "Yu Gothic", "Hiragino Sans", "Meiryo", system-ui, sans-serif',
  ],
  [
    "--font-mono",
    '"Cascadia Mono", Consolas, ui-monospace, "SFMono-Regular", monospace',
  ],
];

/**
 * **暗い側で上書きする綴り、全部**（**色の 12 個だけ**）。
 *
 * **⚠ この一覧はここにしか無い。**
 * `design-tokens.test.ts` の一覧は**書体を3つ含む 15 個**で、別の帳簿である ——
 * **書体は暗い側で上書きしない**（字は明るさで変わらない）という、別の決め。
 */
const DARK_NAMES = [
  "--color-paper",
  "--color-paper-2",
  "--color-paper-3",
  "--color-ink",
  "--color-ink-2",
  "--color-muted",
  "--color-rule",
  "--color-rule-2",
  "--color-read",
  "--color-read-bg",
  "--color-warn",
  "--color-warn-bg",
];

/** **色相を持つ綴り**（accent の4つ）。暖色 / 寒色の帳簿は明るい側が持つ。 */
const ACCENT_NAMES = [
  "--color-read",
  "--color-read-bg",
  "--color-warn",
  "--color-warn-bg",
];

/**
 * **狭い幅で足した宣言、全部**（`選択子 { 綴り: 値 }` の形。**出てきた順**）。
 *
 * **⚠ 人が `src/app.css` を読んで書いた帳簿である。`app.css` から作らない** ——
 * 作ると、宣言を落とす変異で期待値も一緒に消えて緑になる（CLAUDE.md / `16b`）。
 *
 * **⚠ これが無いと、値を見ているのは 8 つのうち 3 つだけだった**
 * （`form label` の列と、`header` / `nav` の折り返し）——
 * **D-11b のレビューが実測: `overflow-wrap` ×2 と `max-width` / `min-width` ×3 を
 * 全部落とす変異が 846 全緑を通った。**
 * **上の「選択子が広い幅にも在る」は、選択子しか見ていない**ので、
 * **中身を空にしても掛からない。**
 *
 * **⚠ 並び順まで見る。** CSS は記述順が勝ち負けを決めうるので、
 * 集合に落とさない（暗い側のトークンは順番に意味が無いので、あちらは並べ替えている）。
 */
const NARROW_DECLARATIONS = [
  "nav { flex-wrap: wrap }",
  "header { flex-wrap: wrap }",
  "form label { grid-template-columns: 1fr }",
  "input, select, textarea { max-width: 100% }",
  "input, select, textarea { min-width: 0 }",
  "label input, label select { min-width: 0 }",
  "code { overflow-wrap: anywhere }",
  "th, td { overflow-wrap: anywhere }",
];

describe("明るい側の値は、D-11a のまま", () => {
  it.each(LIGHT_VALUES)("`%s` の値", (name, value) => {
    expect(declarationOf(tokenRules(), THEME, name)).toBe(value);
  });
});

describe("暗い配色は、トークンの上書きだけ", () => {
  /**
   * **囲みが在ること自体を、先に言う。**
   *
   * **これが無いと、下の `it.each` は「値が `null` だから」で赤くなる** ——
   * 止まりはするが、名指しする名前が違う（CLAUDE.md / `B-38`）。
   */
  it("暗い側の囲みが、`:root` を1つ持っている", () => {
    expect(selectorsIn(tokenRules(), DARK)).toEqual([ROOT]);
  });

  /**
   * **過不足なく 12 個**（多くても少なくても赤）。
   *
   * **並び順は見ていない** —— CSS 側の並びは理由ごとの塊で決まっていて、
   * **順番そのものには意味が無い。** 見ているのは集合である。
   */
  it("上書きしているのは、色の 12 個ちょうど", () => {
    expect([...propertiesIn(tokenRules(), DARK, ROOT)].sort()).toEqual(
      [...DARK_NAMES].sort(),
    );
  });

  it.each(DARK_NAMES)("`%s` は、明るい側と違う値になっている", (name) => {
    const rules = tokenRules();
    const light = declarationOf(rules, THEME, name);
    const dark = declarationIn(rules, DARK, ROOT, name);
    expect(light, `${name} が明るい側に在ること`).not.toBeNull();
    expect(dark, `${name} が暗い側に在ること`).not.toBeNull();
    expect(dark).not.toBe(light);
  });

  /**
   * **暖色は暖色のまま、寒色は寒色のまま。**
   *
   * **境目は写さない**（モジュール doc の ⚠）——
   * **明るい側と同じ色相であること**を見れば、
   * 明るい側の帳簿（`design-tokens.test.ts`）がそのまま繋がる。
   */
  it.each(ACCENT_NAMES)("`%s` の色相は、明るい側と同じ", (name) => {
    const rules = tokenRules();
    const light = oklchHue(declarationOf(rules, THEME, name) ?? "");
    const dark = oklchHue(declarationIn(rules, DARK, ROOT, name) ?? "");
    expect(light, `${name} の明るい側が oklch であること`).not.toBeNull();
    expect(dark, `${name} の暗い側が oklch であること`).not.toBeNull();
    expect(dark).toBe(light);
  });

  /**
   * **閲覧器が用意する部分も追従させる。**
   *
   * **これが無いと、入力欄の既定・巻き取りの帯だけが明るいまま残る。**
   * **囲みの外に置く**（両方を名乗る形）ので、`prefers-color-scheme` を見る必要が無い。
   */
  it("`color-scheme` を名乗っている", () => {
    expect(declarationOf(tokenRules(), ROOT, "color-scheme")).toBe(
      "light dark",
    );
  });
});

describe("狭い幅", () => {
  /**
   * **狭い幅で狙う選択子は、広い幅にも実在する。**
   *
   * **⚠ これが無いと、綴りを1文字ずらした規則が「在るのに効かない」形で通る。**
   * **土台の側にも部品の側にも当たらない選択子は、書いた本人にしか分からない。**
   *
   * **⚠ 比べるのは選択子の綴りそのもの**（`,` の並びまで含めて同じであること）。
   * 「同じ要素に当たるか」は CSS を解釈しないと言えないので、そこは見ていない。
   */
  it("狭い幅の選択子は、全部どこかの素の規則にも在る", () => {
    const plain = new Set(
      SOURCES.flatMap((entry) =>
        cssRules(stylesheetOf(entry))
          .filter((rule) => rule.within === null)
          .map((rule) => rule.selector),
      ),
    );
    const narrow = selectorsIn(tokenRules(), NARROW);
    expect(narrow.length, "狭い幅の規則が在ること").toBeGreaterThanOrEqual(4);
    expect(narrow.filter((selector) => !plain.has(selector))).toEqual([]);
  });

  /** **フォームの3列を、狭い幅で1列に畳む。** */
  it("フォームの列は、広い幅と狭い幅で別の値になっている", () => {
    const rules = tokenRules();
    const wide = declarationOf(rules, "form label", "grid-template-columns");
    const narrow = declarationIn(
      rules,
      NARROW,
      "form label",
      "grid-template-columns",
    );
    expect(wide, "広い幅の列が在ること").not.toBeNull();
    expect(narrow, "狭い幅の列が在ること").not.toBeNull();
    expect(narrow).not.toBe(wide);
    // **広い幅は3列**（見出し / 入力 / 文字数）、**狭い幅は1列。**
    expect((wide as string).split(" ").length).toBeGreaterThan(1);
    expect(narrow).toBe("1fr");
  });

  /** **見出しの帯と、画面を選ぶ帯は、狭い幅で折り返す。** */
  it.each(["header", "nav"])("`%s` は、狭い幅で折り返す", (selector) => {
    expect(declarationIn(tokenRules(), NARROW, selector, "flex-wrap")).toBe(
      "wrap",
    );
  });

  /**
   * **狭い幅で足した宣言を、1つ残らず突き合わせる。**
   *
   * 上の3本が値を見ているのは 8 つのうち 3 つだけで、
   * **残り5つ**（`max-width` / `min-width` ×3 / `overflow-wrap` ×2）**は無防備だった**
   * （[`NARROW_DECLARATIONS`] の ⚠）。
   */
  it("狭い幅の宣言は、帳簿とちょうど同じ", () => {
    const said = tokenRules()
      .filter((rule) => rule.within === NARROW)
      .flatMap((rule) =>
        rule.declarations.map(
          (entry) => `${rule.selector} { ${entry.property}: ${entry.value} }`,
        ),
      );
    expect(said).toEqual(NARROW_DECLARATIONS);
  });
});

/**
 * **対照 —— 植えたソースを、実ファイルと同じ判定に通す。**
 *
 * 実ファイルだけを読む対照は**自己成就する**（CLAUDE.md / `15b`）。
 * **上の判定が使っている引き方**（[`declarationIn`] / [`selectorsIn`] /
 * [`propertiesIn`]）**に、既知のソースを通して答えを固定する。**
 *
 * **⚠ CRLF の腕を対にしてある**（CLAUDE.md / `E-1`）——
 * **作業ツリーの `.css` は checkout 由来で CRLF、新しく書いたファイルは LF。**
 * **LF で書いた対照だけだと、実ファイルより弱い対照になる。**
 */
describe("対照: 植えたソースで、囲みの読み分けが効いていることを見る", () => {
  /** 明るい側と暗い側と狭い幅を、全部持つ既知のソース。 */
  const PLANTED = [
    "@theme {",
    "  --color-warn: oklch(0.56 0.11 65);",
    "}",
    ":root {",
    "  color-scheme: light dark;",
    "}",
    "form label {",
    "  grid-template-columns: 10rem 1fr auto;",
    "}",
    "@media (prefers-color-scheme: dark) {",
    "  :root {",
    "    --color-warn: oklch(0.75 0.11 65);",
    "  }",
    "}",
    "@media (max-width: 40rem) {",
    "  form label {",
    "    grid-template-columns: 1fr;",
    "  }",
    "  header {",
    "    flex-wrap: wrap;",
    "  }",
    "}",
  ];

  /** その改行で組んだ植えたソースを、同じ判定に通す。 */
  function check(newline: string): void {
    const rules = cssRules(PLANTED.join(newline));

    // **囲みの外**（明るい側）。
    expect(declarationOf(rules, THEME, "--color-warn")).toBe(
      "oklch(0.56 0.11 65)",
    );
    expect(declarationOf(rules, ROOT, "color-scheme")).toBe("light dark");
    expect(declarationOf(rules, "form label", "grid-template-columns")).toBe(
      "10rem 1fr auto",
    );

    // **囲みの中**（暗い側）——**外の引き方では取れない。**
    expect(declarationIn(rules, DARK, ROOT, "--color-warn")).toBe(
      "oklch(0.75 0.11 65)",
    );
    expect(declarationOf(rules, ROOT, "--color-warn")).toBeNull();
    expect(selectorsIn(rules, DARK)).toEqual([ROOT]);
    expect(propertiesIn(rules, DARK, ROOT)).toEqual(["--color-warn"]);

    // **囲みの中**（狭い幅）——**囲みが違えば、取れない。**
    expect(declarationIn(rules, NARROW, "form label", "grid-template-columns"))
      .toBe("1fr");
    expect(
      declarationIn(rules, DARK, "form label", "grid-template-columns"),
    ).toBeNull();
    expect(selectorsIn(rules, NARROW)).toEqual(["form label", "header"]);
    expect(declarationIn(rules, NARROW, "header", "flex-wrap")).toBe("wrap");

    // **無い綴りは `null`**（見つからないから緑、を作らない）。
    expect(declarationIn(rules, DARK, ROOT, "--color-read")).toBeNull();
    expect(selectorsIn(rules, "@media print")).toEqual([]);
  }

  it("LF で組んだソース", () => {
    check("\n");
  });

  it("CRLF で組んだソース", () => {
    check("\r\n");
  });

  /** **後の勝ち**（CSS と同じ）。**先勝ちにする変異が、ここで赤になる。** */
  it("同じ綴りが2度出てくれば、後の勝ち", () => {
    const rules = cssRules(
      [
        "@media (prefers-color-scheme: dark) {",
        "  :root { --color-warn: oklch(0.75 0.11 65); }",
        "  :root { --color-warn: oklch(0.72 0.11 250); }",
        "}",
      ].join("\n"),
    );
    expect(declarationIn(rules, DARK, ROOT, "--color-warn")).toBe(
      "oklch(0.72 0.11 250)",
    );
  });
});
