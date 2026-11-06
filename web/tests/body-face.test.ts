/**
 * **会話の画面が言う、本文の面の状態と、送信を止めるか**（作業単位 D-16 / Issue origin#37 / ADR-0042 §決定3 の追記）。
 *
 * # ここで見るもの（**部品だけ** —— 純粋関数）
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **状態ごとの一言・送信を止めるか・裏方の一言** | **ここ** |
 * | 会話の画面が状態を読み、送信の釦に結ぶ（配線） | `chat-view-body-face.test.ts` |
 * | 状態の綴りがサーバの `FaceState::label` と一致する | `upstream-profiles.test.ts` の「写し元の綴り」 |
 *
 * # ⚠ 止めない腕を必ず置く
 *
 * **外の上流と、裏方だけが載せている状態では止めない**（ADR-0042 §決定3 の追記）。
 * 「止める」腕だけを見るテストは、**何でも止める実装**を緑で通す。
 *
 * # ⚠ フィクスチャの2つの面は、1つも同じ値にしない
 *
 * 本文と裏方を取り違える変異（`report.sub` を読む）を、値の違いで捕まえるため。
 */

import { describe, expect, it } from "vitest";

import type { LaunchView, UpstreamReport, UpstreamView } from "../src/lib/api";
import { bodyFaceStatus } from "../src/lib/bodyFace";

function launch(overrides: Partial<LaunchView>): LaunchView {
  return { state: "external", since: null, answered_name: null, reason: null, log: null, ...overrides };
}

function view(face: string, launched: LaunchView): UpstreamView {
  return {
    face,
    endpoint: face === "main" ? "http://127.0.0.1:43001" : "http://127.0.0.1:43002",
    model: face === "main" ? "main-model" : "sub-model",
    answered_name: null,
    queried_context_tokens: null,
    answered_by: null,
    unauthorized: false,
    checked_at: null,
    check_result: "not_checked",
    measured: { checked: 0, matched: 0, drifted_other: 0, drifted_absent: 0, unanswered: 0 },
    launch: launched,
  };
}

/**
 * **選んだプロファイルの名前は、どの行でも在る**（作業単位 F-10b）—— `not_started` 以外の状態で
 * 起こすボタンを出さないことを、名前が無いからではなく状態で決めていると見るため。
 */
const SELECTED = "beta";

function report(main: LaunchView, sub: LaunchView, selected: string | null = SELECTED): UpstreamReport {
  return {
    main: view("main", main),
    sub: view("sub", sub),
    profile_sample: null,
    selected_profile: selected,
  };
}

const READY_MAIN = launch({ state: "ready", answered_name: "main-12b.gguf" });
const READY_SUB = launch({ state: "ready", answered_name: "sub-3b.gguf" });
/** 読んだ時刻と読み手の時間帯（`LOADING_MAIN` の 12 秒後、東京）。 */
const NOW = new Date("2026-09-26T01:00:12Z");
const ZONE = "Asia/Tokyo";

const LOADING_MAIN = launch({ state: "loading", since: "2026-09-26T10:00:00+09:00" });
const LOADING_SUB = launch({ state: "loading", since: "2026-09-26T10:05:00+09:00" });
const FAILED_MAIN = launch({ state: "failed", reason: "子が自分で終了しました", log: "main-20260926-100000.err.log" });
/** 起こすのを待っている面（作業単位 F-10b / ADR-0055）。 */
const NOT_STARTED = launch({ state: "not_started" });
/** その一言と止める理由（**直書き** —— 実装の定数から作らない）。 */
const NOT_STARTED_LINE =
  "本文のモデル: まだ起こしていません（前回選んだプロファイルは、起動しただけでは起こしません）";
const NOT_STARTED_HOLD = "本文のモデルをまだ起こしていないので、送れません。";

describe("状態ごとの一言と、送信を止めるか", () => {
  /**
   * **1本の表で全部の腕を固定する**（一言・止めるか・理由・導線・裏方の一言）。
   * 行を足したら、止めない腕か止める腕かを必ず書く。
   */
  it.each([
    {
      name: "準備完了 → 止めない",
      at: report(READY_MAIN, READY_SUB),
      line: "本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）",
      hold: null,
      link: false,
      sub: null,
    },
    {
      name: "外の上流 → 止めない（状態を知らないので、止める根拠が無い）",
      at: report(launch({ state: "external" }), launch({ state: "external" })),
      line: "本文のモデル: 起こしていません（外の上流に依頼しています）",
      hold: null,
      link: false,
      sub: null,
    },
    {
      name: "裏方だけ載せている → 止めない（その旨だけ言う）",
      at: report(READY_MAIN, LOADING_SUB),
      line: "本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）",
      hold: null,
      link: false,
      sub: "裏方のモデルを載せています。載るまでの間、記憶の抽出と変容は行われません。",
    },
    {
      name: "外の上流で、裏方だけ載せている → 止めない",
      at: report(launch({ state: "external" }), LOADING_SUB),
      line: "本文のモデル: 起こしていません（外の上流に依頼しています）",
      hold: null,
      link: false,
      sub: "裏方のモデルを載せています。載るまでの間、記憶の抽出と変容は行われません。",
    },
    {
      name: "載せている → 止める",
      at: report(LOADING_MAIN, READY_SUB),
      line: "本文のモデル: 載せています（10:00:00 から、12 秒経過）",
      hold: "本文のモデルを載せている最中なので、まだ送れません。載ったら自動で送れるようになります。",
      link: false,
      sub: null,
    },
    {
      name: "差し替えの直後（切り替えの応答の形: 本文は載せ直し、裏方は残した子）→ 止める",
      at: report(launch({ state: "loading", since: "2026-09-26T11:30:00+09:00" }), READY_SUB),
      line: "本文のモデル: 載せています（11:30:00 から）",
      hold: "本文のモデルを載せている最中なので、まだ送れません。載ったら自動で送れるようになります。",
      link: false,
      sub: null,
    },
    {
      name: "失敗 → 止める（理由とログは一言に、導線を出す）",
      at: report(FAILED_MAIN, READY_SUB),
      line: "本文のモデル: 失敗しました: 子が自分で終了しました（ログ: upstream-logs の main-20260926-100000.err.log）",
      hold: "本文のモデルを載せられなかったので、送れません。",
      link: true,
      sub: null,
    },
    {
      name: "止めた → 止める（導線を出す）",
      at: report(launch({ state: "stopped" }), READY_SUB),
      line: "本文のモデル: 止めました",
      hold: "本文のモデルが止まっているので、送れません。",
      link: true,
      sub: null,
    },
    {
      name: "本文も裏方も載せている → 止める。裏方の一言は出さない（送れない理由は1つ）",
      at: report(LOADING_MAIN, LOADING_SUB),
      line: "本文のモデル: 載せています（10:00:00 から、12 秒経過）",
      hold: "本文のモデルを載せている最中なので、まだ送れません。載ったら自動で送れるようになります。",
      link: false,
      sub: null,
    },
    {
      name: "知らない状態 → 止めない（生の綴りを出す。止めて壊すより、今の上流エラーの形に倒す）",
      at: report(launch({ state: "sleeping" }), READY_SUB),
      line: "本文のモデル: sleeping",
      hold: null,
      link: false,
      sub: null,
    },
    // **起こしていない（作業単位 F-10b / ADR-0055 §決定3）。** 止めて、起こすボタンに選んだ名前を渡す。
    {
      name: "起こしていない → 止める。起こすボタンに、選んだ名前を渡す",
      at: report(NOT_STARTED, NOT_STARTED),
      line: NOT_STARTED_LINE,
      hold: NOT_STARTED_HOLD,
      link: false,
      sub: null,
      start: SELECTED,
    },
    {
      name: "本文だけ起こしていない（裏方は準備完了）→ 止める。起こすボタンを出す",
      at: report(NOT_STARTED, READY_SUB),
      line: NOT_STARTED_LINE,
      hold: NOT_STARTED_HOLD,
      link: false,
      sub: null,
      start: SELECTED,
    },
    {
      name: "起こしていないが、選んだ名前が無い → 止める。ボタンは出さず、上流モデルの画面への導線を出す",
      at: report(NOT_STARTED, launch({ state: "external" }), null),
      line: NOT_STARTED_LINE,
      hold: NOT_STARTED_HOLD,
      link: true,
      sub: null,
      start: null,
    },
    {
      name: "裏方だけ起こしていない → 止めない。起こすボタンも出さない（裏方の面の状態では止めない）",
      at: report(READY_MAIN, NOT_STARTED),
      line: "本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）",
      hold: null,
      link: false,
      sub: null,
      start: null,
    },
    {
      name: "外の上流で、裏方だけ起こしていない → 止めない",
      at: report(launch({ state: "external" }), NOT_STARTED),
      line: "本文のモデル: 起こしていません（外の上流に依頼しています）",
      hold: null,
      link: false,
      sub: null,
      start: null,
    },
  ])("$name", ({ at, line, hold, link, sub, start = null }) => {
    expect(bodyFaceStatus(at, null, NOW, ZONE)).toEqual({
      line,
      hold,
      linkToUpstream: link,
      subNote: sub,
      startProfile: start,
    });
  });

  it("まだ読めていない → 止めない（読み込み中と言う）", () => {
    expect(bodyFaceStatus(null, null, NOW, ZONE)).toEqual({
      line: "本文のモデル: 状態を読み込み中…",
      hold: null,
      linkToUpstream: false,
      subNote: null,
      startProfile: null,
    });
  });

  it("読めなかった → 止めない（理由を言い、止めていないと言う）", () => {
    expect(bodyFaceStatus(null, "内部エラーが発生しました", NOW, ZONE)).toEqual({
      line: "本文のモデル: 状態を読めませんでした（内部エラーが発生しました）。送信は止めていません。",
      hold: null,
      linkToUpstream: false,
      subNote: null,
      startProfile: null,
    });
  });

  /** **読めなかった後に、古い「起こしていない」でボタンを出し続けない**（作業単位 F-10b）。 */
  it("前に読めた値が起こしていないでも、読めなかったらボタンを出さない", () => {
    expect(
      bodyFaceStatus(report(NOT_STARTED, NOT_STARTED), "内部エラーが発生しました", NOW, ZONE).startProfile,
    ).toBeNull();
  });

  /**
   * **読めなかったことは、前に読めた値より優先する**（古い「載せています」で止め続けない）。
   * 画面は読み直しに失敗したら `report` を捨てる運用だが、関数の側でもその順を固定する。
   */
  it("前に読めた値があっても、読めなかったことを言い、止めない", () => {
    expect(bodyFaceStatus(report(LOADING_MAIN, READY_SUB), "内部エラーが発生しました", NOW, ZONE).hold).toBeNull();
  });
});

describe("本文の面を読む（裏方の面では止めない）", () => {
  /** **面を取り違えると、ここで逆になる**（本文は準備完了、裏方は失敗）。 */
  it("裏方が失敗していても、本文が準備完了なら止めない", () => {
    const status = bodyFaceStatus(
      report(READY_MAIN, launch({ state: "failed", reason: "裏方の子が終了しました", log: null })),
      null,
      NOW,
      ZONE,
    );
    expect(status.hold).toBeNull();
    expect(status.line).toBe("本文のモデル: 準備完了（上流が名乗った名前: main-12b.gguf）");
  });

  it("本文が失敗していれば、裏方が準備完了でも止める", () => {
    expect(bodyFaceStatus(report(FAILED_MAIN, READY_SUB), null, NOW, ZONE).hold).not.toBeNull();
  });
});
