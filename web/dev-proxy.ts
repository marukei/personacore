/**
 * **dev server の proxy が personacore へ渡す `Host` / `Origin`**（作業単位 B-58）。
 *
 * personacore は `Host` / `Origin` が自分のものでない要求を 403 で断る
 * （`src/server/host_guard.rs`）。dev server の画面は **dev server のオリジン**
 * （`http://127.0.0.1:5173` 等）から API を叩くので、そのまま中継すると
 * `Host` も `Origin` も personacore のものではなく、**全部の API が 403 になる**
 * （2026-10-01 に実行ファイルと `vite` を起こして実測）。
 *
 * * `Host` は proxy の `changeOrigin: true` が宛先の綴りに替える（[`apiProxy`]）。
 * * `Origin` は `changeOrigin` では替わらないので、[`forwardOwnOrigin`] が替える。
 *
 * # 替えるのは「dev server 自身のオリジン」だけ
 *
 * **外のページの `Origin`（`http://evil.example` 等）は替えずにそのまま渡す** ——
 * personacore が断る。全部を宛先の綴りに替えると、dev server が
 * 外のページの依頼を personacore のものとして通す中継になる
 * （**B-58 が塞いだ穴が、`pnpm dev` の間だけ開き直る**）。
 *
 * `Host` は dev server 自身が先に見ている（Vite の `server.allowedHosts`。
 * 外の名前の `Host` は dev server が 403 で断る——同じ回に実測）ので、
 * 「dev server 自身のオリジン」は `http://` ＋ 届いた `Host` で作ってよい。
 *
 * # 配線もここに置く（B-58 のレビュー R-1）
 *
 * **`vite.config.ts` は [`apiProxy`] を呼ぶだけ**にしてある。`proxyReq` を扱う処理を
 * `vite.config.ts` に書くと、そこを壊しても誰も赤くならなかった（レビュアーが実測）。
 * 見張り番は `tests/dev-proxy.test.ts` —— **`vite.config.ts` が実際に組んだ
 * `server.proxy[...].configure` に偽の proxy を通す。**
 */

import type { ProxyOptions } from "vite";

/** `configure` が受け取る proxy（Vite の型から取る。写さない）。 */
type ProxyServer = Parameters<NonNullable<ProxyOptions["configure"]>>[0];

/** **中継の先へ渡す `Origin`。純粋関数。** 替えないなら元の値（無ければ `undefined`）。 */
export function originToForward(
  origin: string | undefined,
  host: string | undefined,
  target: string,
): string | undefined {
  if (origin === undefined || host === undefined) {
    return origin;
  }
  return origin === `http://${host}` ? target : origin;
}

/**
 * **proxy に「dev server 自身のオリジンだけを宛先の綴りに替える」処理を付ける。**
 *
 * 替えないとき（外のページ・`Origin` 無し）は `setHeader` を呼ばない
 * ——届いた値がそのまま中継される。
 */
export function forwardOwnOrigin(target: string): (proxy: ProxyServer) => void {
  return (proxy) => {
    proxy.on("proxyReq", (proxyReq, req) => {
      const origin = req.headers.origin;
      const forwarded = originToForward(origin, req.headers.host, target);
      if (forwarded !== undefined && forwarded !== origin) {
        proxyReq.setHeader("origin", forwarded);
      }
    });
  };
}

/** **API の口1つぶんの proxy の設定**（`vite.config.ts` が前置きごとに載せる）。 */
export function apiProxy(target: string): ProxyOptions {
  return {
    target,
    changeOrigin: true,
    configure: forwardOwnOrigin(target),
  };
}
