/**
 * **変容の走らせ方の選択**（作業単位 D-15 / ADR-0045 / `src/lib/driftModeChoice.ts`）と、
 * それが作成・編集・修復の本文へどう載るか（`characterEdit.ts` / `characterRepair.ts`）。
 *
 * # ここが見るもの
 *
 * | 何 | なぜ |
 * |---|---|
 * | **4つの選択が送る本文**を配列で固定する | 取り違え（`shadow` と `on` の入れ替え等）が1本で見える |
 * | **触らなかったら `stat_drift_mode` のキーが無い**（`null` とは別） | `null` は「未設定に戻す」という正当な指示。「変えない」と同じ表現にしない |
 * | **「既定に従う」の括弧は、渡された既定から作る** | キャラクターの値や `run_mode` を書く取り違えを止める |
 * | **修復はいつも載せる** | 全部の欄を送るモード（`characterRepair.ts` の doc） |
 *
 * **期待値は直書き**（`RUN_MODE_*` や `runModeLabel` から作らない ——
 * 作ると、綴りを入れ替える変異が期待値も一緒に動かして緑になる）。
 *
 * **jsdom は要らない**（DOM を触らない）。
 */

import { describe, expect, it } from "vitest";

import type { CharacterView } from "../src/lib/api";
import {
  buildCreate,
  buildUpdate,
  draftOf,
  emptyDraft,
  type CharacterDraft,
} from "../src/lib/characterEdit";
import {
  buildRepair,
  emptyRepairDraft,
  repairDriftModeChoices,
} from "../src/lib/characterRepair";
import {
  DRIFT_MODE_FIELD_LABEL,
  driftModeChoices,
  followDefaultLabel,
} from "../src/lib/driftModeChoice";

/**
 * 編集の対象。**走らせ方はわざと「サーバが将来足すかもしれない綴り」にしてある**
 * ——4つの選択のどれとも違うので、**4つとも「触った」として本文に載る。**
 */
const FUTURE: CharacterView = {
  id: 7,
  handle: "aoi",
  display_name: "藍生",
  base_persona: "静かな声で話す。",
  output_format_instruction: null,
  stat_scale: 5,
  stat_drift_mode: "holiday",
  axis_discovery_slots: 0,
  archived_at: null,
  created_at: "2026-08-27T00:00:00Z",
};

/** 選択の表示名から値を引く（**画面と同じ一覧を通す**）。 */
function valueOf(label: string, defaultRunMode: string | null = null): string {
  const found = driftModeChoices(defaultRunMode).find((choice) =>
    choice.label.startsWith(label),
  );
  expect(found, `「${label}」の選択肢があること`).toBeDefined();
  return (found as { value: string }).value;
}

/** 本文の `stat_drift_mode` を「キーが無い」と区別して読む。 */
function sent(body: object): unknown {
  return "stat_drift_mode" in body
    ? (body as { stat_drift_mode: unknown }).stat_drift_mode
    : "（キー無し）";
}

describe("選択肢", () => {
  it("4つが、この順で並ぶ", () => {
    expect(driftModeChoices(null).map((choice) => choice.label)).toEqual([
      "既定に従う",
      "止める",
      "記録だけ（値は動かさない）",
      "動かす",
    ]);
  });

  it("欄の見出し", () => {
    expect(DRIFT_MODE_FIELD_LABEL).toBe("変容の走らせ方");
  });

  /**
   * **括弧の中は、渡された既定から言う**（写しを持たない / `runModeLabel` を通す）。
   *
   * **3つの既定で回す** —— 1つだけだと、その値に決め打つ実装が通る。
   */
  it("「既定に従う」の括弧は、渡された既定を言う", () => {
    expect(followDefaultLabel("off")).toBe(
      "既定に従う（いまは: 停止中（判定も記録もしていません））",
    );
    expect(followDefaultLabel("shadow")).toBe(
      "既定に従う（いまは: 記録のみ（票は残りますが、値は動きません））",
    );
    expect(followDefaultLabel("on")).toBe(
      "既定に従う（いまは: 実行中（票を残し、値も動かします））",
    );
    // **分からないなら括弧を出さない**（新規作成・読み込み中・読めなかった回）。
    expect(followDefaultLabel(null)).toBe("既定に従う");
    // **知らない綴りはそのまま出す**（`runModeLabel` と同じ立場）。
    expect(followDefaultLabel("holiday")).toBe("既定に従う（いまは: holiday）");
    // **一覧の先頭も同じ文言**（一覧と関数が別の文言を持たない）。
    expect(driftModeChoices("off")[0]?.label).toBe(followDefaultLabel("off"));
  });
});

describe("作成の本文", () => {
  /**
   * **4つの選択が送るもの。** 「既定に従う」は**キーごと省く**（サーバは省略を未設定と読む）。
   */
  it("4つの選択が送る本文", () => {
    const bodies = driftModeChoices(null).map((choice) => {
      const draft: CharacterDraft = {
        ...emptyDraft(),
        handle: "rin",
        base_persona: "早口で話す。",
        stat_drift_mode: choice.value,
      };
      const built = buildCreate(draft);
      expect(built.ok).toBe(true);
      return built.ok ? sent(built.body) : null;
    });
    expect(bodies).toEqual(["（キー無し）", "off", "shadow", "on"]);
  });

  it("空の下書きは「既定に従う」から始まる", () => {
    expect(emptyDraft().stat_drift_mode).toBe(valueOf("既定に従う"));
  });
});

describe("編集の本文", () => {
  /**
   * **4つの選択が送る本文**（配列で固定 / Issue origin#32 の閉じる条件）。
   *
   * 元の値がどの選択とも違うので、4つとも載る。**「既定に従う」は `null`。**
   */
  it("4つの選択が送る本文", () => {
    const bodies = driftModeChoices(null).map((choice) => {
      const built = buildUpdate(FUTURE, {
        ...draftOf(FUTURE),
        stat_drift_mode: choice.value,
      });
      expect(built.ok).toBe(true);
      return built.ok ? built.body : null;
    });
    expect(bodies).toEqual([
      { stat_drift_mode: null },
      { stat_drift_mode: "off" },
      { stat_drift_mode: "shadow" },
      { stat_drift_mode: "on" },
    ]);
  });

  /**
   * **選択を触らずに他の欄だけ直したら、キーごと無い**（`null` ではない）。
   *
   * **変えるもの（表示名）と変えないもの（走らせ方）が同じ入力に同居する**形で見る
   * （CLAUDE.md「変えなかったら元を返す」関数の見方）。
   * **元の値が設定済み（`on`）と未設定（`null`）の両方で見る** ——
   * 未設定だけだと、「触らなければ `null` を送る」実装が区別できない。
   */
  it("触らずに他の欄だけ直したら、走らせ方のキーは無い", () => {
    for (const stored of ["on", null]) {
      const original: CharacterView = { ...FUTURE, stat_drift_mode: stored };
      const built = buildUpdate(original, {
        ...draftOf(original),
        display_name: "碧",
      });
      expect(built.ok).toBe(true);
      if (built.ok) {
        expect(built.body, `元の値 = ${String(stored)}`).toEqual({
          display_name: "碧",
        });
        expect("stat_drift_mode" in built.body).toBe(false);
      }
    }
  });

  /** **下書きは元の値から始まる**（未設定は「既定に従う」）。 */
  it("下書きは保存されている値から始まる", () => {
    expect(draftOf({ ...FUTURE, stat_drift_mode: "shadow" }).stat_drift_mode).toBe(
      valueOf("記録だけ"),
    );
    expect(draftOf({ ...FUTURE, stat_drift_mode: null }).stat_drift_mode).toBe(
      valueOf("既定に従う"),
    );
    // **知らない綴りは、そのまま持つ**（丸めると、触っていないのに本文に載る）。
    expect(draftOf(FUTURE).stat_drift_mode).toBe("holiday");
    const built = buildUpdate(FUTURE, draftOf(FUTURE));
    expect(built.ok && built.body).toEqual({});
  });
});

/**
 * **修復の本文**（作業単位 D-15 / レビューの是正で分けた）。
 *
 * # 走らせ方の列が壊れているかで、初期値が違う
 *
 * | 行 | 選択肢 | 初期値 | 触らずに保存したとき |
 * |---|---|---|---|
 * | **走らせ方の列が壊れている** | 4つ | 既定に従う | **`null` を送る**（未設定に戻す） |
 * | **壊れていない** | 変えない＋4つ | **変えない** | **キーを省く**（サーバは省いた列を残す） |
 *
 * **壊れていない列の正しい値を、未設定で上書きしない**ため（D-15 のレビュー）。
 * 修復の他の列は元の値が読めないので全部送るが、この列は「壊れていない」ことが
 * `invalid_columns` から分かる —— **分かっているなら触らない。**
 */
describe("修復の本文", () => {
  /** 本文を組む（**他の欄は正当な値で埋める**）。 */
  function repaired(driftModeBroken: boolean, chosen?: string): unknown {
    const draft = emptyRepairDraft(driftModeBroken);
    const built = buildRepair({
      ...draft,
      display_name: "直した",
      base_persona: "直した本文",
      stat_scale: "7",
      stat_drift_mode: chosen ?? draft.stat_drift_mode,
    });
    expect(built.ok).toBe(true);
    return built.ok ? sent(built.body) : null;
  }

  /** **壊れている行**: 4つの選択が送る本文（いつも載る）。 */
  it("走らせ方が壊れている行では、4つの選択がいつも載る", () => {
    const four = repairDriftModeChoices(true);
    expect(four.map((choice) => choice.value)).toEqual(
      driftModeChoices(null).map((choice) => choice.value),
    );
    expect(four.map((choice) => repaired(true, choice.value))).toEqual([
      null,
      "off",
      "shadow",
      "on",
    ]);
    // **触らなければ「既定に従う」＝ `null`（キーは載る）。**
    expect(repaired(true)).toBeNull();
  });

  /**
   * **壊れていない行**: 触らなければキーを省き、選び直せば載る。
   *
   * **選び直しは真ん中（「記録だけ」）で見る**（端を選ぶと決め打ちが通る）。
   */
  it("走らせ方が壊れていない行では、触らなければキーが無く、選び直せば載る", () => {
    const five = repairDriftModeChoices(false);
    expect(five.map((choice) => choice.label)).toEqual([
      "変えない（いまの設定のまま）",
      "既定に従う",
      "止める",
      "記録だけ（値は動かさない）",
      "動かす",
    ]);
    // **触らない**: キーごと無い（`null` とは別）。
    expect(repaired(false)).toBe("（キー無し）");
    // **選び直す**: 真ん中を選ぶと、その値が載る。
    expect(repaired(false, valueOf("記録だけ"))).toBe("shadow");
    // **「既定に従う」を明示で選べば `null`**（変えないとは別の表現）。
    expect(repaired(false, valueOf("既定に従う"))).toBeNull();
  });

  it("修復の下書きの初期値は、列が壊れているかで決まる", () => {
    expect(emptyRepairDraft(true).stat_drift_mode).toBe(valueOf("既定に従う"));
    expect(emptyRepairDraft(false).stat_drift_mode).toBe(
      repairDriftModeChoices(false)[0]?.value,
    );
    // **番兵は、どの走らせ方の綴りとも、既定に従うとも違う。**
    expect(
      driftModeChoices(null).map((choice) => choice.value),
    ).not.toContain(emptyRepairDraft(false).stat_drift_mode);
  });
});
