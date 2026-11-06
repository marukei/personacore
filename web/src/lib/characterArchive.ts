/**
 * **しまう・完全に消す**の、画面の側の判断（作業単位 A-10 / ADR-0049 §決定1）。
 *
 * **I/O を持たない。** 送るのは `api.ts` の `updateCharacter` / `deleteCharacter`、
 * 出すのは `CharacterList.svelte`（と、会話の相手の選択肢を作る `ChatView.svelte`）。
 *
 * # 2段にしてある理由
 *
 * 「しまう」は戻せる（中身は1行も変わらない）。「完全に消す」は戻せない
 * （会話・記憶・性格値・属性ごと消え、控えも残らない）。**完全に消す口は、しまった一覧にしか置かない**
 * ので、一覧から1回の誤操作で中身を失うことは無い。
 *
 * # しまったかどうかは `archived_at` だけで決める
 *
 * サーバは `GET /admin/characters` で**しまったキャラクターも返す**（時刻つき）。
 * 出し分けるのは画面の仕事で、**判定はここの2つの関数だけ**に置く
 * （一覧の画面と会話の画面で別々に書くと、片方だけが古びる）。
 */

import type { CharacterView } from "./api";

/**
 * **しまってあるか。** 時刻の文字列があるときだけ真。
 *
 * **欄が無い応答は「しまっていない」と読む**（`=== null` で比べない）。欄が無いのは
 * しまう操作を知らない版のサーバ（API の版 `1.0`）で、そこにしまったキャラクターは居ない。
 * `!== null` で比べると、そのサーバのキャラクターが全員しまわれて見える。
 */
function isArchived(character: CharacterView): boolean {
  return typeof character.archived_at === "string";
}

/** しまっていないキャラクター（上の一覧・会話の相手）。**並びは保つ。** */
export function activeCharacters(characters: CharacterView[]): CharacterView[] {
  return characters.filter((character) => !isArchived(character));
}

/** しまったキャラクター（しまった一覧）。**並びは保つ。** */
export function archivedCharacters(
  characters: CharacterView[],
): CharacterView[] {
  return characters.filter(isArchived);
}

/**
 * **完全に消してよいか**（識別子を打ち込ませる確認）。
 *
 * **整えてから比べない。** 前後の空白を落とす・大文字小文字を畳むと、
 * 「打ち間違えたのに消える」が作れる。識別子は小文字・数字・`-`・`_` だけなので、
 * 打つ側が困ることも無い。
 */
export function purgeConfirmed(handle: string, typed: string): boolean {
  return typed === handle;
}
