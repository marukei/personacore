// @vitest-environment jsdom
/**
 * **画面が、読み手の時間帯を解決して書式へ渡している**（作業単位 F-7b）。
 *
 * # なぜ別のファイルに置くのか
 *
 * **「配線だけの場所」は、層に関係なく観測対象から落ちる**（CLAUDE.md）。
 * 実例が同じリポジトリにある —— **`D-3` の時点で `web/src/App.svelte` は
 * どのテストからも import されておらず、`handle={screen.handle}` を
 * `handle="aoi"` にする変異が 107 全緑**だった。
 *
 * **⚠ その穴は、その後に塞がっている**（F-7b のレビューが数え直した / 2026-08-29）——
 * いま `App.svelte` は4本が mount し（`app-routing` /
 * `app-routing-characters` / `app-routing-stats` / `stat-scale-path`）、
 * `screen-wording` が走査で読む。
 * **「いまも無防備だ」ではなく、「そう実測された日があり、その後塞がった」。**
 * **引いている教訓は変わらない** ——
 * **配線は、それ自体を見る場所を作らないと誰も通らない。**
 *
 * `timestamp.test.ts` は**書式**を見る（時間帯を名指しで渡すので機械に依らない）。
 * **ここは「誰がその時間帯を決めたか」だけを見る。**
 * 2つを混ぜると、**書式が正しければ配線も正しいように読めてしまう。**
 *
 * # ⚠ 2つの時間帯で、同じフィクスチャを回す
 *
 * **`B-7` の形**（CLAUDE.md）—— **実行時に決まる値を入れたら、
 * その値が既定と違う状態で回る腕を1本置くこと。**
 * **1つの時間帯だけで回すと、「ブラウザに訊いている」と
 * 「その時間帯を決め打っている」が区別できない。**
 *
 * **この機の既定は `Asia/Tokyo`** なので、片方だけなら
 * `const zone = "Asia/Tokyo";` という変異が緑を通る。
 * **2つを別々に名指しすれば、どの1つを決め打っても片方で赤になる**
 * ——そして**それはどんな機械で走らせても成り立つ**（機械の既定に依らない）。
 *
 * # ⚠ 日をまたぐ瞬間を使う
 *
 * **同値のフィクスチャでは帰属を見張れない。**
 * `2026-08-28T20:00:00Z` は**東京では翌日の 05:00、ニューヨークでは同日の 16:00**
 * ——**日付そのものが違う**ので、時間帯を無視する変異は
 * 「時が少しずれる」ではなく「日付が違う」という形で必ず出る。
 *
 * # ⚠ 4つの呼び出し口を、1つずつ当てる
 *
 * **写しを作ったら、列・分岐・枝の1つずつに当てること。代表1つでは足りない**
 * （CLAUDE.md / `D-6` は**1列だけが写しから漏れて 262 全緑**だった）。
 * `timeLabel` / `chargeTimeLabel` の呼び出し口は**4つ**ある ——
 * 記憶の一覧・上書き済みの鎖・変容の票・累積の時刻。**4つとも下に居る。**
 *
 * **`browserZone()` の呼び出し口はこの他に2つある**（Issue origin#40 / D-18）——
 * 上流の状態の文面（`launchLabel`）を出す `ChatView.svelte` の帯と `UpstreamPanel.svelte` の面ごとのカード。
 * **同じ形（日をまたぐ瞬間を東京とニューヨークで）で、それぞれの画面の測定に居る** ——
 * `tests/chat-view-body-face.test.ts` の「帯の時刻は、読み手の時間帯で出る」と
 * `tests/upstream-profiles-panel.test.ts` の「状態の行の時刻は、… の読み手の時間帯で出る」
 * （上流の偽物がそちらにあるため）。
 *
 * # `process.env.TZ` を触ってよい理由
 *
 * **測定ファイルはフォークされた別プロセスで走る**（`vitest.config.ts` の
 * `pool: "forks"` / 実測: 2 ファイルを同時に走らせると別々の pid）。
 * **他のファイルへ漏れない。** それでも各測定の後で機械の値へ戻す。
 *
 * # ⚠ **`TZ` が効かないと、東京の腕だけが緑で残っていた**（作業単位 F-7c）
 *
 * `--pool=threads` で走らせると `process.env.TZ` は ICU に届かない
 * （**CLI は `vitest.config.ts` の固定に勝つ** / F-7b の実測）。
 * そのとき **17 赤の中で、このファイルは 7 本のうち 2 本しか落ちなかった。**
 * 残った 5 本の内訳は、**東京の腕が3本**（**`TZ` が効かず、この機がはじめから東京だから**）と、
 * **ニューヨークを名指ししていたのに東京でも成立する主張が2本**
 * （`title` に生の綴りが残る／読める場所に ISO が出ていない）。
 * **赤の山に偶然の緑が混ざる。**
 *
 * **手当ては [`enterAsReader`] 1つ**（`tests/support/zone.ts`）——
 * **機械の既定とも目的とも違う時間帯を先に一度立ててから**、目的の時間帯へ移る。
 * **東京を頼む腕も、先にニューヨークを通る**ので、`TZ` が死んでいればそこで落ちる。
 *
 * **見ているのは pool ではなく「`TZ` が効いていること」**である
 * （権威は `tests/zone-control.test.ts`）。
 *
 * **実測**（F-7c / `--pool=threads` で走らせたときの、このファイルだけの内訳）:
 *
 * | 状態 | このファイル |
 * |---|---|
 * | F-7c の前（`process.env.TZ` を立てるだけ） | **2 赤 / 5 緑**（東京の腕3本 ＋ ニューヨークを使う2本が偶然の緑） |
 * | [`enterZone`] だけ（探りなし） | **4 赤 / 3 緑**（**東京の腕3本が残る**） |
 * | [`enterAsReader`]（納品状態） | **7 本とも赤**（**偶然の緑ゼロ**） |
 *
 * **⚠ 1行目と2行目は「同じ2本」ではない。**
 * `enterZone` を入れると、**ニューヨークを名乗る4本が全部落ちる**ようになる
 * （偶然の緑だった2本が、時間帯を立てる時点で落ちるため）。
 * **東京の腕3本だけは、探りを通すまで落ちない。**
 *
 * **ブラウザは起動しない**（jsdom だけ / CLAUDE.md 4節）。
 *
 * [`enterAsReader`]: ./support/zone
 * [`enterZone`]: ./support/zone
 */

import { mount, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import MemoryList from "../src/lib/MemoryList.svelte";
import SessionPicker from "../src/lib/SessionPicker.svelte";
import StatsPanel from "../src/lib/StatsPanel.svelte";
import type { DriftView, MemoryView, StatsView } from "../src/lib/api";
import { enterAsReader, machineZone } from "./support/zone";

/**
 * このファイルを読み込んだ時点の、機械の時間帯。**必ずここへ戻す。**
 *
 * **⚠ [`machineZone`] で読むこと**（作業単位 F-7d）——
 * `Intl.DateTimeFormat().resolvedOptions().timeZone` は**門を通る**ので、
 * **まだ名乗っていないこの時点で呼ぶと落ちる。**
 *
 * [`machineZone`]: ./support/zone
 */
const MACHINE_ZONE = machineZone();

/** 対象のキャラクター。 */
const HANDLE = "mio";

/**
 * **日をまたぐ瞬間**（`20:00Z`）。
 *
 * 東京では**翌日の 05:00**、ニューヨークでは**同日の 16:00**。
 * **ミリ秒つき**なので、`title` に残す生の綴りと畳んだ見出しが必ず別物になる。
 */
const CROSSING = "2026-08-28T20:00:00.500Z";

/**
 * 4つの欄が**それぞれ別の瞬間**を持つ（取り違えが見えるように）。
 * **どれも日をまたぐ**（`19:00Z` 以降なら東京で翌日になる）。
 */
const CHAIN_AT = "2026-08-28T21:30:00.250Z";
const VOTE_AT = "2026-08-28T22:15:00.750Z";
const CHARGE_AT = "2026-08-28T23:45:00.125Z";

/**
 * **東京で見たときの綴り**（UTC+9 / 夏時間なし）。
 *
 * **期待値は直書き。** `timeLabel` から作ると、
 * **綴りを変える変異が期待値も一緒に動かして緑**になる（CLAUDE.md）。
 * **手で足した** —— 20:00 + 9:00 = 翌日 05:00。
 */
const IN_TOKYO = {
  crossing: "2026-08-29 05:00:00 (UTC+09:00)",
  chain: "2026-08-29 06:30:00 (UTC+09:00)",
  vote: "2026-08-29 07:15:00 (UTC+09:00)",
  charge: "2026-08-29 08:45:00 (UTC+09:00)",
} as const;

/**
 * **ニューヨークで見たときの綴り**（8 月は EDT = UTC-4）。
 *
 * **手で引いた** —— 20:00 - 4:00 = 同日 16:00。
 * **東京とは日付も時も違う**ので、どちらか一方の決め打ちは必ず片方で落ちる。
 */
const IN_NEW_YORK = {
  crossing: "2026-08-28 16:00:00 (UTC-04:00)",
  chain: "2026-08-28 17:30:00 (UTC-04:00)",
  vote: "2026-08-28 18:15:00 (UTC-04:00)",
  charge: "2026-08-28 19:45:00 (UTC-04:00)",
} as const;

/** いま想起される記憶（1件で足りる ——ここは帰属ではなく配線を見る）。 */
const ACTIVE: MemoryView = {
  id: 43,
  subject: "user",
  content: "紅茶を飲む",
  category: null,
  created_at: CROSSING,
};

/** 上書き済みの行（**鎖の表は別の呼び出し口**）。 */
const SUPERSEDED = {
  id: 41,
  subject: "user" as const,
  content: "珈琲を飲む",
  category: null,
  created_at: CHAIN_AT,
  superseded_by: 43,
};

const STATS: StatsView = {
  character: HANDLE,
  stat_scale: 7,
  stats: [{ axis: "勇気", value: 3 }],
};

const DRIFT: DriftView = {
  character: HANDLE,
  stat_scale: 7,
  axes: [
    {
      axis: "勇気",
      value: 3,
      charge: 1.75,
      charge_updated_at: CHARGE_AT,
      ups: 3,
      downs: 1,
    },
  ],
  total_votes: 9,
  recent_limit: 3,
  recent_votes: [
    { axis: "勇気", delta: "up", session_id: 41, created_at: VOTE_AT },
  ],
  run_mode: "on",
  run_mode_source: "default",
  default_run_mode: "on",
};

let target: HTMLElement;

function stubFetch(): void {
  vi.stubGlobal("fetch", (input: unknown) => {
    const url = String(input);
    const respond = (body: unknown): Promise<Response> =>
      Promise.resolve({
        ok: true,
        status: 200,
        text: () => Promise.resolve(JSON.stringify(body)),
      } as unknown as Response);

    if (url.startsWith(`/admin/characters/${HANDLE}/memories`)) {
      return respond({
        character: HANDLE,
        memories: [ACTIVE],
        unreadable: [],
        superseded: {
          memories: [SUPERSEDED],
          unreadable: [],
          total: 1,
          limit: 100,
        },
      });
    }
    if (url === `/admin/characters/${HANDLE}/stats`) {
      return respond(STATS);
    }
    if (url === `/admin/characters/${HANDLE}/drift`) {
      return respond(DRIFT);
    }
    return Promise.resolve({
      ok: false,
      status: 404,
      text: () => Promise.resolve("{}"),
    } as unknown as Response);
  });
}

/**
 * その節の、その行のセル。
 *
 * **セルの配列が権威**（`data-*` はテストが置いた印であって、
 * 読み手が何を見るかの権威ではない / `D-4` のレビューの実測）。
 */
function cellsIn(section: string): string[] {
  const row = target.querySelector(`tbody[data-section="${section}"] tr`);
  expect(row, `${section} に行があること`).not.toBeNull();
  return [...(row as HTMLElement).querySelectorAll("td")].map((cell) =>
    (cell.textContent ?? "").replace(/\s+/gu, " ").trim(),
  );
}

/** その節の、その行の `title` 属性（**生の綴りが残る場所**）。 */
function titlesIn(section: string): (string | null)[] {
  const row = target.querySelector(`tbody[data-section="${section}"] tr`);
  expect(row, `${section} に行があること`).not.toBeNull();
  return [...(row as HTMLElement).querySelectorAll("td")].map((cell) =>
    cell.getAttribute("title"),
  );
}

/**
 * その時間帯の読み手として、記憶の一覧を開く。
 *
 * **`TZ` は mount より前に立てる** —— 画面は描くときに
 * `browserZone()` を呼ぶので、後から立てても間に合わない。
 *
 * **⚠ `enterAsReader` を通す**（作業単位 F-7c）—— 立てただけで確かめないと、
 * **`TZ` が効かない環境で「機械の既定の時間帯」を読み手として測ってしまう。**
 * **`enterZone` では足りない** —— **東京を頼む腕は、この機では素通りする。**
 */
async function openMemories(zone: string): Promise<ReturnType<typeof mount>> {
  enterAsReader(zone);
  const view = mount(MemoryList, { target, props: { handle: HANDLE } });
  await vi.waitFor(() => {
    // **「上書き済み」の節が並ぶまで待つ**（4つの呼び出し口のうち2つがここ）。
    expect(
      target.querySelector('tbody[data-section="stored-superseded"] tr'),
    ).not.toBeNull();
  });
  return view;
}

/** 同上、変容の画面。 */
async function openStats(zone: string): Promise<ReturnType<typeof mount>> {
  enterAsReader(zone);
  const view = mount(StatsPanel, { target, props: { handle: HANDLE } });
  await vi.waitFor(() => {
    expect(
      target.querySelector('tbody[data-section="drift-votes"] tr'),
    ).not.toBeNull();
    expect(
      target.querySelector('tbody[data-section="drift-axes"] tr'),
    ).not.toBeNull();
  });
  return view;
}

beforeEach(() => {
  target = document.createElement("div");
  document.body.appendChild(target);
  stubFetch();
});

afterEach(() => {
  vi.unstubAllGlobals();
  target.remove();
  // **機械の値へ戻す。** `delete` では戻らない（ICU は消しても直前の値を覚えている）。
  process.env["TZ"] = MACHINE_ZONE;
});

describe("画面が、読み手の時間帯を解決して渡す（作業単位 F-7b）", () => {
  /**
   * **記憶の一覧**（呼び出し口 1 / 2）。
   *
   * **有効な行の表と、上書き済みの鎖の表は別の呼び出し口。**
   * 片方だけ見ると、もう片方が写しから漏れても緑になる（`D-6` の形）。
   */
  it("記憶の一覧が、東京の読み手には東京の日付で出る", async () => {
    const view = await openMemories("Asia/Tokyo");

    expect(cellsIn("active")[4]).toBe(IN_TOKYO.crossing);
    expect(cellsIn("stored-superseded")[4]).toBe(IN_TOKYO.chain);

    await unmount(view, { outro: false });
  });

  /**
   * **同じフィクスチャ・別の時間帯**（`B-7` の腕）。
   *
   * **東京を決め打つ変異は、ここで落ちる。**
   * **UTC のまま出す変異も落ちる**（`(UTC)` は `(UTC-04:00)` ではない）。
   */
  it("記憶の一覧が、ニューヨークの読み手にはニューヨークの日付で出る", async () => {
    const view = await openMemories("America/New_York");

    expect(cellsIn("active")[4]).toBe(IN_NEW_YORK.crossing);
    expect(cellsIn("stored-superseded")[4]).toBe(IN_NEW_YORK.chain);

    await unmount(view, { outro: false });
  });

  /**
   * **変容の画面**（呼び出し口 3 / 4）。
   *
   * **票の日時と、累積が動いた日時は別の呼び出し口**
   * （後者は `chargeTimeLabel` を通る）。
   */
  it("変容の表が、東京の読み手には東京の日付で出る", async () => {
    const view = await openStats("Asia/Tokyo");

    expect(cellsIn("drift-votes")[3]).toBe(IN_TOKYO.vote);
    expect(cellsIn("drift-axes")[3]).toBe(IN_TOKYO.charge);

    await unmount(view, { outro: false });
  });

  /** 同上、別の時間帯（**どちらか一方の決め打ちは、必ずどちらかで落ちる**）。 */
  it("変容の表が、ニューヨークの読み手にはニューヨークの日付で出る", async () => {
    const view = await openStats("America/New_York");

    expect(cellsIn("drift-votes")[3]).toBe(IN_NEW_YORK.vote);
    expect(cellsIn("drift-axes")[3]).toBe(IN_NEW_YORK.charge);

    await unmount(view, { outro: false });
  });

  /**
   * **⚠ 4つの欄が、それぞれ自分の瞬間を出している。**
   *
   * **帰属**（CLAUDE.md「抜けるのはいつも帰属」）——
   * 4つの瞬間を全部違う値にしてあるので、
   * **どれか1つを他の欄へ出す取り違えは、ここで見える。**
   * **時間帯が正しくても、どの時刻かを取り違えていれば意味が無い。**
   */
  it("4つの欄が、それぞれ別の瞬間を出す", async () => {
    const memories = await openMemories("Asia/Tokyo");
    const seenMemories = [
      cellsIn("active")[4],
      cellsIn("stored-superseded")[4],
    ];
    await unmount(memories, { outro: false });

    const stats = await openStats("Asia/Tokyo");
    const seenStats = [cellsIn("drift-votes")[3], cellsIn("drift-axes")[3]];
    await unmount(stats, { outro: false });

    const seen = [...seenMemories, ...seenStats];
    expect(seen).toEqual([
      IN_TOKYO.crossing,
      IN_TOKYO.chain,
      IN_TOKYO.vote,
      IN_TOKYO.charge,
    ]);
    // **4つとも別の綴り**（同値化していないことを値そのもので押さえる）。
    expect(new Set(seen).size).toBe(4);
  });

  /**
   * **生の綴りを `title` に残す**（作業単位 F-7 の性質 / 回帰）。
   *
   * **⚠ 地域化しても、`title` は動かさない。**
   * ここが**ログ・`sqlite3` の UTC と1文字ずつ突き合わせる唯一の道**であり、
   * **同じ秒に立った行を画面から区別する唯一の道**でもある
   * （1ターンで複数の記憶・複数の票が入るので、本番で普通に起きる）。
   *
   * **その行のものであること**まで見る ——
   * 別の行の生の綴りを貼る変異は、値が違うのでここで落ちる。
   */
  it("見出しを地域化しても、`title` の生の綴りはサーバの値のまま", async () => {
    const memories = await openMemories("America/New_York");
    expect(titlesIn("active")[4]).toBe(ACTIVE.created_at);
    expect(titlesIn("stored-superseded")[4]).toBe(SUPERSEDED.created_at);
    // **見出しの側は、生の綴りではない**（`title` を見出しにも使う実装が落ちる）。
    expect(cellsIn("active")[4]).not.toBe(ACTIVE.created_at);
    await unmount(memories, { outro: false });

    const stats = await openStats("America/New_York");
    expect(titlesIn("drift-votes")[3]).toBe(VOTE_AT);
    expect(titlesIn("drift-axes")[3]).toBe(CHARGE_AT);
    expect(cellsIn("drift-votes")[3]).not.toBe(VOTE_AT);
    await unmount(stats, { outro: false });
  });

  /**
   * **生の ISO 表記が、画面のどこにも出ていない**（F-7 の存在理由 / 回帰）。
   *
   * `title` は属性なので `textContent` には出ない。
   */
  it("生の ISO 表記が、読める場所に出ていない", async () => {
    const view = await openMemories("America/New_York");

    const text = target.textContent ?? "";
    expect(text).not.toContain(CROSSING);
    expect(text).not.toContain("T20:00:00");
    expect(text).not.toContain(".500Z");
    // **時間帯データベースの識別子も出さない**（作業単位 F-8 と同じ目）。
    expect(text).not.toContain("America/New_York");

    await unmount(view, { outro: false });
  });
});

/**
 * **会話の一覧**（作業単位 D-12c-2 / **5つ目の呼び出し口**）。
 *
 * **上の「4つの欄」の1本には混ぜない** —— あちらは記憶と変容の画面を開く測定で、
 * この部品は会話の画面の中で開く。**同じ2つの時間帯で回す**（`B-7` の腕）。
 * **時刻は日をまたぐ瞬間**（東京では翌日、ニューヨークでは同日）。
 */
describe("会話の一覧も、読み手の時間帯を解決して渡す（作業単位 D-12c-2）", () => {
  /** 会話の更新日時。**他の4つの瞬間のどれとも違う。** */
  const SESSION_AT = "2026-08-28T20:30:00.375Z";

  function openSessions(zone: string): ReturnType<typeof mount> {
    enterAsReader(zone);
    return mount(SessionPicker, {
      target,
      props: {
        handle: HANDLE,
        list: {
          character: HANDLE,
          sessions: [
            {
              id: 12,
              label: null,
              created_at: "2026-08-01T00:00:00Z",
              updated_at: SESSION_AT,
              message_count: 2,
              first_user_message: "こんにちは",
              first_user_message_truncated: false,
            },
          ],
          total_sessions: 1,
          sessions_limit: 100,
        },
        current: null,
        busy: false,
        onChoose: () => undefined,
        onBeginNew: () => undefined,
      },
    });
  }

  function when(): HTMLElement {
    const found = target.querySelector<HTMLElement>("li[data-session-id] .when");
    expect(found).not.toBeNull();
    return found as HTMLElement;
  }

  it("東京の読み手には東京の日付で出る", async () => {
    const view = openSessions("Asia/Tokyo");
    expect(when().textContent?.trim()).toBe("2026-08-29 05:30:00 (UTC+09:00)");
    await unmount(view, { outro: false });
  });

  it("ニューヨークの読み手にはニューヨークの日付で出る", async () => {
    const view = openSessions("America/New_York");
    expect(when().textContent?.trim()).toBe("2026-08-28 16:30:00 (UTC-04:00)");
    // **生の綴りは `title` に残り、読める場所には出ない。**
    expect(when().getAttribute("title")).toBe(SESSION_AT);
    expect(target.textContent).not.toContain(SESSION_AT);
    await unmount(view, { outro: false });
  });
});
