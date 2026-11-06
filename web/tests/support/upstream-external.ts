/**
 * **会話の画面の測定が返す `GET /admin/upstream`**（作業単位 D-16）。
 *
 * 会話の画面は開くと本文の面の状態を読む（`ChatView.svelte` の `readUpstream`）。
 * **台本（呼ばれた順に応答を返す代役）を持つ測定は、この要求に台本を1本食われる**
 * ——`/sessions` を足したとき（作業単位 D-12c-2）と同じ形なので、同じ直し方をする。
 *
 * **返すのは「外の上流」**（どちらの面も `external`）。**送信を止めない状態**なので、
 * D-16 より前の測定の前提（送れば飛ぶ）が変わらない。止める・戻すの測定は
 * `chat-view-body-face.test.ts` が持つ。
 */

/** 外の上流の1面。 */
function externalFace(face: string): unknown {
  return {
    face,
    endpoint: face === "main" ? "http://127.0.0.1:1234" : "http://127.0.0.1:1235",
    model: face === "main" ? "main-model" : "sub-model",
    answered_name: null,
    queried_context_tokens: null,
    answered_by: null,
    measured: { checked: 0, matched: 0, drifted_other: 0, drifted_absent: 0, unanswered: 0 },
    launch: { state: "external", since: null, answered_name: null, reason: null, log: null },
  };
}

/** **その URL が本文の面の状態の要求なら、外の上流を返す。** それ以外は `null`。 */
export function externalUpstreamFor(url: string): Promise<Response> | null {
  if (url !== "/admin/upstream") {
    return null;
  }
  return Promise.resolve({
    ok: true,
    status: 200,
    text: () =>
      Promise.resolve(
        JSON.stringify({
          main: externalFace("main"),
          sub: externalFace("sub"),
          profile_sample: null,
          selected_profile: null,
        }),
      ),
  } as unknown as Response);
}
