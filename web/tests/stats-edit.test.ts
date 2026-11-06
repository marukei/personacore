/**
 * **性格ステータスの置き換えを組み立てる**（作業単位 D-4 / `src/lib/statsEdit.ts`）。
 *
 * # なぜ純粋関数で見るのか
 *
 * **`PUT .../stats` は丸ごと置き換えである**（`admin.rs` の `replace_stats` の doc）。
 * **要求に含まれない軸は消える。** つまり「何を送るか」ではなく
 * **「何を送らなかったか」が破壊的な操作そのもの**で、
 * 画面越しには観測しづらい（`characterEdit.ts` と同じ分け方）。
 *
 * # 画面側との分担
 *
 * | 見るもの | どこ |
 * |---|---|
 * | **送る本文・外した軸・案内の文言** | **ここ** |
 * | 変容の表示・総数・帰属・失敗の見え方 | `stats-panel.test.ts` |
 * | 一覧からこの画面への導線 | `character-list-stats.test.ts` |
 * | 経路が識別子を運ぶこと | `router-stats.test.ts` / `app-routing-stats.test.ts` |
 *
 * **I/O を持たないので jsdom は要らない**（既定の `node` で走る）。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  ApiRequestError,
  type DriftView,
  type StatsView,
} from "../src/lib/api";
import {
  DOWN_DELTA,
  DUPLICATE_AXIS_CODE,
  NOT_FOUND_CODE,
  NO_CHARGE_TIME_LABEL,
  RUN_MODE_OFF,
  RUN_MODE_ON,
  RUN_MODE_SHADOW,
  RUN_MODE_SOURCE_CHARACTER,
  RUN_MODE_SOURCE_DEFAULT,
  SCALE_CHANGED_CODE,
  UNALLOCATED_LABEL,
  UP_DELTA,
  VALUE_ABOVE_SCALE_CODE,
  axisValueLabel,
  buildReplace,
  chargeTimeLabel,
  deltaLabel,
  draftOfStats,
  emptyRow,
  describeReplaceFailure,
  removedAxes,
  replaceSummary,
  runModeLabel,
  runModeNotice,
  runModeSourceNote,
  noAxesNotice,
  unallocatedAxes,
  unallocatedNotice,
  type StatsDraft,
} from "../src/lib/statsEdit";

/**
 * **1つのキャラクターの、いまの値。**
 *
 * **⚠ 値・尺度・軸の本数を全部違う数にしてある。**
 * どれか2つが同値だと、その取り違えは観測できない（CLAUDE.md の最頻の穴）。
 *
 * **`勇気` は値 0。** `get_stats` は 0 の軸も返す（落とすのは `prompt` 層の責務）ので、
 * **画面も下書きも落としてはならない。**
 */
const MIO_STATS: StatsView = {
  character: "mio",
  stat_scale: 7,
  stats: [
    { axis: "勇気", value: 0 },
    { axis: "慎重さ", value: 2 },
    { axis: "陽気さ", value: 5 },
  ],
};

/** 下書きを作って、1行だけ書き換える。 */
function draftWith(change: (draft: StatsDraft) => void): StatsDraft {
  const draft = draftOfStats(MIO_STATS);
  change(draft);
  return draft;
}

/** 送られる軸名の一覧（順序どおり）。 */
function axesOf(draft: StatsDraft): string[] {
  const built = buildReplace(draft);
  expect(built.ok, "組み立てが通ること").toBe(true);
  if (!built.ok) {
    throw new Error(built.message);
  }
  return built.body.stats.map((stat) => stat.axis);
}

describe("いまの値を下書きに写す", () => {
  it("軸・値・並びをそのまま持つ", () => {
    const draft = draftOfStats(MIO_STATS);
    expect(draft.rows.map((row) => row.axis)).toEqual([
      "勇気",
      "慎重さ",
      "陽気さ",
    ]);
    expect(draft.rows.map((row) => row.value)).toEqual(["0", "2", "5"]);
    expect(draft.rows.every((row) => row.stored)).toBe(true);
    expect(draft.rows.some((row) => row.removed)).toBe(false);
  });

  it("値 0 の軸を落とさない（`get_stats` は 0 も返す）", () => {
    const draft = draftOfStats(MIO_STATS);
    const zero = draft.rows.find((row) => row.axis === "勇気");
    expect(zero, "値 0 の軸が下書きにあること").toBeDefined();
    expect((zero as { value: string }).value).toBe("0");
  });
});

describe("送る本文は、丸ごと置き換え", () => {
  /**
   * **⚠ `characterEdit.ts` の `buildUpdate`（差分だけ送る）とは逆の作法。**
   *
   * 差分だけ送ると、**送らなかった軸が消える。**
   * `characterRepair.ts` の「全部の欄を必ず送る」と同じ立場。
   */
  it("1つも変えていなくても、全軸を送る", () => {
    expect(axesOf(draftOfStats(MIO_STATS))).toEqual([
      "勇気",
      "慎重さ",
      "陽気さ",
    ]);
  });

  it("1軸だけ下げても、触っていない軸まで送る", () => {
    const draft = draftWith((d) => {
      const row = d.rows.find((r) => r.axis === "慎重さ");
      if (row === undefined) {
        throw new Error("慎重さ が無い");
      }
      row.value = "1";
    });
    const built = buildReplace(draft);
    expect(built.ok).toBe(true);
    if (!built.ok) {
      throw new Error(built.message);
    }
    expect(built.body.stats).toEqual([
      { axis: "勇気", value: 0 },
      { axis: "慎重さ", value: 1 },
      { axis: "陽気さ", value: 5 },
    ]);
  });

  it("値 0 の軸も送る（送らなければ、その軸は消える）", () => {
    const built = buildReplace(draftOfStats(MIO_STATS));
    expect(built.ok).toBe(true);
    if (!built.ok) {
      throw new Error(built.message);
    }
    expect(built.body.stats).toContainEqual({ axis: "勇気", value: 0 });
  });

  /**
   * **尺度の上端をこちらで判定しない**（`characterEdit.ts` が
   * `StatScale::MIN` / `MAX` を写していないのと同じ立場）。
   * 超えていればサーバが `stat_value_above_scale` を返し、その理由が画面に出る。
   */
  it("尺度を超える値も、そのまま送る（判定はサーバ）", () => {
    const draft = draftWith((d) => {
      const row = d.rows[2];
      if (row === undefined) {
        throw new Error("行が無い");
      }
      row.value = String(MIO_STATS.stat_scale + 2);
    });
    const built = buildReplace(draft);
    expect(built.ok).toBe(true);
    if (!built.ok) {
      throw new Error(built.message);
    }
    expect(built.body.stats[2]).toEqual({ axis: "陽気さ", value: 9 });
  });
});

describe("軸を消すのは「送らない」でしか表現できない", () => {
  it("外した軸は本文から消える", () => {
    const draft = draftWith((d) => {
      const row = d.rows.find((r) => r.axis === "慎重さ");
      if (row === undefined) {
        throw new Error("慎重さ が無い");
      }
      row.removed = true;
    });
    expect(axesOf(draft)).toEqual(["勇気", "陽気さ"]);
    expect(removedAxes(draft)).toEqual(["慎重さ"]);
  });

  it("全部外すと空の配列（＝全軸が消える）", () => {
    const draft = draftWith((d) => {
      for (const row of d.rows) {
        row.removed = true;
      }
    });
    const built = buildReplace(draft);
    expect(built.ok).toBe(true);
    if (!built.ok) {
      throw new Error(built.message);
    }
    expect(built.body.stats).toEqual([]);
    expect(removedAxes(draft)).toEqual(["勇気", "慎重さ", "陽気さ"]);
  });

  it("足した行を外しても、「消える軸」には数えない（元から無い）", () => {
    const draft = draftWith((d) => {
      d.rows.push({ ...emptyRow(), axis: "度胸", value: "3", removed: true });
    });
    expect(removedAxes(draft)).toEqual([]);
    expect(axesOf(draft)).toEqual(["勇気", "慎重さ", "陽気さ"]);
  });

  it("足した行は本文に載る", () => {
    const draft = draftWith((d) => {
      d.rows.push({ ...emptyRow(), axis: "度胸", value: "3" });
    });
    const built = buildReplace(draft);
    expect(built.ok).toBe(true);
    if (!built.ok) {
      throw new Error(built.message);
    }
    expect(built.body.stats).toContainEqual({ axis: "度胸", value: 3 });
    expect(built.body.stats).toHaveLength(4);
  });
});

describe("保存すると何が起きるか、送る前に言う", () => {
  it("残る軸と、消える軸を、両方名乗る", () => {
    const draft = draftWith((d) => {
      const row = d.rows.find((r) => r.axis === "慎重さ");
      if (row === undefined) {
        throw new Error("慎重さ が無い");
      }
      row.removed = true;
    });
    const summary = replaceSummary(draft);
    expect(summary).toContain("勇気");
    expect(summary).toContain("陽気さ");
    expect(summary).toContain("慎重さ");
    // **消えることを言っている**（「置き換え」の意味が読み取れる）。
    expect(summary).toContain("消え");
  });

  it("外していないときは、消える軸を名乗らない", () => {
    const summary = replaceSummary(draftOfStats(MIO_STATS));
    expect(summary).toContain("勇気");
    expect(summary).toContain("慎重さ");
    expect(summary).toContain("陽気さ");
    expect(summary).not.toContain("消え");
  });

  it("外すと文面が変わる（同じことを言い続けない）", () => {
    const before = replaceSummary(draftOfStats(MIO_STATS));
    const after = replaceSummary(
      draftWith((d) => {
        const row = d.rows[0];
        if (row === undefined) {
          throw new Error("行が無い");
        }
        row.removed = true;
      }),
    );
    expect(after).not.toBe(before);
  });

  it("全部外したときは、1軸も残らないと言う", () => {
    const summary = replaceSummary(
      draftWith((d) => {
        for (const row of d.rows) {
          row.removed = true;
        }
      }),
    );
    expect(summary).toContain("1軸も残りません");
    expect(summary).toContain("勇気");
  });
});

describe("送れない下書きは、送らない", () => {
  for (const bad of ["", " ", "-1", "2.5", "１", "0x1", "３", "one", "1 2"]) {
    it(`値が「${bad}」なら送らない`, () => {
      const draft = draftWith((d) => {
        const row = d.rows.find((r) => r.axis === "陽気さ");
        if (row === undefined) {
          throw new Error("陽気さ が無い");
        }
        row.value = bad;
      });
      const built = buildReplace(draft);
      expect(built.ok).toBe(false);
      if (built.ok) {
        throw new Error("送れてしまった");
      }
      // **どの軸の話かを名乗る**（外した軸の話と取り違えない）。
      expect(built.message).toContain("陽気さ");
      expect(built.message).not.toContain("勇気");
    });
  }

  it("外した行の値が壊れていても、送るのは止めない（送らない行だから）", () => {
    const draft = draftWith((d) => {
      const row = d.rows.find((r) => r.axis === "陽気さ");
      if (row === undefined) {
        throw new Error("陽気さ が無い");
      }
      row.value = "こわれている";
      row.removed = true;
    });
    expect(axesOf(draft)).toEqual(["勇気", "慎重さ"]);
  });

  it("軸名が空の行があれば送らない", () => {
    const draft = draftWith((d) => {
      d.rows.push({ ...emptyRow(), axis: "   ", value: "1" });
    });
    const built = buildReplace(draft);
    expect(built.ok).toBe(false);
    if (built.ok) {
      throw new Error("送れてしまった");
    }
    expect(built.message).toContain("軸名");
  });

  it("値 0 は正当な値（0 を「空」と混同しない）", () => {
    const draft = draftWith((d) => {
      const row = d.rows.find((r) => r.axis === "陽気さ");
      if (row === undefined) {
        throw new Error("陽気さ が無い");
      }
      row.value = "0";
    });
    const built = buildReplace(draft);
    expect(built.ok).toBe(true);
    if (!built.ok) {
      throw new Error(built.message);
    }
    expect(built.body.stats).toContainEqual({ axis: "陽気さ", value: 0 });
  });
});

/**
 * **向きの綴り**（`admin.rs` の `delta_to_wire`）。
 *
 * **知らない綴りは訳さずそのまま出す** —— サーバが向きを増やした日に
 * **画面から向きが消える**のがいちばん質の悪い壊れ方
 * （`characterRepair.ts` の `columnLabel` と同じ立場）。
 */
describe("変容の向き", () => {
  it("上げと下げは別の見出し", () => {
    expect(deltaLabel(UP_DELTA)).not.toBe(deltaLabel(DOWN_DELTA));
    expect(deltaLabel(UP_DELTA)).toContain("上げ");
    expect(deltaLabel(DOWN_DELTA)).toContain("下げ");
  });

  it("知らない綴りは、そのまま返す", () => {
    expect(deltaLabel("sideways")).toBe("sideways");
  });
});

/**
 * **置き換えが拒否されたときの読み分け**。
 *
 * **⚠ `message` をフィクスチャから期待値に使わない**
 * （`character-delete.test.ts` の実測 —— 素通しと決め打ちが同値になる）。
 * 素通しは「文言が改訂された想定」の腕が見る。
 */
describe("置き換えの失敗を読み分ける", () => {
  const ABOVE_SCALE = new ApiRequestError(
    "性格ステータスの値が尺度を超えています",
    400,
    "invalid_request_error",
    "stat_value_above_scale",
  );
  const SCALE_CHANGED = new ApiRequestError(
    "性格ステータスの尺度が変更されました。送り直してください",
    409,
    "invalid_request_error",
    "stat_scale_changed",
  );
  const DUPLICATE = new ApiRequestError(
    "同じ軸を重複して指定することはできません",
    400,
    "invalid_request_error",
    "duplicate_stat_axis",
  );
  const NOT_FOUND = new ApiRequestError(
    "指定されたキャラクターが存在しません",
    404,
    "invalid_request_error",
    "character_not_found",
  );

  it("値が尺度を超えた: 種別と、次の一手が付く", () => {
    const failure = describeReplaceFailure(ABOVE_SCALE);
    expect(failure.reason).toBe("value-above-scale");
    expect(failure.message).toBe(ABOVE_SCALE.message);
    expect(failure.guidance).not.toBeNull();
  });

  it("尺度が変わった: 送り直せると言う（諦めさせない）", () => {
    const failure = describeReplaceFailure(SCALE_CHANGED);
    expect(failure.reason).toBe("scale-changed");
    expect(failure.guidance as string).toContain("再読み込み");
  });

  it("軸の重複: どの操作で直すかを言う", () => {
    const failure = describeReplaceFailure(DUPLICATE);
    expect(failure.reason).toBe("duplicate-axis");
    expect(failure.guidance as string).toContain("外す");
  });

  it("居ない: 取り直しを勧める", () => {
    const failure = describeReplaceFailure(NOT_FOUND);
    expect(failure.reason).toBe("not-found");
    expect(failure.guidance as string).toContain("一覧");
  });

  it("4つの種別は、互いに別の案内", () => {
    const guidances = [ABOVE_SCALE, SCALE_CHANGED, DUPLICATE, NOT_FOUND].map(
      (failure) => describeReplaceFailure(failure).guidance,
    );
    expect(new Set(guidances).size).toBe(4);
    const reasons = [ABOVE_SCALE, SCALE_CHANGED, DUPLICATE, NOT_FOUND].map(
      (failure) => describeReplaceFailure(failure).reason,
    );
    expect(new Set(reasons).size).toBe(4);
  });

  it("`code` の無い 400 を、尺度超え扱いにしない", () => {
    const failure = describeReplaceFailure(
      new ApiRequestError("サーバが 400 を返しました", 400, null, null),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
  });

  it("`code` の無い 409 を、尺度が変わった扱いにしない", () => {
    const failure = describeReplaceFailure(
      new ApiRequestError("サーバが 409 を返しました", 409, null, null),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
  });

  it("知らない `code` は、そのまま「分からない」", () => {
    const failure = describeReplaceFailure(
      new ApiRequestError("何か", 500, "api_error", "stats_are_haunted"),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
    expect(failure.message).toBe("何か");
  });

  it("`ApiRequestError` ですらない失敗は、中身を画面へ流さない", () => {
    const failure = describeReplaceFailure(
      new Error("C:\\Users\\someone\\Rust\\personacore\\web で落ちた"),
    );
    expect(failure.reason).toBe("unknown");
    expect(failure.guidance).toBeNull();
    expect(failure.message).toBe("予期しない失敗です");
    expect(failure.message).not.toContain("Rust");
  });

  /**
   * **文言は素通しする**（`character-delete.test.ts` と同じ理由 ——
   * `code` を保ったまま `message` だけが変わるのは、このリポジトリで起きたこと）。
   */
  it("文言が改訂されても、そのまま出す", () => {
    const reworded = new ApiRequestError(
      "その値はこの個体の目盛りを超えています（文言が改訂された想定）",
      400,
      "invalid_request_error",
      VALUE_ABOVE_SCALE_CODE,
    );
    const failure = describeReplaceFailure(reworded);
    expect(failure.reason).toBe("value-above-scale");
    expect(failure.message).toBe(reworded.message);
    expect(failure.message).not.toBe(ABOVE_SCALE.message);
    expect(failure.message).not.toContain("性格ステータス");
  });

  it("枝どうしが、互いの文言を出していない", () => {
    expect(describeReplaceFailure(ABOVE_SCALE).message).not.toBe(
      describeReplaceFailure(SCALE_CHANGED).message,
    );
  });
});

/**
 * **`code` の綴りは `src/server/error.rs` の写しである。**
 *
 * # なぜ見張るのか
 *
 * **型ではなく値の写し。** 綴りが1文字ずれると**どの枝にも入らず
 * `unknown` に落ち、案内が黙って消える** ——
 * **画面は何も言わなくなるのに、型検査もテストも緑のまま。**
 *
 * **先例は `character-edit.test.ts`（`MAX_CHARS`）と
 * `character-delete.test.ts`（`code`）と `character-repair.test.ts`（列名）。**
 */
describe("`code` は写し元と一致する", () => {
  const errorSource = (): string =>
    readFileSync(
      fileURLToPath(new URL("../../src/server/error.rs", import.meta.url)),
      "utf8",
    );

  it("`src/server/error.rs` の `code` と一致する", () => {
    const source = errorSource();
    expect(codeOf(source, "stat_scale_changed")).toBe(SCALE_CHANGED_CODE);
    expect(codeOf(source, "duplicate_stat_axis")).toBe(DUPLICATE_AXIS_CODE);
    expect(codeOf(source, "character_not_found")).toBe(NOT_FOUND_CODE);
    // **こちらは理由ごとに分岐する構築子**（`code: Some(code)` なので綴りは match の中）。
    expect(codeInArm(source, "invalid_stat_value", "ValueError::AboveScale")).toBe(
      VALUE_ABOVE_SCALE_CODE,
    );
    // **対照: 4つは互いに別物**（同じ値を4回読んで緑、という形にしない）。
    expect(
      new Set([
        SCALE_CHANGED_CODE,
        DUPLICATE_AXIS_CODE,
        NOT_FOUND_CODE,
        VALUE_ABOVE_SCALE_CODE,
      ]).size,
    ).toBe(4);
  });

  /**
   * **走査する側が壊れていないこと**（`15b` の「そもそも探せていないから緑」）。
   *
   * **⚠ 実ファイルだけを見る対照は自己成就する** ——
   * だから、**植えたソース**を同じ関数へ通す。
   */
  it("対照: 植えたソースからは、その場の値を読む", () => {
    const planted = [
      "    pub const fn stat_scale_changed() -> Self {",
      "        Self {",
      "            status: StatusCode::CONFLICT,",
      '            message: "植えた文言",',
      '            kind: "invalid_request_error",',
      '            code: Some("planted_code_for_the_control"),',
      "        }",
      "    }",
    ].join("\n");
    expect(codeOf(planted, "stat_scale_changed")).toBe(
      "planted_code_for_the_control",
    );
    expect(codeOf(planted, "stat_scale_changed")).not.toBe(SCALE_CHANGED_CODE);
  });

  it("対照: 分岐する構築子でも、植えた値を読む", () => {
    const planted = [
      "    pub const fn invalid_stat_value(reason: ValueError) -> Self {",
      "        let (message, code) = match reason {",
      "            ValueError::AboveScale => (",
      '                "植えた文言",',
      '                "planted_arm_for_the_control",',
      "            ),",
      "        };",
      "    }",
    ].join("\n");
    expect(
      codeInArm(planted, "invalid_stat_value", "ValueError::AboveScale"),
    ).toBe("planted_arm_for_the_control");
  });

  it("対照: 見つからなければ落ちる（探せていないことを緑にしない）", () => {
    expect(() => codeOf("", "stat_scale_changed")).toThrow();
    expect(() => codeInArm("", "invalid_stat_value", "ValueError::AboveScale")).toThrow();

    // **次の構築子の値を拾ってこない**（`pub const fn` の手前で切る）。
    const planted = [
      "    pub const fn stat_scale_changed() -> Self {",
      "        Self {",
      "            status: StatusCode::CONFLICT,",
      "        }",
      "    }",
      "",
      "    pub const fn duplicate_stat_axis() -> Self {",
      "        Self {",
      '            code: Some("duplicate_stat_axis"),',
      "        }",
      "    }",
    ].join("\n");
    expect(() => codeOf(planted, "stat_scale_changed")).toThrow();
  });
});

/**
 * **向きの綴りは `src/server/admin.rs` の `delta_to_wire` の写しである。**
 *
 * `delta_to_wire` の doc は**「API の語彙はログと DB の綴りとは別物で、
 * 揃えてあるだけ」**と明言している。**こちらはその API の語彙を写している**ので、
 * 変わったときに気付けないと、**画面から向きが消える**（訳せない綴りは生で出るが、
 * 「上げ」「下げ」の見出しが付かなくなる）。
 */
describe("向きの綴りは写し元と一致する", () => {
  const adminSource = (): string =>
    readFileSync(
      fileURLToPath(new URL("../../src/server/admin.rs", import.meta.url)),
      "utf8",
    );

  it("`delta_to_wire` と一致する", () => {
    const source = adminSource();
    expect(wireDeltaOf(source, "Up")).toBe(UP_DELTA);
    expect(wireDeltaOf(source, "Down")).toBe(DOWN_DELTA);
    // **対照: 2つは別物。**
    expect(UP_DELTA).not.toBe(DOWN_DELTA);
  });

  it("対照: 植えたソースからは、その場の値を読む（向きの入れ替わりも見る）", () => {
    const planted = [
      "const fn delta_to_wire(delta: DriftDelta) -> &'static str {",
      "    match delta {",
      '        DriftDelta::Up => "planted_up",',
      '        DriftDelta::Down => "planted_down",',
      "    }",
      "}",
    ].join("\n");
    expect(wireDeltaOf(planted, "Up")).toBe("planted_up");
    expect(wireDeltaOf(planted, "Down")).toBe("planted_down");
  });

  it("対照: 見つからなければ落ちる", () => {
    expect(() => wireDeltaOf("", "Up")).toThrow();
  });
});

/**
 * `error.rs` のその構築子から `code` を1つ取り出す。
 *
 * **次の `pub const fn` の手前で切る。** 切らないと、`code` を持たない構築子を
 * 尋ねたときに**次の構築子の値を返す**（探せていないのに緑になる）。
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

/**
 * **理由ごとに分岐する構築子**から、その腕の `code` を取り出す。
 *
 * `code: Some(code)` なので `codeOf` では読めない ——
 * 綴りは `match` の腕にある。**腕の中の、英小文字と `_` だけの文字列**が
 * `code` である（もう一方は日本語の文言）。
 */
function codeInArm(source: string, constructor: string, arm: string): string {
  const start = source.indexOf(`pub const fn ${constructor}(`);
  if (start < 0) {
    throw new Error(`${constructor} が見つかりません`);
  }
  const rest = source.slice(start + 1);
  const end = rest.indexOf("pub const fn ");
  const block = end < 0 ? rest : rest.slice(0, end);
  const armAt = block.indexOf(`${arm} =>`);
  if (armAt < 0) {
    throw new Error(`${constructor} に ${arm} がありません`);
  }
  const armBlock = block.slice(armAt, block.indexOf("),", armAt) + 1);
  const found = /"([a-z0-9_]+)"/.exec(armBlock);
  if (found?.[1] === undefined) {
    throw new Error(`${arm} に code がありません`);
  }
  return found[1];
}

/** `admin.rs` の `delta_to_wire` から、その向きの綴りを取り出す。 */
function wireDeltaOf(source: string, variant: "Up" | "Down"): string {
  const start = source.indexOf("fn delta_to_wire(");
  if (start < 0) {
    throw new Error("delta_to_wire が見つかりません");
  }
  const rest = source.slice(start);
  const end = rest.indexOf("\n}");
  const block = end < 0 ? rest : rest.slice(0, end);
  const found = new RegExp(`DriftDelta::${variant} => "([^"]+)"`).exec(block);
  if (found?.[1] === undefined) {
    throw new Error(`delta_to_wire に ${variant} がありません`);
  }
  return found[1];
}

// =========================================================================
// 割り当てを失った軸（作業単位 B-16）
//
// **`PUT .../stats` は丸ごと置き換えなので、軸を改名すると綴りが変わる。**
// サーバ側の累積と票は軸の割り当てを参照していないので消えず、
// **総数には数えられ続ける。** サーバはその軸も返す（`value` は `null`）。
//
// **画面が `?? 0` で埋めた瞬間、値 0 の軸と同じ行になる** —— それを塞ぐ。
// =========================================================================

describe("割り当ての無い軸の見出し", () => {
  /**
   * **値がある軸は、いままでどおり「値 / 尺度」。**
   *
   * この形は `stats-panel.test.ts` の `cellsOf` が
   * `["勇気", "0 / 7", ...]` として突き合わせている。
   */
  it("値がある軸は、値と尺度を出す", () => {
    expect(axisValueLabel(0, 7)).toBe("0 / 7");
    expect(axisValueLabel(6, 7)).toBe("6 / 7");
  });

  /**
   * **割り当ての無い軸は、数を出さない。**
   *
   * # ⚠ `0` にも `null` にもしない
   *
   * `0` にすると**値 0 の軸と同じ行**になる（`api.ts` の `value` の doc）。
   * `null` をそのまま埋め込むと **`null / 7`** と出る
   * ——**読み手には壊れているようにしか見えず、直し方も伝わらない。**
   */
  it("割り当ての無い軸は、数ではなく「割り当てなし」と出す", () => {
    expect(axisValueLabel(null, 7)).toBe(UNALLOCATED_LABEL);
    // **数を1文字も含まない**（尺度が漏れると「なし / 7」のような形になる）。
    expect(axisValueLabel(null, 7)).not.toContain("7");
    expect(axisValueLabel(null, 7)).not.toContain("0");
  });

  /**
   * **値 0 の軸と、割り当ての無い軸は、同じ見出しにならない。**
   *
   * サーバ側の `a_zero_valued_axis_is_never_confused_with_an_axis_that_has_no_allocation`
   * の**画面版の要**。2つを別々に確かめるだけでは、
   * **両方を同じ文字列にする実装**が片方ずつ緑になる。
   */
  it("値 0 と割り当てなしが、同じ見出しにならない", () => {
    expect(axisValueLabel(null, 7)).not.toBe(axisValueLabel(0, 7));
  });
});

describe("割り当ての無い軸の案内", () => {
  /** 割り当ての無い軸を2本持つ変容の応答。 */
  const WITH_LOST: DriftView = {
    character: "mio",
    stat_scale: 7,
    axes: [
      // **値 0 の軸。** 割り当てはある（＝案内の対象ではない）。
      {
        axis: "勇気",
        value: 0,
        charge: 1.75,
        charge_updated_at: "2026-08-26T01:02:03.400Z",
        ups: 3,
        downs: 1,
      },
      {
        axis: "きえた軸",
        value: null,
        charge: 1.5,
        charge_updated_at: "2025-12-31T23:59:59.999Z",
        ups: 0,
        downs: 0,
      },
      // **累積の行が無い軸**（票だけが残っている）。
      { axis: "むかしの軸", value: null, charge: 0, charge_updated_at: null, ups: 0, downs: 3 },
    ],
    total_votes: 7,
    recent_limit: 50,
    recent_votes: [],
    run_mode: RUN_MODE_ON,
    run_mode_source: "default",
    default_run_mode: RUN_MODE_ON,
  };

  /** 割り当ての無い軸が1本も無い応答。 */
  const WITHOUT_LOST: DriftView = {
    ...WITH_LOST,
    axes: [
      {
        axis: "勇気",
        value: 0,
        charge: 1.75,
        charge_updated_at: "2026-08-26T01:02:03.400Z",
        ups: 3,
        downs: 1,
      },
    ],
  };

  /**
   * **拾うのは `value` が `null` の軸だけ。**
   *
   * **値 0 の軸を拾ってはいけない** —— 拾うと、
   * 人が「この軸は語らない」と宣言しただけの軸に「消えている」と言うことになる。
   */
  it("割り当ての無い軸だけを、応答の順のまま拾う", () => {
    expect(unallocatedAxes(WITH_LOST)).toEqual(["きえた軸", "むかしの軸"]);
    expect(unallocatedAxes(WITHOUT_LOST)).toEqual([]);
  });

  /**
   * **1本も無ければ案内を出さない。**
   *
   * 出すと、**何も起きていない画面に「消えた軸があります」と書く**ことになる。
   */
  it("割り当ての無い軸が無ければ、案内は出ない", () => {
    expect(unallocatedNotice(WITHOUT_LOST, true)).toBeNull();
    expect(
      unallocatedNotice({ ...WITH_LOST, axes: [] }, true),
      "軸が1本も無い応答でも出ないこと",
    ).toBeNull();
  });

  /**
   * **置き換えができない状態では、案内を出さない**（レビューの実測 / 2026-08-28）。
   *
   * # ここで初めて観測できること
   *
   * 案内は **「軸を追加」** と **「この内容で置き換える」** を名指しするが、
   * **その2つは置き換えのフォームの中にしか無い。**
   * `GET .../stats` が 500 で `GET .../drift` が 200 の回は、
   * **フォームが描かれないので、名指しした操作が画面に1つも無い。**
   *
   * **`D-7` / `B-15` で潰した「無い操作を案内する」が、失敗経路で復活していた。**
   * 画面側の権威は `tests/stats-panel.test.ts` の
   * `いまの値が読めなくても、変容の側は出る（逆も同じ）`
   * （`assertNoticeNamesOnlyRealOperations` を通す）。
   *
   * **⚠ 「捨て軸がある」ことは、この状態でも表に出ている。**
   * 消えるのは手順の案内だけ。
   */
  it("置き換えができない状態では、案内を出さない", () => {
    expect(unallocatedNotice(WITH_LOST, false)).toBeNull();
    // **対照: 同じ応答でも、置き換えができるなら出る**
    // （`false` を返しているのが「捨て軸が無いから」ではないこと）。
    expect(unallocatedNotice(WITH_LOST, true)).not.toBeNull();
  });

  /**
   * **案内は、消えた軸の綴りと、直す手順を言う。**
   *
   * 直す手段は「**同じ綴りで置き直す**」しかない（票にも累積にも削除の口が無い）。
   *
   * # ⚠ 軸名を `「」` で囲まないこと
   *
   * `tests/stat-scale-path.test.ts` の `quotedOperations` は
   * **`「」` の中身を「名指しした操作」として取り出す。**
   * 軸名を `「」` に入れると、**軸名が実在する操作かどうかを問われて赤になる。**
   * 軸名は `『』` で囲む。
   */
  it("案内は、消えた軸の綴りと、直す手順を言う", () => {
    const notice = unallocatedNotice(WITH_LOST, true);
    expect(notice).not.toBeNull();
    const text = notice as string;

    expect(text).toContain("『きえた軸』");
    expect(text).toContain("『むかしの軸』");
    // **軸名が「」に入っていない**（案内の検査が軸名を操作として読まない）。
    expect(text).not.toContain("「きえた軸」");
    expect(text).not.toContain("「むかしの軸」");

    // **名指しする操作は、この画面に実在するものだけ。**
    // 実在することそのものは `tests/stat-scale-path.test.ts` が画面で見る。
    expect(text).toContain("「軸を追加」");
    expect(text).toContain("「この内容で置き換える」");
  });

  /**
   * **無い操作を案内しない。**
   *
   * 票にも累積にも**消す口がサーバに無い**（追記のみ / ADR-0026 §決定3）。
   * 「消してください」と書いた瞬間、それは実行できない案内になる。
   */
  it("案内は、存在しない操作を勧めない", () => {
    const text = unallocatedNotice(WITH_LOST, true) as string;
    for (const missing of ["票を削除", "記録を消す", "軸を復元", "変容をリセット"]) {
      expect(text, `${missing} を勧めていないこと`).not.toContain(missing);
    }
  });
});

/**
 * **累積が最後に動いた時刻の見出し**（作業単位 D-5a）。
 *
 * `charge` の値だけでは「一度も採点されていない」と「0 に戻った」が区別できない
 * （`api.ts` の `charge_updated_at` の doc）。**分けているのはこの欄である。**
 */
describe("累積が動いた時刻の見出し", () => {
  /**
   * **⚠ 「そのまま出す」から変えた**（作業単位 F-7 / 司令塔の指示）。
   *
   * 以前ここは `chargeTimeLabel("…T01:02:03.400Z")` が
   * **その綴りをそのまま返す**ことを見ていた。
   * **生の ISO 表記を画面に出すのをやめた**ので、期待値が動いている。
   *
   * **見ているものは変えていない** ——
   * 「時刻があるときは、`null` のときと別の、その時刻の綴りを出す」。
   * **畳み方そのものの権威は `tests/timestamp.test.ts`。**
   * **期待値は直書き**（`timeLabel` から作ると、変異が期待値も一緒に動かす）。
   */
  it("時刻があれば、読める綴りにして出す", () => {
    expect(chargeTimeLabel("2026-08-26T01:02:03.400Z", "UTC")).toBe(
      "2026-08-26 01:02:03 (UTC)",
    );
    // **別の時刻は別の綴りになる**（定数へ潰す実装が赤になる）。
    expect(chargeTimeLabel("2023-03-03T03:03:03.030Z", "UTC")).toBe(
      "2023-03-03 03:03:03 (UTC)",
    );
  });

  /**
   * **⚠ 時間帯を、そのまま `timeLabel` へ渡している**（作業単位 F-7b）。
   *
   * **この関数は時間帯を決めない。** 決めさせると、
   * **ここが走らせた機械の時間帯でしか成立しなくなる**
   * （F-7 が地域化を避けた2つ目の理由 —— **正しい懸念だった**）。
   *
   * **同じ瞬間を3つの時間帯に通して、3つとも別の答え**になることを見る。
   * **引数を捨てて中で決めている実装**は、どれを選んでいても2つが外れる。
   * **期待値は直書き**（手で足し引きした / 東京は UTC+9、
   * ニューヨークの 8 月は UTC-4 ——**どちらも日付が動く**）。
   */
  it("渡された時間帯を、そのまま使う", () => {
    const at = "2026-08-26T01:02:03.400Z";

    expect([
      chargeTimeLabel(at, "UTC"),
      chargeTimeLabel(at, "Asia/Tokyo"),
      chargeTimeLabel(at, "America/New_York"),
    ]).toEqual([
      "2026-08-26 01:02:03 (UTC)",
      "2026-08-26 10:02:03 (UTC+09:00)",
      "2026-08-25 21:02:03 (UTC-04:00)",
    ]);
  });

  /**
   * **`null` の枝は、時間帯を見ない。**
   *
   * **対照**（CLAUDE.md「変更する場合としない場合が同じ入力に同居するか」）——
   * **時刻がある枝は時間帯で答えが変わり、無い枝は変わらない。**
   * 片方だけを見ると、**`null` を時刻として畳もうとする実装**も、
   * **常に「記録なし」を返す実装**も、どちらか一方が緑を通る。
   */
  it("対照: 行が無い枝は、時間帯を変えても同じ見出し", () => {
    expect(chargeTimeLabel(null, "Asia/Tokyo")).toBe(
      chargeTimeLabel(null, "America/New_York"),
    );
    expect(chargeTimeLabel(null, "Asia/Tokyo")).toBe(NO_CHARGE_TIME_LABEL);
  });

  /**
   * **`null` は「累積の行が無い」。**
   *
   * **生の `null` を出さない**（`null` と書かれた行になる）し、
   * **`0` でも現在時刻でもない**（「たったいま動いた」と読める）。
   */
  it("行が無ければ、数でも `null` でもない見出しになる", () => {
    const label = chargeTimeLabel(null, "UTC");
    expect(label).toBe(NO_CHARGE_TIME_LABEL);
    expect(label).not.toContain("null");
    expect(label).not.toMatch(/[0-9]/);
  });

  /**
   * **「割り当てなし」と同じ見出しにしない。**
   *
   * 2つは**別のこと**を言っている——
   * 「割り当てなし」は `stat_alloc` に行が無い（軸が改名された）、
   * こちらは**累積**の行が無い（一度も適用まで届いていない）。
   * **同じ文字列にすると、表の2つの列が同じことを言っているように読める。**
   */
  it("「割り当てなし」と同じ見出しにならない", () => {
    expect(chargeTimeLabel(null, "UTC")).not.toBe(UNALLOCATED_LABEL);
  });
});

/**
 * **変容の走らせ方**（作業単位 D-5a / `api.ts` の `DriftView.run_mode`）。
 *
 * **既定は `off`** で、`off` は判定もせず票も累積も1行も作らない。
 * **これを出さないと、「0 件」が「壊れている」なのか「off である」なのかが読めない。**
 */
describe("変容の走らせ方", () => {
  /**
   * **3つが別々の見出しを持つ。**
   *
   * # 同じ文字列にしない
   *
   * `shadow` と `on` の**違いは「値が動くかどうか」だけ**で、
   * どちらも票は残る。**同じ見出しにすると、画面から
   * 「値が動いているのか」が読めなくなる。**
   */
  it("3つの走らせ方が、それぞれ別の見出しを持つ", () => {
    const labels = [RUN_MODE_OFF, RUN_MODE_SHADOW, RUN_MODE_ON].map(runModeLabel);
    expect(new Set(labels).size, `見出しが重複している: ${labels.join(" / ")}`).toBe(3);
    // **綴りをそのまま返していない**（訳した見出しであること）。
    expect(labels).not.toContain(RUN_MODE_OFF);
  });

  /**
   * **どの綴りが、どの見出しになるか**（作業単位 D-5a のレビュー指摘 / 2026-08-28）。
   *
   * # ⚠ 濃度では帰属を見ていない（**実測で緑を通った**）
   *
   * 上の `new Set(labels).size === 3` は**「3つが違うこと」しか見ていない。**
   * レビュアーの実測:
   *
   * | 変異 | 結果 |
   * |---|---|
   * | `shadow` と `on` の見出しを入れ替える | **582 全緑** |
   * | 3つを3巡回で回転（`off`→実行中 / `shadow`→停止中 / `on`→記録のみ） | **582 全緑** |
   *
   * **CLAUDE.md の最頻の穴そのもの** ——「載っているか」と「どれのものか」は別の主張である。
   * **Rust 側は正しく書けていた**（`server::admin::tests::the_view_names_the_run_mode_it_was_given`
   * は `assert_eq!(named, vec!["off", "shadow", "on"])` で順序つきの帰属を見ている）。
   * **doc は写り、doc が名指しする見張り番の「帰属」だけが写らなかった。**
   *
   * # 実害はこの作業単位の存在理由そのもの
   *
   * **サーバが正しく `shadow` と言っているのに、読み手が読む1行が
   * 「実行中（票を残し、値も動かします）」になる。**
   * **値は動いていない。そして変容は本文に出ない**（ADR-0024 §9）ので、
   * **それを読める場所は他に無い。**
   *
   * # 期待値は**関数の外にある値**（直書き）
   *
   * `runModeLabel` から作ると、**綴りを入れ替える変異が期待値も一緒に動かして緑**になる
   * （CLAUDE.md「写しを1つにしても、その1つは自動的には守られない」/
   * `store::drift` の `each_result_uses_its_own_spelling` と同じ形）。
   * **文面ごと固定するので、言い回しを変えるときはここも一緒に直すことになる。**
   * それがこの検査の値段であり、**帰属を見るとはそういうことである。**
   */
  it("どの綴りがどの見出しになるかが、対応ごと固定されている", () => {
    expect(runModeLabel(RUN_MODE_OFF)).toBe("停止中（判定も記録もしていません）");
    expect(runModeLabel(RUN_MODE_SHADOW)).toBe(
      "記録のみ（票は残りますが、値は動きません）",
    );
    expect(runModeLabel(RUN_MODE_ON)).toBe("実行中（票を残し、値も動かします）");
  });

  /**
   * **「値が動くのか」を言っているのは `on` の見出しだけ。**
   *
   * # なぜ上の完全一致だけでは足りないと考えたか
   *
   * 完全一致は**文面が変わると必ず落ちる**ので、**言い回しの調整のたびに
   * 「期待値を写して直す」**ことになる。そのとき**意味まで一緒に壊しても気付けない**
   * ——写す手が正しい文面を写すとは限らない。
   *
   * **ここは意味の側を見る** ——
   * **`shadow` と `on` の違いは「値が動くかどうか」だけ**で、
   * **その1点が入れ替わると、画面は「動いていないもの」を「動いている」と言う。**
   *
   * 判定に使う語は**この検査の中に直書き**してあり、`runModeLabel` からは作らない。
   */
  it("値が動くと言っているのは、`on` の見出しだけ", () => {
    expect(runModeLabel(RUN_MODE_ON)).toContain("値も動かします");
    expect(runModeLabel(RUN_MODE_SHADOW)).toContain("値は動きません");
    // **止まっている側は、値の話をそもそもしない。**
    expect(runModeLabel(RUN_MODE_OFF)).not.toContain("値も動かします");
    expect(runModeLabel(RUN_MODE_SHADOW)).not.toContain("値も動かします");
  });

  /**
   * **知らない綴りは、そのまま返す**（`deltaLabel` と同じ立場）。
   *
   * サーバが4つ目の走らせ方を足した日に、
   * **画面から走らせ方の欄が消えるのがいちばん質の悪い壊れ方**である。
   */
  it("知らない綴りは、そのまま返す（欄ごと消さない）", () => {
    expect(runModeLabel("holiday")).toBe("holiday");
    expect(runModeLabel("")).toBe("");
  });

  /**
   * **`off` のときだけ案内を出す。**
   *
   * `shadow` も `on` も**判定は走っている**ので、
   * 「動いていません」と書くと嘘になる。
   */
  it("案内が出るのは `off` のときだけ", () => {
    // **出どころ（作業単位 D-15）によらない** —— どちらの出どころでも同じ分け方。
    for (const source of [RUN_MODE_SOURCE_DEFAULT, RUN_MODE_SOURCE_CHARACTER]) {
      expect(runModeNotice(RUN_MODE_OFF, source)).not.toBeNull();
      expect(runModeNotice(RUN_MODE_SHADOW, source)).toBeNull();
      expect(runModeNotice(RUN_MODE_ON, source)).toBeNull();
      // **知らない綴りにも出さない**（何が起きているか分からないので、推測しない）。
      expect(runModeNotice("holiday", source)).toBeNull();
    }
  });

  /**
   * **案内は「0 件」の理由を言い、この画面に実在する導線だけを名指しする。**
   *
   * # ⚠ 作業単位 D-15 で主張が変わった
   *
   * **以前は「`「」` で何も名指ししない」を固定していた** ——
   * 走らせ方は起動設定で、**切り替える口が画面にも管理 API にも無かった**から。
   * **いまはキャラクターの編集で切り替えられる**（ADR-0045）。
   * その編集はこの画面には無いので、**この画面に実在する「キャラクターの一覧へ」
   * だけを名指しし**、編集はそこから辿れると言う。
   * 名指しした導線が画面に実在することは `stats-panel.test.ts` の
   * `assertNoticeNamesOnlyRealOperations` が見る。
   *
   * **無い操作の綴りは、どちらの出どころでも書かない**（下の一覧）。
   */
  it("案内は、この画面に実在する導線だけを名指しする", () => {
    for (const source of [RUN_MODE_SOURCE_DEFAULT, RUN_MODE_SOURCE_CHARACTER]) {
      const text = runModeNotice(RUN_MODE_OFF, source) as string;
      const named = [...text.matchAll(/「([^」]+)」/g)].map((found) => found[1]);
      expect(named, source).toEqual(["キャラクターの一覧へ"]);
      for (const missing of ["変容を開始", "走らせる", "切り替える）", "有効にする"]) {
        expect(text, `${missing} というボタンは無い`).not.toContain(missing);
      }
    }
  });
});

/**
 * **止まっている案内と出どころの一言は、出どころで言い分ける**（作業単位 D-15 / ADR-0045）。
 *
 * # 何が変わったのか
 *
 * **以前の案内は、止まっていれば必ず環境変数を案内していた。**
 * **キャラクターが自分で「止める」を選んでいる回には、その案内は効かない操作を指す**
 * （キャラクターの値が環境変数に勝つ / origin#31 のレビュー）。
 *
 * # 腕 —— 出どころだけが違う2つが、違う文を出す
 *
 * **期待値は語の直書き**（関数から作らない）。
 */
describe("走らせ方の出どころで言い分ける", () => {
  it("止まっている案内は、出どころだけが違えば違う文になる", () => {
    const byDefault = runModeNotice(RUN_MODE_OFF, RUN_MODE_SOURCE_DEFAULT) as string;
    const byCharacter = runModeNotice(RUN_MODE_OFF, RUN_MODE_SOURCE_CHARACTER) as string;
    expect(byDefault).not.toBe(byCharacter);

    // **既定から来た停止**: 既定に従っていると言い、編集で動かせると言う。
    // 環境変数の名前は**補足として残す**（測定する人向け）。
    expect(byDefault).toContain("起動時の既定");
    expect(byDefault).toContain("PERSONACORE_STAT_DRIFT");
    expect(byDefault).toContain("次のやり取りから");
    // **「動かす」と「記録だけ」を同じ結果として言わない**（D-15 のレビュー）——
    // 記録だけは値を動かさない。**2つの結果を別々に言う。**
    expect(byDefault).toContain("動かすを選ぶと、次のやり取りから値が動き");
    expect(byDefault).toContain("記録だけを選ぶと、判定と記録だけが始まります（値は動きません）");
    expect(byDefault).not.toContain("動かす（または記録だけ）を選ぶと、次のやり取りから動きます");

    // **キャラクターが選んだ停止**: 自分の設定だと言い、**環境変数を案内しない**
    // （このキャラクターには効かない操作になる）。
    expect(byCharacter).toContain("このキャラクター自身の設定");
    expect(byCharacter).not.toContain("PERSONACORE_STAT_DRIFT");
    expect(byCharacter).not.toContain("起動時の既定に従っています");
    expect(byCharacter).toContain("次のやり取りから");

    // **どちらも「0 件」の理由を言う**（D-5a の主張はそのまま）。
    for (const text of [byDefault, byCharacter]) {
      expect(text).toContain("壊れているからではありません");
    }
  });

  /** **出どころの一言は、走らせ方によらず出る**（`shadow` / `on` でも出どころを言う）。 */
  it("出どころの一言は、出どころごとに決まった文", () => {
    expect(runModeSourceNote(RUN_MODE_SOURCE_CHARACTER)).toBe(
      "この走らせ方は、このキャラクター自身の設定です（キャラクターの編集で変えられます）。",
    );
    expect(runModeSourceNote(RUN_MODE_SOURCE_DEFAULT)).toBe(
      "このキャラクターは走らせ方を選んでいないので、起動時の既定に従っています" +
        "（キャラクターの編集で、このキャラクターだけの走らせ方を選べます）。",
    );
    // **知らない綴りは、そのまま出す**（推測しない / `runModeLabel` と同じ立場）。
    expect(runModeSourceNote("upstream")).toBe("走らせ方の出どころ: upstream");
  });
});

/**
 * **出どころの綴りは `src/domain/drift.rs` の `DriftModeSource::label` の写しである**
 * （作業単位 D-15）。ずれると**出どころの一言と案内の言い分けが黙って外れる**
 * （どちらでもない綴りとして扱われる）。**走らせ方の綴りと同じ作法**で、写し元を読む。
 */
describe("出どころの綴りは写し元と一致する", () => {
  it("`DriftModeSource::label` と一致する", () => {
    const source = readFileSync(
      fileURLToPath(new URL("../../src/domain/drift.rs", import.meta.url)),
      "utf8",
    );
    expect(sourceOf(source, "Character")).toBe(RUN_MODE_SOURCE_CHARACTER);
    expect(sourceOf(source, "Default")).toBe(RUN_MODE_SOURCE_DEFAULT);
    expect(RUN_MODE_SOURCE_CHARACTER).not.toBe(RUN_MODE_SOURCE_DEFAULT);
  });

  it("対照: 植えたソースからは、その場の値を読む（入れ替わりも見る）", () => {
    const planted = [
      "impl DriftModeSource {",
      "    pub const fn label(self) -> &'static str {",
      "        match self {",
      '            Self::Character => "planted_default",',
      '            Self::Default => "planted_character",',
      "        }",
      "    }",
      "}",
    ].join("\n");
    expect(sourceOf(planted, "Character")).toBe("planted_default");
    expect(sourceOf(planted, "Default")).toBe("planted_character");
  });

  it("対照: 見つからなければ落ちる（別の型の `label` を読まない）", () => {
    expect(() => sourceOf("", "Character")).toThrow();
    // **`impl DriftModeSource` が無ければ、他の型の `label` に当たっても読まない。**
    const other = [
      "impl StatDriftMode {",
      "    pub const fn label(self) -> &'static str {",
      '        match self { Self::Default => "off", }',
      "    }",
      "}",
    ].join("\n");
    expect(() => sourceOf(other, "Default")).toThrow();
  });
});

/**
 * `DriftModeSource::label` の腕を1つ読む。**`impl DriftModeSource {` の中だけを見る**
 * （同じファイルに `label` が何本もある / `runModeOf` の doc と同じ罠）。
 */
function sourceOf(source: string, variant: string): string {
  const implAt = source.indexOf("impl DriftModeSource {");
  if (implAt < 0) {
    throw new Error("impl DriftModeSource が見つかりません");
  }
  const scope = source.slice(implAt);
  const start = scope.indexOf("pub const fn label(self) -> &'static str {");
  if (start < 0) {
    throw new Error("DriftModeSource の label が見つかりません");
  }
  const rest = scope.slice(start);
  const end = rest.indexOf("\n    }");
  const block = end < 0 ? rest : rest.slice(0, end);
  const found = new RegExp(`Self::${variant} => "([^"]+)"`).exec(block);
  if (found?.[1] === undefined) {
    throw new Error(`${variant} の綴りがありません`);
  }
  return found[1];
}

/**
 * **走らせ方の綴りは `src/domain/drift.rs` の `StatDriftMode::label` の写しである。**
 *
 * ずれると**画面の見出しが消え、案内も出なくなる**（`off` を `off` と読めない）。
 * **向きの綴り（`delta_to_wire`）と同じ作法**で、写し元を実際に読んで突き合わせる。
 */
describe("走らせ方の綴りは写し元と一致する", () => {
  const domainSource = (): string =>
    readFileSync(
      fileURLToPath(new URL("../../src/domain/drift.rs", import.meta.url)),
      "utf8",
    );

  it("`StatDriftMode::label` と一致する", () => {
    const source = domainSource();
    expect(runModeOf(source, "Off")).toBe(RUN_MODE_OFF);
    expect(runModeOf(source, "Shadow")).toBe(RUN_MODE_SHADOW);
    expect(runModeOf(source, "On")).toBe(RUN_MODE_ON);
    // **対照: 3つは別物。**
    expect(new Set([RUN_MODE_OFF, RUN_MODE_SHADOW, RUN_MODE_ON]).size).toBe(3);
  });

  it("対照: 植えたソースからは、その場の値を読む（入れ替わりも見る）", () => {
    const planted = [
      "    pub const fn label(self) -> &'static str {",
      "        match self {",
      '            Self::Off => "planted_off",',
      '            Self::Shadow => "planted_shadow",',
      '            Self::On => "planted_on",',
      "        }",
      "    }",
    ].join("\n");
    expect(runModeOf(planted, "Off")).toBe("planted_off");
    expect(runModeOf(planted, "Shadow")).toBe("planted_shadow");
    expect(runModeOf(planted, "On")).toBe("planted_on");
  });

  it("対照: 見つからなければ落ちる", () => {
    expect(() => runModeOf("", "Off")).toThrow();
  });
});

/**
 * `StatDriftMode::label` の腕を1つ読む。
 *
 * # ⚠ `impl StatDriftMode` の中に入ってから探すこと（**着手時に実際に踏んだ**）
 *
 * `src/domain/drift.rs` には **`label` という名前の関数が他にもある**
 * （`DriftDelta` / `DriftOutcome` / `DriftForm` …）。
 * ファイルの先頭から `pub const fn label` を探すと**別の型の綴りを読む**ので、
 * **走らせ方の綴りを壊す変異が緑を通る。**
 *
 * **`impl` の閉じ括弧までで切る。** 切らないと、
 * `label` を持たない型を尋ねたときに**次の型の値を返す**（`codeOf` と同じ罠）。
 */
function runModeOf(source: string, variant: string): string {
  // **植えたソースは `impl` 行を持たない**ので、無ければファイル全体を見る。
  const implAt = source.indexOf("impl StatDriftMode {");
  const scope = implAt < 0 ? source : source.slice(implAt);
  const start = scope.indexOf("pub const fn label(self) -> &'static str {");
  if (start < 0) {
    throw new Error("StatDriftMode の label が見つかりません");
  }
  const rest = scope.slice(start);
  const end = rest.indexOf("\n    }");
  const block = end < 0 ? rest : rest.slice(0, end);
  const found = new RegExp(`Self::${variant} => "([^"]+)"`).exec(block);
  if (found?.[1] === undefined) {
    throw new Error(`${variant} の綴りがありません`);
  }
  return found[1];
}

/**
 * **軸が1本も無いときの案内**（作業単位 F-6）。
 *
 * # なぜ要るのか
 *
 * **ログは `drift_skipped=no_axes` と言っている**（`src/server/drift.rs` の
 * `DriftSkipReason::NoAxes` —— `build_drift_prompt` が `None` を返すので、
 * **上流を呼ばずに打ち切る**）。**それなのに画面は**
 * **走らせ方「実行中（票を残し、値も動かします）」と「まだ1件も票がありません」を
 * 並べて出していた** —— **2つが理由で繋がっていない。**
 *
 * **新しいキャラクターは必ずこの状態から始まる。**
 * `D-5a` が「止めてある」と「壊れている」を分けた、**その次の穴**である。
 *
 * # ⚠ 走らせ方には依存しない
 *
 * `runModeNotice` が理由を言うのは `off` のときだけ。
 * **こちらは3つの走らせ方すべてで同じことを言う** ——
 * **軸が無ければ、何を設定しても判定は走らない。**
 */
describe("軸が1本も無いときは、その理由を言う（作業単位 F-6）", () => {
  /** 軸が1本も無い変容の記録。**走らせ方は「実行中」**（F-6 が報告された状態そのもの）。 */
  const NO_AXES: DriftView = {
    character: "aoi",
    stat_scale: 5,
    axes: [],
    total_votes: 0,
    recent_limit: 50,
    recent_votes: [],
    run_mode: RUN_MODE_ON,
    run_mode_source: "default",
    default_run_mode: RUN_MODE_ON,
  };

  /** 軸が1本ある変容の記録（**対照**）。 */
  const WITH_AXIS: DriftView = {
    ...NO_AXES,
    axes: [
      {
        axis: "勇気",
        value: 0,
        charge: 0,
        charge_updated_at: null,
        ups: 0,
        downs: 0,
      },
    ],
  };

  it("軸が1本も無ければ、判定そのものが走らないと言う", () => {
    const text = noAxesNotice(NO_AXES, true);

    expect(text).not.toBeNull();
    const said = text as string;
    // **起きていることを言う。**
    expect(said).toContain("軸が1本もありません");
    // **「0 件」と繋がっている**（走らせ方の見出しと矛盾しないこと）。
    expect(said).toContain("判定");
    expect(said).toContain("票");
  });

  /**
   * **走らせ方を問わない。**
   *
   * **3つとも同じ文**でなければならない —— 「実行中だから走っている」と
   * 読める文が1つでも混ざると、**F-6 の壊れ方がそのまま残る。**
   */
  it("走らせ方が実行中でも、記録のみでも、停止中でも同じことを言う", () => {
    const texts = [RUN_MODE_ON, RUN_MODE_SHADOW, RUN_MODE_OFF].map((mode) =>
      noAxesNotice({ ...NO_AXES, run_mode: mode }, true),
    );

    expect(new Set(texts).size, `走らせ方ごとに違う文が出ている: ${texts.join(" / ")}`).toBe(1);
    expect(texts[0]).not.toBeNull();
  });

  it("軸が1本でもあれば、何も言わない", () => {
    expect(noAxesNotice(WITH_AXIS, true)).toBeNull();
    expect(noAxesNotice(WITH_AXIS, false)).toBeNull();
  });

  /**
   * **割り当ての無い軸も「軸がある」側である。**
   *
   * サーバは `value: null` で返すが、**行は変容の記録に居る**ので、
   * `build_drift_prompt` の側でも許可リストは空ではない。
   * ここで拾うと、**軸が居るのに「1本もありません」と書く。**
   */
  it("割り当てを失った軸だけでも、「1本もありません」とは言わない", () => {
    const onlyLost: DriftView = {
      ...NO_AXES,
      axes: [
        {
          axis: "きえた軸",
          value: null,
          charge: 1.5,
          charge_updated_at: null,
          ups: 0,
          downs: 2,
        },
      ],
    };
    expect(noAxesNotice(onlyLost, true)).toBeNull();
  });

  /**
   * **名指しする操作は、置き換えのフォームが描かれているときだけ。**
   *
   * `unallocatedNotice` と同じ立場（`B-16` の是正）——
   * **`GET .../stats` が失敗した回は「軸を追加」も「この内容で置き換える」も画面に無い。**
   *
   * **⚠ ただし、こちらは黙らない。** 理由そのものは**その状態でも真**であり、
   * **黙ると F-6 の穴が失敗経路でだけ残る。**
   */
  it("置き換えのフォームが無いときは、操作を名指ししない（理由は言う）", () => {
    const withForm = noAxesNotice(NO_AXES, true) as string;
    const without = noAxesNotice(NO_AXES, false) as string;

    expect(without).not.toBeNull();
    // **理由は両方で言う。**
    expect(without).toContain("軸が1本もありません");
    // **名指しは、フォームがあるときだけ。**
    expect(withForm).toMatch(/「[^」]+」/);
    expect(without).not.toMatch(/「[^」]+」/);
  });

  /**
   * **名指しするのは、この画面に実在する2つだけ。**
   *
   * 綴りは**この検査の中に直書き**してある（`StatsPanel.svelte` から作ると、
   * ボタンの綴りを変える変異が期待値も一緒に動かして緑になる）。
   * **画面に実在することは `tests/stats-panel.test.ts` が見る。**
   */
  it("名指しするのは「軸を追加」と「この内容で置き換える」だけ", () => {
    const named = [
      ...(noAxesNotice(NO_AXES, true) as string).matchAll(/「([^」]+)」/g),
    ].map((found) => found[1] as string);

    expect(named).toEqual(["軸を追加", "この内容で置き換える"]);
  });

  /**
   * **無い操作を名指ししない**（`B-15` / `B-16` / `D-6` / `D-7` / `D-8` と同じ立場）。
   *
   * **走らせ方を切り替える口はこの画面には無い**（キャラクターの編集にある / 作業単位 D-15）。
   * 軸ゼロの案内は走らせ方の話をしないので、切り替えの操作を名指ししない。
   * （D-15 より前は「画面にも管理 API にも口が無い」がこのテストの理由だった。）
   */
  it("走らせ方を切り替える操作を、名指ししない", () => {
    const text = noAxesNotice(NO_AXES, true) as string;
    for (const missing of ["変容を開始", "有効にする", "走らせる"]) {
      expect(text, `${missing} という操作は無い`).not.toContain(missing);
    }
  });
});
