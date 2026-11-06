/**
 * **門を据える2行**（作業単位 F-7d）。
 *
 * `vitest.config.ts` の `setupFiles` がここを名指しする。
 * **測定ファイルごとに1回**評価され、
 * 「読み手の時間帯を読むには先に名乗ること」を全ファイルに掛ける。
 *
 * # ⚠ なぜ別ファイルなのか
 *
 * **`./ambient-zone` は、読み込まれただけでは門を据えない。**
 * 据える側をここに分けてあるのは、
 * **`tests/zone-declaration.test.ts`（門の見張り番）が
 * 自分で門を据えて自分で確かめる形**を作らないためである
 * （CLAUDE.md「走査の対照は自己成就しうる」と同じ穴）。
 * **実測: 自己据え付け形にして `setupFiles` を外すと 674 全緑になる。**
 *
 * **`setupFiles` からこの行が消えれば、見張り番は赤になる。**
 *
 * # ⚠ 白紙化がここにある理由
 *
 * **`setupFiles` は `--no-isolate` でも測定ファイルごとに再実行される**
 * （F-7d のレビューが実測）。**module の評価に任せると `--no-isolate` で1回しか通らず、
 * 前のファイルの名乗りが残る。**
 * **`vitest.config.ts` に `isolate: true` を書いて固定する道は採らなかった** ——
 * **`pool` と同じで、CLI の `--no-isolate` は config に勝つ**（F-7b の実測）。
 * **固定できないものを固定するより、どちらでも正しく動くほうを採る。**
 */

import { installAmbientZoneGuard, resetZoneDeclaration } from "./ambient-zone";

installAmbientZoneGuard();
resetZoneDeclaration();
