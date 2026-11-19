/**
 * **「出力形式指示」の欄に差し込む定型文**（作業単位 D-22 / 決定は ADR-0067）。
 *
 * **I/O を持たない。** 画面は `CharacterForm.svelte`、見張り番は
 * `tests/character-form-output-format-snippets.test.ts`。
 *
 * # 文面の出どころ
 *
 * **利用者が長く使ってきた文面をそのまま置いた。測定はしていない**
 * （どの文がどれだけ効くかは測っていない。効き目の主張をここから読まないこと）。
 *
 * # なぜ画面の側にあるのか
 *
 * 本来は言語の束（ADR-0029）に属するものだが、**いまはその束の型がコードに無い。**
 * 束を作るためだけにサーバ・API を広げないよう、**当面は画面だけに置いた。**
 * **束ができたら、この一覧は束へ移すこと**（画面はそこから読む）。
 *
 * # 差し込んだ後は運用者の自由文
 *
 * ボタンは**欄の値を書き換えるだけ**で、差し込んだ文に特別な扱いは無い。
 * サーバに届くのは欄の文字列そのものであり、**差し込んだ文と手で打った文を区別しない**
 * （直しても消しても、ただの `output_format_instruction` として扱われる）。
 */

import { isOverLimit } from "./characterEdit";

/** 定型文。**表示順もこの順。** ボタンの表示はこの文そのもの。 */
export const OUTPUT_FORMAT_SNIPPETS: readonly string[] = [
  "キャラクターの台詞は「」で囲む。",
  "地の文はそのまま出す。",
  "明確な心情描写は（）で囲む。",
  "利用者の台詞を代弁しない。",
];

/**
 * **定型文を差し込んだ後の値。**
 *
 * | いまの値 | 結果 |
 * |---|---|
 * | 空、または空白だけ | **その文だけ**（空白は捨てる） |
 * | 末尾が改行 | そのまま末尾に足す |
 * | それ以外 | 改行1つを挟んで足す |
 *
 * **空白だけの値を捨てるのは、保存の側がそれを「未設定」と読むから**
 * （`characterEdit.ts` の `buildCreate` / `buildUpdate` は `trim() === ""` を空と扱い、
 * `countChars` も `trim` してから数える）。残すと、未設定だった欄に
 * **見えない行頭の空白が付いたまま送られる。**
 */
export function insertSnippet(current: string, snippet: string): string {
  if (current.trim() === "") {
    return snippet;
  }
  if (current.endsWith("\n")) {
    return `${current}${snippet}`;
  }
  return `${current}\n${snippet}`;
}

/**
 * **その文と同じ行が、すでにあるか。** 各行の前後の空白を落として比べる
 * （`\r\n` の `\r` も `trim` で落ちる）。**行の一部に含むだけなら「無い」。**
 */
export function hasSnippetLine(current: string, snippet: string): boolean {
  const wanted = snippet.trim();
  return current.split("\n").some((line) => line.trim() === wanted);
}

/**
 * **その文のボタンを押せるか**（保存中かどうかは画面が見る）。
 *
 * - 同じ行がすでにあれば押せない。
 * - **差し込んだ結果が上限を超えるなら押せない**（ちょうど上限なら押せる）。
 *   **数え方と比べ方は、カウンタ・保存の検査と同じ `isOverLimit` を使う**
 *   （別の数え方を書くと、ボタンとカウンタが食い違う）。
 */
export function canInsertSnippet(current: string, snippet: string): boolean {
  if (hasSnippetLine(current, snippet)) {
    return false;
  }
  return !isOverLimit("output_format_instruction", insertSnippet(current, snippet));
}
