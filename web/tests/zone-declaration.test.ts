/**
 * **時刻を読む画面を開く測定が、時間帯を名乗っていること**（作業単位 F-7d）。
 *
 * # なぜ要るのか
 *
 * F-7c が塞いだのは「`TZ` が実際に効いていること」までで、
 * **各測定ファイルが `readerInZone` / `enterAsReader` を実際に呼んでいることは
 * 誰も見ていなかった**（`tests/zone-control.test.ts` が自分でそう書いていた）。
 *
 * **呼び忘れると、画面は `browserZone()` で機械に訊く。**
 * この機の既定は `Asia/Tokyo` なので、
 * **東京の壁時計で書いた期待値が、この機でだけ緑になる。**
 * **他所の機械では赤**で、しかも**ここでは永久に気付けない。**
 *
 * # 何で塞いだか —— **走査でも帳簿でもない**
 *
 * `tests/support/zone-setup.ts` を `vitest.config.ts` の `setupFiles` に置き、
 * **どの測定ファイルでも「読み手の時間帯」を読んだ瞬間に落ちる**ようにした
 * （名乗っていなければ）。名乗りは [`enterZone`] が付ける ——
 * つまり **`readerInZone` / `enterAsReader` / `enterZone` のどれかを通るまで、
 * 画面は時間帯を読めない。**
 *
 * **⚠ 「どのファイルが対象か」の一覧を持たない。**
 * 対象かどうかは**実際に読んだかどうか**で決まるので、
 * **`App.svelte` 越しに `MemoryList` へ届く形**も、
 * **これから足す画面**も、同じ門を通る。
 * **`import` を書いただけでは名乗ったことにならない**（呼ばないと状態が動かない）。
 *
 * # ⚠ この仕掛けの権威は、このファイル1本しかない
 *
 * **F-7d が置いたものを見張っているのは、ここだけである**
 * （F-7d のレビューが実測: **このファイルを外すと、門の骨抜きも `setupFiles` 外しも 672 全緑**）。
 * **しかも、その大半は1本目の `it` 1本**が持っている。
 *
 * **⚠ このファイルを消すと、F-7d は丸ごと静かに撤回される。**
 * **門は据わったまま何も守らなくなり、どの測定も赤くならない。**
 *
 * **第2の権威を作ろうとすると走査になり、それは `15b` の穴（対照の自己成就）に落ちる。**
 * **1本で妥当と判断した**（レビューも同じ判定）。**だから、ここに手を入れるときは慎重に。**
 *
 * # ⚠ 何を守っていないか（**先に書く**）
 *
 * **⚠ 門が見ているのは `resolvedOptions().timeZone` の欄1つだけである。**
 * **「`Intl.DateTimeFormat` を使えば掛かる」ではない** ——
 * **`timeZone` を渡さない書式の `format` / `formatToParts` は落ちずに機械の既定を返すし、**
 * **`Date` のローカル読みも `Z` の無い綴りの `new Date(...)` も素通りする。**
 * **素通りするものの一覧は `tests/support/ambient-zone.ts` が持つ**（ここに写さない）。
 *
 * そのほか ——
 *
 * * **どの時間帯を名乗るべきかは見ていない。** 名乗ったことしか見ない。
 *   **配線の権威は `tests/timestamp-wiring.test.ts`**（同じフィクスチャを2つの時間帯に通す）。
 * * **`TZ` が効いていることは見ていない。** それは `tests/zone-control.test.ts`。
 * * **明示した時間帯の書式と、`timeZone` 以外の欄は素通しである**（下の1本目の (1)）。
 *   **掛けてはいけない**（`tests/timestamp.test.ts` の書式の節は `TZ` を1バイトも触らない）。
 * * **名乗りが測定ファイル単位であることは、`isolate` に寄りかかっている。**
 *   **既定（`true`）では、白紙化の行を消しても何も赤くならない**（既定では冗長だから）。
 *   **観測できるのは `--no-isolate` のときだけ**である（下の表）。
 * * **画面を mount する形そのものは、ここでは見ていない**（jsdom と `fetch` の差し替えが要る）。
 *   ここが見るのは**画面が呼ぶ関数そのもの**（`browserZone`）である。
 *   **画面越しに効くことは変異検査で測った**（下の表）。
 *
 * # 実測（F-7d の変異検査 / 複製したツリー / 2026-09-02）
 *
 * **写しはここだけが持つ**（`vitest.config.ts` と `tests/support/ambient-zone.ts` は
 * ここを指すだけで、数を1つも書いていない / CLAUDE.md「同じ発見を2箇所に書かない」）。
 *
 * **⚠ 呼び出しを消した変異は、どれも `import` 行を残してある**
 * （＝**「`import` だけ残す」形そのもの**）。**変異後のソースを毎回目で見た。**
 *
 * | 変異 | `forks`（既定） |
 * |---|---|
 * | 無し（納品状態） | **674 全緑 / 40 ファイル**（exit 0） |
 * | `memory-list` から `readerInZone(…)` を消す | **`memory-list` が 29 赤 / 1 緑** |
 * | `stats-panel` から消す | **`stats-panel` が 45 赤**（全滅） |
 * | `memory-superseded` から消す | **`memory-superseded` が 23 赤 / 1 緑** |
 * | `stats-repair-panel` から消す | **`stats-repair-panel` が 17 赤** |
 * | `stat-scale-path` から消す | **`stat-scale-path` が 3 赤** |
 * | `app-routing-stats` から消す | **`app-routing-stats` が 3 赤**（**`App.svelte` 越しでも効く**） |
 * | `app-routing` の呼び出しを**行コメントに変える** | **`app-routing` が 2 赤**（**散文では満たされない**） |
 * | `timestamp-wiring` の `enterAsReader(zone)` を 2 箇所とも消す | **`timestamp-wiring` が 7 赤**（全滅） |
 * | 門の判定を骨抜きに（`state.declared !== null` → `true`） | **ここが 1 赤**（ほかは全部緑） |
 * | `setupFiles` を `vitest.config.ts` から外す | **ここが 1 赤**（ほかは全部緑） |
 * | **未名乗りで `resolvedOptions()` ごと落とす**（欄ではなく呼び出しを止める） | **ここが 1 赤**（**(1-b) が捕まえる**） |
 * | 明示した時間帯まで門に掛ける（素通しの枝を殺す） | **ここが 2 赤**（下の注記） |
 * | **反実仮想**: `ambient-zone` を自分で据える形にして `setupFiles` を外す | **674 全緑**（＝**自己成就**） |
 *
 * **⚠ 「2 赤」の2本目は巻き添えである**（レビューの指摘 / 実測で確認）——
 * **1本目が (1) で落ちて (3) の名乗りに到達しない**ので、
 * **2本目も未名乗りのまま回って落ちる。** **独立した2本ではない。**
 *
 * **⚠ 最後の1行が、`zone-setup.ts` を別ファイルにしてある理由である。**
 * **据え付けを `ambient-zone` の中に書くと、`./zone` を import したファイルが門を据えてしまう** ——
 * **見張り番が自分で門を据えて自分で確かめる形**になり、
 * **配線が消えても緑**になる（しかも**本当に呼び忘れたファイルは `./zone` を import しないので、
 * そもそも門が据わらない**）。
 *
 * # ⚠ 「名乗る前」に置いていなかったせいで、2回とも緑を通った
 *
 * **同じ形の穴を2回踏んでいる**（1回目は着手中に自分で、2回目はレビューが実測）。
 *
 * | 素通しの枝 | 対照の置き場所 | 結果 |
 * |---|---|---|
 * | 明示した時間帯（`timeZone` あり） | ~~名乗った後の2本目~~ → **1本目の (1-a)** | 674 全緑 → **2 赤** |
 * | `timeZone` 以外の欄（`locale`） | ~~名乗った後の2本目~~ → **1本目の (1-b)** | 674 全緑 → **1 赤** |
 *
 * **名乗った後は門が働かないので、素通しと区別が付かない。**
 * **門が「何を通すか」の対照は、必ず名乗る前に置くこと。**
 *
 * # ⚠ `--pool=threads` では
 *
 * **納品状態でも 11 ファイル / 15 赤**（`TZ` が ICU に届かないので `enterZone` が落ちる）。
 * **F-7c の時点は 6 ファイル / 14 赤**だったので、**うるさくなった側に動いている。**
 * **その状態で `memory-list` の呼び出しを消しても、やはり 29 赤**（門は pool に依らない）。
 *
 * # ⚠ `--no-isolate` では（**白紙化の唯一の観測点**）
 *
 * `--no-isolate --max-workers=1 --no-file-parallelism` で測った。
 *
 * | | 結果 |
 * |---|---|
 * | 納品状態 | **1 赤。ただし `zone-control` の1本**（下記。**ここは緑**） |
 * | `resetZoneDeclaration` を何もしない形にする | **ここが 1 赤**（＝**名乗りが前のファイルから漏れた**） |
 *
 * **⚠ 白紙化が観測できるのはここだけである。** 既定（`isolate: true`）では
 * **module ごと作り直されるので冗長**で、消しても 674 全緑のまま。
 *
 * **⚠ そして `--no-isolate` は、そもそもこの測定群の走り方ではない。**
 * **納品状態でも `zone-control` が1本落ちる**が、**理由は門と無関係**である ——
 * **`MACHINE_ZONE` を module の評価時に控える形が、`--no-isolate` では
 * 「前のファイルが残した `TZ`」を機械の既定として控えてしまう。**
 * **F-7d の範囲外なので直していない。**
 * **`vitest.config.ts` に `isolate: true` を書いて固定する道も採らなかった** ——
 * **`pool` と同じで CLI が config に勝つ**（F-7b の実測）ので、固定にはならない。
 *
 * [`enterZone`]: ./support/zone
 */

import { describe, expect, it } from "vitest";

import { browserZone, timeLabel } from "../src/lib/timestamp";
import { enterAsReader, machineZone, probeZoneOtherThan } from "./support/zone";

/** 書式を見るための瞬間（`tests/timestamp.test.ts` と同じ綴り）。 */
const INSTANT = "2026-08-28T14:13:44.983Z";

describe("時間帯を名乗るまで、読み手の時間帯は読めない（作業単位 F-7d）", () => {
  /**
   * **本命。**
   *
   * # ⚠ 1本にまとめてあるのは、順番に寄りかからないため
   *
   * 名乗りは**測定ファイル単位**なので、
   * **一度でも名乗った後の `it` からは「名乗る前」を観測できない。**
   * 別の `it` に割ると、**`it` の実行順という書いていない前提**に寄りかかる。
   *
   * # ⚠ 名乗りを消す口を作らなかった理由
   *
   * 「名乗りを消す」関数を生やせば `it` を割れるが、
   * **その口は、門を素通りさせる口でもある**（CLAUDE.md ——
   * テスト専用の入口を作ると、本番の入口が観測対象から外れる）。
   * **ここは「まだ一度も名乗っていない測定ファイル」という本物の状態を使う。**
   */
  it("名乗る前は落ち、名乗った後は読める", () => {
    // **(1) 門は「読み手の時間帯」の欄だけを見ている。**
    //
    // **⚠ この節を「名乗る前」に置くことが要点である**（着手中とレビューで各1回実測）——
    // **名乗った後に置くと、門を広げる変異が 674 全緑を通る。**
    // 名乗った後は門が働かないので、**素通しと区別が付かない。**

    // (1-a) 時間帯を明示した書式は素通し。
    expect(timeLabel(INSTANT, "Asia/Tokyo"), "(1-a) 明示した時間帯の書式").toBe(
      "2026-08-28 23:13:44 (UTC+09:00)",
    );
    // **`resolvedOptions()` まで読む** —— `timeLabel` は `formatToParts` しか呼ばず、
    // **門は `resolvedOptions().timeZone` に掛かっている**ので、
    // **上の1行だけでは、この枝を一度も通らない。**
    expect(
      new Intl.DateTimeFormat("en-US", {
        timeZone: "America/New_York",
      }).resolvedOptions().timeZone,
      "(1-a) 明示した時間帯は、名乗る前でも読める",
    ).toBe("America/New_York");

    // (1-b) **`timeZone` 以外の欄も素通し。**
    // **時間帯を渡していない書式**で見る —— **門が包むのはこちらだけ**なので、
    // **`resolvedOptions()` ごと落とす形**は、ここでしか捕まらない
    // （レビューの実測: 名乗った後に置いていたので 674 全緑だった）。
    expect(
      new Intl.DateTimeFormat("en-US").resolvedOptions().locale,
      "(1-b) `timeZone` 以外の欄は、名乗る前でも読める",
    ).toBe("en-US");

    // **(2) 読み手の時間帯を読むと、その場で落ちる。**
    // **画面（`MemoryList.svelte` / `StatsPanel.svelte`）が呼ぶのは、この関数そのものである。**
    // **落ちた理由まで見る**（`should_panic` は「落ちた」しか言わない / CLAUDE.md）——
    // 文面が手当ての名前を名指ししていること。
    expect(() => browserZone(), "(2) 名乗る前は落ちる").toThrowError(
      /readerInZone/,
    );
    expect(() => browserZone(), "(2) 文面が手当てを名指しする").toThrowError(
      /enterAsReader/,
    );

    // **(3) 名乗れば読める**（落ちない側の対照）。
    const zone = probeZoneOtherThan(machineZone());
    expect(zone, "(3) 探りが機械の既定と違う").not.toBe(machineZone());
    enterAsReader(zone);

    expect(browserZone(), "(3) 名乗った後は読める").toBe(zone);
  });


  /**
   * **対照: 門は「読み手の時間帯」以外を壊していない。**
   *
   * `Intl.DateTimeFormat` そのものを差し替えているので、
   * **時間帯を明示した書式**と、**`timeZone` 以外の欄**が
   * そのまま読めることを見ておく（ここは名乗った後なので、どちらも通る）。
   */
  it("時間帯を明示した書式は、門の前後で同じ答えを返す", () => {
    const explicit = new Intl.DateTimeFormat("en-US", {
      timeZone: "America/New_York",
      hourCycle: "h23",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
    });

    expect(explicit.resolvedOptions().timeZone).toBe("America/New_York");
    expect(explicit.resolvedOptions().locale).toBe("en-US");
    expect(timeLabel(INSTANT, "America/New_York")).toBe(
      "2026-08-28 10:13:44 (UTC-04:00)",
    );
  });
});
