/**
 * **測定を、名指しした時間帯の読み手として走らせる**（作業単位 F-7b →
 * **F-7c で「効いていること」を確かめるようにした**）。
 *
 * # なぜ要るのか
 *
 * F-7 は**地域化しない**と決めており、理由の2つ目はこうだった ——
 * **「その綴りを見張る検査が、走らせた機械の時間帯でしか成立しなくなる。」**
 * **これは正しい懸念**で、地域化した以上、放っておけばそのとおりになる
 * （**この機の既定は `Asia/Tokyo`**。他所で走らせた瞬間に全部赤くなる）。
 *
 * **書式そのものは時間帯を引数で受け取る**ので、
 * `timestamp.test.ts` はここを使わずに済んでいる。
 * **ここが要るのは「画面を mount する測定」だけ** ——
 * 画面は `browserZone()` で機械に訊くので、**訊く先を名指ししないと決まらない。**
 *
 * # `process.env.TZ` を触ってよい理由
 *
 * **測定ファイルはフォークされた別プロセスで走る**
 * （`vitest.config.ts` が `pool: "forks"` を名指ししている /
 * 実測: 2 ファイルを同時に走らせると別々の pid）。
 * **他のファイルへ漏れない。** それでも後始末はする。
 *
 * # ⚠ **`TZ` が効かない環境がある**（作業単位 F-7c）
 *
 * **`process.env.TZ` はワーカスレッドの中では ICU に届かない。**
 * `vitest.config.ts` は `pool: "forks"` を名指ししているが、
 * **CLI の `--pool=threads` はそれに勝つ**（F-7b の実測）。
 *
 * **⚠ そのとき、この仕掛けは「静かに」壊れる。**
 * `TZ` を立てても機械の既定のままなので、
 * **機械の既定とたまたま同じ時間帯を名指しした腕だけが緑で残る**
 * （**実測の内訳は `tests/timestamp-wiring.test.ts` が持つ。ここに写さない**）。
 *
 * **本当の不変条件は「pool が `forks` であること」ではなく
 * 「`TZ` が実際に効いていること」**である ——
 * 効いていれば pool が何であれ構わないし、効いていなければ pool が何であれ
 * その測定は無意味になる。
 *
 * # ⚠ 仕掛けは2つ。**どちらも既定の走り（`forks`）で観測されている**
 *
 * | 仕掛け | 何をするか | 既定の走りで見ている見張り番 |
 * |---|---|---|
 * | [`enterZone`] | 立てた時間帯が**実際に解決された**ことを確かめる | `対照: 時間帯が動かない環境では、その場で落ちる` |
 * | [`enterAsReader`] | **機械の既定とも目的とも違う時間帯を先に立てて**、仕掛けが動くこと自体を確かめてから移る | `対照: 機械の既定そのものを頼まれても、仕掛けが動くことを先に確かめる` |
 *
 * **⚠ [`enterAsReader`] が要る理由は「東京を立てて東京が返る」では何も見ていないから。**
 * **目的の時間帯が機械の既定と一致した瞬間、[`enterZone`] は素通りする。**
 * **先に別の時間帯を通せば、どの機械で走らせても成り立つ。**
 *
 * **⚠ そして、その主張自体に見張り番が要る**（F-7c のレビューの指摘）。
 * **探りの1行を消しても、いまの利用者はどれもこの機の既定と一致しないので、
 * 既定の走りでは何も起きない。**
 * だから [`enterAsReader`] は **`beforeAll` の外から直接呼べる形**にしてあり、
 * `tests/zone-control.test.ts` が**機械の既定そのものを頼んで**確かめる。
 *
 * **機械に依らない権威は `tests/zone-control.test.ts`。**
 *
 * # ⚠ これは「配線の見張り番」ではない
 *
 * ここを使う測定は**1つの時間帯でしか回らない**ので、
 * **その時間帯を決め打つ変異を捕まえられない**（`B-7` の形 / CLAUDE.md）。
 * **配線の権威は `tests/timestamp-wiring.test.ts`** ——
 * **同じフィクスチャを2つの時間帯に通す**ので、どの1つの決め打ちも落ちる。
 *
 * ここの役目は**期待値を機械から切り離すこと**だけである。
 * **F-7c で足したのは「切り離せたことの確認」**であって、配線は依然として見ていない。
 *
 * # ⚠ 呼び忘れは、もう静かではない（作業単位 F-7d）
 *
 * **F-7c の時点では、ここを呼び忘れても何も起きなかった**
 * （画面は機械の既定で描かれ、この機の既定は東京なので緑のまま残る）。
 * **いまは落ちる** —— [`enterZone`] が `./ambient-zone` の門に名乗りを付け、
 * **名乗るまで `browserZone()` が読めない。**
 * **機械に依らない権威は `tests/zone-declaration.test.ts`。**
 *
 * # ⚠ 別名の時間帯を渡さないこと
 *
 * ICU は既定の時間帯を旧名へ正規化する ——
 * **`TZ=Asia/Kolkata` を立てて `resolvedOptions()` を読むと `Asia/Calcutta`**
 * が返る（F-7b の着手中に実測）。**画面の表示には影響しない**が、
 * `browserZone()` の返り値を直接見る測定では実装が正しいのに赤くなる。
 * **F-7c 以降は [`enterZone`] がその場で落とす**ので、
 * 「実装が壊れた」ではなく「別名を渡した」と読めるようになっている。
 */

import { afterAll, beforeAll } from "vitest";

import { declareZone, machineZone } from "./ambient-zone";

/**
 * **機械が名乗っている時間帯**（門を通さずに読む / 作業単位 F-7d）。
 *
 * **これは名乗りにはならない。** 呼んでも、画面が `browserZone()` を呼べば落ちる。
 */
export { machineZone };

/**
 * 探りに使う時間帯の候補。
 *
 * **3つあることが要点** —— 「機械の既定」と「目的の時間帯」の2つを避けても、
 * **必ず1つ残る。** どの機械で走らせても、仕掛けが動くことを確かめられる。
 *
 * どれも **ICU が旧名へ正規化しない綴り**である
 * （`Asia/Kolkata` → `Asia/Calcutta` のような形にならない）。
 * **この3つは、既にこのリポジトリの測定が使っている時間帯でもある**
 * （`timestamp-wiring` / `memory-list` / `memory-superseded` / `stats-panel`）。
 */
const PROBE_ZONES = [
  "Asia/Tokyo",
  "America/New_York",
  "Australia/Adelaide",
] as const;

/** いまこのプロセスが名乗っている時間帯。 */
function resolvedZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

/**
 * **この module を読み込んだ時点の、機械の時間帯。**
 *
 * **import は巻き上げられる**ので、どの測定ファイルでも
 * **`TZ` が1度も触られていない状態**で決まる。
 *
 * **⚠ [`machineZone`] で読むこと**（作業単位 F-7d）——
 * `resolvedZone()` は**門を通る**ので、
 * **まだ名乗っていないこの時点で呼ぶと落ちる。**
 */
const MACHINE_ZONE = machineZone();

/**
 * 挙げた時間帯の**どれとも違う**候補を1つ返す。
 *
 * **「機械の既定と同じ時間帯を立てて、同じ時間帯が返った」では何も見ていない**
 * ——この関数はその形を作れないようにするためにある。
 *
 * @param avoid 避ける時間帯（機械の既定と、目的の時間帯）。
 */
export function probeZoneOtherThan(...avoid: readonly string[]): string {
  const found = PROBE_ZONES.find((zone) => !avoid.includes(zone));
  if (found === undefined) {
    throw new Error(
      `避けた時間帯（${avoid.join(" / ")}）以外の候補が残っていません。` +
        `候補は ${PROBE_ZONES.join(" / ")} です。` +
        "候補を減らすと、機械の既定と同じ時間帯でしか確かめられなくなり、" +
        "この仕掛けは何も見ていないのと同じになります。",
    );
  }
  return found;
}

/**
 * その時間帯へ移る。**移れなかったら落ちる。**
 *
 * **⚠ ここが F-7c の本体の半分である。**
 * `process.env.TZ` を立てるだけの形だと、`TZ` が効かない環境では
 * **黙って機械の既定のまま**測定が続く。
 *
 * **⚠ ただし、これだけでは足りない。**
 * **目的の時間帯が機械の既定と一致していると素通りする**ので、
 * 読み手を作るときは [`enterAsReader`] を使うこと。
 *
 * @param zone 時間帯データベースの識別子（`America/New_York` 等）。
 * @throws 立てた時間帯が実際には解決されなかったとき。
 */
export function enterZone(zone: string): void {
  process.env["TZ"] = zone;
  // **ここが唯一の名乗り**（作業単位 F-7d / `./ambient-zone`）。
  // **確かめる前に名乗る** —— 下の `resolvedZone()` は門を通るので、
  // 名乗る前に読むと「名乗っていない」で落ちてしまい、
  // **`TZ` が効いていない**という本当の理由が見えなくなる。
  // **素通りにはならない** —— 名乗った直後に、その時間帯になったかを確かめている。
  // **⚠ 落ちても名乗りは残る。** これは意図した振る舞いで、
  // **`try/catch` で包む人が誤解しやすい点である** ——
  // 「立て損ねたのだから名乗りも消えるはず」と読まないこと。
  // 消すと、`enterZone` が落ちた測定ファイルは
  // **そのあと「名乗っていない」でも落ちる**ようになり、**本当の理由が2つに割れる。**
  declareZone(zone);
  const resolved = resolvedZone();
  if (resolved !== zone) {
    throw new Error(
      `時間帯を ${zone} に立てたのに、解決されたのは ${resolved} でした。` +
        "この測定は「読み手の時間帯」を作れていないので、結果に意味がありません。" +
        "考えられるのは2つです —— " +
        "(1) `--pool=threads` で走らせている" +
        "（`process.env.TZ` はワーカスレッドでは ICU に届かない。" +
        '`vitest.config.ts` の `pool: "forks"` は CLI に負ける）。' +
        "(2) 別名の時間帯を渡した（`Asia/Kolkata` は `Asia/Calcutta` に正規化される）。",
    );
  }
}

/**
 * **その時間帯の読み手になる。** 仕掛けが動くことを先に確かめてから移る。
 *
 * **⚠ ここが F-7c の本体のもう半分である。**
 *
 * 先に**機械の既定とも `zone` とも違う**時間帯を一度立てる。
 * これを飛ばすと、**機械の既定がたまたま `zone` と同じ機械では、
 * `TZ` が効いていなくても緑**になる ——
 * **F-7c より前は、この機（東京）で `Asia/Tokyo` を頼む腕が全部そうだった。**
 *
 * **`beforeAll` の外からも呼べる形にしてある**のは、
 * **この探りの1行を見張れるようにするため**（F-7c のレビューの指摘）——
 * `tests/zone-control.test.ts` が**機械の既定そのものを頼んで**、
 * それでも落ちることを確かめる。
 *
 * @param zone 時間帯データベースの識別子。
 * @throws 探りの時間帯へ移れなかったとき、または `zone` へ移れなかったとき。
 */
export function enterAsReader(zone: string): void {
  enterZone(probeZoneOtherThan(MACHINE_ZONE, zone));
  enterZone(zone);
}

/** 機械の値へ戻す。**`delete process.env.TZ` では戻らない**（ICU が直前の値を覚えている）。 */
export function leaveZone(): void {
  process.env["TZ"] = MACHINE_ZONE;
}

/**
 * この測定ファイルを、その時間帯の読み手として走らせる。
 *
 * @param zone 時間帯データベースの識別子（`America/New_York` 等）。
 */
export function readerInZone(zone: string): void {
  beforeAll(() => {
    enterAsReader(zone);
  });

  afterAll(() => {
    leaveZone();
  });
}
