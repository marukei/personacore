/**
 * **時間帯を作り分ける仕掛けそのものが、実際に効いていること**（作業単位 F-7c）。
 *
 * # なぜ要るのか
 *
 * 時刻の測定は `process.env.TZ` を立てて「読み手の時間帯」を作る
 * （`tests/support/zone.ts` / `tests/timestamp-wiring.test.ts` / `tests/timestamp.test.ts`）。
 * **`TZ` はプロセスに効く**ので、**測定ファイルが別プロセスで走ること**に寄りかかっている。
 *
 * `vitest.config.ts` は `pool: "forks"` を名指ししているが、
 * **CLI の `--pool=threads` はそれに勝つ**（F-7b の実測。config の固定では止められない）。
 *
 * # ⚠ 見ているのは pool ではない
 *
 * **「走っている pool が `forks` か」を見張るのは、pool を見張っているだけ**である。
 * **本当の不変条件は「`TZ` が実際に効いていること」** ——
 * 効いていなければ pool が何であれその測定は無意味で、
 * 効いていれば pool が何であれ構わない。
 * **だからここは pool を一切見ない。** 立てた時間帯が解決されるかだけを見る。
 *
 * # ⚠ 機械の既定と違う時間帯でしか、確かめられない
 *
 * **「東京を立てて東京が返った」では何も見ていない** —— この機の既定が東京だからである
 * （**F-7b で3回出た「機械の既定がたまたま同じ」形** / CLAUDE.md の最頻の穴）。
 * ここは**実行時に機械の既定を読んで、それとは違う時間帯を選ぶ**ので、
 * **どの機械で走らせても「本当に動いた」を見ている。**
 *
 * # ⚠ 2つの仕掛けを、**既定の走り（`forks`）で1つずつ**見る
 *
 * F-7c のレビューが実測した ——
 * **`enterZone` の確認を外す変異は既定の走りで 1 赤になるが、
 * 探りの1行を消す変異は `forks` でも `--pool=threads` でも納品状態と完全に同一**だった。
 * **どの利用者も、たまたま機械の既定と違う時間帯を頼んでいただけ。**
 *
 * そこで [`enterAsReader`] を **`beforeAll` の外から呼べる形**にしてもらい、
 * ここが**機械の既定そのものを頼む**（下の2本目の対照）。
 * **それが探りの1行の唯一の見張り番である。**
 *
 * # ⚠ 何を守っていないか（**先に書く**）
 *
 * * **画面の配線は1つも見ていない**（それは `timestamp-wiring.test.ts`）。
 * * **書式も見ていない**（それは `timestamp.test.ts`）。
 * * **`--pool=threads` を禁じてはいない。** 禁じられないので、
 *   **無意味な測定が静かに緑になることだけ**を止めている。
 * * ~~**各測定ファイルが `readerInZone` / `enterAsReader` を実際に呼んでいることは見ていない。**~~
 *   ~~**呼び忘れは、`--pool=threads` で走らせない限り観測されない。**~~
 *   **⚠ 塞いだ**（作業単位 F-7d / 2026-09-02）——
 *   **「読み手の時間帯を読むには、先に名乗ること」の門**を `setupFiles` に据えた。
 *   **権威は `tests/zone-declaration.test.ts`**（変異検査の内訳もそちらが持つ）。
 *   **ここはいまも呼び出しを見ていない** —— 見ているのは門のほうである。
 *
 * # 実測（F-7c の変異検査 / 複製したツリー / 2026-09-02）
 *
 * **⚠ `forks` の列が本体である**（レビューの指摘 —— 既定の走りで観測されない仕掛けは、
 * 「守っている」と数えられない）。
 *
 * | 変異 | `forks`（既定） | `--pool=threads` |
 * |---|---|---|
 * | 無し（納品状態） | **672 全緑** | **6 ファイル赤 / 14 failed / 99 skipped。`timestamp-wiring` は 7 本とも赤** |
 * | `enterZone` の確認を外す | **2 赤**（`対照: 時間帯が動かない環境では…` と `対照: 機械の既定そのものを…`） | `timestamp-wiring` が**着手前と同じ 2 赤 / 5 緑へ丸ごと戻る** |
 * | `enterAsReader` の探りの1行を消す | **1 赤**（`対照: 機械の既定そのものを…` だけ） | `timestamp-wiring` が **4 赤 / 3 緑**（**東京の腕3本が緑に戻る**） |
 * | `PROBE_ZONES` を機械の既定1つ（`["Asia/Tokyo"]`）に | **5 ファイル赤 / 13 failed** | （測っていない） |
 * | `PROBE_ZONES` を2つに減らす | **3 ファイル赤 / 6 failed** | （測っていない） |
 *
 * **⚠ 探りの1行は、F-7c のレビューの時点では `forks` でも `--pool=threads` でも
 * 納品状態と完全に同一だった**（＝どちらでも観測されていなかった）。
 * **いまは既定の走りで 1 赤になる。** それがこのファイルの2本目の対照の存在理由である。
 */

import { afterEach, describe, expect, it } from "vitest";

import { browserZone } from "../src/lib/timestamp";
import {
  enterAsReader,
  enterZone,
  machineZone,
  probeZoneOtherThan,
} from "./support/zone";

/**
 * このファイルを読み込んだ時点の、機械の時間帯。**必ずここへ戻す。**
 *
 * **⚠ [`machineZone`] で読むこと**（作業単位 F-7d）——
 * `Intl.DateTimeFormat().resolvedOptions().timeZone` は**門を通る**ので、
 * **まだ名乗っていないこの時点で呼ぶと落ちる。**
 */
const MACHINE_ZONE = machineZone();

/**
 * **「`TZ` が効かない環境」をその場で作る。**
 *
 * `Intl.DateTimeFormat` を「いつも機械の既定を名乗る」形へ差し替えると、
 * `--pool=threads` と同じ状態になる。
 *
 * **⚠ クラスにしないこと。** `enterZone` は `new` を付けずに呼ぶので、
 * クラスだと「`new` が要る」という**別の理由**で落ちてしまい、
 * 「正しい理由で落ちた」が言えなくなる。
 */
function withFrozenZone(body: () => void): void {
  const real = Intl.DateTimeFormat;
  const frozen = (() => ({
    resolvedOptions: () => ({ timeZone: MACHINE_ZONE }),
  })) as unknown as typeof Intl.DateTimeFormat;

  try {
    (Intl as { DateTimeFormat: unknown }).DateTimeFormat = frozen;
    body();
  } finally {
    (Intl as { DateTimeFormat: unknown }).DateTimeFormat = real;
  }
}

afterEach(() => {
  // **機械の値へ戻す。** `delete` では戻らない（ICU は直前の値を覚えている）。
  process.env["TZ"] = MACHINE_ZONE;
});

describe("時間帯を作り分ける仕掛けが効いている（作業単位 F-7c）", () => {
  /**
   * **本命。**
   *
   * `--pool=threads` で走らせると、`process.env.TZ` は ICU に届かないので
   * **解決される時間帯は機械の既定のまま**になり、ここが落ちる。
   */
  it("機械の既定とは違う時間帯を立てると、実際にその時間帯として解決される", () => {
    const probe = probeZoneOtherThan(MACHINE_ZONE);

    // **⚠ 機械の既定と同じ時間帯で確かめても、何も見ていない。**
    expect(
      probe,
      `探りの時間帯が機械の既定（${MACHINE_ZONE}）と同じでは、何も確かめていない`,
    ).not.toBe(MACHINE_ZONE);

    enterZone(probe);

    expect(Intl.DateTimeFormat().resolvedOptions().timeZone).toBe(probe);
  });

  /**
   * **画面が訊く先まで届いていること。**
   *
   * 画面は `browserZone()` で機械に訊く（`MemoryList.svelte` / `StatsPanel.svelte`）。
   * **仕掛けが効いていても、画面が読む口が別物なら意味が無い。**
   */
  it("画面が訊く先（browserZone）も、立てた時間帯に付いてくる", () => {
    const probe = probeZoneOtherThan(MACHINE_ZONE);
    expect(probe).not.toBe(MACHINE_ZONE);

    enterZone(probe);

    expect(browserZone()).toBe(probe);
  });

  /**
   * **⚠ 2つ以上の時間帯へ移れること。**
   *
   * 1つだけだと、**「1回目に立てた値をそのまま覚えているだけ」**と区別が付かない。
   * 2つ目は**機械の既定とも1つ目とも違う**ものを選ぶ。
   */
  it("続けて別の時間帯へも移れる", () => {
    const first = probeZoneOtherThan(MACHINE_ZONE);
    const second = probeZoneOtherThan(MACHINE_ZONE, first);
    expect(new Set([MACHINE_ZONE, first, second]).size).toBe(3);

    enterZone(first);
    expect(browserZone()).toBe(first);

    enterZone(second);
    expect(browserZone()).toBe(second);
  });

  /**
   * **読み手を作る手順が、機械の既定を頼まれても素通りしないこと。**
   *
   * # ⚠ これが探りの1行の唯一の見張り番である（F-7c のレビューの指摘）
   *
   * **`enterZone` は「立てた時間帯が返ったか」しか見ない**ので、
   * **頼まれた時間帯が機械の既定と一致していると、`TZ` が死んでいても素通りする。**
   * いまの利用者（`timestamp-wiring` / `memory-list` / `memory-superseded` /
   * `stats-panel`）は**たまたま**この機の既定と違う時間帯を頼んでいるだけなので、
   * **探りの1行を消しても、既定の走りでは何も起きなかった**（レビューの実測）。
   *
   * **ここは機械の既定そのものを頼む。** 探りが無ければ、
   * 凍らせた環境でも `enterZone(MACHINE_ZONE)` が通ってしまうので、緑になる。
   */
  it("対照: 機械の既定そのものを頼まれても、仕掛けが動くことを先に確かめる", () => {
    withFrozenZone(() => {
      // **落ちた理由まで見る**（`should_panic` は「落ちた」しか言わない / CLAUDE.md）。
      // 探りは機械の既定以外を選ぶので、文面には**その探りの綴りと機械の既定**が並ぶ。
      const probe = probeZoneOtherThan(MACHINE_ZONE);
      expect(() => enterAsReader(MACHINE_ZONE)).toThrowError(
        new RegExp(`${probe}.*${MACHINE_ZONE}`, "su"),
      );
    });

    // **落ちない側の対照**（凍らせるのをやめたら、機械の既定も頼めること）。
    expect(() => enterAsReader(MACHINE_ZONE)).not.toThrow();
    expect(browserZone()).toBe(MACHINE_ZONE);
  });

  /**
   * **対照: 見張り番そのものが働いていること。**
   *
   * 上の本命3本は**囮を1つも通していない**と、
   * 「そもそも探せていないから緑」と区別が付かない
   * （CLAUDE.md「1件も見つからないから緑」と「そもそも探せていないから緑」）。
   * ここで**`TZ` が効かない環境をその場で作り、同じ判定が赤になること**を見る。
   */
  it("対照: 時間帯が動かない環境では、その場で落ちる", () => {
    const probe = probeZoneOtherThan(MACHINE_ZONE);

    withFrozenZone(() => {
      expect(() => enterZone(probe)).toThrowError(
        new RegExp(`${probe}.*${MACHINE_ZONE}`, "su"),
      );
    });

    // **落ちない側の対照**（差し替えを戻したら通ること）。
    expect(() => enterZone(probe)).not.toThrow();
  });

  /**
   * **対照: 探りの候補が、2つ避けても残ること。**
   *
   * **候補を減らして「機械の既定と同じ時間帯だけ」に狭める形**を落とす。
   * [`enterAsReader`] は**機械の既定と目的の2つ**を避けるので、
   * **2つ避けても残ること**まで見る。
   */
  it("対照: 機械の既定と、もう1つを避けても、探りの候補が残る", () => {
    const first = probeZoneOtherThan(MACHINE_ZONE);
    expect(() => probeZoneOtherThan(MACHINE_ZONE, first)).not.toThrow();
  });
});
