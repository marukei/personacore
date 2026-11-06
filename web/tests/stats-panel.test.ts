// @vitest-environment jsdom
/**
 * **性格値と変容の画面**（作業単位 D-4 / `src/lib/StatsPanel.svelte`）。
 *
 * # この画面の存在理由
 *
 * **変容は本文にもユーザーにも見せない**（[ADR-0024](../../docs/adr/0024-stat-drift.md) §9）。
 * つまり**動いたことが分かる場所が、ログ以外に無かった。**
 * そして **`GET .../stats` が返すのは「いまの値」だけ**なので、
 * **変容したのか、人が `PUT .../stats` で設定したのかを区別できない**
 * （D-4a が `GET .../drift` を足した理由そのもの）。
 *
 * **したがって、この画面は変容の側（累積・上げ下げの件数・直近の票・総数）を
 * 必ず出す。** 出さなければ、画面があっても D-4 の目的は1ミリも果たされない。
 *
 * # `stats-edit.test.ts` との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | 送る本文・外した軸・案内の文言 | `stats-edit.test.ts` |
 * | **画面が何を出すか・どの識別子へ送るか・失敗の見え方** | **ここ** |
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 */

import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import StatsPanel from "../src/lib/StatsPanel.svelte";
import type { DriftView, StatsView } from "../src/lib/api";
import { readerInZone } from "./support/zone";

/**
 * **この測定は、アデレードの読み手として走る**（作業単位 F-7b）。
 *
 * **機械の既定に寄りかからない** —— 寄りかかると、
 * **この機（`Asia/Tokyo`）でしか成立しない期待値**になる。
 *
 * # ⚠ **`timestamp-wiring.test.ts` が使う2つ（東京 / ニューヨーク）を避ける**
 *
 * **レビューの実測**（F-7b）—— **`StatsPanel` の配線の権威が1本しかなかった。**
 * ここが**ニューヨークを名乗っていた**ので、
 * **`const zone = "America/New_York";` という直書きは、
 * この測定を素通りして wiring の1本しか赤にしなかった**（除外すると 651 全緑）。
 * **`MemoryList` は消費側が2本あって別々の時間帯を名乗っている**ので、
 * どれを直書きしても必ずどちらかが赤くなる。**冗長度を揃えるために移した。**
 *
 * # 選び直しても失われない性質
 *
 * アデレードは**夏時間があり、時差が 30 分**なので、
 * この測定が持っていた
 * **「同じ時間帯の中で時差が2種類出る」**（標準時 UTC+9:30 / 夏時間 UTC+10:30）が残る。
 * **「いまの時差」を使う実装は、走らせた日によらずここで落ちる。**
 * **配線の権威は `tests/timestamp-wiring.test.ts`。**
 */
readerInZone("Australia/Adelaide");

/** 見ているキャラクター。 */
const MIO = "mio";
/** **別のキャラクター。** 決め打ちの変異はこちらのデータを出す。 */
const NAR = "nar";
/** **票も軸も1つも無いキャラクター。** */
const AOI = "aoi";
/**
 * **軸が1つも無いのに、変容は「実行中」のキャラクター**（作業単位 F-6）。
 *
 * **`AOI` では F-6 の状態を観測できない** —— あちらは `off` なので、
 * **停止中の案内が「0 件」の理由を言ってしまう。**
 * **報告された状態は「実行中なのに 0 件」**である。
 */
const SORA = "sora";
/**
 * **キャラクター自身が「止める」を選んでいる行**（作業単位 D-15 / ADR-0045）。
 *
 * **起動時の既定は `on`** ——キャラクターの値が既定に勝っている。
 * **`AOI`（既定から来た停止）と、出どころだけが違う**ので、
 * 案内が出どころで言い分けているかをこの2つで見る。
 */
const RIN = "rin";

/**
 * `mio` のいまの値。
 *
 * **⚠ 値・尺度・軸の本数を全部違う数にしてある**（同値化を避ける）。
 * **`勇気` は値 0**（`get_stats` は 0 の軸も返す）。
 */
const MIO_STATS: StatsView = {
  character: MIO,
  stat_scale: 7,
  stats: [
    { axis: "勇気", value: 0 },
    { axis: "慎重さ", value: 2 },
    { axis: "陽気さ", value: 5 },
  ],
};

/**
 * 累積が最後に動いた時刻。**軸ごとに全部違う値**（作業単位 D-5a）。
 *
 * **⚠ 票の `created_at`（`2026-08-27T…`）とも別の日**にしてある。
 * 同じにすると、**票の時刻を累積の欄へ出す取り違えが観測できない。**
 */
const BRAVE_CHARGED_AT = "2026-08-26T01:02:03.400Z";
/** 同上。**`勇気` より古い**（並びに寄りかかった引き当てを落とす）。 */
const CHEER_CHARGED_AT = "2024-05-04T03:02:01.100Z";
/** 同上。**割り当てを失った軸にも時刻は残る。** */
const LOST_CHARGED_AT = "2025-12-31T23:59:59.999Z";
/** `nar` の累積が動いた時刻。**他のどのフィクスチャとも違う値。** */
const LANGUID_CHARGED_AT = "2023-03-03T03:03:03.030Z";

/**
 * **画面に出る時刻の綴り**（作業単位 F-7 → **F-7b で読み手の時間帯になった**）。
 *
 * **⚠ ここに直書きする理由。** `timeLabel` から作ると、
 * **綴りを変える変異が期待値も一緒に動かして緑**になる
 * （CLAUDE.md「写しを1つにしても、その1つは自動的には守られない」）。
 *
 * **⚠ 生の綴りは `title` に残る**（ミリ秒まで含めて）。
 * ログや `sqlite3` の値と目で突き合わせる道を閉じないため。
 *
 * **⚠ 手で足した値である。** アデレードの読み手として走る（上の
 * [`readerInZone`]）ので、**南半球の冬（4〜10 月）は標準時 UTC+9:30、
 * 夏（10〜4 月）は夏時間 UTC+10:30。**
 * **30 分の時差なので、分の桁まで動く。**
 */
const BRAVE_CHARGED_LABEL = "2026-08-26 10:32:03 (UTC+09:30)";
/** 同上（5 月なので標準時）。 */
const CHEER_CHARGED_LABEL = "2024-05-04 12:32:01 (UTC+09:30)";
/**
 * 同上。**12 月は夏時間**なので、時差が1時間大きい。
 * **⚠ 年をまたぐ**（`2025-12-31T23:59Z` + 10:30 は翌年の 1 月 1 日）——
 * 日付だけでなく年まで動く、いちばん強い帰属のフィクスチャ。
 */
const LOST_CHARGED_LABEL = "2026-01-01 10:29:59 (UTC+10:30)";
/** 同上。**3 月 3 日はまだ夏時間**（終わりは 4 月の第1日曜）。 */
const LANGUID_CHARGED_LABEL = "2023-03-03 13:33:03 (UTC+10:30)";
/** 直近の票の日時（**新しい順の1件目と2件目**）。 */
const FIRST_VOTE_LABEL = "2026-08-27 18:30:00 (UTC+09:30)";
/** 同上。 */
const SECOND_VOTE_LABEL = "2026-08-27 17:30:00 (UTC+09:30)";

/**
 * `mio` の変容。**上限で切られている状態**（`total_votes > recent_limit`）。
 *
 * # ⚠ **`陽気さ` の値が `MIO_STATS` とわざとずれている**
 *
 * **`GET .../stats` と `GET .../drift` は別々の要求**であり、
 * `get_drift` の中の5つの読みすら1つのトランザクションで囲っていない
 * （`admin.rs` の `get_drift` の doc）。**間に別の窓が書き換えれば実際にずれる。**
 * **ずらしてあるので、変容の表がどちらの応答を読んでいるかが観測できる。**
 * 揃えると、その取り違えは永遠に緑になる。
 *
 * # ⚠ **`recent_limit` は 50 ではない**
 *
 * 写すと**サーバの定数を倍にする変異が緑を通る**（`RECENT_DRIFT_VOTES` の doc）。
 * **画面は応答の値を出すだけ**なので、ここは 3 にしてある。
 *
 * # ⚠ **割り当ての無い軸が2本入っている**（作業単位 B-16）
 *
 * `PUT .../stats` は丸ごと置き換えなので、**軸を改名すると綴りが変わる。**
 * 累積と票は残るが、いまの値は無い —— サーバは **`value: null`** で返す。
 *
 * **`勇気`（値 0）と同じ応答の中に居ることが要る。**
 * 別々のフィクスチャに分けると、**両方を `0` として描く実装**が
 * 片方ずつ緑になる（サーバ側の
 * `a_zero_valued_axis_is_never_confused_with_an_axis_that_has_no_allocation` と同じ形）。
 *
 * **2本とも入れてある理由** —— 累積だけ／票だけは**別の状態**で、
 * 片方しか無いフィクスチャは、片方だけを拾う実装を通す。
 */
const MIO_DRIFT: DriftView = {
  character: MIO,
  stat_scale: 7,
  axes: [
    // **値 0 なのに変容はしている軸。**
    {
      axis: "勇気",
      value: 0,
      charge: 1.75,
      charge_updated_at: BRAVE_CHARGED_AT,
      ups: 3,
      downs: 1,
    },
    // **累積 0・票 0 の軸**（累積の表に行が無い軸は 0 で返る）。
    // **時刻も `null`** ——「一度も採点されていない」（作業単位 D-5a）。
    { axis: "慎重さ", value: 2, charge: 0, charge_updated_at: null, ups: 0, downs: 0 },
    // **下げが多い軸**（`ups` と `downs` の取り違えが見える）。
    {
      axis: "陽気さ",
      value: 6,
      charge: -0.5,
      charge_updated_at: CHEER_CHARGED_AT,
      ups: 2,
      downs: 8,
    },
    // **割り当てを失った軸（累積だけ）。** 票は1件も無い。
    {
      axis: "きえた軸",
      value: null,
      charge: 1.5,
      charge_updated_at: LOST_CHARGED_AT,
      ups: 0,
      downs: 0,
    },
    // **割り当てを失った軸（票だけ）。** 累積の行が無いので時刻も `null`。
    { axis: "むかしの軸", value: null, charge: 0, charge_updated_at: null, ups: 0, downs: 4 },
  ],
  total_votes: 137,
  recent_limit: 3,
  recent_votes: [
    {
      axis: "陽気さ",
      delta: "down",
      session_id: 41,
      created_at: "2026-08-27T09:00:00Z",
    },
    {
      axis: "勇気",
      delta: "up",
      session_id: 40,
      created_at: "2026-08-27T08:00:00Z",
    },
    // **同じ往復で複数の軸が動く**（1ターンで複数の票が立つ / 本番で普通に起きる）。
    {
      axis: "陽気さ",
      delta: "down",
      session_id: 40,
      created_at: "2026-08-27T07:00:00Z",
    },
  ],
  // **値も動く走らせ方。** `nar`（`shadow`）とも `aoi`（`off`）とも別（同値化を避ける）。
  run_mode: "on",
  run_mode_source: "default",
  default_run_mode: "on",
};

const NAR_STATS: StatsView = {
  character: NAR,
  stat_scale: 9,
  stats: [{ axis: "気だるさ", value: 8 }],
};

/**
 * `nar` の変容。**切られていない状態**（`total_votes <= recent_limit`）。
 *
 * **⚠ この腕が要る理由。** `mio` の側は上限で切られているので
 * **`recent_limit` と「表示件数」が同じ値**になり、**その取り違えが観測できない。**
 * こちらは上限 50・表示 2 件なので、**上限を表示件数にすり替える変異が赤になる。**
 */
const NAR_DRIFT: DriftView = {
  character: NAR,
  stat_scale: 9,
  axes: [
    {
      axis: "気だるさ",
      value: 8,
      charge: 0.25,
      charge_updated_at: LANGUID_CHARGED_AT,
      ups: 6,
      downs: 1,
    },
  ],
  total_votes: 2,
  recent_limit: 50,
  recent_votes: [
    {
      axis: "気だるさ",
      delta: "up",
      session_id: 77,
      created_at: "2026-08-27T05:00:00Z",
    },
    {
      axis: "気だるさ",
      delta: "up",
      session_id: 76,
      created_at: "2026-08-27T04:00:00Z",
    },
  ],
  // **記録専用。** `mio` とも `aoi` とも違う走らせ方（同値化を避ける）。
  run_mode: "shadow",
  run_mode_source: "default",
  default_run_mode: "shadow",
};

/** 軸が1本も無いキャラクター。 */
const AOI_STATS: StatsView = { character: AOI, stat_scale: 5, stats: [] };

/** `RIN` の値（**軸は無い**。`AOI` と揃えて、違うのを出どころだけにする）。 */
const RIN_STATS: StatsView = { character: RIN, stat_scale: 5, stats: [] };

/**
 * `RIN` の変容。**止まっている（`off`）が、それはキャラクターの設定で、既定は `on`。**
 *
 * **`run_mode` と `default_run_mode` をわざと違う値にしてある**
 * ——同じだと、既定を効いている値として読む取り違えが見えない。
 */
const RIN_DRIFT: DriftView = {
  character: RIN,
  stat_scale: 5,
  axes: [],
  total_votes: 0,
  recent_limit: 50,
  recent_votes: [],
  run_mode: "off",
  run_mode_source: "character",
  default_run_mode: "on",
};

/**
 * 軸も票も1つも無いキャラクター。**しかも変容が止まっている**（作業単位 D-5a）。
 *
 * **これが既定の配置そのものである** ——
 * `PERSONACORE_STAT_DRIFT` の既定は `off` で、`off` は
 * **判定もせず、票も累積も1行も作らない。**
 * **つまり「壊れている」ときと応答が1バイトも変わらない。**
 */
const AOI_DRIFT: DriftView = {
  character: AOI,
  stat_scale: 5,
  axes: [],
  total_votes: 0,
  recent_limit: 50,
  recent_votes: [],
  run_mode: "off",
  run_mode_source: "default",
  default_run_mode: "off",
};

/** 軸が1本も無いキャラクター（**尺度は他のどれとも違う**）。 */
const SORA_STATS: StatsView = { character: SORA, stat_scale: 11, stats: [] };

/**
 * **軸も票も1つも無いが、走らせ方は「実行中」**（作業単位 F-6）。
 *
 * **これが「新しいキャラクターを作って、変容を on にした直後」の姿である。**
 * サーバ側のログは `drift_skipped=no_axes` と言っていて、
 * **上流は1度も呼ばれない**（`src/server/drift.rs` の `DriftSkipReason::NoAxes`）。
 * **画面がその理由を言わなければ、「実行中」と「0 件」が矛盾したまま並ぶ。**
 */
const SORA_DRIFT: DriftView = {
  character: SORA,
  stat_scale: 11,
  axes: [],
  total_votes: 0,
  recent_limit: 50,
  recent_votes: [],
  run_mode: "on",
  run_mode_source: "default",
  default_run_mode: "on",
};

interface Call {
  method: string;
  url: string;
  body: unknown;
}

let calls: Call[];
let target: HTMLElement;
/** 次の `PUT` の応答（`null` なら既定の成功）。 */
let putResponse: { status: number; body: string } | null;
/**
 * **サーバがいま返す `mio` の値。**
 *
 * **⚠ 定数にしない。** 定数だと「読み直した」と「打った値がそのまま残っている」が
 * **同じ見え方**になる（レビューの実測 —— 下書きを作り直さない変異が全緑だった）。
 * **別の窓が書き換えた回**を作るために、テストが差し替えられる形にしてある。
 */
let mioStatsNow: StatsView;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown, init?: RequestInit) => {
    const url = String(input);
    const method = init?.method ?? "GET";
    const raw = init?.body;
    calls.push({
      method,
      url,
      body: typeof raw === "string" ? (JSON.parse(raw) as unknown) : null,
    });

    const respond = (body: unknown, status = 200): Promise<Response> =>
      Promise.resolve({
        ok: status >= 200 && status < 300,
        status,
        text: () =>
          Promise.resolve(
            typeof body === "string" ? body : JSON.stringify(body),
          ),
      } as unknown as Response);

    if (method === "PUT") {
      if (putResponse !== null) {
        return respond(putResponse.body, putResponse.status);
      }
      return respond(MIO_STATS);
    }
    // **識別子ごとに違うものを返す**（同じものを返すと取り違えが観測できない）。
    for (const [handle, stats, drift] of [
      [MIO, mioStatsNow, MIO_DRIFT],
      [NAR, NAR_STATS, NAR_DRIFT],
      [AOI, AOI_STATS, AOI_DRIFT],
      [SORA, SORA_STATS, SORA_DRIFT],
      [RIN, RIN_STATS, RIN_DRIFT],
    ] as [string, StatsView, DriftView][]) {
      if (url === `/admin/characters/${handle}/stats`) {
        return respond(stats);
      }
      if (url === `/admin/characters/${handle}/drift`) {
        return respond(drift);
      }
    }
    return respond("{}", 404);
  });
}

/** 画面を出して、読み終わるまで待つ。 */
async function show(handle: string): Promise<{ dispose: () => Promise<void> }> {
  const view = mount(StatsPanel, { target, props: { handle } });
  await vi.waitFor(() => {
    expect(target.querySelector("[data-vote-total]")).not.toBeNull();
  });
  flushSync();
  return {
    dispose: async () => {
      await unmount(view, { outro: false });
    },
  };
}

/** 置き換えの下書きの行。**軸名で引く**（並び順に寄りかからない）。 */
function statRow(axis: string): HTMLElement {
  const found = target.querySelector(`tr[data-axis="${axis}"]`);
  expect(found, `${axis} の行があること`).not.toBeNull();
  return found as HTMLElement;
}

/** 変容の表の行。 */
function driftRow(axis: string): HTMLElement {
  const found = target.querySelector(`tr[data-drift-axis="${axis}"]`);
  expect(found, `${axis} の変容の行があること`).not.toBeNull();
  return found as HTMLElement;
}

/**
 * 空白の並び（改行と字下げ）。
 *
 * **⚠ 綴りを1箇所に置いてある。** この綴りを `Bash` 経由で書き込むと
 * **逆斜線が1段潰れて `/s+/` になる**（CLAUDE.md の「`\\` を `\` に潰す」を
 * このワークツリーで実際に踏んだ —— 潰れても**構文としては通り**、
 * **空白が畳まれないまま緑になりかけた**）。
 */
const WHITESPACE = /\s+/g;

/**
 * **その行のセルの中身**（＝**読み手が実際に見る値**）。
 *
 * # ⚠ なぜ `data-*` では足りないのか
 *
 * `data-ups` / `data-charge` は**テストのためだけに置いた印**であって、
 * **読み手が何を見るかの権威ではない。**
 * **テストは属性を読み、人はセルを読む** ——
 * **この2つがずれると、ずれたことが永久に見えない。**
 *
 * **実測（レビュー / 2026-08-28）**: `<td>{axis.ups}</td>` と
 * `<td>{axis.downs}</td>` を入れ替える変異が **372 全緑**だった
 * （`data-ups` は正しいままなので、属性を見ている限り永遠に分からない）。
 * **実害は重い** —— 変容は本文にもユーザーにも出さない（ADR-0024 §9）ので、
 * **上げ／下げの比を読める場所はこの表しかなく**、
 * 入れ替わると**下げに流れているキャラクターが上げに流れているように見える**
 * （ADR-0024 の「危険2. 一方向へ流れる」は、まさにこの比で判定する）。
 *
 * **だから列を配列で突き合わせる。** 行全体の `toContain` では、
 * **列が入れ替わっても成立する。**
 */
function cellsOf(row: HTMLElement): string[] {
  return [...row.querySelectorAll("td")].map((cell) =>
    (cell.textContent ?? "").replace(WHITESPACE, " ").trim(),
  );
}

/** その行の値の欄に打つ。 */
function typeValue(axis: string, value: string): void {
  const input = statRow(axis).querySelector('input[name="value"]');
  expect(input, `${axis} の値の欄があること`).not.toBeNull();
  const field = input as HTMLInputElement;
  field.value = value;
  field.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
}

/** その行の値の欄。 */
function valueFieldOf(axis: string): HTMLInputElement {
  const found = statRow(axis).querySelector('input[name="value"]');
  expect(found, axis + ' の値の欄があること').not.toBeNull();
  return found as HTMLInputElement;
}

/** その行のボタンを押す。 */
function clickInRow(row: HTMLElement, label: string): void {
  const found = [...row.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `行に「${label}」があること`).toBeDefined();
  (found as HTMLButtonElement).click();
  flushSync();
}

/** 画面のボタン。 */
function buttonLabelled(label: string): HTMLButtonElement {
  const found = [...target.querySelectorAll("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  expect(found, `「${label}」ボタンがあること`).toBeDefined();
  return found as HTMLButtonElement;
}

function submit(): void {
  const form = target.querySelector("form");
  expect(form).not.toBeNull();
  (form as HTMLFormElement).dispatchEvent(
    new Event("submit", { bubbles: true, cancelable: true }),
  );
  flushSync();
}

/** 空白（改行と字下げ）を畳んだ文面。**折り返しの位置に寄りかからない。** */
function flatText(element: HTMLElement): string {
  return (element.textContent ?? "").replace(WHITESPACE, " ").trim();
}

function voteTotal(): HTMLElement {
  const found = target.querySelector("[data-vote-total]");
  expect(found).not.toBeNull();
  return found as HTMLElement;
}

function voteRows(): HTMLElement[] {
  return [
    ...target.querySelectorAll('tbody[data-section="drift-votes"] tr'),
  ] as HTMLElement[];
}

/**
 * 票の表の各行の、**日時の欄（4列目）の `title`**。**並びは表のまま。**
 *
 * **⚠ 1行だけ引かないこと**（レビューの実測 / 2026-08-29）——
 * 先頭行だけを見ていたので、**「常に先頭行の時刻」にする変異が 631 全緑**を通った。
 */
function voteTitles(): (string | null)[] {
  return voteRows().map((row) => {
    const cell = row.querySelectorAll("td")[3];
    expect(cell, "日時の欄があること").toBeDefined();
    return (cell as HTMLTableCellElement).getAttribute("title");
  });
}

/**
 * **どの状態にも当てる判定** ——
 * **案内が出ているなら、名指しした操作は全部この画面に描かれている。**
 *
 * # なぜ「出ていること」ではなく「出ているなら」なのか
 *
 * **案内が出ない状態こそが穴だった**（レビューの実測 / 2026-08-28）。
 * `GET .../stats` が 500 で `GET .../drift` が 200 の回は、
 * **置き換えのフォームが描かれないので「軸を追加」も
 * 「この内容で置き換える」も画面に無い**のに、案内だけが出ていた
 * （そのとき実在した操作は `["再読み込み", "キャラクターの一覧へ"]` の2つだけ）。
 *
 * **その状態を描くテストは既にあった**（`いまの値が読めなくても、変容の側は出る`）。
 * **足りなかったのは「その標本に判定を当てること」** ——
 * **`13c-3a` の「分岐が効く状態にフィクスチャが一度も入らない」の変種**で、
 * ここでは**入っているのに測っていなかった。**
 *
 * **だから判定を条件つきにして、案内が出うる全部の状態で呼ぶ。**
 * 出ない状態では素通りし、出た瞬間に操作の実在を要求する。
 */
/**
 * 網に入れる案内と、その案内が**操作を名指しすべきかどうか**（作業単位 D-5a）。
 *
 * | 案内 | 名指しを要求するか |
 * |---|---|
 * | 捨て軸（`data-unallocated-notice`） | **する。** 直し方が「軸を追加 → 置き換える」の2手そのもの |
 * | 走らせ方（`data-run-mode-notice`） | **する**（作業単位 D-15 から）。走らせ方はキャラクターの編集で切り替えられ、案内はそこへの導線（ヘッダの「キャラクターの一覧へ」）を名指しする。**D-15 より前は「しない」**——切り替える口が管理 API にも画面にも無かった |
 * | 出どころ（`data-run-mode-source`） | **しない。** 出どころを言うだけで、操作を勧めない |
 * | 軸ゼロ（`data-no-axes-notice`） | **しない。** `GET .../stats` が失敗した回は、名指しする2つが描かれない |
 *
 * **⚠ 「名指しを要求しない」は「何を書いてもよい」ではない。**
 * **書いたなら実在すること**は両方に要求する——
 * **緩めているのは「1つ以上書け」だけ**である。
 *
 * **⚠ したがって、軸ゼロの案内から名指しを丸ごと落とす変異は、ここでは赤にならない。**
 * **その押さえは `案内が名指しした手順を、この画面から辿れる`**（綴りを直書きしてある）。
 */
const NOTICE_SELECTORS: [selector: string, mustName: boolean][] = [
  ["[data-unallocated-notice]", true],
  // **作業単位 D-15 から名指しする**（「キャラクターの一覧へ」—— 編集はそこから辿る）。
  ["[data-run-mode-notice]", true],
  // **出どころの一言**（作業単位 D-15）。操作は名指ししない。
  ["[data-run-mode-source]", false],
  ["[data-no-axes-notice]", false],
];

/**
 * **どの状態にも当てる判定** ——
 * **案内が出ているなら、名指しした操作は全部この画面に描かれている。**
 *
 * # なぜ「出ていること」ではなく「出ているなら」なのか
 *
 * **案内が出ない状態こそが穴だった**（レビューの実測 / 2026-08-28）。
 * `GET .../stats` が 500 で `GET .../drift` が 200 の回は、
 * **置き換えのフォームが描かれないので「軸を追加」も
 * 「この内容で置き換える」も画面に無い**のに、案内だけが出ていた
 * （そのとき実在した操作は `["再読み込み", "キャラクターの一覧へ"]` の2つだけ）。
 *
 * **その状態を描くテストは既にあった**（`いまの値が読めなくても、変容の側は出る`）。
 * **足りなかったのは「その標本に判定を当てること」** ——
 * **`13c-3a` の「分岐が効く状態にフィクスチャが一度も入らない」の変種**で、
 * ここでは**入っているのに測っていなかった。**
 *
 * **だから判定を条件つきにして、案内が出うる全部の状態で呼ぶ。**
 * 出ない状態では素通りし、出た瞬間に操作の実在を要求する。
 *
 * > **⚠ 新しい案内を足したら、この網（[`NOTICE_SELECTORS`]）に入れること**
 * > （作業単位 D-5a で走らせ方の案内を入れた）。
 * > 入れないと、**無い操作を案内する形が、その案内でだけ復活する。**
 */
function assertNoticeNamesOnlyRealOperations(): void {
  const labels = new Set(
    [...target.querySelectorAll("button, a")].map(
      (element) => element.textContent?.trim() ?? "",
    ),
  );

  for (const [selector, mustName] of NOTICE_SELECTORS) {
    const notice = target.querySelector(selector);
    if (notice === null) {
      continue;
    }
    const named = [
      ...flatText(notice as HTMLElement).matchAll(/「([^」]+)」/g),
    ].map((found) => found[1] as string);
    if (mustName) {
      // **名指ししていること自体を見る**（「」を外す変異を空振りで通さない）。
      expect(named.length, `${selector} が操作を名指しすること`).toBeGreaterThan(0);
    }

    for (const operation of named) {
      expect(
        labels.has(operation),
        `${selector} の「${operation}」が、いまの画面に実在すること（実在するのは ${[...labels].join(" / ")}）`,
      ).toBe(true);
    }
  }
}

beforeEach(() => {
  calls = [];
  putResponse = null;
  mioStatsNow = MIO_STATS;
  target = document.createElement("div");
  document.body.append(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
});

describe("見ているキャラクターのものを読む", () => {
  it("渡された識別子で、いまの値と変容を引く", async () => {
    const shown = await show(MIO);

    expect(calls.map((call) => call.url)).toContain(
      `/admin/characters/${MIO}/stats`,
    );
    expect(calls.map((call) => call.url)).toContain(
      `/admin/characters/${MIO}/drift`,
    );
    // **対照: 別の識別子は1度も引いていない**（決め打ちを落とす）。
    expect(calls.map((call) => call.url).join(" ")).not.toContain(NAR);
    expect(calls.map((call) => call.url).join(" ")).not.toContain(AOI);
    expect(target.querySelector("h1")?.textContent).toContain(MIO);

    await shown.dispose();
  });

  it("別の識別子なら、そのキャラクターの軸が出る", async () => {
    const shown = await show(NAR);

    expect(target.querySelector('tr[data-axis="気だるさ"]')).not.toBeNull();
    // **`mio` の軸は1つも出ていない。**
    expect(target.querySelector('tr[data-axis="勇気"]')).toBeNull();
    expect(target.textContent).not.toContain("陽気さ");

    await shown.dispose();
  });
});

describe("変容の記録が見える（この画面の存在理由）", () => {
  it("軸ごとに、累積と上げ下げの件数を出す", async () => {
    const shown = await show(MIO);

    const brave = driftRow("勇気");
    expect(brave.dataset["ups"]).toBe("3");
    expect(brave.dataset["downs"]).toBe("1");
    expect(brave.dataset["charge"]).toBe("1.75");

    const cheer = driftRow("陽気さ");
    expect(cheer.dataset["ups"]).toBe("2");
    expect(cheer.dataset["downs"]).toBe("8");
    expect(cheer.dataset["charge"]).toBe("-0.5");

    // **人が見るセルを、列ごと突き合わせる**（`cellsOf` の doc）。
    // 軸 / 値 / 累積 / 累積が動いた日時 / 上げ / 下げ。
    expect(cellsOf(brave)).toEqual([
      "勇気",
      "0 / 7",
      "1.75",
      BRAVE_CHARGED_LABEL,
      "3",
      "1",
    ]);
    expect(cellsOf(cheer)).toEqual([
      "陽気さ",
      "6 / 7",
      "-0.5",
      CHEER_CHARGED_LABEL,
      "2",
      "8",
    ]);
    // **累積の行が無い軸は「記録なし」**（`0` でも `null` でもない）。
    expect(cellsOf(driftRow("慎重さ"))).toEqual([
      "慎重さ",
      "2 / 7",
      "0",
      "記録なし",
      "0",
      "0",
    ]);

    await shown.dispose();
  });

  /**
   * **値 0 の軸も、累積 0 の軸も落とさない。**
   *
   * サーバは両方返す（`get_stats` / `drift_view` の「0 で埋めるのはここ」）。
   * **落とすと「軸を消したのか、まだ動いていないのか」が読み手に区別できない。**
   */
  it("値 0 の軸を落とさない", async () => {
    const shown = await show(MIO);

    expect(driftRow("勇気").dataset["value"]).toBe("0");
    expect(statRow("勇気")).not.toBeNull();

    await shown.dispose();
  });

  it("累積 0・票 0 の軸を落とさない", async () => {
    const shown = await show(MIO);

    const careful = driftRow("慎重さ");
    expect(careful.dataset["charge"]).toBe("0");
    expect(careful.dataset["ups"]).toBe("0");
    expect(careful.dataset["downs"]).toBe("0");

    await shown.dispose();
  });

  /**
   * **⚠ 変容の表の値は、変容の応答のもの。**
   *
   * 2つの読みは別々の要求なので、間に別の窓が書き換えればずれる。
   * **フィクスチャはわざとずらしてある**（`MIO_DRIFT` の doc）。
   */
  it("変容の表は、変容の応答の値を出す（いまの値の応答と混ぜない）", async () => {
    const shown = await show(MIO);

    expect(driftRow("陽気さ").dataset["value"]).toBe("6");
    // 置き換えの下書きの側は `GET .../stats` の値。
    expect(
      (statRow("陽気さ").querySelector('input[name="value"]') as HTMLInputElement)
        .value,
    ).toBe("5");

    await shown.dispose();
  });

  it("直近の票は、軸・向き・どの往復かを出す", async () => {
    const shown = await show(MIO);

    const rows = voteRows();
    expect(rows).toHaveLength(3);
    // **新しい順**（サーバの並びをここで並べ替えない）。
    expect(rows.map((row) => row.dataset["sessionId"])).toEqual([
      "41",
      "40",
      "40",
    ]);
    expect(rows.map((row) => row.dataset["voteAxis"])).toEqual([
      "陽気さ",
      "勇気",
      "陽気さ",
    ]);
    expect(rows.map((row) => row.dataset["delta"])).toEqual([
      "down",
      "up",
      "down",
    ]);
    // **人が見るセルを、列ごと突き合わせる**（軸 / 向き / 会話 / 日時）。
    expect(cellsOf(rows[0] as HTMLElement)).toEqual([
      "陽気さ",
      "下げ",
      "41",
      FIRST_VOTE_LABEL,
    ]);
    expect(cellsOf(rows[1] as HTMLElement)).toEqual([
      "勇気",
      "上げ",
      "40",
      SECOND_VOTE_LABEL,
    ]);

    await shown.dispose();
  });

  /**
   * **累積が最後に動いた日時を、軸ごとに出す**（作業単位 D-5a）。
   *
   * # なぜこの列が要るのか
   *
   * **累積の表は履歴を持たない**（履歴は票とログの側）ので、
   * **表から取れる時間の手掛かりはこの1列だけ**である
   * （`store::drift` の `UPSERT_CHARGE` の doc）。
   * **列は前から書き込まれていたのに、読む口がどこにも無かった。**
   *
   * # ⚠ 票の時刻と混ぜない
   *
   * フィクスチャは**票が `2026-08-27`、累積が別の年**にしてある。
   * 混ぜると、**記録専用モードで回している間**の見え方が壊れる
   * ——そのモードでは**票の時刻だけが進んで、累積の時刻は止まる。**
   *
   * # ⚠ 帰属（**軸ごとに違う時刻を、その軸の行に**）
   *
   * 3軸それぞれ別の時刻で、**軸名の順と時刻の順を食い違わせてある。**
   * **押さえるのは真ん中の行**（`陽気さ`）——2件だと
   * 「常に先頭」「常に末尾」の片方が通り抜ける（`D-2` の実測）。
   */
  it("累積が最後に動いた日時を、軸ごとに出す（票の時刻と混ぜない）", async () => {
    const shown = await show(MIO);

    // **セルの配列が権威**（`data-*` はテストが置いた印にすぎない / `cellsOf` の doc）。
    expect(cellsOf(driftRow("勇気"))[3]).toBe(BRAVE_CHARGED_LABEL);
    expect(cellsOf(driftRow("陽気さ"))[3]).toBe(CHEER_CHARGED_LABEL);
    expect(cellsOf(driftRow("きえた軸"))[3]).toBe(LOST_CHARGED_LABEL);
    // **累積の行が無い軸は「記録なし」。**
    expect(cellsOf(driftRow("慎重さ"))[3]).toBe("記録なし");
    expect(cellsOf(driftRow("むかしの軸"))[3]).toBe("記録なし");

    // **票の時刻がこの列に来ていない。**
    expect(cellsOf(driftRow("陽気さ"))[3]).not.toContain("2026-08-27");
    // **生の `null` が画面に出ていない。**
    expect(flatText(target)).not.toContain("null");

    await shown.dispose();
  });

  /**
   * **「0 に戻った」と「一度も採点されていない」が、画面の上で別物である。**
   *
   * **`慎重さ`（累積 0・行なし）と `陽気さ`（累積 -0.5・行あり）だけでは足りない**
   * ——`0` という数そのものが両方の意味を持ちうるので、
   * **累積が 0 で行がある軸**と**累積が 0 で行が無い軸**を並べて比べる。
   *
   * `むかしの軸` は `charge: 0` で行が無く、`慎重さ` も同じ。
   * **どちらも「記録なし」になるのが正しい**ので、
   * ここで見るのは**「その 0 が時刻の欄に漏れていないこと」**である
   * ——漏れると `0` が日時として並び、**エポックだと読める。**
   */
  it("累積 0 の軸で、0 が日時の欄に漏れない", async () => {
    const shown = await show(MIO);

    const careful = cellsOf(driftRow("慎重さ"));
    // 累積の欄は `0`、日時の欄は数を1文字も含まない。
    expect(careful[2]).toBe("0");
    expect(careful[3]).not.toMatch(/[0-9]/);
    // **2つの欄が同じ文字列になっていない**（列の入れ替わりが見える）。
    expect(careful[3]).not.toBe(careful[2]);

    await shown.dispose();
  });
});

/**
 * **変容が動いているのかどうかが、画面から読める**（作業単位 D-5a）。
 *
 * # なぜこれが要るのか
 *
 * **サーバ側の既定は `off`** で、`off` は判定もせず票も累積も1行も作らない。
 * **つまり既定の配置でこの画面を開くと、
 * 「総数 0 件・軸の累積 0・票の一覧なし」が出る** ——
 * **変容が壊れている状態と、画面上で1ドットも変わらない。**
 * 走らせ方はサーバの起動ログにしか出ておらず、**画面を見ている人は読めない。**
 */
describe("変容が動いているかどうかが読める", () => {
  /** 走らせ方を出している要素。 */
  function runMode(): HTMLElement {
    const found = target.querySelector("[data-run-mode]");
    expect(found, "走らせ方を出すこと").not.toBeNull();
    return found as HTMLElement;
  }

  /**
   * **走らせ方ごとの、読み手が見る1行の目印**（レビュー指摘 / 2026-08-28）。
   *
   * # ⚠ ここに直書きする理由
   *
   * `runModeLabel` から作ると、**見出しを入れ替える変異が期待値も一緒に動かして緑**になる
   * （CLAUDE.md「写しを1つにしても、その1つは自動的には守られない」）。
   * **綴りの写し元は `statsEdit.ts` だが、判定に使う語はこの検査の中にある。**
   *
   * # 3つは互いの部分文字列にならないこと
   *
   * `off` の見出しは「記録**も**していません」なので、`shadow` の「記録**のみ**」とは当たらない。
   * **当たる語を選ぶと、入れ替えても3つとも真になって何も見なくなる。**
   */
  const RUN_MODE_MARKS: [mode: string, mark: string][] = [
    ["off", "停止中"],
    ["shadow", "記録のみ"],
    ["on", "実行中"],
  ];

  /**
   * **3つの走らせ方が、画面で別々に見える。しかも、それぞれ自分の見出しで。**
   *
   * # ⚠ 印だけでなく、読み手が見る文も見る
   *
   * `data-run-mode` は**テストのためだけに置いた印**であって、
   * **読み手が何を見るかの権威ではない**（`cellsOf` の doc の実測 / `D-4`）。
   *
   * # ⚠ 濃度では帰属を見ていない（**レビュアーの実測で緑を通った**）
   *
   * 以前ここは **`new Set(texts).size === 3`** だけを見ていた
   * ——**「3つが違うこと」しか言っていない。**
   *
   * | 変異 | 是正前 |
   * |---|---|
   * | `shadow` と `on` の見出しを入れ替える | **582 全緑** |
   * | 3つを3巡回で回転 | **582 全緑** |
   *
   * **実害はこの作業単位の存在理由そのもの** ——
   * **サーバは正しく `shadow` と言っているのに、画面の1行が
   * 「実行中（票を残し、値も動かします）」になる。**
   * **値は動いていない。そして変容は本文に出ない**（ADR-0024 §9）ので、
   * **読める場所が他に無い。**
   *
   * **⚠ しかも `runModeNotice` は `off` に正しく紐づいたままなので、**
   * **`off` の見出しが「実行中…」に化けると、見出しと案内が矛盾した画面**が出る。
   * **それでも全緑だった。**
   *
   * # 3体を1本で回す
   *
   * **どれか1つの腕だけ**だと、その値へ決め打ちする実装が通る
   * （`mio` = `on` / `nar` = `shadow` / `aoi` = `off`）。
   */
  it("走らせ方を、キャラクターごとの応答のまま出す", async () => {
    const seen: string[] = [];
    const texts: string[] = [];

    for (const [handle, expected] of [
      [MIO, "on"],
      [NAR, "shadow"],
      [AOI, "off"],
    ] as [string, string][]) {
      const shown = await show(handle);
      const element = runMode();
      expect(element.dataset["runMode"], `${handle} の走らせ方`).toBe(expected);
      seen.push(element.dataset["runMode"] as string);
      const text = flatText(element);
      // **生の綴りだけを出していない**（訳した見出しが付いていること）。
      expect(text, `${handle} の見出し`).not.toBe(expected);

      // **帰属**——その走らせ方の目印が出ていて、**他の2つの目印は出ていない。**
      for (const [mode, mark] of RUN_MODE_MARKS) {
        if (mode === expected) {
          expect(
            text,
            `${handle}（${mode}）の見出しに「${mark}」が出ること: ${text}`,
          ).toContain(mark);
          continue;
        }
        expect(
          text,
          `${handle}（${expected}）の見出しに、別の走らせ方（${mode}）の「${mark}」が出ていないこと: ${text}`,
        ).not.toContain(mark);
      }

      texts.push(text);
      await shown.dispose();
      target.replaceChildren();
    }

    expect(seen).toEqual(["on", "shadow", "off"]);
    // **3つの見出しが全部違う**（`shadow` と `on` を同じ文にしない）。
    expect(new Set(texts).size, `見出しが重複している: ${texts.join(" / ")}`).toBe(3);
  });

  /**
   * **止まっている理由を、出どころで言い分ける**（作業単位 D-15 / ADR-0045）。
   *
   * **`AOI` と `RIN` は、出どころだけが違う**（どちらも止まっていて、軸も票も無い）。
   *
   * * **既定から来た停止（`AOI`）** —— 既定に従っていると言い、環境変数も補足する。
   * * **キャラクターが選んだ停止（`RIN`）** —— 自分の設定だと言い、
   *   **環境変数を案内しない**（このキャラクターには効かない操作になる / origin#31 のレビュー）。
   *
   * **どちらも、名指しした導線がこの画面に実在する**（`assertNoticeNamesOnlyRealOperations`）。
   */
  it("止まっている理由を、出どころで言い分ける", async () => {
    const texts: Record<string, [notice: string, source: string]> = {};
    for (const handle of [AOI, RIN]) {
      const shown = await show(handle);
      const notice = target.querySelector("[data-run-mode-notice]");
      const source = target.querySelector("[data-run-mode-source]");
      expect(notice, `${handle} に停止中の案内が出ること`).not.toBeNull();
      expect(source, `${handle} に出どころの一言が出ること`).not.toBeNull();
      texts[handle] = [
        flatText(notice as HTMLElement),
        flatText(source as HTMLElement),
      ];
      assertNoticeNamesOnlyRealOperations();
      await shown.dispose();
      target.replaceChildren();
    }

    const [aoiNotice, aoiSource] = texts[AOI] as [string, string];
    const [rinNotice, rinSource] = texts[RIN] as [string, string];
    expect(aoiNotice).not.toBe(rinNotice);
    expect(aoiSource).not.toBe(rinSource);

    expect(aoiNotice).toContain("PERSONACORE_STAT_DRIFT");
    expect(aoiSource).toContain("起動時の既定");
    expect(rinNotice).toContain("このキャラクター自身の設定");
    expect(rinNotice).not.toContain("PERSONACORE_STAT_DRIFT");
    expect(rinSource).toContain("このキャラクター自身の設定");
  });

  /**
   * **動いているときも、出どころを言う**（作業単位 D-15）。
   *
   * `MIO`（`on`）も `NAR`（`shadow`）も既定から来ている。**停止中の案内は出ない**が、
   * **出どころの一言は出る**（止まっていないときに出どころを黙ると、
   * 「どこで切り替えるか」が止まっているときにしか読めない）。
   */
  it("動いているときも、出どころの一言は出る", async () => {
    for (const handle of [MIO, NAR]) {
      const shown = await show(handle);
      const source = target.querySelector("[data-run-mode-source]");
      expect(source, `${handle} に出どころの一言が出ること`).not.toBeNull();
      expect(flatText(source as HTMLElement)).toContain("起動時の既定");
      await shown.dispose();
      target.replaceChildren();
    }
  });

  /**
   * **止まっている画面で、見出しと案内が食い違わない**（レビュアーの実測から）。
   *
   * `runModeNotice` は `off` に正しく紐づいたままなので、
   * **見出しだけが化けると「実行中」と書いた真下に「変容はいま停止しています」が並ぶ。**
   * **その矛盾は、上のどの濃度の検査にも掛からなかった。**
   *
   * **同じ画面の中で2つを突き合わせる**ので、片方だけが動いた瞬間に赤になる。
   */
  it("停止中の画面では、見出しと案内が同じことを言う", async () => {
    const shown = await show(AOI);

    const heading = flatText(runMode());
    const notice = target.querySelector("[data-run-mode-notice]");
    expect(notice, "停止中の案内が出ること").not.toBeNull();

    // **見出しは止まっていると言い、動いているとは言わない。**
    expect(heading, `見出し: ${heading}`).toContain("停止中");
    expect(heading).not.toContain("実行中");
    expect(heading).not.toContain("記録のみ");
    // **案内も同じことを言っている。**
    expect(flatText(notice as HTMLElement)).toContain("停止しています");

    await shown.dispose();
  });

  /**
   * **止まっているときは、「0 件」の理由を言う。**
   *
   * これが無いと、**壊れているのか止めてあるのかが読み手に区別できない。**
   */
  it("止まっているときは、0 件の理由を言う", async () => {
    const shown = await show(AOI);

    const notice = target.querySelector("[data-run-mode-notice]");
    expect(notice, "停止中の案内が出ること").not.toBeNull();
    const text = flatText(notice as HTMLElement);
    // **次に触る場所を言う**（走らせ方はサーバの起動設定）。
    expect(text).toContain("PERSONACORE_STAT_DRIFT");
    // **「0 件」が壊れているせいではないと言う。**
    expect(text).toContain("壊れている");

    // **票が1件も無いことも、これまでどおり言っている**（片方が消えていない）。
    expect(flatText(target)).toContain("まだ1件も");

    // **無い操作を名指ししていない**（この画面に切り替えの口は無い）。
    assertNoticeNamesOnlyRealOperations();

    await shown.dispose();
  });

  /**
   * **動いているときは、止まっているとは言わない。**
   *
   * `shadow` も `on` も**判定は走っている**ので、
   * 「動いていません」と書くと嘘になる。
   * **2つとも見る**——片方だけだと、`off` 以外の一方に決め打ちする実装が通る。
   */
  it("動いているときは、停止中の案内を出さない", async () => {
    for (const handle of [MIO, NAR]) {
      const shown = await show(handle);
      expect(
        target.querySelector("[data-run-mode-notice]"),
        `${handle} には停止中の案内を出さないこと`,
      ).toBeNull();
      // **走らせ方そのものは出ている**（欄ごと消えているのではない）。
      expect(target.querySelector("[data-run-mode]")).not.toBeNull();
      assertNoticeNamesOnlyRealOperations();
      await shown.dispose();
      target.replaceChildren();
    }
  });
});

/**
 * **「実行中」と「0 件」が、理由で繋がっている**（作業単位 F-6）。
 *
 * # なぜ要るのか
 *
 * **ログは `drift_skipped=no_axes` と言っている**（`src/server/drift.rs`）。
 * **軸が1本も無ければ、`build_drift_prompt` が `None` を返して上流を呼ばない。**
 * それなのに画面は、**走らせ方「実行中（票を残し、値も動かします）」**と
 * **「まだ1件も票がありません」**を並べて出していた。
 *
 * **画面は軸が無いことを知っている**（「軸が1本もありません。」を2箇所に出している）。
 * **`runModeNotice` が理由を言うのは `off` のときだけ**だった。
 *
 * **新しいキャラクターは必ずこの状態から始まる。**
 *
 * # ⚠ 腕は `SORA`（**実行中 × 軸ゼロ**）
 *
 * **`AOI` では観測できない** —— あちらは `off` なので、
 * **停止中の案内が「0 件」の理由を言ってしまう。**
 * **F-6 が報告された状態は「実行中なのに 0 件」**である。
 */
describe("軸が1本も無いことが、0 件の理由として読める（作業単位 F-6）", () => {
  /** 軸が無いときの案内。 */
  function noAxesNotice(): HTMLElement | null {
    return target.querySelector("[data-no-axes-notice]");
  }

  /** 走らせ方を出している要素。 */
  function runModeLine(): HTMLElement {
    const found = target.querySelector("[data-run-mode]");
    expect(found, "走らせ方を出すこと").not.toBeNull();
    return found as HTMLElement;
  }

  it("実行中でも軸が無ければ、判定が走らない理由を言う", async () => {
    const shown = await show(SORA);

    const notice = noAxesNotice();
    expect(notice, "軸が無いときの案内が出ること").not.toBeNull();
    const text = flatText(notice as HTMLElement);
    expect(text).toContain("軸が1本もありません");
    expect(text).toContain("判定");

    // **走らせ方は「実行中」と言っている**（矛盾したままにしない）。
    const heading = flatText(runModeLine());
    expect(heading, `見出し: ${heading}`).toContain("実行中");
    // **停止中の案内は出ていない**（`off` の理由と混ぜていない）。
    expect(target.querySelector("[data-run-mode-notice]")).toBeNull();
    // **「0 件」の側も、これまでどおり出ている。**
    expect(flatText(target)).toContain("まだ1件も票がありません");

    // **名指しした操作は、この画面に実在する。**
    assertNoticeNamesOnlyRealOperations();

    await shown.dispose();
  });

  /**
   * **止めてあって、しかも軸が無い**（既定の配置で新しいキャラクターを作った直後）。
   *
   * **2つは別のことを言っている**ので、両方出す ——
   * 軸を足しても、`off` のままなら動かない。
   */
  it("止まっていて軸も無ければ、2つの理由を両方言う", async () => {
    const shown = await show(AOI);

    expect(noAxesNotice(), "軸が無いときの案内").not.toBeNull();
    expect(
      target.querySelector("[data-run-mode-notice]"),
      "停止中の案内",
    ).not.toBeNull();
    // **2つは同じ文ではない**（片方をもう片方で埋める実装を落とす）。
    const first = flatText(noAxesNotice() as HTMLElement);
    const second = flatText(
      target.querySelector("[data-run-mode-notice]") as HTMLElement,
    );
    expect(first).not.toBe(second);

    assertNoticeNamesOnlyRealOperations();

    await shown.dispose();
  });

  /**
   * **軸があるキャラクターには出ない。**
   *
   * 出しっぱなしにする実装は、上の2本を満たしてしまう。
   * **2体で見る**——片方だけだと、その識別子へ決め打ちする実装が通る。
   */
  it("対照: 軸があるキャラクターには出ない", async () => {
    for (const handle of [MIO, NAR]) {
      const shown = await show(handle);
      expect(noAxesNotice(), `${handle} には出さないこと`).toBeNull();
      // **変容の表そのものは出ている**（画面ごと消えているのではない）。
      expect(target.querySelector("[data-run-mode]")).not.toBeNull();
      assertNoticeNamesOnlyRealOperations();
      await shown.dispose();
      target.replaceChildren();
    }
  });

  /**
   * **名指しした2つが、この画面に本当に描かれている。**
   *
   * `assertNoticeNamesOnlyRealOperations` は
   * **この案内に対しては「書いたなら実在すること」しか要求しない**
   * （`mustName` が `false`）ので、**名指しごと落とす変異はあれでは赤にならない。**
   * **ここで、2つの綴りを直書きして押さえる。**
   */
  /**
   * **いまの値が読めない回は、理由だけを言って、操作を名指ししない**
   * （レビューの実測 / 2026-08-29）。
   *
   * # ⚠ なぜこの腕が要ったのか
   *
   * **画面のテストが `canReplace === true` の側しか通っていなかった**ので、
   * **`noAxesNotice(driftState.value, canReplace)` を
   * `noAxesNotice(driftState.value, true)` へ決め打つ変異が 631 全緑**を通った。
   *
   * **死んだコードではない** —— `GET .../stats` が失敗し、
   * `GET .../drift` は読めて軸ゼロ、という状態は実在する。
   * そのとき画面は**存在しない「軸を追加」「この内容で置き換える」を名指しする。**
   *
   * **CLAUDE.md の「実行時に決まる値を入れたら、
   * その値が既定と違う状態で回る腕を1本置く」**（`B-7`）が、ここで欠けていた。
   *
   * # ⚠ `assertNoticeNamesOnlyRealOperations` だけでは足りない
   *
   * あの網は `[data-no-axes-notice]` に `mustName: false` を与えている
   * ——**名指しが 0 個でも通る**ので、
   * **「名指ししてはいけない状態で名指しした」ことは、あちらでは捕まえられる**が、
   * **その逆（名指しが要る状態で落とした）は捕まえられない。**
   * **両向きは、この検査と `案内が名指しした手順を、この画面から辿れる` の対で押さえる。**
   */
  it("いまの値が読めない回は、理由は言うが、操作は名指ししない", async () => {
    vi.stubGlobal("fetch", (input: unknown) => {
      const url = String(input);
      if (url.endsWith("/stats")) {
        return Promise.resolve({
          ok: false,
          status: 500,
          text: () => Promise.resolve("こわれた"),
        } as unknown as Response);
      }
      return Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(JSON.stringify(SORA_DRIFT)),
      } as unknown as Response);
    });

    const view = mount(StatsPanel, { target, props: { handle: SORA } });
    await vi.waitFor(() => {
      expect(target.querySelector("[data-vote-total]")).not.toBeNull();
    });

    // **前提: 置き換えのフォームは描かれていない。**
    expect(target.querySelector("[data-stats-error]")).not.toBeNull();
    const labels = [...target.querySelectorAll("button, a")].map(
      (element) => element.textContent?.trim() ?? "",
    );
    expect(labels, `この状態の操作: ${labels.join(" / ")}`).not.toContain(
      "軸を追加",
    );
    expect(labels).not.toContain("この内容で置き換える");

    // **理由は言う** —— 黙ると、F-6 の穴が失敗経路でだけ残る。
    const notice = noAxesNotice();
    expect(notice, "軸が無いときの案内は、この状態でも出ること").not.toBeNull();
    const text = flatText(notice as HTMLElement);
    expect(text).toContain("軸が1本もありません");
    expect(text).toContain("判定");

    // **名指しはしない** —— 名指しする2つが、この画面に無い。
    expect(text, `名指ししていないこと: ${text}`).not.toMatch(/「[^」]+」/);
    expect(text).not.toContain("軸を追加");
    expect(text).not.toContain("この内容で置き換える");

    // **共有の判定でも通る**（他の状態と同じ物差しであること）。
    assertNoticeNamesOnlyRealOperations();

    await unmount(view, { outro: false });
  });

  it("案内が名指しした手順を、この画面から辿れる", async () => {
    const shown = await show(SORA);

    const text = flatText(noAxesNotice() as HTMLElement);
    const labels = [...target.querySelectorAll("button, a")].map(
      (element) => element.textContent?.trim() ?? "",
    );
    for (const operation of ["軸を追加", "この内容で置き換える"]) {
      expect(text, `案内が「${operation}」を名指しすること: ${text}`).toContain(
        `「${operation}」`,
      );
      expect(
        labels,
        `「${operation}」が画面に実在すること（実在するのは ${labels.join(" / ")}）`,
      ).toContain(operation);
    }

    await shown.dispose();
  });
});

/**
 * **総数を出す**（`RECENT_DRIFT_VOTES` の doc:
 * **「言えないまま切ると、画面は『これで全部』と嘘をつく」**）。
 */
describe("これで全部だ、と嘘をつかない", () => {
  it("上限で切られているときは、総数と見えていない件数を言う", async () => {
    const shown = await show(MIO);

    const total = voteTotal();
    expect(total.dataset["voteTotal"]).toBe("137");
    expect(total.textContent).toContain("137");
    // **見えていない票の数**（総数 137 − 出した 3）。
    expect(total.textContent).toContain("134");
    // **サーバが言った上限**（写していないので 50 ではない）。
    //
    // **⚠ `toContain("3")` と書かないこと**（レビューの指摘 / 2026-08-28）——
    // **同じ文に `137` と `134` があるので、何を出していても含まれる。**
    expect(flatText(total)).toContain("上限は 3 件");
    expect(total.textContent).not.toContain("50");
    // **出した件数は、総数ではない。**
    expect(voteRows()).toHaveLength(3);

    await shown.dispose();
  });

  /**
   * **⚠ この腕が要る理由**（`NAR_DRIFT` の doc）——
   * `mio` では上限と表示件数が同じ値なので、その取り違えが観測できない。
   */
  it("切られていないときは、上限と表示件数が別の数として出る", async () => {
    const shown = await show(NAR);

    const total = voteTotal();
    expect(total.dataset["voteTotal"]).toBe("2");
    expect(voteRows()).toHaveLength(2);
    // **上限は応答のもの。**
    expect(total.textContent).toContain("50");
    // **見えていない票は無い**ので、その言い方をしない。
    expect(total.textContent).not.toContain("見えていません");

    await shown.dispose();
  });

  it("票が1件も無いときは、無いと言う（表を出さない）", async () => {
    const shown = await show(AOI);

    expect(voteTotal().dataset["voteTotal"]).toBe("0");
    expect(voteRows()).toHaveLength(0);
    expect(target.textContent).toContain("まだ1件も");

    await shown.dispose();
  });
});

describe("置き換えは、全軸を送る", () => {
  it("1軸だけ下げても、触っていない軸まで送る", async () => {
    const shown = await show(MIO);

    typeValue("慎重さ", "1");
    submit();
    await vi.waitFor(() => {
      expect(calls.some((call) => call.method === "PUT")).toBe(true);
    });

    const put = calls.find((call) => call.method === "PUT");
    expect(put?.url).toBe(`/admin/characters/${MIO}/stats`);
    expect(put?.body).toEqual({
      stats: [
        { axis: "勇気", value: 0 },
        { axis: "慎重さ", value: 1 },
        { axis: "陽気さ", value: 5 },
      ],
    });

    await shown.dispose();
  });

  it("外した軸は送らない（丸ごと置き換えなので、それが「消す」）", async () => {
    const shown = await show(MIO);

    clickInRow(statRow("慎重さ"), "外す");
    expect(statRow("慎重さ").dataset["removed"]).toBe("true");

    submit();
    await vi.waitFor(() => {
      expect(calls.some((call) => call.method === "PUT")).toBe(true);
    });

    const put = calls.find((call) => call.method === "PUT");
    // **送り先も見る**（丸ごと置き換えが別のキャラクターへ飛んでいない）。
    expect(put?.url).toBe(`/admin/characters/${MIO}/stats`);
    expect(put?.body).toEqual({
      stats: [
        { axis: "勇気", value: 0 },
        { axis: "陽気さ", value: 5 },
      ],
    });

    await shown.dispose();
  });

  /**
   * **保存済みの軸名は書き換えられない。**
   *
   * 改名は**「消して足す」でしか表現できない**（`PUT` は丸ごと置き換えで、
   * 名前を変える操作は存在しない）。**その2手を1つの欄に見せると、
   * 名前を直したつもりの利用者が軸を1本消す。**
   *
   * # ⚠ 実際に何が起きるか（**このワークツリーで sqlite3 で実測 / 2026-08-28**）
   *
   * `replace_stat_allocs` が触るのは `stat_alloc` だけで、
   * **`stat_drift_charge` には `stat_alloc` への外部キーが無い**
   * （`character(id) ON DELETE CASCADE` だけ / `migrations/0012`）。だから:
   *
   * * **累積の行は消えない。** 改名しても `1.75` はそのまま残っていた。
   * * **それでも画面からは消える** —— `drift_view` は軸の一覧を `stat_alloc` から
   *   作るので、**古い名前の累積と上げ下げは、どの行にも出せなくなる。**
   *   新しい名前は 0 から始まる。
   * * **票は残り、総数には数えられ続ける** —— 画面は「全 N 件」と言うのに、
   *   その票は軸の表のどこにも現れない。
   * * **同じ綴りで足し直すと、累積は戻る**（行は消えていないので）。
   *
   * **守っているのは `{#if row.stored}` の1行だけ**なので、ここで見る
   * （レビューの実測: `{#if false}` にする変異が 372 全緑だった）。
   */
  it("保存済みの軸名は書き換えられない（改名は「消して足す」でしかない）", async () => {
    const shown = await show(MIO);

    expect(statRow("勇気").querySelector('input[name="axis"]')).toBeNull();
    expect(statRow("慎重さ").querySelector('input[name="axis"]')).toBeNull();
    // **軸名はセルの文字として出ている**（欄が無いだけで、消えてもいない）。
    expect(cellsOf(statRow("勇気"))).toEqual(["勇気", "/ 7", "外す"]);

    // **対照: 足した行には軸名の欄がある**（「欄が無い」を画面全体の性質にしない）。
    buttonLabelled("軸を追加").click();
    flushSync();
    const added = target.querySelector('tr[data-added="true"]') as HTMLElement;
    expect(added.querySelector('input[name="axis"]')).not.toBeNull();

    await shown.dispose();
  });

  it("外した軸は、戻せる", async () => {
    const shown = await show(MIO);

    clickInRow(statRow("慎重さ"), "外す");
    clickInRow(statRow("慎重さ"), "戻す");
    expect(statRow("慎重さ").dataset["removed"]).toBe("false");

    await shown.dispose();
  });

  it("軸を足せる（丸ごと置き換えなので、足すのも同じ1本）", async () => {
    const shown = await show(MIO);

    buttonLabelled("軸を追加").click();
    flushSync();
    const added = target.querySelector('tr[data-added="true"]');
    expect(added, "足した行があること").not.toBeNull();
    const axisField = (added as HTMLElement).querySelector(
      'input[name="axis"]',
    ) as HTMLInputElement;
    axisField.value = "度胸";
    axisField.dispatchEvent(new Event("input", { bubbles: true }));
    const valueField = (added as HTMLElement).querySelector(
      'input[name="value"]',
    ) as HTMLInputElement;
    valueField.value = "4";
    valueField.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();

    submit();
    await vi.waitFor(() => {
      expect(calls.some((call) => call.method === "PUT")).toBe(true);
    });
    const put = calls.find((call) => call.method === "PUT");
    expect(put?.body).toEqual({
      stats: [
        { axis: "勇気", value: 0 },
        { axis: "慎重さ", value: 2 },
        { axis: "陽気さ", value: 5 },
        { axis: "度胸", value: 4 },
      ],
    });

    await shown.dispose();
  });

  /**
   * **丸ごと置き換えであることが、押す前に読める。**
   *
   * 「送らない軸は消える」を画面が言わないと、
   * **外したつもりのない軸が消えたことに、押した後まで気付けない。**
   */
  it("保存すると何が残り、何が消えるかを、押す前に言う", async () => {
    const shown = await show(MIO);

    clickInRow(statRow("陽気さ"), "外す");
    const summary = target.querySelector("[data-replace-summary]");
    expect(summary).not.toBeNull();
    const text = (summary as HTMLElement).textContent ?? "";
    expect(text).toContain("勇気");
    expect(text).toContain("慎重さ");
    expect(text).toContain("陽気さ");
    expect(text).toContain("消え");

    await shown.dispose();
  });

  it("送れない値のままでは送らない", async () => {
    const shown = await show(MIO);

    typeValue("慎重さ", "-1");
    submit();
    flushSync();
    expect(calls.some((call) => call.method === "PUT")).toBe(false);
    // **どの軸の話かを名乗る**（他の軸の話に読めない）。
    const failed = target.querySelector("[data-build-error]");
    expect(failed, "送れない理由が画面に出ること").not.toBeNull();
    expect((failed as HTMLElement).textContent).toContain("慎重さ");
    expect((failed as HTMLElement).textContent).not.toContain("陽気さ");

    await shown.dispose();
  });

  it("送った後は、いまの値と変容を取り直す", async () => {
    const shown = await show(MIO);
    const before = calls.filter((call) => call.method === "GET").length;

    typeValue("慎重さ", "1");
    submit();
    await vi.waitFor(() => {
      expect(calls.filter((call) => call.method === "GET").length).toBe(
        before + 2,
      );
    });

    await shown.dispose();
  });
});

/**
 * **読み直しは、本当に読み直す。**
 *
 * # ⚠ なぜ「打った値が残っている」では見えないのか
 *
 * レビューの実測（2026-08-28）:
 * **`onclick={() => void load()}` を空にする変異も、
 * 下書きを作り直さない変異も、372 全緑だった。**
 * スタブが**いつも同じ値**を返していたので、
 * **「サーバから読み直した」と「打った値がそのまま残っている」が同じ見え方**になる。
 *
 * **だから、打った値とも最初の値とも違う値をサーバが返す腕を置く**
 * （＝別の窓が書き換えた回）。
 *
 * **⚠ これは B-15 の一歩先の同じ穴** —— B-15 は「案内した操作が存在しなかった」、
 * ここは**「存在するが何もしない」**。
 * **`SCALE_CHANGED_GUIDANCE` は利用者に「再読み込み」で取り直せと指示している。**
 */
describe("読み直しは、本当に読み直す", () => {
  /** 別の窓が書き換えた想定の値。**打った値とも最初の値とも違う。** */
  const CHANGED_BY_ANOTHER_WINDOW: StatsView = {
    character: MIO,
    stat_scale: 7,
    stats: [
      { axis: "勇気", value: 0 },
      { axis: "慎重さ", value: 4 },
      { axis: "陽気さ", value: 5 },
    ],
  };

  it("「再読み込み」で、サーバのいまの値に戻る（打ちかけは残らない）", async () => {
    const shown = await show(MIO);
    typeValue("慎重さ", "9");
    // **別の窓が書き換えた想定。** 打った値（9）とも、最初の値（2）とも違う。
    mioStatsNow = CHANGED_BY_ANOTHER_WINDOW;
    const before = calls.filter((call) => call.method === "GET").length;

    buttonLabelled("再読み込み").click();

    // **本当に引き直している**（押しても何もしない変異を落とす）。
    await vi.waitFor(() => {
      expect(calls.filter((call) => call.method === "GET").length).toBe(
        before + 2,
      );
    });
    // **下書きも作り直している**（打ちかけを残す変異を落とす）。
    await vi.waitFor(() => {
      expect(valueFieldOf("慎重さ").value).toBe("4");
    });
    expect(valueFieldOf("慎重さ").value).not.toBe("9");

    await shown.dispose();
  });

  /**
   * **保存した後も、サーバの値で作り直す。**
   *
   * **送った値をそのまま画面に残すと、丸ごと置き換え × 古い下書きになる**
   * ——次に押した「置き換える」が、**別の窓の書き込みを古い写しで踏み潰す。**
   */
  it("保存した後も、サーバの値で作り直す（送った値をそのまま残さない）", async () => {
    const shown = await show(MIO);
    typeValue("慎重さ", "1");
    // **こちらの `PUT` の後に、別の窓の書き込みが勝った想定。**
    mioStatsNow = CHANGED_BY_ANOTHER_WINDOW;

    submit();

    await vi.waitFor(() => {
      expect(valueFieldOf("慎重さ").value).toBe("4");
    });
    expect(valueFieldOf("慎重さ").value).not.toBe("1");

    await shown.dispose();
  });
});

describe("拒否されたときに、次の一手を出す", () => {
  it("値が尺度を超えた: 種別と案内が画面に出る", async () => {
    const shown = await show(MIO);
    putResponse = {
      status: 400,
      body: JSON.stringify({
        error: {
          message: "性格ステータスの値が尺度を超えています",
          type: "invalid_request_error",
          code: "stat_value_above_scale",
        },
      }),
    };

    typeValue("慎重さ", "9");
    submit();
    await vi.waitFor(() => {
      expect(
        target.querySelector('[data-replace-reason="value-above-scale"]'),
      ).not.toBeNull();
    });
    expect(target.querySelector("[data-replace-guidance]")).not.toBeNull();
    // **サーバが言ったことを、そのまま出す。**
    expect(target.textContent).toContain("尺度を超えています");

    await shown.dispose();
  });

  it("分類できない失敗には、次の一手を出さない（推測しない）", async () => {
    const shown = await show(MIO);
    putResponse = { status: 500, body: "壊れた本文" };

    typeValue("慎重さ", "1");
    submit();
    await vi.waitFor(() => {
      expect(
        target.querySelector('[data-replace-reason="unknown"]'),
      ).not.toBeNull();
    });
    expect(target.querySelector("[data-replace-guidance]")).toBeNull();

    await shown.dispose();
  });
});

describe("読めなかったとき", () => {
  it("いまの値が読めなくても、変容の側は出る（逆も同じ）", async () => {
    vi.stubGlobal("fetch", (input: unknown) => {
      const url = String(input);
      if (url.endsWith("/stats")) {
        return Promise.resolve({
          ok: false,
          status: 500,
          text: () => Promise.resolve("こわれた"),
        } as unknown as Response);
      }
      return Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(JSON.stringify(MIO_DRIFT)),
      } as unknown as Response);
    });

    const view = mount(StatsPanel, { target, props: { handle: MIO } });
    await vi.waitFor(() => {
      expect(target.querySelector("[data-vote-total]")).not.toBeNull();
    });
    // 変容は出ている。
    expect(driftRow("勇気")).not.toBeNull();
    // いまの値の側は、失敗として出る（0軸のキャラクターに見せない）。
    expect(target.querySelector("[data-stats-error]")).not.toBeNull();

    // **⚠ この状態には捨て軸が2本ある**（`MIO_DRIFT`）。
    // **案内を出すなら、名指しする操作が画面に無ければならない** ——
    // ここが穴だった（レビューの実測 / 2026-08-28。
    // 「軸を追加」も「この内容で置き換える」も、この枝では描かれない）。
    assertNoticeNamesOnlyRealOperations();
    // **事実の側は消えていない** —— 割り当てを失った行は表に出たまま。
    expect(driftRow("きえた軸")).not.toBeNull();
    expect(cellsOf(driftRow("きえた軸"))[1]).toBe("割り当てなし");
    // **手順の案内だけが消える**（置き換えができないので、書いても辿れない）。
    expect(
      target.querySelector("[data-unallocated-notice]"),
      "置き換えができない状態では、手順を書かないこと",
    ).toBeNull();

    await unmount(view, { outro: false });
  });
});

/**
 * **割り当てを失った軸**（作業単位 B-16）。
 *
 * `PUT .../stats` は丸ごと置き換えなので、**軸を改名すると綴りが変わる。**
 * サーバ側の累積と票は残り、**総数には数えられ続ける** ——
 * だからサーバは**その軸も返す**（`value: null`）。
 *
 * **画面が `?? 0` で埋めたら、値 0 の軸と同じ行になる。**
 * **生の `null` を埋め込んだら `null / 7` と出る。**
 * どちらも「落とさずに指し示す」ことにならない。
 */
describe("割り当てを失った軸を、値 0 の軸と混ぜない", () => {
  /**
   * **同じ応答の中に、値 0 の軸と割り当ての無い軸が居る。**
   *
   * # ⚠ セルを配列で突き合わせる
   *
   * `data-*` は**テストのためだけに置いた印**であって、
   * **読み手が何を見るかの権威ではない**（`cellsOf` の doc の実測 ——
   * `<td>` の入れ替えが 372 全緑を通った）。
   * **列ごと固定するので、値の欄が入れ替わればここで赤になる。**
   */
  it("値 0 の軸と、割り当ての無い軸が、別の見え方をする", async () => {
    const shown = await show(MIO);

    // 軸 / 値 / 累積 / 累積が動いた日時 / 上げ / 下げ。
    expect(cellsOf(driftRow("勇気"))).toEqual([
      "勇気",
      "0 / 7",
      "1.75",
      BRAVE_CHARGED_LABEL,
      "3",
      "1",
    ]);
    expect(cellsOf(driftRow("きえた軸"))).toEqual([
      "きえた軸",
      "割り当てなし",
      "1.5",
      LOST_CHARGED_LABEL,
      "0",
      "0",
    ]);
    expect(cellsOf(driftRow("むかしの軸"))).toEqual([
      "むかしの軸",
      "割り当てなし",
      "0",
      "記録なし",
      "0",
      "4",
    ]);

    // **2つの見え方が別物であること**を、行の外の言葉ではなく
    // **セルどうしを比べて**押さえる（両方を同じ文字列にする実装が赤になる）。
    const zeroCell = cellsOf(driftRow("勇気"))[1];
    const lostCell = cellsOf(driftRow("きえた軸"))[1];
    expect(lostCell).not.toBe(zeroCell);
    // **数を1文字も含まない**（尺度が漏れると `なし / 7` の形になる）。
    expect(lostCell).not.toContain("7");
    expect(lostCell).not.toContain("0");
    // **`null` がそのまま出ていない。**
    expect(flatText(driftRow("きえた軸"))).not.toContain("null");
    expect(flatText(target)).not.toContain("null");

    await shown.dispose();
  });

  /**
   * **印の側でも区別が付く。**
   *
   * セルの配列が権威だが、**`data-value` に `0` が入っていると、
   * 属性を読む道具（この先に足す測定を含む）が取り違える。**
   */
  it("印の側でも、割り当ての無い軸は値を名乗らない", async () => {
    const shown = await show(MIO);

    expect(driftRow("勇気").dataset["value"]).toBe("0");
    expect(driftRow("勇気").dataset["allocated"]).toBe("true");

    const lost = driftRow("きえた軸");
    expect(lost.dataset["allocated"]).toBe("false");
    // **属性そのものが無い**（実測 / 2026-08-28 —— Svelte は `null` の属性を落とす）。
    //
    // **⚠ `not.toBe("0")` にしないこと。** それは
    // **空文字でも `"null"` でも通る**ので、「値を名乗らない」を固定できない
    // （レビューの指摘 —— 変異で確かめたうえで、こちらへ変えた。
    // `data-value={axis.value ?? ""}` の変異が `not.toBe("0")` では緑を通る）。
    expect(lost.dataset["value"], "値を1つも名乗らないこと").toBeUndefined();
    expect(lost.hasAttribute("data-value")).toBe(false);
    // **対照: 割り当てのある軸では、その属性がある**（判定が何でも通るわけではない）。
    expect(driftRow("勇気").hasAttribute("data-value")).toBe(true);

    await shown.dispose();
  });

  /**
   * **累積だけの軸も、票だけの軸も、両方出る。**
   *
   * サーバ側で `an_axis_with_only_a_charge_and_an_axis_with_only_votes_are_both_listed`
   * が見ているのと同じ2つの状態。**片方だけを拾う実装をここでも落とす。**
   */
  it("累積だけの軸も、票だけの軸も落とさない", async () => {
    const shown = await show(MIO);

    expect(driftRow("きえた軸").dataset["charge"]).toBe("1.5");
    expect(driftRow("きえた軸").dataset["downs"]).toBe("0");
    expect(driftRow("むかしの軸").dataset["charge"]).toBe("0");
    expect(driftRow("むかしの軸").dataset["downs"]).toBe("4");

    await shown.dispose();
  });

  /**
   * **消えていることと、直し方を言う。**
   *
   * 直す手段は「同じ綴りで置き直す」しかない（票にも累積にも消す口が無い）。
   * **名指しした操作が実在することは `tests/stat-scale-path.test.ts` が機械的に見る。**
   */
  it("消えた軸があることと、直し方を画面が言う", async () => {
    const shown = await show(MIO);

    const notice = target.querySelector("[data-unallocated-notice]");
    expect(notice, "案内が出ること").not.toBeNull();
    const text = flatText(notice as HTMLElement);

    expect(text).toContain("『きえた軸』");
    expect(text).toContain("『むかしの軸』");
    // **割り当てのある軸は名指ししない。**
    expect(text).not.toContain("『勇気』");
    expect(text).not.toContain("『慎重さ』");
    expect(text).toContain("「軸を追加」");
    expect(text).toContain("「この内容で置き換える」");

    await shown.dispose();
  });

  /**
   * **消えた軸が無ければ、案内は出ない。**
   *
   * 出すと、**何も起きていない画面に「消えた軸があります」と書く**ことになる。
   * `nar` の変容は割り当てのある軸1本だけ（`NAR_DRIFT`）。
   */
  it("消えた軸が無いキャラクターには、案内が出ない", async () => {
    const shown = await show(NAR);

    expect(target.querySelector("[data-unallocated-notice]")).toBeNull();
    // **軸そのものは出ている**（画面が丸ごと壊れているのではない）。
    expect(cellsOf(driftRow("気だるさ"))).toEqual([
      "気だるさ",
      "8 / 9",
      "0.25",
      LANGUID_CHARGED_LABEL,
      "6",
      "1",
    ]);

    await shown.dispose();
  });

  /**
   * **案内が名指しした操作が、この画面に実在する。**
   *
   * `tests/stat-scale-path.test.ts` と同じ判定を、**この案内にも当てる。**
   * **無い操作を案内した瞬間に赤くなる。**
   */
  it("案内が名指しした操作は、この画面に実在する", async () => {
    const shown = await show(MIO);

    const text = flatText(
      target.querySelector("[data-unallocated-notice]") as HTMLElement,
    );
    const named = [...text.matchAll(/「([^」]+)」/g)].map(
      (found) => found[1] as string,
    );
    // **名指ししていること自体を見る**（「」を外す変異を空振りで通さない）。
    expect(named.length).toBeGreaterThan(0);

    const labels = new Set(
      [...target.querySelectorAll("button, a")].map(
        (element) => element.textContent?.trim() ?? "",
      ),
    );
    for (const operation of named) {
      expect(labels.has(operation), `「${operation}」が実在すること`).toBe(true);
    }
    // **対照: 判定が何でも通るわけではない。**
    expect(labels.has("票を削除")).toBe(false);

    // **同じ判定を、共有の形でも通す**（他の状態と同じ物差しであること）。
    assertNoticeNamesOnlyRealOperations();

    await shown.dispose();
  });

  /**
   * **置き換えの下書きには現れない。**
   *
   * 下書きは `GET .../stats` から作る（`draftOfStats`）ので、
   * **割り当ての無い軸はそこに居ない。**
   * 居たら、**保存のたびに消えた軸が値なしで送られる**ことになる。
   */
  it("消えた軸は、置き換えの下書きに混ざらない", async () => {
    const shown = await show(MIO);

    expect(target.querySelector('tr[data-axis="きえた軸"]')).toBeNull();
    expect(target.querySelector('tr[data-axis="むかしの軸"]')).toBeNull();
    // **変容の表の側には居る**（前提が崩れていないこと）。
    expect(target.querySelector('tr[data-drift-axis="きえた軸"]')).not.toBeNull();

    await shown.dispose();
  });
});

/**
 * **画面が、利用者に ADR の番号を見せない**（作業単位 F-8）。
 *
 * **文は消さない。** 「変容は本文に出ない」は
 * **利用者が知る必要のある事実**——**この画面の存在理由そのもの**なので、
 * **落とすのは出典だけ。**
 *
 * # ⚠ ソースの走査だけでは足りない
 *
 * `tests/screen-wording.test.ts` は**「出典が無いこと」しか言わない**ので、
 * **文ごと消しても緑になる。** ここは、**実際に描かれた DOM の上で文を見ている。**
 */
/**
 * **時刻が、生の ISO 表記で出ていない**（作業単位 F-7）。
 *
 * **瞬間は動かさない**（地域化しない / `src/lib/timestamp.ts` の doc）。
 * **落としたミリ秒は `title` に残す** —— ログや `sqlite3` の値と
 * 目で突き合わせる道を閉じないため。
 */
describe("時刻を、生の ISO 表記で出さない（作業単位 F-7）", () => {
  it("累積の欄も票の欄も、読める綴りになっている", async () => {
    const shown = await show(MIO);

    expect(cellsOf(driftRow("勇気"))[3]).toBe(BRAVE_CHARGED_LABEL);
    expect(cellsOf(voteRows()[0] as HTMLElement)[3]).toBe(FIRST_VOTE_LABEL);
    // **生の綴りは、画面のどこにも出ていない。**
    const text = flatText(target);
    expect(text).not.toContain(BRAVE_CHARGED_AT);
    expect(text).not.toContain("2026-08-27T09:00:00Z");

    await shown.dispose();
  });

  /**
   * **落とした精度を、`title` に残す。**
   *
   * `data-*` と違い、**`title` は読み手が実際に触れる**（重ねると出る）。
   * **同じ秒に立った票どうしは、これが無いと画面から区別できない**
   * ——1ターンで複数の軸が動くので、本番で普通に起きる。
   */
  it("生の綴りを `title` に残す（累積の欄と票の欄の両方）", async () => {
    const shown = await show(MIO);

    const chargeCell = driftRow("勇気").querySelectorAll("td")[3];
    expect(chargeCell?.getAttribute("title")).toBe(BRAVE_CHARGED_AT);
    // **対照: 別の軸には、その軸の綴りが入っている。**
    const otherCell = driftRow("陽気さ").querySelectorAll("td")[3];
    expect(otherCell?.getAttribute("title")).toBe(CHEER_CHARGED_AT);
    // **累積の行が無い軸は、残す綴りが無いので `title` も無い。**
    const missing = driftRow("慎重さ").querySelectorAll("td")[3];
    expect(missing?.getAttribute("title")).toBeNull();

    // **票の側は、行ごとに配列で突き合わせる**（下の doc / 端だけでは足りない）。
    expect(voteTitles()).toEqual([
      "2026-08-27T09:00:00Z",
      "2026-08-27T08:00:00Z",
      "2026-08-27T07:00:00Z",
    ]);

    await shown.dispose();
  });

  /**
   * **票の `title` が、その行のものである**（レビューの実測 / 2026-08-29）。
   *
   * # ⚠ 端だけを見ると、決め打ちが通り抜ける
   *
   * ここは以前 **`voteRows()[0]`（先頭行）しか見ていなかった**ので、
   * **`title={drift.recent_votes[0]?.created_at}`（＝常に先頭行の時刻）が
   * 631 全緑**を通った。**`D-2` の「選ぶ対象を端に置かない」の再発**である。
   *
   * **実害は、その `title` の存在理由を丸ごと無効化すること** ——
   * 置いた理由は「**同じ秒に立った票を区別できるように、生の綴りを残す**」なのに、
   * 変異後は**全行が同じ綴り**になる。**畳む前より悪い。**
   *
   * **⚠ 同じ画面の中で、累積の欄（2軸ぶん見ている）は 1 赤だった。**
   * **片方だけが守られていた。**
   *
   * # 3行を配列で見る
   *
   * `MIO_DRIFT` の票は**3件とも別の時刻**なので、
   * **「常に先頭」「常に末尾」「常に定数」のどれもここで赤になる。**
   */
  it("票の `title` が、行ごとに別の綴りになっている（端だけを見ない）", async () => {
    const shown = await show(MIO);

    const titles = voteTitles();
    expect(titles).toHaveLength(3);
    // **3つとも違う**（1つの値へ潰す決め打ちが赤になる）。
    expect(new Set(titles).size, `title が重複している: ${titles.join(" / ")}`).toBe(3);
    // **並びは表のまま**（真ん中の行が、真ん中の綴りを持つ）。
    expect(titles[1]).toBe("2026-08-27T08:00:00Z");
    // **見える綴りとは別物**（`title` が畳んだ側になっていない）。
    expect(titles[0]).not.toBe(FIRST_VOTE_LABEL);

    // **別のキャラクターでも、その応答の綴りが入る**（決め打ちを落とす2体目）。
    await shown.dispose();
    target.replaceChildren();

    const other = await show(NAR);
    expect(voteTitles()).toEqual([
      "2026-08-27T05:00:00Z",
      "2026-08-27T04:00:00Z",
    ]);

    await other.dispose();
  });
});

describe("画面が利用者の言葉で話す（作業単位 F-8）", () => {
  it("変容が本文に出ないことを言う（出典は出さない）", async () => {
    const shown = await show(MIO);

    const text = flatText(target);
    expect(text).toContain("変容は本文に出ません");
    // **どこで分かるのかまで言っている**（言わないと、読み手に行き先が無い）。
    expect(text).toContain("変容の記録");
    // **出典は画面に出さない。**
    expect(text).not.toContain("ADR");
    // **括弧の中が空にならない。**
    // **⚠ F-5 で報告された見え方は捕まえていない**（`character-form.test.ts` の同じ注記）。
    // ここが止めるのは「括弧だけ残して中身を落とす」変異。
    expect(text).not.toMatch(/（\s*）/);

    await shown.dispose();
  });
});
