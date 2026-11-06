/**
 * **画面に出す時刻の綴り**（作業単位 F-7 → **F-7b で地域化した**）。
 *
 * # なぜ要るのか
 *
 * 記憶の一覧も変容の表も、**サーバが返した RFC 3339 をそのまま出していた**
 * （`2026-08-28T14:13:44.983Z`）。**画面が機械の綴りで話している。**
 * `F-5` / `F-8` と同じ家族の欠陥である ——
 * **画面が「リポジトリを開いている人」に向けて書かれていた。**
 *
 * # 読み手の時間帯で出す（**F-7b で変えた**）
 *
 * ~~瞬間は動かさない（地域化しない）。~~
 * **→ 覆った**（利用者の判断 / 2026-08-29）——
 * **「sqlite が UTC で保持していれば、ブラウザの時間帯で出せばよい。」**
 *
 * **前提は実測して確かめてある**（F-7b の着手前）:
 *
 * * **保存側** —— `migrations/**.sql` の `DEFAULT` は全部
 *   `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')`、つまり UTC。
 * * **HTTP の上** —— 時刻の欄は例外なく `DateTime<Utc>`（`src/server/admin.rs` の
 *   `MemoryView` / `CharacterView` / `DriftVoteView` / `DriftAxisView`）で、
 *   chrono の serde は `write_rfc3339(..., use_z = true)` を通る。
 *   **つまり画面に届く綴りは、DB の綴りが何であれ必ず `Z` 終わりに正規化されている。**
 *
 * # 時間帯は**引数で受け取る**（**ここが F-7b の要点**）
 *
 * F-7 が地域化を避けた理由は2つあり、**2つ目は正しい懸念**だった ——
 * **「その綴りを見張る検査が、走らせた機械の時間帯でしか成立しなくなる。」**
 *
 * **関数が時間帯を自分で決めなければ、その懸念は消える。**
 * 解決するのは呼び出し側（`MemoryList.svelte` / `StatsPanel.svelte` / `UpstreamPanel.svelte` /
 * `ChatView.svelte`）で、
 * **検査は好きな時間帯を名指しして決定的に回せる**
 * （`tests/timestamp.test.ts` の**書式を見る節**は `process.env.TZ` を一度も触らない。
 * 触るのは [`browserZone`] を見る節と、画面を mount する測定だけ）。
 *
 * **⚠ この関数の中で [`browserZone`] を呼ばないこと。** 呼んだ瞬間に、
 * 上の性質が丸ごと消える。
 *
 * # 印は「時差」で名乗る
 *
 * `(UTC+09:00)` /`(UTC-04:00)` /時差 0 なら `(UTC)`。
 *
 * * **`Asia/Tokyo` のような時間帯データベースの識別子は出さない** ——
 *   読み手の言葉ではない（作業単位 F-8 と同じ目）。
 * * **`JST` のような略称も採らない。** `Intl` の短い名前は
 *   **時間帯ごとに種類が変わる**（`ja-JP` は東京に `JST`、ニューヨークに `GMT-4` を返す）。
 *   **表も要る。**
 * * **時差なら、どの時間帯でも同じ形**で、しかも
 *   **引き算でログ・`sqlite3` の UTC へ戻せる。**
 *   F-7 が心配した「画面の値がログと食い違う」は、**この印があるので読み解ける。**
 *
 * # 読めなければ、手を出さない
 *
 * `statsEdit.ts` の `runModeLabel` / `deltaLabel` と同じ立場。
 * **綴りが読めないときも、時間帯が読めないときも、入力をそのまま返す。**
 * **時差つきの綴り（`+09:00`）もここに落ちる** ——
 * サーバは UTC しか返さないが、**来た日に黙って別の時間帯へ動かすのがいちばん悪い。**
 *
 * 見張り番は `tests/timestamp.test.ts` と `tests/timestamp-wiring.test.ts`。
 */

/**
 * サーバが返す UTC の綴り（RFC 3339 / **`Z` で終わるものだけ**）。
 *
 * **ミリ秒は任意**（`strftime` の `%f` は付けるが、
 * chrono は 0 のとき落とすことがある）。
 */
const UTC_INSTANT = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2}:\d{2})(?:\.\d+)?Z$/;

/** 1 分あたりのミリ秒。 */
const MS_PER_MINUTE = 60_000;

/**
 * 時差として有り得る上限（分）。
 *
 * 時間帯データベースの実在の時差は **±16 時間**に収まる
 * （下端は `Pacific/Kiritimati` の +14:00、地方平時まで含めても ±16 に届かない）。
 *
 * **これは「速さのつまみ」ではなく、健全性の門である。**
 * ここを超える値が出たときは、**引き算の相手が壊れている**ということなので、
 * **印を名乗らずに入力をそのまま返す** —— この module の規則
 * 「読めなければ手を出さない」に合流させる。
 *
 * **⚠ 実際に起きた**（F-7b のレビューの提案A）——
 * `0000-01-01T00:00:00Z` は `en-US` が**紀元前として年 `1` を返す**ので
 * （`era` を要求していないため紀元前紀元後の区別が落ちる）、
 * 壁時計が `0001-01-01`、時差が `(UTC+8784:00)` になっていた。
 * **年ごとに特例を書くのではなく、「有り得ない値が出たら手を出さない」で塞ぐ。**
 */
const MAX_ZONE_OFFSET_MINUTES = 16 * 60;

/**
 * **ブラウザ（＝読み手の機械）が名乗っている時間帯。**
 *
 * 返るのは時間帯データベースの識別子（`Asia/Tokyo` 等）。
 * **画面に出す値ではない** —— [`timeLabel`] に渡すためだけの値で、
 * 出す印は時差の側（上の doc）。
 *
 * **⚠ これを [`timeLabel`] の中から呼ばないこと。**
 * 呼ぶ場所は画面（`MemoryList.svelte` / `StatsPanel.svelte` / 上流の状態の文面を出す
 * `UpstreamPanel.svelte` / `ChatView.svelte`）に限る。
 */
export function browserZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

/** その時間帯の壁時計に割った、瞬間の各欄。 */
interface WallClock {
  year: string;
  month: string;
  day: string;
  hour: string;
  minute: string;
  second: string;
}

/**
 * その瞬間を、その時間帯の壁時計に割る。**読めない時間帯なら `null`。**
 *
 * **地域を持ち込まない** —— `en-US` と `h23` を名指しして、
 * **読み手の言語設定で欄の綴りが変わらない**ようにしてある
 * （`hour12: false` ではなく `hourCycle: "h23"`。前者は真夜中を `24` と綴る
 * 実装がある）。
 *
 * # ⚠ **第1引数を `undefined` にしないこと**
 *
 * `undefined` にすると**読み手の locale** が使われる。
 * **この機では気付けない** —— 既定は `ja-JP` で、
 * **`ja-JP` の出力は `en-US` とバイト一致する**（F-7b のレビューで実測）。
 * **`ar-EG` の読み手の画面だけが `٢٠٢٦-٠٨-٢٨` になって壊れる**（レビュアーは 30 赤、是正後にこちらで測り直して **37 赤**）。
 * **「機械の既定がたまたま同値」という、この家の最頻の穴。**
 *
 * **⚠ 見張り番は `壁時計は、固定した locale で組み立てられる`
 * （`tests/timestamp.test.ts`）だが、あれは
 * 「`Intl.DateTimeFormat` が `"en-US"` で呼ばれたこと」しか見ていない。**
 * **呼び出しの形を見るだけで、振る舞いの権威ではない。**
 * **型でも実装でも表現できなくはしていない。**
 *
 * **⚠ 塞いでいる範囲は、実際に測った**（F-7b の是正 / 転記）:
 *
 * | 変異 | 赤 |
 * |---|---|
 * | `"en-US"` → `undefined`（**是正前は 658 全緑**） | **1**（この1本だけ） |
 * | `"en-US"` → `"ar-EG"` | **37**（この1本を含む） |
 *
 * **`ar-EG` の側は素通りしない** —— 数字が `٢٠٢٦` になるので、
 * **綴りを見ている見張り番が総崩れになる。**
 * **素通りしうるのは「locale を渡さない」形だけ**で、
 * **それがこの機では観測できなかった**（既定 `ja-JP` の出力が `en-US` とバイト一致）。
 * **この1本は、そこだけを塞いでいる。**
 */
function wallClockIn(at: Date, zone: string): WallClock | null {
  let parts: Intl.DateTimeFormatPart[];
  try {
    parts = new Intl.DateTimeFormat("en-US", {
      timeZone: zone,
      hourCycle: "h23",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }).formatToParts(at);
  } catch {
    // 時間帯として読めない綴りは `RangeError` になる。**推測で直さない。**
    return null;
  }

  const found = new Map(parts.map((part) => [part.type, part.value]));
  const pick = (
    type: Intl.DateTimeFormatPartTypes,
    width: number,
  ): string | null => {
    const value = found.get(type);
    return value === undefined ? null : value.padStart(width, "0");
  };

  const year = pick("year", 4);
  const month = pick("month", 2);
  const day = pick("day", 2);
  const hour = pick("hour", 2);
  const minute = pick("minute", 2);
  const second = pick("second", 2);
  if (
    year === null ||
    month === null ||
    day === null ||
    hour === null ||
    minute === null ||
    second === null
  ) {
    return null;
  }
  return { year, month, day, hour, minute, second };
}

/**
 * 壁時計と瞬間の差（分）。
 *
 * **⚠ その瞬間の時差であって、「いまの時差」ではない。**
 * 夏時間のある時間帯では、同じ時間帯でも季節で変わる
 * （見張り番: `夏時間が、その瞬間の時差で出る`）。
 */
function offsetMinutes(wall: WallClock, at: Date): number {
  // **`Date.UTC` を使わないこと。** あれは **0〜99 の年を 1900 年代へ読み替える**ので、
  // `0026-…` が 1926 年として引き算され、**時差が数百万分になる**
  // （F-7b のレビューで実測: `(UTC+16655035:04)`）。
  // **`setUTCFullYear` にはその読み替えが無い。**
  const asUtc = new Date(0);
  asUtc.setUTCFullYear(
    Number(wall.year),
    Number(wall.month) - 1,
    Number(wall.day),
  );
  asUtc.setUTCHours(
    Number(wall.hour),
    Number(wall.minute),
    Number(wall.second),
    0,
  );
  return Math.round((asUtc.getTime() - at.getTime()) / MS_PER_MINUTE);
}

/**
 * 時間帯の印。**時差 0 なら `(UTC)`。**
 *
 * `+00:00` は読み手に情報を1つも足さないので、そこだけ綴りを変える。
 */
function zoneMark(minutes: number): string {
  if (minutes === 0) {
    return "(UTC)";
  }
  const sign = minutes < 0 ? "-" : "+";
  const size = Math.abs(minutes);
  const hours = String(Math.floor(size / 60)).padStart(2, "0");
  const rest = String(size % 60).padStart(2, "0");
  return `(UTC${sign}${hours}:${rest})`;
}

/**
 * 画面に出す綴りへ。**読めない綴り・読めない時間帯は、そのまま返す。**
 *
 * @param at サーバが返した RFC 3339（`Z` 終わり）。
 * @param zone 時間帯データベースの識別子。**画面が [`browserZone`] で解決して渡す。**
 *
 * **⚠ ミリ秒は畳む。** 正確な値が要る場面のために、
 * **呼び出し側は生の綴りを `title` 属性に残すこと**
 * （`MemoryList.svelte` / `StatsPanel.svelte`）。
 */
export function timeLabel(at: string, zone: string): string {
  const found = UTC_INSTANT.exec(at);
  if (found === null) {
    return at;
  }

  // **秒までに切り詰めた瞬間**で測る。
  // **これは二重の保険である**（F-7b のレビューの実測を受けて言い直した）——
  // `offsetMinutes` の `Math.round` が端数を畳むので、
  // **ミリ秒を残しても印は変わらない。** ここで切り詰める理由は
  // 「端数になるから」ではなく、**印の計算に、畳めば消える誤差を持ち込まないこと。**
  const instant = new Date(`${found[1] as string}T${found[2] as string}Z`);
  const wall = wallClockIn(instant, zone);
  if (wall === null) {
    return at;
  }

  const offset = offsetMinutes(wall, instant);
  if (Math.abs(offset) > MAX_ZONE_OFFSET_MINUTES) {
    // **有り得ない時差** —— 壁時計の側が壊れている。**手を出さない。**
    return at;
  }

  const mark = zoneMark(offset);
  return (
    `${wall.year}-${wall.month}-${wall.day} ` +
    `${wall.hour}:${wall.minute}:${wall.second} ${mark}`
  );
}

/**
 * **時差つきの RFC 3339 を瞬間へ**（Issue origin#40 / 起こした上流の `since`）。**読めなければ `null`。**
 *
 * [`timeLabel`] と違い、**`Z` 終わりに限らない** —— `launch.rs` の `since` は
 * `2026-09-26T13:25:15.770371400+09:00` のように**時差つき・小数部 9 桁**で届く。
 *
 * **⚠ 小数部は 3 桁に切り詰めてから `Date` に渡す。** `Date` が読む形式は
 * ミリ秒までしか定めておらず、それを超える桁は実装しだいで読めない。
 */
export function instantOf(at: string): Date | null {
  const found = OFFSET_INSTANT.exec(at);
  if (found === null) {
    return null;
  }
  const millis = (found[2] ?? "").padEnd(3, "0").slice(0, 3);
  const instant = new Date(`${found[1] as string}.${millis}${found[3] as string}`);
  return Number.isNaN(instant.getTime()) ? null : instant;
}

/** [`instantOf`] が読む綴り（日時・任意の小数部・`Z` か `±HH:MM`）。 */
const OFFSET_INSTANT =
  /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(?:\.(\d+))?(Z|[+-]\d{2}:\d{2})$/;

/**
 * その瞬間の、その時間帯の `HH:MM:SS`。**読めない時間帯なら `null`。**
 *
 * **時間帯は引数で受け取る**（上の doc の「ここが F-7b の要点」と同じ理由）。
 */
export function wallTimeOf(at: Date, zone: string): string | null {
  const wall = wallClockIn(at, zone);
  return wall === null ? null : `${wall.hour}:${wall.minute}:${wall.second}`;
}
