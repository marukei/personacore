/**
 * **dev server の proxy が personacore へ渡す `Origin`**（`dev-proxy.ts` / 作業単位 B-58）。
 *
 * 替えるのは **dev server 自身のオリジン**だけで、外のページの `Origin` は替えない
 * （替えると dev server が外のページの依頼を通す中継になる）。
 *
 * **ポートは全部違える**（dev server・personacore・外のページ）。
 *
 * # 配線は `vite.config.ts` が実際に組んだ設定で見る（B-58 のレビュー R-1）
 *
 * 純粋関数だけを見ても、`vite.config.ts` がそれを呼ばずに「全部替える」処理を
 * 自分で書けば緑のまま通る（レビュアーが実測）。下の `describe` は
 * **`vite.config.ts` の既定の書き出しを呼び、`server.proxy[...].configure` に
 * 偽の proxy（`EventEmitter`）を渡して `proxyReq` を流す。**
 * 宛先のポートは `PERSONACORE_PORT` で渡す（`config.rs` を読ませない）。
 */

import { EventEmitter } from "node:events";

import type { ProxyOptions, UserConfig } from "vite";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { originToForward } from "../dev-proxy";
import viteConfig from "../vite.config";

const TARGET = "http://127.0.0.1:47123";
const DEV_HOST = "127.0.0.1:47555";

describe("originToForward", () => {
  it("dev server 自身のオリジンは宛先の綴りに替える", () => {
    expect(originToForward(`http://${DEV_HOST}`, DEV_HOST, TARGET)).toBe(
      TARGET,
    );
    expect(
      originToForward("http://localhost:47555", "localhost:47555", TARGET),
    ).toBe(TARGET);
  });

  it("外のページの Origin は替えずに渡す（personacore が断る）", () => {
    for (const foreign of [
      "http://evil.example",
      "http://evil.example:47555",
      "http://127.0.0.1:47556",
      "https://127.0.0.1:47555",
      "null",
    ]) {
      expect(originToForward(foreign, DEV_HOST, TARGET)).toBe(foreign);
    }
  });

  it("Origin が無ければ無いまま（足さない）", () => {
    expect(originToForward(undefined, DEV_HOST, TARGET)).toBeUndefined();
  });

  it("Host が無ければ替えない", () => {
    expect(originToForward(`http://${DEV_HOST}`, undefined, TARGET)).toBe(
      `http://${DEV_HOST}`,
    );
  });
});

/** **`PERSONACORE_PORT` で渡す宛先のポート**（既定の 8080 とも `DEV_HOST` とも違う）。 */
const WIRED_PORT = "47321";
const WIRED_TARGET = `http://127.0.0.1:${WIRED_PORT}`;

/** 偽の中継先の要求（`setHeader` の呼び出しを記録する）。 */
function fakeProxyReq(): {
  calls: Array<[string, string]>;
  setHeader: (name: string, value: string) => void;
} {
  const calls: Array<[string, string]> = [];
  return {
    calls,
    setHeader: (name: string, value: string) => {
      calls.push([name, value]);
    },
  };
}

describe("vite.config.ts の proxy の配線", () => {
  let saved: string | undefined;

  beforeEach(() => {
    saved = process.env["PERSONACORE_PORT"];
    process.env["PERSONACORE_PORT"] = WIRED_PORT;
  });

  afterEach(() => {
    if (saved === undefined) {
      delete process.env["PERSONACORE_PORT"];
    } else {
      process.env["PERSONACORE_PORT"] = saved;
    }
  });

  /** `vite.config.ts` が実際に組んだ `server.proxy`。 */
  async function proxies(): Promise<Record<string, ProxyOptions>> {
    expect(typeof viteConfig).toBe("function");
    const resolve = viteConfig as (env: {
      command: "serve";
      mode: string;
    }) => UserConfig | Promise<UserConfig>;
    const config = await resolve({ command: "serve", mode: "development" });
    const proxy = config.server?.proxy;
    expect(proxy).toBeDefined();
    return proxy as Record<string, ProxyOptions>;
  }

  /** その口の `configure` に偽の proxy を通し、`proxyReq` を1回流す。 */
  async function emitThrough(
    prefix: string,
    headers: { origin?: string; host?: string },
  ): Promise<Array<[string, string]>> {
    const options = (await proxies())[prefix];
    expect(options, prefix).toBeDefined();
    const configure = options?.configure;
    expect(typeof configure, prefix).toBe("function");
    const emitter = new EventEmitter();
    configure?.(
      emitter as unknown as Parameters<NonNullable<ProxyOptions["configure"]>>[0],
      options as ProxyOptions,
    );
    expect(emitter.listenerCount("proxyReq"), prefix).toBe(1);
    const proxyReq = fakeProxyReq();
    emitter.emit("proxyReq", proxyReq, { headers });
    return proxyReq.calls;
  }

  it("API の2つの口だけを、Host を替える形で宛先へ中継する", async () => {
    const all = await proxies();
    expect(Object.keys(all).sort()).toEqual(["/admin", "/v1"]);
    for (const prefix of ["/v1", "/admin"]) {
      expect(all[prefix]?.target, prefix).toBe(WIRED_TARGET);
      expect(all[prefix]?.changeOrigin, prefix).toBe(true);
    }
  });

  it("dev server 自身の Origin は宛先の綴りに替える", async () => {
    for (const prefix of ["/v1", "/admin"]) {
      expect(
        await emitThrough(prefix, { origin: `http://${DEV_HOST}`, host: DEV_HOST }),
        prefix,
      ).toEqual([["origin", WIRED_TARGET]]);
    }
  });

  it("外のページの Origin には触らない（personacore が断る）", async () => {
    for (const prefix of ["/v1", "/admin"]) {
      for (const foreign of ["http://evil.example", "null"]) {
        expect(
          await emitThrough(prefix, { origin: foreign, host: DEV_HOST }),
          `${prefix} / ${foreign}`,
        ).toEqual([]);
      }
    }
  });

  it("Origin が無ければ足さない", async () => {
    for (const prefix of ["/v1", "/admin"]) {
      expect(await emitThrough(prefix, { host: DEV_HOST }), prefix).toEqual([]);
    }
  });
});
