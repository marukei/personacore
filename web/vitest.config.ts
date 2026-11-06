/// <reference types="vitest/config" />
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vitest/config";

/**
 * **単体テストの器**（ADR-0031 §5 / 決めた同じ日に改めた分）。
 *
 * # `vite.config.ts` と別に持っている
 *
 * あちらは **dev server の proxy と `outDir`** のためにあり、
 * `../src/server/config.rs` を読む。**テストにはどちらも要らない**ので、
 * 読ませない（読ませると、Rust 側のファイル配置がテストの前提に混ざる）。
 *
 * # ブラウザは起動しない
 *
 * **jsdom だけ。Playwright は入れない**——CDP を使うので、この機の EDR が
 * クッキー窃取として検知してプロセスを kill する（CLAUDE.md / ADR-0031 §5）。
 *
 * # 既定は `node`。jsdom が要る1本だけがファイル冒頭で名乗る
 *
 * `// @vitest-environment jsdom` を書いた測定だけが jsdom で走る。
 * 全部を jsdom にすると、**Node の `Response` や `ReadableStream` が
 * jsdom の実装に差し替わっているのかどうかが測定ごとに変わる**ので、
 * 必要な1本だけに閉じる。
 */
export default defineConfig({
  // `.svelte` を読めるようにする（`ChatView.svelte` を mount する測定がある）。
  plugins: [svelte()],
  // Svelte 5 の `mount` はブラウザ側の実装にしか無い。
  // これを外すと SSR 用の実装が解決され、`mount` が使えない。
  resolve: { conditions: ["browser"] },
  test: {
    include: ["tests/**/*.test.ts"],
    environment: "node",
    // **⚠ `forks` を外さないこと**（作業単位 F-7b / レビューの実測）。
    //
    // **時刻の測定は `process.env.TZ` を立てて「読み手の時間帯」を作る**
    // （`tests/support/zone.ts` / `tests/timestamp-wiring.test.ts` /
    // `tests/timestamp.test.ts`）。**`TZ` はプロセスに効く**ので、
    // **測定ファイルが別プロセスで走ることに寄りかかっている。**
    //
    // これは vitest 4 の既定でもあるが、**既定に寄りかかると黙って壊れる**ので
    // ここで名指しする。
    //
    // **⚠ 何を塞いで、何を塞がないか**（F-7b の是正で実測）:
    //
    // | 状態 | 結果 |
    // |---|---|
    // | ここが `forks`（いま） | **666 全緑** |
    // | ここを `threads` にする | **5 ファイル 17 赤**（＝この行は本当に効いている） |
    // | ここは `forks`・**CLI で `--pool=threads`** | **17 赤**（**CLI が勝つ**） |
    //
    // **塞げるのは「既定が変わること」までで、CLI の上書きは塞げない。**
    //
    // **⚠ かつては「うるさく落ちるから安全」でもなかった。**
    // **赤の山の中に「偶然の緑」が混ざっていた**
    // ——**内訳は `tests/timestamp-wiring.test.ts` の冒頭が持つ。ここに写さない**
    // （同じ発見を2箇所に書くと、片方が黙って古びる / CLAUDE.md）。
    //
    // **⚠ そこは作業単位 F-7c で塞いだ。ただし、ここではない。**
    // **見張るべきは pool ではなく「`TZ` が実際に効いていること」**である ——
    // 効いていなければ pool が何であれ測定は無意味で、
    // 効いていれば pool が何であれ構わない。
    // **仕掛けは `tests/support/zone.ts` の `enterZone` と `enterAsReader`**、
    // **機械に依らない権威は `tests/zone-control.test.ts`。**
    // **この行はいまも要る** —— 既定が変わることは、ここでしか止められない。
    pool: "forks",
    // **⚠ この行を外すと、時刻を読む画面の測定が「機械の既定の時間帯」で
    // 静かに回るようになる**（作業単位 F-7d）。
    //
    // 据えているのは**「読み手の時間帯を読むには、先に名乗ること」の門**
    // （`tests/support/ambient-zone.ts`）。**名乗りは `enterZone` が付ける**ので、
    // **`readerInZone` / `enterAsReader` / `enterZone` のどれかを通るまで、
    // 画面は時間帯を読めない。**
    //
    // **⚠ ここが消えても、他のどの測定も赤くならない**
    // ——**唯一の見張り番は `tests/zone-declaration.test.ts`**（1本目が赤になる）。
    // **内訳はそちらが持つ。ここに写さない。**
    setupFiles: ["./tests/support/zone-setup.ts"],
  },
});
