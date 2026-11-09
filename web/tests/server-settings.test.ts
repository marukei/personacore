/**
 * **「設定」の画面が言うこと**（作業単位 F-12b / ADR-0057 §決定5 / `src/lib/serverSettings.ts`）。
 *
 * # ここで見るものと、見ないもの
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **応答の欄から、行の見出しと値を組むこと**（出どころ3種・保存あり／なし／読めない／保存できない起動・次の起動が止まる） | **ここ** |
 * | **保存・消去の後の文**（次の起動から効く・環境変数が勝つ・いまの待ち受けは移らない・使われている番号） | **ここ** |
 * | **失敗の文**（400 の2種・409・500・繋がらない・知らない失敗） | **ここ** |
 * | **入力の読み方**（範囲はサーバが決める。ここは「数字だけか」まで） | **ここ** |
 * | **`code` の綴りが `src/server/error.rs` と一致すること** | **ここ** |
 * | **その文が実際に画面へ描かれること・送る本文** | `tests/settings-panel.test.ts` |
 *
 * **期待値は全部この中に直書きする**（検査対象の定数から作らない / CLAUDE.md「見張り番の書き方」）。
 * **値は欄ごとに全部違う番号にしてある** —— いまの番号と保存した番号と次の番号を取り違えると赤くなる。
 *
 * **jsdom は要らない**（DOM を触らない）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { ApiRequestError, type SavePortView, type ServerView } from "../src/lib/api";
import {
  READ_ONLY_NOTICE,
  SAVE_FAILURE_CODES,
  UNREADABLE_NOTICE,
  canClear,
  describeSaveFailure,
  describeSaved,
  initialDraft,
  parsePortInput,
  portSourceLabel,
  serverRows,
} from "../src/lib/serverSettings";

/** **3つの番号を全部違う値に**（取り違えが観測できるように）。 */
const LISTENING = 8123;
const SAVED = 8456;
const NEXT = 8789;

/** 保存した番号で起き、次の起動も保存した番号を使う、いちばん素直な形。 */
function viewOf(overrides: Partial<ServerView> = {}): ServerView {
  return {
    listening_port: LISTENING,
    listening_port_source: "saved",
    saved_port: SAVED,
    saved_port_unreadable: false,
    next_port: NEXT,
    next_port_source: "saved",
    editable: true,
    ...overrides,
  };
}

function savedViewOf(
  overrides: Partial<ServerView> = {},
  inUse: boolean | null = false,
): SavePortView {
  return { ...viewOf(overrides), saved_port_in_use: inUse };
}

/** 行を `[見出し, 値]` の配列で。 */
function rowsOf(view: ServerView): [string, string][] {
  return serverRows(view).map((row) => [row.label, row.value]);
}

describe("出どころの言い方", () => {
  it("3つの綴りを、それぞれ別の言い方にする", () => {
    expect(portSourceLabel("environment")).toBe("環境変数 PERSONACORE_PORT");
    expect(portSourceLabel("saved")).toBe("保存した番号");
    expect(portSourceLabel("default")).toBe("既定の番号");
  });

  /** **知らない綴りを、知っている言い方に丸めない**（サーバが綴りを足した日に、嘘をつかない）。 */
  it("知らない綴りは、そのまま見せる", () => {
    expect(portSourceLabel("shifted")).toBe("知らない出どころ: shifted");
  });
});

describe("行", () => {
  it("いま・保存・次の起動を、この順に並べる（出どころは保存）", () => {
    expect(rowsOf(viewOf())).toEqual([
      ["いま待ち受けている番号", "8123（保存した番号）"],
      ["保存した番号", "8456"],
      ["次の起動で使う番号", "8789（保存した番号）"],
    ]);
  });

  it("出どころが環境変数", () => {
    expect(
      rowsOf(viewOf({ listening_port_source: "environment", next_port_source: "environment" })),
    ).toEqual([
      ["いま待ち受けている番号", "8123（環境変数 PERSONACORE_PORT）"],
      ["保存した番号", "8456"],
      ["次の起動で使う番号", "8789（環境変数 PERSONACORE_PORT）"],
    ]);
  });

  it("出どころが既定（保存していない）", () => {
    expect(
      rowsOf(
        viewOf({ listening_port_source: "default", saved_port: null, next_port_source: "default" }),
      ),
    ).toEqual([
      ["いま待ち受けている番号", "8123（既定の番号）"],
      ["保存した番号", "保存していません"],
      ["次の起動で使う番号", "8789（既定の番号）"],
    ]);
  });

  /** **いまの出どころと次の出どころは別の欄**（保存した直後は、いまは既定・次は保存）。 */
  it("いまと次の出どころを、別々の欄から読む", () => {
    expect(
      rowsOf(viewOf({ listening_port_source: "default", next_port_source: "saved" })),
    ).toEqual([
      ["いま待ち受けている番号", "8123（既定の番号）"],
      ["保存した番号", "8456"],
      ["次の起動で使う番号", "8789（保存した番号）"],
    ]);
  });

  /** **「保存していない」と「壊れたファイルが在る」を同じ行にしない**（`saved_port_unreadable`）。 */
  it("保存のファイルが読めない", () => {
    expect(
      rowsOf(viewOf({ saved_port: null, saved_port_unreadable: true, next_port_source: "default" })),
    ).toEqual([
      ["いま待ち受けている番号", "8123（保存した番号）"],
      ["保存した番号", "読めません（保存のファイルが壊れています）"],
      ["次の起動で使う番号", "8789（既定の番号）"],
    ]);
  });

  /**
   * **保存できない起動では、保存した番号を読まない**（サーバは `null` を返す）。
   * **「保存していません」と言うと嘘になる** —— ファイルは在るかもしれない。
   */
  it("保存できない起動", () => {
    expect(
      rowsOf(viewOf({ saved_port: null, editable: false, listening_port_source: "default", next_port_source: "default" })),
    ).toEqual([
      ["いま待ち受けている番号", "8123（既定の番号）"],
      ["保存した番号", "この起動では読みません"],
      ["次の起動で使う番号", "8789（既定の番号）"],
    ]);
  });

  it("次の起動が止まる（`PERSONACORE_PORT` を番号として読めない）", () => {
    expect(
      rowsOf(viewOf({ next_port: null, next_port_source: "environment" }))[2],
    ).toEqual([
      "次の起動で使う番号",
      "決まりません（PERSONACORE_PORT を番号として読めないので、次の起動は止まります）",
    ]);
  });
});

describe("断り書き", () => {
  it("保存できない理由を言う", () => {
    expect(READ_ONLY_NOTICE).toBe(
      "この起動では番号を保存できません。環境変数 PERSONACORE_UPSTREAMS で設定の置き場所を変えて起動しているためです" +
        "（scripts/start.ps1 を -Upstream 付きで起動したときも、これに当たります）。この起動は保存した番号を読みません。",
    );
  });

  it("保存のファイルが読めないことと、直し方を言う", () => {
    expect(UNREADABLE_NOTICE).toBe(
      "保存のファイル（listening-port.txt）が読めません。読めない間は、保存した番号を使いません。" +
        "番号を保存し直すとファイルが置き換わり、「保存を消して既定に戻す」を押すとファイルが消えます。",
    );
  });
});

describe("保存・消去の後の文", () => {
  it("保存した: 次の起動から効き、いまの待ち受けは移らない", () => {
    expect(describeSaved(9001, savedViewOf())).toBe(
      "9001 を保存しました。次の起動から効きます。いま待ち受けている 8123 は、この起動のあいだ移りません。",
    );
  });

  it("保存した: 環境変数が効いているので、次の起動では使われない", () => {
    expect(
      describeSaved(9001, savedViewOf({ next_port_source: "environment" })),
    ).toBe(
      "9001 を保存しました。PERSONACORE_PORT が効いているので、保存しても次の起動では使われません。" +
        "いま待ち受けている 8123 は、この起動のあいだ移りません。",
    );
  });

  /** **使われていても保存はしてある**（サーバは止めない / ADR-0057 §決定5）。 */
  it("保存した: その番号をいま他のプログラムが使っている", () => {
    expect(describeSaved(9001, savedViewOf({}, true))).toBe(
      "9001 を保存しました。この番号はいま他のプログラムが使っています（保存はしてあります）。" +
        "次の起動から効きます。いま待ち受けている 8123 は、この起動のあいだ移りません。",
    );
  });

  it("保存した: 使われていなければ、何も添えない", () => {
    expect(describeSaved(9001, savedViewOf({}, false))).not.toContain("他のプログラム");
  });

  it("消した: 次の起動から既定の番号を使う", () => {
    expect(
      describeSaved(
        null,
        savedViewOf({ saved_port: null, next_port_source: "default" }, null),
      ),
    ).toBe(
      "保存を消しました。次の起動から 8789（既定の番号）を使います。" +
        "いま待ち受けている 8123 は、この起動のあいだ移りません。",
    );
  });

  it("消した: 環境変数が効いているので、次の起動もその番号を使う", () => {
    expect(
      describeSaved(
        null,
        savedViewOf({ saved_port: null, next_port_source: "environment" }, null),
      ),
    ).toBe(
      "保存を消しました。PERSONACORE_PORT が効いているので、次の起動もその番号を使います。" +
        "いま待ち受けている 8123 は、この起動のあいだ移りません。",
    );
  });
});

describe("失敗の文", () => {
  /** サーバが返す形（`code` は `error.rs` の綴り。**文面はサーバのものを期待値に使わない**）。 */
  function refused(status: number, code: string | null, message = "サーバの文面"): ApiRequestError {
    return new ApiRequestError(message, status, "invalid_request_error", code);
  }

  it("400 `invalid_listening_port`: 範囲を言う", () => {
    expect(describeSaveFailure(refused(400, "invalid_listening_port"), 80)).toBe(
      "保存しませんでした。番号は 1024〜65535 の範囲で入れてください。",
    );
  });

  it("400 `invalid_body`: 番号として読めなかった", () => {
    expect(describeSaveFailure(refused(400, "invalid_body"), 1e21)).toBe(
      "保存しませんでした。番号として読めませんでした（1024〜65535 の半角の数字で入れてください）。",
    );
  });

  it("409 `listening_port_read_only`: 保存も消去も、この起動ではできない", () => {
    expect(describeSaveFailure(refused(409, "listening_port_read_only"), 9001)).toBe(
      "保存しませんでした。この起動では番号を保存できません" +
        "（環境変数 PERSONACORE_UPSTREAMS で設定の置き場所を変えて起動しています）。",
    );
    expect(describeSaveFailure(refused(409, "listening_port_read_only"), null)).toBe(
      "保存を消しませんでした。この起動では番号を保存できません" +
        "（環境変数 PERSONACORE_UPSTREAMS で設定の置き場所を変えて起動しています）。",
    );
  });

  it("500 `listening_port_write_failed`: 前の番号は残っている", () => {
    expect(describeSaveFailure(refused(500, "listening_port_write_failed"), 9001)).toBe(
      "保存のファイルを書き換えられませんでした。前に保存した番号はそのまま残っています。",
    );
    expect(describeSaveFailure(refused(500, "listening_port_write_failed"), null)).toBe(
      "保存のファイルを消せませんでした。前に保存した番号はそのまま残っています。",
    );
  });

  /** **繋がらなかったときは、保存できたかどうかが分からない。** そう言う。 */
  it("繋がらない: 保存できたかどうかは開き直すと分かる", () => {
    const unreachable = new ApiRequestError(
      "サーバへ接続できませんでした（personacore は動いていますか）",
      0,
      null,
      null,
    );
    expect(describeSaveFailure(unreachable, 9001)).toBe(
      "サーバへ接続できませんでした（personacore は動いていますか）。保存できたかどうかは、この画面を開き直すと分かります。",
    );
  });

  it("知らない失敗は、サーバの文面をそのまま添える", () => {
    expect(describeSaveFailure(refused(503, null, "混んでいます"), 9001)).toBe(
      "保存しませんでした: 混んでいます",
    );
    expect(describeSaveFailure(refused(503, null, "混んでいます"), null)).toBe(
      "保存を消しませんでした: 混んでいます",
    );
    expect(describeSaveFailure(new Error("x"), 9001)).toBe("予期しない失敗です");
  });
});

describe("入力", () => {
  it("前後の空白を落として、数字だけなら番号にする", () => {
    expect(parsePortInput(" 9001 ")).toEqual({ ok: true, port: 9001 });
  });

  /** **範囲はサーバが決める**（400 `invalid_listening_port`）。ここでは断らない。 */
  it("範囲の外でも、数字だけならそのまま送る", () => {
    expect(parsePortInput("70000")).toEqual({ ok: true, port: 70000 });
    expect(parsePortInput("80")).toEqual({ ok: true, port: 80 });
  });

  it("空なら、番号を入れるよう言う", () => {
    expect(parsePortInput("  ")).toEqual({ ok: false, message: "番号を入れてください。" });
  });

  it("数字以外が混ざっていれば、半角の数字だけで入れるよう言う", () => {
    for (const text of ["90a1", "-1", "90.5", "９００１", "+9001"]) {
      expect(parsePortInput(text), text).toEqual({
        ok: false,
        message: "番号は半角の数字だけで入れてください。",
      });
    }
  });

  it("入力欄の初めの値は、保存した番号（無ければ空）", () => {
    expect(initialDraft(viewOf())).toBe("8456");
    expect(initialDraft(viewOf({ saved_port: null }))).toBe("");
  });
});

describe("消せるか", () => {
  it("保存しているか、壊れたファイルが在るときだけ消せる", () => {
    expect(canClear(viewOf())).toBe(true);
    expect(canClear(viewOf({ saved_port: null, saved_port_unreadable: true }))).toBe(true);
    expect(canClear(viewOf({ saved_port: null }))).toBe(false);
    expect(canClear(viewOf({ editable: false }))).toBe(false);
  });
});

/**
 * **`code` の綴りは `src/server/error.rs` の写しである。**
 *
 * 1文字ずれると、上の読み分けのどの枝にも入らず「知らない失敗」に落ちる（型検査は捕まえない）。
 */
describe("`code` の綴り", () => {
  const ERROR_RS = readFileSync(
    fileURLToPath(new URL("../../src/server/error.rs", import.meta.url)),
    "utf8",
  );

  /** **画面が読み分けに使う綴り**（`SAVE_FAILURE_CODES`）を、構築子ごとに写し元と突き合わせる。 */
  it.each([
    ["invalid_listening_port", SAVE_FAILURE_CODES.outOfRange],
    ["listening_port_read_only", SAVE_FAILURE_CODES.readOnly],
    ["listening_port_write_failed", SAVE_FAILURE_CODES.writeFailed],
    ["invalid_body", SAVE_FAILURE_CODES.unreadableBody],
  ])("`%s` の `code` が、画面の綴り `%s` と一致する", (constructor, code) => {
    expect(codeOf(ERROR_RS, constructor)).toBe(code);
  });

  /** **4つは全部違う綴り**（2つを同じにすると、その2つの読み分けが消える）。 */
  it("画面の綴りは4つとも違う", () => {
    expect(new Set(Object.values(SAVE_FAILURE_CODES)).size).toBe(4);
  });

  it("対照: 無い構築子は見つからないと言う", () => {
    expect(() => codeOf(ERROR_RS, "no_such_constructor")).toThrow();
  });
});

/**
 * `error.rs` のその構築子から `code` を1つ取り出す（`tests/character-delete.test.ts` と同じ読み方）。
 *
 * **次の `pub const fn` の手前で切る**（切らないと、次の構築子の値を返す）。
 */
function codeOf(source: string, constructor: string): string {
  const start = source.indexOf(`pub const fn ${constructor}() -> Self {`);
  if (start < 0) {
    throw new Error(`${constructor} が見つかりません`);
  }
  const rest = source.slice(start + 1);
  const end = rest.indexOf("pub const fn ");
  const block = end < 0 ? rest : rest.slice(0, end);
  const found = /code: Some\("([^"]+)"\)/.exec(block);
  if (found?.[1] === undefined) {
    throw new Error(`${constructor} に code がありません`);
  }
  return found[1];
}
