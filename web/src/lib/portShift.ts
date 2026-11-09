/**
 * **待ち受けのポートをずらして起きたことを、全部の画面の上の帯で言う文**
 * （作業単位 F-11c / ADR-0057 §決定4「画面」）。**I/O を持たない。**
 *
 * `GET /admin/server` の応答（`api.ts` の `ServerView`）から帯の文を組む。**帯（`PortShiftNotice.svelte`）は並べるだけ。**
 * 見張り番は `tests/port-shift.test.ts`（文の全部の枝）と `tests/port-shift-notice.test.ts`（帯と `App.svelte` の配線）。
 *
 * # 帯は何のためにあるか
 *
 * **同梱の画面は実際の URL で開くので、利用者が何かをする必要は無い。** 帯は、**ブックマークや外部の
 * クライアント（チャットクライアントの接続先など）に書いた番号を直すきっかけ**を渡すためにある。
 * だから文は「どの番号で起きたか」に続けて、「元の番号を書いた所があれば直す」ことと、番号を変える場所
 * （「設定」の画面。帯がリンクを添える）を言う。
 *
 * # 元の番号を「なぜ」使わなかったか（2通り）
 *
 * | 元の番号 | 文の頭 |
 * |---|---|
 * | 開けなかった | 「ポート {元} は他のプログラムが使っているなどの理由で使えなかったので、{実際} で起動しました」 |
 * | **llama の面が使う番号として飛ばした**（`listening_port_skipped` が元の番号を含む） | 「ポート {元} は上流プロファイルの llama の面が使う番号なので、{実際} で起動しました」 |
 *
 * **開けなかったときも「他のプログラムが使っていた」と言い切らない**（ADR-0057 §決定4 の追記 2026-11-10）——
 * ずらす失敗には `PermissionDenied`（Windows の除外ポート範囲など）も含まれ、API はその区別を載せない。
 * **飛ばした元の番号に「使えなかった」と言わない**——開こうとしていない
 * （起動ログの警告と同じ分け方 / `src/server/port_search.rs` の `shifted_message`）。
 *
 * **ずらしていなければ `null`**（帯を出さない）。`PERSONACORE_PORT=0` で OS に選ばせた起動もずらしていない
 * （サーバが `listening_port_shifted: false` と言う）。
 */

import type { ServerView } from "./api";

/**
 * **帯の文。** 帯は `lead` の後に「設定」の画面へのリンクを置き、その後に `tail` を置く
 * （「番号は「<リンク>」の画面で変えられます」）。
 */
export interface PortShiftNotice {
  /** リンクの前（「…番号は「」まで）。 */
  readonly lead: string;
  /** リンクの後（「」の画面で…」から）。 */
  readonly tail: string;
}

/** **帯の文を組む。** ずらしていなければ `null`。 */
export function portShiftNotice(view: ServerView): PortShiftNotice | null {
  if (!view.listening_port_shifted) {
    return null;
  }
  const requested = view.listening_port_requested;
  const actual = view.listening_port;
  const why = view.listening_port_skipped.includes(requested)
    ? "上流プロファイルの llama の面が使う番号なので"
    : "他のプログラムが使っているなどの理由で使えなかったので";
  return {
    lead:
      `ポート ${requested} は${why}、${actual} で起動しました。` +
      `ブックマークや外部のクライアントに ${requested} を書いているなら、${actual} に直してください。` +
      `番号は「`,
    tail: "」の画面で変えられます（次の起動から効きます）。",
  };
}
