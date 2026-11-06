/**
 * **会話の画面の測定が返す `GET /admin/characters/{handle}/agreements`**（作業単位 A-12c-2）。
 *
 * 会話の画面は、開いたとき（と保存まで届いたターンの後）に約束・決まりの提案を読む
 * （`ProposalPrompt.svelte`）。**台本（呼ばれた順に応答を返す代役）を持つ測定は、この要求に台本を1本食われる**
 * —— `/sessions`（作業単位 D-12c-2）・`/admin/upstream`（D-16 / `upstream-external.ts`）と同じ形なので、
 * 同じ直し方をする。
 *
 * **返すのは「提案も取り決めも無い」一覧**（問いかけは何も出ない）。A-12c-2 より前の測定の前提
 * （画面に問いかけが無い）が変わらない。問いかけを出す測定は `chat-view-proposals.test.ts` が持つ。
 */

/** 取り決めの一覧の口（`/admin/characters/{符号化した識別子}/agreements`）。**GET だけ**を受ける。 */
const AGREEMENTS_PATH = /^\/admin\/characters\/([^/]+)\/agreements$/u;

/** **その URL が取り決めの一覧の要求なら、空の一覧を返す。** それ以外は `null`。 */
export function noProposalsFor(url: string, init?: RequestInit): Promise<Response> | null {
  const found = AGREEMENTS_PATH.exec(url);
  if (found === null || (init?.method ?? "GET") !== "GET") {
    return null;
  }
  return Promise.resolve({
    ok: true,
    status: 200,
    text: () =>
      Promise.resolve(
        JSON.stringify({
          character: decodeURIComponent(found[1] ?? ""),
          agreements: [],
          active_limit: 16,
          last_turn: null,
        }),
      ),
  } as unknown as Response);
}
