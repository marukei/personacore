/**
 * **「読み手の時間帯」を読むには、先に名乗ること**（作業単位 F-7d）。
 *
 * # なぜ要るのか
 *
 * 画面は `browserZone()`（`src/lib/timestamp.ts`）で**機械に時間帯を訊く。**
 * 測定ファイルがその画面を開くときは、
 * `readerInZone` / `enterAsReader`（`./zone`）で**読み手の時間帯を名乗る**約束だった。
 *
 * **その約束を、誰も見ていなかった**（`tests/zone-control.test.ts` が自分でそう書いていた）——
 * 呼び忘れても画面は動く。**機械の既定で動く。**
 * この機の既定は `Asia/Tokyo` なので、
 * **東京の壁時計で書いた期待値が、この機でだけ緑になる。**
 *
 * # 何をするか
 *
 * `Intl.DateTimeFormat` を差し替え、
 * **`timeZone` を渡さずに作った書式の `resolvedOptions().timeZone`** を読むと
 * **名乗るまで落ちる**ようにする。名乗りは [`declareZone`] が付け、
 * それを呼ぶのは `./zone` の `enterZone` だけである。
 *
 * **⚠ 見ているのは `Intl.DateTimeFormat` ではなく、`resolvedOptions().timeZone` の欄1つ。**
 * **この線の引き方が守る範囲は狭い。** 下の「何を守っていないか」を必ず読むこと。
 *
 * **⚠ 一覧を持たない。** 「どの測定ファイルが対象か」は
 * **実際に読んだかどうか**で決まるので、
 * **`App.svelte` 越しに `MemoryList` へ届く形**も、**これから足す画面**も、同じ門を通る。
 * **`import` を書いただけでは名乗ったことにならない**（呼ばないと状態が動かない）。
 *
 * # ⚠ 何を守っていないか（**先に書く**）
 *
 * **⚠ 門が見ているのは `resolvedOptions().timeZone` の欄1つだけである。**
 * **「`Intl.DateTimeFormat` を使えば掛かる」ではない。** 素通りするものを並べる ——
 *
 * * **⚠ `timeZone` を渡さない書式の `format` / `formatToParts`。**
 *   **落ちずに、機械の既定の壁時計を返す**（実測: `format` が `08/28/2026, 23:13`）。
 *   **これがいちばん危ない** —— **画面に時刻を出すいちばん自然な書き方**であり、
 *   **`src/lib/timestamp.ts` の `wallClockIn` がやっていることそのもの**
 *   （あちらは `timeZone` を必ず渡すので掛からない）。
 *   **`timeZone` を書き忘れた瞬間、F-7d が塞いだのと同じ穴が開く。**
 * * **⚠ `Date` のローカル読み** —— `getHours` 族 / `getTimezoneOffset` /
 *   `toString` / `toDateString` / `toTimeString`。**どれも素通りする。**
 * * **⚠ `Z` の無い綴りを `new Date(...)` に渡す形。**
 *   **ローカル時刻として解釈される**ので、門を一度も通らずに機械の既定へ寄りかかる。
 * * **`Date.prototype.toLocaleString` 族。** 内部で時間帯を解決するので素通りする。
 * * **`Intl.DateTimeFormat.prototype.resolvedOptions.call(fmt)` も素通り。**
 *   **門はインスタンスの own property を差し替えている**ため。作為的なので優先度は低い。
 *
 * **いま `src/**` の ambient な読み手は `browserZone` の1つだけ**である
 * （F-7d のレビューが、TypeScript の `createScanner` で
 * 文字列・テンプレート・コメント・正規表現の中を数えない走査を書いて全 20 ファイルを数えた）。
 * **上のどれかを足したら、この門には掛からない。**
 *
 * そのほか ——
 *
 * * **時間帯を明示した書式は素通しである**（`timeLabel(at, zone)` の側）。
 *   **掛けてはいけない** —— `tests/timestamp.test.ts` の書式の節は
 *   `TZ` を1バイトも触らずに回る、というのが F-7b の要点だった。
 * * **`timeZone` 以外の欄（`locale` など）も素通しである。**
 *   **止めると「時間帯と関係の無い理由で落ちた」になり、正しい理由が言えなくなる。**
 * * **名乗りが測定ファイル単位であることは、`isolate` に寄りかかっている。**
 *   既定（`true`）は module ごと作り直すので白紙から始まる。
 *   **`isolate: false` では [`resetZoneDeclaration`] が `./zone-setup` から戻す。**
 *   **⚠ その白紙化は既定の走りでは観測できない**（既定では冗長なので）。
 * * **どの時間帯を名乗るべきかは見ていない。** 名乗ったことしか見ない。
 *   **配線の権威は `tests/timestamp-wiring.test.ts`。**
 * * **据え付けは `vitest.config.ts` の `setupFiles`**（`./zone-setup`）。
 *   **この module は、読み込まれただけでは何もしない** ——
 *   **さもないと「`./zone` を import した測定ファイルだけが門を持つ」ことになり、
 *   見張り番（`tests/zone-declaration.test.ts`）が自分で門を据えて自分で確かめる形**
 *   になってしまう（CLAUDE.md「走査の対照は自己成就しうる」と同じ穴）。
 *   **実測: 自己据え付け形にして `setupFiles` を外すと 674 全緑。**
 *
 * **機械に依らない権威は `tests/zone-declaration.test.ts`。**
 */

/**
 * 名乗っていない測定ファイルが読み手の時間帯を読んだときの文面。
 *
 * **手当ての名前を必ず含める** —— これを読む人は、
 * 「なぜ落ちたか」ではなく「何を書けばいいか」を知りたい。
 */
const NOT_DECLARED =
  "この測定ファイルは「読み手の時間帯」を読みました" +
  "（画面が `browserZone()` で機械に訊いています）。" +
  "けれど、この測定ファイルは時間帯を名乗っていません。" +
  "そのままだと、期待値は走らせた機械の時間帯でしか成立しません" +
  "（この機の既定は Asia/Tokyo。他所の機械では赤になり、ここでは気付けません）。" +
  "手当ては1行です —— 測定ファイルの冒頭で " +
  '`readerInZone("America/New_York")` を呼ぶか、' +
  "画面を開く直前に `enterAsReader(zone)` を呼んでください" +
  "（どちらも `tests/support/zone.ts`）。";

/**
 * 門の状態。
 *
 * **module のローカルに置いてある。**
 * `isolate`（既定 `true`）なら測定ファイルごとに読み込み直されるので、
 * **`declared` はファイルごとに白紙から始まる。**
 * `isolate: false` では module がキャッシュされるので、
 * **白紙化は [`resetZoneDeclaration`] を `./zone-setup` から呼ぶことで行う**
 * （`setupFiles` は `--no-isolate` でも測定ファイルごとに再実行される / F-7d のレビューが実測）。
 */
interface GuardState {
  /** 差し替える前の `Intl.DateTimeFormat`。**据え付けより前に控える。** */
  real: typeof Intl.DateTimeFormat;
  /** 据え付け済みか。**二重に包むと、古い状態を見る包みが残る。** */
  installed: boolean;
  /** この測定ファイルが名乗った時間帯。**名乗っていなければ `null`。** */
  declared: string | null;
}

const state: GuardState = {
  // **⚠ ここが控えの権威。** 据え付けより前に読む。
  real: Intl.DateTimeFormat,
  installed: false,
  declared: null,
};

/**
 * **名乗りを白紙へ戻す。** 呼ぶのは `./zone-setup` だけ（測定ファイルごとに1回）。
 *
 * # ⚠ これが要るのは `isolate: false` のときだけである
 *
 * `isolate`（既定 `true`）なら module ごと作り直されるので、ここは冗長。
 * **`isolate: false` では module がキャッシュされる**ので、
 * **前のファイルの名乗りがそのまま残る** ——
 * **そのとき「名乗る前は落ちる」が成立しなくなる**
 * （F-7d のレビューが `--no-isolate --max-workers=1 --no-file-parallelism` で実測）。
 * **`setupFiles` は `--no-isolate` でも測定ファイルごとに再実行される**ので、ここから戻せる。
 *
 * **⚠ 既定の走り（`isolate: true`）では、この関数を消しても何も赤くならない。**
 * **観測できるのは `--no-isolate` のときだけ**である（内訳は `tests/zone-declaration.test.ts`）。
 */
export function resetZoneDeclaration(): void {
  state.declared = null;
}

/**
 * **機械が名乗っている時間帯**（門を通さずに読む）。
 *
 * `./zone` が「機械の既定」を知るための口。**これは名乗りにはならない** ——
 * ここを呼んでも、画面が `browserZone()` を呼べば落ちる。
 */
export function machineZone(): string {
  return new state.real().resolvedOptions().timeZone;
}

/**
 * **この測定ファイルは、その時間帯の読み手である**と名乗る。
 *
 * 呼ぶのは `./zone` の `enterZone` だけ。
 * **`enterZone` は名乗った後に「本当にその時間帯になったか」を確かめる**ので、
 * 名乗りを先に付けても素通りにはならない。
 */
export function declareZone(zone: string): void {
  state.declared = zone;
}

/**
 * `resolvedOptions()` を包み、**名乗るまで `timeZone` を読ませない。**
 *
 * **`timeZone` の欄だけを止める** —— `locale` などは素通しにする。
 * 全部止めると、**時間帯と関係の無い理由で落ちる**ようになり、
 * 「正しい理由で落ちた」が言えなくなる。
 */
function guardTimeZone(made: Intl.DateTimeFormat): Intl.DateTimeFormat {
  const original = made.resolvedOptions.bind(made);
  Object.defineProperty(made, "resolvedOptions", {
    configurable: true,
    writable: true,
    value: (): Intl.ResolvedDateTimeFormatOptions => {
      const resolved = original();
      if (state.declared !== null) {
        return resolved;
      }
      const withoutZone = { ...resolved };
      Object.defineProperty(withoutZone, "timeZone", {
        configurable: true,
        enumerable: true,
        get: (): string => {
          throw new Error(NOT_DECLARED);
        },
      });
      return withoutZone;
    },
  });
  return made;
}

/**
 * 門を据える。**`vitest.config.ts` の `setupFiles` からのみ呼ぶ。**
 *
 * 2 回目以降は何もしない —— **包みを二重にすると、
 * 外側の包みが古い状態を見たまま残る。**
 */
export function installAmbientZoneGuard(): void {
  if (state.installed) {
    return;
  }
  state.installed = true;

  const real = state.real;
  const guarded = function DateTimeFormat(
    locales?: Intl.LocalesArgument,
    options?: Intl.DateTimeFormatOptions,
  ): Intl.DateTimeFormat {
    const made = new real(locales, options);
    if (options?.timeZone !== undefined) {
      // **時間帯を明示した書式は素通し。** 機械に訊いていない。
      return made;
    }
    return guardTimeZone(made);
  };

  // `supportedLocalesOf` などの静的な口を、そのまま引き継ぐ。
  Object.setPrototypeOf(guarded, real);
  // **`instanceof` を壊さない** —— 返すのは本物の実装が作った書式なので、
  // **`prototype` は本物のものでなければ `made instanceof Intl.DateTimeFormat` が偽になる。**
  guarded.prototype = real.prototype;

  Intl.DateTimeFormat = guarded as unknown as typeof Intl.DateTimeFormat;
}
