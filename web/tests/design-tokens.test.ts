/**
 * **色の綴りが1箇所にしか無い**（作業単位 D-10 / 「計器と声」）。
 *
 * # なぜ要るのか
 *
 * 着手前、**`web/src` には CSS ファイルが1つも無く、生の 16 進が 52 箇所**あった
 * （`#666` ×16 / `#b00020` ×14 / `#ccc` ×7 / …）。
 * **同じ ~8 の規則が 5〜6 のコンポーネントに写っていた**ので、
 * **片方を直しても、もう片方が黙って古びる**（CLAUDE.md）。
 *
 * # ここで見るものと、見ないもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **生の 16 進が `web/src` に1つも無いこと** | **ここ** |
 * | **生の 16 進が、画面の入口の文書にも無いこと** | **ここ**（根の外の1件） |
 * | **トークンを定義しているのが1ファイルだけであること** | **ここ** |
 * | **参照しているトークンが、全部定義されていること** | **ここ** |
 * | **土台が `main.ts` から取り込まれていること** | **ここ**（配線1行 / 下の ⚠） |
 * | **「壊れている」側が暖色、「測っている」側が寒色であること** | **ここ**（帳簿 / 下の ⚠） |
 * | **accent の使用箇所が、全部その帳簿に載っていること** | **ここ** |
 * | **どの綴りに、どの字を当てているか** | **ここ**（字の帳簿） |
 * | **外から書体・様式を取ってこないこと** | **ここ**（`<style>` と入口の文書） |
 * | **色をクラス名の中に書いていないこと**（任意値の**中身の綴り**） | **ここ**（D-11a） |
 * | **色を運ぶ接頭辞の直後に角括弧を書いていないこと**（任意値の**入れ物**） | **ここ**（B-44） |
 * | **ユーティリティを CSS へ書き戻していないこと** | **ここ**（D-11a） |
 * | **ビルドが候補を探す根が、走査の根と同じであること** | **ここ**（D-11a） |
 * | **配られた CSS に何が入ったか** | **どこにも無い**（下の ⚠） |
 * | **その色が画面でどう見えるか** | **どこにも無い。人が見るしかない** |
 *
 * # ⚠ 守れていないこと
 *
 * **色が「意味に合っているか」は、機械では判定できない。**
 * ここが守っているのは**「黙って入れ替わらないこと」**だけである ——
 * 下の帳簿は、**どの綴りがどちら側かを人が書いた表**であって、
 * **その表そのものが正しいかは誰も見ていない。**
 *
 * **⚠ 帳簿に載っていない使用箇所が在れば、網羅の検査が赤くなる**
 * （「accent を使っている箇所が、全部どちらかの側に載っている」）。
 * **D-10 のレビューの時点では、そうなっていなかった** ——
 * `.broken` と `.confirm` の2箇所が表から漏れていて、
 * **`var(--read)` へ差し替えても全緑**だった。
 * **表に載せることと、表が全部を覆っていることは、別の主張である。**
 *
 * **⚠ 字は、割り当てだけを見ている。**「字の割り当て」が
 * **落とす / 入れ替える / 生の書体名を直書きする**を赤にするが、
 * **`td[title]` という選び方が正しいか**（＝ `title` の付いた `<td>` が
 * 時刻だけであるか）は**マークアップの側なので、誰も見ていない。**
 *
 * **⚠ 外部接続を見るのは、入口の文書と `<style>` の中身だけ。**
 * `.ts` のコメントには仕様書への URL が実在する（`lib/chat.ts` / `lib/api.ts`）ので、
 * ソース全体には掛けていない。
 *
 * **⚠ 走査の根は `web/src` ＋ 入口の文書1件。**
 * **`web/` の下に3つ目の入口を作ったら、`support` の一覧へ足すまで射程の外である。**
 * **D-11a から、`src/app.css` の `@source` にも同じ2件が書いてある**
 * （＝**3つ目を作ったら、直す場所は2箇所ある**）。
 *
 * **⚠ ここは Tailwind を1度も走らせない。**
 * 見ているのは**ソースの上の綴り**だけなので、
 * **「既定のパレットが本当に生えないこと」も「配られた CSS に何が入ったか」も
 * この中では言えない。** **測るならビルドを回すしかない**
 * （D-11a の変異検査はそうした。**見張り番としては置いていない**）。
 *
 * **⚠ 色を運ぶ接頭辞の一覧は、v4 の語彙に追いつく必要がある**（B-44）。
 * **新しいユーティリティが色を運ぶようになった日、この走査は黙って素通りする** ——
 * **免除の一覧と同じ向きの穴**（CLAUDE.md / `G-3b`）で、**古びる向きが安全側ではない。**
 * **交換した相手**（角括弧の中身の綴りを数える形）**も同じ穴を持っていた**ので、
 * **どちらかを選ぶ問題ではなく、増えにくいほうを選んだ**、と読むこと。
 * **⚠ そして一覧は、書いた日から既に遅れていた**（B-44 のレビューが実測 / 2026-09-17）——
 * **枠の論理的な辺**（`border-s` / `border-e`）**が抜けていて、
 * 配布 CSS に色が載るのに 969 全緑だった。** **「将来古びる」ではなく「もう遅れていた」。**
 * **⚠ そして「色を運ばない接頭辞」は最初から射程の外である**（`grid-cols-` / `w-` …）。
 * **⚠ その通す側は、現に使われていない**（2026-09-17 に数えた ——
 * **`web/src` ＋ `index.html` に任意値のユーティリティは 0 件、`grid-cols` の綴りも 0 件。**
 * **狭い幅の格子は `app.css` の素の宣言である**）。
 * **かつてここには「`D-12b` が実際に使っている」と書いてあった。偽である。**
 * **通す側の判断（どこまで通すか）の当否は、まだ何にも測られていない。**
 *
 * **変容の「上げ」と「下げ」に至っては、色を当てる手掛かりがマークアップに無い**
 * （`data-delta` はテストが置いた印であって、読み手が見るものではない /
 * CLAUDE.md の `D-4`）。**この作業単位はマークアップを触らないので、
 * 上げ／下げの色は実装していない。守っていない、と読むこと。**
 *
 * # ⚠ 対照は、植えたソースを走査本体に通す
 *
 * 実ファイルだけを読む対照は自己成就する（CLAUDE.md / `15b` の実測）。
 * **走査本体は引数でソースを受け取る形**（`tests/support/design-tokens.ts`）なので、
 * **実ファイルと植えたソースを、同じ判定に通している。**
 *
 * **ブラウザは起動しない**（jsdom すら要らない / 既定の `node` で走る）。
 */

import { describe, expect, it } from "vitest";

import {
  ENTRY_PATH,
  arbitraryColourValues,
  colourCarryingArbitraryValues,
  cssRules,
  declarationOf,
  externalReferences,
  locatedDeclarations,
  networkFetches,
  oklchHue,
  rawHexColours,
  sourceFiles,
  styleBlocks,
  stylesheetOf,
  tokenDefinitions,
  tokenReference,
  tokenReferences,
  utilityWriteBacks,
  type LocatedDeclaration,
  type ScannedFile,
} from "./support/design-tokens";

/** 走査が読んだもの（**1回だけ読む**）。 */
const SOURCES = sourceFiles();

/** トークンを定義している唯一のファイル。 */
const TOKENS = "app.css";

/**
 * **トークンを置く唯一の場所**（Tailwind v4 の主題 / ADR-0040 §決定2）。
 *
 * **⚠ `:root` ではない。** v4 の `@theme` は
 * **「カスタムプロパティを持つ CSS」であると同時に、ユーティリティの生える元**である
 * （`--color-read` から `text-read` / `bg-read` が生える）。
 * **`:root` に書いた綴りからは、何も生えない。**
 */
const THEME = "@theme";

/**
 * `@theme` に置く綴り、全部。
 *
 * **⚠ この一覧はここにしか無い**（`app.css` の側から作らない）。
 * 作ると、**定義を消す変異で期待値も一緒に消えて緑になる**（CLAUDE.md / `16b`）。
 *
 * **⚠ D-11a で名前を機械的に変えた**（`--read` → `--color-read` /
 * `--mincho` → `--font-mincho` / ADR-0040 §決定2 の表）。
 * **値は1文字も変えていない。** 変えたのは綴りだけである。
 * **接頭辞が用途を決める**ので、`--color-` / `--font-` を外すと
 * **その綴りからユーティリティが生えなくなる**（黙って。CSS としては通る）。
 */
const TOKEN_NAMES = [
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
  "--font-mincho",
  "--font-sans",
  "--font-mono",
];

/**
 * **既定を消す綴り**（ADR-0040 §決定3）。
 *
 * **Tailwind は既定で数百色を持つ**（`text-red-500` …）。
 * **この家の決めは「accent は2つだけ」**なので、**両立しない。**
 * `--color-*: initial` と `--font-*: initial` で**既定を丸ごと消してから**、
 * 上の 15 個だけを生やす。**`text-red-500` は存在しないクラスになる。**
 *
 * **⚠ ここに載せてあるのは「消す指示が `@theme` に在ること」までである。**
 * **本当に生えないことを見ているのは、ビルドの出力であって、この検査ではない**
 * （単体テストは Tailwind を走らせない）。**そこは守れていない、と読むこと。**
 *
 * **⚠ 消すのは色と字だけ。** 間隔・字の大きさなどの既定は残す
 * （`D-11b` / `D-12` がユーティリティで使う）。
 */
const RESET_NAMES = ["--color-*", "--font-*"];

function sourceAt(path: string): ScannedFile {
  const found = SOURCES.find((entry) => entry.path === path);
  expect(found, `${path} を走査が読んでいること`).toBeDefined();
  return found as ScannedFile;
}

describe("走査が、実ファイルを本当に読んでいる", () => {
  it("`web/src` の下を、拡張子で選ばずに読んでいる", () => {
    const paths = SOURCES.map((entry) => entry.path);

    // **トークンの置き場所。**
    expect(paths).toContain(TOKENS);
    // **`.svelte` も `.ts` も読んでいる**（拡張子で選んでいないことの現れ）。
    expect(paths).toContain("App.svelte");
    expect(paths).toContain("lib/StatsPanel.svelte");
    expect(paths).toContain("main.ts");
    expect(paths).toContain("lib/statsEdit.ts");
    // **画面の入口の文書**（根の外の1件 / `support` の [`ENTRY_HTML`]）。
    expect(paths).toContain(ENTRY_PATH);
    // **一覧が痩せていない。** 空の一覧なら、この下の判定は全部素通りする。
    expect(paths.length).toBeGreaterThanOrEqual(24);
    // **中身が空でない。**
    for (const entry of SOURCES) {
      expect(entry.source.length, `${entry.path} の中身`).toBeGreaterThan(0);
    }
  });

  it("`<style>` の中身を、実ファイルから取り出せている", () => {
    const panel = sourceAt("lib/StatsPanel.svelte");
    const blocks = styleBlocks(panel.source);
    expect(blocks.length, "StatsPanel の `<style>` が1つ").toBe(1);
    // **判定に使う綴りはこの検査の中に直書き**してある ——
    // 実ファイルから作ると、取り出しが壊れても期待値が一緒に動く。
    expect(blocks[0]).toContain("tr.unallocated td");
    // **`<style>` の外は入っていない。**
    expect(blocks[0]).not.toContain("<h2>");
  });

  it("トークンを定義しているファイルから、規則を読めている", () => {
    const rules = cssRules(stylesheetOf(sourceAt(TOKENS)));
    expect(rules.length).toBeGreaterThanOrEqual(10);
    expect(rules.map((rule) => rule.selector)).toContain(THEME);
  });

  /**
   * **土台が、束ねる対象として取り込まれている。**
   *
   * **⚠ ここは「配線だけの場所」で、着手前は誰も見ていなかった**
   * （CLAUDE.md の「配線だけの場所は、層に関係なく観測対象から落ちる」/
   * `D-3` の `App.svelte` と同型）。
   * **D-10 のレビューで実測: `main.ts` からこの1行を消すと、
   * トークン 15 個・`body` の字と紙・表の罫・フォームの枠・`.error` / `.notice` /
   * `.summary` が全部消えるのに、赤は1本も出ず、`pnpm build` も通った**
   * （CSS が 3.53 kB → 1.58 kB になるだけ）。
   *
   * **⚠ `app.css` と `web/README.md` の「取り込めば失敗はビルドの失敗として出る」は、
   * ファイルが読めない場合についてのみ真である。**
   * **行そのものが消える場合を見るのは、この1本しかない。**
   */
  it("土台を、束ねる対象として取り込んでいる", () => {
    expect(sourceAt("main.ts").source).toContain('import "./app.css";');
  });

  /**
   * **ビルドが候補を探す根が、走査の根と同じ。**
   *
   * **Tailwind は「使われたクラスだけ」を出す**ので、
   * **どこを探すかが、配られる CSS の中身を決める。**
   *
   * **⚠ 既定（プロジェクト全体）だと、走査の射程の外から色が入る。**
   * **D-11a で実測** —— **下の対照が植えている `#b00020` が、
   * 配られる CSS に `.text-[#b00020] { color: #b00020 }` として入った。**
   * **`web/src` に生の 16 進が1つも無いことを守っていても、
   * 配られるものには在る**、という形になる。
   *
   * **⚠ ここが言えること**: **根が2つで、走査の根と同じ綴りであること。**
   * **⚠ ここが言えないこと**: **配られた CSS に何が入ったか。**
   * 単体テストは Tailwind を走らせない。**そこは守れていない、と読むこと。**
   */
  it("ビルドが候補を探す根が、走査の根と同じ", () => {
    const tokens = sourceAt(TOKENS).source;
    // **自動で探す形を切ってあること。** これが消えると根の宣言が効かなくなる。
    //
    // **⚠ 綴りだけを探さない**（`m8` の変異検査で分かった）——
    // **`app.css` の注記の中に同じ綴りが在る**ので、
    // **`toContain("source(none)")` だけだと、取り込みの行から消えても通る。**
    // **取り込みの行そのものを見ること。**
    expect(tokens).toContain(
      '@import "tailwindcss/utilities.css" layer(utilities) source(none);',
    );
    // **根は、走査と同じ2つ**（`web/src` と、画面の入口の文書）。
    // **一覧はここに直書き**（`app.css` から作らない / CLAUDE.md の `16b`）。
    const declared = [...tokens.matchAll(/@source\s+"([^"]+)"/g)].map(
      (found) => found[1],
    );
    expect(declared).toEqual(["../index.html", "."]);
  });

  /** **入口の文書を、本当に読めている。** */
  it("画面の入口の文書を読んでいる", () => {
    const entry = sourceAt(ENTRY_PATH);
    // **判定に使う綴りはこの検査の中に直書き**してある。
    expect(entry.source).toContain('<div id="app"></div>');
    expect(entry.source).toContain('src="/src/main.ts"');
  });
});

describe("生の 16 進の色が、走査の射程に1つも無い", () => {
  // **`web/src` の全ファイル ＋ 画面の入口の文書**（`support` の [`ENTRY_HTML`]）。
  it("全ファイルと、入口の文書", () => {
    const offences: string[] = [];
    for (const entry of SOURCES) {
      for (const found of rawHexColours(entry.source)) {
        offences.push(`${entry.path}:${found.line}: ${found.hex}`);
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });
});

/**
 * **色を、クラス名の中に書いていない**（ADR-0040 §決定4 / 作業単位 D-11a / B-44）。
 *
 * **既定のパレットは `@theme` で消してある**ので `text-red-500` は存在しない。
 * **任意値**（`text-[#b00020]`）**はその外から入る** ——
 * **クラス名に色を直接書けるので、「色は1箇所」という前提が消える。**
 *
 * # 見る目が2つある（**どちらも要る**）
 *
 * | 見る目 | 何を見るか | これだけだと |
 * |---|---|---|
 * | **角括弧の中身の綴り**（`arbitraryColourValues`） | `[#` / `[oklch(` / `[rgb(` / `[hsl(` | **一覧の外の書き方が素通りする**（色名 / `lab()` / `color-mix()`） |
 * | **色を運ぶ入れ物**（`colourCarryingArbitraryValues`） | 接頭辞の直後の `[` | **接頭辞を持たない綴りが素通りする** |
 *
 * **⚠ 中身を見る側は B-44 で残した。** 接頭辞の一覧は v4 の語彙に追いつく必要があるが、
 * **`[#` の形は接頭辞に依らない**ので、**一覧が古びても 16 進だけは止まる。**
 * **⚠ 逆に、入れ物を見る側が無いと、色の書き方が増えた日に黙って通る** ——
 * **D-11a のレビューが実測**（色名の任意値を植えて 789 全緑、配布 CSS に生えた）。
 *
 * **見ていないものは `support` の ⚠ に並べてある。**
 */
describe("色を、クラス名の中に書いていない", () => {
  it("全ファイルと、入口の文書", () => {
    const offences: string[] = [];
    for (const entry of SOURCES) {
      for (const found of arbitraryColourValues(entry.source)) {
        offences.push(`${entry.path}:${found.line}: ${found.text}`);
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });

  /**
   * **色を運ぶ入れ物の側**（作業単位 B-44）。
   *
   * **⚠ 上の1本では届かない。** あちらは**角括弧の中身の綴り**を見るので、
   * **色名も `lab()` も `color-mix()` も、影の値の途中に埋めた色も素通りする。**
   */
  it("色を運ぶ接頭辞の直後に、角括弧を書いていない", () => {
    const offences: string[] = [];
    for (const entry of SOURCES) {
      for (const found of colourCarryingArbitraryValues(entry.source)) {
        offences.push(`${entry.path}:${found.line}: ${found.text}`);
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });
});

/**
 * **ユーティリティを CSS 側へ書き戻していない**（ADR-0040 §決定4）。
 *
 * **`D-10` が消したのは「同じ規則が 5〜6 の部品に写っている」形**である。
 * **書き戻す指示子は、それを CSS の側に作り直す。** v4 自身も勧めていない。
 *
 * **⚠ `app.css` も射程に入っている**（走査は `web/src` の下を拡張子で選ばない）。
 */
describe("ユーティリティを CSS へ書き戻していない", () => {
  it("全ファイルと、入口の文書", () => {
    const offences: string[] = [];
    for (const entry of SOURCES) {
      for (const found of utilityWriteBacks(entry.source)) {
        offences.push(`${entry.path}:${found.line}: ${found.text}`);
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });
});

describe("トークンの定義が1箇所にしか無い", () => {
  it("定義しているのは1ファイルだけ", () => {
    const definers = SOURCES.filter(
      (entry) => tokenDefinitions(stylesheetOf(entry)).length > 0,
    ).map((entry) => entry.path);
    expect(definers).toEqual([TOKENS]);
  });

  it("`@theme` が、決めた綴りを全部持っている", () => {
    const rules = cssRules(stylesheetOf(sourceAt(TOKENS)));
    const theme = rules.filter(
      (rule) => rule.within === null && rule.selector === THEME,
    );
    expect(theme.length, "`@theme` は1つ").toBe(1);

    const defined = (
      theme[0] as { declarations: readonly { property: string }[] }
    ).declarations.map((declaration) => declaration.property);
    // **既定を消す綴りも、ここに並ぶ**（[`RESET_NAMES`] の ⚠）——
    // **落とすと、この1本が赤になる。**
    expect([...defined].sort()).toEqual(
      [...RESET_NAMES, ...TOKEN_NAMES].sort(),
    );
    // **⚠ 消す2行は、定義より先に在ること**（`app.css` の「先に書くこと」）。
    // **上の集合比較は位置を見ない** —— D-11a のレビューが実測: 2行を `@theme` の末尾へ
    // 移すと **789 全緑・ビルドも通り、配布 CSS からトークン 15 個が丸ごと消える**
    // （紙・墨・罫・accent・字が全滅した画面が緑で出荷される）。
    // `parseDeclarations` はソース順を保つので、先頭がその2行であることを固定する。
    expect(defined.slice(0, RESET_NAMES.length)).toEqual(RESET_NAMES);
  });

  it("参照しているトークンは、全部定義されている", () => {
    const defined = new Set(tokenDefinitions(stylesheetOf(sourceAt(TOKENS))));
    const offences: string[] = [];
    for (const entry of SOURCES) {
      for (const name of tokenReferences(stylesheetOf(entry))) {
        if (!defined.has(name)) {
          offences.push(`${entry.path}: ${name}`);
        }
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });

  /**
   * **字は、ネットワークから取らない。**
   *
   * personacore は `127.0.0.1` にしか待ち受けない、手元で完結する道具である。
   * **外部の書体を取りに行くと、画面が外への接続に依存する。**
   *
   * **⚠ D-11a で、判定が「取り込みを一律に咎める」から
   * 「外の宛先を咎める」へ変わった** —— **Tailwind の取り込みが `@import` だから**
   * （ADR-0040 §決定1）。**何を許したかは `support` の [`networkFetches`] の ⚠。**
   * **パッケージ名なら何でも許すので、そこは守れていない、と読むこと。**
   */
  it("外から書体を取ってこない", () => {
    const offences: string[] = [];
    for (const entry of SOURCES) {
      for (const found of networkFetches(stylesheetOf(entry))) {
        offences.push(`${entry.path}: ${found}`);
      }
    }
    expect(offences, offences.join(" / ")).toEqual([]);
  });

  /**
   * **入口の文書も、外へ取りに行かない。**
   *
   * **⚠ 上の1本では届かない。** あちらが読むのは `<style>` の中身だけで、
   * **`<link rel="stylesheet" href="https://…">` はその外に在る。**
   * D-10 のレビューで実測 —— **根が `web/src` だけだった頃は、
   * `index.html` に生の 16 進を書いても全緑**で、
   * **外の書体を `<link>` で引く形は原理的に届かなかった。**
   *
   * **⚠ 掛ける先は入口の文書だけである。**
   * `.ts` のコメントには仕様書への URL が実在する（`lib/chat.ts` / `lib/api.ts`）ので、
   * ソース全体には掛けられない。**そこは守れていない、と読むこと。**
   */
  it("入口の文書が、外から何も取ってこない", () => {
    const entry = sourceAt(ENTRY_PATH);
    expect(externalReferences(entry.source)).toEqual([]);
  });
});

/**
 * **意味の帳簿**（「計器と声」の accent は2つだけ）。
 *
 * * **`--read`** = 測っている側（測定値・リンク・受信中）。**寒色。**
 * * **`--warn`** = 壊れている側（注意・失敗・上流が答えない）。**暖色。**
 *
 * **⚠ ここが言えること**: この表に載せた綴りが、**載せた側の色相を指していること。**
 * **⚠ ここが言えないこと**: **表そのものが正しいかどうか。**
 * 色の意味は機械では判定できない（モジュール doc の「守れていないこと」）。
 *
 * **暖色と寒色の境目は、この検査が持っている**（`app.css` から作らない）——
 * **`@theme` の値を入れ替える変異も、使う側を入れ替える変異も、
 * どちらも赤にするため。**
 */
const WARM = { from: 20, to: 140 };
const COOL = { from: 180, to: 320 };

/** その綴りの色相（`app.css` の `@theme` を引く）。 */
function hueOfToken(name: string): number {
  const rules = cssRules(stylesheetOf(sourceAt(TOKENS)));
  const value = declarationOf(rules, THEME, name);
  expect(value, `${name} が \`@theme\` に在ること`).not.toBeNull();
  const hue = oklchHue(value as string);
  expect(hue, `${name} の値が oklch であること`).not.toBeNull();
  return hue as number;
}

/** accent の綴り（**色として意味を持つ4つ**）。 */
const ACCENT_TOKENS = new Set([
  "--color-read",
  "--color-read-bg",
  "--color-warn",
  "--color-warn-bg",
]);

/**
 * 帳簿の鍵。
 *
 * **`@` に囲まれていれば、その名前も入る** ——
 * 暗い配色（`D-11`）が `@media` の中で色を入れ替えたら、
 * **それは帳簿に無い鍵**になるので、下の網羅の検査が赤くなる。
 */
function keyOf(where: LocatedDeclaration): string {
  const within = where.within === null ? "" : `${where.within} `;
  return `${where.path}: ${within}${where.selector} { ${where.property} }`;
}

/** 実ファイルに在る宣言、全部（在り処つき）。 */
const DECLARATIONS = SOURCES.flatMap((entry) => locatedDeclarations(entry));

/** accent を使っている箇所（**実ファイルから集めたもの**）。 */
const ACCENT_USES = DECLARATIONS.flatMap((where) =>
  tokenReferences(where.value)
    .filter((name) => ACCENT_TOKENS.has(name))
    .map((token) => ({ key: keyOf(where), token })),
);

/** その使用箇所が指している綴り。 */
function accentAt(key: string): string {
  const found = ACCENT_USES.filter((use) => use.key === key);
  expect(found.length, `${key} が accent をちょうど1つ使っていること`).toBe(1);
  return (found[0] as { token: string }).token;
}

describe("壊れている側は暖色、測っている側は寒色", () => {
  /** **壊れている・注意**（`--warn` 系を指すべき綴り）。 */
  const WARNING_SIDE = [
    "app.css: .error { color }",
    "app.css: .counter.over { color }",
    "lib/CharacterList.svelte: .confirm { border }",
    "lib/CharacterRepairForm.svelte: form { border-color }",
    "lib/CharacterRepairForm.svelte: .broken { color }",
    "lib/StatsRepairForm.svelte: form { border-color }",
    "lib/StatsPanel.svelte: tr.unallocated td { background }",
    // **外した属性の行**（作業単位 D-12a）——保存すると消える行なので、
    // **良い報せではない**（`tr.unallocated` と同じ立場）。
    "lib/AttributesPanel.svelte: tr.removed td { background }",
  ];

  /** **測っている・報せ・リンク**（`--read` 系を指すべき綴り）。 */
  const READING_SIDE = [
    "app.css: a { color }",
    "app.css: .notice { color }",
    "app.css: .summary { background }",
    // **共通の部品**（作業単位 D-12b-1 / 利用者が承認した絵）—— **操作できるものの印は read。**
    // **壊れている側（warn）は使わない**: 主ボタンもフォーカスの輪も「ここを押せる・ここにいる」であって、
    // 注意ではない。
    "app.css: :focus-visible { outline }",
    'app.css: input[type="checkbox"], input[type="radio"] { accent-color }',
    "app.css: .primary { background }",
    "app.css: .primary { border-color }",
  ];

  it.each(WARNING_SIDE)("`%s` は暖色", (key) => {
    const hue = hueOfToken(accentAt(key));
    expect(hue).toBeGreaterThanOrEqual(WARM.from);
    expect(hue).toBeLessThan(WARM.to);
  });

  it.each(READING_SIDE)("`%s` は寒色", (key) => {
    const hue = hueOfToken(accentAt(key));
    expect(hue).toBeGreaterThanOrEqual(COOL.from);
    expect(hue).toBeLessThan(COOL.to);
  });

  /**
   * **帳簿が、accent の使用箇所を全部持っていること。**
   *
   * **⚠ これが無いと、表に載っていない1箇所は入れ替えても止まらない。**
   * D-10 のレビューで実測 —— **`CharacterRepairForm.svelte` の `.broken` を
   * `var(--read)` へ差し替えても全緑**だった（表に無かったので）。
   * **`CharacterList.svelte` の `.confirm` も同じく無かった** ——
   * こちらは `border: 1px solid var(--warn)` の**短縮形**なので、
   * `var()` 単体しか読まない形では拾えてもいなかった。
   *
   * **⚠ 向きに注意**: 表は**人が書いたもの**で、実ファイルからは作らない。
   * 作ると、使用箇所を消す変異で期待値も一緒に消えて緑になる（CLAUDE.md / `16b`）。
   */
  it("accent を使っている箇所が、全部どちらかの側に載っている", () => {
    expect([...new Set(ACCENT_USES.map((use) => use.key))].sort()).toEqual(
      [...WARNING_SIDE, ...READING_SIDE].sort(),
    );
  });

  /**
   * **accent は2つだけ**（`--color-read` と `--color-warn`。
   * **値をどうそろえてあるかは `src/app.css` の `@theme` の隣に書いてある**）。
   * **3つ目の色相を足したら、それは新しい accent である。**
   */
  it("暖色と寒色は、重ならない", () => {
    expect(WARM.to).toBeLessThanOrEqual(COOL.from);
    expect(hueOfToken("--color-warn")).not.toBe(hueOfToken("--color-read"));
  });
});

/**
 * **字の帳簿**（「計器と声」 —— 人格の側は明朝、識別子と数は等幅、残りはゴシック）。
 *
 * **⚠ ここが言えること**: **どの綴りに、どの字を当ててあるか。**
 * 割り当てを**落とす**変異も、**明朝とゴシックを入れ替える**変異も、
 * **生の書体名を直書きする**変異も、この1本が赤にする
 * （D-10 のレビューで実測: 着手前は `td[title]` も `h1` も `body` も
 * **落として全緑**だった）。
 *
 * **⚠ ここが言えないこと**: **`td[title]` という選び方が正しいかどうか。**
 * この家では**生の綴りを `title` に残す**と決めてあり（作業単位 F-7）、
 * **いま `title` の付いた `<td>` が4つとも時刻であること**に寄りかかっている。
 * **マークアップの側は誰も見ていない。守れていない、と読むこと。**
 */
describe("字の割り当て", () => {
  /** どの綴りに、どの字を当てているか（**人が書いた表**）。 */
  const TYPEFACE: [string, string][] = [
    ["app.css: body { font-family }", "--font-sans"],
    ["app.css: h1 { font-family }", "--font-mincho"],
    ["app.css: code { font-family }", "--font-mono"],
    ["app.css: td[title] { font-family }", "--font-mono"],
    ["app.css: .counter { font-family }", "--font-mono"],
    ["lib/ChatView.svelte: .dialogue { font-family }", "--font-mincho"],
  ];

  it("`font-family` は、決めた表のとおりに割り当ててある", () => {
    const actual = DECLARATIONS.filter(
      (where) => where.property === "font-family",
    ).map((where) => `${keyOf(where)} = ${tokenReference(where.value)}`);
    expect([...actual].sort()).toEqual(
      TYPEFACE.map(([key, token]) => `${key} = ${token}`).sort(),
    );
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
  it("生の 16 進は、桁の数に関係なく見つかる", () => {
    const planted = [
      ".a { color: #666; }",
      ".b { color: #abcd; }",
      ".c { border: 1px solid #b00020; }",
      ".d { background: #12345678; }",
    ].join("\n");
    expect(rawHexColours(planted).map((found) => found.hex)).toEqual([
      "#666",
      "#abcd",
      "#b00020",
      "#12345678",
    ]);
  });

  /**
   * **⚠ 大文字の腕。**
   *
   * D-10 のレビューで実測 —— **植えた綴りが4件とも小文字だったので、
   * 走査の `[0-9a-fA-F]` を3箇所とも `[0-9a-f]` へ狭める変異が全緑**だった。
   * **狭めたあと、大文字の 16 進は黙って流れ込む**
   * （CLAUDE.md の**第5の分類** —— 2つの変異が互いを隠し合う）。
   *
   * **桁の3通りを、全部大文字を含む綴りで置いてある** ——
   * 3〜4桁 / 6桁 / 8桁のどれを狭めても、この1本が赤になる。
   */
  it("大文字の 16 進も、桁の数に関係なく見つかる", () => {
    const planted = [
      ".a { color: #FFF; }",
      ".b { color: #ABCD; }",
      ".c { border: 1px solid #B00020; }",
      ".d { background: #DEADBEEF; }",
    ].join("\n");
    expect(rawHexColours(planted).map((found) => found.hex)).toEqual([
      "#FFF",
      "#ABCD",
      "#B00020",
      "#DEADBEEF",
    ]);
  });

  it("行番号は、その綴りが在る行を指す", () => {
    const planted = ["/* 1行目 */", ".a {", "  color: #666;", "}"].join("\n");
    expect(rawHexColours(planted)).toEqual([{ hex: "#666", line: 3 }]);
  });

  /**
   * **⚠ CRLF の腕**（CLAUDE.md / `E-1`）。
   *
   * **作業ツリーの `.svelte` は CRLF**（`core.autocrlf` の checkout 由来）で、
   * **この関数は自分で位置から行を数えている。**
   * **LF で書いた対照だけだと、実ファイルより弱い対照になる。**
   */
  it("CRLF でも、同じ行番号を返す", () => {
    const lines = ["/* 1行目 */", ".a {", "  color: #666;", "}"];
    expect(rawHexColours(lines.join("\r\n"))).toEqual([
      { hex: "#666", line: 3 },
    ]);
    expect(rawHexColours(lines.join("\r\n"))).toEqual(
      rawHexColours(lines.join("\n")),
    );
  });

  it("Svelte の波括弧を、色と読まない", () => {
    const planted = [
      "{#each list as item (item.id)}",
      "{#if ready}",
      "{#key handle}",
      '<div id="app"></div>',
      "<a href=\"#/chat\">会話</a>",
    ].join("\n");
    expect(rawHexColours(planted)).toEqual([]);
  });

  it("トークンの参照は、色と読まない", () => {
    expect(rawHexColours(".a { color: var(--warn); }")).toEqual([]);
  });

  it("対照: 色が1つも無いソースからは、何も出ない", () => {
    expect(rawHexColours(".a { color: var(--ink); }")).toEqual([]);
  });

  it("`<style>` の中身だけを取り出す", () => {
    const planted = [
      "<script lang=\"ts\">",
      "  const hint = 1;",
      "</script>",
      "<p>本文</p>",
      "<style>",
      "  .hint { color: var(--muted); }",
      "</style>",
    ].join("\n");
    expect(styleBlocks(planted)).toEqual([
      "\n  .hint { color: var(--muted); }\n",
    ]);
    expect(
      stylesheetOf({ path: "lib/X.svelte", source: planted }),
    ).toContain(".hint");
    // **`<style>` が無ければ、空。**
    expect(styleBlocks("<p>本文</p>")).toEqual([]);
    // **`.ts` はスタイルシートではない。**
    expect(
      stylesheetOf({ path: "lib/x.ts", source: ".a { color: red; }" }),
    ).toBe("");
  });

  it("規則と宣言に分けられる", () => {
    const planted = [
      "/* コメントは落ちる: .ghost { color: var(--warn); } */",
      ":root {",
      "  --warn: oklch(0.56 0.11 65);",
      "}",
      ".error,",
      ".broken {",
      "  color: var(--warn);",
      "  font-weight: 700;",
      "}",
    ].join("\n");
    const rules = cssRules(planted);
    expect(rules.map((rule) => rule.selector)).toEqual([
      ":root",
      ".error, .broken",
    ]);
    expect(rules.map((rule) => rule.within)).toEqual([null, null]);
    expect(declarationOf(rules, ":root", "--warn")).toBe("oklch(0.56 0.11 65)");
    expect(declarationOf(rules, ".error, .broken", "font-weight")).toBe("700");
    // **コメントの中の規則は、規則ではない。**
    expect(declarationOf(rules, ".ghost", "color")).toBeNull();
    // **無い宣言は `null`。**
    expect(declarationOf(rules, ":root", "--read")).toBeNull();
  });

  /**
   * **`@media` の中の規則は、素の規則と混ざらない。**
   *
   * **暗い配色は作業単位 `D-11` でここへ入る。**
   * 混ざると、**帳簿がどちらの値を見ているのか分からなくなる。**
   */
  it("`@media` の中の規則には、囲みの名前が付く", () => {
    const planted = [
      ":root { --warn: oklch(0.56 0.11 65); }",
      "@media (prefers-color-scheme: dark) {",
      "  :root { --warn: oklch(0.72 0.11 65); }",
      "}",
    ].join("\n");
    const rules = cssRules(planted);
    expect(rules.map((rule) => rule.within)).toEqual([
      null,
      "@media (prefers-color-scheme: dark)",
    ]);
    // **帳簿が見るのは素の側だけ。**
    expect(declarationOf(rules, ":root", "--warn")).toBe("oklch(0.56 0.11 65)");
  });

  /**
   * **`@` が宣言を直に持つ形**（`@theme` / ADR-0040 §決定2）。
   *
   * **⚠ `@theme` の中身は「規則」ではない。** 宣言が直に並んでいる。
   * **拾わないと、トークンの定義が丸ごと帳簿から消える**
   * ——`:root` の頃は素の規則だったので、ここは D-11a で新しく要った経路である。
   */
  it("`@theme` の中の宣言を、規則として拾う", () => {
    const planted = [
      "@theme {",
      "  --color-*: initial;",
      "  --color-warn: oklch(0.56 0.11 65);",
      "}",
    ].join("\n");
    const rules = cssRules(planted);
    expect(rules.map((rule) => rule.selector)).toEqual(["@theme"]);
    // **囲みは無い**（`@theme` 自身が囲みなのではなく、置き場所である）。
    expect(rules.map((rule) => rule.within)).toEqual([null]);
    expect(declarationOf(rules, "@theme", "--color-warn")).toBe(
      "oklch(0.56 0.11 65)",
    );
    // **既定を消す綴りも、宣言として出る**（落とすと帳簿が赤くなる側）。
    expect(declarationOf(rules, "@theme", "--color-*")).toBe("initial");
    expect(tokenDefinitions(planted)).toEqual(["--color-*", "--color-warn"]);
  });

  /**
   * **⚠ 逆向き —— 規則を持つ `@` は、宣言を持たない。**
   *
   * **これが無いと、「`@` の中身は全部宣言」と読む実装が緑を通る**
   * （`@media` の中の `:root { … }` が、囲みの名前を持った偽の宣言になる）。
   * **暗い配色（`D-11b`）はそこへ入るので、実害はその日に出る。**
   */
  it("規則を持つ `@` は、宣言を持たない", () => {
    const planted = [
      "@media (prefers-color-scheme: dark) {",
      "  :root { --color-warn: oklch(0.72 0.11 65); }",
      "}",
    ].join("\n");
    const rules = cssRules(planted);
    expect(rules.map((rule) => rule.selector)).toEqual([":root"]);
    expect(rules.map((rule) => rule.within)).toEqual([
      "@media (prefers-color-scheme: dark)",
    ]);
  });

  /**
   * **ブロックを持たない `@` は、次の規則を飲み込まない。**
   *
   * **変異検査で実測して直した**（`m9`）——
   * 直す前は `@import` の1行が `:root` を丸ごと飲み、
   * **帳簿の 13 本が「`:root` にトークンが無い」と言い出した。**
   * **止まりはするが、名指しする名前が違う**（CLAUDE.md / `B-38`）。
   */
  it("ブロックを持たない `@` の次の規則が、消えない", () => {
    const planted = [
      '@charset "utf-8";',
      ":root {",
      "  --warn: oklch(0.56 0.11 65);",
      "}",
    ].join("\n");
    const rules = cssRules(planted);
    expect(rules.map((rule) => rule.selector)).toEqual([":root"]);
    expect(declarationOf(rules, ":root", "--warn")).toBe("oklch(0.56 0.11 65)");
  });

  /**
   * **⚠ 宣言を2つ以上持つ規則を囲む `@` も、宣言を持たない**（作業単位 D-12b-1）。
   *
   * **上の「規則を持つ `@` は、宣言を持たない」は、中の規則が宣言を1つしか持たない形だった。**
   * **2つ目からは `;` で切った断片が `{` を含まない**ので、
   * **着手前の実装はそれを `@` 自身の宣言として拾い、囲みの名前を選択子にした偽の規則を1件足していた**
   * （実ファイルでは暗い配色の `:root` が 11 個の偽の宣言を生んでいた）。
   * **どの帳簿もその偽の規則の鍵を使っていなかったので、表に出なかっただけである。**
   * **層の外に在る規則を数える見張り番（`tests/base-layers.test.ts`）は、それを数えてしまう。**
   */
  it("宣言を2つ持つ規則を囲む `@` も、宣言を持たない", () => {
    const planted = [
      "@media (prefers-color-scheme: dark) {",
      "  :root {",
      "    --color-warn: oklch(0.72 0.11 65);",
      "    --color-read: oklch(0.72 0.11 250);",
      "  }",
      "}",
    ].join("\n");
    const rules = cssRules(planted);
    expect(rules.map((rule) => rule.selector)).toEqual([":root"]);
    expect(rules.map((rule) => rule.within)).toEqual([
      "@media (prefers-color-scheme: dark)",
    ]);
  });

  /**
   * **`@layer` の中の規則は、囲みではなく層として読む**（作業単位 D-12b-1 / ADR-0040 の追記）。
   *
   * **⚠ 囲み（`within`）に入れない理由**: 層は「どの条件で効くか」ではなく
   * **「どの順で勝つか」**しか変えない。**`within` に入れると、帳簿の鍵
   * （`app.css: .error { color }`）と `declarationOf` が、層へ移しただけで全部外れる** ——
   * **見張り番の意味は変わっていないのに、名前だけが変わる。**
   * **だから層は別の欄（`layer`）に置き、囲みは層を素通りして内側の `@media` を名乗る。**
   */
  function checkLayers(newline: string): void {
    const planted = [
      "@layer theme, base, components, utilities;",
      "@layer base {",
      "  body {",
      "    margin: 0;",
      "    color: var(--color-ink);",
      "  }",
      "  @media (max-width: 40rem) {",
      "    nav {",
      "      flex-wrap: wrap;",
      "    }",
      "  }",
      "}",
      "@layer components {",
      "  .error {",
      "    color: var(--color-warn);",
      "  }",
      "}",
      ":root {",
      "  color-scheme: light dark;",
      "}",
    ].join(newline);
    const rules = cssRules(planted);
    // **層の宣言そのもの（ブロックを持たない `@layer`）は、規則ではない。**
    expect(
      rules.map((rule) => [rule.within, rule.layer, rule.selector]),
    ).toEqual([
      [null, "base", "body"],
      ["@media (max-width: 40rem)", "base", "nav"],
      [null, "components", ".error"],
      [null, null, ":root"],
    ]);
    // **層に入れても、素の引き方で取れる**（鍵が変わらない）。
    expect(declarationOf(rules, "body", "color")).toBe("var(--color-ink)");
    expect(declarationOf(rules, ".error", "color")).toBe("var(--color-warn)");
    // **層の中の `@media` は、囲みの外の引き方では取れない**（狭い幅は狭い幅のまま）。
    expect(declarationOf(rules, "nav", "flex-wrap")).toBeNull();
  }

  it("`@layer` の中の規則は、囲みではなく層として読む（LF）", () => {
    checkLayers("\n");
  });

  it("`@layer` の中の規則は、囲みではなく層として読む（CRLF）", () => {
    checkLayers("\r\n");
  });

  /**
   * **宣言を、在り処つきで拾う**（帳簿の網羅は、これを数えている）。
   *
   * **⚠ 短縮形も1件として出る。** `var()` 単体の形しか読まない作りだと、
   * `border: 1px solid var(--warn)` の1箇所が**帳簿から静かに落ちる**
   * （D-10 のレビューで実測: `CharacterList.svelte` の `.confirm`）。
   *
   * **⚠ `@` の中の規則も出る。** 落とすと、暗い配色（`D-11`）が
   * `@media` の中で色を入れ替えても、網羅の検査が気づかない。
   */
  it("宣言を、在り処つきで拾える", () => {
    const planted = [
      "<style>",
      "  .confirm { border: 1px solid var(--warn); }",
      "  @media (prefers-color-scheme: dark) {",
      "    .error { color: var(--read); }",
      "  }",
      "</style>",
    ].join("\n");
    expect(
      locatedDeclarations({ path: "lib/X.svelte", source: planted }),
    ).toEqual([
      {
        path: "lib/X.svelte",
        within: null,
        selector: ".confirm",
        property: "border",
        value: "1px solid var(--warn)",
      },
      {
        path: "lib/X.svelte",
        within: "@media (prefers-color-scheme: dark)",
        selector: ".error",
        property: "color",
        value: "var(--read)",
      },
    ]);
    // **スタイルシートを持たないファイルからは、何も出ない。**
    expect(
      locatedDeclarations({ path: "lib/x.ts", source: ".a { color: red; }" }),
    ).toEqual([]);
  });

  /**
   * **入口の文書の `<style>` も、スタイルシートとして読む。**
   *
   * **`vite build` は `index.html` を `dist/index.html` へ写す**ので、
   * ここに書いた規則は画面に効く。
   */
  it("`.html` の `<style>` も、スタイルシートとして読む", () => {
    expect(
      stylesheetOf({
        path: ENTRY_PATH,
        source: "<style>.a { color: #B00020; }</style>",
      }),
    ).toContain("#B00020");
  });

  /**
   * **外を指している綴りと、手元の綴りを見分ける。**
   *
   * **ホストが点を持つものだけを外と見る**ので、
   * 根からの絶対パス（`/src/main.ts`）にも
   * ハッシュのルータ（`#/chat`）にも当たらない。
   */
  it("外を指す綴りと、手元の綴りを見分ける", () => {
    const planted = [
      '<link rel="stylesheet" href="https://fonts.example.com/x.css" />',
      '<link rel="stylesheet" href="//fonts.example.com/y.css" />',
      '<script type="module" src="/src/main.ts"></script>',
      '<a href="#/chat">会話</a>',
      '<div id="app"></div>',
    ].join("\n");
    expect(externalReferences(planted)).toEqual([
      "https://fonts.example.com",
      "//fonts.example.com",
    ]);
    // **手元だけを指すソースからは、何も出ない。**
    expect(
      externalReferences('<script type="module" src="/src/main.ts"></script>'),
    ).toEqual([]);
  });

  /**
   * **任意値で書いた色**（ADR-0040 §決定4 / 作業単位 D-11a）。
   *
   * **4つの綴りを別々の行に置いてある** ——
   * **どれか1つを走査から落とす変異が、この1本で赤になる。**
   */
  it("任意値で書いた色を見つける", () => {
    const planted = [
      '<span class="text-[#b00020]">壊れている</span>',
      '<span class="bg-[oklch(0.96_0.035_75)]">紙</span>',
      '<span class="text-[rgb(0_0_0)]">墨</span>',
      '<span class="text-[hsl(0_0%_0%)]">墨</span>',
    ].join("\n");
    expect(arbitraryColourValues(planted)).toEqual([
      { text: "[#", line: 1 },
      { text: "[oklch(", line: 2 },
      { text: "[rgb(", line: 3 },
      { text: "[hsl(", line: 4 },
    ]);
  });

  /**
   * **⚠ 角括弧そのものは咎めない。**
   *
   * `list[0]` も `grid-cols-[10rem_1fr]` も**この家では正当**である
   * （ADR-0040 §決定4 は**色**の任意値だけを止めている）。
   * **これが無いと、走査を `/\[/` へ広げる変異が「厳しくなっただけ」に見えて通る。**
   */
  it("色でない角括弧は、咎めない", () => {
    const planted = [
      "const first = list[0];",
      '<td class="grid-cols-[10rem_1fr]">',
      "items[index]",
      '<span class="text-warn">',
    ].join("\n");
    expect(arbitraryColourValues(planted)).toEqual([]);
  });

  /**
   * **⚠ CRLF の腕**（CLAUDE.md / `E-1`）——
   * **行番号を自分で数えている関数には要る。**
   */
  it("CRLF でも、任意値の行番号は同じ", () => {
    const lines = ["/* 1行目 */", "<span", '  class="text-[#b00020]"', ">"];
    expect(arbitraryColourValues(lines.join("\r\n"))).toEqual([
      { text: "[#", line: 3 },
    ]);
    expect(arbitraryColourValues(lines.join("\r\n"))).toEqual(
      arbitraryColourValues(lines.join("\n")),
    );
  });

  /**
   * **色を運ぶ入れ物の側**（作業単位 B-44）。
   *
   * **⚠ 接頭辞を1つずつ別の行に置いてある。**
   * **一覧から1つ落とす変異が、この1本で赤になる** ——
   * **`border-t-` を落としても `border-` では拾えない**（直後が `[` でないので当たらない）し、
   * **`t` は色を運ばないので一覧に無い。`ring-offset-` の `offset` も同じ。**
   * **つまりこの 25 個は、1つずつが単独で要る。**
   *
   * **⚠ 逆に「尻尾がこの一覧に在る長い綴り」は、ここには無い**
   * （`inset-shadow-` など）——**載せると死んだ項目になる**ので、
   * **下の「接頭辞で終わる長い綴りも、止まる」が別に見ている。**
   *
   * **⚠ 中身の綴りは、わざとばらばらにしてある** ——
   * **色名 / `lab()` / `lch()` / `hwb()` / `color-mix()` / 16 進 / `oklch()` /
   * `rgb()` / `hsl()`、そして影のように値の途中に色が埋まる形。**
   * **この側は中身を読まない**ので、**どれも同じに止まる。**
   * **中身の綴りを見る側**（[`arbitraryColourValues`]）**が素通りするのは、この一覧の前半である。**
   */
  it("色を運ぶ接頭辞は、1つ残らず止まる", () => {
    const planted = [
      '<span class="text-[red]">壊れている</span>',
      '<span class="bg-[lab(50%_0_0)]">紙</span>',
      '<div class="border-[color-mix(in_oklch,_red,_blue)]">',
      '<div class="border-t-[hwb(0_0%_0%)]">',
      '<div class="border-r-[lch(50%_0_0)]">',
      '<div class="border-b-[#b00020]">',
      '<div class="border-l-[oklch(0.5_0.11_250)]">',
      '<div class="border-x-[rgb(0_0_0)]">',
      '<div class="border-y-[hsl(0_0%_0%)]">',
      '<div class="border-s-[red]">',
      '<div class="border-e-[oklch(0.5_0.11_250)]">',
      '<button class="outline-[red]">',
      '<button class="ring-[red]">',
      '<button class="ring-offset-[red]">',
      '<svg class="fill-[red]">',
      '<svg class="stroke-[red]">',
      '<div class="shadow-[0_0_4px_lab(50%_0_0)]">',
      '<a class="decoration-[red]">',
      '<input class="accent-[red]" />',
      '<input class="caret-[red]" />',
      '<ul class="divide-[red]">',
      '<input class="placeholder-[red]" />',
      '<div class="from-[red]">',
      '<div class="via-[red]">',
      '<div class="to-[red]">',
    ].join("\n");
    expect(colourCarryingArbitraryValues(planted)).toEqual([
      { text: "text-[", line: 1 },
      { text: "bg-[", line: 2 },
      { text: "border-[", line: 3 },
      { text: "border-t-[", line: 4 },
      { text: "border-r-[", line: 5 },
      { text: "border-b-[", line: 6 },
      { text: "border-l-[", line: 7 },
      { text: "border-x-[", line: 8 },
      { text: "border-y-[", line: 9 },
      { text: "border-s-[", line: 10 },
      { text: "border-e-[", line: 11 },
      { text: "outline-[", line: 12 },
      { text: "ring-[", line: 13 },
      { text: "ring-offset-[", line: 14 },
      { text: "fill-[", line: 15 },
      { text: "stroke-[", line: 16 },
      { text: "shadow-[", line: 17 },
      { text: "decoration-[", line: 18 },
      { text: "accent-[", line: 19 },
      { text: "caret-[", line: 20 },
      { text: "divide-[", line: 21 },
      { text: "placeholder-[", line: 22 },
      { text: "from-[", line: 23 },
      { text: "via-[", line: 24 },
      { text: "to-[", line: 25 },
    ]);
  });

  /**
   * **⚠ 尻尾が一覧に在れば、前置きが付いていても止まる**（作業単位 B-44）。
   *
   * **v4 は色を運ぶ綴りを前置きで増やす。** ここに置いた4つは**全部 v4 の実在の綴り**で、
   * **全部が色を運ぶ**（影の色・輪郭の色）。**どれも一覧には無い。**
   *
   * **⚠ 着手時の形は、この4つを1つも拾わなかった**（手前の `-` を跨がない綴りだった）。
   * **それでも 967 全緑** —— **この族を見る対照が1本も無かったから。**
   * **変異検査で見つけて、走査の側を直した**（`support` の [`COLOUR_UTILITY`] の ⚠）。
   * **足したあと、着手時の形を変異として当て直して 1 赤**（作業単位 B-44 の `m8`）——
   * **「是正を外して緑に戻る」ほうも測ってある**（CLAUDE.md / `G-5` の4周目）。
   *
   * **⚠ 名乗る名前は尻尾のほうになる**（`text-shadow-[…]` は `shadow-[` として出る）。
   * **在り処は行番号が指す**ので、直す先は分かる。
   */
  it("接頭辞で終わる長い綴りも、止まる", () => {
    const planted = [
      '<p class="text-shadow-[0_1px_0_red]">字の影</p>',
      '<img class="drop-shadow-[0_0_4px_lab(50%_0_0)]" />',
      '<div class="inset-shadow-[0_0_4px_red]">',
      '<button class="inset-ring-[color-mix(in_oklch,_red,_blue)]">',
    ].join("\n");
    expect(colourCarryingArbitraryValues(planted)).toEqual([
      { text: "shadow-[", line: 1 },
      { text: "shadow-[", line: 2 },
      { text: "shadow-[", line: 3 },
      { text: "ring-[", line: 4 },
    ]);
  });

  /**
   * **⚠ 語の途中には当てない**（通す側 / 作業単位 B-44）。
   *
   * **手前の `-` は跨ぐ**（上の1本）**が、語の字は跨がない。**
   * **`coloring-` の中の `ring-` も `mytext-` の中の `text-` も、別の語である。**
   *
   * **これが無いと、手前を一切見ない形の変異が通る** ——
   * **この1本を足す前に実測して、967 全緑だった。**
   * **足したあと、同じ変異を当て直して 1 赤**（作業単位 B-44 の `m7`）。
   */
  it("語の途中には、当てない", () => {
    const planted = [
      "const coloring-[2px] = 1;",
      '<span class="mytext-[12px]">',
      "monitoring-[4px]",
      "subtext-[red]",
    ].join("\n");
    expect(colourCarryingArbitraryValues(planted)).toEqual([]);
  });

  /**
   * **⚠ 変位が前に付いていても止まる**（`dark:` / `md:` / `hover:` / `!` …）。
   *
   * **これが無いと、走査を「クラス属性の先頭の語だけ」に狭める変異が通る。**
   *
   * **⚠ 理由は「変位はどの接頭辞の前にも付けられる」ことそれ自体である。**
   * **この家が `dark:` を書くから、ではない** —— **`dark:` は使わない決め**
   * （ADR-0040 §決定5 の訂正。暗い配色は `@media` の中で `--color-*` を差し替える）。
   * **`md:` を使うのは `D-12` から。**
   * **⚠ つまり、いまこの家に変位つきのクラスは1つも無い。**
   * **それでも見るのは、任意値は「書かれた瞬間」に配布 CSS へ入るからで、
   * 書かれるときの形が変位つきでありうる。**
   */
  it("変位が前に付いていても、止める", () => {
    const planted = [
      '<span class="dark:bg-[lab(50%_0_0)]">',
      '<span class="md:hover:text-[color-mix(in_oklch,_red,_blue)]">',
      '<span class="!text-[red]">',
      '<span class="group-hover:border-[red]">',
      '<span class="[&>td]:text-[red]">',
    ].join("\n");
    expect(colourCarryingArbitraryValues(planted)).toEqual([
      { text: "bg-[", line: 1 },
      { text: "text-[", line: 2 },
      { text: "text-[", line: 3 },
      { text: "border-[", line: 4 },
      { text: "text-[", line: 5 },
    ]);
  });

  /**
   * **⚠ 色を運ばない任意値は、咎めない**（通す側）。
   *
   * `list[0]` も `grid-cols-[10rem_1fr_auto]` も**この家では正当**である
   * （ADR-0040 §決定4 は**色**の任意値だけを止めている）。
   * **⚠ ただし、この家に任意値のユーティリティは1つも無い**
   * （2026-09-17 に数えた —— `web/src` ＋ `index.html` で 0 件。`grid-cols` の綴りも 0 件）。
   * **かつてここには「`grid-cols-` は `D-12b` が実際に使っている」と書いてあった。偽である。**
   * **つまりここに植えてある綴りは、想定であって用例ではない。**
   * **これが無いと、走査を `/\[/` へ広げる変異が「厳しくなっただけ」に見えて通る。**
   *
   * **⚠ 見分けているのは接頭辞だけで、中身ではない** ——
   * **`w-` や `grid-cols-` は色を運べない**ので一覧に無い。
   */
  it("色を運ばない任意値は、咎めない", () => {
    const planted = [
      "const first = list[0];",
      '<td class="grid-cols-[10rem_1fr_auto]">',
      "items[index]",
      '<div class="w-[12rem] max-w-[40rem] min-h-[4rem]">',
      '<span class="text-warn bg-paper border-rule">',
      '<div class="gap-[2px] tracking-[0.02em]">',
    ].join("\n");
    expect(colourCarryingArbitraryValues(planted)).toEqual([]);
  });

  /**
   * **⚠ 字の大きさは「長さである」と名乗り、かつ数字で始まる綴りで書く**（作業単位 B-44）。
   *
   * **v4 の `text-[…]` は、色も長さも運ぶ**（`text-[14px]` は字の大きさ）。
   * **どちらであるかは中身を読まないと決まらず、中身を読む形が D-11a の穴だった。**
   * **だから素の `text-[` は止め、逃げ道だけを通す。**
   *
   * **⚠ 逃げ道の条件は「型を名乗ること」ではない。**
   * **`length:` の直後が数字か `-` か `.` であること**である（`support` の `TYPED_LENGTH`）。
   * **型の綴りが信用できないことは、下の1本が実測で押さえている。**
   *
   * **⚠ 代償**: **`calc(…)` と `var(…)` で書いた長さも止まる。**
   * **ここに2行置いてあるのは、その代償を帳簿に載せるため** ——
   * **逃げ道を素の `length:` へ広げ直す変異が、この1本で赤になる。**
   * **要る日は、その形だけを通す綴りを足すこと**（一覧を先に広げない）。
   *
   * **⚠ 割合も通る**（`from-[length:50%]`）。**数字で始まるからである。**
   * **かつてこの doc は「長さでも割合でもないものは逃げ道が無い」と書いていたが、
   * 見ているのは「長さかどうか」ではなく「値の先頭の字」だけである。**
   *
   * **⚠ この1本は両向きを同時に固定している** ——
   * **逃げ道を落とす変異**（数字で始まる長さも止める）**でも、
   * 逃げ道を広げる変異**（素の形も通す / `length:` なら何でも通す）**でも赤になる。**
   */
  it("長さでも素の角括弧は止め、数字で始まる長さだけ通す", () => {
    const planted = [
      '<span class="text-[0.875rem]">素の形</span>',
      '<span class="text-[length:0.875rem]">型を名乗る形</span>',
      '<span class="text-[color:var(--color-warn)]">色を名乗る形</span>',
      '<div class="border-[2px]">',
      '<div class="border-[length:2px]">',
      '<span class="text-[length:.5rem]">小数点から始まる形</span>',
      '<div class="shadow-[length:-2px_0_0]">負の値から始まる形</div>',
      '<div class="from-[length:50%]">割合も、数字で始まる</div>',
      '<span class="text-[length:calc(1rem_+_2px)]">代償: 計算は止まる</span>',
      '<div class="border-[length:var(--gap)]">代償: トークン引きも止まる</div>',
    ].join("\n");
    expect(colourCarryingArbitraryValues(planted)).toEqual([
      { text: "text-[", line: 1 },
      { text: "text-[", line: 3 },
      { text: "border-[", line: 4 },
      { text: "text-[", line: 9 },
      { text: "border-[", line: 10 },
    ]);
  });

  /**
   * **⚠ 型の綴りは、誰も検査していない**（B-44 のレビューが実測 / 2026-09-17）。
   *
   * **Tailwind v4 は `length:` と名乗った値を長さとして検査しない。**
   * **レビュアーが囮を植えて `pnpm build` を回し、配布 CSS に色として載ることを確かめている** ——
   * **`divide-[length:lab(…)]` は `border-color`、`fill-[length:red]` は `fill`、
   * `placeholder-` は `::placeholder` の `color`、`caret-` / `accent-` はそれぞれの色。**
   * **`text-[length:red]` だけは `font-size:red` になり、色にはならない。**
   *
   * **⚠ つまり「長さは、どう書いても色にならない」は偽だった。**
   * **かつてこの家の doc はそう書いていて、走査の逃げ道はその前提の上に載っていた** ——
   * **`length:` と名乗るだけで、5つの綴りが走査3つを素通りした**
   * （969 全緑 / 配布 CSS 6.85 → 7.16 kB）。
   * **信用できるのは「値が数字で始まること」だけである。**
   *
   * **⚠ ここに `text-[length:red]` も置いてある。**
   * **配布 CSS では色にならない綴りだが、走査は止める** ——
   * **接頭辞ごとに「その型なら色になるか」を判定する形は、中身を読む形へ戻る道**であり、
   * **それが D-11a の穴だった。通しすぎない側へ倒してある。**
   */
  it("「長さ」と名乗っても、止まる", () => {
    const planted = [
      '<span class="text-[length:red]">字の大きさになる綴り</span>',
      '<ul class="divide-[length:lab(50%_0_0)]">',
      '<svg class="fill-[length:red]">',
      '<input class="placeholder-[length:color-mix(in_oklch,_red,_blue)]" />',
      '<input class="caret-[length:red]" />',
      '<input class="accent-[length:red]" />',
    ].join("\n");
    expect(colourCarryingArbitraryValues(planted)).toEqual([
      { text: "text-[", line: 1 },
      { text: "divide-[", line: 2 },
      { text: "fill-[", line: 3 },
      { text: "placeholder-[", line: 4 },
      { text: "caret-[", line: 5 },
      { text: "accent-[", line: 6 },
    ]);
  });

  /**
   * **任意プロパティに書いた色**（第2の腕 / B-44 のレビューが実測 / 2026-09-17）。
   *
   * **接頭辞を持たない形がある** —— `[color:red]` / `[background-color:lab(…)]`。
   * **v4 はこれを「CSS の宣言をそのまま書く」形として受け取り、配布 CSS へ載せる**
   * （レビュアーが `pnpm build` で確認）。
   * **16 進で書いた形だけは `rawHexColours` が拾っていた**が、
   * **色名も `lab()` も、3つの目のどれにも掛かっていなかった。**
   *
   * **⚠ 手前が `-` なら当てない。** `text-[color:…]` は**第1の腕が既に名指ししている**ので、
   * **手前を見ない形にすると、同じ1件を二重に数えて上の1本が赤くなる**（レビュアーが実測）。
   *
   * **⚠ 報せる順は「第1の腕の全件 → 第2の腕の全件」**である。
   * **行番号の昇順ではない。** ここの期待値が、その順そのものを固定している。
   *
   * **⚠ 色でない任意プロパティは通る**（`[--gap:2px]` / `[mask-type:alpha]`）——
   * **見ているのはプロパティ名の尻尾が `color` / `colour` であることだけ。**
   *
   * **⚠ CRLF の腕**（CLAUDE.md / `E-1`）——**行番号を自分で数えている関数には要る。**
   */
  it("任意プロパティに書いた色も、止まる", () => {
    const lines = [
      '<span class="[color:red]">字の色</span>',
      '<div class="[background-color:lab(50%_0_0)]">',
      '<span class="text-[color:var(--color-warn)]">手前が - の形</span>',
      '<div class="[--gap:2px]">色ではない任意プロパティ</div>',
      '<div class="[mask-type:alpha]">色ではない任意プロパティ</div>',
    ];
    const expected = [
      { text: "text-[", line: 3 },
      { text: "[color:", line: 1 },
      { text: "[background-color:", line: 2 },
    ];
    expect(colourCarryingArbitraryValues(lines.join("\n"))).toEqual(expected);
    expect(colourCarryingArbitraryValues(lines.join("\r\n"))).toEqual(
      expected,
    );
  });

  /**
   * **⚠ CRLF の腕**（CLAUDE.md / `E-1`）——
   * **行番号を自分で数えている関数には要る。**
   * **止める側と通す側の両方に付けてある** ——
   * **通す側だけ、止める側だけ、のどちらかが CRLF で崩れる形を残さないため。**
   */
  it("CRLF でも、色を運ぶ接頭辞の判定は同じ", () => {
    const stopped = [
      "/* 1行目 */",
      "<span",
      '  class="dark:bg-[lab(50%_0_0)]"',
      ">",
    ];
    expect(colourCarryingArbitraryValues(stopped.join("\r\n"))).toEqual([
      { text: "bg-[", line: 3 },
    ]);
    expect(colourCarryingArbitraryValues(stopped.join("\r\n"))).toEqual(
      colourCarryingArbitraryValues(stopped.join("\n")),
    );

    const passed = [
      "/* 1行目 */",
      "<div",
      '  class="grid-cols-[10rem_1fr_auto] text-[length:0.875rem]"',
      ">",
    ];
    expect(colourCarryingArbitraryValues(passed.join("\r\n"))).toEqual([]);
    expect(colourCarryingArbitraryValues(passed.join("\r\n"))).toEqual(
      colourCarryingArbitraryValues(passed.join("\n")),
    );
  });

  /** **ユーティリティを CSS 側へ書き戻す指示子**（ADR-0040 §決定4）。 */
  it("書き戻しの指示子を見つける", () => {
    const planted = [".error {", "  @apply text-warn;", "}"].join("\n");
    expect(utilityWriteBacks(planted)).toEqual([{ text: "@apply", line: 2 }]);
    // **トークンを引く形は、書き戻しではない。**
    expect(utilityWriteBacks(".error { color: var(--color-warn); }")).toEqual(
      [],
    );
    // **綴りが続いているものは、別の語である。**
    expect(utilityWriteBacks("@applyx")).toEqual([]);
  });

  /**
   * **束ねる側が解決する取り込みと、外への取り込みを見分ける。**
   *
   * **⚠ ここが D-11a で緩んだ場所である。**
   * **緩めた幅を、この1本が固定している** ——
   * **引用符の中がパッケージ名だけの形だけが通り、他は全部咎められる。**
   */
  it("束ねる側の取り込みと、外への取り込みを見分ける", () => {
    // **この家が実際に書いている2つの形。**
    expect(
      networkFetches('@import "tailwindcss/theme.css" layer(theme);'),
    ).toEqual([]);
    expect(
      networkFetches(
        '@import "tailwindcss/utilities.css" layer(utilities) source(none);',
      ),
    ).toEqual([]);
    expect(networkFetches('@import "tailwindcss";')).toEqual([]);
    // **探す根を広げる形は許さない**（`support` の [`BUNDLED_IMPORT`] の ⚠）。
    expect(
      networkFetches('@import "tailwindcss/utilities.css" source("../..");'),
    ).toEqual(["@import"]);
    // **外の宛先は、引用符の中にあっても咎められる。**
    expect(
      networkFetches('@import "https://fonts.example.com/x.css";'),
    ).toEqual(["@import", "https:"]);
    // **scheme を書かない形も。**
    expect(networkFetches("@import url(//fonts.example.com/y.css);")).toEqual([
      "@import",
      "//fonts.example.com",
    ]);
    // **手元に置いた書体の宣言も咎める**（字は既定から選ぶ / `app.css` の冒頭）。
    expect(networkFetches("@font-face { font-weight: 400; }")).toEqual([
      "@font-face",
    ]);
    // **点を持たない宛先**（`localhost`）**も、scheme で捕まる。**
    expect(
      networkFetches('.a { background: url("http://localhost/x.png"); }'),
    ).toEqual(["http:"]);
    // **何も取りに行かないソースからは、何も出ない。**
    expect(networkFetches(".a { color: var(--color-warn); }")).toEqual([]);
  });

  it("トークンの定義と参照を、別々に数える", () => {
    const planted = [
      ":root { --warn: oklch(0.56 0.11 65); --read: oklch(0.5 0.11 250); }",
      ".error { color: var(--warn); }",
      ".notice { color: var(--read); }",
    ].join("\n");
    expect(tokenDefinitions(planted)).toEqual(["--warn", "--read"]);
    expect(tokenReferences(planted)).toEqual(["--warn", "--read"]);
    // **参照しかしないソースは、定義していない。**
    expect(tokenDefinitions(".error { color: var(--warn); }")).toEqual([]);
  });

  it("`var()` の綴りと、そうでない綴りを見分ける", () => {
    expect(tokenReference("var(--warn)")).toBe("--warn");
    expect(tokenReference("1px solid var(--warn)")).toBeNull();
    expect(tokenReference("oklch(0.56 0.11 65)")).toBeNull();
  });

  it("`oklch()` の色相を読む", () => {
    expect(oklchHue("oklch(0.56 0.11 65)")).toBe(65);
    expect(oklchHue("oklch(0.50 0.11 250)")).toBe(250);
    // **色相の無い綴りは読めない**（読めたことにすると、帳簿が黙って緩む）。
    expect(oklchHue("oklch(0.56 0.11)")).toBeNull();
    expect(oklchHue("#b00020")).toBeNull();
  });

  /**
   * **対照の対照 —— 帳簿の判定が、実際に両向きへ動く。**
   *
   * 「暖色の側」と「寒色の側」の綴りを入れ替えた `:root` を植えて、
   * **同じ判定が逆の答えを返すこと**を見る。
   * **これが無いと、`WARM` / `COOL` の幅を広げる変異が黙って通る。**
   */
  it("暖色と寒色の判定は、入れ替えると逆になる", () => {
    const warm = oklchHue("oklch(0.56 0.11 65)") as number;
    const cool = oklchHue("oklch(0.50 0.11 250)") as number;

    expect(warm >= WARM.from && warm < WARM.to).toBe(true);
    expect(cool >= WARM.from && cool < WARM.to).toBe(false);
    expect(cool >= COOL.from && cool < COOL.to).toBe(true);
    expect(warm >= COOL.from && warm < COOL.to).toBe(false);
  });
});
