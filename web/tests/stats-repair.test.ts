/**
 * **いまの値が読めないときの、修復の組み立て**（作業単位 D-8 / `src/lib/statsRepair.ts`）。
 *
 * # この作業単位の存在理由
 *
 * **`GET .../stats` が 500 になる状態がある**（尺度を超える値・孤児・範囲外・
 * **正規化すると同じになる軸の重複** / 作業単位 B-19）。
 * **そのとき画面に残っていたのは「再読み込み」と「キャラクターの一覧へ」だけ**で、
 * **「軸を追加」も「この内容で置き換える」も `{:else}` 側のフォームの中にしか無かった。**
 * **直すには `curl` か `sqlite3` が要った。**
 *
 * **⚠ API は開いていた。** `Store::replace_stat_allocs` は既存の軸を読まないので、
 * 壊れた行があっても `PUT .../stats` は 200 を返す
 * （`tests/server_admin.rs::a_row_beyond_the_scale_can_be_repaired_through_the_stats_endpoint`）。
 * **閉じていたのは画面だけ。**
 *
 * # `stats-repair-panel.test.ts` との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **判定・写し元・下書き**（I/O を持たない） | **ここ** |
 * | 画面が何を出すか・どこへ送るか | `stats-repair-panel.test.ts` |
 *
 * **jsdom を使わない**（`import.meta.url` が `file:` でなくなり、
 * 写し元を読む見張り番が置けなくなる —— このワークツリーで実測）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { ApiRequestError } from "../src/lib/api";
import type { CharacterListView, CharacterView } from "../src/lib/api";
import { NOT_FOUND_CODE } from "../src/lib/statsEdit";
import {
  MALFORMED_LISTING_NOTICE,
  UNKNOWN_LOSS_NOTICE,
  UNKNOWN_SCALE_NOTICE,
  describeStatsFailure,
  emptyRepairDraft,
  repairEntryFor,
  scaleFromListing,
  type ScaleLookup,
  type StatsReadFailure,
} from "../src/lib/statsRepair";

/** 直す相手。**一覧の真ん中に置く**（端を押さない / `D-2` のレビューの実測）。 */
const MIO = "mio";
/** **前の行。** 「一覧の先頭を取る」実装はこちらの尺度を出す。 */
const AOI = "aoi";
/** **後ろの行。** 「一覧の末尾を取る」実装はこちらの尺度を出す。 */
const NAR = "nar";
/** **キャラクターの行そのものが壊れている相手。** */
const KURO = "kuro";

/**
 * ⚠ **尺度は3件とも別の値**（同値化を避ける / CLAUDE.md）。
 * **どれも `StatScale::DEFAULT`（10）ではない** ——
 * 既定を書き込む実装と区別が付かなくなる。
 */
const AOI_SCALE = 5;
const MIO_SCALE = 7;
const NAR_SCALE = 9;

function characterOf(handle: string, id: number, scale: number): CharacterView {
  return {
    id,
    handle,
    display_name: `${handle}の呼び名`,
    base_persona: `${handle}のペルソナ。`,
    output_format_instruction: null,
    stat_scale: scale,
    stat_drift_mode: null,
    axis_discovery_slots: 0,
    archived_at: null,
    created_at: "2026-08-28T00:00:00Z",
  };
}

/** 一覧の応答。**対象は2件目**（先頭でも末尾でもない）。 */
const LISTING: CharacterListView = {
  characters: [
    characterOf(AOI, 1, AOI_SCALE),
    characterOf(MIO, 2, MIO_SCALE),
    characterOf(NAR, 3, NAR_SCALE),
  ],
  unreadable: [
    // **識別子は読めるが、他の列が読めない行**（D-6 の修復の対象）。
    { id: 4, handle: KURO, invalid_columns: ["base_persona"] },
    // **識別子そのものが読めない行**（API からは指せない / D-6）。
    { id: 5, handle: null, invalid_columns: ["handle"] },
  ],
};

describe("読めなかった理由を、分けられるところまで分ける", () => {
  const failureOf = (status: number, code: string | null): StatsReadFailure =>
    describeStatsFailure(
      new ApiRequestError("サーバの文言", status, "server_error", code),
    );

  it("届いていない回（状態番号 0）は、サーバの失敗と別物", () => {
    expect(failureOf(0, null).kind).toBe("unreachable");
    // **対照: 同じ `null` の `code` でも、届いていれば別の種別。**
    expect(failureOf(500, null).kind).toBe("server");
  });

  it("居ない回（`character_not_found`）は、読めない回と別物", () => {
    expect(failureOf(404, NOT_FOUND_CODE).kind).toBe("missing");
    // **対照: `code` が違えば `missing` にしない。**
    expect(failureOf(404, "invalid_body").kind).toBe("server");
  });

  it("`ApiRequestError` ですらないものは、中身を画面へ流さない", () => {
    const failure = describeStatsFailure(
      new Error("F:\\秘密\\personacore.sqlite"),
    );
    expect(failure.kind).toBe("unknown");
    expect(failure.message).toBe("予期しない失敗です");
    expect(failure.message).not.toContain("personacore.sqlite");
  });

  it("サーバが言ったことを、そのまま持つ", () => {
    expect(failureOf(500, null).message).toBe("サーバの文言");
  });
});

/**
 * **「壊れている」と「読めない」を混ぜない**（作業単位 D-8 の設計の根拠）。
 *
 * **`ApiError::internal()` に `code` は無い。**
 * つまり **FE には「格納値が壊れた 500」と「DB 障害の 500」を分ける手掛かりが無い。**
 * **画面が「壊れています」と書いたら、それは推測である。**
 *
 * **写し元を実際に読む**（先例は `stats-edit.test.ts` の `code` /
 * `character-delete.test.ts` / `character-repair.test.ts`）。
 * **`code` が生えた日にこの主張は嘘になる**ので、そのとき赤くなる形にしておく。
 */
describe("500 に細分類が無いことを、写し元で確かめる", () => {
  const errorSource = (): string =>
    readFileSync(
      fileURLToPath(new URL("../../src/server/error.rs", import.meta.url)),
      "utf8",
    );

  /**
   * 引数の無い構築子の本体から、`code:` の右辺を取り出す。
   *
   * **⚠ 改行を先に畳む。** 作業ツリーの `.rs` は CRLF なので、
   * 畳まないと `None` ではなく `None,` ＋ 復帰文字を読むことになる
   * （このワークツリーで実測。CLAUDE.md の「作業ツリーの CRLF は checkout が作る」）。
   */
  const codeExprOf = (source: string, constructor: string): string => {
    const text = source.split("\r\n").join("\n");
    const start = text.indexOf(`pub const fn ${constructor}() -> Self {`);
    if (start < 0) {
      throw new Error(`${constructor} が見つかりません`);
    }
    const rest = text.slice(start + 1);
    const end = rest.indexOf("pub const fn ");
    const block = end < 0 ? rest : rest.slice(0, end);
    const found = /\n\s*code: ([^\n]+)\n/.exec(block);
    if (found?.[1] === undefined) {
      throw new Error(`${constructor} に code の行がありません`);
    }
    const raw = found[1].trim();
    return raw.endsWith(",") ? raw.slice(0, -1) : raw;
  };

  it("`ApiError::internal()` の `code` は `None`", () => {
    expect(codeExprOf(errorSource(), "internal")).toBe("None");
    // **対照: 同じ走査が、`code` を持つ構築子では別の値を読む**
    // （「そもそも探せていないから緑」を潰す / `15b`）。
    expect(codeExprOf(errorSource(), "character_not_found")).toBe(
      `Some("${NOT_FOUND_CODE}")`,
    );
  });

  it("対照: 植えたソースからは、その場の値を読む", () => {
    const planted = [
      "    pub const fn internal() -> Self {",
      "        Self {",
      "            status: StatusCode::INTERNAL_SERVER_ERROR,",
      '            code: Some("planted_internal_code"),',
      "        }",
      "    }",
    ].join("\n");
    expect(codeExprOf(planted, "internal")).toBe(
      'Some("planted_internal_code")',
    );
    expect(codeExprOf(planted, "internal")).not.toBe("None");
  });

  it("対照: 見つからなければ落ちる（探せていないことを緑にしない）", () => {
    expect(() => codeExprOf("", "internal")).toThrow();
    expect(() =>
      codeExprOf(
        "    pub const fn internal() -> Self {\n        Self {}\n",
        "internal",
      ),
    ).toThrow();
  });
});

describe("尺度は一覧から引く", () => {
  it("その識別子の行の尺度を返す（先頭でも末尾でもない）", () => {
    expect(scaleFromListing(LISTING, MIO)).toEqual({
      kind: "known",
      scale: MIO_SCALE,
    });
    // **対照: 隣の行の尺度ではない。**
    expect(scaleFromListing(LISTING, AOI)).toEqual({
      kind: "known",
      scale: AOI_SCALE,
    });
    expect(scaleFromListing(LISTING, NAR)).toEqual({
      kind: "known",
      scale: NAR_SCALE,
    });
    expect(new Set([AOI_SCALE, MIO_SCALE, NAR_SCALE]).size).toBe(3);
  });

  it("キャラクターの行そのものが読めなければ、そう言う", () => {
    expect(scaleFromListing(LISTING, KURO)).toEqual({
      kind: "character-unreadable",
    });
  });

  it("どちらにも居なければ、分からない", () => {
    expect(scaleFromListing(LISTING, "いない")).toEqual({ kind: "unknown" });
  });

  /**
   * **識別子が `null` の行は、決して一致しない。**
   *
   * 一致させると、**API から指せない行**（D-6）を
   * 「キャラクターが読めない」と言い当てたことになってしまう。
   */
  it("識別子が `null` の行は、どの識別子とも一致しない", () => {
    const onlyNull: CharacterListView = {
      characters: [],
      unreadable: [{ id: 5, handle: null, invalid_columns: ["handle"] }],
    };
    expect(scaleFromListing(onlyNull, "null")).toEqual({ kind: "unknown" });
    expect(scaleFromListing(onlyNull, "")).toEqual({ kind: "unknown" });
  });

  /**
   * **想定と違う形は、例外にせず値で返す**（レビューの指摘 / 2026-08-28）。
   *
   * `api.ts` は**応答の形を検証しない**と自ら明記している。
   * **投げると呼び出し側の `catch {}` に飲まれ、道具の壊れが永久に見えなくなる。**
   * **全域関数にして、`malformed` という別の結末にする。**
   */
  it("一覧の形が違えば、例外ではなく `malformed` を返す", () => {
    for (const broken of [
      null,
      undefined,
      42,
      "一覧",
      {},
      { characters: [] },
      { unreadable: [] },
      { characters: {}, unreadable: [] },
      { characters: [], unreadable: "配列ではない" },
    ]) {
      expect(() => scaleFromListing(broken, MIO)).not.toThrow();
      expect(scaleFromListing(broken, MIO)).toEqual({ kind: "malformed" });
    }
  });

  /**
   * **要素の形が違うのも同じ。**
   *
   * **`stat_scale` が数でなければ「読めた」と言わない** ——
   * 言うと、値の欄に `/ undefined` の類が出る。
   */
  it("行はあっても尺度が数でなければ、`malformed`", () => {
    const listing = {
      characters: [{ handle: MIO, stat_scale: "7" }],
      unreadable: [],
    };
    expect(scaleFromListing(listing, MIO)).toEqual({ kind: "malformed" });
    // **対照: 数なら読める**（走査そのものは壊れていない）。
    expect(
      scaleFromListing(
        { characters: [{ handle: MIO, stat_scale: 7 }], unreadable: [] },
        MIO,
      ),
    ).toEqual({ kind: "known", scale: 7 });
  });

  /** **要素が物でなくても落ちない**（`null` の要素が混ざっても走査は続く）。 */
  it("要素が物でなくても、例外にしない", () => {
    const listing = {
      characters: [null, 7, { handle: MIO, stat_scale: MIO_SCALE }],
      unreadable: [undefined],
    };
    expect(scaleFromListing(listing, MIO)).toEqual({
      kind: "known",
      scale: MIO_SCALE,
    });
  });
});

describe("失敗の枝に、何を出すか", () => {
  const failure = (kind: StatsReadFailure["kind"]): StatsReadFailure => ({
    kind,
    message: "サーバの文言",
  });
  const known: ScaleLookup = { kind: "known", scale: MIO_SCALE };

  it("尺度をまだ引いていないうちは、何も出さない", () => {
    expect(repairEntryFor(failure("server"), null).kind).toBe("pending");
  });

  it("サーバが答えた回は、フォームを出す（尺度つき）", () => {
    expect(repairEntryFor(failure("server"), known)).toEqual({
      kind: "form",
      scale: { kind: "known", scale: MIO_SCALE },
    });
    // **想定外の失敗でも塞がない**（`PUT` が通るかどうかは分からない）。
    expect(repairEntryFor(failure("unknown"), known)).toEqual({
      kind: "form",
      scale: { kind: "known", scale: MIO_SCALE },
    });
  });

  it("尺度が分からなくても、フォームは出す（上端を言わないだけ）", () => {
    expect(repairEntryFor(failure("server"), { kind: "unknown" })).toEqual({
      kind: "form",
      scale: { kind: "unreadable", notice: UNKNOWN_SCALE_NOTICE },
    });
  });

  /**
   * **形の食い違いを、握り潰さない**（レビューの指摘 / 2026-08-28）。
   *
   * **同じ「上端が分からない」でも、理由が違えば別の文を出す。**
   * 同じ文にすると、**版の食い違い**（道具の壊れ）が
   * **「サーバが落ちていた」に紛れて永久に見えなくなる。**
   */
  it("一覧の形が違えば、尺度が引けなかった回とは別の文を出す", () => {
    const malformed = repairEntryFor(failure("server"), { kind: "malformed" });
    expect(malformed).toEqual({
      kind: "form",
      scale: { kind: "unreadable", notice: MALFORMED_LISTING_NOTICE },
    });
    // **フォームは出る**（読めない形でも、直すことはできる）。
    expect(malformed.kind).toBe("form");
    // **対照: 引けなかった回とは別の文。**
    expect(MALFORMED_LISTING_NOTICE).not.toBe(UNKNOWN_SCALE_NOTICE);
  });

  it("届いていない回と居ない回は、フォームを出さない", () => {
    // **尺度が引けていても出さない**（押せば必ず失敗するので）。
    expect(repairEntryFor(failure("unreachable"), known)).toMatchObject({
      kind: "blocked",
      reason: "unreachable",
    });
    expect(repairEntryFor(failure("missing"), known)).toMatchObject({
      kind: "blocked",
      reason: "missing",
    });
  });

  it("キャラクターの行そのものが読めない回も、フォームを出さない", () => {
    expect(
      repairEntryFor(failure("server"), { kind: "character-unreadable" }),
    ).toMatchObject({ kind: "blocked", reason: "character-unreadable" });
  });

  it("3つの説明は、互いに別の文である", () => {
    const messages = (
      [
        repairEntryFor(failure("unreachable"), known),
        repairEntryFor(failure("missing"), known),
        repairEntryFor(failure("server"), { kind: "character-unreadable" }),
      ] as { message?: string }[]
    ).map((entry) => entry.message ?? "");
    expect(new Set(messages).size).toBe(3);
  });
});

describe("下書きは、推測で埋めない", () => {
  /**
   * **空の行が1本。**
   *
   * **0 行から始めない** —— 0 行の下書きはそのまま送れてしまい、
   * **何も打たずに押しただけで全軸が消える**（空の配列 ＝ 全消し）。
   */
  it("空の行が1本だけ", () => {
    expect(emptyRepairDraft()).toEqual({
      rows: [{ axis: "", value: "", removed: false, stored: false }],
    });
  });
});

describe("言えないことを、言わない", () => {
  /**
   * **何が消えるかは、押す前に言えない**（D-3c の作法）。
   *
   * 通常のフォームは `replaceSummary` が「外した X は消えます」と名指しで言うが、
   * **この画面ではそれが原理的に書けない**（いまの軸を1本も読めない）。
   */
  it("消えるものについて、断定しない", () => {
    expect(UNKNOWN_LOSS_NOTICE).toContain("言うことができません");
    // **「消えるものはありません」と読める形になっていない。**
    expect(UNKNOWN_LOSS_NOTICE).not.toContain("消えません");
    // **黙りもしない** —— 丸ごと置き換えなので、書かなかった軸は実際に消える。
    expect(UNKNOWN_LOSS_NOTICE).toContain("丸ごと置き換え");
  });

  it("尺度を知らないときも、知らないと言う", () => {
    expect(UNKNOWN_SCALE_NOTICE).toContain("読み取れませんでした");
    expect(UNKNOWN_SCALE_NOTICE).not.toBe(UNKNOWN_LOSS_NOTICE);
  });

  /**
   * **形の食い違いは、別の言い方をする。**
   *
   * 「サーバから引けなかった」と「そもそも読めない形だった」は原因が違う ——
   * 前者はサーバが落ちているだけ、後者は**版の食い違い**である。
   */
  it("形が読めなかったときは、そう名乗る", () => {
    expect(MALFORMED_LISTING_NOTICE).toContain("版が食い違っている");
    expect(MALFORMED_LISTING_NOTICE).not.toBe(UNKNOWN_SCALE_NOTICE);
    expect(MALFORMED_LISTING_NOTICE).not.toBe(UNKNOWN_LOSS_NOTICE);
  });

  /**
   * **3つの断りが名指しする操作は、`StatsPanel` に実在する。**
   *
   * 画面側でも同じ判定を当てるが（`stats-repair-panel.test.ts`）、
   * **文言そのものを見る側にも置く** —— 綴りを変えたときにここが先に落ちる。
   */
  it("断りが名指しする操作は、この画面にあるものだけ", () => {
    // **`StatsPanel` の見出しにある2つ**（`hrefFor` / `labelFor` の外に無い）。
    const onTheScreen = new Set(["再読み込み", "キャラクターの一覧へ"]);
    for (const notice of [UNKNOWN_SCALE_NOTICE, MALFORMED_LISTING_NOTICE]) {
      const named = [...notice.matchAll(/「([^」]+)」/g)].map(
        (found) => found[1] as string,
      );
      expect(named.length, "操作を名指しすること").toBeGreaterThan(0);
      for (const operation of named) {
        expect(onTheScreen.has(operation), `「${operation}」が実在すること`).toBe(
          true,
        );
      }
    }
    // **対照: 判定が何でも通るわけではない。**
    expect(onTheScreen.has("尺度を推測")).toBe(false);
  });
});
