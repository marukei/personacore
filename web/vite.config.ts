import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";

import { apiProxy } from "./dev-proxy";

/**
 * axum 側の待ち受けポートの既定値が書いてある場所。
 *
 * **ここから読む。写さない。** 写すと Rust 側で既定を変えたときに
 * dev server の proxy だけが黙って古びて、「開発中だけ API が 404 になる」という
 * 原因から遠い形で現れる。
 */
const CONFIG_RS = fileURLToPath(
  new URL("../src/server/config.rs", import.meta.url),
);

/** `pub const DEFAULT_PORT: u16 = 8080;` を拾う。 */
const DEFAULT_PORT_PATTERN = /pub const DEFAULT_PORT: u16 = (\d+);/;

/**
 * proxy 先のポート。
 *
 * `PERSONACORE_PORT` を立てて起動しているなら、それに合わせる
 * （Rust 側の [`ENV_PORT`] と同じ綴り）。無ければ `config.rs` の既定値。
 *
 * **読めなければ投げる。** ここで 8080 へ黙って落ちると、それは
 * 「読んでいる」ではなく「写している」のと同じになる。
 */
function backendPort(): number {
  const override = process.env["PERSONACORE_PORT"]?.trim();
  if (override !== undefined && override !== "") {
    const parsed = Number.parseInt(override, 10);
    if (!Number.isInteger(parsed) || parsed < 0 || parsed > 65535) {
      throw new Error(
        `PERSONACORE_PORT がポート番号として読めません: ${override}`,
      );
    }
    return parsed;
  }

  const source = readFileSync(CONFIG_RS, "utf8");
  const found = DEFAULT_PORT_PATTERN.exec(source);
  if (found === null || found[1] === undefined) {
    throw new Error(
      `${CONFIG_RS} から DEFAULT_PORT を読めませんでした。` +
        "綴りが変わったなら vite.config.ts の正規表現も合わせてください",
    );
  }
  return Number.parseInt(found[1], 10);
}

export default defineConfig(() => {
  const target = `http://127.0.0.1:${backendPort()}`;

  return {
    // **Tailwind v4 は束ねる側の道具**（ADR-0040 §決定1 / 作業単位 D-11a）。
    // **実行時のコードは増えない** —— `src/app.css` の取り込みを読んで、
    // **実際に使われたクラスだけ**を出す。
    //
    // **⚠ 単体テストの器（`vitest.config.ts`）には入れていない。**
    // テストは部品を直接 `mount` するので CSS を読まない（`src/app.css` の冒頭）。
    plugins: [tailwindcss(), svelte()],
    build: {
      // **`server::config::DEFAULT_WEB_DIR` と同じ綴り**（`web/dist`）。
      // 片方だけ変えると UI が黙って出なくなる。
      //
      // **⚠ 次の行は Rust 側のテストが読む**
      // （`config.rs` の `the_default_web_dir_is_where_vite_actually_builds`）。
      // **鍵の直後に文字列リテラルを直接書く形**を崩すと——変数に逃がすなど——
      // あちらが「拾えない」と言って赤くなる。走査の側も一緒に直すこと。
      // **⚠ この注記に鍵の綴りを書かないこと。**
      // 走査は最初の一致を採るので、注記の方を拾ってしまう（実際に一度踏んだ）。
      outDir: "dist",
      emptyOutDir: true,
    },
    server: {
      // **ループバックのみ**（Rust 側の絶対制約に合わせる。dev server も外へ出さない）。
      host: "127.0.0.1",
      proxy: {
        // 本番は axum が同一オリジンで配るので proxy は要らない（ADR-0031 §3）。
        // dev server のときだけ、同じ形に見せる。
        //
        // **personacore の `Host` / `Origin` の検査（作業単位 B-58）を通る形で中継する。**
        // `Host` と「dev server 自身の」`Origin` だけを宛先の綴りに替える（`dev-proxy.ts`）。
        // **ここに `proxyReq` の処理を書かないこと**——書くと見張り番
        // （`tests/dev-proxy.test.ts`）の外に出る。
        "/v1": apiProxy(target),
        "/admin": apiProxy(target),
      },
    },
  };
});
