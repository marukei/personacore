/**
 * **色と字の走査**（作業単位 D-10）。
 *
 * # 何を見るのか
 *
 * **`web/src` の下に、生の 16 進の色が1つも無いこと。**
 * 着手前は **52 箇所**あり、**同じ綴りが 5〜6 のコンポーネントに写っていた**
 * （`#666` ×16 / `#b00020` ×14 / `#ccc` ×7 …）。
 * **写しは片方を直しても、もう片方が黙って古びる**（CLAUDE.md）ので、
 * **色の綴りは `web/src/app.css` にしか置かない。**
 *
 * **⚠ かつてここには「`@theme` にしか置かない」と書いてあった。D-11b で偽になった** ——
 * **暗い配色の 12 個は `@media (prefers-color-scheme: dark)` の `:root` に在る**
 * （`@theme` を2つ置かない理由は `src/app.css` の冒頭）。
 * **ファイルが1つであることは変わっていないが、囲みは2つある。**
 *
 * **⚠ D-11a（Tailwind / ADR-0040）で、見るものが3つ増えた。**
 *
 * * **任意値**（[`arbitraryColourValues`]）—— `text-[#b00020]` の形。
 *   **既定のパレットを消しても、この形からは色が生えてしまう**（ADR-0040 §決定4）。
 *   **⚠ B-44 で、これと対になる目が1つ増えた**
 *   （[`colourCarryingArbitraryValues`]）——**色を運ぶ接頭辞の直後の角括弧。**
 *   **こちらは中身を読まない**ので、**色名も `lab()` も `color-mix()` も止まる。**
 *   **2つの目が見ているものと、交換した理由は、その doc にある。**
 *   **⚠ そちらには腕が2本ある**（B-44 のレビューの後）——
 *   **接頭辞を要求する腕と、要求しない腕**（`[color:…]` の形 /
 *   [`ARBITRARY_COLOUR_PROPERTY`]）。**接頭辞の無い形は、どの目にも掛かっていなかった。**
 * * **ユーティリティを CSS 側へ書き戻す指示子**（[`utilityWriteBacks`]）——
 *   **`D-10` が消した「規則が CSS に散る」形を作り直す**ので使わない。
 * * **束ねる側が解決する取り込み**（[`networkFetches`]）——
 *   **`@import` を一律に咎める形は、Tailwind を入れた時点で成り立たない。**
 *   **咎める先を「外の宛先」に絞った。** 何を許したかは、そこの ⚠ に書いてある。
 *
 * # ⚠ 免除の一覧を持たない
 *
 * **`web/src` の下の「全ファイル」を、拡張子で選ばずに読む。**
 *
 * * **免除の一覧があると、定義をそこへ移すだけで帳簿が無言で減る**
 *   （CLAUDE.md / `G-3b` の実測）。**ここには一覧が1つも無い。**
 * * **取り込む拡張子の一覧も持たない。** 持つと、
 *   **`.scss` や別の綴りへ移した日に走査が黙って素通りする。**
 * * **トークンを定義しているファイル（`app.css`）も除外しない。**
 *   除外が要らないのは、**トークンの値が `oklch()` で書いてあって
 *   16 進が1文字も出てこない**から。**16 進で書き直したら、ここが赤くなる。**
 *
 * **⚠ 根は `web/src` ＋ 名指しの1件**（[`ENTRY_HTML`]）**である。**
 * **`web/` の下に3つ目の入口を作ったら、ここへ足すまで射程の外**——
 * **免除の一覧は持たないが、「根の一覧」は持っている。守れていない、と読むこと。**
 *
 * **⚠ 代わりに前提が1つある —— `web/src` の下はテキストだけ。**
 * 画像などを置いたら、**中身を UTF-8 として読んだ偶然の並びが色に見えうる。**
 * そのときはこの走査を作り直すこと。
 *
 * # ⚠ コメントの中も見る
 *
 * **字句解析をしない。** コメントを外すには文字列と正規表現を数える必要があり、
 * **日本語のコメントで走査が壊れた実績がある**（CLAUDE.md / `B-21d`
 * —— `"` が1個だけ出た所から偽の文字列が始まり、6件が 0件になった）。
 * **広く採る側に倒してある**ので、**コメントに昔の綴りを書いても赤くなる。**
 * 書き残したいなら `docs/` か `web/README.md` へ。
 *
 * # ⚠ 走査本体は、ソースを**引数で**受け取る
 *
 * 実ファイルだけを読む走査は、**対照テストが自己成就する**
 * （CLAUDE.md / `15b` の実測）。
 * **一覧を作る側（[`sourceFiles`]）と判定する側（[`rawHexColours`] /
 * [`cssRules`] …）を分けてある**ので、
 * **実ファイルと、テストが植えたソースを、同じ判定に通せる。**
 */

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

/**
 * 生の 16 進の色の綴り。
 *
 * **3 / 4 / 6 / 8 桁**（`#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa`）。
 * **大文字も小文字も拾う** ——
 * **植えた綴りが小文字だけだと、ここを `[0-9a-f]` へ狭める変異が全緑を通る**
 * （D-10 のレビューで実測）。**対照は「大文字の 16 進も、桁の数に関係なく見つかる」。**
 * **後ろに英数字か `_` が続くものは色ではない** —— そうしないと
 * Svelte の `{#each ...}`（`#eac` ＋ `h`）を色として拾う。
 */
const RAW_HEX =
  /#(?:[0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})(?![0-9a-zA-Z_])/g;

/** 見つけた1件。 */
export interface HexFinding {
  /** 綴り、そのまま。 */
  readonly hex: string;
  /** **1 始まりの行番号。** */
  readonly line: number;
}

/**
 * そのソースに含まれる生の 16 進の色（**重複もそのまま**、出てきた順）。
 *
 * **⚠ 行番号は自分で数えている**ので、**改行の綴りに依らないこと**を
 * 対照テストが CRLF の腕で見ている（CLAUDE.md / `E-1` の是正の形）。
 */
export function rawHexColours(source: string): HexFinding[] {
  return findAll(source, RAW_HEX).map((found) => ({
    hex: found.text,
    line: found.line,
  }));
}

/** 走査が見つけた1件（綴りと、**1 始まりの行番号**）。 */
export interface Finding {
  /** 綴り、そのまま。 */
  readonly text: string;
  /** **1 始まりの行番号。** */
  readonly line: number;
}

/**
 * その位置の行番号（**1 始まり**）。
 *
 * `\r` は数えない。**`\n` の数だけが行を決める**ので、CRLF でも LF でも同じ
 * （CLAUDE.md / `E-1` —— 自分で位置から数える関数には CRLF の腕が要る）。
 */
function lineAt(source: string, index: number): number {
  let breaks = 0;
  for (const character of source.slice(0, index)) {
    if (character === "\n") {
      breaks += 1;
    }
  }
  return breaks + 1;
}

/** その綴りに当たる全件を、在り処つきで（出てきた順、重複もそのまま）。 */
function findAll(source: string, pattern: RegExp): Finding[] {
  return [...source.matchAll(pattern)].map((match) => ({
    text: match[0],
    line: lineAt(source, match.index ?? 0),
  }));
}

/**
 * **任意値で書いた色**（`text-[#b00020]` / `bg-[oklch(0.5_0.11_250)]` …）。
 *
 * **⚠ なぜ要るのか**: 既定のパレットは `@theme` で消してある（ADR-0040 §決定3）ので
 * **`text-red-500` は存在しないクラス**になる。**任意値はその外から入る** ——
 * **クラス名の中に色を直接書けるので、色が1箇所に集まっているという前提が消える。**
 *
 * **⚠ 16 進だけなら [`rawHexColours`] も拾う。**
 * **ここでしか拾えないのは `oklch()` / `rgb()` / `hsl()` の形**である
 * （この家のトークンは `oklch()` なので、**写すならその形になる**）。
 *
 * **⚠ ここは「角括弧の中身の綴り」しか見ない。**
 * **色名も `lab()` / `lch()` / `hwb()` / `color-mix()` も素通りする** ——
 * **D-11a のレビューが実測**（色名の任意値を植えて 789 全緑、配布 CSS に生えた）。
 * **それを止めるのは、下の [`colourCarryingArbitraryValues`]**（作業単位 B-44）。
 *
 * **⚠ それでも、この側を残してある。** **接頭辞に依らない**ので、
 * **下の一覧が v4 の語彙に追いつかなくなった日でも、16 進だけは止まる。**
 * **⚠ 逆に言えば、接頭辞を持たない綴り**（`[#` を裸で書く形）**を見るのはここだけである。**
 */
const ARBITRARY_COLOUR = /\[(?:#|(?:oklch|rgb|rgba|hsl|hsla)\()/g;

/** そのソースが任意値で書いている色（出てきた順、重複もそのまま）。 */
export function arbitraryColourValues(source: string): Finding[] {
  return findAll(source, ARBITRARY_COLOUR);
}

/**
 * **色を運べるユーティリティの接頭辞**（作業単位 B-44 / ADR-0040 §決定4）。
 *
 * **⚠ 一覧を伸ばす形では追いつかない。** [`ARBITRARY_COLOUR`] は
 * **角括弧の中身の綴り**を見ていたので、**その一覧の外の書き方が素通りした**
 * （色名 / `lab()` / `color-mix()`、そして影のように値の途中に色が埋まる形）。
 * **CSS の色の書き方は増える**ので、**中身を数える側はいつか必ず負ける**
 * （CLAUDE.md の `21b` —— **主題が「一覧の外」なら、一覧の外を測らないのは測っていないのと同じ**）。
 *
 * **⚠ だから、中身ではなく「入れ物」を見る。**
 * **色を運べる接頭辞の直後に角括弧が来たら、中身を問わず止める。**
 *
 * **⚠ 一覧に載せるのは「尻尾が一覧に無い綴り」だけ。**
 * **手前の `-` は跨ぐ**（[`COLOUR_UTILITY`] の ⚠）ので、
 * **尻尾がここに在る長い綴りは、載せなくても止まる** ——
 * **`text-shadow-` / `drop-shadow-` / `inset-shadow-` は `shadow` で、
 * `inset-ring-` は `ring` で当たる**（どれも v4 の実在の綴りで、どれも色を運ぶ）。
 * **載せると、その分だけ「落としても他が拾う」死んだ項目になる。**
 * 見張り番は `tests/design-tokens.test.ts` の「接頭辞で終わる長い綴りも、止まる」。
 *
 * **⚠ 逆に、尻尾が一覧に無いものは、1つずつが単独で要る。**
 * **`border-t-` は `border-` では拾えない**（直後が `[` でないと当たらない）し、
 * **`t` は色を運ばないので一覧に無い。`ring-offset-` の `offset` も同じ。**
 * 見張り番は同じファイルの「色を運ぶ接頭辞は、1つ残らず止まる」。
 *
 * **⚠ 並びは答えを決めない。** 角括弧が直後に要るので、
 * `border-` が先に当たっても `border-t-…` では成立せず、`border-t-` まで戻る。
 * **読みやすい順に並べてよい。**
 *
 * **⚠ 守れていないこと**: **色を運ぶ「尻尾」が増えたら、ここへ足すまで射程の外である。**
 * **v4 が新しい綴りを生やした日、この一覧は黙って古びる**（**免除の一覧と同じ向きの穴** /
 * CLAUDE.md の `G-3b`）。**「色の書き方」を追うのはやめたが、「色を運ぶ場所」は追う必要がある。**
 * **交換した理由は、後者のほうが増えにくいから**であって、増えないからではない。
 * **⚠ 尻尾で当てる形は、その古び方を少しだけ遅くする** ——
 * **既存の尻尾に前置きを足した綴り**（`…-shadow-` の族）**は、足さなくても止まる。**
 */
const COLOUR_CARRYING_PREFIXES = [
  // 字と背景。
  "text",
  "bg",
  // 枠（辺ごとの綴りも、それぞれが要る / 上の ⚠）。
  // **⚠ 論理的な辺**（`border-s` / `border-e`）**は、B-44 のレビューの時点で抜けていた**
  // （2026-09-17 に実測 —— `border-s-[red]` が走査3つを素通りし、
  // 配布 CSS に `border-inline-start-color:red` が載った。969 全緑）。
  // **「一覧に無い接頭辞は色を運べない」は、この2つで偽になった。**
  "border",
  "border-t",
  "border-r",
  "border-b",
  "border-l",
  "border-x",
  "border-y",
  "border-s",
  "border-e",
  "outline",
  "ring",
  "ring-offset",
  // 図形。
  "fill",
  "stroke",
  // 影（**値の途中に色が埋まる** —— 中身を読む側が原理的に届かない形）。
  // **`inset-` / `drop-` / `text-` を前置きした族は、ここへ足さない**（上の ⚠）。
  "shadow",
  // 字の飾りと、フォームの部品。
  "decoration",
  "accent",
  "caret",
  "divide",
  "placeholder",
  // 階調（`bg-linear-*` の止め色）。
  "from",
  "via",
  "to",
];

/**
 * **逃げ道の綴り** —— **「長さである」と名乗り、かつ値が数字で始まる形**
 * （作業単位 B-44 / **レビューで測り直した形** / 2026-09-17）。
 *
 * **v4 の `text-[…]` は色も長さも運ぶ**（`text-[14px]` は字の大きさ）。
 * **どちらであるかは中身を読まないと決まらず、中身を読む形が D-11a の穴だった。**
 * **だから素の角括弧は止め、逃げ道を1つだけ開けてある。**
 *
 * **⚠ 逃げ道を「型の綴り」で決めることはできない。Tailwind は型を検査しないからである。**
 * **B-44 のレビューが囮を植えて `pnpm build` まで回して実測** ——
 * **`divide-[length:lab(…)]` は `border-color`、`fill-[length:red]` は `fill`、
 * `placeholder-[length:color-mix(…)]` は `::placeholder` の `color`、
 * `caret-` / `accent-` はそれぞれの色**として、**配布 CSS に色のまま載った**
 * （**5つの綴りが走査3つを素通りして 969 全緑。配布 CSS 6.85 → 7.16 kB**）。
 * **⚠ かつてここには「長さは、どう書いても色にならない」と書いてあった。偽である。**
 *
 * **⚠ 代わりに見るのは、値の先頭の字だけ** —— **`length:` の直後が
 * 数字か `-` か `.` のときだけ通す。** **色の綴りはどれもここから始まらない**
 * （色名も `lab(` も `color-mix(` も `var(` も英字、16 進は `#`）。
 *
 * **⚠ 代償**: **`calc(…)` と `var(…)` で書いた長さも止まる。**
 * **要る日に、その形だけを通す綴りを足すこと**（一覧を先に広げない）。
 * 見張り番は `tests/design-tokens.test.ts` の
 * 「長さでも素の角括弧は止め、数字で始まる長さだけ通す」——
 * **代償の2行もそこに植えてある**ので、**逃げ道を素の `length:` へ広げ直す変異が赤になる。**
 *
 * **⚠ 通すのはこの1つだけ。** 他の型の綴りは通さない ——
 * **`color:` は最も露骨に色を運ぶ形**で、残り（`percentage:` / `number:` …）は
 * **要る日に、その綴りだけを足す。** **一覧を先に広げない**
 * （広げた一覧は、使われないまま穴として残る）。
 *
 * **⚠ 逃げ道は全部の接頭辞に効く** —— 枠の太さも、影のぼかしも、同じ綴りで書く。
 * **接頭辞ごとに「この型なら色にならない」を判定する形にはしない** ——
 * **それは中身を読む形へ戻る道**であり、**それが D-11a の穴だった。**
 * **その結果、`text-[length:red]` のように
 * 配布 CSS では色にならない綴り**（実測では `font-size:red`）**も止まる。**
 * **通しすぎない側へ倒してある。**
 *
 * **⚠ 割合も通る**（`from-[length:50%]`）——**数字で始まるからである。**
 * **⚠ かつてここには「長さでも割合でもないものは、逃げ道が無い」と書いてあった。**
 * **見ているのは「長さかどうか」ではなく「先頭の字」だけ**なので、
 * **その言い方では、何が通って何が止まるかを決められない。**
 *
 * **⚠ これは正規表現の綴りである**（[`COLOUR_UTILITY`] の否定先読みに埋め込まれる）。
 * **文字の列ではない**ので、`[-.0-9]` は文字クラスとして読まれる。
 */
const TYPED_LENGTH = "length:[-.0-9]";

/**
 * 色を運べる接頭辞の直後の角括弧（[`COLOUR_CARRYING_PREFIXES`] / [`TYPED_LENGTH`]）。
 *
 * **⚠ 手前が語の字なら当てない。** `coloring-[…]` の中の `ring-[` も、
 * `mytext-[…]` の中の `text-[` も、**別の語であって、色を運ぶユーティリティではない。**
 * 見張り番は `tests/design-tokens.test.ts` の「語の途中には、当てない」。
 *
 * **⚠ 手前の `-` は跨ぐ。ここは意図的である。**
 * **`-` の手前が違う語だということは、`\w` と違って「別物」を意味しない** ——
 * **v4 は色を運ぶ綴りを前置きで増やす**（`text-shadow-` / `drop-shadow-` /
 * `inset-shadow-` / `inset-ring-`）。**`-` を跨がない形にすると、その族が丸ごと素通りする。**
 * **⚠ 変異検査で実測**（作業単位 B-44 / `m8`）—— **`-` を跨がない形は
 * 実在の4つの綴りを1つも拾わず、それでも 967 全緑だった**
 * （**当時は、その族を見る対照が1本も無かった**）。
 * **対照を足したあと、同じ変異を当て直して 1 赤。**
 * 見張り番は同じファイルの「接頭辞で終わる長い綴りも、止まる」。
 *
 * **⚠ 変位は手前に何個あってもよい** —— `dark:` / `md:hover:` / `!` / `[&>td]:` は
 * どれも `\w` ではないので、**その先の接頭辞から当たる。**
 * 見張り番は同じファイルの「変位が前に付いていても、止める」。
 */
const COLOUR_UTILITY = new RegExp(
  `(?<!\\w)(?:${COLOUR_CARRYING_PREFIXES.join("|")})-\\[(?!${TYPED_LENGTH})`,
  "g",
);

/**
 * **任意プロパティに書いた色**（**第2の腕** / 作業単位 B-44 のレビューが実測 / 2026-09-17）。
 *
 * **接頭辞を持たない形がある** —— `[color:red]` / `[background-color:lab(…)]`。
 * **v4 はこれを「CSS の宣言をそのまま書く」形として受け取り、配布 CSS へ載せる。**
 * **[`COLOUR_UTILITY`] は接頭辞を要求するので当たらず、
 * [`ARBITRARY_COLOUR`] は中身の綴りを見るので色名にも `lab()` にも当たらない**
 * （**16 進で書いた `[color:#…]` だけは [`rawHexColours`] が拾っていた**）。
 *
 * **⚠ 手前が語の字か `-` なら当てない。** `text-[color:…]` は
 * **第1の腕が既に名指ししている**ので、**当てると同じ1件を二重に数える**
 * （実測: 手前を見ない形にすると
 * 「長さでも素の角括弧は止め、数字で始まる長さだけ通す」が赤くなる ——
 * **あの1本が `text-[color:…]` を持っているから**）。
 *
 * **⚠ 見るのはプロパティ名の尻尾だけ** —— **`color` / `colour` で終わる名前。**
 * **`[--gap:2px]` も `[mask-type:alpha]` も通る。**
 *
 * **⚠ 小文字の綴りしか見ない。** **CSS のプロパティ名は大文字小文字を区別しない**ので、
 * **`[COLOR:red]` の形は、ここを素通りする。**
 * **⚠ ただし、いまは穴ではない**（2026-09-17 に測った）——
 * **その綴りを `web/src` に植えて `pnpm build` を回すと、
 * 配布 CSS は囮なしのものとバイト単位で同一だった**（md5 も大きさも動かない）。
 * **v4 はその形から規則を1つも作らない。**
 * **⚠ 守れていないのは「v4 がいつか作り始めた日」である。**
 * **そのときは黙って素通りする**（一覧が古びるのと同じ向きの穴）。
 */
const ARBITRARY_COLOUR_PROPERTY = /(?<![\w-])\[[a-z-]*colou?r\s*:/g;

/**
 * そのソースが、**色を運べる接頭辞の直後に書いている任意値**と、
 * **色のプロパティを名乗る任意プロパティ**（[`ARBITRARY_COLOUR_PROPERTY`]）。
 *
 * **綴りは接頭辞と角括弧まで**（`text-[`）、
 * **任意プロパティならプロパティ名と `:` まで**（`[background-color:`）。
 * **中身は読まない**ので、**報せは「どの入れ物か」と行番号までである。**
 *
 * **⚠ 順は「第1の腕の全件 → 第2の腕の全件」で、行番号の昇順ではない。**
 * **腕ごとには、出てきた順（重複もそのまま）。**
 * 見張り番は `tests/design-tokens.test.ts` の「任意プロパティに書いた色も、止まる」——
 * **その1本が、この順そのものを期待値で固定している。**
 */
export function colourCarryingArbitraryValues(source: string): Finding[] {
  return [
    ...findAll(source, COLOUR_UTILITY),
    ...findAll(source, ARBITRARY_COLOUR_PROPERTY),
  ];
}

/**
 * **ユーティリティを CSS 側へ書き戻す指示子**（ADR-0040 §決定4）。
 *
 * **使わないと決めてある。** `D-10` が消した「同じ規則が部品に散る」形を作り直すからで、
 * v4 自身も勧めていない。**綴りはここにしか無い** ——
 * **走査はコメントの中も見る**ので、`web/src` の下に例として書くこともできない。
 */
const UTILITY_WRITE_BACK = /@apply\b/g;

/** そのソースが書き戻している箇所（出てきた順、重複もそのまま）。 */
export function utilityWriteBacks(source: string): Finding[] {
  return findAll(source, UTILITY_WRITE_BACK);
}

/** 走査した1件。 */
export interface ScannedFile {
  /** `web/src` からの相対パス（区切りは `/`）。 */
  readonly path: string;
  /** ファイルの中身、**そのまま**。 */
  readonly source: string;
}

/** 走査の根（`web/src`）。 */
const SRC = fileURLToPath(new URL("../../src", import.meta.url));

/**
 * **画面の入口の文書**（`web/index.html`）。**根の外にある1件を、名指しで足してある。**
 *
 * **⚠ 足した理由**: `vite build` はこの文書を `dist/index.html` へ**写す**ので、
 * **ここに `<style>` を書けば画面に効き、`<link rel="stylesheet" href="https://…">` を
 * 書けば画面が外への接続に依存する。** 根が `web/src` だけだと、そこは射程の外だった
 * （D-10 のレビューで実測: `index.html` に生の 16 進を入れても全緑）。
 *
 * **⚠ 根そのものを `web/` へ広げる形は採らなかった。**
 * `web/README.md` には**着手前の綴りが記録として残っている**（`#666` ×16 / …）ので、
 * 広げると**免除の一覧が要る。** 免除の一覧は、
 * **定義をそこへ移すだけで帳簿が無言で減る**（CLAUDE.md / `G-3b`）。
 * **名指しで1件足す側は、その穴を作らない。**
 *
 * **⚠ 代わりに、守れていないことが1つ増える** ——
 * **`web/` の下に3つ目の入口を作ったら、ここへ足すまで射程の外である。**
 */
const ENTRY_HTML = fileURLToPath(new URL("../../index.html", import.meta.url));

/** [`sourceFiles`] が入口の文書に与える `path`（根の外なので `../` で始まる）。 */
export const ENTRY_PATH = "../index.html";

/**
 * `web/src` の下の**全ファイル**（**拡張子で選ばない** / モジュール doc の ⚠）と、
 * **画面の入口の文書1件**（[`ENTRY_HTML`]）を読む。
 *
 * **`web/tests` は見ない。** テストは色の綴りを**植えたソースとして持つ**必要があり
 * （この走査の対照そのもの）、混ぜると「探せているから緑」と
 * 「見つからないから緑」が区別できなくなる。
 */
export function sourceFiles(): ScannedFile[] {
  const found: ScannedFile[] = [];
  const walk = (directory: string, prefix: string): void => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const full = join(directory, entry.name);
      const relative = prefix === "" ? entry.name : `${prefix}/${entry.name}`;
      if (entry.isDirectory()) {
        walk(full, relative);
        continue;
      }
      found.push({ path: relative, source: readFileSync(full, "utf8") });
    }
  };
  walk(SRC, "");
  // **根の外の1件**（[`ENTRY_HTML`] の ⚠）。
  found.push({ path: ENTRY_PATH, source: readFileSync(ENTRY_HTML, "utf8") });
  return found;
}

/** `<style ...>` ... `</style>` の1件。 */
const STYLE_BLOCK = /<style\b[^>]*>([\s\S]*?)<\/style>/gi;

/** `.svelte` の `<style>` の中身を、出てきた順に返す。 */
export function styleBlocks(source: string): string[] {
  return [...source.matchAll(STYLE_BLOCK)].map((found) => found[1] ?? "");
}

/**
 * その1件の**スタイルシート**。
 *
 * `.css` はそのまま、`.svelte` と `.html` は `<style>` の中身をつないだもの、
 * それ以外は空文字列。
 *
 * **⚠ `.html` を入れてあるのは、入口の文書が根の外から効くから**
 * （[`ENTRY_HTML`] の ⚠）。
 */
export function stylesheetOf(file: ScannedFile): string {
  if (file.path.endsWith(".css")) {
    return file.source;
  }
  if (file.path.endsWith(".svelte") || file.path.endsWith(".html")) {
    return styleBlocks(file.source).join("\n");
  }
  return "";
}

/** 宣言1件。 */
export interface CssDeclaration {
  readonly property: string;
  readonly value: string;
}

/** 規則1件。 */
export interface CssRule {
  /**
   * その規則を囲んでいる `@` の並び（`@media (prefers-color-scheme: dark)`）。
   * **囲まれていなければ `null`。**
   *
   * **⚠ 分けてある理由**: 暗い配色は作業単位 `D-11` で `@media` の中に入る。
   * **そこで再定義されるトークンを、素の `:root` と混ぜて読むと、
   * 帳簿がどちらの値を見ているのか分からなくなる。**
   */
  readonly within: string | null;
  /**
   * その規則が入っている**層**（`@layer base { … }` の `base`）。**層の外なら `null`。**
   *
   * **⚠ 囲み（`within`）とは別の欄にしてある**（作業単位 D-12b-1）。
   * 層は「どの条件で効くか」ではなく**「どの順で勝つか」**しか変えないので、
   * **`within` に混ぜると、層へ移しただけで帳簿の鍵と `declarationOf` が全部外れる。**
   * **囲みは層を素通りする** —— `@layer base { @media (…) { nav {…} } }` の `nav` は、
   * `within` が `@media (…)`、`layer` が `base` になる。
   * **名前の無い `@layer { … }` は空文字列。**
   */
  readonly layer: string | null;
  /** `{` の手前の綴り、空白を畳んだもの。 */
  readonly selector: string;
  readonly declarations: readonly CssDeclaration[];
}

/**
 * スタイルシートを規則に分ける。
 *
 * **これは CSS の完全な構文解析ではない。** ここが前提にしているもの:
 *
 * * **コメントはブロックコメントだけ**（CSS に行コメントは無い）。
 * * **宣言の値に `;` と `{` が出てこない**（`content` に `;` を書く形は無い）。
 * * **入れ子の規則は書かない**（`@` の中の規則だけが入れ子になる）。
 *
 * **⚠ `@` は2通りある。** `@media` は**規則**を持ち、`@theme` は**宣言**を直に持つ
 * （ADR-0040 §決定2）。**後者は `selector` に `@theme` を入れた1件として出る**ので、
 * `declarationOf(rules, "@theme", "--color-warn")` で引ける。
 *
 * **⚠ `@layer` は3つ目の形**（作業単位 D-12b-1）—— 規則を持つが、**囲みにはせず**
 * `layer` の欄に名前を入れる（[`CssRule`] の `layer` の ⚠）。
 *
 * **崩れたら、その日にここを作り直すこと。**
 */
export function cssRules(source: string): CssRule[] {
  const found: CssRule[] = [];
  parseRules(stripCssComments(source), null, null, found);
  return found;
}

/**
 * **層のブロック**（`@layer base`）なら、その名前。**層の宣言**（`@layer a, b;`）はブロックを
 * 持たないので、ここへは来ない（`parseRules` が `;` で飛ばす）。層でなければ `null`。
 */
function layerBlockName(prelude: string): string | null {
  const found = /^@layer(?:\s+([A-Za-z_][\w.-]*))?$/.exec(prelude);
  if (found === null) {
    return null;
  }
  return found[1] ?? "";
}

/** ブロックコメントを落とす。 */
function stripCssComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, "");
}

function parseRules(
  text: string,
  within: string | null,
  layer: string | null,
  out: CssRule[],
): void {
  let at = 0;
  for (;;) {
    const open = text.indexOf("{", at);
    if (open < 0) {
      return;
    }
    // **ブロックを持たない `@`**（`@import "…";` / `@charset "…";`）。
    //
    // **⚠ これを飛ばさないと、次の規則を丸ごと飲む。**
    // 変異検査で実測（`m9`）: `:root` の直前に `@import` を1行足したら、
    // **前置きが `@import …; :root` になって `:root` が `@` の中身として読まれ、
    // 帳簿の 13 本が「`:root` にトークンが無い」と言い出した。**
    // **止まりはするが、名指しする名前が違う**（CLAUDE.md / `B-38`）。
    const semicolon = text.indexOf(";", at);
    if (semicolon >= 0 && semicolon < open) {
      at = semicolon + 1;
      continue;
    }
    const prelude = text.slice(at, open).trim().replace(/\s+/g, " ");
    const close = matchingBrace(text, open);
    const body = text.slice(open + 1, close);
    const layerName = prelude.startsWith("@") ? layerBlockName(prelude) : null;
    if (layerName !== null) {
      // **層は囲みではない**（[`CssRule`] の `layer` の ⚠）——
      // **`within` はそのまま渡し、層の名前だけを差し替える。**
      parseRules(body, within, layerName, out);
    } else if (prelude.startsWith("@")) {
      // **`@` が宣言を直に持つことがある**（`@theme` / ADR-0040 §決定2）。
      // **拾わないと、トークンの定義が丸ごと帳簿から消える** ——
      // `@theme` の中身は規則ではないので、下の再帰では1件も出てこない。
      //
      // **⚠ 中に `{` を持つ `@` は、規則の入れ物であって宣言を持たない**（`@media`）。
      // **着手前は「宣言を拾って、空なら押し込まない」だった** —— それは
      // **中の規則が宣言を1つしか持たないときだけ空になる**（`;` で切った断片が
      // 必ず `{` を含むから）。**2つ目の宣言からは `{` を含まない断片になり、
      // 囲みの名前を選択子にした偽の規則として押し込まれていた**（作業単位 D-12b-1 で発見。
      // 対照は「宣言を2つ持つ規則を囲む `@` も、宣言を持たない」）。
      if (!body.includes("{")) {
        const declarations = parseDeclarations(body);
        if (declarations.length > 0) {
          out.push({ within, layer, selector: prelude, declarations });
        }
      }
      // **`@media` の中は規則**（暗い配色は `D-11b` でここへ入る）。
      parseRules(body, prelude, layer, out);
    } else {
      out.push({
        within,
        layer,
        selector: prelude,
        declarations: parseDeclarations(body),
      });
    }
    at = close + 1;
  }
}

/** `{` に対応する `}` の位置（**無ければ末尾**）。 */
function matchingBrace(text: string, open: number): number {
  let depth = 0;
  for (let at = open; at < text.length; at += 1) {
    if (text[at] === "{") {
      depth += 1;
    } else if (text[at] === "}") {
      depth -= 1;
      if (depth === 0) {
        return at;
      }
    }
  }
  return text.length;
}

function parseDeclarations(body: string): CssDeclaration[] {
  const found: CssDeclaration[] = [];
  for (const piece of body.split(";")) {
    // 入れ子の規則は宣言ではない（この家では書かない前提 / doc）。
    if (piece.includes("{")) {
      continue;
    }
    const colon = piece.indexOf(":");
    if (colon < 0) {
      continue;
    }
    const property = piece.slice(0, colon).trim();
    const value = piece
      .slice(colon + 1)
      .trim()
      .replace(/\s+/g, " ");
    if (property === "") {
      continue;
    }
    found.push({ property, value });
  }
  return found;
}

/**
 * その規則の、その宣言の値（**同じ綴りが2度出てくれば後の勝ち** / CSS と同じ）。
 * 無ければ `null`。
 *
 * **`@` に囲まれていない規則だけを見る**（[`CssRule`] の `within` の ⚠）。
 */
export function declarationOf(
  rules: readonly CssRule[],
  selector: string,
  property: string,
): string | null {
  let value: string | null = null;
  for (const rule of rules) {
    if (rule.within !== null || rule.selector !== selector) {
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

/** `var(--warn)` から `--warn` を取る。**その形でなければ `null`。** */
export function tokenReference(value: string): string | null {
  const found = /^var\((--[a-z0-9-]+)\)$/.exec(value.trim());
  return found === null ? null : (found[1] as string);
}

/** そのソースが参照している全トークン（重複もそのまま）。 */
export function tokenReferences(source: string): string[] {
  return [...source.matchAll(/var\((--[a-z0-9-]+)\)/g)].map(
    (found) => found[1] as string,
  );
}

/** そのソースが**定義**している全トークン（重複もそのまま）。 */
export function tokenDefinitions(source: string): string[] {
  return cssRules(source).flatMap((rule) =>
    rule.declarations
      .filter((declaration) => declaration.property.startsWith("--"))
      .map((declaration) => declaration.property),
  );
}

/**
 * **外を指している綴り**（`https://host/…` / `//host/…`）。
 *
 * **ホストは点を1つ以上持つものだけを見る**ので、
 * `/src/main.ts`（根からの絶対パス）にも `#/chat`（ハッシュのルータ）にも当たらない。
 *
 * **⚠ 当てる先は入口の文書だけ**（`tests/design-tokens.test.ts`）。
 * `.ts` のコメントには仕様書への URL が実在するので、ソース全体には掛けられない。
 */
const EXTERNAL_URL =
  /(?:\bhttps?:)?\/\/[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+/g;

/** そのソースが外へ取りに行っている先（出てきた順、重複もそのまま）。 */
export function externalReferences(source: string): string[] {
  return [...source.matchAll(EXTERNAL_URL)].map((found) => found[0]);
}

/**
 * **束ねる側が解決する取り込み**（`@import "tailwindcss/theme.css" layer(theme);`）。
 *
 * **⚠ ここが `@import` の唯一の例外である。** 許すのは
 * **引用符の中がパッケージ名だけ**の形 —— `:` も `//` も `url(` も入れない。
 * だから `@import "https://…"` も `@import url(//…)` も、この形に当たらず咎められる。
 *
 * **⚠ 後ろに付けてよいのは `layer(…)` と `source(none)` だけ。**
 * **`source("…")` は許さない** —— **探す根を、走査の根の外へ広げる形**だからで、
 * それは「外から取ってくる」とは別の危険だが、**同じ1行で起きる。**
 *
 * **⚠ 守れていないこと**: **パッケージ名なら何でも許す。**
 * **外から書体を取ってくるパッケージを入れれば、この走査は素通りする。**
 * 見ているのは**この文書が直接どこを指しているか**までで、
 * **依存の中身までは追わない。**
 */
const BUNDLED_IMPORT =
  /@import\s+"[a-z0-9@][a-z0-9@./_-]*"(?:\s+(?:layer\([a-z-]+\)|source\(none\)))*\s*;/g;

/**
 * そのスタイルシートが**外から取ってくる**綴り（出てきた順、重複もそのまま）。
 *
 * **`@font-face` / `http:` `https:` / 外を指す `//host.tld` /
 * 束ねる側が解決しない `@import`。**
 *
 * **⚠ 着手前は `/@import|@font-face|https?:/` を素で当てていた。**
 * **Tailwind の取り込みが `@import` なので、そのままでは成り立たない**
 * （ADR-0040 §決定1）。**咎める先を絞ったぶん、上の ⚠ の穴が開いている。**
 */
export function networkFetches(sheet: string): string[] {
  const rest = sheet.replace(BUNDLED_IMPORT, "");
  const found = [...rest.matchAll(/@import|@font-face|https?:/g)].map(
    (match) => match[0],
  );
  for (const external of externalReferences(rest)) {
    // **`//host.tld`（scheme を書かない形）は上の綴りに当たらない。**
    if (!external.startsWith("http")) {
      found.push(external);
    }
  }
  return found;
}

/** 宣言1件と、その在り処。 */
export interface LocatedDeclaration {
  /** [`ScannedFile`] の `path`。 */
  readonly path: string;
  /** 囲んでいる `@` の並び。**囲まれていなければ `null`**（[`CssRule`]）。 */
  readonly within: string | null;
  readonly selector: string;
  readonly property: string;
  readonly value: string;
}

/**
 * そのファイルのスタイルシートに在る宣言を、**在り処つきで全部**返す。
 *
 * **⚠ `@` に囲まれた規則も返す**（`within` に名前が入る）。
 * 落とすと、暗い配色（`D-11`）が `@media` の中で色を入れ替えたときに、
 * 帳簿がそれを「知らない使用箇所」として数えられなくなる。
 *
 * **⚠ 値は加工しない。** `color: var(--warn)` も
 * `border: 1px solid var(--warn)` も同じ1件として出る ——
 * **`var()` 単体の形だけを見ていたら、短縮形で書いた1箇所が帳簿から漏れる**
 * （D-10 のレビューで実測: `CharacterList.svelte` の `.confirm`）。
 */
export function locatedDeclarations(file: ScannedFile): LocatedDeclaration[] {
  const found: LocatedDeclaration[] = [];
  for (const rule of cssRules(stylesheetOf(file))) {
    for (const declaration of rule.declarations) {
      found.push({
        path: file.path,
        within: rule.within,
        selector: rule.selector,
        property: declaration.property,
        value: declaration.value,
      });
    }
  }
  return found;
}

/** `oklch(L C H)` の**色相**（3つ目の数）。読めなければ `null`。 */
export function oklchHue(value: string): number | null {
  const found = /^oklch\(\s*[\d.]+\s+[\d.]+\s+([\d.]+)\s*\)$/.exec(value.trim());
  if (found === null) {
    return null;
  }
  const hue = Number.parseFloat(found[1] as string);
  return Number.isFinite(hue) ? hue : null;
}

/**
 * `oklch(L C H)` の**明度**（1つ目の数）。読めなければ `null`（作業単位 D-12b-1）。
 *
 * **⚠ 明度の差は WCAG のコントラスト比ではない。** 使う側（`tests/base-layers.test.ts`）は
 * **粗い下限**としてだけ使う。
 */
export function oklchLightness(value: string): number | null {
  const found = /^oklch\(\s*([\d.]+)\s+[\d.]+\s+[\d.]+\s*\)$/.exec(value.trim());
  if (found === null) {
    return null;
  }
  const lightness = Number.parseFloat(found[1] as string);
  return Number.isFinite(lightness) ? lightness : null;
}
