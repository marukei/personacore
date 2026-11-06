/**
 * **「いまの上流」の面ごとのカードを読む**（作業単位 D-12b-2 / `src/lib/UpstreamPanel.svelte`）。
 *
 * # なぜ共有するのか
 *
 * 作業単位 D-12b-2 で、起こした上流の状態は**表（見出し「この実行ファイルが上流をどうしているか」）から、
 * 面ごとのカードの1行**（「起こした上流の状態」）へ移った。その行を読む測定は3つのファイルにある
 * （`upstream-panel-start` / `upstream-profiles-panel` / `upstream-panel-cards`）。**読み方を1つにする。**
 *
 * # ⚠ 見るのはカードの見出しと `<dt>` / `<dd>` の文面
 *
 * `data-*` ではない（CLAUDE.md —— 読み手が見るものの権威ではない）。
 * **面の帰属はカードの見出し（`<h3>`）で読む** —— カードの並び順だけに頼らない。
 */

import { expect } from "vitest";

/** 空白を畳んだ文面。**折り返しの位置に寄りかからない。** */
function flat(element: Element | null): string {
  return (element?.textContent ?? "").replace(/\s+/gu, " ").trim();
}

/** **面ごとのカード**（並び順のまま）。 */
export function faceCards(scope: Element): HTMLElement[] {
  return [...scope.querySelectorAll<HTMLElement>("article")];
}

/** カードの見出し（面の名前）。 */
export function cardHeading(card: Element): string {
  return flat(card.querySelector("h3"));
}

/** **見出しで引いたカード。** 1枚だけであること。 */
export function cardNamed(scope: Element, heading: string): HTMLElement {
  const found = faceCards(scope).filter((card) => cardHeading(card) === heading);
  expect(found, `カード「${heading}」が1枚だけあること`).toHaveLength(1);
  return found[0] as HTMLElement;
}

/** カードの `[見出し, 値]` の行（`dl > div` の `<dt>` / `<dd>`）。 */
export function cardRows(card: Element): string[][] {
  return [...card.querySelectorAll("dl > div")].map((row) =>
    [...row.querySelectorAll("dt, dd")].map((cell) => flat(cell)),
  );
}

/** そのカードの、その見出しの行の値。**無ければ落とす。** */
export function cardValue(card: Element, label: string): string {
  const row = cardRows(card).find((cells) => cells[0] === label);
  expect(row, `カード「${cardHeading(card)}」に行「${label}」があること`).toBeDefined();
  return (row as string[])[1] ?? "";
}

/** **起こした上流の状態の行の見出し**（カードの `<dt>` の綴り）。 */
export const LAUNCH_ROW = "起こした上流の状態";

/**
 * **起こした上流の状態を、着手前の表と同じ形で読む** ——
 * `[["項目", 1枚目の見出し, 2枚目の見出し], ["起こした上流の状態", 1枚目の値, 2枚目の値]]`。
 *
 * **1行目の「項目」だけは、表の左上の見出しの写し**（カードには無い）。残りは全部カードの文面。
 * **カードが2枚であること**と、**両方が同じ見出しの行を持つこと**を、ここで確かめる。
 */
export function launchStateRows(scope: Element): string[][] {
  const cards = faceCards(scope);
  expect(cards, "面ごとのカードが2枚あること").toHaveLength(2);
  return [
    ["項目", ...cards.map((card) => cardHeading(card))],
    [LAUNCH_ROW, ...cards.map((card) => cardValue(card, LAUNCH_ROW))],
  ];
}
